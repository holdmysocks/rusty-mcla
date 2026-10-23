//! OPTIONS > DISPLAY > ANTI-ALIASING (settings.toml `display.aa`; DECISIONS.md): ORIGINAL (the default) is the
//! original's own sample counts (UI.md "UI edge smoothing"): the scene 2x while driving and 4x while a menu
//! holds the renderer's menu flag (+373) or the garage flag (+393) is set, the UI 1x while driving and 4x in the
//! menus. OFF / 2X / 4X put that count on the scene and the menus' UI pass; the UI while driving stays 1x and the
//! small UI targets (960 x 640 self pictures 2x, minimap / overhead map / phone / icons 4x) keep the original's.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Original,
    Off,
    X2,
    X4,
}

static MODE: AtomicU8 = AtomicU8::new(0);

/// Set from the settings (at start and when the DISPLAY row changes).
pub fn set(m: Mode) {
    MODE.store(m as u8, Ordering::Relaxed);
}

pub fn get() -> Mode {
    match MODE.load(Ordering::Relaxed) {
        1 => Mode::Off,
        2 => Mode::X2,
        3 => Mode::X4,
        _ => Mode::Original,
    }
}

/// The scene's sample count: `menu` = the original's menu flag (+373) or garage flag (+393) would be set.
pub fn scene_samples(menu: bool) -> u32 {
    match get() {
        Mode::Original => if menu { 4 } else { 2 },
        Mode::Off => 1,
        Mode::X2 => 2,
        Mode::X4 => 4,
    }
}

/// The UI pass's sample count: `menu` = the original's menu flag (+373); while driving always 1.
pub fn ui_samples(menu: bool) -> u32 {
    if !menu {
        return 1;
    }
    match get() {
        Mode::Original | Mode::X4 => 4,
        Mode::Off => 1,
        Mode::X2 => 2,
    }
}
