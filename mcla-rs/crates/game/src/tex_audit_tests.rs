//! Texture name collisions between the dictionaries the port keys by the name's hash alone (LIGHTING.md open item 14).
//!
//! The GPU cache (`Gpu::upload_texture`) keeps the FIRST texture uploaded under a key. The streamed sectors upload
//! their own textures and the shared dictionaries' under the bare hash (`World::upload`'s `sk` / `fk` for a sector
//! that is neither an interior set nor a far model), `World::texture_any` the global DDS files too. Where two of these
//! hold different pictures under one name, whichever was loaded first is drawn for both; the original resolves a
//! sector's names in its own dictionary first. This lists every such name with its sources.
//!
//! `cargo test --release -p mcla tex_audit -- --ignored --nocapture` (MCLA_DATA_DIR, default D:/MCLA/extracted/cache;
//! MCLA_TEX_AUDIT_OUT=<file> also writes the list as TSV).

use crate::assets::{load_resource_file, Assets};
use crate::world::WorldIndex;
use mcla_formats::rage::sector::Sector;
use mcla_formats::rage::texture::Texture;
use mcla_formats::rage::{Dictionary, Rd};
use std::collections::{BTreeMap, HashMap};

/// A picture's identity: size, format and a hash of every level's bytes.
fn ident(t: &Texture) -> (u32, u32, String, u64) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for l in &t.levels {
        l.hash(&mut h);
    }
    (t.width, t.height, format!("{:?}", t.format), h.finish())
}

fn mean0(t: &Texture) -> f32 {
    let l = &t.levels[0];
    if l.is_empty() {
        return 0.0;
    }
    l.iter().map(|&b| b as f32).sum::<f32>() / l.len() as f32
}

