//! The build of the thrown bike rider's ragdoll: the phArticulatedBody the original makes at run time from the
//! rider's fragment type (F-36173 calls F-62449 with no source entry, then F-83435). Decode:
//! docs/reference/vehicle/ragdoll_link_joint_build.md (layouts: ragdoll_solver_decode.md 2.2 - 2.5, 3.10; the
//! swing-twist joint's field map: ragdoll_swing_twist.md 2).
//!
//! The result is the guest's own memory after the build, block by block, big-endian: the body (528 B), the links
//! (1008 B), the joints (784 B hinge, 1408 B swing-twist), the collider's fields the build sets (752 B) and the
//! collider's three arrays (+716 child -> link, +724 / +732 the part pairs). Pointers are the addresses the caller
//! gives ([`GuestAddrs`]), so the blocks compare with a recording and feed the tick (physics/artic.rs reads the same
//! layouts). Bytes no code of the build writes hold [`FILL`], the heap fill the recording shows.
//!
//! Rules, in the original's order and float arithmetic (every add / multiply rounded to f32, a vector multiply-add
//! is a rounded product then an add, a scalar fmadds one rounding):
//! - F-62449: children 1 .. n-1; a child with no bone goes into the root sums; else its ancestor bone is found by
//!   walking up the groups (the first group whose first child's bone is not the child's own and has a link; group 0
//!   or no parent: the root); DOFs by F-41462; a child with a DOF and no link yet gets a link (F-91939) and a joint
//!   (F-49221), the first time after the root link (F-16970); a child without one is merged into the link of its
//!   ancestor (mass added, moments + I + m (y2+z2, x2+z2, x2+y2) of its centre in model space), or into the root
//!   sums when the walk ended at group 0; the collider's +716 takes the child's link (F-31210 path, 0 when none).
//! - F-16970: root link: mass splat, moments the sums, rest frame the instance rows transposed with position 0;
//!   identity rows at +272 / +320 / +368; the body (gravity factor from the archetype), the collider and its
//!   arrays (+716 zeroed).
//! - F-91939: a link's mass and moments (scaled when its smallest moment is below type+8), rest frame F-55164:
//!   rows = child rotation x M, position = (centre - root child's centre - off) x M.
//! - F-49221: one rotation DOF: hinge (type 0) with hard limits lo + 0.03 / hi - 0.03 and inner limits from the
//!   group's fractions (F-13018, F-97677); two or three: swing-twist (type 1), half ranges, F-88270 / F-71882;
//!   k = min(max(G+20, min softness), 1); drives from G+40 .. G+52; AddLink (F-55902).
//! - F-83435: the part pairs, each child of group a x each child of group b, F-16298 (no duplicates either way
//!   round, kept sorted by (a, b)).
//!
//! Inputs the file does not hold: the instance matrix M (the rider's pose at GoRagdoll, rows of [inst+16] with
//! their w lanes; its position row is not used: the build passes the rows with a zero position), the archetype's
//! gravity factor, the run-time least-softness global (0.0 in the running game) and the addresses.
//!
//! Not produced (left at [`FILL`], or as noted), and not compared by the test:
//! - joint frames +432..+479 / +544..+591, anchors +496 / +512 and whatever else F-38962, F-16319 (the hinge's
//!   frame) and F-33259's frame / quaternion code store: not decoded (they also set +8 / +12, which are built);
//! - lane w of the swing-twist joint's +672 / +736 (stale stack in F-49221) and of its constructor's +672 / +688 /
//!   +736 / +752 (stale stack in F-99002);
//! - lane w of a non-root link's rest position +572 (and so of its current +476): M's w lanes are a run-time input,
//!   computed here from what the caller passes;
//! - lane w of the merged moments +508 (the parallel-axis term's lane w is stale stack; taken as 0 here);
//! - the collider's base-class fields (F-33655), its mass / inertia (F-87421) and everything the
//!   articulated inertias pass (F-97481, run at the end of F-62449) writes: the spatial / articulated inertia
//!   blocks of the links (+128..+415 except the root's identity rows, +736..+879) and of the joints (+16..+303)
//!   are left as the constructors leave them;
//! - slider joints (translation DOF) and parked joints (G+56 = -1 or > 0): not used by the riders, not built
//!   ([`BuildError`]).

use mcla_formats::rage::ragdoll::RagdollType;

pub const LINK_SIZE: usize = 1008;
pub const HINGE_SIZE: usize = 784;
pub const BALL_SIZE: usize = 1408;
pub const BODY_SIZE: usize = 528;
pub const COLLIDER_SIZE: usize = 752;
/// What a freshly allocated block holds where no constructor writes (the recording's heap fill).
pub const FILL: u8 = 0xCD;
/// The collider's vtable, stored at +0 by its constructor (F-52262).
pub const COLLIDER_VTABLE: u32 = 0x8208_2114;

