//! The original's contact solver for one rigid body against the fixed world, operation for
//! operation: the rows of an island's contacts (0x825DAD20), the sweeps over them (0x825E8F88
//! with the kernel 0x825E51E8), the impulses they end in (0x825DCA18), and the second solve
//! that only moves the body out of what it stands in (0x825DCEB8, 0x825DDCE8, the body's
//! 0x8259DEA8 / 0x8259DC58 / 0x8259E790). Single precision, the original's order of sums; the
//! vector unit's multiply-add is two roundings, the scalar unit's one.
//!
//! Checked bit for bit against the original code run offline (tools/solver_reference.py; the
//! test `the_contact_solver_is_the_originals_bit_for_bit`). Decode notes: work/veh/solver_rule.md
//! and work/veh/gjk_exact.md part 5.

use super::exact::{cross, dot3, fma, inverse3, recip, world_inverse_inertia};
use glam::Vec3;

/// How deep a contact may stand (0x828FF840, set by 0x827B4850 from 0x82082810).
pub const ALLOW: f32 = f32::from_bits(0x3be5_6042);
/// 0x82084820
const SKIN: f32 = f32::from_bits(0x3a83_126f);
/// The velocity solve's relaxation (0x82813588); the push solve's is 1.
const OMEGA: f32 = f32::from_bits(0x3f8c_cccd);
/// 0x82084FE0
const TINY: f32 = f32::from_bits(0x322b_cc77);
/// 0x82007A3C
const SMALL: f32 = f32::from_bits(0x3727_c5ac);
/// "no thickness" (0x82081BF8)
const NONE: f32 = f32::from_bits(0x7149_f2ca);
/// The sweeps of either solve (0x8281358C as the game sets it, 0x82590660).
pub const SWEEPS: usize = 4;
/// R3: the sweeps of an island that holds a jointed body - a thrown rider - (0x825DC5E8 / 0x825DC6C0: island
/// +20 not 0 takes 0x82813590, which nothing writes: the image's 8).
pub const SWEEPS_JOINTED: usize = 8;
/// R3: the sweeps of the island being solved, where it is not the plain one (0 = SWEEPS).
pub static SWEEPS_NOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// R3: the enabled contacts of the island that are not the car's own - the thrown rider's against the ground
/// and himself: how many, and their elasticities' sum (bits). They count in the island's mean elasticity
/// (island +40), which every contact's impulse and pending term is scaled by.
pub static ISLAND_OTHERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static ISLAND_OTHERS_E: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
fn sweeps_now() -> usize {
    match SWEEPS_NOW.load(std::sync::atomic::Ordering::Relaxed) { 0 => SWEEPS, n => n }
}

/// The body as the solver reads it.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    /// its place (+256): the centre the contacts turn it about
    pub at: Vec3,
    pub rows: [Vec3; 3],
    pub vel: Vec3,
    pub ang: Vec3,
    pub inv_mass: f32,
    /// the reciprocals of its moments (+176)
    pub inv_moments: Vec3,
    /// the sums the tick's forces have left (+336, +352)
    pub force: Vec3,
    pub torque: Vec3,
}

/// One contact of the body with the fixed world (the original's 176-byte record).
#[derive(Clone, Copy, Debug, Default)]
pub struct Contact {
    /// +12
    pub depth: f32,
    /// +28, +44
    pub friction: f32,
    pub elasticity: f32,
    /// the body's point of it in the body's (the instance's) frame (+16) and in the world (+48)
    pub local: Vec3,
    pub on_body: Vec3,
    /// the world's point (+32 = +64: the world does not move)
    pub on_world: Vec3,
    /// the way the body is pushed (+80)
    pub normal: Vec3,
    /// the push solve's multipliers (+0), the velocity solve's (+128): where the next tick's begin
    pub pushed: Vec3,
    pub kept: Vec3,
    /// the velocity asked of the point (+144)
    pub target: Vec3,
    /// the impulse the velocity solve gave (+112)
    pub impulse: Vec3,
}

/// A contact as the sweeps have it (the 272-byte row).
#[derive(Clone, Copy, Debug)]
struct Row {
    /// minus the multipliers so far (+0)
    neg: Vec3,
    bias: Vec3,
    basis: [Vec3; 3],
    /// the basis by columns (+32), the arms across it by columns (+144)
    bcol: [Vec3; 3],
    arm: [Vec3; 3],
    jcol: [Vec3; 3],
    /// the inverse of the block (+96), by columns
    kcol: [Vec3; 3],
    c: Vec3,
    mu: f32,
}

/// The body's velocities as the sweeps move them (the 128-byte state).
#[derive(Clone, Copy, Debug)]
struct State {
    vel: Vec3,
    ang: Vec3,
    mcol: [Vec3; 3],
    icol: [Vec3; 3],
}

fn columns(m: &[Vec3; 3]) -> [Vec3; 3] {
    [Vec3::new(m[0].x, m[1].x, m[2].x), Vec3::new(m[0].y, m[1].y, m[2].y), Vec3::new(m[0].z, m[1].z, m[2].z)]
}

/// The two directions across a contact's normal (0x821CE640).
pub fn basis(n: Vec3) -> [Vec3; 3] {
    let b1 = if n.x.abs() < n.y.abs() { Vec3::new(0.0, n.z, -n.y) } else { Vec3::new(-n.z, 0.0, n.x) };
    let l2 = dot3(b1, b1);
    let mut e = 1.0 / l2.sqrt();
    if e == f32::INFINITY {
        e = 0.0;
    }
    let b1 = b1 * ((-(l2 * (e * e) - 1.0)) * (0.5 * e) + e);
    [n, b1, cross(n, b1)]
}

