//! The thrown rider's jointed body (the original's articulated collider and body) written out
//! operation for operation: its PreCollide (F-75381: motors, move, gravity, damping, joint
//! limits, prediction), its velocity update (F-69485) and its step after the push solve
//! (F-48210). The island solve between them (contacts and joint-limit rows) is not here.
//!
//! The state is the guest's own: four-lane vectors, positions relative to the collider's
//! place, every lane kept (the w lanes carry what the vector code leaves there, NaN at times).
//! Conventions of the code as it runs (the game is built for SSE4.1 by the recompiler):
//! - the vector unit flushes denormals in and out; its multiply-add is a product rounded and
//!   then a sum, `vnmsubfp` is -(a b - c) (so an exact 0 comes out -0);
//! - its three-term dot product is (0 + z z') + (y y' + x x'), splatted to every lane;
//! - its reciprocal and reciprocal square root are the exact 1 / x and 1 / sqrt(x) in single
//!   precision, Newton steps written out where the code has them;
//! - the scalar unit is single precision except its fused `fmadds` family (exact::fma).
//!
//! Decode: docs/reference/vehicle/ragdoll_solver_decode.md (layouts 2.2 - 2.5, passes 3.x),
//! ragdoll_swing_twist.md (the type 1 joint). Checked word for word against the original run
//! offline on recorded states (testdata/ragdoll_ro0.bin; tests below).

use super::exact::{fma, fms, fnms};

// ---------------------------------------------------------------------------------------------
// The vector unit

/// One vector register, lanes x y z w as the guest holds them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V4(pub [f32; 4]);

/// Denormals in and out of the vector unit are zero (flush mode on for every vector float op).
#[inline]
fn ftz(x: f32) -> f32 {
    if x != 0.0 && x.is_subnormal() { if x.is_sign_negative() { -0.0 } else { 0.0 } } else { x }
}

impl V4 {
    pub const ZERO: V4 = V4([0.0; 4]);
    #[inline]
    pub fn splat(x: f32) -> V4 {
        V4([x; 4])
    }
    #[inline]
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> V4 {
        V4([x, y, z, w])
    }
    #[inline]
    fn map2(self, b: V4, f: impl Fn(f32, f32) -> f32) -> V4 {
        let (a, b) = (self.0, b.0);
        V4([0, 1, 2, 3].map(|i| ftz(f(ftz(a[i]), ftz(b[i])))))
    }
    #[inline]
    pub fn add(self, b: V4) -> V4 {
        self.map2(b, |x, y| x + y)
    }
    #[inline]
    pub fn sub(self, b: V4) -> V4 {
        self.map2(b, |x, y| x - y)
    }
    #[inline]
    pub fn mul(self, b: V4) -> V4 {
        self.map2(b, |x, y| x * y)
    }
    /// vmaddfp: self * b + c, two roundings.
    #[inline]
    pub fn madd(self, b: V4, c: V4) -> V4 {
        self.mul(b).add(c)
    }
    /// vnmsubfp: -(self * b - c).
    #[inline]
    pub fn nmsub(self, b: V4, c: V4) -> V4 {
        self.mul(b).sub(c).neg()
    }
    /// The sign bits flipped (the xor the recompiled code does).
    #[inline]
    pub fn neg(self) -> V4 {
        V4(self.0.map(|x| f32::from_bits(x.to_bits() ^ 0x8000_0000)))
    }
    /// vrefp: 1 / x.
    #[inline]
    pub fn recip(self) -> V4 {
        V4::splat(1.0).map2(self, |a, b| a / b)
    }
    /// vrsqrtefp: 1 / sqrt(x).
    #[inline]
    pub fn rsqrte(self) -> V4 {
        V4(self.0.map(|x| ftz(1.0 / ftz(ftz(x).sqrt()))))
    }
    /// vmsum3fp: the three-lane dot product in every lane.
    #[inline]
    pub fn dot3(self, b: V4) -> V4 {
        let p = self.mul(b).0;
        V4::splat(ftz(ftz(0.0 + p[2]) + ftz(p[1] + p[0])))
    }
    /// vmsum4fp
    #[inline]
    pub fn dot4(self, b: V4) -> V4 {
        let p = self.mul(b).0;
        V4::splat(ftz(ftz(p[3] + p[2]) + ftz(p[1] + p[0])))
    }
    #[inline]
    pub fn x(self) -> f32 {
        self.0[0]
    }
    #[inline]
    pub fn lane(self, i: usize) -> V4 {
        V4::splat(self.0[i])
    }
    /// vpermwi 99: (y, z, x, w)
    #[inline]
    pub fn yzx(self) -> V4 {
        let a = self.0;
        V4([a[1], a[2], a[0], a[3]])
    }
    /// vpermwi 135: (z, x, y, w)
    #[inline]
    pub fn zxy(self) -> V4 {
        let a = self.0;
        V4([a[2], a[0], a[1], a[3]])
    }
    /// vmrghw: (a.x, b.x, a.y, b.y)
    #[inline]
    pub fn mergeh(self, b: V4) -> V4 {
        V4([self.0[0], b.0[0], self.0[1], b.0[1]])
    }
    /// vmrglw: (a.z, b.z, a.w, b.w)
    #[inline]
    pub fn mergel(self, b: V4) -> V4 {
        V4([self.0[2], b.0[2], self.0[3], b.0[3]])
    }
    #[inline]
    pub fn bits(self) -> [u32; 4] {
        self.0.map(f32::to_bits)
    }
    #[inline]
    pub fn from_bits(b: [u32; 4]) -> V4 {
        V4(b.map(f32::from_bits))
    }
    /// vcmpgtfp: all ones where self > b.
    #[inline]
    pub fn gt(self, b: V4) -> [bool; 4] {
        [0, 1, 2, 3].map(|i| ftz(self.0[i]) > ftz(b.0[i]))
    }
    /// vcmpeqfp: all ones where self == b (denormals as zero).
    #[inline]
    pub fn eq(self, b: V4) -> [bool; 4] {
        [0, 1, 2, 3].map(|i| ftz(self.0[i]) == ftz(b.0[i]))
    }
    /// vcmpgefp
    #[inline]
    pub fn ge(self, b: V4) -> [bool; 4] {
        [0, 1, 2, 3].map(|i| ftz(self.0[i]) >= ftz(b.0[i]))
    }
    /// vpermwi: lane i takes lane (imm >> (6 - 2 i)) & 3.
    #[inline]
    pub fn permwi(self, imm: u32) -> V4 {
        V4([0, 1, 2, 3].map(|i| self.0[((imm >> (6 - 2 * i)) & 3) as usize]))
    }
    /// The lanes set in `mask` (x = 8, y = 4, z = 2, w = 1) taken from a splat `b` (vrlimi of a splat).
    #[inline]
    pub fn with_lanes(self, mask: u32, b: f32) -> V4 {
        V4([0, 1, 2, 3].map(|i| if mask & (8 >> i) != 0 { b } else { self.0[i] }))
    }
    /// vsel: b where the mask is set, else self.
    #[inline]
    pub fn sel(self, b: V4, m: [bool; 4]) -> V4 {
        V4([0, 1, 2, 3].map(|i| if m[i] { b.0[i] } else { self.0[i] }))
    }
}

/// a x b as the code forms it: -(a.zxy b.yzx - a.yzx b.zxy).
#[inline]
fn cross(a: V4, b: V4) -> V4 {
    a.zxy().nmsub(b.yzx(), a.yzx().mul(b.zxy()))
}

/// c + a x b: -(a.zxy b.yzx - (a.yzx b.zxy + c)).
#[inline]
fn cross_add(a: V4, b: V4, c: V4) -> V4 {
    a.zxy().nmsub(b.yzx(), a.yzx().madd(b.zxy(), c))
}

/// Three dot products as the code puts them in one register: (a, b, c, b) (two merges of splats).
#[inline]
fn three(a: V4, b: V4, c: V4) -> V4 {
    a.mergeh(c).mergeh(b)
}

/// Rows times a vector, each row's dot product: (r0.x, r1.x, r2.x, r1.x).
#[inline]
fn rows_dot(r: &[V4], x: V4) -> V4 {
    three(r[0].dot3(x), r[1].dot3(x), r[2].dot3(x))
}

/// The columns of three rows, as the merges with zero form them; lane w is 0.
#[inline]
fn columns(r: &[V4]) -> [V4; 3] {
    let (h02, h1) = (r[0].mergeh(r[2]), r[1].mergeh(V4::ZERO));
    let (l02, l1) = (r[0].mergel(r[2]), r[1].mergel(V4::ZERO));
    [h02.mergeh(h1), h02.mergel(h1), l02.mergeh(l1)]
}