const PI: u32 = 0x4049_0fdb;
const TWO_PI: u32 = 0x40c9_0fdb;
const C003: u32 = 0x3cf5_c28f; // 0.03
const C049: u32 = 0x3efa_e148; // 0.49
const C2827: u32 = 0x4034_f4ab; // 2.82743
const C001: u32 = 0x3c23_d70a; // 0.01
const FLT_MAX: u32 = 0x7f7f_ffff;
const C200: u32 = 0x4348_0000;
const C10: u32 = 0x4120_0000;
const C1E8: u32 = 0x4cbe_bc20;
const NEG_1E8: u32 = 0xccbe_bc20;
const C01: u32 = 0x3dcc_cccd; // 0.1

#[inline]
fn c(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// PPC fsel: a >= 0 (-0 too) -> b, else (and NaN) c.
#[inline]
fn fsel(a: f32, b: f32, cc: f32) -> f32 {
    if a >= 0.0 {
        b
    } else {
        cc
    }
}

/// fmadds as the recompilation runs it: the product exact in double, one rounding of the sum to double, then to f32.
#[inline]
fn fmadds(a: f32, b: f32, cc: f32) -> f32 {
    (a as f64 * b as f64 + cc as f64) as f32
}

/// max(0, x) the way F-88270 / F-71882 take it: fsel(-x, 0, x).
#[inline]
fn max0(x: f32) -> f32 {
    fsel(-x, 0.0, x)
}

type V4 = [f32; 4];

#[inline]
fn vadd(a: V4, b: V4) -> V4 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}
#[inline]
fn vsub(a: V4, b: V4) -> V4 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}
#[inline]
fn vmul(a: V4, b: V4) -> V4 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}
/// vmaddfp: a * b rounded, then + cc.
#[inline]
fn vmadd(a: V4, b: V4, cc: V4) -> V4 {
    vadd(vmul(a, b), cc)
}
#[inline]
fn splat(x: f32) -> V4 {
    [x; 4]
}

/// Where the guest's objects are (pointers written into the blocks).
#[derive(Debug, Clone, Default)]
pub struct GuestAddrs {
    pub body: u32,
    pub collider: u32,
    /// link i (0 = root), in build order
    pub links: Vec<u32>,
    /// joint j (joins link j+1 to its parent)
    pub joints: Vec<u32>,
    /// the collider's arrays: +684, +692, +700, +708 (93 entries each), +716 (child -> link), +724, +732 (pairs)
    pub collider_arrays: [u32; 7],
    /// collider +16: the fragment instance (entry+232)
    pub inst: u32,
    /// collider +24: the 44-byte object made after the loop
    pub solver_obj: u32,
    /// collider +748: the type's +248 as loaded
    pub type248: u32,
}

/// Run-time inputs of the build.
#[derive(Debug, Clone, Copy)]
pub struct BuildInput {
    /// rows of the instance matrix ([inst+16] +0 / +16 / +32) with their w lanes
    pub m: [V4; 3],
    /// the archetype's gravity factor (body +516)
    pub gravity: f32,
    /// the run-time least-softness global: 0.0 in the running game
    pub min_softness: f32,
}

impl BuildInput {
    pub fn new(m: [V4; 3]) -> Self {
        BuildInput { m, gravity: 1.0, min_softness: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BuildError {
    /// a child's bone is outside the skeleton / the 2048-entry map
    BadBone(usize),
    /// a translation DOF: slider joint, not decoded
    Slider(usize),
    /// G+56 = -1 or > 0: the joint is parked, not decoded further
    Parked(usize),
    /// more links / joints than the caller gave addresses for (or than the body holds)
    TooMany,
    /// no child made a link: no body
    NoBody,
}

/// The guest blocks after the build.
#[derive(Debug, Clone)]
pub struct ArticBuild {
    pub body: Vec<u8>,
    pub links: Vec<Vec<u8>>,
    /// (type, block)
    pub joints: Vec<(u32, Vec<u8>)>,
    pub collider: Vec<u8>,
    /// collider +716: child -> link
    pub child_link: Vec<i32>,
    /// collider +724 / +732 (256 entries each, -1 = free)
    pub pair_a: Vec<i32>,
    pub pair_b: Vec<i32>,
    /// the child each link was made from (entry+368 for the root: the first child of the group the walk ended at)
    pub link_child: Vec<usize>,
    /// parent link of link i (link 0: -1)
    pub parents: Vec<i32>,
}

struct Link {
    child: usize,
    root: bool,
    mass: f32,
    moments: V4,
    rest: [V4; 4],
}

struct Joint {
    ty: u32,
    parent: usize,
    k: f32,
    mode: u32,
    lim: [f32; 6],
    /// swing-twist bytes +632 / +633
    b632: u8,
    b633: u8,
    /// hinge +672..+692
    hd: [f32; 6],
    /// swing-twist vectors +672 / +688 / +704 / +720 / +736 / +752: lanes x y z (w see `vw`)
    bv: [[f32; 3]; 6],
    /// lane w of those vectors where the build knows it (None: stale stack)
    vw: [Option<f32>; 6],
}

fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_be_bytes());
}
fn put_f(b: &mut [u8], o: usize, v: f32) {
    put_u32(b, o, v.to_bits());
}
fn put_v(b: &mut [u8], o: usize, v: V4) {
    for (k, x) in v.iter().enumerate() {
        put_f(b, o + 4 * k, *x);
    }
}
fn put_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_be_bytes());
}

