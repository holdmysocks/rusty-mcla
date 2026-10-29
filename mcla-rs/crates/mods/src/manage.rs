//! What the launcher's Mods tab shows and does (docs/MODS.md stage S5; docs/LAUNCHER.md "Mods tab"): the scan of the mods folder
//! as a plain list model, a cheap change stamp for the folder, and removing a package. The loading rules themselves stay in
//! the crate root ([`ModSet::load_from`], [`install_package`], [`set_enabled`]); this module only reads their result.
//!
//! The list is generic: a mod is a package with contents ([`Content`]); the only kind so far is `"car"`, and the page never
//! assumes one car per mod (rim mods without a car and car mods with parts are coming).

use crate::disc::DiscInfo;
use crate::{disabled_ids, package_paths, LoadOptions, ModInfo, ModSet, ModState};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Cars the port adds itself and a mod may not define (`catalog::CUT_GTR` in the game crate; the game passes the same list
/// to [`LoadOptions::reserved`], so the launcher and the game agree on what loads).
pub const RESERVED_CARS: &[&str] = &["vp_nsn_gtr_08"];

/// One thing a package brings (a car so far).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Content {
    pub kind: &'static str,
    /// the car name the game and the save use
    pub name: String,
    /// what the player sees (the car's long name)
    pub title: String,
    /// why this content cannot be used (empty when it can)
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

/// A content another loaded mod already holds: the reason a mod was skipped.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Clash {
    pub kind: &'static str,
    pub name: String,
    pub title: String,
    pub held_by_id: String,
    pub held_by_name: String,
    pub held_by_version: String,
}

/// For a package skipped because another copy of the same mod id is used instead.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct UsedInstead {
    pub version: String,
    pub file: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Row {
    pub id: String,
    pub version: String,
    /// the mod's name; its id (or the file name of an unreadable package) when it has none
    pub name: String,
    pub author: String,
    pub description: String,
    /// the file or folder name in the mods folder
    pub file: String,
    pub path: String,
    pub is_zip: bool,
    /// `loaded`, `off` (valid, switched off), `invalid` or `skipped`
    pub status: &'static str,
    /// `clash` or `duplicate` when `status` is `skipped`
    pub skipped_why: Option<&'static str>,
    /// the on / off flag of the mod id (settings.toml); meaningful when `can_switch`
    pub on: bool,
    /// a switch makes sense: the package is valid and is the copy that counts for its id
    pub can_switch: bool,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
    pub contents: Vec<Content>,
    pub clashes: Vec<Clash>,
    pub used_instead: Option<UsedInstead>,
}

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub total: usize,
    pub loaded: usize,
    pub off: usize,
    pub invalid: usize,
    pub skipped: usize,
    /// contents of the loaded mods (cars so far)
    pub contents: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct Listing {
    pub mods_dir: String,
    /// the game's vehicle list was found in the data folder: without it the checks against the game are skipped
    pub data_found: bool,
    pub rows: Vec<Row>,
    pub summary: Summary,
    pub stamp: u64,
}

/// Whether a content (a car, by name) is usable by the game now.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// a loaded mod defines it
    Loaded,
    /// a valid mod defines it but is switched off
    Off,
    /// no installed, valid mod defines it
    Missing,
}

impl Listing {
    /// Where the car stands (case-insensitive name): the save import marks the mod cars that are not `Loaded`.
    pub fn car_state(&self, car: &str) -> Availability {
        let has = |status: &str| self.rows.iter().any(|r| r.status == status && r.contents.iter().any(|c| c.kind == "car" && c.name.eq_ignore_ascii_case(car)));
        if has("loaded") {
            Availability::Loaded
        } else if has("off") {
            Availability::Off
        } else {
            Availability::Missing
        }
    }
}

/// Read the mods folder the way the game does and describe every package found.
pub fn scan(data_root: &Path, mods_dir: &Path, settings_file: &Path) -> Listing {
    let disc = DiscInfo::load(data_root);
    let disabled = disabled_ids(settings_file);
    let opts = LoadOptions { disabled: disabled.clone(), reserved: RESERVED_CARS.iter().map(|s| s.to_string()).collect() };
    let set = ModSet::load_from(mods_dir, &disc, &opts);
    describe(&set, &disabled, disc.found, stamp(mods_dir, settings_file))
}

