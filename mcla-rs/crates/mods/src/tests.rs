use super::*;
use mcla_formats::hash::joaat;
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering as AO};

fn tmp(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("mcla-mods-test-{}-{}-{name}", std::process::id(), N.fetch_add(1, AO::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const DISC_LIST: &str = "Manufacturer\r\n{\r\n\tName\t\tAudi\r\n\tCanBeOnFire\t1\r\n}\r\nManufacturer\r\n{\r\n\tName\t\tNissan\r\n\tCanBeOnFire\t1\r\n}\r\n\
Vehicle\r\n{\r\n\tName\t\tvp_aud_r8_08\r\n\tClass\t\tEXOTIC\r\n\tManufacturer\tAudi\r\n\tPrice\t\t118000\r\n\tBaseAccel       0.26\r\n\tModsAccel       0.42\r\n\tBaseSpeed       210.00\r\n\tModsSpeed       255.00\r\n\tBaseHndlg       0.620\r\n\tModsHndlg       0.875\r\n}\r\n\
Vehicle\r\n{\r\n\tName\t\tvp_dev_only\r\n\tDEVONLY\r\n}\r\n\
Vehicle\r\n{\r\n\tName\t\tvp_nsn_350z_06\r\n\tClass\t\tTUNER\r\n\tManufacturer\tNissan\r\n}\r\n";

const SAMPLES: &str = "sample\n{\nname vp_aud_r8_08\nmod 0\nperformance 0.580801\nspeed 0.487276\n}\nsample\n{\nname vp_aud_r8_08\nmod 50\nperformance 0.707273\nspeed 0.619890\n}\nsample\n{\nname vp_aud_r8_08\nmod 100\nperformance 0.822529\nspeed 0.750580\n}\n";

/// A data root with a two-car vehicle list (+ one DEVONLY block), two manufacturers and the R8's samples; the
/// vehicle list under its hashed name, the samples under the plain one (both forms are read).
fn data_root() -> PathBuf {
    let d = tmp("data");
    let v = d.join("tune/vehicle");
    std::fs::create_dir_all(&v).unwrap();
    std::fs::write(v.join(format!("0x{:08x}", joaat("vehicle.lst"))), DISC_LIST).unwrap();
    std::fs::write(v.join("car_performance_sample_0.lst"), SAMPLES).unwrap();
    d
}

fn manifest(id: &str, version: &str, car: &str, extra: &str) -> String {
    format!(
        r#"schema = 1
[mod]
id = "{id}"
version = "{version}"
name = "Test {id}"

[[car]]
name = "{car}"
long_name = "2008 TEST"
short_name = "08 TEST"
[car.row]
class = "exotic"
manufacturer = "audi"
group = "B0"
unlocking_group = "a0"
price = 120000
districts = ["Hollywood", "beaches"]
figures_from = "vp_aud_r8_08"
[car.donors]
sound = "vp_aud_r8_08"
prices = "vp_aud_r8_08"
cameras = "vp_aud_r8_08"
{extra}"#
    )
}

fn xcc(name: &str) -> Vec<u8> {
    let mut b = vec![0u8; validate::XCC_SIZE];
    b[validate::XCC_TYPENAME..validate::XCC_TYPENAME + name.len()].copy_from_slice(name.as_bytes());
    b
}

/// The files of a complete car (folder, set-up, rims, garage camera, pictures).
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

fn write_folder(dir: &Path, manifest: &str, files: &[(String, Vec<u8>)]) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("mod.toml"), manifest).unwrap();
    for (rel, b) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b).unwrap();
    }
}

/// A zip built here, with everything inside one top folder `top/` when given (a zipped folder).
fn write_zip(path: &Path, top: &str, manifest: &str, files: &[(String, Vec<u8>)]) {
    let f = std::fs::File::create(path).unwrap();
    let mut z = zip::ZipWriter::new(f);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file(format!("{top}mod.toml"), opt).unwrap();
    z.write_all(manifest.as_bytes()).unwrap();
    for (rel, b) in files {
        z.start_file(format!("{top}{rel}"), opt).unwrap();
        z.write_all(b).unwrap();
    }
    z.finish().unwrap();
}

