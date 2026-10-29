//! Tauri side of the Mods tab (docs/MODS.md stage S5, docs/LAUNCHER.md "Mods tab"). The rules are in the `mcla-mods` crate
//! (`mcla_mods::manage` for the list model, `install_package` for the update rule, `set_enabled` for the on / off flag in
//! settings.toml); this file finds the folders, keeps the files dropped on the window and turns results into what the page draws.
//! Every sentence the page shows about a mod's state comes from the locales (`dyn.mods.*`); only the checker's own reasons
//! (the crate's English diagnostics, one per problem) are shown as they are.

use mcla_mods::manage::{self, Listing};
use mcla_mods::InstallOutcome;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Files dropped on the window older than this are forgotten (a drop on another tab is not an install later).
const DROP_LIFE: Duration = Duration::from_secs(5);

/// What the window's drag-and-drop events left for the page to pick up.
#[derive(Default)]
pub struct Mods {
    dropped: Mutex<Vec<(Instant, PathBuf)>>,
    hover: AtomicBool,
}

/// Where the mods live for this run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub data_root: PathBuf,
    pub mods_dir: PathBuf,
    pub settings_file: PathBuf,
}

/// The data folder the game would use (`MCLA_DATA`, `paths.data_dir`, `<install root>/data/cache`), its `mods` folder (or the
/// game's test hook `MCLA_MODS_DIR`, so the launcher shows what the game would load) and the settings file. Pure: the
/// environment is read by [`paths`].
pub fn paths_from(root: &Path, env_data: Option<&str>, env_mods: Option<&Path>, settings_file: &Path) -> Paths {
    let settings = mcla_config::Settings::load(settings_file).settings;
    // (the game's exe sits in `<root>/game`; its data folder is found from there)
    let data_root = mcla_config::paths::resolve_data_dir(&mcla_config::paths::DataInputs { flag: None, env: env_data, settings: &settings.paths.data_dir, exe_dir: &root.join("game"), dev_fallback: None }).path;
    let mods_dir = env_mods.map(Path::to_path_buf).unwrap_or_else(|| mcla_mods::mods_dir(&data_root));
    Paths { data_root, mods_dir, settings_file: settings_file.to_path_buf() }
}

pub fn paths(root: &Path) -> Paths {
    let env_mods = std::env::var_os("MCLA_MODS_DIR").filter(|v| !v.is_empty()).map(PathBuf::from);
    paths_from(root, std::env::var("MCLA_DATA").ok().as_deref(), env_mods.as_deref(), &mcla_config::paths::settings_path(None))
}

pub fn view(p: &Paths) -> Listing {
    manage::scan(&p.data_root, &p.mods_dir, &p.settings_file)
}

/// The answer of the page's poll: files dropped on the window, whether a drag is over it, and the folder's change stamp.
#[derive(Serialize)]
pub struct Poll {
    pub dropped: Vec<String>,
    pub hover: bool,
    pub stamp: u64,
}

impl Mods {
    pub fn dropped(&self, paths: Vec<PathBuf>) {
        self.hover.store(false, Ordering::Relaxed);
        let mut g = self.dropped.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        g.extend(paths.into_iter().map(|p| (now, p)));
    }

    pub fn set_hover(&self, on: bool) {
        self.hover.store(on, Ordering::Relaxed);
    }

    pub fn poll(&self, p: &Paths) -> Poll {
        let mut g = self.dropped.lock().unwrap_or_else(|e| e.into_inner());
        let fresh: Vec<String> = g.drain(..).filter(|(t, _)| t.elapsed() < DROP_LIFE).map(|(_, f)| f.to_string_lossy().into_owned()).collect();
        Poll { dropped: fresh, hover: self.hover.load(Ordering::Relaxed), stamp: manage::stamp(&p.mods_dir, &p.settings_file) }
    }
}

#[derive(Serialize, Debug)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum InstallView {
    Installed {
        id: String,
        version: String,
        /// the installed package's name and what became of it in the list (`loaded`, `off`, `invalid`, `skipped`)
        name: String,
        status: &'static str,
        /// copies of the same mod id that were replaced
        replaced: usize,
        listing: Listing,
    },
    /// an older version over a newer one: nothing changed, ask the player and call again with `confirm_older`
    NeedsConfirmation { id: String, installed: String, incoming: String },
}

