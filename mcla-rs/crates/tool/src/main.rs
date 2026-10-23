//! `mcla-tool` – inspect and extract MC:LA disc images.
//!
//! ```text
//! mcla-tool ls      <iso>                       list files inside the XDVDFS partition
//! mcla-tool rpf     <iso> <archive> [names.txt] list an RPF3 archive
//! mcla-tool extract <iso> <archive> <out> [names.txt] [--raw]   extract (resources decompressed)
//! mcla-tool plainscripts <iso> <data dir>       convert the scripts of an old extraction to the plain form
//! mcla-tool hash    <string>...                 print joaat hashes
//! ```

mod cmds;
mod swfcmd;

use anyhow::{bail, Context, Result};
use mcla_formats::iso::Xiso;
use mcla_formats::names::Names;
use mcla_formats::rpf3::{Kind, Rpf3};
use mcla_formats::rsc::Resource;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn main() -> Result<()> {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        bail!("usage: mcla-tool <ls|rpf|extract|hash> ...");
    }
    match args[1].as_str() {
        "ls" => cmd_ls(&args[2..]),
        "rpf" => cmd_rpf(&args[2..]),
        "extract" => cmd_extract(&args[2..]),
        "install" => cmd_install(&args[2..]),
        "shaders-wgsl" => cmd_shaders_wgsl(&args[2..]),
        "ffmpeg" => cmd_ffmpeg(&args[2..]),
        "plainscripts" => cmd_plainscripts(&args[2..]),
        "xmemtest" => cmd_xmemtest(&args[2..]),
        "sector" => cmd_sector(&args[2..]),
        "citymap" => cmd_citymap(&args[2..]),
        "drawable" => cmds::cmd_drawable(&args[2..]),
        "uimesh" => cmds::cmd_uimesh(&args[2..]),
        "shaders" => cmds::cmd_shaders(&args[2..]),
        "meshcol" => cmds::cmd_meshcol(&args[2..]),
        "secshaders" => cmds::cmd_secshaders(&args[2..]),
        "cartex" => cmds::cmd_cartex(&args[2..]),
        "texdict" => cmd_texdict(&args[2..]),
        "uitex" => cmd_uitex(&args[2..]),
        "swf" => swfcmd::cmd_swf(&args[2..]),
        "navpath" => cmd_navpath(&args[2..]),
        "props" => cmd_props(&args[2..]),
        "char" => cmd_char(&args[2..]),
        "sco" => cmd_sco(&args[2..]),
        "tex" => cmd_tex(&args[2..]),
        "cine" => cmd_cine(&args[2..]),
        "scocalls" => {
            // native calls of a script with the arguments a straight run can follow
            let d = std::fs::read(&args[2])?;
            let sc = mcla_formats::sco::decode(&d)?;
            for c in sc.calls() {
                println!("{:06x} {:08x} {:?}", c.pc, c.hash, c.args);
            }
            Ok(())
        }
        "scoraw" => {
            // decoded script: 12 bytes (code size, statics, globals; LE) + data
            let d = std::fs::read(&args[2])?;
            let sc = mcla_formats::sco::decode(&d)?;
            let mut out = Vec::new();
            for v in [sc.code_size, sc.statics, sc.globals] {
                out.extend_from_slice(&(v as u32).to_le_bytes());
            }
            out.extend_from_slice(&sc.data);
            std::fs::write(&args[3], out)?;
            Ok(())
        }
        "mcv" => cmd_mcv(&args[2..]),
        "xex" => {
            // xex <iso> <offset> <length> <out.bin>: decrypt and decompress an XEX2 executable
            use aes::cipher::{BlockDecrypt, KeyInit};
            use std::io::{Read, Seek, SeekFrom};
            let iso = args.get(2).context("usage: xex <iso> <offset> <length> <out>")?;
            let off: u64 = args.get(3).context("offset")?.parse()?;
            let len: usize = args.get(4).context("length")?.parse()?;
            let out = args.get(5).context("out")?;
            let mut f = std::fs::File::open(iso)?;
            f.seek(SeekFrom::Start(off))?;
            let mut d = vec![0u8; len];
            f.read_exact(&mut d)?;
            let be32 = |d: &[u8], o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
            let be16 = |d: &[u8], o: usize| u16::from_be_bytes([d[o], d[o + 1]]);
            if &d[0..4] != b"XEX2" {
                anyhow::bail!("not an XEX2 file");
            }
            let pe_off = be32(&d, 8) as usize;
            let sec = be32(&d, 16) as usize;
            let nhdr = be32(&d, 20) as usize;
            let mut fmt = 0usize;
            let mut base = 0u32;
            let mut entry = 0u32;
            for i in 0..nhdr {
                let (k, v) = (be32(&d, 24 + i * 8), be32(&d, 28 + i * 8));
                match k {
                    0x0000_03ff => fmt = v as usize,
                    0x0001_0201 => base = v,
                    0x0001_0100 => entry = v,
                    _ => {}
                }
            }
            let image_size = be32(&d, sec + 4) as usize;
            let load = be32(&d, sec + 0x110);
            println!("pe data at {:#x}, image size {:#x}, load address {:#x}, base {:#x}, entry {:#x}", pe_off, image_size, load, base, entry);
            let enc = be16(&d, fmt + 4);
            let comp = be16(&d, fmt + 6);
            println!("encryption {} compression {}", enc, comp);
            let mut data = d[pe_off..].to_vec();
            if enc != 0 {
                let retail = [0x20u8, 0xB1, 0x85, 0xA5, 0x9D, 0x28, 0xFD, 0xC3, 0x40, 0x58, 0x3F, 0xBB, 0x08, 0x96, 0xBF, 0x91];
                let mut key = [0u8; 16];
                key.copy_from_slice(&d[sec + 0x150..sec + 0x160]);
                let c = aes::Aes128::new(&retail.into());
                let mut blk = aes::Block::from(key);
                c.decrypt_block(&mut blk);
                let session = aes::Aes128::new(&blk);
                // CBC, zero IV
                let mut prev = [0u8; 16];
                for chunk in data.chunks_exact_mut(16) {
                    let mut cur = [0u8; 16];
                    cur.copy_from_slice(chunk);
                    let mut b = aes::Block::from(cur);
                    session.decrypt_block(&mut b);
                    for k in 0..16 {
                        chunk[k] = b[k] ^ prev[k];
                    }
                    prev = cur;
                }
            }
            let image = match comp {
                1 => {
                    // basic: (data size, zero size) pairs
                    let n = (be32(&d, fmt) as usize - 8) / 8;
                    let mut img = Vec::with_capacity(image_size);
                    let mut p = 0usize;
                    for i in 0..n {
                        let (ds, zs) = (be32(&d, fmt + 8 + i * 8) as usize, be32(&d, fmt + 12 + i * 8) as usize);
                        img.extend_from_slice(&data[p..p + ds]);
                        img.resize(img.len() + zs, 0);
                        p += ds;
                    }
                    img
                }
                2 => {
                    let window = be32(&d, fmt + 8);
                    let mut block_size = be32(&d, fmt + 12) as usize;
                    println!("lzx window {:#x}, first block {:#x}", window, block_size);
                    let ws = match window {
                        0x8000 => lzxd::WindowSize::KB32,
                        0x10000 => lzxd::WindowSize::KB64,
                        0x20000 => lzxd::WindowSize::KB128,
                        0x40000 => lzxd::WindowSize::KB256,
                        0x80000 => lzxd::WindowSize::KB512,
                        0x100000 => lzxd::WindowSize::MB1,
                        0x200000 => lzxd::WindowSize::MB2,
                        _ => anyhow::bail!("window size {:#x}", window),
                    };
                    let mut lzx = lzxd::Lzxd::new(ws);
                    let mut img: Vec<u8> = Vec::with_capacity(image_size);
                    let mut p = 0usize;
                    while block_size != 0 && p + block_size <= data.len() {
                        let blk = &data[p..p + block_size];
                        let next = be32(blk, 0) as usize;
                        let mut q = 24;
                        while q + 2 <= blk.len() {
                            let cs = be16(blk, q) as usize;
                            q += 2;
                            if cs == 0 {
                                break;
                            }
                            let want = (image_size - img.len()).min(0x8000);
                            if want == 0 {
                                break;
                            }
                            let dec = lzx.decompress_next(&blk[q..q + cs], want).map_err(|e| anyhow::anyhow!("lzx at {:#x}: {:?}", img.len(), e))?;
                            img.extend_from_slice(dec);
                            q += cs;
                        }
                        p += block_size;
                        block_size = next;
                    }
                    img
                }
                _ => data,
            };
            println!("image {} bytes, starts {:02x?}", image.len(), &image[..4.min(image.len())]);
            std::fs::write(out, &image)?;
            Ok(())
        }
        "brute" => {
            // brute <dir of 0x-named files> <max letters> <name a> <name b> [alphabet]
            // finds suffixes X such that hash(a + X) and hash(b + X) are both files of the directory
            let dir = args.get(2).context("usage")?;
            let max: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(6);
            let a = args.get(4).context("usage")?.to_ascii_lowercase();
            let b = args.get(5).context("usage")?.to_ascii_lowercase();
            let alpha: Vec<u8> = args.get(6).map(|s| s.as_bytes().to_vec()).unwrap_or_else(|| b"abcdefghijklmnopqrstuvwxyz".to_vec());
            let mut set = std::collections::HashSet::new();
            for e in std::fs::read_dir(dir)?.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if let Some(h) = n.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()) {
                    set.insert(h);
                }
            }
            fn step(mut h: u32, c: u8) -> u32 {
                h = h.wrapping_add(c as u32);
                h = h.wrapping_add(h << 10);
                h ^= h >> 6;
                h
            }
            fn fin(mut h: u32) -> u32 {
                h = h.wrapping_add(h << 3);
                h ^= h >> 11;
                h.wrapping_add(h << 15)
            }
            let pre = |s: &str| s.bytes().fold(0u32, step);
            fn go(ha: u32, hb: u32, depth: usize, max: usize, alpha: &[u8], set: &std::collections::HashSet<u32>, cur: &mut Vec<u8>, tails: &[&str]) {
                if depth > 0 {
                    for t in tails {
                        let (fa, fb) = (fin(t.bytes().fold(ha, step)), fin(t.bytes().fold(hb, step)));
                        if set.contains(&fa) && set.contains(&fb) {
                            println!("{}{}  -> 0x{:08x} 0x{:08x}", String::from_utf8_lossy(cur), t, fa, fb);
                        }
                    }
                }
                if depth == max {
                    return;
                }
                for &c in alpha {
                    cur.push(c);
                    go(step(ha, c), step(hb, c), depth + 1, max, alpha, set, cur, tails);
                    cur.pop();
                }
            }
            let tails: Vec<&str> = args.get(7).map(|s| s.split(',').collect()).unwrap_or_else(|| vec![""]);
            let mut cur = Vec::new();
            if args.get(8).map(|s| s.as_str()) == Some("pre") {
                // the unknown part comes first: X + a + tail
                fn gop(h: u32, depth: usize, max: usize, alpha: &[u8], set: &std::collections::HashSet<u32>, cur: &mut Vec<u8>, a: &str, b: &str, tails: &[&str]) {
                    if depth > 0 {
                        let (ha, hb) = (a.bytes().fold(h, step), b.bytes().fold(h, step));
                        for t in tails {
                            let (fa, fb) = (fin(t.bytes().fold(ha, step)), fin(t.bytes().fold(hb, step)));
                            if set.contains(&fa) && set.contains(&fb) {
                                println!("{}<car>{}  -> 0x{:08x} 0x{:08x}", String::from_utf8_lossy(cur), t, fa, fb);
                            }
                        }
                    }
                    if depth == max {
                        return;
                    }
                    for &c in alpha {
                        cur.push(c);
                        gop(step(h, c), depth + 1, max, alpha, set, cur, a, b, tails);
                        cur.pop();
                    }
                }
                gop(0, 0, max, &alpha, &set, &mut cur, &a, &b, &tails);
                println!("done");
                return Ok(());
            }
            go(pre(&a), pre(&b), 0, max, &alpha, &set, &mut cur, &tails);
            println!("done");
            Ok(())
        }
        "ground" => {
            // census of what the city's ground is made of: up-facing area per shader and texture
            use mcla_formats::rage::sector::Sector;
            let dir = args.get(2).context("usage: ground <city sector dir>")?;
            let mut area: std::collections::HashMap<(String, String), f64> = std::collections::HashMap::new();
            let mut n = 0;
            for e in std::fs::read_dir(dir)?.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("r1") {
                    continue;
                }
                let Ok(res) = load_res(&p.to_string_lossy()) else { continue };
                let Ok(sec) = Sector::parse(&res) else { continue };
                n += 1;
                for b in &sec.batches {
                    let sh = &sec.shaders[b.shader];
                    let tex = sh.diffuse().unwrap_or("").to_string();
                    let m = &b.mesh;
                    let mut a = 0.0f64;
                    for t in m.indices.chunks_exact(3) {
                        let (Some(p0), Some(p1), Some(p2)) = (m.positions.get(t[0] as usize), m.positions.get(t[1] as usize), m.positions.get(t[2] as usize)) else { continue };
                        let c = (*p1 - *p0).cross(*p2 - *p0);
                        if c.y.abs() > c.length() * 0.8 {
                            a += (c.length() * 0.5) as f64;
                        }
                    }
                    if a > 0.0 {
                        *area.entry((sh.name.clone(), tex)).or_default() += a;
                    }
                }
            }
            let mut v: Vec<_> = area.into_iter().collect();
            v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            println!("{} sectors", n);
            for ((sh, tex), a) in v.iter().take(args.get(3).and_then(|x| x.parse().ok()).unwrap_or(120)) {
                println!("{:>12.0} m2  {:28} {}", a, sh, tex);
            }
            Ok(())
        }
        "strings" => {
            // every string table of a `ui` folder as key<TAB>text
            let dir = args.get(2).context("usage: strings <ui dir> <out.tsv>")?;
            let out = args.get(3).context("usage: strings <ui dir> <out.tsv>")?;
            let mut lines = Vec::new();
            let mut files: Vec<_> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
            files.sort();
            for f in files {
                let Ok(d) = std::fs::read(&f) else { continue };
                let Ok(t) = mcla_formats::strtbl::StringTable::parse(&d) else { continue };
                for k in &t.keys {
                    if let Some(v) = t.get(k) {
                        lines.push(format!("{}\t{}\t{}", f.file_name().unwrap().to_string_lossy(), k, v.replace('\n', "\\n")));
                    }
                }
            }
            println!("{} strings", lines.len());
            std::fs::write(out, lines.join("\n"))?;
            Ok(())
        }
        "hash" => {
            for s in &args[2..] {
                println!("{:08x} {}", mcla_formats::hash::joaat(s), s);
            }
            Ok(())
        }
        other => bail!("unknown command {}", other),
    }
}