fn load(data: &Path) -> ModSet {
    ModSet::load(data, &LoadOptions { reserved: vec!["vp_nsn_gtr_08".into()], ..Default::default() })
}

#[test]
fn disc_lists_read() {
    let data = data_root();
    let d = DiscInfo::load(&data);
    assert!(d.found);
    assert_eq!(d.count(), 2, "DEVONLY dropped");
    assert_eq!(d.manufacturers, ["Audi", "Nissan"]);
    assert_eq!(d.row("VP_AUD_R8_08").unwrap().figures.mods_handling, 0.875);
    assert_eq!(d.samples["vp_aud_r8_08"].0, [0.580801, 0.707273, 0.822529]);
    // no lists: the disc's 69
    assert_eq!(DiscInfo::load(&tmp("empty")).count(), DISC_VEHICLE_COUNT);
}

#[test]
fn folder_mod_loads_and_list_round_trips() {
    let data = data_root();
    let m = mods_dir(&data);
    write_folder(&m.join("b_mod"), &manifest("b.mod", "1.0", "vp_zz_test_08", ""), &car_files("vp_zz_test_08"));
    write_folder(&m.join("a_mod"), &manifest("a.mod", "1.0", "vp_aa_test_08", ""), &car_files("vp_aa_test_08"));
    std::fs::write(m.join("readme.txt"), "not a mod").unwrap();
    let set = load(&data);
    assert_eq!(set.mods().len(), 2);
    for i in set.mods() {
        assert_eq!(i.state, ModState::Loaded, "{}: {:?}", i.id, i.reasons);
        assert!(i.warnings.is_empty(), "{:?}", i.warnings);
    }
    // name order after the disc's two cars
    assert_eq!(set.table_index("vp_aa_test_08"), Some(2));
    assert_eq!(set.table_index("VP_ZZ_TEST_08"), Some(3));
    let c = set.car("vp_aa_test_08").unwrap();
    assert_eq!((c.class.as_str(), c.manufacturer.as_str()), ("EXOTIC", "Audi"));
    assert_eq!(c.figures.base_speed, 210.0);
    // the extra list read back as the original's vehicle0003.lst
    let text = set.extra_list();
    assert!(text.starts_with("Vehicle\r\n{\r\n\tName\tvp_aa_test_08\r\n"), "{text}");
    assert!(text.contains("\tUnlockingGroup\tA0\r\n\tDistrictFilter\tHollywood\r\n\tDistrictFilter\tBeaches\r\n"));
    std::fs::write(data.join("tune/vehicle").join(EXTRA_LIST_NAME), &text).unwrap();
    let d = DiscInfo::load(&data);
    assert_eq!(d.rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["vp_aud_r8_08", "vp_nsn_350z_06", "vp_aa_test_08", "vp_zz_test_08"]);
    assert_eq!(d.row("vp_zz_test_08").unwrap().figures, d.row("vp_aud_r8_08").unwrap().figures);
    // strings
    assert!(set.strings().contains(&("vp_aa_test_08_shrt".to_string(), "08 TEST".to_string())));
}

#[test]
fn zip_mod_overlay_and_unpack() {
    let data = data_root();
    let m = mods_dir(&data);
    std::fs::create_dir_all(&m).unwrap();
    let car = "vp_zip_car_08";
    let zip_path = m.join("Zip Car.ZIP");
    write_zip(&zip_path, "zip_car/", &manifest("zip.car", "2.1.0", car, ""), &car_files(car));
    let mut set = load(&data);
    let info = &set.mods()[0];
    assert_eq!((info.state, info.is_zip, info.id.as_str()), (ModState::Loaded, true, "zip.car"), "{:?}", info.reasons);
    // plain and hashed names find the same file
    let plain = set.read(&format!("tune/garage/{car}.garage")).unwrap();
    assert_eq!(plain, b"Height 1.0\r\n");
    assert_eq!(set.read(&format!("TUNE\\garage\\0x{:08X}", joaat(&format!("{car}.garage")))).unwrap(), plain);
    let r9 = format!("resources/ui/textures/garage/dyntextures/showroom/0x{:08x}.r9", joaat(&format!("sh_{car}.xtd")));
    assert_eq!(set.read(&r9).unwrap(), b"showroom/sh_");
    assert!(set.find(&format!("resources/vehicle/{car}/0x00000006.r50")).unwrap().disk_path().is_none());
    assert_eq!(set.list_dir(&format!("resources/vehicle/{car}")).len(), 6);
    // unpacked for path readers; the second run reuses the cache
    let cache = cache_dir(&m);
    set.unpack_zips(&cache).unwrap();
    let p = set.find(&format!("resources/vehicle/{car}/0x00000006.r50")).unwrap().disk_path().unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"0x00000006.r50");
    std::fs::write(&p, b"marker").unwrap();
    let mut again = load(&data);
    assert_eq!(again.mods().len(), 1, "the cache is not a package");
    again.unpack_zips(&cache).unwrap();
    assert_eq!(again.read(&format!("resources/vehicle/{car}/0x00000006.r50")).unwrap(), b"marker", "stamp matched: not unpacked again");
}

