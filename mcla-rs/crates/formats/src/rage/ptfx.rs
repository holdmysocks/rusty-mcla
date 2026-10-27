//! The effect list of RAGE's particle system "rmptfx" (`resources/rmptfx/fxlists/MC4.r107`,
//! a `ptxFxList`).
//!
//! Root: +08 texture dictionary (64), +10 model dictionary (13 drawables of the model
//! particles), +14 particle rules `ptxRule` (145: 135 `ptxSprite`, 10 `ptxModel`), +1c emit
//! rules `ptxEmitRuleStd` (172), +20 effect rules `ptxEffectRuleStd` (93). All dictionaries are
//! keyed by joaat(name).
//!
//! The member offsets come from the executable's own registration of these classes with its
//! parser (name -> offset tables filled by `F-43039`, `F-22325`, `F-62740`,
//! `F-51412` ...), what a member means from the code that uses it (`F-63457` spawn
//! count, `F-70396` spawn, `F-61736` particle start, `F-12752` particle step,
//! `F-91662` / `3250` / `3488` domain points, `F-31524` timeline). The notes with the
//! full tables are in `work/map/ptfx_rule.md`.

use super::drawable::Shader;
use super::geometry::decode;
use super::scan::geometry_at;
use super::{Dictionary, Rd};
use crate::rsc::Resource;
use crate::{bail, Result};
use glam::Vec3;

/// One key of an `rmPtfxKeyframe` (48 bytes): time +00, value +10, slope +20.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    pub t: f32,
    pub v: [f32; 4],
    /// change of the value per unit of time up to the next key
    pub d: [f32; 4],
}

/// `rmPtfxKeyframe` (40 bytes): keys -> +18, count +1c. Four channels over a time of 0..1.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keyframe {
    pub keys: Vec<Key>,
}

impl Keyframe {
    pub fn parse(rd: &Rd, off: usize) -> Keyframe {
        let n = rd.u16(off + 0x1c) as usize;
        let Some(p) = rd.ptr(off + 0x18) else { return Keyframe::default() };
        if n > 256 || !rd.has(p, n * 0x30) {
            return Keyframe::default();
        }
        let f4 = |o: usize| [rd.f32(o), rd.f32(o + 4), rd.f32(o + 8), rd.f32(o + 12)];
        Keyframe { keys: (0..n).map(|i| p + i * 0x30).map(|k| Key { t: rd.f32(k), v: f4(k + 0x10), d: f4(k + 0x20) }).collect() }
    }

    /// The executable's evaluation (the loop at the head of F-42828 and of the emit rule's three evaluators F-87862, F-96958, F-56010): the key before
    /// the first one whose time is not below `t` (the last key when none is), value + slope *
    /// (t - its time).
    pub fn eval(&self, t: f32) -> [f32; 4] {
        if self.keys.is_empty() {
            return [0.0; 4];
        }
        let mut i = 1;
        while i < self.keys.len() && self.keys[i].t < t {
            i += 1;
        }
        let k = &self.keys[i - 1];
        let dt = t - k.t;
        // (the original's fmadds: one rounding for value + slope * dt)
        [k.d[0].mul_add(dt, k.v[0]), k.d[1].mul_add(dt, k.v[1]), k.d[2].mul_add(dt, k.v[2]), k.d[3].mul_add(dt, k.v[3])]
    }

    /// No key has a value other than zero.
    pub fn is_zero(&self) -> bool {
        self.keys.iter().all(|k| k.v == [0.0; 4])
    }
}

/// `ptxDomain` +08 (the class the loader gives it, `F-67552`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Box = 0,
    Sphere = 1,
    Cylinder = 2,
    Vortex = 3,
}

/// `ptxDomain`: where particles start (the emitter domain) or where they head (the velocity
/// domain).
#[derive(Debug, Clone)]
pub struct Domain {
    pub shape: Shape,
    /// +0c `DomainFunction`: 0 the emitter's, 1 the velocity's
    pub function: u32,
    /// +04: all three sizes and the position are multiplied by it (1.0 in every rule)
    pub scale: f32,
    /// +124 `WorldSpace`: not turned by the effect's matrix
    pub world_space: bool,
    /// +125 `PointRelative`: the velocity is the domain's point itself; without it the vector
    /// from the particle's start to that point
    pub point_relative: bool,
    /// +10 `PositionKF`: centre (m), over the emitter's time
    pub position: Keyframe,
    /// +38 `DirectionKF`: turn about x, then y, then z (degrees)
    pub direction: Keyframe,
    /// +60 `SizeKF`. Box: half extents. Sphere: x outer, y inner radius. Cylinder: x / z radii
    /// of the ellipse, y half height (axis y).
    pub size: Keyframe,
    /// +88 `InnerSizeKF` (box, cylinder): the hollow inside
    pub inner_size: Keyframe,
}

impl Domain {
    fn parse(rd: &Rd, off: usize) -> Domain {
        let shape = match rd.u32(off + 0x08) {
            1 => Shape::Sphere,
            2 => Shape::Cylinder,
            3 => Shape::Vortex,
            _ => Shape::Box,
        };
        Domain { shape, function: rd.u32(off + 0x0c), scale: rd.f32(off + 0x04), world_space: rd.u8(off + 0x124) != 0, point_relative: rd.u8(off + 0x125) != 0, position: Keyframe::parse(rd, off + 0x10), direction: Keyframe::parse(rd, off + 0x38), size: Keyframe::parse(rd, off + 0x60), inner_size: Keyframe::parse(rd, off + 0x88) }
    }

    /// A point of the domain at the emitter's time `t`, in the domain's own frame turned by
    /// `DirectionKF` but without its position (`F-91662` box, `F-70386` sphere,
    /// `F-79490` cylinder). `rnd` gives numbers in 0..1.
    pub fn point(&self, t: f32, rnd: &mut dyn FnMut() -> f32) -> Vec3 {
        self.point_sized(t, self.size.eval(t), self.inner_size.eval(t), rnd)
    }

    /// `point` with the size and inner size given (an evolution may change them).
    pub fn point_sized(&self, t: f32, s: [f32; 4], i: [f32; 4], rnd: &mut dyn FnMut() -> f32) -> Vec3 {
        let s = Vec3::new(s[0], s[1], s[2]) * self.scale;
        let i = Vec3::new(i[0], i[1], i[2]) * self.scale;
        let p = match self.shape {
            Shape::Box | Shape::Vortex => {
                let mut p = Vec3::new((rnd() * 2.0 - 1.0) * s.x, (rnd() * 2.0 - 1.0) * s.y, (rnd() * 2.0 - 1.0) * s.z);
                // a hollow box: one coordinate is pushed out of the inner box (the original
                // picks one of the slabs between the two; the spread inside a slab is the same)
                let axes: Vec<usize> = (0..3).filter(|&a| i[a] > 0.0 && i[a] < s[a]).collect();
                if !axes.is_empty() && p.abs().cmplt(i).all() {
                    let a = axes[((rnd() * axes.len() as f32) as usize).min(axes.len() - 1)];
                    let m = i[a] + (s[a] - i[a]) * rnd();
                    p[a] = if rnd() < 0.5 { -m } else { m };
                }
                p
            }
            Shape::Sphere => {
                let d = Vec3::new(rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0).normalize_or_zero();
                d * (s.y + (s.x - s.y) * rnd())
            }
            Shape::Cylinder => {
                if i.x >= s.x && i.y >= s.y && i.z >= s.z {
                    Vec3::ZERO
                } else {
                    let a = rnd() * std::f32::consts::TAU;
                    let (sn, cs) = a.sin_cos();
                    let outer = Vec3::new(s.x * cs, 0.0, s.z * sn);
                    let inner = Vec3::new(i.x * cs, 0.0, i.z * sn).length();
                    let r = inner + (outer.length() - inner) * rnd();
                    let d = outer.normalize_or_zero() * r;
                    Vec3::new(d.x, -s.y + 2.0 * s.y * rnd(), d.z)
                }
            }
        };
        self.turn(t) * p
    }