fn load_names(path: Option<&String>) -> Result<Names> {
    let mut n = Names::new();
    if let Some(p) = path {
        let t = std::fs::read_to_string(p).with_context(|| format!("read {}", p))?;
        n.load_list(&t);
    }
    Ok(n)
}

fn cmd_ls(a: &[String]) -> Result<()> {
    let iso = Xiso::open(Path::new(&a[0]))?;
    println!("game partition base: {:#x}", iso.base);
    for e in &iso.entries {
        println!("{:>14} {:>12} {}", e.offset, e.size, e.path);
    }
    Ok(())
}

fn open_rpf(iso_path: &str, archive: &str) -> Result<Rpf3<mcla_formats::SubRange<memmap2::Mmap>>> {
    let iso = Xiso::open(Path::new(iso_path))?;
    let f = iso.file(archive)?;
    // The key is derived from the disc's own default.xex; nothing is stored.
    let key = mcla_formats::keys::derive(&iso)?;
    Rpf3::open(Arc::new(f), &key)
}

/// `plainscripts <iso> <data dir>`: rewrite the encrypted script objects of an already extracted data folder in
/// the plain form (the same result as a fresh `extract`), in place.
fn cmd_plainscripts(a: &[String]) -> Result<()> {
    let iso = Xiso::open(Path::new(&a[0]))?;
    let key = mcla_formats::keys::derive(&iso)?;
    fn walk(dir: &Path, key: &[u8; 32], done: &mut usize, skipped: &mut usize) -> Result<()> {
        for e in std::fs::read_dir(dir)?.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, key, done, skipped)?;
            } else {
                // by content, not name: most scripts have no recovered name (hash names, or a wrong guessed extension)
                let d = std::fs::read(&p)?;
                if mcla_formats::sco::is_disc_form(&d) {
                    std::fs::write(&p, mcla_formats::sco::to_plain(&d, key).with_context(|| p.display().to_string())?)?;
                    *done += 1;
                } else {
                    *skipped += 1;
                }
            }
        }
        Ok(())
    }
    let (mut done, mut skipped) = (0, 0);
    walk(Path::new(&a[1]), &key, &mut done, &mut skipped)?;
    eprintln!("{} scripts converted, {} left as they were", done, skipped);
    Ok(())
}

