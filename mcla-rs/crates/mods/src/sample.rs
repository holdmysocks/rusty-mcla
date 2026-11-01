//! The sample mod "R8 (SAMPLE)" (docs/MODS.md M-5 stage S6): the disc Audi R8 re-packed as a mod car under another name,
//! the loader's own 1:1 test and the example mod a user can build from their own install.
//!
//! The package holds **data of the game** (the R8's models, tune, pictures, cameras ...), so it is never distributed:
//! this function reads the user's data folder (`<data>/resources`, `<data>/tune`, `<data>/audio`) and writes the zip;
//! `mcla-tool mods-sample <data folder> <out.zip>` is the command. The mod brings everything the formats allow
//! (resources, set-up, rims, pictures, garage and camera files, price list, a sound profile); what it cannot bring it
//! takes by index from the game (the set-up's brake index into the brakes list, the sound objects the profile points
//! at, the driver animation pack).
//!
//! What is renamed and what is not: the car's name is the folder name, the file names of the set-up, rims, pictures,
//! garage and camera files, the set-up's hash / TypeName / name fields and the sound profile's name. Nothing inside the
//! resource files is renamed: the port finds the car's textures (`<car>_maxdamage`, `<car>_zone`, `<car>_light`, ...)
//! by their suffix or through the shaders that name them, and the files inside a car folder have free names.

use mcla_formats::hash::joaat;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The disc car the sample is made from.
pub const SOURCE_CAR: &str = "vp_aud_r8_08";
/// The sample's car name (folder, lists, save).
pub const SAMPLE_CAR: &str = "vp_aud_r8_08_sample";
/// The sample's mod id.
pub const SAMPLE_ID: &str = "sample.r8";

/// The manifest of the sample.
pub fn manifest(version: &str) -> String {
    format!(
        r#"# R8 (SAMPLE): the game's own Audi R8 re-packed as a mod car under another name (docs/MODS.md stage S6).
# Built from your install by `mcla-tool mods-sample`; it holds game data and is never distributed.
[mod]
id = "{SAMPLE_ID}"
version = "{version}"
name = "R8 (SAMPLE)"
author = "Rusty MCLA"
description = "The Audi R8 of the game as a mod car: the loader's own test and an example of a car mod."

[[car]]
name = "{SAMPLE_CAR}"
long_name = "2008 R8 (SAMPLE)"
short_name = "08 R8 (SAMPLE)"

[car.row]
class = "EXOTIC"
manufacturer = "Audi"
group = "B0"
unlocking_group = "A0"
price = 118000
districts = ["Hollywood", "Beaches", "Hills", "Downtown"]
roof = 0
police_desc = "{SOURCE_CAR}"
figures = {{ base_accel = 0.26, mods_accel = 0.42, base_speed = 210.0, mods_speed = 255.0, base_handling = 0.62, mods_handling = 0.875 }}

# (no donors: the package brings its own tune, set-up, rims, pictures, garage and camera files, price list and sound
# profile; the driver animation pack key `aud_r8_08` is the only thing named from the game, and the loader does not read it)
[car.donors]
anim = "aud_r8_08"
"#
    )
}

/// What [`write_r8_sample`] wrote.
#[derive(Debug, Default)]
pub struct Written {
    pub files: usize,
    pub bytes: u64,
}

fn rd(p: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(p).map_err(|e| format!("cannot read {}: {e}", p.display()))
}

fn list(dir: &Path, keep: impl Fn(&str) -> bool) -> Result<Vec<PathBuf>, String> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(&keep)).collect();
    v.sort();
    Ok(v)
}

/// The set-up file of the sample: the R8's stock set-up with the car's hash (+0x1FA8), TypeName (+0x1FAD) and set-up
/// name (+0x1FCD) of the sample and the rim byte (+0x844) 255, "the car's own rim folder" (the sample brings
/// `whl_stk_aud_r8_08_sample`); brake (+0x846) and the rest stay the R8's.
pub fn setup_bytes(base: &[u8]) -> Option<Vec<u8>> {
    if base.len() != 8176 {
        return None;
    }
    let mut b = base.to_vec();
    b[0x1FA8..0x1FAC].copy_from_slice(&joaat(SAMPLE_CAR).to_be_bytes());
    for at in [0x1FAD, 0x1FCD] {
        b[at..at + 32].fill(0);
        b[at..at + SAMPLE_CAR.len()].copy_from_slice(SAMPLE_CAR.as_bytes());
    }
    b[0x844] = 255;
    Some(b)
}

