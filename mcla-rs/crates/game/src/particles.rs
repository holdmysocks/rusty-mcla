//! Particle effects with the game's own sprites (`resources/rmptfx/fxlists/MC4.r107`):
//! tyre smoke, nitrous flames, impact sparks, backfire puffs, water spray - the port's own
//! rules - and the effects of the particle props and the litter on the wind, played from the
//! disc's effect rules (`mcla_formats::rage::ptfx`; `play`, `step_effect`, `step_emitter`,
//! `step_rule_particles`). Notes: `work/map/ptfx_rule.md`.

use crate::assets::load_resource_file;
use crate::render::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use hashbrown::HashMap;
use mcla_formats::rage::ptfx::{flag, EventKind, FxList};
use mcla_formats::rage::texture::Texture;
use mcla_formats::rage::{Dictionary, Rd};
use std::path::Path;
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PInst {
    pos_size: [f32; 4],
    color: [f32; 4],
    uv: [f32; 4],
    misc: [f32; 4],
    /// the checkpoints' smoke (misc.w 3): ShadowColor rgb, the night boost
    extra: [f32; 4],
    /// the arrows projected onto the checkpoints' low smoke (`mc4_rmptfx_projected`, technique
    /// "draw": VSCheckpointProj / PSCheckpointProj): a = EffectPos + ProjMapOffset, EffectPos.w;
    /// b = ProjMapBounds, the target's two turns (`checkpoints::arrow_angles`); c = ArrowColorTint,
    /// gNightArrowBoost; d = the arrow texture (0..3), ProjMapTile, CurrentTime x ProjMapRate, night
    arrow_a: [f32; 4],
    arrow_b: [f32; 4],
    arrow_c: [f32; 4],
    arrow_d: [f32; 4],
    /// the rule's sprites: frame A's u0, v0 (`uv` holds frame B), the blend between them
    /// (`blk+84`, vertex float 17 of `F-70297`)
    uv2: [f32; 4],
    /// DrawDirectional (`F-35603`): the quad's half extents from its centre (`pos_size`) -
    /// u towards the front edge (frame top), v towards the right edge; `quad_u.w` 1 = in use
    quad_u: [f32; 4],
    quad_v: [f32; 4],
    /// the `_BaseLight` programs: BaseLightColor (c98), gFallOff (c97); BaseLightDir (c82), the
    /// base's height (c94.w); zero = none
    base_a: [f32; 4],
    base_b: [f32; 4],
}

impl PInst {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<PInst>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 48, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 64, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 80, shader_location: 5 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 96, shader_location: 6 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 112, shader_location: 7 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 128, shader_location: 8 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 144, shader_location: 9 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 160, shader_location: 10 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 176, shader_location: 11 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 192, shader_location: 12 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 208, shader_location: 13 },
        ],
    };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Smoke = 0,
    Flame = 1,
    Spark = 2,
    Glow = 3,
    Splash = 4,
}

const KINDS: usize = 5;
/// (texture name, atlas columns, atlas rows, additive)
const SHEETS: [(&str, u32, u32, bool); KINDS] = [("smoke_new_plumes", 2, 2, false), ("flames_hb_256", 8, 8, true), ("spark_solid_64", 1, 1, true), ("additive_glow_core_a_64", 1, 1, true), ("water_splash_4f_256", 2, 2, false)];

struct Particle {
    kind: Kind,
    pos: Vec3,
    vel: Vec3,
    age: f32,
    life: f32,
    size0: f32,
    size1: f32,
    color: [f32; 4],
    rot: f32,
    spin: f32,
    frame: u32,
    gravity: f32,
    drag: f32,
}

/// How a particle rule of the effect list is drawn here: a sprite sheet, or - for the model
/// particles (flyers, leaves) - a card per drawable with the part of the texture it shows.
struct Look {
    cards: Vec<Card>,
}

#[derive(Clone, Copy)]
struct Card {
    /// index into `Particles::binds`
    sheet: usize,
    /// u0, v0, du, dv (models; sprites take their atlas frame)
    uv: [f32; 4],
    /// half width and height of the model's sheet (1, 1 for sprites)
    half: [f32; 2],
}

/// A particle of a rule of the effect list.
struct RuleParticle {
    rule: u16,
    card: u16,
    /// the emitter that let it out and the effect instance it belongs to
    uid: u32,
    inst: u32,
    pos: Vec3,
    vel: Vec3,
    /// where it was before its last step (`blk+32`, copied by the update before it moves) and
    /// where it was let out (`blk+64`, AnchorToSpawnPos)
    prev: Vec3,
    spawn: Vec3,
    /// its clock 0..1 and 1 / life
    t: f32,
    inv_life: f32,
    /// what its emitter makes of a second (effect and event scalar, `TimeScaleKF`)
    ts: f32,
    /// its random numbers: size, colour, alpha, acceleration, damping, playback, matrix
    /// weight, spin, wind, noise, trail
    r: [f32; 11],
    theta: f32,
    spin0: f32,
    /// the texture frames it blends between (`blk+92` A, `blk+93` B) by `anim`
    frame0: u8,
    /// its random sort key (node +112, drawn at birth): the draw order with `SORT_RANDOM`
    sort_key: f32,
    frame: u8,
    anim: f32,
    /// the emit rule's factors at its birth (size x y; acceleration, damping x y z; matrix weight)
    size_ot: [f32; 2],
    accel_ot: Vec3,
    damp_ot: Vec3,
    mw_ot: f32,
    /// its effect's evolution values at its birth (`F-61736`: instance +436.. copied to the particle's +76..; the
    /// particle rule's evolutions, list 3, read this copy)
    evo: [f32; 4],
    zoom: f32,
    /// the effect's turn at its birth (accelerations in the effect's frame)
    rot: Quat,
    /// distance fade of its effect
    fade: f32,
    /// the emitter's time at its birth
    te: f32,
    /// its effect's colour tint and shadow colour
    tint: [f32; 3],
    shadow: [f32; 3],
    /// the effect rule and the event that let it out (for the event's evolutions)
    eff: u16,
    ev: u16,
    /// its effect's frame is mirrored (`Effect::flip`)
    flip: bool,
    /// UseParticleTrails: its last positions, oldest first (`blk+32` .. `blk+64`), the share of the
    /// way to the next shift (`f`) and the trail's length
    trail: Option<Box<Trail>>,
    /// a model particle's turn and scale state (modelfx.rs)
    orient: crate::modelfx::Orient,
    /// d+116 bit 2 (it collides with its effect's polygons: the share PercentPhysical decided at
    /// its birth) and bit 1 (at rest after a collision)
    physical: bool,
    at_rest: bool,
}

/// A particle's trail (`F-56257`).
struct Trail {
    nodes: Vec<Vec3>,
    f: f32,
    len: f32,
}

/// One event's emitter of a playing effect.
#[derive(Clone, Copy)]
struct Emitter {
    playing: bool,
    t: f32,
    dur: f32,
    tsc: f32,
    zoom: f32,
    carry: f32,
    uid: u32,
    /// where the emitter was when it last let particles out by distance (em +64: set to its place
    /// at its start, `F-87518`; `F-63457` moves it along the path by the share spawned)
    prev: Option<Vec3>,
}

/// A playing effect: its timeline and one emitter per event; the effects its events start
/// hang below it.
struct Effect {
    key: u64,
    id: u32,
    effect: usize,
    pos: Vec3,
    rot: Quat,
    vel: Vec3,
    /// how far `play` moved it since the last step
    moved: Vec3,
    t: f32,
    dur: f32,
    rate: f32,
    loops: i32,
    done: bool,
    seen: u64,
    fade: f32,
    em: Vec<Emitter>,
    kids: Vec<Effect>,
    /// the instance's colour tint (+400 of the effect instance; rules with flag bit 3 take it -
    /// `F-29887`, `F-89961`) and an opacity of the caller's (the checkpoints')
    tint: [f32; 3],
    alpha: f32,
    /// the checkpoint render setup's ShadowColor (`F-79844`: the look's m_ParticleShadowColor)
    shadow: [f32; 3],
    /// the instance's evolution values (+436, one per name of the rule's group - `F-38579`)
    evo: [f32; 4],
    /// the checkpoint render setup's +48 (EffectPos.w): the projected arrows are on
    arrow: bool,
    /// the instance's random number for its `ZoomKF` (instance byte 492)
    zr: f32,
    /// started once and left to run out (`burst`): it plays without being asked again and is
    /// dropped when it and what it let out are done
    oneshot: bool,
    /// the frame the effect follows is a mirror image (the tyres' left wheels, `tyrefx.rs`): its
    /// local x is negated before `rot` turns it
    flip: bool,
}

struct Rng(u32);

impl Rng {
    fn f(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x % 100000) as f32 / 100000.0
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}

pub struct Particles {
    list: Vec<Particle>,
    /// sprite sheets: the port's own `KINDS` first, then those of the effect list's rules
    binds: Vec<Option<Arc<wgpu::BindGroup>>>,
    additive: Vec<bool>,
    buf: Option<wgpu::Buffer>,
    /// the scene's depth for the particles' bind group 2 (made again every frame: the depth
    /// target is remade when the window changes size)
    depth_bg: Option<wgpu::BindGroup>,
    /// the draws after the last `prepare`: sheet, first instance, count - in the original's order
    runs: Vec<(u32, u32, u32)>,
    rng: u32,
    cap: usize,
    /// the disc's effect list and, per particle rule, how it is drawn (none: cannot be)
    fx: Option<Arc<FxList>>,
    looks: Vec<Option<Look>>,
    /// per effect rule: something of it can be drawn
    playable: Vec<bool>,
    by_hash: HashMap<u32, usize>,
    effects: Vec<Effect>,
    rule_list: Vec<RuleParticle>,
    frame: u64,
    next_id: u32,
    /// `MCLA_FX_STANDIN=1`: the effect rules are not played (the callers fall back)
    standin: bool,
    /// the clock (hours): the checkpoint smoke's night boost
    pub hour: f32,
    /// the lit sprites' fog: RMPTFX_FogMinColor rgba and RMPTFX_FogParams.x (the height)
    pub fog_min: [f32; 5],
    /// the projected arrows' 128 x 128 target this frame: its quad's two turns and which
    /// `checkpoint_arrows_anim_*` it shows (`checkpoints::Markers::update`)
    pub arrow_rt: [f32; 3],
    /// seconds the effects have run (the projected map's CurrentTime)
    clock: f32,
    /// the collision polygons of the effect instances (inst +512, `set_collision`): by effect id
    coll: HashMap<u32, Poly>,
}

/// A polygon the particles of an effect instance collide with (`trail_collision_rule.md` 1a): four
/// corners, the plane normal, and the effect's place (for a rule's PhysicalRange).
#[derive(Clone, Copy)]
pub struct Poly {
    pub v: [Vec3; 4],
    pub n: Vec3,
}

/// The point of the triangle (a, b, c) nearest to `p`.
fn tri_nearest(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let den = 1.0 / (va + vb + vc);
    a + ab * (vb * den) + ac * (vc * den)
}