    /// The turn of `DirectionKF` at `t` (`F-84330`: about x, then y, then z).
    pub fn turn(&self, t: f32) -> glam::Quat {
        let d = self.direction.eval(t);
        if d[0] == 0.0 && d[1] == 0.0 && d[2] == 0.0 {
            return glam::Quat::IDENTITY;
        }
        glam::Quat::from_rotation_z(d[2].to_radians()) * glam::Quat::from_rotation_y(d[1].to_radians()) * glam::Quat::from_rotation_x(d[0].to_radians())
    }

    /// The centre at `t` (m, in the effect's frame unless `world_space`).
    pub fn centre(&self, t: f32) -> Vec3 {
        let p = self.position.eval(t);
        Vec3::new(p[0], p[1], p[2]) * self.scale
    }
}

/// `ptxEmitRuleStd` (512 bytes): when and where particles start.
#[derive(Debug, Clone)]
pub struct EmitRule {
    pub name: String,
    /// +0c `Duration` (s): the emitter's time runs 0..1 over it (times the event's scalar)
    pub duration: f32,
    /// +1fc `OneShot`: the spawn rate is a count, let out at once
    pub one_shot: bool,
    pub emitter_domain: Option<Domain>,
    pub velocity_domain: Option<Domain>,
    /// +34 `SpawnRateKF`: x..y particles a second (a count when `one_shot`), z the most that
    /// may be alive (0 = no limit)
    pub spawn_rate: Keyframe,
    /// +5c `SpawnDistKF`: x..y particles per metre the emitter moves
    pub spawn_dist: Keyframe,
    /// +84 `TimeScaleKF`: x scales the time of the rule's particles and of its spawning
    pub time_scale: Keyframe,
    /// +ac `SpawnLifeKF`: x..y life (s)
    pub spawn_life: Keyframe,
    /// +d4 `SpeedKF`: x..y factor on the velocity domain's vector
    pub speed: Keyframe,
    /// +fc `SizeKFOT`: per cent on the particle's size (x width, y height), at its birth
    pub size_ot: Keyframe,
    /// +124 `AccelerationKFOT`: per cent on the acceleration's x, y, z
    pub accel_ot: Keyframe,
    /// +14c `DampeningKFOT`: per cent on the damping's x, y, z
    pub damp_ot: Keyframe,
    /// +174 `MatrixWeightKFOT`: per cent on the matrix weight
    pub matrix_weight_ot: Keyframe,
    /// +19c `InheritVelKFOT`: x..y per cent of the emitter's velocity given to the particle
    pub inherit_vel: Keyframe,
}

impl EmitRule {
    fn parse(rd: &Rd, o: usize) -> EmitRule {
        let kf = |k: usize| Keyframe::parse(rd, o + 0x20 + k);
        EmitRule {
            name: rd.cstr_ptr(o + 0x1f0).unwrap_or_default(),
            duration: rd.f32(o + 0x0c),
            one_shot: rd.u8(o + 0x1fc) != 0,
            emitter_domain: rd.ptr(o + 0x24).map(|d| Domain::parse(rd, d)),
            velocity_domain: rd.ptr(o + 0x2c).map(|d| Domain::parse(rd, d)),
            spawn_rate: kf(0x14),
            spawn_dist: kf(0x3c),
            time_scale: kf(0x64),
            spawn_life: kf(0x8c),
            speed: kf(0xb4),
            size_ot: kf(0xdc),
            accel_ot: kf(0x104),
            damp_ot: kf(0x12c),
            matrix_weight_ot: kf(0x154),
            inherit_vel: kf(0x17c),
        }
    }
}

/// Bits of `ptxRule` +11c that the particle step and start test (`F-12752`,
/// `F-61736`). The names are the parser's member names, matched to the bits by what the
/// code does with them and by their order.
pub mod flag {
    /// position noise not on x / y / z
    pub const RESTRICT_NOISE_X: u32 = 1 << 4;
    pub const RESTRICT_NOISE_Y: u32 = 1 << 5;
    pub const RESTRICT_NOISE_Z: u32 = 1 << 6;
    /// the damping is not turned into the effect's frame
    pub const WORLD_SPACE_DAMPENING: u32 = 1 << 7;
    /// the acceleration is not turned by the effect's matrix
    pub const WORLD_SPACE_ACCELERATION: u32 = 1 << 8;
    /// the wind's influence is not scaled by (1 - matrix weight)
    pub const WIND_IGNORES_MATRIX_WEIGHT: u32 = 1 << 10;
    /// sprite: not a billboard - along its velocity, or flat on `Sprite::axis` when that is set
    pub const DRAW_DIRECTIONAL: u32 = 1 << 11;
    pub const ANIMATE_TEXTURES: u32 = 1 << 12;
    pub const ANIMATE_RANDOM: u32 = 1 << 13;
    pub const KEY_TEX_ANIM_RATE_ON_EMIT_TIME: u32 = 1 << 20;
    /// the animation runs once through `frame_start..=anim_end` over the particle's life
    pub const SCALE_TEX_ANIM_RATE_BY_LIFE: u32 = 1 << 21;
    pub const HOLD_LAST_TEX_FRAME: u32 = 1 << 22;
    /// sprite: the colour lies between `ColorKF` and `ColorMaxKF` by a random number of the
    /// particle; without it `ColorMaxKF` is not read (`F-66151` A-14861, the bit from
    /// `F-89961` A-32260; model particles: bit 7, `F-29887` A-24501)
    pub const RANDOM_COLOR: u32 = 1 << 23;
    pub const MODEL_RANDOM_COLOR: u32 = 1 << 7;
    /// the acceleration is multiplied by the event's zoom
    pub const ZOOM_ACCELERATION: u32 = 1 << 26;
    /// DrawDirectional: the back half reaches no further than the spawn point (`blk+64`)
    pub const ANCHOR_TO_SPAWN_POS: u32 = 1 << 14;
    /// DrawDirectional: `DirectionalKF` / `DirectionalVelKF` keyed on the emitter's time, not the life
    pub const KEY_DIRECTIONAL_ON_EMIT_TIME: u32 = 1 << 18;
    pub const KEY_DIRECTIONAL_VEL_ON_EMIT_TIME: u32 = 1 << 19;
    /// DrawDirectional: side (1, 0, 0), direction (0, 1, 0)
    pub const USE_DIRECTIONAL_IDENTITY: u32 = 1 << 28;
    /// DrawDirectional: the side is direction x `Sprite::axis` (not x the way to the camera)
    pub const ALIGN_TO_AXIS: u32 = 1 << 30;
    /// bit 9 (its name not read): the sprite draw sorts the emitter's particles, largest key
    /// first - the distance squared to the camera, or with `SORT_RANDOM` the random number the
    /// particle was born with (F-43541; docs/reference/checkpoints/arrow_draw_order.md)
    pub const SORT_PARTICLES: u32 = 1 << 9;
    pub const SORT_RANDOM: u32 = 1 << 29;
    /// UseParticleTrails: a strip along the particle's last positions instead of the sprite
    /// (`F-56257`; `Sprite::trail`)
    pub const USE_PARTICLE_TRAILS: u32 = 1 << 31;
}

