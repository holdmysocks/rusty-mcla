//! Lookup routines of the archive, resource-header, particle-evolution and blend-target code compared with the original
//! (tools/formats_lookup_reference.py runs the generated routines unchanged on a flat guest memory; the cases and the
//! original's answers are in testdata/*_lookup_ref.txt). The dump tests (ignored) write the real inputs the script reads.

use crate::rage::ptfx::FxList;
use crate::rsc::Resource;
use crate::{ReadAt, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct Mem(pub Vec<u8>);
impl ReadAt for Mem {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()> {
        buf.copy_from_slice(&self.0[off as usize..off as usize + buf.len()]);
        Ok(())
    }
}

fn data_root() -> PathBuf {
    std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()).into()
}

fn out_dir() -> PathBuf {
    std::env::var("MCLA_VERIFY_OUT").unwrap_or_else(|_| "D:/MCLA/work/verify-formats/in".into()).into()
}

fn load_res(path: &Path) -> Option<Resource> {
    let d = std::fs::read(path).ok()?;
    if d.len() < 16 || &d[0..4] != b"MCLR" {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let data = d[16..].to_vec();
    let vs = (u(12) as usize).min(data.len());
    Some(Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data })
}

fn toc_text(count: usize, toc: &[u8]) -> String {
    let mut s = format!("N {}\n", count);
    for i in 0..count {
        let w = |k: usize| u32::from_le_bytes(toc[i * 16 + k * 4..i * 16 + k * 4 + 4].try_into().unwrap());
        s += &format!("E {:08x} {:08x} {:08x} {:08x}\n", w(0), w(1), w(2), w(3));
    }
    s
}

/// The decrypted table of contents of the disc's four big archives and the three small ones in the data directory.
#[test]
#[ignore]
fn dump_real_tocs() {
    let iso = std::env::var("MCLA_ISO").unwrap_or_else(|_| "D:/MCLA/Midnight Club - Los Angeles - Complete Edition (USA, Europe).iso".into());
    let iso = crate::iso::Xiso::open(Path::new(&iso)).unwrap();
    let key = crate::keys::derive(&iso).unwrap();
    std::fs::create_dir_all(out_dir()).unwrap();
    let mut srcs: Vec<(String, Arc<dyn ReadAt>)> = Vec::new();
    for n in ["xarchive_audio.rpf", "xarchive_audlo.rpf", "xarchive_cache.rpf", "xarchive_music.rpf"] {
        srcs.push((n.to_string(), Arc::new(iso.file(n).unwrap())));
    }
    for n in ["xarchive_bootscripts.rpf", "xarchive_cameratune.rpf", "xarchive_carconfigs.rpf"] {
        if let Ok(d) = std::fs::read(data_root().join("preload").join(n)) {
            srcs.push((n.to_string(), Arc::new(Mem(d))));
        }
    }
    for (name, src) in srcs {
        let hdr = src.read_vec(0, 0x20).unwrap();
        assert_eq!(&hdr[0..4], b"RPF3");
        let toc_size = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
        let count = u32::from_le_bytes(hdr[8..12].try_into().unwrap()) as usize;
        let mut toc = src.read_vec(0x800, toc_size).unwrap();
        if u32::from_le_bytes(hdr[0x10..0x14].try_into().unwrap()) == 0xFFFF_FFFF {
            crate::crypto::decrypt_toc(&mut toc, &key);
        }
        std::fs::write(out_dir().join(format!("toc_{}.txt", name.trim_end_matches(".rpf"))), toc_text(count, &toc)).unwrap();
    }
}

/// Every evolution group of the effect list (effects and their events), names in the parser's order.
#[test]
#[ignore]
fn dump_evo_groups() {
    let res = load_res(&data_root().join("resources/rmptfx/fxlists/MC4.r107")).expect("MC4.r107");
    let fx = FxList::parse(&res).unwrap();
    let mut groups: Vec<Vec<String>> = Vec::new();
    for e in &fx.effects {
        if let Some(g) = &e.evo {
            groups.push(g.names.clone());
        }
        for ev in &e.events {
            if let Some(g) = &ev.evo {
                groups.push(g.names.clone());
            }
        }
    }
    let mut s = String::new();
    for g in groups {
        s += &g.iter().map(|n| n.bytes().map(|b| format!("{:02x}", b)).collect::<String>()).collect::<Vec<_>>().join(" ");
        s += "\n";
    }
    std::fs::create_dir_all(out_dir()).unwrap();
    std::fs::write(out_dir().join("evo_groups.txt"), s).unwrap();
}

