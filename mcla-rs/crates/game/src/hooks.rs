//! Port-only hooks for the launcher (docs/LAUNCHER.md, "Changes needed in the game"). Nothing here touches gameplay:
//! the data folder, `settings.toml` (display, intro), the rolling log and the crash report, the screenshot key.
//!
//! - Settings: `mcla_config::Settings`, read once by [`init`] from `--settings <file>`, `MCLA_SETTINGS` or
//!   `%LOCALAPPDATA%\RustyMCLA\settings.toml`. Command-line flags win over the file. A headless `--screenshot` run
//!   ignores the user's file (so captures stay repeatable) unless a settings file is named explicitly.
//! - Log: every record goes to stderr as before and, for a normal run, also to `logs/game-<yyyymmdd-hhmmss>.log` (the
//!   newest 10 are kept). Headless `--screenshot` runs and the developer self-tests write no log file (stderr only).
//! - Crash: a panic writes `logs/crash-<time>.txt` (message, place, backtrace, adapter, settings, the last 50 log lines);
//!   a panic on the main thread then ends the process with exit code 101. A worker thread's panic is reported the same
//!   way but the game goes on as it always did. A clean exit is 0.
//! - F12: `App::screenshot_key` saves the next frame as a PNG in the screenshots folder.

use crate::App;
use mcla_config::paths::{self, DataDir, DataInputs, DataSource};
use mcla_config::{FpsMonitor, Intro, Settings, Status, WindowMode};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How many game logs and crash reports are kept.
const KEEP: usize = 10;
/// How many log lines a crash report carries.
const TAIL_LINES: usize = 50;
/// The developer's old extraction folder, used only when nothing next to the exe exists.
/// TODO-remove-before-release (docs/LAUNCHER.md, open list): a player's machine has no such folder; delete this and its test.
const DEV_DATA_FALLBACK: &str = "D:/MCLA/extracted/cache";

pub struct Runtime {
    pub settings: Settings,
    pub settings_path: PathBuf,
    pub settings_status: Status,
    /// the file is not in effect: a headless run without `--settings` uses the defaults
    pub settings_ignored: bool,
    pub data: DataDir,
    pub screenshots: PathBuf,
    pub shot_run: bool,
}

static RT: OnceLock<Runtime> = OnceLock::new();

pub fn get() -> Option<&'static Runtime> {
    RT.get()
}

/// The data folder, the settings and the screenshots folder for this run. The first call wins (later calls, as in the
/// option unit tests, get the same answer). `data_flag` is `--data <dir>` or a bare folder argument.
pub fn init(data_flag: Option<&str>, settings_flag: Option<&str>, shot_run: bool) -> &'static Runtime {
    RT.get_or_init(|| {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
        let explicit = settings_flag.is_some_and(|f| !f.is_empty()) || std::env::var_os("MCLA_SETTINGS").is_some_and(|v| !v.is_empty());
        let settings_path = paths::settings_path(settings_flag);
        let (settings, settings_status, settings_ignored) = if shot_run && !explicit {
            // (what captures always did: no vertical sync, the old window)
            let mut s = Settings::default();
            s.display.vsync = false;
            (s, Status::Missing, true)
        } else {
            let l = Settings::load(&settings_path);
            (l.settings, l.status, false)
        };
        let env_data = std::env::var("MCLA_DATA").ok();
        let data = paths::resolve_data_dir(&DataInputs {
            flag: data_flag,
            env: env_data.as_deref(),
            settings: &settings.paths.data_dir,
            exe_dir: &exe_dir,
            dev_fallback: Some(Path::new(DEV_DATA_FALLBACK)),
        });
        match data.source {
            DataSource::NotFound => log::error!("data folder not found (looked at {}); run the launcher to build it, or pass --data <folder>", data.path.display()),
            DataSource::DevFallback => log::warn!("data folder: using the developer fallback {} (TODO-remove-before-release)", data.path.display()),
            s => log::info!("data folder: {} ({:?})", data.path.display(), s),
        }
        let screenshots = paths::screenshots_dir(&settings.paths.screenshots, &exe_dir);
        Runtime { settings, settings_path, settings_status, settings_ignored, data, screenshots, shot_run }
    })
}

/// The intro setting (`game.intro`); `normal` when settings were never loaded. After the OPTIONS > PERFORMANCE >
/// INTRO row changed it, the row's value.
pub fn intro_mode() -> Intro {
    INTRO_LIVE.lock().ok().and_then(|g| *g).unwrap_or_else(intro_at_start)
}

/// The intro setting this run started with (what the start-up sequence used).
pub fn intro_at_start() -> Intro {
    get().map_or(Intro::Normal, |r| r.settings.game.intro)
}

// ---------------------------------------------------------------- OPTIONS > PERFORMANCE > INTRO (the port's own row)

static INTRO_LIVE: Mutex<Option<Intro>> = Mutex::new(None);

/// The INTRO row's value: NORMAL / FAST / OFF (= `skip`), with "(NEXT START)" while it differs from the one this run
/// started with (the start-up sequence takes it at the next start; the career's opening at once).
pub fn intro_value() -> String {
    let m = intro_mode();
    let w = if m == Intro::Skip { "OFF".to_string() } else { m.word().to_uppercase() };
    if m != intro_at_start() { format!("{w} (NEXT START)") } else { w }
}

/// LEFT / RIGHT on the INTRO row: NORMAL -> FAST -> OFF round, written to `settings.toml` `game.intro` at once
/// (only that key; "skip" for OFF), not in a headless run that ignores the user's file.
pub fn intro_step(fwd: bool) {
    let list = [Intro::Normal, Intro::Fast, Intro::Skip];
    let cur = intro_mode();
    let i = list.iter().position(|x| *x == cur).unwrap_or(0);
    let next = list[if fwd { (i + 1) % 3 } else { (i + 2) % 3 }];
    if let Ok(mut g) = INTRO_LIVE.lock() {
        *g = Some(next);
    }
    let Some(rt) = get() else { return };
    if rt.settings_ignored {
        log::info!("performance page: game.intro not written (headless run without --settings)");
        return;
    }
    match mcla_config::set_key(&rt.settings_path, "game", "intro", mcla_config::toml::Value::String(next.word().into())) {
        Ok(()) => log::info!("performance page: game.intro = {} written to {}", next.word(), rt.settings_path.display()),
        Err(e) => log::warn!("performance page: game.intro not written: {e}"),
    }
}

/// The new-career arrival: the opening film plays with `normal` and `skip` (OFF), and the arrival scene CUT_INTRO
/// in the film's place with `fast` only. The original has no switch that leaves out a new career's opening:
/// CarSelection (0xebb682f7) plays `intro720.bik` unconditionally (0x8ab) before City_DoneStreaming (0xb6d) and
/// VEHICLE SELECT (0xedb on), so OFF - the original's skip path for the start-up movie (boot.rs) - leaves the
/// opening as it is (GitHub #8: with OFF the car was chosen before any opening; DECISIONS.md D24).
pub fn intro_film() -> bool {
    film_for(intro_mode())
}

pub fn intro_scene() -> bool {
    scene_for(intro_mode())
}

fn film_for(m: Intro) -> bool {
    m != Intro::Fast
}

fn scene_for(m: Intro) -> bool {
    m == Intro::Fast
}

// ---------------------------------------------------------------- logging

#[derive(Default)]
struct LogState {
    file: Option<std::fs::File>,
    /// everything logged before the file was opened, written to it when it is
    pre: Vec<u8>,
    tail: VecDeque<String>,
    partial: String,
    path: Option<PathBuf>,
}

static LOG: Mutex<LogState> = Mutex::new(LogState { file: None, pre: Vec::new(), tail: VecDeque::new(), partial: String::new(), path: None });

