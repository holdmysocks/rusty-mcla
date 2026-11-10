//! The parked spawner F-74446 against the original's code run offline (work/traffic-detail/park/off: build.py,
//! main_park.cpp, mk.sh): the stubs' calls in order (the node count F-48973, the type's slot 7 and the seen test
//! F-92805, the request / placement / submit, the item's type and F-94309) and the generator after, from
//! [`park_spawn_core`] with the type draw [`crate::ambient::draw_index`] (F-71916 / F-36934).

use super::*;

struct Spot {
    t8: u32,
    t12: i32,
    cruise: bool,
    x: u32,
    z: u32,
}

struct Item {
    u76: u32,
    i80: i32,
    path: bool,
    p188: bool,
    p171: bool,
    p189: bool,
    tyf: bool,
    aok: bool,
}

struct List {
    first_spot: usize,
    first_item: usize,
    taken: i32,
    free: i32,
}

struct Env<'a> {
    spots: &'a [Spot],
    items: &'a [Item],
    lists: &'a [List],
    w: Vec<f32>,
    total: f32,
    dty: Vec<i32>,
    types: Vec<(u32, bool, u32)>,
    event: u32,
    seenmode: u32,
    slot7: u32,
    log: String,
}

impl ParkEnv for Env<'_> {
    fn spot_taken(&self, l: usize, e: usize) -> bool {
        let s = &self.spots[self.lists[l].first_spot + e];
        s.t8 != 0 || s.t12 >= 0
    }
    fn spot_cruise_only(&self, l: usize, e: usize) -> bool {
        self.spots[self.lists[l].first_spot + e].cruise
    }
    fn spot_xz(&self, l: usize, e: usize) -> (f32, f32) {
        let s = &self.spots[self.lists[l].first_spot + e];
        (f32::from_bits(s.x), f32::from_bits(s.z))
    }
    fn node_count(&mut self, l: usize) -> (i32, i32) {
        self.log += &format!(" N{l},{}", self.event);
        let li = &self.lists[l];
        (li.taken, li.taken + li.free)
    }
    fn type_draw(&mut self, rng: &mut Mwc) -> Option<usize> {
        let k = crate::ambient::draw_index(&self.w, self.total, rng)?;
        (self.dty[k] >= 0).then_some(self.dty[k] as usize)
    }
    fn type_ok(&mut self, ty: usize) -> bool {
        let (b4, taxi, grp) = self.types[ty];
        grp != 2 && b4 != 0 && !taxi
    }
    fn seen(&mut self, l: usize, e: usize, ty: usize) -> bool {
        let s = &self.spots[self.lists[l].first_spot + e];
        self.log += &format!(" V{ty} S{:08x},{:08x},{:08x},{}", s.x, s.z, 1.5f32.to_bits(), self.slot7);
        if self.seenmode == 2 { (s.x >> 4) & 1 != 0 } else { self.seenmode != 0 }
    }
    fn item_taken(&self, l: usize, k: usize) -> bool {
        let i = &self.items[self.lists[l].first_item + k];
        i.u76 != 0 || i.i80 >= 0
    }
    fn item_held(&self, l: usize, k: usize) -> bool {
        let i = &self.items[self.lists[l].first_item + k];
        i.path && i.p188 && (i.p171 || i.p189)
    }
}