/// Is the point (on the polygon's plane) inside the triangle?
fn in_tri(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> bool {
    let n = (b - a).cross(c - a);
    (b - a).cross(p - a).dot(n) >= 0.0 && (c - b).cross(p - b).dot(n) >= 0.0 && (a - c).cross(p - c).dot(n) >= 0.0
}

impl Poly {
    /// The depth of a particle's contact with the polygon, or none (`F-22746` for a swept
    /// particle, `F-17804` for a sphere): the polygon is two triangles (v0 v1 v2) (v0 v2 v3).
    fn depth(&self, pos: Vec3, vel: Vec3, dt: f32, radius: f32) -> Option<f32> {
        let swept = vel.length() * dt > 0.7 * radius;
        let v = &self.v;
        if swept {
            let (start, end) = (pos - vel * dt, pos);
            let (ds, de) = ((start - v[0]).dot(self.n), (end - v[0]).dot(self.n));
            if !(ds * de < 0.0) {
                return None;
            }
            let t = ds / (ds - de);
            if !(0.0..=1.0).contains(&t) {
                return None;
            }
            let at = start + (end - start) * t;
            (in_tri(at, v[0], v[1], v[2]) || in_tri(at, v[0], v[2], v[3])).then(|| radius - de)
        } else {
            let a = tri_nearest(pos, v[0], v[1], v[2]);
            let b = tri_nearest(pos, v[0], v[2], v[3]);
            let d = (pos - a).length().min((pos - b).length());
            (d <= radius).then(|| radius - d)
        }
    }
}

const MAX: usize = 3000;
/// The particle shader's mode for the programs ported as the original draws them (the scene's
/// linear value, not a display colour): 4 = `mc4_rmptfx_mainlist`'s draw_SoftPart_VertLit
/// (the flares' fire and glows when they add, smoke and steam when alpha-blended), 6 = its
/// draw_SoftPart_TexBlend_VertLit and _Rescale (the same vertex light, the two frames blended:
/// the `_BaseLight` variants: the same pixel programs, a light from below added in the vertex -
/// `PInst::base_a` / `base_b`;
/// the tyre smoke; the two techniques' programs are the same microcode; draw_TexBlend_VertLit is
/// the same light and pixel rule without the soft fade), 5 = `mc4_rmptfx_default`'s unlit_draw
/// (the checkpoints' sparks) and `mc4_rmptfx_mainlist`'s unlit draw_SoftPart_TexBlend (its
/// vertex colour as it is, frames A and B blended, soft fade 4); 0 = the generic look.
pub(crate) fn sprite_mode(s: &mcla_formats::rage::ptfx::Sprite, _blend_set: u32) -> f32 {
    match (s.shader.as_str(), s.technique.as_str()) {
        ("mc4_rmptfx_mainlist", "draw_SoftPart_VertLit" | "draw_SoftPart_VertLit_BaseLight") => 4.0,
        ("mc4_rmptfx_mainlist", "draw_SoftPart_TexBlend_VertLit" | "draw_SoftPart_TexBlend_VertLit_Rescale" | "draw_TexBlend_VertLit" | "draw_TexBlend_VertLit_BaseLight") => 6.0,
        ("mc4_rmptfx_default", "unlit_draw") | ("mc4_rmptfx_mainlist", "draw_SoftPart_TexBlend") => 5.0,
        _ => 0.0,
    }
}

/// the texture key of the projected arrows' four textures (`arrow_atlas`)
const ARROW_ATLAS_KEY: u32 = 0x7a77_0c90;

/// `checkpoint_arrows_anim_0..3` (`$/tune/rmptfx/textures`; the last one is stored under its
/// hash, 0xf3b84fdc) one above the other: 128 x 256 DXT1, five levels (each frame keeps its
/// own block rows down to 8 x 4).
fn arrow_atlas(data_dir: &str) -> Option<Texture> {
    use mcla_formats::rage::xenos::TexFormat;
    let dir = Path::new(data_dir).join("tune/rmptfx/textures");
    let mut frames = Vec::new();
    for k in 0..4 {
        let name = format!("checkpoint_arrows_anim_{k}.dds");
        let p = [dir.join(&name), dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(&name)))].into_iter().find(|p| p.is_file())?;
        let t = mcla_formats::dds::parse_dds(&name, &std::fs::read(p).ok()?).ok()?;
        if t.width != 128 || t.height != 64 || !matches!(t.format, TexFormat::Dxt1) {
            return None;
        }
        frames.push(t);
    }
    let mut levels = Vec::new();
    for l in 0..5usize {
        let need = ((128usize >> l) / 4) * ((64usize >> l) / 4) * 8;
        let mut lv = Vec::with_capacity(need * 4);
        for f in &frames {
            lv.extend_from_slice(f.levels.get(l).filter(|d| d.len() >= need)?.get(..need)?);
        }
        levels.push(lv);
    }
    Some(Texture { name: "checkpoint_arrows_anim".into(), width: 128, height: 256, format: TexFormat::Dxt1, levels, fetch: [0; 6] })
}
/// most particles of the effect list's rules alive at once
const MAX_RULE: usize = 5000;
/// most effects playing at once (69 particle props stand in the city, a few in reach; up to four
/// tyre effects a car) - the port's own cap
const MAX_EFFECTS: usize = 160;
/// the effects whose sprites are loaded: the particle props' and the litter's
// (step 19g: + the power-up shots' sparks, powerup_sparks - F-72989)
const PLAYED: [&str; 19] = ["col_", "water_hydrant", "coin", "bang_concrete", "exp_gas", "tree_trunk", "damage_scrape", "water_fountain", "water_rings", "water_waterfall", "amb_steam", "amb_smoke", "wind_debris", "checkpoint_ctf_generic", "light_column", "powerup_sparks", "damage_glassbreak", "wheel_", "veh_"];

