//! The original's contact solver for one rigid body against the fixed world, operation for
//! operation: the rows of an island's contacts (F-36821), the sweeps over them (F-67704
//! with the kernel F-18210), the impulses they end in (F-24076), and the second solve
//! that only moves the body out of what it stands in (F-49764, F-64140, the body's
//! F-35494 / F-12566 / F-67019). Single precision, the original's order of sums; the
//! vector unit's multiply-add is two roundings, the scalar unit's one.
//!
//! Checked bit for bit against the original code run offline (tools/solver_reference.py; the
//! test `the_contact_solver_is_the_originals_bit_for_bit`). Decode notes: work/veh/solver_rule.md
//! and work/veh/gjk_exact.md part 5.

use super::artic::{
    island::{basis_impulse, contact_bias, self_bias, ContactRow, LimitRow, RigidState, SelfRow},
    Articulated, V4,
};
use super::exact::{cross, dot3, fma, inverse3, recip, world_inverse_inertia};
use glam::Vec3;

#[cfg(test)]
#[path = "solver_leaves_tests.rs"]
mod solver_leaves_tests;

/// How deep a contact may stand (0x828FF840, set by F-36556 from 0x82082810).
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
/// The sweeps of either solve (0x8281358C as the game sets it, F-31205).
pub const SWEEPS: usize = 4;
/// R3: the sweeps of an island that holds a jointed body - a thrown rider - (F-50131 / F-78290: island
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

/// The two directions across a contact's normal (F-95825).
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

/// The inverse of the block in place (F-68939): by its lower half, each pivot kept above a
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

/// F-42768: the inverse mass times (1, 0, 0), (0, 1, 0), (0, 0, 1), each lane a product (vmulfp).
pub fn mass_rows(m: f32) -> [Vec3; 3] {
    [Vec3::X * m, Vec3::Y * m, Vec3::Z * m]
}

