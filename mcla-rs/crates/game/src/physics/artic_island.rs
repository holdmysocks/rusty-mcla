//! The thrown rider's jointed body inside the island solver, operation for operation: the
//! active joint-limit degrees of freedom of the body (F-73275, run when the island's work space
//! is made), their rows (type 11 of the row list: built at the tail of F-36821, swept by the
//! kernel F-31194, handed out by the apply passes F-24076 / F-64140), the push solve's part
//! (F-49764: the rows' push bias, the velocities committed, saved and cleared), and the
//! solve-time scratch the kernels go through: every link keeps the change of motion the rows
//! have given it so far (+16 / +32) and what of it its parent has not seen yet (+48 / +64), and
//! a pivot link (body +512) at which the scratch is up to date; a query walks the pivot to the
//! link asked about, carrying the changes up and down the joints on the way (F-55744, F-31346,
//! F-81602), so the joint rows see the whole tree's response to every impulse of the sweep.
//!
//! Not here: the contact rows with an articulated side (types 7 and 13: F-38941 / F-90871,
//! F-45220 / F-33909, the point velocity F-50660, the impulse at a point F-96841, slot 22
//! F-81637, the contact apply through slot 34 F-19400), the hinge's byte +532 set by a contact
//! on its child (F-18964, an input here), sliders.
//!
//! Decode: ragdoll_solver_decode.md 4.x, ragdoll_swing_twist.md 4 and 6, and
//! work/ragdoll/island/island_rows.md. Checked word for word against islands of the recording
//! that hold joint-limit rows only (testdata/ragdoll_ro0_island.bin; tests below).

use super::*;
use crate::physics::exact::fnma;

/// 0.03, what the rows leave of a limit (0x82812BE8)
const ALLOWANCE: f32 = f32::from_bits(0x3cf5_c28f);

/// One active joint-limit degree of freedom (an entry of the collider's arrays +684 .. +708).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LimitDof {
    /// +684: the joint
    pub joint: usize,
    /// +692: the row's kind (hinge 0; swing-twist 0 swing along n, 1 across it, 2 / 3 twist)
    pub kind: u32,
    /// +700: how far past the limit (position - limit)
    pub depth: f32,
    /// +708: the rate along the row
    pub rate: f32,
}

/// A joint-limit row (type 11, 176 B in the island's row list).
#[derive(Clone, Copy, Debug)]
pub struct LimitRow {
    pub dof: LimitDof,
    /// +0: the sum of what the visits gave the bodies (each gives -step), every lane; the
    /// apply hands out lane x
    pub mult: V4,
    /// +16: the velocity asked of the row
    pub bias: V4,
    /// +128: K, the relative inverse inertia along the row (lane w is a stale word of the
    /// builder's work space, taken as K here: no reader of the rows looks at it)
    pub k: V4,
    /// +144: -1 / K
    pub inv: V4,
}

impl Articulated {
    // -----------------------------------------------------------------------------------------
    // The limit tests on the hard limits (F-73275)

    /// F-87542: the hinge against its hard limits (+608 / +612): +632 = +1 at or past the upper,
    /// -1 at or past the lower, else 0, or with byte +532 (a contact on the child) the nearer
    /// one; 1 when a row is wanted.
    fn hinge_test(&mut self, k: usize) -> u32 {
        let angle = self.hinge_angle(k);
        let contact = self.joints[k].contact;
        let h = self.hinge_mut(k);
        let (lo, hi) = (h.hard[0], h.hard[1]);
        h.state = if !(angle - hi < 0.0) {
            1
        } else if angle - lo > 0.0 {
            0
        } else {
            -1
        };
        if h.state != 0 {
            return 1;
        }
        if contact == 0 {
            return 0;
        }
        h.state = if hi - angle >= angle - lo { -1 } else { 1 };
        1
    }

    /// F-79699: the hard swing cone test (+608 / +616): theta past the cone (an ellipse when
    /// byte 632) -> +636 the limit along n, +652 the swing rate; true when active.
    fn ball_cone_hard(&mut self, k: usize) -> bool {
        let (wa, wb) = {
            let j = &self.joints[k];
            (self.links[j.a].ang, self.links[j.b].ang)
        };
        let b = self.ball_mut(k);
        let n = b.n;
        if b.flags[0] == 0 {
            if b.theta - b.limits[0] < 0.0 {
                return false;
            }
            b.swing_limit[0] = b.limits[0];
        } else {
            let w = b.w;
            let ay = fma(w[2].0[1], n.0[2], fma(w[0].0[1], n.0[0], w[1].0[1] * n.0[1]));
            let ax = fma(w[2].0[0], n.0[2], fma(w[0].0[0], n.0[0], w[1].0[0] * n.0[1]));
            let (x, y) = (ay / b.limits[0], ax / b.limits[2]);
            let q = (fma(y, y, x * x) * b.theta) * b.theta;
            if q < 1.0 {
                return false;
            }
            // frsqrte (1 / sqrt in double, to single) and two fused Newton steps
            let y0 = (1.0 / (q as f64).sqrt()) as f32;
            let y1 = fma(fnms(y0 * y0, q, 1.0), y0 * 0.5, y0);
            let y2 = fma(fnms(y1 * y1, q, 1.0), y1 * 0.5, y1);
            b.swing_limit[0] = y2 * b.theta;
        }
        b.swing_rate = n.dot3(wb).sub(n.dot3(wa)).x();
        true
    }

    /// F-16390: the hard limits' evaluation (measure, cone, twist against +624): bytes 634 /
    /// 635, +656 the twist rate, +644 the signed twist limit; the number of limit rows.
    fn ball_hard_limits(&mut self, k: usize) -> u32 {
        let (mode, wa, wb) = {
            let j = &self.joints[k];
            (j.mode, self.links[j.a].ang, self.links[j.b].ang)
        };
        let limited = self.ball(k).flags[1] != 0;
        assert!(limited || mode != 0, "the swing-only measure (F-37900) is not ported");
        self.ball_measure(k);
        let active = self.ball_cone_hard(k);
        let b = self.ball_mut(k);
        b.flags[2] = active as u8;
        let (m, s) = (b.m, b.s);
        b.twist_rate = (dot_yzx(m, wb) - dot_yzx(m, wa)) * s;
        let mut rows = if active { 2 } else { 0 };
        if !limited {
            b.flags[3] = 0;
            return rows;
        }
        let (t, l) = (b.twist, b.limits[4]);
        let on = if !(t - l < 0.0) {
            b.twist_limit[0] = l;
            true
        } else if !(l + t > 0.0) {
            b.twist_limit[0] = -l;
            true
        } else {
            false
        };
        rows += on as u32;
        b.flags[3] = on as u8;
        rows
    }

    /// F-47802: the hinge's row: (kind 0, position, rate, limit), the angle against the
    /// upper hard limit, or all negated against the lower (+632 == -1). The rate is +628 as the
    /// last writer left it (the test does not refresh it).
    fn hinge_dof(&self, k: usize) -> (u32, f32, f32, f32) {
        match &self.joints[k].kind {
            JointKind::Hinge(h) if h.state == -1 => (0, -h.angle, -h.rate, -h.hard[0]),
            JointKind::Hinge(h) => (0, h.angle, h.rate, h.hard[1]),
            _ => unreachable!(),
        }
    }

    /// F-19968: the swing-twist joint's row `i`: the twist row is i == 2 with a swing, else 0
    /// (kind 2 against +644 > 0, else kind 3 all negated); row 0 the swing along n (theta,
    /// +652, +636); row 1 across it along e2 (0, e2.wB - e2.wA, 0), e2 turned over in place
    /// where that rate is negative.
    fn ball_dof(&mut self, k: usize, i: u32) -> (u32, f32, f32, f32) {
        let (wa, wb) = {
            let j = &self.joints[k];
            (self.links[j.a].ang, self.links[j.b].ang)
        };
        let b = self.ball_mut(k);
        let twist_row = if b.flags[2] != 0 { 2 } else { 0 };
        if i == twist_row {
            return if b.twist_limit[0] > 0.0 {
                (2, b.twist, b.twist_rate, b.twist_limit[0])
            } else {
                (3, -b.twist, -b.twist_rate, -b.twist_limit[0])
            };
        }
        if i == 0 {
            return (0, b.theta, b.swing_rate, b.swing_limit[0]);
        }
        let mut r = dot_zyx(b.e2, wb) - dot_zyx(b.e2, wa);
        if r < 0.0 {
            r = -r;
            b.e2 = V4::ZERO.sub(b.e2);
        }
        (1, 0.0, r, 0.0)
    }

    /// F-73275: every joint's hard-limit test (hinge F-87542, swing-twist F-16390) and its
    /// active degrees of freedom (F-47802 / F-19968) into the collider's arrays; +680.
    pub fn limit_dofs(&mut self) {
        let mut dofs = Vec::new();
        for k in 0..self.n().saturating_sub(1) {
            let count = match self.joints[k].kind_id {
                0 => self.hinge_test(k),
                1 => self.ball_hard_limits(k),
                2 => panic!("the slider's limit test (F-88738) is not ported"),
                _ => 0,
            };
            for i in 0..count {
                let (kind, pos, rate, limit) = match self.joints[k].kind_id {
                    1 => self.ball_dof(k, i),
                    _ => self.hinge_dof(k),
                };
                dofs.push(LimitDof { joint: k, kind, depth: pos - limit, rate });
            }
        }
        self.active = dofs.len() as u32;
        self.dofs = dofs;
    }

    // -----------------------------------------------------------------------------------------
    // The rows

    /// The relative inverse inertia along a row: hinge F-58510 (along S.w), swing-twist F-51590
    /// (n, e2, m times s for the twist; 0 past kind 3); 0 for a joint of another type.
    fn row_inertia(&self, d: &LimitDof) -> f32 {
        let k = d.joint;
        match &self.joints[k].kind {
            JointKind::Hinge(h) => self.axis_inertia(k, h.axis[0]),
            JointKind::Ball(b) => match d.kind {
                0 => self.axis_inertia(k, b.n),
                1 => self.axis_inertia(k, b.e2),
                2 | 3 => self.axis_inertia(k, b.m) * b.s,
                _ => 0.0,
            },
            JointKind::Other if self.joints[k].kind_id == 2 => panic!("the slider's row (F-64131) is not ported"),
            JointKind::Other => 0.0,
        }
    }

    /// The tail of F-36821: a row per active degree of freedom, bias = rate - max(0.03 - depth,
    /// 0) / dt (fused), K, and -1 / K by a reciprocal and a Newton step (F-39033).
    pub fn limit_rows(&self, inv_dt: f32) -> Vec<LimitRow> {
        self.dofs
            .iter()
            .map(|d| {
                let short = ALLOWANCE - d.depth;
                let over = fsel(-short, 0.0, short);
                let bias = V4::splat(fnma(over, inv_dt, -d.rate));
                let k = V4::splat(self.row_inertia(d));
                LimitRow { dof: *d, mult: V4::ZERO, bias, k, inv: V4::ZERO.sub(rcp_nr(k)) }
            })
            .collect()
    }

