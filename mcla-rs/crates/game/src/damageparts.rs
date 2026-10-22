//! The moving damage parts of a car as the original's car anim object has them (D = mcCar+84, per-frame
//! `sub_823424F0`): the hood and the trunk lid ("flaps", D+1456 / D+1664) - a virtual hinged particle that falls
//! open once the front / rear zone is wrecked - and the two bumpers (mcCarAnimBreakablePartInst, *(D+1876) front,
//! *(D+1872) rear) - two hanging ends, attached / loose / broken, that sag, shake and finally tear off.
//! Rules: `docs/reference/audio/DAMAGE_PARTS_RULE.md`; log in `docs/CAR_PARTS.md` ("Moving damage parts").
//!
//! Matrices: the original's rows-are-axes matrices are glam column matrices here (row0 X = `x_axis`, row3 =
//! `w_axis`); `C` is the car's world matrix (the port's `rot` / `pos`, the model origin).
//!
//! The fallen-off bumper is a body of the car's loose-body level (`physics/loose.rs` "A car's torn-off bumper": the
//! original's phInst with its archetype, bound, material and sleep), against the city, the car and what is loose
//! about it. Approximate (written down in the doc): other cars do not meet it (each car's level holds only its own
//! pieces), and a car without the exact simulation keeps the small ground-only body below; the gates (car+80)->+41 and the
//! model-class byte +122 are taken as "on" for every car; repair and teleport are found by what they do to the car
//! (damage cleared / a jump of the car) rather than called by the game's code.

use crate::physics::loose::{LooseState, LooseWorld, PieceContact};
use glam::{Mat4, Quat, Vec3};

/// mcCarModelTune +184..+196 defaults when a car has no vehModel (the instance's constructor: bumper max state 2;
/// flaps not locked, not reversed)
pub const DEFAULT_MOVEMENT: [u32; 4] = [2, 2, 1, 1];

const GRAVITY: Vec3 = Vec3::new(0.0, -9.8, 0.0);

/// One impact queued for the bumpers (D+1888 + 32 i, at most 8 a frame): world point, amount, kind (0 = collision).
#[derive(Clone, Copy, Debug)]
pub struct PartHit {
    pub point: Vec3,
    pub amount: f32,
    pub kind: u8,
}

/// What the parts did this tick, for the sounds (`caraudio.rs`). `serial` grows by one per event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PartEvent {
    /// a flap (0 hood, 1 trunk) hit its closed stop (`open` false) or its open stop at angular speed `rate` (rad/s)
    FlapStop { flap: u8, open: bool, rate: f32, pos: Vec3 },
    /// a bumper (0 rear, 1 front) tore off (sub_8234A868)
    BumperOff { bumper: u8, pos: Vec3 },
    /// the fallen bumper struck something: the largest approach speed (m/s) of its new contacts this tick
    BumperImpact { bumper: u8, speed: f32, pos: Vec3 },
    /// the fallen bumper slides on something at `speed` m/s
    BumperScrape { bumper: u8, speed: f32, pos: Vec3 },
}

// ---------------------------------------------------------------- Perlin (sub_8261E060)

/// Ken Perlin's improved-noise permutation (0x828252B8).
const PERM: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148,
    247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54,
    65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64,
    52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213,
    119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104,
    218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157,
    184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
];
/// x of the 16 improved-noise gradients (0x82825AC0)
const GRAD_X: [f32; 16] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, -1.0, 0.0, 0.0];

/// Improved noise in one dimension.
pub fn perlin(x: f32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let i = (i as i32 & 255) as usize;
    let g0 = GRAD_X[(PERM[i] & 15) as usize] * f;
    let g1 = GRAD_X[(PERM[(i + 1) & 255] & 15) as usize] * (f - 1.0);
    let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    g0 + (g1 - g0) * s
}

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------- flaps (hood, trunk)

/// The hood or the trunk lid: a unit-length rod hinged at (0, 2, 0) in car space, closed along +z.
#[derive(Clone, Debug)]
pub struct Flap {
    /// a = 0.2 (hood) / 0.15 (trunk): the open stop
    pub ang_open: f32,
    /// flap+88: falls open (set when the zone reaches 1, cleared by a repair)
    pub loose: bool,
    /// flap+189 NoMovement (drawn and heard at 0), flap+188 ReverseMovement (drawn the other way)
    pub locked: bool,
    pub reversed: bool,
    /// the particle (world) now and a frame ago
    pub p: Vec3,
    pub q: Vec3,
    /// flap+180 (<= 0: open) and flap+184 (last frame's)
    pub angle: f32,
    pub prev_angle: f32,
}

const FLAP_P0: Vec3 = Vec3::new(0.0, 2.0, 0.0);
const FLAP_R: Vec3 = Vec3::new(0.0, 2.0, 1.0);

impl Flap {
    pub fn new(a: f32, movement: u32) -> Flap {
        Flap { ang_open: a, loose: false, locked: movement == 0, reversed: movement == 2, p: Vec3::ZERO, q: Vec3::ZERO, angle: 0.0, prev_angle: 0.0 }
    }

    /// `sub_8233C638`: back to rest (closed) at the car's matrix.
    pub fn reset(&mut self, c: &Mat4) {
        self.p = c.transform_point3(FLAP_R);
        self.q = self.p;
        self.angle = 0.0;
        self.prev_angle = 0.0;
    }

    /// `sub_8233F140` (from `sub_82341330`) for one frame; returns a stop event.
    fn step(&mut self, c: &Mat4, ci: &Mat4, speed: f32, dt: f32, which: u8) -> Option<PartEvent> {
        if !self.loose {
            self.angle = 0.0;
            return None;
        }
        let hinge_w = c.transform_point3(FLAP_P0);
        // world-space Verlet, gravity only
        let p = self.p + (self.p - self.q) * (dt * 30.0) + GRAVITY * dt * dt;
        self.q = self.p;
        // to car space, with the wind lift (sub_8233C740)
        let mut l = ci.transform_point3(p);
        let o = c.w_axis;
        let lift = clamp01(0.2 * speed) * clamp01(perlin(5.0 * (o.y + o.z + o.x))) * 0.05 * clamp01(1.0 + self.angle / 0.05);
        l.y += lift * dt * 30.0;
        let mut d = (l - FLAP_P0).normalize_or(Vec3::Z);
        let t = d.y.atan2(d.z);
        let rate = (self.prev_angle - self.angle).abs() / dt.max(1e-6);
        let mut ev = None;
        if t < 0.0 {
            d = Vec3::Z;
            ev = Some(PartEvent::FlapStop { flap: which, open: false, rate, pos: p });
        } else if t > self.ang_open {
            // Po - P0 = (0, sin a, cos a)
            d = Vec3::new(0.0, self.ang_open.sin(), self.ang_open.cos());
            ev = Some(PartEvent::FlapStop { flap: which, open: true, rate, pos: p });
        }
        // L = P0 + d*len with its x then set to the rest point's (flap+32, 0): the particle goes back into the car's
        // YZ plane, shorter by what it had swung sideways (0x8233F3FC `stfs f0,80(r1)`; recorded 2026-10-19)
        let mut l = FLAP_P0 + d;
        l.x = FLAP_R.x;
        self.p = c.transform_point3(l);
        let rest_w = c.transform_point3(FLAP_R);
        self.prev_angle = self.angle;
        let dot = (self.p - hinge_w).normalize_or_zero().dot((rest_w - hinge_w).normalize_or_zero());
        self.angle = -dot.clamp(-1.0, 1.0).acos();
        // (the one-shot plays at flap+112 as it is at the check - the integrated particle, P+20)
        ev
    }

