//! The layout items' sources (parked.rs holds the items and the spawner draws them): the
//! hangout cores' people and cars and the story scripts' rows of cars. The races' ambient paths
//! are made items by racescript.rs. Decoded 2026-10-21: docs/PEOPLE_TRAFFIC.md "Layout items"
//! (`work/people2/re/ag15_items_report.txt`, `ag16_report.txt`).
//!
//! - local_hangout_core (`game/Hangouts/0xe166126d`) / wager_hangout_core (`0x16e89a34`): once
//!   started (by the hangout's set-up, itself started by career progress - the port's
//!   `mc4_place_open`) the core polls every 500 ms until the player is at least its radius
//!   (flat) from its centre, then AddAmbientToLayout(name i, place i, heading i) for i = 2 ..
//!   count - 1 of its set-up's table (+396 count, +476 places, +672 headings in degrees, +1080
//!   names; a local set-up's from +3280); all removed when the core ends. (The local core also
//!   takes them away while Script_SuspendAllOpponents is set and puts them back once the player
//!   is that far again; only sub_82666850 - the DamagedOut dialog's callback - sets it: not
//!   ported.)
//! - career/Story/0x81d83d96 ("beat me there"): AddRandomCarToLayout(group 0, ...) x 15 at the
//!   strip garage while cut_story_rep_on_strip_garage plays, removed when it ends.

use crate::game::Game;
use crate::peds::Peds;
use crate::render::Gpu;
use crate::vehicles::Vehicles;
use crate::world::World;
use glam::Vec3;

/// A hangout core's items: (type name, place, heading in degrees).
pub struct HangoutItems {
    pub key: &'static str,
    pub local: bool,
    /// the core's centre and radius (local +52 / +64 of the set-up's block)
    pub centre: [f32; 3],
    pub radius: f32,
    pub items: &'static [(&'static str, [f32; 3], f32)],
}

