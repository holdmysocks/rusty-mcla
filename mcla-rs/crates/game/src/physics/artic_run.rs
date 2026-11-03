//! The thrown bike rider's ragdoll as the game runs it: GoRagdoll's activation and one physics tick, chaining the
//! pieces checked one by one (artic.rs, artic_island.rs, artic_collide.rs, artic_pose.rs, solver.rs) in the
//! original's order. Decode: docs/reference/vehicle/ragdoll_solver_decode.md 1.1 (the tick), ragdoll_setup_decode.md
//! 1.2 (GoRagdoll), ragdoll_pose_sync.md (activation, rd+16, the part matrices), work/ragdoll/run/runtime.md (this
//! chain and its check).
//!
//! One tick (the rider alone in his island, against the city):
//! 1. PreCollide, the collider's slot 5 (F-75381, [`Articulated::step`]); inside its move the re-centre ends with
//!    rd+16 (F-18040, [`RiderRagdoll::instance`]) and the part matrices (F-15571, `moved_parts`).
//! 2. Collide: every part against the city (F-82759, [`RiderCity::collide`]) with now = +132 x rd+16, last = +136 x
//!    collider +96; the self pairs' gate (F-41849; their box test and narrow phase are not ported, nor the rider
//!    against the bike and other cars: those contacts would join the manifolds here).
//! 3. Update (F-96487): the contacts refreshed (F-26862 / F-33100) and their materials (F-43430, F-22759); the island
//!    build: the own-bike filter (F-37566, pre-solve run), the hinges' contact bytes (F-18964, every contact of the
//!    island's manifolds with a side on the rider); the velocity solve with the joint-limit rows and the apply
//!    (F-24076); the body-fall trigger (F-15373, post-solve run); slot 15 (F-32404); slot 10 (F-69485); the push
//!    solve (F-49764) and its apply with the post-push (F-48210, whose re-centre ends with F-18040 again).
//! 4. The collider's slot 14 (F-35284): the part matrices (F-43257), then the base F-32811: the pending push
//!    (F-67019), rd+16 (F-18040), collider +32 = rd+16 and, the deep byte +658 clear, +96 = rd+16 and +660 = 1;
//!    +659 = 0.
//!
//! What the rest of the island brings (the bike lying beside the rider: its contacts' count and elasticity in the
//! island's factor isl +40; the rider's parts in contacts that are not ported - the bike's, self contacts - which
//! set the hinge bytes too) comes in as [`Outside`].
//!
//! Open (nothing guessed): a tick where the rider is in no island (no contact at all; in ro.0 he always is: the
//! velocity and push solves, slot 15 and the post-push are skipped here, the post-push being called from the push
//! apply); the own-bike filter and the body fall when the rider has no contact (no run is made here); the bound's
//! box the resting test reads (8259ADA8 per slot 14, not ported: the recorded corners are used); lane w of the
//! eject vector seq+144; the bound's part matrices before the activation (bound+132 from the cache entry's init,
//! which the activation copies into +136); the type-13 self contact rows; the seventh contact of a manifold.

use super::artic::island::part_link;
use super::artic::{Articulated, Surroundings, V4};
use super::artic_build::ArticBuild;
use super::artic_collide::{moved_parts, rows3, self_pairs, shapes, world_parts, ContactGate, PartShape, RiderCity};
use super::artic_pose::{activate_parts, bone_globals, fk, seed_links, tick_parts, LinkFrame, Mat, Parts};
use super::collision::{Surface, WorldCollider};
use super::solver::{self, JointedSide, Touch};
use glam::Vec3;
use mcla_formats::rage::ragdoll::RagdollType;