/// The files of the sample as (path in the package, bytes), read from the data folder `data`.
pub fn r8_sample_files(data: &Path, version: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out: Vec<(String, Vec<u8>)> = vec![("mod.toml".into(), manifest(version).into_bytes())];
    // the resource folder: models, the two .r83, the .r19, the .r50 (not the development dumps `*.obj`)
    let src = data.join("resources/vehicle").join(SOURCE_CAR);
    for p in list(&src, |n| [".r63", ".r83", ".r19", ".r50"].iter().any(|e| n.ends_with(e)))? {
        out.push((format!("resources/vehicle/{SAMPLE_CAR}/{}", p.file_name().unwrap().to_string_lossy()), rd(&p)?));
    }
    // the stock set-up
    let xcc = rd(&data.join("resources/vehicle/configs").join(format!("{SOURCE_CAR}.xcc")))?;
    out.push((format!("resources/vehicle/configs/{SAMPLE_CAR}.xcc"), setup_bytes(&xcc).ok_or("the R8's set-up is not 8176 bytes")?));
    // the stock rims
    let rims = data.join("resources/rims/whl_stk_aud_r8_08");
    for p in list(&rims, |_| true)? {
        out.push((format!("resources/rims/whl_stk_aud_r8_08_sample/{}", p.file_name().unwrap().to_string_lossy()), rd(&p)?));
    }
    // the pictures
    for (sub, prefix) in [("managerides", "mr_"), ("showroom", "sh_"), ("mini_showroom", "msh_")] {
        let from = data.join("resources/ui/textures/garage/dyntextures").join(sub).join(format!("{prefix}{SOURCE_CAR}.r9"));
        out.push((format!("resources/ui/textures/garage/dyntextures/{sub}/{prefix}{SAMPLE_CAR}.r9"), rd(&from)?));
    }
    // garage camera lines and the part price list (the disc names them by the hash of the file name)
    let by_hash = |dir: &str, name: &str| rd(&data.join("tune").join(dir).join(format!("0x{:08x}", joaat(name))));
    out.push((format!("tune/garage/{SAMPLE_CAR}.garage"), by_hash("garage", &format!("{SOURCE_CAR}.garage"))?));
    out.push((format!("tune/garage/{SAMPLE_CAR}_parts.gtun"), by_hash("garage", &format!("{SOURCE_CAR}_parts.gtun"))?));
    // the driving cameras (those the disc has: the R8 has no fixed / heli / polar replay file)
    let cams = ["_boom1.camboomcs", "_boom2.camboomcs", "_boom3.camboomcs", "_POV.camboomcs", "_bumper.camboomcs", "_reverse.camboomcs", "_cinematic.camtrackcs", "_replay.mcreplaycamera", "_replay.mcreplaycamerafixed", "_replay.mcreplaycameraheli", "_replay.mcreplaycamerapolar"];
    for s in cams {
        if let Ok(b) = by_hash("camera", &format!("{SOURCE_CAR}{s}")) {
            out.push((format!("tune/camera/{SAMPLE_CAR}{s}"), b));
        }
    }
    // the sound profile: the R8's `<CAR>_PROFILE` object (22 bytes; it points at the R8's `_DEFAULT` objects of the game's
    // own game.dat) under the sample's name
    let game = mcla_formats::audcfg::Dat::load(&data.join("audio/X360/config/game.dat")).ok_or("cannot read audio/X360/config/game.dat")?;
    let profile = game.get_name(&format!("{}_PROFILE", SOURCE_CAR.to_ascii_uppercase())).ok_or("the R8 has no profile in game.dat")?;
    let name = format!("{}_PROFILE", SAMPLE_CAR.to_ascii_uppercase());
    out.push((crate::GAME_DAT.into(), mcla_formats::audcfg::Dat::build(game.version, &[(&name, profile)], &[])));
    Ok(out)
}

