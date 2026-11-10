//! The opponent manager's frame as the original's: the round-robin racer update (F-28020), the queue of racers
//! deleted on eviction (F-39412), the racer streaming - which racers are loaded into the world, reloaded, evicted
//! or reset (F-31748) - and the racer's extent from its car's bound box (F-57218). Pure functions on plain data;
//! the rules and their marks: docs/reference/ai/opponent_manager.md. Checked against the original's code run
//! offline (work/racerai/oppmgr: build.py, main.cpp): test `opponent_manager_matches_the_original`.
//!
//! Not yet used by the game: vehicles.rs updates every car in list order each frame and main.rs drops the
//! starters not taken past 600 m (the integration is in the doc's "Port" part).
#![allow(dead_code)]

/// The racer's load state (mcRacer +656): 3 = in the world, 0 = out; anything else = loading / unloading.
pub const LOADED: u32 = 3;
pub const UNLOADED: u32 = 0;

/// mcRacerManager's distances (its constructor F-27782): +884 the reach while the map streamer is busy, +888 the
/// reach otherwise, +892 the eviction distance, +896 the loaded racer's head start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub near_reach: f32,
    pub far_reach: f32,
    pub evict: f32,
    pub loaded_bonus: f32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { near_reach: 200.0, far_reach: 400.0, evict: 600.0, loaded_bonus: 100.0 }
    }
}

/// The streamer's least pending request priority (F-66201 stores F-74461's minimum; 10000 when nothing is
/// pending) below which the near reach is used.
pub const BUSY_PRIORITY: f32 = 1000.0;
/// The slots: racers loaded at most, less one per racer for which racer vfunc 40 is true.
pub const SLOTS: i32 = 8;

/// The order of one frame's per-racer step over the opponent list (count = its length, start = its +4): slot
/// (start + i) mod count for i in 0..count (signed remainder; start may be at or past count). Each visited racer
/// first gets the delete-on-eviction check (`queues_delete`), then, unless the game is paused, its update (racer
/// vfunc 92, F-88453). Returns the order and the next start: (start + 1) mod count when not paused (0 for an empty
/// list), else start unchanged.
pub fn update_order(start: i32, count: usize, paused: bool) -> (Vec<usize>, i32) {
    let n = count as i32;
    let order = (0..n).map(|i| (start.wrapping_add(i) % n) as usize).collect();
    let next = if paused { start } else if n > 0 { start.wrapping_add(1) % n } else { 0 };
    (order, next)
}

/// The pause test of F-28020: the pause manager's +4 set and +10 clear.
pub fn paused(pause_flag: bool, pause_flag10: bool) -> bool {
    pause_flag && !pause_flag10
}

/// F-28020's check before the update: a racer not in the world (+656 != 3) marked DeleteOnEviction (+467).
pub fn queues_delete(load_state: u32, delete_on_eviction: bool) -> bool {
    load_state != LOADED && delete_on_eviction
}

/// What F-39412 does to a racer it queues (when it is not in the delete list already): racer vfunc 136(0), the
/// pin +660 = 2 (F-28017: forced out), the evict request +16 = 1 (F-86994), appended to the delete list (no
/// capacity check), +140 = -1 (F-15669). Returns false (nothing done) when the racer is in the list.
pub fn queue_delete<T: PartialEq + Copy>(list: &mut Vec<T>, racer: T) -> bool {
    if list.contains(&racer) {
        return false;
    }
    list.push(racer);
    true
}

/// One racer of mcRacerManager's list as the streaming walk sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamRacer {
    /// +656
    pub load_state: u32,
    /// +660: 1 keep in, 2 force out
    pub pin: u32,
    /// +468 ResetOnEviction
    pub reset_on_eviction: bool,
    /// racer vfunc 40: the racer takes a slot away and is not walked (the players)
    pub not_counted: bool,
    /// the racer's matrix position (F-35658 +48) x and z
    pub x: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StreamAction {
    /// "Reload car": load request +17 = 1 (F-25510), evict request +16 = 0 (F-18442); `rank` = the place in the
    /// sorted walk (the printed priority).
    Reload { index: usize, rank: usize },
    /// "Evict car": +16 = 1, +17 = 0.
    Evict { index: usize },
    /// racer vfunc 76 (its reset to its reset place) instead of an eviction.
    Reset { index: usize },
}

