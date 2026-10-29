//! The save manager: what the current save is, backing it up, restoring a backup and importing a save file.
//!
//! The save is one JSON file, `<user dir>/saves/mcla_save.json` (DECISIONS.md D16). Backups are `<user dir>/backups/mcla_save-<time>-<reason>.json`
//! (`mcla_updater::backup`). Every operation that replaces the current save makes a backup of it first and stops when that fails
//! (`mcla_updater::backup::replace_save`), and writes the new bytes atomically.
//!
//! **Import accepts**
//! * a save of this port (`mcla_save.json`, or one of the backups, or a file from another install): a JSON object with the members the game writes;
//! * a save of the **original game**, which it converts (`mcla_savedecode`, the Rust port of `tools/import_mc4_save.py`): an Xbox 360 `mc4.sav`
//!   (Xenia's file too; it is not encrypted) or a PS3 save: the plain `RAGE.SAV`, or the save folder / any file in it (`PARAM.PFD`, `RAGE.SAV`, ...), which
//!   is decrypted first (`mcla_savedecode::ps3`, the US release's keys, LAUNCHER.md decision 18). The conversion keeps the members of the current save
//!   that the original has no counterpart of (key bindings, ...) and needs the installed game data (its race list names the races);
//! * an Xbox 360 **STFS package** (`CON ` / `LIVE` / `PIRS`, the file the console keeps; signed, not encrypted): `mc4.sav` is taken out of it with
//!   `mcla_savedecode::stfs` and converted like a plain one.
//!
//! **Refused, each with its own message:** a PS3 save that cannot be decrypted (no `PARAM.PFD` beside it, or another game / region), a package that
//! holds no `mc4.sav` or cannot be read, anything unreadable or unknown.

use crate::{Error, Result};
use mcla_updater::backup::{self, BackupEntry, SAVE_FILE};
use mcla_savedecode as savedecode;
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Members the game's save has; a file needs at least two of them to count as a save of this port.
const KNOWN_MEMBERS: &[&str] = &["car", "career", "money", "owned", "cfg", "opts", "mc4", "best_time", "rep", "perf", "color"];
const MAX_SAVE: u64 = 64 << 20;

pub fn save_path(user_dir: &Path) -> PathBuf {
    user_dir.join("saves").join(SAVE_FILE)
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub exists: bool,
    pub size: u64,
    /// Unix seconds of the last write of the save file ("last played").
    pub modified: i64,
    pub path: String,
    pub game_running: bool,
    pub keep: usize,
    pub backup_count: usize,
    pub last_backup: Option<BackupEntry>,
}

/// True while the game's exe is running: Windows refuses to open a running program for writing (sharing violation), which is a check that
/// needs no process list and no extra rights. Always false where that signal does not exist.
pub fn game_running(root: &Path) -> bool {
    let exe = root.join("game").join("mcla.exe");
    if !exe.is_file() {
        return false;
    }
    match std::fs::OpenOptions::new().write(true).open(&exe) {
        Ok(_) => false,
        Err(e) => cfg!(windows) && matches!(e.raw_os_error(), Some(32) | Some(5)),
    }
}

pub fn status(user_dir: &Path, root: &Path, keep: usize) -> Status {
    let p = save_path(user_dir);
    let md = std::fs::metadata(&p).ok().filter(|m| m.is_file());
    let list = backup::list(user_dir);
    Status {
        exists: md.is_some(),
        size: md.as_ref().map(|m| m.len()).unwrap_or(0),
        modified: md.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0),
        path: p.to_string_lossy().into_owned(),
        game_running: game_running(root),
        keep,
        backup_count: list.len(),
        last_backup: list.into_iter().next(),
    }
}

fn from_update(e: mcla_updater::UpdateError) -> Error {
    Error::new(if e.kind == mcla_updater::Kind::Missing { "missing" } else { "io" }, e.detail)
}

/// BACK UP NOW. `Ok(None)` = there is no save to back up.
pub fn backup_now(user_dir: &Path, keep: usize) -> Result<Option<BackupEntry>> {
    let made = backup::backup_save(user_dir, "manual", keep).map_err(from_update)?;
    Ok(made.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).and_then(|f| backup::list(user_dir).into_iter().find(|b| b.file == f)))
}

fn check_game(root: &Path, force: bool) -> Result<()> {
    if !force && game_running(root) {
        return Err(Error::new("game_running", "the game is running"));
    }
    Ok(())
}

/// Reads a candidate save and checks that it is one: a JSON object with the members of a save of this port. Returns the bytes to write
/// (a byte order mark removed).
pub fn validate_save_bytes(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Err(Error::new("empty", "the file is empty"));
    }
    let body = data.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(data);
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| Error::new("not_a_save", format!("not JSON: {e}")))?;
    let Some(o) = v.as_object() else { return Err(Error::new("not_a_save", "the JSON is not an object")) };
    let n = KNOWN_MEMBERS.iter().filter(|k| o.contains_key(**k)).count();
    if n < 2 {
        return Err(Error::new("not_a_save", format!("only {n} of the members a save has were found")));
    }
    Ok(body.to_vec())
}

/// RESTORE: the current save is backed up first (`before-restore`), then replaced by the backup's bytes.
pub fn restore(user_dir: &Path, root: &Path, file: &str, keep: usize, force: bool) -> Result<Option<BackupEntry>> {
    check_game(root, force)?;
    let p = backup::path_of(user_dir, file).map_err(from_update)?;
    let data = std::fs::read(&p).map_err(|e| Error::io(p.display(), &e))?;
    let bytes = validate_save_bytes(&data).map_err(|e| Error::new("backup_damaged", e.detail))?;
    let made = backup::replace_save(user_dir, &bytes, "before-restore", keep).map_err(from_update)?;
    Ok(made.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).and_then(|f| backup::list(user_dir).into_iter().find(|b| b.file == f)))
}

