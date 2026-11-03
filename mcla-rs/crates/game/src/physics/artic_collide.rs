//! The thrown rider's collision: his bound, where its parts stand each tick, what they touch of the city, which of
//! his contacts the solver may use, and the body-fall sound's trigger. Decode and checks:
//! work/ragdoll/contact/rider_contacts.md (bike_rider_ragdoll.md section 5, ragdoll_link_joint_build.md 7).
//!
//! - The bound is the fragment type's composite (T+0xec): one part per child, 17 capsules and four boxes for the
//!   rider ([`mcla_formats::rage::ragdoll::RagPart`]). Its run-time copy keeps the part matrices: +132 now, +136 the
//!   previous update's.
//! - Per tick the slot-5 move rewrites +132 (F-15571: the rule of F-43257 without its copy into +136); rd+16 is the
//!   collider's place less the bound's centre offset (F-18040). The collision takes each part to the world with the
//!   instance matrix (F-35644): now = +132 x rd+16, last = +136 x the collider's +96 (the city has no collider) or +32.
//! - Against the city each part goes through the convex-part-against-cell narrow phase the hulls and props use
//!   (F-82759 -> F-36478 -> F-29567 / F-35118, [`super::hull::part_touches`]); the rider's instance flag word +10 is 0,
//!   so the pair's flag byte is 0 and every polygon takes the discrete detector. One manifold per (part, cell), at most
//!   six contacts, kept from tick to tick (F-90136 sink, F-74262 nearest, F-26862 / F-33100 refresh).
//! - Materials (F-43430): friction max(fA fB, 0), elasticity min(eA eB, 1); the rider's own contact callback (F-22759)
//!   then writes the type's +320 (0.0) into every contact of its run as the elasticity.
//! - Own bike (F-37566): contacts with the bike the rider came off (rd+164) are disabled until three runs of contacts
//!   in a row have none. Body fall (F-15373 -> F-92308): once per ragdoll, on the first run without an own-bike contact.
//!   A "run" is one call per instance from the island's contact lists (F-97446 / F-15350): first the contacts where the
//!   rider is instance A, then those where he is B.
//! - Self pairs (F-41849): the collider's pair list (F-83435, +724 / +732), gated by collider +740 (set for the rider)
//!   and the +744 count-down; each pair's swept boxes in B's frame (F-93803) through the fifteen-axis box test
//!   (F-37533), the sub-manifolds merged by (part B, part A), then F-36478 (the swept test F-98094 on an empty
//!   manifold whose parts moved far, else the detector) and the sink with the seventh contact (F-55965)
//!   ([`RiderSelf`]; docs/reference/vehicle/ragdoll_self_contacts.md). Their rows are of type 13 (solver.rs).
//!
//! Open (not decoded, nothing guessed here): the rider against the bike's and other cars' parts (the same per-part-pair
//! path: the bike's bound parts and matrices per tick are needed), the sink's element ids (+168 / +170: the old
//! contact's kept when it was the deeper - not modelled, the port takes the fresh contact's polygon), the box parts'
//! material id in the file (the recorded friction gives 1.0, the ragdoll material's; [`PartShape::surface`]), lanes w
//! of the F-15571 part matrices.

use super::artic::V4;
use super::artic_pose::{self, LinkFrame, Mat, Parts};
use super::collision::{surface_touch, Surface, WorldCollider};
use super::exact::{dot3, fma};
use super::gjk::{world_matrix, PartCore};
use super::hull::{part_touches, PartHit};
use super::loose::xf;
use super::solver::{Side, Touch};
use glam::Vec3;
use mcla_formats::rage::ragdoll::RagdollType;

/// A part of the rider's bound as the narrow phase takes it.
#[derive(Clone, Copy, Debug)]
pub struct PartShape {
    /// bound +4: 1 capsule, 3 box (0 sphere, read the same way)
    pub kind: u8,
    /// the detector's core in the part's frame: the centroid, a capsule's length along y, a box's half sizes less
    /// its margin (as [`super::gjk::PartCore`] holds them)
    pub centroid: Vec3,
    pub half: Vec3,
    /// what the detector adds: a sphere's or capsule's radius (+0x80), a box's margin (+0x70)
    pub margin: f32,
    /// the part's box in its own frame (+0x20 / +0x10): what a polygon has to cut (F-55826)
    pub bbox: (Vec3, Vec3),
    /// the part's material: the capsules' id 31 = "ragdoll"; the boxes' id is not read (their recorded contacts
    /// carry friction 1.0 x the polygon's, the ragdoll material's)
    pub surface: Surface,
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn xyz(v: V4) -> Vec3 {
    Vec3::new(v.0[0], v.0[1], v.0[2])
}

/// A guest matrix as the collision takes it (lanes x y z).
pub fn rows3(m: &Mat) -> [Vec3; 4] {
    [xyz(m[0]), xyz(m[1]), xyz(m[2]), xyz(m[3])]
}

/// The parts of the type's bound; `table` the game's material table ([`Surface::table`]). None: no bound for that
/// child, or a kind the rider does not have.
pub fn shapes(t: &RagdollType, table: &[Surface]) -> Vec<Option<PartShape>> {
    let surface_of = |id: Option<u32>| match id {
        Some(id) => table.get((id & 0xff) as usize).copied().unwrap_or(Surface::Default),
        None => Surface::Ragdoll,
    };
    t.parts
        .iter()
        .map(|p| {
            let (half, margin) = match p.kind {
                0 => (Vec3::ZERO, p.round),
                1 => (Vec3::splat(p.length), p.round),
                // (A-32016: half the box less the margin)
                3 => ((v3(p.max) - v3(p.min)) * 0.5 - Vec3::splat(p.margin), p.margin),
                _ => return None,
            };
            Some(PartShape { kind: p.kind, centroid: v3(p.centroid), half, margin, bbox: (v3(p.min), v3(p.max)), surface: surface_of(p.material) })
        })
        .collect()
}

impl PartShape {
    /// The detector's core at the part's world matrix.
    pub fn core(&self, m: &[Vec3; 4]) -> PartCore {
        PartCore { kind: self.kind, centroid: self.centroid, half: self.half, rows: [m[0], m[1], m[2]], at: m[3] }
    }
}

/// rd+16, the instance matrix (F-18040): the collider's rows, its place less the bound's centre offset (bound +80 =
/// T+32) carried through them.
pub fn instance_matrix(rows: &[V4; 3], place: V4, offset: V4) -> [Vec3; 4] {
    let r = [xyz(rows[0]), xyz(rows[1]), xyz(rows[2])];
    [r[0], r[1], r[2], super::exact::origin_of(xyz(place), &r, xyz(offset))]
}

/// The slot-5 move's part matrices (F-15571): the rule of F-43257 (`part_i = O_i x link`, in rd+16's frame) without
/// its copy of +132 into +136, so `parts.prev` stays the last update's. Lanes x y z as the original's; lane w not
/// checked.
pub fn moved_parts(t: &RagdollType, links: &[LinkFrame], child_link: &[i32], place: V4, r: &Mat, absent: &[bool], parts: &mut Parts) -> Result<(), artic_pose::PoseOpen> {
    let prev = std::mem::take(&mut parts.prev);
    let out = artic_pose::tick_parts(t, links, child_link, place, r, absent, parts);
    parts.prev = prev;
    out
}

/// Every part's world matrices as the collision forms them (F-35644): now = +132 x the instance's matrix now, last =
/// +136 x `last` (the collider's +96 against an object without a collider - the city -, else its +32).
pub fn world_parts(parts: &Parts, now: &[Vec3; 4], last: &[Vec3; 4]) -> (Vec<[Vec3; 4]>, Vec<[Vec3; 4]>) {
    let w = |ps: &[Mat], m: &[Vec3; 4]| ps.iter().map(|p| world_matrix(&rows3(p), m)).collect::<Vec<_>>();
    (w(&parts.cur, now), w(&parts.prev, last))
}

/// One contact slot of a rider manifold (the 176-byte record's fields the port keeps).
#[derive(Clone, Copy, Debug)]
pub struct RiderSlot {
    pub touch: Touch,
    /// +174
    pub age: u16,
    /// +165 bit 0x80
    pub enabled: bool,
    /// B's element (+170): the polygon
    pub polygon: u32,
    /// the polygon's material
    pub surface: Surface,
}

/// The contacts of one part with one cell of the city (a sub-manifold of the rider's pair with the city).
#[derive(Clone, Debug)]
pub struct RiderManifold {
    pub part: u8,
    pub cell: usize,
    pub slots: Vec<RiderSlot>,
}

/// A contact handed to the solver, with the link it belongs to (collider +716 of its part).
#[derive(Clone, Copy, Debug)]
pub struct RiderTouch {
    pub part: u8,
    pub link: usize,
    pub touch: Touch,
}

/// The rider's manifolds with the city, kept from one tick to the next.
#[derive(Clone, Debug, Default)]
pub struct RiderCity {
    pub manifolds: Vec<RiderManifold>,
}

/// F-74262's reach (4e-4 = 2 cm squared) and the refresh's drop depth (F-26862: -0.04).
const NEAR2: f32 = f32::from_bits(0x39d1_b717);
const DROP_DEPTH: f32 = f32::from_bits(0xbd23_d70a);
/// The refresh's least age for the drift test (a global of the game's simulator, read as 1 in ro.0).
const DRIFT_AGE: u16 = 1;
/// The composite's margin (+0x70) and the city grid's as the island's sides carry them (as loose.rs gives them).
const SIDE_MARGIN: f32 = 0.04;

impl RiderCity {
    /// The collide of one tick (F-82759 for the rider's parts against the city's cells): every present part's
    /// contacts, new ones at age 0, through the sink (F-90136): the nearest kept contact (age != 0) within 2 cm by A's
    /// local point is replaced (its multipliers stay), else appended up to six, else F-55965 picks the one it replaces
    /// ([`sink`]; no seventh city contact in ro.0: unchecked there).
    /// A manifold that ends the collide empty is let go. Manifolds in the order (cell, part).
    pub fn collide(&mut self, shapes: &[Option<PartShape>], now: &[[Vec3; 4]], last: &[[Vec3; 4]], city: &WorldCollider) {
        let mut seen: Vec<(usize, u8)> = Vec::new();
        for (p, s) in shapes.iter().enumerate() {
            let Some(s) = s else { continue };
            let mut hits: Vec<PartHit> = Vec::new();
            part_touches(&s.core(&now[p]), s.margin, s.bbox, &now[p], &last[p], city, None, |h| hits.push(h));
            for h in hits {
                let part = p as u8;
                if !seen.contains(&(h.cell, part)) {
                    seen.push((h.cell, part));
                }
                let k = match self.manifolds.iter().position(|m| m.part == part && m.cell == h.cell) {
                    Some(k) => k,
                    None => {
                        let at = self.manifolds.iter().position(|m| (m.cell, m.part) > (h.cell, part)).unwrap_or(self.manifolds.len());
                        self.manifolds.insert(at, RiderManifold { part, cell: h.cell, slots: Vec::new() });
                        at
                    }
                };
                let fresh = new_touch(&h, &now[p]);
                let slot = RiderSlot { touch: fresh, age: 0, enabled: true, polygon: h.polygon, surface: h.surface };
                sink(&mut self.manifolds[k].slots, slot);
            }
        }
        self.manifolds.retain(|m| !m.slots.is_empty());
    }

