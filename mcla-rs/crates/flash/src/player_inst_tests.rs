//! Instance helpers against the original's own code run offline (tools/flash_inst_reference.py -> testdata/flash_inst_ref.txt): the object
//! graph is built in guest memory from the offsets the code shows, the port's instances are built to match, and what a caller can observe is
//! compared (found child, request flags, the frames the display-only pass runs for, the final frame, the stack's values).

use super::*;
use mcla_formats::rage::swf::{Button, Character, Cmd, FontPage, Frame, Place, Sprite};

fn tagged(tag: &'static str) -> impl Iterator<Item = (Vec<String>, Vec<String>)> {
    include_str!("testdata/flash_inst_ref.txt").lines().filter_map(move |l| {
        let mut w = l.split_whitespace();
        if w.next() != Some(tag) {
            return None;
        }
        let rest: Vec<&str> = w.collect();
        let at = rest.iter().position(|x| *x == "->").unwrap();
        Some((rest[..at].iter().map(|s| s.to_string()).collect(), rest[at + 1..].iter().map(|s| s.to_string()).collect()))
    })
}

fn hx(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap()
}

/// a movie whose root has `n` frames; frame f holds one Place that modifies the empty depth 100 + f (each execution leaves a warning)
fn traced_movie(n: usize) -> Arc<Movie> {
    let frames = (0..n)
        .map(|f| Frame {
            tag: 0,
            cmds: vec![Cmd::Place(Place { depth: 100 + f as u16, clip_depth: 0, character: None, matrix: None, cxform: None, ratio: None, name: None, events: Vec::new() })],
        })
        .collect();
    Arc::new(Movie {
        stage: [0.0, 1280.0, 0.0, 720.0],
        fps: 30,
        root: Sprite { frames },
        chars: vec![None, Some(Character::Sprite(Sprite { frames: Vec::new() })), Some(Character::Button(Button { records: Vec::new(), conditions: 0, condition_actions: Vec::new() }))],
        exports: Vec::new(),
    })
}

/// F-75150: child by depth. Equal for every depth below 65535. At depth 65535 with the cache still in its initial state (depth 65535, no node)
/// the original starts the walk at the empty cache node and finds nothing; the port finds the child. (A finding, kept out of the port: the
/// port has no cache state; the row stays unverified.)
#[test]
fn child_at_matches_the_original_below_the_last_depth() {
    let (mut n, mut edge) = (0, 0);
    for (inp, out) in tagged("CH") {
        let cnt = hx(&inp[0]) as usize;
        let depths: Vec<u16> = inp[1..1 + cnt].iter().map(|s| hx(s) as u16).collect();
        let cache_none = inp[1 + cnt] == "ffffffff";
        let q = (hx(&inp[2 + cnt]) & 0xFFFF) as u16;
        let mut p = Player::new(traced_movie(0));
        let root = p.root;
        let ids: Vec<InstId> = depths.iter().map(|&d| p.alloc_inst(Inst::new(1, d, Some(root)))).collect();
        for (d, id) in depths.iter().zip(&ids) {
            p.inst_mut(root).unwrap().children.insert(*d, *id);
        }
        let want = out[0].parse::<i32>().unwrap();
        let got = p.child_at(root, q).map_or(-1, |id| ids.iter().position(|x| *x == id).unwrap() as i32);
        if got != want {
            assert!(q == 0xFFFF && cache_none && want == -1, "CH {inp:?}");
            edge += 1;
        }
        n += 1;
    }
    assert!(n > 1200 && edge > 0, "{n} {edge}");
}

/// F-38672: play() sets the request flag and a non-sprite forwards to its parent (through the parent's vtable).
#[test]
fn play_matches_the_original() {
    let mut n = 0;
    for (inp, out) in tagged("PL") {
        let cnt = hx(&inp[0]) as usize;
        let mut p = Player::new(traced_movie(0));
        let mut parent = None;
        let mut ids = Vec::new();
        for i in (0..cnt).rev() {
            let ch = if hx(&inp[1 + i]) == 2 { 1 } else { 2 };
            let id = p.alloc_inst(Inst::new(ch, i as u16 + 1, parent));
            ids.push(id);
            parent = Some(id);
        }
        ids.reverse();
        p.play(ids[0]);
        let got: Vec<String> = ids.iter().map(|&i| (p.inst(i).unwrap().play_req as u8).to_string()).collect();
        assert_eq!(got, out, "PL {inp:?}");
        n += 1;
    }
    assert_eq!(n, 120);
}