/// The horizontal distance as the original computes it (single precision, the z term fused).
pub fn flat_distance(x: f32, z: f32, rx: f32, rz: f32) -> f32 {
    let dx = x - rx;
    let dz = z - rz;
    let sq = dx * dx;
    let s = (dz as f64).mul_add(dz as f64, sq as f64) as f32;
    s.sqrt()
}

/// The walk's sort key: -1 while loading / unloading or pinned in, 10000 pinned out, a loaded racer
/// max(d - bonus, 0), an unloaded one d.
pub fn priority(r: &StreamRacer, rx: f32, rz: f32, lim: &Limits) -> f32 {
    let d = flat_distance(r.x, r.z, rx, rz);
    if r.load_state != LOADED && r.load_state != UNLOADED {
        -1.0
    } else if r.pin == 1 {
        -1.0
    } else if r.pin == 2 {
        10000.0
    } else if r.load_state == LOADED {
        let t = d - lim.loaded_bonus;
        if -t >= 0.0 { 0.0 } else { t }
    } else {
        d
    }
}

/// F-31748: mcRacerManager's streaming walk for one frame. `racers` = its list (None = an empty entry), (rx, rz) =
/// the reference position (the first player's matrix position when the player manager has one, else the view's
/// position: `reference`), `streamer_priority` = the map streamer's least pending request priority. Returns the
/// actions in the order the original takes them.
pub fn stream(racers: &[Option<StreamRacer>], rx: f32, rz: f32, lim: &Limits, streamer_priority: f32) -> Vec<StreamAction> {
    let mut slots = SLOTS;
    let mut busy = 0i32;
    let mut sorted: Vec<(f32, usize)> = Vec::new();
    for (i, r) in racers.iter().enumerate() {
        let Some(r) = r else { continue };
        if r.not_counted {
            slots -= 1;
            continue;
        }
        if r.load_state != LOADED && r.load_state != UNLOADED {
            busy += 1;
        }
        let d = priority(r, rx, rz, lim);
        // after the equal ones (the first k with d < key[k])
        let k = sorted.iter().position(|e| d < e.0).unwrap_or(sorted.len());
        sorted.insert(k, (d, i));
    }
    let reach = if streamer_priority < BUSY_PRIORITY { lim.near_reach } else { lim.far_reach };
    let mut out = Vec::new();
    for (j, &(d, i)) in sorted.iter().enumerate() {
        let r = racers[i].as_ref().unwrap();
        let ji = j as i32;
        if d < 0.0 || (d < reach && ji < slots) {
            // one reload a frame, none while any racer loads / unloads or after an eviction
            if r.load_state == UNLOADED && busy == 0 {
                out.push(StreamAction::Reload { index: i, rank: j });
                busy = 1;
            }
        } else if r.load_state == LOADED {
            if r.reset_on_eviction {
                out.push(StreamAction::Reset { index: i });
            } else if d > lim.evict || ji >= slots {
                out.push(StreamAction::Evict { index: i });
                busy += 1;
            }
        }
    }
    out
}

/// The streaming walk's reference position: the first player's when the player manager reports one, else the
/// current view's (x, z).
pub fn reference(player: Option<(f32, f32)>, view: (f32, f32)) -> (f32, f32) {
    player.unwrap_or(view)
}

/// F-57218's extent of a racer from its car's bound box (F-98453 +16 min, +32 max): +472 = 2 hx, +476 = 2 hz,
/// +480 = sqrt(hx^2 + hz^2) with hx / hz the greater magnitude of the box's least and greatest x / z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub width: f32,
    pub length: f32,
    pub radius: f32,
}

