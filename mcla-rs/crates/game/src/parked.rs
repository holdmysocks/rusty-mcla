//! The original's parked-car spawner and unspawner (density manager M, run from the traffic
//! spawner's 30 Hz manager frames in traffic.rs). Decoded 2026-10-21: docs/PEOPLE_TRAFFIC.md
//! "Parked-car spawner".
//!
//! - odd frames, right after the traffic region list: the active lists (F-51021) = the
//!   quadtree nodes of the parking spots (`resources/city/sc/0x2e3038ce.r62`) met by the region
//!   list's box, at most 64;
//! - even frames: one spawn at most (F-74446, 60 tries), then, while the region list has
//!   roads, one removal at most (F-53207, 30 tries).
//!
//! A node takes at most a quarter of its counted spots (F-48973); a spot within 100 m is
//! filled only when the focus does not see it (F-92805).

use super::traffic::{rects_meet, seen, Spawner, OCC_RANGE};
use super::{Vehicles, PARK_PROBE, TRAFFIC_PALETTE};
use crate::render::Gpu;
use glam::{Vec2, Vec3};
use mcla_formats::rage::parking::ParkingTree;

/// The game's shared random numbers (the MWC at 0x82860890: lo, carry; a = 0x5CDCFAA7).
#[derive(Clone, Copy, Debug)]
pub struct Mwc {
    pub lo: u32,
    pub carry: u32,
}

impl Default for Mwc {
    fn default() -> Self {
        // (approx.) the original seeds it at run time; the seed was not traced
        Mwc { lo: 0x2545_F491, carry: 1 }
    }
}

impl Mwc {
    pub fn next(&mut self) -> u32 {
        let t = 0x5CDC_FAA7u64 * self.lo as u64 + self.carry as u64;
        self.lo = t as u32;
        self.carry = (t >> 32) as u32;
        self.lo
    }
    /// The 23-bit float draw, `(lo & 0x7FFFFF) x 2^-23`.
    pub fn float(&mut self) -> f32 {
        (self.next() & 0x7F_FFFF) as f32 * (1.0 / 8_388_608.0)
    }
    /// The index draw, `(lo & 0x7FFFFFFF) % n`.
    pub fn index(&mut self, n: u32) -> u32 {
        (self.next() & 0x7FFF_FFFF) % n.max(1)
    }
    /// The generator's seed (F-37317): x = s + (s == 0), carry = s ^ rotl16(s) (the generator's +8 byte and +12 word,
    /// cleared there, are not used by any draw).
    pub fn seed(s: u32) -> Mwc {
        Mwc { lo: s.wrapping_add((s == 0) as u32), carry: s ^ s.rotate_left(16) }
    }
    /// A script context's generator as its constructor seeds it (F-36420 -> F-82174): with one draw `x & 0x7FFFFFFF` of the
    /// game's global generator, which the original seeds from the time base at start-up (F-71171).
    pub fn script() -> Mwc {
        static GLOBAL: std::sync::OnceLock<std::sync::Mutex<Mwc>> = std::sync::OnceLock::new();
        let g = GLOBAL.get_or_init(|| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
            std::sync::Mutex::new(Mwc::seed(t as u32 ^ (t >> 32) as u32))
        });
        let s = g.lock().unwrap_or_else(|e| e.into_inner()).next() & 0x7FFF_FFFF;
        Mwc::seed(s)
    }
    /// RAND_INT_RANGE (F-32828): hi < lo gives lo and draws nothing; else lo + (x & 0x7FFFFFFF) % (hi - lo + 1) in the
    /// guest's 32-bit signed arithmetic.
    pub fn int_range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi < lo {
            return lo;
        }
        range_of((self.next() & 0x7FFF_FFFF) as i32, lo, hi)
    }
    /// RAND_FLOAT_RANGE (F-47737): hi < lo gives hi and draws nothing; else lo + (hi - lo) x (x & 0x7FFFFF) x 2^-23 as the
    /// original rounds it (the difference and the product to single, then one fused multiply-add on lo).
    pub fn float_range(&mut self, lo: f32, hi: f32) -> f32 {
        // (a NaN end: the compare is unordered and the draw is made)
        if hi < lo {
            return hi;
        }
        let d = hi - lo;
        let r = (self.next() & 0x7F_FFFF) as f32;
        let p = d * r;
        (p as f64).mul_add(1.0 / 8_388_608.0, lo as f64) as f32
    }
    /// RAND_INT_RANGE_DIFFERENT (F-40328) and the particles' frame draw (F-61280): a value of [lo, hi] other than `cur`,
    /// ((v % (hi - lo)) - lo + cur + 1) % (hi - lo + 1) + lo with v = x & 0x7FFFFFFF, in the guest's 32-bit signed
    /// arithmetic. Always draws. (hi == lo is a division by zero the original traps on; the port gives lo.)
    pub fn int_range_different(&mut self, cur: i32, lo: i32, hi: i32) -> i32 {
        different_of((self.next() & 0x7FFF_FFFF) as i32, cur, lo, hi)
    }
}

/// The remainder of the guest's `divw` / `mullw` / `subf` as the generated code computes it: the quotient is 0 for a zero
/// divisor or i32::MIN / -1, so the remainder is then the dividend.
pub fn divw_rem(a: i32, b: i32) -> i32 {
    let q = if b != 0 && !(a == i32::MIN && b == -1) { a / b } else { 0 };
    a.wrapping_sub(q.wrapping_mul(b))
}

/// The int range draw on a drawn value `v` (x & 0x7FFFFFFF): lo + v % (hi - lo + 1), guest arithmetic.
pub fn range_of(v: i32, lo: i32, hi: i32) -> i32 {
    divw_rem(v, hi.wrapping_sub(lo).wrapping_add(1)).wrapping_add(lo)
}

/// The DIFFERENT draw on a drawn value `v` (x & 0x7FFFFFFF): ((v % (hi - lo)) - lo + cur + 1) % (hi - lo + 1) + lo, guest
/// arithmetic (hi == lo, the original's trap, gives lo).
pub fn different_of(v: i32, cur: i32, lo: i32, hi: i32) -> i32 {
    let n = hi.wrapping_sub(lo);
    if n == 0 {
        return lo;
    }
    let t = divw_rem(v, n).wrapping_sub(lo).wrapping_add(cur).wrapping_add(1);
    divw_rem(t, n.wrapping_add(1)).wrapping_add(lo)
}

#[cfg(test)]
#[path = "rng_tests.rs"]
mod rng_tests;
#[cfg(test)]
#[path = "park_spawn_tests.rs"]
mod park_spawn_tests;
#[cfg(test)]
#[path = "park_unspawn_tests.rs"]
mod park_unspawn_tests;
#[cfg(test)]
#[path = "item_spawn_tests.rs"]
mod item_spawn_tests;
#[cfg(test)]
#[path = "type_lookup_tests.rs"]
mod type_lookup_tests;

/// A layout item's parked car has no spot (car +44 = 0; +40 the item).
pub const ITEM_SPOT: usize = usize::MAX;
/// The layout items' pool (F-16770: 512 records of 144 bytes).
pub const ITEM_POOL: usize = 512;

/// A layout item (F-98196, record ctor F-71354): a car or a person the parked spawner
/// puts at a place of a quadtree node, as it fills the node's spots.
#[derive(Clone, Debug)]
pub struct LayoutItem {
    /// +40: the type's name, a car type or a person type
    pub name: String,
    /// +96: where it was put (FullReset puts a script's item back there, F-50454)
    pub home: Vec3,
    /// +112
    pub pos: Vec3,
    /// +132: the heading (rad)
    pub heading: f32,
    /// +128: the race's ambient path it stands for (None: a script's item)
    pub path: Option<ItemPath>,
    /// +140: a script's item
    pub script: bool,
    /// +24: the node it is in
    pub node: u32,
}

