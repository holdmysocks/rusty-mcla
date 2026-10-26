//! The start screen's cameras: the press-start script `game/flow/PressStart` (0xe9dde3bb.sco,
//! `work/career2/re/dis2/game__0x72f9fd0c__0xe9dde3bb.sco.txt`). Rules: `docs/reference/ui/title_cameras.md`.
//!
//! - Seven shots in the script's own tables (0x97 .. 0x4cf): eye, direction, lens, hour, weather,
//!   cutscene depth of field and the ambient manager's front-end values.
//! - One is drawn as the script starts: RAND_RESET twice (the script's generator seeded from the game's
//!   global MWC 0x82860890, sub_822C8618), then RAND_INT_RANGE(0, 6) (sub_822C83B0: both ends in).
//! - The shot is held: a still camera, `Game_SetCamera(eye, eye + dir, lens, 0, 5, 10000)`
//!   (sub_822BFBA8: look-at, the lens in degrees, near 5 m, far 10 km). Only when the attract movie
//!   has played to its end does the next one come (index + 1, 7 -> 0, 0xee6); a movie stopped by a
//!   button leaves the shot as it was (0x131e).
//! - Each shot (0x5a8): `Graphics_WarpToTimeOfDay(hour, 100)` (the clock's update is off from the
//!   script's start, `Graphics_GetAndSetAutoUpdateTimeOfDay(0)`, and given back after START);
//!   `Weather_SetCurrentWeather(w, 1)` (W = w, the weights snapped - 0 Nice, 1 Cloudy);
//!   near > 0.1: `PostProcessing_SetCutsceneDofNear`, far > 0.1: `..Far` and `SetCutsceneDof`
//!   (the post takes the cutscene vector 0x827E5780), else `SetGameDof` (the hour's own, and the
//!   cutscene vector reset to 0x827E5790's 500 / 1000 / 0.75 / 1); then the camera, three frames,
//!   `City_DoneStreaming(250)` polled every 10 ms.

use glam::Vec3;

/// One front-end shot of the press-start script.
#[derive(Clone, Copy, Debug)]
pub struct Shot {
    pub eye: [f32; 3],
    pub dir: [f32; 3],
    /// the lens, degrees (Game_SetCamera's third value)
    pub lens: f32,
    pub hour: f32,
    /// 0 Nice, 1 Cloudy (the weather index W)
    pub weather: u8,
    /// cutscene depth of field near / far (0: the game's own)
    pub dof: [f32; 2],
    /// `AmbientMgr_SetFrontEnd(1, a, b, c, d, n)`: ambient manager +8312, +8316, +8304, +8308, and the
    /// front-end density index (0x8282A7D8, sub_8273B670)
    pub amb: [f32; 4],
    pub amb_index: i32,
}

/// The script's tables (locals 6 / 28 / 50 / 58 / 66 / 74 / 82 / 106 / 114 / 122 / 130 / 138).
/// (locals 90 / 98 - 100 and 1500 for shots 3 and 4 - are tested and left unused, 0x608 / 0x616.)
pub const SHOTS: [Shot; 7] = [
    Shot { eye: [2903.0, 83.0, 22.6], dir: [-0.993, -0.106, 0.044], lens: 39.2, hour: 5.776, weather: 1, dof: [2000.0, 3000.0], amb: [0.06, 0.0, 0.1, 425.0], amb_index: 4 },
    Shot { eye: [-2932.9, -14.3, 368.4], dir: [-0.854, 0.165, 0.494], lens: 40.4, hour: 17.819, weather: 0, dof: [0.0, 0.0], amb: [0.0, 0.0, 0.1, 200.0], amb_index: 4 },
    Shot { eye: [-339.0, 44.1, -753.2], dir: [0.963, -0.15, 0.225], lens: 34.8, hour: 6.498, weather: 0, dof: [0.0, 0.0], amb: [0.0, 0.05, 0.1, 162.0], amb_index: 4 },
    Shot { eye: [2531.0, 9.0, 141.8], dir: [-0.996, 0.088, 0.01], lens: 41.0, hour: 16.752, weather: 0, dof: [800.0, 3000.0], amb: [0.0, 0.165, 0.1, 131.0], amb_index: 5 },
    Shot { eye: [864.9, 39.8, -1415.1], dir: [0.633, 0.135, -0.762], lens: 32.9, hour: 14.81, weather: 0, dof: [800.0, 3000.0], amb: [0.0, 0.037, 0.1, 265.0], amb_index: 4 },
    Shot { eye: [-2603.6, 2.8, 239.3], dir: [-0.371, -0.004, -0.929], lens: 33.3, hour: 10.4, weather: 0, dof: [900.0, 3000.0], amb: [0.0, 0.0, 0.1, 226.0], amb_index: 4 },
    Shot { eye: [939.2, 59.6, 739.7], dir: [0.508, -0.21, -0.835], lens: 50.5, hour: 18.0, weather: 1, dof: [3500.0, 4000.0], amb: [0.055, 0.0, 0.1, 270.0], amb_index: 4 },
];

