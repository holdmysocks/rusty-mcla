//! The player's object model against the original's own code run offline with the dispatch runner (tools/flash_*_reference.py ->
//! testdata/flash_*_ref.txt). The port keeps what the original keeps in hash buckets / linked lists in Vecs and maps, so the comparisons
//! replay the same random operation sequences and compare what the callers can observe.

use super::*;

/// F-86938 (find), F-86805 (insert), F-14453 (erase), F-99291 / F-52153 (set by name record / C string), F-69872 (bucket growth):
/// the member table. Sequences of set / get / erase by raw key and by name; every result equal.
#[test]
fn member_table_matches_the_original() {
    let mut seqs = 0;
    let mut ops_n = 0;
    for line in include_str!("testdata/flash_table_ref.txt").lines() {
        let (ops, res) = line.strip_prefix("SEQ ").unwrap().split_once(" ->").unwrap();
        let mut p = Player::new(Arc::new(Movie { stage: [0.0, 1280.0, 0.0, 720.0], fps: 30, root: mcla_formats::rage::swf::Sprite { frames: Vec::new() }, chars: Vec::new(), exports: Vec::new() }));
        let o = p.alloc(ObjKind::Script);
        let mut it = ops.split_whitespace();
        let mut got = Vec::new();
        let nm: Rc<str> = Rc::from("m");
        let hx = |s: &str| u32::from_str_radix(s, 16).unwrap();
        while let Some(op) = it.next() {
            ops_n += 1;
            match op {
                "S" => {
                    let (k, n) = (hx(it.next().unwrap()), hx(it.next().unwrap()));
                    mset(&mut p.obj_mut(o).unwrap().members, k, &nm, Value::Int(n as i32));
                    got.push("s".to_string());
                }
                "T" => {
                    let (t, n) = (it.next().unwrap(), hx(it.next().unwrap()));
                    p.set_member(Ref::Obj(o), &Value::str(t), Value::Int(n as i32));
                    got.push("t".to_string());
                }
                "G" | "U" => {
                    let k = if op == "G" { hx(it.next().unwrap()) } else { hash(it.next().unwrap()) };
                    got.push(match mget(&p.obj(o).unwrap().members, k) {
                        Some(Value::Int(v)) => format!("g{:x}:0:3", *v as u32),
                        Some(v) => panic!("{v:?}"),
                        None => "g-".to_string(),
                    });
                }
                "E" => {
                    let t = it.next().unwrap();
                    let had = mget(&p.obj(o).unwrap().members, hash(t)).is_some();
                    p.delete_member(Ref::Obj(o), &Value::str(t));
                    got.push(if had { "d1" } else { "d0" }.to_string());
                }
                _ => panic!("{op}"),
            }
        }
        assert_eq!(got.join(" "), res.trim(), "{line:.200}");
        seqs += 1;
    }
    assert!(seqs > 300 && ops_n > 20000, "{seqs} {ops_n}");
}

/// F-43980 (slot by name, created when missing), F-59146 / F-66095 / F-11028 / F-16600 (set int / float / bool / string), F-94547 (get int
/// with the value conversion): the host's accessors for the movie's globals. Both running-flag states of the context (tools/flash_global_reference.py).
#[test]
fn global_accessors_match_the_original() {
    let (mut cases, mut gets) = (0, 0);
    for line in include_str!("testdata/flash_global_ref.txt").lines() {
        let (ops, res) = line.split_once(" ->").unwrap();
        let mut p = Player::new(Arc::new(Movie { stage: [0.0, 1280.0, 0.0, 720.0], fps: 30, root: mcla_formats::rage::swf::Sprite { frames: Vec::new() }, chars: Vec::new(), exports: Vec::new() }));
        let mut it = ops.split_whitespace().skip(1);
        let mut got = Vec::new();
        let hx = |s: &str| u32::from_str_radix(s, 16).unwrap();
        while let Some(op) = it.next() {
            let name = it.next().unwrap();
            match op {
                "SI" => p.set_global(name, Value::Int(hx(it.next().unwrap()) as i32)),
                "SF" => p.set_global(name, Value::Float(f32::from_bits(hx(it.next().unwrap())))),
                "SB" => p.set_global(name, Value::Bool(hx(it.next().unwrap()) != 0)),
                "SS" => p.set_global(name, Value::str(it.next().unwrap())),
                _ => {
                    gets += 1;
                    got.push(match p.get_global(name) {
                        Some(v) => format!("g1:{:x}", v.to_i32() as u32),
                        None => "g0:0".to_string(),
                    });
                    continue;
                }
            }
            got.push("s".to_string());
        }
        assert_eq!(got.join(" "), res.trim(), "{line}");
        cases += 1;
    }
    assert!(cases == 500 && gets > 1000, "{cases} {gets}");
}

/// F-46400: strict equality of two values of one type (tools/flash_equals_reference.py). Not compared: the unnamed type 6 and booleans whose byte is
/// neither 0 nor 1 (the original compares the byte, the port's boolean is one bit). Two undefined values are not equal (the port said they were).
#[test]
fn strict_equals_matches_the_original() {
    let (mut n, mut skipped, mut undef) = (0, 0, 0);
    let hx = |s: &str| u32::from_str_radix(s, 16).unwrap();
    for line in include_str!("testdata/flash_equals_ref.txt").lines() {
        let (inp, out) = line.split_once(" -> ").unwrap();
        let w: Vec<&str> = inp.split_whitespace().skip(1).collect();
        let (ty, p1, t1, p2, t2) = (hx(w[0]), hx(w[1]), w[2], hx(w[3]), w[4]);
        fn txt(t: &str) -> &str {
            if t == "-" {
                ""
            } else {
                t
            }
        }
        let v = |p: u32, t: &str| -> Option<Value> {
            Some(match ty {
                0 | 1 => Value::str(txt(t)),
                2 if p >> 24 <= 1 => Value::Bool(p >> 24 != 0),
                3 => Value::Int(p as i32),
                4 => Value::Float(f32::from_bits(p)),
                5 => Value::Obj(Ref::Obj(p)),
                7 => Value::Undef,
                _ => return None,
            })
        };
        let (Some(a), Some(b)) = (v(p1, t1), v(p2, t2)) else {
            skipped += 1;
            continue;
        };
        undef += (ty == 7) as i32;
        assert_eq!(strict_equals(&a, &b) as u32, hx(out), "{line}");
        n += 1;
    }
    assert!(n > 600 && skipped > 20 && undef == 16, "{n} {skipped} {undef}");
}
