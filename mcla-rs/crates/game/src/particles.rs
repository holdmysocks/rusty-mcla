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
    /// (`blk+84`, vertex float 17 of `sub_825F8358`)
    uv2: [f32; 4],
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
    /// its clock 0..1 and 1 / life
    t: f32,
    inv_life: f32,
    /// what its emitter makes of a second (effect and event scalar, `TimeScaleKF`)
    ts: f32,
    /// its random numbers: size, colour, alpha, acceleration, damping, playback, matrix
    /// weight, spin, wind
    r: [f32; 9],
    theta: f32,
    spin0: f32,
    /// the texture frames it blends between (`blk+92` A, `blk+93` B) by `anim`
    frame0: u8,
    frame: u8,
    anim: f32,
    /// the emit rule's factors at its birth (size x y; acceleration, damping x y z; matrix weight)
    size_ot: [f32; 2],
    accel_ot: Vec3,
    damp_ot: Vec3,
    mw_ot: f32,
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
    /// `sub_824B6E40`, `sub_824BB3E8`) and an opacity of the caller's (the checkpoints')
    tint: [f32; 3],
    alpha: f32,
    /// the checkpoint render setup's ShadowColor (`sub_82264968`: the look's m_ParticleShadowColor)
    shadow: [f32; 3],
    /// the instance's evolution values (+436, one per name of the rule's group - `sub_8247A6A0`)
    evo: [f32; 4],
    /// the checkpoint render setup's +48 (EffectPos.w): the projected arrows are on
    arrow: bool,
    /// the instance's random number for its `ZoomKF` (instance byte 492)
    zr: f32,
    /// started once and left to run out (`burst`): it plays without being asked again and is
    /// dropped when it and what it let out are done
    oneshot: bool,
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
    /// instance ranges per sheet after the last `prepare`
    ranges: Vec<(u32, u32)>,
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
    /// the wind the rules' `WindInfluenceKF` takes a share of (m/s; the port has no wind field)
    pub wind: Vec3,
    /// the clock (hours): the checkpoint smoke's night boost
    pub hour: f32,
    /// the lit sprites' fog: RMPTFX_FogMinColor rgba and RMPTFX_FogParams.x (the height)
    pub fog_min: [f32; 5],
    /// the projected arrows' 128 x 128 target this frame: its quad's two turns and which
    /// `checkpoint_arrows_anim_*` it shows (`checkpoints::Markers::update`)
    pub arrow_rt: [f32; 3],
    /// seconds the effects have run (the projected map's CurrentTime)
    clock: f32,
}

