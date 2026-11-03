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
//!   identity rows at +272 / +320 / +368; the body (gravity factor from the archetype), the collider (base F-33655:
//!   speed limits 500 / 2 pi, identity rows, zeroed vectors and bytes) and its arrays (+716 zeroed); the collider's
//!   mass properties (F-87421): the children's mass (F-25823), its reciprocal, the archetype's inertia floored
//!   (F-88418) and its reciprocals.
//! - F-91939: a link's mass and moments (scaled when its smallest moment is below type+8), rest frame F-55164:
//!   rows = child rotation x M, position = (centre - root child's centre - off) x M (lane w from M's w lanes).
//! - F-49221: one rotation DOF: hinge (type 0) with hard limits lo + 0.03 / hi - 0.03 and inner limits from the
//!   group's fractions (F-13018, F-97677); two or three: swing-twist (type 1), half ranges, F-88270 / F-71882;
//!   k = min(max(G+20, min softness), 1); drives from G+40 .. G+52; AddLink (F-55902). The anchor: the bone's pose
//!   position - the root child's centre - off, through M. The frames (+432 .. +607): the hinge's from its axis (the
//!   unit axis through the bone's pose rows and M) and two unit vectors across it (F-16319, F-40242); the
//!   swing-twist joint's by F-33259 (see `make_joint`), both put in A's and B's frames by F-38962's rule.
//! - F-83435: the part pairs, each child of group a x each child of group b, F-16298 (no duplicates either way
//!   round, kept sorted by (a, b)).
//! - F-97481 at the end of F-62449: the articulated inertias of the built tree (`Articulated::rebuild`), about the
//!   current frames, which are the rest frames then.
//!
//! [`activate`]: what GoRagdoll (F-81936) does after the build: the collider's matrices, the links' frames from
//! the pose (F-93058, artic_pose::seed_links), the reset and the start velocity.
//!
//! Inputs the file does not hold: the instance matrix M (rd+16 at GoRagdoll: rows of [inst+16] with their w lanes;
//! its position row is not used by the build), the archetype's gravity factor and inertia, the run-time
//! least-softness global (0.0 in the running game) and the addresses; for [`activate`] the matrix with its
//! position row, the links' frames and the start velocity.
//!
//! Checked against ro.0's RGD0 of tick 169 (the state right after GoRagdoll): every word of the collider, body,
//! links and joints equal (8060 of 8092) except 32 words that never reach a result, which stay [`FILL`]:
//! - lane w of the swing-twist joint's +672 / +688 / +736 / +752 (stale stack in F-49221 and in its constructor
//!   F-99002): the drive (F-58495, F-11804) reads lanes x y z of the gains, maxima and minima only.
//!
//! Not built: slider joints (translation DOF) and parked joints (G+56 = -1 or > 0), not used by the riders
//! ([`BuildError`]); F-62449's source-entry path (F-71690) and its constraints when T+316 != 0 (0 for riders).

use super::artic::{Articulated, V4 as R4};
use super::exact::{fma, fms, guest_cos, guest_sin, rotation_about_unit};
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

// ---------------------------------------------------------------------------------------------
// The joint frames, in the vector unit's arithmetic ([`R4`]: denormals flushed, multiply-adds rounded twice)

/// (x, y, z, x) from three splatted dot products, as the merges put them: (x, z), (y, that), (that, that).
fn xyzx(x: R4, y: R4, z: R4) -> R4 {
    let a = x.mergeh(z);
    a.mergeh(y.mergeh(a))
}

/// (x, y, z, y): the merges of F-38962's anchors.
fn xyzy(x: R4, y: R4, z: R4) -> R4 {
    x.mergeh(z).mergeh(y)
}

/// The columns of three rows (lane w 0: the three-lane dot products below never read it).
fn cols3(r: &[R4; 3]) -> [R4; 3] {
    [0, 1, 2].map(|k| R4::new(r[0].0[k], r[1].0[k], r[2].0[k], 0.0))
}

/// A row vector times the rows: (v . col x, v . col y, v . col z, v . col x).
fn times_rows(v: R4, r: &[R4; 3]) -> R4 {
    let c = cols3(r);
    xyzx(v.dot3(c[0]), v.dot3(c[1]), v.dot3(c[2]))
}

/// The rows times a vector: (r0 . v, r1 . v, r2 . v, r0 . v).
fn rows_times(r: &[R4; 3], v: R4) -> R4 {
    xyzx(r[0].dot3(v), r[1].dot3(v), r[2].dot3(v))
}

/// -(a.yzx b.zxy - a.zxy b.yzx): the cross product as F-49221 forms it (vnmsubfp), = b x a.
fn cross_ba(a: R4, b: R4) -> R4 {
    a.yzx().nmsub(b.zxy(), a.zxy().mul(b.yzx()))
}

/// -(a.zxy b.yzx - a.yzx b.zxy) = a x b (F-33259, F-40242).
fn cross_ab(a: R4, b: R4) -> R4 {
    a.zxy().nmsub(b.yzx(), a.yzx().mul(b.zxy()))
}

/// A link's frame as the build leaves it (current = rest): rows +416 / +432 / +448, position +464.
#[derive(Clone, Copy)]
struct Cur {
    rows: [R4; 3],
    pos: R4,
}

impl Cur {
    fn of(rest: &[V4; 4]) -> Cur {
        Cur { rows: [R4(rest[0]), R4(rest[1]), R4(rest[2])], pos: R4(rest[3]) }
    }
}

