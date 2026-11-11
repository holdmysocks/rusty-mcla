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

/// Opt-in: the HUD's slipstream meter (`speedometer.SST.meter`, 131 frames) goes to frame 1 + 130 x SSTvalue. Only its
/// last frame (SSTvalue 1, green) places a second child, the nitro bar overlay (sprite 442 at depth 2), and no frame
/// removes it. Back to an earlier value the goto wraps (F-85946 clears every child's placed byte) and frame 0 places
/// depth 1 only, so the overlay stays in the list but the draw (F-92070) skips it: the nitro bar shows its own colour
/// again. Triangles of the meter as the original draws them: the same count before and after a full charge.
#[test]
#[ignore]
fn hud_slipstream_overlay_goes_after_a_full_charge() {
    use mcla_flash::value::Value;
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
    let movie = Arc::new(Movie::parse(&load(&dir.join("resources/ui/hud/0x1f0e1cda.r27"))).unwrap());
    let mut p = Player::new(movie);
    let mut run = |p: &mut Player, v: f32, n: usize| {
        for _ in 0..n {
            for (k, x) in [("Speedisvisible", 1), ("Nitroisvisible", 1), ("Nitrovalue", 2), ("SSTisvisible", 1)] {
                p.set_global(k, Value::Int(x));
            }
            p.set_global("SSTvalue", Value::Float(v));
            p.tick();
        }
    };
    let meter = |p: &Player| p.find("HUD.speedometer.SST.meter").expect("meter");
    let tris = |p: &Player| mcla_flash::draw::build_instance(p, meter(p)).triangle_count();
    let overlay = |p: &Player| p.inst(meter(p)).unwrap().children.get(&2).copied();
    run(&mut p, 0.5, 6);
    let half = tris(&p);
    assert!(overlay(&p).is_none());
    run(&mut p, 1.0, 3);
    let full_overlay = overlay(&p).expect("the last frame places the overlay");
    assert_eq!(p.inst(full_overlay).unwrap().character, 442);
    assert!(p.inst(full_overlay).unwrap().placed());
    run(&mut p, 0.5, 3);
    // (still in the list, as in the original: no RemoveObject; not drawn)
    let kept = overlay(&p).expect("kept");
    assert!(!p.inst(kept).unwrap().placed());
    assert_eq!(tris(&p), half, "the meter at 0.5 after a full charge draws as before it");
    run(&mut p, 0.0, 3);
    assert!(!p.inst(overlay(&p).unwrap()).unwrap().placed());
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

/// What the movies hold that the button / focus, init-action and HTML text paths of the runtime would need: buttons and
/// their condition action records, DoInitAction blocks, edit texts with the HTML flag (UI.md, 2026-11-07 tag scan).
#[test]
#[ignore]
fn button_init_and_html_census() {
    use mcla_flash::swf::{Character, Cmd};
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")).join("resources/ui");
    let mut list = Vec::new();
    movies(&dir, &mut list);
    for path in list {
        let m = Movie::parse(&load(&path)).unwrap();
        let (mut buttons, mut with_cond, mut conds, mut html, mut texts, mut init) = (0, 0, 0, 0, 0, 0);
        let mut sprites = vec![&m.root];
        for c in m.chars.iter().flatten() {
            match c {
                Character::Button(b) => {
                    buttons += 1;
                    conds += b.conditions as usize;
                    with_cond += (b.conditions > 0) as usize;
                    for (w, code) in &b.condition_actions {
                        println!("COND {} {:08x} key {}
{}", path.file_name().unwrap().to_string_lossy(), w, w >> 25, mcla_formats::avm1::disassemble(code));
                    }
                }
                Character::EditText(t) => {
                    texts += 1;
                    html += (t.html != 0) as usize;
                }
                Character::Sprite(s) => sprites.push(s),
                _ => {}
            }
        }
        for s in sprites {
            for f in &s.frames {
                init += f.cmds.iter().filter(|c| matches!(c, Cmd::DoInitAction(_))).count();
            }
        }
        println!("CENSUS {}	buttons {}	with_conditions {}	conditions {}	edit_texts {}	html {}	init_actions {}", path.file_name().unwrap().to_string_lossy(), buttons, with_cond, conds, texts, html, init);
    }
}

/// Every clip-depth mask the 21 movies place (`F-92070` draws them into the stencil): its character, whether the
/// shape's own triangles fill their bounding box exactly (an axis-aligned rectangle) and whether a placement turns
/// or skews it. Prints one line per mask; the port draws masks as clip rectangles in the game.
#[test]
#[ignore]
fn disc_masks_are_rectangles() {
    use mcla_flash::draw::{Builder, Item};
    use mcla_formats::rage::swf::{Character, Cmd, CxForm, Matrix};
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")).join("resources/ui");
    let mut list = Vec::new();
    movies(&dir, &mut list);
    let (mut rect, mut other, mut turned) = (0, 0, 0);
    for path in list {
        let movie = Movie::parse(&load(&path)).unwrap();
        let mut sprites: Vec<(u16, &mcla_formats::rage::swf::Sprite)> = vec![(0, &movie.root)];
        for (i, c) in movie.chars.iter().enumerate() {
            if let Some(Character::Sprite(s)) = c {
                sprites.push((i as u16, s));
            }
        }
        for (sid, s) in sprites {
            // depth -> mask character placed there (to see later moves of a mask)
            let mut at: std::collections::HashMap<u16, u16> = Default::default();
            for (fi, f) in s.frames.iter().enumerate() {
                for c in &f.cmds {
                    let Cmd::Place(pl) = c else { continue };
                    if pl.clip_depth > 0 {
                        if let Some(ch) = pl.character {
                            at.insert(pl.depth, ch);
                        }
                    }
                    let Some(&ch) = at.get(&pl.depth) else { continue };
                    if pl.clip_depth == 0 && pl.character.is_some() {
                        at.remove(&pl.depth);
                        continue;
                    }
                    let m = pl.matrix.unwrap_or(Matrix::IDENTITY);
                    let rot = m.b != 0.0 || m.c != 0.0;
                    if rot {
                        turned += 1;
                    }
                    let kind = match movie.chars.get(ch as usize).and_then(|c| c.as_ref()) {
                        Some(Character::Shape(sh)) => {
                            let mut b = Builder::new(&movie);
                            b.shape(sh, &Matrix::IDENTITY, &CxForm::IDENTITY);
                            let (mut r, mut area) = ([f32::MAX, f32::MAX, f32::MIN, f32::MIN], 0.0f64);
                            for it in &b.out.items {
                                if let Item::Triangles { verts, .. } = it {
                                    for t in verts.chunks(3) {
                                        let [p, q, s] = [t[0].pos, t[1].pos, t[2].pos];
                                        area += (((q[0] - p[0]) * (s[1] - p[1]) - (s[0] - p[0]) * (q[1] - p[1])) as f64).abs() / 2.0;
                                    }
                                    for v in verts {
                                        r = [r[0].min(v.pos[0]), r[1].min(v.pos[1]), r[2].max(v.pos[0]), r[3].max(v.pos[1])];
                                    }
                                }
                            }
                            let boxa = ((r[2] - r[0]) * (r[3] - r[1])) as f64;
                            if (area - boxa).abs() <= 1e-4 * boxa.max(1.0) {
                                rect += 1;
                                format!("rect {:?}", r)
                            } else {
                                other += 1;
                                format!("NOT RECT area {:.2} box {:.2} {:?}", area, boxa, r)
                            }
                        }
                        Some(Character::Sprite(_)) => {
                            other += 1;
                            "SPRITE mask".to_string()
                        }
                        c => {
                            other += 1;
                            format!("other {:?}", c.map(|_| ()))
                        }
                    };
                    println!("{} sprite {} frame {} depth {} clip {} char {} m [{} {} {} {}] {}", path.file_name().unwrap().to_string_lossy(), sid, fi, pl.depth, pl.clip_depth, ch, m.a, m.b, m.c, m.d, kind);
                }
            }
        }
    }
    println!("masks: {} rectangles, {} other, {} placements turned or skewed", rect, other, turned);
}

/// The four mask sprites of [`disc_masks_are_rectangles`]: per frame of each, the shapes its display list holds (nested
/// sprites at their frame 0), whether their triangles in the mask's frame fill their bounding box exactly.
#[test]
#[ignore]
fn disc_mask_sprites_are_rectangles() {
    use mcla_flash::draw::{Builder, Item};
    use mcla_formats::rage::swf::{Character, Cmd, CxForm, Matrix, Movie as M};
    fn tris(movie: &M, ch: u16, m: &Matrix, frame: usize, out: &mut Vec<[[f32; 2]; 3]>) {
        match movie.chars.get(ch as usize).and_then(|c| c.as_ref()) {
            Some(Character::Shape(sh)) => {
                let mut b = Builder::new(movie);
                b.shape(sh, m, &CxForm::IDENTITY);
                for it in &b.out.items {
                    if let Item::Triangles { verts, .. } = it {
                        for t in verts.chunks(3) {
                            out.push([t[0].pos, t[1].pos, t[2].pos]);
                        }
                    }
                }
            }
            Some(Character::Sprite(s)) => {
                let mut list: std::collections::BTreeMap<u16, (u16, Matrix)> = Default::default();
                for f in s.frames.iter().take(frame + 1) {
                    for c in &f.cmds {
                        match c {
                            Cmd::Place(pl) => {
                                let e = list.entry(pl.depth).or_insert((0, Matrix::IDENTITY));
                                if let Some(c) = pl.character {
                                    *e = (c, Matrix::IDENTITY);
                                }
                                if let Some(pm) = pl.matrix {
                                    e.1 = pm;
                                }
                            }
                            Cmd::Remove(d) => {
                                list.remove(d);
                            }
                            _ => {}
                        }
                    }
                }
                for (_, (c, pm)) in list {
                    tris(movie, c, &m.concat(&pm), 0, out);
                }
            }
            _ => println!("  char {} not a shape / sprite", ch),
        }
    }
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")).join("resources/ui");
    for (file, ch) in [("0xfcc35320.r27", 31u16), ("0xfcc35320.r27", 1122), ("0x1f0e1cda.r27", 1357), ("0x3ae388d8.r27", 11)] {
        let mut list = Vec::new();
        movies(&dir, &mut list);
        let path = list.iter().find(|p| p.file_name().unwrap().to_string_lossy() == file).unwrap();
        let movie = Movie::parse(&load(path)).unwrap();
        let n = match movie.chars.get(ch as usize).and_then(|c| c.as_ref()) {
            Some(Character::Sprite(s)) => s.frames.len(),
            _ => 1,
        };
        for f in 0..n {
            let mut t = Vec::new();
            tris(&movie, ch, &Matrix::IDENTITY, f, &mut t);
            let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            let mut area = 0.0f64;
            for [p, q, s] in &t {
                area += (((q[0] - p[0]) * (s[1] - p[1]) - (s[0] - p[0]) * (q[1] - p[1])) as f64).abs() / 2.0;
                for v in [p, q, s] {
                    r = [r[0].min(v[0]), r[1].min(v[1]), r[2].max(v[0]), r[3].max(v[1])];
                }
            }
            let boxa = ((r[2] - r[0]) * (r[3] - r[1])) as f64;
            if std::env::var("MASK_DUMP").is_ok() {
                for x in &t {
                    println!("  tri {:?}", x);
                }
            }
            println!("{} char {} frame {}/{}: {} triangles, area {:.3} box {:.3} {:?} -> {}", file, ch, f, n, t.len(), area, boxa, r,
                if (area - boxa).abs() <= 1e-4 * boxa.max(1.0) { "rect" } else { "NOT RECT" });
        }
    }
}

/// Where the masking sprites of [`disc_masks_are_rectangles`] are placed (parent sprite, depth, name) and what each
/// masks (the characters at the masked depths).
#[test]
#[ignore]
fn disc_mask_users() {
    use mcla_formats::rage::swf::{Character, Cmd};
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")).join("resources/ui");
    let mut list = Vec::new();
    movies(&dir, &mut list);
    for (file, holder) in [("0xfcc35320.r27", 144u16), ("0xfcc35320.r27", 1126), ("0x1f0e1cda.r27", 1360), ("0x3ae388d8.r27", 205)] {
        let path = list.iter().find(|p| p.file_name().unwrap().to_string_lossy() == file).unwrap();
        let movie = Movie::parse(&load(path)).unwrap();
        let mut sprites: Vec<(u16, &mcla_formats::rage::swf::Sprite)> = vec![(0, &movie.root)];
        for (i, c) in movie.chars.iter().enumerate() {
            if let Some(Character::Sprite(s)) = c {
                sprites.push((i as u16, s));
            }
        }
        for (sid, s) in &sprites {
            for (fi, f) in s.frames.iter().enumerate() {
                for c in &f.cmds {
                    if let Cmd::Place(pl) = c {
                        if pl.character == Some(holder) {
                            println!("{}: sprite {} placed in sprite {} frame {} depth {} name {:?}", file, holder, sid, fi, pl.depth, pl.name);
                        }
                    }
                }
            }
        }
        if let Some((_, s)) = sprites.iter().find(|(i, _)| *i == holder) {
            for (fi, f) in s.frames.iter().enumerate() {
                for c in &f.cmds {
                    if let Cmd::Place(pl) = c {
                        let kind = pl.character.and_then(|ch| movie.chars.get(ch as usize).and_then(|c| c.as_ref())).map(|c| match c {
                            Character::Shape(_) => "shape",
                            Character::Sprite(_) => "sprite",
                            Character::Text(_) => "text",
                            Character::EditText(_) => "edittext",
                            _ => "other",
                        });
                        println!("   {} frame {} depth {} clip {} char {:?} {:?} name {:?}", holder, fi, pl.depth, pl.clip_depth, pl.character, kind, pl.name);
                    }
                }
            }
        }
    }
}

/// The four masks after 120 frames of each movie: per mask the triangles it marks, the triangles drawn tested against
/// it, and whether a shape past its end depth turned the test off (`F-92070`'s summed-depth rule) and how many
/// triangles were drawn after that before the next mask.
#[test]
#[ignore]
fn disc_mask_census() {
    use mcla_flash::draw::Item;
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")).join("resources/ui");
    let mut list = Vec::new();
    movies(&dir, &mut list);
    for path in list {
        let movie = Arc::new(Movie::parse(&load(&path)).unwrap());
        let mut p = Player::new(movie);
        for _ in 0..120 {
            p.tick();
        }
        let l = mcla_flash::draw::build(&p);
        let (mut mark, mut test, mut after_off, mut state) = (0usize, 0usize, 0usize, 0u8);
        let mut lines = Vec::new();
        for it in &l.items {
            match it {
                Item::MaskBegin => {
                    if state != 0 {
                        lines.push(format!("mask: marks {mark}, tested {test}, after off {after_off}"));
                    }
                    (mark, test, after_off, state) = (0, 0, 0, 1);
                }
                Item::MaskEnd => state = 2,
                Item::MaskOff => {
                    lines.push(format!("  off after {test} tested"));
                    state = 3;
                }
                Item::Triangles { verts, .. } => match state {
                    1 => mark += verts.len() / 3,
                    2 => test += verts.len() / 3,
                    3 => after_off += verts.len() / 3,
                    _ => {}
                },
            }
        }
        if state != 0 {
            lines.push(format!("mask: marks {mark}, tested {test}, after off {after_off}, ends {}", if state == 2 { "with the test on" } else { "off" }));
        }
        if !lines.is_empty() {
            println!("{}: {}", path.file_name().unwrap().to_string_lossy(), lines.join(" | "));
        }
    }
}

/// The live mask clips of a movie after `n` frames: path, depth, clip depth, summed depth, alpha and frame.
#[test]
#[ignore]
fn disc_mask_clips() {
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")).join("resources/ui");
    let mut list = Vec::new();
    movies(&dir, &mut list);
    for path in list {
        let movie = Arc::new(Movie::parse(&load(&path)).unwrap());
        let mut p = Player::new(movie);
        for _ in 0..120 {
            p.tick();
        }
        for (i, inst) in p.insts.iter().enumerate() {
            let Some(inst) = inst else { continue };
            if inst.clip_depth == 0 {
                continue;
            }
            let id = i as mcla_flash::value::InstId;
            let mut names = Vec::new();
            let mut at = Some(id);
            while let Some(x) = at.and_then(|x| p.inst(x)) {
                names.push(format!("{}@{}", x.name, x.depth));
                at = x.parent;
            }
            names.reverse();
            println!("{}: {} clip {} sum {} cx {:?} tint {:?} placed {} children {}", path.file_name().unwrap().to_string_lossy(), names.join("/"), inst.clip_depth, mcla_flash::draw::depth_sum(&p, id), inst.cxform, inst.tint, inst.placed(), inst.children.len());
        }
    }
}

/// The prompt movie's root and `screen` children after 120 frames: depth, name, cxform, tint, placed, children.
#[test]
#[ignore]
fn prompt_movie_tree() {
    let dir = PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
    let movie = Arc::new(Movie::parse(&load(&dir.join("resources/ui/prompt/0x3ae388d8.r27"))).unwrap());
    let mut p = Player::new(movie);
    for _ in 0..120 {
        p.tick();
    }
    let show = |p: &Player, id: mcla_flash::value::InstId, ind: &str| {
        let i = p.inst(id).unwrap();
        for (&d, &c) in &i.children {
            let ci = p.inst(c).unwrap();
            println!("{ind}{d} {} char {} cx {:?} tint {:?} placed {} children {} clip {}", ci.name, ci.character, ci.cxform, ci.tint, ci.placed(), ci.children.len(), ci.clip_depth);
        }
    };
    println!("root:");
    show(&p, p.root, "  ");
    let s = p.find("screen").unwrap();
    println!("screen:");
    show(&p, s, "  ");
    let b = p.find("screen.BG").unwrap();
    println!("screen.BG:");
    show(&p, b, "  ");
    let l = p.find("localisation").unwrap();
    for it in &mcla_flash::draw::build_instance(&p, l).items {
        if let mcla_flash::draw::Item::Triangles { tex, verts, .. } = it {
            let xs: Vec<f32> = verts.iter().map(|v| v.pos[0]).collect();
            let ys: Vec<f32> = verts.iter().map(|v| v.pos[1]).collect();
            println!("localisation: {:?} {} verts x {:.0}..{:.0} y {:.0}..{:.0} colour {:?}", tex, verts.len(), xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max), ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max), verts[0].color);
        }
    }
    let li = p.inst(l).unwrap();
    for (&d, &c) in &li.children {
        let ci = p.inst(c).unwrap();
        println!("localisation child {} {} char {} text {:?}", d, ci.name, ci.character, ci.text);
    }
}
