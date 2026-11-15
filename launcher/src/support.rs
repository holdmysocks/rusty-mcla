//! Tauri commands of the support features: Export Logs, the save manager, Play and the crash notice, the previous game version.
//! The work is in the `mcla-support` crate (no Tauri, unit-tested); this file owns the state that lives while the launcher runs
//! and turns the results into what the page draws. Every sentence the page shows comes from the locales (`dyn.sup.*`, `dyn.play.*`).

use mcla_support::export::{self, Include};
use mcla_support::launch;
use mcla_support::redact::Redactor;
use mcla_support::saves;
use mcla_support::sysinfo;
use mcla_support::Error;
use mcla_updater::backup::BackupEntry;
use mcla_updater::package;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::logging;
use crate::settings::Settings;

#[derive(Default)]
pub struct Support {
    last_export: Mutex<Option<PathBuf>>,
    hardware: Mutex<Option<sysinfo::Hardware>>,
    /// The game process the launcher started and still watches (only while the launcher stays open).
    watching: Mutex<Option<u32>>,
}

pub struct Env<'a> {
    pub user_dir: &'a Path,
    pub root: &'a Path,
    pub exe: &'a Path,
    pub settings: &'a Settings,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------- Export Logs ----------

#[derive(Serialize)]
pub struct ExportInfo {
    pub dir: String,
    pub default_dir: String,
    pub custom: bool,
}

pub fn export_dir(env: &Env) -> PathBuf {
    match env.settings.export_dir.as_deref().filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => export::default_dir(env.user_dir),
    }
}

pub fn export_info(env: &Env) -> ExportInfo {
    ExportInfo { dir: export_dir(env).to_string_lossy().into_owned(), default_dir: export::default_dir(env.user_dir).to_string_lossy().into_owned(), custom: env.settings.export_dir.as_deref().map(|d| !d.is_empty()).unwrap_or(false) }
}

pub fn pick_dir(title: &str, start: &Path) -> Option<String> {
    rfd::FileDialog::new().set_title(title).set_directory(start).pick_folder().map(|p| p.to_string_lossy().into_owned())
}

pub fn pick_save_file() -> Option<String> {
    rfd::FileDialog::new().set_title("Choose a save").add_filter("Saves", &["json", "sav"]).add_filter("All files", &["*"]).pick_file().map(|p| p.to_string_lossy().into_owned())
}

impl Support {
    fn hardware(&self) -> sysinfo::Hardware {
        if let Some(h) = lock(&self.hardware).clone() {
            return h;
        }
        let h = sysinfo::hardware();
        *lock(&self.hardware) = Some(h.clone());
        h
    }

    pub fn context(&self, env: &Env, displays: Vec<String>, webview: Option<String>) -> sysinfo::Context {
        sysinfo::Context { launcher_version: env!("CARGO_PKG_VERSION").into(), game_version: package::read_record(env.root).map(|r| r.version), webview, displays, root: env.root.to_path_buf(), user_dir: env.user_dir.to_path_buf() }
    }

    /// The system text, redacted (for the COPY button and the export).
    pub fn system_text(&self, ctx: &sysinfo::Context) -> String {
        Redactor::current().text(&sysinfo::render(&self.hardware(), ctx))
    }

    pub fn summary(&self, ctx: &sysinfo::Context) -> sysinfo::Summary {
        let s = sysinfo::summary(&self.hardware(), ctx);
        let r = Redactor::current();
        sysinfo::Summary { windows: r.text(&s.windows), graphics: r.text(&s.graphics), ..s }
    }

