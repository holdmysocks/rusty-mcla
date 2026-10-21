//! Game flow: main menu, garage, free roam, checkpoint races with AI opponents, pause, results.

use crate::ai::AiDriver;
use crate::physics::CarInput;
use crate::mc4::{PAGE_DISPLAY, PAGE_PERFORMANCE};
use crate::vehicle_model::{CarConfig, Slot, CUSTOM_SLOTS};
use crate::vehicles::{Vehicles, PALETTE};
use crate::world::World;
use glam::Vec3;
use std::path::PathBuf;

/// cash a new career starts with
pub use crate::rewards::START_MONEY;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    MainMenu,
    RaceSelect,
    Garage,
    FreeRoam,
    Race,
    Results,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceKind {
    Race,
    /// drive the supplied car to the destination in time without wrecking it
    Delivery,
    /// wreck the target car before the time runs out
    Payback,
}

/// The options of the pause menu (the game's OPTIONS: volumes, subtitles, camera shake,
/// steering sensitivity), in steps of ten.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub master: u8,
    pub music: u8,
    pub subtitles: bool,
    pub shake: bool,
    pub sens: u8,
    /// the original's three further volumes (AUDIO OPTIONS). Kept and saved; the mixer
    /// does not use them yet.
    #[serde(default = "full_volume")]
    pub voice: u8,
    #[serde(default = "full_volume")]
    pub vehicle: u8,
    #[serde(default = "full_volume")]
    pub cutscene: u8,
    /// OPTIONS > PERFORMANCE > HIGH-DETAIL TRAFFIC (the port's own, DECISIONS.md D19): traffic cars keep their
    /// full model at every distance instead of the original's LOD 1 / LOD 2 (vehicles.rs `pick_lods`). Off =
    /// the original.
    #[serde(default)]
    pub hq_traffic: bool,
}

fn full_volume() -> u8 {
    10
}

impl Default for Options {
    fn default() -> Self {
        Options { master: 10, music: 10, subtitles: false, shake: true, sens: 5, voice: 10, vehicle: 10, cutscene: 10, hq_traffic: false }
    }
}

#[derive(Debug, Clone)]
pub struct Race {
    pub route: Vec<usize>,        // link indices
    /// race line polyline (the game's own races)
    pub path: Option<std::sync::Arc<Vec<Vec3>>>,
    pub checkpoints: Vec<Vec3>,   // world positions
    pub next: Vec<usize>,         // per car (index into vehicles.cars) → next checkpoint
    pub finish_time: Vec<Option<f32>>,
    pub time: f32,
    pub countdown: f32,
    pub total_len: f32,
    pub participants: Vec<usize>, // car indices (player first)
    /// checkpoints per lap and number of laps (1 for point-to-point)
    pub lap_len: usize,
    pub laps: usize,
    pub name: String,
    /// seconds to beat (0 = none)
    pub goal_time: f32,
    pub kind: RaceKind,
    /// mission time limit in seconds (0 = none)
    pub time_limit: f32,
    /// payback: index of the car to wreck
    pub target: Option<usize>,
    /// mission ended in failure: the reason
    pub failed: Option<&'static str>,
    /// radius of each checkpoint of a lap (from the race's script; empty = 24 m)
    pub cp_radius: Vec<f32>,
    /// "Line" checkpoints of a lap: the two ends of the gate to drive through
    pub cp_gate: Vec<Option<(Vec3, Vec3)>>,
    /// where every car was a frame ago (for crossing gates)
    pub prev_pos: Vec<Vec3>,
    /// off-road stretches of the racing lines
    pub shortcuts: Vec<Shortcut>,
    /// where the police wait along this race (position, radius, cruiser already there)
    pub cop_zones: Vec<(Vec3, f32, bool)>,
    /// lines found for them (several cars take the same ones)
    pub shortcut_lines: std::collections::HashMap<[i32; 4], Vec<Vec3>>,
    /// an unordered race: the checkpoints in the order of their file, which the computer
    /// drivers keep to (`checkpoints` is then the player's list: taken in any order, the ones
    /// cleared moved to the front, the finish last)
    pub rivals_checkpoints: Option<Vec<Vec3>>,
    /// the checkpoints of a lap as the race's script sets them up (radius, heights, lines) -
    /// `checkpoints.rs`; empty: the port's own reach (its street races, missions without a
    /// script table)
    pub cp_specs: Vec<crate::checkpoints::CpSpec>,
}

/// Take the little loops out of a line laid along the lane network: junction connectors
/// make it double back on itself within a few metres (down a turn lane and up again),
/// which no racing car would drive. Points for which `keep` holds stay.
pub fn drop_loops(line: &mut Vec<Vec3>, keep: &dyn Fn(Vec3) -> bool) -> usize {
    let flat = |a: Vec3, b: Vec3| Vec3::new(a.x - b.x, 0.0, a.z - b.z).length();
    let mut removed = 0;
    let mut i = 0;
    while i + 2 < line.len() {
        let mut cut = None;
        for j in (i + 2..(i + 9).min(line.len())).rev() {
            let d = flat(line[i], line[j]);
            if d < 6.5 && (line[i].y - line[j].y).abs() < 2.0 {
                let along: f32 = (i..j).map(|k| flat(line[k], line[k + 1])).sum();
                if along > d * 2.5 + 4.0 && !(i + 1..j).any(|k| keep(line[k])) {
                    cut = Some(j);
                    break;
                }
            }
        }
        if let Some(j) = cut {
            line.drain(i + 1..j);
            removed += j - i - 1;
        }
        i += 1;
    }
    removed
}

/// Where the vinyl editor is: zone, layer and menu row.
#[derive(Clone, Default, Debug)]
pub struct VinylEdit {
    /// with the garage movie: 0 CHOOSE GROUP, 1 CHOOSE LAYER, 2 the shape strip, 3 EDIT
    /// LAYER's tabs, 4 LAYER OPTIONS (9: the port's own list, without the movie)
    pub page: u8,
    /// EDIT LAYER's tab (position, scale, rotate, skew, mask, colour) / the options' cursor
    pub tab: usize,
    /// the shape strip: what to put back on BACK (the layer's shape, or None = the layer
    /// was added for the strip and goes again)
    pub undo_shape: Option<u16>,
    pub added: bool,
    pub zone: usize,
    pub layer: usize,
    pub row: usize,
}

fn rgb_to_hsv(c: [u8; 4]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let d = mx - mn;
    let h = if d < 1e-5 {
        0.0
    } else if mx == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if mx == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, if mx > 0.0 { d / mx } else { 0.0 }, mx)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 4] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [(r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8, 255]
}

/// What the garage calls a livery choice: a preset of the disc, the player's own, or one
/// of the finished ones the opponents wear.
fn vinyl_label(v: &str, i: usize) -> String {
    if let Some(p) = v.strip_prefix("preset:") {
        let p = p.trim_start_matches("sc_").trim_start_matches("preset_").trim_start_matches("bikepreset");
        format!("PRESET {}", p.to_uppercase())
    } else if v == "custom" {
        "CUSTOM".to_string()
    } else {
        format!("LIVERY {}", i + 1)
    }
}

/// Section of the garage's price lists that a slot's parts are in.
/// The game's vehicle table (read from the running game: the purchase records at profile
/// + 0x3C370, 84 bytes each, begin with the vehicle's name hash): the sub ids of the
/// statistics by vehicle, and the order of VEHICLE STATS.
pub const VEHICLE_TABLE: [&str; 64] = [
    "vp_vw_golf_83", "vp_nsn_240sx_98", "vp_dsn_280z_75", "vp_vw_scirocco_88", "vp_chv_camaross_69", "vp_frd_boss302_69", "vp_pon_transam_79", "vp_chv_impala_96",
    "vp_buk_grandnational_87", "vp_dge_challenger_70", "vp_frd_focus_04", "vp_pon_solstice_06", "vp_mit_eclipse_99", "vp_chv_cobalt_05", "vp_mzd_rx8_06", "vp_duc_monster_s4r_07",
    "vp_frd_mustang_06", "vp_chv_camaro_06", "vp_chv_camaro_06_dub", "vp_dge_charger_06", "vp_dge_challenger_06", "vp_dge_challenger_06_dub", "vp_cry_300c_06", "vp_vw_golf_07",
    "vp_nsn_350z_06", "vp_mit_3000gt_99", "vp_mzd_rx7_95", "vp_nsn_skyline_99", "vp_aud_rs4_07", "vp_mit_evo_06", "vp_duc_999r_06", "vp_mbz_s600_07", "vp_mbz_s600_07_dub",
    "vp_lam_miura_71", "vp_aud_r8_08", "vp_am_vantage_07", "vp_mbz_sl65_07", "vp_chv_corvettez06_07", "vp_sln_s281_06", "vp_lam_gallardo_06", "vp_lam_murcielago_06",
    "vp_lam_murcielago_06_dub", "vp_frd_cobra_04", "vp_frd_gt_06", "vp_sln_s7_06", "vp_kaw_ninja_zx14_06", "vp_chv_police_96", "vp_citybus_07", "vpd_chv_belair_55",
    "vpd_buk_riviera_63", "vpd_chv_impala_64", "vpd_chv_malibu_65", "vpd_pon_gto_66", "vpd_chv_chevelle_70", "vpd_lr_hse_08", "vpd_lr_sport_08", "vpd_mit_eclipse_09",
    "vpd_aud_s5_08", "vpd_mit_evox_08", "vpd_cad_xlrv_08", "vpd_mbz_clk63amg_08", "vpd_mbz_cl65amg_08", "vpd_am_db9_08", "vpd_duc_1098_07",
];

/// Extra garage lists: area row id, section of the car's part file, title and camera.
/// The disc provides rows, brands and prices; the selected model fragment is previewed
/// where one exists. Tires, brakes and door-opening choices have separate drawing paths.
pub const XPARTS: [(usize, &str, &str, &str); 13] = [
    (9960, "headlight", "HEADLIGHTS", "FrontLowClose"),
    (9967, "taillight", "TAILLIGHTS", "RearLowClose"),
    (9965, "steer_whl", "STEERING WHEELS", "SteeringWheel"),
    (9966, "post_gauge", "GAUGES", "PostGauges"),
    (9969, "scissor_door", "DOORS", "SideOverviewClose"),
    // (approx.) the two below were not seen opened: their cameras are their areas' neighbours'
    (9958, "blower", "AIR SCOOPS", "FrontHighClose"),
    (9957, "side_exhaust", "SIDE EXHAUSTS", "SideOverviewClose"),
    // (not of the car's part file: the lists of Wheels.sc.xml, references `g_wh_tire`, `g_wh_brake`)
    (9963, "tires", "TIRES", "Wheels"),
    (9964, "brakes", "BRAKES", "Wheels"),
    // (the lists of the vehicle's `<car>_Exterior.gtun` the port had no rows for; their names are the
    // string table's by the section's own name. (approx.) the views are their areas' neighbours')
    (9953, "intercooler", "INTERCOOLER", "FrontLowClose"),
    (9954, "bike_cowl", "COWL", "FrontHighClose"),
    (9955, "bike_tail", "TAIL", "RearLowClose"),
    (9956, "bike_tank", "TANK", "SideOverviewClose"),
];

/// The performance bars' scales, measured on the R8 and the Ninja ZX14 of MANAGE VEHICLES
/// (references `g_mv0`, `g_mv1`, catalogue figures 0.26 / 0.42, 210 / 255, 0.62 / 0.875 and
/// 0.66 / 0.75, 236 / 258, 0.856 / 0.917): acceleration x 1.3 (approx.), top speed from
/// 100 at 0.006 a unit, handling as it stands.
pub const METER_ACCEL: f32 = 1.3;
pub const METER_SPEED: f32 = 0.006;

/// The plate editor's characters, in the movie's order (`LP_chars` of the garage movie:
/// digits, four signs, a blank, two unused places, the letters).
pub const LP_CHARS: [char; 43] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '&', '+', '*', '@', ' ', 'U', 'U', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W',
    'X', 'Y', 'Z',
];

/// A plate's character as a place in `LP_CHARS`. The records (and the plate's font) write
/// the four signs as the codes after Z (0x5B..0x5E) where the editor shows `& + * @`
/// (the save's R8: " R]G 45" reads R*G 45 on the car).
fn lp_place(ch: char) -> usize {
    match ch as u32 {
        0x5b => 10,
        0x5c => 11,
        0x5d => 12,
        0x5e => 13,
        _ => LP_CHARS.iter().rposition(|x| *x == ch).unwrap_or(14),
    }
}

/// The plate's own character for a place of `LP_CHARS`.
fn lp_plate_char(i: usize) -> char {
    match i {
        10..=13 => char::from_u32(0x5b + i as u32 - 10).unwrap_or(' '),
        i => LP_CHARS[i.min(42)],
    }
}

/// CHANGE BRAND in the performance shop (`garage.ChangeManufacturerInPerformance`,
/// 0x8269B1F0): the makers each of the thirteen parts may come from, as the logo ids of the
/// shop's rows, in the order LB / RB step through them (the tables at 0x82829260..; a part
/// with one entry has no choice). A motorcycle has one maker for the parts in `PERF_BRAND_BIKE`.
pub const PERF_BRANDS: [&[u8]; 13] = [
    &[2, 6, 8, 11, 14, 17, 19, 20, 22, 23, 24, 25, 34],
    &[2, 10, 14, 21, 22],
    &[4, 5, 6, 11, 20, 23, 24, 33, 34, 36],
    &[6, 11, 15, 20, 29, 32, 36, 37],
    &[1],
    &[4, 6, 20, 34],
    &[28, 38],
    &[3, 12, 13, 18, 20],
    &[26],
    &[6, 16, 17, 20],
    &[11, 16],
    &[9, 31],
    &[30],
];
pub const PERF_BRAND_BIKE: [(usize, u8); 6] = [(0, 27), (3, 1), (4, 1), (5, 1), (6, 27), (8, 1)];

fn part_section(slot: Slot) -> Option<&'static str> {
    Some(match slot {
        Slot::BumperF => "bumper_front",
        Slot::BumperR => "bumper_rear",
        Slot::Skirts => "skirts",
        Slot::Spoiler => "spoiler",
        Slot::Hood => "hood",
        Slot::Body => "body",
        Slot::Seats => "seats_f",
        Slot::Exhaust => "exhaust",
        Slot::Intercooler => "intercooler",
        _ => return None,
    })
}

/// A stretch of a racing line that leaves the road network (a shortcut through an alley,
/// a car park, a plaza): laid out straight at first, and given its real line through the
/// collision mesh once the city around it is loaded.
#[derive(Debug, Clone)]
pub struct Shortcut {
    /// whose line: a car index, or `usize::MAX` for the race's own line
    pub car: usize,
    /// first and last point of the stretch in that line
    pub from: usize,
    pub to: usize,
    pub done: bool,
    /// race time of the next attempt (the far end may not be loaded yet)
    pub retry: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    None,
    StartFreeRoam,
    StartRace,
    OpenRaceSelect,
    /// the pause MODES' ARCADE (false) or GOAL ATTACK (true): the mode object's +796 set,
    /// then the ARCADE menu's race type list (sub_82666760 / sub_826669C0)
    OpenArcade(bool),
    OpenGarage,
    Back,
    Quit,
    ApplyCar,
    Resume,
    RestartRace,
    /// free camera over the frozen scene; Enter saves the picture
    PhotoMode,
    MainMenu,
    /// leave the running event and cruise on
    QuitRace,
    /// the showroom's TEST DRIVE: out of the garage in the vehicle looked at
    TestDrive,
    /// the SERVICE STATION menu's FULL FIX (true) or REFILL NITRO (false), then back to the
    /// world (mcGasStationMenuForm, sub_826414A8)
    GasStation(bool),
    /// the GPS map, and the race editor on it
    OpenMap,
    OpenEditor,
}

pub struct Game {
    pub mode: Mode,
    pub prev_mode: Mode,
    pub race: Option<Race>,
    pub car_dirs: Vec<PathBuf>,
    pub selected_car: usize,
    pub current_car: usize,
    pub color_idx: usize,
    /// paint mixed in the garage (hue, saturation, brightness, each 0..1); else the swatch
    pub paint_hsv: Option<[f32; 3]>,
    pub menu_sel: usize,
    pub message: String,
    pub message_timer: f32,
    /// the current message has gone to the HUD movie's splash queue
    pub message_posted: bool,
    /// pause menu: tab (PAUSE, MODES, SETTINGS) and sub-page (0 none, 1 statistics, 2 cheats)
    pub pause_tab: usize,
    pub pause_page: u8,
    /// cheats switched on (unlimited nitro, never damage out, unlimited specials, top down view, no police, no fines)
    pub cheats: [bool; 6],
    /// a call from the mechanic on screen: title, text, seconds left
    pub tip: Option<(String, String, f32)>,
    pub best_time: Option<f32>,
    pub last_results: Vec<(String, Option<f32>)>,
    pub rng: u32,
    pub nav_up: bool,
    pub nav_down: bool,
    pub nav_left: bool,
    pub nav_right: bool,
    /// the shoulder buttons, which the rim shop uses to step between wheel brands
    /// (`uinput.l1` / `uinput.r1` in the game's own Rim.sc.xml)
    pub nav_tab_prev: bool,
    pub nav_tab_next: bool,
    pub nav_enter: bool,
    pub nav_back: bool,
    pub cfg: CarConfig,
    pub garage_row: usize,
    /// where the player is in the garage's nested menus: the menu, the cursor in it, and the
    /// way back
    pub garage_menu: usize,
    pub garage_cur: usize,
    pub garage_stack: Vec<(usize, usize)>,
    /// the part list that is open (a row of `CUSTOM_SLOTS`, counted from 1)
    pub garage_part: Option<usize>,
    /// an open part list of `XPARTS` (index) and the row under its cursor
    pub garage_xpart: Option<(usize, usize)>,
    /// LICENSE PLATES (Main.sc.xml `License_Plate`): the character under the cursor and the
    /// seven characters as places in `LP_CHARS`, while the editor is open
    pub plate_edit: Option<(usize, [usize; 7])>,
    /// DIMENSIONS: the wheel figures of a car as its record holds them (0x83a..0x842 of the
    /// 8176-byte vehicle record - a save's garage record or the car's own
    /// `resources/vehicle/configs/<car>.xcc`): ride height (3 = stock, each step up one
    /// lower), tire width (place in 195..355 by 10), rim size (inches), tire profile (place
    /// in 20..85 by 5), each front then rear. Kept here once changed or imported.
    pub dims: std::collections::HashMap<String, [u8; 8]>,
    /// the cars' own figures, read once from their `.xcc`
    pub dims_stock: std::cell::RefCell<std::collections::HashMap<String, Option<[u8; 8]>>>,
    /// the axle DIMENSIONS works on (0 front, 1 rear, 2 both)
    pub dim_axle: usize,
    /// the plates the player has written, per car, and the kind of plate (CHANGE STATE)
    pub plates: std::collections::HashMap<String, String>,
    pub plate_type: usize,
    /// the parts chosen in those lists: car -> section -> part index
    pub xparts: std::collections::HashMap<String, std::collections::HashMap<String, usize>>,
    /// livery worn by each car (car name -> set-up name)
    pub vinyls: std::collections::HashMap<String, String>,
    /// the liveries there are for the car in the garage
    pub vinyl_list: Vec<String>,
    pub vinyl_for: String,
    /// liveries made in the vinyl editor, per car
    pub liveries: std::collections::HashMap<String, crate::vinyl::Livery>,
    /// counts the edits of the custom livery (its texture is painted again)
    pub livery_rev: u32,
    /// the vinyl editor is open
    pub vinyl_edit: Option<VinylEdit>,
    /// shape groups of the vinyl editor: (name, shapes)
    pub vinyl_cats: Vec<(String, usize)>,
    /// the vinyl presets of the disc
    pub vinyl_presets: crate::vinyl::Presets,
    pub rim_dirs: Vec<PathBuf>,
    pub rim_names: Vec<String>,
    /// the disc's rim shop (`tune/ui/garage/elements/Rim*.sc.xml`): one brand
    /// tab per maker, each with the rims it sells and their real prices
    pub rim_shop: crate::rimshop::RimShop,
    /// `resource` folder -> its place in `rim_dirs`, so a rim the game's script
    /// names resolves to the geometry the port loads
    pub rim_by_name: hashbrown::HashMap<String, usize>,
    /// which brand of the rim shop is open, and which rim is highlighted in it
    pub rim_brand: usize,
    pub rim_pick: usize,
    pub dirty: bool,
    /// original UI art (lazily loaded on first HUD draw)
    pub art: Option<crate::hud::UiArt>,
    pub art_tried: bool,
    /// the original HUD movie (gauge cluster, special ability and slipstream meters);
    /// `MCLA_FLASH_HUD=0` keeps the port's own drawing
    pub flash_hud: Option<crate::flash_ui::FlashHud>,
    /// the player's view as the HUD's camera bar numbers it (0 Bumper .. 4 Boom 3; -1 none)
    /// and the opacity of the cut mask (TRANSITIONMOVIE), set by the camera each frame
    pub hud_camera: i32,
    pub cut_mask: f32,
    /// textures and strings shared by the Flash movies
    pub ui_shared: Option<crate::flash_ui::UiShared>,
    /// the minimap's frame movie (hudmap.swf)
    pub flash_map: Option<crate::flash_ui::FlashMinimap>,
    /// the original's map (`hudmap.rs`): icons, minimap target, clocks
    pub hm: Option<crate::hudmap::State>,
    /// the menu movie (pause.swf) for the pause menu
    pub flash_menu: Option<crate::flash_ui::FlashMenu>,
    /// the garage's menu movie (garage.swf)
    pub flash_garage: Option<crate::flash_ui::FlashMenu>,
    /// the menus' 3D scene: meshes of `resources/ui/meshes` and their textures
    pub ui_meshes: Option<crate::menu3d::Meshes>,
    /// the prompt bar movie (bottom of the screen in the garage)
    pub flash_prompt: Option<crate::flash_ui::FlashPrompt>,
    /// the phone (sidekick movie)
    pub flash_phone: Option<crate::flash_ui::FlashPhone>,
    /// the part list's "ADD TO CART?" question (`INSTALL_CONF_MENU`): the highlighted
    /// answer, 0 = yes
    pub cart_ask: Option<usize>,
    /// the GPS map movie (mission log)
    pub flash_navsys: Option<crate::flash_ui::FlashMenu>,
    /// the mission log on the map: tab (0 active, 1 completed), highlighted line, hidden
    pub mission_log: (usize, usize, bool),
    /// the mission log is up as a new career's lesson (`ng_mlog_active`, `ng_mlog_tutorials`):
    /// ACTIVE and TUTORIALS only, and no way out but setting the target
    pub mlog_guided: bool,
    /// the log came up from a call: the map behind it is seen from near until it is shown
    pub mlog_near: bool,
    /// the GPS map's legend is on (X)
    pub map_legend: bool,
    /// the garage's vehicle list: (showroom rather than the player's own, highlighted line)
    pub garage_list: Option<(bool, usize)>,
    /// the car the SELL VEHICLE box is asking about
    pub sell_pending: Option<usize>,
    /// the GPS map's cursor: which of the map's places it is on
    pub map_cursor: usize,
    /// the place set as the GPS target on the map (SET AS TARGET)
    pub gps_pick: Option<Vec3>,
    /// the mission made the GPS target in the mission log (its id)
    pub mission_target: Option<String>,
    /// where the map's cursor stands in the world (the map view follows it)
    pub map_cursor_pos: Option<Vec3>,
    /// the GPS cursor moved freely with the stick (`hudmap::NavCam`): where it is while it stands
    /// on no place, and the stick's release (the icon under it is then selected)
    pub map_free: Option<Vec3>,
    pub map_free_release: bool,
    /// where the dyno graph goes (from the car's garage file)
    pub dyno_rect: Option<[f32; 4]>,
    /// the garage's CART page is up: its cursor
    pub cart_open: Option<usize>,
    /// the LOADING tag: time left, whether it is up, its fade-out, and the place last seen
    pub loading_tag: f32,
    pub loading_on: bool,
    pub loading_fade: f32,
    pub loading_was_garage: bool,
    /// the songs of the playlists ("ARTIST - TITLE", genre 0..6), from the audio side
    pub songs: Vec<(String, u8)>,
    /// the statistics pages: which vehicle / race type / race / district is open
    pub stat_idx: [usize; 2],
    /// the song that has just begun, for the HUD's info line, and how long it has shown
    pub song_line: Option<(String, f32)>,
    /// the vinyl editor's clipboard (COPY / CUT LAYER)
    pub vinyl_clip: Option<crate::vinyl::Layer>,
    /// after a race: the RACE RESULTS table is up (over the race-over menu)
    pub results_table: bool,
    /// the ARCADE menu: page (0 race type, 1 race, 2 options), the type, and the cursor of
    /// each page
    pub arcade: (u8, usize, [usize; 3]),
    /// its options: opponents, difficulty, power ups, traffic, weather, time of day
    pub arcade_opts: [usize; 6],
    /// the ARCADE menu's two rows of a circuit: NUMBER OF LAPS (the choice, laps - 1) and
    /// LAST MAN OUT (`mc4_arcade_rows`)
    pub arcade_x: [usize; 2],
    /// a race whose opponent count the ARCADE options changed, and its own figure
    pub arcade_restore: Option<(usize, usize)>,
    /// GOAL ATTACK (the mode object's byte +796): the ARCADE menu entered from the pause
    /// MODES' GOAL ATTACK, and the race started from it (`mc4_goal_*`)
    pub goal_attack: bool,
    /// the races added to the favourites from the pause menu (their ids; the save's
    /// "RaceFavorites")
    pub race_favs: Vec<String>,
    /// the ARRESTED panel: seconds until it comes up (the arrest's scene first), the
    /// pursuit's length in seconds and the fine
    pub arrest_panel: Option<(f32, f32, i64)>,
    /// the panel is a ticket's: what it is for (1 reckless driving, 2 traffic violation,
    /// 4 destruction of property)
    pub ticket_for: Option<u32>,
    /// the REWARDS panel stands before the race-over menu (until CONTINUE)
    pub reward_panel: bool,
    /// the race's cars as they ran, for VIEW REPLAY, and the replay being watched
    pub replay_rec: crate::replay::Recording,
    pub replay: Option<crate::replay::Playback>,
    /// the pad's X, Y, Back and Start in the menus (the replay's legend), and the left
    /// stick's left / right held (SLOW)
    pub nav_x: bool,
    pub nav_y: bool,
    pub nav_select: bool,
    pub nav_start: bool,
    pub nav_slow: f32,
    /// GO TO GARAGE of the race-over menu: the garage as soon as the event is left
    pub go_garage: bool,
    /// the place splash after a race: the place and the seconds it still stands
    pub place_splash: Option<(usize, f32)>,
    /// the player's place in the race just run (the replay's end shows it again)
    pub race_place: usize,
    /// the finish was level with another car's: the splash reads TIED
    pub place_tied: bool,
    /// MISSION RESULTS after an event: the missions it finished (title, objectives)
    /// (title, objectives, done: false for a mission the event did not finish - stamped FAIL)
    pub mission_results: Vec<(String, Vec<String>, bool)>,
    /// the race's end in slow motion (EndOfRaceSequence of RaceOver.sch: PerspectiveForm_SetSlowMotion
    /// on form 60, 0.2 s of real time after the end-of-race speech) and the wait before it
    pub race_slow: crate::mc4::SlowMo,
    pub race_slow_wait: Option<f32>,
    /// real seconds since the player crossed the line (the race-end panels, `RACE_END_*`)
    pub race_end_t: Option<f32>,
    /// the results on show are a career race's (panels, then the cruise - no menu)
    pub career_over: bool,
    /// a new career's save boxes: 0 not shown yet, 1 the ALERT is up, 2 done; and the
    /// player chose CONTINUE WITHOUT SAVING (nothing is written this session)
    pub save_box: u8,
    /// the row the last message box was closed on
    pub box_sel: usize,
    /// a tutorial's box is up: the keys of its title and text, seconds before it shows
    /// and seconds it stays
    pub tutorial_box: Option<(String, String, f32, f32)>,
    /// no save was found: a new career, which opens on VEHICLE SELECT
    pub fresh_game: bool,
    /// VEHICLE SELECT is up: which of `STARTERS` stands there
    pub starter_pick: Option<usize>,
    /// the starter to stand there next (main puts it in the player's place)
    pub starter_swap: Option<String>,
    /// a race's opening shot from the air is on: how far through it is (0 = not)
    pub dive_k: f32,
    /// race time stepped since the last UI frame: the HUD's 3-2-1 runs on it, so it stays on the race clock
    pub ui_sim_dt: f32,
    /// a start scene before its countdown event: no HUD (the original brings the HUD - map and 3-2-1 - up
    /// with the countdown, over the scene's camera: work/career2/cap/d1_10.png)
    pub scene_hud_off: bool,
    /// the race markers' tune (`mcUIClass`, loaded on first use)
    pub mark_tune: Option<crate::racemark::MarkTune>,
    /// the free-roam icons' three `3dicons` movies (hookicons.rs, loaded on first use)
    pub hook_icons: Option<crate::hookicons::HookIcons>,
    /// the pause menu's NEXT RACE started this race: not a "first race" for the start-scene picker (the
    /// original drew from the grid list both times: work/career2/cap/README.md n1, n3)
    pub next_race_start: bool,
    /// the hangout pulled into, by its key in the string table (`hw_local_hangout1` ...),
    /// the difficulty chosen on its panel, and the difficulty an event was accepted at
    pub hangout_key: Option<String>,
    pub hangout_diff: usize,
    /// the hangout's panel (0 its own, 1 WAGER AMOUNT) with its cursor, and the slider's
    /// place (0..10)
    pub hangout_page: (u8, usize),
    pub hangout_stake: usize,
    pub event_diff: Option<usize>,
    /// a race against a red light or freeway racer (`althk.rs`)
    pub althk: crate::althk::RaceState,
    /// the race's badge is up over its opening shot
    pub start_badge_on: bool,
    /// the badge going: the popup movie fades its own `cur_alpha` out (template_StartOfRace: 10 a frame
    /// at 30 frames a second) after `cur_visibility` 0, so it is drawn on until that reaches 0
    pub start_badge_out: bool,
    /// the paint shop's PAINT OPTIONS: PARTS, the PAINT AREA worked on (`PAINT_AREAS`), and
    /// the body's own colour while another area's is in the shop's hands
    pub paint_parts: bool,
    pub paint_area: usize,
    pub paint_body: Option<(usize, Option<[f32; 3]>)>,
    /// (test hook `MCLA_RESULTS_TEST=...once`: the made-up results were shown)
    pub results_test_done: bool,
    /// every vehicle's fitted parts: the one in use has them in `cfg`, the others' are kept here
    pub cfgs: std::collections::HashMap<String, crate::vehicle_model::CarConfig>,
    /// every vehicle's paint (mixed colour, swatch, type, fade): the one of the vehicle in
    /// use lives in `paint_hsv`.. and is kept here when another is taken (`paint_for`)
    pub paints: std::collections::HashMap<String, (Option<[f32; 3]>, usize, u8, u8)>,
    pub paint_for: String,
    /// the original save's state the fields here do not cover (`crate::mc4`): the block
    /// `mc4` of the save, and a block that did not fit the types, kept as it was read
    pub mc4: Option<crate::mc4::Mc4>,
    pub mc4_raw: Option<serde_json::Value>,
    /// the vehicle in use: what its record keeps beyond the fields here (`mc4::CarLive`)
    pub car_x: crate::mc4::CarLive,
    /// the options the block keeps that live in the window's loop (camera, clock)
    pub opt_x: crate::mc4::OptLive,
    /// every vehicle's other finishes - the nine colour slots of INTERIOR COLORS and
    /// MATERIALS/PAINT (`slot_hsv`) and the rims' materials: the one of the vehicle in use
    /// lives in `slot_hsv` / `rim_mats`, the others are kept here (with `paint_for`)
    pub finishes: std::collections::HashMap<String, (Vec<Option<[f32; 3]>>, [u8; 3])>,
    /// rims an imported save names by the game's rim table index, per car: fitted (and
    /// forgotten here) when that car next stands in the garage
    pub rim_ids: std::collections::HashMap<String, i32>,
    /// TEST DRIVE (Main.sc.xml `x_pressed` of the showroom, TestDrive.sc.xml): the player is
    /// out in a showroom vehicle (index into `car_dirs`); asked for this frame; and the
    /// showroom opens again when the garage is next entered
    pub test_drive: Option<usize>,
    pub test_drive_req: bool,
    pub showroom_again: bool,
    /// the maker chosen for each performance part, per car (places in `PERF_BRANDS`)
    pub perf_brand: std::collections::HashMap<String, [u8; 13]>,
    /// a pause row chosen whose work waits for the next frame (250 NEXT RACE, 251 favourites)
    pub pause_do: Option<u8>,
    /// the garage's VINYL PACKAGES list is up: the package under the cursor
    pub vinyl_pkg: Option<usize>,
    /// CREATE/EDIT VINYLS was chosen: the editor opens this frame
    pub vinyl_open_req: bool,
    /// the ability chosen on the garage's SPECIAL ABILITIES page (0 ZONE, 1 AGRO, 2 ROAR,
    /// 3 EMP) for the driving side to take up, and the one last chosen
    pub ability_pick: Option<usize>,
    pub ability_sel: usize,
    /// the paint shop's open colour strip: the row, and the colour to go back to
    pub paint_open: Option<(usize, usize, Option<[f32; 3]>)>,
    /// INTERIOR COLORS (ColorsInt.sc.xml): the neon switch, the six colours listed (neon,
    /// gauges A and B, interior A, B, C: swatch numbers) and the row whose strip is open
    /// with the colour it had
    pub int_neon: bool,
    pub int_colors: [usize; 6],
    pub int_open: Option<(usize, usize)>,
    /// the colours mixed with EDIT COLOR SLOT for those rows (0..5 the interior's, 6..8 the
    /// rim's): hue, saturation, value; none = the swatch
    pub slot_hsv: [Option<[f32; 3]>; 9],
    /// the colour picker is open on one of those rows: the row, and the car's own paint,
    /// set aside while the picker's figures stand in `paint_hsv`
    pub pick_slot: Option<(usize, Option<[f32; 3]>)>,
    /// the colour picker is up: the custom colour to go back to
    pub paint_pick: Option<Option<[f32; 3]>>,
    /// a message box that the pause menu shows on its own panel this frame
    pub box_page: Option<crate::flash_ui::MenuPage>,
    /// the road the HUD's info line last named, and for how long
    pub info_road: (String, f32),
    /// the car the showroom's purchase box is asking about, and whether the car in
    /// the garage is traded in for it
    pub buy_pending: Option<(usize, bool)>,
    /// YES was given: the garage buys `selected_car` this frame
    pub buy_confirmed: bool,
    /// the phone's prompt line for the call on show (empty: none)
    pub phone_prompt: String,
    /// the map's road meshes (`resources/ui/meshes/*map_roads_*`)
    pub road_map: Option<crate::mapmesh::RoadMap>,
    /// the popup movie (message boxes)
    pub flash_popup: Option<crate::flash_ui::FlashMenu>,
    /// the message box on screen: its definition, the highlighted choice and what
    /// its confirming choice does
    pub message_box: Option<(crate::flash_ui::MessageBoxDef, usize, MenuAction)>,
    /// the garage view's menu placement (rotation, offset, scale; see `garagecam::View`)
    /// and the views' frame in the world, set by the camera code each frame
    pub garage_panel: Option<(f32, Vec3, [f32; 2], glam::Mat4)>,
    /// the pause movie's camera this frame, in the car's frame (eye, target,
    /// vertical field of view in degrees); the game's camera follows it
    pub pause_cam: Option<(Vec3, Vec3, f32)>,
    pub data_dir: String,
    /// the game's race definitions and the current selection
    pub race_defs: Vec<crate::races::RaceDef>,
    /// the layout actually being driven (the race itself or its easy variant)
    pub active_def: Option<crate::races::RaceDef>,
    pub race_sel: usize,
    pub current_race: Option<usize>,
    /// career: cash and reputation
    pub money: i64,
    pub rep: i64,
    /// vehicle directory names of the cars in the player's garage
    pub owned: Vec<String>,
    pub catalog: crate::catalog::Catalog,
    /// payout of the last race: (money, rep)
    pub last_reward: (i64, i64),
    /// the mission graph of the career
    pub career: crate::career::Career,
    /// headless test runs leave the save file alone
    pub no_save: bool,
    /// the running race was started by challenging a roaming racer
    pub hookman_race: bool,
    /// the named rivals and the ones picked for the next grid
    pub rivals: crate::rivals::Rivals,
    /// performance figures of every car (decide prize class and opponent difficulty)
    pub samples: crate::rewards::Samples,
    /// difficulty (0 Easy .. 3 Impossible) of the running race against its field
    pub race_diff: usize,
    /// the opponents level that follows the player's results
    pub adaptive: crate::rewards::Adaptive,
    /// rubber band step of the running race (chosen when it starts)
    pub rb_step: Option<usize>,
    pub grid_rivals: Vec<crate::rivals::Rival>,
    /// race definitions the career is waiting for (they come first among the nearby races)
    pub mission_races: Vec<usize>,
    /// the races the GPS map shows: the open career events and those already done
    pub map_races: Vec<usize>,
    /// a story scene is running: letterbox + this subtitle instead of the HUD
    pub cine: Option<String>,
    /// collectable barrels found (indices into the script's list)
    pub collected: Vec<u16>,
    /// rim size chosen in the garage (inches; 0 = stock)
    pub rim_inches: u8,
    /// hydraulics fitted in the garage
    pub hydraulics: bool,
    /// the paint type chosen in the garage (`garage.ChangePaintType`: 1 Matte, 2 Gloss,
    /// 3 Metallic, 4 Pearl, 5 Sparkle)
    pub paint_type: u8,
    /// the fade type (`garage.ChangePaintFadeType`: 0 none, 1 two-tone, 2 front to back,
    /// 3 left to right, 4 top to bottom) and the swatch of the second colour
    pub paint_fade: u8,
    pub paint_b: usize,
    /// window tint (the garage's Tint Transparency 1..10, 0 = stock glass; Tint Color)
    pub tint_level: u8,
    pub tint_color: usize,
    /// the exterior neon (the Colors menu's `garage.ToggleNeon`) and its colour
    pub neon_on: bool,
    pub neon_color: usize,
    /// the music playlist picked in the pause menu: 0 all genres, 1.. one of the seven
    pub music_genre: u8,
    /// the pause menu's options
    pub opts: Options,
    /// the HUD's map is in its larger overhead view (D-pad up / O)
    pub hud_map_big: bool,
    /// the overhead map's title (a red light race's landmark, rule_frame.md 9.2) and the script's
    /// last mapGoBig / mapGoNormal (main.rs)
    pub overhead_title: Option<String>,
    /// (p080) its icon and second text (a delivery intro: the district's emblem, "Delivery")
    pub overhead_sub: Option<(i32, String)>,
    pub map_go: bool,
    /// photo mode: nothing but this hint on screen (None while the picture is taken)
    pub photo: Option<String>,
    pub photo_on: bool,
    /// start scene of the race just set up (red light races), played instead of a grid fly-by
    pub prerace_scene: Option<String>,
    /// red light race places (map markers)
    pub redlights: Vec<Vec3>,
    /// a flag game is on
    pub flag: Option<crate::flags::FlagGame>,
    /// the results are scores, not times
    pub results_plain: bool,
    /// hangouts (map markers)
    pub hangouts: Vec<Vec3>,
    /// which of them are wager hangouts (the others run series)
    pub hangout_wager: Vec<bool>,
    /// the hangouts' pull-in scenes, in the order of `hangouts` (step 20b: their set-ups)
    pub hangout_names: Vec<String>,
    /// service stations (map markers)
    pub gas_stations: Vec<Vec3>,
    /// the race-over menu's QUICK FIX was bought: the car is to be repaired
    pub quick_fix: bool,
    /// the event list was opened at one of the four wager hangouts
    pub wager_here: bool,
    /// the WAGER AMOUNT choice is open (the row it is on)
    wager_pick: Option<usize>,
    /// the wager riding on the race: the stake (already paid) and the difficulty it bought
    pub wager: Option<(i64, usize)>,
    /// the race just paid out was a wager (its winnings count for the wager missions)
    pub wager_paid: bool,
    /// the map's own tuning (`HudMap` in tune/ui: icon colours and sizes)
    pub hudmap: std::collections::HashMap<String, Vec<f32>>,
    /// the rain's tuning (tune/rmptfx/raintuning.xml)
    pub rain_tune: std::collections::HashMap<String, f32>,
    /// a movie is playing over everything
    pub movie: Option<crate::video::Video>,
    /// set when a movie has just ended or was skipped
    pub movie_done: bool,
    /// a race has just been set up: (grid slot 0, grid yaw, grid type) for the start-line camera
    pub prerace: Option<(Vec3, f32, String)>,
    /// what the race hands the start-scene picker (prerace.rs `PickArgs`; `front_lr` is filled when the
    /// scene starts) and the places of grid slots 0 and 1
    pub prerace_args: Option<(crate::prerace::PickArgs, [Vec3; 2])>,
    /// the scene is still loading: all black
    pub cine_black: bool,
    /// results screen: state of the career event the race belonged to, and its next race
    pub event_line: String,
    pub next_event_race: Option<usize>,
    /// test runs: MCLA_NAV asked for the screenshot now
    pub shot_now: bool,
    /// where deliveries are taken up
    pub delivery_spots: Vec<Vec3>,
    /// the cruising racers of the districts: where, heading, difficulty
    pub cruiser_marks: Vec<(Vec3, Vec3, usize)>,
    /// the special hookmen for the map (`hudmap.rs`): place, heading, element subtype, icon,
    /// difficulty (time trial: 4 / icon 5, tournament: 4 / icon 7 - the CruisingHookmen scripts)
    pub map_men: Vec<(Vec3, Vec3, u8, u8, usize, bool)>,
    /// the idle camera is on: close shots of the car, the HUD away
    pub idle_cam: bool,
    /// the garages' doors (where their marker stands in the world)
    pub garage_doors: Vec<Vec3>,
    /// the call up is a racer's telephone challenge: Back accepts and opens ARCADE
    pub phone_challenge: bool,
    /// the challenge was taken: main opens the race menu
    pub phone_accept: bool,
    /// the mission's man on the road: where, his name, his car's folder
    pub rival_mark: Option<(Vec3, String, String)>,
    /// a new career's opening is still to play or playing: VEHICLE SELECT waits
    pub hold_starter: bool,
    pub series: Option<Series>,
    /// a series race is over: its RACE RESULTS stand first (`sr_results`)
    pub series_first: bool,
    /// CONTINUE SERIES is asked (`sr_continue`): the row under the cursor
    pub series_ask: Option<usize>,
    /// what is asked: 0 CONTINUE SERIES, 1 QUIT SERIES, 2 RESTART SERIES
    pub series_ask_kind: u8,
    /// SERIES STANDINGS is up over the pause menu (`sr_standings`)
    pub standings_view: bool,
    /// GPS: road route to the career's current target (free roam)
    pub gps: Option<std::sync::Arc<Vec<Vec3>>>,
    /// opponent rubber-band tunings, easiest first
    pub rubber: Vec<crate::rubber::RubberBand>,
    /// full-screen city map (M / Tab)
    pub show_map: bool,
    pub nav_map: bool,
    /// race editor (on the map): E toggles, Backspace removes the last point, R runs the race
    pub nav_edit: bool,
    pub nav_del: bool,
    pub nav_go: bool,
    /// points of the race being laid out (start first)
    pub editor: Option<Vec<Vec3>>,
    pub editor_cursor: Vec3,
    /// races made in the editor: their points
    pub user_races: Vec<Vec<[f32; 3]>>,
    /// race the player is parked at in free roam (index into `race_defs`)
    pub race_here: Option<usize>,
    /// the player is driving a car lent for a delivery
    pub borrowed: bool,
    /// difficulty step (0..3) of the running mission
    pub mission_diff: usize,
    /// performance upgrade level (0..=3) bought per car (directory name)
    pub perf: std::collections::HashMap<String, u8>,
    /// level highlighted in the garage
    pub perf_sel: u8,
    /// performance parts fitted per car: the stage (0..2) of each of the shop's parts
    pub perf_parts: std::collections::HashMap<String, Vec<u8>>,
    /// the shop's price lists per group of car (D, C, B, A): part id, stage 1 and stage 2 price
    pub perf_prices: std::collections::HashMap<char, Vec<(String, i64, i64)>>,
    /// visual part price lists of the garage: (body kit price, section -> prices by variant)
    pub vis_lists: Vec<(i64, std::collections::HashMap<String, Vec<i64>>)>,
    /// every car's part file (`tune/garage/<hash of "<car>_parts.gtun">`, the name the
    /// part menu builds at 0x826A92E0) by that hash: per section its rows in the file's
    /// order - (brand logo id, price, part index)
    /// the game's race table (`tune/race`, the file that begins "City": its `Race { Id }`
    /// entries in order): a race's place in it is the sub id of its statistics
    pub race_table: Vec<String>,
    /// TIRES and BRAKES of Wheels.sc.xml (`TireTopMenu`, `TireTopMenuBike`, `BrakeTopMenu`):
    /// (name key, brand logo id, price, index) per row
    pub wheel_lists: [Vec<(String, i32, i64, usize)>; 3],
    /// the rim list was opened from SOUTH CENTRAL RIMS (the downloadable brands)
    pub rim_dlc: bool,
    /// MATERIALS/PAINT (RimColorMatMenu): the material of spoke A, spoke B and lip (0 painted,
    /// 1 chrome, 2 brushed) and their colours (swatch numbers)
    pub rim_mats: [u8; 3],
    pub rim_cols: [usize; 3],
    pub part_files: std::collections::HashMap<u32, std::collections::HashMap<String, Vec<(i32, i64, usize)>>>,
    /// visual parts paid for, per car ("bumper_front:2")
    pub parts_owned: std::collections::HashMap<String, Vec<String>>,
    /// the performance shop is open (highlighted part)
    pub perf_shop: Option<usize>,
    /// parts were bought: the car has to be re-tuned
    pub perf_dirty: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SaveData {
    pub car: String,
    pub color: usize,
    pub cfg: CarConfig,
    pub best_time: Option<f32>,
    #[serde(default)]
    pub money: Option<i64>,
    #[serde(default)]
    pub rep: i64,
    #[serde(default)]
    pub owned: Vec<String>,
    #[serde(default)]
    pub perf: std::collections::HashMap<String, u8>,
    #[serde(default)]
    pub career: Option<crate::career::SaveState>,
    #[serde(default)]
    pub user_races: Vec<Vec<[f32; 3]>>,
    #[serde(default)]
    pub vinyls: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub paint_hsv: Option<[f32; 3]>,
    #[serde(default)]
    pub collected: Vec<u16>,
    #[serde(default)]
    pub rim_inches: u8,
    #[serde(default)]
    pub hydraulics: bool,
    /// paint type (0 = never chosen: metallic)
    #[serde(default)]
    pub paint_type: u8,
    /// fade type and the second colour's swatch
    #[serde(default)]
    pub paint_fade: u8,
    #[serde(default)]
    pub paint_b: usize,
    /// window tint: level (0 = stock glass, 1..10) and the colour's swatch
    #[serde(default)]
    pub tint_level: u8,
    #[serde(default)]
    pub tint_color: usize,
    /// exterior neon: on, and its colour's swatch
    #[serde(default)]
    pub neon_on: bool,
    #[serde(default)]
    pub neon_color: usize,
    #[serde(default)]
    pub music_genre: u8,
    #[serde(default)]
    pub opts: Options,
    #[serde(default)]
    pub perf_parts: std::collections::HashMap<String, Vec<u8>>,
    #[serde(default)]
    pub parts_owned: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    pub adaptive: crate::rewards::Adaptive,
    #[serde(default)]
    pub liveries: std::collections::HashMap<String, crate::vinyl::Livery>,
    #[serde(default)]
    pub xparts: std::collections::HashMap<String, std::collections::HashMap<String, usize>>,
    #[serde(default)]
    pub race_favs: Vec<String>,
    #[serde(default)]
    pub cfgs: std::collections::HashMap<String, crate::vehicle_model::CarConfig>,
    #[serde(default)]
    pub paints: std::collections::HashMap<String, (Option<[f32; 3]>, usize, u8, u8)>,
    #[serde(default)]
    pub finishes: std::collections::HashMap<String, (Vec<Option<[f32; 3]>>, [u8; 3])>,
    /// an imported save's rims by car (the game's rim table index) and the rim's materials
    #[serde(default)]
    pub rim_ids: std::collections::HashMap<String, i32>,
    #[serde(default)]
    pub rim_mats: Option<[u8; 3]>,
    #[serde(default)]
    pub perf_brand: std::collections::HashMap<String, [u8; 13]>,
    #[serde(default)]
    pub plates: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub dims: std::collections::HashMap<String, [u8; 8]>,
    #[serde(default)]
    pub int_neon: bool,
    #[serde(default)]
    pub int_colors: Option<[usize; 6]>,
    #[serde(default)]
    pub slot_hsv: [Option<[f32; 3]>; 9],
    /// what an original save holds beyond the fields above (`crate::mc4`), as written
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mc4: Option<serde_json::Value>,
    /// the keyboard's bindings (the port's own, `crate::keybinds`): every action by name with
    /// its keys' names; not there while the bindings are the default ones
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<std::collections::BTreeMap<String, Vec<String>>>,
}

impl Game {
    pub fn new(car_dirs: Vec<PathBuf>, current_car: usize) -> Game {
        Game {
            mode: Mode::MainMenu,
            prev_mode: Mode::FreeRoam,
            race: None,
            car_dirs,
            selected_car: current_car,
            current_car,
            color_idx: 0,
            paint_hsv: None,
            menu_sel: 0,
            message: String::new(),
            message_timer: 0.0,
            message_posted: false,
            pause_tab: 0,
            pause_page: 0,
            cheats: [false; 6],
            tip: None,
            best_time: None,
            last_results: Vec::new(),
            rng: 0xC0FFEE,
            nav_up: false,
            nav_down: false,
            nav_left: false,
            nav_right: false,
            nav_enter: false,
            nav_back: false,
            nav_tab_prev: false,
            nav_tab_next: false,
            cfg: CarConfig::default(),
            garage_row: 0,
            garage_menu: 0,
            garage_cur: 0,
            garage_stack: Vec::new(),
            garage_part: None,
            garage_xpart: None,
            plate_edit: None,
            dims: Default::default(),
            dims_stock: Default::default(),
            dim_axle: 0,
            plates: Default::default(),
            plate_type: 0,
            xparts: Default::default(),
            vinyls: Default::default(),
            vinyl_list: Vec::new(),
            vinyl_for: String::new(),
            liveries: Default::default(),
            livery_rev: 0,
            vinyl_edit: None,
            vinyl_cats: Vec::new(),
            vinyl_presets: Default::default(),
            rim_dirs: Vec::new(),
            rim_names: Vec::new(),
            rim_shop: Default::default(),
            rim_by_name: Default::default(),
            rim_brand: 0,
            rim_pick: 0,
            dirty: false,
            art: None,
            art_tried: false,
            flash_hud: None,
            hud_camera: -1,
            cut_mask: 0.0,
            ui_shared: None,
            flash_map: None,
            hm: None,
            flash_menu: None,
            flash_garage: None,
            ui_meshes: None,
            flash_prompt: None,
            flash_phone: None,
            cart_ask: None,
            flash_navsys: None,
            // (the GPS opens on the map; Y brings the mission log up)
            mission_log: (0, 0, std::env::var("MCLA_MAP_LOG").is_err()),
            map_legend: false,
            garage_list: None,
            sell_pending: None,
            map_cursor: usize::MAX,
            gps_pick: None,
            mission_target: None,
            map_cursor_pos: None,
            map_free: None,
            map_free_release: false,
            dyno_rect: None,
            cart_open: None,
            loading_tag: 0.0,
            loading_on: false,
            loading_fade: 0.0,
            loading_was_garage: false,
            songs: Vec::new(),
            stat_idx: [0; 2],
            song_line: None,
            vinyl_clip: None,
            results_table: false,
            arcade: (0, 0, [0; 3]),
            arcade_opts: ARCADE_DEFAULTS,
            arcade_x: [1, 0],
            arcade_restore: None,
            goal_attack: false,
            race_favs: Vec::new(),
            cfgs: Default::default(),
            arrest_panel: None,
            ticket_for: None,
            reward_panel: false,
            replay_rec: Default::default(),
            replay: None,
            nav_x: false,
            nav_y: false,
            nav_select: false,
            nav_start: false,
            nav_slow: 0.0,
            go_garage: false,
            place_splash: None,
            race_place: 0,
            place_tied: false,
            mission_results: Vec::new(),
            race_slow: crate::mc4::SlowMo::default(),
            race_slow_wait: None,
            race_end_t: None,
            career_over: false,
            save_box: 0,
            box_sel: 0,
            tutorial_box: None,
            fresh_game: false,
            starter_pick: None,
            starter_swap: None,
            dive_k: 0.0,
            ui_sim_dt: 0.0,
            scene_hud_off: false,
            mark_tune: None,
            hook_icons: None,
            next_race_start: false,
            hangout_key: None,
            hangout_diff: 0,
            hangout_page: (0, 0),
            hangout_stake: 5,
            event_diff: None,
            althk: Default::default(),
            start_badge_on: false,
            start_badge_out: false,
            paint_parts: false,
            paint_area: 0,
            paint_body: None,
            results_test_done: false,
            paints: Default::default(),
            finishes: Default::default(),
            paint_for: String::new(),
            mc4: None,
            mc4_raw: None,
            car_x: Default::default(),
            opt_x: Default::default(),
            rim_ids: Default::default(),
            test_drive: None,
            test_drive_req: false,
            showroom_again: false,
            perf_brand: Default::default(),
            pause_do: None,
            vinyl_pkg: None,
            vinyl_open_req: false,
            ability_pick: None,
            ability_sel: 0,
            paint_open: None,
            int_neon: false,
            int_colors: [0, 4, 4, 9, 9, 9],
            int_open: None,
            slot_hsv: [None; 9],
            pick_slot: None,
            paint_pick: None,
            box_page: None,
            info_road: (String::new(), 0.0),
            buy_pending: None,
            buy_confirmed: false,
            phone_prompt: String::new(),
            road_map: None,
            flash_popup: None,
            message_box: None,
            garage_panel: None,
            pause_cam: None,
            data_dir: String::new(),
            race_defs: Vec::new(),
            active_def: None,
            race_sel: 0,
            current_race: None,
            money: START_MONEY,
            rep: 0,
            owned: Vec::new(),
            catalog: Default::default(),
            last_reward: (0, 0),
            career: Default::default(),
            no_save: false,
            hookman_race: false,
            rivals: Default::default(),
            samples: Default::default(),
            race_diff: 1,
            adaptive: Default::default(),
            rb_step: None,
            grid_rivals: Vec::new(),
            mission_races: Vec::new(),
            map_races: Vec::new(),
            cine: None,
            collected: Vec::new(),
            rim_inches: 0,
            hydraulics: false,
            paint_type: 2,
            paint_fade: 0,
            paint_b: 1,
            tint_level: 0,
            tint_color: 1,
            neon_on: false,
            neon_color: 4,
            music_genre: 0,
            opts: Options::default(),
            hud_map_big: false,
            overhead_title: None,
            overhead_sub: None,
            map_go: false,
            photo: None,
            photo_on: false,
            prerace_scene: None,
            redlights: Vec::new(),
            flag: None,
            results_plain: false,
            hangouts: Vec::new(),
            hangout_wager: Vec::new(),
            hangout_names: Vec::new(),
            gas_stations: Vec::new(),
            quick_fix: false,
            wager_here: false,
            wager_pick: None,
            wager: None,
            wager_paid: false,
            hudmap: Default::default(),
            rain_tune: Default::default(),
            movie: None,
            movie_done: false,
            prerace: None,
            prerace_args: None,
            cine_black: false,
            event_line: String::new(),
            next_event_race: None,
            shot_now: false,
            delivery_spots: Vec::new(),
            cruiser_marks: Vec::new(),
            map_men: Vec::new(),
            idle_cam: false,
            garage_doors: Vec::new(),
            phone_challenge: false,
            phone_accept: false,
            rival_mark: None,
            hold_starter: false,
            series: None,
            series_first: false,
            series_ask: None,
            series_ask_kind: 0,
            standings_view: false,
            gps: None,
            rubber: Vec::new(),
            show_map: false,
            nav_edit: false,
            nav_del: false,
            nav_go: false,
            editor: None,
            editor_cursor: Vec3::ZERO,
            user_races: Vec::new(),
            nav_map: false,
            mlog_guided: false,
            mlog_near: false,
            race_here: None,
            perf: Default::default(),
            perf_sel: 0,
            perf_parts: Default::default(),
            perf_prices: Default::default(),
            vis_lists: Vec::new(),
            part_files: Default::default(),
            race_table: Vec::new(),
            wheel_lists: Default::default(),
            rim_dlc: false,
            rim_mats: [1, 1, 1],
            // (white, as the original's swatches open - `ref/g_rm_0`)
            rim_cols: [2, 2, 2],
            parts_owned: Default::default(),
            perf_shop: None,
            perf_dirty: false,
            borrowed: false,
            mission_diff: 0,
        }
    }

    pub fn save_path(data_dir: &str) -> PathBuf {
        // (MCLA_SAVE_FILE: another save, for tests that buy and sell)
        if let Ok(f) = std::env::var("MCLA_SAVE_FILE") {
            return PathBuf::from(f);
        }
        // (the game sets a save folder of its own at start, outside the data folder; tests and tools that
        // never call `savedir::init` keep the save beside the data they pass)
        if let Some(d) = savedir::get() {
            return d.join("mcla_save.json");
        }
        PathBuf::from(data_dir).join("mcla_save.json")
    }

    pub fn load_save(&mut self, data_dir: &str) {
        savedir::migrate_legacy(data_dir);
        self.fresh_game = !Self::save_path(data_dir).exists() || std::env::var("MCLA_NEW_GAME").is_ok();
        // (VEHICLE SELECT waits for the opening)
        self.hold_starter = self.fresh_game;
        log::info!("save: reading {} (exists {})", Self::save_path(data_dir).display(), Self::save_path(data_dir).exists());
        if let Ok(t) = std::fs::read_to_string(Self::save_path(data_dir)) {
            let parsed = serde_json::from_str::<SaveData>(&t);
            if let Err(e) = &parsed {
                log::warn!("save: {} not read: {}", Self::save_path(data_dir).display(), e);
            }
            if let Ok(sd) = parsed {
                if let Some(i) = self.car_dirs.iter().position(|p| p.file_name().map_or(false, |n| n.to_string_lossy() == sd.car)) {
                    self.current_car = i;
                    self.selected_car = i;
                }
                self.color_idx = sd.color % PALETTE.len();
                self.cfg = sd.cfg;
                self.best_time = sd.best_time;
                self.money = sd.money.unwrap_or(START_MONEY);
                self.rep = sd.rep;
                self.owned = sd.owned;
                self.perf = sd.perf;
                self.vinyls = sd.vinyls;
                self.paint_hsv = sd.paint_hsv;
                self.collected = sd.collected;
                self.rim_inches = sd.rim_inches;
                self.hydraulics = sd.hydraulics;
                self.paint_type = if (1..=5).contains(&sd.paint_type) { sd.paint_type } else { 2 };
                self.paint_fade = sd.paint_fade.min(4);
                self.paint_b = sd.paint_b;
                // (ten rows, 0..9: the original's Tint_1 .. Tint_10)
                self.tint_level = sd.tint_level.min(9);
                self.tint_color = sd.tint_color;
                self.neon_on = sd.neon_on;
                self.neon_color = sd.neon_color;
                self.music_genre = sd.music_genre;
                self.opts = sd.opts;
                self.perf_parts = sd.perf_parts;
                self.parts_owned = sd.parts_owned;
                self.adaptive = sd.adaptive;
                self.liveries = sd.liveries;
                self.xparts = sd.xparts;
                self.int_neon = sd.int_neon;
                self.plates = sd.plates;
                self.race_favs = sd.race_favs;
                self.cfgs = sd.cfgs;
                self.paints = sd.paints;
                self.finishes = sd.finishes;
                self.rim_ids = sd.rim_ids;
                if let Some(m) = sd.rim_mats {
                    self.rim_mats = m;
                }
                self.perf_brand = sd.perf_brand;
                self.dims = sd.dims;
                if let Some(c) = sd.int_colors {
                    self.int_colors = c;
                }
                self.slot_hsv = sd.slot_hsv;
                self.mc4_load(sd.mc4);
                self.opt_x.ctrl.kbd.binds = crate::keybinds::KeyBinds::from_save(sd.keyboard.as_ref());
                // (what was on the car before parts had prices is the player's)
                let key = self.dir_name(self.current_car).to_ascii_lowercase();
                for slot in CUSTOM_SLOTS {
                    if let Some(sec) = part_section(slot) {
                        let v = self.cfg.get(slot);
                        if v != 0 {
                            let tag = format!("{}:{}", sec, v);
                            let e = self.parts_owned.entry(key.clone()).or_default();
                            if !e.contains(&tag) {
                                e.push(tag);
                            }
                        }
                    }
                }
                self.user_races = sd.user_races;
                for (k, pts) in self.user_races.clone().iter().enumerate() {
                    let pts: Vec<Vec3> = pts.iter().map(|p| Vec3::from(*p)).collect();
                    // (numbered by its slot of the original's twenty where the save has it: `mc4`)
                    // (step 23: a slot made in another city is kept, not offered - sub_82250ED8)
                    if !self.mc4_editor_listed(k) {
                        continue;
                    }
                    self.add_user_race(self.mc4_editor_number(k), &pts);
                }
                if let Some(c) = sd.career.as_ref() {
                    self.career.restore(c);
                }
            }
        }
        // (the block's mission records over the career's own state, and the original's
        // restart of the missions: step 20, `mc4`)
        self.mc4_career_resume();
        self.career.event(crate::career::Ev::Rep(self.rep));
        // (a career start: South Central's set-up script runs again)
        self.opt_x.s23b.sc_setup_t = 0.0;
        self.opt_x.s23b.sc_setup_done = false;
    }

    pub fn write_save(&self, data_dir: &str) {
        if self.no_save {
            return;
        }
        if self.no_save {
            return;
        }
        let sd = SaveData { career: Some(self.career.save()), car: self.mc4_current_out().and_then(|c| self.owned.iter().find(|o| o.eq_ignore_ascii_case(&c)).cloned()).unwrap_or_else(|| self.car_dirs.get(self.current_car).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default()), color: self.color_idx, cfg: self.cfg.clone(), best_time: self.best_time, money: Some(self.money), rep: self.rep, owned: self.mc4_owned_out(), perf: self.perf.clone(), user_races: self.user_races.clone(), vinyls: self.vinyls.clone(), paint_hsv: self.paint_hsv, collected: self.collected.clone(), rim_inches: self.rim_inches, hydraulics: self.hydraulics, paint_type: self.paint_type, paint_fade: self.paint_fade, paint_b: self.paint_b, tint_level: self.tint_level, tint_color: self.tint_color, neon_on: self.neon_on, neon_color: self.neon_color, music_genre: self.music_genre, opts: self.opts, perf_parts: self.perf_parts.clone(), parts_owned: self.parts_owned.clone(), adaptive: self.adaptive, liveries: self.liveries.clone(), xparts: self.xparts.clone(), race_favs: self.race_favs.clone(), cfgs: self.cfgs.clone(), paints: { let mut m = self.paints.clone(); if !self.paint_for.is_empty() { m.insert(self.paint_for.clone(), (self.paint_hsv, self.color_idx, self.paint_type, self.paint_fade)); } m }, finishes: { let mut m = self.finishes.clone(); if !self.paint_for.is_empty() { m.insert(self.paint_for.clone(), (self.slot_hsv.to_vec(), self.rim_mats)); } m }, rim_ids: self.rim_ids.clone(), rim_mats: Some(self.rim_mats), perf_brand: self.perf_brand.clone(), plates: self.plates.clone(), dims: self.dims.clone(), int_neon: self.int_neon, int_colors: Some(self.int_colors), slot_hsv: self.slot_hsv, mc4: self.mc4_save(), keyboard: self.opt_x.ctrl.kbd.binds.to_save() };
        if let Ok(t) = serde_json::to_string_pretty(&sd) {
            let path = Self::save_path(data_dir);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, t);
        }
    }

    /// A race laid out in the race editor: starts at the first point (two columns of
    /// grid slots behind it), runs through the others in order.
    pub fn add_user_race(&mut self, number: usize, pts: &[Vec3]) -> Option<usize> {
        if pts.len() < 2 {
            return None;
        }
        let dir = Vec3::new(pts[1].x - pts[0].x, 0.0, pts[1].z - pts[0].z).normalize_or(Vec3::X);
        let right = Vec3::new(-dir.z, 0.0, dir.x);
        let yaw = (-dir.x).atan2(-dir.z);
        let mut grid = Vec::new();
        for row in 0..4 {
            for col in [-1.0f32, 1.0] {
                grid.push((pts[0] + right * col * 2.4 - dir * (row as f32 * 8.5 + 2.0), yaw));
            }
        }
        let def = crate::races::RaceDef {
            id: format!("user/race_editor_{number}"),
            name: format!("my race {number}"),
            race_type: "Ordered".into(),
            sub_type: "Ordered".into(),
            district: "Race Editor".into(),
            laps: 1,
            beacon: pts[0],
            grid,
            checkpoints: pts[1..].to_vec(),
            opponents: 3,
            grid_type: "2x4_Race_Grid".into(),
            ..Default::default()
        };
        match self.race_defs.iter().position(|d| d.id == def.id) {
            Some(i) => {
                self.race_defs[i] = def;
                Some(i)
            }
            None => {
                self.race_defs.push(def);
                Some(self.race_defs.len() - 1)
            }
        }
    }

    /// The player's paint: the garage's own mix, else the chosen swatch.
    pub fn paint_color(&self) -> [f32; 4] {
        // (MCLA_COLOR=n: a palette swatch for screenshots)
        if let Some(i) = std::env::var("MCLA_COLOR").ok().and_then(|v| v.parse::<usize>().ok()) {
            return PALETTE[i % PALETTE.len()];
        }
        // (a part's colour is in the shop's hands: the body keeps its own)
        if let (true, Some((idx, hsv))) = (self.paint_area != 0, self.paint_body) {
            return match hsv {
                Some(h) => {
                    let c = paint_hsv_to_rgb(h);
                    [c[0], c[1], c[2], 1.0]
                }
                None => PALETTE[idx % PALETTE.len()],
            };
        }
        match self.pick_slot.map_or(self.paint_hsv, |p| p.1) {
            Some([h, s, v]) => {
                let h6 = h.rem_euclid(1.0) * 6.0;
                let (i, f) = (h6.floor(), h6 - h6.floor());
                let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
                let rgb = match i as i32 {
                    0 => [v, t, p],
                    1 => [q, v, p],
                    2 => [p, v, t],
                    3 => [p, q, v],
                    4 => [t, p, v],
                    _ => [v, p, q],
                };
                [rgb[0], rgb[1], rgb[2], 1.0]
            }
            None => PALETTE[self.color_idx % PALETTE.len()],
        }
    }

    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x % 100000) as f32 / 100000.0
    }

    fn dir_name(&self, i: usize) -> String {
        self.car_dirs.get(i).and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }

    /// The car the career starts with is always in the garage.
    pub fn ensure_owned(&mut self) {
        let cur = self.dir_name(self.current_car);
        if !cur.is_empty() && !self.owned.iter().any(|o| o.eq_ignore_ascii_case(&cur)) {
            self.owned.push(cur);
        }
    }

    /// What car `i` fetches when sold or traded in (0x826A74BC, the same at the showroom's
    /// purchase with trade 0x826AF6B4): four fifths of the car's price less one plus what
    /// was spent on it (a 16-bit sum in its purchase record), and nothing at all while the
    /// saved flag `tPAR` is set (the 100 % save: TRADE IN VALUE: $0 on every car).
    /// The sum is the purchase record's (`mc4`: 1 on a model nothing was spent on).
    pub fn trade_in(&self, i: usize) -> i64 {
        crate::mc4::trade_in_value(self.mc4_spent_word(&self.dir_name(i)), self.price_of(i).0, self.parts_free()).max(0)
    }

    /// Sell car `i` for its trade-in value and stand another owned car in the garage.
    pub fn sell_car(&mut self, i: usize) {
        let name = self.dir_name(i);
        if self.owned.len() <= 1 || !self.is_owned(i) {
            return;
        }
        self.owned.retain(|o| !o.eq_ignore_ascii_case(&name));
        self.money += self.trade_in(i);
        if let Some(next) = (0..self.car_dirs.len()).find(|&k| self.is_owned(k)) {
            self.selected_car = next;
            if self.current_car == i {
                self.current_car = next;
            }
        }
        self.garage_list = self.garage_list.map(|(s, _)| (s, 0));
        self.dirty = true;
    }

    pub fn is_owned(&self, i: usize) -> bool {
        let n = self.dir_name(i);
        self.owned.iter().any(|o| o.eq_ignore_ascii_case(&n))
    }

    /// Price and the reputation needed to buy car `i` (0 when the catalogue does not list it).
    pub fn price_of(&self, i: usize) -> (i64, i64) {
        match self.catalog.get(&self.dir_name(i)) {
            // (the group is a level of the Unlocking system - C0, B0, A0: `mc4`)
            Some(c) => (c.price, self.mc4_car_need(&self.dir_name(i), &c.unlock_group)),
            None => (0, 0),
        }
    }

    /// Buy car `i` if it is unlocked and affordable. Returns false (with a message) otherwise.
    pub fn try_buy(&mut self, i: usize) -> bool {
        if self.is_owned(i) {
            return true;
        }
        let (price, rep) = self.price_of(i);
        // (a car the game does not offer: RMR_CANT_BUY_UNPURCHASED_DLC - sub_826AFFD8)
        if !self.mc4_vehicle_available(&self.dir_name(i)) {
            let text = self.career.strings.get("RMR_CANT_BUY_UNPURCHASED_DLC").map(|s| s.to_uppercase()).unwrap_or_else(|| "LOCKED".into());
            self.show_message(&text);
            return false;
        }
        // (a DUB car comes to the showroom with its tournament's Hard completion: sub_826BF068)
        if self.mc4_showroom_locked(&self.dir_name(i)) {
            let text = self.career.strings.get("RMR_CANT_BUY_LOCKED_CAR").map(|s| s.to_uppercase()).unwrap_or_else(|| "LOCKED".into());
            self.show_message(&text);
            return false;
        }
        if self.rep < rep {
            self.show_message(&format!("LOCKED - NEEDS {} REP", rep));
            return false;
        }
        if self.money < price {
            self.show_message(&format!("NOT ENOUGH CASH - ${} NEEDED", price));
            return false;
        }
        self.money -= price;
        let n = self.dir_name(i);
        // (the slot's record, from the vehicle's own set-up file: `mc4`)
        self.mc4_car_added(&n, None);
        self.owned.push(n);
        self.show_message(&format!("PURCHASED FOR ${}", price));
        true
    }

    /// Read the performance shop's price lists (`tune/garage`: files of 13 parts with two
    /// prices each; there is one list per group of car, told apart by their level).
    pub fn load_perf_prices(&mut self, data_dir: &str) {
        // (the rain's tuning: "Name value" lines)
        if let Ok(t) = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/rmptfx/raintuning.xml")) {
            for l in t.lines() {
                let mut tok = l.split_whitespace();
                if let (Some(k), Some(v)) = (tok.next(), tok.next().and_then(|x| x.parse::<f32>().ok())) {
                    self.rain_tune.insert(k.to_string(), v);
                }
            }
        }
        // (and the map's tuning: "Name v v v v" lines of the HudMap block)
        if let Ok(rd) = std::fs::read_dir(std::path::Path::new(data_dir).join("tune/ui")) {
            for e in rd.flatten() {
                if e.metadata().map_or(true, |m| m.len() > 8000 || !m.is_file()) {
                    continue;
                }
                let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
                if !t.contains("HudMap {") {
                    continue;
                }
                for l in t.lines() {
                    let mut tok = l.split_whitespace();
                    let Some(k) = tok.next() else { continue };
                    let v: Vec<f32> = tok.filter_map(|x| x.parse().ok()).collect();
                    if !v.is_empty() {
                        self.hudmap.insert(k.to_string(), v);
                    }
                }
            }
        }
        let Ok(rd) = std::fs::read_dir(std::path::Path::new(data_dir).join("tune/garage")) else { return };
        for e in rd.flatten() {
            if e.metadata().map_or(true, |m| m.len() > 600 || !m.is_file()) {
                continue;
            }
            let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
            let tok: Vec<&str> = t.split_whitespace().collect();
            if tok.first() != Some(&"13") || tok.len() < 1 + 13 * 4 {
                continue;
            }
            let list: Vec<(String, i64, i64)> = (0..13).filter_map(|k| Some((tok[1 + k * 4].to_string(), tok[3 + k * 4].parse().ok()?, tok[4 + k * 4].parse().ok()?))).collect();
            if list.len() != 13 || !list[0].0.starts_with("gp_") {
                continue;
            }
            let group = match (list[0].1, list[0].2) {
                (400, 600) => 'D',
                (600, _) => 'C',
                (880, _) => 'B',
                (1320, _) => 'A',
                _ => continue,
            };
            self.perf_prices.entry(group).or_insert(list);
        }
        if let Ok(t) = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/ui/garage/Wheels.sc.xml")) {
            // (what is commented out is not on the list: the brakes' stock row)
            let mut clean = String::new();
            let mut rest = t.as_str();
            while let Some(a) = rest.find("<!--") {
                clean.push_str(&rest[..a]);
                rest = rest[a..].find("-->").map_or("", |b| &rest[a + b + 3..]);
            }
            clean.push_str(rest);
            let value = |block: &str, name: &str| -> Option<String> {
                let at = block.find(&format!("name=\"{name}\""))?;
                let e = block[at..].find("expr=\"")? + at + 6;
                Some(block[e..e + block[e..].find('"')?].trim_matches('\'').to_string())
            };
            for (k, id) in ["TireTopMenu\"", "TireTopMenuBike\"", "BrakeTopMenu\""].iter().enumerate() {
                let Some(a) = clean.find(&format!("<state id=\"{id}")) else { continue };
                let body = &clean[a..];
                let end = body[1..].find("<state id=").map_or(body.len(), |e| e + 1);
                for block in body[..end].split("<state>").skip(1) {
                    if let (Some(name), Some(brand), Some(price)) = (value(block, "fs_text1"), value(block, "fs_text3"), value(block, "price")) {
                        self.wheel_lists[k].push((name, brand.parse().unwrap_or(0), price.parse().unwrap_or(0), value(block, "index").and_then(|v| v.parse().ok()).unwrap_or(0)));
                    }
                }
            }
            log::info!("garage: {} tires, {} bike tires, {} brakes", self.wheel_lists[0].len(), self.wheel_lists[1].len(), self.wheel_lists[2].len());
        }
        if let Ok(rd) = std::fs::read_dir(std::path::Path::new(data_dir).join("tune/race")) {
            for e in rd.flatten() {
                let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
                if t.trim_start().starts_with("City") {
                    self.race_table = t.lines().filter_map(|l| { let mut it = l.split_whitespace(); (it.next() == Some("Id")).then(|| it.next().unwrap_or("").to_string()) }).collect();
                }
            }
        }
        // the visual parts: one list per car ("<!-- bumper_front --> 5", then name, flag, price,
        // index per variant). Their file names do not say which car; the body kit's price does:
        // it is a fifth of the car's.
        if let Ok(rd) = std::fs::read_dir(std::path::Path::new(data_dir).join("tune/garage")) {
            for e in rd.flatten() {
                if e.metadata().map_or(true, |m| m.len() > 4000 || !m.is_file()) {
                    continue;
                }
                let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
                // DUB editions begin with their first available section (often
                // steering wheels); they have no bumper_front section at all.
                if !t.trim_start().starts_with("<!--") {
                    continue;
                }
                let mut secs: std::collections::HashMap<String, Vec<i64>> = Default::default();
                let mut rows: std::collections::HashMap<String, Vec<(i32, i64, usize)>> = Default::default();
                for part in t.split("<!--").skip(1) {
                    let Some((name, rest)) = part.split_once("-->") else { continue };
                    let tok: Vec<&str> = rest.split_whitespace().collect();
                    let n: usize = tok.first().and_then(|v| v.parse().ok()).unwrap_or(0);
                    // (a row: name, brand, price, part index)
                    let list: Vec<(i32, i64, usize)> = (0..n).filter_map(|k| Some((tok.get(2 + k * 4)?.parse().ok()?, tok.get(3 + k * 4)?.parse().ok()?, tok.get(4 + k * 4)?.parse().ok()?))).collect();
                    if list.len() == n && n > 0 {
                        rows.insert(name.trim().to_string(), list);
                    }
                    let prices: Vec<i64> = (0..n).filter_map(|k| tok.get(3 + k * 4).and_then(|v| v.parse().ok())).collect();
                    if prices.len() == n && n > 0 {
                        secs.insert(name.trim().to_string(), prices);
                    }
                }
                if let Some(h) = e.file_name().to_string_lossy().strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()) {
                    self.part_files.insert(h, rows);
                }
                let base = secs.get("body").and_then(|b| b.get(1)).copied().unwrap_or(0);
                if base > 0 {
                    self.vis_lists.push((base, secs));
                }
            }
            self.vis_lists.sort_by_key(|l| l.0);
        }
        log::info!("garage: {} visual part price lists", self.vis_lists.len());
        log::info!("performance shop: price lists for groups {:?}", {
            let mut g: Vec<char> = self.perf_prices.keys().copied().collect();
            g.sort();
            g
        });
    }

    /// Price of variant `v` of a visual part on car `i` (0 = stock or unknown).
    /// The rows of car `i`'s part file for a slot: (brand, price, part index) in the list's
    /// order, the stock part first.
    pub fn part_entries(&self, i: usize, slot: Slot) -> Option<&Vec<(i32, i64, usize)>> {
        self.part_entries_sec(i, part_section(slot)?)
    }

    /// The rows of list `k` of `XPARTS` for the car in the garage: (label, brand logo id,
    /// price, part index). The part file's rows are STOCK and "n OF m"; tires and brakes
    /// carry their names (string keys of Wheels.sc.xml).
    pub fn xpart_rows(&self, k: usize) -> Vec<(String, i32, i64, usize)> {
        let (_, sec, _, _) = XPARTS[k.min(XPARTS.len() - 1)];
        let free = self.parts_free();
        let car = self.dir_name(self.current_car).to_ascii_lowercase();
        let owned = |index: usize| self.xparts.get(&car).and_then(|m| m.get(sec)).copied() == Some(index)
            || self.parts_owned.get(&car).is_some_and(|v| v.contains(&format!("{sec}:{index}")));
        if sec == "tires" || sec == "brakes" {
            let bike = self.catalog.get(&self.dir_name(self.current_car)).map_or(false, |c| c.class.to_ascii_lowercase().contains("bike"));
            let list = &self.wheel_lists[if sec == "brakes" { 2 } else if bike { 1 } else { 0 }];
            return list.iter().map(|(key, brand, price, index)| (self.career.strings.get(key).filter(|v| !v.contains('<')).map(|v| v.to_uppercase()).unwrap_or_else(|| key.to_uppercase()), *brand, if free || owned(*index) { 0 } else { *price }, *index)).collect();
        }
        let ent = self.part_entries_sec(self.current_car, sec).cloned().unwrap_or_default();
        let n = ent.len();
        ent.iter().enumerate().map(|(i, r)| (if i == 0 { "STOCK".to_string() } else { format!("{} OF {}", i, n - 1) }, r.0, if i == 0 || free || owned(r.2) { 0 } else { r.1 }, r.2)).collect()
    }

    pub fn part_entries_sec(&self, i: usize, sec: &str) -> Option<&Vec<(i32, i64, usize)>> {
        let dir = self.dir_name(i).to_ascii_lowercase();
        // A DUB edition's own file deliberately contains only the sections it can
        // customise. Falling back to its parent offers parts absent from its model.
        [dir.clone(), dir.replace("_dub", "")].iter()
            .find_map(|d| self.part_files.get(&mcla_formats::hash::joaat(&format!("{d}_parts.gtun"))))
            .and_then(|f| f.get(sec))
    }

    /// The vehicles VEHICLE STATS lists (reference `x00`: the game's vehicle table from its
    /// start - 1983 GOLF/GTI, 1998 240SX ... - not the garage): those with a figure in an
    /// imported save, in the table's order; with none, the garage.
    pub fn stat_vehicles(&self) -> Vec<usize> {
        let v: Vec<usize> = VEHICLE_TABLE
            .iter()
            .filter(|n| self.career.ostats.contains_key(&format!("vkDT/{n}")) || self.career.ostats.contains_key(&format!("vWIN/{n}")))
            .filter_map(|n| (0..self.car_dirs.len()).find(|i| self.dir_name(*i).eq_ignore_ascii_case(n)))
            .collect();
        if v.is_empty() { (0..self.car_dirs.len()).filter(|i| self.is_owned(*i)).collect() } else { v }
    }

    /// The wheel figures of car `i` (see `dims`): the player's, else the car's own.
    pub fn car_dims(&self, i: usize) -> [u8; 8] {
        let car = self.dir_name(i).to_ascii_lowercase();
        if let Some(d) = self.dims.get(&car) {
            return *d;
        }
        let stock = *self.dims_stock.borrow_mut().entry(car.clone()).or_insert_with(|| {
            let d = std::fs::read(std::path::Path::new(&self.data_dir).join("resources/vehicle/configs").join(format!("{car}.xcc"))).ok()?;
            d.get(0x83a..0x842).and_then(|b| b.try_into().ok())
        });
        stock.unwrap_or([3, 3, 4, 4, 17, 17, 6, 6])
    }

    /// Each vehicle has its own paint: when the vehicle in use changes, the paint of the one
    /// left is put by and the new one's taken out.
    pub fn sync_car_paint(&mut self) {
        let name = self.dir_name(self.current_car).to_ascii_lowercase();
        if name.is_empty() || name == self.paint_for {
            return;
        }
        if !self.paint_for.is_empty() {
            self.paints.insert(self.paint_for.clone(), (self.paint_hsv, self.color_idx, self.paint_type, self.paint_fade));
            self.finishes.insert(self.paint_for.clone(), (self.slot_hsv.to_vec(), self.rim_mats));
            // (and what its record keeps: `mc4`)
            self.mc4_leave_car();
        }
        // (its cabin's, gauges' and rims' colours and the rims' materials with it)
        if let Some((slots, mats)) = self.finishes.get(&name).cloned() {
            for (k, v) in slots.into_iter().take(9).enumerate() {
                self.slot_hsv[k] = v;
            }
            self.rim_mats = mats;
        }
        if let Some((hsv, idx, kind, fade)) = self.paints.get(&name).copied() {
            self.paint_hsv = hsv;
            self.color_idx = idx;
            self.paint_type = kind;
            self.paint_fade = fade;
        }
        self.mc4_take_car(&name);
        self.paint_for = name;
    }

    /// An imported save names each car's rim by the game's rim table index: the shop's entry
    /// of that index, its resource the port's rim. Fits the current car's (once); true if
    /// the rim changed.
    pub fn apply_imported_rim(&mut self) -> bool {
        if self.rim_by_name.is_empty() {
            return false;
        }
        let Some(id) = self.rim_ids.remove(&self.dir_name(self.current_car).to_ascii_lowercase()) else { return false };
        log::info!("imported rim: car {} id {} -> {:?} at {:?}", self.dir_name(self.current_car), id, self.rim_shop.entry(id).map(|e| e.resource.clone()), self.rim_shop.entry(id).and_then(|e| self.rim_by_name.get(&e.resource)));
        match self.rim_shop.entry(id).and_then(|e| self.rim_by_name.get(&e.resource)).copied() {
            Some(at) if self.cfg.rim != at => {
                self.cfg.set(Slot::Wheels, at);
                true
            }
            _ => false,
        }
    }

    /// The parts of the garage's other lists (`xparts`) as the car's model takes them
    /// (`CarConfig::extra`): what the vehicle in use has, and - while one of those lists is
    /// open - the part under its cursor, so that it shows on the car as the game shows it.
    pub fn sync_extra_parts(&mut self) {
        let car = self.dir_name(self.current_car).to_ascii_lowercase();
        let mut extra: std::collections::BTreeMap<String, usize> = self.xparts.get(&car).map(|m| m.iter().filter(|(k, v)| **v != 0 && (crate::vehicle_model::EXTRA_PARTS.iter().any(|e| e.0 == k.as_str()) || crate::vehicle_model::EXTRA_FLAGS.contains(&k.as_str()))).map(|(k, v)| (k.clone(), *v)).collect()).unwrap_or_default();
        if let (Some((k, cur)), true) = (self.garage_xpart, self.selected_car == self.current_car) {
            let sec = XPARTS[k.min(XPARTS.len() - 1)].1;
            if crate::vehicle_model::EXTRA_PARTS.iter().any(|e| e.0 == sec) || sec == "intercooler" {
                match self.xpart_rows(k).get(cur).map(|r| r.3) {
                    Some(0) | None => { extra.remove(sec); }
                    Some(v) => { extra.insert(sec.to_string(), v); }
                }
            }
        }
        // (the convertible top as the options have it: the player's vehicle starts with it, and
        // CONVERTIBLE TOP changes it - sub_823404E0, sub_8264C6F0)
        if self.mc4.as_ref().map_or(false, |m| m.options.convertible_top_down) {
            extra.insert(crate::vehicle_model::ROOF_DOWN.to_string(), 1);
        }
        if self.cfg.extra != extra {
            self.cfg.extra = extra;
        }
    }

    /// The plate of the player's car, if the player wrote one: what the editor shows while
    /// it is open, else what was accepted.
    pub fn player_plate(&self) -> Option<String> {
        // (a vehicle lent for a delivery or a test drive keeps its own plate)
        if self.borrowed || self.test_drive.is_some() {
            return None;
        }
        if let Some((_, idx)) = self.plate_edit {
            return Some(idx.iter().map(|i| lp_plate_char(*i)).collect());
        }
        self.plates.get(&self.dir_name(self.current_car).to_ascii_lowercase()).cloned()
    }

    /// The part menu's rule (0x826A9710): with the saved flag `tPAR` set every part of the
    /// lists is free. (The 100 % save has it; what sets it was not read.)
    pub fn parts_free(&self) -> bool {
        self.career.stats.get("tPAR").map_or(false, |v| *v != 0)
    }

    pub fn part_price(&self, i: usize, slot: Slot, v: usize) -> i64 {
        // Rims are not in the per-car price lists: the rim shop prices each one
        // itself (`price` in Rim*.sc.xml), by the resource it names.
        if slot == Slot::Wheels {
            // (the free-parts flag covers the wheel lists too: reference `g_wh_1`, rims at $0)
            return if self.parts_free() { 0 } else { self.rim_price(v) };
        }
        let Some(sec) = part_section(slot) else { return 0 };
        if v == 0 || self.parts_free() {
            return 0;
        }
        // the car's own list, by the part's index
        if let Some(e) = self.part_entries(i, slot) {
            return e.iter().find(|r| r.2 == v).map_or(0, |r| r.1);
        }
        if self.vis_lists.is_empty() {
            return 0;
        }
        let (price, _) = self.price_of(i);
        let want = (price as f64 * 0.2) as i64;
        let Some(list) = self.vis_lists.iter().filter(|l| l.1.contains_key(sec)).min_by_key(|l| (l.0 - want).abs()) else { return 0 };
        let p = &list.1[sec];
        p.get(v).or(p.last()).copied().unwrap_or(0)
    }

    /// What the rim in `rim_dirs` slot `v` costs. Entry 0 is the car's own
    /// stock wheels, which the shop does not sell.
    pub fn rim_price(&self, v: usize) -> i64 {
        self.rim_index(v).and_then(|i| self.rim_shop.entry(i)).map_or(0, |r| r.price)
    }

    /// The game's rim-table index of the rim in `rim_dirs` slot `v`, if the
    /// rim shop sells it. The scripts price and record rims by this, not by
    /// their place in the port's list. Entry 0 is the stock set, not sold.
    pub fn rim_index(&self, v: usize) -> Option<i32> {
        let name = self.rim_dirs.get(v)?.file_name()?.to_string_lossy().to_string();
        self.rim_shop.brands.iter().flat_map(|b| &b.rims).find(|r| r.resource == name).map(|r| r.index)
    }

    fn part_is_owned(&self, i: usize, slot: Slot, v: usize) -> bool {
        if slot == Slot::Wheels {
            return self.rim_owned(i, v);
        }
        let Some(sec) = part_section(slot) else { return true };
        v == 0 || self.parts_owned.get(&self.dir_name(i).to_ascii_lowercase()).map_or(false, |o| o.contains(&format!("{}:{}", sec, v)))
    }

    /// A rim the player has already paid for. The DLC rims are free, and so is
    /// the car's own stock set.
    fn rim_owned(&self, i: usize, v: usize) -> bool {
        let Some((_, brand, entry)) = self.rim_entry(v) else { return true };
        entry.price == 0
            || brand.shop == crate::rimshop::Shop::Dlc
            || self.parts_owned.get(&self.dir_name(i).to_ascii_lowercase()).map_or(false, |o| o.contains(&format!("rim:{}", entry.index)))
    }

    /// The rim shop row for `rim_dirs` slot `v`, with the brand that sells it.
    fn rim_entry(&self, v: usize) -> Option<(usize, &crate::rimshop::RimBrand, &crate::rimshop::RimEntry)> {
        let name = self.rim_dirs.get(v)?.file_name()?.to_string_lossy().to_string();
        self.rim_shop.brands.iter().enumerate().find_map(|(i, b)| b.rims.iter().find(|r| r.resource == name).map(|r| (i, b, r)))
    }

    /// Pay for the visual parts chosen in the garage that are not the player's yet.
    pub fn try_buy_parts(&mut self) -> bool {
        let i = self.selected_car;
        let mut cost = 0;
        let mut new: Vec<String> = Vec::new();
        for slot in CUSTOM_SLOTS {
            let v = self.cfg.get(slot);
            if self.part_is_owned(i, slot, v) {
                continue;
            }
            cost += self.part_price(i, slot, v);
            // a rim is remembered by the game's own rim-table index, so the save
            // survives any change to the order the port lists them in; every other
            // part by the section of the per-car price list it comes from
            new.push(if slot == Slot::Wheels {
                match self.rim_index(v) {
                    Some(index) => format!("rim:{}", index),
                    None => continue,
                }
            } else {
                match part_section(slot) {
                    Some(sec) => format!("{}:{}", sec, v),
                    None => continue,
                }
            });
        }
        if new.is_empty() {
            return true;
        }
        if self.money < cost {
            self.show_message(&format!("NOT ENOUGH CASH - ${} NEEDED", cost));
            return false;
        }
        self.money -= cost;
        self.mc4_parts_bought(&self.dir_name(i), &new, cost);
        self.parts_owned.entry(self.dir_name(i).to_ascii_lowercase()).or_default().extend(new);
        if cost > 0 {
            self.show_message(&format!("PARTS FITTED FOR ${}", cost));
        }
        true
    }

    /// Fit one of the garage's extra-part rows, charging only on first purchase.
    pub fn try_buy_xpart(&mut self, k: usize, row: usize) -> bool {
        let Some((_, sec, _, _)) = XPARTS.get(k).copied() else { return false };
        let Some((_, _, price, index)) = self.xpart_rows(k).get(row).cloned() else { return false };
        if price > self.money {
            let message = self.career.strings.get("G_PerformanceOverBudget")
                .map(|s| s.to_uppercase()).unwrap_or_else(|| "NOT ENOUGH CASH".into());
            self.show_message(&message);
            return false;
        }
        let car = self.dir_name(self.current_car).to_ascii_lowercase();
        self.money -= price;
        let key = format!("{sec}:{index}");
        self.mc4_parts_bought(&car, &[key.clone()], price);
        let owned = self.parts_owned.entry(car.clone()).or_default();
        if !owned.contains(&key) { owned.push(key); }
        self.xparts.entry(car).or_default().insert(sec.to_string(), index);
        self.dirty = true;
        true
    }

    /// What a fix of the current car costs (the game's SetQuickFixPrice, 0x8269ea88): nothing
    /// while the body is whole and the damage under one half, else a twentieth of the car's
    /// price times its damage, $125 at least.
    pub fn fix_price(&self, damage: f32, dented: bool) -> i64 {
        if !dented && damage <= 0.5 {
            return 0;
        }
        let price = self.catalog.get(&self.dir_name(self.current_car)).map_or(20000, |c| c.price);
        (((price / 20) as f32 * damage.clamp(0.0, 1.0)) as i64).max(125)
    }

    /// What can be wagered for a race of difficulty `d` (0 easy .. 2 hard): what such a race
    /// is worth to the player's car (the amounts on the original's form come out of its
    /// scripts' variables; this is the game's own prize formula).
    pub fn wager_amount(&self, d: usize) -> i64 {
        (crate::rewards::race_money(self.rep, crate::rewards::perf_class(self.player_perf()), d, 10.0) as i64).max(100)
    }

    /// A hookman's class restriction the player's car does not meet (the mission's id names
    /// the class: BEAT_SH_02_BCLASS, MUSCLE_CHAMP ...): the game's sentence for it.
    pub fn class_restriction(&self) -> Option<String> {
        let info = self.catalog.get(&self.dir_name(self.current_car))?;
        // (the rival on the road is the one of the oldest hookman mission: his rule counts)
        let rival = self.career.hookman_mission().map(|m| m.to_ascii_uppercase());
        for (id, text) in self.career.restrictions().into_iter().filter(|(id, _)| rival.as_deref() == Some(id.as_str())) {
            let ok = if let Some(g) = ["ACLASS", "BCLASS", "CCLASS", "DCLASS"].iter().find(|g| id.contains(*g)) {
                info.group.to_ascii_uppercase().starts_with(&g[..1])
            } else if id.contains("MOTORCYCLE") {
                info.class.eq_ignore_ascii_case("SPORTBIKE")
            } else if let Some(c) = ["MUSCLE", "TUNER", "EXOTIC", "LUXURY"].iter().find(|c| id.contains(*c)) {
                info.class.eq_ignore_ascii_case(c)
            } else {
                true
            };
            if !ok {
                return Some(text.to_uppercase());
            }
        }
        None
    }

    /// The shop's parts and prices for car `i`.
    pub fn perf_list(&self, i: usize) -> Vec<(String, i64, i64)> {
        let group = self.catalog.get(&self.dir_name(i)).map(|c| c.group.clone()).unwrap_or_else(|| "D0".into());
        let letter = group.chars().next().unwrap_or('D').to_ascii_uppercase();
        let mut list = self.perf_prices.get(&letter).or_else(|| self.perf_prices.get(&'D')).cloned().unwrap_or_default();
        // (a stage the model's purchase record has is free: the shop's list builder zeroes
        // its price - sub_826A8640 before 0x826A89A8, level from sub_8269B6D8)
        let car = self.dir_name(i).to_ascii_lowercase();
        for (k, row) in list.iter_mut().enumerate() {
            let own = self.perf_owned(&car, k);
            if own >= 1 {
                row.1 = 0;
            }
            if own >= 2 {
                row.2 = 0;
            }
        }
        list
    }

    /// Stage (0..2) of each performance part of car `i`.
    pub fn perf_stages(&self, i: usize) -> Vec<u8> {
        let key = self.dir_name(i).to_ascii_lowercase();
        if let Some(p) = self.perf_parts.get(&key) {
            let mut v = p.clone();
            v.resize(13, 0);
            return v;
        }
        // (saves from before the shop had three packages: street, sport, race)
        let stage = match self.perf.get(&key).copied().unwrap_or(0) {
            0 => 0,
            1 | 2 => 1,
            _ => 2,
        };
        vec![stage; 13]
    }

    /// How far car `i` is towards its fully upgraded figures (0..1).
    pub fn perf_frac(&self, i: usize) -> f32 {
        let s = self.perf_stages(i);
        // (approx.) On a motorcycle six of the thirteen places are no parts of its own: the
        // Ninja of the 100 % save carries 2 in places 0, 4, 5, 6, 8, 12 and 0 in the rest
        // while its bars stand at the stock figures (reference `g_mv1`). Only the others count.
        if self.catalog.get(&self.dir_name(i)).map_or(false, |c| c.class.to_ascii_lowercase().contains("bike")) {
            let own: Vec<u8> = s.iter().enumerate().filter(|(k, _)| !matches!(k, 0 | 4 | 5 | 6 | 8 | 12)).map(|(_, v)| *v).collect();
            return own.iter().map(|v| (*v).min(2) as f32).sum::<f32>() / (2.0 * own.len().max(1) as f32);
        }
        s.iter().map(|v| (*v).min(2) as f32).sum::<f32>() / (2.0 * s.len().max(1) as f32)
    }

    /// Fit the next stage of part `k` to the current car, if the career has unlocked it
    /// and the money is there.
    /// AUTO UPGRADE: fit every stage that can be had, part by part; one line says what it cost.
    pub fn auto_upgrade(&mut self) {
        let i = self.current_car;
        let list = self.perf_list(i);
        let mut stages = self.perf_stages(i);
        let (mut spent, mut fitted) = (0i64, 0);
        for (k, (_, p1, p2)) in list.iter().enumerate() {
            while let Some(have) = stages.get(k).copied().filter(|h| *h < 2) {
                let price = if self.parts_free() { 0 } else if have == 0 { *p1 } else { *p2 };
                if self.rep < self.perf_rep(i, have + 1) || self.money < price {
                    break;
                }
                self.money -= price;
                spent += price;
                stages[k] = have + 1;
                fitted += 1;
            }
        }
        if fitted > 0 {
            self.perf_parts.insert(self.dir_name(i).to_ascii_lowercase(), stages.clone());
            self.perf_dirty = true;
            self.dirty = true;
            self.mc4_perf_bought(&self.dir_name(i), &stages, spent);
        }
    }

    pub fn buy_perf_part(&mut self, k: usize) -> bool {
        let i = self.current_car;
        let list = self.perf_list(i);
        let mut stages = self.perf_stages(i);
        let (Some((id, p1, p2)), Some(have)) = (list.get(k).cloned(), stages.get(k).copied()) else { return false };
        if have >= 2 {
            return false;
        }
        let need = self.perf_rep(i, have + 1);
        if self.rep < need {
            self.show_message(&format!("LOCKED - NEEDS {} REP", need));
            return false;
        }
        let price = if have == 0 { p1 } else { p2 };
        if self.money < price {
            let m = self.career.strings.get("G_PerformanceOverBudget").map(|s| s.to_uppercase()).unwrap_or_else(|| "NOT ENOUGH CASH".into());
            self.show_message(&m);
            return false;
        }
        self.money -= price;
        stages[k] = have + 1;
        self.mc4_perf_bought(&self.dir_name(i), &stages, price);
        self.perf_parts.insert(self.dir_name(i).to_ascii_lowercase(), stages);
        let name = self.career.strings.get(&id).map(|s| s.to_uppercase()).unwrap_or_else(|| id.to_uppercase());
        self.show_message(&format!("{}  STAGE {}", name, have + 1));
        self.perf_dirty = true;
        true
    }

    /// Performance level fitted to car `i` (0 = stock .. 3 = everything).
    pub fn perf_of(&self, i: usize) -> u8 {
        (self.perf_frac(i) * 3.0).round().clamp(0.0, 3.0) as u8
    }

    /// Price of upgrading car `i` from its current level to `level`.
    pub fn perf_price(&self, i: usize, level: u8) -> i64 {
        let (price, _) = self.price_of(i);
        let base = price.max(3000) as f64;
        // street / sport / race packages: 15 %, 30 %, 50 % of the car's price in total
        let total = |l: u8| match l {
            0 => 0.0,
            1 => 0.15,
            2 => 0.30,
            _ => 0.50,
        };
        let cur = self.perf_of(i);
        if level <= cur {
            return 0;
        }
        (((total(level) - total(cur)) * base) / 50.0).round() as i64 * 50
    }

    /// The garage camera view (as the cars' `.garage` files name them) that goes with the
    /// highlighted row.
    pub fn garage_view(&self) -> &'static str {
        // (test hook: MCLA_GARAGE_VIEW=<name> shows that view)
        if let Ok(v) = std::env::var("MCLA_GARAGE_VIEW") {
            return Box::leak(v.into_boxed_str());
        }
        if self.perf_shop.is_some() {
            return "WholeCarView";
        }
        if self.cart_open.is_some() {
            return "ShoppingCart";
        }
        // The police Impala has no usable cabin part meshes. Its interior
        // camera sits inside dark body geometry, so keep that page outside.
        if self.dir_name(self.current_car) == "vp_chv_police_96"
            && self.selected_car == self.current_car
            && (matches!(self.garage_menu, 7 | 16 | 17)
                || self.garage_xpart.is_some_and(|(k, _)| matches!(XPARTS[k.min(XPARTS.len() - 1)].1, "steer_whl" | "post_gauge"))
                || self.garage_part.is_some_and(|row| CUSTOM_SLOTS.get(row.wrapping_sub(1)) == Some(&Slot::Seats))) {
            return "WholeCarView";
        }
        if let Some(ed) = self.vinyl_edit.as_ref() {
            return ["TopOverViewMed", "LeftOverviewMed", "RightOverviewMed", "FrontOverviewMed", "RearOverviewMed"][ed.zone.min(4)];
        }
        // The menus name their view themselves (`garage.ForceCameraTransition(n)` in
        // tune/ui/garage; n indexes the game's view table at 0x828292A0): the top menu
        // 20 FrontHighFar, CUSTOMIZATION 24 FrontHighMed, the exterior areas through
        // PartAreaMenu's argument (front 18, all 21, rear 19, side 8, interior 3), the
        // wheel pages 5, paint and hydraulics 23.
        // An open part list has its own camera (`garage.GetCameraForPartType`). Read from
        // the running game's camera on every list of the R8 (step 34, view numbers of the
        // table): front bumpers and headlights 6 FrontLowClose, hoods 7 FrontHighClose,
        // widebody 23 LeftFrontQuarter, the wheel lists 5 Wheels, steering wheels 12
        // SteeringWheel, gauges 13 PostGauges, front seats 10 FrontSeats, spoilers 1
        // RearHighClose, rear bumpers, taillights, exhausts and plates 2 RearLowClose, neon,
        // sideskirts and doors 8 SideOverviewClose.
        if self.plate_edit.is_some() {
            return "RearLowClose";
        }
        if let (Some((k, _)), true) = (self.garage_xpart, self.selected_car == self.current_car && self.flash_garage.is_some()) {
            return XPARTS[k.min(XPARTS.len() - 1)].3;
        }
        if let (Some(row), true) = (self.garage_part, self.selected_car == self.current_car && self.flash_garage.is_some()) {
            match CUSTOM_SLOTS.get(row.wrapping_sub(1)) {
                Some(Slot::BumperF) => return "FrontLowClose",
                Some(Slot::Hood) => return "FrontHighClose",
                Some(Slot::Body) => return "LeftFrontQuarter",
                Some(Slot::Wheels) => return "Wheels",
                Some(Slot::Steer) => return "SteeringWheel",
                Some(Slot::Seats) => return "FrontSeats",
                Some(Slot::Spoiler) => return "RearHighClose",
                Some(Slot::BumperR) | Some(Slot::Exhaust) | Some(Slot::Trunk) => return "RearLowClose",
                Some(Slot::Skirts) | Some(Slot::DoorL) | Some(Slot::DoorR) => return "SideOverviewClose",
                _ => {}
            }
        }
        if self.selected_car == self.current_car && self.flash_garage.is_some() {
            match self.garage_menu {
                0 => return "FrontHighFar",
                1 | 14 => return "FrontHighMed",
                3 if self.paint_parts => return PAINT_AREAS[self.paint_area.min(5)].3,
                3 | 13 => return "LeftFrontQuarter",
                15 => return "WholeCarView",
                4 => return "FrontOverviewMed",
                5 => return "WholeCarView",
                // (EXTRA is view 8, as SIDE: read from the running game in step 34)
                9 | 11 => return "SideOverviewClose",
                // (DIMENSIONS: the axle worked on - `sub_826A5BA8`: front 5, rear 14, both 8)
                12 => return ["Wheels", "RearWheels", "SideOverviewClose"][self.dim_axle.min(2)],
                6 => return "Wheels",
                7 | 16 | 17 => return "Interior",
                18 => return "Wheels",
                8 => return "RearOverviewMed",
                10 => return "SideOverviewClose",
                _ => {}
            }
        }
        if self.garage_row == 0 || self.selected_car != self.current_car {
            return "Rotate";
        }
        match CUSTOM_SLOTS.get(self.garage_row - 1) {
            Some(Slot::Wheels) => "Wheels",
            Some(Slot::Body) | Some(Slot::Skirts) => "SideOverviewClose",
            Some(Slot::Hood) => "FrontHighClose",
            Some(Slot::Spoiler) => "RearHighClose",
            Some(Slot::BumperF) => "FrontLowClose",
            Some(Slot::BumperR) | Some(Slot::Exhaust) => "RearLowClose",
            Some(Slot::Seats) => "FrontSeats",
            Some(_) => "Rotate",
            None => {
                // performance, vinyl, the three paint sliders, rim size
                let k = self.garage_row - 1 - CUSTOM_SLOTS.len();
                match k {
                    1 => "LeftOverviewMed",
                    5 => "Wheels",
                    _ => "Rotate",
                }
            }
        }
    }

    /// Reputation needed before performance level `level` (1..3) can be bought for car `i`:
    /// the career unlocks the stages per group of car (D1, D2, C1 ... A2); the race package
    /// comes with the second stage.
    pub fn perf_rep(&self, i: usize, level: u8) -> i64 {
        if level == 0 || std::env::var("MCLA_UNLOCK_ALL").is_ok() {
            return 0;
        }
        let group = self.catalog.get(&self.dir_name(i)).map(|c| c.group.clone()).unwrap_or_else(|| "D0".into());
        let letter = group.chars().next().unwrap_or('D').to_ascii_uppercase();
        let id = format!("{}{}", letter, level.min(2));
        // (a level the save has unlocked is unlocked: `mc4`, performance_levels_unlocked)
        if self.mc4_perf_level_unlocked(&id) {
            return 0;
        }
        // (the level comes from the Unlocking system, by its own amount: `mc4`)
        self.mc4_perf_need(self.career.perf_unlocks.get(&id).copied().unwrap_or(0))
    }

    /// What the career still asks before a non-stock part can go into `slot` of the
    /// selected car: (wins needed, wins so far, class).
    pub fn slot_lock(&self, slot: Slot) -> Option<(i64, i64, String)> {
        if std::env::var("MCLA_UNLOCK_ALL").is_ok() {
            return None;
        }
        let part = match slot {
            Slot::BumperF => "FRONTBUMPER",
            Slot::BumperR => "REARBUMPER",
            Slot::Skirts => "SKIRTS",
            Slot::Spoiler => "SPOILER",
            Slot::Seats => "FRONTSEAT",
            Slot::Hood => "HOOD",
            Slot::Exhaust => "EXHAUST",
            Slot::Intercooler => "INTERCOOLER",
            Slot::Body => "WIDEBODY",
            _ => return None,
        };
        let class = self.catalog.get(&self.dir_name(self.selected_car)).map(|c| c.class.clone()).unwrap_or_default();
        // (the class system: its points and the flag its thresholds set - `mc4`)
        self.mc4_part_lock(&class, part).map(|(need, have)| (need, have, class.to_uppercase()))
    }

    /// Buy the highlighted performance level for the current car.
    pub fn try_buy_perf(&mut self) -> bool {
        let i = self.current_car;
        let level = self.perf_sel.min(3);
        if level <= self.perf_of(i) {
            return true;
        }
        let need = self.perf_rep(i, level);
        if self.rep < need {
            self.show_message(&format!("LOCKED - NEEDS {} REP", need));
            self.perf_sel = self.perf_of(i);
            return false;
        }
        let price = self.perf_price(i, level);
        if self.money < price {
            self.show_message(&format!("NOT ENOUGH CASH - ${} NEEDED", price));
            self.perf_sel = self.perf_of(i);
            return false;
        }
        self.money -= price;
        let n = self.dir_name(i).to_ascii_lowercase();
        self.mc4_parts_bought(&n, &[], price);
        self.perf.insert(n, level);
        self.show_message(&format!("PERFORMANCE UPGRADED FOR ${}", price));
        true
    }

    /// Performance value of the player's car with its upgrades.
    pub fn player_perf(&self) -> f32 {
        let i = self.current_car;
        // (the car's sample at its upgrade percentage as the original counts it - twelve parts,
        // a motorcycle's five: sub_8238F1F0, `mc4_upgrade_share`)
        self.samples.value(&self.dir_name(i), self.mc4_upgrade_share(i)).unwrap_or_else(|| {
            // (cars without a sample: the middle of their group)
            match self.catalog.get(&self.dir_name(i)).map(|c| c.group.clone()).unwrap_or_default().chars().next() {
                Some('A') | Some('a') => 0.7,
                Some('B') | Some('b') => 0.5,
                Some('C') | Some('c') => 0.3,
                _ => 0.12,
            }
        })
    }

    /// How hard car `name` (unmodified, as the rivals drive them) is for the player.
    pub fn difficulty_of(&self, name: &str) -> i32 {
        match self.samples.value(name, 0.0) {
            Some(p) => crate::rewards::difficulty(p, self.player_perf()),
            None => 1,
        }
    }

    /// Pay out a finished mission (delivery / payback).
    /// `delivered`: the damage (0..1) of the car handed over, for a delivery.
    pub fn award_mission(&mut self, success: bool, delivered: Option<f32>) {
        if !success {
            // (a payback that is lost starts again from its first leg)
            if delivered.is_none() {
                self.mc4_payback_end();
            }
            self.last_reward = (0, 0);
            // (an event not finished: the splash's DNF look - place 0)
            self.place_tied = false;
            self.place_splash = Some((0, PLACE_SPLASH_SECS));
            return;
        }
        // (a delivery: the manager's money less the damage, and its event's reward; a payback
        // leg: its share, the experience of the last leg, and the event `be_payback` -
        // sub_826B9DF8, sub_826B9F40, sub_826CD1E8: `mc4_mission_pay`)
        let (money, rep) = self.mc4_mission_pay(delivered);
        self.money += money;
        // (the reputation has no upper limit: sub_826BC348)
        self.rep += rep;
        self.last_reward = (money, rep);
        self.reward_panel = money > 0 || rep > 0 || self.mc4_rewards_listed();
    }

    /// Pay out a finished race: `place` is 1-based.
    pub fn award_race(&mut self, place: usize, field: usize) {
        // (x 0.6 for a race from the ARCADE menu, x 0.35 for a network game's: `mc4_race_factor`)
        let (mut money, mut rep) = crate::rewards::race_reward(self.rep, self.player_perf(), self.race_diff, place, field, self.mc4_race_factor());
        // (a race of the race editor gives nothing: EditorRace.sco sets the results' word
        // +3140 and sub_826B9AE8 leaves at once; a time trial gives money only:
        // TIME_TRIAL_RACE_COMPLETE, sub_826BA198)
        let s14_id = self.current_race.and_then(|i| self.race_defs.get(i)).map(|d| d.id.clone()).unwrap_or_default();
        let s14_editor = s14_id.starts_with("user/");
        let s14_plain = !s14_editor && !(!s14_id.is_empty() && (self.career.is_time_trial(&s14_id) || self.mc4_tt_of(&s14_id).is_some()));
        if s14_editor {
            money = 0;
        }
        if !s14_plain {
            rep = 0;
        }
        // (a pink slip race pays no money - results type 10, sub_826B9AE8 0x826B9BAC; in a
        // network game the experience goes to the Unlocking system alone - 0x826B9CB0)
        if self.career.is_pink_slip_race(&s14_id) || self.mc4_pink_on() {
            money = 0;
        }
        // (step 21: won - the car, lost - the stake stays in the lot)
        if let Some(key) = self.mc4_pink_end(place == 1) {
            let m = self.career.strings.get(key).map(|s| s.to_uppercase()).unwrap_or_else(|| if place == 1 { "PINKSLIP WON".into() } else { "PINKSLIP LOST".into() });
            self.show_message(&m);
        }
        // (a time trial pays when its target time was beaten: TIME_TRIAL_RACE_COMPLETE is
        // posted for "complete" only - generic_timetrial_hookman.sco 0x20a7..0x20fc)
        if self.career.tt_last == Some(false) {
            money = 0;
        }
        let s15_exp = rep;
        // a wager race pays the stake back twice over, or nothing
        self.wager_paid = false;
        if let Some((stake, _)) = self.wager.take() {
            money = if place == 1 { stake * 2 } else { 0 };
            self.wager_paid = place == 1;
            let key = if place == 1 { "WAGER_SUCCESS" } else { "WAGER_FAILURE" };
            let m = self.career.strings.get(key).map(|s| s.to_uppercase()).unwrap_or_else(|| if place == 1 { "WAGER WON".into() } else { "WAGER LOST".into() });
            self.show_message(&m);
        }
        // (a race of a series pays like any race: the manager takes the money away only past
        // third place and in a pink slip race. `sr_rewards` - REP EARNED: 60, no money row -
        // is a fourth place of four: 250 x 1/4 cut to tens, and no money past third)
        self.adaptive.result(place == 1, self.race_diff as i32);
        // (a race's place is the standings form 54: up 0.2 s after the line, closed after its
        // WAITUNWARPED(2200) - the splash's clock runs from the line)
        self.place_splash = Some((place, Self::RACE_END_MENU));
        self.race_place = place;
        self.rb_step = None;
        log::info!("reward: place {}/{} difficulty {} class {} -> ${} rep {} (race time {:.1} s)", place, field, self.race_diff, crate::rewards::perf_class(self.player_perf()), money, rep, self.race.as_ref().map_or(0.0, |r| r.time));
        self.money += money;
        // (past the career's top the reputation still counts up: 119170 -> 119230 after a
        // series race on the 100 % save - `rk95_map`, `sr_rewards`)
        let s14_rep0 = self.rep;
        // (the reputation has no upper limit: sub_826BC348)
        self.rep += rep;
        // (the same experience to the Unlocking system, a point of class reputation for the
        // vehicle driven and one for the ability fitted: sub_826B9AE8 / sub_826BB750, `mc4`)
        let s14_parts = if s14_plain { self.mc4_race_experience(s14_rep0, s15_exp) } else { Vec::new() };
        // (what the class point unlocked is listed on the rewards screen: `unlocked_now`)
        self.career.unlocked_now = s14_parts.clone();
        if !s14_parts.is_empty() {
            log::info!("reward: class reputation unlocks {:?}", s14_parts);
        }
        self.last_reward = (money, rep);
        // (REWARDS follows whenever something was earned)
        self.reward_panel = money > 0 || rep > 0 || self.mc4_rewards_listed();
    }

    /// Where a race starts in the world (pole position, else its beacon). (The start beacon
    /// was tried as the maps' place for it - step 100 - and put back: the tournament's cup
    /// that seemed to stand off the grid is a car on the road, not a place.)
    pub fn race_start(&self, i: usize) -> Option<Vec3> {
        let r = self.race_defs.get(i)?;
        Some(r.grid.first().map(|g| g.0).unwrap_or(r.beacon))
    }

    /// The `n` races starting closest to `p`: (index, start position, distance).
    pub fn races_near(&self, p: Vec3, n: usize) -> Vec<(usize, Vec3, f32)> {
        let mut v: Vec<(usize, Vec3, f32)> = (0..self.race_defs.len()).filter_map(|i| self.race_start(i).map(|s| (i, s, s.distance(p)))).collect();
        // career events stand out: they sort as if five times closer
        let key = |e: &(usize, Vec3, f32)| if self.mission_races.contains(&e.0) { e.2 * 0.2 } else { e.2 };
        v.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
        v.truncate(n);
        v
    }

    /// District name at a position: by the city's zone map (`mc4::district_of_place`,
    /// Racer_GetCurrentDistrict); without the map that of the nearest race event.
    pub fn district_at(&self, p: Vec3) -> String {
        if let Some(d) = crate::mc4::district_of_place(&self.data_dir, p) {
            return crate::mc4::DISTRICTS[d.min(4)].to_string();
        }
        self.races_near(p, 1).first().map(|(i, _, _)| self.race_defs[*i].district.clone()).unwrap_or_default()
    }

    /// The MAP action (D-pad up, "OVERHEAD MAP"; rule_frame.md 9): blocked in a replay (form+5708);
    /// while the SIDEKICK phone is up it dismisses the phone (form+2832, SIDEKICK_DISMISS_SIDEKICK_TIP)
    /// instead; else the HUD toggles hudNormal <-> hudBigMap (the overhead map).
    pub fn map_button(&mut self) {
        if self.replay.is_some() {
            return;
        }
        if self.flash_phone.is_some() && self.tip.is_some() && self.race.is_none() {
            self.tip = None;
            return;
        }
        self.hud_map_big = !self.hud_map_big;
    }

    pub fn in_menu(&self) -> bool {
        matches!(self.mode, Mode::MainMenu | Mode::RaceSelect | Mode::Garage | Mode::Results | Mode::Paused) || self.starter_pick.is_some()
    }

    pub fn car_name(&self, i: usize) -> String {
        let raw = self.car_dirs.get(i).and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        pretty_car_name(&raw)
    }

    pub fn show_message(&mut self, m: &str) {
        self.message = m.to_string();
        self.message_timer = 3.0;
        self.message_posted = false;
    }

    /// Start one of the game's own races: original grid, checkpoints and laps.
    pub fn start_race_def(&mut self, idx: usize, world: &World, vehicles: &mut Vehicles, gpu: &crate::render::Gpu) -> bool {
        self.rb_step = None;
        let Some(def) = self.race_defs.get(idx).cloned() else { return false };
        let Some(nav) = world.nav.as_ref() else { return false };
        if vehicles.cars.iter().position(|c| c.is_player).is_none() {
            return false;
        }
        // (the race joins the history of the last twelve: mcCareer_AddRaceToHistory, `mc4`)
        self.mc4_race_started(idx);
        // On the first half of the career's difficulty steps the easy layout is driven (more
        // checkpoints along the same route), later the open one.
        let easy_wanted = match std::env::var("MCLA_EASY").ok().as_deref() {
            Some("1") => true,
            Some(_) => false,
            // (the race data's layout word: 0 - the easy layout - below Hard for an ARCADE race,
            // a hookman's, a wager and an event from a panel: `mc4_layout_easy`. Elsewhere the
            // port's old guess stands: the first half of the career.)
            None => self.mc4_layout_easy().unwrap_or((self.rep as f32 / crate::catalog::MAX_REP as f32) < 0.5),
        };
        let mut def = def;
        // (a time trial is one lap against the row's target time: `mc4_tt_apply`)
        self.mc4_tt_apply(&mut def);
        let mut easy_course = None;
        if easy_wanted && !def.easy_id.is_empty() && matches!(def.race_type.as_str(), "Ordered" | "Unordered") {
            if let Some(c) = crate::racescript::course(&self.data_dir, &def.easy_id, None).filter(|c| c.cps.len() >= 2 && def.checkpoints.last().map_or(true, |l| c.cps[c.cps.len() - 1].pos.distance(*l) < 400.0)) {
                log::info!("race {}: easy layout, {} checkpoints instead of {}", def.name, c.cps.len(), def.checkpoints.len());
                def.checkpoints = c.cps.iter().map(|p| p.pos).collect();
                def.id = def.easy_id.clone();
                easy_course = Some(c);
            }
        }
        // (the race's script resets the list and fills it from its own tables - CheckpointList_Reset,
        // _ResizeList, Checkpoint_SetPosition ..: where it lists other checkpoints than the race
        // file - be_circuit_mg_02, sc_ordered_jp_02, sc_ordered_mb_01 - the script's list is driven)
        if easy_course.is_none() && matches!(def.race_type.as_str(), "Ordered" | "Unordered") {
            if let Some(s) = crate::checkpoints::script_specs(&self.data_dir, &def.id, def.checkpoints.first().copied()).filter(|s| s.len() != def.checkpoints.len() && !s.is_empty()) {
                log::info!("race {}: the script's {} checkpoints instead of the race file's {}", def.name, s.len(), def.checkpoints.len());
                def.checkpoints = s.iter().map(|c| c.pos).collect();
            }
        }
        self.active_def = Some(def.clone());
        // (an ARCADE circuit runs the menu's NUMBER OF LAPS: `mc4_arcade_laps`)
        let laps = self.mc4_arcade_laps(&def).unwrap_or(def.laps.max(1) as usize);
        // the race's script: which checkpoints are gates, and the routes the opponents know
        let course = easy_course.or_else(|| def.checkpoints.first().and_then(|f| crate::racescript::course(&self.data_dir, &def.id, Some(*f)))).filter(|c| c.cps.len() == def.checkpoints.len() && std::env::var("MCLA_NO_COURSE").is_err());
        if let Some(c) = course.as_ref() {
            log::info!("race course: {} of {} checkpoints are gates, {} route nodes in {} routes ({} driven in the race), {} cop zones", c.cps.iter().filter(|p| p.gate.is_some()).count(), c.cps.len(), c.nodes.len(), c.routes.len(), c.routes.iter().filter(|r| r.weight > 0.0).count(), c.cop_zones.len());
        }
        let lap_cps: Vec<Vec3> = match course.as_ref() {
            Some(c) => c.cps.iter().map(|p| p.mark()).collect(),
            None => def.checkpoints.clone(),
        };
        let mut checkpoints = Vec::new();
        for _ in 0..laps {
            checkpoints.extend(lap_cps.iter().map(|c| *c + Vec3::Y * 0.2));
        }
        // race line: road route through every checkpoint in order
        let (gpos, gyaw) = def.grid[0];
        let fwd0 = Vec3::new(-gyaw.sin(), 0.0, -gyaw.cos());
        let (path, missing) = nav.route_through(gpos, fwd0, &checkpoints, 9.0);
        // checkpoints that are not on a road (plazas, car parks): leave the line there
        let mut path = path;
        let off_road_fix = |path: &mut Vec<Vec3>| -> usize {
            let mut from = 0usize;
            let mut off_road = 0;
            for cp in checkpoints.iter() {
                let mut best = (f32::INFINITY, from, 0.0f32);
                for pi in from..path.len().saturating_sub(1) {
                    let ab = path[pi + 1] - path[pi];
                    let t = ((*cp - path[pi]).dot(ab) / ab.length_squared().max(0.01)).clamp(0.0, 1.0);
                    let d = (path[pi] + ab * t).distance(*cp);
                    // take the first good pass, not a later lap's
                    if d < best.0 - 0.5 {
                        best = (d, pi, t);
                    }
                    if best.0 < 12.0 && d > best.0 + 60.0 {
                        break;
                    }
                }
                if best.0 > 14.0 && best.0.is_finite() {
                    path.insert(best.1 + 1, *cp);
                    off_road += 1;
                    from = best.1 + 1;
                } else {
                    from = best.1;
                }
            }
            off_road
        };
        let mut off_road = off_road_fix(&mut path);
        let is_cp = |p: Vec3| checkpoints.iter().any(|c| *c == p);
        let n_loops = drop_loops(&mut path, &is_cp);
        if n_loops > 0 {
            log::info!("race {}: {} points of junction loops taken out of the line", def.name, n_loops);
        }
        // with the script's routes: through the nodes of the likeliest route of each stretch
        let circuit = laps > 1 || def.sub_type == "Circuit";
        let cuts_on = std::env::var("MCLA_SHORTCUTS").map_or(true, |v| v != "0");
        let course_line = |c: &crate::racescript::Course, rnd: Option<&mut dyn FnMut() -> f32>| -> (Vec<Vec3>, usize, Vec<Shortcut>) {
            let per_leg = c.pick(circuit, rnd);
            let n = c.cps.len();
            // every stop of the line, and whether it lies away from the roads
            let mut stops: Vec<(Vec3, bool)> = Vec::new();
            for lap in 0..laps {
                for k in 0..n {
                    stops.push((c.cps[k].mark() + Vec3::Y * 0.2, false));
                    if !(lap + 1 == laps && k + 1 == n) {
                        for (p, _) in per_leg[k].iter() {
                            let on_road = nav.nearest_link(*p).map_or(false, |l| l.2 < 9.0);
                            if on_road || cuts_on {
                                stops.push((*p, !on_road));
                            }
                        }
                    }
                }
            }
            let mut line: Vec<Vec3> = Vec::new();
            let mut cuts: Vec<(Vec3, Vec3)> = Vec::new();
            let mut run: Vec<Vec3> = Vec::new();
            let mut run_start = gpos;
            let mut heading = fwd0;
            let mut ok = true;
            let mut flush = |line: &mut Vec<Vec3>, run: &mut Vec<Vec3>, start: Vec3, heading: Vec3| -> bool {
                if run.is_empty() {
                    return true;
                }
                let (l, missing) = nav.route_through(start, heading, run, 9.0);
                run.clear();
                for q in l {
                    if line.last().map_or(true, |p| p.distance(q) > 0.5) {
                        line.push(q);
                    }
                }
                missing == 0
            };
            let mut i = 0;
            while i < stops.len() {
                if !stops[i].1 {
                    run.push(stops[i].0);
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < stops.len() && stops[j].1 {
                    j += 1;
                }
                if j >= stops.len() {
                    break;
                }
                ok &= flush(&mut line, &mut run, run_start, heading);
                // straight through the nodes to the next stop on a road
                let a = *line.last().unwrap_or(&gpos);
                if line.is_empty() {
                    line.push(a);
                }
                let mut prev = a;
                for (p, _) in &stops[i..=j] {
                    let steps = (prev.distance(*p) / 8.0).ceil().max(1.0) as usize;
                    for s in 1..steps {
                        line.push(prev.lerp(*p, s as f32 / steps as f32));
                    }
                    line.push(*p);
                    // (each hop from node to node is found on its own)
                    cuts.push((prev, *p));
                    prev = *p;
                }
                run_start = stops[j].0;
                let d = stops[j].0 - stops[j - 1].0;
                heading = Vec3::new(d.x, 0.0, d.z).normalize_or(heading);
                i = j + 1;
            }
            ok &= flush(&mut line, &mut run, run_start, heading);
            if !ok {
                return (Vec::new(), 0, Vec::new());
            }
            let off = off_road_fix(&mut line);
            drop_loops(&mut line, &|p| checkpoints.iter().any(|c| *c == p) || cuts.iter().any(|c| c.0 == p || c.1 == p));
            // where the stretches ended up in the finished line
            let mut out = Vec::new();
            let mut cursor = 0;
            for (a, b) in cuts {
                let Some(from) = (cursor..line.len()).find(|&k| line[k] == a) else { continue };
                let Some(to) = (from + 1..line.len()).find(|&k| line[k] == b) else { continue };
                out.push(Shortcut { car: usize::MAX, from, to, done: false, retry: 0.0 });
                cursor = to;
            }
            (line, off, out)
        };
        let mut shortcuts: Vec<Shortcut> = Vec::new();
        let routed = course.as_ref().filter(|c| c.routes.iter().any(|r| r.weight > 0.0));
        if let Some(c) = routed {
            let (l, off, cuts) = course_line(c, None);
            if l.len() > 4 {
                path = l;
                off_road = off;
                shortcuts = cuts;
            }
        }
        let mut seed = (self.rand() * 4.0e9) as u32 | 1;
        let mut lcg = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / 16777216.0
        };
        let mut total = 0.0;
        for w in path.windows(2) {
            total += w[0].distance(w[1]);
        }
        if off_road > 0 {
            log::info!("race {}: {} checkpoints are off the road network", def.name, off_road);
        }
        if std::env::var("MCLA_COURSE_DUMP").is_ok() {
            for (i, q) in path.iter().enumerate() {
                log::info!("  line[{}] = ({:.1}, {:.1}, {:.1})", i, q.x, q.y, q.z);
            }
            if let Some(c) = routed {
                for (k, l) in c.pick(circuit, None).iter().enumerate() {
                    for (q, r) in l {
                        log::info!("  leg {} node ({:.1}, {:.1}, {:.1}) r {} lane {:?}", k, q.x, q.y, q.z, r, nav.nearest_link(*q).map(|l| l.2));
                    }
                }
            }
        }
        for (i, q) in path.iter().enumerate().take(40) {
            log::debug!("  line[{}] = ({:.1}, {:.1}, {:.1})", i, q.x, q.y, q.z);
        }
        // (MCLA_DUMP_RACELINE=<file>: the race line and the checkpoints as text, for tools)
        if let Ok(f) = std::env::var("MCLA_DUMP_RACELINE") {
            let mut t = String::new();
            for q in path.iter() {
                t.push_str(&format!("L {:.2} {:.2} {:.2}
", q.x, q.y, q.z));
            }
            for q in def.checkpoints.iter() {
                t.push_str(&format!("C {:.2} {:.2} {:.2}
", q.x, q.y, q.z));
            }
            for g in def.grid.iter() {
                t.push_str(&format!("G {:.2} {:.2} {:.2} {:.4}
", g.0.x, g.0.y, g.0.z, g.1));
            }
            // (and a copy by the race's name, for runs of several races)
            let _ = std::fs::write(format!("{}.{}", f, def.name), &t);
            let _ = std::fs::write(f, t);
        }
        let route: Vec<usize> = Vec::new();
        let path = std::sync::Arc::new(path);
        log::info!("race {}: {} checkpoints x {} laps, race line {} points, {:.0} m ({} legs without a road route)", def.name, def.checkpoints.len(), laps, path.len(), total, missing);
        let kind = match def.race_type.as_str() {
            "Delivery" => RaceKind::Delivery,
            "Payback" => RaceKind::Payback,
            _ => RaceKind::Race,
        };
        // mission time limits come in four difficulty steps; the career moves through them
        let diff = ((self.rep as f32 / crate::catalog::MAX_REP as f32) * 4.0).floor().clamp(0.0, 3.0) as usize;
        let time_limit = if kind == RaceKind::Race { 0.0 } else { def.target_times.get(diff).or(def.target_times.first()).copied().unwrap_or(0.0) };
        self.mission_diff = diff;
        // grid
        vehicles.cars.retain(|c| c.is_player);
        let pi = 0;
        let n_opp = match kind {
            RaceKind::Delivery => 0,
            RaceKind::Payback => 1,
            RaceKind::Race if self.career.is_time_trial(&def.id) || self.mc4_tt_of(&def.id).is_some() => 0,
            RaceKind::Race if self.wager.is_some() => 1,
            // (a pink slip race: wager_hangout_core makes one opponent, 0x54c3 - step 21)
            RaceKind::Race if self.mc4_pink_on() => 1,
            // (a hookman flashed on the road races alone against the player: "2/2" in the
            // recomp's first career race - `ng_race_*`)
            RaceKind::Race if self.hookman_race => 1,
            // (step 19d: a generic local hookman's race is his and three more racers' -
            // generic_local_hookman.sco's argument 3)
            RaceKind::Race if self.mc4_local_man_race() => 4.min(def.grid.len().saturating_sub(1)).max(1),
            // (a hangout's series is four cars - `sr_results`)
            RaceKind::Race if self.series.is_some() => 3.min(def.grid.len().saturating_sub(1)),
            RaceKind::Race => def.opponents.clamp(1, 7).min(def.grid.len().saturating_sub(1)),
        };
        if kind == RaceKind::Race {
            // (the race core scripts bind the start scene to RaceGrid_GetPosition(grid, 0) and its
            // heading - CineScript_SetGameCoordSysBinding, OrderedCheckpointCore / UnorderedCheckpointCore)
            let front = if std::env::var("MCLA_PRERACE_MID").is_ok() { def.grid.get(1).map_or(def.grid[0].0, |g| (g.0 + def.grid[0].0) * 0.5) } else { def.grid[0].0 };
            self.prerace = Some((if n_opp == 0 { def.grid[0].0 } else { front }, def.grid[0].1, if n_opp == 0 { "1x1_Race_Grid".to_string() } else { def.grid_type.clone() }));
            // (the race scripts: SafetyCam 1 -> the Safety list; no opponents -> grid 0, 1x1; camOnly =
            // !AllowRaceStarter; a first race = a single race or a series' first round)
            let grid = if def.safety_cam { "Safety".to_string() } else if n_opp == 0 { "1x1_Race_Grid".to_string() } else { def.grid_type.clone() };
            let first_race = self.series.as_ref().map_or(true, |s| s.at == 0) && !std::mem::take(&mut self.next_race_start);
            let slot1 = def.grid.get(1).map_or(def.grid[0].0, |g| g.0);
            self.prerace_args = Some((crate::prerace::PickArgs { grid, first_race, cam_only: def.no_starter, front_lr: false }, [def.grid[0].0, slot1]));
        }
        // the race's racer list puts the player first and racer i on grid slot i (Cars_AssignIdToSlot /
        // RaceGrid): recorded AROUND HOLLYWOOD, the player's Saleen on GridPosition 0 exactly
        // (work/physics2/rec/a.0.bin), the three opponents on slots 1..3
        let pslot = 0;
        let (pp, pyaw) = def.grid[pslot];
        vehicles.cars[pi].state = crate::physics::CarState::new(pp + Vec3::Y * 0.9, pyaw);
        vehicles.cars[pi].placed = false;
        vehicles.cars[pi].ai = None;
        let mut participants = vec![pi];
        // opponents drive player-class cars, never ambient traffic vehicles
        let racers: Vec<_> = vehicles.models.iter().filter(|m| !m.name.starts_with("va_") && !m.name.contains("police")).cloned().collect();
        let models: Vec<_> = if racers.is_empty() { vehicles.models.clone() } else { racers };
        // (a wager is one on one, against the car nearest to the difficulty paid for)
        let wager_model = self.wager.filter(|_| kind == RaceKind::Race).and_then(|(_, d)| models.iter().min_by_key(|m| (self.difficulty_of(&m.name) - d as i32).abs()).cloned());
        for k in 0..n_opp {
            let (p, yaw) = def.grid[(k + 1).min(def.grid.len() - 1)];
            let mut model = wager_model.clone().unwrap_or_else(|| models[(k + 1) % models.len()].clone());
            let mut color = PALETTE[(k * 3 + 1) % PALETTE.len()];
            // a named rival in his own car and paint
            let mut driver = String::new();
            let mut plate = String::new();
            let mut vinyl = String::new();
            // (step 19d: the payback's client in the leg's vehicle and set-up, its driver at
            // the wheel - `mc4_payback_pick`; stock: Racer_SetModValue(0))
            let mut s19d_set = String::new();
            if let Some((car, cfg, drv)) = self.opt_x.arcade.d.client.clone().filter(|_| kind == RaceKind::Payback) {
                if let Some(m) = vehicles.models.iter().find(|m| m.name.eq_ignore_ascii_case(&car)) {
                    model = m.clone();
                    vinyl = cfg;
                    if drv.starts_with("drv_") {
                        s19d_set = drv;
                    } else {
                        driver = drv;
                    }
                }
            }
            if let Some(r) = self.grid_rivals.get(k).filter(|_| kind == RaceKind::Race && self.wager.is_none()) {
                if let Some(m) = vehicles.models.iter().find(|m| m.name.eq_ignore_ascii_case(&r.car)) {
                    model = m.clone();
                    color = r.color;
                    driver = r.id.clone();
                    plate = r.plate.clone();
                    vinyl = r.config.clone();
                }
            }
            let top = model.params.top_speed;
            let idx = vehicles.spawn(gpu, model, p + Vec3::Y * 0.9, yaw, color, false);
            log::debug!("grid car {}: {} plate [{}] livery {}", k, vehicles.cars[idx].model.name, plate, vinyl);
            vehicles.cars[idx].pinned = true;
            vehicles.cars[idx].driver = driver;
            vehicles.cars[idx].plate_text = plate;
            vehicles.cars[idx].vinyl_name = vinyl;
            vehicles.cars[idx].driver_set = s19d_set;
            let mut d = AiDriver::new(0x1234 + k as u32 * 77);
            d.target_speed = (top * (0.62 + 0.05 * (n_opp - k) as f32)).clamp(26.0, 62.0);
            if std::env::var("MCLA_SLOW_RIVALS").is_ok() {
                // self-test: let the test driver win
                d.target_speed = 12.0;
            }
            d.aggression = 0.8;
            // every opponent makes his own choice of the routes he knows
            d.path = Some(match routed {
                Some(c) => {
                    let (l, _, cuts) = course_line(c, Some(&mut lcg as &mut dyn FnMut() -> f32));
                    if l.len() > 4 {
                        shortcuts.extend(cuts.into_iter().map(|mut s| {
                            s.car = idx;
                            s
                        }));
                        std::sync::Arc::new(l)
                    } else {
                        path.clone()
                    }
                }
                None => path.clone(),
            });
            if kind == RaceKind::Payback {
                // the mark runs for it
                d.target_speed = (top * 0.7).clamp(24.0, 48.0);
            }
            vehicles.cars[idx].ai = Some(d);
            participants.push(idx);
        }
        let target = if kind == RaceKind::Payback { participants.get(1).copied() } else { None };
        let n = vehicles.cars.len();
        self.race = Some(Race {
            path: Some(path.clone()),
            route,
            checkpoints,
            next: vec![0; n],
            finish_time: vec![None; n],
            time: 0.0,
            countdown: 4.5,
            total_len: total,
            participants,
            lap_len: def.checkpoints.len(),
            laps,
            name: def.title(),
            goal_time: def.goal_time,
            kind,
            time_limit,
            target,
            failed: None, cp_radius: Vec::new(),
            cp_gate: course.as_ref().map(|c| c.cps.iter().map(|p| p.gate.map(|(b, _)| (p.pos, b))).collect()).unwrap_or_default(),
            prev_pos: Vec::new(),
            shortcuts,
            cop_zones: if kind == RaceKind::Race { course.as_ref().map(|c| c.cop_zones.iter().map(|z| (z.0, z.1, false)).collect()).unwrap_or_default() } else { Vec::new() },
            shortcut_lines: Default::default(),
            rivals_checkpoints: (kind == RaceKind::Race && laps <= 1 && (def.race_type == "Unordered" || def.sub_type == "Unordered")).then(|| def.checkpoints.clone()),
            // (the race's own tables, as the script hands them to Checkpoint_SetPosition ..
            // Checkpoint_SetMaxSpeed)
            cp_specs: if let Some(s23_specs) = self.mc4_editor_specs(&def) {
                // (step 23: an editor race's checkpoints from its slot - Net_EditorRaceCreateCheckpoints)
                s23_specs
            } else if def.sub_type == "RedLight" {
                // (the red light race's finish: radius 12.5 - script 0x2672..0x28c0, the table
                // at +1712 - and the object's other defaults)
                def.checkpoints.iter().map(|c| crate::checkpoints::CpSpec::with_radius(*c, 12.5)).collect()
            } else if matches!(kind, RaceKind::Race | RaceKind::Delivery) && std::env::var("MCLA_NO_COURSE").is_err() {
                // (a delivery's script sets its drop-off the same way: one checkpoint, e.g.
                // sc_delivery_01 radius 20, height 10, activation height 40)
                crate::checkpoints::script_specs(&self.data_dir, &def.id, def.checkpoints.first().copied()).filter(|s| s.len() == def.checkpoints.len()).unwrap_or_default()
            } else {
                Vec::new()
            },
        });
        if let Some(r) = self.race.as_ref() {
            log::info!("race checkpoints: {} of a lap from the script ({} lines, radii {:?})", r.cp_specs.len(), r.cp_specs.iter().filter(|c| c.second.is_some()).count(), r.cp_specs.iter().map(|c| c.radius).collect::<Vec<_>>());
        }
        vehicles.fragile = target;
        self.current_race = Some(idx);
        self.mode = Mode::Race;
        true
    }

    /// Build a race route from the player's current link.
    pub fn start_race(&mut self, world: &World, vehicles: &mut Vehicles, gpu: &crate::render::Gpu) -> bool {
        let Some(nav) = world.nav.as_ref() else { log::warn!("race: no nav"); return false };
        let Some(pi) = vehicles.cars.iter().position(|c| c.is_player) else { log::warn!("race: no player"); return false };
        let pos = vehicles.cars[pi].state.pos;
        let fwd = vehicles.cars[pi].state.forward();
        let Some((start, t)) = nav.nearest_link_aligned(pos, fwd) else { log::warn!("race: no link near {:?}", pos); return false };
        // route: greedy walk preferring straight, ~2.5 km
        let mut route = vec![start];
        let mut total = nav.links[start].len * (1.0 - t);
        let mut cur = start;
        let mut guard = 0;
        while total < 2600.0 && guard < 200 {
            guard += 1;
            let l = &nav.links[cur];
            if l.next.is_empty() {
                break;
            }
            let d = l.dir();
            let mut best = None;
            let mut bs = f32::NEG_INFINITY;
            for &n in &l.next {
                if route.contains(&n) {
                    continue;
                }
                let s = nav.links[n].dir().dot(d) + self.rand() * 0.6 + if nav.links[n].next.is_empty() { -5.0 } else { 0.0 };
                if s > bs {
                    bs = s;
                    best = Some(n);
                }
            }
            let Some(n) = best else { break };
            total += nav.links[n].len;
            route.push(n);
            cur = n;
        }
        log::info!("race: start link {} route {} links, {:.0} m", start, route.len(), total);
        if route.len() < 3 {
            return false;
        }
        // checkpoints every ~250 m along the route + finish
        let mut checkpoints = Vec::new();
        let mut acc = 0.0;
        for (i, &li) in route.iter().enumerate() {
            let l = &nav.links[li];
            if i == 0 {
                continue;
            }
            acc += nav.links[route[i - 1]].len;
            if acc > 250.0 {
                checkpoints.push(l.a + Vec3::Y * 0.2);
                acc = 0.0;
            }
        }
        let last = &nav.links[*route.last().unwrap()];
        checkpoints.push(last.b + Vec3::Y * 0.2);
        // reset player & spawn opponents on the start link
        let l = &nav.links[start];
        let dir = l.dir();
        let right = Vec3::new(-dir.z, 0.0, dir.x);
        let yaw = (-dir.x).atan2(-dir.z);
        let base = l.point_at(t);
        vehicles.cars.retain(|c| c.is_player);
        let pi = 0;
        vehicles.cars[pi].state = crate::physics::CarState::new(base + Vec3::Y * 0.8, yaw);
        vehicles.cars[pi].placed = false;
        vehicles.cars[pi].ai = None;
        let mut participants = vec![pi];
        let models: Vec<_> = vehicles.models.clone();
        for k in 0..3 {
            let side = if k % 2 == 0 { 3.2 } else { -3.2 };
            let back = 6.0 * (k as f32 / 2.0).floor() + 6.0 * ((k + 1) % 2) as f32;
            let p = base + right * side - dir * back + Vec3::Y * 0.8;
            let model = models[(k + 1) % models.len()].clone();
            let color = PALETTE[(k * 3 + 1) % PALETTE.len()];
            let idx = vehicles.spawn(gpu, model, p, yaw, color, false);
            vehicles.cars[idx].pinned = true;
            let mut d = AiDriver::new(0x1234 + k as u32 * 77);
            d.target_speed = 24.0 + k as f32 * 2.5;
            d.route_override = Some(route.clone());
            d.link = Some(start);
            d.t = t;
            vehicles.cars[idx].ai = Some(d);
            participants.push(idx);
        }
        let n = vehicles.cars.len();
        let lap_len = checkpoints.len();
        self.race = Some(Race { path: None, route, checkpoints, next: vec![0; n], finish_time: vec![None; n], time: 0.0, countdown: 3.5, total_len: total, participants, lap_len, laps: 1, name: "Street race".into(), goal_time: 0.0, kind: RaceKind::Race, time_limit: 0.0, target: None, failed: None, cp_radius: Vec::new(), cp_gate: Vec::new(), prev_pos: Vec::new(), shortcuts: Vec::new(), cop_zones: Vec::new(), shortcut_lines: Default::default(), rivals_checkpoints: None, cp_specs: Vec::new() });
        vehicles.fragile = None;
        self.mode = Mode::Race;
        true
    }

    /// Per-frame race logic. Returns true if the race just finished for the player.
    pub fn update_race(&mut self, vehicles: &mut Vehicles, dt: f32, world_ready: bool) -> bool {
        let mut finished_now = false;
        if self.rb_step.is_none() && self.race.is_some() {
            // the event is meant to be as hard as its first racer is for the player; a
            // field that is not all of that difficulty shifts the opponents level
            let diffs: Vec<i32> = self.race.as_ref().map(|r| r.participants.iter().skip(1).filter_map(|i| vehicles.cars.get(*i)).map(|c| self.difficulty_of(&c.model.name)).collect()).unwrap_or_default();
            let desired = diffs.first().copied().unwrap_or(1).clamp(0, 3);
            // (a hookman's race is as hard as the set-up his mission names)
            let desired = self.mc4_hookman_diff().filter(|_| self.hookman_race).map_or(desired, |d| d as i32);
            let desired = self.wager.map_or(desired, |(_, d)| d as i32);
            // (an event accepted at a hangout runs at the difficulty chosen there)
            let desired = self.event_diff.map_or(desired, |d| d as i32);
            // (a red light or freeway man's race is as hard as he is - his script's race data
            // +3100 = mcHookman_GetDifficulty: althk.rs)
            let desired = self.althk.diff.filter(|_| self.active_def.as_ref().map_or(false, |d| crate::althk::is_alt_race(&d.id))).map_or(desired, |d| d as i32);
            // (an ARCADE race runs at the menu's DIFFICULTY: its launcher puts it in the race
            // data - sub_8265CC90, `mc4_arcade_difficulty`)
            // (and a time trial at Medium: its script's +3100 = 1 - `mc4_fixed_difficulty`)
            let desired = match self.mc4_fixed_difficulty() {
                Some(d) if self.event_diff.is_none() => d as i32,
                _ => desired,
            };
            let odd = diffs.iter().copied().find(|d| *d != desired);
            self.race_diff = desired as usize;
            let step = self.adaptive.step(desired, odd);
            log::info!("race: difficulty {} (field {:?}; hookman race {} set-up {:?}), opponents level {} -> rubber band step {}", desired, diffs, self.hookman_race, self.mc4_hookman_diff(), self.adaptive.level, step);
            self.rb_step = Some(step);
        }
        // (step 19c: LAST MAN OUT, the time trial's clock, the payback leg - `mc4`)
        if self.mc4_lmo_tick(vehicles) {
            finished_now = true;
        }
        self.mc4_tt_tick();
        // (step 19d: once the leg's time is up the client's damage decides nothing - the
        // chase does: `mc4_payback_over`)
        let s19a_pb = self.mc4_payback_tick(vehicles);
        let s19a_pb = if s19a_pb.is_finite() { s19a_pb.min(0.999) } else { s19a_pb };
        if self.mc4_payback_over(vehicles) {
            finished_now = true;
        }
        let Some(race) = self.race.as_mut() else {
            vehicles.grid_hold = None;
            return false;
        };
        if race.countdown > 0.0 {
            if world_ready || race.countdown < 3.4 {
                race.countdown -= dt;
            }
            // (N-VP-1: the grid is not frozen - the cars simulate, held by their brakes, so revving and
            // burnouts happen as in the original: `Vehicles::grid_hold`, `CarInput::hold`, `AiDriver::grid_input`)
            vehicles.frozen = false;
            vehicles.grid_hold = (race.countdown > 0.0).then_some(race.countdown);
            return false;
        }
        vehicles.frozen = false;
        vehicles.grid_hold = None;
        race.time += dt;
        let n = race.checkpoints.len();
        while race.next.len() < vehicles.cars.len() {
            race.next.push(0);
            race.finish_time.push(None);
        }
        // missions
        if race.kind != RaceKind::Race && race.failed.is_none() && race.finish_time.first().map_or(true, |t| t.is_none()) {
            // (a delivery is judged by what the customer sees: the body's zones)
            let pdmg = vehicles.cars.iter().find(|c| c.is_player).map_or(0.0, |c| if race.kind == RaceKind::Delivery { c.state.damage.max(c.state.delivery_damage()) } else { c.state.damage });
            // (step 19d: a payback leg's time-out does not fail it - the client's friends
            // give chase: `mc4_payback_tick`)
            if race.time_limit > 0.0 && race.time > race.time_limit && race.kind != RaceKind::Payback {
                race.failed = Some("TIME EXPIRED");
            } else if race.kind == RaceKind::Delivery && pdmg >= 0.999 {
                race.failed = Some("DAMAGE EXCEEDED");
            }
            if let Some(ti) = race.target {
                if let Some(tc) = vehicles.cars.get(ti) {
                    // the marker rides on the mark
                    race.checkpoints = vec![tc.state.pos];
                    // (the leg is won at the difficulty's share of the client's damage:
                    // PaybackLegCore 0x0cc6..0x1017 - `mc4::payback_threshold`)
                    if tc.state.damage >= s19a_pb {
                        if let Some(pi) = vehicles.cars.iter().position(|c| c.is_player) {
                            race.finish_time[pi] = Some(race.time);
                            finished_now = true;
                        }
                    }
                }
            }
            if race.failed.is_some() {
                finished_now = true;
            }
        }
        for (i, c) in vehicles.cars.iter().enumerate() {
            if race.finish_time[i].is_some() || race.kind == RaceKind::Payback {
                continue;
            }
            let k = race.next[i];
            if k >= n {
                continue;
            }
            // an unordered race: the player clears any checkpoint but the finish, which
            // comes when the others are done
            if c.is_player && race.rivals_checkpoints.is_some() && k + 1 < n {
                let p = c.state.pos;
                let prev = race.prev_pos.get(i).copied().filter(|q| q.distance(p) < 30.0).unwrap_or(p);
                let got = (k..n - 1).find(|&j| {
                    let cp = race.checkpoints[j];
                    match crate::checkpoints::spec_at(&race.cp_specs, cp) {
                        // (GetWhichHitCheckpoint, sub_82263338: the first open one hit)
                        Some(s) => s.hit(crate::checkpoints::ahead_point(p, c.state.vel), prev),
                        None => Vec3::new(p.x - cp.x, 0.0, p.z - cp.z).length() < 24.0 && (p.y - cp.y).abs() < 12.0,
                    }
                });
                if let Some(j) = got {
                    race.checkpoints.swap(k, j);
                    race.next[i] += 1;
                }
                continue;
            }
            let cp = match race.rivals_checkpoints.as_ref() {
                Some(own) if !c.is_player => own[k.min(own.len() - 1)],
                _ => race.checkpoints[k],
            };
            let d = Vec3::new(c.state.pos.x - cp.x, 0.0, c.state.pos.z - cp.z).length();
            let reach = if race.cp_radius.is_empty() { 24.0 } else { race.cp_radius[k % race.cp_radius.len()] };
            let mut hit = d < reach && (c.state.pos.y - cp.y).abs() < 12.0;
            if let Some((a, b)) = race.cp_gate.get(k % race.cp_gate.len().max(1)).copied().flatten() {
                // a gate: drive through between its ends (taken a little wider than set)
                let p = c.state.pos;
                let q = race.prev_pos.get(i).copied().filter(|q| q.distance(p) < 30.0).unwrap_or(p);
                let e = Vec3::new(b.x - a.x, 0.0, b.z - a.z).normalize_or_zero() * 10.0;
                let (a, b) = (a - e, b + e);
                let cross = |o: Vec3, u: Vec3, w: Vec3| (u.x - o.x) * (w.z - o.z) - (u.z - o.z) * (w.x - o.x);
                let through = cross(a, b, q) * cross(a, b, p) <= 0.0 && cross(q, p, a) * cross(q, p, b) <= 0.0 && q.distance(p) > 1e-3;
                hit = (p.y - cp.y).abs() < 14.0 && (through || d < 8.0);
            }
            // the original's test where the race's script set the checkpoint up
            // (HasHitCheckpoint: the way from the car's last position to 2 m ahead of it)
            if let Some(s) = crate::checkpoints::spec_at(&race.cp_specs, cp) {
                let p = c.state.pos;
                let prev = race.prev_pos.get(i).copied().filter(|q| q.distance(p) < 30.0).unwrap_or(p);
                hit = s.hit(crate::checkpoints::ahead_point(p, c.state.vel), prev);
            }
            if hit {
                race.next[i] += 1;
                if race.next[i] >= n {
                    race.finish_time[i] = Some(race.time);
                    if c.is_player {
                        finished_now = true;
                    }
                }
            }
        }
        // (test hook: MCLA_RACE_END=<place>[,<seconds>] ends a race that many seconds after GO
        // (5) with the player in that place: the cars ahead get earlier times)
        if let Ok(v) = std::env::var("MCLA_RACE_END") {
            let mut it = v.split(',');
            let place: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(1);
            let after: f32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(5.0);
            if race.time > after && !finished_now && race.finish_time.iter().all(|t| t.is_none()) {
                let mut ahead = place.saturating_sub(1);
                for (i, c) in vehicles.cars.iter().enumerate() {
                    if i >= race.finish_time.len() || !race.participants.contains(&i) {
                        continue;
                    }
                    if c.is_player {
                        race.finish_time[i] = Some(race.time);
                    } else if ahead > 0 {
                        race.finish_time[i] = Some(race.time - 2.0 * ahead as f32);
                        ahead -= 1;
                    }
                }
                finished_now = true;
            }
        }
        race.prev_pos = vehicles.cars.iter().map(|c| c.state.pos).collect();
        // (a computer racer over the line brakes to a stop)
        for (i, c) in vehicles.cars.iter_mut().enumerate() {
            if !c.is_player && race.finish_time.get(i).map_or(false, |t| t.is_some()) {
                if let Some(ai) = c.ai.as_mut() {
                    ai.finished = true;
                }
            }
        }
        if std::env::var("MCLA_RACE_DUMP").map_or(false, |v| (race.time % v.parse::<f32>().ok().filter(|x| *x > 0.5).unwrap_or(10.0)) < dt) {
            for (i, c) in vehicles.cars.iter().enumerate().filter(|(i, _)| race.participants.contains(i)) {
                let k = race.next.get(i).copied().unwrap_or(0);
                if k < n {
                    let cp = race.checkpoints[k];
                    log::info!("race dump t {:.0}: car {} {} at ({:.0},{:.0},{:.0}) next {} dist {:.0} dy {:.1} speed {:.0} ai path_i {:?}", race.time, i, c.model.name, c.state.pos.x, c.state.pos.y, c.state.pos.z, k, Vec3::new(c.state.pos.x - cp.x, 0.0, c.state.pos.z - cp.z).length(), c.state.pos.y - cp.y, c.state.speed(), c.ai.as_ref().map(|a| a.path_i));
                }
            }
        }
        // rubber banding: opponents pace themselves on the player (original tuning)
        if !self.rubber.is_empty() && n > 0 && race.kind == RaceKind::Race {
            let step = self.rb_step.unwrap_or(0);
            let rb = &self.rubber[step.min(self.rubber.len() - 1)];
            let spacing = if race.total_len > 1.0 { race.total_len / n as f32 } else { 300.0 };
            let metric = |i: usize, pos: Vec3| -> f32 {
                let k = race.next.get(i).copied().unwrap_or(0).min(n);
                let dn = if k < n { race.checkpoints[k].distance(pos) } else { 0.0 };
                k as f32 * spacing - dn
            };
            if let Some((pi, ppos)) = vehicles.cars.iter().enumerate().find(|(_, c)| c.is_player).map(|(i, c)| (i, c.state.pos)) {
                let pm = metric(pi, ppos);
                let to_finish = (n as f32 * spacing - pm).max(0.0);
                let mut order: Vec<(usize, f32)> = vehicles.cars.iter().enumerate().filter(|(_, c)| !c.is_player && c.ai.is_some()).map(|(i, c)| (i, metric(i, c.state.pos))).collect();
                order.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                for (rank, (i, m)) in order.into_iter().enumerate() {
                    let thr = rb.throttle(m - pm, rank, to_finish);
                    let c = &mut vehicles.cars[i];
                    let top = c.model.params.top_speed;
                    if let Some(ai) = c.ai.as_mut() {
                        ai.target_speed = (top * thr.max(rb.min_speed)).max(18.0);
                        if std::env::var("MCLA_SLOW_RIVALS").is_ok() {
                            ai.target_speed = 12.0;
                        }
                    }
                }
            }
        }
        // results when the player finishes
        if finished_now {
            let t = race.finish_time[0];
            if let Some(t) = t {
                if self.best_time.map_or(true, |b| t < b) {
                    self.best_time = Some(t);
                }
            }
        }
        finished_now
    }

    /// Race standings: (car index, finished?, progress metric)
    pub fn standings(&self, vehicles: &Vehicles) -> Vec<(usize, Option<f32>, f32)> {
        let Some(race) = self.race.as_ref() else { return vec![] };
        let mut v: Vec<(usize, Option<f32>, f32)> = Vec::new();
        for &i in &race.participants {
            if i >= vehicles.cars.len() {
                continue;
            }
            let k = race.next.get(i).copied().unwrap_or(0).min(race.checkpoints.len());
            let dist_next = if k < race.checkpoints.len() { race.checkpoints[k].distance(vehicles.cars[i].state.pos) } else { 0.0 };
            let progress = k as f32 * 10000.0 - dist_next;
            v.push((i, race.finish_time.get(i).copied().flatten(), progress));
        }
        v.sort_by(|a, b| match (a.1, b.1) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal),
        });
        v
    }

    pub fn player_position(&self, vehicles: &Vehicles) -> (usize, usize) {
        let s = self.standings(vehicles);
        let pos = s.iter().position(|(i, _, _)| vehicles.cars.get(*i).map_or(false, |c| c.is_player)).unwrap_or(0);
        (pos + 1, s.len())
    }

    /// Text in the game's own typeface (falls back to the UI default when the art is missing).
    /// `size` is the capital height in pixels.
    /// Menu panel in the game's own widget art (the dark rounded plate of the pause scene).
    fn panel(&self, painter: &egui::Painter, rect: egui::Rect, sc: f32) {
        match self.art.as_ref().and_then(|a| a.get("pause_0336")) {
            Some(t) => crate::hud::nine_slice(painter, t, rect, 22.0 * sc, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 235)),
            None => {
                painter.rect_filled(rect, 8.0 * sc, egui::Color32::from_rgba_unmultiplied(8, 10, 16, 215));
            }
        }
    }

    /// Selection plate (the light rounded cursor of the pause scene, tinted).
    fn cursor(&self, painter: &egui::Painter, rect: egui::Rect, sc: f32) {
        let r = rect.shrink2(egui::vec2(10.0 * sc, 2.0 * sc));
        match self.art.as_ref().and_then(|a| a.get("pause_0004")) {
            Some(t) => crate::hud::nine_slice(painter, t, r, 12.0 * sc, egui::Color32::from_rgba_unmultiplied(255, 150, 20, 200)),
            None => {
                painter.rect_filled(r, 6.0 * sc, egui::Color32::from_rgba_unmultiplied(255, 140, 0, 60));
            }
        }
    }

    /// Small icon from the pause scene (dollar, medal, flags, stopwatch ...).
    fn icon(&self, painter: &egui::Painter, name: &str, center: egui::Pos2, size: f32, tint: egui::Color32) {
        if let Some(t) = self.art.as_ref().and_then(|a| a.get(name)) {
            crate::hud::image(painter, t, egui::Rect::from_center_size(center, egui::vec2(size, size)), tint);
        }
    }

    fn text(&self, painter: &egui::Painter, pos: egui::Pos2, align: egui::Align2, s: &str, size: f32, color: egui::Color32) -> egui::Rect {
        match self.art.as_ref().and_then(|a| a.font.as_ref()) {
            Some(f) => f.draw(painter, pos, align, s, size, color),
            None => painter.text(pos, align, s, egui::FontId::proportional(size * 1.35), color),
        }
    }

    fn text_width(&self, s: &str, size: f32) -> f32 {
        match self.art.as_ref().and_then(|a| a.font.as_ref()) {
            Some(f) => f.measure(s, size),
            None => s.len() as f32 * size * 0.7,
        }
    }

    /// Break `s` into lines no wider than `width`.
    fn wrap(&self, s: &str, size: f32, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut cur = String::new();
        for w in s.split_whitespace() {
            let cand = if cur.is_empty() { w.to_string() } else { format!("{cur} {w}") };
            if !cur.is_empty() && self.text_width(&cand, size) > width {
                lines.push(std::mem::take(&mut cur));
                cur = w.to_string();
            } else {
                cur = cand;
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }

    /// The career's mission log (pause screen): active missions with the game's own texts.
    fn mission_log(&self, painter: &egui::Painter, screen: egui::Rect, sc: f32, accent: egui::Color32) {
        let list = self.career.active();
        if list.is_empty() {
            return;
        }
        let w = 640.0 * sc;
        let x0 = screen.right() - w - 70.0 * sc;
        let inner = w - 68.0 * sc;
        // lay the text out first to size the panel
        let mut rows: Vec<(String, f32, egui::Color32, f32)> = Vec::new();
        rows.push((format!("MISSIONS   {}/{}", self.career.completed(), self.career.total()), 22.0, egui::Color32::WHITE, 40.0));
        for m in list.iter().take(4) {
            let head = if m.who.is_empty() { m.title.to_uppercase() } else { format!("{}   {}", m.title.to_uppercase(), m.who.to_uppercase()) };
            rows.push((head, 15.0, accent, 26.0));
            for l in self.wrap(&m.desc.to_uppercase(), 10.0 * sc, inner).into_iter().take(4) {
                rows.push((l, 10.0, egui::Color32::from_gray(205), 17.0));
            }
            for o in &m.objectives {
                let t = if o.target > 1 { format!("- {}   {}/{}", o.text.to_uppercase(), o.cur.min(o.target), o.target) } else { format!("- {}", o.text.to_uppercase()) };
                rows.push((t, 11.0, if o.cur >= o.target { egui::Color32::from_rgb(120, 230, 120) } else { egui::Color32::WHITE }, 20.0));
            }
            if let Some(r) = m.next_race.as_ref() {
                rows.push((format!("  NEXT EVENT   {}", r), 10.0, egui::Color32::from_rgb(120, 200, 255), 18.0));
            }
            rows.push((String::new(), 8.0, egui::Color32::TRANSPARENT, 12.0));
        }
        if list.len() > 4 {
            rows.push((format!("+ {} MORE", list.len() - 4), 10.0, egui::Color32::from_gray(160), 18.0));
        }
        let h: f32 = rows.iter().map(|r| r.3).sum::<f32>() * sc + 56.0 * sc;
        let y0 = (screen.center().y - h * 0.5).max(screen.top() + 30.0 * sc);
        self.panel(painter, egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(w, h)), sc);
        let mut y = y0 + 30.0 * sc;
        for (t, size, col, adv) in rows {
            if !t.is_empty() {
                self.text(painter, egui::pos2(x0 + 34.0 * sc, y), egui::Align2::LEFT_TOP, &t, size * sc, col);
            }
            y += adv * sc;
        }
    }

    /// Draw menus / HUD. Returns the action chosen this frame.
    pub fn ui(&mut self, ctx: &egui::Context, vehicles: &Vehicles, world: &World, hud: HudInfo) -> MenuAction {
        let race_dt = std::mem::take(&mut self.ui_sim_dt);
        if !self.art_tried && !self.data_dir.is_empty() {
            self.art_tried = true;
            self.art = Some(crate::hud::UiArt::load(ctx, &self.data_dir));
            if std::env::var("MCLA_FLASH_HUD").map_or(true, |v| v != "0") {
                let shared = crate::flash_ui::UiShared { textures: std::sync::Arc::new(crate::flash_ui::load_shared(&self.data_dir)), strings: std::sync::Arc::new(self.career.strings.clone()), logic: std::sync::Arc::new(crate::menu3d::UiLogic::load(&self.data_dir)) };
                self.flash_hud = crate::flash_ui::FlashHud::load(&self.data_dir, &shared);
                self.flash_map = crate::flash_ui::FlashMinimap::load(&self.data_dir, &shared);
                // (MCLA_OLD_MINIMAP=1: the port's own drawing of the map)
                if std::env::var("MCLA_OLD_MINIMAP").is_err() {
                    let mut hm = crate::hudmap::State::new(&self.data_dir);
                    hm.street = crate::hudmap::StreetNames::load(&self.data_dir, &shared);
                    self.hm = Some(hm);
                }
                self.flash_menu = crate::flash_ui::FlashMenu::load(&self.data_dir, crate::flash_ui::PAUSE_MOVIE, [70.0, 104.0], &shared);
                // (the garage panel sits at the top right of the screen)
                self.flash_garage = crate::flash_ui::FlashMenu::load(&self.data_dir, crate::flash_ui::GARAGE_MOVIE, [760.0, 40.0], &shared);
                // the pause cameras, as the game hands them to the movie (0x82208B08)
                if let Some(fm) = self.flash_menu.as_mut() {
                    shared.logic.write_camera_coord(&mut fm.movie.player);
                }
                self.ui_meshes = Some(crate::menu3d::Meshes::new(&self.data_dir));
                self.flash_prompt = crate::flash_ui::FlashPrompt::load(&self.data_dir, &shared);
                self.flash_phone = crate::flash_ui::FlashPhone::load(&self.data_dir, &shared);
                self.flash_navsys = crate::flash_ui::FlashMenu::load(&self.data_dir, crate::flash_ui::NAVSYS_MOVIE, [0.0, 0.0], &shared);
                self.road_map = crate::mapmesh::RoadMap::load(&self.data_dir);
                self.flash_popup = crate::flash_ui::FlashMenu::load(&self.data_dir, crate::flash_ui::POPUP_MOVIE, [0.0, 0.0], &shared);
                self.ui_shared = Some(shared);
            }
        }
        if !matches!(self.mode, Mode::Paused | Mode::Results) && !(self.mode == Mode::RaceSelect && self.hangout_key.is_some()) {
            if let Some(fm) = self.flash_menu.as_mut() {
                fm.hide();
            }
            self.pause_cam = None;
        }
        if !matches!(self.mode, Mode::Garage) {
            if let Some(fg) = self.flash_garage.as_mut() {
                fg.hide();
            }
            // (the start screen shows its PRESS START on the bar)
            if !matches!(self.mode, Mode::MainMenu) {
                if let Some(fp) = self.flash_prompt.as_mut() {
                    fp.hide();
                }
            }
        }
        if self.photo_on {
            // nothing on screen but the hint (and not even that in the picture itself)
            if let Some(h) = self.photo.as_ref() {
                let screen = ctx.screen_rect();
                let sc = (screen.height() / 900.0).clamp(0.5, 2.0);
                let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("photo")));
                self.text(&p, egui::pos2(screen.center().x + 1.5, screen.bottom() - 38.5 * sc), egui::Align2::CENTER_CENTER, h, 11.0 * sc, egui::Color32::BLACK);
                self.text(&p, egui::pos2(screen.center().x, screen.bottom() - 40.0 * sc), egui::Align2::CENTER_CENTER, h, 11.0 * sc, egui::Color32::WHITE);
            }
            return MenuAction::None;
        }
        if let Some(mut m) = self.movie.take() {
            let skip = std::mem::take(&mut self.nav_enter);
            if !skip && m.show(ctx) {
                // subtitles of the opening movie
                let screen = ctx.screen_rect();
                let sc = (screen.height() / 900.0).clamp(0.5, 2.0);
                let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("movie subs")));
                if let Some(sub) = m.subtitle() {
                    let lines = self.wrap(sub, 15.0 * sc, screen.width() * 0.8);
                    for (i, l) in lines.iter().enumerate() {
                        let pos = egui::pos2(screen.center().x, screen.bottom() - 70.0 * sc + (i as f32 - (lines.len() as f32 - 1.0) * 0.5) * 24.0 * sc);
                        self.text(&p, pos + egui::vec2(1.5, 1.5), egui::Align2::CENTER_CENTER, l, 15.0 * sc, egui::Color32::BLACK);
                        self.text(&p, pos, egui::Align2::CENTER_CENTER, l, 15.0 * sc, egui::Color32::WHITE);
                    }
                }
                if m.time() < 4.0 {
                    self.text(&p, egui::pos2(screen.right() - 30.0 * sc, screen.top() + 30.0 * sc), egui::Align2::RIGHT_CENTER, if m.any_key { "PRESS ANY KEY" } else { "ENTER  SKIP" }, 9.0 * sc, egui::Color32::from_gray(150));
                }
                self.movie = Some(m);
            } else {
                self.movie_done = true;
            }
            return MenuAction::None;
        }
        let mut action = MenuAction::None;
        match self.pause_do.take() {
            Some(254) => {
                self.standings_view = true;
                self.pause_page = 0;
            }
            Some(253) => {
                // VIEW REPLAY from a running race's pause
                self.pause_page = 0;
                self.start_replay();
                if let Some(fm) = self.flash_menu.as_mut() {
                    fm.hide();
                }
            }
            Some(250) => {
                // NEXT RACE: the race after this one on ARCADE's list of its kind, with the same options
                if let Some(cur) = self.current_race {
                    let kind = self.arcade.1;
                    let list = self.arcade_races(kind);
                    if let Some(at) = list.iter().position(|(i, _)| *i == cur) {
                        let next = list[(at + 1) % list.len()].0;
                        if let Some((i, n)) = self.arcade_restore.take() {
                            self.race_defs[i].opponents = n;
                        }
                        self.arcade_restore = Some((next, self.race_defs[next].opponents));
                        self.race_defs[next].opponents = self.arcade_opts[0] + 1;
                        self.race_sel = next;
                        self.current_race = Some(next);
                        self.wager = None;
                        action = MenuAction::StartRace;
                        self.next_race_start = true;
                        self.pause_page = 0;
                    }
                }
            }
            Some(252) => {
                // SAVE GAME
                let d = self.data_dir.clone();
                self.write_save(&d);
            }
            Some(251) => {
                // ADD RACE TO FAVORITES / REMOVE RACE FROM FAVORITES
                if let Some(id) = self.current_race.and_then(|i| self.race_defs.get(i)).map(|d| d.id.clone()) {
                    let name = self.career.strings.get(&id).map(|v| v.to_uppercase()).unwrap_or_else(|| id.clone());
                    if let Some(at) = self.race_favs.iter().position(|f| f.eq_ignore_ascii_case(&id)) {
                        self.race_favs.remove(at);
                        self.show_message(&format!("{} REMOVED FROM FAVORITES", name));
                    } else {
                        self.race_favs.push(id);
                        self.show_message(&format!("{} ADDED TO FAVORITES", name));
                    }
                    self.dirty = true;
                }
            }
            _ => {}
        }
        let screen = ctx.screen_rect();
        let dark = egui::Color32::from_rgba_unmultiplied(8, 10, 16, 215);
        let accent = egui::Color32::from_rgb(255, 140, 0);
        // Test hook: MCLA_NAV="w300 S D A w120 B ..." plays pad presses into the menus -
        // U D L R (stick), A (accept), B (back), Q E (LB RB), S (Start: pause), wN (wait N
        // frames; 40 between presses otherwise) - for walk-throughs without a pad.
        if let Ok(script) = std::env::var("MCLA_NAV") {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static FRAME: AtomicUsize = AtomicUsize::new(0);
            let frame = FRAME.fetch_add(1, Ordering::Relaxed);
            let mut at = 0usize;
            for tok in script.split_whitespace() {
                if let Some(n) = tok.strip_prefix('w').and_then(|n| n.parse::<usize>().ok()) {
                    at += n;
                    continue;
                }
                // (`Z`: wait here until the race is over - the results are up - however long that takes)
                if tok == "Z" {
                    static HOLD: AtomicUsize = AtomicUsize::new(0);
                    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    if frame >= at + HOLD.load(Ordering::Relaxed) && !DONE.load(Ordering::Relaxed) {
                        if self.mode != Mode::Results {
                            HOLD.store(frame - at + 1, Ordering::Relaxed);
                        } else {
                            DONE.store(true, Ordering::Relaxed);
                        }
                    }
                    at += HOLD.load(Ordering::Relaxed);
                    continue;
                }
                at += 40;
                if at == frame {
                    match tok {
                        "U" => self.nav_up = true,
                        "D" => self.nav_down = true,
                        "L" => self.nav_left = true,
                        "R" => self.nav_right = true,
                        "A" => self.nav_enter = true,
                        "B" => self.nav_back = true,
                        "X" => self.nav_x = true,
                        "Y" => self.nav_y = true,
                        "K" => self.nav_select = true,
                        "M" => self.nav_map = true,
                        // (`O`: the MAP action - D-pad up, the overhead map)
                        "O" if matches!(self.mode, Mode::FreeRoam | Mode::Race) => self.map_button(),
                        // (`P`: the screenshot is taken now, whatever --frames says)
                        "P" => self.shot_now = true,
                        "T" => self.nav_start = true,
                        "Q" => self.nav_tab_prev = true,
                        "E" => self.nav_tab_next = true,
                        "S" if matches!(self.mode, Mode::FreeRoam | Mode::Race) => {
                            self.mode = Mode::Paused;
                            self.pause_page = 0;
                            self.pause_tab = 0;
                            self.menu_sel = 0;
                        }
                        _ => {}
                    }
                    log::info!("nav: frame {} key {} (mode {:?}, page {}, tab {}, sel {}, garage menu {} cur {})", frame, tok, self.mode as u8, self.pause_page, self.pause_tab, self.menu_sel, self.garage_menu, self.garage_cur);
                }
            }
        }
        let up = std::mem::take(&mut self.nav_up);
        let down = std::mem::take(&mut self.nav_down);
        let left = std::mem::take(&mut self.nav_left);
        let right = std::mem::take(&mut self.nav_right);
        let enter = std::mem::take(&mut self.nav_enter);
        let back = std::mem::take(&mut self.nav_back);
        let tab_prev = std::mem::take(&mut self.nav_tab_prev);
        let tab_next = std::mem::take(&mut self.nav_tab_next);
        // (test hook: MCLA_BOX_TEST=<box id of MessageBoxes.sc.xml> opens that box once)
        // (test hook: MCLA_SONG_TEST=<text> runs that text across the HUD's info line)
        if let Ok(t) = std::env::var("MCLA_SONG_TEST") {
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if matches!(self.mode, Mode::FreeRoam) && !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                self.song_line = Some((t, 0.0));
            }
        }
        // (test hook: MCLA_RESULTS_TEST shows the race-over screen with made-up results)
        if std::env::var("MCLA_RESULTS_TEST").is_ok() && matches!(self.mode, Mode::FreeRoam) && !self.go_garage && !self.results_test_done {
            self.results_test_done = std::env::var("MCLA_RESULTS_TEST").map_or(false, |v| v.contains("once"));
            self.last_results = vec![("YOU".to_string(), Some(187.42)), ("Andrew".to_string(), Some(189.9)), ("Karol".to_string(), Some(195.31)), ("Booke".to_string(), None)];
            self.mode = Mode::Results;
            // (MCLA_RESULTS_TEST=reward: with the REWARDS panel; =arcade: as after an ARCADE race)
            let v = std::env::var("MCLA_RESULTS_TEST").unwrap_or_default();
            if v.contains("reward") {
                self.last_reward = (0, 30);
                self.reward_panel = true;
            }
            // (=mission: a career race's panels - REWARDS with money, then MISSION RESULTS)
            if v.contains("place") {
                self.place_splash = Some((if v.contains("dnf") { 0 } else { 2 }, 30.0));
                self.place_tied = v.contains("tied");
            }
            if v.contains("mission") {
                self.rep = 50;
                self.money = 5100;
                self.last_reward = (100, 50);
                self.reward_panel = true;
                self.mission_results.push(("FIRST IMPRESSIONS".to_string(), vec!["RACE HENRY".to_string()], !v.contains("fail"))); self.career_over = true;
            }
            if v.contains("arcade") {
                self.last_results = vec![("Henry".to_string(), Some(64.16)), ("Mikey".to_string(), Some(68.71)), ("Julian".to_string(), Some(72.46)), ("YOU".to_string(), Some(1338.84))];
                self.arcade_restore = self.arcade_restore.or(Some((0, self.race_defs.first().map_or(3, |d| d.opponents))));
            }
        }
        // A new career opens on the game's own boxes (SaveGame.sc.xml; references
        // `ng_alert_nosave`, `ng_warning_autosave`): SG_NoGamesFound - ALERT, "A Midnight Club:
        // Los Angeles save game could not be found. What would you like to do?", CREATE A
        // NEW SAVE GAME / RETRY / CONTINUE WITHOUT SAVING - and behind the last one
        // SG_AutoSaveDisabled - WARNING, "Auto save has been disabled...", CONTINUE.
        if self.fresh_game && self.mode == Mode::FreeRoam && self.flash_popup.is_some() && self.message_box.is_none() && std::env::var("MCLA_NO_SAVE_BOX").is_err() {
            if self.save_box == 0 {
                self.save_box = 1;
                self.message_box = Some((
                    crate::flash_ui::MessageBoxDef { title: "Save_Dialog_Title_Message".into(), description: "Save_Dialog_NoGamesFound".into(), options: vec![("create".into(), "Save_CreateNewSave".into()), ("retry".into(), "Common_Retry".into()), ("nosave".into(), "Common_ContinueWithoutSaving".into())] },
                    0,
                    MenuAction::None,
                ));
            } else if self.save_box == 1 {
                // (the choice made: `box_sel` holds it)
                self.save_box = 2;
                match self.box_sel {
                    1 => self.save_box = 0, // RETRY: the same question again
                    2 => {
                        self.no_save = true;
                        self.message_box = Some((crate::flash_ui::MessageBoxDef { title: "Save_Dialog_Title_Warning".into(), description: "Save_Dialog_SaveDisabled".into(), options: Vec::new() }, 0, MenuAction::None));
                    }
                    _ => {}
                }
            }
        }
        // (test hook: MCLA_TUT_TEST=<name> raises that tutorial's box - target, mission_log, sidekick)
        if let Ok(k) = std::env::var("MCLA_TUT_TEST") {
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if self.mode == Mode::FreeRoam && !self.fresh_game && !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                self.tutorial_box = Some((format!("tutorial_{}_title", k), format!("tutorial_{}_desc", k), 0.5, 600.0));
                self.career.stats.insert(format!("tut:{}", k), 1);
            }
        }
        // (test hook: MCLA_HANGOUT_TEST=<key> opens that hangout's panel, e.g. hw_local_hangout1)
        if let Ok(k) = std::env::var("MCLA_HANGOUT_TEST") {
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if self.mode == Mode::FreeRoam && !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                self.hangout_key = Some(k);
                self.mode = Mode::RaceSelect;
            }
        }
        // (test hook: MCLA_REPLAY_TEST="secs[,t,view,paused,legend,menu]" ends a race after
        // `secs` of it and opens its replay - at `t` seconds, on view `view`, paused (1),
        // with the legend's state and the PAUSE panel (1) asked for)
        if let Ok(v) = std::env::var("MCLA_REPLAY_TEST") {
            let v: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if self.mode == Mode::Race && self.race.as_ref().map_or(false, |r| r.time >= v.first().copied().unwrap_or(20.0)) {
                self.last_results = vec![("YOU".to_string(), Some(64.16)), ("Mikey".to_string(), Some(68.71))];
                self.mode = Mode::Results;
                self.arcade_restore = self.arcade_restore.or(Some((0, 3)));
                self.start_replay();
                if let Some(pb) = self.replay.as_mut() {
                    pb.auto_cam = v.len() < 3;
                    if v.len() >= 3 {
                        pb.set_view(v[2] as usize);
                    }
                    pb.t = v.get(1).copied().unwrap_or(0.0);
                    // (a time given: straight there, without the race's opening)
                    if v.len() >= 2 {
                        pb.opening = 0;
                    }
                    pb.paused = v.get(3).map_or(false, |x| *x > 0.0);
                    pb.legend = v.get(4).map_or(0, |x| *x as u8);
                    pb.menu = v.get(5).filter(|x| **x > 0.0).map(|_| 0);
                }
            }
        }
        if let Ok(id) = std::env::var("MCLA_BOX_TEST") {
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if self.flash_popup.is_some() && !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                self.open_message_box(&id, MenuAction::None);
            }
        }
        // a message box (popup movie) takes the input while it is up
        let boxed = self.message_box.is_some();
        if let Some(a) = self.message_box_ui(ctx, up, down, enter, back) {
            action = a;
            // (YES in the garage's SELL VEHICLE box: `garage.FinishVehicleSale`)
            if let Some(car) = self.sell_pending.take() {
                self.sell_car(car);
            }
            // (YES in the paint shop's / vinyl editor's purchase box: `garage.FinishMiscPurchase`,
            // then into the shop - goto(PaintWholeCar) / goto(VE_Wait_Enter))
            if let Some(n) = self.mc4_misc_finish() {
                self.mc4_misc_enter(n);
            }
            // (YES in the garage's REPAIR box: `game.repairCar()`, then on)
            self.mc4_repair_finish();
            // (YES in a vinyl editor box: LoadPresetVinyl / SavePresetVinyl - step 22)
            self.mc4_vinyl_finish();
            // (YES in the showroom's box: `garage.FinishShowroomPurchase`; with a trade-in
            // the car in the garage goes for its trade-in value first)
            if let Some((car, trade)) = self.buy_pending.take() {
                if trade && self.owned.len() >= 1 {
                    let old = self.current_car;
                    let name = self.dir_name(old);
                    self.owned.retain(|o| !o.eq_ignore_ascii_case(&name));
                    self.money += self.trade_in(old);
                }
                self.selected_car = car;
                self.buy_confirmed = true;
            }
        }
        if self.message_box.is_none() {
            self.sell_pending = None;
            self.buy_pending = None;
            self.mc4_misc_idle();
        }
        let (up, down, left, right, enter, back, tab_prev, tab_next) = if boxed { (false, false, false, false, false, false, false, false) } else { (up, down, left, right, enter, back, tab_prev, tab_next) };

        // while driving with the HUD movie, messages go to its splash queue (the top
        // slot, white; the game posts e.g. `splash_damaged_out` there for 2 s)
        let hud_splash = self.flash_hud.is_some() && matches!(self.mode, Mode::FreeRoam | Mode::Race);
        if hud_splash && !self.message_posted && self.message_timer > 0.0 && !self.message.is_empty() {
            let m = self.message.to_uppercase().replace('_', " ");
            if let Some(fh) = self.flash_hud.as_mut() {
                fh.splash(crate::flash_ui::SPLASH_TOP, &m, 0, 3.0);
            }
            self.message_posted = true;
        }
        if self.message_timer > 0.0 && !self.message.is_empty() && !(hud_splash && self.message_posted) {
            // messages in the game's own typeface, fading out
            let sc = (screen.height() / 900.0).clamp(0.5, 2.0);
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("msg")));
            let a = (self.message_timer.min(0.6) / 0.6 * 255.0) as u8;
            let col = egui::Color32::from_rgba_unmultiplied(255, 140, 0, a);
            let msg = self.message.to_uppercase().replace("_", " ");
            self.text(&painter, egui::pos2(screen.center().x, screen.top() + 118.0 * sc), egui::Align2::CENTER_TOP, &msg, 24.0 * sc, col);
        }

        // the mechanic's call: a card at the lower left with the tip written out
        // With the phone movie the call comes in on the phone, as in the original: the
        // message on its screen, the phone sliding up at the lower left.
        // (not over the black ARRESTED / TICKETED panel)
        let panel_up = self.arrest_panel.map_or(false, |p| p.0 <= 0.0) || (self.mode == Mode::Paused && self.flash_menu.is_some());
        if let (Some(fp), Some(meshes), false) = (self.flash_phone.as_mut(), self.ui_meshes.as_mut(), panel_up) {
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            // (the phone is put away for an event: none of the race captures shows it)
            let msg = self.tip.as_ref().filter(|_| self.race.is_none()).map(|(title, body, _)| (title.to_uppercase(), body.to_uppercase()));
            if let Some((title, body, left)) = self.tip.clone() {
                self.tip = if left > dt && !matches!(self.mode, Mode::MainMenu) { Some((title, body, left - dt)) } else { None };
            }
            if msg.is_some() || fp.is_active() {
                // The caller's portrait. The frames run in the order of the career's hookman
                // ids - SH_01..05 (Booke, Karol, Annie, Andrew, Hugo) = 0..4, GH_01..25 = 5..29,
                // 30 and 31 the gamer pictures, GH_26.. = 32.. - checked on the reference capture
                // (CHUNG HEE, GH_13, shows frame 17). A tip without a caller is the garage's
                // Karol (approx.).
                let caller = msg.as_ref().map(|m| m.0.clone()).unwrap_or_default();
                let by_name = (1..=5).map(|n| (format!("SH_{:02}", n), n - 1)).chain((1..=38).map(|n| (format!("GH_{:02}", n), if n <= 25 { n + 4 } else { n + 6 }))).find(|(k, _)| self.career.strings.get(k).map_or(false, |v| v.eq_ignore_ascii_case(&caller)));
                let icon = std::env::var("MCLA_PHONE_ICON").ok().and_then(|v| v.parse().ok()).or(by_name.map(|x| x.1)).unwrap_or(1);
                let prompt = if msg.is_some() { self.phone_prompt.clone() } else { String::new() };
                fp.drive(msg.as_ref().map(|(t, b)| (t.as_str(), b.as_str(), icon)), &prompt, hud.night, dt);
                if let Some(scene) = crate::menu3d::read_scene(&mut fp.movie.player) {
                    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("flash_phone"));
                    crate::menu3d::paint(ctx, &ctx.layer_painter(layer), meshes, &mut fp.movie, &scene, None);
                }
            }
        } else if panel_up && self.flash_phone.is_some() {
            // (the call waits behind the panel)
        } else if let Some((title, body, left)) = self.tip.clone() {
            let dt = ctx.input(|i| i.stable_dt);
            self.tip = if left > dt && !matches!(self.mode, Mode::MainMenu) { Some((title.clone(), body.clone(), left - dt)) } else { None };
            let sc = (screen.height() / 900.0).clamp(0.5, 2.0);
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("tip")));
            let w = 430.0 * sc;
            let size = 15.0 * sc;
            // wrap by measured width
            let mut lines: Vec<String> = Vec::new();
            for para in body.lines() {
                let mut cur = String::new();
                for word in para.split_whitespace() {
                    let t = if cur.is_empty() { word.to_string() } else { format!("{} {}", cur, word) };
                    if self.text_width(&t, size) > w - 28.0 * sc && !cur.is_empty() {
                        lines.push(std::mem::take(&mut cur));
                        cur = word.to_string();
                    } else {
                        cur = t;
                    }
                }
                lines.push(cur);
            }
            let h = 62.0 * sc + lines.len() as f32 * size * 1.45;
            let a = (left.min(0.5) / 0.5).clamp(0.0, 1.0);
            let x0 = screen.left() + 40.0 * sc;
            let y0 = screen.bottom() - 250.0 * sc - h;
            let rect = egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(w, h));
            painter.rect_filled(rect, 6.0 * sc, egui::Color32::from_rgba_unmultiplied(8, 10, 16, (215.0 * a) as u8));
            painter.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(w, 3.0 * sc)), 0.0, egui::Color32::from_rgba_unmultiplied(255, 140, 0, (255.0 * a) as u8));
            let orange = egui::Color32::from_rgba_unmultiplied(255, 140, 0, (255.0 * a) as u8);
            let white = egui::Color32::from_rgba_unmultiplied(235, 238, 245, (255.0 * a) as u8);
            self.text(&painter, egui::pos2(x0 + 14.0 * sc, y0 + 12.0 * sc), egui::Align2::LEFT_TOP, "INCOMING CALL!", 12.0 * sc, white);
            self.text(&painter, egui::pos2(x0 + 14.0 * sc, y0 + 28.0 * sc), egui::Align2::LEFT_TOP, &title.to_uppercase(), 18.0 * sc, orange);
            for (i, l) in lines.iter().enumerate() {
                self.text(&painter, egui::pos2(x0 + 14.0 * sc, y0 + 56.0 * sc + i as f32 * size * 1.45), egui::Align2::LEFT_TOP, l, size, white);
            }
        }

        match self.mode {
            Mode::MainMenu => {
                // the start screen: the original has no main menu, the career begins right away
                let sc = (screen.height() / 900.0).clamp(0.6, 2.0);
                // The original's start screen is the city under a slow camera with nothing on
                // it but the prompt bar's "PRESS (Start)" at the bottom right (reference `t00`).
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("menu")));
                let painter = if let Some(fp) = self.flash_prompt.as_mut() {
                    let press = self.career.strings.get("prompt_press").map(|v| v.to_uppercase()).unwrap_or_else(|| "PRESS".into());
                    fp.show(&[(press, crate::flash_ui::ICON_START)], None, None);
                    fp.movie.player.update(ctx.input(|i| i.stable_dt).min(0.1));
                    fp.movie.paint(ctx, &painter);
                    // (the port's own title is not drawn then)
                    painter.with_clip_rect(egui::Rect::NOTHING)
                } else {
                    painter
                };
                let band = egui::Rect::from_min_max(egui::pos2(screen.left(), screen.bottom() - 250.0 * sc), egui::pos2(screen.right(), screen.bottom() - 90.0 * sc));
                painter.rect_filled(band, 0.0, egui::Color32::from_rgba_unmultiplied(6, 8, 12, 170));
                let cx = screen.center().x;
                self.text(&painter, egui::pos2(cx, band.top() + 22.0 * sc), egui::Align2::CENTER_TOP, "MIDNIGHT CLUB", 44.0 * sc, egui::Color32::WHITE);
                self.text(&painter, egui::pos2(cx, band.top() + 78.0 * sc), egui::Align2::CENTER_TOP, "LOS ANGELES", 24.0 * sc, accent);
                let t = ctx.input(|i| i.time);
                let a = (150.0 + 105.0 * (t * 3.0).sin()) as u8;
                self.text(&painter, egui::pos2(cx, band.top() + 122.0 * sc), egui::Align2::CENTER_TOP, "PRESS ENTER", 16.0 * sc, egui::Color32::from_rgba_unmultiplied(255, 255, 255, a));
                let clicked = ctx.input(|i| i.pointer.primary_clicked());
                let _ = back;
                if enter || clicked {
                    action = MenuAction::StartFreeRoam;
                }
            }
            Mode::Paused if self.replay.is_some() => self.replay_ui(ctx, up, down, left, right, enter, back),
            Mode::Paused if self.standings_view => {
                // SERIES STANDINGS: the last race's table, BACK (`sr_standings`)
                self.results_table = true;
                self.results_movie(ctx, &[], 0);
                if back || enter {
                    self.standings_view = false;
                    self.results_table = false;
                }
            }
            Mode::Paused => {
                // (the controller pages, `mc4` step 13b: what they answer of this frame's presses
                // is taken; `ctrl_page` is the page they show)
                let [up, down, enter, back] = self.mc4_ctrl_input([up, down, enter, back], ctx.input(|i| i.key_pressed(egui::Key::X)), ctx.input(|i| i.key_pressed(egui::Key::Y)));
                let mut ctrl_page: Option<crate::mc4::CtrlPage> = None;
                // The original pause menu: three tabs (PAUSE, MODES, SETTINGS) with the entries
                // and texts of `tune/ui/PauseMenu.sc.xml`.
                #[derive(Clone, Copy, PartialEq)]
                enum Go {
                    Act(MenuAction),
                    Page(u8),
                    /// a statistics page and what it is about (`stat_idx[level]` = index)
                    Sub(u8, usize, usize),
                    Off,
                }
                let s = |key: &str, fallback: &str| -> String { self.career.strings.get(key).filter(|v| !v.contains('<')).map(|v| v.to_string()).unwrap_or_else(|| fallback.to_string()).to_uppercase() };
                let racing = self.race.is_some() || self.flag.is_some();
                let kind = self.race.as_ref().map(|r| r.kind);
                let found = self.collected.len();
                // (the line under a page's title: the vehicle, race or district it is about)
                let mut sub_title = String::new();
                let long_name = |i: usize| self.career.strings.get(&self.dir_name(i).to_ascii_lowercase()).map(|v| v.to_uppercase()).unwrap_or_else(|| self.car_name(i).to_uppercase());
                const CHEATS: [(&str, &str, usize); 6] = [
                    ("cheat_Unlimited_Nitro", "UNLIMITED NITRO", 10),
                    ("cheat_Unlimited_Damage", "NEVER DAMAGE OUT", 20),
                    ("cheat_Unlimited_Special", "UNLIMITED SPECIAL ABILITIES", 30),
                    ("cheat_TopDown_Race", "TOP DOWN VIEW", 40),
                    ("cheat_No_Cops", "NO POLICE", 50),
                    ("cheat_No_charges", "NO FINES", 60),
                ];
                let mut title = String::new();
                let mut items: Vec<(String, String, Go)> = Vec::new();
                match self.pause_page {
                    1 if self.flash_menu.is_some() => {
                        // the original's STATISTICS: six pages (the port's figures stand under the
                        // first; the others are not kept by the port and are greyed)
                        title = s("pause_statistics", "STATISTICS");
                        items.push((s("Pause_Game_Stats", "GAME STATS"), String::new(), Go::Page(9)));
                        items.push((s("Pause_Car_Stats", "VEHICLE STATS"), String::new(), Go::Sub(60, 0, 0)));
                        items.push((s("Pause_Race_Stats", "RACE STATS"), String::new(), Go::Sub(62, 0, 0)));
                        items.push((s("Pause_Career_Stats", "CAREER STATS"), String::new(), Go::Sub(65, 0, 0)));
                        // (ACHIEVEMENTS opens the console's own list: nothing here)
                        items.push((s("Pause_Achievements", "ACHIEVEMENTS"), String::new(), Go::Sub(1, 0, 0)));
                        items.push((s("Pause_Collectables", "COLLECTABLES"), String::new(), Go::Sub(66, 0, 0)));
                    }
                    // The statistics pages (references `w01`..`w12`, `x00`..`x11`). The game keeps
                    // its statistics under coded keys, `stat_<page><code>_<id>` in the string
                    // table: page g = game, k = career; a code's first character says what it
                    // counts ($ cash, % percent, : a time, h hours, k / m distance, s seconds,
                    // v speed). A page lists its keys in byte order, ids by number.
                    9 | 65 if self.flash_menu.is_some() => {
                        let cat = if self.pause_page == 9 { 'g' } else { 'k' };
                        title = if cat == 'g' { s("Pause_Game_Stats", "GAME STATS") } else { s("Pause_Career_Stats", "CAREER STATS") };
                        let mut keys: Vec<(String, u32, String)> = self
                            .career
                            .strings
                            .keys
                            .iter()
                            .filter_map(|k| {
                                let rest = k.strip_prefix("stat_")?;
                                let (code, id) = rest.rsplit_once('_')?;
                                (code.len() == 4 && code.starts_with(cat)).then(|| (code.to_string(), id.parse().unwrap_or(0), k.clone()))
                            })
                            .collect();
                        // (and the figures of an imported save whose names only the smaller tables hold,
                        // by hash: SOUTH CENTRAL WAGER WIN AMOUNT, reference `x06`)
                        for k in self.career.ostats.keys() {
                            let Some((code, sub)) = k.split_once('/') else { continue };
                            let key = format!("stat_{code}_{sub}");
                            if code.len() == 4 && code.starts_with(cat) && sub.parse::<u32>().is_ok() && self.career.strings.get(&key).map_or(false, |v| !v.contains('<')) {
                                keys.push((code.to_string(), sub.parse().unwrap_or(0), key));
                            }
                        }
                        keys.sort();
                        keys.dedup();
                        // (not on the original's list: reference `w01`..`w12`)
                        keys.retain(|k| !matches!(k.0.as_str(), "g:LO" | "gDIP" | "gsAT"));
                        let pct = self.career.percent_complete().max(0) as usize;
                        let money = |v: i64| {
                            let d = v.abs().to_string();
                            let mut out = String::new();
                            for (i, c) in d.chars().enumerate() {
                                if i > 0 && (d.len() - i) % 3 == 0 {
                                    out.push(',');
                                }
                                out.push(c);
                            }
                            format!("${}", out)
                        };
                        for (code, id, key) in keys {
                            // The figure: the port's own count under the same id where it keeps one, else
                            // what an imported original save holds (`Career::stat`). The forms are the
                            // game's (references `w01`..`w12`, the same save): hours as H:MM, distances
                            // (k) with one decimal and MLS, metres (m) as feet, seconds with two decimals.
                            let raw = self.career.stat(&code, &id.to_string()).unwrap_or(0.0);
                            let raw = match code.as_str() {
                                "gCOL" => raw.max(found as f64),
                                "k%AG" => raw.max(pct as f64),
                                _ => raw,
                            };
                            let v = raw as i64;
                            let value = match code.as_bytes()[1] {
                                b'$' => money(v),
                                b'%' => format!("{}%", v),
                                b':' => stat_time(v),
                                b'h' => format!("{}:{:02}", raw.floor() as i64, (raw.fract() * 60.0) as i64),
                                b'k' => format!("{:.1}MLS", raw),
                                b'm' => format!("{:.1}FT", raw * 3.28084),
                                b's' => format!("{:.2}S", raw),
                                _ => if raw.fract() != 0.0 { format!("{:.1}", raw) } else { v.to_string() },
                            };
                            items.push((s(&key, &code), value, Go::Off));
                        }
                    }
                    60 => {
                        title = s("Pause_Car_Stats", "VEHICLE STATS");
                        for (k, i) in self.stat_vehicles().into_iter().enumerate() {
                            items.push((long_name(i), String::new(), Go::Sub(61, 0, k)));
                        }
                    }
                    61 => {
                        title = s("Pause_Car_Stats", "VEHICLE STATS");
                        let owned: Vec<usize> = self.stat_vehicles();
                        if !owned.is_empty() {
                            // (LT / RT - left / right here - to the next vehicle)
                            let at = if left || right { (self.stat_idx[0] + if right { 1 } else { owned.len() - 1 }) % owned.len() } else { self.stat_idx[0].min(owned.len() - 1) };
                            sub_title = long_name(owned[at]);
                            self.stat_idx[0] = at;
                        }
                        // (the port keeps no figures by vehicle: those of an imported original save)
                        let car = owned.get(self.stat_idx[0]).map(|i| self.dir_name(*i).to_ascii_lowercase()).unwrap_or_default();
                        let wins = self.career.stat("vWIN", &car).unwrap_or(0.0);
                        let dist = self.career.stat("vkDT", &car).unwrap_or(0.0);
                        items.push((s("stat_kWIN_0", "NUMBER OF WINS"), format!("{}", wins as i64), Go::Off));
                        items.push((s("Stat_Distance_Travled", "DISTANCE TRAVELED"), format!("{:.1}MLS", dist), Go::Off));
                    }
                    62 => {
                        title = s("Pause_Race_Stats", "RACE STATS");
                        for (k, (key, fb)) in [("ordered_race", "ORDERED RACES"), ("circuit_race", "CIRCUIT RACES"), ("unordered_race", "UNORDERED RACES"), ("blitz_race", "LANDMARK RACES")].iter().enumerate() {
                            items.push((s(key, fb), String::new(), Go::Sub(63, 0, k)));
                        }
                    }
                    63 => {
                        title = s("Pause_Race_Stats", "RACE STATS");
                        for (k, (_, name)) in self.arcade_races(self.stat_idx[0]).iter().enumerate() {
                            items.push((name.clone(), String::new(), Go::Sub(64, 1, k)));
                        }
                    }
                    64 => {
                        title = s("Pause_Race_Stats", "RACE STATS");
                        let list = self.arcade_races(self.stat_idx[0]);
                        if !list.is_empty() {
                            if left || right {
                                self.stat_idx[1] = (self.stat_idx[1] + if right { 1 } else { list.len() - 1 }) % list.len();
                            }
                            sub_title = list[self.stat_idx[1].min(list.len() - 1)].1.clone();
                        }
                        // The race's board (reference `x05`: AROUND HOLLYWOOD 01:58.13, four times
                        // COMPLETED): the statistics r:TI, rBTT, rCGL, rMDG and rWIN under the race's
                        // place in the game's race table. (The port keeps no board of its own: the
                        // figures are an imported original save's.)
                        let sub = list.get(self.stat_idx[1].min(list.len().saturating_sub(1))).and_then(|(i, _)| self.race_table.iter().position(|id| id.eq_ignore_ascii_case(&self.race_defs[*i].id))).map(|k| k.to_string()).unwrap_or_else(|| "-".into());
                        let ms = self.career.stat("r:TI", &sub).unwrap_or(0.0);
                        items.push((s("stat_Race_Times", "RACE TIME"), stat_time(ms as i64), Go::Off));
                        for (key, fb, code) in [("stat_Beat_TargetTime", "BEAT TARGET TIME", "rBTT"), ("stat_Complete_All", "COMPLETE ALL GOALS", "rCGL"), ("stat_Min_Damage", "MIN DAMAGE", "rMDG"), ("stat_Won_Race", "WON RACE", "rWIN")] {
                            let done = self.career.stat(code, &sub).unwrap_or(0.0) != 0.0;
                            items.push((s(key, fb), if done { s("stat_Complete", "COMPLETED") } else { s("Stat_Incomplete", "INCOMPLETE") }, Go::Off));
                        }
                    }
                    66 => {
                        title = s("Pause_Collectables", "COLLECTABLES");
                        for (k, (key, fb)) in [("Pause_Collectables_Hollywood", "HOLLYWOOD COLLECTABLES"), ("Pause_Collectables_Beach", "BEACH COLLECTABLES"), ("Pause_Collectables_Downtown", "DOWNTOWN COLLECTABLES"), ("Pause_Collectables_Hills", "HILLS COLLECTABLES")].iter().enumerate() {
                            items.push((s(key, fb), String::new(), Go::Sub(67, 0, k)));
                        }
                    }
                    67 => {
                        title = s("Pause_Collectables", "COLLECTABLES");
                        if left || right {
                            self.stat_idx[0] = (self.stat_idx[0] + if right { 1 } else { 3 }) % 4;
                        }
                        sub_title = [s("Hollywood", "HOLLYWOOD"), s("Beaches", "BEACH"), s("Downtown", "DOWNTOWN"), s("Hills", "HILLS")][self.stat_idx[0].min(3)].clone();
                        // (approx.) fifteen to a district, in the order the port numbers them
                        for n in 0..15usize {
                            let id = (self.stat_idx[0] * 15 + n) as u16;
                            // (an imported save holds the count alone: all sixty found is every one found)
                            let got = self.collected.contains(&id) || self.career.stat("gCOL", "0").map_or(false, |v| v >= 60.0);
                            items.push((s(&format!("stat_Collectable_{}", n + 1), &format!("COLLECTABLE  {}", n + 1)), if got { s("Collectables_Found", "FOUND") } else { String::new() }, Go::Off));
                        }
                    }
                    68 => {
                        // VIEW SONGS: the genre's songs (reference `x13`)
                        const NAMES: [(&str, &str); 8] = [("MUSIC_GENRE_ALL", "ALL GENRES"), ("MUSIC_GENRE_ECLECTIC", "ECLECTIC"), ("MUSIC_GENRE_ELECTRONIC", "ELECTRONIC"), ("MUSIC_GENRE_HARDROCK", "HARD ROCK"), ("MUSIC_GENRE_HIPHOP", "HIP HOP"), ("MUSIC_GENRE_ROCK", "ROCK"), ("MUSIC_GENRE_TECHNO", "TECHNO"), ("MUSIC_GENRE_WEST_RAP", "WEST COAST RAP")];
                        let g = self.stat_idx[0].min(7);
                        title = if self.stat_idx[0] == 8 { s("MUSIC_GENRE_ON_THE_FLY", "CUSTOM PLAYLIST") } else { s(NAMES[g].0, NAMES[g].1) };
                        sub_title = self.career.strings.get("playlist_desc").map(|v| v.to_string()).unwrap_or_else(|| "Music List:".into());
                        // (the original's lists, `mc4`: the songs in the list's own order, each one
                        // to PLAY; an empty custom playlist says so - sub_82640338)
                        let listed = crate::mc4::music_rows(&self.opt_x, self.stat_idx[0]);
                        if let Some(rows) = listed.as_ref() {
                            if rows.is_empty() {
                                sub_title = self.career.strings.get("playlist_desc_empty").map(|v| v.to_string()).unwrap_or_else(|| "Your custom playlist is empty.".into());
                            }
                            for t in rows {
                                items.push((t.to_uppercase(), String::new(), Go::Page(240)));
                            }
                        }
                        // (the game's songs are the string table's `Music_<Genre>_<Artist>_<Song>`)
                        const PREFIX: [&[&str]; 8] = [&[""], &["eclectic_"], &["electronic_"], &["hardrock_", "hard_rock_"], &["hiphop_"], &["rock_"], &["techno_"], &["westcoastrap_", "west_rap_"]];
                        let mut list: Vec<String> = self
                            .career
                            .strings
                            .keys
                            .iter()
                            .filter(|k| k.to_ascii_lowercase().strip_prefix("music_").map_or(false, |r| !r.starts_with("genre") && PREFIX[g].iter().any(|p| r.starts_with(p))))
                            .filter_map(|k| self.career.strings.get(k))
                            .filter(|t| t.contains(" - "))
                            .map(|t| t.to_uppercase())
                            .collect();
                        list.sort();
                        list.dedup();
                        if listed.is_some() {
                            list.clear();
                        }
                        for t in list {
                            items.push((t, String::new(), Go::Off));
                        }
                    }
                    1 | 9 => {
                        title = if self.pause_page == 9 { s("Pause_Game_Stats", "GAME STATS") } else { s("pause_statistics", "STATISTICS") };
                        let pct = self.career.percent_complete().max(0) as usize;
                        items.push((s("stat_k%AG_0", "PERCENT COMPLETED"), format!("{} %", pct), Go::Off));
                        items.push((s("Pause_Rep", "TOTAL REP").trim_end_matches(|c: char| c == ':' || c == ' ').to_string(), format!("{}", self.rep), Go::Off));
                        items.push(("CASH".to_string(), format!("${}", self.money), Go::Off));
                        items.push((s("stat_gCOL_0", "COLLECTABLES"), format!("{} / 60", found), Go::Off));
                        let mut keys: Vec<(&String, &i64)> = self.career.stats.iter().filter(|(k, _)| !k.contains(':')).collect();
                        keys.sort();
                        for (k, v) in keys {
                            if let Some(label) = self.career.strings.get(&format!("stat_{}_0", k)) {
                                if items.len() < 15 {
                                    items.push((label.to_uppercase(), if k.contains('$') { format!("${}", v) } else { v.to_string() }, Go::Off));
                                }
                            }
                        }
                    }
                    2 => {
                        title = s("pause_Cheat_Menu", "CHEAT MENU");
                        for (i, (key, fb, need)) in CHEATS.iter().enumerate() {
                            if found >= *need {
                                items.push((s(key, fb), if self.flash_menu.is_some() { if self.cheats[i] { crate::flash_ui::CHECK_ON } else { crate::flash_ui::CHECK_OFF }.to_string() } else if self.cheats[i] { "ON".to_string() } else { "OFF".to_string() }, Go::Page(10 + i as u8)));
                            } else {
                                // (with the movie a locked cheat is a greyed, empty box; what it needs is the port's own text)
                                items.push((s(key, fb), if self.flash_menu.is_some() { crate::flash_ui::CHECK_OFF.to_string() } else { s(&format!("{}_Locked", key), &format!("{} COLLECTABLES NEEDED", need)) }, Go::Off));
                            }
                        }
                    }
                    4 => {
                        // OPTIONS (left and right change the row)
                        title = s("pause_options", "OPTIONS");
                        let bar = |v: u8| format!("{}{}", "|".repeat(v as usize), ".".repeat(10usize.saturating_sub(v as usize)));
                        let onoff = |b: bool| if b { s("G_On", "ON") } else { s("G_Off", "OFF") };
                        if self.flash_menu.is_some() {
                            // the original's OPTIONS: four pages
                            items.push((s("controller", "CONTROLLER"), String::new(), Go::Page(5)));
                            items.push((s("game", "GAME"), String::new(), Go::Page(6)));
                            items.push((s("audio", "AUDIO"), String::new(), Go::Page(7)));
                            items.push((s("camera", "CAMERA"), String::new(), Go::Page(8)));
                            // (the port's own pages, DECISIONS.md D19: DISPLAY a placeholder, PERFORMANCE
                            // the traffic's model detail)
                            items.push(("DISPLAY".to_string(), String::new(), Go::Sub(PAGE_DISPLAY, 0, 0)));
                            items.push(("PERFORMANCE".to_string(), String::new(), Go::Sub(PAGE_PERFORMANCE, 0, 0)));
                        } else {
                            items.push((s("master volume", "MASTER VOLUME"), bar(self.opts.master), Go::Page(40)));
                            items.push((s("music volume", "MUSIC VOLUME"), bar(self.opts.music), Go::Page(41)));
                            items.push((s("option_subtitles", "SUBTITLES"), onoff(self.opts.subtitles), Go::Page(42)));
                            items.push((s("Camera Shake", "CAMERA SHAKE"), onoff(self.opts.shake), Go::Page(43)));
                            items.push((s("option_sensitivity", "STEERING SENS."), bar(self.opts.sens), Go::Page(44)));
                            items.push(("HIGH-DETAIL TRAFFIC".to_string(), onoff(self.opts.hq_traffic), Go::Page(52)));
                        }
                    }
                    PAGE_DISPLAY => {
                        // (the port's own page, D19: nothing to set yet)
                        title = "DISPLAY".to_string();
                        items.push(("NO DISPLAY SETTINGS YET".to_string(), String::new(), Go::Off));
                    }
                    PAGE_PERFORMANCE => {
                        // (the port's own page, D19)
                        title = "PERFORMANCE".to_string();
                        let on = |b: bool| format!("{}{}", crate::flash_ui::CHOICE, if b { s("G_On", "ON") } else { s("G_Off", "OFF") });
                        items.push(("HIGH-DETAIL TRAFFIC".to_string(), on(self.opts.hq_traffic), Go::Page(52)));
                    }
                    5 | 70..=73 => {
                        // (the controller pages, `mc4` step 13b: 5 the port's own GAMEPAD / KEYBOARD /
                        // RACING WHEEL, 70 CONTROL STYLE, 71 the mapping editor / viewer, 72 and 73 the
                        // keyboard's and the wheel's pages - not made yet. A row that can be chosen is
                        // marked with an entry the menu's own code does nothing with: mc4_ctrl_input acts)
                        let cp = self.mc4_ctrl_page();
                        title = cp.title.clone();
                        for (label, value, on) in &cp.rows {
                            items.push((label.clone(), value.clone(), if *on { Go::Sub(1, 0, 0) } else { Go::Off }));
                        }
                        ctrl_page = Some(cp);
                    }
                    5..=8 => {
                        // The pages of OPTIONS, in the original's row styles: a value between
                        // arrows, or a tick box. Only the settings the port has are listed (the
                        // original's AUDIO also has voice, player vehicle and cutscene volumes;
                        // its CONTROL STYLE lists five layouts, of which the port drives one).
                        use crate::flash_ui::{CHECK_OFF, CHECK_ON, CHOICE};
                        let num = |v: u8| format!("{}{}", CHOICE, v);
                        let tick = |b: bool| if b { CHECK_ON } else { CHECK_OFF }.to_string();
                        match self.pause_page {
                            5 => {
                                title = s("controller styles", "CONTROL STYLE");
                                // (the disc's list of styles, `mc4`: its first eight, each by its name as
                                // the text's key; the pad's five can be set - A, sub_8264D070 - the
                                // wheel's three are greyed without a wheel)
                                let style = self.mc4.as_ref().map_or(1, |m| m.options.control_config_pad);
                                const STYLES: [(&str, &str); 8] = [("Throttle_Stick", "STICK - AUTOMATIC"), ("Throttle_Trigger", "TRIGGER - AUTOMATIC"), ("Throttle_Stick_Manual", "STICK - MANUAL"), ("Throttle_Trigger_Manual", "TRIGGER - MANUAL"), ("Custom_Pad", "CUSTOM CONTROLLER"), ("Steering_Wheel", "RACING WHEEL - AUTOMATIC"), ("Steering_Wheel_Manual", "RACING WHEEL - MANUAL"), ("Custom_Steering_Wheel", "CUSTOM RACING WHEEL")];
                                for (i, (key, fb)) in STYLES.iter().enumerate() {
                                    if i < 5 {
                                        items.push((s(key, fb), tick(style == i as i32), Go::Page(220 + i as u8)));
                                    } else {
                                        items.push((s(key, fb), String::new(), Go::Off));
                                    }
                                }
                            }
                            6 => {
                                // (AUTO SAVE and ARROW have no counterpart in the port: shown, greyed)
                                title = s("Game_Options", "GAME OPTIONS");
                                let on = |b: bool| format!("{}{}", CHOICE, if b { s("G_On", "ON") } else { s("G_Off", "OFF") });
                                items.push((s("option_subtitles", "SUBTITLES"), on(self.opts.subtitles), Go::Page(42)));
                                items.push((s("option_autosave", "AUTO SAVE"), on(true), Go::Off));
                                // (option_arrow_compass / _top / _none: where the GPS arrow is - `mc4`, options)
                                let arrow = [("option_arrow_compass", "COMPASS"), ("option_arrow_top", "TOP"), ("option_arrow_none", "NONE")][self.mc4_gps_arrow() as usize];
                                items.push((s("option_arrow", "ARROW"), format!("{}{}", CHOICE, s(arrow.0, arrow.1)), Go::Page(51)));
                                // (the slider's place, -10 .. 10: reference `q20_game` shows 0 for the save's 0.0)
                                items.push((s("option_sensitivity", "STEERING SENS."), format!("{}{}", CHOICE, self.mc4_sens_step()), Go::Page(44)));
                                // (option_up / option_down: the roof of the convertibles - `mc4`, options.
                                // Greyed, without a value, while the vehicle is no convertible: `q20_game`, the R8)
                                let down = self.mc4.as_ref().map_or(false, |m| m.options.convertible_top_down);
                                if vehicles.player().map_or(false, |c| c.model.extra_variants.contains(&(crate::vehicle_model::ROOF_DOWN.to_string(), 0))) {
                                    items.push((s("option_convertible", "CONVERTIBLE TOP"), format!("{}{}", CHOICE, if down { s("option_down", "DOWN") } else { s("option_up", "UP") }), Go::Page(50)));
                                } else {
                                    items.push((s("option_convertible", "CONVERTIBLE TOP"), String::new(), Go::Off));
                                }
                            }
                            7 => {
                                title = s("audio options", "AUDIO OPTIONS");
                                items.push((s("effects volume", "EFFECTS VOLUME"), num(self.opts.master), Go::Page(40)));
                                items.push((s("music volume", "MUSIC VOLUME"), num(self.opts.music), Go::Page(41)));
                                items.push((s("voice volume", "VOICE VOLUME"), num(self.opts.voice), Go::Page(45)));
                                items.push((s("player vehicle volume", "PLAYER VEHICLE VOLUME"), num(self.opts.vehicle), Go::Page(46)));
                                items.push((s("cutscene_volume", "CUTSCENE VOLUME"), num(self.opts.cutscene), Go::Page(47)));
                            }
                            _ => {
                                // (DEFAULT CAMERA and CINEMATIC CAMERA are not options of the port: greyed)
                                title = s("camera options", "CAMERA OPTIONS");
                                let on = |b: bool| format!("{}{}", CHOICE, if b { s("G_On", "ON") } else { s("G_Off", "OFF") });
                                // (both are the save's: `mc4`, options.camera and the "cinematics" word)
                                let cam = self.mc4_camera_name();
                                items.push((s("Default Camera", "DEFAULT CAMERA"), format!("{}{}", CHOICE, s(cam, &cam.to_uppercase())), Go::Page(48)));
                                items.push((s("Idle Camera", "CINEMATIC CAMERA"), on(self.mc4_idle_camera()), Go::Page(49)));
                                items.push((s("Camera Shake", "CAMERA SHAKE"), on(self.opts.shake), Go::Page(43)));
                            }
                        }
                    }
                    3 => {
                        // MUSIC PLAYLISTS: all genres or one of the game's seven
                        title = s("playlist_title", "MUSIC PLAYLISTS");
                        const GENRES: [(&str, &str); 8] = [("MUSIC_GENRE_ALL", "ALL GENRES"), ("MUSIC_GENRE_ECLECTIC", "ECLECTIC"), ("MUSIC_GENRE_ELECTRONIC", "ELECTRONIC"), ("MUSIC_GENRE_HARDROCK", "HARD ROCK"), ("MUSIC_GENRE_HIPHOP", "HIP HOP"), ("MUSIC_GENRE_ROCK", "ROCK"), ("MUSIC_GENRE_TECHNO", "TECHNO"), ("MUSIC_GENRE_WEST_RAP", "WEST COAST RAP")];
                        if self.flash_menu.is_some() {
                            // the original lists the seven genres, then ALL GENRES and the custom
                            // playlist (not in the port: greyed); the genre playing is the one the
                            // cursor opens on
                            // (with the original's lists, `mc4`: A is VIEW SONGS on every row -
                            // sub_82642768, button 55)
                            let listed = self.opt_x.music.is_some();
                            for i in [1usize, 2, 3, 4, 5, 6, 7, 0] {
                                items.push((s(GENRES[i].0, GENRES[i].1), String::new(), Go::Page(if listed { 230 } else { 30 } + i as u8)));
                            }
                            items.push((s("MUSIC_GENRE_ON_THE_FLY", "CUSTOM PLAYLIST"), String::new(), if listed { Go::Page(238) } else { Go::Off }));
                        } else {
                            for (i, (key, fb)) in GENRES.iter().enumerate() {
                                items.push((s(key, fb), if self.music_genre as usize == i { "ON".to_string() } else { String::new() }, Go::Page(30 + i as u8)));
                            }
                        }
                    }
                    _ => match self.pause_tab {
                        0 if self.test_drive.is_some() && !racing => {
                            title = s("G_Test_Driving", "TEST DRIVE");
                            items.push((s("TD_Continue", "CONTINUE"), String::new(), Go::Act(MenuAction::Resume)));
                            items.push((s("TD_Return_To_Garage", "RETURN TO GARAGE"), String::new(), Go::Act(MenuAction::OpenGarage)));
                        }
                        0 => {
                            items.push((s("pause_continue", "CONTINUE"), String::new(), Go::Act(MenuAction::Resume)));
                            if racing {
                                let series_race = self.event_diff.is_some() && self.wager.is_none() && self.flash_menu.is_some();
                                let (restart, quit) = match kind {
                                    Some(RaceKind::Delivery) => (s("pause_restart_delivery", "RESTART DELIVERY"), s("pause_quit_delivery", "QUIT DELIVERY")),
                                    Some(RaceKind::Payback) => (s("pause_restart_payback", "RESTART PAYBACK"), s("pause_quit_payback", "QUIT PAYBACK")),
                                    // (a hangout's series - `sr_racepause`: CONTINUE, QUIT SERIES, RESTART SERIES,
                                    // SERIES STANDINGS (grey in its first race), ADD RACE TO FAVORITES, VIEW REPLAY ...)
                                    _ if series_race => (s("pause_restart_series", "RESTART SERIES"), s("pause_quit_series", "QUIT SERIES")),
                                    _ => (s("pause_restart_race", "RESTART RACE"), s("race_quit", "QUIT RACE")),
                                };
                                // (the original lists QUIT before RESTART - reference `t24`, an arcade race,
                                // which also has NEXT RACE, ADD RACE TO FAVORITES and VIEW REPLAY)
                                // (an ARCADE race - reference `y01`: CONTINUE, NEXT RACE, QUIT RACE, RESTART RACE,
                                // ADD RACE TO FAVORITES, VIEW REPLAY; the three the port lacks are greyed)
                                let arcade_race = self.arcade_restore.is_some() && self.flash_menu.is_some();
                                if arcade_race {
                                    items.push((s("pause_next_race", "NEXT RACE"), String::new(), Go::Page(250)));
                                }
                                items.push((quit, String::new(), Go::Act(MenuAction::QuitRace)));
                                items.push((restart, String::new(), Go::Act(MenuAction::RestartRace)));
                                if series_race {
                                    // (live once a race of the series is run - `sr_pause2`)
                                    items.push((s("pause_series_standings", "SERIES STANDINGS"), String::new(), if self.series.as_ref().map_or(false, |sr| !sr.wins.is_empty()) { Go::Page(254) } else { Go::Off }));
                                }
                                // (a career race on the road has the same rows less NEXT RACE, and SAVE GAME
                                // last - `ng_racepause`: CONTINUE, QUIT RACE, RESTART RACE, ADD RACE TO
                                // FAVORITES, VIEW REPLAY, SAVE GAME)
                                let road_race = self.flash_menu.is_some() && self.race.as_ref().map_or(false, |r| r.kind == RaceKind::Race);
                                if arcade_race || road_race {
                                    // (the row turns to REMOVE RACE FROM FAVORITES once the race is one)
                                    let fav = self.current_race.and_then(|i| self.race_defs.get(i)).map_or(false, |d| self.race_favs.iter().any(|f| f.eq_ignore_ascii_case(&d.id)));
                                    items.push((if fav { s("pause_remove_favorites", "REMOVE RACE FROM FAVORITES") } else { s("pause_add_favorites", "ADD RACE TO FAVORITES") }, String::new(), Go::Page(251)));
                                    // (white in the original's pause of a running race - `ng_racepause`,
                                    // `sr_racepause`: the race so far, then back to this menu)
                                    items.push((s("pause_view_replay", "VIEW REPLAY"), String::new(), if self.replay_rec.len() >= 1.0 { Go::Page(253) } else { Go::Off }));
                                }
                                if road_race && !arcade_race {
                                    items.push((s("pause_save_game", "SAVE GAME"), String::new(), Go::Page(252)));
                                }
                            } else {
                                // The original's free-roam tab is CONTINUE and GO TO GARAGE (reference
                                // capture); the GPS map is on the prompt bar's button, and TAKE PHOTO is a
                                // state the game keeps disabled here. Without the movie the port lists both.
                                if self.flash_menu.is_none() {
                                    items.push((s("pause_gps_map", "GPS MAP"), String::new(), Go::Act(MenuAction::OpenMap)));
                                }
                                // (a new career has no garage yet: CONTINUE / SAVE GAME - `ng_c5`; with the
                                // 100 % save CONTINUE / GO TO GARAGE / SAVE GAME)
                                if self.flash_menu.is_none() || !self.career.loaded() || self.career.reached("GARAGE_HOLLYWOOD") {
                                    items.push((s("pause_go_to_garage", "GO TO GARAGE"), String::new(), Go::Act(MenuAction::OpenGarage)));
                                }
                                if self.flash_menu.is_some() {
                                    items.push((s("pause_save_game", "SAVE GAME"), String::new(), Go::Page(252)));
                                }
                                if self.flash_menu.is_none() {
                                    items.push((s("pause_take_photo", "TAKE PHOTO"), String::new(), Go::Act(MenuAction::PhotoMode)));
                                }
                            }
                        }
                        1 => {
                            let go = |a: MenuAction| if racing { Go::Off } else { Go::Act(a) };
                            items.push((s("pause_arcade", "ARCADE"), String::new(), go(MenuAction::OpenArcade(false))));
                            // (the original's tab - reference `t03`; the online modes are not in
                            // the port: listed, greyed. GOAL ATTACK (`PM_GoalAttack`) as ARCADE)
                            if self.flash_menu.is_some() {
                                items.push((s("pause_goalattack", "GOAL ATTACK"), String::new(), go(MenuAction::OpenArcade(true))));
                                items.push((s("pause_play_online", "XBOX LIVE"), String::new(), Go::Off));
                                items.push((s("pause_online_tournament", "ONLINE TOURNAMENT"), String::new(), Go::Off));
                                items.push((s("pause_play_lan", "SYSTEM LINK"), String::new(), Go::Off));
                            }
                            items.push((s("pause_race_editor", "RACE EDITOR"), String::new(), go(MenuAction::OpenEditor)));
                        }
                        _ => {
                            items.push((s("pause_options", "OPTIONS"), String::new(), Go::Page(4)));
                            items.push((s("pause_statistics", "STATISTICS"), String::new(), Go::Page(1)));
                            items.push((s("pause_cheat_codes", "CHEAT CODES"), String::new(), Go::Page(2)));
                            items.push((s("pause_playlists", "MUSIC PLAYLISTS"), String::new(), Go::Page(3)));
                            items.push(("QUIT GAME".to_string(), String::new(), Go::Act(MenuAction::Quit)));
                        }
                    },
                }
                if ((4..=8).contains(&self.pause_page) || self.pause_page == PAGE_PERFORMANCE) && (left || right) {
                    let adj = |v: &mut u8| *v = if right { (*v + 1).min(10) } else { v.saturating_sub(1) };
                    // (the row under the cursor says which setting it is)
                    match items.get(self.menu_sel).map(|i| i.2) {
                        Some(Go::Page(40)) => adj(&mut self.opts.master),
                        Some(Go::Page(41)) => adj(&mut self.opts.music),
                        Some(Go::Page(42)) => self.opts.subtitles = !self.opts.subtitles,
                        Some(Go::Page(43)) => self.opts.shake = !self.opts.shake,
                        Some(Go::Page(44)) => crate::mc4::sens_set(&mut self.mc4, &mut self.opts.sens, right, false, self.fresh_game),
                        Some(Go::Page(p)) if p == 48 || p == 49 || p == 50 || p == 51 => crate::mc4::option_step(&mut self.mc4, &mut self.opt_x, p, right, self.fresh_game),
                        Some(Go::Page(45)) => adj(&mut self.opts.voice),
                        Some(Go::Page(46)) => adj(&mut self.opts.vehicle),
                        Some(Go::Page(47)) => adj(&mut self.opts.cutscene),
                        Some(Go::Page(52)) => self.opts.hq_traffic = !self.opts.hq_traffic,
                        _ => {}
                    }
                    self.dirty = true;
                }
                // (out on a test drive the pause menu is the plain TEST DRIVE menu, template 2 of
                // TestDrive.sc.xml: no tabs)
                let on_tabs = self.pause_page == 0 && !(self.test_drive.is_some() && !racing);
                // (the tabs are the shoulder buttons' - LB / RB on the panel; left / right as well)
                let (left, right) = (left || tab_prev, right || tab_next);
                if on_tabs && (left || right) {
                    self.pause_tab = if right { (self.pause_tab + 1) % 3 } else { (self.pause_tab + 2) % 3 };
                    self.menu_sel = 0;
                }
                if up && self.menu_sel > 0 {
                    self.menu_sel -= 1;
                }
                if down && self.menu_sel + 1 < items.len() {
                    self.menu_sel += 1;
                }
                let sel = self.menu_sel.min(items.len().saturating_sub(1));
                'pause_port: {
                let page = self.flash_menu.is_some().then(|| {
                    // the original menu movie (pause.swf) shows the page
                    use crate::flash_ui::{MenuPage, MenuRow, ICON_A, ICON_B, ICON_BACK_BUTTON};
                    // (OPTIONS reads EDIT; a page of values has nothing to accept)
                    let mut prompts = match self.pause_page {
                        4 => vec![(s("prompt_edit", "EDIT"), ICON_A), (s("prompt_back", "BACK"), ICON_B)],
                        5 | 70..=73 => ctrl_page.as_ref().map(|c| c.prompts.clone()).unwrap_or_default(),
                        7 => vec![(s("prompt_back", "BACK"), ICON_B), (s("prompt_defaults", "DEFAULTS"), crate::flash_ui::ICON_X)],
                        6 | 8 => vec![(s("prompt_back", "BACK"), ICON_B), (s("prompt_defaults", "DEFAULTS"), crate::flash_ui::ICON_X)],
                        PAGE_DISPLAY | PAGE_PERFORMANCE => vec![(s("prompt_back", "BACK"), ICON_B)],
                        // (the statistics' pages of figures: BACK alone, or with LT/RT to the next)
                        9 | 65 => vec![(s("prompt_back", "BACK"), ICON_B)],
                        61 => vec![(s("prompt_back", "BACK"), ICON_B), (s("prompt_Prev/Next_Car", "PREV/NEXT VEHICLE"), crate::flash_ui::ICON_LT_RT)],
                        64 | 67 => vec![(s("prompt_back", "BACK"), ICON_B), (s("prompt_Prev/Next", "PREV/NEXT BOARD"), crate::flash_ui::ICON_LT_RT)],
                        // (the original's lists, `mc4`: an empty list has BACK alone; Y adds the song
                        // to the custom playlist or takes it out - sub_82641000; X says what it
                        // would do to shuffle - sub_82642510)
                        68 if self.opt_x.music.is_some() && items.is_empty() => vec![(s("prompt_back", "BACK"), ICON_B)],
                        68 if crate::mc4::music_in_custom(&self.opt_x, self.stat_idx[0], self.menu_sel) => vec![(s("playlist_prompt_play", "PLAY"), ICON_A), (s("prompt_back", "BACK"), ICON_B), (s("playlist_prompt_remove", "REMOVE FROM PLAYLIST"), crate::flash_ui::ICON_Y)],
                        3 if crate::mc4::music_shuffle(&self.opt_x) == Some(false) => vec![(s("playlist_prompt_view", "VIEW SONGS"), ICON_A), (s("prompt_back", "BACK"), ICON_B), (s("playlist_prompt_shuffle", "TURN SHUFFLE ON"), crate::flash_ui::ICON_X)],
                        68 => vec![(s("playlist_prompt_play", "PLAY"), ICON_A), (s("prompt_back", "BACK"), ICON_B), (s("playlist_prompt_add", "ADD TO PLAYLIST"), crate::flash_ui::ICON_Y)],
                        3 => vec![(s("playlist_prompt_view", "VIEW SONGS"), ICON_A), (s("prompt_back", "BACK"), ICON_B), (s("playlist_prompt_unshuffle", "TURN SHUFFLE OFF"), crate::flash_ui::ICON_X)],
                        _ => vec![(s("prompt_accept", "ACCEPT"), ICON_A), (s("prompt_back", "BACK"), ICON_B)],
                    };
                    // (GPS stands on every tab, in a race too: references `p01`, `t03`, `y01`)
                    if on_tabs {
                        if self.gps_unlocked() {
                            prompts.push((s("pause_gps_map", "GPS MAP").replace(" MAP", ""), ICON_BACK_BUTTON));
                        }
                    }
                    // (MUSIC PLAYLISTS is a box with a text over its rows: the message box's
                    // template, 480 wide in the pause movie, all its rows shown)
                    let genres = self.pause_page == 3 || self.pause_page == 68;
                    MenuPage {
                        // (a page with a line under its title is the box template)
                        layout: if self.pause_page == 68 {
                            crate::flash_ui::MenuLayout::SongList { description: sub_title.clone() }
                        } else if genres {
                            crate::flash_ui::MenuLayout::SongList { description: self.career.strings.get("genre_desc").map(|v| v.to_string()).unwrap_or_else(|| "Select Genre:".into()) }
                        } else if let Some(h) = ctrl_page.as_ref().map(|c| &c.headers).filter(|h| !h.is_empty()) {
                            crate::flash_ui::MenuLayout::Columns { headers: h.clone() }
                        } else if !sub_title.is_empty() && std::env::var("MCLA_STAT_LIST").is_err() {
                            crate::flash_ui::MenuLayout::Confirm { description: sub_title.clone() }
                        } else {
                            crate::flash_ui::MenuLayout::List
                        },
                        description: sub_title.clone(),
                        // (a page under a tab carries that tab's icon: the tick for SETTINGS)
                        // (the original's playlist icon is a globe that none of the movies' icon frames
                        // carry; the empty ring, frame 18, stands in)
                        icon: if genres { std::env::var("MCLA_GENRE_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(18) } else { self.pause_tab as i32 + 1 },
                        tabs: if on_tabs { vec![s("pause_tab_mainmenu", "PAUSE"), s("pause_tab_gamemodes", "MODES"), s("pause_tab_settings", "SETTINGS")] } else { Vec::new() },
                        cur_tab: self.pause_tab,
                        title: title.clone(),
                        rows: items.iter().enumerate().map(|(k, (l, v, g))| MenuRow { label: l.clone(), value: v.replace('|', "I"), enabled: (*g != Go::Off || !(on_tabs || self.pause_page == 2)) && !(self.pause_page == 6 && v.is_empty()) && !(self.pause_page == PAGE_DISPLAY && *g == Go::Off) && !(ctrl_page.is_some() && *g == Go::Off && k < usize::MAX), part: None }).collect(),
                        select: sel,
                        prompts,
                        money: None,
                        pad_labels: 0,
                        tab_icons: Vec::new(),
                        row_props: ctrl_page.as_ref().map(|c| c.props.clone()).unwrap_or_default(),
                        list_max: if self.pause_page == 68 { 15 } else if genres { 0 } else { 6 },
                    }
                });
                // (a message box takes the panel while it is up)
                let page = self.box_page.take().or(page);
                if let (Some(fm), Some(page)) = (self.flash_menu.as_mut(), page) {
                    fm.show(&page);
                    // the pause menu proper (0x82675700): the movie then steers the camera
                    // itself, between the tuned pause cameras. Free roam uses PauseCamera2.
                    fm.movie.player.set_global("moviepausemode", mcla_flash::value::Value::Int(3));
                    fm.movie.player.set_global("cur_camera", mcla_flash::value::Value::Int(pause_camera_index(self.pause_page)));
                    // (the playlist box is drawn flat on the screen, over the driving camera)
                    let flat = self.pause_page == 3 || self.pause_page == 68;
                    fm.movie.player.set_global("view2d", mcla_flash::value::Value::Int(flat as i32));
                    let dt = ctx.input(|i| i.stable_dt).min(0.1);
                    fm.movie.player.update(dt);
                    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_menu"));
                    ctx.move_to_top(layer);
                    // the movie hangs in the scene on its panel mesh (see menu3d)
                    // The playlist box is the GPS movie's (`navsys`): that movie has the globe title
                    // icon (frame 1) and lays the wide `template_SongList` box out flat by itself.
                    let nav_drawn = flat && std::env::var("MCLA_SONG_PAUSE").is_err() && match (self.flash_navsys.as_mut(), self.ui_meshes.as_mut()) {
                        (Some(nm), Some(meshes)) => {
                            nm.show(&crate::flash_ui::MenuPage { icon: 1, ..page.clone() });
                            nm.movie.player.update(dt);
                            match crate::menu3d::read_scene(&mut nm.movie.player) {
                                Some(scene) => crate::menu3d::paint(ctx, &ctx.layer_painter(layer), meshes, &mut nm.movie, &scene, None),
                                None => nm.movie.paint(ctx, &ctx.layer_painter(layer)),
                            }
                            true
                        }
                        _ => false,
                    };
                    if nav_drawn {
                        self.pause_cam = None;
                    }
                    match (if nav_drawn { None } else { crate::menu3d::read_scene(&mut fm.movie.player) }, self.ui_meshes.as_mut()) {
                        (Some(mut scene), Some(meshes)) if std::env::var("MCLA_FLAT_MENU").is_err() => {
                            scene.wide_stage = flat;
                            if flat {
                                // (the clip that holds the movie's fonts is parked right of the stage,
                                // where the wide box now reaches: character 460, never meant to show)
                                let p = &mut fm.movie.player;
                                let parked: Vec<_> = p.inst(p.root).map(|r| r.children.values().copied().collect()).unwrap_or_default();
                                for id in parked {
                                    if let Some(i) = p.inst_mut(id) {
                                        if i.character == 460 {
                                            i.tint[3] = 0;
                                        }
                                    }
                                }
                            }
                            if flat {
                                // (approx.) the 2D viewport's camera is not the pause camera; it looks
                                // straight at the box. Fitted to the reference capture (the 1020-wide
                                // box spans 805 of 1280 pixels, centred, its top at 140 of 720).
                                let f: Vec<f32> = std::env::var("MCLA_FLAT_CAM").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                                let g = |i: usize, d: f32| f.get(i).copied().unwrap_or(d);
                                scene.cam.position = Vec3::new(g(0, -2.33), g(1, 1.42), g(2, 5.64));
                                scene.cam.lookat = Vec3::new(g(0, -2.33), g(1, 1.42), 0.0);
                                scene.cam.fovy = g(3, 70.0);
                            }
                            self.pause_cam = if flat { None } else { Some((scene.cam.position, scene.cam.lookat, scene.cam.fovy)) };
                            crate::menu3d::paint(ctx, &ctx.layer_painter(layer), meshes, &mut fm.movie, &scene, None);
                        }
                        _ => {
                            if !nav_drawn {
                                fm.movie.paint(ctx, &ctx.layer_painter(layer))
                            }
                        }
                    }
                    let mut chosen: Option<Go> = None;
                    if enter {
                        chosen = items.get(sel).map(|i| i.2);
                    }
                    // the GPS prompt of the free-roam tab (Back on the pad; M or Tab here), and
                    // the photo mode the list no longer carries (P)
                    // DEFAULTS (X on the pad, X here) on the audio page
                    if self.pause_page == 7 && ctx.input(|i| i.key_pressed(egui::Key::X)) {
                        let d = Options::default();
                        self.opts.master = d.master;
                        self.opts.music = d.music;
                        self.opts.voice = d.voice;
                        self.opts.vehicle = d.vehicle;
                        self.opts.cutscene = d.cutscene;
                        self.dirty = true;
                    }
                    if on_tabs && self.pause_tab == 0 && !racing {
                        if ctx.input(|i| i.key_pressed(egui::Key::M) || i.key_pressed(egui::Key::Tab)) {
                            chosen = Some(Go::Act(MenuAction::OpenMap));
                        } else if ctx.input(|i| i.key_pressed(egui::Key::P)) {
                            chosen = Some(Go::Act(MenuAction::PhotoMode));
                        }
                    }
                    match chosen {
                        // leaving a race asks first (`QuitRaceMenu` of MessageBoxes.sc.xml)
                        // (in a series both ask about the repair; either answer goes on - the option
                        // ids end in "yes" so that the box confirms on both)
                        Some(Go::Act(a @ (MenuAction::QuitRace | MenuAction::RestartRace))) if self.flash_popup.is_some() && self.series.is_some() && self.event_diff.is_some() => {
                            let quit = a == MenuAction::QuitRace;
                            let def = crate::flash_ui::MessageBoxDef {
                                title: if quit { "Dlg_Title_Series_Quit" } else { "Dlg_Title_Series_restart" }.into(),
                                description: if quit { "Dlg_Desc_Series_Quit" } else { "Dlg_Desc_Series_Restart" }.into(),
                                options: vec![("repair_yes".into(), "Common_Yes".into()), ("leave_yes".into(), "Common_No".into())],
                            };
                            self.message_box = Some((def, 0, a));
                        }
                        Some(Go::Act(MenuAction::QuitRace)) if self.flash_popup.is_some() => match crate::flash_ui::MessageBoxDef::load(&self.data_dir, "QuitRaceMenu") {
                            Some(def) if !def.options.is_empty() => self.message_box = Some((def, 0, MenuAction::QuitRace)),
                            _ => {
                                action = MenuAction::QuitRace;
                                self.pause_page = 0;
                            }
                        },
                        Some(Go::Act(a)) => {
                            action = a;
                            self.pause_page = 0;
                        }
                        // (NEXT RACE and the favourites row: done at the head of the next frame)
                        // (CONTROL STYLE, `mc4`: SET)
                        Some(Go::Page(p)) if (220..=228).contains(&p) => {
                            if crate::mc4::control_set(&mut self.mc4, (p - 220) as i32, self.fresh_game) {
                                self.dirty = true;
                            }
                        }
                        // (the music lists, `mc4`: PLAY on a song; VIEW SONGS on a list)
                        Some(Go::Page(240)) => {
                            if let Some(g) = crate::mc4::music_play(&self.opt_x, self.stat_idx[0], self.menu_sel) {
                                self.music_genre = g;
                                self.dirty = true;
                            }
                        }
                        Some(Go::Page(p)) if (230..=238).contains(&p) => {
                            self.stat_idx[0] = (p - 230) as usize;
                            self.pause_page = 68;
                            self.menu_sel = 0;
                        }
                        Some(Go::Page(p)) if p >= 250 => self.pause_do = Some(p),
                        Some(Go::Page(p)) if p >= 40 => {
                            let step = |v: &mut u8| *v = if *v >= 10 { 0 } else { *v + 1 };
                            match p {
                                40 => step(&mut self.opts.master),
                                41 => step(&mut self.opts.music),
                                42 => self.opts.subtitles = !self.opts.subtitles,
                                43 => self.opts.shake = !self.opts.shake,
                                45 => step(&mut self.opts.voice),
                                46 => step(&mut self.opts.vehicle),
                                47 => step(&mut self.opts.cutscene),
                                52 => self.opts.hq_traffic = !self.opts.hq_traffic,
                                48 | 49 | 50 | 51 => crate::mc4::option_step(&mut self.mc4, &mut self.opt_x, p, true, self.fresh_game),
                                _ => crate::mc4::sens_set(&mut self.mc4, &mut self.opts.sens, true, true, self.fresh_game),
                            }
                            self.dirty = true;
                        }
                        Some(Go::Page(p)) if p >= 30 => {
                            self.music_genre = p - 30;
                            crate::mc4::music_set_genre(&self.opt_x, (p - 30) as usize);
                            self.dirty = true;
                        }
                        Some(Go::Page(p)) if p >= 10 => {
                            let i = (p - 10) as usize;
                            self.cheats[i] = !self.cheats[i];
                        }
                        Some(Go::Page(p)) => {
                            self.pause_page = p;
                            self.menu_sel = 0;
                        }
                        Some(Go::Sub(p, level, idx)) if p != 1 => {
                            self.pause_page = p;
                            self.stat_idx[level.min(1)] = idx;
                            self.menu_sel = 0;
                        }
                        _ => {}
                    }
                    // (the music lists, `mc4`: X on the lists turns shuffle on or off, Y on a song
                    // adds it to the custom playlist or takes it out. Test hook: MCLA_MUSIC_TEST=x|y,
                    // once, half a second after the page is up)
                    if self.opt_x.music.is_some() && (self.pause_page == 3 || self.pause_page == 68) {
                        static HOOK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                        let hook = std::env::var("MCLA_MUSIC_TEST").ok().filter(|_| HOOK.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 30);
                        let x = std::mem::take(&mut self.nav_x) || ctx.input(|i| i.key_pressed(egui::Key::X)) || hook.as_deref() == Some("x");
                        let y = std::mem::take(&mut self.nav_y) || ctx.input(|i| i.key_pressed(egui::Key::Y)) || hook.as_deref() == Some("y");
                        if x && self.pause_page == 3 {
                            crate::mc4::music_toggle_shuffle(&self.opt_x);
                            self.dirty = true;
                        }
                        if y && self.pause_page == 68 {
                            if let Some(g) = crate::mc4::music_add_remove(&self.opt_x, self.stat_idx[0], self.menu_sel) {
                                self.music_genre = g;
                                self.dirty = true;
                            }
                        }
                    }
                    // VIEW SONGS (A on a genre keeps the port's meaning - play it; V here, the
                    // original's A)
                    if self.pause_page == 3 && ctx.input(|i| i.key_pressed(egui::Key::V)) {
                        // (the list's order: seven genres, then ALL GENRES)
                        self.stat_idx[0] = [1usize, 2, 3, 4, 5, 6, 7, 0].get(self.menu_sel).copied().unwrap_or(0);
                        self.pause_page = 68;
                        self.menu_sel = 0;
                    }
                    if back {
                        let up_one = match self.pause_page {
                            9 | 60 | 62 | 65 | 66 => Some((1u8, match self.pause_page { 9 => 0, 60 => 1, 62 => 2, 65 => 3, _ => 5 })),
                            61 => Some((60, self.stat_idx[0])),
                            63 => Some((62, self.stat_idx[0])),
                            64 => Some((63, self.stat_idx[1])),
                            67 => Some((66, self.stat_idx[0])),
                            68 => Some((3, 0)),
                            _ => None,
                        };
                        if let Some((page, at)) = up_one {
                            self.menu_sel = at;
                            self.pause_page = page;
                        } else if self.pause_page == 9 {
                            self.menu_sel = 0;
                            self.pause_page = 1;
                        } else if (5..=8).contains(&self.pause_page) {
                            // (a page of OPTIONS returns to OPTIONS)
                            self.menu_sel = self.pause_page as usize - 5;
                            self.pause_page = 4;
                        } else if self.pause_page == PAGE_DISPLAY || self.pause_page == PAGE_PERFORMANCE {
                            // (the port's own pages of OPTIONS, D19: rows 5 and 6)
                            self.menu_sel = if self.pause_page == PAGE_DISPLAY { 4 } else { 5 };
                            self.pause_page = 4;
                        } else if self.pause_page != 0 {
                            self.menu_sel = if self.pause_page == 2 { 1 } else { 0 };
                            self.pause_page = 0;
                        } else {
                            action = MenuAction::Resume;
                        }
                    }
                    if !matches!(action, MenuAction::None) {
                        fm.hide();
                    }
                    break 'pause_port;
                }
                let sc = (screen.height() / 900.0).clamp(0.6, 2.0);
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("menu")));
                let x0 = screen.left() + 70.0 * sc;
                let w = if on_tabs { 560.0 } else if self.pause_page == 2 { 1010.0 } else { 760.0 } * sc;
                let row_h = if on_tabs { 46.0 } else { 36.0 } * sc;
                let h = 190.0 * sc + row_h * items.len().max(4) as f32;
                let y0 = screen.center().y - h * 0.5 - 30.0 * sc;
                let panel = egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(w, h));
                self.panel(&painter, panel, sc);
                let tx = x0 + 34.0 * sc;
                let pointer = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_clicked()));
                let mut chosen: Option<Go> = None;
                if on_tabs {
                    // the tab strip
                    let tabs = [s("pause_tab_mainmenu", "PAUSE"), s("pause_tab_gamemodes", "MODES"), s("pause_tab_settings", "SETTINGS")];
                    let mut x = tx;
                    for (i, t) in tabs.iter().enumerate() {
                        let size = if i == self.pause_tab { 28.0 } else { 17.0 } * sc;
                        let col = if i == self.pause_tab { egui::Color32::WHITE } else { egui::Color32::from_gray(140) };
                        let r = self.text(&painter, egui::pos2(x, y0 + 66.0 * sc), egui::Align2::LEFT_BOTTOM, t, size, col);
                        if i == self.pause_tab {
                            painter.rect_filled(egui::Rect::from_min_size(egui::pos2(r.left(), y0 + 72.0 * sc), egui::vec2(r.width(), 3.0 * sc)), 0.0, accent);
                        }
                        if pointer.1 && pointer.0.map_or(false, |p| r.expand(6.0 * sc).contains(p)) && i != self.pause_tab {
                            self.pause_tab = i;
                            self.menu_sel = 0;
                        }
                        x = r.right() + 26.0 * sc;
                    }
                } else {
                    self.text(&painter, egui::pos2(tx, y0 + 40.0 * sc), egui::Align2::LEFT_TOP, &title, 28.0 * sc, egui::Color32::WHITE);
                }
                let list_y = y0 + 110.0 * sc;
                for (i, (label, value, go)) in items.iter().enumerate() {
                    let r = egui::Rect::from_min_size(egui::pos2(x0 + 6.0 * sc, list_y + row_h * i as f32), egui::vec2(w - 6.0 * sc, row_h));
                    let hovered = pointer.0.map_or(false, |p| r.contains(p));
                    if hovered && pointer.1 {
                        self.menu_sel = i;
                        chosen = Some(*go);
                    }
                    let selected = i == sel && (on_tabs || self.pause_page == 2);
                    if selected {
                        self.cursor(&painter, r, sc);
                    }
                    let off = *go == Go::Off && (on_tabs || self.pause_page == 2);
                    let col = if selected { egui::Color32::from_rgb(20, 14, 6) } else if off { egui::Color32::from_gray(105) } else if hovered { egui::Color32::WHITE } else { egui::Color32::from_gray(200) };
                    let size = if on_tabs { 20.0 } else { 15.0 } * sc;
                    self.text(&painter, egui::pos2(tx, r.center().y), egui::Align2::LEFT_CENTER, label, size, col);
                    if !value.is_empty() {
                        let vcol = if selected { col } else if off { egui::Color32::from_gray(105) } else { accent };
                        self.text(&painter, egui::pos2(r.right() - 34.0 * sc, r.center().y), egui::Align2::RIGHT_CENTER, value, size, vcol);
                    }
                }
                let fy = panel.bottom() - 46.0 * sc;
                let hint = if on_tabs { format!("LEFT / RIGHT  {}", s("prompt_changeTabs", "CHANGE TABS")) } else { "ESC  BACK".to_string() };
                self.text(&painter, egui::pos2(tx, fy), egui::Align2::LEFT_TOP, &hint, 11.0 * sc, egui::Color32::from_gray(150));
                if on_tabs && self.pause_tab == 0 {
                    self.mission_log(&painter, screen, sc, accent);
                }
                if enter {
                    chosen = items.get(sel).map(|i| i.2);
                }
                match chosen {
                    // (CONTROL STYLE, `mc4`: SET)
                    Some(Go::Page(p)) if (220..=228).contains(&p) => {
                        if crate::mc4::control_set(&mut self.mc4, (p - 220) as i32, self.fresh_game) {
                            self.dirty = true;
                        }
                    }
                    // (the music lists, `mc4`: PLAY on a song; VIEW SONGS on a list)
                    Some(Go::Page(240)) => {
                        if let Some(g) = crate::mc4::music_play(&self.opt_x, self.stat_idx[0], self.menu_sel) {
                            self.music_genre = g;
                            self.dirty = true;
                        }
                    }
                    Some(Go::Page(p)) if (230..=238).contains(&p) => {
                        self.stat_idx[0] = (p - 230) as usize;
                        self.pause_page = 68;
                        self.menu_sel = 0;
                    }
                    Some(Go::Act(a)) => {
                        action = a;
                        self.pause_page = 0;
                    }
                    Some(Go::Page(p)) if p >= 40 => {
                        // (Enter steps a row on, round and round)
                        let step = |v: &mut u8| *v = if *v >= 10 { 0 } else { *v + 1 };
                        match p {
                            40 => step(&mut self.opts.master),
                            41 => step(&mut self.opts.music),
                            42 => self.opts.subtitles = !self.opts.subtitles,
                            43 => self.opts.shake = !self.opts.shake,
                            52 => self.opts.hq_traffic = !self.opts.hq_traffic,
                            48 | 49 | 50 | 51 => crate::mc4::option_step(&mut self.mc4, &mut self.opt_x, p, true, self.fresh_game),
                            _ => crate::mc4::sens_set(&mut self.mc4, &mut self.opts.sens, true, true, self.fresh_game),
                        }
                        self.dirty = true;
                    }
                    Some(Go::Page(p)) if p >= 30 => {
                        self.music_genre = p - 30;
                        crate::mc4::music_set_genre(&self.opt_x, (p - 30) as usize);
                        self.dirty = true;
                    }
                    Some(Go::Page(p)) if p >= 10 => {
                        let i = (p - 10) as usize;
                        self.cheats[i] = !self.cheats[i];
                    }
                    Some(Go::Page(p)) => {
                        self.pause_page = p;
                        self.menu_sel = 0;
                    }
                    _ => {}
                }
                if back {
                    if self.pause_page != 0 {
                        self.menu_sel = if self.pause_page == 2 { 1 } else { 0 };
                        self.pause_page = 0;
                    } else {
                        action = MenuAction::Resume;
                    }
                }
                }
            }
            Mode::RaceSelect => {
                // (while the WAGER AMOUNT form is open the list underneath does not listen)
                let (f_up, f_down, f_enter, f_back) = (up, down, enter, back);
                // (self-test: the form over the list)
                if self.wager_pick.is_none() && std::env::var("MCLA_WAGER_TEST").is_ok() {
                    self.wager_pick = Some(2);
                }
                let picking = self.wager_pick.is_some();
                let (up, down, left, right, enter, back) = if picking { (false, false, false, false, false, false) } else { (up, down, left, right, enter, back) };
                let n = self.race_defs.len();
                // the original's ARCADE menu (race type, race, options) on the GPS map's movie;
                // the port's own list stays for the hangouts' wager races and as the fallback
                // (a hangout: its own panels, not the ARCADE menu or the port's list)
                let hangout = n > 0 && self.flash_menu.is_some() && self.hangout_key.is_some() && std::env::var("MCLA_OLD_RACESEL").is_err();
                let arcade = n > 0 && self.flash_navsys.is_some() && !self.wager_here && !picking && std::env::var("MCLA_OLD_RACESEL").is_err();
                if hangout {
                    // (step 23b: a hangout's tutorial stands over its scene, the panel waits under
                    // it; PINKSLIP RACE's choice is the camera's on the cars, under the prompt bar)
                    let key = self.hangout_key.clone().unwrap_or_default();
                    if self.mc4_hangout_tut_ui(ctx, f_enter) {
                    } else if self.hangout_page.0 == 2 && key.contains("_wager_") {
                        if let Some(a) = self.mc4_pink_choice_ui(ctx, &key, left, right, f_enter, f_back) {
                            action = a;
                        }
                    } else if let Some(a) = self.hangout_ui(ctx, f_up, f_down, left, right, f_enter, f_back) {
                        action = a;
                    }
                } else if arcade {
                    if let Some(a) = self.arcade_ui(ctx, up, down, left, right, enter, back) {
                        action = a;
                    }
                } else if n == 0 {
                    action = MenuAction::StartRace;
                } else {
                    if up {
                        self.race_sel = (self.race_sel + n - 1) % n;
                    }
                    if down {
                        self.race_sel = (self.race_sel + 1) % n;
                    }
                    if left || right {
                        // jump to the previous / next district
                        let cur = self.race_defs[self.race_sel].district.clone();
                        let mut i = self.race_sel;
                        for _ in 0..n {
                            i = if right { (i + 1) % n } else { (i + n - 1) % n };
                            if self.race_defs[i].district != cur {
                                break;
                            }
                        }
                        if left {
                            let d = self.race_defs[i].district.clone();
                            while i > 0 && self.race_defs[i - 1].district == d {
                                i -= 1;
                            }
                        }
                        self.race_sel = i;
                    }
                    let sel = self.race_sel;
                    let rows = 14usize;
                    let first = sel.saturating_sub(rows / 2).min(n.saturating_sub(rows));
                    let sc = (screen.height() / 900.0).clamp(0.6, 2.0);
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("racesel")));
                    let x0 = screen.left() + 70.0 * sc;
                    let w = 820.0 * sc;
                    let row_h = 34.0 * sc;
                    let h = 250.0 * sc + row_h * rows as f32;
                    let y0 = screen.center().y - h * 0.5;
                    let panel = egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(w, h));
                    self.panel(&painter, panel, sc);
                    let tx = x0 + 34.0 * sc;
                    let def = self.race_defs[sel].clone();
                    self.text(&painter, egui::pos2(tx, y0 + 30.0 * sc), egui::Align2::LEFT_TOP, "SELECT RACE", 28.0 * sc, egui::Color32::WHITE);
                    self.text(&painter, egui::pos2(tx, y0 + 74.0 * sc), egui::Align2::LEFT_TOP, &format!("{}   {}/{}", def.district.to_uppercase(), sel + 1, n), 15.0 * sc, accent);
                    let pointer = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_clicked()));
                    let list_y = y0 + 118.0 * sc;
                    for (k, i) in (first..(first + rows).min(n)).enumerate() {
                        let d = &self.race_defs[i];
                        let r = egui::Rect::from_min_size(egui::pos2(x0 + 6.0 * sc, list_y + row_h * k as f32), egui::vec2(w - 6.0 * sc, row_h));
                        let hovered = pointer.0.map_or(false, |p| r.contains(p));
                        if hovered && pointer.1 {
                            if self.race_sel == i {
                                self.current_race = Some(i);
                                action = MenuAction::StartRace;
                            }
                            self.race_sel = i;
                        }
                        let selected = i == sel;
                        if selected {
                            self.cursor(&painter, r, sc);
                        }
                        let col = if selected { egui::Color32::from_rgb(20, 14, 6) } else if hovered { egui::Color32::WHITE } else { egui::Color32::from_gray(200) };
                        let kind = match d.sub_type.as_str() {
                            "Blitz" => "TIME TRIAL",
                            "Circuit" => "CIRCUIT",
                            "Ordered" => "ORDERED",
                            "Unordered" => "UNORDERED",
                            _ => "RACE",
                        };
                        let cy = r.center().y;
                        self.text(&painter, egui::pos2(tx, cy), egui::Align2::LEFT_CENTER, &d.district.to_uppercase(), 12.0 * sc, col);
                        self.text(&painter, egui::pos2(tx + 250.0 * sc, cy), egui::Align2::LEFT_CENTER, kind, 12.0 * sc, col);
                        self.text(&painter, egui::pos2(tx + 450.0 * sc, cy), egui::Align2::LEFT_CENTER, &d.name.to_uppercase().replace('_', " "), 12.0 * sc, col);
                    }
                    let fy = list_y + row_h * rows as f32 + 22.0 * sc;
                    let laps = if def.laps > 1 { format!("{} LAPS", def.laps) } else { "POINT TO POINT".to_string() };
                    let info = format!("{} CHECKPOINTS   {}   {:.1} KM   {} OPPONENTS", def.checkpoints.len(), laps, def.lap_len() * def.laps.max(1) as f32 / 1000.0, def.opponents.clamp(1, 7));
                    self.text(&painter, egui::pos2(tx, fy), egui::Align2::LEFT_TOP, &info, 11.0 * sc, egui::Color32::from_gray(190));
                    self.text(&painter, egui::pos2(tx, fy + 28.0 * sc), egui::Align2::LEFT_TOP, &format!("TARGET TIME {}", fmt_time(def.goal_time)), 11.0 * sc, egui::Color32::from_gray(140));
                    if enter {
                        self.current_race = Some(self.race_sel);
                        let plain = !matches!(def.race_type.as_str(), "Delivery" | "Payback" | "CaptureTheFlag");
                        if self.wager_here && plain {
                            self.wager_pick = Some(0);
                        } else {
                            self.wager = None;
                            action = MenuAction::StartRace;
                        }
                    }
                    if back {
                        self.wager_here = false;
                        action = MenuAction::Back;
                    }
                    // ---- the wager form (WAGER AMOUNT): no wager, or one of three stakes; the
                    // bigger the stake, the harder the opponent; the money goes now and comes
                    // back double with a win ----
                    if let (true, Some(sel)) = (picking, self.wager_pick) {
                        let st = |key: &str, fallback: &str| -> String { self.career.strings.get(key).filter(|v| !v.contains('<')).map(|v| v.to_string()).unwrap_or_else(|| fallback.to_string()) };
                        let wp = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("wager")));
                        let panel = egui::Rect::from_center_size(screen.center(), egui::vec2(640.0, 330.0) * sc);
                        wp.rect_filled(screen, 0.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 150));
                        wp.rect_filled(panel, 10.0 * sc, egui::Color32::from_rgb(34, 38, 44));
                        self.panel(&wp, panel, sc);
                        let tx = panel.left() + 36.0 * sc;
                        self.text(&wp, egui::pos2(tx, panel.top() + 26.0 * sc), egui::Align2::LEFT_TOP, &st("WAGER_AMOUNT_TITLE", "WAGER AMOUNT").to_uppercase(), 24.0 * sc, egui::Color32::WHITE);
                        let desc = st("tutorial_wager_desc", "The more the wager, the harder the race.");
                        self.text(&wp, egui::pos2(tx, panel.top() + 64.0 * sc), egui::Align2::LEFT_TOP, &desc.rsplit(".  ").next().unwrap_or(&desc).to_uppercase(), 11.0 * sc, egui::Color32::from_gray(190));
                        self.text(&wp, egui::pos2(panel.right() - 36.0 * sc, panel.top() + 30.0 * sc), egui::Align2::RIGHT_TOP, &format!("CASH ${}", self.money), 12.0 * sc, egui::Color32::from_gray(200));
                        let names = ["EASY", "MEDIUM", "HARD"];
                        let row_h = 46.0 * sc;
                        let sel = if f_up { sel.saturating_sub(1) } else if f_down { (sel + 1).min(3) } else { sel };
                        self.wager_pick = Some(sel);
                        for k in 0..4usize {
                            let r = egui::Rect::from_min_size(egui::pos2(panel.left() + 6.0 * sc, panel.top() + 104.0 * sc + row_h * k as f32), egui::vec2(panel.width() - 12.0 * sc, row_h));
                            if k == sel {
                                self.cursor(&wp, r, sc);
                            }
                            let col = if k == sel { egui::Color32::from_rgb(20, 14, 6) } else { egui::Color32::from_gray(210) };
                            if k == 0 {
                                self.text(&wp, egui::pos2(tx, r.center().y), egui::Align2::LEFT_CENTER, "NO WAGER", 17.0 * sc, col);
                            } else {
                                let stake = self.wager_amount(k - 1);
                                let dc = crate::rewards::DIFF_COLORS[k - 1];
                                wp.circle_filled(egui::pos2(tx + 8.0 * sc, r.center().y), 7.0 * sc, egui::Color32::from_rgb(dc[0], dc[1], dc[2]));
                                self.text(&wp, egui::pos2(tx + 28.0 * sc, r.center().y), egui::Align2::LEFT_CENTER, &format!("${}", stake), 17.0 * sc, col);
                                self.text(&wp, egui::pos2(tx + 190.0 * sc, r.center().y), egui::Align2::LEFT_CENTER, names[k - 1], 13.0 * sc, col);
                                self.text(&wp, egui::pos2(panel.right() - 36.0 * sc, r.center().y), egui::Align2::RIGHT_CENTER, &format!("WIN ${}", stake * 2), 13.0 * sc, col);
                            }
                        }
                        if f_enter {
                            if sel == 0 {
                                self.wager = None;
                                self.wager_pick = None;
                                self.wager_here = false;
                                action = MenuAction::StartRace;
                            } else {
                                let stake = self.wager_amount(sel - 1);
                                if self.money < stake {
                                    let m = st("Dialog_Description_WagerInsufficientFunds", "You don't have enough money to wager.").to_uppercase();
                                    self.show_message(&m);
                                } else {
                                    self.money -= stake;
                                    self.dirty = true;
                                    self.wager = Some((stake, sel - 1));
                                    self.wager_pick = None;
                                    self.wager_here = false;
                                    action = MenuAction::StartRace;
                                }
                            }
                        } else if f_back {
                            self.wager_pick = None;
                        }
                    }
                }
            }
            Mode::Garage if self.vinyl_edit.as_ref().map_or(false, |e| e.page < 9) && self.flash_garage.is_some() && std::env::var("MCLA_OLD_VINYL").is_err() => {
                self.vinyl_front_ui(ctx, [left, right, up, down, enter, back, tab_prev, tab_next]);
            }
            Mode::Garage if self.vinyl_edit.is_some() => {
                // ---- the vinyl editor: layers of shapes in five zones, painted onto the car as they change ----
                let car = self.vinyl_for.clone();
                let mut ed = self.vinyl_edit.clone().unwrap_or_default();
                let mut livery = self.liveries.get(&car).cloned().unwrap_or_default();
                let before = livery.clone();
                const ROWS: [&str; 20] = [
                    "ZONE", "LAYER", "SHAPE GROUP", "SHAPE", "POSITION ALONG", "POSITION ACROSS", "WIDTH", "HEIGHT", "ROTATION", "SKEW", "HUE", "SATURATION", "BRIGHTNESS", "OPACITY", "TYPE", "ADD A LAYER", "REMOVE THIS LAYER",
                    "MOVE LAYER UP / DOWN", "COPY ZONE TO THE OTHER SIDE", "CLEAR THIS ZONE",
                ];
                let n = ROWS.len();
                if up {
                    ed.row = (ed.row + n - 1) % n;
                }
                if down {
                    ed.row = (ed.row + 1) % n;
                }
                let d: isize = if left { -1 } else if right { 1 } else { 0 };
                let fine = ctx.input(|i| i.modifiers.shift);
                let cats: Vec<(String, usize)> = self.vinyl_cats.clone();
                let count = livery.zones[ed.zone].len();
                ed.layer = ed.layer.min(count.saturating_sub(1));
                let base = crate::vinyl::base_rot(ed.zone);
                {
                    let zone = &mut livery.zones[ed.zone];
                    let step = if fine { 0.001 } else { 0.01 };
                    if d != 0 {
                        match ed.row {
                            0 => {
                                ed.zone = (ed.zone as isize + d).rem_euclid(5) as usize;
                                ed.layer = 0;
                            }
                            1 if count > 0 => ed.layer = (ed.layer as isize + d).rem_euclid(count as isize) as usize,
                            17 if count > 1 => {
                                let to = (ed.layer as isize + d).clamp(0, count as isize - 1) as usize;
                                zone.swap(ed.layer, to);
                                ed.layer = to;
                            }
                            _ => {}
                        }
                        if let Some(l) = zone.get_mut(ed.layer).filter(|_| (2..=14).contains(&ed.row)) {
                            let (mut h, mut s, mut v) = rgb_to_hsv(l.color);
                            match ed.row {
                                2 if !cats.is_empty() => {
                                    let c = (l.shape as isize / 1000 + d).rem_euclid(cats.len() as isize) as usize;
                                    l.shape = (c * 1000) as u16;
                                }
                                3 => {
                                    let c = l.shape as usize / 1000;
                                    let m = cats.get(c).map_or(1, |x| x.1.max(1)) as isize;
                                    l.shape = (c * 1000) as u16 + (l.shape as isize % 1000 + d).rem_euclid(m) as u16;
                                }
                                4 => l.pos[0] = (l.pos[0] + d as f32 * step).clamp(-0.6, 0.6),
                                5 => l.pos[1] = (l.pos[1] + d as f32 * step).clamp(-0.4, 0.4),
                                6 => l.scale[0] = (l.scale[0] + d as f32 * step).clamp(-1.2, 1.2),
                                7 => l.scale[1] = (l.scale[1] + d as f32 * step).clamp(-1.2, 1.2),
                                8 => l.rot = (l.rot + d as f32 * if fine { 1.0 } else { 5.0 }).rem_euclid(360.0),
                                9 => l.skew = (l.skew + d as f32 * 0.05).clamp(-2.0, 2.0),
                                10 => h = (h + d as f32 * if fine { 0.005 } else { 0.02 }).rem_euclid(1.0),
                                11 => s = (s + d as f32 * 0.05).clamp(0.0, 1.0),
                                12 => v = (v + d as f32 * 0.05).clamp(0.0, 1.0),
                                13 => l.color[3] = (l.color[3] as i32 + d as i32 * 13).clamp(13, 255) as u8,
                                14 => {
                                    use crate::vinyl::Blend;
                                    const KINDS: [Blend; 3] = [Blend::Normal, Blend::AlphaCutout, Blend::NegativeCutout];
                                    let k = KINDS.iter().position(|b| *b == l.blend).unwrap_or(0) as isize;
                                    l.blend = KINDS[(k + d).rem_euclid(3) as usize];
                                }
                                _ => {}
                            }
                            if (10..=12).contains(&ed.row) {
                                let a = l.color[3];
                                l.color = hsv_to_rgb(h, s, v);
                                l.color[3] = a;
                            }
                        }
                    }
                }
                if enter {
                    let cap = crate::vinyl::ZONE_CAPACITY[ed.zone];
                    match ed.row {
                        15 => {
                            if livery.zones[ed.zone].len() < cap {
                                // a new layer goes on top, lying along the zone
                                let shape = livery.zones[ed.zone].get(ed.layer).map_or(8000, |l| l.shape);
                                livery.zones[ed.zone].push(crate::vinyl::Layer { shape, rot: base.rem_euclid(360.0), ..Default::default() });
                                ed.layer = livery.zones[ed.zone].len() - 1;
                                ed.row = 3;
                            } else {
                                self.show_message("THIS ZONE IS FULL");
                            }
                        }
                        16 if count > 0 => {
                            livery.zones[ed.zone].remove(ed.layer);
                            ed.layer = ed.layer.saturating_sub(1);
                        }
                        18 if ed.zone == 1 || ed.zone == 2 => {
                            // the other side is this one seen in a mirror
                            let other = 3 - ed.zone;
                            livery.zones[other] = livery.zones[ed.zone].iter().map(|l| crate::vinyl::Layer { scale: [-l.scale[0], l.scale[1]], pos: [-l.pos[0], l.pos[1]], rot: -l.rot, ..l.clone() }).collect();
                            self.show_message("COPIED TO THE OTHER SIDE");
                        }
                        18 if ed.zone == 3 || ed.zone == 4 => {
                            let other = 7 - ed.zone;
                            livery.zones[other] = livery.zones[ed.zone].clone();
                            self.show_message("COPIED TO THE OTHER END");
                        }
                        19 => {
                            livery.zones[ed.zone].clear();
                            ed.layer = 0;
                        }
                        _ => {}
                    }
                }
                if livery != before {
                    self.liveries.insert(car.clone(), livery.clone());
                    self.livery_rev += 1;
                    self.vinyls.insert(car.clone(), "custom".to_string());
                    if !self.vinyl_list.iter().any(|v| v == "custom") {
                        self.vinyl_list.insert(0, "custom".to_string());
                    }
                }
                // ---- drawing ----
                let sc = (screen.height() / 900.0).clamp(0.6, 2.0);
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("vinyledit")));
                let x0 = screen.left() + 50.0 * sc;
                let w = 560.0 * sc;
                let row_h = 30.0 * sc;
                let h = 150.0 * sc + row_h * n as f32;
                let y0 = (screen.center().y - h * 0.5).max(screen.top() + 12.0 * sc);
                let panel = egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(w, h));
                self.panel(&painter, panel, sc);
                let tx = x0 + 30.0 * sc;
                self.text(&painter, egui::pos2(tx, y0 + 24.0 * sc), egui::Align2::LEFT_TOP, "VINYL EDITOR", 26.0 * sc, egui::Color32::WHITE);
                self.text(&painter, egui::pos2(panel.right() - 30.0 * sc, y0 + 32.0 * sc), egui::Align2::RIGHT_TOP, &format!("{} LAYERS", livery.layer_count()), 13.0 * sc, egui::Color32::from_gray(190));
                let layer = livery.zones[ed.zone].get(ed.layer).cloned();
                let count = livery.zones[ed.zone].len();
                let pointer = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_clicked()));
                let list_y = y0 + 82.0 * sc;
                for (k, label) in ROWS.iter().enumerate() {
                    let r = egui::Rect::from_min_size(egui::pos2(x0 + 6.0 * sc, list_y + row_h * k as f32), egui::vec2(w - 12.0 * sc, row_h));
                    if pointer.0.map_or(false, |p| r.contains(p)) && pointer.1 {
                        ed.row = k;
                    }
                    let selected = k == ed.row;
                    if selected {
                        self.cursor(&painter, r, sc);
                    }
                    let needs_layer = (1..=14).contains(&k) || k == 16 || k == 17;
                    let dim = needs_layer && layer.is_none();
                    let col = if selected { egui::Color32::from_rgb(20, 14, 6) } else if dim { egui::Color32::from_gray(110) } else { egui::Color32::from_gray(205) };
                    self.text(&painter, egui::pos2(tx, r.center().y), egui::Align2::LEFT_CENTER, label, 14.0 * sc, col);
                    let value = match (k, layer.as_ref()) {
                        (0, _) => format!("{}   {} / {}", crate::vinyl::ZONE_NAMES[ed.zone], count, crate::vinyl::ZONE_CAPACITY[ed.zone]),
                        (1, Some(_)) => format!("{} / {}", ed.layer + 1, count),
                        (2, Some(l)) => cats.get(l.shape as usize / 1000).map_or("-".to_string(), |c| c.0.to_uppercase()),
                        (3, Some(l)) => format!("{} / {}", l.shape % 1000 + 1, cats.get(l.shape as usize / 1000).map_or(0, |c| c.1)),
                        (4, Some(l)) => format!("{:+.3}", l.pos[0]),
                        (5, Some(l)) => format!("{:+.3}", l.pos[1]),
                        (6, Some(l)) => format!("{:.3}", l.scale[0]),
                        (7, Some(l)) => format!("{:.3}", l.scale[1]),
                        (8, Some(l)) => format!("{:.0}", (l.rot - base).rem_euclid(360.0)),
                        (9, Some(l)) => format!("{:+.2}", l.skew),
                        (10, Some(l)) => format!("{:.0}", rgb_to_hsv(l.color).0 * 360.0),
                        (11, Some(l)) => format!("{:.0} %", rgb_to_hsv(l.color).1 * 100.0),
                        (12, Some(l)) => format!("{:.0} %", rgb_to_hsv(l.color).2 * 100.0),
                        (13, Some(l)) => format!("{:.0} %", l.color[3] as f32 / 2.55),
                        (14, Some(l)) => match l.blend {
                            crate::vinyl::Blend::Normal => "NORMAL".to_string(),
                            crate::vinyl::Blend::AlphaCutout => "ALPHA CUTOUT".to_string(),
                            crate::vinyl::Blend::NegativeCutout => "NEGATIVE ALPHA CUTOUT".to_string(),
                        },
                        (15..=19, _) if k != 17 => "ENTER".to_string(),
                        _ => String::new(),
                    };
                    self.text(&painter, egui::pos2(r.right() - 24.0 * sc, r.center().y), egui::Align2::RIGHT_CENTER, &value, 13.0 * sc, col);
                    // the layer's colour
                    if let (10..=13, Some(l)) = (k, layer.as_ref()) {
                        let sw = egui::Rect::from_center_size(egui::pos2(x0 + 300.0 * sc, r.center().y), egui::vec2(46.0 * sc, 16.0 * sc));
                        painter.rect_filled(sw, 2.0, egui::Color32::from_rgba_unmultiplied(l.color[0], l.color[1], l.color[2], 255));
                        painter.rect_stroke(sw, 2.0, egui::Stroke::new(1.0, egui::Color32::BLACK), egui::StrokeKind::Inside);
                    }
                }
                self.text(&painter, egui::pos2(tx, panel.bottom() - 40.0 * sc), egui::Align2::LEFT_TOP, "LEFT / RIGHT  CHANGE    SHIFT  FINE    ESC  DONE", 11.0 * sc, egui::Color32::from_gray(150));
                self.vinyl_edit = Some(ed);
                if back {
                    self.vinyl_edit = None;
                    // (painted once more at full size)
                    self.livery_rev += 1;
                    self.dirty = true;
                }
            }
            Mode::Garage if self.perf_shop.is_some() => {
                // ---- the performance shop: thirteen parts in two stages (`tune/garage` price lists) ----
                let i = self.current_car;
                let list = self.perf_list(i);
                let stages = self.perf_stages(i);
                let n = list.len().max(1);
                let mut sel = self.perf_shop.unwrap_or(0).min(n - 1);
                if up {
                    sel = (sel + n - 1) % n;
                }
                if down {
                    sel = (sel + 1) % n;
                }
                self.perf_shop = Some(sel);
                let movie_drawn = self.perf_shop_movie(ctx, &list, &stages, sel);
                if movie_drawn {
                    // The dyno graph: the movie only says where it goes (`dynopos`); the game
                    // draws the engine's curves there - torque red, horsepower blue (reference
                    // `n13`). (approx.) The place is measured on the capture, the curves are
                    // the port's own torque table, each to its own peak.
                    if let Some(c) = vehicles.player() {
                        let k = screen.height() / 720.0;
                        let o = egui::pos2(screen.center().x - 640.0 * k, screen.top());
                        // (the car's garage file says where: DynoScreenPosHD / DynoScreenSizeHD,
                        // fractions of the screen; measured on the capture without it)
                        let r = match self.dyno_rect {
                            Some(d) => egui::Rect::from_min_size(o + egui::vec2(d[0] * 1280.0, d[1] * 720.0) * k, egui::vec2(d[2] * 1280.0, d[3] * 720.0) * k),
                            None => egui::Rect::from_min_max(o + egui::vec2(215.0, 318.0) * k, o + egui::vec2(468.0, 432.0) * k),
                        };
                        let gp = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_garage")));
                        // (the engine's own torque table where the car has one)
                        let own: Option<Vec<f32>> = c.model.simtunes.as_ref().map(|t| t.base.engine.torque.clone()).filter(|t| t.len() >= 3);
                        let t: Vec<f32> = own.unwrap_or_else(|| c.model.params.torque.to_vec());
                        let tmax = t.iter().cloned().fold(1e-3f32, f32::max);
                        let hp: Vec<f32> = t.iter().enumerate().map(|(i, v)| v * i as f32).collect();
                        let hmax = hp.iter().cloned().fold(1e-3f32, f32::max);
                        let own_table = c.model.simtunes.is_some();
                        let line = |v: &[f32], max: f32, peak: f32, col: egui::Color32| {
                            let n = (v.len().max(2) - 1) as f32;
                            // (the engine's table runs to its maximum speed, fall-off included)
                            let span = if own_table { 1.0 } else { 0.84 };
                            let at = |x: f32| -> f32 {
                                // Catmull-Rom through the samples
                                let i = x.floor() as isize;
                                let f = x - i as f32;
                                let g = |k: isize| v[k.clamp(0, v.len() as isize - 1) as usize];
                                let (p0, p1, p2, p3) = (g(i - 1), g(i), g(i + 1), g(i + 2));
                                0.5 * ((2.0 * p1) + (-p0 + p2) * f + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * f * f + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * f * f * f)
                            };
                            let steps = 96;
                            let mut pts: Vec<egui::Pos2> = (0..=steps).map(|k| { let x = n * k as f32 / steps as f32; egui::pos2(r.left() + r.width() * span * x / n, r.bottom() - r.height() * peak * (at(x) / max).max(0.03)) }).collect();
                            if !own_table {
                                pts.push(egui::pos2(r.left() + r.width() * 0.9, r.bottom() - r.height() * 0.06));
                                pts.push(egui::pos2(r.right(), r.bottom() - r.height() * 0.04));
                            }
                            gp.add(egui::Shape::line(pts, egui::Stroke::new(1.2 * k, col)));
                        };
                        line(&t, tmax, 0.66, crate::flash_ui::plain_color(200, 60, 50, 255));
                        line(&hp, hmax, 0.80, crate::flash_ui::plain_color(80, 70, 220, 255));
                    }
                }
                let mut buy = false;
                if !movie_drawn {
                let sc = (screen.height() / 900.0).clamp(0.6, 2.0);
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("perfshop")));
                let x0 = screen.left() + 60.0 * sc;
                let w = 930.0 * sc;
                let row_h = 38.0 * sc;
                let h = 190.0 * sc + row_h * n as f32;
                let y0 = (screen.center().y - h * 0.5).max(screen.top() + 20.0 * sc);
                let panel = egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(w, h));
                self.panel(&painter, panel, sc);
                let tx = x0 + 34.0 * sc;
                let title = self.career.strings.get("G_Performance").map(|s| s.to_uppercase()).unwrap_or_else(|| "PERFORMANCE SHOP".into());
                self.text(&painter, egui::pos2(tx, y0 + 30.0 * sc), egui::Align2::LEFT_TOP, &title, 28.0 * sc, egui::Color32::WHITE);
                self.text(&painter, egui::pos2(panel.right() - 34.0 * sc, y0 + 34.0 * sc), egui::Align2::RIGHT_TOP, &format!("CASH ${}", self.money), 16.0 * sc, egui::Color32::from_rgb(120, 230, 120));
                self.text(&painter, egui::pos2(panel.right() - 34.0 * sc, y0 + 58.0 * sc), egui::Align2::RIGHT_TOP, &format!("REP {}", self.rep), 12.0 * sc, egui::Color32::from_gray(190));
                let pointer = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_clicked()));
                let list_y = y0 + 104.0 * sc;
                for (k, (id, p1, p2)) in list.iter().enumerate() {
                    let r = egui::Rect::from_min_size(egui::pos2(x0 + 6.0 * sc, list_y + row_h * k as f32), egui::vec2(w - 12.0 * sc, row_h));
                    if pointer.0.map_or(false, |p| r.contains(p)) && pointer.1 {
                        sel = k;
                        self.perf_shop = Some(k);
                        buy = true;
                    }
                    let selected = k == sel;
                    if selected {
                        self.cursor(&painter, r, sc);
                    }
                    let col = if selected { egui::Color32::from_rgb(20, 14, 6) } else { egui::Color32::from_gray(200) };
                    let label = self.career.strings.get(id).map(|s| s.to_uppercase()).unwrap_or_else(|| id.trim_start_matches("gp_").to_uppercase());
                    self.text(&painter, egui::pos2(tx, r.center().y), egui::Align2::LEFT_CENTER, &label, 16.0 * sc, col);
                    let have = stages.get(k).copied().unwrap_or(0);
                    let fitted = ["STOCK", "STAGE 1", "STAGE 2"][have.min(2) as usize];
                    self.text(&painter, egui::pos2(x0 + 450.0 * sc, r.center().y), egui::Align2::LEFT_CENTER, fitted, 14.0 * sc, col);
                    // what the next stage costs, or what it waits for
                    let (next, vcol) = if have >= 2 {
                        ("FITTED".to_string(), if selected { col } else { egui::Color32::from_gray(140) })
                    } else {
                        let need = self.perf_rep(i, have + 1);
                        let price = if have == 0 { *p1 } else { *p2 };
                        if self.rep < need {
                            (format!("LOCKED  {} REP", need), if selected { col } else { egui::Color32::from_rgb(255, 90, 70) })
                        } else {
                            (format!("STAGE {}   ${}", have + 1, price), if selected { col } else if self.money >= price { accent } else { egui::Color32::from_gray(140) })
                        }
                    };
                    self.text(&painter, egui::pos2(r.right() - 28.0 * sc, r.center().y), egui::Align2::RIGHT_CENTER, &next, 14.0 * sc, vcol);
                }
                self.text(&painter, egui::pos2(tx, panel.bottom() - 46.0 * sc), egui::Align2::LEFT_TOP, "ENTER  BUY THE NEXT STAGE      ESC  BACK", 11.0 * sc, egui::Color32::from_gray(150));
                }
                if enter || buy {
                    self.buy_perf_part(sel);
                }
                // AUTO UPGRADE (X, reference `n13`): every part to the last stage the career has
                // opened and the money reaches, one after the other down the list
                if self.flash_garage.is_some() && (ctx.input(|i| i.key_pressed(egui::Key::X)) || std::env::var("MCLA_AUTO_UPGRADE").is_ok()) {
                    self.auto_upgrade();
                }
                // CHANGE BRAND (LB / RB): the highlighted part's next maker, round its list
                if self.flash_garage.is_some() && (tab_prev || tab_next) && sel < 13 {
                    let n = PERF_BRANDS[sel].len();
                    if n > 1 {
                        let e = self.perf_brand.entry(self.dir_name(self.current_car).to_ascii_lowercase()).or_insert([0; 13]);
                        e[sel] = ((e[sel] as usize + n + tab_next as usize - tab_prev as usize) % n) as u8;
                        self.dirty = true;
                    }
                }
                if back {
                    self.perf_shop = None;
                }
            }
            Mode::Garage => {
                let n = self.car_dirs.len().max(1);
                let perf_row = 1 + CUSTOM_SLOTS.len();
                let vinyl_row = perf_row + 1;
                // hue, saturation, brightness of the paint
                let hsv_row = vinyl_row + 1;
                // rim size (`rim_15` .. `rim_22` of the garage's wheel options)
                // the paint type (the paint shop's Paint Type menu)
                let ptype_row = hsv_row + 3;
                // the fade type and the second colour (the paint shop's Fade Type, Secondary Color)
                let fade_row = ptype_row + 1;
                // window tint: transparency and colour (the Colors menu)
                let tint_row = fade_row + 2;
                // exterior neon and its colour (the Colors menu)
                let neon_row = tint_row + 2;
                let rim_row = neon_row + 2;
                // hydraulics on or off (the garage's Hydraulics menu)
                let hyd_row = rim_row + 1;
                // The garage's menus as the game nests them (tune/ui/garage/Main.sc.xml:
                // Garage_customization_menu = Performance, Exterior, Vinyl Editor, Paint Shop,
                // Hydraulics; the exterior by Front / Side / Rear / Wheels / colours / Interior).
                // A row is one of the settings below, by its number; a sub-menu opens another list.
                #[derive(Clone, Copy)]
                enum It {
                    Sub(&'static str, usize),
                    Row(usize),
                    /// a setting under the game's own name for it
                    RowAs(&'static str, usize),
                    /// a list of parts to fit
                    Part(&'static str, Slot),
                    Exit,
                }
                let slot_row = |s: Slot| CUSTOM_SLOTS.iter().position(|x| *x == s).map_or(0, |i| i + 1);
                let garage_car = self.dir_name(self.current_car).to_ascii_lowercase();
                let model_variants = vehicles.player().map(|c| &c.model.variants);
                let extra_variants = vehicles.player().map(|c| c.model.extra_variants.clone()).unwrap_or_default();
                let available_xparts: std::collections::HashSet<usize> = XPARTS.iter().enumerate()
                    .filter(|(k, (_, sec, _, _))| {
                        let rows = self.xpart_rows(*k);
                        if matches!(*sec, "tires" | "brakes" | "scissor_door" | "intercooler") { return !rows.is_empty(); }
                        rows.iter().any(|r| r.3 > 0 && extra_variants.contains(&(sec.to_string(), r.3)))
                    })
                    .map(|(k, _)| k).collect();
                let available_slots: std::collections::HashSet<Slot> = CUSTOM_SLOTS.iter().copied().filter(|slot| {
                    if matches!(slot, Slot::Paint | Slot::Wheels) { return true; }
                    let count = model_variants.and_then(|v| v.iter().find(|(s, _)| s == slot)).map_or(0, |(_, v)| v.len());
                    let rows = self.part_entries(self.current_car, *slot);
                    count > 0 && rows.map_or(!garage_car.ends_with("_dub") && count > 1,
                        |r| r.iter().any(|entry| entry.2 < count))
                }).collect();
                // The exterior's areas are a ring of tabs turned with left and right (as the
                // game shows it: SIDE < FRONT > VEHICLE ...), each a short list of part kinds.
                // (the areas as the game orders them, references `g_tab*`: FRONT, VEHICLE, WHEEL,
                // COLOR, INTERIOR, REAR, EXTRA, SIDE; COLOR is menu 16, added at the list's end)
                const RING: [usize; 8] = [4, 5, 6, 16, 7, 8, 9, 10];
                /// The width the rim shop's box is held at, in stage pixels. The movie
                /// measures its own content, and the brand logos and price columns it
                /// measures range about 10% either side of this; pinning the box here
                /// keeps the panel from resizing as the tabs are stepped.
                const RIM_PANEL_W: f32 = 456.0;
                let menus: Vec<(&'static str, Vec<It>)> = vec![
                    // (the original's four rows; the port's one vehicle list stands behind both MANAGE
                    // VEHICLES and VEHICLE SHOWROOM)
                    ("GARAGE", vec![It::RowAs("MANAGE VEHICLES", 0), It::Sub("CUSTOMIZE VEHICLE", 1), It::RowAs("VEHICLE SHOWROOM", 0), It::Exit]),
                    ("CUSTOMIZATION", vec![It::RowAs("PERFORMANCE SHOP", perf_row), It::Sub("EXTERIOR/INTERIOR", RING[0]), if self.flash_garage.is_some() { It::Sub("VINYL EDITOR", 15) } else { It::RowAs("VINYL EDITOR", vinyl_row) }, It::Sub("PAINT SHOP", 3), It::Sub("HYDRAULICS/AIR BAGS", 13), It::Sub("SPECIAL ABILITIES", 14)]),
                    ("-", vec![It::Exit]),
                    // (PAINT OPTIONS: ALL PARTS has the fade and its colour, PARTS the PAINT AREA -
                    // references `g_m_paint`, `g_pp_1`)
                    ("PAINT SHOP", if self.flash_garage.is_none() {
                        vec![It::RowAs("PAINT TYPE", ptype_row), It::RowAs("FADE TYPE", fade_row), It::RowAs("PRIMARY COLOR", slot_row(Slot::Paint)), It::RowAs("SECONDARY COLOR", fade_row + 1)]
                    } else if self.paint_parts {
                        vec![It::RowAs("PAINT OPTIONS", 9975), It::RowAs("PAINT AREA", 9976), It::RowAs("PAINT TYPE", ptype_row), It::RowAs("PRIMARY COLOR", slot_row(Slot::Paint))]
                    } else {
                        vec![It::RowAs("PAINT OPTIONS", 9975), It::RowAs("PAINT TYPE", ptype_row), It::RowAs("FADE TYPE", fade_row), It::RowAs("PRIMARY COLOR", slot_row(Slot::Paint)), It::RowAs("SECONDARY COLOR", fade_row + 1)]
                    }),
                    // (the original's FRONT also has HEADLIGHTS - reference `c02`; the port has no such part: greyed)
                    // (AIR SCOOPS where the car's part file has blowers: the '66 GTO, reference `g_gto2`)
                    // (INTERCOOLER, COWL likewise by the part file's sections - the vehicle's `<car>_Exterior.gtun`:
                    // ext_front = bumper_front, headlight, hood, then intercooler / blower; a bike's = bike_cowl alone)
                    ("FRONT", if self.part_entries_sec(self.current_car, "bike_cowl").is_some() {
                        vec![It::RowAs("COWL", 9954)]
                    } else {
                        let mut v = vec![It::Part("FRONT BUMPERS", Slot::BumperF), It::RowAs("HEADLIGHTS", 9960), It::Part("HOODS", Slot::Hood)];
                        if self.part_entries_sec(self.current_car, "intercooler").is_some() { v.push(It::RowAs("INTERCOOLER", 9953)); }
                        if self.part_entries_sec(self.current_car, "blower").is_some() { v.push(It::RowAs("AIR SCOOPS", 9958)); }
                        v
                    }),
                    ("VEHICLE", vec![It::Part("WIDEBODY", Slot::Body)]),
                    // (rows the port has no parts for are listed greyed, as the original lists them: `a00`..`a07`)
                    ("WHEEL", vec![It::Part("RIMS", Slot::Wheels), It::RowAs("SOUTH CENTRAL RIMS", 9961), It::Sub("MATERIALS/PAINT", 18), It::Sub("DIMENSIONS", 12), It::RowAs("TIRES", 9963), It::RowAs("BRAKES", 9964)]),
                    ("INTERIOR", vec![It::RowAs("STEERING WHEELS", 9965), It::RowAs("GAUGES", 9966), It::Part("FRONT SEATS", Slot::Seats)]),
                    // (a bike's ext_rear = bike_tail, license_plate)
                    ("REAR", if self.part_entries_sec(self.current_car, "bike_tail").is_some() { vec![It::RowAs("TAIL", 9955), It::RowAs("LICENSE PLATES", 9968)] } else { vec![It::Part("REAR BUMPERS", Slot::BumperR), It::Part("SPOILERS", Slot::Spoiler), It::RowAs("TAILLIGHTS", 9967), It::Part("EXHAUSTS", Slot::Exhaust), It::RowAs("LICENSE PLATES", 9968)] }),
                    ("EXTRA", vec![It::Sub("NEON/WINDOW TINT", 11)]),
                    // (ext_side = skirts, scissor_door, then side_exhaust where the file has it; a bike's = bike_tank)
                    ("SIDE", if self.part_entries_sec(self.current_car, "bike_tank").is_some() {
                        vec![It::RowAs("TANK", 9956)]
                    } else if self.part_entries_sec(self.current_car, "side_exhaust").is_some() {
                        vec![It::Part("SIDESKIRTS", Slot::Skirts), It::RowAs("DOORS", 9969), It::RowAs("SIDE EXHAUSTS", 9957)]
                    } else {
                        vec![It::Part("SIDESKIRTS", Slot::Skirts), It::RowAs("DOORS", 9969)]
                    }),
                    ("NEON/WINDOW TINT", vec![It::RowAs("NEON", neon_row), It::RowAs("NEON COLOR", neon_row + 1), It::RowAs("WINDOW TINT", tint_row), It::RowAs("TINT COLOR", tint_row + 1)]),
                    // (DimensionMenu of Wheels.sc.xml: axle, rim size, tyre profile, tyre width, ride height)
                    ("DIMENSIONS", if self.flash_garage.is_some() { vec![It::RowAs("AXLE", 9970), It::RowAs("RIMS SIZE", rim_row), It::RowAs("TIRE PROFILE", 9971), It::RowAs("TIRE WIDTH", 9972), It::RowAs("RIDE HEIGHT", 9973)] } else { vec![It::RowAs("RIMS SIZE", rim_row)] }),
                    // (Hydraulics.sc.xml: a flat page like the paint shop's. The port has the
                    // hydraulics switch; air bags and their patterns are listed, not kept)
                    ("HYDRAULICS/AIR BAGS", vec![It::RowAs("HYDRAULICS", hyd_row), It::RowAs("AIR BAGS", 9990), It::RowAs("LOWERING PATTERN", 9991)]),
                    // (GarageSpecAbilMenu of Main.sc.xml: ACCEPT takes the ability and goes back)
                    ("SPECIAL ABILITIES", vec![It::RowAs("ZONE", 9993), It::RowAs("AGRO", 9994), It::RowAs("ROAR", 9995), It::RowAs("EMP", 9996)]),
                    // (the vinyl editor's own menu: reference `t45`)
                    ("VINYL EDITOR", vec![It::RowAs("VINYL PACKAGES", 9980), It::RowAs("CREATE/EDIT VINYLS", 9984), It::RowAs("REMOVE ALL VINYLS", 9981), It::RowAs("STORE VINYL PACKAGES", 9982), It::RowAs("EXIT VINYL EDITOR", 9983)]),
                    // (the interior's colours, ColorsInt.sc.xml: reference `g_col0`)
                    ("COLOR", vec![It::Sub("COLOR", 17)]),
                    ("INTERIOR COLORS", vec![It::RowAs("INTERIOR NEON", 9940), It::RowAs("NEON COLOR", 9941), It::RowAs("GAUGE COLOR A", 9942), It::RowAs("GAUGE COLOR B", 9943), It::RowAs("INTERIOR COLOR A", 9944), It::RowAs("INTERIOR COLOR B", 9945), It::RowAs("INTERIOR COLOR C", 9946), It::RowAs("INTERIOR COLOR D", 9947), It::RowAs("INTERIOR COLOR E", 9948)]),
                    // (RimColorMatMenu of Wheels.sc.xml: reference `g_wh_mat`)
                    ("MATERIALS/PAINT", vec![It::RowAs("SPOKE A", 9930), It::RowAs("SPOKE B", 9931), It::RowAs("LIP", 9932), It::RowAs("SPOKE A", 9933), It::RowAs("SPOKE B", 9934), It::RowAs("LIP", 9935)]),
                ];
                if self.selected_car != self.current_car || self.garage_menu >= menus.len() {
                    // (looking through the vehicles happens at the top)
                    self.garage_menu = 0;
                    self.garage_cur = 0;
                    self.garage_stack.clear();
                    self.garage_part = None;
                    self.garage_xpart = None;
                    self.plate_edit = None;
                }
                // (MCLA_GARAGE_MENU=n: open at that menu, MCLA_GARAGE_PART=n: with that part list, for screenshots)
                // (after the top menu has stood for a while, as when it is reached by hand: a
                // jump on the very first frame leaves the movie's move-in transition unfinished)
                static HOOK_WAIT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let hook_due = std::env::var("MCLA_GARAGE_MENU").is_ok() && HOOK_WAIT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 400;
                if let (true, Some(m)) = (hook_due && self.garage_stack.is_empty() && self.garage_menu == 0, std::env::var("MCLA_GARAGE_MENU").ok().and_then(|v| v.parse::<usize>().ok()).filter(|m| *m > 0 && *m < menus.len())) {
                    self.garage_stack.push((0, 1));
                    self.garage_menu = m;
                    self.garage_cur = std::env::var("MCLA_GARAGE_CUR").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(0).min(menus[m].1.len().saturating_sub(1));
                    self.garage_part = std::env::var("MCLA_GARAGE_PART").ok().and_then(|v| v.parse::<usize>().ok()).filter(|r| (1..=CUSTOM_SLOTS.len()).contains(r));
                    // (MCLA_PLATE_EDIT=<cursor>: with the plate editor open)
                    if let Some(c) = std::env::var("MCLA_PLATE_EDIT").ok().and_then(|v| v.parse::<usize>().ok()) {
                        let text = vehicles.player().map(|c| c.plate_text.to_ascii_uppercase()).unwrap_or_default();
                        let mut idx = [14usize; 7];
                        for (i, ch) in text.chars().take(7).enumerate() {
                            idx[i] = lp_place(ch);
                        }
                        self.plate_edit = Some((c.min(6), idx));
                    }
                    // (MCLA_GARAGE_XPART="k,row": with that list of XPARTS open)
                    let x: Vec<usize> = std::env::var("MCLA_GARAGE_XPART").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                    if let Some(k) = x.first().copied().filter(|k| *k < XPARTS.len()) {
                        self.garage_xpart = Some((k, x.get(1).copied().unwrap_or(0)));
                    }
                }
                // MANAGE VEHICLES / VEHICLE SHOWROOM: a list of vehicles (the player's own, or
                // those for sale). The highlighted one stands in the garage; ACCEPT takes or
                // buys it through the rules already there, BACK returns to the car driven.
                if std::env::var("MCLA_GARAGE_LIST").is_ok() && self.garage_list.is_none() && self.flash_garage.is_some() {
                    static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    if !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        self.garage_list = Some((std::env::var("MCLA_GARAGE_LIST").map_or(false, |v| v == "showroom"), 0));
                    }
                }
                // (an imported save's rim for this car, should it not have been fitted at loading)
                if self.selected_car == self.current_car && self.apply_imported_rim() {
                    self.dirty = true;
                }
                // (back from a test drive the showroom is open again)
                if std::mem::take(&mut self.showroom_again) && self.flash_garage.is_some() {
                    self.garage_list = Some((true, 0));
                }
                let list_cars: Vec<usize> = match self.garage_list {
                    // (the showroom offers what the catalogue prices and the game offers at all -
                    // mcGarage::ListShowroomVehicles, sub_826A9D58: `mc4_vehicle_available`)
                    Some((showroom, _)) => (0..n).filter(|i| self.is_owned(*i) != showroom && (!showroom || (self.price_of(*i).0 > 0 && self.mc4_vehicle_available(&self.dir_name(*i))))).collect(),
                    None => Vec::new(),
                };
                // (MANAGE VEHICLES lists the garage in its own order - the save's slots, reference
                // `g_mv0`: R8, Ninja, GTO, Impala ... - not the catalogue's)
                let list_cars: Vec<usize> = if matches!(self.garage_list, Some((false, _))) {
                    let mut l = list_cars;
                    let at = |i: &usize| { let n = self.dir_name(*i); self.owned.iter().position(|o| o.eq_ignore_ascii_case(&n)).unwrap_or(usize::MAX) };
                    l.sort_by_key(at);
                    l
                } else {
                    list_cars
                };
                let listing = self.garage_list.is_some() && !list_cars.is_empty();
                let mut list_back = false;
                let mut list_swallow = false;
                if let (true, Some((showroom, sel))) = (listing, self.garage_list) {
                    let m = list_cars.len();
                    // (the showroom's strip runs sideways)
                    let (prev, next) = if showroom { (up || left, down || right) } else { (up, down) };
                    let sel = if prev { (sel + m - 1) % m } else if next { (sel + 1) % m } else { sel.min(m - 1) };
                    self.garage_list = Some((showroom, sel));
                    if self.selected_car != list_cars[sel] {
                        self.selected_car = list_cars[sel];
                        self.perf_sel = self.perf_of(self.selected_car);
                    }
                    if back {
                        self.garage_list = None;
                        self.selected_car = self.current_car;
                        list_back = true;
                    }
                    // TEST DRIVE (X in the showroom; test hook MCLA_TEST_DRIVE): out in the vehicle
                    // under the cursor
                    static TD_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    let td_hook = showroom && std::env::var("MCLA_TEST_DRIVE").is_ok() && !TD_ONCE.swap(true, std::sync::atomic::Ordering::Relaxed);
                    if showroom && self.message_box.is_none() && !self.is_owned(list_cars[sel]) && (td_hook || ctx.input(|i| i.key_pressed(egui::Key::X))) {
                        self.test_drive = Some(list_cars[sel]);
                        self.test_drive_req = true;
                        self.garage_list = None;
                        self.selected_car = self.current_car;
                    }
                    // PURCHASE (A) / PURCHASE WITH TRADE-IN (Y): `garage.CheckOKVehiclePurchase[WithTrade]`
                    // asks first, or says the money is short; a locked vehicle takes neither
                    let with_trade = ctx.input(|i| i.key_pressed(egui::Key::Y));
                    // (test hook: MCLA_BUY_TEST presses PURCHASE once)
                    static BUY_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    let enter = enter || (showroom && std::env::var("MCLA_BUY_TEST").is_ok() && !BUY_ONCE.swap(true, std::sync::atomic::Ordering::Relaxed));
                    if showroom && (enter || with_trade) && self.message_box.is_none() && !self.is_owned(list_cars[sel]) {
                        let (price, rep) = self.price_of(list_cars[sel]);
                        let trade = if with_trade { self.trade_in(self.current_car) } else { 0 };
                        if self.rep < rep && std::env::var("MCLA_UNLOCK_ALL").is_err() {
                            list_swallow = self.flash_popup.is_some();
                        } else if self.money + trade < price {
                            list_swallow = self.open_message_box("ShowroomPriceError", MenuAction::None);
                        } else if self.open_message_box(if with_trade { "G_PurchaseVehicleWithTrade" } else { "G_PurchaseVehicle" }, MenuAction::None) {
                            self.buy_pending = Some((list_cars[sel], with_trade));
                            list_swallow = true;
                        }
                    }
                    // SELL RIDE (X): `garage.CheckOKVehicleSale` - the last vehicle cannot be sold
                    if !showroom && (ctx.input(|i| i.key_pressed(egui::Key::X)) || std::env::var("MCLA_SELL_TEST").is_ok()) && self.message_box.is_none() {
                        if self.owned.len() <= 1 {
                            self.open_message_box("ManageRideSaleError", MenuAction::None);
                        } else if self.open_message_box("G_SellVehicle", MenuAction::None) {
                            self.sell_pending = Some(list_cars[sel]);
                        }
                    }
                } else if self.garage_list.is_some() {
                    self.garage_list = None;
                }
                let (up, down, left, right, back) = if listing { (false, false, false, false, false) } else { (up, down, left, right, back) };
                let enter = enter && !list_swallow;
                // The paint shop's colour rows (Paint.sc.xml): ACCEPT opens the strip of colour
                // slots, left/right move along it, ACCEPT keeps and BACK restores; X on the strip
                // opens the colour picker (LB/RB the hue, the stick saturation and brightness).
                // The CART (reference `c07`): leaving the exterior areas with parts on the car
                // that are not paid for shows them with their prices. PAY AND CHECK OUT (A)
                // buys and goes on, BACK returns to the parts, X takes the part under the
                // cursor off again, Y all of them.
                let unpaid: Vec<Slot> = if self.selected_car != self.current_car || self.flash_garage.is_none() { Vec::new() } else { CUSTOM_SLOTS.iter().copied().filter(|s| !self.part_is_owned(self.current_car, *s, self.cfg.get(*s))).collect() };
                if std::env::var("MCLA_CART_TEST").is_ok() && self.cart_open.is_none() {
                    // (test hook: fit a front bumper that is not paid for, then open the cart)
                    static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    if unpaid.is_empty() && !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        self.parts_owned.clear();
                        self.cfg.set(Slot::BumperF, 1);
                    } else if !unpaid.is_empty() {
                        self.cart_open = Some(0);
                    }
                }
                let (up, down, left, right, back, enter) = if let Some(sel) = self.cart_open {
                    let n = unpaid.len();
                    let mut leave = false;
                    if n == 0 {
                        self.cart_open = None;
                        leave = true;
                    } else {
                        let sel = if up { (sel + n - 1) % n } else if down { (sel + 1) % n } else { sel.min(n - 1) };
                        self.cart_open = Some(sel);
                        if enter {
                            if self.try_buy_parts() {
                                self.cart_open = None;
                                leave = true;
                            }
                        } else if back {
                            self.cart_open = None;
                        } else if ctx.input(|i| i.key_pressed(egui::Key::X)) {
                            self.cfg.set(unpaid[sel], 0);
                            self.dirty = true;
                        } else if ctx.input(|i| i.key_pressed(egui::Key::Y)) {
                            for s in &unpaid {
                                self.cfg.set(*s, 0);
                            }
                            self.dirty = true;
                        }
                    }
                    // (paid, or emptied: on out of the areas)
                    (false, false, false, false, leave, false)
                } else if back && RING.contains(&self.garage_menu) && self.garage_part.is_none() && self.garage_xpart.is_none() && self.plate_edit.is_none() && !unpaid.is_empty() {
                    self.cart_open = Some(0);
                    (false, false, false, false, false, false)
                } else {
                    (up, down, left, right, back, enter)
                };
                let mut pick_d = (0.0f32, 0.0f32, 0.0f32);
                // VINYL PACKAGES: the disc's presets for this kind of vehicle; APPLY puts the one
                // under the cursor on the car, BACK leaves the car as it is
                // (step 22: the rows of garage.CreateVinylPresetMenu - the vehicle's own preset,
                // the disc's, the stored packages - as "preset:<name>" / "package:<slot>")
                let pkg_rows = self.mc4_vinyl_rows();
                let packages: Vec<String> = pkg_rows.iter().map(|r| match r { crate::mc4::PkgRow::Preset(p) => format!("preset:{p}"), crate::mc4::PkgRow::User(k) => format!("package:{k}") }).collect();
                // (step 22: STORE VINYL PACKAGES - PACKAGE 1 .. 3, ACCEPT asks before it stores)
                let (up, down, left, right, back, enter) = if self.garage_menu == 15 && self.mc4_vinyl_save_page(up, down, enter, back) { (false, false, false, false, false, false) } else { (up, down, left, right, back, enter) };
                let (up, down, left, right, back, enter) = if let Some(sel) = self.vinyl_pkg {
                    let n = packages.len().max(1);
                    let sel = if up { (sel + n - 1) % n } else if down { (sel + 1) % n } else { sel.min(n - 1) };
                    self.vinyl_pkg = Some(sel);
                    if self.message_box.is_some() {
                    } else if enter {
                        // (ACCEPT asks first - OverwriteVinylsConfirmation - and YES puts the row
                        // on the vehicle: garage.LoadPresetVinyl; the list stays: step 22)
                        if let Some(r) = pkg_rows.get(sel).cloned() {
                            self.mc4_vinyl_ask(crate::mc4::VinylAsk::Load(r));
                        }
                    } else if back {
                        self.vinyl_pkg = None;
                    }
                    (false, false, false, false, false, false)
                } else {
                    (up, down, left, right, back, enter)
                };
                let (up, down, left, right, back, enter) = if let Some(saved) = self.paint_pick {
                    pick_d = (right as i32 as f32 - left as i32 as f32, up as i32 as f32 - down as i32 as f32, tab_next as i32 as f32 - tab_prev as i32 as f32);
                    if back {
                        self.paint_hsv = saved;
                        self.paint_pick = None;
                        self.dirty = true;
                    } else if enter {
                        self.paint_pick = None;
                    }
                    (false, false, false, false, false, false)
                } else if let Some((row, idx, hsv)) = self.paint_open {
                    let primary = row == CUSTOM_SLOTS.iter().position(|x| *x == Slot::Paint).map_or(0, |i| i + 1);
                    if back {
                        if primary {
                            self.color_idx = idx;
                            self.paint_hsv = hsv;
                        } else {
                            self.paint_b = idx;
                        }
                        self.paint_open = None;
                        self.dirty = true;
                    } else if enter {
                        self.paint_open = None;
                    } else if primary && ctx.input(|i| i.key_pressed(egui::Key::X)) {
                        // EDIT COLOR SLOT: the picker starts from the colour on the car
                        self.paint_pick = Some(self.paint_hsv);
                        if self.paint_hsv.is_none() {
                            self.paint_hsv = Some(paint_rgb_to_hsv(PALETTE[self.color_idx % PALETTE.len()]));
                        }
                    } else if primary && ctx.input(|i| i.key_pressed(egui::Key::Y)) {
                        // RESET COLOR SLOT: back to the slot's own colour
                        self.paint_hsv = None;
                        self.dirty = true;
                    }
                    (false, false, left, right, false, false)
                } else {
                    (up, down, left, right, back, enter)
                };
                // LICENSE PLATES: left / right along the plate, up / down through the characters
                // (`garage.ChangeLetter(+-1)`), LB / RB the kind of plate, ACCEPT keeps, BACK restores
                let (up, down, left, right, back, enter) = if let Some((cur, mut idx)) = self.plate_edit {
                    let cur = (cur + 7 + right as usize - left as usize) % 7;
                    if up || down {
                        let mut v = idx[cur] as isize;
                        loop {
                            v = (v + up as isize - down as isize).rem_euclid(43);
                            if v != 15 && v != 16 {
                                break;
                            }
                        }
                        idx[cur] = v as usize;
                    }
                    if tab_prev || tab_next {
                        self.plate_type = (self.plate_type + 4 + tab_next as usize - tab_prev as usize) % 4;
                    }
                    self.plate_edit = Some((cur, idx));
                    if enter {
                        let text: String = idx.iter().map(|i| lp_plate_char(*i)).collect();
                        self.plates.insert(self.dir_name(self.current_car).to_ascii_lowercase(), text);
                        self.plate_edit = None;
                        self.dirty = true;
                    } else if back {
                        self.plate_edit = None;
                    }
                    (false, false, false, false, false, false)
                } else {
                    (up, down, left, right, back, enter)
                };
                // INTERIOR COLORS: ACCEPT on a colour row opens its strip (left / right through the
                // swatches, ACCEPT keeps, BACK puts the old colour back)
                let (up, down, left, right, back, enter) = if let Some((k, saved)) = self.int_open {
                    let n = PALETTE.len();
                    let cell = if k < 6 { &mut self.int_colors[k] } else if k < 9 { &mut self.rim_cols[k - 6] } else { &mut self.car_x.int_de_idx[(k - 9).min(1)].0 };
                    if left || right {
                        *cell = (*cell + n + right as usize - left as usize) % n;
                        if k < 9 { self.slot_hsv[k] = None; } else { self.car_x.int_de_hsv[(k - 9).min(1)] = None; }
                        self.dirty = true;
                    }
                    let swatch = *cell;
                    // (test hook: MCLA_SLOT_PICK presses X once on the open strip)
                    static PICK_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    let hook = std::env::var("MCLA_SLOT_PICK").is_ok() && !PICK_ONCE.swap(true, std::sync::atomic::Ordering::Relaxed);
                    if hook || ctx.input(|i| i.key_pressed(egui::Key::X)) {
                        // EDIT COLOR SLOT: the picker starts from the row's colour; the car's own paint
                        // is set aside meanwhile
                        let from = (if k < 9 { self.slot_hsv[k] } else { self.car_x.int_de_hsv[(k - 9).min(1)] }).unwrap_or_else(|| paint_rgb_to_hsv(PALETTE[swatch % PALETTE.len()]));
                        self.pick_slot = Some((k.min(10), self.paint_hsv));
                        self.paint_pick = Some(Some(from));
                        self.paint_hsv = Some(from);
                    } else if ctx.input(|i| i.key_pressed(egui::Key::Y)) {
                        // RESET COLOR SLOT
                        if k < 9 { self.slot_hsv[k] = None; } else { self.car_x.int_de_hsv[(k - 9).min(1)] = None; }
                        self.dirty = true;
                    }
                    let cell = if k < 6 { &mut self.int_colors[k] } else if k < 9 { &mut self.rim_cols[k - 6] } else { &mut self.car_x.int_de_idx[(k - 9).min(1)].0 };
                    if back {
                        *cell = saved;
                        self.int_open = None;
                    } else if enter {
                        self.int_open = None;
                    }
                    (false, false, false, false, false, false)
                } else if self.garage_menu == 18 && self.garage_part.is_none() && enter && (9933..=9935).contains(&self.garage_row) {
                    let k = self.garage_row - 9933;
                    self.int_open = Some((6 + k, self.rim_cols[k]));
                    (up, down, left, right, back, false)
                } else if self.garage_menu == 17 && self.garage_part.is_none() && enter && (9941..=9948).contains(&self.garage_row) {
                    // (rows 9947, 9948 - INTERIOR COLOR D, E - are 9 and 10 here)
                    let k = if self.garage_row > 9946 { self.garage_row - 9947 + 9 } else { self.garage_row - 9941 };
                    self.int_open = Some((k, if k < 6 { self.int_colors[k] } else { self.car_x.int_de_idx[(k - 9).min(1)].0 }));
                    (up, down, left, right, back, false)
                } else {
                    (up, down, left, right, back, enter)
                };
                if let (Some((k, own)), None) = (self.pick_slot, self.paint_pick) {
                    // the picker has closed: what it holds is the row's colour (BACK left the old
                    // one there), and the car has its paint back
                    if k < 9 { self.slot_hsv[k] = self.paint_hsv; } else { self.car_x.int_de_hsv[(k - 9).min(1)] = self.paint_hsv; }
                    self.paint_hsv = own;
                    self.pick_slot = None;
                    self.dirty = true;
                }
                if self.paint_pick.is_some() && pick_d != (0.0, 0.0, 0.0) {
                    let mut hsv = self.paint_hsv.unwrap_or([0.0, 0.0, 1.0]);
                    hsv[0] = (hsv[0] + pick_d.2 / 48.0).rem_euclid(1.0);
                    hsv[1] = (hsv[1] + pick_d.0 * 0.05).clamp(0.0, 1.0);
                    hsv[2] = (hsv[2] + pick_d.1 * 0.05).clamp(0.0, 1.0);
                    self.paint_hsv = Some(hsv);
                    self.dirty = true;
                }
                let in_ring = RING.contains(&self.garage_menu);
                let (mut went_in, mut went_back, mut exit_now) = (false, false, false);
                if list_back {
                    went_back = true;
                }
                // (the GARAGE menu asks for a repair before its rows and before the way out:
                // `garage.RepairBeforeCustomizing` / `RepairBeforeExiting` - `mc4`. Its rows: MANAGE
                // VEHICLES, CUSTOMIZE VEHICLE, VEHICLE SHOWROOM, EXIT GARAGE.)
                let (enter, back) = self.mc4_repair_gate(!listing && self.garage_menu == 0 && self.garage_part.is_none() && self.garage_stack.is_empty(), enter, back, self.garage_cur == 3, self.garage_cur, vehicles.player().map(|c| (c.state.zones, c.state.damage)));
                // the two rows of the top menu open the list
                let enter = if !listing && enter && self.flash_garage.is_some() && self.garage_menu == 0 && self.garage_part.is_none() && self.garage_stack.is_empty() && (self.garage_cur == 0 || self.garage_cur == 2) {
                    self.garage_list = Some((self.garage_cur == 2, 0));
                    false
                } else {
                    enter
                };
                let mut part_d: isize = 0;
                let bike_in_garage = vehicles.player().map_or(false, |c| c.model.bike);
                // The rim shop (Rim.sc.xml) groups rims into maker tabs, so it is
                // the one part list with a second axis: up/down pick a rim of the
                // open brand, LB/RB (Q/E) step the brands, and the highlight is
                // kept per brand.
                // (RIMS and SOUTH CENTRAL RIMS are two lists of Wheels.sc.xml: RimSelectMenu and
                // DLC_RimSelectMenu)
                let rim_tabs: Vec<crate::rimshop::RimBrand> = self.rim_shop.tabs(bike_in_garage).into_iter().filter(|b| self.flash_garage.is_none() || (b.shop == crate::rimshop::Shop::Dlc) == self.rim_dlc).cloned().collect();
                let rim_label_pad = self.rim_shop.label_pad();
                let rim_open = self.garage_part.map_or(false, |row| CUSTOM_SLOTS[(row - 1).min(CUSTOM_SLOTS.len() - 1)] == Slot::Wheels) && !rim_tabs.is_empty();
                // (test hook: MCLA_CART_ASK=1 opens the part list's question)
                if self.garage_part.is_some() && self.cart_ask.is_none() && std::env::var("MCLA_CART_ASK").is_ok() && self.flash_garage.is_some() {
                    static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    if !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        self.cart_ask = Some(0);
                    }
                }
                // Choosing a part asks "ADD TO CART?" first (YES leaves the list with the part
                // on the car, to be paid for on leaving the garage; NO goes back to the list).
                let asking = self.cart_ask.is_some() && self.garage_part.is_some();
                let (enter, back) = if let (true, Some(sel)) = (asking, self.cart_ask) {
                    let sel = if up { 0 } else if down { 1 } else { sel };
                    self.cart_ask = Some(sel);
                    if enter || back {
                        self.cart_ask = None;
                    }
                    (enter && sel == 0, false)
                } else if self.garage_part.is_some() && enter && self.flash_garage.is_some() {
                    self.cart_ask = Some(0);
                    (false, back)
                } else {
                    (enter, back)
                };
                let (up, down, tab_prev, tab_next) = if asking { (false, false, false, false) } else { (up, down, tab_prev, tab_next) };
                if let Some((k, cur)) = self.garage_xpart {
                    // a list of XPARTS: up and down through the file's rows, ACCEPT takes the row's
                    // part (paid like any part unless free), BACK leaves it
                    let (_, sec, _, _) = XPARTS[k.min(XPARTS.len() - 1)];
                    let ent = self.xpart_rows(k);
                    let n = ent.len().max(1);
                    let cur = ((cur.min(n - 1)) as isize + down as isize - up as isize).rem_euclid(n as isize) as usize;
                    self.garage_xpart = Some((k, cur));
                    if enter {
                        let supported = ent.get(cur).is_some_and(|r| r.3 == 0
                            || matches!(sec, "tires" | "brakes" | "scissor_door")
                            || extra_variants.contains(&(sec.to_string(), r.3)));
                        if !self.mc4_part_row_open(crate::mc4::section_part_type(sec), cur, ent.len()) {
                            // (a closed row of a locked part type takes nothing)
                        } else if !supported {
                            self.show_message("PART UNAVAILABLE FOR THIS VEHICLE");
                        } else if self.try_buy_xpart(k, cur) {
                            self.garage_xpart = None;
                            self.garage_cur = 0;
                        }
                        went_in = true;
                    } else if sec == "brakes" && ctx.input(|i| i.key_pressed(egui::Key::X)) {
                        // STOCK BRAKES
                        let car = self.dir_name(self.current_car).to_ascii_lowercase();
                        self.xparts.entry(car).or_default().insert(sec.to_string(), 0);
                        self.dirty = true;
                        self.garage_xpart = None;
                    } else if back {
                        // (the area menu stands on its first row again when a list closes)
                        self.garage_xpart = None;
                        self.garage_cur = 0;
                        went_back = true;
                    }
                } else if let Some(row) = self.garage_part {
                    part_d = down as isize - up as isize;
                    if rim_open {
                        let n = rim_tabs.len();
                        let d = tab_next as isize - tab_prev as isize;
                        if d != 0 {
                            self.rim_brand = (self.rim_brand as isize + d).rem_euclid(n as isize) as usize;
                        }
                        let brand = &rim_tabs[self.rim_brand.min(n - 1)];
                        let m = brand.rims.len();
                        if m == 0 {
                            self.rim_pick = 0;
                        } else {
                            self.rim_pick = self.rim_pick.min(m - 1);
                            if part_d != 0 {
                                self.rim_pick = (self.rim_pick as isize + part_d).rem_euclid(m as isize) as usize;
                            }
                            // the rim under the highlight is the one fitted (the
                            // game's InstallRimOnVehicle, run as the row is entered)
                            if let Some(at) = brand.rims.get(self.rim_pick).and_then(|r| self.rim_by_name.get(&r.resource)) {
                                if self.cfg.rim != *at {
                                    self.cfg.set(Slot::Wheels, *at);
                                    self.dirty = true;
                                }
                            }
                        }
                    }
                    if back {
                        self.garage_part = None;
                        went_back = true;
                        if self.flash_garage.is_some() && RING.contains(&self.garage_menu) {
                            self.garage_cur = 0;
                        }
                    } else if enter {
                        self.garage_part = None;
                    }
                    self.garage_row = row;
                } else {
                    let items = &menus[self.garage_menu].1;
                    let ni = items.len();
                    self.garage_cur = self.garage_cur.min(ni - 1);
                    if up {
                        self.garage_cur = (self.garage_cur + ni - 1) % ni;
                    }
                    if down {
                        self.garage_cur = (self.garage_cur + 1) % ni;
                    }
                    if in_ring && (left || right) {
                        let at = RING.iter().position(|m| *m == self.garage_menu).unwrap_or(0);
                        self.garage_menu = RING[(at + RING.len() + right as usize - left as usize) % RING.len()];
                        self.garage_cur = 0;
                    }
                    let items = &menus[self.garage_menu].1;
                    match (enter, items[self.garage_cur.min(items.len() - 1)]) {
                        // (`garage.CheckMiscPartPurchased`: the paint shop and the vinyl editor
                        // are bought once per model)
                        (true, It::Sub(_, m)) if (m == 3 || m == 15) && !self.mc4_misc_check(if m == 3 { 0 } else { 1 }) => {}
                        (true, It::Sub(_, m)) => {
                            self.garage_stack.push((self.garage_menu, self.garage_cur));
                            self.garage_menu = m;
                            self.garage_cur = 0;
                            went_in = true;
                        }
                        (true, It::Part(_, slot)) if available_slots.contains(&slot) => {
                            if slot == Slot::Wheels && self.rim_dlc {
                                self.rim_dlc = false;
                                self.rim_brand = 0;
                            }
                            self.garage_part = Some(slot_row(slot));
                            self.mc4_part_list_opened(crate::mc4::slot_part_type(slot));
                            went_in = true;
                        }
                        (true, It::RowAs(_, 9968)) if self.flash_garage.is_some() => {
                            // LICENSE PLATES: the editor opens on the car's plate
                            let text = vehicles.player().map(|c| c.plate_text.to_ascii_uppercase()).unwrap_or_default();
                            let mut idx = [14usize; 7];
                            for (i, ch) in text.chars().take(7).enumerate() {
                                idx[i] = lp_place(ch);
                            }
                            self.plate_edit = Some((0, idx));
                            went_in = true;
                        }
                        (true, It::RowAs(_, 9961)) if self.flash_garage.is_some() => {
                            // SOUTH CENTRAL RIMS: the rim list with the downloadable brands
                            self.rim_dlc = true;
                            self.rim_brand = 0;
                            self.garage_part = Some(slot_row(Slot::Wheels));
                            went_in = true;
                        }
                        (true, It::RowAs(_, id)) if self.flash_garage.is_some() && XPARTS.iter().position(|x| x.0 == id).map_or(false, |k| available_xparts.contains(&k)) => {
                            let k = XPARTS.iter().position(|x| x.0 == id).unwrap_or(0);
                            // (the list opens on the part the car has)
                            let have = self.xparts.get(&self.dir_name(self.current_car).to_ascii_lowercase()).and_then(|m| m.get(XPARTS[k].1)).copied().unwrap_or(0);
                            let at = self.xpart_rows(k).iter().position(|r| r.3 == have).unwrap_or(0);
                            self.garage_xpart = Some((k, at));
                            self.mc4_part_list_opened(crate::mc4::section_part_type(XPARTS[k].1));
                            went_in = true;
                        }
                        (true, It::Exit) => exit_now = true,
                        _ => {
                            if back {
                                if let Some((m, c)) = self.garage_stack.pop() {
                                    self.garage_menu = m;
                                    self.garage_cur = c;
                                    went_back = true;
                                }
                            }
                        }
                    }
                    // (999: on a sub-menu's name - no setting is selected)
                    let items = &menus[self.garage_menu].1;
                    self.garage_row = match items[self.garage_cur.min(items.len() - 1)] {
                        It::Row(r) | It::RowAs(_, r) => r,
                        _ => 999,
                    };
                }
                let (left, right) = if self.garage_part.is_some() || part_d != 0 { (part_d < 0, part_d > 0) } else if in_ring { (false, false) } else { (left, right) };
                let enter = enter && !went_in && !exit_now;
                let back = (back && !went_back) || exit_now;
                let variants: Vec<(Slot, Vec<String>)> = vehicles.player().map(|c| c.model.variants.clone()).unwrap_or_default();
                let rim_count = self.rim_dirs.len().max(1);
                // A part list is the car's part file's (references `g_p_*`: the R8 has two hoods and
                // two spoilers where its model holds a third, the wide body's): the rows in the
                // file's order, each naming the model's part by its index.
                let part_order: Vec<(Slot, Vec<usize>)> = variants
                    .iter()
                    .filter_map(|(sl, v)| {
                        let e = self.part_entries(self.current_car, *sl)?;
                        let o: Vec<usize> = e.iter().map(|r| r.2).filter(|k| *k < v.len()).collect();
                        (!o.is_empty()).then(|| (*sl, o))
                    })
                    .collect();
                let order_of = |slot: Slot| -> Option<&Vec<usize>> { part_order.iter().find(|(x, _)| *x == slot).map(|(_, o)| o) };
                let count_for = |slot: Slot| -> usize {
                    if !available_slots.contains(&slot) { return 0; }
                    match slot {
                        Slot::Paint => PALETTE.len(),
                        Slot::Wheels => rim_count,
                        s => order_of(s).map(|o| o.len()).unwrap_or_else(|| variants.iter().find(|(x, _)| *x == s).map(|(_, v)| v.len()).unwrap_or(0)),
                    }
                };
                let enter = if self.garage_menu == 15 && self.garage_part.is_none() && self.flash_garage.is_some() {
                    if enter || std::env::var("MCLA_VINYL_PKG").is_ok() && self.vinyl_pkg.is_none() && self.garage_cur == 0 {
                        match self.garage_cur {
                            0 if !packages.is_empty() => {
                                let on = self.vinyls.get(&self.vinyl_for).and_then(|v| packages.iter().position(|p| p == v));
                                self.vinyl_pkg = Some(on.unwrap_or(0));
                            }
                            1 => self.vinyl_open_req = true,
                            2 if self.mc4_vinyl_has_layers() => self.mc4_vinyl_ask(crate::mc4::VinylAsk::RemoveAll),
                            3 if self.mc4_vinyl_has_layers() => self.car_x.vinyl_save = Some(0),
                            4 => {
                                if let Some((m, c)) = self.garage_stack.pop() {
                                    self.garage_menu = m;
                                    self.garage_cur = c;
                                }
                            }
                            _ => {}
                        }
                    }
                    false
                } else {
                    enter
                };
                let enter = if self.garage_menu == 14 && self.garage_part.is_none() && self.flash_garage.is_some() {
                    if enter {
                        self.ability_sel = self.garage_cur.min(3);
                        self.ability_pick = Some(self.ability_sel);
                        self.car_x.ability = Some(self.ability_sel);
                        self.dirty = true;
                        if let Some((m, c)) = self.garage_stack.pop() {
                            self.garage_menu = m;
                            self.garage_cur = c;
                        }
                    }
                    false
                } else {
                    enter
                };
                let colour_row = self.garage_menu == 3 && self.garage_part.is_none() && self.flash_garage.is_some() && (self.garage_row == slot_row(Slot::Paint) || (self.garage_row == fade_row + 1 && self.paint_fade != 0));
                let (left, right, enter) = if colour_row && self.paint_open.is_none() && self.paint_pick.is_none() {
                    if enter || std::env::var("MCLA_PAINT_OPEN").is_ok() {
                        self.paint_open = Some(if self.garage_row == fade_row + 1 { (self.garage_row, self.paint_b, None) } else { (self.garage_row, self.color_idx, self.paint_hsv) });
                        if std::env::var("MCLA_PAINT_OPEN").map_or(false, |v| v == "pick") {
                            self.paint_pick = Some(self.paint_hsv);
                            self.paint_hsv = Some(self.paint_hsv.unwrap_or_else(|| paint_rgb_to_hsv(PALETTE[self.color_idx % PALETTE.len()])));
                        }
                    }
                    (false, false, false)
                } else {
                    (left, right, enter)
                };
                if left || right {
                    let d: isize = if right { 1 } else { -1 };
                    if self.garage_menu == 12 && self.flash_garage.is_some() && (matches!(self.garage_row, 9970..=9973) || self.garage_row == rim_row) {
                        // DIMENSIONS: the axle, then the figures of that axle (both: the front's
                        // figure goes to the rear as well)
                        if self.garage_row == 9970 {
                            self.dim_axle = (self.dim_axle as isize + d).rem_euclid(3) as usize;
                        } else {
                            let mut v = self.car_dims(self.current_car);
                            // (place in the record, lowest and highest value)
                            let (at, lo, hi) = if self.garage_row == rim_row { (4, 12, 28) } else if self.garage_row == 9971 { (6, 0, 13) } else if self.garage_row == 9972 { (2, 0, 16) } else { (0, 0, 14) };
                            let step = if at == 0 { -d } else { d };
                            let axles: &[usize] = match self.dim_axle { 0 => &[0], 1 => &[1], _ => &[0, 1] };
                            let nv = (v[at + axles[0]] as isize + step).clamp(lo, hi) as u8;
                            for a in axles {
                                v[at + a] = nv;
                            }
                            self.dims.insert(self.dir_name(self.current_car).to_ascii_lowercase(), v);
                            // (the rims on the car follow the front's size)
                            self.rim_inches = v[4];
                            self.dirty = true;
                        }
                    } else if self.garage_row == 9975 && self.garage_menu == 3 {
                        // PAINT OPTIONS: ALL PARTS / PARTS
                        self.set_paint_area(0);
                        self.paint_parts = !self.paint_parts;
                    } else if self.garage_row == 9976 && self.garage_menu == 3 {
                        self.set_paint_area((self.paint_area as isize + d).rem_euclid(PAINT_AREAS.len() as isize) as usize);
                    } else if (9930..=9932).contains(&self.garage_row) {
                        // (a rim part's material: painted, chrome, brushed)
                        let k = self.garage_row - 9930;
                        self.rim_mats[k] = ((self.rim_mats[k] as isize + d).rem_euclid(3)) as u8;
                        self.dirty = true;
                    } else if self.garage_row == 9940 {
                        // (INTERIOR NEON on / off)
                        self.int_neon = !self.int_neon;
                        self.dirty = true;
                    } else if self.garage_row == 9990 {
                        // (AIR BAGS: garage.InstallHydraulics(0, on) - the vehicle's own, `mc4`)
                        self.car_x.airbags = !self.car_x.airbags;
                        self.dirty = true;
                    } else if self.garage_row == 9991 {
                        // (LOWERING PATTERN: garage.ChangeAirridePattern(0..4))
                        let o = &mut self.mc4_mut().options;
                        o.airride_pattern = (o.airride_pattern as isize + d).rem_euclid(5) as u8;
                        self.dirty = true;
                    } else if self.garage_row >= 999 {
                        // (a sub-menu's name)
                    } else if self.garage_row == 0 && self.flash_garage.is_some() {
                        // (the port's own old garage list had a VEHICLE row here, turned with left
                        // and right. With the original's garage movie that row is not shown but its
                        // cursor still stood on it: left / right on the GARAGE menu leafed through
                        // the cars and brought the old panel up. The original does nothing there -
                        // vehicles are changed in MANAGE VEHICLES.)
                    } else if self.garage_row == 0 {
                        self.selected_car = ((self.selected_car as isize + d).rem_euclid(n as isize)) as usize;
                        self.perf_sel = self.perf_of(self.selected_car);
                    } else if self.garage_row == perf_row {
                        // (the row opens the shop)
                    } else if self.garage_row >= hsv_row && self.garage_row < hsv_row + 3 {
                        // start from the swatch that is on the car
                        let mut hsv = self.paint_hsv.unwrap_or_else(|| {
                            let c = PALETTE[self.color_idx % PALETTE.len()];
                            let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
                            let d = mx - mn;
                            let h = if d < 1e-4 {
                                0.0
                            } else if mx == c[0] {
                                ((c[1] - c[2]) / d).rem_euclid(6.0) / 6.0
                            } else if mx == c[1] {
                                ((c[2] - c[0]) / d + 2.0) / 6.0
                            } else {
                                ((c[0] - c[1]) / d + 4.0) / 6.0
                            };
                            [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
                        });
                        let k = self.garage_row - hsv_row;
                        let step = if k == 0 { 1.0 / 48.0 } else { 0.05 };
                        hsv[k] += d as f32 * step;
                        hsv[k] = if k == 0 { hsv[k].rem_euclid(1.0) } else { hsv[k].clamp(0.0, 1.0) };
                        self.paint_hsv = Some(hsv);
                        self.dirty = true;
                    } else if self.garage_row == neon_row {
                        self.neon_on = !self.neon_on;
                        self.dirty = true;
                    } else if self.garage_row == neon_row + 1 {
                        self.neon_color = (self.neon_color as isize + d).rem_euclid(PALETTE.len() as isize) as usize;
                        self.dirty = true;
                    } else if self.garage_row == tint_row {
                        // (ten rows, Tint_1 .. Tint_10: garage.ChangeTintTransparency(0..9))
                        self.tint_level = (self.tint_level as isize + d).rem_euclid(10) as u8;
                        self.dirty = true;
                    } else if self.garage_row == tint_row + 1 {
                        self.tint_color = (self.tint_color as isize + d).rem_euclid(PALETTE.len() as isize) as usize;
                        self.dirty = true;
                    } else if self.garage_row == fade_row {
                        self.paint_fade = (self.paint_fade as isize + d).rem_euclid(5) as u8;
                        self.dirty = true;
                    } else if self.garage_row == fade_row + 1 {
                        self.paint_b = (self.paint_b as isize + d).rem_euclid(PALETTE.len() as isize) as usize;
                        self.dirty = true;
                    } else if self.garage_row == ptype_row {
                        // (the shop's order: Gloss, Metallic, Pearlescent, Sparkle, Matte)
                        const ORDER: [u8; 5] = [2, 3, 4, 5, 1];
                        let at = ORDER.iter().position(|t| *t == self.paint_type.clamp(1, 5)).unwrap_or(0) as isize;
                        self.paint_type = ORDER[(at + d).rem_euclid(5) as usize];
                        self.dirty = true;
                    } else if self.garage_row == hyd_row {
                        self.hydraulics = !self.hydraulics;
                        self.dirty = true;
                    } else if self.garage_row == rim_row {
                        const SIZES: [u8; 9] = [0, 15, 16, 17, 18, 19, 20, 21, 22];
                        let cur = SIZES.iter().position(|s| *s == self.rim_inches).unwrap_or(0) as isize;
                        self.rim_inches = SIZES[(cur + d).rem_euclid(SIZES.len() as isize) as usize];
                        self.dirty = true;
                    } else if self.garage_row == vinyl_row {
                        // none, then the liveries of this car
                        let n = self.vinyl_list.len() as isize + 1;
                        let car = self.vinyl_for.clone();
                        let cur = self.vinyls.get(&car).and_then(|v| self.vinyl_list.iter().position(|x| x == v)).map_or(0, |i| i as isize + 1);
                        let nv = (cur + d).rem_euclid(n);
                        if nv == 0 {
                            self.vinyls.remove(&car);
                        } else {
                            self.vinyls.insert(car, self.vinyl_list[nv as usize - 1].clone());
                        }
                    } else {
                        let slot = CUSTOM_SLOTS[self.garage_row - 1];
                        let c = count_for(slot);
                        // (a locked list keeps its first rows open; the others are closed one by one -
                        // sub_826A92E0, `mc4_part_row_open` - and nothing is said about it)
                        let lock: Option<(i64, i64, String)> = None;
                        if let (Some((need, have, class)), true) = (lock.as_ref(), c > 0) {
                            // the part has not arrived at the garage yet: so many wins in this class of car
                            self.show_message(&format!("LOCKED - {} OF {} {} WINS", have, need, class));
                        }
                        // (a part fitted earlier may always come off again)
                        let fitted = slot != Slot::Paint && self.cfg.get(slot) != 0;
                        if c > 0 && (lock.is_none() || fitted) {
                            let order = order_of(slot).filter(|_| slot != Slot::Paint);
                            let cur = if slot == Slot::Paint { self.color_idx } else { self.cfg.get(slot) };
                            // (through the list's rows, which name the parts in the file's order)
                            let cur = order.map_or(cur, |o| o.iter().position(|k| *k == cur).unwrap_or(0));
                            let nv = {
                                let mut n = cur;
                                for _ in 0..c {
                                    n = ((n as isize + d).rem_euclid(c as isize)) as usize;
                                    if self.mc4_part_row_open(crate::mc4::slot_part_type(slot), n, c) {
                                        break;
                                    }
                                }
                                n
                            };
                            let nv = order.map_or(nv, |o| o[nv.min(o.len() - 1)]);
                            if slot == Slot::Paint {
                                self.color_idx = nv;
                                self.paint_hsv = None;
                            } else {
                                self.cfg.set(slot, nv);
                            }
                            self.dirty = true;
                        }
                    }
                }
                let car_changed = self.selected_car != self.current_car;
                let car_label = format!("{}  [{}/{}]", self.car_name(self.selected_car), self.selected_car + 1, n);
                let rim_label = self.rim_names.get(self.cfg.rim % rim_count).cloned().unwrap_or_else(|| "stock".into());
                let color_idx = self.color_idx;
                let cfg = self.cfg.clone();
                let garage_row = self.garage_row;
                let mut clicked_color: Option<usize> = None;
                let labels: Vec<(Slot, Vec<String>)> = vehicles.player().map(|c| c.model.variant_labels.clone()).unwrap_or_default();
                let bike_in_garage = vehicles.player().map_or(false, |c| c.model.bike);
                {
                    let sc = (screen.height() / 900.0).clamp(0.6, 2.0);
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("garage")));
                    // visible rows: vehicle + the slots this car has
                    let mut rows_v: Vec<(usize, String, String, usize)> = vec![(0, "VEHICLE".to_string(), car_label.to_uppercase(), n)];
                    for (i, slot) in CUSTOM_SLOTS.iter().enumerate() {
                        let count = count_for(*slot);
                        let value = match slot {
                            Slot::Paint => if self.paint_hsv.is_some() { "CUSTOM".to_string() } else { format!("COLOUR {}", color_idx + 1) },
                            Slot::Wheels => rim_label.to_uppercase(),
                            sl => labels.iter().find(|(x, _)| x == sl).and_then(|(_, v)| v.get(cfg.get(*sl) % v.len().max(1)).cloned()).unwrap_or_else(|| "-".into()),
                        };
                        // a part that is not the player's yet shows its price
                        let v = cfg.get(*slot);
                        let value = if !car_changed && !self.part_is_owned(self.current_car, *slot, v) {
                            let p = self.part_price(self.current_car, *slot, v);
                            if p > 0 { format!("{}   ${}", value, p) } else { value }
                        } else {
                            value
                        };
                        if count > 0 || matches!(slot, Slot::Paint | Slot::Wheels) {
                            rows_v.push((i + 1, slot.label().to_uppercase(), value, count));
                        }
                    }
                    if !car_changed {
                        let st = self.perf_stages(self.current_car);
                        let fitted: u32 = st.iter().map(|v| *v as u32).sum();
                        let value = format!("{} OF {} UPGRADES   ENTER", fitted, st.len() * 2);
                        rows_v.push((perf_row, "PERFORMANCE".to_string(), value, 0));
                        if !self.vinyl_list.is_empty() {
                            let cur = self.vinyls.get(&self.vinyl_for).and_then(|v| self.vinyl_list.iter().position(|x| x == v));
                            let value = match cur {
                                Some(i) => format!("{}   ENTER TO EDIT", vinyl_label(&self.vinyl_list[i], i)),
                                None => "NONE   ENTER TO EDIT".to_string(),
                            };
                            rows_v.push((vinyl_row, "VINYL".to_string(), value, self.vinyl_list.len() + 1));
                        }
                        let hsv = self.paint_hsv;
                        for (k, label) in ["PAINT HUE", "PAINT SATURATION", "PAINT BRIGHTNESS"].iter().enumerate() {
                            let value = match hsv {
                                Some(v) => format!("{:.0}{}", if k == 0 { v[0] * 360.0 } else { v[k] * 100.0 }, if k == 0 { " DEG" } else { " %" }),
                                None => "-".to_string(),
                            };
                            rows_v.push((hsv_row + k, label.to_string(), value, 2));
                        }
                        rows_v.push((ptype_row, "PAINT TYPE".to_string(), crate::render::PAINT_TYPES[self.paint_type.min(5) as usize].to_string(), 5));
                        rows_v.push((fade_row, "FADE TYPE".to_string(), ["NONE", "TWO TONE", "FRONT BACK", "LEFT RIGHT", "TOP BOTTOM"][self.paint_fade.min(4) as usize].to_string(), 5));
                        rows_v.push((fade_row + 1, "SECONDARY COLOUR".to_string(), if self.paint_fade == 0 { "-".to_string() } else { format!("COLOUR {}", self.paint_b % PALETTE.len() + 1) }, PALETTE.len()));
                        rows_v.push((tint_row, "WINDOW TINT".to_string(), if self.tint_level == 0 { "STOCK".to_string() } else { format!("{} OF 10", self.tint_level + 1) }, 10));
                        rows_v.push((tint_row + 1, "TINT COLOUR".to_string(), if self.tint_level == 0 { "-".to_string() } else { format!("COLOUR {}", self.tint_color % PALETTE.len() + 1) }, PALETTE.len()));
                        rows_v.push((neon_row, "NEON".to_string(), if self.neon_on { "ON" } else { "OFF" }.to_string(), 2));
                        rows_v.push((neon_row + 1, "NEON COLOUR".to_string(), if self.neon_on { format!("COLOUR {}", self.neon_color % PALETTE.len() + 1) } else { "-".to_string() }, PALETTE.len()));
                        // (the paint shop's two leading choices)
                        rows_v.push((9975, "PAINT OPTIONS".to_string(), "ALL PARTS".to_string(), 2));
                        rows_v.push((9976, "PAINT AREA".to_string(), "MAIN BODY".to_string(), 6));
                        rows_v.push((rim_row, "RIM SIZE".to_string(), if self.rim_inches == 0 { "STOCK".to_string() } else { format!("{} INCH", self.rim_inches) }, 9));
                        if !bike_in_garage {
                            // (the disc's title is "Hydraulics/Air Bags"; the air bags have no travel in its tuning)
                            let label = self.career.strings.get("G_Hydraulics").map(|s| s.split('/').next().unwrap_or(s).trim().to_uppercase()).unwrap_or_else(|| "HYDRAULICS".into());
                            let (on, off) = (self.career.strings.get("G_On").map(|s| s.to_uppercase()).unwrap_or_else(|| "ON".into()), self.career.strings.get("G_Off").map(|s| s.to_uppercase()).unwrap_or_else(|| "OFF".into()));
                            rows_v.push((hyd_row, label, if self.hydraulics { on } else { off }, 2));
                        }
                    }
                    // The game's menu box: a small dark panel at the top right with the menu's
                    // name in yellow and the selected line on a teal bar; prompts and the
                    // money along the bottom.
                    let (mut title, mut items) = (menus[self.garage_menu].0, menus[self.garage_menu].1.clone());
                    // An open part list, as the game sets it out: STOCK, then "1 OF n" ... with
                    // each one's price (nothing for a part already owned); the chosen line on
                    // the teal bar with its name beneath.
                    // Rims are the exception: Rim.sc.xml puts them in maker tabs on the
                    // movie's own template 47, with the maker logo and `List_max` 4.
                    let flash_part_template = self.garage_xpart.is_some() || self.garage_part.map_or(false, |row| CUSTOM_SLOTS[(row - 1).min(CUSTOM_SLOTS.len() - 1)] != Slot::Wheels);
                    for id in (9940..=9948usize).chain(9930..=9935) {
                        rows_v.push((id, String::new(), String::new(), 0));
                    }
                    for (k, (id, _, t, _)) in XPARTS.iter().enumerate() {
                        if self.flash_garage.is_some() && available_xparts.contains(&k) {
                            rows_v.push((*id, t.to_string(), String::new(), 0));
                        }
                    }
                    if self.flash_garage.is_some() {
                        rows_v.push((9961, String::new(), String::new(), 0));
                        rows_v.push((9968, String::new(), String::new(), 0));
                    }
                    let mut part_rows: Vec<(String, i64, bool, String, bool)> = Vec::new();
                    let mut part_brands: Vec<Option<i32>> = Vec::new();
                    if rim_open {
                        // the rims of the brand under the LB/RB tabs, at the game's prices
                        let brand = rim_tabs[self.rim_brand.min(rim_tabs.len() - 1)].clone();
                        for (i, r) in brand.rims.iter().enumerate() {
                            let owned = self.rim_by_name.get(&r.resource).map_or(true, |at| self.rim_owned(self.current_car, *at));
                            // the name the disc shows: the folder with whl_/am_ off
                            let name = r.label();
                            part_rows.push((name, if owned || self.parts_free() { 0 } else { r.price }, i == self.rim_pick, String::new(), true));
                        }
                        title = "RIMS";
                        items.clear();
                    } else if let Some((k, cur)) = self.garage_xpart {
                        let (_, sec, t, _) = XPARTS[k.min(XPARTS.len() - 1)];
                        title = t;
                        items.clear();
                        let ent = self.xpart_rows(k);
                        for (i, r) in ent.iter().enumerate() {
                            part_brands.push(Some(r.1));
                            let supported = r.3 == 0 || matches!(sec, "tires" | "brakes" | "scissor_door")
                                || extra_variants.contains(&(sec.to_string(), r.3));
                            let supported = supported && self.mc4_part_row_open(crate::mc4::section_part_type(sec), i, ent.len());
                            part_rows.push((r.0.clone(), r.2, i == cur.min(ent.len().saturating_sub(1)), String::new(), supported));
                        }
                    } else if let Some(row) = self.garage_part {
                        let slot = CUSTOM_SLOTS[(row - 1).min(CUSTOM_SLOTS.len() - 1)];
                        title = menus.iter().flat_map(|m| m.1.iter()).find_map(|it| match it { It::Part(l, sl) if *sl == slot => Some(*l), _ => None }).unwrap_or(title);
                        items.clear();
                        let count = rows_v.iter().find(|x| x.0 == row).map_or(0, |x| x.3).max(1);
                        let order = order_of(slot).cloned().unwrap_or_else(|| (0..count).collect());
                        let cur = order.iter().position(|k| *k == cfg.get(slot)).unwrap_or(0);
                        let names = labels.iter().find(|(x, _)| *x == slot).map(|(_, v)| v.clone()).unwrap_or_default();
                        // (long lists - the rims - scroll about the chosen one)
                        let (first, end) = if self.flash_garage.is_some() && flash_part_template { (0, count) } else {
                            let first = cur.saturating_sub(4).min(count.saturating_sub(9));
                            (first, count.min(first + 9))
                        };
                        for i in first..end {
                            let v = order.get(i).copied().unwrap_or(i);
                            let name = names.get(v).cloned().unwrap_or_default().to_uppercase();
                            let label = if i == 0 { "STOCK".to_string() } else { format!("{} OF {}", i, count - 1) };
                            let price = if self.part_is_owned(self.current_car, slot, v) { 0 } else { self.part_price(self.current_car, slot, v) };
                            let name = if name == label { String::new() } else { name };
                            // (the row's flag 16 of the original: closed rows of a locked part type)
                            let enabled = self.mc4_part_row_open(crate::mc4::slot_part_type(slot), i, count);
                            // (the row's brand: the logo id of the car's part file)
                            part_brands.push(self.part_entries(self.current_car, slot).and_then(|e| e.iter().find(|r| r.2 == v)).map(|r| r.0));
                            part_rows.push((label, price, i == cur, name, enabled));
                        }
                    }
                    // with the garage movie active the port's box is drawn nowhere (its rows go to the movie)
                    let bp = if self.flash_garage.is_some() { painter.with_clip_rect(egui::Rect::NOTHING) } else { painter.clone() };
                    let mut flash_rows: Vec<crate::flash_ui::MenuRow> = Vec::new();
                    let mut flash_sel = 0usize;
                    // where the garage movie laid out its menu box, so the outline and the
                    // prompt bar below can line up with it
                    let mut flash_box: Option<egui::Rect> = None;
                    // the menu went through the 3D scene and the prompt movie: the port's own
                    // bar and stats card stay away
                    let mut flash_3d = false;
                    let w = 430.0 * sc;
                    let row_h = 28.0 * sc;
                    let x0 = screen.right() - w - 70.0 * sc;
                    let y0 = screen.top() + 64.0 * sc;
                    let h = 62.0 * sc + row_h * (items.len() + part_rows.len() + part_rows.iter().filter(|r| r.2 && !r.3.is_empty()).count()) as f32;
                    let panel = egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(w, h));
                    bp.rect_filled(panel, 10.0 * sc, egui::Color32::from_rgba_unmultiplied(8, 12, 14, 228));
                    bp.rect_stroke(panel, 10.0 * sc, egui::Stroke::new(2.5 * sc, egui::Color32::from_rgb(200, 206, 206)), egui::StrokeKind::Inside);
                    let tx = x0 + 26.0 * sc;
                    if let Some(at) = RING.iter().position(|m| *m == self.garage_menu).filter(|_| self.garage_part.is_none() && self.garage_xpart.is_none() && self.plate_edit.is_none()) {
                        // (the ring of areas: the one before and the one after, dim, to either side)
                        let (prev, next) = (menus[RING[(at + RING.len() - 1) % RING.len()]].0, menus[RING[(at + 1) % RING.len()]].0);
                        let cx = panel.center().x;
                        let yel = egui::Color32::from_rgb(255, 205, 50);
                        let r = self.text(&bp, egui::pos2(cx, y0 + 14.0 * sc), egui::Align2::CENTER_TOP, title, 17.0 * sc, yel);
                        for (x, dir) in [(r.left() - 9.0 * sc, -1.0f32), (r.right() + 9.0 * sc, 1.0)] {
                            let cy = r.center().y;
                            bp.add(egui::Shape::convex_polygon(vec![egui::pos2(x + 5.0 * sc * dir, cy), egui::pos2(x - 3.0 * sc * dir, cy - 7.0 * sc), egui::pos2(x - 3.0 * sc * dir, cy + 7.0 * sc)], yel, egui::Stroke::NONE));
                        }
                        self.text(&bp, egui::pos2(r.left() - 22.0 * sc, y0 + 17.0 * sc), egui::Align2::RIGHT_TOP, prev, 12.0 * sc, egui::Color32::from_gray(150));
                        self.text(&bp, egui::pos2(r.right() + 22.0 * sc, y0 + 17.0 * sc), egui::Align2::LEFT_TOP, next, 12.0 * sc, egui::Color32::from_gray(150));
                    } else {
                        self.text(&bp, egui::pos2(x0 + 20.0 * sc, y0 + 14.0 * sc), egui::Align2::LEFT_TOP, title, 17.0 * sc, egui::Color32::from_rgb(255, 205, 50));
                    }
                    bp.line_segment([egui::pos2(x0 + 12.0 * sc, y0 + 42.0 * sc), egui::pos2(panel.right() - 12.0 * sc, y0 + 42.0 * sc)], egui::Stroke::new(1.0 * sc, egui::Color32::from_gray(90)));
                    let list_y = y0 + 48.0 * sc;
                    let pointer = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_clicked()));
                    let _ = garage_row;
                    let mut py = list_y;
                    // The logo under the chosen line: the car's maker for the stock part, the
                    // game's own "custom" badge for the others (reference: R8 front bumpers).
                    let maker_logo = crate::flash_ui::maker_logo(&self.dir_name(self.current_car));
                    for (label, price, chosen, name, enabled) in &part_rows {
                        if *chosen {
                            flash_sel = flash_rows.len();
                        }
                        let brand = match part_brands.get(flash_rows.len()).copied().flatten() {
                            Some(b) => b,
                            None => if flash_rows.is_empty() { maker_logo } else { 0 },
                        };
                        flash_rows.push(crate::flash_ui::MenuRow {
                            label: label.clone(), value: format!("${}", price), enabled: *enabled,
                            part: (flash_part_template || rim_open).then_some(crate::flash_ui::PartRow { price: *price, brand }),
                        });
                        let tall = if *chosen && !name.is_empty() { 2.0 } else { 1.0 };
                        let r = egui::Rect::from_min_size(egui::pos2(x0 + 8.0 * sc, py), egui::vec2(w - 16.0 * sc, row_h * tall - 2.0 * sc));
                        let cy = py + row_h * 0.5;
                        if *chosen {
                            bp.rect_filled(r, 2.0 * sc, egui::Color32::from_rgb(34, 150, 158));
                            bp.add(egui::Shape::convex_polygon(vec![egui::pos2(x0 + 13.0 * sc, cy - 5.0 * sc), egui::pos2(x0 + 20.0 * sc, cy), egui::pos2(x0 + 13.0 * sc, cy + 5.0 * sc)], egui::Color32::WHITE, egui::Stroke::NONE));
                            if !name.is_empty() {
                                self.text(&bp, egui::pos2(panel.center().x, cy + row_h), egui::Align2::CENTER_CENTER, name, 13.0 * sc, egui::Color32::WHITE);
                            }
                        }
                        let col = if *chosen { egui::Color32::WHITE } else { egui::Color32::from_gray(205) };
                        self.text(&bp, egui::pos2(tx, cy), egui::Align2::LEFT_CENTER, label, 12.0 * sc, col);
                        self.text(&bp, egui::pos2(panel.right() - 20.0 * sc, cy), egui::Align2::RIGHT_CENTER, &format!("${}", price), 12.0 * sc, col);
                        py += row_h * tall;
                    }
                    for (k, it) in items.iter().enumerate() {
                        let r = egui::Rect::from_min_size(egui::pos2(x0 + 8.0 * sc, list_y + row_h * k as f32), egui::vec2(w - 16.0 * sc, row_h - 2.0 * sc));
                        if pointer.1 && pointer.0.map_or(false, |p| r.contains(p)) {
                            self.garage_cur = k;
                        }
                        let selected = k == self.garage_cur;
                        let (label, value, count) = match it {
                            It::Sub(l, _) => (l.to_string(), String::new(), 0),
                            It::Exit => ("EXIT GARAGE".to_string(), String::new(), 0),
                            // (a kind of part this car has none of stays grey)
                            It::Part(l, sl) => (l.to_string(), if rows_v.iter().any(|x| x.0 == slot_row(*sl)) { String::new() } else { "-".to_string() }, 0),
                            It::Row(id) | It::RowAs(_, id) => match rows_v.iter().find(|x| x.0 == *id) {
                                Some((_, l, v, c)) => {
                                    // (the game's own names for these lines)
                                    let l = if *id == perf_row { "PERFORMANCE SHOP".to_string() } else if *id == vinyl_row { "VINYL EDITOR".to_string() } else if *id == slot_row(Slot::Paint) { "PRIMARY COLOR".to_string() } else if *id == slot_row(Slot::Wheels) { "RIMS".to_string() } else if *id == slot_row(Slot::Seats) { "INTERIOR".to_string() } else { l.clone() };
                                    let v = if *id == perf_row { String::new() } else if *id == vinyl_row { v.replace("   ENTER TO EDIT", "") } else { v.clone() };
                                    (l, v, *c)
                                }
                                // (this car has no such part)
                                None => (if (1..=CUSTOM_SLOTS.len()).contains(id) { CUSTOM_SLOTS[*id - 1].label().to_uppercase() } else { "-".to_string() }, "-".to_string(), 0),
                            },
                        };
                        let label = if let It::RowAs(l, _) = it { l.to_string() } else { label };
                        let value = if matches!(it, It::Part(..)) && value == "-" { String::new() } else { value };
                        let grey = matches!(it, It::Part(_, sl) if !rows_v.iter().any(|x| x.0 == slot_row(*sl)));
                        let dim = grey || count == 0 && matches!(it, It::Row(_) | It::RowAs(..)) && value == "-";
                        if selected {
                            flash_sel = flash_rows.len();
                        }
                        flash_rows.push(crate::flash_ui::MenuRow { label: label.clone(), value: if matches!(it, It::RowAs(_, 0)) { String::new() } else { value.clone() }, enabled: !dim, part: None });
                        // (a part type the career has just unlocked wears the "new" mark until its list
                        // is opened: the row's `type` 50 - sub_826A8D48, `mc4_part_new`)
                        let s16_new = match it {
                            It::Part(_, sl) => self.mc4_part_new(crate::mc4::slot_part_type(*sl)),
                            It::RowAs(_, id) => XPARTS.iter().find(|x| x.0 == *id).map_or(false, |x| self.mc4_part_new(crate::mc4::section_part_type(x.1))),
                            _ => false,
                        };
                        if s16_new && !dim {
                            if let Some(r) = flash_rows.last_mut() {
                                r.value = crate::flash_ui::NEW_ROW.to_string();
                            }
                        }
                        let col = if selected { egui::Color32::WHITE } else if dim { egui::Color32::from_gray(110) } else { egui::Color32::from_gray(205) };
                        let cy = r.center().y;
                        if selected {
                            bp.rect_filled(r, 2.0 * sc, egui::Color32::from_rgb(34, 150, 158));
                            bp.add(egui::Shape::convex_polygon(vec![egui::pos2(x0 + 13.0 * sc, cy - 5.0 * sc), egui::pos2(x0 + 20.0 * sc, cy), egui::pos2(x0 + 13.0 * sc, cy + 5.0 * sc)], egui::Color32::WHITE, egui::Stroke::NONE));
                        }
                        self.text(&bp, egui::pos2(tx, cy), egui::Align2::LEFT_CENTER, &label, 12.0 * sc, col);
                        if !value.is_empty() {
                            // (the vehicle's name stands under the menu instead)
                            let value = if matches!(it, It::RowAs(_, 0)) { String::new() } else { value };
                            // long names shrink to fit
                            let avail = w * 0.52;
                            let mut size = 12.0 * sc;
                            if let Some(f) = self.art.as_ref().and_then(|a| a.font.as_ref()) {
                                let wv = f.measure(&value, size);
                                if wv > avail {
                                    size *= avail / wv;
                                }
                            }
                            self.text(&bp, egui::pos2(panel.right() - 20.0 * sc, cy), egui::Align2::RIGHT_CENTER, &value, size, col);
                        }
                    }
                    if self.garage_menu == 0 {
                        self.text(&bp, egui::pos2(panel.center().x, panel.bottom() + 16.0 * sc), egui::Align2::CENTER_TOP, &format!("<  {}  >", car_label.to_uppercase()).replace('<', "").replace('>', ""), 13.0 * sc, egui::Color32::WHITE);
                    }
                    if self.flash_garage.is_some() {
                        use crate::flash_ui::{MenuLayout, MenuPage, ICON_A, ICON_B};
                        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).unwrap_or_else(|| d.to_string());
                        let mut rows = flash_rows.clone();
                        // (the original's top menu is a plain list; the name only shows while another
                        // vehicle is being looked at, or on the flat fallback)
                        if self.garage_menu == 0 && self.garage_part.is_none() && (car_changed || self.garage_panel.is_none()) {
                            // the vehicle's name stands under the menu in the port; here it is the first row's value
                            if let Some(r) = rows.first_mut() {
                                r.value = self.car_name(self.selected_car).to_uppercase();
                            }
                        }
                        let owned_sel = self.is_owned(self.selected_car);
                        let accept = if car_changed && !owned_sel { st("prompt_purchase", "PURCHASE") } else { st("prompt_accept", "ACCEPT") };
                        let vehicle_list = self.garage_list.filter(|_| listing).map(|(showroom, sel)| {
                            let st2 = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).unwrap_or_else(|| d.to_string());
                            let info = |i: usize| self.catalog.get(&self.dir_name(i)).cloned();
                            let details: Vec<(String, String, i64)> = list_cars
                                .iter()
                                .map(|&i| {
                                    let c = info(i);
                                    // the class the car races in now, and the group that unlocks it (D..A = 1..4)
                                    let group = c.as_ref().map(|c| c.unlock_group.clone()).unwrap_or_default();
                                    let gn = match group.chars().next().map(|x| x.to_ascii_uppercase()) { Some('A') => 4, Some('B') => 3, Some('C') => 2, _ => 1 };
                                    let class = c.as_ref().and_then(|c| c.group.chars().next()).unwrap_or('D').to_ascii_uppercase();
                                    // (approx.) upgrades move a car up the classes ("vehicles can be upgraded or
                                    // downgraded to match a performance class"): the fully upgraded R8 of the 100 %
                                    // save is A-CLASS where the catalogue has it in B (reference `g_mv0`). The game's
                                    // rule was not read: a car with every part at its last stage counts one class up.
                                    let class = if !showroom && self.perf_frac(i) > 0.999 { match class { 'D' => 'C', 'C' => 'B', _ => 'A' } } else { class };
                                    let kind = c.as_ref().map(|c| c.class.to_uppercase()).unwrap_or_default();
                                    // (the catalogue's SPORTBIKE reads SPORTS BIKE: reference `g_mv1`)
                                    let kind = if kind == "SPORTBIKE" { self.career.strings.get("SportBike").filter(|v| !v.contains('<')).map(|v| v.to_uppercase()).unwrap_or_else(|| "SPORTS BIKE".into()) } else { kind };
                                    (format!("VEHICLE: {}-CLASS / GROUP {}", class, gn), kind, if showroom { self.price_of(i).0 } else { self.trade_in(i) })
                                })
                                .collect();
                            let c = info(list_cars[sel.min(list_cars.len() - 1)]);
                            let f = if showroom { 0.0 } else { self.perf_frac(list_cars[sel.min(list_cars.len() - 1)]) };
                            let meters = match c {
                                Some(c) => {
                                    // (teal: the stock car; yellow up to what is fitted; the dark bar is what
                                    // the car can reach)
                                    let row = |base: f32, mods: f32| [(base + (mods - base) * f) * 100.0, base * 100.0, mods * 100.0];
                                    [row(c.base_accel * METER_ACCEL, c.mods_accel * METER_ACCEL), row((c.base_speed - 100.0) * METER_SPEED, (c.mods_speed - 100.0) * METER_SPEED), row(c.base_handling, c.mods_handling)]
                                }
                                None => [[0.0, 0.0, 100.0]; 3],
                            };
                            let names: Vec<crate::flash_ui::MenuRow> = list_cars.iter().map(|&i| crate::flash_ui::MenuRow { label: { let d = self.dir_name(i).to_ascii_lowercase(); self.career.strings.get(&format!("{d}_shrt")).or_else(|| self.career.strings.get(&d)).map(|v| v.to_uppercase()).unwrap_or_else(|| self.car_name(i).to_uppercase()) }, value: String::new(), enabled: true, part: None }).collect();
                            let title = if showroom { st2("G_Showroom", "VEHICLE SHOWROOM") } else { st2("G_ManageRides", "MANAGE VEHICLES") };
                            // (the showroom's carousel wants each vehicle's id and whether it is for sale)
                            let ids: Vec<(i32, bool)> = if showroom && std::env::var("MCLA_SHOWROOM_LIST").is_err() { list_cars.iter().map(|&i| (mcla_formats::hash::joaat(&self.dir_name(i).to_ascii_lowercase()) as i32, self.rep >= self.price_of(i).1 || std::env::var("MCLA_UNLOCK_ALL").is_ok())).collect() } else { Vec::new() };
                            (MenuLayout::Vehicles { details, meters, value_label: if showroom { "PRICE:".to_string() } else { st2("G_Trade_In_Value", "TRADE IN VALUE:") }, ids }, names, sel, title)
                        });
                        let asking_cart = self.cart_ask.filter(|_| self.garage_part.is_some());
                        if let Some(sel) = asking_cart {
                            rows = [st("Yes", "YES"), st("No", "NO")].into_iter().map(|l| crate::flash_ui::MenuRow { label: l, value: String::new(), enabled: true, part: None }).collect();
                            flash_sel = sel;
                        }
                        let title = if asking_cart.is_some() { st("INSTALL_CONF_MENU", "ADD TO CART?") } else { title.to_string() };
                        let (rim_open, flash_part_template) = if asking_cart.is_some() { (false, false) } else { (rim_open, flash_part_template) };
                        let listing_page = vehicle_list.is_some();
                        let mut list_layout = None;
                        let title = match vehicle_list {
                            Some((layout, names, sel, t)) => {
                                rows = names;
                                flash_sel = sel;
                                list_layout = Some(layout);
                                t
                            }
                            None => title,
                        };
                        let (rim_open, flash_part_template) = if listing_page { (false, false) } else { (rim_open, flash_part_template) };
                        // The paint shop is one of the movie's flat panels with its own row kinds:
                        // PAINT OPTIONS / PAINT TYPE / FADE TYPE are choices, the colours swatches.
                        let paint_page = title == "PAINT SHOP" && self.garage_part.is_none() && !listing_page;
                        let rgb_of = |c: [f32; 4]| ((c[0].clamp(0.0, 1.0) * 255.0) as u32) << 16 | ((c[1].clamp(0.0, 1.0) * 255.0) as u32) << 8 | (c[2].clamp(0.0, 1.0) * 255.0) as u32;
                        let strip: Vec<u32> = PALETTE.iter().map(|c| rgb_of(*c)).collect();
                        let primary_rgb = match self.paint_hsv {
                            Some(h) => { let c = paint_hsv_to_rgb(h); rgb_of([c[0], c[1], c[2], 1.0]) }
                            None => strip[self.color_idx % strip.len()],
                        };
                        if paint_page {
                            use crate::flash_ui::{choice_list, swatch_value};
                            let open = self.paint_open.map(|o| o.0);
                            let (prow, srow) = (slot_row(Slot::Paint), fade_row + 1);
                            let mut out: Vec<crate::flash_ui::MenuRow> = Vec::new();
                            let areas: Vec<String> = PAINT_AREAS.iter().map(|a| st(a.0, a.1)).collect();
                            // (the shop's own lists, in its order: Paint.sc.xml)
                            let types: Vec<String> = [("Matte", 1u8), ("Gloss", 2), ("Metallic", 3), ("Pearl", 4), ("Sparkle", 5)].iter().map(|(k, _)| st(k, &k.to_uppercase())).collect();
                            let fades: Vec<String> = [("None", "NONE"), ("TwoTone", "TWO TONE"), ("FrontBack", "FRONT BACK"), ("LeftRight", "LEFT RIGHT"), ("TopBottom", "TOP BOTTOM")].iter().map(|(k, d)| st(k, d)).collect();
                            for r in rows.iter() {
                                let mut r = r.clone();
                                match r.label.as_str() {
                                    "PAINT OPTIONS" => {
                                        r.label = st("G_PaintOptions", "PAINT OPTIONS");
                                        r.value = choice_list(self.paint_parts as usize, &[st("P_WholeCar", "ALL PARTS"), st("P_PartsSeparately", "PARTS")]);
                                    }
                                    "PAINT AREA" => {
                                        r.label = st("G_PaintArea", "PAINT AREA");
                                        r.value = choice_list(self.paint_area.min(5), &areas);
                                    }
                                    "PAINT TYPE" => r.value = choice_list((self.paint_type.clamp(1, 5) - 1) as usize, &types),
                                    "FADE TYPE" => r.value = choice_list(self.paint_fade.min(4) as usize, &fades),
                                    "PRIMARY COLOR" => r.value = swatch_value(primary_rgb, if open == Some(prow) { (self.color_idx % strip.len()) as i32 } else { -1 }, &strip),
                                    "SECONDARY COLOR" => {
                                        // (the second colour is listed with a fade only)
                                        if self.paint_fade == 0 {
                                            continue;
                                        }
                                        r.enabled = true;
                                        r.value = swatch_value(strip[self.paint_b % strip.len()], if open == Some(srow) { (self.paint_b % strip.len()) as i32 } else { -1 }, &strip);
                                    }
                                    _ => {}
                                }
                                out.push(r);
                            }
                            rows = out;
                        }
                        let hyd_page = title == "HYDRAULICS/AIR BAGS" && self.garage_part.is_none() && !listing_page;
                        if hyd_page {
                            use crate::flash_ui::choice_list;
                            let onoff = [st("G_Off", "OFF"), st("G_On", "ON")];
                            let patterns: Vec<String> = [("G_patternTogether", "ALL"), ("G_patternFrontBack", "FRONT TO BACK"), ("G_patternBackFront", "BACK TO FRONT"), ("G_patternLeftRight", "LEFT TO RIGHT"), ("G_patternRightLeft", "RIGHT TO LEFT")].iter().map(|(k, d)| st(k, d)).collect();
                            let row = |label: String, value: String| crate::flash_ui::MenuRow { label, value, enabled: true, part: None };
                            rows = vec![
                                row(st("G_HydraulicsOnOff", "HYDRAULICS"), choice_list(self.hydraulics as usize, &onoff)),
                                row(st("G_AirbagsOnOff", "AIR BAGS"), choice_list(self.car_x.airbags as usize, &onoff)),
                                row(st("G_AirbagsPattern", "LOWERING PATTERN"), choice_list(self.mc4.as_ref().map_or(0, |m| m.options.airride_pattern as usize).min(4), &patterns)),
                            ];
                        }
                        let dim_page = title == "DIMENSIONS" && self.garage_part.is_none() && !listing_page;
                        if dim_page {
                            // The car's own figures (references `a09` the R8: 18, 35, 245, STOCK; `g_wh_2`
                            // the save's GTO: 15, 80, 205, -1): rim size in inches, tire profile and
                            // width by their places in the game's lists (0x82051144: 20..85 by 5,
                            // 0x82051258: 195..355 by 10), ride height as steps from stock.
                            use crate::flash_ui::choice_list;
                            let v = self.car_dims(self.current_car);
                            let ax = if self.dim_axle == 1 { 1 } else { 0 };
                            let row = |label: String, value: String| crate::flash_ui::MenuRow { label, value, enabled: true, part: None };
                            let one = |text: String| choice_list(0, &[text]);
                            let ride = v[ax] as i32;
                            rows = vec![
                                row(st("Axle", "AXLE"), choice_list(self.dim_axle.min(2), &[st("G_FrontAxle", "FRONT"), st("G_RearAxle", "REAR"), st("G_BothAxle", "BOTH")])),
                                row(st("G_RimSize", "RIMS SIZE"), one(format!("{}", v[4 + ax]))),
                                row(st("G_TireProfile", "TIRE PROFILE"), one(format!("{}", 20 + 5 * v[6 + ax] as i32))),
                                row(st("G_TireWidth", "TIRE WIDTH"), one(format!("{}", 195 + 10 * v[2 + ax] as i32))),
                                row(st("G_RideHeight", "RIDE HEIGHT"), one(if ride == 3 { st("G_Stock", "STOCK") } else { format!("{}", 3 - ride) })),
                            ];
                        }
                        // INTERIOR COLORS (ColorsInt.sc.xml, the paint shop's flat template 42; references
                        // `g_col0`..`g_col5`): the neon switch and six colour rows, six rows shown.
                        // (approx.) the port's car has no interior colours: the choices are kept, not shown.
                        let int_page = title == "INTERIOR COLORS" && self.garage_part.is_none() && !listing_page;
                        if int_page {
                            use crate::flash_ui::{choice_list, swatch_value};
                            let row = |label: String, value: String| crate::flash_ui::MenuRow { label, value, enabled: true, part: None };
                            let labels = [("G_IntNeonColor", "NEON COLOR"), ("G_GaugesColor", "GAUGE COLOR A"), ("G_GaugesBColor", "GAUGE COLOR B"), ("G_IntAColor", "INTERIOR COLOR A"), ("G_IntBColor", "INTERIOR COLOR B"), ("G_IntCColor", "INTERIOR COLOR C")];
                            let mut out = vec![row(st("G_IntNeon", "INTERIOR NEON"), choice_list(self.int_neon as usize, &[st("G_Off", "OFF"), st("G_On", "ON")]))];
                            for (k, (key, d)) in labels.iter().enumerate() {
                                let idx = self.int_colors[k] % strip.len();
                                let rgb = self.slot_hsv[k].map_or(strip[idx], |h| { let c = paint_hsv_to_rgb(h); rgb_of([c[0], c[1], c[2], 1.0]) });
                                out.push(row(st(key, d), swatch_value(rgb, if self.int_open.map(|o| o.0) == Some(k) { idx as i32 } else { -1 }, &strip)));
                            }
                            // (INTERIOR COLOR D, E: colour slots 12, 13 of ColorsInt.sc.xml - the vehicle's own, `mc4`)
                            for (k, (key, d)) in [("G_IntDColor", "INTERIOR COLOR D"), ("G_IntEColor", "INTERIOR COLOR E")].iter().enumerate() {
                                let idx = self.car_x.int_de_idx[k].0 % strip.len();
                                let rgb = self.car_x.int_de_hsv[k].map_or(strip[idx], |h| { let c = paint_hsv_to_rgb(h); rgb_of([c[0], c[1], c[2], 1.0]) });
                                out.push(row(st(key, d), swatch_value(rgb, if self.int_open.map(|o| o.0) == Some(9 + k) { idx as i32 } else { -1 }, &strip)));
                            }
                            rows = out;
                        }
                        // MATERIALS/PAINT (RimColorMatMenu, template 42; reference `g_wh_mat`): the
                        // material of spoke A, spoke B and the lip, then their colours.
                        // (approx.) the port's rims have one finish: the choices are kept, not shown.
                        let mat_page = title == "MATERIALS/PAINT" && self.garage_part.is_none() && !listing_page;
                        if mat_page {
                            use crate::flash_ui::{choice_list, swatch_value};
                            let row = |label: String, value: String| crate::flash_ui::MenuRow { label, value, enabled: true, part: None };
                            let mats = [st("G_Painted", "PAINTED"), st("G_Chrome", "CHROME"), st("G_Brushed", "BRUSHED")];
                            let mut out = Vec::new();
                            for (k, (key, d)) in [("G_RimMat1", "SPOKE A"), ("G_RimMat2", "SPOKE B"), ("G_RimMat3", "LIP")].iter().enumerate() {
                                out.push(row(st(key, d), choice_list(self.rim_mats[k].min(2) as usize, &mats)));
                            }
                            for (k, (key, d)) in [("G_Rim1Color", "SPOKE A"), ("G_Rim2Color", "SPOKE B"), ("G_Rim3Color", "LIP")].iter().enumerate() {
                                let idx = self.rim_cols[k] % strip.len();
                                let rgb = self.slot_hsv[6 + k].map_or(strip[idx], |h| { let c = paint_hsv_to_rgb(h); rgb_of([c[0], c[1], c[2], 1.0]) });
                                out.push(row(st(key, d), swatch_value(rgb, if self.int_open.map(|o| o.0) == Some(6 + k) { idx as i32 } else { -1 }, &strip)));
                            }
                            rows = out;
                        }
                        let paint_page = paint_page || hyd_page || dim_page || int_page || mat_page;
                        // (the vinyl editor's menu is one of the flat panels too: reference `t45`)
                        let mut vinyl_flat = false;
                        let vinyl_menu = title == "VINYL EDITOR" && self.garage_part.is_none() && !listing_page;
                        if vinyl_menu {
                            let has = self.mc4_vinyl_has_layers();
                            let row = |k: &str, d: &str, on: bool| crate::flash_ui::MenuRow { label: st(k, d), value: String::new(), enabled: on, part: None };
                            vinyl_flat = true;
                            rows = vec![row("VE_VinylPackages", "VINYL PACKAGES", true), row("VE_CreateEditVinyls", "CREATE/EDIT VINYLS", true), row("VE_RemoveAllVinyls", "REMOVE ALL VINYLS", has), row("VE_SaveAsVinylPackage", "STORE VINYL PACKAGES", has), row("VE_ExitVinylEditor", "EXIT VINYL EDITOR", true)];
                            // (step 22: the STORE VINYL PACKAGES page, `VinylPresetSave`)
                            if let Some(sel) = self.car_x.vinyl_save {
                                rows = (1..=3).map(|k| row(&format!("VE_Preset{k}"), &format!("PACKAGE {k}"), true)).collect();
                                flash_sel = sel;
                            }
                        }
                        let pkg_layout = self.vinyl_pkg.map(|sel| {
                            // (the package's name from the string table where it has one: "B-BOY")
                            // (step 22: the rows of garage.CreateVinylPresetMenu, named by the string table)
                            rows = self.mc4_vinyl_rows().iter().map(|r| crate::flash_ui::MenuRow { label: self.mc4_vinyl_row_label(r), value: String::new(), enabled: true, part: None }).collect();
                            flash_sel = sel;
                            MenuLayout::VinylPackages { vehicle: mcla_formats::hash::joaat(&self.dir_name(self.current_car).to_ascii_lowercase()) as i32 }
                        });
                        let title = if pkg_layout.is_some() { "VINYL PACKAGES".to_string() } else if vinyl_menu && self.car_x.vinyl_save.is_some() { st("VE_SaveAsVinylPackage", "STORE VINYL PACKAGES") } else { title.to_string() };
                        let cart_layout = self.cart_open.map(|sel| {
                            let list: Vec<Slot> = CUSTOM_SLOTS.iter().copied().filter(|s| !self.part_is_owned(self.current_car, *s, self.cfg.get(*s))).collect();
                            let mut total = 0i64;
                            rows = list
                                .iter()
                                .map(|s| {
                                    let p = self.part_price(self.current_car, *s, self.cfg.get(*s));
                                    total += p;
                                    crate::flash_ui::MenuRow { label: s.label().to_uppercase(), value: format!("${}", p), enabled: true, part: None }
                                })
                                .collect();
                            flash_sel = sel.min(rows.len().saturating_sub(1));
                            MenuLayout::Cart { total, label: st("G_Total", "TOTAL:") }
                        });
                        let title = if cart_layout.is_some() { st("G_Shopping_Cart", "CART") } else { title };
                        // LICENSE PLATES (template 45, reference `g_lp_0`): the seven characters, the one
                        // under the cursor lit
                        let plate_layout = self.plate_edit.map(|(cur, idx)| {
                            flash_sel = cur;
                            MenuLayout::LicensePlate { chars: idx }
                        });
                        let title = if plate_layout.is_some() { st("LicensePlate", "LICENSE PLATES") } else { title };
                        let abil_page = title == "SPECIAL ABILITIES" && self.garage_part.is_none() && !listing_page;
                        if abil_page {
                            // (the port's abilities have no levels: all three stars, a full meter - approx.)
                            let names = [("ZONE", "ZONE"), ("AGRO", "AGRO"), ("ROAR", "ROAR"), ("PULSE", "EMP")];
                            rows = names.iter().map(|(k, d)| crate::flash_ui::MenuRow { label: st(k, d), value: String::new(), enabled: true, part: None }).collect();
                        }
                        let abil_layout = abil_page.then(|| MenuLayout::Abilities {
                            // (stars = the ability's level + 1, the meter from its system's amount: sub_826A6460)
                            details: ["G_ZONE_EXPLAIN", "G_AGRO_EXPLAIN", "G_ROAR_EXPLAIN", "G_PULSE_EXPLAIN"].iter().enumerate().map(|(n, k)| { let (stars, meter) = self.mc4_ability_panel(n); (self.career.strings.get(k).map(|v| v.to_string()).unwrap_or_default(), stars, meter) }).collect(),
                            level_label: st("G_Level", "LEVEL"),
                        });
                        let picker = self.paint_pick.is_some().then(|| {
                            let h = self.paint_hsv.unwrap_or([0.0, 0.0, 1.0]);
                            let c = paint_hsv_to_rgb([h[0], 1.0, 1.0]);
                            MenuLayout::ColorPicker { h: (h[0] * 100.0).round() as i32, s: (h[1] * 100.0).round() as i32, b: (h[2] * 100.0).round() as i32, hue_rgb: rgb_of([c[0], c[1], c[2], 1.0]) as i32 }
                        });
                        let title = if picker.is_some() { "COLOR PICKER".to_string() } else { title.to_string() };
                        let page = MenuPage {
                            layout: if let Some(l) = list_layout {
                                l
                            } else if let Some(l) = picker {
                                l
                            } else if let Some(l) = abil_layout {
                                l
                            } else if let Some(l) = pkg_layout {
                                l
                            } else if let Some(l) = cart_layout {
                                l
                            } else if let Some(l) = plate_layout {
                                l
                            } else if paint_page || vinyl_flat {
                                MenuLayout::Paint
                            } else if rim_open {
                                MenuLayout::GarageRims
                            } else if flash_part_template {
                                MenuLayout::GarageParts
                            } else if let Some(at) = RING.iter().rev().position(|m| *m == self.garage_menu).filter(|_| self.garage_part.is_none() && self.garage_xpart.is_none() && self.plate_edit.is_none()) {
                                // title_loop puts index+1 to the left. Keep its names in
                                // the reverse of the port's rightward navigation order.
                                MenuLayout::GarageLoop { names: RING.iter().rev().map(|m| menus[*m].0.to_string()).collect(), index: at, flat: matches!(self.garage_menu, 7 | 16) }
                            } else { MenuLayout::List },
                            icon: if self.cart_open.is_some() { std::env::var("MCLA_CART_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(12) } else if RING.contains(&self.garage_menu) && self.garage_part.is_none() && self.garage_xpart.is_none() && self.plate_edit.is_none() && !listing_page { 11 } else { 10 },
                            // the rim shop shows the maker of every brand in the tab strip,
                            // and the logo at tabs[cur_tab-1] + 1
                            tabs: if rim_open { rim_tabs.iter().map(|b| b.mfr.to_string()).collect() } else { Vec::new() },
                            cur_tab: if rim_open { self.rim_brand } else { 0 },
                            // (the movie widens its panel to the title: neighbours left out)
                            title: title.to_string(),
                            rows,
                            select: flash_sel,
                            // (the full-screen vehicle list carries its prompts and the money inside the
                            // panel; elsewhere the bar is the prompt movie's along the bottom)
                            prompts: if listing_page {
                                use crate::flash_ui::ICON_X;
                                let showroom = self.garage_list.map_or(false, |l| l.0);
                                let mut v = vec![(accept.clone(), ICON_A), (st("prompt_back", "BACK"), ICON_B)];
                                v.push(if showroom { (st("G_TestDrive", "TEST DRIVE"), ICON_X) } else { (st("G_SellRide", "SELL RIDE"), ICON_X) });
                                if showroom {
                                    v.push((st("G_PurchaseTradeIn", "PURCHASE WITH TRADE-IN"), crate::flash_ui::ICON_Y));
                                }
                                v
                            } else {
                                Vec::new()
                            },
                            money: listing_page.then(|| (st("G_Money", "MONEY:"), self.money)),
                            // rim names run from "BBS CH" to "KONIG FF GRAPHITE"; pad them
                            // to the shop's longest so the movie measures one width and the panel keeps its size
                            pad_labels: if rim_open { rim_label_pad } else { 0 },
                            tab_icons: Vec::new(),
                        row_props: Vec::new(),
                            description: String::new(),
                            list_max: 0,
                        };
                        let pictures = std::path::Path::new(&self.data_dir).join("resources/ui/textures/garage/dyntextures");
                        // (the highlighted vehicle's, whether or not its model has arrived yet)
                        let shown_car = self.garage_list.and_then(|(_, sel)| list_cars.get(sel).copied()).unwrap_or(self.current_car);
                        let picture = pictures.join("managerides").join(format!("mr_{}.r9", self.dir_name(shown_car)));
                        let big_picture = pictures.join("showroom").join(format!("sh_{}.r9", self.dir_name(shown_car)));
                        let list_dirs: Vec<String> = list_cars.iter().map(|&i| self.dir_name(i)).collect();
                        let fg = self.flash_garage.as_mut().unwrap();
                        fg.show(&page);
                        // The vehicle's picture: the movie's `sh_dyntexture` clip names a texture
                        // (`__texturename` = `mr_bigpicture`) that the game points at
                        // `ui/garage/dyntextures/manageRides/mr_<car>.dds` - the car with its
                        // maker's logo. Without the file the stand-in is hidden.
                        if self.garage_list.map_or(false, |l| l.0) {
                            // the showroom: the large picture (`showroom/sh_<car>`) and the six of
                            // the strip (`mini_showroom/msh_<car>`), which the movie scrolls itself
                            fg.movie.swap_texture(ctx, "screen.menu.showroom_panel.sh_dyntexture", 2, &big_picture);
                            let first = fg.movie.player.get_global("ScrollOffset").map_or(0.0, |v| v.to_f32()).max(0.0) as usize;
                            for k in 0..6usize {
                                let file = pictures.join("mini_showroom").join(format!("msh_{}.r9", list_dirs.get(first + k).cloned().unwrap_or_else(|| "default".into())));
                                fg.movie.swap_texture(ctx, &format!("screen.menu.showroom_panel.car_list_rt.slot{k}"), 10 + k as u32, &file);
                            }
                        }
                        if self.garage_list.is_some() {
                            let shown = fg.movie.swap_texture(ctx, "screen.menu.manageride_layer.sh_dyntexture", 1, &picture);
                            if let Some(id) = fg.movie.player.find("screen.menu.manageride_layer.sh_dyntexture") {
                                if let Some(i) = fg.movie.player.inst_mut(id) {
                                    i.tint[3] = if shown { 255 } else { 0 };
                                }
                            }
                        }
                        // where this view hangs the menu (0x826B32E0)
                        let scene_view = match (self.garage_panel, hud.cam_vp) {
                            (Some((rot, off, scale, frame)), Some(vp)) if std::env::var("MCLA_FLAT_MENU").is_err() => {
                                use mcla_flash::value::Value;
                                let p = &mut fg.movie.player;
                                p.set_global("cur_menuRotation", Value::Float(rot));
                                for (i, v) in [off.x, off.y, off.z].into_iter().enumerate() {
                                    p.set_path(&format!("cur_MenuPosition.{i}"), Value::Float(v));
                                }
                                p.set_path("cur_Menuscale.0", Value::Float(scale[0]));
                                p.set_path("cur_Menuscale.1", Value::Float(scale[1]));
                                Some(crate::menu3d::View { vp, frame })
                            }
                            _ => None,
                        };
                        fg.movie.player.update(ctx.input(|i| i.stable_dt).min(0.1));
                        // The rim shop's box is measured from its content, and what it
                        // measures - the brand logo, the price column - is a different
                        // size on every brand, so the panel jumps as the tabs are stepped.
                        // The original hides this by rendering the menu into one fixed
                        // panel; here the box the movie laid out is scaled about its left
                        // edge to a single width instead.
                        // reset first: the scale is not something the movie clears, so measuring with
                        // last frame's scale still applied would just re-derive it
                        // The rim shop's box is measured from its content, and what it measures -
                        // the brand logo, the price column - is a different size on every
                        // brand, so the panel jumps as the tabs are stepped. The original
                        // hides this by rendering the menu into one fixed panel; here the
                        // box the movie laid out is scaled about its left edge to a single
                        // width instead. The scale is reset first: the movie never clears
                        // `_xscale` itself, so measuring with the previous frame's scale
                        // still applied would just re-derive it.
                        if rim_open && scene_view.is_none() {
                            fg.movie.scale_sprite_x("screen.menu", 1.0);
                            let box_w = fg.movie.sprite_rect("screen.menu.backgound").map_or(0.0, |r| r.width());
                            fg.movie.scale_sprite_x("screen.menu", if box_w > 1.0 { RIM_PANEL_W / box_w } else { 1.0 });
                        }
                        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_garage"));
                        ctx.move_to_top(layer);
                        let lp = ctx.layer_painter(layer);
                        if std::env::var("MCLA_MENU3D_LOG").is_ok() {
                            let p = &mut fg.movie.player;
                            let names = ["menu.name", "menu.count", "menu.type", "menu.iconID", "template", "current_template", "cur_width", "cur_height", "TransitionLevel", "current_TransitionLevel", "transition_isplaying", "title_alpha", "view2d", "cur_alpha", "type", "tabs_count"];
                            let line: Vec<String> = names.iter().map(|n| format!("{}={:?}", n, p.get_path(n))).collect();
                            log::info!("menu3d: garage {}", line.join(" "));
                        }
                        if let (Ok(_), Some(v)) = (std::env::var("MCLA_MENU3D_AXES"), scene_view) {
                            // debug: the car frame's origin (white), +x 1 m (red), -z 2 m (green), +y 1 m (blue)
                            for (p, c) in [(Vec3::ZERO, egui::Color32::WHITE), (Vec3::X, egui::Color32::RED), (Vec3::new(0.0, 0.0, -2.0), egui::Color32::GREEN), (Vec3::Y, egui::Color32::BLUE)] {
                                if let Some(at) = crate::menu3d::to_screen(v.vp * v.frame * p.extend(1.0), ctx.screen_rect()) {
                                    lp.circle_filled(at, 7.0, c);
                                }
                            }
                        }
                        // the menu hangs in the garage on its panel mesh, inside the glass frame
                        // mesh (see menu3d); the prompt bar is its own movie along the bottom
                        let scene = scene_view.and_then(|v| crate::menu3d::read_scene(&mut fg.movie.player).map(|s| (s, v)));
                        match (scene, self.ui_meshes.as_mut()) {
                            (Some((scene, view)), Some(meshes)) => {
                                crate::menu3d::paint_scene(ctx, &lp, meshes, &mut fg.movie, &scene, Some(view), listing_page);
                                flash_3d = true;
                            }
                            _ => {
                                fg.movie.paint(ctx, &lp);
                                // (flat fallback: the movie paints only the fill, so the outline is stroked
                                // over the box it laid out for itself)
                                flash_box = fg.movie.sprite_rect_on("screen.menu.backgound", screen);
                                if let Some(r) = flash_box {
                                    lp.rect_stroke(r, 10.0 * sc, egui::Stroke::new(2.5 * sc, egui::Color32::from_rgb(200, 206, 206)), egui::StrokeKind::Inside);
                                }
                            }
                        }
                        // what the parts on the car that are not the player's yet come to
                        let cart_cost: i64 = if car_changed { 0 } else { CUSTOM_SLOTS.iter().map(|slot| { let v = self.cfg.get(*slot); if self.part_is_owned(self.current_car, *slot, v) { 0 } else { self.part_price(self.current_car, *slot, v) } }).sum() };
                        // (the cart's own page carries the total)
                        let cart_cost = (cart_cost > 0 && self.cart_open.is_none()).then_some(cart_cost);
                        if listing_page {
                            if let Some(fp) = self.flash_prompt.as_mut() {
                                fp.hide();
                                fp.movie.player.update(ctx.input(|i| i.stable_dt).min(0.1));
                            }
                        } else if let (true, Some(fp)) = (flash_3d, self.flash_prompt.as_mut()) {
                            use crate::flash_ui::{ICON_A, ICON_B, ICON_X};
                            let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).unwrap_or_else(|| d.to_string());
                            let accept = if car_changed && !owned_sel { st("prompt_purchase", "PURCHASE") } else { st("prompt_accept", "ACCEPT") };
                            let mut prompts = vec![(accept, ICON_A), (st("prompt_back", "BACK"), ICON_B)];
                            // (step 22b: prompts 146 / 147 of the vinyl packages' pages - sub_8220F040 cases
                            // 146 VE_SaveAsPreset (A), 147 VE_ApplyPreset (A), both with prompt_back (B))
                            if self.car_x.vinyl_save.is_some() {
                                prompts = vec![(st("VE_SaveAsPreset", "SAVE"), ICON_A), (st("prompt_back", "BACK"), ICON_B)];
                            } else if self.vinyl_pkg.is_some() {
                                prompts = vec![(st("VE_ApplyPreset", "APPLY"), ICON_A), (st("prompt_back", "BACK"), ICON_B)];
                            }
                            if rim_open {
                                prompts.push(("STOCK RIMS".to_string(), ICON_X));
                            }
                            // (the brake list has no stock row: STOCK BRAKES on X - reference `g_wh_brake`)
                            if self.garage_xpart.map_or(false, |(k, _)| XPARTS[k.min(XPARTS.len() - 1)].1 == "brakes") {
                                prompts.push((st("G_DefaultBrake", "STOCK BRAKES"), ICON_X));
                            }
                            // (the hood list: CARBON FIBER ON/OFF on X - reference `c09`; `garage.SwapCarbonFiber()`)
                            if self.garage_part.map_or(false, |row| CUSTOM_SLOTS.get(row.wrapping_sub(1)) == Some(&Slot::Hood)) {
                                prompts.push((st("G_SwapCarbonFiber", "CARBON FIBER ON/OFF"), ICON_X));
                                if ctx.input(|i| i.key_pressed(egui::Key::X)) {
                                    // (taken up by `mc4_misc_idle`, outside this panel's borrow)
                                    self.car_x.swap_cf = true;
                                }
                            }
                            // (the paint shop: BACK alone; on a colour CHANGE COLOR, on its strip
                            // ACCEPT / EDIT / RESET COLOR SLOT, in the picker CHANGE COLOR on the stick)
                            if self.vinyl_pkg.is_some() {
                                prompts = vec![(st("prompt_apply", "APPLY"), ICON_A), (st("prompt_back", "BACK"), ICON_B)];
                            }
                            if self.cart_open.is_some() {
                                prompts = vec![(st("prompt_back", "BACK"), ICON_B), (st("G_Pay_And_Checkout", "PAY AND CHECK OUT"), ICON_A), (st("G_Remove_Item", "REMOVE ITEM"), ICON_X), (st("G_Remove_All_Items", "REMOVE ALL ITEMS"), crate::flash_ui::ICON_Y)];
                            }
                            if self.pick_slot.is_some() && self.paint_pick.is_some() {
                                prompts = vec![(st("prompt_back", "BACK"), ICON_B), (st("G_AcceptColor", "ACCEPT COLOR"), ICON_A), (st("G_ChangeColor", "CHANGE COLOR"), 11)];
                            }
                            if self.garage_menu == 12 && self.garage_part.is_none() {
                                // (DIMENSIONS has BACK alone: reference `g_wh_2`)
                                prompts = vec![(st("prompt_back", "BACK"), ICON_B)];
                            }
                            if self.plate_edit.is_some() {
                                // (reference `g_lp_0`: CHANGE STATE LB RB, BACK, ACCEPT)
                                prompts = vec![(st("prompt_accept", "ACCEPT"), ICON_A), (st("prompt_back", "BACK"), ICON_B), (st("G_ChangeState", "CHANGE STATE"), crate::flash_ui::ICON_LB_RB)];
                            }
                            if self.garage_menu == 18 && self.garage_part.is_none() && self.paint_pick.is_none() {
                                let back = (st("prompt_back", "BACK"), ICON_B);
                                prompts = if self.int_open.is_some() {
                                    vec![back, (st("VE_accept_color", "ACCEPT COLOR"), ICON_A), (st("VE_edit_color", "EDIT COLOR SLOT"), ICON_X), (st("G_ResetColorSlot", "RESET COLOR SLOT"), crate::flash_ui::ICON_Y)]
                                } else if (9933..=9935).contains(&self.garage_row) {
                                    vec![back, (st("VE_change_color", "CHANGE COLOR"), ICON_A)]
                                } else {
                                    vec![back]
                                };
                            }
                            if self.garage_menu == 17 && self.garage_part.is_none() && self.paint_pick.is_none() {
                                // (references `g_col1`, `g_col2`)
                                let back = (st("prompt_back", "BACK"), ICON_B);
                                prompts = if self.int_open.is_some() {
                                    vec![back, (st("VE_accept_color", "ACCEPT COLOR"), ICON_A), (st("VE_edit_color", "EDIT COLOR SLOT"), ICON_X), (st("G_ResetColorSlot", "RESET COLOR SLOT"), crate::flash_ui::ICON_Y)]
                                } else if (9941..=9948).contains(&self.garage_row) {
                                    vec![back, (st("VE_change_color", "CHANGE COLOR"), ICON_A)]
                                } else {
                                    vec![back]
                                };
                            }
                            if self.garage_menu == 3 && self.garage_part.is_none() {
                                let back = (st("prompt_back", "BACK"), ICON_B);
                                prompts = if self.paint_pick.is_some() {
                                    vec![back, (st("G_AcceptColor", "ACCEPT COLOR"), ICON_A), (st("G_ChangeColor", "CHANGE COLOR"), 11)]
                                } else if self.paint_open.is_some() {
                                    vec![back, (st("G_AcceptColor", "ACCEPT COLOR"), ICON_A), (st("G_EditColorSlot", "EDIT COLOR SLOT"), ICON_X), (st("G_ResetColorSlot", "RESET COLOR SLOT"), crate::flash_ui::ICON_Y)]
                                } else if colour_row {
                                    vec![back, (st("G_ChangeColor", "CHANGE COLOR"), ICON_A)]
                                } else {
                                    vec![back]
                                };
                            }
                            fp.show(&prompts, Some(self.money), cart_cost);
                            fp.movie.player.update(ctx.input(|i| i.stable_dt).min(0.1));
                            fp.movie.paint(ctx, &lp);
                        }
                    }
                    // paint swatches (in the paint shop)
                    let sy = panel.bottom() + 12.0 * sc;
                    if title == "PAINT SHOP" && self.flash_garage.is_none() {
                        for (i, c) in PALETTE.iter().enumerate() {
                            let colr = egui::Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
                            let rect = egui::Rect::from_min_size(egui::pos2(x0 + 10.0 * sc + i as f32 * 32.0 * sc, sy), egui::vec2(26.0 * sc, 26.0 * sc));
                            painter.rect_filled(rect, 0.0, colr);
                            if i == color_idx {
                                painter.rect_stroke(rect, 0.0, egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Outside);
                            }
                            if pointer.1 && pointer.0.map_or(false, |p| rect.contains(p)) {
                                clicked_color = Some(i);
                            }
                        }
                    }
                    // showroom card of the selected vehicle (original catalogue data)
                    let owned = self.is_owned(self.selected_car);
                    let (price, need_rep) = self.price_of(self.selected_car);
                    let info = self.catalog.get(&self.dir_name(self.selected_car)).cloned();
                    // (the original shows a vehicle's figures in the showroom only)
                    if (!flash_3d || car_changed) && self.garage_list.is_none() {
                        let card = egui::Rect::from_min_size(egui::pos2(screen.left() + 60.0 * sc, screen.bottom() - 330.0 * sc), egui::vec2(430.0 * sc, 250.0 * sc));
                        self.panel(&painter, card, sc);
                        let cx = card.left() + 24.0 * sc;
                        let status = if owned {
                            ("IN YOUR GARAGE".to_string(), egui::Color32::from_rgb(120, 230, 120))
                        } else if self.rep < need_rep {
                            (format!("LOCKED  {} REP", need_rep), egui::Color32::from_rgb(255, 90, 70))
                        } else {
                            (format!("PRICE ${}", price), if self.money >= price { egui::Color32::WHITE } else { egui::Color32::from_rgb(255, 90, 70) })
                        };
                        self.text(&painter, egui::pos2(cx, card.top() + 22.0 * sc), egui::Align2::LEFT_TOP, &status.0, 16.0 * sc, status.1);
                        if let Some(ci) = info.as_ref() {
                            self.text(&painter, egui::pos2(cx, card.top() + 56.0 * sc), egui::Align2::LEFT_TOP, &format!("{}  CLASS {}", ci.class.to_uppercase(), ci.group.chars().next().unwrap_or('D')), 11.0 * sc, egui::Color32::from_gray(190));
                            let bars = [("TOP SPEED", (ci.base_speed - 100.0) / 160.0, (ci.mods_speed - 100.0) / 160.0), ("ACCELERATION", ci.base_accel / 0.75, ci.mods_accel / 0.75), ("HANDLING", ci.base_handling / 0.92, ci.mods_handling / 0.92)];
                            for (k, (label, base, mods)) in bars.iter().enumerate() {
                                let by = card.top() + (96.0 + 46.0 * k as f32) * sc;
                                self.text(&painter, egui::pos2(cx, by), egui::Align2::LEFT_TOP, label, 10.0 * sc, egui::Color32::from_gray(170));
                                let bw = card.width() - 48.0 * sc;
                                let br = egui::Rect::from_min_size(egui::pos2(cx, by + 18.0 * sc), egui::vec2(bw, 9.0 * sc));
                                painter.rect_filled(br, 0.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30));
                                let mut m = br;
                                m.set_width(bw * mods.clamp(0.0, 1.0));
                                painter.rect_filled(m, 0.0, egui::Color32::from_rgba_unmultiplied(255, 140, 0, 90));
                                let mut b = br;
                                b.set_width(bw * base.clamp(0.0, 1.0));
                                painter.rect_filled(b, 0.0, accent);
                            }
                            self.text(&painter, egui::pos2(card.right() - 24.0 * sc, card.top() + 56.0 * sc), egui::Align2::RIGHT_TOP, &format!("{:.0} MPH", ci.base_speed), 11.0 * sc, egui::Color32::from_gray(190));
                        }
                    }
                    // the prompt bar along the bottom: what the buttons do, and the money
                    let hint = if car_changed && !owned { "ACCEPT  BUY THIS VEHICLE" } else if car_changed { "ACCEPT  LOAD THIS VEHICLE" } else if self.garage_stack.is_empty() { "BACK  EXIT      ACCEPT" } else { "BACK      ACCEPT" };
                    // With the garage movie up it is told there are no prompts, and the bar
                    // is drawn here instead: the original runs it along the bottom of the
                    // screen, the movie draws it hard under the menu box.
                    let bar = match flash_box {
                        // (under the menu box, and the same width, so the money has room)
                        Some(b) => egui::Rect::from_min_max(egui::pos2(b.left(), screen.bottom() - 92.0 * sc), egui::pos2(b.right(), screen.bottom() - 60.0 * sc)),
                        None => egui::Rect::from_min_max(egui::pos2(screen.right() - 640.0 * sc, screen.bottom() - 92.0 * sc), egui::pos2(screen.right() - 70.0 * sc, screen.bottom() - 60.0 * sc)),
                    };
                    let painter = if flash_3d { painter.with_clip_rect(egui::Rect::NOTHING) } else { painter.clone() };
                    painter.rect_filled(bar, 4.0 * sc, egui::Color32::from_rgba_unmultiplied(8, 12, 14, 228));
                    let hint = if self.flash_garage.is_some() {
                        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).unwrap_or_else(|| d.to_string());
                        // the original reads label then button: "STOCK RIMS X  BACK B  ACCEPT A"
                        let mut parts: Vec<(String, &str)> = Vec::new();
                        if rim_open {
                            parts.push(("STOCK RIMS".to_string(), "X"));
                        }
                        parts.push((st("prompt_back", "BACK"), "B"));
                        let accept = if car_changed && !self.is_owned(self.selected_car) { st("prompt_purchase", "PURCHASE") } else { st("prompt_accept", "ACCEPT") };
                        parts.push((accept, "A"));
                        let mut x = bar.left() + 16.0 * sc;
                        for (label, key) in parts {
                            let t = self.text(&painter, egui::pos2(x, bar.center().y), egui::Align2::LEFT_CENTER, &label, 11.0 * sc, egui::Color32::from_gray(225));
                            x = t.right() + 5.0 * sc;
                            let g = self.text(&painter, egui::pos2(x, bar.center().y), egui::Align2::LEFT_CENTER, key, 10.0 * sc, egui::Color32::from_rgb(255, 205, 50));
                            x = g.right() + 14.0 * sc;
                        }
                        String::new()
                    } else {
                        hint.to_string()
                    };
                    if !hint.is_empty() {
                        self.text(&painter, egui::pos2(bar.left() + 16.0 * sc, bar.center().y), egui::Align2::LEFT_CENTER, &hint, 11.0 * sc, egui::Color32::from_gray(225));
                    }
                    let money = if self.flash_garage.is_some() { format!("MONEY: ${}", self.money) } else { format!("REP {}      MONEY: ${}", self.rep, self.money) };
                    self.text(&painter, egui::pos2(bar.right() - 16.0 * sc, bar.center().y), egui::Align2::RIGHT_CENTER, &money, 11.0 * sc, egui::Color32::from_rgb(255, 205, 50));
                }
                if let Some(c) = clicked_color {
                    self.color_idx = c;
                    self.paint_hsv = None;
                    self.dirty = true;
                }
                if enter && !car_changed && self.garage_row == perf_row {
                    self.perf_shop = Some(0);
                } else if (enter && !car_changed && self.garage_row == vinyl_row && self.mc4_misc_check(1)) || std::mem::take(&mut self.vinyl_open_req) {
                    // the editor starts from what the car wears: its own livery or a preset
                    let car = self.vinyl_for.clone();
                    if let Some(p) = self.vinyls.get(&car).and_then(|v| v.strip_prefix("preset:").map(|p| p.to_string())) {
                        self.liveries.insert(car.clone(), self.vinyl_presets.livery(&p));
                        self.livery_rev += 1;
                        self.vinyls.insert(car, "custom".to_string());
                        if !self.vinyl_list.iter().any(|v| v == "custom") {
                            self.vinyl_list.insert(0, "custom".to_string());
                        }
                    }
                    self.vinyl_edit = Some(VinylEdit::default());
                } else if (enter && (self.flash_garage.is_none() || listing || car_changed) || std::mem::take(&mut self.buy_confirmed)) && self.try_buy(self.selected_car) && (car_changed || self.try_buy_parts()) {
                    // (with the garage movie ACCEPT on a row of settings is not "take the car and
                    // leave": only a vehicle list's ACCEPT is - found by the scripted walk-through)
                    action = MenuAction::ApplyCar;
                    // (the list closes on the vehicle taken: the top menu again, as in the recomp -
                    // `g_mv2`)
                    self.garage_list = None;
                }
                if back {
                    action = MenuAction::Back;
                }
                if std::mem::take(&mut self.test_drive_req) {
                    action = MenuAction::TestDrive;
                }
            }
            Mode::Results if self.replay.is_some() => self.replay_ui(ctx, up, down, left, right, enter, back),
            Mode::Results => {
                let sc = (screen.height() / 900.0).clamp(0.6, 2.0);
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("results")));
                let rows = self.last_results.len().max(1);
                let w = 820.0 * sc;
                let row_h = 40.0 * sc;
                // the race-over entries of the original (`RO_*` in PauseMenu.sc.xml), in its words
                let s = |key: &str, fallback: &str| -> String { self.career.strings.get(key).filter(|v| !v.contains('<')).map(|v| v.to_string()).unwrap_or_else(|| fallback.to_string()).to_uppercase() };
                let mut ro: Vec<(String, u8)> = Vec::new();
                // (RO_FullFix, shown as QUICK FIX: only for a car that needs it)
                let fix = vehicles.player().map_or(0, |c| self.fix_price(c.state.damage.max(c.state.zone_mean()), c.state.zones.iter().any(|z| *z > 0.05)));
                if fix > 0 && !self.quick_fix {
                    ro.push((format!("{}   ${}", s("quick_fix", "QUICK FIX"), fix), 3));
                }
                // (reference `y_over1`, an ARCADE race: NEXT RACE, RETURN TO CRUISE, RACE AGAIN,
                // RACE RESULTS, ADD RACE TO FAVORITES, VIEW REPLAY)
                let arcade_over = self.arcade_restore.is_some() && self.flash_menu.is_some();
                if self.next_event_race.is_some() {
                    ro.push((s("pause_next_series", "NEXT RACE IN SERIES"), 0));
                } else if arcade_over {
                    ro.push((s("pause_next_race", "NEXT RACE"), 10));
                }
                // (step 19d: a local hookman's race - RO_NextRace while `bNextRace` stands)
                if self.mc4_next_race_on() && self.next_event_race.is_none() && !arcade_over {
                    ro.push((s("pause_next_race", "NEXT RACE"), 20));
                }
                ro.push((s("pause_return_cruise", "RETURN TO CRUISE"), 2));
                if self.next_event_race.is_none() {
                    ro.push((s("pause_race_again", "RACE AGAIN"), 1));
                }
                // (RO_RaceBack after a red light race - the registry's raceBack: althk.rs)
                if self.alt_race_back_offered() {
                    ro.push((s("pause_race_back", "RACE BACK"), 21));
                }
                // (with the movie the race-over menu is the pause panel's - `RaceOverTab` of
                // PauseMenu.sc.xml - and the results are a page behind RACE RESULTS)
                let movie = self.flash_menu.is_some() && std::env::var("MCLA_OLD_RESULTS").is_err();
                if movie {
                    ro.push((s("pause_race_results", "RACE RESULTS"), 9));
                }
                if arcade_over {
                    let fav = self.current_race.and_then(|i| self.race_defs.get(i)).map_or(false, |d| self.race_favs.iter().any(|f| f.eq_ignore_ascii_case(&d.id)));
                    ro.push((if fav { s("pause_remove_favorites", "REMOVE RACE FROM FAVORITES") } else { s("pause_add_favorites", "ADD RACE TO FAVORITES") }, 11));
                    ro.push((s("pause_view_replay", "VIEW REPLAY"), 12));
                    // (the seventh row, under the list's fold: reference `y_over_garage`)
                    ro.push((s("pause_go_to_garage", "GO TO GARAGE"), 13));
                }
                // (a series' race-over menu - mcSeriesMenu, sub_826639B8 / sub_82664090; `sr_raceover`:
                // NEXT RACE IN SERIES, QUIT SERIES, RESTART SERIES, SERIES STANDINGS, ADD RACE TO
                // FAVORITES, VIEW REPLAY, GO TO GARAGE. Once won or lost - bComplete / bFailure - no
                // NEXT row: RETURN TO CRUISE, RACE SERIES AGAIN, ...; the walkthrough at 0:32:10)
                let decided = self.series_decided();
                if movie && self.series.is_some() && (self.next_event_race.is_some() || decided) {
                    let fav = self.current_race.and_then(|i| self.race_defs.get(i)).map_or(false, |d| self.race_favs.iter().any(|f| f.eq_ignore_ascii_case(&d.id)));
                    ro = if decided {
                        vec![(s("pause_continue_to_cruise", "RETURN TO CRUISE"), 2), (s("pause_race_series_again", "RACE SERIES AGAIN"), 14)]
                    } else {
                        vec![(s("pause_next_series", "NEXT RACE IN SERIES"), 0), (s("pause_quit_series", "QUIT SERIES"), 2), (s("pause_restart_series", "RESTART SERIES"), 14)]
                    };
                    ro.extend([
                        (s("pause_series_standings", "SERIES STANDINGS"), 9),
                        (if fav { s("pause_remove_favorites", "REMOVE RACE FROM FAVORITES") } else { s("pause_add_favorites", "ADD RACE TO FAVORITES") }, 11),
                        (s("pause_view_replay", "VIEW REPLAY"), 12),
                        (s("pause_go_to_garage", "GO TO GARAGE"), 13),
                    ]);
                }
                // (the place splash stands by itself for a moment)
                let splash_on = match self.place_splash {
                    Some((p, left)) if movie && left > 0.0 => {
                        self.place_splash = Some((p, left - ctx.input(|i| i.stable_dt).min(0.1)));
                        true
                    }
                    _ => {
                        self.place_splash = None;
                        false
                    }
                };
                // (step 19d: form 56's banner stands for its 3.5 s by itself - `mc4_banner_page`)
                let splash_on = splash_on || (movie && self.opt_x.arcade.d.banner.is_some());
                // (the REWARDS panel first: CONTINUE takes it away)
                let (up, down, enter, back) = if splash_on {
                    (false, false, false, false)
                } else if let (Some(cur), true) = (self.series_ask, movie) {
                    // CONTINUE SERIES: repair first? YES / NO (`sr_continue`); either goes on
                    let cur = if up { 0 } else if down { 1 } else { cur };
                    self.series_ask = Some(cur);
                    if enter {
                        if cur == 0 && self.money >= fix.max(125) {
                            self.money -= fix.max(125);
                            self.quick_fix = true;
                            self.dirty = true;
                        }
                        self.series_ask = None;
                        self.menu_sel = 0;
                        match self.series_ask_kind {
                            // QUIT SERIES: back to the cruise
                            1 => action = MenuAction::StartFreeRoam,
                            // RACE BACK (Dialog_Title_RaceBack): the race back (althk.rs)
                            3 => self.althk.back_req = true,
                            // RETURN TO CRUISE of a decided series
                            4 => action = MenuAction::StartFreeRoam,
                            // RESTART SERIES / RACE SERIES AGAIN: from its first race, every tally at
                            // nought (OrderedSeriesCore `restart`, 0x247c)
                            2 | 5 => {
                                if let Some(sr) = self.series.as_mut() {
                                    sr.at = 0;
                                    sr.wins.clear();
                                    self.current_race = sr.races.first().copied();
                                    self.next_event_race = None;
                                    action = MenuAction::StartRace;
                                }
                            }
                            _ => {
                                if let Some(n) = self.next_event_race.take() {
                                    self.current_race = Some(n);
                                    action = MenuAction::StartRace;
                                }
                            }
                        }
                    } else if back {
                        self.series_ask = None;
                    }
                    (false, false, false, false)
                } else if self.series_first && movie {
                    if enter && std::env::var("MCLA_REPLAY_TRACE").is_ok() {
                        log::info!("series: results dismissed (reward {} replay {})", self.reward_panel, self.replay.is_some());
                    }
                    // (a series race's RACE RESULTS come first: CONTINUE)
                    self.results_table = !enter;
                    self.series_first = !enter;
                    (false, false, false, false)
                // (a time trial's results come before its rewards: the end of race sequence's
                // index 5, then 2 - generic_timetrial_hookman.sco 0x2984, 0x2b08)
                } else if self.reward_panel && movie && !self.mc4_tt_panel_first() {
                    // (CONTINUE: the next page of the form while there is one - sub_826764E8)
                    // (test hook MCLA_RO_CONTINUE: `mc4_continue_hook`)
                    if enter || self.mc4_continue_hook() {
                        let total = self.mc4_reward_pages().len() + (self.last_reward.0 > 0 || self.last_reward.1 > 0) as usize;
                        if self.career.reward_page + 1 < total {
                            self.career.reward_page += 1;
                        } else {
                            self.career.reward_page = 0;
                            self.reward_panel = false;
                        }
                    }
                    (false, false, false, false)
                } else if !self.mission_results.is_empty() && movie {
                    // (then MISSION RESULTS, a panel for each mission the event finished)
                    if enter {
                        self.mission_results.remove(0);
                    }
                    (false, false, false, false)
                } else {
                    (up, down, enter, back)
                };
                // After a career race on the road the game goes straight back to the cruise
                // (`ng_after_0`): the race-over menu is ARCADE's and the events'.
                if movie && !arcade_over && self.next_event_race.is_none() && self.career_over && !self.reward_panel && self.mission_results.is_empty() && self.replay.is_none() {
                    self.career_over = false;
                    self.menu_sel = 0;
                    action = MenuAction::StartFreeRoam;
                }
                if std::env::var("MCLA_RESULTS_TABLE").is_ok() {
                    self.results_table = true;
                }
                let (up, down, enter, back) = if self.results_table {
                    if enter || back {
                        self.results_table = false;
                    }
                    (false, false, false, false)
                } else {
                    (up, down, enter, back)
                };
                if up && self.menu_sel > 0 {
                    self.menu_sel -= 1;
                }
                if down && self.menu_sel + 1 < ro.len() {
                    self.menu_sel += 1;
                }
                let ro_sel = self.menu_sel.min(ro.len() - 1);
                let title_rr = s("pause_race_results", "RACE RESULTS");
                let painter = if movie && self.results_movie(ctx, &ro, ro_sel) { painter.with_clip_rect(egui::Rect::NOTHING) } else { painter };
                let h = 250.0 * sc + row_h * (rows + ro.len()) as f32;
                let panel = egui::Rect::from_center_size(screen.center(), egui::vec2(w, h));
                self.panel(&painter, panel, sc);
                let tx = panel.left() + 40.0 * sc;
                let title = if self.results_plain { "RESULTS".to_string() } else { title_rr.clone() };
                self.text(&painter, egui::pos2(tx, panel.top() + 32.0 * sc), egui::Align2::LEFT_TOP, &title, 28.0 * sc, egui::Color32::WHITE);
                // the game's own result stamps: COMPLETE for a win, FAIL otherwise
                let won = self.last_results.first().map_or(false, |r| r.0.starts_with("YOU"));
                if let Some(t) = self.art.as_ref().and_then(|a| a.get(if won { "pause_0139" } else { "pause_0127" })) {
                    let sz = t.size_vec2();
                    let hgt = 70.0 * sc;
                    let r = egui::Rect::from_min_size(egui::pos2(panel.right() - 40.0 * sc - hgt * sz.x / sz.y, panel.top() + 22.0 * sc), egui::vec2(hgt * sz.x / sz.y, hgt));
                    crate::hud::image(&painter, t, r, if won { egui::Color32::from_rgb(120, 255, 120) } else { egui::Color32::from_rgb(255, 90, 70) });
                }
                let list_y = panel.top() + 120.0 * sc;
                for (i, (name, t)) in self.last_results.iter().enumerate() {
                    let cy = list_y + row_h * (i as f32 + 0.5);
                    let col = if name.starts_with("YOU") { accent } else { egui::Color32::WHITE };
                    self.text(&painter, egui::pos2(tx, cy), egui::Align2::LEFT_CENTER, &format!("{}", i + 1), 16.0 * sc, col);
                    self.text(&painter, egui::pos2(tx + 60.0 * sc, cy), egui::Align2::LEFT_CENTER, &name.to_uppercase(), 14.0 * sc, col);
                    let value = if self.results_plain { t.map(|v| format!("{:.0}", v)).unwrap_or_default() } else { t.map(fmt_time).unwrap_or_else(|| "DNF".into()) };
                    self.text(&painter, egui::pos2(panel.right() - 40.0 * sc, cy), egui::Align2::RIGHT_CENTER, &value, 14.0 * sc, col);
                }
                let base = list_y + row_h * rows as f32 + 16.0 * sc;
                if self.last_reward != (0, 0) {
                    self.text(&painter, egui::pos2(tx, base), egui::Align2::LEFT_TOP, &format!("WINNINGS ${}      REP +{}", self.last_reward.0, self.last_reward.1), 14.0 * sc, egui::Color32::from_rgb(120, 230, 120));
                }
                self.text(&painter, egui::pos2(panel.right() - 40.0 * sc, base), egui::Align2::RIGHT_TOP, &format!("CASH ${}", self.money), 12.0 * sc, egui::Color32::from_gray(200));
                let pointer = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_clicked()));
                let mut picked: Option<u8> = None;
                for (i, (label, id)) in ro.iter().enumerate() {
                    let r = egui::Rect::from_min_size(egui::pos2(panel.left() + 6.0 * sc, base + 34.0 * sc + row_h * i as f32), egui::vec2(w - 12.0 * sc, row_h));
                    let hovered = pointer.0.map_or(false, |p| r.contains(p));
                    if hovered && pointer.1 {
                        self.menu_sel = i;
                        picked = Some(*id);
                    }
                    if i == ro_sel {
                        self.cursor(&painter, r, sc);
                    }
                    let col = if i == ro_sel { egui::Color32::from_rgb(20, 14, 6) } else { egui::Color32::from_gray(200) };
                    self.text(&painter, egui::pos2(tx, r.center().y), egui::Align2::LEFT_CENTER, label, 18.0 * sc, col);
                }
                if enter {
                    picked = Some(ro[ro_sel].1);
                }
                // (test hook MCLA_NEXT_RACE=<n>: `mc4_next_race_hook`)
                if picked.is_none() && !self.reward_panel && self.mission_results.is_empty() && !self.results_table && ro.iter().any(|r| r.1 == 20) && self.mc4_next_race_hook() {
                    picked = Some(20);
                }
                if !self.event_line.is_empty() {
                    self.text(&painter, egui::pos2(tx, panel.top() + 74.0 * sc), egui::Align2::LEFT_TOP, &self.event_line, 13.0 * sc, accent);
                }
                match picked {
                    Some(9) => self.results_table = true,
                    // (step 19d: NEXT RACE with the same man - `mc4_next_race_take`)
                    Some(20) => {
                        if self.mc4_next_race_take().is_some() {
                            self.menu_sel = 0;
                            action = MenuAction::StartRace;
                        }
                    }
                    Some(10) => self.pause_do = Some(250),
                    Some(11) => self.pause_do = Some(251),
                    Some(12) => self.start_replay(),
                    Some(13) => {
                        // (out of the event first, then into the garage)
                        self.menu_sel = 0;
                        self.go_garage = true;
                        action = MenuAction::StartFreeRoam;
                    }
                    Some(3) => {
                        if self.money >= fix {
                            self.money -= fix;
                            self.quick_fix = true;
                            self.dirty = true;
                        } else {
                            let m = self.career.strings.get("G_PerformanceOverBudget").map(|s| s.to_uppercase()).unwrap_or_else(|| "NOT ENOUGH CASH".into());
                            self.show_message(&m);
                        }
                    }
                    // (NEXT RACE IN SERIES asks every time: the series handler has no damage test - form
                    // 71, Dialog_Title_NextSeries, sub_826624C8)
                    Some(0) if movie && self.series.is_some() => {
                        self.series_ask_kind = 0;
                        self.series_ask = Some(0);
                    }
                    // (a decided series: RETURN TO CRUISE and RACE SERIES AGAIN with their own boxes -
                    // Dialog_Title_ReturnToCruise, Dlg_RaceAgainSeries_Title)
                    Some(2) if movie && self.series_decided() => {
                        self.series_ask_kind = 4;
                        self.series_ask = Some(0);
                    }
                    Some(14) if self.series_decided() => {
                        self.series_ask_kind = 5;
                        self.series_ask = Some(0);
                    }
                    // (QUIT SERIES and RESTART SERIES ask the same way - the strings `Dlg_Title_Series_Quit`
                    // / `Dlg_Desc_Series_Quit`, `Dlg_Title_Series_restart` / `Dlg_Desc_Series_Restart`;
                    // their boxes were not captured: the CONTINUE SERIES box's shape)
                    Some(14) if self.series.is_some() => {
                        self.series_ask_kind = 2;
                        self.series_ask = Some(0);
                    }
                    Some(2) if movie && self.series.is_some() && self.next_event_race.is_some() => {
                        self.series_ask_kind = 1;
                        self.series_ask = Some(0);
                    }
                    Some(21) => {
                        self.menu_sel = 0;
                        // (asked every time: sub_826698B8 opens form 71 - Dialog_Title_RaceBack /
                        // Dialog_Description_RaceBack - as soon as RO_RaceBack is pressed)
                        if movie {
                            self.series_ask_kind = 3;
                            self.series_ask = Some(0);
                        } else {
                            self.althk.back_req = true;
                        }
                    }
                    Some(0) | Some(1) => {
                        self.menu_sel = 0;
                        action = match self.next_event_race.take() {
                            Some(n) => {
                                self.current_race = Some(n);
                                MenuAction::StartRace
                            }
                            None => MenuAction::RestartRace,
                        };
                    }
                    Some(_) => {
                        self.menu_sel = 0;
                        action = MenuAction::StartFreeRoam;
                    }
                    None => {}
                }
                if back {
                    self.menu_sel = 0;
                    action = MenuAction::StartFreeRoam;
                }
            }
            Mode::FreeRoam | Mode::Race => {
                if self.mode == Mode::FreeRoam && std::mem::take(&mut self.go_garage) {
                    action = MenuAction::OpenGarage;
                }
                if std::mem::take(&mut self.nav_map) {
                    // (a new career has no GPS map before its first mission is done: Back only
                    // puts the phone away - seen in the recomp; after FIRST IMPRESSIONS it opens)
                    let lesson = self.tutorial_box.as_ref().map_or(false, |t| t.0.contains("mission_log"));
                    if self.phone_challenge && self.tip.is_some() && !self.show_map {
                        // (LET'S RACE. YOU PICK THE LOCATION. - Back: ACCEPT -> ARCADE's "Choose race
                        // type:" - `call_challenge`, `call_accept_0`)
                        self.tip = None;
                        self.phone_challenge = false;
                        self.phone_accept = true;
                    } else if self.mlog_guided {
                        // (the lesson is left by setting the target)
                    } else if (lesson || (self.tip.is_some() && !self.phone_prompt.is_empty())) && !self.show_map {
                        // (Back on a call that offers MISSION LOG opens the log on its active
                        // missions - `ng_mlog_active` - whether or not the map is there yet. The
                        // first time, under the MISSION LOG box, it is a lesson.)
                        self.tip = None;
                        self.show_map = true;
                        self.mission_log = (0, 0, false);
                        self.mlog_near = true;
                        self.mlog_guided = self.career.stats.contains_key("tut:mission_log") && !self.career.stats.contains_key("tut:target");
                        if lesson {
                            self.tutorial_box = None;
                            if let Some(fp) = self.flash_popup.as_mut() {
                                fp.hide();
                            }
                        }
                    } else if self.gps_unlocked() || self.show_map {
                        self.show_map = !self.show_map;
                        self.map_cursor = usize::MAX;
                        // (the GPS opens on the map, the log put away)
                        self.mission_log.2 = std::env::var("MCLA_MAP_LOG").is_err();
                        self.mlog_near = false;
                    } else {
                        self.tip = None;
                    }
                }
                if back {
                    if self.show_map && self.mlog_guided {
                    } else if self.show_map {
                        self.show_map = false;
                    } else {
                        action = MenuAction::Back; // pause
                    }
                }
                // ---- race editor (on the map, free roam) ----
                let (edit, del, go) = (std::mem::take(&mut self.nav_edit), std::mem::take(&mut self.nav_del), std::mem::take(&mut self.nav_go));
                if !(self.show_map && self.mode == Mode::FreeRoam) {
                    self.editor = None;
                } else {
                    if edit {
                        self.editor = if self.editor.is_some() { None } else { Some(Vec::new()) };
                        self.editor_cursor = hud.player_pos;
                    }
                    if self.editor.is_some() {
                        let step = 28.0;
                        self.editor_cursor += Vec3::new((right as i32 - left as i32) as f32, 0.0, (down as i32 - up as i32) as f32) * step;
                        // onto the nearest street
                        let snapped = world.nav.as_ref().and_then(|nav| nav.nearest_link(self.editor_cursor).map(|(li, t, d)| (nav.links[li].point_at(t), d)));
                        let n = self.editor.as_ref().map_or(0, |p| p.len());
                        if enter {
                            if let Some((p, d)) = snapped.filter(|s| s.1 < 80.0) {
                                let _ = d;
                                if let Some(pts) = self.editor.as_mut() {
                                    if pts.last().map_or(true, |l| l.distance(p) > 20.0) && pts.len() < 40 {
                                        pts.push(p);
                                    }
                                }
                            }
                        }
                        if del {
                            if let Some(pts) = self.editor.as_mut() {
                                pts.pop();
                            }
                        }
                        if go && n >= 2 {
                            let pts = self.editor.take().unwrap_or_default();
                            self.user_races.push(pts.iter().map(|p| p.to_array()).collect());
                            if self.user_races.len() > 12 {
                                self.user_races.remove(0);
                            }
                            // (numbers follow the list, so the oldest slot is reused)
                            let all = self.user_races.clone();
                            let mut idx = None;
                            for (k, r) in all.iter().enumerate() {
                                let v: Vec<Vec3> = r.iter().map(|p| Vec3::from(*p)).collect();
                                idx = self.add_user_race(k + 1, &v);
                            }
                            self.write_save(&self.data_dir.clone());
                            if let Some(i) = idx {
                                self.current_race = Some(i);
                                self.race_sel = i;
                                self.show_map = false;
                                action = MenuAction::StartRace;
                            }
                        }
                    }
                }
                let enter = enter && self.editor.is_none();
                // free roam: pull up at a race start to enter the event
                // (the game has no such places: races are entered by flashing at a racer on the
                // road, at a hangout or from the map - `ng_chase_*`, `hg_*`. Kept for the
                // port's own HUD and behind MCLA_RACE_PLACES.)
                self.race_here = None;
                if self.mode == Mode::FreeRoam && hud.heat == 0 && (self.flash_hud.is_none() || std::env::var("MCLA_RACE_PLACES").is_ok()) {
                    if let Some((i, _, d)) = self.races_near(hud.player_pos, 1).into_iter().next() {
                        if d < 16.0 && hud.speed_kmh.abs() < 25.0 {
                            self.race_here = Some(i);
                            if enter {
                                self.current_race = Some(i);
                                self.race_sel = i;
                                self.show_map = false;
                                action = MenuAction::StartRace;
                            }
                        }
                    }
                }
                // A race's opening shot carries the race's badge - its name, its kind, the
                // event's icon in a ring (the popup movie's `template_StartOfRace`; reference
                // `y_replay_intro`) - and no bars.
                let dive_name = String::new();
                let p080_dl = self.race.as_ref().map_or(false, |r| r.kind == RaceKind::Delivery);
                // (the fly-in has no badge - the original's opening shots, work/career2/cap/d1_03..04; it comes
                // with the start scene's racename event)
                let _ = (&dive_name, p080_dl);
                let badge_from = self.cine.as_ref().filter(|s| !s.is_empty());
                let race_badge = match (badge_from, self.race.as_ref(), self.current_race.and_then(|i| self.race_defs.get(i))) {
                    (Some(sub), Some(_), Some(d)) if !self.cine_black && self.flash_popup.is_some() && self.message_box.is_none() && self.arrest_panel.is_none() => {
                        let name = self.career.strings.get(&d.id).filter(|v| !v.contains('<')).map(|v| v.to_uppercase()).unwrap_or_else(|| sub.clone());
                        let key = match d.sub_type.to_ascii_lowercase().as_str() {
                            "ordered" => "cp_ordered",
                            "unordered" => "cp_unordered",
                            "circuit" => "Circuit",
                            "blitz" => "Blitz",
                            "delivery" => "Delivery",
                            _ => "",
                        };
                        let kind = self.career.strings.get(key).map(|v| if key == "Delivery" { v.to_string() } else { v.to_uppercase() }).unwrap_or_else(|| format!("{} RACE", d.sub_type.to_uppercase()));
                        // (GOAL ATTACK: "TARGET TIME: mm:ss.hh" in place of the kind - `mc4_goal_badge`)
                        let kind = self.mc4_goal_badge().map(|t| t.to_uppercase()).unwrap_or(kind);
                        // the icon is the district's emblem (the clip's frames: 0 a star -
                        // Hollywood, as captured; 1 a palm, 2 a wave, 4 SC, 6 LA; 9 handcuffs)
                        let dl = format!("{} {}", d.district, d.id).to_ascii_lowercase();
                        let icon = if dl.contains("south") || dl.contains("sc_") { 4 } else if dl.contains("downtown") || dl.contains("dt_") { 6 } else if dl.contains("beach") || dl.contains("monica") || dl.contains("sm_") { 1 } else if dl.contains("hills") || dl.contains("bh_") { 2 } else { START_BADGE_ICON };
                        Some((name, kind, icon))
                    }
                    _ => None,
                };
                if let Some((name, kind, icon)) = race_badge {
                    let page = crate::flash_ui::MenuPage {
                        layout: crate::flash_ui::MenuLayout::StartOfRace { description: kind, ring: std::env::var("MCLA_BADGE_RING").ok().and_then(|v| u32::from_str_radix(&v, 16).ok()).unwrap_or_else(|| self.badge_ring()) },
                        icon: std::env::var("MCLA_BADGE_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(icon),
                        title: name,
                        select: usize::MAX,
                        ..Default::default()
                    };
                    let dt = ctx.input(|i| i.stable_dt).min(0.1);
                    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("cine"));
                    if let Some(fp) = self.flash_popup.as_mut() {
                        fp.show(&page);
                        fp.movie.player.update(dt);
                        fp.movie.paint(ctx, &ctx.layer_painter(layer));
                        if std::env::var("MCLA_BADGE_LOG").is_ok() {
                            for path in ["screen", "screen.menu", "screen.menu.raceName", "screen.menu.raceName.slot0"] {
                                let st = fp.movie.player.find(path).and_then(|id| fp.movie.player.inst(id)).map(|i| (i.tint, i.x, i.y));
                                log::info!("badge: {} {:?} template {:?} name {:?} vis {:?} alpha {:?}", path, st, fp.movie.player.get_path("template").map(|v| v.to_i32()), fp.movie.player.get_path("menu.name").map(|v| v.to_i32()), fp.movie.player.get_path("cur_visibility").map(|v| v.to_i32()), fp.movie.player.get_path("cur_alpha").map(|v| v.to_f32()));
                            }
                        }
                    }
                    self.start_badge_on = true;
                    self.start_badge_out = false;
                } else if std::mem::take(&mut self.start_badge_on) {
                    if let Some(fp) = self.flash_popup.as_mut() {
                        fp.hide();
                    }
                    self.start_badge_out = true;
                }
                // the badge's fade: the popup movie's own clip event (`cur_visibility` 1: `cur_alpha` += 10 a
                // frame to 100; 0: -= 10 - 5 only for template_CutsceneTitile -, the movie disabled at 0),
                // 1/3 s each way at the movie's 30 frames a second - drawn on after the racename event ends
                if self.start_badge_out && !self.start_badge_on {
                    let dt = ctx.input(|i| i.stable_dt).min(0.1);
                    match self.flash_popup.as_mut() {
                        Some(fp) if !fp.is_shown() => {
                            fp.movie.player.update(dt);
                            fp.movie.paint(ctx, &ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("cine"))));
                            let a = fp.movie.player.get_path("cur_alpha").map_or(0.0, |v| v.to_f32());
                            if std::env::var("MCLA_BADGE_LOG").is_ok() {
                                log::info!("badge fade out: cur_alpha {:.0}", a);
                            }
                            if a <= 0.0 {
                                self.start_badge_out = false;
                            }
                        }
                        _ => self.start_badge_out = false,
                    }
                }
                // a race's start scene: no HUD before its countdown event, the race HUD (map and 3-2-1)
                // from it on over the scene's camera (GO 3 s later, prerace.rs)
                if race_badge_shown(self.start_badge_on) || self.dive_k > 0.0 || self.scene_hud_off {
                } else if let Some(sub) = self.cine.as_ref() {
                    // story scene: black bars, the line being spoken in the lower one
                    let screen = ctx.screen_rect();
                    let sc = (screen.height() / 900.0).clamp(0.5, 2.0);
                    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("cine")));
                    // (the original's story scenes fill the frame - no bars, no skip line, and in a
                    // new career no subtitles either: `ng_c0`..`ng_c3`. The port's own UI keeps
                    // its bars.)
                    let plain = self.flash_hud.is_some() && !self.cine_black && std::env::var("MCLA_CINE_BARS").is_err();
                    let bar = if self.cine_black { screen.height() * 0.5 } else { screen.height() * 0.12 };
                    if !plain {
                        p.rect_filled(egui::Rect::from_min_size(screen.min, egui::vec2(screen.width(), bar)), 0.0, egui::Color32::BLACK);
                        p.rect_filled(egui::Rect::from_min_size(egui::pos2(screen.left(), screen.bottom() - bar), egui::vec2(screen.width(), bar)), 0.0, egui::Color32::BLACK);
                    }
                    let sub = if plain && !self.opts.subtitles { "" } else { sub.as_str() };
                    let lines = self.wrap(sub, 15.0 * sc, screen.width() * 0.8);
                    for (i, l) in lines.iter().enumerate() {
                        let pos = egui::pos2(screen.center().x, screen.bottom() - bar * 0.5 + (i as f32 - (lines.len() as f32 - 1.0) * 0.5) * 24.0 * sc);
                        if plain {
                            self.text(&p, pos + egui::vec2(1.5, 1.5), egui::Align2::CENTER_CENTER, l, 15.0 * sc, egui::Color32::BLACK);
                        }
                        self.text(&p, pos, egui::Align2::CENTER_CENTER, l, 15.0 * sc, egui::Color32::WHITE);
                    }
                    if !plain {
                        self.text(&p, egui::pos2(screen.right() - 30.0 * sc, screen.top() + bar * 0.5), egui::Align2::RIGHT_CENTER, "ENTER  SKIP", 9.0 * sc, egui::Color32::from_gray(120));
                    }
                } else if self.fresh_game && self.mode == Mode::FreeRoam && self.flash_prompt.is_some() && (self.message_box.is_some() || (self.save_box < 2 && self.flash_popup.is_some() && std::env::var("MCLA_NO_SAVE_BOX").is_err())) {
                    // (the save boxes stand over a dark screen before VEHICLE SELECT)
                    ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("savebox_dim"))).rect_filled(ctx.screen_rect(), 0.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 235));
                } else if self.fresh_game && self.mode == Mode::FreeRoam && self.flash_prompt.is_some() && self.hold_starter {
                    // (the opening is about to play)
                } else if self.fresh_game && self.mode == Mode::FreeRoam && self.flash_prompt.is_some() {
                    self.starter_ui(ctx, left, right, enter);
                } else if self.show_map {
                    // The original's icons on the GPS map (`hudmap.rs`, mode 1): the same elements,
                    // each in the 3D scene of the map camera, sized by its distance from it
                    if let (true, Some(vp)) = (self.hm.is_some() && self.flash_navsys.is_some() && self.editor.is_none(), hud.map_vp) {
                        let els = crate::hudmap::elements(self, vehicles, hud.heat >= 2);
                        let racing = self.race.is_some();
                        let strobe_on = self.race.as_ref().map_or(true, |r| r.rivals_checkpoints.is_none());
                        let hm = self.hm.as_mut().unwrap();
                        let dt = ctx.input(|i| i.stable_dt).min(0.1);
                        hm.tick_once(ctx, dt, els.iter().filter(|e| matches!(e.kind, crate::hudmap::Kind::Checkpoint(_))).count());
                        hm.gps_cam = hud.cam_pos;
                        let f = hm.frame(1, vp, hud.cam_pos, 0.0, racing, hud.heat as i32, false, strobe_on);
                        let q = crate::hudmap::build(&hm.tune, &mut hm.assets, &f, &els, &[], false);
                        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("gps_icons"));
                        ctx.move_to_top(layer);
                        // MAP MODE's road overlay first (the UI components' 3D pass comes before the icons)
                        if hm.map_mode {
                            crate::hudmap::overlay_fade(&mut hm.road_fades, hud.cam_pos.y, dt);
                        } else {
                            hm.road_fades = [0.0; 5];
                        }
                        if let (true, Some(roads)) = (hm.map_mode, self.road_map.as_ref()) {
                            // (drawn in the 3D frame against its depth, as the original - hudmap_gpu.rs;
                            // MCLA_XRAY_CPU=1: the egui meshes with the relief's x-ray test)
                            if crate::hudmap_gpu::cpu_path() {
                                crate::hudmap::paint_roads(&ctx.layer_painter(layer), roads, vp, ctx.screen_rect(), hud.cam_pos, &hm.road_fades, hm.heights.as_ref());
                            } else {
                                ctx.layer_painter(layer).add(crate::hudmap_gpu::roads(ctx.screen_rect(), roads, vp, hud.cam_pos, &hm.road_fades));
                            }
                        }
                        // the street names (MAP MODE's NAVSYS2DMOVIE), after the 3D scene and before the icons
                        if let (true, Some(sn)) = (hm.map_mode, hm.street.as_mut()) {
                            sn.update(vp, hud.cam_pos.y, crate::hudmap::nav_zoom_u(hud.cam_pos.y), dt);
                            sn.movie.paint(ctx, &ctx.layer_painter(layer));
                        }
                        let ripple = crate::hudmap::Ripple::new(&hm.tune, hm.t, 9000.0);
                        if crate::hudmap_gpu::cpu_path() {
                            let hf = hm.heights.clone();
                            crate::hudmap::paint_world(ctx, &ctx.layer_painter(layer), &mut hm.assets, &q, vp, ctx.screen_rect(), Some(ripple), hf.as_ref().map(|h| (h, hud.cam_pos)));
                        } else {
                            ctx.layer_painter(layer).add(crate::hudmap_gpu::icons(ctx.screen_rect(), &mut hm.assets, &q, vp, hud.cam_pos, Some(ripple)));
                        }
                    }
                    self.city_map(ctx, vehicles, world, hud);
                    self.mission_log_ui(ctx, [up, down, left, right, enter], tab_prev, tab_next, hud.map_vp, hud.player_pos);
                } else {
                    self.hook_icons(ctx, vehicles, &hud);
                    self.race_markers(ctx, vehicles, &hud);
                    // (the first target set brings the MINI MAP box - `ng_tut_minimap`: "GPS Targets
                    // will appear highlighted in the Mini Map...", CONTINUE (A))
                    if self.tutorial_box.is_none() && self.mode == Mode::FreeRoam && self.career.stats.contains_key("tut:target_set") && !self.career.stats.contains_key("tut:target") {
                        self.career.stats.insert("tut:target".into(), 1);
                        self.tutorial_box = Some(("tutorial_target_title".into(), "tutorial_target_desc".into(), 0.5, 600.0));
                        // (the phone is away under this box - `ng_tut_minimap`)
                        self.tip = None;
                    }
                    // a tutorial's box over the cruise
                    if let Some((title, text, wait, left)) = self.tutorial_box.clone() {
                        // (a box with CONTINUE stays until it is answered)
                        let left = if (title.contains("target") || self.mc4_tut_continue(&title)) && wait <= 0.0 && enter { 0.0 } else { left };
                        let dt = ctx.input(|i| i.stable_dt).min(0.1);
                        if self.mode != Mode::FreeRoam || left <= 0.0 {
                            self.tutorial_box = None;
                            if let Some(fp) = self.flash_popup.as_mut() {
                                fp.hide();
                            }
                        } else if wait > 0.0 {
                            self.tutorial_box = Some((title, text, wait - dt, left));
                        } else {
                            let st = |k: &str| self.career.strings.get(k).map(|v| v.to_string()).unwrap_or_default();
                            // (the mission log's box carries its prompt: MISSION LOG (Back) - `ng_tut_missionlog`)
                            let prompts = if title.contains("target") || self.mc4_tut_continue(&title) { vec![(self.career.strings.get("prompt_continue").map(|v| v.to_uppercase()).unwrap_or_else(|| "CONTINUE".into()), crate::flash_ui::ICON_A)] } else if title.contains("mission_log") { vec![(self.career.strings.get("Navmap_Prompt_MissionLog").map(|v| v.to_uppercase()).unwrap_or_else(|| "MISSION LOG".into()), crate::flash_ui::ICON_BACK_BUTTON)] } else { Vec::new() };
                            let page = crate::flash_ui::MenuPage { layout: crate::flash_ui::MenuLayout::Tutorial { description: st(&text) }, title: st(&title), select: usize::MAX, prompts, ..Default::default() };
                            let which = std::env::var("MCLA_TUT_MOVIE").unwrap_or_default();
                            let fm = if which == "pause" { self.flash_menu.as_mut() } else { self.flash_popup.as_mut() };
                            if let Some(fp) = fm {
                                fp.show(&page);
                                fp.movie.player.update(dt);
                                fp.movie.paint(ctx, &ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("tutorial"))));
                            }
                            self.tutorial_box = Some((title, text, 0.0, left - dt));
                        }
                    }
                    self.hud(ctx, vehicles, world, hud);
                    // (step 19c: the thirty seconds of a run to the line are the race clock's)
                    let race_hud = self.race_hud(vehicles, &hud).or_else(|| self.mc4_bmt_hud());
                    let idle_cam = self.idle_cam;
                    let (shown_camera, menu_up) = (self.hud_camera, self.in_menu());
                    if let Some(fh) = self.flash_hud.as_mut().filter(|_| !idle_cam) {
                        let dt = ctx.input(|i| i.stable_dt).min(0.1);
                        // the info line under the speed: the game queues texts for it (0x822068F8)
                        // and leaves it empty otherwise. (approx.) Here: the name of the road the
                        // car is on while it moves, as the reference captures show it.
                        let road = world.nav.as_ref().and_then(|nav| {
                            let mut best: Option<(f32, usize)> = None;
                            for i in nav.links_near(hud.player_pos, 20.0) {
                                let l = &nav.links[i];
                                let q = l.point_at(l.project(hud.player_pos));
                                let d = q.distance_squared(hud.player_pos);
                                if (q.y - hud.player_pos.y).abs() < 4.0 && d < 400.0 && best.map_or(true, |b| d < b.0) {
                                    best = Some((d, i));
                                }
                            }
                            // (road names carry their district: "HW_OH_SELMA AVE.")
                            // (a car is on a road once it is in one of its lanes: standing at the kerb
                            // of SUNSET BLVD. a new career's line is empty and the plate still on the
                            // movie's 55 - `ng_c4`; the road then stays until another is entered)
                            let near = best.map_or(false, |(d, _)| d < 2.6 * 2.6);
                            best.and_then(|(_, i)| nav.links[i].zone).and_then(|z| nav.zones.get(z)).map(|z| (z.name.rsplit('_').next().unwrap_or("").trim().to_uppercase(), z.speed_limit_mps())).filter(|r| near || r.0 == self.info_road.0)
                        });
                        // (approx.) shown while moving, and for eight seconds after the road changes
                        // or an event begins (reference `t10`..`t13`: SUNSET BLVD. on the grid)
                        let moving = hud.speed_kmh.abs() > 8.0;
                        let name = road.as_ref().map(|r| r.0.clone()).unwrap_or_default();
                        if name != self.info_road.0 || self.race.as_ref().map_or(false, |r| r.countdown > 0.0) {
                            self.info_road = (name, 0.0);
                        } else {
                            self.info_road.1 += dt;
                        }
                        let fresh = self.info_road.1 < 8.0;
                        // a new song runs across the line first: eighteen characters at a time,
                        // five a second, with a pause at either end (approx.)
                        let song = self.song_line.take().and_then(|(t, at)| {
                            const WIDTH: usize = 18;
                            let chars: Vec<char> = t.chars().collect();
                            let travel = chars.len().saturating_sub(WIDTH) as f32 / 5.0;
                            if at > travel + 4.0 {
                                return None;
                            }
                            let first = (((at - 2.0).max(0.0) * 5.0) as usize).min(chars.len().saturating_sub(WIDTH));
                            let shown: String = chars.iter().skip(first).take(WIDTH).collect();
                            self.song_line = Some((t, at + dt));
                            Some(shown)
                        });
                        match song {
                            Some(t) => fh.info_line(Some(t.as_str())),
                            None => fh.info_line(road.as_ref().map(|r| r.0.as_str()).filter(|n| (moving || fresh) && !n.is_empty())),
                        }
                        // the boxed figure is the limit of the road the car is on (`t_speed_limit`;
                        // the movie starts on 55 and keeps the last road's between roads)
                        if let Some((_, limit)) = road.as_ref() {
                            fh.movie.player.set_global("t_speed_limit", mcla_flash::value::Value::str(&format!("{:.0}", limit / 0.44704)));
                        }
                        // (hudOverheadMap's onentry sets HUDMOVIE cur_visibility = 0, but hudBigMap's
                        // refreshHUD follows it: the original keeps the gauges up under the overhead
                        // map - recorded, `work/mm/cap/ov_s2.png`)
                        // (the camera bar: the view's list place, not passed on while a menu is up)
                        fh.camera(shown_camera, menu_up);
                        // (before GO the movie steps with the race clock: its 3-2-1 ends on the GO tick)
                        fh.drive(&hud, race_hud, if self.race.as_ref().map_or(false, |r| r.countdown > 0.0) { race_dt } else { dt });
                        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_hud"));
                        // above the port's own HUD layer, which still draws the map's streets
                        ctx.move_to_top(layer);
                        let p = ctx.layer_painter(layer);
                        fh.movie.paint(ctx, &p);
                    }
                    // the cut's black mask (TRANSITIONMOVIE `mask`, mask_color 0) over everything
                    if self.cut_mask > 0.0 {
                        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("cut_mask"));
                        ctx.move_to_top(layer);
                        ctx.layer_painter(layer).rect_filled(ctx.screen_rect(), 0.0, egui::Color32::from_black_alpha((self.cut_mask.clamp(0.0, 1.0) * 255.0).round() as u8));
                    }
                    let hm_els = if self.hm.is_some() { crate::hudmap::elements(self, vehicles, hud.heat >= 2) } else { Vec::new() };
                    if let (Some(fm), false) = (self.flash_map.as_mut(), (self.hud_map_big && (self.hm.is_none() || crate::hudmap::old_bigmap())) || idle_cam) {
                        let dt = ctx.input(|i| i.stable_dt).min(0.1);
                        // the arrow on the ring points at the next checkpoint (`gpsArrowRotation`,
                        // degrees from straight ahead)
                        let gps = self.race.as_ref().and_then(|race| {
                            // (an unordered race: the nearest open checkpoint - checkpoints::gps_target)
                            let cp = crate::checkpoints::gps_target(&race.checkpoints, race.next.first().copied().unwrap_or(0), race.rivals_checkpoints.is_some(), hud.player_pos)?;
                            let d = cp - hud.player_pos;
                            Some(((d.x.atan2(-d.z) - hud.player_heading).to_degrees(), d))
                        });
                        // (cruising: at the GPS target - "The directional arrow will point in the
                        // direction of the target", `ng_tut_minimap`)
                        let gps = if self.hm.is_some() {
                            // The original's arrow (`sub_8223EFC8`): only for a GPS target that has been
                            // SET (navsys `sub_82622DC0` gives the arrow the target's position), and
                            // straight at it - the signed angle in the car's yaw frame, no route
                            // (rule_frame.md 5). The port's own guidance route is not a target.
                            gps.or_else(|| {
                                let t = if self.mission_target.is_some() || self.gps_pick.is_some() { self.gps.as_ref().and_then(|p| p.last().copied()).or(self.gps_pick)? } else { return None };
                                let d = t - hud.player_pos;
                                Some(((d.x.atan2(-d.z) - hud.player_heading).to_degrees(), d))
                            })
                        } else {
                            gps
                        };
                        let gps = gps.or_else(|| {
                            if self.hm.is_some() {
                                return None;
                            }
                            // (along the way: the route's point some eighty metres on - the arrow of
                            // `ng_tut_minimap` does not point straight at the target)
                            let path = self.gps.as_ref()?;
                            let mut left = 80.0;
                            let mut at = *path.first()?;
                            for w in path.windows(2) {
                                let l = w[0].distance(w[1]);
                                if l >= left {
                                    at = w[0].lerp(w[1], left / l.max(1e-3));
                                    left = 0.0;
                                    break;
                                }
                                left -= l;
                                at = w[1];
                            }
                            let _ = left;
                            let d = at - hud.player_pos;
                            Some(((d.x.atan2(-d.z) - hud.player_heading).to_degrees(), d))
                        });
                        // GAME OPTIONS' ARROW (`mc4`): COMPASS - the arrow on the ring (the movie's
                        // `gpsArrowActive`, sub_8223F2D0); TOP - the 3D arrow over the car, which takes
                        // the target in the car's level frame (right, up, back); NONE - neither
                        let arrow_mode = self.mc4.as_ref().map_or(0, |m| m.options.gps_arrow.min(2));
                        let (sin, cos) = hud.player_heading.sin_cos();
                        let top = gps.filter(|_| arrow_mode == 1).map(|(_, d)| [d.x * cos + d.z * sin, d.y, d.z * cos - d.x * sin]);
                        crate::mc4::top_arrow(&mut self.car_x, &self.data_dir, ctx, top, dt);
                        let gps = gps.filter(|_| arrow_mode == 0).map(|g| g.0);
                        fm.highlight = self.tutorial_box.as_ref().map_or(false, |t| t.0.contains("target") && t.2 <= 0.0);
                        fm.own_map = self.hm.is_some();
                        // the overhead map (D-pad up / the map key, a red light start): the movie in
                        // mode 2 (rule_frame.md 9)
                        fm.overhead = self.hud_map_big && self.hm.is_some();
                        fm.overhead_title = self.overhead_title.clone();
                        fm.overhead_sub = self.overhead_sub.clone();
                        fm.drive(&hud, gps, dt);
                        // The original's map (`hudmap.rs`): the 220x220 target drawn from the movie's
                        // camera, shown by the movie itself in `HUD.renderMe` at its depth (20, between
                        // the ring at 3 and the north marker at 22) over the `miniBG` plate.
                        if let (Some(hm), Some(car)) = (self.hm.as_mut(), vehicles.player()) {
                            let h = crate::hudmap::heading(-car.state.forward());
                            // (the overhead map, mode 2: the movie's camera in world coordinates -
                            // fixed over the city, north up, no heading turn; its own 749x430 target)
                            let over = fm.overhead;
                            let (vp, eye) = if over { (crate::hudmap::overhead_view_proj(fm.cam_eye, fm.cam_look, fm.cam_fovy, fm.aspect), fm.cam_eye) } else { crate::hudmap::minimap_view_proj(car.state.pos, h, fm.cam_eye, fm.cam_look, fm.cam_fovy) };
                            let racing = self.race.is_some();
                            let target = (self.mission_target.is_some() || self.gps_pick.is_some()) && !racing;
                            let strobe_on = self.race.as_ref().map_or(true, |r| r.rivals_checkpoints.is_none());
                            let f = hm.frame(if over { 2 } else { 0 }, vp, eye, if over { 0.0 } else { h }, racing, hud.heat as i32, target, strobe_on);
                            hm.tick_once(ctx, dt, hm_els.iter().filter(|e| matches!(e.kind, crate::hudmap::Kind::Checkpoint(_))).count());
                            if over {
                                let tex = hm.overhead(ctx, &f, &hm_els, self.road_map.as_ref());
                                fm.movie.swap_handle("HUD.renderMe", 0x4d4d_0002, tex);
                            } else {
                                let tex = hm.minimap(ctx, &f, &hm_els, self.road_map.as_ref());
                                fm.movie.swap_handle("HUD.renderMe", 0x4d4d_0001, tex);
                            }
                            // map_rotation: world north 500 m from the player through the map camera
                            let n = vp * (car.state.pos + Vec3::new(0.0, 0.0, -500.0)).extend(1.0);
                            if n.w.abs() > 1e-4 {
                                fm.map_rotation = (std::f32::consts::FRAC_PI_2 - (n.y / n.w).atan2(n.x / n.w)).to_degrees();
                            }
                            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_map"));
                            ctx.move_to_top(layer);
                            fm.movie.paint(ctx, &ctx.layer_painter(layer));
                        } else {
                        // The movie stacks the ring (depth 3) under the map target (20) and the north
                        // marker (22) over it, so the map covers all but the ring's outer edge. Two
                        // passes round the port's own map layer: the ring below it, the marker above.
                        let alpha = |fm: &mut crate::flash_ui::FlashMinimap, path: &str, a: u8| {
                            if let Some(id) = fm.movie.player.find(path) {
                                if let Some(i) = fm.movie.player.inst_mut(id) {
                                    i.tint[3] = a;
                                }
                            }
                        };
                        let (ring, north) = ("HUD.ring", "HUD.ringNorth");
                        let ring_a = fm.movie.player.find(ring).and_then(|id| fm.movie.player.inst(id)).map_or(255, |i| i.tint[3]);
                        let north_a = fm.movie.player.find(north).and_then(|id| fm.movie.player.inst(id)).map_or(255, |i| i.tint[3]);
                        alpha(fm, north, 0);
                        fm.movie.paint(ctx, &ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("flash_map_under"))));
                        alpha(fm, north, north_a);
                        alpha(fm, ring, 0);
                        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_map"));
                        ctx.move_to_top(layer);
                        fm.movie.paint(ctx, &ctx.layer_painter(layer));
                        alpha(fm, ring, ring_a);
                        }
                    }
                }
            }
        }
        self.message_timer = (self.message_timer - ctx.input(|i| i.stable_dt)).max(0.0);
        action
    }

    /// The running event as the HUD movie shows it.
    fn race_hud(&self, vehicles: &Vehicles, info: &HudInfo) -> Option<crate::flash_ui::RaceHud> {
        use crate::flash_ui::{RaceHud, RaceHudKind};
        let race = self.race.as_ref()?;
        let (pos, total) = self.player_position(vehicles);
        let next = race.next.first().copied().unwrap_or(0);
        let lap_len = race.lap_len.max(1);
        let laps = race.laps.max(1);
        // (step 19c: the race clock counting down - a time trial, LAST MAN OUT's ten
        // seconds -, the field LAST MAN OUT has left, the payback's wins: `mc4`)
        let s19a_clock = self.mc4_hud_clock();
        let s19a_pb = self.mc4_payback_hud();
        let total = self.mc4_hud_total(total);
        Some(RaceHud {
            kind: match race.kind {
                RaceKind::Race => RaceHudKind::Race,
                RaceKind::Delivery => RaceHudKind::Delivery,
                RaceKind::Payback => RaceHudKind::Payback,
            },
            elapsed: race.time.max(0.0),
            time_left: s19a_clock.map(|c| c.0).or((race.kind != RaceKind::Race && race.time_limit > 0.0).then(|| (race.time_limit - race.time).max(0.0))),
            // (a payback's leg script turns the clock's property 0 on when the client is
            // reached - PaybackLegCore 0x0bf4; a delivery's clock has no countdown pie)
            clock_count: s19a_clock.map(|c| (c.1, c.2)).or((race.kind == RaceKind::Payback && race.time_limit > 0.0).then(|| ((race.time_limit - race.time) / 10.0, false))),
            clock_off: s19a_pb.map_or(false, |p| !p.3),
            payback: s19a_pb.map(|p| (p.0, p.1, p.2)),
            payback_vis: self.mc4_payback_vis(),
            lap: (next / lap_len + 1).min(laps) as i32,
            laps: laps as i32,
            pos: pos as i32,
            total: total as i32,
            countdown: race.countdown,
            delivery_damage: info.damage,
            target_damage: race.target.and_then(|t| vehicles.cars.get(t)).map_or(0.0, |c| c.state.damage),
        })
    }

    fn hud(&self, ctx: &egui::Context, vehicles: &Vehicles, world: &World, info: HudInfo) {
        use crate::hud::{arc, dial_angle, image, image_rot};
        let screen = ctx.screen_rect();
        let accent = egui::Color32::from_rgb(255, 140, 0);
        let white = egui::Color32::WHITE;
        let sc = (screen.height() / 900.0).clamp(0.5, 2.0);
        let overlay = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("hud_art")));
        let art = self.art.as_ref();

        // (the garage's and the deliveries' markers in the world are the free-roam icons now: the
        // original's `3dicons` movie - hookicons.rs, `Game::hook_icons`)

        // ---- rain: drops in the air around the camera, as the game's rain tuning describes
        // them (raintuning.xml: the box they fall in, their fall speeds and the wind, how long
        // and thick a streak is, between which distances they show, the splashes) ----
        // (MCLA_DEBUG_VIEW=97 only: the rain is drawn into the scene now, as the original does - rain.rs)
        if let (true, Some(vp)) = (info.rain > 0.02 && crate::rain::old_look(), info.cam_vp) {
            let rain = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("rain")));
            let rt = |k: &str, d: f32| self.rain_tune.get(k).copied().unwrap_or(d);
            let tm = ctx.input(|i| i.time) as f32;
            let size = rt("EmitterPosRangeX", 39.5).abs().max(rt("EmitterPosRangeZ", 37.8).abs()).max(10.0);
            let top = rt("EmitterPosY", 6.0).max(2.0);
            let (wx, wy) = (rt("WindX", 0.0), rt("WindY", -7.25).min(-1.0));
            let (wrx, wry) = (rt("WindRandomRangeX", 4.0), rt("WindRandomRangeY", -11.2));
            let (near, far) = (rt("NearSlice", 2.25), rt("FarSlice", 34.0));
            let (len_k, thick) = (rt("Streaklength", 0.105), rt("Streakthickness", 0.136));
            let opacity = if info.night { rt("RainOpacityNight", 0.0175) } else { rt("RainOpacity", 0.014) } / 0.014;
            let fall = top + 5.0;
            // (the box rides ahead of the camera; the drops themselves stay where they are in
            // the world, so driving through them slants the streaks)
            let centre = info.cam_pos + info.cam_fwd * (size * 0.25);
            let wrap = |v: f32, c: f32| c + (v - c + size * 0.5).rem_euclid(size) - size * 0.5;
            let to_screen = |p: Vec3| -> Option<(egui::Pos2, f32)> {
                let c = vp * p.extend(1.0);
                (c.w > 0.3).then(|| (egui::pos2(screen.left() + (c.x / c.w * 0.5 + 0.5) * screen.width(), screen.top() + (0.5 - c.y / c.w * 0.5) * screen.height()), c.w))
            };
            let hash = |k: u32, s: u32| -> f32 {
                let mut x = k.wrapping_mul(747796405).wrapping_add(s.wrapping_mul(2891336453));
                x ^= x >> 16;
                x = x.wrapping_mul(2246822519);
                x ^= x >> 13;
                (x % 10000) as f32 / 10000.0
            };
            let n = (1500.0 * info.rain) as u32;
            for k in 0..n {
                let vel = Vec3::new(wx + wrx * (hash(k, 2) - 0.5), wy + wry * hash(k, 1), wrx * (hash(k, 6) - 0.5) * 0.5);
                let t = (tm * -vel.y / fall + hash(k, 5)).fract();
                let p = Vec3::new(wrap(hash(k, 3) * size + vel.x * tm, centre.x), info.cam_pos.y + top - t * fall, wrap(hash(k, 4) * size + vel.z * tm, centre.z));
                // the streak: where the drop was a moment ago, as the camera saw it
                let tail = p - (vel - info.cam_vel) * len_k;
                let (Some((a, da)), Some((b, _))) = (to_screen(p), to_screen(tail)) else { continue };
                let fade = ((da - near) / 2.0).clamp(0.0, 1.0) * ((far - da) / 8.0).clamp(0.0, 1.0);
                if fade <= 0.0 || (a - b).length() > screen.height() * 0.6 {
                    continue;
                }
                let w = (thick * 0.1 * screen.height() / da).clamp(0.5, 2.0);
                rain.line_segment([a, b], egui::Stroke::new(w, egui::Color32::from_rgba_unmultiplied(205, 212, 226, (75.0 * opacity * fade).min(160.0) as u8)));
            }
            // splashes on the ground around the car
            let (s_size, s_op) = (rt("SplashSizeScale", 0.135), rt("SplashOpacity", 0.095) / 0.095);
            let ground = info.player_pos.y + rt("SplashOffset", 0.068);
            for k in 0..(160.0 * info.rain) as u32 {
                let ph = tm / 0.35 + hash(k, 11);
                let (cycle, t) = (ph.floor() as u32, ph.fract());
                let seed = k.wrapping_mul(977).wrapping_add(cycle.wrapping_mul(131));
                let p = Vec3::new(info.cam_pos.x + info.cam_fwd.x * 9.0 + (hash(seed, 12) - 0.5) * 30.0, ground, info.cam_pos.z + info.cam_fwd.z * 9.0 + (hash(seed, 13) - 0.5) * 30.0);
                let Some((c, d)) = to_screen(p) else { continue };
                if d < near || d > far {
                    continue;
                }
                // (a ring on the ground, seen flat)
                let r = s_size * (0.6 + 2.4 * t) * 0.9 * screen.height() / d;
                let flat = ((info.cam_pos.y - ground) / d).clamp(0.08, 0.6);
                let col = egui::Color32::from_rgba_unmultiplied(215, 222, 235, (110.0 * s_op * (1.0 - t)) as u8);
                rain.add(egui::Shape::ellipse_stroke(c, egui::vec2(r, r * flat), egui::Stroke::new(1.0, col)));
            }
        }

        // the original HUD movie draws the gauge cluster and its meters
        // (the idle camera shows the car and the world's markers, no HUD - `idle_garage_marker`)
        if self.idle_cam {
            return;
        }
        let flash = self.flash_hud.is_some();
        // ---- special ability meter (left of the gauge cluster) ----
        if !info.ability.is_empty() && !flash {
            let c = egui::pos2(screen.right() - 430.0 * sc, screen.bottom() - 92.0 * sc);
            let ready = info.ability_meter >= 1.0;
            let blink = (ctx.input(|i| i.time) * 4.0) as i64 % 2 == 0;
            let col = if info.ability_fx > 0.0 { egui::Color32::from_rgb(120, 220, 255) } else if ready && blink { white } else { accent };
            arc(&overlay, c, 30.0 * sc, 38.0 * sc, -90f32.to_radians(), -90f32.to_radians() + std::f32::consts::TAU, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120));
            let frac = if info.ability_fx > 0.0 { info.ability_fx * info.ability_fx } else { info.ability_meter };
            if frac > 0.01 {
                arc(&overlay, c, 30.0 * sc, 38.0 * sc, -90f32.to_radians(), -90f32.to_radians() + frac.min(1.0) * std::f32::consts::TAU, col);
            }
            self.text(&overlay, c, egui::Align2::CENTER_CENTER, info.ability, 10.0 * sc, col);
            if info.draft > 0.15 || info.sst > 0.02 {
                // the slipstream turbo: a bar that fills in the stream; full, the nitrous button fires it
                let blue = egui::Color32::from_rgb(120, 220, 255);
                let full = info.sst_full;
                let flash = (ctx.input(|i| i.time) * 6.0) as i64 % 2 == 0;
                self.text(&overlay, c + egui::vec2(0.0, 52.0 * sc), egui::Align2::CENTER_CENTER, if full { "SLIPSTREAM TURBO" } else { "SLIPSTREAM" }, 10.0 * sc, if full && flash { white } else { blue });
                let bar = egui::Rect::from_center_size(c + egui::vec2(0.0, 66.0 * sc), egui::vec2(84.0 * sc, 5.0 * sc));
                overlay.rect_filled(bar, 1.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 140));
                let mut fill = bar;
                fill.set_width(bar.width() * info.sst.clamp(0.0, 1.0));
                overlay.rect_filled(fill, 1.0, if full && flash { white } else { blue });
            }
        }
        // ability flash: a cool wash over the screen while the effect runs
        if let Some((tint, blend)) = info.ability_tint {
            // the disc's tints (Zone blue, Agro red): the inner colour over the screen, the outer
            // one deepening towards the edges
            let c = |v: [f32; 4], k: f32| egui::Color32::from_rgba_unmultiplied((v[0] * 255.0) as u8, (v[1] * 255.0) as u8, (v[2] * 255.0) as u8, (v[3] * 255.0 * k * blend).min(255.0) as u8);
            // (a fan from the middle of the screen out to its border)
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(screen.center(), c(tint[0], 0.7));
            // (the ring runs along the screen's border, corners included)
            let per = 12usize;
            let n = per * 4;
            let corners = [screen.left_top(), screen.right_top(), screen.right_bottom(), screen.left_bottom()];
            for side in 0..4 {
                let (p0, p1) = (corners[side], corners[(side + 1) % 4]);
                for k in 0..per {
                    mesh.colored_vertex(p0 + (p1 - p0) * (k as f32 / per as f32), c(tint[1], 0.7));
                }
            }
            for k in 0..n {
                mesh.add_triangle(0, 1 + k as u32, 1 + ((k + 1) % n) as u32);
            }
            overlay.add(egui::Shape::mesh(mesh));
        } else if info.ability_fx > 0.0 {
            let a = (info.ability_fx * 46.0) as u8;
            overlay.rect_filled(screen, 0.0, egui::Color32::from_rgba_unmultiplied(60, 140, 255, a));
        }

        // ---- race start prompt ----
        if let Some(i) = self.race_here {
            if let Some(rd) = self.race_defs.get(i) {
                let c = egui::pos2(screen.center().x, screen.bottom() - 150.0 * sc);
                let bg = egui::Rect::from_center_size(c + egui::vec2(0.0, 14.0 * sc), egui::vec2(760.0 * sc, 84.0 * sc));
                overlay.rect_filled(bg, 0.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 170));
                self.text(&overlay, c, egui::Align2::CENTER_CENTER, &rd.title().to_uppercase().replace("_", " ").replace(" - ", "   "), 18.0 * sc, white);
                let laps = if rd.laps > 1 { format!("{} LAPS   ", rd.laps) } else { String::new() };
                let tag = if self.mission_races.contains(&i) { "CAREER EVENT      " } else { "" };
                self.text(&overlay, c + egui::vec2(0.0, 30.0 * sc), egui::Align2::CENTER_CENTER, &format!("{}{}{} OPPONENTS      ENTER  RACE", tag, laps, rd.opponents), 12.0 * sc, accent);
            }
        }

        // ---- police status (top centre); with the HUD movie the strip under the
        // cluster shows pull-over / pursuit / searching (arrest is still drawn here) ----
        if info.heat > 0 && !(flash && info.heat <= 3) {
            let blink = (ctx.input(|i| i.time) * 4.0) as i64 % 2 == 0;
            let red = egui::Color32::from_rgb(255, 60, 50);
            let blue = egui::Color32::from_rgb(80, 140, 255);
            // (the game's own HUD messages: cop_hud_msg_pullover / _pursuit / _searching, cop_arrest)
            let (key, fallback, col) = match info.heat {
                1 => ("cop_hud_msg_pullover", "PULL OVER", if blink { red } else { blue }),
                2 => ("cop_hud_msg_pursuit", "POLICE PURSUIT", if blink { red } else { blue }),
                3 => ("cop_hud_msg_searching", "POLICE SEARCHING", egui::Color32::from_rgb(255, 200, 60)),
                _ => ("cop_arrest", "ARRESTED", red),
            };
            let label_s = self.career.strings.get(key).map(|s| s.to_uppercase()).unwrap_or_else(|| fallback.to_string());
            let label = label_s.as_str();
            let top = egui::pos2(screen.center().x, screen.top() + 34.0 * sc);
            self.text(&overlay, top, egui::Align2::CENTER_TOP, label, 34.0 * sc, col);
            if info.heat >= 2 {
                self.text(&overlay, top + egui::vec2(0.0, 40.0 * sc), egui::Align2::CENTER_TOP, &format!("FINE ${}", info.fine), 22.0 * sc, white);
            }
            if info.heat == 3 {
                // search timer: bar empties as the cruisers give up
                let w = 220.0 * sc;
                let r = egui::Rect::from_min_size(egui::pos2(top.x - w * 0.5, top.y + 70.0 * sc), egui::vec2(w, 8.0 * sc));
                overlay.rect_filled(r, 2.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 150));
                let mut f = r;
                f.set_width(w * (1.0 - info.search).clamp(0.0, 1.0));
                overlay.rect_filled(f, 2.0, egui::Color32::from_rgb(255, 200, 60));
            }
        }

        // ---- speedometer / tachometer cluster (bottom right) ----
        // Laid out against the game itself (same spot, same frame size): the dark body with the
        // C-shaped meter ring and its bar running off to the right, the small tachometer
        // inside it, the LCD below - speed, gear, a row of red dashes - and the little
        // ability dial above the LCD.
        let gl = screen.right() - 433.0 * sc;
        let gt = screen.bottom() - 258.0 * sc;
        let gk = 1.44 * sc;
        let c = egui::pos2(gl + 64.0 * gk, gt + 64.0 * gk);
        let mph = info.speed_kmh.abs() / 1.609;
        if flash {
            // drawn by the movie
        } else if let Some(art) = art {
            let body = egui::Rect::from_min_size(egui::pos2(gl, gt), egui::vec2(256.0, 128.0) * gk);
            if let Some(t) = art.get("hud_0062") {
                image(&overlay.with_clip_rect(egui::Rect::from_min_max(body.min, egui::pos2(screen.right() - 122.0 * sc, body.bottom()))), t, body, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 150));
            }
            if let Some(t) = art.get("hud_0946") {
                image(&overlay.with_clip_rect(egui::Rect::from_min_max(body.min, egui::pos2(screen.right() - 122.0 * sc, body.bottom()))), t, body, egui::Color32::from_rgba_unmultiplied(200, 205, 205, 110));
            }
            // the meter: cyan, its segments emptying with the nitrous (full with none fitted)
            let cyan = egui::Color32::from_rgb(0, 222, 255);
            if let Some(t) = art.get("hud_0440") {
                let fill = if info.nitro > 0.001 { info.nitro.clamp(0.0, 1.0) } else { 1.0 };
                let end = screen.right() - 122.0 * sc;
                image(&overlay.with_clip_rect(egui::Rect::from_min_max(body.min, egui::pos2(end, body.bottom()))), t, body, egui::Color32::from_rgb(0, 96, 108));
                let clip = egui::Rect::from_min_max(body.min, egui::pos2(body.left() + (end - body.left()) * (0.25 + 0.75 * fill), body.bottom()));
                image(&overlay.with_clip_rect(clip), t, body, cyan);
            }
            // tachometer: the 0..10 dial, redline, an orange needle
            let dk = 1.16 * sc;
            if let Some(t) = art.get("hud_0159") {
                image(&overlay, t, egui::Rect::from_min_size(c - egui::vec2(64.0, 70.0) * dk, egui::vec2(128.0, 128.0) * dk), egui::Color32::from_rgba_unmultiplied(255, 255, 255, 235));
            }
            let frac = (info.rpm / 10000.0).clamp(0.0, 1.0);
            let red0 = (info.redline.max(1000.0) / 10000.0).clamp(0.0, 1.0);
            arc(&overlay, c, 59.0 * sc, 63.0 * sc, dial_angle(red0), dial_angle(1.0), egui::Color32::from_rgb(215, 40, 36));
            let na = dial_angle(frac);
            let nd = egui::vec2(na.cos(), na.sin());
            overlay.line_segment([c + nd * 14.0 * sc, c + nd * 60.0 * sc], egui::Stroke::new(2.6 * sc, egui::Color32::from_rgb(255, 150, 20)));
            // the LCD
            let lcd = egui::Rect::from_min_size(egui::pos2(screen.right() - 374.0 * sc, screen.bottom() - 169.0 * sc), egui::vec2(250.0, 72.0) * sc);
            let lcd_img = egui::Rect::from_min_size(lcd.min - egui::vec2(14.0 * lcd.width() / 224.0, 31.0 * lcd.height() / 67.0), egui::vec2(256.0 * lcd.width() / 224.0, 128.0 * lcd.height() / 67.0));
            if let Some(t) = art.get("hud_1081") {
                image(&overlay, t, lcd_img, egui::Color32::from_rgb(168, 222, 208));
            } else {
                overlay.rect_filled(lcd, 6.0 * sc, egui::Color32::from_rgb(120, 170, 160));
            }
            let ink = egui::Color32::from_rgb(12, 26, 24);
            let row = lcd.top() + lcd.height() * 0.27;
            let txt = format!("{:.0}", mph);
            let sx = lcd.left() + lcd.width() * 0.46;
            if let Some(f) = art.digits.as_ref() {
                f.draw_right(&overlay, &txt, sx, row - 9.0 * sc, 26.0 * sc, ink);
            } else {
                overlay.text(egui::pos2(sx, row), egui::Align2::RIGHT_CENTER, &txt, egui::FontId::proportional(22.0 * sc), ink);
            }
            self.text(&overlay, egui::pos2(sx + 3.0 * sc, row + 6.0 * sc), egui::Align2::LEFT_CENTER, "mph", 7.0 * sc, ink);
            let gear = if info.gear < 0 { "R".to_string() } else if info.gear == 0 { "N".to_string() } else { info.gear.to_string() };
            let gx = lcd.left() + lcd.width() * 0.66;
            overlay.line_segment([egui::pos2(lcd.left() + lcd.width() * 0.575, row - 12.0 * sc), egui::pos2(lcd.left() + lcd.width() * 0.555, row + 12.0 * sc)], egui::Stroke::new(1.0 * sc, ink));
            if let Some(f) = art.digits.as_ref() {
                f.draw_right(&overlay, &gear, gx, row - 9.0 * sc, 26.0 * sc, ink);
            } else {
                overlay.text(egui::pos2(gx, row), egui::Align2::RIGHT_CENTER, &gear, egui::FontId::proportional(22.0 * sc), ink);
            }
            // (the cruise assist's speed in its box)
            if info.cruise {
                let bx = egui::Rect::from_min_max(egui::pos2(lcd.left() + lcd.width() * 0.70, row - 10.0 * sc), egui::pos2(lcd.right() - 10.0 * sc, row + 10.0 * sc));
                overlay.rect_filled(bx, 2.0 * sc, egui::Color32::from_rgb(60, 110, 100));
                self.text(&overlay, bx.center(), egui::Align2::CENTER_CENTER, "40mph", 9.0 * sc, egui::Color32::from_rgb(190, 240, 228));
            }
            // the dashes between the LCD's rows: red, one lit for each fifth of the damage taken
            let lit = (info.damage.clamp(0.0, 1.0) * 5.0).ceil() as usize;
            for k in 0..5 {
                let x = lcd.left() + lcd.width() * (0.20 + 0.14 * k as f32);
                let y = lcd.top() + lcd.height() * 0.50;
                overlay.rect_filled(egui::Rect::from_min_size(egui::pos2(x, y - 1.5 * sc), egui::vec2(lcd.width() * 0.10, 3.0 * sc)), 0.0, if k < lit { egui::Color32::from_rgb(255, 40, 30) } else { egui::Color32::from_rgb(150, 16, 20) });
            }
            // the ability dial
            let ac = egui::pos2(screen.right() - 197.0 * sc, screen.bottom() - 208.0 * sc);
            let ar = egui::Rect::from_center_size(ac, egui::vec2(62.0, 62.0) * sc);
            if let Some(t) = art.get("hud_0307") {
                image(&overlay, t, ar, if info.sst_full { cyan } else { egui::Color32::from_rgba_unmultiplied(0, 222, 255, (70.0 + 185.0 * info.sst.clamp(0.0, 1.0)) as u8) });
            }
            if let Some(t) = art.get("hud_0314") {
                image(&overlay, t, ar, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 220));
            }
        } else {
            overlay.text(c + egui::vec2(46.0, 0.0) * sc, egui::Align2::RIGHT_CENTER, format!("{:.0} MPH", mph), egui::FontId::proportional(48.0 * sc), white);
        }

        // ---- race info (top centre); the HUD movie shows clock, position, laps,
        // the delivery/payback meters and the countdown itself ----
        if let Some(race) = self.race.as_ref().filter(|_| !flash) {
            let (pos, total) = self.player_position(vehicles);
            let next = race.next.first().copied().unwrap_or(0);
            let cx = screen.center().x;
            let top = screen.top() + 18.0 * sc;
            self.panel(&overlay, egui::Rect::from_center_size(egui::pos2(cx, top + 44.0 * sc), egui::vec2(520.0 * sc, 100.0 * sc)), sc);
            let head = match race.kind {
                RaceKind::Race => format!("{} / {}", ordinal(pos).to_uppercase(), total),
                RaceKind::Delivery => "DELIVERY".to_string(),
                RaceKind::Payback => "PAYBACK".to_string(),
            };
            self.text(&overlay, egui::pos2(cx, top + 8.0 * sc), egui::Align2::CENTER_TOP, &head, if race.kind == RaceKind::Race { 30.0 } else { 20.0 } * sc, white);
            let lap_len = race.lap_len.max(1);
            let lap = (next / lap_len + 1).min(race.laps.max(1));
            let in_lap = if next >= race.checkpoints.len() { lap_len } else { next % lap_len };
            let lap_txt = if race.laps > 1 { format!("LAP {}/{}    ", lap, race.laps) } else { String::new() };
            if race.kind == RaceKind::Race {
                self.text(&overlay, egui::pos2(cx, top + 58.0 * sc), egui::Align2::CENTER_TOP, &format!("{}CHECKPOINT {}/{}    {}", lap_txt, in_lap, lap_len, fmt_time(race.time)), 13.0 * sc, egui::Color32::from_gray(225));
            } else {
                // missions: time left, and the damage that matters
                let left = (race.time_limit - race.time).max(0.0);
                let urgent = race.time_limit > 0.0 && left < 10.0;
                let tcol = if urgent { egui::Color32::from_rgb(255, 70, 50) } else { egui::Color32::from_gray(235) };
                if race.time_limit > 0.0 {
                    self.text(&overlay, egui::pos2(cx, top + 40.0 * sc), egui::Align2::CENTER_TOP, &fmt_time(left), 24.0 * sc, tcol);
                }
                let (label, dmg) = match race.kind {
                    RaceKind::Delivery => ("VEHICLE DAMAGE", info.damage),
                    _ => ("TARGET DAMAGE", race.target.and_then(|t| vehicles.cars.get(t)).map_or(0.0, |c| c.state.damage)),
                };
                let w = 260.0 * sc;
                // damage bar at the left edge of the plate, clear of the direction arrow
                let cx = cx - 420.0 * sc;
                let top = top - 60.0 * sc;
                self.panel(&overlay, egui::Rect::from_center_size(egui::pos2(cx, top + 100.0 * sc), egui::vec2(300.0 * sc, 56.0 * sc)), sc);
                let bar = egui::Rect::from_min_size(egui::pos2(cx - w * 0.5, top + 104.0 * sc), egui::vec2(w, 10.0 * sc));
                overlay.rect_filled(bar, 3.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160));
                let mut f = bar;
                f.set_width(w * dmg.clamp(0.0, 1.0));
                overlay.rect_filled(f, 3.0, if race.kind == RaceKind::Delivery { egui::Color32::from_rgb(255, 70, 50) } else { accent });
                self.text(&overlay, egui::pos2(cx, top + 86.0 * sc), egui::Align2::CENTER_TOP, label, 10.0 * sc, egui::Color32::from_gray(200));
            }
            if race.countdown > 0.0 {
                let n = race.countdown.ceil() as i32;
                let t = if n > 3 { "READY".to_string() } else { n.to_string() };
                self.text(&overlay, egui::pos2(cx, screen.center().y - 150.0 * sc), egui::Align2::CENTER_CENTER, &t, 90.0 * sc, accent);
                self.text(&overlay, egui::pos2(cx, screen.center().y - 60.0 * sc), egui::Align2::CENTER_CENTER, &race.name.to_uppercase().replace('_', " "), 14.0 * sc, white);
            } else if race.time < 1.5 {
                self.text(&overlay, egui::pos2(cx, screen.center().y - 150.0 * sc), egui::Align2::CENTER_CENTER, "GO!", 90.0 * sc, accent);
            }
        }

        let p = info.player_pos;
        let heading = info.player_heading;
        // ---- minimap (bottom left, circular) ----
        // (with `hudmap.rs` the map is the movie's `renderMe`: none of this is drawn)
        'old_map: {
        // (and the big map is the original's overhead map in the hudmap movie: the port's larger
        // copy of its own drawing only with MCLA_OLD_BIGMAP=1)
        if self.hm.is_some() && self.flash_map.is_some() && (!self.hud_map_big || !crate::hudmap::old_bigmap()) {
            break 'old_map;
        }
        // (the HUD view toggle: the map at twice the reach, drawn larger)
        // (the original's map covers the ring up to its outer edge: 94 pixels at 720p)
        let r = if self.hud_map_big { 180.0 } else if self.flash_map.is_some() { 117.5 } else { 115.0 } * sc;
        let mc = egui::pos2(screen.left() + 124.0 * sc + r, screen.bottom() - 71.0 * sc - r);
        let rect = egui::Rect::from_center_size(mc, egui::vec2(2.0 * r, 2.0 * r));
        let painter = overlay.with_clip_rect(rect.expand(4.0));
        // (a dark disc behind the map; the art that was used here is a square plate and showed
        // as a box around the dial)
        // The original map is a camera straight above the player (see `FlashMinimap`): what
        // it shows is its height times tan(35/2 degrees) either side over half the viewport,
        // and the player stands below the middle because it looks 110 m ahead.
        let k720 = screen.height() / 720.0;
        let mini = self.flash_map.as_ref().filter(|_| !self.hud_map_big);
        let (scale, pc) = match mini {
            Some(m) => {
                let ppm = m.viewport[3] * 0.5 * k720 / (m.cam_height * 17.5f32.to_radians().tan());
                (ppm, mc + egui::vec2(0.0, m.look_ahead * ppm))
            }
            None => (r / if self.hud_map_big { 620.0 } else { 330.0 }, mc),
        };
        // (reference capture at night: the ground reads 72, 84, 72 over a dark scene)
        painter.circle_filled(mc, r, if mini.is_some() { crate::flash_ui::plain_color(72, 85, 73, 235) } else { egui::Color32::from_rgba_unmultiplied(74, 88, 98, 235) });
        let p = info.player_pos;
        let heading = info.player_heading;
        let to_map = |w: Vec3| -> egui::Pos2 {
            let dx = w.x - p.x;
            let dz = w.z - p.z;
            let (s, c) = (-heading).sin_cos();
            let rx = dx * c - dz * s;
            let rz = dx * s + dz * c;
            egui::pos2(pc.x + rx * scale, pc.y + rz * scale)
        };
        let clip = |a: egui::Pos2, b: egui::Pos2| -> Option<(egui::Pos2, egui::Pos2)> { clip_to_circle(a, b, mc, r - 2.0) };
        if let (Some(roads), Some(_)) = (self.road_map.as_ref(), mini) {
            // the original's map: its five road meshes, each in its HudMap colour (see mapmesh)
            let reach = (r + 4.0) / scale + 40.0;
            let centre = {
                // the world point under the middle of the disc (the player stands below it)
                let d = (mc - pc) / scale;
                let (s, c) = heading.sin_cos();
                glam::Vec2::new(p.x + d.x * c - d.y * s, p.z + d.x * s + d.y * c)
            };
            let mut ids: Vec<u32> = Vec::new();
            let mut poly: Vec<egui::Pos2> = Vec::with_capacity(12);
            let rim = r - 1.5 * k720;
            for (layer, (_, key, or)) in roads.layers.iter().zip(crate::mapmesh::LAYERS) {
                let v = self.hudmap.get(key);
                let ch = |i: usize| v.and_then(|v| v.get(i)).map_or(or[i], |x| (x.clamp(0.0, 1.0) * 255.0) as u8);
                // (approx.) measured on the reference, the shortcut layer (alpha 0.745 in the
                // tune) reads as 0.44 over the ground: 0.59 of its own value
                let a = ch(3);
                let col = crate::flash_ui::plain_color(ch(0), ch(1), ch(2), if a < 255 { (a as f32 * 0.59) as u8 } else { a });
                let mut mesh = egui::Mesh::default();
                layer.near(centre, reach, &mut ids);
                for &i in &ids {
                    let t = layer.tris[i as usize];
                    let q = [to_map(Vec3::new(t[0].x, 0.0, t[0].y)), to_map(Vec3::new(t[1].x, 0.0, t[1].y)), to_map(Vec3::new(t[2].x, 0.0, t[2].y))];
                    let far = |a: egui::Pos2| a.distance(mc) > rim;
                    poly.clear();
                    poly.extend_from_slice(&q);
                    if far(q[0]) || far(q[1]) || far(q[2]) {
                        if far(q[0]) && far(q[1]) && far(q[2]) && q.iter().all(|a| a.distance(mc) > rim + 400.0 * scale) {
                            continue;
                        }
                        crate::mapmesh::clip_to_disc(&mut poly, mc, rim, 48);
                        if poly.is_empty() {
                            continue;
                        }
                    }
                    let base = mesh.vertices.len() as u32;
                    for a in &poly {
                        mesh.vertices.push(egui::epaint::Vertex { pos: *a, uv: egui::epaint::WHITE_UV, color: col });
                    }
                    for k in 1..poly.len() as u32 - 1 {
                        mesh.indices.extend([base, base + k, base + k + 1]);
                    }
                }
                if !mesh.indices.is_empty() {
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
        } else if let Some(nav) = world.nav.as_ref() {
            // roads in the map's own colour (HudMap MainRoadColor); on the reference they are
            // about 5.5 pixels wide at 720p whatever the zoom
            let reach = if mini.is_some() { (r + 12.0) / scale + 60.0 } else if self.hud_map_big { 560.0 } else { 300.0 };
            let (road, width) = if mini.is_some() { (self.map_color("MainRoadColor", [24, 30, 33]), 5.5 * k720) } else { (egui::Color32::from_rgb(34, 40, 50), 4.0 * sc) };
            for li in nav.links_near(p, reach) {
                let l = &nav.links[li];
                if let Some((a, b)) = clip(to_map(l.a), to_map(l.b)) {
                    painter.line_segment([a, b], egui::Stroke::new(width, road));
                    // (round joints, or the wide strokes leave notches at every bend)
                    painter.circle_filled(a, width * 0.5, road);
                }
            }
        }
        // (the original draws no line to the GPS target, on neither map: the ring's arrow
        // leads - `ng_tut_minimap`, `ng_gps_1` with a target set)
        if let (None, Some(path), true) = (self.race.as_ref(), self.gps.as_ref(), self.flash_map.is_none()) {
            // GPS line to the career's target
            for w in path.windows(2) {
                if w[0].distance(p) < 620.0 || w[1].distance(p) < 620.0 {
                    if let Some((a, b)) = clip(to_map(w[0]), to_map(w[1])) {
                        painter.line_segment([a, b], egui::Stroke::new(3.0 * sc, egui::Color32::from_rgb(90, 200, 255)));
                    }
                }
            }
        }
        if let Some(path) = self.race.as_ref().and_then(|r| r.path.as_ref()) {
            for w in path.windows(2) {
                if w[0].distance(p) < 620.0 || w[1].distance(p) < 620.0 {
                    if let Some((a, b)) = clip(to_map(w[0]), to_map(w[1])) {
                        painter.line_segment([a, b], egui::Stroke::new(3.5 * sc, accent));
                    }
                }
            }
        }
        // (the original's race map - reference `t10`, `t11` - shows the next checkpoints, a
        // thin green line from the player to the next one and the ring's arrow; the port's
        // own map draws the whole route instead)
        if let Some(race) = self.race.as_ref() {
            for w in race.route.windows(2).filter(|_| mini.is_none()) {
                if let Some(nav) = world.nav.as_ref() {
                    if let Some((a, b)) = clip(to_map(nav.links[w[0]].a), to_map(nav.links[w[1]].a)) {
                        painter.line_segment([a, b], egui::Stroke::new(3.5 * sc, accent));
                    }
                }
            }
            let next = race.next.first().copied().unwrap_or(0);
            if let (true, Some(cp)) = (mini.is_some(), race.checkpoints.get(next)) {
                if let Some((a, b)) = clip(to_map(info.player_pos), to_map(*cp)) {
                    painter.line_segment([a, b], egui::Stroke::new(2.0 * sc, egui::Color32::from_rgb(120, 220, 40)));
                }
            }
            // (the map's tuning: the next MaxCheckpointsInMinimap checkpoints, in its colours -
            // the next one, the one after, the end of a lap, the finish; an unordered race shows
            // all that are left and its finish only at the end)
            let n = race.checkpoints.len();
            let unordered = race.rivals_checkpoints.is_some() && next + 1 < n;
            let shown = if unordered { n } else { self.hudmap.get("MaxCheckpointsInMinimap").and_then(|v| v.first().copied()).unwrap_or(2.0).max(1.0) as usize };
            for (i, cp) in race.checkpoints.iter().enumerate().skip(next).take(shown) {
                if unordered && i + 1 == n {
                    continue;
                }
                let q = to_map(*cp);
                let off = q - mc;
                // (targets outside the map sit on its rim)
                let q = if off.length() < r - 6.0 { q } else if i == next || unordered { mc + off.normalized() * (r - 6.0) } else { continue };
                let col = if i + 1 == n {
                    self.map_color("CheckpointFinishColor", [214, 15, 15])
                } else if race.laps > 1 && (i + 1) % race.lap_len.max(1) == 0 {
                    self.map_color("CheckpointLapColor", [77, 204, 0])
                } else if i == next || unordered {
                    self.map_color("CheckpointNextColor", [255, 240, 23])
                } else {
                    self.map_color("CheckpointNextNextColor", [255, 244, 97])
                };
                painter.circle_filled(q, if i == next || unordered { 6.0 } else { 4.0 } * sc, col);
                painter.circle_stroke(q, if i == next || unordered { 6.0 } else { 4.0 } * sc, egui::Stroke::new(1.0 * sc, egui::Color32::BLACK));
                // (the next one wears a ring of its colour)
                if mini.is_some() && i == next {
                    painter.circle_stroke(q, 11.0 * sc, egui::Stroke::new(1.5 * sc, col.gamma_multiply(0.8)));
                }
            }
        }
        // service stations and garages in reach
        let in_race = self.race.is_some() && mini.is_some();
        for g in &self.gas_stations {
            let q = to_map(*g);
            if q.distance(mc) < r - 10.0 && !in_race {
                self.map_icon(&painter, "icon_gasstation", q, 17.0 * sc, self.map_color("GasStationColor", [128, 128, 128]));
            }
        }
        for (mn, mx) in world.set_bounds.values() {
            let centre = (*mn + *mx) * 0.5;
            let q = to_map(self.garage_doors.iter().copied().find(|d| d.distance(centre) < 80.0).unwrap_or(centre));
            if q.distance(mc) < r - 10.0 && !in_race {
                if mini.is_some() && self.flash_map.is_some() {
                    // (the wrench in a white frame - `s11/rk103_c.png`, the frame behind the car
                    // outside the Hollywood garage)
                    if self.career.loaded() && !self.career.reached("GARAGE_HOLLYWOOD") {
                        continue;
                    }
                    let h = 11.0 * k720;
                let r = egui::Rect::from_center_size(q, egui::vec2(2.0 * h, 2.0 * h));
                overlay.rect_filled(r, h * 0.18, egui::Color32::from_rgba_unmultiplied(12, 12, 12, 230));
                let (l, w) = (h * 0.62, h * 0.2);
                for (cx, cy, sx, sy) in [(r.left(), r.top(), 1.0, 1.0), (r.right(), r.top(), -1.0, 1.0), (r.left(), r.bottom(), 1.0, -1.0), (r.right(), r.bottom(), -1.0, -1.0)] {
                    overlay.line_segment([egui::pos2(cx, cy), egui::pos2(cx + sx * l, cy)], egui::Stroke::new(w, egui::Color32::WHITE));
                    overlay.line_segment([egui::pos2(cx, cy), egui::pos2(cx, cy + sy * l)], egui::Stroke::new(w, egui::Color32::WHITE));
                }
                    self.map_icon(&overlay, "icon_garage", q, 19.0 * k720, egui::Color32::WHITE);
                    continue;
                }
                self.map_icon(&painter, "icon_garage", q, 22.0 * sc, self.map_color("GarageColor", [255, 255, 255]));
            }
        }
        for cinst in &vehicles.cars {
            let q = to_map(cinst.state.pos);
            if q.distance(mc) < r - 4.0 {
                if cinst.is_player {
                    match art.and_then(|a| a.get("hudmap_0087")) {
                        Some(t) => image_rot(&painter, t, q, egui::vec2(22.0, 22.0) * sc, 0.0, white),
                        None => {
                            painter.circle_filled(q, 5.0 * sc, white);
                        }
                    }
                } else if cinst.racer {
                    // roaming racer: can be challenged
                    // the hookman a mission is about stands out
                    let named = !cinst.driver.is_empty();
                    // the icon is coloured by how hard the racer is for the player's car
                    let dc = crate::rewards::DIFF_COLORS[self.mc4_hookman_diff().filter(|_| named).unwrap_or(self.difficulty_of(&cinst.model.name).clamp(0, 3) as usize)];
                    let accent = egui::Color32::from_rgb(dc[0], dc[1], dc[2]);
                    if mini.is_some() && self.flash_map.is_some() {
                        // The original's racer on the minimap (`ng_chase_010` enlarged): the flag on a
                        // dark disc in a ring of his difficulty's colour, a wedge the way he heads.
                        // The mission's man set as the GPS target is yellow, with the mission's
                        // mark (`ng_tut_minimap`).
                        let target = named && self.mission_target.is_some();
                        let col = if target { egui::Color32::from_rgb(250, 235, 20) } else { accent };
                        let f = cinst.state.forward();
                        let d = to_map(cinst.state.pos + Vec3::new(f.x, 0.0, f.z) * 30.0) - q;
                        let dir = if d.length() > 1e-3 { d.normalized() } else { egui::vec2(0.0, -1.0) };
                        let side = egui::vec2(-dir.y, dir.x);
                        // (26 pixels across at 720p, the wedge to 21 from the middle)
                        let (r0, r1, hw) = (8.0 * k720, 21.0 * k720, 9.5 * k720);
                        overlay.add(egui::Shape::convex_polygon(vec![q + dir * r1, q + dir * r0 + side * hw, q + dir * r0 - side * hw], col, egui::Stroke::NONE));
                        overlay.circle_filled(q, 11.0 * k720, egui::Color32::from_rgb(12, 12, 12));
                        self.map_icon(&overlay, "icon_ring", q, 36.0 * k720, col);
                        if !self.map_icon(&overlay, "icon_ring", q, 33.5 * k720, col) {
                            overlay.circle_stroke(q, 12.0 * k720, egui::Stroke::new(3.2 * k720, col));
                        }
                        self.map_icon(&overlay, if target { "icon_ingame1" } else { "icon_ingame4" }, q, 32.0 * k720, egui::Color32::WHITE);
                        continue;
                    }
                    painter.circle_filled(q, if named { 7.0 } else { 5.5 } * sc, if named { white } else { accent });
                    painter.circle_stroke(q, if named { 7.0 } else { 5.5 } * sc, egui::Stroke::new(if named { 2.5 } else { 1.5 } * sc, if named { accent } else { white }));
                } else if cinst.cop {
                    let blink = (ctx.input(|i| i.time) * 5.0) as i64 % 2 == 0;
                    let col = if info.heat == 0 { self.map_color("CopColorCruising", [0, 68, 132]) } else if blink { self.map_color("CopColorFlashA", [255, 0, 0]) } else { self.map_color("CopColorFlashB", [66, 116, 255]) };
                    if mini.is_some() && self.flash_map.is_some() {
                        // (a cruiser is a dark blue arrow the way it heads - `s11/rk95_mm.png`)
                        let f = cinst.state.forward();
                        let d = to_map(cinst.state.pos + Vec3::new(f.x, 0.0, f.z) * 30.0) - q;
                        let dir = if d.length() > 1e-3 { d.normalized() } else { egui::vec2(0.0, -1.0) };
                        let side = egui::vec2(-dir.y, dir.x);
                        overlay.add(egui::Shape::convex_polygon(vec![q + dir * 9.0 * k720, q - dir * 6.0 * k720 + side * 6.5 * k720, q - dir * 6.0 * k720 - side * 6.5 * k720], col, egui::Stroke::NONE));
                    } else {
                        painter.circle_filled(q, 5.0 * sc, col);
                    }
                } else if (cinst.pinned && self.race.is_some()) || mini.is_none() || self.flash_map.is_none() {
                    // (the original's minimap does not show the traffic: `ng_chase_010`,
                    // `ng_tut_minimap`, `h_cruise_elevado`)
                    painter.circle_filled(q, 3.5 * sc, if cinst.pinned { egui::Color32::from_rgb(255, 80, 80) } else { egui::Color32::from_rgb(200, 200, 210) });
                }
            }
        }
        if let Some(fg) = self.flag.as_ref() {
            for (bp, col) in fg.hud().blips {
                let q = to_map(bp);
                let off = q - mc;
                let q = if off.length() < r - 8.0 { q } else { mc + off.normalized() * (r - 8.0) };
                painter.circle_filled(q, 6.0 * sc, egui::Color32::from_rgb(col[0], col[1], col[2]));
                painter.circle_stroke(q, 6.0 * sc, egui::Stroke::new(1.5 * sc, egui::Color32::BLACK));
            }
        }
        if let (Some(_), true) = (mini, self.race.is_none() && self.flag.is_none()) {
            // The original keeps every event in reach as a round marker, those beyond the
            // map on its rim (HudMap RimIconRadius), coloured by how hard they are.
            let rim = r - 1.0 * k720;
            // (a career marks what it has opened: a new one has no event on its map - `ng_chase_010`,
            // `ng_gps_1` - and a hookman's race is marked by the hookman, not by its start)
            let career = self.career.loaded() && self.flash_hud.is_some();
            let mm_events = if career { self.career.map_events() } else { Vec::new() };
            let id_ends = |i: usize, r: &str| self.race_defs[i].id.to_ascii_lowercase().ends_with(r);
            let hook = self.career.hookman_race().map(|r| r.to_ascii_lowercase());
            let early = career && !self.career.reached("GARAGE_HOLLYWOOD");
            let mut shown = 0;
            for (i, sp, _) in self.races_near(p, if career { 400 } else { 28 }).into_iter().filter(|(i, _, _)| !early || (self.map_races.contains(i) && !hook.as_ref().map_or(false, |h| self.race_defs[*i].id.to_ascii_lowercase().ends_with(h.as_str())))).take(if career { 400 } else { 28 }) {
                let q = to_map(sp);
                let off = q - mc;
                let inside = off.length() < rim - 12.0 * k720;
                let q = if off.length() < rim { q } else { mc + off.normalized() * rim };
                let dc = if self.mission_races.contains(&i) { [255, 255, 255] } else { crate::rewards::DIFF_COLORS[(i % 4).min(3)] };
                let col = egui::Color32::from_rgb(dc[0], dc[1], dc[2]);
                // The career's events are marked as on the GPS map (`s11/rk95_mm.png`, the 100 %
                // save beside a tournament): cup, red light, flag ... on a dark disc in a ring of
                // the difficulty's colour with a wedge the way the grid faces; the meeting
                // places in their frames.
                let style = mm_events.iter().find(|e| id_ends(i, &e.0)).map(|e| self.event_style(i, &e.1)).filter(|s| !Self::event_unmarked(s.0));
                // (a career marks its own events only)
                if career && (style.is_none() || shown >= 28) {
                    continue;
                }
                shown += 1;
                if let (true, Some((icon, None, ring))) = (inside, style) {
                    let yaw = self.race_defs[i].grid.first().map_or(0.0, |g| g.1);
                    let d = to_map(sp + Vec3::new(yaw.sin(), 0.0, -yaw.cos()) * 30.0) - q;
                    let dir = if d.length() > 1e-3 { d.normalized() } else { egui::vec2(0.0, -1.0) };
                    let side = egui::vec2(-dir.y, dir.x);
                    let (r0, r1, hw) = (8.0 * k720, 21.0 * k720, 9.5 * k720);
                    overlay.add(egui::Shape::convex_polygon(vec![q + dir * r1, q + dir * r0 + side * hw, q + dir * r0 - side * hw], ring, egui::Stroke::NONE));
                    overlay.circle_filled(q, 11.0 * k720, egui::Color32::from_rgb(12, 12, 12));
                    self.map_icon(&overlay, "icon_ring", q, 36.0 * k720, ring);
                    if !self.map_icon(&overlay, "icon_ring", q, 33.5 * k720, ring) {
                        overlay.circle_stroke(q, 12.0 * k720, egui::Stroke::new(3.2 * k720, ring));
                    }
                    self.map_icon(&overlay, icon, q, 32.0 * k720, egui::Color32::WHITE);
                    continue;
                }
                let (col, plate_icon) = match style {
                    Some((icon, Some(fc), _)) => (fc, icon),
                    Some((_, None, ring)) => (ring, Self::event_icon(&self.race_defs[i].sub_type)),
                    None => (col, Self::event_icon(&self.race_defs[i].sub_type)),
                };
                if inside {
                    // within the map an event is its icon on a dark plate in corner brackets of
                    // its colour (reference: the flags in a yellow frame)
                    let h = 11.0 * k720;
                    overlay.rect_filled(egui::Rect::from_center_size(q, egui::vec2(2.0 * h, 2.0 * h)), 2.0 * k720, crate::flash_ui::plain_color(20, 22, 20, 215));
                    let w = egui::Stroke::new(2.2 * k720, col);
                    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                        let c = q + egui::vec2(sx * h, sy * h);
                        overlay.line_segment([c, c - egui::vec2(sx * h * 0.6, 0.0)], w);
                        overlay.line_segment([c, c - egui::vec2(0.0, sy * h * 0.6)], w);
                    }
                    self.map_icon(&overlay, plate_icon, q, 17.0 * k720, egui::Color32::WHITE);
                    continue;
                }
                overlay.circle_filled(q, 5.5 * k720, egui::Color32::from_rgb(28, 30, 24));
                overlay.circle_filled(q, 4.5 * k720, col);
            }
            // places beyond the map (garages, red-light starts) stand on the rim in white
            // (HudMap GarageColor / HangoutColor)
            let far = |w: Vec3| { let off = to_map(w) - mc; (off.length() >= rim).then(|| mc + off.normalized() * rim) };
            if career && !early {
                for (cp, f, d) in self.cruiser_marks.iter().filter(|c| c.0.distance(p) < 1500.0 && !vehicles.cars.iter().any(|v| v.racer && v.state.pos.distance(c.0) < 150.0)) {
                    let q = to_map(*cp);
                    let dc = crate::rewards::DIFF_COLORS[(*d).min(3)];
                    let col = egui::Color32::from_rgb(dc[0], dc[1], dc[2]);
                    if (q - mc).length() < rim - 12.0 * k720 {
                        let dd = to_map(*cp + Vec3::new(f.x, 0.0, f.z) * 30.0) - q;
                        let dir = if dd.length() > 1e-3 { dd.normalized() } else { egui::vec2(0.0, -1.0) };
                        let side = egui::vec2(-dir.y, dir.x);
                        let (r0, r1, hw) = (8.0 * k720, 21.0 * k720, 9.5 * k720);
                        overlay.add(egui::Shape::convex_polygon(vec![q + dir * r1, q + dir * r0 + side * hw, q + dir * r0 - side * hw], col, egui::Stroke::NONE));
                        overlay.circle_filled(q, 11.0 * k720, egui::Color32::from_rgb(12, 12, 12));
                        self.map_icon(&overlay, "icon_ring", q, 36.0 * k720, col);
                        self.map_icon(&overlay, "icon_ring", q, 33.5 * k720, col);
                        self.map_icon(&overlay, "icon_ingame4", q, 32.0 * k720, egui::Color32::WHITE);
                    } else if let Some(q) = far(*cp) {
                        overlay.circle_filled(q, 5.5 * k720, egui::Color32::from_rgb(28, 30, 24));
                        overlay.circle_filled(q, 4.5 * k720, col);
                    }
                }
            }
            if career && !early {
                for (n, h) in self.hangouts.iter().enumerate().filter(|(_, h)| h.x.abs() >= 1.0 || h.z.abs() >= 1.0) {
                    let q = to_map(*h);
                    if (q - mc).length() < rim - 12.0 * k720 {
                        self.hangout_frame(&overlay, q, self.hangout_wager.get(n).copied().unwrap_or(false), k720 * 0.85);
                    } else if let (Some(q), true) = (far(*h), h.distance(p) < 1500.0) {
                        overlay.circle_filled(q, 5.5 * k720, egui::Color32::from_rgb(28, 30, 24));
                        overlay.circle_filled(q, 4.5 * k720, egui::Color32::WHITE);
                    }
                }
            }
            let places = world.set_bounds.values().map(|(mn, mx)| (*mn + *mx) * 0.5).chain(self.redlights.iter().copied());
            for wpos in places {
                if wpos.distance(p) < 1500.0 && !early {
                    if let Some(q) = far(wpos) {
                        overlay.circle_filled(q, 5.5 * k720, egui::Color32::from_rgb(28, 30, 24));
                        overlay.circle_filled(q, 4.5 * k720, egui::Color32::WHITE);
                    }
                }
            }
        } else if self.race.is_none() && self.flag.is_none() {
            // race starts: icons inside the map, the nearest one pinned to the rim
            for (k, (_, sp, d)) in self.races_near(p, 6).into_iter().enumerate() {
                let q = to_map(sp);
                let off = q - mc;
                let inside = off.length() < r - 8.0;
                if !inside && k > 0 {
                    continue;
                }
                let q = if inside { q } else { mc + off.normalized() * (r - 8.0) };
                let sz = if inside { 7.0 } else { 5.0 } * sc;
                let pts = vec![egui::pos2(q.x, q.y - sz), egui::pos2(q.x + sz, q.y), egui::pos2(q.x, q.y + sz), egui::pos2(q.x - sz, q.y)];
                painter.add(egui::Shape::convex_polygon(pts, if d < 16.0 { white } else { accent }, egui::Stroke::new(1.0, egui::Color32::BLACK)));
            }
        }
        // (the game's map has a thin dull rim, the event markers sitting on it - no bright ring)
        if self.flash_map.is_none() || self.hud_map_big {
            overlay.circle_stroke(mc, r, egui::Stroke::new(3.0 * sc, egui::Color32::from_rgb(92, 88, 62)));
        }
        // the player: a cyan arrow in a white frame at the middle (the map turns about it)
        if mini.is_some() {
            // the reference's marker: a dark plate in four white corner brackets, 24 pixels
            // across at 720p, with the arrow in HudMap HumanColor
            let h = 12.0 * k720;
            let w = egui::Stroke::new(2.4 * k720, egui::Color32::WHITE);
            // (the arrow stands alone, cruising too: `h_cruise_elevado`, `ng_tut_minimap`; the
            // frame once drawn here was a hangout's, the car standing on it - `h_hangout_fence`)
            let framed = false;
            if framed {
                overlay.rect_filled(egui::Rect::from_center_size(pc, egui::vec2(2.0 * h, 2.0 * h)), 2.0 * k720, egui::Color32::from_rgba_unmultiplied(20, 24, 24, 200));
            }
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].into_iter().filter(|_| framed) {
                let c = pc + egui::vec2(sx * h, sy * h);
                overlay.line_segment([c, c - egui::vec2(sx * h * 0.62, 0.0)], w);
                overlay.line_segment([c, c - egui::vec2(0.0, sy * h * 0.62)], w);
            }
            let a = 7.6 * k720;
            overlay.add(egui::Shape::convex_polygon(vec![pc + egui::vec2(0.0, -a), pc + egui::vec2(a * 0.72, a * 0.82), pc + egui::vec2(-a * 0.72, a * 0.82)], self.map_color("HumanColor", [115, 245, 255]), egui::Stroke::NONE));
        } else if !self.hud_map_big {
            let a = 8.0 * sc;
            overlay.rect_stroke(egui::Rect::from_center_size(mc, egui::vec2(2.6 * a, 2.6 * a)), 2.0 * sc, egui::Stroke::new(2.0 * sc, egui::Color32::WHITE), egui::StrokeKind::Inside);
            overlay.add(egui::Shape::convex_polygon(vec![mc + egui::vec2(0.0, -a), mc + egui::vec2(a * 0.75, a * 0.8), mc + egui::vec2(-a * 0.75, a * 0.8)], egui::Color32::from_rgb(60, 215, 255), egui::Stroke::NONE));
        }

        }
        // ---- flag game: score plate, what to do, arrow to it ----
        if let Some(fg) = self.flag.as_ref() {
            let h = fg.hud();
            let cx = screen.center().x;
            let top = screen.top() + 18.0 * sc;
            self.panel(&overlay, egui::Rect::from_center_size(egui::pos2(cx, top + 44.0 * sc), egui::vec2(620.0 * sc, 100.0 * sc)), sc);
            self.text(&overlay, egui::pos2(cx, top + 12.0 * sc), egui::Align2::CENTER_TOP, &h.score, 17.0 * sc, white);
            self.text(&overlay, egui::pos2(cx, top + 44.0 * sc), egui::Align2::CENTER_TOP, &h.title, 10.0 * sc, egui::Color32::from_gray(190));
            self.text(&overlay, egui::pos2(cx, top + 62.0 * sc), egui::Align2::CENTER_TOP, &h.objective, 13.0 * sc, accent);
            if h.countdown > 0.0 {
                let n = h.countdown.ceil() as i32;
                let t = if n > 3 { "READY".to_string() } else { n.to_string() };
                self.text(&overlay, egui::pos2(cx, screen.center().y - 150.0 * sc), egui::Align2::CENTER_CENTER, &t, 90.0 * sc, accent);
            } else if let (Some(tp), true) = (h.target, self.flash_map.is_none()) {
                let d = Vec3::new(tp.x - p.x, 0.0, tp.z - p.z);
                let ang = d.x.atan2(-d.z) - heading;
                let ac = egui::pos2(cx, screen.top() + 160.0 * sc);
                match art.and_then(|a| a.get("hudmap_0087")) {
                    Some(t) => image_rot(&overlay, t, ac, egui::vec2(70.0, 70.0) * sc, ang, accent),
                    None => {
                        let dir = egui::vec2(ang.sin(), -ang.cos());
                        overlay.line_segment([ac - dir * 20.0, ac + dir * 24.0], egui::Stroke::new(5.0 * sc, accent));
                    }
                }
                self.text(&overlay, ac + egui::vec2(0.0, 44.0 * sc), egui::Align2::CENTER_CENTER, &format!("{:.0} M", d.length()), 12.0 * sc, white);
            }
        }
        // ---- arrow to the next checkpoint (top centre) ----
        // (the port's own cue; the original has none there - its cue is the arrow on the
        // minimap's ring - so it goes when the minimap movie is in)
        if let (Some(race), true) = (self.race.as_ref(), self.flash_map.is_none()) {
            let next = race.next.first().copied().unwrap_or(0);
            if next < race.checkpoints.len() {
                let cp = race.checkpoints[next];
                let d = Vec3::new(cp.x - p.x, 0.0, cp.z - p.z);
                let dist = d.length();
                let ang = d.x.atan2(-d.z) - heading;
                let ac = egui::pos2(screen.center().x, screen.top() + 150.0 * sc);
                match art.and_then(|a| a.get("hudmap_0087")) {
                    Some(t) => image_rot(&overlay, t, ac, egui::vec2(70.0, 70.0) * sc, ang, accent),
                    None => {
                        let dir = egui::vec2(ang.sin(), -ang.cos());
                        let tip = ac + dir * 26.0;
                        let perp = egui::vec2(-dir.y, dir.x);
                        overlay.add(egui::Shape::convex_polygon(vec![tip, ac - dir * 14.0 + perp * 14.0, ac - dir * 6.0, ac - dir * 14.0 - perp * 14.0], accent, egui::Stroke::NONE));
                    }
                }
                self.text(&overlay, ac + egui::vec2(0.0, 44.0 * sc), egui::Align2::CENTER_CENTER, &format!("{:.0} M", dist), 12.0 * sc, white);
            }
        }
    }
}

/// Clip the segment a-b to the disc (c, r); None when fully outside.
impl Game {
    /// A colour of the map's tuning.
    fn map_color(&self, key: &str, or: [u8; 3]) -> egui::Color32 {
        match self.hudmap.get(key) {
            Some(v) if v.len() >= 3 => egui::Color32::from_rgb((v[0] * 255.0) as u8, (v[1] * 255.0) as u8, (v[2] * 255.0) as u8),
            _ => egui::Color32::from_rgb(or[0], or[1], or[2]),
        }
    }

    /// One of the map's icons (the art of the game's icon meshes) at `q`; false when it is not there.
    /// Events the maps do not mark by their races: a hookman's mission is marked by the man
    /// himself out on the road (the "!" of an active mission is his - none stands on the
    /// 100 % save's map, `s11/rk95_map.png`), and wagers and series belong to their hangouts.
    fn event_unmarked(icon: &str) -> bool {
        matches!(icon, "icon_ingame1" | "icon_ingame6" | "icon_series" | "icon_tournament_man")
    }

    /// A hangout's frame on the maps: the wager's "$" in white, the series' flags in yellow
    /// (`s11/rk95_map.png`, `z03`).
    fn hangout_frame(&self, painter: &egui::Painter, q: egui::Pos2, wager: bool, k: f32) {
        let fc = if wager { egui::Color32::WHITE } else { egui::Color32::from_rgb(250, 235, 20) };
        let r = egui::Rect::from_center_size(q, egui::vec2(32.0, 32.0) * k);
        painter.rect_filled(r, 3.0 * k, egui::Color32::from_rgb(12, 12, 12));
        let (l, w) = (10.0 * k, 3.4 * k);
        for (cx, cy, sx, sy) in [(r.left(), r.top(), 1.0, 1.0), (r.right(), r.top(), -1.0, 1.0), (r.left(), r.bottom(), 1.0, -1.0), (r.right(), r.bottom(), -1.0, -1.0)] {
            painter.line_segment([egui::pos2(cx, cy), egui::pos2(cx + sx * l, cy)], egui::Stroke::new(w, fc));
            painter.line_segment([egui::pos2(cx, cy), egui::pos2(cx, cy + sy * l)], egui::Stroke::new(w, fc));
        }
        self.map_icon(painter, if wager { "icon_ingame6" } else { "icon_series" }, q, 38.0 * k, egui::Color32::WHITE);
    }

    /// How the maps mark race `i` of mission `mission`: its icon, the colour of its frame
    /// if it stands in one (meeting places), and its ring's colour (the difficulty's).
    fn event_style(&self, i: usize, mission: &str) -> (&'static str, Option<egui::Color32>, egui::Color32) {
        let d = &self.race_defs[i];
        let (icon, framed) = if mission.contains("WAGER") {
            ("icon_ingame6", Some(egui::Color32::WHITE))
        } else if mission.contains("SERIES") {
            ("icon_series", Some(egui::Color32::from_rgb(250, 235, 20)))
        } else if mission.contains("PINKSLIP") {
            ("icon_pinkslip", Some(egui::Color32::WHITE))
        } else if mission.contains("TOURNAMENT") {
            // (a tournament is no place: its cup is the tournament man's car on the maps - map element
            // vehicle subtype 4, Racer_SetIconType(car, 7), `main.rs` map_men -, gone once it is won:
            // `game/CruisingHookmen/0xdcd7fe26`, career2 `series/notes.md` (c))
            ("icon_tournament_man", None)
        } else if mission.contains("CHAMP") {
            // (a cup in a ring with its pointer, like the races - `s11/rk95_mapz.png`)
            ("icon_ingame7", None)
        } else if mission.starts_with("TIMETRIAL") {
            ("icon_ingame5", None)
        } else if mission.starts_with("DELIVERY") || d.race_type == "Delivery" {
            // (the envelope in a white frame, at the garage the deliveries leave from; a payback
            // its car in a white frame - `s11/frames107.png`, enlarged from `rk95_map`)
            ("icon_delivery", Some(egui::Color32::WHITE))
        } else if mission.starts_with("PAYBACK") || d.race_type == "Payback" {
            ("icon_payback", Some(egui::Color32::WHITE))
        } else if mission.starts_with("REDLIGHT") {
            ("icon_ingame3", None)
        } else if mission.contains("FREEWAY") {
            ("icon_ingame8", None)
        } else if mission.starts_with("RACE_") || mission.starts_with("BEAT_") || mission.starts_with("BESTOF") {
            ("icon_ingame1", None)
        } else {
            ("icon_ingame4", None)
        };
        let by_rival = self.career.event_rival_diff(mission).or_else(|| self.career.event_rival_car(mission).map(|car| self.difficulty_of(&car).clamp(0, 3) as usize)).map(|k| { let c = crate::rewards::DIFF_COLORS[k]; egui::Color32::from_rgb(c[0], c[1], c[2]) });
        let col = if let Some(c) = by_rival { c } else { match d.opponents { 0..=2 => egui::Color32::from_rgb(120, 230, 30), 3..=4 => egui::Color32::from_rgb(250, 235, 20), 5..=6 => egui::Color32::from_rgb(250, 140, 20), _ => egui::Color32::from_rgb(230, 40, 30) } };
        (icon, framed, col)
    }

    fn map_icon(&self, painter: &egui::Painter, name: &str, q: egui::Pos2, size: f32, tint: egui::Color32) -> bool {
        match self.art.as_ref().and_then(|a| a.get(name)) {
            Some(t) => {
                crate::hud::image(painter, t, egui::Rect::from_center_size(q, egui::vec2(size, size)), tint);
                true
            }
            None => false,
        }
    }

    /// The race markers over the computer racers (racemark.rs; `sub_822272C8`): in a race with
    /// checkpoints, no flag game, not in a replay, while the HUD is up *(I: the gate
    /// `[0x8286D80C]+456` vfunc 23 is taken for "HUD shown")*; every racer of the race but the
    /// player whose car is in the world and whose icon is not hidden by its script (the payback
    /// client: type 10 cruising, 18 chased, hidden once the clock has run out).
    /// The free-roam icons over the three nearest hookmen / garages / hangouts / service stations
    /// (hookicons.rs; drawn with the race markers in the icon manager's queue, after them in the
    /// port - the queue is sorted far first and the two seldom meet).
    fn hook_icons(&mut self, ctx: &egui::Context, vehicles: &Vehicles, hud: &HudInfo) {
        if std::env::var("MCLA_NO_HOOK_ICONS").is_ok() || self.flag.is_some() || self.replay.is_some() || self.editor.is_some() || !matches!(self.mode, Mode::FreeRoam | Mode::Race) {
            return;
        }
        let Some(vp) = hud.cam_vp else { return };
        if self.hm.is_none() || self.ui_shared.is_none() {
            return;
        }
        let tune = self.mark_tune.get_or_insert_with(|| crate::racemark::MarkTune::load(&self.data_dir)).clone();
        let els = crate::hudmap::elements(self, vehicles, hud.heat >= 2);
        // (a police pursuit is a race to the icon manager: `[0x8286EBF8]`)
        let racing = self.race.is_some() || hud.heat >= 2;
        let checkpoints = self.race.as_ref().map_or(false, |r| !r.checkpoints.is_empty());
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if self.hook_icons.is_none() {
            self.hook_icons = crate::hookicons::HookIcons::new(&self.data_dir, self.ui_shared.as_ref().unwrap());
            if self.hook_icons.is_none() {
                log::warn!("hook icons: the 3dicons movie did not load");
                self.mark_tune = Some(tune);
                return;
            }
        }
        let hm = self.hm.as_mut().unwrap();
        let garage = hm.tune.c("GarageColor", [1.0; 4]);
        let gas = hm.tune.c("GasStationColor", [0.501961, 0.501961, 0.501961, 1.0]);
        let ripple = crate::hudmap::Ripple::new(&hm.tune, hm.t, 0.0);
        let hi = self.hook_icons.as_mut().unwrap();
        hi.update(&tune, &els, garage, gas, hud.cam_pos, hud.cam_look - hud.cam_pos, racing, checkpoints, dt);
        let draws = hi.draws();
        if draws.is_empty() {
            return;
        }
        let layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("hook_icons"));
        ctx.layer_painter(layer).add(crate::hudmap_gpu::hook_icons(ctx.screen_rect(), &mut hm.assets, draws, vp, Some(ripple)));
    }

    fn race_markers(&mut self, ctx: &egui::Context, vehicles: &Vehicles, hud: &HudInfo) {
        if std::env::var("MCLA_NO_RACE_MARKERS").is_ok() || self.flag.is_some() || self.replay.is_some() {
            return;
        }
        let (Some(race), Some(vp)) = (self.race.as_ref(), hud.cam_vp) else { return };
        if race.checkpoints.is_empty() {
            return;
        }
        let tune = self.mark_tune.get_or_insert_with(|| crate::racemark::MarkTune::load(&self.data_dir)).clone();
        let pb_phase = self.opt_x.arcade.pb.as_ref().map_or(0, |l| l.phase);
        let alpha = vehicles.render_alpha;
        let mut racers = Vec::new();
        for &ci in &race.participants {
            let Some(c) = vehicles.cars.get(ci) else { continue };
            if c.is_player || !c.placed {
                continue;
            }
            let icon = if race.kind == RaceKind::Payback && race.target == Some(ci) {
                match pb_phase {
                    0 => 10,
                    1 => 18,
                    _ => continue,
                }
            } else {
                0
            };
            let height = crate::racemark::bound_height(c);
            racers.push(crate::racemark::Racer { pos: c.render_pose(alpha).pos, height, icon });
        }
        if std::env::var("MCLA_MARK_LOG").is_ok() {
            if let Some(p) = vehicles.player() {
                crate::racemark::bound_height(p);
            }
        }
        let marks = crate::racemark::marks(&tune, hud.cam_pos, hud.cam_look - hud.cam_pos, &racers);
        if std::env::var("MCLA_MARK_LOG").is_ok() {
            for m in &marks {
                log::info!("race marker: {:.1} m at {:?} colour {:?}", m.dist, m.m.w_axis.truncate(), m.colour);
            }
        }
        if marks.is_empty() {
            return;
        }
        let Some(hm) = self.hm.as_mut() else { return };
        let layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("race_markers"));
        ctx.layer_painter(layer).add(crate::hudmap_gpu::race_marks(ctx.screen_rect(), &mut hm.assets, &marks, vp, tune.depth_test));
    }

    /// The icon of an event of this kind.
    fn event_icon(sub_type: &str) -> &'static str {
        match sub_type {
            "Blitz" => "icon_ingame5",
            "Delivery" => "icon_delivery",
            "Payback" => "icon_payback",
            "RedLight" => "icon_ingame3",
            _ => "icon_ingame4",
        }
    }

    /// Full-screen city map: the whole lane network, race starts, police and the player.
    fn city_map(&self, ctx: &egui::Context, vehicles: &Vehicles, world: &World, info: HudInfo) {
        let screen = ctx.screen_rect();
        let sc = (screen.height() / 900.0).clamp(0.5, 2.0);
        // (a layer egui has not been told the place of is drawn over those it has: name the
        // map's first, so that the mission log, moved to the top after it, covers it)
        ctx.move_to_top(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("city_map")));
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("city_map")));
        if info.map_vp.is_none() {
            painter.rect_filled(screen, 0.0, egui::Color32::from_rgb(5, 8, 13));
        } else {
            // over the city itself: a cool wash so that the markers stand out, bars behind the text
            // (the original's map has neither bars nor captions: a darker wash alone)
            let movie = self.flash_navsys.is_some() && self.editor.is_none();
            // (the original draws the city and its icons and nothing over them)
            if !(movie && self.hm.is_some()) {
                painter.rect_filled(screen, 0.0, if movie { egui::Color32::from_rgba_unmultiplied(0, 0, 0, 30) } else { egui::Color32::from_rgba_unmultiplied(8, 16, 34, 70) });
            }
            let bar = egui::Color32::from_rgba_unmultiplied(0, 0, 0, if movie { 0 } else { 150 });
            painter.rect_filled(egui::Rect::from_min_max(screen.left_top(), egui::pos2(screen.right(), screen.top() + 86.0 * sc)), 0.0, bar);
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(screen.left(), screen.bottom() - 64.0 * sc), screen.right_bottom()), 0.0, bar);
        }
        // (markers are drawn larger over the city)
        let sc_ui = sc;
        let sc = if info.map_vp.is_some() { sc * 1.5 } else { sc };
        let Some(nav) = world.nav.as_ref() else { return };
        // fit the network into the screen
        let mut mn = Vec3::splat(f32::INFINITY);
        let mut mx = Vec3::splat(f32::NEG_INFINITY);
        for l in nav.links.iter().step_by(7) {
            mn = mn.min(l.a);
            mx = mx.max(l.a);
        }
        let size = (mx - mn).max(Vec3::splat(1.0));
        let area = screen.shrink2(egui::vec2(70.0 * sc, 80.0 * sc));
        let k = (area.width() / size.x).min(area.height() / size.z);
        let org = egui::pos2(area.center().x - (mn.x + size.x * 0.5) * k, area.center().y - (mn.z + size.z * 0.5) * k);
        let to = |w: Vec3| match info.map_vp {
            // (over the city itself: through the map camera; behind it = off the screen)
            Some(vp) => {
                let c = vp * w.extend(1.0);
                if c.w <= 0.01 {
                    egui::pos2(-9999.0, -9999.0)
                } else {
                    egui::pos2(screen.left() + (c.x / c.w * 0.5 + 0.5) * screen.width(), screen.top() + (0.5 - c.y / c.w * 0.5) * screen.height())
                }
            }
            None => egui::pos2(org.x + w.x * k, org.y + w.z * k),
        };
        if info.map_vp.is_none() {
            let road = egui::Stroke::new(1.0, egui::Color32::from_rgb(95, 110, 135));
            for l in &nav.links {
                painter.line_segment([to(l.a), to(l.b)], road);
            }
        }
        if let (None, Some(path), true) = (self.race.as_ref(), self.gps.as_ref(), self.flash_navsys.is_none()) {
            for w in path.windows(2) {
                painter.line_segment([to(w[0]), to(w[1])], egui::Stroke::new(2.5 * sc, egui::Color32::from_rgb(90, 200, 255)));
            }
        }
        let accent = egui::Color32::from_rgb(255, 140, 0);
        // district names over the middle of their events
        let mut districts: Vec<(String, Vec3, f32)> = Vec::new();
        for i in 0..self.race_defs.len() {
            let Some(sp) = self.race_start(i) else { continue };
            let d = &self.race_defs[i].district;
            if d.is_empty() || d.eq_ignore_ascii_case("All") {
                continue;
            }
            match districts.iter_mut().find(|x| x.0 == *d) {
                Some(x) => {
                    x.1 += sp;
                    x.2 += 1.0;
                }
                None => districts.push((d.clone(), sp, 1.0)),
            }
        }
        for (name, sum, n) in districts.iter().filter(|_| !(self.flash_navsys.is_some() && self.editor.is_none())) {
            let label = self.career.strings.get(name).map(|s| s.to_string()).unwrap_or_else(|| name.to_uppercase());
            self.text(&painter, to(*sum / *n), egui::Align2::CENTER_CENTER, &label.to_uppercase(), 15.0 * sc, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 46));
        }
        // (with `hudmap.rs` the icons are the original's, drawn before this - none of the port's)
        let own = self.hm.is_some() && self.flash_navsys.is_some() && self.editor.is_none() && info.map_vp.is_some();
        let map_movie = self.flash_navsys.is_some() && self.editor.is_none();
        'port_icons: {
        if own {
            break 'port_icons;
        }
        let pulse = 1.0 + 0.25 * (ctx.input(|i| i.time) as f32 * 4.0).sin();
        let scale_of = |k: &str| self.hudmap.get(k).and_then(|v| v.first().copied()).unwrap_or(100.0) / 100.0;
        let map_movie = self.flash_navsys.is_some() && self.editor.is_none();
        // (the map's events as the career stands now - those open and those done - each with
        // the mission it belongs to)
        let map_events: Vec<(usize, String)> = if map_movie {
            let ev = self.career.map_events();
            (0..self.race_defs.len())
                .filter_map(|i| {
                    let id = self.race_defs[i].id.to_ascii_lowercase();
                    ev.iter().find(|(r, _)| id.ends_with(r.as_str())).map(|(_, m)| (i, m.clone()))
                })
                .collect()
        } else {
            Vec::new()
        };
        for i in 0..self.race_defs.len() {
            if let Some(sp) = self.race_start(i) {
                let q = to(sp);
                if map_movie {
                    // The original's marker (references `z00`..`z10`): the event's icon on a
                    // dark disc in a ring of its difficulty's colour - green, yellow, orange,
                    // red by the legend. Only the races the game names are places of the map.
                    // (approx.) The port has no difficulty per event: the colour goes by the
                    // number of opponents; a career event is ringed light blue.
                    let d = &self.race_defs[i];
                    // (the map shows the career's events that are open or done, not every race there is)
                    let Some(mission) = map_events.iter().find(|e| e.0 == i).map(|e| e.1.as_str()) else { continue };
                    // The marker by the kind of event (the legend's icons, `meshtextures`; reference
                    // `z00` enlarged): events held at a meeting place - wager, series, pink slips,
                    // tournament - stand in a square frame, white (the series' yellow); the others
                    // in a ring of the difficulty's colour with a pointer on it.
                    let (icon, framed, col) = self.event_style(i, mission);
                    if Self::event_unmarked(icon) {
                        continue;
                    }
                    let k = screen.height() / 720.0;
                    // (an event held at a meeting place is one place of the map, not one per race of it)
                    if (framed.is_some() || icon == "icon_ingame7") && map_events.iter().find(|e| e.1 == mission).map(|e| e.0) != Some(i) {
                        continue;
                    }
                    if let Some(fc) = framed {
                        // (a dark plate in four corner brackets)
                        let r = egui::Rect::from_center_size(q, egui::vec2(32.0, 32.0) * k);
                        painter.rect_filled(r, 3.0 * k, egui::Color32::from_rgb(12, 12, 12));
                        let (l, w) = (10.0 * k, 3.4 * k);
                        for (cx, cy, sx, sy) in [(r.left(), r.top(), 1.0, 1.0), (r.right(), r.top(), -1.0, 1.0), (r.left(), r.bottom(), 1.0, -1.0), (r.right(), r.bottom(), -1.0, -1.0)] {
                            painter.line_segment([egui::pos2(cx, cy), egui::pos2(cx + sx * l, cy)], egui::Stroke::new(w, fc));
                            painter.line_segment([egui::pos2(cx, cy), egui::pos2(cx, cy + sy * l)], egui::Stroke::new(w, fc));
                        }
                        self.map_icon(&painter, icon, q, 38.0 * k, egui::Color32::WHITE);
                        continue;
                    }
                    // The ring is the event's difficulty colour (the legend's: `ng_gps_3`) - how
                    // hard its racer's car is for the player's. (approx.) Events without a
                    // racer of their own go by the number of opponents.
                    // the pointer: a wedge on the ring, the way the event's grid faces (map north up)
                    let yaw = d.grid.first().map_or(0.0, |g| g.1);
                    let dir = egui::vec2(yaw.sin(), -yaw.cos());
                    let side = egui::vec2(-dir.y, dir.x);
                    let (r0, r1, hw) = (12.0 * k, 32.0 * k, 14.0 * k);
                    painter.add(egui::Shape::convex_polygon(vec![q + dir * r1, q + dir * r0 + side * hw, q + dir * r0 - side * hw], col, egui::Stroke::NONE));
                    painter.circle_filled(q, 15.5 * k, egui::Color32::from_rgb(12, 12, 12));
                    self.map_icon(&painter, "icon_ring", q, 42.5 * k, col);
                    if !self.map_icon(&painter, "icon_ring", q, 46.0 * k, col) {
                        painter.circle_stroke(q, 15.0 * k, egui::Stroke::new(4.0 * k, col));
                    }
                    self.map_icon(&painter, icon, q, 38.0 * k, egui::Color32::WHITE);
                    continue;
                }
                // career events are bigger, blue and pulse
                let mission = self.mission_races.contains(&i);
                let col = if mission { egui::Color32::from_rgb(90, 200, 255) } else { accent };
                if self.map_icon(&painter, Self::event_icon(&self.race_defs[i].sub_type), q, if mission { 34.0 * pulse } else { 22.0 } * sc, col) {
                    continue;
                }
                let sz = if mission { 9.0 * pulse } else { 4.0 } * sc;
                let pts = vec![egui::pos2(q.x, q.y - sz), egui::pos2(q.x + sz, q.y), egui::pos2(q.x, q.y + sz), egui::pos2(q.x - sz, q.y)];
                painter.add(egui::Shape::convex_polygon(pts, col, if mission { egui::Stroke::new(1.5 * sc, egui::Color32::WHITE) } else { egui::Stroke::new(1.0, egui::Color32::BLACK) }));
            }
        }
        // service stations
        let gas = self.map_color("GasStationColor", [128, 128, 128]);
        for g in &self.gas_stations {
            let q = to(*g);
            if !self.map_icon(&painter, "icon_gasstation", q, 24.0 * sc * scale_of("IconGasStationScale"), gas) {
                painter.circle_filled(q, 4.0 * sc, gas);
            }
        }
        // (a new career's map has the service stations and the mission's man, nothing else -
        // `ng_gps_1`; garages, red lights and hangouts come with the garage. approx.: the moment)
        let early_map = map_movie && self.career.loaded() && !self.career.reached("GARAGE_HOLLYWOOD");
        // garages
        let garage = self.map_color("GarageColor", [255, 255, 255]);
        for (mn, mx) in world.set_bounds.values().filter(|_| !early_map).filter(|(mn, mx)| self.mc4_setup_shown(&crate::mc4::marker_script(1, self.mc4_district_code((*mn + *mx) * 0.5)))) {
            let centre = (*mn + *mx) * 0.5;
            let q = to(self.garage_doors.iter().copied().find(|d| d.distance(centre) < 80.0).unwrap_or(centre));
            if map_movie {
                // (a garage is its icon in a white frame - `z03`, `rk95_map`: three of them, one
                // under the player's arrow where the 100 % save stands. It was read as a
                // delivery's envelope in step 95, and as the player's own frame.)
                let k = screen.height() / 720.0;
                let r = egui::Rect::from_center_size(q, egui::vec2(32.0, 32.0) * k);
                painter.rect_filled(r, 3.0 * k, egui::Color32::from_rgb(12, 12, 12));
                let (l, w) = (10.0 * k, 3.4 * k);
                for (cx, cy, sx, sy) in [(r.left(), r.top(), 1.0, 1.0), (r.right(), r.top(), -1.0, 1.0), (r.left(), r.bottom(), 1.0, -1.0), (r.right(), r.bottom(), -1.0, -1.0)] {
                    painter.line_segment([egui::pos2(cx, cy), egui::pos2(cx + sx * l, cy)], egui::Stroke::new(w, egui::Color32::WHITE));
                    painter.line_segment([egui::pos2(cx, cy), egui::pos2(cx, cy + sy * l)], egui::Stroke::new(w, egui::Color32::WHITE));
                }
                self.map_icon(&painter, "icon_garage", q, 30.0 * k, egui::Color32::WHITE);
                continue;
            }
            if !self.map_icon(&painter, "icon_garage", q, 24.0 * sc * scale_of("IconGarageScale"), garage) {
                painter.rect_filled(egui::Rect::from_center_size(q, egui::vec2(13.0, 13.0) * sc), 2.0, egui::Color32::from_rgb(120, 230, 120));
                self.text(&painter, q, egui::Align2::CENTER_CENTER, "G", 8.0 * sc, egui::Color32::BLACK);
            }
        }
        // red light races
        for h in self.redlights.iter().filter(|_| !early_map).filter(|h| self.mc4_setup_shown(&crate::mc4::marker_script(2, self.mc4_district_code(**h)))) {
            let q = to(*h);
            if !self.map_icon(&painter, "icon_ingame3", q, 22.0 * sc, egui::Color32::WHITE) {
                painter.circle_filled(q, 4.5 * sc, egui::Color32::from_rgb(255, 70, 60));
                painter.circle_stroke(q, 4.5 * sc, egui::Stroke::new(1.0 * sc, egui::Color32::WHITE));
            }
        }
        // hangouts
        let hang = self.map_color("HangoutColor", [255, 255, 255]);
        for (n, h) in self.hangouts.iter().enumerate().filter(|_| !early_map).filter(|(n, _)| self.hangout_names.get(*n).map_or(true, |s| self.mc4_hangout_open(s))) {
            let q = to(*h);
            // (a scene without a place of its own lies at the origin)
            if h.x.abs() < 1.0 && h.z.abs() < 1.0 {
                continue;
            }
            if map_movie {
                self.hangout_frame(&painter, q, self.hangout_wager.get(n).copied().unwrap_or(false), screen.height() / 720.0);
                continue;
            }
            if self.map_icon(&painter, "icon_ring", q, 24.0 * sc * scale_of("IconHangoutScale"), hang) {
                self.map_icon(&painter, "icon_hangout_circle", q, 14.0 * sc, hang);
            } else {
                painter.circle_filled(q, 7.0 * sc, egui::Color32::from_rgb(200, 120, 255));
                self.text(&painter, q, egui::Align2::CENTER_CENTER, "H", 8.0 * sc, egui::Color32::BLACK);
            }
        }
        // the districts' cruising racers (not those the roamers have put on the road nearby)
        if map_movie && !early_map {
            let k = screen.height() / 720.0;
            for (p, f, d) in self.cruiser_marks.iter().filter(|c| !vehicles.cars.iter().any(|v| v.racer && v.state.pos.distance(c.0) < 150.0)) {
                let q = to(*p);
                let dc = crate::rewards::DIFF_COLORS[(*d).min(3)];
                let col = egui::Color32::from_rgb(dc[0], dc[1], dc[2]);
                let dd = to(*p + Vec3::new(f.x, 0.0, f.z) * 30.0) - q;
                let dir = if dd.length() > 1e-3 { dd.normalized() } else { egui::vec2(0.0, -1.0) };
                let side = egui::vec2(-dir.y, dir.x);
                let (r0, r1, hw) = (12.0 * k, 32.0 * k, 14.0 * k);
                painter.add(egui::Shape::convex_polygon(vec![q + dir * r1, q + dir * r0 + side * hw, q + dir * r0 - side * hw], col, egui::Stroke::NONE));
                painter.circle_filled(q, 15.5 * k, egui::Color32::from_rgb(12, 12, 12));
                self.map_icon(&painter, "icon_ring", q, 42.5 * k, col);
                self.map_icon(&painter, "icon_ring", q, 46.0 * k, col);
                self.map_icon(&painter, "icon_ingame4", q, 38.0 * k, egui::Color32::WHITE);
            }
        }
        // the hookman a mission is about
        let racer = self.map_color("RacerColor", [135, 203, 0]);
        for c in vehicles.cars.iter().filter(|c| c.racer) {
            let q = to(c.state.pos);
            let named = !c.driver.is_empty();
            if map_movie {
                // The original's marker for a racer out on the road (reference `z00` enlarged): the
                // cruising racer's flag - a mission's "!" for the hookman a mission is about - on
                // a dark disc in a ring, with a pointer the way he is heading.
                // (approx.) the ring's colour: the port's racers carry no difficulty - green, the
                // hookman's yellow.
                let k = screen.height() / 720.0;
                // (the ring is his difficulty's colour - TREVOR green in `ng_gps_1`; yellow once
                // his mission is the GPS target)
                let dc = crate::rewards::DIFF_COLORS[self.mc4_hookman_diff().filter(|_| named).unwrap_or(self.difficulty_of(&c.model.name).clamp(0, 3) as usize)];
                let col = if named && self.mission_target.is_some() { egui::Color32::from_rgb(250, 235, 20) } else { egui::Color32::from_rgb(dc[0], dc[1], dc[2]) };
                let f = c.state.forward();
                // (the heading as the map shows it: a point ahead of him, through the map's own projection)
                let d = to(c.state.pos + Vec3::new(f.x, 0.0, f.z) * 30.0) - q;
                let dir = if d.length() > 1e-3 { d.normalized() } else { egui::vec2(0.0, -1.0) };
                let side = egui::vec2(-dir.y, dir.x);
                let (r0, r1, hw) = (12.0 * k, 32.0 * k, 14.0 * k);
                painter.add(egui::Shape::convex_polygon(vec![q + dir * r1, q + dir * r0 + side * hw, q + dir * r0 - side * hw], col, egui::Stroke::NONE));
                painter.circle_filled(q, 15.5 * k, egui::Color32::from_rgb(12, 12, 12));
                self.map_icon(&painter, "icon_ring", q, 42.5 * k, col);
                    if !self.map_icon(&painter, "icon_ring", q, 46.0 * k, col) {
                    painter.circle_stroke(q, 15.0 * k, egui::Stroke::new(4.0 * k, col));
                }
                // (the flag; the mission's mark once he is the target - `ng_gps_2`, `ng_gps_1`)
                self.map_icon(&painter, if named && self.mission_target.is_some() { "icon_ingame1" } else { "icon_ingame4" }, q, 38.0 * k, egui::Color32::WHITE);
                continue;
            }
            // (the racer icon is an arrow standing on its place)
            let sz = if named { 30.0 } else { 20.0 } * sc;
            if !self.map_icon(&painter, "icon_racer", q - egui::vec2(0.0, sz * 0.45), sz, if named { egui::Color32::WHITE } else { racer }) {
                painter.circle_filled(q, if named { 7.0 } else { 4.5 } * sc, if named { egui::Color32::WHITE } else { accent });
                painter.circle_stroke(q, if named { 7.0 } else { 4.5 } * sc, egui::Stroke::new(2.0 * sc, if named { accent } else { egui::Color32::WHITE }));
            }
            if named {
                self.text(&painter, q + egui::vec2(12.0 * sc, 0.0), egui::Align2::LEFT_CENTER, &self.career.name_of(&c.driver), 10.0 * sc, egui::Color32::WHITE);
            }
        }
        if let Some(race) = self.race.as_ref() {
            let cp_col = self.map_color("CheckpointNormalColor", [255, 244, 97]);
            for cp in &race.checkpoints {
                painter.circle_filled(to(*cp), 4.0 * sc, cp_col);
            }
        }
        for c in vehicles.cars.iter().filter(|c| c.cop) {
            painter.circle_filled(to(c.state.pos), 4.0 * sc, self.map_color("CopColorCruising", [0, 68, 132]));
            painter.circle_stroke(to(c.state.pos), 4.0 * sc, egui::Stroke::new(1.0 * sc, egui::Color32::WHITE));
        }
        }
        // race editor: the course so far and the cursor
        if let Some(pts) = self.editor.as_ref() {
            for w in pts.windows(2) {
                painter.line_segment([to(w[0]), to(w[1])], egui::Stroke::new(3.0 * sc, egui::Color32::from_rgb(255, 230, 60)));
            }
            for (k, p) in pts.iter().enumerate() {
                painter.circle_filled(to(*p), 7.0 * sc, if k == 0 { egui::Color32::from_rgb(90, 230, 90) } else { egui::Color32::from_rgb(255, 230, 60) });
                self.text(&painter, to(*p), egui::Align2::CENTER_CENTER, &(if k == 0 { "S".to_string() } else { k.to_string() }), 7.0 * sc, egui::Color32::BLACK);
            }
            let c = to(self.editor_cursor);
            painter.circle_stroke(c, 10.0 * sc, egui::Stroke::new(2.0 * sc, egui::Color32::WHITE));
            painter.line_segment([c - egui::vec2(16.0, 0.0) * sc, c + egui::vec2(16.0, 0.0) * sc], egui::Stroke::new(1.5 * sc, egui::Color32::WHITE));
            painter.line_segment([c - egui::vec2(0.0, 16.0) * sc, c + egui::vec2(0.0, 16.0) * sc], egui::Stroke::new(1.5 * sc, egui::Color32::WHITE));
            let len: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
            self.text(&painter, egui::pos2(screen.center().x, screen.top() + 40.0 * sc), egui::Align2::CENTER_TOP, &format!("RACE EDITOR    {} POINTS    {:.1} KM", pts.len(), len / 1000.0), 16.0 * sc, egui::Color32::from_rgb(255, 230, 60));
            self.text(&painter, egui::pos2(screen.center().x, screen.top() + 66.0 * sc), egui::Align2::CENTER_TOP, "ARROWS  MOVE      ENTER  PLACE (START FIRST)      BACKSPACE  UNDO      R  RACE IT      E  LEAVE", 10.0 * sc, egui::Color32::from_gray(200));
        }
        // player arrow
        if !own {
        let q = to(info.player_pos);
        if false && map_movie && self.map_cursor > 0 && self.mission_log.2 {
            // (the player stands in a white frame while the cursor is away - `z03`)
            let k = screen.height() / 720.0;
            let r = egui::Rect::from_center_size(q, egui::vec2(32.0, 32.0) * k);
            painter.rect_filled(r, 3.0 * k, egui::Color32::from_rgb(12, 12, 12));
            let (l, w) = (10.0 * k, 3.4 * k);
            for (cx, cy, sx, sy) in [(r.left(), r.top(), 1.0, 1.0), (r.right(), r.top(), -1.0, 1.0), (r.left(), r.bottom(), 1.0, -1.0), (r.right(), r.bottom(), -1.0, -1.0)] {
                painter.line_segment([egui::pos2(cx, cy), egui::pos2(cx + sx * l, cy)], egui::Stroke::new(w, egui::Color32::WHITE));
                painter.line_segment([egui::pos2(cx, cy), egui::pos2(cx, cy + sy * l)], egui::Stroke::new(w, egui::Color32::WHITE));
            }
        }
        let (s, c) = info.player_heading.sin_cos();
        let f = egui::vec2(s, -c);
        let rt = egui::vec2(c, s);
        let a = 11.0 * sc;
        painter.add(egui::Shape::convex_polygon(vec![q + f * a, q - f * a * 0.7 + rt * a * 0.6, q - f * a * 0.7 - rt * a * 0.6], self.map_color("HumanColor", [115, 245, 255]), egui::Stroke::new(1.0, egui::Color32::BLACK)));
        }
        // (the original's map carries no caption or key hints; they stay for the port's own map only)
        let painter = if self.flash_navsys.is_some() && self.editor.is_none() { painter.with_clip_rect(egui::Rect::NOTHING) } else { painter };
        self.text(&painter, egui::pos2(screen.left() + 50.0 * sc_ui, screen.top() + 34.0 * sc_ui), egui::Align2::LEFT_TOP, "LOS ANGELES", 26.0 * sc_ui, egui::Color32::WHITE);
        self.text(&painter, egui::pos2(screen.right() - 50.0 * sc_ui, screen.top() + 38.0 * sc_ui), egui::Align2::RIGHT_TOP, &format!("CASH ${}    REP {}", self.money, self.rep), 13.0 * sc_ui, egui::Color32::from_rgb(120, 230, 120));
        self.text(&painter, egui::pos2(screen.left() + 50.0 * sc_ui, screen.bottom() - 46.0 * sc_ui), egui::Align2::LEFT_TOP, "ARROWS  MOVE      Z / X  ZOOM      ORANGE  RACE EVENTS      BLUE  CAREER EVENTS      E  RACE EDITOR      M / ESC  CLOSE", 10.0 * sc_ui, egui::Color32::from_gray(150));
    }
}

fn clip_to_circle(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2, r: f32) -> Option<(egui::Pos2, egui::Pos2)> {
    let d = b - a;
    let f = a - c;
    let aa = d.dot(d);
    if aa < 1e-6 {
        return if f.length() < r { Some((a, b)) } else { None };
    }
    let bb = 2.0 * f.dot(d);
    let cc = f.dot(f) - r * r;
    let disc = bb * bb - 4.0 * aa * cc;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let t0 = ((-bb - sq) / (2.0 * aa)).max(0.0);
    let t1 = ((-bb + sq) / (2.0 * aa)).min(1.0);
    if t0 >= t1 {
        return None;
    }
    Some((a + d * t0, a + d * t1))
}


#[derive(Clone, Copy, Default)]
pub struct HudInfo {
    /// GOAL ATTACK's damage goal on the meter (mcGoalDamageCtrl, sub_826506F0: the HUD movie's
    /// `damage_goal` = the mode's +796 while the control is active)
    pub damage_goal: bool,
    pub speed_kmh: f32,
    /// the body's whole speed, m/s: the HUD's d_Speedvalue (|velocity|, sub_822112A8), not the forward speed
    pub speed_abs: f32,
    pub gear: i32,
    pub rpm: f32,
    pub redline: f32,
    pub nitro: f32,
    pub damage: f32,
    pub player_pos: Vec3,
    pub player_heading: f32,
    /// the map screen's camera when the city is shown from the air
    pub map_vp: Option<glam::Mat4>,
    /// the slipstream turbo's charge (0..1), and whether it can be fired
    pub sst: f32,
    pub sst_full: bool,
    /// the running ability's screen tint (inner and outer RGBA) and how far in it is
    pub ability_tint: Option<([[f32; 4]; 2], f32)>,
    /// police: 0 none, 1 pull over, 2 pursuit, 3 search, 4 busted
    pub heat: u8,
    pub fine: i64,
    /// law-breaking score 0..1 and search progress 0..1
    pub law: f32,
    pub search: f32,
    /// rain intensity 0..1
    pub rain: f32,
    /// the frame's camera, for what is drawn into the world from here (the rain): view
    /// projection, place, flat heading, speed; and whether it is night
    pub cam_vp: Option<glam::Mat4>,
    pub cam_pos: Vec3,
    /// the point the camera looks at (its forward is `cam_look - cam_pos`)
    pub cam_look: Vec3,
    pub cam_fwd: Vec3,
    pub cam_vel: Vec3,
    pub night: bool,
    /// the cruise assist is holding the speed limit
    pub cruise: bool,
    /// special ability: name, meter 0..1, effect strength 0..1, slipstream 0..1
    pub ability: &'static str,
    pub ability_meter: f32,
    pub ability_fx: f32,
    pub draft: f32,
    /// step 19e: the HUD's power-up slots - PowerUptype_0, PowerUptype_1, PowerUpvalue
    pub pu_hud: [f32; 3],
}

/// A time of the statistics pages from its milliseconds: MM:SS.cc, cut not rounded
/// (references `x05` 01:58.13 from 118131, `x08` 02:10.28 from 130287).
pub fn stat_time(ms: i64) -> String {
    if ms <= 0 {
        return "--:--.--".to_string();
    }
    format!("{:02}:{:02}.{:02}", ms / 60000, ms / 1000 % 60, ms / 10 % 100)
}

pub fn fmt_time(t: f32) -> String {
    let m = (t / 60.0).floor() as i32;
    let s = t - m as f32 * 60.0;
    format!("{}:{:05.2}", m, s)
}

fn ordinal(n: usize) -> String {
    match n {
        1 => "1st".into(),
        2 => "2nd".into(),
        3 => "3rd".into(),
        n => format!("{}th", n),
    }
}

/// "vp_vw_golf_83" → "VW Golf 83"
pub fn pretty_car_name(raw: &str) -> String {
    let s = raw.trim_start_matches("vpd_").trim_start_matches("vp_");
    let mut parts: Vec<String> = s.split('_').map(|w| {
        let up = ["vw", "bmw", "gmc", "ssc", "rs4", "rs6", "m3", "m5", "s5", "s600", "ctsv", "sl65", "sls", "gtr", "nsx", "db9", "slr", "gt", "evox", "sti", "rx7", "rx8", "350z", "370z", "300c", "3000gt", "280z", "240sx", "cl500", "f430", "f599", "mr2", "cc8s", "clk", "dub"];
        if up.contains(&w) || w.chars().any(|c| c.is_ascii_digit()) {
            w.to_ascii_uppercase()
        } else {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        }
    }).collect();
    // brand abbreviations
    if let Some(first) = parts.first_mut() {
        let keep = first.clone();
        *first = match first.to_ascii_lowercase().as_str() {
            "chv" => "Chevrolet".into(),
            "dge" => "Dodge".into(),
            "frd" => "Ford".into(),
            "mit" => "Mitsubishi".into(),
            "nsn" => "Nissan".into(),
            "dsn" => "Datsun".into(),
            "aud" => "Audi".into(),
            "lam" => "Lamborghini".into(),
            "cry" => "Chrysler".into(),
            "cad" => "Cadillac".into(),
            "pnt" => "Pontiac".into(),
            "mbz" | "mb" => "Mercedes-Benz".into(),
            "hnd" => "Honda".into(),
            "myz" | "maz" => "Mazda".into(),
            "sbr" | "sub" => "Subaru".into(),
            "tyt" | "toy" => "Toyota".into(),
            "lex" => "Lexus".into(),
            "plm" => "Plymouth".into(),
            "sat" => "Saturn".into(),
            "sci" => "Scion".into(),
            "vlv" => "Volvo".into(),
            "pls" | "pol" => "Police".into(),
            "ast" => "Aston Martin".into(),
            "fer" => "Ferrari".into(),
            "ssc" => "SSC".into(),
            "kaw" => "Kawasaki".into(),
            "duc" => "Ducati".into(),
            "hum" => "Hummer".into(),
            "rng" => "Range Rover".into(),
            "sal" => "Saleen".into(),
            "mcl" => "McLaren".into(),
            "koe" => "Koenigsegg".into(),
            "buk" => "Buick".into(),
            "olds" => "Oldsmobile".into(),
            "gmc" => "GMC".into(),
            _ => keep,
        };
    }
    parts.join(" ")
}

impl Default for Mode {
    fn default() -> Self {
        Mode::MainMenu
    }
}

#[allow(dead_code)]
pub fn zero_input() -> CarInput {
    CarInput::default()
}

/// Which pause camera the pause menu opens on (`cur_camera`, an index into the
/// `PauseCamera` table). The reference capture of the pause menu in free roam
/// shows PauseCamera2; `MCLA_PAUSE_CAM` picks another for comparison shots.
///
/// By page, from the reference captures: the tabs and the option pages PauseCamera2,
/// STATISTICS PauseCamera4 (the car's rear quarter, the panel at the left), MUSIC
/// PLAYLISTS PauseCamera1 (from behind and above, the panel over the car).
fn pause_camera_index(page: u8) -> i32 {
    let by_page = match page {
        1 | 9 | 60..=67 => 3,
        3 => 0,
        _ => 1,
    };
    std::env::var("MCLA_PAUSE_CAM").ok().and_then(|v| v.parse().ok()).unwrap_or(by_page)
}

impl Game {
    /// Put one of the game's message boxes up. `confirm` is what its "Yes" does.
    /// False when the box or the popup movie is not there.
    pub fn open_message_box(&mut self, id: &str, confirm: MenuAction) -> bool {
        if self.flash_popup.is_none() {
            return false;
        }
        match crate::flash_ui::MessageBoxDef::load(&self.data_dir, id) {
            // (a box without choices only says something: ACCEPT closes it)
            Some(def) if !def.options.is_empty() || !def.description.is_empty() => {
                self.message_box = Some((def, 0, confirm));
                true
            }
            _ => false,
        }
    }

    /// The message box: its page in the popup movie, navigation, and the choice.
    /// The confirming choice is the one whose id or text key says yes; anything else,
    /// and BACK, closes the box.
    fn message_box_ui(&mut self, ctx: &egui::Context, up: bool, down: bool, enter: bool, back: bool) -> Option<MenuAction> {
        use crate::flash_ui::{MenuLayout, MenuPage, MenuRow, ICON_A};
        // The ARRESTED panel (reference `y_arrested`): after the arrest's scene the screen goes
        // black and the panel says how long the pursuit was and what the fine is; CONTINUE (A).
        if std::env::var("MCLA_ARREST_TEST").is_ok() && self.arrest_panel.is_none() {
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                self.arrest_panel = Some((0.5, 25.0, 951));
                // (MCLA_ARREST_TEST=ticket: the TICKETED panel)
                if std::env::var("MCLA_ARREST_TEST").map_or(false, |v| v.contains("ticket")) {
                    self.arrest_panel = Some((0.5, 0.0, 200));
                    self.ticket_for = Some(3);
                }
            }
        }
        if let (Some((wait, secs, fine)), true) = (self.arrest_panel, self.message_box.is_none()) {
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            if wait > 0.0 {
                self.arrest_panel = Some((wait - dt, secs, fine));
            } else {
                let st = |k: &str, d: &str| self.career.strings.get(k).filter(|v| !v.contains('<')).map(|v| v.to_string()).unwrap_or_else(|| d.to_string());
                let t = secs.max(0.0) as i64;
                // TICKETED (reference `y_ticketed`): "You've been ticketed.", INFRACTION: with a
                // line for each kind, FINE:, CONTINUE (A) - the same box
                let ticket_rows: Option<Vec<MenuRow>> = self.ticket_for.map(|f| {
                    let mut rows: Vec<MenuRow> = [(1, "ticket_infraction_reckless", "RECKLESS DRIVING"), (2, "ticket_infraction_traffic", "TRAFFIC VIOLATION"), (4, "ticket_infraction_destruction", "DESTRUCTION OF PROPERTY")]
                        .iter()
                        .filter(|k| f & k.0 != 0)
                        .enumerate()
                        .map(|(i, k)| MenuRow { label: if i == 0 { st("ticket_infraction_header", "INFRACTION:").to_uppercase() } else { String::new() }, value: st(k.1, k.2).to_uppercase(), enabled: true, part: None })
                        .collect();
                    rows.push(MenuRow { label: st("cop_fine_header", "FINE:").to_uppercase(), value: format!("${}", fine), enabled: true, part: None });
                    rows
                });
                let ticket = ticket_rows.is_some();
                let page = MenuPage {
                    // (the box template with a line of text and rows of label and figure; the
                    // movie's template 59 is the banner over the arrest's scene, not this panel)
                    layout: MenuLayout::Confirm { description: if ticket { st("ticket_desc", "You've been ticketed.") } else { st("arrest_desc", "You've been arrested.") } },
                    icon: std::env::var("MCLA_ARREST_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(1),
                    title: if ticket { st("ticket_title", "TICKETED").to_uppercase() } else { st("arrest_title", "ARRESTED").to_uppercase() },
                    rows: if let Some(r) = ticket_rows { r } else { vec![
                        MenuRow { label: st("arrest_duration_header", "PURSUIT DURATION:").to_uppercase(), value: format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60), enabled: true, part: None },
                        MenuRow { label: st("cop_fine_header", "FINE:").to_uppercase(), value: format!("${}", fine), enabled: true, part: None },
                    ] },
                    // (no row is under a cursor: the rows are figures)
                    select: usize::MAX,
                    prompts: vec![(st("prompt_continue", "CONTINUE").to_uppercase(), ICON_A)],
                    ..MenuPage::default()
                };
                let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("flash_arrest"));
                ctx.move_to_top(layer);
                ctx.layer_painter(layer).rect_filled(ctx.screen_rect(), 0.0, egui::Color32::BLACK);
                if let Some(fp) = self.flash_popup.as_mut() {
                    fp.show(&page);
                    fp.movie.player.update(dt);
                    fp.movie.paint(ctx, &ctx.layer_painter(layer));
                }
                if enter {
                    self.ticket_for = None;
                    self.arrest_panel = None;
                    if let Some(fp) = self.flash_popup.as_mut() {
                        fp.hide();
                    }
                }
                return None;
            }
        }
        let Some((def, mut sel, confirm)) = self.message_box.clone() else {
            // The LOADING tag (the popup movie's `save_mc`: `savegame_text`, `savegame_anim`
            // 1 = come in, 2 = go) while the game moves between the city and the garage
            // (reference `t28`). (approx.) The original shows it for as long as it loads,
            // flying over the map; the port changes place at once, so the tag stands for
            // a second and a half.
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            let garage_now = matches!(self.mode, Mode::Garage);
            if garage_now != self.loading_was_garage || std::env::var("MCLA_LOADING_TEST").is_ok() {
                self.loading_was_garage = garage_now;
                self.loading_tag = 1.5;
            }
            let text = self.career.strings.get("VE_Loading").map(|v| v.to_uppercase()).unwrap_or_else(|| "LOADING".into());
            // (the race's badge is the popup movie's too: it stays while it is up)
            let badge_up = self.start_badge_on || self.tutorial_box.as_ref().map_or(false, |t| t.2 <= 0.0);
            if let Some(fp) = self.flash_popup.as_mut() {
                if fp.is_shown() && !badge_up {
                    fp.hide();
                }
                use mcla_flash::value::Value;
                if self.loading_tag > 0.0 {
                    if !self.loading_on {
                        fp.movie.player.set_global("savegame_text", Value::str(&text));
                        fp.movie.player.set_global("savegame_anim", Value::Int(1));
                        self.loading_on = true;
                    }
                    self.loading_tag -= dt;
                } else if self.loading_on {
                    fp.movie.player.set_global("savegame_anim", Value::Int(2));
                    self.loading_on = false;
                    self.loading_fade = 1.0;
                }
                // let it fade out
                fp.movie.player.update(dt);
                if self.loading_on || self.loading_fade > 0.0 {
                    self.loading_fade = (self.loading_fade - dt).max(0.0);
                    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("flash_loading"));
                    fp.movie.paint(ctx, &ctx.layer_painter(layer));
                }
            }
            return None;
        };
        let n = def.options.len();
        // (test hook: MCLA_BOX_ACCEPT answers YES once the box has stood for 300 frames)
        static STOOD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let auto = std::env::var("MCLA_BOX_ACCEPT").is_ok() && STOOD.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 300;
        if auto {
            sel = def.options.iter().position(|o| o.0.to_ascii_lowercase().ends_with("yes") || o.1.eq_ignore_ascii_case("Common_Yes")).unwrap_or(0);
        }
        let enter = enter || auto;
        if up && sel > 0 {
            sel -= 1;
        }
        if down && sel + 1 < n {
            sel += 1;
        }
        let st = |k: &str| self.career.strings.get(k).map(|v| v.to_string()).unwrap_or_else(|| k.to_string());
        let page = MenuPage {
            layout: MenuLayout::Confirm { description: st(&def.description) },
            // (the warning triangle; 2 and up are the question mark)
            icon: 1,
            title: st(&def.title).to_uppercase(),
            rows: def.options.iter().map(|(_, d)| MenuRow { label: st(d).to_uppercase(), value: String::new(), enabled: true, part: None }).collect(),
            select: sel,
            // (a box that only says something closes on CONTINUE)
            prompts: vec![(st(if def.options.is_empty() { "prompt_continue" } else { "prompt_accept" }).to_uppercase(), ICON_A)],
            ..MenuPage::default()
        };
        // From the pause menu the box stands on the menu's own panel, in the pause movie
        // (reference `t25`: QUIT RACE with BACK and ACCEPT), not in the popup movie.
        let in_pause = matches!(self.mode, Mode::Paused) && self.flash_menu.is_some();
        let mut out = None;
        let yes = |o: &(String, String)| o.0.to_ascii_lowercase().ends_with("yes") || o.1.eq_ignore_ascii_case("yes");
        if enter {
            if def.options.get(sel).map_or(false, yes) {
                out = Some(confirm);
            }
            self.box_sel = sel;
            self.message_box = None;
        } else if back {
            self.message_box = None;
        } else {
            self.message_box = Some((def, sel, confirm));
        }
        if in_pause {
            self.box_page = self.message_box.is_some().then(|| MenuPage { prompts: vec![(st("prompt_accept").to_uppercase(), ICON_A), (st("prompt_back").to_uppercase(), crate::flash_ui::ICON_B)], list_max: 6, ..page.clone() });
            return out;
        }
        if let Some(fp) = self.flash_popup.as_mut() {
            if self.message_box.is_some() {
                fp.show(&page);
            } else {
                fp.hide();
            }
            fp.movie.player.update(ctx.input(|i| i.stable_dt).min(0.1));
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_popup"));
            ctx.move_to_top(layer);
            fp.movie.paint(ctx, &ctx.layer_painter(layer));
        }
        out
    }
}

impl Game {
    /// The performance shop on the garage movie's own page (`template_PS_level`).
    /// False when the movie is not there (the port's own table is drawn then).
    fn perf_shop_movie(&mut self, ctx: &egui::Context, list: &[(String, i64, i64)], stages: &[u8], sel: usize) -> bool {
        use crate::flash_ui::{MenuLayout, MenuPage, MenuRow, ICON_A, ICON_B};
        if self.flash_garage.is_none() || self.ui_meshes.is_none() || std::env::var("MCLA_FLAT_MENU").is_ok() {
            return false;
        }
        let i = self.current_car;
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).unwrap_or_else(|| d.to_string());
        // the maker each part comes from: the one chosen with CHANGE BRAND, else the first of
        // the part's list (`PERF_BRANDS`)
        let bike = self.catalog.get(&self.dir_name(i)).map_or(false, |c| c.class.to_ascii_lowercase().contains("bike"));
        let chosen = self.perf_brand.get(&self.dir_name(i).to_ascii_lowercase()).copied().unwrap_or([0; 13]);
        let brand_at = |k: usize| -> i32 {
            if let (true, Some((_, b))) = (bike, PERF_BRAND_BIKE.iter().find(|x| x.0 == k)) {
                return *b as i32;
            }
            let list = PERF_BRANDS[k.min(12)];
            list[(chosen[k.min(12)] as usize).min(list.len() - 1)] as i32
        };
        let unlocked = 1 + (self.rep >= self.perf_rep(i, 1)) as u8 + (self.rep >= self.perf_rep(i, 2)) as u8;
        let mut details = Vec::new();
        let mut rows = Vec::new();
        for (k, (id, p1, p2)) in list.iter().enumerate() {
            let have = stages.get(k).copied().unwrap_or(0).min(2);
            let price = if have >= 2 || have + 1 >= unlocked || self.parts_free() { "---".to_string() } else { format!("${}", if have == 0 { *p1 } else { *p2 }) };
            details.push((have, unlocked, price, brand_at(k)));
            rows.push(MenuRow { label: self.career.strings.get(id).map(|s| s.to_uppercase()).unwrap_or_else(|| id.trim_start_matches("gp_").to_uppercase()), value: String::new(), enabled: true, part: None });
        }
        let f = self.perf_frac(i);
        let meters = match self.catalog.get(&self.dir_name(i)) {
            Some(c) => {
                let row = |base: f32, mods: f32| [(base + (mods - base) * f) * 100.0, base * 100.0, mods * 100.0];
                [row(c.base_accel * METER_ACCEL, c.mods_accel * METER_ACCEL), row((c.base_speed - 100.0) * METER_SPEED, (c.mods_speed - 100.0) * METER_SPEED), row(c.base_handling, c.mods_handling)]
            }
            None => [[0.0, 0.0, 100.0]; 3],
        };
        let page = MenuPage {
            layout: MenuLayout::PerfShop { details, class: crate::rewards::perf_class(self.player_perf()) as i32, total: 0, meters },
            icon: 10,
            title: st("G_Performance", "PERFORMANCE SHOP"),
            rows,
            select: sel,
// (the original's bar also carries AUTO UPGRADE and CHANGE BRAND - reference `n13`;
            // neither is wired in the port)
            prompts: vec![(st("prompt_accept", "ACCEPT"), ICON_A), (st("prompt_back", "BACK"), ICON_B), (st("G_AutoUpgrade", "AUTO UPGRADE"), crate::flash_ui::ICON_X), (st("G_ChangeBrand", "CHANGE BRAND"), crate::flash_ui::ICON_LB_RB)],
            money: Some((st("G_Money", "MONEY:"), self.money)),
            ..MenuPage::default()
        };
        let (Some(fg), Some(meshes)) = (self.flash_garage.as_mut(), self.ui_meshes.as_mut()) else { return false };
        fg.show(&page);
        fg.movie.player.set_global("t_pstotal", mcla_flash::value::Value::str("TOTAL:"));
        fg.movie.player.update(ctx.input(|i| i.stable_dt).min(0.1));
        let Some(scene) = crate::menu3d::read_scene(&mut fg.movie.player) else { return false };
        if let Some(fp) = self.flash_prompt.as_mut() {
            fp.hide();
        }
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_garage"));
        ctx.move_to_top(layer);
        crate::menu3d::paint_scene(ctx, &ctx.layer_painter(layer), meshes, &mut fg.movie, &scene, None, true);
        true
    }
}

impl Game {
    /// The mission log over the GPS map, on the map movie's own page: ACTIVE MISSIONS
    /// and COMPLETED MISSIONS (LB / RB), the chosen mission's text and objectives.
    /// Y (the GPS prompt) folds it away to leave the map.
    fn mission_log_ui(&mut self, ctx: &egui::Context, keys: [bool; 5], tab_prev: bool, tab_next: bool, map_vp: Option<glam::Mat4>, player: Vec3) {
        let [up, down, left, right, enter] = keys;
        use crate::flash_ui::{MenuLayout, MenuPage, MenuRow, ICON_B, ICON_Y};
        if self.flash_navsys.is_none() || self.editor.is_some() {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Y)) {
            self.mission_log.2 = !self.mission_log.2;
        }
        let guided = self.mlog_guided;
        if guided {
            self.mission_log.2 = false;
        }
        let (mut tab, mut sel, hidden) = self.mission_log;
        if guided {
            // (two tabs, and they do not go round: `ng_mlog_tutorials` has RB dark)
            if tab_next && tab == 0 {
                (tab, sel) = (2, 0);
            } else if tab_prev && tab == 2 {
                (tab, sel) = (0, 0);
            }
        } else if tab_prev || tab_next {
            tab = (tab + if tab_next { 1 } else { 2 }) % 3;
            sel = 0;
        }
        if hidden && ctx.input(|i| i.key_pressed(egui::Key::X)) {
            self.map_legend = !self.map_legend;
        }
        if std::env::var("MCLA_MAP_LEGEND").is_ok() {
            self.map_legend = true;
        }
        if let Ok(v) = std::env::var("MCLA_MAP_TAB") {
            tab = v.parse::<usize>().unwrap_or(0).min(2);
        }
        // (test hook: MCLA_MAP_SEL=n puts the cursor on the log's row n - step 20)
        if let Ok(v) = std::env::var("MCLA_MAP_SEL") {
            sel = v.parse::<usize>().unwrap_or(0);
        }
        // (the third tab is TUTORIALS: the list of MissionLog.sc.xml - title, picture,
        // text - reference `z06`)
        let tutorials: Vec<(String, String, String)> = if tab == 2 {
            std::fs::read_to_string(std::path::Path::new(&self.data_dir).join("tune/ui/MissionLog.sc.xml"))
                .unwrap_or_default()
                .lines()
                .filter(|l| l.contains("TutorialItem.sc"))
                .filter_map(|l| {
                    let f: Vec<&str> = l.split('\'').collect();
                    Some((f.get(1)?.to_string(), f.get(3)?.to_string(), f.get(5)?.to_string()))
                })
                // (the list holds the tutorials the career has shown - `ng_mlog_tutorials`: only
                // MISSION LOG after the first race. A career that keeps no such record - an
                // older or imported save - lists them all, as the 100 % save does: `z06`.)
                .filter(|t: &(String, String, String)| !self.career.stats.keys().any(|k| k.starts_with("tut:")) || self.career.stats.contains_key(&format!("tut:{}", t.0.trim_start_matches("tutorial_").trim_end_matches("_title"))))
                .collect()
        } else {
            Vec::new()
        };
        let views = match tab {
            0 => self.career.active(),
            1 => self.career.completed_views(),
            _ => tutorials
                .iter()
                .map(|(title, picture, text)| crate::career::MissionView {
                    id: picture.clone(),
                    title: self.career.strings.get(title).map(|v| v.to_string()).unwrap_or_else(|| title.clone()),
                    who: String::new(),
                    desc: self.career.strings.get(text).map(|v| v.to_string()).unwrap_or_default(),
                    objectives: Vec::new(),
                    next_race: None,
                })
                .collect(),
        };
        // (missions the disc has struck out are titled DELETE: never listed)
        let views: Vec<_> = views.into_iter().filter(|v| !v.title.trim().eq_ignore_ascii_case("DELETE")).collect();
        let n = views.len();
        if n > 0 {
            if up {
                sel = (sel + n - 1) % n;
            }
            if down {
                sel = (sel + 1) % n;
            }
            sel = sel.min(n - 1);
        } else {
            sel = 0;
        }
        self.mission_log = (tab, sel, hidden);
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).unwrap_or_else(|| d.to_string());
        let picture = views.get(sel).map(|v| {
            let dir = std::path::Path::new(&self.data_dir).join("resources/ui/textures/navsys/dyntextures");
            let named = dir.join(format!("{}.r9", v.id));
            // (files whose names the extraction did not recover go by the hash of `<id>.xtd`)
            if named.exists() { named } else { dir.join(format!("0x{:08x}.r9", mcla_formats::hash::joaat(&format!("{}.xtd", v.id.to_ascii_lowercase())))) }
        });
        let (summary, objectives) = match views.get(sel) {
            // (an objective's line ends in its count where the condition has one: "(0/6)" -
            // sub_826CC350, step 20)
            Some(v) => (v.desc.clone(), v.objectives.iter().take(5).map(|o| (o.count.as_ref().map_or_else(|| o.text.clone(), |c| format!("{} {}", o.text, c)).to_uppercase(), tab == 1 || o.cur >= o.target)).collect()),
            None => (String::new(), Vec::new()),
        };
        let titles = [st("Navmap_MLog_ActiveMissions", "ACTIVE MISSIONS"), st("Navmap_MLog_CompletedMissions", "COMPLETED MISSIONS"), st("Navmap_MLog_Tutorials", "TUTORIALS")];
        let page = MenuPage {
            layout: MenuLayout::MissionLog { summary, objectives },
            // (tab icon frames: 36 "!", 37 "x", 38 "?")
            icon: 36 + tab as i32,
            tabs: if guided { vec![titles[0].clone(), titles[2].clone()] } else { titles.to_vec() },
            tab_icons: if guided { vec![36, 38] } else { vec![36, 37, 38] },
            cur_tab: if guided { tab / 2 } else { tab },
            title: titles[tab].clone(),
            rows: views.iter().map(|v| MenuRow { label: v.title.to_uppercase(), value: String::new(), enabled: true, part: None }).collect(),
            select: sel,
            ..MenuPage::default()
        };
        let prompts = vec![(st("Navmap_Prompt_Exit", "EXIT"), ICON_B), (st("Navmap_Prompt_GPSMap", "GPS"), ICON_Y), (st("prompt_changeTabs", "CHANGE TABS"), crate::flash_ui::ICON_LB_RB)];
        // (the active missions can be made the GPS target; the port has no target to set yet)
        let mut prompts = prompts;
        if tab == 0 && !views.is_empty() {
            prompts.insert(0, (st("Navmap_Prompt_SetTarget", "SET AS TARGET"), crate::flash_ui::ICON_A));
            if enter && !hidden && !guided {
                self.mission_target = views.get(sel).map(|v| v.id.clone());
                self.career.stats.entry("tut:target_set".into()).or_insert(1);
            }
        }
        if guided {
            prompts = if tab == 0 { vec![(st("Prompt_SetMissionAsTarget", "SET TO ACTIVE TARGET"), crate::flash_ui::ICON_A)] } else { vec![(st("Prompt_NavigateToMissionLog", "VIEW MISSIONS"), crate::flash_ui::ICON_LB)] };
            if tab == 0 && enter && !views.is_empty() {
                // the target set: the lesson is over, and the MINI MAP box follows
                self.mission_target = views.get(sel).map(|v| v.id.clone());
                self.mlog_guided = false;
                self.show_map = false;
                self.career.stats.insert("tut:target_set".into(), 1);
                self.dirty = true;
            }
        }
        // The map's cursor (references `z03`, `z07`): it stands on one of the map's places -
        // CHANGE TARGET moves it to the nearest in that direction - and a small box names
        // the place; SET AS TARGET (A) routes the GPS there.
        let screen = ctx.screen_rect();
        let to_screen = |w: Vec3| -> Option<egui::Pos2> {
            let c = map_vp? * w.extend(1.0);
            (c.w > 0.01).then(|| egui::pos2(screen.left() + (c.x / c.w * 0.5 + 0.5) * screen.width(), screen.top() + (0.5 - c.y / c.w * 0.5) * screen.height()))
        };
        let ev = self.career.map_events();
        let map_races: Vec<usize> = (0..self.race_defs.len()).filter(|&i| { let id = self.race_defs[i].id.to_ascii_lowercase(); ev.iter().any(|(r, _)| id.ends_with(r.as_str())) }).collect();
        let places: Vec<(Vec3, String, String)> = if hidden {
            // (the cursor opens on the player - `s11/rk95_map.png` - and names nothing there)
            let early = self.career.loaded() && !self.career.reached("GARAGE_HOLLYWOOD");
            let man = self.rival_mark.clone().map(|(p, name, car)| {
                let car_name = self.career.strings.get(&car.to_ascii_lowercase()).or_else(|| self.career.strings.get(&car)).map(|v| v.to_uppercase()).filter(|v| !v.contains('<')).unwrap_or_else(|| pretty_car_name(&car).to_uppercase());
                (p, name.to_uppercase(), car_name)
            });
            std::iter::once((player, String::new(), String::new()))
                .chain(man)
                .chain(map_races
                .iter()
                .filter(|&&i| { let id = self.race_defs[i].id.to_ascii_lowercase(); ev.iter().find(|(r, _)| id.ends_with(r.as_str())).map_or(true, |(_, m)| !Self::event_unmarked(self.event_style(i, m).0)) })
                .filter_map(|&i| {
                    let d = self.race_defs.get(i)?;
                    let name = self.career.strings.get(&d.id).filter(|n| !n.contains('<')).map(|n| n.to_uppercase()).unwrap_or_else(|| d.name.to_uppercase().replace('_', " "));
                    Some((self.race_start(i)?, name, st(&d.sub_type, &d.sub_type.to_uppercase())))
                }))
                .chain(self.hangouts.iter().enumerate().filter(|(n, h)| !early && (h.x.abs() >= 1.0 || h.z.abs() >= 1.0) && self.hangout_names.get(*n).map_or(true, |s| self.mc4_hangout_open(s))).map(|(n, h)| (*h, st(if self.hangout_wager.get(n).copied().unwrap_or(false) { "Navmap_Wager" } else { "Navmap_series" }, "HANGOUT"), String::new())))
                .chain((if self.hm.is_some() { crate::hudmap::GAS_STATIONS.iter().map(|g| Vec3::from(*g)).collect::<Vec<_>>() } else { self.gas_stations.clone() }).into_iter().map(|g| (g, st("tutorial_gas_station_title", "SERVICE STATION"), String::new())))
                .collect()
        } else {
            Vec::new()
        };
        let mut cursor_at: Option<(egui::Pos2, String, String)> = None;
        if hidden && !places.is_empty() {
            if self.map_cursor >= places.len() {
                // (it opens on the place nearest the player)
                // (on the target once the mission's man is one - `ng_gps_1`)
                self.map_cursor = if self.mission_target.is_some() && self.rival_mark.is_some() { 1 } else { 0 };
            }
            let here = to_screen(places[self.map_cursor].0);
            let dir = egui::vec2(right as i32 as f32 - left as i32 as f32, down as i32 as f32 - up as i32 as f32);
            if let (true, true) = (self.hm.is_some(), dir != egui::Vec2::ZERO) {
                // the original's rule (`sub_82628878`): the nearest place in 3D within 1 rad of the
                // way pressed, from the cursor, in the world (up = north)
                let pts: Vec<Vec3> = places.iter().map(|p| p.0).collect();
                let from = self.map_free.unwrap_or(places[self.map_cursor].0);
                if let Some(i) = crate::hudmap::pick_in_direction(from, Vec3::new(dir.x, 0.0, dir.y), &pts) {
                    self.map_cursor = i;
                    self.map_free = None;
                }
            } else if let (Some(h), true) = (here, dir != egui::Vec2::ZERO) {
                // the nearest place within a quarter turn of the way pressed
                let best = (0..places.len())
                    .filter(|i| *i != self.map_cursor)
                    .filter_map(|i| {
                        let d = to_screen(places[i].0)? - h;
                        let along = d.dot(dir);
                        (along > 4.0 && along > d.length() * 0.7 && screen.contains(h + d)).then(|| (i, d.length()))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((i, _)) = best {
                    self.map_cursor = i;
                }
            }
            // (stick released: the icon under the cursor is selected - `sub_826281E8`; reach = the
            // ring's radius at the cursor's distance from the camera)
            if let (true, Some(fp)) = (std::mem::take(&mut self.map_free_release), self.map_free) {
                let cam = self.hm.as_ref().map_or(Vec3::new(0.0, 9000.0, 0.0), |h| h.gps_cam);
                let reach = 1.3424 * 139.0 * ((fp - cam).length() * 7e-5 + 0.04);
                let best = (0..places.len()).map(|i| (i, glam::Vec2::new(places[i].0.x - fp.x, places[i].0.z - fp.z).length())).filter(|x| x.1 < reach).min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((i, _)) = best {
                    self.map_cursor = i;
                    self.map_free = None;
                }
            }
            // MAP MODE (START, `sub_826293D0` event 55): the road overlay with the street names
            if std::mem::take(&mut self.nav_start) {
                if let Some(hm) = self.hm.as_mut() {
                    hm.map_mode = !hm.map_mode;
                }
            }
            let free = self.map_free.is_some();
            let on_man = self.map_cursor == 1 && self.rival_mark.is_some() && !free;
            let enter = enter && !free;
            if enter && on_man {
                // SET AS TARGET / UNSET TARGET on the mission's man
                self.mission_target = if self.mission_target.is_some() { None } else { self.career.hookman_mission().map(|m| m.to_string()) };
                self.career.stats.entry("tut:target_set".into()).or_insert(1);
            } else if enter && self.map_cursor > 0 {
                self.gps_pick = Some(places[self.map_cursor].0);
                self.career.stats.entry("tut:target_set".into()).or_insert(1);
            }
            let (w, name, kind) = match self.map_free {
                Some(fp) => (fp, String::new(), String::new()),
                None => places[self.map_cursor].clone(),
            };
            self.map_cursor_pos = Some(w);
            cursor_at = to_screen(w).map(|q| (q, name, kind));
        }
        // The map itself (references `z00`, `z07`): the GPS MAP panel - rank and
        // reputation, the vehicle's rank and class, the money - with the legend on X.
        let (page, prompts) = if hidden {
            // (the thirteen ranks are the reputation curve's levels, as on REWARDS - step 86;
            // `ng_gps_3`: BACKSEAT DRIVER - 50 REP, NEXT RANK AT 800)
            const RANKS: [&str; 13] = ["Rank_1_Backseat_Driver", "Rank_2_Navigator", "Rank_3_Student_Driver", "Rank_4_Rookie", "Rank_5_Driver", "Rank_6_Racer", "Rank_7_Veteran", "Rank_8_Elite_Racer", "Rank_8_5_Champion", "Rank_9_Legend", "Rank_10_Savant", "Rank_10_5_Hero", "Rank_11_Idol"];
            let rank = crate::rewards::progress(self.rep).0.min(12);
            let rank_step = |l: usize| crate::rewards::rep_curve(crate::rewards::LEVEL_FRAC[l.min(12)]);
            let info = self.catalog.get(&self.dir_name(self.current_car)).cloned();
            let kind = info.as_ref().map(|c| c.class.to_uppercase()).unwrap_or_default();
            // (the class as MANAGE VEHICLES has it: the catalogue's, one up when fully upgraded)
            let base = info.as_ref().and_then(|c| c.group.chars().next()).unwrap_or('D').to_ascii_uppercase();
            let base = if self.perf_frac(self.current_car) > 0.999 { match base { 'D' => 'C', 'C' => 'B', _ => 'A' } } else { base };
            let class = match base { 'A' => "A", 'B' => "B", 'C' => "C", _ => "D" };
            // the vehicle kind's rank: the step its class system stands on + 1, and the points on
            // that step out of its length (sub_8262D560: `veh_level` "(%d/%d)", `veh_meter`; `mc4`)
            let (veh_rank, veh_a, veh_b) = self.mc4_class_rank(&kind);
            let next = if rank >= 12 { "N/A".to_string() } else { format!("{:.0}", rank_step(rank + 1)) };
            let mut globals: Vec<(String, String)> = vec![
                ("experience".into(), format!("{} - {} {}", st(RANKS[rank], "RANK"), self.rep, st("Navmap_Rep", "REP"))),
                ("gps_rank".into(), String::new()),
                ("next_rank".into(), format!("{} {}", st("Next_Rank", "NEXT RANK AT"), next)),
                ("gps_veh_type".into(), String::new()),
                ("gps_veh_level".into(), format!("{} {} {}", kind, st("Car_Type_Rank", "RANK"), veh_rank)),
                ("veh_level".into(), format!("({}/{})", veh_a, veh_b)),
                ("t_vehicle".into(), st("Navmap_Vehicle", "VEHICLE")),
                ("gps_class_type".into(), st(&format!("vehicle_PerfClass{}_Short", class), &format!("CLASS {}", class))),
                ("gps_vehicle".into(), self.career.strings.get(&format!("{}_shrt", self.dir_name(self.current_car).to_ascii_lowercase())).map(|v| v.to_uppercase()).unwrap_or_else(|| self.car_name(self.current_car).to_uppercase())),
                ("t_bank".into(), st("G_Money", "MONEY:")),
                ("gps_racename".into(), String::new()),
            ];
            for (var, key, fb) in [
                ("t_you", "Navmap_Player", "PLAYER"), ("t_mission", "Navmap_Mission", "MISSION"), ("t_wager", "Navmap_Wager", "WAGER"), ("t_localrace", "Navmap_LocalRace", "CRUISING RACER"),
                ("t_timetrial", "Navmap_TimeTrial", "TIME TRIAL"), ("t_redlight", "Navmap_redlight", "RED LIGHT"), ("t_delivery", "Navmap_Delivery", "DELIVERY"), ("t_police", "Navmap_Police", "POLICE"),
                ("t_dynamic", "Navmap_Dynamic", "FREEWAY RACER"), ("t_tournament", "Navmap_Tournament", "TOURNAMENT"), ("t_pinkslip", "Navmap_pinkslip", "PINKSLIPS"), ("t_garage", "Navmap_Garage", "GARAGE"),
                ("t_series", "Navmap_series", "SERIES RACE"), ("t_payback", "Navmap_Payback", "PAYBACK"), ("t_difficulty", "Navmap_Difficulty", "DIFFICULTY"), ("t_difficulty_1", "Navmap_Difficulty1", "EASIEST"),
                ("t_difficulty_2", "Navmap_Difficulty2", "MEDIUM"), ("t_difficulty_3", "Navmap_Difficulty3", "HARD"), ("t_difficulty_4", "Navmap_Difficulty4", "HARDEST"), ("t_opponent", "Navmap_Opponent", "OPPONENT"),
                ("t_checkpoints", "Navmap_Checkpoints", "CHECKPOINTS"),
            ] {
                globals.push((var.into(), st(key, fb)));
            }
            (
                MenuPage {
                    layout: MenuLayout::GpsMap { legend: self.map_legend, globals, meters: [if rank >= 12 { 1.0 } else { ((self.rep as f32 - rank_step(rank)) / (rank_step(rank + 1) - rank_step(rank)).max(1.0)).clamp(0.0, 1.0) }, if veh_b > 0 { (veh_a as f32 / veh_b as f32).clamp(0.0, 1.0) } else { 1.0 }] },
                    icon: 1,
                    title: st("Navmap_GPSMap", "GPS MAP"),
                    rows: vec![MenuRow { label: cursor_at.as_ref().map(|c| c.1.clone()).unwrap_or_default(), value: cursor_at.as_ref().map(|c| c.2.clone()).unwrap_or_default(), enabled: true, part: None }],
                    money: Some((st("G_Money", "MONEY:"), self.money)),
                    ..MenuPage::default()
                },
                vec![
                    (st("Navmap_Prompt_Exit", "EXIT"), ICON_B),
                    (st("Navmap_Prompt_MissionLog", "MISSION LOG"), ICON_Y),
                    (st(if self.map_legend { "Navmap_Prompt_TurnOffLegend" } else { "Navmap_Prompt_Legend" }, "LEGEND ON"), crate::flash_ui::ICON_X),
                    (st("Navmap_Prompt_MapMode", "MAP MODE"), if self.hm.is_some() { crate::flash_ui::ICON_START } else { crate::flash_ui::ICON_BACK_BUTTON }),
                    (st("Navmap_Prompt_ChangeTarget", "CHANGE TARGET"), 19),
                    (st("Navmap_Prompt_Zoom", "ZOOM"), crate::flash_ui::ICON_LT_RT),
                ],
            )
        } else {
            (page, prompts)
        };
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_navsys"));
        ctx.move_to_top(layer);
        let lp = ctx.layer_painter(layer);
        if let Some(fm) = self.flash_navsys.as_mut() {
            fm.show(&page);
            // The cursor as the game hands it to the movie (`sub_826263C0`): place = cursor + NM+544,
            // no turn, size (|place - camera| * 7e-05 + 0.04) * 4000; the movie's `target` clip
            // puts them on its `cursor_mesh` (navsys_cursor, the movie as its texture) and shows
            // the ring (ring_BG, or ring_circle + ring_Anim on a selected icon).
            if let (Some(hm), Some(_), Some(w), true) = (self.hm.as_ref(), map_vp, self.map_cursor_pos, hidden) {
                use mcla_flash::value::Value;
                let t = ((hm.gps_cam.y - 850.0) / (9000.0 - 850.0)).clamp(0.0, 1.0);
                let at = w + Vec3::new(-7.0, -1.0, -4.0).lerp(Vec3::new(-33.0, 0.0, -9.0), t);
                let sz = ((at - hm.gps_cam).length() * 7e-5 + 0.04) * 4000.0;
                let p = &mut fm.movie.player;
                for (k, v) in [at.x, at.y, at.z].into_iter().enumerate() {
                    p.set_path(&format!("cur_cursor_Position.{k}"), Value::Float(v));
                    p.set_path(&format!("cur_cursor_Rotation.{k}"), Value::Float(0.0));
                    p.set_path(&format!("cur_cursor_Menuscale.{k}"), Value::Float(sz));
                }
                p.set_global("cursor_visibility", Value::Int(1));
                p.set_global("iconisselected", Value::Int((self.map_cursor > 0 && self.map_free.is_none()) as i32));
            }
            fm.movie.player.update(dt);
            // The box that names the place under the cursor. In the game it lies on a
            // mesh of its own at the cursor (`cursorpanel_mesh`, frame 13); here its clip is
            // drawn flat beside the cursor (approx.: place and size by `z03`).
            let panel = fm.movie.player.find("screen.menu.target_panel");
            if let (true, Some(id)) = (hidden, panel) {
                use mcla_flash::value::Value;
                let unset = self.map_cursor == 1 && self.rival_mark.is_some() && self.mission_target.is_some();
                fm.movie.player.set_path("menu.elements.0.text4", Value::str(&if unset { st("Nav_UnSetAsTarget", "UNSET TARGET") } else { st("Navmap_Prompt_SetTarget", "SET AS TARGET") }));
                fm.movie.player.set_global("cursorpanel_visibility", Value::Int(cursor_at.as_ref().map_or(false, |c| !c.1.is_empty()) as i32));
                if let Some((q, _, _)) = cursor_at.as_ref() {
                    if let Some(i) = fm.movie.player.inst_mut(id) {
                        i.tint[3] = 255;
                    }
                    let list = mcla_flash::draw::build_instance(&fm.movie.player, id);
                    let k = screen.height() / 720.0 * 0.62;
                    let (w, h) = (screen.width(), screen.height());
                    // (the clip lies turned and parked beside the stage, for its mesh: undo its own
                    // matrix, then set it left of the cursor at the movie's own measure of the box)
                    let im = fm.movie.player.inst(id).map(|i| i.matrix).unwrap_or(mcla_formats::rage::swf::Matrix::IDENTITY);
                    let det = (im.a * im.d - im.b * im.c).abs().max(1e-6) * (im.a * im.d - im.b * im.c).signum();
                    let inv = glam::Mat4::from_cols(
                        glam::Vec4::new(im.d / det, -im.b / det, 0.0, 0.0),
                        glam::Vec4::new(-im.c / det, im.a / det, 0.0, 0.0),
                        glam::Vec4::Z,
                        glam::Vec4::new((im.c * im.ty - im.d * im.tx) / det, (im.b * im.tx - im.a * im.ty) / det, 0.0, 1.0),
                    );
                    let g = |n: &str, d: f32| fm.movie.player.get_global(n).map_or(d, |v| v.to_f32()).clamp(20.0, 600.0);
                    let (bw, bh) = (g("cur_panel_target_width", 200.0), g("cur_panel_target_height", 90.0));
                    let (px, py) = (q.x - screen.left() - (bw + 26.0) * k, q.y - screen.top() - bh * 0.5 * k);
                    let m = glam::Mat4::from_cols(glam::Vec4::new(2.0 * k / w, 0.0, 0.0, 0.0), glam::Vec4::new(0.0, -2.0 * k / h, 0.0, 0.0), glam::Vec4::Z, glam::Vec4::new(2.0 * px / w - 1.0, 1.0 - 2.0 * py / h, 0.0, 1.0)) * inv;
                    let far = [-4000.0, -4000.0, 4000.0, 4000.0];
                    if self.map_cursor > 0 {
                        fm.movie.paint_warped(ctx, &lp, &list, m, screen, 1.0, Some(far));
                    }
                    // (the cursor itself: a ring of the map's light blue)
                    // The original's cursor (`sub_82227428` case 0x20000000, rule_frame.md 4.2):
                    // the mesh `navsys_cursor_BG` in white at the cursor's place + NM+544, scaled
                    // (|cursor - camera| * 7e-05 + 0.04) * 4000 (the recorded frame: 2708.5).
                    let own = match (self.hm.as_mut(), map_vp, self.map_cursor_pos) {
                        (Some(hm), Some(vp), Some(w)) => {
                            let t = ((hm.gps_cam.y - 850.0) / (9000.0 - 850.0)).clamp(0.0, 1.0);
                            let off = Vec3::new(-7.0, -1.0, -4.0).lerp(Vec3::new(-33.0, 0.0, -9.0), t);
                            let at = w + off;
                            let s = ((at - hm.gps_cam).length() * 7e-5 + 0.04) * 4000.0;
                            // (navsys_cursor_BG's picture is empty - alpha 0 throughout: it only writes
                            // depth.) The ring is `navsys_cursor` with the movie's own picture as its
                            // texture (`0x8290A9A8`, a copy of the whole stage): its uv take the stage's
                            // corner x 1201.2..1279.1, y 641.4..719.4, where the movie parks its
                            // `target` clip; the quad spans x +-0.0874, z +-0.0898 at y 0.0146 (mesh
                            // units), scaled (s, 1, s) at the cursor.
                            let (u0, u1, v0, v1) = (0.9384308f32, 0.9993286f32, 0.89089966f32, 0.99913025f32);
                            let (x0, x1, z0, z1) = (-0.087402344f32, 0.087402344f32, -0.08984375f32, 0.08984375f32);
                            let (sw, sh) = (1280.0f32, 720.0f32);
                            let kx = (x1 - x0) / ((u1 - u0) * sw);
                            let kz = (z1 - z0) / ((v1 - v0) * sh);
                            let stage_to_local = glam::Mat4::from_cols(glam::Vec4::new(kx, 0.0, 0.0, 0.0), glam::Vec4::new(0.0, 0.0, kz, 0.0), glam::Vec4::new(0.0, 1.0, 0.0, 0.0), glam::Vec4::new(x0 - u0 * sw * kx, 0.0146484375, z0 - v0 * sh * kz, 1.0));
                            let world = glam::Mat4::from_scale_rotation_translation(Vec3::new(s, 1.0, s), glam::Quat::IDENTITY, at);
                            let list = mcla_flash::draw::build(&fm.movie.player);
                            fm.movie.paint_warped(ctx, &lp, &list, vp * world * stage_to_local, screen, 1.0, Some([u0 * sw, v0 * sh, u1 * sw, v1 * sh]));
                            true
                        }
                        _ => false,
                    };
                    // (eight dashes round the marker - `ng_gps_1` enlarged)
                    let (rr, col) = (20.0 * screen.height() / 720.0, crate::flash_ui::plain_color(40, 200, 230, 255));
                    for n in (0..8).filter(|_| !own) {
                        let a0 = (n as f32 + 0.12) * std::f32::consts::TAU / 8.0;
                        let pts: Vec<egui::Pos2> = (0..=6).map(|j| { let a = a0 + j as f32 / 6.0 * 0.76 * std::f32::consts::TAU / 8.0; *q + egui::vec2(a.cos(), a.sin()) * rr }).collect();
                        lp.add(egui::Shape::line(pts, egui::Stroke::new(4.5 * screen.height() / 720.0, col)));
                    }
                }
                if let Some(i) = fm.movie.player.inst_mut(id) {
                    i.tint[3] = 0;
                }
            }
            // The mission's picture: the `mission_images` clip takes a texture the game
            // names after the mission (`ui/navsys/dyntextures/<MISSION_ID>.dds`). A mission
            // without one keeps the clip hidden.
            let shown = match &picture {
                Some(file) => fm.movie.swap_texture(ctx, "screen.menu.mission_layer.mission_images", 1, file),
                None => false,
            };
            for name in ["screen.menu.mission_layer.mission_images", "screen.menu.mission_layer.picture_bg"] {
                if let Some(id) = fm.movie.player.find(name) {
                    if let Some(i) = fm.movie.player.inst_mut(id) {
                        i.tint[3] = if shown { 255 } else { 0 };
                    }
                }
            }
            {
                match (crate::menu3d::read_scene(&mut fm.movie.player), self.ui_meshes.as_mut()) {
                    // (the movie's world meshes - the cursor - through the map camera)
                    (Some(scene), Some(meshes)) => crate::menu3d::paint(ctx, &lp, meshes, &mut fm.movie, &scene, map_vp.filter(|_| self.hm.is_some()).map(|vp| crate::menu3d::View { vp, frame: glam::Mat4::IDENTITY })),
                    _ => fm.movie.paint(ctx, &lp),
                }
            }
        }
        if let Some(fp) = self.flash_prompt.as_mut() {
            fp.show(&prompts, None, None);
            fp.movie.player.update(dt);
            fp.movie.paint(ctx, &lp);
        }
    }
}

impl Game {
    /// The picture of vinyl shape `shape` (group * 1000 + index, by `vinyl_shapes.list`).
    fn vinyl_thumb(&self, shape: u16) -> Option<std::path::PathBuf> {
        let root = std::path::Path::new(&self.data_dir);
        let text = std::fs::read_to_string(root.join("tune/vehicle/vinyl_shapes.list")).ok()?;
        let mut cats: Vec<(String, Vec<String>)> = Vec::new();
        for l in text.lines().map(|l| l.trim()) {
            if l.starts_with("vnyl") {
                if let Some(c) = cats.last_mut() {
                    c.1.push(l.to_ascii_lowercase());
                }
            } else if !l.is_empty() && l.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
                cats.push((l.to_string(), Vec::new()));
            }
        }
        let (cat, names) = cats.get(shape as usize / 1000)?;
        let name = names.get(shape as usize % 1000)?;
        let f = root.join("resources/ui/textures/garage/dyn_vinyls").join(cat).join(format!("{name}.r9"));
        f.exists().then_some(f)
    }

    /// A page of the garage movie drawn flat with the prompt bar under it (the vinyl
    /// editor's pages). False without the movie.
    fn garage_flat_page(&mut self, ctx: &egui::Context, page: &crate::flash_ui::MenuPage, prompts: &[(String, i32)]) -> bool {
        self.garage_flat_page_with(ctx, page, prompts, &[])
    }

    /// `pictures`: clips that take a picture from a file (keys 30..), or are hidden
    /// without one - set after the movie's own frame scripts, which show them again.
    fn garage_flat_page_with(&mut self, ctx: &egui::Context, page: &crate::flash_ui::MenuPage, prompts: &[(String, i32)], pictures: &[(String, Option<std::path::PathBuf>)]) -> bool {
        let (Some(fg), Some(meshes)) = (self.flash_garage.as_mut(), self.ui_meshes.as_mut()) else { return false };
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        fg.show(page);
        fg.movie.player.update(dt);
        for (k, (path, file)) in pictures.iter().enumerate() {
            let shown = file.as_ref().map_or(false, |f| fg.movie.swap_texture(ctx, path, 30 + k as u32, f));
            if std::env::var("MCLA_MENU3D_LOG").is_ok() && k == 0 {
                let p = &fg.movie.player;
                if let Some(m) = p.find("screen.menu") {
                    for &c in p.inst(m).map(|i| i.children.values().copied().collect::<Vec<_>>()).unwrap_or_default().iter() {
                        if let Some(ci) = p.inst(c) {
                            if ci.tint[3] > 0 {
                                log::info!("flat page: menu child {:?} #{} alpha {}", &*ci.name, ci.character, ci.tint[3]);
                            }
                        }
                    }
                }
                if let Some(m) = p.find("screen.menu.VE_layers_layer.slot0") {
                    for &c in p.inst(m).map(|i| i.children.values().copied().collect::<Vec<_>>()).unwrap_or_default().iter() {
                        if let Some(ci) = p.inst(c) {
                            log::info!("flat page: slot0 child {:?} #{} alpha {} swap {}", &*ci.name, ci.character, ci.tint[3], ci.tex_swap);
                        }
                    }
                }
            }
            if std::env::var("MCLA_MENU3D_LOG").is_ok() && k < 2 {
                log::info!("flat page: {} found {:?} file {:?} shown {}", path, fg.movie.player.find(path), file, shown);
            }
            if let Some(id) = fg.movie.player.find(path) {
                if let Some(i) = fg.movie.player.inst_mut(id) {
                    i.tint[3] = if shown { 255 } else { 0 };
                }
            }
        }
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_garage"));
        ctx.move_to_top(layer);
        let lp = ctx.layer_painter(layer);
        match crate::menu3d::read_scene(&mut fg.movie.player) {
            Some(scene) => crate::menu3d::paint_scene(ctx, &lp, meshes, &mut fg.movie, &scene, None, false),
            None => fg.movie.paint(ctx, &lp),
        }
        if let Some(fp) = self.flash_prompt.as_mut() {
            fp.show(prompts, Some(self.money), None);
            fp.movie.player.update(dt);
            fp.movie.paint(ctx, &lp);
        }
        true
    }

    /// The vinyl editor on the garage movie's own pages (references `t47`..`t49`,
    /// `u07`..`u28`): CHOOSE GROUP, CHOOSE LAYER (ACCEPT = the shape strip, X = EDIT
    /// LAYER, Y = LAYER OPTIONS), the shape strip with its groups as tabs, and EDIT
    /// LAYER's tabs POSITION / SCALE / ROTATE / SKEW / MASK / COLOR.
    fn vinyl_front_ui(&mut self, ctx: &egui::Context, keys: [bool; 8]) {
        use crate::flash_ui::{MenuLayout, MenuPage, MenuRow, ICON_A, ICON_B, ICON_LB_RB, ICON_X, ICON_Y};
        let [left, right, up, down, enter, back, tab_prev, tab_next] = keys;
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).filter(|v| !v.contains('<')).unwrap_or_else(|| d.to_string());
        let mut ed = self.vinyl_edit.clone().unwrap_or_default();
        let car = self.vinyl_for.clone();
        let zone = ed.zone.min(4);
        let cap = crate::vinyl::ZONE_CAPACITY[zone];
        let mut livery = self.liveries.get(&car).cloned().unwrap_or_default();
        let before = livery.clone();
        let count = livery.zones[zone].len();
        let d: isize = right as isize - left as isize;
        let dv: isize = up as isize - down as isize;
        let cats: Vec<(String, usize)> = self.vinyl_cats.clone();
        let x_key = ctx.input(|i| i.key_pressed(egui::Key::X));
        let y_key = ctx.input(|i| i.key_pressed(egui::Key::Y));
        let fine = ctx.input(|i| i.modifiers.shift);
        let mut close = false;
        let mut warn = false;
        match ed.page {
            0 => {
                if d != 0 {
                    ed.zone = (zone as isize + d).rem_euclid(5) as usize;
                    ed.layer = 0;
                }
                if enter {
                    ed.page = 1;
                } else if back {
                    close = true;
                }
            }
            1 => {
                // (the cursor runs over the layers there are and one free slot)
                let n = (count + 1).min(cap).max(1);
                if d != 0 {
                    ed.layer = (ed.layer as isize + d).rem_euclid(n as isize) as usize;
                }
                ed.layer = ed.layer.min(n - 1);
                if enter {
                    // ACCEPT: the shape strip - for a free slot a new layer to try the shapes on
                    if ed.layer >= count {
                        livery.zones[zone].push(crate::vinyl::Layer { shape: 0, rot: crate::vinyl::base_rot(zone).rem_euclid(360.0), ..Default::default() });
                        ed.added = true;
                        ed.undo_shape = None;
                    } else {
                        ed.added = false;
                        ed.undo_shape = Some(livery.zones[zone][ed.layer].shape);
                    }
                    ed.page = 2;
                } else if x_key {
                    // EDIT LAYER: only a layer with a vinyl on it
                    if ed.layer < count {
                        ed.page = 3;
                        ed.tab = 0;
                    } else {
                        warn = true;
                    }
                } else if y_key {
                    ed.page = 4;
                    ed.tab = 0;
                } else if back {
                    ed.page = 0;
                }
            }
            2 => {
                if let Some(l) = livery.zones[zone].get_mut(ed.layer) {
                    let nc = cats.len().max(1) as isize;
                    let mut c = l.shape as isize / 1000;
                    let mut k = l.shape as isize % 1000;
                    if tab_prev || tab_next {
                        c = (c + tab_next as isize - tab_prev as isize).rem_euclid(nc);
                        k = 0;
                    }
                    let m = cats.get(c as usize).map_or(1, |x| x.1.max(1)) as isize;
                    k = (k + d).rem_euclid(m);
                    l.shape = (c * 1000 + k) as u16;
                }
                if enter {
                    ed.page = 1;
                } else if back {
                    match (ed.added, ed.undo_shape) {
                        (true, _) => {
                            livery.zones[zone].pop();
                        }
                        (false, Some(sh)) => {
                            if let Some(l) = livery.zones[zone].get_mut(ed.layer) {
                                l.shape = sh;
                            }
                        }
                        _ => {}
                    }
                    ed.page = 1;
                }
            }
            3 => {
                if tab_prev || tab_next {
                    ed.tab = (ed.tab as isize + tab_next as isize - tab_prev as isize).rem_euclid(6) as usize;
                }
                if let Some(l) = livery.zones[zone].get_mut(ed.layer) {
                    let step = if fine { 0.001 } else { 0.01 };
                    match ed.tab {
                        0 => {
                            l.pos[0] = (l.pos[0] + d as f32 * step).clamp(-0.6, 0.6);
                            l.pos[1] = (l.pos[1] + dv as f32 * step).clamp(-0.4, 0.4);
                        }
                        1 => {
                            l.scale[0] = (l.scale[0] + d as f32 * step).clamp(-1.2, 1.2);
                            l.scale[1] = (l.scale[1] + dv as f32 * step).clamp(-1.2, 1.2);
                        }
                        2 => l.rot = (l.rot + d as f32 * if fine { 1.0 } else { 5.0 }).rem_euclid(360.0),
                        3 => l.skew = (l.skew + d as f32 * 0.05).clamp(-2.0, 2.0),
                        4 if d != 0 => {
                            use crate::vinyl::Blend;
                            const KINDS: [Blend; 3] = [Blend::Normal, Blend::AlphaCutout, Blend::NegativeCutout];
                            let k = KINDS.iter().position(|b| *b == l.blend).unwrap_or(0) as isize;
                            l.blend = KINDS[(k + d).rem_euclid(3) as usize];
                        }
                        5 => {
                            if d != 0 {
                                let at = VINYL_COLORS.iter().position(|c| c[0] == l.color[0] && c[1] == l.color[1] && c[2] == l.color[2]).unwrap_or(0) as isize;
                                let c = VINYL_COLORS[(at + d).rem_euclid(VINYL_COLORS.len() as isize) as usize];
                                l.color = [c[0], c[1], c[2], l.color[3]];
                            }
                            // (up / down: the opacity meter on the cursor)
                            l.color[3] = (l.color[3] as i32 + dv as i32 * 13).clamp(13, 255) as u8;
                        }
                        _ => {}
                    }
                }
                if enter || back {
                    ed.page = 1;
                }
            }
            _ => {
                const N: usize = 7;
                if dv != 0 {
                    ed.tab = (ed.tab as isize - dv).rem_euclid(N as isize) as usize;
                }
                if enter {
                    match ed.tab {
                        // COPY / CUT / DELETE / PASTE / INSERT work on the layer under the cursor
                        // (the original's marks are not kept: approx.)
                        0 | 1 => {
                            self.vinyl_clip = livery.zones[zone].get(ed.layer).cloned();
                            if ed.tab == 1 && ed.layer < count {
                                livery.zones[zone].remove(ed.layer);
                            }
                        }
                        2 if ed.layer < count => {
                            livery.zones[zone].remove(ed.layer);
                        }
                        3 | 4 => {
                            if let (Some(l), true) = (self.vinyl_clip.clone(), livery.zones[zone].len() < cap) {
                                let at = if ed.tab == 4 { ed.layer.min(livery.zones[zone].len()) } else { livery.zones[zone].len() };
                                livery.zones[zone].insert(at, l);
                            }
                        }
                        _ => {}
                    }
                    ed.page = 1;
                } else if back {
                    ed.page = 1;
                }
            }
        }
        if let Ok(v) = std::env::var("MCLA_VINYL_PAGE") {
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let f: Vec<usize> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                ed.page = f.first().copied().unwrap_or(0).min(4) as u8;
                ed.tab = f.get(1).copied().unwrap_or(0);
                if ed.page == 2 && livery.zones[zone].is_empty() {
                    livery.zones[zone].push(crate::vinyl::Layer { shape: 0, rot: crate::vinyl::base_rot(zone).rem_euclid(360.0), ..Default::default() });
                    ed.added = true;
                }
            }
        }
        if livery != before {
            self.liveries.insert(car.clone(), livery.clone());
            self.livery_rev += 1;
            self.vinyls.insert(car.clone(), "custom".to_string());
            if !self.vinyl_list.iter().any(|v| v == "custom") {
                self.vinyl_list.insert(0, "custom".to_string());
            }
        }
        let count = livery.zones[zone].len();
        let layer = livery.zones[zone].get(ed.layer).cloned();
        let row = |l: String| MenuRow { label: l, value: String::new(), enabled: true, part: None };
        let back_p = (st("prompt_back", "BACK"), ICON_B);
        // (prompt icons: 12 LS, 13/14 LS with arrows, 16 RS, 19 the plain d-pad, 20..22 with arrows)
        let cam_p = (st("VE_change_camHeight", "CAMERA HEIGHT"), 19);
        let mut pictures: Vec<(String, Option<std::path::PathBuf>)> = Vec::new();
        let strip = |this: &Self, first: usize| -> Vec<(String, Option<std::path::PathBuf>)> {
            (0..10).map(|k| (format!("screen.menu.VE_layers_layer.swatches{:02}", k + 1), livery.zones[zone].get(first + k).and_then(|l| this.vinyl_thumb(l.shape)))).collect()
        };
        let first = self.flash_garage.as_ref().and_then(|fg| fg.movie.player.get_global("ScrollOffset")).map_or(0.0, |v| v.to_f32()).max(0.0) as usize;
        let (page, prompts) = match ed.page {
            0 => (
                MenuPage { layout: MenuLayout::VinylGroup, icon: 10, title: st("VE_Group_Select", "CHOOSE GROUP"), rows: (0..5).map(|_| row(String::new())).collect(), select: zone, ..MenuPage::default() },
                vec![(st("VE_Group_Edit", "EDIT GROUP"), ICON_A), back_p],
            ),
            1 => {
                pictures = strip(self, first);
                (
                    MenuPage {
                        layout: MenuLayout::VinylLayers { group: zone as i32 },
                        icon: 10,
                        title: st("VE_Vinyls", "CHOOSE LAYER"),
                        // (the layers there are and one free slot; the movie fades the slots beyond)
                        rows: (0..(count + 1).min(cap).max(1)).map(|_| row(String::new())).collect(),
                        select: ed.layer,
                        ..MenuPage::default()
                    },
                    vec![(st("VE_Layer_Select", "ACCEPT"), ICON_A), back_p, (st("VE_Layer_Options", "LAYER OPTIONS"), ICON_Y), (st("VE_Layer_Edit", "EDIT LAYER"), ICON_X), (st("VE_Mark_layer", "MARK/UNMARK"), ICON_LB_RB)],
                )
            }
            2 => {
                let shape = layer.as_ref().map_or(0, |l| l.shape) as usize;
                let (c, k) = (shape / 1000, shape % 1000);
                let n = cats.get(c).map_or(1, |x| x.1.max(1));
                let offset = k.saturating_sub(6).min(n.saturating_sub(7));
                pictures = (0..7).map(|i| (format!("screen.menu.VE_shapes_layer.slot{i}.sh_dyntexture"), (offset + i < n).then(|| self.vinyl_thumb((c * 1000 + offset + i) as u16)).flatten())).collect();
                (
                    MenuPage {
                        layout: MenuLayout::VinylShapes { offset: offset as i32 },
                        icon: 10,
                        tabs: cats.iter().map(|(name, _)| vinyl_group_name(&st, name)).collect(),
                        // (GarageTabs icon ids: 201.. the shape groups in the list's order)
                        tab_icons: (0..cats.len() as i32).map(|i| std::env::var("MCLA_VE_ICON").ok().and_then(|v| v.parse::<i32>().ok()).unwrap_or(201) + i).collect(),
                        cur_tab: c,
                        title: cats.get(c).map(|(name, _)| vinyl_group_name(&st, name)).unwrap_or_default(),
                        rows: (0..n).map(|_| row(String::new())).collect(),
                        select: k,
                        ..MenuPage::default()
                    },
                    vec![back_p, (st("prompt_accept", "ACCEPT"), ICON_A), (st("VE_change_shape", "CHANGE SHAPE"), 11), (st("VE_Prev_Next_Category", "PREV/NEXT"), ICON_LB_RB), cam_p],
                )
            }
            3 => {
                let l = layer.clone().unwrap_or_default();
                let names = [("Position", "POSITION"), ("Scale", "SCALE"), ("Rotate", "ROTATE"), ("Skew", "SKEW"), ("Alpha", "MASK"), ("Color", "COLOR")];
                use crate::vinyl::Blend;
                // (approx.) the figures as whole numbers, in the port's own measures
                let (template, dims, colors, what): (i32, [i32; 2], Vec<u32>, Option<(String, i32)>) = match ed.tab {
                    0 => (104, [(l.pos[0] * 100.0).round() as i32, (l.pos[1] * 100.0).round() as i32], Vec::new(), Some((st("VE_changePosition", "CHANGE POSITION"), 22))),
                    1 => (104, [(l.scale[0] * 10.0).round() as i32, (l.scale[1] * 10.0).round() as i32], Vec::new(), Some((st("VE_changeScale", "CHANGE SCALE"), 11))),
                    2 => (105, [l.rot.round() as i32, 0], Vec::new(), Some((st("Rotate", "ROTATE"), 11))),
                    3 => (105, [(l.skew * 10.0).round() as i32, 0], Vec::new(), Some((st("Skew", "SKEW"), 11))),
                    4 => (116, [match l.blend { Blend::Normal => 0, Blend::AlphaCutout => 1, _ => 2 }, 0], Vec::new(), Some((st("VE_changeMask", "CHANGE MASK"), 11))),
                    _ => (103, [0, 0], VINYL_COLORS.iter().map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect(), None),
                };
                pictures = vec![("screen.menu.VE_area_layer_mini.shapes".to_string(), self.vinyl_thumb(l.shape))];
                let n_rows = if colors.is_empty() { 1 } else { colors.len() };
                let select = if colors.is_empty() { 0 } else { VINYL_COLORS.iter().position(|c| c[0] == l.color[0] && c[1] == l.color[1] && c[2] == l.color[2]).unwrap_or(0) };
                let mut prompts = vec![back_p, (st(if ed.tab == 5 { "VE_accept_color" } else { "prompt_accept" }, "ACCEPT"), ICON_A)];
                if let Some(w) = what {
                    prompts.push(w);
                }
                prompts.push(cam_p);
                (
                    MenuPage {
                        layout: MenuLayout::VinylEditTab { template, dims, colors },
                        icon: 10,
                        tabs: names.iter().map(|(k, d2)| st(k, d2)).collect(),
                        // (GarageTabs icon ids: 32 position, 33 scale, 34 rotate, 35 skew, 36 mask, 31 the palette)
                        tab_icons: vec![32, 33, 34, 35, 36, 31],
                        cur_tab: ed.tab,
                        title: st(names[ed.tab.min(5)].0, names[ed.tab.min(5)].1),
                        rows: (0..n_rows).map(|_| row(String::new())).collect(),
                        select,
                        ..MenuPage::default()
                    },
                    prompts,
                )
            }
            _ => {
                pictures = strip(self, first);
                let rows = [("VE_Copy_Layer", "COPY LAYER(S)"), ("VE_Cut_Layer", "CUT LAYER(S)"), ("VE_Delete_Layer", "DELETE LAYER(S)"), ("VE_Paste_Layer", "PASTE LAYER(S)"), ("VE_Insert_Layer", "INSERT LAYER(S)"), ("VE_Mark_All_Layers", "MARK ALL LAYERS"), ("VE_UnMark_All_Layers", "UNMARK ALL LAYERS")];
                (
                    MenuPage {
                        layout: MenuLayout::VinylOptions { group: zone as i32 },
                        icon: 10,
                        title: st("VE_Vinyls", "CHOOSE LAYER"),
                        rows: rows.iter().map(|(k, d2)| row(st(k, d2))).collect(),
                        select: ed.tab.min(6),
                        ..MenuPage::default()
                    },
                    vec![(st("prompt_accept", "ACCEPT"), ICON_A), (st("VE_Hide_Options", "HIDE OPTIONS"), ICON_B), (st("VE_Mark_layer", "MARK/UNMARK"), ICON_LB_RB)],
                )
            }
        };
        self.garage_flat_page_with(ctx, &page, &prompts, &pictures);
        if warn {
            // ("You must place a vinyl before you can edit a layer.")
            self.open_message_box("Layer_NoShapeError", MenuAction::None);
        }
        if close {
            self.vinyl_edit = None;
            self.livery_rev += 1;
            self.dirty = true;
        } else {
            self.vinyl_edit = Some(ed);
        }
    }
}

/// The colour strip of the vinyl editor's COLOR tab (approx.: the port's own set, laid
/// out like the original's - light to dark, then the hues).
const VINYL_COLORS: [[u8; 3]; 23] = [
    [255, 255, 255], [214, 208, 170], [205, 200, 150], [20, 22, 60], [30, 40, 120], [45, 80, 130], [100, 130, 160], [120, 180, 230], [90, 75, 50], [120, 95, 60], [190, 100, 90], [70, 20, 25],
    [130, 120, 135], [170, 140, 160], [45, 70, 60], [90, 100, 30], [60, 130, 110], [20, 150, 120], [40, 150, 40], [90, 95, 100], [125, 130, 135], [20, 20, 20], [200, 30, 30],
];

/// A shape group's name as the tabs show it (`VE_<Group>` in the string table).
fn vinyl_group_name(st: &dyn Fn(&str, &str) -> String, group: &str) -> String {
    let mut c = group.chars();
    let cap: String = c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default();
    let up = group.to_uppercase().replace('_', " ");
    let key = match group {
        "ammftr" => "VE_amMftr".to_string(),
        "mftrtxt" => "VE_MftrTxt".to_string(),
        "mftr" => "VE_Mftr".to_string(),
        _ => format!("VE_{}", cap),
    };
    st(&key, &up)
}

impl Game {
    /// After a race, on the pause movie: the race-over menu (the pause panel with the
    /// `RO_*` rows) and, behind RACE RESULTS, the results table (`template_ResultsPanel`).
    /// (approx.) No reference capture of either yet: titles and columns follow the
    /// state file and the template.
    /// The rims' finish as MATERIALS/PAINT has it (`Vehicles::player_rim_finish`): by zone
    /// - spoke A, spoke B, lip - the colour and 1 painted, 2 chrome, 3 brushed.
    pub fn rim_finish(&self) -> [[f32; 4]; 3] {
        let mut out = [[0.0; 4]; 3];
        for k in 0..3 {
            let c = match self.slot_hsv[6 + k] {
                Some(h) => {
                    let c = paint_hsv_to_rgb(h);
                    [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2)]
                }
                None => {
                    let p = PALETTE[self.rim_cols[k] % PALETTE.len()];
                    [p[0], p[1], p[2]]
                }
            };
            // (the meshes' zone 2 is the thin outer ring - the lip - and zone 3 the second spokes)
            // (w: 1 painted, 2 chrome, 3 brushed metal, 4 carbon fibre - the record's material 2)
            out[[0, 2, 1][k]] = [c[0], c[1], c[2], (self.rim_mats[k].min(3) + 1) as f32];
        }
        out
    }

    /// The cabin's colours as INTERIOR COLORS has them (rows INTERIOR COLOR A, B, C), for
    /// `Vehicles::player_interior`; w 0 where the row was never changed (the stock cabin).
    /// Then GAUGE COLOR A and B, which colour the trim around the dials and the carpet.
    pub fn interior_finish(&self) -> [[f32; 4]; 5] {
        let mut out = [[0.0; 4]; 5];
        // (rows of the page: 0 neon, 1 / 2 gauge A / B, 3..5 interior A..C)
        for (k, row) in [3usize, 4, 5, 1, 2].into_iter().enumerate() {
            let stock = if row < 3 { 4 } else { 9 };
            out[k] = match self.slot_hsv[row] {
                Some(h) => {
                    let c = paint_hsv_to_rgb(h);
                    [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), 1.0]
                }
                None if self.int_colors[row] != stock => {
                    let p = PALETTE[self.int_colors[row] % PALETTE.len()];
                    [p[0], p[1], p[2], 1.0]
                }
                None => [0.0; 4],
            };
        }
        out
    }

    /// The colour the shop has in hand, as the car's paint is given (`paint_color` without
    /// the body's own put in its place).
    fn paint_in_hand(&self) -> [f32; 4] {
        match self.pick_slot.map_or(self.paint_hsv, |p| p.1) {
            Some(h) => {
                let c = paint_hsv_to_rgb(h);
                [c[0], c[1], c[2], 1.0]
            }
            None => PALETTE[self.color_idx % PALETTE.len()],
        }
    }

    /// PAINT AREA: the shop's colour rows now work on area `to`. The body's colour is put
    /// by while a part's is in hand; a part not painted yet starts from the body's.
    pub fn set_paint_area(&mut self, to: usize) {
        let to = to % PAINT_AREAS.len();
        if to == self.paint_area {
            return;
        }
        if self.paint_area == 0 {
            self.paint_body = Some((self.color_idx, self.paint_hsv));
        }
        self.paint_area = to;
        self.paint_open = None;
        self.paint_pick = None;
        if to == 0 {
            if let Some((idx, hsv)) = self.paint_body.take() {
                self.color_idx = idx;
                self.paint_hsv = hsv;
            }
        } else {
            let own = PAINT_AREAS[to].2.iter().find_map(|k| self.cfg.part_paint.get(*k).copied());
            match (own, self.paint_body) {
                // (the part's colour as the screen shows it)
                (Some(c), _) => self.paint_hsv = Some(paint_rgb_to_hsv([(c[0] as f32 / 255.0).powf(2.2), (c[1] as f32 / 255.0).powf(2.2), (c[2] as f32 / 255.0).powf(2.2), 1.0])),
                (None, Some((idx, hsv))) => {
                    self.color_idx = idx;
                    self.paint_hsv = hsv;
                }
                _ => {}
            }
        }
    }

    /// The part in the paint shop's hands takes the colour shown, as it changes.
    pub fn sync_part_paint(&mut self) {
        // (out of the shop: the body's colour is the shop's again)
        if self.paint_area != 0 && (self.mode != Mode::Garage || self.garage_menu != 3) {
            self.set_paint_area(0);
            return;
        }
        if self.paint_area == 0 || self.paint_body.is_none() {
            return;
        }
        let c = self.paint_in_hand();
        let body = self.paint_color();
        let same = (0..3).all(|i| (c[i] - body[i]).abs() < 0.004);
        let rgb = [(c[0].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8, (c[1].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8, (c[2].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8];
        for k in PAINT_AREAS[self.paint_area].2 {
            let had = self.cfg.part_paint.get(*k).copied();
            // (a part left in the body's colour is not a painted part)
            if same && had.is_none() {
                continue;
            }
            if had != Some(rgb) {
                self.cfg.part_paint.insert(k.to_string(), rgb);
                self.dirty = true;
            }
        }
    }

    /// The tire set the garage's TIRES list has on the car (`resources/tires/textures/<name>`,
    /// the rows' own names) - under the list's cursor while it is open; empty for none.
    pub fn tire_set(&self) -> String {
        let car = self.dir_name(self.current_car).to_ascii_lowercase();
        let bike = self.catalog.get(&self.dir_name(self.current_car)).map_or(false, |c| c.class.to_ascii_lowercase().contains("bike"));
        let Some(k) = XPARTS.iter().position(|x| x.1 == "tires") else { return String::new() };
        let index = match self.garage_xpart {
            Some((open, cur)) if open == k && self.selected_car == self.current_car => self.xpart_rows(k).get(cur).map(|r| r.3),
            _ => self.xparts.get(&car).and_then(|m| m.get("tires")).copied(),
        };
        index.and_then(|i| self.wheel_lists[bike as usize].iter().find(|r| r.3 == i)).map(|r| r.0.to_ascii_lowercase()).unwrap_or_default()
    }

    /// The brake model the garage's BRAKES list has on the car (`resources/brakes/<name>`,
    /// the rows' own names in Wheels.sc.xml) - under the list's cursor while it is open -
    /// or `None` for the car's own.
    pub fn brake_model(&self) -> Option<String> {
        let car = self.dir_name(self.current_car).to_ascii_lowercase();
        let k = XPARTS.iter().position(|x| x.1 == "brakes")?;
        let index = match self.garage_xpart {
            Some((open, cur)) if open == k && self.selected_car == self.current_car => self.xpart_rows(k).get(cur).map(|r| r.3)?,
            // (no row of the menu chosen: the model the vehicle's record names, `mc4`)
            _ => match self.xparts.get(&car).and_then(|m| m.get("brakes")).copied() {
                Some(i) if i != 0 => i,
                _ => return self.mc4_brake_name(&car),
            },
        };
        self.wheel_lists[2].iter().find(|r| r.3 == index && index != 0).map(|r| r.0.to_ascii_lowercase()).or_else(|| self.mc4_brake_name(&car))
    }

    /// A hangout's panels as the game shows them on pulling in.
    ///
    /// A local hangout (reference `hg_series_0`..`4`, THE GREAT PALACE): the place's name,
    /// "Compete in a SERIES of races out of this HANGOUT. First to win 3 races wins.",
    /// DIFFICULTY (EASIEST / MEDIUM / HARD / HARDEST, left and right, round), COMPLETION
    /// REWARD and RACE WINS NEEDED: 3; EXIT HANGOUT (B), ACCEPT (A). The reward is one and
    /// a half times the race money of the difficulty ($7,800 / 9,750 / 11,700 / 13,650 with
    /// the 100 % save's R8: the captured figures).
    ///
    /// A wager hangout (`hg_wager_1`..`4`, MACK'S BURGER): "Select the type of race you
    /// want to enter." - WAGER RACE, PINKSLIP RACE - and behind WAGER RACE the WAGER AMOUNT
    /// panel (`hg_wager_a0`..`a6`): DIFFICULTY (opens on MEDIUM) and the amount on a slider
    /// of eleven places between its least and most, the player's money on the foot; BACK
    /// (B), ACCEPT (A).
    fn hangout_ui(&mut self, ctx: &egui::Context, up: bool, down: bool, left: bool, right: bool, enter: bool, back: bool) -> Option<MenuAction> {
        use crate::flash_ui::{choice_list, MenuLayout, MenuPage, MenuRow, ICON_A, ICON_B};
        let key = self.hangout_key.clone()?;
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_string()).filter(|v| !v.contains('<')).unwrap_or_else(|| d.to_string());
        let wager = key.contains("_wager_");
        let diffs: Vec<String> = [("Navmap_Difficulty1", "EASIEST"), ("Navmap_Difficulty2", "MEDIUM"), ("Navmap_Difficulty3", "HARD"), ("Navmap_Difficulty4", "HARDEST")].iter().map(|(k, d)| st(k, d).to_uppercase()).collect();
        let class = crate::rewards::perf_class(self.player_perf());
        let commas = |v: i64| { let s = v.to_string(); let mut o = String::new(); for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { o.push(','); } o.push(c); } o };
        let exit = (st("prompt_exit_hangout", "EXIT HANGOUT").to_uppercase(), ICON_B);
        let accept = (st("prompt_accept", "ACCEPT").to_uppercase(), ICON_A);
        // (page 0: the place's own panel; 1: WAGER AMOUNT)
        // (test hook: MCLA_HANGOUT_PAGE="page,row" opens the panel on that page - step 21)
        if let Some(v) = std::env::var("MCLA_HANGOUT_PAGE").ok() {
            static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let f: Vec<usize> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                self.hangout_page = (f.first().copied().unwrap_or(0) as u8, f.get(1).copied().unwrap_or(0));
            }
        }
        let (page_no, cur) = self.hangout_page;
        // (step 21: the hangout's pink slip profiles - its own car while its mission runs, the
        // cars held in the lot; the player needs a second vehicle to stake one)
        let pink_profiles = if wager { self.mc4_pink_profiles(&key) } else { Vec::new() };
        let pink_open = !pink_profiles.is_empty() && self.owned.len() >= 2;
        let stake_of = |this: &Self, d: usize, place: usize| -> (i64, i64, i64) {
            // (measured on the R8 at the top of the career: 5,000 / 9,100 / 14,560 / 21,840 in
            // the middle, steps of 200 / 360 / 580 / 875 - the race money of the difficulty
            // times 25/26, 7/5, 28/15, 12/5; other cars and ranks in proportion)
            const MID: [f32; 4] = [25.0 / 26.0, 1.4, 28.0 / 15.0, 2.4];
            const STEP: [f32; 4] = [200.0 / 5000.0, 360.0 / 9100.0, 580.0 / 14560.0, 875.0 / 21840.0];
            let mid = (crate::rewards::race_money(this.rep, class, d, 10.0) * MID[d.min(3)] / 10.0).round() * 10.0;
            let step = ((mid * STEP[d.min(3)]) / 5.0).round() * 5.0;
            ((mid + (place as f32 - 5.0) * step) as i64, (mid - 5.0 * step) as i64, (mid + 5.0 * step) as i64)
        };
        let delivery = key.contains("delivery");
        // (the tournament's and the payback's panels - mcTournamentForm / mcPaybackForm: `mc4_event_panel`)
        let s15b_event = key.contains("_tournament") || key == "be_payback";
        // (the time trial man's panel - form 65, sub_8267B370: a list of four rows, one a
        // vehicle, with the columns CAR_COL_NAME, TARGET_TIME_LABEL, BEST_TIME_COL_NAME; the
        // title is the district's key, the text TIMETRIAL_HOOKMAN_DESC; ACCEPT / BACK. No
        // capture of it exists: it is put on the template the tournament's panel uses, the
        // column names as a first, grey row.)
        let s19h_tt = Self::mc4_tt_panel_district(&key);
        // (the SERVICE STATION menu - form 61, mcGasStationMenuForm: SimpleListForm with the title
        // "Gas Station", no description, rows FULL FIX and REFILL NITRO - the second only with
        // nitrous fitted -, ACCEPT / BACK, the base template 2; career2 `gasphoto/notes.md`)
        let gas = key == GAS_MENU;
        let page = if gas {
            let cur = (if down { cur + 1 } else if up { cur.saturating_sub(1) } else { cur }).min(1);
            self.hangout_page = (0, cur);
            MenuPage {
                layout: MenuLayout::List,
                title: st("Gas Station", "SERVICE STATION").to_uppercase(),
                rows: vec![
                    MenuRow { label: st("GasStation_FullFix", "FULL FIX").to_uppercase(), value: String::new(), enabled: true, part: None },
                    MenuRow { label: st("GasStation_RefillNitro", "REFILL NITRO").to_uppercase(), value: String::new(), enabled: self.opt_x.s23b.gas_nitro, part: None },
                ],
                select: cur,
                prompts: vec![accept.clone(), (st("prompt_back", "BACK").to_uppercase(), ICON_B)],
                ..MenuPage::default()
            }
        } else if let Some(d) = s19h_tt {
            let rows = self.mc4_tt_rows(d);
            let n = rows.len().max(1);
            let mut cur = self.hangout_page.1.min(n - 1);
            // (test hook: MCLA_TT_ROW=0..3 puts the cursor on that row)
            if let Some(r) = std::env::var("MCLA_TT_ROW").ok().and_then(|v| v.parse::<usize>().ok()) {
                cur = r.min(n - 1);
            }
            // (the cursor goes to enabled rows only)
            if up {
                cur = (0..cur).rev().find(|&k| rows[k].enabled).unwrap_or(cur);
            }
            if down {
                cur = (cur + 1..n).find(|&k| rows[k].enabled).unwrap_or(cur);
            }
            self.hangout_page = (0, cur);
            let mut list = vec![MenuRow { label: st("CAR_COL_NAME", "CAR").to_uppercase(), value: format!("{}     {}", st("TARGET_TIME_LABEL", "TARGET TIME").to_uppercase(), st("BEST_TIME_COL_NAME", "BEST TIME").to_uppercase()), enabled: false, part: None }];
            list.extend(rows.iter().map(|r| MenuRow { label: r.car.to_uppercase(), value: if r.enabled { format!("{}     {}", r.target, r.best) } else { String::new() }, enabled: r.enabled, part: None }));
            MenuPage {
                layout: MenuLayout::Hangout { description: st("TIMETRIAL_HOOKMAN_DESC", ""), slider: None, money: None },
                icon: std::env::var("MCLA_HANGOUT_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(HANGOUT_ICON),
                title: st(crate::mc4::TIME_TRIALS[d].key, "TIME TRIAL").to_uppercase(),
                rows: list,
                select: cur + 1,
                prompts: vec![accept.clone(), (st("prompt_back", "BACK").to_uppercase(), ICON_B)],
                list_max: 5,
                ..MenuPage::default()
            }
        } else if s15b_event {
            if left {
                self.hangout_diff = (self.hangout_diff + 3) % 4;
            }
            if right {
                self.hangout_diff = (self.hangout_diff + 1) % 4;
            }
            // (test hook: MCLA_HANGOUT_DIFF=0..3 shows the panel at that difficulty)
            if let Some(d) = std::env::var("MCLA_HANGOUT_DIFF").ok().and_then(|v| v.parse::<usize>().ok()) {
                self.hangout_diff = d.min(3);
            }
            let p = self.mc4_event_panel(&key, self.hangout_diff);
            let reward = match (&p.reward_car, p.reward_money) {
                (Some(car), _) => st(&format!("{}_shrt", car.to_ascii_lowercase()), &crate::game::pretty_car_name(car)).to_uppercase(),
                (None, Some(m)) => format!("${}", commas(m)),
                _ => String::new(),
            };
            MenuPage {
                layout: MenuLayout::Hangout { description: st(&p.desc, ""), slider: None, money: None },
                icon: std::env::var("MCLA_HANGOUT_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(HANGOUT_ICON),
                title: st(&p.title, &key).to_uppercase(),
                rows: vec![
                    MenuRow { label: st("Navmap_Difficulty", "DIFFICULTY").to_uppercase(), value: choice_list(self.hangout_diff, &diffs), enabled: true, part: None },
                    MenuRow { label: st(&p.reward_label, "COMPLETION REWARD:").to_uppercase(), value: reward, enabled: true, part: None },
                    MenuRow { label: st(&p.count_label, "TOTAL RACES:").to_uppercase(), value: p.count.to_string(), enabled: true, part: None },
                ],
                select: usize::MAX,
                // (these panels are no hangout's: BACK, as on the delivery's - not captured)
                prompts: vec![accept.clone(), (st("prompt_back", "BACK").to_uppercase(), ICON_B)],
                list_max: 3,
                ..MenuPage::default()
            }
        } else if delivery {
            // DELIVERY (`ref/dl_panel_medium.png`, `dl_diff_0..3`): the text, DIFFICULTY (round,
            // opening on MEDIUM) and DELIVERY REWARD as a range - the race money of the
            // difficulty to twice that, a perfect delivery's ($5,200 - $10,400 / 6,500 - 13,000 /
            // 7,800 - 15,600 / 9,100 - 18,200 on the 100 % save's R8); BACK (B), ACCEPT (A)
            if left {
                self.hangout_diff = (self.hangout_diff + 3) % 4;
            }
            if right {
                self.hangout_diff = (self.hangout_diff + 1) % 4;
            }
            let base = crate::rewards::race_money(self.rep, class, self.hangout_diff, 10.0) as i64;
            MenuPage {
                layout: MenuLayout::Hangout { description: st("DELIVERY_MENU_DESC", "Deliver cars to the right locations without inflicting too much visual damage to earn cash."), slider: None, money: None },
                icon: std::env::var("MCLA_HANGOUT_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(DELIVERY_ICON),
                title: st("DELIVERY_MENU_TITLE", "DELIVERY").to_uppercase(),
                rows: vec![
                    MenuRow { label: st("Navmap_Difficulty", "DIFFICULTY").to_uppercase(), value: choice_list(self.hangout_diff, &diffs), enabled: true, part: None },
                    MenuRow { label: st("REWARD_DELIVERY_LABEL", "DELIVERY REWARD").to_uppercase(), value: format!("${} - ${}", commas(base), commas(base * 2)), enabled: true, part: None },
                ],
                select: std::env::var("MCLA_DL_SEL").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX),
                prompts: vec![accept.clone(), (st("prompt_back", "BACK").to_uppercase(), ICON_B)],
                list_max: 3,
                ..MenuPage::default()
            }
        } else if !wager {
            if left {
                self.hangout_diff = (self.hangout_diff + 3) % 4;
            }
            if right {
                self.hangout_diff = (self.hangout_diff + 1) % 4;
            }
            let reward = (crate::rewards::race_money(self.rep, class, self.hangout_diff, 10.0) * 1.5) as i64;
            MenuPage {
                layout: MenuLayout::Hangout { description: st("LOCAL_HANGOUT_DESC", "Compete in a SERIES of races out of this HANGOUT.  First to win 3 races wins."), slider: None, money: None },
                icon: std::env::var("MCLA_HANGOUT_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(HANGOUT_ICON),
                title: st(&key, "HANGOUT").to_uppercase(),
                rows: vec![
                    MenuRow { label: st("Navmap_Difficulty", "DIFFICULTY").to_uppercase(), value: choice_list(self.hangout_diff, &diffs), enabled: true, part: None },
                    MenuRow { label: st("REWARD_SERIES_LABEL", "COMPLETION REWARD:").to_uppercase(), value: format!("${}", commas(reward)), enabled: true, part: None },
                    MenuRow { label: st("RACES_NEEDED_LABEL", "RACE WINS NEEDED:").to_uppercase(), value: "3".to_string(), enabled: true, part: None },
                ],
                // (no cursor bar: the template lights the first row's text and arrows by itself
                // when nothing is selected; its prompts show in the pause movie, where it lives)
                select: usize::MAX,
                prompts: vec![accept.clone(), exit.clone()],
                list_max: 3,
                ..MenuPage::default()
            }
        } else if page_no == 2 {
            // (the car raced for - the original turns the camera on the profiles' cars and names
            // each, VE_select_vehicle; the port names them on a row - and the warning of
            // the confirmation box: `pinkslip_confirmation`, `_max` with a full lot)
            let n = pink_profiles.len().max(1);
            if left {
                self.opt_x.arcade.pink.pick = (self.opt_x.arcade.pink.pick + n - 1) % n;
            }
            if right {
                self.opt_x.arcade.pink.pick = (self.opt_x.arcade.pink.pick + 1) % n;
            }
            let pick = self.opt_x.arcade.pink.pick.min(n - 1);
            let car = pink_profiles.get(pick).map(|p| self.career.strings.get(&p.car).map(|s| s.to_string()).unwrap_or_else(|| pretty_car_name(&p.car))).unwrap_or_default();
            let warn = if self.mc4_pink_lot_full() { st("pinkslip_confirmation_max", "Are you sure you want to race for pinkslips?") } else { st("pinkslip_confirmation", "Are you sure you want to race for pinkslips?") };
            MenuPage {
                layout: MenuLayout::Hangout { description: warn, slider: None, money: None },
                icon: std::env::var("MCLA_HANGOUT_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(WAGER_ICON),
                title: st("Pinkslip", "PINKSLIP RACE").to_uppercase(),
                rows: vec![MenuRow { label: car.to_uppercase(), value: format!("{} / {}", pick + 1, n), enabled: true, part: None }],
                select: 0,
                prompts: vec![accept.clone(), (st("prompt_back", "BACK").to_uppercase(), ICON_B)],
                list_max: 2,
                ..MenuPage::default()
            }
        } else if page_no == 0 {
            let cur = if up { 0 } else if down { 1 } else { cur.min(1) };
            self.hangout_page = (0, cur);
            MenuPage {
                layout: MenuLayout::Hangout { description: st("wager_hangout_desc", "Select the type of race you want to enter."), slider: None, money: None },
                icon: std::env::var("MCLA_HANGOUT_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(WAGER_ICON),
                title: st(&key, "HANGOUT").to_uppercase(),
                // (PINKSLIP RACE is grey in the capture - the 100 % save has won them all - but
                // the cursor goes there)
                rows: vec![MenuRow { label: st("Wager", "WAGER RACE").to_uppercase(), value: String::new(), enabled: true, part: None }, MenuRow { label: st("Pinkslip", "PINKSLIP RACE").to_uppercase(), value: String::new(), enabled: pink_open, part: None }],
                select: cur,
                prompts: vec![accept.clone(), exit.clone()],
                list_max: 2,
                ..MenuPage::default()
            }
        } else {
            let cur = if up { 0 } else if down { 1 } else { cur.min(1) };
            self.hangout_page = (1, cur);
            if cur == 0 {
                if left {
                    self.hangout_diff = (self.hangout_diff + 3) % 4;
                }
                if right {
                    self.hangout_diff = (self.hangout_diff + 1) % 4;
                }
                if left || right {
                    self.hangout_stake = 5;
                }
            } else {
                if left {
                    self.hangout_stake = self.hangout_stake.saturating_sub(1);
                }
                if right {
                    self.hangout_stake = (self.hangout_stake + 1).min(10);
                }
            }
            let (amount, lo, hi) = stake_of(self, self.hangout_diff, self.hangout_stake);
            MenuPage {
                layout: MenuLayout::Hangout { description: st("WAGER_AMOUNT_DESC", "Lots of guys are looking to race for CASH.  Set your DIFFICULTY and the amount you want to wager."), slider: Some((1, amount, lo, hi)), money: Some(self.money) },
                icon: std::env::var("MCLA_HANGOUT_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(WAGER_ICON),
                title: st("WAGER_AMOUNT_TITLE", "WAGER AMOUNT").to_uppercase(),
                rows: vec![MenuRow { label: st("Navmap_Difficulty", "DIFFICULTY").to_uppercase(), value: choice_list(self.hangout_diff, &diffs), enabled: true, part: None }, MenuRow { label: String::new(), value: amount.to_string(), enabled: true, part: None }, MenuRow { label: String::new(), value: String::new(), enabled: false, part: None }],
                select: cur,
                prompts: vec![accept.clone(), (st("prompt_back", "BACK").to_uppercase(), ICON_B)],
                list_max: std::env::var("MCLA_SLIDER_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(4),
                ..MenuPage::default()
            }
        };
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_navsys"));
        ctx.move_to_top(layer);
        let lp = ctx.layer_painter(layer);
        if let Some(fm) = self.flash_menu.as_mut() {
            fm.show(&page);
            fm.movie.player.set_global("moviepausemode", mcla_flash::value::Value::Int(3));
            if delivery {
                // (the delivery's panel stands beside the car as the pause menu's does, on the
                // pause camera - `dl_panel_medium`; the hangouts' own are flat at the top left)
                fm.movie.player.set_global("cur_camera", mcla_flash::value::Value::Int(std::env::var("MCLA_PAUSE_CAM").ok().and_then(|v| v.parse().ok()).unwrap_or(1)));
                fm.movie.player.set_global("view2d", mcla_flash::value::Value::Int(0));
            }
            fm.movie.player.update(dt);
            if delivery {
                // (its DIFFICULTY line is white with the arrows alone in the cursor's colour -
                // `dl_diff_0..3`; the series' line is all in that colour. The rule that tells
                // them apart was not found in the movie: the text is tinted here.)
                for col in ["slotsA", "slotsB"] {
                    if let Some(id) = fm.movie.player.find(&format!("screen.menu.{}.slot0.slot", col)) {
                        if let Some(inst) = fm.movie.player.inst_mut(id) {
                            inst.tint = [0xe6, 0xe6, 0xe6, inst.tint[3]];
                        }
                    }
                }
            }
            match (crate::menu3d::read_scene(&mut fm.movie.player), self.ui_meshes.as_mut()) {
                (Some(scene), Some(meshes)) => {
                    if delivery {
                        self.pause_cam = Some((scene.cam.position, scene.cam.lookat, scene.cam.fovy));
                    }
                    crate::menu3d::paint(ctx, &lp, meshes, &mut fm.movie, &scene, None)
                }
                _ => fm.movie.paint(ctx, &lp),
            }
        }
        let leave = |this: &mut Self| {
            this.hangout_key = None;
            this.hangout_page = (0, 0);
            if let Some(fm) = this.flash_menu.as_mut() {
                fm.hide();
            }
        };
        if gas {
            // (test hook: MCLA_GAS_ACCEPT=0|1 presses ACCEPT on that row)
            let hook = std::env::var("MCLA_GAS_ACCEPT").ok().and_then(|v| v.parse::<usize>().ok());
            if let Some(r) = hook {
                self.hangout_page.1 = r.min(1);
            }
            if enter || hook.is_some() {
                // (REFILL NITRO without nitrous: UI_MENU_ERROR, the menu stays)
                if self.hangout_page.1 == 1 && !self.opt_x.s23b.gas_nitro {
                    self.opt_x.s23b.ui_obj = Some("UI_MENU_ERROR");
                    return None;
                }
                let fix = self.hangout_page.1 == 0;
                leave(self);
                return Some(MenuAction::GasStation(fix));
            }
            if back {
                leave(self);
                return Some(MenuAction::Resume);
            }
            return None;
        }
        // (test hook: MCLA_TT_ACCEPT presses ACCEPT on the time trial panel)
        if (enter || std::env::var("MCLA_TT_ACCEPT").is_ok()) && s19h_tt.is_some() {
            let row = self.hangout_page.1;
            if self.mc4_tt_panel_accept(&key, row) {
                log::info!("hangout: {} accepted, row {}", key, row);
                leave(self);
                return Some(MenuAction::StartRace);
            }
        // (test hook: MCLA_EVENT_ACCEPT presses ACCEPT on a tournament's or the payback's panel)
        } else if (enter || std::env::var("MCLA_EVENT_ACCEPT").is_ok()) && s15b_event {
            if self.mc4_event_panel_accept(&key) {
                leave(self);
                return Some(MenuAction::StartRace);
            }
        } else if enter && delivery {
            // the next delivery of the district's list, at the difficulty chosen
            let list: Vec<usize> = (0..self.race_defs.len()).filter(|&i| self.race_defs[i].race_type == "Delivery" && (self.race_defs[i].district == "SouthCentral") == key.starts_with("sc_")).collect();
            let n = self.career.stats.get("delivery_at").copied().unwrap_or(0).max(0) as usize;
            if let Some(&i) = list.get(n % list.len().max(1)) {
                self.career.stats.insert("delivery_at".into(), (n + 1) as i64);
                self.current_race = Some(i);
                self.event_diff = Some(self.hangout_diff);
                leave(self);
                return Some(MenuAction::StartRace);
            }
        } else if enter && !wager {
            // the series' next race, at the difficulty chosen
            let races = self.career.event_races(&format!("{}_series", key));
            let won = self.career.done_races();
            let pick = races.iter().find(|r| !won.iter().any(|w| w.eq_ignore_ascii_case(r))).or(races.first()).cloned();
            let at = pick.and_then(|r| { let r = r.to_ascii_lowercase(); self.race_defs.iter().position(|d| { let id = d.id.to_ascii_lowercase(); id.ends_with(&r) || r.ends_with(&id) }) });
            log::info!("hangout: {} accepted at difficulty {}: {} races, race {:?}", key, self.hangout_diff, races.len(), at);
            // (a series is run from its first race, the tally at nought)
            let order: Vec<usize> = races.iter().filter_map(|r| { let r = r.to_ascii_lowercase(); self.race_defs.iter().position(|d| { let id = d.id.to_ascii_lowercase(); id.ends_with(&r) || r.ends_with(&id) }) }).collect();
            // (the tally starts at nought each time, but the series goes on with the race after
            // the last one started: the recomp ran race 1, began race 2, and on the next visit
            // opened with race 3 - `sr_results_race3`)
            let first = self.career.stats.get(&format!("series_at:{}", key)).copied().unwrap_or(0).max(0) as usize % order.len().max(1);
            let at = if order.is_empty() { at } else { Some(order[first]) };
            if let Some(i) = at {
                self.current_race = Some(i);
                self.event_diff = Some(self.hangout_diff);
                if !order.is_empty() {
                    self.career.stats.insert(format!("series_at:{}", key), ((first + 1) % order.len()) as i64);
                }
                // (test hook: MCLA_SERIES_WINS=n starts the series with the player at n wins)
                let wins = std::env::var("MCLA_SERIES_WINS").ok().and_then(|v| v.parse::<u32>().ok()).map(|n| vec![("YOU".to_string(), n)]).unwrap_or_default();
                self.series = (!order.is_empty()).then(|| Series { key: format!("{}_series", key), races: order, at: first, wins, rivals: Vec::new(), diff: self.hangout_diff });
                leave(self);
                return Some(MenuAction::StartRace);
            }
        } else if (enter || std::env::var("MCLA_PINK_ACCEPT").is_ok()) && wager && page_no == 2 {
            // (the race for pink slips: the stake into the lot, the hangout's race - step 21)
            let pick = self.opt_x.arcade.pink.pick;
            // (step 22b: the original's confirmation box first - its YES stakes the vehicle and
            // starts the race, `mc4_pink_confirmed`; the panel's own start below no longer runs)
            if self.mc4_pink_ask(&key, pick) && self.mc4_pink_start(&key, pick) {
                self.current_race = self.mc4_pink_race(&key).or(Some(self.race_sel.min(self.race_defs.len().saturating_sub(1))));
                self.wager_here = false;
                log::info!("hangout: {} pink slip race", key);
                leave(self);
                return Some(MenuAction::StartRace);
            }
        } else if enter && page_no == 0 && self.hangout_page.1 == 1 && pink_open {
            self.hangout_page = (2, 0);
            self.opt_x.arcade.pink.pick = 0;
        } else if enter && page_no == 0 {
            if self.hangout_page.1 == 0 {
                // (WAGER AMOUNT opens on MEDIUM, the middle of the slider)
                self.hangout_page = (1, 0);
                self.hangout_diff = 1;
                self.hangout_stake = 5;
            }
        } else if enter {
            let (stake, _, _) = stake_of(self, self.hangout_diff, self.hangout_stake);
            if self.money < stake {
                let m = st("Dialog_Description_WagerInsufficientFunds", "You don't have enough money to wager.").to_uppercase();
                self.show_message(&m);
            } else {
                // (a race of the hangout's: the nearest, as `race_sel` was put on pulling in)
                self.money -= stake;
                self.dirty = true;
                self.wager = Some((stake, self.hangout_diff));
                self.current_race = Some(self.race_sel.min(self.race_defs.len().saturating_sub(1)));
                self.wager_here = false;
                log::info!("hangout: {} wager ${} at difficulty {}", key, stake, self.hangout_diff);
                leave(self);
                return Some(MenuAction::StartRace);
            }
        }
        if back {
            if wager && page_no >= 1 {
                self.hangout_page = (0, if page_no == 2 { 1 } else { 0 });
            } else {
                leave(self);
                self.wager_here = false;
                return Some(MenuAction::Back);
            }
        }
        None
    }

    /// A starter's own paint, from its set-up file (`resources/vehicle/configs/starter_<car>.xcc`:
    /// the six paint entries from 0x660 - body, doors, hood, front bumper, rear bumper,
    /// sideskirts - colours as the screen shows them, the type at +0x20): the 240SX white
    /// with a grey hood, the Scirocco olive with primer doors, the Golf red with a grey door.
    pub fn set_starter_paint(&mut self, car: &str) {
        let Ok(d) = std::fs::read(std::path::Path::new(&self.data_dir).join("resources/vehicle/configs").join(format!("starter_{}.xcc", car.to_ascii_lowercase()))) else { return };
        let entry = |n: usize| -> Option<([f32; 3], u32)> {
            let e = 0x660 + 0x30 * n;
            let f = |o: usize| d.get(o..o + 4).map(|b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]));
            let c = [f(e)?, f(e + 4)?, f(e + 8)?];
            let kind = d.get(e + 0x20..e + 0x24).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))?;
            c.iter().all(|v| (0.0..=1.0).contains(v)).then_some((c, kind))
        };
        let Some((body, kind)) = entry(0) else { return };
        self.paint_hsv = Some(paint_rgb_to_hsv([body[0].powf(2.2), body[1].powf(2.2), body[2].powf(2.2), 1.0]));
        self.paint_type = if (1..=5).contains(&kind) { kind as u8 } else { 2 };
        self.paint_fade = 0;
        self.paint_for = car.to_ascii_lowercase();
        let mut cfg = CarConfig::default();
        for (n, key) in ["doors", "hood", "bumper_f", "bumper_r", "skirts"].iter().enumerate() {
            if let Some((c, _)) = entry(n + 1) {
                if (0..3).any(|i| (c[i] - body[i]).abs() > 0.02) {
                    cfg.part_paint.insert(key.to_string(), [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8]);
                }
            }
        }
        self.cfg = cfg;
        // (and its record is the starter set-up's: `mc4`)
        self.mc4_car_added(car, Some(&format!("starter_{}", car.to_ascii_lowercase())));
        self.mc4_take_car(car);
    }

    /// VEHICLE SELECT, a new career's first screen (references `ng_vsel_0`..`3`): the three
    /// starters one at a time - 1988 SCIROCCO, 1998 240SX, 1983 GOLF/GTI, round - under the
    /// prompt movie's own title (`vehicle_select`) with ACCEPT (A) and the vehicle's name
    /// between arrows at the bottom right. No HUD.
    fn starter_ui(&mut self, ctx: &egui::Context, left: bool, right: bool, enter: bool) {
        let n = STARTERS.len();
        let mut at = match self.starter_pick {
            Some(a) => a,
            None => {
                self.starter_swap = Some(STARTERS[0].to_string());
                0
            }
        };
        if left || right {
            at = if right { (at + 1) % n } else { (at + n - 1) % n };
            self.starter_swap = Some(STARTERS[at].to_string());
        }
        self.starter_pick = Some(at);
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).filter(|v| !v.contains('<')).unwrap_or_else(|| d.to_string());
        let name = st(STARTERS[at], &STARTERS[at].to_uppercase());
        let prompts = vec![(name, std::env::var("MCLA_STARTER_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(crate::flash_ui::ICON_ARROWS)), (st("prompt_accept", "ACCEPT"), crate::flash_ui::ICON_A)];
        let title = st("PROMPT_VEHICLE_SELECT", "VEHICLE SELECT");
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if let Some(fp) = self.flash_prompt.as_mut() {
            fp.movie.player.set_global("vehicle_select", mcla_flash::value::Value::Int(1));
            fp.movie.player.set_global("t_vehicle_select", mcla_flash::value::Value::str(&title));
            fp.show(&prompts, None, None);
            fp.movie.player.update(dt);
            // (the arrow that leads off the list's end is dim - `ng_vsel_0`, `ng_vsel_2`)
            for (clip, dim) in [("arrow_left", at == 0), ("arrow_right", at + 1 == n), ("arrow_rignt", at + 1 == n)] {
                if let Some(id) = fp.movie.player.find(&format!("screen.allprompts.prompt1.icon.{}", clip)) {
                    if let Some(inst) = fp.movie.player.inst_mut(id) {
                        inst.tint[3] = if dim { 60 } else { 255 };
                    }
                }
            }
            if std::env::var("MCLA_STARTER_LOG").is_ok() {
                log::info!("starter arrows: {:?} {:?}", fp.movie.player.find("screen.allprompts.prompt1.icon.arrow_left"), fp.movie.player.find("screen.allprompts.prompt1.icon.arrow_right"));
            }
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("starter"));
            fp.movie.paint(ctx, &ctx.layer_painter(layer));
        }
        if enter {
            self.starter_pick = None;
            self.fresh_game = false;
            // (a new career's first tutorial, with the first call - `ng_c4`)
            self.tutorial_box = Some(("tutorial_sidekick_title".into(), "tutorial_sidekick_desc".into(), 3.0, 12.0));
            self.career.stats.insert("tut:sidekick".into(), 1);
            self.dirty = true;
            if let Some(fp) = self.flash_prompt.as_mut() {
                fp.movie.player.set_global("vehicle_select", mcla_flash::value::Value::Int(0));
                fp.hide();
            }
        }
    }

    /// The GPS map is there once the career's first mission is behind the player (a new
    /// career: no map on Back, no GPS on the pause bar - `ng_c5`; after it, `ng_gps_1`).
    /// The MINI MAP lesson holds the game until it is answered (`ng_tut_minimap`).
    pub fn tutorial_hold(&self) -> bool {
        self.tutorial_box.as_ref().map_or(false, |t| t.0.contains("target") && t.2 <= 0.0)
    }

    pub fn gps_unlocked(&self) -> bool {
        // (the original's `enableGps`: the save's switch and the three events that set it - `mc4`)
        !self.flash_hud.is_some() || self.mc4_gps_enabled()
    }

    /// The badge's ring is the event's difficulty colour (green on the first career race,
    /// `ng_chase_027`; yellow on an ARCADE race at MEDIUM, `y_replay_intro`) - the colours
    /// of the map's event dots.
    fn badge_ring(&self) -> u32 {
        let d = self.event_diff.or(self.wager.map(|w| w.1)).unwrap_or(self.mc4_arcade_difficulty().unwrap_or(self.race_diff)).min(3);
        let c = crate::rewards::DIFF_COLORS[d];
        (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
    }

    /// The race's badge (`MenuLayout::StartOfRace`) for the race in hand: its name, its kind,
    /// its district's emblem.
    fn start_badge_page(&self) -> Option<crate::flash_ui::MenuPage> {
        let d = self.current_race.and_then(|i| self.race_defs.get(i))?;
        let name = self.career.strings.get(&d.id).filter(|v| !v.contains('<')).map(|v| v.to_uppercase()).unwrap_or_else(|| d.name.to_uppercase().replace('_', " "));
        let key = match d.sub_type.to_ascii_lowercase().as_str() {
            "ordered" => "cp_ordered",
            "unordered" => "cp_unordered",
            "circuit" => "Circuit",
            "blitz" => "Blitz",
            "delivery" => "Delivery",
            _ => "",
        };
        let kind = self.career.strings.get(key).map(|v| if key == "Delivery" { v.to_string() } else { v.to_uppercase() }).unwrap_or_else(|| format!("{} RACE", d.sub_type.to_uppercase()));
        // (GOAL ATTACK: "TARGET TIME: mm:ss.hh" in place of the kind - `mc4_goal_badge`)
        let kind = self.mc4_goal_badge().map(|t| t.to_uppercase()).unwrap_or(kind);
        let dl = format!("{} {}", d.district, d.id).to_ascii_lowercase();
        let icon = if dl.contains("south") || dl.contains("sc_") { 4 } else if dl.contains("downtown") || dl.contains("dt_") { 6 } else if dl.contains("beach") || dl.contains("monica") || dl.contains("sm_") { 1 } else if dl.contains("hills") || dl.contains("bh_") { 2 } else { START_BADGE_ICON };
        Some(crate::flash_ui::MenuPage { layout: crate::flash_ui::MenuLayout::StartOfRace { description: kind, ring: self.badge_ring() }, icon, title: name, select: usize::MAX, ..Default::default() })
    }

    /// VIEW REPLAY: the race just run from its grid (`replay.rs`).
    fn start_replay(&mut self) {
        if std::env::var("MCLA_REPLAY_TRACE").is_ok() {
            log::info!("start_replay: mode {:?} menu_sel {} pause_do {:?} first {} reward {} table {}", self.mode as u8, self.menu_sel, self.pause_do, self.series_first, self.reward_panel, self.results_table);
        }
        if self.replay_rec.len() < 1.0 {
            return;
        }
        let car = self.dir_name(self.current_car).to_ascii_lowercase();
        let tune = crate::replay::CamTune::load(&self.data_dir, &car);
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        self.replay = Some(crate::replay::Playback::new(tune, self.rng));
        self.reward_panel = false;
        self.results_table = false;
    }

    /// The replay as the game shows it (references `y_replay`, `y_replay_paused`,
    /// `y_replay_hidehud`, `y_replay_menu`): the prompt movie's legend at the top right -
    /// HIDE LEGEND (Back), FRAME ADVANCE (B), PAUSE / RESUME (A), MENU (Start), NEXT CAMERA
    /// (Y), DISABLE / ENABLE CAMERA SWITCHING (X) and, paused, SLOW (left stick) - with the
    /// playhead along the bottom; Back leaves the one line HIDE HUD, once more nothing;
    /// Start opens a PAUSE panel: CONTINUE REPLAY, EXIT REPLAY, RESTART REPLAY, SAVE GAME.
    fn replay_ui(&mut self, ctx: &egui::Context, up: bool, down: bool, left: bool, right: bool, enter: bool, back: bool) {
        use crate::flash_ui::{ICON_A, ICON_B, ICON_BACK_BUTTON, ICON_START, ICON_X, ICON_Y};
        // (the replay runs in real time: the frame's own length, not egui's predicted one)
        let dt = ctx.input(|i| i.unstable_dt).min(0.1);
        let (x, y, select, start) = (std::mem::take(&mut self.nav_x), std::mem::take(&mut self.nav_y), std::mem::take(&mut self.nav_select), std::mem::take(&mut self.nav_start));
        let slow = self.nav_slow + if right { 4.0 } else if left { -4.0 } else { 0.0 };
        let len = self.replay_rec.len().max(crate::replay::STEP);
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).filter(|v| !v.contains('<')).unwrap_or_else(|| d.to_string());
        let Some(pb) = self.replay.as_mut() else { return };
        if let Some(cur) = pb.menu {
            let rows = [(st("replay_pause_continue_replay", "CONTINUE REPLAY"), 0u8), (st("replay_pause_exit_replay", "EXIT REPLAY"), 1), (st("replay_pause_restart_replay", "RESTART REPLAY"), 2), (st("pause_save_game", "SAVE GAME"), 3)];
            let mut cur = cur;
            if up && cur > 0 {
                cur -= 1;
            }
            if down && cur + 1 < rows.len() {
                cur += 1;
            }
            pb.menu = Some(cur);
            let mut leave = false;
            let mut save = false;
            if enter {
                match cur {
                    0 => pb.menu = None,
                    1 => leave = true,
                    2 => pb.restart(),
                    _ => save = true,
                }
            } else if back || start {
                pb.menu = None;
            }
            if leave {
                self.replay = None;
                self.replay_rec.restore = true;
                self.menu_sel = 0;
                return;
            }
            if save {
                let d = self.data_dir.clone();
                self.write_save(&d);
            }
            if self.replay.as_ref().map_or(false, |p| p.menu.is_some()) {
                self.results_movie(ctx, &rows, cur);
            }
            return;
        }
        // (Start is also the menus' "back": it opens the panel, B alone steps a frame)
        if start {
            pb.menu = Some(0);
            self.nav_back = false;
            return;
        }
        // (the opening - the race's start played again - takes no buttons but MENU)
        let opening = pb.opening != 0;
        let (enter, back, y, x, select, slow) = if opening { (false, false, false, false, false, 0.0) } else { (enter, back, y, x, select, slow) };
        if enter {
            pb.paused = !pb.paused;
        }
        if back {
            pb.paused = true;
            pb.t += crate::replay::STEP;
        }
        if y {
            pb.next_view();
        }
        if x {
            pb.auto_cam = !pb.auto_cam;
        }
        if select {
            pb.legend = (pb.legend + 1) % 3;
        }
        if opening {
        } else if pb.paused {
            pb.t += slow * 0.25 * dt;
        } else {
            pb.t += dt;
        }
        pb.t = pb.t.clamp(0.0, len);
        if pb.t >= len && !pb.paused && !opening {
            // the end of the recording: the original leaves the replay by itself - the race's
            // end again (its camera and the place in its ring, `cap2/end_*`), then the race-over
            // menu on its first row (mcReplayForm sub_82641F10 at the last frame)
            self.replay = None;
            self.replay_rec.restore = true;
            self.menu_sel = 0;
            self.race_end_t = Some(0.0);
            self.place_splash = Some((self.race_place, Self::RACE_END_MENU));
            if let Some(fp) = self.flash_prompt.as_mut() {
                fp.hide();
            }
            return;
        }
        let (paused, legend, auto, pos) = (pb.paused, pb.legend, pb.auto_cam, pb.t.max(0.0) / len);
        // the race's start again: its 3-2-1 from the HUD movie, nothing else of the HUD (in the
        // opening the start scene's own count, then the recording's)
        let countdown = if opening { self.race.as_ref().map_or(0.0, |r| r.countdown) } else { self.replay_rec.countdown0 - pb.t };
        // (and the start scene's racename event its badge)
        let badge = if opening && self.cine.as_ref().map_or(false, |c| !c.is_empty()) { self.start_badge_page() } else { None };
        match (badge, self.flash_popup.as_mut()) {
            (Some(page), Some(fp)) => {
                fp.show(&page);
                fp.movie.player.update(dt);
                fp.movie.paint(ctx, &ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("cine"))));
                self.start_badge_on = true;
            }
            (None, Some(fp)) if self.start_badge_on => {
                fp.hide();
                self.start_badge_on = false;
            }
            _ => {}
        }
        if countdown > -2.0 {
            if let Some(fh) = self.flash_hud.as_mut() {
                // (before the count the movie runs on even when paused, so that the HUD it hides goes)
                fh.drive_replay_intro(countdown, if paused && countdown <= 3.0 { 0.0 } else { dt });
                let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_hud"));
                fh.movie.paint(ctx, &ctx.layer_painter(layer));
            }
        }
        if let Some(fm) = self.flash_menu.as_mut() {
            fm.hide();
        }
        self.pause_cam = None;
        let ls = std::env::var("MCLA_REPLAY_LS").ok().and_then(|v| v.parse().ok()).unwrap_or(REPLAY_ICON_LS);
        let prompts: Vec<(String, i32)> = match legend {
            0 => {
                let mut v = vec![
                    (st("replay_prompt_hideLegend", "HIDE LEGEND"), ICON_BACK_BUTTON),
                    (st("replay_prompt_step", "FRAME ADVANCE"), ICON_B),
                    (if paused { st("replay_prompt_resume", "RESUME") } else { st("replay_prompt_pause", "PAUSE") }, ICON_A),
                    (st("replay_prompt_menu", "MENU"), ICON_START),
                    (st("replay_prompt_next_camera", "NEXT CAMERA"), ICON_Y),
                    (if auto { st("replay_prompt_disable_camera_cycle", "DISABLE CAMERA SWITCHING") } else { st("replay_prompt_enable_camera_cycle", "ENABLE CAMERA SWITCHING") }, ICON_X),
                ];
                if paused {
                    v.push((st("replay_prompt_slow", "SLOW"), ls));
                }
                v
            }
            1 => vec![(st("replay_prompt_hideHud", "HIDE HUD"), ICON_BACK_BUTTON)],
            _ => Vec::new(),
        };
        // (no legend or bar over the race's opening and its count: `vr_1`, `vr_2`)
        if legend < 2 && countdown <= 0.0 {
            if let Some(fp) = self.flash_prompt.as_mut() {
                let icon = std::env::var("MCLA_REPLAY_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(if paused { 2 } else { 1 });
                fp.show_replay(&prompts, icon, pos);
                fp.movie.player.update(dt);
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("replay")));
                fp.movie.paint(ctx, &painter);
            }
        }
    }

    fn results_movie(&mut self, ctx: &egui::Context, ro: &[(String, u8)], sel: usize) -> bool {
        use crate::flash_ui::{MenuLayout, MenuPage, MenuRow, ICON_A, ICON_B};
        if self.ui_meshes.is_none() {
            return false;
        }
        // (the race's end: nothing over the picture until WAITUNWARPED(200) is over)
        if self.race_end_t.map_or(false, |t| t < Self::RACE_END_PANEL) && self.replay.is_none() {
            return true;
        }
        // (step 19d: form 56's banner, after the place's splash - `mc4_banner_page`)
        let s19d_banner = if self.place_splash.filter(|_| self.replay.is_none()).is_none() { self.mc4_banner_page(ctx.input(|i| i.stable_dt).min(0.1)) } else { None };
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).filter(|v| !v.contains('<')).unwrap_or_else(|| d.to_string());
        let page = if let Some((place, _)) = self.place_splash.filter(|_| self.replay.is_none()) {
            // the place, in its ring, before anything else (reference `ng_finish_2nd`)
            let suffix = match place { 1 => "st", 2 => "nd", 3 => "rd", _ => "th" };
            let look = if place == 0 { 2 } else if self.place_tied { 1 } else { 0 };
            // (the movie's own rule: TIED stands over the place; DNF is the name itself, larger)
            MenuPage { layout: MenuLayout::Position { suffix: if look == 2 { String::new() } else { st(suffix, &suffix.to_uppercase()) }, look }, title: if look == 2 { st("RACE_RESULT_DNF", "DNF") } else { place.to_string() }, select: usize::MAX, ..MenuPage::default() }
        } else if let Some(page) = s19d_banner {
            // (step 19d: a time trial's event results banner - form 56 - before its panel)
            page
        } else if let Some(cur) = self.replay.as_ref().and_then(|p| p.menu) {
            // the replay's own PAUSE panel (reference `y_replay_menu`)
            MenuPage {
                layout: MenuLayout::List,
                icon: 1,
                title: st("pause_tab_mainmenu", "PAUSE"),
                rows: ro.iter().map(|(l, _)| MenuRow { label: l.clone(), value: String::new(), enabled: true, part: None }).collect(),
                select: cur,
                prompts: vec![(st("prompt_accept", "ACCEPT"), ICON_A), (st("prompt_back", "BACK"), ICON_B), (st("prompt_gps", "GPS"), crate::flash_ui::ICON_BACK_BUTTON)],
                list_max: 6,
                ..MenuPage::default()
            }
        } else if let Some(cur) = self.series_ask {
            MenuPage {
                layout: MenuLayout::Confirm { description: self.career.strings.get(["Dialog_Description_NextSeries", "Dlg_Desc_Series_Quit", "Dlg_Desc_Series_Restart", "Dialog_Description_RaceBack", "Dialog_Description_ReturnToCruise", "Dlg_RaceAgainSeries_Desc"][self.series_ask_kind.min(5) as usize]).map(|v| v.to_string()).unwrap_or_else(|| "Do you want to repair your vehicle before you CONTINUE the series?".into()) },
                icon: std::env::var("MCLA_RES_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(13),
                title: [st("Dialog_Title_NextSeries", "CONTINUE SERIES"), st("Dlg_Title_Series_Quit", "QUIT SERIES"), st("Dlg_Title_Series_restart", "RESTART SERIES"), st("Dialog_Title_RaceBack", "RACE BACK"), st("Dialog_Title_ReturnToCruise", "RETURN TO CRUISE"), st("Dlg_RaceAgainSeries_Title", "RACE AGAIN")][self.series_ask_kind.min(5) as usize].clone(),
                rows: vec![MenuRow { label: st("Common_Yes", "YES"), value: String::new(), enabled: true, part: None }, MenuRow { label: st("Common_No", "NO"), value: String::new(), enabled: true, part: None }],
                select: cur,
                prompts: vec![(st("prompt_accept", "ACCEPT"), ICON_A), (st("prompt_back", "BACK"), ICON_B)],
                list_max: 6,
                ..MenuPage::default()
            }
        } else if self.reward_panel && !self.series_first && !self.mc4_tt_panel_first() {
            // REWARDS (reference `y_rewards`): REP EARNED: 30 / RANK: IDOL / TOTAL REP: 119110 /
            // NEXT RANK AT: N/A and the rank's meter; money earned as a second row.
            // The thirteen ranks are the thirteen levels of the reputation curve (`rewards`):
            // the first step comes out at 800, as captured (`ng_rewards`: NEXT RANK AT: 800),
            // and the 100 % save's 119,110 is the last, IDOL, with N/A (`y_rewards`).
            const RANKS: [&str; 13] = ["Rank_1_Backseat_Driver", "Rank_2_Navigator", "Rank_3_Student_Driver", "Rank_4_Rookie", "Rank_5_Driver", "Rank_6_Racer", "Rank_7_Veteran", "Rank_8_Elite_Racer", "Rank_8_5_Champion", "Rank_9_Legend", "Rank_10_Savant", "Rank_10_5_Hero", "Rank_11_Idol"];
            let rank = crate::rewards::progress(self.rep).0.min(12);
            let step = |l: usize| crate::rewards::rep_curve(crate::rewards::LEVEL_FRAC[l.min(12)]);
            let next = if rank >= 12 { "N/A".to_string() } else { format!("{:.0}", step(rank + 1)) };
            let meter = if rank >= 12 { 100.0 } else { ((self.rep as f32 - step(rank)) / (step(rank + 1) - step(rank)).max(1.0) * 100.0).clamp(0.0, 100.0) };
            let commas = |v: i64| { let s = v.to_string(); let mut o = String::new(); for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { o.push(','); } o.push(c); } o };
            let (money, rep) = self.last_reward;
            let mut rows = Vec::new();
            let mut props = Vec::new();
            if rep > 0 {
                rows.push(MenuRow { label: format!("REP EARNED:  {}", rep), value: format!("{} {}", st("G_Rank", "RANK:"), st(RANKS[rank], "RANK").to_uppercase()), enabled: true, part: None });
                props.push(11);
            }
            if money > 0 {
                // (MONEY EARNED: $100 / TOTAL: $5,100 - `ng_rewards`)
                rows.push(MenuRow { label: format!("MONEY EARNED:  ${}", commas(money)), value: format!("{} ", st("EOR_money_total_format", "TOTAL:").trim()), enabled: true, part: None });
                props.push(12);
            }
            // (the part types a class threshold has just unlocked: the form's UNLOCKED group - a
            // header row, kind 13, with the count, and a row of kind 19 for each, eight at
            // the most: sub_82676B08 0x82677414..0x826775F4)
            // (they are on the pages after this one, the REWARDED group - header kind 17 -
            // before them: `mc4_reward_page`, `mc4::reward_pages`)
            if let Some(page) = self.mc4_reward_page(!rows.is_empty()) {
                rows.clear();
                props.clear();
                for (kind, label, value) in page {
                    rows.push(MenuRow { label, value, enabled: true, part: None });
                    props.push(kind);
                }
            }
            // (GOAL ATTACK: after the money an empty row of kind 12, then the form's item
            // `Goal Attack` of kind 15 - its four goal icons from `text2`: sub_82676B08
            // 0x82676F68, navsys FillReward 0x16d0a. Where the game fills the item's texts was
            // not found: the title and this race's goals are taken - approx.)
            if self.goal_attack && self.career.reward_page == 0 {
                rows.push(MenuRow { label: String::new(), value: String::new(), enabled: true, part: None });
                props.push(12);
                let mask = self.current_race.map_or(0, |i| self.mc4_goal_mask(i));
                rows.push(MenuRow { label: st("Goal Attack", "GOAL ATTACK").to_uppercase(), value: mask.to_string(), enabled: true, part: None });
                props.push(15);
            }
            MenuPage {
                layout: MenuLayout::Reward { total_rep: self.rep, next_rank: next, rep_label: format!("{}  ", st("Pause_Rep", "TOTAL REP:").trim()), next_label: format!("{}:  ", st("Next_Rank", "NEXT RANK AT").trim().trim_end_matches(':')), meter, money: self.money },
                icon: std::env::var("MCLA_RES_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(13),
                title: st("EOR_rewards_title", "REWARDS"),
                rows,
                row_props: props,
                select: usize::MAX,
                prompts: vec![(st("prompt_continue", "CONTINUE"), ICON_A)],
                list_max: 6,
                ..MenuPage::default()
            }
        } else if let Some((title, objectives, done)) = self.mission_results.first().cloned() {
            // (the movie's rule: the header's `text4` 1 stamps COMPLETE, anything else FAIL;
            // an objective's `select` ticks it)
            let mark = if done { "1" } else { "0" };
            // MISSION RESULTS (reference `ng_mission_results`; the rewards' template): the
            // mission's title with its COMPLETE stamp, its objectives ticked; CONTINUE (A)
            // (a time trial's panel is its own form - 42, TIME TRIAL RESULTS: the header TIME
            // of kind 25 with the stamp, TARGET TIME and YOUR TIME as rows of kind 19 -
            // `mc4::TT_PANEL`, sub_82678B90)
            let s19a_tt = title.starts_with(crate::mc4::TT_PANEL);
            let title = title.trim_start_matches(crate::mc4::TT_PANEL).to_string();
            let mut rows = vec![MenuRow { label: title.to_uppercase(), value: mark.to_string(), enabled: true, part: None }];
            let mut props = vec![if s19a_tt { 25 } else { 18 }];
            for o in objectives {
                rows.push(MenuRow { label: o.to_uppercase(), value: if s19a_tt { String::new() } else { mark.to_string() }, enabled: true, part: None });
                props.push(if s19a_tt { 19 } else { 6 });
            }
            MenuPage {
                layout: MenuLayout::Reward { total_rep: self.rep, next_rank: String::new(), rep_label: String::new(), next_label: String::new(), meter: 0.0, money: 0 },
                // (step 19d: form 42's own `iconID` is the form base's 0 - its constructor
                // sub_82677C40 and update sub_82678B90 never call vfunc 648)
                icon: std::env::var("MCLA_RES_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(if s19a_tt { 0 } else { 15 }),
                title: if s19a_tt { st("TIMETRIAL_RESULTS_TITLE", "TIME TRIAL RESULTS") } else { st("EOR_missionResultsTitle", "MISSION RESULTS") },
                rows,
                row_props: props,
                select: usize::MAX,
                prompts: vec![(st("prompt_continue", "CONTINUE"), ICON_A)],
                list_max: 6,
                ..MenuPage::default()
            }
        } else if self.results_table {
            let plain = self.results_plain;
            // (NAME and TIME, the times MM:SS.cc, the player's line the last column's colour -
            // reference `y_results`)
            let cells: Vec<Vec<String>> = self
                .last_results
                .iter()
                .map(|(name, t)| vec![if name.starts_with("YOU") { st("G_User", "USER") } else { name.split(" (").next().unwrap_or(name).to_uppercase() }, if plain { t.map(|v| format!("{:.0}", v)).unwrap_or_default() } else { t.map(|v| stat_time((v * 1000.0) as i64)).unwrap_or_else(|| st("RACE_RESULT_DNF", "DNF")) }])
                .collect();
            // (a series: POS., NAME, TIME and WINS - "1 / 3" -, titled RACE RESULTS with the
            // series' flags: `sr_results`)
            let series = self.series.clone();
            let cells: Vec<Vec<String>> = match series.as_ref() {
                Some(sr) => cells
                    .into_iter()
                    .zip(self.last_results.iter())
                    .enumerate()
                    .map(|(i, (c, r))| {
                        let who = if r.0.starts_with("YOU") { "YOU" } else { r.0.as_str() };
                        let w = sr.wins.iter().find(|w| w.0 == who).map_or(0, |w| w.1);
                        vec![(i + 1).to_string(), c[0].clone(), c[1].clone(), format!("{} / {}", w, SERIES_WINS)]
                    })
                    .collect(),
                None => cells,
            };
            let headers = if series.is_some() { vec![st("Leaderboard_Pos", "POS."), st("Leaderboard_Name", "NAME"), st("Leaderboard_Time", "TIME"), st("Leaderboard_Wins", "WINS")] } else { vec![st("Leaderboard_Name", "NAME"), st("Leaderboard_Time", "TIME")] };
            MenuPage {
                layout: MenuLayout::Results { headers, cells: cells.clone() },
                icon: std::env::var("MCLA_RES_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(if series.is_some() { HANGOUT_ICON } else { 13 }),
                title: if series.is_some() { st("pause_race_results", "RACE RESULTS") } else { st("resultsTitle", "RESULTS") },
                // (the rows' own first text is the name; the player's row is the list's chosen one)
                rows: cells.iter().map(|c| MenuRow { label: c[0].clone(), value: String::new(), enabled: true, part: None }).collect(),
                // (no cursor: the player's own line is written in the title's yellow, below)
                select: usize::MAX,
                prompts: if self.series_first { vec![(st("prompt_continue", "CONTINUE"), ICON_A)] } else { vec![(st("prompt_back", "BACK"), ICON_B)] },
                list_max: 8,
                ..MenuPage::default()
            }
        } else {
            MenuPage {
                layout: MenuLayout::List,
                icon: 1,
                tabs: vec![st("pause_tab_mainmenu", "PAUSE"), st("pause_tab_gamemodes", "MODES"), st("pause_tab_settings", "SETTINGS")],
                cur_tab: 0,
                title: st("pause_tab_mainmenu", "PAUSE"),
                rows: ro.iter().map(|(l, _)| MenuRow { label: l.clone(), value: String::new(), enabled: true, part: None }).collect(),
                select: sel,
                prompts: vec![(st("prompt_accept", "ACCEPT"), ICON_A)],
                list_max: 6,
                ..MenuPage::default()
            }
        };
        let user_row = (self.results_table && (!self.reward_panel || self.series_first)).then(|| self.last_results.iter().position(|r| r.0.starts_with("YOU"))).flatten();
        let replay_menu = self.replay.as_ref().map_or(false, |p| p.menu.is_some());
        // (the car rows' pictures: `mc4_reward_pictures`)
        let s19a_pics = if self.reward_panel && !self.series_first && !self.mc4_tt_panel_first() { self.mc4_reward_pictures() } else { Vec::new() };
        let (Some(fm), Some(meshes)) = (self.flash_menu.as_mut(), self.ui_meshes.as_mut()) else { return false };
        fm.show(&page);
        if !s19a_pics.is_empty() {
            let n = fm.movie.swap_textures_named(ctx, "reward_dyntexture", 60, &s19a_pics);
            if std::env::var("MCLA_MENU3D_LOG").is_ok() {
                log::info!("rewards: {} car picture(s) bound of {:?}", n, s19a_pics);
            }
        }
        fm.movie.player.set_global("moviepausemode", mcla_flash::value::Value::Int(3));
        fm.movie.player.set_global("cur_camera", mcla_flash::value::Value::Int(if replay_menu { pause_camera_index(200).max(0).min(9) * 0 + std::env::var("MCLA_PAUSE_CAM").ok().and_then(|v| v.parse().ok()).unwrap_or(REPLAY_PAUSE_CAMERA) } else { 1 }));
        fm.movie.player.set_global("view2d", mcla_flash::value::Value::Int(0));
        fm.movie.player.update(ctx.input(|i| i.stable_dt).min(0.1));
        // (RESULTS: the player's own line in the title's yellow - reference `y_results`; the
        // movie colours it by a rule of the game's that was not read)
        if let Some(row) = user_row {
            for col in ["slotsA", "slotsB", "slotsC", "slotsD"] {
                if let Some(id) = fm.movie.player.find(&format!("screen.menu.{}.slot{}.slot", col, row)) {
                    if let Some(inst) = fm.movie.player.inst_mut(id) {
                        inst.tint = [0xf4, 0xcc, 0x04, inst.tint[3]];
                    }
                }
            }
        }
        let Some(scene) = crate::menu3d::read_scene(&mut fm.movie.player) else { return false };
        self.pause_cam = Some((scene.cam.position, scene.cam.lookat, scene.cam.fovy));
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_menu"));
        ctx.move_to_top(layer);
        crate::menu3d::paint(ctx, &ctx.layer_painter(layer), meshes, &mut fm.movie, &scene, None);
        true
    }
}

/// The paint shop's PAINT AREA choices (references `g_pp_*`, the R8): string key, name, the
/// parts painted (`CarConfig::part_paint` keys), the view. They are the six paint entries
/// of a vehicle's record, in its order. (approx.) FRONT BUMPER's and REAR BUMPER's views
/// are chosen by eye from the view list.
pub const PAINT_AREAS: [(&str, &str, &[&str], &str); 6] = [
    ("P_MainBody", "MAIN BODY", &[], "FrontHighClose"),
    ("P_Doors", "DOORS", &["doors"], "LeftFrontQuarter"),
    ("P_Hood", "HOOD", &["hood"], "FrontHighClose"),
    ("P_FrontBumper", "FRONT BUMPER", &["bumper_f"], "FrontLowClose"),
    ("P_RearBumper", "REAR BUMPER", &["bumper_r"], "RightRearQuarter"),
    ("P_Sideskirts", "SIDESKIRTS", &["skirts"], "LeftFrontQuarter"),
];

/// A hangout's series under way (references `sr_*`): its races in order, the same field
/// every race, a tally of wins - the first to three takes it.
#[derive(Clone, Debug, Default)]
pub struct Series {
    pub key: String,
    pub races: Vec<usize>,
    pub at: usize,
    /// (name as the results have it - "YOU" for the player -, wins)
    pub wins: Vec<(String, u32)>,
    pub rivals: Vec<crate::rivals::Rival>,
    pub diff: usize,
}

impl Game {
    /// The series under way is won or lost for good: a racer has its wins (sub_826CA568's
    /// bComplete / bFailure).
    pub fn series_decided(&self) -> bool {
        self.series.as_ref().map_or(false, |s| s.wins.iter().any(|w| w.1 >= SERIES_WINS))
    }
}

/// wins a series asks for (`RACE WINS NEEDED: 3`)
pub const SERIES_WINS: u32 = 3;

/// (approx.) the delivery panel's title icon: the envelope's frame of the title icons
const DELIVERY_ICON: i32 = 9;

/// a new career's three vehicles, in VEHICLE SELECT's order
pub const STARTERS: [&str; 3] = ["vp_vw_scirocco_88", "vp_nsn_240sx_98", "vp_vw_golf_83"];

/// (approx.) the hangout panel's title icon: a frame of the movie's title icons chosen by
/// its look against the capture
const HANGOUT_ICON: i32 = 8;

/// `hangout_key` of the SERVICE STATION menu (form 61).
pub const GAS_MENU: &str = "gas_station";
/// a wager hangout's: the dollar (frame 6, as captured)
const WAGER_ICON: i32 = 6;

/// (approx.) how long the place stands after a race before REWARDS
const PLACE_SPLASH_SECS: f32 = 3.0;

/// The race badge's ring colour and icon (reference `y_replay_intro`: a star in a yellow
/// ring - Hollywood's). (approx.) the ring's colour is one for all; the emblems of the
/// districts other than Hollywood are assigned by their look, not captured.
const START_BADGE_RING: u32 = 0xC8C814;
const START_BADGE_ICON: i32 = 0;
fn race_badge_shown(on: bool) -> bool {
    on
}

/// (approx.) the prompt movie's icon of the left stick, for the replay's SLOW
const REPLAY_ICON_LS: i32 = 13;
/// the pause camera under the replay's PAUSE panel (reference `y_replay_menu`: the panel
/// to the right, the car from behind its right: `PauseCamera12`, the mirror of the
/// pause menu's own `PauseCamera2`)
const REPLAY_PAUSE_CAMERA: i32 = 11;

/// The ARCADE options as the original opens them (reference `t06`): 3 opponents, MEDIUM,
/// power ups OFF, traffic NORMAL, weather and time of day DYNAMIC.
const ARCADE_DEFAULTS: [usize; 6] = [2, 1, 0, 2, 0, 0];

impl Game {
    fn frame_log_once(&self) -> bool {
        static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed)
    }

    /// The races of an ARCADE type (0 ordered, 1 circuit, 2 unordered, 3 landmark) under
    /// their names in the string table, district by district.
        fn arcade_races(&self, kind: usize) -> Vec<(usize, String)> {
    const TYPES: [&str; 4] = ["Ordered", "Circuit", "Unordered", "Blitz"];
    let kind = kind.min(3);
        let mut v: Vec<(usize, String)> = self
            .race_defs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.sub_type.eq_ignore_ascii_case(TYPES[kind]) && !matches!(d.race_type.as_str(), "Delivery" | "Payback" | "CaptureTheFlag") && !d.dev_only)
            // (an original save's list of unlocked races: `mc4`)
            .filter(|(_, d)| self.mc4_race_unlocked(&d.id))
            .filter_map(|(i, d)| self.career.strings.get(&d.id).map(|n| (i, n.to_uppercase())))
            .filter(|(i, n)| !n.eq_ignore_ascii_case(&self.race_defs[*i].name) && !n.contains('<'))
            .collect();
        // (district by district as the race table has them - Hollywood first - and by
        // name within one: reference `t05`)
        // (in the order of the race table, which the original's list walks - sub_826571D8:
        // Hollywood, Beaches, Downtown, Hills, South Central, and by name within a district)
        if !self.race_table.is_empty() {
            v.sort_by_key(|(i, _)| self.mc4_race_index(&self.race_defs[*i].id).unwrap_or(usize::MAX));
            return v;
        }
        let mut order: Vec<&str> = Vec::new();
        for d in &self.race_defs {
            if !order.contains(&d.district.as_str()) {
                order.push(d.district.as_str());
            }
        }
        let rank = |i: usize| order.iter().position(|x| *x == self.race_defs[i].district).unwrap_or(0);
        v.sort_by(|a, b| rank(a.0).cmp(&rank(b.0)).then(a.1.cmp(&b.1)));
        v
    }

    /// The ARCADE menu of the pause menu's MODES tab (`mcUIArcade`; references `t04`..`t06`):
    /// "Choose race type:", "Choose race:" (the races of that type by their names in the
    /// string table), "Choose race options:". START RACE begins the race chosen.
    fn arcade_ui(&mut self, ctx: &egui::Context, up: bool, down: bool, left: bool, right: bool, enter: bool, back: bool) -> Option<MenuAction> {
        use crate::flash_ui::{choice_list, MenuLayout, MenuPage, MenuRow, ICON_A, ICON_B, ICON_BACK_BUTTON, ICON_X, ICON_Y};
        // (the list of unlocked races up to date: `mc4`)
        self.mc4_races_sync();
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_string()).filter(|v| !v.contains('<')).unwrap_or_else(|| d.to_string());
        let up_st = |k: &str, d: &str| st(k, d).to_uppercase();
        const TYPES: [(&str, &str, &str); 4] = [("Ordered", "Ordered", "ORDERED RACE"), ("Circuit", "Circuit", "CIRCUIT RACE"), ("Unordered", "unordered", "UNORDERED RACE"), ("Blitz", "Blitz", "LANDMARK RACE")];
        // (test hook: MCLA_ARCADE="page,type,cursor" opens that page)
        static HOOKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if let (Ok(v), false) = (std::env::var("MCLA_ARCADE"), HOOKED.swap(true, std::sync::atomic::Ordering::Relaxed)) {
            let f: Vec<usize> = v.split(',').filter_map(|x| x.parse().ok()).collect();
            if f.len() == 3 {
                self.arcade = (f[0].min(2) as u8, f[1].min(3), [f[1].min(3), 0, 0]);
                self.arcade.2[f[0].min(2)] = f[2];
                // (test hook MCLA_GOAL_ATTACK=1: the menu as GOAL ATTACK's)
                if std::env::var("MCLA_GOAL_ATTACK").is_ok() {
                    self.goal_attack = true;
                }
                // (test hook MCLA_ARCADE_POWER=0..2: the POWER UPS row's choice on that page)
                if let Some(v) = std::env::var("MCLA_ARCADE_POWER").ok().and_then(|v| v.parse::<usize>().ok()) {
                    self.arcade_opts[2] = v.min(2);
                }
            }
        }
        if let Some((i, n)) = self.arcade_restore.take() {
            if let Some(d) = self.race_defs.get_mut(i) {
                d.opponents = n;
            }
        }
        let (mut page_no, mut kind, mut cur) = self.arcade;
        let races = self.arcade_races(kind);
        // (a type is offered when a race of it is listed: sub_826598D8 0x82659AA4..0x82659B88,
        // sub_82644C48)
        let shown: Vec<usize> = (0..TYPES.len()).filter(|k| !self.arcade_races(*k).is_empty()).collect();
        if std::env::var("MCLA_MENU3D_LOG").is_ok() && self.frame_log_once() {
            for d in self.race_defs.iter().take(6) {
                log::info!("arcade: race id {:?} name {:?} type {:?} sub {:?} string {:?}", d.id, d.name, d.race_type, d.sub_type, self.career.strings.get(&d.id));
            }
        }
        // (the options page's rows for the race chosen: `mc4_arcade_rows`)
        let orows = self.mc4_arcade_rows(races.get(cur[1]).map(|r| r.0));
        let count = match page_no {
            0 => shown.len().max(1),
            1 => races.len().max(1),
            _ => orows.len().max(1),
        };
        let c = &mut cur[page_no as usize];
        if up {
            *c = (*c + count - 1) % count;
        }
        if down {
            *c = (*c + 1) % count;
        }
        *c = (*c).min(count - 1);
        let mut started: Option<usize> = None;
        let mut out = None;
        let mut nag = false;
        match page_no {
            0 => {
                if enter && !shown.is_empty() {
                    kind = shown[cur[0].min(shown.len() - 1)];
                    cur[1] = 0;
                    page_no = 1;
                } else if back {
                    out = Some(MenuAction::Back);
                    // (the type list's BACK leaves GOAL ATTACK: sub_82654F38 0x82655078)
                    self.goal_attack = false;
                }
            }
            1 if self.goal_attack => {
                // GOAL ATTACK: ACCEPT starts the race at once, no options page (sub_8265D770
                // 0x8265D9F8..0x8265DB10): the options' defaults (sub_826583C0), then TIME OF DAY
                // Midnight, POWER UPS off, WEATHER Clear; START RACE forces TRAFFIC to its low
                // choice (sub_8265D2B8 0x8265D3C4..0x8265D3E4); xNAG + 1
                if enter && !races.is_empty() {
                    let (i, _) = races[cur[1].min(races.len() - 1)];
                    for r in self.mc4_arcade_rows(Some(i)) {
                        crate::mc4::arcade_set(&mut self.arcade_opts, &mut self.arcade_x, &mut self.opt_x, r.slot, r.default);
                    }
                    self.opt_x.arcade.custom_off = [false; 11];
                    for (slot, v) in [(5, 1), (2, 0), (4, 1), (3, 1)] {
                        crate::mc4::arcade_set(&mut self.arcade_opts, &mut self.arcade_x, &mut self.opt_x, slot, v);
                    }
                    nag = true;
                    started = Some(i);
                    self.race_sel = i;
                    self.current_race = Some(i);
                    self.wager = None;
                    out = Some(MenuAction::StartRace);
                } else if back {
                    page_no = 0;
                }
            }
            1 => {
                if enter && !races.is_empty() {
                    page_no = 2;
                    cur[2] = 0;
                    // (the race's own laps; no more opponents than it allows: sub_826583C0)
                    for r in self.mc4_arcade_rows(races.get(cur[1]).map(|r| r.0)) {
                        if r.slot == 6 {
                            self.arcade_x[0] = r.default;
                        }
                        if r.slot == 0 && self.arcade_opts[0] >= r.choices.len() {
                            self.arcade_opts[0] = r.default;
                        }
                    }
                } else if back {
                    page_no = 0;
                }
            }
            _ => {
                let k = cur[2];
                if let (true, Some(r)) = (left || right, orows.get(k)) {
                    let n = r.choices.len().max(1);
                    let v = (self.mc4_arcade_value(r.slot).min(n - 1) + if right { 1 } else { n - 1 }) % n;
                    crate::mc4::arcade_set(&mut self.arcade_opts, &mut self.arcade_x, &mut self.opt_x, r.slot, v);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::X)) {
                    for r in &orows {
                        crate::mc4::arcade_set(&mut self.arcade_opts, &mut self.arcade_x, &mut self.opt_x, r.slot, r.default);
                    }
                    // (DEFAULT puts every widget back, the Custom rows' too whether they are
                    // listed or not: sub_826583C0)
                    self.opt_x.arcade.custom_off = [false; 11];
                }
                if enter {
                    if let Some((i, _)) = races.get(cur[1]) {
                        // NUMBER OF OPPONENTS: the race is started with that many (the race's
                        // own figure comes back when the menu is next opened)
                        started = Some(*i);
                        self.race_sel = *i;
                        self.current_race = Some(*i);
                        self.wager = None;
                        out = Some(MenuAction::StartRace);
                    }
                } else if back {
                    page_no = 1;
                }
            }
        }
        self.arcade = (page_no, kind, cur);
        let races = self.arcade_races(kind);
        if let Some(i) = started {
            self.arcade_restore = Some((i, self.race_defs[i].opponents));
            self.race_defs[i].opponents = self.arcade_opts[0] + 1;
        }
        let o = self.arcade_opts;
        let (description, rows, prompts): (String, Vec<MenuRow>, Vec<(String, i32)>) = match page_no {
            0 => (
                st("pause_choose_race_type", "Choose race type:"),
                shown.iter().map(|i| TYPES[*i]).map(|(_, k, d)| MenuRow { label: up_st(k, d), value: String::new(), enabled: true, part: None }).collect(),
                vec![(up_st("prompt_accept", "ACCEPT"), ICON_A), (up_st("prompt_back", "BACK"), ICON_B), (up_st("prompt_information", "INFORMATION"), ICON_BACK_BUTTON)],
            ),
            1 => (
                st("pause_choose_race", "Choose race:"),
                races.iter().map(|(_, n)| MenuRow { label: n.clone(), value: String::new(), enabled: true, part: None }).collect(),
                // (GOAL ATTACK off line: prompt_accept / Common_Back / Navmap_Prompt_ViewRace -
                // 0x82906F9C)
                if self.goal_attack {
                    vec![(up_st("prompt_accept", "ACCEPT"), ICON_A), (up_st("Common_Back", "BACK"), ICON_B), (up_st("Navmap_Prompt_ViewRace", "VIEW RACE"), ICON_Y)]
                } else {
                    vec![(up_st("prompt_accept", "ACCEPT"), ICON_A), (up_st("prompt_back", "BACK"), ICON_B), (up_st("Navmap_Prompt_ViewRace", "VIEW RACE"), ICON_Y)]
                },
            ),
            _ => {
                // (the rows and their lists are the original's: `mc4_arcade_rows`)
                let orows = self.mc4_arcade_rows(races.get(self.arcade.2[1]).map(|r| r.0));
                let _ = o;
                (
                    st("arcade_options_desc", "Choose race options:"),
                    orows.iter().map(|r| MenuRow { label: r.label.clone(), value: choice_list(self.mc4_arcade_value(r.slot).min(r.choices.len() - 1), &r.choices), enabled: true, part: None }).collect(),
                    vec![(up_st("net_prompt_lobbyStart", "START RACE"), ICON_A), (up_st("prompt_back", "BACK"), ICON_B), (up_st("default", "DEFAULT"), ICON_X), (up_st("Navmap_Prompt_ViewRace", "VIEW RACE"), ICON_Y)],
                )
            }
        };
        let rows_n = rows.len();
        // (GOAL ATTACK's race list: template 15, each race's goal icons and the panel of the
        // one under the cursor - `mc4_goal_mask`, `mc4_goal_lines`)
        let layout = if self.goal_attack && page_no == 1 {
            MenuLayout::GoalAttack { goals: races.iter().map(|(i, _)| (self.mc4_goal_mask(*i), self.mc4_goal_lines(*i))).collect() }
        } else {
            MenuLayout::Confirm { description }
        };
        let page = MenuPage {
            layout,
            // (the GPS movie's title icons: 1 globe, 4 pad)
            icon: std::env::var("MCLA_ARCADE_ICON").ok().and_then(|v| v.parse().ok()).unwrap_or(4),
            // (the original marks each race with a star in a ring; that icon is not among the
            // row icons this template knows - `property1` 3..6: padlock, crossed box - so the
            // rows go without. MCLA_ROW_ICON tries a value.)
            row_props: match std::env::var("MCLA_ROW_ICON").ok().and_then(|v| v.parse().ok()) { Some(v) if page_no == 1 => vec![v; rows_n], _ => Vec::new() },
            title: if self.goal_attack { up_st("goal_attack_title", "GOAL ATTACK") } else { up_st("pause_arcade", "ARCADE") },
            rows,
            select: self.arcade.2[page_no as usize],
            list_max: 11,
            ..MenuPage::default()
        };
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("flash_navsys"));
        ctx.move_to_top(layer);
        let lp = ctx.layer_painter(layer);
        if let Some(fm) = self.flash_navsys.as_mut() {
            fm.show(&page);
            fm.movie.player.update(dt);
            match (crate::menu3d::read_scene(&mut fm.movie.player), self.ui_meshes.as_mut()) {
                (Some(scene), Some(meshes)) => crate::menu3d::paint(ctx, &lp, meshes, &mut fm.movie, &scene, None),
                _ => fm.movie.paint(ctx, &lp),
            }
        }
        if let Some(fp) = self.flash_prompt.as_mut() {
            fp.show(&prompts, None, None);
            fp.movie.player.update(dt);
            fp.movie.paint(ctx, &lp);
        }
        // (a GOAL ATTACK race started: xNAG + 1 - sub_8265D770 after START RACE)
        if nag {
            self.mc4_stat_add("xNAG", 0, 1.0);
        }
        out
    }
}

/// A colour (RGB 0..1) as hue, saturation, brightness (each 0..1).
fn paint_rgb_to_hsv(c: [f32; 4]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    let h = if d < 1e-4 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0) / 6.0
    } else if mx == c[1] {
        ((c[2] - c[0]) / d + 2.0) / 6.0
    } else {
        ((c[0] - c[1]) / d + 4.0) / 6.0
    };
    [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

/// Hue, saturation, brightness (0..1) as RGB (0..1).
fn paint_hsv_to_rgb(h: [f32; 3]) -> [f32; 3] {
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

/// Where the save lives. The save is kept outside the data folder (decision 16, docs/LAUNCHER.md), so wiping,
/// rebuilding or moving the data folder never loses it: `%LOCALAPPDATA%\RustyMCLA\saves` on Windows,
/// `$XDG_DATA_HOME/RustyMCLA/saves` (or `~/.local/share/...`) elsewhere, or `--save-dir <dir>`.
pub mod savedir {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    static DIR: OnceLock<PathBuf> = OnceLock::new();
    const FILE: &str = "mcla_save.json";

    /// The folder set by [`init`], if any.
    pub fn get() -> Option<&'static Path> {
        DIR.get().map(|p| p.as_path())
    }

    pub fn default_dir() -> Option<PathBuf> {
        let base = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        };
        base.map(|b| b.join("RustyMCLA").join("saves"))
    }

    /// Set the save folder for this run: `--save-dir`, else the default; with neither the save stays beside the data.
    pub fn init(arg: Option<&str>) {
        if let Some(dir) = arg.map(PathBuf::from).or_else(default_dir) {
            let _ = DIR.set(dir);
        }
    }

    /// First start with the save folder: copy the save an older build kept in the data folder (the old file is left alone).
    pub fn migrate_legacy(data_dir: &str) {
        let Some(dir) = get() else { return };
        let (new, old) = (dir.join(FILE), Path::new(data_dir).join(FILE));
        if std::env::var("MCLA_SAVE_FILE").is_ok() || new.exists() || !old.exists() {
            return;
        }
        match std::fs::create_dir_all(dir).and_then(|_| std::fs::copy(&old, &new)) {
            Ok(_) => log::info!("save: copied {} to {}", old.display(), new.display()),
            Err(e) => log::warn!("save: could not copy {} to {}: {}", old.display(), new.display(), e),
        }
    }
}
