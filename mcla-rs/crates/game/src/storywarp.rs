//! The warp before a story meeting's scene: `Warper_TransitionTo('STORY_LOCATION_CUT_SIMPLE', element, eye, up point,
//! look-at, 'WarpFromCarToWaitPosition_CarneySign')` in nine story cores (career2, 2026-11-12; docs/CAREER_RACES.md
//! "The story meeting warp"). Each core loads the scene's characters and animation pack, then calls it, polls
//! Warper_IsReadyToLoadStuff every 100 ms ("Waiting For Camera To Get To Safe Spot"), sets the cutscene depth of field and
//! Warper_SetIsWarpingToCutscene(1), and starts the scene's generated script.
//!
//! The transition is the garages' (`garagein`, docs/reference/cutscenes/garage_entry.md, docs/UI.md "STORY_LOCATION_CUT_SIMPLE"):
//! Warp() F-32081 type 17 - the render camera within 100 of the wait camera: `Warp_Target_Close` (the 1 s ease-in-out
//! Lerp, the lens to 21.865 deg, and 0.1 s in the place's band for 2 s: ShowLocationName with the element's registered
//! location name and description); else the climb over the car (F-98643, lens 25 deg), held at its end (F-99030), then
//! warper state 15 waits for the scene and state 26 dives onto the scene's camera at its t 1.0 (F-18267), the scene
//! starting 1 s before the flight lands. Close, state 17 hands over to the scene's camera once the scene is up.

use glam::Vec3;

use crate::hudmap::CamRows;

/// One core's warp.
pub struct Setup {
    /// the id the port's career gives the meeting's scene (`career::News::Scene`)
    pub id: &'static str,
    /// the core (story script = joaat of `<core>_core.sco`)
    pub core: &'static str,
    /// the location element (mcCareer_RegisterLocation's uid)
    pub element: u32,
    /// the eye, the point above it, the point it looks at (the core's three vector locals)
    pub cam: [Vec3; 3],
}

/// From the cores' code (work/career2/raceover/warpcams.py: the floats the three locals are imploded from).
pub const SETUPS: [Setup; 9] = [
    Setup {
        id: "BESTOF_SH_05",
        core: "BESTOF_SH_05",
        element: 609,
        cam: [Vec3::new(1384.344_97, -9.091, 288.316), Vec3::new(1384.292_97, -6.939, 289.284), Vec3::new(1386.085, -5.649, 280.755)],
    },
    Setup {
        id: "REP_ON_HW_STRIP",
        core: "RACE_GH_02",
        element: 500,
        cam: [Vec3::new(731.083, 8.539, -293.005), Vec3::new(730.374, 10.817, -293.258), Vec3::new(748.261, 14.564, -286.889)],
    },
    Setup {
        id: "HANGOUT_SERIES_HOLLYWOODC",
        core: "FIND_SH_03",
        element: 606,
        cam: [Vec3::new(255.119, 10.891, -420.667), Vec3::new(255.864, 13.075, -421.257), Vec3::new(250.214, 14.895, -412.051)],
    },
    Setup {
        id: "DISTRICT_CHAMPS",
        core: "FIND_SH_01B",
        element: 602,
        cam: [Vec3::new(1658.834, 1.44, 230.878), Vec3::new(1658.365, 5.153, 231.521), Vec3::new(1656.145, 1.749, 227.173)],
    },
    Setup {
        id: "FIND_SH_02C",
        core: "FIND_SH_02C",
        element: 605,
        cam: [Vec3::new(-26.695, 37.142, -893.581), Vec3::new(-28.311, 40.664, -891.53), Vec3::new(-23.467, 41.384, -898.323)],
    },
    Setup {
        id: "CITY_CHAMP",
        core: "CITY_CHAMP",
        element: 602,
        cam: [Vec3::new(1658.834, 1.44, 230.878), Vec3::new(1658.365, 5.153, 231.521), Vec3::new(1656.145, 1.749, 227.173)],
    },
    Setup {
        id: "RACE_SH_01",
        core: "RACE_SH_01",
        element: 600,
        // (the point above the eye is written with y -22.627 in the core: the eye's up row points down)
        cam: [Vec3::new(-130.112, 18.506, -732.812), Vec3::new(-130.681, -22.627, -731.19), Vec3::new(-137.082, 23.798, -748.708)],
    },
    Setup {
        id: "RACES_IN_BEACHES",
        core: "FIND_SH_03B",
        element: 607,
        cam: [Vec3::new(-2924.722, 5.161, 57.139), Vec3::new(-2925.24, 6.862, 56.478), Vec3::new(-2891.103, 25.46, 83.02)],
    },
    Setup {
        id: "HANGOUT_PINKSLIP_HILLS",
        core: "FIND_SH_02",
        element: 603,
        cam: [Vec3::new(-3121.483, 10.008, 514.29), Vec3::new(-3121.753, 12.406, 514.232), Vec3::new(-3155.511, 6.522, 528.823)],
    },
];