    fn sign(&self) -> f32 {
        if self.reversed { -1.0 } else { 1.0 }
    }

    /// `sub_8233C850`: the drawn angle (signed)
    pub fn drawn(&self) -> f32 {
        self.sign() * if self.locked { 0.0 } else { self.angle }
    }

    /// `sub_8233C890`: last frame's angle (not zeroed when locked)
    pub fn drawn_prev(&self) -> f32 {
        self.sign() * self.prev_angle
    }
}

// ---------------------------------------------------------------- bumpers

/// One bumper end: world position now / before, car-local now / before.
#[derive(Clone, Copy, Debug, Default)]
pub struct End {
    pub rest: Vec3,
    pub pos: Vec3,
    pub prev: Vec3,
    pub local: Vec3,
    pub local_prev: Vec3,
    /// 0 attached, 1 loose, 2 broken
    pub state: u8,
}

/// The piece once it has torn off: a box 1.8 x 0.333 x 0.5, mass 4. Its frame (pos, rot) is the level's body's
/// (`level`) or, where the car has no level, the small ground-only body's (`own_body`).
#[derive(Clone, Copy, Debug)]
pub struct Loose {
    /// the piece in the car's level (`LooseState::add_piece`)
    pub level: Option<u64>,
    /// moved by `step_loose` (no level took it)
    pub own_body: bool,
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub ang_vel: Vec3,
    /// contacts of the last tick (a contact is "new" when the corner was not touching before)
    pub touching: u8,
    /// at rest on the ground for this long; the body sleeps after 1 s
    pub still: f32,
}

#[derive(Clone, Debug)]
pub struct Bumper {
    pub front: bool,
    /// M_rest: the slot bone's matrix in car space (slot 6 bumper_f / slot 3 bumper_r)
    pub m_rest: Mat4,
    pub a: End,
    pub b: End,
    /// +480
    pub clock: f32,
    /// +536: 0 NoMovement, 1 AllowLoose, 2 AllowBroken
    pub max_state: u8,
    /// +485 / the rigid body
    pub detached: Option<Loose>,
    /// the pose delta F_local (car space), identity while both ends are attached
    pub pose: Mat4,
}

impl Bumper {
    pub fn new(front: bool, m_rest: Mat4, movement: u32) -> Bumper {
        let mut b = Bumper { front, m_rest, a: End::default(), b: End::default(), clock: 0.0, max_state: movement.min(2) as u8, detached: None, pose: Mat4::IDENTITY };
        // `sub_8233F648`: the half vector (0.5, 0, 0) both ways
        b.a.rest = m_rest.transform_point3(Vec3::new(0.5, 0.0, 0.0));
        b.b.rest = m_rest.transform_point3(Vec3::new(-0.5, 0.0, 0.0));
        b
    }

    /// `sub_8233CF78`: both ends attached at the car's matrix, the free piece removed.
    pub fn reset(&mut self, c: &Mat4) {
        for e in [&mut self.a, &mut self.b] {
            e.state = 0;
            e.pos = c.transform_point3(e.rest);
            e.prev = e.pos;
            e.local = e.rest;
            e.local_prev = e.rest;
        }
        self.detached = None;
        self.pose = Mat4::IDENTITY;
    }

    /// `sub_8233F738`: a queued hit changes the ends' states.
    fn hit(&mut self, c: &Mat4, h: &PartHit, heavy: bool) {
        if h.kind != 0 {
            return;
        }
        // (pA = C*restA + carVel*dt*K, K = 0x8288DCA8: 0 in the image, no writer found)
        if (h.point - c.transform_point3(self.a.rest)).length() < 0.8 && h.amount > 0.15 {
            if self.a.state == 0 && self.b.state == 0 {
                self.a.state = 1;
            } else if self.a.state == 1 && heavy {
                self.a.state = 2;
            }
            if self.b.state == 2 {
                self.a.state = 2;
            }
        }
        if (h.point - c.transform_point3(self.b.rest)).length() < 0.8 && h.amount > 0.15 {
            if self.b.state == 0 && self.a.state == 0 {
                self.b.state = 1;
            } else if self.b.state == 1 && heavy {
                self.b.state = 2;
            }
            if self.a.state == 2 {
                self.b.state = 2;
            }
        }
    }