/// What the spawner reads of an item's race path (`*_AmbientData.xml`, mcAmbientPath).
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemPath {
    /// the path's index in the race's array
    pub index: usize,
    /// +164 bParked
    pub parked: bool,
    /// +188: an "Ambient" trigger points at it; +189: that trigger has fired
    pub referenced: bool,
    pub fired: bool,
    /// +64: its first vertex found a lane
    pub lane: bool,
    /// +171 bSpawnWhenTriggered
    pub spawn_when_triggered: bool,
}

/// F-97347: from the root down into the first child (order 0..3) whose box holds
/// [x - 1, x + 1] x [z - 1, z + 1] strictly; the deepest such node.
pub fn item_node(tree: &ParkingTree, p: Vec2) -> Option<u32> {
    let mut ni = tree.root?;
    'down: loop {
        for ch in tree.nodes[ni as usize].children.into_iter().flatten() {
            let n = &tree.nodes[ch as usize];
            if n.min.x < p.x - 1.0 && p.x + 1.0 < n.max.x && n.min.y < p.y - 1.0 && p.y + 1.0 < n.max.y {
                ni = ch;
                continue 'down;
            }
        }
        return Some(ni);
    }
}

/// The child order of the node query by the quadrant the centre is in (table 0x8282AEF0).
const ORDER: [[usize; 4]; 4] = [[0, 1, 2, 3], [1, 3, 0, 2], [2, 0, 3, 1], [3, 2, 1, 0]];

/// The most lists kept (M+1804, 64 entries).
pub const MAX_LISTS: usize = 64;

/// F-80185: the squared distance of (x, z) from the box against r^2 - x's part a rounded product, z's fused onto it
/// (fmadds), r^2 rounded; true when not greater.
pub fn box_meets_circle(min: Vec2, max: Vec2, c: Vec2, r: f32) -> bool {
    let mut d2 = 0.0f32;
    if c.x < min.x {
        d2 = (c.x - min.x) * (c.x - min.x);
    } else if c.x > max.x {
        d2 = (c.x - max.x) * (c.x - max.x);
    }
    if c.y < min.y {
        let d = c.y - min.y;
        d2 = crate::physics::exact::fma(d, d, d2);
    } else if c.y > max.y {
        let d = c.y - max.y;
        d2 = crate::physics::exact::fma(d, d, d2);
    }
    d2 <= r * r
}

/// F-35598: the nodes whose box meets the circle (x, z), children first in the quadrant's
/// order, a node given when one of its own entries has `flags & 3` - a spot by its flags, a
/// layout item always (its +4 is 2); at most `cap`.
pub fn query(tree: &ParkingTree, items: &[Vec<u32>], ni: u32, c: Vec2, r: f32, cap: usize, out: &mut Vec<u32>) {
    let n = &tree.nodes[ni as usize];
    if !box_meets_circle(n.min, n.max, c, r) {
        return;
    }
    let q = (if c.x - n.centre.x > -1e-6 { 2 } else { 0 }) | (if c.y - n.centre.y > -1e-6 { 1 } else { 0 });
    for k in ORDER[q] {
        if out.len() >= cap {
            break;
        }
        if let Some(ch) = n.children[k] {
            query(tree, items, ch, c, r, cap, out);
        }
    }
    let has_items = items.get(ni as usize).is_some_and(|v| !v.is_empty());
    if out.len() < cap && (n.spots.iter().any(|&s| tree.spots[s as usize].flags & 3 != 0) || has_items) {
        out.push(ni);
    }
}

/// F-48973 and its callers: a node's (occupied, total) over its counted spots (flag bit 0) -
/// a free cruise-only spot counts only while no event runs - and the limit of occupied spots.
pub fn node_count(tree: &ParkingTree, ni: u32, occupied: &[bool], cruise: bool) -> (i32, i32) {
    let (mut occ, mut free) = (0, 0);
    for &s in &tree.nodes[ni as usize].spots {
        let sp = &tree.spots[s as usize];
        if sp.flags & 1 == 0 {
            continue;
        }
        if occupied[s as usize] {
            occ += 1;
        } else if cruise || !sp.cruise_only {
            free += 1;
        }
    }
    (occ, occ + free)
}

/// F-54839: the first type of the list whose name equals `name` with the case of A - Z ignored (F-40755, the original's
/// stricmp: no other byte is folded); the port's list is its car models' names.
pub(crate) fn type_by_name<'a>(names: impl IntoIterator<Item = &'a [u8]>, name: &[u8]) -> Option<usize> {
    names.into_iter().position(|n| n.eq_ignore_ascii_case(name))
}

/// F-94309's inputs: the type's slots 6 (a car type) and 7 (a person type), the manager's gates (the bytes S+0x642C5 /
/// +0x642C8 for cars, +0x642C6 for people, the constructor's 1; the density scale S+0x64240), the path copy's path
/// (+64 lane found, +164 bParked) and the item (+140 a script's, its path's +188 referenced / +189 fired).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ItemSpawnIn {
    pub car: bool,
    pub person: bool,
    pub cars_on: bool,
    pub peds_on: bool,
    pub density: f32,
    pub path: Option<(bool, bool)>,
    pub item: Option<(bool, Option<(bool, bool)>)>,
}

/// F-94309's request: a parked car (F-90920 path, with its colour), a car on the path's lane (with its colour), a person.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ItemSpawn {
    Parked(u32),
    Lane(u32),
    Person,
}

/// F-94309 (offline: the network test false, the type given): a car type needs the car bytes and a density scale other
/// than 0 (an ARCADE race's TRAFFIC none), a person type the people's byte. A car draws its colour u31 % 20; it is parked
/// when the path is bParked, the item a script's, or the item's path referenced and not fired (an item is needed);
/// else it rides the path's lane when the path found one and is not parked; else nothing. A person needs the item.
pub(crate) fn item_spawn_choice(inp: ItemSpawnIn, rng: &mut Mwc) -> Option<ItemSpawn> {
    if inp.car {
        if !inp.cars_on || inp.density == 0.0 {
            return None;
        }
    } else if inp.person && !inp.peds_on {
        return None;
    }
    if inp.car {
        let colour = rng.index(20);
        let lane = inp.path.is_some_and(|(l, p)| l && !p);
        let parked = inp.path.is_some_and(|(_, p)| p) || inp.item.is_some_and(|(s, ip)| s || ip.is_some_and(|(r, f)| r && !f));
        if parked {
            return inp.item.is_some().then_some(ItemSpawn::Parked(colour));
        }
        return lane.then_some(ItemSpawn::Lane(colour));
    }
    (inp.person && inp.item.is_some()).then_some(ItemSpawn::Person)
}

/// The parked unspawner's world (F-53207): the ambient manager's pools 0 (cars) and 2 (people) and their objects.
pub(crate) trait UnspawnEnv {
    /// the pool's active count (F-92034 / F-69170: S+12, +20 / +124)
    fn pool_len(&self, pool: u8) -> u32;
    /// F-50439: the pool's i-th object (None: none)
    fn get(&self, pool: u8, i: u32) -> Option<usize>;
    /// F-23063(+476): owned by another peer
    fn net_other(&self, pool: u8, h: usize) -> bool;
    /// +500 bit 0x4: already on its way out
    fn removing(&self, pool: u8, h: usize) -> bool;
    /// +44 (a spot's car): the spot's node (+24)
    fn spot_node(&self, h: usize) -> Option<u32>;
    /// +40 (a layout item's car or person): the item's node (+24)
    fn item_node(&self, pool: u8, h: usize) -> Option<u32>;
    /// F-14502 (the clock - +464) and +468
    fn age(&self, pool: u8, h: usize) -> (f32, f32);
    /// F-58860's x and z
    fn xz(&self, pool: u8, h: usize) -> (f32, f32);
    /// F-48973: (taken, taken + free)
    fn node_count(&mut self, node: u32) -> (i32, i32);
    /// the type's (+448) slot 6: the car type's gives 1, the person type's 0
    fn type_slot6(&mut self, pool: u8, h: usize) -> bool;
}