/// The registered locations' strings (the career script `game/career/0x1529a945`: mcCareer_RegisterLocation(uid, name,
/// description, ..)) the band shows.
pub fn location_keys(element: u32) -> Option<(&'static str, &'static str)> {
    Some(match element {
        500 => ("hw_local_hangout1", "hw_local_hangout1_desc"),
        600 => ("CARNEYS_TITLE", "CARNEYS_DESC"),
        601 => ("FIGUROA_TITLE", "FIGUROA_DESC"),
        602 => ("STANDARD_TITLE", "STANDARD_DESC"),
        603 => ("PIER_TITLE", "PIER_DESC"),
        604 => ("PROMENADE_TITLE", "PROMENADE_DESC"),
        605 => ("CHATEAU_TITLE", "CHATEAU_DESC"),
        606 => ("PINKS_TITLE", "PINKS_DESC"),
        607 => ("GEORGIAN_TITLE", "GEORGIAN_DESC"),
        608 => ("BRIDGE_TITLE", "BRIDGE_DESC"),
        609 => ("PANTRY_TITLE", "PANTRY_DESC"),
        _ => return None,
    })
}

pub fn of_id(id: &str) -> Option<usize> {
    SETUPS.iter().position(|s| s.id == id)
}

/// The wait camera (the Warper's destination matrix from the eye, the point above it and the look-at).
pub fn wait_rows(s: &Setup) -> CamRows {
    CamRows::look_at(s.cam[0], s.cam[2], s.cam[1] - s.cam[0])
}

/// The core polls Warper_IsReadyToLoadStuff at the call and then every 100 ms: the first poll at or after the flight's end.
pub const POLL: f32 = 0.1;

pub fn ready_at(flight: f32) -> f32 {
    ((flight - 1e-4).max(0.0) / POLL).ceil() * POLL
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// the flight (the Lerp or the climb) and the hold to the core's poll
    Warp,
    /// far: the held climb's end, then the dive onto the scene's camera
    Dive,
    /// the scene is up (the dive may still fly over its first second)
    Scene,
}

pub struct Entry {
    pub setup: usize,
    /// the scene (`cutscene::Cutscenes` index)
    pub scene: usize,
    pub stage: Stage,
    pub t: f32,
    /// time since the call (the band runs on it)
    pub clock: f32,
    pub warp: Option<crate::hudmap::CineWarp>,
    pub climb: Option<crate::hudmap::CineClimb>,
    pub dive: Option<crate::hudmap::CineWarp>,
    /// where the car was held (the core's Racer_Deactivate)
    pub hold: (Vec3, glam::Quat),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every set-up's numbers are the core's: the floats of the three vectors are pushf immediates of the core's
    /// script (a negative one written positive and negated), the element follows the transition's name, and the
    /// scene's id names the scene the core starts (skipped without the extracted data).
    #[test]
    fn the_setups_are_the_cores() {
        let root = std::path::Path::new("D:/MCLA/extracted/cache/script_obj/game/career/Story");
        if !root.is_dir() {
            return;
        }
        for s in &SETUPS {
            let h = mcla_formats::hash::joaat(&format!("{}_core.sco", s.core));
            let f = std::fs::read(root.join(format!("0x{:08x}", h))).unwrap_or_else(|_| panic!("{}", s.core));
            let d = mcla_formats::sco::decode(&f).unwrap().data;
            let has = |b: &[u8]| d.windows(b.len()).any(|w| w == b);
            assert!(has(b"STORY_LOCATION_CUT_SIMPLE"), "{}", s.core);
            for v in s.cam {
                for x in [v.x, v.y, v.z] {
                    let bits = x.abs().to_le_bytes();
                    assert!(has(&bits), "{} {}", s.core, x);
                }
            }
            assert!(location_keys(s.element).is_some(), "{}", s.element);
        }
        assert_eq!(of_id("CITY_CHAMP"), Some(5));
        assert_eq!(ready_at(1.0), 1.0);
        assert!((ready_at(1.05) - 1.1).abs() < 1e-6);
    }

    /// RACE_SH_01's up point lies below its eye, as the core writes it.
    #[test]
    fn race_sh_01_up_is_as_written() {
        let r = wait_rows(&SETUPS[6]);
        assert!(r.r1.y < 0.0);
        let b = wait_rows(&SETUPS[0]);
        assert!(b.r1.y > 0.9);
    }
}
