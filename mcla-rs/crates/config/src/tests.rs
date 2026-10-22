use super::*;
use crate::paths::*;
use std::sync::atomic::{AtomicU32, Ordering};

fn tmp(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("mcla-config-test-{}-{}-{name}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn schema_matches_defaults() {
    let d = Settings::default().to_table();
    for e in schema() {
        let (tab, key) = e.key.split_once('.').unwrap();
        let got = d.get(tab).and_then(|t| t.get(key)).unwrap_or_else(|| panic!("{} missing from the default table", e.key));
        let want: Table = format!("v = {}", e.default).parse().unwrap();
        assert_eq!(got, &want["v"], "default of {}", e.key);
    }
    // every key the model writes is in the schema
    let mut n = 0;
    for (tab, v) in &d {
        if let Some(t) = v.as_table() {
            n += t.len();
            for key in t.keys() {
                assert!(schema().iter().any(|e| e.key == format!("{tab}.{key}")), "{tab}.{key} not in the schema");
            }
        }
    }
    assert_eq!(n, schema().len());
    assert!(schema_toml().contains("name = \"display.vsync\""));
}

#[test]
fn defaults_survive_a_round_trip() {
    let s = Settings::default();
    let text = s.to_toml();
    let (back, warn) = Settings::from_table(&text.parse().unwrap());
    assert!(warn.is_empty(), "{warn:?}");
    assert_eq!(back.display, s.display);
    assert_eq!(back.game, s.game);
    assert_eq!(back.paths, s.paths);
    assert_eq!(back.launcher, s.launcher);
}

#[test]
fn values_are_read() {
    let t: Table = r#"
schema = 1
[display]
window_mode = "borderless"
resolution = "2560x1440"
vsync = false
fps_cap = 144
render_scale = 0.75
[game]
intro = "skip"
[paths]
data_dir = "data/cache"
screenshots = "D:/shots"
"#
    .parse()
    .unwrap();
    let (s, warn) = Settings::from_table(&t);
    assert!(warn.is_empty(), "{warn:?}");
    assert_eq!(s.display.window_mode, WindowMode::Borderless);
    assert_eq!(s.display.resolution, Resolution { w: 2560, h: 1440 });
    assert!(!s.display.vsync);
    assert_eq!(s.display.fps_cap, 144);
    assert_eq!(s.display.render_scale, 0.75);
    assert_eq!(s.game.intro, Intro::Skip);
    assert_eq!(s.paths.data_dir, "data/cache");
    // what was not in the file keeps its default
    assert_eq!(s.display.aa, Aa::Off);
}

#[test]
fn a_bad_key_costs_only_itself() {
    let t: Table = r#"
[display]
window_mode = "sideways"
resolution = "banana"
vsync = "yes"
fps_cap = 5
render_scale = 9.0
monitor = -1
[game]
intro = "fast"
"#
    .parse()
    .unwrap();
    let (s, warn) = Settings::from_table(&t);
    let d = Settings::default();
    assert_eq!(s.display, d.display);
    assert_eq!(s.game.intro, Intro::Fast, "a good key in the same file still counts");
    assert_eq!(warn.len(), 6, "{warn:?}");
    assert!(warn[0].starts_with("display.window_mode = \"sideways\": expected one of windowed, borderless, fullscreen"));
}

#[test]
fn resolution_limits() {
    assert_eq!(Resolution::parse("1920x1080"), Some(Resolution { w: 1920, h: 1080 }));
    assert_eq!(Resolution::parse(" 3840 X 2160 "), Some(Resolution { w: 3840, h: 2160 }));
    assert_eq!(Resolution::parse("639x360"), None);
    assert_eq!(Resolution::parse("640x359"), None);
    assert_eq!(Resolution::parse("16385x1000"), None);
    assert_eq!(Resolution::parse("1920"), None);
}

#[test]
fn unknown_keys_are_kept() {
    let dir = tmp("unknown");
    let path = dir.join("settings.toml");
    std::fs::write(&path, "schema = 1\nfuture = \"x\"\n[display]\nvsync = false\nhdr = true\n[newtable]\na = [1, 2]\n").unwrap();
    let mut l = Settings::load(&path);
    assert_eq!(l.status, Status::Ok);
    assert!(l.warnings.is_empty());
    l.settings.display.fps_cap = 60;
    l.settings.save(&path).unwrap();
    let back: Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert_eq!(back["future"].as_str(), Some("x"));
    assert_eq!(back["display"]["hdr"].as_bool(), Some(true));
    assert_eq!(back["display"]["vsync"].as_bool(), Some(false));
    assert_eq!(back["display"]["fps_cap"].as_integer(), Some(60));
    assert_eq!(back["newtable"]["a"].as_array().map(|a| a.len()), Some(2));
    assert!(!dir.join("settings.toml.tmp").exists(), "the temp file is gone after the rename");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_newer_schema_is_read_with_a_note() {
    let t: Table = "schema = 7\n[display]\nvsync = false\n".parse().unwrap();
    let (s, warn) = Settings::from_table(&t);
    assert!(!s.display.vsync);
    assert_eq!(warn.len(), 1);
    assert!(warn[0].contains("newer"));
    assert_eq!(s.to_table()["schema"].as_integer(), Some(7), "the number is not lowered");
}

#[test]
fn missing_and_broken_files_give_defaults() {
    let dir = tmp("broken");
    let l = Settings::load(&dir.join("nope.toml"));
    assert_eq!(l.status, Status::Missing);
    assert_eq!(l.settings.display, Settings::default().display);
    assert!(l.warnings.is_empty());
    let bad = dir.join("bad.toml");
    std::fs::write(&bad, "[display\nvsync = = 3").unwrap();
    let l = Settings::load(&bad);
    assert_eq!(l.status, Status::Broken);
    assert_eq!(l.settings.display, Settings::default().display);
    assert_eq!(l.warnings.len(), 1);
    assert!(l.warnings[0].contains("not valid TOML") && l.warnings[0].ends_with("using the defaults"), "{:?}", l.warnings);
    // a file with a byte order mark still reads
    let bom = dir.join("bom.toml");
    std::fs::write(&bom, "\u{feff}[display]\nvsync = false\n").unwrap();
    assert!(!Settings::load(&bom).settings.display.vsync);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn render_scale_is_written_cleanly() {
    let mut s = Settings::default();
    s.display.render_scale = 0.8;
    assert!(s.to_toml().contains("render_scale = 0.8\n"), "{}", s.to_toml());
}

fn inputs<'a>(flag: Option<&'a str>, env: Option<&'a str>, settings: &'a str, exe: &'a Path, dev: Option<&'a Path>) -> DataInputs<'a> {
    DataInputs { flag, env, settings, exe_dir: exe, dev_fallback: dev }
}

#[test]
fn data_dir_resolution_order() {
    let root = tmp("data");
    let exe = root.join("game");
    let up = root.join("data").join("cache");
    let beside = exe.join("data").join("cache");
    let dev = root.join("dev");
    std::fs::create_dir_all(&exe).unwrap();
    std::fs::create_dir_all(&dev).unwrap();

    // nothing but the dev fallback
    let r = resolve_data_dir(&inputs(None, None, "", &exe, Some(&dev)));
    assert_eq!((r.source, r.path.as_path()), (DataSource::DevFallback, dev.as_path()));
    // a dev fallback that does not exist is not used; the first place looked at is reported
    let r = resolve_data_dir(&inputs(None, None, "", &exe, Some(&root.join("missing"))));
    assert_eq!(r.source, DataSource::NotFound);
    assert_eq!(r.path, up, "the first place looked at is reported");
    let r = resolve_data_dir(&inputs(None, None, "", &exe, None));
    assert_eq!(r.source, DataSource::NotFound);

    // <exe dir>/data/cache beats the dev fallback
    std::fs::create_dir_all(&beside).unwrap();
    let r = resolve_data_dir(&inputs(None, None, "", &exe, Some(&dev)));
    assert_eq!((r.source, r.path.clone()), (DataSource::BesideExe, beside.clone()));
    // <exe dir>/../data/cache beats that
    std::fs::create_dir_all(&up).unwrap();
    let r = resolve_data_dir(&inputs(None, None, "", &exe, Some(&dev)));
    assert_eq!((r.source, r.path), (DataSource::InstallRoot, up.clone()));
    // settings beat the exe-relative folders; a relative value is taken from the install root
    let r = resolve_data_dir(&inputs(None, None, "data/cache", &exe, Some(&dev)));
    assert_eq!((r.source, r.path), (DataSource::Settings, up.clone()));
    let abs = root.join("elsewhere");
    let r = resolve_data_dir(&inputs(None, None, abs.to_str().unwrap(), &exe, Some(&dev)));
    assert_eq!((r.source, r.path.clone()), (DataSource::Settings, abs.clone()), "taken as given even if it does not exist");
    // the environment beats settings, the flag beats the environment
    let r = resolve_data_dir(&inputs(None, Some("E:/env"), abs.to_str().unwrap(), &exe, Some(&dev)));
    assert_eq!((r.source, r.path), (DataSource::Env, PathBuf::from("E:/env")));
    let r = resolve_data_dir(&inputs(Some("F:/flag"), Some("E:/env"), abs.to_str().unwrap(), &exe, Some(&dev)));
    assert_eq!((r.source, r.path), (DataSource::Flag, PathBuf::from("F:/flag")));
    // empty values count as not set
    let r = resolve_data_dir(&inputs(Some(""), Some("  "), "", &exe, None));
    assert_eq!(r.source, DataSource::InstallRoot);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn screenshot_folder_choice() {
    let exe = Path::new("C:/MCLA/game");
    assert_eq!(screenshots_dir("", exe), Path::new("C:/MCLA/screenshots"));
    assert_eq!(screenshots_dir("pics", exe), Path::new("C:/MCLA/pics"));
    let abs = std::env::temp_dir().join("shots");
    assert_eq!(screenshots_dir(abs.to_str().unwrap(), exe), abs);
}

#[test]
fn settings_file_choice() {
    assert_eq!(settings_path(Some("x/y.toml")), PathBuf::from("x/y.toml"));
    // (the environment is process-wide: this test is the only one that touches these variables)
    std::env::set_var("RUSTY_MCLA_USER_DIR", "U:/user");
    std::env::remove_var("MCLA_SETTINGS");
    assert_eq!(settings_path(None), PathBuf::from("U:/user/settings.toml"));
    assert_eq!(logs_dir(), PathBuf::from("U:/user/logs"));
    std::env::set_var("MCLA_SETTINGS", "E:/s.toml");
    assert_eq!(settings_path(None), PathBuf::from("E:/s.toml"));
    assert_eq!(settings_path(Some("F:/f.toml")), PathBuf::from("F:/f.toml"));
    std::env::remove_var("MCLA_SETTINGS");
    std::env::remove_var("RUSTY_MCLA_USER_DIR");
}

#[test]
fn set_key_changes_one_key_only() {
    let d = tmp("setkey");
    let p = d.join("settings.toml");
    // (missing: created with the schema and the key)
    set_key(&p, "display", "window_mode", Value::String("borderless".into())).unwrap();
    let l = Settings::load(&p);
    assert_eq!((l.status, l.settings.display.window_mode, l.warnings.len()), (Status::Ok, WindowMode::Borderless, 0));
    // (an existing file: every other key, unknown keys and a value this build refuses stay as they were)
    let text = "schema = 1\nfuture = { a = 1 }\n[display]\nvsync = false\nresolution = \"1280x720\"\nfps_cap = 7\nwindow_mode = \"windowed\"\nnew_key = \"kept\"\n[launcher]\nlanguage = \"de\"\n";
    std::fs::write(&p, text).unwrap();
    set_key(&p, "display", "window_mode", Value::String(WindowMode::Windowed.next().next().word().into())).unwrap();
    let before: Table = text.parse().unwrap();
    let mut want = before.clone();
    want["display"].as_table_mut().unwrap().insert("window_mode".into(), Value::String("fullscreen".into()));
    let after: Table = std::fs::read_to_string(&p).unwrap().parse().unwrap();
    assert_eq!(after, want);
    set_key(&p, "display", "monitor", Value::Integer(2)).unwrap();
    assert_eq!(Settings::load(&p).settings.display.monitor, 2);
    // (not TOML: refused, the file is left alone, no temp file)
    let bad = "[display\nvsync = = oops";
    std::fs::write(&p, bad).unwrap();
    assert_eq!(set_key(&p, "display", "window_mode", Value::String("windowed".into())).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), bad);
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    // (the cycle)
    assert_eq!((WindowMode::Windowed.next(), WindowMode::Borderless.next(), WindowMode::Fullscreen.next()), (WindowMode::Borderless, WindowMode::Fullscreen, WindowMode::Windowed));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn fps_monitor_is_read_and_written() {
    let t: Table = "[display]\nfps_monitor = \"frame_time\"\n".parse().unwrap();
    let (s, warn) = Settings::from_table(&t);
    assert!(warn.is_empty(), "{warn:?}");
    assert_eq!(s.display.fps_monitor, FpsMonitor::FrameTime);
    assert_eq!(s.to_table()["display"]["fps_monitor"].as_str(), Some("frame_time"));
    let (s, warn) = Settings::from_table(&"[display]\nfps_monitor = \"graph\"\n".parse().unwrap());
    assert_eq!((s.display.fps_monitor, warn.len()), (FpsMonitor::Off, 1));
}