/// `ptxRenderState` (`ptxRule` +100).
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderState {
    /// +04 `BlendSet`: 0 on 124 rules (smoke, water, debris), 7 on 21 (flames, sparks, glows)
    pub blend_set: u32,
    /// +08 `DepthBias`
    pub depth_bias: f32,
    /// +0c `LightingMode` (0, 1, 2)
    pub lighting_mode: u32,
    pub depth_write: bool,
    pub depth_test: bool,
    pub alpha_blend: bool,
}

/// What a `ptxSprite` adds to the rule.
#[derive(Debug, Clone)]
pub struct Sprite {
    /// +160 `AxisAligned`: read only with `flag::ALIGN_TO_AXIS` (and `DRAW_DIRECTIONAL`): the
    /// quad's side is direction x this, raw (not turned, not normalised) - (0, 1, 0) for the
    /// rings on water, which then lie flat
    pub axis: Vec3,
    /// +170 / +174: the first frame is picked at random in this range
    pub frame_start: i32,
    pub frame_end: i32,
    /// +178 `EndTexAnimFrame`: the animation runs `frame_start..=anim_end`
    pub anim_end: i32,
    /// +17c / +180: the atlas (frames count along x first)
    pub tiles_x: u32,
    pub tiles_y: u32,
    /// `SizeKF`: half width x..z, half height y..w (m) over the particle's life
    pub size: Keyframe,
    /// `ThetaKF` (degrees, not seen in use: zero on every sprite of the effects played)
    pub theta: Keyframe,
    /// `InitThetaKFOT`: x..y first turn of the sprite (degrees), over the emitter's time
    pub init_theta: Keyframe,
    /// `InitRotateVelKFOT`: x..y first spin (turns a second), over the emitter's time
    pub init_rot_vel: Keyframe,
    /// `RotateVelKF`: x..y spin (turns a second) over the particle's life
    pub rot_vel: Keyframe,
    /// `DirectionalKF`, `DirectionalVelKF`: DrawDirectional's extents = (Zy, Zy, Zx, Zx) x
    /// DirectionalKF + DirectionalVelKF x speed (front, back, left, right; `F-35603`)
    pub directional: Keyframe,
    pub directional_vel: Keyframe,
    /// `TextureAnimRateKF`: x..y frames a second
    pub tex_anim_rate: Keyframe,
    /// `TrailPropsKF`: x..y the trail's length (m), z..w its number of points (at least 3), by
    /// one random number of the particle, on the emitter's time (`F-61736`, `F-56257`)
    pub trail: Keyframe,
    /// +678 shader data: the effect file, its technique and the sprite's texture
    pub shader: String,
    pub technique: String,
    pub texture: String,
    /// the shader data's float variables (`rmPtfxShaderVar`, type +12: 2 float .. 5 float4,
    /// the values at +20) by their name (+08), e.g. `mc4_rmptfx_projected`'s ProjMapOffset,
    /// ProjMapBounds, ProjMapTile, ProjMapRate, NightArrowBoost, ArrowColorTint
    pub vars: Vec<(String, [f32; 4])>,
}

/// One drawable of a `ptxModel` (model particles: flyers, leaves, cans).
#[derive(Debug, Clone)]
pub struct ModelDrawable {
    pub name: String,
    /// the drawable's box
    pub min: Vec3,
    pub max: Vec3,
    /// diffuse texture of its first shader and the part of it the model shows (u0, v0, u1, v1)
    pub texture: String,
    pub uv: [f32; 4],
    /// the geometry of its first level of detail (drawable +0x40; all 13 have LOD 0 only, one
    /// `mc4_rmptfx_model` shader each), every geometry of it in one triangle list
    pub mesh: ModelMesh,
}

/// The triangles of a model particle's drawable, in the drawable's own frame.
#[derive(Debug, Clone, Default)]
pub struct ModelMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    /// the vertex colour's four bytes as stored (`Mesh::colors`); 255s where the geometry has none
    pub colors: Vec<[u8; 4]>,
    pub indices: Vec<u32>,
}

/// What a `ptxModel` adds to the rule (`F-34386` birth, `F-91023` step,
/// `F-29887` draw; `work/map2/model/model_particle_rule.md`).
#[derive(Debug, Clone)]
pub struct Model {
    pub drawables: Vec<ModelDrawable>,
    /// `SizeMin` / `SizeMax` (PL+0x31c / +0x350): scale of the model (x, y, z) over the particle's life
    pub size_min: Keyframe,
    pub size_max: Keyframe,
    /// `InitialRotationMin` / `Max` (PL+0x454 / +0x488, the parser's names): the code takes them
    /// as the SPIN AXIS - x, y, z each between the two by a fresh random number, then normalised
    pub init_rotation_min: Keyframe,
    pub init_rotation_max: Keyframe,
    /// `InitRotationSpeed` (PL+0x4bc, x..y turns a second, keyed on the emitter's time), the spin
    /// at birth; `RotationSpeed` (PL+0x4f0, x..y turns a second over the life) added every step
    pub init_rotation_speed: Keyframe,
    pub rotation_speed: Keyframe,
    /// `InitialThetaMin` / `Max` (PL+0x384 / +0x3b8, degrees about x, y, z, keyed on the emitter's
    /// time): the Euler angles at birth
    pub init_theta_min: Keyframe,
    pub init_theta_max: Keyframe,
    /// `ThetaMin` (PL+0x3ec, degrees, keyed on the emitter's time): added to the Euler angles at
    /// every draw (ThetaMax is read but its lerp factor is the constant 0.0)
    pub theta_min: Keyframe,
}

/// `ptxRule` (`ptxSprite` / `ptxModel`, 1696 bytes): how a particle looks and moves.
#[derive(Debug, Clone)]
pub struct ParticleRule {
    pub name: String,
    /// +11c, see [`flag`]
    pub flags: u32,
    pub render: RenderState,
    /// `ColorKF` / `ColorMaxKF`: the colour lies between the two (one random number per
    /// particle); the alpha channel is not used (the loader of old files sets it to 1)
    pub color: Keyframe,
    pub color_max: Keyframe,
    /// `AccelerationMinKF` / `MaxKF` (m/s2, x y z) over the particle's life
    pub accel_min: Keyframe,
    pub accel_max: Keyframe,
    /// `DampeningMinKF` / `MaxKF` (1/s, x y z): velocity *= max(1 - d dt, 0)
    pub damp_min: Keyframe,
    pub damp_max: Keyframe,
    /// `MatrixWeightKF` x..y: how much of the emitter's movement the particle follows
    pub matrix_weight: Keyframe,
    /// `PlaybackRateKF` x..y: speed of the particle's own clock
    pub playback_rate: Keyframe,
    /// `AlphaKF` x..y over the particle's life
    pub alpha: Keyframe,
    /// `PositionNoiseKF` x..y, `VelocityNoiseKF` x..y (velocity) and z..w (direction): noise
    /// amplitudes over the particle's life (`F-92194`)
    pub position_noise: Keyframe,
    pub velocity_noise: Keyframe,
    /// `WindInfluenceKF` x..y: share of the wind the particle takes
    pub wind_influence: Keyframe,
    /// +114 `PhysicalRange` (-1: every particle within the effect's collision polygons' reach; 0:
    /// no collision; else the range in metres), +118 `StopVel`, +132 / +133 `PercentPhysical` /
    /// `PercentKill` (per cent; `flags` bit 0 enables the kill), `CollisionVelDmpKF` (per cent
    /// lost) and `CollisionImpVarKF` (`work/map2/trail/trail_collision_rule.md` 1)
    pub physical_range: f32,
    pub stop_vel: f32,
    pub percent_physical: u8,
    pub percent_kill: u8,
    pub vel_dmp: Keyframe,
    pub imp_var: Keyframe,
    pub sprite: Option<Sprite>,
    pub model: Option<Model>,
}

