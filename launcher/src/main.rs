#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Rusty MCLA launcher shell: window, settings and logs. The page (ui/) holds all layout, text and themes.
//! Nothing here touches the network; the page's content security policy forbids it too.

mod controls;
mod display;
mod install;
mod logging;
mod platform;
mod settings;
mod support;
mod update;

use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Manager, PhysicalPosition, WebviewWindow};

use settings::Settings;

struct AppState {
    settings: Mutex<Settings>,
    user_dir: PathBuf,
    root: PathBuf,
    selftest_out: Option<PathBuf>,
    install: install::Runner,
    update: update::Updater,
    controls: controls::Controls,
    support: support::Support,
    exe: PathBuf,
}

impl AppState {
    /// The settings as they are now, for the support code (which takes them by reference).
    fn snapshot(&self) -> Settings {
        self.settings.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl AppState {
    fn folder(&self, id: &str) -> Option<PathBuf> {
        let s = self.settings.lock().ok()?;
        Some(match id {
            "install" => self.root.clone(),
            "data" => self.root.join("data"),
            "tools" => self.root.join("tools"),
            "screenshots" => match &s.background.folder {
                Some(f) if !f.is_empty() => PathBuf::from(f),
                _ => self.root.join("screenshots"),
            },
            "saves" => self.user_dir.join("saves"),
            "backups" => self.user_dir.join("backups"),
            "logs" => self.user_dir.join("logs"),
            _ => return None,
        })
    }
}

#[derive(Serialize)]
struct Boot {
    version: &'static str,
    os: &'static str,
    arch: &'static str,
    settings: Settings,
    /// false until the install pipeline has written `manifest.local.json` beside the launcher
    installed: bool,
    folders: Value,
    log_file: Option<String>,
    selftest: bool,
    /// Test hook: RUSTY_MCLA_INSTALL_TEST=<iso> makes the page run a whole install and report (see ui/js/install.js `test`)
    install_test: Option<String>,
    install_test_cancel: bool,
    /// Test hook: RUSTY_MCLA_UPDATE_TEST=<steps> makes the page drive the update flow against the mock server and report (ui/js/update.js `test`)
    update_test: Option<String>,
    /// Test hook: RUSTY_MCLA_CONTROLS_TEST=1 makes the page drive the Controls tab and the monitor row and report (ui/js/controls.js `test`)
    controls_test: Option<String>,
    /// Test hook: RUSTY_MCLA_SUPPORT_TEST=<steps> makes the page drive Export Logs, the save manager and Play and report (ui/js/support.js `test`)
    support_test: Option<String>,
}

#[tauri::command]
fn boot(state: tauri::State<AppState>) -> Boot {
    let settings = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let ids = ["install", "data", "saves", "backups", "logs", "screenshots", "tools"];
    let folders: serde_json::Map<String, Value> = ids
        .iter()
        .filter_map(|id| state.folder(id).map(|p| (id.to_string(), json!(p.to_string_lossy()))))
        .collect();
    Boot {
        version: env!("CARGO_PKG_VERSION"),
        os: platform::os_name(),
        arch: std::env::consts::ARCH,
        settings,
        installed: state.root.join("manifest.local.json").is_file(),
        folders: Value::Object(folders),
        log_file: logging::path().map(|p| p.to_string_lossy().into_owned()),
        selftest: state.selftest_out.is_some(),
        install_test: std::env::var("RUSTY_MCLA_INSTALL_TEST").ok().filter(|s| !s.is_empty()),
        install_test_cancel: std::env::var_os("RUSTY_MCLA_INSTALL_TEST_CANCEL").is_some(),
        update_test: std::env::var("RUSTY_MCLA_UPDATE_TEST").ok().filter(|s| !s.is_empty()),
        controls_test: std::env::var("RUSTY_MCLA_CONTROLS_TEST").ok().filter(|s| !s.is_empty()),
        support_test: std::env::var("RUSTY_MCLA_SUPPORT_TEST").ok().filter(|s| !s.is_empty()),
    }
}

/// Merges `patch` into the stored settings and writes the file. Returns the settings as stored.
#[tauri::command]
fn save_settings(state: tauri::State<AppState>, patch: Value) -> Result<Settings, String> {
    let mut guard = state.settings.lock().map_err(|e| e.to_string())?;
    let next = settings::apply_patch(&guard, &patch).map_err(|e| {
        logging::warn(&format!("settings patch refused: {e}"));
        e
    })?;
    settings::save(&state.user_dir, &next).map_err(|e| {
        logging::error(&format!("cannot write launcher.json: {e}"));
        e
    })?;
    *guard = next.clone();
    Ok(next)
}

#[tauri::command]
fn log_line(level: String, msg: String) {
    let level = match level.as_str() {
        "error" => "ERROR",
        "warn" => "WARN",
        _ => "PAGE",
    };
    logging::write(level, &msg);
}

#[tauri::command]
fn open_folder(state: tauri::State<AppState>, id: String) -> Result<(), String> {
    let path = state.folder(&id).ok_or_else(|| format!("unknown folder {id}"))?;
    if !path.is_dir() && matches!(id.as_str(), "saves" | "backups" | "logs") {
        let _ = std::fs::create_dir_all(&path);
    }
    if !path.is_dir() {
        return Err("missing".into());
    }
    logging::info(&format!("open folder {id}"));
    platform::open_folder(&path)
}

#[tauri::command]
fn win_minimize(window: WebviewWindow) {
    let _ = window.minimize();
}

#[tauri::command]
fn win_drag(window: WebviewWindow) {
    let _ = window.start_dragging();
}

#[tauri::command]
fn win_close(window: WebviewWindow) {
    save_window_position(&window);
    logging::info("launcher closed");
    window.app_handle().exit(0);
}

/// Test hook: when RUSTY_MCLA_SELFTEST names a file, the page reports what it measured, the launcher writes it and exits.
#[tauri::command]
fn selftest_report(app: tauri::AppHandle, state: tauri::State<AppState>, text: String) {
    if let Some(out) = &state.selftest_out {
        let _ = std::fs::write(out, text);
    }
    app.exit(0);
}

// ---------- install pipeline (see install.rs) ----------

#[tauri::command]
async fn install_info(state: tauri::State<'_, AppState>) -> Result<install::Info, String> {
    let (runner, root) = (state.install.clone(), state.root.clone());
    tauri::async_runtime::spawn_blocking(move || install::info(&runner, &root)).await.map_err(|e| e.to_string())
}

/// The native file dialog for the disc image. `None` when the user closes it.
#[tauri::command]
async fn pick_iso() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(install::pick_iso).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn validate_iso(state: tauri::State<'_, AppState>, path: String) -> Result<mcla_install::IsoIdentity, install::ErrView> {
    let runner = state.install.clone();
    tauri::async_runtime::spawn_blocking(move || install::validate(&runner, &path))
        .await
        .map_err(|e| install::ErrView { code: "io", message: "The check did not finish.", detail: e.to_string() })?
}

#[tauri::command]
fn start_install(state: tauri::State<AppState>, iso: String, ffmpeg_path: Option<String>) -> Result<(), install::ErrView> {
    install::start(&state.install, &state.root, &state.user_dir.join("logs"), &iso, ffmpeg_path)
}

#[tauri::command]
fn cancel_install(state: tauri::State<AppState>) {
    logging::info("build cancel requested");
    install::cancel(&state.install);
}

#[tauri::command]
fn install_poll(state: tauri::State<AppState>, since: u64) -> install::Poll {
    state.install.poll(since)
}

#[tauri::command]
async fn verify_install(state: tauri::State<'_, AppState>) -> Result<(), install::ErrView> {
    let (runner, root) = (state.install.clone(), state.root.clone());
    tauri::async_runtime::spawn_blocking(move || install::verify(&runner, &root))
        .await
        .map_err(|e| install::ErrView { code: "io", message: "The check did not finish.", detail: e.to_string() })?
}

// ---------- updates (see update.rs) ----------

#[tauri::command]
fn update_status(state: tauri::State<AppState>) -> update::StatusView {
    let s = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
    state.update.status(&s)
}

/// Starts a check in the background; `auto` is the start-up check (does nothing when `check_updates` is off).
#[tauri::command]
fn update_check(state: tauri::State<AppState>, auto: bool) {
    let enabled = state.settings.lock().map(|s| s.check_updates).unwrap_or(true);
    state.update.check(auto, enabled);
}

fn backup_flag(state: &tauri::State<AppState>) -> bool {
    state.settings.lock().map(|s| s.backup_before_update).unwrap_or(true)
}

#[tauri::command]
fn update_now(app: tauri::AppHandle, state: tauri::State<AppState>) {
    let (backup, skipped) = (backup_flag(&state), None);
    state.update.update_now(app, backup, skipped);
}

#[tauri::command]
fn update_setup(state: tauri::State<AppState>, check_first: bool) {
    state.update.setup_game(check_first);
}

#[tauri::command]
fn update_cancel(state: tauri::State<AppState>) {
    state.update.cancel();
}

#[tauri::command]
fn update_clear_result(state: tauri::State<AppState>) {
    state.update.clear_result();
}

#[tauri::command]
async fn update_list(state: tauri::State<'_, AppState>) -> Result<Vec<update::ReleaseRow>, update::ErrorView> {
    let u = state.update.clone();
    u.list().await
}

#[tauri::command]
fn update_install_version(state: tauri::State<AppState>, tag: String) {
    let backup = backup_flag(&state);
    state.update.install_tag(tag, backup);
}

#[tauri::command]
async fn pick_package() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(update::pick_package).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn update_use_package(state: tauri::State<AppState>, path: String) {
    let backup = backup_flag(&state);
    state.update.use_local_package(path, backup);
}

// ---------- controls (see controls.rs) and display settings (display.rs) ----------

#[tauri::command]
fn kbd_get(state: tauri::State<AppState>) -> controls::KbdView {
    state.controls.get()
}

#[tauri::command]
fn kbd_set(state: tauri::State<AppState>, act: String, slot: usize, code: String, alt: bool, force: bool) -> controls::EditResult {
    state.controls.set(&act, slot, &code, alt, force)
}

#[tauri::command]
fn kbd_clear(state: tauri::State<AppState>, act: String, slot: usize) -> controls::EditResult {
    state.controls.clear(&act, slot)
}

#[tauri::command]
fn kbd_reset(state: tauri::State<AppState>) -> controls::EditResult {
    state.controls.reset()
}

#[tauri::command]
fn display_get(window: WebviewWindow) -> display::DisplayView {
    display::view(&window)
}

#[tauri::command]
fn display_set_monitor(window: WebviewWindow, index: u32) -> Result<display::DisplayView, String> {
    display::set_monitor(&window, index)
}

#[tauri::command]
fn display_set(window: WebviewWindow, key: String, value: serde_json::Value) -> Result<display::DisplayView, String> {
    display::set(&window, &key, value)
}

// ---------- support: Export Logs, save manager, Play (see support.rs) ----------

macro_rules! env_of {
    ($state:expr, $s:ident, $env:ident) => {
        let $s = $state.snapshot();
        let $env = support::Env { user_dir: &$state.user_dir, root: &$state.root, exe: &$state.exe, settings: &$s };
    };
}

#[tauri::command]
fn export_info(state: tauri::State<AppState>) -> support::ExportInfo {
    env_of!(state, s, env);
    support::export_info(&env)
}

/// The folder dialog for "Change"; `None` when the user closes it. Nothing is stored until `export_set_dir`.
#[tauri::command]
async fn export_pick_dir(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    let start = {
        env_of!(state, s, env);
        support::export_dir(&env)
    };
    tauri::async_runtime::spawn_blocking(move || support::pick_dir("Where should the logs zip go?", &start)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn export_set_dir(state: tauri::State<AppState>, dir: Option<String>) -> Result<(), String> {
    let mut guard = state.settings.lock().map_err(|e| e.to_string())?;
    let next = settings::apply_patch(&guard, &json!({ "export_dir": dir.filter(|d| !d.is_empty()) }))?;
    settings::save(&state.user_dir, &next)?;
    *guard = next;
    Ok(())
}

fn system_context(state: &AppState, window: &WebviewWindow) -> mcla_support::sysinfo::Context {
    env_of!(state, s, env);
    state.support.context(&env, support::display_lines(window), tauri::webview_version().ok())
}

#[derive(Serialize)]
struct ExportView {
    dir: String,
    #[serde(flatten)]
    done: mcla_support::export::Done,
}

#[tauri::command]
async fn export_logs(window: WebviewWindow, state: tauri::State<'_, AppState>, include: mcla_support::export::Include) -> Result<ExportView, mcla_support::Error> {
    let ctx = system_context(&state, &window);
    let app = window.app_handle().clone();
    let shots = state.folder("screenshots").unwrap_or_else(|| state.root.join("screenshots"));
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        env_of!(state, s, env);
        state.support.export(&env, &ctx, &include, &shots, None).map(|d| ExportView { dir: Path::new(&d.path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(), done: d })
    })
    .await
    .map_err(|e| mcla_support::Error::new("io", e.to_string()))?
}

#[tauri::command]
fn export_reveal(state: tauri::State<AppState>) -> Result<(), String> {
    state.support.reveal_export()
}

#[tauri::command]
async fn report_url(window: WebviewWindow, state: tauri::State<'_, AppState>) -> Result<String, mcla_support::Error> {
    let base = state.update.issues_base();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        env_of!(state, s, env);
        let os = state.support.os_line();
        state.support.report_url(&env, &base, &os)
    })
    .await
    .map_err(|e| mcla_support::Error::new("io", e.to_string()))?
}

#[tauri::command]
async fn system_summary(window: WebviewWindow, state: tauri::State<'_, AppState>) -> Result<mcla_support::sysinfo::Summary, String> {
    let ctx = system_context(&state, &window);
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().support.summary(&ctx)).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn system_text(window: WebviewWindow, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let ctx = system_context(&state, &window);
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().support.system_text(&ctx)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn save_view(state: tauri::State<AppState>) -> support::SaveView {
    env_of!(state, s, env);
    support::save_view(&env)
}

#[tauri::command]
fn save_backup_now(state: tauri::State<AppState>) -> Result<support::SaveView, mcla_support::Error> {
    env_of!(state, s, env);
    match mcla_support::saves::backup_now(&state.user_dir, support::keep(&env)) {
        Ok(Some(b)) => logging::info(&format!("save backed up by hand: {} ({} bytes)", b.file, b.size)),
        Ok(None) => return Err(mcla_support::Error::new("no_save", "there is no save to back up")),
        Err(e) => {
            logging::error(&format!("save backup failed: {e}"));
            return Err(e);
        }
    }
    Ok(support::save_view(&env))
}

#[tauri::command]
fn save_restore(state: tauri::State<AppState>, file: String, force: bool) -> Result<support::SaveView, mcla_support::Error> {
    env_of!(state, s, env);
    logging::info(&format!("RESTORE pressed for {file} (force {force})"));
    match mcla_support::saves::restore(&state.user_dir, &state.root, &file, support::keep(&env), force) {
        Ok(pre) => logging::info(&format!("save restored from {file}; the replaced save is {:?}", pre.map(|b| b.file))),
        Err(e) => {
            logging::warn(&format!("restore refused: {e}"));
            return Err(e);
        }
    }
    Ok(support::save_view(&env))
}

#[tauri::command]
async fn save_import_pick() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(support::pick_save_file).await.map_err(|e| e.to_string())
}

/// What IMPORT would do with the chosen file (a save of this game as it is, or a save of the original game converted), before the user confirms.
#[tauri::command]
fn save_import_inspect(state: tauri::State<AppState>, path: String) -> Result<mcla_support::saves::Inspection, mcla_support::Error> {
    logging::info(&format!("IMPORT A SAVE: looking at {path}"));
    match mcla_support::saves::inspect(&state.user_dir, &state.root, Path::new(&path)) {
        Ok(i) => {
            logging::info(&format!("import preview: {} {:?}", i.kind, i.converted));
            Ok(i)
        }
        Err(e) => {
            logging::warn(&format!("import preview refused: {e}"));
            Err(e)
        }
    }
}

#[tauri::command]
fn save_import(state: tauri::State<AppState>, path: String, force: bool) -> Result<support::ImportView, mcla_support::Error> {
    env_of!(state, s, env);
    logging::info(&format!("IMPORT A SAVE: {path} (force {force})"));
    let done = match mcla_support::saves::import(&state.user_dir, &state.root, Path::new(&path), support::keep(&env), force) {
        Ok(done) => {
            logging::info(&format!("save imported; converted {:?}; the replaced save is {:?}", done.converted, done.backup.as_ref().map(|b| &b.file)));
            done
        }
        Err(e) => {
            logging::warn(&format!("import refused: {e}"));
            return Err(e);
        }
    };
    Ok(support::ImportView { view: support::save_view(&env), converted: done.converted })
}

#[tauri::command]
fn save_set_keep(state: tauri::State<AppState>, n: u32) -> Result<support::SaveView, String> {
    let n = n.clamp(1, 100);
    {
        let mut guard = state.settings.lock().map_err(|e| e.to_string())?;
        let next = settings::apply_patch(&guard, &json!({ "backups_keep": n }))?;
        settings::save(&state.user_dir, &next)?;
        *guard = next;
    }
    state.update.set_keep(n as usize);
    env_of!(state, s, env);
    Ok(support::save_view(&env))
}

#[tauri::command]
fn play_info(state: tauri::State<AppState>) -> support::PlayInfo {
    env_of!(state, s, env);
    support::play_info(&env, state.update.is_updating())
}

#[tauri::command]
async fn play(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<support::PlayOk, support::PlayError> {
    let handle = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        env_of!(state, s, env);
        state.support.play(&env, state.update.is_updating())
    })
    .await
    .map_err(|e| support::PlayError { code: "io", detail: e.to_string(), exit_code: None })?;
    if let Ok(ok) = &r {
        // a self-test run keeps the launcher alive long enough to write its report
        if ok.closing && state.selftest_out.is_none() {
            logging::info("the game started; closing the launcher (Settings > General)");
            if let Some(w) = app.get_webview_window("main") {
                save_window_position(&w);
            }
            app.exit(0);
        }
    }
    r
}