#[test]
fn layered_priority() {
    let data = data_root();
    let car = "vp_lay_car_08";
    let mut files = car_files(car);
    files.push(("tune/vehicle/wheel_models.list".into(), b"mod".to_vec()));
    files.push(("tune/vehicle/only_in_mod.txt".into(), b"gap".to_vec()));
    write_folder(&mods_dir(&data).join("lay"), &manifest("lay", "1", car, ""), &files);
    std::fs::write(data.join("tune/vehicle/wheel_models.list"), b"disc").unwrap();
    let set = load(&data);
    assert_eq!(set.read_layered(&data, "tune/vehicle/wheel_models.list", Priority::ModFirst).unwrap(), b"mod");
    assert_eq!(set.read_layered(&data, "tune/vehicle/wheel_models.list", Priority::DiscFirst).unwrap(), b"disc");
    assert_eq!(set.read_layered(&data, "tune/vehicle/only_in_mod.txt", Priority::DiscFirst).unwrap(), b"gap");
    assert!(set.read_layered(&data, "tune/vehicle/nothing", Priority::ModFirst).is_none());
    assert!(matches!(set.locate(&data, "tune/vehicle/wheel_models.list", Priority::DiscFirst), Some(Located::Disc(_))));
}

#[test]
fn invalid_mod_reasons() {
    let data = data_root();
    let m = mods_dir(&data);
    let car = "vp_bad_car_08";
    let files: Vec<_> = car_files(car).into_iter().filter(|(r, _)| !r.ends_with(".r19") && !r.ends_with(".xcc") && !r.contains("mini_showroom")).collect();
    let text = manifest("bad", "1.0", car, "").replace("sound = \"vp_aud_r8_08\"\n", "").replace("figures_from = \"vp_aud_r8_08\"", "figures_from = \"vp_nope\"");
    write_folder(&m.join("bad"), &text, &files);
    // not a package at all / broken manifest
    std::fs::create_dir_all(m.join("empty")).unwrap();
    write_folder(&m.join("broken"), "[mod\nid=", &[]);
    let set = load(&data);
    let bad = set.mods().iter().find(|i| i.id == "bad").unwrap();
    assert_eq!(bad.state, ModState::Invalid);
    let rs = &bad.cars[0].reasons;
    for row in ["row 3:", "row 5:", "row 6: figures_from vp_nope", "row 16:", "row 19:"] {
        assert!(rs.iter().any(|r| r.starts_with(row)), "{row} missing in {rs:?}");
    }
    assert_eq!(rs.len(), 5, "{rs:?}");
    assert!(bad.reasons[0].starts_with("car vp_bad_car_08: "));
    for name in ["empty", "broken"] {
        let i = set.mods().iter().find(|i| i.id == name).unwrap();
        assert_eq!(i.state, ModState::Invalid);
        assert_eq!(i.reasons.len(), 1, "{:?}", i.reasons);
    }
    assert!(set.cars().is_empty() && set.extra_list().is_empty());
    assert!(set.read(&format!("tune/garage/{car}.garage")).is_none(), "an invalid mod's files are not overlaid");
}