/// The inverse of the block in place (0x825DE528): by its lower half, each pivot kept above a
/// floor of its trace's hundred-thousandth.
fn block_inverse(k: &[Vec3; 3]) -> [Vec3; 3] {
    let (m00, m10, m11, m20, m21, m22) = (k[0].x, k[1].x, k[1].y, k[2].x, k[2].y, k[2].z);
    let floor = { let t = (((m11 + m22) + m00) * SMALL).abs(); if TINY - t >= 0.0 { TINY } else { t } };
    let d0 = if floor - m00 >= 0.0 { floor } else { m00 };
    let i0 = 1.0 / d0;
    let l10 = m10 * i0;
    let s11 = -fma(m10, l10, -m11);
    let l20 = m20 * i0;
    let s22 = -fma(m20, l20, -m22);
    let s21 = -fma(m10, l20, -m21);
    let d1 = if floor - s11 >= 0.0 { floor } else { s11 };
    let i1 = 1.0 / d1;
    let l21 = i1 * s21;
    let a = i1 * l10;
    let t22 = -fma(l21, s21, -s22);
    let g = fma(l21, l10, -l20);
    let h = a * l10;
    let d2 = if floor - t22 >= 0.0 { floor } else { t22 };
    let i2 = 1.0 / d2;
    let p = g * i2;
    let q = i2 * l21;
    let pl = p * l21;
    let r11 = -fma(-q, l21, -i1);
    let r01 = -pl - a;
    let r00 = fma(p, g, h) + i0;
    [Vec3::new(r00, r01, p), Vec3::new(r01, r11, -q), Vec3::new(p, -q, i2)]
}

impl State {
    fn of(body: &Body) -> State {
        let m = body.inv_mass;
        State {
            vel: body.vel,
            ang: body.ang,
            mcol: columns(&[Vec3::new(m, 0.0, 0.0), Vec3::new(0.0, m, 0.0), Vec3::new(0.0, 0.0, m)]),
            icol: columns(&world_inverse_inertia(&body.rows, body.inv_moments)),
        }
    }

    /// What a step of a row's multipliers does to the velocities.
    fn give(&mut self, row: &Row, step: Vec3) {
        let push = Vec3::new(dot3(row.bcol[0], step), dot3(row.bcol[1], step), dot3(row.bcol[2], step));
        self.vel = self.vel + Vec3::new(dot3(self.mcol[0], push), dot3(self.mcol[1], push), dot3(self.mcol[2], push));
        let turn = Vec3::new(dot3(row.jcol[0], step), dot3(row.jcol[1], step), dot3(row.jcol[2], step));
        self.ang = self.ang + Vec3::new(dot3(self.icol[0], turn), dot3(self.icol[1], turn), dot3(self.icol[2], turn));
    }
}

impl Row {
    /// 0x825E4FE8, 0x825E5110 (0x825E4C40, 0x825DE528).
    fn new(body: &Body, state: &State, c: &Contact) -> Row {
        let basis = basis(c.normal);
        let r = c.on_body - body.at;
        let arm = [cross(r, basis[0]), cross(r, basis[1]), cross(r, basis[2])];
        let (bcol, jcol) = (columns(&basis), columns(&arm));
        // (the block: what an impulse along each direction does to the point's velocity along each)
        let rows_of = |cols: &[Vec3; 3]| columns(cols);
        let (mrow, irow) = (rows_of(&state.mcol), rows_of(&state.icol));
        let t: [Vec3; 3] = std::array::from_fn(|k| (bcol[0] * mrow[k].x + bcol[1] * mrow[k].y) + bcol[2] * mrow[k].z);
        let w: [Vec3; 3] = std::array::from_fn(|k| (jcol[0] * irow[k].x + jcol[1] * irow[k].y) + jcol[2] * irow[k].z);
        let k: [Vec3; 3] = std::array::from_fn(|i| {
            let lin = (t[0] * basis[i].x + t[1] * basis[i].y) + t[2] * basis[i].z;
            let ang = (w[0] * arm[i].x + w[1] * arm[i].y) + w[2] * arm[i].z;
            ang + lin
        });
        let cc = if k[0].x == 0.0 { Vec3::X } else { k[0] * recip(k[0].x) };
        Row { neg: Vec3::ZERO, bias: Vec3::ZERO, basis, bcol, arm, jcol, kcol: columns(&block_inverse(&k)), c: cc, mu: c.friction }
    }

    /// One visit of the kernel (0x825E51E8); returns the step's squares.
    fn visit(&mut self, s: &mut State, omega: f32) -> Vec3 {
        let lin = Vec3::new(dot3(self.basis[0], s.vel), dot3(self.basis[1], s.vel), dot3(self.basis[2], s.vel));
        let ang = Vec3::new(dot3(self.arm[0], s.ang), dot3(self.arm[1], s.ang), dot3(self.arm[2], s.ang));
        let u = (((ang - Vec3::ZERO) + lin) - Vec3::ZERO) - self.bias;
        let ku = Vec3::new(dot3(self.kcol[0], u), dot3(self.kcol[1], u), dot3(self.kcol[2], u));
        let x = ((Vec3::ZERO - ku) * omega) - self.neg;
        let t = Vec3::new(x.x * 0.0, x.y, x.z);
        let nn = dot3(self.c, x);
        let d = x - Vec3::new(nn, nn * 0.0, nn * 0.0);
        let t2 = dot3(t, t);
        let len = if t2 == 0.0 {
            0.0
        } else {
            let e = 1.0 / t2.sqrt();
            t2 * ((-(t2 * (e * e) - 1.0)) * (0.5 * e) + e)
        };
        let lm = len * self.mu;
        let g = Vec3::new(-(lm - t.x), -(lm * 0.0 - t.y), -(lm * 0.0 - t.z));
        let (dg, xg) = (dot3(d, g), dot3(x, g));
        let k = if xg > 0.0 { xg * recip(dg + TINY) } else { 0.0 };
        let next = if 0.0 > nn { Vec3::ZERO } else { -(d * k - x) };
        let step = next + self.neg;
        s.give(self, step);
        self.neg = self.neg - step;
        step * step
    }
}

