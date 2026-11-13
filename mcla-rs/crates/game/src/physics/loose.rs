//! Loose props as the car's hull meets them: what the original does when an active body
//! reaches an INACTIVE instance of the physics level (work/veh/gjkfix/z4/z4_decode.md, the
//! decodes in work/veh/gjkfix/p1/ and work/veh/gjk_exact.md part 10). The prop has no body when
//! it is hit: the tick's velocity solve takes it for immovable, the breaking pass gives the car
//! only a share of that impulse, turns the prop - or the group of it that broke off - into a
//! body, and solves again.
//!
//! This file holds the props' data as the physics reads it (the fragment types and where they
//! stand), the cache entries and pieces that have come loose, the narrow phase between a car's
//! hull and a prop's parts, the breaking pass, and the pieces' own motion.

use super::collision::{surface_touch, Surface};
use super::exact::{self as ex, cross, dot3, fma, recip};
use super::gjk::{convex_contact, HullCore, Nearest, PartCore, Support};
use super::hull::HullShape;
use super::solver::{self, Body, Pushed, Side, Touch};
use glam::Vec3;
use mcla_formats::rage::fragphys::{FragBound, FragInstance, FragType};
use std::collections::HashMap;
use std::sync::Arc;

/// The edge of the cells the instances are looked up by (m).
const CELL: f32 = 16.0;

/// Every prop of the level: the types and the instances, as the file has them.
#[derive(Debug, Default)]
pub struct LooseWorld {
    pub types: Vec<FragType>,
    /// each instance and the number of its type in `types`
    pub instances: Vec<(FragInstance, usize)>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// the corners of every polyhedron part (bound type 4), by type and part: what its core scans (bound +186)
    polyhedra: HashMap<(usize, usize), Vec<Vec3>>,
    /// the cache entries held for good: one for every instance in the level whose type keeps one (F-30607 gives it
    /// as the instance is put in; 102 in the city, as the running original holds at the title and in free roam)
    pub kept_entries: usize,
}

fn cell_of(x: f32, z: f32) -> (i32, i32) {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}

pub fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

pub fn m4(a: &[[f32; 3]; 4]) -> [Vec3; 4] {
    [v(a[0]), v(a[1]), v(a[2]), v(a[3])]
}

/// A point of a frame carried into the world (a sum of four, vmsum4fp with the column).
pub fn xf(m: &[Vec3; 4], p: Vec3) -> Vec3 {
    Vec3::new((m[3].x + m[2].x * p.z) + (m[1].x * p.y + m[0].x * p.x), (m[3].y + m[2].y * p.z) + (m[1].y * p.y + m[0].y * p.x), (m[3].z + m[2].z * p.z) + (m[1].z * p.y + m[0].z * p.x))
}

/// A world vector in a frame's axes (three sums of three).
pub fn inv(m: &[Vec3; 4], d: Vec3) -> Vec3 {
    Vec3::new(dot3(m[0], d), dot3(m[1], d), dot3(m[2], d))
}

impl LooseWorld {
    pub fn new(types: Vec<FragType>, instances: Vec<FragInstance>) -> LooseWorld {
        let by_off: HashMap<usize, usize> = types.iter().enumerate().map(|(i, t)| (t.off, i)).collect();
        let mut polyhedra = HashMap::new();
        for (i, t) in types.iter().enumerate() {
            for (c, p) in t.parts.iter().enumerate() {
                if let Some(b) = p.bound.as_ref().filter(|b| b.kind == 4 && !b.verts.is_empty()) {
                    let n = if b.scan > 0 { (b.scan as usize).min(b.verts.len()) } else { b.verts.len() };
                    polyhedra.insert((i, c), b.verts[..n].iter().map(|x| v(*x)).collect());
                }
            }
        }
        let mut out = LooseWorld { types, instances: Vec::new(), cells: HashMap::new(), polyhedra, kept_entries: 0 };
        // (the torn bumpers' own type, made by the game's code rather than read from the file)
        out.types.push(piece_type());
        for inst in instances {
            let Some(&t) = by_off.get(&inst.frag) else { continue };
            // (state 3 is never put into the level: the glow cards)
            if inst.state == 3 || !inst.matrix[3].iter().all(|x| x.is_finite()) {
                continue;
            }
            let i = out.instances.len() as u32;
            out.cells.entry(cell_of(inst.matrix[3][0], inst.matrix[3][2])).or_default().push(i);
            out.kept_entries += out.types[t].keeps_entry as usize;
            out.instances.push((inst, t));
        }
        out
    }

    /// The prop file of a data directory.
    pub fn load(file: &std::path::Path) -> Option<Arc<LooseWorld>> {
        let res = crate::assets::load_resource_file(file).ok()?;
        let types = mcla_formats::rage::fragphys::parse_frag_types(&res);
        let instances = mcla_formats::rage::fragphys::parse_frag_instances(&res);
        if std::env::var_os("MCLA_DBG_LOOSELIST").is_some() {
            for (n, i) in instances.iter().enumerate() {
                eprintln!("LOOSEINST {n} state {} frag {:x} at {:?}", i.state, i.frag, i.matrix[3]);
            }
        }
        Some(Arc::new(LooseWorld::new(types, instances)))
    }

    /// The instances whose place is within `r` of `p` on the map, in the file's order.
    pub fn near(&self, p: Vec3, r: f32) -> Vec<u32> {
        let (x0, z0) = cell_of(p.x - r, p.z - r);
        let (x1, z1) = cell_of(p.x + r, p.z + r);
        let mut out = Vec::new();
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                if let Some(list) = self.cells.get(&(cx, cz)) {
                    for &i in list {
                        let m = &self.instances[i as usize].0.matrix[3];
                        if (m[0] - p.x).abs() <= r && (m[2] - p.z).abs() <= r {
                            out.push(i);
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Cache entries: which groups of a prop an instance still holds, and what it weighs
// (F-13409, F-99875, F-68804, F-58768 / F-88418; p1/break_decode.md 3.3, 3.4).

/// A fragment cache entry: the groups it has lost, its age, and its mass properties.
#[derive(Clone, Copy, Debug, Default)]
pub struct Entry {
    /// the groups NOT in it (+268)
    pub absent: u64,
    /// seconds since the prop first got an entry (+420)
    pub age: f32,
    /// the composite's centre of gravity (bound +80), the archetype's mass and moments
    pub cg: Vec3,
    pub mass: f32,
    pub inv_mass: f32,
    pub inertia: Vec3,
    pub inv_inertia: Vec3,
    /// (the port's own: the prop instance a piece broke off, for drawing it, props.rs `follow_level`)
    pub from: Option<u32>,
    /// the pool's frame it was last stamped in (+12, F-95559: when taken and on every contact run, F-17408)
    pub stamp: u32,
    /// (the port's own) its place in the pool's list: the higher, the nearer the front (`LooseState::pool_take`), and
    /// whether F-18636 has taken it for another this tick (its teardown waits for the tick's end, `pool_teardown`)
    pub slot: u64,
    pub taken: bool,
}

fn group_of(t: &FragType, child: usize) -> usize {
    t.children.get(child).map_or(0, |c| c.group as usize)
}

// ---------------------------------------------------------------------------------------------
// The fragment cache's pool (F-56091, F-18636; reference/vehicle/prop_reset_2026-11-10.md "The pool"): 160 entries in
// one list, the used ones in front. An entry is taken from the back and put in front; when none is free a used one is
// taken from what it serves (F-74663). The instances whose type always holds one (vt[44], `FragType::keeps_entry`) hold
// theirs from the moment they are put in the level (F-30607), never given up: 102 in the city, as the running original
// shows at the title and in free roam.

/// The pool's size (F-47157 hands 160 to F-56091).
pub const POOL_CAP: usize = 160;

/// One used entry as F-18636's walk sees it: its stamp (+12), whether its instance's type keeps its entry (vt[44]),
/// and the instance's bound sphere (+96: centre, w = radius).
#[derive(Clone, Copy, Debug)]
pub struct PoolSeen {
    pub stamp: u32,
    pub keeps: bool,
    pub sphere: [f32; 4],
}

/// F-18636's victim when the pool is full, among the used entries in the list's order (`used[0]` the front): only an
/// entry not stamped this frame or the last (stamp < frame - 1, unsigned) whose type does not keep its entry. With
/// the viewer sphere set (manager +776; F-44865 sets it every frame from the prop manager's update, camera place and
/// radius 300): the gap g = (R + r)^2 - |c - p|^2 (each step rounded: vsubfp, vaddfp of the splats, vmsum3fp, vmulfp,
/// vsubfp); the first entry whose g is not >= 0 (outside the sphere, or NaN) is taken at once, else the one with the
/// least g (a g not >= the best so far takes its place; the best starts at FLT_MAX). None: no candidate.
pub fn pool_victim(used: &[PoolSeen], frame: u32, viewer: [f32; 4]) -> Option<usize> {
    use super::artic::V4;
    let c = V4(viewer);
    let big = V4::splat(viewer[3]);
    let mask = [true, true, true, false];
    let (zero, w1) = (V4::ZERO, V4::new(0.0, 0.0, 0.0, 1.0));
    let mut best = V4::new(f32::MAX, f32::MAX, f32::MAX, 0.0);
    let mut out = None;
    for (k, e) in used.iter().enumerate() {
        if e.stamp >= frame.wrapping_sub(1) || e.keeps {
            continue;
        }
        let p = V4(e.sphere);
        let d = c.sub(p);
        let rr = big.add(V4::splat(e.sphere[3]));
        let g = rr.mul(rr).sub(d.dot3(d));
        let g4 = zero.sel(g, mask).sel(w1, [false, false, false, true]);
        if !g4.ge(zero).iter().all(|x| *x) {
            return Some(k);
        }
        let b4 = zero.sel(best, mask);
        if !g4.ge(b4).iter().all(|x| *x) {
            out = Some(k);
            best = g;
        }
    }
    out
}

/// F-94385: a bound sphere (centre, w = radius) carried by a matrix (rows 0..2 and the place): the centre (x, y, z, 1)
/// against each column (vmsum4fp), the radius times the longest row: l2 = the largest squared row length (vmsum3fp,

/// One side of the level's broad phase (F-84048): box min / max, bound centroid, radius, matrix now, matrix before the
/// move (collider +32; None: inactive).
pub type BroadSide = (Vec3, Vec3, Vec3, f32, [Vec3; 4], Option<[Vec3; 4]>);

/// F-84048's sphere for the octree query: the bound's centroid through the matrix (vmsum4fp on its columns), moved half the
/// tick's way back, the radius grown by half that way's length - the length by vrsqrtefp and one Newton step (an estimate of
/// inf taken as 0), as the original computes it.
pub fn broad_sphere(own: &BroadSide) -> [f32; 4] {
    use super::artic::V4;
    let m = &own.4;
    let col = |i: usize| V4::new(m[0][i], m[1][i], m[2][i], m[3][i]);
    let c = V4::new(own.2.x, own.2.y, own.2.z, 1.0);
    let mut centre = V4::new(c.dot4(col(0)).x(), c.dot4(col(1)).x(), c.dot4(col(2)).x(), 0.0);
    let mut radius = own.3;
    if let Some(prev) = own.5.as_ref() {
        let d = V4::new(m[3].x - prev[3].x, m[3].y - prev[3].y, m[3].z - prev[3].z, 0.0);
        let half = V4::splat(0.5);
        centre = d.nmsub(half, centre);
        let l2 = d.dot3(d);
        let est = l2.rsqrte();
        let e = est.sel(V4::ZERO, est.eq(V4::splat(f32::INFINITY)));
        let k = l2.nmsub(e.mul(e), V4::splat(1.0)).madd(half.mul(e), e);
        radius = l2.mul(k).madd(half, V4::splat(radius)).x();
    }
    [centre.0[0], centre.0[1], centre.0[2], radius]
}

/// F-84048's box test of a candidate the query gave: the two whole boxes - each grown to hold itself at its matrix before the
/// move, if it is active - along their six face axes (eps 1e-6).
pub fn broad_boxes(own: &BroadSide, other: &BroadSide) -> bool {
    let (m, n) = (&own.4, &other.4);
    let swept = |s: &(Vec3, Vec3, Vec3, f32, [Vec3; 4], Option<[Vec3; 4]>)| {
        let (mut h, mut c) = ((s.1 - s.0) * 0.5, (s.1 + s.0) * 0.5);
        if let Some(l) = s.5.as_ref() {
            let m = &s.4;
            let r: [Vec3; 3] = std::array::from_fn(|i| inv(m, l[i]));
            let t = inv(m, l[3] - m[3]);
            let c2 = t + r[0] * c.x + r[1] * c.y + r[2] * c.z;
            let e2 = r[0].abs() * h.x + r[1].abs() * h.y + r[2].abs() * h.z;
            let (hi, lo) = ((c + h).max(c2 + e2), (c - h).min(c2 - e2));
            c = (hi + lo) * 0.5;
            h = hi - c;
        }
        (xf(&s.4, c), h)
    };
    let ((wa, ha), (wb, hb)) = (swept(own), swept(other));
    let eps = Vec3::splat(1e-6);
    let r: [Vec3; 3] = std::array::from_fn(|i| Vec3::new(dot3(m[i], n[0]), dot3(m[i], n[1]), dot3(m[i], n[2])));
    let ab: [Vec3; 3] = std::array::from_fn(|i| r[i].abs() + eps);
    let tw = wa - wb;
    let t = Vec3::new(dot3(tw, n[0]), dot3(tw, n[1]), dot3(tw, n[2]));
    let q = Vec3::ZERO - Vec3::new(dot3(r[0], t), dot3(r[1], t), dot3(r[2], t));
    let w = Vec3::new(dot3(Vec3::new(r[0].x, r[1].x, r[2].x), q), dot3(Vec3::new(r[0].y, r[1].y, r[2].y), q), dot3(Vec3::new(r[0].z, r[1].z, r[2].z), q));
    let col = |k: usize| Vec3::new(ab[0][k], ab[1][k], ab[2][k]);
    (0..3).all(|i| ha[i] + dot3(ab[i], hb) > q[i].abs()) && (0..3).all(|k| hb[k] + dot3(col(k), ha) > w[k].abs())
}

/// vmaxfp row 0 and 1, then 2), y = vrsqrtefp(l2) (an infinite estimate made 0), y + (1 - l2 y y)(0.5 y), times l2.
pub fn sphere_through(m: &[Vec3; 4], s: [f32; 4]) -> [f32; 4] {
    use super::artic::V4;
    let row = |r: Vec3| V4::new(r.x, r.y, r.z, 0.0);
    let (r0, r1, r2, r3) = (row(m[0]), row(m[1]), row(m[2]), V4::new(m[3].x, m[3].y, m[3].z, 1.0));
    let c = V4::new(s[0], s[1], s[2], 1.0);
    let col = |i: usize| V4::new(r0.0[i], r1.0[i], r2.0[i], r3.0[i]);
    let (x, y, z) = (c.dot4(col(0)).x(), c.dot4(col(1)).x(), c.dot4(col(2)).x());
    let l2 = r0.dot3(r0).max(r1.dot3(r1)).max(r2.dot3(r2));
    let est = l2.rsqrte();
    let e = est.sel(V4::ZERO, est.eq(V4::splat(f32::INFINITY)));
    let k = l2.nmsub(e.mul(e), V4::splat(1.0)).madd(V4::splat(0.5).mul(e), e);
    let r = V4::splat(s[3]).mul(l2.mul(k)).x();
    [x, y, z, r]
}

/// Is part `child` in an entry: the type has a bound there and its group has not left.

pub fn part_present(t: &FragType, absent: u64, child: usize) -> bool {
    t.parts.get(child).map_or(false, |p| p.bound.is_some()) && absent & (1 << group_of(t, child)) == 0
}

/// A group and everything that hangs on it, as a bit set.
pub fn subtree(t: &FragType, g: usize) -> u64 {
    let mut bits = [0u32; 2];
    mark_subtree(t, g, 255, &mut bits);
    bits[0] as u64 | (bits[1] as u64) << 32
}

/// F-45377: group `g` and the groups under it set in the bit array `bits` (bit g of word g / 32); the sub-tree of group
/// `skip` is left out (255: none).
pub fn mark_subtree(t: &FragType, g: usize, skip: u32, bits: &mut [u32]) {
    bits[g >> 5] |= 1 << (g & 31);
    let grp = &t.groups[g];
    for k in 0..grp.groups as u32 {
        let c = grp.first_group as u32 + k;
        if c != skip {
            mark_subtree(t, c as usize, skip, bits);
        }
    }
}

/// The mass of what an entry holds (F-49276 / F-85792): the children's, in their order.
pub fn entry_mass(t: &FragType, absent: u64) -> f32 {
    let mut m = 0.0f32;
    for (c, child) in t.children.iter().enumerate() {
        if absent & (1 << group_of(t, c)) == 0 {
            m = child.mass + m;
        }
    }
    m
}

/// F-31082, the archetype's SetMass: the mass and its inverse (fdivs), 0 for a mass of 0.
fn set_mass(mass: f32) -> (f32, f32) {
    (mass, if mass == 0.0 { 0.0 } else { 1.0 / mass })
}

/// F-58768, the archetype's SetInertia: the inertia after the floor F-88418 and its inverse (the
/// estimate and one Newton step), 0x7f7fffff where a lane is 0. The vector unit runs in flush
/// mode: a subnormal lane compares equal to 0, and a subnormal inverse (an inertia above about
/// 8.5e37) is 0 of its sign.
pub(super) fn set_inertia(i: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let i = inertia_floor(i);
    let inv = |x: f32| {
        if x == 0.0 || x.is_subnormal() {
            return f32::MAX;
        }
        let r = recip(x);
        if r.is_subnormal() { 0.0f32.copysign(r) } else { r }
    };
    (i, i.map(inv))
}

/// F-72436, a rope's normal and stretch: d = B - A over its length (the estimate of 1 / length
/// and one Newton step) where the squared length is above 0x2b8cbccc, else (0, 1, 0) and a
/// length of 0; the stretch is the length less the rope's.
fn rope_normal(a: Vec3, b: Vec3, len: f32) -> (Vec3, f32) {
    let d = b - a;
    let s2 = dot3(d, d);
    let (n, l) = if s2 > f32::from_bits(0x2b8c_bccc) {
        let l = s2.sqrt();
        let e = 1.0 / l;
        let e = e + e * (-(e * l - 1.0));
        (d * e, l)
    } else {
        (Vec3::Y, 0.0)
    };
    (n, l - len)
}

/// F-88418: the largest of the three as its fcmpu / blt find it (x < y, then y < z or x < z: a
/// tie or a NaN keeps the earlier one), then each other one whose 20 times is below it raised
/// to the largest x 0.05.
fn inertia_floor(mut i: [f32; 3]) -> [f32; 3] {
    let big = if i[0] < i[1] { if i[1] < i[2] { 2 } else { 1 } } else if i[0] < i[2] { 2 } else { 0 };
    for j in 0..3 {
        if j != big && i[j] * 20.0 < i[big] {
            i[j] = i[big] * 0.05;
        }
    }
    i
}

/// F-99875: one part's place for the centre of volume - its bound's centre of gravity through the part's matrix where the
/// bound has one, else the matrix's origin.
pub fn part_place(m: &[Vec3; 4], cg: Option<Vec3>) -> Vec3 {
    match cg {
        Some(cg) => xf(m, cg),
        None => m[3],
    }
}

/// F-99875: the centre of the parts' volumes: each place by its volume, added in turn, over the sum of the volumes
/// where that is above 1e-6 (the estimate of 1 / sum and one Newton step); the sum 0 .. 1e-6 leaves the plain sum.
pub fn volume_centre(parts: impl Iterator<Item = (f32, Vec3)>) -> Vec3 {
    let mut acc = Vec3::ZERO;
    let mut tot = 0.0f32;
    for (w, p) in parts {
        tot = w + tot;
        acc = p * w + acc;
    }
    if tot > f32::from_bits(0x3586_37bd) {
        acc = acc * recip(tot);
    }
    acc
}

/// F-13409: the entry's centre of gravity (by the parts' volumes; shifted by the type's +0x40
/// while the entry is younger than a quarter of a second), its mass, and its moments (each
/// part's own about its place, carried to the centre without its turn; no moment under a
/// twentieth of the largest).
pub fn entry_refresh(t: &FragType, e: &mut Entry, mass: f32) {
    e.cg = volume_centre(t.parts.iter().enumerate().filter_map(|(c, part)| {
        let b = part.bound.as_ref().filter(|_| part_present(t, e.absent, c))?;
        Some((b.volume, part_place(&m4(&part.matrix), b.flag6.then(|| v(b.cg)))))
    }));
    if e.age < 0.25 {
        e.cg = e.cg + v(t.cg_shift);
    }
    (e.mass, e.inv_mass) = set_mass(mass);
    let mut sum = 0.0f32;
    let mut inertia = Vec3::ZERO;
    for (c, part) in t.parts.iter().enumerate() {
        let Some(b) = part.bound.as_ref().filter(|_| part_present(t, e.absent, c)) else { continue };
        let (m, own) = (t.children[c].mass, v(t.children[c].inertia));
        let d = (v(part.matrix[3]) + v(b.cg)) - e.cg;
        sum = m + sum;
        let (zz, yy, xx) = (d.z * d.z, d.y * d.y, d.x * d.x);
        inertia = inertia + Vec3::new(fma(yy + zz, m, own.x), fma(xx + zz, m, own.y), fma(xx + yy, m, own.z));
    }
    if sum > f32::MIN_POSITIVE {
        let (i, inv) = set_inertia(inertia.to_array());
        e.inertia = Vec3::from_array(i);
        e.inv_inertia = Vec3::from_array(inv);
    }
}

// ---------------------------------------------------------------------------------------------
// Bodies of what has come loose (the simulator's pool bodies, 672 bytes; p1/body_decode.md).

/// A body's speed limit and its turning limit (2 pi), as its constructor F-33655 sets them.
pub const BODY_MAX_SPEED: f32 = 500.0;
pub const BODY_MAX_SPIN: f32 = f32::from_bits(0x40c9_0fdb);

/// The body of an activated prop or piece.
#[derive(Clone, Copy, Debug)]
pub struct PBody {
    /// +256 and +208..: the centre of gravity in the world and the rows
    pub at: Vec3,
    pub rows: [Vec3; 3],
    pub v: Vec3,
    pub w: Vec3,
    /// the momenta (+304, +320)
    pub p: Vec3,
    pub l: Vec3,
    pub force: Vec3,
    pub torque: Vec3,
    /// the impulses not yet in the momenta (+368, +384)
    pub jf: Vec3,
    pub jt: Vec3,
    /// what the push solve asks (+400, +416)
    pub push: Vec3,
    pub turn: Vec3,
    /// the instance's matrix as the last collide is to have it (+96), and whether it is set (+660)
    pub last: [Vec3; 4],
    /// the instance's matrix before this tick's move (+32)
    pub prev: [Vec3; 4],
    pub last_set: bool,
    pub deep: bool,
    pub deep_count: u8,
    pub mass: f32,
    pub inv_mass: f32,
    pub inertia: Vec3,
    pub inv_inertia: Vec3,
    pub max_speed: f32,
    pub max_spin: f32,
    /// moves since the rows were made square (+664)
    pub moves: u32,
}

/// F-80759 / F-29166: a velocity and its momentum cut back to the body's limit.
fn capped(v: Vec3, p: Vec3, max: f32) -> (Vec3, Vec3) {
    let l2 = fma(v.y, v.y, fma(v.z, v.z, v.x * v.x));
    // (blelr: a NaN length is left as it is)
    if !(l2 > max * max) {
        return (v, p);
    }
    let mut e = (1.0 / (l2 as f64).sqrt()) as f32;
    for _ in 0..2 {
        e = fma(ex::fnms(e * e, l2, 1.0), e * 0.5, e);
    }
    let k = e * max;
    (v * k, p * k)
}

impl PBody {
    /// F-28128 on an instance at `matrix` whose bound's centre of gravity is `cg`.
    fn new(matrix: &[Vec3; 4], cg: Vec3, e: &Entry, max_speed: f32, max_spin: f32) -> PBody {
        PBody {
            at: xf(matrix, cg),
            rows: [matrix[0], matrix[1], matrix[2]],
            v: Vec3::ZERO,
            w: Vec3::ZERO,
            p: Vec3::ZERO,
            l: Vec3::ZERO,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            jf: Vec3::ZERO,
            jt: Vec3::ZERO,
            push: Vec3::ZERO,
            turn: Vec3::ZERO,
            last: *matrix,
            prev: *matrix,
            last_set: false,
            deep: false,
            deep_count: 0,
            mass: e.mass,
            inv_mass: e.inv_mass,
            inertia: e.inertia,
            inv_inertia: e.inv_inertia,
            max_speed,
            max_spin,
            moves: 0,
        }
    }

    /// F-21091: the impulses not yet in the momenta go in (and the velocities follow, cut to the body's limits) where they
    /// are not zero in every lane.
    fn take_impulses(&mut self) {
        if self.jf != Vec3::ZERO {
            self.p = self.p + self.jf;
            (self.v, self.p) = capped(self.p * self.inv_mass, self.p, self.max_speed);
            self.jf = Vec3::ZERO;
        }
        if self.jt != Vec3::ZERO {
            self.l = self.l + self.jt;
            (self.w, self.l) = capped(ex::spin_of(self.l, &self.rows, self.inv_inertia), self.l, self.max_spin);
            self.jt = Vec3::ZERO;
        }
    }

    /// The angular momentum of a turning rate (F-47804).
    fn momentum_of(&self, w: Vec3) -> Vec3 {
        let iw = ex::world_inertia(&self.rows, self.inertia);
        Vec3::new(dot3(Vec3::new(iw[0].x, iw[1].x, iw[2].x), w), dot3(Vec3::new(iw[0].y, iw[1].y, iw[2].y), w), dot3(Vec3::new(iw[0].z, iw[1].z, iw[2].z), w))
    }

    /// F-97724: an impulse at a point, taken into the momenta at once.
    pub fn impulse_now(&mut self, vec: Vec3, point: Vec3) {
        let (p, l) = impulse_sums(self.p, self.l, self.at, vec, point);
        self.p = p;
        (self.v, self.p) = capped(self.p * self.inv_mass, self.p, self.max_speed);
        self.l = l;
        (self.w, self.l) = capped(ex::spin_of(self.l, &self.rows, self.inv_inertia), self.l, self.max_spin);
    }

    pub fn as_solver(&self) -> Body {
        Body { at: self.at, rows: self.rows, vel: self.v, ang: self.w, inv_mass: self.inv_mass, inv_moments: self.inv_inertia, force: self.force, torque: self.torque }
    }
}

// ---------------------------------------------------------------------------------------------
// The objects of the level that a car has had to do with.

/// A prop instance or a piece broken off one.
#[derive(Clone, Debug)]
pub struct Obj {
    pub ty: usize,
    /// the prop instance this is (None: a piece, a cache entry's own instance)
    pub inst: Option<u32>,
    /// the instance's matrix (inst +16)
    pub matrix: [Vec3; 4],
    pub entry: Option<Entry>,
    /// inst +80 bit 1: it has been moved or broken by the breaking pass
    pub hit: bool,
    pub body: Option<PBody>,
    /// its place among the level's indices and among the instances' addresses, against the
    /// car's (0): who is A in a pair, and whose contacts an island's rows begin with
    pub level: i64,
    pub addr: i64,
    /// (stage R) another vehicle: its bound and what its archetype holds
    pub veh: Option<VehObj>,
    /// its body's sleep object (body_decode.md 6.1; a torn bumper's is the piece's own)
    pub sleep: Sleep,
    /// taken out of the simulator asleep (F-38307): inactive at its last matrix until a contact wakes it (`wake_obj`)
    pub dormant: bool,
    /// an ambient (traffic) car of the session (`Amb`)
    pub amb: Option<Amb>,
}

/// Who a contact's side is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Car,
    Obj(usize),
    /// the city (always side B)
    City,
}

/// A contact of a pair of parts as the manifold keeps it.
#[derive(Clone, Copy, Debug)]
pub struct Slot2 {
    pub t: Touch,
    pub age: u16,
    pub enabled: bool,
    /// which triangle of the car's hull (for its material)
    pub element: u16,
    /// what the city's polygon is made of
    pub surface: Surface,
    /// what side A is made of, where A is an ambient car (its triangle's material)
    pub surface_a: Surface,
}

/// The contacts of one part of an object with one part of the car (a sub-manifold).
#[derive(Clone, Debug)]
pub struct Mani {
    pub a: Who,
    pub b: Who,
    pub part_a: u16,
    pub part_b: u16,
    /// the city's cell (side B the city)
    pub cell: usize,
    /// its parts' instance broke this tick: the pair it belonged to is no more
    pub stale: bool,
    /// the tick's collide has looked at this pair of parts
    pub visited: bool,
    /// the island the tick's collide put it in (the car's: 0), and its pair's place in the island's tree
    pub island: usize,
    pub order: usize,
    pub slots: Vec<Slot2>,
    /// a constraint's manifold (F-87905): its one contact made afresh every update
    pub rope: bool,
    /// (the game's level step) a manifold of another car's part lent to the island's solve: that car's id + 1, else 0
    pub owner: u32,
}

impl Mani {
    /// A new contact into the manifold (F-90136 -> F-74262, F-96165 / F-28977): it
    /// takes the place of the kept one within 2 cm of it, by A's point, that a refresh has seen -
    /// and its multipliers, and its element if that one was the deeper - or is added.
    fn insert(&mut self, fresh: Touch, element: u16, surface: Surface, surface_a: Surface) {
        let nearest = super::hull::nearest_kept(self.slots.iter().map(|s| (s.t.a.local, s.age)), fresh.a.local);
        let slot = Slot2 { t: fresh, age: 0, enabled: true, element, surface, surface_a };
        match nearest {
            Some(i) => {
                let old = self.slots[i];
                self.slots[i] = Slot2 { t: Touch { kept: old.t.kept, pushed: old.t.pushed, ..fresh }, element: if old.t.depth >= fresh.depth { old.element } else { element }, surface: if old.t.depth >= fresh.depth { old.surface } else { surface }, surface_a: if old.t.depth >= fresh.depth { old.surface_a } else { surface_a }, ..slot };
            }
            None if self.slots.len() < 6 => self.slots.push(slot),
            // (the seventh, F-55965)
            None => {
                let k = super::hull::seventh_contact(std::array::from_fn(|i| self.slots[i].t.a.local), fresh.a.local);
                if k < 6 && !super::vehsim::dbg_flag("MCLA_DBG_RA3") {
                    self.slots[k] = slot;
                }
            }
        }
    }
}

/// A part ready for the detector.
struct Shape<'a> {
    core: Core<'a>,
    margin: f32,
    cg: Vec3,
    bbox: (Vec3, Vec3),
    now: [Vec3; 4],
    last: [Vec3; 4],
}

/// The two parts' boxes, each grown to hold itself at its last matrix too, tested along their
/// six face axes (F-82759's cull; a decision only, written plainly).
fn boxes_meet(a: &Shape, b: &Shape) -> bool {
    parts_meet((a.bbox, a.now, a.last), (b.bbox, b.now, b.last))
}

/// [`boxes_meet`] on two parts given as (box, now, last): the thrown rider's parts against a vehicle's.
pub fn parts_meet(a: ((Vec3, Vec3), [Vec3; 4], [Vec3; 4]), b: ((Vec3, Vec3), [Vec3; 4], [Vec3; 4])) -> bool {
    struct S {
        bbox: (Vec3, Vec3),
        now: [Vec3; 4],
        last: [Vec3; 4],
    }
    let (a, b) = (S { bbox: a.0, now: a.1, last: a.2 }, S { bbox: b.0, now: b.1, last: b.2 });
    let swept = |s: &S| {
        let (c, h) = ((s.bbox.1 + s.bbox.0) * 0.5, (s.bbox.1 - s.bbox.0) * 0.5);
        let (m, l) = (&s.now, &s.last);
        let r: [Vec3; 3] = std::array::from_fn(|i| inv(m, l[i]));
        let t = inv(m, l[3] - m[3]);
        let c2 = t + r[0] * c.x + r[1] * c.y + r[2] * c.z;
        let e2 = r[0].abs() * h.x + r[1].abs() * h.y + r[2].abs() * h.z;
        let (hi, lo) = ((c + h).max(c2 + e2), (c - h).min(c2 - e2));
        let mid = (hi + lo) * 0.5;
        (xf(m, mid), hi - mid)
    };
    let ((wa, ha), (wb, hb)) = (swept(&a), swept(&b));
    let eps = Vec3::splat(1e-6);
    let r: [Vec3; 3] = std::array::from_fn(|i| inv(&b.now, a.now[i]));
    let ab: [Vec3; 3] = std::array::from_fn(|i| r[i].abs() + eps);
    let t = inv(&b.now, wa - wb);
    let q = -Vec3::new(r[0].dot(t), r[1].dot(t), r[2].dot(t));
    let w = r[0] * q.x + r[1] * q.y + r[2] * q.z;
    let col = |k: usize| Vec3::new(ab[0][k], ab[1][k], ab[2][k]);
    (0..3).all(|i| ha[i] + ab[i].dot(hb) > q[i].abs()) && (0..3).all(|k| hb[k] + col(k).dot(ha) > w[k].abs())
}

/// The car's contact with the city as a row of an island with other bodies (the car body 0; as
/// [`LooseState::solve_car_island`] lists them): A the car's point, B the city's.
pub fn car_city_touch(c: &solver::Contact, rows: [Vec3; 3]) -> Touch {
    let a = Side { body: Some(0), point: c.on_body, local: c.local, rows, margin: 0.04, core: Vec3::ZERO };
    let b = Side { body: None, point: c.on_world, local: c.on_world, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
    Touch { depth: c.depth, friction: c.friction, elasticity: c.elasticity, a, b, normal: c.normal, back: Vec3::ZERO - c.normal, pushed: c.pushed, kept: c.kept, target: c.target, impulse: Vec3::ZERO, kind: 0, jointed: None }
}

/// What the car is, for the props' side of a tick.
pub struct CarIn<'a> {
    pub hull: &'a HullShape,
    /// the hull bound's centre of gravity (bound +0x50) and margin
    pub hull_cg: Vec3,
    pub hull_margin: f32,
    /// the car's matrix as the collide has it, and as its last one (body +96)
    pub now: [Vec3; 4],
    pub last: [Vec3; 4],
    /// the car's collider +32: its matrix before the tick's move (against another body, F-65617)
    pub prev: [Vec3; 4],
    /// chassis +224
    pub speed: f32,
    /// the composite bound's box (min, max)
    pub bound: (Vec3, Vec3),
    /// the wheels' parts (the composite's parts 1..), None where there is none
    pub wheels: Vec<Option<WheelIn>>,
    /// (stage R, agent D) the body's velocity as the tick came in (collider +272 at the callbacks), BoundFriction /
    /// BoundElasticity, and chassis +592 bits 0x10 / 0x04
    pub vel: Vec3,
    pub touch: (f32, f32),
    pub wiped: bool,
    /// what the car's contact callback F-55998 reads of its driver and state ([`car_terms`])
    pub driver: Driver,
}

/// The car's driver and state as F-55998's materials part reads them: the player's driver record (+24 == 3), the
/// two-wheel state (mcCarGyro +18), stuck (mcCarStuck +56 == 2 with the chassis' +20 -> +20), HeadOnCGOffset (tune
/// +280). The default is the player's car on four wheels (what the port had before the AI branch was decoded).
#[derive(Clone, Copy, Debug)]
pub struct Driver {
    pub player: bool,
    pub two: bool,
    pub stuck: bool,
    pub cg_offset: f32,
}

impl Default for Driver {
    fn default() -> Self {
        Driver { player: true, two: false, stuck: false, cg_offset: 0.0 }
    }
}

/// [`car_terms`] on a contact of the car's with an instance: `fixed` its partner's level state is 2 (a vehicle never
/// is, nor a prop the loose world holds - inactive or active); the normal as it pushes the car, the car's matrix row
/// 2 and its speed above 22.35 for the head-on test; the own point is side A's when the car is A.
fn car_terms_loose(car: &CarIn, car_is_a: bool, fixed: bool, t: &mut Touch) {
    let d = car.driver;
    let n = if car_is_a { t.normal } else { Vec3::ZERO - t.normal };
    let along = car_along(n, car.now[2]);
    if let Some(x) = car_terms(d.player, d.two, car.now[1].y, car.wiped, fixed, along, car.speed > 22.35, d.stuck, (car.touch.0, car.touch.1, d.cg_offset)) {
        t.friction = x.friction;
        if let Some(e) = x.elasticity {
            t.elasticity = e;
        }
        let own = if car_is_a { &mut t.a.point } else { &mut t.b.point };
        own.y = x.drop.apply(own.y);
    }
}

/// A wheel's part of the car's bound for the props' side of a tick: its core where it stands, its matrix
/// (part x instance) now and as the last collide is to have had it - the car's last matrix with the wheel
/// at the top of its travel - and its box's half sizes.
#[derive(Clone, Copy)]
pub struct WheelIn {
    pub core: super::gjk::WheelCore,
    pub now: [Vec3; 4],
    pub last: [Vec3; 4],
    /// the part's last matrix on the car's +32 (against another body)
    pub prev: [Vec3; 4],
    pub half: Vec3,
}

/// The car's momenta while the breaking pass hands out its shares.
pub struct CarDyn<'a> {
    pub p: &'a mut Vec3,
    pub l: &'a mut Vec3,
    pub mass: f32,
    pub inverse: Vec3,
    /// the rows the turning rate is taken through (+208..)
    pub rows: [Vec3; 3],
}

/// Everything loose a car has met.
#[derive(Clone, Debug, Default)]
pub struct LooseState {
    pub objs: Vec<Obj>,
    by_inst: HashMap<u32, usize>,
    pub manis: Vec<Mani>,
    pieces_made: usize,
    /// (debug) ticks
    pub tick: u32,
    /// the cache entries made so far: each took the next instance of the pool (0x90 bytes, downwards)
    entries_made: i64,
    /// the pool's frame counter (+8: one a tick, F-89933), the place the next entry taken from the back gets
    /// (`Entry::slot`), the viewer sphere (manager +736: the camera's place and 300, F-44865 every frame; None: never
    /// set)
    pool_frame: u32,
    pool_slot: u64,
    viewer: Option<[f32; 4]>,
    /// the prop instances F-74663 took out of the level when the pool took their entry (+80 bit 0x20: not met, not drawn,
    /// F-92987) until the whole city is reset (`reset_props`)
    pub out: std::collections::BTreeSet<u32>,
    /// the level indices handed out in the update `.0` so far
    indices_taken: (i64, usize),
    /// the place of the car's own pair with the city in its island's tree (None: no such pair this tick)
    car_city_order: Option<usize>,
    /// the car's torn-off bumpers in the level (`add_piece`)
    pub pieces: Vec<Piece>,
    /// (the game) one level shared by every car: the other cars are given as vehicles before each car's
    /// step (`set_vehicles`, not integrated here), and the loose bodies' pre-collide and end of tick run
    /// once a tick - with the first car's step and the last car's (`skip_pre`, `skip_end` for the others)
    pub live: bool,
    pub skip_pre: bool,
    pub skip_end: bool,
    /// (the game) a car after the tick's first: the loose bodies' own pairs are made already
    pub later: bool,
    /// (the game) this tick's collide / refresh of what is loose has been done (the first car to meet the level
    /// does them; the original has one of each an update)
    collided: bool,
    refreshed: bool,
    /// the car is under MegaStealth (vehicles.rs `pu_ghost`, POWERUPS.md): it meets the map and nothing
    /// loose - its step leaves the level out
    pub ghost: bool,
    /// the constraints of the semis' cabs and trailers (F-15252), in the update's list order
    pub ropes: Vec<Rope>,
    /// what the ambient cars' post-solve callback (slot 25, F-13089) adds to their render instance's
    /// four side sums (+560): (the car's uid, the side 0 .. 3, the amount) - vehsim.rs, taken by
    /// vehicles.rs after the steps
    pub amb_dmg: Vec<(u32, usize, f32)>,
    /// the effects the props' events start (propfx.rs, `prop_hit_fx_rule.md`): taken by the game after the steps
    pub fx_events: Vec<FxPlay>,
    /// objects whose island a car's step solved this tick with the thrown rider and his bike (vehsim.rs): the islands no
    /// car is in that hold one are not solved again (`step_rest_rider`, which empties it)
    pub merged: Vec<usize>,
    /// (the game's level step) the other cars of the stepping car's island, whose solve and push its step runs
    pub extras: Vec<ExtraCar>,
    /// (the game's level step) the stepping car's own object in the level (the other cars' steps name it so)
    pub lead: Option<usize>,
    /// how many times `prune` has renumbered the objects, and the last renumberings (generation, old index -> new,
    /// `usize::MAX` gone): what holds an object's index across ticks outside the level (propfx.rs) catches up by them
    pub renum_gen: u64,
    pub renums: std::collections::VecDeque<(u64, Vec<usize>)>,
}

/// An effect a prop's event starts: its name and where it is played.
#[derive(Clone, Debug)]
pub struct FxPlay {
    pub effect: String,
    pub at: FxAt,
    /// the effect instance's velocity (the deflection pass gives the car's; every other event 0)
    pub vel: Vec3,
}

/// Where an event effect is placed (`F-75655`).
#[derive(Clone, Copy, Debug)]
pub enum FxAt {
    /// at a world point, unrotated (the position parameter: impact and deflection events)
    World(Vec3),
    /// on a matrix it follows each frame (the matrix-pointer parameter: the break events): the object's
    /// matrix, or the matrix of one of its bones (`part`); the offset in that frame
    Follow { obj: usize, part: Option<u16>, offset: Vec3 },
}

/// What a run does not let one derive (the level's indices and the heap's addresses), given per
/// replay: `MCLA_LOOSE_HINTS` = words `b:<prop instance>` (the prop's level index is above the
/// car's: the car is A against it), `after:<prop instance>` (its address is above the car's),
/// `pb:<n>` / `pafter:<n>` the same for the n-th piece made.
fn hint(kind: &str, n: usize) -> bool {
    static H: std::sync::OnceLock<Vec<(String, usize)>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        std::env::var("MCLA_LOOSE_HINTS")
            .unwrap_or_default()
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|w| w.split_once(':').and_then(|(k, n)| n.parse().ok().map(|n| (k.to_string(), n))))
            .collect()
    })
    .iter()
    .any(|(k, i)| k == kind && *i == n)
}

/// `MCLA_LOOSE_HINTS` words `l<object>:<rank>` / `a<object>:<rank>`: the level index / the address of
/// the n-th object a replay comes to (props as the car nears them, pieces as they are made), as a
/// rank against the car's 0.
fn rank(kind: char, obj: usize) -> Option<i64> {
    static H: std::sync::OnceLock<Vec<(char, usize, i64)>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        std::env::var("MCLA_LOOSE_HINTS")
            .unwrap_or_default()
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|w| {
                let (k, v) = w.split_once(':')?;
                let kind = k.chars().next()?;
                Some((kind, k[1..].parse().ok()?, v.parse().ok()?))
            })
            .collect()
    })
    .iter()
    .find(|x| x.0 == kind && x.1 == obj)
    .map(|x| x.2)
}

/// The update of the recording a replay is at (set by the replay's loop; i64::MIN: none).
pub static REPLAY_K: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(i64::MIN);

/// What a stage-R recording logs of the session's physics level (`MCLA_LOOSE_LEVEL` = a file that
/// `veh_diff.py` writes from the side file): the car's level index and instance address, the
/// instance address of every prop it logged (by level index), the first address of the pool the
/// cache entries' instances come from, per update the head of the level's free index list, and the
/// city's own instance (the B of every contact with the city).
#[derive(Debug, Default)]
pub struct LevelHints {
    pub car: (i64, i64),
    pub addr: HashMap<i64, i64>,
    pub pool: Option<i64>,
    pub free: HashMap<i64, Vec<i64>>,
    pub city: Option<i64>,
}

pub fn level_hints() -> Option<&'static LevelHints> {
    static H: std::sync::OnceLock<Option<LevelHints>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        let text = std::fs::read_to_string(std::env::var_os("MCLA_LOOSE_LEVEL")?).ok()?;
        let mut h = LevelHints::default();
        let num = |w: &str| if let Some(x) = w.strip_prefix("0x") { i64::from_str_radix(x, 16).ok() } else { w.parse::<i64>().ok() };
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let n: Vec<i64> = w.iter().skip(1).filter_map(|x| num(x)).collect();
            match (w.first().copied(), n.len()) {
                (Some("car"), 2..) => h.car = (n[0], n[1]),
                (Some("obj"), 2..) => {
                    h.addr.insert(n[0], n[1]);
                }
                (Some("pool"), 1..) => h.pool = Some(n[0]),
                (Some("city"), 1..) => h.city = Some(n[0]),
                (Some("free"), 2..) => {
                    h.free.insert(n[0], n[1..].to_vec());
                }
                _ => {}
            }
        }
        Some(h)
    })
    .as_ref()
}

/// `MCLA_DBG_RA1`: the level indices and addresses by the older guesses, whatever the recording logs.
fn r1() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var_os("MCLA_DBG_RA1").is_some())
}

/// `MCLA_DBG_CARLAST96`: the car against a loose body at its +96 (`CarIn::last`) in the pair's box test, swept test and
/// broad phase, the boxes at +96, as before.
fn car_last96() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var_os("MCLA_DBG_CARLAST96").is_some())
}

fn dbg() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var_os("MCLA_DBG_LOOSE").is_some())
}

/// MCLA_DBG_POOL: log every entry the pool takes from another (F-18636).
fn pool_dbg() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var_os("MCLA_DBG_POOL").is_some())
}

/// A convex part with its matrix: what the detector is handed.
#[derive(Clone, Copy)]
pub enum Core<'a> {
    Part(PartCore),
    Hull(HullCore<'a>),
    Wheel(super::gjk::WheelCore),
}

impl Support for Core<'_> {
    fn support(&self, dir: Vec3) -> Vec3 {
        match self {
            Core::Part(p) => p.support(dir),
            Core::Hull(h) => h.support(dir),
            Core::Wheel(w) => w.support(dir),
        }
    }
    fn support_at(&self, dir: Vec3, by: Vec3, then: Vec3) -> Vec3 {
        match self {
            Core::Part(p) => p.support_at(dir, by, then),
            Core::Hull(h) => h.support_at(dir, by, then),
            Core::Wheel(w) => w.support_at(dir, by, then),
        }
    }
    fn support_id(&self, dir: Vec3, by: Vec3, then: Vec3) -> (Vec3, u32) {
        match self {
            Core::Part(p) => p.support_id(dir, by, then),
            Core::Hull(h) => h.support_id(dir, by, then),
            Core::Wheel(w) => w.support_id(dir, by, then),
        }
    }
}

impl<'a> Core<'a> {
    fn placed(&self, at: Vec3) -> Core<'a> {
        match *self {
            Core::Part(p) => Core::Part(PartCore { at, ..p }),
            Core::Hull(h) => Core::Hull(HullCore { at, ..h }),
            Core::Wheel(mut w) => {
                w.centre = at;
                Core::Wheel(w)
            }
        }
    }
}

/// A part of a prop's bound at a matrix: its core, its margin, its centre of gravity.
fn part_core(b: &FragBound, m: &[Vec3; 4]) -> (PartCore, f32) {
    let rows = [m[0], m[1], m[2]];
    match b.kind {
        0 => (PartCore { kind: 0, centroid: v(b.centroid), half: Vec3::ZERO, rows, at: m[3] }, b.round),
        1 => (PartCore { kind: 1, centroid: v(b.centroid), half: Vec3::splat(b.length), rows, at: m[3] }, b.round),
        _ => {
            // (A-32016: half the box less the margin, each a rounded step)
            let half = (v(b.max) - v(b.min)) * 0.5 - Vec3::splat(b.margin);
            (PartCore { kind: 3, centroid: v(b.centroid), half, rows, at: m[3] }, b.margin)
        }
    }
}

/// What the swept test gives (F-98094): the touch's normal, the two points in the parts'
/// frames, how far through the tick it is, and the corners the hull's side ended on.
pub(super) struct Swept {
    pub(super) normal: Vec3,
    pub(super) local_a: Vec3,
    pub(super) local_b: Vec3,
    pub(super) lambda: f32,
    pub(super) last: Nearest,
}

/// The swept test of two convex parts (F-98094; p1/swept_decode.md): each is carried from
/// where its centre of gravity was to where it is, with the rows it has now; the detector is
/// asked again and again until the parts are 2 cm apart.
pub(super) fn sweep(a: &Core, ma: f32, cga: Vec3, a_now: &[Vec3; 4], a_last: &[Vec3; 4], b: &Core, mb: f32, cgb: Vec3, b_now: &[Vec3; 4], b_last: &[Vec3; 4]) -> Option<Swept> {
    let da = xf(a_now, cga) - xf(a_last, cga);
    let db = xf(b_now, cgb) - xf(b_last, cgb);
    let rel = db - da;
    let (mut pa, mut pb) = (a_now[3] - da, b_now[3] - db);
    let mut dir = Vec3::Z;
    let mut r = convex_contact(&a.placed(pa), ma, &b.placed(pb), mb, &mut dir, true)?;
    let (mut lambda, mut last) = (0.0f32, 0.0f32);
    let tol = f32::from_bits(0x3a83_126f);
    let stop = f32::from_bits(0x3ca3_d70a);
    if r.distance > tol {
        let mut gap = r.distance;
        let mut steps = 0;
        loop {
            steps += 1;
            if steps > 32 {
                return None;
            }
            let proj = dot3(rel, r.normal) + 0.0;
            let step = gap * if proj == 0.0 { f32::MAX } else { recip(proj) };
            lambda = lambda + step;
            if 0.0 > step || lambda > 1.0 {
                return None;
            }
            if last >= lambda {
                break;
            }
            last = lambda;
            let k = 1.0 - lambda;
            pb = -(db * k - b_now[3]);
            pa = -(da * k - a_now[3]);
            r = convex_contact(&a.placed(pa), ma, &b.placed(pb), mb, &mut dir, true)?;
            if stop >= r.distance {
                break;
            }
            gap = r.distance - stop;
            if !(gap > tol) {
                break;
            }
        }
    }
    let at = |m: &[Vec3; 4], p: Vec3| [m[0], m[1], m[2], p];
    Some(Swept { normal: r.normal, local_a: inv(&at(a_now, pa), r.on_a - pa), local_b: inv(&at(b_now, pb), r.on_b - pb), lambda, last: r })
}

/// The swept test of a convex part against one city polygon (F-35118's swept branch, F-98094 inside): the
/// part's places now and last taken about the polygon's first corner `origin`, the mesh at rest at the origin
/// (its move is nil), the polygon `local` about that corner. The detector starts from (1, 0, 0) and keeps its
/// direction from one call to the next; the touch's normal, the part's point carried to its place now, the
/// polygon's point, and the distance between them along the normal - about `origin`, as the discrete detector
/// answers. (No "lambda below 1" test after it here, unlike the part pairs' F-36478.)
fn sweep_polygon(core: &Core, margin: f32, cg: Vec3, now: &[Vec3; 4], last: &[Vec3; 4], local: &[Vec3; 3], face: Vec3, bisectors: [Vec3; 3], origin: Vec3) -> Option<Nearest> {
    let now = [now[0], now[1], now[2], now[3] - origin];
    let last = [last[0], last[1], last[2], last[3] - origin];
    let da = xf(&now, cg) - xf(&last, cg);
    let rel = Vec3::ZERO - da;
    let mut pa = now[3] - da;
    let mut dir = Vec3::X;
    let pm = super::hull::POLYGON_MARGIN;
    let mut r = super::gjk::contact_local(&core.placed(pa), Vec3::ZERO, local, face, bisectors, margin, pm, &mut dir)?;
    let (mut lambda, mut last_l) = (0.0f32, 0.0f32);
    let tol = f32::from_bits(0x3a83_126f);
    let stop = f32::from_bits(0x3ca3_d70a);
    if r.distance > tol {
        let mut gap = r.distance;
        let mut steps = 0;
        loop {
            steps += 1;
            if steps > 32 {
                return None;
            }
            let proj = dot3(rel, r.normal) + 0.0;
            let step = gap * if proj == 0.0 { f32::MAX } else { recip(proj) };
            lambda = lambda + step;
            if 0.0 > step || lambda > 1.0 {
                return None;
            }
            if last_l >= lambda {
                break;
            }
            last_l = lambda;
            pa = -(da * (1.0 - lambda) - now[3]);
            r = super::gjk::contact_local(&core.placed(pa), Vec3::ZERO, local, face, bisectors, margin, pm, &mut dir)?;
            if stop >= r.distance {
                break;
            }
            gap = r.distance - stop;
            if !(gap > tol) {
                break;
            }
        }
    }
    let la = inv(&[now[0], now[1], now[2], pa], r.on_a - pa);
    let (on_a, on_b) = (xf(&now, la), r.on_b);
    Some(Nearest { on_a, on_b, distance: 0.0 - dot3(r.normal, on_b - on_a), ..r })
}

impl LooseState {
    /// The object a prop instance is, made on first need.
    fn obj_of(&mut self, world: &LooseWorld, inst: u32) -> usize {
        if let Some(&i) = self.by_inst.get(&inst) {
            return i;
        }
        let (fi, ty) = &world.instances[inst as usize];
        let i = self.objs.len();
        let n = inst as usize;
        if let Some(h) = level_hints().filter(|_| !r1()) {
            // (the level's order is the file's: the city 0, then every instance that is put in; the address is
            // the heap's)
            let level = 1 + n as i64;
            let addr = h.addr.get(&level).copied().unwrap_or(0xA000_0000 + 0x100 * n as i64);
            self.objs.push(Obj { ty: *ty, inst: Some(inst), matrix: m4(&fi.matrix), entry: None, hit: false, body: None, level, addr, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
            self.by_inst.insert(inst, i);
            return i;
        }
        self.objs.push(Obj {
            ty: *ty,
            inst: Some(inst),
            matrix: m4(&fi.matrix),
            entry: None,
            hit: false,
            body: None,
            level: rank('l', i).unwrap_or(if hint("b", n) { 1 + n as i64 } else { -1_000_000_000 + n as i64 }),
            addr: rank('a', i).unwrap_or(if hint("after", n) { 1 + n as i64 } else { -1_000_000_000 + n as i64 }),
            veh: None,
            sleep: Sleep::reset(),
            dormant: false,
            amb: None,
        });
        self.by_inst.insert(inst, i);
        i
    }

    /// Is part `c` of an object there to be met?
    fn has_part(&self, world: &LooseWorld, o: usize, c: usize) -> bool {
        let obj = &self.objs[o];
        if obj.veh.is_some() {
            return c == 0;
        }
        part_present(&world.types[obj.ty], obj.entry.map_or(0, |e| e.absent), c)
    }

    /// The centre of gravity of an object's whole bound (bound +80).
    fn cg_of(&self, world: &LooseWorld, o: usize) -> Vec3 {
        let obj = &self.objs[o];
        if let Some(vh) = obj.veh {
            return vh.cg;
        }
        obj.entry.map_or(v(world.types[obj.ty].cg), |e| e.cg)
    }

    /// An object's whole bound (F-65996 / F-32566 for an entry that has lost groups: the union of its
    /// parts' boxes through their matrices, centroid = the box's middle, radius = its half diagonal; else the
    /// file's): box min, max, centroid, radius.
    fn whole_bound(&self, world: &LooseWorld, o: usize) -> (Vec3, Vec3, Vec3, f32) {
        let obj = &self.objs[o];
        if let Some(vh) = obj.veh {
            let (mut lo, mut hi) = (vh.shape.min - Vec3::splat(0.041), vh.shape.max + Vec3::splat(0.041));
            if let Some(h) = vh.shape2 {
                lo = lo.min(h.min - Vec3::splat(0.041));
                hi = hi.max(h.max + Vec3::splat(0.041));
            }
            let c = (hi + lo) * 0.5;
            return (lo, hi, c, (hi - c).length());
        }
        let t = &world.types[obj.ty];
        let absent = obj.entry.map_or(0, |e| e.absent);
        if absent == 0 {
            return (v(t.min), v(t.max), v(t.centroid), t.radius);
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(-f32::MAX));
        let mut any = false;
        for (c, part) in t.parts.iter().enumerate().rev() {
            let Some(b) = part.bound.as_ref().filter(|_| part_present(t, absent, c)) else { continue };
            let m = m4(&part.matrix);
            let mid = (v(b.min) + v(b.max)) * 0.5;
            let half = v(b.max) - mid;
            let centre = xf(&m, mid);
            let ext = Vec3::new(dot3(Vec3::new(m[0].x.abs(), m[1].x.abs(), m[2].x.abs()), half), dot3(Vec3::new(m[0].y.abs(), m[1].y.abs(), m[2].y.abs()), half), dot3(Vec3::new(m[0].z.abs(), m[1].z.abs(), m[2].z.abs()), half));
            lo = lo.min(centre - ext);
            hi = hi.max(ext + centre);
            any = true;
        }
        if !any {
            return (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 0.0);
        }
        let centroid = (hi + lo) * 0.5;
        let d = hi - centroid;
        (lo, hi, centroid, fma(d.z, d.z, fma(d.y, d.y, d.x * d.x)).sqrt())
    }

    /// The level's broad phase for an ACTIVE object `own` and another (F-84048): own's sphere, moved half
    /// the tick's way back and grown by half its length, against the other's as it stands; then the two whole
    /// boxes - each grown to hold itself at its matrix before the move, if it is active - along their six face
    /// axes. Each side: box min / max, centroid, radius, matrix now, matrix before the move (None: inactive).
    fn broad(own: &BroadSide, other: &BroadSide) -> bool {
        let [cx, cy, cz, radius] = broad_sphere(own);
        let centre = Vec3::new(cx, cy, cz);
        let oc = xf(&other.4, other.2);
        let dd = centre - oc;
        let sum = radius + other.3;
        if !(sum * sum > dot3(dd, dd)) {
            return false;
        }
        broad_boxes(own, other)
    }

    fn broad_side(&self, world: &LooseWorld, o: usize) -> BroadSide {
        let (lo, hi, c, r) = self.whole_bound(world, o);
        (lo, hi, c, r, self.objs[o].matrix, self.objs[o].body.as_ref().map(|b| b.prev))
    }

    /// One side's part as the detector needs it: its core and margin, its bound's centre of
    /// gravity and box, its matrix now and as the last collide is to have had it.
    fn shape<'a>(&self, world: &'a LooseWorld, car: &CarIn<'a>, who: Who, part: u16) -> Option<Shape<'a>> {
        match who {
            Who::Car if part != 0 => {
                // (RA7: a wheel's part, the composite's parts 1..)
                let w = car.wheels.get(part as usize - 1).copied().flatten()?;
                Some(Shape { core: Core::Wheel(w.core), margin: super::gjk::WHEEL_MARGIN, cg: Vec3::ZERO, bbox: (Vec3::ZERO - w.half, w.half), now: w.now, last: w.last })
            }
            Who::Car => {
                let core = Core::Hull(HullCore { corners: car.hull.scanned(), rows: [car.now[0], car.now[1], car.now[2]], at: car.now[3] });
                Some(Shape { core, margin: car.hull_margin, cg: car.hull_cg, bbox: (car.hull.min - Vec3::splat(0.041), car.hull.max + Vec3::splat(0.041)), now: car.now, last: car.last })
            }
            Who::Obj(o) => {
                let obj = &self.objs[o];
                if let Some(vh) = obj.veh {
                    // (stage R: a vehicle's one polyhedron, A-51822 as the car's hull; an ambient
                    // trailer's composite: each of its two, at the identity part matrix)
                    if part >= vh.parts() || !vh.here {
                        return None;
                    }
                    let now = obj.matrix;
                    let h = vh.part(part);
                    let core = Core::Hull(HullCore { corners: h.scanned(), rows: [now[0], now[1], now[2]], at: now[3] });
                    return Some(Shape { core, margin: f32::from_bits(0x3d23_d70a), cg: vh.cg, bbox: (h.min - Vec3::splat(0.041), h.max + Vec3::splat(0.041)), now, last: obj.body.map_or(obj.matrix, |b| b.last) });
                }
                let t = &world.types[obj.ty];
                if !part_present(t, obj.entry.map_or(0, |e| e.absent), part as usize) {
                    return None;
                }
                let p = &t.parts[part as usize];
                let b = p.bound.as_ref()?;
                if !matches!(b.kind, 0 | 1 | 3 | 4) {
                    return None;
                }
                let pm = m4(&p.matrix);
                let now = super::gjk::world_matrix(&pm, &obj.matrix);
                let last = super::gjk::world_matrix(&pm, &obj.body.map_or(obj.matrix, |b| b.last));
                // (a polyhedron part, A-51822: as a vehicle's hull, its own corners and margin)
                if b.kind == 4 {
                    let corners = world.polyhedra.get(&(obj.ty, part as usize))?;
                    let core = Core::Hull(HullCore { corners, rows: [now[0], now[1], now[2]], at: now[3] });
                    return Some(Shape { core, margin: b.margin, cg: v(b.cg), bbox: (v(b.min), v(b.max)), now, last });
                }
                let (core, margin) = part_core(b, &now);
                Some(Shape { core: Core::Part(core), margin, cg: v(b.cg), bbox: (v(b.min), v(b.max)), now, last })
            }
            Who::City => None,
        }
    }

    /// A loose body's part at the matrix the body had as the tick began (its collider's +32; `PBody::prev`).
    fn prev_part(&self, world: &LooseWorld, who: Who, part: u16) -> Option<[Vec3; 4]> {
        let Who::Obj(o) = who else { return None };
        let b = self.objs[o].body.as_ref()?;
        let p = world.types[self.objs[o].ty].parts.get(part as usize)?;
        Some(super::gjk::world_matrix(&m4(&p.matrix), &b.prev))
    }

    fn parts_of(&self, world: &LooseWorld, car: &CarIn, who: Who) -> Vec<u16> {
        match who {
            Who::Obj(o) if self.objs[o].veh.is_some() => (0..self.objs[o].veh.unwrap().parts()).collect(),
            Who::Obj(o) => (0..world.types[self.objs[o].ty].parts.len() as u16).collect(),
            Who::Car => (0..=car.wheels.len() as u16).collect(),
            _ => vec![0],
        }
    }

    /// The car's level index and its instance's address (0, 0 where no recording says).
    pub fn car_place(&self) -> (i64, i64) {
        level_hints().filter(|_| !r1()).map_or((0, 0), |h| h.car)
    }

    /// The city instance's address (below every other where no recording says: the recorded ones are).
    fn city_addr(&self) -> i64 {
        level_hints().filter(|_| !r1()).and_then(|h| h.city).unwrap_or(i64::MIN)
    }

    /// The instance a new cache entry gets (0x90 bytes each, from the top of their pool downwards).
    fn entry_instance(&mut self) -> Option<i64> {
        let n = self.entries_made;
        self.entries_made += 1;
        level_hints().filter(|_| !r1()).and_then(|h| h.pool).map(|top| top - 0x90 * n)
    }

    /// The level index a new object gets: the next of the level's free index list (F-72385).
    fn new_index(&mut self) -> Option<i64> {
        let h = level_hints().filter(|_| !r1())?;
        let k = REPLAY_K.load(std::sync::atomic::Ordering::Relaxed);
        if self.indices_taken.0 != k {
            self.indices_taken = (k, 0);
        }
        let n = self.indices_taken.1;
        self.indices_taken.1 += 1;
        h.free.get(&k).and_then(|f| f.get(n)).copied()
    }

    fn level(&self, who: Who) -> i64 {
        match who {
            Who::Car => self.car_place().0,
            Who::Obj(o) => self.objs[o].level,
            Who::City => i64::MIN,
        }
    }

    /// The contacts of two objects' parts (F-82759 with F-36478 per pair of parts): B's
    /// parts outside, A's inside; a pair of parts whose boxes meet, with no contact yet and one
    /// of them a prop that has not broken, goes through the swept test, every other through the
    /// detector.
    fn pair(&mut self, world: &LooseWorld, car: &CarIn, a: Who, b: Who) {
        // (record byte +280: an mcPropInst that stands loose has flag 2; a piece's own instance none)
        let is_prop = |w: Who| matches!(w, Who::Obj(o) if self.objs[o].inst.is_some());
        let sweeps = (is_prop(a) || is_prop(b)) && !(hint("x", 6) && a != Who::Car && b != Who::Car);
        for pb in self.parts_of(world, car, b) {
            let Some(sb) = self.shape(world, car, b, pb) else { continue };
            for pa in self.parts_of(world, car, a) {
                // (stage R: only the car's hull is taken against a vehicle - no wheel contact with one is kept
                // in the recordings)
                let veh_pair = [a, b].into_iter().any(|w| matches!(w, Who::Obj(o) if self.objs[o].veh.is_some()));
                if veh_pair && ((a == Who::Car && pa != 0) || (b == Who::Car && pb != 0)) {
                    continue;
                }
                let Some(sa) = self.shape(world, car, a, pa) else { continue };
                let at = self.manis.iter().position(|m| m.a == a && m.b == b && m.part_a == pa && m.part_b == pb);
                // (the instances' last matrices, F-65617: the collider's +96 only while the other instance has no
                // collider record - two bodies in the simulator are each taken at +32, the matrix they had as the tick
                // began: V s_rtv s390 / s391, 11286 x 22286 / 22320)
                // (the car is a body too: against a loose body both are taken at +32 (F-65617's input +128 / +192 - the
                // car's collider +32 is its matrix before the tick's move, `CarIn::prev`), and so is the box test of the
                // parts (F-36478: Plast from the same input); MCLA_DBG_CARLAST96: the car at +96 and the boxes at +96, as before)
                let old96 = car_last96();
                let body_side = |w: Who| match w {
                    Who::Car => !old96,
                    Who::Obj(o) => self.objs[o].body.is_some() && self.objs[o].veh.is_none(),
                    Who::City => false,
                };
                let both = [a, b].into_iter().all(|w| body_side(w));
                let car_prev = |part: u16, last: [Vec3; 4]| if part == 0 { car.prev } else { car.wheels.get(part as usize - 1).copied().flatten().map_or(last, |w| w.prev) };
                let prev_of = |w: Who, part: u16, last: [Vec3; 4]| if w == Who::Car { car_prev(part, last) } else { self.prev_part(world, w, part).unwrap_or(last) };
                // (and an active side at +32 when the other is inactive with level byte bit 8 - manifold +1249 / +1250,
                // an ambient car with inst +140 bit 0x80 moved this frame)
                let posed = |w: Who| matches!(w, Who::Obj(o) if self.objs[o].body.is_none() && self.objs[o].amb.as_ref().is_some_and(|x| x.posed));
                let prop_body = |w: Who| matches!(w, Who::Obj(o) if self.objs[o].body.is_some() && self.objs[o].veh.is_none());
                let (la, lb) = if both {
                    (prev_of(a, pa, sa.last), prev_of(b, pb, sb.last))
                } else {
                    let la = if posed(b) && prop_body(a) { self.prev_part(world, a, pa).unwrap_or(sa.last) } else { sa.last };
                    let lb = if posed(a) && prop_body(b) { self.prev_part(world, b, pb).unwrap_or(sb.last) } else { sb.last };
                    (la, lb)
                };
                let meet = if old96 { boxes_meet(&sa, &sb) } else { parts_meet((sa.bbox, sa.now, la), (sb.bbox, sb.now, lb)) };
                if !meet && !hint("x", 7) {
                    continue;
                }
                if let Some(k) = at {
                    self.manis[k].visited = true;
                }
                let count = at.map_or(0, |k| self.manis[k].slots.len());
                let found: Option<(Vec3, Vec3, Vec3, f32, Nearest)> = if count == 0 && sweeps {
                    sweep(&sa.core, sa.margin, sa.cg, &sa.now, &la, &sb.core, sb.margin, sb.cg, &sb.now, &lb).filter(|s| !(s.lambda >= 1.0)).map(|s| {
                        let (qa, qb) = (xf(&sa.now, s.local_a), xf(&sb.now, s.local_b));
                        (s.normal, qa, qb, 0.0 - dot3(s.normal, qb - qa), s.last)
                    })
                } else {
                    let mut dir = Vec3::Z;
                    convex_contact(&sa.core, sa.margin, &sb.core, sb.margin, &mut dir, true).map(|r| (r.normal, r.on_a, r.on_b, r.distance, r))
                };
                if dbg() {
                    eprintln!("LOOSEPAIR tick {} {:?}/{pa} x {:?}/{pb} swept {} -> {:?}", self.tick, a, b, count == 0 && sweeps, found.map(|f| (f.0, f.3)));
                }
                let Some((normal, qa, qb, dist, r)) = found else { continue };
                // (the sink, F-90136: nothing farther than 2 cm)
                if !(f32::from_bits(0x3ca3_d70a) > dist) {
                    continue;
                }
                let (local_a, local_b) = (inv(&sa.now, qa - sa.now[3]), inv(&sb.now, qb - sb.now[3]));
                if dbg() {
                    eprintln!("LOOSECON tick {} {:?}/{pa} x {:?}/{pb} depth {:?} la {:?} lb {:?} pa {:?} pb {:?} n {:?}", self.tick, a, b, 0.0 - dist, local_a, local_b, qa, qb, normal);
                }
                // (which of the hull's triangles: by the corners the loop ended on and the
                // contact's direction in the hull's frame - away from the other part)
                let element = if (b == Who::Car && pb != 0) || (a == Who::Car && pa != 0) {
                    0
                } else if b == Who::Car {
                    car.hull.element(&r.corners_b.0[..r.corners_b.1], inv(&car.now, normal))
                } else if a == Who::Car {
                    car.hull.element(&r.corners.0[..r.corners.1], inv(&car.now, Vec3::ZERO - normal))
                } else {
                    0
                };
                if dbg() && ((a == Who::Car && pa == 0) || (b == Who::Car && pb == 0)) {
                    eprintln!("LOOSEELEM tick {} element {element} deep {} corners {:?} / {:?}", self.tick, r.deep, &r.corners.0[..r.corners.1], &r.corners_b.0[..r.corners_b.1]);
                }
                // (stage R: and which of a vehicle's - side B: the direction +n in its frame)
                let veh_surface = match (a, b) {
                    (_, Who::Obj(o)) if self.objs[o].veh.is_some() => Some(self.objs[o].veh.unwrap().part(pb)).map(|h| h.surface_of(h.element(&r.corners_b.0[..r.corners_b.1], inv(&sb.now, normal)))),
                    (Who::Obj(o), _) if self.objs[o].veh.is_some() && self.objs[o].amb.is_none() => Some(self.objs[o].veh.unwrap().part(pa)).map(|h| h.surface_of(h.element(&r.corners.0[..r.corners.1], inv(&sa.now, Vec3::ZERO - normal)))),
                    _ => None,
                };
                // (an ambient car as A: its own triangle's material, kept apart from B's)
                let amb_surface = match a {
                    Who::Obj(o) if self.objs[o].amb.is_some() => Some(self.objs[o].veh.unwrap().part(pa)).map(|h| h.surface_of(h.element(&r.corners.0[..r.corners.1], inv(&sa.now, Vec3::ZERO - normal)))),
                    _ => None,
                };
                let side = |local: Vec3, point: Vec3| Side { body: None, point, local, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
                let fresh = Touch { depth: 0.0 - dist, a: side(local_a, qa), b: side(local_b, qb), normal, back: Vec3::ZERO - normal, ..Default::default() };
                let k = at.unwrap_or_else(|| {
                    self.manis.push(Mani { a, b, part_a: pa, part_b: pb, cell: 0, stale: false, visited: true, island: 0, order: 0, slots: Vec::new(), rope: false, owner: 0 });
                    self.manis.len() - 1
                });
                self.manis[k].insert(fresh, element, veh_surface.unwrap_or(Surface::Prop), amb_surface.unwrap_or(Surface::Prop));
            }
        }
    }

    /// The tick's collide of everything loose (F-27744): the car's hull against the props and
    /// pieces about it, the pieces against each other and against the props they reach, and
    /// every loose body's parts against the city.
    pub fn collide(&mut self, world: &LooseWorld, car: &CarIn, city: &super::collision::WorldCollider, car_city: bool) {
        // (the manifolds of a pair whose instance broke last tick went with the pair)
        self.manis.retain(|m| !m.stale);
        // (a level shared by the cars: what is loose meets what is loose and the city once a tick, in the first
        // car's collide; the later cars' keep those pairs as they are)
        let shared_later = self.live && self.later && self.collided;
        self.collided = true;
        for m in self.manis.iter_mut() {
            m.visited = shared_later && m.a != Who::Car && m.b != Who::Car;
        }
        // (the constraints first, once an update: F-87905 ahead of the collide)
        if !shared_later {
            self.ropes_touch();
        }
        // (the level's props about the car and about every loose body)
        let mut about = vec![car.now[3]];
        // (and about an inactive ambient car with level byte bit 8: its pairs with inactive props)
        about.extend(self.objs.iter().filter(|o| o.body.is_some() || o.amb.as_ref().is_some_and(|a| a.posed)).map(|o| o.matrix[3]));
        for (k, p) in about.into_iter().enumerate() {
            for inst in world.near(p, 8.0) {
                let (fi, ty) = &world.instances[inst as usize];
                // (only what can be knocked: level state 1)
                // (nor what the pool took out of the level, F-74663)
                if fi.state != 1 || self.out.contains(&inst) {
                    continue;
                }
                let t = &world.types[*ty];
                let centre = xf(&m4(&fi.matrix), v(t.centroid));
                let reach = t.radius + if k == 0 { 3.2 } else { 1.5 };
                // (stage R, agent D: MCLA_REPLAY_OTHERS_ONLY - a one-step check of vehicle ticks takes no prop in:
                // what this port's own run knocked loose before the checked ticks is not the recording's)
                if (centre - p).length_squared() <= reach * reach && !others_only() {
                    self.obj_of(world, inst);
                }
            }
        }
        // (the pairs, the lower level index A)
        let n = self.objs.len();
        let mut pairs: Vec<(Who, Who)> = Vec::new();
        let ball = |o: &Obj| {
            // (an ambient car: its bound's box)
            if let Some(vh) = o.veh {
                let (mut lo, mut hi) = (vh.shape.min, vh.shape.max);
                if let Some(h) = vh.shape2 {
                    lo = lo.min(h.min);
                    hi = hi.max(h.max);
                }
                return (xf(&o.matrix, (lo + hi) * 0.5), ((hi - lo) * 0.5).length());
            }
            let t = &world.types[o.ty];
            (xf(&o.matrix, v(t.centroid)), t.radius)
        };
        // (RA8: the pairs by the level's own test, F-84048 - MCLA_DBG_RA8: by plain balls, as before)
        let exact_broad = !super::vehsim::dbg_flag("MCLA_DBG_RA8");
        let car_side = {
            let (lo, hi) = car.bound;
            let c = (hi + lo) * 0.5;
            let d = hi - c;
            // (the car's collider +32 (+80 its place), F-84048, as any active object's)
            (lo, hi, c, fma(d.z, d.z, fma(d.y, d.y, d.x * d.x)).sqrt(), car.now, Some(if car_last96() { car.last } else { car.prev }))
        };
        for o in 0..n {
            if let Some(vh) = self.objs[o].veh {
                // (stage R: a vehicle about the car - the parts' boxes decide)
                if vh.here && vh.pair {
                    pairs.push(if self.objs[o].level < self.car_place().0 { (Who::Obj(o), Who::Car) } else { (Who::Car, Who::Obj(o)) });
                }
                continue;
            }
            let (c, r) = ball(&self.objs[o]);
            let near = if exact_broad {
                // (two active ones: the lower index asks)
                let side = self.broad_side(world, o);
                if side.5.is_some() && self.objs[o].level < self.car_place().0 { Self::broad(&side, &car_side) } else { Self::broad(&car_side, &side) }
            } else {
                (c - car.now[3]).length_squared() <= (r + 3.2) * (r + 3.2)
            };
            if near {
                pairs.push(if self.objs[o].level < self.car_place().0 { (Who::Obj(o), Who::Car) } else { (Who::Car, Who::Obj(o)) });
            }
        }
        // (a pair is made only with a body in it: walked from the bodies - the level keeps every prop a
        // car came near - and taken in the order of the two indices, as a walk over every pair would;
        // two inactive objects only when one has level byte bit 8, F-27744: an ambient car with
        // inst +140 bit 0x80 moved this frame, `ambient_posed`)
        let posed = |o: &Obj| o.body.is_none() && o.amb.as_ref().is_some_and(|a| a.posed);
        let start = |o: &Obj| o.body.is_some() || posed(o);
        let mut cand: Vec<(usize, usize)> = Vec::new();
        for o in 0..if shared_later { 0 } else { n } {
            if !start(&self.objs[o]) || (self.objs[o].veh.is_some() && self.objs[o].amb.is_none()) {
                continue;
            }
            for q in 0..n {
                // (a car whose pairs another car's step makes: not here)
                if self.objs[q].veh.is_some_and(|v| !v.pair && v.here) && self.objs[q].amb.is_none() {
                    continue;
                }
                if q != o && !(q < o && start(&self.objs[q])) {
                    cand.push((o.min(q), o.max(q)));
                }
            }
        }
        cand.sort_unstable();
        for (o1, o2) in cand {
            {
                let (a, b) = (&self.objs[o1], &self.objs[o2]);
                // (stage R: vehicle x prop, vehicle x vehicle: not ported - but an ambient car's, with what is
                // loose and with another ambient car, active or not)
                if (a.veh.is_some() && a.amb.is_none()) || (b.veh.is_some() && b.amb.is_none()) {
                    continue;
                }
                if a.body.is_none() && b.body.is_none() && !posed(a) && !posed(b) {
                    continue;
                }
                let ((ca, ra), (cb, rb)) = (ball(a), ball(b));
                if exact_broad {
                    let (sa, sb) = (self.broad_side(world, o1), self.broad_side(world, o2));
                    let found = match (sa.5.is_some(), sb.5.is_some()) {
                        (true, true) => if a.level < b.level { Self::broad(&sa, &sb) } else { Self::broad(&sb, &sa) },
                        (true, false) => Self::broad(&sa, &sb),
                        _ => Self::broad(&sb, &sa),
                    };
                    if !found {
                        continue;
                    }
                } else if (ca - cb).length_squared() > (ra + rb + 0.3) * (ra + rb + 0.3) {
                    continue;
                }
                pairs.push(if a.level < b.level { (Who::Obj(o1), Who::Obj(o2)) } else { (Who::Obj(o2), Who::Obj(o1)) });
            }
        }
        for (a, b) in pairs {
            self.pair(world, car, a, b);
        }
        // (every loose body's parts against the city)
        for o in 0..if shared_later { 0 } else { n } {
            // (stage R: a vehicle's own contacts with the city are not made - an ambient car's are)
            if self.objs[o].body.is_none() || (self.objs[o].veh.is_some() && self.objs[o].amb.is_none()) {
                continue;
            }
            for c in self.parts_of(world, car, Who::Obj(o)) {
                let Some(s) = self.shape(world, car, Who::Obj(o), c) else { continue };
                let mut hits: Vec<super::hull::PartHit> = Vec::new();
                if dbg() && self.objs[o].veh.is_none() {
                    let t = &world.types[self.objs[o].ty];
                    let p = &t.parts[c as usize];
                    if let Some(bd) = p.bound.as_ref() {
                        eprintln!("LOOSESHAPE tick {} obj {o} part {c} kind {} round {:e} len {:e} margin {:e} centroid {:?} min {:?} max {:?} pm {:?} now {:?} last {:?}", self.tick, bd.kind, bd.round, bd.length, bd.margin, bd.centroid, bd.min, bd.max, p.matrix, s.now, s.last);
                    }
                }
                // (pair byte +280: a loose prop's every polygon goes through the swept test - V s_rtv 11286, s386 / s389 /
                // s391: the city's points lie where the part was first within 2 cm, 4 to 7 cm along the ground)
                let sweeps = self.objs[o].inst.is_some();
                let swept = |local: &[Vec3; 3], face: Vec3, bisectors: [Vec3; 3], origin: Vec3| sweep_polygon(&s.core, s.margin, s.cg, &s.now, &s.last, local, face, bisectors, origin);
                super::hull::part_touches(&s.core, s.margin, s.bbox, &s.now, &s.last, city, if sweeps { Some(&swept) } else { None }, |h| hits.push(h));
                for m in self.manis.iter_mut() {
                    if m.a == Who::Obj(o) && m.b == Who::City && m.part_a == c {
                        m.visited = true;
                    }
                }
                let own = self.objs[o].veh.filter(|_| self.objs[o].amb.is_some()).map(|vh| vh.part(c));
                for h in hits {
                    let surface_a = own.map_or(Surface::Prop, |sh| sh.surface_of(sh.element(&h.corners.0[..h.corners.1], inv(&s.now, Vec3::ZERO - h.normal))));
                    let a = Side { body: None, point: h.on_a, local: h.local, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
                    let b = Side { body: None, point: h.on_b, local: h.on_b, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
                    let fresh = Touch { depth: h.depth, a, b, normal: h.normal, back: Vec3::ZERO - h.normal, ..Default::default() };
                    let k = self.manis.iter().position(|m| m.a == Who::Obj(o) && m.b == Who::City && m.part_a == c && m.cell == h.cell).unwrap_or_else(|| {
                        self.manis.push(Mani { a: Who::Obj(o), b: Who::City, part_a: c, part_b: 0, cell: h.cell, stale: false, visited: true, island: 0, order: 0, slots: Vec::new(), rope: false, owner: 0 });
                        self.manis.len() - 1
                    });
                    if dbg() {
                        eprintln!("LOOSECITY tick {} obj {o} part {c} cell {} polygon {} depth {:?} n {:?} local {:?} on_a {:?} on_b {:?}", self.tick, h.cell, h.polygon, h.depth, h.normal, h.local, h.on_a, h.on_b);
                    }
                    self.manis[k].insert(fresh, 0, h.surface, surface_a);
                }
            }
        }
        self.manis.retain(|m| m.visited && !m.slots.is_empty());
        // (an inactive ambient car in a manifold with contacts is activated - no threshold, before the
        // islands and the solver: F-27744 at A-28475 -> F-24099)
        let woken: Vec<usize> = self.manis.iter().filter(|m| !m.rope).flat_map(|m| [m.a, m.b]).filter_map(|w| if let Who::Obj(o) = w { Some(o) } else { None }).filter(|&o| self.objs[o].amb.is_some() && self.objs[o].body.is_none()).collect();
        for o in woken {
            self.ambient_activate(o);
        }
        // (the simulator's list emptied after the collide: level byte bit 8 cleared, F-96197 -> F-79250)
        if !shared_later {
            for o in self.objs.iter_mut() {
                if let Some(a) = o.amb.as_mut() {
                    a.posed = false;
                }
            }
        }
        // (the islands, F-14298 at the end of the collide: every pair that holds a contact NOW joins its
        // two objects - a contact that the update's refresh then drops has still joined them. The pairs come
        // in the cache's order, (lower, higher level index); each is a node of a union-find forest whose
        // roots are the islands; FIND F-28830 hangs every node it passes under the root, at the tail of
        // the root's children; UNION F-91971 by rank. The update walks each tree in pre-order.)
        self.build_islands(car_city);
    }

    /// The islands of the tick's manifolds (F-14298, see [`Self::collide`]): each manifold's island (the car's 0) and
    /// its pair's place in the island's tree.
    fn build_islands(&mut self, car_city: bool) {
        let n_obj = self.objs.len() + 1;
        let node = |w: Who| match w {
            Who::Car => Some(0usize),
            Who::Obj(o) => Some(o + 1),
            Who::City => None,
        };
        let mut pairs: Vec<((i64, i64), Who, Who)> = Vec::new();
        if car_city {
            pairs.push(((i64::MIN, self.car_place().0), Who::Car, Who::City));
        }
        for m in self.manis.iter().filter(|m| !m.rope) {
            let (la, lb) = (self.level(m.a), self.level(m.b));
            let key = (la.min(lb), la.max(lb));
            if !pairs.iter().any(|p| p.0 == key) {
                pairs.push((key, m.a, m.b));
            }
        }
        pairs.sort_by_key(|p| p.0);
        // (the constraints' unions come first: F-87905 joins its pair before the collide)
        let mut first: Vec<((i64, i64), Who, Who)> = Vec::new();
        for m in self.manis.iter().filter(|m| m.rope) {
            let (la, lb) = (self.level(m.a), self.level(m.b));
            let key = (la.min(lb), la.max(lb));
            if !first.iter().any(|p| p.0 == key) {
                first.push((key, m.a, m.b));
            }
        }
        pairs.retain(|p| !first.iter().any(|q| q.0 == p.0));
        first.extend(pairs);
        let pairs = first;
        let np = pairs.len();
        let mut parent: Vec<Option<usize>> = vec![None; np];
        let mut kids: Vec<Vec<usize>> = vec![Vec::new(); np];
        let mut rank = vec![0u32; np];
        let mut roots: Vec<usize> = Vec::new();
        let mut link: Vec<Option<usize>> = vec![None; n_obj];
        fn rehang(parent: &mut [Option<usize>], kids: &mut [Vec<usize>], roots: &mut Vec<usize>, nd: usize, to: usize) {
            if parent[nd] == Some(to) {
                return;
            }
            match parent[nd] {
                Some(p) => kids[p].retain(|x| *x != nd),
                None => roots.retain(|x| *x != nd),
            }
            parent[nd] = Some(to);
            kids[to].push(nd);
        }
        fn find_root(parent: &mut [Option<usize>], kids: &mut [Vec<usize>], roots: &mut Vec<usize>, x: usize) -> usize {
            if parent[x].is_none() {
                return x;
            }
            let mut r = x;
            while let Some(p) = parent[r] {
                r = p;
            }
            let mut nd = x;
            while nd != r {
                let next = parent[nd].unwrap();
                rehang(parent, kids, roots, nd, r);
                nd = next;
            }
            r
        }
        fn union(parent: &mut [Option<usize>], kids: &mut [Vec<usize>], roots: &mut Vec<usize>, rank: &mut [u32], x: usize, y: usize) -> usize {
            if x == y {
                return x;
            }
            if rank[x] > rank[y] {
                rehang(parent, kids, roots, y, x);
                x
            } else if rank[x] < rank[y] {
                rehang(parent, kids, roots, x, y);
                y
            } else {
                rehang(parent, kids, roots, y, x);
                rank[x] += 1;
                x
            }
        }
        for k in 0..np {
            roots.push(k);
            let mut r = find_root(&mut parent, &mut kids, &mut roots, k);
            let (a, b) = (node(pairs[k].1), node(pairs[k].2));
            if let Some(a) = a {
                if let Some(l) = link[a] {
                    let x = find_root(&mut parent, &mut kids, &mut roots, l);
                    r = union(&mut parent, &mut kids, &mut roots, &mut rank, x, r);
                }
                link[a] = Some(r);
            }
            if let Some(b) = b.filter(|b| Some(*b) != a) {
                if let Some(l) = link[b] {
                    let x = find_root(&mut parent, &mut kids, &mut roots, l);
                    r = union(&mut parent, &mut kids, &mut roots, &mut rank, x, r);
                }
                link[b] = Some(r);
                if let Some(a) = a {
                    link[a] = Some(r);
                }
            }
        }
        // (pre-order; the island of the car is number 0, the others by their roots)
        let mut place = vec![(0usize, 0usize); np];
        let mut count = 0;
        for &r in roots.iter() {
            let mut stack = vec![r];
            while let Some(nd) = stack.pop() {
                place[nd] = (r + 1, count);
                count += 1;
                for c in kids[nd].iter().rev() {
                    stack.push(*c);
                }
            }
        }
        let car_root = pairs.iter().position(|p| p.1 == Who::Car || p.2 == Who::Car).map(|k| place[k].0);
        self.car_city_order = pairs.iter().position(|p| p.1 == Who::Car && p.2 == Who::City).map(|k| place[k].1);
        for mi in 0..self.manis.len() {
            let (la, lb) = (self.level(self.manis[mi].a), self.level(self.manis[mi].b));
            let key = (la.min(lb), la.max(lb));
            let k = pairs.iter().position(|p| p.0 == key).unwrap();
            let m = &mut self.manis[mi];
            m.island = if Some(place[k].0) == car_root { 0 } else { place[k].0 };
            m.order = place[k].1;
        }
    }

    /// The matrix of one side's part as the tick's update has it (F-21340).
    fn part_matrix(&self, world: &LooseWorld, who: Who, part: u16, car: &CarIn) -> [Vec3; 4] {
        match who {
            Who::Car if part != 0 => car.wheels.get(part as usize - 1).copied().flatten().map_or(car.now, |w| w.now),
            Who::Car => car.now,
            Who::Obj(o) => {
                let obj = &self.objs[o];
                if obj.veh.is_some() {
                    return obj.matrix;
                }
                super::loose_words::side_matrix(Some(&obj.matrix), Some(&m4(&world.types[obj.ty].parts[part as usize].matrix)))
            }
            Who::City => super::loose_words::side_matrix(None, None),
        }
    }

    /// The kept contacts brought up to the two matrices as they stand (F-26862 with
    /// F-33100), their materials (F-43430) and the instances' callbacks: a prop's sets the
    /// elasticity of its type on all its contacts (F-22759); the car's lays a normal flat
    /// that would push it down against a prop that stands (F-55998) - and, fast enough,
    /// feels nothing of a prop that broke two seconds ago and more (F-58598).
    pub fn refresh(&mut self, world: &LooseWorld, car: &CarIn) {
        // (a level shared by the cars: the contacts of what is loose are refreshed once a tick, in the first car's
        // step, with its collide - a second refresh would age a fresh contact, and the slide rule below would drop it
        // before its solve: a prop moving faster than 1.2 m/s would meet no ground)
        let again = self.live && self.refreshed;
        self.refreshed = true;
        // (the instances' contact callbacks stamp their cache entries, F-17408)
        self.pool_stamp_contacts();
        for k in 0..self.manis.len() {
            if again && self.manis[k].a != Who::Car && self.manis[k].b != Who::Car {
                continue;
            }
            let (ma, mb) = (self.part_matrix(world, self.manis[k].a, self.manis[k].part_a, car), self.part_matrix(world, self.manis[k].b, self.manis[k].part_b, car));
            let m = &mut self.manis[k];
            let city = m.b == Who::City;
            if m.rope {
                // (F-33100 on a contact of age 0: its points and depth left; +144 zeroed, age 1)
                for s in m.slots.iter_mut() {
                    s.t.target = Vec3::ZERO;
                    s.age = 1;
                }
                continue;
            }
            for s in m.slots.iter_mut().rev() {
                if s.age != 0 {
                    (s.t.a.point, s.t.b.point, s.t.depth) = refresh_points(&ma, if city { None } else { Some(&mb) }, s.t.a.local, s.t.b.local, s.t.back);
                }
                s.t.target = Vec3::ZERO;
                s.age = s.age.saturating_add(1);
            }
            super::hull::drop_contacts(&mut m.slots, |s| (s.t.depth, s.age, s.t.back, s.t.a.point, s.t.b.point));
        }
        self.manis.retain(|m| !m.slots.is_empty());
        let prop = surface_touch(Surface::Prop);
        let partners = |a: Who, b: Who, objs: &Vec<Obj>| match (a, b) {
            (Who::Obj(x), Who::Obj(y)) => match (objs[x].amb.as_ref(), objs[y].amb.as_ref()) {
                (Some(p), Some(q)) => p.partner == Some(q.uid) || q.partner == Some(p.uid),
                _ => false,
            },
            _ => false,
        };
        for m in self.manis.iter_mut() {
            if again && m.a != Who::Car && m.b != Who::Car {
                continue;
            }
            // (a constraint's contact: no material, F-43430 for kind != 0 zeroes +112 only)
            if m.rope {
                for s in m.slots.iter_mut() {
                    s.t.impulse = Vec3::ZERO;
                    s.enabled = true;
                }
                continue;
            }
            // (a semi's cab and its trailer: their contacts are off every solve - slot 23, F-45219)
            let off = partners(m.a, m.b, &self.objs);
            let (who_a, who_b) = (m.a, m.b);
            let car_is_a = who_a == Who::Car;
            let with_car = car_is_a || who_b == Who::Car;
            // (a wheel's contact with something loose: the wheel's callback F-50733 takes it from the solver;
            // it stays in the manifold)
            let wheel = (car_is_a && m.part_a != 0) || (who_b == Who::Car && m.part_b != 0);
            // (an mcPropInst on either side: its callback writes its type's elasticity)
            let props: Vec<usize> = [who_a, who_b].into_iter().filter_map(|w| if let Who::Obj(o) = w { Some(o) } else { None }).filter(|o| self.objs[*o].inst.is_some()).collect();
            // (stage R: a vehicle on the other side)
            let other_veh = [who_a, who_b].into_iter().find_map(|w| if let Who::Obj(o) = w { self.objs[o].veh.map(|vh| (o, vh.class)) } else { None });
            // (F-58598 resets the whole manifold: F-12537)
            let mut reset = false;
            for s in m.slots.iter_mut() {
                let of = |w: Who| match w {
                    Who::Car => surface_touch(car.hull.surface_of(s.element)),
                    Who::Obj(o) if self.objs[o].amb.is_some() && w == who_a => surface_touch(s.surface_a),
                    Who::Obj(o) if self.objs[o].veh.is_some() => surface_touch(s.surface),
                    Who::Obj(_) => prop,
                    Who::City => surface_touch(s.surface),
                };
                let (ta, tb) = (of(who_a), of(who_b));
                (s.t.friction, s.t.elasticity) = contact_terms(ta, tb);
                s.t.impulse = Vec3::ZERO;
                s.enabled = true;
                if wheel || off {
                    s.enabled = false;
                    continue;
                }
                for o in &props {
                    s.t.elasticity = world.types[self.objs[*o].ty].elasticity;
                }
                if !with_car {
                    continue;
                }
                if let Some((o, class)) = other_veh {
                    car_meets_vehicle(car, &self.objs[o].matrix, class, car_is_a, &mut s.t);
                    continue;
                }
                let Some(&o) = props.first() else { continue };
                let obj = &self.objs[o];
                let t = &world.types[obj.ty];
                // (the car's, F-58598: a hull contact with a prop of a type that only breaks,
                // broken more than two seconds ago, is not felt above 20 m/s - and the manifold is emptied)
                if breaks_off(car.speed, t.min_move, obj.hit, obj.entry.map(|e| e.age)) {
                    s.enabled = false;
                    reset = true;
                    continue;
                }
                // (F-55998, against an mcPropInst: the normal laid flat, the materials part; the loose world's
                // props are inactive or active, level state 1 / 0, never fixed)
                car_meets_prop(car, obj.matrix[1].y, car_is_a, false, &mut s.t);
            }
            // (F-12537: the count 0, the selected -1 - every contact of it gone, this tick's island and the next tick's
            // refresh see none)
            if reset {
                m.slots.clear();
            }
        }
        self.pieces_touched(car);
    }
}

/// F-58598's test (the car's contact callback, on an enabled hull contact with a prop instance, types 4 / 5): the car's
/// speed (chassis +224) over 20, the prop's type breaks only (its least move +316 below 0), it is hit (+80 bit 1) and its
/// entry broken more than 2 s ago (+84 +420). Then the contact is taken from the solver and its manifold is reset. The
/// hit-prop statistics (gHPR / nHPR) the callback counts for the player are not ported.
pub(super) fn breaks_off(speed: f32, min_move: f32, hit: bool, age: Option<f32>) -> bool {
    speed > 20.0 && min_move < 0.0 && hit && age.is_some_and(|a| a > 2.0)
}

impl LooseState {
    /// The ability hook on the car's contacts (F-46500, mcCarInstance slot 23, pre-solve, after `refresh`): for each
    /// enabled contact of the car with another instance, Agro's strike (F-70024, `agro.rs`) on an ambient car's body
    /// while Agro runs, else the gauge's strike for the class of the other instance. The struck body takes the car's
    /// velocity, then the push as an impulse and the spin as an angular impulse, which this step's end consumes.
    /// A car of the session that the level only holds as a vehicle for this step (parked, riding, not in the level)
    /// is told by its answer: the change of its momentum goes into the impulse. (approx.: an mcCar - class 8 -
    /// is never struck: which instances carry the archetype flags 0x0C that let it through is not traced.)
    pub fn agro_contacts(&mut self, car: &CarIn, hook: &mut super::agro::Hook) {
        use super::agro::{strike, Class, PUSH_SCALE};
        for k in 0..self.manis.len() {
            let (a, b) = (self.manis[k].a, self.manis[k].b);
            let car_is_a = a == Who::Car;
            if !(car_is_a || b == Who::Car) || self.manis[k].rope {
                continue;
            }
            let other = if car_is_a { b } else { a };
            for i in 0..self.manis[k].slots.len() {
                let s = self.manis[k].slots[i];
                if !s.enabled {
                    continue;
                }
                // (the normal into the car)
                let n = if car_is_a { s.t.normal } else { Vec3::ZERO - s.t.normal };
                let Who::Obj(o) = other else {
                    hook.arm(Class::Other, n.y);
                    continue;
                };
                let obj = &self.objs[o];
                // (F-83713, the law's word: ambient traffic on a new contact above 6.705 m/s of the car's own, a prop on any
                // new contact, broken or not)
                if let Some(vh) = obj.veh.filter(|v| v.class == 6) {
                    if hook.law_new_contact(1 << 32 | vh.index as u64, car.speed) && car.speed > 6.705 {
                        hook.law_word |= 8;
                    }
                } else if obj.veh.is_none() && obj.inst.is_some() && hook.law_new_contact(o as u64, car.speed) {
                    hook.law_word |= 1;
                }
                // (approx.: a torn-off part's instance class is not read - it is sorted with the city's)
                let class = if obj.veh.is_some() {
                    Class::Vehicle
                } else if obj.inst.is_some() {
                    Class::Prop
                } else {
                    Class::Other
                };
                if !hook.agro_run {
                    hook.arm(class, n.y);
                    continue;
                }
                // (Agro: the other instance is ambient traffic with a body - a parked or fixed one has none)
                let (Some(vh), Some(tune), true) = (obj.veh.filter(|v| v.class == 6), hook.tune, obj.body.is_some()) else { continue };
                // (the user's car damps its turning harder from here on, before the cooldown)
                hook.damp = true;
                if hook.ring.inside(hook.now, vh.index.wrapping_add(1), tune.delay) {
                    continue;
                }
                let live = obj.amb.is_none();
                // (the contact's point on the struck car, for its Agro crash sound; approx.: the original picks +48 / +64
                // of the record by an instance test not traced - the struck side's point is taken)
                let at = if car_is_a { s.t.b.point } else { s.t.a.point };
                let b = self.objs[o].body.as_mut().unwrap();
                let st = strike(&tune, false, b.w.y, &[car.now[0], car.now[1], car.now[2]], car.speed, &mut crate::brain::rng());
                let (mass, was) = (b.mass, b.p);
                b.v = car.vel;
                b.p = car.vel * mass;
                if live {
                    b.jf = b.jf + (b.p - was);
                }
                b.jf = b.jf + st.push * (mass * PUSH_SCALE);
                let spin = b.momentum_of(st.spin);
                b.jt = b.jt + spin;
                hook.struck(st.amp, vh.index, at);
            }
        }
    }

}

// ---------------------------------------------------------------------------------------------
// The breaking pass (F-12210, F-54981, F-80619; p1/break_decode.md).

/// F-43430: a contact's friction and elasticity from the materials of its two sides (friction, elasticity): the products,
/// the friction not below 0 (a NaN is 0), the elasticity cut to 1.
pub fn contact_terms(ta: (f32, f32), tb: (f32, f32)) -> (f32, f32) {
    let f = ta.0 * tb.0;
    let e = ta.1 * tb.1;
    (if f >= 0.0 { f } else { 0.0 }, if e - 1.0 >= 0.0 { 1.0 } else { e })
}

/// F-14324: strength `m` laid on group `g` goes into `gi` for the group and, as the walk leaves it, for the groups around
/// it: down through the sub-groups that are present (`dir` below zero: each level takes the group's down share) and up
/// through the parents (`dir` above zero: each takes its up share), a group no longer in the entry (not `present`)
/// ending the walk and getting nothing. `scales` are the tuning object's two numbers (+12 on the way down, +16 on the way
/// up) where it exists - the port runs without it.
fn spread(t: &FragType, present: &dyn Fn(usize) -> bool, gi: &mut [f32], mut m: f32, mut g: usize, mut dir: i32, scales: Option<(f32, f32)>) {
    loop {
        let grp = &t.groups[g];
        if dir < 0 {
            m = grp.down * m;
            if let Some((down, _)) = scales {
                m = down * m;
            }
        }
        if present(g) {
            gi[g] = gi[g] + m;
        }
        if dir <= 0 {
            for k in 0..grp.groups as usize {
                let c = grp.first_group as usize + k;
                if present(c) {
                    spread(t, present, gi, m, c, dir - 1, scales);
                }
            }
            if dir < 0 {
                return;
            }
        }
        let p = grp.parent as usize;
        if grp.parent == 255 || !present(p) {
            return;
        }
        m = grp.up * m;
        if let Some((_, up)) = scales {
            m = up * m;
        }
        dir += 1;
        g = p;
    }
}

/// F-66303: what a body's archetype asks of its damping, in the world axes: for the moving and for the turning, the
/// velocity in the body's axes `l` is slowed by `c` times its unit vector, `|l| l` times `sq` and `l` times `lin`
/// (`d`: those three for each, the force's first), the acceleration cut at the point where it would carry the velocity
/// past rest in a tick (then it is exactly the one that stops it); times the mass or the moments, back to the world.
pub(super) fn damping_forces(rows: &[Vec3; 3], v0: Vec3, w0: Vec3, mass: f32, inertia: Vec3, d: &[[f32; 3]; 6], dt: f32) -> (Vec3, Vec3) {
    let rs = super::gjk::rsqrt_as_original;
    let inv_dt = recip(dt);
    let cols = [Vec3::new(rows[0].x, rows[1].x, rows[2].x), Vec3::new(rows[0].y, rows[1].y, rows[2].y), Vec3::new(rows[0].z, rows[1].z, rows[2].z)];
    let slow = |x: Vec3, c: Vec3, lin: Vec3, sq: Vec3| {
        let l = Vec3::new(dot3(rows[0], x), dot3(rows[1], x), dot3(rows[2], x));
        let l2 = dot3(l, l);
        let sp = if l2 == 0.0 { 0.0 } else { l2 * rs(l2) };
        let u = if l2 > f32::from_bits(0x3727_c5ac) { l * rs(l2) } else { Vec3::ZERO };
        let mut a = Vec3::ZERO - ((u * c) + (((l * sp) * sq) + (l * lin)));
        let n = a * dt + l;
        if 0.0 >= dot3(n, l) {
            a = -(l * inv_dt - Vec3::ZERO);
        }
        a
    };
    let fl = slow(v0, v(d[0]), v(d[1]), v(d[2])) * mass;
    let tl = slow(w0, v(d[3]), v(d[4]), v(d[5])) * inertia;
    (Vec3::new(dot3(cols[0], fl), dot3(cols[1], fl), dot3(cols[2], fl)), Vec3::new(dot3(cols[0], tl), dot3(cols[1], tl), dot3(cols[2], tl)))
}

/// F-34309: how much of the impulses on its parts (`comp`, by part) breaks or moves an
/// instance: the share `t` and the group that leaves (255: the whole of it).
fn break_strength(t: &FragType, obj: &Obj, comp: &[Vec3]) -> Option<(f32, u8)> {
    let entry = obj.entry;
    let present = |g: usize| entry.map_or(true, |e| e.absent & (1 << g) == 0);
    if entry.is_none() && t.flags & 1 == 0 {
        // (a prop that goes as a whole)
        let f = t.min_move;
        if f < 0.0 {
            return None;
        }
        let mut tot = 0.0f32;
        for c in comp.iter().take(t.parts.len()) {
            tot = fma(c.y, c.y, fma(c.x, c.x, c.z * c.z)).sqrt() + tot;
        }
        if tot < f {
            return None;
        }
        return Some((f / tot, 255));
    }
    let mut gi = vec![0.0f32; t.groups.len()];
    let mut total = 0.0f32;
    for (p, c) in comp.iter().enumerate().take(t.parts.len()) {
        if c.x == 0.0 && c.y == 0.0 && c.z == 0.0 {
            continue;
        }
        let m = fma(c.y, c.y, fma(c.z, c.z, c.x * c.x)).sqrt();
        let g = group_of(t, p);
        if entry.is_some() && !present(g) {
            continue;
        }
        total = m + total;
        spread(t, &present, &mut gi, m, g, 0, None);
    }
    if !obj.hit && t.min_move >= 0.0 {
        let x = t.groups[0].up * gi[0];
        if x >= t.min_move {
            return Some((t.min_move / x, 255));
        }
    }
    let (mut best, mut win) = (0.0f32, 255u8);
    for g in 0..t.groups.len() {
        let x = gi[g];
        if x <= 0.0 {
            continue;
        }
        let grp = &t.groups[g];
        let s = grp.strength;
        if s < 0.0 && grp.latch == 0.0 {
            continue;
        }
        let ok = if grp.parent != 255 { entry.is_none() || present(grp.parent as usize) } else { t.root_groups > 1 && (0..t.root_groups as usize).filter(|r| present(*r)).count() >= 2 };
        if !ok {
            continue;
        }
        let ratio = if s > f32::from_bits(0x3586_37bd) { x / s } else if s < 0.0 { 0.0 } else { f32::MAX };
        if ratio > 1.0 && ratio > best {
            win = g as u8;
            best = ratio;
        }
    }
    if win == 255 {
        return None;
    }
    Some((t.groups[win as usize].strength / total, win))
}

/// The original's sort of an island's contact list (STLport std::sort, F-52683: the introsort loop
/// F-98871 with the partition F-69639 while a range holds more than 16, then insertion sorts
/// F-35665 / F-66880): ascending by `key`, stable up to 16 elements, above that as its steps fall. The loop's depth
/// limit is twice floor(log2 n); a range it reaches is heap sorted (the partial sort F-76221 over the whole range:
/// make_heap F-28981, sort_heap F-78034, both through adjust_heap F-49539 and push_heap F-57527).
pub fn stl_sort<T: Clone>(a: &mut Vec<T>, key: impl Fn(&T) -> i64) {
    stl_sort_by(a, |x, y| key(x) < key(y));
}

/// [`stl_sort`] with the comparison itself (`less`, the original's `<` on the key: false both ways for a NaN).
pub fn stl_sort_by<T: Clone>(a: &mut [T], less: impl Fn(&T, &T) -> bool) {
    fn insertion<T: Clone>(a: &mut [T], lo: usize, hi: usize, less: &impl Fn(&T, &T) -> bool) {
        for i in lo + 1..hi {
            let v = a[i].clone();
            if less(&v, &a[lo]) {
                for j in (lo..i).rev() {
                    a[j + 1] = a[j].clone();
                }
                a[lo] = v;
            } else {
                let mut j = i;
                while less(&v, &a[j - 1]) {
                    a[j] = a[j - 1].clone();
                    j -= 1;
                }
                a[j] = v;
            }
        }
    }
    fn adjust<T: Clone>(b: &mut [T], mut hole: usize, bottom: usize, v: T, less: &impl Fn(&T, &T) -> bool) {
        let top = hole;
        let mut i = 2 * hole + 2;
        while i < bottom {
            if less(&b[i], &b[i - 1]) {
                i -= 1;
            }
            b[hole] = b[i].clone();
            hole = i;
            i = 2 * i + 2;
        }
        if i == bottom {
            b[hole] = b[bottom - 1].clone();
            hole = bottom - 1;
        }
        while top < hole {
            let p = (hole - 1) / 2;
            if !(less(&b[p], &v)) {
                break;
            }
            b[hole] = b[p].clone();
            hole = p;
        }
        b[hole] = v;
    }
    fn heap<T: Clone>(b: &mut [T], less: &impl Fn(&T, &T) -> bool) {
        let n = b.len();
        if n >= 2 {
            for h in (0..=(n - 2) / 2).rev() {
                let v = b[h].clone();
                adjust(b, h, n, v, less);
            }
        }
        for len in (2..=n).rev() {
            let v = b[len - 1].clone();
            b[len - 1] = b[0].clone();
            adjust(b, 0, len - 1, v, less);
        }
    }
    fn quick<T: Clone>(a: &mut [T], first: usize, mut last: usize, mut depth: u32, less: &impl Fn(&T, &T) -> bool) {
        while last - first > 16 {
            if depth == 0 {
                heap(&mut a[first..last], less);
                return;
            }
            depth -= 1;
            let n = last - first;
            let (ka, kb, kc) = (a[first].clone(), a[first + n / 2].clone(), a[last - 1].clone());
            let m = if less(&ka, &kb) {
                if less(&kb, &kc) { first + n / 2 } else if less(&ka, &kc) { last - 1 } else { first }
            } else if less(&ka, &kc) {
                first
            } else if less(&kb, &kc) {
                last - 1
            } else {
                first + n / 2
            };
            let kp = a[m].clone();
            let (mut f, mut l) = (first, last);
            loop {
                while less(&a[f], &kp) {
                    f += 1;
                }
                l -= 1;
                while less(&kp, &a[l]) {
                    l -= 1;
                }
                if !(f < l) {
                    break;
                }
                a.swap(f, l);
                f += 1;
            }
            quick(a, f, last, depth, less);
            last = f;
        }
    }
    let n = a.len();
    if n > 16 {
        quick(a, 0, n, 2 * n.ilog2(), &less);
        insertion(a, 0, 16, &less);
        // (the rest without the guard: the least stands in the first sixteen)
        for i in 16..n {
            let v = a[i].clone();
            let mut j = i;
            while j > 0 && less(&v, &a[j - 1]) {
                a[j] = a[j - 1].clone();
                j -= 1;
            }
            a[j] = v;
        }
    } else {
        insertion(a, 0, n, &less);
    }
}

/// The bone whose matrix a break event of group `g` follows: that of the part that stays, the first child of the
/// parent group, if that has a bone (`prop_hit_fx_rule.md` 2c; none: the instance's matrix).
fn fx_part(t: &FragType, g: usize) -> Option<u16> {
    let pg = t.groups.get(g)?.parent as usize;
    if pg == 255 {
        return None;
    }
    let c = t.groups.get(pg)?.first_child as usize;
    t.children.get(c).filter(|ch| ch.bone != -1).map(|ch| ch.bone as u16)
}

/// One contact of an island as the breaking pass sees it.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub a: Option<Who>,
    pub b: Option<Who>,
    pub part_a: u16,
    pub part_b: u16,
    /// where it is kept: a manifold of this state and its slot, or (None) the car's own
    pub from: Option<(usize, usize)>,
    /// a contact of the car's own (`from` None): its number among the car's contacts with the city as
    /// they were handed in - the sort can move it among the car's other rows, and what the solve leaves
    /// is the contact's own
    pub car_row: usize,
    /// a row of the thrown rider's ([`RiderMerge`]): his side is the island's jointed body, not a side here
    pub rider: bool,
}

/// Where a row of a car's island comes from.
#[derive(Clone, Copy)]
enum RowSrc {
    /// the car's contact with the city of this number
    City(usize),
    /// a slot of this manifold
    Mani(usize),
    /// the thrown rider's row of this number ([`RiderMerge`])
    Rider(usize),
}

/// The thrown rider in one island with the loose world's (one island build and solve per tick, F-96487): his ragdoll, his
/// share of the island as his build made it ([`super::artic_run::RiderIsland`]: his rows and the loose world's in the
/// island's order), where each loose row is kept (per node of [`super::artic_run::LooseRows`], its manifold and slot) and
/// the vehicle his rows meet (their vehicle side).
pub struct RiderMerge<'a> {
    pub body: &'a mut super::artic::Articulated,
    pub isl: super::artic_run::RiderIsland,
    pub slots: Vec<Vec<(usize, usize)>>,
    pub other: usize,
    pub rider_key: i64,
}

/// The rider's level index and instance address in the loose world's numbering: above every object's (ov.0 and xk.0 had
/// the vehicle he met below him - its instance and level both; the order in general follows the level's free index list,
/// not kept by the port: vehsim.rs `OTHER_LEVEL`).
pub const RIDER_LOOSE_LEVEL: i64 = i64::MAX / 2;
pub const RIDER_LOOSE_KEY: i64 = i64::MAX / 2;

/// The rider's share handed to [`LooseState::step_rest_rider`]: what his build takes from outside the loose world (the
/// bike's rows' places, the partner vehicle's rows) and the partner vehicle's object.
#[derive(Clone, Debug)]
pub struct RiderJob {
    pub outside: super::artic_run::Outside,
    pub other: usize,
}

/// A car of an island of several cars whose solve another car's step runs (the game's level step: one solve an island,
/// F-96487): its object in the level, its solver body as its own step made it (the move, this tick's forces), its
/// contacts with the city as its callbacks left them; and what the island gave it.
pub struct ExtraCar {
    pub uid: u32,
    pub obj: usize,
    pub body: Body,
    /// its momenta (+368 / +384 before the solve) and the push solve's turn it starts from
    pub p: Vec3,
    pub l: Vec3,
    pub turn_left: Vec3,
    pub city: Vec<solver::Contact>,
    pub city_part: Vec<u8>,
    /// its part's place of its pair with the city in its own island's tree (kept for its part)
    pub city_order: Option<usize>,
    /// the answer: the impulse and its turn (+368 / +384), its runs of the callbacks and its disabled rows' normals (A, B),
    /// the island as its own step sees it, the push
    pub impulse: Vec3,
    pub turning: Vec3,
    pub runs: Vec<(bool, bool, bool)>,
    pub off: (Vec3, Vec3),
    pub view: Option<IslandOut>,
    pub pushed: Pushed,
    /// its mass (the breaking pass's shares and the solve of an island it is the car of: the rider's partner)
    pub mass: f32,
}

impl Clone for ExtraCar {
    fn clone(&self) -> ExtraCar {
        ExtraCar { view: None, runs: self.runs.clone(), city: self.city.clone(), city_part: self.city_part.clone(), ..*self }
    }
}

impl std::fmt::Debug for ExtraCar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtraCar").field("uid", &self.uid).field("obj", &self.obj).finish_non_exhaustive()
    }
}

impl ExtraCar {
    pub fn new(body: Body, p: Vec3, l: Vec3, turn_left: Vec3, city: Vec<solver::Contact>, city_part: Vec<u8>) -> ExtraCar {
        ExtraCar { uid: 0, obj: usize::MAX, body, p, l, turn_left, city, city_part, city_order: None, impulse: Vec3::ZERO, turning: Vec3::ZERO, runs: Vec::new(), off: (Vec3::ZERO, Vec3::ZERO), view: None, pushed: Pushed::default(), mass: 1.0 / body.inv_mass }
    }
}

/// What a car's island came to.
pub struct IslandOut {
    pub touches: Vec<Touch>,
    pub links: Vec<Link>,
    pub bodies: Vec<Body>,
    /// which object each body after the car's (0) is
    pub owners: Vec<usize>,
    pub solved: solver::IslandSolved,
    /// what the car's +368 / +384 gain
    pub impulse: Vec3,
    pub turning: Vec3,
    /// the rows (numbers into `links`) in the island's second list, sorted by instance B's address
    /// (F-68442's list for the B side, island +296)
    pub by_b: Vec<usize>,
    /// the car's runs of the callbacks (its A run, then its B run, those it has): the first row's other object
    /// active, a row on a part of the car that is not a wheel, a row with an impulse
    pub car_runs: Vec<(bool, bool, bool)>,
    /// the car's normals summed over the rows the callbacks disabled, of the car's A run and of its B run
    /// (mcCarDamage's scrape normal takes every row of its run, A-49818)
    pub off_a: Vec3,
    pub off_b: Vec3,
    /// the rider's rows (`RiderMerge`): the row's number and his row's number
    pub rider_rows: Vec<(usize, usize)>,
}

impl LooseState {
    /// Is this object another car of the island, whose solve this step runs ([`ExtraCar`])?
    pub fn is_extra(&self, o: usize) -> bool {
        self.extras.iter().any(|e| e.obj == o)
    }

    /// Another car's share into this step's island (the game's level step): its part's manifolds as this level names
    /// them (its own side its object, the stepping car's object the car), its contacts with the city a manifold of its
    /// object and the city (one per run of the same part), its body the solver body its own step made. `lead`: the
    /// stepping car's object.
    pub fn lend_extra(&mut self, mut e: ExtraCar, part: CarPart, lead: Option<usize>) {
        let tag = e.uid + 1;
        let me = Who::Obj(e.obj);
        e.city_order = part.car_city_order;
        let swap = |w: Who| match w {
            Who::Car => me,
            Who::Obj(o) if Some(o) == lead => Who::Car,
            w => w,
        };
        for mut m in part.manis {
            m.a = swap(m.a);
            m.b = swap(m.b);
            m.owner = tag;
            self.manis.push(m);
        }
        let rows = e.body.rows;
        let mut k = 0;
        while k < e.city.len() {
            let part_a = e.city_part.get(k).copied().unwrap_or(0) as u16;
            let mut slots = Vec::new();
            while k < e.city.len() && e.city_part.get(k).copied().unwrap_or(0) as u16 == part_a {
                let t = car_city_touch(&e.city[k], rows);
                slots.push(Slot2 { t, age: 1, enabled: true, element: 0, surface: Surface::Road, surface_a: Surface::Road });
                k += 1;
            }
            self.manis.push(Mani { a: me, b: Who::City, part_a, part_b: 0, cell: usize::MAX, stale: false, visited: true, island: 0, order: 0, slots, rope: false, owner: tag });
        }
        if let Some(b) = self.objs[e.obj].body.as_mut() {
            let x = e.body;
            b.at = x.at;
            b.rows = x.rows;
            b.v = x.vel;
            b.w = x.ang;
            b.inv_mass = x.inv_mass;
            b.inv_inertia = x.inv_moments;
            b.force = x.force;
            b.torque = x.torque;
            b.p = e.p;
            b.l = e.l;
            b.push = Vec3::ZERO;
            b.turn = e.turn_left;
            b.jf = Vec3::ZERO;
            b.jt = Vec3::ZERO;
        }
        let o = &mut self.objs[e.obj];
        o.matrix = [rows[0], rows[1], rows[2], o.matrix[3]];
        e.view = None;
        self.extras.push(e);
    }

    /// The islands again after other cars' parts were lent ([`Self::lend_extra`]); the stepping car's island then holds
    /// theirs.
    pub fn islands_again(&mut self, car_city: bool) {
        self.build_islands(car_city);
    }

    /// The island as each other car's step sees it, after the push solve: the stepping car's rows its object's, the other
    /// car's own rows the car's, its contacts with the city its own rows (`from` None, their numbers).
    pub fn extra_views(&mut self, out: &IslandOut) {
        let lead = self.lead.unwrap_or(usize::MAX);
        for e in 0..self.extras.len() {
            let (me, tag) = (self.extras[e].obj, self.extras[e].uid + 1);
            let mut city_row: HashMap<(usize, usize), usize> = HashMap::new();
            let mut n = 0;
            for (k, m) in self.manis.iter().enumerate().filter(|(_, m)| m.owner == tag && m.b == Who::City && m.cell == usize::MAX) {
                for i in 0..m.slots.len() {
                    city_row.insert((k, i), n);
                    n += 1;
                }
            }
            let swap = |w: Option<Who>| match w {
                Some(Who::Car) => Some(Who::Obj(lead)),
                Some(Who::Obj(o)) if o == me => Some(Who::Car),
                w => w,
            };
            let links: Vec<Link> = out
                .links
                .iter()
                .map(|l| {
                    let mut l = *l;
                    l.a = swap(l.a);
                    l.b = swap(l.b);
                    if let Some(r) = l.from.and_then(|f| city_row.get(&f)) {
                        l.from = None;
                        l.car_row = *r;
                        l.b = None;
                    } else if l.from.is_none() && !l.rider {
                        // (the stepping car's own contacts with the city: none of this car's)
                        l.from = Some((usize::MAX, l.car_row));
                    }
                    l
                })
                .collect();
            // (a car whose rows are not in this island - joined to it through the thrown rider only, its own island solved
            // apart: vehsim.rs - gets none here)
            if !links.iter().any(|l| l.a == Some(Who::Car) || l.b == Some(Who::Car) || (l.from.is_none() && !l.rider && l.b.is_none())) {
                continue;
            }
            let x = &self.extras[e];
            let view = IslandOut { touches: out.touches.clone(), links, bodies: out.bodies.clone(), owners: out.owners.clone(), solved: solver::IslandSolved::empty(), impulse: Vec3::ZERO, turning: Vec3::ZERO, by_b: out.by_b.clone(), car_runs: x.runs.clone(), off_a: x.off.0, off_b: x.off.1, rider_rows: Vec::new() };
            self.extras[e].view = Some(view);
        }
    }

    /// The other cars' shares back out of the level after this step's solve and push: each car's answer (impulse,
    /// turn, momenta after the breaking pass's shares, push), its contacts with the city with their results, and its part
    /// as its own level names it.
    pub fn return_extras(&mut self, lead: Option<usize>) -> Vec<(ExtraCar, CarPart)> {
        let mut out = Vec::new();
        for mut e in std::mem::take(&mut self.extras) {
            let tag = e.uid + 1;
            let me = Who::Obj(e.obj);
            let swap = |w: Who| match w {
                w if w == me => Who::Car,
                Who::Car => lead.map_or(Who::Car, Who::Obj),
                w => w,
            };
            let (mine, rest): (Vec<Mani>, Vec<Mani>) = std::mem::take(&mut self.manis).into_iter().partition(|m| m.owner == tag);
            self.manis = rest;
            let mut part = CarPart { manis: Vec::new(), car_city_order: e.city_order };
            let mut k = 0;
            for mut m in mine {
                if m.b == Who::City && m.cell == usize::MAX {
                    for s in &m.slots {
                        if let Some(c) = e.city.get_mut(k) {
                            c.kept = s.t.kept;
                            c.impulse = s.t.impulse;
                            c.pushed = s.t.pushed;
                        }
                        k += 1;
                    }
                    continue;
                }
                m.a = swap(m.a);
                m.b = swap(m.b);
                m.owner = 0;
                part.manis.push(m);
            }
            if let Some(b) = self.objs[e.obj].body {
                e.impulse = b.jf;
                e.turning = b.jt;
                e.p = b.p;
                e.l = b.l;
                e.pushed = Pushed { push: b.push, turn: b.turn, deep: b.deep };
            }
            out.push((e, part));
        }
        out
    }

    /// Is an instance one the breaking pass lists (instance vtable[8], F-83042)?
    fn breakable(&self, world: &LooseWorld, o: usize) -> bool {
        let obj = &self.objs[o];
        if obj.veh.is_some() {
            return false;
        }
        if !obj.hit {
            return true;
        }
        let Some(e) = obj.entry else { return false };
        (0..world.types[obj.ty].groups.len()).filter(|g| e.absent & (1 << g) == 0).count() >= 2
    }

    /// The bodies of an island's contacts, the car's first, and each side's body number.
    fn bodies_of(&self, car: &Body, links: &[Link], touches: &mut [Touch]) -> (Vec<Body>, Vec<usize>) {
        let mut bodies = vec![*car];
        let mut owners: Vec<usize> = Vec::new();
        for (l, t) in links.iter().zip(touches.iter_mut()) {
            for (who, side) in [(l.a, &mut t.a), (l.b, &mut t.b)] {
                side.body = match who {
                    Some(Who::Car) => Some(0),
                    Some(Who::Obj(o)) => self.objs[o].body.map(|b| match owners.iter().position(|x| *x == o) {
                        Some(k) => k + 1,
                        None => {
                            owners.push(o);
                            bodies.push(b.as_solver());
                            owners.len()
                        }
                    }),
                    Some(Who::City) | None => None,
                };
            }
        }
        (bodies, owners)
    }

    /// The velocity solve of the car's island with what is loose in it, and the breaking pass
    /// (F-68693 whose apply stands back, then F-12210): the solve with every prop that has
    /// no body taken for immovable; the instances asked what of those impulses breaks them; the
    /// winner's share given at once to the bodies; the break; and the solve again from where the
    /// bodies then are, until nothing breaks. `city`: the car's own contacts with the world
    /// (it is A, the world has no body), which stand first or last by the addresses.
    pub fn solve_car_island(&mut self, world: &LooseWorld, car_in: &CarIn, car: &mut Body, dynm: &mut CarDyn, city: &[solver::Contact], city_rows: &[bool], city_part: &[u8], ids: &[usize], dt: f32) -> IslandOut {
        self.solve_island(world, car_in, car, dynm, city, city_rows, city_part, ids, dt, None)
    }

    /// [`Self::solve_car_island`] with the thrown rider in the island (`merge`): his rows and the loose world's in the order
    /// his build gave ([`super::artic_run::RiderRagdoll::build`] over [`Self::rider_nodes`]), his ragdoll the island's
    /// jointed body (F-36821 type 7 / 13 rows, 8 sweeps, one elasticity factor), his impulses applied (F-24076) and his rows'
    /// results back in `merge.isl`.
    #[allow(clippy::too_many_arguments)]
    pub fn solve_island(&mut self, world: &LooseWorld, car_in: &CarIn, car: &mut Body, dynm: &mut CarDyn, city: &[solver::Contact], city_rows: &[bool], city_part: &[u8], ids: &[usize], dt: f32, mut merge: Option<&mut RiderMerge>) -> IslandOut {
        let _ = car_in;
        let (car_level, car_addr) = self.car_place();
        // (the island's contacts, F-68442: every contact of every manifold in the tree's pre-order - a
        // pair's sub-manifolds by (B's part, A's part), a manifold's contacts by their number - the ones the
        // callbacks disabled too; that list sorted by the address of each one's instance A with the
        // original's own sort, which is not stable above 16; the enabled ones of it are the rows.
        // `city_rows`: the car's contacts with the city in their order, false = one that is not enabled.
        // MCLA_DBG_RA5: pairs by their indices and a stable sort, as before.)
        let old_order = super::vehsim::dbg_flag("MCLA_DBG_RA5");
        let car_order = self.car_city_order.unwrap_or(0);
        // (each row: its place in the tree walk, instance A's address, instance B's, the row)
        let city_addr = self.city_addr();
        let (mut off_a, mut off_b) = (Vec3::ZERO, Vec3::ZERO);
        let mut list: Vec<((i64, i64, u64, u64), i64, i64, Option<(Link, Touch)>)> = Vec::new();
        let mut next_city = 0;
        // (where each row of the walk comes from: the car's contact with the city of that number, or a manifold)
        let mut src: Vec<RowSrc> = Vec::new();
        // (the other cars of the island: each one's disabled rows' normals as A and as B)
        let mut extra_off: Vec<(usize, Vec3, Vec3)> = self.extras.iter().map(|e| (e.obj, Vec3::ZERO, Vec3::ZERO)).collect();
        // (and, for a manifold's, which slot: the merged order names them so)
        let mut slot_of: Vec<Option<(usize, usize)>> = Vec::new();
        let city_link = |(row, c): (usize, &solver::Contact)| {
            let a = Side { body: Some(0), point: c.on_body, local: c.local, rows: car.rows, margin: 0.04, core: Vec3::ZERO };
            let b = Side { body: None, point: c.on_world, local: c.on_world, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
            (Link { a: Some(Who::Car), b: None, part_a: 0, part_b: 0, from: None, car_row: row, rider: false }, Touch { depth: c.depth, friction: c.friction, elasticity: c.elasticity, a, b, normal: c.normal, back: Vec3::ZERO - c.normal, pushed: c.pushed, kept: c.kept, target: c.target, impulse: Vec3::ZERO, kind: 0, jointed: None })
        };
        if old_order || city_rows.is_empty() {
            for (n, c) in city.iter().enumerate() {
                list.push(((if old_order { i64::MIN } else { car_order as i64 }, car_level, 0, n as u64), car_addr, city_addr, Some(city_link((n, c)))));
                src.push(RowSrc::City(n));
                slot_of.push(None);
            }
        } else {
            for (n, on) in city_rows.iter().enumerate() {
                let row = if *on {
                    next_city += 1;
                    city.get(next_city - 1).map(|c| city_link((next_city - 1, c)))
                } else {
                    None
                };
                list.push(((car_order as i64, car_level, 0, n as u64), car_addr, city_addr, row));
                src.push(RowSrc::City(n));
                slot_of.push(None);
            }
        }
        for &k in ids {
            let m = &self.manis[k];
            let (la, lb) = (self.level(m.a), self.level(m.b));
            let pair = if old_order { (la.min(lb), la.max(lb), if m.b == Who::City { m.cell as u64 } else { m.part_b as u64 }, m.part_a as u64) } else { (m.order as i64, 0, if m.b == Who::City { m.cell as u64 } else { m.part_b as u64 }, m.part_a as u64) };
            let key = match m.a {
                Who::Obj(o) => self.objs[o].addr,
                _ => car_addr,
            };
            let key_b = match m.b {
                Who::Obj(o) => self.objs[o].addr,
                Who::Car => car_addr,
                Who::City => city_addr,
            };
            let rows_of = |w: Who| match w {
                Who::Car => car.rows,
                Who::Obj(o) => [self.objs[o].matrix[0], self.objs[o].matrix[1], self.objs[o].matrix[2]],
                Who::City => [Vec3::X, Vec3::Y, Vec3::Z],
            };
            for (i, s) in m.slots.iter().enumerate() {
                if !s.enabled {
                    if m.a == Who::Car {
                        off_a += s.t.normal;
                    } else if m.b == Who::Car {
                        off_b += Vec3::ZERO - s.t.normal;
                    }
                    for e in extra_off.iter_mut() {
                        if m.a == Who::Obj(e.0) {
                            e.1 += s.t.normal;
                        } else if m.b == Who::Obj(e.0) {
                            e.2 += Vec3::ZERO - s.t.normal;
                        }
                    }
                    if !old_order {
                        list.push((pair, key, key_b, None));
                        src.push(RowSrc::Mani(k));
                        slot_of.push(Some((k, i)));
                    }
                    continue;
                }
                let mut t = s.t;
                t.a.rows = rows_of(m.a);
                t.b.rows = rows_of(m.b);
                // (the whole bound of either side is a composite or the city's grid: its margin +112)
                t.a.margin = 0.04;
                t.b.margin = 0.04;
                // (stage R: a vehicle's whole bound is its polyhedron - the thickness from a corner of it)
                // (an ambient trailer's whole bound is its composite: margin +112, no core)
                if let Who::Obj(o) = m.a {
                    if let Some(vh) = self.objs[o].veh.filter(|v| v.shape2.is_none() && !self.is_extra(o)) {
                        t.a.margin = f32::from_bits(0x3d23_d70a);
                        t.a.core = veh_core(vh.shape, &t.a.rows, t.normal);
                    }
                }
                if let Who::Obj(o) = m.b {
                    if let Some(vh) = self.objs[o].veh.filter(|v| v.shape2.is_none() && !self.is_extra(o)) {
                        t.b.margin = f32::from_bits(0x3d23_d70a);
                        t.b.core = veh_core(vh.shape, &t.b.rows, t.back);
                    }
                }
                // (a torn bumper's whole bound is its box: the thickness from the corner of the box less its margin
                // along the normal, A-32016's support)
                if let Who::Obj(o) = m.a {
                    if let Some(c) = piece_core(world, &self.objs[o], &t.a.rows, t.normal) {
                        t.a.core = c;
                    }
                }
                if let Who::Obj(o) = m.b {
                    if let Some(c) = piece_core(world, &self.objs[o], &t.b.rows, t.back) {
                        t.b.core = c;
                    }
                }
                list.push((pair, key, key_b, Some((Link { a: Some(m.a), b: Some(m.b), part_a: m.part_a, part_b: m.part_b, car_row: 0, from: Some((k, i)), rider: false }, t))));
                src.push(RowSrc::Mani(k));
                slot_of.push(Some((k, i)));
            }
        }
        // (the rider's rows: his side the jointed body, his vehicle's side its object - a row's other side the city or
        // himself has none)
        if let Some(mg) = merge.as_ref() {
            for (j, t) in mg.isl.touches.iter().enumerate() {
                // (his vehicle side: the bike - the island's car, body 0 - or the other vehicle's object)
                let who = match mg.isl.partner(j) {
                    Some(true) => Who::Car,
                    // (`other` usize::MAX: his partner vehicle is the island's car - another car's island, solved in his
                    // bike's step: vehsim.rs)
                    _ if mg.other == usize::MAX => Who::Car,
                    _ => Who::Obj(mg.other),
                };
                let veh = |b: Option<usize>| if b.is_some() { Some(who) } else { None };
                let link = Link { a: veh(t.a.body), b: veh(t.b.body), part_a: 0, part_b: 0, from: None, car_row: 0, rider: true };
                list.push(((i64::MAX, 0, 0, j as u64), mg.rider_key, mg.rider_key, Some((link, *t))));
                src.push(RowSrc::Rider(j));
                slot_of.push(None);
            }
        }
        // (the rows' sources go with them)
        let mut both: Vec<_> = list.into_iter().zip(src.into_iter().zip(slot_of)).collect();
        if merge.is_none() {
            both.sort_by_key(|x| x.0 .0);
        }
        let (list, src_slot): (Vec<_>, Vec<(RowSrc, Option<(usize, usize)>)>) = both.into_iter().unzip();
        let (src, slot_of): (Vec<RowSrc>, Vec<Option<(usize, usize)>>) = src_slot.into_iter().unzip();
        if dbg() {
            eprintln!("LOOSEROWS tick {} n {} city_rows {:?} walk {:?}", self.tick, list.len(), city_rows, list.iter().map(|x| (x.0 .0, x.0 .2, x.0 .3, x.1, x.3.is_some())).collect::<Vec<_>>());
            eprintln!("LOOSEWARM tick {} {:?}", self.tick, list.iter().filter_map(|x| x.3.as_ref().map(|(l, t)| (l.a, l.b, t.depth, t.kept))).collect::<Vec<_>>());
        }
        // (the same walk sorted by instance B's address, the same sort: the list the callbacks' second
        // pass walks, F-15350 - by the rows' places in the walk)
        let mut walk_b: Vec<(i64, usize)> = list.iter().enumerate().map(|(n, x)| (x.2, n)).collect();
        let enabled: Vec<bool> = list.iter().map(|x| x.3.is_some()).collect();
        let mut tagged: Vec<(i64, usize, Option<(Link, Touch)>)> = list.into_iter().enumerate().map(|(n, x)| (x.1, n, x.3)).collect();
        if let Some(mg) = merge.as_ref() {
            // (the rider's build sorted the island's rows, his and the loose world's, by instance A: that order; the
            // B-sorted list is his build's too - here only the A list is walked)
            let at: HashMap<(usize, usize), usize> = slot_of.iter().enumerate().filter_map(|(n, s)| s.map(|s| (s, n))).collect();
            let rider_at: HashMap<usize, usize> = src.iter().enumerate().filter_map(|(n, s)| if let RowSrc::Rider(j) = s { Some((*j, n)) } else { None }).collect();
            let city_at: HashMap<usize, usize> = src.iter().enumerate().filter_map(|(n, s)| if let RowSrc::City(r) = s { Some((*r, n)) } else { None }).collect();
            let place = |row: &super::artic_run::IslRow| match *row {
                super::artic_run::IslRow::Loose(p, r) => mg.slots.get(p).and_then(|v| v.get(r)).and_then(|s| at.get(s)).copied(),
                super::artic_run::IslRow::Rider(j) => rider_at.get(&j).copied(),
                // (the bike's contact with the city of that number: the island's car is the bike)
                super::artic_run::IslRow::Bike(r) => city_at.get(&r).copied(),
            };
            let mut taken: Vec<Option<(i64, usize, Option<(Link, Touch)>)>> = tagged.into_iter().map(Some).collect();
            let mut ordered = Vec::with_capacity(taken.len());
            // (the build's whole A list where it made one - the disabled rows in their places - else its enabled rows)
            let list_a = if mg.isl.list_a.is_empty() { &mg.isl.rows } else { &mg.isl.list_a };
            for row in list_a {
                if let Some(x) = place(row).and_then(|n| taken[n].take()) {
                    ordered.push(x);
                }
            }
            // (a row the build did not name - none is expected - goes last, in the walk's order)
            ordered.extend(taken.into_iter().flatten());
            tagged = ordered;
            // (the B list: the build's)
            if !mg.isl.list_b.is_empty() {
                let mut b: Vec<(i64, usize)> = mg.isl.list_b.iter().filter_map(|row| place(row).map(|n| (0, n))).collect();
                let named: std::collections::HashSet<usize> = b.iter().map(|x| x.1).collect();
                b.extend(walk_b.iter().filter(|x| !named.contains(&x.1)).map(|x| (0, x.1)));
                walk_b = b;
            }
        } else if old_order {
            tagged.sort_by_key(|x| x.0);
            walk_b.sort_by_key(|x| x.0);
        } else {
            stl_sort(&mut tagged, |x| x.0);
            stl_sort(&mut walk_b, |x| x.0);
        }
        // (the car's two runs - its rows of the A-sorted list, where it is A, then of the B-sorted one - as the
        // callbacks are handed them, F-15350: every row of the run, the disabled ones too)
        let car_runs_at: [Vec<usize>; 2] = if let Some(mg) = merge.as_ref().filter(|m| !m.isl.list_a.is_empty()) {
            // (the bike in his island: its rows the car is A of - its contacts with the city, the loose pairs it is A in -
            // then those it is B of, in the build's two lists. His rows with the bike are left out of its runs, as in the
            // shared island of the bike alone (vehsim.rs: unverified))
            let car_side = |n: usize, as_a: bool| match src[n] {
                RowSrc::City(_) => as_a,
                RowSrc::Mani(k) => if as_a { self.manis[k].a == Who::Car } else { self.manis[k].b == Who::Car },
                RowSrc::Rider(_) => false,
            };
            let _ = mg;
            [tagged.iter().map(|x| x.1).filter(|&n| car_side(n, true)).collect(), walk_b.iter().map(|x| x.1).filter(|&n| car_side(n, false)).collect()]
        } else if merge.is_some() {
            [Vec::new(), Vec::new()]
        } else {
            [tagged.iter().filter(|x| x.0 == car_addr).map(|x| x.1).collect(), walk_b.iter().filter(|x| x.0 == car_addr).map(|x| x.1).collect()]
        };
        // (each other car's two runs, as the car's: its rows of the A-sorted list where its instance is A, then of the
        // B-sorted one)
        let extra_runs_at: Vec<[Vec<usize>; 2]> = self
            .extras
            .iter()
            .map(|e| {
                let k = self.objs[e.obj].addr;
                [tagged.iter().filter(|x| x.0 == k).map(|x| x.1).collect(), walk_b.iter().filter(|x| x.0 == k).map(|x| x.1).collect()]
            })
            .collect();
        let mut row_of = vec![usize::MAX; enabled.len()];
        for (r, x) in tagged.iter().filter(|x| x.2.is_some()).enumerate() {
            row_of[x.1] = r;
        }
        let by_b: Vec<usize> = walk_b.iter().filter(|x| enabled[x.1]).map(|x| row_of[x.1]).collect();
        // (the rider's rows among the enabled ones: their numbers)
        let rider_rows: Vec<(usize, usize)> = tagged.iter().filter(|x| x.2.is_some()).enumerate().filter_map(|(r, x)| if let RowSrc::Rider(j) = src[x.1] { Some((r, j)) } else { None }).collect();
        let list: Vec<(Link, Touch)> = tagged.into_iter().filter_map(|x| x.2).collect();
        let mut links: Vec<Link> = list.iter().map(|x| x.0).collect();
        let mut touches: Vec<Touch> = list.iter().map(|x| x.1).collect();
        let inv_dt = 1.0 / dt;
        let mut rounds_left = 64;
        // (the wake, F-44716 -> F-24099: an inactive instance that was hit before, in a contact of an island
        // with something breakable in it (island +75), gets a new body at rest. The decode has the call in the
        // breaking pass's listing (F-99488), after the first solve; the recording says the woken body is
        // already in the first solve - s512 / s518 of the long scrape exact only so - so it is done before the
        // rows. Whether the island +75 condition matters is not shown by the recording: the same run without it is
        // the same, bit for bit. Every manifold of the island counts, also one whose contacts are all disabled: V s_rtv
        // s473 / s496, the car's wheel 3 1.1 cm from asleep piece 22360 (contact flags 0x4d) wakes it.)
        self.wake_island(world, ids);
        loop {
            let (bodies, owners) = self.bodies_of(car, &links, &mut touches);
            let solved = match merge.as_mut() {
                Some(mg) => solver::island_solve_jointed(&bodies, &touches, std::slice::from_mut(&mut *mg.body), (0, 0.0), dt, inv_dt),
                None => solver::island_solve(&bodies, &touches, dt, inv_dt),
            };
            solver::island_impulses(&solved, &mut touches);
            if dbg() {
                for (l, t) in links.iter().zip(touches.iter()) {
                    eprintln!("LOOSESOLVE tick {} {:?}/{} x {:?}/{} depth {:?} mu {:?} e {:?} n {:?} P {:?} J {:?} |J| {}", self.tick, l.a, l.part_a, l.b, l.part_b, t.depth, t.friction, t.elasticity, t.normal, t.kept, t.impulse, t.impulse.length());
                }
            }
            // ---- F-80619: the rounds ----
            let mut j: Vec<Vec3> = touches.iter().map(|t| t.impulse).collect();
            let mut broke = false;
            for _ in 0..33 {
                // (F-99488: the breakable instances and the impulse on each of their parts)
                let mut listed: Vec<(usize, Vec<Vec3>)> = Vec::new();
                for (i, l) in links.iter().enumerate() {
                    for (who, part, first_minus) in [(l.a, l.part_a, false), (l.b, l.part_b, true)] {
                        let Some(Who::Obj(o)) = who else { continue };
                        if !self.breakable(world, o) {
                            continue;
                        }
                        match listed.iter().position(|x| x.0 == o) {
                            Some(k) => listed[k].1[part as usize] = listed[k].1[part as usize] + j[i],
                            None => {
                                let mut comp = vec![Vec3::ZERO; 128];
                                comp[part as usize] = if first_minus { Vec3::ZERO - j[i] } else { j[i] };
                                listed.push((o, comp));
                            }
                        }
                    }
                }
                // (F-29199: the instance that breaks at the least share)
                let mut win: Option<(usize, f32, u8)> = None;
                let mut least = 1.0f32;
                for (o, comp) in &listed {
                    if let Some((tk, g)) = break_strength(&world.types[self.objs[*o].ty], &self.objs[*o], comp) {
                        if tk < least {
                            least = tk;
                            win = Some((*o, tk, g));
                        }
                    }
                }
                let Some((w, share, group)) = win else { break };
                if dbg() {
                    eprintln!("LOOSEBREAK tick {} obj {w} group {group} t {:?} ({:08x})", self.tick, share, share.to_bits());
                }
                // (the winner's share of every contact of it, to its body - if it has one now -
                // and, the other way, to the other's)
                let w_has_body = self.objs[w].body.is_some();
                for (i, l) in links.iter().enumerate() {
                    let s = j[i] * share;
                    for side in 0..2 {
                        let who = if side == 0 { l.a } else { l.b };
                        if who != Some(Who::Obj(w)) {
                            continue;
                        }
                        let t = &touches[i];
                        let (point, vec) = if side == 0 { (t.a.point, s) } else { (t.b.point, Vec3::ZERO - s) };
                        if w_has_body {
                            self.objs[w].body.as_mut().unwrap().impulse_now(vec, point);
                        }
                        let (other, opoint, ovec) = if side == 0 { (l.b, t.b.point, Vec3::ZERO - s) } else { (l.a, t.a.point, s) };
                        match other {
                            Some(Who::Car) => {
                                *dynm.p = *dynm.p + ovec;
                                let vel = *dynm.p * car.inv_mass;
                                *dynm.l = *dynm.l + cross(opoint - car.at, ovec);
                                let ang = ex::spin_of(*dynm.l, &dynm.rows, dynm.inverse);
                                car.vel = vel;
                                car.ang = ang;
                            }
                            Some(Who::Obj(o)) => {
                                if let Some(b) = self.objs[o].body.as_mut() {
                                    b.impulse_now(ovec, opoint);
                                }
                            }
                            Some(Who::City) | None => {}
                        }
                    }
                }
                // (the break / the activation, F-27351)
                let target = self.break_apart(world, w, group);
                // (the contacts of the parts that left now name the new instance, F-79003)
                if let Some((new, bits)) = target {
                    let t = &world.types[self.objs[w].ty];
                    for l in links.iter_mut() {
                        for (who, part) in [(&mut l.a, l.part_a), (&mut l.b, l.part_b)] {
                            if *who == Some(Who::Obj(w)) && bits & (1 << group_of(t, part as usize)) != 0 {
                                *who = Some(Who::Obj(new));
                            }
                        }
                    }
                    for m in self.manis.iter_mut() {
                        if m.a == Who::Obj(w) && bits & (1 << group_of(t, m.part_a as usize)) != 0 {
                            m.a = Who::Obj(new);
                            m.stale = true;
                        }
                        if m.b == Who::Obj(w) && bits & (1 << group_of(t, m.part_b as usize)) != 0 {
                            m.b = Who::Obj(new);
                            m.stale = true;
                        }
                    }
                }
                let k1 = 1.0 - share;
                for (i, l) in links.iter().enumerate() {
                    if l.a == Some(Who::Obj(w)) || l.b == Some(Who::Obj(w)) {
                        j[i] = j[i] * k1;
                    }
                }
                broke = true;
            }
            rounds_left -= 1;
            if broke && rounds_left > 0 {
                continue;
            }
            // (nothing broke: the impulses go to the bodies' accumulators, F-71376)
            // (a body woken above is in the apply's list now: the owners are the solve's and the woken)
            let (bodies, owners) = self.bodies_of(car, &links, &mut touches);
            let given = solver::island_apply(&bodies, &touches);
            for (k, o) in owners.iter().enumerate() {
                let b = self.objs[*o].body.as_mut().unwrap();
                apply_impulse(b, given[k + 1].0);
                b.jt = b.jt + given[k + 1].1;
            }
            // (stage R, MCLA_DBG_VEHCON: every contact with a vehicle as it was solved, the numbers' bits -
            // tick, the vehicle's level index, depth, friction, elasticity, normal, A's point, B's point, impulse)
            if vehcon() {
                for (l, t) in links.iter().zip(touches.iter()) {
                    let other = [l.a, l.b].into_iter().find_map(|w| if let Some(Who::Obj(o)) = w { self.objs[o].veh } else { None });
                    if let Some(vh) = other {
                        let h = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
                        eprintln!("VEHCON {} {} {:08x} {:08x} {:08x} {} {} {} {}", REPLAY_TICK.load(std::sync::atomic::Ordering::Relaxed), vh.index, t.depth.to_bits(), t.friction.to_bits(), t.elasticity.to_bits(), h(t.normal), h(t.a.point), h(t.b.point), h(t.impulse));
                    }
                }
            }
            // (the contacts' numbers back into what is kept)
            for (l, t) in links.iter().zip(touches.iter()) {
                if let Some((k, i)) = l.from {
                    if let Some(s) = self.manis.get_mut(k).and_then(|m| m.slots.get_mut(i)) {
                        s.t.kept = t.kept;
                        s.t.impulse = t.impulse;
                    }
                }
            }
            // (each run as the callbacks find it, after the breaking pass: the first row's other object - re-pointed to
            // a piece if it broke off - active or not (level state 0: a body, not asleep), and whether a row is on a
            // part of the car that is not a wheel, and whether a row of it has an impulse from the solve)
            let side_of = |n: usize| -> Option<(Who, u16)> {
                match src[n] {
                    RowSrc::City(r) => Some((Who::City, city_part.get(r).copied().unwrap_or(0) as u16)),
                    RowSrc::Rider(_) => None,
                    RowSrc::Mani(k) => {
                        let m = &self.manis[k];
                        if m.a == Who::Car {
                            Some((m.b, m.part_a))
                        } else if m.b == Who::Car {
                            Some((m.a, m.part_b))
                        } else {
                            None
                        }
                    }
                }
            };
            let car_runs = car_runs_at
                .iter()
                .filter(|r| !r.is_empty())
                .map(|r| {
                    let first_active = side_of(r[0]).is_some_and(|(o, _)| matches!(o, Who::Obj(o) if self.objs.get(o).is_some_and(|x| x.body.is_some() && !x.dormant)));
                    let hull = r.iter().any(|&n| side_of(n).is_some_and(|(_, part)| part == 0));
                    let any = r.iter().any(|&n| row_of[n] != usize::MAX && touches[row_of[n]].impulse != Vec3::ZERO);
                    if super::vehsim::dbg_flag("MCLA_DBG_STUCK") { eprintln!("STUCKRUN tick {} first {:?} body {:?} dormant {:?} rows {}", REPLAY_TICK.load(std::sync::atomic::Ordering::Relaxed), side_of(r[0]), side_of(r[0]).and_then(|(o, _)| if let Who::Obj(o) = o { self.objs.get(o).map(|x| x.body.is_some()) } else { None }), side_of(r[0]).and_then(|(o, _)| if let Who::Obj(o) = o { self.objs.get(o).map(|x| x.dormant) } else { None }), r.len()); }
                    (first_active, hull, any)
                })
                .collect();
            // (the other cars' runs and disabled normals, for their own steps' callbacks)
            for (e, runs_at) in extra_runs_at.iter().enumerate() {
                let me = Who::Obj(self.extras[e].obj);
                let side_of = |n: usize| -> Option<(Who, u16)> {
                    let RowSrc::Mani(k) = src[n] else { return None };
                    let m = &self.manis[k];
                    if m.a == me {
                        Some((m.b, m.part_a))
                    } else if m.b == me {
                        Some((m.a, m.part_b))
                    } else {
                        None
                    }
                };
                let runs: Vec<(bool, bool, bool)> = runs_at
                    .iter()
                    .filter(|r| !r.is_empty())
                    .map(|r| {
                        let first_active = side_of(r[0]).is_some_and(|(o, _)| matches!(o, Who::Obj(o) if self.objs.get(o).is_some_and(|x| x.body.is_some() && !x.dormant)));
                        let hull = r.iter().any(|&n| side_of(n).is_some_and(|(_, part)| part == 0));
                        let any = r.iter().any(|&n| row_of[n] != usize::MAX && touches[row_of[n]].impulse != Vec3::ZERO);
                        (first_active, hull, any)
                    })
                    .collect();
                let (_, oa, ob) = extra_off[e];
                let x = &mut self.extras[e];
                x.runs = runs;
                x.off = (oa, ob);
            }
            // (the rider's impulses into his links, F-24076, and his rows' results back to his share)
            if let Some(mg) = merge.as_mut() {
                solver::island_apply_jointed(&solved, std::slice::from_mut(&mut *mg.body));
                for &(r, j) in &rider_rows {
                    mg.isl.touches[j] = touches[r];
                }
            }
            return IslandOut { impulse: given[0].0, turning: given[0].1, touches, links, bodies, owners, solved, by_b, off_a, off_b, car_runs, rider_rows };
        }
    }

    /// The prop instances' impact report after the island's solve (`F-81755`, slot 25; `prop_hit_fx_rule.md` 2a):
    /// per run of an instance's rows, as A and as B, the row with the largest impulse (the contact's +12, I) gives
    /// the point on the prop, the normal turned to the prop's side and the closing speed of the two bodies' velocities;
    /// when it is over 0.1 the type's events whose least speed it reaches are started at the point (+ the event's
    /// offset, world axes).
    pub fn impact_reports(&mut self, world: &LooseWorld, out: &IslandOut) {
        for as_a in [true, false] {
            let mut done: Vec<usize> = Vec::new();
            for l in out.links.iter() {
                let Some(Who::Obj(o)) = (if as_a { l.a } else { l.b }) else { continue };
                if done.contains(&o) {
                    continue;
                }
                done.push(o);
                let ob = &self.objs[o];
                if ob.veh.is_some() || ob.amb.is_some() || world.types[ob.ty].type_events.is_empty() {
                    continue;
                }
                let mut best: Option<(f32, usize)> = None;
                for (j, m) in out.links.iter().enumerate() {
                    if (if as_a { m.a } else { m.b }) == Some(Who::Obj(o)) && best.map_or(true, |b| out.touches[j].impulse.length() > b.0) {
                        best = Some((out.touches[j].impulse.length(), j));
                    }
                }
                let Some((_, j)) = best else { continue };
                let t = &out.touches[j];
                let (me, other, n) = if as_a { (&t.a, &t.b, t.normal) } else { (&t.b, &t.a, t.back) };
                let vp = me.body.and_then(|k| out.bodies.get(k)).map_or(Vec3::ZERO, |b| b.vel);
                let vo = other.body.and_then(|k| out.bodies.get(k)).map_or(Vec3::ZERO, |b| b.vel + b.ang.cross(me.point - b.at));
                let speed = -n.dot(vp - vo);
                if speed > 0.1 {
                    for e in &world.types[ob.ty].type_events {
                        if speed >= e.min_speed {
                            self.fx_events.push(FxPlay { effect: e.effect.clone(), at: FxAt::World(me.point + v(e.offset)), vel: Vec3::ZERO });
                        }
                    }
                }
            }
        }
    }

    /// Start the events of a break set on object `w`, following the matrix of `part` (none: the instance's).
    fn fx_follow(&mut self, w: usize, set: &[mcla_formats::rage::fragphys::PropEvent], part: Option<u16>) {
        for e in set {
            self.fx_events.push(FxPlay { effect: e.effect.clone(), at: FxAt::Follow { obj: w, part, offset: v(e.offset) }, vel: Vec3::ZERO });
        }
    }

    /// The break-from-root sets of group `g` and, below it, of its sub-groups that have not broken (`F-96808`).
    fn fx_root(&mut self, t: &FragType, w: usize, g: usize, absent: u64) {
        let Some(gr) = t.groups.get(g) else { return };
        let part = fx_part(t, g);
        for c in gr.first_child as usize..gr.first_child as usize + gr.children as usize {
            if let Some(ch) = t.children.get(c) {
                self.fx_follow(w, &ch.break_root_events, part);
            }
        }
        for sg in gr.first_group as usize..gr.first_group as usize + gr.groups as usize {
            if sg < 64 && absent & (1 << sg) == 0 {
                self.fx_root(t, w, sg, absent);
            }
        }
    }

    /// The matrix an event effect follows, with the offset it is placed at (`FxAt::Follow`).
    pub fn fx_matrix(&self, world: &LooseWorld, obj: usize, part: Option<u16>) -> Option<[Vec3; 4]> {
        let o = self.objs.get(obj)?;
        match part {
            None => Some(o.matrix),
            // (the bone as the unposed skeleton has it, carried by the instance's matrix; a prop that has not been
            // posed has every bone where its skeleton puts it - the hydrant's are both the identity)
            Some(b) => Some(world.types[o.ty].bones.get(b as usize).map_or(o.matrix, |m| super::gjk::world_matrix(&m4(m), &o.matrix))),
        }
    }

    /// F-27351 -> F-97875: an instance breaks at `group` (255: it goes as a whole). Gives
    /// the new instance and the groups it took, if a piece came off.
    fn break_apart(&mut self, world: &LooseWorld, w: usize, group: u8) -> Option<(usize, u64)> {
        let ty = self.objs[w].ty;
        let t = &world.types[ty];
        let (max_speed, max_spin) = (BODY_MAX_SPEED, BODY_MAX_SPIN);
        let was_hit = self.objs[w].hit;
        if self.objs[w].entry.is_none() {
            if t.flags & 1 == 0 {
                // (it goes as it is, with the file's archetype: child 0's break set on the instance's matrix, F-27351)
                if let Some(c0) = t.children.first() {
                    self.fx_follow(w, &c0.break_events, None);
                }
                let obj = &mut self.objs[w];
                obj.hit = true;
                if obj.body.is_none() {
                    let a = &t.archetype;
                    let e = Entry { absent: 0, age: 0.0, cg: v(t.cg), mass: a.mass, inv_mass: a.inv_mass, inertia: v(a.inertia), inv_inertia: v(a.inv_inertia), ..Default::default() };
                    obj.body = Some(PBody::new(&obj.matrix, e.cg, &e, a.max_speed, a.max_spin));
                    (obj.sleep, obj.dormant) = (Sleep::reset(), false);
                }
                return None;
            }
            // (the prop gets an entry that holds every group, F-44697; from the pool, F-18636 - a type that keeps one
            // has held it since it was put in the level)
            let mut e = Entry::default();
            let mass = entry_mass(t, 0);
            entry_refresh(t, &mut e, mass);
            if !t.keeps_entry {
                (e.slot, e.stamp) = self.pool_take(world);
            }
            self.objs[w].entry = Some(e);
            // (the entry has an instance of its own, which a prop's does not use)
            self.entry_instance();
        }
        if group == 255 {
            // (the whole instance comes loose: the break-from-root sets of the root groups, A-75924)
            let absent = self.objs[w].entry.map_or(0, |e| e.absent);
            for g in 0..t.root_groups as usize {
                self.fx_root(t, w, g, absent);
            }
            let obj = &mut self.objs[w];
            obj.hit = true;
            if obj.body.is_none() {
                let e = obj.entry.unwrap();
                obj.body = Some(PBody::new(&obj.matrix, e.cg, &e, max_speed, max_spin));
                (obj.sleep, obj.dormant) = (Sleep::reset(), false);
            }
            return None;
        }
        let parent = self.objs[w].entry.unwrap();
        let bits = subtree(t, group as usize) & !parent.absent;
        let all: u64 = (0..t.groups.len()).fold(0, |b, g| b | 1 << g);
        // (something must stay behind, else it goes as a whole)
        if (all & !parent.absent) & !bits == 0 {
            for g in 0..t.root_groups as usize {
                self.fx_root(t, w, g, parent.absent);
            }
            let obj = &mut self.objs[w];
            obj.hit = true;
            if obj.body.is_none() {
                obj.body = Some(PBody::new(&obj.matrix, parent.cg, &parent, max_speed, max_spin));
                (obj.sleep, obj.dormant) = (Sleep::reset(), false);
            }
            return None;
        }
        // (the new entry: the parent's state, less everything outside the group's own tree)
        let mut e = Entry { absent: all & !bits, age: parent.age, from: self.objs[w].inst.or(parent.from), ..Default::default() };
        let mass = entry_mass(t, e.absent);
        entry_refresh(t, &mut e, mass);
        // (its own entry from the pool, F-18636)
        (e.slot, e.stamp) = self.pool_take(world);

        let matrix = self.objs[w].matrix;
        let mut body = PBody::new(&matrix, e.cg, &e, max_speed, max_spin);
        // (broken off a moving parent it takes the parent's motion with it, F-47138 / F-74100)
        if let Some(pb) = self.objs[w].body {
            let pos = xf(&matrix, parent.cg);
            let vel = piece_velocity(matrix[3], pos, pb.v, pb.w);
            set_velocity(&mut body, vel);
            body.w = pb.w;
            body.l = body.momentum_of(pb.w);
        }
        // (the events of the break, F-97875: with the instance not broken before, the break-from-root sets of the
        // group's subtree, then the break sets of the group's own children, on the part that stays)
        if !was_hit {
            self.fx_root(t, w, group as usize, parent.absent);
        }
        if let Some(gr) = t.groups.get(group as usize) {
            let part = fx_part(t, group as usize);
            for c in gr.first_child as usize..gr.first_child as usize + gr.children as usize {
                if let Some(ch) = t.children.get(c) {
                    self.fx_follow(w, &ch.break_events, part);
                }
            }
        }
        self.pieces_made += 1;
        let n = self.pieces_made;
        let new = self.objs.len();
        let (inst_addr, index) = (self.entry_instance(), self.new_index());
        self.objs.push(Obj { ty, inst: None, matrix, entry: Some(e), hit: true, body: Some(body), level: index.unwrap_or(rank('l', new).unwrap_or(if hint("pb", n) { 2_000_000 + n as i64 } else { -1000 - n as i64 })), addr: inst_addr.unwrap_or(rank('a', new).unwrap_or(if hint("pafter", n) { 2_000_000 + n as i64 } else { -1000 - n as i64 })), veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
        if dbg() {
            eprintln!("LOOSENEW tick {} obj {new} level {} addr {:x}", self.tick, self.objs[new].level, self.objs[new].addr);
        }
        // (the parent loses the groups, F-85792 and F-13409; a body keeps its velocities)
        let mut pe = parent;
        pe.absent |= bits;
        let rest = entry_mass(t, pe.absent);
        entry_refresh(t, &mut pe, rest);
        let obj = &mut self.objs[w];
        obj.entry = Some(pe);
        if let Some(b) = obj.body.as_mut() {
            b.rows = [obj.matrix[0], obj.matrix[1], obj.matrix[2]];
            b.at = xf(&obj.matrix, pe.cg);
            b.mass = pe.mass;
            b.inv_mass = pe.inv_mass;
            b.inertia = pe.inertia;
            b.inv_inertia = pe.inv_inertia;
            b.p = b.v * b.mass;
            b.l = b.momentum_of(b.w);
        }
        if dbg() {
            eprintln!("LOOSEPIECE tick {} from obj {w}: obj {new} groups {bits:b} mass {:?} cg {:?} inertia {:?}; the rest mass {:?} cg {:?}", self.tick, e.mass, e.cg, e.inertia, pe.mass, pe.cg);
        }
        Some((new, bits))
    }

    /// The island's push solve (F-56590) and what it leaves on the loose bodies; gives the
    /// car's push.
    pub fn push_car_island(&mut self, out: &mut IslandOut, car: &Body, car_start: Pushed, dt: f32) -> Pushed {
        self.push_island(out, car, car_start, dt, None)
    }

    /// [`Self::push_car_island`] with the thrown rider's ragdoll the island's jointed body (F-49764's rows of type 7 / 13,
    /// his velocities committed and his push multipliers applied, F-64140); his rows' results back in `merge.isl`.
    pub fn push_island(&mut self, out: &mut IslandOut, car: &Body, car_start: Pushed, dt: f32, merge: Option<&mut RiderMerge>) -> Pushed {
        // (the bodies as the integration left them: only their inverse masses and matrices
        // enter - and those have not changed)
        let mut bodies = out.bodies.clone();
        bodies[0] = *car;
        let mut pushes = vec![Pushed::default(); bodies.len()];
        pushes[0] = car_start;
        for (k, o) in out.owners.iter().enumerate() {
            let b = self.objs[*o].body.unwrap();
            pushes[k + 1] = Pushed { push: b.push, turn: b.turn, deep: false };
        }
        match merge {
            Some(mg) => {
                solver::island_push_jointed(&bodies, &mut out.touches, &mut out.solved, std::slice::from_mut(&mut *mg.body), dt, 1.0 / dt, &mut pushes);
                for &(r, j) in &out.rider_rows {
                    mg.isl.touches[j] = out.touches[r];
                }
            }
            None => solver::island_push(&bodies, &mut out.touches, &mut out.solved, dt, 1.0 / dt, &mut pushes),
        }
        for (k, o) in out.owners.iter().enumerate() {
            let b = self.objs[*o].body.as_mut().unwrap();
            b.push = pushes[k + 1].push;
            b.turn = pushes[k + 1].turn;
            if pushes[k + 1].deep && !b.deep {
                b.deep = true;
                b.deep_count = b.deep_count.saturating_add(1).min(254);
            }
        }
        for (l, t) in out.links.iter().zip(out.touches.iter()) {
            if let Some((k, i)) = l.from {
                if let Some(s) = self.manis.get_mut(k).and_then(|m| m.slots.get_mut(i)) {
                    s.t.pushed = t.pushed;
                }
            }
        }
        pushes[0]
    }
}

// ---------------------------------------------------------------------------------------------
// The loose bodies' own tick (p1/body_decode.md): the move, gravity and damping ahead of the
// collide (F-44566), the integration after the solves (F-94789), the push applied
// (F-32811), and the entries' ages (F-26321).

const GRAVITY: f32 = f32::from_bits(0xc11c_cccd);

impl LooseState {
    /// The groups of manifolds that hang together through an object that can move or be moved
    /// (the city joins nothing): the car's - empty if nothing loose touches it - and the rest.
    fn islands(&self) -> (Vec<usize>, Vec<Vec<usize>>) {
        if !super::vehsim::dbg_flag("MCLA_DBG_RA2") {
            let live = |m: &Mani| m.slots.iter().any(|s| s.enabled);
            let mut car = Vec::new();
            let mut rest: Vec<(usize, Vec<usize>)> = Vec::new();
            for (k, m) in self.manis.iter().enumerate() {
                if m.island == 0 {
                    car.push(k);
                } else {
                    match rest.iter().position(|x| x.0 == m.island) {
                        Some(i) => rest[i].1.push(k),
                        None => rest.push((m.island, vec![k])),
                    }
                }
            }
            // (an island's rows count the contacts that are not enabled too; one with none enabled is not solved)
            if !car.iter().any(|k| live(&self.manis[*k])) {
                car.clear();
            }
            rest.retain(|x| x.1.iter().any(|k| live(&self.manis[*k])));
            return (car, rest.into_iter().map(|x| x.1).collect());
        }
        let n = self.objs.len() + 1;
        let mut root: Vec<usize> = (0..n).collect();
        fn find(root: &mut Vec<usize>, mut x: usize) -> usize {
            while root[x] != x {
                root[x] = root[root[x]];
                x = root[x];
            }
            x
        }
        let node = |w: Who| match w {
            Who::Car => Some(0),
            Who::Obj(o) => Some(o + 1),
            Who::City => None,
        };
        let live = |m: &Mani| m.slots.iter().any(|s| s.enabled);
        for m in self.manis.iter().filter(|m| live(m)) {
            if let (Some(a), Some(b)) = (node(m.a), node(m.b)) {
                let (ra, rb) = (find(&mut root, a), find(&mut root, b));
                if ra != rb {
                    root[rb.max(ra)] = rb.min(ra);
                }
            }
        }
        let car_root = find(&mut root, 0);
        let mut car = Vec::new();
        let mut rest: Vec<(usize, Vec<usize>)> = Vec::new();
        for (k, m) in self.manis.iter().enumerate() {
            if !live(m) {
                continue;
            }
            let r = find(&mut root, node(m.a).unwrap_or(0));
            if r == car_root {
                car.push(k);
            } else {
                match rest.iter().position(|x| x.0 == r) {
                    Some(i) => rest[i].1.push(k),
                    None => rest.push((r, vec![k])),
                }
            }
        }
        // (the car's own island holds something loose only if a manifold of the car is in it)
        if !car.iter().any(|k| self.manis[*k].a == Who::Car || self.manis[*k].b == Who::Car) {
            for k in car.drain(..) {
                let r = node(self.manis[k].a).unwrap_or(0);
                match rest.iter().position(|x| x.0 == r) {
                    Some(i) => rest[i].1.push(k),
                    None => rest.push((r, vec![k])),
                }
            }
        }
        (car, rest.into_iter().map(|x| x.1).collect())
    }

    /// For whoever draws the props (stage P4): the prop instances that have lost groups - the instance's number in
    /// the file's order and the bit set of the groups gone - and every piece or loose prop that has a body or lies asleep (`dormant`): its
    /// type, the groups it does NOT hold, and its instance's matrix (rows and place).
    pub fn drawn(&self) -> (Vec<(u32, u64)>, Vec<(usize, u64, [Vec3; 4])>) {
        let broken = self.objs.iter().filter_map(|o| Some((o.inst?, o.entry?.absent))).filter(|x| x.1 != 0).collect();
        let moving = self.objs.iter().filter(|o| o.body.is_some() || o.dormant).map(|o| (o.ty, o.entry.map_or(0, |e| e.absent), o.matrix)).collect();
        (broken, moving)
    }

    /// The wake of an island's inactive objects (F-44716 -> F-24099): every object of every manifold `ids` - one
    /// whose contacts are all disabled too - when the island has a prop or piece in it.
    fn wake_island(&mut self, world: &LooseWorld, ids: &[usize]) {
        let objs: Vec<usize> = ids.iter().flat_map(|&k| [self.manis[k].a, self.manis[k].b]).filter_map(|w| if let Who::Obj(o) = w { Some(o) } else { None }).collect();
        if dbg() {
            eprintln!("LOOSEWAKEISLE tick {} {:?}", self.tick, objs.iter().map(|&o| (o, self.breakable(world, o), self.objs[o].dormant, self.objs[o].hit, self.objs[o].body.is_some())).collect::<Vec<_>>());
        }
        // (every island with a prop or piece in it goes to the breaking pass - gjkfix part 10: contact limit +60 = 0,
        // island +75 set for every island - not only one with something left to break: V s_rtv s473, 22360 a piece of
        // one group woken by the car's island)
        if objs.iter().any(|&o| self.objs[o].veh.is_none()) {
            for o in objs {
                self.wake_obj(world, o);
            }
        }
    }

    /// The car's island with no loose contact enabled in it: the port solves the car alone (or not at all), the original
    /// has the car's loose manifolds in the island and wakes what they hold, solved or not (V s_rtv s473: the car's wheel
    /// 3 1.1 cm from asleep piece 22360, with car x city contacts; s496: wheel 1, none - contact flags 0x4d both times).
    pub fn wake_car_island(&mut self, world: &LooseWorld) {
        let ids: Vec<usize> = (0..self.manis.len()).filter(|&k| self.manis[k].island == 0).collect();
        self.wake_island(world, &ids);
    }

    /// The manifolds of the car's island (empty: the car's island is the car and the city).
    pub fn car_island(&self) -> Vec<usize> {
        self.islands().0
    }

    /// The manifolds of the island no car is in that holds object `o` (empty: none). Whether it sleeps is not asked
    /// (deactivation B runs with the islands' step, `step_rest_rider`).
    pub fn island_with(&self, o: usize) -> Vec<usize> {
        self.islands().1.into_iter().find(|ids| ids.iter().any(|&k| self.manis[k].a == Who::Obj(o) || self.manis[k].b == Who::Obj(o))).unwrap_or_default()
    }

    /// The city instance's key in this world's numbering (as the island's sort takes it).
    pub fn city_key(&self) -> i64 {
        self.city_addr()
    }

    /// Every island the car is not in: its solve, its breaking pass, and its push solve.
    pub fn step_rest(&mut self, world: &LooseWorld, car_in: &CarIn, dt: f32) {
        self.step_rest_rider(world, car_in, dt, None);
    }

    /// [`Self::step_rest`] with the thrown rider whose partner vehicle is in an island of the loose world (`rider`: his
    /// ragdoll, his share of the island's build, his tick's answers): that island and his are one (F-96487 / F-14298:
    /// every pair that holds a contact joins its objects, one solve per island) - his build over the island's pairs
    /// ([`Self::rider_nodes`]), one solve and one push with his ragdoll the jointed body, his post-solve passes between
    /// them and his post-push after. An island of his partner that is not found (asleep, or gone into a car's island by
    /// then) leaves him in his own island without the vehicle's rows.
    pub fn step_rest_rider(&mut self, world: &LooseWorld, car_in: &CarIn, dt: f32, mut rider: Option<(&mut super::artic_run::RiderRagdoll, RiderJob, &mut super::artic_run::TickOut)>) {
        let asleep = self.islands_asleep();
        let merged = std::mem::take(&mut self.merged);
        for ids in self.islands().1 {
            if ids.iter().any(|k| asleep.contains(&self.manis[*k].island)) {
                continue;
            }
            // (solved already, in the bike's island with the rider)
            if ids.iter().any(|&k| [self.manis[k].a, self.manis[k].b].iter().any(|w| matches!(w, Who::Obj(o) if merged.contains(o)))) {
                continue;
            }
            let mut none = Body { at: Vec3::ZERO, rows: [Vec3::X, Vec3::Y, Vec3::Z], vel: Vec3::ZERO, ang: Vec3::ZERO, inv_mass: 0.0, inv_moments: Vec3::ZERO, force: Vec3::ZERO, torque: Vec3::ZERO };
            let (mut p, mut l) = (Vec3::ZERO, Vec3::ZERO);
            let mut dynm = CarDyn { p: &mut p, l: &mut l, mass: 0.0, inverse: Vec3::ZERO, rows: [Vec3::X, Vec3::Y, Vec3::Z] };
            let holds = rider.as_ref().is_some_and(|(_, job, _)| ids.iter().any(|&k| self.manis[k].a == Who::Obj(job.other) || self.manis[k].b == Who::Obj(job.other)));
            if !holds {
                let mut out = self.solve_car_island(world, car_in, &mut none, &mut dynm, &[], &[], &[], &ids, dt);
                self.push_car_island(&mut out, &none, Pushed::default(), dt);
                continue;
            }
            let (rd, job, tick) = rider.take().unwrap();
            let (nodes, slots) = self.rider_nodes(&ids);
            let mut outside = job.outside;
            let o = job.other;
            if let Some(x) = outside.other.as_mut() {
                x.level = self.objs[o].level;
                x.rider_level = RIDER_LOOSE_LEVEL;
                x.shared = true;
            }
            if let Some(b) = outside.bike.as_mut() {
                b.rider_level = RIDER_LOOSE_LEVEL;
            }
            outside.loose = Some(super::artic_run::LooseRows { nodes, rider_level: RIDER_LOOSE_LEVEL, rider_key: RIDER_LOOSE_KEY, city_key: self.city_addr(), other_key: self.objs[o].addr, bike_key: self.car_place().1 });
            let isl = rd.build(&outside);
            tick.island = true;
            tick.contacts = isl.city_rows;
            let mut mg = RiderMerge { body: &mut rd.body, isl, slots, other: o, rider_key: RIDER_LOOSE_KEY };
            let mut out = self.solve_island(world, car_in, &mut none, &mut dynm, &[], &[], &[], &ids, dt, Some(&mut mg));
            let RiderMerge { isl, slots, .. } = mg;
            let (fall, motion) = rd.after_solve(&isl, dt);
            tick.body_fall = fall;
            tick.motion = motion;
            let mut mg = RiderMerge { body: &mut rd.body, isl, slots, other: o, rider_key: RIDER_LOOSE_KEY };
            self.push_island(&mut out, &none, Pushed::default(), dt, Some(&mut mg));
            let isl = mg.isl;
            rd.finish(&isl, dt);
        }
        // (his partner's island not found: his own, the vehicle's rows not in it)
        if let Some((rd, job, tick)) = rider {
            let mut outside = job.outside;
            if let Some(x) = outside.other.as_mut() {
                x.shared = false;
            }
            let isl = rd.build(&outside);
            rd.alone(isl, (0, 0.0), dt, tick);
        }
    }

    /// The pairs of an island (`ids`, manifolds) as nodes of the merged walk ([`super::artic_run::WalkNode`]): one node per
    /// pair of objects (the constraints' manifolds and the contacts' of two objects one pair, as the collide's union-find
    /// takes them), in the collide's order - the constraints' pairs first, then by (lower, higher) level, the city lowest -
    /// each node's rows its sub-manifolds by (B's part or the city's cell, A's part), contacts by number, the disabled
    /// ones included, every row with its instance A's and B's address; and where each row is kept (manifold, slot).
    pub fn rider_nodes(&self, ids: &[usize]) -> (Vec<super::artic_run::WalkNode>, Vec<Vec<(usize, usize)>>) {
        let city_addr = self.city_addr();
        let (_, car_addr) = self.car_place();
        let addr = |w: Who| match w {
            Who::Obj(o) => self.objs[o].addr,
            Who::Car => car_addr,
            Who::City => city_addr,
        };
        let lvl = |w: Who| if w == Who::City { None } else { Some(self.level(w)) };
        let mut keys: Vec<((i64, i64), bool, Vec<usize>)> = Vec::new();
        for &k in ids {
            let m = &self.manis[k];
            let (la, lb) = (self.level(m.a), self.level(m.b));
            let key = (la.min(lb), la.max(lb));
            match keys.iter_mut().find(|x| x.0 == key) {
                Some(x) => {
                    x.1 |= m.rope;
                    x.2.push(k);
                }
                None => keys.push((key, m.rope, vec![k])),
            }
        }
        let mut nodes = Vec::new();
        let mut slots = Vec::new();
        for (_, rope, mut ms) in keys {
            ms.sort_by_key(|&k| {
                let m = &self.manis[k];
                (if m.b == Who::City { m.cell as u64 } else { m.part_b as u64 }, m.part_a as u64)
            });
            let m0 = &self.manis[ms[0]];
            let mut n = super::artic_run::WalkNode { a: lvl(m0.a), b: lvl(m0.b), keys: Vec::new(), constraint: rope };
            let mut sl = Vec::new();
            for &k in &ms {
                let m = &self.manis[k];
                for i in 0..m.slots.len() {
                    n.keys.push((addr(m.a), addr(m.b)));
                    sl.push((k, i));
                }
            }
            nodes.push(n);
            slots.push(sl);
        }
        (nodes, slots)
    }

    /// The islands no car is in, for a level shared by the cars (the game: once a tick, after every car).
    pub fn step_rest_alone(&mut self, world: &LooseWorld, dt: f32) {
        self.step_rest_alone_rider(world, dt, None);
    }

    /// [`Self::step_rest_alone`] with the thrown rider's island left by his bike's step (`VehState::rider_job`).
    pub fn step_rest_alone_rider(&mut self, world: &LooseWorld, dt: f32, rider: Option<(&mut super::artic_run::RiderRagdoll, RiderJob, &mut super::artic_run::TickOut)>) {
        static NONE: std::sync::OnceLock<HullShape> = std::sync::OnceLock::new();
        let hull = NONE.get_or_init(|| HullShape::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z], &[[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]]));
        let m = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO];
        let car = CarIn { driver: Default::default(), hull, hull_cg: Vec3::ZERO, hull_margin: 0.04, now: m, last: m, prev: m, speed: 0.0, bound: (Vec3::ZERO, Vec3::ZERO), wheels: Vec::new(), vel: Vec3::ZERO, touch: (0.0, 0.0), wiped: false };
        self.step_rest_rider(world, &car, dt, rider);
    }

    /// (replay diagnostics, `MCLA_REPLAY_PIECES=<file>`: every loose body put into the recording's state before each
    /// update - lines "<update> <level index>" and the pool body's place, rows, v, w, P, L, force, torque, the two
    /// impulse sums, push, turn, its "last" matrix (12) and whether it is set, and the instance's matrix (12), as
    /// work/proppath/mkpieces.py writes them from the OBJ records - so that each tick is one step from the original's)
    fn replay_pieces(&mut self) {
        static P: std::sync::OnceLock<HashMap<(i64, i64), Vec<f32>>> = std::sync::OnceLock::new();
        let p = P.get_or_init(|| {
            let Some(path) = std::env::var_os("MCLA_REPLAY_PIECES") else { return HashMap::new() };
            std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                let (k, lev) = (w.first()?.parse().ok()?, w.get(1)?.parse().ok()?);
                Some(((k, lev), w[2..].iter().filter_map(|x| x.parse().ok()).collect()))
            }).collect()
        });
        if p.is_empty() {
            return;
        }
        let k = REPLAY_K.load(std::sync::atomic::Ordering::Relaxed);
        for o in self.objs.iter_mut() {
            let Some(b) = o.body.as_mut() else { continue };
            let Some(q) = p.get(&(k, o.level)).filter(|q| q.len() >= 70) else { continue };
            let v3 = |i: usize| Vec3::new(q[i], q[i + 1], q[i + 2]);
            b.at = v3(0);
            b.rows = [v3(3), v3(6), v3(9)];
            b.v = v3(12);
            b.w = v3(15);
            b.p = v3(18);
            b.l = v3(21);
            b.force = v3(24);
            b.torque = v3(27);
            b.jf = v3(30);
            b.jt = v3(33);
            b.push = v3(36);
            b.turn = v3(39);
            b.last = [v3(42), v3(45), v3(48), v3(51)];
            b.last_set = q[54] != 0.0;
            o.matrix = [v3(55), v3(58), v3(61), v3(64)];
            b.deep = q[67] != 0.0;
            b.deep_count = q[68] as u8;
            b.moves = q[69] as u32;
        }
    }

    /// The instance's matrix from the body (F-18040).
    fn matrix_from_body(&mut self, world: &LooseWorld, o: usize) {
        let cg = self.cg_of(world, o);
        let obj = &mut self.objs[o];
        if let Some(b) = obj.body.as_ref() {
            obj.matrix = [b.rows[0], b.rows[1], b.rows[2], ex::origin_of(b.at, &b.rows, cg)];
        }
    }

    /// Ahead of the collide (F-44566): what the push solve left is applied, the body is moved
    /// with the velocities the tick came in with, and gravity and the type's damping go into the
    /// tick's force sums.
    pub fn pre_collide(&mut self, world: &LooseWorld, dt: f32) {
        (self.collided, self.refreshed) = (false, false);
        self.replay_pieces();
        self.pieces_wake(dt);
        self.bodies_wake(dt);
        self.sync_others();
        for o in 0..self.objs.len() {
            let Some(mut b) = self.objs[o].body else { continue };
            let veh = self.objs[o].veh;
            // (a live car is moved by its own step; `set_vehicles` hands it in already moved - an ambient
            // car's body is the level's)
            if veh.is_some() && self.live && self.objs[o].amb.is_none() {
                continue;
            }
            let (gravity, damping) = match veh {
                Some(vh) => (vh.gravity, VEH_DAMPING),
                None => {
                    let t = &world.types[self.objs[o].ty];
                    (t.archetype.gravity, t.damping)
                }
            };
            b.prev = self.objs[o].matrix;
            // (F-67019 - normally nothing is pending)
            b.at = b.at + b.push;
            b.push = Vec3::ZERO;
            if solver::turn_applies(b.turn) {
                b.rows = solver::rows_pushed(&b.rows, b.turn);
                b.turn = Vec3::ZERO;
            }
            // (F-10928)
            b.at = b.v * dt + b.at;
            let turn = b.w * dt;
            if dot3(turn, turn) > f32::from_bits(0x2b8c_bccc) {
                b.rows = ex::rows_turned(&b.rows, turn);
                (b.w, b.l) = capped(ex::spin_of(b.l, &b.rows, b.inv_inertia), b.l, b.max_spin);
            }
            self.objs[o].body = Some(b);
            self.matrix_from_body(world, o);
            // (gravity, F-37896: a force)
            let g = Vec3::new(0.0 * gravity, GRAVITY * gravity, 0.0 * gravity);
            b.force = b.force + g * b.mass;
            // (damping, F-66303: in the body's axes, of an entry's archetype - and of the file's own
            // archetype for a prop that went loose whole (phArchetypeDamp, vtable[17] true: V s_rtv 11286,
            // 25.5 kg, in flight from s386 its speed 0.269 % less a tick on every axis, the type's 0.02 /
            // 0.1 / 0.02 at 2.7 m/s). Not the torn bumper's plain phArchetypePhys.)
            let file_type = veh.is_none() && world.types[self.objs[o].ty].off != usize::MAX;
            if self.objs[o].entry.is_some() || veh.is_some() || file_type {
                let (df, dtq) = damping_forces(&b.rows, b.v, b.w, b.mass, b.inertia, &damping, dt);
                b.force = b.force + df;
                b.torque = b.torque + dtq;
            }
            b.moves += 1;
            if b.moves > 32 {
                b.rows = ex::rows_squared(&b.rows);
                b.moves = 0;
            }
            self.objs[o].body = Some(b);
            self.matrix_from_body(world, o);
            if dbg() {
                eprintln!("LOOSEBODY tick {} obj {o} at {:?} v {:?} w {:?} origin {:?} up {:?}", self.tick + 1, b.at, b.v, b.w, self.objs[o].matrix[3], b.rows[1]);
            }
        }
    }

    /// After the solves: the forces and impulses into the momenta (F-94789), the push solve's
    /// answer into the pose (F-32811), and every entry a tick older (F-26321).
    pub fn end_tick(&mut self, world: &LooseWorld, dt: f32) {
        self.end_tick_with(world, dt, None)
    }

    /// `end_tick`, with the city an ambient car's wheels probe (slot 14, after the integration and
    /// before the push goes into the pose: F-37370, then the base F-32811).
    pub fn end_tick_with(&mut self, world: &LooseWorld, dt: f32, city: Option<&super::collision::WorldCollider>) {
        self.pieces_sleep(dt);
        self.bodies_sleep(dt);
        for o in 0..self.objs.len() {
            let Some(mut b) = self.objs[o].body else { continue };
            if self.objs[o].veh.is_some() && self.live && self.objs[o].amb.is_none() {
                continue;
            }
            b.jf = b.force * dt + b.jf;
            b.jt = b.torque * dt + b.jt;
            b.force = Vec3::ZERO;
            b.torque = Vec3::ZERO;
            b.take_impulses();
            if let Some(city) = city {
                let m = self.objs[o].matrix;
                if let Some(a) = self.objs[o].amb.as_mut() {
                    ambient_wheels(a, &mut b, &m, city, dt);
                }
            }
            // (F-67019, then the instance's matrix and the collide's "last" one)
            b.at = b.at + b.push;
            b.push = Vec3::ZERO;
            if solver::turn_applies(b.turn) {
                b.rows = solver::rows_pushed(&b.rows, b.turn);
                b.turn = Vec3::ZERO;
            }
            self.objs[o].body = Some(b);
            self.matrix_from_body(world, o);
            let now = self.objs[o].matrix;
            let b = self.objs[o].body.as_mut().unwrap();
            let (took, deep, count, set) = super::solver::end_matrix(b.deep as u8, b.deep_count, b.last_set as u8);
            (b.deep, b.deep_count, b.last_set) = (deep != 0, count, set != 0);
            if took {
                b.last = now;
            }
            // (then the instance's vtable[27] (F-18540 -> F-42894) calls the prop's "moved" callback F-33764: an
            // active prop, or a piece broken off one, whose place is below y -300 is taken out of the simulator
            // (F-38307), inactive at that matrix - one that fell through the world stops falling. A car or a torn
            // bumper is no prop instance)
            if self.objs[o].veh.is_none() && !self.pieces.iter().any(|p| p.obj == o) && now[3].y < -300.0 {
                self.deactivate(o);
            }
        }
        // (the entries' ages; a quarter of a second after the first hit the centre of gravity
        // goes back where the parts have it)
        for o in 0..self.objs.len() {
            let ty = self.objs[o].ty;
            let Some(mut e) = self.objs[o].entry else { continue };
            let t = &world.types[ty];
            let age = e.age;
            e.age = age + dt;
            if age < 0.25 && e.age >= 0.25 && v(t.cg_shift) != Vec3::ZERO {
                let mass = entry_mass(t, e.absent);
                entry_refresh(t, &mut e, mass);
                let obj = &mut self.objs[o];
                if let Some(b) = obj.body.as_mut() {
                    b.rows = [obj.matrix[0], obj.matrix[1], obj.matrix[2]];
                    b.at = xf(&obj.matrix, e.cg);
                    b.mass = e.mass;
                    b.inv_mass = e.inv_mass;
                    b.inertia = e.inertia;
                    b.inv_inertia = e.inv_inertia;
                    b.p = b.v * b.mass;
                    b.l = b.momentum_of(b.w);
                }
            }
            self.objs[o].entry = Some(e);
        }
        // (F-89933: after the entries' updates the pool's frame steps; the contacts of the tick have stamped theirs)
        self.pool_stamp_contacts();
        self.pool_frame = self.pool_frame.wrapping_add(1);
        self.pool_teardown();
        if dbg() {
            let h = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
            for o in self.objs.iter() {
                if let Some(b) = o.body.as_ref() {
                    eprintln!("LOOSEEND k {} level {} age{:?}/{:08x} at {} v {} w {} p {} l {} m {} {} {} {}", REPLAY_K.load(std::sync::atomic::Ordering::Relaxed), o.level, o.entry.map(|e| e.age), dt.to_bits(), h(b.at), h(b.v), h(b.w), h(b.p), h(b.l), h(o.matrix[0]), h(o.matrix[1]), h(o.matrix[2]), h(o.matrix[3]));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Stage R (agent D): another VEHICLE as a body of the car's island (flag `MCLA_DBG_RD1` = none).
// An ambient traffic vehicle (instance class 0x820C4314, class number 6, collider class
// 0x820C4114) is one polyhedron bound (type 4, `resources/ambients/<name>/*.r155`) on a body of
// 600 kg whose archetype (phArchetypeDamp, made by F-58444) has gravity factor 1.2 and the
// damping (C, V, V2) = (0.01, 0.01, 0.001) for moving and for turning. Its own driving (the AI,
// its ray suspension F-37370 that leaves its forces in the body's sums for the next step) is
// the session's: a replay hands the body's state before every tick (`MCLA_REPLAY_OTHERS`, written
// by veh_diff.py from the recording's OBJ records); the move, gravity, damping, the contacts
// with the car's hull, the car's callback F-55998, the island's solve and the push are this
// port's own.

/// What a vehicle object is beyond a prop's `Obj`.
#[derive(Clone, Copy, Debug)]
pub struct VehObj {
    pub shape: &'static HullShape,
    /// an ambient car's composite's second part (a semi trailer's: the file's part matrices are all
    /// the identity)
    pub shape2: Option<&'static HullShape>,
    /// instance vtable[1]: 6 ambient traffic, 8 an mcCar
    pub class: u8,
    /// archetype +48
    pub gravity: f32,
    /// bound +80 as the running game has it
    pub cg: Vec3,
    /// its level index
    pub index: u32,
    /// it is in this tick's list as an active object
    pub here: bool,
    /// its mcCarDamage +20, the "damage inflicted" factor a car it hits takes its blow times (F-61340)
    pub inflict: f32,
    /// an mcCar's callback (F-55998) on a contact with the thrown rider (a partner of class 11), as this tick finds the
    /// car: (friction, elasticity, how far its own contact point goes down) when it writes them, None when it leaves
    /// the contact alone (an ambient car's callbacks never write: F-65962 / F-13089). See [`rider_touch`].
    pub rider_touch: Option<RiderTouch>,
    /// (the game's level step) the stepping car makes its pair with this vehicle (the lower car of two makes it, in
    /// its own step); false: a body of the level only (its pairs are another car's)
    pub pair: bool,
}

/// F-55998 (an mcCarInstance's contact callback), the part after its vehicle rules (A-31611 .. A-85558 .. A-95434),
/// for a contact whose partner is no vehicle (its type not 8 / 6): decoded 2026-11-04, the whole branch checked
/// offline 2026-11-12 (tools/car_terms_reference.py, docs/reference/vehicle/car_terms_check.md):
/// - the driver record's +24 (the 104 B table at [[car +80] +76] +76; index -1 none): 3 the player, else not;
/// - not the player: unless its mcCarGyro's halfword +18 (the two-wheel state, the dynamic cast's) is set or its
///   matrix's up.y (M +20; M = car +272 or [car +28] +16 by a thread byte) is at least 0x3f3504e6 (cos 45 degrees),
///   the contact takes friction = BoundFriction (sim tune +116) and elasticity = BoundElasticity (+120), and with
///   up.y below 0 the car's own world point (contact +48 when the car is A, +64 when B) goes down by HeadOnCGOffset
///   (+280) x |up.y| (fnmsubs); that ends it;
/// - otherwise chassis +592 bit 0x10 or 0x04 (wiped out, damaged out) gives BoundFriction / BoundElasticity;
/// - else only a fixed partner (its level state & 3 == 2) is touched: the normal as it pushes the car along the
///   matrix's row 2 above (float) cos(0.4363323152065277) and the speed +224 above 22.35 give BoundFriction /
///   BoundElasticity and the own point's y less HeadOnCGOffset (fsubs); failing that, the car stuck (mcCarStuck +56
///   == 2 with the chassis' +20 -> +20 set) gives friction 0.
/// Only the enabled contacts (+165 bit 0x80) are walked. `tune` = (bound friction, bound elasticity, head-on CG
/// offset); `along` the normal along row 2 ([`car_along`]); `fast` the speed above 22.35.
#[allow(clippy::too_many_arguments)]
pub fn car_terms(player: bool, two: bool, up_y: f32, wiped: bool, fixed: bool, along: f32, fast: bool, stuck: bool, tune: (f32, f32, f32)) -> Option<CarTerms> {
    let (friction, elasticity, cg_offset) = tune;
    if !player && !two && up_y < f32::from_bits(0x3f35_04e6) {
        let drop = if up_y < 0.0 { Drop::Tilted(cg_offset, up_y.abs()) } else { Drop::None };
        return Some(CarTerms { friction, elasticity: Some(elasticity), drop });
    }
    if wiped {
        return Some(CarTerms { friction, elasticity: Some(elasticity), drop: Drop::None });
    }
    if !fixed {
        return None;
    }
    if along > ex::guest_cos(f64::from_bits(0x3FDB_ECDE_6000_0000)) as f32 && fast {
        Some(CarTerms { friction, elasticity: Some(elasticity), drop: Drop::Head(cg_offset) })
    } else if stuck {
        Some(CarTerms { friction: 0.0, elasticity: None, drop: Drop::None })
    } else {
        None
    }
}

/// The normal `n` (as it pushes the car) along the matrix's row 2, as F-55998 forms it.
pub fn car_along(n: Vec3, row2: Vec3) -> f32 {
    fma(n.x, row2.x, fma(row2.y, n.y, row2.z * n.z))
}

/// What [`car_terms`] writes: the friction, the elasticity (None: left), how the car's own point goes down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarTerms {
    pub friction: f32,
    pub elasticity: Option<f32>,
    pub drop: Drop,
}

/// How [`car_terms`] moves the car's own world point's y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drop {
    None,
    /// y = fnmsubs(offset, |up.y|, y)
    Tilted(f32, f32),
    /// y = y - offset
    Head(f32),
}

impl Drop {
    pub fn apply(self, y: f32) -> f32 {
        match self {
            Drop::None => y,
            Drop::Tilted(k, a) => -k.mul_add(a, -y),
            Drop::Head(k) => y - k,
        }
    }
}

/// [`car_terms`] on a contact with the thrown rider (class 11: never fixed), as the rider's island takes it.
pub fn rider_touch(player: bool, two: bool, up_y: f32, wiped: bool, tune: (f32, f32, f32)) -> Option<RiderTouch> {
    car_terms(player, two, up_y, wiped, false, 0.0, false, false, tune).map(|t| (t.friction, t.elasticity.unwrap_or(tune.1), match t.drop {
        Drop::Tilted(k, a) => Some((k, a)),
        _ => None,
    }))
}

/// What [`rider_touch`] writes: friction, elasticity, and (offset, |up.y|) for the car's own point.
pub type RiderTouch = (f32, f32, Option<(f32, f32)>);

/// The car's own world point after [`rider_touch`]'s drop (fnmsubs: one rounding).
pub fn rider_touch_point(y: f32, drop: Option<(f32, f32)>) -> f32 {
    drop.map_or(y, |(k, a)| Drop::Tilted(k, a).apply(y))
}

#[cfg(test)]
mod car_terms_tests {
    use super::*;

    /// F-55998's materials part against the original run offline (tools/car_terms_reference.py, 6,000 contacts:
    /// partner types 11 / 3 / 1, level states 0..3, drivers player / AI / none, two-wheel, wiped, stuck, head-on).
    #[test]
    fn car_terms_match_the_original() {
        let text = include_str!("testdata/car_terms_ref.txt");
        let mut n = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = inp.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let o: Vec<u32> = out.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let fl = |i: usize| f32::from_bits(w[i]);
            let player = w[2] != 0xFFFF_FFFF && w[3] == 3;
            let two = w[4] as u16 != 0;
            let fixed = w[1] & 3 == 2;
            let wiped = w[10] & 0x14 != 0;
            let stuck = w[12] == 2 && w[13] != 0;
            let car_a = w[14] != 0;
            let n0 = Vec3::new(fl(15), fl(16), fl(17));
            let nrm = if car_a { n0 } else { -n0 };
            let along = car_along(nrm, Vec3::new(fl(7), fl(8), fl(9)));
            let fast = fl(11) > 22.35;
            let t = car_terms(player, two, fl(6), wiped, fixed, along, fast, stuck, (fl(19), fl(20), fl(21)));
            let (mut fr, mut el, mut ya, mut yb) = (0x1111_1111u32, 0x2222_2222u32, w[18], w[22]);
            if let Some(t) = t {
                fr = t.friction.to_bits();
                if let Some(e) = t.elasticity {
                    el = e.to_bits();
                }
                if car_a {
                    ya = t.drop.apply(fl(18)).to_bits();
                } else {
                    yb = t.drop.apply(fl(22)).to_bits();
                }
            }
            assert_eq!([fr, el, ya, yb], [o[0], o[1], o[2], o[3]], "case {n}: {inp}");
            n += 1;
        }
        assert_eq!(n, 6000);
    }

    /// F-55998's vehicle and prop parts against the original run offline (tools/car_vehicle_reference.py, 6,000
    /// contacts: partners of types 8 / 6 / 4 / 13, the car A or B, upright and tilted, head-on, from behind and the side,
    /// and the materials part after them). The points' fourth lanes (+60 / +76) are not the port's.
    #[test]
    fn car_vehicle_parts_match_the_original() {
        let text = include_str!("testdata/car_vehicle_ref.txt");
        let hull = HullShape::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z], &[[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]]);
        let mut n = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = inp.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let o: Vec<u32> = out.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let fl = |i: usize| f32::from_bits(w[i]);
            let v3 = |i: usize| Vec3::new(fl(i), fl(i + 1), fl(i + 2));
            let m4 = |i: usize| [v3(i), v3(i + 3), v3(i + 6), v3(i + 9)];
            let (ty, car_a) = (w[0], w[9] != 0);
            let driver = Driver { player: w[2] != 0xFFFF_FFFF && w[3] == 3, two: w[4] as u16 != 0, stuck: w[7] == 2 && w[8] != 0, cg_offset: fl(12) };
            let now = m4(13);
            let car = CarIn { driver, hull: &hull, hull_cg: Vec3::ZERO, hull_margin: 0.04, now, last: now, prev: now, speed: fl(6), bound: (Vec3::ZERO, Vec3::ZERO), wheels: Vec::new(), vel: v3(37), touch: (fl(10), fl(11)), wiped: w[5] & 0x14 != 0 };
            let om = m4(25);
            let mut t = Touch { friction: f32::from_bits(0x1111_1111), elasticity: f32::from_bits(0x2222_2222), normal: v3(40), back: Vec3::ZERO - v3(40), ..Default::default() };
            t.a.point = v3(43);
            t.b.point = v3(47);
            if ty == 8 || ty == 6 {
                car_meets_vehicle(&car, &om, ty as u8, car_a, &mut t);
            } else {
                car_meets_prop(&car, om[1].y, car_a, w[1] & 3 == 2, &mut t);
            }
            let got = [t.friction.to_bits(), t.elasticity.to_bits(), t.a.point.x.to_bits(), t.a.point.y.to_bits(), t.a.point.z.to_bits(), t.b.point.x.to_bits(), t.b.point.y.to_bits(), t.b.point.z.to_bits(), t.normal.x.to_bits(), t.normal.y.to_bits(), t.normal.z.to_bits(), t.back.x.to_bits(), t.back.y.to_bits(), t.back.z.to_bits()];
            let want = [o[0], o[1], o[2], o[3], o[4], o[6], o[7], o[8], o[10], o[11], o[12], o[13], o[14], o[15]];
            assert_eq!(got, want, "case {n} type {ty}: {inp}");
            n += 1;
        }
        assert_eq!(n, 6000);
    }
}

impl VehObj {
    /// its bound's part `p`
    pub fn part(&self, p: u16) -> &'static HullShape {
        if p == 1 { self.shape2.unwrap_or(self.shape) } else { self.shape }
    }

    pub fn parts(&self) -> u16 {
        if self.shape2.is_some() { 2 } else { 1 }
    }
}

#[derive(Clone, Debug)]
struct OtherVeh {
    index: u32,
    addr: u32,
    class: u8,
    state: u32,
    model: String,
    matrix: [Vec3; 4],
    body: PBody,
    gravity: f32,
    cg: Vec3,
}

/// A contact the pair car x vehicle kept from the tick before (one-step replays only).
#[derive(Clone, Copy, Debug)]
struct OtherKept {
    index: u32,
    local_a: Vec3,
    local_b: Vec3,
    normal: Vec3,
    back: Vec3,
    kept: Vec3,
    depth: f32,
    element: u16,
    element_b: u16,
}

pub struct Others {
    mine: u32,
    car_addr: u32,
    ticks: HashMap<usize, Vec<OtherVeh>>,
    kept: HashMap<usize, Vec<OtherKept>>,
    shapes: HashMap<String, HullShape>,
    /// the kept contacts are the recording's before every tick (a one-step replay)
    inject: bool,
}

static OTHERS: std::sync::OnceLock<Option<Others>> = std::sync::OnceLock::new();
/// The line of the replay's input the tick in hand is (replay.rs).
pub static REPLAY_TICK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

fn vehcon() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MCLA_DBG_VEHCON").is_some())
}

fn others_only() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MCLA_REPLAY_OTHERS_ONLY").is_some())
}

fn others() -> Option<&'static Others> {
    OTHERS.get().and_then(|o| o.as_ref())
}

/// `MCLA_REPLAY_OTHERS=<file>`: lines of words, numbers as the hexadecimal bits:
/// `H <car's level index> <car's instance address>`;
/// `V <tick> <level index> <instance address> <class number> <level state> <model> <12 instance matrix>
///    <3 place +256> <9 rows +208> <3 v> <3 w> <3 P> <3 L> <3 F +336> <3 T +352> <mass> <1/mass> <3 I> <3 1/I>
///    <max speed> <max spin> <gravity factor> <3 cg> <moves +664> <12 last matrix +96> <3 push +400> <3 turn +416>`;
/// `K <tick> <level index> <3 local A> <3 local B> <3 normal +80> <3 +96> <3 P +128> <depth> <element A> <element B>`.
/// `hull_of`: the model's bound from the game's files.
pub fn load_others(path: &str, inject: bool, hull_of: &dyn Fn(&str) -> Option<HullShape>) {
    let _ = OTHERS.get_or_init(|| {
        let text = std::fs::read_to_string(path).ok()?;
        let mut o = Others { mine: 0, car_addr: 0, ticks: HashMap::new(), kept: HashMap::new(), shapes: HashMap::new(), inject };
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let x = |i: usize| u32::from_str_radix(w.get(i).copied().unwrap_or("0"), 16).unwrap_or(0);
            let f = |i: usize| f32::from_bits(x(i));
            let v3 = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            match w.first().copied() {
                Some("H") => {
                    o.mine = x(1);
                    o.car_addr = x(2);
                }
                Some("V") if w.len() >= 82 => {
                    let model = w[6].to_string();
                    if !o.shapes.contains_key(&model) {
                        match hull_of(&model) {
                            Some(h) => {
                                o.shapes.insert(model.clone(), h);
                            }
                            None => {
                                println!("replay: no bound for the vehicle {model}");
                                continue;
                            }
                        }
                    }
                    let b = 7;
                    let matrix = [v3(b), v3(b + 3), v3(b + 6), v3(b + 9)];
                    let c = b + 12;
                    let body = PBody {
                        at: v3(c),
                        rows: [v3(c + 3), v3(c + 6), v3(c + 9)],
                        v: v3(c + 12),
                        w: v3(c + 15),
                        p: v3(c + 18),
                        l: v3(c + 21),
                        force: v3(c + 24),
                        torque: v3(c + 27),
                        jf: Vec3::ZERO,
                        jt: Vec3::ZERO,
                        push: v3(c + 57),
                        turn: v3(c + 60),
                        last: [v3(c + 45), v3(c + 48), v3(c + 51), v3(c + 54)],
                        prev: [v3(c + 45), v3(c + 48), v3(c + 51), v3(c + 54)],
                        last_set: true,
                        deep: false,
                        deep_count: 0,
                        mass: f(c + 30),
                        inv_mass: f(c + 31),
                        inertia: v3(c + 32),
                        inv_inertia: v3(c + 35),
                        max_speed: f(c + 38),
                        max_spin: f(c + 39),
                        moves: x(c + 44),
                    };
                    o.ticks.entry(x(1) as usize).or_default().push(OtherVeh { index: x(2), addr: x(3), class: x(4) as u8, state: x(5), model, matrix, body, gravity: f(c + 40), cg: v3(c + 41) });
                }
                Some("K") if w.len() >= 21 => {
                    o.kept.entry(x(1) as usize).or_default().push(OtherKept { index: x(2), local_a: v3(3), local_b: v3(6), normal: v3(9), back: v3(12), kept: v3(15), depth: f(18), element: x(19) as u16, element_b: x(20) as u16 });
                }
                _ => {}
            }
        }
        println!("replay: other vehicles in {} ticks, {} bounds{}", o.ticks.len(), o.shapes.len(), if inject { ", kept contacts injected" } else { "" });
        Some(o)
    });
}

/// F-66303's constants on a traffic vehicle's archetype (F-58444: 0x82002C60 = 0.01, 0x820022B4 = 0.001).
const VEH_DAMPING: [[f32; 3]; 6] = [[0.01; 3], [0.01; 3], [0.001; 3], [0.01; 3], [0.01; 3], [0.001; 3]];

/// F-55998's flattening of the normal `n` (as it pushes the car) by how upright the partner (`other_up_y`, its
/// matrix's up.y) and the car stand: k = 1 - |a b|; below 1 the normal's y is taken k times and x, z scaled to keep
/// the length (when 1 - y^2 > 0); a result of squared length in (0x3f670a3d, 0x3f8d1eb7) is made unit as the
/// callback does and written (Some). The first answer is the normal the vehicle part goes on with: the unit one when
/// written, the scaled one when the length test fails, `n` when k is not below 1.
pub fn lay_flat(n: Vec3, other_up_y: f32, car_up_y: f32) -> (Vec3, Option<Vec3>) {
    let k = 1.0 - (other_up_y * car_up_y).abs();
    if k >= 1.0 {
        return (n, None);
    }
    let mut n = n;
    let mut y2 = n.y * n.y;
    let rest = 1.0 - y2;
    if rest > 0.0 {
        n.y = k * n.y;
        y2 = n.y * n.y;
        let q = ((1.0 - y2) / rest).sqrt();
        n.x *= q;
        n.z *= q;
    }
    let l2 = fma(n.z, n.z, n.x * n.x) + y2;
    if l2 > f32::from_bits(0x3f67_0a3d) && l2 < f32::from_bits(0x3f8d_1eb7) {
        let u = unit_cb(n);
        (u, Some(u))
    } else {
        (n, None)
    }
}

/// A vector made unit as the callback does (vmsum3fp, vrsqrtefp, one step).
fn unit_cb(n: Vec3) -> Vec3 {
    let d = dot3(n, n);
    let e = 1.0 / d.sqrt();
    let e = if e == f32::INFINITY { 0.0 } else { e };
    n * ((1.0 - d * (e * e)) * (0.5 * e) + e)
}

/// The car's callback F-55998 on a contact whose other instance is a vehicle (class 6 or 8):
/// the normal laid flat by how upright both stand; then by where the car is on the other and
/// how it travels - in front of it and head-on (BoundFriction / BoundElasticity, each side's
/// normal the other's axis, both points onto the two centre planes), behind it and the same
/// way (normals likewise, both points the car's on its centre plane), or from the side and
/// into it (the car's point onto its centre plane) - and, the car's own point rewritten on side
/// A, the contact's limit (+60, the point's fourth lane) becomes 0: the island goes through the
/// breaking pass, where nothing breaks. `om`: the other instance's matrix.
fn car_meets_vehicle(car: &CarIn, om: &[Vec3; 4], class: u8, car_is_a: bool, t: &mut Touch) {
    let mut n = if car_is_a { t.normal } else { Vec3::ZERO - t.normal };
    let store = |t: &mut Touch, mine: Vec3, theirs: Vec3| {
        if car_is_a {
            t.normal = mine;
            t.back = theirs;
        } else {
            t.back = mine;
            t.normal = theirs;
        }
    };
    let (flat, unit) = lay_flat(n, om[1].y, car.now[1].y);
    n = flat;
    if let Some(u) = unit {
        store(t, u, Vec3::ZERO - u);
    }
    let (me, back) = (car.now, car.now[2]);
    let fwd = Vec3::ZERO - back;
    let dir = if fma(car.vel.x, fwd.x, fma(car.vel.z, fwd.z, car.vel.y * fwd.y)) > 0.0 { unit_cb(fwd + car.vel) } else { fwd };
    let ob = om[2];
    let along = |a: Vec3| fma(a.x, ob.x, fma(ob.z, a.z, ob.y * a.y));
    let onto = |m: &[Vec3; 4], p: Vec3| {
        let d = p - m[3];
        let s = -fma(m[0].x, d.x, fma(m[0].y, d.y, m[0].z * d.z));
        m[0] * s + p
    };
    let c45 = ex::guest_cos(f64::from_bits(0x3FE9_21FB_6000_0000)) as f32;
    let side_of = along(n);
    let (mine, theirs) = if car_is_a { (t.a.point, t.b.point) } else { (t.b.point, t.a.point) };
    let (mut mine, mut theirs) = (mine, theirs);
    if side_of < f32::from_bits(0xbf33_3333) {
        if along(dir) > c45 {
            t.friction = car.touch.0;
            t.elasticity = car.touch.1;
            store(t, unit_cb(Vec3::new(-ob.x, n.y, -ob.z)), unit_cb(Vec3::new(-back.x, -n.y, -back.z)));
            mine = onto(om, theirs);
            theirs = onto(&me, mine);
        }
    } else if side_of > f32::from_bits(0x3f33_3333) {
        if along(dir) < -c45 {
            store(t, unit_cb(Vec3::new(ob.x, n.y, ob.z)), unit_cb(Vec3::new(-back.x, -n.y, -back.z)));
            mine = onto(&me, mine);
            theirs = mine;
        }
    } else if fma(dir.x, n.x, fma(dir.z, n.z, dir.y * n.y)) < -c45 {
        mine = onto(&me, mine);
    }
    if car_is_a {
        t.a.point = mine;
        t.b.point = theirs;
    } else {
        t.b.point = mine;
        t.a.point = theirs;
    }
    // (A-31611: an mcCar partner, class 8, takes BoundFriction / BoundElasticity; any other the driver branch,
    // `car_terms`: a vehicle is never fixed - level state 0 or 1)
    if class == 8 {
        t.friction = car.touch.0;
        t.elasticity = car.touch.1;
    } else {
        car_terms_loose(car, car_is_a, false, t);
    }
}

/// The car's callback F-55998 on a contact with a prop (an mcPropInst, type 4 / 13): the normal as it pushes the car,
/// pointing down, laid flat by how upright both stand ([`lay_flat`], `other_up_y` the prop matrix's up.y); then its
/// materials part ([`car_terms_loose`]).
fn car_meets_prop(car: &CarIn, other_up_y: f32, car_is_a: bool, fixed: bool, t: &mut Touch) {
    let n = if car_is_a { t.normal } else { Vec3::ZERO - t.normal };
    if n.y < 0.0 {
        if let (_, Some(unit)) = lay_flat(n, other_up_y, car.now[1].y) {
            let back = Vec3::ZERO - unit;
            if car_is_a {
                t.normal = unit;
                t.back = back;
            } else {
                t.back = unit;
                t.normal = back;
            }
        }
    }
    car_terms_loose(car, car_is_a, fixed, t);
}

/// The corner of a box bound the push solve measures a torn bumper's thickness from (type 3: per axis the half size less
/// the margin, signed by the direction in the box's frame; the box is about its origin). None for anything else.
fn piece_core(world: &LooseWorld, obj: &Obj, rows: &[Vec3; 3], n: Vec3) -> Option<Vec3> {
    let t = world.types.get(obj.ty)?;
    if obj.veh.is_some() || t.off != usize::MAX {
        return None;
    }
    let b = t.parts.first()?.bound.as_ref()?;
    let h = (v(b.max) - v(b.min)) * 0.5 - Vec3::splat(b.margin);
    let nl = Vec3::new(dot3(rows[0], n), dot3(rows[1], n), dot3(rows[2], n));
    let s = |d: f32, x: f32| if d >= 0.0 { x } else { -x };
    Some(Vec3::new(s(nl.x, h.x), s(nl.y, h.y), s(nl.z, h.z)))
}

/// The corner of a polyhedron bound the push solve measures a side's thickness from
/// (F-49764, type 4: the last scanned corner unless an earlier one lies farther along).
fn veh_core(shape: &HullShape, rows: &[Vec3; 3], n: Vec3) -> Vec3 {
    let nl = Vec3::new(dot3(rows[0], n), dot3(rows[1], n), dot3(rows[2], n));
    let c = shape.scanned();
    let Some((&last, firsts)) = c.split_last() else { return Vec3::ZERO };
    let (mut best, mut bd) = (last, dot3(nl, last));
    for p in firsts {
        let d = dot3(nl, *p);
        if d > bd {
            best = *p;
            bd = d;
        }
    }
    best
}

/// A car of the session as another car's step meets it (the game; a replay takes them from its file).
pub struct LiveVeh {
    /// the car's own number (its place among the session's cars)
    pub id: u32,
    pub shape: &'static HullShape,
    /// 8 a car with a driver (mcCar), 6 ambient traffic
    pub class: u8,
    pub gravity: f32,
    /// its body's place in its own frame (the model offset): the instance matrix's origin from the body
    pub cg: Vec3,
    /// its place among the level's indices against the stepping car's (0)
    pub level: i64,
    /// its body moved as the tick's pre-collide moves a body (F-10928), its last matrix, and its forces
    pub body: PBody,
    pub inflict: f32,
    /// parked: no body - immovable to the solve
    pub parked: bool,
    /// its callback on a contact with the thrown rider ([`rider_touch`])
    pub rider_touch: Option<RiderTouch>,
    /// the stepping car makes its pair with it ([`VehObj::pair`])
    pub pair: bool,
}

/// A car hull shape kept for the level's whole life (the vehicle objects hold a plain reference).
pub fn static_shape(a: &Arc<HullShape>) -> &'static HullShape {
    use std::sync::Mutex;
    static KEPT: std::sync::OnceLock<Mutex<HashMap<usize, &'static HullShape>>> = std::sync::OnceLock::new();
    let mut m = KEPT.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    *m.entry(Arc::as_ptr(a) as usize).or_insert_with(|| Box::leak(Box::new((**a).clone())))
}

/// What belongs to one car in a shared level: its manifolds (with the city, props, pieces, the other cars)
/// and its pair with the city's place.
#[derive(Clone, Debug, Default)]
pub struct CarPart {
    manis: Vec<Mani>,
    car_city_order: Option<usize>,
}

impl PBody {
    /// A car's body for another car's island: `at` its place (+256), `rows`, velocities, mass and moments,
    /// the matrix the last collide had and the one it has now, the tick's force and torque so far.
    #[allow(clippy::too_many_arguments)]
    pub fn of_car(at: Vec3, rows: [Vec3; 3], v: Vec3, w: Vec3, mass: f32, inertia: Vec3, last: [Vec3; 4], now: [Vec3; 4], force: Vec3, torque: Vec3) -> PBody {
        let mut b = PBody {
            at,
            rows,
            v,
            w,
            p: v * mass,
            l: Vec3::ZERO,
            force,
            torque,
            jf: Vec3::ZERO,
            jt: Vec3::ZERO,
            push: Vec3::ZERO,
            turn: Vec3::ZERO,
            last,
            prev: now,
            last_set: true,
            deep: false,
            deep_count: 0,
            mass,
            inv_mass: recip(mass),
            inertia,
            inv_inertia: Vec3::new(recip(inertia.x), recip(inertia.y), recip(inertia.z)),
            max_speed: 134.1,
            max_spin: 12.566_371,
            moves: 0,
        };
        b.l = b.momentum_of(w);
        b
    }
}

/// F-61340: what mcCarDamage multiplies a blow by - against a vehicle (`inflict`: the other's mcCarDamage +20) the car's own
/// +16 (`blow[1]`) times it, else its +12 (`blow[0]`).
pub(crate) fn blow_for(inflict: Option<f32>, blow: [f32; 2]) -> f32 {
    inflict.map_or(blow[0], |f| f * blow[1])
}

impl LooseState {
    /// Two objects' names swapped in every manifold (the island of another car solved in this car's step, that car the
    /// island's `Car`: vehsim.rs).
    pub fn swap_who(&mut self, a: Who, b: Who) {
        for m in self.manis.iter_mut() {
            for w in [&mut m.a, &mut m.b] {
                if *w == a {
                    *w = b;
                } else if *w == b {
                    *w = a;
                }
            }
        }
    }

    /// The stepping car's own part out of the level (before another car steps in it).
    pub fn take_car(&mut self) -> CarPart {
        let (mine, rest): (Vec<Mani>, Vec<Mani>) = std::mem::take(&mut self.manis).into_iter().partition(|m| m.a == Who::Car || m.b == Who::Car);
        self.manis = rest;
        CarPart { manis: mine, car_city_order: self.car_city_order.take() }
    }

    /// A car's own part into the level, for its step.
    pub fn put_car(&mut self, part: CarPart) {
        self.manis.extend(part.manis);
        self.car_city_order = part.car_city_order;
    }

    /// The other cars for this car's step: each an object with a body (none when parked); those not
    /// listed are not there, and the manifolds with them go.
    pub fn set_vehicles(&mut self, list: Vec<LiveVeh>) {
        self.live = true;
        for o in self.objs.iter_mut() {
            if o.amb.is_some() {
                continue;
            }
            if let Some(vh) = o.veh.as_mut() {
                vh.here = false;
                o.body = None;
            }
        }
        for v in list {
            let i = match self.objs.iter().position(|o| o.amb.is_none() && o.veh.map_or(false, |x| x.index == v.id)) {
                Some(i) => i,
                None => {
                    self.objs.push(Obj { ty: usize::MAX, inst: None, matrix: v.body.prev, entry: None, hit: false, body: None, level: 0, addr: 0, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
                    self.objs.len() - 1
                }
            };
            let obj = &mut self.objs[i];
            obj.level = v.level;
            obj.addr = v.level;
            obj.matrix = [v.body.rows[0], v.body.rows[1], v.body.rows[2], ex::origin_of(v.body.at, &v.body.rows, v.cg)];
            obj.body = if v.parked { None } else { Some(v.body) };
            obj.veh = Some(VehObj { shape: v.shape, shape2: None, class: v.class, gravity: v.gravity, cg: v.cg, index: v.id, here: true, inflict: v.inflict, rider_touch: v.rider_touch, pair: v.pair });
        }
        let objs = &self.objs;
        self.manis.retain(|m| ![m.a, m.b].iter().any(|w| matches!(w, Who::Obj(o) if objs[*o].veh.map_or(false, |x| !x.here))));
    }

    /// What this step's solves gave each other car: its id, the impulse and its turn about its place (for
    /// the momenta), and the push solve's move and turn. Taken: the bodies are handed in afresh each step.
    pub fn vehicle_answers(&mut self) -> Vec<(u32, Vec3, Vec3, Vec3, Vec3)> {
        let mut out = Vec::new();
        for o in self.objs.iter_mut() {
            if o.amb.is_some() {
                continue;
            }
            let (Some(vh), Some(b)) = (o.veh, o.body.as_mut()) else { continue };
            if b.jf != Vec3::ZERO || b.jt != Vec3::ZERO || b.push != Vec3::ZERO || b.turn != Vec3::ZERO {
                out.push((vh.index, b.jf, b.jt, b.push, b.turn));
            }
            b.jf = Vec3::ZERO;
            b.jt = Vec3::ZERO;
            b.push = Vec3::ZERO;
            b.turn = Vec3::ZERO;
        }
        out
    }

    /// What mcCarDamage multiplies the car's blow from `other` by (F-61340): against a vehicle the car's own +16
    /// (`blow[1]`) times the vehicle's "damage inflicted" factor (its mcCarDamage +20), against anything else the
    /// car's own +12 (`blow[0]`; `CarState::blow`).
    pub fn vehicle_factor(&self, other: Option<Who>, blow: [f32; 2]) -> f32 {
        let vehicle = match other {
            Some(Who::Obj(o)) => self.objs.get(o).and_then(|o| o.veh).map(|v| v.inflict),
            _ => None,
        };
        blow_for(vehicle, blow)
    }

    /// The other vehicles of the tick in hand, as the replay's file has them before it: each an
    /// object with the logged body; those not in the list are gone, and their contacts with them.
    fn sync_others(&mut self) {
        let Some(ot) = others() else { return };
        let tick = REPLAY_TICK.load(std::sync::atomic::Ordering::Relaxed);
        for o in self.objs.iter_mut() {
            if let Some(vh) = o.veh.as_mut() {
                vh.here = false;
                o.body = None;
            }
        }
        for v in ot.ticks.get(&tick).into_iter().flatten() {
            // (an inactive one is activated by its driver's code with a state no record holds: left out)
            if v.state != 0 {
                continue;
            }
            let Some(shape) = ot.shapes.get(&v.model) else { continue };
            let i = match self.objs.iter().position(|o| o.veh.map_or(false, |x| x.index == v.index)) {
                Some(i) => i,
                None => {
                    self.objs.push(Obj { ty: usize::MAX, inst: None, matrix: v.matrix, entry: None, hit: false, body: None, level: 0, addr: 0, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
                    self.objs.len() - 1
                }
            };
            // (its place among the level's indices and the heap's addresses, against the car's)
            let (car_level, car_addr) = self.car_place();
            let obj = &mut self.objs[i];
            obj.level = v.index as i64 - ot.mine as i64 + car_level;
            obj.addr = v.addr as i64 - ot.car_addr as i64 + car_addr;
            obj.matrix = v.matrix;
            obj.body = Some(v.body);
            obj.veh = Some(VehObj { shape, shape2: None, class: v.class, gravity: v.gravity, cg: v.cg, index: v.index, here: true, inflict: 1.0, rider_touch: None, pair: true });
        }
        let gone = |w: Who, objs: &Vec<Obj>| matches!(w, Who::Obj(o) if objs[o].veh.map_or(false, |x| !x.here || ot.inject));
        let objs = &self.objs;
        self.manis.retain(|m| !gone(m.a, objs) && !gone(m.b, objs));
        if ot.inject {
            for k in ot.kept.get(&tick).into_iter().flatten() {
                let Some(o) = self.objs.iter().position(|o| o.veh.map_or(false, |x| x.index == k.index && x.here)) else { continue };
                let vh = self.objs[o].veh.unwrap();
                let side = |local: Vec3| Side { body: None, point: Vec3::ZERO, local, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: 0.04, core: Vec3::ZERO };
                let t = Touch { depth: k.depth, a: side(k.local_a), b: side(k.local_b), normal: k.normal, back: k.back, kept: k.kept, ..Default::default() };
                let slot = Slot2 { t, age: 1, enabled: true, element: k.element, surface: vh.shape.surface_of(k.element_b), surface_a: Surface::Prop };
                match self.manis.iter().position(|m| m.a == Who::Car && m.b == Who::Obj(o)) {
                    Some(i) => self.manis[i].slots.push(slot),
                    None => self.manis.push(Mani { a: Who::Car, b: Who::Obj(o), part_a: 0, part_b: 0, cell: 0, stale: false, visited: true, island: 0, order: 0, slots: vec![slot], rope: false, owner: 0 }),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The ambient (traffic) cars in the level (PEOPLE_TRAFFIC.md "Hitting riding traffic", "Physical
// traffic"). Each is an mcAmbientCar whose instance (car +112) is in the level from its spawn:
// inactive while it rides (level state 1; its matrix moved each frame, F-86858, and its
// velocity the place's change over the frame, inst +96, F-53150); activated by any contact
// (F-27744 -> F-24099 -> slot 28 F-41833 -> vfunc 18 F-15252 -> F-35377); then its
// mcAmbientCollider (car +2688) is the base phCollider with its own wheels (slot 14 F-37370 ->
// F-19974) until the rest rule (F-72067) takes it out again. A semi's cab and trailer
// are joined by three constraints while they are active (F-15252, F-82928).

/// An ambient type's physics: `mcAmbientTypePhysicsTuning.xml` (mass, inertia box, gravity factor),
/// `mcAmbientWheelTuning.xml` (F-82909's derived numbers), the `.r155` bound and the skeleton's
/// wheel bones.
#[derive(Clone, Debug)]
pub struct AmbTune {
    /// the bound: one polyhedron, or a composite's two (a semi trailer's)
    pub shape: &'static HullShape,
    pub shape2: Option<&'static HullShape>,
    /// fMass x fMassScale; mass / 12 x (y^2 + z^2, x^2 + z^2, x^2 + y^2) of vInertialBox x
    /// fInertialBoxScale (F-30573), each at least a twentieth of the largest (F-88418)
    pub mass: f32,
    pub inertia: Vec3,
    /// fGravityFactor (archetype +48)
    pub gravity: f32,
    /// bound +80 as the game sets it: (0, fCogOffset x centroid.y, 0)
    pub cg: Vec3,
    /// type +240: fCogSim x centroid.y
    pub cog_sim: f32,
    /// type +244 .. +256 (F-82909, Wq = mass x gravity factor x 9.8 / 4): spring Ts Wq / travel,
    /// its damping 2 Sd sqrt(Wq k), the tyre's 2 Wq Tt / St and 2 Td sqrt(Wq kt)
    pub k: f32,
    pub c: f32,
    pub kt: f32,
    pub ct: f32,
    /// fSuspensionTravel (+216), fTireStretch (+228), fTireGrip (+204)
    pub travel: f32,
    pub stretch: f32,
    pub grip: f32,
    /// the wheels (`iWheelCount`; the `wheel_i` bones, F-19769; a trailer's 2 and 3 are its
    /// bones 0 and 1 with z turned)
    pub wheels: Vec<Vec3>,
    /// the hitch (F-45572): a cab's or a trailer's
    pub hitch: Option<Vec3>,
}

/// A wheel's record (collider +676, 24 bytes): the spring's compression (+8), the tyre's stretch
/// (+12), the stretch as it was when it slid (+16) and the slide flag (+20).
#[derive(Clone, Copy, Debug, Default)]
pub struct AmbWheel {
    pub d: f32,
    pub s: f32,
    pub raw: f32,
    pub slide: bool,
}

/// An ambient car in the level.
#[derive(Clone, Debug)]
pub struct Amb {
    /// the session's car (vehicles.rs `uid`)
    pub uid: u32,
    pub tune: Arc<AmbTune>,
    pub wheels: [AmbWheel; 6],
    /// collider +864: the rest rule's count
    pub count: u32,
    /// inst +96: the place's change over the frame
    pub moved: Vec3,
    /// the collider's turn (+288), kept while inactive: zero from Init (F-50121)
    pub turn: Vec3,
    /// the semi's other car (+3604)
    pub partner: Option<u32>,
    /// made active / put to rest since `ambient_events` last looked
    pub woke: bool,
    pub slept: bool,
    last_at: Option<Vec3>,
    /// level byte bit 8 for this update's collide: the instance has +140 bit 0x80 (a race path's
    /// bPhysical, path +170 -> F-92604 -> F-55165) and its matrix was moved this frame
    /// while inactive (F-86858 -> F-77745: the simulator's list, at most 120); the list
    /// is emptied after the collide (F-96197 -> F-79250)
    pub posed: bool,
}

/// A vector float as the vector unit takes and gives it in flush mode: a denormal is a zero of its sign.
pub(super) fn flush(x: f32) -> f32 {
    if x.is_subnormal() { 0.0f32.copysign(x) } else { x }
}

/// The body's SetVelocity F-64271 (vfunc 41 of the body class): +272 = v, +304 = v x the mass +192 (vmulfp,
/// flush mode).
fn set_velocity(b: &mut PBody, v: Vec3) {
    let m = flush(b.mass);
    b.v = v;
    b.p = Vec3::new(flush(flush(v.x) * m), flush(flush(v.y) * m), flush(flush(v.z) * m));
}

/// The body's ApplyImpulse F-50175 (vfunc 35): +368 += J (vaddfp, flush mode).
fn apply_impulse(b: &mut PBody, j: Vec3) {
    let f = |a: f32, b: f32| flush(flush(a) + flush(b));
    b.jf = Vec3::new(f(b.jf.x, j.x), f(b.jf.y, j.y), f(b.jf.z, j.z));
}

// The vector unit in flush mode on whole vectors (x, y, z), and the two lane rotations its cross products use.
pub(super) fn vadd(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(flush(flush(a.x) + flush(b.x)), flush(flush(a.y) + flush(b.y)), flush(flush(a.z) + flush(b.z)))
}
pub(super) fn vsub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(flush(flush(a.x) - flush(b.x)), flush(flush(a.y) - flush(b.y)), flush(flush(a.z) - flush(b.z)))
}
pub(super) fn vmul(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(flush(flush(a.x) * flush(b.x)), flush(flush(a.y) * flush(b.y)), flush(flush(a.z) * flush(b.z)))
}
pub(super) fn vyzx(v: Vec3) -> Vec3 {
    Vec3::new(v.y, v.z, v.x)
}
pub(super) fn vzxy(v: Vec3) -> Vec3 {
    Vec3::new(v.z, v.x, v.y)
}

/// The cross product of the vector unit in flush mode as F-97724 and F-74100 form it: -(zxy(a) yzx(b) - yzx(a) zxy(b)),
/// each product rounded (vmulfp), then the difference (vnmsubfp) negated.
fn vcross(a: Vec3, b: Vec3) -> Vec3 {
    let m = vmul(vyzx(a), vzxy(b));
    -vsub(vmul(vzxy(a), vyzx(b)), m)
}

/// The impulse at a point F-97724 up to the body's setters: P + J (vaddfp; handed to the momentum setter, vfunc 42) and
/// the angular momentum L + (point - place) x J (written to +320 before the turn is re-derived, vfunc 46).
pub(super) fn impulse_sums(p: Vec3, l: Vec3, at: Vec3, vec: Vec3, point: Vec3) -> (Vec3, Vec3) {
    (vadd(p, vec), vadd(l, vcross(vsub(point, at), vec)))
}

/// The velocity F-74100 hands to the body of a piece broken off a moving parent (vfunc 41): the parent's velocity plus the
/// parent's turn crossed with the arm from the parent's centre to the piece's place (the turn crossed with the arm is
/// the negated cross product of the arm and the turn, as the vector unit forms it).
pub(super) fn piece_velocity(place: Vec3, centre: Vec3, v: Vec3, w: Vec3) -> Vec3 {
    vadd(vcross(w, vsub(place, centre)), v)
}

/// `xf` in flush mode (vmsum4fp: every product and sum taken with a denormal as a zero of its sign).
pub(super) fn vxf(m: &[Vec3; 4], p: Vec3) -> Vec3 {
    let f = |a: f32, b: f32| flush(flush(a) * flush(b));
    let s = |a: f32, b: f32| flush(flush(a) + flush(b));
    let k = |c: usize| s(s(m[3][c], f(m[2][c], p.z)), s(f(m[1][c], p.y), f(m[0][c], p.x)));
    Vec3::new(k(0), k(1), k(2))
}

/// The vector unit's sum of three products in flush mode, the fourth lane masked: (0 + z z') + (y y' + x x').
pub(super) fn vdot3(a: Vec3, b: Vec3) -> f32 {
    let m = vmul(a, b);
    flush(flush(flush(0.0 + m.z) + flush(flush(m.y) + flush(m.x))))
}

/// The refresh of one kept contact F-33100 (age != 0): A's world point from A's frame, B's from B's (None: the city, which
/// stands still: its local point is already in the world, a denormal one a zero as the vector unit takes it), and the depth along the back normal (a sum of three products
/// of the vector unit, the fourth lane masked).
pub(super) fn refresh_points(ma: &[Vec3; 4], mb: Option<&[Vec3; 4]>, la: Vec3, lb: Vec3, back: Vec3) -> (Vec3, Vec3, f32) {
    let a = vxf(ma, la);
    let b = mb.map_or(Vec3::new(flush(lb.x), flush(lb.y), flush(lb.z)), |m| vxf(m, lb));
    (a, b, vdot3(vsub(a, b), back))
}

/// A semi's hitch in the world as F-15252 forms it: the hitch with w = 1 through the instance's matrix (rows, place),
/// a vmsum4fp a lane.
pub(super) fn hitch_world(m: &[Vec3; 4], h: Vec3) -> Vec3 {
    use super::artic::V4;
    let hw = V4([h.x, h.y, h.z, 1.0]);
    let col = |k: usize| V4([m[0][k], m[1][k], m[2][k], m[3][k]]);
    Vec3::new(hw.dot4(col(0)).x(), hw.dot4(col(1)).x(), hw.dot4(col(2)).x())
}

/// The second and third pair's point: the hitch + 10 x a row of the matrix (vmulfp, then vaddfp).
pub(super) fn hitch_along(p: Vec3, row: Vec3) -> Vec3 {
    use super::artic::V4;
    let v = |a: Vec3| V4([a.x, a.y, a.z, 0.0]);
    let r = v(row).mul(V4::splat(10.0)).add(v(p)).0;
    Vec3::new(r[0], r[1], r[2])
}

/// A world point in an instance's frame, F-21064: the point less the place, into the rows (vmsum3fp); with the
/// bound a composite, then the same through the part's matrix.
pub(super) fn rope_local(m: &[Vec3; 4], p: Vec3, part: Option<&[Vec3; 4]>) -> Vec3 {
    use super::artic::V4;
    let v = |a: Vec3| V4([a.x, a.y, a.z, 0.0]);
    let into = |m: &[Vec3; 4], p: V4| {
        let d = p.sub(v(m[3]));
        V4([v(m[0]).dot3(d).x(), v(m[1]).dot3(d).x(), v(m[2]).dot3(d).x(), 0.0])
    };
    let mut l = into(m, v(p));
    if let Some(pm) = part {
        l = into(pm, l);
    }
    Vec3::new(l.0[0], l.0[1], l.0[2])
}

/// F-83853's wake test for an inactive side: the two world points farther apart than the
/// length + 0.01 (squared, the sum in vmsum3fp's order; NaN is not far).
fn rope_far(pa: Vec3, pb: Vec3, len: f32) -> bool {
    let d = pa - pb;
    let lim = len + 0.01;
    dot3(d, d) > lim * lim
}

/// F-83853's contact on the max-distance path: none for a slack rope (err < 0); else the kind
/// (1 for a length below 0.01, else 2), the depth min(err, +164) by `err > +164` with the allowed
/// depth on top (+320 = 0) and the excess past +164.
fn rope_depth(err: f32, len: f32) -> Option<(u8, f32, f32)> {
    if err < 0.0 {
        return None;
    }
    let (depth, excess) = if err > ROPE_DEPTH { (ROPE_DEPTH, err - ROPE_DEPTH) } else { (err, 0.0) };
    Some((if len < 0.01 { 1 } else { 2 }, depth + solver::ALLOW, excess))
}

/// F-64595, the excess split between the two sides' direct pushes (collider slot 27): with both,
/// by the squared inverse masses (the larger's share 1 / (sqrt(small / large) + 1), the other the
/// rest, equal halves), A along the normal and B against it; with one side only, all of it.
fn rope_push(ia: Option<f32>, ib: Option<f32>, n: Vec3, e: f32) -> [Option<Vec3>; 2] {
    match (ia, ib) {
        (Some(ia), Some(ib)) => {
            let (sa, sb) = (ia * ia, ib * ib);
            let (wa, wb) = if sa > sb {
                let wa = 1.0 / ((sb / sa).sqrt() + 1.0);
                (wa, 1.0 - wa)
            } else if sb > sa {
                let wb = 1.0 / ((sa / sb).sqrt() + 1.0);
                (1.0 - wb, wb)
            } else {
                (0.5, 0.5)
            };
            [Some(n * (wa * e)), Some(n * (-(wb * e)))]
        }
        (Some(_), None) => [Some(n * e), None],
        (None, Some(_)) => [None, Some(n * (-e))],
        (None, None) => [None, None],
    }
}

/// A constraint between two ambient cars (rage::phConstraint, max distance: F-44049): the
/// owner (the car that was activated, +3592 .. +3600), its two cars, a point in each one's frame
/// (+80 / +96) and the length (+160); +164 = 0.6 (F-31401), +320 = 0, +328 = -1 (never
/// breaks), flags = 2 (F-95940).
#[derive(Clone, Copy, Debug)]
pub struct Rope {
    pub owner: u32,
    pub a: u32,
    pub b: u32,
    pub la: Vec3,
    pub lb: Vec3,
    pub len: f32,
}

/// (diagnostics, `MCLA_DBG_ROPE`: the constraints and the ambient cars' activations)
fn rope_dbg() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MCLA_DBG_ROPE").is_some())
}

/// The first constraint's length: global 0x82909F6C, in the image's zero-filled tail and written by
/// nothing - 0 (a point constraint, kind 1).
const ROPE_POINT: f32 = 0.0;
/// +164: how deep one constraint's contact stands at most; the rest moves the bodies (F-64595)
const ROPE_DEPTH: f32 = 0.6;

impl LooseState {
    fn amb_obj(&self, uid: u32) -> Option<usize> {
        self.objs.iter().position(|o| o.amb.as_ref().is_some_and(|a| a.uid == uid))
    }

    /// A riding (or otherwise kinematic) ambient car's instance where the game puts it this frame:
    /// into the level inactive at its first place; its velocity the place's change (inst +96).
    pub fn ambient_place(&mut self, uid: u32, tune: &Arc<AmbTune>, matrix: [Vec3; 4], partner: Option<u32>, inflict: f32, dt: f32) {
        let o = match self.amb_obj(uid) {
            Some(o) => o,
            None => {
                let veh = VehObj { shape: tune.shape, shape2: tune.shape2, class: 6, gravity: tune.gravity, cg: tune.cg, index: uid, here: true, inflict, rider_touch: None, pair: true };
                let amb = Amb { uid, tune: tune.clone(), wheels: [AmbWheel::default(); 6], count: 0, moved: Vec3::ZERO, turn: Vec3::ZERO, partner, woke: false, slept: false, last_at: None, posed: false };
                // (its level index: above the stepping cars' 0 - the car is A against it)
                let level = 1_000_000 + uid as i64;
                let obj = Obj { ty: usize::MAX, inst: None, matrix, entry: None, hit: false, body: None, level, addr: level, veh: Some(veh), sleep: Sleep::reset(), dormant: false, amb: Some(amb) };
                // (a slot an ambient car left)
                match self.objs.iter().position(|o| o.amb.is_none() && o.veh.is_some_and(|v| v.index == u32::MAX)) {
                    Some(i) => {
                        self.objs[i] = obj;
                        i
                    }
                    None => {
                        self.objs.push(obj);
                        self.objs.len() - 1
                    }
                }
            }
        };
        if self.objs[o].body.is_some() {
            return;
        }
        self.objs[o].matrix = matrix;
        let a = self.objs[o].amb.as_mut().unwrap();
        a.partner = partner;
        a.moved = match a.last_at {
            Some(p) if dt > 0.0 => (matrix[3] - p) / dt,
            _ => Vec3::ZERO,
        };
        a.last_at = Some(matrix[3]);
    }

    /// A car whose instance has +140 bit 0x80 had its matrix moved this frame (F-86858, after
    /// `ambient_place`): while it is inactive (level state 1) F-77745 puts it on the
    /// simulator's list (at most 120) and sets its level byte bit 8 until the collide; the
    /// collision pass (F-27744) then takes a pair of two inactive objects when either one has
    /// bit 8 - such a car meets riding / parked cars and knockable props while it is kinematic, and
    /// a contact activates both inactive sides (A-28475 -> F-24099).
    pub fn ambient_posed(&mut self, uid: u32) {
        let Some(o) = self.amb_obj(uid) else { return };
        if self.objs[o].body.is_some() {
            return;
        }
        let listed = self.objs.iter().filter(|x| x.amb.as_ref().is_some_and(|a| a.posed)).count();
        if listed < 120 {
            self.objs[o].amb.as_mut().unwrap().posed = true;
        }
    }

    /// The ambient cars the session no longer has go from the level, with their pairs and constraints.
    pub fn ambient_keep(&mut self, alive: &dyn Fn(u32) -> bool) {
        let gone: Vec<usize> = (0..self.objs.len()).filter(|&o| self.objs[o].amb.as_ref().is_some_and(|a| !alive(a.uid))).collect();
        if gone.is_empty() {
            return;
        }
        for &o in &gone {
            let uid = self.objs[o].amb.as_ref().unwrap().uid;
            self.ropes.retain(|r| r.a != uid && r.b != uid);
            let obj = &mut self.objs[o];
            obj.amb = None;
            obj.body = None;
            if let Some(v) = obj.veh.as_mut() {
                v.here = false;
                v.index = u32::MAX;
            }
        }
        self.manis.retain(|m| ![m.a, m.b].iter().any(|w| matches!(w, Who::Obj(o) if gone.contains(o))));
    }

    /// The game makes an ambient car active (its footprint probe, an explosion, slots 16 / 17: the
    /// same F-24099).
    pub fn ambient_let_go(&mut self, uid: u32) {
        if let Some(o) = self.amb_obj(uid) {
            self.ambient_activate(o);
        }
    }

    /// An active ambient car: its instance's matrix, velocity and turn.
    /// An ambient car's wheels' compression d (collider +676 + 24 i + 8), whether it is active or not
    /// (kept between activations; the rest rule's reset zeroes it).
    /// An ambient car's wheel `i` sliding (its collider record +20, `AmbWheel::slide`); false without a record.
    pub fn ambient_wheel_slide(&self, uid: u32, i: usize) -> bool {
        self.amb_obj(uid).and_then(|o| self.objs[o].amb.as_ref()).is_some_and(|a| a.wheels.get(i).is_some_and(|w| w.slide))
    }

    pub fn ambient_wheel_d(&self, uid: u32) -> Option<[f32; 6]> {
        let a = self.objs[self.amb_obj(uid)?].amb.as_ref()?;
        Some(std::array::from_fn(|i| a.wheels[i].d))
    }

    pub fn ambient_pose(&self, uid: u32) -> Option<([Vec3; 4], Vec3, Vec3)> {
        let o = &self.objs[self.amb_obj(uid)?];
        o.body.map(|b| (o.matrix, b.v, b.w))
    }

    /// What became of the ambient cars since the last look: (car, made active, put to rest).
    pub fn ambient_events(&mut self) -> Vec<(u32, bool, bool)> {
        let mut out = Vec::new();
        for o in self.objs.iter_mut() {
            if let Some(a) = o.amb.as_mut() {
                if a.woke || a.slept {
                    out.push((a.uid, a.woke, a.slept));
                    a.woke = false;
                    a.slept = false;
                }
            }
        }
        out
    }

    /// The thrown rider's partner among the other vehicles (artic_run.rs `RiderRagdoll::collide_other`): the ambient
    /// car - one polyhedron at its instance, as ov.0's - or an mcCar of this step's list (class 8, an AI racer: its
    /// body as `set_vehicles` hands it in, moved) whose place is nearest `at` within `reach` m. The rider's pair with it
    /// is the only one the port collides (two at once: the nearer). A semi trailer's two-part bound is taken with both parts; an mcCar that
    /// stepped before the bike is not in the bike's level (its pairs were in its own step).
    pub fn rider_partner(&self, at: Vec3, reach: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (o, obj) in self.objs.iter().enumerate() {
            let Some(vh) = obj.veh else { continue };
            let ok = match vh.class {
                6 => obj.amb.is_some(),
                8 => obj.amb.is_none() && vh.here && obj.body.is_some(),
                _ => false,
            };
            if !ok || vh.index == u32::MAX {
                continue;
            }
            let d = (obj.matrix[3] - at).length();
            if d <= reach && best.map_or(true, |(_, b)| d < b) {
                best = Some((o, d));
            }
        }
        best.map(|(o, _)| o)
    }

    /// A vehicle object's bound as the rider's collide takes it: its hull, its centre, its matrix now (as this tick's
    /// move left it) and last (collider +32: the matrix before the move; an inactive car's own).
    pub fn rider_side(&self, o: usize) -> Option<(&'static HullShape, Option<&'static HullShape>, Vec3, [Vec3; 4], [Vec3; 4])> {
        let obj = self.objs.get(o)?;
        let vh = obj.veh?;
        // (an mcCar's `cg` is its body's pivot; the collide takes the hull bound's own centre, +80)
        let cg = if vh.class == 8 { vh.shape.cg } else { vh.cg };
        Some((vh.shape, vh.shape2, cg, obj.matrix, obj.body.map_or(obj.matrix, |b| b.prev)))
    }

    /// The object is in a pair of the loose world this tick (a manifold with contacts).
    pub fn in_pairs(&self, o: usize) -> bool {
        self.manis.iter().any(|m| m.a == Who::Obj(o) || m.b == Who::Obj(o))
    }

    /// The objects the level holds by kind (the perf line, GitHub #48): untouched props, moved or broken props,
    /// pieces (made objects: torn bumpers, broken-off parts), vehicles here, vehicles not here, ambient cars,
    /// empty slots.
    pub fn census(&self) -> [usize; 7] {
        let mut n = [0; 7];
        for o in &self.objs {
            let k = match (&o.amb, &o.veh, o.inst) {
                (Some(_), _, _) => 5,
                (None, Some(v), _) if v.index == u32::MAX => 6,
                (None, Some(v), _) if v.here => 3,
                (None, Some(_), _) => 4,
                (None, None, Some(_)) if Self::untouched(o) => 0,
                (None, None, Some(_)) => 1,
                (None, None, None) => 2,
            };
            n[k] += 1;
        }
        n
    }

    /// A prop instance as the level has it before anything met it: inactive at its file matrix, no cache
    /// entry, never moved or broken, its sleep object as made. The port makes such an object on first need
    /// (`obj_of`) - the original's level holds every instance of a resident sector inactive - so one made
    /// again later is the same object.
    fn untouched(o: &Obj) -> bool {
        o.inst.is_some() && o.body.is_none() && o.entry.is_none() && !o.hit && !o.dormant && o.veh.is_none() && o.amb.is_none() && o.sleep.state == 1 && o.sleep.a == 12 && o.sleep.b == 17
    }

    /// The port's own bookkeeping out of the level, before a tick (GitHub #48: the list only grew, and every
    /// car's step walks it): an untouched prop no car and no loose body is near - farther than the collide's
    /// reach (8 m about each, then the type's radius + 3.2) by 16 m - is dropped (`obj_of` makes the same object
    /// again when one comes near: no behaviour of the original changes); a vehicle object whose car the game no
    /// longer has (`alive` by its id; the original takes a car out of the level with it, F-36702 -> F-27119), and
    /// an ambient car's left slot. Nothing a contact, a piece, a constraint or an extra car names goes. The
    /// others keep their order; every index held is renumbered (`renums` for those outside). Returns how many went.
    pub fn prune(&mut self, world: &LooseWorld, near: &[Vec3], alive: &dyn Fn(u32) -> bool) -> usize {
        let mut about: Vec<Vec3> = near.to_vec();
        about.extend(self.objs.iter().filter(|o| o.body.is_some() || o.amb.as_ref().is_some_and(|a| a.posed)).map(|o| o.matrix[3]));
        let named = |s: &LooseState, o: usize| {
            s.in_pairs(o)
                || s.pieces.iter().any(|p| p.obj == o)
                || s.extras.iter().any(|e| e.obj == o)
                || s.lead == Some(o)
                || s.merged.contains(&o)
                || s.fx_events.iter().any(|f| matches!(f.at, FxAt::Follow { obj, .. } if obj == o))
        };
        let gone: Vec<bool> = (0..self.objs.len())
            .map(|o| {
                let obj = &self.objs[o];
                let drop = if Self::untouched(obj) {
                    let t = &world.types[obj.ty];
                    let centre = xf(&obj.matrix, v(t.centroid));
                    let keep = 8.0 + t.radius + 3.2 + 16.0;
                    about.iter().all(|p| (centre - *p).length_squared() > keep * keep)
                } else {
                    match (&obj.amb, &obj.veh) {
                        (None, Some(vh)) => obj.body.is_none() && !vh.here && (vh.index == u32::MAX || !alive(vh.index)),
                        _ => false,
                    }
                };
                drop && !named(self, o)
            })
            .collect();
        self.remove_objs(&gone)
    }

    /// Take the objects marked `gone` out of the level (prune, reset_props): the others keep their order; every index
    /// held is renumbered (`renums` for those outside). Returns how many went.
    fn remove_objs(&mut self, gone: &[bool]) -> usize {
        let n = gone.iter().filter(|g| **g).count();
        if n == 0 {
            return 0;
        }
        let mut map = vec![usize::MAX; self.objs.len()];
        let mut k = 0;
        for o in 0..self.objs.len() {
            if !gone[o] {
                map[o] = k;
                k += 1;
            }
        }
        let mut i = 0;
        self.objs.retain(|_| {
            i += 1;
            !gone[i - 1]
        });
        let re = |o: usize| map[o];
        self.by_inst = self.objs.iter().enumerate().filter_map(|(k, o)| o.inst.filter(|_| o.veh.is_none() && o.amb.is_none()).map(|i| (i, k))).collect();
        for m in self.manis.iter_mut() {
            for w in [&mut m.a, &mut m.b] {
                if let Who::Obj(o) = *w {
                    *w = Who::Obj(re(o));
                }
            }
        }
        for p in self.pieces.iter_mut() {
            p.obj = re(p.obj);
        }
        for e in self.extras.iter_mut() {
            e.obj = re(e.obj);
        }
        self.lead = self.lead.map(re);
        for o in self.merged.iter_mut() {
            *o = re(*o);
        }
        for f in self.fx_events.iter_mut() {
            if let FxAt::Follow { obj, .. } = &mut f.at {
                *obj = re(*obj);
            }
        }
        self.renum_gen += 1;
        self.renums.push_back((self.renum_gen, map));
        while self.renums.len() > 64 {
            self.renums.pop_front();
        }
        n
    }

    /// The camera's place for the pool's viewer sphere (F-44865 from the prop manager's update every frame: radius 300).
    pub fn set_viewer(&mut self, at: Vec3) {
        self.viewer = Some([at.x, at.y, at.z, 300.0]);
    }

    /// The objects holding an entry from the pool (not one their type keeps for good, not one taken this tick).
    fn pool_users(&self, world: &LooseWorld) -> Vec<usize> {
        (0..self.objs.len())
            .filter(|o| {
                let obj = &self.objs[*o];
                obj.entry.is_some_and(|e| !e.taken) && obj.veh.is_none() && !(obj.inst.is_some() && world.types[obj.ty].keeps_entry)
            })
            .collect()
    }

    /// F-18636: an entry from the pool - its place in the list and its stamp. Room (F-56091's 160, less the entries kept
    /// for good and those in use): the back one, put in front. Full: the victim `pool_victim` picks among the used ones in
    /// the list's order, by their instances' bound spheres (+96 = F-94385 of the bound's centroid and radius by the
    /// matrix: `whole_bound`), whose place the new one takes; its own teardown waits for the tick's end (`pool_teardown`).
    /// No victim (every candidate stamped this frame or the last, or no viewer sphere): the port gives it all the same.
    fn pool_take(&mut self, world: &LooseWorld) -> (u64, u32) {
        let mut users = self.pool_users(world);
        if world.kept_entries + users.len() < POOL_CAP {
            self.pool_slot += 1;
            return (self.pool_slot, self.pool_frame);
        }
        users.sort_by_key(|o| std::cmp::Reverse(self.objs[*o].entry.map_or(0, |e| e.slot)));
        let seen: Vec<PoolSeen> = users
            .iter()
            .map(|&o| {
                let (_, _, c, r) = self.whole_bound(world, o);
                let e = self.objs[o].entry.unwrap();
                PoolSeen { stamp: e.stamp, keeps: world.types[self.objs[o].ty].keeps_entry, sphere: sphere_through(&self.objs[o].matrix, [c.x, c.y, c.z, r]) }
            })
            .collect();
        match self.viewer.and_then(|vw| pool_victim(&seen, self.pool_frame, vw)) {
            Some(k) => {
                let o = users[k];
                if dbg() || pool_dbg() {
                    eprintln!("POOLTAKE tick {} frame {} obj {o} inst {:?} at {:?} of {}", self.tick, self.pool_frame, self.objs[o].inst, self.objs[o].matrix[3], users.len());
                }
                let e = self.objs[o].entry.as_mut().unwrap();
                e.taken = true;
                (e.slot, self.pool_frame)
            }
            None => {
                self.pool_slot += 1;
                (self.pool_slot, self.pool_frame)
            }
        }
    }

    /// F-17408 (an instance's contact callback, for every run of contacts it has): its entry stamped with the frame.
    fn pool_stamp_contacts(&mut self) {
        let frame = self.pool_frame;
        for k in 0..self.manis.len() {
            if self.manis[k].slots.is_empty() {
                continue;
            }
            for w in [self.manis[k].a, self.manis[k].b] {
                if let Who::Obj(o) = w {
                    if let Some(e) = self.objs[o].entry.as_mut() {
                        e.stamp = frame;
                    }
                }
            }
        }
    }

    /// F-74663 on the entries F-18636 took this tick: a piece's own instance leaves the level (vt[40]); a prop's entry -
    /// manager byte +778 is 1 in play (F-47157 sets it) - takes the prop out of the level and flags it +80 0x20, which
    /// the draw visit F-92987 skips: it is neither met nor drawn until the whole city is reset (F-79581 clears the flag).
    /// Here both objects go (the prop's instance into `out`, which the collide does not take in again).
    fn pool_teardown(&mut self) {
        let named = |s: &LooseState, o: usize| s.pieces.iter().any(|p| p.obj == o) || s.extras.iter().any(|e| e.obj == o) || s.lead == Some(o) || s.merged.contains(&o);
        let mut gone = vec![false; self.objs.len()];
        for o in 0..self.objs.len() {
            // (one a piece, an extra car or a rider's merge still names waits for a later tick)
            if !self.objs[o].entry.is_some_and(|e| e.taken) || named(self, o) {
                continue;
            }
            gone[o] = true;
            if let Some(i) = self.objs[o].inst {
                self.out.insert(i);
            }
        }
        self.manis.retain(|m| ![m.a, m.b].iter().any(|w| matches!(*w, Who::Obj(o) if gone[o])));
        self.fx_events.retain(|f| !matches!(f.at, FxAt::Follow { obj, .. } if gone[obj]));
        self.remove_objs(&gone);
    }

    /// The whole city's props as new (PropManager_ResetProps, F-27147 -> F-87948; GitHub #48): first the fragment cache's

    /// sweep (F-48442 -> F-86846: every entry of the pool's used list released, F-74663 - the teardown takes a broken-off
    /// piece's own instance out of the level -; an instance whose type always holds one gets a new entry), then every prop instance's reset (the prop resource's slot 3,
    /// F-12466, calls the instance's vt+168 F-79581): its flags cleared, back at its file matrix (rebuilt from the packed
    /// placement, F-57499), inactive. Here: a broken-off piece (an object of a cache entry: no instance, `entry.from`) goes, and every
    /// prop object is as `obj_of` makes it (no body, no entry, not hit, its sleep object as made, at its file matrix) in its
    /// place in the list; the pairs that name one of them go (the next step meets them anew). The torn bumpers (`pieces`:
    /// a phInst of their own, no cache entry) are not the cache's and stay. Returns (props reset, pieces gone).
    pub fn reset_props(&mut self, world: &LooseWorld) -> (usize, usize) {
        let named = |s: &LooseState, o: usize| s.pieces.iter().any(|p| p.obj == o) || s.extras.iter().any(|e| e.obj == o) || s.lead == Some(o) || s.merged.contains(&o);
        let mut changed = vec![false; self.objs.len()];
        let mut gone = vec![false; self.objs.len()];
        // (what the pool took out of the level is put back: F-79581 clears +80 0x20 and the file matrix stands)
        let mut reset = self.out.len();
        self.out.clear();
        for o in 0..self.objs.len() {
            let obj = &self.objs[o];
            if obj.veh.is_some() || obj.amb.is_some() {
                continue;
            }
            match obj.inst {
                None => {
                    if obj.entry.is_some_and(|e| e.from.is_some()) && !named(self, o) {
                        gone[o] = true;
                        changed[o] = true;
                    }
                }
                Some(i) => {
                    if Self::untouched(obj) && obj.matrix == m4(&world.instances[i as usize].0.matrix) {
                        continue;
                    }
                    let obj = &mut self.objs[o];
                    obj.matrix = m4(&world.instances[i as usize].0.matrix);
                    obj.body = None;
                    obj.entry = None;
                    obj.hit = false;
                    obj.dormant = false;
                    obj.sleep = Sleep::reset();
                    changed[o] = true;
                    reset += 1;
                }
            }
        }
        self.manis.retain(|m| ![m.a, m.b].iter().any(|w| matches!(*w, Who::Obj(o) if changed[o])));
        self.fx_events.retain(|f| !matches!(f.at, FxAt::Follow { obj, .. } if gone[obj]));
        (reset, self.remove_objs(&gone))
    }

    /// An object's index of an older numbering (`seen`: the `renum_gen` it was taken at) in today's; None when it went
    /// (or more renumberings passed than are kept).
    pub fn renumbered(&self, seen: u64, o: usize) -> Option<usize> {
        if seen == self.renum_gen {
            return Some(o);
        }
        if self.renums.front().map_or(true, |f| f.0 > seen + 1) {
            return None;
        }
        let mut o = o;
        for (g, map) in &self.renums {
            if *g > seen {
                o = *map.get(o)?;
                if o == usize::MAX {
                    return None;
                }
            }
        }
        Some(o)
    }

    /// An inactive ambient car the rider's contact meets is made active as a car's does (A-28475 -> F-24099).
    pub fn ambient_wake(&mut self, o: usize) {
        if self.objs.get(o).is_some_and(|x| x.amb.is_some()) {
            self.ambient_activate(o);
        }
    }

    /// An inactive ambient car made active (F-24099 -> slot 28 F-41833, vfunc 41: its collider at
    /// the instance's velocity, inst +96; its turn the collider's own; the rest count from 0). A semi's
    /// cab or trailer then joins its partner by three constraints if the partner holds none, and makes
    /// the partner active (F-15252 -> F-10339).
    fn ambient_activate(&mut self, o: usize) {
        if self.objs[o].body.is_some() {
            return;
        }
        let Some(a) = self.objs[o].amb.as_ref() else { return };
        let t = a.tune.clone();
        let (uid, partner, moved, turn) = (a.uid, a.partner, a.moved, a.turn);
        let m = self.objs[o].matrix;
        let b = PBody::of_car(xf(&m, t.cg), [m[0], m[1], m[2]], moved, turn, t.mass, t.inertia, m, m, Vec3::ZERO, Vec3::ZERO);
        self.objs[o].body = Some(b);
        let a = self.objs[o].amb.as_mut().unwrap();
        a.count = 0;
        a.woke = true;
        if dbg() || rope_dbg() {
            eprintln!("AMBWAKE tick {} uid {uid} v {:?} hitch {:?} partner {:?} ropes {}", self.tick, moved, t.hitch, partner, self.ropes.len());
        }
        let (Some(h), Some(p)) = (t.hitch, partner) else { return };
        let Some(q) = self.amb_obj(p) else { return };
        if self.ropes.iter().any(|r| r.owner == p) {
            return;
        }
        let hp = self.objs[q].amb.as_ref().unwrap().tune.hitch.unwrap_or(Vec3::ZERO);
        // (F-15252: the hitches through each instance's matrix into the world; the points 10 m along each one's own
        // +z and +y from them: 6 m and 1.5 m apart at most - their z axes within 2 asin 0.3, their y axes within
        // 2 asin 0.075. The init F-44049 takes each point back into its instance's frame, F-21064 - through part 0
        // of a composite (a trailer's: the file's part matrices are the identity).)
        let (ma, mb) = (self.objs[o].matrix, self.objs[q].matrix);
        const IDENTITY: [Vec3; 4] = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO];
        let part = |x: usize| self.objs[x].veh.is_some_and(|v| v.shape2.is_some()).then_some(IDENTITY);
        let (part_a, part_b) = (part(o), part(q));
        let (wa, wb) = (hitch_world(&ma, h), hitch_world(&mb, hp));
        for (row, len) in [(None, ROPE_POINT), (Some(2), 6.0), (Some(1), 1.5)] {
            let (pa, pb) = match row {
                None => (wa, wb),
                Some(r) => (hitch_along(wa, ma[r]), hitch_along(wb, mb[r])),
            };
            let (la, lb) = (rope_local(&ma, pa, part_a.as_ref()), rope_local(&mb, pb, part_b.as_ref()));
            self.ropes.push(Rope { owner: uid, a: uid, b: p, la, lb, len });
        }
        self.ambient_activate(q);
    }

    /// After the tick (car slot 6, F-43567): an active ambient car's velocity is its place's
    /// change (F-53150), and the rest rule (F-72067): from its fourth frame active, a frame
    /// whose velocity is within 0.001 m/s on every axis resets its wheels (F-73960) and takes it
    /// out (F-38307 -> slot 19 F-38021: its constraints go; the collider keeps its turn).
    pub fn ambient_frame(&mut self, dt: f32) {
        for o in 0..self.objs.len() {
            let Some(b) = self.objs[o].body else { continue };
            let at = self.objs[o].matrix[3];
            let Some(a) = self.objs[o].amb.as_mut() else { continue };
            a.moved = match a.last_at {
                Some(p) if dt > 0.0 => (at - p) / dt,
                _ => a.moved,
            };
            a.last_at = Some(at);
            if a.count > 2 {
                let still = |x: f32| x.abs() <= f32::from_bits(0x3a83_126f);
                if still(a.moved.x) && still(a.moved.y) && still(a.moved.z) {
                    a.wheels = [AmbWheel::default(); 6];
                    a.turn = b.w;
                    a.slept = true;
                    a.count = 0;
                    let uid = a.uid;
                    self.objs[o].body = None;
                    self.ropes.retain(|r| r.owner != uid);
                    if dbg() {
                        eprintln!("AMBREST tick {} uid {uid}", self.tick);
                    }
                }
            }
            if let Some(a) = self.objs[o].amb.as_mut() {
                a.count += 1;
            }
        }
    }

    /// The constraints' contacts for this update (F-87905 -> F-83853), before the collide: each
    /// one's manifold made afresh. A side inactive is made active once the two points are more than
    /// the length and 0.01 apart, else nothing; a rope that is slack has no contact; past +164 the
    /// rest moves the two bodies apart by their masses (F-64595, into the push).
    fn ropes_touch(&mut self) {
        self.manis.retain(|m| !m.rope);
        let ropes = self.ropes.clone();
        for r in ropes {
            let (Some(oa), Some(ob)) = (self.amb_obj(r.a), self.amb_obj(r.b)) else { continue };
            let (pa, pb) = (xf(&self.objs[oa].matrix, r.la), xf(&self.objs[ob].matrix, r.lb));
            let far = |pa: Vec3, pb: Vec3| rope_far(pa, pb, r.len);
            if self.objs[oa].body.is_none() {
                if !far(pa, pb) {
                    continue;
                }
                self.ambient_activate(oa);
            }
            if self.objs[ob].body.is_none() {
                if !far(pa, pb) {
                    continue;
                }
                self.ambient_activate(ob);
            }
            let (n, err) = rope_normal(pa, pb, r.len);
            if rope_dbg() {
                eprintln!("ROPE tick {} {} -> {} len {} err {:.4} active {} {} island-to-be", self.tick, r.a, r.b, r.len, err, self.objs[oa].body.is_some(), self.objs[ob].body.is_some());
            }
            let Some((kind, depth, excess)) = rope_depth(err, r.len) else { continue };
            if excess > 0.0 {
                let pushes = rope_push(self.objs[oa].body.map(|b| b.inv_mass), self.objs[ob].body.map(|b| b.inv_mass), n, excess);
                for (o, p) in [(oa, pushes[0]), (ob, pushes[1])] {
                    if let (Some(b), Some(p)) = (self.objs[o].body.as_mut(), p) {
                        b.push = solver::direct(b.push, p);
                    }
                }
            }
            let side = |m: &[Vec3; 4], p: Vec3| Side { body: None, point: p, local: inv(m, p - m[3]), rows: [m[0], m[1], m[2]], margin: 0.04, core: Vec3::ZERO };
            let t = Touch { depth, friction: 0.0, elasticity: 0.0, a: side(&self.objs[oa].matrix, pa), b: side(&self.objs[ob].matrix, pb), normal: n, back: Vec3::ZERO - n, kind, ..Default::default() };
            let slot = Slot2 { t, age: 0, enabled: true, element: 0, surface: Surface::Prop, surface_a: Surface::Prop };
            self.manis.push(Mani { a: Who::Obj(oa), b: Who::Obj(ob), part_a: 0, part_b: 0, cell: 0, stale: false, visited: true, island: 0, order: 0, slots: vec![slot], rope: true, owner: 0 });
        }
    }
}

/// An active ambient car's wheels (slot 14 F-37370 -> F-19974, per wheel): a probe from half a metre
/// above to half a metre below the wheel's place in the car's floor plane (its x and z; F-18688 -> F-57005,
/// the ground; a polygon steeper than 45 degrees is no hit); the spring along the hit's normal with a
/// constant quarter of the car's weight on top, its push when it is past its travel; the tyre's
/// stretch across the normal against the slip - the front pair's whole, the others' less their rolling
/// - into the body's forces for the next update. `m` the instance matrix (rows X, up, Z, place).
fn ambient_wheels(a: &mut Amb, b: &mut PBody, m: &[Vec3; 4], city: &super::collision::WorldCollider, dt: f32) {
    if dt == 0.0 {
        return;
    }
    let t = a.tune.clone();
    let wt = WheelTune { grip: t.grip, travel: t.travel, stretch: t.stretch, cog_sim: t.cog_sim, k: t.k, c: t.c, kt: t.kt, ct: t.ct, mass: t.mass };
    let (x, u, z, p) = (m[0], m[1], m[2], m[3]);
    for (i, w) in t.wheels.iter().enumerate().take(6) {
        let rec = &mut a.wheels[i];
        // (F-18688 with f1 0.5 from F-37370: base +- 0.5 the up row, the wheel's height not used; the spring's
        // zero the segment's middle, (start + end) x 0.5 - the wheel's base itself)
        let base = (x * w.x + p) + z * w.z;
        let top = base + u * 0.5;
        let bottom = base - u * 0.5;
        let mid = (top + bottom) * 0.5;
        let seg = bottom - top;
        let len = seg.length();
        // (the city bound only, one-sided, the nearest hit: collision.rs `probe_front`)
        let hit = (len > 0.0).then(|| city.probe_front(top, seg / len, len)).flatten().map(|h| (h.point, h.normal)).filter(|(_, n)| !(n.y.abs() < f32::from_bits(0x3f35_0481)));
        let Some((contact, n)) = hit else {
            ambient_wheel_air(rec, u.y, wt.travel, dt);
            continue;
        };
        // (slot 21: the point's velocity)
        let vp = b.v + cross(b.w, contact - b.at);
        let out = ambient_wheel_contact(rec, &wt, m, i <= 1, contact, n, mid, vp, b.v, dt);
        b.force = b.force + out.force;
        b.torque = b.torque + out.torque;
        if let Some(push) = out.push {
            b.push = solver::direct(b.push, push);
        }
        b.force = b.force + out.tyre;
        b.torque = b.torque + cross(contact - b.at, out.tyre);
    }
}

/// The type's numbers F-19974 reads (+204 grip, +216 travel, +228 stretch, +240 cog, +244 k, +248 c, +252 kt, +256 ct) and
/// the archetype's mass (the quarter weight F-60791).
#[derive(Clone, Copy, Debug)]
pub(crate) struct WheelTune {
    pub grip: f32,
    pub travel: f32,
    pub stretch: f32,
    pub cog_sim: f32,
    pub k: f32,
    pub c: f32,
    pub kt: f32,
    pub ct: f32,
    pub mass: f32,
}

/// What F-19974 hands the body for one wheel on the ground: the spring's force (vfunc +128), its torque (+132), the push
/// past the travel (+108), the tyre's force at the contact (+124).
#[derive(Clone, Copy, Debug)]
pub(crate) struct WheelCalls {
    pub force: Vec3,
    pub torque: Vec3,
    pub push: Option<Vec3>,
    pub tyre: Vec3,
}

/// F-19974 with no hit: d less the up row's y x dt, slide off, d kept within the travel.
pub(crate) fn ambient_wheel_air(rec: &mut AmbWheel, u_y: f32, travel: f32, dt: f32) {
    rec.slide = false;
    rec.d = ex::fnms(u_y, dt, rec.d);
    rec.d = rec.d.clamp(-travel, travel);
}

/// F-19974 on a hit (`contact`, normal `n`; `mid` the probe's middle, `vp` the point's velocity from the body's slot 21, `v`
/// the body's velocity +272, `m` the instance matrix, `front` the front pair): the deflection d along the up row, its rate
/// (0 when the old d is 0), the spring along n with the quarter weight on y, the push past the travel, the tyre's stretch
/// across n against the slip (the front pair's whole, the others' less their rolling), its force; d kept within the travel.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ambient_wheel_contact(rec: &mut AmbWheel, t: &WheelTune, m: &[Vec3; 4], front: bool, contact: Vec3, n: Vec3, mid: Vec3, vp: Vec3, v: Vec3, dt: f32) -> WheelCalls {
    let inv_dt = 1.0 / dt;
    let (x, u, z, p) = (m[0], m[1], m[2], m[3]);
    let yzx = |v: Vec3| Vec3::new(v.y, v.z, v.x);
    let zxy = |v: Vec3| Vec3::new(v.z, v.x, v.y);
    let gj = |a: Vec3, b: Vec3| zxy(a) * yzx(b) - yzx(a) * zxy(b);
    let s = fma(v.z, v.z, fma(v.x, v.x, v.y * v.y)).sqrt();
    let slip = if front {
        let wk = Vec3::new(dot3(z, Vec3::new(x.x, u.x, z.x)), dot3(z, Vec3::new(x.y, u.y, z.y)), dot3(z, Vec3::new(x.z, u.z, z.z)));
        let uu = zxy(wk) * yzx(vp) - yzx(wk) * zxy(vp);
        vp - wk * uu
    } else {
        z * s + vp
    };
    let c1 = gj(n, slip);
    let c2 = gj(n, c1);
    let diff = contact - mid;
    let d_old = rec.d;
    let d = fma(diff.x, u.x, fma(u.y, diff.y, u.z * diff.z));
    rec.d = d;
    let rate = if d_old == 0.0 { 0.0 } else { (d - d_old) * inv_dt };
    let cosn = fma(u.x, n.x, fma(u.z, n.z, u.y * n.y));
    let mut f = n * (cosn * fma(t.k, d, t.c * rate));
    // (a quarter of the archetype's weight, no gravity factor)
    let sup = quarter_weight(Some(t.mass));
    f.y = sup + f.y;
    let r = u * (t.cog_sim * u.y) + (contact - p);
    let torque = gj(f, r);
    let push = (d > t.travel).then(|| n * (d - t.travel));
    let s_old = rec.s;
    let st = fma(c2.x, c2.x, fma(c2.y, c2.y, c2.z * c2.z)).sqrt() * dt;
    rec.s = st;
    if st > t.stretch {
        rec.raw = st;
        rec.slide = true;
        rec.s = t.grip * t.stretch;
    } else {
        rec.slide = false;
    }
    let rate_s = (rec.s - s_old) * inv_dt;
    let l2 = dot3(c2, c2);
    let dir = if l2 == 0.0 { Vec3::ZERO } else { c2 * super::gjk::rsqrt_as_original(l2) };
    let fac = if s < f32::from_bits(0x3f64_e26d) { 4.0 } else { 1.0 };
    let ft = ex::fms(fac * t.kt, rec.s, t.ct * rate_s) * u.y;
    let tyre = dir * ft;
    rec.d = rec.d.clamp(-t.travel, t.travel);
    WheelCalls { force: f, torque, push, tyre }
}

// ---------------------------------------------------------------------------------------------
// A car's torn-off bumper (docs/reference/audio/DAMAGE_PARTS_RULE.md 2.4, 2.5): the anim object's
// mcCarAnimBreakablePartInst puts its own phInst into the level when both ends have broken
// (F-44446 @A-63668 -> F-10733). Its archetype and bound are made by the bumper's
// constructor (F-48443): a phArchetypePhys (F-90982: max speed 500, max spin 2 pi) of mass 4,
// the angular inertia of a box 1.25 x the bound (F-30573, floored by F-88418), gravity factor
// 1.5 (vt+56 = F-18589 writes archetype +48, the factor pre_collide multiplies gravity by), type /
// include flags 2 / 0xFFFFBFFF as a prop's; a box bound 1.8 x 0.333 x 0.5 (F-17528, full sizes,
// margin 0.04 from the bound's constructor F-26930), centre of gravity (0, 0, 0.15) (F-83058:
// bound +80, flag +6), material "prop" (0x82043524 through the material manager's vt+24, bound vt+52).
// No damping (a plain phArchetypePhys). It is a pool body with the simulator's sleep object (p1
// body_decode.md section 6), which the port's props do not have yet: its rule is kept here for the
// pieces alone.

/// The name the port gives the bumper's type among the level's (it is not in the prop file).
pub const PIECE_TYPE: &str = "mcCarAnimBreakablePartInst";

/// The bumper's type as the loose bodies read a type.
/// F-30573: a box's moments, ((y^2 + z^2) m) / 12 and the like (the twelfth as [0x8202E194]).
/// F-60791: an ambient car's quarter weight - its archetype's mass (the archetype's mass call: +40; 500 kg with no
/// archetype) x 9.8 x 0.25.
pub fn quarter_weight(mass: Option<f32>) -> f32 {
    (mass.unwrap_or(f32::from_bits(0x43fa_0000)) * f32::from_bits(0x411c_cccd)) * 0.25
}

pub fn box_inertia(mass: f32, s: Vec3) -> [f32; 3] {
    let k = f32::from_bits(0x3daa_aaab);
    [((s.z * s.z + s.y * s.y) * mass) * k, ((s.x * s.x + s.z * s.z) * mass) * k, ((s.x * s.x + s.y * s.y) * mass) * k]
}

pub fn piece_type() -> FragType {
    let size = Vec3::new(1.8, 0.333, 0.5);
    let half = size * 0.5;
    let cg = [0.0, 0.0, 0.15];
    let radius = half.length();
    let mass = 4.0f32;
    // (F-30573: ((y^2 + z^2) m) / 12 ... of the box 1.25 x the bound; F-88418's floor leaves these as they are)
    let s = size * 1.25;
    let inertia = box_inertia(mass, s);
    let corners: Vec<[f32; 3]> = (0..8).map(|i| [if i & 1 == 0 { -half.x } else { half.x }, if i & 2 == 0 { -half.y } else { half.y }, if i & 4 == 0 { -half.z } else { half.z }]).collect();
    let bound = FragBound { kind: 3, flag6: true, radius, max: half.to_array(), min: (-half).to_array(), centroid: [0.0; 3], cg, volume: size.x * size.y * size.z, margin: 0.04, verts: corners, ..Default::default() };
    FragType {
        off: usize::MAX,
        name: PIECE_TYPE.to_string(),
        // (a phInst of its own: no cache entry; activated again by any blow once it has gone to sleep)
        flags: 0,
        min_move: 0.0,
        elasticity: 0.5,
        archetype: mcla_formats::rage::fragphys::FragArchetype {
            type_flags: 2,
            include: 0xFFFF_BFFF,
            mass,
            inv_mass: 1.0 / mass,
            gravity: 1.5,
            max_speed: BODY_MAX_SPEED,
            max_spin: BODY_MAX_SPIN,
            inertia,
            inv_inertia: inertia.map(recip),
        },
        radius,
        max: half.to_array(),
        min: (-half).to_array(),
        centroid: [0.0; 3],
        cg,
        margin: 0.04,
        parts: vec![mcla_formats::rage::fragphys::FragPart { bound: Some(bound), matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]] }],
        ..Default::default()
    }
}

impl LooseWorld {
    /// The number of the torn bumper's type (appended by `new`).
    pub fn piece_type_index(&self) -> Option<usize> {
        self.types.iter().rposition(|t| t.off == usize::MAX && t.name == PIECE_TYPE)
    }
}

/// The simulator's sleep object of a pool body (0x82080724; body_decode.md 6.1): state 1 awake,
/// 0 settling, 2 asleep; the counters A and B against 15 and 20 ticks.
#[derive(Clone, Copy, Debug)]
pub struct Sleep {
    pub state: u8,
    pub a: u32,
    pub b: u32,
}

impl Sleep {
    /// F-81453, at every activation: A = 15 - 3, B = 20 - 3, awake.
    pub fn reset() -> Sleep {
        Sleep { state: 1, a: 12, b: 17 }
    }

    /// The update (F-43494) with the object's own limits 15 and 20.
    pub fn update(&mut self, still: bool) {
        self.update_with(still, 15, 20);
    }

    /// F-43494: where the body is still, counter A runs while the object is awake (state 1) and settles it (state 0) at
    /// its limit, then counter B runs while it is not awake and puts it to sleep (state 2) at its limit; where it moves,
    /// an awake object starts both counters again and any other is woken (the original's slot 1, which the port does
    /// as state 1; this gives whether it was called).
    pub fn update_with(&mut self, still: bool, limit_a: u32, limit_b: u32) -> bool {
        if still {
            if self.state == 1 {
                self.a += 1;
                if self.a >= limit_a {
                    self.state = 0;
                }
            }
            if self.state != 1 {
                self.b += 1;
                if self.b >= limit_b {
                    self.state = 2;
                }
            }
            false
        } else if self.state == 1 {
            self.a = 0;
            self.b = 0;
            false
        } else {
            self.state = 1;
            true
        }
    }
}

/// The "still" test (F-61306 -> F-66163) with the game's tolerances: v^2 and w^2 0.05.
fn still(b: &PBody, dt: f32, f2: f32) -> bool {
    let (v, w, fifth) = (f32::from_bits(0x3d4c_cccd), f32::from_bits(0x3d4c_cccd), f32::from_bits(0x3e4c_cccd));
    still_with(still_terms(b, dt, f2), 0.0, [v, w, fifth])
}

/// F-61306: the five terms against the object's tolerances (+4 for the two velocity terms, +8 for the two spin terms,
/// +12 for the fifth, which a pool body's slot 15 gives as 0): not one above (a NaN is not).
fn still_with(terms: [f32; 4], fifth: f32, tol: [f32; 3]) -> bool {
    let [a, bb, c, d] = terms;
    !(a > tol[1]) && !(bb > tol[0]) && !(c > tol[1]) && !(d > tol[0]) && !(fifth > tol[2])
}

/// The four terms the still test compares with its tolerances: |w1|^2, |v1|^2, and the growths by the tick's sums.
fn still_terms(b: &PBody, dt: f32, f2: f32) -> [f32; 4] {
    let invdt = if dt != 0.0 { 1.0 / dt } else { f32::MAX };
    let sq = |x: Vec3| fma(x.x, x.x, fma(x.y, x.y, x.z * x.z));
    // (+432 / +448 hold nothing the port has: zero)
    let w1 = b.turn * invdt + b.w;
    let a = sq(w1);
    let v1 = b.push * invdt + b.v;
    let bb = sq(v1);
    let x = b.torque * f2 + b.jt;
    let m = ex::world_inverse_inertia(&b.rows, b.inv_inertia);
    let w2 = Vec3::new(dot3(x, Vec3::new(m[0].x, m[1].x, m[2].x)), dot3(x, Vec3::new(m[0].y, m[1].y, m[2].y)), dot3(x, Vec3::new(m[0].z, m[1].z, m[2].z))) + w1;
    let c = sq(w2) - a;
    let y = b.force * f2 + b.jf;
    let v2 = y * b.inv_mass + v1;
    let d = sq(v2) - bb;
    [a, bb, c, d]
}

/// What the bumper's contact callback (F-80340) reads of one of its contacts this tick: every contact whose
/// +165 bit 0x80 is set - the contact's "enabled" bit (set with 0x40 when it is made, F-51639 / F-34223;
/// cleared by a callback that takes the contact from the solver, `clrlwi ..,25` in F-58598, F-50733 ...), which
/// is the port's `Slot2::enabled`; tested at A-53896.
#[derive(Clone, Copy, Debug)]
pub struct PieceContact {
    /// the relative velocity at the contact (the piece less the other side), the normal as the
    /// callback turns it (negated when the piece is A of the pair), the place
    pub v: Vec3,
    pub n: Vec3,
    pub point: Vec3,
}

/// One torn-off bumper in the level.
#[derive(Clone, Debug)]
pub struct Piece {
    pub id: u64,
    pub obj: usize,
    pub sleep: Sleep,
    /// asleep and taken out of the simulator (F-38307): an inactive object at its last matrix
    pub off: bool,
    /// its contacts as the last refresh had them
    pub contacts: Vec<PieceContact>,
}

/// MCLA_DBG_PIECE: every enabled contact of a piece, tick by tick (stderr)
static DBG_PIECE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static PIECE_IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl LooseState {
    /// Put a torn bumper into the level at `matrix` (its frame: rows and place) moving at `v`, turning
    /// at `w` (F-10733, then the body's vt+164 / vt+172). None where the level has no such type.
    pub fn add_piece(&mut self, world: &LooseWorld, matrix: [Vec3; 4], v: Vec3, w: Vec3) -> Option<u64> {
        let ty = world.piece_type_index()?;
        let t = &world.types[ty];
        let a = &t.archetype;
        let e = Entry { absent: 0, age: 0.0, cg: self::v(t.cg), mass: a.mass, inv_mass: a.inv_mass, inertia: self::v(a.inertia), inv_inertia: self::v(a.inv_inertia), ..Default::default() };
        let mut body = PBody::new(&matrix, e.cg, &e, a.max_speed, a.max_spin);
        set_velocity(&mut body, v);
        body.w = w;
        body.l = body.momentum_of(w);
        let o = self.objs.len();
        // (inserted after the car: the car is A against it)
        self.objs.push(Obj { ty, inst: None, matrix, entry: None, hit: true, body: Some(body), level: 3_000_000 + o as i64, addr: 3_000_000 + o as i64, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
        let id = PIECE_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.pieces.push(Piece { id, obj: o, sleep: Sleep::reset(), off: false, contacts: Vec::new() });
        Some(id)
    }

    /// Take a piece out of the level (the bumper's reset, F-40944). The object stays in the
    /// list (other objects are known by their place in it) far below the city, without a body.
    pub fn remove_piece(&mut self, id: u64) {
        let Some(k) = self.pieces.iter().position(|p| p.id == id) else { return };
        let o = self.pieces.remove(k).obj;
        let obj = &mut self.objs[o];
        obj.body = None;
        obj.hit = true;
        obj.matrix = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(0.0, -100_000.0, 0.0)];
        self.manis.retain(|m| m.a != Who::Obj(o) && m.b != Who::Obj(o));
    }

    /// Take out every piece no car's bumper holds any more: the original removes a torn-off piece only with its
    /// bumper, on the reset (F-40944) or when the car itself goes (the car's destructor F-81512 -> the anim object's
    /// F-13108 -> the bumper's, which takes the level handle +532 out, F-27119). Returns how many went.
    pub fn keep_pieces(&mut self, held: &dyn Fn(u64) -> bool) -> usize {
        let gone: Vec<u64> = self.pieces.iter().map(|p| p.id).filter(|id| !held(*id)).collect();
        for id in &gone {
            self.remove_piece(*id);
        }
        gone.len()
    }

    /// A piece's matrix as the level has it, its contacts of the last tick and whether it is asleep
    /// (None: not in this level).
    pub fn piece(&self, id: u64) -> Option<([Vec3; 4], &[PieceContact], bool)> {
        let p = self.pieces.iter().find(|p| p.id == id)?;
        Some((self.objs[p.obj].matrix, &p.contacts, p.off))
    }

    /// PreCollide's sleep test (F-24444): a settling or asleep body that is not still wakes up.
    fn pieces_wake(&mut self, dt: f32) {
        for p in self.pieces.iter_mut() {
            let Some(b) = self.objs[p.obj].body.as_ref() else { continue };
            if p.off {
                // (hit while inactive: the breaking pass gave it a new body, v = w = 0)
                p.off = false;
                p.sleep = Sleep::reset();
                continue;
            }
            if p.sleep.state != 1 && !still(b, dt, 0.0) {
                p.sleep.state = 1;
            }
        }
    }

    /// The bumper's contact callback's view of its contacts after the refresh (F-83112 -> F-80340).
    fn pieces_touched(&mut self, car: &CarIn) {
        for k in 0..self.pieces.len() {
            let o = self.pieces[k].obj;
            let mut out = Vec::new();
            if let Some(b) = self.objs[o].body {
                let vel_of = |w: Who, at: Vec3| match w {
                    Who::Obj(x) => self.objs[x].body.map_or(Vec3::ZERO, |ob| ob.v + cross(ob.w, at - ob.at)),
                    Who::Car => car.vel,
                    Who::City => Vec3::ZERO,
                };
                for m in self.manis.iter().filter(|m| m.a == Who::Obj(o) || m.b == Who::Obj(o)) {
                    let piece_is_a = m.a == Who::Obj(o);
                    let other = if piece_is_a { m.b } else { m.a };
                    for s in m.slots.iter().filter(|s| s.enabled) {
                        let at = if piece_is_a { s.t.a.point } else { s.t.b.point };
                        // (each body's velocity at its own point of the contact: the pair's +48 / +64, F-80340)
                        let other_at = if piece_is_a { s.t.b.point } else { s.t.a.point };
                        let v = (b.v + cross(b.w, at - b.at)) - vel_of(other, other_at);
                        let n = if piece_is_a { Vec3::ZERO - s.t.normal } else { s.t.normal };
                        out.push(PieceContact { v, n, point: at });
                        if *DBG_PIECE.get_or_init(|| std::env::var_os("MCLA_DBG_PIECE").is_some()) {
                            eprintln!("PIECECON tick {} other {:?} age {} depth {:.4} n {:?} v {:?} at {:?} body v {:?} w {:?}", self.tick, other, s.age, s.t.depth, n, v, at, b.v, b.w);
                        }
                    }
                }
            }
            self.pieces[k].contacts = out;
        }
    }

    /// The sleep object's update (F-43494, after the velocity solves, before the integration) and the
    /// deactivation of a piece asleep in an island of its own (F-50834 -> F-38307; the car and the
    /// port's props have no sleep object, so an island with any of them never sleeps).
    fn pieces_sleep(&mut self, dt: f32) {
        for k in 0..self.pieces.len() {
            let o = self.pieces[k].obj;
            let Some(b) = self.objs[o].body else { continue };
            if self.pieces[k].sleep.state == 2 {
                let alone = !self.manis.iter().any(|m| {
                    (m.a == Who::Obj(o) || m.b == Who::Obj(o)) && [m.a, m.b].into_iter().any(|w| w == Who::Car || matches!(w, Who::Obj(x) if x != o && self.objs[x].body.is_some()))
                });
                if alone {
                    self.pieces[k].off = true;
                    let obj = &mut self.objs[o];
                    obj.body = None;
                    // (an inactive instance the breaking pass may move again)
                    obj.hit = false;
                    continue;
                }
            }
            let s = &mut self.pieces[k].sleep;
            s.update(still(&b, dt, dt));
        }
    }
}

impl LooseState {
    /// Has the object a sleep object of the generic kind (a pool body that is no vehicle and no torn bumper)?
    fn sleeper(&self, o: usize) -> bool {
        // (an ambient car's collider has its own, collider +820 - F-91199 -> F-64883: the same rule)
        (self.objs[o].veh.is_none() || self.objs[o].amb.is_some()) && !self.pieces.iter().any(|p| p.obj == o)
    }

    /// Take an asleep object out of the simulator (F-38307): inactive at its last matrix, its entry and
    /// its "hit" bit kept.
    fn deactivate(&mut self, o: usize) {
        // (an ambient car: F-38307 -> inst vtable[29] F-87090 -> car slot 19 F-38021 - its
        // constraints go; the collider keeps its turn and its sleep object; the wheels are not reset)
        let body = self.objs[o].body;
        if let (Some(a), Some(b)) = (self.objs[o].amb.as_mut(), body) {
            a.turn = b.w;
            a.slept = true;
            let uid = a.uid;
            self.ropes.retain(|r| r.owner != uid);
            self.objs[o].body = None;
            if dbg() || rope_dbg() {
                eprintln!("AMBSLEEP tick {} uid {uid}", self.tick);
            }
            return;
        }
        let obj = &mut self.objs[o];
        obj.body = None;
        obj.dormant = true;
        if dbg() {
            eprintln!("LOOSESLEEP tick {} obj {o} level {}", self.tick, obj.level);
        }
    }

    /// F-44716 -> F-24099: an inactive object that has been hit is activated again with a new body at
    /// rest (F-28128: v = w = 0) and a reset sleep object; one never hit refuses (vtable[28] F-79777).
    fn wake_obj(&mut self, world: &LooseWorld, o: usize) {
        if !self.objs[o].dormant || self.objs[o].body.is_some() || !self.objs[o].hit || !self.sleeper(o) {
            return;
        }
        let t = &world.types[self.objs[o].ty];
        let obj = &mut self.objs[o];
        obj.body = Some(match obj.entry {
            Some(e) => PBody::new(&obj.matrix, e.cg, &e, BODY_MAX_SPEED, BODY_MAX_SPIN),
            None => {
                let a = &t.archetype;
                let e = Entry { absent: 0, age: 0.0, cg: v(t.cg), mass: a.mass, inv_mass: a.inv_mass, inertia: v(a.inertia), inv_inertia: v(a.inv_inertia), ..Default::default() };
                PBody::new(&obj.matrix, e.cg, &e, a.max_speed, a.max_spin)
            }
        });
        (obj.sleep, obj.dormant) = (Sleep::reset(), false);
        if dbg() {
            eprintln!("LOOSEWAKE tick {} obj {o} level {}", self.tick, obj.level);
        }
    }

    /// PreCollide's sleep test (F-24444) for every body with a sleep object.
    fn bodies_wake(&mut self, dt: f32) {
        for o in 0..self.objs.len() {
            if !self.sleeper(o) {
                continue;
            }
            let Some(b) = self.objs[o].body.as_ref() else { continue };
            if self.objs[o].sleep.state != 1 && !still(b, dt, 0.0) {
                self.objs[o].sleep.state = 1;
            }
        }
    }

    /// The sleep object's update (F-43494, in F-96487's loop before the integration) and deactivation A
    /// (body_decode.md 6.2): asleep with no manifold this tick - not integrated, taken out.
    fn bodies_sleep(&mut self, dt: f32) {
        for o in 0..self.objs.len() {
            if !self.sleeper(o) {
                continue;
            }
            // (the Update's test runs before the push solve, and PreCollide's F-67019 has left +432 / +448 zero:
            // no push or turn enters it - the port's are this tick's push solve's. V s_rtv 22265: not still at s385
            // by its w alone, asleep at s420 as the original, not s419)
            let Some(mut b) = self.objs[o].body else { continue };
            (b.push, b.turn) = (Vec3::ZERO, Vec3::ZERO);
            let s = &mut self.objs[o].sleep;
            s.update(still(&b, dt, dt));
            if dbg() {
                let s = &self.objs[o].sleep;
                eprintln!("LOOSESLEEPOBJ tick {} obj {o} state {} a {} b {} still {} terms {:?}", self.tick, s.state, s.a, s.b, still(&b, dt, dt), still_terms(&b, dt, dt));
            }
            if self.objs[o].sleep.state == 2 && !self.manis.iter().any(|m| m.a == Who::Obj(o) || m.b == Who::Obj(o)) {
                self.deactivate(o);
            }
        }
    }

    /// Deactivation B (F-50834 -> F-24167 / F-56803): an island whose bodies are all asleep is taken
    /// out before its solve - no breaking, no impulses. A body without a sleep object of this kind (the car, a
    /// vehicle, a torn bumper - the port keeps the bumper's own rule) keeps its island awake.
    fn islands_asleep(&mut self) -> Vec<usize> {
        let mut seen: Vec<usize> = Vec::new();
        let mut asleep = Vec::new();
        for k in 0..self.manis.len() {
            let isl = self.manis[k].island;
            if isl == 0 || seen.contains(&isl) {
                continue;
            }
            seen.push(isl);
            let mut bodies: Vec<usize> = Vec::new();
            let mut all = true;
            for m in self.manis.iter().filter(|m| m.island == isl) {
                for w in [m.a, m.b] {
                    match w {
                        Who::Car => all = false,
                        Who::City => {}
                        Who::Obj(o) => {
                            if self.objs[o].body.is_none() {
                                continue;
                            }
                            if !self.sleeper(o) || self.objs[o].sleep.state != 2 {
                                all = false;
                            } else if !bodies.contains(&o) {
                                bodies.push(o);
                            }
                        }
                    }
                }
            }
            if all {
                asleep.push(isl);
                for o in bodies {
                    self.deactivate(o);
                }
            }
        }
        asleep
    }
}

/// A piece on its way from one car's level to another's (the original has one level: whichever car reaches the
/// piece meets it).
#[derive(Clone, Debug)]
pub struct PieceMove {
    id: u64,
    matrix: [Vec3; 4],
    body: Option<PBody>,
    hit: bool,
    sleep: Sleep,
    off: bool,
}

impl LooseState {
    /// Where a piece is and whether it touches this level's car now.
    pub fn piece_place(&self, id: u64) -> Option<(Vec3, bool)> {
        let p = self.pieces.iter().find(|p| p.id == id)?;
        let o = p.obj;
        let touching = self.manis.iter().any(|m| (m.a == Who::Obj(o) && m.b == Who::Car) || (m.b == Who::Obj(o) && m.a == Who::Car));
        Some((self.objs[o].matrix[3], touching))
    }

    /// The ids of the pieces this level holds.
    pub fn piece_ids(&self) -> Vec<u64> {
        self.pieces.iter().map(|p| p.id).collect()
    }

    /// Take a piece out of this level with its body and sleep state, for another car's.
    pub fn take_piece(&mut self, id: u64) -> Option<PieceMove> {
        let p = self.pieces.iter().find(|p| p.id == id)?.clone();
        let obj = &self.objs[p.obj];
        let out = PieceMove { id, matrix: obj.matrix, body: obj.body, hit: obj.hit, sleep: p.sleep, off: p.off };
        self.remove_piece(id);
        Some(out)
    }

    /// Put a piece taken from another car's level into this one, as it was.
    pub fn put_piece(&mut self, world: &LooseWorld, m: PieceMove) {
        let Some(ty) = world.piece_type_index() else { return };
        let o = self.objs.len();
        let mut body = m.body;
        if let Some(b) = body.as_mut() {
            // (the collide's "last" matrix is this level's from now on)
            b.last = m.matrix;
            b.last_set = true;
        }
        self.objs.push(Obj { ty, inst: None, matrix: m.matrix, entry: None, hit: m.hit, body, level: 3_000_000 + o as i64, addr: 3_000_000 + o as i64, veh: None, sleep: Sleep::reset(), dormant: false, amb: None });
        self.pieces.push(Piece { id: m.id, obj: o, sleep: m.sleep, off: m.off, contacts: Vec::new() });
    }
}

/// The cars' levels as the pieces see them: each car's place, how far it reaches, and its level.
pub struct CarLevel<'a> {
    pub at: Vec3,
    pub reach: f32,
    pub level: &'a mut LooseState,
}

/// Every piece goes to the level of the car that reaches it: a piece its own car neither touches nor reaches moves to
/// the nearest car that reaches it (two cars at one piece: the one that holds it keeps it).
pub fn hand_over_pieces(world: &LooseWorld, cars: &mut [CarLevel]) {
    for i in 0..cars.len() {
        for id in cars[i].level.piece_ids() {
            let Some((at, touching)) = cars[i].level.piece_place(id) else { continue };
            if touching || (at - cars[i].at).length() < cars[i].reach {
                continue;
            }
            let best = (0..cars.len()).filter(|&j| j != i).map(|j| (j, (at - cars[j].at).length())).filter(|&(j, d)| d < cars[j].reach).min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((j, _)) = best {
                if let Some(m) = cars[i].level.take_piece(id) {
                    cars[j].level.put_piece(world, m);
                }
            }
        }
    }
}

#[cfg(test)]
mod piece_tests {
    use super::*;

    fn same3(g: Vec3, e: &[u32]) -> bool {
        g.to_array().iter().zip(e).all(|(g, e)| g.to_bits() == *e || (g.is_nan() && f32::from_bits(*e).is_nan()))
    }

    fn vec_at(w: &[u32], o: usize) -> Vec3 {
        Vec3::new(f32::from_bits(w[o]), f32::from_bits(w[o + 1]), f32::from_bits(w[o + 2]))
    }

    /// The impulse at a point F-97724 run offline (tools/force_leaves_reference.py, 2,500 cases, flush mode as the SDK sets
    /// it, testdata/impulse_at_point_ref.txt): the momentum P + J it hands to the body's setter (vfunc 42), the call of
    /// vfunc 46 after it, and the angular momentum +320 it leaves, x, y and z, every word (NaN as NaN). What the two
    /// virtual calls do (the caps, the speeds) is not in the file.
    #[test]
    fn impulse_at_point_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let (mut n, mut bad) = (0, Vec::new());
        for line in include_str!("testdata/impulse_at_point_ref.txt").lines() {
            let (lhs, rhs) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = lhs.split(' ').skip(1).map(h).collect();
            let (log, l) = rhs.split_once(" | ").unwrap();
            let log: Vec<&str> = log.split(' ').collect();
            let l: Vec<u32> = l.split(' ').map(h).collect();
            let (p, nl) = impulse_sums(vec_at(&w, 0), vec_at(&w, 3), vec_at(&w, 6), vec_at(&w, 9), vec_at(&w, 12));
            let want_p: Vec<u32> = log[1..4].iter().map(|x| h(x)).collect();
            n += 1;
            if !(log[0] == "c1" && log[4] == "c2" && same3(p, &want_p) && same3(nl, &l)) {
                bad.push(line);
            }
        }
        assert_eq!(n, 2500);
        assert!(bad.is_empty(), "{} mismatches, first {:?}", bad.len(), &bad[..bad.len().min(2)]);
    }

    /// The velocity of a piece broken off a moving parent, F-74100 run offline (tools/force_leaves_reference.py, 2,500
    /// cases, testdata/piece_velocity_ref.txt): the velocity handed to the body's setter (vfunc 41) and the turn handed to
    /// vfunc 43, every word (NaN as NaN). The case words are: the level index, the flag word's low bits, whether the
    /// body is there, then the place, the centre, the velocity and the turn; the lookup (index not 0xFFFF, flags & 3 == 0,
    /// body present) is the port's `if let Some(parent body)`.
    #[test]
    fn piece_velocity_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let (mut n, mut called, mut bad) = (0, 0, Vec::new());
        for line in include_str!("testdata/piece_velocity_ref.txt").lines() {
            let (lhs, rhs) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = lhs.split(' ').skip(1).map(h).collect();
            let fires = w[0] != 0xFFFF && w[1] & 3 == 0 && w[2] != 0;
            n += 1;
            let ok = if fires {
                called += 1;
                let t: Vec<&str> = rhs.trim_end_matches('|').trim().split(' ').collect();
                let (v, turn): (Vec<u32>, Vec<u32>) = (t[2..5].iter().map(|x| h(x)).collect(), t[7..10].iter().map(|x| h(x)).collect());
                let got = piece_velocity(vec_at(&w, 3), vec_at(&w, 6), vec_at(&w, 9), vec_at(&w, 12));
                t[0] == "v" && t[5] == "w" && same3(got, &v) && same3(vec_at(&w, 12), &turn)
            } else {
                rhs.trim() == "|"
            };
            if !ok { bad.push(line); }
        }
        assert_eq!(n, 2500);
        assert!(called > 2000, "{called} calls");
        assert!(bad.is_empty(), "{} mismatches, first {:?}", bad.len(), &bad[..bad.len().min(2)]);
    }

    /// The refresh of one contact F-33100 run offline (tools/force_leaves_reference.py, 1,200 cases, flush mode as the SDK
    /// sets it, testdata/contact_refresh_ref.txt): from age 0 on, the target (+144) 0 and the age + 1 (to 16 bits); for age
    /// != 0 A's and B's world points (x, y, z, every word, NaN as NaN) and, for kind 0, the depth +12; an age 0 or a
    /// kind other than 0 leaves the words as they were. B's frame is the identity in a tenth of the cases, which the city
    /// path (B's local point kept) reproduces word for word apart from the sign of a zero.
    #[test]
    fn contact_refresh_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let (mut n, mut moved, mut kept, mut city, mut zero_sign, mut bad) = (0, 0, 0, 0, 0, Vec::new());
        for line in include_str!("testdata/contact_refresh_ref.txt").lines() {
            let (lhs, rhs) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = lhs.split(' ').skip(1).map(h).collect();
            let o: Vec<u32> = rhs.trim_start_matches("| ").trim().split(' ').map(h).collect();
            let (age, kind) = (w[0] as u16, w[1]);
            let mat = |at: usize| -> [Vec3; 4] { std::array::from_fn(|r| vec_at(&w, at + 4 * r)) };
            let (ma, mb) = (mat(11), mat(27));
            let (la, lb, back) = (vec_at(&w, 2), vec_at(&w, 5), vec_at(&w, 8));
            n += 1;
            // (the words: 0 .. 3 the record's first four, 4 .. 7 A's point, 8 .. 11 B's, 12 .. 15 the target, 16 the age)
            let mut ok = o[16] == (age.wrapping_add(1)) as u32 && o[12..16].iter().all(|x| *x == 0);
            if age != 0 {
                moved += 1;
                let (a, b, depth) = refresh_points(&ma, Some(&mb), la, lb, back);
                ok &= same3(a, &o[4..7]) && same3(b, &o[8..11]);
                if kind == 0 {
                    ok &= depth.to_bits() == o[3] || (depth.is_nan() && f32::from_bits(o[3]).is_nan());
                } else {
                    ok &= o[3] == 0xA5A5_000C;
                }
                let identity = mb.iter().zip([Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO]).all(|(r, e)| *r == e);
                if identity {
                    city += 1;
                    let (_, c, _) = refresh_points(&ma, None, la, lb, back);
                    // (a zero of the other sign is the only difference allowed)
                    let eq = |g: f32, e: u32| g.to_bits() == e || (g == 0.0 && f32::from_bits(e) == 0.0) || (g.is_nan() && f32::from_bits(e).is_nan());
                    let same_but_zero = c.to_array().iter().zip(&o[8..11]).all(|(g, e)| eq(*g, *e));
                    zero_sign += (!same3(c, &o[8..11]) && same_but_zero) as usize;
                    ok &= same_but_zero;
                }
            } else {
                kept += 1;
                ok &= o[4..12].iter().enumerate().all(|(i, x)| *x == 0xA5A5_0000 + 48 + 4 * i as u32) && o[3] == 0xA5A5_000C;
            }
            if !ok { bad.push(line); }
        }
        assert_eq!(n, 1200);
        assert!(moved > 800 && kept > 40 && city > 40, "{moved} refreshed, {kept} kept, {city} city; {zero_sign} zero signs");
        assert!(bad.is_empty(), "{} mismatches, first {:?}", bad.len(), &bad[..bad.len().min(2)]);
    }

    /// The semi's constraints as the original makes them (tools/semi_ropes_reference.py, testdata/semi_ropes_ref.txt):
    /// F-15252's world points of the three pairs (each hitch through its instance's matrix, then + 10 x row 2 and
    /// + 10 x row 1), their lengths 6 and 1.5 (the first's is the run-time global, `ROPE_POINT`), the instances, parts
    /// 0, and each constraint's mode 2, +320 0, +328 -1, +164 0.6 on the first; and the init F-44049's points in each
    /// instance's frame (F-21064, through part 0 or 1 of a composite), every word of x, y and z equal.
    #[test]
    fn semi_ropes_match_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mat = |w: &[u32], o: usize| -> [Vec3; 4] {
            std::array::from_fn(|r| Vec3::new(f32::from_bits(w[o + 4 * r]), f32::from_bits(w[o + 4 * r + 1]), f32::from_bits(w[o + 4 * r + 2])))
        };
        let v = |w: &[u32], o: usize| Vec3::new(f32::from_bits(w[o]), f32::from_bits(w[o + 1]), f32::from_bits(w[o + 2]));
        let bits = |a: Vec3| [a.x.to_bits(), a.y.to_bits(), a.z.to_bits()];
        let (mut seen, mut bad) = ([0usize; 2], Vec::new());
        for line in include_str!("testdata/semi_ropes_ref.txt").lines() {
            let (lhs, rhs) = line.split_once(" -> ").unwrap();
            let mut it = lhs.split(' ');
            let kind = it.next().unwrap();
            let w: Vec<u32> = it.map(h).collect();
            let ok = if kind == "pair" {
                seen[0] += 1;
                let (calls, made) = rhs.split_once(" | ").unwrap();
                let o: Vec<u32> = calls.split(' ').map(h).collect();
                let k: Vec<u32> = made.split(' ').map(h).collect();
                let (ma, mb) = (mat(&w, 0), mat(&w, 16));
                let (pa, pb) = (hitch_world(&ma, v(&w, 32)), hitch_world(&mb, v(&w, 36)));
                let want = [(pa, pb), (hitch_along(pa, ma[2]), hitch_along(pb, mb[2])), (hitch_along(pa, ma[1]), hitch_along(pb, mb[1]))];
                (0..3).all(|c| {
                    let r = &o[c * 13..c * 13 + 13];
                    r[0..3] == bits(want[c].0) && r[4..7] == bits(want[c].1) && r[9..13] == [0x1070, 0x2070, 0, 0]
                        && (c == 0 || r[8] == [6.0f32, 1.5][c - 1].to_bits())
                        && k[c * 4..c * 4 + 4] == [if c == 0 { 0.6f32.to_bits() } else { 0 }, 0, (-1.0f32).to_bits(), 2]
                })
            } else {
                seen[1] += 1;
                let o: Vec<u32> = rhs.split(' ').map(h).collect();
                let side = |m: usize, p: usize, t: usize| {
                    let part = (w[t] == 12).then(|| mat(&w, t + 2 + 16 * w[t + 1] as usize));
                    rope_local(&mat(&w, m), v(&w, p), part.as_ref())
                };
                o[0..4] == [0x6400, 0x6c00, w[41], w[75]] && o[4..7] == bits(side(0, 32, 40)) && o[8..11] == bits(side(16, 36, 74))
                    && o[12..20].iter().all(|x| *x == 0) && o[20] == 0xffff_ffff && o[21] == w[108]
            };
            if !ok { bad.push(line); }
        }
        assert_eq!(seen, [600, 1000]);
        assert!(bad.is_empty(), "{} mismatches, first {:?}", bad.len(), &bad[..bad.len().min(2)]);
    }

    /// The constraint update F-83853 on the semi's ropes (tools/rope_update_reference.py, testdata/rope_update_ref.txt:
    /// the constraint made by F-31401 / F-44049 / F-95940 as F-15252 makes it, then one update): the points in each
    /// instance's frame (rope_local), back to the world (xf), the wake of each inactive side when the points are far
    /// (A first, nothing more when one is not), the kind, the record's points, normal, back, depth and friction /
    /// elasticity (+100 / +104: 0, 0), and the push split (F-64595) handed to each collider - x, y and z equal as
    /// values (a composite's identity part gives a -0 lane the port's point has as +0). Cases the port has no counterpart for are counted, not compared: a failed
    /// wake, a level index 0xFFFF, a part matrix that is not the identity.
    #[test]
    fn rope_update_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(&x.replace('_', ""), 16).unwrap();
        let f = |w: u32| f32::from_bits(w);
        let mat = |w: &[u32], o: usize| -> [Vec3; 4] { std::array::from_fn(|r| Vec3::new(f(w[o + 4 * r]), f(w[o + 4 * r + 1]), f(w[o + 4 * r + 2]))) };
        let v = |w: &[u32], o: usize| Vec3::new(f(w[o]), f(w[o + 1]), f(w[o + 2]));
        let ident: [u32; 16] = [0x3f80_0000, 0, 0, 0, 0, 0x3f80_0000, 0, 0, 0, 0, 0x3f80_0000, 0, 0, 0, 0, 0];
        let (mut n_cmp, mut n_skip, mut rets, mut bad) = (0, 0, [0usize; 3], Vec::new());
        for line in include_str!("testdata/rope_update_ref.txt").lines() {
            let (lhs, rhs) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = lhs.split(' ').skip(1).map(h).collect();
            let (log, rest) = rhs.split_once("| ret ").unwrap();
            let (head, rec) = rest.split_once(" | rec ").unwrap();
            let ret = h(head.split(' ').next().unwrap());
            let rec: Vec<u32> = rec.split(' ').map(h).collect();
            let log: Vec<&str> = log.split_whitespace().collect();
            let part_ok = |t: usize| w[t] != 12 || w[t + 2 + 16 * w[t + 1] as usize..t + 18 + 16 * w[t + 1] as usize] == ident;
            if w[146] == 0xffff || w[147] == 0xffff || w[109] == 2 || w[110] == 2 || !part_ok(40) || !part_ok(74) {
                n_skip += 1;
                continue;
            }
            n_cmp += 1;
            rets[ret as usize] += 1;
            let side = |m: usize, p: usize, t: usize| {
                let part = (w[t] == 12).then_some([Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO]);
                rope_local(&mat(&w, m), v(&w, p), part.as_ref())
            };
            let (ma, mb, len) = (mat(&w, 0), mat(&w, 16), f(w[108]));
            let (pa, pb) = (xf(&ma, side(0, 32, 40)), xf(&mb, side(16, 36, 74)));
            // (the port's steps: the wakes, then the contact and the push)
            let mut want = Vec::new();
            let mut stop = false;
            for (inactive, idx) in [(w[109] == 1, 5u32), (w[110] == 1, 9)] {
                if inactive && !stop {
                    if rope_far(pa, pb, len) { want.push(format!("wake {idx:x} 0 0 -> {:x}", if idx == 5 { 0x4000 } else { 0x4400 })); } else { stop = true; }
                }
            }
            let (n, err) = rope_normal(pa, pb, len);
            let mut ok = true;
            let eq = |a: Vec3, r: &[u32]| a.x == f(r[0]) && a.y == f(r[1]) && a.z == f(r[2]);
            let contact = if stop { None } else { rope_depth(err, len) };
            let mut pushes = Vec::new();
            match contact {
                None => ok &= ret == 0,
                Some((kind, depth, excess)) => {
                    ok &= ret == kind as u32 && eq(pa, &rec[8..]) && eq(pb, &rec[12..]) && eq(n, &rec[16..]) && eq(Vec3::ZERO - n, &rec[20..]) && depth.to_bits() == rec[24] && rec[25..27] == [0, 0];
                    if excess > 0.0 {
                        let p = rope_push(Some(f(w[111])), Some(f(w[112])), n, excess);
                        for (c, q) in [(0x4000, p[0]), (0x4400, p[1])] {
                            if let Some(q) = q { pushes.push((c, q)); }
                        }
                    }
                }
            }
            // (the log: the wakes, then each push "push <collider> <r4> <r5> x y z w")
            let wl = want.join(" ");
            let nw = want.iter().map(|s| s.split(' ').count()).sum::<usize>();
            ok &= log.len() >= nw && log[..nw].join(" ") == wl;
            let pl = &log[nw.min(log.len())..];
            ok &= pl.len() == 8 * pushes.len();
            if ok {
                for (k, (c, q)) in pushes.iter().enumerate() {
                    let r: Vec<u32> = pl[8 * k + 4..8 * k + 7].iter().map(|x| h(x)).collect();
                    ok &= pl[8 * k] == "push" && h(pl[8 * k + 1]) == *c && eq(*q, &r);
                }
            }
            if !ok { bad.push(line); }
        }
        assert!(n_cmp > 300 && rets.iter().all(|r| *r > 50), "{n_cmp} compared, {n_skip} skipped, returns {rets:?}");
        assert!(bad.is_empty(), "{} of {n_cmp} mismatch, first {:?}", bad.len(), &bad[..bad.len().min(1)]);
    }

    /// SetVelocity F-64271 and ApplyImpulse F-50175 run offline (tools/body_setters_reference.py, flush mode as the
    /// SDK sets it, testdata/body_setters_ref.txt): +272, +304 and +368 in x, y and z equal, NaN as NaN. (The
    /// constraint's mode F-95940 is in the same file: the low two bits 2, the rest kept - every case.)
    #[test]
    fn body_setters_match_the_original() {
        let m = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO];
        let same = |g: f32, e: u32| g.to_bits() == e || (g.is_nan() && f32::from_bits(e).is_nan());
        let (mut seen, mut bad) = ([0usize; 3], Vec::new());
        for line in include_str!("testdata/body_setters_ref.txt").lines() {
            let (lhs, rhs) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = lhs.split(' ').collect();
            let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
            let o: Vec<u32> = rhs.split(' ').map(h).collect();
            let f = |k: usize| f32::from_bits(h(w[k]));
            let mut b = PBody::new(&m, Vec3::ZERO, &Entry::default(), 1.0, 1.0);
            let ok = match w[0] {
                "vel" => {
                    seen[0] += 1;
                    b.mass = f(5);
                    set_velocity(&mut b, Vec3::new(f(1), f(2), f(3)));
                    [b.v.x, b.v.y, b.v.z].iter().zip(&o[0..3]).all(|(g, e)| same(*g, *e))
                        && [b.p.x, b.p.y, b.p.z].iter().zip(&o[4..7]).all(|(g, e)| same(*g, *e))
                }
                "imp" => {
                    seen[1] += 1;
                    b.jf = Vec3::new(f(1), f(2), f(3));
                    apply_impulse(&mut b, Vec3::new(f(5), f(6), f(7)));
                    [b.jf.x, b.jf.y, b.jf.z].iter().zip(&o[0..3]).all(|(g, e)| same(*g, *e))
                }
                "flag" => {
                    seen[2] += 1;
                    o[0] == (h(w[1]) & !3) | 2
                }
                k => panic!("{k}"),
            };
            if !ok { bad.push(line); }
        }
        assert_eq!(seen, [3000, 3000, 500]);
        assert!(bad.is_empty(), "{} mismatches, first {:?}", bad.len(), &bad[..bad.len().min(3)]);
    }

    /// The rope's normal F-72436, SetMass F-31082 and SetInertia F-58768 run offline
    /// (tools/loose_setters_reference.py, flush mode as the SDK sets it): n and -n, the stretch,
    /// the mass and its inverse, the floored inertia and its inverse, every x / y / z word (NaN as
    /// NaN).
    #[test]
    fn rope_normal_and_setters_match_the_original() {
        let w = |s: &str| u32::from_str_radix(&s.replace('_', ""), 16).unwrap();
        let f = |s: &str| f32::from_bits(w(s));
        let same = |x: f32, o: u32| x.to_bits() == o || (x.is_nan() && f32::from_bits(o).is_nan());
        let mut n = [0; 3];
        for line in include_str!("../testdata/loose_setters_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let a: Vec<&str> = i.split_whitespace().collect();
            let o: Vec<u32> = o.split_whitespace().map(w).collect();
            match a[0] {
                "rope" => {
                    let (nn, err) = rope_normal(Vec3::new(f(a[1]), f(a[2]), f(a[3])), Vec3::new(f(a[5]), f(a[6]), f(a[7])), f(a[9]));
                    let m = Vec3::ZERO - nn;
                    for (k, x) in [nn.x, nn.y, nn.z, m.x, m.y, m.z].into_iter().enumerate() {
                        assert!(same(x, o[if k < 3 { k } else { k + 1 }]), "{line} lane {k}");
                    }
                    assert!(same(err, o[8]), "{line}");
                    n[0] += 1;
                }
                "mass" => {
                    let (m, inv) = set_mass(f(a[1]));
                    assert!(same(m, o[0]) && same(inv, o[1]), "{line}");
                    n[1] += 1;
                }
                _ => {
                    let (i, inv) = set_inertia([f(a[1]), f(a[2]), f(a[3])]);
                    for k in 0..3 {
                        assert!(same(i[k], o[k]) && same(inv[k], o[4 + k]), "{line} lane {k}");
                    }
                    n[2] += 1;
                }
            }
        }
        assert_eq!(n, [3000, 1500, 3000]);
    }

    /// The type's mass F-25823 run offline (tools/entry_mass_reference.py): 3,000 child lists of
    /// 0 .. 255, the sum from 0.0 one child after another, every word (NaN as NaN).
    #[test]
    fn entry_mass_matches_the_original() {
        let w = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let mut n = 0;
        for line in include_str!("../testdata/entry_mass_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let a: Vec<u32> = i.split_whitespace().map(w).collect();
            let children = a[1..].iter().map(|&m| mcla_formats::rage::fragphys::FragChild { mass: f32::from_bits(m), ..Default::default() }).collect();
            assert_eq!(a.len() - 1, a[0] as usize);
            let t = FragType { children, ..Default::default() };
            let (m, r) = (entry_mass(&t, 0), f32::from_bits(w(o)));
            assert!(m.to_bits() == w(o) || (m.is_nan() && r.is_nan()), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The inertia floor F-88418 run offline (tools/inertia_floor_reference.py): 5,000 triples
    /// with ties, zeros, negatives, NaN, inf and subnormals, every word.
    #[test]
    fn inertia_floor_matches_the_original() {
        let w = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let mut n = 0;
        for line in include_str!("../testdata/inertia_floor_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let a: Vec<f32> = i.split_whitespace().map(|x| f32::from_bits(w(x))).collect();
            let o: Vec<u32> = o.split_whitespace().map(w).collect();
            let r = inertia_floor([a[0], a[1], a[2]]);
            for k in 0..3 {
                assert!(r[k].to_bits() == o[k] || (r[k].is_nan() && f32::from_bits(o[k]).is_nan()), "{line}");
            }
            n += 1;
        }
        assert_eq!(n, 5000);
    }
    use crate::physics::collision::{SectorMesh, WorldCollider};

    /// The island lists' STLport sort F-52683 / F-18034 run offline whole (tools/stl_sort_reference.py): 3,068 lists
    /// of 0 .. 1,200 contacts by key A and by key B, with few or many equal keys, sorted, reversed, organ pipes and
    /// median-of-three killers that reach the depth limit's heap sort - the order the port leaves equals the original's.
    /// The speed and spin caps F-80759 / F-29166 run offline (tools/capped_reference.py): 4,032 velocities or spins with
    /// their momenta, under and over MaxSpeed 500 / MaxAngSpeed 2 pi and other limits - every word equal (the NaNs of an
    /// overflowing length as NaNs).
    #[test]
    fn caps_match_the_original() {
        let h = |x: &str| f32::from_bits(u32::from_str_radix(x, 16).unwrap());
        let mut n = 0;
        for line in include_str!("../testdata/capped_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let w: Vec<f32> = case.split(' ').skip(1).map(h).collect();
            let (v, p) = capped(Vec3::new(w[0], w[1], w[2]), Vec3::new(w[3], w[4], w[5]), w[6]);
            // (a NaN only as a NaN: the harness runs on x86, whose default NaN ffc00000 the console's 7fc00000 is not)
            let nan = |x: u32| if f32::from_bits(x).is_nan() { 0x7fc0_0000 } else { x };
            let got = [v.x, v.y, v.z, p.x, p.y, p.z].map(|x| format!("{:08x}", nan(x.to_bits()))).join(" ");
            let want = want.split(' ').map(|x| format!("{:08x}", nan(u32::from_str_radix(x, 16).unwrap()))).collect::<Vec<_>>().join(" ");
            assert_eq!(got, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 4032);
    }

    #[test]
    fn stl_sort_matches_the_original() {
        let text = include_str!("../testdata/stl_sort_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (case, want) = line.split_once(" ->").unwrap();
            let mut w = case.split_whitespace();
            w.next();
            let len: usize = w.next().unwrap().parse().unwrap();
            let keys: Vec<i64> = w.map(|x| i64::from_str_radix(x, 16).unwrap()).collect();
            assert_eq!(keys.len(), len);
            let want: Vec<usize> = want.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let mut got: Vec<usize> = (0..len).collect();
            stl_sort(&mut got, |&i| keys[i]);
            assert_eq!(got, want, "{line:.80}");
            n += 1;
        }
        assert_eq!(n, 3068);
    }

    /// The bumper's type as F-48443 makes it.
    #[test]
    fn the_torn_bumper_is_the_originals_box() {
        let t = piece_type();
        let a = &t.archetype;
        assert_eq!((a.mass, a.gravity, a.max_speed, a.type_flags, a.include), (4.0, 1.5, 500.0, 2, 0xFFFF_BFFF));
        // (the box 2.25 x 0.41625 x 0.625: (0.4163^2 + 0.625^2) 4 / 12 ...)
        assert!((a.inertia[0] - 0.187963).abs() < 1e-5 && (a.inertia[1] - 1.817708).abs() < 1e-5 && (a.inertia[2] - 1.745255).abs() < 1e-5, "{:?}", a.inertia);
        let b = t.parts[0].bound.as_ref().unwrap();
        assert_eq!((b.kind, b.max, b.margin, b.cg), (3, [0.9, 0.1665, 0.25], 0.04, [0.0, 0.0, 0.15]));
    }

    /// A piece no bumper holds (its car gone) leaves the level with its body; the held one stays.
    #[test]
    fn a_piece_goes_with_its_car() {
        let world = LooseWorld::new(Vec::new(), Vec::new());
        let mut s = LooseState::default();
        let kept = s.add_piece(&world, [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(10.0, 0.3, 0.0)], Vec3::ZERO, Vec3::ZERO).unwrap();
        let gone = s.add_piece(&world, [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(20.0, 0.3, 0.0)], Vec3::ZERO, Vec3::ZERO).unwrap();
        let o = s.pieces[1].obj;
        assert_eq!(s.keep_pieces(&|id| id == kept), 1);
        assert!(s.piece(kept).is_some() && s.piece(gone).is_none());
        assert!(s.objs[o].body.is_none());
        assert_eq!(s.keep_pieces(&|id| id == kept), 0);
    }

    /// A piece its car has left goes, body and sleep state and all, to the level of the car that comes to it.
    #[test]
    fn a_piece_goes_to_the_car_that_reaches_it() {
        let world = LooseWorld::new(Vec::new(), Vec::new());
        let (mut a, mut b) = (LooseState::default(), LooseState::default());
        let id = a.add_piece(&world, [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(10.0, 0.3, 0.0)], Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO).unwrap();
        {
            let mut cars = [CarLevel { at: Vec3::ZERO, reach: 4.0, level: &mut a }, CarLevel { at: Vec3::new(12.0, 0.0, 0.0), reach: 4.0, level: &mut b }];
            hand_over_pieces(&world, &mut cars);
        }
        assert!(a.piece(id).is_none());
        let (m, _, off) = b.piece(id).unwrap();
        assert!(m[3] == Vec3::new(10.0, 0.3, 0.0) && !off);
        assert_eq!(b.objs.last().unwrap().body.unwrap().v, Vec3::new(1.0, 0.0, 0.0));
        // (the car that holds it and reaches it keeps it)
        let mut cars = [CarLevel { at: Vec3::new(9.0, 0.0, 0.0), reach: 4.0, level: &mut a }, CarLevel { at: Vec3::new(12.0, 0.0, 0.0), reach: 4.0, level: &mut b }];
        hand_over_pieces(&world, &mut cars);
        assert!(b.piece(id).is_some());
    }

    /// GitHub #48: an untouched prop far from everything leaves the level, the near one and a piece stay; every
    /// index held follows the renumbering, and the prop made again is the object it was.
    #[test]
    fn prune_drops_only_far_untouched_props() {
        let ty = FragType { off: 1, radius: 0.5, ..Default::default() };
        let inst = |x: f32| FragInstance { frag: 1, state: 1, matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [x, 0.0, 0.0]], ..Default::default() };
        let world = LooseWorld::new(vec![ty], vec![inst(0.0), inst(100.0)]);
        let mut s = LooseState::default();
        let (a, b) = (s.obj_of(&world, 0), s.obj_of(&world, 1));
        assert_eq!((a, b), (0, 1));
        let before = format!("{:?}", s.objs[1]);
        let id = s.add_piece(&world, [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(200.0, 0.3, 0.0)], Vec3::ZERO, Vec3::ZERO).unwrap();
        let p = s.objs.len() - 1;
        assert_eq!(p, 2);
        s.fx_events.push(FxPlay { effect: "x".into(), at: FxAt::Follow { obj: p, part: None, offset: Vec3::ZERO }, vel: Vec3::ZERO });
        assert_eq!(s.prune(&world, &[Vec3::ZERO], &|_| true), 1);
        assert_eq!(s.objs.len(), 2);
        assert_eq!(s.objs[0].inst, Some(0));
        assert!(matches!(s.fx_events[0].at, FxAt::Follow { obj: 1, .. }));
        assert_eq!(s.pieces[0].obj, 1);
        assert!(s.piece(id).is_some());
        assert_eq!((s.renumbered(0, 2), s.renumbered(0, 1), s.renumbered(0, 0), s.renumbered(1, 1)), (Some(1), None, Some(0), Some(1)));
        // (nothing more to drop: the piece is not a prop, the near prop is near)
        assert_eq!(s.prune(&world, &[Vec3::ZERO], &|_| true), 0);
        // (made again when a car comes near: the same object, at the end)
        let again = s.obj_of(&world, 1);
        assert_eq!(again, 2);
        assert_eq!(format!("{:?}", s.objs[2]), before);
    }

    /// PropManager_ResetProps on the level: a moved, broken prop stands untouched at its file matrix in its place, a piece
    /// broken off one (a cache entry's object) goes with its pair and its effect, a torn bumper and a car stay.
    #[test]
    fn reset_props_puts_the_city_back() {
        let ty = FragType { off: 1, radius: 0.5, ..Default::default() };
        let inst = |x: f32| FragInstance { frag: 1, state: 1, matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [x, 0.0, 0.0]], ..Default::default() };
        let world = LooseWorld::new(vec![ty], vec![inst(0.0), inst(5.0)]);
        let mut s = LooseState::default();
        let (a, b) = (s.obj_of(&world, 0), s.obj_of(&world, 1));
        let fresh = format!("{:?}", s.objs[a]);
        let e = Entry { absent: 1, from: Some(0), ..Default::default() };
        s.objs[a].matrix[3] = Vec3::new(1.0, 0.2, 3.0);
        s.objs[a].entry = Some(e);
        s.objs[a].hit = true;
        s.objs[a].dormant = true;
        // (the piece that broke off it: no instance, the entry's)
        let mut piece = s.objs[a].clone();
        piece.inst = None;
        s.objs.push(piece);
        let p = s.objs.len() - 1;
        let id = s.add_piece(&world, [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(20.0, 0.3, 0.0)], Vec3::ZERO, Vec3::ZERO).unwrap();
        let bumper = s.objs.len() - 1;
        s.fx_events.push(FxPlay { effect: "x".into(), at: FxAt::Follow { obj: p, part: None, offset: Vec3::ZERO }, vel: Vec3::ZERO });
        s.fx_events.push(FxPlay { effect: "y".into(), at: FxAt::Follow { obj: bumper, part: None, offset: Vec3::ZERO }, vel: Vec3::ZERO });
        s.manis.push(Mani { a: Who::Car, b: Who::Obj(p), part_a: 0, part_b: 0, cell: 0, stale: false, visited: true, island: 0, order: 0, slots: Vec::new(), rope: false, owner: 0 });
        s.manis.push(Mani { a: Who::Car, b: Who::Obj(b), part_a: 0, part_b: 0, cell: 0, stale: false, visited: true, island: 0, order: 0, slots: Vec::new(), rope: false, owner: 0 });
        assert_eq!(s.reset_props(&world), (1, 1));
        assert_eq!(s.objs.len(), 3);
        assert_eq!(format!("{:?}", s.objs[a]), fresh);
        assert_eq!(s.objs[b].inst, Some(1));
        assert_eq!(s.pieces[0].obj, 2);
        assert!(s.piece(id).is_some());
        assert_eq!(s.fx_events.len(), 1);
        assert!(matches!(s.fx_events[0].at, FxAt::Follow { obj: 2, .. }));
        // (the untouched prop's pair stays; the gone piece's goes)
        assert_eq!(s.manis.len(), 1);
        assert!(s.manis[0].b == Who::Obj(b));
        // (nothing left to reset)
        assert_eq!(s.reset_props(&world), (0, 0));
    }

    fn flat() -> WorldCollider {
        let mut w = WorldCollider::default();
        let (a, b, c, d) = (Vec3::new(-20.0, 0.0, -20.0), Vec3::new(-20.0, 0.0, 20.0), Vec3::new(20.0, 0.0, -20.0), Vec3::new(20.0, 0.0, 20.0));
        w.sectors.insert(1, SectorMesh::of_bound(vec![[a, b, c], [c, b, d]], vec![Surface::Road as u8, Surface::Road as u8], &[800.0, 800.0], vec![[None, None, None]; 2]));
        w
    }

    /// Dropped from half a metre, the piece comes to rest on the face it lands on, 0.25 m (standing on its 1.8 x
    /// 0.333 face) or 0.1665 m (lying on 1.8 x 0.5) above the ground, and is put to sleep (taken out of the
    /// simulator) about a second later.
    #[test]
    fn a_dropped_piece_lies_still_and_sleeps() {
        let world = LooseWorld::new(Vec::new(), Vec::new());
        let city = flat();
        let hull = HullShape::new(vec![Vec3::new(-1.0, -0.5, -2.0), Vec3::new(1.0, -0.5, -2.0), Vec3::new(0.0, 0.5, -2.0), Vec3::new(0.0, -0.5, 2.0)], &[[0, 1, 2], [0, 3, 1], [1, 3, 2], [2, 3, 0]]);
        let away = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1000.0, 0.0, 1000.0)];
        let car = CarIn { driver: Default::default(), hull: &hull, hull_cg: Vec3::ZERO, hull_margin: 0.04, now: away, last: away, prev: away, speed: 0.0, bound: (hull.min, hull.max), wheels: Vec::new(), vel: Vec3::ZERO, touch: (0.5, 0.1), wiped: false };
        for (rows, height) in [([Vec3::X, Vec3::Z, Vec3::NEG_Y], 0.25f32), ([Vec3::X, Vec3::Y, Vec3::Z], 0.1665)] {
            let mut s = LooseState::default();
            let id = s.add_piece(&world, [rows[0], rows[1], rows[2], Vec3::new(0.0, height + 0.5, 0.0)], Vec3::ZERO, Vec3::ZERO).unwrap();
            let dt = 1.0 / 60.0;
            let mut asleep = None;
            for k in 0..600 {
                s.pre_collide(&world, dt);
                s.collide(&world, &car, &city, false);
                s.refresh(&world, &car);
                s.step_rest(&world, &car, dt);
                s.end_tick(&world, dt);
                if asleep.is_none() && s.piece(id).unwrap().2 {
                    asleep = Some(k);
                }
            }
            let (m, _, off) = s.piece(id).unwrap();
            println!("rest {:?} up rows {:?} asleep at tick {:?}", m[3], (m[0].y, m[1].y, m[2].y), asleep);
            assert!(off && (m[3].y - height).abs() < 0.01, "{:?}", m);
        }
    }

    /// A polyhedron part (bound type 4) meets the city: before F-33272's core was used it had no contact and fell for ever. The
    /// corners are the box's less the margin, so it rests where the box rests.
    #[test]
    fn a_polyhedron_part_rests_on_the_ground() {
        let mut world = LooseWorld::new(Vec::new(), Vec::new());
        let ty = world.piece_type_index().unwrap();
        let b = world.types[ty].parts[0].bound.as_mut().unwrap();
        b.kind = 4;
        let (h, g) = (b.max, b.margin);
        let (x, y, z) = (h[0] - g, h[1] - g, h[2] - g);
        let corners: Vec<Vec3> = [[-x, -y, -z], [x, -y, -z], [-x, y, -z], [x, y, -z], [-x, -y, z], [x, -y, z], [-x, y, z], [x, y, z]].iter().map(|c| v(*c)).collect();
        world.polyhedra.insert((ty, 0), corners);
        let city = flat();
        let hull = HullShape::new(vec![Vec3::new(-1.0, -0.5, -2.0), Vec3::new(1.0, -0.5, -2.0), Vec3::new(0.0, 0.5, -2.0), Vec3::new(0.0, -0.5, 2.0)], &[[0, 1, 2], [0, 3, 1], [1, 3, 2], [2, 3, 0]]);
        let away = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1000.0, 0.0, 1000.0)];
        let car = CarIn { driver: Default::default(), hull: &hull, hull_cg: Vec3::ZERO, hull_margin: 0.04, now: away, last: away, prev: away, speed: 0.0, bound: (hull.min, hull.max), wheels: Vec::new(), vel: Vec3::ZERO, touch: (0.5, 0.1), wiped: false };
        let mut s = LooseState::default();
        let id = s.add_piece(&world, [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(0.0, 0.6, 0.0)], Vec3::ZERO, Vec3::ZERO).unwrap();
        let dt = 1.0 / 60.0;
        for _ in 0..600 {
            s.pre_collide(&world, dt);
            s.collide(&world, &car, &city, false);
            s.refresh(&world, &car);
            s.step_rest(&world, &car, dt);
            s.end_tick(&world, dt);
        }
        let (m, _, _) = s.piece(id).unwrap();
        assert!((m[3].y - h[1]).abs() < 0.01, "{:?}", m);
    }
}

#[cfg(test)]
mod vec_small2_tests {
    use super::*;

    /// A part's matrix in the world F-35644, a bound's centre of gravity in the world F-86352, a rope's anchor carried
    /// to the world F-36793 (through a composite's part first) and the tick's pending momentum change F-24169, run
    /// offline (tools/vec_small2_reference.py): 2,000 / 2,000 / 2,000 / 1,000 cases, every word equal.
    #[test]
    fn small_vector_routines_2_match_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let v3 = |c: &[f32], i: usize| Vec3::new(c[i], c[i + 1], c[i + 2]);
        let m4 = |c: &[f32], i: usize| [v3(c, i), v3(c, i + 3), v3(c, i + 6), v3(c, i + 9)];
        let mut n = 0;
        for line in include_str!("../testdata/vec_small2_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let mut w = case.split(' ');
            let kind = w.next().unwrap();
            let c: Vec<f32> = w.map(|x| f32::from_bits(h(x))).collect();
            let got: Vec<Vec3> = match kind {
                "wm" => super::super::gjk::world_matrix(&m4(&c, 12), &m4(&c, 0)).to_vec(),
                "cg" => vec![if c[0] != 0.0 { xf(&m4(&c, 1), v3(&c, 13)) } else { m4(&c, 1)[3] }],
                "anchor" => {
                    let (inst, part, p) = (m4(&c, 1), m4(&c, 13), v3(&c, 25));
                    vec![if c[0] != 0.0 { xf(&inst, xf(&part, p)) } else { xf(&inst, p) }]
                }
                _ => vec![v3(&c, 0) * c[6] + v3(&c, 3)],
            };
            let got: Vec<u32> = got.iter().flat_map(|v| v.to_array()).map(f32::to_bits).collect();
            let want: Vec<u32> = want.split(' ').map(h).collect();
            assert_eq!(got, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 7000);
    }
}

#[cfg(test)]
mod quarter_weight_tests {
    /// F-60791 run offline (tools/quarter_weight_reference.py): 6,000 masses with each archetype class (Phys / Damp read
    /// +40, the base class's call answers 1.0) and none - every word equal.
    #[test]
    fn quarter_weight_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut n = 0;
        for line in include_str!("../testdata/quarter_weight_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let (kind, mass) = case.split_once(' ').unwrap();
            let mass = match kind {
                "0" => None,
                "3" => Some(1.0),
                _ => Some(f32::from_bits(h(mass))),
            };
            assert_eq!(super::quarter_weight(mass).to_bits(), h(want), "{line}");
            n += 1;
        }
        assert_eq!(n, 6000);
    }
}

#[cfg(test)]
mod xf_tests {
    use glam::Vec3;

    /// The rider x vehicle collide's point transform F-88353 run offline (tools/xf_reference.py): 8,000 frames and
    /// points, the inputs' w lanes filled with garbage - every word equal to `xf`.
    #[test]
    fn xf_matches_the_original() {
        let h = |x: &str| f32::from_bits(u32::from_str_radix(x, 16).unwrap());
        let mut n = 0;
        for line in include_str!("../testdata/xf_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let c: Vec<f32> = case.split(' ').map(h).collect();
            let r = |i: usize| Vec3::new(c[3 * i], c[3 * i + 1], c[3 * i + 2]);
            let got = super::xf(&[r(0), r(1), r(2), r(3)], r(4));
            let want: Vec<u32> = want.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            assert_eq!(got.to_array().map(f32::to_bits).to_vec(), want, "{line}");
            n += 1;
        }
        assert_eq!(n, 8000);
    }
}

#[cfg(test)]
mod amb_wheel_tests {
    use super::*;

    /// F-19974 on the original's code run offline (tools/amb_wheel_reference.py, 4,000 wheels: hits and none, the front
    /// pair and the rest, deflections past the travel, slides): the record (+8 d, +12 s, +16 raw, +20 slide) and every
    /// vector the body is handed (the force, its torque, the push, the tyre's force and its point) equal bit for bit.
    #[test]
    fn ambient_wheel_matches_the_original() {
        let h = |w: &str| u32::from_str_radix(w, 16).unwrap();
        let mut n = 0;
        for l in include_str!("../testdata/amb_wheel_ref.txt").lines() {
            let (c, o) = l.split_once(" -> ").unwrap();
            let w: Vec<u32> = c.split(' ').map(h).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let v3 = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let m = [v3(0), v3(3), v3(6), v3(9)];
            let mut rec = AmbWheel { d: f(12), s: f(13), raw: f(14), slide: w[40] != 0 };
            let t = WheelTune { grip: f(31), travel: f(32), stretch: f(33), cog_sim: f(34), k: f(35), c: f(36), kt: f(37), ct: f(38), mass: f(39) };
            let mut got = String::new();
            let hx = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
            if w[41] != 0 {
                let out = ambient_wheel_contact(&mut rec, &t, &m, w[42] != 0, v3(15), v3(18), v3(21), v3(27), v3(24), f(30));
                got += &format!(" f {} t {}", hx(out.force), hx(out.torque));
                if let Some(p) = out.push {
                    got += &format!(" p {}", hx(p));
                }
                got += &format!(" y {} at {}", hx(out.tyre), hx(v3(15)));
            } else {
                ambient_wheel_air(&mut rec, m[1].y, t.travel, f(30));
            }
            got += &format!(" | {:08x} {:08x} {:08x} {}", rec.d.to_bits(), rec.s.to_bits(), rec.raw.to_bits(), rec.slide as u8);
            assert_eq!(got.trim(), o.trim(), "{l}");
            n += 1;
        }
        assert_eq!(n, 4000);
    }
}

#[cfg(test)]
#[path = "loose_small_tests.rs"]
mod loose_small_tests;

#[cfg(test)]
mod broad_tests {
    use super::*;

    /// testdata/broad_ref.txt (tools/broad_reference.py): the level's broad-phase worker F-84048 run offline with the octree
    /// query stubbed to hand back every object: per active object the sphere it gives the query (centre, radius) and the
    /// pairs its box test writes, against broad_sphere / broad_boxes with the worker's own filter (an active candidate only
    /// above the object's index; the pair written lower index first).
    #[test]
    fn broad_matches_the_original() {
        let (mut nq, mut np, mut nt) = (0, 0, 0);
        for line in include_str!("../testdata/broad_ref.txt").lines() {
            let (case, out) = line.split_once(" -> ").unwrap();
            let h = |s: &str| u32::from_str_radix(s.strip_prefix('x').unwrap(), 16).unwrap();
            let f = |s: &str| f32::from_bits(h(s));
            let mut parts = case.split(" | ");
            let n: usize = parts.next().unwrap().strip_prefix("B ").unwrap().parse().unwrap();
            let list: Vec<usize> = parts.next().unwrap().split_whitespace().map(|x| x.parse().unwrap()).collect();
            let orders: Vec<Vec<usize>> = (0..list.len()).map(|_| parts.next().unwrap().split_whitespace().map(|x| x.parse().unwrap()).collect()).collect();
            let objs: Vec<(BroadSide, bool)> = (0..n)
                .map(|_| {
                    let w: Vec<&str> = parts.next().unwrap().split_whitespace().collect();
                    let v3 = |k: usize| Vec3::new(f(w[k]), f(w[k + 1]), f(w[k + 2]));
                    let rows = |b: usize| [v3(b), v3(b + 4), v3(b + 8), v3(b + 12)];
                    let active = w[45] == "0";
                    ((v3(36), v3(32), v3(40), f(w[44]), rows(0), active.then(|| rows(16))), active)
                })
                .collect();
            let mut want = out.split(" P");
            let qs: Vec<Vec<u32>> = want.next().unwrap().split("Q ").filter(|x| !x.trim().is_empty()).map(|q| q.split_whitespace().map(h).collect()).collect();
            let pairs: Vec<u32> = want.next().unwrap().split_whitespace().map(|x| x.parse().unwrap()).collect();
            assert_eq!(qs.len(), list.len());
            let mut got = Vec::new();
            for ((&me, order), q) in list.iter().zip(&orders).zip(&qs) {
                let s = broad_sphere(&objs[me].0);
                assert_eq!([s[0].to_bits(), s[1].to_bits(), s[2].to_bits(), s[3].to_bits()], [q[0], q[1], q[2], q[4]], "sphere of {me}: {line}");
                nq += 1;
                for &c in order {
                    nt += 1;
                    if objs[c].1 && me >= c {
                        continue;
                    }
                    if broad_boxes(&objs[me].0, &objs[c].0) {
                        got.extend_from_slice(&[me.min(c) as u32, me.max(c) as u32, 0]);
                    }
                }
            }
            assert_eq!(got, pairs, "{line}");
            np += pairs.len() / 3;
        }
        assert!(nq > 500 && np > 500, "{nq} {np} {nt}");
    }
}

#[cfg(test)]
mod pool_tests {
    use super::*;

    /// testdata/pool_evict_ref.txt (tools/pool_evict_reference.py): F-18636 run offline with the pool full and the viewer
    /// sphere set, 1,500 lists of 1 - 24 entries (stamps about the frame, kept types, spheres in and out of the sphere,
    /// equal ones, a few NaN), and F-94385 on 1,500 matrices and spheres: the port's victim and sphere bit for bit.
    /// (Words are written x + 8 hex digits, so float bits are not read as addresses by `registry check`.)
    #[test]
    fn pool_victim_matches_the_original() {
        let (mut nv, mut ns) = (0, 0);
        for line in include_str!("../testdata/pool_evict_ref.txt").lines() {
            let (case, out) = line.split_once(" -> ").unwrap();
            let h = |s: &str| u32::from_str_radix(s.strip_prefix('x').unwrap(), 16).unwrap();
            let f = |s: &str| f32::from_bits(h(s));
            if let Some(rest) = case.strip_prefix("V ") {
                let mut parts = rest.split(" | ");
                let head: Vec<&str> = parts.next().unwrap().split_whitespace().collect();
                let frame = h(head[0]);
                let viewer = [f(head[1]), f(head[2]), f(head[3]), f(head[4])];
                let used: Vec<PoolSeen> = parts
                    .map(|p| {
                        let w: Vec<&str> = p.split_whitespace().collect();
                        PoolSeen { stamp: h(w[0]), keeps: w[1] == "1", sphere: [f(w[2]), f(w[3]), f(w[4]), f(w[5])] }
                    })
                    .collect();
                assert_eq!(used.len(), head[5].parse::<usize>().unwrap());
                let want: i64 = out.trim().parse().unwrap();
                let got = pool_victim(&used, frame, viewer).map_or(-1, |k| k as i64);
                assert_eq!(got, want, "{line}");
                nv += 1;
            } else if let Some(rest) = case.strip_prefix("S ") {
                let w: Vec<f32> = rest.split_whitespace().map(f).collect();
                let m = [Vec3::new(w[0], w[1], w[2]), Vec3::new(w[3], w[4], w[5]), Vec3::new(w[6], w[7], w[8]), Vec3::new(w[9], w[10], w[11])];
                let got = sphere_through(&m, [w[12], w[13], w[14], w[15]]);
                let want: Vec<u32> = out.split_whitespace().map(h).collect();
                assert_eq!(got.map(f32::to_bits).to_vec(), want, "{line}");
                ns += 1;
            }
        }
        assert_eq!((nv, ns), (1500, 1500));
    }

    /// The pool in the level: 160 entries taken with room; the 161st takes the newest stale one outside the viewer
    /// sphere, which leaves the level at the tick's end (its instance `out`: not taken in again) and is back after the
    /// city's reset.
    #[test]
    fn a_full_pool_takes_a_prop_out_until_the_reset() {
        let ty = FragType { off: 1, radius: 0.5, flags: 1, ..Default::default() };
        let inst = |x: f32| FragInstance { frag: 1, state: 1, matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [x, 0.0, 0.0]], ..Default::default() };
        let world = LooseWorld::new(vec![ty], (0..161).map(|i| inst(10.0 * i as f32)).collect());
        assert_eq!(world.kept_entries, 0);
        let mut s = LooseState::default();
        s.set_viewer(Vec3::ZERO);
        for i in 0..160 {
            let o = s.obj_of(&world, i);
            let mut e = Entry::default();
            (e.slot, e.stamp) = s.pool_take(&world);
            assert_eq!(e.slot, i as u64 + 1);
            s.objs[o].entry = Some(e);
            s.objs[o].hit = true;
        }
        assert!(s.objs.iter().all(|o| !o.entry.unwrap().taken));
        s.pool_frame += 2;
        let o = s.obj_of(&world, 160);
        let (slot, stamp) = s.pool_take(&world);
        // (the walk starts with the newest: instance 159 at 1590 m, outside the 300 m sphere)
        assert_eq!((slot, stamp), (160, 2));
        assert!(s.objs[159].entry.unwrap().taken);
        s.objs[o].entry = Some(Entry { slot, stamp, ..Default::default() });
        s.pool_teardown();
        assert_eq!(s.objs.len(), 160);
        assert!(s.out.contains(&159) && s.out.len() == 1);
        assert!(s.objs.iter().all(|o| o.inst != Some(159)));
        // (all within the sphere and stale: the least gap, the farthest - instance 29 at 290 m of 0 .. 29 and 160)
        let mut t = LooseState::default();
        t.set_viewer(Vec3::ZERO);
        for i in (0..30).chain(160..161) {
            let o = t.obj_of(&world, i);
            t.objs[o].entry = Some(Entry { slot: i as u64 + 1, ..Default::default() });
        }
        t.pool_frame = 5;
        let seen: Vec<PoolSeen> = (0..31).rev().map(|k| PoolSeen { stamp: 0, keeps: false, sphere: sphere_through(&t.objs[k].matrix, [0.0, 0.0, 0.0, 0.5]) }).collect();
        assert_eq!(pool_victim(&seen, 5, [0.0, 0.0, 0.0, 300.0]), Some(0));
        assert_eq!(pool_victim(&seen[1..], 5, [0.0, 0.0, 0.0, 300.0]), Some(0));
        assert_eq!(s.reset_props(&world).0, 161);
        assert!(s.out.is_empty());
    }
}

#[cfg(test)]
mod breaks_off_tests {
    /// tools/prop_break_reference.py: F-58598 run offline, one contact a case (the other's type 3 / 4 / 5, the contact on or
    /// off, the car's speed, the prop's least move, its hit bit, its entry and age, the car's part; each test on its edge):
    /// whether the contact is taken from the solver and its manifold reset - 4,000 cases, every one equal.
    #[test]
    fn breaks_off_matches_the_original() {
        let (mut n, mut reset) = (0, 0);
        for line in include_str!("testdata/prop_break_ref.txt").lines() {
            let (case, out) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = case.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let o: Vec<&str> = out.split(' ').collect();
            let f = |i: usize| f32::from_bits(w[i]);
            // (the port reaches the test for an enabled contact of the hull - a wheel's is off before it - with a prop instance)
            let fires = (w[0] == 4 || w[0] == 5) && w[1] != 0 && w[7] == 0 && super::breaks_off(f(2), f(3), w[4] != 0, (w[5] != 0).then(|| f(6)));
            let was = if w[1] != 0 { "c0" } else { "40" };
            let want = if fires { ["40", "0", "-1"] } else { [was, "3", "1"] };
            assert_eq!(o, want, "{line}");
            reset += fires as usize;
            n += 1;
        }
        assert_eq!((n, reset), (4000, 439));
    }
}
