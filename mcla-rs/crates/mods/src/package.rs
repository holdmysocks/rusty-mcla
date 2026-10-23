//! A mod package on disk: a folder or a `.zip` in the `mods` folder, with `mod.toml` at its root or inside its one
//! top folder (a zip made by zipping a folder). Lists the package's files and reads them.

use crate::manifest::MANIFEST_NAME;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub enum Source {
    Folder(PathBuf),
    Zip { path: PathBuf, archive: Mutex<zip::ZipArchive<BufReader<File>>> },
}

/// One file of a package.
#[derive(Clone, Debug)]
pub struct PackFile {
    /// path inside the package below the manifest's folder, forward slashes, as written by the author
    pub rel: String,
    /// for a zip: the entry's full name in the archive
    pub entry: String,
    pub size: u64,
}

pub struct Package {
    /// the folder or zip in the `mods` folder
    pub path: PathBuf,
    pub source: Source,
    /// the folder inside the package that holds `mod.toml` ("" = the root), forward slashes, with a trailing `/`
    prefix: String,
    /// every file except the manifest, sorted by `rel`
    pub files: Vec<PackFile>,
    pub manifest_text: String,
}

impl Package {
    /// Open a folder or zip package. `Err` = not a usable package (no manifest, unreadable zip...).
    pub fn open(path: &Path) -> Result<Package, String> {
        if path.is_dir() {
            Self::open_folder(path)
        } else {
            Self::open_zip(path)
        }
    }

    fn open_folder(dir: &Path) -> Result<Package, String> {
        let mut all = Vec::new();
        walk(dir, "", &mut all).map_err(|e| format!("cannot read the folder: {e}"))?;
        let prefix = manifest_prefix(all.iter().map(|(r, _)| r.as_str())).ok_or_else(|| format!("no {MANIFEST_NAME} in the folder (or in its one top folder)"))?;
        let manifest_text = std::fs::read_to_string(dir.join(format!("{prefix}{MANIFEST_NAME}"))).map_err(|e| format!("cannot read {MANIFEST_NAME}: {e}"))?;
        let files = files_below(&prefix, all.into_iter().map(|(r, s)| (r.clone(), r, s)));
        Ok(Package { path: dir.to_path_buf(), source: Source::Folder(dir.to_path_buf()), prefix, files, manifest_text })
    }

    fn open_zip(path: &Path) -> Result<Package, String> {
        let f = File::open(path).map_err(|e| format!("cannot open: {e}"))?;
        let mut z = zip::ZipArchive::new(BufReader::new(f)).map_err(|e| format!("not a readable zip: {e}"))?;
        let mut all = Vec::new();
        for i in 0..z.len() {
            let e = z.by_index_raw(i).map_err(|e| format!("bad zip entry {i}: {e}"))?;
            if e.is_dir() {
                continue;
            }
            // (no absolute paths, no `..`: an entry must stay inside the package)
            let Some(safe) = e.enclosed_name() else { return Err(format!("unsafe path in the zip: {}", e.name())) };
            let rel = safe.to_string_lossy().replace('\\', "/");
            all.push((rel, e.name().to_string(), e.size()));
        }
        let prefix = manifest_prefix(all.iter().map(|(r, _, _)| r.as_str())).ok_or_else(|| format!("no {MANIFEST_NAME} in the zip (or in its one top folder)"))?;
        let entry = all.iter().find(|(r, _, _)| r.eq_ignore_ascii_case(&format!("{prefix}{MANIFEST_NAME}"))).map(|x| x.1.clone()).unwrap();
        let mut manifest_text = String::new();
        z.by_name(&entry).map_err(|e| e.to_string())?.read_to_string(&mut manifest_text).map_err(|e| format!("cannot read {MANIFEST_NAME}: {e}"))?;
        let files = files_below(&prefix, all.into_iter());
        Ok(Package { path: path.to_path_buf(), source: Source::Zip { path: path.to_path_buf(), archive: Mutex::new(z) }, prefix, files, manifest_text })
    }

    pub fn is_zip(&self) -> bool {
        matches!(self.source, Source::Zip { .. })
    }

    /// The file's bytes.
    pub fn read(&self, f: &PackFile) -> std::io::Result<Vec<u8>> {
        match &self.source {
            Source::Folder(dir) => std::fs::read(dir.join(format!("{}{}", self.prefix, f.rel))),
            Source::Zip { archive, .. } => {
                let mut z = archive.lock().unwrap_or_else(|p| p.into_inner());
                let mut e = z.by_name(&f.entry).map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, e.to_string()))?;
                let mut v = Vec::with_capacity(f.size as usize);
                e.read_to_end(&mut v)?;
                Ok(v)
            }
        }
    }

    /// A real file path, for a folder package (zip packages: `None`; see `ModSet::unpack_zips`).
    pub fn disk_path(&self, f: &PackFile) -> Option<PathBuf> {
        match &self.source {
            Source::Folder(dir) => Some(dir.join(format!("{}{}", self.prefix, f.rel))),
            Source::Zip { .. } => None,
        }
    }

    /// Unpack a zip package into `dir` (emptied first) and switch to reading from there. A folder package is left as
    /// it is.
    pub fn unpack_to(&mut self, dir: &Path) -> std::io::Result<()> {
        if !self.is_zip() {
            return Ok(());
        }
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        std::fs::create_dir_all(dir)?;
        for f in &self.files {
            let bytes = self.read(f)?;
            let out = dir.join(&f.rel);
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(out, bytes)?;
        }
        std::fs::write(dir.join(MANIFEST_NAME), &self.manifest_text)?;
        self.use_folder(dir);
        Ok(())
    }

    /// Read from `dir`, which holds this package's files unpacked (by [`Package::unpack_to`], possibly in an earlier run).
    pub fn use_folder(&mut self, dir: &Path) {
        self.source = Source::Folder(dir.to_path_buf());
        self.prefix.clear();
    }
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, u64)>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        let ft = e.file_type()?;
        if ft.is_dir() {
            walk(&e.path(), &r, out)?;
        } else if ft.is_file() {
            out.push((r, e.metadata().map(|m| m.len()).unwrap_or(0)));
        }
    }
    Ok(())
}

/// `""` when the manifest is at the root, `"<top>/"` when it is in the one top folder and nothing else is at the root.
fn manifest_prefix<'a>(rels: impl Iterator<Item = &'a str> + Clone) -> Option<String> {
    if rels.clone().any(|r| r.eq_ignore_ascii_case(MANIFEST_NAME)) {
        return Some(String::new());
    }
    let mut tops = rels.clone().map(|r| r.split_once('/').map_or(("", r), |(a, _)| (a, r)));
    let first = tops.next()?.0.to_string();
    if first.is_empty() || !rels.clone().all(|r| r.starts_with(&format!("{first}/"))) {
        return None;
    }
    let want = format!("{first}/{MANIFEST_NAME}");
    rels.clone().find(|r| r.eq_ignore_ascii_case(&want)).map(|r| r[..first.len() + 1].to_string())
}

fn files_below(prefix: &str, all: impl Iterator<Item = (String, String, u64)>) -> Vec<PackFile> {
    let mut v: Vec<PackFile> = all
        .filter_map(|(rel, entry, size)| {
            let r = rel.strip_prefix(prefix)?.to_string();
            (!r.eq_ignore_ascii_case(MANIFEST_NAME)).then_some(PackFile { rel: r, entry, size })
        })
        .collect();
    v.sort_by(|a, b| a.rel.cmp(&b.rel));
    v
}
