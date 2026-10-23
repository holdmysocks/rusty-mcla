//! The start-up sequence before the start screen, as the original plays it (docs/reference/ui/boot.md):
//! one Flash movie, `legals/legals.swf`, made by `sub_822BFD48` -> `sub_821F7E40` from `sub_822C2EA8` (game/flow/Boot)
//! and driven by its own frame-counted script: two legal pages, the Rockstar Games and Rockstar San Diego logos, the
//! game's logo with LOADING, then - once the game says loading is done - the black background fades off the city and
//! the logo stays on the start screen until START.
//!
//! What the game does to the movie (decoded):
//! - `sub_821F8038`: four updates of 1/60 s, then `_global.aspect` (1 = wide screen, `sub_82178848`), `lang`, `zone`,
//!   `platform` (0 = Xbox 360).
//! - Every frame (the loading callback `sub_821FC008` -> `sub_821F9918`): one update of 1/60 s (the movie is authored
//!   at 60 frames a second; the console presents every second vblank, so it plays 30 movie frames a second - the
//!   recomp holds it to that too, `frame_timing.toml` "Intro movies"); `StartOfTransitionOut` = 1 once
//!   `Game_SetLoadingDone` was called; `cur_visibility` = 0 while `Game_FadeLoadscreenLogo(1)`; `EndOfTransitionOut` 1 / 2
//!   read back as `Game_GetLoadingDone`.
//! - `Game_LoadScreenShutdown` (START pressed): the object goes 4 frames later and is not drawn in between - the logo
//!   is gone at once (recorded).
//! - The movie's `FSCommand:StartLoadingMusic` at its frame 340 -> the sound `LOADING_MUSIC_01` (`sub_821EEB48`).
//!
//! No button skips any of it: the press-start script (`game/flow/PressStart`, 0xe9dde3bb) reads the pad only after
//! `Game_GetLoadingDone`.

use crate::flash_ui::{FlashMovie, UiShared};
use mcla_config::Intro;
use mcla_flash::value::Value;

/// `legals/legals.swf` (the other three, `sc_legals`, `jp_legals`, `jp_sc_legals`, are for 4:3 / standard-definition
/// video modes and Japanese, `sub_821F7E40`)
pub const LEGALS_MOVIE: &str = "resources/ui/legals/0x260847ab.r27";
/// one movie frame per present at 30 a second
pub const FRAME: f32 = 1.0 / 30.0;
/// the movie's `_global.timer` when its script starts the loading music (`FSCommand:StartLoadingMusic`)
const MUSIC_TIMER: i32 = 340;
/// FAST: the movie's `_global.timer` it is run to without being shown - the frame before the game's logo plays
/// (timer 623 `logomc.play()`): the legal pages and the publisher logos are left out
const FAST_TIMER: i32 = 622;
/// The press-start script's wait before the attract movie: `TIMERA() > 110000` (ms), the timer set to 0 when
/// `Game_GetLoadingDone` comes and again after each attract movie. Other buttons do not reset it.
pub const ATTRACT_IDLE: f32 = 110.0;
/// the loading music's sound (`sub_821EEB48`)
pub const LOADING_MUSIC: &str = "LOADING_MUSIC_01";

pub struct Boot {
    pub mode: Intro,
    pub movie: Option<FlashMovie>,
    tried: bool,
    acc: f32,
    loading_done_sent: bool,
    /// `Game_GetLoadingDone`: the movie's transition out has ended; the start screen takes START from now
    pub title: bool,
    hidden: bool,
}

impl Boot {
    /// `None` for `skip` (OFF): the original's skip path (the recomp's `--mcla_intro=skip`, jump 0x822C2F08 ->
    /// 0x822C2F7C) never makes the movie and the start screen has no logo.
    pub fn new(mode: Intro) -> Option<Boot> {
        (mode != Intro::Skip).then_some(Boot { mode, movie: None, tried: false, acc: 0.0, loading_done_sent: false, title: false, hidden: false })
    }

