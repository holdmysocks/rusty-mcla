//! Settings > Display and game: the rows the game reads from `settings.toml` (`mcla_config`). Live: intro, window mode,
//! resolution, frame-rate cap, vsync, monitor and the frame-rate readout (the game reads them at its start; its OPTIONS > DISPLAY page writes the
//! same keys); upscaling, render scale and anti-aliasing stay greyed until the game reads them (the key table says).
//!
//! Monitor numbering is the game's (crates/game/src/hooks.rs `monitor_list`): 0 = the primary monitor, then the
//! others in the order the system lists them (Tauri and winit both ask Windows' `EnumDisplayMonitors`).

use crate::logging;
use serde::Serialize;
use std::path::PathBuf;
use tauri::WebviewWindow;

#[derive(Serialize)]
pub struct MonitorView {
    index: u32,
    /// the system's name (`\\.\DISPLAY1` on Windows, shown without the prefix)
    name: String,
    width: u32,
    height: u32,
    primary: bool,
    /// its refresh rate in Hz (0 = not known)
    hz: u32,
}

#[derive(Serialize)]
pub struct DisplayView {
    monitors: Vec<MonitorView>,
    /// `display.monitor` as the game will read it (0 when the file is missing or the key is bad)
    monitor: u32,
    /// the key table says the game acts on the key
    implemented: bool,
    /// the other live rows, as the game will read them
    intro: &'static str,
    window_mode: &'static str,
    resolution: String,
    /// the resolutions offered: common sizes up to the chosen monitor's, its own, and the one in the file
    resolutions: Vec<String>,
    vsync: bool,
    fps_cap: u32,
    /// the frame-rate caps offered (0 = none); the file's own value among them
    fps_caps: Vec<u32>,
    /// the frame-rate readout: off | fps | frame_time
    fps_monitor: &'static str,
    /// anti-aliasing: original | off | 2x | 4x, and the effect toggles (true = the original's)
    aa: &'static str,
    texture_filtering: &'static str,
    render_scale: f32,
    shadow_resolution: &'static str,
    draw_distance: &'static str,
    detail_level: &'static str,
    bloom: bool,
    dof: bool,
    motion_blur: bool,
    /// the keys of these rows the key table marks as read by the game
    live_keys: Vec<&'static str>,
    settings_path: String,
    /// `ok`, `missing` (defaults), `broken` (not TOML: the launcher will not overwrite it)
    file: &'static str,
}

fn settings_path() -> PathBuf {
    mcla_config::paths::settings_path(None)
}