/// What slot 5 reads outside the body for the rider (the recording's RGDS pages): the world's gravity (run-time
/// (0, -9.8, 0, 0)), the rider archetype's damping (0.02 / 0.02 / 0.01, lane w as stored) and bound corners, the
/// instance flag clear, the resting limit 0.007. The corners are those of ro.0's bound (the per-tick box, 8259ADA8,
/// is not ported).
pub fn rider_surroundings() -> Surroundings {
    let s = |b: u32| f32::from_bits(b);
    let d = |b: u32| V4::from_bits([b, b, b, 0x7f80_0001]);
    Surroundings {
        gravity: V4::new(0.0, s(0xc11c_cccd), 0.0, 0.0),
        bound: [
            V4::from_bits([0x3f09_804a, 0x3f4e_998e, 0x3ee0_da07, 0x3ff7_7a06]),
            V4::from_bits([0xbf9a_5900, 0xbf80_067f, 0xbf84_e24c, 0xbf80_067f]),
        ],
        no_gravity: false,
        damping: Some([d(0x3ca3_d70a), d(0x3ca3_d70a), d(0x3c23_d70a), d(0x3ca3_d70a), d(0x3ca3_d70a), d(0x3c23_d70a)]),
        rest_limit: s(0x3be5_6042),
    }
}

/// Lane w of the rider's point (contact +60) in every contact the city's narrow phase made for him in ro.0
/// (FLT_MAX); the arm of a type-7 row keeps it.
const POINT_W: f32 = f32::MAX;

/// The island's share that is not the rider's own contacts with the city.
#[derive(Clone, Debug, Default)]
pub struct Outside {
    /// the island's enabled contacts that share no row with the rider (the bike lying on the street): their count
    /// and the sum of their elasticities (isl +40)
    pub others: (usize, f32),
    /// the rider's parts in the island's other contacts (enabled or not: bike, self): F-18964 marks their hinges
    pub parts: Vec<i32>,
}

/// What one tick gives the game.
#[derive(Clone, Copy, Debug, Default)]
pub struct TickOut {
    /// BIKER_BODYFALL_MASTER plays at this position (rd+64) this tick (F-92308)
    pub body_fall: Option<Vec3>,
    /// the rider's contacts with the city in the island's solve
    pub contacts: usize,
    /// the rider was in an island (the solves ran)
    pub island: bool,
    /// slot 5's resting test (collider +657)
    pub resting: bool,
    /// slot 15's mean squared motion
    pub motion: f32,
}

/// The rider's ragdoll while it is active.
#[derive(Clone, Debug)]
pub struct RiderRagdoll {
    /// the fragment type (T = rd+92)
    pub ty: RagdollType,
    /// collider +716: child -> link; +724 / +732 the self pairs
    pub child_link: Vec<i32>,
    pub pair_a: Vec<i32>,
    pub pair_b: Vec<i32>,
    /// the articulated collider and body
    pub body: Articulated,
    /// bound +132 / +136
    pub parts: Parts,
    /// bound +128: parts not present (none for the rider)
    pub absent: Vec<bool>,
    /// the bound's parts as the narrow phase takes them
    pub shapes: Vec<Option<PartShape>>,
    /// the rider's manifolds with the city (with their warm-start multipliers)
    pub city: RiderCity,
    /// rd+169 / +170 / +172
    pub gate: ContactGate,
    /// rd+164: the level index of the bike he came off (-1: none)
    pub own_bike: i32,
    /// rd+16, the instance matrix
    pub rd16: Mat,
    /// bound +80 (= T+32): the bound's centre offset F-18040 takes off
    pub offset: V4,
    /// the skeleton's locals: the eject clip's last frame (no writer during the ragdoll was found)
    pub locals: Vec<Mat>,
    /// what slot 5 reads outside the body
    pub world: Surroundings,
}

fn wr(b: &mut [u8], o: usize, v: V4) {
    for i in 0..4 {
        b[o + 4 * i..o + 4 * i + 4].copy_from_slice(&v.0[i].to_bits().to_be_bytes());
    }
}

fn rdv(b: &[u8], o: usize) -> V4 {
    V4::from_bits([0, 1, 2, 3].map(|i| u32::from_be_bytes(b[o + 4 * i..o + 4 * i + 4].try_into().unwrap())))
}

fn mat_at(b: &[u8], o: usize) -> Mat {
    [rdv(b, o), rdv(b, o + 16), rdv(b, o + 32), rdv(b, o + 48)]
}

