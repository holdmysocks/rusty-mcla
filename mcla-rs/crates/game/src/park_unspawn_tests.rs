//! The parked unspawner F-53207 against the original's code run offline (work/traffic-detail/unspawn/off: build.py,
//! main_unspawn.cpp, mk.sh): the stubs' calls in order (the pool entry F-50439, the node count F-48973, the type's
//! slot 6, the removal F-79896 with its reason) and the generator after, from [`park_unspawn_core`].

use super::*;

struct Ent {
    null: bool,
    id: i32,
    b500: u32,
    item: i32,
    spot: i32,
    born: f32,
    life: f32,
    x: f32,
    z: f32,
    keep: bool,
}

struct Env {
    n: [u32; 2],
    ents: Vec<Option<Ent>>,
    nodes: Vec<(i32, i32)>,
    event: u32,
    clock: f32,
    sess: bool,
    me: u32,
    log: String,
}

impl Env {
    fn e(&self, h: usize) -> &Ent {
        self.ents[h].as_ref().unwrap()
    }
}

impl UnspawnEnv for Env {
    fn pool_len(&self, pool: u8) -> u32 {
        self.n[(pool / 2) as usize]
    }
    fn get(&self, pool: u8, i: u32) -> Option<usize> {
        // (the stub logs here; the log is written by the test's own wrapper below)
        let h = i as usize + if pool == 2 { 64 } else { 0 };
        (!self.e(h).null).then_some(h)
    }
    fn net_other(&self, _pool: u8, h: usize) -> bool {
        let id = self.e(h).id;
        id >= 0 && (id & 31) as u32 != if self.sess { self.me } else { 0 }
    }
    fn removing(&self, _pool: u8, h: usize) -> bool {
        self.e(h).b500 & 4 != 0
    }
    fn spot_node(&self, h: usize) -> Option<u32> {
        let s = self.e(h).spot;
        (s >= 0).then_some(s as u32)
    }
    fn item_node(&self, _pool: u8, h: usize) -> Option<u32> {
        let s = self.e(h).item;
        (s >= 0).then_some(s as u32)
    }
    fn age(&self, _pool: u8, h: usize) -> (f32, f32) {
        let e = self.e(h);
        (self.clock - e.born, e.life)
    }
    fn xz(&self, _pool: u8, h: usize) -> (f32, f32) {
        (self.e(h).x, self.e(h).z)
    }
    fn node_count(&mut self, node: u32) -> (i32, i32) {
        self.log += &format!(" N{node},{}", self.event);
        let (t, f) = self.nodes[node as usize];
        (t, t + f)
    }
    fn type_slot6(&mut self, _pool: u8, h: usize) -> bool {
        self.log += &format!(" K{h}");
        self.e(h).keep
    }
}

/// Logs the pool entry calls as the stub does (F-50439 is called with the drawn index, whatever it returns).
struct Logged<'a>(&'a mut Env);

impl UnspawnEnv for Logged<'_> {
    fn pool_len(&self, pool: u8) -> u32 {
        self.0.pool_len(pool)
    }
    fn get(&self, pool: u8, i: u32) -> Option<usize> {
        // interior log: the trait takes &self, so the entry calls are counted through a cell
        GETS.with(|g| g.borrow_mut().push_str(&format!(" G{pool},{i}")));
        self.0.get(pool, i)
    }
    fn net_other(&self, pool: u8, h: usize) -> bool {
        self.0.net_other(pool, h)
    }
    fn removing(&self, pool: u8, h: usize) -> bool {
        self.0.removing(pool, h)
    }
    fn spot_node(&self, h: usize) -> Option<u32> {
        self.0.spot_node(h)
    }
    fn item_node(&self, pool: u8, h: usize) -> Option<u32> {
        self.0.item_node(pool, h)
    }
    fn age(&self, pool: u8, h: usize) -> (f32, f32) {
        self.0.age(pool, h)
    }
    fn xz(&self, pool: u8, h: usize) -> (f32, f32) {
        self.0.xz(pool, h)
    }
    fn node_count(&mut self, node: u32) -> (i32, i32) {
        GETS.with(|g| {
            let s = std::mem::take(&mut *g.borrow_mut());
            self.0.log += &s;
        });
        self.0.node_count(node)
    }
    fn type_slot6(&mut self, pool: u8, h: usize) -> bool {
        GETS.with(|g| {
            let s = std::mem::take(&mut *g.borrow_mut());
            self.0.log += &s;
        });
        self.0.type_slot6(pool, h)
    }
}