fn parse_model_drawable(rd: &Rd, name: String, dr: Option<usize>) -> ModelDrawable {
    let mut d = ModelDrawable { name, min: Vec3::ZERO, max: Vec3::ZERO, texture: String::new(), uv: [0.0, 0.0, 1.0, 1.0], mesh: ModelMesh::default() };
    let Some(dr) = dr else { return d };
    let v = |k: usize| Vec3::new(rd.f32(dr + k), rd.f32(dr + k + 4), rd.f32(dr + k + 8));
    d.min = v(0x20);
    d.max = v(0x30);
    if let Some(sg) = rd.ptr(dr + 0x08) {
        let count = rd.u16(sg + 0x0c) as usize;
        if let Some(arr) = rd.ptr(sg + 0x08) {
            for i in 0..count.min(8) {
                if let Some(t) = rd.ptr(arr + i * 4).and_then(|so| Shader::parse(rd, so).ok()).and_then(|s| s.diffuse().map(|t| t.to_string())) {
                    d.texture = t;
                    break;
                }
            }
        }
    }
    // the part of the texture the first level of detail shows
    let (mut lo, mut hi) = (glam::Vec2::splat(f32::INFINITY), glam::Vec2::splat(f32::NEG_INFINITY));
    for lod in 0..4 {
        let Some(slot) = rd.ptr(dr + 0x40 + lod * 4) else { continue };
        let n = rd.u16(slot + 4) as usize;
        let Some(arr) = rd.ptr(slot) else { continue };
        if n == 0 || n > 16 {
            continue;
        }
        for i in 0..n {
            let Some(m) = rd.ptr(arr + i * 4) else { continue };
            let cnt = rd.u16(m + 0x08) as usize;
            let Some(ga) = rd.ptr(m + 0x04) else { continue };
            for g in 0..cnt.min(16) {
                let Some(gr) = rd.ptr(ga + g * 4).and_then(|g| geometry_at(rd, g)) else { continue };
                if let Ok(mesh) = decode(rd, &gr.vb, &gr.ib) {
                    for uv in &mesh.uv0 {
                        lo = lo.min(*uv);
                        hi = hi.max(*uv);
                    }
                    // (the triangles, appended to those of the geometries before)
                    let n = mesh.positions.len();
                    let base = d.mesh.positions.len() as u32;
                    let mm = &mut d.mesh;
                    mm.positions.extend(mesh.positions.iter().map(|p| p.to_array()));
                    mm.normals.extend((0..n).map(|i| mesh.normals.get(i).map_or([0.0, 0.0, 1.0], |v| v.to_array())));
                    mm.uv.extend((0..n).map(|i| mesh.uv0.get(i).map_or([0.0, 0.0], |v| v.to_array())));
                    mm.colors.extend((0..n).map(|i| mesh.colors.get(i).copied().unwrap_or([255; 4])));
                    mm.indices.extend(mesh.indices.iter().filter(|&&k| (k as usize) < n).map(|k| k + base));
                }
            }
        }
        if lo.x.is_finite() {
            break;
        }
    }
    if lo.x.is_finite() && hi.x > lo.x && hi.y > lo.y {
        d.uv = [lo.x, lo.y, hi.x, hi.y];
    }
    d
}

impl ParticleRule {
    fn parse(rd: &Rd, o: usize) -> ParticleRule {
        // +128: the class name, in place ("ptxsprite" / "ptxmodel")
        let is_model = rd.cstr(o + 0x128) == "ptxmodel";
        // the property list: ptxSprite +188, ptxModel +174; a property is 52 bytes, keyframe first
        let pl = o + if is_model { 0x174 } else { 0x188 };
        let kf = |k: usize| Keyframe::parse(rd, pl + k);
        let rs = o + 0x100;
        let render = RenderState { blend_set: rd.u32(rs + 4), depth_bias: rd.f32(rs + 8), lighting_mode: rd.u32(rs + 0x0c), depth_write: rd.u8(rs + 0x10) != 0, depth_test: rd.u8(rs + 0x11) != 0, alpha_blend: rd.u8(rs + 0x12) != 0 };
        let (mut sprite, mut model) = (None, None);
        if is_model {
            let mut drawables = Vec::new();
            if let Some(arr) = rd.ptr(o + 0x168) {
                // ptxDrawable: name -> +0, drawable -> +4
                for i in 0..(rd.u16(o + 0x16c) as usize).min(16) {
                    drawables.push(parse_model_drawable(rd, rd.cstr_ptr(arr + i * 8).unwrap_or_default(), rd.ptr(arr + i * 8 + 4)));
                }
            }
            model = Some(Model {
                drawables,
                size_min: kf(0x31c),
                size_max: kf(0x350),
                init_rotation_min: kf(0x454),
                init_rotation_max: kf(0x488),
                init_rotation_speed: kf(0x4bc),
                rotation_speed: kf(0x4f0),
                init_theta_min: kf(0x384),
                init_theta_max: kf(0x3b8),
                theta_min: kf(0x3ec),
            });
        } else {
            let sh = o + 0x678;
            let mut texture = String::new();
            let mut fvars = Vec::new();
            if let Some(vars) = rd.ptr(sh + 0x14) {
                for i in 0..(rd.u16(sh + 0x18) as usize).min(32) {
                    let Some(v) = rd.ptr(vars + i * 4) else { continue };
                    // rmPtfxShaderVar: name -> +08, type +10 (6 = texture: object -> +20, name -> +24)
                    let ty = rd.u8(v + 0x12);
                    if ty == 6 {
                        if texture.is_empty() {
                            if let Some(n) = rd.cstr_ptr(v + 0x24) {
                                texture = n;
                            }
                        }
                    } else if (2..=5).contains(&ty) {
                        let mut val = [0.0f32; 4];
                        for (k, x) in val.iter_mut().enumerate().take(ty as usize - 1) {
                            *x = rd.f32(v + 0x20 + k * 4);
                        }
                        fvars.push((rd.cstr_ptr(v + 0x08).unwrap_or_default(), val));
                    }
                }
            }
            let tiles_x = rd.u32(o + 0x17c).clamp(1, 64);
            let tiles_y = rd.u32(o + 0x180).clamp(1, 64);
            let last = (tiles_x * tiles_y) as i32 - 1;
            // (the loader: an end of -1 means the whole atlas)
            let anim_end = match rd.u32(o + 0x178) as i32 {
                -1 => last,
                e => e.clamp(0, last),
            };
            sprite = Some(Sprite {
                axis: Vec3::new(rd.f32(o + 0x160), rd.f32(o + 0x164), rd.f32(o + 0x168)),
                frame_start: (rd.u32(o + 0x170) as i32).clamp(0, last),
                frame_end: (rd.u32(o + 0x174) as i32).clamp(0, last),
                anim_end,
                tiles_x,
                tiles_y,
                size: kf(0x31c),
                theta: kf(0x350),
                init_theta: kf(0x384),
                init_rot_vel: kf(0x3b8),
                rot_vel: kf(0x3ec),
                directional: kf(0x420),
                directional_vel: kf(0x454),
                tex_anim_rate: kf(0x488),
                trail: kf(0x4bc),
                shader: rd.cstr_ptr(sh + 4).unwrap_or_default(),
                technique: rd.cstr_ptr(sh + 8).unwrap_or_default(),
                texture,
                vars: fvars,
            });
        }
        ParticleRule {
            name: rd.cstr_ptr(o + 0x124).unwrap_or_default(),
            flags: rd.u32(o + 0x11c),
            render,
            color: kf(0x10),
            color_max: kf(0x44),
            accel_min: kf(0x78),
            accel_max: kf(0xac),
            damp_min: kf(0xe0),
            damp_max: kf(0x114),
            matrix_weight: kf(0x148),
            playback_rate: kf(0x17c),
            alpha: kf(0x1b0),
            position_noise: kf(0x1e4),
            velocity_noise: kf(0x218),
            wind_influence: kf(0x2b4),
            physical_range: rd.f32(o + 0x114),
            stop_vel: rd.f32(o + 0x118),
            percent_physical: rd.u8(o + 0x132),
            percent_kill: rd.u8(o + 0x133),
            vel_dmp: kf(0x24c),
            imp_var: kf(0x280),
            sprite,
            model,
        }
    }
}