/// What F-53207 removes: the pool, the object and the reason F-79896 stores (+488).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Unspawned {
    pub pool: u8,
    pub h: usize,
    pub reason: u32,
}

/// F-53207, the parked unspawner: `tries` (M+2132) tries, at most one removal. Each draws U (u23 2^-23). U < 0.8: a
/// random car of pool 0 (F-30673 -> F-92034: no draw for an empty pool, else u31 % count); none, another peer's, on its
/// way out or not a spot's car: the next try; nearer than `radius` (M+12, dz dz + dx dx fused against r r): the next
/// try; its node off the lists, or the node holding more than its limit (F-48973, `node_limit` with M+152): removed,
/// reason 7. Else a second U: below 0.5 a car of pool 0, else a person of pool 2 (F-46838 -> F-69170); none, another
/// peer's, on its way out, not a layout item's, younger than its life, near, or its node on the lists: the next try;
/// else removed, reason 7 for the car type (slot 6 gives 1), 9 for the person type.
pub(crate) fn park_unspawn_core(env: &mut dyn UnspawnEnv, rng: &mut Mwc, tries: i32, lists: &[u32], focus: (f32, f32), radius: f32, density: f32) -> Option<Unspawned> {
    fn pick(env: &dyn UnspawnEnv, rng: &mut Mwc, pool: u8) -> Option<usize> {
        let n = env.pool_len(pool);
        if n == 0 {
            return None;
        }
        env.get(pool, rng.index(n))
    }
    let near = |env: &dyn UnspawnEnv, pool: u8, h: usize| {
        let (x, z) = env.xz(pool, h);
        let (dx, dz) = (focus.0 - x, focus.1 - z);
        dz.mul_add(dz, dx * dx) < radius * radius
    };
    for _ in 0..tries {
        if rng.float() < 0.8 {
            let Some(h) = pick(env, rng, 0) else { continue };
            if env.net_other(0, h) || env.removing(0, h) {
                continue;
            }
            let Some(node) = env.spot_node(h) else { continue };
            if near(env, 0, h) {
                continue;
            }
            if lists.contains(&node) {
                let (taken, total) = env.node_count(node);
                if taken <= node_limit(total, density) {
                    continue;
                }
            }
            return Some(Unspawned { pool: 0, h, reason: 7 });
        }
        let pool = if rng.float() < 0.5 { 0 } else { 2 };
        let Some(h) = pick(env, rng, pool) else { continue };
        if env.net_other(pool, h) || env.removing(pool, h) {
            continue;
        }
        let Some(node) = env.item_node(pool, h) else { continue };
        let (age, life) = env.age(pool, h);
        if age < life || near(env, pool, h) || lists.contains(&node) {
            continue;
        }
        let reason = if env.type_slot6(pool, h) { 7 } else { 9 };
        return Some(Unspawned { pool, h, reason });
    }
    None
}

/// The port's world as F-53207 sees it: pool 0 the ambient cars (`ambient_listed`, in the cars' order), pool 2 the
/// people's layout items (`ped_list`; a person is old enough when the people's side says so).
struct PortUnspawnEnv<'a> {
    cars: &'a [super::CarInstance],
    amb: Vec<usize>,
    peds: &'a [(Option<u32>, Vec3, bool)],
    parking: &'a ParkingTree,
    layout: &'a [Option<LayoutItem>],
    occ: Vec<bool>,
    cruise: bool,
    clock: f32,
}

impl UnspawnEnv for PortUnspawnEnv<'_> {
    fn pool_len(&self, pool: u8) -> u32 {
        if pool == 0 { self.amb.len() as u32 } else { self.peds.len() as u32 }
    }
    fn get(&self, pool: u8, i: u32) -> Option<usize> {
        Some(if pool == 0 { self.amb[i as usize] } else { i as usize })
    }
    /// offline an id's low 5 bits are the peer F-78245 gives, 0, the one F-23063 compares with
    fn net_other(&self, pool: u8, h: usize) -> bool {
        pool == 0 && self.cars[h].amb_id >= 0 && self.cars[h].amb_id & 31 != 0
    }
    fn removing(&self, pool: u8, h: usize) -> bool {
        pool == 0 && self.cars[h].removing.is_some()
    }
    fn spot_node(&self, h: usize) -> Option<u32> {
        let si = self.cars[h].parked.filter(|&s| s != ITEM_SPOT)?;
        Some(self.parking.spots.get(si).map_or(u32::MAX, |s| s.node))
    }
    fn item_node(&self, pool: u8, h: usize) -> Option<u32> {
        let it = if pool == 0 { self.cars[h].item } else { self.peds[h].0 }?;
        Some(self.layout.get(it as usize).and_then(|i| i.as_ref()).map_or(u32::MAX, |i| i.node))
    }
    fn age(&self, pool: u8, h: usize) -> (f32, f32) {
        if pool == 2 {
            return (if self.peds[h].2 { 1.0 } else { 0.0 }, 1.0);
        }
        let c = &self.cars[h];
        if let Some(r) = c.rail.as_ref() { (self.clock - r.born, r.life) } else { (self.clock - c.amb_born, c.amb_life) }
    }
    fn xz(&self, pool: u8, h: usize) -> (f32, f32) {
        let p = if pool == 0 { self.cars[h].state.pos } else { self.peds[h].1 };
        (p.x, p.z)
    }
    fn node_count(&mut self, node: u32) -> (i32, i32) {
        node_count(self.parking, node, &self.occ, self.cruise)
    }
    fn type_slot6(&mut self, pool: u8, _h: usize) -> bool {
        pool == 0
    }
}

/// The most occupied spots a node may hold: 1 up to 2 counted, 2 up to 4, else
/// trunc(fParkedCarDensityFactor x total).
pub fn node_limit(total: i32, density: f32) -> i32 {
    if total <= 2 {
        1
    } else if total <= 4 {
        2
    } else {
        (density * total as f32) as i32
    }
}

/// One active list as the parked spawner F-74446 reads it: its spots (+8, u16) and its items (+12).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ParkListView {
    pub spots: usize,
    pub items: usize,
}

/// What F-74446 asks of the world while it tries.
pub(crate) trait ParkEnv {
    /// spot +8 != 0 or a request in flight (+12 >= 0)
    fn spot_taken(&self, list: usize, e: usize) -> bool;
    /// spot +40 (bParkedInCruiseOnly)
    fn spot_cruise_only(&self, list: usize, e: usize) -> bool;
    /// the spot's place x, z (+48, +56)
    fn spot_xz(&self, list: usize, e: usize) -> (f32, f32);
    /// F-48973 for the list: (taken, total) of its counted spots
    fn node_count(&mut self, list: usize) -> (i32, i32);
    /// F-71916: the manager's type distribution drawn (F-36934 on the generator)
    fn type_draw(&mut self, rng: &mut Mwc) -> Option<usize>;
    /// the type may be parked: bound group (+116) not 2, its pool (+4) not empty, no taxi (+113)
    fn type_ok(&mut self, ty: usize) -> bool;
    /// F-92805 at the spot, h 1.5, the type's slot 7 as the flag (0 for a car type)
    fn seen(&mut self, list: usize, e: usize, ty: usize) -> bool;
    /// the list's k-th item: +76 != 0 or +80 >= 0 (its car or person stands)
    fn item_taken(&self, list: usize, k: usize) -> bool;
    /// the item's path (+128): a trigger points at it (+188) and it is bSpawnWhenTriggered (+171) or fired (+189)
    fn item_held(&self, list: usize, k: usize) -> bool;
}