/// The writer env_logger is given: stderr as before, plus the file once [`start_file_log`] has opened it.
struct Tee;

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Ok(mut st) = LOG.lock() {
            let text = String::from_utf8_lossy(buf);
            st.partial.push_str(&text);
            while let Some(i) = st.partial.find('\n') {
                let line: String = st.partial.drain(..=i).collect();
                st.tail.push_back(line.trim_end().to_string());
                if st.tail.len() > TAIL_LINES {
                    st.tail.pop_front();
                }
            }
            if st.file.is_some() {
                if let Some(f) = st.file.as_mut() {
                    let _ = f.write_all(buf);
                }
            } else if st.pre.len() < 1 << 20 {
                st.pre.extend_from_slice(buf);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn init_logging() {
    let env = env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn");
    let _ = env_logger::Builder::from_env(env).target(env_logger::Target::Pipe(Box::new(Tee))).try_init();
}

pub fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Delete the oldest `<prefix>*<ext>` files of a folder until `keep` are left (names carry a sortable time).
pub fn prune(dir: &Path, prefix: &str, ext: &str, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut names: Vec<String> = rd.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.starts_with(prefix) && n.ends_with(ext)).collect();
    names.sort();
    let extra = names.len().saturating_sub(keep);
    for n in names.into_iter().take(extra) {
        let _ = std::fs::remove_file(dir.join(n));
    }
}

fn windows_version() -> String {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(o) = std::process::Command::new("cmd").args(["/C", "ver"]).creation_flags(0x0800_0000).output() {
            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

/// Open `logs/game-<time>.log`, write the header, then everything logged so far. Called once, for a normal run only.
pub fn start_file_log() {
    let Some(rt) = get() else { return };
    if rt.shot_run {
        return;
    }
    let dir = paths::logs_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        log::warn!("log: cannot create {}", dir.display());
        return;
    }
    prune(&dir, "game-", ".log", KEEP - 1);
    let mut path = dir.join(format!("game-{}.log", timestamp()));
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(format!("game-{}-{n}.log", timestamp()));
    }
    let Ok(mut f) = std::fs::File::create(&path) else {
        log::warn!("log: cannot create {}", path.display());
        return;
    };
    let _ = f.write_all(header(rt, &path).as_bytes());
    if let Ok(mut st) = LOG.lock() {
        let pre = std::mem::take(&mut st.pre);
        let _ = f.write_all(&pre);
        st.file = Some(f);
        st.path = Some(path.clone());
    }
    log::info!("log file: {}", path.display());
}

fn header(rt: &Runtime, log_path: &Path) -> String {
    let save = crate::game::savedir::get().map_or_else(|| "(beside the data)".to_string(), |p| p.display().to_string());
    let settings = if rt.settings_ignored {
        "defaults (headless run, no --settings)".to_string()
    } else {
        format!("{} ({:?})", rt.settings_path.display(), rt.settings_status)
    };
    format!(
        "Rusty MCLA game log\nversion:     {}\nstarted:     {}\nos:          {} {} {}\nexe:         {}\nargs:        {}\nlog:         {}\nsettings:    {}\nin effect:   {}\ndata:        {} ({:?})\nsave:        {}\nscreenshots: {}\ngpu:         (logged as \"adapter:\" once the window exists)\n----\n",
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S %z"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        windows_version(),
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
        std::env::args().skip(1).collect::<Vec<_>>().join(" "),
        log_path.display(),
        settings,
        rt.settings.summary(),
        rt.data.path.display(),
        rt.data.source,
        save,
        rt.screenshots.display(),
    )
}

static ADAPTER: Mutex<String> = Mutex::new(String::new());
/// The game draws through DX12: its swap chain cannot be made on an exclusive-fullscreen window (DXGI_ERROR_INVALID_CALL,
/// every card - GitHub #12), so fullscreen is borderless there (`fullscreen_on`, `render::Gpu::base`).
static ON_DX12: AtomicBool = AtomicBool::new(false);

/// The game draws through DX12 (`set_adapter`).
pub fn on_dx12() -> bool {
    ON_DX12.load(Ordering::Relaxed)
}

/// Called by `Gpu::new` with the adapter wgpu chose.
pub fn set_adapter(info: &wgpu::AdapterInfo) {
    ON_DX12.store(info.backend == wgpu::Backend::Dx12, Ordering::Relaxed);
    let line = format!("{} (backend {:?}, {:?}, driver {} {}, vendor 0x{:04x} device 0x{:04x})", info.name, info.backend, info.device_type, info.driver, info.driver_info, info.vendor, info.device);
    log::info!("adapter: {line}");
    if let Ok(mut a) = ADAPTER.lock() {
        *a = line;
    }
}

// ---------------------------------------------------------------- entry, panic hook

static MAIN_THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();

/// Runs the game: logging, the panic hook, the exit code (0 normal, 1 an error was returned, 101 a panic).
pub fn run(game: fn() -> anyhow::Result<()>) -> std::process::ExitCode {
    let _ = MAIN_THREAD.set(std::thread::current().id());
    init_logging();
    std::panic::set_hook(Box::new(panic_hook));
    match game() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            log::error!("fatal: {e:#}");
            std::process::ExitCode::from(1)
        }
    }
}

fn panic_hook(info: &std::panic::PanicHookInfo<'_>) {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("unnamed").to_string();
    let on_main = MAIN_THREAD.get() == Some(&thread.id());
    let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into());
    let place = info.location().map_or_else(|| "(unknown place)".to_string(), |l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
    let bt = std::backtrace::Backtrace::force_capture().to_string();
    let report = crash_report(&name, &msg, &place, &trim_backtrace(&bt));
    eprintln!("panic in thread '{name}': {msg}\n  at {place}");
    let rt = get();
    match rt.filter(|r| !r.shot_run) {
        Some(_) => {
            let dir = paths::logs_dir();
            let _ = std::fs::create_dir_all(&dir);
            prune(&dir, "crash-", ".txt", KEEP - 1);
            let mut path = dir.join(format!("crash-{}.txt", timestamp()));
            let mut n = 1;
            while path.exists() {
                n += 1;
                path = dir.join(format!("crash-{}-{n}.txt", timestamp()));
            }
            match std::fs::write(&path, &report) {
                Ok(()) => eprintln!("crash report: {}", path.display()),
                Err(e) => eprintln!("crash report not written ({}): {e}", path.display()),
            }
        }
        None => eprintln!("{report}"),
    }
    if on_main {
        std::process::exit(101);
    }
}

