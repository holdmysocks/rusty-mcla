//! The `settings.toml` model shared by the game and the launcher (docs/LAUNCHER.md, "Settings contract").
//!
//! Rules: the file is versioned (`schema = 1`); a missing or broken file means defaults plus a warning, never a crash;
//! a key that is wrong (bad type, out of range, unknown enum word) falls back to its own default and costs one warning,
//! the rest of the file still counts; unknown keys and tables are kept and written back untouched. The launcher writes
//! the file, the game reads it, a command-line flag overrides it. [`schema`] lists every key with type, default, range and
//! whether the game implements it, so the launcher can grey out the ones that do nothing yet.
//!
//! No game dependencies, no platform code except the small helpers in [`paths`]: it builds with the GNU and the MSVC toolchain.

pub mod keybinds;
pub mod paths;
mod schema;

pub use schema::{schema, schema_toml, KeyInfo};
/// The TOML crate this one uses (for [`set_key`]'s value).
pub use toml;

use std::path::{Path, PathBuf};
use toml::{Table, Value};

/// The only schema this crate writes. A file with a higher number is still read key by key.
pub const SCHEMA: i64 = 1;

macro_rules! words {
    ($name:ident { $($var:ident = $word:expr),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($var),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];
            pub const WORDS: &'static [&'static str] = &[$($word),+];
            pub fn word(self) -> &'static str { match self { $($name::$var => $word),+ } }
            pub fn parse(s: &str) -> Option<$name> {
                let s = s.trim().to_ascii_lowercase();
                $(if s == $word { return Some($name::$var); })+
                None
            }
        }
    };
}

words!(WindowMode { Windowed = "windowed", Borderless = "borderless", Fullscreen = "fullscreen" });
impl WindowMode {
    /// The game's Alt+Enter: windowed -> borderless -> fullscreen -> windowed.
    pub fn next(self) -> WindowMode {
        match self {
            WindowMode::Windowed => WindowMode::Borderless,
            WindowMode::Borderless => WindowMode::Fullscreen,
            WindowMode::Fullscreen => WindowMode::Windowed,
        }
    }
}

words!(Intro { Normal = "normal", Fast = "fast", Skip = "skip" });
words!(Aspect { Auto = "auto", Wide21x9 = "21:9", Wide32x9 = "32:9", Fit16x9 = "16:9" });
words!(Upscaler { Off = "off", Bilinear = "bilinear", Fsr = "fsr" });
// (anti-aliasing: "original" = the original's own sample counts; the old words msaa2 / msaa4 / fxaa are still read)
words!(Aa { Original = "original", Off = "off", X2 = "2x", X4 = "4x" });
impl Aa {
    /// `parse` plus the words of the first schema (msaa2 -> 2x, msaa4 -> 4x, fxaa -> original).
    pub fn parse_any(s: &str) -> Option<Aa> {
        Aa::parse(s).or_else(|| match s.trim().to_ascii_lowercase().as_str() {
            "msaa2" => Some(Aa::X2),
            "msaa4" => Some(Aa::X4),
            "fxaa" => Some(Aa::Original),
            _ => None,
        })
    }
}
// (the graphics API: auto = Vulkan where the driver has it, else DX12; a port-only switch, read at start - issue #12)
words!(Backend { Auto = "auto", Vulkan = "vulkan", Dx12 = "dx12" });
// (texture filtering: the original's anisotropy (4 driving, 16 in the menus) or 16x everywhere)
words!(TexFilter { Original = "original", X16 = "16x" });
// (shadow map size: the original's, or 2x / 4x its side)
words!(ShadowRes { Original = "original", X2 = "2x", X4 = "4x" });
// (draw distance and detail level: the original's, or a multiple)
words!(Scale { Original = "original", X1_5 = "1.5x", X2 = "2x", X3 = "3x" });
impl Scale {
    pub fn factor(self) -> f32 {
        match self {
            Scale::Original => 1.0,
            Scale::X1_5 => 1.5,
            Scale::X2 => 2.0,
            Scale::X3 => 3.0,
        }
    }
}
impl ShadowRes {
    pub fn factor(self) -> u32 {
        match self {
            ShadowRes::Original => 1,
            ShadowRes::X2 => 2,
            ShadowRes::X4 => 4,
        }
    }
}
// (the port's frame-rate readout, DECISIONS.md D25: off, frames per second, or frames per second and frame time)
words!(FpsMonitor { Off = "off", Fps = "fps", FrameTime = "frame_time" });