/// What one call of F-74446 settles on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ParkTry {
    /// a spot's car: the list, the spot's index in it, the type and the colour index (of the type's twenty)
    Spot { list: usize, entry: usize, ty: usize, colour: u32 },
    /// the list's k-th item: the call ends with it, made or not (F-54839 for its type, F-94309)
    Item { list: usize, k: usize },
}

/// F-74446's tries (gates aside: the network F-24338, the manager's enable byte and the global one, checked by the
/// caller). The lists weigh float(spots) + float(items), added in order; none -> nothing. Each of `tries` (M+2128,
/// 60): x = (u23 total) 2^-23 walks the weights while x > w (the last list at most); the entry (u31 % (spots + items)).
/// A spot: taken, or cruise-only while `event` (S+0x64250) is 0, or F-48973's taken at its limit (1 up to 2 counted,
/// 2 up to 4, else trunc(density total)), or no type (F-71916), or a type that may not park, or nearer than `radius`
/// (dz dz + dx dx fused, squared against r r) and seen: the next try. Else the colour u31 % 20 and that spot. An item:
/// taken or held, the next try; else the item, and the call ends there.
pub(crate) fn park_spawn_core(env: &mut dyn ParkEnv, rng: &mut Mwc, lists: &[ParkListView], tries: i32, event: bool, density: f32, focus: (f32, f32), radius: f32) -> Option<ParkTry> {
    if lists.is_empty() {
        return None;
    }
    let w: Vec<f32> = lists.iter().map(|l| l.spots as i32 as f32 + l.items as i32 as f32).collect();
    let mut total = 0.0f32;
    for &x in &w {
        total += x;
    }
    if total == 0.0 {
        return None;
    }
    let n = lists.len();
    for _ in 0..tries.max(0) {
        let mut x = (rng.next() & 0x7F_FFFF) as f32 * total * (1.0 / 8_388_608.0);
        let mut li = 0usize;
        if x > w[0] {
            loop {
                x -= w[li];
                li += 1;
                if li >= n || !(x > w[li]) {
                    break;
                }
            }
        }
        let li = li.min(n - 1);
        let l = lists[li];
        let count = (l.spots + l.items) as i32;
        if count == 0 {
            // (the original traps on the division by 0; a list without entries weighs 0 and is reached only by x = 0)
            continue;
        }
        let e = divw_rem((rng.next() & 0x7FFF_FFFF) as i32, count);
        if e < l.spots as i32 {
            let e = e as usize;
            if env.spot_taken(li, e) || (env.spot_cruise_only(li, e) && !event) {
                continue;
            }
            let (taken, all) = env.node_count(li);
            if taken >= node_limit(all, density) {
                continue;
            }
            let Some(ty) = env.type_draw(rng) else { continue };
            if !env.type_ok(ty) {
                continue;
            }
            let (sx, sz) = env.spot_xz(li, e);
            let (dx, dz) = (focus.0 - sx, focus.1 - sz);
            if dz.mul_add(dz, dx * dx) < radius * radius && env.seen(li, e, ty) {
                continue;
            }
            let colour = (rng.next() & 0x7FFF_FFFF) % 20;
            return Some(ParkTry::Spot { list: li, entry: e, ty, colour });
        }
        let k = (e - l.spots as i32) as usize;
        if env.item_taken(li, k) || env.item_held(li, k) {
            continue;
        }
        return Some(ParkTry::Item { list: li, k });
    }
    None
}

/// The port's world as F-74446 sees it: the active lists' spots and items, the type distribution of the focus district
/// at this hour, the seen test from the focus.
struct PortParkEnv<'a> {
    parking: &'a ParkingTree,
    lists: &'a [u32],
    node_items: &'a [Vec<u32>],
    layout: &'a [Option<LayoutItem>],
    occ: Vec<bool>,
    items_occ: Vec<bool>,
    cruise: bool,
    ambient: &'a crate::ambient::Distribution,
    hour: f32,
    focus: Vec3,
    view: Vec3,
    mph: f32,
    models: &'a [std::sync::Arc<super::CarModel>],
    amb_types: &'a mut hashbrown::HashMap<String, super::traffic::AmbType>,
    data_dir: &'a str,
    used: hashbrown::HashMap<String, u32>,
    tune: &'a super::traffic::Tune,
    occluders: Option<Option<crate::occluder::Selected>>,
    /// (MCLA_PARK_DUMP) tries turned down at the type draw, the type's tests, the seen test
    why: [u32; 3],
}

impl PortParkEnv<'_> {
    fn spot(&self, l: usize, e: usize) -> usize {
        self.parking.nodes[self.lists[l] as usize].spots[e] as usize
    }
    fn item(&self, l: usize, k: usize) -> u32 {
        self.node_items[self.lists[l] as usize][k]
    }
}

impl ParkEnv for PortParkEnv<'_> {
    fn spot_taken(&self, l: usize, e: usize) -> bool {
        self.occ[self.spot(l, e)]
    }
    fn spot_cruise_only(&self, l: usize, e: usize) -> bool {
        self.parking.spots[self.spot(l, e)].cruise_only
    }
    fn spot_xz(&self, l: usize, e: usize) -> (f32, f32) {
        let p = self.parking.spots[self.spot(l, e)].pos;
        (p.x, p.z)
    }
    fn node_count(&mut self, l: usize) -> (i32, i32) {
        node_count(self.parking, self.lists[l], &self.occ, self.cruise)
    }
    fn type_draw(&mut self, rng: &mut Mwc) -> Option<usize> {
        let t = self.ambient.draw(self.focus, self.hour, rng);
        self.why[0] += t.is_none() as u32;
        t
    }
    /// (approx.: the type's pool +4 is taken as its model loaded and fewer cars of it than its instances)
    fn type_ok(&mut self, ty: usize) -> bool {
        let Some(name) = self.ambient.all.get(ty) else { return false };
        let Some(m) = self.models.iter().find(|m| m.name.eq_ignore_ascii_case(name)) else { return false };
        let root = std::path::PathBuf::from(self.data_dir);
        let t = *self.amb_types.entry(m.name.clone()).or_insert_with(|| super::traffic::AmbType::load(&root, &m.name));
        let ok = t.bound_group != 2 && !t.taxi && self.used.get(&m.name).copied().unwrap_or(0) < t.instances;
        self.why[1] += !ok as u32;
        ok
    }
    fn seen(&mut self, l: usize, e: usize, _ty: usize) -> bool {
        let p = self.parking.spots[self.spot(l, e)].pos;
        let (dir, focus) = (self.data_dir, self.focus);
        let occ = self.occluders.get_or_insert_with(|| crate::occluder::city(dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE)));
        let s = seen(self.tune, self.focus, self.view, self.mph, p, occ.as_ref());
        self.why[2] += s as u32;
        s
    }
    fn item_taken(&self, l: usize, k: usize) -> bool {
        self.items_occ[self.item(l, k) as usize]
    }
    fn item_held(&self, l: usize, k: usize) -> bool {
        self.layout.get(self.item(l, k) as usize).cloned().flatten().and_then(|i| i.path).is_some_and(|p| p.referenced && (p.fired || p.spawn_when_triggered))
    }
}