/// The transpose of three rows times a vector.
#[inline]
fn cols_dot(r: &[V4], x: V4) -> V4 {
    rows_dot(&columns(r), x)
}

/// A symmetric 6 x 6 as the body keeps it, three blocks of three rows (m0, m1, m2), applied to
/// the pair (x, y): (m1^T x + m0 y, m1 y + m2 x). With the spatial inertia (A, B, C) and the
/// motion (w, v_O) it gives the force (linear, angular); with an inverse (P, Q, R) and an
/// impulse (linear, angular) the change of motion (w, v_O).
#[inline]
fn block_mul(m: &[V4; 9], x: V4, y: V4) -> (V4, V4) {
    (cols_dot(&m[3..6], x).add(rows_dot(&m[0..3], y)), rows_dot(&m[3..6], y).add(rows_dot(&m[6..9], x)))
}

// ---------------------------------------------------------------------------------------------
// Guest memory

fn rv(b: &[u8], o: usize) -> V4 {
    V4([0, 1, 2, 3].map(|i| f32::from_bits(u32::from_be_bytes(b[o + 4 * i..o + 4 * i + 4].try_into().unwrap()))))
}
fn rvs<const N: usize>(b: &[u8], o: usize) -> [V4; N] {
    std::array::from_fn(|i| rv(b, o + 16 * i))
}
fn ru(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}
fn rf(b: &[u8], o: usize) -> f32 {
    f32::from_bits(ru(b, o))
}
fn wv(b: &mut [u8], o: usize, v: V4) {
    for i in 0..4 {
        b[o + 4 * i..o + 4 * i + 4].copy_from_slice(&v.0[i].to_bits().to_be_bytes());
    }
}
fn wvs(b: &mut [u8], o: usize, v: &[V4]) {
    for (i, x) in v.iter().enumerate() {
        wv(b, o + 16 * i, *x);
    }
}
fn wu(b: &mut [u8], o: usize, x: u32) {
    b[o..o + 4].copy_from_slice(&x.to_be_bytes());
}
fn wf(b: &mut [u8], o: usize, x: f32) {
    wu(b, o, x.to_bits());
}

// ---------------------------------------------------------------------------------------------
// State

pub const LINK_SIZE: usize = 1008;
pub const BODY_SIZE: usize = 528;
pub const COLLIDER_SIZE: usize = 752;
pub const HINGE_SIZE: usize = 784;
pub const BALL_SIZE: usize = 1408;

/// One link (1008 B). Positions are relative to the collider's place.
#[derive(Clone, Debug)]
pub struct Link {
    /// +0 .. +127: the solver's scratch (pending pairs of the pivot propagation) and words not read here
    pub head: [V4; 8],
    /// +128: the spatial inertia about the common origin, blocks (A, B, C) (F-53726)
    pub inertia: [V4; 9],
    /// +272: the articulated inertia of the whole tree seen from this link
    pub artic: [V4; 9],
    /// +416: orientation rows (world)
    pub rows: [V4; 3],
    /// +464: position
    pub pos: V4,
    /// +480: mass in lanes x y z
    pub mass: V4,
    /// +496: principal moments
    pub moments: V4,
    /// +512: the rest frame (rows, position) and +576 / +592 rest velocities
    pub rest: [V4; 6],
    /// +608 linear velocity at the centre, +624 angular velocity
    pub vel: V4,
    pub ang: V4,
    /// +640 / +656: the motion vector (w, v_O)
    pub motion: [V4; 2],
    /// +672 force, +688 torque
    pub force: V4,
    pub torque: V4,
    /// +704 linear impulse, +720 angular impulse
    pub impulse: V4,
    pub ang_impulse: V4,
    /// +736: the inverse of the articulated inertia, blocks (P, Q, R)
    pub inv: [V4; 9],
    /// +880 / +896: the articulated bias force
    pub bias: [V4; 2],
    /// +912 / +928: the change of motion being propagated
    pub dmotion: [V4; 2],
    /// +944 / +960: the real motion kept over the push solve
    pub saved: [V4; 2],
    /// +976 / +992: the predicted change of motion of the pending forces
    pub predicted: [V4; 2],
}

impl Link {
    pub fn from_guest(b: &[u8]) -> Link {
        Link {
            head: rvs(b, 0),
            inertia: rvs(b, 128),
            artic: rvs(b, 272),
            rows: rvs(b, 416),
            pos: rv(b, 464),
            mass: rv(b, 480),
            moments: rv(b, 496),
            rest: rvs(b, 512),
            vel: rv(b, 608),
            ang: rv(b, 624),
            motion: rvs(b, 640),
            force: rv(b, 672),
            torque: rv(b, 688),
            impulse: rv(b, 704),
            ang_impulse: rv(b, 720),
            inv: rvs(b, 736),
            bias: rvs(b, 880),
            dmotion: rvs(b, 912),
            saved: rvs(b, 944),
            predicted: rvs(b, 976),
        }
    }
    pub fn to_guest(&self) -> Vec<u8> {
        let mut b = vec![0u8; LINK_SIZE];
        wvs(&mut b, 0, &self.head);
        wvs(&mut b, 128, &self.inertia);
        wvs(&mut b, 272, &self.artic);
        wvs(&mut b, 416, &self.rows);
        wv(&mut b, 464, self.pos);
        wv(&mut b, 480, self.mass);
        wv(&mut b, 496, self.moments);
        wvs(&mut b, 512, &self.rest);
        wv(&mut b, 608, self.vel);
        wv(&mut b, 624, self.ang);
        wvs(&mut b, 640, &self.motion);
        wv(&mut b, 672, self.force);
        wv(&mut b, 688, self.torque);
        wv(&mut b, 704, self.impulse);
        wv(&mut b, 720, self.ang_impulse);
        wvs(&mut b, 736, &self.inv);
        wvs(&mut b, 880, &self.bias);
        wvs(&mut b, 912, &self.dmotion);
        wvs(&mut b, 944, &self.saved);
        wvs(&mut b, 976, &self.predicted);
        b
    }
    /// v_O = v + c x w as every pass refreshes it: +640 = w, +656 = v + c x w.
    fn refresh_motion(&mut self) {
        self.motion[0] = self.ang;
        self.motion[1] = cross_add(self.pos, self.ang, self.vel);
    }
}

/// The hinge's own fields (type 0, 784 B).
#[derive(Clone, Debug, Default)]
pub struct Hinge {
    /// +608 / +612 hard limits, +616 / +620 inner limits
    pub hard: [f32; 2],
    pub inner: [f32; 2],
    /// +624 angle, +628 rate
    pub angle: f32,
    pub rate: f32,
    /// +632: +1 upper, -1 lower, 0 free (written by the solver's limit test)
    pub state: i32,
    /// +640 / +656: the spatial axis S = (s, p x s)
    pub axis: [V4; 2],
    /// +672 position gain, +676 velocity gain, +680 target angle, +684 target rate, +688 max, +692 min
    pub drive: [f32; 6],
    /// +704 / +720: U = IA_child S; +736 / +752: U' = I_rest S
    pub u: [V4; 2],
    pub u_rest: [V4; 2],
    /// +768 d = S . U, +772 (1 - k) / d, +776 (1 - k) / ((1 - k) d' + k d)
    pub d: f32,
    pub inv_d: f32,
    pub inv_d_rest: f32,
}

/// The swing-twist joint's own fields (type 1, 1408 B; ragdoll_swing_twist.md section 2).
#[derive(Clone, Debug, Default)]
pub struct Ball {
    /// +608 sA hard, +612 sA soft, +616 sB hard, +620 sB soft, +624 twist hard, +628 twist soft
    pub limits: [f32; 6],
    /// +632 elliptic, +633 twist limited, +634 swing active, +635 twist active
    pub flags: [u8; 4],
    /// +636 / +640 swing limit along n (hard / soft), +644 / +648 twist limit (hard / soft)
    pub swing_limit: [f32; 2],
    pub twist_limit: [f32; 2],
    /// +652 swing rate, +656 twist rate, +660 swing angle, +664 twist angle
    pub swing_rate: f32,
    pub twist_rate: f32,
    pub theta: f32,
    pub twist: f32,
    /// +672 position gains, +688 velocity gains, +704 targets, +720 target rates, +736 max, +752 min
    pub gains: [V4; 6],
    /// +768 swing axis n, +784 m = unit(aZ + bZ), +800 e2
    pub n: V4,
    pub m: V4,
    pub e2: V4,
    /// +816: 2 / |aZ + bZ|
    pub s: f32,
    /// +832: A's joint frame in the world, +880 A's position
    pub w: [V4; 3],
    pub ca: V4,
    /// +896: the joint point
    pub p: V4,
    /// +912 rest inertia about p, +1056 child inertia about p
    pub rest_p: [V4; 9],
    pub child_p: [V4; 9],
    /// +1200 G', +1248 G, +1296 H', +1344 H
    pub g_rest: [V4; 3],
    pub g: [V4; 3],
    pub h_rest: [V4; 3],
    pub h: [V4; 3],
    /// +1392 the regularisation of the rotational block
    pub reg: f32,
}

