//! `%LOCALAPPDATA%\RustyMCLA\launcher.json`: theme, language, window state, skipped version and the launcher's own switches.
//!
//! The file is versioned (`schema`). Unknown keys are kept and written back, so an older launcher does not drop what a
//! newer one stored. A file that cannot be read is moved aside (`launcher.json.bad`) and defaults are used.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::logging;

pub const SCHEMA: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct WindowState {
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// the size the user gave the window (logical pixels, the content inside is scaled to it)
    pub w: Option<u32>,
    pub h: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Background {
    pub rotate: bool,
    pub interval_s: u32,
    /// None = the `screenshots` folder beside the launcher
    pub folder: Option<String>,
}

impl Default for Background {
    fn default() -> Self {
        Background { rotate: true, interval_s: 10, folder: None }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub schema: u32,
    /// theme id; see ui/js/themes.js (`steel` is the default, `system` follows the OS)
    pub theme: String,
    /// "auto" = the OS language on first run, else a code from ui/locales
    pub language: String,
    pub open_on: String,
    pub check_updates: bool,
    pub close_on_launch: bool,
    pub backup_before_update: bool,
    /// how many save backups the save manager keeps (the newest ones)
    pub backups_keep: u32,
    /// where Export Logs writes its zip; `None` = the Desktop
    pub export_dir: Option<String>,
    pub show_notes_after_update: bool,
    pub remember_window: bool,
    /// the release (version, no leading v) the user chose to skip; its banner stays hidden until a newer release appears
    pub skipped_version: Option<String>,
    pub window: WindowState,
    pub background: Background,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            schema: SCHEMA,
            theme: "steel".into(),
            language: "auto".into(),
            open_on: "home".into(),
            check_updates: true,
            close_on_launch: true,
            backup_before_update: true,
            backups_keep: 10,
            export_dir: None,
            show_notes_after_update: true,
            remember_window: true,
            skipped_version: None,
            window: WindowState::default(),
            background: Background::default(),
            extra: Map::new(),
        }
    }
}

pub fn file_path(user_dir: &Path) -> PathBuf {
    user_dir.join("launcher.json")
}

pub fn load(user_dir: &Path) -> Settings {
    let path = file_path(user_dir);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            logging::info("launcher.json not found, using defaults");
            return Settings::default();
        }
        Err(e) => {
            logging::error(&format!("cannot read launcher.json: {e}; using defaults"));
            return Settings::default();
        }
    };
    match serde_json::from_str::<Settings>(&text) {
        Ok(s) => s,
        Err(e) => {
            logging::error(&format!("launcher.json is not valid ({e}); moved to launcher.json.bad, using defaults"));
            let _ = fs::rename(&path, path.with_extension("json.bad"));
            Settings::default()
        }
    }
}

/// Writes through a temporary file and a rename, so a crash never leaves half a file.
pub fn save(user_dir: &Path, s: &Settings) -> Result<(), String> {
    fs::create_dir_all(user_dir).map_err(|e| e.to_string())?;
    let path = file_path(user_dir);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Merges `patch` (objects merge key by key, everything else replaces) into `s`. A patch that does not fit the
/// schema is refused and `s` is left as it was.
pub fn apply_patch(s: &Settings, patch: &Value) -> Result<Settings, String> {
    let mut v = serde_json::to_value(s).map_err(|e| e.to_string())?;
    merge(&mut v, patch);
    let mut next: Settings = serde_json::from_value(v).map_err(|e| e.to_string())?;
    next.schema = SCHEMA;
    Ok(next)
}

fn merge(dst: &mut Value, patch: &Value) {
    match (dst, patch) {
        (Value::Object(d), Value::Object(p)) => {
            for (k, pv) in p {
                match d.get_mut(k) {
                    Some(dv) => merge(dv, pv),
                    None => {
                        d.insert(k.clone(), pv.clone());
                    }
                }
            }
        }
        (d, p) => *d = p.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patch_merges_nested_and_keeps_unknown_keys() {
        let mut s = Settings::default();
        s.extra.insert("future_key".into(), json!(7));
        let next = apply_patch(&s, &json!({ "theme": "dracula", "window": { "x": 10 } })).unwrap();
        assert_eq!(next.theme, "dracula");
        assert_eq!(next.window.x, Some(10));
        assert_eq!(next.extra.get("future_key"), Some(&json!(7)));
    }

    #[test]
    fn patch_with_wrong_type_is_refused() {
        let s = Settings::default();
        assert!(apply_patch(&s, &json!({ "check_updates": "yes" })).is_err());
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let s: Settings = serde_json::from_str(r#"{"theme":"teal","from_newer":{"a":1}}"#).unwrap();
        let back = serde_json::to_string(&s).unwrap();
        assert!(back.contains("from_newer"));
        assert_eq!(s.theme, "teal");
        assert!(s.check_updates);
    }
}
