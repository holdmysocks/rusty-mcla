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
    k("display.render_scale", "float", "1.0", "0.5 .. 2.0", false),
    k("display.aa", "enum", "\"off\"", "off | fxaa | msaa2 | msaa4", false),
    k("display.monitor", "int", "0", "0 .. 15 (0 = primary, then the others in the system's order; a missing one = primary)", true),
    k("display.fps_monitor", "enum", "\"off\"", "off | fps | frame_time (the frame-rate readout: frames per second, or that and the frame time)", true),
    k("game.intro", "enum", "\"normal\"", "normal | fast | skip", true),
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