impl RiderRagdoll {
    /// The ragdoll around a body as it stands (a built one, or a state of the original's), before any tick:
    /// rd+16 from collider +32, no contacts, the gate as GoRagdoll leaves it.
    #[allow(clippy::too_many_arguments)]
    pub fn assemble(ty: &RagdollType, body: Articulated, child_link: Vec<i32>, pair_a: Vec<i32>, pair_b: Vec<i32>, parts: Parts, locals: &[Mat], surfaces: &[Surface]) -> RiderRagdoll {
        let rd16 = mat_at(&body.raw_collider, 32);
        let mut gate = ContactGate::default();
        gate.go_ragdoll();
        RiderRagdoll {
            ty: ty.clone(),
            child_link,
            pair_a,
            pair_b,
            body,
            absent: vec![false; ty.children.len()],
            parts,
            shapes: shapes(ty, surfaces),
            city: RiderCity::default(),
            gate,
            own_bike: -1,
            rd16,
            offset: V4(ty.cg),
            locals: locals.to_vec(),
            world: rider_surroundings(),
        }
    }

    /// GoRagdoll (F-81936) on a freshly built body: the skeleton's globals = FK of the eject clip's last frame
    /// (`locals`) under the character matrix (F-97374); rd+16 = the character matrix; the part matrices (F-36901);
    /// the links' frames from their first children (F-93058); collider +96 and +32 = rd+16 (F-24099); the reset
    /// (F-47797) and one velocity for the whole body, seq+144 + (0, 1, 0, 0), no spin (F-86625). `surfaces`: the
    /// game's material table ([`Surface::table`]). The links' velocities F-93058 forms are wiped by the reset and
    /// not formed here.
    pub fn go(ty: &RagdollType, build: ArticBuild, locals: &[Mat], character: Mat, launch: Vec3, surfaces: &[Surface]) -> RiderRagdoll {
        let n = u32::from_be_bytes(build.body[0..4].try_into().unwrap()) as usize;
        let addr: Vec<u32> = (0..n).map(|i| u32::from_be_bytes(build.body[4 + 4 * i..8 + 4 * i].try_into().unwrap())).collect();
        let links: Vec<(u32, &[u8])> = addr.iter().zip(build.links.iter()).map(|(a, b)| (*a, b.as_slice())).collect();
        let joints: Vec<&[u8]> = build.joints.iter().map(|(_, b)| b.as_slice()).collect();
        let body = Articulated::from_guest(&build.collider, &build.body, &links, &joints);
        let globals = fk(locals, &ty.bone_parent, &character);
        let mut parts = Parts::new(ty.children.len());
        activate_parts(ty, &globals, &character, &[], &mut parts);
        let mut rd = RiderRagdoll::assemble(ty, body, build.child_link.clone(), build.pair_a.clone(), build.pair_b.clone(), parts, locals, surfaces);
        rd.rd16 = character;
        let frames = seed_links(ty, &rd.child_link, n, &rd.parts, &character, rd.body.place, &[]);
        for (l, f) in rd.body.links.iter_mut().zip(frames) {
            if let Some(f) = f {
                f.put(l);
            }
        }
        for o in [32, 96] {
            for (r, v) in character.iter().enumerate() {
                wr(&mut rd.body.raw_collider, o + 16 * r, *v);
            }
        }
        rd.body.reset();
        rd.body.set_velocity(V4::new(launch.x, launch.y, launch.z, 0.0).add(V4::new(0.0, 1.0, 0.0, 0.0)));
        rd
    }

    /// F-18040: rd+16 = the collider's rows, its place less the bound's centre offset through them (lane w: the
    /// place's less the x dot, as the merges put it).
    pub fn instance(&self) -> Mat {
        let (r, p) = (&self.body.rows, self.body.place);
        let cols = [V4::new(r[0].0[0], r[1].0[0], r[2].0[0], p.0[0]), V4::new(r[0].0[1], r[1].0[1], r[2].0[1], p.0[1]), V4::new(r[0].0[2], r[1].0[2], r[2].0[2], p.0[2])];
        let d = cols.map(|c| self.offset.dot3(c).x());
        let pos = p.add(V4::ZERO.sub(V4::new(d[0], d[1], d[2], d[0])));
        [r[0], r[1], r[2], pos]
    }