    /// The row's axis as a force pair (linear, angular): hinge F-85878 S.w, turned over at
    /// the lower limit; swing-twist F-30472 n, e2, m, -m. The linear part is 0.
    fn row_axis(&self, d: &LimitDof) -> [V4; 2] {
        let ang = match &self.joints[d.joint].kind {
            JointKind::Hinge(h) if h.state == -1 => V4::ZERO.sub(h.axis[0]),
            JointKind::Hinge(h) => h.axis[0],
            JointKind::Ball(b) => match d.kind {
                0 => b.n,
                1 => b.e2,
                2 => b.m,
                3 => V4::ZERO.sub(b.m),
                _ => panic!("swing-twist row kind {} (its axis is a stale word)", d.kind),
            },
            JointKind::Other => panic!("the slider's row axis (F-20890) is not ported"),
        };
        [V4::ZERO, ang]
    }

    /// F-94941: the row's velocity as the sweep has moved it: the pivot walked to the joint's
    /// child (carried up to the parent if a change waits there), then axis . (B's seen motion)
    /// - axis . (A's).
    fn row_velocity(&mut self, d: &LimitDof) -> V4 {
        let c = d.joint + 1;
        let par = self.parent[c];
        self.pivot = self.walk(c, self.pivot);
        if self.flags[c] & 1 != 0 {
            self.scratch_up(c, par);
            self.pivot = par as i32;
        }
        let [lin, ang] = self.row_axis(d);
        let j = &self.joints[d.joint];
        let (la, lb) = (&self.links[j.a], &self.links[j.b]);
        let va = lin.dot3(la.head[2]).add(ang.dot3(la.head[1]));
        let vb = lin.dot3(lb.head[2]).add(ang.dot3(lb.head[1]));
        vb.sub(va)
    }

    /// F-17591: an impulse `s` along the row: axis * -s on the parent, its opposite on the
    /// child, each through its own inverse into the scratch (F-48604).
    fn row_impulse(&mut self, d: &LimitDof, s: f32) {
        let c = d.joint + 1;
        let par = self.parent[c];
        let [lin, ang] = self.row_axis(d);
        let t = V4::splat(-s);
        let pair = [lin.mul(t), ang.mul(t)];
        self.pivot = self.add_response(par, &pair, self.pivot);
        let back = [V4::ZERO.sub(pair[0]), V4::ZERO.sub(pair[1])];
        self.pivot = self.add_response(c, &back, self.pivot);
    }

    /// F-31194: one visit of a row: x = (-u - bias) (-1 / K); the step is x, or what takes the
    /// multiplier back to 0 where x would make it pull (lane x only); its square goes to the
    /// sweep's sum (`left`), -step to the bodies (F-97543, F-17591).
    pub fn visit_limit_row(&mut self, row: &mut LimitRow, left: &mut V4) {
        let u = self.row_velocity(&row.dof);
        let x = V4::ZERO.sub(u).sub(row.bias).mul(row.inv);
        let lane_x = V4::new(1.0, 0.0, 0.0, 0.0);
        let test = x.sub(row.mult).mul(lane_x);
        let step = x.sel(row.mult, V4::ZERO.gt(test));
        let give = V4::ZERO.sub(step);
        *left = give.madd(give, *left);
        self.row_impulse(&row.dof, give.x());
        row.mult = row.mult.sub(step);
    }

    /// The apply passes' joint part (F-24076 / F-64140): the row's multiplier word (+0, lane x)
    /// into the angular impulse accumulators (+720): hinge F-74238 (negated unless at the
    /// upper limit), swing-twist F-60451 (kind 3 along -m).
    pub fn apply_limit_row(&mut self, row: &LimitRow) {
        let s = row.mult.x();
        let k = row.dof.joint;
        let t = match &self.joints[k].kind {
            JointKind::Hinge(h) => h.axis[0].mul(V4::splat(if h.state == 1 { s } else { -s })),
            JointKind::Ball(b) => match row.dof.kind {
                0 => b.n.mul(V4::splat(s)),
                1 => b.e2.mul(V4::splat(s)),
                2 => b.m.mul(V4::splat(s)),
                3 => b.m.mul(V4::ZERO.sub(V4::splat(s))),
                _ => return,
            },
            JointKind::Other => panic!("the slider's row apply (F-82511) is not ported"),
        };
        let (a, b) = (self.joints[k].a, self.joints[k].b);
        self.links[b].ang_impulse = self.links[b].ang_impulse.add(t);
        self.links[a].ang_impulse = self.links[a].ang_impulse.add(V4::ZERO.sub(t));
    }

    // -----------------------------------------------------------------------------------------
    // The solve-time scratch

    /// F-80860 (from the sweeps' first pass F-90205, for every row of an articulated
    /// body): the scratch, the bits and the joints' +304 / +336 pairs cleared, no pivot.
    pub fn clear_scratch(&mut self) {
        for (l, f) in self.links.iter_mut().zip(self.flags.iter_mut()) {
            *f = 0;
            for v in &mut l.head[1..5] {
                *v = V4::ZERO;
            }
        }
        let n = self.n();
        for j in self.joints.iter_mut().take(n.saturating_sub(1)) {
            j.up = [V4::ZERO; 2];
            j.down = [V4::ZERO; 2];
        }
        self.pivot = -1;
    }

    /// F-48604: a force pair through link `i`'s own inverse (M^-1 (lin, ang)) into the scratch
    /// (F-10668); the new pivot.
    fn add_response(&mut self, i: usize, pair: &[V4; 2], pivot: i32) -> i32 {
        let (dw, dv) = block_mul(&self.links[i].inv, pair[0], pair[1]);
        self.add_motion(i, [dw, dv], pivot)
    }

    /// F-10668: the pivot walked to link `i` (where there is one), the change added to what the
    /// link has seen (+16 / +32) and to what waits to go up (+48 / +64); bits 1 | 2; `i`.
    fn add_motion(&mut self, i: usize, m: [V4; 2], pivot: i32) -> i32 {
        if pivot != -1 {
            self.walk(i, pivot);
        }
        let l = &mut self.links[i];
        l.head[1] = l.head[1].add(m[0]);
        l.head[2] = l.head[2].add(m[1]);
        l.head[3] = l.head[3].add(m[0]);
        l.head[4] = l.head[4].add(m[1]);
        self.flags[i] |= 3;
        i as i32
    }

    /// F-55744: the pivot walked to `target`: up from the deeper of the two (higher index) until
    /// they meet, the waiting changes carried up (F-31346), then down the target's side from
    /// where they met (F-81602). Returns where they met (-1 with no pivot).
    fn walk(&mut self, target: usize, pivot: i32) -> i32 {
        if pivot == -1 {
            return -1;
        }
        let (mut p, mut t) = (pivot as usize, target);
        let mut down = Vec::new();
        while p != t {
            if p > t {
                let pp = self.parent[p];
                self.scratch_up(p, pp);
                p = pp;
            } else {
                down.push(t);
                t = self.parent[t];
            }
        }
        while let Some(c) = down.pop() {
            self.scratch_down(c, t);
            t = c;
        }
        p as i32
    }

    /// F-31346: what waits at child `c` (bit 1) taken up its joint (F-45440 / F-75508) into
    /// the parent's seen and waiting motion and the joint's +336 pair; bit 1 off, the
    /// parent's bits 1 | 2.
    fn scratch_up(&mut self, c: usize, p: usize) {
        if self.flags[c] & 1 == 0 {
            return;
        }
        let x = [self.links[c].head[3], self.links[c].head[4]];
        let j = &mut self.joints[c - 1];
        let out = j.motion_up(&x).expect("the slider's maps are not ported");
        j.down = [j.down[0].add(out[0]), j.down[1].add(out[1])];
        let pl = &mut self.links[p];
        pl.head[1] = pl.head[1].add(out[0]);
        pl.head[2] = pl.head[2].add(out[1]);
        pl.head[3] = pl.head[3].add(out[0]);
        pl.head[4] = pl.head[4].add(out[1]);
        let cl = &mut self.links[c];
        cl.head[3] = V4::ZERO;
        cl.head[4] = V4::ZERO;
        self.flags[c] &= !1;
        self.flags[p] |= 3;
    }

    /// F-81602: where the parent (bit 2) has seen a motion the joint's +336 pair does not hold
    /// (x y z compared as words), the difference taken down the joint (F-71908 / F-91578, into
    /// +336) and added to the child's seen motion; +336 = the parent's; the child's bit 2.
    fn scratch_down(&mut self, c: usize, p: usize) {
        if self.flags[p] & 2 == 0 {
            return;
        }
        let (ps, pv) = (self.links[p].head[1], self.links[p].head[2]);
        let j = &mut self.joints[c - 1];
        let same = |a: V4, b: V4| a.bits()[..3] == b.bits()[..3];
        if same(j.down[0], ps) && same(j.down[1], pv) {
            return;
        }
        let d = [ps.sub(j.down[0]), pv.sub(j.down[1])];
        j.down = j.motion_down(&d).expect("the slider's maps are not ported");
        let out = j.down;
        j.down = [ps, pv];
        let cl = &mut self.links[c];
        cl.head[1] = cl.head[1].add(out[0]);
        cl.head[2] = cl.head[2].add(out[1]);
        self.flags[c] |= 2;
    }

    /// F-32404 (the collider's slot 15): the prediction of the forces and the impulses now
    /// pending (F-61076 with the impulses), and the mean squared motion (|root +992|^2 + the
    /// links' |w|^2 + |+976|^2) / N. It runs once a tick between the velocity solve's apply and
    /// slot 10 (its prediction is what +976 / +992 hold at slot 10's entry, dt as slot 10's);
    /// who calls it is not found (open).
    pub fn motion_measure(&mut self, dt: f32) -> f32 {
        self.predict(dt, true);
        let mut acc = self.links[0].predicted[1].dot3(self.links[0].predicted[1]);
        for l in &self.links {
            acc = acc.add(l.ang.dot3(l.ang)).add(l.predicted[0].dot3(l.predicted[0]));
        }
        acc.x() / self.n() as f32
    }

    // -----------------------------------------------------------------------------------------
    // The push solve's part

