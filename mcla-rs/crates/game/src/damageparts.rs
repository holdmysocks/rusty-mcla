//! The moving damage parts of a car as the original's car anim object has them (D = mcCar+84, per-frame
//! `F-24651`): the hood and the trunk lid ("flaps", D+1456 / D+1664) - a virtual hinged particle that falls
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
use glam::{Mat4, Quat, Vec3, Vec4};

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
    /// a bumper (0 rear, 1 front) tore off (F-82611)
    BumperOff { bumper: u8, pos: Vec3 },
    /// the fallen bumper struck something: the largest approach speed (m/s) of its new contacts this tick
    BumperImpact { bumper: u8, speed: f32, pos: Vec3 },
    /// the fallen bumper slides on something at `speed` m/s
    BumperScrape { bumper: u8, speed: f32, pos: Vec3 },
}

// ---------------------------------------------------------------- Perlin (F-52219)

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
    // (the gradient's dot with (f, 0, 0) is fma(f, gx, 0*gy + 0*gz) in the original: the zero's sign only decides the sign of a zero result;
    // it is -0 where gy and gz are both negative, gradients 11 and 15)
    let zero = |k: usize| if k == 11 || k == 15 { -0.0 } else { 0.0 };
    let (k0, k1) = ((PERM[i] & 15) as usize, (PERM[(i + 1) & 255] & 15) as usize);
    let g0 = crate::physics::exact::fma(f, GRAD_X[k0], zero(k0));
    let g1 = crate::physics::exact::fma(f - 1.0, GRAD_X[k1], zero(k1));
    // (F-52219: the scalar unit's fused operations: ((fma(fms(f, 6, 15), f, 10) f) f) f, then fma(g1 - g0, s, g0))
    use crate::physics::exact::{fma, fms};
    let s = ((fma(fms(f, 6.0, 15.0), f, 10.0) * f) * f) * f;
    fma(g1 - g0, s, g0)
}

/// fsel: `a >= 0` (a NaN is not) picks `b`, else `c`.
fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 { b } else { c }
}

/// The bumper's tune (`TUNERS_DAMAGE`, read live: docs/reference/audio/DAMAGE_PARTS_RULE.md): +468 the spring, +472 the
/// Verlet keep, +496 the attached ends' limits.
const BUMPER_SPRING: f32 = 1000.0;
const BUMPER_KEEP: f32 = 0.9;
const BUMPER_LIMIT: Vec3 = Vec3::new(0.01, 0.05, 0.15);
/// gravity's y in the bumper forces (the image's -9.8)
const GRAVITY_Y: f32 = -9.8;
/// F-57572's reach of a hit round an end (0.8, an image word) and the least amount that counts (0.15)
const HIT_NEAR: f32 = 0.8;
const HIT_MIN: f32 = 0.15;

/// A matrix's rows as the original keeps them (glam's columns), w included.
fn rows4(m: &Mat4) -> [Vec4; 4] {
    [m.x_axis, m.y_axis, m.z_axis, m.w_axis]
}

/// The vector unit's 4-term dot (vmsum4fp, dpps): (a.x b.x + a.y b.y) + (a.z b.z + a.w b.w).
fn dp4(a: Vec4, b: Vec4) -> f32 {
    (a.x * b.x + a.y * b.y) + (a.z * b.z + a.w * b.w)
}

/// A point (w = 1) through rows r: each coordinate the 4-term dot with r's column.
fn through(p: Vec3, r: &[Vec4; 4]) -> Vec3 {
    let q = p.extend(1.0);
    let col = |i: usize| Vec4::new(r[0][i], r[1][i], r[2][i], r[3][i]);
    Vec3::new(dp4(q, col(0)), dp4(q, col(1)), dp4(q, col(2)))
}

/// A vector made unit as the vector unit does it: vmsum3fp's (x^2 + y^2) + z^2, then `exact::rsqrt`'s estimate and
/// Newton step (a zero vector stays zero).
fn unit(v: Vec3) -> Vec3 {
    let n = (v.x * v.x + v.y * v.y) + v.z * v.z;
    if n == 0.0 {
        return Vec3::ZERO;
    }
    v * crate::physics::exact::rsqrt(n)
}

/// The vpermwi 135 / 99 cross: perm(a, zxy) perm(b, yzx) - perm(a, yzx) perm(b, zxy) = b x a, unfused.
fn cross_vmx(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.z * b.y - a.y * b.z, a.x * b.z - a.z * b.x, a.y * b.x - a.x * b.y)
}

/// A frame's inverse by F-30106 (rows; where it refuses, the rows as they were), w lanes kept.
fn inverse_rows(r: &[Vec4; 4]) -> [Vec4; 4] {
    match crate::physics::exact::inverse_frame(&[r[0].truncate(), r[1].truncate(), r[2].truncate()], r[3].truncate()) {
        Some(i) => [i[0].extend(r[0].w), i[1].extend(r[1].w), i[2].extend(r[2].w), i[3].extend(r[3].w)],
        None => *r,
    }
}

/// Rows a (x, y, z of each) times rows m in the original's row order: row k = ((m0 a.x [+ m3]) + m1 a.y) + m2 a.z.
fn row_product(a: &[Vec3; 4], m: &[Vec4; 4]) -> [Vec4; 4] {
    let row = |k: usize| {
        let v = a[k];
        let t = if k == 3 { m[0] * v.x + m[3] } else { m[0] * v.x };
        m[2] * v.z + (m[1] * v.y + t)
    };
    [row(0), row(1), row(2), row(3)]
}

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------- flaps (hood, trunk)