#[test]
#[ignore]
fn tex_audit() {
    let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
    let assets = Assets::new(&data);
    let Ok(index) = WorldIndex::load_or_build(&assets) else { eprintln!("no world index under {data}"); return };
    // name hash -> [(source, ident, name, mean of level 0 bytes)]
    let mut by: HashMap<u32, Vec<(String, (u32, u32, String, u64), String, f32)>> = HashMap::new();
    fn add(by: &mut HashMap<u32, Vec<(String, (u32, u32, String, u64), String, f32)>>, src: String, h: u32, t: &Texture) {
        by.entry(h).or_default().push((src, ident(t), t.name.clone(), mean0(t)));
    }
    // the shared dictionaries, in the lookup order of `SharedTextures::get` (the far one is keyed apart only for far models)
    let mut dict_files: Vec<std::path::PathBuf> = index.dicts.iter().map(|f| assets.path(f)).collect();
    for d in ["resources/tires/textures", "resources/brakes", "resources/license_plates"] {
        fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(p) else { return };
            let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            v.sort();
            for p in v {
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().map_or(false, |x| x == "r1") {
                    out.push(p);
                }
            }
        }
        walk(&assets.path(d), &mut dict_files);
    }
    let mut nd = 0;
    for p in &dict_files {
        let Ok(r) = load_resource_file(p) else { continue };
        let rd = Rd::new(&r);
        if rd.vtable(0) != mcla_formats::rage::vt::TEXTURE_DICT {
            continue;
        }
        let Ok(d) = Dictionary::parse(&rd, 0) else { continue };
        nd += 1;
        let tag = format!("dict:{}", p.strip_prefix(assets.path("")).unwrap_or(p).display());
        for (h, o) in d.hashes.iter().zip(&d.values).filter_map(|(h, o)| o.map(|o| (*h, o))) {
            if let Ok(t) = Texture::parse(&rd, o) {
                add(&mut by, tag.clone(), h, &t);
            }
        }
    }
    // the streamed sectors' own dictionaries
    let secs: Vec<_> = index.sectors.iter().map(|s| s.file.clone()).collect();
    // (identities only, worked out per sector: holding every sector's pictures at once would take gigabytes)
    type Row = (u32, (u32, u32, String, u64), String, f32);
    let own: Vec<Vec<Row>> = {
        use rayon::prelude::*;
        secs.par_iter().map(|f| load_resource_file(&assets.path(f)).ok().and_then(|r| Sector::parse(&r).ok()).map(|s| s.textures.iter().map(|(h, t)| (*h, ident(t), t.name.clone(), mean0(t))).collect()).unwrap_or_default()).collect()
    };
    let mut ns = 0;
    for (f, ts) in secs.iter().zip(own) {
        if !ts.is_empty() {
            ns += 1;
        }
        for (h, id, name, m) in ts {
            by.entry(h).or_default().push((format!("sector:{f}"), id, name, m));
        }
    }
    // the global DDS files (`World::load_globaltex`: the file stem's hash)
    let gdir = assets.path("textures/global/cars/globaltex");
    let mut ng = 0;
    if let Ok(rd) = std::fs::read_dir(&gdir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map_or(false, |x| x.eq_ignore_ascii_case("dds")) {
                let stem = p.file_stem().unwrap().to_string_lossy().to_ascii_lowercase();
                if let Ok(d) = std::fs::read(&p) {
                    if let Ok(t) = mcla_formats::dds::parse_dds(&p.to_string_lossy(), &d) {
                        ng += 1;
                        add(&mut by, format!("globaltex:{stem}"), mcla_formats::hash::joaat(&stem), &t);
                    }
                }
            }
        }
    }
    // every car's own maps (`vehicles.rs` uploads them by `car_upload` under the bare name, as stored: the key
    // `h ^ 0x0052_4157` that `World::texture_any_raw` also gives the shared and global textures), one car at a time
    let mut nc_cars = 0;
    if let Ok(rd) = std::fs::read_dir(assets.path("resources/vehicle")) {
        let mut dirs: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            if let Ok(car) = crate::vehicle_model::load_car(&assets, &d) {
                nc_cars += 1;
                let tag = format!("car:{}", d.file_name().unwrap().to_string_lossy());
                for (h, t) in &car.textures {
                    add(&mut by, tag.clone(), *h, t);
                }
            }
        }
    }
    eprintln!("cars {nc_cars}");
    eprintln!("dictionaries {nd}, sectors with own textures {ns} of {}, global DDS {ng}, names {}", secs.len(), by.len());
    let mut out = String::from("hash\tname\tdistinct\tsources\tdetail\n");
    let mut rows: BTreeMap<String, String> = BTreeMap::new();
    let (mut nc, mut nsec) = (0, 0);
    for (h, v) in &by {
        let mut ids: Vec<&(u32, u32, String, u64)> = v.iter().map(|x| &x.1).collect();
        ids.sort();
        ids.dedup();
        if ids.len() < 2 {
            continue;
        }
        nc += 1;
        let sector_side = v.iter().any(|x| x.0.starts_with("sector:"));
        if sector_side {
            nsec += 1;
        }
        // one entry per distinct picture: its size / format / mean and how many sources hold it (first source named)
        let mut groups: Vec<String> = Vec::new();
        for id in &ids {
            let of: Vec<&(String, (u32, u32, String, u64), String, f32)> = v.iter().filter(|x| &x.1 == *id).collect();
            groups.push(format!("{}x{} {} mean {:.1} x{} [{}]", id.0, id.1, id.2, of[0].3, of.len(), of[0].0));
        }
        let name = v.iter().map(|x| x.2.clone()).find(|n| !n.is_empty()).unwrap_or_default();
        rows.insert(format!("{name}\t{h:08x}"), format!("{h:08x}\t{name}\t{}\t{}\t{}", ids.len(), v.len(), groups.join(" | ")));
    }
    for r in rows.values() {
        out.push_str(r);
        out.push('\n');
    }
    eprintln!("names with different pictures under one key: {nc} ({nsec} with a sector's own copy among them)");
    for r in rows.values().take(400) {
        eprintln!("{r}");
    }
    if let Ok(p) = std::env::var("MCLA_TEX_AUDIT_OUT") {
        std::fs::write(p, out).unwrap();
    }
}
