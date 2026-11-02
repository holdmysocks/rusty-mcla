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

/// Stage S4: the opponent block resolved (districts, samples, several drivers, own or made set-ups) and checked against
/// the disc's `Opponent.lst` and set-up folder; the text the opponent pick gets.
#[test]
fn opponent_use_and_text() {
    let data = data_root();
    let cc = data.join("tune/career/carcfg");
    std::fs::create_dir_all(&cc).unwrap();
    std::fs::write(cc.join(format!("0x{:08x}", joaat("opponent.lst"))), "Character\r\n{\r\n\tName\tGH_12\r\n}\r\nCharacter\r\n{\r\n\tName\tGH_26\r\n\tCar\r\n\t{\r\n\t\tName\tvp_aud_r8_08\r\n\t}\r\n}\r\n").unwrap();
    let cfg = data.join("resources/vehicle/configs");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("GH_26_3_vp_aud_r8_08.xcc"), xcc("vp_aud_r8_08")).unwrap();
    let d = DiscInfo::load(&data);
    assert_eq!(d.drivers, ["GH_12", "GH_26"]);
    assert!(d.has_setup("gh_26_3_VP_AUD_R8_08") && !d.has_setup("GH_12_2_vp_aud_r8_08"));
    let m = mods_dir(&data);
    // one shorthand driver with a made set-up and a plate, one more with the mod's own set-up; own districts
    let ok = "[car.opponent]\nenabled = true\ndistricts = [\"downtown\"]\nperformance = [0.5, 0.6, 0.7]\nspeed = [0.4, 0.5, 0.6]\n\
              driver = \"GH_12\"\nsetup = \"GH_12_2_vp_opp_08\"\nsetup_from = \"GH_26_3_vp_aud_r8_08\"\nplate = \"OPP 08\"\n\
              [[car.opponent.drivers]]\ndriver = \"gh_26\"\nsetup = \"GH_26_1_vp_opp_08\"\n";
    let mut files = car_files("vp_opp_08");
    files.push(("resources/vehicle/configs/GH_26_1_vp_opp_08.xcc".into(), xcc("vp_opp_08")));
    write_folder(&m.join("opp"), &manifest("opp", "1", "vp_opp_08", ok), &files);
    // every rule of row 15 broken once
    let bad = "[car.opponent]\nenabled = true\nsamples_from = \"vp_aud_r8_08\"\ndriver = \"GH_99\"\nsetup = \"GH_99_1_vp_other_08\"\nsetup_from = \"vp_nope\"\nplate = \"toolong99\"\n\
               [[car.opponent.drivers]]\ndriver = \"GH_12\"\nsetup = \"GH_26_3_vp_aud_r8_08\"\nsetup_from = \"GH_26_3_vp_aud_r8_08\"\n\
               [[car.opponent.drivers]]\nsetup = \"GH_12_2_vp_opp2_08\"\n";
    write_folder(&m.join("opp2"), &manifest("opp2", "1", "vp_opp2_08", bad), &car_files("vp_opp2_08"));
    // marked, no driver at all (stage S4b: valid, the pick draws one), random rims; and a bad rims value
    write_folder(&m.join("opp3"), &manifest("opp3", "1", "vp_opp3_08", "[car.opponent]\nenabled = true\nsamples_from = \"vp_aud_r8_08\"\nrims = \"Random\"\n"), &car_files("vp_opp3_08"));
    write_folder(&m.join("opp5"), &manifest("opp5", "1", "vp_opp5_08", "[car.opponent]\nenabled = true\nsamples_from = \"vp_aud_r8_08\"\nrims = \"fancy\"\n"), &car_files("vp_opp5_08"));
    // not marked (enabled false): a car of the showroom only
    write_folder(&m.join("opp4"), &manifest("opp4", "1", "vp_opp4_08", "[car.opponent]\nenabled = false\n"), &car_files("vp_opp4_08"));
    let set = load(&data);
    let c = set.car("vp_opp_08").expect("loaded");
    let u = c.opponent_use.as_ref().unwrap();
    assert_eq!(u.districts, ["Downtown"]);
    assert_eq!(u.samples, ([0.5, 0.6, 0.7], [0.4, 0.5, 0.6]));
    assert_eq!(u.drivers.iter().map(|d| (d.driver.as_str(), d.setup.as_str())).collect::<Vec<_>>(), [("GH_12", "GH_12_2_vp_opp_08"), ("gh_26", "GH_26_1_vp_opp_08")]);
    let t = opponent_text(c, None).unwrap();
    assert!(t.starts_with("Vehicle\r\n{\r\n\tName\tvp_opp_08\r\n"));
    assert!(t.contains("\tDistrictFilter\tDowntown\r\n") && !t.contains("Hollywood"), "{t}");
    assert!(t.contains("Character\r\n{\r\n\tName\tGH_12\r\n\tCar\r\n\t{\r\n\t\tName\tvp_opp_08\r\n\t\tConfig\r\n\t\t{\r\n\t\t\tName\tGH_12_2_vp_opp_08\r\n"));
    assert!(t.contains("\tName\tgh_26\r\n") && t.contains("\tName\tGH_26_1_vp_opp_08\r\n"));
    assert!(t.contains("sample\r\n{\r\n\tname\tvp_opp_08\r\n\tmod\t100\r\n\tperformance\t0.7\r\n\tspeed\t0.6\r\n}"));
    // only the drivers the game could make
    let one = opponent_text(c, Some(&[("GH_12".into(), "GH_12_2_vp_opp_08".into())])).unwrap();
    assert!(!one.contains("gh_26"));
    assert!(opponent_text(c, Some(&[])).is_none());
    let reasons = |id: &str| set.mods().iter().find(|i| i.id == id).unwrap().cars[0].reasons.clone();
    let r = reasons("opp2");
    for want in [
        "row 15: driver GH_99 is not a Character",
        "row 15: set-up name GH_99_1_vp_other_08 must end with _vp_opp2_08",
        "row 15: setup_from vp_nope is not a set-up of the game",
        "row 15: plate \"toolong99\"",
        "row 15: set-up name GH_26_3_vp_aud_r8_08 must end",
        "row 15: set-up name GH_26_3_vp_aud_r8_08 is a set-up of the game",
        "row 15: set-up \"GH_12_2_vp_opp2_08\" has no driver",
        "row 15: opponent set-up GH_12_2_vp_opp2_08 has no",
    ] {
        assert!(r.iter().any(|x| x.starts_with(want)), "{want}: {r:?}");
    }
    assert_eq!(r.len(), 8, "{r:?}");
    let c3 = set.car("vp_opp3_08").expect("a driverless opponent car is valid");
    let u3 = c3.opponent_use.as_ref().unwrap();
    assert!(u3.drivers.is_empty() && u3.rims_random && !u.rims_random);
    let t3 = opponent_text(c3, None).unwrap();
    assert!(t3.contains("\tName\tvp_opp3_08\r\n") && !t3.contains("Character") && t3.contains("performance"), "{t3}");
    let r5 = reasons("opp5");
    assert!(r5.len() == 1 && r5[0].starts_with("opponent: rims \"fancy\""), "{r5:?}");
    let c4 = set.car("vp_opp4_08").expect("loaded");
    assert!(c4.opponent_use.is_none() && opponent_text(c4, None).is_none());
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