/// What a file is, for the refusal messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    PortSave,
    OriginalXbox360,
    XboxPackage,
    Ps3,
    Folder,
    Unknown,
}

/// Looks at the first bytes (and the name) of a candidate. Never reads more than a few KB, so it also names big files that are refused.
pub fn classify(path: &Path) -> Kind {
    if path.is_dir() {
        return if path.join("PARAM.PFD").is_file() || path.join("RAGE.SAV").is_file() { Kind::Ps3 } else { Kind::Folder };
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
    let mut head = [0u8; 64];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"CON ") || head.starts_with(b"LIVE") || head.starts_with(b"PIRS") {
        return Kind::XboxPackage;
    }
    // the plain 360 save (and a decrypted PS3 one) start 00 00 00 0F 00 00 00 09 00 00 00 05 "Stat"
    if head.len() >= 16 && head[..12] == [0, 0, 0, 0x0F, 0, 0, 0, 9, 0, 0, 0, 5] && &head[12..16] == b"Stat" {
        return Kind::OriginalXbox360;
    }
    if (head.len() >= 8 && &head[4..8] == b"PFDB") || matches!(name.as_str(), "RAGE.SAV" | "PARAM.PFD" | "PARAM.SFO" | "ICON0.PNG") {
        return Kind::Ps3;
    }
    if name == "MC4.SAV" || name.ends_with(".SAV") {
        return Kind::OriginalXbox360;
    }
    if head.first().map(|b| matches!(b, b'{' | 0xEF)).unwrap_or(false) {
        return Kind::PortSave;
    }
    Kind::Unknown
}

/// The biggest file read as a save of the original game (a real one is 537,428 bytes).
const MAX_ORIGINAL: u64 = 8 << 20;
/// The game's race list inside the data folder; the converter names the races of the career from it.
pub const RACE_TABLE: &str = "tune/race/0x0f1f4d8d";

/// What the conversion of a save of the original game carried over, for the confirmation text and the log.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConvertedInfo {
    /// `xbox360` (container version 14) or `ps3` (15).
    pub platform: &'static str,
    /// The file was an encrypted PS3 save and has been decrypted on the way.
    pub decrypted: bool,
    /// The save was taken out of an Xbox 360 console package (STFS: `CON `, `LIVE`, `PIRS`) on the way.
    pub from_package: bool,
    pub cars: usize,
    pub current_car: String,
    pub money: u32,
    pub reputation: Option<u32>,
    pub missions: usize,
    pub missions_complete: usize,
    pub statistics: usize,
    pub collectables: usize,
    pub editor_races: usize,
    pub favourite_races: usize,
}

impl ConvertedInfo {
    fn new(s: &savedecode::Summary, decrypted: bool, from_package: bool) -> ConvertedInfo {
        ConvertedInfo {
            // a console package is an Xbox 360 file whatever the save inside says
            platform: if s.container_version == 15 && !from_package { "ps3" } else { "xbox360" },
            decrypted,
            from_package,
            cars: s.vehicles.len(),
            current_car: s.current_vehicle.clone(),
            money: s.money,
            reputation: s.reputation,
            missions: s.missions,
            missions_complete: s.missions_complete,
            statistics: s.statistics,
            collectables: s.collectables,
            editor_races: s.user_races,
            favourite_races: s.favourite_races,
        }
    }
}

/// What a chosen file is, before anything is replaced: the answer to "what will IMPORT do with this?".
#[derive(Debug, Clone, Serialize)]
pub struct Inspection {
    /// `port` (a save of this game, taken as it is) or `original` (converted).
    pub kind: &'static str,
    pub converted: Option<ConvertedInfo>,
    /// a port save's mod cars (`mod_cars`, docs/MODS.md S3), by their last known long names: they come over with the save as
    /// they are, and wait as placeholders in the garage while their mod is not installed
    pub mod_cars: Vec<String>,
    /// the same cars with the car name and the id of their mod, for the Mods tab to mark which are installed (docs/MODS.md S5)
    pub mod_refs: Vec<ModCarRef>,
}

/// A mod car a port save names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModCarRef {
    /// the car name the game and the save use
    pub car: String,
    pub mod_id: String,
    /// the last known long name (the car name when the save has none)
    pub title: String,
}

/// The mod cars a port save names (its `mod_cars` member: car name -> { mod_id, long_name, short_name }), long names first.
pub fn mod_cars_of(save: &[u8]) -> Vec<String> {
    mod_car_refs_of(save).into_iter().map(|r| r.title).collect()
}

/// [`mod_cars_of`] with the car name and mod id of each.
pub fn mod_car_refs_of(save: &[u8]) -> Vec<ModCarRef> {
    let v: serde_json::Value = match serde_json::from_slice(save) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(m) = v.get("mod_cars").and_then(|m| m.as_object()) else { return Vec::new() };
    m.iter()
        .map(|(car, e)| ModCarRef {
            car: car.clone(),
            mod_id: e.get("mod_id").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
            title: e.get("long_name").and_then(|n| n.as_str()).filter(|n| !n.trim().is_empty()).unwrap_or(car).to_string(),
        })
        .collect()
}

/// The result of an import.
#[derive(Debug, Clone)]
pub struct ImportDone {
    /// The backup of the save that was replaced (None when there was no save).
    pub backup: Option<BackupEntry>,
    pub converted: Option<ConvertedInfo>,
}

enum Source {
    Port(PathBuf),
    /// The bytes of a plain save of the original game (decrypted when it came from a PS3 folder, `package` when taken out of an Xbox 360 package).
    Original { bytes: Vec<u8>, decrypted: bool, package: bool },
}

fn read_limited(p: &Path) -> Result<Vec<u8>> {
    let len = std::fs::metadata(p).map_err(|e| Error::io(p.display(), &e))?.len();
    if len > MAX_ORIGINAL {
        return Err(Error::new("too_big", format!("{len} bytes")));
    }
    std::fs::read(p).map_err(|e| Error::io(p.display(), &e))
}