    /// `sub_823414C8` for one frame.
    fn step(&mut self, c: &Mat4, ci: &Mat4, speed: f32, dt: f32, which: u8, ground: &dyn Fn(Vec3) -> Option<(f32, Vec3)>, events: &mut Vec<PartEvent>) {
        self.a.state = self.a.state.min(self.max_state);
        self.b.state = self.b.state.min(self.max_state);
        if let Some(body) = self.detached.as_mut() {
            if body.own_body {
                step_loose(body, dt, which, ground, events);
            }
            // A = (instMatrix * C^-1) * M_rest^-1: the piece's frame in car space
            self.pose = *ci * Mat4::from_rotation_translation(body.rot, body.pos);
            return;
        }
        if self.a.state == 0 && self.b.state == 0 {
            self.pose = Mat4::IDENTITY;
            return;
        }
        // 1 forces (sub_8233D488)
        let k = clamp01(speed / 50.0);
        self.clock += k;
        let mut force = [Vec3::ZERO; 2];
        for (i, e) in [&self.a, &self.b].into_iter().enumerate() {
            let re = c.transform_point3(e.rest);
            let (f, w) = match e.state {
                0 => (Vec3::ZERO, 1.0),
                1 => (GRAVITY + Vec3::new(0.0, 15.0 * (2.0 * self.clock).sin() * k + 5.0 * perlin(self.clock) * k, 0.0), 0.1),
                _ => (GRAVITY, 0.0),
            };
            force[i] = f + (re - e.pos) * 1000.0 * w;
        }
        // 2 Verlet (sub_8233D7A8)
        for (e, f) in [&mut self.a, &mut self.b].into_iter().zip(force) {
            let p = e.pos + (e.pos - e.prev) * (0.9 * dt * 30.0) + f * dt * dt;
            e.prev = e.pos;
            e.pos = p;
        }
        // 3 constraints in car space (sub_8233FC20)
        let front = self.front;
        for e in [&mut self.a, &mut self.b] {
            if e.state == 0 {
                e.pos = c.transform_point3(e.rest);
            }
            let mut d = ci.transform_point3(e.pos) - e.rest;
            let l = if e.state < 2 { Vec3::new(0.01, 0.05, 0.15) } else { Vec3::new(0.01, 0.2, 10.0) };
            d.x = d.x.clamp(-l.x, l.x);
            d.y = d.y.clamp(-l.y, 0.0);
            d.z = if front { d.z.clamp(-l.z, 0.0) } else { d.z.clamp(0.0, l.z) };
            e.pos = c.transform_point3(e.rest + d);
            e.local_prev = e.local;
            e.local = e.rest + d;
        }
        let len = (self.a.rest - self.b.rest).length();
        if self.a.state == 2 {
            self.a.pos = self.b.pos + (self.a.pos - self.b.pos).normalize_or_zero() * len;
        } else if self.b.state == 2 {
            self.b.pos = self.a.pos + (self.b.pos - self.a.pos).normalize_or_zero() * len;
        }
        // 4 tearing (sub_8233D2F0)
        if self.a.state == 2 && (self.a.prev - c.transform_point3(self.a.rest)).length() > 0.85 {
            self.b.state = 2;
        } else if self.b.state == 2 && (self.b.prev - c.transform_point3(self.b.rest)).length() > 0.85 {
            self.a.state = 2;
        }
        // 5 the bumper's frame (world)
        let up = c.y_axis.truncate();
        let mid = (self.a.pos + self.b.pos) * 0.5;
        let x = (mid - self.b.pos).normalize_or(c.x_axis.truncate());
        let z = x.cross(up).normalize_or(c.z_axis.truncate());
        let y = z.cross(x).normalize_or(up);
        let f = Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), mid.extend(1.0));
        // 6 both ends broken: the piece comes off (2.4)
        if self.a.state == 2 && self.b.state == 2 {
            let va = self.a.pos - self.a.prev;
            let vb = self.b.pos - self.b.prev;
            let (_, rot, _) = f.to_scale_rotation_translation();
            let spin = (self.a.pos - self.b.pos).cross(va - vb) * -30.0 + c.transform_vector3(Vec3::new(-3.0, 0.0, 0.0));
            self.detached = Some(Loose { level: None, own_body: false, pos: mid, rot, vel: (va + vb) * 0.5 * 30.0, ang_vel: spin, touching: 0, still: 0.0 });
            events.push(PartEvent::BumperOff { bumper: which, pos: mid });
            self.pose = *ci * f;
            return;
        }
        // 7 pose: F in car space
        self.pose = *ci * f;
    }

    /// The bumper's place in the drawn car: the rest of the car space maps the slot's mesh by
    /// M_slot' = M_slot * A with A = M_rest^-1 * F_local, its axes scaled by 0.9995 (0x827E6344).
    pub fn draw_delta(&self) -> Mat4 {
        let a = if self.a.state == 0 && self.b.state == 0 && self.detached.is_none() { Mat4::IDENTITY } else { self.m_rest.inverse() * self.pose };
        self.m_rest * scale_axes(a, 0.9995) * self.m_rest.inverse()
    }

    /// `sub_8233D208`: the fastest moving end relative to the car (m/s) and where it is; None when both are gone.
    pub fn creak(&self, dt: f32) -> Option<(f32, Vec3)> {
        if (self.a.state == 2 && self.b.state == 2) || self.detached.is_some() {
            return None;
        }
        let mut best: Option<(f32, Vec3)> = None;
        for e in [&self.a, &self.b] {
            if e.state != 0 {
                let v = (e.local - e.local_prev).length() / dt.max(1e-6);
                if best.map_or(true, |b| v >= b.0) {
                    best = Some((v, e.pos));
                }
            }
        }
        Some(best.unwrap_or((0.0, self.a.pos)))
    }
}

fn scale_axes(m: Mat4, s: f32) -> Mat4 {
    Mat4::from_cols(m.x_axis * s, m.y_axis * s, m.z_axis * s, m.w_axis)
}

/// The fallen-off piece: a box (half extents 0.9 x 0.1665 x 0.25) under gravity, its eight corners against the
/// ground under them, an impulse and friction at each touching corner (port's own; the original's phInst is open).
fn step_loose(b: &mut Loose, dt: f32, which: u8, ground: &dyn Fn(Vec3) -> Option<(f32, Vec3)>, events: &mut Vec<PartEvent>) {
    if b.still > 0.5 {
        return;
    }
    const HALF: Vec3 = Vec3::new(0.9, 0.1665, 0.25);
    const MASS: f32 = 4.0;
    // the inertia of a box 2.25 x 0.416 x 0.625 (x 1.25 of the bound, sub_825A42C8)
    let s = HALF * 2.0 * 1.25;
    let inertia = Vec3::new(s.y * s.y + s.z * s.z, s.x * s.x + s.z * s.z, s.x * s.x + s.y * s.y) * (MASS / 12.0);
    b.vel += GRAVITY * dt;
    b.pos += b.vel * dt;
    if b.ang_vel.length_squared() > 1e-8 {
        b.rot = (Quat::from_scaled_axis(b.ang_vel * dt) * b.rot).normalize();
    }
    let mut touching = 0u8;
    let (mut impact, mut scrape, mut at) = (0.0f32, 0.0f32, b.pos);
    // the corners under the ground: lift the piece out by the deepest, then an impulse and friction at each
    let mut contacts: Vec<(u8, Vec3, Vec3)> = Vec::new();
    let mut lift = 0.0f32;
    for k in 0..8u8 {
        let corner = Vec3::new(if k & 1 != 0 { HALF.x } else { -HALF.x }, if k & 2 != 0 { HALF.y } else { -HALF.y }, if k & 4 != 0 { HALF.z } else { -HALF.z });
        let w = b.pos + b.rot * corner;
        let Some((h, n)) = ground(w) else { continue };
        let pen = h - w.y;
        if pen > 0.0 {
            lift = lift.max(pen);
            contacts.push((k, w, n));
        }
    }
    b.pos.y += lift;
    for (pass, &(k, w, n)) in contacts.iter().enumerate().flat_map(|(_, c)| [(0, c), (1, c)]) {
        touching |= 1 << k;
        let w = w + Vec3::Y * lift;
        let r = w - b.pos;
        let v = b.vel + b.ang_vel.cross(r);
        let vn = v.dot(n);
        // (sub_8234CF80: an approach - dot(n, v) at least 0.1 of the speed - on a contact that was not there
        // before is an impact; any other contact is a slide at the whole relative speed)
        let speed = v.length();
        if pass == 1 {
        } else if b.touching & (1 << k) == 0 && speed > 1e-4 && (-vn) / speed >= 0.1 {
            impact = impact.max(-vn);
            at = w;
        } else if speed > scrape {
            scrape = speed;
            at = w;
        }
        if vn < 0.0 {
            let rn = r.cross(n);
            let k_inv = 1.0 / MASS + (rn * rn / inertia).dot(Vec3::ONE);
            let j = -(1.0 + if pass == 0 { 0.2 } else { 0.0 }) * vn / k_inv;
            b.vel += n * j / MASS;
            b.ang_vel += rot_inv_apply(b.rot, rn * j, inertia);
            // friction 0.6 on the sliding part
            let vt = v - n * vn;
            let t = vt.normalize_or_zero();
            let rt = r.cross(t);
            let kt = 1.0 / MASS + (rt * rt / inertia).dot(Vec3::ONE);
            let jt = (vt.length() / kt).min(0.6 * j);
            b.vel -= t * jt / MASS;
            b.ang_vel -= rot_inv_apply(b.rot, rt * jt, inertia);
        }
    }
    if touching != 0 {
        b.ang_vel *= 0.9;
        if Vec3::new(b.vel.x, 0.0, b.vel.z).length() < 0.3 && b.vel.y.abs() < 0.5 && b.ang_vel.length() < 0.5 {
            b.still += dt;
        } else {
            b.still = 0.0;
        }
    }
    b.touching = touching;
    if impact > 0.0 {
        events.push(PartEvent::BumperImpact { bumper: which, speed: impact, pos: at });
    }
    if scrape > 0.0 {
        events.push(PartEvent::BumperScrape { bumper: which, speed: scrape, pos: at });
    }
}

