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
// Tests: islands of the recording with joint-limit rows only

#[cfg(test)]
mod tests {
    use super::super::tests::{check, load, run_cases, Case};
    use super::*;
    use crate::physics::solver;

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
    /// with a zero multiplier leave when they hand it out through slot 34 (F-19400, not ported:
    /// 0 + (-0) and r x 0 with a stale lane w).
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
        let solved = solver::island_solve_jointed(&[], &[], &mut jointed, dt, 1.0 / dt);
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
        assert!(report.is_empty(), "{}", report.join("
"));
    }
}