/// The resource files the blend-shape parser takes (at least one target): their paths below the data directory.
#[test]
#[ignore]
fn dump_blend_files() {
    let root = data_root();
    let mut found = Vec::new();
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let mut es: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        es.sort();
        for p in es {
            if p.is_dir() {
                walk(&p, found);
            } else if p.extension().map_or(false, |e| e == "r3") {
                found.push(p);
            }
        }
    }
    walk(&root.join("resources/character"), &mut found);
    let mut s = String::new();
    for p in found {
        let Some(res) = load_res(&p) else { continue };
        if let Ok(bs) = crate::rage::blendshape::BlendShapes::parse(&res) {
            if !bs.targets.is_empty() {
                s += &format!("{}\n", p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    std::fs::create_dir_all(out_dir()).unwrap();
    std::fs::write(out_dir().join("blend_files.txt"), s).unwrap();
}

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn build_rpf(words: &[[u32; 4]]) -> crate::rpf3::Rpf3<Mem> {
    let mut img = vec![0u8; 0x800 + words.len() * 16];
    img[0..4].copy_from_slice(b"RPF3");
    img[4..8].copy_from_slice(&((words.len() * 16) as u32).to_le_bytes());
    img[8..12].copy_from_slice(&(words.len() as u32).to_le_bytes());
    for (i, w) in words.iter().enumerate() {
        for k in 0..4 {
            img[0x800 + i * 16 + k * 4..0x800 + i * 16 + k * 4 + 4].copy_from_slice(&w[k].to_le_bytes());
        }
    }
    crate::rpf3::Rpf3::open(Arc::new(Mem(img)), &[0u8; 32]).unwrap()
}

/// The archives' find (`F-82183`) on the real tables of contents: every path of the reference, answers equal.
fn check_find(text: &str) -> (usize, usize) {
    let (mut n, mut found) = (0, 0);
    let mut words: Vec<[u32; 4]> = Vec::new();
    let mut rpf: Option<crate::rpf3::Rpf3<Mem>> = None;
    let mut name = String::new();
    for l in text.lines() {
        let f: Vec<&str> = l.split(' ').collect();
        match f[0] {
            "A" => {
                words.clear();
                rpf = None;
                name = f[1].to_string();
            }
            "E" => words.push([1, 2, 3, 4].map(|i| u32::from_str_radix(f[i], 16).unwrap())),
            "Q" => {
                let r = rpf.get_or_insert_with(|| build_rpf(&words));
                let path = String::from_utf8(unhex(f[1])).unwrap();
                assert!(!f[2].starts_with('X'), "{}", l);
                let want: i64 = f[2].parse().unwrap();
                let got = r.lookup(&path).map_or(-1, |i| i as i64);
                assert_eq!(got, want, "{} path {:?}", name, path);
                n += 1;
                found += (want >= 0) as usize;
            }
            _ => {}
        }
    }
    (n, found)
}

#[test]
fn find_matches_the_original() {
    let (n, found) = check_find(include_str!("testdata/rpf_lookup_ref.txt"));
    assert!(n > 10000 && found > 5000, "{} {}", n, found);
}

/// The disc's big archive (its reference is too large to keep): <MCLA_VERIFY_OUT>/../out/rpf_lookup_ref_cache.txt.
#[test]
#[ignore]
fn find_matches_the_original_cache() {
    let text = std::fs::read_to_string(out_dir().parent().unwrap().join("out/rpf_lookup_ref_cache.txt")).unwrap();
    let (n, found) = check_find(&text);
    eprintln!("{} queries, {} found", n, found);
}

fn formats_ref() -> &'static str {
    include_str!("testdata/formats_lookup_ref.txt")
}

#[test]
fn name_hash_matches_the_original() {
    let mut n = 0;
    for l in formats_ref().lines().filter(|l| l.starts_with("H ")) {
        let f: Vec<&str> = l.split(' ').collect();
        let (seed, s, want) = (u32::from_str_radix(f[1], 16).unwrap(), unhex(f[2]), u32::from_str_radix(f[3], 16).unwrap());
        assert_eq!(crate::hash::at_string_hash(&s, seed), want, "{:?} seed {:x}", s, seed);
        n += 1;
    }
    assert!(n > 5000);
}

#[test]
fn evolution_index_matches_the_original() {
    use crate::rage::ptfx::EvoGroup;
    let (mut group, mut n, mut skipped) = (None::<EvoGroup>, 0, 0);
    for l in formats_ref().lines() {
        let f: Vec<&str> = l.split(' ').collect();
        match f[0] {
            "G" => {
                let names: Option<Vec<String>> = f[1..].iter().filter(|h| !h.is_empty()).map(|h| String::from_utf8(unhex(h)).ok()).collect();
                group = names.map(|names| EvoGroup { names, props: Vec::new() });
            }
            "P" => {
                let (Some(g), Ok(q)) = (&group, String::from_utf8(unhex(f[1]))) else {
                    skipped += 1;
                    continue;
                };
                let want: i64 = f[2].parse().unwrap();
                assert_eq!(g.index(&q).map_or(-1, |i| i as i64), want, "{:?} in {:?}", q, g.names);
                n += 1;
            }
            _ => {}
        }
    }
    assert!(n > 5000, "{} {}", n, skipped);
}

#[test]
fn blend_target_matches_the_original() {
    use crate::rage::blendshape::BlendShapes;
    let (mut files, mut hits) = (0, 0);
    let mut it = formats_ref().lines().filter(|l| l.starts_with("B ") || l.starts_with("K "));
    while let Some(l) = it.next() {
        let f: Vec<&str> = l.split(' ').collect();
        assert_eq!(f[0], "B");
        let n: usize = f[2].parse().unwrap();
        let want: Vec<(u16, u32)> = (0..n)
            .map(|_| {
                let k: Vec<&str> = it.next().unwrap().split(' ').collect();
                (k[1].parse().unwrap(), k[2].parse().unwrap())
            })
            .collect();
        let Some(res) = load_res(&data_root().join(f[1])) else { continue };
        let bs = BlendShapes::parse(&res).unwrap();
        let got: Vec<(u16, u32)> = (0..=u16::MAX).filter_map(|k| bs.target(k).map(|t| (k, t.slot as u32))).collect();
        assert_eq!(got, want, "{}", f[1]);
        files += 1;
        hits += want.len();
    }
    // (without the extracted data nothing is compared)
    assert!(files == 0 || (files >= 10 && hits > 500), "{} {}", files, hits);
}

#[test]
fn resource_info_matches_the_original() {
    use crate::rpf3::{classify, Kind};
    let mut n = 0;
    for l in formats_ref().lines().filter(|l| l.starts_with("I ")) {
        let (head, res) = l[2..].split_once(" -> ").unwrap();
        let h: Vec<u32> = head.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
        let (found, c, d, avail, words, open_fail, f0) = (h[0], h[1], h[2], h[3] as usize, [h[4], h[5], h[6]], h[7], h[8]);
        let r: Vec<&str> = res.split(' ').collect();
        let want = (r[0].parse::<i32>().unwrap(), u32::from_str_radix(r[1], 16).unwrap());
        let got = if found == 0 {
            (0, f0)
        } else if let Kind::Resource { rtype, flags, .. } = classify(0, c, d, usize::MAX) {
            ((rtype & 0xFF) as i32, flags)
        } else if open_fail != 0 {
            (0, f0)
        } else {
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).take(avail.min(12)).collect();
            crate::rsc::header(&bytes).map_or((0, f0), |(t, f)| (t as i32, f))
        };
        assert_eq!(got, want, "{}", l);
        n += 1;
    }
    assert!(n >= 6000);
}