#[derive(Clone, Debug)]
pub enum JointKind {
    Hinge(Hinge),
    Ball(Ball),
    /// type 2 (slider) or another: kept as it is (the rider has none)
    Other,
}

/// One joint: child link `b` hangs on parent link `a`.
#[derive(Clone, Debug)]
pub struct Joint {
    /// +0 type
    pub kind_id: u32,
    /// +4 softness k (0 free .. 1 rigid)
    pub k: f32,
    /// +8 / +12 parent and child link (indices)
    pub a: usize,
    pub b: usize,
    /// +16: the rest of the tree seen from A (after the forward pass)
    pub rest_ia: [V4; 9],
    /// +160: B's articulated inertia at the backward pass
    pub child_ia: [V4; 9],
    /// +304 child -> parent, +336 parent -> child propagation, +368 bias, +400 velocity product
    pub up: [V4; 2],
    pub down: [V4; 2],
    pub bias: [V4; 2],
    pub cvel: [V4; 2],
    /// +432 joint frame in A (rows), +496 anchor in A, +512 anchor in B, +544 frame in B
    pub frame_a: [V4; 3],
    pub anchor_a: V4,
    pub anchor_b: V4,
    pub frame_b: [V4; 3],
    /// +528 drive mode (0 none, 1 velocity, 2 position, 3 both)
    pub mode: u32,
    /// +532 byte: the child is in contact
    pub contact: u8,
    pub kind: JointKind,
    /// the block as read (words not held above)
    pub raw: Vec<u8>,
}

impl Joint {
    pub fn from_guest(b: &[u8], link_of: impl Fn(u32) -> usize) -> Joint {
        let kind_id = ru(b, 0);
        let kind = match kind_id {
            0 => JointKind::Hinge(Hinge {
                hard: [rf(b, 608), rf(b, 612)],
                inner: [rf(b, 616), rf(b, 620)],
                angle: rf(b, 624),
                rate: rf(b, 628),
                state: ru(b, 632) as i32,
                axis: rvs(b, 640),
                drive: std::array::from_fn(|i| rf(b, 672 + 4 * i)),
                u: rvs(b, 704),
                u_rest: rvs(b, 736),
                d: rf(b, 768),
                inv_d: rf(b, 772),
                inv_d_rest: rf(b, 776),
            }),
            1 => JointKind::Ball(Ball {
                limits: std::array::from_fn(|i| rf(b, 608 + 4 * i)),
                flags: [b[632], b[633], b[634], b[635]],
                swing_limit: [rf(b, 636), rf(b, 640)],
                twist_limit: [rf(b, 644), rf(b, 648)],
                swing_rate: rf(b, 652),
                twist_rate: rf(b, 656),
                theta: rf(b, 660),
                twist: rf(b, 664),
                gains: rvs(b, 672),
                n: rv(b, 768),
                m: rv(b, 784),
                e2: rv(b, 800),
                s: rf(b, 816),
                w: rvs(b, 832),
                ca: rv(b, 880),
                p: rv(b, 896),
                rest_p: rvs(b, 912),
                child_p: rvs(b, 1056),
                g_rest: rvs(b, 1200),
                g: rvs(b, 1248),
                h_rest: rvs(b, 1296),
                h: rvs(b, 1344),
                reg: rf(b, 1392),
            }),
            _ => JointKind::Other,
        };
        Joint {
            kind_id,
            k: rf(b, 4),
            a: link_of(ru(b, 8)),
            b: link_of(ru(b, 12)),
            rest_ia: rvs(b, 16),
            child_ia: rvs(b, 160),
            up: rvs(b, 304),
            down: rvs(b, 336),
            bias: rvs(b, 368),
            cvel: rvs(b, 400),
            frame_a: rvs(b, 432),
            anchor_a: rv(b, 496),
            anchor_b: rv(b, 512),
            frame_b: rvs(b, 544),
            mode: ru(b, 528),
            contact: b[532],
            kind,
            raw: b.to_vec(),
        }
    }
    pub fn to_guest(&self, link_at: impl Fn(usize) -> u32) -> Vec<u8> {
        let mut b = self.raw.clone();
        wu(&mut b, 0, self.kind_id);
        wf(&mut b, 4, self.k);
        wu(&mut b, 8, link_at(self.a));
        wu(&mut b, 12, link_at(self.b));
        wvs(&mut b, 16, &self.rest_ia);
        wvs(&mut b, 160, &self.child_ia);
        wvs(&mut b, 304, &self.up);
        wvs(&mut b, 336, &self.down);
        wvs(&mut b, 368, &self.bias);
        wvs(&mut b, 400, &self.cvel);
        wvs(&mut b, 432, &self.frame_a);
        wv(&mut b, 496, self.anchor_a);
        wv(&mut b, 512, self.anchor_b);
        wu(&mut b, 528, self.mode);
        b[532] = self.contact;
        wvs(&mut b, 544, &self.frame_b);
        match &self.kind {
            JointKind::Hinge(h) => {
                wf(&mut b, 608, h.hard[0]);
                wf(&mut b, 612, h.hard[1]);
                wf(&mut b, 616, h.inner[0]);
                wf(&mut b, 620, h.inner[1]);
                wf(&mut b, 624, h.angle);
                wf(&mut b, 628, h.rate);
                wu(&mut b, 632, h.state as u32);
                wvs(&mut b, 640, &h.axis);
                for (i, x) in h.drive.iter().enumerate() {
                    wf(&mut b, 672 + 4 * i, *x);
                }
                wvs(&mut b, 704, &h.u);
                wvs(&mut b, 736, &h.u_rest);
                wf(&mut b, 768, h.d);
                wf(&mut b, 772, h.inv_d);
                wf(&mut b, 776, h.inv_d_rest);
            }
            JointKind::Ball(j) => {
                for (i, x) in j.limits.iter().enumerate() {
                    wf(&mut b, 608 + 4 * i, *x);
                }
                b[632..636].copy_from_slice(&j.flags);
                wf(&mut b, 636, j.swing_limit[0]);
                wf(&mut b, 640, j.swing_limit[1]);
                wf(&mut b, 644, j.twist_limit[0]);
                wf(&mut b, 648, j.twist_limit[1]);
                wf(&mut b, 652, j.swing_rate);
                wf(&mut b, 656, j.twist_rate);
                wf(&mut b, 660, j.theta);
                wf(&mut b, 664, j.twist);
                wvs(&mut b, 672, &j.gains);
                wv(&mut b, 768, j.n);
                wv(&mut b, 784, j.m);
                wv(&mut b, 800, j.e2);
                wf(&mut b, 816, j.s);
                wvs(&mut b, 832, &j.w);
                wv(&mut b, 880, j.ca);
                wv(&mut b, 896, j.p);
                wvs(&mut b, 912, &j.rest_p);
                wvs(&mut b, 1056, &j.child_p);
                wvs(&mut b, 1200, &j.g_rest);
                wvs(&mut b, 1248, &j.g);
                wvs(&mut b, 1296, &j.h_rest);
                wvs(&mut b, 1344, &j.h);
                wf(&mut b, 1392, j.reg);
            }
            JointKind::Other => {}
        }
        b
    }
}

/// The articulated body (528 B) and its collider (752 B), with the links and joints they point at.
#[derive(Clone, Debug)]
pub struct Articulated {
    pub links: Vec<Link>,
    pub joints: Vec<Joint>,
    /// body +252: parent link of each link (link 0's entry unused)
    pub parent: Vec<usize>,
    /// body +516 gravity factor, +520 angular and +524 linear damping per second
    pub gravity_factor: f32,
    pub damp_ang: f32,
    pub damp_lin: f32,
    /// collider +192 mass, +200 / +204 max speed / max angular speed
    pub mass: f32,
    pub max_speed: f32,
    pub max_ang_speed: f32,
    /// collider +160: the reciprocal moments (read by the momentum mirror)
    pub inv_moments: V4,
    /// collider +208: rows (transposed root rows), +256 place
    pub rows: [V4; 3],
    pub place: V4,
    /// collider +272 / +288 / +304 / +320: velocity, angular velocity, momentum, angular momentum
    pub mirror: [V4; 4],
    /// collider +432: the last push's displacement
    pub push: V4,
    /// collider +664: ticks since the links were last orthonormalised
    pub ortho_count: u32,
    /// the guest blocks as read, and the guest addresses
    pub raw_body: Vec<u8>,
    pub raw_collider: Vec<u8>,
    pub link_addr: Vec<u32>,
}