fn cmd_rpf(a: &[String]) -> Result<()> {
    let rpf = open_rpf(&a[0], &a[1])?;
    let names = load_names(a.get(2))?;
    let mut known = 0usize;
    for (i, e) in rpf.entries.iter().enumerate() {
        if names.try_get(e.hash).is_some() {
            known += 1;
        }
        let p = rpf.path_of(i, &names);
        match e.kind {
            Kind::Dir { count, .. } => println!("D {:>10} {}/", count, p),
            Kind::File { size, compressed, .. } => {
                println!("F {:>10} {} {}", size, p, if compressed.is_some() { "(deflate)" } else { "" })
            }
            Kind::Resource { size, rtype, flags, .. } => println!(
                "R {:>10} {} type={} flags={:#010x} vsize={:#x}",
                size,
                p,
                rtype,
                flags,
                mcla_formats::rsc::virtual_size_from_flags(flags)
            ),
        }
    }
    eprintln!("{} entries, {} with known names", rpf.entries.len(), known);
    Ok(())
}

/// Progress sink for the command line: a line for every finished stage and checklist entry, a percentage every few seconds.
fn cli_ctx() -> mcla_install::Ctx {
    use mcla_install::{CheckStatus, Unit};
    let last = std::sync::Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(10));
    let sink: mcla_install::progress::Sink = Arc::new(move |e: &mcla_install::Event| {
        if let Some(c) = &e.check {
            let tag = match c.status {
                CheckStatus::Pass => "ok  ",
                CheckStatus::Warn => "WARN",
                CheckStatus::Fail => "FAIL",
                CheckStatus::Pending => "... ",
            };
            eprintln!("  [{}] {} {}: {}", e.stage.id(), tag, c.id, c.text);
        } else if e.finished {
            eprintln!("[{}] {}", e.stage.id(), if e.message.is_empty() { "done".to_string() } else { e.message.clone() });
        } else if e.total == 0 {
            eprintln!("[{}] {}", e.stage.id(), e.message);
        } else if let Ok(mut l) = last.lock() {
            if l.elapsed() >= std::time::Duration::from_secs(5) {
                *l = std::time::Instant::now();
                let (d, t) = (e.done, e.total);
                match e.unit {
                    Unit::Bytes => eprintln!("[{}] {} / {} MB ({}%)", e.stage.id(), d / 1_000_000, t / 1_000_000, d * 100 / t.max(1)),
                    Unit::Items => eprintln!("[{}] {} / {} ({}%)", e.stage.id(), d, t, d * 100 / t.max(1)),
                }
            }
        }
    });
    mcla_install::Ctx::new(sink, mcla_install::Cancel::new())
}

fn cmd_extract(a: &[String]) -> Result<()> {
    let raw = a.iter().any(|s| s == "--raw");
    let a: Vec<&String> = a.iter().filter(|s| !s.starts_with("--")).collect();
    let rpf = open_rpf(a[0], a[1])?;
    let out = PathBuf::from(a[2]);
    let names = load_names(a.get(3).copied())?;
    // the logic lives in mcla-install (the launcher runs the same code); finished files are skipped, so a rerun resumes
    let opts = mcla_install::extract::ExtractOptions { raw };
    let rep = mcla_install::extract::extract_archive(&rpf, &out, &names, opts, mcla_install::Stage::Extract, &cli_ctx())?;
    for (p, why) in &rep.failed {
        eprintln!("FAIL {}: {}", p, why);
    }
    eprintln!("done, {} failed ({} written, {} already there)", rep.failed.len(), rep.written, rep.skipped);
    Ok(())
}

/// `ffmpeg <folder> [--manifest <file>] [--no-download] [--path <ffmpeg>]`: find ffmpeg for the install folder, or download the pinned build into `<folder>/tools/ffmpeg`.
fn cmd_ffmpeg(a: &[String]) -> Result<()> {
    let (mut pos, mut manifest, mut download, mut path) = (Vec::new(), None, true, None);
    let mut it = a.iter();
    while let Some(s) = it.next() {
        match s.as_str() {
            "--manifest" => manifest = Some(PathBuf::from(it.next().context("--manifest needs a file")?)),
            "--path" => path = Some(PathBuf::from(it.next().context("--path needs a file")?)),
            "--no-download" => download = false,
            f if f.starts_with("--") => bail!("unknown option {}", f),
            _ => pos.push(s.clone()),
        }
    }
    if pos.len() != 1 {
        bail!("usage: mcla-tool ffmpeg <folder> [--manifest <file>] [--path <ffmpeg>] [--no-download]");
    }
    let m = match manifest {
        Some(f) => mcla_install::Manifest::load(&f).map_err(|e| anyhow::anyhow!("{e}"))?,
        None => mcla_install::Manifest::embedded(),
    };
    let ff = mcla_install::ffmpeg::ensure(Path::new(&pos[0]), path.as_deref(), download, &m, &cli_ctx()).map_err(|e| anyhow::anyhow!("{e}"))?;
    eprintln!("ffmpeg {} ({:?}) at {}", ff.version, ff.source, ff.path.display());
    Ok(())
}