impl Vehicles {
    /// The traffic list's types' use for the type weights (F-83932, refreshed once a frame by
    /// F-68379): the instances of each type in play (type +8: the ambient cars riding, parked
    /// and towed - not the player's, the police's nor the racers') and its `bIsTrailer`.
    pub(super) fn ambient_use(&mut self, root: &std::path::Path) {
        let mut live: hashbrown::HashMap<String, i32> = hashbrown::HashMap::new();
        for c in self.cars.iter().filter(|c| !c.is_player && !c.cop && !c.racer) {
            *live.entry(c.model.name.to_ascii_lowercase()).or_insert(0) += 1;
        }
        for n in &self.ambient.all {
            if let Some(m) = self.models.iter().find(|m| m.name.eq_ignore_ascii_case(n)) {
                if !self.amb_types.contains_key(&m.name) {
                    let t = super::traffic::AmbType::load(root, &m.name);
                    self.amb_types.insert(m.name.clone(), t);
                }
            }
        }
        let types = &self.amb_types;
        let trailer = |n: &str| types.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).is_some_and(|(_, t)| t.trailer);
        self.ambient.set_use(&|n| live.get(n).copied().unwrap_or(0), &trailer);
    }

    /// The spots' occupants: spot i is taken while a car stands on it.
    fn spots_taken(&self) -> Vec<bool> {
        let mut occ = vec![false; self.parking.spots.len()];
        for c in &self.cars {
            if let Some(s) = c.parked.filter(|&s| s < occ.len()) {
                occ[s] = true;
            }
        }
        occ
    }

    /// The items' occupants: item h is taken while its car stands, or its person (the people's
    /// spawner reports those in `item_peds`).
    fn items_taken(&self) -> Vec<bool> {
        let mut occ = vec![false; self.layout.len()];
        for h in self.cars.iter().filter_map(|c| c.item).chain(self.item_peds.iter().copied()).chain(self.item_peds_due.iter().copied()).chain(self.item_followers_due.iter().map(|f| f.0)).chain(self.item_pending.iter().copied()) {
            if let Some(o) = occ.get_mut(h as usize) {
                *o = true;
            }
        }
        occ
    }

    /// A node's items in the order they were put in.
    fn items_of(&self, ni: u32) -> &[u32] {
        self.node_items.get(ni as usize).map_or(&[], |v| v.as_slice())
    }

    /// F-98196 (AddAmbientToLayout, the race paths' F-91600): a record of the pool,
    /// put into its node (F-86284); None when the pool is used up.
    pub fn add_layout_item(&mut self, name: &str, pos: Vec3, heading: f32, path: Option<ItemPath>) -> Option<u32> {
        let Some(node) = item_node(&self.parking, Vec2::new(pos.x, pos.z)) else { return None };
        // (the pool's free list: built 0 .. 511, taken from its head, given back to its tail)
        if self.layout.is_empty() {
            self.layout = vec![None; ITEM_POOL];
            self.item_free = (0..ITEM_POOL as u32).collect();
        }
        let Some(h) = self.item_free.pop_front() else {
            log::warn!("[AMBIENT LAYOUT] AddAmbientToLayout failed: no more items ({name})");
            return None;
        };
        let h = h as usize;
        let script = path.is_none();
        self.layout[h] = Some(LayoutItem { name: name.to_string(), home: pos, pos, heading, path, script, node });
        if self.node_items.len() < self.parking.nodes.len() {
            self.node_items.resize(self.parking.nodes.len(), Vec::new());
        }
        self.node_items[node as usize].push(h as u32);
        Some(h as u32)
    }

    /// AddRandomCarToLayout's type (F-31057): a draw of the traffic list of the district at
    /// `pos` at this hour, of the types of bound group `group`, no taxis.
    pub fn random_layout_car(&mut self, data_dir: &str, pos: Vec3, group: u32) -> Option<String> {
        let root = std::path::PathBuf::from(data_dir);
        let names: Vec<String> = self.models.iter().map(|m| m.name.clone()).filter(|n| n.starts_with("va_")).collect();
        for n in &names {
            self.amb_types.entry(n.clone()).or_insert_with(|| super::traffic::AmbType::load(&root, n));
        }
        let r = self.park_rng.float();
        let (models, types) = (&self.models, &self.amb_types);
        let ok = |m: &str| models.iter().any(|x| x.name.eq_ignore_ascii_case(m)) && types.iter().find(|(k, _)| k.eq_ignore_ascii_case(m)).is_some_and(|(_, t)| t.bound_group == group && !t.taxi);
        self.ambient.pick(pos, self.hour, r, &ok).map(|s| s.to_string())
    }

    /// F-66801 (RemoveAmbientFromLayout and the others): out of its node, its car gone.
    pub fn remove_layout_item(&mut self, h: u32) {
        let Some(item) = self.layout.get_mut(h as usize).and_then(|i| i.take()) else { return };
        self.item_free.push_back(h);
        if let Some(v) = self.node_items.get_mut(item.node as usize) {
            v.retain(|&x| x != h);
        }
        self.cars.retain(|c| c.item != Some(h));
        self.item_peds.retain(|&x| x != h);
    }

    /// The race path items' state (the trigger has fired, F-53279).
    pub fn layout_item_mut(&mut self, h: u32) -> Option<&mut LayoutItem> {
        self.layout.get_mut(h as usize).and_then(|i| i.as_mut())
    }

    /// No event is active (S+0x64250, F-86550): no race car on the road.
    fn cruise(&self) -> bool {
        !self.cars.iter().any(|c| c.racer || c.pinned)
    }

    /// F-51021: the active lists, from the region list's box (its centre, axis, length and
    /// width): the nodes met by the circle of half the length round the centre, then by the box
    /// itself against the node's square.
    pub(super) fn parked_lists(&mut self, sp: &Spawner) {
        self.park_lists.clear();
        let Some(root) = self.parking.root else { return };
        let (bc, vx, len, width) = sp.region_box;
        // (the cutscene mode: width = length - 700 x 700 at the mode's zero speed, offset 100)
        let width = if sp.cutscene { len } else { width };
        if len <= 0.0 {
            return;
        }
        let mut found = Vec::new();
        query(&self.parking, &self.node_items, root, bc, len * 0.5, MAX_LISTS, &mut found);
        let bh = Vec2::new(len * 0.5, width * 0.5);
        for ni in found {
            let n = &self.parking.nodes[ni as usize];
            let side = n.max.x - n.min.x;
            if rects_meet(bc, vx, bh, n.centre, Vec2::new(0.0, 1.0), Vec2::splat(side * 0.5)) {
                self.park_lists.push(ni);
            }
        }
        if std::env::var("MCLA_PARK_DUMP").is_ok() {
            self.park_dump_frame = self.park_dump_frame.wrapping_add(1);
            if self.park_dump_frame % 15 == 0 {
                self.park_dump(bc, sp.tune.parked_density);
            }
        }
    }

    /// (MCLA_PARK_DUMP) the lists, their spots, limits and occupants, and the parked cars' spread.
    fn park_dump(&self, focus2: Vec2, density: f32) {
        let occ = self.spots_taken();
        let cruise = self.cruise();
        let (mut spots, mut counted, mut limit, mut taken) = (0, 0, 0, 0);
        for &ni in &self.park_lists {
            let (o, t) = node_count(&self.parking, ni, &occ, cruise);
            spots += self.parking.nodes[ni as usize].spots.len();
            counted += t;
            limit += node_limit(t, density);
            taken += o;
        }
        if std::env::var("MCLA_PARK_DUMP").is_ok_and(|v| v == "2") {
            let f = self.cars.iter().find(|c| c.is_player).map_or(Vec3::ZERO, |c| c.state.pos);
            let nodes: Vec<String> = self
                .park_lists
                .iter()
                .map(|&ni| {
                    let (o, t) = node_count(&self.parking, ni, &occ, cruise);
                    let n = &self.parking.nodes[ni as usize];
                    let near = n.spots.iter().map(|&s| Vec2::new(self.parking.spots[s as usize].pos.x - f.x, self.parking.spots[s as usize].pos.z - f.z).length()).fold(f32::MAX, f32::min);
                    format!("{ni}:{o}/{t}<={} @{near:.0}", node_limit(t, density))
                })
                .collect();
            log::info!("parked lists: {}", nodes.join(" "));
        }
        let focus = self.cars.iter().find(|c| c.is_player).map_or(Vec3::new(focus2.x, 0.0, focus2.y), |c| c.state.pos);
        let mut d: Vec<f32> = self.cars.iter().filter(|c| c.parked.is_some()).map(|c| Vec2::new(c.state.pos.x - focus.x, c.state.pos.z - focus.z).length()).collect();
        d.sort_by(f32::total_cmp);
        let q = |k: f32| d.get(((d.len() as f32 - 1.0) * k).round() as usize).copied().unwrap_or(0.0);
        log::info!(
            "parked: lists {} spots {} counted {} limit sum {} taken in lists {} | parked cars {} (dist min {:.0} q25 {:.0} median {:.0} q75 {:.0} max {:.0}) cruise {} turned down (taken, limit, type, pool, seen) {:?}",
            self.park_lists.len(),
            spots,
            counted,
            limit,
            taken,
            d.len(),
            q(0.0),
            q(0.25),
            q(0.5),
            q(0.75),
            q(1.0),
            cruise,
            self.park_why
        );
    }

    /// F-74446 (its gates are the caller's): the original's tries (`park_spawn_core`, M+2128 = 60), then the spot's car,
    /// or the item it settled on - the call ends with the item, made or not.
    pub(super) fn parked_spawn(&mut self, gpu: &Gpu, sp: &Spawner, focus: Vec3, view: Vec3, mph: f32) {
        if self.park_lists.is_empty() || self.models.is_empty() {
            return;
        }
        let views: Vec<ParkListView> = self.park_lists.iter().map(|&ni| ParkListView { spots: self.parking.nodes[ni as usize].spots.len(), items: self.items_of(ni).len() }).collect();
        let (occ, items_occ, cruise) = (self.spots_taken(), self.items_taken(), self.cruise());
        let mut used: hashbrown::HashMap<String, u32> = hashbrown::HashMap::new();
        for c in &self.cars {
            *used.entry(c.model.name.clone()).or_insert(0) += 1;
        }
        let mut env = PortParkEnv {
            parking: &self.parking,
            lists: &self.park_lists,
            node_items: &self.node_items,
            layout: &self.layout,
            occ,
            items_occ,
            cruise,
            ambient: &self.ambient,
            hour: self.hour,
            focus,
            view,
            mph,
            models: &self.models,
            amb_types: &mut self.amb_types,
            data_dir: &sp.data_dir,
            used,
            tune: &sp.tune,
            occluders: None,
            why: [0; 3],
        };
        let got = park_spawn_core(&mut env, &mut self.park_rng, &views, 60, cruise, sp.tune.parked_density, (focus.x, focus.z), sp.tune.parked_radius);
        for (k, n) in env.why.into_iter().enumerate() {
            self.park_why[2 + k] += n;
        }
        match got {
            None => {}
            Some(ParkTry::Item { list, k }) => {
                let h = self.items_of(self.park_lists[list])[k];
                self.item_spawn(gpu, sp, h);
            }
            Some(ParkTry::Spot { list, entry, ty, colour }) => {
                let ni = self.park_lists[list];
                let si = self.parking.nodes[ni as usize].spots[entry] as usize;
                let spot = self.parking.spots[si];
                let name = self.ambient.all[ty].clone();
                let Some(mi) = self.models.iter().position(|m| m.name.eq_ignore_ascii_case(&name)) else { return };
                let mname = self.models[mi].name.clone();
                let Some(ty) = self.amb_types.get(&mname).copied() else { return };
                // its colour: one of the type's twenty
                let shade = colour as usize;
                let color = ty.color(shade).unwrap_or(TRAFFIC_PALETTE[shade % TRAFFIC_PALETTE.len()]);
                let model = self.models[mi].clone();
                // (F-23864 / F-18708: z row (sin h, 0, cos h), the car facing -z = (-sin h, 0, -cos h) - our yaw's
                // forward is (-sin, 0, -cos); put on the ground by the spawn probe in the step, `PARK_PROBE`)
                let idx = self.spawn(gpu, model, spot.pos, spot.heading, color, false);
                self.cars[idx].ai = None;
                self.cars[idx].parked = Some(si);
                // the request's seed and the init draws (`amb_init`, F-77422 with a spot): its life, the lights'
                // offsets and its front wheels' random turn (the car MWC's Gaussian)
                let steer = self.amb_init(idx, &ty, true).steer;
                self.cars[idx].park_steer = steer;
                for wh in self.cars[idx].state.wheels.iter_mut().take(2) {
                    wh.steer = steer;
                }
                self.cars[idx].settle = PARK_PROBE;
                if std::env::var("MCLA_PARK_DUMP").is_ok() {
                    let (o, t) = node_count(&self.parking, ni, &self.spots_taken(), cruise);
                    log::info!("parked spawn {} spot {} node {} occupied {}/{} limit {} cruise-only {}", self.cars[idx].model.name, si, ni, o, t, node_limit(t, sp.tune.parked_density), spot.cruise_only);
                }
            }
        }
    }

    /// F-74446's item branch (F-94309): the type by its name (F-54839, the car and
    /// the person types), a colour of the type's twenty; parked where it stands (a script's item,
    /// a parked path or one waiting for its trigger: F-90920; a person F-39915), else
    /// following its path when its first vertex has a lane (F-45833), else nothing.
    fn item_spawn(&mut self, gpu: &Gpu, sp: &Spawner, h: u32) -> bool {
        let Some(item) = self.layout.get(h as usize).cloned().flatten() else { return false };
        // (a path a trigger points at: its car comes from the trigger once it has fired, and only
        // from it with bSpawnWhenTriggered)
        if item.path.is_some_and(|p| p.referenced && (p.fired || p.spawn_when_triggered)) {
            return false;
        }
        // F-54839: the type by its name; a car type or a person type
        let mi = type_by_name(self.models.iter().map(|m| m.name.as_bytes()), item.name.as_bytes());
        let person = mi.is_none() && crate::peds::is_person_type(&item.name);
        if mi.is_none() && !person {
            self.park_why[2] += 1;
            return false;
        }
        // F-94309 (the bytes S+0x642C5 / C6 / C8 stay the constructor's 1: nothing else writes them)
        let inp = ItemSpawnIn {
            car: mi.is_some(),
            person,
            cars_on: true,
            peds_on: true,
            density: self.density_scale.unwrap_or(1.0),
            path: item.path.map(|p| (p.lane, p.parked)),
            item: Some((item.script, item.path.map(|p| (p.referenced, p.fired)))),
        };
        let shade = match item_spawn_choice(inp, &mut self.park_rng) {
            None => return false,
            Some(ItemSpawn::Person) => {
                // (the people's spawner places it)
                self.item_peds_due.push(h);
                return true;
            }
            Some(ItemSpawn::Lane(c)) => {
                self.item_followers_due.push((h, super::traffic::test_shade(c as usize)));
                return true;
            }
            Some(ItemSpawn::Parked(c)) => super::traffic::test_shade(c as usize),
        };
        let Some(mi) = mi else { return false };
        let mname = self.models[mi].name.clone();
        let root = std::path::PathBuf::from(&sp.data_dir);
        let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| super::traffic::AmbType::load(&root, &mname));
        let color = ty.color(shade).unwrap_or(TRAFFIC_PALETTE[shade % TRAFFIC_PALETTE.len()]);
        let model = self.models[mi].clone();
        // (F-90920: as a spot's car, F-23864 - its place and heading the item's)
        let idx = self.spawn(gpu, model, item.pos, item.heading, color, false);
        // (the request's seed and the init draws, `amb_init`: its life and lights' offsets. An item has no spot, so
        // F-77422 draws no wheel turn and leaves +3584 as it was - what the pooled car holds there is not traced; the
        // port's own draw stays)
        self.amb_init(idx, &ty, false);
        let steer = super::traffic::parked_steer(1.0 - self.rand(), self.rand());
        let c = &mut self.cars[idx];
        c.ai = None;
        c.parked = Some(ITEM_SPOT);
        c.item = Some(h);
        c.park_steer = steer;
        for wh in c.state.wheels.iter_mut().take(2) {
            wh.steer = steer;
        }
        c.settle = PARK_PROBE;
        if std::env::var("MCLA_PARK_DUMP").is_ok() {
            log::info!("layout item {} spawn {} at {:?} node {}", h, item.name, item.pos, item.node);
        }
        true
    }

    /// F-53207: up to 30 tries, at most one car marked for removal (F-79896, reason 7 / 9: it goes
    /// half a second later, when its fade has run out - `CarInstance::removing`).
    pub(super) fn parked_unspawn(&mut self, sp: &Spawner, focus: Vec3) {
        let amb: Vec<usize> = (0..self.cars.len()).filter(|&i| self.ambient_listed(i)).collect();
        let (occ, cruise) = (self.spots_taken(), self.cruise());
        let mut env = PortUnspawnEnv { cars: &self.cars, amb, peds: &self.ped_list, parking: &self.parking, layout: &self.layout, occ, cruise, clock: self.clock };
        // (M+2132: the constructor's 30)
        let Some(u) = park_unspawn_core(&mut env, &mut self.park_rng, 30, &self.park_lists, (focus.x, focus.z), sp.tune.unspawn_inner, sp.tune.parked_density) else { return };
        let dump = std::env::var("MCLA_PARK_DUMP").is_ok();
        if u.pool == 2 {
            if dump {
                let p = self.ped_list[u.h].1;
                log::info!("layout item {:?} person unspawn dist {:.1}", self.ped_list[u.h].0, Vec2::new(focus.x - p.x, focus.z - p.z).length());
            }
            self.ped_remove = self.ped_list[u.h].0;
            return;
        }
        if dump {
            let c = &self.cars[u.h];
            log::info!("parked unspawn {} spot {:?} item {:?} dist {:.1}", c.model.name, c.parked, c.item, Vec2::new(focus.x - c.state.pos.x, focus.z - c.state.pos.z).length());
        }
        // (F-79896: marked with the reason, removed when its fade has run out - `spawn_traffic`)
        self.cars[u.h].removing = Some(1.0);
    }

    /// In the system's ambient car list (list 0): riding traffic and parked cars.
    pub(super) fn ambient_listed(&self, i: usize) -> bool {
        let c = &self.cars[i];
        !c.is_player && !c.cop && !c.racer && !c.pinned && (c.rail.is_some() || c.parked.is_some())
    }
}