#[tauri::command]
fn crash_notice(state: tauri::State<AppState>) -> Option<mcla_support::launch::CrashNotice> {
    mcla_support::launch::pending_crash(&state.user_dir)
}

#[tauri::command]
fn crash_dismiss(state: tauri::State<AppState>) {
    logging::info("crash notice dismissed");
    mcla_support::launch::dismiss_crash(&state.user_dir);
}

#[tauri::command]
fn go_back_previous(state: tauri::State<AppState>, force: bool) -> Result<support::PlayInfo, mcla_support::Error> {
    env_of!(state, s, env);
    support::go_back(&env, state.update.is_updating(), force)?;
    Ok(support::play_info(&env, state.update.is_updating()))
}

/// Opens an http(s) link (release notes links, "Full notes on GitHub") in the system browser; nothing else is ever opened.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    update::open_url(&url)
}

fn save_window_position(window: &WebviewWindow) {
    let state = window.state::<AppState>();
    let Ok(pos) = window.outer_position() else { return };
    let remember = state.settings.lock().map(|s| s.remember_window).unwrap_or(false);
    if !remember || window.is_minimized().unwrap_or(false) {
        return;
    }
    let patch = json!({ "window": { "x": pos.x, "y": pos.y } });
    let next = state.settings.lock().ok().and_then(|g| settings::apply_patch(&g, &patch).ok());
    if let Some(next) = next {
        if settings::save(&state.user_dir, &next).is_ok() {
            if let Ok(mut g) = state.settings.lock() {
                *g = next;
            }
        }
    }
}

