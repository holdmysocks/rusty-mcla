//! The camera after START on the start screen (docs/reference/ui/title_cameras.md, "After START").
//!
//! The press-start script (game/flow/PressStart, 0x13cf on) after START: Game_FadeLoadscreenLogo(1),
//! Game_LoadScreenShutdown, Graphics_SetMovieRenderingMode(0), AmbientMgr_SetFrontEnd(0, ..),
//! Warper_SetUseCinematicWarp(0) (req+9), Warper_SetPauseCameraAtEndOfWarp() (req+7), Warper_Warp('MAXHEIGHT'),
//! then it polls Warper_IsReadyToLoadStuff (warper+273) every 10 ms, locks the streaming at the default start
//! and posts EVENT_START_SCREEN_EXIT (the career loads).
//!
//! - **Climb** - `sub_8221A440` case 9 (0x8221B2D8): Entering_Map_MaxHeight (`sub_822192F8`: type 9 ->
//!   0x8201F7D4) from the render camera (the shot) to the eye (NM+352.x, NM+320 = 9000, NM+360.z) =
//!   (-327, 9000, 111.5) looking down along (0, 9000, 10), up (0, 0, -1) (`sub_82202EC0`); 1.5 s, the NavMap
//!   opening's move (`hudmap::NavCam::open`). Lens: Lerp_Float (`sub_82219BA0`) from the shot's lens to 25
//!   degrees, delay 0, over 0.8 T (0x820082E0) = 1.2 s.
//! - **Hold** - the end callback `sub_8221B8C8` with req+7 set: `sub_82204480(pm, -1)` = the pause camera
//!   holds the last matrix (and lens) with no time limit; warper+273 = 1.
//! - **Down** - the career script (game/career, 0x65c) once the player's car is streamed, the districts purged
//!   and WAITUNWARPED(300): Warper_Warp('USER_WAIT_NO_CUT_DOWN') = type 14 (0x8221B5E8): E = the player's
//!   camera matrix (`sub_822AFA10(player+880)`), non-cinematic, lens target 25. Update state 14 (0x8221CA48):
//!   3 frames (m_CityPrestreamWaitFrames), then until no unstreamed sector lies within 100 m (`sub_822D52A0`)
//!   or 5 s; state 26: 0.1 s; then `sub_82219DA0`: the GPS chooser (`sub_82219678`, req+9 = 0) from the held
//!   camera (9000 m > 6000) = Exiting_Map_GPS_HighAltitude (`hudmap::NavExit`), 3.0 s; the hold released
//!   (`sub_82204468`). The lens stays 25 (Lerp from the held 25 to req+96 = 25, delay 0.3 T, over 0.6 T); when
//!   the flight ends (`sub_8221B790` clears warper+272) the game camera and its lens take over.
//! - **LOADING** - Boot.sc.xml: UI.startscreenexit -> Startup_Checks -> the save's load (file.loadStart) ->
//!   SaveGame.sc.xml SG_LoadFile (UISpinner, Save_Icon_Loading) until mc.loadSuccess, which the career
//!   script throws after the flight, Racer_Activate and WAITUNWARPED(1000).
//!
//! - **New career** - the career script's other branch (local8 false: RACE_GH_06 not past state 2) loads the player
//!   at (-199.5, 18.85, -752.03) and calls UILogic_UnPauseCamera, not Warper_Warp: there is no way down. The held
//!   picture stays under the start-up boxes (SaveGame.sc.xml), SAVING and LOADING until CarSelection's fade to
//!   black (recording `work/cutscenes-intro/rec`, c_000 - c_007); the port lets the warp go under that black
//!   (`StartWarp::stay`, main.rs `opening_tick`).
//!
//! (approx.) The port has no career load: the hold lasts until the world is streamed at the car
//! (`ready`), then the script's 0.3 s, 3 frames and 0.1 s; E is the game camera when the flight starts.

use crate::hudmap::{CamRows, NavCam, NavExit, NavLens};
use glam::Vec3;

/// Entering_Map_MaxHeight's length (rule_navcam 3.3)
pub const CLIMB_T: f32 = 1.5;
/// the climb's lens move, a share of the transition's length (0x820082E0)
pub const CLIMB_LENS_SHARE: f32 = 0.8;
/// the map's lens, held and kept down to the car (req+96, 0x8201D298)
pub const MAP_LENS: f32 = 25.0;
/// the career script's WAITUNWARPED(300) after the districts are purged, before Warper_Warp
pub const SCRIPT_WAIT: f32 = 0.3;
/// state 14's frames (m_CityPrestreamWaitFrames, `>= 3`) at 30 Hz
pub const PRESTREAM_FRAMES: f32 = 3.0 / 30.0;
/// state 26 (0x82004288)
pub const HANDOVER: f32 = 0.1;
/// the career script's WAITUNWARPED(1000) after the flight, before 'loadSuccess' (LOADING away)
pub const LOAD_SUCCESS_WAIT: f32 = 1.0;
/// (port only: never hold longer than this for the world)
const HOLD_CAP: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Climb,
    Hold,
    Down,
}

pub struct StartWarp {
    phase: Phase,
    climb: NavCam,
    lens: NavLens,
    t: f32,
    held: CamRows,
    /// time since the world was ready (the script's and the warper's waits)
    ready_t: Option<f32>,
    hold_t: f32,
    down: Option<NavExit>,
    aspect: f32,
    /// a new career: the career script (local8 false, RACE_GH_06 not past state 2) never asks for
    /// USER_WAIT_NO_CUT_DOWN - UILogic_UnPauseCamera instead; the hold stays until CarSelection's black is up
    pub stay: bool,
}