/// Price scaling of the donor's list (DECISIONS.md D28 "Prices"): `prices_level` (a disc car) or `prices_kit` (a
/// body-kit price) with `prices_maker`; it needs a prices donor, one of the two, sane values; an own `_parts.gtun`
/// is used as written (the fields only warn).
#[test]
fn price_scaling_fields() {
    let data = data_root();
    let m = mods_dir(&data);
    let car = |c: &str| car_files(c);
    write_folder(&m.join("lvl"), &manifest("lvl", "1", "vp_lvl_08", "prices_level = \"vp_aud_r8_08\"\nprices_maker = 52\n"), &car("vp_lvl_08"));
    write_folder(&m.join("kit"), &manifest("kit", "1", "vp_kit_08", "prices_kit = 30600\n"), &car("vp_kit_08"));
    write_folder(&m.join("both"), &manifest("both", "1", "vp_both_08", "prices_level = \"vp_aud_r8_08\"\nprices_kit = 0\nprices_maker = -1\n"), &car("vp_both_08"));
    write_folder(&m.join("nodonor"), &manifest("nodonor", "1", "vp_nod_08", "prices_level = \"vp_nope_08\"\n").replace("prices = \"vp_aud_r8_08\"\n", ""), &car("vp_nod_08"));
    let mut own = car("vp_own_08");
    own.push(("tune/garage/vp_own_08_parts.gtun".into(), b"<!-- body -->\r\n1\r\n".to_vec()));
    write_folder(&m.join("own"), &manifest("own", "1", "vp_own_08", "prices_kit = 30600\n"), &own);
    let set = load(&data);
    let get = |id: &str| set.mods().iter().find(|i| i.id == id).unwrap();
    for id in ["lvl", "kit", "own"] {
        assert_eq!(get(id).state, ModState::Loaded, "{id}: {:?}", get(id).reasons);
    }
    let lvl = set.car("vp_lvl_08").unwrap();
    assert_eq!((lvl.donors.prices_level.as_deref(), lvl.donors.prices_maker), (Some("vp_aud_r8_08"), Some(52)));
    assert_eq!(set.car("vp_kit_08").unwrap().donors.prices_kit, Some(30600));
    assert!(get("own").cars[0].warnings.iter().any(|w| w.starts_with("row 11: own") && w.contains("used as written")), "{:?}", get("own").cars[0].warnings);
    let rs = &get("both").cars[0].reasons;
    assert_eq!(rs.len(), 3, "{rs:?}");
    assert!(rs[0].contains("not both") && rs[1].contains("prices_kit must be above 0") && rs[2].contains("prices_maker"), "{rs:?}");
    let rs = &get("nodonor").cars[0].reasons;
    assert!(rs.iter().any(|r| r.contains("no tune/garage/vp_nod_08_parts.gtun and no prices donor")), "{rs:?}");
    assert!(rs.iter().any(|r| r.contains("name a prices donor")), "{rs:?}");
    assert!(rs.iter().any(|r| r.contains("prices_level): donor vp_nope_08 is not a car")), "{rs:?}");
}

/// The sample mod "R8 (SAMPLE)" (stage S6) built from the real data folder (skipped without it): valid with no error and
/// no warning, the car's row and names as the disc R8's, nothing named from the game but the unread animation key; the
/// package holds no file under the disc R8's name; the update rule replaces it in place (a higher version over it), an
/// older version asks first.
#[test]
fn r8_sample_is_valid_and_updates_in_place() {
    let root = std::env::var_os("MCLA_DATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("D:/MCLA/extracted/cache"));
    if !root.join("resources/vehicle/vp_aud_r8_08").is_dir() {
        eprintln!("no data at {}: skipped", root.display());
        return;
    }
    let disc = DiscInfo::load(&root);
    let work = tmp("sample");
    let zip1 = work.join("sample_r8_1.zip");
    let w = sample::write_r8_sample(&root, &zip1, "1.0.0").unwrap();
    assert!(w.files > 130, "{} files", w.files);
    let mods = work.join("mods");
    let opts = LoadOptions { reserved: vec!["vp_nsn_gtr_08".into()], ..Default::default() };
    assert!(matches!(install_package(&mods, &zip1, false).unwrap(), InstallOutcome::Installed { .. }));
    let set = ModSet::load_from(&mods, &disc, &opts);
    let i = &set.mods()[0];
    assert_eq!((i.id.as_str(), i.name.as_str(), i.state), ("sample.r8", "R8 (SAMPLE)", ModState::Loaded), "{:?}", i.reasons);
    assert!(i.reasons.is_empty() && i.warnings.is_empty() && i.cars[0].warnings.is_empty(), "{:?} {:?} {:?}", i.reasons, i.warnings, i.cars[0].warnings);
    let car = set.car(sample::SAMPLE_CAR).unwrap();
    let d = &car.donors;
    // (no donor but the animation key)
    assert_eq!((&d.tune, &d.setup, &d.rims, &d.brake, &d.garage, &d.prices, &d.sound, &d.cameras, &d.pictures), (&None, &None, &None, &None, &None, &None, &None, &None, &None));
    assert_eq!(car.index, DISC_VEHICLE_COUNT);
    let block = vehicle_block(car);
    for want in ["vp_aud_r8_08_sample", "EXOTIC", "Audi", "118000", "0.875"] {
        assert!(block.contains(want), "{want} in {block}");
    }
    // every file the loader resolves for the sample is the disc R8's file, byte for byte (set-up: the same but for the
    // car's hash and names and the rim byte 255; sound: the profile object under the sample's name)
    let same = |rel: &str, disc: PathBuf| {
        let m = set.find(rel).unwrap_or_else(|| panic!("{rel} not in the package")).read().unwrap();
        assert_eq!(m, std::fs::read(&disc).unwrap(), "{rel}");
    };
    let mut n = 0;
    for e in std::fs::read_dir(root.join("resources/vehicle/vp_aud_r8_08")).unwrap().filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        if [".r63", ".r83", ".r19", ".r50"].iter().any(|x| name.ends_with(x)) {
            same(&format!("resources/vehicle/vp_aud_r8_08_sample/{name}"), e.path());
            n += 1;
        }
    }
    assert_eq!(n, 130, "126 r63 + 2 r83 + r19 + r50");
    for e in std::fs::read_dir(root.join("resources/rims/whl_stk_aud_r8_08")).unwrap().filter_map(|e| e.ok()) {
        same(&format!("resources/rims/whl_stk_aud_r8_08_sample/{}", e.file_name().to_string_lossy()), e.path());
    }
    for (sub, p) in [("managerides", "mr_"), ("showroom", "sh_"), ("mini_showroom", "msh_")] {
        same(&format!("resources/ui/textures/garage/dyntextures/{sub}/{p}vp_aud_r8_08_sample.r9"), root.join(format!("resources/ui/textures/garage/dyntextures/{sub}/{p}vp_aud_r8_08.r9")));
    }
    let hashed = |dir: &str, name: &str| root.join(format!("tune/{dir}/0x{:08x}", joaat(name)));
    for (dir, suffix) in [("garage", ".garage"), ("garage", "_parts.gtun"), ("camera", "_boom1.camboomcs"), ("camera", "_boom2.camboomcs"), ("camera", "_boom3.camboomcs"), ("camera", "_POV.camboomcs"), ("camera", "_bumper.camboomcs"), ("camera", "_reverse.camboomcs"), ("camera", "_cinematic.camtrackcs"), ("camera", "_replay.mcreplaycamera")] {
        // (the readers' own route: the hashed name of the plain file name, disc first, then the mods)
        let rel = format!("tune/{dir}/0x{:08x}", joaat(&format!("vp_aud_r8_08_sample{suffix}")));
        assert_eq!(runtime::read_data(Some(&set), &root, &rel).unwrap(), std::fs::read(hashed(dir, &format!("vp_aud_r8_08{suffix}"))).unwrap(), "{suffix}");
    }
    // the set-up: the R8's with the sample's hash and names and rim 255, nothing else
    let (r8x, smx) = (std::fs::read(root.join("resources/vehicle/configs/vp_aud_r8_08.xcc")).unwrap(), set.find("resources/vehicle/configs/vp_aud_r8_08_sample.xcc").unwrap().read().unwrap());
    let diff: Vec<usize> = (0..r8x.len()).filter(|&k| r8x[k] != smx[k]).collect();
    assert!(diff.iter().all(|&k| k == 0x844 || (0x1FA8..0x1FAC).contains(&k) || (0x1FAD..0x1FAD + 32).contains(&k) || (0x1FCD..0x1FCD + 32).contains(&k)), "{diff:x?}");
    assert_eq!((r8x[0x844], smx[0x844], r8x[0x846], smx[0x846]), (143, 255, 14, 14));
    // the sound profile: the sample's `_PROFILE` is the R8's object byte for byte
    let game = mcla_formats::audcfg::Dat::load(&root.join("audio/X360/config/game.dat")).unwrap();
    let own = set.read_each(GAME_DAT);
    assert_eq!(own.len(), 1);
    let own = mcla_formats::audcfg::Dat::parse(&own[0].1).unwrap();
    assert_eq!(own.len(), 1);
    assert_eq!(own.get_name("VP_AUD_R8_08_SAMPLE_PROFILE").unwrap(), game.get_name("VP_AUD_R8_08_PROFILE").unwrap());
    assert!(game.get_name("VP_AUD_R8_08_SAMPLE_PROFILE").is_none());
    assert_eq!(set.car_names(sample::SAMPLE_CAR, Donor::Cameras), [sample::SAMPLE_CAR], "no camera donor");
    assert_eq!(set.car_names(sample::SAMPLE_CAR, Donor::Sound), [sample::SAMPLE_CAR], "no sound donor");
    // the update rule: the same id with a higher version replaces the zip in place; an older one asks first
    let zip2 = work.join("sample_r8_2.zip");
    sample::write_r8_sample(&root, &zip2, "1.1.0").unwrap();
    match install_package(&mods, &zip2, false).unwrap() {
        InstallOutcome::Installed { id, version, path, replaced } => {
            assert_eq!((id.as_str(), version.as_str()), ("sample.r8", "1.1.0"));
            assert_eq!(path, mods.join("sample.r8.zip"));
            assert!(replaced.is_empty(), "the same file is replaced in place");
        }
        o => panic!("{o:?}"),
    }
    let n = std::fs::read_dir(&mods).unwrap().filter_map(|e| e.ok()).filter(|e| !e.file_name().to_string_lossy().starts_with('.')).count();
    assert_eq!(n, 1, "one package after the update");
    let set = ModSet::load_from(&mods, &disc, &opts);
    assert_eq!((set.mods().len(), set.mods()[0].version.as_str()), (1, "1.1.0"));
    assert!(matches!(install_package(&mods, &zip1, false).unwrap(), InstallOutcome::NeedsConfirmation { .. }));
    assert_eq!(ModSet::load_from(&mods, &disc, &opts).mods()[0].version, "1.1.0", "nothing changed by the refused install");
    let _ = std::fs::remove_dir_all(&work);
}

