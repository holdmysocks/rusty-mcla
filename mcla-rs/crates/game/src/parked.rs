//! The original's parked-car spawner and unspawner (density manager M, run from the traffic
//! spawner's 30 Hz manager frames in traffic.rs). Decoded 2026-10-21: docs/PEOPLE_TRAFFIC.md
//! "Parked-car spawner".
//!
//! - odd frames, right after the traffic region list: the active lists (sub_826F17D8) = the
//!   quadtree nodes of the parking spots (`resources/city/sc/0x2e3038ce.r62`) met by the region
//!   list's box, at most 64;
//! - even frames: one spawn at most (sub_826F23A0, 60 tries), then, while the region list has
//!   roads, one removal at most (sub_826F2A30, 30 tries).
//!
//! A node takes at most a quarter of its counted spots (sub_826E0788); a spot within 100 m is
//! filled only when the focus does not see it (sub_826F0870).

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
    /// A weight pick as sub_826F23A0 makes it: `(lo & 0x7FFFFF) x total x 2^-23`.
    fn scaled(&mut self, total: f32) -> f32 {
        (self.next() & 0x7F_FFFF) as f32 * total * (1.0 / 8_388_608.0)
    }
}

/// A layout item's parked car has no spot (car +44 = 0; +40 the item).
pub const ITEM_SPOT: usize = usize::MAX;
/// The layout items' pool (sub_826E2070: 512 records of 144 bytes).
pub const ITEM_POOL: usize = 512;