#[test]
fn names_header_and_manifest_rules() {
    let data = data_root();
    let m = mods_dir(&data);
    write_folder(&m.join("disc"), &manifest("disc", "1", "vp_aud_r8_08", ""), &car_files("vp_aud_r8_08"));
    write_folder(&m.join("gtr"), &manifest("gtr", "1", "vp_nsn_gtr_08", ""), &car_files("vp_nsn_gtr_08"));
    write_folder(&m.join("hdr"), &manifest("Bad ID", "1.x", "va_car", ""), &[]);
    let typo = manifest("typo", "1", "vp_typo_08", "tunes = \"vp_aud_r8_08\"\n");
    write_folder(&m.join("typo"), &typo, &car_files("vp_typo_08"));
    let future = format!("{}\n[[part]]\nname = \"x\"\n", manifest("future", "1", "vp_future_08", ""));
    write_folder(&m.join("future"), &future, &car_files("vp_future_08"));
    let set = load(&data);
    let get = |id: &str| set.mods().iter().find(|i| i.id == id || i.path.ends_with(id)).unwrap();
    assert!(get("disc").reasons[0].contains("is a car of the game"));
    assert!(get("gtr").reasons[0].contains("the port adds itself"));
    let h = get("hdr");
    assert_eq!(h.reasons.len(), 2, "id and version: {:?}", h.reasons);
    assert!(get("typo").reasons[0].contains("tunes"), "{:?}", get("typo").reasons);
    let f = get("future");
    assert_eq!(f.state, ModState::Loaded);
    assert_eq!(f.warnings.len(), 1);
    assert_eq!(manifest::car_name_problem("va_car").unwrap(), "name must start with vp_ or vpd_");
    assert!(manifest::car_name_problem("vp_a_very_long_name_of_32_chars_").is_some());
}

#[test]
fn conflicts_and_duplicates() {
    let data = data_root();
    let m = mods_dir(&data);
    let car = "vp_same_08";
    write_folder(&m.join("one"), &manifest("aaa", "1", car, ""), &car_files(car));
    write_folder(&m.join("two"), &manifest("bbb", "1", car, ""), &car_files(car));
    // the same mod id twice: the newer (zip) is used, the folder copy is a duplicate
    write_folder(&m.join("old"), &manifest("ccc", "1.9", "vp_ccc_08", ""), &car_files("vp_ccc_08"));
    write_zip(&m.join("new.zip"), "", &manifest("ccc", "1.10", "vp_ccc_08", ""), &car_files("vp_ccc_08"));
    let set = load(&data);
    let st = |p: &str| set.mods().iter().find(|i| i.path.ends_with(p)).unwrap();
    assert_eq!(st("one").state, ModState::Loaded);
    assert_eq!(st("two").state, ModState::Conflict);
    assert_eq!(st("two").reasons, ["car vp_same_08 is already defined by mod aaa"]);
    assert_eq!(st("new.zip").state, ModState::Loaded);
    assert_eq!(st("old").state, ModState::Duplicate);
    assert_eq!(set.cars().iter().map(|c| (c.name.as_str(), c.mod_id.as_str())).collect::<Vec<_>>(), [("vp_ccc_08", "ccc"), ("vp_same_08", "aaa")]);
}

#[test]
fn on_off_in_settings() {
    let data = data_root();
    let settings = tmp("settings").join("settings.toml");
    std::fs::write(&settings, "schema = 1\n[display]\nvsync = false\n").unwrap();
    write_folder(&mods_dir(&data).join("x"), &manifest("x.mod", "1", "vp_x_08", ""), &car_files("vp_x_08"));
    set_enabled(&settings, "x.mod", false).unwrap();
    set_enabled(&settings, "other", false).unwrap();
    let off = disabled_ids(&settings);
    assert_eq!(off, HashSet::from(["x.mod".to_string(), "other".to_string()]));
    let text = std::fs::read_to_string(&settings).unwrap();
    assert!(text.contains("vsync = false"), "other keys kept: {text}");
    let set = ModSet::load(&data, &LoadOptions { disabled: off, ..Default::default() });
    assert_eq!(set.mods()[0].state, ModState::Disabled);
    assert!(set.cars().is_empty());
    set_enabled(&settings, "x.mod", true).unwrap();
    assert_eq!(disabled_ids(&settings), HashSet::from(["other".to_string()]));
    assert!(disabled_ids(&settings.with_file_name("missing.toml")).is_empty());
}

