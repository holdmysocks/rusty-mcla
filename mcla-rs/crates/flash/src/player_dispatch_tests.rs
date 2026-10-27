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