/// F-38962 (and the tail of F-16319): a joint's frame given by three vectors whose x lanes make X, y lanes Y, z
/// lanes Z (merges with zero), and its anchor (world, relative to the collider's place), each in A's and B's frame:
/// row k of a frame = (R0[k] X + R1[k] Y) + R2[k] Z; anchor = the columns dotted with anchor - position, merged
/// (x, y, z, y). Returns +432 / +448 / +464 (A's frame), +480 (A's position), +496 / +512 (the anchor in A / B),
/// +544 / +560 / +576 (B's frame), +592 (B's position).
fn frames(anchor: R4, c: [R4; 3], a: &Cur, b: &Cur) -> [R4; 10] {
    let (h, l) = (c[0].mergeh(c[2]), c[0].mergel(c[2]));
    let (h1, l1) = (c[1].mergeh(R4::ZERO), c[1].mergel(R4::ZERO));
    let (x, y, z) = (h.mergeh(h1), h.mergel(h1), l.mergeh(l1));
    let rows = |k: &Cur| -> [R4; 3] {
        cols3(&k.rows).map(|col| {
            let t = col.lane(0).mul(x);
            let t = col.lane(1).madd(y, t);
            col.lane(2).madd(z, t)
        })
    };
    let anchor_in = |k: &Cur| {
        let d = anchor.sub(k.pos);
        let cl = cols3(&k.rows);
        xyzy(cl[0].dot3(d), cl[1].dot3(d), cl[2].dot3(d))
    };
    let (fa, fb) = (rows(a), rows(b));
    [fa[0], fa[1], fa[2], a.pos, anchor_in(a), anchor_in(b), fb[0], fb[1], fb[2], b.pos]
}

/// Where [`frames`] go in the joint.
const FRAME_AT: [usize; 10] = [432, 448, 464, 480, 496, 512, 544, 560, 576, 592];

/// x / sqrt(x . x) with the estimate and one Newton step (F-40242): r + (1 - l r r) (0.5 r).
fn unit_nr(v: R4) -> R4 {
    let l = v.dot3(v);
    let r = l.rsqrte();
    let e = l.nmsub(r.mul(r), R4::splat(1.0));
    v.mul(e.madd(R4::splat(0.5).mul(r), r))
}

/// F-40242: two unit vectors across the unit axis n: u from (ny, -nx, 0) when |nx| or |ny| > 0.5, else from
/// (0, nz, -ny); w = unit(n x u). Returns (u, w) (the callee's r3, r4).
fn across(n: R4) -> (R4, R4) {
    let neg = R4::ZERO.sub(n);
    let big = n.mergeh(R4::splat(-0.5)).gt(R4::splat(0.5).mergeh(n)).iter().any(|b| *b);
    let v = if big { n.lane(1).mergeh(R4::ZERO).mergeh(neg) } else { R4::ZERO.mergeh(neg).mergel(n.lane(2)) };
    let u = unit_nr(v);
    (u, unit_nr(cross_ab(n, u)))
}

/// F-28753: the rows of the rotation by the angles (x, y, z) (a zero angle: exactly 1 / 0), 3 x 3.
fn euler_rows(a: [f32; 3]) -> [[f32; 3]; 3] {
    let cs = |x: f32| if x == 0.0 { (1.0, 0.0) } else { (guest_cos(x as f64) as f32, guest_sin(x as f64) as f32) };
    let ((cx, sx), (cy, sy), (cz, sz)) = (cs(a[0]), cs(a[1]), cs(a[2]));
    [
        [cz * cy, sz * cy, -sy],
        [fms(cz * sy, sx, sz * cx), fma(sz * sy, sx, cz * cx), cy * sx],
        [fma(cz * sy, cx, sz * sx), fms(sz * sy, cx, cz * sx), cy * cx],
    ]
}

/// F-50812: the quaternion (x, y, z, w) of a rotation, from the largest of the trace and the diagonal.
fn quat_of(m: &[[f32; 3]; 3]) -> [f32; 4] {
    let (m00, m11, m22) = (m[0][0], m[1][1], m[2][2]);
    let tr = (m00 + m11) + m22;
    let y_or_z = if !(m11 < m22) { 2 } else { 3 };
    let case = if !(tr < m00) {
        if tr < m11 {
            y_or_z
        } else if tr < m22 {
            3
        } else {
            0
        }
    } else if m00 < m11 {
        y_or_z
    } else if m00 < m22 {
        3
    } else {
        1
    };
    let (half, quarter) = (0.5f32, 0.25f32);
    let big = |d: f32| (fms(d, 2.0, tr) + 1.0).sqrt() * half;
    match case {
        0 => {
            let w = (tr + 1.0).sqrt() * half;
            let s = quarter / w;
            [(m[1][2] - m[2][1]) * s, (m[2][0] - m[0][2]) * s, (m[0][1] - m[1][0]) * s, w]
        }
        1 => {
            let x = big(m00);
            let s = quarter / x;
            [x, (m[1][0] + m[0][1]) * s, (m[2][0] + m[0][2]) * s, (m[1][2] - m[2][1]) * s]
        }
        2 => {
            let y = big(m11);
            let s = quarter / y;
            [(m[1][0] + m[0][1]) * s, y, (m[2][1] + m[1][2]) * s, (m[2][0] - m[0][2]) * s]
        }
        _ => {
            let z = big(m22);
            let s = quarter / z;
            [(m[2][0] + m[0][2]) * s, (m[2][1] + m[1][2]) * s, z, (m[0][1] - m[1][0]) * s]
        }
    }
}