    fn link_frames(&self) -> Vec<LinkFrame> {
        self.body.links.iter().map(LinkFrame::of).collect()
    }

    /// One physics tick with nothing else in the rider's island ([`RiderRagdoll::tick_with`]).
    pub fn tick(&mut self, dt: f32, city: &WorldCollider) -> TickOut {
        self.tick_with(dt, city, &Outside::default())
    }

    /// One physics tick in the original's order (module doc), dt the step (1 / 60).
    pub fn tick_with(&mut self, dt: f32, city: &WorldCollider, outside: &Outside) -> TickOut {
        let mut out = TickOut::default();
        // 1. slot 5, with rd+16 and the part matrices of its move
        self.body.step(dt, &self.world);
        out.resting = self.body.resting != 0;
        self.rd16 = self.instance();
        let links = self.link_frames();
        // (the rider's type has part frames O: no other rule applies)
        let _ = moved_parts(&self.ty, &links, &self.child_link, self.body.place, &self.rd16, &self.absent, &mut self.parts);

        // 2. collide
        let last = rows3(&mat_at(&self.body.raw_collider, 96));
        let (now, before) = world_parts(&self.parts, &rows3(&self.rd16), &last);
        self.city.collide(&self.shapes, &now, &before, city);
        let raw = &mut self.body.raw_collider;
        let mut countdown = i32::from_be_bytes(raw[744..748].try_into().unwrap());
        // (the pairs' box test and narrow phase are not ported: their contacts would join the island here)
        let _ = self_pairs(raw[740] != 0, &mut countdown, &self.pair_a, &self.pair_b);
        raw[744..748].copy_from_slice(&countdown.to_be_bytes());

        // 3. update: the refresh and the materials
        self.city.refresh(&now);
        self.city.materials(&self.shapes, self.ty.f320);
        let n_links = self.body.links.len();
        let mut slots: Vec<(usize, usize)> = Vec::new();
        for (m, mf) in self.city.manifolds.iter().enumerate() {
            for k in 0..mf.slots.len() {
                slots.push((m, k));
            }
        }
        let island = !slots.is_empty() || outside.others.0 > 0 || !outside.parts.is_empty();
        out.island = island;
        // the own-bike filter: the run with the rider as A (the city's level index is 0)
        if !slots.is_empty() {
            let others = vec![0u16; slots.len()];
            let mut enabled: Vec<bool> = slots.iter().map(|&(m, k)| self.city.manifolds[m].slots[k].enabled).collect();
            self.gate.own_bike_run(self.own_bike, &others, &mut enabled);
            for (&(m, k), e) in slots.iter().zip(enabled) {
                self.city.manifolds[m].slots[k].enabled = e;
            }
        }
        if !island {
            self.end_tick();
            return out;
        }
        // the island build: the hinges above every touched part
        for &(m, _) in &slots {
            let part = self.city.manifolds[m].part as i32;
            self.body.contact_on_part(&self.child_link, part);
        }
        for &p in &outside.parts {
            self.body.contact_on_part(&self.child_link, p);
        }
        let mut used: Vec<(usize, usize)> = Vec::new();
        let mut touches: Vec<Touch> = Vec::new();
        for &(m, k) in &slots {
            let mf = &self.city.manifolds[m];
            let s = &mf.slots[k];
            if !s.enabled {
                continue;
            }
            let link = part_link(&self.child_link, n_links, mf.part as i32);
            let mut t = s.touch;
            t.jointed = Some(JointedSide { body: 0, link, on_b: false, point_w: POINT_W });
            // (+0 xyz is 0 at every push solve's start in ro.0: island_rows.md 11)
            t.pushed = Vec3::ZERO;
            touches.push(t);
            used.push((m, k));
        }
        out.contacts = touches.len();
        let inv_dt = 1.0 / dt;
        let body = std::slice::from_mut(&mut self.body);
        let mut solved = solver::island_solve_jointed(&[], &touches, body, outside.others, dt, inv_dt);
        solver::island_apply_jointed(&solved, body);
        solver::island_impulses(&solved, &mut touches);
        // the post-solve run: the body fall at rd+64
        if !slots.is_empty() {
            let others = vec![0u16; slots.len()];
            out.body_fall = self.gate.body_fall_run(self.own_bike, &others, rows3(&self.rd16)[3]);
        }
        out.motion = self.body.motion_measure(dt);
        self.body.velocity_update(dt);
        let body = std::slice::from_mut(&mut self.body);
        solver::island_push_jointed(&[], &mut touches, &mut solved, body, dt, inv_dt, &mut []);
        self.body.post_push(dt);
        self.rd16 = self.instance();
        for (&(m, k), t) in used.iter().zip(touches.iter()) {
            let s = &mut self.city.manifolds[m].slots[k].touch;
            s.kept = t.kept;
            s.impulse = t.impulse;
            s.pushed = t.pushed;
        }
        // 4. slot 14
        self.end_tick();
        out
    }