#[test]
fn update_rule_and_install() {
    assert_eq!(update_decision(None, "1.0"), UpdateDecision::Install);
    assert_eq!(update_decision(Some("1.0"), "1.0.0"), UpdateDecision::Replace { installed: "1.0".into() });
    assert_eq!(update_decision(Some("1.2"), "1.10"), UpdateDecision::Replace { installed: "1.2".into() });
    assert_eq!(update_decision(Some("2.0"), "1.9.9"), UpdateDecision::NeedsConfirmation { installed: "2.0".into() });
    let data = data_root();
    let m = mods_dir(&data);
    let src = tmp("downloads");
    let car = "vp_inst_08";
    let (v1, v2) = (src.join("inst-v1.zip"), src.join("inst-v2.zip"));
    write_zip(&v1, "inst/", &manifest("inst", "1.0", car, ""), &car_files(car));
    write_zip(&v2, "", &manifest("inst", "2.0", car, ""), &car_files(car));
    // an installed folder copy of v1 (put there by hand)
    write_folder(&m.join("inst_by_hand"), &manifest("inst", "1.0", car, ""), &car_files(car));
    let r = install_package(&m, &v2, false).unwrap();
    let target = m.join("inst.zip");
    assert_eq!(r, InstallOutcome::Installed { id: "inst".into(), version: "2.0".into(), path: target.clone(), replaced: vec![m.join("inst_by_hand")] });
    assert!(!m.join("inst_by_hand").exists());
    assert_eq!(load(&data).mods().len(), 1);
    // older over newer: nothing changes until confirmed
    assert_eq!(install_package(&m, &v1, false).unwrap(), InstallOutcome::NeedsConfirmation { id: "inst".into(), installed: "2.0".into(), incoming: "1.0".into() });
    assert_eq!(load(&data).mods()[0].version, "2.0");
    assert!(matches!(install_package(&m, &v1, true).unwrap(), InstallOutcome::Installed { ref version, .. } if version == "1.0"));
    let set = load(&data);
    assert_eq!((set.mods().len(), set.mods()[0].version.as_str(), set.mods()[0].state), (1, "1.0", ModState::Loaded));
    assert!(install_package(&m, &src, false).is_err());
}

#[test]
fn opponent_block_stub() {
    let data = data_root();
    let m = mods_dir(&data);
    let ok = "[car.opponent]\nenabled = true\nsamples_from = \"vp_aud_r8_08\"\ndriver = \"GH_12\"\nsetup = \"GH_12_2_vp_opp_08\"\nsetup_from = \"GH_26_3_vp_aud_r8_08\"\nplate = \"GODZILA\"\n";
    write_folder(&m.join("opp"), &manifest("opp", "1", "vp_opp_08", ok), &car_files("vp_opp_08"));
    let bad = "[car.opponent]\nenabled = true\nsetup = \"GH_12_2_vp_opp2_08\"\n";
    write_folder(&m.join("opp2"), &manifest("opp2", "1", "vp_opp2_08", bad), &car_files("vp_opp2_08"));
    let set = load(&data);
    let c = set.car("vp_opp_08").unwrap();
    assert_eq!(c.samples.unwrap().0, [0.580801, 0.707273, 0.822529]);
    assert_eq!(c.opponent.as_ref().unwrap().driver.as_deref(), Some("GH_12"));
    let r = &set.mods().iter().find(|i| i.id == "opp2").unwrap().cars[0].reasons;
    assert_eq!(r.len(), 3, "{r:?}");
    assert!(r.iter().any(|x| x.starts_with("row 14:")) && r.iter().any(|x| x.starts_with("row 15: opponent set-up")));
}

#[test]
fn overlay_keys() {
    let k = overlay::keys("Resources\\UI/x/mr_vp_a.r9");
    assert_eq!(k[0], "resources/ui/x/mr_vp_a.r9");
    assert_eq!(k[1], format!("resources/ui/x/0x{:08x}", joaat("mr_vp_a.r9")));
    assert_eq!(k[2], format!("resources/ui/x/0x{:08x}.r9", joaat("mr_vp_a.xtd")));
    assert_eq!(overlay::keys("tune/garage/0xABCDEF01"), ["tune/garage/0xabcdef01"]);
    assert_eq!(manifest::cmp_version("1.10", "1.9"), Ordering::Greater);
}

