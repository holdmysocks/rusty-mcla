//! What the physics needs of the props' fragment types and of their placement: the file
//! `resources/prop/0x0e66973c.r131` read for the numbers the original's breaking pass, cache
//! entries and narrow phase use (decode: work/veh/gjkfix/z4/z4_decode.md, sections 1.3 - 1.5
//! and 2). Every number is kept as the file has it, to the bit.
//!
//! ```text
//! root +0x08 -> dictionary, +0x18 -> fragment types (u16 count +0x1c); root +0x0c -> instances (u16 +0x10)
//! type: +0x40 centre-of-gravity shift of a fresh cache entry, +0x50.. six damping vectors,
//!       +0xb0 -> name, +0xd0 -> groups, +0xd4 -> children, +0xe4 -> archetype, +0xec -> composite,
//!       +0xf0 / +0xf4 -> per-child angular inertia (whole / damaged), +0x12e groups, +0x12f children,
//!       +0x130 root groups, +0x133 flags (bit 0: needs a cache entry), +0x13c least force that moves
//!       it, +0x140 the elasticity its callback writes
//! group: +0x08 strength, +0x0c share up, +0x10 share down, +0x38 latch, +0x3c mass, +0x44 first
//!       child group, +0x45 parent, +0x46 first child, +0x47 children, +0x48 child groups
//! child: +0x04 mass, +0x08 damaged mass, +0x0c group
//! bound: +0x04 type (0 sphere, 1 capsule, 3 box, 4 polyhedron, 12 composite), +0x08 radius,
//!       +0x10 / +0x20 box max / min, +0x30 centroid, +0x50 centre of gravity, +0x60 I / M (w = volume),
//!       +0x70 margin; composite: +0x80 -> parts, +0x84 -> matrices (64 bytes), +0x90 u16 count
//! instance: +0x10 matrix (64 bytes), +0x5c -> type, +0x80 level state (1 loose, 2 fixed, 3 none)
//! ```

use super::Rd;
use crate::rsc::Resource;

/// One convex part of a prop's composite bound, in the part's own frame.
#[derive(Debug, Clone, Default)]
pub struct FragBound {
    /// bound +4: 0 sphere, 1 capsule, 3 box, 4 polyhedron
    pub kind: u8,
    /// bound +6 (the centre of gravity is carried through the part's matrix)
    pub flag6: bool,
    pub radius: f32,
    pub max: [f32; 3],
    pub min: [f32; 3],
    pub centroid: [f32; 3],
    pub cg: [f32; 3],
    pub volume: f32,
    pub margin: f32,
    /// +0x80 of a sphere or capsule (its radius) and +0x90 of a capsule (its length)
    pub round: f32,
    pub length: f32,
    /// a box's or polyhedron's corners and triangles (with each triangle's neighbours)
    pub verts: Vec<[f32; 3]>,
    pub tris: Vec<[u16; 3]>,
    pub beside: Vec<[u16; 3]>,
    /// material number: +0x90 of a sphere, +0xd0 of a capsule
    pub material: u32,
}

#[derive(Debug, Clone, Default)]
pub struct FragPart {
    pub bound: Option<FragBound>,
    /// rows 0..2 and the place
    pub matrix: [[f32; 3]; 4],
}

#[derive(Debug, Clone, Default)]
pub struct FragGroup {
    pub name: String,
    pub strength: f32,
    pub up: f32,
    pub down: f32,
    pub latch: f32,
    pub mass: f32,
    pub first_group: u8,
    pub parent: u8,
    pub first_child: u8,
    pub children: u8,
    pub groups: u8,
}

#[derive(Debug, Clone, Default)]
pub struct FragChild {
    /// +0x0e: its bone of the skeleton (-1: none)
    pub bone: i16,
    /// +0xa0 / +0xa4: the effects played when its group breaks off / when it first comes off its
    /// standing instance (work/map2/colfx/prop_hit_fx_rule.md)
    pub break_events: Vec<PropEvent>,
    pub break_root_events: Vec<PropEvent>,
    pub mass: f32,
    pub damaged_mass: f32,
    pub group: u8,
    pub inertia: [f32; 3],
    pub damaged_inertia: [f32; 3],
}

/// An `evtInstancePtfx` of a prop's fragment type (class 0x680940): the effect it starts, where
/// (in the frame it is played in) and from what relative speed on.
#[derive(Debug, Clone, Default)]
pub struct PropEvent {
    /// +0x30
    pub effect: String,
    /// +0x20
    pub offset: [f32; 3],
    /// +0xac
    pub min_speed: f32,
}

