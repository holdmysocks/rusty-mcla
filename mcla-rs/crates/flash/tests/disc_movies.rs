//! Opt-in: every UI movie on the disc parses, runs 120 frames of its own
//! ActionScript and draws. `MCLA_DATA_DIR=<extracted cache> cargo test -p mcla-flash -- --include-ignored`

use mcla_flash::player::Player;
use mcla_flash::swf::Movie;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn movies(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            movies(&p, out);
        } else if p.extension().map_or(false, |x| x == "r27") {
            out.push(p);
        }
    }
}

fn load(p: &Path) -> mcla_formats::rsc::Resource {
    let d = std::fs::read(p).unwrap();
    assert_eq!(&d[0..4], b"MCLR");
    let rtype = u32::from_le_bytes(d[4..8].try_into().unwrap());
    let flags = u32::from_le_bytes(d[8..12].try_into().unwrap());
    let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
    let data = d[16..].to_vec();
    mcla_formats::rsc::Resource { rtype, flags, virtual_size: vs.min(data.len()), physical_size: data.len().saturating_sub(vs), data }
}

#[test]
#[ignore]
fn every_disc_movie_runs() {
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")).join("resources/ui");
    let mut list = Vec::new();
    movies(&dir, &mut list);
    assert_eq!(list.len(), 21, "UI movies on the disc");
    for path in list {
        let movie = Arc::new(Movie::parse(&load(&path)).unwrap_or_else(|e| panic!("{}: {}", path.display(), e)));
        let mut p = Player::new(movie);
        for _ in 0..120 {
            p.tick();
        }
        let list = mcla_flash::draw::build(&p);
        println!("{}: {} instances, {} triangles, {} warnings", path.display(), p.insts.iter().flatten().count(), list.triangle_count(), p.warnings.len());
    }
}

#[test]
#[ignore]
fn hud_shows_speed_and_gear_from_globals() {
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
    let movie = Arc::new(Movie::parse(&load(&dir.join("resources/ui/hud/0x1f0e1cda.r27"))).unwrap());
    let mut p = Player::new(movie);
    for _ in 0..30 {
        // 20 m/s -> the movie's own conversion: int(20 * 2.237) = 44 mph
        p.set_global("d_Speedvalue", mcla_flash::value::Value::Float(20.0));
        p.set_global("GearValue", mcla_flash::value::Value::str("3"));
        p.tick();
    }
    let speed = p.find("HUD.speedometer.speed").expect("speed field");
    assert_eq!(p.inst(speed).unwrap().text.as_deref(), Some("44"));
    let gear = p.find("HUD.speedometer.gearindicator.slot").expect("gear field");
    assert_eq!(p.inst(gear).unwrap().text.as_deref(), Some("3"));
}

/// Opt-in: the letters the five text languages use (the string table `ui/0xa4c1e8fa`, blocks 0..4) against the glyphs
/// of the movies' fonts. Prints, per font name, the letters some language uses that no instance of that font has.
#[test]
#[ignore]
fn language_letters_in_the_fonts() {
    let root = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
    let t = mcla_formats::strtbl::StringTable::parse(&std::fs::read(root.join("ui/0xa4c1e8fa")).unwrap()).unwrap();
    let mut list = Vec::new();
    movies(&root.join("resources/ui"), &mut list);
    let mut fonts: std::collections::BTreeMap<String, std::collections::BTreeSet<u16>> = Default::default();
    for path in list {
        let movie = Movie::parse(&load(&path)).unwrap();
        for c in movie.chars.iter().flatten() {
            if let mcla_formats::rage::swf::Character::Font(f) = c {
                fonts.entry(f.name.clone()).or_default().extend(f.codes.iter().copied());
            }
        }
    }
    for lang in 0..5 {
        let used: std::collections::BTreeSet<char> = t.langs[lang].values().flat_map(|s| s.chars()).filter(|&c| c as u32 > 127).collect();
        eprintln!("language {lang}: {} letters above ASCII: {}", used.len(), used.iter().collect::<String>());
        for (name, codes) in &fonts {
            if codes.len() < 120 {
                continue;
            }
            let miss: String = used.iter().filter(|&&c| !codes.contains(&(c as u16))).collect();
            eprintln!("  {name} ({} glyphs) lacks: {miss:?}", codes.len());
        }
    }
}
