//! Finding, fetching and driving ffmpeg. ffmpeg is never bundled (licence); the order of lookup is
//! `<root>/tools/ffmpeg/`, a path the user chose, `PATH`; otherwise a pinned upstream build is downloaded, its SHA-256 checked
//! against the manifest *before* it is unpacked, and never run if the hash is wrong.

use crate::error::{ErrorKind, InstallError, IoCtx, Result};
use crate::manifest::{Download, Manifest};
use crate::progress::{Ctx, Stage, Unit};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    ToolsFolder,
    UserPath,
    SystemPath,
    Downloaded,
}

#[derive(Debug, Clone)]
pub struct Ffmpeg {
    pub path: PathBuf,
    pub version: String,
    pub source: Source,
}

pub fn exe_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// `<root>/tools/ffmpeg`
pub fn tools_dir(root: &Path) -> PathBuf {
    root.join("tools").join("ffmpeg")
}

pub fn platform_key() -> String {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    format!("{os}-{arch}")
}

/// A command that does not flash a console window on Windows.
pub fn command(program: &Path) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.stdin(Stdio::null());
    c
}

fn run_capture(program: &Path, args: &[&str]) -> Option<Output> {
    command(program).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).output().ok()
}

/// Run `ffmpeg -version`; returns the version string if this really is ffmpeg.
fn version_of(program: &Path) -> Option<String> {
    let out = run_capture(program, &["-hide_banner", "-version"])?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?;
    let v = first.strip_prefix("ffmpeg version ")?;
    Some(v.split_whitespace().next()?.to_string())
}

/// What the converters need: the MP3 encoder (libmp3lame), the XMA2 and Bink decoders, the MJPEG encoder.
pub fn check_capabilities(program: &Path) -> std::result::Result<(), String> {
    let enc = run_capture(program, &["-hide_banner", "-encoders"]).map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let dec = run_capture(program, &["-hide_banner", "-decoders"]).map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let mut missing = Vec::new();
    if !enc.contains("libmp3lame") {
        missing.push("libmp3lame encoder");
    }
    if !enc.contains("mjpeg") {
        missing.push("mjpeg encoder");
    }
    if !dec.contains("xma2") {
        missing.push("xma2 decoder");
    }
    if !dec.contains("binkvideo") || !dec.contains("binkaudio") {
        missing.push("bink decoders");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("this ffmpeg lacks: {}", missing.join(", ")))
    }
}

fn candidate(path: PathBuf, source: Source) -> Option<Ffmpeg> {
    if !path.is_file() {
        return None;
    }
    let version = version_of(&path)?;
    Some(Ffmpeg { path, version, source })
}

/// Find ffmpeg without the network. `user_path` is a file or a folder containing ffmpeg.
pub fn locate(root: &Path, user_path: Option<&Path>) -> Option<Ffmpeg> {
    if let Some(f) = candidate(tools_dir(root).join(exe_name()), Source::ToolsFolder) {
        return Some(f);
    }
    if let Some(u) = user_path {
        let p = if u.is_dir() { u.join(exe_name()) } else { u.to_path_buf() };
        if let Some(f) = candidate(p, Source::UserPath) {
            return Some(f);
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if let Some(f) = candidate(dir.join(exe_name()), Source::SystemPath) {
                return Some(f);
            }
        }
    }
    None
}

/// Which download this machine would use, if the manifest has one.
pub fn pinned(m: &Manifest) -> Option<&Download> {
    m.ffmpeg.downloads.get(&platform_key())
}