/// What an event of an effect's timeline starts.
#[derive(Debug, Clone)]
pub enum EventKind {
    /// `ptxEventEmitter` (+14 = 0): an emit rule lets particles of a particle rule out.
    Emitter {
        /// indices into [`FxList::emit_rules`] / [`FxList::particle_rules`] (+48 / +4c)
        emit: Option<usize>,
        particle: Option<usize>,
        /// +20..+34: min / max of the factor on the emit rule's duration, on its clock, of the
        /// zoom (size and domain scale)
        duration_scalar: [f32; 2],
        time_scalar: [f32; 2],
        zoom: [f32; 2],
        /// +38 / +3c `ColorTintMin` / `Max` (rgba bytes)
        tint: [u32; 2],
    },
    /// `ptxEventEffect` (+14 = 1): another effect is started.
    Effect {
        /// index into [`FxList::effects`] (+94; name -> +90)
        effect: Option<usize>,
        name: String,
        /// +20 `RotationMin` (degrees)
        rotation: Vec3,
        /// -> +9c `EmitterDomain`: where in the parent's frame
        domain: Option<Domain>,
    },
}

/// `ptxEvent`.
#[derive(Debug, Clone)]
pub struct Event {
    /// +08 `TriggerTime` (s into the effect's loop)
    pub time: f32,
    /// +18 `TriggerCap` (-1 in every event)
    pub cap: i32,
    pub kind: EventKind,
    /// -> +0c `EvoData`: what the effect instance's evolution values change in this event
    pub evo: Option<EvoGroup>,
}

/// One property an evolution changes (`F-31783`): the list (0 the emit rule's
/// properties - 0 SpawnRate, 1 TimeScale, 2 SpawnLife, 3 Speed, 4 SizeOT, 5 AccelerationOT,
/// 6 DampeningOT, 7 MatrixWeightOT, 8 InheritVelOT, 9 SpawnDist -; 1 the emitter domain, 2 the
/// velocity domain - 0 position, 1 direction, 2 size, 3 inner size -; 3 the particle rule's
/// properties by their id, +28 of each: 0 Color, 4 Size, 19 Alpha ...), how they blend, and the
/// keyframe of each evolution that has one.
#[derive(Debug, Clone)]
pub struct EvoProp {
    pub list: u8,
    pub prop: u8,
    /// 0 the mean of base..value by weight over the evolutions on (`F-18828`), 1 the base
    /// plus the sum of weight x (value - base) (`F-76095`), 2 the largest
    /// (`F-81297`), 3 the mean over all evolutions, the base for those off (`F-39561`)
    pub mode: u8,
    pub overrides: Vec<(u8, Keyframe)>,
}

/// `ptxEvolutionGroup`: +00 -> the evolutions {name -> +0, -> per list a table (+0 40 bytes:
/// slot of each property or -1, +28 -> its `rmPtxfxProp`s), u16 count +8}, u16 count +4; +08 ->
/// per list the blend mode of each property (40 bytes); +10 a bit per list x 40 + property.
#[derive(Debug, Clone, Default)]
pub struct EvoGroup {
    pub names: Vec<String>,
    pub props: Vec<EvoProp>,
}

impl EvoGroup {
    fn parse(rd: &Rd, g: usize) -> EvoGroup {
        let n = (rd.u16(g + 4) as usize).min(16);
        let evos: Vec<usize> = match rd.ptr(g) {
            Some(p) => (0..n).filter_map(|i| rd.ptr(p + i * 4)).collect(),
            None => Vec::new(),
        };
        let names = evos.iter().map(|e| rd.cstr_ptr(*e).unwrap_or_default()).collect();
        let mut props = Vec::new();
        for idx in 0..160usize {
            if rd.u32(g + 16 + (idx >> 5) * 4) >> (idx & 31) & 1 == 0 {
                continue;
            }
            let (list, prop) = (idx / 40, idx % 40);
            let mode = rd.ptr(g + 8).map_or(0, |m| rd.ptr(m + list * 8).map_or(0, |l| rd.u8(l + prop)));
            let mut overrides = Vec::new();
            for (ei, e) in evos.iter().enumerate() {
                if list >= rd.u16(e + 8) as usize {
                    continue;
                }
                let Some(ent) = rd.ptr(e + 4).and_then(|t| rd.ptr(t + list * 4)) else { continue };
                let slot = rd.u8(ent + prop) as i8;
                if slot < 0 {
                    continue;
                }
                if let Some(p) = rd.ptr(ent + 40).and_then(|a| rd.ptr(a + slot as usize * 4)) {
                    overrides.push((ei as u8, Keyframe::parse(rd, p)));
                }
            }
            props.push(EvoProp { list: list as u8, prop: prop as u8, mode, overrides });
        }
        EvoGroup { names, props }
    }

    /// A property's value with the evolutions applied (`F-31783` and its four modes):
    /// `base` is the property's own value at `t`, `values` the instance's evolution values.
    pub fn evolve(&self, list: u8, prop: u8, base: [f32; 4], t: f32, values: &[f32]) -> [f32; 4] {
        let Some(p) = self.props.iter().find(|p| p.list == list && p.prop == prop) else { return base };
        let lerp = |v: [f32; 4], w: f32| -> [f32; 4] { [base[0] + (v[0] - base[0]) * w, base[1] + (v[1] - base[1]) * w, base[2] + (v[2] - base[2]) * w, base[3] + (v[3] - base[3]) * w] };
        let w_of = |ei: u8| values.get(ei as usize).copied().unwrap_or(0.0);
        match p.mode {
            1 => {
                let mut out = base;
                for (ei, kf) in &p.overrides {
                    let w = w_of(*ei);
                    if w > 0.0 {
                        let v = kf.eval(t);
                        for c in 0..4 {
                            out[c] += (v[c] - base[c]) * w;
                        }
                    }
                }
                out
            }
            2 => {
                let mut out = base;
                for (ei, kf) in &p.overrides {
                    let w = w_of(*ei);
                    if w > 0.0 {
                        let v = lerp(kf.eval(t), w);
                        for c in 0..4 {
                            out[c] = out[c].max(v[c]);
                        }
                    }
                }
                out
            }
            3 => {
                let n = self.names.len().max(1);
                let mut acc = [0.0f32; 4];
                for ei in 0..n as u8 {
                    let w = w_of(ei);
                    let v = match p.overrides.iter().find(|o| o.0 == ei) {
                        Some((_, kf)) if w > 0.0 => lerp(kf.eval(t), w),
                        _ => base,
                    };
                    for c in 0..4 {
                        acc[c] += v[c];
                    }
                }
                acc.map(|x| x / n as f32)
            }
            _ => {
                let (mut acc, mut k) = ([0.0f32; 4], 0);
                for (ei, kf) in &p.overrides {
                    let w = w_of(*ei);
                    if w > 0.0 {
                        let v = lerp(kf.eval(t), w);
                        for c in 0..4 {
                            acc[c] += v[c];
                        }
                        k += 1;
                    }
                }
                if k == 0 {
                    base
                } else {
                    acc.map(|x| x / k as f32)
                }
            }
        }
    }

