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
//!    collider +96; the self pairs (F-41849, [`RiderSelf::collide`]) behind their gate. The rider against the bike
//!    and other cars is not collided here: the bike's rows come in as [`BikeRows`] (their places in the row order
//!    and the gate's runs); an enabled contact with the bike or another car is not solved (open).
//! 3. Update (F-96487): the contacts refreshed (F-26862 / F-33100) and their materials (F-43430, F-22759); the island
//!    build: the own-bike filter (F-37566 over the rider's two runs, F-97446), the hinges' contact bytes (F-18964, every contact of the
//!    island's manifolds with a side on the rider); the velocity solve with the joint-limit rows and the apply
//!    (F-24076); the body-fall trigger (F-15373, the same two runs, F-15350); slot 15 (F-32404); slot 10 (F-69485); the push
//!    solve (F-49764) and its apply with the post-push (F-48210, whose re-centre ends with F-18040 again).
//! 4. The collider's slot 14 (F-35284): the part matrices (F-43257), then the base F-32811: the pending push
//!    (F-67019), rd+16 (F-18040), collider +32 = rd+16 and, the deep byte +658 clear, +96 = rd+16 and +660 = 1;
//!    +659 = 0.
//!
//! The island's rows (the city's of type 7, the self contacts' of type 13) in the original's list order
//! ([`island_rows`]: the pairs' union-find walk sorted by instance A with the STLport sort, not stable above 16 rows -
//! so the bike's rows, solved or not, move the rider's: ro.0 194). The rider's runs for the callbacks: his rows as A
//! in that list, then his rows as B in the list sorted by B (a self contact is in both). What the rest of the island
//! brings (the bike lying beside the rider: its contacts' count and elasticity in the island's factor isl +40, its
//! rows' places; the rider's parts in his contacts with the bike, which set the hinge bytes too) comes in as
//! [`Outside`].
//!
//! Open (nothing guessed): a tick where the rider is in no island (no contact at all; in ro.0 he always is: the
//! velocity and push solves, slot 15 and the post-push are skipped here, the post-push being called from the push
//! apply); the bound's
//! box the resting test reads (F-65996 per slot 14, not ported: the recorded corners are used); lane w of the
//! eject vector seq+144; the bound's part matrices before the activation (bound+132 from the cache entry's init,
//! which the activation copies into +136); the instances' address order (city < bike < rider) is the recordings',
//! not a decoded rule.

use super::artic::island::part_link;
use super::artic::{Articulated, Surroundings, V4};
use super::artic_build::ArticBuild;
use super::artic_collide::{moved_parts, rows3, self_pairs, shapes, world_parts, ContactGate, PartShape, RiderCity, RiderSelf, RiderVeh, VehSide};
use super::artic_pose::{activate_parts, bone_globals, fk, seed_links, tick_parts, LinkFrame, Mat, Parts};
use super::collision::{Surface, WorldCollider};
use super::loose::stl_sort;
use super::solver::{self, JointedSide, Touch};
use glam::Vec3;
use mcla_formats::rage::ragdoll::RagdollType;

/// What slot 5 reads outside the body for the rider (the recording's RGDS pages): the world's gravity (run-time
/// (0, -9.8, 0, 0)), the rider archetype's damping (0.02 / 0.02 / 0.01, lane w as stored) and bound corners, the
/// instance flag clear, the resting limit 0.007. The corners are those of ro.0's bound (the per-tick box, F-65996,
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
/// Lane w of B's point (contact +76) in the rider's self contacts of ro.0 (a NaN, 0x7fc00001); A's is FLT_MAX.
const POINT_W_B: f32 = f32::from_bits(0x7fc0_0001);

/// The island's share that is not the rider's own contacts with the city.
#[derive(Clone, Debug, Default)]
pub struct Outside {
    /// the island's enabled contacts that share no row with the rider (the bike lying on the street): their count
    /// and the sum of their elasticities (isl +40)
    pub others: (usize, f32),
    /// the rider's parts in the island's other contacts (enabled or not: the bike's; the self contacts' are the
    /// tick's own): F-18964 marks their hinges
    pub parts: Vec<i32>,
    /// the bike's rows in the island (they take places in its row order): None, the rider's island holds only his own
    pub bike: Option<BikeRows>,
    /// another vehicle the rider's collide met this tick ([`RiderRagdoll::collide_other`]): its rows' places in the
    /// island's order; None, no other vehicle
    pub other: Option<OtherRows>,
    /// the loose world's island that vehicle is in (its other pairs: a semi's hitch, its contacts with the city or the
    /// props): one island with the rider's ([`LooseRows`]); None, the rider's pairs alone
    pub loose: Option<LooseRows>,
}

/// The loose world's share of the rider's island (one island build per tick, F-96487 / F-14298): the other pairs of the
/// island his partner vehicle is in, in the loose world's cache order, and the keys his own pairs take in that world (the
/// level indices for the union-find and the cache order, the instances' addresses for the sort).
#[derive(Clone, Debug, Default)]
pub struct LooseRows {
    /// the island's pairs that are not the rider's ([`WalkNode`]: a constraint's pair is walked last, F-87905)
    pub nodes: Vec<WalkNode>,
    pub rider_level: i64,
    pub rider_key: i64,
    pub city_key: i64,
    pub other_key: i64,
    /// the bike's instance key in that world (the car's own, `LooseState::car_place`): its rows with the city and the
    /// rider sort by it when the bike is in the island (the step with his bike and loose objects together)
    pub bike_key: i64,
}

/// A pair of objects as the merged island walk takes it ([`merged_rows`]): the two objects' level indices (None: the city,
/// which joins nothing), the instance addresses of side A and B of each of its rows in the walk's order (the sub-manifolds
/// in their order, contacts by number, the disabled ones included; a pair whose rows do not all name the same side A - a
/// semi's hitch beside the cab x trailer contacts - keeps each row's own), and whether it is a constraint's pair.
#[derive(Clone, Debug, Default)]
pub struct WalkNode {
    pub a: Option<i64>,
    pub b: Option<i64>,
    pub keys: Vec<(i64, i64)>,
    pub constraint: bool,
}

/// The cache's order of `nodes` as the island build walks them (F-14298): the pairs by (lower, higher) level - the city
/// lowest - in a stable sort, then the constraints' pairs in their order (F-87905's contacts are made after the collide).
/// The constraints last is yc.0's (813 - 817: of every place for the hitch's node only the last rebuilds both recorded
/// lists of all five ticks). Answers the nodes' numbers in that order.
pub fn cache_order(nodes: &[WalkNode]) -> Vec<usize> {
    let key = |n: &WalkNode| {
        let (a, b) = (n.a.unwrap_or(i64::MIN), n.b.unwrap_or(i64::MIN));
        (a.min(b), a.max(b))
    };
    let mut order: Vec<usize> = (0..nodes.len()).filter(|&k| !nodes[k].constraint).collect();
    order.sort_by_key(|&k| key(&nodes[k]));
    order.extend((0..nodes.len()).filter(|&k| nodes[k].constraint));
    order
}