/// The set-ups' tables, entries 2 .. count - 1 (`work/people2/gen_hang_items.py` from
/// `ag15_items_hangouts.txt`; an entry the set-up never writes has no name: be_wager 4 and 8).
pub const HANGOUT_ITEMS: [HangoutItems; 12] = [
    HangoutItems { key: "sc_local_hangout1", local: true, centre: [-1391.9, -8.8, 1111.9], radius: 100.0, items: &[("ped_sc_mb_001_set", [-1342.16, -8.004, 1123.49], 168.087), ("ped_sc_mb_002_set", [-1342.53, -8.004, 1124.77], -15.667), ("ped_sc_mc_001_set", [-1340.61, -7.975, 1131.38], 168.087), ("ped_sc_fb_001_set", [-1340.72, -7.934, 1132.48], -18.467), ("ped_sc_mh_003_set", [-1341.12, -8.04, 1133.89], 88.354), ("ped_sc_mh_004_set", [-1342.11, -8.017, 1133.77], -124.6), ("ped_sc_fa_001_set", [-1342.01, -8.053, 1134.84], -37.197), ("ped_sc_fh_005_set", [-1342.85, -7.838, 1135.86], 144.156), ("ped_fh_001_set", [-1343.12, -7.953, 1137.03], -0.963), ("ped_sc_mc_006_set", [-1344.01, -7.899, 1136.22], -101.242), ("va_chv_chevelle_70", [-1350.33, -8.397, 1107.38], 390.53), ("va_gmc_yukon_05", [-1349.42, -8.494, 1112.18], -232.962), ("va_chv_cobalt_05", [-1347.32, -8.201, 1115.91], 293.177), ("va_cad_ext_03", [-1356.44, -8.275, 1124.19], -62.994)] },
    HangoutItems { key: "be_local_hangout1", local: true, centre: [-2436.3, -8.7, 349.9], radius: 100.0, items: &[("ped_mh_002_set", [-2434.91, -9.527, 352.474], 346.875), ("ped_ma_004_set", [-2439.05, -8.654, 362.178], -174.133), ("ped_mh_003_set", [-2423.91, -9.526, 342.69], -199.066), ("ped_ma_001_set", [-2418.82, -8.588, 354.698], 105.836), ("ped_fb_002_set", [-2438.33, -8.562, 368.129], -112.232), ("ped_fb_005_set", [-2437.83, -8.596, 368.952], -4.775), ("ped_fa_002_set", [-2419.48, -8.576, 356.174], 334.364), ("ped_fh_006_set", [-2437.05, -9.526, 351.532], 0.0), ("ped_fc_002_set", [-2439.11, -8.639, 362.894], -9.504), ("ped_fc_003_set", [-2429.11, -9.527, 370.803], -1.942), ("va_chv_chevelle_70", [-2436.64, -9.525, 356.314], -89.916), ("va_golfr32_04", [-2436.86, -9.637, 358.409], -269.086), ("va_cad_xlr_04", [-2422.74, -9.525, 357.975], 80.776), ("va_vw_jetta_03", [-2423.06, -9.525, 352.178], -114.523)] },
    HangoutItems { key: "sc_wager_hangout1", local: false, centre: [1386.2, -13.1, 1223.6], radius: 178.6, items: &[("ped_sc_mh_004_set", [1422.1, -12.91, 1198.55], 10.378), ("ped_sc_fh_005_set", [1418.0, -12.42, 1181.36], 65.85), ("ped_sc_mb_002_set", [1416.76, -12.424, 1180.91], -124.6), ("ped_sc_ma_001_set", [1409.14, -12.83, 1191.83], -101.242), ("ped_sc_fb_002_set", [1410.29, -11.68, 1191.48], 144.156), ("ped_sc_fb_001_set", [1409.86, -12.644, 1193.06], -0.963), ("ped_sc_mc_001_set", [1408.75, -12.58, 1201.43], 168.087), ("ped_sc_fa_001_set", [1408.64, -12.648, 1202.52], -18.467), ("ped_sc_fh_001_set", [1414.98, -12.581, 1202.52], 252.833), ("ped_sc_mh_002_set", [1415.95, -12.585, 1202.81], 70.206), ("va_cad_escalade_03", [1422.03, -12.608, 1178.2], 304.443), ("va_eclipse_04", [1416.7, -12.608, 1177.58], -119.723), ("va_350z_04", [1406.52, -12.608, 1194.53], 90.641), ("va_mbz_cl500_04", [1412.72, -12.608, 1201.18], 450.739)] },
    HangoutItems { key: "hw_local_hangout1", local: true, centre: [754.7, 7.9, -286.4], radius: 100.0, items: &[("ped_mc_001_set", [743.547, 7.388, -285.87], 259.584), ("ped_fa_001_set", [749.279, 7.388, -286.402], -2.893), ("ped_mh_001_set", [753.3, 7.389, -285.622], 82.933), ("ped_mc_005_set", [758.637, 7.388, -302.299], 204.14), ("ped_mb_004_set", [771.205, 7.389, -298.63], 53.984), ("ped_ma_003_set", [770.132, 7.389, -299.376], -102.837), ("ped_mb_001_set", [771.578, 7.389, -300.076], 125.971), ("ped_fh_001_set", [766.216, 7.389, -303.479], 19.141), ("ped_fh_001_set", [766.326, 7.389, -304.77], 139.435), ("va_350z_04", [746.375, 7.439, -286.63], 226.225), ("va_cad_ctsv_04", [763.504, 7.39, -286.781], 226.225), ("va_gmc_yukon_05", [773.215, 7.39, -290.432], 42.858), ("va_mb_eclipse_04", [772.458, 7.39, -297.102], 138.899)] },
    HangoutItems { key: "hw_local_hangout2", local: true, centre: [-477.5, 3.2, -167.6], radius: 100.0, items: &[("ped_ma_001_set", [-494.726, 2.54, -171.879], 258.357), ("ped_mc_001_set", [-489.317, 2.54, -167.003], -2.893), ("ped_mh_001_set", [-484.199, 2.542, -166.755], 82.933), ("ped_mc_006_set", [-482.932, 2.541, -160.883], 146.606), ("ped_mb_002_set", [-482.358, 2.542, -160.177], 53.984), ("ped_ma_003_set", [-481.062, 2.542, -160.922], -102.837), ("ped_mb_003_set", [-480.647, 2.542, -159.033], 4125.92), ("Ped_mh_004_set", [-492.529, 2.542, -162.614], -0.552), ("ped_fc_002_set", [-491.565, 2.542, -163.904], 139.435), ("ped_fh_003_set", [-492.586, 2.541, -164.787], 97.415), ("va_cad_ext_03", [-491.11, 2.542, -173.189], -173.189), ("va_gmc_yukon_05", [-485.961, 2.542, -158.867], -180.223), ("va_mbz_cl500_04", [-496.689, 2.542, -173.937], -0.743), ("va_vw_jetta_03", [-478.946, 2.542, -159.074], 178.271)] },
    HangoutItems { key: "be_wager_hangout1", local: false, centre: [-2742.3, -3.7, -220.1], radius: 100.0, items: &[("ped_ma_003_set", [-2731.93, 10.477, -251.526], 728.474), ("ped_mb_001_set", [-2731.56, 10.477, -206.427], 133.351), ("", [-2725.44, 10.479, -227.425], -215.886), ("ped_mb_004_set", [-2723.21, 10.478, -232.605], -118.197), ("ped_ma_004_set", [-2722.33, 10.479, -226.997], 101.701), ("ped_mc_005_set", [-2731.13, 10.479, -251.478], -344.148), ("", [-2722.11, 10.479, -232.63], 73.717), ("ped_fc_003_set", [-2725.52, 10.479, -226.406], 21.078), ("ped_fh_005_set", [-2721.8, 10.479, -222.397], -108.303)] },
    HangoutItems { key: "dt_local_hangout1", local: true, centre: [2094.8, -2.5, -231.4], radius: 100.0, items: &[("ped_ma_001_set", [2120.47, -2.98, -221.467], 130.798), ("ped_mc_001_set", [2110.15, -2.981, -213.485], 6.642), ("Ped_mh_001_set", [2119.81, -2.979, -218.763], 26.583), ("ped_mc_006_set", [2118.64, -2.98, -220.481], -76.367), ("ped_mb_002_set", [2084.01, -2.979, -227.272], -251.098), ("ped_ma_003_set", [2083.68, -2.979, -226.205], -305.218), ("ped_mb_003_set", [2104.89, -2.979, -215.776], -100.777), ("Ped_mh_004_set", [2118.54, -2.979, -235.208], -216.56), ("ped_fa_001_set", [2117.75, -2.979, -233.54], -45.77), ("ped_fc_003_set", [2119.09, -2.98, -232.9], 49.452), ("va_chr_300c_05", [2116.02, -2.979, -231.047], 335.424), ("va_vw_jetta_03", [2115.92, -2.979, -222.756], -70.934), ("va_golfr32_04", [2107.21, -2.979, -211.71], -141.678)] },
    HangoutItems { key: "dt_wager_hangout1", local: false, centre: [1840.0, -9.5, 605.6], radius: 100.0, items: &[("ped_mh_001_set", [1830.13, -9.979, 566.169], 265.497), ("ped_mb_001_set", [1826.89, -9.98, 572.933], -30.615), ("ped_mb_003_set", [1831.59, -9.978, 565.596], 148.412), ("ped_mh_004_set", [1821.98, -9.979, 595.816], 362.363), ("ped_mc_004_set", [1845.21, -9.978, 568.881], 67.147), ("ped_fc_002_set", [1837.09, -9.978, 567.737], -271.308), ("ped_fc_003_set", [1845.22, -9.978, 567.134], 445.901), ("ped_fc_003_set", [1825.67, -9.978, 579.296], -211.357), ("ped_fb_004_set", [1830.99, -9.978, 567.253], 358.424), ("ped_fa_001_set", [1833.64, -9.979, 566.235], 210.051), ("va_mbz_cl500_04", [1837.04, -9.978, 565.557], 89.011)] },
    HangoutItems { key: "hw_local_hangout3", local: true, centre: [-1690.9, -4.9, 257.9], radius: 100.0, items: &[("Ped_mh_001_set", [-1709.94, -5.316, 237.619], 258.357), ("ped_mc_006_set", [-1713.63, -5.317, 233.053], -2.893), ("ped_mb_002_set", [-1707.62, -4.338, 226.163], 82.933), ("ped_ma_003_set", [-1713.52, -5.316, 231.955], 169.12), ("ped_mb_003_set", [-1703.07, -5.315, 229.114], -37.313), ("ped_fc_002_set", [-1708.62, -5.315, 225.841], -193.422), ("ped_fc_005_set", [-1702.68, -5.315, 227.668], 245.845), ("ped_fc_003_set", [-1709.21, -5.315, 238.842], -0.552), ("ped_fb_004_set", [-1708.25, -5.315, 237.552], 139.435), ("ped_fa_001_set", [-1701.7, -5.316, 227.783], 97.415), ("va_chv_z28_69", [-1702.63, -5.315, 224.856], 84.86), ("va_cad_ctsv_04", [-1708.36, -5.315, 229.577], -113.887), ("va_mb_eclipse_04", [-1712.24, -5.315, 245.14], 159.122), ("va_vw_jetta_03", [-1710.5, -5.315, 234.354], 362.207)] },
    HangoutItems { key: "hw_wager_hangout1", local: false, centre: [-153.6, 7.12, -473.4], radius: 100.0, items: &[("ped_mc_004_set", [-167.546, 7.173, -480.427], 258.357), ("ped_ma_003_set", [-165.152, 7.173, -464.735], -2.81), ("ped_mb_004_set", [-153.716, 7.175, -486.503], -161.763), ("ped_mc_001_set", [-156.954, 7.174, -485.785], 153.623), ("", [-153.015, 7.175, -485.675], 40.162), ("ped_fc_002_set", [-164.162, 7.175, -464.57], -355.36), ("ped_fh_005_set", [-153.562, 7.175, -484.588], -86.334), ("ped_fh_003_set", [-167.425, 7.175, -485.871], -149.639), ("ped_fc_003_set", [-157.792, 7.175, -485.716], -99.97), ("ped_fa_001_set", [-152.254, 7.174, -484.916], 79.959), ("va_golfr32_04", [-147.109, 7.175, -480.758], -43.422)] },
    HangoutItems { key: "hl_wager_hangout1", local: false, centre: [690.2, 77.0, -1505.5], radius: 100.0, items: &[("ped_mc_001_set", [701.441, 76.574, -1490.78], 92.432), ("ped_mc_006_set", [696.115, 76.573, -1488.21], 28.158), ("ped_mh_002_set", [681.425, 76.575, -1497.88], -5.255), ("ped_mb_001_set", [711.226, 76.574, -1501.12], -16.814), ("ped_mb_004_set", [709.851, 76.575, -1494.98], 63.063), ("ped_mh_001_set", [706.091, 76.575, -1494.52], -119.449), ("ped_mc_005_set", [712.15, 76.575, -1501.99], 111.123), ("ped_mc_004_set", [696.281, 76.575, -1490.45], 151.471), ("ped_fc_002_set", [700.49, 76.575, -1490.12], -49.798), ("ped_fa_001_set", [697.269, 76.574, -1489.42], 86.863), ("va_cad_ext_03", [762.172, 2.542, -308.711], -4.123)] },
    HangoutItems { key: "hl_local_hangout1", local: true, centre: [-1021.8, 30.7, -1856.2], radius: 100.0, items: &[("ped_mh_004_set", [-1033.0, 30.044, -1855.32], 192.087), ("ped_ma_003_set", [-1021.59, 30.986, -1859.46], 7.436), ("ped_mb_003_set", [-1017.5, 30.011, -1858.84], -188.403), ("ped_mb_002_set", [-1032.04, 30.015, -1859.99], 116.781), ("ped_mh_003_set", [-1029.29, 30.017, -1859.17], 63.063), ("ped_mc_006_set", [-1021.8, 30.022, -1842.92], -161.477), ("ped_mc_001_set", [-1028.72, 30.007, -1860.14], 156.554), ("ped_mc_004_set", [-1028.42, 30.051, -1849.16], -77.037), ("ped_ma_001_set", [-1028.82, 30.029, -1851.01], -157.748), ("ped_fh_003_set", [-1027.55, 30.064, -1850.12], -244.879), ("va_cad_ctsv_04", [-1030.97, 30.118, -1848.47], 416.421), ("va_cad_ext_03", [-1025.23, 30.118, -1862.04], -18.249), ("va_chv_cobalt_05", [-1024.34, 30.118, -1846.86], 102.537), ("va_golfr32_04", [-1019.66, 30.118, -1858.01], -30.395)] },
];