/// `install <iso> <folder> [--ffmpeg <path>] [--no-download] [--no-checksum] [--jobs N] [--manifest <file>] [--repair]`
fn cmd_install(a: &[String]) -> Result<()> {
    let mut pos = Vec::new();
    let (mut ffmpeg, mut manifest, mut jobs) = (None, None, 0usize);
    let (mut download, mut checksum, mut repair) = (true, true, false);
    let mut it = a.iter();
    while let Some(s) = it.next() {
        match s.as_str() {
            "--ffmpeg" => ffmpeg = Some(PathBuf::from(it.next().context("--ffmpeg needs a path")?)),
            "--manifest" => manifest = Some(PathBuf::from(it.next().context("--manifest needs a file")?)),
            "--jobs" => jobs = it.next().context("--jobs needs a number")?.parse()?,
            "--no-download" => download = false,
            "--no-checksum" => checksum = false,
            "--repair" => repair = true,
            f if f.starts_with("--") => bail!("unknown option {}", f),
            _ => pos.push(s.clone()),
        }
    }
    if pos.len() != 2 {
        bail!("usage: mcla-tool install <iso> <folder> [--ffmpeg <path>] [--no-download] [--no-checksum] [--jobs N] [--manifest <file>] [--repair]");
    }
    let mut opts = mcla_install::InstallOptions::new(&pos[0], &pos[1]);
    opts.ffmpeg_path = ffmpeg;
    opts.allow_download = download;
    opts.full_checksum = checksum;
    opts.jobs = jobs;
    if let Some(m) = manifest {
        opts.manifest = mcla_install::Manifest::load(&m).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let ctx = cli_ctx();
    let t0 = std::time::Instant::now();
    let r = if repair { mcla_install::repair(&opts, &ctx).map_err(|e| anyhow::anyhow!("{e}"))? } else { Some(mcla_install::install(&opts, &ctx).map_err(|e| anyhow::anyhow!("{e}"))?) };
    match r {
        None => eprintln!("everything is in place, nothing to repair"),
        Some(o) => {
            for w in &o.warnings {
                eprintln!("warning: {}", w);
            }
            for p in o.audio_problems.iter().take(20) {
                eprintln!("audio problem: {}", p);
            }
            eprintln!(
                "installed: {} files, {:.2} GB, disc image {} ({} written, {} already there) in {:.1} s",
                o.local.files_count,
                o.local.files_bytes as f64 / 1e9,
                o.local.iso.dump,
                o.extract_written,
                o.extract_skipped,
                t0.elapsed().as_secs_f64()
            );
            eprintln!("run the game with: mcla --data \"{}\"", Path::new(&pos[1]).join("data").join("cache").display());
        }
    }
    Ok(())
}

/// `shaders-wgsl <data cache folder> [--iso <iso>]`: translate the game's sky / road / sea shader programs into `<folder>/shaders_wgsl/`
/// (the install stage "Translate shaders", for an existing extraction). The disc image is only needed when `<folder>/shaders/exe/exe_82801b00`
/// (the cloud pass cut out of default.xex) is not there yet. Prints the SHA-256 of every file.
fn cmd_shaders_wgsl(a: &[String]) -> Result<()> {
    let (mut pos, mut iso) = (Vec::new(), None);
    let mut it = a.iter();
    while let Some(s) = it.next() {
        match s.as_str() {
            "--iso" => iso = Some(PathBuf::from(it.next().context("--iso needs a file")?)),
            f if f.starts_with("--") => bail!("unknown option {}", f),
            _ => pos.push(s.clone()),
        }
    }
    if pos.len() != 1 {
        bail!("usage: mcla-tool shaders-wgsl <data cache folder> [--iso <iso>]");
    }
    let xiso = match &iso {
        Some(p) => Some(Xiso::open(p)?),
        None => None,
    };
    let cache = Path::new(&pos[0]);
    let t0 = std::time::Instant::now();
    let rep = mcla_install::shaders::run(cache, xiso.as_ref(), &cli_ctx()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let hashes = mcla_install::shaders::hashes(cache).map_err(|e| anyhow::anyhow!("{e}"))?;
    for f in &rep.files {
        println!("{}  {:>7} bytes  {}  {}", hashes.get(f.name).map(|s| s.as_str()).unwrap_or("?"), f.bytes, if f.written { "written  " } else { "unchanged" }, f.name);
    }
    eprintln!("{} files, {} written{} in {:.2} s", rep.files.len(), rep.written(), if rep.effect_cut { ", the executable's effect was cut out of the disc" } else { "" }, t0.elapsed().as_secs_f64());
    Ok(())
}

fn cmd_xmemtest(a: &[String]) -> Result<()> {
    use lzxd::{Lzxd, WindowSize};
    let rpf = open_rpf(&a[0], &a[1])?;
    let want = u32::from_str_radix(a[2].trim_start_matches("0x"), 16)?;
    let idx = rpf
        .entries
        .iter()
        .position(|e| e.hash == want && matches!(e.kind, Kind::Resource { .. }))
        .context("resource hash not found")?;
    let blob = rpf.read(idx)?;
    println!("entry {} kind {:?}", idx, rpf.entries[idx].kind);
    let stream = &blob[12..];
    let total = u32::from_be_bytes(stream[4..8].try_into().unwrap()) as usize;
    // collect chunks
    let mut chunks = Vec::new();
    let mut p = 8usize;
    let end = 8 + total;
    while p < end {
        let (u, c, hl) = if stream[p] == 0xFF {
            (
                u16::from_be_bytes([stream[p + 1], stream[p + 2]]) as usize,
                u16::from_be_bytes([stream[p + 3], stream[p + 4]]) as usize,
                5,
            )
        } else {
            (0x8000, u16::from_be_bytes([stream[p], stream[p + 1]]) as usize, 2)
        };
        p += hl;
        chunks.push((u, &stream[p..p + c]));
        p += c;
    }
    println!("{} chunks, first sizes: {:?}", chunks.len(), chunks.iter().take(4).map(|(u, c)| (*u, c.len())).collect::<Vec<_>>());
    let windows = [
        ("32K", WindowSize::KB32),
        ("64K", WindowSize::KB64),
        ("128K", WindowSize::KB128),
        ("256K", WindowSize::KB256),
        ("512K", WindowSize::KB512),
        ("1M", WindowSize::MB1),
        ("2M", WindowSize::MB2),
    ];
    for (name, w) in windows {
        for reset in [false, true] {
            let mut lzx = Lzxd::new(w);
            let mut out = Vec::new();
            let mut err = None;
            for (ci, (u, c)) in chunks.iter().enumerate() {
                if reset {
                    lzx = Lzxd::new(w);
                }
                match lzx.decompress_next(c, *u) {
                    Ok(d) => out.extend_from_slice(d),
                    Err(e) => {
                        err = Some(format!("chunk {}: {:?}", ci, e));
                        break;
                    }
                }
            }
            let head: Vec<String> = out.iter().take(16).map(|b| format!("{:02x}", b)).collect();
            println!("window {:>4} reset={} -> out {} bytes, err {:?}, head {}", name, reset, out.len(), err, head.join(" "));
        }
    }
    Ok(())
}

fn load_res(path: &str) -> Result<Resource> {
    let d = std::fs::read(path).with_context(|| format!("read {}", path))?;
    if d.len() > 16 && &d[0..4] == b"MCLR" {
        // extracted form: MCLR, type, flags, vsize, data
        let rtype = u32::from_le_bytes(d[4..8].try_into().unwrap());
        let flags = u32::from_le_bytes(d[8..12].try_into().unwrap());
        let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
        let data = d[16..].to_vec();
        return Ok(Resource { rtype, flags, virtual_size: vs.min(data.len()), physical_size: data.len().saturating_sub(vs), data });
    }
    Resource::parse(&d)
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' }).collect()
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let f = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header()?;
    wr.write_image_data(rgba)?;
    Ok(())
}

/// mcla-tool texdict <file.r1> <outdir>   – dump every texture of a texture dictionary as PNG
fn cmd_texdict(a: &[String]) -> Result<()> {
    use mcla_formats::rage::{texture::Texture, Dictionary, Rd};
    let res = load_res(&a[0])?;
    let out = PathBuf::from(&a[1]);
    std::fs::create_dir_all(&out)?;
    let rd = Rd::new(&res);
    // optional third argument: offset of the dictionary inside the resource (hex)
    // ("@8": the dictionary the pointer at that offset leads to - a city sector's textures)
    let at = match a.get(2) {
        Some(x) if x.starts_with('@') => usize::from_str_radix(&x[1..], 16).ok().and_then(|o| rd.ptr(o)).unwrap_or(0),
        x => x.and_then(|x| usize::from_str_radix(x.trim_start_matches("0x"), 16).ok()).unwrap_or(0),
    };
    let d = Dictionary::parse(&rd, at)?;
    println!("{} textures", d.len());
    for (h, v) in d.hashes.iter().zip(d.values.iter()) {
        let Some(o) = v else { continue };
        match Texture::parse(&rd, *o) {
            Ok(t) => {
                let rgba = t.to_rgba8();
                let name = if t.name.is_empty() { format!("{:08x}", h) } else { t.name.clone() };
                println!("{:08x} {}x{} {:?} levels={} {} means={:?}", h, t.width, t.height, t.format, t.levels.len(), name, t.level_means());
                write_png(&out.join(format!("{}.png", sanitize(&name))), t.width, t.height, &rgba)?;
            }
            Err(e) => println!("{:08x} ERROR {:#}", h, e),
        }
    }
    Ok(())
}

/// mcla-tool navpath <navsys.r66> x1 y1 z1 x2 y2 z2  – graph statistics and a path query
fn cmd_navpath(a: &[String]) -> Result<()> {
    use mcla_formats::rage::navsys::NavSys;
    let res = load_res(&a[0])?;
    let nav = NavSys::parse(&res)?;
    let (lanes, links, with_next) = nav.stats();
    println!("lanes {} links {} with successor {}", lanes, links, with_next);
    let f = |i: usize| a[i].parse::<f32>().unwrap_or(0.0);
    let p1 = glam::Vec3::new(f(1), f(2), f(3));
    let p2 = glam::Vec3::new(f(4), f(5), f(6));
    let (l1, _, d1) = nav.nearest_link(p1).unwrap();
    let (l2, _, d2) = nav.nearest_link(p2).unwrap();
    println!("from link {} (d {:.1}) to link {} (d {:.1}); reachable from start: {}", l1, d1, l2, d2, nav.reachable(l1));
    match nav.route_points(p1, None, p2) {
        Some(p) => {
            println!("route: {} points, {:.0} m", p.len(), p.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>());
            // (a seventh figure's place: a file for the route as "L x y z" lines - the
            // format of the game's race line dumps, for scripted driving)
            if let Some(out) = a.get(7) {
                let text: String = p.iter().map(|v| format!("L {:.2} {:.2} {:.2}
", v.x, v.y, v.z)).collect();
                std::fs::write(out, text)?;
            }
        }
        None => println!("route: none"),
    }
    Ok(())
}

/// mcla-tool props <props.r131> [foliage.r134]  – list prop models and foliage placements
fn cmd_props(a: &[String]) -> Result<()> {
    use mcla_formats::rage::props;
    let res = load_res(&a[0])?;
    let ps = props::parse_props(&res);
    println!("{} props", ps.len());
    for p in &ps {
        let tris: usize = p.meshes.iter().map(|m| m.0.indices.len() / 3).sum();
        let mut sh: Vec<String> = Vec::new();
        for (_, si) in &p.meshes {
            if let Some(s) = p.shaders.get(*si as usize) {
                let t = s.textures().next().map(|x| x.1.to_string()).unwrap_or_default();
                let e = format!("{}:{}", s.name, t);
                if !sh.contains(&e) {
                    sh.push(e);
                }
            }
        }
        println!("{:<52} tris {:>5} size ({:.1},{:.1},{:.1}) min.y {:.1}  {}", p.name, tris, p.max.x - p.min.x, p.max.y - p.min.y, p.max.z - p.min.z, p.min.y, sh.join(" | "));
        // MCLA_PROP_MESH=<name substring>: dump the meshes of matching props
        if let Ok(f) = std::env::var("MCLA_PROP_MESH") {
            if p.name.contains(f.as_str()) {
                for (k, (m, si)) in p.meshes.iter().enumerate() {
                    let sn = p.shaders.get(*si as usize).map(|s| s.name.clone()).unwrap_or_default();
                    println!("  mesh {} shader {} verts {} uv1 {} colors {}", k, sn, m.positions.len(), m.uv1.len(), m.colors.len());
                    if std::env::var("MCLA_PROP_VERTS").is_ok() {
                        let (mn, mx) = m.bbox();
                        println!("  bbox {:?} .. {:?}", mn, mx);
                        // quads: centre and size of every 4 vertices
                        for q in m.positions.chunks(4).take(40) {
                            let c = q.iter().fold(glam::Vec3::ZERO, |a, b| a + *b) / q.len() as f32;
                            let e = q.iter().fold(glam::Vec3::ZERO, |a, b| a.max((*b - c).abs()));
                            println!("    quad centre ({:.2},{:.2},{:.2}) half ({:.2},{:.2},{:.2})", c.x, c.y, c.z, e.x, e.y, e.z);
                        }
                    }
                    if sn.contains("TrafficSignal") {
                        for i in 0..m.positions.len() {
                            let q = m.positions[i];
                            let n = m.normals.get(i).copied().unwrap_or_default();
                            let u0 = m.uv0.get(i).copied().unwrap_or_default();
                            let u1 = m.uv1.get(i).copied().unwrap_or_default();
                            let c = m.colors.get(i).copied().unwrap_or_default();
                            println!("    p ({:6.2},{:6.2},{:6.2}) n ({:5.2},{:5.2},{:5.2}) uv0 ({:.3},{:.3}) uv1 ({:.2},{:.2}) col {:?}", q.x, q.y, q.z, n.x, n.y, n.z, u0.x, u0.y, u1.x, u1.y, c);
                        }
                    }
                }
            }
        }
    }
    if a.len() > 1 {
        let res = load_res(&a[1])?;
        let fs = props::parse_foliage(&res);
        let total: usize = fs.iter().map(|f| f.instances.len()).sum();
        println!("{} foliage types, {} instances", fs.len(), total);
        for f in &fs {
            let i0 = f.instances[0];
            let smin = f.instances.iter().map(|i| i.width).fold(f32::INFINITY, f32::min);
            let smax = f.instances.iter().map(|i| i.width).fold(0.0f32, f32::max);
            let (hmin, hmax) = f.instances.iter().map(|i| i.height()).fold((f32::INFINITY, 0.0f32), |a, h| (a.0.min(h), a.1.max(h)));
            println!("{:<52} n {:>5} width {:.2}..{:.2} height {:.2}..{:.2} turn steps {} first at {:?}", f.name, f.instances.len(), smin, smax, hmin, hmax, f.rot_steps, i0.pos);
        }
    }
    Ok(())
}

/// mcla-tool uitex <scene.r27> <outdir>   – dump the images embedded in a UI scene as PNG (+ its strings)
fn cmd_uitex(a: &[String]) -> Result<()> {
    use mcla_formats::rage::ui;
    let res = load_res(&a[0])?;
    let out = PathBuf::from(&a[1]);
    std::fs::create_dir_all(&out)?;
    let texs = ui::ui_textures(&res);
    println!("{} textures", texs.len());
    for t in &texs {
        let rgba = t.to_rgba8();
        println!("{}x{} {:?} levels={} {}", t.width, t.height, t.format, t.levels.len(), t.name);
        write_png(&out.join(format!("{}.png", sanitize(&t.name))), t.width, t.height, &rgba)?;
    }
    let strings = ui::ui_strings(&res);
    std::fs::write(out.join("strings.txt"), strings.join("
"))?;
    println!("{} strings -> strings.txt", strings.len());
    Ok(())
}

/// mcla-tool sector <sector.r1> <outdir> [shared texture dict .r1 ...]
/// Exports OBJ + MTL + PNG textures for visual validation.
fn cmd_sector(a: &[String]) -> Result<()> {
    use mcla_formats::rage::sector::Sector;
    use mcla_formats::rage::{texture::Texture, Dictionary, Rd};
    use std::io::Write;
    let res = load_res(&a[0])?;
    let out = PathBuf::from(&a[1]);
    std::fs::create_dir_all(&out)?;
    let sec = Sector::parse(&res)?;
    println!("textures={} shaders={} batches={} errors={}", sec.textures.len(), sec.shaders.len(), sec.batches.len(), sec.errors.len());
    for e in sec.errors.iter().filter(|e| !e.starts_with("texture ")).take(10).chain(sec.errors.iter().filter(|e| e.starts_with("texture ")).take(3)) {
        println!("  err: {}", e);
    }
    // shared dictionaries
    let mut shared: Vec<(Resource, Dictionary)> = Vec::new();
    for p in &a[2..] {
        let r = load_res(p)?;
        let d = Dictionary::parse(&Rd::new(&r), 0)?;
        println!("shared dict {} : {} textures", p, d.len());
        shared.push((r, d));
    }
    let find_tex = |name: &str| -> Option<Texture> {
        let h = mcla_formats::hash::joaat(name);
        if let Some(t) = sec.textures.get(&h) {
            return Some(t.clone());
        }
        for (r, d) in &shared {
            if let Some(o) = d.get(h) {
                if let Ok(t) = Texture::parse(&Rd::new(r), o) {
                    return Some(t);
                }
            }
        }
        None
    };
    let mut mtl = std::fs::File::create(out.join("sector.mtl"))?;
    let mut obj = std::fs::File::create(out.join("sector.obj"))?;
    writeln!(obj, "mtllib sector.mtl")?;
    let mut written = std::collections::HashSet::new();
    let mut missing = 0;
    for (i, s) in sec.shaders.iter().enumerate() {
        writeln!(mtl, "newmtl s{}", i)?;
        if let Some(dn) = s.diffuse() {
            let fname = format!("{}.png", sanitize(&dn.to_ascii_lowercase()));
            if !written.contains(&fname) {
                match find_tex(dn) {
                    Some(t) => {
                        write_png(&out.join(&fname), t.width, t.height, &t.to_rgba8())?;
                        written.insert(fname.clone());
                    }
                    None => {
                        missing += 1;
                    }
                }
            }
            writeln!(mtl, "map_Kd {}", fname)?;
        }
        writeln!(mtl, "Kd 0.8 0.8 0.8")?;
    }
    let mut base = 1usize;
    let mut tris = 0usize;
    for (bi, b) in sec.batches.iter().enumerate() {
        let m = &b.mesh;
        writeln!(obj, "o batch{}_{}", bi, sec.shaders[b.shader].name)?;
        writeln!(obj, "usemtl s{}", b.shader)?;
        for p in &m.positions {
            writeln!(obj, "v {} {} {}", p.x, p.y, p.z)?;
        }
        for t in &m.uv0 {
            writeln!(obj, "vt {} {}", t.x, 1.0 - t.y)?;
        }
        for n in &m.normals {
            writeln!(obj, "vn {} {} {}", n.x, n.y, n.z)?;
        }
        let has_uv = !m.uv0.is_empty();
        let has_n = !m.normals.is_empty();
        for f in m.indices.chunks_exact(3) {
            let v = |k: u32| {
                let i = base + k as usize;
                match (has_uv, has_n) {
                    (true, true) => format!("{}/{}/{}", i, i, i),
                    (true, false) => format!("{}/{}", i, i),
                    (false, true) => format!("{}//{}", i, i),
                    _ => format!("{}", i),
                }
            };
            writeln!(obj, "f {} {} {}", v(f[0]), v(f[1]), v(f[2]))?;
            tris += 1;
        }
        base += m.positions.len();
    }
    println!("wrote {} triangles, {} textures, {} missing textures", tris, written.len(), missing);
    // vertex colour statistics per batch (raw byte order as stored)
    for (bi, b) in sec.batches.iter().enumerate().take(40) {
        let m = &b.mesh;
        let avg = |v: &Vec<[u8; 4]>| -> [u32; 4] {
            if v.is_empty() { return [0; 4]; }
            let mut s = [0u64; 4];
            for c in v { for k in 0..4 { s[k] += c[k] as u64; } }
            [(s[0] / v.len() as u64) as u32, (s[1] / v.len() as u64) as u32, (s[2] / v.len() as u64) as u32, (s[3] / v.len() as u64) as u32]
        };
        let (umin, umax) = m.uv0.iter().fold((glam::Vec2::splat(1e9), glam::Vec2::splat(-1e9)), |(a, c), u| (a.min(*u), c.max(*u)));
        println!("  batch {:3} {:<24} flags={} verts={:6} c0={:?} c1={:?} uv0=[{:.1},{:.1}]..[{:.1},{:.1}] stride={}", bi, sec.shaders[b.shader].name, b.flags, m.positions.len(), avg(&m.colors), avg(&m.colors1), umin.x, umin.y, umax.x, umax.y, m.raw_stride);
    }
    // print shader summary
    for s in sec.shaders.iter() {
        println!("  shader {} params: {:?}", s.name, s.params.iter().map(|(h, p)| match p {
            mcla_formats::rage::drawable::ShaderParam::Texture { name } => format!("{:08x}=tex:{}", h, name),
            mcla_formats::rage::drawable::ShaderParam::Vector(v) => format!("{:08x}=vec[{}]", h, v.len()),
            _ => format!("{:08x}=?", h),
        }).collect::<Vec<_>>());
    }
    Ok(())
}

/// mcla-tool citymap <city/sc dir> <out.png> – parse every sector, print extents, draw a top-down height map
fn cmd_citymap(a: &[String]) -> Result<()> {
    use mcla_formats::rage::sector::Sector;
    let dir = PathBuf::from(&a[0]);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |x| x == "r1"))
        .collect();
    files.sort();
    let t0 = std::time::Instant::now();
    let results: Vec<(PathBuf, Option<(glam::Vec3, glam::Vec3, usize, Vec<[glam::Vec3; 3]>)>)> = files
        .par_iter()
        .map(|p| {
            let res = match load_res(p.to_str().unwrap()) {
                Ok(r) => r,
                Err(_) => return (p.clone(), None),
            };
            if mcla_formats::rage::Rd::new(&res).vtable(0) != mcla_formats::rage::vt::CITY_SECTOR {
                return (p.clone(), None);
            }
            let sec = match Sector::parse(&res) {
                Ok(s) => s,
                Err(_) => return (p.clone(), None),
            };
            let mut mn = glam::Vec3::splat(f32::INFINITY);
            let mut mx = glam::Vec3::splat(f32::NEG_INFINITY);
            let mut tris = Vec::new();
            let mut ntri = 0;
            for b in &sec.batches {
                let (a, c) = b.mesh.bbox();
                mn = mn.min(a);
                mx = mx.max(c);
                ntri += b.mesh.indices.len() / 3;
                // keep a decimated set for drawing
                for (k, t) in b.mesh.indices.chunks_exact(3).enumerate() {
                    if k % 1 == 0 {
                        let p = &b.mesh.positions;
                        if (t[0] as usize) < p.len() && (t[1] as usize) < p.len() && (t[2] as usize) < p.len() {
                            tris.push([p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]]);
                        }
                    }
                }
            }
            (p.clone(), Some((mn, mx, ntri, tris)))
        })
        .collect();
    let mut gmn = glam::Vec3::splat(f32::INFINITY);
    let mut gmx = glam::Vec3::splat(f32::NEG_INFINITY);
    let mut total = 0usize;
    let mut nsec = 0usize;
    for (p, r) in &results {
        if let Some((mn, mx, n, _)) = r {
            gmn = gmn.min(*mn);
            gmx = gmx.max(*mx);
            total += n;
            nsec += 1;
            println!("{:<40} tris={:>7} min=({:8.1},{:7.1},{:8.1}) max=({:8.1},{:7.1},{:8.1})", p.file_name().unwrap().to_string_lossy(), n, mn.x, mn.y, mn.z, mx.x, mx.y, mx.z);
        }
    }
    println!("{} sectors, {} triangles, world min {:?} max {:?}, parsed in {:.1?}", nsec, total, gmn, gmx, t0.elapsed());
    // draw
    let size = 4096usize;
    let ext = (gmx - gmn).max_element();
    let mut img = vec![0u8; size * size * 4];
    let mut zbuf = vec![f32::NEG_INFINITY; size * size];
    for (_, r) in &results {
        let Some((_, _, _, tris)) = r else { continue };
        for t in tris {
            // project x,z → pixel
            let pts: Vec<(f32, f32, f32)> = t
                .iter()
                .map(|v| (((v.x - gmn.x) / ext) * (size as f32 - 1.0), ((v.z - gmn.z) / ext) * (size as f32 - 1.0), v.y))
                .collect();
            let minx = pts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
            let maxx = pts.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max).ceil().min(size as f32 - 1.0) as usize;
            let miny = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
            let maxy = pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max).ceil().min(size as f32 - 1.0) as usize;
            if maxx - minx > 400 || maxy - miny > 400 {
                continue; // skip sky/huge polys
            }
            let (x0, y0, _) = pts[0];
            let (x1, y1, _) = pts[1];
            let (x2, y2, _) = pts[2];
            let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
            if area.abs() < 1e-6 {
                continue;
            }
            for py in miny..=maxy {
                for px in minx..=maxx {
                    let fx = px as f32 + 0.5;
                    let fy = py as f32 + 0.5;
                    let w0 = ((x1 - fx) * (y2 - fy) - (x2 - fx) * (y1 - fy)) / area;
                    let w1 = ((x2 - fx) * (y0 - fy) - (x0 - fx) * (y2 - fy)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 >= -0.002 && w1 >= -0.002 && w2 >= -0.002 {
                        let h = w0 * pts[0].2 + w1 * pts[1].2 + w2 * pts[2].2;
                        let idx = py * size + px;
                        if h > zbuf[idx] {
                            zbuf[idx] = h;
                            let t = ((h - gmn.y) / (gmx.y - gmn.y).max(1.0)).clamp(0.0, 1.0);
                            let shade = (40.0 + 215.0 * t.powf(0.5)) as u8;
                            img[idx * 4] = shade;
                            img[idx * 4 + 1] = (60.0 + 150.0 * t) as u8;
                            img[idx * 4 + 2] = (200.0 - 150.0 * t) as u8;
                            img[idx * 4 + 3] = 255;
                        }
                    }
                }
            }
        }
    }
    write_png(Path::new(&a[1]), size as u32, size as u32, &img)?;
    println!("wrote {}", a[1]);
    Ok(())
}

/// mcla-tool char <set.r6>  - bones, meshes and skin binding of a character set
fn cmd_char(a: &[String]) -> Result<()> {
    use mcla_formats::rage::character::Character;
    let res = load_res(&a[0])?;
    let c = Character::parse(&res)?;
    println!("{}: {} bones, {} models, {} textures, shader textures {:?}", c.name, c.bones.len(), c.models.len(), c.textures.len(), c.shader_tex);
    for t in &c.textures {
        println!("  texture {} {}x{} {:?}", t.name, t.width, t.height, t.format);
    }
    for (i, b) in c.bones.iter().enumerate() {
        println!("  bone {:2} {:<14} parent {:?} world ({:.2},{:.2},{:.2}) local ({:.2},{:.2},{:.2})", i, b.name, b.parent, b.world_pos.x, b.world_pos.y, b.world_pos.z, b.local_pos.x, b.local_pos.y, b.local_pos.z);
    }
    println!("  lods {:?}; shaders {:?}", c.lods, c.shader_names);
    for (mi, m) in c.models.iter().enumerate() {
        for (mesh, si) in m {
            println!("  model {} shader {} verts {} tangents {} uv1 {} colors {} first tangent {:?}", mi, si, mesh.positions.len(), mesh.tangents.len(), mesh.uv1.len(), mesh.colors.len(), mesh.tangents.first());
        }
    }
    for (i, sh) in c.shaders.iter().enumerate() {
        println!("  shader {} {} ({})", i, sh.name, sh.source_path);
        for (h, p) in &sh.params {
            println!("    {:08x} {:?}", h, p);
        }
    }
    for extra in &a[1..] {
        let res = load_res(extra)?;
        let mut anims = mcla_formats::rage::anim::find_animations(&res);
        if anims.is_empty() {
            let rd = mcla_formats::rage::Rd::new(&res);
            let want = std::env::var("MCLA_ANIM").unwrap_or_default().to_ascii_lowercase();
            for (o, name) in mcla_formats::rage::anim::pack_entries(&res) {
                if name.to_ascii_lowercase().contains(&want) {
                    anims.extend(mcla_formats::rage::anim::animation_named(&rd, o));
                }
            }
        }
        for an in anims {
            if std::env::var("MCLA_ANIM").is_ok() {
                for b in ["root", "head", "wrist_l", "wrist_r", "ankle_l"] {
                    let Some(bi) = c.bone(b) else { continue };
                    let id = c.bones[bi].id;
                    let last = (an.frames - 1) as f32;
                    let t = an.track(0, id).map(|t| (t.data.vec(0.0), t.data.vec(last)));
                    let r = an.track(1, id).map(|t| (t.data.rot(0.0), t.data.rot(last)));
                    println!("    {} id {:04x} trans {:?} rot {:?}", b, id, t, r);
                }
            }
            if std::env::var("MCLA_UNMATCHED").is_ok() {
                let nb: Vec<String> = c.bones.iter().filter(|b| an.track(1, b.id).is_none()).map(|b| format!("{}({})", b.name, b.id)).collect();
                let nt: Vec<String> = an.tracks.iter().filter(|t| !c.bones.iter().any(|b| b.id == t.bone_id)).map(|t| format!("{}:{}", t.kind, t.bone_id)).collect();
                println!("    bones without rotation track: {}", nb.join(" "));
                println!("    tracks without bone: {}", nt.join(" "));
            }
            let matched = an.tracks.iter().filter(|t| c.bones.iter().any(|b| b.id == t.bone_id)).count();
            let mover = an.track(5, 0).map(|t| t.data.vec((an.frames - 1) as f32) - t.data.vec(0.0));
            println!("  anim {} frames {} dur {:.2} loop {} tracks {} (matched {}) mover travel {:?}", an.short(), an.frames, an.duration, an.looped, an.tracks.len(), matched, mover);
        }
    }
    for (mi, m) in c.models.iter().enumerate() {
        for (mesh, si) in m {
            let (mn, mx) = mesh.bbox();
            // per component: mean weight, and weight-averaged distance to the palette bone
            let mut stat = [[0f32; 3]; 4];
            let mut best = 0f32;
            let mut used: std::collections::BTreeSet<String> = Default::default();
            for ((v, bi), w) in mesh.positions.iter().zip(mesh.blend_indices.iter()).zip(mesh.blend_weights.iter()) {
                let w = w.to_array();
                let mut nearest = f32::MAX;
                for k in 0..4 {
                    let bone = mesh.bone_palette.get(bi[k] as usize).copied().unwrap_or(bi[k] as u16) as usize;
                    if let Some(b) = c.bones.get(bone) {
                        stat[k][0] += w[k];
                        stat[k][1] += w[k] * v.distance(b.world_pos);
                        stat[k][2] += 1.0;
                        if w[k] > 0.3 {
                            nearest = nearest.min(v.distance(b.world_pos));
                            used.insert(b.name.clone());
                        }
                    }
                }
                best = best.max(if nearest == f32::MAX { 9.0 } else { nearest });
            }
            let top: Vec<String> = stat.iter().map(|s| format!("w{:.2} d{:.3}", s[0] / s[2].max(1.0), s[1] / s[0].max(1e-6))).collect();
            println!("  model {} shader {} tris {} y {:.2}..{:.2} palette {} comps {:?} worst-dominant {:.2} bones {:?}", mi, si, mesh.indices.len() / 3, mn.y, mx.y, mesh.bone_palette.len(), top, best, used);
            if let (Some(w), Some(bi)) = (mesh.blend_weights.first(), mesh.blend_indices.first()) {
                println!("     sample weights {:?} indices {:?}", w, bi);
            }
        }
    }
    Ok(())
}

/// mcla-tool sco <file or dir>   - decode script objects and print their strings
fn cmd_sco(a: &[String]) -> Result<()> {
    fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(p) {
                let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
                v.sort();
                for c in v {
                    walk(&c, out);
                }
            }
        } else {
            out.push(p.to_path_buf());
        }
    }
    let mut files = Vec::new();
    walk(Path::new(&a[0]), &mut files);
    for f in files {
        let Ok(d) = std::fs::read(&f) else { continue };
        match mcla_formats::sco::decode(&d) {
            Ok(s) => println!("{}	code {} statics {} globals {}	{}", f.display(), s.code_size, s.statics, s.globals, s.strings(4).join(" | ")),
            Err(e) => println!("{}	ERROR {}", f.display(), e),
        }
    }
    Ok(())
}

