//! Extraction of an RPF3 archive into a folder (the logic of `mcla-tool extract`, moved here so the launcher runs it in process).
//!
//! * Files are written to `<name>.part` and renamed when complete, so a file that exists is a finished file.
//! * Resume: a file whose size matches (plain files) or that exists (resources, whose size is not known before decoding) is skipped.
//! * Script objects ("Scr", and the uncompressed "scr") are stored in the plain form ("Scp"), so the game never needs the key.

use crate::error::{ErrorKind, InstallError, Result};
use crate::progress::{Ctx, Stage, Unit};
use mcla_formats::names::Names;
use mcla_formats::rpf3::{Kind, Rpf3};
use mcla_formats::rsc::Resource;
use mcla_formats::ReadAt;
use rayon::prelude::*;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractOptions {
    /// Write resources and scripts exactly as stored (`--raw`).
    pub raw: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileRecord {
    /// Relative to the output folder, `/` separated.
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Default)]
pub struct ExtractReport {
    pub written: usize,
    pub skipped: usize,
    /// (entry path, reason)
    pub failed: Vec<(String, String)>,
    /// Every file of the archive as it is on disk now (written or already there), sorted by path.
    pub files: Vec<FileRecord>,
}

fn part_name(target: &Path) -> PathBuf {
    let mut s: OsString = target.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Where an entry goes when its recovered name cannot be created on this file system (a few of the names in the list are junk, for
/// example a folder part that ends in a space): the entry's folder chain with the hash name `0x<hash>` as the file name.
fn fallback_path<R: ReadAt>(rpf: &Rpf3<R>, idx: usize, names: &Names) -> String {
    let e = &rpf.entries[idx];
    let hash_name = format!("0x{:08x}", e.hash);
    if e.parent == u32::MAX || e.parent as usize >= rpf.entries.len() || e.parent == 0 {
        return hash_name;
    }
    let parent = rpf.path_of(e.parent as usize, names);
    format!("{parent}/{hash_name}")
}

fn safe_rel(rel: &str) -> bool {
    !rel.is_empty() && Path::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
}

fn write_atomic(target: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(p) = target.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = part_name(target);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, target)
}

/// Extract every entry of `rpf` below `out`. `stage` is the stage the progress events carry.
pub fn extract_archive<R: ReadAt>(rpf: &Rpf3<R>, out: &Path, names: &Names, opts: ExtractOptions, stage: Stage, ctx: &Ctx) -> Result<ExtractReport> {
    std::fs::create_dir_all(out).map_err(|e| InstallError::io(out.display(), &e))?;
    let n = rpf.entries.len();
    let paths: Vec<String> = (0..n).map(|i| rpf.path_of(i, names)).collect();
    let weight = |i: usize| -> u64 {
        match rpf.entries[i].kind {
            Kind::File { size, .. } | Kind::Resource { size, .. } => size as u64,
            Kind::Dir { .. } => 0,
        }
    };
    let total: u64 = (0..n).map(weight).sum();
    let counter = ctx.counter(stage, Unit::Bytes, total);
    let files = Mutex::new(Vec::<FileRecord>::new());
    let failed = Mutex::new(Vec::<(String, String)>::new());
    let written = std::sync::atomic::AtomicUsize::new(0);
    let skipped = std::sync::atomic::AtomicUsize::new(0);
    use std::sync::atomic::Ordering::Relaxed;

    let res: Result<()> = (0..n).into_par_iter().try_for_each(|i| {
        let e = &rpf.entries[i];
        let rel = &paths[i];
        let one = |rel: &str| -> std::io::Result<Option<(PathBuf, u64, bool)>> {
            // returns (target, size, was_skipped) for files
            let target = out.join(rel);
            match e.kind {
                Kind::Dir { .. } => {
                    std::fs::create_dir_all(&target)?;
                    Ok(None)
                }
                Kind::File { size, .. } => {
                    if std::fs::metadata(&target).map(|m| m.is_file() && m.len() == size as u64).unwrap_or(false) {
                        return Ok(Some((target, size as u64, true)));
                    }
                    let data = rpf.read(i).map_err(|er| std::io::Error::other(format!("{er:#}")))?;
                    let data = if !opts.raw && mcla_formats::sco::is_disc_form(&data) { mcla_formats::sco::to_plain(&data, &rpf.key).map_err(|er| std::io::Error::other(format!("{er:#}")))? } else { data };
                    write_atomic(&target, &data)?;
                    Ok(Some((target, data.len() as u64, false)))
                }
                Kind::Resource { rtype, flags, .. } => {
                    let target = if opts.raw { target.with_extension(format!("rsc{}", rtype)) } else { target.with_extension(format!("r{}", rtype)) };
                    if let Ok(m) = std::fs::metadata(&target) {
                        if m.is_file() {
                            return Ok(Some((target, m.len(), true)));
                        }
                    }
                    let blob = rpf.read(i).map_err(|er| std::io::Error::other(format!("{er:#}")))?;
                    if opts.raw {
                        write_atomic(&target, &blob)?;
                        return Ok(Some((target, blob.len() as u64, false)));
                    }
                    let r = Resource::parse(&blob).map_err(|er| std::io::Error::other(format!("{er:#}")))?;
                    // a tiny header keeps the virtual/physical split on disk
                    let mut v = Vec::with_capacity(r.data.len() + 16);
                    v.extend_from_slice(b"MCLR");
                    v.extend_from_slice(&rtype.to_le_bytes());
                    v.extend_from_slice(&flags.to_le_bytes());
                    v.extend_from_slice(&(r.virtual_size as u32).to_le_bytes());
                    v.extend_from_slice(&r.data);
                    write_atomic(&target, &v)?;
                    Ok(Some((target, v.len() as u64, false)))
                }
            }
        };
        if !safe_rel(rel) && i != 0 {
            failed.lock().unwrap_or_else(|p| p.into_inner()).push((rel.clone(), "unsafe path".into()));
            return counter.add(weight(i));
        }
        if i == 0 {
            // the root directory entry is the output folder itself
            return counter.add(0);
        }
        let mut res = one(rel);
        if let Err(er) = &res {
            if InstallError::io(rel, er).kind != ErrorKind::DiskSpace {
                // the recovered name cannot be created here: keep the file under its hash name instead
                let fb = fallback_path(rpf, i, names);
                if fb != *rel && safe_rel(&fb) {
                    if let Ok(ok) = one(&fb) {
                        res = Ok(ok);
                    }
                }
            }
        }
        match res {
            Ok(Some((target, size, was_skipped))) => {
                if was_skipped {
                    skipped.fetch_add(1, Relaxed);
                } else {
                    written.fetch_add(1, Relaxed);
                }
                let r = target.strip_prefix(out).unwrap_or(&target).to_string_lossy().replace('\\', "/");
                files.lock().unwrap_or_else(|p| p.into_inner()).push(FileRecord { path: r, size });
            }
            Ok(None) => {}
            Err(er) => {
                let ioe = InstallError::io(rel, &er);
                if ioe.kind == ErrorKind::DiskSpace {
                    return Err(ioe);
                }
                failed.lock().unwrap_or_else(|p| p.into_inner()).push((rel.clone(), er.to_string()));
            }
        }
        counter.add(weight(i))
    });
    res?;
    counter.finish();
    let mut files = files.into_inner().unwrap_or_default();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(ExtractReport { written: written.load(Relaxed), skipped: skipped.load(Relaxed), failed: failed.into_inner().unwrap_or_default(), files })
}