/// Puts the window back where it was, but only if that place is still on a screen.
fn restore_window_position(window: &WebviewWindow, s: &Settings) {
    if !s.remember_window {
        return;
    }
    let (Some(x), Some(y)) = (s.window.x, s.window.y) else { return };
    let Ok(monitors) = window.available_monitors() else { return };
    let (px, py) = (x + 120, y + 30); // a point inside the title bar
    let visible = monitors.iter().any(|m| {
        let (mp, ms) = (m.position(), m.size());
        px >= mp.x && px < mp.x + ms.width as i32 && py >= mp.y && py < mp.y + ms.height as i32
    });
    if visible {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    } else {
        logging::info("saved window position is off screen, keeping it centred");
    }
}

fn main() {
    let user_dir = platform::user_data_dir();
    let log_path = logging::init(&user_dir.join("logs"));
    logging::install_bridge();
    let root = platform::portable_root();
    let updated = std::env::args().any(|a| a == "--updated");
    let exe = std::env::current_exe().unwrap_or_else(|_| root.join("RustyMCLA.exe"));
    let settings = settings::load(&user_dir);
    logging::info(&format!(
        "Rusty MCLA launcher {} on {} {}",
        env!("CARGO_PKG_VERSION"),
        platform::os_name(),
        std::env::consts::ARCH
    ));
    logging::info(&format!("log file {}", log_path.display()));
    logging::info(&format!("portable root {}", root.display()));
    logging::info(&format!("settings theme={} language={}", settings.theme, settings.language));

    let selftest_out = std::env::var_os("RUSTY_MCLA_SELFTEST").map(PathBuf::from);
    let updater = update::Updater::new(&user_dir, &root, &exe);
    if updated {
        logging::info(&format!("started after a self-update: launcher {}", updater.running_version()));
    }
    updater.startup_cleanup(updated);
    let state = AppState { settings: Mutex::new(settings.clone()), user_dir, root, selftest_out, install: install::Runner::new(updater.clone()), update: updater, controls: controls::Controls::new(&platform::user_data_dir()), support: support::Support::default(), exe };

    let result = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            boot,
            save_settings,
            log_line,
            open_folder,
            win_minimize,
            win_drag,
            win_close,
            selftest_report,
            install_info,
            pick_iso,
            validate_iso,
            start_install,
            cancel_install,
            install_poll,
            verify_install,
            update_status,
            update_check,
            update_now,
            update_setup,
            update_cancel,
            update_clear_result,
            update_list,
            update_install_version,
            pick_package,
            update_use_package,
            open_url,
            kbd_get,
            kbd_set,
            kbd_clear,
            kbd_reset,
            display_get,
            display_set_monitor,
            display_set,
            export_info,
            export_pick_dir,
            export_set_dir,
            export_logs,
            export_reveal,
            report_url,
            system_summary,
            system_text,
            save_view,
            save_backup_now,
            save_restore,
            save_import_pick,
            save_import_inspect,
            save_import,
            save_set_keep,
            play_info,
            play,
            crash_notice,
            crash_dismiss,
            go_back_previous
        ])
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                restore_window_position(&window, &settings);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(w) = window.app_handle().get_webview_window(window.label()) {
                    save_window_position(&w);
                }
                logging::info("launcher closed");
            }
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        logging::error(&format!("launcher failed to run: {e}"));
        std::process::exit(1);
    }
    logging::info("launcher closed");
}