/// The hood or the trunk lid: a unit-length rod hinged at (0, 2, 0) in car space, closed along +z.
#[derive(Clone, Debug)]
pub struct Flap {
    /// flap+80: the open stop's angle (0.20000005 hood, 0.15 trunk)
    pub ang_open: f32,
    /// unit(Po - P0): where the open stop puts the arm (the refined unit of F-50514)
    pub open_dir: Vec3,
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
/// The open stop as the binding F-53023 and the set-up F-71271 leave it (flap +48 and +80): Po = P0 + the rest arm
/// turned by -0.2 (hood) / -0.15 (trunk) in F-43547, angOpen = atan2 of unit(Po - P0) - the words of the original's
/// run (tools/flap_step_reference.py; test `flap_step_matches_the_original` checks them).
const HOOD_OPEN: ([u32; 2], u32) = ([0x400c_b700, 0x3f7a_e5a5], 0x3e4c_ccd3);
const TRUNK_OPEN: ([u32; 2], u32) = ([0x4009_9065, 0x3f7d_201a], 0x3e19_999a);

impl Flap {
    /// `a` = 0.2 the hood, 0.15 the trunk lid.
    pub fn new(a: f32, movement: u32) -> Flap {
        let (po, open) = if a < 0.175 { TRUNK_OPEN } else { HOOD_OPEN };
        let po = Vec3::new(0.0, f32::from_bits(po[0]), f32::from_bits(po[1]));
        Flap {
            ang_open: f32::from_bits(open),
            open_dir: unit(po - FLAP_P0),
            loose: false,
            locked: movement == 0,
            reversed: movement == 2,
            p: Vec3::ZERO,
            q: Vec3::ZERO,
            angle: 0.0,
            prev_angle: 0.0,
        }
    }

    /// `F-97771`: back to rest (closed) at the car's matrix - the rest point through it by the 4-term dots (checked
    /// word for word, test `flap_reset_matches_the_original`).
    pub fn reset(&mut self, c: &Mat4) {
        self.p = through(FLAP_R, &rows4(c));
        self.q = self.p;
        self.angle = 0.0;
        self.prev_angle = 0.0;
    }

    /// F-16752 and F-50514 for one frame, in the original's operations; returns a stop event. Checked word for word
    /// (test `flap_step_matches_the_original`).
    fn step(&mut self, c: &Mat4, speed: f32, dt: f32, which: u8) -> Option<PartEvent> {
        // (F-16752) the hinge in the world; a flap that is not loose sits closed (its last angle kept)
        let r = rows4(c);
        let hinge_w = through(FLAP_P0, &r);
        if !self.loose {
            self.angle = 0.0;
            return None;
        }
        // Verlet in the world, gravity only: p + ((p - q) (dt x 30) + (g dt) dt), the vector unit's unfused lanes
        let p = self.p + ((self.p - self.q) * (dt * 30.0) + GRAVITY * dt * dt);
        self.q = self.p;
        self.p = p;
        // (F-50514) to car space by the inverse F-30106, the wind lift (F-66314) added fused: y = fma(lift dt, 30, y)
        let mut l = through(p, &inverse_rows(&r));
        let o = c.w_axis;
        let lift = clamp01(0.2 * speed) * clamp01(perlin(5.0 * ((o.z + o.y) + o.x))) * 0.05 * clamp01(1.0 + self.angle / 0.05);
        l.y = crate::physics::exact::fma(lift * dt, 30.0, l.y);
        let mut d = unit(l - FLAP_P0);
        let t = crate::physics::exact::guest_atan2(d.y as f64, d.z as f64) as f32;
        // the stops (closed: angClosed +84 = 0, the arm to unit(Pc - P0); open: +80, unit(Po - P0)); the sound's rate
        // is |last angle - angle| times the frame block's 1 / dt
        let rate = (self.prev_angle - self.angle).abs() * (1.0 / dt);
        let mut ev = None;
        if t < 0.0 {
            d = unit(FLAP_R - FLAP_P0);
            ev = Some(PartEvent::FlapStop { flap: which, open: false, rate, pos: p });
        } else if t > self.ang_open {
            d = self.open_dir;
            ev = Some(PartEvent::FlapStop { flap: which, open: true, rate, pos: p });
        }
        // L = d len + P0 (len +176 = 1), its x then the rest point's (flap+32, 0): back into the car's YZ plane
        // (A-15101 `stfs f0,80(r1)`; recorded 2026-10-19)
        let mut l = d * 1.0 + FLAP_P0;
        l.x = FLAP_R.x;
        self.p = through(l, &r);
        let rest_w = through(FLAP_R, &r);
        self.prev_angle = self.angle;
        // the angle: -acos of the fused dot of the two refined units, kept in [-1, 1] by two fsel
        let (u, v) = (unit(self.p - hinge_w), unit(rest_w - hinge_w));
        let dot = crate::physics::exact::fma(u.x, v.x, crate::physics::exact::fma(u.y, v.y, u.z * v.z));
        let dot = if dot - -1.0 >= 0.0 { dot } else { -1.0 };
        let dot = if dot - 1.0 >= 0.0 { 1.0 } else { dot };
        self.angle = -(crate::physics::exact::guest_acos(dot as f64) as f32);
        // (the one-shot plays at flap+112 as it is at the check - the integrated particle, P+20)
        ev
    }

    fn sign(&self) -> f32 {
        if self.reversed { -1.0 } else { 1.0 }
    }

    /// `F-76270`: the drawn angle (signed)
    pub fn drawn(&self) -> f32 {
        self.sign() * if self.locked { 0.0 } else { self.angle }
    }

    /// `F-15658`: last frame's angle (not zeroed when locked)
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
    /// the drawn delta of a hanging bumper, the original's +16 (F-44446): F M^-1 M_rest^-1 in its row order
    pub delta: Mat4,
}

impl Bumper {
    pub fn new(front: bool, m_rest: Mat4, movement: u32) -> Bumper {
        let mut b = Bumper { front, m_rest, a: End::default(), b: End::default(), clock: 0.0, max_state: movement.min(2) as u8, detached: None, pose: Mat4::IDENTITY, delta: Mat4::IDENTITY };
        // `F-92886`: the caller's point (0.5, 0, 0) and its x negated through the rest matrix (the 4-term dots); the
        // movement byte +484 is kept raw there and capped where it is read (`max_state`). Checked word for word
        // (test `bumper_new_matches_the_original`); its closing reset F-40944 is the caller's `reset` here.
        let r = rows4(&m_rest);
        b.a.rest = through(Vec3::new(0.5, 0.0, 0.0), &r);
        b.b.rest = through(Vec3::new(-0.5, 0.0, 0.0), &r);
        b
    }

