//! Driving into a garage as the original does it: `Game/Garages/garage_core` (0x3487df6f), started by
//! each garage's set-up script with its places (Hollywood 0x64e52c82, Beaches 0xd4597f42, South Central
//! 0xd7d0628a), then `<x>_garage_enter_cine` with the garage's `_ent` scene. Decoded 2026-10-21
//! (docs/reference/cutscenes/garage_entry.md):
//!
//! 1. The player within 8 m of the set-up's door point (+92; `Racer_DistanceToPoint`), the game not paused,
//!    `PoliceManager_CheckPoliceActivity(1) <= 1`, not networked. No speed test.
//! 2. `Warper_TransitionTo('STORY_LOCATION_CUT_SIMPLE', element, +24, +36, +48, ..)`: the render camera is
//!    within 100 of the wait camera (eye +24, looking at +48, up +36 - +24 = the `_ent` scene's first
//!    camera), so `Warp_Target_Close` (`hudmap::CineWarp::target_close`, 1 s); its Lerp's `onfocused`
//!    timer of 0.1 s opens ShowLocationName (popup template 63: the place's name, its district) for 2 s.
//! 3. The warp's end sets IsReadyToLoadStuff (`sub_8221BB90`); garage_core polls it every 30 ms, posts
//!    LOCATION_EVENT, waits 500 ms and posts `<X>_GARAGE_ENTER` (MISSION_EVENT): a mission's event script
//!    may play its story scene (REDLIGHT_GH_01_GARAGE_ON_ENTER), else `<x>_garage_enter_cine` places the
//!    car (Racer_InitializeResetPosition) and plays the `_ent` scene (1.167 s, camera only, the car on its
//!    spot), whose `fade_to_black` event (0.167 s, 1.0 s) starts game/misctasks/screen_fade_to_black:
//!    TRANSITIONMOVIE fade_speed_in = the event's duration, held (TransitionOutisReady 0).
//! 4. The scene over, the script sets `<x>_garage_state` 5; garage_core, polling it every 100 ms, fades up
//!    (fade_speed_out 1.0) and brings the garage up.

use glam::Vec3;

use crate::hudmap::CamRows;

/// One garage's places from its set-up script.
pub struct Setup {
    /// the string keys of the place's name and district (the map element's `<key>`, `<key>_desc`)
    pub key: &'static str,
    /// +108, the event garage_core posts as MISSION_EVENT
    pub enter: &'static str,
    /// +92, the door point
    pub door: Vec3,
    /// +24, +36, +48: the wait camera's eye, the point above it, the point it looks at
    pub cam: [Vec3; 3],
    /// the scene `<x>_garage_enter_cine` plays
    pub scene: &'static str,
}

pub const SETUPS: [Setup; 3] = [
    Setup {
        key: "hw_garage",
        enter: "HW_GARAGE_ENTER",
        door: Vec3::new(844.0, 17.6, -819.6),
        cam: [Vec3::new(826.147, 20.826, -804.038), Vec3::new(825.875, 23.433, -803.876), Vec3::new(846.906, 23.986, -815.548)],
        scene: "cut_la_hw_oh_garage_01x_ent",
    },
    Setup {
        key: "be_garage",
        enter: "BE_GARAGE_ENTER",
        door: Vec3::new(-3079.0, -24.8, 159.1),
        cam: [Vec3::new(-3059.59, -16.802, 166.12), Vec3::new(-3059.66, -14.206, 166.116), Vec3::new(-3070.43, -17.059, 182.095)],
        scene: "cut_la_be_smb_garage_01x_ent",
    },
    Setup {
        key: "sc_garage",
        enter: "SC_GARAGE_ENTER",
        door: Vec3::new(-53.6, -19.6, 1331.6),
        cam: [Vec3::new(-54.75, -15.488, 1315.65), Vec3::new(-54.808, -12.893, 1315.64), Vec3::new(-54.368, -15.433, 1336.61)],
        scene: "sc_cut_garage_01_ent",
    },
];