pub fn racer_extent(min: [f32; 3], max: [f32; 3]) -> Extent {
    let pick = |a: f32, b: f32| if b.abs() - a.abs() >= 0.0 { b.abs() } else { a.abs() };
    let hx = pick(min[0], max[0]);
    let hz = pick(min[2], max[2]);
    let zz = hz * hz;
    let radius = ((hx as f64).mul_add(hx as f64, zz as f64) as f32).sqrt();
    Extent { width: hx * 2.0, length: hz * 2.0, radius }
}

/// mcRacerManager's racer list: +0 the entries (null = an empty slot), +400 their count, and the frame's snapshot +404 /
/// +804 (taken at the frame's start by F-25724), which IndexOf searches while the thread block's byte +72 is set
/// (`updating`). Checked against the original run offline: test `racer_list_matches_the_original`.
#[derive(Debug, Clone, Default)]
pub struct RacerList<T> {
    pub list: Vec<Option<T>>,
    pub snap: Vec<Option<T>>,
    pub updating: bool,
}

impl<T: Copy + PartialEq> RacerList<T> {
    /// F-95558 (AddRacer): into the first empty slot; on the way a human racer (vfunc 40) takes the place of the first
    /// non-human one, which is carried on (humans first, each group in the order added); at the end appended.
    pub fn add(&mut self, racer: T, human: impl Fn(&T) -> bool) {
        let mut r = racer;
        for slot in self.list.iter_mut() {
            match *slot {
                None => {
                    *slot = Some(r);
                    return;
                }
                Some(e) => {
                    if human(&r) && !human(&e) {
                        *slot = Some(r);
                        r = e;
                    }
                }
            }
        }
        self.list.push(Some(r));
    }

    /// F-44187 (RemoveRacer): the first slot holding it (a null racer: the first empty slot) is emptied; the count stays.
    /// Returns false when it is not in the list - the original then writes 0 to the word before the list (index -1).
    pub fn remove(&mut self, racer: Option<T>) -> bool {
        match self.list.iter().position(|e| *e == racer) {
            Some(i) => {
                self.list[i] = None;
                true
            }
            None => false,
        }
    }

    /// F-80858 (IndexOf): -1 for a null racer or one not in the list (the snapshot while `updating`).
    pub fn index_of(&self, racer: Option<T>) -> i32 {
        let Some(r) = racer else { return -1 };
        let l = if self.updating { &self.snap } else { &self.list };
        l.iter().position(|e| *e == Some(r)).map_or(-1, |i| i as i32)
    }

    /// F-52302: the racer to slot `to`, swapping with what is there; the list grows with empty slots to reach it. Its own
    /// index is F-80858's (the snapshot's while `updating`), the swap is in the list.
    pub fn move_to(&mut self, racer: Option<T>, to: i32) {
        let i = self.index_of(racer);
        if i < 0 || to < 0 {
            return;
        }
        let to = to as usize;
        while self.list.len() <= to {
            self.list.push(None);
        }
        self.list[i as usize] = self.list[to];
        self.list[to] = racer;
    }

    /// F-25724's copy of the list into the snapshot.
    pub fn snapshot(&mut self) {
        self.snap = self.list.clone();
    }
}

