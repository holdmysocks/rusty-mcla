//! `mcla-shaders`: the game's own sky, road and ocean shader programs, translated on the user's machine.
//!
//! The original game draws its sky dome, the night road and the sea with Xbox 360 shader microcode that sits in the effect files of the
//! user's disc (and, for the cloud noise pass, inside `default.xex`). The port runs those programs on the GPU as WGSL. The WGSL is a
//! mechanical translation of the microcode ([`xenos`]), so it is a derivative of the game's data and is NOT part of the source
//! repository: the installer ([`translate_shaders`], install stage "Translate shaders") writes the nine files below into the user's
//! data folder and the game loads them from there at start-up.
//!
//! Inputs (relative to the data folder `data/cache`, written by the extract stage): four effect files below `shaders/city/fxl_final/`
//! and the Perlin pass cut out of the executable, `shaders/exe/exe_82801b00` ([`cut_exe_effect`]). Output: `shaders_wgsl/xs_*.wgsl`.
//! `tools/xs2wgsl.py` (+ `xsh.py`, `xsrun.py`) stay in the repository as the readable Python reference of [`xenos`].

pub mod xenos;

use regex::Regex;
use std::path::{Path, PathBuf};
use xenos::{translate, Translated};

/// Folder (below the data folder) the translated programs are written to and the game reads them from.
pub const OUT_DIR: &str = "shaders_wgsl";

/// The nine output files, in the order the installer writes them.
pub const OUTPUTS: [&str; 9] = ["xs_sky.wgsl", "xs_perlin.wgsl", "xs_sky_cheap.wgsl", "xs_sky_env.wgsl", "xs_minisky.wgsl", "xs_road.wgsl", "xs_roadn.wgsl", "xs_water.wgsl", "xs_pond.wgsl"];

/// Where the executable's Perlin effect is kept (below the data folder), cut out of `default.xex` by [`cut_exe_effect`].
pub const EXE_EFFECT: &str = "shaders/exe/exe_82801b00";
/// Its place in the executable's image (the image starts at the guest address 0x82000000).
pub const EXE_EFFECT_RVA: usize = 0x80_1b00;

/// Effect files of the disc (data folder relative), named by the hash the archive stores them under.
const SKY: &str = "shaders/city/fxl_final/0x29901476";
const CITY: &str = "shaders/city/fxl_final/0x8667f75a";
const OCEAN: &str = "shaders/city/fxl_final/0x487d7df0";
const OCEAN_LOD: &str = "shaders/city/fxl_final/0x25bee0f2";
const POND: &str = "shaders/city/fxl_final/0x5a516d4c";

/// First line of every translated file.
const FIRST_LINE: &str = "// GENERATED on this machine from your own game disc by Rusty MCLA (mcla-shaders, install stage \"Translate shaders\") - do not edit, do not share.";