    /// Index of the evolution called `name` (`F-95887`).
    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
}

/// `ptxEffectRuleStd` (416 bytes) with its `ptxTimeLine` (+120).
#[derive(Debug, Clone)]
pub struct EffectRule {
    pub name: String,
    /// +15c / +160 `DurationMin` / `Max` (s): length of one loop of the timeline
    pub duration: [f32; 2],
    /// +164 / +168 `TimeScalarMin` / `Max`: speed of the effect's clock
    pub time_scalar: [f32; 2],
    /// timeline +08 `PreUpdate` (s the effect has already run when it starts)
    pub pre_update: f32,
    /// timeline +0c `NumLoops`: -1 for ever, 0 once
    pub num_loops: i32,
    /// +14c `CullDistance`, +144 `FadeDistance`, +150 / +154 `LodNearDistance` / `Far`
    pub cull_distance: f32,
    pub fade_distance: f32,
    pub lod_near: f32,
    pub lod_far: f32,
    /// +0x58 `ZoomKF` (per cent, x..y by a random number of the instance, over the effect's
    /// time): scales every particle's size and the emitter / velocity domains with the event's
    /// zoom (`F-11951` A-89936 -> instance +500; `F-89961` A-55427)
    pub zoom: Keyframe,
    /// +0x180 / +0x148: the cull sphere's centre offset in the effect's frame and its radius
    /// (`F-66872`: centre = effect matrix x offset x scale, radius x scale, 1 when <= 0) -
    /// the lit sprites' light picker works at its centre
    pub cull_offset: [f32; 3],
    pub cull_radius: f32,
    /// +0x178 `SortEvents`: the draw takes the emitter events farthest first instead of in
    /// the timeline's order (draw_order_rule.md; ptxEffectRuleStd::Draw F-20173)
    pub sort_events: bool,
    pub events: Vec<Event>,
    /// +100 -> `PtxEvoGroup`: the effect's evolutions by name (the instance keeps a value for
    /// each, `F-38579`; "LOD" is set from the camera's distance, `F-11951`)
    pub evo_names: Vec<String>,
}

/// The whole effect list.
#[derive(Debug, Clone, Default)]
pub struct FxList {
    pub effects: Vec<EffectRule>,
    pub emit_rules: Vec<EmitRule>,
    pub particle_rules: Vec<ParticleRule>,
    /// joaat(name) of `effects`, in the same order
    pub effect_hashes: Vec<u32>,
}

impl FxList {
    pub fn parse(res: &Resource) -> Result<FxList> {
        let rd = Rd::new(res);
        let dict = |o: usize| -> Result<Dictionary> {
            match rd.ptr(o) {
                Some(d) => Dictionary::parse(&rd, d),
                None => bail!("effect list: no dictionary at +{:#x}", o),
            }
        };
        let (dp, de, df) = (dict(0x14)?, dict(0x1c)?, dict(0x20)?);
        let index = |d: &Dictionary| -> hashbrown::HashMap<usize, usize> { d.values.iter().enumerate().filter_map(|(i, v)| v.map(|o| (o, i))).collect() };
        let (ip, ie, ifx) = (index(&dp), index(&de), index(&df));
        let particle_rules: Vec<ParticleRule> = dp.values.iter().map(|v| ParticleRule::parse(&rd, v.unwrap_or(0))).collect();
        let emit_rules: Vec<EmitRule> = de.values.iter().map(|v| EmitRule::parse(&rd, v.unwrap_or(0))).collect();
        let mut effects = Vec::with_capacity(df.len());
        for v in &df.values {
            let o = v.unwrap_or(0);
            let tl = o + 0x120;
            let mut events = Vec::new();
            if let Some(arr) = rd.ptr(tl + 0x10) {
                for i in 0..(rd.u16(tl + 0x14) as usize).min(64) {
                    let Some(e) = rd.ptr(arr + i * 4) else { continue };
                    let kind = if rd.u32(e + 0x14) == 1 {
                        EventKind::Effect { effect: rd.ptr(e + 0x94).and_then(|p| ifx.get(&p).copied()), name: rd.cstr_ptr(e + 0x90).unwrap_or_default(), rotation: Vec3::new(rd.f32(e + 0x20), rd.f32(e + 0x24), rd.f32(e + 0x28)), domain: rd.ptr(e + 0x9c).map(|d| Domain::parse(&rd, d)) }
                    } else {
                        EventKind::Emitter { emit: rd.ptr(e + 0x48).and_then(|p| ie.get(&p).copied()), particle: rd.ptr(e + 0x4c).and_then(|p| ip.get(&p).copied()), duration_scalar: [rd.f32(e + 0x20), rd.f32(e + 0x24)], time_scalar: [rd.f32(e + 0x28), rd.f32(e + 0x2c)], zoom: [rd.f32(e + 0x30), rd.f32(e + 0x34)], tint: [rd.u32(e + 0x38), rd.u32(e + 0x3c)] }
                    };
                    events.push(Event { time: rd.f32(e + 0x08), cap: rd.u32(e + 0x18) as i32, kind, evo: rd.ptr(e + 0x0c).map(|g| EvoGroup::parse(&rd, g)) });
                }
            }
            effects.push(EffectRule { name: rd.cstr_ptr(o + 0xf8).unwrap_or_default(), duration: [rd.f32(o + 0x15c), rd.f32(o + 0x160)], time_scalar: [rd.f32(o + 0x164), rd.f32(o + 0x168)], pre_update: rd.f32(tl + 0x08), num_loops: rd.u32(tl + 0x0c) as i32, cull_distance: rd.f32(o + 0x14c), fade_distance: rd.f32(o + 0x144), lod_near: rd.f32(o + 0x150), lod_far: rd.f32(o + 0x154), zoom: Keyframe::parse(&rd, o + 0x58), cull_offset: [rd.f32(o + 0x180), rd.f32(o + 0x184), rd.f32(o + 0x188)], cull_radius: rd.f32(o + 0x148), sort_events: rd.u8(o + 0x178) != 0, events, evo_names: rd.ptr(o + 0x100).map(|g| EvoGroup::parse(&rd, g).names).unwrap_or_default() });
        }
        Ok(FxList { effects, emit_rules, particle_rules, effect_hashes: df.hashes.clone() })
    }

    /// Index of the effect rule called `name`.
    pub fn effect(&self, name: &str) -> Option<usize> {
        let h = crate::hash::joaat(name);
        self.effect_hashes.iter().position(|&x| x == h)
    }
}

/// Class id of `evtInstancePtfx` in the prop file.
const VT_EVT_PTFX: u32 = 0x0068_0940;