/// F-40856: the rows of the quaternion's rotation (scaled by sqrt 2 first). It writes lanes x y z only; lane w of
/// each row is what the caller's stack held there (`w`).
fn quat_rows(q: [f32; 4], w: [f32; 3]) -> [R4; 3] {
    let k = f32::from_bits(0x3fb5_04f3);
    let (x, y, z, s) = (q[0] * k, q[1] * k, q[2] * k, q[3] * k);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (yx, zx, sy, sz, zy, sx) = (y * x, z * x, s * y, s * z, z * y, s * x);
    let (yy_xx, zz_yy, zz_xx) = (yy + xx, zz + yy, zz + xx);
    [
        R4::new(1.0 - zz_yy, yx + sz, zx - sy, w[0]),
        R4::new(yx - sz, 1.0 - zz_xx, sx + zy, w[1]),
        R4::new(sy + zx, zy - sx, 1.0 - yy_xx, w[2]),
    ]
}

/// F-33259's turn of a frame: row i = (Q0 f_i.x + Q1 f_i.y) + Q2 f_i.z.
fn turn_frame(q: &[R4; 3], f: [R4; 3]) -> [R4; 3] {
    f.map(|r| {
        let t = q[0].mul(r.lane(0));
        let t = q[1].madd(r.lane(1), t);
        q[2].madd(r.lane(2), t)
    })
}

/// F-88418: no moment below 1 / 20 of the largest (found by x < y, then against z): such a lane becomes 0.05 x
/// the largest.
fn floor_inertia(mut i: V4) -> V4 {
    let (x, y, z) = (i[0], i[1], i[2]);
    let k = if !(x < y) {
        if !(x < z) {
            0
        } else {
            2
        }
    } else if !(y < z) {
        1
    } else {
        2
    };
    for l in (0..3).filter(|l| *l != k) {
        if !(i[l] * 20.0 >= i[k]) {
            i[l] = i[k] * f32::from_bits(0x3d4c_cccd);
        }
    }
    i
}

/// The angular inertia of a fresh cache entry's archetype (entry+32, what F-16970 reads through its vfunc 9 =
/// entry+96): made when GoRagdoll gets the rider a cache entry (F-95132 -> F-71690 -> F-59990 -> F-13409), from the
/// composite bound's part matrices at that moment. F-11525 has just set them from the skeleton (F-36901 through the
/// instance's vtable +188: the eject clip's last frame in rd+16's frame, what [`super::artic_pose::activate_parts`]
/// makes), so the inertia follows the rider's pose at the throw; the .r6's own archetype (128.1, 6.5, 130.3) is not
/// used.
///
/// F-13409, the articulated branch (F-36601: at least two whole children whose bone has a free DOF, F-41462): the
/// bound's centre-of-gravity offset is the type bound's (F-83058), plus T+64 while the entry is younger than 0.25
/// (always, a new entry has age 0). Then F-23677 -> F-68804 over the parts (a missing part skipped; a whole child's
/// mass +4 and moments T+240[i]): `I_i + m (dy^2 + dz^2, dx^2 + dz^2, dx^2 + dy^2)` (fused, single rounding) with
/// `d = (part position + part cg) - bound cg`, summed in vector adds from 0 (lane w the moments' own), and set by
/// F-58768 with the floor F-88418. Not taken: the volume rule of a type without free bones (F-13409's other branch)
/// and the damaged moments (T+244, child +8) of a group at no health: `None` for the first, the second never
/// happens on a new entry.
///
/// `parts`: the part matrices (bound +132), one per child.
pub fn archetype_inertia(t: &RagdollType, parts: &[[R4; 4]]) -> Option<V4> {
    let free = t
        .children
        .iter()
        .filter(|c| c.bone >= 0 && t.bones.get(c.bone as usize).map_or(false, |b| b.dofs().iter().any(|&d| d)))
        .count();
    if free < 2 {
        return None;
    }
    let cg = R4(t.bound_cg).add(R4(t.cg_shift));
    let mut acc = R4::ZERO;
    let mut total = 0.0f32;
    for (i, part) in t.parts.iter().enumerate() {
        if !part.present {
            continue;
        }
        let (Some(c), Some(own), Some(m4)) = (t.children.get(i), t.inertia.get(i), parts.get(i)) else { continue };
        let m = c.mass;
        let d = m4[3].add(R4(part.cg_offset)).sub(cg);
        let (xx, yy, zz) = (d.0[0] * d.0[0], d.0[1] * d.0[1], d.0[2] * d.0[2]);
        let mut own = *own;
        own[0] = fma(yy + zz, m, own[0]);
        own[1] = fma(xx + zz, m, own[1]);
        own[2] = fma(xx + yy, m, own[2]);
        total = m + total;
        acc = acc.add(R4(own));
    }
    // (F-13409 sets the moments only over a positive mass: 1.1754944e-38)
    if !(total > f32::from_bits(0x0080_0000)) {
        return None;
    }
    Some(floor_inertia(acc.0))
}

/// The low word of a float register saved as a double (what a callee's stack slot keeps of it).
fn low_word(x: f32) -> f32 {
    f32::from_bits((x as f64).to_bits() as u32)
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
    /// the angular inertia the cache entry's archetype answers (its vfunc 9, read by F-16970 for the collider's
    /// mass properties): [`archetype_inertia`] of the throw's pose, not the .r6's archetype (128.1, 6.5, 130.3)
    pub inertia: V4,
}

impl BuildInput {
    pub fn new(m: [V4; 3], inertia: V4) -> Self {
        BuildInput { m, gravity: 1.0, min_softness: 0.0, inertia }
    }
}