impl Articulated {
    /// From the guest blocks: the collider, the body, the links (with their addresses, which the
    /// joints point at) and the joints.
    pub fn from_guest(collider: &[u8], body: &[u8], links: &[(u32, &[u8])], joints: &[&[u8]]) -> Articulated {
        let n = ru(body, 0) as usize;
        assert_eq!(n, links.len());
        let link_addr: Vec<u32> = links.iter().map(|l| l.0).collect();
        for i in 0..n {
            assert_eq!(ru(body, 4 + 4 * i), link_addr[i]);
        }
        let link_of = |a: u32| link_addr.iter().position(|x| *x == a).expect("joint link not in the body");
        let joints: Vec<Joint> = joints.iter().map(|b| Joint::from_guest(b, link_of)).collect();
        Articulated {
            links: links.iter().map(|(_, b)| Link::from_guest(b)).collect(),
            joints,
            parent: (0..n).map(|i| if i == 0 { 0 } else { ru(body, 252 + 4 * i) as usize }).collect(),
            gravity_factor: rf(body, 516),
            damp_ang: rf(body, 520),
            damp_lin: rf(body, 524),
            mass: rf(collider, 192),
            max_speed: rf(collider, 200),
            max_ang_speed: rf(collider, 204),
            inv_moments: rv(collider, 160),
            rows: rvs(collider, 208),
            place: rv(collider, 256),
            mirror: rvs(collider, 272),
            push: rv(collider, 432),
            ortho_count: ru(collider, 664),
            raw_body: body.to_vec(),
            raw_collider: collider.to_vec(),
            link_addr,
        }
    }
    pub fn body_to_guest(&self) -> Vec<u8> {
        let mut b = self.raw_body.clone();
        wf(&mut b, 516, self.gravity_factor);
        wf(&mut b, 520, self.damp_ang);
        wf(&mut b, 524, self.damp_lin);
        b
    }
    pub fn collider_to_guest(&self) -> Vec<u8> {
        let mut b = self.raw_collider.clone();
        wv(&mut b, 160, self.inv_moments);
        wf(&mut b, 192, self.mass);
        wf(&mut b, 200, self.max_speed);
        wf(&mut b, 204, self.max_ang_speed);
        wvs(&mut b, 208, &self.rows);
        wv(&mut b, 256, self.place);
        wvs(&mut b, 272, &self.mirror);
        wv(&mut b, 432, self.push);
        wu(&mut b, 664, self.ortho_count);
        b
    }
    pub fn joint_to_guest(&self, k: usize) -> Vec<u8> {
        self.joints[k].to_guest(|i| self.link_addr[i])
    }
}

// ---------------------------------------------------------------------------------------------
// The joints' maps between a child's and a parent's motion or force

const ONE: V4 = V4([1.0; 4]);
const HALF: V4 = V4([0.5; 4]);

/// The hinge's rank-one maps: t = (a0 . x1 + a1 . x0) s, out = base - d t. Up (F-45440):
/// a = U', s = +776, d = S; down (F-71908): a = U, s = +772, d = S; the bias backward
/// (F-14682): a = S, s = +772, d = U; forward (F-30264): a = S, s = +776, d = U', base = out + x;
/// down added into out (F-23856): base = out + x.
fn hinge_map(a: &[V4; 2], s: f32, d: &[V4; 2], x: &[V4; 2], base: [V4; 2]) -> [V4; 2] {
    let t = a[0].dot3(x[1]).add(a[1].dot3(x[0])).mul(V4::splat(s));
    let nt = V4::ZERO.sub(t);
    [d[0].madd(nt, base[0]), d[1].madd(nt, base[1])]
}

/// The swing-twist joint's map of a force (f, tau) about its point p (F-44719 with G, H;
/// F-50524 with G', H'): tp = tau + f x p, out = (f - G^T tp, (tp - H^T tp) + p x out0).
fn ball_force_map(p: V4, g: &[V4; 3], h: &[V4; 3], x: &[V4; 2]) -> [V4; 2] {
    let tp = cross_add(x[0], p, x[1]);
    let out0 = x[0].sub(cols_dot(g, tp));
    let r = tp.sub(cols_dot(h, tp));
    [out0, cross_add(p, out0, r)]
}

/// The swing-twist joint's map of a motion (w, v) (down F-91578 with G, H; up F-75508 with
/// G', H'): u = v + w x p, out = (w - (G u + H w), u + p x out0).
fn ball_motion_map(p: V4, g: &[V4; 3], h: &[V4; 3], x: &[V4; 2]) -> [V4; 2] {
    let hx = rows_dot(h, x[0]);
    let u = cross_add(x[0], p, x[1]);
    let out0 = x[0].sub(rows_dot(g, u).add(hx));
    [out0, cross_add(p, out0, u)]
}