/// The events of an `evtSet` (+04 -> event pointers, +08 u16 count); only the particle effect events.
fn event_set(rd: &Rd, set: Option<usize>) -> Vec<PropEvent> {
    let mut out = Vec::new();
    let Some(set) = set else { return out };
    let (Some(arr), n) = (rd.ptr(set + 4), rd.u16(set + 8) as usize) else { return out };
    for k in 0..n.min(16) {
        let Some(e) = rd.ptr(arr + k * 4) else { continue };
        if rd.vtable(e) == 0x0068_0940 {
            out.push(PropEvent { effect: cstr(rd, e + 0x30), offset: v3(rd, e + 0x20), min_speed: rd.f32(e + 0xac) });
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct FragArchetype {
    pub type_flags: u32,
    pub include: u32,
    pub mass: f32,
    pub inv_mass: f32,
    pub gravity: f32,
    pub max_speed: f32,
    pub max_spin: f32,
    pub inertia: [f32; 3],
    pub inv_inertia: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub struct FragType {
    /// its offset in the file (what an instance's +0x5c points at)
    pub off: usize,
    pub name: String,
    /// +0x133
    pub flags: u8,
    pub root_groups: u8,
    pub min_move: f32,
    pub elasticity: f32,
    pub cg_shift: [f32; 3],
    pub damping: [[f32; 3]; 6],
    /// +0x10c: the effects played on every impact report of the prop (closing speed > 0.1)
    pub type_events: Vec<PropEvent>,
    /// the skeleton's bones in the type's frame, as it stands unposed (drawable +0x0c: matrices +0x10, count +0x14,
    /// parents +0x04; rows x y z and the place): what a break event follows
    pub bones: Vec<[[f32; 3]; 4]>,
    pub groups: Vec<FragGroup>,
    pub children: Vec<FragChild>,
    pub archetype: FragArchetype,
    /// the composite's own numbers
    pub radius: f32,
    pub max: [f32; 3],
    pub min: [f32; 3],
    pub centroid: [f32; 3],
    pub cg: [f32; 3],
    pub margin: f32,
    pub parts: Vec<FragPart>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FragInstance {
    /// offset of its type
    pub frag: usize,
    /// rows 0..2 and the place, as the file has them
    pub matrix: [[f32; 3]; 4],
    /// 1 loose, 2 fixed, 3 not in the level
    pub state: u32,
}

fn v3(rd: &Rd, o: usize) -> [f32; 3] {
    [rd.f32(o), rd.f32(o + 4), rd.f32(o + 8)]
}

fn cstr(rd: &Rd, o: usize) -> String {
    let d = rd.data();
    let mut e = o;
    while e < d.len() && d[e] != 0 {
        e += 1;
    }
    String::from_utf8_lossy(&d[o.min(d.len())..e]).into_owned()
}

fn bound_at(rd: &Rd, o: usize) -> FragBound {
    let kind = rd.u8(o + 4);
    let mut b = FragBound {
        kind,
        flag6: rd.u8(o + 6) != 0,
        radius: rd.f32(o + 8),
        max: v3(rd, o + 0x10),
        min: v3(rd, o + 0x20),
        centroid: v3(rd, o + 0x30),
        cg: v3(rd, o + 0x50),
        volume: rd.f32(o + 0x6c),
        margin: rd.f32(o + 0x70),
        ..Default::default()
    };
    match kind {
        0 => {
            b.round = rd.f32(o + 0x80);
            b.material = rd.u32(o + 0x90);
        }
        1 => {
            b.round = rd.f32(o + 0x80);
            b.length = rd.f32(o + 0x90);
            b.material = rd.u32(o + 0xd0);
        }
        3 | 4 => {
            let (nv, nt) = (rd.u32(o + 0xc8) as usize, rd.u32(o + 0xcc) as usize);
            if let (Some(verts), Some(tris)) = (rd.ptr(o + 0x80), rd.ptr(o + 0x8c)) {
                if nv < 4096 && nt < 8192 && rd.has(verts, nv * 16) && rd.has(tris, nt * 16) {
                    for v in 0..nv {
                        b.verts.push(v3(rd, verts + v * 16));
                    }
                    for t in 0..nt {
                        let p = tris + t * 16;
                        b.tris.push([rd.u16(p + 4), rd.u16(p + 6), rd.u16(p + 8)]);
                        b.beside.push([rd.u16(p + 10), rd.u16(p + 12), rd.u16(p + 14)]);
                    }
                }
            }
        }
        _ => {}
    }
    b
}

/// The unposed skeleton of a fragment type: the local matrices carried up the parent chain.
fn bones_of(rd: &Rd, fr: usize) -> Vec<[[f32; 3]; 4]> {
    let Some(lod) = rd.ptr(fr + 0xb4).and_then(|d| rd.ptr(d + 0x0c)) else { return Vec::new() };
    let (Some(mats), Some(par), n) = (rd.ptr(lod + 0x10), rd.ptr(lod + 4), rd.u16(lod + 0x14) as usize) else { return Vec::new() };
    if n == 0 || n > 256 || !rd.has(mats, n * 64) || !rd.has(par, n * 4) {
        return Vec::new();
    }
    let local = |i: usize| -> [[f32; 3]; 4] { [v3(rd, mats + i * 64), v3(rd, mats + i * 64 + 16), v3(rd, mats + i * 64 + 32), v3(rd, mats + i * 64 + 48)] };
    // (row convention: a point is carried by the bone's matrix, then by its parent's)
    let mul = |a: &[[f32; 3]; 4], b: &[[f32; 3]; 4]| -> [[f32; 3]; 4] {
        let rot = |v: [f32; 3]| -> [f32; 3] { [v[0] * b[0][0] + v[1] * b[1][0] + v[2] * b[2][0], v[0] * b[0][1] + v[1] * b[1][1] + v[2] * b[2][1], v[0] * b[0][2] + v[1] * b[1][2] + v[2] * b[2][2]] };
        let t = rot(a[3]);
        [rot(a[0]), rot(a[1]), rot(a[2]), [t[0] + b[3][0], t[1] + b[3][1], t[2] + b[3][2]]]
    };
    let mut world: Vec<Option<[[f32; 3]; 4]>> = vec![None; n];
    fn resolve(i: usize, n: usize, rd: &Rd, par: usize, local: &dyn Fn(usize) -> [[f32; 3]; 4], mul: &dyn Fn(&[[f32; 3]; 4], &[[f32; 3]; 4]) -> [[f32; 3]; 4], world: &mut Vec<Option<[[f32; 3]; 4]>>, depth: usize) -> [[f32; 3]; 4] {
        if let Some(w) = world[i] {
            return w;
        }
        let p = rd.u32(par + i * 4) as usize;
        let w = if p >= n || p == i || depth > 64 { local(i) } else { mul(&local(i), &resolve(p, n, rd, par, local, mul, world, depth + 1)) };
        world[i] = Some(w);
        w
    }
    (0..n).map(|i| resolve(i, n, rd, par, &local, &mul, &mut world, 0)).collect()
}

/// Every fragment type of the prop file.
pub fn parse_frag_types(res: &Resource) -> Vec<FragType> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(d) = rd.ptr(8) else { return out };
    let (Some(vp), vc) = (rd.ptr(d + 0x18), rd.u16(d + 0x1c) as usize) else { return out };
    for i in 0..vc {
        let Some(fr) = rd.ptr(vp + i * 4) else { continue };
        let mut t = FragType {
            off: fr,
            name: rd.ptr(fr + 0xb0).map(|o| cstr(&rd, o)).unwrap_or_default(),
            flags: rd.u8(fr + 0x133),
            root_groups: rd.u8(fr + 0x130),
            min_move: rd.f32(fr + 0x13c),
            elasticity: rd.f32(fr + 0x140),
            cg_shift: v3(&rd, fr + 0x40),
            ..Default::default()
        };
        for k in 0..6 {
            t.damping[k] = v3(&rd, fr + 0x50 + 16 * k);
        }
        t.type_events = event_set(&rd, rd.ptr(fr + 0x10c));
        t.bones = bones_of(&rd, fr);
        let (ng, nc) = (rd.u8(fr + 0x12e) as usize, rd.u8(fr + 0x12f) as usize);
        if let Some(g) = rd.ptr(fr + 0xd0) {
            for k in 0..ng {
                let Some(o) = rd.ptr(g + 4 * k) else {
                    t.groups.push(FragGroup::default());
                    continue;
                };
                t.groups.push(FragGroup {
                    name: cstr(&rd, o + 0x54),
                    strength: rd.f32(o + 8),
                    up: rd.f32(o + 0xc),
                    down: rd.f32(o + 0x10),
                    latch: rd.f32(o + 0x38),
                    mass: rd.f32(o + 0x3c),
                    first_group: rd.u8(o + 0x44),
                    parent: rd.u8(o + 0x45),
                    first_child: rd.u8(o + 0x46),
                    children: rd.u8(o + 0x47),
                    groups: rd.u8(o + 0x48),
                });
            }
        }
        if let Some(c) = rd.ptr(fr + 0xd4) {
            let (i0, i1) = (rd.ptr(fr + 0xf0), rd.ptr(fr + 0xf4));
            for k in 0..nc {
                let Some(o) = rd.ptr(c + 4 * k) else {
                    t.children.push(FragChild::default());
                    continue;
                };
                t.children.push(FragChild {
                    bone: rd.u16(o + 0x0e) as i16,
                    break_events: event_set(&rd, rd.ptr(o + 0xa0)),
                    break_root_events: event_set(&rd, rd.ptr(o + 0xa4)),
                    mass: rd.f32(o + 4),
                    damaged_mass: rd.f32(o + 8),
                    group: rd.u8(o + 12),
                    inertia: i0.map_or([0.0; 3], |p| v3(&rd, p + 16 * k)),
                    damaged_inertia: i1.map_or([0.0; 3], |p| v3(&rd, p + 16 * k)),
                });
            }
        }
        if let Some(a) = rd.ptr(fr + 0xe4) {
            t.archetype = FragArchetype {
                type_flags: rd.u32(a + 0x10),
                include: rd.u32(a + 0x14),
                mass: rd.f32(a + 0x28),
                inv_mass: rd.f32(a + 0x2c),
                gravity: rd.f32(a + 0x30),
                max_speed: rd.f32(a + 0x34),
                max_spin: rd.f32(a + 0x38),
                inertia: v3(&rd, a + 0x40),
                inv_inertia: v3(&rd, a + 0x50),
            };
        }
        if let Some(b) = rd.ptr(fr + 0xec) {
            t.radius = rd.f32(b + 8);
            t.max = v3(&rd, b + 0x10);
            t.min = v3(&rd, b + 0x20);
            t.centroid = v3(&rd, b + 0x30);
            t.cg = v3(&rd, b + 0x50);
            t.margin = rd.f32(b + 0x70);
            if rd.u8(b + 4) == 12 {
                if let (Some(arr), Some(mats)) = (rd.ptr(b + 0x80), rd.ptr(b + 0x84)) {
                    for k in 0..rd.u16(b + 0x90) as usize {
                        let m = mats + 64 * k;
                        t.parts.push(FragPart { bound: rd.ptr(arr + 4 * k).map(|o| bound_at(&rd, o)), matrix: [v3(&rd, m), v3(&rd, m + 16), v3(&rd, m + 32), v3(&rd, m + 48)] });
                    }
                }
            }
        }
        out.push(t);
    }
    out
}

/// Every placed prop, with its matrix as the file has it.
pub fn parse_frag_instances(res: &Resource) -> Vec<FragInstance> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(arr) = rd.ptr(0x0c) else { return out };
    for i in 0..rd.u16(0x10) as usize {
        let Some(o) = rd.ptr(arr + i * 4) else { continue };
        let Some(frag) = rd.ptr(o + 0x5c) else { continue };
        out.push(FragInstance { frag, matrix: [v3(&rd, o + 0x10), v3(&rd, o + 0x20), v3(&rd, o + 0x30), v3(&rd, o + 0x40)], state: rd.u32(o + 0x80) });
    }
    out
}

#[cfg(test)]
mod event_tests {
    use super::*;

    /// The event sets of the prop types (work/map2/colfx/props_events.txt): the hydrant's water on the break of its
    /// body, the poles' sparks from 5 m/s on, the meter's coins as it comes off its post, the gas pump's explosion.
    #[test]
    fn prop_events_of_the_types() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/prop/0x0e66973c.r131")) else { return };
        if d.len() < 16 || &d[0..4] != b"MCLR" {
            return;
        }
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        let res = Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data };
        let types = parse_frag_types(&res);
        let of = |n: &str| types.iter().find(|t| t.name == n).unwrap();
        let h = of("la_prop_fire_hydrant_01x");
        assert!(h.type_events.is_empty() && h.children.len() == 3);
        assert_eq!((h.children[1].break_events.len(), h.children[1].break_events[0].effect.as_str(), h.children[1].break_events[0].offset, h.children[1].break_events[0].min_speed), (1, "water_hydrant", [0.0; 3], 0.0));
        assert!(h.children[0].break_events.is_empty() && h.children[2].break_events.is_empty());
        assert_eq!((h.children[0].bone, h.children[1].bone, h.bones.len()), (0, 1, 2));
        assert!(h.bones.iter().all(|b| b[3] == [0.0; 3] && b[0] == [1.0, 0.0, 0.0] && b[1] == [0.0, 1.0, 0.0]));
        let pole = of("la_prop_static_frwy_green_01x");
        assert_eq!((pole.type_events.len(), pole.type_events[0].effect.as_str(), pole.type_events[0].offset, pole.type_events[0].min_speed), (1, "col_metal_pole", [0.0, 0.2, 0.0], 5.0));
        assert_eq!(of("l_prop_parkingmeter_01x").children[1].break_root_events[0].effect, "coin");
        assert_eq!(of("la_prop_gas_pump_76_01x").children[1].break_root_events[0].effect, "exp_gas_station");
        // 45 light / signal types play col_metal_pole on a hit
        assert!(types.iter().filter(|t| t.type_events.first().is_some_and(|e| e.effect == "col_metal_pole")).count() >= 40);
    }
}