/// The list model of a loaded set (separate from [`scan`] so it can be tested with a set built by hand).
pub fn describe(set: &ModSet, disabled: &HashSet<String>, data_found: bool, stamp: u64) -> Listing {
    let infos = set.mods();
    // car (lower case) -> the loaded mod that holds it
    let mut holder: HashMap<String, &ModInfo> = HashMap::new();
    for m in infos.iter().filter(|m| m.state == ModState::Loaded) {
        for c in &m.cars {
            holder.entry(c.name.to_ascii_lowercase()).or_insert(m);
        }
    }
    let mut rows: Vec<Row> = Vec::new();
    for m in infos {
        let file = m.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (status, skipped_why) = match m.state {
            ModState::Loaded => ("loaded", None),
            ModState::Disabled => ("off", None),
            ModState::Invalid => ("invalid", None),
            ModState::Conflict => ("skipped", Some("clash")),
            ModState::Duplicate => ("skipped", Some("duplicate")),
        };
        let contents: Vec<Content> = m.cars.iter().map(|c| Content { kind: "car", name: c.name.clone(), title: if c.long_name.trim().is_empty() { c.name.clone() } else { c.long_name.clone() }, problems: c.reasons.clone(), notes: c.warnings.clone() }).collect();
        let clashes: Vec<Clash> = if m.state == ModState::Conflict {
            m.cars
                .iter()
                .filter_map(|c| {
                    let h = holder.get(&c.name.to_ascii_lowercase())?;
                    Some(Clash { kind: "car", name: c.name.clone(), title: if c.long_name.trim().is_empty() { c.name.clone() } else { c.long_name.clone() }, held_by_id: h.id.clone(), held_by_name: shown_name(h), held_by_version: h.version.clone() })
                })
                .collect()
        } else {
            Vec::new()
        };
        // the copy that is used instead of a duplicate: the other package with the same id that is not itself a duplicate
        let used_instead = (m.state == ModState::Duplicate)
            .then(|| infos.iter().find(|o| o.id == m.id && o.path != m.path && o.state != ModState::Duplicate))
            .flatten()
            .map(|o| UsedInstead { version: o.version.clone(), file: o.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() });
        // the crate's own sentence for a skipped mod is replaced by the structured fields above
        // (the crate's per-car reasons are also given per content, so they are not repeated here: what stays is the package's own)
        let own = |r: &String| !m.cars.iter().any(|c| !c.reasons.is_empty() && r.starts_with(&format!("car {}: ", c.name)));
        let reasons = if status == "skipped" { Vec::new() } else { m.reasons.iter().filter(|r| own(r)).cloned().collect() };
        rows.push(Row {
            id: m.id.clone(),
            version: m.version.clone(),
            name: shown_name(m),
            author: m.author.clone(),
            description: m.description.clone(),
            file,
            path: m.path.to_string_lossy().into_owned(),
            is_zip: m.is_zip,
            status,
            skipped_why,
            on: !disabled.contains(&m.id),
            can_switch: matches!(m.state, ModState::Loaded | ModState::Disabled | ModState::Conflict),
            reasons,
            warnings: m.warnings.clone(),
            contents,
            clashes,
            used_instead,
        });
    }
    let count = |s: &str| rows.iter().filter(|r| r.status == s).count();
    let summary = Summary { total: rows.len(), loaded: count("loaded"), off: count("off"), invalid: count("invalid"), skipped: count("skipped"), contents: rows.iter().filter(|r| r.status == "loaded").map(|r| r.contents.len()).sum() };
    Listing { mods_dir: set.mods_dir.to_string_lossy().into_owned(), data_found, rows, summary, stamp }
}

fn shown_name(m: &ModInfo) -> String {
    if m.name.trim().is_empty() {
        m.id.clone()
    } else {
        m.name.clone()
    }
}

/// A number that changes when something in the mods folder (a package added, replaced, removed or edited) or the settings file
/// changes: names, sizes and modification times of the top-level entries and of the files inside folder mods (at most
/// [`STAMP_LIMIT`] entries), no file is read. The launcher polls it to pick up mods copied in by hand.
pub fn stamp(mods_dir: &Path, settings_file: &Path) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut budget = STAMP_LIMIT;
    walk_stamp(mods_dir, 0, &mut budget, &mut h);
    if let Ok(md) = std::fs::metadata(settings_file) {
        (md.len(), md.modified().ok()).hash(&mut h);
    }
    h.finish()
}