    /// The collider's slot 14 (F-35284): the part matrices (F-43257), then F-32811.
    fn end_tick(&mut self) {
        let links = self.link_frames();
        let _ = tick_parts(&self.ty, &links, &self.child_link, self.body.place, &self.rd16, &self.absent, &mut self.parts);
        self.body.apply_push();
        self.rd16 = self.instance();
        let rd16 = self.rd16;
        let raw = &mut self.body.raw_collider;
        for (r, v) in rd16.iter().enumerate() {
            wr(raw, 32 + 16 * r, *v);
        }
        let (deep, count, set) = (raw[658], raw[659], raw[660]);
        if deep == 0 {
            for (r, v) in rd16.iter().enumerate() {
                wr(raw, 96 + 16 * r, *v);
            }
            raw[660] = 1;
        } else if set == 0 || count > 8 {
            for (r, v) in rd16.iter().enumerate() {
                wr(raw, 96 + 16 * r, *v);
            }
        }
        if deep != 0 {
            raw[658] = 0;
        } else {
            raw[659] = 0;
        }
    }

    /// The drawn skeleton's world matrices (F-46606): from the part matrices and rd+16 as the last tick left them.
    pub fn bones(&self) -> Vec<Mat> {
        bone_globals(&self.ty, &self.locals, &self.parts, &self.rd16, &[])
    }

    /// The rider's velocity while ragdolled (character slot 11, F-62429): the level's collider's +272, the
    /// velocity slot 10 copies from the link of bone 0.
    pub fn velocity(&self) -> Vec3 {
        let v = self.body.mirror[0].0;
        Vec3::new(v[0], v[1], v[2])
    }

    /// The rider's matrix while ragdolled (character slot 10, F-31677): rd+16.
    pub fn matrix(&self) -> Mat {
        self.rd16
    }
}