/// 0x81d83d96: the strip garage's 15 cars (y 7.445 / 7.365; headings in degrees).
pub const STRIP_GARAGE: [([f32; 3], f32); 15] = [
    ([758.611, 7.445, -317.641], -2.0), ([761.68, 7.445, -317.854], 1.0), ([764.485, 7.445, -317.641], 2.0), ([767.397, 7.445, -317.749], 0.0),
    ([770.23, 7.445, -317.749], -1.5), ([773.192, 7.445, -317.641], 0.0), ([776.104, 7.445, -317.906], 2.0), ([778.936, 7.445, -318.011], 0.0),
    ([758.472, 7.445, -304.607], 177.123), ([761.349, 7.445, -304.621], 180.185), ([764.404, 7.445, -304.607], 177.123), ([767.281, 7.445, -304.621], 180.185),
    ([769.799, 7.365, -285.91], 179.756), ([772.999, 7.365, -285.91], 180.699), ([776.041, 7.365, -285.91], 181.658),
];
pub const STRIP_GARAGE_SCENE: &str = "cut_story_rep_on_strip_garage";

/// AddAmbientToLayout's heading: degrees x 0.01745329.
fn rad(deg: f32) -> f32 {
    deg * 0.017_453_29
}

#[derive(Default)]
struct Core {
    running: bool,
    /// seconds to the next 500 ms poll; the items once added
    poll: f32,
    handles: Option<Vec<u32>>,
}