impl State {
    fn of(body: &Body) -> State {
        let m = body.inv_mass;
        State {
            vel: body.vel,
            ang: body.ang,
            mcol: columns(&mass_rows(m)),
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
    /// F-13531, F-36219 (F-91803, F-68939).
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

    /// One visit of the kernel (F-18210); returns the step's squares.
    fn visit(&mut self, s: &mut State, omega: f32) -> Vec3 {
        let lin = Vec3::new(dot3(self.basis[0], s.vel), dot3(self.basis[1], s.vel), dot3(self.basis[2], s.vel));
        let ang = Vec3::new(dot3(self.arm[0], s.ang), dot3(self.arm[1], s.ang), dot3(self.arm[2], s.ang));
        let u = (((ang - Vec3::ZERO) + lin) - Vec3::ZERO) - self.bias;
        let ku = Vec3::new(dot3(self.kcol[0], u), dot3(self.kcol[1], u), dot3(self.kcol[2], u));
        let x = ((Vec3::ZERO - ku) * omega) - self.neg;
        let next = cone(x, self.c, self.mu);
        let step = next + self.neg;
        s.give(self, step);
        self.neg = self.neg - step;
        step * step
    }
}

/// F-67704: what the rows start with is given first (F-64443), then the sweeps, until one
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

/// The friction cone a contact row's multipliers are projected on (F-48120; inline in the kernel F-18210):
/// none when the normal part `c . x` is negative, else the tangent part `x * (0, 1, 1)` brought back to `mu` times the
/// normal one along `x - (c . x, 0, 0)` when it lies outside (`|t|` from the estimate and one Newton step).
pub(super) fn cone(x: Vec3, c: Vec3, mu: f32) -> Vec3 {
    let t = Vec3::new(x.x * 0.0, x.y, x.z);
    let nn = dot3(c, x);
    let d = x - Vec3::new(nn, nn * 0.0, nn * 0.0);
    let t2 = dot3(t, t);
    let len = if t2 == 0.0 {
        0.0
    } else {
        let e = 1.0 / t2.sqrt();
        t2 * ((-(t2 * (e * e) - 1.0)) * (0.5 * e) + e)
    };
    let lm = len * mu;
    let g = Vec3::new(-(lm - t.x), -(lm * 0.0 - t.y), -(lm * 0.0 - t.z));
    let (dg, xg) = (dot3(d, g), dot3(x, g));
    let k = if xg > 0.0 { xg * recip(dg + TINY) } else { 0.0 };
    if 0.0 > nn { Vec3::ZERO } else { -(d * k - x) }
}

/// What the tick's torque will add to the body's turning rate, F-14402: the torque +352 x dt in the body's frame
/// (rows +208 ..), times the inverse moments +176, back in the world's; nothing for a torque of 0 in x, y and z.
pub(super) fn turn_change(body: &Body, dt: f32) -> Vec3 {
    if body.torque == Vec3::ZERO {
        return Vec3::ZERO;
    }
    let t = body.torque * dt;
    let local = Vec3::new(dot3(body.rows[0], t), dot3(body.rows[1], t), dot3(body.rows[2], t)) * body.inv_moments;
    let cols = columns(&body.rows);
    Vec3::new(dot3(cols[0], local), dot3(cols[1], local), dot3(cols[2], local))
}

/// F-43928: a side's pending velocity as the rows take it: zero where the side has no collider; the collider's answer
/// (`res`, F-64437) times `less` (one over the island's bounce), or as it is where `unscaled` (a point constraint's row,
/// the flag byte the caller passes). The original's third way, a side with an instance but no collider (its slot 6), is
/// not in the port: such a side gives zero.
fn pend_of(res: Option<Vec3>, unscaled: bool, less: f32) -> Vec3 {
    match res {
        None => Vec3::ZERO,
        Some(r) if unscaled => r,
        Some(r) => r * less,
    }
}

/// What the tick's forces will add to the velocity of the body's point `p` (F-64437).
fn pending(body: &Body, p: Vec3, dt: f32) -> Vec3 {
    let lin = body.force * (body.inv_mass * dt);
    if body.torque == Vec3::ZERO {
        return lin;
    }
    let dw = turn_change(body, dt);
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

/// The velocity solve (F-68693). The contacts' `kept` and `impulse` are written.
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
        // (the body's F-96615)
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

/// The body's direct push (F-12566): `by` added to what is pending, less its share along
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

/// The push solve (F-83361 and F-64140) on the rows of the velocity solve. `margins`:
/// the body's bound's and the world's (bound +112 of the car's composite and of the city's
/// grid). The contacts' `pushed` is written.
pub fn push(body: &Body, contacts: &mut [Contact], solved: &mut Solved, dt: f32, inv_dt: f32, margins: (f32, f32), was_deep: bool, turn_left: Vec3) -> Pushed {
    // (`turn_left`: what body +416 still holds - a turn too small to be applied stays there, F-67019)
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
        // (the body's F-35494)
        out.push = v * (body.inv_mass * dt) + out.push;
        let t = cross(c.on_body - body.at, v);
        out.turn = Vec3::new(dot3(icol[0], t), dot3(icol[1], t), dot3(icol[2], t)) * dt + out.turn;
    }
    out
}

/// Sine and cosine as the vector unit's own routine has them (inside F-58201): the angle
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

/// The rotation by `angle` about the unit vector `axis` as the push's turn builds it (F-58201), by rows.
fn rotation(axis: Vec3, angle: f32) -> [Vec3; 3] {
    let (s, c) = sin_cos(angle);
    let a = axis * s;
    let n = Vec3::ZERO - a;
    let t = 1.0 - c;
    [(axis * axis.x) * t + Vec3::new(c, a.z, n.y), (axis * axis.y) * t + Vec3::new(n.z, c, a.x), (axis * axis.z) * t + Vec3::new(a.y, n.x, c)]
}

/// F-67019's test: is the turn large enough to be applied (and cleared)? One that is not stays in
/// body +416 and the next tick's is added to it.
pub fn turn_applies(turn: Vec3) -> bool {
    dot3(turn, turn) > f32::from_bits(0x2b8c_bccc)
}

/// The body's rows with the push solve's turn applied (F-67019): nothing below a millionth
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
    /// +164: 0 a contact; a constraint's (F-83853): 2 a rope (a length of 0.01 or more), 1 a
    /// point (a 256-byte row of its own, A-81774)
    pub kind: u8,
    /// a side on one of the island's jointed bodies (a thrown rider's part): the row is then of
    /// type 7 (that Side has no body)
    pub jointed: Option<JointedSide>,
}

/// A contact's side on a jointed body (row type 7 of F-36821).
#[derive(Clone, Copy, Debug, Default)]
pub struct JointedSide {
    /// which of the island's jointed bodies
    pub body: usize,
    /// the link of the side's part (F-19886, artic::island::part_link)
    pub link: usize,
    /// the jointed side is the contact's B (the row is then the B side's, its sign -1)
    pub on_b: bool,
    /// lane w of the side's point (contact +60 / +76), which the point's arm keeps
    pub point_w: f32,
    /// both sides on this body (the rider against himself, row type 13): B's link and lane w
    /// of B's point (+76); the side above is then A's
    pub self_b: Option<(usize, f32)>,
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

/// A row of an island's contact: a rigid one (type 1 and the constraints'), or one with a
/// jointed side (type 7) with its jointed body, B's state, and what the apply passes read: the
/// side, whether it is a contact (kind 0: its impulse is times the island's elasticity factor)
/// and the jointed side's point.
#[derive(Clone, Copy, Debug)]
enum IRow {
    Rigid(Row2),
    Jointed { row: ContactRow, body: usize, sb: usize, on_b: bool, contact: bool, point: V4 },
    /// type 13: both sides on the jointed body `body`, at the points `pa` (+48) and `pb` (+64)
    Own { row: SelfRow, body: usize, contact: bool, pa: V4, pb: V4 },
}

fn v4(v: Vec3) -> V4 {
    V4::new(v.x, v.y, v.z, 0.0)
}
fn v3(v: V4) -> Vec3 {
    Vec3::new(v.0[0], v.0[1], v.0[2])
}
/// B's state as the type-7 code reads it: the state's +32 / +80 blocks as they lie in the
/// island's work space (F-71455), row by row - the transposes of `mcol` / `icol`. F-11306 (the
/// row's K) takes them as rows and F-67470 (the give) through their transposes; the world inverse
/// inertia is not exactly symmetric in floats, so the side matters (rc.0 1336 / 1337: the
/// bike's and the car's +80 in the snapshot's work space are `icol` transposed, word for word).
fn rigid_state(s: &State) -> RigidState {
    RigidState { vel: v4(s.vel), ang: v4(s.ang), m: columns(&s.mcol).map(v4), i: columns(&s.icol).map(v4) }
}
/// A type-7 visit or give with B's state taken out of the island's states and put back.
fn with_b(states: &mut [State], sb: usize, f: impl FnOnce(&mut RigidState)) {
    let mut b = rigid_state(&states[sb]);
    f(&mut b);
    states[sb].vel = v3(b.vel);
    states[sb].ang = v3(b.ang);
}

/// The K block of a row between two sides (F-72162 for a point constraint, `point`; the contact rows' F-11306 / F-36219
/// order otherwise): what an impulse along each direction does to the point's velocity along each - A's angular term,
/// A's linear, B's, with `point` B's linear term before its angular one. `angular_only` (the original's byte +240) leaves
/// the linear terms out: zero vectors in their places.
fn k_block(basis: &[Vec3; 3], bcol: &[Vec3; 3], arm: &[Vec3; 3], jcol: &[Vec3; 3], arm_b: &[Vec3; 3], jcol_b: &[Vec3; 3], sa: &State, sb: &State, point: bool, angular_only: bool) -> [Vec3; 3] {
    let through = |s: &State, j: &[Vec3; 3]| {
        let (mrow, irow) = (columns(&s.mcol), columns(&s.icol));
        let t: [Vec3; 3] = std::array::from_fn(|k| (bcol[0] * mrow[k].x + bcol[1] * mrow[k].y) + bcol[2] * mrow[k].z);
        let w: [Vec3; 3] = std::array::from_fn(|k| (j[0] * irow[k].x + j[1] * irow[k].y) + j[2] * irow[k].z);
        (t, w)
    };
    let (ta, wa) = through(sa, jcol);
    let (tb, wb) = through(sb, jcol_b);
    std::array::from_fn(|i| {
        let la = (ta[0] * basis[i].x + ta[1] * basis[i].y) + ta[2] * basis[i].z;
        let aa = (wa[0] * arm[i].x + wa[1] * arm[i].y) + wa[2] * arm[i].z;
        let lb = (tb[0] * basis[i].x + tb[1] * basis[i].y) + tb[2] * basis[i].z;
        let ab = (wb[0] * arm_b[i].x + wb[1] * arm_b[i].y) + wb[2] * arm_b[i].z;
        let (la, lb) = if angular_only { (Vec3::ZERO, Vec3::ZERO) } else { (la, lb) };
        // (a point constraint's block, F-72162: B's linear term before its angular one)
        if point { ((aa + la) + lb) + ab } else { ((aa + la) + ab) + lb }
    })
}

impl Row2 {
    fn new(bodies: &[Body], states: &[State], c: &Touch) -> Row2 {
        let basis = basis(c.normal);
        let place = |s: &Side| s.body.map_or(Vec3::ZERO, |i| bodies[i].at);
        let (ra, rb) = (c.a.point - place(&c.a), c.b.point - place(&c.b));
        let arm = [cross(ra, basis[0]), cross(ra, basis[1]), cross(ra, basis[2])];
        let arm_b = [cross(rb, basis[0]), cross(rb, basis[1]), cross(rb, basis[2])];
        let (bcol, jcol, jcol_b) = (columns(&basis), columns(&arm), columns(&arm_b));
        let (sa, sb) = (c.a.body.map_or(0, |i| i + 1), c.b.body.map_or(0, |i| i + 1));
        let k = k_block(&basis, &bcol, &arm, &jcol, &arm_b, &jcol_b, &states[sa], &states[sb], c.kind == 1, false);
        if c.kind == 1 {
            // (no c row, no block of ten thousands: F-68939 on the block as it is)
            return Row2 { row: Row { neg: Vec3::ZERO, bias: Vec3::ZERO, basis, bcol, arm, jcol, kcol: columns(&block_inverse(&k)), c: Vec3::X, mu: 0.0 }, arm_b, jcol_b, sa, sb, kind: 1 };
        }
        let cc = if k[0].x == 0.0 { Vec3::X } else { k[0] * recip(k[0].x) };
        // (F-36219: a row between two sides without bodies is given a block of ten thousands)
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
            // (a point constraint, F-52995 / F-70556: both ways, no relaxation, no cone)
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
        let next = cone(x, r.c, r.mu);
        let step = next + r.neg;
        self.give(states, step);
        self.row.neg = self.row.neg - step;
        step * step
    }
}

/// F-42898 (inline in F-50131 as well): the sweeps of an island. Of the three counts `plain` (the first global, the game sets 4),
/// `joint` (the second, the image's 8) and `typed_n` (the third, the game sets 32): without the island's byte +80
/// (`typed`: it holds contacts of type 3 or 4, which the port has none of) an island with a jointed body (+20) takes `joint`,
/// the others `plain`; with it `typed_n`, or the larger (signed) of `joint` and `typed_n` where it has a jointed body.
pub fn sweeps_count(typed: bool, jointed: bool, plain: i32, joint: i32, typed_n: i32) -> i32 {
    match (typed, jointed) {
        (false, false) => plain,
        (false, true) => joint,
        (true, false) => typed_n,
        (true, true) => joint.max(typed_n),
    }
}

/// F-50131: whether the island's velocity solve runs the sweeps at all - its rows (+52) or its other count (+16) above
/// zero. (The port runs them always; with no rows they change nothing.)
#[allow(dead_code)]
pub fn runs_sweeps(rows: i32, others: i32) -> bool {
    rows > 0 || others > 0
}

/// The sweeps over an island's rows (F-67704): the contacts' in the island's order, then the
/// joint-limit rows of each jointed body (the tail of the row list). First pass (F-90205): a
/// jointed body with rows (types 7 and 11) has its solve-time scratch cleared (F-80860) and the
/// joint rows' multipliers are zeroed (no warm start for type 11); the contacts' kept
/// multipliers given (F-64443: types 1 and 7); then the sweeps, 8 of them with a jointed body
/// (SWEEPS_JOINTED), until one changes nothing.
fn sweeps2(rows: &mut [IRow], states: &mut [State], omega: f32, jointed: &mut [Articulated], limits: &mut [Vec<LimitRow>]) {
    for row in rows.iter() {
        if let IRow::Jointed { body, .. } | IRow::Own { body, .. } = row {
            jointed[*body].clear_scratch();
        }
    }
    for (a, l) in jointed.iter_mut().zip(limits.iter_mut()) {
        if !l.is_empty() {
            a.clear_scratch();
        }
        for r in l.iter_mut() {
            r.mult = V4::ZERO;
        }
    }
    for row in rows.iter() {
        match row {
            IRow::Rigid(r) => r.give(states, Vec3::ZERO - r.row.neg),
            IRow::Jointed { row, body, sb, .. } => {
                // (the sum this adds to is cleared by every sweep)
                let mut sum = V4::ZERO;
                with_b(states, *sb, |b| row.give(&mut jointed[*body], b, V4::ZERO.sub(row.neg), &mut sum));
            }
            IRow::Own { row, body, .. } => {
                // (F-64443's type 13: F-85931)
                let mut sum = V4::ZERO;
                row.give(&mut jointed[*body], V4::ZERO.sub(row.neg), &mut sum);
            }
        }
    }
    let count = sweeps_count(false, !jointed.is_empty(), sweeps_now() as i32, SWEEPS_JOINTED as i32, 32).max(0) as usize;
    for _ in 0..count {
        let mut left = Vec3::ZERO;
        for row in rows.iter_mut() {
            match row {
                IRow::Rigid(r) => left = r.visit(states, omega) + left,
                IRow::Jointed { row, body, sb, .. } => {
                    let mut l4 = v4(left);
                    with_b(states, *sb, |b| row.visit(&mut jointed[*body], b, &mut l4));
                    left = v3(l4);
                }
                IRow::Own { row, body, .. } => {
                    let mut l4 = v4(left);
                    row.visit(&mut jointed[*body], &mut l4);
                    left = v3(l4);
                }
            }
        }
        if limits.iter().any(|l| !l.is_empty()) {
            // (the same sum of squares, which the joint-limit kernel F-31194 adds to in vector form)
            let mut l4 = V4::new(left.x, left.y, left.z, 0.0);
            for (a, l) in jointed.iter_mut().zip(limits.iter_mut()) {
                for r in l.iter_mut() {
                    a.visit_limit_row(r, &mut l4);
                }
            }
            left = Vec3::new(l4.0[0], l4.0[1], l4.0[2]);
        }
        if 0.0 >= dot3(left, left) {
            break;
        }
    }
}

/// What an island's velocity solve leaves.
pub struct IslandSolved {
    rows: Vec<IRow>,
    states: Vec<State>,
    pub bounce: f32,
    /// the joint-limit rows of each jointed body of the island, in its order
    pub limits: Vec<Vec<LimitRow>>,
}

impl IslandSolved {
    /// An island's solve left by another step (the game's level step: a car's view of an island it did not solve).
    pub fn empty() -> IslandSolved {
        IslandSolved { rows: Vec::new(), states: Vec::new(), bounce: 0.0, limits: Vec::new() }
    }
}

fn states_of(bodies: &[Body]) -> Vec<State> {
    std::iter::once(NO_STATE).chain(bodies.iter().map(State::of)).collect()
}

/// An island's rows and their sweeps (F-36821, F-50131), from the bodies' velocities as
/// they stand and the contacts' kept multipliers. Nothing is written back.
pub fn island_solve(bodies: &[Body], touches: &[Touch], dt: f32, inv_dt: f32) -> IslandSolved {
    island_solve_jointed(bodies, touches, &mut [], (0, 0.0), dt, inv_dt)
}

/// The same with the island's jointed bodies (a thrown rider): their active joint limits found
/// when the work space is made (F-73275, the hinges' contact byte +532 set before by the
/// caller, F-18964), the contacts with a jointed side as rows of type 7 (F-36821's branch with
/// one jointed side, F-38941, F-11306), the joint rows after the contacts' (the tail of
/// F-36821), and 8 sweeps. `others`: the island's enabled contacts not in `touches` (their
/// count and the sum of their elasticities: those of a body that shares no row with these, as
/// the thrown rider's bike lying on the street), which count in the island's elasticity factor
/// (isl +40). The jointed bodies' solve-time scratch is written; their impulses are not
/// (island_apply_jointed). A contact with both sides on one jointed body (`self_b`) is a row of
/// type 13 (self_row).
pub fn island_solve_jointed(bodies: &[Body], touches: &[Touch], jointed: &mut [Articulated], others: (usize, f32), dt: f32, inv_dt: f32) -> IslandSolved {
    let mut sum = others.1 + 0.0f32;
    for c in touches.iter() {
        sum = c.elasticity + sum;
    }
    let n = touches.len() + others.0;
    let share = if n == 0 { f32::MAX } else { 1.0 / n as f32 };
    let bounce = fma(share, sum, 1.0);
    let less = 1.0 / bounce;
    let mut states = states_of(bodies);
    let mut rows: Vec<IRow> = Vec::with_capacity(touches.len());
    for c in touches.iter() {
        if let Some(j) = c.jointed {
            rows.push(match j.self_b {
                Some((lb, wb)) => self_row(jointed, c, &j, lb, wb, less, inv_dt),
                None => jointed_row(bodies, &states, jointed, c, &j, less, dt, inv_dt),
            });
            continue;
        }
        let mut row = Row2::new(bodies, &states, c);
        if c.kind == 1 {
            // (A-48276: the target minus the two points' pending velocities - not over the island's
            // elasticity - through the basis as its transpose; no warm start, F-90205)
            let pend = |s: &Side| pend_of(s.body.map(|i| pending(&bodies[i], s.point, dt)), true, less);
            let v = c.target - (pend(&c.a) - pend(&c.b));
            let b = &row.row.basis;
            row.row.bias = Vec3::new(dot3(v, Vec3::new(b[0].x, b[1].x, b[2].x)), dot3(v, Vec3::new(b[0].y, b[1].y, b[2].y)), dot3(v, Vec3::new(b[0].z, b[1].z, b[2].z)));
            rows.push(IRow::Rigid(row));
            continue;
        }
        let pend = |s: &Side| pend_of(s.body.map(|i| pending(&bodies[i], s.point, dt)), false, less);
        let p = pend(&c.a) - pend(&c.b);
        let g = (ALLOW - SKIN) - c.depth;
        let gap = if -g >= 0.0 { 0.0 } else { g };
        let b = &row.row.basis;
        let x0 = fma(gap, inv_dt, fma(p.z, b[0].z, fma(p.x, b[0].x, p.y * b[0].y)));
        let x1 = fma(b[1].z, p.z, fma(b[1].x, p.x, b[1].y * p.y));
        let x2 = fma(b[2].z, p.z, fma(b[2].x, p.x, b[2].y * p.y));
        row.row.bias = (Vec3::ZERO - Vec3::new(x0, x1, x2)) + c.target;
        row.row.neg = Vec3::ZERO - c.kept;
        rows.push(IRow::Rigid(row));
    }
    for a in jointed.iter_mut() {
        a.limit_dofs();
    }
    let mut limits: Vec<Vec<LimitRow>> = jointed.iter().map(|a| a.limit_rows(inv_dt)).collect();
    sweeps2(&mut rows, &mut states, OMEGA, jointed, &mut limits);
    IslandSolved { rows, states, bounce, limits }
}

/// F-36821's branch for a contact with one jointed side (row type 7). The row is the jointed
/// side's: its normal (+80, or +96 when it is B), its point, its sign (+1, or -1 when it is
/// B); the other side is the row's B (its state, F-71455; the fixed world's is the zero state).
/// Pending velocities (F-43928, times 1 / the island's elasticity factor for a contact): the
/// contact's A collider's at the row's point less the contact's B collider's at the row's B
/// point - so when the jointed side is B each collider is asked at the other side's point, as
/// the code does. Bias by contact_bias (the gap without the skin); multipliers warm-started
/// from +128 (the warm-start switch, set).
#[allow(clippy::too_many_arguments)]
fn jointed_row(bodies: &[Body], states: &[State], jointed: &[Articulated], c: &Touch, j: &JointedSide, less: f32, dt: f32, inv_dt: f32) -> IRow {
    let a = &jointed[j.body];
    let (mine, other) = if j.on_b { (&c.b, &c.a) } else { (&c.a, &c.b) };
    let one = V4::new(1.0, 1.0, 1.0, 0.0);
    let sign = if j.on_b { V4::ZERO.sub(one) } else { one };
    let normal = if j.on_b { c.back } else { c.normal };
    let point = V4::new(mine.point.x, mine.point.y, mine.point.z, j.point_w);
    let sb = other.body.map_or(0, |i| i + 1);
    let place_b = other.body.map_or(V4::ZERO, |i| v4(bodies[i].at));
    let rows = basis(normal).map(v4);
    let mut row = ContactRow::new(a, j.link, point, v4(other.point), place_b, &rows, &rigid_state(&states[sb]), c.friction);
    let scale = |v: V4| if c.kind == 0 { v.mul(V4::splat(less)) } else { v };
    let rigid = |s: &Side, at: Vec3| v4(s.body.map_or(Vec3::ZERO, |i| pending(&bodies[i], at, dt)));
    let p = if j.on_b {
        scale(rigid(&c.a, c.b.point)).sub(scale(a.point_pending(j.link, v4(c.a.point))))
    } else {
        scale(a.point_pending(j.link, point)).sub(scale(rigid(&c.b, c.b.point)))
    };
    row.bias = contact_bias(&rows, p, if c.kind == 0 { Some(c.depth) } else { None }, sign, inv_dt, v4(c.target));
    row.neg = V4::ZERO.sub(v4(c.kept));
    IRow::Jointed { row, body: j.body, sb, on_b: j.on_b, contact: c.kind == 0, point }
}

/// F-36821's branch for a contact with both sides on one jointed body (row type 13): the
/// basis of the normal (+80, F-95825), the points +48 (A) and +64 (B) on their parts' links;
/// pending velocities (F-43928: slot 22 of each side, times 1 / the island's elasticity
/// factor for a contact) A's less B's; bias by self_bias; multipliers warm-started from +128.
#[allow(clippy::too_many_arguments)]
fn self_row(jointed: &[Articulated], c: &Touch, j: &JointedSide, lb: usize, wb: f32, less: f32, inv_dt: f32) -> IRow {
    let a = &jointed[j.body];
    let pa = V4::new(c.a.point.x, c.a.point.y, c.a.point.z, j.point_w);
    let pb = V4::new(c.b.point.x, c.b.point.y, c.b.point.z, wb);
    let rows = basis(c.normal).map(v4);
    let mut row = SelfRow::new(a, j.link, pa, lb, pb, &rows, c.friction);
    let scale = |v: V4| if c.kind == 0 { v.mul(V4::splat(less)) } else { v };
    let p = scale(a.point_pending(j.link, pa)).sub(scale(a.point_pending(lb, pb)));
    row.bias = self_bias(&rows, p, if c.kind == 0 { Some(c.depth) } else { None }, inv_dt, v4(c.target));
    row.neg = V4::ZERO.sub(v4(c.kept));
    IRow::Own { row, body: j.body, contact: c.kind == 0, pa, pb }
}

/// The impulse a type-13 row hands out (F-24076 / F-64140): its multipliers through the
/// inverse basis, times the island's elasticity factor for a contact of the velocity solve.
fn self_impulse(row: &SelfRow, scale: Option<f32>) -> V4 {
    let j = basis_impulse(&row.bcol, V4::ZERO.sub(row.neg));
    match scale {
        Some(e) => j.mul(V4::splat(e)),
        None => j,
    }
}

/// The impulse a type-7 row hands out (F-24076 / F-64140): its multipliers through the inverse
/// basis, turned over when the jointed side is B, times the island's elasticity factor for a
/// contact of the velocity solve. The contact's A gets it, B its opposite.
fn jointed_impulse(row: &ContactRow, on_b: bool, scale: Option<f32>) -> V4 {
    let p = V4::ZERO.sub(row.neg);
    let p = if on_b { V4::ZERO.sub(p) } else { p };
    let j = basis_impulse(&row.bcol, p);
    match scale {
        Some(e) => j.mul(V4::splat(e)),
        None => j,
    }
}

/// The joint part of the velocity solve's apply pass (F-24076's tail): each jointed body's
/// rows' multipliers into its angular impulse accumulators (after the contacts').
pub fn island_apply_limits(solved: &IslandSolved, jointed: &mut [Articulated]) {
    for (a, l) in jointed.iter_mut().zip(solved.limits.iter()) {
        for r in l {
            a.apply_limit_row(r);
        }
    }
}

/// The jointed bodies' part of the velocity solve's apply pass (F-24076): each type-7 row's
/// impulse into its link through the collider's slot 34 (F-78198 -> F-19400) at the jointed
/// side's point - the contact's A gets the impulse, B its opposite - in the island's order,
/// then the joint rows (island_apply_limits).
pub fn island_apply_jointed(solved: &IslandSolved, jointed: &mut [Articulated]) {
    for row in &solved.rows {
        match row {
            IRow::Jointed { row, body, on_b, contact, point, .. } => {
                let j = jointed_impulse(row, *on_b, if *contact { Some(solved.bounce) } else { None });
                let j = if *on_b { V4::ZERO.sub(j) } else { j };
                jointed[*body].apply_at_point(row.link, j, *point);
            }
            IRow::Own { row, body, contact, pa, pb } => {
                // (A's slot 34 with the impulse at +48, then B's with its opposite at +64)
                let j = self_impulse(row, if *contact { Some(solved.bounce) } else { None });
                jointed[*body].apply_at_point(row.la, j, *pa);
                jointed[*body].apply_at_point(row.lb, V4::ZERO.sub(j), *pb);
            }
            IRow::Rigid(_) => {}
        }
    }
    island_apply_limits(solved, jointed);
}

/// The contacts' multipliers and impulses as the solve left them (F-66979; the same numbers
/// F-24076 hands out): `kept` (+128) and `impulse` (+112, the vector that pushes A).
pub fn island_impulses(solved: &IslandSolved, touches: &mut [Touch]) {
    for (row, c) in solved.rows.iter().zip(touches.iter_mut()) {
        let row = match row {
            IRow::Rigid(r) => r,
            IRow::Jointed { row, on_b, contact, .. } => {
                // (+128 keeps its lane w; the port's Vec3 has none)
                c.kept = v3(V4::ZERO.sub(row.neg));
                c.impulse = v3(jointed_impulse(row, *on_b, if *contact { Some(solved.bounce) } else { None }));
                continue;
            }
            IRow::Own { row, contact, .. } => {
                c.kept = v3(V4::ZERO.sub(row.neg));
                c.impulse = v3(self_impulse(row, if *contact { Some(solved.bounce) } else { None }));
                continue;
            }
        };
        let p = Vec3::ZERO - row.row.neg;
        c.kept = p;
        let j = rigid_impulse(&row.row.bcol, V4::ZERO.sub(v4(row.row.neg)));
        // (F-66979: a constraint's not times the island's elasticity)
        c.impulse = v3(if c.kind == 0 { j.mul(V4::splat(solved.bounce)) } else { j });
    }
}

/// A rigid row's impulse as F-66979 makes it: the multipliers times the inverse of the row's 3 x 3, in the vector
/// unit's own operations (its cofactors by vnmsubfp, the determinant by vmsum3fp, the reciprocal's Newton step, the
/// transpose by merges, the sum by vmaddfp), with the unit's denormal flush - which decides the sign of a zero where a
/// cofactor is a denormal (ce.0 86).
fn rigid_impulse(m: &[Vec3; 3], p: V4) -> V4 {
    let (r0, r1, r2) = (v4(m[0]), v4(m[1]), v4(m[2]));
    let ab = r0.zxy().nmsub(r1.yzx(), r0.yzx().mul(r1.zxy()));
    let bc = r1.zxy().nmsub(r2.yzx(), r1.yzx().mul(r2.zxy()));
    let ca = r2.zxy().nmsub(r0.yzx(), r2.yzx().mul(r0.zxy()));
    let det = ab.dot3(r2);
    let e = det.recip();
    let r = e.madd(e.nmsub(det, V4::splat(1.0)), e);
    let (h, l) = (bc.mergeh(ab), ca.mergeh(V4::ZERO));
    let rows = [h.mergeh(l).mul(r), h.mergel(l).mul(r), bc.mergel(ab).mergeh(ca.mergel(V4::ZERO)).mul(r)];
    let j = p.lane(0).mul(rows[0]);
    let j = p.lane(1).madd(rows[1], j);
    p.lane(2).madd(rows[2], j)
}

/// The impulses given to the bodies' accumulators (F-71376 -> F-96615): per body what
/// +368 and +384 gain, contact by contact, A then B.
pub fn island_apply(bodies: &[Body], touches: &[Touch]) -> Vec<(Vec3, Vec3)> {
    let mut out = vec![(Vec3::ZERO, Vec3::ZERO); bodies.len()];
    for c in touches {
        for (side, call) in [&c.a, &c.b].into_iter().zip(applications(c)) {
            if let (Some(i), Some((j, point))) = (side.body, call) {
                out[i].0 = out[i].0 + j;
                out[i].1 = out[i].1 + cross(point - bodies[i].at, j);
            }
        }
    }
    out
}

/// F-71376: the two applications a contact's update hands the collider (F-96615 -> F-78198: impulse and point): A's the impulse at
/// A's point (+48), B's its opposite - taken from zero - at B's point (+64); a side with no collider is not applied.
pub fn applications(c: &Touch) -> [Option<(Vec3, Vec3)>; 2] {
    [c.a.body.map(|_| (c.impulse, c.a.point)), c.b.body.map(|_| (Vec3::ZERO - c.impulse, c.b.point))]
}

/// F-54406: every state's velocity (+0) and angular velocity (+16) from rest; the rest of the 128 bytes stays.
fn rest_states(states: &mut [State]) {
    for s in states.iter_mut() {
        s.vel = Vec3::ZERO;
        s.ang = Vec3::ZERO;
    }
}

/// The push solve of an island (F-49764, F-78290, F-64140) on the rows of its last
/// velocity solve. `pushes`: per body what +400 / +416 hold as it begins and end with, and
/// whether a contact of it stood deeper than the bodies are thick there (+658).
pub fn island_push(bodies: &[Body], touches: &mut [Touch], solved: &mut IslandSolved, dt: f32, inv_dt: f32, pushes: &mut [Pushed]) {
    island_push_jointed(bodies, touches, solved, &mut [], dt, inv_dt, pushes)
}

/// The same with the island's jointed bodies (as in island_solve_jointed): their rows' push
/// bias, their velocities committed, saved and cleared (F-49764's tail), the sweeps, and the
/// rows' push multipliers into their angular impulse accumulators (F-64140's tail). Their
/// post-push step (F-48210) is the caller's.
pub fn island_push_jointed(bodies: &[Body], touches: &mut [Touch], solved: &mut IslandSolved, jointed: &mut [Articulated], dt: f32, inv_dt: f32, pushes: &mut [Pushed]) {
    let thick = |s: &Side, n: Vec3| {
        let nl = Vec3::new(dot3(s.rows[0], n), dot3(s.rows[1], n), dot3(s.rows[2], n));
        let q = (unit_or_x(nl) * s.margin + s.core) - s.local;
        let t = fma(q.z, nl.z, fma(nl.x, q.x, q.y * nl.y));
        if t >= 0.0 { t } else { 0.0 }
    };
    for (row, c) in solved.rows.iter_mut().zip(touches.iter()) {
        let row = match row {
            IRow::Rigid(r) => r,
            IRow::Jointed { row, body, .. } => {
                // (F-49764: a row with a side on a jointed collider of type 1 - the rider's - skips the
                // thickness test and the direct push: the bias is what the depth asks over the step)
                assert!(jointed[*body].collider_type() != 3, "the push solve of a type-3 jointed collider is not ported");
                let d = c.depth - ALLOW;
                let half = d * 0.5;
                let m = if half - ALLOW >= 0.0 { ALLOW } else { half };
                let m = if -m >= 0.0 { 0.0 } else { m };
                row.bias = V4::new((d - m) * inv_dt, 0.0, 0.0, 0.0);
                // (+0's lane w is the contact's depth, +12)
                row.neg = V4::ZERO.sub(V4::new(c.pushed.x, c.pushed.y, c.pushed.z, c.depth));
                continue;
            }
            IRow::Own { row, .. } => {
                // (both sides on the rider's collider, type 1: as type 7)
                let d = c.depth - ALLOW;
                let half = d * 0.5;
                let m = if half - ALLOW >= 0.0 { ALLOW } else { half };
                let m = if -m >= 0.0 { 0.0 } else { m };
                row.bias = V4::new((d - m) * inv_dt, 0.0, 0.0, 0.0);
                row.neg = V4::ZERO.sub(V4::new(c.pushed.x, c.pushed.y, c.pushed.z, c.depth));
                continue;
            }
        };
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
    // (F-54406: every state from rest; the bodies' matrices as they stand now)
    solved.states = states_of(bodies);
    rest_states(&mut solved.states);
    for (a, l) in jointed.iter_mut().zip(solved.limits.iter_mut()) {
        a.push_start(l, inv_dt);
    }
    sweeps2(&mut solved.rows, &mut solved.states, 1.0, jointed, &mut solved.limits);
    for (row, c) in solved.rows.iter().zip(touches.iter_mut()) {
        let v = match row {
            IRow::Rigid(row) => {
                let q = Vec3::ZERO - row.row.neg;
                c.pushed = q;
                let inv = inverse3(&row.row.bcol);
                (inv[0] * q.x + inv[1] * q.y) + inv[2] * q.z
            }
            IRow::Jointed { row, body, on_b, point, .. } => {
                c.pushed = v3(V4::ZERO.sub(row.neg));
                // (F-64140 -> F-93628: A's slot 36 with the impulse, B's with its opposite; a jointed
                // collider's slot 36, F-41014, is its slot 34)
                let j = jointed_impulse(row, *on_b, None);
                jointed[*body].apply_at_point(row.link, if *on_b { V4::ZERO.sub(j) } else { j }, *point);
                v3(j)
            }
            IRow::Own { row, body, pa, pb, .. } => {
                c.pushed = v3(V4::ZERO.sub(row.neg));
                let j = self_impulse(row, None);
                jointed[*body].apply_at_point(row.la, j, *pa);
                jointed[*body].apply_at_point(row.lb, V4::ZERO.sub(j), *pb);
                v3(j)
            }
        };
        for (s, v) in [(&c.a, v), (&c.b, Vec3::ZERO - v)] {
            if let Some(i) = s.body {
                let icol = solved.states[i + 1].icol;
                pushes[i].push = v * (bodies[i].inv_mass * dt) + pushes[i].push;
                let t = cross(s.point - bodies[i].at, v);
                pushes[i].turn = Vec3::new(dot3(icol[0], t), dot3(icol[1], t), dot3(icol[2], t)) * dt + pushes[i].turn;
            }
        }
    }
    island_apply_limits(solved, jointed);
}

/// The collider's end of tick (F-32811), its flags: `deep` +658 (a deep push this tick), `count` +659 (deep-push ticks in a
/// row), `set` +660 (the "last" matrix is valid; the set-down F-68574 clears it). Gives whether the instance's matrix
/// becomes the "last" one (+96) the next tick's collision takes, and the three bytes after it: with no deep push it is
/// taken and +660 = 1; with one only while +660 is clear or after more than 8 such ticks. Then +658 is cleared if it was
/// set, else +659. Checked over every state against the original run offline (`end_matrix_matches_the_original`).
pub fn end_matrix(deep: u8, count: u8, set: u8) -> (bool, u8, u8, u8) {
    if deep == 0 {
        (true, 0, 0, 1)
    } else {
        (set == 0 || count > 8, 0, count, set)
    }
}

#[cfg(test)]
mod end_matrix_tests {
    /// tools/end_matrix_reference.py: F-32811 run offline over +658 / +660 in {0, 1, 2, 0x80, 0xff} and +659 0 .. 255.
    #[test]
    fn end_matrix_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/end_matrix_ref.txt").lines() {
            let w: Vec<u32> = line.split(' ').filter(|x| *x != "->").map(|x| x.parse().unwrap()).collect();
            let (took, d, c, s) = super::end_matrix(w[0] as u8, w[1] as u8, w[2] as u8);
            assert_eq!([took as u32, d as u32, c as u32, s as u32], [w[3], w[4], w[5], w[6]], "{line}");
            n += 1;
        }
        assert_eq!(n, 6400);
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

#[cfg(test)]
mod turn_change_tests {
    use super::*;

    /// F-14402 as the original runs it (tools/turn_change_reference.py, 5000 cases, testdata/turn_change_ref.txt):
    /// x, y and z equal bit for bit, NaN as NaN. (The routine's w lane is its y - nothing reads it.)
    #[test]
    fn turn_change_matches_the_original() {
        let mut bad = Vec::new();
        let mut n = 0;
        for line in include_str!("testdata/turn_change_ref.txt").lines() {
            let (lhs, rhs) = line.split_once(" -> ").unwrap();
            let w: Vec<f32> = lhs.split(' ').map(|x| f32::from_bits(u32::from_str_radix(x, 16).unwrap())).collect();
            let o: Vec<u32> = rhs.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let v = |k: usize| Vec3::new(w[k], w[k + 1], w[k + 2]);
            let body = Body { at: Vec3::ZERO, rows: [v(0), v(4), v(8)], vel: Vec3::ZERO, ang: Vec3::ZERO, inv_mass: 0.0,
                inv_moments: v(12), force: Vec3::ZERO, torque: v(16) };
            let got = turn_change(&body, w[20]);
            let same = |g: f32, e: u32| g.to_bits() == e || (g.is_nan() && f32::from_bits(e).is_nan());
            if !(same(got.x, o[0]) && same(got.y, o[1]) && same(got.z, o[2])) { bad.push((line, got)); }
            n += 1;
        }
        assert_eq!(n, 5000);
        assert!(bad.is_empty(), "{} mismatches, first {:?}", bad.len(), &bad[..bad.len().min(3)]);
    }
}

#[cfg(test)]
mod cone_tests {
    use super::*;

    /// The friction cone F-48120 run offline (tools/cone_reference.py): 6,000 multipliers (no tangent part, no normal
    /// part, the normal row along x and not, mu 0 .. 2) - every word equal.
    #[test]
    fn cone_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut n = 0;
        for line in include_str!("../testdata/cone_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let c: Vec<f32> = case.split(' ').map(|x| f32::from_bits(h(x))).collect();
            let got = cone(Vec3::new(c[0], c[1], c[2]), Vec3::new(c[3], c[4], c[5]), c[6]);
            let got: Vec<u32> = got.to_array().iter().map(|x| x.to_bits()).collect();
            let want: Vec<u32> = want.split(' ').map(h).collect();
            assert_eq!(got, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 6000);
    }
}

#[cfg(test)]
mod rest_tests {
    use super::*;

    /// F-54406 run offline (tools/hull_small_reference.py): every state's velocity and angular velocity zeroed, the other six
    /// vectors of its 128 bytes and the states past the count untouched (counts -1 .. 8). Compared on x, y, z; a zeroed
    /// vector's w lane is 0 in the original too.
    #[test]
    fn rest_states_matches_the_original() {
        let mut n = 0;
        for l in include_str!("../testdata/hull_small_ref.txt").lines() {
            let Some(rest) = l.strip_prefix("zero ") else { continue };
            let (i, o) = rest.split_once(" -> ").unwrap();
            let w = |s: &str| -> Vec<u32> { s.split_whitespace().map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap()).collect() };
            let (i, o) = (w(i), w(o));
            let count = (i[0] as i32).max(0) as usize;
            let v3 = |k: usize| Vec3::new(f32::from_bits(i[1 + k]), f32::from_bits(i[2 + k]), f32::from_bits(i[3 + k]));
            let mut states: Vec<State> = (0..8)
                .map(|s| State { vel: v3(32 * s), ang: v3(32 * s + 4), mcol: [v3(32 * s + 8), v3(32 * s + 12), v3(32 * s + 16)], icol: [v3(32 * s + 20), v3(32 * s + 24), v3(32 * s + 28)] })
                .collect();
            rest_states(&mut states[..count]);
            for (s, st) in states.iter().enumerate() {
                let vs = [st.vel, st.ang, st.mcol[0], st.mcol[1], st.mcol[2], st.icol[0], st.icol[1], st.icol[2]];
                for (k, v) in vs.iter().enumerate() {
                    let at = 32 * s + 4 * k;
                    let same = |a: f32, b: u32| a.to_bits() == b || (a.is_nan() && f32::from_bits(b).is_nan());
                    assert!(same(v.x, o[at]) && same(v.y, o[at + 1]) && same(v.z, o[at + 2]), "state {s} vector {k}: {l}");
                    if s < count && k < 2 {
                        assert_eq!(o[at + 3], 0, "w lane of a zeroed vector: {l}");
                    }
                }
            }
            n += 1;
        }
        assert_eq!(n, 1500);
    }
}

#[cfg(test)]
mod island_small_tests {
    use super::*;