/// The parallel-axis term of F-62449: I_child + m (y2+z2, x2+z2, x2+y2) of the child's centre (vmulfp, fadds,
/// vmaddfp). Lane w of the distance vector is stale stack in the original; 0 here.
fn parallel(i_child: V4, m: f32, pos: V4) -> V4 {
    let sq = vmul(pos, pos);
    let d = [sq[2] + sq[1], sq[2] + sq[0], sq[1] + sq[0], 0.0];
    vmadd(d, splat(m), i_child)
}

/// F-33001 (rest -> current): +416.. = rest, +608 = +576, +624 = +640 = +592, +656 = v + c x w.
fn rest_to_current(b: &mut [u8], rest: &[V4; 4]) {
    for (r, row) in rest.iter().enumerate() {
        put_v(b, 416 + 16 * r, *row);
    }
    let v = [0.0f32; 4]; // +576, zeroed by the link constructor
    let w = [0.0f32; 4]; // +592
    put_v(b, 608, v);
    put_v(b, 624, w);
    put_v(b, 640, w);
    let cc = rest[3];
    let yzx = |a: V4| [a[1], a[2], a[0], a[3]];
    let zxy = |a: V4| [a[2], a[0], a[1], a[3]];
    let t = vmadd(yzx(cc), zxy(w), v);
    put_v(b, 656, vsub(t, vmul(zxy(cc), yzx(w))));
}

/// The link constructor (F-60898): zeroes the vectors at +16..+111, +576..+735, +880..+1007 and the words
/// +0 / +112.
fn link_block(l: &Link) -> Vec<u8> {
    let mut b = vec![FILL; LINK_SIZE];
    for o in [16, 32, 48, 64, 80, 96, 576, 592, 608, 624, 640, 656, 672, 688, 704, 720, 880, 896, 944, 960, 976, 992] {
        put_v(&mut b, o, [0.0; 4]);
    }
    put_u32(&mut b, 0, 0);
    put_u32(&mut b, 112, 0);
    put_v(&mut b, 480, splat(l.mass));
    put_v(&mut b, 496, l.moments);
    for (r, row) in l.rest.iter().enumerate() {
        put_v(&mut b, 512 + 16 * r, *row);
    }
    rest_to_current(&mut b, &l.rest);
    if l.root {
        // vupkd3d (3, 3, 0, 1) permuted: identity rows of the three articulated-inertia blocks
        let id = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
        for base in [272, 320, 368] {
            for (r, row) in id.iter().enumerate() {
                put_v(&mut b, base + 16 * r, *row);
            }
        }
    }
    b
}

/// The base joint constructor (F-15442): +4 = 0, +8 / +12 = 0, +528 = 0, byte +532 = 0, +16..+431 zero.
fn joint_base(size: usize) -> Vec<u8> {
    let mut b = vec![FILL; size];
    put_f(&mut b, 4, 0.0);
    put_u32(&mut b, 8, 0);
    put_u32(&mut b, 12, 0);
    put_u32(&mut b, 528, 0);
    b[532] = 0;
    b[16..432].fill(0);
    b
}

fn joint_block(j: &Joint, a: &GuestAddrs, child: u32) -> Vec<u8> {
    let mut b;
    if j.ty == 0 {
        // F-77258 constructor, then the build
        b = joint_base(HINGE_SIZE);
        put_u32(&mut b, 0, 0);
        put_f(&mut b, 624, 0.0);
        put_f(&mut b, 628, 0.0);
        for (k, o) in [672, 676, 680, 684, 688, 692].iter().enumerate() {
            put_f(&mut b, *o, j.hd[k]);
        }
        for (k, o) in [608, 612, 616, 620].iter().enumerate() {
            put_f(&mut b, *o, j.lim[k]);
        }
    } else {
        // F-99002 constructor, then the build
        b = joint_base(BALL_SIZE);
        put_u32(&mut b, 0, 1);
        for o in [636, 640, 644, 648] {
            put_u32(&mut b, o, FLT_MAX);
        }
        for o in [652, 656, 660, 664, 816] {
            put_f(&mut b, o, 0.0);
        }
        b[632..636].fill(0);
        for (k, o) in [608, 612, 616, 620, 624, 628].iter().enumerate() {
            put_f(&mut b, *o, j.lim[k]);
        }
        b[632] = j.b632;
        b[633] = j.b633;
        for (k, base) in [672, 688, 704, 720, 736, 752].iter().enumerate() {
            for ln in 0..3 {
                put_f(&mut b, base + 4 * ln, j.bv[k][ln]);
            }
            if let Some(w) = j.vw[k] {
                put_f(&mut b, base + 12, w);
            }
        }
    }
    put_f(&mut b, 4, j.k);
    put_u32(&mut b, 8, a.links[j.parent]);
    put_u32(&mut b, 12, child);
    put_u32(&mut b, 528, j.mode);
    b
}