#[derive(Default)]
pub struct Layout {
    cores: Vec<Core>,
    strip: Option<Vec<u32>>,
}

impl Layout {
    /// Once a frame: the cores and the story scene. `scene` the cine scene playing.
    pub fn update(&mut self, game: &Game, vehicles: &mut Vehicles, data_dir: &str, scene: Option<&str>, dt: f32) {
        if self.cores.len() != HANGOUT_ITEMS.len() {
            self.cores = (0..HANGOUT_ITEMS.len()).map(|_| Core::default()).collect();
        }
        let player = vehicles.player().map(|c| c.state.pos);
        // (test hooks: MCLA_LAYOUT_CORE=<hangout key> runs that core, MCLA_LAYOUT_STRIP=1 the
        // strip garage scene's cars)
        let forced = std::env::var("MCLA_LAYOUT_CORE").unwrap_or_default();
        for (k, h) in HANGOUT_ITEMS.iter().enumerate() {
            let run = game.mc4_place_open(h.key) || forced == h.key;
            let core = &mut self.cores[k];
            if run && !core.running {
                core.poll = 0.0;
            }
            if !run {
                if let Some(v) = core.handles.take() {
                    for x in v {
                        vehicles.remove_layout_item(x);
                    }
                    log::info!("layout: {} core ended, its items removed", h.key);
                }
            } else if core.handles.is_none() {
                core.poll -= dt;
                if core.poll <= 0.0 {
                    core.poll = 0.5;
                    let c = Vec3::from(h.centre);
                    let far = player.map_or(true, |p| glam::Vec2::new(p.x - c.x, p.z - c.z).length() >= h.radius);
                    if far {
                        let v: Vec<u32> = h.items.iter().filter_map(|(n, p, d)| vehicles.add_layout_item(n, Vec3::from(*p), rad(*d), None)).collect();
                        log::info!("layout: {} core - {} items added", h.key, v.len());
                        core.handles = Some(v);
                    }
                }
            }
            core.running = run;
        }
        // the strip garage's scene
        let strip = scene.is_some_and(|s| s.contains(STRIP_GARAGE_SCENE)) || std::env::var("MCLA_LAYOUT_STRIP").is_ok();
        if strip && self.strip.is_none() {
            let mut v = Vec::new();
            for (p, d) in STRIP_GARAGE {
                let p = Vec3::from(p);
                let name = vehicles.random_layout_car(data_dir, p, 0).unwrap_or_default();
                if let Some(x) = vehicles.add_layout_item(&name, p, rad(d), None) {
                    v.push(x);
                }
            }
            log::info!("layout: strip garage scene - {} cars", v.len());
            self.strip = Some(v);
        } else if !strip {
            if let Some(v) = self.strip.take() {
                for x in v {
                    vehicles.remove_layout_item(x);
                }
            }
        }
    }

    /// The people's side of the items, after the spawners ran: the persons the spawner drew,
    /// the one its unspawner removes, those of items gone; then what the next frame sees.
    pub fn people(vehicles: &mut Vehicles, peds: &mut Peds, gpu: &mut Gpu, world: &World) {
        for h in std::mem::take(&mut vehicles.item_peds_due) {
            let Some(it) = vehicles.layout.get(h as usize).cloned().flatten() else { continue };
            if !peds.spawn_item(gpu, &world.collider, h, &it.name, it.pos, it.heading) {
                log::warn!("layout: no person type {}", it.name);
            } else if std::env::var("MCLA_PARK_DUMP").is_ok() {
                log::info!("layout item {} person {} at {:?}", h, it.name, it.pos);
            }
        }
        if let Some(h) = vehicles.ped_remove.take() {
            peds.remove_item_person(h);
        }
        let live = |h: u32| vehicles.layout.get(h as usize).is_some_and(|i| i.is_some());
        peds.remove_items(&live);
        vehicles.item_peds = peds.item_peds();
        vehicles.ped_list = peds.item_list();
    }
}
