//! Tests of the converter: the container checks, a blank save, and (when the files are on this disk) the real saves against the output of the
//! Python importer they were ported from. The real saves and the Python output live in `work/launcher-saveimport/{in,ref}` (not in the
//! repository); a test that cannot find them says so and passes. How to regenerate them: `docs/SAVE.md`, "Importing without Python".

use mcla_savedecode::{convert, race_table, testing::blank_save, validate};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn map(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => panic!("not an object"),
    }
}

#[test]
fn a_blank_save_converts_and_keeps_the_port_only_members() {
    let base = map(json!({"keyboard": {"gas": "W"}, "best_time": {"x": 1}, "career": {"stats": {"zzzz": 5}}}));
    let out = convert(&blank_save(14), base, &["game/Race/a".to_string()]).unwrap();
    let s = Value::Object(out.save);
    assert_eq!(s["keyboard"]["gas"], "W");
    assert_eq!(s["best_time"]["x"], 1);
    assert_eq!(s["career"]["stats"]["zzzz"], 5, "members of the base that the original has no counterpart of survive");
    assert_eq!(s["money"], 0);
    assert!(s["mc4"]["container_version"] == 14 && s["mc4"]["garage"].as_array().unwrap().is_empty());
    assert!(s.get("car").is_none(), "no garage: no vehicle is set");
    assert_eq!(out.summary.vehicles.len(), 0);
    let ps3 = convert(&blank_save(15), Map::new(), &[]).unwrap();
    assert_eq!(ps3.summary.container_version, 15);
}

#[test]
fn damaged_input_is_refused_with_a_code_and_never_panics() {
    let good = blank_save(14);
    assert_eq!(validate(&good), Ok(()));
    let err = |b: &[u8]| convert(b, Map::new(), &[]).err().expect("must be refused").code;
    assert_eq!(err(&[]), "short");
    assert_eq!(err(&good[..7]), "short");
    assert_eq!(err(&good[..good.len() - 1]), "short");
    assert_eq!(err(&good[..1000]), "short");
    let mut v = good.clone();
    v[3] = 16;
    assert_eq!(err(&v), "version");
    let mut v = good.clone();
    v[7] = 8;
    assert_eq!(err(&v), "layout");
    // an entry of the wrong size
    let mut v = good.clone();
    let at = 8 + 4 + 5 + 1;
    v[at + 3] = 9;
    assert!(["layout", "short"].contains(&err(&v)));
    // the Career entry at another layout version
    let mut v = good.clone();
    let career = v.windows(6).position(|w| w == b"Career").unwrap() + 6 + 5;
    v[career + 4] = 12;
    assert_eq!(err(&v), "career");
    // an unknown condition type inside a mission
    let mut v = good.clone();
    // events 0, money, 3 empty system groups, then one mission with a prerequisite of an unknown type
    let w = career + 5;
    let mut o = w + 4; // events count 0
    o += 4; // money
    o += 12; // three group counts
    v[o + 3] = 1; // one mission
    o += 4 + 128 + 16; // name + index, log, done, state
    v[o + 3] = 1; // one prerequisite
    o += 4 + 2; // hidden, satisfied
    v[o + 3] = 1; // one condition
    o += 4;
    v[o + 3] = 99; // type 99
    assert_eq!(err(&v), "career");
    // random garbage after the container header must come back as an error or a conversion, never a panic
    let mut seed = 12345u32;
    for _ in 0..40 {
        let mut v = good.clone();
        for _ in 0..200 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let p = 0x100 + (seed as usize >> 8) % (v.len() - 0x100);
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            v[p] = (seed >> 24) as u8;
        }
        let _ = convert(&v, Map::new(), &[]);
    }
}

#[test]
fn the_race_table_is_read_the_way_the_python_decoder_read_it() {
    let t = race_table(b"Id game/Race/A\r\n  Id\tgame/Race/B aId x\n Id");
    assert_eq!(t, vec!["game/Race/A", "game/Race/B", "x"]);
    assert!(race_table(b"nothing here").is_empty());
}

fn fixture(name: &str) -> Option<Vec<u8>> {
    std::fs::read(root().join("work/launcher-saveimport").join(name)).ok()
}