/// A file of `dir` by name, without regard to case (PS3 folders are copied around by tools of every kind).
fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_file() && p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false))
}

/// A PS3 save: `path` is the save folder or a file inside it. A plain `RAGE.SAV` is taken as it is; a protected one is decrypted with the keys
/// of `mcla_savedecode::ps3`, which needs `PARAM.PFD` beside it.
fn resolve_ps3(path: &Path) -> Result<Source> {
    let dir = if path.is_dir() { path.to_path_buf() } else { path.parent().map(Path::to_path_buf).unwrap_or_default() };
    let pfd = find_ci(&dir, "PARAM.PFD");
    let mut names = vec!["RAGE.SAV".to_string()];
    if let Some(list) = pfd.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| savedecode::ps3::protected_files(&b).ok()) {
        names.extend(list.into_iter().filter(|n| !n.eq_ignore_ascii_case("RAGE.SAV") && !n.to_ascii_uppercase().starts_with("PARAM.")));
    }
    let mut last: Option<&'static str> = None;
    for name in names {
        let Some(file) = find_ci(&dir, &name) else { continue };
        if is_plain_original(&file) {
            return Ok(Source::Original { bytes: read_limited(&file)?, decrypted: false, package: false });
        }
        let Some(pfd) = &pfd else { continue };
        let pfd_bytes = std::fs::read(pfd).map_err(|e| Error::io(pfd.display(), &e))?;
        match savedecode::ps3::decrypt(&pfd_bytes, &name, &read_limited(&file)?) {
            Ok(bytes) if savedecode::looks_like_original(&bytes) => return Ok(Source::Original { bytes, decrypted: true, package: false }),
            Ok(_) => last = Some("key"),
            Err(e) => last = Some(e.code),
        }
    }
    // "key": a database and a save file that do not belong together or to this game (another region's id, a damaged file)
    if last == Some("key") {
        return Err(Error::new("ps3_key", format!("{} could not be decrypted", dir.display())));
    }
    Err(Error::new("ps3", format!("{} is a PS3 save without a readable PARAM.PFD or without its save file", dir.display())))
}

/// The biggest Xbox 360 package read (a real one holds the 537,428-byte save in about 0.7 MB).
const MAX_PACKAGE: u64 = 32 << 20;
/// The save's name inside the console package.
pub const PACKAGE_SAVE: &str = "mc4.sav";

/// An Xbox 360 STFS package (signed, not encrypted): `mc4.sav` is taken out of it by `mcla_savedecode::stfs`, then converted like a plain one.
fn resolve_xbox_package(path: &Path) -> Result<Source> {
    let len = std::fs::metadata(path).map_err(|e| Error::io(path.display(), &e))?.len();
    if len > MAX_PACKAGE {
        return Err(Error::new("too_big", format!("{len} bytes")));
    }
    let package = std::fs::read(path).map_err(|e| Error::io(path.display(), &e))?;
    match savedecode::stfs::extract(&package, PACKAGE_SAVE, MAX_ORIGINAL) {
        Ok(bytes) => Ok(Source::Original { bytes, decrypted: false, package: true }),
        Err(e) => Err(Error::new(
            match e.code {
                "stfs_no_file" => "xbox_package_no_save",
                "stfs_too_big" => "too_big",
                _ => "xbox_package_damaged",
            },
            format!("{}: {e}", path.display()),
        )),
    }
}

/// Decides what the file is and where the save is.
fn resolve(path: &Path) -> Result<Source> {
    match classify(path) {
        Kind::PortSave => Ok(Source::Port(path.to_path_buf())),
        Kind::OriginalXbox360 => Ok(Source::Original { bytes: read_limited(path)?, decrypted: false, package: false }),
        Kind::Ps3 => resolve_ps3(path),
        Kind::XboxPackage => resolve_xbox_package(path),
        Kind::Folder => Err(Error::new("not_a_save", format!("{} is a folder without a save", path.display()))),
        Kind::Unknown => Err(Error::new("not_a_save", format!("{} is not a save", path.display()))),
    }
}

fn is_plain_original(path: &Path) -> bool {
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    savedecode::looks_like_original(&head[..n])
}

fn race_list(data_dir: &Path) -> Result<Vec<String>> {
    let p = data_dir.join(RACE_TABLE);
    let bytes = std::fs::read(&p).map_err(|e| Error::new("no_data", format!("{}: {e}", p.display())))?;
    let list = savedecode::race_table(&bytes);
    if list.is_empty() {
        return Err(Error::new("no_data", format!("{} holds no races", p.display())));
    }
    Ok(list)
}

/// The current save as the base of a conversion: its members that the original has no counterpart of survive. No save, or one that is not a
/// JSON object (it is backed up before it is replaced), gives an empty base.
fn current_base(user_dir: &Path) -> serde_json::Map<String, serde_json::Value> {
    let parsed = std::fs::read(save_path(user_dir)).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&b)).ok());
    match parsed {
        Some(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    }
}

/// Converts a save of the original game. Returns the bytes of the new save and what was converted.
fn convert_original(user_dir: &Path, data_dir: &Path, data: &[u8], decrypted: bool, package: bool) -> Result<(Vec<u8>, ConvertedInfo)> {
    savedecode::validate(data).map_err(|e| Error::new("original_damaged", e.to_string()))?;
    let races = race_list(data_dir)?;
    let done = savedecode::convert(data, current_base(user_dir), &races).map_err(|e| Error::new("original_damaged", e.to_string()))?;
    let info = ConvertedInfo::new(&done.summary, decrypted, package);
    let bytes = serde_json::to_vec(&serde_json::Value::Object(done.save)).map_err(|e| Error::new("original_damaged", e.to_string()))?;
    // the result must pass the same test as any save that is imported
    validate_save_bytes(&bytes)?;
    Ok((bytes, info))
}

