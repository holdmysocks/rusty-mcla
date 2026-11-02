//! `mcla-tool swf ...`: inspect and run the game's UI movies (`resources/ui/**/*.r27`).
//!
//! swf info  <movie.r27>                          stage, counts, exports, root timeline
//! swf chars <movie.r27>                          every character with a short description
//! swf tree  <movie.r27> [ticks]                  display list after running `ticks` frames
//! swf asm   <movie.r27>                          ActionScript of every frame and clip event
//! swf png   <movie.r27> <out.png> [ticks] [scale] [name=value ...]
//!                                                run the movie (ActionScript included) and render it;
//!                                                `name=value` sets `_global.name` before every tick
//! swf char  <movie.r27> <id> <out.png> [scale]   render one character at its first frame, no scripts

use crate::{load_res, write_png};
use anyhow::{bail, Context, Result};
use mcla_flash::draw::TexRef;
use mcla_flash::player::Player;
use mcla_flash::raster::{Canvas, TexImage};
use mcla_flash::swf::{Character, Cmd, Movie};
use mcla_flash::value::Value;
use mcla_formats::avm1;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub fn cmd_swf(a: &[String]) -> Result<()> {
    let sub = a.first().context("usage: swf <info|chars|tree|asm|png|char> <movie.r27> ...")?;
    let path = a.get(1).context("movie path")?;
    let res = load_res(path)?;
    let movie = Arc::new(Movie::parse(&res)?);
    match sub.as_str() {
        "info" => info(&movie),
        "chars" => chars(&movie),
        "tree" => {
            let ticks: usize = a.get(2).map(|s| s.parse()).transpose()?.unwrap_or(1);
            let vars: Vec<(String, String)> = a.iter().skip(3).filter_map(|s| s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
            let p = run(&movie, path, ticks, &vars);
            print_inst(&p, p.root, 0);
            report_warnings(&p);
            Ok(())
        }
        "asm" => asm(&movie),
        "run" => {
            let script = std::fs::read_to_string(a.get(2).context("script file")?)?;
            run_script(&movie, path, &script)
        }
        "png" => {
            let out = a.get(2).context("out.png")?;
            let ticks: usize = a.get(3).map(|s| s.parse()).transpose()?.unwrap_or(1);
            let scale: f32 = a.get(4).map(|s| s.parse()).transpose()?.unwrap_or(1.0);
            let vars: Vec<(String, String)> = a.iter().skip(5).filter_map(|s| s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
            let p = run(&movie, path, ticks, &vars);
            report_warnings(&p);
            let list = mcla_flash::draw::build(&p);
            let (w, h) = ((movie.stage[1] - movie.stage[0]) * scale, (movie.stage[3] - movie.stage[2]) * scale);
            render(&movie, path, &list, w as u32, h as u32, scale, [0.0, 0.0], Path::new(out))
        }
        "char" => {
            let id: u16 = a.get(2).context("id")?.parse()?;
            let out = a.get(3).context("out.png")?;
            let scale: f32 = a.get(4).map(|s| s.parse()).transpose()?.unwrap_or(1.0);
            if movie.char(id).is_none() {
                bail!("no character {}", id);
            }
            let mut p = Player::new(movie.clone());
            let inst = p.preview(id);
            // (an optional frame, from 0)
            if let Some(f) = a.get(5).map(|s| s.parse::<usize>()).transpose()? {
                p.goto(inst, f);
            }
            let list = mcla_flash::draw::build_instance(&p, inst);
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for it in &list.items {
                if let mcla_flash::draw::Item::Triangles { verts, .. } = it {
                    for v in verts {
                        x0 = x0.min(v.pos[0]);
                        y0 = y0.min(v.pos[1]);
                        x1 = x1.max(v.pos[0]);
                        y1 = y1.max(v.pos[1]);
                    }
                }
            }
            if x0 > x1 {
                bail!("character {} draws nothing", id);
            }
            let (w, h) = (((x1 - x0) * scale).ceil() as u32 + 4, ((y1 - y0) * scale).ceil() as u32 + 4);
            render(&movie, path, &list, w, h, scale, [2.0 - x0 * scale, 2.0 - y0 * scale], Path::new(out))
        }
        _ => bail!("unknown swf subcommand {}", sub),
    }
}

/// Run `ticks` frames with a logging FSCommand handler.
fn run(movie: &Arc<Movie>, path: &str, ticks: usize, vars: &[(String, String)]) -> Player {
    let mut p = Player::new(movie.clone());
    let strings = load_strings(path);
    p.fscommand = Some(Box::new(move |p: &mut Player, cmd: &str, arg: &str| {
        if cmd.eq_ignore_ascii_case("getstring") {
            let key = p.get_global("fs_stringID").map(|v| v.to_string().to_string()).unwrap_or_default();
            p.set_global("t_stringID", Value::str(strings.get_or(&key)));
        } else if !mcla_flash::player::common_fscommand(p, cmd, arg) {
            log::info!("FSCommand {:?} {:?}", cmd, arg);
        }
    }));
    for _ in 0..ticks {
        for (k, v) in vars {
            let val = if let Ok(i) = v.parse::<i32>() {
                Value::Int(i)
            } else if let Ok(f) = v.parse::<f32>() {
                Value::Float(f)
            } else {
                Value::str(v)
            };
            if k.contains('.') {
                p.set_path(k, val);
            } else {
                p.set_global(k, val);
            }
        }
        p.tick();
    }
    p
}

/// `swf run <movie> <script>`: drive a movie the way the game does. Lines:
///   set <path> <value>   value: 12, 1.5, "text", {} (new object), [] (new array), @a.b (instance)
///   tick <n>             run n movie frames
///   call <path> <args>   call a function/method reachable from _global
///   png <out.png>        render the stage
///   tree                 print the display list
///   get <path>           print a value
fn run_script(movie: &Arc<Movie>, movie_path: &str, script: &str) -> Result<()> {
    let mut p = run(movie, movie_path, 0, &[]);
    let parse = |p: &mut Player, v: &str| -> Value {
        let v = v.trim();
        if v == "{}" {
            p.new_object()
        } else if v == "[]" {
            p.new_array(Vec::new())
        } else if let Some(s) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            Value::str(s)
        } else if let Some(i) = v.strip_prefix('@') {
            p.find(i).map_or(Value::Undef, |id| Value::Obj(mcla_flash::value::Ref::Inst(id)))
        } else if let Ok(i) = v.parse::<i32>() {
            Value::Int(i)
        } else if let Ok(f) = v.parse::<f32>() {
            Value::Float(f)
        } else {
            Value::str(v)
        }
    };
    for (n, line) in script.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
        match cmd {
            "set" => {
                let (k, v) = rest.split_once(' ').context("set <path> <value>")?;
                let v = parse(&mut p, v);
                p.set_path(k, v);
            }
            "tick" => {
                for _ in 0..rest.trim().parse::<usize>().unwrap_or(1) {
                    p.tick();
                }
            }
            "call" => {
                let mut it = rest.split_whitespace();
                let f = it.next().context("call <path>")?;
                let args: Vec<Value> = it.map(|a| parse(&mut p, a)).collect();
                let r = p.call_path(f, &args);
                println!("call {} -> {:?}", f, r);
            }
            "get" => println!("{} = {:?}", rest, p.get_path(rest.trim())),
            "tree" => print_inst(&p, p.root, 0),
            "png" => {
                let list = mcla_flash::draw::build(&p);
                let (w, h) = (movie.stage[1] - movie.stage[0], movie.stage[3] - movie.stage[2]);
                render(movie, movie_path, &list, w as u32, h as u32, 1.0, [0.0, 0.0], Path::new(rest.trim()))?;
            }
            _ => bail!("line {}: unknown command {}", n + 1, cmd),
        }
    }
    report_warnings(&p);
    Ok(())
}

