//! Verify (and prepare repair): re-check the built data folder against the file list written when it was stamped.

use crate::error::{ErrorKind, InstallError, Result};
use crate::layout::{parse_list, Layout, State};
use crate::progress::{Ctx, Stage, Unit};
use rayon::prelude::*;
use sha2::Digest;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VerifyReport {
    pub checked: usize,
    pub missing: Vec<String>,
    pub wrong_size: Vec<String>,
    /// Translated shader files (`cache/shaders_wgsl/*`) whose content differs from the hash in `manifest.local.json`.
    #[serde(default)]
    pub changed: Vec<String>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.missing.is_empty() && self.wrong_size.is_empty() && self.changed.is_empty()
    }
    pub fn damaged(&self) -> usize {
        self.missing.len() + self.wrong_size.len() + self.changed.len()
    }
    /// Stage ids (`extract`, `shaders`, `audio`, `video`) that have damaged files.
    pub fn damaged_stages(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        let (mut extract, mut shaders, mut audio, mut video) = (false, false, false, false);
        for p in self.missing.iter().chain(self.wrong_size.iter()).chain(self.changed.iter()) {
            if crate::shaders::OWNED_PREFIXES.iter().any(|o| p.starts_with(o)) {
                shaders = true;
            } else if p.starts_with("cache/") {
                extract = true;
            } else if p.starts_with("audio_pc/") {
                audio = true;
            } else if p.starts_with("video_pc/") {
                video = true;
            }
        }
        for (on, stage) in [(extract, "extract"), (shaders, "shaders"), (audio, "audio"), (video, "video")] {
            if on {
                v.push(stage);
            }
        }
        v
    }
}

/// Compare every listed file's presence and size. Files in `data/` are listed relative to `data/`.
pub fn verify(l: &Layout, ctx: &Ctx) -> Result<VerifyReport> {
    let text = std::fs::read_to_string(l.files_list()).map_err(|_| InstallError::new(ErrorKind::DataDamaged, "the file list data/.install/files.tsv is missing: the data was not built here"))?;
    let list = parse_list(&text);
    let counter = ctx.counter(Stage::Verify, Unit::Items, list.len() as u64);
    counter.set_message("Checking the built files");
    let data = l.data();
    let bad: std::sync::Mutex<(Vec<String>, Vec<String>)> = std::sync::Mutex::new((Vec::new(), Vec::new()));
    list.par_chunks(512).try_for_each(|chunk| {
        for f in chunk {
            match std::fs::metadata(data.join(&f.path)) {
                Ok(m) if m.len() == f.size => {}
                Ok(_) => bad.lock().unwrap_or_else(|p| p.into_inner()).1.push(f.path.clone()),
                Err(_) => bad.lock().unwrap_or_else(|p| p.into_inner()).0.push(f.path.clone()),
            }
        }
        counter.add(chunk.len() as u64)
    })?;
    counter.finish();
    let (mut missing, mut wrong_size) = bad.into_inner().unwrap_or_default();
    // a media folder with no file at all is damage even when the list (written by a build that skipped it) names none
    for dir in l.empty_media_dirs() {
        missing.push(format!("{dir}/ (no files)"));
    }
    missing.sort();
    wrong_size.sort();
    // the translated shaders are checked by content (a few hundred KB): the same size can hide a changed file
    let mut changed = Vec::new();
    if let Some(lm) = crate::layout::LocalManifest::load(l) {
        for (name, want) in &lm.shaders {
            let rel = format!("cache/{}/{}", crate::shaders::OUT_DIR, name);
            if missing.contains(&rel) || wrong_size.contains(&rel) {
                continue;
            }
            let got = std::fs::read(data.join(&rel)).map(|b| crate::validate::hex(&sha2::Sha256::digest(&b))).unwrap_or_default();
            if got != *want {
                changed.push(rel);
            }
        }
        changed.sort();
    }
    Ok(VerifyReport { checked: list.len(), missing, wrong_size, changed })
}

/// Prepare a repair: delete files of the wrong size and clear the `done` flag of every damaged stage, so the next install run rebuilds
/// only what is missing.
pub fn prepare_repair(l: &Layout, report: &VerifyReport) -> Result<()> {
    for p in report.wrong_size.iter().chain(report.changed.iter()) {
        let _ = std::fs::remove_file(l.data().join(p));
    }
    let mut st = State::load(l);
    for s in report.damaged_stages() {
        st.done.insert(s.to_string(), false);
    }
    st.save(l)
}