    fn lines(tag: &'static str) -> impl Iterator<Item = (Vec<u32>, Vec<u32>)> {
        let w = |s: &str| -> Vec<u32> { s.split_whitespace().map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap()).collect() };
        include_str!("../testdata/island_small_ref.txt").lines().filter_map(move |l| {
            let rest = l.strip_prefix(tag)?.strip_prefix(' ')?;
            let (i, o) = rest.split_once(" -> ")?;
            Some((w(i), w(o)))
        })
    }

    fn same(a: f32, b: u32) -> bool {
        a.to_bits() == b || (a.is_nan() && f32::from_bits(b).is_nan())
    }

    /// F-71376 run offline with its callee F-78198 recorded (tools/island_small_reference.py): a side is applied exactly when
    /// the manifold has its collider (non-null, id not 0xffff); A with the impulse at A's point, B with the impulse taken from
    /// zero at B's point; the manifold's instance (+8 / +12) and part (+28 / +36) words ride along as r4 and r6.
    #[test]
    fn contact_applications_match_the_original() {
        let (mut n, mut both) = (0, 0);
        for (i, o) in lines("apply") {
            let f = |k: usize| f32::from_bits(i[k]);
            let v = |k: usize| Vec3::new(f(k), f(k + 1), f(k + 2));
            let gate = |has: u32, id: u32| has != 0 && (id & 0xFFFF) != 0xFFFF;
            let (ga, gb) = (gate(i[9], i[10]), gate(i[11], i[12]));
            let mut c = Touch { impulse: v(0), ..Default::default() };
            c.a.point = v(3);
            c.b.point = v(6);
            c.a.body = ga.then_some(0);
            c.b.body = gb.then_some(0);
            let calls = applications(&c);
            assert_eq!(o[0] as usize, ga as usize + gb as usize, "calls {i:x?}");
            // (the original makes A's call first, then B's, each into the next record)
            let mut rec = 0;
            for (k, call) in calls.iter().enumerate() {
                let Some((j, p)) = call else { continue };
                let r = &o[1 + 9 * rec..1 + 9 * rec + 9];
                assert!(same(j.x, r[3]) && same(j.y, r[4]) && same(j.z, r[5]), "impulse {k} {i:x?}");
                assert!(same(p.x, r[6]) && same(p.y, r[7]) && same(p.z, r[8]), "point {k} {i:x?}");
                assert_eq!(r[0], i[13 + k], "instance {k}");
                assert_eq!(r[2], i[15 + k], "part {k}");
                assert_ne!(r[1], 0);
                rec += 1;
            }
            both += (ga && gb) as u32;
            n += 1;
        }
        assert_eq!(n, 3000);
        assert!(both > 100);
    }