/// Game_SetCamera's near and far planes for the shots (5, 10000).
pub const NEAR: f32 = 5.0;
pub const FAR: f32 = 10000.0;

/// The cutscene depth-of-field vector's start (0x827E5780 in the image) and what SetGameDof copies
/// into it (0x827E5790).
const DOF_CUT_START: [f32; 4] = [100.0, 300.0, 1.0, 1.0];
const DOF_GAME: [f32; 4] = [500.0, 1000.0, 0.75, 1.0];

/// What a newly set shot asks of the game (applied by the caller).
#[derive(Clone, Copy, Debug)]
pub struct Apply {
    pub hour: f32,
    /// 0 Nice, 1 Cloudy
    pub weather: u8,
}

/// The ambient manager's front-end mode while a shot is up (AmbientMgr_SetFrontEnd): the camera's eye
/// and its direction, the four values and the density index.
#[derive(Clone, Copy, Debug)]
pub struct FrontEndAmbient {
    pub eye: Vec3,
    pub dir: Vec3,
    pub amb: [f32; 4],
    pub index: i32,
}

#[derive(Default)]
pub struct TitleCam {
    /// the shot up, while the start screen is
    pub shot: Option<usize>,
    /// the cutscene depth-of-field vector (persists as the original's global does)
    dof_cut: Option<[f32; 4]>,
    /// the cutscene depth of field in force (SetCutsceneDof) or the game's (None)
    dof_on: bool,
    /// the hour before the start screen, given back after it
    saved_hour: Option<f32>,
    /// the attract movie as last seen: (time, duration)
    movie: Option<(f32, f32)>,
    /// the black around the attract movie (TRANSITIONMOVIE, mask_color 0 - boomcam::CutMask)
    mask: crate::boomcam::CutMask,
    fade: Fade,
    /// the attract movie has just gone: played to its end (true) or stopped (false)
    ended: Option<bool>,
}

/// The press-start script's steps around the attract movie (0xad2 .. 0xe9f, 0xea4 .. 0x12f0, 0x131e .. 0x13b1).
#[derive(Default, Clone, Copy, Debug, PartialEq)]
enum Fade {
    #[default]
    None,
    /// TIMERA > 110000: function 0xaf8(1, 1, 1.0, 0): transition_type 0, mask_color 0, fade_speed_in 1.0,
    /// StartOfTransitionin 1, waited until the movie clears it (the mask full)
    Down,
    /// 0x9fd: the movie player's tint 1 -> 0 in ten steps of WAIT(30), StopMovie, three WAIT(0) - then the
    /// attract movie starts
    Tint(f32),
    /// the attract movie asked for; `f32`: seconds waiting for it to show
    Movie(f32),
    /// the movie gone: the frames before the fade up (0xfc4(1, 1, 1.0, 0): StartOfTransitionout 1,
    /// fade_speed_out 1.0); `bool`: played to its end - the next shot's City_DoneStreaming first
    Up(f32, bool),
}

/// 0x9fd's ten tint steps (each WAIT(30): one 1/30 s frame) and the three WAIT(0) after StopMovie.
pub const TINT_FRAMES: f32 = 13.0 / 30.0;
/// The natural end's frames before its fade up: three WAIT(0) after StopMovie, three after the camera
/// (0x63d), then City_DoneStreaming.
pub const SHOT_FRAMES: f32 = 6.0 / 30.0;