/// The order of the race's racer list after AddRacer of each of `ids` in turn (`human`: vfunc 40).
pub fn racer_order(ids: &[usize], human: impl Fn(usize) -> bool) -> Vec<usize> {
    let mut l = RacerList::default();
    for &i in ids {
        l.add(i, |&j| human(j));
    }
    l.list.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-95558, F-44187, F-80858 and F-52302 run offline on 4,000 random op sequences (tools/racer_list_reference.py,
    /// verify_racer_list.md): after every op the list's count and entries, every IndexOf, and whether an absent racer
    /// was removed (the original's stray write) equal RacerList's.
    #[test]
    fn racer_list_matches_the_original() {
        let text = include_str!("testdata/racer_list_ref.txt");
        let hex = |w: &str| u32::from_str_radix(&w.replace('_', ""), 16).unwrap();
        let (mut cases, mut ops, mut bad) = (0, 0, 0);
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let arrow = w.iter().position(|x| *x == "->").unwrap();
            let v: Vec<u32> = w[1..arrow].iter().map(|x| hex(x)).collect();
            let o: Vec<u32> = w[arrow + 1..].iter().map(|x| hex(x)).collect();
            let humans = v[0];
            let human = |r: &u32| (humans >> r) & 1 != 0;
            let rac = |k: u32| if k < 7 { Some(k) } else { None };
            let mut l: RacerList<u32> = RacerList::default();
            let mut stray = false;
            let mut got = Vec::new();
            for i in 0..v[1] as usize {
                let (op, k, b) = (v[2 + 3 * i], v[3 + 3 * i], v[4 + 3 * i]);
                let mut r = 0;
                match op {
                    0 => l.add(k, human),
                    1 => stray |= !l.remove(rac(k)),
                    2 => l.move_to(rac(k), b as i32),
                    3 => r = l.index_of(rac(k)) as u32,
                    4 => l.snapshot(),
                    _ => l.updating = b != 0,
                }
                let h = l.list.iter().map(|e| e.unwrap_or(7)).fold(2_166_136_261u32, |h, x| x.to_le_bytes().iter().fold(h, |h, b| (h ^ *b as u32).wrapping_mul(16_777_619)));
                got.extend([r, l.list.len() as u32, h]);
                ops += 1;
            }
            got.push(if stray { 0 } else { 0x1234_5678 });
            cases += 1;
            if got != o {
                if bad < 5 {
                    println!("{line}{}  port {:08x?}", char::from(10), got);
                }
                bad += 1;
            }
        }
        println!("racer list: {cases} cases, {ops} ops, {bad} differ");
        assert!(cases >= 4000);
        assert_eq!(bad, 0);
    }

    fn words(s: &str) -> Vec<u32> {
        s.split_whitespace().map(|t| u32::from_str_radix(t.trim_start_matches('h'), 16).unwrap()).collect()
    }
    fn fl(w: u32) -> f32 {
        f32::from_bits(w)
    }

    /// One harness case of the manager's frame: F-28020 with F-39412 and F-31748 run on synthetic racers.
    fn frame_case(line: &str) -> bool {
        let (a, b) = line[1..].split_once('|').unwrap();
        let (a, b) = (words(a), words(b));
        let (start, oc, mc, nr, dc) = (a[2] as i32, a[3] as usize, a[4] as usize, a[6] as usize, a[7] as usize);
        let lim = Limits { near_reach: fl(a[9]), far_reach: fl(a[10]), evict: fl(a[11]), loaded_bonus: fl(a[12]) };
        let (gl, rx, rz) = (fl(a[8]), fl(a[13]), fl(a[14]));
        let mut p = 15;
        struct R { st: u32, pin: u32, del: bool, rst: bool, f40: bool, x: f32, z: f32, b16: u32, b17: u32, w140: u32 }
        let mut rs: Vec<R> = (0..nr).map(|_| {
            let v = &a[p..p + 10];
            p += 10;
            R { st: v[0], pin: v[1], del: v[2] != 0, rst: v[3] != 0, f40: v[4] != 0, x: fl(v[5]), z: fl(v[6]), b16: v[7], b17: v[8], w140: v[9] }
        }).collect();
        let olist: Vec<usize> = a[p..p + oc].iter().map(|&w| w as usize).collect();
        p += oc;
        let mlist: Vec<Option<usize>> = a[p..p + mc].iter().map(|&w| if w == 0xFFFF { None } else { Some(w as usize) }).collect();
        p += mc;
        let mut dl: Vec<usize> = a[p..p + dc].iter().map(|&w| w as usize).collect();
        let mut ev: Vec<(u32, u32, Option<u32>)> = Vec::new();
        let pz = paused(a[0] != 0, a[1] != 0);
        let (order, next) = update_order(start, oc, pz);
        for k in order {
            let r = olist[k];
            if queues_delete(rs[r].st, rs[r].del) && queue_delete(&mut dl, r) {
                ev.push((136, r as u32, Some(0)));
                rs[r].pin = 2;
                rs[r].b16 = 1;
                rs[r].w140 = 0xFFFF_FFFF;
            }
            if !pz {
                ev.push((92, r as u32, None));
            }
        }
        let sr: Vec<Option<StreamRacer>> = mlist.iter().map(|m| m.map(|r| {
            let q = &rs[r];
            StreamRacer { load_state: q.st, pin: q.pin, reset_on_eviction: q.rst, not_counted: q.f40, x: q.x, z: q.z }
        })).collect();
        for act in stream(&sr, rx, rz, &lim, gl) {
            match act {
                StreamAction::Reload { index, rank } => {
                    let r = mlist[index].unwrap();
                    rs[r].b17 = 1;
                    rs[r].b16 = 0;
                    ev.push((202, index as u32, Some(rank as u32)));
                }
                StreamAction::Evict { index } => {
                    let r = mlist[index].unwrap();
                    rs[r].b16 = 1;
                    rs[r].b17 = 0;
                    ev.push((201, index as u32, None));
                }
                StreamAction::Reset { index } => ev.push((76, mlist[index].unwrap() as u32, None)),
            }
        }
        let mut out = vec![next as u32];
        for q in &rs {
            out.extend([q.b16, q.b17, q.pin, q.w140]);
        }
        out.push(dl.len() as u32);
        out.extend(dl.iter().map(|&r| r as u32));
        let q = 1 + 4 * nr;
        let head = q + 1 + b[q] as usize;
        let n = b[head] as usize;
        let got = &b[head + 1..];
        out[..] == b[..head]
            && n == ev.len()
            && ev.iter().enumerate().all(|(i, e)| got[3 * i] == e.0 && got[3 * i + 1] == e.1 && e.2.map_or(true, |x| got[3 * i + 2] == x))
    }

    /// One harness case of F-57218: the extent words and the car's two flag bytes (+597 / +598 from the racer's
    /// +689 / +690, only when the car's instance pointer +12 less 16 is not null).
    fn extent_case(line: &str) -> bool {
        let (a, b) = line[1..].split_once('|').unwrap();
        let (a, b) = (words(a), words(b));
        let e = racer_extent([fl(a[0]), fl(a[1]), fl(a[2])], [fl(a[3]), fl(a[4]), fl(a[5])]);
        let bytes = if a[6] >= 3 { (a[8], a[9]) } else { (0, 0) };
        [e.width.to_bits(), e.length.to_bits(), e.radius.to_bits(), bytes.0, bytes.1] == b[..5]
    }

    fn run(text: &str, f: fn(&str) -> bool) -> (usize, usize) {
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        let bad: Vec<usize> = (0..lines.len()).filter(|&i| !f(lines[i])).collect();
        (lines.len() - bad.len(), lines.len())
    }

    #[test]
    fn opponent_manager_matches_the_original() {
        for (text, f, name) in [
            (include_str!("testdata/oppmgr_frame_ref.txt"), frame_case as fn(&str) -> bool, "frame"),
            (include_str!("testdata/oppmgr_extent_ref.txt"), extent_case, "extent"),
        ] {
            let (ok, n) = run(text, f);
            assert!(n >= 400 && ok == n, "{name}: {ok} of {n}");
        }
    }

    /// The 20,000-case sets from the harness, when present.
    #[test]
    fn opponent_manager_matches_the_original_20000() {
        for (path, f) in [
            ("D:/MCLA/work/racerai/oppmgr/frame_big.txt", frame_case as fn(&str) -> bool),
            ("D:/MCLA/work/racerai/oppmgr/extent_big.txt", extent_case),
        ] {
            let Ok(text) = std::fs::read_to_string(path) else { return };
            let (ok, n) = run(&text, f);
            assert!(ok == n, "{path}: {ok} of {n}");
            eprintln!("{path}: {ok} of {n}");
        }
    }
}