/// An angular impulse to an angular velocity change (inertia in the body's axes).
fn rot_inv_apply(rot: Quat, l: Vec3, inertia: Vec3) -> Vec3 {
    rot * ((rot.inverse() * l) / inertia)
}

// ---------------------------------------------------------------- the whole car

#[derive(Clone, Debug)]
pub struct DamageParts {
    pub hood: Flap,
    pub trunk: Flap,
    /// rear, front (their own slots); None when the car's skeleton has no bumper bone
    pub bumpers: [Option<Bumper>; 2],
    /// the impacts of this tick (filled by `CarState::add_dent`)
    pub hits: Vec<PartHit>,
    /// the parts' events, newest last, with a running count (the sounds take those they have not seen)
    pub events: Vec<(u64, PartEvent)>,
    pub serial: u64,
    /// the car matrix of the last step (teleport test) and whether the parts were placed yet
    last_c: Mat4,
    placed: bool,
    /// the car's own set-up has been applied (rest matrices, movement)
    pub rigged: bool,
    /// game time not yet stepped: the parts move once a game frame (1/30 s, T+8 live) while the port ticks at 60 Hz
    frame_acc: f32,
    /// the torn bumpers this car has put into its level
    level_ids: Vec<u64>,
}

impl Default for DamageParts {
    fn default() -> Self {
        DamageParts {
            hood: Flap::new(0.2, 1),
            trunk: Flap::new(0.15, 1),
            bumpers: [None, None],
            hits: Vec::new(),
            events: Vec::new(),
            serial: 0,
            last_c: Mat4::IDENTITY,
            placed: false,
            rigged: false,
            frame_acc: 0.0,
            level_ids: Vec::new(),
        }
    }
}

/// A torn bumper as the level that holds it has it: its frame (rows, place) and its enabled contacts of the tick.
#[derive(Clone, Debug)]
pub struct PieceView {
    pub matrix: [Vec3; 4],
    pub contacts: Vec<PieceContact>,
}

/// What the car's model gives the parts: the movement settings (front bumper, rear bumper, hood, trunk) and the
/// bumper slots' bone matrices in car space (rear, front).
#[derive(Clone, Copy, Debug, Default)]
pub struct PartRig {
    pub movement: [u32; 4],
    pub bumper_rest: [Option<Mat4>; 2],
}

impl DamageParts {
    /// `sub_823404E0`: the per-car settings.
    pub fn rig(&mut self, rig: &PartRig) {
        let [front, rear, hood, trunk] = rig.movement;
        self.hood = Flap::new(0.2, hood);
        self.trunk = Flap::new(0.15, trunk);
        self.bumpers = [rig.bumper_rest[0].map(|m| Bumper::new(false, m, rear)), rig.bumper_rest[1].map(|m| Bumper::new(true, m, front))];
        self.rigged = true;
        self.placed = false;
    }

    /// Any part out of its rest state.
    pub fn damaged(&self) -> bool {
        self.hood.loose || self.trunk.loose || self.bumpers.iter().flatten().any(|b| b.a.state != 0 || b.b.state != 0 || b.detached.is_some())
    }

    /// `sub_8233E1B0`: the repair - flaps shut and not loose, bumpers on.
    pub fn repair(&mut self, c: &Mat4) {
        for f in [&mut self.hood, &mut self.trunk] {
            f.loose = false;
            f.reset(c);
        }
        for b in self.bumpers.iter_mut().flatten() {
            b.reset(c);
        }
        self.hits.clear();
    }

    /// Queue an impact (at most 8 a frame, D+2144).
    pub fn queue(&mut self, h: PartHit) {
        if self.hits.len() < 8 {
            self.hits.push(h);
        }
    }

    fn push(&mut self, e: PartEvent) {
        self.serial += 1;
        self.events.push((self.serial, e));
        if self.events.len() > 32 {
            self.events.remove(0);
        }
    }

    /// The game frame's length the parts run at: the original's frame block T+8, 1/30 s in the running game
    /// (docs/reference/vehicle/bikegyro_pseudo.md, live values).
    pub const FRAME: f32 = 1.0 / 30.0;

    /// Advance by a port tick of `dt`: the parts step once for every whole game frame (1/30 s) that has passed; the
    /// torn bumper of this frame goes into the car's level (`level`: the level's types and the car's loose bodies,
    /// None for a car without them); `read_pieces` brings the pieces' frames back every tick.
    pub fn tick(&mut self, c: Mat4, speed: f32, heavy: bool, dt: f32, ground: &dyn Fn(Vec3) -> Option<(f32, Vec3)>, level: Option<(&LooseWorld, &mut LooseState)>) {
        self.frame_acc += dt;
        if self.frame_acc + 1e-5 >= Self::FRAME {
            self.frame_acc = (self.frame_acc - Self::FRAME).max(0.0);
            self.step(c, speed, heavy, Self::FRAME, ground);
        }
        self.pieces(level);
    }