/// The real disc lists, when the extracted data is here (skipped elsewhere): 69 table cars, 33 manufacturers.
#[test]
fn real_disc_lists() {
    let root = std::env::var_os("MCLA_DATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("D:/MCLA/extracted/cache"));
    if !root.join("tune/vehicle").is_dir() {
        eprintln!("no data at {}: skipped", root.display());
        return;
    }
    let d = DiscInfo::load(&root);
    assert_eq!(d.count(), DISC_VEHICLE_COUNT);
    assert_eq!(d.rows[0].name, "vp_vw_golf_83");
    assert_eq!(d.rows[34].name, "vp_aud_r8_08");
    assert_eq!(d.manufacturers.len(), 33);
    assert_eq!(d.samples["vp_aud_r8_08"].0, [0.580801, 0.707273, 0.822529]);
    // the game's sound objects the row-16 checks resolve against
    assert!(d.audio_found);
    assert!(d.game_objects.contains(&joaat("DEFAULT_CAR_PROFILE")) && d.game_objects.contains(&joaat("VP_NSN_350Z_06_PROFILE")));
    assert_eq!(d.sound_objects.len(), 14_683);
}

/// A 22-byte profile object as the game's (type 12; +10 player DEFAULT, +14 opponent DEFAULT, +18 alternative).
fn profile(player: &str, opponent: &str) -> Vec<u8> {
    let mut o = vec![0u8; 22];
    o[0] = 12;
    o[10..14].copy_from_slice(&joaat(player).to_be_bytes());
    o[14..18].copy_from_slice(&joaat(opponent).to_be_bytes());
    o
}

/// The disc's sound files in the data root: one profile with its two DEFAULT objects, one sound.
fn disc_audio(data: &Path) {
    let cfg = data.join("audio/X360/config");
    std::fs::create_dir_all(&cfg).unwrap();
    let p = profile("VP_AUD_R8_08_DEFAULT", "VP_AUD_R8_08_O_DEFAULT");
    let g = mcla_formats::audcfg::Dat::build(0x16, &[("VP_AUD_R8_08_PROFILE", &p), ("VP_AUD_R8_08_DEFAULT", &[16]), ("VP_AUD_R8_08_O_DEFAULT", &[16])], &[]);
    std::fs::write(cfg.join("game.dat"), g).unwrap();
    std::fs::write(cfg.join("sounds.dat"), mcla_formats::audcfg::Dat::build(0x10, &[("DISC_SOUND", &[6])], &["VEHICLES\\VEHICLE_AUX"])).unwrap();
}

/// A mod that brings its own camera files and sound objects: no sound or camera donor needed, its files are found by
/// the readers' hashed names, a disc car's reads stay the disc's.
#[test]
fn own_cameras_and_sound() {
    let data = data_root();
    disc_audio(&data);
    let car = "vp_own_snd_08";
    let up = car.to_ascii_uppercase();
    let mut files = car_files(car);
    for v in ["boom1", "boom2", "boom3", "POV", "bumper", "reverse"] {
        files.push((format!("tune/camera/{car}_{v}.camboomcs"), format!("{v} own").into_bytes()));
    }
    files.push((format!("tune/camera/{car}_cinematic.camtrackcs"), b"cinematic own".to_vec()));
    files.push((format!("tune/camera/{car}_replay.mcreplaycamera"), b"replay own".to_vec()));
    // own profile -> own player DEFAULT, the disc R8's opponent DEFAULT (borrowing, as 15 disc profiles do)
    let (pd, od) = (format!("{up}_DEFAULT"), "VP_AUD_R8_08_O_DEFAULT".to_string());
    let game = mcla_formats::audcfg::Dat::build(0x16, &[(&format!("{up}_PROFILE"), &profile(&pd, &od)), (&pd, &[16, 0, 0])], &[]);
    let bank = format!("PLAYER_VEHICLES\\{up}");
    let sounds = mcla_formats::audcfg::Dat::build(0x10, &[(&format!("PLAYER_VEHICLES_{up}_IDLE"), &[12])], &[&bank]);
    files.push((GAME_DAT.into(), game));
    files.push((SOUNDS_DAT.into(), sounds));
    let bdir = format!("audio_pc/player_vehicles/{car}");
    files.push((format!("{bdir}/_index.tsv"), format!("idle_1000\t{:08x}\t100\t44100\t0\t0\t0\r\n", joaat("idle_1000")).into_bytes()));
    files.push((format!("{bdir}/idle_1000.wav"), b"RIFF".to_vec()));
    let text = manifest("own.snd", "1", car, "").replace("sound = \"vp_aud_r8_08\"\n", "").replace("cameras = \"vp_aud_r8_08\"\n", "");
    write_folder(&mods_dir(&data).join("own"), &text, &files);
    // a disc camera file of the R8
    let cam = data.join("tune/camera");
    std::fs::create_dir_all(&cam).unwrap();
    let r8 = format!("tune/camera/0x{:08x}", joaat("vp_aud_r8_08_boom1.camboomcs"));
    std::fs::write(data.join(&r8), b"r8 disc").unwrap();
    let set = load(&data);
    let i = &set.mods()[0];
    assert_eq!(i.state, ModState::Loaded, "{:?}", i.reasons);
    assert!(i.warnings.is_empty() && i.cars[0].warnings.is_empty(), "{:?} {:?}", i.warnings, i.cars[0].warnings);
    // cameras: the car's own, by the hashed name the loaders build (case as boomcam writes it: "_POV")
    assert_eq!(set.car_names(car, Donor::Cameras), [car]);
    let rel = format!("tune/camera/0x{:08x}", joaat(&format!("{car}_POV.camboomcs")));
    assert_eq!(runtime::read_text(Some(&set), &data, &rel).unwrap(), "POV own");
    let rel = format!("tune/camera/0x{:08x}", joaat(&format!("{car}_replay.mcreplaycamera")));
    assert_eq!(runtime::read_text(Some(&set), &data, &rel).unwrap(), "replay own");
    // a disc car: exactly the disc's file, with or without mods
    assert_eq!(runtime::read_data(Some(&set), &data, &r8).unwrap(), std::fs::read(data.join(&r8)).unwrap());
    assert_eq!(runtime::read_data(None, &data, &r8).unwrap(), b"r8 disc");
    assert_eq!(runtime::car_names(Some(&set), "vp_aud_r8_08", Donor::Cameras), ["vp_aud_r8_08"]);
    // sound: the mod's objects are read for merging, its wave bank is a folder on disk
    let each = set.read_each(GAME_DAT);
    assert_eq!(each.len(), 1);
    let g = mcla_formats::audcfg::Dat::parse(&each[0].1).unwrap();
    assert!(g.get_name(&runtime::profile_name(car)).is_some());
    let d = set.wave_bank_dir(&bank).unwrap();
    assert!(d.join("idle_1000.wav").is_file(), "{}", d.display());
    assert!(set.wave_bank_dir("PLAYER_VEHICLES\\VP_AUD_R8_08").is_none());
}

/// Donors are the fallback: a mod without its own cameras looks them up under the donor after its own name; the
/// sound rules: own profile, donor, the default profile (warning) or nothing (invalid).
#[test]
fn donor_fallback_and_sound_rules() {
    let data = data_root();
    disc_audio(&data);
    let m = mods_dir(&data);
    // cameras and sound from the donor (the S1 manifest)
    write_folder(&m.join("don"), &manifest("don", "1", "vp_don_08", ""), &car_files("vp_don_08"));
    // game.dat without the car's profile, no donor: the default profile plays (warning only)
    let no_donor = manifest("dflt", "1", "vp_dflt_08", "").replace("sound = \"vp_aud_r8_08\"\n", "");
    let mut f = car_files("vp_dflt_08");
    f.push((GAME_DAT.into(), mcla_formats::audcfg::Dat::build(0x16, &[("SOME_OTHER_OBJECT", &[16])], &[])));
    write_folder(&m.join("dflt"), &no_donor, &f);
    // a profile whose DEFAULT is nowhere, a bank whose listed wave is missing, an object the game already has
    let mut f = car_files("vp_brk_08");
    f.push((GAME_DAT.into(), mcla_formats::audcfg::Dat::build(0x16, &[("VP_BRK_08_PROFILE", &profile("VP_NOWHERE_DEFAULT", "VP_AUD_R8_08_O_DEFAULT")), ("VP_AUD_R8_08_DEFAULT", &[16])], &[])));
    f.push((SOUNDS_DAT.into(), mcla_formats::audcfg::Dat::build(0x10, &[("BRK", &[12])], &["PLAYER_VEHICLES\\VP_BRK_08", "VEHICLES\\VEHICLE_AUX"])));
    f.push(("audio_pc/player_vehicles/vp_brk_08/_index.tsv".into(), b"gone\t1234abcd\t1\t44100\t0\t-1\t0\n".to_vec()));
    write_folder(&m.join("brk"), &manifest("brk", "1", "vp_brk_08", "").replace("sound = \"vp_aud_r8_08\"\n", ""), &f);
    let set = load(&data);
    let get = |id: &str| set.mods().iter().find(|i| i.id == id).unwrap();
    assert_eq!(get("don").state, ModState::Loaded);
    assert_eq!(set.car_names("vp_don_08", Donor::Cameras), ["vp_don_08", "vp_aud_r8_08"]);
    assert_eq!(set.car_names("vp_don_08", Donor::Sound), ["vp_don_08", "vp_aud_r8_08"]);
    assert_eq!(set.car_names("vp_don_08", Donor::Garage), ["vp_don_08"], "own .garage, no garage donor");
    assert_eq!(get("dflt").state, ModState::Loaded, "{:?}", get("dflt").reasons);
    assert!(get("dflt").cars[0].warnings.iter().any(|w| w.starts_with("row 16:") && w.contains("default car profile")), "{:?}", get("dflt").cars[0].warnings);
    let b = get("brk");
    assert_eq!(b.state, ModState::Invalid);
    let rs = &b.cars[0].reasons;
    assert_eq!(rs.len(), 2, "{rs:?}");
    assert!(rs[0].contains("+10 (player DEFAULT") && rs[1].contains("gone.wav"), "{rs:?}");
    let ws = &b.cars[0].warnings;
    assert!(ws.iter().any(|w| w.contains("already in the game's") && w.contains("VP_AUD_R8_08_DEFAULT")), "{ws:?}");
    assert!(ws.iter().any(|w| w.contains("bank VEHICLES\\VEHICLE_AUX has no")), "{ws:?}");
}

/// Against the real game.dat (when the extracted data is here): a mod profile that only points at the R8's DEFAULT
/// objects (as 15 disc profiles borrow) is valid without a donor; one pointing at nothing is not.
#[test]
fn real_disc_profile_borrow() {
    let root = std::env::var_os("MCLA_DATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("D:/MCLA/extracted/cache"));
    if !root.join("audio/X360/config/game.dat").is_file() {
        eprintln!("no data at {}: skipped", root.display());
        return;
    }
    let disc = DiscInfo::load(&root);
    let m = tmp("realsnd");
    for (dir, car, def) in [("ok", "vp_borrow_08", "VP_AUD_R8_08_DEFAULT"), ("bad", "vp_nothing_08", "VP_NOTHING_08_DEFAULT")] {
        let up = car.to_ascii_uppercase();
        let mut f = car_files(car);
        f.push((GAME_DAT.into(), mcla_formats::audcfg::Dat::build(0x16, &[(&format!("{up}_PROFILE"), &profile(def, "VP_AUD_R8_08_O_DEFAULT"))], &[])));
        write_folder(&m.join(dir), &manifest(dir, "1", car, "").replace("sound = \"vp_aud_r8_08\"\n", ""), &f);
    }
    let set = ModSet::load_from(&m, &disc, &LoadOptions::default());
    let get = |id: &str| set.mods().iter().find(|i| i.id == id).unwrap();
    assert_eq!(get("ok").state, ModState::Loaded, "{:?}", get("ok").reasons);
    assert!(get("ok").cars[0].warnings.is_empty(), "{:?}", get("ok").cars[0].warnings);
    assert_eq!(get("bad").state, ModState::Invalid);
    assert!(get("bad").cars[0].reasons[0].contains("+10 (player DEFAULT"), "{:?}", get("bad").cars[0].reasons);
}