/// 0x825E8F88: what the rows start with is given first (0x825E8B90), then the sweeps, until one
/// changes nothing.
fn sweeps(rows: &mut [Row], s: &mut State, omega: f32) {
    for i in 0..rows.len() {
        let step = Vec3::ZERO - rows[i].neg;
        let row = rows[i];
        s.give(&row, step);
    }
    for _ in 0..sweeps_now() {
        let mut left = Vec3::ZERO;
        for row in rows.iter_mut() {
            left = row.visit(s, omega) + left;
        }
        if 0.0 >= dot3(left, left) {
            break;
        }
    }
}

/// What the tick's forces will add to the velocity of the body's point `p` (0x8259D528).
fn pending(body: &Body, p: Vec3, dt: f32) -> Vec3 {
    let lin = body.force * (body.inv_mass * dt);
    if body.torque == Vec3::ZERO {
        return lin;
    }
    let t = body.torque * dt;
    let local = Vec3::new(dot3(body.rows[0], t), dot3(body.rows[1], t), dot3(body.rows[2], t)) * body.inv_moments;
    let cols = columns(&body.rows);
    let dw = Vec3::new(dot3(cols[0], local), dot3(cols[1], local), dot3(cols[2], local));
    let r = p - body.at;
    let yzx = |a: Vec3| Vec3::new(a.y, a.z, a.x);
    let zxy = |a: Vec3| Vec3::new(a.z, a.x, a.y);
    -(zxy(dw) * yzx(r) - (yzx(dw) * zxy(r) + lin))
}

/// What the velocity solve leaves besides the contacts' own numbers.
pub struct Solved {
    rows: Vec<Row>,
    state: State,
    /// 1 + the contacts' mean elasticity (island +40)
    pub bounce: f32,
    /// the impulses' sums as the body's accumulators take them (+368, +384)
    pub impulse: Vec3,
    pub turning: Vec3,
}

/// The velocity solve (0x825DE200). The contacts' `kept` and `impulse` are written.
pub fn velocity(body: &Body, contacts: &mut [Contact], dt: f32, inv_dt: f32) -> Solved {
    let mut sum = 0.0f32;
    for c in contacts.iter() {
        sum = c.elasticity + sum;
    }
    let others = ISLAND_OTHERS.load(std::sync::atomic::Ordering::Relaxed);
    if others > 0 {
        sum = f32::from_bits(ISLAND_OTHERS_E.load(std::sync::atomic::Ordering::Relaxed)) + sum;
    }
    let share = if contacts.is_empty() { 1.0 } else { 1.0 / (contacts.len() + others) as f32 };
    let bounce = fma(share, sum, 1.0);
    let less = 1.0 / bounce;
    let mut state = State::of(body);
    let mut rows: Vec<Row> = contacts
        .iter()
        .map(|c| {
            let mut row = Row::new(body, &state, c);
            let p = (pending(body, c.on_body, dt) * less) - Vec3::ZERO;
            let g = (ALLOW - SKIN) - c.depth;
            let gap = if -g >= 0.0 { 0.0 } else { g };
            let b = &row.basis;
            let x0 = fma(gap, inv_dt, fma(p.z, b[0].z, fma(p.x, b[0].x, p.y * b[0].y)));
            let x1 = fma(b[1].z, p.z, fma(b[1].x, p.x, b[1].y * p.y));
            let x2 = fma(b[2].z, p.z, fma(b[2].x, p.x, b[2].y * p.y));
            row.bias = (Vec3::ZERO - Vec3::new(x0, x1, x2)) + c.target;
            row.neg = Vec3::ZERO - c.kept;
            row
        })
        .collect();
    sweeps(&mut rows, &mut state, OMEGA);
    let (mut impulse, mut turning) = (Vec3::ZERO, Vec3::ZERO);
    for (row, c) in rows.iter().zip(contacts.iter_mut()) {
        let p = Vec3::ZERO - row.neg;
        c.kept = p;
        let inv = inverse3(&row.bcol);
        c.impulse = ((inv[0] * p.x + inv[1] * p.y) + inv[2] * p.z) * bounce;
        // (the body's 0x8259DE60)
        impulse = impulse + c.impulse;
        turning = turning + cross(c.on_body - body.at, c.impulse);
    }
    Solved { rows, state, bounce, impulse, turning }
}

/// What the push solve leaves: the body's push (+400) and turn (+416), and whether a contact
/// stood deeper than the body is thick there (byte +658 is then set and +659 counted up).
#[derive(Clone, Copy, Debug, Default)]
pub struct Pushed {
    pub push: Vec3,
    pub turn: Vec3,
    pub deep: bool,
}

/// A unit vector or, of nothing, the first axis (the thickness's direction).
fn unit_or_x(v: Vec3) -> Vec3 {
    let l2 = dot3(v, v);
    if l2 > 0.0 {
        let e = 1.0 / l2.sqrt();
        v * ((-(l2 * (e * e) - 1.0)) * (0.5 * e) + e)
    } else {
        Vec3::X
    }
}

/// The body's direct push (0x8259DC58): `by` added to what is pending, less its share along
/// what is pending already; no longer than 100 m.
pub fn direct(pending: Vec3, by: Vec3) -> Vec3 {
    let l2 = dot3(by, by);
    let cap = 100.0f32;
    let by = if l2 > cap * cap {
        let e = 1.0 / l2.sqrt();
        (by * ((-(l2 * (e * e) - 1.0)) * (0.5 * e) + e)) * cap
    } else {
        by
    };
    let p2 = dot3(pending, pending);
    let inv = { let r = 1.0 / p2; r * (-(r * p2 - 1.0)) + r };
    let along = dot3(pending, by);
    let add = if along > 0.0 && inv > 0.0 { -(pending * (along * inv) - by) } else { by };
    pending + add
}