// ---- rim mods (M-6) ----

/// A rim shop in the shape of the disc's scripts: BBS (paid: 2000, 4000, 6000 -> median 4000), VOLK (1000, 1002), one
/// bike brand (paid: 1000, 3000 -> median 2000) and a tab of free download rims.
const RIM_SC: &str = r#"
<state id="rim_bbs"><data name="fs_mfr" expr="78"></data><onenter expr="nav.setFlashVar('cur_tab',1)"></onenter>
 <state><data name="fs_text1" expr="'whl_am_bbs_a'"></data><data name="fs_text2" expr="2000"></data><data name="index" expr="10"></data></state>
 <state><data name="fs_text1" expr="'whl_am_bbs_b'"></data><data name="fs_text2" expr="4000"></data><data name="index" expr="11"></data></state>
 <state><data name="fs_text1" expr="'whl_am_bbs_c'"></data><data name="fs_text2" expr="6000"></data><data name="index" expr="12"></data></state>
</state>
<state id="rim_volk"><data name="fs_mfr" expr="30"></data><onenter expr="nav.setFlashVar('cur_tab',2)"></onenter>
 <state><data name="fs_text1" expr="'whl_am_volk_a'"></data><data name="fs_text2" expr="1000"></data><data name="index" expr="20"></data></state>
 <state><data name="fs_text1" expr="'whl_am_volk_b'"></data><data name="fs_text2" expr="1002"></data><data name="index" expr="21"></data></state>
</state>"#;
const RIM_BIKE: &str = r#"
<state id="rim_rc_bike"><data name="fs_mfr" expr="79"></data><onenter expr="nav.setFlashVar('cur_tab',1)"></onenter>
 <state><data name="fs_text1" expr="'whl_am_rc_a'"></data><data name="fs_text2" expr="1000"></data><data name="index" expr="30"></data></state>
 <state><data name="fs_text1" expr="'whl_am_rc_b'"></data><data name="fs_text2" expr="3000"></data><data name="index" expr="31"></data></state>
</state>"#;
const RIM_DLC: &str = r#"
<state id="rim_Dayton_DLC"><data name="fs_mfr" expr="1"></data><onenter expr="nav.setFlashVar('cur_tab',1)"></onenter>
 <state><data name="fs_text1" expr="'whld_am_dytn_a'"></data><data name="fs_text2" expr="0"></data><data name="index" expr="40"></data></state>
</state>"#;

/// `data_root()` with the three rim shop scripts and a few disc rim folders.
fn data_root_rims() -> PathBuf {
    let d = data_root();
    let e = d.join("tune/ui/garage/elements");
    std::fs::create_dir_all(&e).unwrap();
    std::fs::write(e.join("Rim.sc.xml"), RIM_SC).unwrap();
    std::fs::write(e.join("RimBike.sc.xml"), RIM_BIKE).unwrap();
    std::fs::write(e.join("DLC_Rim.sc.xml"), RIM_DLC).unwrap();
    for f in ["whl_am_bbs_a", "whl_stk_aud_r8_08"] {
        std::fs::create_dir_all(d.join("resources/rims").join(f)).unwrap();
    }
    d
}

/// A rim mod's manifest: `entries` are `[[rim]]` blocks written out.
fn rim_manifest(id: &str, version: &str, entries: &str) -> String {
    format!("[mod]\nid = \"{id}\"\nversion = \"{version}\"\nname = \"Rims {id}\"\n{entries}")
}

fn rim_block(folder: &str, extra: &str) -> String {
    format!("\n[[rim]]\nfolder = \"{folder}\"\n{extra}\n")
}

/// The files of a rim: a test model with zones 1, 2, 3 and a look.
fn rim_files(folder: &str) -> Vec<(String, Vec<u8>)> {
    vec![(format!("resources/rims/{folder}/0x00000001.r63"), b"TESTRIM\x01\x02\x03".to_vec()), (format!("resources/rims/{folder}/0x00000002.r83"), b"look".to_vec())]
}

fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b
}