/// Drop the frames of the panic machinery (the hook, std's panic code) from the top of a backtrace.
fn trim_backtrace(bt: &str) -> String {
    let lines: Vec<&str> = bt.lines().collect();
    match lines.iter().position(|l| l.contains("rust_begin_unwind")) {
        Some(i) => lines[(i + 2).min(lines.len())..].join("
"),
        None => bt.to_string(),
    }
}

fn crash_report(thread: &str, msg: &str, place: &str, backtrace: &str) -> String {
    let rt = get();
    let tail: Vec<String> = LOG.try_lock().map(|st| st.tail.iter().cloned().collect()).unwrap_or_default();
    let adapter = ADAPTER.try_lock().map(|a| a.clone()).unwrap_or_default();
    format!(
        "Rusty MCLA crash report\nversion:   {}\ntime:      {}\nos:        {} {} {}\nthread:    {}\nmessage:   {}\nplace:     {}\nadapter:   {}\nsettings:  {}\ndata:      {}\nargs:      {}\n\n---- backtrace ----\n{}\n\n---- last {} log lines ----\n{}\n",
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S %z"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        windows_version(),
        thread,
        msg,
        place,
        if adapter.is_empty() { "(not created yet)" } else { &adapter },
        rt.map_or_else(|| "(not loaded)".to_string(), |r| format!("{} [{}]", r.settings_path.display(), r.settings.summary())),
        rt.map_or_else(String::new, |r| r.data.path.display().to_string()),
        std::env::args().skip(1).collect::<Vec<_>>().join(" "),
        backtrace,
        TAIL_LINES,
        tail.join("\n"),
    )
}

// ---------------------------------------------------------------- display

/// Vertical sync: `display.vsync` (the auto modes pick what the surface supports).
pub fn present_mode() -> wgpu::PresentMode {
    let vsync = display().vsync;
    let m = if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
    log::info!("present mode: {m:?} (vsync {vsync})");
    m
}

/// The launcher's monitor numbering (`display.monitor`): 0 = the primary monitor, then the others in the order the system
/// lists them (winit and the launcher's Tauri both ask Windows' `EnumDisplayMonitors`). `primary` = its place in `all`;
/// none = the first one counts as primary.
pub fn monitor_order(count: usize, primary: Option<usize>) -> Vec<usize> {
    let p = primary.filter(|p| *p < count).unwrap_or(0);
    if count == 0 {
        return Vec::new();
    }
    std::iter::once(p).chain((0..count).filter(|i| *i != p)).collect()
}

/// The monitor of `display.monitor`; an index that is not there (a monitor unplugged) falls back to the primary one with a
/// warning. None only when the system lists no monitor at all.
pub fn chosen_monitor(event_loop: &winit::event_loop::ActiveEventLoop, index: u32) -> Option<winit::monitor::MonitorHandle> {
    let all: Vec<_> = event_loop.available_monitors().collect();
    let primary = event_loop.primary_monitor();
    let order = monitor_order(all.len(), primary.as_ref().and_then(|p| all.iter().position(|m| m == p)));
    let name = |m: &winit::monitor::MonitorHandle| format!("{} ({}x{} at {},{})", m.name().unwrap_or_default(), m.size().width, m.size().height, m.position().x, m.position().y);
    match order.get(index as usize) {
        Some(&i) => {
            let hz = all[i].refresh_rate_millihertz().map_or(0, |m| (m + 500) / 1000);
            MONITOR_HZ.store(hz, std::sync::atomic::Ordering::Relaxed);
            log::info!("monitor: display.monitor = {index} -> {} of {} ({}), {hz} Hz", name(&all[i]), all.len(), if index == 0 { "primary" } else { "listed" });
            let d = display();
            // (a cap that is not the refresh rate or a whole fraction of it, with vsync: frames stay up for one refresh or
            // two in turn - a steady judder; GitHub #11, 60 on a 75 Hz monitor)
            if d.vsync && d.fps_cap > 0 && hz > 0 && d.fps_cap < hz && hz % d.fps_cap != 0 {
                log::warn!("monitor: fps_cap {} with vsync on a {hz} Hz monitor shows frames for uneven numbers of refreshes (judder); a cap of {hz} or none is even", d.fps_cap);
            }
            Some(all[i].clone())
        }
        None => {
            let m = primary.or_else(|| all.first().cloned());
            log::warn!("monitor: display.monitor = {index} but only {} monitor(s) are connected; the primary one is used: {}", all.len(), m.as_ref().map_or_else(|| "none".to_string(), name));
            m
        }
    }
}

/// Fullscreen on a monitor: the exclusive video mode of `res` (highest refresh, then bit depth), else borderless with a
/// warning.
fn fullscreen_on(monitor: Option<winit::monitor::MonitorHandle>, res: mcla_config::Resolution) -> winit::window::Fullscreen {
    use winit::window::Fullscreen;
    if ON_DX12.load(Ordering::Relaxed) {
        log::warn!("fullscreen: borderless on DX12 (no swap chain on an exclusive-fullscreen window there)");
        return Fullscreen::Borderless(monitor);
    }
    let want = winit::dpi::PhysicalSize::new(res.w, res.h);
    let mode = monitor.as_ref().and_then(|m| m.video_modes().filter(|v| v.size() == want).max_by_key(|v| (v.refresh_rate_millihertz(), v.bit_depth())));
    match mode {
        Some(v) => {
            log::info!("fullscreen: video mode {}x{} @ {} mHz, {} bit", v.size().width, v.size().height, v.refresh_rate_millihertz(), v.bit_depth());
            Fullscreen::Exclusive(v)
        }
        None => {
            log::warn!("fullscreen: the monitor has no {} video mode; borderless fullscreen instead", res.text());
            Fullscreen::Borderless(monitor)
        }
    }
}

/// The window's size, mode and monitor from `display.window_mode` / `display.resolution` / `display.monitor`. A headless
/// run without `--settings` keeps the old 1600 x 900 logical window. In borderless and fullscreen the monitor decides the
/// size; fullscreen ("exclusive") asks for a video mode of the resolution on the chosen monitor and falls back to
/// borderless. Windowed on monitor 0 is placed by the system, as before; on another monitor it is centred on it.
pub fn window_attributes(event_loop: &winit::event_loop::ActiveEventLoop, attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    use winit::window::Fullscreen;
    let Some(rt) = get().filter(|r| !r.settings_ignored) else {
        set_mode(WindowMode::Windowed);
        return attrs;
    };
    let d = &rt.settings.display;
    set_mode(d.window_mode);
    let attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(d.resolution.w, d.resolution.h));
    let monitor = chosen_monitor(event_loop, d.monitor);
    match d.window_mode {
        WindowMode::Windowed => match monitor.filter(|_| d.monitor != 0) {
            Some(m) => {
                let (mp, ms) = (m.position(), m.size());
                let x = mp.x + (ms.width as i32 - d.resolution.w as i32).max(0) / 2;
                let y = mp.y + (ms.height as i32 - d.resolution.h as i32).max(0) / 2;
                attrs.with_position(winit::dpi::PhysicalPosition::new(x, y))
            }
            None => attrs,
        },
        WindowMode::Borderless => attrs.with_fullscreen(Some(Fullscreen::Borderless(monitor))),
        WindowMode::Fullscreen => attrs.with_fullscreen(Some(fullscreen_on(monitor, d.resolution))),
    }
}

// ---------------------------------------------------------------- Alt+Enter: window mode

/// The window mode in effect (what was asked for; a fullscreen that fell back to borderless still counts as fullscreen,
/// so the next Alt+Enter goes on to windowed).
static MODE: Mutex<Option<WindowMode>> = Mutex::new(None);
/// The windowed size before leaving windowed, given back on the way back.
static WINDOWED_SIZE: Mutex<Option<winit::dpi::PhysicalSize<u32>>> = Mutex::new(None);
/// Alt is held (winit's `ModifiersChanged`).
static ALT: AtomicBool = AtomicBool::new(false);

fn set_mode(m: WindowMode) {
    if let Ok(mut g) = MODE.lock() {
        *g = Some(m);
    }
}

pub fn window_mode() -> WindowMode {
    MODE.lock().ok().and_then(|g| *g).unwrap_or(WindowMode::Windowed)
}

/// The window's `ModifiersChanged`.
pub fn set_alt(down: bool) {
    ALT.store(down, Ordering::Relaxed);
}

/// What a key event means for Alt+Enter (`mcla_config::keybinds::alt_enter`, either Enter, either Alt): `Some(true)` =
/// cycle the window mode, `Some(false)` = Alt+Enter again (key repeat, a synthetic press, the release): swallowed, nothing
/// else hears it; `None` = not Alt+Enter.
pub fn alt_enter_event(alt: bool, code: winit::keyboard::KeyCode, pressed: bool, repeat: bool, synthetic: bool) -> Option<bool> {
    let key = crate::keybinds::to_key(code)?;
    if !mcla_config::keybinds::alt_enter(alt, key) {
        return None;
    }
    Some(pressed && !repeat && !synthetic)
}

