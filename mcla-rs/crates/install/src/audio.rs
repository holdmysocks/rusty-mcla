//! Audio conversion: port of `tools/convert_audio.py`.
//!
//! The disc stores RAGE audio banks whose waves are XMA2. They are unwrapped here and decoded by ffmpeg:
//!
//! * wave banks   -> `<out>/<bank path>/<wave name>.wav` (16-bit PCM, mono) and `<out>/<bank path>/_index.tsv`
//! * stream files -> `<out>/<path>.mp3` (stereo, surround folded down) and `<path>.mp3.txt`
//!
//! Wave names are recovered by hashing (Jenkins one-at-a-time, lower case) every name found in the game's sound metadata
//! (`config/sounds.dat`, `game.dat`, `speech.dat`). Unlike the script this reads the `audio/X360/sfx` trees straight from the
//! disc archives, so no 2.3 GB of raw intermediate files is written. File names and contents are meant to be identical to the script's.
//! Text files are written with CRLF line ends, as the script (Python text mode on Windows) wrote them.

use crate::error::{ErrorKind, InstallError, IoCtx, Result};
use crate::ffmpeg;
use crate::progress::{Ctx, Stage, Unit};
use mcla_formats::rpf3::{Kind, Rpf3};
use mcla_formats::ReadAt;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub type NameMap = HashMap<u32, String>;

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'\\' | b'/' | b' ' | b'.' | b'-')
}

