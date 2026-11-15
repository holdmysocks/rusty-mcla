//! The portable folder layout (LAUNCHER.md "Folder layout") and the small files the pipeline keeps there.

use crate::error::{ErrorKind, InstallError, IoCtx, Result};
use crate::extract::FileRecord;
use crate::validate::{hex, IsoIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Layout {
        Layout { root: root.into() }
    }
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    /// What `mcla.exe --data` points at.
    pub fn cache(&self) -> PathBuf {
        self.data().join("cache")
    }
    pub fn audio_pc(&self) -> PathBuf {
        self.data().join("audio_pc")
    }
    pub fn video_pc(&self) -> PathBuf {
        self.data().join("video_pc")
    }
    pub fn install_dir(&self) -> PathBuf {
        self.data().join(".install")
    }
    pub fn files_list(&self) -> PathBuf {
        self.install_dir().join("files.tsv")
    }
    pub fn state_file(&self) -> PathBuf {
        self.install_dir().join("state.json")
    }
    pub fn local_manifest(&self) -> PathBuf {
        self.root.join("manifest.local.json")
    }
    /// The converted-media folders (`audio_pc`, `video_pc`) that hold no file at all: missing, or empty. A game started on such a
    /// folder plays without sound or videos, so neither the build nor the launcher may count the data as complete.
    pub fn empty_media_dirs(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        for (name, dir) in [("audio_pc", self.audio_pc()), ("video_pc", self.video_pc())] {
            if !dir_has_files(&dir) {
                v.push(name);
            }
        }
        v
    }
    /// `manifest.local.json` is there and the converted media are too: what "installed" means to the launcher.
    pub fn is_complete(&self) -> bool {
        self.local_manifest().is_file() && self.empty_media_dirs().is_empty()
    }
}

/// Whether `dir` holds at least one file anywhere below it (unfinished `.part` work files do not count).
pub fn dir_has_files(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if dir_has_files(&e.path()) {
                return true;
            }
        } else if ft.is_file() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if !(n.ends_with(".part") || n.ends_with(".part.mp3")) {
                return true;
            }
        }
    }
    false
}

/// Work in progress, in `data/.install/state.json` (the launcher treats only `manifest.local.json` as "installed").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub data_version: u32,
    /// Stage id -> finished.
    pub done: BTreeMap<String, bool>,
    /// Identity of the disc the work was started from; a different disc restarts the extraction.
    pub iso_size: u64,
    pub iso_name: String,
}

impl State {
    pub fn load(l: &Layout) -> State {
        std::fs::read_to_string(l.state_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self, l: &Layout) -> Result<()> {
        std::fs::create_dir_all(l.install_dir()).ctx(l.install_dir().display())?;
        let t = serde_json::to_string_pretty(self).map_err(|e| InstallError::new(ErrorKind::Io, e.to_string()))?;
        write_file_atomic(&l.state_file(), t.as_bytes())
    }
    pub fn is_done(&self, stage: &str) -> bool {
        self.done.get(stage).copied().unwrap_or(false)
    }
}

pub fn write_file_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data).ctx(tmp.display())?;
    let _ = std::fs::remove_file(path);
    std::fs::rename(&tmp, path).ctx(path.display())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageSummary {
    pub files: usize,
    pub bytes: u64,
}

/// `manifest.local.json`: written last, when the data folder is complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalManifest {
    pub schema: u32,
    pub data_version: u32,
    /// Unix seconds.
    pub installed_unix: u64,
    pub iso: IsoIdentity,
    pub ffmpeg: Option<FfmpegNote>,
    /// Relative to the folder this file is in.
    pub paths: BTreeMap<String, String>,
    pub stages: BTreeMap<String, StageSummary>,
    pub files_list: String,
    pub files_count: usize,
    pub files_bytes: u64,
    pub files_list_sha256: String,
    /// Audio files/waves ffmpeg could not convert (kept in the build log).
    pub audio_problems: usize,
    /// SHA-256 of the translated shader files `cache/shaders_wgsl/*.wgsl` (name -> hex); verify compares them. Absent before data version 2.
    #[serde(default)]
    pub shaders: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FfmpegNote {
    pub version: String,
    pub source: crate::ffmpeg::Source,
}

impl LocalManifest {
    pub fn load(l: &Layout) -> Option<LocalManifest> {
        std::fs::read_to_string(l.local_manifest()).ok().and_then(|t| serde_json::from_str(&t).ok())
    }
    pub fn save(&self, l: &Layout) -> Result<()> {
        let t = serde_json::to_string_pretty(self).map_err(|e| InstallError::new(ErrorKind::Io, e.to_string()))?;
        write_file_atomic(&l.local_manifest(), t.as_bytes())
    }
}

/// Every file below `dir`, recursively, as paths relative to `base` (`/` separated) with sizes.
pub fn scan(base: &Path, dir: &Path, out: &mut Vec<FileRecord>) -> std::io::Result<()> {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for e in rd {
        let e = e?;
        let p = e.path();
        let ft = e.file_type()?;
        if ft.is_dir() {
            scan(base, &p, out)?;
        } else if ft.is_file() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            if n.ends_with(".part") || n.ends_with(".part.mp3") {
                continue; // unfinished work files
            }
            let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.push(FileRecord { path: rel, size: e.metadata()?.len() });
        }
    }
    Ok(())
}

/// Text of the file list: `path<TAB>size` per line, sorted.
pub fn list_text(files: &[FileRecord]) -> String {
    let mut s = String::with_capacity(files.len() * 64);
    for f in files {
        s.push_str(&f.path);
        s.push('\t');
        s.push_str(&f.size.to_string());
        s.push('\n');
    }
    s
}

pub fn list_hash(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

pub fn parse_list(text: &str) -> Vec<FileRecord> {
    text.lines()
        .filter_map(|l| {
            let (p, s) = l.rsplit_once('\t')?;
            Some(FileRecord { path: p.to_string(), size: s.parse().ok()? })
        })
        .collect()
}