/// A window size in pixels. Written as `"1600x900"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub w: u32,
    pub h: u32,
}

impl Resolution {
    pub const MIN: (u32, u32) = (640, 360);
    pub const MAX: (u32, u32) = (16384, 16384);

    pub fn parse(s: &str) -> Option<Resolution> {
        let (a, b) = s.trim().split_once(['x', 'X'])?;
        let (w, h) = (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?);
        let ok = w >= Self::MIN.0 && h >= Self::MIN.1 && w <= Self::MAX.0 && h <= Self::MAX.1;
        ok.then_some(Resolution { w, h })
    }

    pub fn text(self) -> String {
        format!("{}x{}", self.w, self.h)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub window_mode: WindowMode,
    pub resolution: Resolution,
    pub aspect: Aspect,
    pub vsync: bool,
    /// frames per second, 0 = no cap
    pub fps_cap: u32,
    pub upscaler: Upscaler,
    pub render_scale: f32,
    pub aa: Aa,
    /// index of the monitor, 0 = the primary one
    pub monitor: u32,
    /// the frame-rate readout (a port-only addition)
    pub fps_monitor: FpsMonitor,
    /// bloom, depth of field, motion blur: ON = the original's
    pub bloom: bool,
    pub dof: bool,
    pub motion_blur: bool,
    pub texture_filtering: TexFilter,
    /// the graphics API (read at start)
    pub backend: Backend,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameSettings {
    pub intro: Intro,
    pub shadow_resolution: ShadowRes,
    pub draw_distance: Scale,
    pub detail_level: Scale,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paths {
    /// empty = find it next to the exe (see [`paths::resolve_data_dir`]); a relative path is taken from the install root
    pub data_dir: String,
    /// empty = `<install root>/screenshots`
    pub screenshots: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LauncherSettings {
    /// empty = the system language
    pub language: String,
    pub check_updates: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub schema: i64,
    pub display: Display,
    pub game: GameSettings,
    pub paths: Paths,
    pub launcher: LauncherSettings,
    /// the file as it was read: unknown keys and tables travel through [`Settings::to_table`] untouched
    extra: Table,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            schema: SCHEMA,
            display: Display {
                window_mode: WindowMode::Windowed,
                resolution: Resolution { w: 1600, h: 900 },
                aspect: Aspect::Auto,
                vsync: true,
                fps_cap: 0,
                upscaler: Upscaler::Off,
                render_scale: 1.0,
                aa: Aa::Original,
                monitor: 0,
                fps_monitor: FpsMonitor::Off,
                bloom: true,
                dof: true,
                motion_blur: true,
                texture_filtering: TexFilter::Original,
                backend: Backend::Auto,
            },
            game: GameSettings { intro: Intro::Normal, shadow_resolution: ShadowRes::Original, draw_distance: Scale::Original, detail_level: Scale::Original },
            paths: Paths { data_dir: String::new(), screenshots: String::new() },
            launcher: LauncherSettings { language: String::new(), check_updates: true },
            extra: Table::new(),
        }
    }
}

/// What happened to the file on [`Settings::load`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// read; individual bad keys (if any) are in `warnings`
    Ok,
    /// no such file: all defaults
    Missing,
    /// not readable or not TOML: all defaults
    Broken,
}

pub struct Loaded {
    pub settings: Settings,
    pub status: Status,
    /// human-readable lines for the log (they are also sent to the `log` crate)
    pub warnings: Vec<String>,
}

impl Settings {
    /// Read a file. Never fails: see [`Status`].
    pub fn load(path: &Path) -> Loaded {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded { settings: Settings::default(), status: Status::Missing, warnings: Vec::new() },
            Err(e) => return Self::broken(path, format!("cannot read: {e}")),
        };
        // (a UTF-8 byte order mark from a Windows editor)
        match text.trim_start_matches('\u{feff}').parse::<Table>() {
            Ok(t) => {
                let (settings, warnings) = Settings::from_table(&t);
                for w in &warnings {
                    log::warn!("settings {}: {}", path.display(), w);
                }
                Loaded { settings, status: Status::Ok, warnings }
            }
            Err(e) => Self::broken(path, format!("not valid TOML: {}", e.message())),
        }
    }