#[test]
fn rim_price_rule_and_join() {
    let shop = rims::RimShop { brands: [rims::parse(RIM_SC, rims::Shop::Car), rims::parse(RIM_BIKE, rims::Shop::Bike), rims::parse(RIM_DLC, rims::Shop::Dlc)].concat() };
    // medians rounded to a multiple of 4, halves up
    assert_eq!(rims::median_price(vec![2000, 4000, 6000]), 4000);
    assert_eq!(rims::median_price(vec![1000, 1002]), 1000, "1001 -> 1000");
    assert_eq!(rims::median_price(vec![1000, 1010]), 1004, "1005 -> 1004");
    assert_eq!(rims::median_price(vec![1002, 1004]), 1004, "1003 -> 1004");
    assert_eq!(rims::median_price(vec![1006, 1010]), 1008, "1008 stays");
    assert_eq!(rims::median_price(vec![]), 0);
    // joining a tab: its own median; a new brand or a free tab: all paid rims of the kind
    let join = |b: &str, bike| rims::resolve_join(&shop, b, bike).unwrap();
    assert_eq!(join("BBS", false), rims::Join::Disc("bbs".into()), "any case");
    assert_eq!(rims::computed_price(&shop, &join("bbs", false), false), 4000);
    assert_eq!(rims::computed_price(&shop, &join("volk", false), false), 1000);
    // all paid car rims: 1000 1002 2000 4000 6000 -> median 2000
    assert_eq!(rims::computed_price(&shop, &join("Acme", false), false), 2000);
    assert_eq!(rims::computed_price(&shop, &join("dayton_dlc", false), false), 2000, "a tab of free rims");
    // bikes: the bike tabs only (1000, 3000 -> 2000)
    assert_eq!(rims::computed_price(&shop, &join("Acme", true), true), 2000);
    assert_eq!(rims::computed_price(&shop, &join("rc_bike", true), true), 2000);
    // a rim never goes into a tab of the other kind
    assert!(rims::resolve_join(&shop, "rc_bike", false).unwrap_err().contains("bike tab"));
    assert!(rims::resolve_join(&shop, "bbs", true).unwrap_err().contains("car tab"));
}

#[test]
fn rim_mods_load_with_only_rims_and_join_the_shop() {
    let data = data_root_rims();
    let m = mods_dir(&data);
    // mod "b": a rim into BBS (price given), a free rim into VOLK, a bike rim and two rims of a new brand "Acme"
    let man_b = rim_manifest(
        "b.rims",
        "1.0",
        &[
            rim_block("whl_am_zz_one", "brand = \"bbs\"\nprice = 777\n"),
            rim_block("whl_am_zz_free", "brand = \"volk\"\nprice = 0\nlabel = \"Free One\"\n"),
            rim_block("whl_am_zz_bike", "brand = \"Acme\"\nbike = true\nlogo = \"logo.png\"\n"),
            rim_block("whl_am_acme_two", "brand = \"Acme\"\nlogo = \"logo.png\"\n"),
            rim_block("whl_am_acme_one", "brand = \"acme\"\n"),
        ]
        .concat(),
    );
    let mut files_b: Vec<(String, Vec<u8>)> = ["whl_am_zz_one", "whl_am_zz_free", "whl_am_zz_bike", "whl_am_acme_two", "whl_am_acme_one"].iter().flat_map(|f| rim_files(f)).collect();
    files_b.push(("logo.png".into(), png_bytes(64, 32)));
    write_folder(&m.join("b_mod"), &man_b, &files_b);
    // mod "a": a rim of a brand "Zeta" (a new tab after Acme's), no price
    write_folder(&m.join("a_mod"), &rim_manifest("a.rims", "1.0", &rim_block("whl_am_aa_zeta", "brand = \"Zeta\"\n")), &rim_files("whl_am_aa_zeta"));
    let set = load(&data);
    for i in set.mods() {
        assert_eq!(i.state, ModState::Loaded, "{}: {:?}", i.id, i.reasons);
    }
    assert!(set.cars().is_empty(), "a mod with only rims is valid");
    // load order: mod id (a, then b), each manifest's order
    let order: Vec<&str> = set.rims().iter().map(|r| r.folder.as_str()).collect();
    assert_eq!(order, ["whl_am_aa_zeta", "whl_am_zz_one", "whl_am_zz_free", "whl_am_zz_bike", "whl_am_acme_two", "whl_am_acme_one"]);
    // rim numbers: 175 + place in folder name order over all rims
    let num = |f: &str| set.rim(f).unwrap().index;
    assert_eq!([num("whl_am_aa_zeta"), num("whl_am_acme_one"), num("whl_am_acme_two"), num("whl_am_zz_bike"), num("whl_am_zz_free"), num("whl_am_zz_one")], [175, 176, 177, 178, 179, 180]);
    // prices: given as given (0 is free); absent = the median (a new brand: all paid car rims 2000; the bike rim: the bike tabs 2000)
    let price = |f: &str| set.rim(f).unwrap().price;
    assert_eq!((price("whl_am_zz_one"), price("whl_am_zz_free")), (777, 0));
    assert_eq!((price("whl_am_aa_zeta"), price("whl_am_acme_one"), price("whl_am_zz_bike")), (2000, 2000, 2000));
    // the launcher's view: final prices, whether given, the number
    let b = set.mods().iter().find(|m| m.id == "b.rims").unwrap();
    let st = |f: &str| b.rims.iter().find(|r| r.folder == f).unwrap();
    assert!(st("whl_am_zz_one").price_given && !st("whl_am_acme_one").price_given);
    assert_eq!((st("whl_am_acme_one").index, st("whl_am_acme_one").price), (Some(176), 2000));
    assert_eq!(st("whl_am_zz_free").label, "Free One");
    assert_eq!(st("whl_am_acme_one").label, "ACME ONE");
    // warnings: the new brand's first rim of mod b has no logo itself (the brand is the same tab, but each rim says)
    assert!(st("whl_am_acme_one").warnings.iter().any(|w| w.contains("no logo")), "{:?}", st("whl_am_acme_one").warnings);
    assert!(st("whl_am_acme_two").warnings.is_empty(), "{:?}", st("whl_am_acme_two").warnings);
    // the shop: BBS gains its rim after the disc's three, VOLK the free one; new brands after the disc's tabs, by name
    let disc = DiscInfo::load(&data);
    let shop = rims::merge(disc.shop.clone(), set.rims());
    let bbs = shop.brands.iter().find(|b| b.id == "bbs").unwrap();
    assert_eq!(bbs.rims.iter().map(|r| r.resource.as_str()).collect::<Vec<_>>(), ["whl_am_bbs_a", "whl_am_bbs_b", "whl_am_bbs_c", "whl_am_zz_one"]);
    let ids: Vec<(&str, rims::Shop, usize)> = shop.brands.iter().map(|b| (b.id.as_str(), b.shop, b.rims.len())).collect();
    assert_eq!(&ids[ids.len() - 3..], [("acme", rims::Shop::Car, 2), ("acme", rims::Shop::Bike, 1), ("zeta", rims::Shop::Car, 1)], "after all disc tabs, in name order");
    assert!(shop.brands[..ids.len() - 3].iter().all(|b| b.custom.is_none()));
    let acme = shop.brands.iter().find(|b| b.id == "acme" && b.shop == rims::Shop::Car).unwrap();
    assert_eq!(acme.rims.iter().map(|r| r.resource.as_str()).collect::<Vec<_>>(), ["whl_am_acme_two", "whl_am_acme_one"], "the manifest's order");
    assert!(acme.custom.as_ref().unwrap().logo.is_some(), "the logo of the tab comes from any rim that gave one");
    assert_eq!(acme.custom.as_ref().unwrap().name, "Acme");
    // a bike sees the bike tabs and new brands of its kind; a car does not see the bike ones
    assert!(shop.tabs(false).iter().all(|b| b.shop != rims::Shop::Bike));
    assert_eq!(shop.tabs(true).iter().filter(|b| b.id == "acme").count(), 2);
    // car_rims (the opponents' random pool) holds the car and download rims with the mods' numbers, no bike rim
    let pool: Vec<i32> = shop.car_rims().iter().map(|r| r.0).collect();
    assert!(pool.contains(&175) && pool.contains(&177) && !pool.contains(&178), "{pool:?}");
    // a disabled mod brings nothing
    let mut dis = std::collections::HashSet::new();
    dis.insert("b.rims".to_string());
    let set2 = ModSet::load(&data, &LoadOptions { disabled: dis, ..Default::default() });
    assert_eq!(set2.rims().len(), 1);
    assert_eq!(set2.rim("whl_am_aa_zeta").unwrap().index, 175, "the numbers follow what is loaded");
}