/// RAND_INT_RANGE(0, 6) after RAND_RESET twice: the global MWC's state at that point depends on every draw
/// of the game's start-up, so the port takes the system clock (approximate - see the doc).
/// Test hook: MCLA_TITLE_SHOT=<0..6>.
fn draw() -> usize {
    if let Some(n) = std::env::var("MCLA_TITLE_SHOT").ok().and_then(|v| v.trim().parse::<usize>().ok()) {
        return n % SHOTS.len();
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    // (the script generator's step, sub_822C83B0: x = lo * 0x5CDCFAA7 + carry, value (x & 0x7fffffff) % 7)
    let seed = (t ^ (t >> 32)) as u32 | 1;
    let x = (seed as u64) * 0x5CDC_FAA7 + (seed ^ seed.rotate_left(16)) as u64;
    ((x as u32 & 0x7fff_ffff) % 7) as usize
}

impl TitleCam {
    /// Once a frame. `title`: the start screen is up (Mode::MainMenu); `attract`: the attract movie's
    /// (time, duration) while it plays. Returns what a newly set shot asks of the clock and weather.
    pub fn frame(&mut self, title: bool, attract: Option<(f32, f32)>, hour_now: f32) -> Option<Apply> {
        if !title {
            self.shot = None;
            self.movie = None;
            return None;
        }
        let next = match (self.shot, self.movie, attract) {
            // the script's start: one drawn
            (None, _, _) => {
                self.saved_hour.get_or_insert(hour_now);
                Some(draw())
            }
            // the attract movie gone: played to its end -> the next shot (0xea4 .. 0xfb2); stopped by
            // a button -> the same (0x131e)
            (Some(i), Some((t, d)), None) if d > 0.0 && t >= d - 0.25 => {
                self.ended = Some(true);
                Some((i + 1) % SHOTS.len())
            }
            (Some(_), Some(_), None) => {
                self.ended = Some(false);
                None
            }
            _ => None,
        };
        self.movie = attract;
        let i = next?;
        self.shot = Some(i);
        let s = &SHOTS[i];
        if s.dof[0] > 0.1 {
            self.dof_cut.get_or_insert(DOF_CUT_START)[0] = s.dof[0];
        }
        if s.dof[1] > 0.1 {
            self.dof_cut.get_or_insert(DOF_CUT_START)[1] = s.dof[1];
            self.dof_on = true;
        } else {
            self.dof_cut = Some(DOF_GAME);
            self.dof_on = false;
        }
        log::info!("title camera: shot {} (eye {:?}, hour {}, weather {})", i, s.eye, s.hour, s.weather);
        Some(Apply { hour: s.hour, weather: s.weather })
    }

    /// The black around the attract movie, once a frame after `frame`: `idle` is the start screen's
    /// attract clock (main.rs; the movie starts once it passes `limit`) - held under the limit while the
    /// screen goes black, let through when the script would start the movie. Returns the mask's opacity.
    pub fn attract_fade(&mut self, dt: f32, idle: &mut f32, limit: f32, movie_on: bool, streamed: bool) -> f32 {
        if self.shot.is_none() {
            self.fade = Fade::None;
            self.mask = Default::default();
            self.ended = None;
            return 0.0;
        }
        // (main.rs adds the frame's time and starts the movie past the limit in one step: the clock is held a
        // quarter second under it, and the fade starts on the frame before it would pass - TIMERA > 110000)
        let hold = limit - 0.25;
        let ended = self.ended.take();
        self.fade = match self.fade {
            Fade::None if !movie_on && *idle + dt > limit => {
                *idle = hold;
                self.mask.start(1.0, 0.01, f32::INFINITY);
                Fade::Down
            }
            Fade::Down => {
                *idle = hold;
                if self.mask.full() { Fade::Tint(0.0) } else { Fade::Down }
            }
            Fade::Tint(t) => {
                *idle = hold;
                if t + dt >= TINT_FRAMES {
                    // (the movie starts on the next frame: main.rs's attract clock)
                    *idle = limit + 1.0;
                    Fade::Movie(0.0)
                } else {
                    Fade::Tint(t + dt)
                }
            }
            Fade::Movie(_) if ended.is_some() => Fade::Up(0.0, ended == Some(true)),
            // (no movie came - a headless run: as a stopped one)
            Fade::Movie(w) if !movie_on && w > 0.5 => Fade::Up(0.0, false),
            Fade::Movie(w) => Fade::Movie(if movie_on { 0.0 } else { w + dt }),
            Fade::Up(t, natural) => {
                let wait = if natural { SHOT_FRAMES } else { TINT_FRAMES };
                if t + dt >= wait && (streamed || !natural) {
                    self.mask.release(1.0);
                    Fade::None
                } else {
                    Fade::Up(t + dt, natural)
                }
            }
            f => f,
        };
        self.mask.update(dt)
    }

    /// The hour the start screen will give back (`take_saved_hour`), while it is up.
    pub fn saved_hour(&self) -> Option<f32> {
        self.saved_hour
    }

    /// The hour to give back once the start screen has gone (the script hands the clock's update
    /// back after START; the career's load then sets its own clock).
    pub fn take_saved_hour(&mut self) -> Option<f32> {
        if self.shot.is_none() {
            self.saved_hour.take()
        } else {
            None
        }
    }

    /// The view: eye, the point looked at, the lens in degrees.
    pub fn view(&self) -> Option<(Vec3, Vec3, f32)> {
        let s = &SHOTS[self.shot?];
        let eye = Vec3::from(s.eye);
        Some((eye, eye + Vec3::from(s.dir), s.lens))
    }

    pub fn eye(&self) -> Option<Vec3> {
        self.shot.map(|i| Vec3::from(SHOTS[i].eye))
    }

    /// The cutscene depth of field in force, if any (else the hour's own).
    pub fn dof(&self) -> Option<[f32; 4]> {
        self.shot?;
        if self.dof_on { self.dof_cut } else { None }
    }

    /// The ambient manager's front-end mode.
    pub fn ambient(&self) -> Option<FrontEndAmbient> {
        let s = &SHOTS[self.shot?];
        Some(FrontEndAmbient { eye: Vec3::from(s.eye), dir: Vec3::from(s.dir), amb: s.amb, index: s.amb_index })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shots_follow_after_a_played_attract_movie_only() {
        std::env::set_var("MCLA_TITLE_SHOT", "6");
        let mut t = TitleCam::default();
        assert!(t.frame(true, None, 14.0).is_some());
        assert_eq!(t.shot, Some(6));
        assert_eq!(t.dof(), Some([3500.0, 4000.0, 1.0, 1.0]));
        // a movie stopped part-way: the same shot
        t.frame(true, Some((10.0, 60.0)), 18.0);
        assert!(t.frame(true, None, 18.0).is_none());
        assert_eq!(t.shot, Some(6));
        // played to its end: 6 -> 0
        t.frame(true, Some((59.9, 60.0)), 18.0);
        let a = t.frame(true, None, 18.0).unwrap();
        assert_eq!(t.shot, Some(0));
        assert_eq!((a.hour, a.weather), (5.776, 1));
        // 0 -> 1: no depth of field, the cutscene vector reset by SetGameDof
        t.frame(true, Some((60.0, 60.0)), 5.776);
        t.frame(true, None, 5.776);
        assert_eq!(t.shot, Some(1));
        assert_eq!(t.dof(), None);
        t.frame(true, Some((60.0, 60.0)), 5.776);
        t.frame(true, None, 5.776);
        t.frame(true, Some((60.0, 60.0)), 5.776);
        t.frame(true, None, 5.776);
        assert_eq!(t.shot, Some(3));
        assert_eq!(t.dof(), Some([800.0, 3000.0, 0.75, 1.0]));
        // leaving the start screen gives the first hour back
        t.frame(false, None, 16.752);
        assert_eq!(t.take_saved_hour(), Some(14.0));
        std::env::remove_var("MCLA_TITLE_SHOT");
    }

    /// The black around the attract movie at 30 frames a second (recording rec4: 1.0 s down from 133.4 s,
    /// the movie at 135.1 s, black until the next shot is streamed, 0.9 - 1.0 s up).
    #[test]
    fn attract_movie_fades() {
        let dt = 1.0 / 30.0;
        let mut t = TitleCam::default();
        t.frame(true, None, 12.0);
        t.shot = Some(0);
        let mut idle = 109.95f32;
        // down: from the frame before the clock passes 110 s, full black after 30 movie frames
        let mut frames = 0;
        let mut a = 0.0;
        while a < 1.0 {
            idle += dt;
            a = t.attract_fade(dt, &mut idle, 110.0, false, true);
            frames += 1;
            assert!(idle < 110.0 || a >= 1.0, "movie let through before the black");
        }
        // (one frame of the clock, then 30 movie frames of 3.333 % each: 1.0 s)
        assert_eq!(frames, 31, "frames to full black");
        // then the tint steps and three frames before the movie is let through
        let mut n = 0;
        while idle < 110.0 {
            idle += dt;
            t.attract_fade(dt, &mut idle, 110.0, false, true);
            n += 1;
            assert!(n < 40);
        }
        assert!((13..=15).contains(&n), "{n}");
        // the movie plays to its end; black until streamed, then up in 1 s
        t.frame(true, Some((36.99, 37.0)), 5.776);
        assert_eq!(t.attract_fade(dt, &mut idle, 110.0, true, true), 1.0);
        t.frame(true, None, 5.776);
        assert_eq!(t.shot, Some(1));
        for _ in 0..20 {
            assert_eq!(t.attract_fade(dt, &mut idle, 110.0, false, false), 1.0);
        }
        let mut up = 0;
        loop {
            up += 1;
            if t.attract_fade(dt, &mut idle, 110.0, false, true) <= 0.0 {
                break;
            }
            assert!(up < 60);
        }
        assert!((30..=38).contains(&up), "{up}");
    }
}