/// Put a window into a mode at once (Alt+Enter). Fullscreen uses the window's current monitor and `display.resolution`.
pub fn apply_window_mode(window: &winit::window::Window, from: WindowMode, to: WindowMode) {
    use winit::window::Fullscreen;
    if from == WindowMode::Windowed {
        if let Ok(mut s) = WINDOWED_SIZE.lock() {
            *s = Some(window.inner_size());
        }
    }
    let res = get().filter(|r| !r.settings_ignored).map_or(mcla_config::Settings::default().display.resolution, |r| r.settings.display.resolution);
    match to {
        WindowMode::Windowed => {
            window.set_fullscreen(None);
            if let Some(s) = WINDOWED_SIZE.lock().ok().and_then(|g| *g) {
                let _ = window.request_inner_size(s);
            }
        }
        WindowMode::Borderless => window.set_fullscreen(Some(Fullscreen::Borderless(window.current_monitor()))),
        WindowMode::Fullscreen => window.set_fullscreen(Some(fullscreen_on(window.current_monitor().or_else(|| window.primary_monitor()), res))),
    }
}

/// Write the new window mode into `settings.toml`: only that key (`mcla_config::set_key`, atomic). Not in a headless run
/// that ignores the user's file, and never over a file that is not valid TOML.
pub fn write_window_mode(m: WindowMode) -> Result<PathBuf, String> {
    let rt = get().ok_or("no settings")?;
    if rt.settings_ignored {
        return Err("headless run without --settings: the user's settings file is not touched".into());
    }
    mcla_config::set_key(&rt.settings_path, "display", "window_mode", mcla_config::toml::Value::String(m.word().into())).map_err(|e| e.to_string())?;
    Ok(rt.settings_path.clone())
}

impl App {
    /// A key event of the window, before anything else hears it: true = Alt+Enter took it.
    pub(crate) fn alt_enter_key(&mut self, code: winit::keyboard::KeyCode, pressed: bool, repeat: bool, synthetic: bool) -> bool {
        match alt_enter_event(ALT.load(Ordering::Relaxed), code, pressed, repeat, synthetic) {
            None => false,
            Some(false) => true,
            Some(true) => {
                self.cycle_window_mode();
                true
            }
        }
    }

    /// Alt+Enter: windowed -> borderless -> fullscreen -> windowed, at once, and written back to `settings.toml`.
    pub(crate) fn cycle_window_mode(&mut self) {
        let from = window_mode();
        let to = from.next();
        if let Some(w) = self.window.as_ref() {
            apply_window_mode(w, from, to);
        }
        set_mode(to);
        match write_window_mode(to) {
            Ok(p) => log::info!("Alt+Enter: window mode {} -> {} (written to {})", from.word(), to.word(), p.display()),
            Err(e) => log::warn!("Alt+Enter: window mode {} -> {}; not written: {e}", from.word(), to.word()),
        }
        if let Some(w) = self.window.as_ref() {
            log_window(w);
        }
    }
}

/// One log line about the window that was made.
pub fn log_window(window: &winit::window::Window) {
    let s = window.inner_size();
    log::info!(
        "window: inner {}x{} px, scale {:.2}, fullscreen {}, monitor {:?}",
        s.width,
        s.height,
        window.scale_factor(),
        match window.fullscreen() {
            None => "no".to_string(),
            Some(winit::window::Fullscreen::Borderless(_)) => "borderless".to_string(),
            Some(winit::window::Fullscreen::Exclusive(v)) => format!("exclusive {}x{}", v.size().width, v.size().height),
        },
        window.current_monitor().and_then(|m| m.name())
    );
}

// ---------------------------------------------------------------- frame limiter

static LAST_FRAME: Mutex<Option<Instant>> = Mutex::new(None);
/// The chosen monitor's refresh rate in Hz (0 = not known): offered among the DISPLAY page's caps.
static MONITOR_HZ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// How long to wait so frames start at least `period` apart.
pub fn frame_wait(last: Instant, now: Instant, period: Duration) -> Duration {
    (last + period).saturating_duration_since(now)
}

/// Called after every rendered frame: sleeps (the last millisecond spins) to hold `display.fps_cap`.
pub fn limit_frame() {
    let cap = display().fps_cap;
    if cap == 0 {
        return;
    }
    let period = Duration::from_secs_f64(1.0 / cap as f64);
    let Ok(mut last) = LAST_FRAME.lock() else { return };
    let now = Instant::now();
    let Some(prev) = *last else {
        *last = Some(now);
        return;
    };
    let wait = frame_wait(prev, now, period);
    if wait.is_zero() {
        // (late: no catching up with a burst)
        *last = Some(now);
        return;
    }
    let target = now + wait;
    if wait > Duration::from_millis(2) {
        std::thread::sleep(wait - Duration::from_millis(2));
    }
    while Instant::now() < target {
        std::hint::spin_loop();
    }
    *last = Some(target);
}

// ---------------------------------------------------------------- OPTIONS > DISPLAY (the port's own page, D19)

/// The display settings in effect: `settings.toml`'s at the start (the defaults in a headless run without `--settings`),
/// then what the DISPLAY page changed.
static LIVE: Mutex<Option<mcla_config::Display>> = Mutex::new(None);
/// The monitors in the launcher's numbering (`monitor_order`): name, size, and the sizes of their video modes.
static MONITORS: Mutex<Vec<(String, (u32, u32), Vec<(u32, u32)>)>> = Mutex::new(Vec::new());

/// The DISPLAY page's rows (`Go::Page` ids in game.rs).
pub const ROW_WINDOW_MODE: u8 = 60;
pub const ROW_RESOLUTION: u8 = 61;
pub const ROW_MONITOR: u8 = 62;
pub const ROW_VSYNC: u8 = 63;
pub const ROW_FPS_CAP: u8 = 64;
pub const ROW_FPS_MONITOR: u8 = 65;
pub const ROW_TEX_FILTER: u8 = 55;
pub const ROW_RENDER_SCALE: u8 = 56;
pub const ROW_SHADOW_RES: u8 = 57;
pub const ROW_DRAW_DISTANCE: u8 = 58;
pub const ROW_DETAIL_LEVEL: u8 = 59;
pub const ROW_AA: u8 = 66;
pub const ROW_BLOOM: u8 = 67;
pub const ROW_DOF: u8 = 68;
pub const ROW_MOTION_BLUR: u8 = 69;
/// The render scales the DISPLAY row steps through (1.0 = native, the original's).
const RENDER_SCALES: [f32; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];
/// The frame-rate caps the page steps through (0 = none); a cap from the file that is not here is put in its place.
const FPS_CAPS: [u32; 7] = [0, 30, 60, 120, 144, 165, 240];

pub fn display() -> mcla_config::Display {
    LIVE.lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| get().map_or_else(|| Settings::default().display, |r| r.settings.display.clone()))
}

fn set_display(d: mcla_config::Display) {
    if let Ok(mut g) = LIVE.lock() {
        *g = Some(d);
    }
}

fn monitors() -> Vec<(String, (u32, u32), Vec<(u32, u32)>)> {
    MONITORS.lock().map(|g| g.clone()).unwrap_or_default()
}

/// The resolutions the page offers: the chosen monitor's video mode sizes (from 640 x 360 up), and the one in effect.
fn resolutions(d: &mcla_config::Display) -> Vec<(u32, u32)> {
    let mons = monitors();
    let mut v: Vec<(u32, u32)> = mons.get(d.monitor as usize).or(mons.first()).map(|m| m.2.clone()).unwrap_or_default();
    v.retain(|r| r.0 >= mcla_config::Resolution::MIN.0 && r.1 >= mcla_config::Resolution::MIN.1);
    v.push((d.resolution.w, d.resolution.h));
    v.sort();
    v.dedup();
    v
}

fn fps_caps(d: &mcla_config::Display) -> Vec<u32> {
    let mut v = FPS_CAPS.to_vec();
    // (the monitor's own rate too - 75, 100, 180 Hz ...: a cap that is even with vsync, GitHub #11)
    let hz = MONITOR_HZ.load(std::sync::atomic::Ordering::Relaxed);
    for c in [d.fps_cap, hz] {
        if c > 0 && !v.contains(&c) {
            v.push(c);
        }
    }
    v.sort();
    v
}

