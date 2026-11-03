//! The night fill light by place, for the renderer (work/map/render_rule.md section 1; the data
//! layer is `mcla_formats::rage::aomap`). The original looks the map up on the CPU, nearest
//! cell, and hands the colour on per object:
//!   props (the prop list's plain instances): trunc(map / 4), uploaded as gPropNFLColor.rgb =
//!     c / 255 x s with s = 1.5 x m_NFLFade, and added as c99.rgb x 1.25 x s in the pixel program;
//!   plants: the map's colour is baked into each instance record (`FoliageInstance::fill`);
//!   traffic cars, pedestrians, drivers: gLightAmbient + map / 255 x 0.25 x m_NFLFade.
//! `MCLA_NO_FILL_MAP=1` switches all of it off (every lookup answers "no value": the shaders
//! then use what they used before - one constant colour for plants, nothing for the others).

use mcla_formats::rage::aomap::AoMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

static MAP: OnceLock<Option<AoMap>> = OnceLock::new();
/// m_NFLFade of the frame (f32 bits)
static FADE: AtomicU32 = AtomicU32::new(0);

fn off() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    *OFF.get_or_init(|| std::env::var("MCLA_NO_FILL_MAP").is_ok())
}

/// Load the map (`<data>/tune/city/sc`). Without this call - or without the files - nothing has a value.
pub fn init(data_dir: &str) {
    let m = MAP.get_or_init(|| if off() { None } else { AoMap::load(&std::path::Path::new(data_dir).join("tune/city/sc")) });
    match m {
        Some(m) => log::info!("night fill map: {} x {} cells of {} m", m.nx, m.nz, m.cell),
        None => log::info!("night fill map: none ({})", if off() { "MCLA_NO_FILL_MAP" } else { "files missing" }),
    }
}

/// m_NFLFade of this frame (0 by day), set once a frame before the objects' uniforms are written.
pub fn set_fade(nfl: f32) {
    FADE.store(nfl.max(0.0).to_bits(), Ordering::Relaxed);
}

/// A prop's instance colour: trunc(map / 4) as the original stores it at instance +148, alpha
/// 255 = "has a value". All zero without the map (and black outside it, as the original).
pub fn prop(x: f32, z: f32) -> [u8; 4] {
    match MAP.get().and_then(|m| m.as_ref()) {
        Some(m) => {
            let c = m.color_at(x, z);
            [c[0] / 4, c[1] / 4, c[2] / 4, 255]
        }
        None => [0; 4],
    }
}

/// A plant's instance colour: the map's colour baked into its record, as it is.
pub fn plant(fill: [u8; 3]) -> [u8; 4] {
    if off() { [0; 4] } else { [fill[0], fill[1], fill[2], 255] }
}

/// The same for a figure (pedestrian, driver; F-24398), as the colour its vertices carry:
/// four times the sum as bytes (the sum is at most 0.25), alpha 255. Zero by day and without the map.
pub fn figure_now(x: f32, z: f32) -> [u8; 4] {
    let c = body_now(x, z);
    let b = |v: f32| (v * 4.0 * 255.0).round().clamp(0.0, 255.0) as u8;
    [b(c[0]), b(c[1]), b(c[2]), 255]
}

/// The player's car: the same sum, and in w the factor the whole ambient (gLightAmbient + the
/// sum) is shown with on it at night - 0.4, RECORDED (car_n: its gLightAmbient is 0.4 x (base +
/// map / 4) to the digit), its source in the code not found. w = 0: no factor (by day, no map).
pub fn player_now(x: f32, z: f32) -> [f32; 4] {
    let n = f32::from_bits(FADE.load(Ordering::Relaxed));
    if n <= 0.0 || MAP.get().and_then(|m| m.as_ref()).is_none() {
        return [0.0; 4];
    }
    let c = body_now(x, z);
    [c[0], c[1], c[2], 0.4]
}

/// The map's colour at this place as the ambient cube's faces take it at night (F-35455 from the
/// characters' night path: map / 255 x 0.25, no fade - the downward face x 0.2 besides), zero without the map.
pub fn cube_at(x: f32, z: f32) -> glam::Vec3 {
    match MAP.get().and_then(|m| m.as_ref()) {
        Some(m) if !off() => m.body_fill(x, z),
        _ => glam::Vec3::ZERO,
    }
}

/// What a traffic car (pedestrian, driver) at this place adds to its ambient now:
/// map / 255 x 0.25 x m_NFLFade - zero by day, off the map and without the map.
pub fn body_now(x: f32, z: f32) -> [f32; 4] {
    let n = f32::from_bits(FADE.load(Ordering::Relaxed));
    if n <= 0.0 {
        return [0.0; 4];
    }
    match MAP.get().and_then(|m| m.as_ref()) {
        Some(m) => {
            let c = m.body_fill(x, z) * n;
            [c.x, c.y, c.z, 0.0]
        }
        None => [0.0; 4],
    }
}