pub fn install(p: &Paths, zip: &Path, confirm_older: bool) -> Result<InstallView, String> {
    match mcla_mods::install_package(&p.mods_dir, zip, confirm_older)? {
        InstallOutcome::NeedsConfirmation { id, installed, incoming } => Ok(InstallView::NeedsConfirmation { id, installed, incoming }),
        InstallOutcome::Installed { id, version, path, replaced } => {
            let listing = view(p);
            let same = |r: &manage::Row| std::fs::canonicalize(&r.path).ok().zip(std::fs::canonicalize(&path).ok()).is_some_and(|(a, b)| a == b);
            let (name, status) = listing.rows.iter().find(|r| same(r)).map(|r| (r.name.clone(), r.status)).unwrap_or_else(|| (id.clone(), "invalid"));
            Ok(InstallView::Installed { id, version, name, status, replaced: replaced.len(), listing })
        }
    }
}

/// Switch a mod id on or off (settings.toml `[mods] disabled`), then read the list again.
pub fn set_on(p: &Paths, id: &str, on: bool) -> Result<Listing, String> {
    mcla_mods::set_enabled(&p.settings_file, id, on).map_err(|e| e.to_string())?;
    Ok(view(p))
}

/// Delete one package; when no other package has its id any more the id's "off" flag goes too, so a later install starts on.
pub fn remove(p: &Paths, package: &Path) -> Result<Listing, String> {
    let removed = manage::remove_package(&p.mods_dir, package)?;
    let l = view(p);
    if let Some(id) = removed.id {
        if !l.rows.iter().any(|r| r.id == id) && mcla_mods::disabled_ids(&p.settings_file).contains(&id) {
            // (a failure here only leaves a harmless id in the file)
            let _ = mcla_mods::set_enabled(&p.settings_file, &id, true);
        }
    }
    Ok(view(p))
}

/// One mod car a save names, as the import preview shows it.
#[derive(Serialize)]
pub struct CarState {
    pub car: String,
    pub mod_id: String,
    pub title: String,
    pub state: manage::Availability,
}

/// Where each mod car of a save stands against the mods installed now (`loaded`, `off` = installed but switched off, `missing`).
pub fn car_states(p: &Paths, refs: &[mcla_support::saves::ModCarRef]) -> Vec<CarState> {
    if refs.is_empty() {
        return Vec::new();
    }
    let l = view(p);
    refs.iter().map(|r| CarState { car: r.car.clone(), mod_id: r.mod_id.clone(), title: r.title.clone(), state: l.car_state(&r.car) }).collect()
}