/// F-48063: after the frame: stop request clears playing, play request sets it, a playing clip steps to the next frame.
#[test]
fn sprite_step_flags_match_the_original() {
    let mut n = 0;
    for (inp, out) in tagged("SP") {
        let mut p = Player::new(traced_movie(2));
        let root = p.root;
        let i = p.inst_mut(root).unwrap();
        i.playing = hx(&inp[0]) != 0;
        i.play_req = hx(&inp[1]) != 0;
        i.stop_req = hx(&inp[2]) != 0;
        p.sprite_frame(root);
        let i = p.inst(root).unwrap();
        let got = [i.playing as u32, i.play_req as u32, i.stop_req as u32, i.frame as u32];
        assert_eq!(got, [hx(&out[0]), hx(&out[1]), hx(&out[2]), hx(&out[3])], "SP {inp:?}");
        n += 1;
    }
    assert_eq!(n, 8);
}

/// F-85946 (goto), F-41374 (nextFrame), F-90378 (prevFrame): the frames the display-only pass runs for, in order, the final frame, the children
/// reset at the wrap. The 'full' pass of the original (flag 1) has no counterpart in the port: those cases are skipped (the port's callers all
/// pass 0, as the original's callMethod and frame step do).
#[test]
fn goto_matches_the_original() {
    let (mut n, mut skipped) = (0, 0);
    for tag in ["GO", "NX", "PV"] {
        for (inp, out) in tagged(tag) {
            let (frames, cur) = (hx(&inp[0]) as usize, hx(&inp[1]) as u16);
            let (target, flag, kids) = if tag == "GO" { (hx(&inp[2]) as usize, hx(&inp[3]), hx(&inp[4])) } else { (0, 0, hx(&inp[2])) };
            if flag != 0 {
                skipped += 1;
                continue;
            }
            let mut p = Player::new(traced_movie(frames));
            let root = p.root;
            p.inst_mut(root).unwrap().frame = cur;
            p.inst_mut(root).unwrap().playing = false; // (the guest instance starts with the flag clear)
            let ks: Vec<InstId> = (0..kids)
                .map(|k| {
                    let id = p.alloc_inst(Inst::new(1, k as u16 + 1, Some(root)));
                    p.inst_mut(id).unwrap().placed_by = Some(7);
                    p.inst_mut(root).unwrap().children.insert(k as u16 + 1, id);
                    id
                })
                .collect();
            match tag {
                "GO" => p.goto(root, target),
                "NX" => {
                    p.call_method(Ref::Inst(root), "nextFrame", &[]);
                }
                _ => {
                    p.call_method(Ref::Inst(root), "prevFrame", &[]);
                }
            }
            let seq: Vec<String> = p
                .warnings
                .iter()
                .filter_map(|w| w.strip_prefix("PlaceObject modifies empty depth "))
                .map(|d| format!("d{:x}", d.parse::<u32>().unwrap() - 100))
                .collect();
            let sep: Vec<usize> = out.iter().enumerate().filter(|(_, w)| *w == "|").map(|(i, _)| i).collect();
            assert_eq!(hx(&out[0]), p.inst(root).unwrap().frame as u32, "{tag} {inp:?} final frame");
            assert_eq!(seq, out[sep[0] + 1..sep[1]].to_vec(), "{tag} {inp:?} frames run");
            let cleared = ks.iter().filter(|&&k| p.inst(k).unwrap().placed_by.is_none()).count() as u32;
            assert_eq!([cleared, p.inst(root).unwrap().playing as u32], [hx(&out[sep[1] + 1]), hx(&out[sep[1] + 2])], "{tag} {inp:?} children / playing");
            n += 1;
        }
    }
    assert!(n > 400 && skipped > 50, "{n} {skipped}");
}

/// F-23783: the first glyph page made for a height.
#[test]
fn glyph_page_matches_the_original() {
    let mut n = 0;
    for (inp, out) in tagged("GL") {
        let cnt = hx(&inp[0]);
        let pages: Vec<FontPage> = if cnt & 0x8000 != 0 {
            Vec::new()
        } else {
            inp[1..1 + cnt as usize].iter().map(|h| FontPage { height: hx(h) as u16 as i16, sheets: Vec::new(), glyphs: Vec::new() }).collect()
        };
        let q = hx(inp.last().unwrap()) as i32;
        let want = out[0].parse::<i32>().unwrap();
        // the original compares the signed page height with the 32 bit argument; the port's argument is the 16 bit height
        let got = match i16::try_from(q) {
            Ok(h) => crate::draw::glyph_page(&pages, h).map_or(-1, |i| i as i32),
            Err(_) => -1,
        };
        assert_eq!(got, want, "GL {inp:?}");
        n += 1;
    }
    assert_eq!(n, 300);
}