/// Entries read by one [`stamp`] (a mod folder has some hundred files; this bounds a folder of the wrong kind).
pub const STAMP_LIMIT: usize = 5000;

fn walk_stamp(dir: &Path, depth: usize, budget: &mut usize, h: &mut impl Hasher) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        // (the unpack cache and the installer's temporary files change by themselves)
        let name = e.file_name().to_string_lossy().into_owned();
        if depth == 0 && name.starts_with('.') {
            continue;
        }
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Ok(md) = e.metadata() else { continue };
        (&name, md.is_dir(), if md.is_dir() { 0 } else { md.len() }, md.modified().ok()).hash(h);
        if md.is_dir() {
            walk_stamp(&e.path(), depth + 1, budget, h);
        }
    }
}

/// What [`remove_package`] removed.
#[derive(Debug, PartialEq, Eq)]
pub struct Removed {
    pub path: PathBuf,
    /// the mod id read from the package's manifest (None when it had none)
    pub id: Option<String>,
}

/// Delete one package (a folder or a `.zip` directly inside the mods folder, as listed by [`ModInfo::path`]) and its unpacked copy in
/// the cache. Refuses anything that is not a package of that folder. Cars of the mod that a save holds stay in the save (the
/// game shows them as greyed placeholders, DECISIONS.md D27 / D28).
pub fn remove_package(mods_dir: &Path, path: &Path) -> Result<Removed, String> {
    let dir = std::fs::canonicalize(mods_dir).map_err(|e| format!("the mods folder {} cannot be read: {e}", mods_dir.display()))?;
    let target = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if target.parent() != Some(dir.as_path()) {
        return Err(format!("{} is not inside the mods folder", path.display()));
    }
    // (the same test as the scan: a package is a sub-folder or a .zip whose name does not start with a dot)
    if !package_paths(&dir).into_iter().any(|p| std::fs::canonicalize(&p).ok().as_deref() == Some(target.as_path())) {
        return Err(format!("{} is not a mod package", path.display()));
    }
    let id = crate::package::Package::open(&target).ok().and_then(|p| crate::manifest::parse(&p.manifest_text).ok()).map(|m| m.info.id);
    if target.is_dir() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("cannot remove {}: {e}", target.display()))?;
    } else {
        std::fs::remove_file(&target).map_err(|e| format!("cannot remove {}: {e}", target.display()))?;
    }
    if let Some(id) = &id {
        // (best effort: the game may still be using the unpacked files)
        let cache = crate::cache_dir(&dir);
        let _ = std::fs::remove_dir_all(cache.join(id));
        let _ = std::fs::remove_file(cache.join(format!("{id}.stamp")));
    }
    Ok(Removed { path: target, id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcla_formats::hash::joaat;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tmp(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!("mcla-mods-manage-{}-{}-{name}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const DISC_LIST: &str = "Manufacturer\r\n{\r\n\tName\t\tAudi\r\n}\r\nVehicle\r\n{\r\n\tName\t\tvp_aud_r8_08\r\n\tClass\t\tEXOTIC\r\n\tManufacturer\tAudi\r\n\tPrice\t\t118000\r\n\tBaseAccel 0.26\r\n\tModsAccel 0.42\r\n\tBaseSpeed 210.0\r\n\tModsSpeed 255.0\r\n\tBaseHndlg 0.62\r\n\tModsHndlg 0.875\r\n}\r\n";

    fn data_root() -> PathBuf {
        let d = tmp("data");
        let v = d.join("tune/vehicle");
        std::fs::create_dir_all(&v).unwrap();
        std::fs::write(v.join(format!("0x{:08x}", joaat("vehicle.lst"))), DISC_LIST).unwrap();
        d
    }

    fn manifest(id: &str, version: &str, cars: &[&str]) -> String {
        let mut s = format!("schema = 1\n[mod]\nid = \"{id}\"\nversion = \"{version}\"\nname = \"Mod {id}\"\nauthor = \"Tester\"\n");
        for car in cars {
            s.push_str(&format!(
                "[[car]]\nname = \"{car}\"\nlong_name = \"LONG {car}\"\nshort_name = \"SHORT\"\n[car.row]\nclass = \"exotic\"\nmanufacturer = \"audi\"\ngroup = \"B0\"\nunlocking_group = \"a0\"\nprice = 120000\ndistricts = [\"Hollywood\"]\nfigures_from = \"vp_aud_r8_08\"\n[car.donors]\nsound = \"vp_aud_r8_08\"\nprices = \"vp_aud_r8_08\"\ncameras = \"vp_aud_r8_08\"\n"
            ));
        }
        s
    }

    fn xcc(name: &str) -> Vec<u8> {
        let mut b = vec![0u8; crate::validate::XCC_SIZE];
        b[crate::validate::XCC_TYPENAME..crate::validate::XCC_TYPENAME + name.len()].copy_from_slice(name.as_bytes());
        b
    }

    fn car_files(car: &str) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<(String, Vec<u8>)> = ["0x00000001.r63", "0x00000002.r63", "0x00000003.r83", "0x00000004.r83", "0x00000005.r19", "0x00000006.r50"].iter().map(|f| (format!("resources/vehicle/{car}/{f}"), f.as_bytes().to_vec())).collect();
        v.push((format!("resources/vehicle/configs/{car}.xcc"), xcc(car)));
        v.push((format!("resources/rims/whl_stk_{car}/0x00000007.r63"), b"rim".to_vec()));
        v.push((format!("tune/garage/{car}.garage"), b"Height 1.0\r\n".to_vec()));
        for p in ["managerides/mr_", "showroom/sh_", "mini_showroom/msh_"] {
            v.push((format!("resources/ui/textures/garage/dyntextures/{p}{car}.r9"), p.as_bytes().to_vec()));
        }
        v
    }

    fn folder_mod(mods: &Path, folder: &str, id: &str, version: &str, car: &str) {
        let d = mods.join(folder);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("mod.toml"), manifest(id, version, &[car])).unwrap();
        for (rel, b) in car_files(car) {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b).unwrap();
        }
    }

    fn zip_mod(path: &Path, id: &str, version: &str, car: &str) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let opt = zip::write::SimpleFileOptions::default();
        z.start_file("mod.toml", opt).unwrap();
        z.write_all(manifest(id, version, &[car]).as_bytes()).unwrap();
        for (rel, b) in car_files(car) {
            z.start_file(rel, opt).unwrap();
            z.write_all(&b).unwrap();
        }
        z.finish().unwrap();
    }

    fn row<'a>(l: &'a Listing, file: &str) -> &'a Row {
        l.rows.iter().find(|r| r.file == file).unwrap_or_else(|| panic!("no row {file}: {:?}", l.rows.iter().map(|r| &r.file).collect::<Vec<_>>()))
    }

    #[test]
    fn the_list_tells_every_state_and_why() {
        let data = data_root();
        let mods = data.join("mods");
        let settings = data.join("settings.toml");
        folder_mod(&mods, "alpha", "a.first", "1.0.0", "vp_test_a");
        // a second mod with the same car: skipped, and the list says which car and who holds it
        folder_mod(&mods, "bravo", "b.second", "1.0.0", "vp_test_a");
        folder_mod(&mods, "charlie", "c.third", "1.0.0", "vp_test_c");
        // an invalid one: a manifest that names a car without any of its files
        std::fs::create_dir_all(mods.join("broken")).unwrap();
        std::fs::write(mods.join("broken").join("mod.toml"), manifest("d.broken", "1.0.0", &["vp_test_d"])).unwrap();
        // a folder without a manifest, and a zip that is not a zip
        std::fs::create_dir_all(mods.join("empty")).unwrap();
        std::fs::write(mods.join("junk.zip"), b"not a zip").unwrap();
        // the same id twice: the newer one is used
        zip_mod(&mods.join("e_old.zip"), "e.dup", "1.0.0", "vp_test_e");
        zip_mod(&mods.join("e_new.zip"), "e.dup", "1.2.0", "vp_test_e");
        // a loose file and a dot folder are not packages
        std::fs::write(mods.join("readme.txt"), b"x").unwrap();
        std::fs::create_dir_all(mods.join(".cache")).unwrap();
        crate::set_enabled(&settings, "c.third", false).unwrap();

        let l = scan(&data, &mods, &settings);
        assert!(l.data_found);
        assert_eq!(l.rows.len(), 8, "{:?}", l.rows.iter().map(|r| (&r.file, r.status)).collect::<Vec<_>>());
        let a = row(&l, "alpha");
        assert_eq!((a.status, a.on, a.can_switch, a.id.as_str(), a.version.as_str(), a.author.as_str()), ("loaded", true, true, "a.first", "1.0.0", "Tester"));
        assert_eq!(a.contents, vec![Content { kind: "car", name: "vp_test_a".into(), title: "LONG vp_test_a".into(), problems: vec![], notes: vec![] }]);
        let b = row(&l, "bravo");
        assert_eq!((b.status, b.skipped_why), ("skipped", Some("clash")));
        assert!(b.reasons.is_empty() && b.can_switch);
        assert_eq!(b.clashes, vec![Clash { kind: "car", name: "vp_test_a".into(), title: "LONG vp_test_a".into(), held_by_id: "a.first".into(), held_by_name: "Mod a.first".into(), held_by_version: "1.0.0".into() }]);
        let c = row(&l, "charlie");
        assert_eq!((c.status, c.on), ("off", false));
        let d = row(&l, "broken");
        assert_eq!(d.status, "invalid");
        assert!(!d.can_switch);
        assert!(d.reasons.is_empty() && !d.contents[0].problems.is_empty(), "the car lists its own problems, once: {:?}", d.reasons);
        let e = row(&l, "empty");
        assert_eq!((e.status, e.name.as_str(), e.contents.len()), ("invalid", "empty", 0));
        assert!(e.reasons[0].contains("mod.toml"), "{:?}", e.reasons);
        assert_eq!(row(&l, "junk.zip").status, "invalid");
        let (old, new) = (row(&l, "e_old.zip"), row(&l, "e_new.zip"));
        assert_eq!((old.status, old.skipped_why, new.status), ("skipped", Some("duplicate"), "loaded"));
        assert_eq!(old.used_instead, Some(UsedInstead { version: "1.2.0".into(), file: "e_new.zip".into() }));
        assert!(!old.can_switch);
        assert_eq!(l.summary, Summary { total: 8, loaded: 2, off: 1, invalid: 3, skipped: 2, contents: 2 });
        // the same data, a different way: where each car stands
        assert_eq!(l.car_state("VP_TEST_A"), Availability::Loaded);
        assert_eq!(l.car_state("vp_test_c"), Availability::Off);
        assert_eq!(l.car_state("vp_test_d"), Availability::Missing);
        assert_eq!(l.car_state("vp_gone"), Availability::Missing);
    }

    #[test]
    fn no_mods_folder_and_no_game_data_are_not_errors() {
        let d = tmp("nothing");
        let l = scan(&d, &d.join("mods"), &d.join("settings.toml"));
        assert!(l.rows.is_empty() && !l.data_found);
        assert_eq!(l.summary, Summary::default());
        assert_eq!(l.car_state("anything"), Availability::Missing);
    }

    #[test]
    fn the_stamp_follows_the_folder() {
        let data = data_root();
        let mods = data.join("mods");
        let settings = data.join("settings.toml");
        let s0 = stamp(&mods, &settings);
        assert_eq!(s0, stamp(&mods, &settings), "same folder, same stamp");
        folder_mod(&mods, "alpha", "a.first", "1.0.0", "vp_test_a");
        let s1 = stamp(&mods, &settings);
        assert_ne!(s0, s1, "a folder mod appeared");
        std::fs::write(mods.join("alpha").join("tune/garage/vp_test_a.garage"), b"Height 2.0000\r\n").unwrap();
        let s2 = stamp(&mods, &settings);
        assert_ne!(s1, s2, "a file inside a folder mod changed");
        zip_mod(&mods.join("z.zip"), "z.zip.mod", "1.0.0", "vp_test_z");
        let s3 = stamp(&mods, &settings);
        assert_ne!(s2, s3, "a zip appeared");
        // the cache and temporary files (dot names) are ignored
        std::fs::create_dir_all(mods.join(".cache/x")).unwrap();
        std::fs::write(mods.join(".tmp.zip"), b"half").unwrap();
        assert_eq!(s3, stamp(&mods, &settings));
        crate::set_enabled(&settings, "a.first", false).unwrap();
        assert_ne!(s3, stamp(&mods, &settings), "the on / off file changed");
    }

    #[test]
    fn remove_deletes_a_package_and_its_cache_and_nothing_else() {
        let data = data_root();
        let mods = data.join("mods");
        folder_mod(&mods, "alpha", "a.first", "1.0.0", "vp_test_a");
        zip_mod(&mods.join("zed.zip"), "z.mod", "1.0.0", "vp_test_z");
        std::fs::create_dir_all(mods.join(".cache/z.mod")).unwrap();
        std::fs::write(mods.join(".cache/z.mod.stamp"), b"x").unwrap();
        std::fs::write(mods.join("readme.txt"), b"x").unwrap();
        let outside = tmp("outside");
        std::fs::create_dir_all(outside.join("pkg")).unwrap();

        // refused: a loose file, the cache, a folder elsewhere, the mods folder itself
        assert!(remove_package(&mods, &mods.join("readme.txt")).is_err());
        assert!(remove_package(&mods, &mods.join(".cache")).is_err());
        assert!(remove_package(&mods, &outside.join("pkg")).is_err());
        assert!(remove_package(&mods, &mods).is_err());
        assert!(remove_package(&mods, &mods.join("..").join("mods").join("..").join("settings.toml")).is_err());
        assert!(outside.join("pkg").is_dir() && mods.join("readme.txt").is_file());

        let r = remove_package(&mods, &mods.join("zed.zip")).unwrap();
        assert_eq!(r.id.as_deref(), Some("z.mod"));
        assert!(!mods.join("zed.zip").exists() && !mods.join(".cache/z.mod").exists() && !mods.join(".cache/z.mod.stamp").exists());
        let r = remove_package(&mods, &mods.join("alpha")).unwrap();
        assert_eq!(r.id.as_deref(), Some("a.first"));
        assert!(!mods.join("alpha").exists());
        assert!(mods.join("readme.txt").is_file());
        assert!(scan(&data, &mods, &data.join("settings.toml")).rows.is_empty());
        // a folder with no readable manifest can still be removed
        std::fs::create_dir_all(mods.join("junk")).unwrap();
        assert_eq!(remove_package(&mods, &mods.join("junk")).unwrap().id, None);
    }

    #[test]
    fn install_remove_and_scan_agree() {
        // the launcher's path: install a zip, see it, switch it off, see that, replace with an older one (asks), remove
        let data = data_root();
        let mods = data.join("mods");
        let settings = data.join("settings.toml");
        let src = tmp("zips");
        zip_mod(&src.join("new.zip"), "x.mod", "2.0.0", "vp_test_x");
        zip_mod(&src.join("old.zip"), "x.mod", "1.0.0", "vp_test_x");
        assert!(matches!(crate::install_package(&mods, &src.join("new.zip"), false).unwrap(), crate::InstallOutcome::Installed { .. }));
        let l = scan(&data, &mods, &settings);
        assert_eq!((l.rows.len(), l.rows[0].status, l.rows[0].file.as_str()), (1, "loaded", "x.mod.zip"));
        crate::set_enabled(&settings, "x.mod", false).unwrap();
        assert_eq!(scan(&data, &mods, &settings).rows[0].status, "off");
        assert!(matches!(crate::install_package(&mods, &src.join("old.zip"), false).unwrap(), crate::InstallOutcome::NeedsConfirmation { .. }));
        assert_eq!(scan(&data, &mods, &settings).rows[0].version, "2.0.0", "nothing changed while the question is open");
        assert!(matches!(crate::install_package(&mods, &src.join("old.zip"), true).unwrap(), crate::InstallOutcome::Installed { .. }));
        let l = scan(&data, &mods, &settings);
        assert_eq!((l.rows.len(), l.rows[0].version.as_str(), l.rows[0].status), (1, "1.0.0", "off"), "replaced in place, still switched off");
        remove_package(&mods, Path::new(&l.rows[0].path)).unwrap();
        assert!(scan(&data, &mods, &settings).rows.is_empty());
    }
}