// ---------------------------------------------------------------------------------------------
// Test: the chain from the recorded state at slot 5's entry of tick 169 (the first tick after GoRagdoll), each
// tick's output fed to the next, against the recording's state at the next tick's slot 5 entry.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::artic::{BODY_SIZE, COLLIDER_SIZE, LINK_SIZE};
    use std::collections::HashMap;

    struct State {
        collider: Vec<u8>,
        body: Vec<u8>,
        links: Vec<(u32, Vec<u8>)>,
        joints: Vec<Vec<u8>>,
    }
    struct Case {
        tick: u32,
        v1: f32,
        inp: State,
        out: State,
    }

    fn le(d: &[u8], o: &mut usize) -> u32 {
        let x = u32::from_le_bytes(d[*o..*o + 4].try_into().unwrap());
        *o += 4;
        x
    }
    fn take(d: &[u8], o: &mut usize, n: usize) -> Vec<u8> {
        let b = d[*o..*o + n].to_vec();
        *o += n;
        b
    }
    fn state(d: &[u8], o: &mut usize) -> State {
        le(d, o);
        let collider = take(d, o, COLLIDER_SIZE);
        le(d, o);
        let body = take(d, o, BODY_SIZE);
        let nl = le(d, o);
        let links = (0..nl).map(|_| (le(d, o), take(d, o, LINK_SIZE))).collect();
        let nj = le(d, o);
        let joints = (0..nj)
            .map(|_| {
                le(d, o);
                le(d, o);
                let sz = le(d, o) as usize;
                take(d, o, sz)
            })
            .collect();
        State { collider, body, links, joints }
    }
    /// testdata/ragdoll_run_ro0.bin (work/ragdoll/run/export_run.py): per tick 169 - 176, RGD0 (slot 5's entry) and
    /// RGX2 (the post-push's exit).
    fn cases() -> Vec<Case> {
        let d: &[u8] = include_bytes!("testdata/ragdoll_run_ro0.bin");
        assert_eq!(&d[..4], b"RGF1");
        let mut o = 4;
        let n = le(d, &mut o);
        (0..n)
            .map(|_| {
                o += 4;
                let tick = le(d, &mut o);
                le(d, &mut o);
                let v1 = f32::from_bits(le(d, &mut o));
                o += 12;
                let inp = state(d, &mut o);
                let out = state(d, &mut o);
                Case { tick, v1, inp, out }
            })
            .collect()
    }

    fn load(s: &State) -> Articulated {
        let links: Vec<(u32, &[u8])> = s.links.iter().map(|(a, b)| (*a, b.as_slice())).collect();
        let joints: Vec<&[u8]> = s.joints.iter().map(|b| b.as_slice()).collect();
        Articulated::from_guest(&s.collider, &s.body, &links, &joints)
    }

    /// Words that differ (both NaN counted apart).
    fn compare(a: &Articulated, want: &State) -> (Vec<String>, usize) {
        let (mut bad, mut nan) = (Vec::new(), 0);
        let mut one = |name: String, got: &[u8], want: &[u8]| {
            for o in (0..want.len()).step_by(4) {
                let g = u32::from_be_bytes(got[o..o + 4].try_into().unwrap());
                let w = u32::from_be_bytes(want[o..o + 4].try_into().unwrap());
                if g == w {
                    continue;
                }
                if f32::from_bits(g).is_nan() && f32::from_bits(w).is_nan() {
                    nan += 1;
                } else {
                    bad.push(format!("{name}+{o}: {g:08x} want {w:08x} ({} / {})", f32::from_bits(g), f32::from_bits(w)));
                }
            }
        };
        one("collider".into(), &a.collider_to_guest(), &want.collider);
        one("body".into(), &a.body_to_guest(), &want.body);
        for i in 0..a.links.len() {
            one(format!("link{i}"), &a.links[i].to_guest(), &want.links[i].1);
        }
        for k in 0..a.joints.len() {
            one(format!("joint{k}"), &a.joint_to_guest(k), &want.joints[k]);
        }
        (bad, nan)
    }

    /// The island's other share per tick (testdata/rider_island_run_ro0.bin, work/ragdoll/island/
    /// export_island_contacts.py over 169 - 180): the bike's own contacts (side 2: count, elasticity 0.1 each) and
    /// every rider part of the island's manifolds; also the part -> link map and each tick's contact count by side.
    fn outside() -> (Vec<i32>, HashMap<u32, (Outside, [usize; 4])>) {
        let b: &[u8] = include_bytes!("testdata/rider_island_run_ro0.bin");
        assert_eq!(&b[..8], b"RGISL001");
        let w: Vec<u32> = b[8..].chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let (ticks, mut i) = (w[0] as usize, 1);
        let n = w[i] as usize;
        let map: Vec<i32> = w[i + 1..i + 1 + n].iter().map(|&x| x as i32).collect();
        i += 1 + n;
        let mut out = HashMap::new();
        for _ in 0..ticks {
            let len = w[i] as usize;
            let t = &w[i + 1..i + 1 + len];
            i += 1 + len;
            let mut k = 4;
            let mut o = Outside::default();
            let mut sides = [0; 4];
            for _ in 0..t[3] {
                sides[t[k] as usize] += 1;
                if t[k] == 2 {
                    o.others = (o.others.0 + 1, o.others.1 + 0.1);
                }
                k += 56;
            }
            o.parts = t[k + 1..k + 1 + t[k] as usize].iter().map(|&x| x as i32).collect();
            out.insert(t[0], (o, sides));
        }
        (map, out)
    }

    /// A fixture of named sections of u32 words (rider_contacts_ro0.bin, ragdoll_pose_ro0.bin).
    fn sections(d: &[u8], magic: &[u8]) -> HashMap<String, Vec<u32>> {
        assert_eq!(&d[..8], magic);
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let mut o = 12;
        let mut m: HashMap<String, Vec<u32>> = HashMap::new();
        for _ in 0..u(8) {
            let name = String::from_utf8(d[o..o + 8].iter().copied().take_while(|&b| b != 0).collect()).unwrap();
            let n = u(o + 8) as usize;
            m.insert(name, (0..n).map(|i| u(o + 12 + 4 * i)).collect());
            o += 12 + 4 * n;
        }
        m
    }

    fn mats(m: &HashMap<String, Vec<u32>>, n: &str) -> Vec<Mat> {
        let w = &m[n];
        (0..w.len() / 16).map(|i| std::array::from_fn(|r| V4::from_bits(std::array::from_fn(|l| w[16 * i + 4 * r + l])))).collect()
    }

    /// bound +132 / +136 at slot 10's entry of tick 169 (testdata/rider_contacts_ro0.bin): +136 is the activation's
    /// copy, which slot 5 does not touch.
    fn parts_169() -> Parts {
        let m = sections(include_bytes!("testdata/rider_contacts_ro0.bin"), b"RGCONT01");
        Parts { cur: mats(&m, "cu1_169"), prev: mats(&m, "pv1_169") }
    }

    fn rider_type(root: &std::path::Path) -> RagdollType {
        crate::assets::load_resource_file(&root.join("resources/character/drv_mp_01_mot_set/0x5eb8d142.r6"))
            .ok()
            .and_then(|r| mcla_formats::rage::ragdoll::parse_character_ragdoll(&r))
            .expect("rider type")
    }

    /// GoRagdoll's part of the activation on the recorded blocks of tick 169 with every word it writes spoilt: the
    /// eject clip's last frame and the character matrix (testdata/ragdoll_pose_ro0.bin), the eject vector seq+144 of
    /// ro.0's GoRagdoll. Every word of links, joints, body and collider equals RGD0 169 (slot 5's entry of the first
    /// tick), the velocities' lane w apart (seq+144's lane w is not known: 0 here, a NaN in the recording).
    #[test]
    fn go_ragdoll_matches_recording() {
        let Some(root) = data_dir() else {
            eprintln!("skipped: the extracted data (MCLA_DATA_DIR) is missing");
            return;
        };
        let ty = rider_type(&root);
        let pose = sections(include_bytes!("testdata/ragdoll_pose_ro0.bin"), b"RGPOSE01");
        let (locals, character) = (mats(&pose, "locals"), mats(&pose, "r_act")[0]);
        let s = &cases()[0].inp;
        assert_eq!(cases()[0].tick, 169);
        let junk = |b: &mut Vec<u8>, from: usize, to: usize| b[from..to].iter_mut().for_each(|x| *x = 0x41);
        let mut links: Vec<Vec<u8>> = s.links.iter().map(|l| l.1.clone()).collect();
        for l in links.iter_mut() {
            junk(l, 416, 480);
            junk(l, 608, 736);
            junk(l, 880, 1008);
        }
        let mut joints: Vec<(u32, Vec<u8>)> = Vec::new();
        for j in &s.joints {
            let mut b = j.clone();
            let ty = u32::from_be_bytes(b[0..4].try_into().unwrap());
            junk(&mut b, 304, 432);
            if ty == 0 {
                junk(&mut b, 628, 632);
            } else {
                junk(&mut b, 652, 660);
            }
            joints.push((ty, b));
        }
        let mut collider = s.collider.clone();
        junk(&mut collider, 32, 160);
        junk(&mut collider, 272, 464);
        let (map, _) = outside();
        let build = ArticBuild { body: s.body.clone(), links, joints, collider, child_link: map, pair_a: vec![-1], pair_b: vec![-1], link_child: vec![], parents: vec![] };
        let launch = Vec3::new(f32::from_bits(0x3f9e_1bca), f32::from_bits(0xbec6_9cc7), f32::from_bits(0x4017_850c));
        let rd = RiderRagdoll::go(&ty, build, &locals, character, launch, &[]);
        let (bad, nan) = compare(&rd.body, s);
        let lane_w = |b: &String| {
            let (name, rest) = b.split_once('+').unwrap();
            let o: usize = rest.split(':').next().unwrap().parse().unwrap();
            (name.starts_with("link") && (o == 620 || o == 668)) || (name == "collider" && (o == 284 || o == 316))
        };
        let other: Vec<&String> = bad.iter().filter(|b| !lane_w(b)).collect();
        eprintln!("GoRagdoll: {} words differ, {} of them the velocities' lane w; {nan} NaN on both sides", bad.len(), bad.len() - other.len());
        assert!(other.is_empty(), "{:#?}", &other[..other.len().min(16)]);
        assert_eq!(rd.velocity().to_array().map(f32::to_bits), [0x3f9e_1bca, 0x3f1c_b19c, 0x4017_850c]);
    }

    fn data_dir() -> Option<std::path::PathBuf> {
        let root = std::env::var_os("MCLA_DATA_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("D:/MCLA/extracted/cache"));
        root.join("resources/character/drv_mp_01_mot_set/0x5eb8d142.r6").exists().then_some(root)
    }

    /// Ticks 169 - 175 of ro.0, chained: the recorded state at slot 5's entry of 169, the city from the extracted
    /// data, the island's other share (the bike's contacts) from the recording; after each tick every word of the
    /// links, joints, body and collider against the recorded state at the next tick's slot 5 entry (and the
    /// post-push's exit of the same tick, which differs from it only in what slot 14 writes).
    #[test]
    fn chained_ticks_match_recording() {
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
        let ty = rider_type(&root);
        let cases = cases();
        let (map, extra) = outside();
        let body = load(&cases[0].inp);
        let mut rd = RiderRagdoll::assemble(&ty, body, map, vec![-1], vec![-1], parts_169(), &[], &Surface::table(&rs));
        rd.own_bike = 0x56d7;
        let mut exact = Vec::new();
        let mut first_bad: Option<(u32, Vec<String>)> = None;
        for i in 0..cases.len() - 1 {
            let c = &cases[i];
            let (o, sides) = extra.get(&c.tick).cloned().unwrap_or_default();
            let got = rd.tick_with(c.v1, &city, &o);
            let (post, _) = compare(&rd.body, &c.out);
            let (bad, nan) = compare(&rd.body, &cases[i + 1].inp);
            eprintln!(
                "tick {}: {} city contacts (recorded: {} rider / {} bike / {} self), body fall {:?}; {} words differ from RGD0 {} ({nan} NaN on both sides), {} from RGX2 {}",
                c.tick,
                got.contacts,
                sides[0] + sides[1],
                sides[2],
                sides[3],
                got.body_fall,
                bad.len(),
                cases[i + 1].tick,
                post.len(),
                c.tick
            );
            if bad.is_empty() {
                exact.push(c.tick);
            } else {
                for b in bad.iter().take(12) {
                    eprintln!("    {b}");
                }
                if first_bad.is_none() {
                    first_bad = Some((c.tick, bad.clone()));
                }
            }
            if c.tick == 169 {
                assert!(got.body_fall.is_some(), "the body fall plays on the first run");
            }
        }
        eprintln!("exact ticks: {exact:?}");
        assert_eq!(exact, [169, 170, 171, 172, 173, 174], "first difference: {first_bad:?}");
    }
}
