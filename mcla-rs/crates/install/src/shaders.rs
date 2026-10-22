//! The "Translate shaders" stage: the sky, road and ocean programs of the original game, translated to WGSL on the user's machine.
//!
//! The translator is the crate `mcla-shaders`; this module is the pipeline side: the executable's Perlin effect is cut out of the disc's
//! `default.xex` once (the other four effect files were extracted from the archive by the extract stage), the nine `xs_*.wgsl` files are
//! written below `data/cache/shaders_wgsl/`, and their SHA-256 values go into `manifest.local.json` so verify can tell a changed file.

use crate::error::{ErrorKind, InstallError, IoCtx, Result};
use crate::layout::write_file_atomic;
use crate::progress::{Ctx, Stage, Unit};
use mcla_formats::iso::Xiso;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use mcla_shaders::{FileReport, OUTPUTS, OUT_DIR};

/// What the stage did.
#[derive(Debug, Clone)]
pub struct ShaderReport {
    pub files: Vec<FileReport>,
    /// The executable's effect was cut out of the disc in this run.
    pub effect_cut: bool,
}

impl ShaderReport {
    pub fn written(&self) -> usize {
        self.files.iter().filter(|f| f.written).count()
    }
    pub fn bytes(&self) -> usize {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

/// Data-folder relative paths (below `data/`) the stage owns, for the verify / repair classification.
pub const OWNED_PREFIXES: [&str; 2] = ["cache/shaders_wgsl/", "cache/shaders/exe/"];

fn shader_error(e: mcla_shaders::ShaderError) -> InstallError {
    InstallError::new(ErrorKind::Shaders, e.to_string())
}

pub fn effect_path(cache: &Path) -> PathBuf {
    cache.join(mcla_shaders::EXE_EFFECT)
}

/// Cut the Perlin pass' effect out of the disc's `default.xex` into the data folder.
pub fn cut_effect_from_disc(iso: &Xiso, cache: &Path) -> Result<()> {
    let xe = iso.find("default.xex").ok_or_else(|| InstallError::new(ErrorKind::MissingFile, "default.xex is not on the disc image"))?;
    let raw = iso.file("default.xex").and_then(|f| mcla_formats::ReadAt::read_vec(&f, 0, xe.size as usize)).map_err(|e| InstallError::new(ErrorKind::MissingFile, format!("default.xex: {e:#}")))?;
    let image = mcla_formats::xex::image(&raw).map_err(|e| InstallError::new(ErrorKind::UnsupportedEdition, format!("default.xex: {e:#}")))?;
    let blob = mcla_shaders::cut_exe_effect(&image).map_err(|e| InstallError::new(ErrorKind::UnsupportedEdition, e.to_string()))?;
    let path = effect_path(cache);
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).ctx(p.display())?;
    }
    write_file_atomic(&path, &blob)
}

/// Run the stage on the data folder `cache` (`data/cache`). `disc` is only needed when the executable's effect has not been cut out yet.
pub fn run(cache: &Path, disc: Option<&Xiso>, ctx: &Ctx) -> Result<ShaderReport> {
    ctx.message(Stage::Shaders, "Translating the sky, road and sea shader programs");
    let mut effect_cut = false;
    if std::fs::metadata(effect_path(cache)).map(|m| m.len() == 0).unwrap_or(true) {
        let Some(iso) = disc else {
            return Err(InstallError::new(ErrorKind::MissingFile, format!("{} is missing and the disc image is needed to cut it out of default.xex", effect_path(cache).display())));
        };
        ctx.message(Stage::Shaders, "Reading the cloud shader from the game's executable");
        cut_effect_from_disc(iso, cache)?;
        effect_cut = true;
    }
    ctx.check()?;
    let counter = ctx.counter(Stage::Shaders, Unit::Items, OUTPUTS.len() as u64);
    let mut files = Vec::new();
    for name in OUTPUTS {
        counter.set_message(format!("Translating {name}"));
        files.push(mcla_shaders::write_output(cache, name).map_err(shader_error)?);
        counter.add(1)?;
    }
    counter.finish();
    Ok(ShaderReport { files, effect_cut })
}

/// SHA-256 of every output file as it is on disk now (file name -> hex), for `manifest.local.json`.
pub fn hashes(cache: &Path) -> Result<BTreeMap<String, String>> {
    hashes_of(cache, &OUTPUTS)
}

/// The same for the named output files only.
pub fn hashes_of(cache: &Path, names: &[&str]) -> Result<BTreeMap<String, String>> {
    let mut m = BTreeMap::new();
    for name in names {
        let p = mcla_shaders::output_path(cache, name);
        let data = std::fs::read(&p).ctx(p.display())?;
        m.insert(name.to_string(), crate::validate::hex(&Sha256::digest(&data)));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_inputs_the_stage_says_what_is_missing() {
        let dir = std::env::temp_dir().join(format!("mcla-shaders-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let e = run(&dir, None, &Ctx::silent()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::MissingFile);
        // with the executable's effect present (any bytes) the next missing input is named
        std::fs::create_dir_all(effect_path(&dir).parent().unwrap()).unwrap();
        std::fs::write(effect_path(&dir), b"x").unwrap();
        let e = run(&dir, None, &Ctx::silent()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Shaders);
        assert!(e.detail.contains("fxl_final") || e.detail.contains("cannot"), "{}", e.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