fn read_port_save(p: &Path) -> Result<Vec<u8>> {
    let len = std::fs::metadata(p).map_err(|e| Error::io(p.display(), &e))?.len();
    if len > MAX_SAVE {
        return Err(Error::new("too_big", format!("{len} bytes")));
    }
    validate_save_bytes(&std::fs::read(p).map_err(|e| Error::io(p.display(), &e))?)
}

/// Looks at a chosen file and tells what IMPORT would do with it; a save of the original game is converted in memory to prove that it can be
/// read. Nothing is written. `data_dir` = the game's data folder.
pub fn inspect_in(user_dir: &Path, data_dir: &Path, path: &Path) -> Result<Inspection> {
    match resolve(path)? {
        Source::Port(p) => {
            let bytes = read_port_save(&p)?;
            Ok(Inspection { kind: "port", converted: None, mod_cars: mod_cars_of(&bytes), mod_refs: mod_car_refs_of(&bytes) })
        }
        Source::Original { bytes, decrypted, package } => Ok(Inspection { kind: "original", converted: Some(convert_original(user_dir, data_dir, &bytes, decrypted, package)?.1), mod_cars: Vec::new(), mod_refs: Vec::new() }),
    }
}

/// `inspect_in` with the data folder the game would use.
pub fn inspect(user_dir: &Path, root: &Path, path: &Path) -> Result<Inspection> {
    inspect_in(user_dir, &crate::launch::data_folder(root, user_dir), path)
}

/// IMPORT: a save of this port is taken as it is, a save of the original game is converted (see the module comment). The current save is backed
/// up first (`before-import`); the new one is written through a temporary file.
pub fn import_in(user_dir: &Path, root: &Path, data_dir: &Path, path: &Path, keep: usize, force: bool) -> Result<ImportDone> {
    let (bytes, converted) = match resolve(path)? {
        Source::Port(p) => (read_port_save(&p)?, None),
        Source::Original { bytes, decrypted, package } => {
            let (b, info) = convert_original(user_dir, data_dir, &bytes, decrypted, package)?;
            (b, Some(info))
        }
    };
    // importing the file that already is the current save would only make a pointless backup
    if let Ok(cur) = std::fs::canonicalize(save_path(user_dir)) {
        if std::fs::canonicalize(path).map(|p| p == cur).unwrap_or(false) {
            return Err(Error::new("same_file", "that is the current save"));
        }
    }
    check_game(root, force)?;
    let made = backup::replace_save(user_dir, &bytes, "before-import", keep).map_err(from_update)?;
    let backup = made.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).and_then(|f| backup::list(user_dir).into_iter().find(|b| b.file == f));
    Ok(ImportDone { backup, converted })
}