    /// The update's refresh of every contact (F-66632 matrices, F-26862 / F-33100): kept contacts (age != 0) get their
    /// world points again from their local points (A by the part's matrix now, B the city's own point) and the depth
    /// along the stored minus normal; every contact ages by one; then those with depth not above -0.04 go, and of the
    /// older ones the one drifted farthest past 2 cm goes. `now`: the parts' world matrices now.
    pub fn refresh(&mut self, now: &[[Vec3; 4]]) {
        for m in self.manifolds.iter_mut() {
            let ma = now[m.part as usize];
            for s in m.slots.iter_mut().rev() {
                let t = &mut s.touch;
                if s.age != 0 {
                    t.a.point = xf(&ma, t.a.local);
                    t.b.point = xf(&[Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO], t.b.local);
                    t.depth = dot3(t.a.point - t.b.point, t.back);
                }
                t.target = Vec3::ZERO;
                s.age += 1;
            }
            let (mut worst, mut k_worst) = (0.0f32, None);
            let mut i = 0;
            while i < m.slots.len() {
                let t = m.slots[i].touch;
                if !(t.depth > DROP_DEPTH) {
                    m.slots.swap_remove(i);
                    continue;
                }
                if m.slots[i].age > DRIFT_AGE {
                    let q = Vec3::ZERO - (t.back * t.depth - t.a.point);
                    let s = t.b.point - q;
                    let d2 = dot3(s, s);
                    if !(worst >= d2) && !(NEAR2 >= d2) {
                        worst = d2;
                        k_worst = Some(i);
                    }
                }
                i += 1;
            }
            if let Some(k) = k_worst {
                m.slots.swap_remove(k);
            }
        }
    }

    /// The materials of every contact (F-43430: friction max(fA fB, 0), elasticity min(eA eB, 1), the part's
    /// material and the polygon's), then the rider's callback (F-22759): elasticity = the type's +320.
    pub fn materials(&mut self, shapes: &[Option<PartShape>], type_elasticity: f32) {
        for m in self.manifolds.iter_mut() {
            let sa = shapes.get(m.part as usize).copied().flatten().map_or(Surface::Ragdoll, |s| s.surface);
            let (fa, ea) = surface_touch(sa);
            for s in m.slots.iter_mut() {
                let (fb, eb) = surface_touch(s.surface);
                s.touch.friction = (fa * fb).max(0.0);
                s.touch.elasticity = (ea * eb).min(1.0);
                s.touch.elasticity = type_elasticity;
                s.touch.impulse = Vec3::ZERO;
            }
        }
    }

    /// The enabled contacts for the solver, each with its link (`child_link` = collider +716).
    pub fn touches(&self, child_link: &[i32], n_links: usize) -> Vec<RiderTouch> {
        let mut out = Vec::new();
        for m in &self.manifolds {
            let link = artic_pose::link_of(child_link, m.part as usize, n_links);
            for s in m.slots.iter().filter(|s| s.enabled) {
                out.push(RiderTouch { part: m.part, link, touch: s.touch });
            }
        }
        out
    }
}

/// The record the sink makes of a hit (F-34223): A the rider's part (its point in the part's frame), B the city (its
/// point in the world); the normal pushes A, the stored minus normal beside it; depth = minus the distance.
fn new_touch(h: &PartHit, now: &[Vec3; 4]) -> Touch {
    let a = Side { body: None, point: h.on_a, local: h.local, rows: [now[0], now[1], now[2]], margin: SIDE_MARGIN, core: Vec3::ZERO };
    let b = Side { body: None, point: h.on_b, local: h.on_b, rows: [Vec3::X, Vec3::Y, Vec3::Z], margin: SIDE_MARGIN, core: Vec3::ZERO };
    Touch { depth: h.depth, a, b, normal: h.normal, back: Vec3::ZERO - h.normal, ..Default::default() }
}

/// The own-bike filter (F-37566, rd vtable +92) and the body-fall trigger (F-15373, +100) of one ragdoll: rd+169,
/// +172, +170. GoRagdoll clears +169 / +172; only the recovery states' exits clear +170 ([`ContactGate::rearm`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContactGate {
    /// rd+169: the filter has stopped for good
    pub stopped: bool,
    /// rd+172: runs in a row without an own-bike contact
    pub clear_runs: u32,
    /// rd+170: the body-fall sound has played
    pub fell: bool,
}

impl ContactGate {
    /// GoRagdoll (F-81936): +169 = +172 = 0.
    pub fn go_ragdoll(&mut self) {
        self.stopped = false;
        self.clear_runs = 0;
    }

    /// The recovery exits (F-29164 / F-41940) and F-29285: +170 = 0.
    pub fn rearm(&mut self) {
        self.fell = false;
    }

    /// F-37566 over one run: `others` the other instance's level index of each contact (F-78981; the rider himself for
    /// a self contact), `enabled` their +165 bit 0x80. While the filter runs, every contact with the own bike
    /// (`own` = rd+164, a u16 compared with the signed word: -1 matches none) is disabled; a run without one counts,
    /// one with one resets the count, and the third in a row stops the filter.
    pub fn own_bike_run(&mut self, own: i32, others: &[u16], enabled: &mut [bool]) {
        if self.stopped {
            return;
        }
        let mut found = false;
        for (k, &o) in others.iter().enumerate() {
            if o as i32 == own {
                found = true;
                enabled[k] = false;
            }
        }
        if found {
            self.clear_runs = 0;
        } else {
            self.clear_runs += 1;
            if self.clear_runs >= 3 {
                self.stopped = true;
            }
        }
    }

    /// F-15373 over one run: when the sound has not played and no contact of the run is with the own bike (an empty
    /// run included), it plays now (F-92308: BIKER_BODYFALL_MASTER at rd+64, the instance matrix's position) - the
    /// answer is that position - and +170 is set.
    pub fn body_fall_run(&mut self, own: i32, others: &[u16], rd_position: Vec3) -> Option<Vec3> {
        if self.fell || others.iter().any(|&o| o as i32 == own) {
            return None;
        }
        self.fell = true;
        Some(rd_position)
    }
}

/// The self pairs of a collide (F-41849): None while collider +740 is clear or its +744 count-down runs (which it
/// steps); else the pair list (+724 / +732, F-83435) up to the first -1, for [`RiderSelf::collide`].
pub fn self_pairs(enabled: bool, countdown: &mut i32, pair_a: &[i32], pair_b: &[i32]) -> Option<Vec<(usize, usize)>> {
    if !enabled {
        return None;
    }
    if *countdown > 0 {
        *countdown -= 1;
        return None;
    }
    Some(pair_a.iter().zip(pair_b).take_while(|(a, _)| **a != -1).map(|(a, b)| (*a as usize, *b as usize)).collect())
}

// ---------------------------------------------------------------------------------------------
// Self pairs: the rider's parts against each other (F-41849 -> F-93803, F-37533 -> F-36478)

/// The 1e-6 F-37533 adds to every |R| (0x827D8510).
const OBB_EPS: f32 = f32::from_bits(0x3586_37bd);