    /// F-43928 run offline with the collider's slot 22 and the instance's slot 6 recorded: zero stored first; with a collider its
    /// answer times `less` (the port's `pend_of`), or as it is where the flag byte (r10 & 0xff) is set; the step, the part and the
    /// point reach the collider as given. The instance-only way (slot 6, no collider) is not in the port and is not compared
    /// but for what it is given.
    #[test]
    fn pending_velocity_matches_the_original() {
        let (mut n, mut by_collider, mut zero) = (0, 0, 0);
        for (i, o) in lines("pend") {
            let f = |k: usize| f32::from_bits(i[k]);
            let (has_collider, has_instance, unscaled, less) = (i[0] != 0, i[1] != 0, i[2] & 0xFF != 0, f(4));
            let res = Vec3::new(f(9), f(10), f(11));
            let got = pend_of(has_collider.then_some(res), unscaled, less);
            assert_eq!(o[0], 1, "returns its out pointer");
            if has_collider {
                by_collider += 1;
                assert!(same(got.x, o[1]) && same(got.y, o[2]) && same(got.z, o[3]), "{i:x?} -> {o:x?}: {got:?}");
                // (what the collider is given: dt, the part, the point)
                assert_eq!(&o[5..10], &[i[3], i[8], i[5], i[6], i[7]], "arguments {i:x?}");
            } else if has_instance {
                // (the instance's answer is written as it is; the port has no such side)
                assert_eq!(&o[1..4], &i[13..16]);
                assert_eq!(&o[10..14], &[i[8], i[5], i[6], i[7]][..], "instance arguments");
            } else {
                zero += 1;
                assert!(got == Vec3::ZERO && o[1..5] == [0, 0, 0, 0], "{i:x?} -> {o:x?}");
            }
            n += 1;
        }
        assert_eq!(n, 4000);
        assert!(by_collider > 2000 && zero > 200);
    }