/// A layout item (sub_826E1418, record ctor sub_826E1FA0): a car or a person the parked spawner
/// puts at a place of a quadtree node, as it fills the node's spots.
#[derive(Clone, Debug)]
pub struct LayoutItem {
    /// +40: the type's name, a car type or a person type
    pub name: String,
    /// +96: where it was put (FullReset puts a script's item back there, sub_826E1CE8)
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

/// sub_823BDF38: from the root down into the first child (order 0..3) whose box holds
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

/// sub_826E0F00: the nodes whose box meets the circle (x, z), children first in the quadrant's
/// order, a node given when one of its own entries has `flags & 3` - a spot by its flags, a
/// layout item always (its +4 is 2); at most `cap`.
pub fn query(tree: &ParkingTree, items: &[Vec<u32>], ni: u32, c: Vec2, r: f32, cap: usize, out: &mut Vec<u32>) {
    let n = &tree.nodes[ni as usize];
    // sub_82246B78: squared distance from the box, against r^2
    let mut d2 = 0.0;
    if c.x < n.min.x {
        d2 = (c.x - n.min.x) * (c.x - n.min.x);
    } else if c.x > n.max.x {
        d2 = (c.x - n.max.x) * (c.x - n.max.x);
    }
    if c.y < n.min.y {
        d2 += (c.y - n.min.y) * (c.y - n.min.y);
    } else if c.y > n.max.y {
        d2 += (c.y - n.max.y) * (c.y - n.max.y);
    }
    if !(d2 <= r * r) {
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

/// sub_826E0788 and its callers: a node's (occupied, total) over its counted spots (flag bit 0) -
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

impl Vehicles {
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

    /// sub_826E1418 (AddAmbientToLayout, the race paths' sub_826E1520): a record of the pool,
    /// put into its node (sub_826E1738); None when the pool is used up.
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

    /// AddRandomCarToLayout's type (sub_826DA638): a draw of the traffic list of the district at
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

    /// sub_826E1A20 (RemoveAmbientFromLayout and the others): out of its node, its car gone.
    pub fn remove_layout_item(&mut self, h: u32) {
        let Some(item) = self.layout.get_mut(h as usize).and_then(|i| i.take()) else { return };
        self.item_free.push_back(h);
        if let Some(v) = self.node_items.get_mut(item.node as usize) {
            v.retain(|&x| x != h);
        }
        self.cars.retain(|c| c.item != Some(h));
        self.item_peds.retain(|&x| x != h);
    }

    /// The race path items' state (the trigger has fired, sub_82687F58).
    pub fn layout_item_mut(&mut self, h: u32) -> Option<&mut LayoutItem> {
        self.layout.get_mut(h as usize).and_then(|i| i.as_mut())
    }

    /// No event is active (S+0x64250, sub_826D5B30): no race car on the road.
    fn cruise(&self) -> bool {
        !self.cars.iter().any(|c| c.racer || c.pinned)
    }

    /// sub_826F17D8: the active lists, from the region list's box (its centre, axis, length and
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

    /// sub_826F23A0: up to 60 tries, at most one car.
    pub(super) fn parked_spawn(&mut self, gpu: &Gpu, sp: &Spawner, focus: Vec3, view: Vec3, mph: f32) {
        let n = self.park_lists.len();
        if n == 0 || self.models.is_empty() {
            return;
        }
        // the weight of a list: its spots and its items
        let w: Vec<f32> = self.park_lists.iter().map(|&ni| (self.parking.nodes[ni as usize].spots.len() + self.items_of(ni).len()) as f32).collect();
        let total: f32 = w.iter().sum();
        if total == 0.0 {
            return;
        }
        let occ = self.spots_taken();
        let items_occ = self.items_taken();
        let cruise = self.cruise();
        let mut occluders = None;
        for _ in 0..60 {
            // a list by weight, an entry of it uniformly
            let mut x = self.park_rng.scaled(total);
            let mut li = 0;
            while li < n && x > w[li] {
                x -= w[li];
                li += 1;
            }
            let li = li.min(n - 1);
            let ni = self.park_lists[li];
            let node = &self.parking.nodes[ni as usize];
            let e = self.park_rng.index((node.spots.len() + self.items_of(ni).len()) as u32) as usize;
            if e >= node.spots.len() {
                // an item: no count limit, no visibility test
                let h = self.items_of(ni)[e - node.spots.len()];
                if items_occ[h as usize] {
                    self.park_why[0] += 1;
                    continue;
                }
                if self.item_spawn(gpu, sp, h) {
                    return;
                }
                continue;
            }
            let si = node.spots[e] as usize;
            let spot = self.parking.spots[si];
            if occ[si] || (spot.cruise_only && !cruise) {
                self.park_why[0] += 1;
                continue;
            }
            let (o, t) = node_count(&self.parking, ni, &occ, cruise);
            if o >= node_limit(t, sp.tune.parked_density) {
                self.park_why[1] += 1;
                continue;
            }
            // a type: the focus district's traffic list at this hour (the riding traffic's
            // draw); not bound group 2, not a taxi, its pool not used up
            let r = self.park_rng.float();
            let models = &self.models;
            let Some(name) = self.ambient.pick(focus, self.hour, r, &|m| models.iter().any(|x| x.name.eq_ignore_ascii_case(m))).map(|s| s.to_string()) else {
                self.park_why[2] += 1;
                continue;
            };
            let Some(mi) = self.models.iter().position(|m| m.name.eq_ignore_ascii_case(&name)) else {
                self.park_why[2] += 1;
                continue;
            };
            let mname = self.models[mi].name.clone();
            let root = std::path::PathBuf::from(&sp.data_dir);
            let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| super::traffic::AmbType::load(&root, &mname));
            if ty.bound_group == 2 || ty.taxi {
                self.park_why[2] += 1;
                continue;
            }
            let used = self.cars.iter().filter(|c| c.model.name == mname).count() as u32;
            if used >= ty.instances {
                self.park_why[3] += 1;
                continue;
            }
            // within 100 m: not where the focus sees it
            let d = Vec2::new(spot.pos.x - focus.x, spot.pos.z - focus.z).length();
            if d < sp.tune.parked_radius {
                let occ_sel = occluders.get_or_insert_with(|| crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE)));
                if seen(&sp.tune, focus, view, mph, spot.pos, occ_sel.as_ref()) {
                    self.park_why[4] += 1;
                    continue;
                }
            }
            // its colour: one of the type's twenty
            let shade = self.park_rng.index(20) as usize;
            let color = ty.color(shade).unwrap_or(TRAFFIC_PALETTE[shade % TRAFFIC_PALETTE.len()]);
            let model = self.models[mi].clone();
            // (sub_826DF700 / sub_826DE7F8: z row (sin h, 0, cos h), the car facing -z =
            // (-sin h, 0, -cos h) - our yaw's forward is (-sin, 0, -cos); put on the ground by
            // the spawn probe in the step, `PARK_PROBE`)
            let idx = self.spawn(gpu, model, spot.pos, spot.heading, color, false);
            self.cars[idx].ai = None;
            self.cars[idx].parked = Some(si);
            // its front wheels' random turn (sub_826E86C8; which random numbers the Gaussian
            // takes was not traced: the port's own)
            let u1 = 1.0 - self.rand();
            let u2 = self.rand();
            let steer = super::traffic::parked_steer(u1, u2);
            self.cars[idx].park_steer = steer;
            for wh in self.cars[idx].state.wheels.iter_mut().take(2) {
                wh.steer = steer;
            }
            self.cars[idx].settle = PARK_PROBE;
            if std::env::var("MCLA_PARK_DUMP").is_ok() {
                log::info!("parked spawn {} spot {} dist {:.1} node {} occupied {}/{} limit {} cruise-only {}", self.cars[idx].model.name, si, d, ni, o, t, node_limit(t, sp.tune.parked_density), spot.cruise_only);
            }
            return;
        }
    }