impl Joint {
    /// A child's change of motion taken up to the parent (F-45440 / F-75508).
    fn motion_up(&self, x: &[V4; 2]) -> Option<[V4; 2]> {
        match &self.kind {
            JointKind::Hinge(h) => Some(hinge_map(&h.u_rest, h.inv_d_rest, &h.axis, x, *x)),
            JointKind::Ball(b) => Some(ball_motion_map(b.p, &b.g_rest, &b.h_rest, x)),
            JointKind::Other => None,
        }
    }
    /// The parent's change of motion taken down to the child (F-71908 / F-91578).
    fn motion_down(&self, x: &[V4; 2]) -> Option<[V4; 2]> {
        match &self.kind {
            JointKind::Hinge(h) => Some(hinge_map(&h.u, h.inv_d, &h.axis, x, *x)),
            JointKind::Ball(b) => Some(ball_motion_map(b.p, &b.g, &b.h, x)),
            JointKind::Other => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The velocity update (slot 10)

/// |x| no more than `max`: x / |x| max where |x|^2 > max^2 (F-39990's select, rsqrt and one
/// Newton step).
fn clamp_length(x: V4, max: f32) -> V4 {
    let l2 = x.dot3(x);
    let y = l2.rsqrte();
    let unit = x.mul(l2.nmsub(y.mul(y), ONE).madd(HALF.mul(y), y));
    let below = V4::ZERO.mul(V4::ZERO).gt(l2);
    let above = l2.gt(V4::splat(max).mul(V4::splat(max)));
    x.sel(V4::ZERO.mul(unit), below).sel(V4::splat(max).mul(unit), above)
}

/// R diag(inv) R^T of the collider's rows (F-? 8259D348 in the decode; the plain body's world
/// inverse inertia): row i = sum_k (col_i inv)_k R_k.
fn world_rows(rows: &[V4; 3], inv: V4) -> [V4; 3] {
    columns(rows).map(|c| {
        let s = c.mul(inv);
        s.lane(2).madd(rows[2], s.lane(1).madd(rows[1], s.lane(0).mul(rows[0])))
    })
}

impl Articulated {
    fn n(&self) -> usize {
        self.links.len()
    }

    /// Slot 10, the velocity update (F-69485), dt as the routine takes it (v1 splatted): the
    /// bias forces, the tick's forces and the impulses of the velocity solve into the links'
    /// velocities (F-90112), the speed clamps (F-39990), the body's damping (F-19734), then the
    /// collider's plain velocity / momentum mirror from the link of bone 0.
    pub fn velocity_update(&mut self, dt: f32) {
        self.forces_to_velocities(dt);
        self.clamp_speeds(self.max_speed, self.max_ang_speed);
        self.body_damping(dt);
        // bone 0's link: the bone -> link map's entry 0, which the build sets to 0 (and an
        // invalid entry also means link 0)
        let root = &self.links[0];
        self.mirror[0] = root.vel;
        self.mirror[1] = root.ang;
        self.mirror[2] = self.mirror[0].mul(V4::splat(self.mass));
        let w = world_rows(&self.rows, self.inv_moments);
        self.mirror[3] = cols_dot(&w, self.mirror[1]);
    }

    /// F-90112: velocities made relative to the root's, the bias forces (F-60999) turned into a
    /// change of motion -M^-1 p dt, forces * dt added to the impulses, the impulses committed
    /// (F-91555), the root's velocity added back.
    fn forces_to_velocities(&mut self, dt: f32) {
        let v0 = self.links[0].vel;
        for l in &mut self.links {
            l.vel = l.vel.sub(v0);
            l.refresh_motion();
        }
        self.bias_forces();
        let dtv = V4::splat(dt);
        let ndt = V4::ZERO.sub(dtv);
        for l in &mut self.links {
            let (dw, dv) = block_mul(&l.inv, l.bias[0], l.bias[1]);
            l.dmotion = [dw.mul(ndt), dv.mul(ndt)];
            l.motion[0] = l.motion[0].add(l.dmotion[0]);
            l.motion[1] = l.motion[1].add(l.dmotion[1]);
            l.impulse = l.force.madd(dtv, l.impulse);
            l.ang_impulse = l.torque.madd(dtv, l.ang_impulse);
            l.force = V4::ZERO;
            l.torque = V4::ZERO;
        }
        self.commit_impulses();
        for l in &mut self.links {
            l.vel = v0.add(l.vel);
            l.refresh_motion();
        }
    }

    /// F-60999: the articulated bias forces, two ways. Per link the velocity-product force
    /// (w x h_lin, w x h_ang + v x h_lin) with h = I (w, v_O); backward each child's bias plus
    /// IA_C c (c = the joint's velocity product) projected through its joint and added to the
    /// parent; forward the parent's minus the rest of the tree's, projected into the child.
    fn bias_forces(&mut self) {
        for l in &mut self.links {
            let (hl, ha) = block_mul(&l.inertia, l.motion[0], l.motion[1]);
            let (w, v) = (l.motion[0], l.motion[1]);
            l.bias = [cross(w, hl), cross_add(w, ha, cross(v, hl))];
        }
        let n = self.n();
        for j in self.joints.iter_mut().take(n.saturating_sub(1)) {
            j.bias = [V4::ZERO; 2];
        }
        for k in (0..n.saturating_sub(1)).rev() {
            let (c, p) = (k + 1, self.parent[k + 1]);
            let (wp, vp) = (self.links[p].motion[0], self.links[p].motion[1]);
            let (wc, vc) = (self.links[c].motion[0], self.links[c].motion[1]);
            let cb = self.links[c].bias;
            let j = &mut self.joints[k];
            j.cvel = [cross(wp, wc), cross_add(wp, vc, cross(vp, wc))];
            let (x0, x1) = block_mul(&j.child_ia, j.cvel[0], j.cvel[1]);
            let x = [x0.add(cb[0]), x1.add(cb[1])];
            match &j.kind {
                JointKind::Hinge(h) => j.bias = hinge_map(&h.axis, h.inv_d, &h.u, &x, x),
                JointKind::Ball(b) => j.bias = ball_force_map(b.p, &b.g, &b.h, &x),
                JointKind::Other => {}
            }
            let jb = j.bias;
            let pl = &mut self.links[p];
            pl.bias = [pl.bias[0].add(jb[0]), pl.bias[1].add(jb[1])];
        }
        for k in 0..n.saturating_sub(1) {
            let (c, p) = (k + 1, self.parent[k + 1]);
            let pb = self.links[p].bias;
            let mut out = self.links[c].bias;
            let j = &self.joints[k];
            let (o0, o1) = block_mul(&j.rest_ia, j.cvel[0], j.cvel[1]);
            let x = [pb[0].sub(o0).sub(j.bias[0]), pb[1].sub(o1).sub(j.bias[1])];
            match &j.kind {
                JointKind::Hinge(h) => out = hinge_map(&h.axis, h.inv_d_rest, &h.u_rest, &x, [out[0].add(x[0]), out[1].add(x[1])]),
                JointKind::Ball(b) => {
                    let t = ball_force_map(b.p, &b.g_rest, &b.h_rest, &x);
                    out = [out[0].add(t[0]), out[1].add(t[1])];
                }
                JointKind::Other => {}
            }
            self.links[c].bias = out;
        }
    }

    /// F-39401: every link's impulse through its own inverse (M^-1 (lin, ang)), then taken up
    /// to the root and down again through the joints, so every link holds the whole tree's
    /// response; the impulses are cleared.
    fn propagate_impulses(&mut self) {
        for l in &mut self.links {
            let (dw, dv) = block_mul(&l.inv, l.impulse, l.ang_impulse);
            l.dmotion = [dw, dv];
            l.impulse = V4::ZERO;
            l.ang_impulse = V4::ZERO;
        }
        let n = self.n();
        for k in (0..n.saturating_sub(1)).rev() {
            let (c, p) = (k + 1, self.parent[k + 1]);
            let x = self.links[c].dmotion;
            if let Some(up) = self.joints[k].motion_up(&x) {
                self.joints[k].up = up;
            }
            let up = self.joints[k].up;
            let pl = &mut self.links[p];
            pl.dmotion = [pl.dmotion[0].add(up[0]), pl.dmotion[1].add(up[1])];
        }
        for k in 0..n.saturating_sub(1) {
            let (c, p) = (k + 1, self.parent[k + 1]);
            let (pd, up) = (self.links[p].dmotion, self.joints[k].up);
            let x = [pd[0].sub(up[0]), pd[1].sub(up[1])];
            if let Some(down) = self.joints[k].motion_down(&x) {
                self.joints[k].down = down;
            }
            let down = self.joints[k].down;
            let cl = &mut self.links[c];
            cl.dmotion = [cl.dmotion[0].add(down[0]), cl.dmotion[1].add(down[1])];
        }
    }

    /// F-91555: the impulses propagated (F-39401) and added to the motion; w and v_c from it
    /// (v = v_O + w x c).
    fn commit_impulses(&mut self) {
        self.propagate_impulses();
        for l in &mut self.links {
            l.motion[0] = l.motion[0].add(l.dmotion[0]);
            l.motion[1] = l.motion[1].add(l.dmotion[1]);
            l.ang = l.motion[0];
            l.vel = cross_add(l.ang, l.pos, l.motion[1]);
        }
    }

    /// F-39990: |w| and |v| of every link clamped, the motion refreshed.
    fn clamp_speeds(&mut self, max_v: f32, max_w: f32) {
        for l in &mut self.links {
            l.ang = clamp_length(l.ang, max_w);
            l.vel = clamp_length(l.vel, max_v);
            l.refresh_motion();
        }
    }

    /// F-19734: w *= 1 - c_w dt, v *= 1 - c_v dt (the body's 0.1 / 0.1 per second).
    fn body_damping(&mut self, dt: f32) {
        let dtv = V4::splat(dt);
        let fw = V4::splat(self.damp_ang).nmsub(dtv, ONE);
        let fv = V4::splat(self.damp_lin).nmsub(dtv, ONE);
        for l in &mut self.links {
            l.ang = l.ang.mul(fw);
            l.vel = l.vel.mul(fv);
            l.refresh_motion();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Matrices of the rebuild

/// s.x r0 + s.y r1 + s.z r2 as the code sums it: ((s.x r0) + s.y r1) + s.z r2, products and
/// sums rounded apart.
#[inline]
fn combo(s: V4, r: &[V4]) -> V4 {
    s.lane(2).madd(r[2], s.lane(1).madd(r[1], s.lane(0).mul(r[0])))
}

/// 1e8, what the code takes for 1 / 0 (and for 1 / sqrt of a non-positive value)
const BIG: f32 = 1.0e8;

/// F-17142: the inverse of a symmetric 3 x 3 from its lower triangle, by cofactors (scalar,
/// fused), det == 0 giving 1e8 for 1 / det; the w lanes are the input rows'.
fn inverse_sym3(r: &[V4; 3]) -> [V4; 3] {
    let (m00, m10, m11, m20, m21, m22) = (r[0].0[0], r[1].0[0], r[1].0[1], r[2].0[0], r[2].0[1], r[2].0[2]);
    let c01 = fms(m20, m21, m10 * m22);
    let c02 = fms(m10, m21, m20 * m11);
    let c00 = fms(m22, m11, m21 * m21);
    let c22 = fms(m00, m11, m10 * m10);
    let c12 = fms(m10, m20, m00 * m21);
    let c11 = fms(m00, m22, m20 * m20);
    let det = fma(m00, c00, fma(c02, m20, m10 * c01));
    let inv = if det == 0.0 { BIG } else { 1.0 / det };
    let (a, b, c) = (inv * c00, inv * c01, inv * c02);
    let (d, e, g) = (inv * c11, inv * c12, inv * c22);
    [V4::new(a, b, c, r[0].0[3]), V4::new(b, d, e, r[1].0[3]), V4::new(c, e, g, r[2].0[3])]
}

/// a >= 0 ? b : c (fsel; NaN takes c)
#[inline]
fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 { b } else { c }
}

/// F-68939: the inverse of a symmetric 3 x 3 by LDL^T with every pivot at least
/// max(1e-8, |trace| 1e-5), in place on lanes x y z (scalar, the original's order).
fn ldl_inverse(m: &mut [V4; 3]) {
    let floor_k = f32::from_bits(0x3727_c5ac);
    let tiny = f32::from_bits(0x322b_cc77);
    let tr = (m[1].0[1] + m[2].0[2]) + m[0].0[0];
    let a = (tr * floor_k).abs();
    let floor = fsel(tiny - a, tiny, a);
    let d0 = fsel(floor - m[0].0[0], floor, m[0].0[0]);
    m[0].0[0] = d0;
    let (m11, m20, m10, m22, m21) = (m[1].0[1], m[2].0[0], m[1].0[0], m[2].0[2], m[2].0[1]);
    let i0 = 1.0 / d0;
    let l10 = m10 * i0;
    let d1 = fnms(m10, l10, m11);
    let l20 = m20 * i0;
    let e22 = fnms(m20, l20, m22);
    let e21 = fnms(m10, l20, m21);
    let d1 = fsel(floor - d1, floor, d1);
    let i1 = 1.0 / d1;
    let l21 = i1 * e21;
    let q = i1 * l10;
    let d2 = fnms(l21, e21, e22);
    let u = fms(l21, l10, l20);
    let v = q * l10;
    let d2 = fsel(floor - d2, floor, d2);
    let i2 = 1.0 / d2;
    m[2].0[2] = i2;
    let x02 = u * i2;
    let y = i2 * l21;
    let z = x02 * l21;
    let n12 = -y;
    m[1].0[2] = n12;
    m[2].0[1] = n12;
    m[1].0[1] = fnms(n12, l21, i1);
    m[0].0[2] = x02;
    m[2].0[0] = x02;
    let x01 = -z - q;
    m[0].0[1] = x01;
    m[1].0[0] = x01;
    m[0].0[0] = fma(m[2].0[0], u, v) + i0;
}

/// F-11734: the inverse of the symmetric 6 x 6 (A, B, C) as (P, Q, R): X = C^-1 (F-17142),
/// Y = B^T X, S^-1 = (A - Y B)^-1 (F-68939), W = S^-1 Y, P = X (I + B W), Q = -W, R = S^-1.
fn block_inverse(m: &[V4; 9]) -> [V4; 9] {
    let (a, b) = (&m[0..3], &m[3..6]);
    let x = inverse_sym3(&[m[6], m[7], m[8]]);
    let y = columns(b).map(|c| combo(c, &x));
    let mut s: [V4; 3] = std::array::from_fn(|i| a[i].sub(combo(y[i], b)));
    ldl_inverse(&mut s);
    let w: [V4; 3] = std::array::from_fn(|i| combo(s[i], &y));
    let e = [V4::new(1.0, 0.0, 0.0, 0.0), V4::new(0.0, 1.0, 0.0, 0.0), V4::new(0.0, 0.0, 1.0, 0.0)];
    let t: [V4; 3] = std::array::from_fn(|i| combo(b[i], &w).add(e[i]));
    let p: [V4; 3] = std::array::from_fn(|i| combo(x[i], &t));
    [p[0], p[1], p[2], w[0].neg(), w[1].neg(), w[2].neg(), s[0], s[1], s[2]]
}

/// The run-time lane mask (~0, ~0, ~0, 0) the image's zero vector is set to before the game runs.
const XYZ: [u32; 4] = [!0, !0, !0, 0];
/// The unit axes with lane w 0 (image constants).
const EX: V4 = V4([1.0, 0.0, 0.0, 0.0]);
const EY: V4 = V4([0.0, 1.0, 0.0, 0.0]);
const EZ: V4 = V4([0.0, 0.0, 1.0, 0.0]);

/// F-53726: a link's spatial inertia about the common origin, (A, B, C) = (diag m, [m c]x
/// rows e_k x m c, R diag(J) R^T - [c]x B).
fn spatial_inertia(l: &Link) -> [V4; 9] {
    let mb = l.mass.bits();
    let m = V4::from_bits([0, 1, 2, 3].map(|i| mb[i] & XYZ[i]));
    let mc = l.mass.mul(l.pos);
    let b = [cross(EX, mc), cross(EY, mc), cross(EZ, mc)];
    let cx = [cross(EX, l.pos), cross(EY, l.pos), cross(EZ, l.pos)];
    let r = &l.rows;
    let c: [V4; 3] = std::array::from_fn(|i| rows_dot(r, r[i].mul(l.moments)).sub(combo(cx[i], &b)));
    [m.permwi(63), m.permwi(223), m.permwi(251), b[0], b[1], b[2], c[0], c[1], c[2]]
}

/// F-25250 / F-93538 (through F-64060 and F-80967): src - (U U^T s) by blocks, as
/// (u0.x u0 s, u0.y u0 s, u0.z u0 s, u1.x u0 s, .., u1.z u1 s).
fn minus_outer(src: &[V4; 9], u: &[V4; 2], s: f32) -> [V4; 9] {
    let (a, b) = (u[0].mul(V4::splat(s)), u[1].mul(V4::splat(s)));
    let o = [u[0].lane(0).mul(a), u[0].lane(1).mul(a), u[0].lane(2).mul(a), u[1].lane(0).mul(a), u[1].lane(1).mul(a), u[1].lane(2).mul(a), u[1].lane(0).mul(b), u[1].lane(1).mul(b), u[1].lane(2).mul(b)];
    std::array::from_fn(|i| src[i].sub(o[i]))
}

/// (1 - k) / x with x == 0 giving (1 - k) 1e8 (F-84092's reciprocal and one Newton step).
fn softened_recip(x: V4, k: f32, zero_test: bool) -> f32 {
    let r = x.recip();
    let mut e = r.madd(r.nmsub(x, ONE), r);
    if zero_test {
        e = e.sel(V4::splat(BIG), x.eq(V4::ZERO));
    }
    ONE.sub(V4::splat(k)).mul(e).x()
}

/// The swing-twist joint's inertia (A, B, C) moved to its point p: (A, Bn, C') with
/// Bn_i = (-r_i) A + B_i, C'_i = Bn_i r - (B_k . r_i)_k + C_i, r_i = e_i x p.
fn shift_to(p: V4, m: &[V4; 9]) -> [V4; 9] {
    let r = [cross(EX, p), cross(EY, p), cross(EZ, p)];
    let bn: [V4; 3] = std::array::from_fn(|i| combo(r[i].neg(), &m[0..3]).add(m[3 + i]));
    let c: [V4; 3] = std::array::from_fn(|i| combo(bn[i], &r).sub(three(m[3].dot3(r[i]), m[4].dot3(r[i]), m[5].dot3(r[i]))).add(m[6 + i]));
    [m[0], m[1], m[2], bn[0], bn[1], bn[2], c[0], c[1], c[2]]
}

/// D = C' + reg I inverted (F-17142), H = D^-1 C', G = D^-1 Bn.
fn ball_gains(s: &[V4; 9], reg: f32) -> ([V4; 3], [V4; 3]) {
    let reg = V4::splat(reg);
    let d0 = s[6].with_lanes(8, s[6].lane(0).add(reg).x());
    let c11 = s[7].lane(1).add(reg).x();
    let d1 = V4::new(s[7].0[0], c11, s[7].0[2], c11);
    let d2 = s[8].with_lanes(3, s[8].lane(2).add(reg).x());
    let di = inverse_sym3(&[d0, d1, d2]);
    (di.map(|r| combo(r, &s[3..6])), di.map(|r| combo(r, &s[6..9])))
}

/// F-63945 / F-84028 (the general branch: k != 0 or a velocity drive): the inertia projected
/// through the joint about p and shifted back: IaA = A - Bn^T G, F = Bn - C' G, E = C' - C' H,
/// IaB = (r_i IaA) + F, IaC = (IaB q - (F_k . q_i)_k) + E with q_i = e_i x (-p).
fn ball_project(p: V4, s: &[V4; 9], g: &[V4; 3], h: &[V4; 3]) -> [V4; 9] {
    let (a, bn, c) = (&s[0..3], &s[3..6], &s[6..9]);
    let cb = columns(bn);
    let ia: [V4; 3] = std::array::from_fn(|i| a[i].sub(combo(cb[i], g)));
    let f: [V4; 3] = std::array::from_fn(|i| bn[i].sub(combo(c[i], g)));
    let e: [V4; 3] = std::array::from_fn(|i| c[i].sub(combo(c[i], h)));
    let np = V4::ZERO.sub(p);
    let q = [cross(EX, np), cross(EY, np), cross(EZ, np)];
    let ib: [V4; 3] = std::array::from_fn(|i| combo(q[i].neg(), &ia).add(f[i]));
    let ic: [V4; 3] = std::array::from_fn(|i| combo(ib[i], &q).sub(three(f[0].dot3(q[i]), f[1].dot3(q[i]), f[2].dot3(q[i]))).add(e[i]));
    [ia[0], ia[1], ia[2], ib[0], ib[1], ib[2], ic[0], ic[1], ic[2]]
}

/// F-68966: the rotation (rows) taking `from` onto `to` by the shortest arc; identity when
/// |from x to|^2 < 1e-12; the rows are built as F-38496's, from the sine and cosine of the arc.
fn shortest_arc(from: V4, to: V4) -> [V4; 3] {
    let c = cross(from, to);
    let l2 = c.dot3(c);
    if V4::splat(f32::from_bits(0x2b8c_bccc)).gt(l2).iter().all(|x| *x) {
        return [EX, EY, EZ];
    }
    let y = l2.rsqrte();
    let ff = from.dot3(from);
    let tt = to.dot3(to);
    let ft = from.dot3(to);
    let fftt = ff.mul(tt);
    let y2 = fftt.rsqrte();
    let s = l2.mul(l2.nmsub(y.mul(y), ONE).madd(HALF.mul(y), y)).sel(V4::ZERO, l2.eq(V4::ZERO));
    let rs = fftt.nmsub(y2.mul(y2), ONE).madd(HALF.mul(y2), y2).sel(V4::splat(BIG), V4::ZERO.ge(fftt));
    let (sin, cos) = (s.mul(rs), ft.mul(rs));
    let r = s.recip();
    let axis = c.mul(r.madd(r.nmsub(s, ONE), r));
    let a = axis.mul(sin);
    let n = V4::ZERO.sub(a);
    let t = ONE.sub(cos);
    let k = cos.x();
    [
        axis.mul(axis.lane(0)).madd(t, V4::new(k, n.0[2], a.0[1], a.0[0])),
        axis.mul(axis.lane(1)).madd(t, V4::new(a.0[2], k, n.0[0], a.0[0])),
        axis.mul(axis.lane(2)).madd(t, V4::new(n.0[1], a.0[0], k, a.0[0])),
    ]
}

/// F-38496: the rotation by `angle` about the unit `axis` (rows; lanes x y z are what is read):
/// the vector unit's sine and cosine (solver::sin_cos), (axis axis_i)(1 - c) + the skew part.
fn turn_rows(axis: V4, angle: f32) -> [V4; 3] {
    let (s, c) = super::solver::sin_cos(angle);
    let a = axis.mul(V4::splat(s));
    let n = V4::ZERO.sub(a);
    let t = ONE.sub(V4::splat(c));
    [
        axis.mul(axis.lane(0)).madd(t, V4::new(c, a.0[2], n.0[1], a.0[0])),
        axis.mul(axis.lane(1)).madd(t, V4::new(n.0[2], c, a.0[0], a.0[0])),
        axis.mul(axis.lane(2)).madd(t, V4::new(a.0[1], n.0[0], c, a.0[0])),
    ]
}

/// The z column of a joint frame as the code gathers it: (f0.z, f1.z, f2.z, f1.z).
fn frame_z(f: &[V4; 3]) -> V4 {
    three(f[0].lane(2), f[1].lane(2), f[2].lane(2))
}

/// 1 / sqrt(l2) with one Newton step.
#[inline]
fn rsqrt_nr(l2: V4) -> V4 {
    let y = l2.rsqrte();
    l2.nmsub(y.mul(y), ONE).madd(HALF.mul(y), y)
}

// ---------------------------------------------------------------------------------------------
// Moving the links, re-seating them, rebuilding the inertias (post-push; slot 5's move)

impl Articulated {
    /// After the push solve (F-48210): the push impulses as velocities, integrated (F-54255),
    /// the real velocities back (F-60931), children re-seated (F-52649), the push's displacement
    /// kept, the collider re-centred on the root (F-69287) and the inertias rebuilt (F-97481).
    /// Not here: the instance matrix and the level's object move at the end of F-69287.
    pub fn post_push(&mut self, dt: f32) {
        self.push_integrate(dt);
        self.restore_velocities();
        self.reseat();
        self.push = self.links[0].pos;
        self.recentre();
        self.rebuild();
    }

    /// F-54255: the push impulses committed as velocities, clamped to |v| <= 17 / dt,
    /// |w| <= 1.414 / dt (1 / dt = 1e8 at dt == 0), damped, integrated.
    fn push_integrate(&mut self, dt: f32) {
        self.commit_impulses();
        let inv = if dt == 0.0 { BIG } else { 1.0 / dt };
        self.clamp_speeds(inv * 17.0, inv * f32::from_bits(0x3fb4_fdf4));
        self.body_damping(dt);
        self.integrate(dt);
    }

    /// F-45950: c += v dt; the rows turned by |w| dt about w / |w| when |w|^2 > 1e-6.
    fn integrate(&mut self, dt: f32) {
        let dtv = V4::splat(dt);
        let small = V4::splat(f32::from_bits(0x3586_37bd));
        for l in &mut self.links {
            let l2 = l.ang.dot3(l.ang);
            l.pos = l.vel.madd(dtv, l.pos);
            if !l2.gt(small).iter().all(|x| *x) {
                continue;
            }
            let a = l2.mul(rsqrt_nr(l2)).sel(V4::ZERO, l2.eq(V4::ZERO));
            let r = a.recip();
            let axis = l.ang.mul(r.madd(r.nmsub(a, ONE), r));
            let angle = V4::ZERO.sub(V4::splat(dt * a.x())).x();
            let q = turn_rows(axis, angle);
            let old = l.rows;
            l.rows = q.map(|qi| combo(qi, &old));
        }
    }

    /// F-60931: the real motion back from +944 / +960 (kept over the push solve).
    fn restore_velocities(&mut self) {
        for l in &mut self.links {
            l.motion = l.saved;
            l.ang = l.motion[0];
            l.vel = cross_add(l.ang, l.pos, l.motion[1]);
        }
    }

    /// F-52649: every child put back on its parent through its joint, parents first (hinge
    /// F-72940, swing-twist F-70063), its motion refreshed.
    fn reseat(&mut self) {
        for k in 0..self.n().saturating_sub(1) {
            let (a, b) = (self.joints[k].a, self.joints[k].b);
            let pa = self.links[a].clone();
            let j = &self.joints[k];
            let lb = &mut self.links[b];
            match &j.kind {
                JointKind::Hinge(_) => {
                    let aa = rows_dot(&pa.rows, frame_z(&j.frame_a));
                    let ab = rows_dot(&lb.rows, frame_z(&j.frame_b));
                    let q = shortest_arc(ab, aa);
                    let old = lb.rows;
                    lb.rows = q.map(|qi| combo(qi, &old));
                    let ra = rows_dot(&pa.rows, j.anchor_a);
                    let rb = rows_dot(&old, j.anchor_b);
                    let pt = ra.add(pa.pos);
                    lb.pos = pt.sub(rb);
                    let d = pt.sub(pa.pos);
                    lb.ang = aa.madd(aa.dot3(lb.ang).sub(aa.dot3(pa.ang)), pa.ang);
                    lb.vel = cross(pa.ang, d).add(pa.vel).sub(cross(lb.ang, rb));
                }
                JointKind::Ball(_) => {
                    let ra = rows_dot(&pa.rows, j.anchor_a);
                    let rb = rows_dot(&lb.rows, j.anchor_b);
                    let pt = ra.add(pa.pos);
                    lb.pos = pt.sub(rb);
                    let d = pt.sub(pa.pos);
                    lb.refresh_motion();
                    lb.vel = cross(pa.ang, d).add(pa.vel).sub(cross(lb.ang, rb));
                }
                JointKind::Other => {}
            }
            self.links[k + 1].refresh_motion();
        }
    }

    /// F-69287: the collider's rows = the root's transposed, orthonormalised; its place moved to
    /// the root's; every link shifted back by that (F-66740 / F-25736: c += d, v_O += d x w).
    fn recentre(&mut self) {
        let root = self.links[0].pos;
        let d = V4::ZERO.sub(root);
        let c = columns(&self.links[0].rows);
        let z = cross(c[0], c[1]);
        let x = cross(c[1], z);
        self.rows = [x.mul(rsqrt_nr(x.dot3(x))), c[1].mul(rsqrt_nr(c[1].dot3(c[1]))), z.mul(rsqrt_nr(z.dot3(z)))];
        self.place = self.place.add(root);
        for l in &mut self.links {
            l.pos = l.pos.add(d);
            l.motion[1] = cross_add(d, l.motion[0], l.motion[1]);
        }
    }

    /// F-97481: every link's spatial inertia (F-53726); backward each child's articulated
    /// inertia projected through its joint into the parent's; forward the rest of the tree
    /// projected into each child; every link's inverse (F-11734).
    fn rebuild(&mut self) {
        for l in &mut self.links {
            l.inertia = spatial_inertia(l);
            l.artic = l.inertia;
        }
        let n = self.n();
        for k in (0..n.saturating_sub(1)).rev() {
            let (c, p) = (k + 1, self.parent[k + 1]);
            self.joints[k].child_ia = self.links[c].artic;
            let lb = self.links[self.joints[k].b].clone();
            let j = &mut self.joints[k];
            let k_soft = j.k;
            match &mut j.kind {
                JointKind::Hinge(h) => {
                    // F-84092 then F-25250
                    let s = rows_dot(&lb.rows, frame_z(&j.frame_b));
                    let pt = rows_dot(&lb.rows, j.anchor_b).add(lb.pos);
                    h.axis = [s, cross(pt, s)];
                    let (u0, u1) = block_mul(&j.child_ia, h.axis[0], h.axis[1]);
                    h.u = [u0, u1];
                    let d = h.axis[0].dot3(u1).add(h.axis[1].dot3(u0));
                    h.d = d.x();
                    h.inv_d = softened_recip(d, k_soft, true);
                    j.rest_ia = minus_outer(&j.child_ia, &h.u, h.inv_d);
                }
                JointKind::Ball(b) => {
                    // F-79577 then F-63945
                    b.p = rows_dot(&lb.rows, j.anchor_b).add(lb.pos);
                    b.child_p = shift_to(b.p, &j.child_ia);
                    if k_soft.to_bits() == 0x3f80_0000 {
                        b.h = [V4::ZERO; 3];
                        b.g = [V4::ZERO; 3];
                    } else {
                        let c = &b.child_p;
                        let tr = c[6].lane(0).add(c[7].lane(1)).add(c[8].lane(2));
                        let third = V4::splat(f32::from_bits(0x3eaa_aaab));
                        let omk = ONE.sub(V4::splat(k_soft));
                        let r = omk.recip();
                        b.reg = third.mul(tr).mul(V4::splat(k_soft).mul(r.madd(r.nmsub(omk, ONE), r))).x();
                        let (g, h) = ball_gains(&b.child_p, b.reg);
                        b.g = g;
                        b.h = h;
                    }
                    j.rest_ia = ball_project_or_hard(b, j.mode, k_soft, &b.child_p, &b.g, &b.h);
                }
                JointKind::Other => {}
            }
            let add = self.joints[k].rest_ia;
            let pl = &mut self.links[p];
            for i in 0..9 {
                pl.artic[i] = pl.artic[i].add(add[i]);
            }
        }
        for k in 0..n.saturating_sub(1) {
            let (c, p) = (k + 1, self.parent[k + 1]);
            let pa = self.links[p].artic;
            let j = &mut self.joints[k];
            for i in 0..9 {
                j.rest_ia[i] = pa[i].sub(j.rest_ia[i]);
            }
            let k_soft = j.k;
            let out = match &mut j.kind {
                JointKind::Hinge(h) => {
                    // F-55089 then F-93538
                    let (u0, u1) = block_mul(&j.rest_ia, h.axis[0], h.axis[1]);
                    h.u_rest = [u0, u1];
                    let dr = h.axis[0].dot3(u1).add(h.axis[1].dot3(u0));
                    let kv = V4::splat(k_soft);
                    let omk = ONE.sub(kv);
                    let den = omk.madd(dr, kv.mul(V4::splat(h.d)));
                    h.inv_d_rest = softened_recip(den, k_soft, false);
                    Some(minus_outer(&j.rest_ia, &h.u_rest, h.inv_d_rest))
                }
                JointKind::Ball(b) => {
                    // F-22471 then F-84028
                    b.rest_p = shift_to(b.p, &j.rest_ia);
                    if k_soft.to_bits() == 0x3f80_0000 {
                        b.h_rest = [V4::ZERO; 3];
                        b.g_rest = [V4::ZERO; 3];
                    } else {
                        let (g, h) = ball_gains(&b.rest_p, b.reg);
                        b.g_rest = g;
                        b.h_rest = h;
                    }
                    Some(ball_project_or_hard(b, j.mode, k_soft, &b.rest_p, &b.g_rest, &b.h_rest))
                }
                JointKind::Other => None,
            };
            if let Some(out) = out {
                let cl = &mut self.links[c];
                for i in 0..9 {
                    cl.artic[i] = cl.artic[i].add(out[i]);
                }
            }
        }
        for l in &mut self.links {
            l.inv = block_inverse(&l.artic);
        }
    }
}

/// F-63945 / F-84028's test: the general projection unless k is +-0 and the drive has no
/// velocity part; that branch (its tail F-92796) never runs on the rider and is not ported.
fn ball_project_or_hard(b: &Ball, mode: u32, k: f32, s: &[V4; 9], g: &[V4; 3], h: &[V4; 3]) -> [V4; 9] {
    debug_assert!(k.to_bits() & 0x7fff_ffff != 0 || mode == 1 || mode == 3, "the hard projection (F-92796) is not ported");
    ball_project(b.p, s, g, h)
}

// ---------------------------------------------------------------------------------------------
// Tests: the recorded states (ro.0, six ticks) through each routine, word for word

#[cfg(test)]
mod tests {
    use super::*;

    struct State {
        collider: Vec<u8>,
        body: Vec<u8>,
        links: Vec<(u32, Vec<u8>)>,
        joints: Vec<Vec<u8>>,
    }
    struct Case {
        op: u8,
        tick: u32,
        f1: f32,
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
    /// testdata/ragdoll_ro0.bin (work/ragdoll/fixtures/export.py).
    fn cases() -> Vec<Case> {
        let d: &[u8] = include_bytes!("testdata/ragdoll_ro0.bin");
        assert_eq!(&d[..4], b"RGF1");
        let mut o = 4;
        let n = le(d, &mut o);
        (0..n)
            .map(|_| {
                let op = d[o];
                o += 4;
                let tick = le(d, &mut o);
                let f1 = f32::from_bits(le(d, &mut o));
                let v1 = f32::from_bits(le(d, &mut o));
                o += 12;
                let inp = state(d, &mut o);
                let out = state(d, &mut o);
                Case { op, tick, f1, v1, inp, out }
            })
            .collect()
    }

    fn load(s: &State) -> Articulated {
        let links: Vec<(u32, &[u8])> = s.links.iter().map(|(a, b)| (*a, b.as_slice())).collect();
        let joints: Vec<&[u8]> = s.joints.iter().map(|b| b.as_slice()).collect();
        Articulated::from_guest(&s.collider, &s.body, &links, &joints)
    }

    /// Words that differ (both NaN not counted, counted apart), with where.
    fn compare(name: &str, got: &[u8], want: &[u8], bad: &mut Vec<String>, nan: &mut usize) {
        for o in (0..want.len()).step_by(4) {
            let (g, w) = (ru(got, o), ru(want, o));
            if g == w {
                continue;
            }
            if f32::from_bits(g).is_nan() && f32::from_bits(w).is_nan() {
                *nan += 1;
            } else {
                bad.push(format!("{name}+{o}: {:08x} want {:08x} ({} / {})", g, w, f32::from_bits(g), f32::from_bits(w)));
            }
        }
    }

    fn check(a: &Articulated, want: &State, bad: &mut Vec<String>, nan: &mut usize) {
        compare("collider", &a.collider_to_guest(), &want.collider, bad, nan);
        compare("body", &a.body_to_guest(), &want.body, bad, nan);
        for i in 0..a.links.len() {
            compare(&format!("link{i}"), &a.links[i].to_guest(), &want.links[i].1, bad, nan);
        }
        for k in 0..a.joints.len() {
            compare(&format!("joint{k}"), &a.joint_to_guest(k), &want.joints[k], bad, nan);
        }
    }

    #[test]
    fn the_state_round_trips() {
        for c in cases() {
            for s in [&c.inp, &c.out] {
                let a = load(s);
                let (mut bad, mut nan) = (Vec::new(), 0);
                check(&a, s, &mut bad, &mut nan);
                assert!(bad.is_empty() && nan == 0, "{:?}", &bad[..bad.len().min(5)]);
            }
        }
    }

    fn run(op: u8, f: impl Fn(&mut Articulated, &Case)) {
        let (mut total, mut nan_total, mut n) = (0, 0, 0);
        let mut report = Vec::new();
        for c in cases().iter().filter(|c| c.op == op) {
            let mut a = load(&c.inp);
            f(&mut a, c);
            let (mut bad, mut nan) = (Vec::new(), 0);
            check(&a, &c.out, &mut bad, &mut nan);
            n += 1;
            total += bad.len();
            nan_total += nan;
            if !bad.is_empty() {
                report.push(format!("tick {}: {} words, first {:#?}", c.tick, bad.len(), &bad[..bad.len().min(12)]));
            }
        }
        eprintln!("op {}: {n} cases, {total} differing words, {nan_total} words NaN on both sides", op as char);
        assert!(n > 0);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    /// Slot 10 (F-69485) on the six recorded ticks: 0 differing words.
    #[test]
    fn velocity_update_is_the_originals() {
        run(b'v', |a, c| a.velocity_update(c.v1));
    }

    /// The post-push step (F-48210) on the six recorded ticks (dt from f1).
    #[test]
    fn post_push_is_the_originals() {
        run(b'p', |a, c| a.post_push(c.f1));
    }
}