    /// Load the movie (once, when the UI's shared textures exist).
    pub fn load(&mut self, data_dir: &str, shared: &UiShared) {
        if std::mem::replace(&mut self.tried, true) {
            return;
        }
        let Some(mut m) = FlashMovie::load(data_dir, LEGALS_MOVIE, shared) else {
            // (no movie: the start screen as with OFF)
            self.title = true;
            return;
        };
        // sub_821F8038: four updates, then the globals
        for _ in 0..4 {
            m.player.tick();
        }
        let p = &mut m.player;
        p.set_global("aspect", Value::Int(1));
        p.set_global("lang", Value::Int(0));
        // (zone 1 = the North American pages with the ESRB line, as recorded; the movie's own default)
        p.set_global("zone", Value::Int(1));
        p.set_global("platform", Value::Int(0));
        if self.mode == Intro::Fast {
            let mut n = 0;
            while timer(&m) < FAST_TIMER && n < 2000 {
                m.player.tick();
                n += 1;
            }
        }
        log::info!("boot: {} loaded ({} frames a second), mode {}, starts at movie timer {}", LEGALS_MOVIE, m.player.movie.fps, self.mode.word(), timer(&m));
        self.movie = Some(m);
    }

    /// One game frame. `loading_done`: `Game_SetLoadingDone` (the city is there); `hide_logo`:
    /// `Game_FadeLoadscreenLogo(1)` (the press-start script passes "the attract movie is playing").
    pub fn update(&mut self, dt: f32, loading_done: bool, hide_logo: bool) {
        let Some(m) = self.movie.as_mut() else { return };
        let p = &mut m.player;
        if loading_done && !std::mem::replace(&mut self.loading_done_sent, true) {
            p.set_global("StartOfTransitionOut", Value::Int(1));
            log::info!("boot: loading done at movie timer {}", p.get_global("timer").map_or(0, |v| v.to_i32()));
        }
        if hide_logo != self.hidden {
            self.hidden = hide_logo;
            if !hide_logo {
                // (the port's player does not run a clip whose alpha is 0 - player.rs `tick_inst` -, which the
                // movie's root clip `legal` reaches when it fades out; its script goes on easing back up, so the
                // clip is given the smallest alpha to run again)
                if let Some(id) = p.find("legal") {
                    if let Some(i) = p.inst_mut(id) {
                        i.tint[3] = i.tint[3].max(1);
                    }
                }
            }
        }
        p.set_global("cur_visibility", Value::Int(!hide_logo as i32));
        self.acc += dt.min(0.25);
        let mut n = 0;
        while self.acc >= FRAME && n < 8 {
            self.acc -= FRAME;
            m.player.tick();
            n += 1;
        }
        if !self.title {
            let end = m.player.get_global("EndOfTransitionOut").map_or(0, |v| v.to_i32());
            if end == 1 || end == 2 {
                self.title = true;
                log::info!("boot: start screen (movie timer {})", timer(m));
            }
        }
    }

    /// The loading music is asked for (from the movie's frame 340 on).
    pub fn music(&self) -> bool {
        self.movie.as_ref().is_some_and(|m| timer(m) >= MUSIC_TIMER)
    }

    /// What is drawn: the movie over the city. The movie's black background (`black_bg`, 1280 x 720) is also laid
    /// over the parts of a screen wider than 16:9, at its alpha (the original's screen is the stage).
    pub fn paint(&mut self, ctx: &egui::Context, painter: &egui::Painter) {
        let screen = ctx.screen_rect();
        let Some(m) = self.movie.as_mut() else {
            // (before the movie is there: black, as the original's first frames)
            painter.rect_filled(screen, 0.0, egui::Color32::BLACK);
            return;
        };
        let a = m.player.get_global("cur_blackbg_alpha").map_or(0.0, |v| v.to_f32()).clamp(0.0, 100.0) / 100.0;
        let half = screen.height() * 1280.0 / 720.0 * 0.5;
        if a > 0.0 && screen.width() * 0.5 > half {
            let c = egui::Color32::from_black_alpha((a * 255.0).round() as u8);
            let (l, r) = (screen.center().x - half, screen.center().x + half);
            painter.rect_filled(egui::Rect::from_min_max(screen.min, egui::pos2(l, screen.max.y)), 0.0, c);
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(r, screen.min.y), screen.max), 0.0, c);
        }
        m.paint(ctx, painter);
    }
}