/// F-49221's numbers for one joint.
fn make_joint(t: &RagdollType, child: usize, bone: usize, dof: [bool; 6], min_softness: f32) -> Result<Joint, BuildError> {
    let g = &t.groups[t.children[child].group as usize];
    let bn = &t.bones[bone];
    if dof[3] || dof[4] || dof[5] {
        return Err(BuildError::Slider(child));
    }
    let (rx, ry, rz) = (dof[0], dof[1], dof[2]);
    let nrot = rx as u32 + ry as u32 + rz as u32;
    let mut j = Joint { ty: 0, parent: 0, k: 0.0, mode: 0, lim: [0.0; 6], b632: 0, b633: 0, hd: [0.0; 6], bv: [[0.0; 3]; 6], vw: [None; 6] };
    let (zero, one, half) = (0.0f32, 1.0f32, 0.5f32);
    if nrot == 1 {
        // hinge (F-77258), F-13018, F-97677
        j.ty = 0;
        let ax = if rx { 0 } else if ry { 1 } else { 2 };
        let (mut lo, mut hi) = (bn.rot_min[ax], bn.rot_max[ax]);
        if lo > c(PI) || !(hi <= c(PI)) {
            lo = lo - c(TWO_PI);
            hi = hi - c(TWO_PI);
        }
        let (mut a, mut cc) = (c(C003) + lo, hi - c(C003));
        if !(a <= cc) {
            a = (lo + hi) * half;
            cc = a;
        }
        j.lim[0] = a;
        j.lim[1] = cc;
        let fr = |v: f32| {
            let v = fsel(-1.0 - v, -1.0, v);
            fsel(one - v, v, one)
        };
        let (mut f0, mut f1) = (fr(g.frac[0]), fr(g.frac[1]));
        if !(f0 <= f1) {
            f0 = (f1 + f0) * half;
            f1 = f0;
        }
        let h = (hi - lo) * c(C049);
        let mid = (hi + lo) * half;
        let (lo2, hi2) = (fmadds(h, f0, mid), fmadds(h, f1, mid));
        let (mut a, mut cc) = (c(C003) + lo2, hi2 - c(C003));
        if !(a <= cc) {
            a = (lo2 + hi2) * half;
            cc = a;
        }
        j.lim[2] = a;
        j.lim[3] = cc;
        // drive (constructor: 200, 10, 0, 0, 1e8, -1e8)
        j.hd = [c(C200), c(C10), 0.0, 0.0, c(C1E8), c(NEG_1E8)];
        let mut mode1 = false;
        if g.vel_gain != zero {
            j.hd[3] = g.target_rate;
            j.hd[1] = g.vel_gain;
            j.mode = 1;
            mode1 = true;
        }
        if g.pos_gain != zero && g.max_torque != zero {
            j.hd[0] = g.pos_gain;
            j.mode = if mode1 { 3 } else { 2 };
            j.hd[4] = g.max_torque;
        }
    } else {
        // swing-twist (F-99002), F-33259 -> F-88270, F-71882
        j.ty = 1;
        let mut tw = if nrot == 3 {
            let v = (bn.rot_max[2] - bn.rot_min[2]) * half;
            if v < c(PI) {
                v
            } else {
                c(FLT_MAX)
            }
        } else {
            zero
        };
        let (a_ax, b_ax) = if nrot == 3 || !rz {
            (0, 1)
        } else if rx {
            (2, 0)
        } else {
            (2, 1)
        };
        let mut sa = (bn.rot_max[a_ax] - bn.rot_min[a_ax]) * half;
        let mut sb = (bn.rot_max[b_ax] - bn.rot_min[b_ax]) * half;
        if tw < c(C001) || sa >= c(PI) || !(sb < c(PI)) {
            tw = c(C001);
            if !(sa < c(PI)) {
                sa = c(C2827);
            }
            if !(sb < c(PI)) {
                sb = c(C2827);
            }
        }
        // F-88270 (its +632 / +633 are overwritten by F-71882)
        j.lim[0] = max0(sa - c(C003));
        j.lim[2] = max0(sb - c(C003));
        j.lim[4] = max0(tw - c(C003));
        // F-71882
        let mn1 = |v: f32| fsel(one - v, v, one);
        j.lim[1] = max0(mn1(g.frac[1]) * sa - c(C003));
        j.lim[3] = max0(mn1(g.frac[2]) * sb - c(C003));
        j.lim[5] = max0(mn1(g.frac[3]) * tw - c(C003));
        j.b632 = 1;
        j.b633 = (j.lim[5] != c(FLT_MAX)) as u8;
        // constructor vectors: (200 x3), (10 x3), 0, 0, (1e8 x3), (-1e8 x3)
        j.bv = [[c(C200); 3], [c(C10); 3], [0.0; 3], [0.0; 3], [c(C1E8); 3], [c(NEG_1E8); 3]];
        j.vw = [None, None, Some(0.0), Some(0.0), None, None];
        let mut mode1 = false;
        if g.target_rate != zero && g.vel_gain != zero {
            j.bv[3][2] = g.target_rate; // +728
            j.bv[1] = [g.vel_gain; 3];
            j.vw[1] = Some(g.vel_gain);
            j.mode = 1;
            mode1 = true;
        }
        if g.pos_gain != zero && g.max_torque != zero {
            j.mode = if mode1 { 3 } else { 2 };
            j.bv[0] = [g.pos_gain; 3];
            j.vw[0] = None;
            j.bv[4] = [g.max_torque; 3];
            j.vw[4] = None;
        }
    }
    let k = fsel(g.k - min_softness, g.k, min_softness);
    j.k = fsel(k - one, one, k);
    if g.park == -1.0 || g.park > zero {
        return Err(BuildError::Parked(child));
    }
    Ok(j)
}