#[test]
fn rim_mod_reasons_warnings_and_clashes() {
    let data = data_root_rims();
    let m = mods_dir(&data);
    let one = |dir: &str, id: &str, entries: String, files: Vec<(String, Vec<u8>)>| write_folder(&m.join(dir), &rim_manifest(id, "1.0", &entries), &files);
    // clash with a disc rim folder (the script's or the resources folder)
    one("m1", "m1", rim_block("whl_am_bbs_a", "brand = \"bbs\"\n"), rim_files("whl_am_bbs_a"));
    // bad names
    one("m2", "m2", rim_block("rim_x", "brand = \"bbs\"\n"), rim_files("rim_x"));
    one("m3", "m3", rim_block("whl_am_Upper", "brand = \"bbs\"\n"), rim_files("whl_am_Upper"));
    one("m4", "m4", rim_block("whl_am_a_very_long_folder_name_over_limit", "brand = \"bbs\"\n"), rim_files("whl_am_a_very_long_folder_name_over_limit"));
    // no files / two models / unreadable model / no zones
    one("m5", "m5", rim_block("whl_am_nofiles", "brand = \"bbs\"\n"), vec![]);
    let mut two = rim_files("whl_am_two");
    two.push(("resources/rims/whl_am_two/0x00000009.r63".into(), b"TESTRIM\x01".to_vec()));
    one("m6", "m6", rim_block("whl_am_two", "brand = \"bbs\"\n"), two);
    let mut bad = rim_files("whl_am_bad");
    bad[0].1 = b"not a model".to_vec();
    one("m7", "m7", rim_block("whl_am_bad", "brand = \"bbs\"\n"), bad);
    let mut nozone = rim_files("whl_am_nozone");
    nozone[0].1 = b"TESTRIM".to_vec();
    one("m8", "m8", rim_block("whl_am_nozone", "brand = \"bbs\"\n"), nozone);
    // brand of the other kind, bad price, missing / bad logo, logo on a disc brand
    one("m9", "m9", rim_block("whl_am_kind", "brand = \"rc_bike\"\n"), rim_files("whl_am_kind"));
    one("n1", "n1", rim_block("whl_am_neg", "brand = \"bbs\"\nprice = -5\n"), rim_files("whl_am_neg"));
    one("n2", "n2", rim_block("whl_am_nologo", "brand = \"New\"\nlogo = \"nope.png\"\n"), rim_files("whl_am_nologo"));
    let mut notpng = rim_files("whl_am_notpng");
    notpng.push(("l.png".into(), b"GIF89a".to_vec()));
    one("n3", "n3", rim_block("whl_am_notpng", "brand = \"New\"\nlogo = \"l.png\"\n"), notpng);
    let mut ondisc = rim_files("whl_am_ondisc");
    ondisc.push(("l.png".into(), png_bytes(8, 8)));
    one("n4", "n4", rim_block("whl_am_ondisc", "brand = \"bbs\"\nlogo = \"l.png\"\n"), ondisc);
    // twice in one manifest; a mod with neither car nor rim
    one("n5", "n5", [rim_block("whl_am_dup", "brand = \"bbs\"\n"), rim_block("whl_am_dup", "brand = \"bbs\"\n")].concat(), rim_files("whl_am_dup"));
    one("n6", "n6", String::new(), vec![]);
    // two mods with one folder: the second (by id) is a conflict and not loaded
    one("o1", "o1", rim_block("whl_am_same", "brand = \"bbs\"\n"), rim_files("whl_am_same"));
    one("o2", "o2", rim_block("whl_am_same", "brand = \"volk\"\n"), rim_files("whl_am_same"));
    let set = load(&data);
    let get = |id: &str| set.mods().iter().find(|m| m.id == id).unwrap();
    let why = |id: &str| get(id).reasons.join(" | ");
    assert_eq!(get("m1").state, ModState::Invalid);
    assert!(why("m1").contains("rim folder of the game"), "{}", why("m1"));
    assert!(why("m2").contains("whl_am_"), "{}", why("m2"));
    assert!(why("m3").contains("lower-case"), "{}", why("m3"));
    assert!(why("m4").contains("longer than 31"), "{}", why("m4"));
    assert!(why("m5").contains("one .r63") && why("m5").contains("one .r83"), "{}", why("m5"));
    assert!(why("m6").contains("one .r63 model (found 2)"), "{}", why("m6"));
    assert!(why("m7").contains("unreadable"), "{}", why("m7"));
    // no rim zones: loaded, with a warning
    assert_eq!(get("m8").state, ModState::Loaded, "{}", why("m8"));
    assert!(get("m8").rims[0].warnings.iter().any(|w| w.contains("no rim zones")), "{:?}", get("m8").rims[0].warnings);
    assert!(why("m9").contains("bike tab"), "{}", why("m9"));
    assert!(why("n1").contains("negative"), "{}", why("n1"));
    assert!(why("n2").contains("not in the package"), "{}", why("n2"));
    assert!(why("n3").contains("not a PNG"), "{}", why("n3"));
    assert_eq!(get("n4").state, ModState::Loaded);
    assert!(get("n4").rims[0].warnings.iter().any(|w| w.contains("ignored")), "{:?}", get("n4").rims[0].warnings);
    assert!(why("n5").contains("defined twice"), "{}", why("n5"));
    assert!(why("n6").contains("no car and no rim"), "{}", why("n6"));
    assert_eq!(get("o1").state, ModState::Loaded);
    assert_eq!(get("o2").state, ModState::Conflict);
    assert!(why("o2").contains("rim whl_am_same is already defined by mod o1"), "{}", why("o2"));
    assert_eq!(set.rims().iter().filter(|r| r.folder == "whl_am_same").count(), 1);
    assert_eq!(set.rim("whl_am_same").unwrap().mod_id, "o1");
}

