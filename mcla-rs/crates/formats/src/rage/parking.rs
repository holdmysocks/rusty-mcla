//! Parking spots and their quadtree (`resources/city/sc/0x2e3038ce.r62`, the system's S+0x64220).
//!
//! ```text
//! root  +08 quadtree root node   +18 spot array (64-byte records)   +1c u16 spot count
//! spot  +04 occupant handle {+08 car, +0c record (-1: none)}  (runtime; empty in the file)
//!       +18 its quadtree node    +1c u32 flags (bit 0: counted; the node query takes mask 3)
//!       +28 u8 bParkedInCruiseOnly (only filled while no event runs)
//!       +30 vec3 position        +3c f32 heading
//! node  (80 bytes) +04 spot pointer array  +08 u16 spot count  +0c count / +10 list of runtime
//!       items (script-placed; none in the file)  +18 parent  +1c..+28 four children
//!       +2c / +30 centre x / z   +34 / +38 min x / z   +3c / +40 max x / z
//! ```
//! (docs/PEOPLE_TRAFFIC.md "Parking spots"; the cruise-only byte matches the district
//! `*_parked_ambients_AmbientData.xml` lists' `bParkedInCruiseOnly` one for one.)
//!
//! The heading h is in radians; the parked car's z row is (sin h, 0, cos h) and it faces -z =
//! `(-sin h, 0, -cos h)` (sub_826DF700 / sub_826DE7F8). Checked on the data: so placed, 87 % of the
//! cruise-only spots stand side by side with their nearest neighbour 3.3 m away (nose-in rows).

use super::Rd;
use crate::rsc::Resource;
use glam::{Vec2, Vec3};

#[derive(Debug, Clone, Copy)]
pub struct ParkingSpot {
    pub pos: Vec3,
    pub heading: f32,
    /// +0x1c: bit 0 = counted by the spawner's limit; the node query takes `flags & 3`
    pub flags: u32,
    /// +0x28 `bParkedInCruiseOnly`: only filled while no event is active
    pub cruise_only: bool,
    /// +0x18: its quadtree node (index into `ParkingTree::nodes`)
    pub node: u32,
}

impl ParkingSpot {
    /// Direction the parked car faces.
    pub fn dir(&self) -> Vec3 {
        Vec3::new(-self.heading.sin(), 0.0, -self.heading.cos())
    }
}

/// A quadtree node ("list") of the spots.
#[derive(Debug, Clone, Default)]
pub struct ParkNode {
    /// +0x04 / +0x08: the node's own spots (indices into `ParkingTree::spots`)
    pub spots: Vec<u32>,
    /// +0x1c..+0x28: the four children (indices into `ParkingTree::nodes`)
    pub children: [Option<u32>; 4],
    /// +0x2c / +0x30
    pub centre: Vec2,
    /// +0x34 / +0x38 and +0x3c / +0x40 (x, z)
    pub min: Vec2,
    pub max: Vec2,
    pub depth: u8,
}

#[derive(Debug, Clone, Default)]
pub struct ParkingTree {
    pub spots: Vec<ParkingSpot>,
    pub nodes: Vec<ParkNode>,
    /// the root node (root +0x08)
    pub root: Option<u32>,
}

pub fn parse_parking(res: &Resource) -> ParkingTree {
    let rd = Rd::new(res);
    let mut out = ParkingTree::default();
    let Some(arr) = rd.ptr(0x18) else { return out };
    let count = rd.u16(0x1c) as usize;
    let mut node_of_spot = Vec::with_capacity(count);
    for i in 0..count {
        let o = arr + i * 64;
        if !rd.has(o, 64) {
            break;
        }
        let pos = Vec3::new(rd.f32(o + 0x30), rd.f32(o + 0x34), rd.f32(o + 0x38));
        out.spots.push(ParkingSpot { pos, heading: rd.f32(o + 0x3c), flags: rd.u32(o + 0x1c), cruise_only: rd.u8(o + 0x28) != 0, node: u32::MAX });
        node_of_spot.push(rd.ptr(o + 0x18));
    }
    // the tree, depth first (a node's index is given before its children's)
    let mut index: hashbrown::HashMap<usize, u32> = hashbrown::HashMap::new();
    fn walk(rd: &Rd, o: usize, depth: u8, arr: usize, n_spots: usize, out: &mut ParkingTree, index: &mut hashbrown::HashMap<usize, u32>) -> Option<u32> {
        if !rd.has(o, 80) || index.contains_key(&o) || depth > 32 {
            return None;
        }
        let me = out.nodes.len() as u32;
        index.insert(o, me);
        let n = rd.u16(o + 8) as usize;
        let mut spots = Vec::with_capacity(n);
        if let Some(p) = rd.ptr(o + 4) {
            for k in 0..n {
                if let Some(s) = rd.ptr(p + 4 * k) {
                    if s >= arr && (s - arr) % 64 == 0 && (s - arr) / 64 < n_spots {
                        spots.push(((s - arr) / 64) as u32);
                    }
                }
            }
        }
        let v2 = |a: usize| Vec2::new(rd.f32(a), rd.f32(a + 4));
        out.nodes.push(ParkNode { spots, children: [None; 4], centre: v2(o + 0x2c), min: v2(o + 0x34), max: v2(o + 0x3c), depth });
        for k in 0..4 {
            if let Some(c) = rd.ptr(o + 0x1c + 4 * k) {
                let ci = walk(rd, c, depth + 1, arr, n_spots, out, index);
                out.nodes[me as usize].children[k] = ci;
            }
        }
        Some(me)
    }
    if let Some(r) = rd.ptr(0x08) {
        let n_spots = out.spots.len();
        out.root = walk(&rd, r, 0, arr, n_spots, &mut out, &mut index);
    }
    for (s, p) in out.spots.iter_mut().zip(node_of_spot) {
        if let Some(n) = p.and_then(|p| index.get(&p)) {
            s.node = *n;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file's tree: 10998 spots, 3656 nodes, spots in 1870 of them (10992 at depth 8, 6 at
    /// the root), every spot's +0x18 its containing node; 8938 cruise-only (the XML lists' count).
    #[test]
    fn parking_quadtree() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/city/sc/0x2e3038ce.r62")) else { return };
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        let t = parse_parking(&Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data });
        assert_eq!(t.spots.len(), 10998);
        assert_eq!(t.nodes.len(), 3656);
        assert_eq!(t.root, Some(0));
        assert_eq!(t.nodes.iter().filter(|n| !n.spots.is_empty()).count(), 1870);
        assert_eq!(t.nodes.iter().map(|n| n.spots.len()).sum::<usize>(), 10998);
        assert_eq!(t.nodes[0].spots.len(), 6);
        assert_eq!(t.nodes.iter().filter(|n| n.depth == 8).map(|n| n.spots.len()).sum::<usize>(), 10992);
        assert_eq!(t.nodes.iter().map(|n| n.spots.len()).max(), Some(52));
        for (ni, n) in t.nodes.iter().enumerate() {
            for &s in &n.spots {
                assert_eq!(t.spots[s as usize].node, ni as u32);
            }
        }
        assert_eq!(t.spots.iter().filter(|s| s.cruise_only).count(), 8938);
        assert!(t.spots.iter().all(|s| s.flags & 1 != 0));
        // a leaf is 22.72 m square
        let leaf = t.nodes.iter().find(|n| n.depth == 8).unwrap();
        assert!((leaf.max.x - leaf.min.x - 22.72).abs() < 0.01 && (leaf.max.y - leaf.min.y - 22.72).abs() < 0.01);
    }
}