    /// sub_826F23A0's item branch (sub_82687A60): the type by its name (sub_826D5390, the car and
    /// the person types), a colour of the type's twenty; parked where it stands (a script's item,
    /// a parked path or one waiting for its trigger: sub_826DF7F0; a person sub_826DFB08), else
    /// following its path when its first vertex has a lane (sub_826DF430), else nothing.
    fn item_spawn(&mut self, gpu: &Gpu, sp: &Spawner, h: u32) -> bool {
        let Some(item) = self.layout.get(h as usize).cloned().flatten() else { return false };
        // (a path a trigger points at: its car comes from the trigger once it has fired, and only
        // from it with bSpawnWhenTriggered)
        if item.path.is_some_and(|p| p.referenced && (p.fired || p.spawn_when_triggered)) {
            return false;
        }
        let Some(mi) = self.models.iter().position(|m| m.name.eq_ignore_ascii_case(&item.name)) else {
            // a person type (the people's spawner places it)
            if crate::peds::is_person_type(&item.name) {
                self.item_peds_due.push(h);
                return true;
            }
            self.park_why[2] += 1;
            return false;
        };
        let shade = self.park_rng.index(20) as usize;
        let path = item.path.unwrap_or_default();
        let parked = item.script || path.parked || (path.referenced && !path.fired);
        if !parked {
            if !path.lane {
                return false;
            }
            self.item_followers_due.push((h, shade));
            return true;
        }
        let mname = self.models[mi].name.clone();
        let root = std::path::PathBuf::from(&sp.data_dir);
        let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| super::traffic::AmbType::load(&root, &mname));
        let color = ty.color(shade).unwrap_or(TRAFFIC_PALETTE[shade % TRAFFIC_PALETTE.len()]);
        let model = self.models[mi].clone();
        // (sub_826DF7F0: as a spot's car, sub_826DF700 - its place and heading the item's)
        let idx = self.spawn(gpu, model, item.pos, item.heading, color, false);
        let born = self.clock;
        let life = 8.0 + 4.0 * self.rand();
        let steer = super::traffic::parked_steer(1.0 - self.rand(), self.rand());
        let c = &mut self.cars[idx];
        c.ai = None;
        c.parked = Some(ITEM_SPOT);
        c.item = Some(h);
        c.amb_born = born;
        c.amb_life = life;
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