fn short_name(n: Option<&String>) -> String {
    let n = n.map(|s| s.as_str()).unwrap_or("");
    n.trim_start_matches(r"\\.\").to_string()
}

/// A monitor's current refresh rate in Hz from its device name (`\\.\DISPLAY1`); 0 when Windows does not say.
#[cfg(windows)]
fn refresh_hz(device: &str) -> u32 {
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};
    let name: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: a zeroed DEVMODEW with dmSize set is what the call expects; the name is NUL-terminated.
    unsafe {
        let mut dm: DEVMODEW = std::mem::zeroed();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(name.as_ptr(), ENUM_CURRENT_SETTINGS, &mut dm) != 0 && dm.dmDisplayFrequency > 1 {
            dm.dmDisplayFrequency
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
fn refresh_hz(_device: &str) -> u32 {
    0
}

/// The monitors in the game's order: the primary first, then the rest as listed.
pub fn monitors(window: &WebviewWindow) -> Vec<MonitorView> {
    let all = window.available_monitors().unwrap_or_default();
    let primary = window.primary_monitor().ok().flatten();
    let same = |a: &tauri::Monitor, b: &tauri::Monitor| a.name() == b.name() && a.position() == b.position() && a.size() == b.size();
    let pi = primary.as_ref().and_then(|p| all.iter().position(|m| same(m, p))).unwrap_or(0);
    let mut order: Vec<usize> = vec![pi];
    order.extend((0..all.len()).filter(|i| *i != pi));
    order
        .into_iter()
        .filter(|i| *i < all.len())
        .enumerate()
        .map(|(n, i)| {
            let m = &all[i];
            MonitorView { index: n as u32, name: short_name(m.name()), width: m.size().width, height: m.size().height, primary: n == 0, hz: m.name().map_or(0, |s| refresh_hz(s)) }
        })
        .collect()
}

pub fn view(window: &WebviewWindow) -> DisplayView {
    let path = settings_path();
    let l = mcla_config::Settings::load(&path);
    let file = match l.status {
        mcla_config::Status::Ok => "ok",
        mcla_config::Status::Missing => "missing",
        mcla_config::Status::Broken => "broken",
    };
    let d = &l.settings.display;
    let mons = monitors(window);
    let mon = mons.iter().find(|m| m.index == d.monitor).or(mons.first());
    let (mw, mh) = mon.map_or((u32::MAX, u32::MAX), |m| (m.width, m.height));
    let mut res: Vec<(u32, u32)> = COMMON_RESOLUTIONS.iter().copied().filter(|&(w, h)| w <= mw && h <= mh).collect();
    if let Some(m) = mon {
        res.push((m.width, m.height));
    }
    res.push((d.resolution.w, d.resolution.h));
    res.sort();
    res.dedup();
    let mut caps = FPS_CAPS.to_vec();
    // (the file's own cap, and the chosen monitor's refresh rate - the game's DISPLAY page offers it too, GitHub #11)
    for c in [d.fps_cap, mon.map_or(0, |m| m.hz)] {
        if c > 0 && !caps.contains(&c) {
            caps.push(c);
        }
    }
    caps.sort();
    let live_keys = mcla_config::schema().iter().filter(|k| k.implemented && LIVE_KEYS.contains(&k.key)).map(|k| k.key).collect();
    DisplayView {
        monitors: mons,
        monitor: d.monitor,
        implemented: mcla_config::schema().iter().any(|k| k.key == "display.monitor" && k.implemented),
        intro: l.settings.game.intro.word(),
        window_mode: d.window_mode.word(),
        resolution: d.resolution.text(),
        resolutions: res.into_iter().map(|(w, h)| format!("{w}x{h}")).collect(),
        vsync: d.vsync,
        fps_cap: d.fps_cap,
        fps_caps: caps,
        fps_monitor: d.fps_monitor.word(),
        aa: d.aa.word(),
        texture_filtering: d.texture_filtering.word(),
        render_scale: d.render_scale,
        shadow_resolution: l.settings.game.shadow_resolution.word(),
        draw_distance: l.settings.game.draw_distance.word(),
        detail_level: l.settings.game.detail_level.word(),
        bloom: d.bloom,
        dof: d.dof,
        motion_blur: d.motion_blur,
        live_keys,
        settings_path: path.to_string_lossy().into_owned(),
        file,
    }
}

/// The rows `set` writes (besides the monitor's own command).
const LIVE_KEYS: [&str; 15] = ["game.intro", "display.window_mode", "display.resolution", "display.vsync", "display.fps_cap", "display.fps_monitor", "display.aa", "display.bloom", "display.dof", "display.motion_blur", "display.texture_filtering", "display.render_scale", "game.shadow_resolution", "game.draw_distance", "game.detail_level"];
/// Window sizes offered beside the monitor's own (those that fit on it).
const COMMON_RESOLUTIONS: [(u32, u32); 9] = [(1280, 720), (1366, 768), (1600, 900), (1920, 1080), (2560, 1080), (2560, 1440), (3440, 1440), (3840, 1600), (3840, 2160)];
/// The caps the game's DISPLAY page steps through (crates/game/src/hooks.rs `FPS_CAPS`).
const FPS_CAPS: [u32; 7] = [0, 30, 60, 120, 144, 165, 240];

/// Writes one of the live rows' keys (only that key; atomic) after checking the value as the game's reader would. A
/// broken file is not overwritten.
pub fn set(window: &WebviewWindow, key: &str, value: serde_json::Value) -> Result<DisplayView, String> {
    use mcla_config::toml::Value;
    let text = || value.as_str().map(str::to_string).ok_or_else(|| "bad_value".to_string());
    let v = match key {
        "game.intro" => Value::String(mcla_config::Intro::parse(&text()?).ok_or("bad_value")?.word().into()),
        "display.window_mode" => Value::String(mcla_config::WindowMode::parse(&text()?).ok_or("bad_value")?.word().into()),
        "display.resolution" => Value::String(mcla_config::Resolution::parse(&text()?).ok_or("bad_value")?.text()),
        "display.vsync" => Value::Boolean(value.as_bool().ok_or("bad_value")?),
        "display.fps_monitor" => Value::String(mcla_config::FpsMonitor::parse(&text()?).ok_or("bad_value")?.word().into()),
        "display.aa" => Value::String(mcla_config::Aa::parse_any(&text()?).ok_or("bad_value")?.word().into()),
        "display.bloom" | "display.dof" | "display.motion_blur" => Value::Boolean(value.as_bool().ok_or("bad_value")?),
        "display.texture_filtering" => Value::String(mcla_config::TexFilter::parse(&text()?).ok_or("bad_value")?.word().into()),
        "display.render_scale" => {
            let f = value.as_f64().ok_or("bad_value")?;
            if !(0.5..=2.0).contains(&f) {
                return Err("out_of_range".into());
            }
            Value::Float(f)
        }
        "game.shadow_resolution" => Value::String(mcla_config::ShadowRes::parse(&text()?).ok_or("bad_value")?.word().into()),
        "game.draw_distance" | "game.detail_level" => Value::String(mcla_config::Scale::parse(&text()?).ok_or("bad_value")?.word().into()),
        "display.fps_cap" => {
            let n = value.as_u64().ok_or("bad_value")?;
            if n != 0 && !(20..=1000).contains(&n) {
                return Err("out_of_range".into());
            }
            Value::Integer(n as i64)
        }
        _ => return Err("unknown_key".into()),
    };
    let (table, name) = key.split_once('.').ok_or("unknown_key")?;
    let path = settings_path();
    let shown = v.to_string();
    mcla_config::set_key(&path, table, name, v).map_err(|e| {
        logging::error(&format!("settings.toml: {key} not written: {e}"));
        if e.kind() == std::io::ErrorKind::InvalidData { "broken".to_string() } else { "io".to_string() }
    })?;
    logging::info(&format!("settings.toml: {key} = {shown} ({})", path.display()));
    Ok(view(window))
}

/// Writes `display.monitor` (only that key; atomic). A broken file is not overwritten.
pub fn set_monitor(window: &WebviewWindow, index: u32) -> Result<DisplayView, String> {
    if index > 15 {
        return Err("out_of_range".into());
    }
    let path = settings_path();
    mcla_config::set_key(&path, "display", "monitor", mcla_config::toml::Value::Integer(index as i64)).map_err(|e| {
        logging::error(&format!("settings.toml: display.monitor not written: {e}"));
        if e.kind() == std::io::ErrorKind::InvalidData { "broken".to_string() } else { "io".to_string() }
    })?;
    logging::info(&format!("settings.toml: display.monitor = {index} ({})", path.display()));
    Ok(view(window))
}