#[derive(Debug)]
pub enum ShaderError {
    /// An input file of the data folder is missing or unreadable.
    Input { path: PathBuf, reason: String },
    /// The microcode could not be translated (or is not the expected program).
    Translate { output: &'static str, reason: String },
    /// An output file could not be written.
    Output { path: PathBuf, reason: String },
    /// The executable does not contain the effect where the supported edition has it.
    Executable(String),
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShaderError::Input { path, reason } => write!(f, "cannot read {}: {reason}", path.display()),
            ShaderError::Translate { output, reason } => write!(f, "cannot translate the shader programs of {output}: {reason}"),
            ShaderError::Output { path, reason } => write!(f, "cannot write {}: {reason}", path.display()),
            ShaderError::Executable(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for ShaderError {}

pub type Result<T> = std::result::Result<T, ShaderError>;

/// Where one output file lives.
pub fn output_path(data: &Path, name: &str) -> PathBuf {
    data.join(OUT_DIR).join(name)
}

fn read_input(data: &Path, rel: &str) -> Result<Vec<u8>> {
    let p = data.join(rel);
    std::fs::read(&p).map_err(|e| ShaderError::Input { path: p, reason: e.to_string() })
}

fn tr(out: &'static str, effect: &[u8], prog: &str, func: &str) -> Result<Translated> {
    translate(effect, prog, func).map_err(|e| ShaderError::Translate { output: out, reason: format!("{prog}: {e}") })
}

fn cs(v: &[i64]) -> String {
    v.iter().map(|c| format!("c{c}")).collect::<Vec<_>>().join(" ")
}

/// Rename the sampler calls `u(` -> `u'(` for every sampler `u` of the program (`xs_s0` -> `<prefix>0`).
fn rename_samplers(src: &str, samplers: &[String], prefix: &str) -> String {
    let mut s = src.to_string();
    for u in samplers {
        s = s.replace(&format!("{u}("), &format!("{}(", u.replace("xs_s", prefix)));
    }
    s
}

fn word_renamed(src: &str, f: impl Fn(u32) -> String) -> String {
    let re = Regex::new(r"\bxs_s(\d+)").expect("regex");
    re.replace_all(src, |c: &regex::Captures| f(c[1].parse().unwrap_or(0))).into_owned()
}

fn road_sampler(n: u32) -> String {
    format!("xs_road_s{n}")
}

/// Build the text of one output file from the data folder's inputs.
pub fn build(data: &Path, name: &str) -> Result<String> {
    let out: &'static str = OUTPUTS.iter().copied().find(|o| *o == name).ok_or_else(|| ShaderError::Translate { output: "?", reason: format!("{name} is not one of the generated shaders") })?;
    match out {
        "xs_sky.wgsl" => {
            let sky = read_input(data, SKY)?;
            let vs = tr(out, &sky, "vs_main", "xs_sky_vs")?;
            let ps = tr(out, &sky, "ps_main_withstarfield", "xs_sky_ps")?;
            let head = format!(
                "{FIRST_LINE}\n// The game's own Skyhat programs (effect city/0x29901476), microcode instruction by instruction.\n// vs_main reads constants {}\n// ps_main_withstarfield reads constants {} and samplers {}\n",
                cs(&vs.consts),
                cs(&ps.consts),
                ps.samplers.join(" ")
            );
            Ok(format!("{head}\n{}\n{}", vs.wgsl, ps.wgsl))
        }
        "xs_perlin.wgsl" => {
            let exe = read_input(data, EXE_EFFECT)?;
            let pp = tr(out, &exe, "ps_main", "xs_perlin_ps")?;
            let head = format!("{FIRST_LINE}\n// The game's Perlin noise pass (an effect inside the executable): ps_main, constants {}, samplers {}\n", cs(&pp.consts), pp.samplers.join(" "));
            Ok(format!("{head}\n{}", pp.wgsl))
        }
        "xs_sky_cheap.wgsl" => {
            // the dome as the cars' environment map passes show it (two samplers: the Perlin target and the detail noise; named apart from the main program's)
            let sky = read_input(data, SKY)?;
            let cp = tr(out, &sky, "ps_main_cheap", "xs_sky_cheap_ps")?;
            let body = rename_samplers(&cp.wgsl, &cp.samplers, "xs_cheap_s");
            let head = format!("{FIRST_LINE}\n// The Skyhat program of the environment map passes: ps_main_cheap, constants {}, samplers {}\n", cs(&cp.consts), cp.samplers.join(" "));
            Ok(format!("{head}\n{body}"))
        }
        "xs_sky_env.wgsl" => {
            // ps_dpenvmapsky: a full-screen pass over the dual paraboloid map that makes its own direction from the pixel
            let sky = read_input(data, SKY)?;
            let ep = tr(out, &sky, "ps_dpenvmapsky", "xs_sky_env_ps")?;
            let body = rename_samplers(&ep.wgsl, &ep.samplers, "xs_envsky_s");
            let head = format!("{FIRST_LINE}\n// The Skyhat program of the cars' environment map: ps_dpenvmapsky, constants {}, samplers {}\n", cs(&ep.consts), ep.samplers.join(" "));
            Ok(format!("{head}\n{body}"))
        }
        "xs_minisky.wgsl" => {
            let sky = read_input(data, SKY)?;
            let ps = tr(out, &sky, "ps_miniSky", "xs_minisky_ps")?;
            let body = rename_samplers(&ps.wgsl, &ps.samplers, "xs_mini_s");
            let head = format!(
                "{FIRST_LINE}\n// The Skyhat program of the mini sky (the 128 x 128 map the window panes mirror): ps_miniSky,\n// constants {}, samplers {}\n",
                cs(&ps.consts),
                ps.samplers.join(" ")
            );
            Ok(format!("{head}\n{body}"))
        }
        "xs_road.wgsl" => {
            let fx = read_input(data, CITY)?;
            let vs = tr(out, &fx, "VSCityRoad", "xs_road_vs")?;
            let ps = tr(out, &fx, "PSCityRoad", "xs_road_ps")?;
            let body = word_renamed(&ps.wgsl, road_sampler);
            let used: Vec<String> = ps.samplers.iter().map(|u| u.replace("xs_s", "xs_road_s")).collect();
            let head = format!(
                "{FIRST_LINE}\n// The game's road programs (effect city/0x8667f75a), microcode instruction by instruction.\n// VSCityRoad reads constants {}\n// PSCityRoad reads constants {} and samplers {}\n",
                cs(&vs.consts),
                cs(&ps.consts),
                used.join(" ")
            );
            Ok(format!("{head}\n{}\n{}", vs.wgsl, body))
        }
        "xs_roadn.wgsl" => {
            // sampler names: the day road's (xs_road_s0 .. s4), the lamp tables' (xs_lamp_s7 / s8 / s9 / s11 / s12) and the night collector (xs_roadn_s13)
            let fx = read_input(data, CITY)?;
            let vs = tr(out, &fx, "VSCityRoadNight", "xs_roadn_vs")?;
            let ps = tr(out, &fx, "PSMultiLight", "xs_roadn_ps")?;
            let body = word_renamed(&ps.wgsl, |n| if n <= 4 { format!("xs_road_s{n}") } else if n == 13 { format!("xs_roadn_s{n}") } else { format!("xs_lamp_s{n}") });
            let head = format!(
                "{FIRST_LINE}\n// The game's night road programs (effect city/0x8667f75a), microcode instruction by instruction.\n// VSCityRoadNight reads constants {}\n// PSMultiLight reads constants {} and samplers {}\n",
                cs(&vs.consts),
                cs(&ps.consts),
                ps.samplers.join(" ")
            );
            Ok(format!("{head}\n{}\n{}", vs.wgsl, body))
        }
        "xs_pond.wgsl" => {
            // the fountain basins' and ponds' water (effect CityPondWater): VSCityPondWater + PSCityPondWater
            let fx = read_input(data, POND)?;
            let vs = tr(out, &fx, "VSCityPondWater", "xs_pond_vs")?;
            let ps = tr(out, &fx, "PSCityPondWater", "xs_pond_ps")?;
            let body = word_renamed(&ps.wgsl, |n| format!("xs_pond_s{n}"));
            let used: Vec<String> = ps.samplers.iter().map(|u| u.replace("xs_s", "xs_pond_s")).collect();
            let head = format!(
                "{FIRST_LINE}
// The game's pond water programs (effect city/0x5a516d4c CityPondWater), microcode instruction by instruction.
// VSCityPondWater reads constants {}
// PSCityPondWater reads constants {} and samplers {}
",
                cs(&vs.consts),
                cs(&ps.consts),
                used.join(" ")
            );
            Ok(format!("{head}
{}
{}", vs.wgsl, body))
        }
        _ => {
            // xs_water.wgsl: the detailed sea's vertex program, the LOD sea's, and the pixel program both effects share
            let fx = read_input(data, OCEAN)?;
            let fx_lod = read_input(data, OCEAN_LOD)?;
            let vs = tr(out, &fx, "VSCityOceanWater", "xs_water_vs")?;
            let vsl = tr(out, &fx_lod, "VSCityOceanWater", "xs_water_lod_vs")?;
            let ps = tr(out, &fx, "PSCityOceanWater", "xs_water_ps")?;
            let ps2 = tr(out, &fx_lod, "PSCityOceanWater", "xs_water_ps")?;
            if ps.wgsl != ps2.wgsl {
                return Err(ShaderError::Translate { output: out, reason: "the two ocean effects' pixel programs differ".into() });
            }
            let body = word_renamed(&ps.wgsl, |n| format!("xs_water_s{n}"));
            let used: Vec<String> = ps.samplers.iter().map(|u| u.replace("xs_s", "xs_water_s")).collect();
            let head = format!(
                "{FIRST_LINE}\n// The game's ocean programs (effects city/0x487d7df0 CityOceanWater, city/0x25bee0f2 CityOceanWaterLOD),\n// microcode instruction by instruction.\n// VSCityOceanWater (detailed) reads constants {}\n// VSCityOceanWater (LOD) reads constants {}\n// PSCityOceanWater reads constants {} and samplers {}\n",
                cs(&vs.consts),
                cs(&vsl.consts),
                cs(&ps.consts),
                used.join(" ")
            );
            Ok(format!("{head}\n{}\n{}\n{}", vs.wgsl, vsl.wgsl, body))
        }
    }
}

/// What [`write_output`] did with one file.
#[derive(Debug, Clone)]
pub struct FileReport {
    pub name: &'static str,
    pub bytes: usize,
    /// False when the file already had exactly this content.
    pub written: bool,
}

/// Translate one output and write it (atomically; an identical file is left alone).
pub fn write_output(data: &Path, name: &'static str) -> Result<FileReport> {
    let text = build(data, name)?;
    let path = output_path(data, name);
    if std::fs::read(&path).map(|old| old == text.as_bytes()).unwrap_or(false) {
        return Ok(FileReport { name, bytes: text.len(), written: false });
    }
    let werr = |e: std::io::Error| ShaderError::Output { path: path.clone(), reason: e.to_string() };
    std::fs::create_dir_all(path.parent().unwrap_or(data)).map_err(werr)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text.as_bytes()).map_err(werr)?;
    let _ = std::fs::remove_file(&path);
    std::fs::rename(&tmp, &path).map_err(werr)?;
    Ok(FileReport { name, bytes: text.len(), written: true })
}

/// Translate all nine programs of the data folder `data` (`data/cache`) into `data/shaders_wgsl/`.
pub fn translate_shaders(data: &Path) -> Result<Vec<FileReport>> {
    OUTPUTS.iter().map(|n| write_output(data, n)).collect()
}

/// Are all nine output files present and non-empty? (A cheap check for the game's start-up; the installer's verify compares content.)
pub fn outputs_present(data: &Path) -> bool {
    OUTPUTS.iter().all(|n| std::fs::metadata(output_path(data, n)).map(|m| m.len() > 0).unwrap_or(false))
}

/// Cut the Perlin pass' effect container out of the decrypted, expanded image of `default.xex`
/// (from its start at [`EXE_EFFECT_RVA`] to the next `rgxa` container, at most 128 KB).
pub fn cut_exe_effect(image: &[u8]) -> Result<Vec<u8>> {
    let bad = |s: &str| ShaderError::Executable(format!("default.xex does not hold the expected shader effect: {s}"));
    let Some(rest) = image.get(EXE_EFFECT_RVA..) else { return Err(bad("the image is too short")) };
    if rest.get(..4) != Some(b"rgxa") {
        return Err(bad("no effect container at the expected place (another edition of the game?)"));
    }
    let limit = rest.len().min(0x20000);
    let end = rest[4..limit].windows(4).position(|w| w == b"rgxa").map(|p| p + 4).unwrap_or(limit);
    let blob = rest[..end].to_vec();
    if !xenos::programs(&blob).iter().any(|p| p.name == "ps_main") {
        return Err(bad("the container has no ps_main program"));
    }
    Ok(blob)
}