/// The effects a prop's fragment type keeps running (`resources/prop/0x0e66973c.r131`): type
/// +d4 -> its groups; group +98 -> the set of its continuous events {-> instances +4, count
/// +8}; an `evtInstancePtfx` has the effect's name in place at +30 and where it sits in the
/// prop's frame at +20. Only the particle props (`la_prop_pt_*`) and the flare have any.
pub fn prop_effects(res: &Resource, frag: usize) -> Vec<(String, Vec3)> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(groups) = rd.ptr(frag + 0xd4) else { return out };
    // (the particle props have one group; a pointer that is none ends the list)
    for gi in 0..8 {
        let Some(g) = rd.ptr(groups + gi * 4) else { break };
        let Some(set) = rd.ptr(g + 0x98) else { continue };
        let Some(arr) = rd.ptr(set + 4) else { continue };
        let n = rd.u16(set + 8) as usize;
        if n > 16 {
            continue;
        }
        for i in 0..n {
            let Some(e) = rd.ptr(arr + i * 4) else { continue };
            if rd.vtable(e) == VT_EVT_PTFX {
                out.push((rd.cstr(e + 0x30), Vec3::new(rd.f32(e + 0x20), rd.f32(e + 0x24), rd.f32(e + 0x28))));
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// The rails of `city/sc/0xad0a8b94.r66` (`trash.xrn`) as segments, each with the effect its
/// group runs on the player's car near it: a rail (class 0x479d24, 0x60 bytes: +4c u16
/// points, +50 -> u16 vertex indices; vertices root +2c, vec4 each) points at its group at
/// +08, the group has joaat(effect name) at +90 (`wind_debris_hw` / `_la` / `_beach`).
pub fn trash_rail_effects(res: &Resource) -> Vec<(Vec3, Vec3, u32)> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(verts) = rd.ptr(0x2c) else { return out };
    let v = |i: usize| Vec3::new(rd.f32(verts + i * 16), rd.f32(verts + i * 16 + 4), rd.f32(verts + i * 16 + 8));
    let mut o = 0;
    while o + 0x60 <= res.virtual_size {
        if rd.vtable(o) != 0x0047_9d24 {
            o += 4;
            continue;
        }
        let n = rd.u16(o + 0x4c) as usize;
        let effect = rd.ptr(o + 0x08).map_or(0, |g| rd.u32(g + 0x90));
        if let Some(idx) = rd.ptr(o + 0x50).filter(|p| n >= 2 && n < 256 && rd.has(*p, n * 2)) {
            let pts: Vec<Vec3> = (0..n).map(|k| v(rd.u16(idx + k * 2) as usize)).filter(|p| p.is_finite()).collect();
            out.extend(pts.windows(2).map(|w| (w[0], w[1], effect)));
        }
        o += 0x60;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptfx_trash_rails() {
        let Some(res) = file("resources/city/sc/0xad0a8b94.r66") else { return };
        let rails = trash_rail_effects(&res);
        let n = |name: &str| rails.iter().filter(|r| r.2 == crate::hash::joaat(name)).count();
        assert!(rails.len() > 300 && n("wind_debris_hw") + n("wind_debris_la") + n("wind_debris_beach") == rails.len());
        assert!(n("wind_debris_la") > 50 && n("wind_debris_beach") > 20 && n("wind_debris_hw") > 100);
    }

    fn file(rel: &str) -> Option<Resource> {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let d = std::fs::read(std::path::Path::new(&root).join(rel)).ok()?;
        if d.len() < 16 || &d[0..4] != b"MCLR" {
            return None;
        }
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        Some(Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data })
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-3 * b.abs().max(1.0)
    }

    #[test]
    fn ptfx_keyframe_eval() {
        let k = Keyframe { keys: vec![Key { t: 0.0, v: [1.0, 0.0, 0.0, 0.0], d: [2.0, 0.0, 0.0, 0.0] }, Key { t: 0.5, v: [2.0, 0.0, 0.0, 0.0], d: [0.0; 4] }] };
        assert_eq!(k.eval(0.25)[0], 1.5);
        assert_eq!(k.eval(0.5)[0], 2.0);
        assert_eq!(k.eval(0.9)[0], 2.0);
        assert_eq!(Keyframe::default().eval(0.3), [0.0; 4]);
    }

    #[test]
    fn ptfx_effect_list() {
        let Some(res) = file("resources/rmptfx/fxlists/MC4.r107") else { return };
        let fx = FxList::parse(&res).unwrap();
        assert_eq!((fx.effects.len(), fx.emit_rules.len(), fx.particle_rules.len()), (93, 172, 145));
        assert_eq!(fx.particle_rules.iter().filter(|r| r.model.is_some()).count(), 10);
        assert!(fx.particle_rules.iter().all(|r| r.sprite.is_some() != r.model.is_some() && !r.name.is_empty()));
        // every event of every effect finds its rules
        for e in &fx.effects {
            for ev in &e.events {
                match &ev.kind {
                    EventKind::Emitter { emit, particle, .. } => assert!(emit.is_some() && particle.is_some(), "{}", e.name),
                    EventKind::Effect { effect, name, .. } => assert!(effect.is_some(), "{} -> {}", e.name, name),
                }
            }
        }

        // water_fountain_beverly: every 4 s a spout (at once) and, 1.45 s later, its splash
        let bev = &fx.effects[fx.effect("water_fountain_beverly").unwrap()];
        assert_eq!((bev.duration, bev.num_loops, bev.events.len()), ([4.0, 4.0], -1, 2));
        let sub = |i: usize| match &bev.events[i].kind {
            EventKind::Effect { effect, name, .. } => (bev.events[i].time, name.as_str(), &fx.effects[effect.unwrap()]),
            _ => panic!(),
        };
        let (t, name, splash) = sub(0);
        assert!(near(t, 1.45) && name == "water_fountain_spash" && splash.num_loops == 0 && splash.duration == [3.0, 3.0]);
        let (t, name, sprout) = sub(1);
        assert!(t == 0.0 && name == "water_fountain_beverly_sprout" && sprout.events.len() == 2);
        let EventKind::Emitter { emit, particle, zoom, .. } = &sprout.events[0].kind else { panic!() };
        let (em, pr) = (&fx.emit_rules[emit.unwrap()], &fx.particle_rules[particle.unwrap()]);
        assert_eq!((em.name.as_str(), pr.name.as_str(), *zoom), ("water_fountain_jet_beverly", "water_fountain_jet_beverly", [1.0, 1.0]));
        // the jet: 9 sprites at once, 1.55 s, straight up at 5.7 m/s, pulled back at 7..7.5 m/s2
        assert!(em.one_shot && em.duration == 1.0);
        assert!(near(em.spawn_rate.eval(0.0)[0], 9.057) && near(em.spawn_life.eval(0.0)[1], 1.55) && em.speed.eval(0.0)[0] == 1.0);
        let (ed, vd) = (em.emitter_domain.as_ref().unwrap(), em.velocity_domain.as_ref().unwrap());
        assert!(ed.shape == Shape::Box && ed.function == 0 && ed.size.is_zero() && ed.direction.eval(0.0)[0] == -90.0);
        assert!(vd.shape == Shape::Cylinder && vd.function == 1 && vd.point_relative && !vd.world_space);
        assert!(near(vd.centre(0.0).y, 5.7) && near(vd.size.eval(0.0)[1], 0.02116));
        assert_eq!(em.accel_ot.eval(0.0), [100.0, 100.0, -100.0, 100.0]);
        assert!(pr.flags == 0x0780_4100 && pr.render.blend_set == 0 && pr.render.depth_test && !pr.render.depth_write);
        assert!(pr.accel_min.eval(0.0)[1] == -7.0 && pr.accel_max.eval(0.0)[1] == -7.5 && pr.alpha.keys.len() == 5);
        assert!((pr.alpha.eval(0.443)[1] - 0.6954).abs() < 2e-3 && pr.alpha.eval(1.0)[1].abs() < 1e-4 && pr.alpha.eval(0.0) == [0.0; 4]);
        let sp = pr.sprite.as_ref().unwrap();
        assert_eq!((sp.texture.as_str(), sp.technique.as_str(), sp.tiles_x, sp.tiles_y, sp.frame_start, sp.frame_end), ("water_splash_4f_256", "draw_SoftPart_VertLit", 2, 2, 0, 3));
        let s0 = sp.size.eval(0.0);
        assert!(near(s0[0], 0.01583) && near(s0[1], 0.2711) && near(s0[2], 0.06331) && near(s0[3], 0.3939));
        assert_eq!(sp.init_theta.eval(0.0)[..2], [-5.0, 5.0]);
        // the drops beside it: at most three alive
        let EventKind::Emitter { emit, particle, .. } = &sprout.events[1].kind else { panic!() };
        assert!(fx.emit_rules[emit.unwrap()].spawn_rate.eval(0.0)[2] == 3.0 && fx.particle_rules[particle.unwrap()].sprite.as_ref().unwrap().texture == "water_drop");

        // steam: an animated 7 x 7 sheet, slowed to half speed
        let steam = &fx.effects[fx.effect("amb_steam_grate").unwrap()];
        let EventKind::Emitter { emit, particle, time_scalar, .. } = &steam.events[0].kind else { panic!() };
        let (em, pr) = (&fx.emit_rules[emit.unwrap()], &fx.particle_rules[particle.unwrap()]);
        let sp = pr.sprite.as_ref().unwrap();
        assert!(em.time_scale.eval(0.0)[0] == 0.5 && em.spawn_rate.eval(0.0)[0] == 10.0 && near(time_scalar[0], 0.9));
        assert!(sp.texture == "smoke_cloud_anim" && sp.tiles_x == 7 && sp.anim_end == 48 && pr.flags & flag::ANIMATE_TEXTURES != 0 && pr.flags & flag::SCALE_TEX_ANIM_RATE_BY_LIFE != 0);

        // the litter on the wind: model particles, flyers from one atlas
        let litter = &fx.effects[fx.effect("wind_debris_la").unwrap()];
        assert_eq!(litter.events.len(), 3);
        let EventKind::Emitter { particle, .. } = &litter.events[1].kind else { panic!() };
        let m = fx.particle_rules[particle.unwrap()].model.as_ref().unwrap();
        assert!(m.drawables.iter().any(|d| d.name == "flyer_la" && d.texture.starts_with("flyers") && d.max.x > 0.1 && d.uv[2] - d.uv[0] < 0.9), "{:?}", m.drawables.iter().map(|d| &d.name).collect::<Vec<_>>());
        // every model particle's drawable has its triangles, and the rules their orientation keys
        for r in fx.particle_rules.iter().filter(|r| r.model.is_some()) {
            let m = r.model.as_ref().unwrap();
            for d in &m.drawables {
                let mm = &d.mesh;
                assert!(!mm.indices.is_empty() && mm.indices.len() % 3 == 0 && mm.normals.len() == mm.positions.len() && mm.uv.len() == mm.positions.len(), "{} {}", r.name, d.name);
                if std::env::var("MCLA_MODEL_DUMP").is_ok() {
                    let c0 = mm.colors.first().copied().unwrap_or([0; 4]);
                    let cmin = (0..4).map(|k| mm.colors.iter().map(|c| c[k]).min().unwrap_or(0)).collect::<Vec<_>>();
                    eprintln!("{} {}: {} verts {} tris tex {} box {:?}..{:?} colour first {:?} min {:?} n0 {:?}", r.name, d.name, mm.positions.len(), mm.indices.len() / 3, d.texture, d.min, d.max, c0, cmin, mm.normals[0]);
                }
            }
            if std::env::var("MCLA_MODEL_DUMP").is_ok() {
                eprintln!("{} flags {:#x}: theta {:?}..{:?} theta_min {:?} axis {:?}..{:?} speed {:?} rs {:?} size {:?}..{:?} damp w {:?}", r.name, r.flags, m.init_theta_min.eval(0.0), m.init_theta_max.eval(0.0), m.theta_min.eval(0.0), m.init_rotation_min.eval(0.0), m.init_rotation_max.eval(0.0), m.init_rotation_speed.eval(0.0), m.rotation_speed.eval(0.0), m.size_min.eval(0.5), m.size_max.eval(0.5), r.damp_min.eval(0.0)[3]);
            }
        }
        if std::env::var("MCLA_MODEL_DUMP").is_ok() {
            let hw = &fx.effects[fx.effect("wind_debris_hw").unwrap()];
            eprintln!("wind_debris_hw zoom keys {:?} evo names {:?}", hw.zoom.keys, hw.evo_names);
            for ev in &hw.events {
                if let EventKind::Emitter { emit: Some(e), particle: Some(p), zoom, .. } = &ev.kind {
                    let er = &fx.emit_rules[*e];
                    eprintln!("  {} / {}: event zoom {:?} size_ot keys {:?} evo {:?}", er.name, fx.particle_rules[*p].name, zoom, er.size_ot.keys, ev.evo.as_ref().map(|g| (&g.names, g.props.iter().map(|q| (q.list, q.prop, q.mode)).collect::<Vec<_>>())));
                }
            }
        }
        let paper = &fx.particle_rules[fx.particle_rules.iter().position(|r| r.name == "wind_debris_paper").unwrap()];
        let m = paper.model.as_ref().unwrap();
        assert!(near(m.init_theta_min.eval(0.0)[0], 79.5) || m.init_theta_min.eval(0.0)[0] > 70.0, "{:?}", m.init_theta_min.eval(0.0));
    }

    #[test]
    fn ptfx_prop_effects() {
        let Some(res) = file("resources/prop/0x0e66973c.r131") else { return };
        let props = super::super::props::parse_props(&res);
        let of = |n: &str| -> Vec<String> { prop_effects(&res, props.iter().find(|p| p.name == n).unwrap().off).into_iter().map(|e| e.0).collect() };
        assert_eq!(of("la_prop_pt_fountain_beverly_01x"), ["water_fountain_beverly"]);
        assert_eq!(of("la_prop_pt_fountain_01x"), ["water_fountain", "water_rings_b"]);
        assert_eq!(of("la_prop_pt_sewergrate_01x"), ["amb_steam_grate"]);
        assert_eq!(of("la_prop_pt_smokestack_01x"), ["amb_smoke_stack"]);
        assert_eq!(of("la_prop_pt_fountain_cc"), ["water_fountain_cc_m"]);
        // every particle prop has its effect in the file
        let pt: Vec<&str> = props.iter().filter(|p| p.name.starts_with("la_prop_pt_")).map(|p| p.name.as_str()).collect();
        assert_eq!(pt.len(), 14);
        assert!(pt.iter().all(|n| !of(n).is_empty()));
        assert!(of("la_prop_trash_can_04x").is_empty());
    }
    /// The key-frame evaluation against the original's own code run offline (tools/map_keyframe_reference.py):
    /// four functions, 1,500 random key tables each (0..9 keys, times on and between the keys), every output bit for bit.
    #[test]
    fn keyframe_eval_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for l in include_str!("testdata/map_keyframe_ref.txt").lines() {
            let (head, table) = l.split_once(" | ").unwrap();
            let h: Vec<&str> = head.split_whitespace().collect();
            let count: usize = h[1].parse().unwrap();
            let t = f(h[2]);
            let w: Vec<f32> = table.split_whitespace().map(f).collect();
            assert_eq!(w.len(), count * 9);
            let keys = (0..count)
                .map(|i| {
                    let k = &w[i * 9..i * 9 + 9];
                    Key { t: k[0], v: [k[1], k[2], k[3], k[4]], d: [k[5], k[6], k[7], k[8]] }
                })
                .collect();
            let got = Keyframe { keys }.eval(t);
            for j in 0..4 {
                assert_eq!(got[j].to_bits(), f(h[3 + j]).to_bits(), "{l} lane {j}");
            }
            n += 1;
        }
        assert_eq!(n, 6000);
    }
}