    fn broken(path: &Path, why: String) -> Loaded {
        let w = format!("{why}; using the defaults");
        log::warn!("settings {}: {}", path.display(), w);
        Loaded { settings: Settings::default(), status: Status::Broken, warnings: vec![w] }
    }

    /// Read from parsed TOML. Every key is checked on its own; a bad one keeps its default and adds a warning.
    pub fn from_table(t: &Table) -> (Settings, Vec<String>) {
        let mut s = Settings { extra: t.clone(), ..Settings::default() };
        let mut warn = Vec::new();
        match t.get("schema") {
            None => {}
            Some(Value::Integer(n)) => {
                s.schema = *n;
                if *n > SCHEMA {
                    warn.push(format!("schema {n} is newer than this build knows ({SCHEMA}); unknown keys are kept and ignored"));
                }
            }
            Some(_) => warn.push("schema: not an integer".into()),
        }
        let d = Settings::default();
        let mut r = Reader { warn: &mut warn };
        let dt = sub(t, "display");
        s.display.window_mode = r.word(dt, "display.window_mode", "window_mode", d.display.window_mode, WindowMode::parse, WindowMode::WORDS);
        s.display.resolution = r.str_with(dt, "display.resolution", "resolution", d.display.resolution, Resolution::parse, "a size like \"1600x900\" (640x360 up to 16384x16384)");
        s.display.aspect = r.word(dt, "display.aspect", "aspect", d.display.aspect, Aspect::parse, Aspect::WORDS);
        s.display.vsync = r.boolean(dt, "display.vsync", "vsync", d.display.vsync);
        s.display.fps_cap = r.uint(dt, "display.fps_cap", "fps_cap", d.display.fps_cap, |n| n == 0 || (20..=1000).contains(&n), "0 (no cap) or 20..1000");
        s.display.upscaler = r.word(dt, "display.upscaler", "upscaler", d.display.upscaler, Upscaler::parse, Upscaler::WORDS);
        s.display.render_scale = r.float(dt, "display.render_scale", "render_scale", d.display.render_scale, 0.5, 2.0);
        s.display.aa = r.word(dt, "display.aa", "aa", d.display.aa, Aa::parse_any, Aa::WORDS);
        s.display.monitor = r.uint(dt, "display.monitor", "monitor", d.display.monitor, |n| n < 16, "0..15");
        s.display.fps_monitor = r.word(dt, "display.fps_monitor", "fps_monitor", d.display.fps_monitor, FpsMonitor::parse, FpsMonitor::WORDS);
        s.display.texture_filtering = r.word(dt, "display.texture_filtering", "texture_filtering", d.display.texture_filtering, TexFilter::parse, TexFilter::WORDS);
        s.display.backend = r.word(dt, "display.backend", "backend", d.display.backend, Backend::parse, Backend::WORDS);
        s.display.bloom = r.boolean(dt, "display.bloom", "bloom", d.display.bloom);
        s.display.dof = r.boolean(dt, "display.dof", "dof", d.display.dof);
        s.display.motion_blur = r.boolean(dt, "display.motion_blur", "motion_blur", d.display.motion_blur);
        let gt = sub(t, "game");
        // ("off", the in-game INTRO row's name for it, is read as "skip"; "skip" is what is written)
        s.game.intro = r.word(gt, "game.intro", "intro", d.game.intro, |v| Intro::parse(v).or_else(|| v.trim().eq_ignore_ascii_case("off").then_some(Intro::Skip)), Intro::WORDS);
        s.game.shadow_resolution = r.word(gt, "game.shadow_resolution", "shadow_resolution", d.game.shadow_resolution, ShadowRes::parse, ShadowRes::WORDS);
        s.game.draw_distance = r.word(gt, "game.draw_distance", "draw_distance", d.game.draw_distance, Scale::parse, Scale::WORDS);
        s.game.detail_level = r.word(gt, "game.detail_level", "detail_level", d.game.detail_level, Scale::parse, Scale::WORDS);
        let pt = sub(t, "paths");
        s.paths.data_dir = r.string(pt, "paths.data_dir", "data_dir", &d.paths.data_dir);
        s.paths.screenshots = r.string(pt, "paths.screenshots", "screenshots", &d.paths.screenshots);
        let lt = sub(t, "launcher");
        s.launcher.language = r.string(lt, "launcher.language", "language", &d.launcher.language);
        s.launcher.check_updates = r.boolean(lt, "launcher.check_updates", "check_updates", d.launcher.check_updates);
        for name in ["display", "game", "paths", "launcher"] {
            if matches!(t.get(name), Some(v) if !v.is_table()) {
                warn.push(format!("{name}: not a table; using the defaults"));
            }
        }
        (s, warn)
    }