/// F-37533: do two boxes overlap - B's frame the reference, `rel` = A's three axes in B's frame (rows) and the
/// centre of A less that of B in B's frame (row 3), as F-93803 forms them; `ha`, `hb` the half sizes. The fifteen
/// separating axes: A's three faces and B's three (both in one go, then out if any is apart), then the nine edge
/// pairs; every |R| term carries the 1e-6. Lane by lane as the vector code orders its sums (decode:
/// docs/reference/vehicle/ragdoll_self_contacts.md 2).
pub fn boxes_overlap(rel: &[Vec3; 4], ha: Vec3, hb: Vec3) -> bool {
    let (r, t) = ([rel[0], rel[1], rel[2]], rel[3]);
    let a: [Vec3; 3] = r.map(|x| x.abs() + Vec3::splat(OBB_EPS));
    // d = minus the centre offset along A's axes (0 - vmsum3)
    let d = Vec3::new(0.0 - dot3(r[0], t), 0.0 - dot3(r[1], t), 0.0 - dot3(r[2], t));
    let col = |m: &[Vec3; 3], k: usize| Vec3::new(m[0][k], m[1][k], m[2][k]);
    // A's faces and B's faces
    let face_a = (0..3).all(|i| dot3(a[i], hb) + ha[i] > d[i].abs());
    let face_b = (0..3).all(|k| dot3(col(&a, k), ha) + hb[k] > dot3(col(&r, k), d).abs());
    if !(face_a && face_b) {
        return false;
    }
    // The edges A_i x B_k, lane k: B's part (hB_k1 a_i[k2] + hB_k2 a_i[k1], the three-term chain the code runs with
    // the lane's own term 0 x a_i[k]) plus A's (column k of a with A's other two half sizes), against column k of R
    // with d's other two lanes, signed.
    let hb_w = Vec3::new(0.0, hb.z, hb.y); // vpermwi 231 (lane w of hB is 0)
    let hb_z = Vec3::new(hb.z, 0.0, hb.x); // vpermwi 179
    let hb_y = Vec3::new(hb.y, hb.x, 0.0); // vpermwi 79
    let chain = |ai: Vec3| {
        let t1 = hb_w * ai.x;
        let t2 = hb_z * ai.y + t1;
        hb_y * ai.z + t2
    };
    // A axis 0: (0, hA.z, hA.y) and (0, d.z, -d.y); axis 1: (hA.z, 0, hA.x) and (-d.z, 0, d.x); axis 2: (hA.y, hA.x,
    // 0) and (d.y, -d.x, 0)
    let sides = [
        (Vec3::new(0.0, ha.z, ha.y), Vec3::new(0.0, d.z, -d.y)),
        (Vec3::new(ha.z, 0.0, ha.x), Vec3::new(-d.z, 0.0, d.x)),
        (Vec3::new(ha.y, ha.x, 0.0), Vec3::new(d.y, -d.x, 0.0)),
    ];
    sides.iter().enumerate().all(|(i, (h, e))| {
        let b_part = chain(a[i]);
        (0..3).all(|k| b_part[k] + dot3(col(&a, k), *h) > dot3(col(&r, k), *e).abs())
    })
}

/// F-93803: B's frame (rows, the point `pb`) the reference for A's (rows, `pa`): A's rows and pa - pb in B's frame.
pub fn relative_frame(b: &[Vec3; 4], pb: Vec3, a: &[Vec3; 4], pa: Vec3) -> [Vec3; 4] {
    let inb = |v: Vec3| Vec3::new(dot3(b[0], v), dot3(b[1], v), dot3(b[2], v));
    [inb(a[0]), inb(a[1]), inb(a[2]), inb(pa - pb)]
}

/// F-41849's box of one part: the part's own box (bound +140 = the part's min / max) grown to hold itself at its last
/// matrix too, in the part's frame now. Gives the box's middle in the world and its half sizes. `now` / `last` the
/// part's world matrices (+132 x rd+16, +136 x collider +32).
pub fn swept_part_box(now: &[Vec3; 4], last: &[Vec3; 4], min: Vec3, max: Vec3) -> (Vec3, Vec3) {
    let c = (max + min) * 0.5;
    let h = (max - min) * 0.5;
    let q = last[3] - now[3];
    let (mut hi, mut lo) = (Vec3::ZERO, Vec3::ZERO);
    for i in 0..3 {
        // the last box's centre and reach along now's axis i (vmsum4 with lane w the offset, vmsum3)
        let r = [dot3(now[i], last[0]), dot3(now[i], last[1]), dot3(now[i], last[2])];
        let p = [r[0] * c.x, r[1] * c.y, r[2] * c.z, dot3(now[i], q)];
        let c2 = (p[3] + p[2]) + (p[1] + p[0]);
        let e2 = dot3(Vec3::new(r[0].abs(), r[1].abs(), r[2].abs()), h);
        hi[i] = (c[i] + h[i]).max(c2 + e2);
        lo[i] = (c[i] - h[i]).min(c2 - e2);
    }
    let mid = (hi + lo) * 0.5;
    let half = hi - mid;
    let w = |k: usize| {
        let p = [now[0][k] * mid.x, now[1][k] * mid.y, now[2][k] * mid.z, now[3][k]];
        (p[3] + p[2]) + (p[1] + p[0])
    };
    (Vec3::new(w(0), w(1), w(2)), half)
}

/// The contacts of one self pair (a sub-manifold of the rider's pair with himself).
#[derive(Clone, Debug)]
pub struct SelfManifold {
    pub a: u8,
    pub b: u8,
    pub slots: Vec<RiderSlot>,
}

/// The rider's sub-manifolds with himself, kept from one tick to the next (the manifold's lists +192 / +712, in the
/// order the pair list passed them).
#[derive(Clone, Debug, Default)]
pub struct RiderSelf {
    pub manifolds: Vec<SelfManifold>,
}

/// F-36478's motion test (byte 0x828127D6 is set in the game): the two parts' moves since last (their matrices'
/// points) differ along one of either part's axes by more than that part's half box: the pair takes the swept test.
fn moved_far(a_now: &[Vec3; 4], a_last: &[Vec3; 4], a_box: (Vec3, Vec3), b_now: &[Vec3; 4], b_last: &[Vec3; 4], b_box: (Vec3, Vec3)) -> bool {
    let m = (a_now[3] - a_last[3]) - (b_now[3] - b_last[3]);
    let ha = (a_box.1 - a_box.0) * 0.5;
    let hb = (b_box.1 - b_box.0) * 0.5;
    (0..3).any(|i| dot3(a_now[i], m).abs() > ha[i] || dot3(b_now[i], m).abs() > hb[i])
}

impl RiderSelf {
    /// The self pairs of one collide (F-41849, after the gate [`self_pairs`]): per pair of the list the two parts'
    /// swept boxes ([`swept_part_box`]) in B's frame (F-93803) through the box test (F-37533). A pair that passes
    /// keeps its sub-manifold when the old list holds it at the merge's place, else gets a new one; an old
    /// sub-manifold the merge passes by is let go (the merge runs by (part B, part A) as the original's does, so a
    /// pair list in another order can let a kept one go). Then F-36478: with no contact yet and the parts moved far
    /// ([`moved_far`]) the swept test (F-98094), else the detector (F-47540); the sink ([`sink`]).
    pub fn collide(&mut self, pairs: &[(usize, usize)], shapes: &[Option<PartShape>], now: &[[Vec3; 4]], last: &[[Vec3; 4]]) {
        let old = std::mem::take(&mut self.manifolds);
        let mut k = 0usize;
        let less = |m: &SelfManifold, a: usize, b: usize| (m.b as usize) < b || (m.b as usize == b && (m.a as usize) < a);
        for &(pa, pb) in pairs {
            let (Some(sa), Some(sb)) = (shapes.get(pa).copied().flatten(), shapes.get(pb).copied().flatten()) else { continue };
            let (ma, ha) = swept_part_box(&now[pa], &last[pa], sa.bbox.0, sa.bbox.1);
            let (mb, hb) = swept_part_box(&now[pb], &last[pb], sb.bbox.0, sb.bbox.1);
            let rel = relative_frame(&now[pb], mb, &now[pa], ma);
            if !boxes_overlap(&rel, ha, hb) {
                continue;
            }
            while k < old.len() && less(&old[k], pa, pb) {
                k += 1;
            }
            let mut m = if k < old.len() && old[k].a as usize == pa && old[k].b as usize == pb {
                k += 1;
                old[k - 1].clone()
            } else {
                SelfManifold { a: pa as u8, b: pb as u8, slots: Vec::new() }
            };
            self_pair_contact(&mut m, &sa, &sb, &now[pa], &last[pa], &now[pb], &last[pb]);
            self.manifolds.push(m);
        }
    }

    /// The update's refresh (F-26862 / F-33100) as for the city, B's point from B's part now.
    pub fn refresh(&mut self, now: &[[Vec3; 4]]) {
        for m in self.manifolds.iter_mut() {
            refresh_slots(&mut m.slots, &now[m.a as usize], &now[m.b as usize]);
        }
    }

    /// The materials (F-43430, both parts' own) and the rider's callback (F-22759: elasticity = the type's +320).
    pub fn materials(&mut self, shapes: &[Option<PartShape>], type_elasticity: f32) {
        let sur = |p: u8| shapes.get(p as usize).copied().flatten().map_or(Surface::Ragdoll, |s| s.surface);
        for m in self.manifolds.iter_mut() {
            let ((fa, ea), (fb, eb)) = (surface_touch(sur(m.a)), surface_touch(sur(m.b)));
            for s in m.slots.iter_mut() {
                s.touch.friction = (fa * fb).max(0.0);
                s.touch.elasticity = (ea * eb).min(1.0);
                s.touch.elasticity = type_elasticity;
                s.touch.impulse = Vec3::ZERO;
            }
        }
    }
}