#[test]
fn park_spawn_matches_the_original() {
    let text = include_str!("testdata/park_spawn_ref.txt");
    let (mut cases, mut spots_made, mut items_made, mut seen_calls) = (0, 0, 0, 0);
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let parts: Vec<&str> = line.split(" |").collect();
        let head: Vec<&str> = parts[0].split_whitespace().collect();
        let h = |i: usize| u32::from_str_radix(head[i], 16).unwrap();
        let d = |i: usize| head[i].parse::<i32>().unwrap();
        let (net, en, glob, nl, tries, event) = (h(0), h(1), h(2), d(3) as usize, d(4), h(5));
        let (density, radius, fx, fz, m972) = (f32::from_bits(h(6)), f32::from_bits(h(7)), f32::from_bits(h(8)), f32::from_bits(h(9)), h(10));
        let mut rng = Mwc { lo: h(11), carry: h(12) };
        let (seenmode, slot7) = (h(13), h(14));
        // the lists, their spots and items
        let mut t = parts[1].split_whitespace();
        let mut nx = || t.next().unwrap();
        let (mut spots, mut items, mut lists, mut views) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for _ in 0..nl {
            let (ns, ni): (usize, usize) = (nx().parse().unwrap(), nx().parse().unwrap());
            let (taken, free) = (nx().parse().unwrap(), nx().parse().unwrap());
            lists.push(List { first_spot: spots.len(), first_item: items.len(), taken, free });
            views.push(ParkListView { spots: ns, items: ni });
            for _ in 0..ns {
                spots.push(Spot { t8: u32::from_str_radix(nx(), 16).unwrap(), t12: nx().parse().unwrap(), cruise: nx() == "1", x: u32::from_str_radix(nx(), 16).unwrap(), z: u32::from_str_radix(nx(), 16).unwrap() });
            }
            for _ in 0..ni {
                let mut b = || nx() != "0";
                let u76 = b() as u32;
                let i80: i32 = nx().parse().unwrap();
                let mut b = || nx() != "0";
                items.push(Item { u76, i80, path: b(), p188: b(), p171: b(), p189: b(), tyf: b(), aok: b() });
            }
        }
        let dist: Vec<&str> = parts[2].split_whitespace().collect();
        let nt: usize = dist[0].parse().unwrap();
        let total = f32::from_bits(u32::from_str_radix(dist[1], 16).unwrap());
        let w: Vec<f32> = (0..nt).map(|k| f32::from_bits(u32::from_str_radix(dist[2 + 2 * k], 16).unwrap())).collect();
        let dty: Vec<i32> = (0..nt).map(|k| dist[3 + 2 * k].parse().unwrap()).collect();
        let ty: Vec<&str> = parts[3].split_whitespace().collect();
        let types: Vec<(u32, bool, u32)> = (0..5).map(|k| (u32::from_str_radix(ty[1 + 3 * k], 16).unwrap(), ty[2 + 3 * k] == "1", ty[3 + 3 * k].parse().unwrap())).collect();
        let want_log = parts[4].trim_end();
        let tail: Vec<&str> = parts[5].split_whitespace().collect();
        let want_rng = (u32::from_str_radix(tail[0], 16).unwrap(), u32::from_str_radix(tail[1], 16).unwrap());

        let mut env = Env { spots: &spots, items: &items, lists: &lists, w, total, dty, types, event, seenmode, slot7, log: String::new() };
        if net == 0 && en != 0 && glob != 0 {
            match park_spawn_core(&mut env, &mut rng, &views, tries, event != 0, density, (fx, fz), radius) {
                None => {}
                Some(ParkTry::Spot { list, entry, ty, colour }) => {
                    spots_made += 1;
                    env.log += &format!(" Q P{ty},{},{colour},{m972:08x},{:08x} B", lists[list].first_spot + entry, 1.0f32.to_bits());
                }
                Some(ParkTry::Item { list, k }) => {
                    items_made += 1;
                    let g = lists[list].first_item + k;
                    let it = &items[g];
                    env.log += &format!(" T{g}");
                    if it.tyf {
                        let path = if it.path { 0x170000 + 0x100 * g as u32 } else { 0 };
                        env.log += &format!(" D{path:08x} A{g},{m972:08x}");
                        if !it.aok {
                            env.log += " X";
                        }
                    }
                }
            }
        }
        seen_calls += env.log.matches(" S").count();
        assert_eq!(env.log, want_log, "case {cases}: {line}");
        assert_eq!((rng.lo, rng.carry), want_rng, "case {cases} generator");
        cases += 1;
    }
    assert!(cases >= 4000 && spots_made > 500 && items_made > 500 && seen_calls > 500, "{cases} {spots_made} {items_made} {seen_calls}");
}