/// F-39219: the AVM1 value stack pop (LIFO; an empty stack gives integer 0).
#[test]
fn value_pop_matches_the_original() {
    let mut n = 0;
    for (inp, out) in tagged("PO") {
        let mut p = Player::new(traced_movie(0));
        let mut it = inp.iter();
        let mut got = Vec::new();
        while let Some(op) = it.next() {
            if op == "P" {
                let (w, t) = (hx(it.next().unwrap()), hx(it.next().unwrap()));
                p.stack.push(match t {
                    3 => Value::Int(w as i32),
                    4 => Value::Float(f32::from_bits(w)),
                    _ => Value::Bool(w >> 24 != 0),
                });
            } else {
                got.push(match p.pop() {
                    Value::Int(v) => format!("{:x}:3", v as u32),
                    Value::Float(v) => format!("{:x}:4", v.to_bits()),
                    Value::Bool(b) => format!("{}:2", b as u8),
                    v => panic!("{v:?}"),
                });
            }
        }
        // a boolean's word is its first byte in the original; compare the truth value
        let want: Vec<String> = out.iter().map(|w| if w.ends_with(":2") { format!("{}:2", (hx(&w[..w.len() - 2]) >> 24 != 0) as u8) } else { w.clone() }).collect();
        assert_eq!(got, want, "PO {inp:?}");
        n += 1;
    }
    assert_eq!(n, 200);
}