#[test]
fn rim_mod_zip_unpacks_its_folder() {
    let data = data_root_rims();
    let m = mods_dir(&data);
    std::fs::create_dir_all(&m).unwrap();
    write_zip(&m.join("z.zip"), "zipped/", &rim_manifest("z.rims", "1.2", &rim_block("whl_am_zip_a", "brand = \"volk\"\n")), &rim_files("whl_am_zip_a"));
    let mut set = load(&data);
    assert_eq!(set.mods()[0].state, ModState::Loaded, "{:?}", set.mods()[0].reasons);
    set.unpack_zips(&cache_dir(&m)).unwrap();
    let f = set.list_dir("resources/rims/whl_am_zip_a");
    assert_eq!(f.len(), 2);
    assert!(f.iter().all(|f| f.disk_path().is_some_and(|p| p.is_file())));
}

/// The disc's real rim shop (skipped without the data): the medians MODS.md M-6 item 2b names, every disc price a
/// multiple of 4, the rim table numbers under 175.
#[test]
fn real_disc_rim_shop_prices() {
    let root = std::env::var_os("MCLA_DATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("D:/MCLA/extracted/cache"));
    if !root.join("tune/ui/garage/elements/Rim.sc.xml").is_file() {
        eprintln!("no data at {}: skipped", root.display());
        return;
    }
    let d = DiscInfo::load(&root);
    let rims_all: Vec<&rims::RimEntry> = d.shop.brands.iter().flat_map(|b| &b.rims).collect();
    assert!(rims_all.len() >= 100, "{} rims", rims_all.len());
    assert!(rims_all.iter().all(|r| r.price % rims::PRICE_STEP == 0 && r.index < rims::DISC_RIM_COUNT && !r.modded));
    // the raw medians of the design (2026-10-28): the rule rounds them to a multiple of 4, halves up
    let med = |id: &str| rims::computed_price(&d.shop, &rims::Join::Disc(id.into()), false);
    assert_eq!([med("amer"), med("bbs"), med("davin"), med("konig"), med("volk"), med("works")], [1152, 4568, 11648, 1148, 3492, 2140]);
    // a new brand, or a free download tab: all paid car rims (87, median 3600); a bike rim: the bike tabs (10, 3612)
    assert_eq!(rims::computed_price(&d.shop, &rims::Join::New("Acme".into()), false), 3600);
    assert_eq!(rims::computed_price(&d.shop, &rims::Join::Disc("Dayton_DLC".into()), false), 3600);
    assert_eq!(rims::computed_price(&d.shop, &rims::Join::New("Acme".into()), true), 3612);
    assert_eq!(rims::computed_price(&d.shop, &rims::Join::Disc("rc_bike".into()), true), 2768);
    // every folder of the shop is a folder of resources/rims (apart from the one typo the port corrects)
    assert!(d.rim_folders.contains("whl_am_volk_le28n") && d.rim_folders.contains("whl_stk_aud_r8_08"));
}

/// A disc aftermarket rim packed as a rim mod under another folder name (the M-6 1:1 sample, built from the real data):
/// valid with no warning, joins the Volk tab after the disc's seven, takes the disc rim's price when asked and the tab's
/// median when not. Also the disc rims' zones: all of them have zone 1 (spoke A).
#[test]
fn rim_sample_from_the_disc_is_valid() {
    let root = std::env::var_os("MCLA_DATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("D:/MCLA/extracted/cache"));
    if !root.join("resources/rims").join(sample::RIM_SOURCE).is_dir() {
        eprintln!("no data at {}: skipped", root.display());
        return;
    }
    let disc = DiscInfo::load(&root);
    let work = tmp("rimsample");
    let mods = work.join("mods");
    let mut o = sample::RimSampleOpts::default();
    let w = sample::write_rim_sample(&root, &work.join("a.zip"), &o).unwrap();
    assert_eq!(w.files, 3);
    install_package(&mods, &work.join("a.zip"), false).unwrap();
    let set = ModSet::load_from(&mods, &disc, &LoadOptions::default());
    let i = &set.mods()[0];
    assert_eq!(i.state, ModState::Loaded, "{:?}", i.reasons);
    assert!(i.warnings.is_empty() && i.rims[0].warnings.is_empty(), "{:?} {:?}", i.warnings, i.rims[0].warnings);
    let volk = disc.shop.brands.iter().find(|b| b.id == "volk").unwrap();
    let le28n = volk.rims.iter().find(|r| r.resource == sample::RIM_SOURCE).unwrap();
    // no price: the Volk tab's median; price set equal: the disc rim's
    assert_eq!((set.rims()[0].price, i.rims[0].price_given), (3492, false));
    o.price = Some(le28n.price);
    o.id = "sample.rim2".into();
    sample::write_rim_sample(&root, &work.join("b.zip"), &o).unwrap();
    let mods2 = work.join("mods2");
    install_package(&mods2, &work.join("b.zip"), false).unwrap();
    let set2 = ModSet::load_from(&mods2, &disc, &LoadOptions::default());
    assert_eq!((set2.rims()[0].price, set2.rims()[0].index), (le28n.price, 175));
    // the Mods tab's listing: the rim as content, its number, tab and price (the computed one too)
    let l = crate::manage::describe(&set, &Default::default(), true, 0);
    let c = &l.rows[0].contents[0];
    assert_eq!((c.kind, c.name.as_str(), c.problems.is_empty()), ("rim", sample::RIM_SAMPLE, true));
    assert_eq!(c.notes[0], "rim 175, joins the volk tab, $3,492 (not set: the median of the tab's paid rims)");
    let l2 = crate::manage::describe(&set2, &Default::default(), true, 0);
    assert_eq!(l2.rows[0].contents[0].notes[0], format!("rim 175, joins the volk tab, ${}", crate::with_commas(le28n.price)));
    let shop = rims::merge(disc.shop.clone(), set2.rims());
    let volk2 = shop.brands.iter().find(|b| b.id == "volk").unwrap();
    assert_eq!(volk2.rims.len(), volk.rims.len() + 1);
    assert_eq!(volk2.rims.last().unwrap().resource, sample::RIM_SAMPLE);
    assert_eq!(&volk2.rims[..volk.rims.len()].iter().map(|r| (r.resource.clone(), r.index, r.price)).collect::<Vec<_>>(), &volk.rims.iter().map(|r| (r.resource.clone(), r.index, r.price)).collect::<Vec<_>>(), "the disc's rims in the tab did not move");
    // the zones of every rim of the shop
    let mut none = Vec::new();
    for r in disc.shop.brands.iter().flat_map(|b| &b.rims) {
        let dir = root.join("resources/rims").join(rims::folder_for(&r.resource));
        let Some(p) = std::fs::read_dir(&dir).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "r63"))) else { continue };
        let (z, _) = rims::model_zones(&std::fs::read(p).unwrap()).unwrap();
        if z.is_empty() {
            none.push(r.resource.clone());
        }
    }
    assert!(none.is_empty(), "disc rims without zones: {none:?}");
}

/// The line the Mods tab shows for a rim: number, tab, kind and price - given, free or computed.
#[test]
fn rim_detail_line_and_thousands() {
    use crate::rims::Join;
    assert_eq!((crate::with_commas(0), crate::with_commas(999), crate::with_commas(3492), crate::with_commas(1234567)), ("0".into(), "999".into(), "3,492".into(), "1,234,567".into()));
    let mut r = crate::RimStatus { folder: "whl_am_x".into(), label: "X".into(), brand: "volk".into(), join: Some(Join::Disc("rim_volk".into())), bike: false, price: 3492, price_given: false, index: Some(175), reasons: vec![], warnings: vec![] };
    assert_eq!(r.detail(), "rim 175, joins the volk tab, $3,492 (not set: the median of the tab's paid rims)");
    r.price_given = true;
    r.price = 0;
    assert_eq!(r.detail(), "rim 175, joins the volk tab, free");
    r.price = 2612;
    r.bike = true;
    r.join = Some(Join::New("Acme".into()));
    r.index = None;
    assert_eq!(r.detail(), "bike rim, new tab \"Acme\", $2,612");
}