/// The push solve (0x825DDCB0 and 0x825DDCE8) on the rows of the velocity solve. `margins`:
/// the body's bound's and the world's (bound +112 of the car's composite and of the city's
/// grid). The contacts' `pushed` is written.
pub fn push(body: &Body, contacts: &mut [Contact], solved: &mut Solved, dt: f32, inv_dt: f32, margins: (f32, f32), was_deep: bool, turn_left: Vec3) -> Pushed {
    // (`turn_left`: what body +416 still holds - a turn too small to be applied stays there, 0x8259E790)
    let mut out = Pushed { turn: turn_left, ..Pushed::default() };
    let mut deep = was_deep;
    for (row, c) in solved.rows.iter_mut().zip(contacts.iter()) {
        let d = c.depth - ALLOW;
        let half = d * 0.5;
        let m = if half - ALLOW >= 0.0 { ALLOW } else { half };
        let m = if -m >= 0.0 { 0.0 } else { m };
        let mut ask = d - m;
        // (how thick each bound is beyond the contact along its normal: of a composite and of
        // a grid, the margin about the bound's own origin)
        let thick = |n: Vec3, margin: f32, local: Vec3| {
            let q = (unit_or_x(n) * margin + Vec3::ZERO) - local;
            let t = fma(q.z, n.z, fma(n.x, q.x, q.y * n.y));
            if t >= 0.0 { t } else { 0.0 }
        };
        let nl = Vec3::new(dot3(body.rows[0], c.normal), dot3(body.rows[1], c.normal), dot3(body.rows[2], c.normal));
        let ta = thick(nl, margins.0, c.local);
        let back = Vec3::ZERO - c.normal;
        let nw = Vec3::new(dot3(Vec3::X, back), dot3(Vec3::Y, back), dot3(Vec3::Z, back));
        let tb = thick(nw, margins.1, c.on_world);
        let t = if ta - tb >= 0.0 { tb } else { ta };
        if t != NONE && ask > 0.0 && ask > t {
            if !deep {
                deep = true;
                out.deep = true;
            }
            let rest = t * 0.5;
            let by = c.normal * (ask - rest);
            // (the body's share against the fixed world: all of it)
            let share = { let s = (0.0f32 / (body.inv_mass * body.inv_mass)).sqrt(); 1.0 / (s + 1.0) };
            out.push = direct(out.push, by * share);
            ask = rest;
        }
        row.bias = Vec3::new(ask * inv_dt, 0.0, 0.0);
        row.neg = Vec3::ZERO - c.pushed;
    }
    solved.state.vel = Vec3::ZERO;
    solved.state.ang = Vec3::ZERO;
    sweeps(&mut solved.rows, &mut solved.state, 1.0);
    let icol = solved.state.icol;
    for (row, c) in solved.rows.iter().zip(contacts.iter_mut()) {
        let q = Vec3::ZERO - row.neg;
        c.pushed = q;
        let inv = inverse3(&row.bcol);
        let v = (inv[0] * q.x + inv[1] * q.y) + inv[2] * q.z;
        // (the body's 0x8259DEA8)
        out.push = v * (body.inv_mass * dt) + out.push;
        let t = cross(c.on_body - body.at, v);
        out.turn = Vec3::new(dot3(icol[0], t), dot3(icol[1], t), dot3(icol[2], t)) * dt + out.turn;
    }
    out
}

/// Sine and cosine as the vector unit's own routine has them (inside 0x8259E480): the angle
/// less the nearest quarter turn, a polynomial each.
pub(crate) fn sin_cos(x: f32) -> (f32, f32) {
    let f = f32::from_bits;
    let turns = x * f(0x3f22_f983);
    let q = (turns + 0.5f32.copysign(turns)) as i32;
    let qf = q as f32;
    let r = -(qf * f(0x33a2_2169) - (-(qf * f(0x3fc9_0fda) - x)));
    let r2 = r * r;
    let cos = ((f(0xbab2_4993) * r2 + f(0x3d2a_a036)) * r2 + f(0xbeff_ffdf)) * r2 + 1.0;
    let sin = ((f(0xb94c_8c6e) * r2 + f(0x3c08_8342)) * r2 + f(0xbe2a_aaa1)) * (r2 * r) + r;
    let (k, k1) = (q & 3, (q & 3) + 1);
    let s = if k & 1 == 0 { sin } else { cos };
    let c = if k1 & 1 == 0 { sin } else { cos };
    (if k & 2 == 0 { s } else { -s }, if k1 & 2 == 0 { c } else { -c })
}

/// The rotation by `angle` about the unit vector `axis` as the push's turn builds it (0x8259E480), by rows.
fn rotation(axis: Vec3, angle: f32) -> [Vec3; 3] {
    let (s, c) = sin_cos(angle);
    let a = axis * s;
    let n = Vec3::ZERO - a;
    let t = 1.0 - c;
    [(axis * axis.x) * t + Vec3::new(c, a.z, n.y), (axis * axis.y) * t + Vec3::new(n.z, c, a.x), (axis * axis.z) * t + Vec3::new(a.y, n.x, c)]
}

/// 0x8259E790's test: is the turn large enough to be applied (and cleared)? One that is not stays in
/// body +416 and the next tick's is added to it.
pub fn turn_applies(turn: Vec3) -> bool {
    dot3(turn, turn) > f32::from_bits(0x2b8c_bccc)
}