    pub fn export(&self, env: &Env, ctx: &sysinfo::Context, include: &Include, shots: &Path, dir_override: Option<&Path>) -> Result<export::Done, Error> {
        logging::info(&format!("export logs requested: {include:?}"));
        let redactor = Redactor::current();
        let system = if include.system { redactor.text(&sysinfo::render(&self.hardware(), ctx)) } else { String::new() };
        let dir = dir_override.map(Path::to_path_buf).unwrap_or_else(|| export_dir(env));
        let done = export::export(&export::Request { user_dir: env.user_dir, root: env.root, screenshots_dir: shots, launcher_exe: env.exe, launcher_version: env!("CARGO_PKG_VERSION"), system_text: &system, include, out_dir: &dir, redactor: &redactor });
        match &done {
            Ok(d) => {
                *lock(&self.last_export) = Some(PathBuf::from(&d.path));
                logging::info(&format!("export logs written: {} ({} bytes, {} files, {} names redacted, notes: {:?})", d.path, d.size, d.entries.len(), d.redactions, d.notes));
            }
            Err(e) => logging::error(&format!("export logs failed: {e}")),
        }
        done
    }

    pub fn reveal_export(&self) -> Result<(), String> {
        let p = lock(&self.last_export).clone().ok_or_else(|| "no export yet".to_string())?;
        if !p.is_file() {
            return Err("missing".into());
        }
        crate::platform::reveal_file(&p)
    }

    pub fn report_url(&self, env: &Env, issues_base: &str, os: &str) -> Result<String, Error> {
        let rec = package::read_record(env.root);
        export::issue_url(issues_base, env!("CARGO_PKG_VERSION"), rec.as_ref().map(|r| r.version.as_str()), os, rec.as_ref().map(|r| r.data_version), &Redactor::current())
    }

    pub fn os_line(&self) -> String {
        self.hardware().os
    }
}

// ---------- save manager ----------

#[derive(Serialize)]
pub struct SaveView {
    pub status: saves::Status,
    pub backups: Vec<BackupEntry>,
}

/// The answer of IMPORT: the save manager's view, plus what was converted when the file was a save of the original game.
#[derive(Serialize)]
pub struct ImportView {
    #[serde(flatten)]
    pub view: SaveView,
    pub converted: Option<saves::ConvertedInfo>,
}

pub fn keep(env: &Env) -> usize {
    (env.settings.backups_keep as usize).clamp(1, 100)
}

pub fn save_view(env: &Env) -> SaveView {
    SaveView { status: saves::status(env.user_dir, env.root, keep(env)), backups: mcla_updater::backup::list(env.user_dir) }
}

// ---------- Play ----------

#[derive(Serialize)]
pub struct PlayInfo {
    /// manifest.local.json exists: the data has been built
    pub installed: bool,
    /// game/mcla.exe exists
    pub exe: bool,
    pub updating: bool,
    pub game_running: bool,
    pub game_version: Option<String>,
    pub close_on_launch: bool,
    pub previous: bool,
    pub previous_version: Option<String>,
}