/// A vehicle's parts as the rider's pair with it takes them: part 0 its hull, parts 1.. its wheels.
pub struct VehSide<'a> {
    pub hull: &'a super::hull::HullShape,
    /// the hull bound's centre of gravity and margin (+0x70)
    pub hull_cg: Vec3,
    pub hull_margin: f32,
    /// the vehicle's matrix now and last: both instances of the pair have a collider record, so each side's last is
    /// its collider's +32, the matrix it had as the tick began (F-65617, as loose.rs takes two bodies)
    pub now: [Vec3; 4],
    pub last: [Vec3; 4],
    /// the wheels' parts (their now / last already in the world)
    pub wheels: Vec<Option<super::loose::WheelIn>>,
}

impl VehSide<'_> {
    /// Part `p`'s core, margin, centre, box, now and last.
    fn part(&self, p: usize) -> Option<(super::loose::Core<'_>, f32, Vec3, (Vec3, Vec3), [Vec3; 4], [Vec3; 4])> {
        use super::gjk::HullCore;
        use super::loose::Core;
        if p == 0 {
            let n = self.now;
            let core = Core::Hull(HullCore { corners: self.hull.scanned(), rows: [n[0], n[1], n[2]], at: n[3] });
            // (the hull's box as loose.rs takes it for the part cull)
            let bbox = (self.hull.min - Vec3::splat(0.041), self.hull.max + Vec3::splat(0.041));
            return Some((core, self.hull_margin, self.hull_cg, bbox, n, self.last));
        }
        let w = self.wheels.get(p - 1).copied().flatten()?;
        Some((Core::Wheel(w.core), super::gjk::WHEEL_MARGIN, Vec3::ZERO, (Vec3::ZERO - w.half, w.half), w.now, w.last))
    }

    fn parts(&self) -> usize {
        1 + self.wheels.len()
    }
}

/// The contacts of one vehicle part with one rider part (a sub-manifold of the rider's pair with the vehicle).
#[derive(Clone, Debug)]
pub struct VehManifold {
    /// the vehicle's part (0 the hull) and the rider's
    pub veh: u8,
    pub part: u8,
    /// the vehicle's element (+168 or +170: the hull's polygon) and its material ride in each slot's `polygon` /
    /// `surface`
    pub slots: Vec<RiderSlot>,
}

/// The rider's sub-manifolds with a vehicle (his own bike), kept from one tick to the next. `rider_is_a`: the pair's
/// A is the instance of the lower level index (the rider's above the bike's in every recording: the bike is A).
#[derive(Clone, Debug, Default)]
pub struct RiderVeh {
    pub manifolds: Vec<VehManifold>,
    pub rider_is_a: bool,
}

impl RiderVeh {
    /// The collide of the rider's parts against a vehicle's (F-82759, composite against composite): per (B's part,
    /// A's part) the parts' swept boxes ([`super::loose::parts_meet`]); a pair that passes keeps its sub-manifold
    /// (an old one the cull passes by is let go), then the narrow phase of the self pairs' collision object (manifold
    /// +1252 = the same object): the detector (F-47540, see [`veh_pair_contact`] for why never the swept test here),
    /// nothing farther than 2 cm, the sink ([`sink`]); a sub-manifold that ends the collide empty is let go. The
    /// hull's element is the polygon the detector's corners and the contact's direction pick (as loose.rs's pairs
    /// with the car). Checked: rs.0 154 - 155 and rt.0 156 - 175, both sides as A, 70 / 70 new contacts and the
    /// polygons word for word (`tests::bike_contacts_match_recording`).
    pub fn collide(&mut self, shapes: &[Option<PartShape>], now: &[[Vec3; 4]], last: &[[Vec3; 4]], veh: &VehSide) {
        let old = std::mem::take(&mut self.manifolds);
        let (n_a, n_b) = if self.rider_is_a { (shapes.len(), veh.parts()) } else { (veh.parts(), shapes.len()) };
        for pb in 0..n_b {
            for pa in 0..n_a {
                let (rp, vp) = if self.rider_is_a { (pa, pb) } else { (pb, pa) };
                let Some(rs) = shapes.get(rp).copied().flatten() else { continue };
                let Some((vcore, vmargin, vcg, vbox, vnow, vlast)) = veh.part(vp) else { continue };
                if !super::loose::parts_meet((rs.bbox, now[rp], last[rp]), (vbox, vnow, vlast)) {
                    continue;
                }
                let mut m = match old.iter().find(|m| m.veh as usize == vp && m.part as usize == rp) {
                    Some(m) => m.clone(),
                    None => VehManifold { veh: vp as u8, part: rp as u8, slots: Vec::new() },
                };
                let rcore = super::loose::Core::Part(rs.core(&now[rp]));
                let r_side = (rcore, rs.margin, rs.centroid, rs.bbox, now[rp], last[rp]);
                let v_side = (vcore, vmargin, vcg, vbox, vnow, vlast);
                let (a, b) = if self.rider_is_a { (&r_side, &v_side) } else { (&v_side, &r_side) };
                veh_pair_contact(&mut m, a, b, !self.rider_is_a, veh, vp);
                self.manifolds.push(m);
            }
        }
        // (a sub-manifold that ends the collide empty is let go, as the city's: rs.0 154 - 156)
        self.manifolds.retain(|m| !m.slots.is_empty());
    }

    /// The update's refresh (F-26862 / F-33100), A's and B's points from their parts now. `veh_now`: the vehicle's
    /// part matrices now (0 the hull).
    pub fn refresh(&mut self, now: &[[Vec3; 4]], veh_now: &[[Vec3; 4]]) {
        for m in self.manifolds.iter_mut() {
            let (r, v) = (now[m.part as usize], veh_now[m.veh as usize]);
            let (ma, mb) = if self.rider_is_a { (r, v) } else { (v, r) };
            refresh_slots(&mut m.slots, &ma, &mb);
        }
    }

    /// The materials (F-43430: the rider part's and the vehicle element's) and the rider's callback (F-22759: the
    /// type's +320 on every contact handed to it).
    pub fn materials(&mut self, shapes: &[Option<PartShape>], type_elasticity: f32) {
        for m in self.manifolds.iter_mut() {
            let sr = shapes.get(m.part as usize).copied().flatten().map_or(Surface::Ragdoll, |s| s.surface);
            let (fr, er) = surface_touch(sr);
            for s in m.slots.iter_mut() {
                let (fv, ev) = surface_touch(s.surface);
                s.touch.friction = (fr * fv).max(0.0);
                s.touch.elasticity = (er * ev).min(1.0);
                s.touch.elasticity = type_elasticity;
                s.touch.impulse = Vec3::ZERO;
            }
        }
    }
}

type PairSide<'a> = (super::loose::Core<'a>, f32, Vec3, (Vec3, Vec3), [Vec3; 4], [Vec3; 4]);

/// F-36478 for one rider x vehicle part pair, then the sink. `veh_is_a`: which side the vehicle is. The detector
/// (F-47540) always: the motion test's global (0x828127D6) is 0 at run time (the RGDS snapshots of ro.0, rs.0 and
/// rt.0), so the swept test (F-98094) is reached only by the collision context's byte +280, which is not set for this
/// pair (every new contact of rs.0 154 and rt.0 156 - 175 is the detector's, 11 of them where the parts moved far).
fn veh_pair_contact(m: &mut VehManifold, a: &PairSide, b: &PairSide, veh_is_a: bool, veh: &VehSide, vp: usize) {
    use super::gjk::convex_contact;
    use super::loose::inv;
    let (ca, ma_, _, _, a_now, _) = a;
    let (cb, mb_, _, _, b_now, _) = b;
    let mut dir = Vec3::Z;
    let found = convex_contact(ca, *ma_, cb, *mb_, &mut dir, true).map(|r| (r.normal, r.on_a, r.on_b, r.distance, r));
    let Some((normal, qa, qb, dist, r)) = found else { return };
    if !(f32::from_bits(0x3ca3_d70a) > dist) {
        return;
    }
    // (the hull's polygon: the corners the detector ended on and the contact's direction in the hull's frame, away
    // from the other part; a wheel's element 0)
    let element = match (vp, veh_is_a) {
        (0, true) => veh.hull.element(&r.corners.0[..r.corners.1], inv(&veh.now, Vec3::ZERO - normal)),
        (0, false) => veh.hull.element(&r.corners_b.0[..r.corners_b.1], inv(&veh.now, normal)),
        _ => 0,
    };
    // (a wheel's material: the hull's first polygon's - unverified, the recordings' wheel contacts carry the hull's
    // friction)
    let surface = veh.hull.surface_of(element);
    let side = |now: &[Vec3; 4], q: Vec3| Side { body: None, point: q, local: inv(now, q - now[3]), rows: [now[0], now[1], now[2]], margin: SIDE_MARGIN, core: Vec3::ZERO };
    let fresh = Touch { depth: 0.0 - dist, a: side(a_now, qa), b: side(b_now, qb), normal, back: Vec3::ZERO - normal, ..Default::default() };
    sink(&mut m.slots, RiderSlot { touch: fresh, age: 0, enabled: true, polygon: element as u32, surface });
}