/// Download the pinned build into `<root>/tools/ffmpeg/`. The archive is hashed while it arrives and deleted if it does not match.
pub fn download(root: &Path, m: &Manifest, ctx: &Ctx) -> Result<Ffmpeg> {
    let spec = pinned(m).ok_or_else(|| InstallError::new(ErrorKind::FfmpegMissing, format!("no pinned ffmpeg build for {}", platform_key())))?;
    let dir = tools_dir(root);
    std::fs::create_dir_all(&dir).ctx(dir.display())?;
    let zip_path = dir.join("download.zip.part");
    ctx.message(Stage::Ffmpeg, format!("Downloading ffmpeg {} ({:.0} MB) from {}", m.ffmpeg.version, spec.size as f64 / 1e6, spec.url));
    let counter = ctx.counter(Stage::Ffmpeg, Unit::Bytes, spec.size);
    counter.set_message("Downloading ffmpeg");
    let resp = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent("RustyMCLA-installer")
        .build()
        .get(&spec.url)
        .call()
        .map_err(|e| InstallError::new(ErrorKind::FfmpegDownload, e.to_string()))?;
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&zip_path).ctx(zip_path.display())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 << 10];
    let mut got = 0u64;
    let cleanup = |p: &Path| {
        let _ = std::fs::remove_file(p);
    };
    loop {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                cleanup(&zip_path);
                return Err(InstallError::new(ErrorKind::FfmpegDownload, e.to_string()));
            }
        };
        if n == 0 {
            break;
        }
        got += n as u64;
        if got > spec.size {
            cleanup(&zip_path);
            return Err(InstallError::new(ErrorKind::FfmpegHash, format!("download is larger than the pinned {} bytes", spec.size)));
        }
        hasher.update(&buf[..n]);
        if let Err(e) = file.write_all(&buf[..n]) {
            cleanup(&zip_path);
            return Err(InstallError::io("write the ffmpeg download", &e));
        }
        if let Err(e) = counter.add(n as u64) {
            cleanup(&zip_path);
            return Err(e);
        }
    }
    drop(file);
    let hex = crate::validate::hex(&hasher.finalize());
    if got != spec.size || !hex.eq_ignore_ascii_case(&spec.sha256) {
        cleanup(&zip_path);
        return Err(InstallError::new(ErrorKind::FfmpegHash, format!("got {got} bytes, sha256 {hex}; expected {} bytes, {}", spec.size, spec.sha256)));
    }
    counter.finish();
    ctx.message(Stage::Ffmpeg, "Checksum matches; unpacking ffmpeg");
    let target = dir.join(exe_name());
    let part = dir.join(format!("{}.part", exe_name()));
    let r: Result<()> = (|| {
        let f = std::fs::File::open(&zip_path).ctx(zip_path.display())?;
        let mut zip = zip::ZipArchive::new(f).map_err(|e| InstallError::new(ErrorKind::FfmpegDownload, format!("not a zip: {e}")))?;
        let mut member = zip.by_name(&spec.member).map_err(|e| InstallError::new(ErrorKind::FfmpegDownload, format!("{} not in the archive: {e}", spec.member)))?;
        let mut out = std::fs::File::create(&part).ctx(part.display())?;
        std::io::copy(&mut member, &mut out).ctx("unpack ffmpeg")?;
        drop(out);
        let _ = std::fs::remove_file(&target);
        std::fs::rename(&part, &target).ctx(target.display())?;
        Ok(())
    })();
    cleanup(&zip_path);
    if let Err(e) = r {
        cleanup(&part);
        return Err(e);
    }
    let notes = format!("ffmpeg {} downloaded by Rusty MCLA\nfrom: {}\nsha256 of the archive: {}\nlicence: {}\n", m.ffmpeg.version, spec.url, spec.sha256, m.ffmpeg.license);
    let _ = std::fs::write(dir.join("SOURCE.txt"), notes);
    candidate(target, Source::Downloaded).ok_or_else(|| InstallError::new(ErrorKind::FfmpegMissing, "the unpacked ffmpeg does not run"))
}

/// Locate ffmpeg, or download it when `allow_download` is set. Also checks that it has the codecs the converters need.
pub fn ensure(root: &Path, user_path: Option<&Path>, allow_download: bool, m: &Manifest, ctx: &Ctx) -> Result<Ffmpeg> {
    ctx.message(Stage::Ffmpeg, "Looking for ffmpeg");
    let mut found = locate(root, user_path);
    if found.is_none() {
        if !allow_download {
            return Err(InstallError::new(ErrorKind::FfmpegMissing, "not in tools/ffmpeg, the chosen path or PATH"));
        }
        found = Some(download(root, m, ctx)?);
    }
    let f = found.ok_or_else(|| InstallError::new(ErrorKind::FfmpegMissing, "ffmpeg not found"))?;
    if let Err(why) = check_capabilities(&f.path) {
        return Err(InstallError::new(ErrorKind::FfmpegMissing, format!("{}: {why}", f.path.display())));
    }
    ctx.message(Stage::Ffmpeg, format!("Using ffmpeg {} at {}", f.version, f.path.display()));
    Ok(f)
}

/// Run ffmpeg, true on exit code 0 (stderr tail returned for the log).
/// Run `attempt`; if it reports failure, wait a moment and run it once more, and return the second result. A conversion that fails
/// with no message while many ffmpeg processes run side by side (#56 follow-up: one stream out of 1,631 failed in a full install and
/// converted fine alone, byte for byte as in the other installs) is a passing failure; a real one fails again and is reported.
pub fn retry_once(mut attempt: impl FnMut() -> (bool, String)) -> (bool, String) {
    let first = attempt();
    if first.0 {
        return first;
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    attempt()
}

pub fn run(ff: &Path, args: &[std::ffi::OsString]) -> (bool, String) {
    match command(ff).args(args).stdout(Stdio::null()).stderr(Stdio::piped()).output() {
        Ok(o) => {
            let e = String::from_utf8_lossy(&o.stderr);
            let tail: String = e.trim().chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
            (o.status.success(), tail)
        }
        Err(e) => (false, e.to_string()),
    }
}