pub fn play_info(env: &Env, updating: bool) -> PlayInfo {
    let prev = env.root.join(package::OLD_DIR);
    PlayInfo {
        installed: mcla_install::layout::Layout::new(env.root).is_complete(),
        exe: launch::game_exe(env.root).is_file(),
        updating,
        game_running: saves::game_running(env.root),
        game_version: package::read_record(env.root).map(|r| r.version),
        close_on_launch: env.settings.close_on_launch,
        previous: prev.is_dir(),
        previous_version: std::fs::read_to_string(prev.join("package.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v.get("version").and_then(|x| x.as_str()).map(String::from)),
    }
}

#[derive(Serialize)]
pub struct PlayError {
    pub code: &'static str,
    pub detail: String,
    /// The exit code, when the game ended inside the watch.
    pub exit_code: Option<i32>,
}

impl From<Error> for PlayError {
    fn from(e: Error) -> PlayError {
        PlayError { code: e.code, detail: e.detail, exit_code: None }
    }
}

#[derive(Serialize)]
pub struct PlayOk {
    pub pid: u32,
    /// The launcher is closing now (Settings > General) - or would, in a self-test.
    pub closing: bool,
    /// What the game was started with, for the log and the checks.
    pub args: Vec<String>,
}

impl Support {
    /// Starts the game and watches it for a few seconds. Blocking: run it on a worker thread.
    pub fn play(&self, env: &Env, updating: bool) -> Result<PlayOk, PlayError> {
        if updating {
            return Err(Error::new("updating", "an update is being applied").into());
        }
        if saves::game_running(env.root) {
            return Err(Error::new("already_running", "the game is already running").into());
        }
        let plan = launch::plan(env.root, env.user_dir)?;
        logging::info(&format!("PLAY: {} {}", plan.exe.display(), plan.args.join(" ")));
        let mut child = launch::spawn(&plan)?;
        let pid = child.id();
        launch::record_start(env.user_dir, pid);
        match launch::watch(&mut child, launch::WATCH) {
            launch::Outcome::Exited { code, millis } if code != Some(0) => {
                launch::record_exit(env.user_dir, pid, code);
                logging::error(&format!("the game ended after {millis} ms with exit code {code:?}"));
                return Err(PlayError { code: "exited_early", detail: format!("exit code {}, after {millis} ms", code.map(|c| c.to_string()).unwrap_or_else(|| "none".into())), exit_code: code });
            }
            launch::Outcome::Exited { code, millis } => {
                launch::record_exit(env.user_dir, pid, code);
                logging::info(&format!("the game ended by itself after {millis} ms with exit code 0"));
            }
            launch::Outcome::Running => logging::info(&format!("the game is running (process {pid})")),
        }
        // started: the previous game version is no longer needed
        package::cleanup_after_start(env.root);
        let closing = env.settings.close_on_launch;
        if !closing {
            *lock(&self.watching) = Some(pid);
            let user_dir = env.user_dir.to_path_buf();
            std::thread::spawn(move || {
                let code = child.wait().ok().and_then(|s| s.code());
                launch::record_exit(&user_dir, pid, code);
                logging::info(&format!("the game ended with exit code {code:?}"));
            });
        }
        Ok(PlayOk { pid, closing, args: plan.args })
    }
}

// ---------- the previous game version ----------

/// "Go back to the previous game version": the save is backed up first, then `game.old/` becomes `game/`.
pub fn go_back(env: &Env, updating: bool, force: bool) -> Result<(), Error> {
    if updating {
        return Err(Error::new("updating", "an update is being applied"));
    }
    if !env.root.join(package::OLD_DIR).is_dir() {
        return Err(Error::new("no_previous", "there is no previous game version"));
    }
    if !force && saves::game_running(env.root) {
        return Err(Error::new("game_running", "the game is running"));
    }
    // a failing backup stops everything: the old game must not meet a save it cannot read
    match mcla_updater::backup::backup_save(env.user_dir, "before-restore", keep(env)) {
        Ok(Some(p)) => logging::info(&format!("go back: save backed up to {}", p.display())),
        Ok(None) => logging::info("go back: there is no save to back up"),
        Err(e) => return Err(Error::new("io", format!("could not back up the save, nothing was changed: {}", e.detail))),
    }
    package::restore_previous(env.root).map_err(|e| Error::new("io", e.detail))?;
    // restore_previous keeps the replaced folder as game.bad/; nobody needs a 170 MB copy of the version the user just left
    let bad = env.root.join("game.bad");
    if bad.exists() {
        match std::fs::remove_dir_all(&bad) {
            Ok(()) => logging::info("go back: removed game.bad/"),
            Err(e) => logging::warn(&format!("go back: could not remove game.bad/: {e}")),
        }
    }
    logging::info("go back: the previous game version is back in game/");
    Ok(())
}

/// Monitors for the system text.
pub fn display_lines(window: &tauri::WebviewWindow) -> Vec<String> {
    let primary = window.primary_monitor().ok().flatten().map(|m| (m.position().x, m.position().y));
    let mut v = Vec::new();
    let mut mons: Vec<tauri::Monitor> = window.available_monitors().unwrap_or_default();
    // the primary one first, as the game numbers them
    if let Some(p) = primary {
        mons.sort_by_key(|m| (m.position().x, m.position().y) != p);
    }
    for (i, m) in mons.iter().enumerate() {
        let name = m.name().map(|n| n.trim_start_matches(r"\\.\").to_string()).unwrap_or_else(|| "display".into());
        let is_primary = Some((m.position().x, m.position().y)) == primary;
        v.push(format!("{}. {} {}x{} at {},{} scale {:.0}%{}", i + 1, name, m.size().width, m.size().height, m.position().x, m.position().y, m.scale_factor() * 100.0, if is_primary { " (main)" } else { "" }));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let (root, user) = (d.path().join("app"), d.path().join("user"));
        for (dir, text) in [("game", "NEW"), ("game.old", "OLD")] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("mcla.exe"), text).unwrap();
            std::fs::write(root.join(dir).join("package.json"), format!("{{\"version\":\"{}\"}}", if text == "NEW" { "0.2.0" } else { "0.1.0" })).unwrap();
        }
        std::fs::create_dir_all(user.join("saves")).unwrap();
        std::fs::write(user.join("saves").join("mcla_save.json"), "{\"car\":\"x\",\"money\":1}").unwrap();
        (d, root, user)
    }

    #[test]
    fn going_back_backs_up_the_save_swaps_the_folders_and_cleans_up() {
        let (_d, root, user) = stage();
        std::fs::create_dir_all(root.join("game.bad")).unwrap(); // a leftover of an earlier go-back
        let s = Settings::default();
        let env = Env { user_dir: &user, root: &root, exe: &root.join("RustyMCLA.exe"), settings: &s };
        let before = play_info(&env, false);
        assert!(before.previous && before.previous_version.as_deref() == Some("0.1.0"));
        go_back(&env, false, false).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("game").join("mcla.exe")).unwrap(), "OLD");
        assert!(!root.join("game.old").exists() && !root.join("game.bad").exists());
        let b = mcla_updater::backup::list(&user);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].reason, "before-restore");
        assert_eq!(std::fs::read_to_string(user.join("backups").join(&b[0].file)).unwrap(), "{\"car\":\"x\",\"money\":1}");
        assert!(!play_info(&env, false).previous);
    }

    #[test]
    fn going_back_refuses_while_updating_without_a_previous_version_and_when_the_backup_fails() {
        let (_d, root, user) = stage();
        let s = Settings::default();
        let env = Env { user_dir: &user, root: &root, exe: &root.join("RustyMCLA.exe"), settings: &s };
        assert_eq!(go_back(&env, true, false).unwrap_err().code, "updating");
        // a file where the backups folder should be: the backup fails and nothing may change
        std::fs::write(user.join("backups"), "in the way").unwrap();
        assert_eq!(go_back(&env, false, false).unwrap_err().code, "io");
        assert_eq!(std::fs::read_to_string(root.join("game").join("mcla.exe")).unwrap(), "NEW");
        assert!(root.join("game.old").is_dir());
        std::fs::remove_file(user.join("backups")).unwrap();
        std::fs::remove_dir_all(root.join("game.old")).unwrap();
        assert_eq!(go_back(&env, false, false).unwrap_err().code, "no_previous");
    }

    #[test]
    fn play_info_tells_what_is_missing() {
        let d = tempfile::tempdir().unwrap();
        let s = Settings::default();
        let env = Env { user_dir: d.path(), root: d.path(), exe: &d.path().join("RustyMCLA.exe"), settings: &s };
        let i = play_info(&env, false);
        assert!(!i.installed && !i.exe && !i.previous && i.game_version.is_none() && i.close_on_launch);
        assert!(play_info(&env, true).updating);
        let p = Support::default().play(&env, false).err().unwrap();
        assert_eq!(p.code, "missing_exe");
        assert_eq!(Support::default().play(&env, true).err().unwrap().code, "updating");
    }
}