/// F-36478 for one self pair, then the sink.
fn self_pair_contact(m: &mut SelfManifold, sa: &PartShape, sb: &PartShape, a_now: &[Vec3; 4], a_last: &[Vec3; 4], b_now: &[Vec3; 4], b_last: &[Vec3; 4]) {
    use super::gjk::convex_contact;
    use super::loose::{inv, sweep, Core};
    let (ca, cb) = (sa.core(a_now), sb.core(b_now));
    let found = if m.slots.is_empty() && moved_far(a_now, a_last, sa.bbox, b_now, b_last, sb.bbox) {
        sweep(&Core::Part(ca), sa.margin, sa.centroid, a_now, a_last, &Core::Part(cb), sb.margin, sb.centroid, b_now, b_last).filter(|s| !(s.lambda >= 1.0)).map(|s| {
            let (qa, qb) = (xf(a_now, s.local_a), xf(b_now, s.local_b));
            (s.normal, qa, qb, 0.0 - dot3(s.normal, qb - qa))
        })
    } else {
        let mut dir = Vec3::Z;
        convex_contact(&ca, sa.margin, &cb, sb.margin, &mut dir, true).map(|r| (r.normal, r.on_a, r.on_b, r.distance))
    };
    let Some((normal, qa, qb, dist)) = found else { return };
    // (the sink takes nothing farther than 2 cm)
    if !(f32::from_bits(0x3ca3_d70a) > dist) {
        return;
    }
    let side = |now: &[Vec3; 4], q: Vec3| Side { body: None, point: q, local: inv(now, q - now[3]), rows: [now[0], now[1], now[2]], margin: SIDE_MARGIN, core: Vec3::ZERO };
    let fresh = Touch { depth: 0.0 - dist, a: side(a_now, qa), b: side(b_now, qb), normal, back: Vec3::ZERO - normal, ..Default::default() };
    sink(&mut m.slots, RiderSlot { touch: fresh, age: 0, enabled: true, polygon: 0, surface: sb.surface });
}

/// The sink (F-90136): the nearest kept contact (age != 0) within 2 cm by A's local point takes the new one's place
/// (its multipliers stay), else the new one is appended up to six, else F-55965 picks the one it replaces (or none).
fn sink(slots: &mut Vec<RiderSlot>, slot: RiderSlot) {
    let fresh = slot.touch;
    let mut near = (NEAR2, None);
    for (i, o) in slots.iter().enumerate() {
        let d = o.touch.a.local - fresh.a.local;
        let d2 = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y));
        if d2 < near.0 && o.age != 0 {
            near = (d2, Some(i));
        }
    }
    match near.1 {
        Some(i) => {
            // (the element ids +168 / +170 - B's polygon and with it its material - stay the old contact's when it
            // was the deeper)
            let old = slots[i];
            let keep = old.touch.depth >= fresh.depth;
            let (polygon, surface) = if keep { (old.polygon, old.surface) } else { (slot.polygon, slot.surface) };
            slots[i] = RiderSlot { touch: Touch { kept: old.touch.kept, pushed: old.touch.pushed, ..fresh }, polygon, surface, ..slot };
        }
        None if slots.len() < 6 => slots.push(slot),
        None => {
            let k = super::hull::seventh_contact(std::array::from_fn(|i| slots[i].touch.a.local), fresh.a.local);
            if k < 6 {
                slots[k] = slot;
            }
        }
    }
}