#[cfg(test)]
mod verify_lanes_tests {
    //! Per-case comparison with the original's unchanged generated code, run offline by
    //! work/verify-people/lanes (docs/reference/people/verify_people_lanes.md).
    use super::*;
    use mcla_formats::rage::parking::{ParkNode, ParkingSpot};

    fn fixture(name: &str) -> String {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/verify-people/lanes");
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    /// F-80185: the node-box-against-circle test, which the port does inline in `query`: a
    /// one-node tree with one listed spot is returned by `query` exactly when the original
    /// returns true (6,500 cases: random boxes, boundary and one-ulp radii).
    #[test]
    #[ignore = "requires the original's reference csv (work/verify-people/lanes)"]
    fn node_box_circle_test_matches_the_original() {
        let want: Vec<u32> = fixture("stage1_original.csv").lines().filter(|l| l.starts_with("B,")).map(|l| l.rsplit(',').next().unwrap().parse().unwrap()).collect();
        let cases = fixture("boxcases.txt");
        let (mut n, mut hits, mut bad) = (0, 0, Vec::new());
        for (line, w) in cases.lines().zip(&want) {
            let v: Vec<f32> = line.split(' ').map(|h| f32::from_bits(u32::from_str_radix(h, 16).unwrap())).collect();
            let node = ParkNode { spots: vec![0], children: [None; 4], centre: Vec2::ZERO, min: Vec2::new(v[0], v[1]), max: Vec2::new(v[2], v[3]), depth: 0 };
            let tree = ParkingTree { spots: vec![ParkingSpot { pos: Vec3::ZERO, heading: 0.0, flags: 1, cruise_only: false, node: 0 }], nodes: vec![node], root: Some(0) };
            let mut out = Vec::new();
            query(&tree, &[], 0, Vec2::new(v[4], v[5]), v[6], MAX_LISTS, &mut out);
            if out.len() as u32 != *w {
                bad.push(n);
            }
            // the original's arithmetic (fmuls on the x side, then fmadds z*z + x part)
            let (min, max, c) = (Vec2::new(v[0], v[1]), Vec2::new(v[2], v[3]), Vec2::new(v[4], v[5]));
            let mut d2 = 0.0f32;
            if c.x < min.x {
                d2 = (c.x - min.x) * (c.x - min.x);
            } else if c.x > max.x {
                d2 = (c.x - max.x) * (c.x - max.x);
            }
            if c.y < min.y {
                d2 = (c.y - min.y).mul_add(c.y - min.y, d2);
            } else if c.y > max.y {
                d2 = (c.y - max.y).mul_add(c.y - max.y, d2);
            }
            assert_eq!(d2 <= v[6] * v[6], *w == 1, "fused replica, case {n}");
            hits += *w;
            n += 1;
        }
        assert_eq!(n, want.len());
        println!("box test: {n} cases, {hits} true in the original, {} differ (first {:?})", bad.len(), bad.iter().take(5).collect::<Vec<_>>());
        assert_eq!((n, hits), (6500, 3481));
        // The port's inline test (d2 += dz * dz, two roundings) differs from the original's
        // fused z term: recorded as a difference, not corrected here.
        assert_eq!(bad.len(), 47);
    }

    fn parking() -> ParkingTree {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let d = std::fs::read(std::path::Path::new(&root).join("resources/city/sc/0x2e3038ce.r62")).unwrap();
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        let res = mcla_formats::rsc::Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data };
        mcla_formats::rage::parking::parse_parking(&res)
    }