/// What the movie needs from the UI before the game is loaded (bootload.rs): the UI texture dictionary, the string
/// tables (the main one and those beside it, as `career::Career::load` merges them) and `UILogic`.
pub fn shared(data_dir: &str) -> UiShared {
    use mcla_formats::strtbl::StringTable;
    let ui = std::path::Path::new(data_dir).join("ui");
    let mut strings = std::fs::read(ui.join("0xa4c1e8fa")).ok().and_then(|d| StringTable::parse(&d).ok()).unwrap_or_default();
    if let Ok(rd) = std::fs::read_dir(&ui) {
        let mut files: Vec<std::path::PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.file_name().map_or(false, |n| n != "0xa4c1e8fa" && n != "0xac5297d5")).collect();
        files.sort();
        for f in files {
            if let Some(t) = std::fs::read(&f).ok().and_then(|d| StringTable::parse(&d).ok()) {
                strings.merge(t);
            }
        }
    }
    UiShared { textures: std::sync::Arc::new(crate::flash_ui::load_shared(data_dir)), strings: std::sync::Arc::new(strings), logic: std::sync::Arc::new(crate::menu3d::UiLogic::load(data_dir)) }
}

/// The movie's `_global.timer` (logs).
pub fn movie_timer(b: &Boot) -> i32 {
    b.movie.as_ref().map_or(0, timer)
}

fn timer(m: &FlashMovie) -> i32 {
    m.player.get_global("timer").map_or(0, |v| v.to_i32())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn boot(mode: Intro) -> Boot {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let strings = mcla_formats::strtbl::StringTable::parse(&std::fs::read(std::path::Path::new(&dir).join("ui/0xa4c1e8fa")).unwrap()).unwrap();
        let shared = UiShared { textures: Arc::new(Default::default()), strings: Arc::new(strings), logic: Arc::new(crate::menu3d::UiLogic::load(&dir)) };
        let mut b = Boot::new(mode).unwrap();
        b.load(&dir, &shared);
        b
    }

    /// The movie's own clock, as recorded from the original (docs/reference/ui/boot.md): the loading music at timer
    /// 340, and - loading done from the start - the start screen at timer 692 (662 + the 30-frame transition out),
    /// 23.0 s at 30 movie frames a second. FAST starts at the game's logo (timer 622) and reaches the start screen 2.3 s later.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn legals_movie_reaches_the_start_screen_on_the_original_clock() {
        assert!(Boot::new(Intro::Skip).is_none());
        for (mode, secs) in [(Intro::Normal, 23.0), (Intro::Fast, 2.33)] {
            let mut b = boot(mode);
            let (mut t, mut music_at) = (0.0f32, None);
            while !b.title && t < 60.0 {
                b.update(FRAME, true, false);
                t += FRAME;
                if music_at.is_none() && b.music() {
                    music_at = Some(t);
                }
            }
            println!("{:?}: start screen after {:.2} s (timer {}), music from {:?}", mode.word(), t, timer(b.movie.as_ref().unwrap()), music_at);
            assert!((t - secs).abs() < 0.1, "{t}");
            assert_eq!(timer(b.movie.as_ref().unwrap()), 692);
            if mode == Intro::Normal {
                assert!((music_at.unwrap() - 339.0 / 30.0).abs() < 0.1);
            }
        }
    }
}
