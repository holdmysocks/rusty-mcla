//! The translated programs, byte for byte. Needs the user's extracted data (`MCLA_DATA`, or `extracted/cache` of a development checkout);
//! skipped without it, so a clean clone builds and passes.
//!
//! The SHA-256 values are those of the files the translation of the Python reference tools (`tools/xs2wgsl.py`) produced for the
//! supported disc, apart from the first (comment) line, which now says the file was generated locally from the user's own disc.

use mcla_shaders::{build, OUTPUTS};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const EXPECTED: [(&str, &str); 9] = [
    ("xs_sky.wgsl", "ba82a9046f57e467e3dfee44ab3fd337b78baa0c10e065a4022e06d395336942"),
    ("xs_perlin.wgsl", "bce1ea5d95543b6b0ad238ba7e6593d13c5c45100b171cee1e3010127a3ebe8c"),
    ("xs_sky_cheap.wgsl", "00100cc4364916953859452451e25229c4aff7a0ed4b9ba3e80566da405ae314"),
    ("xs_sky_env.wgsl", "c67bf903f34314fc9f4c39b3783b72153d81fa8d56e584a5eb2f2c5d8de1c880"),
    ("xs_minisky.wgsl", "04f78fd5b2bdadc722354ffbc4eabdad754b6a88bf8ba8e3578d77f11205a3a5"),
    ("xs_road.wgsl", "50cfa4737636e7e07fdcfcf7ad187f61d96daeac31a764415b20e48c1271622b"),
    ("xs_roadn.wgsl", "5531839b9e756ed906d9869cc460ee2803f943cdc0b4962192ced3ea1bbe4537"),
    ("xs_water.wgsl", "9f7cf194d48c1502968b017f81bb8aae66e49663020725349e96f1a725150de0"),
    ("xs_pond.wgsl", "a4439d5bcf19a08dc6022342fa8103e353a487e1078eb124d728aece66f920dc"),
];

fn data_dir() -> Option<PathBuf> {
    let d = std::env::var_os("MCLA_DATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/cache"));
    // the executable's effect (written by the install stage) and the archive's effects must both be there
    (d.join(mcla_shaders::EXE_EFFECT).is_file() && d.join("shaders/city/fxl_final/0x29901476").is_file()).then_some(d)
}

#[test]
fn the_nine_programs_match_the_reference_translation() {
    let Some(d) = data_dir() else {
        eprintln!("skipped: no extracted game data with shaders/exe/exe_82801b00 (set MCLA_DATA)");
        return;
    };
    assert_eq!(OUTPUTS.len(), EXPECTED.len());
    for (name, want) in EXPECTED {
        let text = build(&d, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        let got: String = Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, want, "{name} differs from the reference translation");
        assert!(text.starts_with("// GENERATED on this machine from your own game disc"), "{name}: header");
    }
}

#[test]
fn an_unknown_output_is_refused() {
    let r = build(std::path::Path::new("."), "xs_nothing.wgsl");
    assert!(r.is_err());
}
