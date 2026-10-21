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

/// The child order of the node query by the quadrant the centre is in (table 0x8282AEF0).
const ORDER: [[usize; 4]; 4] = [[0, 1, 2, 3], [1, 3, 0, 2], [2, 0, 3, 1], [3, 2, 1, 0]];

/// The most lists kept (M+1804, 64 entries).
pub const MAX_LISTS: usize = 64;

/// sub_826E0F00: the nodes whose box meets the circle (x, z), children first in the quadrant's
/// order, a node given when one of its own spots has `flags & 3`; at most `cap`.
pub fn query(tree: &ParkingTree, ni: u32, c: Vec2, r: f32, cap: usize, out: &mut Vec<u32>) {
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
            query(tree, ch, c, r, cap, out);
        }
    }
    if out.len() < cap && n.spots.iter().any(|&s| tree.spots[s as usize].flags & 3 != 0) {
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
        if len <= 0.0 {
            return;
        }
        let mut found = Vec::new();
        query(&self.parking, root, bc, len * 0.5, MAX_LISTS, &mut found);
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
        // the weight of a list: its spots (and its runtime items: none)
        let w: Vec<f32> = self.park_lists.iter().map(|&ni| self.parking.nodes[ni as usize].spots.len() as f32).collect();
        let total: f32 = w.iter().sum();
        if total == 0.0 {
            return;
        }
        let occ = self.spots_taken();
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
            let e = self.park_rng.index(node.spots.len() as u32) as usize;
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

    /// sub_826F2A30: up to 30 tries, at most one car removed.
    pub(super) fn parked_unspawn(&mut self, sp: &Spawner, focus: Vec3) {
        // S car list 0: the ambient cars (riding and parked)
        let amb: Vec<usize> = (0..self.cars.len()).filter(|&i| self.ambient_listed(i)).collect();
        let r_in = sp.tune.unspawn_inner;
        for _ in 0..30 {
            if self.park_rng.float() >= 0.8 {
                // the cars of runtime items (lists 0 or 2): none here; the draws as the original's
                if self.park_rng.float() < 0.5 && !amb.is_empty() {
                    self.park_rng.index(amb.len() as u32);
                }
                continue;
            }
            if amb.is_empty() {
                continue;
            }
            let i = amb[self.park_rng.index(amb.len() as u32) as usize];
            let Some(si) = self.cars[i].parked else { continue };
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
                self.cars.remove(i);
                return;
            }
        }
    }

    /// In the system's ambient car list (list 0): riding traffic and parked cars.
    fn ambient_listed(&self, i: usize) -> bool {
        let c = &self.cars[i];
        !c.is_player && !c.cop && !c.racer && !c.pinned && (c.rail.is_some() || c.parked.is_some())
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