// ---- car makers (MODS.md M-7) ----

/// A PNG signature and IHDR of the given size (the loader reads the size only; the game decodes the picture itself).
fn fake_png(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b
}

fn maker_mod(id: &str, car: &str, maker: &str, decls: &str) -> String {
    manifest(id, "1.0.0", car, decls).replace("manufacturer = \"audi\"", &format!("manufacturer = \"{maker}\""))
}

#[test]
fn a_mod_declares_a_maker_and_a_car_names_it() {
    let data = data_root();
    let car = "vp_mcl_mp4_11";
    let mut files = car_files(car);
    files.push(("logos/mclaren.png".into(), fake_png(512, 128)));
    let decl = "\n[[manufacturer]]\nname = \"McLaren\"\nlogo = \"logos/mclaren.png\"\ncan_be_on_fire = false\n";
    write_folder(&mods_dir(&data).join("mcl"), &maker_mod("mcl", car, "mclaren", decl), &files);
    let set = load(&data);
    let m = &set.mods()[0];
    assert_eq!(m.state, ModState::Loaded, "{:?}", m.reasons);
    assert!(m.reasons.is_empty() && m.warnings.is_empty(), "{:?} {:?}", m.reasons, m.warnings);
    // one new maker with its logo; the car carries the declared spelling
    assert_eq!(set.makers().len(), 1);
    let k = set.maker("MCLAREN").unwrap();
    assert_eq!((k.name.as_str(), k.mod_id.as_str(), k.can_be_on_fire, k.logo.is_some()), ("McLaren", "mcl", false, true));
    assert_eq!(set.car(car).unwrap().manufacturer, "McLaren");
    // the in-memory list: the Manufacturer block comes before the Vehicle blocks that name it
    let list = set.extra_list();
    let (mb, vb) = (list.find("Manufacturer\r\n{\r\n\tName\tMcLaren\r\n\tCanBeOnFire\t0\r\n}\r\n").expect("maker block"), list.find("Vehicle\r\n").unwrap());
    assert!(mb < vb, "{list}");
    assert!(list.contains("\tManufacturer\tMcLaren\r\n"));
    // the Mods tab: the maker with its logo
    let l = crate::manage::describe(&set, &Default::default(), true, 0);
    let c = l.rows[0].contents.iter().find(|c| c.kind == "maker").unwrap();
    assert_eq!((c.name.as_str(), c.problems.is_empty()), ("McLaren", true));
    assert!(c.logo.as_deref().unwrap().starts_with("data:image/png;base64,iVBORw0KGgo"), "{:?}", c.logo);
    assert_eq!(c.notes, ["own logo"]);
}

#[test]
fn a_maker_without_a_logo_is_drawn_as_text() {
    let data = data_root();
    let car = "vp_mcl_text_11";
    write_folder(&mods_dir(&data).join("t"), &maker_mod("t", car, "TESTMAKER", "\n[[manufacturer]]\nname = \"TESTMAKER\"\n"), &car_files(car));
    let set = load(&data);
    assert_eq!(set.mods()[0].state, ModState::Loaded, "{:?}", set.mods()[0].reasons);
    assert!(set.maker("testmaker").unwrap().logo.is_none());
    assert!(set.maker("testmaker").unwrap().can_be_on_fire, "the original's default is 1");
    let w = &set.mods()[0].makers[0].warnings;
    assert!(w.len() == 1 && w[0].contains("no logo") && w[0].contains("drawn as text"), "{w:?}");
    assert_eq!(set.extra_list().matches("CanBeOnFire\t1").count(), 1);
}

#[test]
fn a_maker_of_the_game_means_the_games_maker() {
    let data = data_root();
    let car = "vp_aud_x_11";
    let decl = "\n[[manufacturer]]\nname = \"audi\"\nlogo = \"a.png\"\n";
    let mut files = car_files(car);
    files.push(("a.png".into(), fake_png(64, 64)));
    write_folder(&mods_dir(&data).join("a"), &maker_mod("a", car, "AUDI", decl), &files);
    let set = load(&data);
    let m = &set.mods()[0];
    assert_eq!(m.state, ModState::Loaded, "{:?}", m.reasons);
    // no second entry, the game's spelling, a warning that says so
    assert!(set.makers().is_empty() && !set.extra_list().contains("Manufacturer\r\n{"));
    assert_eq!(set.car(car).unwrap().manufacturer, "Audi");
    assert_eq!(m.makers[0].disc.as_deref(), Some("Audi"));
    assert!(m.makers[0].warnings[0].contains("maker of the game"), "{:?}", m.makers[0].warnings);
    let l = crate::manage::describe(&set, &Default::default(), true, 0);
    let c = l.rows[0].contents.iter().find(|c| c.kind == "maker").unwrap();
    assert!(c.logo.is_none() && c.notes[0].contains("the game's maker Audi"), "{c:?}");
}

#[test]
fn two_mods_declaring_one_maker_share_it() {
    let data = data_root();
    let (c1, c2) = ("vp_sh_one_11", "vp_sh_two_11");
    let mut f1 = car_files(c1);
    f1.push(("l.png".into(), fake_png(100, 40)));
    let mut f2 = car_files(c2);
    f2.push(("l.png".into(), fake_png(200, 80)));
    let d1 = "\n[[manufacturer]]\nname = \"Shared\"\nlogo = \"l.png\"\n";
    let d2 = "\n[[manufacturer]]\nname = \"SHARED\"\nlogo = \"l.png\"\n";
    // (mod ids order the declarations: "aa" before "bb")
    write_folder(&mods_dir(&data).join("p2"), &maker_mod("bb", c2, "shared", d2), &f2);
    write_folder(&mods_dir(&data).join("p1"), &maker_mod("aa", c1, "shared", d1), &f1);
    let set = load(&data);
    assert!(set.mods().iter().all(|m| m.state == ModState::Loaded), "{:?}", set.mods().iter().map(|m| &m.reasons).collect::<Vec<_>>());
    assert_eq!(set.makers().len(), 1);
    let k = set.maker("shared").unwrap();
    assert_eq!((k.name.as_str(), k.mod_id.as_str()), ("Shared", "aa"), "first by mod id keeps its entry");
    assert_eq!(k.logo.as_ref().map(|l| crate::rims::png_size(l)), Some(Some((100, 40))));
    assert_eq!(set.extra_list().matches("Manufacturer\r\n{").count(), 1);
    assert_eq!(set.car(c2).unwrap().manufacturer, "Shared");
    let (a, b) = (&set.mods()[0], &set.mods()[1]);
    assert_eq!((a.id.as_str(), b.id.as_str()), ("aa", "bb"));
    assert_eq!(a.makers[0].shared_by, ["bb"]);
    assert_eq!(b.makers[0].shared_with.as_deref(), Some("aa"));
    assert!(b.makers[0].detail().contains("shared with mod aa"), "{}", b.makers[0].detail());
    assert!(a.makers[0].detail().contains("also declared by bb"), "{}", a.makers[0].detail());
}