    /// The file's content: the known keys from this struct over everything else that was in the file.
    pub fn to_table(&self) -> Table {
        let mut t = self.extra.clone();
        t.insert("schema".into(), Value::Integer(self.schema.max(SCHEMA)));
        fn put(t: &mut Table, table: &str, key: &str, v: Value) {
            let e = t.entry(table.to_string()).or_insert_with(|| Value::Table(Table::new()));
            if !e.is_table() {
                *e = Value::Table(Table::new());
            }
            e.as_table_mut().unwrap().insert(key.into(), v);
        }
        let (d, g, p, l) = (&self.display, &self.game, &self.paths, &self.launcher);
        put(&mut t, "display", "window_mode", d.window_mode.word().into());
        put(&mut t, "display", "resolution", d.resolution.text().into());
        put(&mut t, "display", "aspect", d.aspect.word().into());
        put(&mut t, "display", "vsync", d.vsync.into());
        put(&mut t, "display", "fps_cap", Value::Integer(d.fps_cap as i64));
        put(&mut t, "display", "upscaler", d.upscaler.word().into());
        // (f32 -> f64 directly would write 0.800000011920929)
        put(&mut t, "display", "render_scale", Value::Float(format!("{}", d.render_scale).parse::<f64>().unwrap_or(1.0)));
        put(&mut t, "display", "aa", d.aa.word().into());
        put(&mut t, "display", "monitor", Value::Integer(d.monitor as i64));
        put(&mut t, "display", "fps_monitor", d.fps_monitor.word().into());
        put(&mut t, "display", "texture_filtering", d.texture_filtering.word().into());
        put(&mut t, "display", "backend", d.backend.word().into());
        put(&mut t, "display", "bloom", d.bloom.into());
        put(&mut t, "display", "dof", d.dof.into());
        put(&mut t, "display", "motion_blur", d.motion_blur.into());
        put(&mut t, "game", "intro", g.intro.word().into());
        put(&mut t, "game", "shadow_resolution", g.shadow_resolution.word().into());
        put(&mut t, "game", "draw_distance", g.draw_distance.word().into());
        put(&mut t, "game", "detail_level", g.detail_level.word().into());
        put(&mut t, "paths", "data_dir", p.data_dir.clone().into());
        put(&mut t, "paths", "screenshots", p.screenshots.clone().into());
        put(&mut t, "launcher", "language", l.language.clone().into());
        put(&mut t, "launcher", "check_updates", l.check_updates.into());
        t
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(&self.to_table()).unwrap_or_default()
    }

    /// Write the file atomically: a temp file beside it, flushed, then renamed over it. Creates the folder.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        atomic_write(path, self.to_toml().as_bytes())
    }

    /// One-line summary for the top of a log.
    pub fn summary(&self) -> String {
        let (d, g) = (&self.display, &self.game);
        format!(
            "window_mode={} resolution={} vsync={} fps_cap={} intro={} (aspect={} upscaler={} render_scale={} aa={} monitor={} backend={})",
            d.window_mode.word(), d.resolution.text(), d.vsync, d.fps_cap, g.intro.word(), d.aspect.word(), d.upscaler.word(), d.render_scale, d.aa.word(), d.monitor, d.backend.word()
        )
    }
}

/// Write a file atomically: `<name>.tmp` beside it, flushed, then renamed over it. Creates the folder.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp: PathBuf = path.to_path_buf();
    let mut name = tmp.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "file".into());
    name.push(".tmp");
    tmp.set_file_name(name);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e
    })
}