/// garage_core's test: `Racer_DistanceToPoint(player, +92) < 8`
pub const RADIUS: f32 = 8.0;
/// GO TO GARAGE's handler (F-17595): the fixed points it measures the player against, in `SETUPS` order; the nearest
/// garage (of those whose state is 2 - not modelled) gets state 4, which its garage_core takes as the drive-in
pub const GOTO_POINTS: [Vec3; 3] = [Vec3::new(819.693, 17.731, -804.765), Vec3::new(-3049.5, -25.0, 160.0), Vec3::new(-53.6, -19.6, 1331.6)];
/// `sub_8221A440` case 17: Warp_Target_Close below this distance of the render camera to the wait camera
pub const CLOSE: f32 = 100.0;
/// req+96 of Warp_Target_Close (0x8201F9D8), degrees
pub const LENS_DEG: f32 = 21.865_47;
/// the ShowLocationName state: opened 0.1 s into the Lerp, closed by its own 2 s timer
pub const TITLE_FROM: f32 = 0.1;
pub const TITLE_LEN: f32 = 2.0;
/// garage_core: `WAITUNWARPED(500)` between LOCATION_EVENT and the garage's MISSION_EVENT
pub const EVENT_WAIT: f32 = 0.5;
/// the scene's fade_to_black event: its start and duration (fade_speed_in)
pub const FADE_AT: f32 = 0.167;
pub const FADE_LEN: f32 = 1.0;
/// garage_core's poll of `<x>_garage_state` and its fade up (fade_speed_out)
pub const STATE_POLL: f32 = 0.1;
pub const FADE_UP: f32 = 1.0;
/// leaving the garage for the street: LeaveGarage (`sub_826AA930`) starts the fade to black over the garage
/// (`sub_8221F980`: TRANSITIONMOVIE fade_speed_in 1.0, held) and the garage menu's update `sub_826AAA40` sets
/// EXITGARAGE_TRIG 2 after 20 updates; garage_core polls it every 100 ms, waits for the car's vinyl (a load),
/// posts `<X>_GARAGE_EXIT` and the exit script starts the `_ext` scene, which comes up with its `fade_to_scene`
/// event (at 0, duration 0 = 1.0 s) from where the black is. Recorded (`work/garage-intro/capX`): the scene
/// 0.95 s after the press, the rest being loads.
pub const EXIT_FADE: f32 = 1.0;
pub const EXIT_WAIT: f32 = 20.0 / 30.0 + 0.1;

/// The garage whose door point the player is within `RADIUS` of.
pub fn at_door(p: Vec3) -> Option<usize> {
    SETUPS.iter().position(|s| s.door.distance(p) < RADIUS)
}

/// The garage nearest to `p` (its door point).
pub fn nearest(p: Vec3) -> usize {
    (0..SETUPS.len()).min_by(|&a, &b| SETUPS[a].door.distance_squared(p).total_cmp(&SETUPS[b].door.distance_squared(p))).unwrap_or(0)
}

/// The wait camera (the Warper's destination matrix from +24, +36, +48).
pub fn wait_rows(s: &Setup) -> CamRows {
    CamRows::look_at(s.cam[0], s.cam[2], s.cam[1] - s.cam[0])
}

/// Where the entry is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// the warp and the wait after it, `t` from the trigger
    Warp,
    /// the `_ent` scene (or a story scene) is playing
    Scene,
    /// the scene is over: garage_core's next poll of the state, `t` from the scene's end
    Poll,
}

pub struct Entry {
    pub garage: usize,
    pub stage: Stage,
    /// time in the stage
    pub t: f32,
    /// time since the trigger (the place's title runs on it)
    pub clock: f32,
    pub warp: Option<crate::hudmap::CineWarp>,
    /// where the car was held (Racer_Deactivate / the warp)
    pub hold: (Vec3, glam::Quat),
    /// the garage was asked for (the fade up waits for the car to stand in it)
    pub opened: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wait camera is the `_ent` scene's first camera: eye +24, looking along (+48 - +24) - the
    /// Hollywood scene's CSCamera at frame 0 (826.147, 20.826, -804.038), rotation
    /// (0.058, -0.506, 0.032, 0.860).
    #[test]
    fn wait_camera_is_the_scene_camera() {
        let r = wait_rows(&SETUPS[0]);
        let q = glam::Quat::from_xyzw(0.058, -0.506, 0.032, 0.860).normalize();
        let fwd = q * Vec3::NEG_Z;
        let up = q * Vec3::Y;
        let look = (r.look_point() - r.eye).normalize();
        assert!(look.dot(fwd) > 0.9999, "{look:?} {fwd:?}");
        assert!(r.r1.normalize().dot(up) > 0.999, "{:?} {up:?}", r.r1);
        assert_eq!(at_door(Vec3::new(837.18, 17.37, -815.88)), Some(0));
        assert_eq!(at_door(Vec3::new(823.642, 17.5, -807.588)), None);
    }
}