/// The row lists of the island that holds `of` over nodes already in the cache's order: the union-find walk of
/// [`island_walk`], sorted by each row's instance A address and, apart, by its instance B address with the STLport sort
/// (as [`island_rows`]). Rows as (node, row of the node).
pub fn merged_rows(nodes: &[WalkNode], of: i64) -> Option<(Vec<(usize, usize)>, Vec<(usize, usize)>)> {
    let ends: Vec<(Option<i64>, Option<i64>)> = nodes.iter().map(|n| (n.a, n.b)).collect();
    let rows: Vec<usize> = nodes.iter().map(|n| n.keys.len()).collect();
    let walk = island_walk(&ends, &rows, of)?;
    let mut by_a = walk.clone();
    stl_sort(&mut by_a, |x| nodes[x.0].keys[x.1].0);
    let mut by_b = walk;
    stl_sort(&mut by_b, |x| nodes[x.0].keys[x.1].1);
    Some((by_a, by_b))
}

/// Another vehicle's share of the rider's island (an ambient car: ov.0 724 - 735).
#[derive(Clone, Debug, Default)]
pub struct OtherRows {
    /// the level indices of the vehicle and the rider (the pair's order: the lower is A). ov.0 had the car below the
    /// rider (0x5745 / 0x5754); the order in general follows the level's free index list - the port keeps ov.0's
    pub level: i64,
    pub rider_level: i64,
    /// the rider x vehicle island: the vehicle is body 0 of the solve and its rows with the rider are rows of it
    /// ([`RiderRagdoll::with_car`]); false: the rows only join and count in the callbacks' runs (an island the port does
    /// not solve together - the bike in it too, or the vehicle in other pairs of the loose world)
    pub shared: bool,
    /// an mcCar's callback on the rider's contacts (F-55998, loose.rs `rider_touch`): friction, elasticity and the
    /// drop of the car's own point; None for an ambient car (its callbacks leave them) or a car the branch passes over
    pub touch: Option<super::loose::RiderTouch>,
}

/// A vehicle's level as the rider's runs (F-37566 / F-15373) see it: never the own bike's.
const OTHER_RUN_LEVEL: u16 = 0xfffe;

/// The bike's share of the rider's island as its row order needs it.
#[derive(Clone, Debug, Default)]
pub struct BikeRows {
    /// the level indices of the bike and the rider (the cache's pair order and its side A: the lower one)
    pub level: i64,
    pub rider_level: i64,
    /// the bike's contacts with the city in the walk's order (sub-manifolds, contacts by number - those of the wheel
    /// parts the update leaves in the list counted in); None: the pair held no contact at the collide's end
    pub city: Option<usize>,
    /// the bike x rider contacts in the walk's order: their enabled bit (+165 & 0x80), which the own-bike runs clear;
    /// None as for `city`. Used when the rider's collide against the bike did not run this tick (the recorded share of
    /// a test); else his own [`RiderVeh`] holds them.
    pub rider: Option<Vec<bool>>,
    /// the bike's city rows' enabled bits (empty: all enabled)
    pub city_on: Vec<bool>,
    /// the bike's callback on its enabled contacts with the rider (F-55998 in its runs): BoundFriction,
    /// BoundElasticity while the bike is wiped out (chassis +592 bits 0x10 / 0x04), None otherwise (nothing written)
    pub touch: Option<(f32, f32)>,
    /// the shared step: the bike is body 0 of the island's solve, its city rows and the bike x rider rows are rows of
    /// it ([`RiderIsland::rows`])
    pub shared: bool,
}

/// A row of the shared island in its order: the bike's contact with the city (its number among the bike's city rows,
/// the disabled ones counted), or one of the rider's (an index into [`RiderIsland::touches`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IslRow {
    Bike(usize),
    Rider(usize),
    /// a row of the loose world's share ([`LooseRows`]): its node and its row there, enabled or not
    Loose(usize, usize),
}

/// The rider's share of his island between the update's passes ([`RiderRagdoll::build`] .. [`RiderRagdoll::finish`]).
#[derive(Clone, Debug, Default)]
pub struct RiderIsland {
    /// his rows in the island list's order, the enabled ones (types 7 and 13); the solve's results go back into them
    pub touches: Vec<Touch>,
    used: Vec<(Src, usize, usize)>,
    /// the other instances' level indices of his two runs (as A, as B), the empty ones left out
    runs: Vec<Vec<u16>>,
    /// he is in an island
    pub island: bool,
    /// how many of the rows are his contacts with the city
    pub city_rows: usize,
    /// the shared step: every enabled row of the island in its order
    pub rows: Vec<IslRow>,
    /// his pair with the bike holds contacts (the disabled ones too): the bike is in his island
    pub with_bike: bool,
    /// his pair with another vehicle holds contacts: that vehicle is in his island
    pub with_other: bool,
    /// the island's two lists (by instance A, by instance B: F-97446 / F-15350 walk them) as the merged build made them:
    /// every row of the bike's with the city and of the loose world's share, the disabled ones too, and the rider's
    /// enabled rows; empty outside a merged build ([`Outside::loose`])
    pub list_a: Vec<IslRow>,
    pub list_b: Vec<IslRow>,
}

impl RiderIsland {
    /// The vehicle side of his row `k` (an index into `touches`): Some(true) the bike, Some(false) the other vehicle,
    /// None a row with the city or a self contact.
    pub fn partner(&self, k: usize) -> Option<bool> {
        match self.used.get(k).map(|u| u.0) {
            Some(Src::BikeRider) => Some(true),
            Some(Src::OtherRider) => Some(false),
            _ => None,
        }
    }

    /// (tests) His share as a recording gives it: his rows, every enabled row of the island in its order, and each of his
    /// rows' vehicle side as [`Self::partner`] answers it.
    #[cfg(test)]
    pub fn recorded(touches: Vec<Touch>, rows: Vec<IslRow>, partners: &[Option<bool>]) -> RiderIsland {
        let used = partners
            .iter()
            .map(|p| match p {
                Some(true) => (Src::BikeRider, 0, 0),
                Some(false) => (Src::OtherRider, 0, 0),
                None => (Src::City, 0, 0),
            })
            .collect();
        RiderIsland { touches, used, island: true, rows, ..Default::default() }
    }
}

/// Where a row of the rider's island comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Src {
    City,
    Own,
    BikeCity,
    BikeRider,
    OtherRider,
    Loose(usize),
}

/// One pair of an island for its row order: the two objects (the level index; None: the city, which joins
/// nothing), the order of the instances' addresses of side A and B (the sort's keys) and the rows it holds in the
/// walk's order (its sub-manifolds in their array's order, contacts by number, the disabled ones included).
#[derive(Clone, Copy, Debug)]
pub struct WalkPair {
    pub a: Option<i64>,
    pub b: Option<i64>,
    pub key_a: i64,
    pub key_b: i64,
    pub rows: usize,
}