/// The value a DISPLAY row shows.
pub fn display_value(row: u8) -> String {
    let d = display();
    match row {
        ROW_WINDOW_MODE => d.window_mode.word().to_uppercase(),
        ROW_RESOLUTION => format!("{} X {}", d.resolution.w, d.resolution.h),
        ROW_MONITOR => match monitors().get(d.monitor as usize) {
            Some((name, (w, h), _)) => format!("{}  {} ({} X {})", d.monitor + 1, name.trim_start_matches(r"\\.\").to_uppercase(), w, h),
            None => format!("{}", d.monitor + 1),
        },
        ROW_VSYNC => if d.vsync { "ON" } else { "OFF" }.to_string(),
        ROW_FPS_CAP => if d.fps_cap == 0 { "OFF".to_string() } else { d.fps_cap.to_string() },
        ROW_FPS_MONITOR => match d.fps_monitor {
            FpsMonitor::Off => "OFF",
            FpsMonitor::Fps => "FPS",
            FpsMonitor::FrameTime => "FPS + FRAME TIME",
        }
        .to_string(),
        ROW_TEX_FILTER => d.texture_filtering.word().to_uppercase(),
        ROW_RENDER_SCALE => format!("{}%", (d.render_scale * 100.0).round() as i32),
        ROW_SHADOW_RES => game_live().shadow_resolution.word().to_uppercase(),
        ROW_DRAW_DISTANCE => game_live().draw_distance.word().to_uppercase(),
        ROW_DETAIL_LEVEL => game_live().detail_level.word().to_uppercase(),
        ROW_AA => match d.aa {
            mcla_config::Aa::Original => "ORIGINAL",
            mcla_config::Aa::Off => "OFF",
            mcla_config::Aa::X2 => "2X",
            mcla_config::Aa::X4 => "4X",
        }
        .to_string(),
        ROW_BLOOM => if d.bloom { "ON" } else { "OFF" }.to_string(),
        ROW_DOF => if d.dof { "ON" } else { "OFF" }.to_string(),
        ROW_MOTION_BLUR => if d.motion_blur { "ON" } else { "OFF" }.to_string(),
        _ => String::new(),
    }
}

/// The anti-aliasing mode the renderer's `aamode` takes for a settings value.
fn aa_mode(a: mcla_config::Aa) -> crate::aamode::Mode {
    use crate::aamode::Mode;
    match a {
        mcla_config::Aa::Original => Mode::Original,
        mcla_config::Aa::Off => Mode::Off,
        mcla_config::Aa::X2 => Mode::X2,
        mcla_config::Aa::X4 => Mode::X4,
    }
}

/// Hands the settings' anti-aliasing mode to the renderer (`aamode`); once at start-up (a headless run without
/// `--settings` has the defaults, ORIGINAL).
pub fn apply_aa_at_start() {
    crate::aamode::set(aa_mode(display().aa));
}

/// Texture filtering 16x everywhere (`display.texture_filtering`, live); false = the original's anisotropy (4 while
/// driving, 16 in the menus' / garage 4x mode).
pub fn aniso_16x() -> bool {
    display().texture_filtering == mcla_config::TexFilter::X16
}

/// The render scale (`display.render_scale`, live; 1.0 = native). The scene is drawn at window x scale.
pub fn render_scale() -> f32 {
    display().render_scale.clamp(0.5, 2.0)
}

static GAME_LIVE: Mutex<Option<mcla_config::GameSettings>> = Mutex::new(None);

/// The `game.*` performance settings in effect (the file's at the start, then what the PERFORMANCE page changed).
fn game_live() -> mcla_config::GameSettings {
    GAME_LIVE.lock().ok().and_then(|g| g.clone()).unwrap_or_else(|| get().map_or_else(|| Settings::default().game, |r| r.settings.game.clone()))
}

fn set_game_live(g: mcla_config::GameSettings) {
    if let Ok(mut l) = GAME_LIVE.lock() {
        *l = Some(g);
    }
}

/// Shadow map side multiplier: 1 (the original's), 2 or 4.
pub fn shadow_res_scale() -> u32 {
    game_live().shadow_resolution.factor()
}

/// Draw distance multiplier (1.0 = the original's).
pub fn draw_distance_scale() -> f32 {
    game_live().draw_distance.factor()
}

/// Detail level multiplier (1.0 = the original's); 3.0 also keeps traffic cars at LOD 0.
pub fn detail_level_scale() -> f32 {
    game_live().detail_level.factor()
}

/// Traffic cars at LOD 0 wherever the original draws them (the old HIGH-DETAIL TRAFFIC; folded into DETAIL LEVEL 3X).
pub fn hq_traffic() -> bool {
    detail_level_scale() >= 3.0
}

/// A save written with the old HIGH-DETAIL TRAFFIC row ON becomes DETAIL LEVEL 3X, when `game.detail_level` is not in
/// the settings file yet (the old key is read for this and nothing else).
pub fn migrate_hq_traffic(was_on: bool) {
    let Some(rt) = get() else { return };
    if !was_on || rt.settings_ignored {
        return;
    }
    let present = std::fs::read_to_string(&rt.settings_path)
        .ok()
        .and_then(|t| t.parse::<mcla_config::toml::Table>().ok())
        .is_some_and(|t| t.get("game").and_then(|g| g.get("detail_level")).is_some());
    if present {
        return;
    }
    let mut g = game_live();
    g.detail_level = mcla_config::Scale::X3;
    set_game_live(g);
    write_game_key("detail_level", mcla_config::toml::Value::String("3x".into()));
}

/// One key of `settings.toml`'s `[game]` table (only that key; atomic); not in a headless run that ignores the file.
fn write_game_key(key: &str, value: mcla_config::toml::Value) {
    let Some(rt) = get() else { return };
    if rt.settings_ignored {
        log::info!("performance page: game.{key} not written (headless run without --settings)");
        return;
    }
    match mcla_config::set_key(&rt.settings_path, "game", key, value) {
        Ok(()) => log::info!("performance page: game.{key} written to {}", rt.settings_path.display()),
        Err(e) => log::warn!("performance page: game.{key} not written: {e}"),
    }
}

/// Bloom on (`display.bloom`, live; ON = the original's). The renderer reads it.
pub fn bloom_on() -> bool {
    display().bloom
}

/// Depth of field on (`display.dof`, live; ON = the original's).
pub fn dof_on() -> bool {
    display().dof
}

/// Motion blur on (`display.motion_blur`, live; ON = the original's).
pub fn motion_blur_on() -> bool {
    display().motion_blur
}

/// The next (or previous) item of `list` after `cur`, round and round.
fn step_in<T: PartialEq + Copy>(list: &[T], cur: T, fwd: bool) -> T {
    let n = list.len();
    if n == 0 {
        return cur;
    }
    let i = list.iter().position(|x| *x == cur).unwrap_or(0);
    list[if fwd { (i + 1) % n } else { (i + n - 1) % n }]
}

/// One key of `settings.toml` (only that key, atomic); not in a headless run that ignores the user's file.
fn write_display_key(key: &str, value: mcla_config::toml::Value) {
    let Some(rt) = get() else { return };
    if rt.settings_ignored {
        log::info!("display page: display.{key} not written (headless run without --settings)");
        return;
    }
    match mcla_config::set_key(&rt.settings_path, "display", key, value) {
        Ok(()) => log::info!("display page: display.{key} written to {}", rt.settings_path.display()),
        Err(e) => log::warn!("display page: display.{key} not written: {e}"),
    }
}

/// The monitors in the launcher's numbering, with their names, sizes and video mode sizes.
fn list_monitors(w: &winit::window::Window) -> Vec<winit::monitor::MonitorHandle> {
    let all: Vec<_> = w.available_monitors().collect();
    let primary = w.primary_monitor();
    let order = monitor_order(all.len(), primary.as_ref().and_then(|p| all.iter().position(|m| m == p)));
    let ordered: Vec<_> = order.iter().map(|&i| all[i].clone()).collect();
    if let Ok(mut g) = MONITORS.lock() {
        *g = ordered.iter().map(|m| (m.name().unwrap_or_default(), (m.size().width, m.size().height), m.video_modes().map(|v| (v.size().width, v.size().height)).collect())).collect();
    }
    ordered
}

impl App {
    /// The monitor list for the DISPLAY page's labels.
    pub(crate) fn note_monitors(&self) {
        if let Some(w) = self.window.as_ref() {
            list_monitors(w);
        }
    }

    /// A row of OPTIONS > DISPLAY stepped on (`fwd`) or back: applied to the window at once and written to `settings.toml`.
    pub(crate) fn display_step(&mut self, row: u8, fwd: bool) {
        use mcla_config::toml::Value;
        use winit::window::Fullscreen;
        let Some(w) = self.window.clone() else { return };
        // (fresh: a monitor may have been plugged in since)
        let ordered = list_monitors(&w);
        let mut d = display();
        let size = |d: &mcla_config::Display| winit::dpi::PhysicalSize::new(d.resolution.w, d.resolution.h);
        match row {
            ROW_WINDOW_MODE => {
                let modes = [WindowMode::Windowed, WindowMode::Borderless, WindowMode::Fullscreen];
                let from = window_mode();
                let to = step_in(&modes, from, fwd);
                d.window_mode = to;
                set_display(d);
                apply_window_mode(&w, from, to);
                set_mode(to);
                write_display_key("window_mode", Value::String(to.word().into()));
            }
            ROW_RESOLUTION => {
                let (rw, rh) = step_in(&resolutions(&d), (d.resolution.w, d.resolution.h), fwd);
                d.resolution = mcla_config::Resolution { w: rw, h: rh };
                match window_mode() {
                    WindowMode::Windowed => {
                        let _ = w.request_inner_size(size(&d));
                    }
                    WindowMode::Fullscreen => w.set_fullscreen(Some(fullscreen_on(w.current_monitor().or_else(|| w.primary_monitor()), d.resolution))),
                    // (borderless takes the monitor's size; the resolution counts again in the other modes)
                    WindowMode::Borderless => {}
                }
                if let Ok(mut s) = WINDOWED_SIZE.lock() {
                    *s = Some(size(&d));
                }
                write_display_key("resolution", Value::String(d.resolution.text()));
                set_display(d);
            }
            ROW_MONITOR => {
                let n = ordered.len().max(1) as u32;
                d.monitor = if fwd { (d.monitor + 1) % n } else { (d.monitor + n - 1) % n };
                if let Some(m) = ordered.get(d.monitor as usize).cloned() {
                    match window_mode() {
                        WindowMode::Windowed => {
                            let (mp, ms, sz) = (m.position(), m.size(), w.outer_size());
                            let x = mp.x + (ms.width as i32 - sz.width as i32).max(0) / 2;
                            let y = mp.y + (ms.height as i32 - sz.height as i32).max(0) / 2;
                            w.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
                        }
                        WindowMode::Borderless => w.set_fullscreen(Some(Fullscreen::Borderless(Some(m)))),
                        WindowMode::Fullscreen => w.set_fullscreen(Some(fullscreen_on(Some(m), d.resolution))),
                    }
                }
                write_display_key("monitor", Value::Integer(d.monitor as i64));
                set_display(d);
            }
            ROW_VSYNC => {
                d.vsync = !d.vsync;
                write_display_key("vsync", Value::Boolean(d.vsync));
                set_display(d);
                if let Some(g) = self.gpu.as_mut() {
                    g.config.present_mode = present_mode();
                    g.surface.configure(&g.device, &g.config);
                }
            }
            ROW_FPS_CAP => {
                d.fps_cap = step_in(&fps_caps(&d), d.fps_cap, fwd);
                write_display_key("fps_cap", Value::Integer(d.fps_cap as i64));
                set_display(d);
            }
            ROW_FPS_MONITOR => {
                d.fps_monitor = step_in(FpsMonitor::ALL, d.fps_monitor, fwd);
                write_display_key("fps_monitor", Value::String(d.fps_monitor.word().into()));
                set_display(d);
            }
            ROW_TEX_FILTER => {
                d.texture_filtering = step_in(mcla_config::TexFilter::ALL, d.texture_filtering, fwd);
                write_display_key("texture_filtering", Value::String(d.texture_filtering.word().into()));
                set_display(d);
            }
            ROW_RENDER_SCALE => {
                let i = RENDER_SCALES.iter().enumerate().min_by(|a, b| (a.1 - d.render_scale).abs().total_cmp(&(b.1 - d.render_scale).abs())).map_or(0, |x| x.0);
                let n = RENDER_SCALES.len();
                d.render_scale = RENDER_SCALES[if fwd { (i + 1) % n } else { (i + n - 1) % n }];
                write_display_key("render_scale", Value::Float(format!("{}", d.render_scale).parse::<f64>().unwrap_or(1.0)));
                set_display(d);
            }
            ROW_SHADOW_RES => {
                let mut g = game_live();
                g.shadow_resolution = step_in(mcla_config::ShadowRes::ALL, g.shadow_resolution, fwd);
                write_game_key("shadow_resolution", Value::String(g.shadow_resolution.word().into()));
                set_game_live(g);
            }
            ROW_DRAW_DISTANCE => {
                let mut g = game_live();
                g.draw_distance = step_in(mcla_config::Scale::ALL, g.draw_distance, fwd);
                write_game_key("draw_distance", Value::String(g.draw_distance.word().into()));
                set_game_live(g);
            }
            ROW_DETAIL_LEVEL => {
                let mut g = game_live();
                g.detail_level = step_in(mcla_config::Scale::ALL, g.detail_level, fwd);
                write_game_key("detail_level", Value::String(g.detail_level.word().into()));
                set_game_live(g);
            }
            ROW_AA => {
                d.aa = step_in(mcla_config::Aa::ALL, d.aa, fwd);
                crate::aamode::set(aa_mode(d.aa));
                write_display_key("aa", Value::String(d.aa.word().into()));
                set_display(d);
            }
            ROW_BLOOM => {
                d.bloom = !d.bloom;
                write_display_key("bloom", Value::Boolean(d.bloom));
                set_display(d);
            }
            ROW_DOF => {
                d.dof = !d.dof;
                write_display_key("dof", Value::Boolean(d.dof));
                set_display(d);
            }
            ROW_MOTION_BLUR => {
                d.motion_blur = !d.motion_blur;
                write_display_key("motion_blur", Value::Boolean(d.motion_blur));
                set_display(d);
            }
            _ => return,
        }
        log::info!("display page: row {} -> {}", row, display_value(row));
        log_window(&w);
    }
}

// ---------------------------------------------------------------- FPS counter (the port's own, DECISIONS.md D25)

/// The FPS counter's meter: the time between presented frames (after the frame limiter), averaged over a window of
/// [`FPS_WINDOW`] and shown anew at the end of each window.
struct FpsMeter {
    last: Option<Instant>,
    frames: u32,
    sum: f64,
    /// frames per second, milliseconds per frame (the last window's)
    shown: Option<(f32, f32)>,
}

static FPS_METER: Mutex<FpsMeter> = Mutex::new(FpsMeter { last: None, frames: 0, sum: 0.0, shown: None });
/// How long the counter averages over (three new figures a second).
const FPS_WINDOW: f64 = 1.0 / 3.0;

/// One frame presented at `now`.
fn note_frame(now: Instant) {
    if let Ok(mut m) = FPS_METER.lock() {
        m.note(now);
    }
}

impl FpsMeter {
    fn note(&mut self, now: Instant) {
        if let Some(prev) = self.last {
            let dt = now.duration_since(prev).as_secs_f64();
            // (a stall of over a second - loading, the window dragged - starts afresh instead of showing a crawl)
            if dt > 1.0 {
                self.frames = 0;
                self.sum = 0.0;
            } else {
                self.frames += 1;
                self.sum += dt;
                if self.sum >= FPS_WINDOW {
                    let ms = self.sum / self.frames as f64;
                    self.shown = Some(((1.0 / ms) as f32, (ms * 1000.0) as f32));
                    self.frames = 0;
                    self.sum = 0.0;
                }
            }
        }
        self.last = Some(now);
    }
}

/// The counter's figures in effect (frames per second, milliseconds per frame), once a window has passed.
pub fn fps_shown() -> Option<(f32, f32)> {
    FPS_METER.lock().ok().and_then(|m| m.shown)
}

/// The counter's lines: the figure, its unit, and the frame time with `FrameTime`.
pub fn fps_lines(mode: FpsMonitor, fps: f32, ms: f32) -> Vec<(String, &'static str)> {
    match mode {
        FpsMonitor::Off => Vec::new(),
        FpsMonitor::Fps => vec![(format!("{:.0}", fps), "FPS")],
        FpsMonitor::FrameTime => vec![(format!("{:.0}", fps), "FPS"), (format!("{:.1}", ms), "MS")],
    }
}

/// Draws the FPS counter (when `display.fps_monitor` is on) at the top right of the screen, in the game's typeface:
/// the figures white over a dark shadow as the HUD's own numbers, the units in the cyan of the menus' selected row.
pub fn draw_fps_monitor(ctx: &egui::Context, font: Option<&crate::hud::TextFont>, hidden: bool) {
    let mode = display().fps_monitor;
    if mode == FpsMonitor::Off || hidden {
        return;
    }
    let Some((fps, ms)) = fps_shown() else { return };
    let screen = ctx.screen_rect();
    let sc = (screen.height() / 900.0).clamp(0.5, 2.0);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("fps_monitor")));
    let (big, small) = (15.0 * sc, 9.0 * sc);
    let measure = |t: &str, size: f32| font.map_or(t.len() as f32 * size * 0.7, |f| f.measure(t, size));
    let draw = |pos: egui::Pos2, t: &str, size: f32, color: egui::Color32| {
        for (d, c) in [(egui::vec2(1.5, 1.5) * sc, egui::Color32::from_black_alpha(200)), (egui::Vec2::ZERO, color)] {
            match font {
                Some(f) => {
                    f.draw(&painter, pos + d, egui::Align2::LEFT_BOTTOM, t, size, c);
                }
                None => {
                    painter.text(pos + d, egui::Align2::LEFT_BOTTOM, t, egui::FontId::proportional(size * 1.35), c);
                }
            }
        }
    };
    let white = egui::Color32::from_rgb(235, 238, 245);
    // (sampled from the pause menu's selected row: 4, 229, 230)
    let cyan = egui::Color32::from_rgb(4, 229, 230);
    let right = screen.right() - FPS_MARGIN.0 * sc;
    let mut base = screen.top() + FPS_MARGIN.1 * sc + big;
    for (fig, unit) in fps_lines(mode, fps, ms) {
        let size = if unit == "FPS" { big } else { small * 1.25 };
        let (wu, gap) = (measure(unit, small), 4.0 * sc);
        let wf = measure(&fig, size);
        draw(egui::pos2(right - wu - gap - wf, base), &fig, size, white);
        draw(egui::pos2(right - wu, base), unit, small, cyan);
        base += size + 8.0 * sc;
    }
}