/// `[A-Za-z0-9_\\/ .\-]{3,120}` matches over raw bytes, as `re.finditer` would find them.
fn name_matches(d: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < d.len() {
        if !is_name_byte(d[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < d.len() && is_name_byte(d[i]) {
            i += 1;
        }
        let mut p = start;
        while i - p >= 3 {
            let n = (i - p).min(120);
            out.push(&d[p..p + n]);
            p += n;
        }
    }
    out
}

/// The script's hash: lower-cased bytes, backslashes kept (unlike `mcla_formats::hash::joaat`).
fn joaat_latin1(s: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &c in s {
        let c = c.to_ascii_lowercase();
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h
}

/// Recover wave names from the audio config files in `config_dir`.
pub fn build_names(config_dir: &Path) -> NameMap {
    let mut names = NameMap::new();
    for f in ["sounds.dat", "game.dat", "speech.dat"] {
        let Ok(d) = std::fs::read(config_dir.join(f)) else { continue };
        for m in name_matches(&d) {
            let s = String::from_utf8_lossy(m).into_owned(); // ASCII only
            // strings are length-prefixed: try without the leading byte(s), and every "_" suffix
            for base in [&s[..], &s[1.min(s.len())..], &s[2.min(s.len())..]] {
                let parts: Vec<&str> = base.split('_').collect();
                for i in 0..parts.len() {
                    let t = parts[i..].join("_");
                    if t.len() >= 2 {
                        names.entry(joaat_latin1(t.as_bytes())).or_insert(t);
                    }
                }
            }
        }
    }
    for extra in ["LEFT", "RIGHT", "L", "R", "CENTRE", "CENTER", "LFE", "LS", "RS", "MONO", "FRONT_LEFT", "FRONT_RIGHT", "REAR_LEFT", "REAR_RIGHT"] {
        names.entry(joaat_latin1(extra.as_bytes())).or_insert_with(|| extra.to_string());
    }
    names
}

// ---- binary helpers: a short read is an error, like struct.error in the script ----

struct Be<'a>(&'a [u8]);

impl<'a> Be<'a> {
    fn bytes(&self, o: u64, n: usize) -> std::result::Result<&'a [u8], String> {
        let o = usize::try_from(o).map_err(|_| "offset too large".to_string())?;
        self.0.get(o..o.checked_add(n).ok_or("offset overflow")?).ok_or_else(|| format!("short read at {o:#x}"))
    }
    fn u16(&self, o: u64) -> std::result::Result<u16, String> {
        let b = self.bytes(o, 2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&self, o: u64) -> std::result::Result<u32, String> {
        let b = self.bytes(o, 4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&self, o: u64) -> std::result::Result<u64, String> {
        let b = self.bytes(o, 8)?;
        Ok(u64::from_be_bytes(b.try_into().unwrap_or([0; 8])))
    }
    /// Python slice semantics: clamped, never an error.
    fn slice(&self, start: u64, len: u64) -> &'a [u8] {
        let n = self.0.len() as u64;
        let a = start.min(n);
        let b = start.saturating_add(len).min(n);
        &self.0[a as usize..b as usize]
    }
}

pub struct Wave {
    pub hash: u32,
    pub data: Vec<u8>,
    pub rate: u16,
    pub samples: u32,
    pub flags: u32,
    /// Loop start sample (-1 = one-shot).
    pub loop_start: i32,
    /// Headroom in 0.01 dB: subtracted from the volume when the wave plays.
    pub headroom: i16,
}

pub fn parse_bank(d: &[u8]) -> std::result::Result<Vec<Wave>, String> {
    let r = Be(d);
    let tab = r.u64(0)?;
    let n = r.u32(16)? as u64;
    let ds = r.u32(24)? as u64;
    let info0 = tab.wrapping_add(n * 16);
    let mut out = Vec::new();
    for i in 0..n {
        let e = tab.wrapping_add(i * 16);
        let off = r.u64(e)?;
        let h = r.u32(e + 8)?;
        let o = info0.wrapping_add(off);
        let doff = r.u64(o)?;
        let size = r.u32(o + 12)? as u64;
        let ns = r.u32(o + 16)?;
        let loop_start = r.u32(o + 20)? as i32;
        let rate = r.u16(o + 24)?;
        let headroom = r.u16(o + 26)? as i16;
        let flags = r.u32(o + 48)?;
        out.push(Wave { hash: h, data: r.slice(ds.wrapping_add(doff), size).to_vec(), rate, samples: ns, flags, loop_start, headroom });
    }
    Ok(out)
}

pub struct Channel {
    pub hash: u32,
    pub rate: u16,
    pub samples: u32,
    /// Headroom in 0.01 dB (as a wave's).
    pub headroom: i16,
    pub data: Vec<u8>,
}

pub fn parse_stream(d: &[u8]) -> std::result::Result<Vec<Channel>, String> {
    let r = Be(d);
    let _btab = r.u64(0)?;
    let nblocks = r.u32(8)? as u64;
    let bsize = r.u32(12)? as u64;
    let ctab = r.u32(0x18)? as u64;
    let nch = r.u32(0x24)? as u64;
    let hdr = r.u32(0x2C)? as u64;
    let info0 = ctab + nch * 16;
    let mut chans = Vec::new();
    let mut sizes = Vec::new();
    for c in 0..nch {
        let e = ctab + c * 16;
        let off = r.u64(e)?;
        let h = r.u32(e + 8)?;
        let o = info0.wrapping_add(off);
        let _z = r.u64(o)?;
        let _h2 = r.u32(o + 8)?;
        let size = r.u32(o + 12)? as usize;
        let ns = r.u32(o + 16)?;
        let rate = r.u16(o + 24)?;
        let headroom = r.u16(o + 26)? as i16;
        chans.push(Channel { hash: h, rate, samples: ns, headroom, data: Vec::new() });
        sizes.push(size);
    }
    for b in 0..nblocks {
        let bo = hdr + b * bsize;
        let choff = r.u64(bo)?;
        let seekoff = r.u64(bo + 8)?;
        let mut counts = Vec::with_capacity(nch as usize);
        let mut skips = Vec::with_capacity(nch as usize);
        for c in 0..nch {
            let base = bo + choff + c * 16;
            let _start = r.u32(base)?;
            let npk = r.u32(base + 4)? as u64;
            let skip = r.u32(base + 8)?;
            let _ns = r.u32(base + 12)?;
            counts.push(npk);
            skips.push(skip);
        }
        let total_seek: u64 = counts.iter().sum();
        let data0 = bo + ((seekoff + total_seek * 8 + 0x7FF) & !0x7FF);
        let mut p = data0;
        for c in 0..nch as usize {
            // A block whose channel record has samples to skip starts with the previous block's
            // last packet again (the frame across the boundary); the stream is the packets once.
            let first = if skips[c] != 0 { 1.min(counts[c]) } else { 0 };
            chans[c].data.extend_from_slice(r.slice(p + first * 0x800, (counts[c] - first) * 0x800));
            p += counts[c] * 0x800;
        }
    }
    for (c, size) in chans.iter_mut().zip(sizes) {
        if size != 0 {
            c.data.truncate(size);
        }
    }
    Ok(chans)
}

/// Mono XMA2 stream wrapped in a RIFF/WAVE container with an XMA2WAVEFORMATEX.
pub fn xma_riff(data: &[u8], rate: u16, samples: u32) -> std::result::Result<Vec<u8>, String> {
    let blocks = (data.len() / 2048).max(1);
    let avg = (data.len() as u128 * rate as u128 / (samples.max(1) as u128)).max(1);
    let avg = u32::try_from(avg).map_err(|_| "byte rate out of range".to_string())?;
    let blocks = u16::try_from(blocks).map_err(|_| "too many blocks".to_string())?;
    let mut fmt = Vec::with_capacity(34 + 18);
    fmt.extend_from_slice(&0x166u16.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&(rate as u32).to_le_bytes());
    fmt.extend_from_slice(&avg.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    fmt.extend_from_slice(&34u16.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    for v in [0u32, samples, (data.len() as u32).min(2048 * 16), 0, samples, 0, 0] {
        fmt.extend_from_slice(&v.to_le_bytes());
    }
    fmt.push(0);
    fmt.push(4);
    fmt.extend_from_slice(&blocks.to_le_bytes());
    let mut body = Vec::with_capacity(data.len() + fmt.len() + 20);
    body.extend_from_slice(b"WAVE");
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(data);
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// `re.sub(r"[^A-Za-z0-9_.\-]", "_", name).strip(".").lower()`
pub fn safe(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' }).collect();
    s.trim_matches('.').to_ascii_lowercase()
}

/// Is this file a stream (as opposed to a wave bank)? Needs the first 0x30 bytes and the file size.
pub fn is_stream(head: &[u8], size: u64) -> Option<bool> {
    if head.len() < 0x30 {
        return None;
    }
    let nblocks = u32::from_be_bytes([head[8], head[9], head[10], head[11]]) as u64;
    let bsize = u32::from_be_bytes([head[12], head[13], head[14], head[15]]) as u64;
    let hdr = u32::from_be_bytes([head[0x2C], head[0x2D], head[0x2E], head[0x2F]]) as u64;
    Some(bsize >= 0x800 && bsize % 0x800 == 0 && nblocks > 0 && hdr + (nblocks - 1) * bsize < size && size <= hdr + nblocks * bsize)
}

fn crlf(lines: &[String]) -> String {
    let mut s = lines.join("\r\n");
    s.push_str("\r\n");
    s
}

fn tmp_remove(p: &Path) {
    let _ = std::fs::remove_file(p);
}

/// Convert one wave bank. Returns (waves ok, waves in the bank) and notes problems.
fn convert_bank(ff: &Path, data: &[u8], out_dir: &Path, names: &NameMap, problems: &Mutex<Vec<String>>, label: &str) -> Result<(usize, usize)> {
    let waves = parse_bank(data).map_err(|e| InstallError::new(ErrorKind::Convert, format!("{label}: {e}")))?;
    std::fs::create_dir_all(out_dir).ctx(out_dir.display())?;
    let mut used = HashSet::new();
    let mut scheduled: HashSet<PathBuf> = HashSet::new();
    struct Todo {
        nm: String,
        wav: PathBuf,
        tmp: PathBuf,
    }
    let mut lines = Vec::new();
    let mut todo: Vec<(usize, Todo)> = Vec::new(); // index into `entries`
    let mut entries: Vec<(String, &Wave, bool)> = Vec::new(); // name, wave, ok
    for w in &waves {
        let mut nm = safe(&names.get(&w.hash).cloned().unwrap_or_else(|| format!("{:08x}", w.hash)));
        if used.contains(&nm) {
            nm = format!("{}_{:08x}", nm, w.hash);
        }
        used.insert(nm.clone());
        let wav = out_dir.join(format!("{nm}.wav"));
        let mut ok = true;
        // a name used twice in one bank is converted once (the script found the file already there)
        if !wav.exists() && !scheduled.contains(&wav) && !w.data.is_empty() {
            scheduled.insert(wav.clone());
            let tmp = out_dir.join(format!("{nm}.xma"));
            match xma_riff(&w.data, w.rate, w.samples) {
                Ok(riff) => {
                    std::fs::write(&tmp, riff).ctx(tmp.display())?;
                    todo.push((entries.len(), Todo { nm: nm.clone(), wav, tmp }));
                }
                Err(e) => {
                    problems.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{label} {nm}: {e}"));
                    ok = false;
                }
            }
        }
        entries.push((nm, w, ok));
    }
    // decode in batches: one ffmpeg process for many inputs; a failing batch is redone one by one
    for batch in todo.chunks(48) {
        let mut args: Vec<OsString> = vec!["-y".into(), "-v".into(), "error".into()];
        for (_, t) in batch {
            args.push("-i".into());
            args.push(t.tmp.clone().into());
        }
        for (k, (_, t)) in batch.iter().enumerate() {
            for a in ["-map".to_string(), format!("{k}:a"), "-ac".into(), "1".into(), "-c:a".into(), "pcm_s16le".into()] {
                args.push(a.into());
            }
            args.push(t.wav.clone().into());
        }
        let (good, _) = ffmpeg::run(ff, &args);
        if !good {
            for (idx, t) in batch {
                let (g, err) = ffmpeg::run(ff, &["-y".into(), "-v".into(), "error".into(), "-i".into(), t.tmp.clone().into(), "-ac".into(), "1".into(), "-c:a".into(), "pcm_s16le".into(), t.wav.clone().into()]);
                if !g {
                    tmp_remove(&t.wav);
                    entries[*idx].2 = false;
                    problems.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{label} {}: ffmpeg failed: {err}", t.nm));
                }
            }
        }
        for (_, t) in batch {
            tmp_remove(&t.tmp);
        }
    }
    let mut ok = 0;
    for (nm, w, good) in &entries {
        if *good {
            ok += 1;
            // + loop start (-1 = one-shot) and headroom (0.01 dB) of the wave info
            lines.push(format!("{}\t{:08x}\t{}\t{}\t{:x}\t{}\t{}", nm, w.hash, w.samples, w.rate, w.flags, w.loop_start, w.headroom));
        }
    }
    let idx = out_dir.join("_index.tsv");
    std::fs::write(&idx, crlf(&lines)).ctx(idx.display())?;
    Ok((ok, waves.len()))
}

/// Remove every converted stream (`<name>.mp3` with its `<name>.mp3.txt`) below `dir`, so the next audio stage converts them again.
pub fn remove_streams(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            remove_streams(&p);
        } else if let Some(mp3) = p.to_str().and_then(|s| s.strip_suffix(".mp3.txt")) {
            let _ = std::fs::remove_file(format!("{mp3}.mp3"));
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn convert_stream(ff: &Path, data: &[u8], base_name: &str, out_path: &Path, names: &NameMap, problems: &Mutex<Vec<String>>, label: &str) -> Result<(usize, usize)> {
    let txt = PathBuf::from(format!("{}.txt", out_path.display()));
    if out_path.exists() && txt.exists() {
        return Ok((1, 1));
    }
    let chans = parse_stream(data).map_err(|e| InstallError::new(ErrorKind::Convert, format!("{label}: {e}")))?;
    if let Some(p) = out_path.parent() {
        std::fs::create_dir_all(p).ctx(p.display())?;
    }
    // Channels are stored in hash order: find out which speaker each one is from its name
    // ("<STREAM>.L", "..._LS", "..._RIGHT"); the names hash from the stream's own name.
    // stereo fold of the speaker masks (the original's speaker-mask panning in stereo: front to its side, centre and
    // surround add 0.7071, LFE is not heard)
    fn weights(r: &str) -> (f64, f64) {
        match r {
            "L" => (1.0, 0.0),
            "R" => (0.0, 1.0),
            "C" => (0.7071, 0.7071),
            "LS" => (0.7071, 0.0),
            "RS" => (0.0, 0.7071),
            "LFE" => (0.0, 0.0),
            _ => (0.7071, 0.7071), // "M"
        }
    }
    let role = |c: &Channel| -> &'static str {
        let n = names.get(&c.hash).cloned().unwrap_or_default().to_ascii_uppercase();
        if n.is_empty() || n == "?" {
            for (suf, r) in [(".L", "L"), (".R", "R"), (".C", "C"), (".LS", "LS"), (".RS", "RS"), ("_L", "L"), ("_R", "R"), ("_C", "C"), ("_LS", "LS"), ("_RS", "RS"), ("_LEFT", "L"), ("_RIGHT", "R"), (".LFE", "LFE")] {
                if joaat_latin1(format!("{base_name}{suf}").as_bytes()) == c.hash {
                    return r;
                }
            }
            return "M";
        }
        for (suf, r) in [("LS", "LS"), ("RS", "RS"), ("LFE", "LFE"), ("LEFT", "L"), ("RIGHT", "R"), ("CENTRE", "C"), ("CENTER", "C"), ("L", "L"), ("R", "R"), ("C", "C")] {
            if n.ends_with(&format!("_{suf}")) || n.ends_with(&format!(".{suf}")) {
                return r;
            }
        }
        "M"
    };
    let mut chans = chans;
    let mut rl: Vec<&'static str> = chans.iter().map(&role).collect();
    if chans.len() == 2 && rl[0] == "R" && rl[1] == "L" {
        chans.reverse();
        rl.reverse();
    }
    if chans.len() == 2 && rl.contains(&"M") {
        rl = vec!["L", "R"];
    }
    let mut tmps = Vec::new();
    for (i, c) in chans.iter().enumerate() {
        let tmp = PathBuf::from(format!("{}.ch{}.xma", out_path.display(), i));
        let riff = xma_riff(&c.data, c.rate, c.samples).map_err(|e| InstallError::new(ErrorKind::Convert, format!("{label}: {e}")))?;
        std::fs::write(&tmp, riff).ctx(tmp.display())?;
        tmps.push(tmp);
    }
    let mut args: Vec<OsString> = vec!["-y".into(), "-v".into(), "error".into()];
    for t in &tmps {
        // (say what it is: some payloads look like MPEG-TS to the prober)
        args.push("-f".into());
        args.push("wav".into());
        args.push("-i".into());
        args.push(t.clone().into());
    }
    let n = tmps.len();
    let enc = ["-c:a", "libmp3lame", "-q:a", "2"];
    if n == 1 {
        for a in enc {
            args.push(a.into());
        }
        args.push(out_path.into());
    } else {
        // one mono stream per channel -> stereo, every speaker folded to its side
        // each channel's own headroom, relative to the stream's lowest (the rest is applied by the game, from the .txt)
        let h0 = chans.iter().map(|c| c.headroom).min().unwrap_or(0);
        let g: Vec<f64> = chans.iter().map(|c| 10f64.powf(-((c.headroom as i32 - h0 as i32) as f64) / 2000.0)).collect();
        let side = |pick: fn((f64, f64)) -> f64| -> String {
            let parts: Vec<String> = rl.iter().enumerate().filter(|(_, r)| pick(weights(r)) > 0.0).map(|(i, r)| format!("{:.4}*c{}", pick(weights(r)) * g[i], i)).collect();
            if parts.is_empty() {
                "0*c0".to_string()
            } else {
                parts.join("+")
            }
        };
        let left = side(|w| w.0);
        let right = side(|w| w.1);
        let inputs: String = (0..n).map(|i| format!("[{i}:a]")).collect();
        args.push("-filter_complex".into());
        args.push(format!("{inputs}amerge=inputs={n},pan=stereo|c0={left}|c1={right}[a]").into());
        args.push("-map".into());
        args.push("[a]".into());
        for a in enc {
            args.push(a.into());
        }
        args.push(out_path.into());
    }
    let (good, err) = ffmpeg::retry_once(|| ffmpeg::run(ff, &args));
    for t in &tmps {
        tmp_remove(t);
    }
    if !good {
        tmp_remove(out_path);
        problems.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{label}: ffmpeg failed: {err}"));
        return Ok((0, 1));
    }
    let lines: Vec<String> = chans.iter().enumerate().map(|(i, c)| format!("{}\t{:08x}\t{}\t{}\t{}\t{}", i, c.hash, names.get(&c.hash).map(|s| s.as_str()).unwrap_or("?"), c.samples, c.rate, c.headroom)).collect();
    std::fs::write(&txt, crlf(&lines)).ctx(txt.display())?;
    Ok((1, 1))
}

/// One audio file of the disc to convert.
pub struct Item {
    /// Index of the archive in the list passed to [`convert`].
    pub archive: usize,
    pub entry: usize,
    /// Path below `audio/X360/sfx/`, `/` separated, as the script's `rel`.
    pub rel: String,
    /// The file's own name (last path part).
    pub name: String,
    /// Stored size in the archive; progress is counted in these bytes.
    pub size: u64,
}

#[derive(Debug, Default)]
pub struct AudioReport {
    pub files: usize,
    pub banks: usize,
    pub streams: usize,
    /// Messages for files / waves that could not be converted.
    pub problems: Vec<String>,
}

const SFX_PREFIX: &str = "audio/x360/sfx/";

/// List the `audio/X360/sfx` files of the archives.
pub fn list_items<R: ReadAt>(archives: &[&Rpf3<R>], names: &mcla_formats::names::Names) -> Vec<Item> {
    let mut items = Vec::new();
    for (a, rpf) in archives.iter().enumerate() {
        for i in 0..rpf.entries.len() {
            let Kind::File { size, .. } = rpf.entries[i].kind else { continue };
            let p = rpf.path_of(i, names);
            if p.len() > SFX_PREFIX.len() && p[..SFX_PREFIX.len()].eq_ignore_ascii_case(SFX_PREFIX) {
                let rel = p[SFX_PREFIX.len()..].to_string();
                let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
                items.push(Item { archive: a, entry: i, rel, name, size: size as u64 });
            }
        }
    }
    items
}

/// Convert every item. `read` returns the file's bytes. Problems are collected, not fatal (the script did the same).
pub fn convert(items: &[Item], read: &(dyn Fn(&Item) -> Result<Vec<u8>> + Sync), ff: &Path, out: &Path, names: &NameMap, jobs: usize, ctx: &Ctx) -> Result<AudioReport> {
    std::fs::create_dir_all(out).ctx(out.display())?;
    let counter = ctx.counter(Stage::Audio, Unit::Bytes, items.iter().map(|i| i.size.max(1)).sum());
    counter.set_message("Converting audio");
    let problems = Mutex::new(Vec::<String>::new());
    let banks = std::sync::atomic::AtomicUsize::new(0);
    let streams = std::sync::atomic::AtomicUsize::new(0);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(jobs.max(1)).build().map_err(|e| InstallError::new(ErrorKind::Io, e.to_string()))?;
    let res: Result<()> = pool.install(|| {
        items.par_iter().try_for_each(|it| {
            let data = read(it)?;
            let relsafe: Vec<String> = it.rel.split('/').map(safe).collect();
            let relsafe = relsafe.join("/");
            let label = it.rel.as_str();
            match is_stream(&data[..data.len().min(0x30)], data.len() as u64) {
                None => {} // shorter than a header: skipped
                Some(true) => {
                    streams.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let target = out.join(format!("{relsafe}.mp3"));
                    match convert_stream(ff, &data, &it.name, &target, names, &problems, label) {
                        Ok((a, b)) if a == b => {}
                        Ok(_) => {}
                        Err(e) if e.kind == ErrorKind::Convert => problems.lock().unwrap_or_else(|p| p.into_inner()).push(e.detail),
                        Err(e) => return Err(e),
                    }
                }
                Some(false) => {
                    banks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    match convert_bank(ff, &data, &out.join(&relsafe), names, &problems, label) {
                        Ok((a, b)) if a == b => {}
                        Ok((a, b)) => problems.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{label}: {a} of {b} waves converted")),
                        Err(e) if e.kind == ErrorKind::Convert => problems.lock().unwrap_or_else(|p| p.into_inner()).push(e.detail),
                        Err(e) => return Err(e),
                    }
                }
            }
            counter.add(it.size.max(1))
        })
    });
    res?;
    counter.finish();
    let mut problems = problems.into_inner().unwrap_or_default();
    problems.sort();
    Ok(AudioReport { files: items.len(), banks: banks.load(std::sync::atomic::Ordering::Relaxed), streams: streams.load(std::sync::atomic::Ordering::Relaxed), problems })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joaat_matches_the_formats_crate() {
        // same as the formats crate for names without backslashes (which the script's hash keeps)
        for s in ["AMB_QUADSHOT_TUNNEL_05_L", "LEFT", "x"] {
            assert_eq!(joaat_latin1(s.as_bytes()), mcla_formats::hash::joaat(s));
        }
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe("Foo Bar.wav"), "foo_bar.wav");
        assert_eq!(safe("..x.."), "x");
        assert_eq!(safe("A-b_C"), "a-b_c");
    }

    #[test]
    fn name_matches_like_the_regex() {
        let d = b"\x01ab\x00abc\x00";
        let m = name_matches(d);
        assert_eq!(m, vec![&b"abc"[..]]);
        let long = vec![b'a'; 245];
        let m = name_matches(&long);
        assert_eq!(m.iter().map(|x| x.len()).collect::<Vec<_>>(), vec![120, 120, 5]);
        let long = vec![b'a'; 242];
        assert_eq!(name_matches(&long).iter().map(|x| x.len()).collect::<Vec<_>>(), vec![120, 120]);
    }

    #[test]
    fn riff_layout() {
        let r = xma_riff(&[1, 2, 3], 48000, 100).unwrap();
        assert_eq!(&r[0..4], b"RIFF");
        assert_eq!(&r[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes([r[16], r[17], r[18], r[19]]), 52); // WAVEFORMATEX (18) + XMA2 extension (34)
        assert_eq!(r.len(), 8 + 4 + 8 + 52 + 8 + 3);
        assert_eq!(u16::from_le_bytes([r[20], r[21]]), 0x166);
    }

    #[test]
    fn short_bank_is_an_error() {
        assert!(parse_bank(&[0u8; 10]).is_err());
        assert!(parse_stream(&[0u8; 10]).is_err());
    }

    #[test]
    fn stream_block_repeat_packet_is_dropped() {
        // one channel, two blocks; the second has samples to skip and starts with the first
        // block's last packet again (as in every music stream of the disc)
        let (hdr, bsize) = (0x800u64, 0x2000u64);
        let mut d = vec![0u8; (hdr + 2 * bsize) as usize];
        d[8..12].copy_from_slice(&2u32.to_be_bytes());
        d[12..16].copy_from_slice(&(bsize as u32).to_be_bytes());
        d[0x18..0x1C].copy_from_slice(&0x40u32.to_be_bytes());
        d[0x24..0x28].copy_from_slice(&1u32.to_be_bytes());
        d[0x2C..0x30].copy_from_slice(&(hdr as u32).to_be_bytes());
        d[0x48..0x4C].copy_from_slice(&0x1234u32.to_be_bytes());
        d[0x5C..0x60].copy_from_slice(&0x1800u32.to_be_bytes()); // size: three packets
        d[0x68..0x6A].copy_from_slice(&44100u16.to_be_bytes());
        for (b, skip, pk) in [(0u64, 0u32, [1u8, 2]), (1, 700, [2, 3])] {
            let bo = (hdr + b * bsize) as usize;
            d[bo..bo + 8].copy_from_slice(&0x18u64.to_be_bytes());
            d[bo + 8..bo + 16].copy_from_slice(&0x28u64.to_be_bytes());
            d[bo + 0x1C..bo + 0x20].copy_from_slice(&2u32.to_be_bytes());
            d[bo + 0x20..bo + 0x24].copy_from_slice(&skip.to_be_bytes());
            for (i, v) in pk.iter().enumerate() {
                let p = bo + 0x800 + i * 0x800;
                d[p..p + 0x800].fill(*v);
            }
        }
        let ch = parse_stream(&d).unwrap();
        let firsts: Vec<u8> = ch[0].data.chunks(0x800).map(|p| p[0]).collect();
        assert_eq!(firsts, vec![1, 2, 3]);
    }

    #[test]
    fn stream_detection() {
        let mut h = vec![0u8; 0x30];
        h[8..12].copy_from_slice(&2u32.to_be_bytes());
        h[12..16].copy_from_slice(&0x800u32.to_be_bytes());
        h[0x2C..0x30].copy_from_slice(&0x1000u32.to_be_bytes());
        assert_eq!(is_stream(&h, 0x1000 + 0x1000), Some(true));
        assert_eq!(is_stream(&h, 0x1000 + 0x2001), Some(false));
        assert_eq!(is_stream(&h[..10], 100), None);
    }
}