/// Write the sample as a zip at `out`.
pub fn write_r8_sample(data: &Path, out: &Path, version: &str) -> Result<Written, String> {
    let files = r8_sample_files(data, version)?;
    let f = std::fs::File::create(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut w = Written::default();
    for (name, bytes) in files {
        z.start_file(name, opts).map_err(|e| e.to_string())?;
        z.write_all(&bytes).map_err(|e| e.to_string())?;
        w.files += 1;
        w.bytes += bytes.len() as u64;
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(w)
}

// ---- the rim sample (M-6) ----

/// The disc rim the rim sample is made from, and the folder it gets as a mod (joins the same brand tab).
pub const RIM_SOURCE: &str = "whl_am_volk_le28n";
pub const RIM_SAMPLE: &str = "whl_am_sample_le28n";
pub const RIM_SAMPLE_ID: &str = "sample.rim";

/// What a rim sample is made like (`mcla-tool mods-sample-rim`).
#[derive(Clone, Debug)]
pub struct RimSampleOpts {
    pub id: String,
    pub version: String,
    /// the disc rim folder copied
    pub source: String,
    /// the folder it gets
    pub folder: String,
    /// `brand` of the manifest
    pub brand: String,
    pub price: Option<i64>,
    pub bike: bool,
    pub label: Option<String>,
    /// a PNG to ship as the brand logo (its bytes)
    pub logo: Option<Vec<u8>>,
}

impl Default for RimSampleOpts {
    fn default() -> Self {
        RimSampleOpts { id: RIM_SAMPLE_ID.into(), version: "1.0.0".into(), source: RIM_SOURCE.into(), folder: RIM_SAMPLE.into(), brand: "volk".into(), price: None, bike: false, label: None, logo: None }
    }
}

/// The manifest of a rim sample.
pub fn rim_manifest(o: &RimSampleOpts) -> String {
    let mut s = format!(
        "# Rim sample: the game's rim {} re-packed as a rim mod under another folder name (docs/MODS.md M-6).\n# Built from your install by `mcla-tool mods-sample-rim`; it holds game data and is never distributed.\n[mod]\nid = \"{}\"\nversion = \"{}\"\nname = \"Rim sample ({})\"\nauthor = \"Rusty MCLA\"\ndescription = \"A rim of the game as a rim mod: the loader's own test and an example of a rim mod.\"\n\n[[rim]]\nfolder = \"{}\"\nbrand = \"{}\"\n",
        o.source, o.id, o.version, o.folder, o.folder, o.brand
    );
    if let Some(l) = &o.label {
        s.push_str(&format!("label = \"{l}\"\n"));
    }
    if let Some(p) = o.price {
        s.push_str(&format!("price = {p}\n"));
    }
    if o.bike {
        s.push_str("bike = true\n");
    }
    if o.logo.is_some() {
        s.push_str("logo = \"logo.png\"\n");
    }
    s
}

/// The files of a rim sample: the manifest, the disc rim's folder under the new name, the logo.
pub fn rim_sample_files(data: &Path, o: &RimSampleOpts) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out: Vec<(String, Vec<u8>)> = vec![("mod.toml".into(), rim_manifest(o).into_bytes())];
    for p in list(&data.join("resources/rims").join(&o.source), |_| true)? {
        out.push((format!("resources/rims/{}/{}", o.folder, p.file_name().unwrap().to_string_lossy()), rd(&p)?));
    }
    if let Some(l) = &o.logo {
        out.push(("logo.png".into(), l.clone()));
    }
    Ok(out)
}

fn write_zip(files: Vec<(String, Vec<u8>)>, out: &Path) -> Result<Written, String> {
    let f = std::fs::File::create(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut w = Written::default();
    for (name, bytes) in files {
        z.start_file(name, opts).map_err(|e| e.to_string())?;
        z.write_all(&bytes).map_err(|e| e.to_string())?;
        w.files += 1;
        w.bytes += bytes.len() as u64;
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(w)
}

/// Write a rim sample as a zip at `out`.
pub fn write_rim_sample(data: &Path, out: &Path, o: &RimSampleOpts) -> Result<Written, String> {
    write_zip(rim_sample_files(data, o)?, out)
}