    /// F-40944: both ends attached - each place and last place the rest point through the car's matrix (the 4-term
    /// dot), car space the rest point, the forces 0 -, the states 0, the free piece removed (F-27119), the clock 0.
    /// (It also writes the rest frame +272 (F-27388), which nothing the port draws reads.)
    pub fn reset(&mut self, c: &Mat4) {
        let r = rows4(c);
        for e in [&mut self.a, &mut self.b] {
            e.state = 0;
            e.pos = through(e.rest, &r);
            e.prev = e.pos;
            e.local = e.rest;
            e.local_prev = e.rest;
        }
        self.detached = None;
        self.clock = 0.0;
        self.pose = Mat4::IDENTITY;
    }

    /// F-57572: a queued hit changes the ends' states. An end counts when the hit is within 0.8 m of it and
    /// its amount above 0.15 (bge / ble on the fcmpu: a NaN distance or amount does not count).
    fn hit(&mut self, c: &Mat4, h: &PartHit, heavy: bool) {
        if h.kind != 0 {
            return;
        }
        // (an end's place = the rest point through C (4-term dots) + the body's velocity x dt x K, fused; K is an
        // image word that is 0 and that nothing writes. The bumper's byte +486 - with heavy damage, B broken - is set
        // by nothing: the constructor clears it.)
        let r = rows4(c);
        let (pa, pb) = (through(self.a.rest, &r), through(self.b.rest, &r));
        // (|d| as fsqrts(dx dx + (dz dz + dy dy)), fused)
        let dist = |p: Vec3| {
            let d = h.point - p;
            d.x.mul_add(d.x, d.z.mul_add(d.z, d.y * d.y)).sqrt()
        };
        let counts = |p: Vec3| dist(p) < HIT_NEAR && h.amount > HIT_MIN;
        if counts(pa) {
            if self.a.state == 0 && self.b.state == 0 {
                self.a.state = 1;
            } else if self.a.state == 1 && heavy {
                self.a.state = 2;
            }
            if self.b.state == 2 {
                self.a.state = 2;
            }
        }
        if counts(pb) {
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

    /// `F-44446` for one frame.
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
        // (F-44446, every step in the original's operations: the vector unit's products and sums unfused, its 4-term
        // dot as dpps sums it, the scalar unit's multiply-adds fused; `r` the car matrix's rows, `inv` their inverse
        // by F-30106)
        let r = rows4(c);
        let inv = inverse_rows(&r);
        // 1 forces (F-13331): k = speed / 50 kept in [0, 1] by two fsel, the clock advanced by k; a loose end's force
        // is gravity plus 15 sin(2 clock) k and 5 perlin(clock) k upwards (fused, the guest's double sine), with its
        // spring to the rest point x 0.1; a broken end gravity alone; an attached one the spring alone
        let k = fsel(speed / 50.0, speed / 50.0, 0.0);
        let k = fsel(k - 1.0, 1.0, k);
        self.clock += k;
        let mut force = [Vec3::ZERO; 2];
        for (i, e) in [&self.a, &self.b].into_iter().enumerate() {
            let at = through(e.rest, &r);
            let (mut f, s) = match e.state {
                0 => (Vec3::ZERO, 1.0),
                1 => (Vec3::new(0.0, GRAVITY_Y, 0.0), 0.1),
                _ => (Vec3::new(0.0, GRAVITY_Y, 0.0), 0.0),
            };
            if e.state == 1 {
                let sn = crate::physics::exact::guest_sin(f64::from(self.clock * 2.0)) as f32;
                f.y = crate::physics::exact::fma(sn * 15.0, k, f.y);
                f.y = crate::physics::exact::fma(perlin(self.clock * 1.0) * 5.0, k, f.y);
            }
            force[i] = ((at - e.pos) * BUMPER_SPRING) * s + f;
        }
        // 2 Verlet (F-73128): the keep 0.9 x dt x 30 (dt the game frame): p += (p - prev) keep + (f dt) dt
        let keep = (BUMPER_KEEP * dt) * 30.0;
        for (e, f) in [&mut self.a, &mut self.b].into_iter().zip(force) {
            let v = (e.pos - e.prev) * keep + (f * dt) * dt;
            e.prev = e.pos;
            e.pos = e.pos + v;
        }
        // 3 the limits in car space (F-56713): an attached end is put back on its rest point; each end's offset from
        // its rest point, through the car's inverse, is held in [-x, x], [-y, 0] and the front's [-z, 0] / the rear's
        // [0, z] by fsel pairs - the tune's (0.01, 0.05, 0.15), a broken end's (that x, 0.2, 10.0) - and goes back to
        // the world through the car's matrix; a broken end is then held at the rest length from the other
        let front = self.front;
        if self.a.state == 0 {
            self.a.pos = through(self.a.rest, &r);
        }
        if self.b.state == 0 {
            self.b.pos = through(self.b.rest, &r);
        }
        let da = through(self.a.pos, &inv) - self.a.rest;
        let db = through(self.b.pos, &inv) - self.b.rest;
        for (e, d) in [(&mut self.a, da), (&mut self.b, db)] {
            let l = if e.state == 2 { Vec3::new(BUMPER_LIMIT.x, 0.2, 10.0) } else { BUMPER_LIMIT };
            let x = fsel(d.x - -l.x, d.x, -l.x);
            let x = fsel(x - l.x, l.x, x);
            let y = fsel(d.y - -l.y, d.y, -l.y);
            let y = fsel(y, 0.0, y);
            let z = if front {
                let z = fsel(d.z - -l.z, d.z, -l.z);
                fsel(z, 0.0, z)
            } else {
                let z = fsel(d.z, d.z, 0.0);
                fsel(z - l.z, l.z, z)
            };
            let local = Vec3::new(x, y, z) + e.rest;
            e.pos = through(local, &r);
            e.local_prev = e.local;
            e.local = local;
        }
        let rd = self.a.rest - self.b.rest;
        let len = crate::physics::exact::fma(rd.x, rd.x, crate::physics::exact::fma(rd.z, rd.z, rd.y * rd.y)).sqrt();
        if self.a.state == 2 {
            self.a.pos = unit(self.a.pos - self.b.pos) * len + self.b.pos;
        } else if self.b.state == 2 {
            self.b.pos = unit(self.b.pos - self.a.pos) * len + self.a.pos;
        }
        // 4 tearing (F-49920): a broken end whose last place is more than 0.85 from its rest point breaks the
        // other; the second end is looked at only when the first is not broken
        let gone = |p: Vec3, rest: Vec3| {
            let d = p - through(rest, &r);
            crate::physics::exact::fma(d.x, d.x, crate::physics::exact::fma(d.z, d.z, d.y * d.y)).sqrt() > 0.85
        };
        if self.a.state == 2 {
            if gone(self.a.prev, self.a.rest) {
                self.b.state = 2;
            }
        } else if self.b.state == 2 && gone(self.b.prev, self.b.rest) {
            self.a.state = 2;
        }
        // 5 the bumper's frame (world): x from B to the middle of the ends, z = x x up, y = z x x, each made unit
        let up = r[1].truncate();
        let mid = (self.a.pos + self.b.pos) * 0.5;
        let x = unit(mid - self.b.pos);
        let z = unit(cross_vmx(up, x));
        let y = unit(cross_vmx(x, z));
        let f = Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), mid.extend(1.0));
        // the drawn delta (the original's +16): F M^-1 M_rest^-1, the products as the vector unit sums them
        let rest_inv = inverse_rows(&rows4(&self.m_rest));
        let p = row_product(&[x, y, z, mid], &inv);
        let q = row_product(&p.map(|v| v.truncate()), &rest_inv);
        self.delta = Mat4::from_cols(q[0], q[1], q[2], q[3]);
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