/// `import_in` with the data folder the game would use.
pub fn import(user_dir: &Path, root: &Path, path: &Path, keep: usize, force: bool) -> Result<ImportDone> {
    import_in(user_dir, root, &crate::launch::data_folder(root, user_dir), path, keep, force)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256_file;

    const SAVE: &str = r#"{"car":"vp_x","money":1234,"career":{"a":1},"mc4":{}}"#;

    fn setup(save: Option<&str>) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let (user, root) = (d.path().join("user"), d.path().join("app"));
        std::fs::create_dir_all(&root).unwrap();
        if let Some(s) = save {
            std::fs::create_dir_all(user.join("saves")).unwrap();
            std::fs::write(save_path(&user), s).unwrap();
        }
        (d, user, root)
    }

    fn hash(p: &Path) -> String {
        sha256_file(p).unwrap()
    }

    #[test]
    fn status_reports_size_time_and_backups() {
        let (_d, user, root) = setup(Some(SAVE));
        let s = status(&user, &root, 10);
        assert!(s.exists && s.size == SAVE.len() as u64 && s.modified > 1_700_000_000 && s.backup_count == 0 && s.last_backup.is_none() && !s.game_running);
        backup_now(&user, 10).unwrap().unwrap();
        let s = status(&user, &root, 10);
        assert_eq!(s.backup_count, 1);
        assert_eq!(s.last_backup.unwrap().reason, "manual");
        let none = status(&user.join("nowhere"), &root, 10);
        assert!(!none.exists && none.size == 0);
    }

    #[test]
    fn backup_restore_round_trip_keeps_the_bytes() {
        let (_d, user, root) = setup(Some(SAVE));
        let before = hash(&save_path(&user));
        let b = backup_now(&user, 10).unwrap().unwrap();
        // the game plays on and changes the save
        std::fs::write(save_path(&user), r#"{"car":"vp_y","money":9,"career":{}}"#).unwrap();
        let changed = hash(&save_path(&user));
        assert_ne!(before, changed);
        let pre = restore(&user, &root, &b.file, 10, false).unwrap().unwrap();
        assert_eq!(hash(&save_path(&user)), before, "restored bytes differ");
        assert_eq!(pre.reason, "before-restore");
        // the save that was replaced is in the backup made first
        assert_eq!(hash(&user.join("backups").join(&pre.file)), changed);
        assert!(!user.join("saves").join("mcla_save.json.part").exists());
    }

    #[test]
    fn restore_refuses_odd_names_and_damaged_backups() {
        let (_d, user, root) = setup(Some(SAVE));
        assert_eq!(restore(&user, &root, "..\\saves\\mcla_save.json", 10, false).unwrap_err().code, "missing");
        assert_eq!(restore(&user, &root, "mcla_save-20200101-000000-manual.json", 10, false).unwrap_err().code, "missing");
        std::fs::create_dir_all(user.join("backups")).unwrap();
        std::fs::write(user.join("backups").join("mcla_save-20200101-000000-manual.json"), "{ half a file").unwrap();
        let before = hash(&save_path(&user));
        assert_eq!(restore(&user, &root, "mcla_save-20200101-000000-manual.json", 10, false).unwrap_err().code, "backup_damaged");
        assert_eq!(hash(&save_path(&user)), before);
        assert_eq!(backup::list(&user).len(), 1, "a refused restore must not make a backup");
    }

    #[test]
    fn import_a_port_save_backs_up_first() {
        let (d, user, root) = setup(Some(SAVE));
        let other = d.path().join("friend.json");
        let incoming = "\u{feff}{\"car\":\"vp_z\",\"money\":77,\"opts\":{}}";
        std::fs::write(&other, incoming).unwrap();
        let cur = hash(&save_path(&user));
        let b = import(&user, &root, &other, 10, false).unwrap().backup.unwrap();
        assert_eq!(b.reason, "before-import");
        assert_eq!(hash(&user.join("backups").join(&b.file)), cur);
        // the BOM is dropped, the rest is the same bytes
        assert_eq!(std::fs::read_to_string(save_path(&user)).unwrap(), incoming.trim_start_matches('\u{feff}'));
        // with no save yet it just creates the file
        let (d2, user2, root2) = setup(None);
        let f = d2.path().join("s.json");
        std::fs::write(&f, SAVE).unwrap();
        assert!(import(&user2, &root2, &f, 10, false).unwrap().backup.is_none());
        assert_eq!(hash(&save_path(&user2)), hash(&f));
    }

    /// docs/MODS.md S3: a port save with mod cars and a garage past the original's 30 slots (a missing-mod placeholder
    /// among them) is imported byte for byte, and IMPORT's inspection names its mod cars.
    #[test]
    fn a_port_save_with_mod_cars_imports_whole() {
        let (d, user, root) = setup(Some(SAVE));
        let mut owned: Vec<String> = (0..33).map(|k| format!("vp_disc_car_{k:02}")).collect();
        owned.insert(5, "vp_test_mod_01".into());
        owned.push("vp_gone_car_08".into());
        let save = serde_json::json!({
            "car": "vp_test_mod_01", "money": 5, "owned": owned,
            "mc4": { "garage": owned.iter().enumerate().map(|(k, c)| serde_json::json!({ "slot": k, "vehicle": c, "TypeName": c })).collect::<Vec<_>>() },
            "mod_cars": {
                "vp_gone_car_08": { "mod_id": "gone", "long_name": "", "short_name": "" },
                "vp_test_mod_01": { "mod_id": "test.mod", "long_name": "2006 TEST MOD", "short_name": "06 TEST" }
            }
        });
        let f = d.path().join("modded.json");
        std::fs::write(&f, serde_json::to_vec_pretty(&save).unwrap()).unwrap();
        let i = inspect_in(&user, &d.path().join("nodata"), &f).unwrap();
        assert_eq!((i.kind, i.mod_cars.clone()), ("port", vec!["vp_gone_car_08".to_string(), "2006 TEST MOD".to_string()]));
        assert_eq!(i.mod_refs, vec![ModCarRef { car: "vp_gone_car_08".into(), mod_id: "gone".into(), title: "vp_gone_car_08".into() }, ModCarRef { car: "vp_test_mod_01".into(), mod_id: "test.mod".into(), title: "2006 TEST MOD".into() }]);
        import(&user, &root, &f, 10, false).unwrap();
        assert_eq!(hash(&save_path(&user)), hash(&f), "the save as it is: every mod-car record and the 35-car garage");
        assert!(mod_cars_of(SAVE.as_bytes()).is_empty());
    }

    #[test]
    fn files_that_are_not_readable_saves_are_refused_without_touching_anything() {
        let (d, user, root) = setup(Some(SAVE));
        let before = hash(&save_path(&user));
        let mut xbox = vec![0, 0, 0, 0x0F, 0, 0, 0, 9, 0, 0, 0, 5];
        xbox.extend_from_slice(b"Stat");
        xbox.extend_from_slice(&[0u8; 200]);
        let f1 = d.path().join("mc4.sav");
        std::fs::write(&f1, &xbox).unwrap();
        let f2 = d.path().join("RAGE.SAV");
        std::fs::write(&f2, [1u8; 100]).unwrap();
        let f3 = d.path().join("pkg");
        std::fs::write(&f3, b"CON \0\0\0\0").unwrap();
        let ps3 = d.path().join("BLES00001");
        std::fs::create_dir_all(&ps3).unwrap();
        std::fs::write(ps3.join("PARAM.PFD"), [0u8; 16]).unwrap();
        let junk = d.path().join("notes.txt");
        std::fs::write(&junk, "hello").unwrap();
        let empty = d.path().join("e.json");
        std::fs::write(&empty, "").unwrap();
        let notasave = d.path().join("other.json");
        std::fs::write(&notasave, r#"{"theme":"steel"}"#).unwrap();
        let empty_dir = d.path().join("emptydir");
        std::fs::create_dir_all(&empty_dir).unwrap();
        let arr = d.path().join("a.json");
        std::fs::write(&arr, "[1,2]").unwrap();
        for (f, code) in [(&f1, "original_damaged"), (&f2, "ps3"), (&f3, "xbox_package_damaged"), (&ps3, "ps3"), (&junk, "not_a_save"), (&empty, "not_a_save"), (&notasave, "not_a_save"), (&arr, "not_a_save"), (&empty_dir, "not_a_save")] {
            let e = import(&user, &root, f, 10, false).unwrap_err();
            assert_eq!(e.code, code, "{}: {}", f.display(), e.detail);
        }
        assert_eq!(hash(&save_path(&user)), before);
        assert!(backup::list(&user).is_empty(), "a refused import must not make a backup");
    }

    #[test]
    fn importing_the_current_save_is_refused() {
        let (_d, user, root) = setup(Some(SAVE));
        let e = import(&user, &root, &save_path(&user), 10, false).unwrap_err();
        assert_eq!(e.code, "same_file");
    }

    #[test]
    fn a_failed_backup_stops_the_replacement() {
        let (d, user, root) = setup(Some(SAVE));
        // a file where the backups folder should be: the backup cannot be made
        std::fs::write(user.join("backups"), "in the way").unwrap();
        let f = d.path().join("s.json");
        std::fs::write(&f, r#"{"car":"a","money":1}"#).unwrap();
        let before = hash(&save_path(&user));
        assert!(import(&user, &root, &f, 10, false).is_err());
        assert_eq!(hash(&save_path(&user)), before);
    }

    #[test]
    #[cfg(windows)]
    fn a_running_program_counts_as_the_game_running() {
        let (d, _user, root) = setup(None);
        // a real running exe stands in for the game: a copy of this test program, started and kept alive
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        std::fs::copy(sys.join("System32").join("ping.exe"), game.join("mcla.exe")).unwrap();
        assert!(!game_running(&root));
        let mut child = std::process::Command::new(game.join("mcla.exe")).args(["-n", "6", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(game_running(&root), "a running exe must be detected");
        let f = d.path().join("s.json");
        std::fs::write(&f, SAVE).unwrap();
        let (_d2, user2, _) = setup(Some(SAVE));
        assert_eq!(import(&user2, &root, &f, 10, false).unwrap_err().code, "game_running");
        assert!(import(&user2, &root, &f, 10, true).is_ok(), "force goes ahead");
        let _ = child.kill();
        let _ = child.wait();
        assert!(!game_running(&root));
    }

    // ---------- saves of the original game ----------

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// A data folder holding the game's race list: a copy of the extracted one when this disk has it, else a tiny stand-in.
    fn data_dir(d: &Path) -> PathBuf {
        let data = d.join("data");
        let table = data.join(RACE_TABLE);
        std::fs::create_dir_all(table.parent().unwrap()).unwrap();
        let real = repo().join("extracted/cache").join(RACE_TABLE);
        if real.is_file() {
            std::fs::copy(&real, &table).unwrap();
        } else {
            std::fs::write(&table, "Id game/Race/a\nId game/Race/b\n").unwrap();
        }
        data
    }

    fn blank(version: u32) -> Vec<u8> {
        savedecode::testing::blank_save(version)
    }

    #[test]
    fn a_save_of_the_original_game_is_converted_backed_up_first_and_keeps_what_only_this_game_has() {
        let (d, user, root) = setup(Some(r#"{"car":"vp_x","money":1234,"career":{"a":1},"keyboard":{"gas":"W"},"mc4":{}}"#));
        let data = data_dir(d.path());
        let f = d.path().join("mc4.sav");
        std::fs::write(&f, blank(14)).unwrap();
        let cur = hash(&save_path(&user));
        let done = import_in(&user, &root, &data, &f, 10, false).unwrap();
        let b = done.backup.expect("the old save is backed up");
        assert_eq!(b.reason, "before-import");
        assert_eq!(hash(&user.join("backups").join(&b.file)), cur, "the backup is the save that was replaced");
        let info = done.converted.expect("a conversion is reported");
        assert_eq!((info.platform, info.cars, info.money), ("xbox360", 0, 0));
        let now: serde_json::Value = serde_json::from_slice(&std::fs::read(save_path(&user)).unwrap()).unwrap();
        assert_eq!(now["keyboard"]["gas"], "W", "key bindings survive");
        assert_eq!(now["money"], 0);
        assert_eq!(now["mc4"]["container_version"], 14);
        assert_eq!(now["career"]["a"], 1);
        assert!(!user.join("saves").join("mcla_save.json.part").exists());
        // with no save at all it just creates the file
        let (d2, user2, root2) = setup(None);
        let data2 = data_dir(d2.path());
        let f2 = d2.path().join("RAGE.SAV");
        std::fs::write(&f2, blank(15)).unwrap();
        let done = import_in(&user2, &root2, &data2, &f2, 10, false).unwrap();
        assert!(done.backup.is_none() && done.converted.unwrap().platform == "ps3");
        assert!(save_path(&user2).is_file());
    }

    #[test]
    fn a_missing_game_data_folder_stops_the_conversion_before_anything_changes() {
        let (d, user, root) = setup(Some(SAVE));
        let f = d.path().join("mc4.sav");
        std::fs::write(&f, blank(14)).unwrap();
        let before = hash(&save_path(&user));
        let e = import_in(&user, &root, &d.path().join("no-such-data"), &f, 10, false).unwrap_err();
        assert_eq!(e.code, "no_data");
        assert_eq!(inspect_in(&user, &d.path().join("no-such-data"), &f).unwrap_err().code, "no_data");
        assert_eq!(hash(&save_path(&user)), before);
        assert!(backup::list(&user).is_empty());
    }

    #[test]
    fn a_ps3_save_is_decrypted_with_its_database_and_refused_without_or_with_the_wrong_one() {
        use mcla_savedecode::ps3::testing::protect;
        let (d, user, root) = setup(Some(SAVE));
        let data = data_dir(d.path());
        let (pfd, enc) = protect("RAGE.SAV", &blank(15), [9u8; 16]);
        // 1. the whole folder, a file inside it (any), the lower-case spelling a copying tool may give
        let prot = d.path().join("BLUS30190MC4_SAV");
        std::fs::create_dir_all(&prot).unwrap();
        std::fs::write(prot.join("PARAM.PFD"), &pfd).unwrap();
        std::fs::write(prot.join("param.sfo"), b"PSF").unwrap();
        std::fs::write(prot.join("rage.sav"), &enc).unwrap();
        for choice in [prot.clone(), prot.join("rage.sav"), prot.join("PARAM.PFD")] {
            let (d2, user2, root2) = setup(Some(SAVE));
            let data2 = data_dir(d2.path());
            let info = inspect_in(&user2, &data2, &choice).unwrap().converted.unwrap();
            assert!(info.decrypted && info.platform == "ps3", "{}", choice.display());
            let done = import_in(&user2, &root2, &data2, &choice, 10, false).unwrap();
            assert!(done.converted.unwrap().decrypted && done.backup.unwrap().reason == "before-import");
        }
        // 2. refused, with the save and the backups untouched: no database beside the file, a database that is not one, another game's file
        let alone = d.path().join("alone");
        std::fs::create_dir_all(&alone).unwrap();
        std::fs::write(alone.join("RAGE.SAV"), &enc).unwrap();
        let junk_db = d.path().join("junkdb");
        std::fs::create_dir_all(&junk_db).unwrap();
        std::fs::write(junk_db.join("PARAM.PFD"), [0u8; 64]).unwrap();
        std::fs::write(junk_db.join("RAGE.SAV"), (0..4096u32).map(|i| (i * 7 + 3) as u8).collect::<Vec<u8>>()).unwrap();
        let other_game = d.path().join("othergame");
        std::fs::create_dir_all(&other_game).unwrap();
        std::fs::write(other_game.join("PARAM.PFD"), &pfd).unwrap();
        let mut wrong = enc.clone();
        wrong[5] ^= 0x40;
        std::fs::write(other_game.join("RAGE.SAV"), &wrong).unwrap();
        let before = hash(&save_path(&user));
        for (what, code) in [(&alone.join("RAGE.SAV"), "ps3"), (&alone, "ps3"), (&junk_db, "ps3"), (&junk_db.join("RAGE.SAV"), "ps3"), (&other_game, "ps3_key")] {
            assert_eq!(import_in(&user, &root, &data, what, 10, false).unwrap_err().code, code, "{}", what.display());
        }
        assert_eq!(hash(&save_path(&user)), before);
        assert!(backup::list(&user).is_empty(), "a refused import must not make a backup");
        // 3. a plain RAGE.SAV in a PS3 folder (already decrypted by another tool) needs no database
        let plain = d.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("PARAM.PFD"), [0u8; 64]).unwrap();
        std::fs::write(plain.join("RAGE.SAV"), blank(15)).unwrap();
        let done = import_in(&user, &root, &data, &plain, 10, false).unwrap();
        assert!(!done.converted.unwrap().decrypted);
        assert_eq!(import_in(&user, &root, &data, &plain.join("RAGE.SAV"), 10, false).unwrap().converted.unwrap().platform, "ps3");
    }

    #[test]
    fn an_xbox_360_package_gives_up_its_save_which_is_converted_like_a_plain_one() {
        use mcla_savedecode::stfs::testing::{build, File, Options};
        let (d, user, root) = setup(Some(r#"{"car":"vp_x","money":1234,"keyboard":{"gas":"W"}}"#));
        let data = data_dir(d.path());
        let plain = blank(14);
        let cur = hash(&save_path(&user));
        // the console's own kinds of package, the save chained or in a plain run, behind a filler so that it crosses the hash tables
        for (magic, male, consecutive) in [(*b"CON ", true, true), (*b"CON ", true, false), (*b"LIVE", false, true), (*b"PIRS", false, false)] {
            let pkg = build(&[File::new("filler", &vec![7u8; 200_000], true), File::new("MC4.SAV", &plain, consecutive)], &Options { magic, male, ..Options::default() });
            let f = d.path().join("save.bin");
            std::fs::write(&f, &pkg).unwrap();
            let (d2, user2, root2) = setup(Some(r#"{"car":"vp_x","money":1234,"keyboard":{"gas":"W"}}"#));
            let data2 = data_dir(d2.path());
            let info = inspect_in(&user2, &data2, &f).unwrap();
            let c = info.converted.unwrap();
            assert!(info.kind == "original" && c.platform == "xbox360" && c.from_package && !c.decrypted, "{magic:?}");
            let done = import_in(&user2, &root2, &data2, &f, 10, false).unwrap();
            assert_eq!(done.backup.unwrap().reason, "before-import");
            let now: serde_json::Value = serde_json::from_slice(&std::fs::read(save_path(&user2)).unwrap()).unwrap();
            assert_eq!((now["keyboard"]["gas"].as_str(), now["mc4"]["container_version"].as_u64()), (Some("W"), Some(14)));
        }
        // the same save inside a package gives the same result as the plain file
        let pkg = build(&[File::new("mc4.sav", &plain, true)], &Options::default());
        let fp = d.path().join("pkg.bin");
        std::fs::write(&fp, &pkg).unwrap();
        let fs = d.path().join("mc4.sav");
        std::fs::write(&fs, &plain).unwrap();
        let (_d3, user3, root3) = setup(Some(SAVE));
        let (_d4, user4, root4) = setup(Some(SAVE));
        import_in(&user3, &root3, &data, &fp, 10, false).unwrap();
        import_in(&user4, &root4, &data, &fs, 10, false).unwrap();
        assert_eq!(hash(&save_path(&user3)), hash(&save_path(&user4)));
        // a version-15 save inside a package is still reported as an Xbox 360 file
        let p15 = d.path().join("p15.bin");
        std::fs::write(&p15, build(&[File::new("mc4.sav", &blank(15), true)], &Options::default())).unwrap();
        assert_eq!(inspect_in(&user, &data, &p15).unwrap().converted.unwrap().platform, "xbox360");
        // refusals, each with its own code and nothing changed: no mc4.sav in it, a save that is not one, cut off, a header only, a package of nothing
        let other = d.path().join("other.bin");
        std::fs::write(&other, build(&[File::new("profile.dat", b"x", true)], &Options::default())).unwrap();
        let junk = d.path().join("junk.bin");
        std::fs::write(&junk, build(&[File::new("mc4.sav", &[9u8; 3000], true)], &Options::default())).unwrap();
        let cut = d.path().join("cut.bin");
        std::fs::write(&cut, &pkg[..pkg.len() / 2]).unwrap();
        let head = d.path().join("head.bin");
        std::fs::write(&head, &pkg[..0x400]).unwrap();
        for (f, code) in [(&other, "xbox_package_no_save"), (&junk, "original_damaged"), (&cut, "xbox_package_damaged"), (&head, "xbox_package_damaged")] {
            assert_eq!(inspect_in(&user, &data, f).unwrap_err().code, code, "{}", f.display());
            assert_eq!(import_in(&user, &root, &data, f, 10, false).unwrap_err().code, code, "{}", f.display());
        }
        assert_eq!(hash(&save_path(&user)), cur);
        assert!(backup::list(&user).is_empty(), "a refused import must not make a backup");
    }

    #[test]
    fn inspect_tells_what_import_would_do_and_writes_nothing() {
        let (d, user, _root) = setup(Some(SAVE));
        let data = data_dir(d.path());
        let port = d.path().join("friend.json");
        std::fs::write(&port, r#"{"car":"vp_z","money":77,"opts":{}}"#).unwrap();
        let orig = d.path().join("mc4.sav");
        std::fs::write(&orig, blank(14)).unwrap();
        let junk = d.path().join("x.txt");
        std::fs::write(&junk, "hello").unwrap();
        let before = hash(&save_path(&user));
        let p = inspect_in(&user, &data, &port).unwrap();
        assert!(p.kind == "port" && p.converted.is_none());
        let o = inspect_in(&user, &data, &orig).unwrap();
        assert!(o.kind == "original" && o.converted.as_ref().unwrap().platform == "xbox360");
        assert_eq!(inspect_in(&user, &data, &junk).unwrap_err().code, "not_a_save");
        assert_eq!(hash(&save_path(&user)), before);
        assert!(backup::list(&user).is_empty());
    }

    /// The user's real saves (copies in `work/launcher-saveimport/in`) through the whole import, against the Python importer's output. Skipped
    /// when those files are not on this disk. The real save in %LOCALAPPDATA% is never touched: everything happens in a temporary folder.
    #[test]
    fn the_real_saves_import_to_what_the_python_importer_wrote() {
        let fx = repo().join("work/launcher-saveimport");
        let table = repo().join("extracted/cache").join(RACE_TABLE);
        if !fx.join("in/full_v15.sav").is_file() || !table.is_file() || !fx.join("in/base_real_copy.json").is_file() {
            eprintln!("skipped: the real saves are not on this disk");
            return;
        }
        let mut done = 0;
        for name in ["xenia_current_v14", "full_v15", "newgame_v14", "recomp_trace_v14", "ps3_RAGE_dec"] {
            let (d, user, root) = setup(None);
            let data = data_dir(d.path());
            std::fs::create_dir_all(user.join("saves")).unwrap();
            std::fs::copy(fx.join("in/base_real_copy.json"), save_path(&user)).unwrap();
            let base_hash = hash(&save_path(&user));
            let sav = d.path().join(format!("{name}.sav"));
            std::fs::copy(fx.join(format!("in/{name}.sav")), &sav).unwrap();
            let done_ = import_in(&user, &root, &data, &sav, 10, false).unwrap();
            assert_eq!(hash(&user.join("backups").join(done_.backup.unwrap().file)), base_hash, "{name}: the backup is the old save");
            let got: serde_json::Value = serde_json::from_slice(&std::fs::read(save_path(&user)).unwrap()).unwrap();
            let want: serde_json::Value = serde_json::from_slice(&std::fs::read(fx.join(format!("ref/{name}__real_copy.json"))).unwrap()).unwrap();
            assert!(got == want, "{name}: the imported save differs from the Python importer's output");
            // the same save inside a console package (built by the test) imports to the same result
            let raw = std::fs::read(&sav).unwrap();
            let pkg = d.path().join(format!("{name}.pkg"));
            std::fs::write(&pkg, mcla_savedecode::stfs::testing::build(&[mcla_savedecode::stfs::testing::File::new("mc4.sav", &raw, false)], &Default::default())).unwrap();
            std::fs::copy(fx.join("in/base_real_copy.json"), save_path(&user)).unwrap();
            import_in(&user, &root, &data, &pkg, 10, false).unwrap();
            let got: serde_json::Value = serde_json::from_slice(&std::fs::read(save_path(&user)).unwrap()).unwrap();
            assert!(got == want, "{name}: the save taken out of a package differs from the Python importer's output");
            done += 1;
        }
        // the user's real PS3 save folder (read only; copied into a scratch folder first) imports to the same save as its decrypted copy
        let ps3 = repo().join("save/PS3/SAVEDATA/BLUS30190MC4_SAV");
        if ps3.join("PARAM.PFD").is_file() {
            let (d, user, root) = setup(None);
            let data = data_dir(d.path());
            std::fs::create_dir_all(user.join("saves")).unwrap();
            std::fs::copy(fx.join("in/base_real_copy.json"), save_path(&user)).unwrap();
            let folder = d.path().join("BLUS30190MC4_SAV");
            std::fs::create_dir_all(&folder).unwrap();
            for f in std::fs::read_dir(&ps3).unwrap().flatten() {
                std::fs::copy(f.path(), folder.join(f.file_name())).unwrap();
            }
            let done_ = import_in(&user, &root, &data, &folder, 10, false).unwrap();
            assert!(done_.converted.unwrap().decrypted);
            let got: serde_json::Value = serde_json::from_slice(&std::fs::read(save_path(&user)).unwrap()).unwrap();
            let want: serde_json::Value = serde_json::from_slice(&std::fs::read(fx.join("ref/ps3_RAGE_dec__real_copy.json")).unwrap()).unwrap();
            assert!(got == want, "the real PS3 folder imports to something other than the decrypted save does");
            done += 1;
        }
        eprintln!("{done} real saves imported equal to the Python importer's output");
    }
}