thread_local! {
    static GETS: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

#[test]
fn park_unspawn_matches_the_original() {
    let text = include_str!("testdata/park_unspawn_ref.txt");
    let (mut cases, mut removed, mut nine, mut counts) = (0, 0, 0, 0);
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let parts: Vec<&str> = line.split(" |").collect();
        let head: Vec<&str> = parts[0].split_whitespace().collect();
        let h = |i: usize| f32::from_bits(u32::from_str_radix(head[i], 16).unwrap());
        let d = |i: usize| head[i].parse::<i64>().unwrap();
        let (tries, event) = (d(0) as i32, d(1) as u32);
        let (radius, density, fx, fz, clock) = (h(2), h(3), h(4), h(5), h(6));
        let mut rng = Mwc { lo: d(7) as u32, carry: d(8) as u32 };
        let (sess, me, n0, n2) = (d(9) != 0, d(10) as u32, d(11) as u32, d(12) as u32);
        let l: Vec<u32> = parts[1].split_whitespace().map(|w| w.parse().unwrap()).collect();
        let lists = &l[1..];
        let nd: Vec<i32> = parts[2].split_whitespace().map(|w| w.parse().unwrap()).collect();
        let nodes: Vec<(i32, i32)> = nd.chunks(2).map(|c| (c[0], c[1])).collect();
        let mut t = parts[3].split_whitespace();
        let mut ents: Vec<Option<Ent>> = (0..128).map(|_| None).collect();
        for k in (0..n0 as usize).chain((64..64 + n2 as usize).filter(|_| true)) {
            let mut nx = || t.next().unwrap();
            let f = |w: &str| f32::from_bits(u32::from_str_radix(w, 16).unwrap());
            let null = nx() == "1";
            let id: i32 = nx().parse().unwrap();
            let b500: u32 = nx().parse().unwrap();
            let item: i32 = nx().parse().unwrap();
            let spot: i32 = nx().parse().unwrap();
            let (born, life, x, z) = (f(nx()), f(nx()), f(nx()), f(nx()));
            let keep = nx() == "1";
            ents[k] = Some(Ent { null, id, b500, item, spot, born, life, x, z, keep });
        }
        let want_log = parts[4].trim_end();
        let tail: Vec<&str> = parts[5].split_whitespace().collect();
        let want_rng: (u32, u32) = (tail[0].parse().unwrap(), tail[1].parse().unwrap());

        let mut env = Env { n: [n0, n2], ents, nodes, event, clock, sess, me, log: String::new() };
        GETS.with(|g| g.borrow_mut().clear());
        let got = park_unspawn_core(&mut Logged(&mut env), &mut rng, tries, lists, (fx, fz), radius, density);
        GETS.with(|g| {
            let s = std::mem::take(&mut *g.borrow_mut());
            env.log += &s;
        });
        if let Some(u) = got {
            env.log += &format!(" R{},{}", u.h, u.reason);
            removed += 1;
            nine += (u.reason == 9) as i32;
        }
        counts += env.log.matches(" N").count();
        assert_eq!(env.log, want_log, "case {cases}: {line}");
        assert_eq!((rng.lo, rng.carry), want_rng, "case {cases} generator");
        cases += 1;
    }
    assert!(cases >= 4000 && removed > 1500 && nine > 100 && counts > 300, "{cases} {removed} {nine} {counts}");
}