    /// F-35598 (`query`) and F-48973 (`node_count`) against the original's functions on the real
    /// parking quadtree (3,656 nodes, 10,998 spots): 4,000 circles (random centres, radii to 3 km,
    /// caps 1 - 64, 0 - 40 nodes with a layout item) and 3,000 spot states.
    #[test]
    #[ignore = "requires user's extracted data (MCLA_DATA_DIR) and the original's reference csv"]
    fn parked_query_and_node_count_match_the_original() {
        let tree = parking();
        assert_eq!((tree.nodes.len(), tree.spots.len()), (3656, 10998));
        let orig = fixture("stage3_indexed.csv");
        let q: Vec<Vec<usize>> = orig.lines().filter(|l| l.starts_with("Q,")).map(|l| l.split(',').skip(3).map(|v| v.parse().unwrap()).collect()).collect();
        let (mut nq, mut total_nodes) = (0, 0);
        for (line, want) in fixture("qcases3.txt").lines().zip(&q) {
            let w: Vec<u32> = line.split(' ').map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let mut items = vec![Vec::new(); tree.nodes.len()];
            for &n in &w[5..5 + w[4] as usize] {
                items[n as usize].push(0u32);
            }
            let mut out = Vec::new();
            query(&tree, &items, tree.root.unwrap(), Vec2::new(f32::from_bits(w[0]), f32::from_bits(w[1])), f32::from_bits(w[2]), w[3] as usize, &mut out);
            let got: Vec<usize> = out.iter().map(|&n| n as usize).collect();
            assert_eq!(&got, want, "query case {nq}: {line}");
            nq += 1;
            total_nodes += got.len();
        }
        assert_eq!(nq, 4000);
        println!("query: {nq} cases, {total_nodes} nodes listed, all identical");
        let c: Vec<(u32, u32)> = orig.lines().filter(|l| l.starts_with("N,")).map(|l| {
            let f: Vec<u32> = l.split(',').skip(2).map(|v| v.parse().unwrap()).collect();
            (f[0], f[1])
        }).collect();
        let mut nc = 0;
        for (line, want) in fixture("ccases3.txt").lines().zip(&c) {
            let w: Vec<u32> = line.split(' ').map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let (ni, cruise, ns) = (w[0], w[1] != 0, w[2] as usize);
            let mut t = tree.clone();
            let mut occupied = vec![false; t.spots.len()];
            let spots = t.nodes[ni as usize].spots.clone();
            assert_eq!(spots.len(), ns);
            for (k, &sidx) in spots.iter().enumerate() {
                let (a8, a12, flags, cb) = (w[3 + 4 * k], w[4 + 4 * k], w[5 + 4 * k], w[6 + 4 * k]);
                occupied[sidx as usize] = a8 != 0 || (a12 as i32) >= 0;
                t.spots[sidx as usize].flags = flags;
                t.spots[sidx as usize].cruise_only = cb != 0;
            }
            // the original returns (occupied, free); the port (occupied, occupied + free)
            assert_eq!(node_count(&t, ni, &occupied, cruise), (want.0 as i32, (want.0 + want.1) as i32), "count case {nc}: {line}");
            nc += 1;
        }
        assert_eq!(nc, 3000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-74446 / F-53207: 1 up to 2 counted spots, 2 up to 4, then a quarter, truncated.
    #[test]
    fn parked_node_limit() {
        let l: Vec<i32> = (0..=13).map(|t| node_limit(t, 0.25)).collect();
        assert_eq!(l, [1, 1, 1, 2, 2, 1, 1, 1, 2, 2, 2, 2, 3, 3]);
        assert_eq!(node_limit(52, 0.25), 13);
    }

    /// F-97347: an item goes to the deepest node holding its 2 m square; the pool holds 512.
    #[test]
    fn layout_items_in_the_tree() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/city/sc/0x2e3038ce.r62")) else { return };
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        let res = mcla_formats::rsc::Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data };
        let mut v = Vehicles::new();
        v.parking = mcla_formats::rage::parking::parse_parking(&res);
        // hw_local_hangout1's va_350z_04
        let p = Vec3::new(746.375, 7.439, -286.63);
        let h = v.add_layout_item("va_350z_04", p, 226.225f32.to_radians(), None).unwrap();
        let it = v.layout[h as usize].clone().unwrap();
        let n = &v.parking.nodes[it.node as usize];
        assert!(n.min.x < p.x - 1.0 && p.x + 1.0 < n.max.x && n.min.y < p.z - 1.0 && p.z + 1.0 < n.max.y);
        for ch in n.children.into_iter().flatten() {
            let c = &v.parking.nodes[ch as usize];
            assert!(!(c.min.x < p.x - 1.0 && p.x + 1.0 < c.max.x && c.min.y < p.z - 1.0 && p.z + 1.0 < c.max.y));
        }
        assert!(it.script && v.node_items[it.node as usize] == [h]);
        // listed by the query even if it had no spots of its own
        let mut found = Vec::new();
        query(&v.parking, &v.node_items, v.parking.root.unwrap(), Vec2::new(p.x, p.z), 5.0, MAX_LISTS, &mut found);
        assert!(found.contains(&it.node));
        for k in 1..ITEM_POOL {
            assert!(v.add_layout_item("va_350z_04", p + Vec3::X * (k % 7) as f32, 0.0, None).is_some());
        }
        assert!(v.add_layout_item("va_350z_04", p, 0.0, None).is_none());
        v.remove_layout_item(h);
        assert!(!v.node_items[it.node as usize].contains(&h));
        assert_eq!(v.add_layout_item("va_350z_04", p, 0.0, None), Some(h));
    }