/// Same table precedence as Career::load: main English table, then sorted DLC
/// and auxiliary tables. Locate the cache from the movie path, not the CWD.
fn load_strings(movie_path: &str) -> mcla_formats::strtbl::StringTable {
    use mcla_formats::strtbl::StringTable;
    let mut strings = StringTable::default();
    let Some(root) = Path::new(movie_path).ancestors().find(|d| d.join("ui/0xa4c1e8fa").is_file()) else { return strings };
    let dir = root.join("ui");
    if let Some(t) = std::fs::read(dir.join("0xa4c1e8fa")).ok().and_then(|d| StringTable::parse(&d).ok()) {
        strings = t;
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.file_name().map_or(false, |n| n != "0xa4c1e8fa" && n != "0xac5297d5")).collect();
        files.sort();
        for path in files {
            if let Some(t) = std::fs::read(path).ok().and_then(|d| StringTable::parse(&d).ok()) {
                strings.merge(t);
            }
        }
    }
    strings
}

fn report_warnings(p: &Player) {
    if !p.warnings.is_empty() {
        eprintln!("{} script warnings, first: ", p.warnings.len());
        for w in p.warnings.iter().take(15) {
            eprintln!("  {}", w);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render(movie: &Movie, movie_path: &str, list: &mcla_flash::draw::DrawList, w: u32, h: u32, scale: f32, offset: [f32; 2], out: &Path) -> Result<()> {
    let shared = load_shared(movie_path);
    let mut table: HashMap<TexRef, Option<Arc<TexImage>>> = HashMap::new();
    for it in &list.items {
        let mcla_flash::draw::Item::Triangles { tex: t, .. } = it else { continue };
        if table.contains_key(t) {
            continue;
        }
        let tex = mcla_flash::textures::resolve(movie, *t, &shared);
        if tex.is_none() && *t != TexRef::None {
            log::warn!("swf: no texture for {:?}", t);
        }
        table.insert(*t, tex.map(|t| Arc::new(TexImage { w: t.width, h: t.height, rgba: t.to_rgba8() })));
    }
    let lookup = |t: TexRef| table.get(&t).cloned().flatten();
    let mut canvas = Canvas::new(w, h, [40, 40, 48, 255]);
    canvas.draw(list, scale, offset, &lookup);
    write_png(out, w, h, &canvas.rgba)?;
    println!("{}x{} -> {} ({} triangles)", w, h, out.display(), list.triangle_count());
    Ok(())
}

/// Textures of `resources/ui/shared_latin.r9`, by short lower-case name.
fn load_shared(movie_path: &str) -> hashbrown::HashMap<String, mcla_formats::rage::texture::Texture> {
    use mcla_formats::rage::{texture::Texture, Dictionary, Rd};
    let mut out = hashbrown::HashMap::new();
    let mut dir = Path::new(movie_path).parent();
    while let Some(d) = dir {
        let p = d.join("shared_latin.r9");
        if p.exists() {
            if let Ok(res) = load_res(&p.to_string_lossy()) {
                let rd = Rd::new(&res);
                if let Ok(dict) = Dictionary::parse(&rd, 0) {
                    for o in dict.values.iter().flatten() {
                        if let Ok(t) = Texture::parse(&rd, *o) {
                            let short = t.name.rsplit(['/', '\\']).next().unwrap_or(&t.name).to_ascii_lowercase();
                            out.insert(short, t);
                        }
                    }
                }
            }
            break;
        }
        dir = d.parent();
    }
    out
}

fn info(m: &Movie) -> Result<()> {
    println!("stage {:?} fps {} characters {} root frames {}", m.stage, m.fps, m.chars.len(), m.root.frames.len());
    let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
    for c in m.chars.iter().flatten() {
        *kinds.entry(c.kind()).or_default() += 1;
    }
    println!("{:?}", kinds);
    for (n, id) in &m.exports {
        println!("export {:?} -> {}", n, id);
    }
    for (i, f) in m.root.frames.iter().enumerate() {
        println!("root frame {} tag {:#010x}: {} commands", i, f.tag, f.cmds.len());
    }
    Ok(())
}

fn chars(m: &Movie) -> Result<()> {
    for (i, c) in m.chars.iter().enumerate() {
        let Some(c) = c else { continue };
        let d = match c {
            Character::Shape(s) => format!("bounds {:?} fills {} lines {} cmds {}", s.bounds, s.fills.len(), s.lines.len(), s.cmds.len()),
            Character::Sprite(s) => format!("{} frames", s.frames.len()),
            Character::Bitmap(b) => format!("{}x{} {}", b.width, b.height, b.texture.as_ref().map(|t| t.name.clone()).or(b.external.clone()).unwrap_or_default()),
            Character::Font(f) => format!("{:?} {} glyphs, pages {:?}", f.name, f.codes.len(), f.pages.iter().map(|p| p.height).collect::<Vec<_>>()),
            Character::Text(t) => format!("{} records", t.records.len()),
            Character::EditText(e) => format!("var {:?} text {:?} font {} h {} color {:08x} leading {} html {} align {} rect {:?}", e.variable, e.initial, e.font, e.height, e.color, e.leading, e.html, e.align, e.rect),
            _ => String::new(),
        };
        println!("{:5} {:8} {}", i, c.kind(), d);
    }
    Ok(())
}

fn print_inst(p: &Player, id: u32, indent: usize) {
    let Some(i) = p.inst(id) else { return };
    for &c in i.children.values() {
        let Some(ci) = p.inst(c) else { continue };
        let kind = p.movie.char(ci.character).map_or("?", |c| c.kind());
        println!(
            "{:indent$}depth {} {} #{} {:?} pos ({:.1},{:.1}) frame {}{} alpha {} tint {:02x}{:02x}{:02x}{}",
            "",
            ci.depth,
            kind,
            ci.character,
            &*ci.name,
            ci.matrix.tx,
            ci.matrix.ty,
            ci.frame,
            if ci.playing { "" } else { " stopped" },
            ci.tint[3],
            ci.tint[0],
            ci.tint[1],
            ci.tint[2],
            ci.text.as_ref().map_or(String::new(), |t| format!(" text {:?}", t)),
            indent = indent
        );
        print_inst(p, c, indent + 2);
    }
}

fn asm(m: &Movie) -> Result<()> {
    let mut sprites: Vec<(u16, &mcla_flash::swf::Sprite)> = vec![(0, &m.root)];
    for (i, c) in m.chars.iter().enumerate() {
        if let Some(Character::Sprite(s)) = c {
            sprites.push((i as u16, s));
        }
    }
    for (id, s) in sprites {
        for (fi, f) in s.frames.iter().enumerate() {
            for cmd in &f.cmds {
                match cmd {
                    Cmd::DoAction(code) | Cmd::DoInitAction(code) => {
                        println!("=== sprite {} frame {} ({} bytes)", id, fi, code.len());
                        print!("{}", avm1::disassemble(code));
                    }
                    Cmd::Place(p) => {
                        for e in &p.events {
                            println!("=== sprite {} frame {} clip {:?} event {:#x} key {:#x}", id, fi, p.name, e.flags, e.key);
                            print!("{}", avm1::disassemble(&e.actions));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