/// The body's rows with the push solve's turn applied (0x8259E790): nothing below a millionth
/// of a radian.
pub fn rows_pushed(rows: &[Vec3; 3], turn: Vec3) -> [Vec3; 3] {
    let l2 = dot3(turn, turn);
    if !turn_applies(turn) {
        return *rows;
    }
    let e = 1.0 / l2.sqrt();
    let angle = l2 * ((-(l2 * (e * e) - 1.0)) * (0.5 * e) + e);
    let r = rotation(turn * recip(angle), angle);
    std::array::from_fn(|i| (r[0] * rows[i].x + r[1] * rows[i].y) + r[2] * rows[i].z)
}

// ---------------------------------------------------------------------------------------------
// The same solver for an island of several bodies (work/veh/gjkfix/p1/part4_solver.md): every row
// names two states - a side without a body (the city, a prop that has not come loose) has the
// one that is all zeros - and what the one-body form above has as literal zeros are the second
// body's terms here. Used when a car's island holds a prop or a piece of one.

/// One side of an island's contact.
#[derive(Clone, Copy, Debug, Default)]
pub struct Side {
    /// which of the island's bodies; None: an object without one
    pub body: Option<usize>,
    /// its point in the world (+48 / +64) and in its part's frame (+16 / +32)
    pub point: Vec3,
    pub local: Vec3,
    /// its instance's rows, and its whole bound's margin about the bound's origin (a composite's,
    /// a grid's: +112) for the push solve's thickness
    pub rows: [Vec3; 3],
    pub margin: f32,
    /// the whole bound's core point along the normal (zero for a composite or a grid; a polyhedron's corner)
    pub core: Vec3,
}

/// A contact between two objects of an island (the 176-byte record).
#[derive(Clone, Copy, Debug, Default)]
pub struct Touch {
    pub depth: f32,
    pub friction: f32,
    pub elasticity: f32,
    pub a: Side,
    pub b: Side,
    /// the way A is pushed (+80), and the other way as stored (+96)
    pub normal: Vec3,
    pub back: Vec3,
    pub pushed: Vec3,
    pub kept: Vec3,
    pub target: Vec3,
    pub impulse: Vec3,
    /// +164: 0 a contact; a constraint's (0x825B6F78): 2 a rope (a length of 0.01 or more), 1 a
    /// point (a 256-byte row of its own, 0x825DBFA4)
    pub kind: u8,
}

#[derive(Clone, Copy, Debug)]
struct Row2 {
    row: Row,
    arm_b: [Vec3; 3],
    jcol_b: [Vec3; 3],
    /// the states of the two sides (0: the one of zeros)
    sa: usize,
    sb: usize,
    kind: u8,
}

const NO_STATE: State = State { vel: Vec3::ZERO, ang: Vec3::ZERO, mcol: [Vec3::ZERO; 3], icol: [Vec3::ZERO; 3] };

impl Row2 {
    fn new(bodies: &[Body], states: &[State], c: &Touch) -> Row2 {
        let basis = basis(c.normal);
        let place = |s: &Side| s.body.map_or(Vec3::ZERO, |i| bodies[i].at);
        let (ra, rb) = (c.a.point - place(&c.a), c.b.point - place(&c.b));
        let arm = [cross(ra, basis[0]), cross(ra, basis[1]), cross(ra, basis[2])];
        let arm_b = [cross(rb, basis[0]), cross(rb, basis[1]), cross(rb, basis[2])];
        let (bcol, jcol, jcol_b) = (columns(&basis), columns(&arm), columns(&arm_b));
        let (sa, sb) = (c.a.body.map_or(0, |i| i + 1), c.b.body.map_or(0, |i| i + 1));
        let through = |s: &State, j: &[Vec3; 3]| {
            let (mrow, irow) = (columns(&s.mcol), columns(&s.icol));
            let t: [Vec3; 3] = std::array::from_fn(|k| (bcol[0] * mrow[k].x + bcol[1] * mrow[k].y) + bcol[2] * mrow[k].z);
            let w: [Vec3; 3] = std::array::from_fn(|k| (j[0] * irow[k].x + j[1] * irow[k].y) + j[2] * irow[k].z);
            (t, w)
        };
        let (ta, wa) = through(&states[sa], &jcol);
        let (tb, wb) = through(&states[sb], &jcol_b);
        let k: [Vec3; 3] = std::array::from_fn(|i| {
            let la = (ta[0] * basis[i].x + ta[1] * basis[i].y) + ta[2] * basis[i].z;
            let aa = (wa[0] * arm[i].x + wa[1] * arm[i].y) + wa[2] * arm[i].z;
            let lb = (tb[0] * basis[i].x + tb[1] * basis[i].y) + tb[2] * basis[i].z;
            let ab = (wb[0] * arm_b[i].x + wb[1] * arm_b[i].y) + wb[2] * arm_b[i].z;
            // (a point constraint's block, 0x825E5CC0: B's linear term before its angular one)
            if c.kind == 1 { ((aa + la) + lb) + ab } else { ((aa + la) + ab) + lb }
        });
        if c.kind == 1 {
            // (no c row, no block of ten thousands: 0x825DE528 on the block as it is)
            return Row2 { row: Row { neg: Vec3::ZERO, bias: Vec3::ZERO, basis, bcol, arm, jcol, kcol: columns(&block_inverse(&k)), c: Vec3::X, mu: 0.0 }, arm_b, jcol_b, sa, sb, kind: 1 };
        }
        let cc = if k[0].x == 0.0 { Vec3::X } else { k[0] * recip(k[0].x) };
        // (0x825E5110: a row between two sides without bodies is given a block of ten thousands)
        let kcol = if k[0].x == 0.0 && k[1].y == 0.0 && k[2].z == 0.0 { [Vec3::splat(10000.0); 3] } else { columns(&block_inverse(&k)) };
        Row2 { row: Row { neg: Vec3::ZERO, bias: Vec3::ZERO, basis, bcol, arm, jcol, kcol, c: cc, mu: c.friction }, arm_b, jcol_b, sa, sb, kind: c.kind }
    }