    /// The torn bumpers and the car's level: a new one goes in at its frame with its velocity and spin (2.4). The
    /// level may later hand it to another car's (`loose::hand_over_pieces`); `dropped` and `read_pieces` find it there.
    fn pieces(&mut self, level: Option<(&LooseWorld, &mut LooseState)>) {
        let Some((world, state)) = level else {
            for b in self.bumpers.iter_mut().flatten() {
                if let Some(body) = b.detached.as_mut().filter(|d| d.level.is_none()) {
                    body.own_body = true;
                }
            }
            return;
        };
        for b in self.bumpers.iter_mut().flatten() {
            let Some(body) = b.detached.as_mut().filter(|d| !d.own_body && d.level.is_none()) else { continue };
            let f = Mat4::from_rotation_translation(body.rot, body.pos);
            let m = [f.x_axis.truncate(), f.y_axis.truncate(), f.z_axis.truncate(), f.w_axis.truncate()];
            match state.add_piece(world, m, body.vel, body.ang_vel) {
                Some(id) => {
                    body.level = Some(id);
                    self.level_ids.push(id);
                }
                None => body.own_body = true,
            }
        }
    }

    /// The pieces this car put into a level that its bumpers no longer have (repair, reset): to be taken out of
    /// whichever car's level holds them now (sub_8233CF78).
    pub fn dropped(&mut self) -> Vec<u64> {
        let live: Vec<u64> = self.bumpers.iter().flatten().filter_map(|b| b.detached.and_then(|d| d.level)).collect();
        let out = self.level_ids.iter().copied().filter(|id| !live.contains(id)).collect();
        self.level_ids.retain(|id| live.contains(id));
        out
    }

    /// Each piece's frame and contacts read back from the level that holds it (`find`, by id), every tick: the drawn
    /// pose and the impact and scrape sounds (sub_8234CF80, 2.5).
    pub fn read_pieces(&mut self, c: &Mat4, find: &dyn Fn(u64) -> Option<PieceView>) {
        let ci = c.inverse();
        let mut evs = Vec::new();
        for (i, b) in self.bumpers.iter_mut().enumerate() {
            let Some(b) = b.as_mut() else { continue };
            let Some(body) = b.detached.as_mut().filter(|d| !d.own_body) else { continue };
            let Some(id) = body.level else { continue };
            let Some(view) = find(id) else {
                // (no level holds it any more - the cars' levels were made afresh: the piece stays where it was)
                body.level = None;
                body.own_body = true;
                body.still = 1.0;
                continue;
            };
            let m = view.matrix;
            let f = Mat4::from_cols(m[0].extend(0.0), m[1].extend(0.0), m[2].extend(0.0), m[3].extend(1.0));
            let (_, rot, pos) = f.to_scale_rotation_translation();
            body.rot = rot;
            body.pos = pos;
            b.pose = ci * f;
            // (sub_8234CF80: of the piece's enabled contacts this tick, an approach at least 0.1 of the speed is an
            // impact - the largest counts - any other a slide; the thresholds in mph are the sound's, caraudio.rs)
            let (mut impact, mut scrape, mut at_i, mut at_s) = (0.0f32, 0.0f32, m[3], m[3]);
            for k in view.contacts.iter() {
                let speed = k.v.length();
                if speed > 0.0 && k.n.dot(k.v / speed) >= 0.1 {
                    let a = k.v.dot(k.n);
                    if a > impact {
                        impact = a;
                        at_i = k.point;
                    }
                } else if speed > scrape {
                    scrape = speed;
                    at_s = k.point;
                }
            }
            if impact > 0.0 {
                evs.push(PartEvent::BumperImpact { bumper: i as u8, speed: impact, pos: at_i });
            }
            if scrape > 0.0 {
                evs.push(PartEvent::BumperScrape { bumper: i as u8, speed: scrape, pos: at_s });
            }
        }
        for e in evs {
            self.push(e);
        }
    }

    /// `sub_823424F0` for one car and one frame. `heavy` = damage / MaxDamage > 0.66; `ground(p)` = the ground's
    /// height and normal under a point (for the fallen-off bumpers).
    pub fn step(&mut self, c: Mat4, speed: f32, heavy: bool, dt: f32, ground: &dyn Fn(Vec3) -> Option<(f32, Vec3)>) {
        if !self.placed {
            self.placed = true;
            self.repair(&c);
            self.last_c = c;
        }
        // (sub_823411B8, the teleport: the flaps re-rested, the bumpers' points carried with the car - here a
        // jump of more than 10 m in a frame)
        if (c.w_axis - self.last_c.w_axis).truncate().length() > 10.0 {
            let carry = c * self.last_c.inverse();
            for f in [&mut self.hood, &mut self.trunk] {
                f.reset(&c);
            }
            for b in self.bumpers.iter_mut().flatten() {
                for e in [&mut b.a, &mut b.b] {
                    e.pos = carry.transform_point3(e.pos);
                    e.prev = carry.transform_point3(e.prev);
                }
            }
        }
        let ci = c.inverse();
        // the queued hits, front bumper first, then the rear
        let hits = std::mem::take(&mut self.hits);
        for h in &hits {
            for b in self.bumpers.iter_mut().rev().flatten() {
                b.hit(&c, h, heavy);
            }
        }
        let mut evs = Vec::new();
        // trunk, hood, rear bumper, front bumper (sub_82341DB8)
        if let Some(e) = self.trunk.step(&c, &ci, speed, dt, 1) {
            evs.push(e);
        }
        if let Some(e) = self.hood.step(&c, &ci, speed, dt, 0) {
            evs.push(e);
        }
        for (i, b) in self.bumpers.iter_mut().enumerate() {
            if let Some(b) = b.as_mut() {
                b.step(&c, &ci, speed, dt, i as u8, ground, &mut evs);
            }
        }
        for e in evs {
            self.push(e);
        }
        self.last_c = c;
    }

    /// The hood's pose in car space for a part on the hood slot whose bone matrix is `slot`: M_slot * RotX(-(s*ang))
    /// with the axes scaled by 1.0005 (0x827E6348), as a delta on the car-space drawing.
    pub fn hood_delta(&self, slot: &Mat4) -> Mat4 {
        let a = Mat4::from_rotation_x(-self.hood.drawn());
        *slot * scale_axes(a, 1.0005) * slot.inverse()
    }

    /// The trunk bone turned in place by RotX(s*ang) (sub_82227DE0), as a delta on the car-space drawing.
    pub fn trunk_delta(&self, bone: &Mat4) -> Mat4 {
        *bone * Mat4::from_rotation_x(self.trunk.drawn()) * bone.inverse()
    }
}