/// The refresh of one manifold's contacts (F-26862 / F-33100): kept contacts get their world points again (A by
/// `ma`, B by `mb`) and the depth along the stored minus normal; every contact ages by one; those with depth not
/// above -0.04 go, and of the older ones the one drifted farthest past 2 cm goes.
fn refresh_slots(slots: &mut Vec<RiderSlot>, ma: &[Vec3; 4], mb: &[Vec3; 4]) {
    for s in slots.iter_mut().rev() {
        let t = &mut s.touch;
        if s.age != 0 {
            t.a.point = xf(ma, t.a.local);
            t.b.point = xf(mb, t.b.local);
            t.depth = dot3(t.a.point - t.b.point, t.back);
        }
        t.target = Vec3::ZERO;
        s.age += 1;
    }
    let (mut worst, mut k_worst) = (0.0f32, None);
    let mut i = 0;
    while i < slots.len() {
        let t = slots[i].touch;
        if !(t.depth > DROP_DEPTH) {
            slots.swap_remove(i);
            continue;
        }
        if slots[i].age > DRIFT_AGE {
            let q = Vec3::ZERO - (t.back * t.depth - t.a.point);
            let s = t.b.point - q;
            let d2 = dot3(s, s);
            if !(worst >= d2) && !(NEAR2 >= d2) {
                worst = d2;
                k_worst = Some(i);
            }
        }
        i += 1;
    }
    if let Some(k) = k_worst {
        slots.swap_remove(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// testdata/rider_contacts_ro0.bin (work/ragdoll/contact/export_contacts.py): sections of u32 words.
    struct Fix(HashMap<String, Vec<u32>>);

    impl Fix {
        fn load() -> Fix {
            let d: &[u8] = include_bytes!("testdata/rider_contacts_ro0.bin");
            assert_eq!(&d[..8], b"RGCONT01");
            let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
            let mut o = 12;
            let mut m = HashMap::new();
            for _ in 0..u(8) {
                let name = String::from_utf8(d[o..o + 8].iter().copied().take_while(|&b| b != 0).collect()).unwrap();
                let n = u(o + 8) as usize;
                m.insert(name, (0..n).map(|i| u(o + 12 + 4 * i)).collect());
                o += 12 + 4 * n;
            }
            Fix(m)
        }
        fn w(&self, n: &str) -> &[u32] {
            &self.0[n]
        }
        fn v4(&self, n: &str, at: usize) -> V4 {
            let w = &self.w(n)[at..at + 4];
            V4::from_bits([w[0], w[1], w[2], w[3]])
        }
        fn mat(&self, n: &str, i: usize) -> Mat {
            [0, 1, 2, 3].map(|r| self.v4(n, 16 * i + 4 * r))
        }
        fn mats(&self, n: &str) -> Vec<Mat> {
            (0..self.w(n).len() / 16).map(|i| self.mat(n, i)).collect()
        }
        fn links(&self, t: u32) -> Vec<LinkFrame> {
            (0..15).map(|k| { let m = self.mat(&format!("lnk{t}"), k); LinkFrame { rows: [m[0], m[1], m[2]], pos: m[3] } }).collect()
        }
        fn clink(&self) -> Vec<i32> {
            self.w("clink").iter().map(|&x| x as i32).collect()
        }
        /// rd+16 from the collider after slot 5 (F-18040).
        fn inst(&self, t: u32) -> [Vec3; 4] {
            let c = self.mat(&format!("crw{t}"), 0);
            instance_matrix(&[c[0], c[1], c[2]], c[3], self.v4("bnd80", 0))
        }
        /// The fragment type's O matrices only (what the part rule reads).
        fn ty(&self) -> RagdollType {
            let mut t = RagdollType { ptr248: 1, ..Default::default() };
            for i in 0..21 {
                let o = self.mat("cO", i).map(|r| r.0);
                t.children.push(mcla_formats::rage::ragdoll::RagChild { o, ..Default::default() });
            }
            t
        }
        /// The recorded contacts of tick `t`: (pair kind, part A, part B, age, flags, element A, element B, record).
        fn contacts(&self, t: u32) -> Vec<([u32; 8], Vec<u32>)> {
            let w = self.w(&format!("ct{t}"));
            w.chunks(52).map(|c| (c[..8].try_into().unwrap(), c[8..].to_vec())).collect()
        }
    }

    fn bits3(v: Vec3) -> [u32; 3] {
        [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
    }

    /// A record's vector at a byte offset (the words are the guest's big-endian words).
    fn rec3(r: &[u32], off: usize) -> [u32; 3] {
        [r[off / 4], r[off / 4 + 1], r[off / 4 + 2]]
    }

    /// The parts' matrices of the tick's collide: rd+16 (F-18040) and +132 (F-15571) from the links and collider
    /// after slot 5, against the snapshot taken at slot 10's entry of ticks 169 and 170 (rd+16 and bound+132).
    #[test]
    fn placement_matches_recording() {
        let f = Fix::load();
        let t = f.ty();
        let (mut ok, mut all) = (0, 0);
        for tick in [169u32, 170] {
            let inst = f.inst(tick);
            let want = f.mat(&format!("in1_{tick}"), 0);
            for r in 0..4 {
                for (l, x) in [inst[r].x, inst[r].y, inst[r].z].iter().enumerate() {
                    all += 1;
                    ok += (x.to_bits() == want[r].0[l].to_bits()) as usize;
                }
            }
            let mut parts = Parts { cur: f.mats(&format!("pv1_{tick}")), prev: f.mats(&format!("pv1_{tick}")) };
            let c = f.mat(&format!("crw{tick}"), 0);
            moved_parts(&t, &f.links(tick), &f.clink(), c[3], &want, &[], &mut parts).unwrap();
            let cur = f.mats(&format!("cu1_{tick}"));
            for i in 0..21 {
                for r in 0..4 {
                    for l in 0..3 {
                        all += 1;
                        ok += (parts.cur[i][r].0[l].to_bits() == cur[i][r].0[l].to_bits()) as usize;
                    }
                }
            }
            // (+136 untouched by the move)
            assert_eq!(parts.prev.len(), 21);
        }
        eprintln!("placement: {ok} / {all} words (rd+16 x y z, 21 parts x 12 lanes, ticks 169 and 170)");
        assert_eq!(ok, all);
    }

    fn data_dir() -> Option<std::path::PathBuf> {
        let root = std::env::var_os("MCLA_DATA_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("D:/MCLA/extracted/cache"));
        root.join("resources/character/drv_mp_01_mot_set/0x5eb8d142.r6").exists().then_some(root)
    }

    fn rider_type(root: &std::path::Path) -> Option<RagdollType> {
        let d = std::fs::read(root.join("resources/character/drv_mp_01_mot_set/0x5eb8d142.r6")).ok()?;
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        let res = mcla_formats::rsc::Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data };
        mcla_formats::rage::ragdoll::parse_character_ragdoll(&res)
    }

    /// The rider's parts against the city, ticks 169 - 216 of ro.0: every contact the collide made in the tick (age 1
    /// in the record: made, then aged by the refresh) is the port's, word for word (depth, both local points, both world
    /// points, normal) and on the same polygon; its friction is the material rule's. Parts: the snapshot's at 169, then
    /// from the links (F-15571 / F-18040, checked above); last = +136 x collider +96.
    #[test]
    fn city_contacts_match_recording() {
        let Some(root) = data_dir() else {
            eprintln!("skipped: the extracted data (MCLA_DATA_DIR) is missing");
            return;
        };
        let assets = crate::assets::Assets::new(&root);
        let Some(cells) = crate::world::city_bound(&assets) else {
            eprintln!("skipped: the city's bound is missing");
            return;
        };
        let rs = root.to_string_lossy().to_string();
        super::super::collision::load_surfaces(&rs);
        let mut city = WorldCollider::default();
        for (i, c) in cells.into_iter().enumerate() {
            city.sectors.insert(i, c);
        }
        let ty = rider_type(&root).expect("rider type");
        let sh = shapes(&ty, &Surface::table(&rs));
        let f = Fix::load();
        let t = f.ty();
        let ticks: Vec<u32> = f.w("ticks").to_vec();
        let mut parts = Parts { cur: f.mats("cu1_169"), prev: f.mats("pv1_169") };
        let (mut matched, mut recorded, mut extra, mut fric, mut other_polygon) = (0, 0, 0, 0, 0);
        for &tick in &ticks {
            let inst = f.inst(tick);
            if tick > 169 {
                parts.prev = parts.cur.clone();
                let c = f.mat(&format!("crw{tick}"), 0);
                let r: Mat = [0, 1, 2, 3].map(|k| V4::new(inst[k].x, inst[k].y, inst[k].z, 0.0));
                moved_parts(&t, &f.links(tick), &f.clink(), c[3], &r, &[], &mut parts).unwrap();
            }
            let last = rows3(&f.mat(&format!("c96_{tick}"), 0));
            let (now, before) = world_parts(&parts, &inst, &last);
            let mut rc = RiderCity::default();
            rc.collide(&sh, &now, &before, &city);
            rc.materials(&sh, ty.f320);
            let mut mine: Vec<(u8, usize, RiderSlot)> = rc.manifolds.iter().flat_map(|m| m.slots.iter().map(move |s| (m.part, m.cell, *s))).collect();
            for (h, r) in f.contacts(tick).into_iter().filter(|(h, _)| h[0] == 0 && h[3] == 1) {
                recorded += 1;
                let want = (r[3], rec3(&r, 16), rec3(&r, 32), rec3(&r, 48), rec3(&r, 64), rec3(&r, 80));
                let k = mine.iter().position(|(p, _, s)| *p as u32 == h[1] && (s.touch.depth.to_bits(), bits3(s.touch.a.local), bits3(s.touch.b.local), bits3(s.touch.a.point), bits3(s.touch.b.point), bits3(s.touch.normal)) == want);
                match k {
                    Some(k) => {
                        matched += 1;
                        let (_, cell, s) = mine.remove(k);
                        if s.polygon == h[6] {
                            fric += (s.touch.friction.to_bits() == r[7]) as usize;
                        } else {
                            // (open: the record's element B is another polygon than the one the port found it on)
                            let mesh = &city.sectors[&cell];
                            eprintln!("tick {tick} part {}: equal contact, polygon {} in the port, element {} recorded (the port's neighbours {:?}; friction {} / {})", h[1], s.polygon, h[6], mesh.beside[s.polygon as usize], s.touch.friction, f32::from_bits(r[7]));
                            other_polygon += 1;
                        }
                    }
                    None => eprintln!("tick {tick} part {} polygon {}: no equal contact of the port (depth {})", h[1], h[6], f32::from_bits(r[3])),
                }
            }
            for (p, _, s) in &mine {
                eprintln!("tick {tick} part {p} polygon {}: the port's, not a recorded new one (depth {})", s.polygon, s.touch.depth);
            }
            extra += mine.len();
        }
        eprintln!("city contacts: {matched} / {recorded} recorded new contacts equal word for word, {other_polygon} of them on another polygon, friction {fric} / {}; {extra} of the port's not among them", matched - other_polygon);
        assert!(recorded > 0);
        assert_eq!(matched, recorded);
        assert_eq!(extra, 0);
        assert_eq!(fric, matched - other_polygon);
        // (known: tick 184, part 20, a point on the ridge of polygons 4620 / 4622 - rider_contacts.md, open items)
        assert!(other_polygon <= 1);
    }

    /// The update's refresh (F-26862 / F-33100) against ro.0, ticks 171 - 216: each rider-city sub-manifold of tick
    /// T - 1, in its recorded order, refreshed with the parts of tick T (the drift rule drops one worst contact per
    /// manifold, so the whole manifold goes in). Every kept contact of tick T (age 2 or more) is one of the survivors,
    /// with world points, depth and age word for word.
    #[test]
    fn refresh_matches_recording() {
        let f = Fix::load();
        let t = f.ty();
        let (mut ok, mut all, mut gone) = (0, 0, 0);
        for &tick in f.w("ticks").iter().filter(|&&k| k >= 171) {
            let inst = f.inst(tick);
            let mut parts = Parts { cur: f.mats("cu1_169"), prev: f.mats("pv1_169") };
            let c = f.mat(&format!("crw{tick}"), 0);
            let r: Mat = [0, 1, 2, 3].map(|k| V4::new(inst[k].x, inst[k].y, inst[k].z, 0.0));
            moved_parts(&t, &f.links(tick), &f.clink(), c[3], &r, &[], &mut parts).unwrap();
            let (now, _) = world_parts(&parts, &inst, &inst);
            let mut rc = RiderCity { manifolds: vec![] };
            let mut subs: Vec<u32> = vec![];
            for (h0, r0) in f.contacts(tick - 1).into_iter().filter(|(h, _)| h[0] == 0) {
                let v = |o: usize| Vec3::new(f32::from_bits(r0[o / 4]), f32::from_bits(r0[o / 4 + 1]), f32::from_bits(r0[o / 4 + 2]));
                let a = Side { local: v(16), point: v(48), ..Default::default() };
                let b = Side { local: v(32), point: v(64), ..Default::default() };
                let touch = Touch { depth: f32::from_bits(r0[3]), a, b, normal: v(80), back: v(96), ..Default::default() };
                let slot = RiderSlot { touch, age: h0[3] as u16, enabled: true, polygon: h0[6], surface: Surface::Default };
                match subs.iter().position(|&s| s == h0[7]) {
                    Some(k) => rc.manifolds[k].slots.push(slot),
                    None => {
                        subs.push(h0[7]);
                        rc.manifolds.push(RiderManifold { part: h0[1] as u8, cell: 0, slots: vec![slot] });
                    }
                }
            }
            rc.refresh(&now);
            for (h, r) in f.contacts(tick).into_iter().filter(|(h, _)| h[0] == 0 && h[3] >= 2) {
                all += 1;
                let found = subs.iter().position(|&s| s == h[7]).and_then(|k| {
                    rc.manifolds[k].slots.iter().find(|s| bits3(s.touch.a.local) == rec3(&r, 16) && bits3(s.touch.b.local) == rec3(&r, 32))
                });
                let Some(s) = found else {
                    gone += 1;
                    eprintln!("tick {tick} part {}: kept in the recording, not among the port's survivors", h[1]);
                    continue;
                };
                ok += (s.age as u32 == h[3] && s.touch.depth.to_bits() == r[3] && bits3(s.touch.a.point) == rec3(&r, 48) && bits3(s.touch.b.point) == rec3(&r, 64)) as usize;
            }
        }
        eprintln!("refresh: {ok} / {all} kept contacts equal (world points, depth, age), {gone} missing");
        assert!(all > 0);
        assert_eq!(ok, all);
    }

    /// The own-bike filter and the body fall against ro.0: every tick's runs (the rider as A: city and self contacts;
    /// as B: bike and self contacts) leave exactly the bike contacts disabled, as recorded (+165 bit 0x80), and the
    /// filter never stops (a bike contact every tick); rd+169 / +170 / +172 at slot 10's entry of ticks 169 and 170
    /// are the recorded ones: the sound played in tick 169, on the rider's first run.
    #[test]
    fn own_bike_gate_matches_recording() {
        let f = Fix::load();
        let (rider, bike) = (0x5703u16, 0x56d7u16);
        let own = f.w("rdb0_169")[0] as i32;
        assert_eq!(own, bike as i32);
        let byte = |w: &[u32], k: usize| (w[1] >> (24 - 8 * k)) & 0xff;
        let mut g = ContactGate::default();
        g.go_ragdoll();
        assert_eq!((byte(f.w("rdb0_169"), 1), byte(f.w("rdb0_169"), 2), f.w("rdb0_169")[2]), (0, 0, 0));
        let (mut ok, mut all) = (0, 0);
        let mut played = Vec::new();
        for &tick in f.w("ticks") {
            let cs = f.contacts(tick);
            // (other instance of each contact; A-run: rider as A, B-run: rider as B)
            let a_run: Vec<usize> = (0..cs.len()).filter(|&k| cs[k].0[0] != 1).collect();
            let b_run: Vec<usize> = (0..cs.len()).filter(|&k| cs[k].0[0] != 0).collect();
            let other = |k: usize, rider_is_a: bool| match cs[k].0[0] {
                0 => 0u16,
                1 => if rider_is_a { rider } else { bike },
                _ => rider,
            };
            let mut enabled = vec![true; cs.len()];
            for (run, is_a) in [(&a_run, true), (&b_run, false)] {
                let others: Vec<u16> = run.iter().map(|&k| other(k, is_a)).collect();
                let mut en: Vec<bool> = run.iter().map(|&k| enabled[k]).collect();
                g.own_bike_run(own, &others, &mut en);
                for (i, &k) in run.iter().enumerate() {
                    enabled[k] = en[i];
                }
                if g.body_fall_run(own, &others, Vec3::ZERO).is_some() {
                    played.push(tick);
                }
            }
            for (k, (h, _)) in cs.iter().enumerate() {
                all += 1;
                ok += ((h[4] & 0x80 != 0) == enabled[k]) as usize;
            }
            if tick == 169 || tick == 170 {
                let w = f.w(&format!("rdb1_{tick}"));
                assert_eq!((g.stopped as u32, g.fell as u32, g.clear_runs), (byte(w, 1), byte(w, 2), w[2]), "tick {tick}");
            }
        }
        eprintln!("own-bike filter: {ok} / {all} recorded enabled bits; body fall in ticks {played:?}");
        assert_eq!(ok, all);
        assert_eq!(played, [169]);
        assert!(!g.stopped);
    }

    /// The self pairs against ro.0, ticks 169 - 216, chained (each tick's sub-manifolds carried to the next, refreshed
    /// as the update does): the sub-manifolds of the tick (the pairs that passed the box test, in the list's order) are
    /// the recorded ones, and every contact made in the tick (age 1) is the port's, word for word (depth, both local
    /// points, both world points, normal), with friction 1 and elasticity 0. Parts as for the city; last = +136 x
    /// collider +32.
    #[test]
    fn self_contacts_match_recording() {
        let Some(root) = data_dir() else {
            eprintln!("skipped: the extracted data (MCLA_DATA_DIR) is missing");
            return;
        };
        let rs = root.to_string_lossy().to_string();
        super::super::collision::load_surfaces(&rs);
        let ty = rider_type(&root).expect("rider type");
        let sh = shapes(&ty, &Surface::table(&rs));
        let f = Fix::load();
        let t = f.ty();
        let pairs = [(1, 7), (1, 11), (8, 1), (8, 3), (8, 5), (8, 14), (12, 1), (12, 3), (12, 5), (12, 14), (15, 18), (16, 19)];
        let mut parts = Parts { cur: f.mats("cu1_169"), prev: f.mats("pv1_169") };
        let mut me = RiderSelf::default();
        let (mut subs_ok, mut ticks_n, mut matched, mut recorded, mut extra) = (0, 0, 0, 0, 0);
        for &tick in f.w("ticks") {
            let inst = f.inst(tick);
            if tick > 169 {
                parts.prev = parts.cur.clone();
                let c = f.mat(&format!("crw{tick}"), 0);
                let r: Mat = [0, 1, 2, 3].map(|k| V4::new(inst[k].x, inst[k].y, inst[k].z, 0.0));
                moved_parts(&t, &f.links(tick), &f.clink(), c[3], &r, &[], &mut parts).unwrap();
            }
            let last = rows3(&f.mat(&format!("c32_{tick}"), 0));
            let (now, before) = world_parts(&parts, &inst, &last);
            me.collide(&pairs, &sh, &now, &before);
            let mine: Vec<(u32, u32)> = me.manifolds.iter().map(|m| (m.a as u32, m.b as u32)).collect();
            let want: Vec<(u32, u32)> = f.w(&format!("sb{tick}")).chunks(4).map(|c| (c[0], c[1])).collect();
            ticks_n += 1;
            if mine == want {
                subs_ok += 1;
            } else {
                eprintln!("tick {tick}: sub-manifolds {mine:?}, recorded {want:?}");
            }
            let mut fresh: Vec<(u8, u8, RiderSlot)> = me.manifolds.iter().flat_map(|m| m.slots.iter().filter(|s| s.age == 0).map(move |s| (m.a, m.b, *s))).collect();
            for (h, r) in f.contacts(tick).into_iter().filter(|(h, _)| h[0] == 2 && h[3] == 1) {
                recorded += 1;
                let want = (r[3], rec3(&r, 16), rec3(&r, 32), rec3(&r, 48), rec3(&r, 64), rec3(&r, 80));
                let k = fresh.iter().position(|(a, b, s)| (*a as u32, *b as u32) == (h[1], h[2]) && (s.touch.depth.to_bits(), bits3(s.touch.a.local), bits3(s.touch.b.local), bits3(s.touch.a.point), bits3(s.touch.b.point), bits3(s.touch.normal)) == want);
                match k {
                    Some(k) => {
                        fresh.remove(k);
                        matched += 1;
                    }
                    None => {
                        let near = fresh.iter().find(|(a, b, _)| (*a as u32, *b as u32) == (h[1], h[2]));
                        eprintln!("tick {tick} parts {} / {}: recorded depth {} n {:?}, port {:?}", h[1], h[2], f32::from_bits(r[3]), [r[20], r[21], r[22]].map(f32::from_bits), near.map(|(_, _, s)| (s.touch.depth, s.touch.normal, s.touch.a.local, f32::from_bits(r[4]))));
                    }
                }
            }
            for (a, b, s) in &fresh {
                eprintln!("tick {tick} parts {a} / {b}: the port's contact, not a recorded new one (depth {})", s.touch.depth);
            }
            extra += fresh.len();
            me.refresh(&now);
            me.materials(&sh, ty.f320);
            for m in &me.manifolds {
                for s in &m.slots {
                    assert_eq!((s.touch.friction, s.touch.elasticity), (1.0, 0.0));
                }
            }
        }
        eprintln!("self pairs: sub-manifolds equal in {subs_ok} / {ticks_n} ticks; contacts {matched} / {recorded} recorded new ones word for word, {extra} of the port's not among them");
        assert!(recorded > 0);
        assert_eq!(subs_ok, ticks_n);
        assert_eq!(matched, recorded);
        assert_eq!(extra, 0);
    }

    /// testdata/rider_bike_<rec>.bin (work/ragdoll/contacts2/export_bike.py over the RGDS snapshots at slot 10's
    /// entry): the rider's and the bike's part matrices of each snapshot tick and the pair's recorded contacts.
    fn bike_fix(d: &[u8]) -> Fix {
        assert_eq!(&d[..8], b"RGBIKE01");
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let mut o = 12;
        let mut m = HashMap::new();
        for _ in 0..u(8) {
            let name = String::from_utf8(d[o..o + 8].iter().copied().take_while(|&b| b != 0).collect()).unwrap();
            let n = u(o + 8) as usize;
            m.insert(name, (0..n).map(|i| u(o + 12 + 4 * i)).collect());
            o += 12 + 4 * n;
        }
        Fix(m)
    }

    /// The rider's parts against his bike's hull in the thrown rider's first ticks (the ZX-14 into the wall at the
    /// spawn): rs.0 154 - 155 (the bike A, the lower level index) and rt.0 156 - 175 (the rider A): the sub-manifolds
    /// holding contacts after the refresh and every contact the collide made in the tick (age 1 in the record), word
    /// for word - depth, both local points, both world points, the normal - and the hull's polygon. The bike's wheels
    /// (parts 1, 2) are not ported for this pair: their contacts are counted apart.
    #[test]
    fn bike_contacts_match_recording() {
        let Some(root) = data_dir() else {
            eprintln!("skipped: the extracted data (MCLA_DATA_DIR) is missing");
            return;
        };
        let rs = root.to_string_lossy().to_string();
        super::super::collision::load_surfaces(&rs);
        let ty = rider_type(&root).expect("rider type");
        let sh = shapes(&ty, &Surface::table(&rs));
        let assets = crate::assets::Assets::new(&root);
        let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle/vp_kaw_ninja_zx14_06")).expect("the ZX-14");
        let st = car.simtune.as_ref().unwrap();
        let hull = super::super::hull::shaped(car.hull.as_ref().unwrap(), &st.base, 0.3, true);
        let (mut matched, mut recorded, mut extra, mut wheel, mut subs_ok, mut ticks_n) = (0, 0, 0, 0, 0, 0);
        let (mut elements, mut kept_ok, mut kept_all) = (0, 0, 0);
        let (mut mat_ok, mut mat_all) = (0, 0);
        for (name, d) in [("rs.0", &include_bytes!("testdata/rider_bike_rs0.bin")[..]), ("rt.0", &include_bytes!("testdata/rider_bike_rt0.bin")[..])] {
        let f = bike_fix(d);
        eprintln!("{name}");
        let lv = f.w("lv");
        let rider_is_a = lv[0] < lv[1];
        let mut pair = RiderVeh { manifolds: Vec::new(), rider_is_a };
        // (a recorded (part A, part B) as (the bike's part, the rider's))
        let vr = |a: u32, b: u32| if rider_is_a { (b, a) } else { (a, b) };
        for &tick in f.w("ticks") {
            let mats = |n: &str| f.mats(&format!("{n}_{tick}")).iter().map(rows3).collect::<Vec<_>>();
            let (r16, r32) = (rows3(&f.mat(&format!("r16_{tick}"), 0)), rows3(&f.mat(&format!("r32_{tick}"), 0)));
            let (b16, b32) = (rows3(&f.mat(&format!("b16_{tick}"), 0)), rows3(&f.mat(&format!("b32_{tick}"), 0)));
            let now: Vec<[Vec3; 4]> = mats("rcu").iter().map(|p| world_matrix(p, &r16)).collect();
            let last: Vec<[Vec3; 4]> = mats("rpv").iter().map(|p| world_matrix(p, &r32)).collect();
            let bnow: Vec<[Vec3; 4]> = mats("bcu").iter().map(|p| world_matrix(p, &b16)).collect();
            let blast: Vec<[Vec3; 4]> = mats("bpv").iter().map(|p| world_matrix(p, &b32)).collect();
            let veh = VehSide { hull: &hull, hull_cg: hull.cg, hull_margin: f32::from_bits(0x3d23_d70a), now: bnow[0], last: blast[0], wheels: vec![None; bnow.len() - 1] };
            pair.collide(&sh, &now, &last, &veh);
            ticks_n += 1;
            let mut fresh: Vec<(u8, u8, RiderSlot)> = pair.manifolds.iter().flat_map(|m| m.slots.iter().filter(|s| s.age == 0).map(move |s| (m.veh, m.part, *s))).collect();
            for c in f.w(&format!("bct{tick}")).chunks(52).filter(|c| c[2] == 1) {
                let (h, r) = (&c[..8], &c[8..]);
                let (hv, hr) = vr(c[0], c[1]);
                let h = [hv, hr, h[2], h[3], if rider_is_a { h[5] } else { h[4] }];
                if h[0] != 0 {
                    wheel += 1;
                    continue;
                }
                recorded += 1;
                let want = (r[3], rec3(r, 16), rec3(r, 32), rec3(r, 48), rec3(r, 64), rec3(r, 80));
                let k = fresh.iter().position(|(v, p, s)| (*v as u32, *p as u32) == (h[0], h[1]) && (s.touch.depth.to_bits(), bits3(s.touch.a.local), bits3(s.touch.b.local), bits3(s.touch.a.point), bits3(s.touch.b.point), bits3(s.touch.normal)) == want);
                match k {
                    Some(k) => {
                        if fresh[k].2.polygon == h[4] {
                            elements += 1;
                        } else {
                            eprintln!("tick {tick} parts {} / {}: element {} recorded {}", h[0], h[1], fresh[k].2.polygon, h[4]);
                        }
                        fresh.remove(k);
                        matched += 1;
                    }
                    None => {
                        let near = fresh.iter().find(|(v, p, _)| (*v as u32, *p as u32) == (h[0], h[1]));
                        eprintln!("tick {tick} parts {} / {}: recorded depth {} n {:?} la {:?}, port {:?}", h[0], h[1], f32::from_bits(r[3]), [r[20], r[21], r[22]].map(f32::from_bits), [r[4], r[5], r[6]].map(f32::from_bits), near.map(|(_, _, s)| (s.touch.depth, s.touch.normal, s.touch.a.local)));
                    }
                }
            }
            for (v, p, s) in &fresh {
                eprintln!("tick {tick} parts {v} / {p}: the port's contact, not a recorded new one (depth {})", s.touch.depth);
            }
            extra += fresh.len();
            pair.refresh(&now, &bnow);
            // (every recorded contact after the refresh: its age, depth and both world points)
            for c in f.w(&format!("bct{tick}")).chunks(52) {
                let (hv, hr) = vr(c[0], c[1]);
                if hv != 0 {
                    continue;
                }
                let r = &c[8..];
                kept_all += 1;
                let ok = pair.manifolds.iter().filter(|m| (m.veh as u32, m.part as u32) == (hv, hr)).flat_map(|m| m.slots.iter()).any(|s| {
                    s.age as u32 == c[2] && bits3(s.touch.a.local) == rec3(r, 16) && s.touch.depth.to_bits() == r[3] && bits3(s.touch.a.point) == rec3(r, 48) && bits3(s.touch.b.point) == rec3(r, 64)
                });
                if ok {
                    kept_ok += 1;
                } else {
                    eprintln!("tick {tick} parts {hv} / {hr}: the refreshed contact (age {}, depth {}) is not the port's", c[2], f32::from_bits(r[3]));
                }
            }
            let mine: Vec<(u32, u32)> = pair.manifolds.iter().filter(|m| !m.slots.is_empty()).map(|m| (m.veh as u32, m.part as u32)).collect();
            let want: Vec<(u32, u32)> = f.w(&format!("bsb{tick}")).chunks(3).filter(|c| c[2] > 0).map(|c| vr(c[0], c[1])).filter(|s| s.0 == 0).collect();
            if mine == want {
                subs_ok += 1;
            } else {
                eprintln!("tick {tick}: hull sub-manifolds {mine:?}, recorded {want:?}");
            }
            pair.materials(&sh, ty.f320);
            // (the materials: friction +28, elasticity +44 as the record holds them at the tick's end)
            for c in f.w(&format!("bct{tick}")).chunks(52) {
                let (hv, hr) = vr(c[0], c[1]);
                let r = &c[8..];
                if let Some(sl) = pair.manifolds.iter().filter(|m| (m.veh as u32, m.part as u32) == (hv, hr)).flat_map(|m| m.slots.iter()).find(|s| bits3(s.touch.a.local) == rec3(r, 16)) {
                    mat_all += 1;
                    if (sl.touch.friction.to_bits(), sl.touch.elasticity.to_bits()) == (r[7], r[11]) {
                        mat_ok += 1;
                    } else if mat_all - mat_ok < 4 {
                        eprintln!("{name} tick {tick}: friction / elasticity {} / {}, recorded {} / {}", sl.touch.friction, sl.touch.elasticity, f32::from_bits(r[7]), f32::from_bits(r[11]));
                    }
                }
            }
        }
        }
        eprintln!("rider x bike: hull sub-manifolds equal in {subs_ok} / {ticks_n} ticks; contacts {matched} / {recorded} word for word ({elements} with the recorded polygon), {extra} of the port's not among them; {wheel} wheel contacts not compared; after the refresh {kept_ok} / {kept_all} hull contacts equal (age, depth, points); materials {mat_ok} / {mat_all}");
        assert_eq!(kept_ok, kept_all);
        assert!(recorded > 0);
        assert_eq!(matched, recorded);
        assert_eq!(elements, matched);
        assert_eq!(extra, 0);
        assert_eq!(subs_ok, ticks_n);
    }

    /// The bound: 21 parts, capsules with the ragdoll material, the self-pair gate (collider +740 = 1, +744 = 0 in ro.0).
    #[test]
    fn shapes_and_self_pairs() {
        let pairs_a = [1, 1, 8, 8, 8, 8, 12, 12, 12, 12, 15, 16, -1];
        let pairs_b = [7, 11, 1, 3, 5, 14, 1, 3, 5, 14, 18, 19, -1];
        let mut cd = 0;
        let p = self_pairs(true, &mut cd, &pairs_a, &pairs_b).unwrap();
        assert_eq!(p.len(), 12);
        assert_eq!(self_pairs(false, &mut cd, &pairs_a, &pairs_b), None);
        let mut cd = 2;
        assert_eq!(self_pairs(true, &mut cd, &pairs_a, &pairs_b), None);
        assert_eq!(cd, 1);
        let Some(root) = data_dir() else {
            eprintln!("skipped: the extracted data (MCLA_DATA_DIR) is missing");
            return;
        };
        let ty = rider_type(&root).expect("rider type");
        let sh = shapes(&ty, &Surface::table(&root.to_string_lossy()));
        assert_eq!(sh.len(), 21);
        assert!(sh.iter().all(|s| s.is_some_and(|s| s.surface == Surface::Ragdoll)));
        let s = sh[15].unwrap();
        assert_eq!((s.kind, s.margin, s.half.y), (1, 0.07, 0.4));
        let b = sh[9].unwrap();
        assert_eq!((b.kind, b.margin), (3, 0.01));
        assert_eq!(b.half, Vec3::new(0.02, 0.075, 0.04) - Vec3::splat(0.01));
    }
}