    /// The pose of a piece that came off (its body's frame in car space, `pose`): M_rest^-1 pose, as the drawn delta.
    fn detached_delta(&self) -> Mat4 {
        self.m_rest.inverse() * self.pose
    }

    /// The bumper's place in the drawn car: the rest of the car space maps the slot's mesh by
    /// M_slot' = M_slot * A with A = M_rest^-1 * F_local, its axes scaled by 0.9995 (0x827E6344).
    pub fn draw_delta(&self) -> Mat4 {
        self.draw_delta_on(&self.m_rest)
    }

    /// The same about the bone M_b the slot's part hangs on (its matrix index, F-14987): M_b' = M_b * A', drawn
    /// on the car-space mesh as M_b A' M_b^-1.
    pub fn draw_delta_on(&self, mb: &Mat4) -> Mat4 {
        let a = if self.detached.is_some() { self.detached_delta() } else if self.a.state == 0 && self.b.state == 0 { Mat4::IDENTITY } else { self.delta };
        slot_move(mb, &a, BUMPER_SCALE) * crate::physics::exact_inv4::inverse_of(mb)
    }

    /// The slot's part in the drawn car (car matrix `m`), hung on its bone `mb`; a fallen piece (its body's matrix
    /// `fallen`) has A = M_rest^-1 m^-1 B, so that it is drawn where its body is.
    pub fn slot_draw(&self, m: &Mat4, mb: &Mat4, fallen: Option<Mat4>) -> Mat4 {
        match fallen {
            Some(b) => *m * slot_move(mb, &(self.m_rest.inverse() * m.inverse() * b), BUMPER_SCALE) * crate::physics::exact_inv4::inverse_of(mb),
            None => *m * self.draw_delta_on(mb),
        }
    }

    /// F-87243, a teleport's carry: each end's place through the car's old matrix's inverse (F-30106), then
    /// through the new one, the 4-term dots each, its last place set to the same (the end keeps no speed). Checked word for
    /// word (test `bumper_carry_matches_the_original`).
    pub fn carry(&mut self, old: &Mat4, new: &Mat4) {
        let (i, n) = (inverse_rows(&rows4(old)), rows4(new));
        for e in [&mut self.a, &mut self.b] {
            e.pos = through(through(e.pos, &i), &n);
            e.prev = e.pos;
        }
    }

    /// `F-57399`: the fastest moving end relative to the car (m/s) and where it is; None when both are gone.
    /// `dt` is the anim frame's; the original multiplies by the frame block's 1 / dt (F-75502's 1.0 / dt).
    pub fn creak(&self, dt: f32) -> Option<(f32, Vec3)> {
        if (self.a.state == 2 && self.b.state == 2) || self.detached.is_some() {
            return None;
        }
        let end = |e: &End| (e.state, e.local, e.local_prev, e.pos);
        let (v, at) = creak_of(end(&self.a), end(&self.b), 1.0 / dt);
        Some((v, at.unwrap_or(self.a.pos)))
    }
}

/// F-57399's body on the two ends (state, car-space place, its last, world place): -1 and no place when neither
/// end is loose; end A's speed when it is (set, not compared); end B's unless it is below that (a NaN takes B). Speed =
/// sqrt(x x + (z z + y y)) of the car-space move, fused adds, times `inv_dt`. Checked word for word (test
/// `creak_matches_the_original`).
pub fn creak_of(a: (u8, Vec3, Vec3, Vec3), b: (u8, Vec3, Vec3, Vec3), inv_dt: f32) -> (f32, Option<Vec3>) {
    use crate::physics::exact::fma;
    let speed = |now: Vec3, last: Vec3| {
        let d = now - last;
        fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)).sqrt() * inv_dt
    };
    let mut out = (-1.0f32, None);
    if a.0 != 0 {
        out = (speed(a.1, a.2), Some(a.3));
    }
    if b.0 != 0 {
        let v = speed(b.1, b.2);
        if !(v < out.0) {
            out = (v, Some(b.3));
        }
    }
    out
}

fn scale_axes(m: Mat4, s: f32) -> Mat4 {
    Mat4::from_cols(m.x_axis * s, m.y_axis * s, m.z_axis * s, m.w_axis)
}

/// The slot pass's scales of the damage matrix's axes: the bumpers and the intercooler (0x827E6344), the hoods (0x827E6348).
pub const BUMPER_SCALE: f32 = 0.9995;
pub const HOOD_SCALE: f32 = 1.0005;