/// F-78981: the other instance of a row seen from a run (the callbacks walk the list sorted by A, then the one sorted by
/// B): the run of the A-side instance (`as_a`) gets side B, the other side A (the city is 0). The original answers side A
/// unless that is the run's own instance, then side B; the two agree wherever the row belongs to the run (a self contact
/// has the rider on both sides).
fn other_level(w: &WalkPair, as_a: bool) -> i64 {
    if as_a {
        w.b.unwrap_or(0)
    } else {
        w.a.unwrap_or(0)
    }
}

/// The row lists of the island that holds object `of` (docs/reference/vehicle/gjk_exact.md Part 10 item 4 and
/// ragdoll_self_contacts.md 8): `pairs` in the cache's order - those that held a contact at the collide's end, the
/// ones the update's refresh then empties too (rows 0: they still join) - each a node of a union-find forest
/// (F-14298 at the end of the collide; FIND F-28830 hangs the nodes it passes at the tail of the root's children,
/// UNION F-91971 by rank - as loose.rs builds a car's islands); the tree of `of` walked in pre-order, every row of
/// each pair; that walk sorted by instance A's address and by instance B's with the STLport sort ([`stl_sort`]:
/// not stable above 16 rows). Rows as (pair, row of the pair); None when no pair holds `of`.
pub fn island_rows(pairs: &[WalkPair], of: i64) -> Option<(Vec<(usize, usize)>, Vec<(usize, usize)>)> {
    let ends: Vec<(Option<i64>, Option<i64>)> = pairs.iter().map(|p| (p.a, p.b)).collect();
    let rows: Vec<usize> = pairs.iter().map(|p| p.rows).collect();
    let walk = island_walk(&ends, &rows, of)?;
    let mut by_a = walk.clone();
    stl_sort(&mut by_a, |x| pairs[x.0].key_a);
    let mut by_b = walk;
    stl_sort(&mut by_b, |x| pairs[x.0].key_b);
    Some((by_a, by_b))
}