/// GoRagdoll's steps after the build (F-81936 11 - 13): what they take from outside the body.
#[derive(Debug, Clone)]
pub struct Activation {
    /// rd+16: the instance matrix with its position row and w lanes (copied into collider +32 and +96)
    pub r: [V4; 4],
    /// the links' frames F-93058 sets (artic_pose::seed_links): rows +416 / +432 / +448, position +464
    pub frames: Vec<([V4; 3], V4)>,
    /// the start velocity seq+144 + (0, 1, 0, 0) (rider_eject), lane w as the guest holds it
    pub velocity: V4,
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
    /// the frames, anchors and positions at [`FRAME_AT`]
    fr: [R4; 10],
}

/// What a joint's frames are made from: the instance matrix's rows, the joint's anchor (world, about the root
/// child's centre) and the two links' frames.
struct FrameCtx {
    m: [R4; 3],
    anchor: R4,
    pose: [R4; 3],
    a: Cur,
    b: Cur,
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
    for (v, o) in j.fr.iter().zip(FRAME_AT) {
        put_v(&mut b, o, v.0);
    }
    put_f(&mut b, 4, j.k);
    put_u32(&mut b, 8, a.links[j.parent]);
    put_u32(&mut b, 12, child);
    put_u32(&mut b, 528, j.mode);
    b
}

/// F-49221's numbers for one joint.
fn make_joint(t: &RagdollType, child: usize, bone: usize, dof: [bool; 6], min_softness: f32, fc: &FrameCtx) -> Result<Joint, BuildError> {
    let g = &t.groups[t.children[child].group as usize];
    let bn = &t.bones[bone];
    if dof[3] || dof[4] || dof[5] {
        return Err(BuildError::Slider(child));
    }
    let (rx, ry, rz) = (dof[0], dof[1], dof[2]);
    let nrot = rx as u32 + ry as u32 + rz as u32;
    let mut j = Joint { ty: 0, parent: 0, k: 0.0, mode: 0, lim: [0.0; 6], b632: 0, b633: 0, hd: [0.0; 6], bv: [[0.0; 3]; 6], vw: [None; 6], fr: [R4::ZERO; 10] };
    let (zero, one, half) = (0.0f32, 1.0f32, 0.5f32);
    let unit = |k: usize| {
        let mut e = R4::ZERO;
        e.0[k] = 1.0;
        e
    };
    if nrot == 1 {
        // hinge (F-77258), F-13018, F-97677
        j.ty = 0;
        let ax = if rx { 0 } else if ry { 1 } else { 2 };
        // the axis: the unit axis through the bone's pose rows, then M (F-49221); the frame F-16319 (F-40242)
        let axis = times_rows(times_rows(unit(ax), &fc.pose), &fc.m);
        let (u, w) = across(axis);
        j.fr = frames(fc.anchor, [u, w, axis], &fc.a, &fc.b);
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
        // the frame (F-49221): two unit axes (A's: z, B's: y; z and y: x, y; z and x: y, x) through the bone's pose
        // rows, turned about their cross product by A's middle angle (F-65964), through M, back through the pose
        // rows; F-33259 makes the frame from (b, a x b, a) (F-38962) and turns A's by the quaternion (F-50812) of
        // the angles -0.5 |z, y, x sums| (F-28753), B's by the unit quaternion (F-40856 both)
        let (e1, e2) = if nrot == 3 || !rz {
            (2, 1)
        } else if rx {
            (1, 0)
        } else {
            (0, 1)
        };
        let a1 = times_rows(unit(e1), &fc.pose);
        let a2 = times_rows(unit(e2), &fc.pose);
        let mid = (bn.rot_max[a_ax] + bn.rot_min[a_ax]) * half;
        let ax = cross_ba(a1, a2);
        let turn = rotation_about_unit(glam::Vec3::new(ax.0[0], ax.0[1], ax.0[2]), mid);
        let turn = turn.map(|r| R4::new(r.x, r.y, r.z, 0.0));
        let a_w = times_rows(times_rows(a1, &turn), &fc.m);
        let b_w = times_rows(times_rows(a2, &turn), &fc.m);
        let a = rows_times(&fc.pose, a_w);
        let b = rows_times(&fc.pose, b_w);
        let sum = |k: usize| (bn.rot_max[k] + bn.rot_min[k]).abs() * -0.5;
        let q = quat_of(&euler_rows([sum(2), sum(1), sum(0)]));
        // lane w of F-40856's rows: the stack slots F-28753's prologue saved F-49221's f26 / f28 / f30 in (the twist
        // range, B's lower angle, B's half range), their low words
        let stale = [low_word(tw), low_word(bn.rot_min[b_ax]), low_word(sb)];
        let mut fr = frames(fc.anchor, [b, cross_ab(a, b), a], &fc.a, &fc.b);
        let fa = turn_frame(&quat_rows(q, stale), [fr[0], fr[1], fr[2]]);
        let fb = turn_frame(&quat_rows([0.0, 0.0, 0.0, 1.0], stale), [fr[6], fr[7], fr[8]]);
        fr[0..3].copy_from_slice(&fa);
        fr[6..9].copy_from_slice(&fb);
        j.fr = fr;
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
            // the anchor (F-49221): the bone's pose position - the root child's centre - off, through M
            let parent = get(&map, r27) as usize;
            let pose = t.bone_pose.get(b as usize).ok_or(BuildError::BadBone(i))?;
            let p = R4(pose[3]).sub(R4(t.children[root_child].pos)).sub(R4(off));
            let mr = m.map(R4);
            let fc = FrameCtx { m: mr, anchor: times_rows(p, &mr), pose: [R4(pose[0]), R4(pose[1]), R4(pose[2])], a: Cur::of(&links[parent].rest), b: Cur::of(&rest) };
            let mut jt = make_joint(t, i, b as usize, dof, input.min_softness, &fc)?;
            jt.parent = parent;
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
    // the base constructor (F-33655): speed limits, identity rows, place, zeroed vectors and bytes
    put_f(&mut col, 200, f32::from_bits(0x43fa_0000));
    put_f(&mut col, 204, c(TWO_PI));
    for (r, row) in [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0; 4]].iter().enumerate() {
        put_v(&mut col, 208 + 16 * r, *row);
    }
    for o in [272, 288, 304, 320, 400, 432] {
        put_v(&mut col, o, [0.0; 4]);
    }
    col[658..661].fill(0);
    // F-16970 -> F-87421: the mass (the children's, F-25823 adds them in order), its reciprocal, the archetype's
    // inertia with its floor (F-88418) and reciprocals (1e8 where 0)
    let mass = t.children.iter().fold(0.0f32, |s, ch| ch.mass + s);
    put_f(&mut col, 192, mass);
    put_f(&mut col, 196, if mass > f32::from_bits(0x2b8c_bccc) { 1.0 / mass } else { c(FLT_MAX) });
    let inertia = R4(floor_inertia(input.inertia));
    let r = inertia.recip();
    let inv = r.madd(r.nmsub(inertia, R4::splat(1.0)), r).sel(R4::splat(c(C1E8)), inertia.eq(R4::ZERO));
    put_v(&mut col, 160, inertia.0);
    put_v(&mut col, 176, inv.0);

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

    // F-97481 at the end of F-62449: the inertias of the built tree, about the links' current (= rest) frames
    let (link_blocks, joint_blocks) = {
        let lk: Vec<(u32, &[u8])> = link_blocks.iter().enumerate().map(|(i, b)| (addrs.links[i], b.as_slice())).collect();
        let jk: Vec<&[u8]> = joint_blocks.iter().map(|j| j.1.as_slice()).collect();
        let mut art = Articulated::from_guest(&col, &body, &lk, &jk);
        art.rebuild();
        let jb = joint_blocks.iter().enumerate().map(|(k, j)| (j.0, art.joint_to_guest(k))).collect::<Vec<_>>();
        (art.links.iter().map(|l| l.to_guest()).collect::<Vec<_>>(), jb)
    };

    let link_child = links.iter().map(|l| l.child).collect();
    Ok(ArticBuild { body, links: link_blocks, joints: joint_blocks, collider: col, child_link, pair_a, pair_b, link_child, parents })
}