/// F-16298 with add and sort: append (x, y) unless it (or (y, x)) is there, then sort by (a, b) (F-19490).
fn add_pair(a: &mut [i32], b: &mut [i32], dirty: &mut u8, x: i32, y: i32) {
    let mut k = 0;
    while k < a.len() && a[k] != -1 {
        if (a[k] == x && b[k] == y) || (a[k] == y && b[k] == x) {
            return;
        }
        k += 1;
    }
    if k >= a.len() {
        return;
    }
    a[k] = x;
    b[k] = y;
    *dirty = 1;
    let mut v: Vec<(i32, i32)> = a.iter().zip(b.iter()).take_while(|(p, q)| **p != -1 && **q != -1).map(|(p, q)| (*p, *q)).collect();
    v.sort();
    for (i, (p, q)) in v.into_iter().enumerate() {
        a[i] = p;
        b[i] = q;
    }
}

/// Build the rider's articulated body from its fragment type (F-62449 with no source entry, then F-83435).
pub fn build(t: &RagdollType, input: &BuildInput, addrs: &GuestAddrs) -> Result<ArticBuild, BuildError> {
    let n = t.children.len();
    let m = input.m;
    let off = vsub(t.cg, t.cg_ref);
    let mut map = vec![-1i32; 2048];
    map[0] = 0;
    let get = |map: &Vec<i32>, b: i32| -> i32 { if (0..2048).contains(&b) { map[b as usize] } else { -1 } };
    let mut root_mass = t.children[0].mass;
    let mut root_i = t.inertia[0];
    let mut links: Vec<Link> = Vec::new();
    let mut joints: Vec<Joint> = Vec::new();
    let mut parents: Vec<i32> = Vec::new();
    let mut child_link = vec![0i32; n];
    let mut root_child = 0usize;
    for i in 1..n {
        let ch = &t.children[i];
        let b = ch.bone as i32;
        if b <= 0 {
            root_mass = ch.mass + root_mass;
            root_i = vadd(root_i, parallel(t.inertia[i], ch.mass, ch.pos));
            continue;
        }
        if b as usize >= t.bones.len() || b >= 2048 {
            return Err(BuildError::BadBone(i));
        }
        // the ancestor walk
        let mut r31 = ch.group as usize;
        let mut r27 = 0i32;
        let mut r11 = t.groups[r31].parent as usize;
        if r11 != 255 {
            loop {
                r31 = r11;
                let fb = t.children[t.groups[r11].first_child as usize].bone as i32;
                if fb != b && get(&map, fb) != -1 {
                    r27 = fb;
                    break;
                }
                r11 = t.groups[r31].parent as usize;
                if r31 == 0 || r31 == 255 || r11 == 255 {
                    r27 = 0;
                    break;
                }
            }
        }
        let dof = t.bones[b as usize].dofs();
        let any = dof.iter().any(|&d| d);
        if !any && get(&map, r27) <= 0 {
            if r31 == 0 {
                root_mass = ch.mass + root_mass;
                root_i = vadd(root_i, parallel(t.inertia[i], ch.mass, ch.pos));
            }
            if !links.is_empty() {
                child_link[i] = get(&map, b).max(0);
            }
            continue;
        }
        if links.is_empty() {
            // F-16970: the root link from the sums, rest frame = M transposed (position row 0)
            root_child = t.groups[r31].first_child as usize;
            let rest = [[m[0][0], m[1][0], m[2][0], 0.0], [m[0][1], m[1][1], m[2][1], 0.0], [m[0][2], m[1][2], m[2][2], 0.0], [0.0; 4]];
            links.push(Link { child: root_child, root: true, mass: root_mass, moments: root_i, rest });
            parents.push(-1);
        }
        if any && map[b as usize] == -1 {
            // F-91939: mass and moments, scaled when the smallest moment is below type+8
            let mut mo = t.inertia[i];
            let mut mass = ch.mass;
            let (x, y, z) = (mo[0], mo[1], mo[2]);
            let f10 = x - y;
            let mn = fsel(f10, y, x);
            let mn = fsel(mn - z, z, mn);
            if !(mn >= t.min_moment) {
                let mx = fsel(f10, x, y);
                let mut s = t.min_moment / mn;
                let mx = fsel(mx - z, mx, z);
                if mx * s > t.max_moment {
                    s = (mx / t.max_moment) * s;
                }
                if s > 1.0 {
                    mass = s * mass;
                    mo = vmul(mo, splat(s));
                }
            }
            // F-55164 (no source instance): rows = child rotation x M, position = (centre - root centre - off) x M
            let p = vsub(vsub(ch.pos, t.children[root_child].pos), off);
            let pos = vmadd(m[2], splat(p[2]), vmadd(m[1], splat(p[1]), vmadd(m[0], splat(p[0]), [0.0; 4])));
            let row = |r: V4| vmadd(m[2], splat(r[2]), vmadd(m[1], splat(r[1]), vmul(m[0], splat(r[0]))));
            let (n0, n1, n2) = (row(ch.rot[0]), row(ch.rot[1]), row(ch.rot[2]));
            let rest = [[n0[0], n1[0], n2[0], pos[0]], [n0[1], n1[1], n2[1], pos[1]], [n0[2], n1[2], n2[2], pos[2]], pos];
            let mut jt = make_joint(t, i, b as usize, dof, input.min_softness)?;
            jt.parent = get(&map, r27) as usize;
            // AddLink (F-55902): the new link's index; parent index = the parent link's place in the body
            let li = links.len();
            links.push(Link { child: i, root: false, mass, moments: mo, rest });
            parents.push(jt.parent as i32);
            joints.push(jt);
            map[b as usize] = li as i32;
        } else {
            if map[b as usize] == -1 {
                map[b as usize] = get(&map, r27);
            }
            let l = &mut links[map[b as usize] as usize];
            l.mass = ch.mass + l.mass;
            l.moments = vadd(l.moments, parallel(t.inertia[i], ch.mass, ch.pos));
        }
        child_link[i] = map[b as usize].max(0);
    }
    if links.is_empty() {
        return Err(BuildError::NoBody);
    }
    if links.len() > 32 || joints.len() > 30 || addrs.links.len() < links.len() || addrs.joints.len() < joints.len() {
        return Err(BuildError::TooMany);
    }

    // the body (constructor F-39370, F-16970, AddLink F-55902)
    let mut body = vec![FILL; BODY_SIZE];
    put_u32(&mut body, 0, links.len() as u32);
    for i in 0..links.len() {
        put_u32(&mut body, 4 + 4 * i, addrs.links[i]);
    }
    for j in 0..joints.len() {
        put_u32(&mut body, 132 + 4 * j, addrs.joints[j]);
        put_u32(&mut body, 256 + 4 * j, parents[j + 1] as u32);
    }
    put_f(&mut body, 516, input.gravity);
    put_f(&mut body, 520, c(C01));
    put_f(&mut body, 524, c(C01));

    let link_blocks: Vec<Vec<u8>> = links.iter().map(link_block).collect();
    let joint_blocks: Vec<(u32, Vec<u8>)> = joints
        .iter()
        .enumerate()
        .map(|(j, jt)| {
            (jt.ty, joint_block(jt, addrs, addrs.links[j + 1]))
        })
        .collect();

    // the collider: its constructor's fields (F-52262), the setters, the arrays (+716 zeroed by F-59962)
    let mut col = vec![FILL; COLLIDER_SIZE];
    put_u32(&mut col, 0, COLLIDER_VTABLE);
    put_u32(&mut col, 16, addrs.inst);
    put_u32(&mut col, 20, 1);
    put_u32(&mut col, 24, addrs.solver_obj);
    put_u32(&mut col, 672, addrs.body);
    put_u32(&mut col, 680, 0);
    let caps: [u16; 7] = [93, 93, 93, 93, n as u16, 256, 256];
    for (k, o) in [684usize, 692, 700, 708, 716, 724, 732].iter().enumerate() {
        put_u32(&mut col, *o, addrs.collider_arrays[k]);
        put_u16(&mut col, o + 4, caps[k]);
        put_u16(&mut col, o + 6, caps[k]);
    }
    put_u32(&mut col, 744, 0);
    put_u32(&mut col, 748, addrs.type248);

    // F-83435: the part pairs
    let mut pair_a = vec![-1i32; 256];
    let mut pair_b = vec![-1i32; 256];
    let mut dirty = 0u8;
    let ng = t.groups.len();
    for &(ga, gb) in &t.pairs {
        let (ga, gb) = (ga as usize, gb as usize);
        if ga >= ng || gb >= ng || ga == gb || t.groups[ga].children == 0 {
            continue;
        }
        let (a, b) = (&t.groups[ga], &t.groups[gb]);
        for jj in 0..a.children as i32 {
            for ii in 0..b.children as i32 {
                add_pair(&mut pair_a, &mut pair_b, &mut dirty, a.first_child as i32 + jj, b.first_child as i32 + ii);
            }
        }
    }
    col[740] = dirty;

    let link_child = links.iter().map(|l| l.child).collect();
    Ok(ArticBuild { body, links: link_blocks, joints: joint_blocks, collider: col, child_link, pair_a, pair_b, link_child, parents })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcla_formats::rage::ragdoll::parse_character_ragdoll;

    struct State {
        col_addr: u32,
        col: Vec<u8>,
        body_addr: u32,
        body: Vec<u8>,
        links: Vec<(u32, Vec<u8>)>,
        joints: Vec<(u32, u32, Vec<u8>)>,
    }

    /// The first state of the fixture (format: work/ragdoll/fixtures/export.py): case 0 is op '5' of tick 169, its
    /// input = RGD0, the state right after GoRagdoll and before any tick.
    fn fixture_rgd0() -> State {
        let d: &[u8] = include_bytes!("testdata/ragdoll_ro0.bin");
        assert_eq!(&d[0..4], b"RGF1");
        let u = |o: &mut usize| {
            let v = u32::from_le_bytes(d[*o..*o + 4].try_into().unwrap());
            *o += 4;
            v
        };
        let mut o = 8;
        assert_eq!(d[o], b'5');
        o += 4;
        assert_eq!(u(&mut o), 169);
        o += 4 + 16;
        let col_addr = u(&mut o);
        let col = d[o..o + COLLIDER_SIZE].to_vec();
        o += COLLIDER_SIZE;
        let body_addr = u(&mut o);
        let body = d[o..o + BODY_SIZE].to_vec();
        o += BODY_SIZE;
        let nl = u(&mut o) as usize;
        let mut links = Vec::new();
        for _ in 0..nl {
            let a = u(&mut o);
            links.push((a, d[o..o + LINK_SIZE].to_vec()));
            o += LINK_SIZE;
        }
        let nj = u(&mut o) as usize;
        let mut joints = Vec::new();
        for _ in 0..nj {
            let (a, ty, sz) = (u(&mut o), u(&mut o), u(&mut o) as usize);
            joints.push((a, ty, d[o..o + sz].to_vec()));
            o += sz;
        }
        State { col_addr, col, body_addr, body, links, joints }
    }

    fn be(b: &[u8], o: usize) -> u32 {
        u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
    }

    fn rider() -> Option<RagdollType> {
        let root = std::env::var_os("MCLA_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("D:/MCLA/extracted/cache"));
        let p = root.join("resources/character/drv_mp_01_mot_set/0x5eb8d142.r6");
        let res = crate::assets::load_resource_file(&p).ok()?;
        parse_character_ragdoll(&res)
    }

    /// The build against the recording (rm.0 / ro.0 tick 169, RGD0): the 675 build-time fields build_model.py
    /// compares (masses, moments, rest frames, joint types, k, pointers, drive modes, limits, drive gains, the
    /// swing-twist bytes), bit for bit; then the whole body block and the collider fields the build sets.
    #[test]
    fn rider_build_matches_recording() {
        let Some(t) = rider() else {
            eprintln!("skipped: no extracted game data with drv_mp_01_mot_set (set MCLA_DATA)");
            return;
        };
        let s = fixture_rgd0();
        let rl = |i: usize| -> &[u8] { &s.links[i].1 };
        // M: the root's rest rows are M transposed (F-16970); M's w lanes are not recorded
        let f = |b: &[u8], o: usize| f32::from_bits(be(b, o));
        let mut m = [[0.0f32; 4]; 3];
        for j in 0..3 {
            for k in 0..3 {
                m[j][k] = f(rl(0), 512 + 16 * k + 4 * j);
            }
        }
        let ca = |o: usize| be(&s.col, o);
        let addrs = GuestAddrs {
            body: s.body_addr,
            collider: s.col_addr,
            links: s.links.iter().map(|l| l.0).collect(),
            joints: s.joints.iter().map(|j| j.0).collect(),
            collider_arrays: [ca(684), ca(692), ca(700), ca(708), ca(716), ca(724), ca(732)],
            inst: ca(16),
            solver_obj: ca(24),
            type248: ca(748),
        };
        let input = BuildInput { m, gravity: f(&s.body, 516), min_softness: 0.0 };
        let r = build(&t, &input, &addrs).unwrap();

        let mut total = 0;
        let mut bad: Vec<String> = Vec::new();
        let mut cmp = |name: String, mine: u32, rec: u32| {
            total += 1;
            if mine != rec {
                bad.push(format!("{name}: built {mine:#010x} recorded {rec:#010x}"));
            }
        };
        cmp("link count".into(), r.links.len() as u32, s.links.len() as u32);
        for i in 1..r.links.len() {
            cmp(format!("L{i} parent"), be(&r.body, 252 + 4 * i), be(&s.body, 252 + 4 * i));
        }
        for (i, lb) in r.links.iter().enumerate() {
            for ln in 0..4 {
                cmp(format!("L{i} mass[{ln}]"), be(lb, 480 + 4 * ln), be(rl(i), 480 + 4 * ln));
            }
            for ln in 0..3 {
                cmp(format!("L{i} moment[{ln}]"), be(lb, 496 + 4 * ln), be(rl(i), 496 + 4 * ln));
            }
            if i == 0 {
                for rr in 0..3 {
                    cmp(format!("L0 rest-row[{rr}].w"), be(lb, 512 + 16 * rr + 12), be(rl(0), 512 + 16 * rr + 12));
                }
                for ln in 0..4 {
                    cmp(format!("L0 rest-pos[{ln}]"), be(lb, 560 + 4 * ln), be(rl(0), 560 + 4 * ln));
                }
                continue;
            }
            for rr in 0..3 {
                for ln in 0..4 {
                    cmp(format!("L{i} rest-row[{rr}][{ln}]"), be(lb, 512 + 16 * rr + 4 * ln), be(rl(i), 512 + 16 * rr + 4 * ln));
                }
            }
            for ln in 0..3 {
                cmp(format!("L{i} rest-pos[{ln}]"), be(lb, 560 + 4 * ln), be(rl(i), 560 + 4 * ln));
            }
        }
        assert_eq!(r.joints.len(), s.joints.len());
        for (k, (ty, jb)) in r.joints.iter().enumerate() {
            let (_, rty, rb) = &s.joints[k];
            cmp(format!("J{k} type"), *ty, *rty);
            for (nm, o) in [("k", 4), ("parent", 8), ("child", 12), ("mode", 528), ("lim608", 608), ("lim612", 612), ("lim616", 616), ("lim620", 620)] {
                cmp(format!("J{k} {nm}"), be(jb, o), be(rb, o));
            }
            if *ty == 0 {
                for o in [672, 676, 680, 684, 688, 692] {
                    cmp(format!("J{k} drive+{o}"), be(jb, o), be(rb, o));
                }
            } else {
                for o in [624, 628] {
                    cmp(format!("J{k} lim+{o}"), be(jb, o), be(rb, o));
                }
                cmp(format!("J{k} byte+632"), jb[632] as u32, rb[632] as u32);
                cmp(format!("J{k} byte+633"), jb[633] as u32, rb[633] as u32);
                for base in [672, 688, 704, 720, 736, 752] {
                    for ln in 0..3 {
                        cmp(format!("J{k} drive+{base}[{ln}]"), be(jb, base + 4 * ln), be(rb, base + 4 * ln));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{} of {total} build-time fields differ:\n{}", bad.len(), bad.join("\n"));
        assert_eq!(total, 675);

        // the body: every byte (what the build does not write is the heap fill in the recording too)
        assert_eq!(r.body, s.body);
        // the collider fields the build sets
        for o in [0usize, 16, 20, 24, 672, 680, 684, 688, 692, 696, 700, 704, 708, 712, 716, 720, 724, 728, 732, 736, 744, 748] {
            assert_eq!(be(&r.collider, o), be(&s.col, o), "collider +{o}");
        }
        assert_eq!(r.collider[740], s.col[740]);
        // the constructors' fields of the joints the recording still holds before the first tick
        for (k, (ty, jb)) in r.joints.iter().enumerate() {
            let rb = &s.joints[k].2;
            assert_eq!(be(jb, 0), be(rb, 0), "J{k} +0");
            assert_eq!(jb[532], rb[532], "J{k} +532");
            if *ty == 0 {
                assert_eq!((be(jb, 624), be(jb, 628)), (be(rb, 624), be(rb, 628)), "J{k} hinge angle / rate");
            } else {
                assert_eq!(&jb[634..668], &rb[634..668], "J{k} +634..+667");
                assert_eq!(be(jb, 816), be(rb, 816), "J{k} +816");
            }
        }
        // the decode's tables (ragdoll_link_joint_build.md 2.2 and 7)
        assert_eq!(r.child_link, [0, 1, 1, 1, 1, 2, 1, 3, 4, 5, 1, 6, 7, 8, 9, 10, 11, 12, 9, 13, 14]);
        assert_eq!(r.link_child, [0, 1, 5, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 19, 20]);
        let pairs: Vec<(i32, i32)> = r.pair_a.iter().zip(&r.pair_b).take_while(|(a, _)| **a != -1).map(|(a, b)| (*a, *b)).collect();
        assert_eq!(pairs, [(1, 7), (1, 11), (8, 1), (8, 3), (8, 5), (8, 14), (12, 1), (12, 3), (12, 5), (12, 14), (15, 18), (16, 19)]);
    }
}