    /// What a step of the row's multipliers does to the two bodies' velocities.
    fn give(&self, states: &mut [State], step: Vec3) {
        let r = &self.row;
        let push = Vec3::new(dot3(r.bcol[0], step), dot3(r.bcol[1], step), dot3(r.bcol[2], step));
        let turn_a = Vec3::new(dot3(r.jcol[0], step), dot3(r.jcol[1], step), dot3(r.jcol[2], step));
        let turn_b = Vec3::new(dot3(self.jcol_b[0], step), dot3(self.jcol_b[1], step), dot3(self.jcol_b[2], step));
        let through = |m: &[Vec3; 3], x: Vec3| Vec3::new(dot3(m[0], x), dot3(m[1], x), dot3(m[2], x));
        let (dva, dvb) = (through(&states[self.sa].mcol, push), through(&states[self.sb].mcol, push));
        let (dwa, dwb) = (through(&states[self.sa].icol, turn_a), through(&states[self.sb].icol, turn_b));
        states[self.sa].vel = states[self.sa].vel + dva;
        states[self.sb].vel = states[self.sb].vel - dvb;
        states[self.sa].ang = states[self.sa].ang + dwa;
        states[self.sb].ang = states[self.sb].ang - dwb;
    }

    fn visit(&mut self, states: &mut [State], omega: f32) -> Vec3 {
        let r = &self.row;
        let (a, b) = (states[self.sa], states[self.sb]);
        let lin_a = Vec3::new(dot3(r.basis[0], a.vel), dot3(r.basis[1], a.vel), dot3(r.basis[2], a.vel));
        let lin_b = Vec3::new(dot3(r.basis[0], b.vel), dot3(r.basis[1], b.vel), dot3(r.basis[2], b.vel));
        let ang_a = Vec3::new(dot3(r.arm[0], a.ang), dot3(r.arm[1], a.ang), dot3(r.arm[2], a.ang));
        let ang_b = Vec3::new(dot3(self.arm_b[0], b.ang), dot3(self.arm_b[1], b.ang), dot3(self.arm_b[2], b.ang));
        if self.kind == 1 {
            // (a point constraint, 0x825E8468 / 0x825E7D78: both ways, no relaxation, no cone)
            let u = ((lin_a + ang_a) - (lin_b + ang_b)) - r.bias;
            let ku = Vec3::new(dot3(r.kcol[0], u), dot3(r.kcol[1], u), dot3(r.kcol[2], u));
            let step = Vec3::ZERO - ku;
            self.give(states, step);
            self.row.neg = self.row.neg - step;
            return step * step;
        }
        let u = (((ang_a - ang_b) + lin_a) - lin_b) - r.bias;
        let ku = Vec3::new(dot3(r.kcol[0], u), dot3(r.kcol[1], u), dot3(r.kcol[2], u));
        let x = ((Vec3::ZERO - ku) * omega) - r.neg;
        let t = Vec3::new(x.x * 0.0, x.y, x.z);
        let nn = dot3(r.c, x);
        let d = x - Vec3::new(nn, nn * 0.0, nn * 0.0);
        let t2 = dot3(t, t);
        let len = if t2 == 0.0 {
            0.0
        } else {
            let e = 1.0 / t2.sqrt();
            t2 * ((-(t2 * (e * e) - 1.0)) * (0.5 * e) + e)
        };
        let lm = len * r.mu;
        let g = Vec3::new(-(lm - t.x), -(lm * 0.0 - t.y), -(lm * 0.0 - t.z));
        let (dg, xg) = (dot3(d, g), dot3(x, g));
        let k = if xg > 0.0 { xg * recip(dg + TINY) } else { 0.0 };
        let next = if 0.0 > nn { Vec3::ZERO } else { -(d * k - x) };
        let step = next + r.neg;
        self.give(states, step);
        self.row.neg = self.row.neg - step;
        step * step
    }
}

fn sweeps2(rows: &mut [Row2], states: &mut [State], omega: f32) {
    for i in 0..rows.len() {
        let step = Vec3::ZERO - rows[i].row.neg;
        let row = rows[i];
        row.give(states, step);
    }
    for _ in 0..sweeps_now() {
        let mut left = Vec3::ZERO;
        for row in rows.iter_mut() {
            left = row.visit(states, omega) + left;
        }
        if 0.0 >= dot3(left, left) {
            break;
        }
    }
}

/// What an island's velocity solve leaves.
pub struct IslandSolved {
    rows: Vec<Row2>,
    states: Vec<State>,
    pub bounce: f32,
}

fn states_of(bodies: &[Body]) -> Vec<State> {
    std::iter::once(NO_STATE).chain(bodies.iter().map(State::of)).collect()
}