/// The union-find walk of [`island_rows`] alone: `ends` the pairs' two objects in the cache's order (None: the city),
/// `rows` how many rows each holds; the tree of `of` in pre-order, every row of each pair, as (pair, row). None when
/// no pair holds `of`.
pub fn island_walk(ends: &[(Option<i64>, Option<i64>)], rows: &[usize], of: i64) -> Option<Vec<(usize, usize)>> {
    let live: Vec<usize> = (0..ends.len()).collect();
    let np = live.len();
    let mut parent: Vec<Option<usize>> = vec![None; np];
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); np];
    let mut rank = vec![0u32; np];
    let mut roots: Vec<usize> = Vec::new();
    let mut link: Vec<(i64, usize)> = Vec::new();
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
    fn find(parent: &mut [Option<usize>], kids: &mut [Vec<usize>], roots: &mut Vec<usize>, x: usize) -> usize {
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
            x
        } else if rank[x] > rank[y] {
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
        let mut r = find(&mut parent, &mut kids, &mut roots, k);
        let p = ends[live[k]];
        let mut ends: Vec<i64> = p.0.into_iter().collect();
        if let Some(b) = p.1.filter(|b| Some(*b) != p.0) {
            ends.push(b);
        }
        for e in &ends {
            if let Some(&(_, l)) = link.iter().find(|x| x.0 == *e) {
                let x = find(&mut parent, &mut kids, &mut roots, l);
                r = union(&mut parent, &mut kids, &mut roots, &mut rank, x, r);
            }
            match link.iter_mut().find(|x| x.0 == *e) {
                Some(x) => x.1 = r,
                None => link.push((*e, r)),
            }
        }
        for e in &ends {
            link.iter_mut().find(|x| x.0 == *e).unwrap().1 = r;
        }
    }
    let holds = |k: usize| ends[live[k]].0 == Some(of) || ends[live[k]].1 == Some(of);
    let mut walk: Vec<(usize, usize)> = Vec::new();
    for &root in roots.iter() {
        let mut tree = Vec::new();
        let mut stack = vec![root];
        while let Some(nd) = stack.pop() {
            tree.push(nd);
            stack.extend(kids[nd].iter().rev());
        }
        if tree.iter().any(|&k| holds(k)) {
            walk = tree.iter().flat_map(|&k| (0..rows[live[k]]).map(move |r| (k, r))).map(|(k, r)| (live[k], r)).collect();
            break;
        }
    }
    if walk.is_empty() {
        return None;
    }
    Some(walk)
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
    /// the rider's sub-manifolds with himself
    pub own: RiderSelf,
    /// the rider's sub-manifolds with his bike, and the bike's part matrices of this tick's collide (empty: no
    /// collide against the bike this tick)
    pub veh: RiderVeh,
    pub veh_now: Vec<[Vec3; 4]>,
    /// the rider's sub-manifolds with another vehicle (an ambient car) and its part matrix of this tick's collide
    /// (empty: none met this tick)
    pub other: RiderVeh,
    pub other_now: Vec<[Vec3; 4]>,
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
            own: RiderSelf::default(),
            veh: RiderVeh::default(),
            veh_now: Vec::new(),
            other: RiderVeh::default(),
            other_now: Vec::new(),
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

    /// One physics tick in the original's order (module doc), dt the step (1 / 60): the passes below with the rider
    /// alone in his island (the bike's share as `outside`).
    pub fn tick_with(&mut self, dt: f32, city: &WorldCollider, outside: &Outside) -> TickOut {
        let mut out = TickOut::default();
        out.resting = self.pre_collide(dt);
        self.collide(city);
        let isl = self.build(outside);
        self.alone(isl, outside.others, dt, &mut out);
        out
    }

    /// The rest of a tick with the rider in an island of his own (or none): the velocity solve, the post-solve
    /// passes, the push and the post-push. `others` as [`Outside::others`].
    pub fn alone(&mut self, mut isl: RiderIsland, others: (usize, f32), dt: f32, out: &mut TickOut) {
        out.island = isl.island;
        out.contacts = isl.city_rows;
        if !isl.island {
            self.lone_end(dt);
            return;
        }
        let inv_dt = 1.0 / dt;
        let body = std::slice::from_mut(&mut self.body);
        let mut solved = solver::island_solve_jointed(&[], &isl.touches, body, others, dt, inv_dt);
        solver::island_apply_jointed(&solved, body);
        solver::island_impulses(&solved, &mut isl.touches);
        let (fall, motion) = self.after_solve(&isl, dt);
        out.body_fall = fall;
        out.motion = motion;
        let body = std::slice::from_mut(&mut self.body);
        solver::island_push_jointed(&[], &mut isl.touches, &mut solved, body, dt, inv_dt, &mut []);
        self.finish(&isl, dt);
    }

    /// 1. PreCollide: slot 5 (F-75381), with rd+16 and the part matrices of its move. Answers the resting flag.
    pub fn pre_collide(&mut self, dt: f32) -> bool {
        self.body.step(dt, &self.world);
        let resting = self.body.resting != 0;
        self.rd16 = self.instance();
        let links = self.link_frames();
        // (the rider's type has part frames O: no other rule applies)
        let _ = moved_parts(&self.ty, &links, &self.child_link, self.body.place, &self.rd16, &self.absent, &mut self.parts);
        resting
    }

    /// The parts' world matrices of this tick's collide: now (+132 x rd+16) and last (+136 x collider +96).
    pub fn part_matrices(&self) -> (Vec<[Vec3; 4]>, Vec<[Vec3; 4]>) {
        let last = rows3(&mat_at(&self.body.raw_collider, 96));
        world_parts(&self.parts, &rows3(&self.rd16), &last)
    }

    /// 2. Collide: the city (F-82759) and the self pairs behind their gate (F-41849).
    pub fn collide(&mut self, city: &WorldCollider) {
        let (now, before) = self.part_matrices();
        self.city.collide(&self.shapes, &now, &before, city);
        let raw = &mut self.body.raw_collider;
        let mut countdown = i32::from_be_bytes(raw[744..748].try_into().unwrap());
        let pairs = self_pairs(raw[740] != 0, &mut countdown, &self.pair_a, &self.pair_b);
        raw[744..748].copy_from_slice(&countdown.to_be_bytes());
        if let Some(pairs) = pairs {
            self.own.collide(&pairs, &self.shapes, &now, &before);
        }
    }

    /// 2b. The collide against his bike's parts (F-82759, composite against composite; [`RiderVeh::collide`]): his
    /// parts now, and last as +136 x the collider's +32 (both instances have a collider record, F-65617). `rider_is_a`:
    /// his level index the lower.
    pub fn collide_bike(&mut self, veh: &VehSide, rider_is_a: bool) {
        let (now, _) = self.part_matrices();
        let (_, last) = world_parts(&self.parts, &rows3(&self.rd16), &rows3(&mat_at(&self.body.raw_collider, 32)));
        self.veh.rider_is_a = rider_is_a;
        self.veh.collide(&self.shapes, &now, &last, veh);
        self.veh_now = std::iter::once(veh.now).chain(veh.wheels.iter().map(|w| w.map_or(veh.now, |w| w.now))).collect();
    }

    /// 2c. The collide against another vehicle (an ambient car, its bound one polyhedron at the instance): the same
    /// composite-against-composite collide as the bike's (F-82759, [`RiderVeh::collide`]: the same collision object at
    /// manifold +1252, 0xBCEA58C0 in ov.0) with that vehicle's hull as part 0 and its collider's +32 as its last.
    /// Checked word for word on ov.0 720 - 735 (`artic_collide::tests::car_contacts_match_recording`). None: no
    /// vehicle near this tick - the pair is let go.
    pub fn collide_other(&mut self, veh: Option<(&VehSide, bool)>) {
        let Some((veh, rider_is_a)) = veh else {
            self.other = RiderVeh::default();
            self.other_now.clear();
            return;
        };
        let (now, _) = self.part_matrices();
        let (_, last) = world_parts(&self.parts, &rows3(&self.rd16), &rows3(&mat_at(&self.body.raw_collider, 32)));
        self.other.rider_is_a = rider_is_a;
        self.other.collide(&self.shapes, &now, &last, veh);
        // (every hull part of the vehicle at its instance: a two-part bound's part matrices are the identity)
        self.other_now = vec![veh.now; 1 + veh.hull2.is_some() as usize];
    }

    /// The rest of a tick with the rider and another vehicle in one island (no bike, no other row): the vehicle body 0
    /// (rigid), the rider the jointed body, every enabled row in the island's order, 8 sweeps and one elasticity factor
    /// (F-68442 / F-36821, as the bike's shared island), the vehicle's share from the island's apply (into its
    /// accumulators +368 / +384), the post-solve passes, the push solve with the vehicle's push as it stands (its +400 /
    /// +416) and the post-push. Checked on ov.0 724 - 735 (`artic_island::tests::
    /// rider_car_island_velocity_solve_is_the_originals`: 11 ticks word for word, 730 open). Answers the vehicle's
    /// impulse and turning and its push.
    pub fn with_car(&mut self, mut isl: RiderIsland, car: solver::Body, car_push: solver::Pushed, dt: f32, out: &mut TickOut) -> (Vec3, Vec3, solver::Pushed) {
        out.island = true;
        out.contacts = isl.city_rows;
        let inv_dt = 1.0 / dt;
        let bodies = [car];
        let body = std::slice::from_mut(&mut self.body);
        let mut solved = solver::island_solve_jointed(&bodies, &isl.touches, body, (0, 0.0), dt, inv_dt);
        solver::island_apply_jointed(&solved, body);
        solver::island_impulses(&solved, &mut isl.touches);
        let given = solver::island_apply(&bodies, &isl.touches);
        let (fall, motion) = self.after_solve(&isl, dt);
        out.body_fall = fall;
        out.motion = motion;
        let body = std::slice::from_mut(&mut self.body);
        let mut pushes = [car_push];
        solver::island_push_jointed(&bodies, &mut isl.touches, &mut solved, body, dt, inv_dt, &mut pushes);
        self.finish(&isl, dt);
        (given[0].0, given[0].1, pushes[0])
    }

    /// 3. The update's share of the rider before the solve: the refresh and the materials, the island's row lists
    /// ([`island_rows`]), the own-bike filter over his two runs (F-37566, F-97446), the hinges' contact bytes
    /// (F-18964) and his rows in the list's order.
    pub fn build(&mut self, outside: &Outside) -> RiderIsland {
        let (now, _) = self.part_matrices();
        // (the pairs that hold a contact now join the island, F-14298 at the collide's end: one the refresh empties too)
        let city_joins = self.city.manifolds.iter().any(|m| !m.slots.is_empty());
        let own_joins = self.own.manifolds.iter().any(|m| !m.slots.is_empty());
        let veh_real = !self.veh_now.is_empty();
        let veh_joins = veh_real && self.veh.manifolds.iter().any(|m| !m.slots.is_empty());
        self.city.refresh(&now);
        self.city.materials(&self.shapes, self.ty.f320);
        self.own.refresh(&now);
        self.own.materials(&self.shapes, self.ty.f320);
        if veh_real {
            self.veh.refresh(&now, &self.veh_now);
            self.veh.materials(&self.shapes, self.ty.f320);
        }
        // (another vehicle: its pair joins as the bike's does; its materials the rider's F-22759 - an ambient car's own
        // callbacks leave a partner of class 11 alone: its pre-solve F-65962 disables contacts with classes 4 / 5 only
        // (F-44543) and casts the partner to its own type (F-45219), its post-solve F-13089 casts it to mcCarInstance)
        let other = outside.other.as_ref().filter(|_| !self.other_now.is_empty());
        let other_joins = other.is_some() && self.other.manifolds.iter().any(|m| !m.slots.is_empty());
        if other.is_some() {
            self.other.refresh(&now, &self.other_now);
            self.other.materials(&self.shapes, self.ty.f320);
        }
        let mut orows: Vec<(usize, usize)> = Vec::new();
        for (m, mf) in self.other.manifolds.iter().enumerate().filter(|_| other.is_some()) {
            for k in 0..mf.slots.len() {
                orows.push((m, k));
            }
        }
        let mut other_rider: Vec<bool> = orows.iter().map(|&(m, k)| self.other.manifolds[m].slots[k].enabled).collect();
        let mut own: Vec<(usize, usize)> = Vec::new();
        for (m, mf) in self.own.manifolds.iter().enumerate() {
            for k in 0..mf.slots.len() {
                own.push((m, k));
            }
        }
        let n_links = self.body.links.len();
        let mut slots: Vec<(usize, usize)> = Vec::new();
        for (m, mf) in self.city.manifolds.iter().enumerate() {
            for k in 0..mf.slots.len() {
                slots.push((m, k));
            }
        }
        // (the bike x rider contacts in the pair's walk: sub-manifolds in their order, contacts by number)
        let mut vrows: Vec<(usize, usize)> = Vec::new();
        for (m, mf) in self.veh.manifolds.iter().enumerate() {
            for k in 0..mf.slots.len() {
                vrows.push((m, k));
            }
        }
        // the island's pairs in the cache's order and its row lists ([`island_rows`]): the city's pairs by the
        // object's level (the object is A), then the others by (lower, higher) level (the lower is A). The sort's keys
        // are the instances' addresses: the city's below the bike's below the rider's in ro.0, rm.0, rh.0 and rf.0.
        let bike = outside.bike.as_ref();
        let loose = outside.loose.as_ref();
        let rider = loose.map(|l| l.rider_level).or(bike.map(|b| b.rider_level)).or(other.map(|o| o.rider_level)).unwrap_or(0);
        // (the other vehicle's instance below the rider's too: ov.0's car at 0xA931B220, the rider at 0xBF5322D0; in one
        // island with the loose world, that world's own keys)
        let (city_key, bike_key, rider_key, other_key) = match loose {
            Some(l) => (l.city_key, l.bike_key, l.rider_key, l.other_key),
            None => (0i64, 1i64, 2i64, 1i64),
        };
        let shared = bike.map_or(false, |b| b.shared);
        let other_shared = other.map_or(false, |o| o.shared);
        let mut pairs: Vec<(WalkPair, Src)> = Vec::new();
        if city_joins {
            pairs.push((WalkPair { a: Some(rider), b: None, key_a: rider_key, key_b: city_key, rows: slots.len() }, Src::City));
        }
        // (the bike x rider rows' enabled bits: the real collide's, else the recorded share's)
        let mut bike_rider: Vec<bool> = if veh_real { vrows.iter().map(|&(m, k)| self.veh.manifolds[m].slots[k].enabled).collect() } else { bike.and_then(|b| b.rider.clone()).unwrap_or_default() };
        if let Some(b) = bike {
            if let Some(n) = b.city {
                pairs.push((WalkPair { a: Some(b.level), b: None, key_a: bike_key, key_b: city_key, rows: n }, Src::BikeCity));
            }
            if (veh_real && veh_joins) || (!veh_real && b.rider.is_some()) {
                let (lo, hi) = if b.level < rider { ((b.level, bike_key), (rider, rider_key)) } else { ((rider, rider_key), (b.level, bike_key)) };
                pairs.push((WalkPair { a: Some(lo.0), b: Some(hi.0), key_a: lo.1, key_b: hi.1, rows: bike_rider.len() }, Src::BikeRider));
            }
        }
        if let (Some(o), true) = (other, other_joins) {
            let (lo, hi) = if o.level < rider { ((o.level, other_key), (rider, rider_key)) } else { ((rider, rider_key), (o.level, other_key)) };
            pairs.push((WalkPair { a: Some(lo.0), b: Some(hi.0), key_a: lo.1, key_b: hi.1, rows: other_rider.len() }, Src::OtherRider));
        }
        if own_joins {
            pairs.push((WalkPair { a: Some(rider), b: Some(rider), key_a: rider_key, key_b: rider_key, rows: own.len() }, Src::Own));
        }
        let (walk, srcs, by_a, by_b) = match loose {
            None => {
                pairs.sort_by_key(|(p, _)| match p.b {
                    None => (0, p.a.unwrap_or(0)),
                    Some(b) => (1 + p.a.unwrap_or(0).min(b), p.a.unwrap_or(0).max(b)),
                });
                let (walk, srcs): (Vec<WalkPair>, Vec<Src>) = pairs.into_iter().unzip();
                let (by_a, by_b) = island_rows(&walk, rider).unwrap_or_default();
                (walk, srcs, by_a, by_b)
            }
            Some(l) => {
                // (one island with the loose world's: its pairs and his in the cache's order, every row with its own
                // instances' keys; `walk` keeps each node's ends for the runs)
                let mut nodes: Vec<(WalkNode, Src)> = pairs.into_iter().map(|(p, s)| (WalkNode { a: p.a, b: p.b, keys: vec![(p.key_a, p.key_b); p.rows], constraint: false }, s)).collect();
                nodes.extend(l.nodes.iter().cloned().enumerate().map(|(i, n)| (n, Src::Loose(i))));
                let order = cache_order(&nodes.iter().map(|x| x.0.clone()).collect::<Vec<_>>());
                let nodes: Vec<(WalkNode, Src)> = order.into_iter().map(|k| nodes[k].clone()).collect();
                let (ns, srcs): (Vec<WalkNode>, Vec<Src>) = nodes.into_iter().unzip();
                let (by_a, by_b) = merged_rows(&ns, rider).unwrap_or_default();
                let walk = ns.iter().map(|n| WalkPair { a: n.a, b: n.b, key_a: n.keys.first().map_or(0, |k| k.0), key_b: n.keys.first().map_or(0, |k| k.1), rows: n.keys.len() }).collect();
                (walk, srcs, by_a, by_b)
            }
        };
        let island = !by_a.is_empty() || outside.others.0 > 0 || !outside.parts.is_empty();
        // the callbacks' runs (F-97446): the list sorted by A walked by instance in the addresses' order, then the list
        // sorted by B - so the bike's run as A, the rider's as A, the bike's as B, the rider's as B. The rider's (F-37566)
        // writes the type's +320 into each of its contacts (F-22759), then clears the enabled bit of those with his own
        // bike (the own-bike filter); the bike's writes BoundFriction / BoundElasticity into its enabled contacts with
        // the rider (rs.0 154: 0.85 / 0 with the bike A, rh.0 208: 0.85 / 0.1 with the rider A and the gate open, rt.0:
        // the polygons' with the rider A and the gate shut - F-55998's branch for the rider's class is not decoded). The
        // other instance of a row is its other side (F-78981: the city 0, the rider himself in a self contact).
        let level_of = other_level;
        let run = |list: &[(usize, usize)], me: i64, as_a: bool| -> Vec<(usize, usize)> {
            list.iter().filter(|x| if as_a { walk[x.0].a == Some(me) } else { walk[x.0].b == Some(me) }).copied().collect()
        };
        let bike_level = bike.map(|b| b.level);
        let mut rider_runs: Vec<Vec<u16>> = Vec::new();
        for (list, as_a) in [(&by_a, true), (&by_b, false)] {
            if let (Some(bl), Some(touch)) = (bike_level, bike.and_then(|b| b.touch)) {
                for &(p, r) in &run(list, bl, as_a) {
                    if srcs[p] == Src::BikeRider && bike_rider[r] && veh_real {
                        let (m, k) = vrows[r];
                        let t = &mut self.veh.manifolds[m].slots[k].touch;
                        t.friction = touch.0;
                        t.elasticity = touch.1;
                    }
                }
            }
            // (an mcCar's run, F-55998, on its enabled contacts with the rider - before his in each list as the bike's is;
            // the two instances' address order is not kept by the port (vehsim.rs BIKE_LEVEL), so unverified in general)
            if let (Some(o), Some(touch)) = (other.filter(|_| other_joins), other.and_then(|o| o.touch)) {
                for &(p, r) in &run(list, o.level, as_a) {
                    if srcs[p] == Src::OtherRider && other_rider[r] {
                        let (m, k) = orows[r];
                        let car_a = !self.other.rider_is_a;
                        let t = &mut self.other.manifolds[m].slots[k].touch;
                        t.friction = touch.0;
                        t.elasticity = touch.1;
                        let side = if car_a { &mut t.a } else { &mut t.b };
                        side.point.y = super::loose::rider_touch_point(side.point.y, touch.2);
                    }
                }
            }
            let mine = run(list, rider, as_a);
            if mine.is_empty() {
                continue;
            }
            let others: Vec<u16> = mine.iter().map(|x| if srcs[x.0] == Src::OtherRider { OTHER_RUN_LEVEL } else { level_of(&walk[x.0], as_a) as u16 }).collect();
            let mut enabled: Vec<bool> = mine
                .iter()
                .map(|&(p, r)| match srcs[p] {
                    Src::City => self.city.manifolds[slots[r].0].slots[slots[r].1].enabled,
                    Src::Own => self.own.manifolds[own[r].0].slots[own[r].1].enabled,
                    Src::BikeRider => bike_rider[r],
                    Src::OtherRider => other_rider[r],
                    Src::BikeCity | Src::Loose(_) => true,
                })
                .collect();
            for &(p, r) in &mine {
                if srcs[p] == Src::BikeRider && veh_real {
                    let (m, k) = vrows[r];
                    self.veh.manifolds[m].slots[k].touch.elasticity = self.ty.f320;
                }
                if srcs[p] == Src::OtherRider {
                    let (m, k) = orows[r];
                    self.other.manifolds[m].slots[k].touch.elasticity = self.ty.f320;
                }
            }
            self.gate.own_bike_run(self.own_bike, &others, &mut enabled);
            for (&(p, r), e) in mine.iter().zip(enabled) {
                match srcs[p] {
                    Src::City => self.city.manifolds[slots[r].0].slots[slots[r].1].enabled = e,
                    Src::Own => self.own.manifolds[own[r].0].slots[own[r].1].enabled = e,
                    Src::BikeRider => bike_rider[r] = e,
                    Src::OtherRider => other_rider[r] = e,
                    Src::BikeCity | Src::Loose(_) => {}
                }
            }
            rider_runs.push(others);
        }
        if veh_real {
            for (&(m, k), &e) in vrows.iter().zip(bike_rider.iter()) {
                self.veh.manifolds[m].slots[k].enabled = e;
            }
        }
        for (&(m, k), &e) in orows.iter().zip(other_rider.iter()) {
            self.other.manifolds[m].slots[k].enabled = e;
        }
        let with_bike = srcs.contains(&Src::BikeRider);
        let with_other = srcs.contains(&Src::OtherRider);
        let mut isl = RiderIsland { touches: Vec::new(), used: Vec::new(), runs: rider_runs, island, city_rows: 0, rows: Vec::new(), with_bike, with_other, list_a: Vec::new(), list_b: Vec::new() };
        if !island {
            return isl;
        }
        // the island build: the hinges above every touched part
        for &(m, _) in &slots {
            let part = self.city.manifolds[m].part as i32;
            self.body.contact_on_part(&self.child_link, part);
        }
        for &(m, _) in &own {
            let mf = &self.own.manifolds[m];
            self.body.contact_on_part(&self.child_link, mf.a as i32);
            self.body.contact_on_part(&self.child_link, mf.b as i32);
        }
        if veh_joins {
            for &(m, _) in &vrows {
                let part = self.veh.manifolds[m].part as i32;
                self.body.contact_on_part(&self.child_link, part);
            }
        }
        if other_joins {
            for &(m, _) in &orows {
                let part = self.other.manifolds[m].part as i32;
                self.body.contact_on_part(&self.child_link, part);
            }
        }
        for &p in &outside.parts {
            self.body.contact_on_part(&self.child_link, p);
        }
        // the rows in the list's order (sorted by instance A), the enabled ones: the city's of type 7, the self
        // contacts' of type 13, the bike's own and the bike x rider rows in the shared step (outside it the bike's rows
        // are not the rider's: they come in as `outside.others`)
        let city_on = |r: usize| bike.map_or(true, |b| b.city_on.get(r).copied().unwrap_or(true));
        let mut named: Vec<((usize, usize), IslRow)> = Vec::new();
        for &(p, r) in &by_a {
            let before = isl.touches.len();
            match srcs[p] {
                Src::City => {
                    let (m, k) = slots[r];
                    let mf = &self.city.manifolds[m];
                    if !mf.slots[k].enabled {
                        continue;
                    }
                    let link = part_link(&self.child_link, n_links, mf.part as i32);
                    let mut t = mf.slots[k].touch;
                    t.jointed = Some(JointedSide { body: 0, link, on_b: false, point_w: POINT_W, self_b: None });
                    isl.rows.push(IslRow::Rider(isl.touches.len()));
                    isl.touches.push(t);
                    isl.used.push((Src::City, m, k));
                    isl.city_rows += 1;
                }
                Src::Own => {
                    let (m, k) = own[r];
                    let mf = &self.own.manifolds[m];
                    if !mf.slots[k].enabled {
                        continue;
                    }
                    let la = part_link(&self.child_link, n_links, mf.a as i32);
                    let lb = part_link(&self.child_link, n_links, mf.b as i32);
                    let mut t = mf.slots[k].touch;
                    t.jointed = Some(JointedSide { body: 0, link: la, on_b: false, point_w: POINT_W, self_b: Some((lb, POINT_W_B)) });
                    isl.rows.push(IslRow::Rider(isl.touches.len()));
                    isl.touches.push(t);
                    isl.used.push((Src::Own, m, k));
                }
                Src::BikeCity => {
                    if shared && city_on(r) {
                        isl.rows.push(IslRow::Bike(r));
                    }
                }
                Src::Loose(i) => isl.rows.push(IslRow::Loose(i, r)),
                Src::BikeRider => {
                    if !(shared && veh_real && bike_rider[r]) {
                        continue;
                    }
                    let (m, k) = vrows[r];
                    let mf = &self.veh.manifolds[m];
                    let link = part_link(&self.child_link, n_links, mf.part as i32);
                    let mut t = mf.slots[k].touch;
                    let on_b = !self.veh.rider_is_a;
                    // (the bike's side: body 0 of the solve, its part's rows, the composite's margin)
                    let vm = self.veh_now[mf.veh as usize];
                    let side = if on_b { &mut t.a } else { &mut t.b };
                    side.body = Some(0);
                    side.rows = [vm[0], vm[1], vm[2]];
                    t.jointed = Some(JointedSide { body: 0, link, on_b, point_w: POINT_W, self_b: None });
                    isl.rows.push(IslRow::Rider(isl.touches.len()));
                    isl.touches.push(t);
                    isl.used.push((Src::BikeRider, m, k));
                }
                Src::OtherRider => {
                    if !(other_shared && other_rider[r]) {
                        continue;
                    }
                    // (the vehicle's side: body 0 of the solve, its rows (one part at the instance), the hull's margin)
                    let (m, k) = orows[r];
                    let mf = &self.other.manifolds[m];
                    let link = part_link(&self.child_link, n_links, mf.part as i32);
                    let mut t = mf.slots[k].touch;
                    let on_b = !self.other.rider_is_a;
                    let vm = self.other_now[mf.veh as usize];
                    let side = if on_b { &mut t.a } else { &mut t.b };
                    side.body = Some(0);
                    side.rows = [vm[0], vm[1], vm[2]];
                    t.jointed = Some(JointedSide { body: 0, link, on_b, point_w: POINT_W, self_b: None });
                    isl.rows.push(IslRow::Rider(isl.touches.len()));
                    isl.touches.push(t);
                    isl.used.push((Src::OtherRider, m, k));
                }
            }
            if isl.touches.len() > before {
                named.push(((p, r), IslRow::Rider(before)));
            }
        }
        // (the merged build: both lists as the callbacks walk them - the bike's and the loose world's rows all, his enabled
        // ones - for the vehicle's runs over the island, loose.rs `solve_island`)
        if loose.is_some() {
            let row_of = |&(p, r): &(usize, usize)| match srcs[p] {
                Src::BikeCity => Some(IslRow::Bike(r)),
                Src::Loose(i) => Some(IslRow::Loose(i, r)),
                _ => named.iter().find(|x| x.0 == (p, r)).map(|x| x.1),
            };
            isl.list_a = by_a.iter().filter_map(row_of).collect();
            isl.list_b = by_b.iter().filter_map(row_of).collect();
        }
        isl
    }

    /// The rider in no island: no solve, no push and so no post-push; slot 10 still runs, as on every active
    /// collider after the islands (ragdoll_solver_decode.md 1.1), then slot 14.
    pub fn lone_end(&mut self, dt: f32) {
        self.body.velocity_update(dt);
        self.end_tick();
    }

    /// After the island's velocity solve and apply: the post-solve runs (F-15350, the same two: the body fall at
    /// rd+64, F-15373), slot 15 (F-32404) and slot 10 (F-69485). Answers the body fall and the motion measure.
    pub fn after_solve(&mut self, isl: &RiderIsland, dt: f32) -> (Option<Vec3>, f32) {
        let mut fall = None;
        for others in &isl.runs {
            if let Some(at) = self.gate.body_fall_run(self.own_bike, others, rows3(&self.rd16)[3]) {
                fall = Some(at);
            }
        }
        let motion = self.body.motion_measure(dt);
        self.body.velocity_update(dt);
        (fall, motion)
    }

    /// After the push solve: the post-push (F-48210), rd+16, the rows' multipliers back to their contacts, slot 14.
    pub fn finish(&mut self, isl: &RiderIsland, dt: f32) {
        self.body.post_push(dt);
        self.rd16 = self.instance();
        for (&(src, m, k), t) in isl.used.iter().zip(isl.touches.iter()) {
            let s = match src {
                Src::City => &mut self.city.manifolds[m].slots[k].touch,
                Src::BikeRider => &mut self.veh.manifolds[m].slots[k].touch,
                Src::OtherRider => &mut self.other.manifolds[m].slots[k].touch,
                _ => &mut self.own.manifolds[m].slots[k].touch,
            };
            s.kept = t.kept;
            s.impulse = t.impulse;
            s.pushed = t.pushed;
        }
        self.end_tick();
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
    /// testdata/ragdoll_run_ro0.bin (work/ragdoll/run/export_run.py): per tick 169 - 186, RGD0 (slot 5's entry) and
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
    /// export_island_contacts.py over 169 - 186): the bike's own contacts (side 2: count, elasticity 0.1 each) and
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

    /// The rider's island as the original listed it (testdata/rider_island_order.txt, work/ragdoll/contacts2/
    /// export_order.py: PAIR / ISLE records of ro.0 and rm.0 169 - 216, rh.0 169 - 212, 140 ticks): per tick the
    /// pairs in the cache's order and the lists sorted by instance A and by instance B.
    fn order_cases() -> Vec<(String, i64, Vec<WalkPair>, Vec<(usize, usize)>, Vec<(usize, usize)>)> {
        let text = include_str!("testdata/rider_island_order.txt");
        let mut out: Vec<(String, i64, Vec<WalkPair>, Vec<(usize, usize)>, Vec<(usize, usize)>)> = Vec::new();
        let rows = |w: &[&str]| -> Vec<(usize, usize)> {
            w.iter()
                .map(|x| {
                    let (p, r) = x.split_once('.').unwrap();
                    (p.parse().unwrap(), r.parse().unwrap())
                })
                .collect()
        };
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.first() {
                Some(&"T") => out.push((format!("{} {}", w[1], w[2]), w[3].parse().unwrap(), Vec::new(), Vec::new(), Vec::new())),
                Some(&"P") => {
                    let n: Vec<i64> = w[1..].iter().map(|x| x.parse().unwrap()).collect();
                    let side = |l: i64| (l >= 0).then_some(l);
                    out.last_mut().unwrap().2.push(WalkPair { a: side(n[0]), b: side(n[1]), key_a: n[2], key_b: n[3], rows: n[4] as usize });
                }
                Some(&"A") => out.last_mut().unwrap().3 = rows(&w[1..]),
                Some(&"B") => out.last_mut().unwrap().4 = rows(&w[1..]),
                _ => {}
            }
        }
        out
    }

    /// The row order rule ([`island_rows`]) rebuilds both lists of every recorded tick, the unstable sort's moves
    /// above 16 rows included (ro.0 194: the self contacts ahead of the city's). The recorded cache order is the
    /// one tick_with makes: the city's pairs by the object's level, then the others by (lower, higher) level; the
    /// instance A of a pair with the city is the object, of two objects the lower level's.
    #[test]
    fn island_row_order_matches_recordings() {
        let cases = order_cases();
        assert_eq!(cases.len(), 140);
        let mut over16 = 0;
        for (name, rider, pairs, want_a, want_b) in &cases {
            let key = |p: &WalkPair| match p.b {
                None => (0, p.a.unwrap()),
                Some(b) => (1 + p.a.unwrap().min(b), p.a.unwrap().max(b)),
            };
            assert!(pairs.windows(2).all(|w| key(&w[0]) < key(&w[1])), "{name}: cache order");
            assert!(pairs.iter().all(|p| p.b.is_none() || p.a <= p.b), "{name}: side A");
            let (a, b) = island_rows(pairs, *rider).expect("the rider's island");
            assert_eq!(&a, want_a, "{name}: list by instance A");
            assert_eq!(&b, want_b, "{name}: list by instance B");
            over16 += (a.len() > 16) as usize;
        }
        eprintln!("{} ticks, {over16} of them above 16 rows", cases.len());
        assert!(over16 > 40);
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

    /// Ticks 169 - 185 of ro.0, chained: the recorded state at slot 5's entry of 169, the city from the extracted
    /// data, the island's other share (the bike's contacts) from the recording; after each tick every word of the
    /// links, joints, body and collider against the recorded state at the next tick's slot 5 entry (and the
    /// post-push's exit of the same tick, which differs from it only in what slot 14 writes). The self contacts are
    /// the port's own (from 175; ro.0's self pair list); the bike's rows from the order fixture. Run on 169 - 216
    /// (export_run.py and island/export_island_contacts.py to 216, 3 MB, not kept) the chain is exact on every tick,
    /// 169 - 215 against RGD0 170 - 216 (the end of the ragdoll).
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
        // (the bike's rows in the island: its pairs with the city and with the rider, the latter disabled by the gate)
        let mut bikes: HashMap<u32, BikeRows> = HashMap::new();
        for (name, rider, pairs, _, _) in order_cases() {
            let Some(t) = name.strip_prefix("ro.0 ") else { continue };
            let mut b = BikeRows { level: 0x56d7, rider_level: rider, city: None, rider: None, ..Default::default() };
            for p in &pairs {
                match (p.a, p.b) {
                    (Some(0x56d7), None) => b.city = Some(p.rows),
                    (Some(0x56d7), Some(r)) if r == rider => b.rider = Some(vec![false; p.rows]),
                    _ => {}
                }
            }
            bikes.insert(t.parse().unwrap(), b);
        }
        let body = load(&cases[0].inp);
        // (the self pair list of ro.0's collider, +724 / +732)
        let pair_a = vec![1, 1, 8, 8, 8, 8, 12, 12, 12, 12, 15, 16, -1];
        let pair_b = vec![7, 11, 1, 3, 5, 14, 1, 3, 5, 14, 18, 19, -1];
        let mut rd = RiderRagdoll::assemble(&ty, body, map, pair_a, pair_b, parts_169(), &[], &Surface::table(&rs));
        rd.own_bike = 0x56d7;
        let mut exact = Vec::new();
        let mut first_bad: Option<(u32, Vec<String>)> = None;
        for i in 0..cases.len() - 1 {
            let c = &cases[i];
            let (mut o, sides) = extra.get(&c.tick).cloned().unwrap_or_default();
            o.bike = bikes.get(&c.tick).cloned();
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
        assert_eq!(exact, (169..=185).collect::<Vec<u32>>(), "first difference: {first_bad:?}");
    }

    /// The merged island's lists (the rider in a loose island: [`cache_order`], [`merged_rows`]) against a recording's
    /// (testdata/rider_semi_yb0_order.txt, export_isl.py): per tick 'T name tick rider-level', its pairs in the cache's
    /// order as 'N a b first rows keyA,keyB ...' (-1: the city or an object that is not active) and the recorded lists
    /// as node.row. Answers (ticks, ticks whose PAIR records' slot order is the port's rule, differences).
    fn merged_order_check(text: &str) -> (usize, usize, Vec<String>) {
        let mut ticks: Vec<(String, i64, Vec<WalkNode>, Vec<(usize, usize)>, Vec<(usize, usize)>)> = Vec::new();
        let rows = |w: &[&str]| -> Vec<(usize, usize)> {
            w.iter()
                .map(|x| {
                    let (p, r) = x.split_once('.').unwrap();
                    (p.parse().unwrap(), r.parse().unwrap())
                })
                .collect()
        };
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.first() {
                Some(&"T") => ticks.push((format!("{} {}", w[1], w[2]), w[3].parse().unwrap(), Vec::new(), Vec::new(), Vec::new())),
                Some(&"N") => {
                    let side = |s: &str| Some(s.parse::<i64>().unwrap()).filter(|&l| l >= 0);
                    let keys = w[5..]
                        .iter()
                        .map(|k| {
                            let (a, b) = k.split_once(',').unwrap();
                            (a.parse().unwrap(), b.parse().unwrap())
                        })
                        .collect();
                    ticks.last_mut().unwrap().2.push(WalkNode { a: side(w[1]), b: side(w[2]), keys, constraint: w[3] == "1" });
                }
                Some(&"A") => ticks.last_mut().unwrap().3 = rows(&w[1..]),
                Some(&"B") => ticks.last_mut().unwrap().4 = rows(&w[1..]),
                _ => {}
            }
        }
        let mut bad = Vec::new();
        let mut same_cache = 0;
        for (name, rider, nodes, want_a, want_b) in &ticks {
            // (the file's nodes: constraints first, then the PAIR records by cache slot)
            let order = cache_order(nodes);
            let pairs_kept = order.iter().filter(|&&k| !nodes[k].constraint).copied().collect::<Vec<_>>() == (0..nodes.len()).filter(|&k| !nodes[k].constraint).collect::<Vec<_>>();
            same_cache += pairs_kept as usize;
            let sorted: Vec<WalkNode> = order.iter().map(|&k| nodes[k].clone()).collect();
            let back = |l: Vec<(usize, usize)>| l.into_iter().map(|(i, r)| (order[i], r)).collect::<Vec<_>>();
            match merged_rows(&sorted, *rider).map(|(a, b)| (back(a), back(b))) {
                Some((a, b)) => {
                    if &a != want_a {
                        bad.push(format!("{name}: list by instance A {a:?}, recorded {want_a:?}"));
                    }
                    if &b != want_b {
                        bad.push(format!("{name}: list by instance B {b:?}, recorded {want_b:?}"));
                    }
                }
                None => bad.push(format!("{name}: no island holds the rider")),
            }
        }
        (ticks.len(), same_cache, bad)
    }

    /// The merged island's lists against yc.0 813 - 817 (the rider, his bike, a semi trailer and the body touching it):
    /// both lists of every tick (the hitch's constraint walked last, [`cache_order`]), and the PAIR records' slots in the
    /// port's (lower, higher) level order.
    #[test]
    fn merged_island_order_matches_recording() {
        let (ticks, same, bad) = merged_order_check(include_str!("testdata/rider_semi_yc0_order.txt"));
        eprintln!("{ticks} ticks, {same} in the port's cache order");
        assert_eq!(ticks, 5);
        assert_eq!(same, ticks);
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    /// The merged island's lists against rc.0 1328 - 1351 (the rider thrown head on into an ambient car, his bike in the
    /// same island on 10 of the 13 ticks; testdata/rider_bike_car_rc0_order.txt).
    #[test]
    fn merged_island_order_matches_rider_bike_car_recording() {
        let (ticks, same, bad) = merged_order_check(include_str!("testdata/rider_bike_car_rc0_order.txt"));
        eprintln!("{ticks} ticks, {same} in the port's cache order");
        assert_eq!(ticks, 13);
        assert_eq!(same, ticks);
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }
}

#[cfg(test)]
#[path = "artic_run_offline.rs"]
mod offline;