    /// F-49764's tail for a jointed body: its rows' push bias (depth - 0.03) / dt (the fresh
    /// work of the push sweeps), then the impulses committed (F-91555), the velocities saved to
    /// +944 / +960 and zeroed (F-17416, F-87768), so the push sweeps start from rest.
    pub fn push_start(&mut self, rows: &mut [LimitRow], inv_dt: f32) {
        for r in rows.iter_mut() {
            // (negated twice in the code: the same bits)
            r.bias = V4::splat((r.dof.depth - ALLOWANCE) * inv_dt);
        }
        self.commit_impulses();
        for l in &mut self.links {
            l.saved = l.motion;
            l.motion = [V4::ZERO; 2];
            l.ang = V4::ZERO;
            l.vel = V4::ZERO;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Contact rows with an articulated side (type 7)

/// The link a part of the bound stands on (F-19886): the collider's part -> link map (+716;
/// its length is the bound's part count, composite +146); 0 for a part out of range, a negative
/// entry, or one past the body's links. Slots 22, 34 and 50 take the link this way.
pub fn part_link(map: &[i32], n_links: usize, part: i32) -> usize {
    if part < 0 || part as usize >= map.len() {
        return 0;
    }
    let l = map[part as usize];
    if l < 0 || l as usize >= n_links { 0 } else { l as usize }
}

/// What an impulse along each of the contact basis' directions is in the world (the inverse
/// of the basis by columns: cofactors over the determinant, a reciprocal and a Newton step),
/// applied to the multipliers `p` ((p.x k0 + p.y k1) + p.z k2): F-24076 / F-64140 inline.
pub fn basis_impulse(bcol: &[V4; 3], p: V4) -> V4 {
    let c01 = cross(bcol[0], bcol[1]);
    let c12 = cross(bcol[1], bcol[2]);
    let c20 = cross(bcol[2], bcol[0]);
    let det = c01.dot3(bcol[2]);
    let k = columns(&[c12, c20, c01]);
    let r = det.recip();
    let r = r.madd(r.nmsub(det, ONE), r);
    let (k0, k1, k2) = (k[0].mul(r), k[1].mul(r), k[2].mul(r));
    p.lane(2).madd(k2, p.lane(1).madd(k1, p.lane(0).mul(k0)))
}

/// A rigid side's state in the island (the 128-byte state F-71455 makes; all zeros for the
/// fixed world): +0 velocity, +16 angular velocity, +32 the inverse mass rows, +80 the world
/// inverse inertia rows.
#[derive(Clone, Copy, Debug, Default)]
pub struct RigidState {
    pub vel: V4,
    pub ang: V4,
    pub m: [V4; 3],
    pub i: [V4; 3],
}

/// A contact row with an articulated side (type 7, 304 B). The row is the articulated side's:
/// its normal (+80 of the contact, or +96 when the articulated side is B), its point, and the
/// rigid (or fixed) other side as the row's B.
#[derive(Clone, Copy, Debug)]
pub struct ContactRow {
    /// +0: minus the multipliers so far
    pub neg: V4,
    /// +16: the velocity asked of the point along the basis
    pub bias: V4,
    /// +32: the basis by columns
    pub bcol: [V4; 3],
    /// +96: the inverse of the block (rows; lane w the block's)
    pub kinv: [V4; 3],
    /// +192: B's arms across the basis by columns
    pub arm_b: [V4; 3],
    /// +240: the block's first row over its first element; +256 the friction
    pub c: V4,
    pub mu: V4,
    /// +272: the articulated side's point less the collider's place; +288 its link
    pub r: V4,
    pub link: usize,
}

impl Articulated {
    /// F-18964 (the island build, on every contact of the island with a side on this body): a
    /// contact on a part marks the hinge above the part's link (byte +532), the joint whose
    /// child the link is; nothing for the root, a swing-twist or slider, or a part without link.
    pub fn contact_on_part(&mut self, map: &[i32], part: i32) {
        if part < 0 || part as usize >= map.len() {
            return;
        }
        let l = map[part as usize];
        if l <= 0 || l as usize >= self.n() {
            return;
        }
        let j = &mut self.joints[l as usize - 1];
        if j.kind_id == 0 {
            j.contact = 1;
        }
    }

    /// F-81637 (the collider's slot 22): the velocity the pending forces give the point `p`
    /// of link `k`: +992 + +976 x (p - place) (the prediction slot 5 left).
    pub fn point_pending(&self, k: usize, p: V4) -> V4 {
        let l = &self.links[k];
        cross_add(l.predicted[0], p.sub(self.place), l.predicted[1])
    }

    /// F-78078 (slot 50) -> F-67069: what an impulse at the point `p` of link `k` does to the
    /// point's velocity, by the link's own inverse blocks (+736 P, +784 Q, +832 R) and the
    /// arms c_j = e_j x r: rows ((T_i + U_i) + U^T_i) + R_i, M_i = sum P_ij c_j, U_i = sum
    /// Q_ij c_j, T_i = sum_j (c_j)_i M_j.
    pub fn point_response(&self, k: usize, p: V4) -> [V4; 3] {
        let r = p.sub(self.place);
        let e = [V4::new(1.0, 0.0, 0.0, 0.0), V4::new(0.0, 1.0, 0.0, 0.0), V4::new(0.0, 0.0, 1.0, 0.0)];
        let c = e.map(|e| cross(e, r));
        let inv = &self.links[k].inv;
        let comb = |q: V4| q.lane(2).madd(c[2], q.lane(1).madd(c[1], q.lane(0).mul(c[0])));
        let m = [comb(inv[0]), comb(inv[1]), comb(inv[2])];
        let u = [comb(inv[3]), comb(inv[4]), comb(inv[5])];
        let cc = columns(&c);
        let t = cc.map(|s| s.lane(2).madd(m[2], s.lane(1).madd(m[1], s.lane(0).mul(m[0]))));
        let ut = columns(&u);
        std::array::from_fn(|i| t[i].add(u[i]).add(ut[i]).add(inv[6 + i]))
    }

    /// F-50660: the velocity of the point at `r` (from the place) of link `k` as the sweep has
    /// moved it: the pivot walked to the link, then (+16 + w) x r + (+32 + v_O).
    fn point_velocity(&mut self, k: usize, r: V4) -> V4 {
        self.pivot = self.walk(k, self.pivot);
        let l = &self.links[k];
        let w = l.head[1].add(l.motion[0]);
        let v = l.head[2].add(l.motion[1]);
        cross(w, r).add(v)
    }

    /// F-96841: an impulse `j` at `r` on link `k` into the scratch (F-48604 with (j, r x j)).
    fn point_impulse(&mut self, k: usize, r: V4, j: V4) {
        let pair = [j, cross(r, j)];
        self.pivot = self.add_response(k, &pair, self.pivot);
    }

    /// F-19400 (slot 34; the push's slot 36, F-41014, comes here too): an impulse `j` at the
    /// point `p` into link `k`'s accumulators: +720 += (p - place) x j, +704 += j.
    pub fn apply_at_point(&mut self, k: usize, j: V4, p: V4) {
        let r = p.sub(self.place);
        let l = &mut self.links[k];
        l.ang_impulse = cross_add(r, j, l.ang_impulse);
        l.impulse = l.impulse.add(j);
    }

    /// The collider's type (+20): 1 the rider's articulated collider, 3 the other kind the push
    /// solve treats apart.
    pub fn collider_type(&self) -> u32 {
        ru(&self.raw_collider, 20)
    }
}

/// The relative velocity of the row's point along its basis: F-89413, the articulated side's
/// point velocity (F-50660) along the basis less B's ((basis . v_B) + (arms . w_B)).
fn row_velocity_of(row: &ContactRow, a: &mut Articulated, b: &RigidState) -> V4 {
    let va = a.point_velocity(row.link, row.r);
    let ua = cols_dot(&row.bcol, va);
    let lin = cols_dot(&row.bcol, b.vel);
    let ang = cols_dot(&row.arm_b, b.ang);
    ua.sub(lin.add(ang))
}

impl ContactRow {
    /// F-38941 with the rest of F-36821's type-7 branch: the basis `basis` (rows; F-95825 of
    /// the row's normal) by columns and B's arms (F-13531: B's point `pb` less its place), the
    /// articulated point `pa` and its link, K = the articulated point response (F-78078) turned
    /// into the basis (F-83464: D_ij = (K b_i) . b_j, by columns) plus B's (F-11306), c = K_0
    /// over K_00 (a reciprocal and a Newton step), the inverse in place (F-68939). Bias and
    /// multipliers are set by the caller.
    pub fn new(a: &Articulated, link: usize, pa: V4, pb: V4, place_b: V4, basis: &[V4; 3], b: &RigidState, friction: f32) -> ContactRow {
        let rb = pb.sub(place_b);
        let arms = basis.map(|x| cross(rb, x));
        let (bcol, arm_b) = (columns(basis), columns(&arms));
        let ka = in_basis(&columns(&a.point_response(link, pa)), basis);
        // (B's part, F-11306: T_i = sum_j M_ij bcol_j, L_i = sum_j (b_i)_j T_j; S_i = sum_j I_ij
        // acol_j, R_i = sum_j (a_i)_j S_j)
        let comb = |q: V4, x: &[V4; 3]| q.lane(2).madd(x[2], q.lane(1).madd(x[1], q.lane(0).mul(x[0])));
        let t = b.m.map(|q| comb(q, &bcol));
        let brow = columns(&bcol);
        let l = brow.map(|q| comb(q, &t));
        let s = b.i.map(|q| comb(q, &arm_b));
        let arow = columns(&arm_b);
        let rr = arow.map(|q| comb(q, &s));
        let mut k: [V4; 3] = std::array::from_fn(|i| rr[i].add(l[i]).add(ka[i]));
        let k00 = k[0].lane(0);
        let e = k00.recip();
        let c = k[0].mul(e.madd(e.nmsub(k00, ONE), e));
        ldl_inverse(&mut k);
        ContactRow { neg: V4::ZERO, bias: V4::ZERO, bcol, kinv: k, arm_b, c, mu: V4::splat(friction), r: pa.sub(a.place), link }
    }

    /// F-67470: a step of the multipliers given out: its square to the sweep's sum, B's state
    /// less M_B (basis . s) and I_B (arms . s), the articulated side the impulse (basis . s)
    /// at its point (F-96841).
    pub fn give(&self, a: &mut Articulated, b: &mut RigidState, step: V4, sum: &mut V4) {
        *sum = step.madd(step, *sum);
        let push = rows_dot(&self.bcol, step);
        let turn = rows_dot(&self.arm_b, step);
        b.vel = b.vel.sub(cols_dot(&b.m, push));
        b.ang = b.ang.sub(cols_dot(&b.i, turn));
        a.point_impulse(self.link, self.r, push);
    }

    /// F-90871: one visit: x = -(K^-1 (u - bias)) - (+0), no relaxation; the friction cone
    /// (lanes y z against mu times lane x through c), nothing where the normal part would
    /// pull; the step given out (F-67470) and taken off +0.
    pub fn visit(&mut self, a: &mut Articulated, b: &mut RigidState, sum: &mut V4) {
        let u = row_velocity_of(self, a, b).sub(self.bias);
        let step = cone_step(&self.kinv, self.c, self.mu, self.neg, u);
        self.give(a, b, step, sum);
        self.neg = self.neg.sub(step);
    }
}

/// F-83464: a 3 x 3 `kt` (by columns) into the basis `basis` (rows): W_i = sum_j kt_j b_i[j],
/// D_ij = W_i . b_j, out_k = (D_0k, D_1k, D_2k, 0).
fn in_basis(kt: &[V4; 3], basis: &[V4; 3]) -> [V4; 3] {
    let w = basis.map(|x| x.lane(2).madd(kt[2], x.lane(1).madd(kt[1], x.lane(0).mul(kt[0]))));
    let d: [[V4; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| w[i].dot3(basis[j])));
    std::array::from_fn(|k| V4::new(d[0][k].x(), d[1][k].x(), d[2][k].x(), 0.0))
}

/// The kernels' common part (F-90871 for type 7, F-33909 for type 13): from the relative
/// velocity less the bias `u`, x = -(K^-1 u) - (+0), no relaxation; the friction cone (lanes y
/// z against mu times lane x through c), nothing where the normal part would pull. Returns the
/// step (what +0 loses).
fn cone_step(kinv: &[V4; 3], c: V4, mu: V4, neg: V4, u: V4) -> V4 {
    // (a constant of the kernel: 1e-8 in lanes x y z, 0 in w)
    let t8 = f32::from_bits(0x322b_cc77);
    let tiny = V4::new(t8, t8, t8, 0.0);
    let ex = V4::new(1.0, 0.0, 0.0, 0.0);
    let x = V4::ZERO.sub(cols_dot(kinv, u)).sub(neg);
    let t = x.mul(V4::new(0.0, 1.0, 1.0, 0.0));
    let nn = c.dot3(x);
    let t2 = t.dot3(t);
    let n1 = nn.mul(ex);
    let e = t2.rsqrte();
    let zero = t2.eq(V4::ZERO);
    let pull = V4::ZERO.gt(n1.lane(0));
    let d = x.sub(n1);
    let y = t2.nmsub(e.mul(e), ONE).madd(HALF.mul(e), e);
    let len = t2.mul(y).sel(V4::ZERO, zero);
    let g = len.mul(mu).nmsub(ex, t);
    let (dg, xg) = (d.dot3(g), x.dot3(g));
    let s = dg.add(tiny);
    let r = s.recip();
    let r = r.madd(r.nmsub(s, ONE), r);
    let k = V4::ZERO.sel(xg.mul(r), xg.gt(V4::ZERO));
    let next = k.nmsub(d, x).sel(V4::ZERO, pull);
    next.add(neg)
}

// ---------------------------------------------------------------------------------------------
// Contact rows with both sides on one articulated body (type 13: the rider against himself)

impl Articulated {
    /// F-31685 (slot 49) -> F-46941: the response at the point `pb` of link `kb` to an impulse
    /// at the point `pa` of link `ka`, by columns: for each unit vector e_j F-45981 carries
    /// link `ka`'s change of motion to link `kb` and reads it at `pb`; the three results are
    /// merged as columns. Points less the collider's place; links by F-19886 (the caller's).
    pub fn cross_response(&self, ka: usize, pa: V4, kb: usize, pb: V4) -> [V4; 3] {
        let (ra, rb) = (pa.sub(self.place), pb.sub(self.place));
        let e = [V4::new(1.0, 0.0, 0.0, 0.0), V4::new(0.0, 1.0, 0.0, 0.0), V4::new(0.0, 0.0, 1.0, 0.0)];
        columns(&e.map(|e| self.carry(ka, ra, e, kb, rb)))
    }

    /// F-45981: link `a`'s change of motion from the impulse `e` at `ra` (its own inverse, the
    /// block product with (e, ra x e)), taken up the joints from the deeper side (the higher
    /// index: F-45440 / F-75508; a slider's F-91377) until it meets link `b`'s path, then down
    /// that path (F-71908 / F-91578; a slider's F-51575); the velocity of link `b`'s point at
    /// `rb`: w x rb + v.
    fn carry(&self, a: usize, ra: V4, e: V4, b: usize, rb: V4) -> V4 {
        let (dw, dv) = block_mul(&self.links[a].inv, e, cross(ra, e));
        let mut m = [dw, dv];
        let (mut a, mut b) = (a, b);
        let mut down = Vec::new();
        while a != b {
            if a > b {
                m = self.joints[a - 1].motion_up(&m).expect("the slider's maps are not ported");
                a = self.parent[a];
            } else {
                down.push(b);
                b = self.parent[b];
            }
        }
        while let Some(c) = down.pop() {
            m = self.joints[c - 1].motion_down(&m).expect("the slider's maps are not ported");
        }
        cross(m[0], rb).add(m[1])
    }
}

/// A contact row with both sides on one articulated body (type 13, 320 B; F-36821's branch
/// where both colliders are articulated, F-45220, F-30978).
#[derive(Clone, Copy, Debug)]
pub struct SelfRow {
    /// +0: minus the multipliers so far
    pub neg: V4,
    /// +16: the velocity asked of the points along the basis
    pub bias: V4,
    /// +32: the basis by columns
    pub bcol: [V4; 3],
    /// +96: the inverse of the block
    pub kinv: [V4; 3],
    /// +240: the block's first row over its first element; +256 the friction
    pub c: V4,
    pub mu: V4,
    /// +272 / +288: the two points less the collider's place; +304 / +308 their links
    pub ra: V4,
    pub rb: V4,
    pub la: usize,
    pub lb: usize,
}

impl SelfRow {
    /// The block: K = slot 50 at A's point + slot 50 at B's (F-78078), less the cross term of
    /// the one body (slot 49, F-31685) and its transpose, row by row: K_i = (K_i - C_i) -
    /// C^T_i; into the basis (F-83464 on K's columns); c = K_0 over K_00 (a reciprocal and a
    /// Newton step) and the inverse in place (F-30978 -> F-68939). Bias and multipliers are
    /// the caller's.
    pub fn new(a: &Articulated, la: usize, pa: V4, lb: usize, pb: V4, basis: &[V4; 3], friction: f32) -> SelfRow {
        let (ka, kb) = (a.point_response(la, pa), a.point_response(lb, pb));
        let k: [V4; 3] = std::array::from_fn(|i| ka[i].add(kb[i]));
        let c = a.cross_response(la, pa, lb, pb);
        let ct = columns(&c);
        let k: [V4; 3] = std::array::from_fn(|i| k[i].sub(c[i]).sub(ct[i]));
        let mut k = in_basis(&columns(&k), basis);
        let k00 = k[0].lane(0);
        let e = k00.recip();
        let cc = k[0].mul(e.madd(e.nmsub(k00, ONE), e));
        ldl_inverse(&mut k);
        SelfRow { neg: V4::ZERO, bias: V4::ZERO, bcol: columns(basis), kinv: k, c: cc, mu: V4::splat(friction), ra: pa.sub(a.place), rb: pb.sub(a.place), la, lb }
    }

    /// F-85931: a step given out: its square to the sweep's sum, the impulse (basis . s) at A's
    /// point and its opposite at B's (F-96841 each, A first).
    pub fn give(&self, a: &mut Articulated, step: V4, sum: &mut V4) {
        *sum = step.madd(step, *sum);
        let push = rows_dot(&self.bcol, step);
        a.point_impulse(self.la, self.ra, push);
        a.point_impulse(self.lb, self.rb, V4::ZERO.sub(push));
    }

    /// F-33909: one visit, the kernel of type 7 with u = F-11968: (bcol . the velocity of A's
    /// point) - (bcol . B's), both by F-50660 (A's first), less the bias; the step given out
    /// (F-85931).
    pub fn visit(&mut self, a: &mut Articulated, sum: &mut V4) {
        let va = a.point_velocity(self.la, self.ra);
        let vb = a.point_velocity(self.lb, self.rb);
        let u = cols_dot(&self.bcol, va).sub(cols_dot(&self.bcol, vb)).sub(self.bias);
        let step = cone_step(&self.kinv, self.c, self.mu, self.neg, u);
        self.give(a, step, sum);
        self.neg = self.neg.sub(step);
    }
}

/// F-36821's bias of a type-13 row: the pending velocities' difference `p` along the basis
/// rows (x0 in the rigid row's order: fma(p.z, n.z, fma(p.x, n.x, p.y n.y)); x1 x2 fused, z
/// first), for a contact the depth's allowance over the step (max(ALLOW - depth, 0), no skin,
/// no sign), then (0 - x) + target.
pub fn self_bias(basis: &[V4; 3], p: V4, gap: Option<f32>, inv_dt: f32, target: V4) -> V4 {
    let (n, b1, b2, p) = (basis[0].0, basis[1].0, basis[2].0, p.0);
    let mut x0 = fma(p[2], n[2], fma(p[0], n[0], p[1] * n[1]));
    if let Some(depth) = gap {
        let short = super::super::solver::ALLOW - depth;
        x0 = fma(fsel(-short, 0.0, short), inv_dt, x0);
    }
    let x1 = fma(b1[2], p[2], fma(b1[0], p[0], b1[1] * p[1]));
    let x2 = fma(b2[2], p[2], fma(b2[0], p[0], b2[1] * p[1]));
    V4::ZERO.sub(V4::new(x0, x1, x2, 0.0)).add(target)
}

/// F-36821's bias of a type-7 row: the pending velocities' difference `p` along the basis rows
/// (fused, z first), the depth's allowance over the step on the normal for a contact (`gap`:
/// max(ALLOW - depth, 0) times the row's sign, no skin taken off), then target + (0 - x) sign.
pub fn contact_bias(basis: &[V4; 3], p: V4, gap: Option<f32>, sign: V4, inv_dt: f32, target: V4) -> V4 {
    let (n, b1, b2, p) = (basis[0].0, basis[1].0, basis[2].0, p.0);
    let mut x0 = fma(p[0], n[0], fma(p[1], n[1], p[2] * n[2]));
    if let Some(depth) = gap {
        let short = super::super::solver::ALLOW - depth;
        x0 = fma(fsel(-short, 0.0, short) * sign.x(), inv_dt, x0);
    }
    let x1 = fma(b1[0], p[0], fma(b1[1], p[1], b1[2] * p[2]));
    let x2 = fma(b2[0], p[0], fma(b2[1], p[1], b2[2] * p[2]));
    V4::ZERO.sub(V4::new(x0, x1, x2, 0.0)).madd(sign, target)
}

// ---------------------------------------------------------------------------------------------
// Tests: islands of the recording with joint-limit rows only

#[cfg(test)]
mod tests {
    use super::super::tests::{check, load, run_cases, Case};
    use super::*;
    use crate::physics::solver;
    use glam::Vec3;

    fn island_cases() -> Vec<Case> {
        run_cases(include_bytes!("testdata/ragdoll_ro0_island.bin"))
    }

    /// F-73275 on the slot 5 exits of every recorded tick in the fixtures (the joints as the
    /// update leaves them, before the island solve's rows: RGD1's joint words outside the
    /// scratch pairs +304 / +336; the collider's +680). The hinge's byte +532 is the island's
    /// (F-18964, from the contacts): taken from RGD1.
    #[test]
    fn limit_dofs_are_the_originals() {
        let mut all = super::super::tests::cases();
        all.extend(island_cases());
        let (mut n, mut bad) = (0, Vec::new());
        for c in all.iter().filter(|c| c.op == b'i') {
            let mut a = load(&c.inp);
            for (k, j) in a.joints.iter_mut().enumerate() {
                j.contact = c.out.joints[k][532];
            }
            a.limit_dofs();
            n += 1;
            let want = ru(&c.out.collider, 680);
            if a.active != want {
                bad.push(format!("tick {}: {} rows, want {}", c.tick, a.active, want));
            }
            for k in 0..a.joints.len() {
                let (got, want) = (a.joint_to_guest(k), &c.out.joints[k]);
                for o in (0..want.len()).step_by(4) {
                    if (304..368).contains(&o) || ru(&got, o) == ru(want, o) {
                        continue;
                    }
                    bad.push(format!("tick {} joint{k}+{o}: {:08x} want {:08x}", c.tick, ru(&got, o), ru(want, o)));
                }
            }
        }
        eprintln!("limit dofs: {n} cases, {} differing words", bad.len());
        assert!(n > 0);
        assert!(bad.is_empty(), "{:#?}", &bad[..bad.len().min(20)]);
    }

    /// Every word of the body (links, joints, body, collider) against the recording. Apart
    /// (`contact`): words of a link's impulse accumulators (+704 .. +735) that are a zero here
    /// and a zero of the other sign or a NaN in lane w there - what the island's contact rows
    /// with a zero multiplier leave when they hand it out through slot 34 (F-19400): these
    /// tests run without the contacts (the ticks with contact rows of type 7 are checked word
    /// for word below, with the rider's self contacts, type 13, from 175).
    fn words(a: &Articulated, want: &super::super::tests::State) -> (Vec<String>, usize, usize) {
        let (mut bad, mut nan, mut contact) = (Vec::new(), 0, 0);
        for (i, l) in a.links.iter().enumerate() {
            let (got, w) = (l.to_guest(), &want.links[i].1);
            for o in (704..736).step_by(4) {
                let (g, x) = (ru(&got, o), ru(w, o));
                let zeros = (g | x) & 0x7fff_ffff == 0;
                let stale_w = o % 16 == 12 && g & 0x7fff_ffff == 0 && f32::from_bits(x).is_nan();
                if g != x && (zeros || stale_w) {
                    contact += 1;
                }
            }
        }
        check(a, want, &mut bad, &mut nan);
        bad.retain(|b| {
            let Some((name, rest)) = b.split_once('+') else { return true };
            let o: usize = rest.split(':').next().unwrap().parse().unwrap();
            let (g, x) = {
                let mut it = rest.split_whitespace();
                (u32::from_str_radix(it.nth(1).unwrap(), 16).unwrap(), u32::from_str_radix(it.nth(1).unwrap(), 16).unwrap())
            };
            let zeros = (g | x) & 0x7fff_ffff == 0;
            let stale_w = o % 16 == 12 && g & 0x7fff_ffff == 0 && f32::from_bits(x).is_nan();
            !(name.starts_with("link") && (704..736).contains(&o) && (zeros || stale_w))
        });
        (bad, nan, contact)
    }

    /// The island's velocity solve with joint-limit rows only (RGX0 -> RGD1; dt = v1 as the
    /// recording's slot 10 takes it): the limit tests, the rows, 8 sweeps, the apply.
    fn velocity_solve(c: &Case) -> (Articulated, solver::IslandSolved) {
        let mut a = load(&c.inp);
        for (k, j) in a.joints.iter_mut().enumerate() {
            j.contact = c.out.joints[k][532];
        }
        let dt = c.v1;
        let mut jointed = [a];
        let solved = solver::island_solve_jointed(&[], &[], &mut jointed, (0, 0.0), dt, 1.0 / dt);
        solver::island_apply_limits(&solved, &mut jointed);
        let [mut a] = jointed;
        a.motion_measure(dt);
        (a, solved)
    }

    /// Ticks 169 and 173 of the recording: islands whose solve moved the rider through joint
    /// rows only (no link took a linear impulse).
    #[test]
    fn joint_rows_velocity_solve_is_the_originals() {
        let (mut n, mut report) = (0, Vec::new());
        for c in island_cases().iter().filter(|c| c.op == b'i' && (c.tick == 169 || c.tick == 173)) {
            let (a, _) = velocity_solve(c);
            let (bad, nan, contact) = words(&a, &c.out);
            eprintln!("velocity solve, tick {}: {} rows, {} differing words, {nan} NaN on both sides, {contact} from zero contact impulses", c.tick, a.active, bad.len());
            if !bad.is_empty() {
                report.push(format!("tick {}: {} words, first {:#?}", c.tick, bad.len(), &bad[..bad.len().min(16)]));
            }
            n += 1;
        }
        assert_eq!(n, 2);
        assert!(report.is_empty(), "{}", report.join("
"));
    }

    /// The push solve (RGX1 -> RGD2, dt = f1 as the post-push takes it) on the rows the
    /// velocity solve of the same tick built, at ticks where the push moved the rider through
    /// joint rows only.
    #[test]
    fn joint_rows_push_solve_is_the_originals() {
        let cases = island_cases();
        let (mut n, mut report) = (0, Vec::new());
        for c in cases.iter().filter(|c| c.op == b's') {
            let vc = cases.iter().find(|v| v.op == b'i' && v.tick == c.tick).expect("the tick's velocity case");
            let (_, mut solved) = velocity_solve(vc);
            let a = load(&c.inp);
            let dt = c.f1;
            let mut jointed = [a];
            solver::island_push_jointed(&[], &mut [], &mut solved, &mut jointed, dt, 1.0 / dt, &mut []);
            let [a] = jointed;
            let (bad, nan, contact) = words(&a, &c.out);
            eprintln!("push solve, tick {}: {} differing words, {nan} NaN on both sides, {contact} from zero contact impulses", c.tick, bad.len());
            if !bad.is_empty() {
                report.push(format!("tick {}: {} words, first {:#?}", c.tick, bad.len(), &bad[..bad.len().min(16)]));
            }
            n += 1;
        }
        assert!(n > 0);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    // -----------------------------------------------------------------------------------------
    // Islands of the recording with the rider's contacts against the city (rows of type 7)

    /// One contact of a tick's island (rider_island_ro0.bin): which side the rider is (0 A, 1
    /// B, 2 neither: the bike's own, 3 both: the rider against himself), its parts, the
    /// 176-byte record's words as the tick's PAIR record holds them (after the solve).
    struct IslContact {
        side: u32,
        parts: [i32; 2],
        w: [u32; 44],
        /// the last earlier tick whose PAIR records hold a contact at the same address (0: none),
        /// and that record's +0, +128 and +16 (A's local point)
        prev_tick: u32,
        prev: [u32; 12],
    }

    struct IslTick {
        tick: u32,
        bounce: u32,
        n: u32,
        contacts: Vec<IslContact>,
        hinge_parts: Vec<i32>,
    }

    fn island_ticks() -> (Vec<i32>, Vec<IslTick>) {
        let b = include_bytes!("testdata/rider_island_ro0.bin");
        assert_eq!(&b[..8], b"RGISL002");
        let w: Vec<u32> = b[8..].chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let (ticks, mut i) = (w[0] as usize, 1);
        let n = w[i] as usize;
        let map: Vec<i32> = w[i + 1..i + 1 + n].iter().map(|&x| x as i32).collect();
        i += 1 + n;
        let mut out = Vec::new();
        for _ in 0..ticks {
            let len = w[i] as usize;
            let t = &w[i + 1..i + 1 + len];
            i += 1 + len;
            let mut k = 4;
            let mut contacts = Vec::new();
            for _ in 0..t[3] {
                contacts.push(IslContact { side: t[k], parts: [t[k + 1] as i32, t[k + 2] as i32], w: t[k + 3..k + 47].try_into().unwrap(), prev_tick: t[k + 47], prev: t[k + 48..k + 60].try_into().unwrap() });
                k += 60;
            }
            let hinge_parts = t[k + 1..k + 1 + t[k] as usize].iter().map(|&x| x as i32).collect();
            out.push(IslTick { tick: t[0], bounce: t[1], n: t[2], contacts, hinge_parts });
        }
        (map, out)
    }

    /// The rider's contacts of a tick as the island solve takes them: the solve's inputs from the
    /// record (depth +12, friction +28, points +48 / +64 with their lanes w, normal +80, back +96,
    /// target +144, kind +164), elasticity 0 (the rider's material against the city and
    /// himself). Kept multipliers (+128) and push multipliers (+0 xyz), the warm starts: the
    /// previous tick's record of the contact at the same address when it is the same contact -
    /// the sink (F-90136) gave the fresh contact the old one's +128 and +0 because A's local
    /// points (+16) are within 2 cm (F-74262) - else 0: a contact the previous tick's pairs do
    /// not hold is new (also when the address held one two ticks before), and so is one that
    /// took a freed slot's address (170: 2.6 and 48 cm, 172: 4.6 cm, 174: 7.1 and 3.2 cm, 179:
    /// 26 cm). The bike's own contact (side 2) shares no body with these: its count and
    /// elasticity (0.1) go in as `others`.
    fn rider_touches(map: &[i32], n_links: usize, t: &IslTick) -> (Vec<solver::Touch>, (usize, f32)) {
        let mut others = (0, 0.0f32);
        let mut out = Vec::new();
        for c in &t.contacts {
            if c.side == 2 {
                others = (others.0 + 1, others.1 + 0.1);
                continue;
            }
            let f = |o: usize| f32::from_bits(c.w[o / 4]);
            let v = |o: usize| Vec3::new(f(o), f(o + 4), f(o + 8));
            let on_b = c.side == 1;
            let jointed = if c.side == 3 {
                // (both sides on the rider: type 13)
                let lb = part_link(map, n_links, c.parts[1]);
                solver::JointedSide { body: 0, link: part_link(map, n_links, c.parts[0]), on_b: false, point_w: f(60), self_b: Some((lb, f(76))) }
            } else {
                solver::JointedSide { body: 0, link: part_link(map, n_links, c.parts[c.side as usize]), on_b, point_w: if on_b { f(76) } else { f(60) }, self_b: None }
            };
            let side = |o: usize| solver::Side { point: v(o), ..Default::default() };
            let pw = |i: usize| f32::from_bits(c.prev[i]);
            let d = v(16) - Vec3::new(pw(8), pw(9), pw(10));
            let near = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)) < f32::from_bits(0x39d1_b717);
            let prev = |i: usize| if c.prev_tick + 1 != t.tick || !near { Vec3::ZERO } else { Vec3::new(f32::from_bits(c.prev[i]), f32::from_bits(c.prev[i + 1]), f32::from_bits(c.prev[i + 2])) };
            out.push(solver::Touch {
                depth: f(12),
                friction: f(28),
                a: side(48),
                b: side(64),
                normal: v(80),
                back: v(96),
                target: v(144),
                kind: (c.w[41] >> 24) as u8,
                jointed: Some(jointed),
                kept: prev(4),
                pushed: prev(0),
                ..Default::default()
            });
        }
        (out, others)
    }

    /// Strict: every word of links, joints, body and collider.
    fn strict(a: &Articulated, want: &super::super::tests::State) -> (Vec<String>, usize) {
        let (mut bad, mut nan) = (Vec::new(), 0);
        check(a, want, &mut bad, &mut nan);
        (bad, nan)
    }

    fn vec_words(got: Vec3, want: &[u32]) -> bool {
        got.x.to_bits() == want[0] && got.y.to_bits() == want[1] && got.z.to_bits() == want[2]
    }

    /// The island build's hinge bytes (F-18964 over every contact of the island's manifolds with
    /// a side on the rider, enabled or not): from slot 5's exit (RGX0, all clear) to RGD1's.
    #[test]
    fn hinge_contact_bytes_are_the_originals() {
        let (map, ticks) = island_ticks();
        let cases = island_contact_cases();
        let mut n = 0;
        for t in &ticks {
            let Some(c) = cases.iter().find(|c| c.op == b'i' && c.tick == t.tick) else { continue };
            let mut a = load(&c.inp);
            for &p in &t.hinge_parts {
                a.contact_on_part(&map, p);
            }
            for (k, j) in a.joints.iter().enumerate() {
                assert_eq!(j.contact, c.out.joints[k][532], "tick {} joint {k}", t.tick);
            }
            n += 1;
        }
        assert_eq!(n, CONTACT_TICKS);
    }

    /// The ticks of rider_island_ro0.bin (169 - 184) with velocity and push cases: 169 - 177,
    /// 179, 180, 184 (175 on: self contacts, type 13; ragdoll_ro0_self.bin: 176, 179, 180, 184).
    const CONTACT_TICKS: usize = 12;

    fn island_contact_cases() -> Vec<Case> {
        let mut all = island_cases();
        all.extend(run_cases(include_bytes!("testdata/ragdoll_ro0_contact.bin")));
        all.extend(run_cases(include_bytes!("testdata/ragdoll_ro0_self.bin")));
        all
    }

    /// The velocity solve of ticks 169 - 181 with the rider's contacts (rows of type 7, and of
    /// type 13 from 175: the rider against himself), then
    /// the apply (F-24076: slot 34 per contact, then the joint rows): RGX0 -> RGD1 word for word,
    /// the island's elasticity factor (isl +40), each contact's kept multipliers (+128) and
    /// impulse (+112).
    fn contact_velocity_solve(map: &[i32], t: &IslTick, c: &Case) -> (Articulated, solver::IslandSolved, Vec<solver::Touch>, Vec<String>) {
        let mut a = load(&c.inp);
        for &p in &t.hinge_parts {
            a.contact_on_part(map, p);
        }
        let (mut touches, others) = rider_touches(map, a.n(), t);
        assert_eq!(touches.len() + others.0, t.n as usize);
        let dt = c.v1;
        let mut jointed = [a];
        let solved = solver::island_solve_jointed(&[], &touches, &mut jointed, others, dt, 1.0 / dt);
        solver::island_apply_jointed(&solved, &mut jointed);
        let [mut a] = jointed;
        a.motion_measure(dt);
        let mut bad = Vec::new();
        if solved.bounce.to_bits() != t.bounce {
            bad.push(format!("bounce {:08x} want {:08x}", solved.bounce.to_bits(), t.bounce));
        }
        solver::island_impulses(&solved, &mut touches);
        for (k, (x, rec)) in touches.iter().zip(t.contacts.iter().filter(|c| c.side != 2)).enumerate() {
            if !vec_words(x.kept, &rec.w[32..35]) {
                bad.push(format!("contact {k} kept {:?} want {:08x?}", x.kept, &rec.w[32..35]));
            }
            if !vec_words(x.impulse, &rec.w[28..31]) {
                bad.push(format!("contact {k} impulse {:?} want {:08x?}", x.impulse, &rec.w[28..31]));
            }
        }
        (a, solved, touches, bad)
    }

    #[test]
    fn contact_rows_velocity_solve_is_the_originals() {
        let (map, ticks) = island_ticks();
        let cases = island_contact_cases();
        let mut report = Vec::new();
        let mut n = 0;
        for t in &ticks {
            let Some(c) = cases.iter().find(|c| c.op == b'i' && c.tick == t.tick) else { continue };
            n += 1;
            let (a, _, touches, mut bad) = contact_velocity_solve(&map, t, c);
            let (words, nan) = strict(&a, &c.out);
            eprintln!("velocity solve with contacts, tick {}: {} contact rows, {} joint rows, {} differing words, {nan} NaN on both sides, {} contact results differ", t.tick, touches.len(), a.active, words.len(), bad.len());
            bad.extend(words);
            if !bad.is_empty() {
                report.push(format!("tick {}: {} words, first {:#?}", t.tick, bad.len(), &bad[..bad.len().min(16)]));
            }
        }
        assert_eq!(n, CONTACT_TICKS);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    /// The push solve of the same ticks (RGX1 -> RGD2, dt = f1) on the rows of the same tick's
    /// velocity solve, word for word, and each contact's push multipliers (+0).
    #[test]
    fn contact_rows_push_solve_is_the_originals() {
        let (map, ticks) = island_ticks();
        let cases = island_contact_cases();
        let mut report = Vec::new();
        let mut n = 0;
        for t in &ticks {
            let Some(vc) = cases.iter().find(|c| c.op == b'i' && c.tick == t.tick) else { continue };
            let c = cases.iter().find(|c| c.op == b's' && c.tick == t.tick).expect("the tick's push case");
            n += 1;
            // (the contacts keep the push multipliers they came with: pushed is the push solve's input)
            let (_, mut solved, mut touches, _) = contact_velocity_solve(&map, t, vc);
            let dt = c.f1;
            let mut jointed = [load(&c.inp)];
            solver::island_push_jointed(&[], &mut touches, &mut solved, &mut jointed, dt, 1.0 / dt, &mut []);
            let [a] = jointed;
            let (mut bad, nan) = strict(&a, &c.out);
            let words = bad.len();
            for (k, (x, rec)) in touches.iter().zip(t.contacts.iter().filter(|c| c.side != 2)).enumerate() {
                if !vec_words(x.pushed, &rec.w[0..3]) {
                    bad.push(format!("contact {k} pushed {:?} want {:08x?}", x.pushed, &rec.w[0..3]));
                }
            }
            eprintln!("push solve with contacts, tick {}: {words} differing words, {nan} NaN on both sides, {} contact results differ", t.tick, bad.len() - words);
            if !bad.is_empty() {
                report.push(format!("tick {}: {} words, first {:#?}", t.tick, bad.len(), &bad[..bad.len().min(16)]));
            }
        }
        assert_eq!(n, CONTACT_TICKS);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    // -----------------------------------------------------------------------------------------
    // The shared island: the thrown rider and his bike in one solve (rider_bike_shared.bin)

    /// One tick of rider_bike_shared.bin (export_shared.py over rv.0): the island's enabled contacts in
    /// the A list's order with which side is the rider and which the bike, the hinge parts, the bike's
    /// collider at slot 5's entry and at slot 10's entry (the RGD0 / RGD1 snapshots), and its instance's
    /// matrix at slot 10's entry (the file also holds the one at slot 5's entry, unused).
    struct SharedTick {
        tick: u32,
        bounce: u32,
        n: u32,
        contacts: Vec<(u32, IslContact)>,
        hinge_parts: Vec<i32>,
        bike0: Vec<u32>,
        bike1: Vec<u32>,
        matrix1: Vec<u32>,
    }

    fn shared_ticks() -> (Vec<i32>, Vec<SharedTick>) {
        shared_ticks_of(include_bytes!("testdata/rider_bike_shared.bin"))
    }

    fn shared_ticks_of(b: &[u8]) -> (Vec<i32>, Vec<SharedTick>) {
        assert_eq!(&b[..8], b"RGSHR001");
        let w: Vec<u32> = b[8..].chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let (ticks, mut i) = (w[0] as usize, 1);
        let n = w[i] as usize;
        let map: Vec<i32> = w[i + 1..i + 1 + n].iter().map(|&x| x as i32).collect();
        i += 1 + n;
        let mut out = Vec::new();
        for _ in 0..ticks {
            let len = w[i] as usize;
            let t = &w[i + 1..i + 1 + len];
            i += 1 + len;
            let mut k = 4;
            let mut contacts = Vec::new();
            for _ in 0..t[3] {
                let c = IslContact { side: t[k], parts: [t[k + 2] as i32, t[k + 3] as i32], w: t[k + 4..k + 48].try_into().unwrap(), prev_tick: t[k + 48], prev: t[k + 49..k + 61].try_into().unwrap() };
                contacts.push((t[k + 1], c));
                k += 61;
            }
            let hinge_parts = t[k + 1..k + 1 + t[k] as usize].iter().map(|&x| x as i32).collect();
            k += 1 + t[k] as usize;
            let bike0 = t[k..k + 176].to_vec();
            let bike1 = t[k + 176..k + 352].to_vec();
            let matrix1 = t[k + 368..k + 384].to_vec();
            out.push(SharedTick { tick: t[0], bounce: t[1], n: t[2], contacts, hinge_parts, bike0, bike1, matrix1 });
        }
        (map, out)
    }

    fn wf(w: &[u32], o: usize) -> f32 {
        f32::from_bits(w[o / 4])
    }
    fn wv(w: &[u32], o: usize) -> Vec3 {
        Vec3::new(wf(w, o), wf(w, o + 4), wf(w, o + 8))
    }

    /// The bike's body as the island takes it, from its collider at slot 5's entry: place +256, velocity +272,
    /// angular velocity +288, inverse mass +196, inverse moments +176, the sums +336 / +352; its rows the instance's.
    /// (its collider at slot 10's entry - the solve writes none of these - and its instance's rows there)
    fn shared_bike(t: &SharedTick) -> solver::Body {
        let (w, m) = (&t.bike1, &t.matrix1);
        solver::Body {
            at: wv(w, 256),
            rows: [wv(m, 0), wv(m, 16), wv(m, 32)],
            vel: wv(w, 272),
            ang: wv(w, 288),
            inv_mass: wf(w, 196),
            inv_moments: wv(w, 176),
            force: wv(w, 336),
            torque: wv(w, 352),
        }
    }

    /// The island's rows as the shared step makes them: the rider's (type 7 / 13) with the bike, where it is the
    /// other side, as the island's body 0; the bike's with the city as its own rows. Every row's elasticity as
    /// recorded (the rider's 0, the bike's on the rider 0.1, F-55998).
    fn shared_touches(map: &[i32], n_links: usize, t: &SharedTick) -> Vec<solver::Touch> {
        let mut out = Vec::new();
        for (bside, c) in &t.contacts {
            let f = |o: usize| f32::from_bits(c.w[o / 4]);
            let v = |o: usize| Vec3::new(f(o), f(o + 4), f(o + 8));
            let on = |s: u32| if *bside == s { Some(0) } else { None };
            let side = |o: usize, s: u32| solver::Side { body: on(s), point: v(o), ..Default::default() };
            let jointed = match c.side {
                3 => Some(solver::JointedSide { body: 0, link: part_link(map, n_links, c.parts[0]), on_b: false, point_w: f(60), self_b: Some((part_link(map, n_links, c.parts[1]), f(76))) }),
                0 | 1 => {
                    let on_b = c.side == 1;
                    Some(solver::JointedSide { body: 0, link: part_link(map, n_links, c.parts[c.side as usize]), on_b, point_w: if on_b { f(76) } else { f(60) }, self_b: None })
                }
                _ => None,
            };
            let pw = |i: usize| f32::from_bits(c.prev[i]);
            let d = v(16) - Vec3::new(pw(8), pw(9), pw(10));
            let near = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)) < f32::from_bits(0x39d1_b717);
            let prev = |i: usize| if c.prev_tick + 1 != t.tick || !near { Vec3::ZERO } else { Vec3::new(pw(i), pw(i + 1), pw(i + 2)) };
            out.push(solver::Touch {
                depth: f(12),
                friction: f(28),
                elasticity: f(44),
                a: side(48, 0),
                b: side(64, 1),
                normal: v(80),
                back: v(96),
                target: v(144),
                kind: (c.w[41] >> 24) as u8,
                jointed,
                kept: prev(4),
                pushed: prev(0),
                ..Default::default()
            });
        }
        out
    }

    /// The shared island's velocity solve (F-68442 / F-36821 with the bike as a rigid body and the rider as the
    /// jointed one) over rv.0 156 - 167, the gate open, the rider A and the bike B: the rider RGX0 -> RGD1 word for
    /// word, the elasticity factor (isl +40), every row's kept multipliers (+128) and impulse (+112), and the bike's
    /// share - what the island's apply adds to its accumulators +368 / +384 between the two snapshots - bit for bit.
    /// The bike's body is its collider at slot 10's entry (the move between the snapshots is its own: place +256,
    /// rows, angular velocity +288 from the turned momentum; velocity +272 and the sums +336 / +352 unchanged).
    #[test]
    fn shared_island_velocity_solve_is_the_originals() {
        let (map, ticks) = shared_ticks();
        let cases = run_cases(include_bytes!("testdata/rider_bike_shared_run.bin"));
        let mut report = Vec::new();
        let mut n = 0;
        for t in &ticks {
            let Some(c) = cases.iter().find(|c| c.op == b'i' && c.tick == t.tick) else { continue };
            n += 1;
            let mut a = load(&c.inp);
            for &p in &t.hinge_parts {
                a.contact_on_part(&map, p);
            }
            let mut touches = shared_touches(&map, a.n(), t);
            assert_eq!(touches.len(), t.n as usize);
            let bike = shared_bike(t);
            let dt = c.v1;
            let mut jointed = [a];
            let solved = solver::island_solve_jointed(&[bike], &touches, &mut jointed, (0, 0.0), dt, 1.0 / dt);
            solver::island_apply_jointed(&solved, &mut jointed);
            let [mut a] = jointed;
            a.motion_measure(dt);
            let mut bad = Vec::new();
            if solved.bounce.to_bits() != t.bounce {
                bad.push(format!("bounce {:08x} want {:08x}", solved.bounce.to_bits(), t.bounce));
            }
            solver::island_impulses(&solved, &mut touches);
            for (k, (x, (_, rec))) in touches.iter().zip(t.contacts.iter()).enumerate() {
                if !vec_words(x.kept, &rec.w[32..35]) {
                    bad.push(format!("row {k} (side {}) kept {:?} want {:08x?}", rec.side, x.kept, &rec.w[32..35]));
                }
                if !vec_words(x.impulse, &rec.w[28..31]) {
                    bad.push(format!("row {k} (side {}) impulse {:?} want {:08x?}", rec.side, x.impulse, &rec.w[28..31]));
                }
            }
            // (the bike's share: the island's apply into its accumulators +368 / +384, empty at slot 5's entry;
            // its velocities +272 / +288 are not written by the solve)
            let given = solver::island_apply(&[bike], &touches);
            let (b0, b1) = (&t.bike0, &t.bike1);
            for (name, o, got) in [("impulse +368", 368, given[0].0), ("turning +384", 384, given[0].1)] {
                if wv(b0, o) != Vec3::ZERO || !vec_words(got, &b1[o / 4..o / 4 + 3]) {
                    bad.push(format!("bike {name}: island {got:?}, recorded {:?} -> {:?}", wv(b0, o), wv(b1, o)));
                }
            }
            let (words, nan) = strict(&a, &c.out);
            eprintln!("shared velocity solve, tick {}: {} rows ({} with the bike), {} differing words, {nan} NaN on both sides, {} row results differ", t.tick, touches.len(), t.contacts.iter().filter(|(b, _)| *b != 2).count(), words.len(), bad.len());
            bad.extend(words);
            if OPEN.contains(&t.tick) {
                // (open: see OPEN)
                if bad.is_empty() {
                    report.push(format!("tick {}: exact now - take it off OPEN", t.tick));
                }
            } else if !bad.is_empty() {
                report.push(format!("tick {}: {} differences, first {:#?}", t.tick, bad.len(), &bad[..bad.len().min(16)]));
            }
        }
        assert_eq!(n, 12);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    /// The ticks of rv.0 whose shared solve is not yet the original's word for word (unverified, open): 156, 158 and
    /// 159. The rows that differ there are exactly the tick's rows on the friction cone (their multipliers within
    /// about 1e-5 of the recorded ones, the bike's share within 1e-6), while the cone rows of the other nine ticks are
    /// exact. Ruled out: the warm starts (every choice of the earlier records' +0 / +128), the hinge bytes (each part
    /// added or taken away), the bike's body (place, rows, angular velocity and sums from either snapshot), the rows'
    /// inputs (the RGD1 snapshot's records equal the PAIR records but for +0), the row order (A list), the sweep
    /// count (island +20 = 1: 8) and the elasticity factor (equal on every tick).
    const OPEN: [u32; 3] = [156, 158, 159];

    /// The thrown rider against another vehicle: ov.0 724 - 735 (recomp recorder, a parked ambient car 0x5745 -
    /// mcAmbientPhysInst, level state 0 (active) after the bike struck it - the rider 0x5754 lying on it, the bike
    /// elsewhere). Island = the car (rigid body 0, side A of every row, its part 0) and the rider (jointed, side B, or
    /// both sides of his self rows); no city row. Same check as the bike's: the rider RGX0 -> RGD1 word for word, the
    /// elasticity factor (isl +40 = 1: the rows' elasticity is 0 - the rider's F-22759 - and neither the ambient car's
    /// pre-solve callback nor anything else writes it), every row's kept multipliers and impulse, and the car's share
    /// in its accumulators +368 / +384 between the snapshots. Fixtures rider_car_ov0.bin (export_car.py, RGSHR001 with
    /// the car as the "bike") and rider_car_ov0_run.bin (export_island.py, ops 'i').
    #[test]
    fn rider_car_island_velocity_solve_is_the_originals() {
        car_island(include_bytes!("testdata/rider_car_ov0.bin"), include_bytes!("testdata/rider_car_ov0_run.bin"), &OPEN_CAR, 12);
    }

    /// The thrown rider against an AI racer's car (class 8, an mcCar): xi.0 1387 - 1392 (recomp recorder: racer 58's car
    /// 0x572e held side on, the rider 0x5762 thrown onto it off an ambient car). Island = the car (rigid body 0, side A)
    /// and the rider; 1387 also holds the car's own hull row with the city (side A only), 1390 none of the car's (his
    /// self rows only, the car still a body of the island). The same check as ov.0's; the rows' friction / elasticity
    /// as the rider's F-22759 leaves them (the car's F-55998 writes nothing: upright, not wiped out). Fixtures
    /// rider_car_xi0.bin (export_car.py) and rider_car_xi0_run.bin (export_island.py, ops 'i').
    #[test]
    fn rider_racer_island_velocity_solve_is_the_originals() {
        car_island(include_bytes!("testdata/rider_car_xi0.bin"), include_bytes!("testdata/rider_car_xi0_run.bin"), &OPEN_RACER, 6);
    }

    fn car_island(fixture: &[u8], run: &[u8], open: &[u32], count: usize) {
        let (map, ticks) = shared_ticks_of(fixture);
        let cases = run_cases(run);
        let mut report = Vec::new();
        let mut n = 0;
        for t in &ticks {
            let Some(c) = cases.iter().find(|c| c.op == b'i' && c.tick == t.tick) else { continue };
            n += 1;
            let mut a = load(&c.inp);
            for &p in &t.hinge_parts {
                a.contact_on_part(&map, p);
            }
            let mut touches = shared_touches(&map, a.n(), t);
            assert_eq!(touches.len(), t.n as usize);
            let car = shared_bike(t);
            let dt = c.v1;
            let mut jointed = [a];
            let solved = solver::island_solve_jointed(&[car], &touches, &mut jointed, (0, 0.0), dt, 1.0 / dt);
            solver::island_apply_jointed(&solved, &mut jointed);
            let [mut a] = jointed;
            a.motion_measure(dt);
            let mut bad = Vec::new();
            if solved.bounce.to_bits() != t.bounce {
                bad.push(format!("bounce {:08x} want {:08x}", solved.bounce.to_bits(), t.bounce));
            }
            solver::island_impulses(&solved, &mut touches);
            for (k, (x, (_, rec))) in touches.iter().zip(t.contacts.iter()).enumerate() {
                if !vec_words(x.kept, &rec.w[32..35]) {
                    bad.push(format!("row {k} (side {}) kept {:?} want {:08x?}", rec.side, x.kept, &rec.w[32..35]));
                }
                if !vec_words(x.impulse, &rec.w[28..31]) {
                    bad.push(format!("row {k} (side {}) impulse {:?} want {:08x?}", rec.side, x.impulse, &rec.w[28..31]));
                }
            }
            let given = solver::island_apply(&[car], &touches);
            let (b0, b1) = (&t.bike0, &t.bike1);
            for (name, o, got) in [("impulse +368", 368, given[0].0), ("turning +384", 384, given[0].1)] {
                if wv(b0, o) != Vec3::ZERO || !vec_words(got, &b1[o / 4..o / 4 + 3]) {
                    bad.push(format!("car {name}: island {got:?}, recorded {:?} -> {:?}", wv(b0, o), wv(b1, o)));
                }
            }
            let (words, nan) = strict(&a, &c.out);
            eprintln!("rider x car velocity solve, tick {}: {} rows ({} with the car), {} differing words, {nan} NaN on both sides, {} row results differ", t.tick, touches.len(), t.contacts.iter().filter(|(b, _)| *b != 2).count(), words.len(), bad.len());
            bad.extend(words);
            if open.contains(&t.tick) {
                if bad.is_empty() {
                    report.push(format!("tick {}: exact now - take it off the open list", t.tick));
                }
            } else if !bad.is_empty() {
                report.push(format!("tick {}: {} differences, first {:#?}", t.tick, bad.len(), &bad[..bad.len().min(16)]));
            }
        }
        assert_eq!(n, count);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    /// xi.0's racer ticks not yet word for word (unverified, open).
    const OPEN_RACER: [u32; 0] = [];

    /// The tick of ov.0 whose rider x car solve is not yet the original's word for word (unverified, open): 730, the
    /// first tick of six rows all with the car (no self row), four of them new sub-manifolds. One row (rider part 17)
    /// ends about 1e-5 from the recorded multipliers and impulse, and the rider's links by an ulp or two - the same kind
    /// of difference as rv.0 156 / 158 / 159 ([`OPEN`]). The car's share and the other eleven ticks are exact.
    const OPEN_CAR: [u32; 1] = [730];

    // -----------------------------------------------------------------------------------------
    // One island of several rigid bodies and the rider (RGISL001, export_isl.py): the rider thrown onto a vehicle that
    // is in other pairs of the loose world (a semi trailer with its hitch to the cab), one solve for all

    /// A rigid body of an RGISL001 tick: its level index, its collider at slot 5's and at slot 10's entry of the rider (the
    /// RGD0 / RGD1 snapshots) and its instance's matrix at both.
    struct IslBody {
        level: u32,
        col0: Vec<u32>,
        col1: Vec<u32>,
        matrix1: Vec<u32>,
    }

    /// A row of an RGISL001 tick: the rider's side, the rigid body of each side (None: the city, a fixed object, the
    /// rider), whether the record is the RGD1 snapshot's (a row in no PAIR record: a constraint's), and the record.
    struct IslRow2 {
        bodies: [Option<usize>; 2],
        snap: bool,
        c: IslContact,
    }

    struct IslTick2 {
        tick: u32,
        bounce: u32,
        n: u32,
        rows: Vec<IslRow2>,
        hinge_parts: Vec<i32>,
        bodies: Vec<IslBody>,
    }

    fn isl2_ticks(b: &[u8]) -> (Vec<i32>, Vec<IslTick2>) {
        assert_eq!(&b[..8], b"RGISL001");
        let w: Vec<u32> = b[8..].chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let (ticks, mut i) = (w[0] as usize, 1);
        let n = w[i] as usize;
        let map: Vec<i32> = w[i + 1..i + 1 + n].iter().map(|&x| x as i32).collect();
        i += 1 + n;
        let mut out = Vec::new();
        for _ in 0..ticks {
            let len = w[i] as usize;
            let t = &w[i + 1..i + 1 + len];
            i += 1 + len;
            let mut k = 4;
            let mut rows = Vec::new();
            let body = |x: u32| if x == u32::MAX { None } else { Some(x as usize) };
            for _ in 0..t[3] {
                let c = IslContact { side: t[k], parts: [t[k + 3] as i32, t[k + 4] as i32], w: t[k + 6..k + 50].try_into().unwrap(), prev_tick: t[k + 50], prev: t[k + 51..k + 63].try_into().unwrap() };
                rows.push(IslRow2 { bodies: [body(t[k + 1]), body(t[k + 2])], snap: t[k + 5] != 0, c });
                k += 63;
            }
            let hinge_parts = t[k + 1..k + 1 + t[k] as usize].iter().map(|&x| x as i32).collect();
            k += 1 + t[k] as usize;
            let nb = t[k] as usize;
            k += 1;
            let mut bodies = Vec::new();
            for _ in 0..nb {
                bodies.push(IslBody { level: t[k], col0: t[k + 1..k + 177].to_vec(), col1: t[k + 177..k + 353].to_vec(), matrix1: t[k + 369..k + 385].to_vec() });
                k += 385;
            }
            out.push(IslTick2 { tick: t[0], bounce: t[1], n: t[2], rows, hinge_parts, bodies });
        }
        (map, out)
    }

    /// A rigid body as the island takes it (as [`shared_bike`]): its collider at slot 10's entry and its instance's rows.
    fn isl2_body(b: &IslBody) -> solver::Body {
        let (w, m) = (&b.col1, &b.matrix1);
        solver::Body { at: wv(w, 256), rows: [wv(m, 0), wv(m, 16), wv(m, 32)], vel: wv(w, 272), ang: wv(w, 288), inv_mass: wf(w, 196), inv_moments: wv(w, 176), force: wv(w, 336), torque: wv(w, 352) }
    }

    /// The island's rows: the rider's of type 7 / 13 with the other side's body where it has one; the others rigid rows
    /// between their bodies (a constraint's of its kind). The warm start of a PAIR row as [`shared_touches`]; of a row in no
    /// PAIR record (a constraint's, or a sub-manifold the record did not carry) its record at the RGD0 snapshot (export_isl.py
    /// writes it as the tick before's).
    fn isl2_touches(map: &[i32], n_links: usize, t: &IslTick2) -> Vec<solver::Touch> {
        let mut out = Vec::new();
        for r in &t.rows {
            let c = &r.c;
            let f = |o: usize| f32::from_bits(c.w[o / 4]);
            let v = |o: usize| Vec3::new(f(o), f(o + 4), f(o + 8));
            let side = |o: usize, s: usize| solver::Side { body: r.bodies[s], point: v(o), ..Default::default() };
            let jointed = match c.side {
                3 => Some(solver::JointedSide { body: 0, link: part_link(map, n_links, c.parts[0]), on_b: false, point_w: f(60), self_b: Some((part_link(map, n_links, c.parts[1]), f(76))) }),
                0 | 1 => {
                    let on_b = c.side == 1;
                    Some(solver::JointedSide { body: 0, link: part_link(map, n_links, c.parts[c.side as usize]), on_b, point_w: if on_b { f(76) } else { f(60) }, self_b: None })
                }
                _ => None,
            };
            let pw = |i: usize| f32::from_bits(c.prev[i]);
            let d = v(16) - Vec3::new(pw(8), pw(9), pw(10));
            let near = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)) < f32::from_bits(0x39d1_b717);
            let prev = |i: usize| if c.prev_tick + 1 != t.tick || !near { Vec3::ZERO } else { Vec3::new(pw(i), pw(i + 1), pw(i + 2)) };
            out.push(solver::Touch {
                depth: f(12),
                friction: f(28),
                elasticity: f(44),
                a: side(48, 0),
                b: side(64, 1),
                normal: v(80),
                back: v(96),
                target: v(144),
                kind: (c.w[41] >> 24) as u8,
                jointed,
                kept: prev(4),
                pushed: prev(0),
                ..Default::default()
            });
        }
        out
    }

    /// The island of `fixture` (RGISL001) with `run` (ops 'i'): the rider RGX0 -> RGD1 word for word, the elasticity factor
    /// (isl +40), every row's kept multipliers (+128) and impulse (+112), and every rigid body's share - what the island's
    /// apply adds to its accumulators +368 / +384 between the snapshots - bit for bit. Answers the report and the count.
    fn many_island(fixture: &[u8], run: &[u8], open: &[u32]) -> (Vec<String>, usize) {
        let (map, ticks) = isl2_ticks(fixture);
        let cases = run_cases(run);
        let mut report = Vec::new();
        let mut n = 0;
        for t in &ticks {
            let Some(c) = cases.iter().find(|c| c.op == b'i' && c.tick == t.tick) else { continue };
            n += 1;
            let mut a = load(&c.inp);
            for &p in &t.hinge_parts {
                a.contact_on_part(&map, p);
            }
            let mut touches = isl2_touches(&map, a.n(), t);
            assert_eq!(touches.len(), t.n as usize);
            let bodies: Vec<solver::Body> = t.bodies.iter().map(isl2_body).collect();
            let dt = c.v1;
            let mut jointed = [a];
            let solved = solver::island_solve_jointed(&bodies, &touches, &mut jointed, (0, 0.0), dt, 1.0 / dt);
            solver::island_apply_jointed(&solved, &mut jointed);
            let [mut a] = jointed;
            a.motion_measure(dt);
            let mut bad = Vec::new();
            if solved.bounce.to_bits() != t.bounce {
                bad.push(format!("bounce {:08x} want {:08x}", solved.bounce.to_bits(), t.bounce));
            }
            solver::island_impulses(&solved, &mut touches);
            for (k, (x, r)) in touches.iter().zip(t.rows.iter()).enumerate() {
                if !vec_words(x.kept, &r.c.w[32..35]) {
                    bad.push(format!("row {k} (side {}, kind {}) kept {:?} want {:08x?}", r.c.side, x.kind, x.kept, &r.c.w[32..35]));
                }
                if !vec_words(x.impulse, &r.c.w[28..31]) {
                    bad.push(format!("row {k} (side {}, kind {}) impulse {:?} want {:08x?}", r.c.side, x.kind, x.impulse, &r.c.w[28..31]));
                }
            }
            let given = solver::island_apply(&bodies, &touches);
            for (bi, b) in t.bodies.iter().enumerate() {
                for (name, o, got) in [("impulse +368", 368, given[bi].0), ("turning +384", 384, given[bi].1)] {
                    if wv(&b.col0, o) != Vec3::ZERO || !vec_words(got, &b.col1[o / 4..o / 4 + 3]) {
                        bad.push(format!("body {bi} (level {:#x}) {name}: island {got:?}, recorded {:?} -> {:?}", b.level, wv(&b.col0, o), wv(&b.col1, o)));
                    }
                }
            }
            let (words, nan) = strict(&a, &c.out);
            eprintln!("island of {} bodies and the rider, tick {}: {} rows ({} of the rider, {} from the snapshot), {} differing words, {nan} NaN on both sides, {} row / body results differ", t.bodies.len(), t.tick, touches.len(), t.rows.iter().filter(|r| r.c.side != 2).count(), t.rows.iter().filter(|r| r.snap).count(), words.len(), bad.len());
            bad.extend(words);
            if open.contains(&t.tick) {
                if bad.is_empty() {
                    report.push(format!("tick {}: exact now - take it off the open list", t.tick));
                }
            } else if !bad.is_empty() {
                report.push(format!("tick {}: {} differences, first {:#?}", t.tick, bad.len(), &bad[..bad.len().min(16)]));
            }
        }
        (report, n)
    }

    /// The rider thrown onto a moving semi from behind: yc.0 813 - 817 (recomp recorder, command `R`: trailer 0x56e3, the
    /// body 0x571d touching it, the bike 0x5732, the rider 0x5770). One island of three rigid bodies and the rider, rows
    /// of every pair (the bike's with the city and the trailer, the trailer's with 0x571d, the rider's with all), two of
    /// them in no PAIR record (read from slot 10's snapshot). The island's velocity solve with every body: the rider
    /// RGX0 -> RGD1 word for word, isl +40, every row's +128 / +112 and every body's +368 / +384.
    #[test]
    fn rider_semi_island_velocity_solve_is_the_originals() {
        let (report, n) = many_island(include_bytes!("testdata/rider_semi_yc0.bin"), include_bytes!("testdata/rider_semi_yc0_run.bin"), &OPEN_SEMI);
        assert_eq!(n, 5);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    /// yc.0 ticks not yet word for word (open).
    const OPEN_SEMI: [u32; 0] = [];
}