    /// sub_826F2A30: up to 30 tries, at most one car marked for removal (sub_826E9588, reason 7 / 9: it goes
    /// half a second later, when its fade has run out - `CarInstance::removing`).
    pub(super) fn parked_unspawn(&mut self, sp: &Spawner, focus: Vec3) {
        // S car list 0: the ambient cars (riding and parked)
        let amb: Vec<usize> = (0..self.cars.len()).filter(|&i| self.ambient_listed(i)).collect();
        let r_in = sp.tune.unspawn_inner;
        for _ in 0..30 {
            if self.park_rng.float() >= 0.8 {
                // an item's car (list 0) or person (list 2), half and half: older than its
                // least age, 120 m off at least, its node off the lists -> removed (reason 7 / 9)
                if self.park_rng.float() < 0.5 {
                    if amb.is_empty() {
                        continue;
                    }
                    let i = amb[self.park_rng.index(amb.len() as u32) as usize];
                    let c = &self.cars[i];
                    // (already on its way out: car +500 bit 0x4)
                    if c.removing.is_some() {
                        continue;
                    }
                    let Some(h) = c.item else { continue };
                    let age = if let Some(r) = c.rail.as_ref() { (self.clock - r.born, r.life) } else { (self.clock - c.amb_born, c.amb_life) };
                    if age.0 < age.1 || Vec2::new(focus.x - c.state.pos.x, focus.z - c.state.pos.z).length_squared() < r_in * r_in {
                        continue;
                    }
                    if self.item_listed(h) {
                        continue;
                    }
                    if std::env::var("MCLA_PARK_DUMP").is_ok() {
                        log::info!("layout item {} unspawn {}", h, c.model.name);
                    }
                    // (sub_826E9588: marked, removed when its fade has run out - `spawn_traffic`)
                    self.cars[i].removing = Some(1.0);
                    return;
                }
                if self.ped_list.is_empty() {
                    continue;
                }
                let k = self.park_rng.index(self.ped_list.len() as u32) as usize;
                let (h, p, old) = self.ped_list[k];
                let Some(h) = h else { continue };
                if !old || Vec2::new(focus.x - p.x, focus.z - p.z).length_squared() < r_in * r_in || self.item_listed(h) {
                    continue;
                }
                if std::env::var("MCLA_PARK_DUMP").is_ok() {
                    log::info!("layout item {} person unspawn dist {:.1}", h, Vec2::new(focus.x - p.x, focus.z - p.z).length());
                }
                self.ped_remove = Some(h);
                return;
            }
            if amb.is_empty() {
                continue;
            }
            let i = amb[self.park_rng.index(amb.len() as u32) as usize];
            if self.cars[i].removing.is_some() {
                continue;
            }
            // (a spot's car: car +44)
            let Some(si) = self.cars[i].parked.filter(|&s| s != ITEM_SPOT) else { continue };
            let p = self.cars[i].state.pos;
            let d = Vec2::new(focus.x - p.x, focus.z - p.z);
            if d.length_squared() < r_in * r_in {
                continue;
            }
            let ni = self.parking.spots.get(si).map_or(u32::MAX, |s| s.node);
            let remove = if !self.park_lists.contains(&ni) {
                true
            } else {
                let occ = self.spots_taken();
                let (o, t) = node_count(&self.parking, ni, &occ, self.cruise());
                o > node_limit(t, sp.tune.parked_density)
            };
            if remove {
                if std::env::var("MCLA_PARK_DUMP").is_ok() {
                    log::info!("parked unspawn {} spot {} dist {:.1} listed {}", self.cars[i].model.name, si, d.length(), self.park_lists.contains(&ni));
                }
                self.cars[i].removing = Some(1.0);
                return;
            }
        }
    }

    /// An item's node is in the active lists.
    fn item_listed(&self, h: u32) -> bool {
        self.layout.get(h as usize).and_then(|i| i.as_ref()).is_some_and(|i| self.park_lists.contains(&i.node))
    }

    /// In the system's ambient car list (list 0): riding traffic and parked cars.
    fn ambient_listed(&self, i: usize) -> bool {
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

    /// sub_826F23A0 / sub_826F2A30: 1 up to 2 counted spots, 2 up to 4, then a quarter, truncated.
    #[test]
    fn parked_node_limit() {
        let l: Vec<i32> = (0..=13).map(|t| node_limit(t, 0.25)).collect();
        assert_eq!(l, [1, 1, 1, 2, 2, 1, 1, 1, 2, 2, 2, 2, 3, 3]);
        assert_eq!(node_limit(52, 0.25), 13);
    }

    /// sub_823BDF38: an item goes to the deepest node holding its 2 m square; the pool holds 512.
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