/// The gate of the zone damage (`sub_82356D98` @82357444): only a hit above 0.12 of the way up from the car's
/// origin (car-local direction) counts for the zones and the bumpers.
pub fn hit_counts(local: Vec3) -> bool {
    local.normalize_or_zero().y > f32::from_bits(0x3df5_c28f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(_: Vec3) -> Option<(f32, Vec3)> {
        Some((0.0, Vec3::Y))
    }

    #[test]
    fn perlin_is_the_improved_noise() {
        assert_eq!(perlin(0.0), 0.0);
        assert_eq!(perlin(3.0), 0.0);
        // (at 0.5: g0 = grad[perm[0]=151 & 15 = 7] = -1 * 0.5, g1 = grad[perm[1]=160 & 15 = 0] = 1 * -0.5, s = 0.5)
        assert!((perlin(0.5) - -0.5).abs() < 1e-6);
    }

    #[test]
    fn the_zone_gate_is_0_12() {
        assert!(hit_counts(Vec3::new(0.0, 0.5, -2.0)));
        assert!(!hit_counts(Vec3::new(0.0, 0.2, -2.0)));
        assert_eq!(f32::from_bits(0x3df5_c28f), 0.12);
    }

    /// A loose hood held shut by gravity on a standing car stays shut; the wind at speed lifts it, at most 0.2 rad.
    #[test]
    fn a_loose_hood_opens_at_most_a() {
        let mut p = DamageParts::default();
        let mut c = Mat4::IDENTITY;
        p.step(c, 0.0, false, DamageParts::FRAME, &flat);
        p.hood.loose = true;
        for _ in 0..120 {
            p.step(c, 0.0, false, DamageParts::FRAME, &flat);
        }
        assert!(p.hood.angle.abs() < 1e-3, "{}", p.hood.angle);
        let mut min: f32 = 0.0;
        for i in 0..2000 {
            c = Mat4::from_translation(Vec3::new(0.0, 0.0, -(i as f32) * 0.5));
            p.step(c, 30.0, false, DamageParts::FRAME, &flat);
            min = min.min(p.hood.angle);
        }
        assert!(min >= -0.2 - 1e-4, "{min}");
    }

    /// A particle swung sideways is put back into the car's YZ plane (0x8233F3FC), so the hood never opens past its
    /// stop: recorded 2026-10-19, an R8 rammed into a wall and driven on with the hood loose, 2,399 frames, deepest
    /// -0.20001 rad; without the rule the port's hood reached -1.02 rad on the same frames.
    #[test]
    fn a_sideways_swing_stays_in_the_hinge_plane() {
        let mut p = DamageParts::default();
        let c = Mat4::IDENTITY;
        p.step(c, 0.0, false, DamageParts::FRAME, &flat);
        p.hood.loose = true;
        // (the particle thrown 0.6 m sideways and a little up, as a car spinning off a wall does)
        p.hood.p = Vec3::new(0.6, 2.06, 0.8);
        p.hood.q = Vec3::new(0.3, 2.04, 0.9);
        p.step(c, 0.0, false, DamageParts::FRAME, &flat);
        assert_eq!(p.hood.p.x, 0.0);
        assert!(p.hood.angle >= -0.2 - 1e-5 && p.hood.angle <= 0.0, "{}", p.hood.angle);
        // (the particle is shorter than the rod by the sideways share: |(y, z) - P0| < 1)
        assert!(((p.hood.p - FLAP_P0).length() - 1.0).abs() > 1e-3, "{:?}", p.hood.p);
    }

    /// A first hit near one end loosens it (sags at most 5 cm); with heavy damage a second hit breaks it; the
    /// broken end hangs from the other and, once it is 0.85 m from where it belongs, the bumper falls off.
    #[test]
    fn bumper_ends_loosen_break_and_tear_off() {
        let mut p = DamageParts::default();
        p.rig(&PartRig { movement: [2, 2, 1, 1], bumper_rest: [Some(Mat4::from_translation(Vec3::new(0.0, 0.4, 2.2))), Some(Mat4::from_translation(Vec3::new(0.0, 0.4, -2.2)))] });
        let c = Mat4::IDENTITY;
        let dt = DamageParts::FRAME;
        p.step(c, 0.0, false, dt, &flat);
        p.queue(PartHit { point: Vec3::new(0.6, 0.4, -2.2), amount: 0.5, kind: 0 });
        p.step(c, 0.0, false, dt, &flat);
        let f = p.bumpers[1].as_ref().unwrap();
        assert_eq!((f.a.state, f.b.state), (1, 0));
        for _ in 0..60 {
            p.step(c, 0.0, false, dt, &flat);
        }
        let f = p.bumpers[1].as_ref().unwrap();
        assert!((f.a.local.y - 0.4 - -0.05).abs() < 1e-3, "{:?}", f.a.local);
        // a hit that is not a collision, or too weak, does nothing
        p.queue(PartHit { point: Vec3::new(0.6, 0.4, -2.2), amount: 0.5, kind: 1 });
        p.queue(PartHit { point: Vec3::new(0.6, 0.4, -2.2), amount: 0.1, kind: 0 });
        p.step(c, 0.0, true, dt, &flat);
        assert_eq!(p.bumpers[1].as_ref().unwrap().a.state, 1);
        p.queue(PartHit { point: Vec3::new(0.6, 0.4, -2.2), amount: 0.5, kind: 0 });
        p.step(c, 0.0, true, dt, &flat);
        assert_eq!(p.bumpers[1].as_ref().unwrap().a.state, 2);
        // (standing, the broken end only hangs 0.2 m: the tear test measures last frame's point against this
        // frame's rest point, so the other end goes once the car covers 0.85 m a frame - about 25 m/s)
        let mut off = false;
        let mut at = 0;
        for i in 0..60 {
            let c = Mat4::from_translation(Vec3::new(0.0, 0.0, -(i as f32) * 0.9));
            p.step(c, 27.0, true, dt, &flat);
            if !off && p.events.iter().any(|(_, e)| matches!(e, PartEvent::BumperOff { bumper: 1, .. })) {
                at = i;
            }
            off |= p.events.iter().any(|(_, e)| matches!(e, PartEvent::BumperOff { bumper: 1, .. }));
        }
        assert!(off && at <= 2, "{at} {:?}", p.bumpers[1].as_ref().unwrap().a);
        // the rear one is untouched, and a repair puts the front one back
        assert_eq!(p.bumpers[0].as_ref().unwrap().a.state, 0);
        p.repair(&c);
        assert!(!p.damaged());
    }

    /// NoMovement (AllowLoose = 1) never lets an end break.
    #[test]
    fn the_tune_caps_the_states() {
        let mut p = DamageParts::default();
        p.rig(&PartRig { movement: [1, 0, 1, 1], bumper_rest: [Some(Mat4::from_translation(Vec3::new(0.0, 0.4, 2.2))), Some(Mat4::from_translation(Vec3::new(0.0, 0.4, -2.2)))] });
        let c = Mat4::IDENTITY;
        for pt in [Vec3::new(0.6, 0.4, -2.2), Vec3::new(0.6, 0.4, 2.2)] {
            for _ in 0..3 {
                p.queue(PartHit { point: pt, amount: 0.5, kind: 0 });
                p.step(c, 0.0, true, DamageParts::FRAME, &flat);
            }
        }
        assert_eq!(p.bumpers[1].as_ref().unwrap().a.state, 1);
        assert_eq!(p.bumpers[0].as_ref().unwrap().a.state, 0);
    }
}

/// Replay of a recording of the original (work/partsrec: the recomp recorder's ANIM records, exported by
/// `export.py`): the recorded car matrix, speed, queued hits and the flaps' loose flags go in frame by frame, and the
/// hood / trunk angles, the bumpers' end states and points come out against the recorded ones.
/// `MCLA_PARTS_REPLAY=<frames.txt> cargo test --release -p mcla replay_recording -- --ignored --nocapture`
#[cfg(test)]
mod replay {
    use super::*;

    fn rows(v: &[f32]) -> Mat4 {
        Mat4::from_cols(
            Vec3::new(v[0], v[1], v[2]).extend(0.0),
            Vec3::new(v[4], v[5], v[6]).extend(0.0),
            Vec3::new(v[8], v[9], v[10]).extend(0.0),
            Vec3::new(v[12], v[13], v[14]).extend(1.0),
        )
    }

    #[test]
    #[ignore]
    fn replay_recording() {
        let Ok(path) = std::env::var("MCLA_PARTS_REPLAY") else { return };
        let text = std::fs::read_to_string(&path).unwrap();
        let mut p = DamageParts::default();
        let mut lines = text.lines();
        let r: Vec<f32> = lines.next().unwrap().split_whitespace().skip(1).map(|x| x.parse().unwrap()).collect();
        let mov = [r[32] as u32, r[33] as u32, r[34] as u32, r[35] as u32];
        p.rig(&PartRig { movement: mov, bumper_rest: [Some(rows(&r[0..16])), Some(rows(&r[16..32]))] });
        let flat = |_: Vec3| -> Option<(f32, Vec3)> { None };
        let (mut frames, mut state_miss) = (0, 0);
        let mut pose_err: f32 = 0.0;
        let (mut hood_err, mut trunk_err, mut end_err, mut clock_err): (f32, f32, f32, f32) = (0.0, 0.0, 0.0, 0.0);
        let (mut hood_min_rec, mut hood_min_port): (f32, f32) = (0.0, 0.0);
        let mut hood_at = 0;
        let mut out = String::new();
        while let (Some(fl), Some(el)) = (lines.next(), lines.next()) {
            let f: Vec<f32> = fl.split_whitespace().skip(1).map(|x| x.parse().unwrap_or(f32::NAN)).collect();
            let e: Vec<f32> = el.split_whitespace().skip(1).map(|x| x.parse().unwrap_or(f32::NAN)).collect();
            let (tick, speed, heavy) = (f[0] as u32, f[1], f[2] != 0.0);
            p.hood.loose = f[3] != 0.0;
            p.trunk.loose = f[4] != 0.0;
            let c = rows(&f[5..21]);
            let n = f[21] as usize;
            for i in 0..n {
                let h = &f[22 + 5 * i..27 + 5 * i];
                p.queue(PartHit { point: Vec3::new(h[0], h[1], h[2]), amount: h[3], kind: h[4] as u8 });
            }
            let before: Vec<(u8, u8)> = p.bumpers.iter().flatten().map(|b| (b.a.state, b.b.state)).collect();
            p.step(c, speed, heavy, DamageParts::FRAME, &flat);
            frames += 1;
            // the recorded: hood, trunk angle; per bumper (rear, front) states, detached, clock, A, B
            let (ha, ta) = (e[0], e[1]);
            if (p.hood.angle - ha).abs() > hood_err {
                hood_at = tick;
            }
            hood_err = hood_err.max((p.hood.angle - ha).abs());
            trunk_err = trunk_err.max((p.trunk.angle - ta).abs());
            hood_min_rec = hood_min_rec.min(ha);
            hood_min_port = hood_min_port.min(p.hood.angle);
            for (k, b) in p.bumpers.iter_mut().enumerate() {
                let Some(b) = b.as_mut() else { continue };
                let o = 8 + 10 * k;
                let (sa, sb, det, clock) = (e[o] as u8, e[o + 1] as u8, e[o + 2] != 0.0, e[o + 3]);
                let (ra, rb) = (Vec3::new(e[o + 4], e[o + 5], e[o + 6]), Vec3::new(e[o + 7], e[o + 8], e[o + 9]));
                if (b.a.state, b.b.state) != (sa, sb) || b.detached.is_some() != det {
                    state_miss += 1;
                    if state_miss < 40 {
                        out += &format!("tick {tick} bumper {k}: port {:?} det {} recorded ({sa}, {sb}) det {det} (before {:?}, hits {n}, heavy {heavy})\n", (b.a.state, b.b.state), b.detached.is_some(), before.get(k));
                    }
                    b.a.state = sa;
                    b.b.state = sb;
                }
                if !det && b.detached.is_none() {
                    let d = (b.a.pos - ra).length().max((b.b.pos - rb).length());
                    if d > end_err {
                        end_err = d;
                        out += &format!("tick {tick} bumper {k} ({sa}{sb}): end error {d:.5} port A {:.4?} B {:.4?} rec A {ra:.4?} B {rb:.4?}\n", b.a.pos, b.b.pos);
                    }
                    if sa != 0 || sb != 0 {
                        clock_err = clock_err.max((b.clock - clock).abs());
                        // the drawn delta A = M_rest^-1 * F_local (bumper +16, rows = axes)
                        let rec = rows(&e[60 + 16 * k..76 + 16 * k]);
                        let port = b.m_rest.inverse() * b.pose;
                        pose_err = pose_err.max((0..4).map(|c| (port.col(c).truncate() - rec.col(c).truncate()).length()).fold(0.0, f32::max));
                    }
                }
            }
            if tick % 600 == 0 || (ha - p.hood.angle).abs() > 1e-3 && frames % 15 == 0 {
                out += &format!("tick {tick}: hood rec {ha:.5} port {:.5}  trunk rec {ta:.5} port {:.5}  speed {speed:.2}\n", p.hood.angle, p.trunk.angle);
            }
        }
        println!("{out}");
        println!("{frames} frames: hood angle max error {hood_err:.6} at tick {hood_at} (deepest rec {hood_min_rec:.5} port {hood_min_port:.5}), trunk {trunk_err:.6}, bumper ends {end_err:.6} m, drawn pose {pose_err:.6}, clock {clock_err:.5}, state mismatches {state_miss}");
    }
}

/// The torn bumper against a recording of the original (`work/bumperbody/export_piece.py` writes the file): the
/// recorded car is carried through its ticks (an immovable body moving as recorded, its hull only), the piece
/// goes into the car's level at the frame its recorded ends give at the detach, with 2.4's velocity and spin,
/// and the loose bodies' tick runs as vehsim runs it. Prints the port's piece against the recorded one.
/// `MCLA_PIECE_REPLAY=<file>;<data dir>;<car>`, `cargo test --release -p mcla piece_against_recording -- --ignored --nocapture`.
#[cfg(test)]
mod piece_replay {
    use crate::physics::loose::{CarDyn, CarIn, LooseState, LooseWorld};
    use crate::physics::solver::{Body, Pushed};
    use glam::Vec3;

    fn v3(a: &[f32]) -> Vec3 {
        Vec3::new(a[0], a[1], a[2])
    }
    fn m4(a: &[f32]) -> [Vec3; 4] {
        [v3(&a[0..3]), v3(&a[3..6]), v3(&a[6..9]), v3(&a[9..12])]
    }

    #[test]
    #[ignore]
    fn piece_against_recording() {
        let Ok(spec) = std::env::var("MCLA_PIECE_REPLAY") else { return };
        let f: Vec<&str> = spec.split(';').collect();
        let (path, data_dir, car_name) = (f[0], f[1], f.get(2).copied().unwrap_or("vp_aud_r8_08"));
        let assets = crate::assets::Assets::new(data_dir);
        crate::physics::collision::load_surfaces(data_dir);
        let mut city = crate::physics::collision::WorldCollider::default();
        for (i, mesh) in crate::world::city_bound(&assets).expect("city bound").into_iter().enumerate() {
            city.sectors.insert(crate::world::BOUND_ID + i, mesh);
        }
        let world = LooseWorld::load(&assets.path("resources/prop/0x0e66973c.r131")).expect("prop file");
        let car = crate::vehicle_model::load_car(&assets, &std::path::Path::new(data_dir).join("resources/vehicle").join(car_name)).unwrap();
        let st = car.simtune.as_ref().unwrap();
        let tune = mcla_formats::rage::simtune::SimTune::lerp(&st.base, &st.mods, 0.0);
        let hubs = car.hubs.unwrap();
        let hull = crate::physics::hull::shaped(car.hull.as_ref().unwrap(), &tune, (hubs[0].y + hubs[2].y) * 0.5, car.bike);
        let text = std::fs::read_to_string(path).unwrap();
        let mut lines = text.lines();
        let d: Vec<f32> = lines.next().unwrap().split_whitespace().skip(1).map(|x| x.parse().unwrap()).collect();
        let det_tick = d[0] as u32;
        let (a, ap, b, bp) = (v3(&d[13..16]), v3(&d[16..19]), v3(&d[19..22]), v3(&d[22..25]));
        let c = m4(&d[25..37]);
        // (2.3 step 5 and 2.4: the frame of the two ends, the ends' mean velocity and their spin plus -3 rad/s about the car's X)
        let up = c[1];
        let mid = (a + b) * 0.5;
        let x = (mid - b).normalize();
        let z = x.cross(up).normalize();
        let y = z.cross(x).normalize();
        let (va, vb) = (a - ap, b - bp);
        let vel = (va + vb) * 0.5 * 30.0;
        let spin = (a - b).cross(va - vb) * -30.0 + c[0] * -3.0;
        let ticks: Vec<(u32, [Vec3; 4], Vec3, Vec3, Option<[Vec3; 4]>)> = lines
            .map(|l| {
                let t: Vec<f32> = l.split_whitespace().skip(1).map(|x| x.parse().unwrap_or(f32::NAN)).collect();
                (t[0] as u32, m4(&t[1..13]), v3(&t[13..16]), v3(&t[16..19]), t[19].is_finite().then(|| m4(&t[19..31])))
            })
            .collect();
        let mut loose = LooseState::default();
        let dt = 1.0f32 / 60.0;
        let mut id = None;
        let mut out = String::new();
        let (mut rest_tick, mut last_pos, mut moving_since) = (None::<u32>, Vec3::ZERO, 0u32);
        for k in 1..ticks.len() {
            let (tick, now, cv, cw, rec) = ticks[k];
            let last = ticks[k - 1].1;
            if tick == det_tick {
                id = loose.add_piece(&world, [x, y, z, mid], vel, spin);
            }
            if tick < det_tick {
                continue;
            }
            let car_in = CarIn { hull: &hull, hull_cg: hull.cg, hull_margin: f32::from_bits(0x3d23_d70a), now, last, speed: cv.length(), bound: (hull.min - Vec3::splat(0.041), hull.max + Vec3::splat(0.041)), wheels: Vec::new(), vel: cv, touch: (tune.s.bound_friction, tune.s.bound_elasticity), wiped: false };
            loose.pre_collide(&world, dt);
            loose.tick += 1;
            loose.collide(&world, &car_in, &city, false);
            loose.refresh(&world, &car_in);
            let ids = loose.car_island();
            let mut body = Body { at: now[3] + now[1] * 0.708, rows: [now[0], now[1], now[2]], vel: cv, ang: cw, inv_mass: 0.0, inv_moments: Vec3::ZERO, force: Vec3::ZERO, torque: Vec3::ZERO };
            if !ids.is_empty() {
                let (mut p, mut l) = (Vec3::ZERO, Vec3::ZERO);
                let mut dynm = CarDyn { p: &mut p, l: &mut l, mass: 0.0, inverse: Vec3::ZERO, rows: body.rows };
                let mut o = loose.solve_car_island(&world, &car_in, &mut body, &mut dynm, &[], &[], &[], &ids, dt);
                loose.push_car_island(&mut o, &body, Pushed::default(), dt);
            }
            loose.step_rest(&world, &car_in, dt);
            loose.end_tick(&world, dt);
            let Some((m, contacts, off)) = id.and_then(|i| loose.piece(i)) else { panic!("no piece") };
            if (m[3] - last_pos).length() > 1e-4 {
                moving_since = tick;
                rest_tick = None;
            } else if rest_tick.is_none() {
                rest_tick = Some(moving_since);
            }
            last_pos = m[3];
            let ground = city.ground(m[3], 1.0, 3.0).map_or(f32::NAN, |h| m[3].y - h.point.y);
            let face = if m[2].y.abs() > 0.9 { "1.8x0.333" } else if m[1].y.abs() > 0.9 { "1.8x0.5" } else if m[0].y.abs() > 0.9 { "0.333x0.5" } else { "edge" };
            let rec_s = rec.map_or("-".to_string(), |r| format!("{:.4} {:.4} {:.4} z.y {:.3} above {:.3} d {:.3}", r[3].x, r[3].y, r[3].z, r[2].y, city.ground(r[3], 1.0, 3.0).map_or(f32::NAN, |h| r[3].y - h.point.y), (r[3] - m[3]).length()));
            out += &format!("{tick} port {:.4} {:.4} {:.4} z.y {:.3} above {:.3} {face} contacts {} off {off} | rec {rec_s}\n", m[3].x, m[3].y, m[3].z, m[2].y, ground, contacts.len());
        }
        println!("{out}");
        println!("detach {det_tick}: port piece at rest from tick {:?} ({} ticks after the detach)", rest_tick, rest_tick.map_or(-1, |t| t as i64 - det_tick as i64));
    }
}