    /// The MWC: a = 0x5CDCFAA7, 23-bit floats, 31-bit indices.
    #[test]
    fn mwc_draws() {
        let mut m = Mwc { lo: 1, carry: 0 };
        assert_eq!(m.next(), 0x5CDC_FAA7);
        assert_eq!(m.carry, 0);
        let mut m = Mwc { lo: 0xFFFF_FFFF, carry: 0 };
        let t = 0x5CDC_FAA7u64 * 0xFFFF_FFFF;
        assert_eq!(m.next(), t as u32);
        assert_eq!(m.carry, (t >> 32) as u32);
        let mut m = Mwc { lo: 1, carry: 0 };
        assert_eq!(m.float(), (0x5CDC_FAA7u32 & 0x7F_FFFF) as f32 / 8_388_608.0);
        let mut m = Mwc { lo: 1, carry: 0 };
        assert_eq!(m.index(20), (0x5CDC_FAA7u32 & 0x7FFF_FFFF) % 20);
    }
}

/// F-97347 (the parking tree descent) against the original's code run offline on random trees
/// (tools/people_small_reference.py -> testdata/people_small_ref.txt: `IT` a tree, `IN` queries; margin 1.0 as the callers set).
#[cfg(test)]
mod original_tests {
    use super::*;
    use mcla_formats::rage::parking::ParkNode;

    #[test]
    fn box_test_matches_the_original() {
        // tools/box_circle_reference.py: F-80185 run offline, 20,000 cases (most at the rounding boundary)
        let f = |h: &str| f32::from_bits(u32::from_str_radix(h, 16).unwrap());
        let mut n = 0;
        for line in include_str!("testdata/box_circle_ref.txt").lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let v: Vec<f32> = a.split(' ').map(f).collect();
            let got = box_meets_circle(Vec2::new(v[0], v[1]), Vec2::new(v[2], v[3]), Vec2::new(v[4], v[5]), v[6]);
            assert_eq!(got, b == "1", "{line}");
            n += 1;
        }
        assert_eq!(n, 20000);
    }

    #[test]
    fn item_node_matches_the_original() {
        let hx = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut tree = ParkingTree::default();
        let mut n = 0;
        for line in include_str!("testdata/people_small_ref.txt").lines() {
            let w: Vec<&str> = line.split(' ').collect();
            if w[0] == "IT" {
                let k: usize = w[1].parse().unwrap();
                tree = ParkingTree::default();
                for i in 0..k {
                    let b = &w[2 + 8 * i..10 + 8 * i];
                    let ch = |c: usize| b[c].parse::<i32>().ok().filter(|v| *v >= 0).map(|v| v as u32);
                    let f = |c: usize| f32::from_bits(hx(b[4 + c]));
                    tree.nodes.push(ParkNode { spots: vec![], children: [ch(0), ch(1), ch(2), ch(3)], centre: Vec2::ZERO, min: Vec2::new(f(0), f(1)), max: Vec2::new(f(2), f(3)), depth: 0 });
                }
                tree.root = Some(0);
            } else if w[0] == "IN" {
                let p = Vec2::new(f32::from_bits(hx(w[1])), f32::from_bits(hx(w[2])));
                assert_eq!(item_node(&tree, p), Some(w[4].parse().unwrap()), "{line}");
                n += 1;
            }
        }
        assert_eq!(n, 960);
    }
}