/// mcla-tool tex <file.r1> <out.png>   - a resource whose root is a single texture object
fn cmd_tex(a: &[String]) -> Result<()> {
    let res = load_res(&a[0])?;
    let rd = mcla_formats::rage::Rd::new(&res);
    // the texture object is the root, or hangs off one of its first pointers
    let mut found = None;
    // optional third argument: offset (hex) of the pointer to the texture inside the root object
    let at = a.get(2).and_then(|x| usize::from_str_radix(x.trim_start_matches("0x"), 16).ok()).and_then(|o| rd.ptr(o));
    for o in [at, Some(0), rd.ptr(0), rd.ptr(4), rd.ptr(8), rd.ptr(0x0c)].into_iter().flatten() {
        if let Ok(t) = mcla_formats::rage::texture::Texture::parse(&rd, o) {
            found = Some(t);
            break;
        }
    }
    let Some(t) = found else { bail!("no texture object found") };
    println!("{} {}x{} {:?} levels {}", t.name, t.width, t.height, t.format, t.levels.len());
    write_png(Path::new(&a[1]), t.width, t.height, &t.to_rgba8())?;
    Ok(())
}

/// mcla-tool cine <pack.r1> [name substring] - raw track listing of cine animations
fn cmd_cine(a: &[String]) -> Result<()> {
    let res = load_res(&a[0])?;
    let rd = mcla_formats::rage::Rd::new(&res);
    let mut o = 4;
    while o + 4 <= res.virtual_size {
        let Some(an) = rd.ptr(o) else { break };
        o += 4;
        let Some(np) = rd.ptr(an + 0x1c) else { continue };
        let name = rd.cstr(np);
        if let Some(f) = a.get(1) {
            if !name.to_ascii_lowercase().contains(&f.to_ascii_lowercase()) {
                continue;
            }
        }
        let frames = rd.u16(an + 8) as usize;
        let nblk = rd.u16(an + 0x18) as usize;
        println!("{} flags {:04x} frames {} dur {:.3} blocks {} tracks {} hdr {:08x} {:08x} {:08x} {:08x}", name, rd.u16(an + 4), frames, rd.f32(an + 0x0c), nblk, rd.u16(an + 0x24), rd.u32(an + 0x10), rd.u32(an + 0x28), rd.u32(an + 0x2c), rd.u32(an));
        let Some(bl) = rd.ptr(an + 0x14) else { continue };
        for b in 0..nblk.min(if std::env::var("MCLA_ALLBLK").is_ok() { 99 } else { 1 }) {
            let Some(blk) = rd.ptr(bl + b * 4) else { continue };
            let Some(ta) = rd.ptr(blk + 4) else { continue };
            let nt = rd.u16(blk + 8) as usize;
            println!("  block {} tracks {} frames {} raw {:08x} {:08x} {:08x}", b, nt, rd.u16(blk + 0x10), rd.u32(blk + 0x0c), rd.u32(blk + 0x10), rd.u32(blk + 0x14));
            for t in 0..nt.min(if std::env::var("MCLA_ALLTRK").is_ok() { 999 } else { 12 }) {
                let Some(tr) = rd.ptr(ta + t * 4) else { continue };
                let n = rd.u32(tr + 0x14) as usize;
                let mut desc = String::new();
                for k in 0..n.min(4) {
                    let Some(ch) = rd.ptr(tr + 4 + k * 4) else { continue };
                    let ty = rd.u8(ch + 5);
                    match ty {
                        4 => desc.push_str(&format!(" [const {:.4}]", rd.f32(ch + 8))),
                        6 => {
                            let nb = rd.u32(ch + 0x0c);
                            let cnt = rd.u32(ch + 0x10);
                            let (sc, off) = (rd.f32(ch + 0x14), rd.f32(ch + 0x18));
                            desc.push_str(&format!(" [q{}x{} {:.4}..{:.4}]", nb, cnt, off, off + sc * ((1u64 << nb.min(32)) - 1) as f32));
                        }
                        9 => {
                            let p = rd.ptr(ch + 8).unwrap_or(0);
                            desc.push_str(&format!(" [quat {:.3} {:.3} {:.3} {:.3}]", rd.f32(p), rd.f32(p + 4), rd.f32(p + 8), rd.f32(p + 12)));
                        }
                        0x0d => {
                            let p = rd.ptr(ch + 8).unwrap_or(0);
                            desc.push_str(&format!(" [vec {:.3} {:.3} {:.3}]", rd.f32(p), rd.f32(p + 4), rd.f32(p + 8)));
                        }
                        o => desc.push_str(&format!(" [type {:#x}: {:08x} {:08x} {:08x} {:08x} {:08x}]", o, rd.u32(ch + 8), rd.u32(ch + 0x0c), rd.u32(ch + 0x10), rd.u32(ch + 0x14), rd.u32(ch + 0x18))),
                    }
                }
                println!("    track kind {} flags {:#04x} id {:5} ch {}:{}", rd.u8(tr), rd.u8(tr + 1), rd.u16(tr + 2), n, desc);
            }
        }
        // MCLA_JAW=<set.r6>: mouth movement per second (which lines a character speaks)
        if let Ok(setf) = std::env::var("MCLA_JAW") {
            let ch = mcla_formats::rage::character::Character::parse(&load_res(&setf)?)?;
            let bone = std::env::var("MCLA_BONE").unwrap_or("jaw_c".into());
            if let (Some(bi), Some(anim)) = (ch.bone(&bone), mcla_formats::rage::anim::animation_named(&rd, an)) {
                let id = ch.bones[bi].id;
                if let Some(t) = anim.track(1, id) {
                    let fps = (anim.frames.max(2) - 1) as f32 / anim.duration;
                    let mut line = String::new();
                    let rest = t.data.rot(0.0);
                    for sec in 0..anim.duration as usize {
                        let mut mx = 0.0f32;
                        for k in 0..10 {
                            let f = (sec as f32 + k as f32 / 10.0) * fps;
                            mx = mx.max(rest.angle_between(t.data.rot(f)).to_degrees());
                        }
                        line.push_str(&format!("{:.0} ", mx));
                    }
                    println!("    {} (id {}) per second: {}", bone, id, line);
                } else {
                    println!("    no {} track (id {})", bone, id);
                }
            }
            continue;
        }
        // decoded view
        if let Some(an) = mcla_formats::rage::anim::animation_named(&rd, an) {
            for t in &an.tracks {
                let last = (an.frames.max(1) - 1) as f32;
                match &t.data {
                    mcla_formats::rage::anim::TrackData::Vec(_) => println!("    decoded kind {} id {} vec {:?} -> {:?} -> {:?}", t.kind, t.bone_id, t.data.vec(0.0), t.data.vec(last * 0.5), t.data.vec(last)),
                    mcla_formats::rage::anim::TrackData::Rot(_) => println!("    decoded kind {} id {} rot {:?} -> {:?}", t.kind, t.bone_id, t.data.rot(0.0), t.data.rot(last)),
                }
                if an.tracks.len() > 12 {
                    break;
                }
            }
        } else {
            println!("    (not decodable by animation_named)");
        }
    }
    Ok(())
}

/// mcla-tool mcv <video.mcv> <frame> <out.png> - decode one frame of a converted movie
fn cmd_mcv(a: &[String]) -> Result<()> {
    let d = std::fs::read(&a[0])?;
    if d.len() < 20 || &d[0..4] != b"MCV1" {
        bail!("not an MCV file");
    }
    let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as usize;
    let (w, h, fps, n) = (u(4), u(8), u(12) as f32 / 1000.0, u(16));
    println!("{}x{} {:.2} fps {} frames", w, h, fps, n);
    let i: usize = a[1].parse()?;
    let off = |k: usize| u64::from_le_bytes(d[20 + k * 8..28 + k * 8].try_into().unwrap()) as usize;
    let t = std::time::Instant::now();
    let img = mcla_formats::jpeg::decode(&d[off(i)..off(i + 1)])?;
    println!("decoded in {:.1} ms", t.elapsed().as_secs_f32() * 1000.0);
    write_png(Path::new(&a[2]), img.width as u32, img.height as u32, &img.rgba)?;
    Ok(())
}