/// Every real save against the Python importer's output, with an empty base and with a copy of the real port save as the base.
#[test]
fn the_real_saves_convert_to_what_the_python_importer_wrote() {
    let Some(table) = std::fs::read(root().join("extracted/cache/tune/race/0x0f1f4d8d")).ok() else {
        eprintln!("skipped: the race table of the extracted data is not on this disk");
        return;
    };
    let races = race_table(&table);
    let mut checked = 0;
    for name in ["xenia_current_v14", "full_v15", "newgame_v14", "recomp_trace_v14", "ps3_RAGE_dec"] {
        for base in ["empty", "real_copy"] {
            let (Some(sav), Some(basejson), Some(reference)) = (fixture(&format!("in/{name}.sav")), fixture(&format!("in/base_{base}.json")), fixture(&format!("ref/{name}__{base}.json"))) else {
                continue;
            };
            let got = convert(&sav, map(serde_json::from_slice(&basejson).unwrap()), &races).unwrap_or_else(|e| panic!("{name}/{base}: {e}"));
            let want: Value = serde_json::from_slice(&reference).unwrap();
            let mut got = Value::Object(got.save);
            // (ColorData, which the Python importer left out: the 40 slots, in every sample save the executable's table)
            let cd = got["mc4"].as_object_mut().unwrap().remove("color_data").expect("color_data");
            let want_cd: Vec<u32> = vec![
                0x000000, 0xFFFFFF, 0xDDD8BC, 0xD8CFA0, 0x010032, 0x10186E, 0x204465, 0x6A777D, 0x85C2E5, 0x524636, 0x6B5945, 0x934C3C, 0x2D1111, 0x6D6164,
                0x967886, 0x2F4432, 0x4C4900, 0x4D7569, 0x009B7F, 0x2A8400, 0x47484C, 0x595F65, 0x727272, 0x917F63, 0x93908B, 0xADA39A, 0xA3A3A3, 0xD8D8D8,
                0xE24412, 0xC17A38, 0xE57D32, 0xEDBA36, 0xBFA624, 0xB2AC10, 0x93346F, 0x8C705C, 0x3A0808, 0x8A0B22, 0xAF2613, 0xDF2B12,
            ];
            assert_eq!(cd, json!(want_cd), "{name}/{base}: ColorData");
            if got != want {
                // say where
                let (g, w) = (got.as_object().unwrap(), want.as_object().unwrap());
                for k in w.keys().chain(g.keys()) {
                    if g.get(k) != w.get(k) {
                        panic!("{name}/{base}: member {k} differs from the Python importer's");
                    }
                }
            }
            checked += 1;
        }
    }
    eprintln!("{checked} conversions equal to the Python importer's output");
}

#[test]
fn a_protected_ps3_file_decrypts_and_a_wrong_file_is_refused() {
    use mcla_savedecode::ps3;
    let plain = blank_save(15);
    let (pfd, enc) = ps3::testing::protect("RAGE.SAV", &plain, [7u8; 16]);
    assert_ne!(&enc[..16], &plain[..16]);
    assert_eq!(ps3::protected_files(&pfd).unwrap(), vec!["RAGE.SAV"]);
    assert_eq!(ps3::decrypt(&pfd, "rage.sav", &enc).unwrap(), plain, "decrypts to the original bytes, cut to the listed size");
    assert_eq!(ps3::decrypt(&pfd, "OTHER.SAV", &enc).unwrap_err().code, "pfd");
    assert_eq!(ps3::decrypt(&pfd, "RAGE.SAV", &enc[..8]).unwrap_err().code, "key");
    let mut bad = enc.clone();
    bad[3] ^= 1;
    assert_eq!(ps3::decrypt(&pfd, "RAGE.SAV", &bad).unwrap_err().code, "key");
    assert_eq!(ps3::decrypt(&pfd[..100], "RAGE.SAV", &enc).unwrap_err().code, "pfd");
    assert_eq!(ps3::decrypt(b"not a pfd at all, no", "RAGE.SAV", &enc).unwrap_err().code, "pfd");
}

/// The user's real PS3 save folder (read only) against the file `tools/ps3dec.py` wrote from it.
#[test]
fn the_real_ps3_save_decrypts_to_what_ps3dec_py_wrote() {
    let dir = root().join("save/PS3/SAVEDATA/BLUS30190MC4_SAV");
    let (Ok(pfd), Ok(enc), Some(want)) = (std::fs::read(dir.join("PARAM.PFD")), std::fs::read(dir.join("RAGE.SAV")), fixture("in/ps3_RAGE_dec.sav")) else {
        eprintln!("skipped: the real PS3 save is not on this disk");
        return;
    };
    let got = mcla_savedecode::ps3::decrypt(&pfd, "RAGE.SAV", &enc).unwrap();
    assert!(got == want, "differs from the Python decryption");
    eprintln!("the real PS3 RAGE.SAV decrypts to {} bytes equal to ps3dec.py's output", got.len());
}