/// F-92070's clip-depth masks against the original's draw run offline (tools/draw_mask_reference.py -> testdata/flash_mask_ref.txt):
/// 3,000 random instance trees (masks one in four, alpha 0 one in twelve, depths near 65535 so the sums wrap); the original's
/// render-state calls read as mask begin (F-90239's states), mask end (F-71617's) and test off (5 = 3, 20 = 0), its owner draws as
/// the shapes drawn, against `draw::build`'s MaskBegin / MaskEnd / MaskOff and triangles, in order.
#[test]
fn masks_match_the_original() {
    use crate::draw::Item;
    use mcla_formats::rage::swf::{FillStyle, Matrix, Shape, ShapeCmd};
    let shape = |k: i32| {
        Some(Character::Shape(Shape {
            bounds: [0, 0, 0, 0],
            fills: vec![FillStyle { kind: 0, color: 0xff80_8080, matrix: Matrix::IDENTITY, bitmap: None, gradient: None }],
            lines: Vec::new(),
            cmds: vec![ShapeCmd::Fill(1), ShapeCmd::FanInt(vec![[k * 200, 0], [k * 200 + 100, 0], [k * 200, 100]])],
        }))
    };
    let mut chars = vec![None, Some(Character::Sprite(Sprite { frames: Vec::new() }))];
    chars.extend((0..40).map(shape));
    let movie = Arc::new(Movie { stage: [0.0, 1280.0, 0.0, 720.0], fps: 30, root: Sprite { frames: Vec::new() }, chars, exports: Vec::new() });
    let (mut n, mut masked, mut off, mut wrapped) = (0, 0, 0, 0);
    for line in include_str!("testdata/flash_mask_ref.txt").lines().filter(|l| l.starts_with("MK ")) {
        let (spec, log) = line.split_once(" -> ").unwrap();
        let w: Vec<&str> = spec.split_whitespace().collect();
        let count: usize = w[1].parse().unwrap();
        // (the guest keeps depths in 16 bits: a clip of 65,500 + .. is stored cut)
        let node = |i: usize| -> (u16, u16, u8, i32) { (w[2 + 4 * i].parse::<u32>().unwrap() as u16, w[3 + 4 * i].parse::<u32>().unwrap() as u16, w[4 + 4 * i].parse().unwrap(), w[5 + 4 * i].parse().unwrap()) };
        let has_kids: Vec<bool> = (0..count).map(|i| (0..count).any(|j| node(j).3 == i as i32)).collect();
        let mut p = Player::new(movie.clone());
        let mut ids = vec![p.root];
        for i in 1..count {
            let (d, _, _, parent) = node(i);
            let ch = if has_kids[i] { 1 } else { 2 + i as u16 };
            let id = p.alloc_inst(Inst::new(ch, d, Some(ids[parent as usize])));
            p.inst_mut(ids[parent as usize]).unwrap().children.insert(d, id);
            ids.push(id);
        }
        for i in 0..count {
            let (d, clip, alpha, _) = node(i);
            let inst = p.inst_mut(ids[i]).unwrap();
            inst.depth = d;
            inst.clip_depth = clip;
            inst.placed_by = Some(u64::MAX);
            if i == 0 && !has_kids[0] {
                inst.character = 2;
            }
            if alpha == 0 {
                inst.cxform.mult[3] = 0;
            }
            if d >= 65500 {
                wrapped += 1;
            }
        }
        // the original's log as tokens
        let s: Vec<&str> = log.split_whitespace().collect();
        let mut want = Vec::new();
        let mut i = 0;
        let st = |t: &str| -> (u32, u32) {
            let (a, b) = t[1..].split_once(',').unwrap();
            (a.parse().unwrap(), b.parse().unwrap())
        };
        while i < s.len() {
            if s[i].starts_with('D') {
                want.push(s[i].to_string());
                i += 1;
                continue;
            }
            let seq: Vec<(u32, u32)> = s[i..(i + 6).min(s.len())].iter().filter(|t| t.starts_with('S')).map(|t| st(t)).collect();
            let at = |k: usize| seq.get(k).copied().unwrap_or((99, 99));
            if at(0) == (16, 0) && at(1) == (5, 3) && at(2) == (23, 2) && at(3).0 == 25 && at(4) == (20, 1) && at(5) == (24, 1) {
                want.push("B".into());
                i += 6;
            } else if at(0) == (16, 15) && at(1) == (5, 5) && at(2) == (23, 0) && at(3).0 == 25 && at(4) == (20, 1) && at(5) == (24, 4) {
                want.push("E".into());
                i += 6;
            } else if at(0) == (5, 3) && at(1) == (20, 0) {
                want.push("O".into());
                i += 2;
            } else {
                panic!("unknown state calls at {i}: {line}");
            }
        }
        let list = crate::draw::build(&p);
        let mut got = Vec::new();
        for it in &list.items {
            match it {
                Item::MaskBegin => got.push("B".to_string()),
                Item::MaskEnd => got.push("E".to_string()),
                Item::MaskOff => got.push("O".to_string()),
                Item::Triangles { verts, .. } => {
                    for t in verts.chunks_exact(3) {
                        got.push(format!("D{}", (t[0].pos[0] / 10.0).round() as i32));
                    }
                }
            }
        }
        assert_eq!(got, want, "{line}");
        masked += want.iter().any(|t| t == "B") as usize;
        off += want.iter().any(|t| t == "O") as usize;
        n += 1;
    }
    assert!(n == 3000 && masked > 2000 && off > 1000 && wrapped > 100, "{n} {masked} {off} {wrapped}");
}

/// F-92070's world matrix against the original's draw run offline (the "WM" lines of testdata/flash_mask_ref.txt): 4,000 local and parent
/// matrices, floats of mixed size and raw bit patterns; `draw::world_matrix` bit for bit.
#[test]
fn world_matrix_matches_the_original() {
    use mcla_formats::rage::swf::Matrix;
    let mut n = 0;
    for line in include_str!("testdata/flash_mask_ref.txt").lines().filter(|l| l.starts_with("WM ")) {
        let w: Vec<u32> = line[3..].split_whitespace().filter(|t| *t != "->").map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
        let m = |o: usize| Matrix { a: f32::from_bits(w[o]), b: f32::from_bits(w[o + 1]), c: f32::from_bits(w[o + 2]), d: f32::from_bits(w[o + 3]), tx: f32::from_bits(w[o + 4]), ty: f32::from_bits(w[o + 5]) };
        let got = crate::draw::world_matrix(&m(6), &m(0));
        let got = [got.a, got.b, got.c, got.d, got.tx, got.ty].map(|f| f.to_bits());
        // (NaN results: any NaN; the original's payload is the PowerPC's)
        for k in 0..6 {
            let want = w[12 + k];
            assert!(got[k] == want || (f32::from_bits(got[k]).is_nan() && f32::from_bits(want).is_nan()), "{line}: field {k} {:08x}", got[k]);
        }
        n += 1;
    }
    assert_eq!(n, 4000);
}