/// The slot pass's move of the bone a moved slot's part hangs on (F-14987): M_b' = A' . M_b in the original's row
/// order, A' the damage matrix with its axes scaled by `s` - the column form M_b * A', summed as the game does
/// (vmulfp / vmaddfp, not fused): axis i = r0 a'i.x + r1 a'i.y + r2 a'i.z, the place (r0 a3.x + r3) + r1 a3.y + r2 a3.z.
pub fn slot_move(mb: &Mat4, a: &Mat4, s: f32) -> Mat4 {
    let a = scale_axes(*a, s);
    let axis = |c: Vec4| mb.x_axis * c.x + mb.y_axis * c.y + mb.z_axis * c.z;
    let w = a.w_axis;
    Mat4::from_cols(axis(a.x_axis), axis(a.y_axis), axis(a.z_axis), mb.x_axis * w.x + mb.w_axis + mb.y_axis * w.y + mb.z_axis * w.z)
}

/// The fallen-off piece: a box (half extents 0.9 x 0.1665 x 0.25) under gravity, its eight corners against the
/// ground under them, an impulse and friction at each touching corner (port's own; the original's phInst is open).
fn step_loose(b: &mut Loose, dt: f32, which: u8, ground: &dyn Fn(Vec3) -> Option<(f32, Vec3)>, events: &mut Vec<PartEvent>) {
    if b.still > 0.5 {
        return;
    }
    const HALF: Vec3 = Vec3::new(0.9, 0.1665, 0.25);
    const MASS: f32 = 4.0;
    // the inertia of a box 2.25 x 0.416 x 0.625 (x 1.25 of the bound, F-30573)
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
        // (F-80340: an approach - dot(n, v) at least 0.1 of the speed - on a contact that was not there
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
    /// `F-53023`: the per-car settings.
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

    /// `F-75221`: the repair - flaps shut and not loose, bumpers on.
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

    /// The pieces this car's bumpers hold in a level now (the bumper's handle +532).
    pub fn held(&self) -> impl Iterator<Item = u64> + '_ {
        self.bumpers.iter().flatten().filter_map(|b| b.detached.and_then(|d| d.level))
    }

    /// The pieces this car put into a level that its bumpers no longer have (repair, reset): to be taken out of
    /// whichever car's level holds them now (F-40944).
    pub fn dropped(&mut self) -> Vec<u64> {
        let live: Vec<u64> = self.bumpers.iter().flatten().filter_map(|b| b.detached.and_then(|d| d.level)).collect();
        let out = self.level_ids.iter().copied().filter(|id| !live.contains(id)).collect();
        self.level_ids.retain(|id| live.contains(id));
        out
    }

    /// Each piece's frame and contacts read back from the level that holds it (`find`, by id), every tick: the drawn
    /// pose and the impact and scrape sounds (F-80340, 2.5).
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
            // (F-80340: of the piece's enabled contacts this tick, an approach at least 0.1 of the speed is an
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

    /// `F-24651` for one car and one frame. `heavy` = damage / MaxDamage > 0.66; `ground(p)` = the ground's
    /// height and normal under a point (for the fallen-off bumpers).
    pub fn step(&mut self, c: Mat4, speed: f32, heavy: bool, dt: f32, ground: &dyn Fn(Vec3) -> Option<(f32, Vec3)>) {
        if !self.placed {
            self.placed = true;
            self.repair(&c);
            self.last_c = c;
        }
        // (F-18198, the teleport: the flaps re-rested, the bumpers' points carried with the car - here a
        // jump of more than 10 m in a frame)
        if (c.w_axis - self.last_c.w_axis).truncate().length() > 10.0 {
            for f in [&mut self.hood, &mut self.trunk] {
                f.reset(&c);
            }
            for b in self.bumpers.iter_mut().flatten() {
                b.carry(&self.last_c, &c);
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
        // trunk, hood, rear bumper, front bumper (F-98654)
        if let Some(e) = self.trunk.step(&c, speed, dt, 1) {
            evs.push(e);
        }
        if let Some(e) = self.hood.step(&c, speed, dt, 0) {
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
        slot_move(slot, &a, HOOD_SCALE) * slot.inverse()
    }

    /// The trunk bone turned in place by RotX(s*ang) (F-52786), as a delta on the car-space drawing.
    pub fn trunk_delta(&self, bone: &Mat4) -> Mat4 {
        *bone * Mat4::from_rotation_x(self.trunk.drawn()) * bone.inverse()
    }
}

/// The gate of the zone damage (`F-74840` @A-82665): only a hit above 0.12 of the way up from the car's
/// origin (car-local direction) counts for the zones and the bumpers.
pub fn hit_counts(local: Vec3) -> bool {
    local.normalize_or_zero().y > f32::from_bits(0x3df5_c28f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hanging bumper's step against F-44446 run offline (tools/bumper_step_reference.py, testdata/bumper_step_ref.txt:
    /// 3,000 random cars, rest frames, end places, clocks, speeds, states and tune maxima, front and rear; the free
    /// piece's factory answering none): the ends' places, last places and car-space places, the clock, the states and
    /// the drawn delta (+16) - every word equal.
    #[test]
    fn bumper_step_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let m4 = |w: &[f32]| Mat4::from_cols(Vec4::from_slice(&w[0..4]), Vec4::from_slice(&w[4..8]), Vec4::from_slice(&w[8..12]), Vec4::from_slice(&w[12..16]));
        let v3 = |w: &[f32], k: usize| Vec3::new(w[k], w[k + 1], w[k + 2]);
        let mut n = 0;
        for line in include_str!("testdata/bumper_step_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let iv: Vec<&str> = i.split_whitespace().collect();
            let w: Vec<f32> = iv[..74].iter().map(|x| f(x)).collect();
            let ints: Vec<u8> = iv[74..].iter().map(|x| x.parse().unwrap()).collect();
            let ov: Vec<u32> = o.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let c = m4(&w[0..16]);
            let mut b = Bumper::new(ints[0] != 0, m4(&w[16..32]), u32::from(ints[3]));
            b.a.rest = v3(&w, 32);
            b.b.rest = v3(&w, 36);
            for (e, k) in [(&mut b.a, 40), (&mut b.b, 56)] {
                (e.pos, e.prev, e.local, e.local_prev) = (v3(&w, k), v3(&w, k + 4), v3(&w, k + 8), v3(&w, k + 12));
            }
            (b.a.state, b.b.state, b.clock) = (ints[1], ints[2], w[72]);
            let mut evs = Vec::new();
            b.step(&c, &c.inverse(), w[73], DamageParts::FRAME, 0, &|_| None, &mut evs);
            let bits = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            let want3 = |k: usize| [ov[k], ov[k + 1], ov[k + 2]];
            for (e, k, name) in [(&b.a, 16, "A"), (&b.b, 36, "B")] {
                for (got, at, what) in [(e.pos, k, "pos"), (e.prev, k + 4, "prev"), (e.local, k + 8, "local"), (e.local_prev, k + 12, "local prev")] {
                    assert_eq!(bits(got), want3(at), "{name} {what}: {line}");
                }
            }
            assert_eq!((b.clock.to_bits(), u32::from(b.a.state), u32::from(b.b.state)), (ov[72], ov[73], ov[74]), "clock / states: {line}");
            let delta = if b.a.state == 0 && b.b.state == 0 { Mat4::IDENTITY } else { b.delta };
            let got: Vec<u32> = delta.to_cols_array().iter().map(|x| x.to_bits()).collect();
            assert_eq!(got, ov[0..16].to_vec(), "delta: {line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The hood's / trunk lid's frame step against F-16752 with F-50514 run offline (tools/flap_step_reference.py,
    /// testdata/flap_step_ref.txt: 3,000 random cars, flaps built by the binding's set-up F-71271 and reset F-97771,
    /// loose or not, particles, angles and speeds; the stop sounds logged): the particle, its last place, the angle,
    /// the last angle, which stop sounded and its rate - every word equal.
    #[test]
    fn flap_step_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let m4 = |w: &[f32]| Mat4::from_cols(Vec4::from_slice(&w[0..4]), Vec4::from_slice(&w[4..8]), Vec4::from_slice(&w[8..12]), Vec4::from_slice(&w[12..16]));
        let v3 = |w: &[f32], k: usize| Vec3::new(w[k], w[k + 1], w[k + 2]);
        let (mut n, mut bad) = (0, [0usize; 5]);
        let mut first: Vec<String> = Vec::new();
        for line in include_str!("testdata/flap_step_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let iv: Vec<&str> = i.split_whitespace().collect();
            let w: Vec<f32> = iv[..27].iter().map(|x| f(x)).collect();
            let ints: Vec<u8> = iv[27..].iter().map(|x| x.parse().unwrap()).collect();
            let ov: Vec<u32> = o.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let c = m4(&w[0..16]);
            let trunk = ints[0] != 0;
            // the set-up's open stop (flap +52 / +56 and +80) as the port keeps it
            let (po, open) = if trunk { TRUNK_OPEN } else { HOOD_OPEN };
            assert_eq!((ov[13], ov[14], ov[20]), (po[0], po[1], open), "open stop: {line}");
            // and the rest of the set-up as the port's constants: P0 +0, R +32, Pc +64, angClosed +84, len +176
            let b3 = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            assert_eq!([&ov[0..3], &ov[8..11], &ov[16..19]], [&b3(FLAP_P0)[..], &b3(FLAP_R)[..], &b3(FLAP_R)[..]], "set-up points: {line}");
            assert_eq!((ov[21], ov[44]), (0, 1.0f32.to_bits()), "angClosed / len: {line}");
            let mut fl = Flap::new(if trunk { 0.15 } else { 0.2 }, 1);
            fl.loose = ints[1] != 0;
            (fl.p, fl.q, fl.angle, fl.prev_angle) = (v3(&w, 16), v3(&w, 20), w[24], w[25]);
            let ev = fl.step(&c, w[26], DamageParts::FRAME, u8::from(trunk));
            let bits = |v: Vec3| vec![v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            let (kind, rate) = match ev {
                Some(PartEvent::FlapStop { open, rate, .. }) => (if open { 2 } else { 1 }, rate.to_bits()),
                _ => (0, 0),
            };
            let checks = [
                bits(fl.p) == ov[28..31].to_vec(),
                bits(fl.q) == ov[32..35].to_vec(),
                fl.angle.to_bits() == ov[45],
                fl.prev_angle.to_bits() == ov[46],
                (kind, rate) == (ov[48], ov[49]),
            ];
            for (k, ok) in checks.iter().enumerate() {
                if !ok {
                    bad[k] += 1;
                    if first.len() < 6 {
                        first.push(format!("check {k}: got p {:?} q {:?} a {:08x} prev {:08x} ev {kind} {rate:08x}: {line}", bits(fl.p), bits(fl.q), fl.angle.to_bits(), fl.prev_angle.to_bits()));
                    }
                }
            }
            n += 1;
        }
        assert_eq!(n, 3000);
        assert_eq!(bad, [0; 5], "mismatches (p, q, angle, prev, event): {bad:?}
{}", first.join("
"));
    }

    /// A teleport's carry F-87243 run offline (tools/bumper_carry_reference.py, testdata/bumper_carry_ref.txt: 3,000
    /// random old and new car matrices and end places): both ends' places and last places - every word equal.
    #[test]
    fn bumper_carry_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let m4 = |w: &[f32]| Mat4::from_cols(Vec4::from_slice(&w[0..4]), Vec4::from_slice(&w[4..8]), Vec4::from_slice(&w[8..12]), Vec4::from_slice(&w[12..16]));
        let v3 = |w: &[f32], k: usize| Vec3::new(w[k], w[k + 1], w[k + 2]);
        let mut n = 0;
        for line in include_str!("testdata/bumper_carry_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<f32> = i.split_whitespace().map(|x| f(x)).collect();
            let ov: Vec<u32> = o.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let mut b = Bumper::new(true, Mat4::IDENTITY, 1);
            (b.a.pos, b.a.prev, b.b.pos, b.b.prev) = (v3(&w, 32), v3(&w, 36), v3(&w, 40), v3(&w, 44));
            b.carry(&m4(&w[0..16]), &m4(&w[16..32]));
            let bits = |v: Vec3| vec![v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            for (got, k, what) in [(b.a.pos, 0, "A"), (b.a.prev, 4, "A prev"), (b.b.pos, 20, "B"), (b.b.prev, 24, "B prev")] {
                assert_eq!(bits(got), ov[k..k + 3].to_vec(), "{what}: {line}");
            }
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The bumper's set-up F-92886 run offline (tools/bumper_new_reference.py, testdata/bumper_new_ref.txt: 3,000 rest
    /// matrices, movements 0 - 3): both rest points, the rest frame copy and the movement byte - every word equal.
    #[test]
    fn bumper_new_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for line in include_str!("testdata/bumper_new_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let t: Vec<&str> = i.split_whitespace().collect();
            let w: Vec<f32> = t[0..16].iter().map(|x| f(x)).collect();
            let mv: u32 = t[16].parse().unwrap();
            let ov: Vec<&str> = o.split_whitespace().collect();
            let m = Mat4::from_cols(Vec4::from_slice(&w[0..4]), Vec4::from_slice(&w[4..8]), Vec4::from_slice(&w[8..12]), Vec4::from_slice(&w[12..16]));
            let b = Bumper::new(true, m, mv);
            let words = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits(), 1f32.to_bits()];
            let mut got: Vec<u32> = words(b.a.rest).into_iter().chain(words(b.b.rest)).collect();
            got.extend(m.to_cols_array().iter().map(|x| x.to_bits()));
            let want: Vec<u32> = ov[0..24].iter().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            assert_eq!(got, want, "{line}");
            assert_eq!((u32::from_str_radix(ov[24], 16).unwrap(), ov[25]), (mv, "1"), "{line}");
            assert_eq!(b.max_state, mv.min(2) as u8);
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The creak's speed getter F-57399 run offline (tools/creak_reference.py, testdata/creak_ref.txt: 3,000 random
    /// end states, places and moves, ties among them): the speed and the place written (or none) - every word equal.
    /// A bumper with no part (+0 null) gives -1 and no place; the port has no bumper then.
    #[test]
    fn creak_matches_the_original() {
        let inv_dt = 1.0 / DamageParts::FRAME;
        assert_eq!(inv_dt.to_bits(), 0x41ef_ffff);
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let v3 = |w: &[f32], k: usize| Vec3::new(w[k], w[k + 1], w[k + 2]);
        let mut n = 0;
        for line in include_str!("testdata/creak_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let t: Vec<&str> = i.split_whitespace().collect();
            let ints: Vec<u8> = t[0..3].iter().map(|x| x.parse().unwrap()).collect();
            let w: Vec<f32> = t[3..].iter().map(|x| f(x)).collect();
            let ov: Vec<u32> = o.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            n += 1;
            if ints[0] == 0 {
                assert_eq!(ov, vec![0xbf80_0000, 0xdead_beef, 0xdead_beef, 0xdead_beef, 0xdead_beef], "{line}");
                continue;
            }
            let (v, at) = creak_of((ints[1], v3(&w, 0), v3(&w, 3), v3(&w, 6)), (ints[2], v3(&w, 10), v3(&w, 13), v3(&w, 16)), inv_dt);
            let both_gone = ints[1] == 2 && ints[2] == 2;
            let got: Vec<u32> = if both_gone {
                vec![0xbf80_0000, 0xdead_beef, 0xdead_beef, 0xdead_beef, 0xdead_beef]
            } else {
                let p = at.map_or([0xdead_beef; 4], |p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), 0x3f80_0000]);
                vec![v.to_bits(), p[0], p[1], p[2], p[3]]
            };
            assert_eq!(got, ov, "{line}");
        }
        assert_eq!(n, 3000);
    }

    /// The flap's reset F-97771 as the binding runs it (tools/flap_step_reference.py, the reset's words logged before
    /// the random state): the particle and its last place (+112 / +128) and both angles 0 - every word equal.
    #[test]
    fn flap_reset_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let m4 = |w: &[f32]| Mat4::from_cols(Vec4::from_slice(&w[0..4]), Vec4::from_slice(&w[4..8]), Vec4::from_slice(&w[8..12]), Vec4::from_slice(&w[12..16]));
        let mut n = 0;
        for line in include_str!("testdata/flap_step_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<f32> = i.split_whitespace().take(16).map(|x| f(x)).collect();
            let ov: Vec<u32> = o.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let mut fl = Flap::new(0.2, 1);
            fl.angle = 0.3;
            fl.prev_angle = 0.3;
            fl.reset(&m4(&w));
            let bits = |v: Vec3| vec![v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            assert_eq!((bits(fl.p), bits(fl.q)), (ov[50..53].to_vec(), ov[54..57].to_vec()), "particle: {line}");
            assert_eq!((fl.angle.to_bits(), fl.prev_angle.to_bits()), (ov[67], ov[68]), "angles: {line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The bumper's reset against F-40944 run offline (tools/bumper_reset_hit_reference.py, testdata/bumper_reset_ref.txt:
    /// 3,000 random cars, rest points (w 1, 0 or other), clocks, states and piece ids): the ends' places, last places,
    /// car-space places (xyz) and forces (0), the clock, the states, the piece (none) - every word equal; the level's
    /// removal is called exactly when the piece id is not -1.
    #[test]
    fn bumper_reset_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let m4 = |w: &[f32]| Mat4::from_cols(Vec4::from_slice(&w[0..4]), Vec4::from_slice(&w[4..8]), Vec4::from_slice(&w[8..12]), Vec4::from_slice(&w[12..16]));
        let v3 = |w: &[f32], k: usize| Vec3::new(w[k], w[k + 1], w[k + 2]);
        let mut n = 0;
        for line in include_str!("testdata/bumper_reset_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let iv: Vec<&str> = i.split_whitespace().collect();
            let w: Vec<f32> = iv[..25].iter().map(|x| f(x)).collect();
            let ints: Vec<i64> = iv[25..].iter().map(|x| x.parse().unwrap()).collect();
            let ov: Vec<u32> = o.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let c = m4(&w[0..16]);
            let mut b = Bumper::new(true, Mat4::IDENTITY, 2);
            (b.a.rest, b.b.rest) = (v3(&w, 16), v3(&w, 20));
            (b.a.state, b.b.state, b.clock) = (ints[0] as u8, ints[1] as u8, w[24]);
            b.detached = Some(Loose { level: None, own_body: true, pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, ang_vel: Vec3::ZERO, touching: 0, still: 0.0 });
            b.reset(&c);
            let bits = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            let want3 = |k: usize| [ov[k], ov[k + 1], ov[k + 2]];
            for (e, k, name) in [(&b.a, 0, "A"), (&b.b, 20, "B")] {
                for (got, at, what) in [(e.pos, k, "pos"), (e.prev, k + 4, "prev"), (e.local, k + 8, "local"), (e.local_prev, k + 12, "local prev")] {
                    assert_eq!(bits(got), want3(at), "{name} {what}: {line}");
                }
                assert_eq!(want3(k + 16), [0; 3], "{name} force: {line}");
            }
            assert_eq!((b.clock.to_bits(), u32::from(b.a.state), u32::from(b.b.state), b.detached.is_some()), (ov[40], ov[41], ov[42], ov[43] != 0), "clock / states / piece: {line}");
            assert_eq!((ov[44], ov[45] != 0), (u32::MAX, ints[2] != -1), "piece id / removal: {line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The queued hit against F-57572 run offline (tools/bumper_reset_hit_reference.py, testdata/bumper_hit_ref.txt:
    /// 3,000 random cars, rest points, hits at and round 0.8 m from an end, amounts at and an ulp above 0.15 and NaN,
    /// damage ratios at and round the heavy ratio, kinds 0 / 1, every pair of states): both states equal (and +486
    /// stays 0), with heavy = damage / maximum > the ratio as the original divides and compares.
    #[test]
    fn bumper_hit_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let m4 = |w: &[f32]| Mat4::from_cols(Vec4::from_slice(&w[0..4]), Vec4::from_slice(&w[4..8]), Vec4::from_slice(&w[8..12]), Vec4::from_slice(&w[12..16]));
        let v3 = |w: &[f32], k: usize| Vec3::new(w[k], w[k + 1], w[k + 2]);
        let mut n = 0;
        for line in include_str!("testdata/bumper_hit_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let iv: Vec<&str> = i.split_whitespace().collect();
            let w: Vec<f32> = iv[..37].iter().map(|x| f(x)).collect();
            let ints: Vec<u8> = iv[37..].iter().map(|x| x.parse().unwrap()).collect();
            let ov: Vec<u32> = o.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let c = m4(&w[0..16]);
            let mut b = Bumper::new(true, Mat4::IDENTITY, 2);
            (b.a.rest, b.b.rest) = (v3(&w, 16), v3(&w, 20));
            (b.a.state, b.b.state) = (ints[1], ints[2]);
            let heavy = w[33] / w[34] > w[35];
            b.hit(&c, &PartHit { point: v3(&w, 28), amount: w[32], kind: ints[0] }, heavy);
            assert_eq!([u32::from(b.a.state), u32::from(b.b.state), 0], [ov[0], ov[1], ov[2]], "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// tools/slot_pass_reference.py: F-14987 run whole offline on 400 random cars; each bone a moved slot's part
    /// hangs on, moved by the slots in the original's order (756 moves), is bit-equal to `slot_move` applied in that order.
    #[test]
    fn slot_pass_matches_the_original() {
        let words = |s: &str| s.split_whitespace().map(|w| u32::from_str_radix(w, 16).unwrap()).collect::<Vec<u32>>();
        let mat = |w: &[u32]| Mat4::from_cols_array(&std::array::from_fn(|i| f32::from_bits(w[i])));
        let (mut n, mut moves) = (0, 0);
        for line in include_str!("testdata/slot_pass_ref.txt").lines().filter(|l| !l.is_empty()) {
            let f: Vec<&str> = line.split('|').collect();
            let a = words(f[1]);
            let am = [mat(&a[0..16]), mat(&a[16..32]), mat(&a[32..48])];
            let bones: Vec<&str> = f[3].split_whitespace().collect();
            let mut g: std::collections::HashMap<u32, (Mat4, Mat4)> = std::collections::HashMap::new();
            for c in bones.chunks(33) {
                let w = words(&c[1..].join(" "));
                g.insert(c[0].parse().unwrap(), (mat(&w[0..16]), mat(&w[16..32])));
            }
            for e in f[2].split_whitespace().filter(|e| !e.starts_with('B')) {
                let (k, b) = e.split_once(':').unwrap();
                let (k, Ok(b)) = (k.parse::<u32>().unwrap(), b.parse::<u32>()) else { continue };
                let (ai, s) = match k {
                    3 => (0, BUMPER_SCALE),
                    5 | 6 => (1, BUMPER_SCALE),
                    2 | 40 => (2, HOOD_SCALE),
                    _ => continue,
                };
                let Some(m) = g.get_mut(&b) else { continue };
                m.0 = slot_move(&m.0, &am[ai], s);
                moves += 1;
            }
            for (b, (port, orig)) in &g {
                let (p, o) = (port.to_cols_array().map(f32::to_bits), orig.to_cols_array().map(f32::to_bits));
                assert_eq!(p, o, "case {n} bone {b}: {line}");
            }
            n += 1;
        }
        assert_eq!(n, 400);
        assert_eq!(moves, 756);
    }

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

    /// A particle swung sideways is put back into the car's YZ plane (A-15101), so the hood never opens past its
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
            let car_in = CarIn { hull: &hull, hull_cg: hull.cg, hull_margin: f32::from_bits(0x3d23_d70a), now, last, prev: last, speed: cv.length(), bound: (hull.min - Vec3::splat(0.041), hull.max + Vec3::splat(0.041)), wheels: Vec::new(), vel: cv, touch: (tune.s.bound_friction, tune.s.bound_elasticity), wiped: false };
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

#[cfg(test)]
#[path = "damageparts_offline.rs"]
mod offline;