impl StartWarp {
    /// START: from the shot's camera (eye, the point looked at, the lens in degrees).
    pub fn start(eye: Vec3, look: Vec3, lens_deg: f32, car: Vec3, aspect: f32) -> StartWarp {
        let mut climb = NavCam::default();
        climb.open(eye, look, car, aspect);
        let held = CamRows::look_at(eye, look, Vec3::Y);
        log::info!("start warp: Entering_Map_MaxHeight from {:?} lens {:.1}", eye, lens_deg);
        StartWarp { phase: Phase::Climb, climb, lens: NavLens::between(lens_deg, MAP_LENS, 0.0, CLIMB_LENS_SHARE * CLIMB_T), t: 0.0, held, ready_t: None, hold_t: 0.0, down: None, aspect, stay: false }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The climb runs (the press-start script's camera: the shot's hour and its streaming stay).
    pub fn climbing(&self) -> bool {
        self.phase == Phase::Climb
    }

    /// The LOADING tag is up (from the climb's end; the caller keeps it 1 s after the landing).
    pub fn loading(&self) -> bool {
        self.phase != Phase::Climb
    }

    /// One frame: `ready` = the world is streamed at the car; `game` = this frame's game camera (eye,
    /// point looked at, up). The camera (eye, point looked at, up, lens in radians), or None once the
    /// flight has landed (the game camera takes over).
    pub fn step(&mut self, dt: f32, ready: bool, game: (Vec3, Vec3, Vec3)) -> Option<(Vec3, Vec3, Vec3, f32)> {
        self.t += dt;
        match self.phase {
            Phase::Climb => {
                self.climb.update(dt, false, false, glam::Vec2::ZERO, self.aspect);
                let rows = self.climb.view_rows();
                let lens = self.lens.step(dt);
                if self.t >= CLIMB_T - 1e-4 {
                    // (sub_8221B8C8: the pause camera keeps the last matrix and lens)
                    self.held = rows;
                    self.phase = Phase::Hold;
                    log::info!("start warp: at max height {:?}, held", rows.eye);
                }
                Some((rows.eye, rows.look_point(), rows.r1, lens.to_radians()))
            }
            Phase::Hold => {
                self.hold_t += dt;
                if !self.stay && (ready || self.hold_t > HOLD_CAP) {
                    *self.ready_t.get_or_insert(0.0) += dt;
                }
                if self.ready_t.map_or(false, |r| r >= SCRIPT_WAIT + PRESTREAM_FRAMES + HANDOVER - 1e-4) {
                    let e = CamRows::look_at(game.0, game.1, game.2);
                    let ex = NavExit::new(self.held, e);
                    log::info!("start warp: down after {:.2} s held: {:?} ({:.2} s) to {:?}", self.hold_t, ex.kind(), ex.total(), e.eye);
                    self.down = Some(ex);
                    self.phase = Phase::Down;
                }
                let h = self.held;
                Some((h.eye, h.look_point(), h.r1, MAP_LENS.to_radians()))
            }
            Phase::Down => {
                let rows = self.down.as_mut()?.step(dt)?;
                Some((rows.eye, rows.look_point(), rows.r1, MAP_LENS.to_radians()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// shot 1 (the Santa Monica pier) up to the whole city, held, then down onto a car's chase camera
    #[test]
    fn climb_hold_and_down() {
        let s = crate::titlecam::SHOTS[1];
        let eye = Vec3::from(s.eye);
        let mut w = StartWarp::start(eye, eye + Vec3::from(s.dir), s.lens, Vec3::new(820.0, 19.6, -808.0), 16.0 / 9.0);
        let game = (Vec3::new(823.773, 19.52, -816.039), Vec3::new(823.773, 19.52, -816.039) - Vec3::new(0.474, 0.070, -0.878) * 100.0, Vec3::Y);
        let dt = 1.0 / 30.0;
        let mut frames = Vec::new();
        for _ in 0..200 {
            let v = w.step(dt, true, game);
            frames.push((w.phase(), v));
        }
        // the climb: 1.5 s (45 frames), ending straight above the map's centre at 9000 m, lens 25
        let n_climb = frames.iter().take_while(|f| f.0 == Phase::Climb).count();
        assert_eq!(n_climb, 44);
        let (e, l, up, lens) = frames[44].1.unwrap();
        assert!((e - Vec3::new(-327.0, 9000.0, 111.5)).length() < 0.01, "{:?}", e);
        assert!((l.z - (e.z - 100.0 * 10.0 / (9000f32 * 9000.0 + 100.0).sqrt())).abs() < 0.01);
        assert!(up.z < -0.99, "north up: {:?}", up);
        assert!((lens.to_degrees() - 25.0).abs() < 1e-3);
        // the lens: 1.2 s from 40.4 to 25 with the cosine ease (half way at 0.6 s)
        let (_, _, _, l19) = frames[18].1.unwrap();
        assert!((l19.to_degrees() - (40.4 + 25.0) * 0.5).abs() < 0.05, "{}", l19.to_degrees());
        // the hold: ready at once -> 0.3 + 0.1 + 0.1 s, then HighAltitude (3.0 s) to the chase camera
        let n_hold = frames.iter().filter(|f| f.0 == Phase::Hold).count();
        assert_eq!(n_hold, 15);
        let n_down = frames.iter().filter(|f| f.0 == Phase::Down && f.1.is_some()).count();
        assert_eq!(n_down, 90);
        let last = frames.iter().filter(|f| f.1.is_some()).last().unwrap().1.unwrap();
        assert!((last.0 - game.0).length() < 0.5, "{:?}", last.0);
        assert!((last.3.to_degrees() - 25.0).abs() < 1e-3);
    }
}
