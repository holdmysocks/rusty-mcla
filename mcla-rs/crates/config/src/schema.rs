//! The key table the launcher reads: one entry per settings key, with its type, default, range and whether the game
//! acts on it yet. A key is switched to `implemented: true` only once the game reads it and it was checked
//! (docs/LAUNCHER.md, "Settings contract"). `tests::schema_matches_defaults` keeps this table and `Settings::default()` in step.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    /// `table.key`
    pub key: &'static str,
    /// `bool`, `int`, `float`, `string`, or `enum`
    pub ty: &'static str,
    /// the default as written in the file (TOML syntax)
    pub default: &'static str,
    /// the allowed values or range, in words
    pub range: &'static str,
    /// the game acts on this key
    pub implemented: bool,
}

const fn k(key: &'static str, ty: &'static str, default: &'static str, range: &'static str, implemented: bool) -> KeyInfo {
    KeyInfo { key, ty, default, range, implemented }
}

static KEYS: &[KeyInfo] = &[
    k("display.window_mode", "enum", "\"windowed\"", "windowed | borderless | fullscreen", true),
    k("display.resolution", "string", "\"1600x900\"", "WxH, 640x360 .. 16384x16384 (the window size; borderless and fullscreen use the monitor)", true),
    k("display.aspect", "enum", "\"auto\"", "auto | 16:9 | 21:9 | 32:9", false),
    k("display.vsync", "bool", "true", "true | false", true),
    k("display.fps_cap", "int", "0", "0 (no cap) or 20 .. 1000", true),
    k("display.upscaler", "enum", "\"off\"", "off | bilinear | fsr", false),
    k("display.render_scale", "float", "1.0", "0.5 .. 2.0 (the game offers 1.0 1.25 1.5 1.75 2.0: the scene is drawn at window x scale and filtered down; the UI stays at window size)", true),
    k("display.aa", "enum", "\"original\"", "original | off | 2x | 4x (original = the original's own sample counts; the old msaa2 / msaa4 / fxaa are still read)", true),
    k("display.monitor", "int", "0", "0 .. 15 (0 = primary, then the others in the system's order; a missing one = primary)", true),
    k("display.fps_monitor", "enum", "\"off\"", "off | fps | frame_time (the frame-rate readout: frames per second, or that and the frame time)", true),
    k("display.bloom", "bool", "true", "true | false (true = the original's bloom)", true),
    k("display.dof", "bool", "true", "true | false (depth of field; true = the original's)", true),
    k("display.motion_blur", "bool", "true", "true | false (true = the original's motion blur)", true),
    k("display.texture_filtering", "enum", "\"original\"", "original | 16x (original = anisotropy 4 while driving, 16 in the menus; 16x = 16 everywhere)", true),
    k("game.intro", "enum", "\"normal\"", "normal | fast | skip", true),
    k("game.shadow_resolution", "enum", "\"original\"", "original | 2x | 4x (the shadow map's side)", true),
    k("game.draw_distance", "enum", "\"original\"", "original | 1.5x | 2x | 3x", true),
    k("game.detail_level", "enum", "\"original\"", "original | 1.5x | 2x | 3x (model detail distances; 3x also keeps traffic cars at LOD 0)", true),
    k("paths.data_dir", "string", "\"\"", "empty = next to the exe; a relative path is taken from the install root", true),
    k("paths.screenshots", "string", "\"\"", "empty = <install root>/screenshots", true),
    k("launcher.language", "string", "\"\"", "empty = the system language", false),
    k("launcher.check_updates", "bool", "true", "true | false", false),
];

/// Every key of schema 1. `implemented` is about the game; the launcher's own keys (`launcher.*`) are false here and
/// are the launcher's business.
pub fn schema() -> &'static [KeyInfo] {
    KEYS
}

/// The table as TOML text (`[[key]]` entries), for tools that do not link this crate.
pub fn schema_toml() -> String {
    let mut s = String::from("schema = 1\n");
    for e in KEYS {
        s += &format!("\n[[key]]\nname = {:?}\ntype = {:?}\ndefault = {:?}\nrange = {:?}\nimplemented = {}\n", e.key, e.ty, e.default, e.range, e.implemented);
    }
    s
}