const MAX: usize = 3000;
/// The particle shader's mode for the programs ported as the original draws them (the scene's
/// linear value, not a display colour): 4 = `mc4_rmptfx_mainlist`'s draw_SoftPart_VertLit when
/// it adds (blend 7 - the flares' fire, glows), 5 = `mc4_rmptfx_default`'s unlit_draw (the
/// checkpoints' sparks); 0 = the generic look.
pub(crate) fn sprite_mode(s: &mcla_formats::rage::ptfx::Sprite, blend_set: u32) -> f32 {
    match (s.shader.as_str(), s.technique.as_str()) {
        ("mc4_rmptfx_mainlist", "draw_SoftPart_VertLit") if blend_set == 7 => 4.0,
        ("mc4_rmptfx_default", "unlit_draw") => 5.0,
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
/// most effects playing at once (69 particle props stand in the city, a few in reach)
const MAX_EFFECTS: usize = 48;
/// the effects whose sprites are loaded: the particle props' and the litter's
// (step 19g: + the power-up shots' sparks, powerup_sparks - sub_82748340)
const PLAYED: [&str; 10] = ["water_fountain", "water_rings", "water_waterfall", "amb_steam", "amb_smoke", "wind_debris", "checkpoint_ctf_generic", "light_column", "powerup_sparks", "damage_glassbreak"];

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
            ranges: vec![(0, 0); KINDS],
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
            wind: Vec3::new(1.5, 0.0, 0.8),
            hour: 12.0,
            fog_min: [0.0, 0.0, 0.0, 0.0, 1000.0],
            arrow_rt: [0.0, std::f32::consts::PI, 1.0],
            clock: 0.0,
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

        // the projected arrows' four textures (`sub_82262600`: `$/tune/rmptfx/textures`
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
                let key = h ^ 0x7f00_0000;
                let tex = gpu.upload_texture(key, &t);
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
        let mut flat = 0;
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
                            // a quad lying on an axis (rings on the water, a fountain's base) is not
                            // a billboard: the particle pipeline cannot draw it
                            if pr.flags & flag::DRAW_DIRECTIONAL != 0 && sp.axis.length_squared() > 0.0 {
                                flat += 1;
                            } else if let Some(s) = sheet(self, gpu, &sp.texture, additive) {
                                cards.push(Card { sheet: s, uv: [0.0, 0.0, 1.0, 1.0], half: [1.0, 1.0] });
                            }
                        } else if let Some(m) = &pr.model {
                            for d in &m.drawables {
                                let e = (d.max - d.min) * 0.5;
                                // the sheet of paper as a card: its two long sides
                                let mut s = [e.x, e.y, e.z];
                                s.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                                if let Some(sh) = sheet(self, gpu, &d.texture, false).filter(|_| s[1] > 0.0) {
                                    cards.push(Card { sheet: sh, uv: [d.uv[0], d.uv[1], d.uv[2] - d.uv[0], d.uv[3] - d.uv[1]], half: [s[1], s[0]] });
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
        self.ranges = vec![(0, 0); self.binds.len()];
        log::info!("particles: {} effect / {} emit / {} particle rules; {} effects playable from them with {} rules on {} sheets ({} flat rules not drawn)", fx.effects.len(), fx.emit_rules.len(), fx.particle_rules.len(), self.playable.iter().filter(|p| **p).count(), looks.iter().filter(|l| l.is_some()).count(), self.binds.len() - KINDS, flat);
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
    /// `evo`: evolution values by name (`sub_8247A6A0`, e.g. the checkpoints' "Height", "time").
    pub fn play_tinted(&mut self, key: u64, effect: u32, pos: Vec3, tint: [f32; 3], shadow: [f32; 3], alpha: f32, dist: f32, evo: &[(&str, f32)]) -> bool {
        self.play_ex(key, effect, pos, Quat::IDENTITY, Vec3::ZERO, dist, tint, shadow, alpha, f32::INFINITY, evo)
    }

    #[allow(clippy::too_many_arguments)]
    fn play_ex(&mut self, key: u64, effect: u32, pos: Vec3, rot: Quat, vel: Vec3, dist: f32, tint: [f32; 3], shadow: [f32; 3], alpha: f32, cap: f32, evo_in: &[(&str, f32)]) -> bool {
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
        // distance between the rule's LOD near and far (`sub_824B1058` 0x824B1300..0x824B1348)
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
        self.effects.push(Effect { key, id, effect: ei, pos, rot, vel, moved: Vec3::ZERO, t, dur, rate: if rate > 0.0 { rate } else { 1.0 }, loops: 0, done: false, seen: self.frame, fade, em: Vec::new(), kids: Vec::new(), tint, alpha, shadow, evo, arrow: false, zr, oneshot: false });
        true
    }

    /// Start an effect of the disc's list once at a place, moving with `vel`, and let it run out on
    /// its own (a one-shot such as `damage_glassbreak`: the game's `sub_8247FEE0` + start, the
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

    /// The checkpoint render setup's EffectPos.w for the effect playing under `key`: the arrows
    /// projected onto its low smoke are on (`sub_82265BB0` 0x82265D48).
    pub fn set_arrow_on(&mut self, key: u64, on: bool) {
        for e in self.effects.iter_mut().filter(|e| e.key == key) {
            e.arrow = on;
        }
    }

    /// One step of an effect's timeline (`sub_824DAF28`): events whose time lies in the step
    /// start their emitter or effect; at the end of the loop it starts again or ends.
    fn step_effect(&mut self, fx: &FxList, e: &mut Effect, dt: f32, live: bool) {
        let rule = &fx.effects[e.effect];
        if e.em.len() != rule.events.len() {
            e.em = (0..rule.events.len())
                .map(|_| {
                    self.next_id += 1;
                    Emitter { playing: false, t: 0.0, dur: 1.0, tsc: 1.0, zoom: 1.0, carry: 0.0, uid: self.next_id }
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
                    }
                    EventKind::Effect { effect: Some(k), domain, .. } if self.playable[*k] && e.kids.len() < 12 => {
                        let kr = &fx.effects[*k];
                        // where: a point of the event's domain, in the parent's frame
                        let at = domain.as_ref().map_or(Vec3::ZERO, |d| d.centre(0.0) + d.point(0.0, &mut || rng.f()));
                        let id = e.id;
                        e.kids.push(Effect { key: 0, id, effect: *k, pos: e.pos + e.rot * at, rot: e.rot, vel: e.vel, moved: Vec3::ZERO, t: 0.0, dur: rng.range(kr.duration[0], kr.duration[1]).max(0.01), rate: rng.range(kr.time_scalar[0], kr.time_scalar[1]).max(0.01), loops: 0, done: false, seen: 0, fade: e.fade, em: Vec::new(), kids: Vec::new(), tint: e.tint, alpha: e.alpha, shadow: e.shadow, evo: [0.0; 4], arrow: false, zr: rng.f(), oneshot: false });
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

    /// One step of an emitter (`sub_824B40B8`, `sub_824B3B18`): its clock, how many particles
    /// the step lets out, and their start (`sub_824B32D8`, `sub_824BD930`).
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
        let mut n = if er.one_shot {
            em.t = 1.0;
            r as i32
        } else {
            let x = r * frac * dt * ts + em.carry;
            em.carry = x.fract();
            x as i32
        };
        if em.t >= 1.0 {
            em.playing = false;
        }
        // (the zoom: the event's x the effect's own ZoomKF / 100 now - instance +500 x emitter
        // +244, on sizes and domains; taken at the particle's birth here)
        let ez = fx.effects[e.effect].zoom.eval(e.t);
        let ez = if fx.effects[e.effect].zoom.keys.is_empty() { 1.0 } else { lerp(ez[0], ez[1], e.zr) * 0.01 };
        let (uid, zoom, pscale) = (em.uid, em.zoom * ez, e.rate * em.tsc * ts);
        self.rng = rng.0;
        let Some(look) = self.looks[*pi].as_ref().filter(|_| live && n > 0) else { return };
        let ncards = look.cards.len();
        // SpawnRateKF z: the most that may be alive
        if rate[2] >= 1.0 {
            n = n.min(rate[2] as i32 - self.rule_list.iter().filter(|p| p.uid == uid).count() as i32);
        }
        let mut rng = Rng(self.rng);
        for k in 1..=n.min(64) {
            if self.rule_list.len() >= MAX_RULE {
                break;
            }
            // a point of the emitter domain; the velocity domain's point is the velocity, or
            // where the particle heads from its start
            let lf = ev(0, 2, er.spawn_life.eval(te));
            let life = rng.range(lf[0], lf[1]);
            let (mut start, mut at) = (Vec3::ZERO, e.pos);
            // (the domains' position and sizes: lists 1 and 2)
            let dom = |l: u8, d: &mcla_formats::rage::ptfx::Domain, rnd: &mut dyn FnMut() -> f32| -> Vec3 {
                let c = ev(l, 0, d.position.eval(te));
                Vec3::new(c[0], c[1], c[2]) * d.scale + d.point_sized(te, ev(l, 2, d.size.eval(te)), ev(l, 3, d.inner_size.eval(te)), rnd)
            };
            if let Some(d) = &er.emitter_domain {
                start = dom(1, d, &mut || rng.f()) * zoom;
                at += if d.world_space { start } else { e.rot * start };
            }
            let mut vel = Vec3::ZERO;
            if let Some(d) = &er.velocity_domain {
                let p = dom(2, d, &mut || rng.f()) * zoom;
                let v = if d.point_relative { p } else { p - start };
                vel = if d.world_space { v } else { e.rot * v };
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
                t: 0.0,
                inv_life: if life > 1e-3 { 1.0 / life } else { 1.0 },
                ts: pscale,
                r: [0.0; 9],
                theta: 0.0,
                spin0: 0.0,
                frame0: 0,
                frame: 0,
                anim: 0.0,
                size_ot: [sot[0] * 0.01, sot[1] * 0.01],
                accel_ot: v3(ev(0, 5, er.accel_ot.eval(te))) * 0.01,
                damp_ot: v3(ev(0, 6, er.damp_ot.eval(te))) * 0.01,
                mw_ot: ev(0, 7, er.matrix_weight_ot.eval(te))[0] * 0.01,
                zoom,
                rot: e.rot,
                fade: e.fade,
                te,
                tint: e.tint,
                shadow: e.shadow,
                eff: e.effect as u16,
                ev: i as u16,
            };
            for r in p.r.iter_mut() {
                *r = rng.f();
            }
            if let Some(s) = &pr.sprite {
                // first frame, first turn (degrees), first spin (turns a second)
                p.frame = (s.frame_start + (rng.f() * (s.frame_end - s.frame_start + 1) as f32) as i32).clamp(0, 255) as u8;
                p.frame0 = p.frame;
                let (th, rv) = (s.init_theta.eval(te), s.init_rot_vel.eval(te));
                p.theta = rng.range(th[0], th[1]).to_radians();
                p.spin0 = rng.range(rv[0], rv[1]) * std::f32::consts::TAU;
            } else if let Some(m) = &pr.model {
                // a model particle tumbles in three dimensions; its card only turns in the screen
                let rs = m.init_rotation_speed.eval(te);
                p.theta = rng.f() * std::f32::consts::TAU;
                p.spin0 = rng.range(rs[0], rs[1]) * std::f32::consts::TAU * if rng.f() < 0.5 { -1.0 } else { 1.0 };
            }
            // spread over the step: the earlier ones have already moved on
            if !er.one_shot {
                let age = (1.0 - k as f32 / n as f32) * dt * pscale;
                p.pos += p.vel * age;
                p.t = (age * p.inv_life).min(0.99);
            }
            self.rule_list.push(p);
        }
        self.rng = rng.0;
    }

    /// One step of the particles of the effect list's rules (`sub_824BE248`).
    fn step_rule_particles(&mut self, fx: &FxList, dt: f32, moved: &[(u32, Vec3)]) {
        let wind = self.wind;
        let mut rng = Rng(self.rng);
        self.rule_list.retain_mut(|p| {
            let pr = &fx.particle_rules[p.rule as usize];
            let dt = dt * p.ts;
            let pb = pr.playback_rate.eval(p.t);
            let t = p.t + lerp(pb[0], pb[1], p.r[5]) * p.inv_life * dt;
            if t > 1.0 {
                return false;
            }
            let mut acc = v3(pr.accel_min.eval(t)).lerp(v3(pr.accel_max.eval(t)), p.r[3]) * p.accel_ot;
            if pr.flags & flag::WORLD_SPACE_ACCELERATION == 0 {
                acc = p.rot * acc;
            }
            if pr.flags & flag::ZOOM_ACCELERATION != 0 {
                acc *= p.zoom;
            }
            let damp = v3(pr.damp_min.eval(t)).lerp(v3(pr.damp_max.eval(t)), p.r[4]) * p.damp_ot;
            p.vel += acc * dt;
            let mw = pr.matrix_weight.eval(t);
            let mw = (lerp(mw[0], mw[1], p.r[6]) * p.mw_ot).clamp(0.0, 1.0);
            // the share of its effect's movement it follows
            if let Some(m) = moved.iter().find(|m| m.0 == p.inst) {
                p.pos += m.1 * mw;
            }
            p.vel *= (Vec3::ONE - damp * dt).max(Vec3::ZERO);
            let wi = pr.wind_influence.eval(t);
            let mut wi = lerp(wi[0], wi[1], p.r[8]);
            if pr.flags & flag::WIND_IGNORES_MATRIX_WEIGHT == 0 {
                wi *= 1.0 - mw;
            }
            p.pos += (p.vel + wind * wi) * dt;
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
                        // (sub_824BE248: A takes B, B the next frame)
                        p.frame0 = p.frame;
                        p.frame = next.clamp(0, 255) as u8;
                    }
                }
            } else {
                p.theta += p.spin0 * dt;
            }
            p.t = t;
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

    /// Nitrous flame out of an exhaust tip.
    pub fn nitro_flame(&mut self, pos: Vec3, back: Vec3, car_vel: Vec3) {
        let frame = (self.rand() * 64.0) as u32;
        let rot = self.rand() * 0.4 - 0.2;
        let p = Particle { kind: Kind::Flame, pos, vel: car_vel + back * (5.0 + self.rand() * 3.0), age: 0.0, life: 0.14 + self.rand() * 0.08, size0: 0.22, size1: 0.06, color: [0.45, 0.65, 1.6, 0.9], rot, spin: 0.0, frame, gravity: 0.0, drag: 3.0 };
        self.push(p);
        if self.rand() < 0.35 {
            self.push(Particle { kind: Kind::Glow, pos, vel: car_vel, age: 0.0, life: 0.06, size0: 0.55, size1: 0.4, color: [0.3, 0.5, 1.4, 0.5], rot: 0.0, spin: 0.0, frame: 0, gravity: 0.0, drag: 0.0 });
        }
    }

    /// Exhaust pop when lifting off at high revs.
    pub fn backfire(&mut self, pos: Vec3, back: Vec3, car_vel: Vec3) {
        for _ in 0..5 {
            let frame = (self.rand() * 64.0) as u32;
            let j = Vec3::new(self.rand() - 0.5, self.rand() - 0.5, self.rand() - 0.5) * 1.5;
            let p = Particle { kind: Kind::Flame, pos, vel: car_vel + back * (4.0 + self.rand() * 4.0) + j, age: 0.0, life: 0.16 + self.rand() * 0.1, size0: 0.3, size1: 0.08, color: [1.6, 0.9, 0.35, 0.9], rot: 0.0, spin: 0.0, frame, gravity: 0.0, drag: 3.0 };
            self.push(p);
        }
        self.push(Particle { kind: Kind::Glow, pos, vel: car_vel, age: 0.0, life: 0.09, size0: 0.9, size1: 0.3, color: [1.5, 0.8, 0.3, 0.6], rot: 0.0, spin: 0.0, frame: 0, gravity: 0.0, drag: 0.0 });
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
        self.step_rule_particles(&fx, dt, &moved);
        if self.frame % 60 == 0 && std::env::var("MCLA_FX_LOG").is_ok() {
            let names: Vec<String> = self.effects.iter().map(|e| format!("{} t {:.2} kids {} em {}", fx.effects[e.effect].name, e.t, e.kids.len(), e.em.iter().filter(|m| m.playing).count())).collect();
            log::info!("fx: dt {:.4} effects {} particles {} first {:?}: {:?}", dt, self.effects.len(), self.rule_list.len(), self.rule_list.first().map(|p| (p.pos, p.t, p.ts)), names);
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
        for r in self.ranges.iter_mut() {
            *r = (0, 0);
        }
        if self.list.is_empty() && self.rule_list.is_empty() {
            return;
        }
        let mut items: Vec<(u32, f32, PInst)> = Vec::with_capacity(self.list.len() + self.rule_list.len());
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
            items.push((k as u32, p.pos.distance_squared(cam), PInst { pos_size: [p.pos.x, p.pos.y, p.pos.z, size], color: [p.color[0], p.color[1], p.color[2], p.color[3] * fade], uv: [fx as f32 * du, fy as f32 * dv, du, dv], misc: [rot, sx, sy, if matches!(p.kind, Kind::Smoke | Kind::Splash) { 1.0 } else { 0.0 }], extra: [0.0; 4], arrow_a: [0.0; 4], arrow_b: [0.0; 4], arrow_c: [0.0; 4], arrow_d: [0.0; 4], uv2: [0.0; 4] }));
        }
        if let Some(fx) = self.fx.as_ref() {
            // (every effect instance's opacity and evolution values, by its id)
            let mut inst: HashMap<u32, (f32, [f32; 4])> = HashMap::new();
            // (and where it stands, with its render setup's EffectPos.w)
            let mut inst_at: HashMap<u32, (Vec3, bool)> = HashMap::new();
            // (and its cull sphere's centre offset, turned with it: `sub_824C1B08`)
            let mut inst_cull: HashMap<u32, Vec3> = HashMap::new();
            for e in &self.effects {
                inst.insert(e.id, (e.alpha, e.evo));
                inst_at.insert(e.id, (e.pos, e.arrow));
                inst_cull.insert(e.id, e.rot * Vec3::from(fx.effects[e.effect].cull_offset));
            }
            for p in &self.rule_list {
                let pr = &fx.particle_rules[p.rule as usize];
                let (ialpha, ievo) = inst.get(&p.inst).copied().unwrap_or((1.0, [0.0; 4]));
                let evo_g = fx.effects.get(p.eff as usize).and_then(|r| r.events.get(p.ev as usize)).and_then(|e| e.evo.as_ref());
                let pev = |id: u8, base: [f32; 4]| -> [f32; 4] {
                    match evo_g {
                        Some(g) => g.evolve(3, id, base, p.t, &ievo),
                        None => base,
                    }
                };
                let Some(card) = self.looks[p.rule as usize].as_ref().and_then(|l| l.cards.get(p.card as usize)) else { continue };
                // colour between ColorKF and ColorMaxKF, alpha between AlphaKF's x and y
                // (`sub_824B5728`: ColorKF; between it and ColorMaxKF by a random number of the
                // particle only with the rule's random-colour bit)
                let mut c = v3(pev(0, pr.color.eval(p.t)));
                if pr.flags & if pr.sprite.is_some() { flag::RANDOM_COLOR } else { flag::MODEL_RANDOM_COLOR } != 0 {
                    c = c.lerp(v3(pev(1, pr.color_max.eval(p.t))), p.r[1]);
                }
                if pr.flags & 8 != 0 {
                    c *= Vec3::from(p.tint);
                }
                // (the vertex holds it as 8-bit A, R, G, B: clamped, x 255, truncated - `sub_821D2470`)
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
                    // and mc4_rmptfx_default's PS_Draw 0.1, ps_main_SoftPart_VertLit 4; 0 none)
                    let soft = match sprite_mode(s, pr.render.blend_set) {
                        _ if projected && s.technique == "draw" => 0.1,
                        4.0 => 4.0,
                        5.0 => 0.1,
                        _ => 0.0,
                    };
                    uv2 = [(f0 % s.tiles_x) as f32 * du, (f0 / s.tiles_x) as f32 * dv, p.anim, soft];
                    rot = p.theta;
                    if pr.flags & flag::DRAW_DIRECTIONAL != 0 {
                        // along its velocity, as the billboard sees it
                        let to_cam = (cam - p.pos).normalize_or_zero();
                        let right = Vec3::Y.cross(to_cam).normalize_or_zero();
                        let up = to_cam.cross(right);
                        let (vr, vu) = (p.vel.dot(right), p.vel.dot(up));
                        if vr * vr + vu * vu > 1e-6 {
                            rot = (-vr).atan2(vu);
                        }
                    }
                } else if let Some(m) = &pr.model {
                    let (s0, s1) = (m.size_min.eval(p.t), m.size_max.eval(p.t));
                    let sc = lerp(s0[0], s1[0], p.r[0]) * p.size_ot[0] * p.zoom;
                    // (the flutter of a sheet of paper: its card narrows and widens)
                    w = card.half[0] * sc * (0.35 + 0.65 * (p.theta * 1.7).cos().abs());
                    h = card.half[1] * sc;
                    uv = card.uv;
                    rot = p.theta;
                } else {
                    continue;
                }
                if !(w > 1e-4 && h > 1e-4) {
                    continue;
                }
                let mode = pr.sprite.as_ref().map_or(0.0, |s| sprite_mode(s, pr.render.blend_set));
                let lit = if projected { 3.0 } else if column { 2.0 } else if mode > 0.0 { mode } else if self.additive[card.sheet] { 0.0 } else { 1.0 };
                let mut extra = if projected { [p.shadow[0], p.shadow[1], p.shadow[2], (2.0 * night).max(1.0)] } else { [0.0; 4] };
                let mut normal_arc = 0.0;
                if let (4.0, Some(s)) = (mode, pr.sprite.as_ref()) {
                    // (vs_main_SoftPart_VertLit's variables, the effect file's defaults where the rule has none)
                    let var = |n: &str, d: f32| s.vars.iter().find(|(k, _)| k == n).map_or(d, |(_, v)| v[0]);
                    extra = [var("gDiffuse", 1.0), var("gAmbient", 1.0), var("gAmbientDayControl", 1.0), var("gTranslucency", 0.0)];
                    normal_arc = var("gNormalArc", 0.0);
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
                if mode == 4.0 {
                    // (x: gNormalArc; arrow_b xyz: the light picker's position - the effect's cull
                    // sphere centre, its place + the rule's offset x the event's and the effect's
                    // zoom, from the frame before; checkpoint_ctf_generic: 68 m straight up)
                    arrow_a[0] = normal_arc;
                    let at = inst_at.get(&p.inst).map_or(p.pos, |e| e.0) + inst_cull.get(&p.inst).copied().unwrap_or(Vec3::ZERO) * p.zoom;
                    arrow_b = [at.x, at.y, at.z, 0.0];
                    // (arrow_c rgb: what the picker adds to gLightAmbient there - the night fill
                    // light, map / 255 x 0.25 x m_NFLFade, as for the traffic cars, not cut to bytes;
                    // recorded c26 (0.2255, 0.2382, 0.2363) = (0.0353, 0.0510, 0.0588) + (0.1902,
                    // 0.1873, 0.1775) at checkpoint 0 of AROUND HOLLYWOOD, 01:06)
                    let f = crate::fillmap::body_now(at.x, at.z);
                    arrow_c = [f[0], f[1], f[2], self.fog_min[4]];
                    // (arrow_d: the fog's colour at its height; arrow_c.w the height)
                    arrow_d = [self.fog_min[0], self.fog_min[1], self.fog_min[2], self.fog_min[3]];
                }
                items.push((card.sheet as u32, p.pos.distance_squared(cam), PInst { pos_size: [p.pos.x, p.pos.y, p.pos.z, h], color: [c.x, c.y, c.z, alpha], uv, misc: [rot, w / h, 1.0, lit], extra, arrow_a, arrow_b, arrow_c, arrow_d, uv2 }));
            }
        }
        items.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)));
        let mut insts: Vec<PInst> = Vec::with_capacity(items.len());
        for (k, _, inst) in items {
            let k = k as usize;
            if self.ranges[k].1 == 0 {
                self.ranges[k].0 = insts.len() as u32;
            }
            self.ranges[k].1 += 1;
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
        for k in 0..self.binds.len().min(self.ranges.len()) {
            let (start, n) = self.ranges[k];
            let Some(bind) = self.binds[k].as_ref() else { continue };
            if n == 0 {
                continue;
            }
            pass.set_pipeline(if self.additive[k] { &gpu.pipeline_particle_add } else { &gpu.pipeline_particle });
            pass.set_bind_group(1, bind.as_ref(), &[]);
            pass.draw(0..4, start..start + n);
        }
    }
}