/// The counter's distance from the screen's top right corner, in pixels of a 900-line screen.
const FPS_MARGIN: (f32, f32) = (28.0, 24.0);

// ---------------------------------------------------------------- screenshot key, test hooks

/// The file F12 asked for, until it has been written.
static SHOT_PENDING: Mutex<Option<(PathBuf, u32)>> = Mutex::new(None);
static TEST_PANIC_DONE: AtomicBool = AtomicBool::new(false);

/// `game-<time>.png`, with `-2`, `-3` ... when that name is taken (two presses in one second).
pub fn screenshot_name(dir: &Path, stamp: &str) -> PathBuf {
    let mut p = dir.join(format!("game-{stamp}.png"));
    let mut n = 1;
    while p.exists() {
        n += 1;
        p = dir.join(format!("game-{stamp}-{n}.png"));
    }
    p
}

impl App {
    /// F12: the next frame goes to a PNG in the screenshots folder (the photo mode's capture path).
    pub(crate) fn screenshot_key(&mut self) {
        if self.photo_request.is_some() {
            return;
        }
        let dir = get().map_or_else(|| PathBuf::from("screenshots"), |r| r.screenshots.clone());
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("screenshot: cannot create {}: {e}", dir.display());
            self.game.show_message("SCREENSHOT FAILED");
            return;
        }
        let path = screenshot_name(&dir, &timestamp());
        self.photo_request = Some(path.to_string_lossy().into_owned());
        if let Ok(mut p) = SHOT_PENDING.lock() {
            *p = Some((path, 0));
        }
    }

    /// After every rendered frame: the screenshot key's confirmation, the frame limiter, the test hooks.
    pub(crate) fn after_frame(&mut self) {
        // (a row of OPTIONS > DISPLAY was changed in the menu)
        if let Some((row, fwd)) = self.game.display_step.take() {
            self.display_step(row, fwd);
        } else if self.frame_no == 1 {
            self.note_monitors();
        }
        let pending = SHOT_PENDING.lock().ok().and_then(|mut p| match p.take() {
            Some((path, n)) if path.exists() => Some((path, n, true)),
            Some((path, n)) if n < 120 => {
                *p = Some((path, n + 1));
                None
            }
            Some((path, _)) => Some((path, 0, false)),
            None => None,
        });
        if let Some((path, _, done)) = pending {
            if done {
                let (w, h) = std::fs::File::open(&path).ok().and_then(|f| png::Decoder::new(std::io::BufReader::new(f)).read_info().ok()).map_or((0, 0), |r| (r.info().width, r.info().height));
                log::info!("screenshot key: saved {} ({}x{})", path.display(), w, h);
                if self.photo.is_none() {
                    self.game.show_message("SCREENSHOT SAVED");
                }
            } else {
                log::warn!("screenshot key: {} was not written", path.display());
            }
        }
        limit_frame();
        note_frame(Instant::now());
        // (test hooks: MCLA_SHOT_KEY_TEST=<frame> presses F12 at that frame; MCLA_TEST_PANIC=main|thread panics
        // at frame 30, in the main thread or in a worker; =quit leaves normally (exit code 0) - they exist only to check
        // the screenshot key, the crash report and the exit codes)
        if let Some(n) = std::env::var("MCLA_SHOT_KEY_TEST").ok().and_then(|v| v.parse::<u64>().ok()) {
            if self.frame_no as u64 == n {
                self.screenshot_key();
            }
        }
        // (MCLA_ALT_ENTER_TEST=<frame>[,<frame>...]: at each frame the key sequence Alt down, Enter, Enter (repeat), Enter up,
        // Alt up, Enter (no Alt) goes through the window handler's own decision (`alt_enter_key`); 30 frames later the window
        // is logged - it exists only to check Alt+Enter headlessly)
        if let Ok(v) = std::env::var("MCLA_ALT_ENTER_TEST") {
            let frames: Vec<u64> = v.split(',').filter_map(|f| f.trim().parse().ok()).collect();
            let n = self.frame_no as u64;
            if frames.contains(&n) {
                use winit::keyboard::KeyCode;
                set_alt(true);
                let a = self.alt_enter_key(KeyCode::Enter, true, false, false);
                let b = self.alt_enter_key(KeyCode::Enter, true, true, false);
                let c = self.alt_enter_key(KeyCode::Enter, false, false, false);
                set_alt(false);
                let d = self.alt_enter_key(KeyCode::Enter, true, false, false);
                log::info!("alt-enter test at frame {n}: press {a}, repeat {b}, release {c} (taken = true); plain Enter {d} (false = passed on); mode now {}", window_mode().word());
            }
            if frames.iter().any(|f| f + 30 == n) {
                if let Some(w) = self.window.as_ref() {
                    log::info!("alt-enter test: 30 frames later, mode {}", window_mode().word());
                    log_window(w);
                }
            }
        }
        if self.frame_no == 30 && !TEST_PANIC_DONE.swap(true, Ordering::Relaxed) {
            match std::env::var("MCLA_TEST_PANIC").as_deref() {
                Ok("thread") => {
                    let _ = std::thread::Builder::new().name("test-worker".into()).spawn(|| panic!("MCLA_TEST_PANIC in a worker thread")).map(|h| h.join());
                }
                Ok("quit") => self.quit = true,
                Ok(_) => panic!("MCLA_TEST_PANIC"),
                Err(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcla-hooks-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn fps_meter_averages_presented_frames() {
        let t0 = Instant::now();
        let mut m = FpsMeter { last: None, frames: 0, sum: 0.0, shown: None };
        // 60 frames 16.667 ms apart: nothing until a third of a second has passed, then 60 fps / 16.7 ms
        for i in 0..=21 {
            m.note(t0 + Duration::from_micros(16_667 * i));
        }
        let (fps, ms) = m.shown.unwrap();
        assert!((fps - 60.0).abs() < 0.1 && (ms - 16.667).abs() < 0.01, "{fps} {ms}");
        // a two-second stall starts afresh and keeps the last figures
        m.note(t0 + Duration::from_secs(3));
        assert_eq!((m.frames, m.shown.unwrap().0.round()), (0, 60.0));
        assert_eq!(fps_lines(FpsMonitor::FrameTime, 59.6, 16.78), vec![("60".to_string(), "FPS"), ("16.8".to_string(), "MS")]);
        assert!(fps_lines(FpsMonitor::Off, 60.0, 16.7).is_empty());
    }

    #[test]
    fn frame_wait_holds_the_period() {
        let t0 = Instant::now();
        let p = Duration::from_secs_f64(1.0 / 60.0);
        // 60 fps = 16.667 ms; a frame that took 5 ms waits 11.667 ms
        let w = frame_wait(t0, t0 + Duration::from_millis(5), p);
        assert_eq!(w.as_micros(), 11_666);
        assert_eq!(frame_wait(t0, t0 + Duration::from_millis(17), p), Duration::ZERO);
        // 144 fps = 6.944 ms
        let p = Duration::from_secs_f64(1.0 / 144.0);
        assert_eq!(frame_wait(t0, t0, p).as_micros(), 6_944);
    }

    #[test]
    fn old_logs_are_pruned_oldest_first() {
        let d = tmp("prune");
        for i in 0..14 {
            std::fs::write(d.join(format!("game-20260101-0000{:02}.log", i)), "x").unwrap();
        }
        std::fs::write(d.join("crash-20260101-000000.txt"), "x").unwrap();
        std::fs::write(d.join("launcher-20260101-000000.log"), "x").unwrap();
        prune(&d, "game-", ".log", 9);
        let mut left: Vec<String> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).filter(|n| n.starts_with("game-")).collect();
        left.sort();
        assert_eq!(left.len(), 9);
        assert_eq!(left[0], "game-20260101-000005.log", "the five oldest went");
        assert!(d.join("crash-20260101-000000.txt").exists() && d.join("launcher-20260101-000000.log").exists(), "other files are not touched");
        let _ = std::fs::remove_dir_all(d);
    }

    /// `display.monitor`: 0 is the primary wherever the system lists it, then the others in the system's order.
    #[test]
    fn monitor_numbering() {
        assert_eq!(monitor_order(1, Some(0)), vec![0]);
        assert_eq!(monitor_order(3, Some(0)), vec![0, 1, 2]);
        assert_eq!(monitor_order(3, Some(1)), vec![1, 0, 2]);
        assert_eq!(monitor_order(3, Some(2)), vec![2, 0, 1]);
        // (no primary reported, or one that is not in the list: the first counts)
        assert_eq!(monitor_order(2, None), vec![0, 1]);
        assert_eq!(monitor_order(2, Some(5)), vec![0, 1]);
        assert_eq!(monitor_order(0, None), Vec::<usize>::new());
        // (an index past the end is the caller's fallback to the primary: `chosen_monitor`)
        assert_eq!(monitor_order(2, Some(1)).get(2), None);
    }

    /// Alt+Enter: either Enter with Alt cycles once per press; repeats, synthetic presses and the release are swallowed;
    /// Enter without Alt and other keys with Alt are not Alt+Enter.
    #[test]
    fn alt_enter_events() {
        use winit::keyboard::KeyCode as K;
        assert_eq!(alt_enter_event(true, K::Enter, true, false, false), Some(true));
        assert_eq!(alt_enter_event(true, K::NumpadEnter, true, false, false), Some(true));
        assert_eq!(alt_enter_event(true, K::Enter, true, true, false), Some(false));
        assert_eq!(alt_enter_event(true, K::Enter, true, false, true), Some(false));
        assert_eq!(alt_enter_event(true, K::Enter, false, false, false), Some(false));
        assert_eq!(alt_enter_event(false, K::Enter, true, false, false), None);
        assert_eq!(alt_enter_event(true, K::KeyE, true, false, false), None);
        assert_eq!(alt_enter_event(true, K::AltLeft, true, false, false), None);
        // (the cycle and the reserved key in the shared model)
        assert_eq!([WindowMode::Windowed.next(), WindowMode::Borderless.next(), WindowMode::Fullscreen.next()], [WindowMode::Borderless, WindowMode::Fullscreen, WindowMode::Windowed]);
        assert!(crate::keybinds::reserved(K::Enter) && !crate::keybinds::bindable(K::Enter));
    }

    #[test]
    fn screenshot_names_do_not_collide() {
        let d = tmp("shots");
        let a = screenshot_name(&d, "20261015-120000");
        assert_eq!(a.file_name().unwrap(), "game-20261015-120000.png");
        std::fs::write(&a, "x").unwrap();
        let b = screenshot_name(&d, "20261015-120000");
        assert_eq!(b.file_name().unwrap(), "game-20261015-120000-2.png");
        std::fs::write(&b, "x").unwrap();
        assert_eq!(screenshot_name(&d, "20261015-120000").file_name().unwrap(), "game-20261015-120000-3.png");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn timestamp_has_the_documented_shape() {
        let t = timestamp();
        assert_eq!(t.len(), 15);
        assert_eq!(t.as_bytes()[8], b'-');
        assert!(t.chars().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit()));
    }

    #[test]
    fn crash_report_names_what_a_bug_report_needs() {
        let r = crash_report("main", "boom", "src/x.rs:1:2", "frame 0");
        for want in ["version:", "message:   boom", "place:     src/x.rs:1:2", "adapter:", "---- backtrace ----", "frame 0", "last 50 log lines"] {
            assert!(r.contains(want), "{want} missing in\n{r}");
        }
    }

    #[test]
    fn backtrace_starts_at_the_panic() {
        let bt = "   0: hook\n             at a.rs:1\n   1: rust_begin_unwind\n             at b.rs:2\n   2: core::panicking::panic_fmt\n             at c.rs:3\n   3: mcla::main\n";
        assert_eq!(trim_backtrace(bt), "   2: core::panicking::panic_fmt\n             at c.rs:3\n   3: mcla::main");
        assert_eq!(trim_backtrace("no frames"), "no frames");
    }

    #[test]
    fn intro_modes() {
        // (settings are not loaded in unit tests: the intro is the normal one)
        assert!(intro_film() && !intro_scene());
        // (GitHub #8: OFF keeps the career's opening film - VEHICLE SELECT never comes before it)
        assert!(film_for(Intro::Normal) && !scene_for(Intro::Normal));
        assert!(!film_for(Intro::Fast) && scene_for(Intro::Fast));
        assert!(film_for(Intro::Skip) && !scene_for(Intro::Skip));
    }
}