/// A vector of an effect's frame into the world: x negated first when the frame is a mirror image.
fn turn(rot: Quat, flip: bool, v: Vec3) -> Vec3 {
    rot * if flip { Vec3::new(-v.x, v.y, v.z) } else { v }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn v3(a: [f32; 4]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

impl Particles {
    pub fn new() -> Particles {
        Particles {
            list: Vec::new(),
            binds: vec![None; KINDS],
            depth_bg: None,
            additive: SHEETS.iter().map(|s| s.3).collect(),
            buf: None,
            runs: Vec::new(),
            rng: 0x9e37_79b9,
            cap: 0,
            fx: None,
            looks: Vec::new(),
            playable: Vec::new(),
            by_hash: HashMap::new(),
            effects: Vec::new(),
            rule_list: Vec::new(),
            frame: 0,
            next_id: 1,
            standin: std::env::var("MCLA_FX_STANDIN").map_or(false, |v| v == "1"),
            hour: 12.0,
            fog_min: [0.0, 0.0, 0.0, 0.0, 1000.0],
            arrow_rt: [0.0, std::f32::consts::PI, 1.0],
            clock: 0.0,
            coll: HashMap::new(),
        }
    }

    pub fn load(&mut self, gpu: &mut Gpu, data_dir: &str) {
        let p = Path::new(data_dir).join("resources/rmptfx/fxlists/MC4.r107");
        let Ok(res) = load_resource_file(&p) else {
            log::warn!("particles: {} missing", p.display());
            return;
        };
        let rd = Rd::new(&res);
        // root+0x08 -> texture dictionary of the effect list
        let Some(doff) = rd.ptr(0x08) else { return };
        let Ok(dict) = Dictionary::parse(&rd, doff) else { return };
        let mut found = 0;
        for (k, (name, _, _, _)) in SHEETS.iter().enumerate() {
            let h = mcla_formats::hash::joaat(name);
            let Some(o) = dict.get(h) else { continue };
            let Ok(t) = Texture::parse(&rd, o) else { continue };
            let key = h ^ 0x7f00_0000;
            let tex = gpu.upload_texture(key, &t);
            self.binds[k] = Some(gpu.texture_bind_group(&tex, &tex, key, key));
            found += 1;
        }
        log::info!("particles: {} of {} effect sprites loaded ({} in the effect list)", found, KINDS, dict.len());

        // the projected arrows' four textures (`F-76837`: `$/tune/rmptfx/textures`
        // checkpoint_arrows_anim_0..3, 128 x 64 DXT1) stacked into one 128 x 256 picture, the
        // second texture of every effect sheet (binding 2; only the checkpoints' low smoke reads it)
        let atlas = arrow_atlas(data_dir).map(|t| gpu.upload_texture_raw(ARROW_ATLAS_KEY, &t));
        if atlas.is_none() {
            log::warn!("particles: the checkpoint arrow textures were not found");
        }
        // the effect rules, and the sprites of those that are played from them
        let fx = match FxList::parse(&res) {
            Ok(fx) => Arc::new(fx),
            Err(e) => {
                log::warn!("particles: effect rules not read: {e}");
                return;
            }
        };
        let mut looks: Vec<Option<Look>> = (0..fx.particle_rules.len()).map(|_| None).collect();
        let mut sheets: HashMap<(String, bool), Option<usize>> = HashMap::new();
        let mut sheet = |this: &mut Particles, gpu: &mut Gpu, name: &str, additive: bool| -> Option<usize> {
            *sheets.entry((name.to_string(), additive)).or_insert_with(|| {
                let bare = name.trim_end_matches(".dds");
                let h = [name.to_string(), bare.to_string(), format!("{bare}.dds")].iter().map(|n| mcla_formats::hash::joaat(n)).find(|h| dict.get(*h).is_some())?;
                let t = Texture::parse(&rd, dict.get(h)?).ok()?;
                // (read as stored, no gamma decoding: the fetch constants of the original's particle
                // draws ask for none - signs 0 on r, g, b in the recorded tyre and checkpoint smoke,
                // where the road's ask for gamma (3); `work/map2/rec`, MAP.md "Tyre smoke")
                let key = h ^ 0x7e00_0000;
                let tex = gpu.upload_texture_raw(key, &t);
                this.binds.push(Some(match &atlas {
                    Some(a) => gpu.texture_bind_group(&tex, a, key, ARROW_ATLAS_KEY),
                    None => gpu.texture_bind_group(&tex, &tex, key, key),
                }));
                this.additive.push(additive);
                Some(this.binds.len() - 1)
            })
        };
        // rules reached from the played effects
        let mut todo: Vec<usize> = fx.effects.iter().enumerate().filter(|(_, e)| PLAYED.iter().any(|p| e.name.starts_with(p))).map(|(i, _)| i).collect();
        let mut seen = vec![false; fx.effects.len()];
        while let Some(ei) = todo.pop() {
            if std::mem::replace(&mut seen[ei], true) {
                continue;
            }
            for ev in &fx.effects[ei].events {
                match &ev.kind {
                    EventKind::Effect { effect: Some(e), .. } => todo.push(*e),
                    EventKind::Emitter { particle: Some(pi), .. } if looks[*pi].is_none() => {
                        let pr = &fx.particle_rules[*pi];
                        let additive = pr.render.blend_set == 7;
                        let mut cards = Vec::new();
                        if let Some(sp) = &pr.sprite {
                            if let Some(s) = sheet(self, gpu, &sp.texture, additive) {
                                cards.push(Card { sheet: s, uv: [0.0, 0.0, 1.0, 1.0], half: [1.0, 1.0] });
                            }
                        } else if let Some(m) = &pr.model {
                            // (a card per drawable: its triangles are drawn by modelfx.rs with its texture)
                            for d in &m.drawables {
                                if crate::modelfx::add_mesh(gpu, *pi, cards.len(), d, &rd, &dict) {
                                    cards.push(Card { sheet: 0, uv: [0.0, 0.0, 1.0, 1.0], half: [1.0, 1.0] });
                                }
                            }
                        }
                        if !cards.is_empty() {
                            looks[*pi] = Some(Look { cards });
                        }
                    }
                    _ => {}
                }
            }
        }
        // an effect can be played when one of its emitters (or of the effects it starts) can be drawn
        fn can(fx: &FxList, looks: &[Option<Look>], ei: usize, depth: u32) -> bool {
            depth < 6
                && fx.effects[ei].events.iter().any(|ev| match &ev.kind {
                    EventKind::Emitter { emit: Some(_), particle: Some(pi), .. } => looks[*pi].is_some(),
                    EventKind::Effect { effect: Some(e), .. } => can(fx, looks, *e, depth + 1),
                    _ => false,
                })
        }
        self.playable = (0..fx.effects.len()).map(|ei| can(&fx, &looks, ei, 0)).collect();
        self.by_hash = fx.effect_hashes.iter().enumerate().map(|(i, h)| (*h, i)).collect();
        log::info!("particles: {} effect / {} emit / {} particle rules; {} effects playable from them with {} rules on {} sheets", fx.effects.len(), fx.emit_rules.len(), fx.particle_rules.len(), self.playable.iter().filter(|p| **p).count(), looks.iter().filter(|l| l.is_some()).count(), self.binds.len() - KINDS);
        self.looks = looks;
        self.fx = Some(fx);
    }

    /// Keep an effect of the disc's list playing at a place: call it every tick while the
    /// place is in reach (an effect not asked for any more stops spawning and is dropped).
    /// `key` tells the places apart, `effect` is joaat(name), `dist` the camera's distance
    /// (the rule's cull and fade distances are applied). False: the rule cannot be played
    /// here - the caller falls back to its stand-in.
    pub fn play(&mut self, key: u64, effect: u32, pos: Vec3, rot: Quat, vel: Vec3, dist: f32) -> bool {
        self.play_ex(key, effect, pos, rot, vel, dist, [1.0; 3], [0.0; 3], 1.0, 250.0, &[])
    }

    /// `play` with the effect instance's colour tint and an opacity, culled only by the rule's
    /// own distance (the checkpoints' flare and light column: 8000 m and none).
    /// `evo`: evolution values by name (`F-38579`, e.g. the checkpoints' "Height", "time").
    pub fn play_tinted(&mut self, key: u64, effect: u32, pos: Vec3, tint: [f32; 3], shadow: [f32; 3], alpha: f32, dist: f32, evo: &[(&str, f32)]) -> bool {
        self.play_ex(key, effect, pos, Quat::IDENTITY, Vec3::ZERO, dist, tint, shadow, alpha, f32::INFINITY, evo)
    }

    #[allow(clippy::too_many_arguments)]
    fn play_ex(&mut self, key: u64, effect: u32, pos: Vec3, rot: Quat, vel: Vec3, dist: f32, tint: [f32; 3], shadow: [f32; 3], alpha: f32, cap: f32, evo_in: &[(&str, f32)]) -> bool {
        self.play_full(key, effect, pos, rot, false, vel, dist, tint, shadow, alpha, cap, evo_in)
    }

    /// `play` following a frame that may be a mirror image (`flip`), with evolution values by
    /// name (the tyres' effects, `tyrefx.rs`).
    #[allow(clippy::too_many_arguments)]
    pub fn play_evo(&mut self, key: u64, effect: u32, pos: Vec3, rot: Quat, flip: bool, vel: Vec3, dist: f32, evo: &[(&str, f32)]) -> bool {
        self.play_full(key, effect, pos, rot, flip, vel, dist, [1.0; 3], [0.0; 3], 1.0, 250.0, evo)
    }

    /// Whether the effect list has an effect rule of that name hash.
    pub fn has_effect(&self, effect: u32) -> bool {
        self.by_hash.contains_key(&effect)
    }

    #[allow(clippy::too_many_arguments)]
    fn play_full(&mut self, key: u64, effect: u32, pos: Vec3, rot: Quat, flip: bool, vel: Vec3, dist: f32, tint: [f32; 3], shadow: [f32; 3], alpha: f32, cap: f32, evo_in: &[(&str, f32)]) -> bool {
        if self.standin {
            return false;
        }
        let Some(fx) = self.fx.as_ref() else { return false };
        let Some(&ei) = self.by_hash.get(&effect) else { return false };
        if !self.playable[ei] {
            return false;
        }
        let rule = &fx.effects[ei];
        let cull = if rule.cull_distance > 0.0 { rule.cull_distance.min(cap) } else { cap.min(1e9) };
        if dist > cull {
            return true;
        }
        let fade = if rule.fade_distance > 0.0 && rule.fade_distance < cull { ((cull - dist) / (cull - rule.fade_distance)).clamp(0.0, 1.0) } else { 1.0 };
        // the instance's evolution values: the caller's by name, and "LOD" from the camera's
        // distance between the rule's LOD near and far (`F-11951` A-16517..A-85242)
        let mut evo = [0.0f32; 4];
        for (k, n) in rule.evo_names.iter().enumerate().take(4) {
            if n == "LOD" {
                evo[k] = if dist <= rule.lod_near { 0.0 } else if dist >= rule.lod_far { 1.0 } else { (dist - rule.lod_near) / (rule.lod_far - rule.lod_near).max(1e-3) };
            } else if let Some((_, v)) = evo_in.iter().find(|(m, _)| *m == n.as_str()) {
                evo[k] = *v;
            }
        }
        if let Some(e) = self.effects.iter_mut().find(|e| e.key == key && e.effect == ei) {
            e.moved += pos - e.pos;
            e.pos = pos;
            e.rot = rot;
            e.flip = flip;
            e.vel = vel;
            e.fade = fade;
            e.seen = self.frame;
            e.tint = tint;
            e.alpha = alpha;
            e.shadow = shadow;
            e.evo = evo;
            return true;
        }
        if self.effects.len() >= MAX_EFFECTS {
            return true;
        }
        let mut rng = Rng(self.rng);
        let (dur, rate) = (rng.range(rule.duration[0], rule.duration[1]).max(0.01), rng.range(rule.time_scalar[0], rule.time_scalar[1]));
        let zr = rng.f();
        self.rng = rng.0;
        let id = self.next_id;
        self.next_id += 1;
        // (for looking at an effect in a screenshot: where in its loop it starts, 0..1)
        let t = std::env::var("MCLA_FX_PHASE").ok().and_then(|v| v.parse::<f32>().ok()).map_or(0.0, |v| v.clamp(0.0, 0.99));
        self.effects.push(Effect { key, id, effect: ei, pos, rot, vel, moved: Vec3::ZERO, t, dur, rate: if rate > 0.0 { rate } else { 1.0 }, loops: 0, done: false, seen: self.frame, fade, em: Vec::new(), kids: Vec::new(), tint, alpha, shadow, evo, arrow: false, zr, oneshot: false, flip });
        true
    }

    /// Start an effect of the disc's list once at a place, moving with `vel`, and let it run out on
    /// its own (a one-shot such as `damage_glassbreak`: the game's `F-44063` + start, the
    /// instance not kept). False: the rule cannot be played here.
    pub fn burst(&mut self, effect: u32, pos: Vec3, vel: Vec3, dist: f32) -> bool {
        let key = 0x4255_5253_0000_0000 | self.next_id as u64;
        if !self.play(key, effect, pos, Quat::IDENTITY, vel, dist) {
            return false;
        }
        if let Some(e) = self.effects.iter_mut().find(|e| e.key == key) {
            e.oneshot = true;
        }
        true
    }

    /// `burst` in a frame, with evolution values by name (a scrape's `damage_scrape_sparks`).
    pub fn burst_evo(&mut self, effect: u32, pos: Vec3, rot: Quat, vel: Vec3, dist: f32, evo: &[(&str, f32)]) -> bool {
        let key = 0x4255_5253_0000_0000 | self.next_id as u64;
        if !self.play_evo(key, effect, pos, rot, false, vel, dist, evo) {
            return false;
        }
        if let Some(e) = self.effects.iter_mut().find(|e| e.key == key) {
            e.oneshot = true;
        }
        true
    }

    /// `play_evo` for an effect that is started once and left to run out (`burst_evo`), asked for
    /// again with the same `key` each frame while its place or evolutions follow something
    /// (`propfx.rs`: the event effects of the props). False: the rule cannot be played here.
    #[allow(clippy::too_many_arguments)]
    pub fn play_oneshot_evo(&mut self, key: u64, effect: u32, pos: Vec3, rot: Quat, vel: Vec3, dist: f32, evo: &[(&str, f32)]) -> bool {
        if !self.play_evo(key, effect, pos, rot, false, vel, dist, evo) {
            return false;
        }
        if let Some(e) = self.effects.iter_mut().find(|e| e.key == key) {
            e.oneshot = true;
        }
        true
    }

    /// Is the effect started under `key` still playing (or its particles alive)?
    pub fn is_alive(&self, key: u64) -> bool {
        self.effects.iter().any(|e| e.key == key)
    }

    /// The polygon the particles of the effect under `key` collide with this frame (the effect
    /// instance's +512, set by the game's update functor before the particle step; none: no collision).
    pub fn set_collision(&mut self, key: u64, poly: Option<Poly>) {
        let Some(id) = self.effects.iter().find(|e| e.key == key).map(|e| e.id) else { return };
        match poly {
            Some(p) => {
                self.coll.insert(id, p);
            }
            None => {
                self.coll.remove(&id);
            }
        }
    }

    /// The checkpoint render setup's EffectPos.w for the effect playing under `key`: the arrows
    /// projected onto its low smoke are on (`F-71936` A-73859).
    pub fn set_arrow_on(&mut self, key: u64, on: bool) {
        for e in self.effects.iter_mut().filter(|e| e.key == key) {
            e.arrow = on;
        }
    }

    /// One step of an effect's timeline (`F-31524`): events whose time lies in the step
    /// start their emitter or effect; at the end of the loop it starts again or ends.
    fn step_effect(&mut self, fx: &FxList, e: &mut Effect, dt: f32, live: bool) {
        let rule = &fx.effects[e.effect];
        if e.em.len() != rule.events.len() {
            e.em = (0..rule.events.len())
                .map(|_| {
                    self.next_id += 1;
                    Emitter { playing: false, t: 0.0, dur: 1.0, tsc: 1.0, zoom: 1.0, carry: 0.0, uid: self.next_id, prev: None }
                })
                .collect();
        }
        if !e.done {
            let prev = e.t * e.dur;
            e.t += dt * e.rate / e.dur;
            let now = e.t * e.dur;
            let mut rng = Rng(self.rng);
            for (i, ev) in rule.events.iter().enumerate() {
                if !live || ev.time < prev || ev.time > now {
                    continue;
                }
                match &ev.kind {
                    EventKind::Emitter { emit: Some(ei), duration_scalar, time_scalar, zoom, .. } => {
                        let em = &mut e.em[i];
                        em.playing = true;
                        em.t = 0.0;
                        em.dur = (fx.emit_rules[*ei].duration * rng.range(duration_scalar[0], duration_scalar[1])).max(0.01);
                        em.tsc = rng.range(time_scalar[0], time_scalar[1]);
                        em.zoom = rng.range(zoom[0], zoom[1]);
                        em.prev = Some(e.pos);
                    }
                    EventKind::Effect { effect: Some(k), domain, .. } if self.playable[*k] && e.kids.len() < 12 => {
                        let kr = &fx.effects[*k];
                        // where: a point of the event's domain, in the parent's frame
                        let at = domain.as_ref().map_or(Vec3::ZERO, |d| d.centre(0.0) + d.point(0.0, &mut || rng.f()));
                        let id = e.id;
                        e.kids.push(Effect { key: 0, id, effect: *k, pos: e.pos + turn(e.rot, e.flip, at), rot: e.rot, vel: e.vel, moved: Vec3::ZERO, t: 0.0, dur: rng.range(kr.duration[0], kr.duration[1]).max(0.01), rate: rng.range(kr.time_scalar[0], kr.time_scalar[1]).max(0.01), loops: 0, done: false, seen: 0, fade: e.fade, em: Vec::new(), kids: Vec::new(), tint: e.tint, alpha: e.alpha, shadow: e.shadow, evo: [0.0; 4], arrow: false, zr: rng.f(), oneshot: false, flip: e.flip });
                    }
                    _ => {}
                }
            }
            self.rng = rng.0;
            if e.t >= 1.0 {
                if rule.num_loops >= 0 && e.loops >= rule.num_loops {
                    e.done = true;
                } else {
                    e.t = 0.0;
                    e.loops += 1;
                }
            }
        }
        for i in 0..e.em.len() {
            if e.em[i].playing {
                self.step_emitter(fx, e, i, dt, live);
            }
        }
        let mut kids = std::mem::take(&mut e.kids);
        for k in kids.iter_mut() {
            k.pos += e.moved;
            k.fade = e.fade;
            k.tint = e.tint;
            k.alpha = e.alpha;
            k.shadow = e.shadow;
            self.step_effect(fx, k, dt, live);
        }
        kids.retain(|k| !(k.done && k.kids.is_empty() && k.em.iter().all(|m| !m.playing)));
        e.kids = kids;
    }

    /// One step of an emitter (`F-69014`, `F-63457`): its clock, how many particles
    /// the step lets out, and their start (`F-70396`, `F-61736`).
    fn step_emitter(&mut self, fx: &FxList, e: &mut Effect, i: usize, dt: f32, live: bool) {
        let EventKind::Emitter { emit: Some(ei), particle: Some(pi), .. } = &fx.effects[e.effect].events[i].kind else {
            e.em[i].playing = false;
            return;
        };
        let (er, pr) = (&fx.emit_rules[*ei], &fx.particle_rules[*pi]);
        let mut rng = Rng(self.rng);
        let em = &mut e.em[i];
        let prev = em.t;
        let t1 = prev + dt * e.rate * em.tsc / em.dur;
        // (the part of the step the emitter still runs in)
        let frac = if t1 > 1.0 { ((1.0 - prev) / (t1 - prev)).clamp(0.0, 1.0) } else { 1.0 };
        em.t = t1.min(1.0);
        let te = em.t;
        let evo_g = fx.effects[e.effect].events[i].evo.as_ref();
        let evo_v = e.evo;
        let ev = |list: u8, prop: u8, base: [f32; 4]| -> [f32; 4] {
            match evo_g {
                Some(g) => g.evolve(list, prop, base, te, &evo_v),
                None => base,
            }
        };
        let ts = match ev(0, 1, er.time_scale.eval(te))[0] {
            x if x > 0.0 => x,
            _ => 1.0,
        };
        let rate = ev(0, 0, er.spawn_rate.eval(te));
        let r = rng.range(rate[0], rate[1]).max(0.0);
        // (`F-63457`: the rate's whole particles of the step - its fraction carried - plus
        // SpawnDistKF's random(x, y) a metre the emitter moved since `prev` (no fraction carried
        // there: `prev` only moves on by the share let out); one-shot emitters only the rate)
        let (cur, prev) = (e.pos, em.prev.unwrap_or(e.pos));
        let mut per = 0.0;
        let total = if er.one_shot {
            em.t = 1.0;
            r
        } else {
            let x = r * frac * dt * ts + em.carry;
            em.carry = x.fract();
            let sd = ev(0, 9, er.spawn_dist.eval(te));
            per = rng.range(sd[0], sd[1]);
            x.trunc() + cur.distance(prev) * per
        };
        let mut n = total as i32;
        let wanted = n;
        if em.t >= 1.0 {
            em.playing = false;
        }
        // (the zoom: the event's x the effect's own ZoomKF / 100 now - instance +500 x emitter
        // +244, on sizes and domains; taken at the particle's birth here)
        let ez = fx.effects[e.effect].zoom.eval(e.t);
        let ez = if fx.effects[e.effect].zoom.keys.is_empty() { 1.0 } else { lerp(ez[0], ez[1], e.zr) * 0.01 };
        let (uid, zoom, pscale) = (em.uid, em.zoom * ez, e.rate * em.tsc * ts);
        self.rng = rng.0;
        // SpawnRateKF z: the most that may be alive
        if rate[2] >= 1.0 {
            n = n.min(rate[2] as i32 - self.rule_list.iter().filter(|p| p.uid == uid).count() as i32);
        }
        // (the remembered place: to here when nothing is let out by distance or some were
        // refused, else on along the path by the share let out)
        e.em[i].prev = Some(if per == 0.0 || wanted > n.max(0) { cur } else if n > 0 && total > 0.0 { prev + (cur - prev) * (n as f32 / total) } else { prev });
        let Some(look) = self.looks[*pi].as_ref().filter(|_| live && n > 0) else { return };
        let ncards = look.cards.len();
        let mut rng = Rng(self.rng);
        for k in 1..=n.min(64) {
            if self.rule_list.len() >= MAX_RULE {
                break;
            }
            // a point of the emitter domain; the velocity domain's point is the velocity, or
            // where the particle heads from its start
            let lf = ev(0, 2, er.spawn_life.eval(te));
            let life = rng.range(lf[0], lf[1]);
            // (the emitter's place for the k-th of the step: along the path from `prev`, k / total of it)
            let along = if total > 0.0 { (k as f32 / total * frac).min(1.0) } else { 1.0 };
            let (mut start, mut at) = (Vec3::ZERO, prev + (cur - prev) * along);
            // (the domains' position and sizes: lists 1 and 2)
            let dom = |l: u8, d: &mcla_formats::rage::ptfx::Domain, rnd: &mut dyn FnMut() -> f32| -> Vec3 {
                let c = ev(l, 0, d.position.eval(te));
                Vec3::new(c[0], c[1], c[2]) * d.scale + d.point_sized(te, ev(l, 2, d.size.eval(te)), ev(l, 3, d.inner_size.eval(te)), rnd)
            };
            if let Some(d) = &er.emitter_domain {
                start = dom(1, d, &mut || rng.f()) * zoom;
                at += if d.world_space { start } else { turn(e.rot, e.flip, start) };
            }
            let mut vel = Vec3::ZERO;
            if let Some(d) = &er.velocity_domain {
                let p = dom(2, d, &mut || rng.f()) * zoom;
                let v = if d.point_relative { p } else { p - start };
                vel = if d.world_space { v } else { turn(e.rot, e.flip, v) };
            }
            let sp = ev(0, 3, er.speed.eval(te));
            let inh = ev(0, 8, er.inherit_vel.eval(te));
            vel = vel * rng.range(sp[0], sp[1]) + e.vel * rng.range(inh[0], inh[1]) * 0.01;
            let sot = ev(0, 4, er.size_ot.eval(te));
            let mut p = RuleParticle {
                rule: *pi as u16,
                card: ((rng.f() * ncards as f32) as usize).min(ncards - 1) as u16,
                uid,
                inst: e.id,
                pos: at,
                vel,
                prev: at,
                spawn: at,
                t: 0.0,
                inv_life: if life > 1e-3 { 1.0 / life } else { 1.0 },
                ts: pscale,
                r: [0.0; 11],
                theta: 0.0,
                spin0: 0.0,
                frame0: 0,
                sort_key: 0.0,
                frame: 0,
                anim: 0.0,
                size_ot: [sot[0] * 0.01, sot[1] * 0.01],
                accel_ot: v3(ev(0, 5, er.accel_ot.eval(te))) * 0.01,
                damp_ot: v3(ev(0, 6, er.damp_ot.eval(te))) * 0.01,
                mw_ot: ev(0, 7, er.matrix_weight_ot.eval(te))[0] * 0.01,
                evo: evo_v,
                zoom,
                rot: e.rot,
                fade: e.fade,
                te,
                tint: e.tint,
                shadow: e.shadow,
                eff: e.effect as u16,
                ev: i as u16,
                flip: e.flip,
                trail: None,
                orient: Default::default(),
                // (`F-61736` A-17264: a share of PercentPhysical of 101; the draw is made only
                // for the rules that can collide, so the other effects' random numbers stay as they were)
                physical: pr.physical_range != 0.0 && (rng.f() * 101.0) as u32 <= pr.percent_physical as u32,
                at_rest: false,
            };
            for r in p.r.iter_mut() {
                *r = rng.f();
            }
            if let Some(s) = &pr.sprite {
                // first frame, first turn (degrees), first spin (turns a second)
                p.frame = (s.frame_start + (rng.f() * (s.frame_end - s.frame_start + 1) as f32) as i32).clamp(0, 255) as u8;
                p.frame0 = p.frame;
                // (InitTheta, InitRotateVel with their evolutions, list 3 ids 6 / 7: `F-70576`, `F-61736`)
                let (th, rv) = (ev(3, 6, s.init_theta.eval(te)), ev(3, 7, s.init_rot_vel.eval(te)));
                p.theta = rng.range(th[0], th[1]).to_radians();
                p.spin0 = rng.range(rv[0], rv[1]) * std::f32::consts::TAU;
                if pr.flags & flag::USE_PARTICLE_TRAILS != 0 {
                    // (n = max(lerp(TrailPropsKF z, w), 3) points, all at the spawn point)
                    let tp = s.trail.eval(te);
                    let n = (lerp(tp[2], tp[3], p.r[10]) as usize).max(3);
                    p.trail = Some(Box::new(Trail { nodes: vec![p.pos; n], f: 0.0, len: lerp(tp[0], tp[1], p.r[10]) }));
                }
            } else if let Some(m) = &pr.model {
                // (its Euler angles, spin axis and speed, scale numbers - modelfx.rs, F-34386)
                p.orient = crate::modelfx::Orient::birth(pr, m, te, ez, sot, &mut || rng.f());
            }
            // spread over the step: the earlier ones have already moved on
            if !er.one_shot {
                let age = (1.0 - k as f32 / n as f32) * dt * pscale;
                p.pos += p.vel * age;
                p.t = (age * p.inv_life).min(0.99);
            }
            p.sort_key = rng.f();
            self.rule_list.push(p);
        }
        self.rng = rng.0;
    }

    /// One step of the particles of the effect list's rules (`F-12752`).
    fn step_rule_particles(&mut self, fx: &FxList, dt: f32, moved: &[(u32, Vec3)], born_from: usize) {
        let mut idx = 0usize;
        let mut rng = Rng(self.rng);
        // (the trails' history moves once a draw of the original's 30 Hz frame: every other step here)
        let shift = self.frame % 2 == 0;
        let coll = &self.coll;
        self.rule_list.retain_mut(|p| {
            let newborn = idx >= born_from;
            idx += 1;
            let pr = &fx.particle_rules[p.rule as usize];
            // (the rule's keyframes with its event's evolutions, list 3 by the particle's birth values:
            // `F-57835` ids 9 / 10 acceleration, 11 / 12 damping, 13 matrix weight, 17 playback, 20 / 21 noise;
            // `F-53755`, `F-28037`)
            let evo_g = fx.effects.get(p.eff as usize).and_then(|r| r.events.get(p.ev as usize)).and_then(|e| e.evo.as_ref());
            let pevo = p.evo;
            let pev = |id: u8, base: [f32; 4], t: f32| -> [f32; 4] {
                match evo_g {
                    Some(g) => g.evolve(3, id, base, t, &pevo),
                    None => base,
                }
            };
            p.prev = p.pos;
            let dt = dt * p.ts;
            let pb = pev(17, pr.playback_rate.eval(p.t), p.t);
            let t = p.t + lerp(pb[0], pb[1], p.r[5]) * p.inv_life * dt;
            if t > 1.0 {
                return false;
            }
            let mut acc = v3(pev(9, pr.accel_min.eval(t), t)).lerp(v3(pev(10, pr.accel_max.eval(t), t)), p.r[3]) * p.accel_ot;
            if pr.flags & flag::WORLD_SPACE_ACCELERATION == 0 {
                acc = turn(p.rot, p.flip, acc);
            }
            if pr.flags & flag::ZOOM_ACCELERATION != 0 {
                acc *= p.zoom;
            }
            let damp = v3(pev(11, pr.damp_min.eval(t), t)).lerp(v3(pev(12, pr.damp_max.eval(t), t)), p.r[4]) * p.damp_ot;
            p.vel += acc * dt;
            // position / velocity noise (`F-92194`, after the acceleration): amplitudes by one
            // random number of the particle at its life, three fresh numbers in -1..1 each time,
            // the restricted axes masked; the direction noise turns the velocity, keeping its speed
            let pn = pev(20, pr.position_noise.eval(t), t);
            let vn = pev(21, pr.velocity_noise.eval(t), t);
            let (pa, va, da) = (lerp(pn[0], pn[1], p.r[9]), lerp(vn[0], vn[1], p.r[9]), lerp(vn[2], vn[3], p.r[9]));
            if pa > 0.0 || va > 0.0 || da > 0.0 {
                let mask = Vec3::new(
                    if pr.flags & flag::RESTRICT_NOISE_X != 0 { 0.0 } else { 1.0 },
                    if pr.flags & flag::RESTRICT_NOISE_Y != 0 { 0.0 } else { 1.0 },
                    if pr.flags & flag::RESTRICT_NOISE_Z != 0 { 0.0 } else { 1.0 },
                );
                let mut r3 = || Vec3::new(rng.f() - 0.5, rng.f() - 0.5, rng.f() - 0.5) * 2.0;
                if pa > 0.0 {
                    p.pos += r3() * pa * dt * mask;
                }
                if va > 0.0 {
                    p.vel += r3() * va * dt * mask;
                }
                if da > 0.0 {
                    let sp = p.vel.length();
                    if sp > 0.001 {
                        let u = p.vel + r3() * da * dt * sp;
                        p.vel = u.normalize_or_zero() * sp;
                    }
                    p.vel *= mask;
                }
            }
            let mw = pev(13, pr.matrix_weight.eval(t), t);
            let mw = (lerp(mw[0], mw[1], p.r[6]) * p.mw_ot).clamp(0.0, 1.0);
            // the share of its effect's movement it follows
            if let Some(m) = moved.iter().find(|m| m.0 == p.inst).filter(|_| !newborn) {
                p.pos += m.1 * mw;
            }
            p.vel *= (Vec3::ONE - damp * dt).max(Vec3::ZERO);
            // (the wind: WindInfluenceKF's share of what the manager's wind function gives at the
            // particle, added to the velocity - and that function is `F-43055`, which gives
            // (0, 0, 0): the original's particles have no wind)
            p.pos += p.vel * dt;
            let mut killed = false;
            // (the effect instance's polygons, `F-99991`: a physical particle that is not at rest, of a rule
            // with PhysicalRange -1 or a range it is within; the polygon is the car's underside for the
            // hydrant, `set_collision`)
            if p.physical && !p.at_rest && (pr.physical_range == -1.0 || pr.physical_range > 0.0) {
                if let (Some(poly), Some(s)) = (coll.get(&p.inst), pr.sprite.as_ref()) {
                    let sz = s.size.eval(t);
                    let radius = (lerp(sz[0], sz[2], p.r[0]) * p.size_ot[0] * p.zoom).max(lerp(sz[1], sz[3], p.r[0]) * p.size_ot[1] * p.zoom);
                    let sp = p.vel.length();
                    if sp > 0.0 && poly.n.dot(p.vel / sp) < 0.0 {
                        if let Some(depth) = poly.depth(p.pos, p.vel, dt, radius) {
                            let dm = pr.vel_dmp.eval(t);
                            let dmp = lerp(dm[0], dm[1], p.r[4]);
                            let iv = pr.imp_var.eval(t);
                            let imp = lerp(iv[0], iv[1], p.r[5]);
                            let mut n = poly.n;
                            if imp != 0.0 {
                                let mut r = Vec3::new(rng.f() * 2.0 * imp - imp, rng.f() * 2.0 * imp - imp, rng.f() * 2.0 * imp - imp);
                                r -= n * (2.0 * r.dot(n));
                                n = (n + r).normalize_or_zero();
                            }
                            p.vel = (p.vel - n * (2.0 * p.vel.dot(n))) * (1.0 - dmp * 0.01);
                            if p.vel.length_squared() < pr.stop_vel * pr.stop_vel {
                                p.vel = Vec3::ZERO;
                                p.at_rest = true;
                            }
                            if pr.flags & 1 != 0 && (rng.f() * 101.0) as u32 <= pr.percent_kill as u32 {
                                killed = true;
                            }
                            p.pos += poly.n * depth;
                        }
                    }
                }
            }
            if let Some(tr) = p.trail.as_mut().filter(|_| shift) {
                // (`F-56257`: a point every length / count; at most one shift a draw)
                let spacing = tr.len / tr.nodes.len() as f32;
                let last = *tr.nodes.last().unwrap();
                let dist = p.pos.distance(last);
                if dist >= spacing {
                    tr.nodes.rotate_left(1);
                    *tr.nodes.last_mut().unwrap() = p.pos;
                    tr.f = 0.0;
                } else {
                    tr.f = if spacing > 0.0 { dist / spacing } else { 0.0 };
                }
            }
            if let Some(s) = &pr.sprite {
                let rv = s.rot_vel.eval(t);
                p.theta += (p.spin0 + lerp(rv[0], rv[1], p.r[7]) * std::f32::consts::TAU) * dt;
                if pr.flags & flag::ANIMATE_TEXTURES != 0 {
                    let rate = if pr.flags & flag::SCALE_TEX_ANIM_RATE_BY_LIFE != 0 {
                        (s.anim_end + 1) as f32 * p.inv_life
                    } else {
                        let ar = s.tex_anim_rate.eval(if pr.flags & flag::KEY_TEX_ANIM_RATE_ON_EMIT_TIME != 0 { p.te } else { t });
                        rng.range(ar[0], ar[1])
                    };
                    p.anim += rate * dt;
                    if p.anim >= 1.0 {
                        p.anim = (p.anim - 1.0).min(1.0);
                        let (a, b) = (s.frame_start, s.anim_end);
                        let next = if pr.flags & flag::ANIMATE_RANDOM != 0 {
                            a + (rng.f() * (b - a + 1) as f32) as i32
                        } else if p.frame as i32 + 1 > b {
                            if pr.flags & flag::HOLD_LAST_TEX_FRAME != 0 {
                                b
                            } else {
                                a
                            }
                        } else {
                            p.frame as i32 + 1
                        };
                        // (F-12752: A takes B, B the next frame)
                        p.frame0 = p.frame;
                        p.frame = next.clamp(0, 255) as u8;
                    }
                }
            } else if let Some(m) = &pr.model {
                p.orient.step(pr, m, t, p.r[7], p.r[4], dt);
            }
            p.t = if killed { 1.0 } else { t };
            true
        });
        self.rng = rng.0;
    }

    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x % 100000) as f32 / 100000.0
    }

    fn push(&mut self, p: Particle) {
        if self.list.len() < MAX {
            self.list.push(p);
        }
    }

    /// Tyre smoke puff at a wheel contact. `amount` 0..1.
    pub fn tyre_smoke(&mut self, pos: Vec3, car_vel: Vec3, amount: f32) {
        let j = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5);
        let frame = (self.rand() * 4.0) as u32;
        let life = 1.1 + self.rand() * 0.9;
        let spin = (self.rand() - 0.5) * 1.2;
        let rot = self.rand() * 6.28;
        let p = Particle { kind: Kind::Smoke, pos: pos + Vec3::Y * 0.12 + j * 0.2, vel: car_vel * 0.18 + j * 0.9 + Vec3::Y * (0.5 + self.rand() * 0.5), age: 0.0, life, size0: 0.28, size1: 1.6 + amount * 1.3, color: [0.92, 0.92, 0.94, 0.26 * amount.clamp(0.2, 1.0)], rot, spin, frame, gravity: -0.25, drag: 1.2 };
        self.push(p);
    }

    /// Dust thrown up by a wheel on loose ground (sand, gravel, dry grass).
    pub fn dust(&mut self, pos: Vec3, car_vel: Vec3, amount: f32, color: [f32; 3]) {
        let j = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5);
        let frame = (self.rand() * 4.0) as u32;
        let life = 0.9 + self.rand() * 0.8;
        let spin = (self.rand() - 0.5) * 1.0;
        let rot = self.rand() * 6.28;
        let p = Particle { kind: Kind::Smoke, pos: pos + Vec3::Y * 0.15 + j * 0.3, vel: car_vel * 0.3 + j * 1.2 + Vec3::Y * (0.8 + self.rand() * 0.8), age: 0.0, life, size0: 0.35, size1: 1.7 + amount * 1.6, color: [color[0], color[1], color[2], 0.3 * amount.clamp(0.25, 1.0)], rot, spin, frame, gravity: 0.6, drag: 1.0 };
        self.push(p);
    }

    /// Coloured flare smoke rising from a checkpoint.
    pub fn flare_smoke(&mut self, pos: Vec3, color: [f32; 3], radius: f32) {
        let a = self.rand() * 6.28;
        let r = self.rand().sqrt() * radius;
        let j = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
        let frame = (self.rand() * 4.0) as u32;
        let life = 2.6 + self.rand() * 1.6;
        let spin = (self.rand() - 0.5) * 0.8;
        let rot = self.rand() * 6.28;
        let p = Particle { kind: Kind::Smoke, pos: pos + j + Vec3::Y * 0.2, vel: Vec3::new((self.rand() - 0.5) * 0.6, 2.6 + self.rand() * 1.8, (self.rand() - 0.5) * 0.6), age: 0.0, life, size0: 0.7, size1: 3.6, color: [color[0] * 1.5, color[1] * 1.5, color[2] * 1.5, 0.34], rot, spin, frame, gravity: -0.4, drag: 0.35 };
        self.push(p);
        if self.rand() < 0.35 {
            // the flare itself
            let g = Particle { kind: Kind::Glow, pos: pos + j * 0.3 + Vec3::Y * 0.4, vel: Vec3::Y * 0.5, age: 0.0, life: 0.5, size0: 1.6, size1: 2.4, color: [color[0], color[1], color[2], 0.5], rot: 0.0, spin: 0.0, frame: 0, gravity: 0.0, drag: 0.0 };
            self.push(g);
        }
    }

    /// Dust / spray kicked up when driving in the wet.
    pub fn spray(&mut self, pos: Vec3, car_vel: Vec3, amount: f32) {
        let j = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5);
        let frame = (self.rand() * 4.0) as u32;
        let rot = self.rand() * 6.28;
        let p = Particle { kind: Kind::Splash, pos: pos + Vec3::Y * 0.15, vel: car_vel * 0.35 + j * 1.5 + Vec3::Y * (1.2 + self.rand()), age: 0.0, life: 0.55, size0: 0.25, size1: 1.0, color: [0.8, 0.84, 0.9, 0.35 * amount], rot, spin: 0.0, frame, gravity: 4.0, drag: 0.8 };
        self.push(p);
    }

    /// A fountain's jet (the particle props, see `props::emitter_kind`): water thrown up to
    /// about `height` metres, falling back.
    pub fn fountain(&mut self, pos: Vec3, height: f32) {
        let j = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5);
        let frame = (self.rand() * 4.0) as u32;
        let rot = self.rand() * 6.28;
        let g = 6.0;
        let up = (2.0 * g * height.max(0.05)).sqrt() * (0.85 + 0.3 * self.rand());
        let life = (2.0 * up / g).clamp(0.3, 2.5);
        let p = Particle { kind: Kind::Splash, pos: pos + j * 0.15, vel: j * (0.6 + 0.25 * height) + Vec3::Y * up, age: 0.0, life, size0: 0.14 + 0.04 * height, size1: 0.3 + 0.12 * height, color: [0.6, 0.65, 0.7, 0.2], rot, spin: 0.0, frame, gravity: g, drag: 0.15 };
        self.push(p);
    }

    /// Water coming down a fall.
    pub fn waterfall(&mut self, pos: Vec3) {
        let j = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5);
        let frame = (self.rand() * 4.0) as u32;
        let rot = self.rand() * 6.28;
        let p = Particle { kind: Kind::Splash, pos: pos + j * 0.8, vel: j * 0.5 - Vec3::Y * 0.5, age: 0.0, life: 1.1, size0: 0.4, size1: 1.3, color: [0.85, 0.9, 0.95, 0.35], rot, spin: 0.0, frame, gravity: 5.0, drag: 0.2 };
        self.push(p);
    }

    /// Steam from a grate (`size` 1) or the smoke of a stack (`size` 3).
    pub fn steam(&mut self, pos: Vec3, size: f32) {
        let j = Vec3::new(self.rand() - 0.5, 0.0, self.rand() - 0.5);
        let frame = (self.rand() * 4.0) as u32;
        let life = (2.2 + self.rand() * 1.4) * size.sqrt();
        let spin = (self.rand() - 0.5) * 0.6;
        let rot = self.rand() * 6.28;
        let grey = if size > 2.0 { 0.55 } else { 0.92 };
        let p = Particle { kind: Kind::Smoke, pos: pos + j * 0.3 * size, vel: j * 0.4 + Vec3::Y * (0.9 + self.rand() * 0.6) * size.sqrt(), age: 0.0, life, size0: 0.35 * size, size1: 1.8 * size, color: [grey, grey, grey, 0.22], rot, spin, frame, gravity: -0.3, drag: 0.5 };
        self.push(p);
    }

    /// Shower of sparks from a scrape or crash.
    pub fn sparks(&mut self, pos: Vec3, dir: Vec3, count: usize) {
        for _ in 0..count {
            let j = Vec3::new(self.rand() - 0.5, self.rand() * 0.8, self.rand() - 0.5);
            let v = dir * (3.0 + self.rand() * 7.0) + j * 6.0;
            let life = 0.35 + self.rand() * 0.5;
            self.push(Particle { kind: Kind::Spark, pos, vel: v, age: 0.0, life, size0: 0.11, size1: 0.03, color: [1.8, 1.1, 0.45, 1.0], rot: 0.0, spin: 0.0, frame: 0, gravity: 9.0, drag: 0.6 });
        }
        self.push(Particle { kind: Kind::Glow, pos, vel: Vec3::ZERO, age: 0.0, life: 0.08, size0: 0.8, size1: 0.2, color: [1.6, 1.0, 0.5, 0.6], rot: 0.0, spin: 0.0, frame: 0, gravity: 0.0, drag: 0.0 });
    }

    pub fn update(&mut self, dt: f32) {
        for p in self.list.iter_mut() {
            p.age += dt;
            p.vel.y -= p.gravity * dt;
            p.vel *= (1.0 - p.drag * dt).max(0.0);
            p.pos += p.vel * dt;
            p.rot += p.spin * dt;
        }
        self.list.retain(|p| p.age < p.life);

        // the effects played from the disc's rules
        self.frame += 1;
        self.clock += dt;
        let Some(fx) = self.fx.clone() else { return };
        let mut effects = std::mem::take(&mut self.effects);
        let mut moved: Vec<(u32, Vec3)> = Vec::new();
        let frame = self.frame;
        // (the particles let out from here on are born at their emitter's new place: they do not follow its
        // movement of this step as well - else a moving car's flames start a step's travel ahead of the tips)
        let born_from = self.rule_list.len();
        for e in effects.iter_mut() {
            // (asked for in the last ticks: it runs; not any more: what it has let out lives on)
            let live = frame - e.seen <= 3 || e.oneshot;
            self.step_effect(&fx, e, dt, live);
            if e.moved != Vec3::ZERO {
                moved.push((e.id, std::mem::take(&mut e.moved)));
            }
        }
        effects.retain(|e| frame - e.seen <= 3 || (e.oneshot && !(e.done && e.kids.is_empty() && e.em.iter().all(|m| !m.playing))));
        self.effects = effects;
        if !self.coll.is_empty() {
            let alive: Vec<u32> = self.effects.iter().map(|e| e.id).collect();
            self.coll.retain(|id, _| alive.contains(id));
        }
        self.step_rule_particles(&fx, dt, &moved, born_from);
        if self.frame % 60 == 0 && std::env::var("MCLA_FX_LOG").is_ok() {
            let names: Vec<String> = self.effects.iter().map(|e| format!("{} t {:.2} kids {} em {}", fx.effects[e.effect].name, e.t, e.kids.len(), e.em.iter().filter(|m| m.playing).count())).collect();
            log::info!("fx: dt {:.4} effects {} particles {} first {:?}: {:?}", dt, self.effects.len(), self.rule_list.len(), self.rule_list.first().map(|p| (p.pos, p.t, p.ts)), names);
            let mut per: HashMap<&str, usize> = HashMap::new();
            for p in &self.rule_list {
                *per.entry(fx.particle_rules[p.rule as usize].name.as_str()).or_default() += 1;
            }
            log::info!("fx: particles by rule {:?}", per);
            let mut by: HashMap<(u32, &str), usize> = HashMap::new();
            for p in &self.rule_list {
                *by.entry((p.inst, fx.particle_rules[p.rule as usize].name.as_str())).or_default() += 1;
            }
            let at: HashMap<u32, Vec3> = self.effects.iter().map(|e| (e.id, e.pos)).collect();
            let mut v: Vec<_> = by.into_iter().map(|((i, n), c)| (i, n, c, at.get(&i).copied())).collect();
            v.sort_by_key(|x| (x.0, x.1));
            log::info!("fx: particles by instance {:?}", v);
        }
    }

    pub fn count(&self) -> usize {
        self.list.len() + self.rule_list.len()
    }

    /// Build the instance buffer (sorted by sheet, far to near inside a sheet).
    pub fn prepare(&mut self, gpu: &Gpu, cam: Vec3) {
        self.depth_bg = Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle depth"),
            layout: &gpu.particle_depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&gpu.depth) }],
        }));
        self.runs.clear();
        crate::modelfx::begin();
        if self.list.is_empty() && self.rule_list.is_empty() {
            return;
        }
        // (sheet, draw rank, key - larger first within a rank, the instance): the port's own
        // particles first by sheet, farthest first; then the effect list's as the original's pass
        // draws them (draw_order_rule.md): instances farthest first, in each its emitter events
        // in order - one draw each -, in a draw the particles in list order (newest first) or,
        // with the rule's sort bit, by distance or their random key, largest first
        let mut items: Vec<(u32, u64, f32, PInst)> = Vec::with_capacity(self.list.len() + self.rule_list.len());
        for p in &self.list {
            let k = p.kind as usize;
            let (_, cols, rows, _) = SHEETS[k];
            let t = (p.age / p.life).clamp(0.0, 1.0);
            let size = p.size0 + (p.size1 - p.size0) * t;
            // fade in fast, out slowly
            let fade = (t * 8.0).min(1.0) * (1.0 - t).powf(if p.kind == Kind::Smoke { 1.4 } else { 0.7 });
            // flames run through their animation frames
            let frame = if p.kind == Kind::Flame { p.frame + (t * 12.0) as u32 } else { p.frame } % (cols * rows);
            let (fx, fy) = (frame % cols, frame / cols);
            let (du, dv) = (1.0 / cols as f32, 1.0 / rows as f32);
            // sparks are streaks along their velocity (screen-space approximation: stretch)
            let (sx, sy, rot) = if p.kind == Kind::Spark {
                let v = p.vel;
                let ang = v.y.atan2((v.x * v.x + v.z * v.z).sqrt().max(0.01));
                (0.35, 1.0 + v.length() * 0.12, std::f32::consts::FRAC_PI_2 - ang)
            } else {
                (1.0, 1.0, p.rot)
            };
            items.push((k as u32, k as u64, p.pos.distance_squared(cam), PInst { pos_size: [p.pos.x, p.pos.y, p.pos.z, size], color: [p.color[0], p.color[1], p.color[2], p.color[3] * fade], uv: [fx as f32 * du, fy as f32 * dv, du, dv], misc: [rot, sx, sy, if matches!(p.kind, Kind::Smoke | Kind::Splash) { 1.0 } else { 0.0 }], extra: [0.0; 4], arrow_a: [0.0; 4], arrow_b: [0.0; 4], arrow_c: [0.0; 4], arrow_d: [0.0; 4], uv2: [0.0; 4], quad_u: [0.0; 4], quad_v: [0.0; 4], base_a: [0.0; 4], base_b: [0.0; 4] }));
        }
        if let Some(fx) = self.fx.as_ref() {
            // (every effect instance's opacity and evolution values, by its id)
            let mut inst: HashMap<u32, (f32, [f32; 4])> = HashMap::new();
            // (and where it stands, with its render setup's EffectPos.w)
            let mut inst_at: HashMap<u32, (Vec3, bool)> = HashMap::new();
            // (and its cull sphere's centre offset, turned with it: `F-66872`)
            let mut inst_cull: HashMap<u32, Vec3> = HashMap::new();
            for e in &self.effects {
                inst.insert(e.id, (e.alpha, e.evo));
                inst_at.insert(e.id, (e.pos, e.arrow));
                inst_cull.insert(e.id, turn(e.rot, e.flip, Vec3::from(fx.effects[e.effect].cull_offset)));
            }
            // the emitters' clocks (DrawDirectional's keys on the emit time)
            let mut em_t: HashMap<u32, f32> = HashMap::new();
            fn clocks(e: &Effect, m: &mut HashMap<u32, f32>) {
                for em in &e.em {
                    m.insert(em.uid, em.t);
                }
                for k in &e.kids {
                    clocks(k, m);
                }
            }
            for e in &self.effects {
                clocks(e, &mut em_t);
            }
            // the `_BaseLight` programs' height c94.w (`draw_order_rule.md`): the pass draws the effect
            // instances farthest first (`F-64135`, key |camera - instance|^2; children are instances
            // of their own), in each the events in order, only emitters with live particles; an
            // emitter's light callback binds the vertex block (the draw sees the value then held) and
            // then stores its box's min.y (`F-75930`, `F-33282`). The callback acts at night
            // for every emitter, by day only for the checkpoints' (their render setup); the pass starts
            // with the time-of-day accumulator's w, never a height (recorded -4.3e8: no base light)
            let mut base_w: HashMap<u32, f32> = HashMap::new();
            // (each emitter's place in the pass: its draw's rank)
            let mut em_rank: HashMap<u32, u64> = HashMap::new();
            {
                const RESET_W: f32 = -4.3e8;
                let mut live: HashMap<u32, usize> = HashMap::new();
                for p in &self.rule_list {
                    *live.entry(p.uid).or_default() += 1;
                }
                fn flat<'a>(e: &'a Effect, cp: bool, fx: &FxList, out: &mut Vec<(&'a Effect, bool)>) {
                    // (the checkpoints' effects - checkpoints.rs FLARE / COLUMN - and what they start)
                    let n = fx.effects[e.effect].name.as_str();
                    let cp = cp || n == "checkpoint_ctf_generic" || n == "light_column";
                    out.push((e, cp));
                    for k in &e.kids {
                        flat(k, cp, fx, out);
                    }
                }
                let mut order: Vec<(&Effect, bool)> = Vec::new();
                for e in &self.effects {
                    flat(e, false, &fx, &mut order);
                }
                order.sort_by(|a, b| b.0.pos.distance_squared(cam).partial_cmp(&a.0.pos.distance_squared(cam)).unwrap_or(std::cmp::Ordering::Equal));
                // (SortEvents would take the emitters farthest first; no effect on the disc sets it)
                for (e, _) in &order {
                    for em in &e.em {
                        let n = em_rank.len() as u64;
                        em_rank.insert(em.uid, (1 << 32) + n);
                    }
                }
                let night = self.hour > 18.0 || self.hour < 6.0;
                let (mut w_cpu, mut w_gpu) = (RESET_W, RESET_W);
                for (e, cp) in &order {
                    let rule = &fx.effects[e.effect];
                    let ez = rule.zoom.eval(e.t);
                    let ez = if rule.zoom.keys.is_empty() { 1.0 } else { lerp(ez[0], ez[1], e.zr) * 0.01 };
                    for em in &e.em {
                        if live.get(&em.uid).copied().unwrap_or(0) == 0 {
                            continue;
                        }
                        if night || *cp {
                            w_gpu = w_cpu;
                            // (the box: centre = the instance's matrix x CullSphere x s, radius |CullRadius x s|
                            // or 1; s = the event's zoom x the instance's - `F-66872`)
                            let sc = em.zoom * ez;
                            let c = e.pos + turn(e.rot, e.flip, Vec3::from(rule.cull_offset)) * sc;
                            let r = if rule.cull_radius > 0.0 { (rule.cull_radius * sc).abs() } else { 1.0 };
                            w_cpu = c.y - r;
                        }
                        base_w.insert(em.uid, w_gpu);
                    }
                }
            }
            for (pi, p) in self.rule_list.iter().enumerate() {
                let pr = &fx.particle_rules[p.rule as usize];
                let rank = em_rank.get(&p.uid).copied().unwrap_or(u64::MAX);
                let skey = if pr.flags & flag::SORT_PARTICLES == 0 { pi as f32 } else if pr.flags & flag::SORT_RANDOM != 0 { p.sort_key } else { p.pos.distance_squared(cam) };
                let (ialpha, _) = inst.get(&p.inst).copied().unwrap_or((1.0, [0.0; 4]));
                let evo_g = fx.effects.get(p.eff as usize).and_then(|r| r.events.get(p.ev as usize)).and_then(|e| e.evo.as_ref());
                // (list 3 by the particle's birth copy of the evolution values, as the step)
                let pev = |id: u8, base: [f32; 4]| -> [f32; 4] {
                    match evo_g {
                        Some(g) => g.evolve(3, id, base, p.t, &p.evo),
                        None => base,
                    }
                };
                let Some(card) = self.looks[p.rule as usize].as_ref().and_then(|l| l.cards.get(p.card as usize)) else { continue };
                // colour between ColorKF and ColorMaxKF, alpha between AlphaKF's x and y
                // (`F-66151`: ColorKF; between it and ColorMaxKF by a random number of the
                // particle only with the rule's random-colour bit)
                let mut c = v3(pev(0, pr.color.eval(p.t)));
                if pr.flags & if pr.sprite.is_some() { flag::RANDOM_COLOR } else { flag::MODEL_RANDOM_COLOR } != 0 {
                    c = c.lerp(v3(pev(1, pr.color_max.eval(p.t))), p.r[1]);
                }
                if pr.flags & 8 != 0 {
                    c *= Vec3::from(p.tint);
                }
                // (the vertex holds it as 8-bit A, R, G, B: clamped, x 255, truncated - `F-44647`)
                let c_raw = c;
                let c = (c.clamp(Vec3::ZERO, Vec3::ONE) * 255.0).floor() / 255.0;
                // (`mc4_rmptfx_column`: turned about the up axis only, fading to its ends - the
                // particle shader's mode 2)
                let column = pr.sprite.as_ref().map_or(false, |s| s.shader == "mc4_rmptfx_column");
                // (`mc4_rmptfx_projected`, PSCheckpoint: not lit by the scene - the shadow colour
                // under the colour by the texture's alpha, the colour doubled at night: the
                // particle shader's mode 3)
                let projected = pr.sprite.as_ref().map_or(false, |s| s.shader == "mc4_rmptfx_projected");
                let night = (4.0 * ((self.hour / 12.0 - 1.0).abs() - 0.4)).clamp(0.0, 1.0);
                let a = pev(19, pr.alpha.eval(p.t));
                let alpha = ((lerp(a[0], a[1], p.r[2]) * p.fade * ialpha).clamp(0.0, 1.0) * 255.0).floor() / 255.0;
                if alpha <= 0.002 {
                    continue;
                }
                let (w, h, uv, mut rot);
                let mut quad: Option<(Vec3, Vec3, Vec3)> = None;
                let mut uv2 = [0.0f32; 4];
                if let Some(s) = &pr.sprite {
                    // SizeKF: half width x..z, half height y..w, one random number for both
                    let sz = pev(4, s.size.eval(p.t));
                    w = lerp(sz[0], sz[2], p.r[0]) * p.size_ot[0] * p.zoom;
                    h = lerp(sz[1], sz[3], p.r[0]) * p.size_ot[1] * p.zoom;
                    let f = p.frame as u32 % (s.tiles_x * s.tiles_y);
                    let (du, dv) = (1.0 / s.tiles_x as f32, 1.0 / s.tiles_y as f32);
                    uv = [(f % s.tiles_x) as f32 * du, (f / s.tiles_x) as f32 * dv, du, dv];
                    let f0 = p.frame0 as u32 % (s.tiles_x * s.tiles_y);
                    // (w: the soft fade's factor k of sat(k d^2) - technique "draw" of mc4_rmptfx_projected
                    // and mc4_rmptfx_default's PS_Draw 0.1, ps_main_SoftPart_VertLit and
                    // ps_main_SoftPart_TexBlend_VertLit and ps_main_SoftPart_TexBlend 4; 0 none:
                    // ps_main_TexBlend_VertLit)
                    let soft = match sprite_mode(s, pr.render.blend_set) {
                        _ if projected && s.technique == "draw" => 0.1,
                        _ if !s.technique.starts_with("draw_SoftPart") && s.shader == "mc4_rmptfx_mainlist" => 0.0,
                        4.0 | 6.0 | 5.0 if s.shader == "mc4_rmptfx_mainlist" => 4.0,
                        5.0 => 0.1,
                        _ => 0.0,
                    };
                    uv2 = [(f0 % s.tiles_x) as f32 * du, (f0 / s.tiles_x) as f32 * dv, p.anim, soft];
                    rot = p.theta;
                    if pr.flags & flag::DRAW_DIRECTIONAL != 0 {
                        // DrawDirectional (`F-35603`): the quad's corners from the particle -
                        // along d (its last step, else its velocity, else up), sideways along s =
                        // d x (the way to the camera, or the rule's raw axis with AlignToAxis)
                        let mut d = (p.pos - p.prev).normalize_or_zero();
                        if d == Vec3::ZERO {
                            d = p.vel.normalize_or_zero();
                        }
                        if d == Vec3::ZERO {
                            d = Vec3::Y;
                        }
                        let e = if pr.flags & flag::ALIGN_TO_AXIS != 0 { s.axis } else { cam - p.pos };
                        let mut sd = d.cross(e).normalize_or_zero();
                        if pr.flags & flag::USE_DIRECTIONAL_IDENTITY != 0 {
                            (sd, d) = (Vec3::X, Vec3::Y);
                        }
                        let et = em_t.get(&p.uid).copied().unwrap_or(p.te);
                        let dk = s.directional.eval(if pr.flags & flag::KEY_DIRECTIONAL_ON_EMIT_TIME != 0 { et } else { p.t });
                        let dv = s.directional_vel.eval(if pr.flags & flag::KEY_DIRECTIONAL_VEL_ON_EMIT_TIME != 0 { et } else { p.t });
                        let speed = p.vel.length();
                        let k = [h * dk[0] + dv[0] * speed, h * dk[1] + dv[1] * speed, w * dk[2] + dv[2] * speed, w * dk[3] + dv[3] * speed];
                        let front = d * k[0];
                        let mut back = -d * k[1];
                        let (left, right) = (-sd * k[2], sd * k[3]);
                        if pr.flags & flag::ANCHOR_TO_SPAWN_POS != 0 {
                            let a = p.spawn - p.pos;
                            if back.length() > a.length() {
                                back = if a.length() > 0.0 { a } else { back * 0.01 };
                            }
                        }
                        // (the recorded quad: the front edge takes frame A's top, the left its u0)
                        quad = Some((p.pos + (front + back) * 0.5 + (left + right) * 0.5, (front - back) * 0.5, (right - left) * 0.5));
                        if self.frame % 60 == 0 && std::env::var("MCLA_FX_LOG").is_ok() {
                            log::info!("fx quad {}: front {:?} back {:?} left {:?} right {:?} (pos {:?}) colour {:?} alpha {:.3}", pr.name, p.pos + front, p.pos + back, p.pos + left, p.pos + right, p.pos, c, alpha);
                        }
                    }
                } else if let Some(m) = &pr.model {
                    // (its drawable's triangles, turned and scaled; gForcedColor the colour and alpha
                    // as floats; the lights taken at its effect's cull-sphere centre - modelfx.rs)
                    let at = inst_at.get(&p.inst).map_or(p.pos, |e| e.0) + inst_cull.get(&p.inst).copied().unwrap_or(Vec3::ZERO) * p.zoom;
                    let tem = em_t.get(&p.uid).copied().unwrap_or(p.te);
                    let a_raw = (lerp(a[0], a[1], p.r[2]) * p.fade * ialpha).clamp(0.0, 1.0);
                    let world = p.orient.world(pr.flags, m, p.t, tem, p.pos, p.rot);
                    crate::modelfx::push(p.rule, p.card, world, [c_raw.x, c_raw.y, c_raw.z, a_raw], at, pr.render.lighting_mode);
                    continue;
                } else {
                    continue;
                }
                // (a trail's strip takes the half width only - the height is unused, `trail_collision_rule.md`)
                if !(w > 1e-4 && (h > 1e-4 || p.trail.is_some())) {
                    continue;
                }
                let (centre, quad_u, quad_v) = match quad {
                    Some((c, u, v)) => (c, [u.x, u.y, u.z, 1.0], [v.x, v.y, v.z, 0.0]),
                    None => (p.pos, [0.0; 4], [0.0; 4]),
                };
                let mode = pr.sprite.as_ref().map_or(0.0, |s| sprite_mode(s, pr.render.blend_set));
                let lit = if projected { 3.0 } else if column { 2.0 } else if mode > 0.0 { mode } else if self.additive[card.sheet] { 0.0 } else { 1.0 };
                let mut extra = if projected { [p.shadow[0], p.shadow[1], p.shadow[2], (2.0 * night).max(1.0)] } else { [0.0; 4] };
                let mut normal_arc = 0.0;
                let (mut base_a, mut base_b) = ([0.0f32; 4], [0.0f32; 4]);
                if let (4.0 | 6.0, Some(s)) = (mode, pr.sprite.as_ref()) {
                    // (vs_main_SoftPart_VertLit's variables, the effect file's defaults where the rule has none)
                    let var = |n: &str, d: f32| s.vars.iter().find(|(k, _)| k == n).map_or(d, |(_, v)| v[0]);
                    extra = [var("gDiffuse", 1.0), var("gAmbient", 1.0), var("gAmbientDayControl", 1.0), var("gTranslucency", 0.0)];
                    normal_arc = var("gNormalArc", 0.0);
                    if s.technique.ends_with("_BaseLight") {
                        // (BaseLightColor c98, gFallOff c97, BaseLightDir c82; the height c94.w: `base_w`)
                        let v4 = |n: &str, d: [f32; 4]| s.vars.iter().find(|(k, _)| k == n).map_or(d, |(_, v)| *v);
                        let (bc, bd) = (v4("BaseLightColor", [0.0; 4]), v4("BaseLightDir", [0.0, 1.0, 0.0, 0.0]));
                        base_a = [bc[0], bc[1], bc[2], var("gFallOff", 0.0)];
                        base_b = [bd[0], bd[1], bd[2], base_w.get(&p.uid).copied().unwrap_or(-4.3e8)];
                    }
                }
                // (the technique "draw" of `mc4_rmptfx_projected` - the low smoke - maps the arrows'
                // target round its effect; "draw_noarrow" - the plume - does not)
                let (mut arrow_a, mut arrow_b, mut arrow_c, mut arrow_d) = ([0.0f32; 4], [0.0f32; 4], [0.0f32; 4], [0.0f32; 4]);
                if let Some(s) = pr.sprite.as_ref().filter(|s| projected && s.technique == "draw") {
                    // (the rule's shader variables; missing ones take the effect file's defaults)
                    let var = |n: &str, d: [f32; 4]| s.vars.iter().find(|(k, _)| k == n).map_or(d, |(_, v)| *v);
                    let off = var("ProjMapOffset", [0.0; 4]);
                    let bounds = var("ProjMapBounds", [20.0, 20.0, 0.0, 0.0]);
                    let tint = var("ArrowColorTint", [1.0, 1.0, 1.0, 0.0]);
                    let boost = var("NightArrowBoost", [0.0; 4])[0];
                    let tile = var("ProjMapTile", [1.0; 4])[0];
                    let rate = var("ProjMapRate", [1.0; 4])[0];
                    let (at, on) = inst_at.get(&p.inst).copied().unwrap_or((p.pos, false));
                    arrow_a = [at.x + off[0], at.y + off[1], at.z + off[2], if on { 1.0 } else { 0.0 }];
                    arrow_b = [bounds[0], bounds[1], self.arrow_rt[0], self.arrow_rt[1]];
                    arrow_c = [tint[0], tint[1], tint[2], boost];
                    arrow_d = [self.arrow_rt[2], tile, self.clock * rate, night];
                }
                if mode == 4.0 || mode == 6.0 {
                    // (x: gNormalArc; arrow_b xyz: the light picker's position - the effect's cull
                    // sphere centre, its place + the rule's offset x the event's and the effect's
                    // zoom, from the frame before; checkpoint_ctf_generic: 68 m straight up)
                    arrow_a[0] = normal_arc;
                    let at = inst_at.get(&p.inst).map_or(p.pos, |e| e.0) + inst_cull.get(&p.inst).copied().unwrap_or(Vec3::ZERO) * p.zoom;
                    // (w: the key light's way - RenderState.LightingMode 2 and up: towards the sun (a point
                    // 100 m off the effect, `F-75930`); 0 and 1: the light group's default (0, -1, 0),
                    // straight down, as recorded on the tyre smoke and the fountains)
                    arrow_b = [at.x, at.y, at.z, if pr.render.lighting_mode >= 2 { 1.0 } else { 0.0 }];
                    // (arrow_c rgb: what the picker adds to gLightAmbient there - the night fill
                    // light, map / 255 x 0.25 x m_NFLFade, as for the traffic cars, not cut to bytes;
                    // recorded c26 (0.2255, 0.2382, 0.2363) = (0.0353, 0.0510, 0.0588) + (0.1902,
                    // 0.1873, 0.1775) at checkpoint 0 of AROUND HOLLYWOOD, 01:06)
                    let f = crate::fillmap::body_now(at.x, at.z);
                    arrow_c = [f[0], f[1], f[2], self.fog_min[4]];
                    // (arrow_d: the fog's colour at its height; arrow_c.w the height)
                    arrow_d = [self.fog_min[0], self.fog_min[1], self.fog_min[2], self.fog_min[3]];
                }
                if let Some(tr) = p.trail.as_ref() {
                    // UseParticleTrails (`F-56257`): count - 2 quads along the points (each moved
                    // the share f towards the next), one side for the strip: normalize((P - Pprev) x
                    // (P - camera)), half width = the sprite's; v along the strip, the oldest end at
                    // the frame's bottom, the newest at its top; u across
                    let n = tr.nodes.len();
                    let side = (p.pos - p.prev).normalize_or_zero().cross((p.pos - cam).normalize_or_zero()).normalize_or_zero() * w;
                    let q = |i: usize| tr.nodes[i] + (tr.nodes[(i + 1).min(n - 1)] - tr.nodes[i]) * tr.f;
                    let k = |i: usize| if i + 2 >= n { 0.0 } else { 1.0 - i as f32 / (n - 2) as f32 };
                    if self.frame % 60 == 0 && std::env::var("MCLA_FX_LOG").is_ok() {
                        log::info!("fx trail {}: points {:?} f {:.2} side {:?} half height {:.3} uv {:?} colour {:?} alpha {:.3}", pr.name, tr.nodes, tr.f, side, h, uv, c, alpha);
                    }
                    for i in 0..n.saturating_sub(2) {
                        let (a, b) = (q(i), q(i + 1));
                        let (ka, kb) = (k(i), k(i + 1));
                        let c3 = (a + b) * 0.5;
                        let mut uvi = uv;
                        uvi[1] = uv[1] + uv[3] * kb;
                        uvi[3] = uv[3] * (ka - kb);
                        let mut uv2i = uv2;
                        uv2i[1] = uv2[1] + uv[3] * kb;
                        let (du, dvv) = ((b - a) * 0.5, side);
                        items.push((card.sheet as u32, rank, skey, PInst { pos_size: [c3.x, c3.y, c3.z, w], color: [c.x, c.y, c.z, alpha], uv: uvi, misc: [rot, 1.0, 1.0, lit], extra, arrow_a, arrow_b, arrow_c, arrow_d, uv2: uv2i, quad_u: [du.x, du.y, du.z, 1.0], quad_v: [dvv.x, dvv.y, dvv.z, 0.0], base_a, base_b }));
                    }
                    continue;
                }
                items.push((card.sheet as u32, rank, skey, PInst { pos_size: [centre.x, centre.y, centre.z, h], color: [c.x, c.y, c.z, alpha], uv, misc: [rot, w / h, 1.0, lit], extra, arrow_a, arrow_b, arrow_c, arrow_d, uv2, quad_u, quad_v, base_a, base_b }));
            }
        }
        // (stable: a trail's pieces keep their order)
        items.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)));
        let mut insts: Vec<PInst> = Vec::with_capacity(items.len());
        for (k, _, _, inst) in items {
            match self.runs.last_mut() {
                Some(r) if r.0 == k => r.2 += 1,
                _ => self.runs.push((k, insts.len() as u32, 1)),
            }
            insts.push(inst);
        }
        if insts.is_empty() {
            return;
        }
        if self.buf.is_none() || self.cap < insts.len() {
            self.cap = (insts.len() * 2).max(256);
            self.buf = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("particles"), size: (self.cap * std::mem::size_of::<PInst>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        }
        if let Some(b) = self.buf.as_ref() {
            gpu.queue.write_buffer(b, 0, bytemuck::cast_slice(&insts));
        }
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu) {
        let (Some(buf), Some(dbg)) = (self.buf.as_ref(), self.depth_bg.as_ref()) else { return };
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.set_bind_group(2, dbg, &[]);
        for &(k, start, n) in &self.runs {
            let k = k as usize;
            let Some(bind) = self.binds.get(k).and_then(|b| b.as_ref()) else { continue };
            if n == 0 {
                continue;
            }
            pass.set_pipeline(if self.additive[k] { &gpu.pipeline_particle_add } else { &gpu.pipeline_particle });
            pass.set_bind_group(1, bind.as_ref(), &[]);
            pass.draw(0..4, start..start + n);
        }
    }
}