/// An island's rows and their sweeps (0x825DAD20, 0x825DC5E8), from the bodies' velocities as
/// they stand and the contacts' kept multipliers. Nothing is written back.
pub fn island_solve(bodies: &[Body], touches: &[Touch], dt: f32, inv_dt: f32) -> IslandSolved {
    let mut sum = 0.0f32;
    for c in touches.iter() {
        sum = c.elasticity + sum;
    }
    let share = if touches.is_empty() { f32::MAX } else { 1.0 / touches.len() as f32 };
    let bounce = fma(share, sum, 1.0);
    let less = 1.0 / bounce;
    let mut states = states_of(bodies);
    let mut rows: Vec<Row2> = touches
        .iter()
        .map(|c| {
            let mut row = Row2::new(bodies, &states, c);
            if c.kind == 1 {
                // (0x825DC0AC: the target minus the two points' pending velocities - not over the island's
                // elasticity - through the basis as its transpose; no warm start, 0x825E7A70)
                let pend = |s: &Side| s.body.map_or(Vec3::ZERO, |i| pending(&bodies[i], s.point, dt));
                let v = c.target - (pend(&c.a) - pend(&c.b));
                let b = &row.row.basis;
                row.row.bias = Vec3::new(dot3(v, Vec3::new(b[0].x, b[1].x, b[2].x)), dot3(v, Vec3::new(b[0].y, b[1].y, b[2].y)), dot3(v, Vec3::new(b[0].z, b[1].z, b[2].z)));
                return row;
            }
            let pend = |s: &Side| s.body.map_or(Vec3::ZERO, |i| pending(&bodies[i], s.point, dt) * less);
            let p = pend(&c.a) - pend(&c.b);
            let g = (ALLOW - SKIN) - c.depth;
            let gap = if -g >= 0.0 { 0.0 } else { g };
            let b = &row.row.basis;
            let x0 = fma(gap, inv_dt, fma(p.z, b[0].z, fma(p.x, b[0].x, p.y * b[0].y)));
            let x1 = fma(b[1].z, p.z, fma(b[1].x, p.x, b[1].y * p.y));
            let x2 = fma(b[2].z, p.z, fma(b[2].x, p.x, b[2].y * p.y));
            row.row.bias = (Vec3::ZERO - Vec3::new(x0, x1, x2)) + c.target;
            row.row.neg = Vec3::ZERO - c.kept;
            row
        })
        .collect();
    sweeps2(&mut rows, &mut states, OMEGA);
    IslandSolved { rows, states, bounce }
}

/// The contacts' multipliers and impulses as the solve left them (0x825919F8; the same numbers
/// 0x825DCA18 hands out): `kept` (+128) and `impulse` (+112, the vector that pushes A).
pub fn island_impulses(solved: &IslandSolved, touches: &mut [Touch]) {
    for (row, c) in solved.rows.iter().zip(touches.iter_mut()) {
        let p = Vec3::ZERO - row.row.neg;
        c.kept = p;
        let inv = inverse3(&row.row.bcol);
        let j = (inv[0] * p.x + inv[1] * p.y) + inv[2] * p.z;
        // (0x825919F8: a constraint's not times the island's elasticity)
        c.impulse = if c.kind == 0 { j * solved.bounce } else { j };
    }
}

/// The impulses given to the bodies' accumulators (0x825A20E8 -> 0x8259DE60): per body what
/// +368 and +384 gain, contact by contact, A then B.
pub fn island_apply(bodies: &[Body], touches: &[Touch]) -> Vec<(Vec3, Vec3)> {
    let mut out = vec![(Vec3::ZERO, Vec3::ZERO); bodies.len()];
    for c in touches {
        if let Some(i) = c.a.body {
            out[i].0 = out[i].0 + c.impulse;
            out[i].1 = out[i].1 + cross(c.a.point - bodies[i].at, c.impulse);
        }
        if let Some(i) = c.b.body {
            let j = Vec3::ZERO - c.impulse;
            out[i].0 = out[i].0 + j;
            out[i].1 = out[i].1 + cross(c.b.point - bodies[i].at, j);
        }
    }
    out
}