    /// F-50131 run offline with F-67704 recorded: the sweeps run when the island's +52 or +16 is above zero, with the count
    /// F-42898 picks from the island's byte +80 and +20 and the three globals, and the relaxation 1.1 of the port's `OMEGA`;
    /// byte +78 is 1 either way.
    #[test]
    fn velocity_solve_sweeps_match_the_original() {
        let (mut n, mut ran) = (0, 0);
        for (i, o) in lines("sweeps") {
            let (rows, others) = (i[3] as i32, i[4] as i32);
            assert_eq!(o[0] == 1, runs_sweeps(rows, others), "{i:x?}");
            assert_eq!(o[5], 1, "byte +78");
            if o[0] == 1 {
                let sweeps = sweeps_count(i[6] & 0xFF != 0, i[5] as i32 != 0, i[0] as i32, i[1] as i32, i[2] as i32);
                assert_eq!(o[3] as i32, sweeps, "{i:x?}");
                assert_eq!(o[4], OMEGA.to_bits());
                ran += 1;
            }
            n += 1;
        }
        assert_eq!(n, 3000);
        assert!(ran > 1500);
        // (what the port runs: 4 sweeps, 8 with a jointed body)
        assert_eq!((sweeps_count(false, false, SWEEPS as i32, 8, 32), sweeps_count(false, true, SWEEPS as i32, SWEEPS_JOINTED as i32, 32)), (4, 8));
    }
}