#[test]
fn a_car_needs_a_maker_the_game_or_a_loaded_mod_has() {
    let data = data_root();
    let car = "vp_nomaker_11";
    write_folder(&mods_dir(&data).join("n"), &maker_mod("n", car, "Nobody", ""), &car_files(car));
    let set = load(&data);
    let m = &set.mods()[0];
    assert_eq!(m.state, ModState::Invalid);
    assert!(m.cars[0].reasons[0].contains("\"Nobody\" is neither in vehicle.lst") && m.cars[0].reasons[0].contains("[[manufacturer]]"), "{:?}", m.cars[0].reasons);
    // a car in one mod, the maker in another: fine; switch the declaring mod off and the car's mod is not loaded either
    let data = data_root();
    let (cm, ck) = ("vp_split_car_11", "vp_split_mk_11");
    write_folder(&mods_dir(&data).join("maker"), &maker_mod("m1", ck, "Split", "\n[[manufacturer]]\nname = \"Split\"\n"), &car_files(ck));
    write_folder(&mods_dir(&data).join("car"), &maker_mod("m2", cm, "split", ""), &car_files(cm));
    let set = load(&data);
    assert!(set.mods().iter().all(|m| m.state == ModState::Loaded), "{:?}", set.mods().iter().map(|m| &m.reasons).collect::<Vec<_>>());
    assert_eq!(set.car(cm).unwrap().manufacturer, "Split");
    let off = ModSet::load(&data, &LoadOptions { disabled: ["m1".to_string()].into(), ..Default::default() });
    let by = |id: &str| off.mods().iter().find(|m| m.id == id).unwrap();
    assert_eq!(by("m1").state, ModState::Disabled);
    assert_eq!(by("m2").state, ModState::Invalid, "{:?}", by("m2").reasons);
    assert!(off.cars().is_empty() && off.makers().is_empty());
}

#[test]
fn maker_names_and_logos_are_checked() {
    let long = "x".repeat(31);
    let too_long = "x".repeat(32);
    for (name, ok) in [("McLaren", true), ("Land_Rover", true), ("A-B.c", true), ("", false), ("Two Words", false), ("Quo\"te", false), ("\u{160}koda", false), (long.as_str(), true), (too_long.as_str(), false)] {
        assert_eq!(manifest::maker_name_problem(name).is_none(), ok, "{name:?}");
    }
    let data = data_root();
    let car = "vp_badlogo_11";
    let mut files = car_files(car);
    files.push(("notpng.png".into(), b"not a png".to_vec()));
    let decl = "\n[[manufacturer]]\nname = \"Bad\"\nlogo = \"notpng.png\"\n[[manufacturer]]\nname = \"Gone\"\nlogo = \"missing.png\"\n[[manufacturer]]\nname = \"Two Words\"\n";
    write_folder(&mods_dir(&data).join("b"), &maker_mod("b", car, "Bad", decl), &files);
    let set = load(&data);
    let m = &set.mods()[0];
    assert_eq!(m.state, ModState::Invalid);
    let all = m.reasons.join(" | ");
    assert!(all.contains("not a PNG picture") && all.contains("missing.png is not in the package") && all.contains("no spaces"), "{all}");
}

#[test]
fn png_pictures_are_read_and_a_donor_warns_about_the_logo() {
    let data = data_root();
    let car = "vp_png_car_11";
    // own pictures as PNG (one of the disc's size, one of another), no .r9
    let mut files: Vec<(String, Vec<u8>)> = car_files(car).into_iter().filter(|(p, _)| !p.ends_with(".r9")).collect();
    for (p, (w, h)) in validate::PICTURES.iter().zip([(512u32, 128u32), (512, 512), (256, 256)]) {
        files.push((format!("resources/ui/textures/garage/dyntextures/{p}{car}.png"), fake_png(w, h)));
    }
    write_folder(&mods_dir(&data).join("pngs"), &maker_mod("pngs", car, "audi", ""), &files);
    let set = load(&data);
    let m = &set.mods()[0];
    assert_eq!(m.state, ModState::Loaded, "{:?}", m.reasons);
    let w = &m.cars[0].warnings;
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("msh_vp_png_car_11.png is 256x256") && w[0].contains("128x64") && w[0].contains("scaled"), "{}", w[0]);
    assert!(set.find(&format!("resources/ui/textures/garage/dyntextures/showroom/sh_{car}.png")).is_some());
    // a picture that is no PNG is refused
    let data = data_root();
    let car = "vp_png_bad_11";
    let mut files: Vec<(String, Vec<u8>)> = car_files(car).into_iter().filter(|(p, _)| !p.contains("managerides")).collect();
    files.push((format!("resources/ui/textures/garage/dyntextures/managerides/mr_{car}.png"), b"nope".to_vec()));
    write_folder(&mods_dir(&data).join("bad"), &maker_mod("bad", car, "audi", ""), &files);
    let m = load(&data).mods()[0].clone();
    assert_eq!(m.state, ModState::Invalid);
    assert!(m.reasons[0].contains("is not a PNG picture"), "{:?}", m.reasons);
    // a car without own pictures borrows the donor's: a warning when the donor is another maker's (the logo is in the picture)
    let data = data_root();
    let car = "vp_donor_pic_11";
    let files: Vec<(String, Vec<u8>)> = car_files(car).into_iter().filter(|(p, _)| !p.ends_with(".r9")).collect();
    let decl = "\n[[manufacturer]]\nname = \"Zed\"\n";
    let text = maker_mod("d", car, "Zed", decl).replace("[car.donors]\n", "[car.donors]\npictures = \"vp_aud_r8_08\"\n");
    write_folder(&mods_dir(&data).join("d"), &text, &files);
    let m = load(&data).mods()[0].clone();
    assert_eq!(m.state, ModState::Loaded, "{:?}", m.reasons);
    assert!(m.cars[0].warnings.iter().any(|w| w.contains("show Audi's logo, not Zed's")), "{:?}", m.cars[0].warnings);
    let text = maker_mod("d", car, "audi", "").replace("[car.donors]\n", "[car.donors]\npictures = \"vp_aud_r8_08\"\n");
    write_folder(&mods_dir(&data).join("d"), &text, &files);
    let m = load(&data).mods()[0].clone();
    assert!(m.cars[0].warnings.is_empty(), "{:?}", m.cars[0].warnings);
}

#[test]
fn hide_driver_is_a_bool_default_false() {
    let data = data_root();
    let (a, b) = ("vp_hd_off_11", "vp_hd_on_11");
    write_folder(&mods_dir(&data).join("a"), &manifest("a", "1", a, ""), &car_files(a));
    let text = manifest("b", "1", b, "").replace("short_name = \"08 TEST\"\n", "short_name = \"08 TEST\"\nhide_driver = true\n");
    write_folder(&mods_dir(&data).join("b"), &text, &car_files(b));
    let set = load(&data);
    assert!(set.mods().iter().all(|m| m.state == ModState::Loaded), "{:?}", set.mods().iter().map(|m| &m.reasons).collect::<Vec<_>>());
    assert!(!set.car(a).unwrap().hide_driver && set.car(b).unwrap().hide_driver);
    let l = crate::manage::describe(&set, &Default::default(), true, 0);
    assert!(l.rows[1].contents[0].notes.iter().any(|n| n.contains("no driver is drawn")));
    assert!(l.rows[0].contents[0].notes.is_empty());
    // a non-bool is refused by the manifest parser
    assert!(manifest::parse(&manifest("c", "1", "vp_hd_bad_11", "").replace("short_name = \"08 TEST\"\n", "short_name = \"08 TEST\"\nhide_driver = \"yes\"\n")).is_err());
}