/// GoRagdoll after the build (F-81936): on a successful activation (F-24099) rd's matrix into the collider's +32
/// and +96; the links' frames F-93058 sets (its velocities are wiped next); the reset (F-47797: F-50121 the
/// collider's velocities and momenta, F-23679 -> F-54596 its +336 .. +463 and F-63136, F-14567 every link's
/// motion, forces, impulses and the solver's vectors, every joint's propagation vectors, the hinge's rate, the
/// swing-twist joint's rates); the velocity (F-86625: F-64271 +272 = v, +304 = v x mass; F-91185 every link
/// +608 += v - link 0's, +640 = +624, +656 = v + c x w).
pub fn activate(b: &mut ArticBuild, act: &Activation) {
    for (r, row) in act.r.iter().enumerate() {
        put_v(&mut b.collider, 32 + 16 * r, *row);
        put_v(&mut b.collider, 96 + 16 * r, *row);
    }
    for (l, (rows, pos)) in b.links.iter_mut().zip(&act.frames) {
        for (r, row) in rows.iter().enumerate() {
            put_v(l, 416 + 16 * r, *row);
        }
        put_v(l, 464, *pos);
    }
    for o in (272..464).step_by(16) {
        put_v(&mut b.collider, o, [0.0; 4]);
    }
    for l in b.links.iter_mut() {
        for o in [608, 624, 640, 656, 672, 688, 704, 720, 880, 896, 912, 928, 944, 960, 976, 992] {
            put_v(l, o, [0.0; 4]);
        }
    }
    for (ty, j) in b.joints.iter_mut() {
        for o in [304, 320, 336, 352, 368, 384, 400, 416] {
            put_v(j, o, [0.0; 4]);
        }
        if *ty == 1 {
            put_f(j, 652, 0.0);
            put_f(j, 656, 0.0);
        } else {
            put_f(j, 628, 0.0);
        }
    }
    let rv = |b: &[u8], o: usize| R4(std::array::from_fn(|k| f32::from_bits(u32::from_be_bytes(b[o + 4 * k..o + 4 * k + 4].try_into().unwrap()))));
    let v = R4(act.velocity);
    let mass = rv(&b.collider, 192).0[0];
    put_v(&mut b.collider, 272, v.0);
    put_v(&mut b.collider, 304, v.mul(R4::splat(mass)).0);
    let d = v.sub(rv(&b.links[0], 608));
    for l in b.links.iter_mut() {
        let vel = rv(l, 608).add(d);
        let (ang, pos) = (rv(l, 624), rv(l, 464));
        put_v(l, 608, vel.0);
        put_v(l, 640, ang.0);
        put_v(l, 656, pos.zxy().nmsub(ang.yzx(), pos.yzx().madd(ang.zxy(), vel)).0);
    }
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
        rider_set("drv_mp_01_mot_set/0x5eb8d142.r6")
    }

    fn rider_set(file: &str) -> Option<RagdollType> {
        let root = std::env::var_os("MCLA_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("D:/MCLA/extracted/cache"));
        let p = root.join("resources/character").join(file);
        let res = crate::assets::load_resource_file(&p).ok()?;
        parse_character_ragdoll(&res)
    }

    /// The part matrices GoRagdoll's cache entry sees (F-11525 -> F-36901): the eject clip's last frame of ro.0
    /// (the pose fixture's locals) under rd+16 (r_act).
    fn activation_parts(t: &RagdollType) -> crate::physics::artic_pose::Parts {
        use crate::physics::artic_pose::{activate_parts, fk, Parts};
        let pf = pose_fixture();
        let mat = |n: &str, i: usize| -> [R4; 4] {
            let w = &pf[n][16 * i..16 * i + 16];
            [0, 1, 2, 3].map(|r| R4::from_bits([w[4 * r], w[4 * r + 1], w[4 * r + 2], w[4 * r + 3]]))
        };
        let r = mat("r_act", 0);
        let locals: Vec<[R4; 4]> = (0..pf["locals"].len() / 16).map(|i| mat("locals", i)).collect();
        let g = fk(&locals, &t.bone_parent, &r);
        let mut parts = Parts::new(t.children.len());
        activate_parts(t, &g, &r, &[], &mut parts);
        parts
    }

    /// The archetype's run-time inertia (F-13409 / F-68804 / F-58768) from the throw's pose: collider +160 of ro.0's
    /// RGD0 (tick 169) bit for bit, lane w (NaN) too: (277.52197, 77.846954, 207.65668). The player's rider is
    /// drv_mp_01_mot_set_h (its .r6 is the resource the recording's parts lie in); mot_set's ragdoll tables give the
    /// same.
    #[test]
    fn archetype_inertia_matches_recording() {
        let (_, rgd0, _) = case0();
        let want: Vec<u32> = (0..4).map(|k| be(&rgd0.col, 160 + 4 * k)).collect();
        assert_eq!(want, [0x438a_c2d0, 0x429b_b1a4, 0x434f_a81c, 0x7fc0_0001]);
        let mut n = 0;
        for file in ["drv_mp_01_mot_set_h/0x9afc7d46.r6", "drv_mp_01_mot_set/0x5eb8d142.r6"] {
            let Some(t) = rider_set(file) else { continue };
            let parts = activation_parts(&t);
            let got = archetype_inertia(&t, &parts.cur).expect("an articulated type");
            assert_eq!(got.map(f32::to_bits).to_vec(), want, "{file}");
            eprintln!("archetype inertia of {file}: bit for bit");
            n += 1;
        }
        if n == 0 {
            eprintln!("skipped: no extracted game data with drv_mp_01_mot_set(_h) (set MCLA_DATA)");
        }
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
        let input = BuildInput { m, gravity: f(&s.body, 516), min_softness: 0.0, inertia: std::array::from_fn(|k| f(&s.col, 160 + 4 * k)) };
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

    /// A state of the fixture: the collider, the body, the links (with their addresses) and the joints.
    struct Blocks {
        col: Vec<u8>,
        body: Vec<u8>,
        links: Vec<(u32, Vec<u8>)>,
        joints: Vec<Vec<u8>>,
    }

    fn blocks(d: &[u8], o: &mut usize) -> Blocks {
        let u = |o: &mut usize| {
            let v = u32::from_le_bytes(d[*o..*o + 4].try_into().unwrap());
            *o += 4;
            v
        };
        let take = |o: &mut usize, n: usize| {
            *o += n;
            d[*o - n..*o].to_vec()
        };
        u(o);
        let col = take(o, COLLIDER_SIZE);
        u(o);
        let body = take(o, BODY_SIZE);
        let links = (0..u(o)).map(|_| (u(o), take(o, LINK_SIZE))).collect();
        let joints = (0..u(o))
            .map(|_| {
                let (_, _, n) = (u(o), u(o), u(o));
                take(o, n as usize)
            })
            .collect();
        Blocks { col, body, links, joints }
    }

    /// Case 0 of the fixture: op '5' of tick 169, slot 5's dt (v1), its input RGD0 and output RGX0.
    fn case0() -> (f32, Blocks, Blocks) {
        let d: &[u8] = include_bytes!("testdata/ragdoll_ro0.bin");
        assert_eq!((d[8], u32::from_le_bytes(d[12..16].try_into().unwrap())), (b'5', 169));
        let dt = f32::from_bits(u32::from_le_bytes(d[20..24].try_into().unwrap()));
        let mut o = 36;
        let rgd0 = blocks(d, &mut o);
        let rgx0 = blocks(d, &mut o);
        (dt, rgd0, rgx0)
    }

    /// testdata/ragdoll_pose_ro0.bin (work/ragdoll/skin/export_pose.py): named sections of u32 words.
    fn pose_fixture() -> std::collections::HashMap<String, Vec<u32>> {
        let d: &[u8] = include_bytes!("testdata/ragdoll_pose_ro0.bin");
        assert_eq!(&d[..8], b"RGPOSE01");
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let mut o = 12;
        let mut m = std::collections::HashMap::new();
        for _ in 0..u(8) {
            let name = String::from_utf8(d[o..o + 8].iter().copied().take_while(|&b| b != 0).collect()).unwrap();
            let n = u(o + 8) as usize;
            m.insert(name, (0..n).map(|i| u(o + 12 + 4 * i)).collect());
            o += 12 + 4 * n;
        }
        m
    }

    /// What slot 5 reads outside the body (artic.rs's test values: the run-time gravity and resting limit, the
    /// rider archetype's damping and bound, from the recording's RGDS pages).
    fn surroundings() -> crate::physics::artic::Surroundings {
        let s = |b: u32| f32::from_bits(b);
        let d = |b: u32| R4::from_bits([b, b, b, 0x7f80_0001]);
        crate::physics::artic::Surroundings {
            gravity: R4::new(0.0, s(0xc11c_cccd), 0.0, 0.0),
            bound: [R4::from_bits([0x3f09_804a, 0x3f4e_998e, 0x3ee0_da07, 0x3ff7_7a06]), R4::from_bits([0xbf9a_5900, 0xbf80_067f, 0xbf84_e24c, 0xbf80_067f])],
            no_gravity: false,
            damping: Some([d(0x3ca3_d70a), d(0x3ca3_d70a), d(0x3c23_d70a), d(0x3ca3_d70a), d(0x3ca3_d70a), d(0x3c23_d70a)]),
            rest_limit: s(0x3be5_6042),
        }
    }

    /// The rider built from the .r6 and activated as GoRagdoll does it, with the run-time inputs of ro.0's tick 169:
    /// rd+16 (the pose fixture's r_act), the links' frames F-93058 gives from the eject clip's last frame
    /// (artic_pose, from the pose fixture), the archetype's inertia from that pose ([`archetype_inertia`]), the gravity
    /// factor and the start velocity (RGD0),
    /// the guest addresses (RGD0).
    fn built_rider(t: &RagdollType, rgd0: &Blocks) -> ArticBuild {
        use crate::physics::artic_pose::{activate_parts, fk, seed_links, Parts};
        let pf = pose_fixture();
        let mat = |n: &str, i: usize| -> [R4; 4] {
            let w = &pf[n][16 * i..16 * i + 16];
            [0, 1, 2, 3].map(|r| R4::from_bits([w[4 * r], w[4 * r + 1], w[4 * r + 2], w[4 * r + 3]]))
        };
        let r = mat("r_act", 0);
        let place = R4::from_bits(pf["cpos_act"][..4].try_into().unwrap());
        let locals: Vec<[R4; 4]> = (0..pf["locals"].len() / 16).map(|i| mat("locals", i)).collect();
        let f = |b: &[u8], o: usize| f32::from_bits(be(b, o));
        let v = |b: &[u8], o: usize| -> V4 { std::array::from_fn(|k| f(b, o + 4 * k)) };
        let ca = |o: usize| be(&rgd0.col, o);
        let addrs = GuestAddrs {
            body: be(&rgd0.col, 672),
            collider: 0,
            links: rgd0.links.iter().map(|l| l.0).collect(),
            joints: (0..rgd0.joints.len()).map(|k| be(&rgd0.body, 132 + 4 * k)).collect(),
            collider_arrays: [ca(684), ca(692), ca(700), ca(708), ca(716), ca(724), ca(732)],
            inst: ca(16),
            solver_obj: ca(24),
            type248: ca(748),
        };
        let g = fk(&locals, &t.bone_parent, &r);
        let mut parts = Parts::new(t.children.len());
        activate_parts(t, &g, &r, &[], &mut parts);
        let inertia = archetype_inertia(t, &parts.cur).unwrap();
        let input = BuildInput { m: [r[0].0, r[1].0, r[2].0], gravity: f(&rgd0.body, 516), min_softness: 0.0, inertia };
        let mut b = build(t, &input, &addrs).unwrap();
        let frames = seed_links(t, &b.child_link, b.links.len(), &parts, &r, place, &[]);
        let act = Activation {
            r: r.map(|x| x.0),
            frames: frames.iter().map(|l| l.expect("every link seeded")).map(|l| (l.rows.map(|x| x.0), l.pos.0)).collect(),
            velocity: v(&rgd0.col, 272),
        };
        activate(&mut b, &act);
        b
    }

    /// Words of RGD0 the build and activation do not reproduce, and why the tick never reads them (shown by the code
    /// and by `unread_words_reach_nothing`): the swing-twist joint's +684 / +700 / +748 / +764, lane w of the
    /// drive's position gains, velocity gains, maxima and minima (stale stack in F-49221 / F-99002); the drive
    /// (F-58495, F-11804) takes lanes x y z only, and nothing else reads them.
    fn not_read(block: &str, ty: u32, o: usize) -> bool {
        block.starts_with('J') && ty == 1 && [684, 700, 748, 764].contains(&o)
    }

    /// Every word of the built and activated rider against RGD0 of tick 169 (collider, body, 15 links, 14 joints),
    /// except the words [`not_read`] lists.
    #[test]
    fn rider_ragdoll_matches_rgd0() {
        let Some(t) = rider() else {
            eprintln!("skipped: no extracted game data with drv_mp_01_mot_set (set MCLA_DATA)");
            return;
        };
        let (_, rgd0, _) = case0();
        let b = built_rider(&t, &rgd0);
        let (mut total, mut same, mut skipped) = (0, 0, 0);
        let mut bad = Vec::new();
        let mut cmp = |name: String, ty: u32, got: &[u8], want: &[u8]| {
            assert_eq!(got.len(), want.len(), "{name} size");
            for o in (0..want.len()).step_by(4) {
                total += 1;
                if be(got, o) == be(want, o) {
                    same += 1;
                } else if not_read(&name, ty, o) {
                    skipped += 1;
                } else {
                    bad.push(format!("{name}+{o}: built {:08x} recorded {:08x}", be(got, o), be(want, o)));
                }
            }
        };
        cmp("collider".into(), 0, &b.collider, &rgd0.col);
        cmp("body".into(), 0, &b.body, &rgd0.body);
        for (i, l) in b.links.iter().enumerate() {
            cmp(format!("L{i}"), 0, l, &rgd0.links[i].1);
        }
        for (k, (ty, j)) in b.joints.iter().enumerate() {
            cmp(format!("J{k}"), *ty, j, &rgd0.joints[k]);
        }
        eprintln!("built rider vs RGD0: {same} / {total} words equal, {skipped} differing words never read, {} other", bad.len());
        assert!(bad.is_empty(), "{} words differ:\n{}", bad.len(), bad.join("\n"));
    }


    /// The articulated body of the blocks (the links at RGD0's addresses), `ticks` times slot 5, then (when `more`)
    /// slot 10 and the post-push after each; every word of the result (collider, body, links, joints) with its name.
    fn run_ticks(b: &ArticBuild, addr: &[u32], dt: f32, ticks: usize, more: bool) -> Vec<(String, u32, Vec<u8>)> {
        let links: Vec<(u32, &[u8])> = b.links.iter().zip(addr).map(|(l, a)| (*a, l.as_slice())).collect();
        let joints: Vec<&[u8]> = b.joints.iter().map(|j| j.1.as_slice()).collect();
        let mut a = Articulated::from_guest(&b.collider, &b.body, &links, &joints);
        for _ in 0..ticks {
            a.step(dt, &surroundings());
            if more {
                a.velocity_update(dt);
                a.post_push(dt);
            }
        }
        let mut out = vec![("collider".to_string(), 0, a.collider_to_guest()), ("body".to_string(), 0, a.body_to_guest())];
        out.extend(a.links.iter().enumerate().map(|(i, l)| (format!("L{i}"), 0, l.to_guest())));
        out.extend((0..a.joints.len()).map(|k| (format!("J{k}"), b.joints[k].0, a.joint_to_guest(k))));
        out
    }

    /// Slot 5 (`Articulated::step`) from the built rider: RGX0 of tick 169 word for word, words NaN on both sides
    /// aside and the [`not_read`] words, which slot 5 passes through as they were.
    #[test]
    fn tick_from_the_build_matches_rgx0() {
        let Some(t) = rider() else {
            eprintln!("skipped: no extracted game data with drv_mp_01_mot_set (set MCLA_DATA)");
            return;
        };
        let (dt, rgd0, rgx0) = case0();
        let b = built_rider(&t, &rgd0);
        let addr: Vec<u32> = rgd0.links.iter().map(|l| l.0).collect();
        let got = run_ticks(&b, &addr, dt, 1, false);
        let mut want = vec![&rgx0.col, &rgx0.body];
        want.extend(rgx0.links.iter().map(|l| &l.1));
        want.extend(rgx0.joints.iter());
        let (mut total, mut same, mut nan, mut unread) = (0, 0, 0, 0);
        let mut bad = Vec::new();
        for ((name, ty, g), w) in got.iter().zip(want) {
            for o in (0..w.len()).step_by(4) {
                total += 1;
                let (x, y) = (be(g, o), be(w, o));
                if x == y {
                    same += 1;
                } else if f32::from_bits(x).is_nan() && f32::from_bits(y).is_nan() {
                    nan += 1;
                } else if not_read(name, *ty, o) {
                    unread += 1;
                } else {
                    bad.push(format!("{name}+{o}: {x:08x} want {y:08x}"));
                }
            }
        }
        eprintln!("slot 5 from the build vs RGX0: {same} / {total} words equal, {nan} NaN on both sides, {unread} never read, {} other", bad.len());
        assert!(bad.is_empty(), "{} words differ:\n{}", bad.len(), bad[..bad.len().min(40)].join("\n"));
    }

    /// The [`not_read`] words reach nothing: each set to other values, two ticks of slot 5, slot 10 and the post-push
    /// leave every other word of the body as it was.
    #[test]
    fn unread_words_reach_nothing() {
        let Some(t) = rider() else {
            eprintln!("skipped: no extracted game data with drv_mp_01_mot_set (set MCLA_DATA)");
            return;
        };
        let (dt, rgd0, _) = case0();
        let b = built_rider(&t, &rgd0);
        let addr: Vec<u32> = rgd0.links.iter().map(|l| l.0).collect();
        let base = run_ticks(&b, &addr, dt, 2, true);
        let mut n = 0;
        for k in 0..b.joints.len() {
            for o in (0..b.joints[k].1.len()).step_by(4) {
                if !not_read(&format!("J{k}"), b.joints[k].0, o) {
                    continue;
                }
                n += 1;
                for v in [0x0000_0000u32, 0x8000_0000, 0x3f80_0000, 0xc61c_4000, 0x7fc0_0000, 0x0000_0420] {
                    let mut c = b.clone();
                    put_u32(&mut c.joints[k].1, o, v);
                    for ((name, _, x), (_, _, y)) in run_ticks(&c, &addr, dt, 2, true).iter().zip(&base) {
                        for w in (0..y.len()).step_by(4) {
                            let (p, q) = (be(x, w), be(y, w));
                            let skip = *name == format!("J{k}") && w == o;
                            assert!(skip || p == q || (f32::from_bits(p).is_nan() && f32::from_bits(q).is_nan()), "J{k}+{o} = {v:08x} reaches {name}+{w}");
                        }
                    }
                }
            }
        }
        assert_eq!(n, 32);
    }
}