pub fn pick_zips() -> Vec<String> {
    rfd::FileDialog::new().set_title("Choose a mod").add_filter("Mod package", &["zip"]).pick_files().unwrap_or_default().into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with_manifest(path: &Path, id: &str, version: &str) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        z.start_file("mod.toml", zip::write::SimpleFileOptions::default()).unwrap();
        // (a header and one car without its files: listed as invalid, which is enough to see the install rule at work)
        z.write_all(format!("schema = 1\n[mod]\nid = \"{id}\"\nversion = \"{version}\"\nname = \"Mod {id}\"\n[[car]]\nname = \"vp_x_{}\"\nlong_name = \"X\"\nshort_name = \"X\"\n[car.row]\nclass = \"tuner\"\nmanufacturer = \"nissan\"\ngroup = \"B0\"\nunlocking_group = \"A0\"\nprice = 1000\ndistricts = [\"Hollywood\"]\nfigures_from = \"vp_nsn_350z_06\"\n", id.replace('.', "_")).as_bytes()).unwrap();
        z.finish().unwrap();
    }

    fn setup() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = paths_from(d.path(), None, None, &d.path().join("settings.toml"));
        (d, p)
    }

    #[test]
    fn the_folders_follow_the_games_rules() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let settings = root.join("settings.toml");
        // nothing installed: the first place the game looks at
        let p = paths_from(root, None, None, &settings);
        assert_eq!(p.data_root, root.join("data").join("cache"));
        assert_eq!(p.mods_dir, root.join("data").join("cache").join("mods"));
        // MCLA_DATA wins, and the test hook MCLA_MODS_DIR moves only the mods
        let other = root.join("elsewhere");
        let p = paths_from(root, other.to_str(), None, &settings);
        assert_eq!(p.mods_dir, other.join("mods"));
        let p = paths_from(root, other.to_str(), Some(&root.join("m")), &settings);
        assert_eq!((p.data_root, p.mods_dir), (other, root.join("m")));
        // paths.data_dir of settings.toml (relative = from the install root)
        std::fs::write(&settings, "schema = 1\n[paths]\ndata_dir = \"mydata\"\n").unwrap();
        assert_eq!(paths_from(root, None, None, &settings).data_root, root.join("mydata"));
    }

    #[test]
    fn dropped_files_are_handed_over_once_and_forgotten_when_old() {
        let (_d, p) = setup();
        let m = Mods::default();
        assert!(m.poll(&p).dropped.is_empty() && !m.poll(&p).hover);
        m.set_hover(true);
        assert!(m.poll(&p).hover);
        m.dropped(vec![PathBuf::from("a.zip"), PathBuf::from("b.zip")]);
        let r = m.poll(&p);
        assert_eq!((r.dropped, r.hover), (vec!["a.zip".to_string(), "b.zip".to_string()], false), "a drop ends the hover");
        assert!(m.poll(&p).dropped.is_empty(), "handed over once");
        // a drop nobody asked for in time
        m.dropped.lock().unwrap().push((Instant::now() - DROP_LIFE - Duration::from_secs(1), PathBuf::from("old.zip")));
        assert!(m.poll(&p).dropped.is_empty());
    }

    #[test]
    fn install_switch_and_remove_through_the_launchers_calls() {
        let (d, p) = setup();
        let src = d.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        zip_with_manifest(&src.join("two.zip"), "t.mod", "2.0.0");
        zip_with_manifest(&src.join("one.zip"), "t.mod", "1.0.0");
        std::fs::write(src.join("notes.txt"), b"x").unwrap();

        assert!(install(&p, &src.join("notes.txt"), false).unwrap_err().contains(".zip"));
        let InstallView::Installed { id, status, replaced, listing, .. } = install(&p, &src.join("two.zip"), false).unwrap() else { panic!("installed") };
        assert_eq!((id.as_str(), status, replaced, listing.rows.len()), ("t.mod", "invalid", 0, 1), "the car has no files: listed, with its reasons");
        assert!(!listing.rows[0].contents[0].problems.is_empty());
        // an older version asks first and changes nothing; yes replaces in place
        let InstallView::NeedsConfirmation { installed, incoming, .. } = install(&p, &src.join("one.zip"), false).unwrap() else { panic!("asks") };
        assert_eq!((installed.as_str(), incoming.as_str()), ("2.0.0", "1.0.0"));
        assert_eq!(view(&p).rows[0].version, "2.0.0");
        let InstallView::Installed { listing, .. } = install(&p, &src.join("one.zip"), true).unwrap() else { panic!("installed") };
        assert_eq!((listing.rows.len(), listing.rows[0].version.as_str()), (1, "1.0.0"));

        // the switch writes the crate's key and nothing else
        std::fs::write(&p.settings_file, "schema = 1\n[display]\nvsync = false\n").unwrap();
        let l = set_on(&p, "t.mod", false).unwrap();
        assert!(!l.rows[0].on);
        let text = std::fs::read_to_string(&p.settings_file).unwrap();
        assert!(text.contains("disabled") && text.contains("t.mod") && text.contains("vsync = false"), "{text}");
        // a broken settings file is refused, not overwritten
        std::fs::write(&p.settings_file, "this is [not toml").unwrap();
        assert!(set_on(&p, "t.mod", true).is_err());
        assert_eq!(std::fs::read_to_string(&p.settings_file).unwrap(), "this is [not toml");
        std::fs::write(&p.settings_file, text).unwrap();

        // removing the last copy also forgets the "off" flag
        let l = remove(&p, Path::new(&l.rows[0].path)).unwrap();
        assert!(l.rows.is_empty());
        assert!(!mcla_mods::disabled_ids(&p.settings_file).contains("t.mod"));
        assert!(remove(&p, &src.join("notes.txt")).is_err());
    }

    #[test]
    fn a_saves_mod_cars_are_marked_installed_off_or_missing() {
        let (d, p) = setup();
        let mods = &p.mods_dir;
        // (which states a mod car can have is tested with real packages in `mcla_mods::manage`; here: the glue)
        std::fs::create_dir_all(mods).unwrap();
        let refs = vec![
            mcla_support::saves::ModCarRef { car: "vp_x_gone".into(), mod_id: "gone".into(), title: "GONE".into() },
            mcla_support::saves::ModCarRef { car: "vp_x_here".into(), mod_id: "here".into(), title: "HERE".into() },
        ];
        let s = car_states(&p, &refs);
        assert!(s.iter().all(|c| c.state == manage::Availability::Missing), "no mods installed");
        assert!(car_states(&p, &[]).is_empty());
        let _ = d;
    }
}