/// The push solve of an island (0x825DCEB8, 0x825DC6C0, 0x825DDCE8) on the rows of its last
/// velocity solve. `pushes`: per body what +400 / +416 hold as it begins and end with, and
/// whether a contact of it stood deeper than the bodies are thick there (+658).
pub fn island_push(bodies: &[Body], touches: &mut [Touch], solved: &mut IslandSolved, dt: f32, inv_dt: f32, pushes: &mut [Pushed]) {
    let thick = |s: &Side, n: Vec3| {
        let nl = Vec3::new(dot3(s.rows[0], n), dot3(s.rows[1], n), dot3(s.rows[2], n));
        let q = (unit_or_x(nl) * s.margin + s.core) - s.local;
        let t = fma(q.z, nl.z, fma(nl.x, q.x, q.y * nl.y));
        if t >= 0.0 { t } else { 0.0 }
    };
    for (row, c) in solved.rows.iter_mut().zip(touches.iter()) {
        if c.kind == 1 {
            // (a point constraint: its depth over the step, nothing taken off; no warm start)
            row.row.bias = Vec3::new(c.depth * inv_dt, 0.0, 0.0);
            row.row.neg = Vec3::ZERO;
            continue;
        }
        let d = c.depth - ALLOW;
        let half = d * 0.5;
        let m = if half - ALLOW >= 0.0 { ALLOW } else { half };
        let m = if -m >= 0.0 { 0.0 } else { m };
        let mut ask = d - m;
        let (ta, tb) = (thick(&c.a, c.normal), thick(&c.b, c.back));
        let t = if ta - tb >= 0.0 { tb } else { ta };
        if t != NONE && ask > 0.0 && ask > t {
            for s in [&c.a, &c.b] {
                if let Some(i) = s.body {
                    pushes[i].deep = true;
                }
            }
            let rest = t * 0.5;
            let by = c.normal * (ask - rest);
            let sq = |s: &Side| s.body.map_or(0.0, |i| bodies[i].inv_mass * bodies[i].inv_mass);
            let (a, b) = (sq(&c.a), sq(&c.b));
            let (wa, wb) = if a > b {
                let wa = 1.0 / ((b / a).sqrt() + 1.0);
                (wa, 1.0 - wa)
            } else if b > a {
                let wb = 1.0 / ((a / b).sqrt() + 1.0);
                (1.0 - wb, wb)
            } else {
                (0.5, 0.5)
            };
            let to_b = by * (-wb);
            if let Some(i) = c.a.body {
                pushes[i].push = direct(pushes[i].push, by * wa);
            }
            if let Some(i) = c.b.body {
                pushes[i].push = direct(pushes[i].push, to_b);
            }
            ask = rest;
        }
        row.row.bias = Vec3::new(ask * inv_dt, 0.0, 0.0);
        row.row.neg = Vec3::ZERO - c.pushed;
    }
    // (0x825E7D20: every state from rest; the bodies' matrices as they stand now)
    solved.states = states_of(bodies);
    for s in solved.states.iter_mut() {
        s.vel = Vec3::ZERO;
        s.ang = Vec3::ZERO;
    }
    sweeps2(&mut solved.rows, &mut solved.states, 1.0);
    for (row, c) in solved.rows.iter().zip(touches.iter_mut()) {
        let q = Vec3::ZERO - row.row.neg;
        c.pushed = q;
        let inv = inverse3(&row.row.bcol);
        let v = (inv[0] * q.x + inv[1] * q.y) + inv[2] * q.z;
        for (s, v) in [(&c.a, v), (&c.b, Vec3::ZERO - v)] {
            if let Some(i) = s.body {
                let icol = solved.states[i + 1].icol;
                pushes[i].push = v * (bodies[i].inv_mass * dt) + pushes[i].push;
                let t = cross(s.point - bodies[i].at, v);
                pushes[i].turn = Vec3::new(dot3(icol[0], t), dot3(icol[1], t), dot3(icol[2], t)) * dt + pushes[i].turn;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against the original code run offline: `MCLA_SOLVER_CASES` = the cases file given to
    /// tools/solver_reference.py, `MCLA_SOLVER_REFERENCE` = the CSV it wrote.
    #[test]
    #[ignore = "needs MCLA_SOLVER_CASES and MCLA_SOLVER_REFERENCE (tools/solver_reference.py)"]
    fn the_contact_solver_is_the_originals_bit_for_bit() {
        let cases = std::fs::read_to_string(std::env::var("MCLA_SOLVER_CASES").expect("MCLA_SOLVER_CASES")).unwrap();
        let reference = std::fs::read_to_string(std::env::var("MCLA_SOLVER_REFERENCE").expect("MCLA_SOLVER_REFERENCE")).unwrap();
        let cases: Vec<&str> = cases.lines().filter(|l| !l.starts_with("body") && !l.trim().is_empty()).collect();
        let (mut bad, mut all, mut empty) = (0, 0, 0);
        let mut counts = std::collections::BTreeMap::<&str, (usize, usize)>::new();
        for (case, want) in cases.iter().zip(reference.lines()) {
            let (line, label) = case.split_once('#').unwrap_or((case, ""));
            let w: Vec<u32> = line.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let dt = f(0);
            let body = Body { at: v(1), rows: [v(4), v(7), v(10)], vel: v(13), ang: v(16), inv_mass: f(20), inv_moments: v(21), force: v(24), torque: v(27) };
            let n = w[30] as usize;
            // (an island with no contact row: the original leaves before its bounce factor, E reads 0;
            // the port's 1.0 is never read - nothing to compare)
            if n == 0 {
                empty += 1;
                continue;
            }
            let mut contacts: Vec<Contact> = (0..n)
                .map(|i| {
                    let o = 31 + 24 * i;
                    Contact { depth: f(o), friction: f(o + 1), elasticity: f(o + 2), on_body: v(o + 3), on_world: v(o + 6), normal: v(o + 9), kept: v(o + 12), target: v(o + 15), pushed: v(o + 18), local: v(o + 21), impulse: Vec3::ZERO }
                })
                .collect();
            let inv_dt = 1.0 / dt;
            let mut solved = velocity(&body, &mut contacts, dt, inv_dt);
            let rows = solved.rows.clone();
            let pushed = push(&body, &mut contacts, &mut solved, dt, inv_dt, (0.04, 0.04), false, Vec3::ZERO);
            let want: Vec<u32> = want.trim().split(',').map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let mut got: Vec<(&str, u32)> = vec![("bounce", solved.bounce.to_bits())];
            let mut put = |name: &'static str, x: Vec3, got: &mut Vec<(&str, u32)>| for c in [x.x, x.y, x.z] { got.push((name, c.to_bits())) };
            for (c, row) in contacts.iter().zip(rows.iter()) {
                put("multipliers", c.kept, &mut got);
                put("impulse", c.impulse, &mut got);
                let k = columns(&row.kcol);
                put("inverse", k[0], &mut got);
                put("inverse", k[1], &mut got);
                put("inverse", k[2], &mut got);
                put("c", row.c, &mut got);
                put("bias", row.bias, &mut got);
                put("push multipliers", c.pushed, &mut got);
            }
            put("impulse sum", solved.impulse, &mut got);
            put("turning sum", solved.turning, &mut got);
            put("push", pushed.push, &mut got);
            put("turn", pushed.turn, &mut got);
            got.push(("deep", pushed.deep as u32));
            got.push(("deep", want[got.len()]));
            let rows = rows_pushed(&body.rows, pushed.turn);
            put("rows pushed", rows[0], &mut got);
            put("rows pushed", rows[1], &mut got);
            put("rows pushed", rows[2], &mut got);
            put("place pushed", body.at + pushed.push, &mut got);
            let mut first = true;
            for (i, (name, g)) in got.iter().enumerate() {
                let e = counts.entry(name).or_default();
                e.1 += 1;
                let same = *g == want[i] || (*g | want[i]) == 0x8000_0000;
                if same {
                    e.0 += 1;
                } else if first && bad < 12 {
                    first = false;
                    eprintln!("case {all} ({}) {name} [{i}]: original {:08x} {:e}, here {:08x} {:e}", label.trim(), want[i], f32::from_bits(want[i]), g, f32::from_bits(*g));
                }
            }
            if !first {
                bad += 1;
            }
            all += 1;
        }
        for (name, (ok, n)) in &counts {
            eprintln!("{name:18} {ok} / {n}");
        }
        eprintln!("cases compared {all}, empty islands skipped {empty}, differing {bad}");
        assert_eq!(bad, 0, "{bad} of {all} cases differ");
    }
}