/// Change one key of the settings file and nothing else (the game's Alt+Enter, the launcher's single rows): the file
/// is read as plain TOML, `[table] key = value` is set, everything else is written back as it was read (other keys,
/// unknown keys and tables, values this build would refuse; comments and layout are not kept - the TOML reader has no
/// lossless round trip). A missing file is created with `schema = 1` and that key. A file that is not valid TOML is
/// **not** overwritten (the player's text stays): `InvalidData`. Atomic write.
pub fn set_key(path: &Path, table: &str, key: &str, value: Value) -> std::io::Result<()> {
    let mut t = match std::fs::read_to_string(path) {
        Ok(text) => text
            .trim_start_matches('\u{feff}')
            .parse::<Table>()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{} is not valid TOML ({}); not changed", path.display(), e.message())))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut t = Table::new();
            t.insert("schema".into(), Value::Integer(SCHEMA));
            t
        }
        Err(e) => return Err(e),
    };
    let e = t.entry(table.to_string()).or_insert_with(|| Value::Table(Table::new()));
    if !e.is_table() {
        *e = Value::Table(Table::new());
    }
    e.as_table_mut().expect("a table").insert(key.to_string(), value);
    let text = toml::to_string(&t).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    atomic_write(path, text.as_bytes())
}

fn sub<'a>(t: &'a Table, name: &str) -> Option<&'a Table> {
    t.get(name).and_then(Value::as_table)
}

struct Reader<'a> {
    warn: &'a mut Vec<String>,
}

impl Reader<'_> {
    fn bad(&mut self, full: &str, got: &Value, want: &str) {
        self.warn.push(format!("{full} = {got}: expected {want}; using the default"));
    }

    fn word<T: Copy>(&mut self, t: Option<&Table>, full: &str, key: &str, default: T, parse: fn(&str) -> Option<T>, words: &[&str]) -> T {
        self.str_with(t, full, key, default, parse, &format!("one of {}", words.join(", ")))
    }

    fn str_with<T: Copy>(&mut self, t: Option<&Table>, full: &str, key: &str, default: T, parse: fn(&str) -> Option<T>, want: &str) -> T {
        match t.and_then(|t| t.get(key)) {
            None => default,
            Some(Value::String(s)) => parse(s).unwrap_or_else(|| {
                self.bad(full, &Value::String(s.clone()), want);
                default
            }),
            Some(v) => {
                self.bad(full, v, want);
                default
            }
        }
    }

    fn boolean(&mut self, t: Option<&Table>, full: &str, key: &str, default: bool) -> bool {
        match t.and_then(|t| t.get(key)) {
            None => default,
            Some(Value::Boolean(b)) => *b,
            Some(v) => {
                self.bad(full, v, "true or false");
                default
            }
        }
    }

    fn uint(&mut self, t: Option<&Table>, full: &str, key: &str, default: u32, ok: fn(u32) -> bool, want: &str) -> u32 {
        match t.and_then(|t| t.get(key)) {
            None => default,
            Some(Value::Integer(n)) if u32::try_from(*n).map_or(false, ok) => *n as u32,
            Some(v) => {
                self.bad(full, v, want);
                default
            }
        }
    }

    fn float(&mut self, t: Option<&Table>, full: &str, key: &str, default: f32, lo: f32, hi: f32) -> f32 {
        let v = match t.and_then(|t| t.get(key)) {
            None => return default,
            Some(Value::Float(f)) => *f as f32,
            Some(Value::Integer(n)) => *n as f32,
            Some(v) => {
                self.bad(full, v, &format!("a number {lo}..{hi}"));
                return default;
            }
        };
        if v.is_finite() && v >= lo && v <= hi {
            v
        } else {
            self.bad(full, &Value::Float(v as f64), &format!("a number {lo}..{hi}"));
            default
        }
    }

    fn string(&mut self, t: Option<&Table>, full: &str, key: &str, default: &str) -> String {
        match t.and_then(|t| t.get(key)) {
            None => default.to_string(),
            Some(Value::String(s)) => s.clone(),
            Some(v) => {
                self.bad(full, v, "a string");
                default.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests;
