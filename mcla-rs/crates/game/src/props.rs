//! Instanced city props: the foliage set (trees, palms, hedges, bushes) placed by
//! `resources/prop/0x982ff5e0.r134`, drawn with the models of the prop dictionary
//! `resources/prop/0x0e66973c.r131` and the prop texture dictionary `0xf7b7d95f.r1`.

use crate::assets::load_resource_file;
use crate::render::{Frustum, Gpu, GpuTexture, MatMode, Vertex};
use crate::world::World;
use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec2, Vec3};
use hashbrown::HashMap;
use mcla_formats::rage::props::{parse_foliage, parse_props};
use mcla_formats::rage::texture::Texture;
use mcla_formats::rage::{Dictionary, Rd};
use std::path::Path;
use std::sync::Arc;
use wgpu::util::DeviceExt;

mod impostor;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Inst {
    /// xyz position, w = scale (of the model's x and z axes where the height differs)
    pub pos_scale: [f32; 4],
    /// orientation quaternion (xyzw), its squared length the height scale over the width
    /// scale (1 for everything but plants); for light glows: colour + intensity
    pub rot_tint: [f32; 4],
    /// the night fill light at the place (fillmap.rs): plants the colour of their own record,
    /// the prop list's instances a quarter of the map's; alpha 255 = "has a value", 0 = none
    pub fill: [u8; 4],
    /// plants: their own lamp at night (formats `FoliageLamp`; LIGHTING_WORK.md "The plants' own
    /// lamp at night") - xyz its place relative to the instance, w its strength (0: none)
    pub lamp_pos: [f32; 4],
    /// ... its colour, a = the cone's inner byte
    pub lamp_col: [u8; 4],
    /// ... its direction bytes (x 2 - 1), a = the cone's outer byte
    pub lamp_dir: [u8; 4],
}

impl Inst {
    /// All zero: for `..Inst::NONE` in the constructors (no fill light, no lamp).
    pub const NONE: Inst = Inst { pos_scale: [0.0; 4], rot_tint: [0.0; 4], fill: [0; 4], lamp_pos: [0.0; 4], lamp_col: [0; 4], lamp_dir: [0; 4] };
    /// A model at `pos`, turned by `rot`, its x / z axes scaled by `width` and its y axis by `height`.
    pub fn placed(pos: Vec3, rot: Quat, width: f32, height: f32) -> Inst {
        let k = (height / width.max(1e-4)).max(1e-4).sqrt();
        Inst { pos_scale: [pos.x, pos.y, pos.z, width], rot_tint: [rot.x * k, rot.y * k, rot.z * k, rot.w * k], fill: [0; 4], ..Inst::NONE }
    }
    fn pos(&self) -> Vec3 {
        Vec3::new(self.pos_scale[0], self.pos_scale[1], self.pos_scale[2])
    }
    /// (unit rotation, height scale over width scale)
    fn rot(&self) -> (Quat, f32) {
        let q = Quat::from_xyzw(self.rot_tint[0], self.rot_tint[1], self.rot_tint[2], self.rot_tint[3]);
        let k = q.length_squared();
        (if k > 1e-8 { q.normalize() } else { Quat::IDENTITY }, k)
    }
}

impl Inst {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Inst>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 6 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 7 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 32, shader_location: 9 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 36, shader_location: 10 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 52, shader_location: 11 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 56, shader_location: 12 },
        ],
    };
}

struct PropBatch {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
    bind: Arc<wgpu::BindGroup>,
    mode: MatMode,
}

struct PropModel {
    batches: Vec<PropBatch>,
    radius: f32,
    /// radius of the drawable's bounding sphere: what the draw distances are taken from
    sphere: f32,
    /// 0 a prop, 1 a plant with a far card, 2 a hedge, 3 a plant's card
    kind: u8,
}

impl PropModel {
    /// How far the original draws an instance of scale `scale` through a lens of `fov` radians
    /// (work/map/lod_rule.md, read from the executable and checked against every recorded fade
    /// constant): (the model, anything of it).
    /// - props: R (80 / tan(fov / 2) - 0.5), at most 2250 m (360 R at the 25 degree lens);
    /// - plants: the model to 2 R / tan(0.45 degrees per degree of half lens) (20.3 R at 25
    ///   degrees), cards to 2 R / tan(0.025 degrees ...) (367 R), both times the plant's up
    ///   scale, the scale capped at 2. The cards' reach is cut at `FAR_DIST` here (the
    ///   original's far plane and fog end them);
    /// - hedges: no card, the model to 2 R / tan(0.075 degrees ...) (122 R).
    ///
    /// `k` = (DRAW DISTANCE, DETAIL LEVEL) of OPTIONS > PERFORMANCE (`Lod`; 1 = the original): the
    /// draw distance scales where anything of it ends (the prop's distance and its 2250 m cap, the
    /// cards' and hedges' far distance and `FAR_DIST`), the detail level where a plant's model gives
    /// way to its card (city.wgsl `vs_inst` takes the same two from `Globals::lod`).
    fn reach(&self, scale: f32, fov: f32, k: Lod) -> (f32, f32) {
        let r = self.sphere;
        let half_deg = fov.to_degrees() * 0.5;
        let by = |per: f32| 2.0 * r / (half_deg * std::f32::consts::PI / per).tan().max(1e-6) * scale.min(2.0);
        let cap = FAR_DIST * k.draw;
        match self.kind {
            1 => {
                let far = (by(7200.0) * k.draw).min(cap);
                // (the model's angle x 0.9 while the game is paused: F-63362, 0x82007FA8)
                let model = 2.0 * r / (half_deg * k.plant_angle * std::f32::consts::PI / 400.0).tan().max(1e-6) * scale.min(2.0);
                ((model * k.detail).min(far), far)
            }
            // hedges: the high level to L = 2R / tan(h pi/600) (DETAIL LEVEL), the low level
            // (`Props::hedge_low`, kind 5) from L - 16 to the far end
            2 => {
                let far = (by(2400.0) * k.draw).min(cap);
                ((by(600.0) * k.detail).min(far), far)
            }
            5 => {
                let far = (by(2400.0) * k.draw).min(cap);
                (0.0, far)
            }
            3 => {
                let far = (by(7200.0) * k.draw).min(cap);
                (0.0, far)
            }
            _ => {
                let far = (r * scale * (80.0 / (fov * 0.5).tan().max(0.02) - 0.5) * k.draw).min(2250.0 * k.draw);
                (far, far)
            }
        }
    }
}

/// The two OPTIONS > PERFORMANCE scales the draw distances take (1 = the original's), and the
/// original's own paused-game factor on the plants' model angle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lod {
    /// DRAW DISTANCE: where things end
    pub draw: f32,
    /// DETAIL LEVEL: where the detailed form gives way to the coarser one
    pub detail: f32,
    /// the plants' model angle factor: 0.9 (0x82007FA8) while the renderer's flag of F-31789
    /// is set - the game paused (F-23567 sets it with the pause, F-35144 puts the old
    /// value back) - else 1 (`set_paused`)
    pub plant_angle: f32,
}

/// The renderer's paused-detail flag (renderer +373, read by F-31789).
static PAUSED_DETAIL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// main.rs, once a frame: whether the game is paused (the original's flag of F-31789).
pub fn set_paused(on: bool) {
    PAUSED_DETAIL.store(on, std::sync::atomic::Ordering::Relaxed);
}

impl Lod {
    pub const ORIGINAL: Lod = Lod { draw: 1.0, detail: 1.0, plant_angle: 1.0 };
    /// What the PERFORMANCE page has now (read once a frame by each user: the getters lock).
    pub fn now() -> Lod {
        let plant_angle = if PAUSED_DETAIL.load(std::sync::atomic::Ordering::Relaxed) { 0.9 } else { 1.0 };
        Lod { draw: crate::hooks::draw_distance_scale().clamp(1.0, 4.0), detail: crate::hooks::detail_level_scale().clamp(1.0, 4.0), plant_angle }
    }
}

/// The lens of a standing car (the widest reach of the rules above).
const FOV_REST: f32 = 0.436_332_3;

/// Instances of one model inside one grid cell: a contiguous range of the instance buffer.
struct Run {
    model: usize,
    start: u32,
    count: u32,
    min: Vec3,
    max: Vec3,
    /// largest scale among its instances (the fade distance grows with it)
    scale: f32,
    /// plants: the model of their far card
    card: Option<usize>,
    /// plants: the run again, split by fine cell (so that only the near ones are full models)
    subs: Vec<Run>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Solidity {
    /// drive-through (bushes, flowers)
    None,
    /// immovable (`_static_` props, tree trunks)
    Solid,
    /// knocked away by cars (bins, benches, signs, fences ...)
    Dynamic,
}

/// Collision footprint of a prop type (local space, near the ground).
#[derive(Clone, Copy, Debug)]
struct Foot {
    kind: Solidity,
    center: Vec2,
    half: Vec2,
    y0: f32,
    height: f32,
    mass: f32,
    /// sound category (see `sound::prop_sound`)
    snd: u8,
    /// the type's name hash: its audio object in game.dat (`collaudio`)
    audio: u32,
    /// posts: radius of the wide capsule round them, the zone of the deflection (0: none)
    wide: f32,
    /// in the physics level FIXED (level state 2, the file's instance word 2: `_static_` props, the
    /// deflecting posts, trees): mcCarDamage counts a contact with it 0.4 of itself (F-19931);
    /// state 1 (loose, the parked aircraft too) whole
    fixed: bool,
}

struct Collider {
    /// index into the instance buffer
    inst: u32,
    foot: Foot,
    pos: Vec3,
    rot: Quat,
    scale: f32,
    knocked: bool,
}

/// A knocked-over prop flying / tumbling until it comes to rest.
struct Debris {
    collider: u32,
    com: Vec3,
    vel: Vec3,
    rot: Quat,
    ang: Vec3,
    rest: f32,
    age: f32,
}

const COL_CELL: f32 = 24.0;

fn col_cell(p: Vec3) -> (i32, i32) {
    ((p.x / COL_CELL).floor() as i32, (p.z / COL_CELL).floor() as i32)
}

/// 2D separating-axis test of two oriented boxes on the ground plane. Returns the
/// minimum-penetration axis (pointing from b to a) and the depth.
fn obb_overlap(ca: Vec2, ax: [Vec2; 2], ha: Vec2, cb: Vec2, bx: [Vec2; 2], hb: Vec2) -> Option<(Vec2, f32)> {
    let d = ca - cb;
    let mut best: Option<(Vec2, f32)> = None;
    for axis in [ax[0], ax[1], bx[0], bx[1]] {
        let ra = ha.x * ax[0].dot(axis).abs() + ha.y * ax[1].dot(axis).abs();
        let rb = hb.x * bx[0].dot(axis).abs() + hb.y * bx[1].dot(axis).abs();
        let dist = d.dot(axis);
        let pen = ra + rb - dist.abs();
        if pen <= 0.0 {
            return None;
        }
        if best.map_or(true, |b| pen < b.1) {
            best = Some((if dist >= 0.0 { axis } else { -axis }, pen));
        }
    }
    best
}

/// The light of a prop that has a light file (`tune/lighting/0x{joaat("<prop name>.xml")}`: the
/// street, parking and freeway lights): a spot at the lamp head, as the original's night lamp
/// table holds it (LIGHTING_WORK.md).
#[derive(Clone, Copy, Debug)]
pub struct PropLamp {
    pub pos: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    /// from the lit ground back to the lamp (the lamp shines along the opposite way)
    pub back: Vec3,
    /// the cone: sat(cos x scale + bias)
    pub cone: [f32; 2],
}

/// A prop's light file: (colour, intensity, cone scale and bias).
/// The light files' Direction (90, 0, 171.469223) as the original turns it into a vector
/// (the Euler converter reads degrees x 0.0174556): (sin z, -cos z) of 2.99310 rad - every one of
/// the 75 spot light files has the same Direction.
const LAMP_BACK: [f32; 2] = [0.147_949_35, 0.988_994_95];

fn prop_light_file(data_dir: &str, name: &str) -> Option<(Vec3, f32, [f32; 2])> {
    let file = Path::new(data_dir).join("tune/lighting").join(format!("0x{:08x}", mcla_formats::hash::joaat(&format!("{}.xml", name.to_ascii_lowercase()))));
    let x = std::fs::read_to_string(file).ok()?;
    if !x.contains("<Light>") {
        return None;
    }
    let attr = |tag: &str, key: &str| -> Option<f32> {
        let i = x.find(&format!("<{}", tag))?;
        let rest = &x[i..x[i..].find('>').map(|j| i + j)?];
        let k = rest.find(&format!("{}=\"", key)).or_else(|| rest.find(&format!("{} =\"", key)))?;
        let v = &rest[k..];
        let a = v.find('"')? + 1;
        let b = a + v[a..].find('"')?;
        v[a..b].trim().parse().ok()
    };
    let color = Vec3::new(attr("Colour", "x")?, attr("Colour", "y")?, attr("Colour", "z")?);
    let intensity = attr("Intensity", "value")?;
    let outer = attr("ConeAngle", "value").unwrap_or(360.0);
    // (Type 0 files with a cone angle are cone lamps in the original's table as well: 196 lamps of the
    // Intensity 30 / 40 types read out of it - LIGHTING_WORK.md "The lamp slots' order")
    let cone = if outer >= 359.0 {
        [0.0, 1.0]
    } else {
        // lit from the inner cone (ConeAngle + Penumbra, the penumbra being negative) out to
        // nothing at ConeAngle
        let inner = (outer + attr("Penumbra", "value").unwrap_or(0.0)).clamp(1.0, outer);
        let (co, ci) = ((outer.to_radians() * 0.5).cos(), (inner.to_radians() * 0.5).cos());
        let scale = 1.0 / (ci - co).max(1e-3);
        [scale, -co * scale]
    };
    Some((color, intensity, cone))
}

/// The glow sprites at the lit lamps of the signal heads (the original's F-79347;
/// work/map/signal_rule.md, "Glows in the port").
#[derive(Default)]
struct SignalGlows {
    /// this frame's instances of the glow pipeline
    buf: Option<wgpu::Buffer>,
    count: u32,
    /// per head (as `Props::heads`): the locators `red`, `red1` / `yellow`, `yellow1` /
    /// `green`, `green1` of its type, in the head's frame
    lamps: Vec<[[Option<Vec3>; 2]; 3]>,
    /// `sm_fSignalGlowSizes` and `sm_fSignalGlowIntensitys` of tune/lighting/GlowTuning.xml
    sizes: [f32; 4],
    intensities: [f32; 4],
    /// this frame's lit lamps for the glow pass after the tone map (halo.rs): the locator's
    /// place, the head's z axis, the colour, the head (index into `Props::heads`)
    lit: Vec<(Vec3, Vec3, [f32; 3], u32)>,
    /// per head: what the game's prop list tests (halo::ListView::holds) - its box centre and half
    /// diagonal (prop + 96, the box round its collision parts, as for the lamp props), its drawable's
    /// radius - and its box for the occlusion query
    cull: Vec<(Vec3, f32, f32, (Vec3, Vec3))>,
}

pub struct Props {
    /// a hedge's high-level model -> its low level (the drawable's second LOD slot; kind 5)
    hedge_low: HashMap<usize, usize>,
    /// the plants' impostors: a picture per type, redrawn as the camera turns (props/impostor.rs)
    impostors: Option<impostor::Impostors>,
    /// the signal lamps' glows (rebuilt every frame)
    signal_glows: SignalGlows,
    /// light glow instances (pos + size, colour + intensity)
    glow_buf: Option<wgpu::Buffer>,
    glow_count: u32,
    pub lights: Vec<mcla_formats::rage::lights::CityLight>,
    /// lamps carried by props (street lights ...): see `PropLamp`
    pub prop_lamps: Vec<PropLamp>,
    /// every prop with a light file, its lamp's place and colour - the lights the cars'
    /// environment map shows as glows (VS_LightGlow); those of Intensity 0 are in this list only
    /// (third: the radius of the prop's bounding sphere x its scale - what its draw distance goes by)
    pub glow_lamps: Vec<(Vec3, Vec3, f32)>,
    /// ... and, in step with it, the prop's box as its draw is culled by (place +- radius x scale)
    pub glow_boxes: Vec<(Vec3, Vec3)>,
    /// ... and what the game's lamp list goes by (halo::ListView): the prop's box centre, the
    /// box's half diagonal, its drawable's radius, the prop's lamp place (all its lights' mean:
    /// the key of its on / off minute, as `prop_lamps` has it, and of its grid cell)
    pub glow_cull: Vec<(Vec3, f32, f32, Vec3)>,
    light_grid: HashMap<(i32, i32), Vec<u32>>,
    models: Vec<PropModel>,
    inst_buf: Option<wgpu::Buffer>,
    cells: HashMap<(i32, i32), Vec<Run>>,
    pub instances: usize,
    /// CPU copy of the instance buffer (knocked props are rewritten)
    insts: Vec<Inst>,
    colliders: Vec<Collider>,
    /// gas stations: the middle of each group of pumps and its reach (m)
    pub gas_stations: Vec<(Vec3, f32)>,
    col_grid: HashMap<(i32, i32), Vec<u32>>,
    debris: Vec<Debris>,
    /// traffic signal heads and the junctions they belong to
    pub heads: Vec<crate::signals::SignalHead>,
    pub signals: crate::signals::Signals,
    /// prop impacts of this frame (for the sound driver)
    pub hits: Vec<crate::sound::PropHit>,
    /// the effects the props' events start this tick (the type collision sets of the posts, the deflection pass;
    /// propfx.rs plays them)
    pub fx: Vec<crate::physics::loose::FxPlay>,
    /// the colliders whose deflection event has played in this pass (prop +88 = 0; restored when the car is out of reach)
    defl_done: std::collections::HashSet<u32>,
    /// props knocked over so far (stat)
    pub knocked: u32,
    rng: u32,
    /// the loose props are the cars' physics level's (physics/loose.rs): this file's knock leaves them be,
    /// and they are drawn where their bodies are (`follow_level`); the collider each level instance is
    level_on: bool,
    level_map: Option<HashMap<(i32, i32, i32), u32>>,
    /// props a race script may place (ramps, flares): model and its triangles
    race_models: HashMap<String, (usize, Vec<[Vec3; 3]>)>,
    race_buf: Option<wgpu::Buffer>,
    /// (model, instance) of the props placed for the running race
    race_runs: Vec<(usize, u32)>,
    /// the hangouts' barriers (`HANGOUT_BARRIERS`), their instances, and which are out (bit mask)
    barriers: Vec<Barrier>,
    barrier_buf: Option<wgpu::Buffer>,
    barrier_on: u16,
    /// the particle props (`la_prop_pt_*`: fountains, steam, smoke): place, stand-in kind
    /// (`emitter_kind`), turn, and the effects of the disc's list the type keeps running
    /// (joaat(name), place in the prop's frame). Kind `EMIT_RAIL`: a segment of a wind-debris
    /// rail instead - from the place to the one entry's point, with the effect of its group.
    pub emitters: Vec<(Vec3, u8, Quat, Vec<(u32, Vec3)>)>,
    /// the wind-debris streets: segments of the rails of `city/sc/0xad0a8b94.r66` (`trash.xrn`)
    pub trash: Vec<(Vec3, Vec3)>,
}

/// `trash.xrn` (root class 0x47b4cc like the road network): 98 short rails in four groups laid
/// along streets and alleys. While the player's car is within 20 m of one the original runs
/// that group's litter effect on the car (`wind_debris_hw` / `_la` / `_beach`; F-52018).
/// Vertices: root +2c, vec4 each; a rail (class 0x479d24, 0x60 bytes): +4c u16 points,
/// +50 -> u16 vertex indices. Returns the rails as segments.
fn trash_rails(res: &mcla_formats::rsc::Resource) -> Vec<(Vec3, Vec3)> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(verts) = rd.ptr(0x2c) else { return out };
    let v = |i: usize| Vec3::new(rd.f32(verts + i * 16), rd.f32(verts + i * 16 + 4), rd.f32(verts + i * 16 + 8));
    let mut o = 0;
    while o + 0x60 <= res.virtual_size {
        if rd.vtable(o) == 0x0047_9d24 {
            let n = rd.u16(o + 0x4c) as usize;
            if let Some(idx) = rd.ptr(o + 0x50).filter(|p| n >= 2 && n < 256 && rd.has(*p, n * 2)) {
                let pts: Vec<Vec3> = (0..n).map(|k| v(rd.u16(idx + k * 2) as usize)).filter(|p| p.is_finite()).collect();
                out.extend(pts.windows(2).map(|w| (w[0], w[1])));
            }
            o += 0x60;
        } else {
            o += 4;
        }
    }
    out
}

/// Collider slot of the props a race places.
pub const RACE_PROP_ID: usize = usize::MAX - 77;
/// Collider slot of the hangouts' barriers.
pub const BARRIER_ID: usize = usize::MAX - 78;

/// The barriers of the hangouts: wall pieces 7 to 14 m long that no instance of the prop file
/// places. Each `Hangouts/<district>_*_hangout*` script hands one prop name and one position (no
/// rotation) to `Game/Hangouts/wager_hangout_core` or `local_hangout_core`. The core creates the
/// prop detached (PropManager_CreateRaceProp does not attach) and, in its loop, attaches it while
/// `PoliceManager_CheckPoliceActivity(1) > 1` and detaches it when that drops: the barrier shuts
/// the lot while the player is chased and is absent otherwise. The local core also attaches it
/// while the player is in the hangout (work/map/behaviour_rule.md Q2). MCLA_NO_BARRIERS leaves
/// them out, MCLA_BARRIERS_UP=1 keeps them standing.
const HANGOUT_BARRIERS: [(&str, [f32; 3]); 12] = [
    ("la_prop_static_barrier_bh_blk_10a", [-466.012, 2.1227, -176.8271]),
    ("la_prop_static_barrier_smc_11b", [-2442.6299, -9.6608, 327.7095]),
    ("la_prop_static_barrier_smc_blk_04x", [-2750.8259, -4.5744, -204.3996]),
    ("sc_prop_static_barrier_bh_hangout_01x", [-1388.5797, -8.5946, 1114.6421]),
    ("la_prop_static_barrier_wh_blk_08x", [724.1904, 7.4532, -282.1239]),
    ("la_prop_static_barrier_hl_ml_blk_11x", [686.5153, 76.0312, -1511.327]),
    ("la_prop_static_barrier_dt_hangout_local", [2099.6404, -2.9968, -259.8941]),
    ("la_prop_static_barrier_dt_sp_blk_02x", [1843.2139, -9.9915, 610.0057]),
    ("la_prop_static_barrier_cc_westfieldmall_01x", [-1687.093, -5.3118, 258.924]),
    ("la_prop_static_barrier_wh_blk_05x", [-140.6556, 6.8605, -466.5895]),
    ("la_prop_static_barrier_dt_sp_blk_02x", [1382.6316, -13.1516, 1220.5601]),
    ("la_prop_static_barrier_hl_va_blk_05x", [-1003.1, 30.0142, -1853.6]),
];
/// The barriers handed to `local_hangout_core` (the other five go to `wager_hangout_core`).
const LOCAL_BARRIERS: [&str; 7] = ["bh_blk_10a", "smc_11b", "wh_blk_08x", "dt_hangout_local", "westfieldmall", "hl_va_blk_05x", "bh_hangout_01x"];

struct Barrier {
    model: usize,
    pos: Vec3,
    /// of a local hangout: also out while the player is in the hangout
    local: bool,
    tris: Vec<[Vec3; 3]>,
}

const RACE_PROPS: [&str; 6] = ["la_prop_trailer_ramp_01x", "la_prop_trailer_ramp_02x", "la_prop_flare_01x", "la_prop_construction_cone_01x", "la_prop_bench_02x", "la_prop_col_barrel_01x"];

const CELL: f32 = 96.0;
/// The original fades a prop out over `FADE_BAND` metres from `FADE_PER_RADIUS` times the radius
/// of its drawable's bounding sphere (gPropFadeInfo in a recorded frame: sign, radius 1.501,
/// 532.9 .. 540.9 m; dumpster 1.18, 417.4; pedestrian light 1987 m - MAP_WORK.md B3).
/// That is at the 25 degree lens of a standing car; the factor goes with the lens (recorded 355
/// at 25.0 degrees, 319 at 27.4, 252 at 34.8). The rule itself is `PropModel::reach`.
/// The camera's vertical lens in radians, as bits (25 degrees until told).
static LENS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3edf_66f3);

/// Tell the props the camera's vertical lens (radians); call when it changes.
pub fn set_lens(fov_y: f32) {
    LENS.store(fov_y.clamp(0.1, 2.5).to_bits(), std::sync::atomic::Ordering::Relaxed);
}

pub fn lens() -> f32 {
    f32::from_bits(LENS.load(std::sync::atomic::Ordering::Relaxed))
}
/// Props seen further than this are kept in the coarse grid (`FAR_CELL`), the others in the
/// fine one (`CELL`).
const NEAR_DIST: f32 = 440.0;
const FAR_CELL: f32 = 384.0;
/// nothing is drawn beyond this: the main view's far clip, 6000 (0x82037480, set with the near 0.2
/// by F-82384; recorded 5994 .. 6000) - the original's tree cards would reach 3 - 11 km
const FAR_DIST: f32 = 6000.0;
/// coarse cells live in the same map as the fine ones, moved out of their way
const FAR_KEY: i32 = 1 << 20;

fn cell_of(p: Vec3) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.z / CELL).floor() as i32)
}

fn far_cell_of(p: Vec3) -> (i32, i32) {
    ((p.x / FAR_CELL).floor() as i32 + FAR_KEY, (p.z / FAR_CELL).floor() as i32)
}

/// Level distance from `p` to a box.
fn dist_to_box(p: Vec3, mn: Vec3, mx: Vec3) -> f32 {
    let c = p.clamp(mn, mx);
    Vec3::new(c.x - p.x, 0.0, c.z - p.z).length()
}

/// A prop type's footprint from its own collision shape (`Prop::parts`): posts stand fast with
/// the radius of their inner capsule, everything else is the box round its parts with the
/// type's real mass. None where the type has no shape.
///
/// `placed`: the kind its placed instances carry (`PropInstance::kind`: 1 loose, 2 fixed, 3 glow
/// card); for a type nothing places, its name decides as before.
fn physics_foot(prop: &mcla_formats::rage::props::Prop, placed: Option<u32>) -> Option<Foot> {
    let name = prop.name.to_ascii_lowercase();
    let snd = crate::sound::prop_sound(&name);
    if let Some(p) = prop.post() {
        let r = ((p.max.x - p.min.x) * 0.5).max(0.04);
        let y0 = p.min.y.max(0.0);
        return Some(Foot { kind: Solidity::Solid, center: Vec2::new((p.min.x + p.max.x) * 0.5, (p.min.z + p.max.z) * 0.5), half: Vec2::splat(r), y0, height: (p.max.y - y0).max(0.1), mass: prop.mass, snd, audio: mcla_formats::hash::joaat(&name), wide: (prop.parts[0].max.x - prop.parts[0].min.x) * 0.5, fixed: placed != Some(1) });
    }
    if prop.parts.is_empty() {
        return None;
    }
    let lo = prop.parts.iter().map(|p| p.min.y).fold(f32::INFINITY, f32::min);
    let top = prop.parts.iter().map(|p| p.max.y).fold(f32::NEG_INFINITY, f32::max);
    // what a car's body meets: the parts that reach down to within 1.2 m of the ground
    let (mut mn, mut mx) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for p in prop.parts.iter().filter(|p| p.min.y < lo.max(0.0) + 1.2) {
        mn = mn.min(Vec2::new(p.min.x, p.min.z));
        mx = mx.max(Vec2::new(p.max.x, p.max.z));
    }
    if !mn.is_finite() || !mx.is_finite() {
        return None;
    }
    // (the hangout barriers collide with their own triangles: some are not a straight wall)
    // (the file's own state word decides - instance +80, the same for every instance of a type:
    // 1 loose and breakable, 2 fixed, 3 a glow card. Glass panes are loose, 10 - 50 kg: a car
    // knocks them out. A loose prop no car can move - the parked helicopters, 18 t - stands.)
    let kind = if name.contains("flare") || name.contains("static_barrier") || prop.mass <= 0.0 {
        Solidity::None
    } else {
        match placed {
            Some(2) => Solidity::Solid,
            Some(1) if prop.mass > 5000.0 => Solidity::Solid,
            Some(1) => Solidity::Dynamic,
            Some(_) => Solidity::None,
            None if name.contains("_static_") || name.contains("trailer_ramp") => Solidity::Solid,
            None => Solidity::Dynamic,
        }
    };
    let y0 = lo.max(0.0);
    Some(Foot { kind, center: (mn + mx) * 0.5, half: ((mx - mn) * 0.5).max(Vec2::splat(0.04)), y0, height: (top - y0).max(0.1), mass: prop.mass.max(1.0), snd, audio: mcla_formats::hash::joaat(&name), wide: 0.0, fixed: placed == Some(2) || (placed.is_none() && kind == Solidity::Solid) })
}

/// The particle props have no model: each stands for effects of the disc's effect list
/// (`rmptfx/fxlists/MC4.r107`), named by the continuous events of its fragment type
/// (`ptfx::prop_effects`) and played from the effect rules by `Props::emit`. Returned for a
/// particle prop: the kind of the port's own stand-in (used when a rule cannot be played, or
/// with `MCLA_FX_STANDIN=1`) - 0-9 a fountain whose jet rises about (kind + 1) / 2 metres, 10
/// rings on the water, 11 a waterfall, 12 steam from a grate, 13 a smoke stack - and the effect
/// its name stands for (used only for a type whose file names none: no type of the disc).
fn emitter_kind(name: &str) -> Option<(u8, &'static str)> {
    let n = name.to_ascii_lowercase();
    let n = n.strip_prefix("la_prop_pt_")?;
    Some(if n.contains("sewergrate") {
        (12, "amb_steam_grate")
    } else if n.contains("smokestack") {
        (13, "amb_smoke_stack")
    } else if n.contains("waterfall") {
        (11, "water_waterfall")
    } else if n.contains("water_rings") {
        (10, "water_rings_b")
    } else if n.contains("beverly") {
        (9, "water_fountain_beverly")
    } else if n.contains("fountain_cc") {
        (8, "water_fountain_cc_m")
    } else if n.contains("skinny") {
        (7, "water_fountain_skinny")
    } else if n.contains("small") {
        (1, "water_fountain_small")
    } else if n.contains("promenade_b") {
        (3, "water_fountain_promenade_b")
    } else if n.contains("promenade") {
        (3, "water_fountain_promenade")
    } else if n.contains("rodeo") {
        (5, "water_fountain_rodeo")
    } else if n.contains("multi") {
        (5, "water_fountain_multi")
    } else if n.contains("mac") {
        (5, "water_fountain_mac")
    } else {
        (5, "water_fountain")
    })
}

/// Kind of an entry of `Props::emitters` that is a segment of a wind-debris rail.
const EMIT_RAIL: u8 = 255;

fn prop_mode(shader: &str) -> MatMode {
    let s = shader.to_ascii_lowercase();
    if s.contains("foliage") && !s.contains("trunk") {
        MatMode::Foliage
    } else if s.contains("alpha") || s.contains("fence") || s.contains("cutout") {
        MatMode::AlphaTest
    } else if s.contains("trafficsignal") {
        MatMode::Signal
    } else if s.contains("propglass") {
        // (the glass props' own effect, xPropGlass: door leaves, glass walls - city.wgsl mode 26)
        MatMode::PropGlass
    } else if s.contains("glass") {
        MatMode::BlackGloss
    } else {
        MatMode::Object
    }
}

impl Props {
    pub fn empty() -> Props {
        Props { hedge_low: HashMap::new(), impostors: None, signal_glows: SignalGlows::default(), glow_buf: None, glow_count: 0, lights: Vec::new(), prop_lamps: Vec::new(), glow_lamps: Vec::new(), glow_cull: Vec::new(), glow_boxes: Vec::new(), light_grid: HashMap::new(), models: Vec::new(), inst_buf: None, cells: HashMap::new(), instances: 0, insts: Vec::new(), colliders: Vec::new(), gas_stations: Vec::new(), col_grid: HashMap::new(), debris: Vec::new(), hits: Vec::new(), fx: Vec::new(), defl_done: Default::default(), knocked: 0, rng: 12345, level_on: false, level_map: None, heads: Vec::new(), signals: Default::default(), race_models: HashMap::new(), race_buf: None, race_runs: Vec::new(), barriers: Vec::new(), barrier_buf: None, barrier_on: 0, emitters: Vec::new(), trash: Vec::new() }
    }

    pub fn load(gpu: &mut Gpu, world: &mut World, data_dir: &str) -> Props {
        let dir = Path::new(data_dir).join("resources/prop");
        crate::fillmap::init(data_dir);
        let (Ok(pres), Ok(fres)) = (load_resource_file(&dir.join("0x0e66973c.r131")), load_resource_file(&dir.join("0x982ff5e0.r134"))) else {
            log::warn!("props: resources missing");
            return Props::empty();
        };
        let tex_res = load_resource_file(&dir.join("0xf7b7d95f.r1")).ok();
        let tex_dict = tex_res.as_ref().and_then(|r| Dictionary::parse(&Rd::new(r), 0).ok());
        let mut texture = |gpu: &mut Gpu, world: &mut World, name: &str| -> (Arc<GpuTexture>, u32) {
            let h = mcla_formats::hash::joaat(&name.to_ascii_lowercase());
            // (2026-10-15: the prop dictionary's OWN textures keep to the props. The GPU's texture
            // cache goes by the name's hash alone, and the prop dictionary carries textures named
            // like the city's with other content - its `l_sha_gfxlight2_lit.dds` has a mean of
            // 179 where the city dictionary's has 92: uploaded first under the bare hash it lit the
            // city's wall lamps (CityNeon batches of that atlas) five times too bright, each with a
            // bloom halo. Their keys get a mark of their own, as the garage sets' do in world.rs.
            // MCLA_PROP_TEX_SHARED=1: as before.)
            const PROP_TEX_KEY: u32 = 0x5052_4f50;
            let own = std::env::var("MCLA_PROP_TEX_SHARED").is_err() && tex_dict.as_ref().map_or(false, |d| d.get(h).is_some());
            let hk = if own { h ^ PROP_TEX_KEY } else { h };
            if let Some(t) = gpu.textures.get(&hk) {
                return (t.clone(), hk);
            }
            if let (Some(r), Some(d)) = (tex_res.as_ref(), tex_dict.as_ref()) {
                if let Some(o) = d.get(h) {
                    if let Ok(t) = Texture::parse(&Rd::new(r), o) {
                        if log::log_enabled!(log::Level::Debug) {
                            log::debug!("prop tex {} {}x{} {:?} means {:?}", name, t.width, t.height, t.format, t.level_means());
                        }
                        return (gpu.upload_texture(hk, &t), hk);
                    }
                }
            }
            if let Some(t) = world.texture_any(gpu, h) {
                return (t, h);
            }
            log::debug!("prop texture miss: {}", name);
            (gpu.white.clone(), 0)
        };
        let mut all = parse_props(&pres);
        // the hedges' low level (their drawable's second LOD slot) as a prop of its own
        let hedge_low_of: HashMap<usize, usize> = {
            let mut m = HashMap::new();
            for i in 0..all.len() {
                if all[i].name.to_ascii_lowercase().contains("hedge") && !all[i].lod1.is_empty() {
                    let p = &all[i];
                    let low = mcla_formats::rage::props::Prop { off: p.off, name: format!("{}#low", p.name), hash: p.hash, meshes: p.lod1.clone(), lod1: Vec::new(), shaders: p.shaders.clone(), min: p.min, max: p.max, center: p.center, radius: p.radius, mass: 0.0, parts: Vec::new(), locators: Vec::new() };
                    all.push(low);
                    m.insert(i, all.len() - 1);
                }
            }
            m
        };
        let mut hedge_low: HashMap<usize, usize> = HashMap::new();
        let by_name: HashMap<String, usize> = all.iter().enumerate().map(|(i, p)| (p.name.to_ascii_lowercase(), i)).collect();
        let foliage = parse_foliage(&fres);
        let mut models: Vec<PropModel> = Vec::new();
        let mut model_of: HashMap<usize, Option<usize>> = HashMap::new();
        let mut insts: Vec<Inst> = Vec::new();
        let mut cells: HashMap<(i32, i32), Vec<Run>> = HashMap::new();
        let mut total = 0usize;
        // GPU model of prop `pi` (built on first use)
        let mut model_for = |gpu: &mut Gpu, world: &mut World, models: &mut Vec<PropModel>, pi: usize| -> Option<usize> {
            if let Some(m) = model_of.get(&pi) {
                return *m;
            }
            let prop = &all[pi];
            // diagnostics: MCLA_PROP_HIDE=<name part> leaves a kind of prop out
            if std::env::var("MCLA_PROP_HIDE").map_or(false, |h| prop.name.to_ascii_lowercase().contains(&h.to_ascii_lowercase())) {
                model_of.insert(pi, None);
                return None;
            }
            let mut batches = Vec::new();
            for (mesh, si) in &prop.meshes {
                let sh = prop.shaders.get(*si as usize);
                let sname = sh.map(|s| s.name.as_str()).unwrap_or("");
                let tname = sh.and_then(|s| s.textures().next().map(|x| x.1.to_string()));
                let (tex, k) = match tname {
                    Some(n) => texture(gpu, world, &n),
                    None => (gpu.white.clone(), 0),
                };
                // Lamp glass and glow cards (PSPropLampGlassTint): the glass texture by day; from
                // the material's OnTime to its OffTime the LitUp texture times NightIntensity,
                // drawn opaque (recorded at 23:30: blend off, NightIntensity 2, 18 h to 6 h).
                // The sign material does just that; its terms ride in the vertices' uv2 as
                // world::mark_window packs them for signs.
                let lamp = sh.filter(|s| s.name.to_ascii_lowercase().contains("lampglass")).and_then(|s| {
                    use mcla_formats::rage::drawable::ShaderParam;
                    let lit = s.textures().find(|(h, _)| *h == crate::world::ph::LITUP)?.1.to_string();
                    let f = |h: u32, d: f32| {
                        s.params.iter().find_map(|(ph, p)| match p {
                            ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0].x),
                            _ => None,
                        }).unwrap_or(d)
                    };
                    let q = |x: f32| (x * 8.0).round().clamp(0.0, 255.0);
                    // (OnTime 0xeec362e8, OffTime 0xd60eac7d, NightIntensity 0xcebc7801)
                    Some((lit, [q(f(0xeec362e8, 18.0)).max(1.0) + q(f(0xd60eac7d, 6.0)) * 256.0 + q(f(0xcebc7801, 2.0)) * 65536.0, 9.0 + 8192.0]))
                });
                let lamp = lamp.map(|(name, uv2)| (texture(gpu, world, &name), uv2));
                let uv2 = lamp.as_ref().map_or([0.0, 0.0], |l| l.1);
                // (glass props, PSPropGlass: the material's Transparency - the pane's alpha, c100 - and
                // ReflectionIntensity, c101, ride in uv2; 0.4 / 0.5 on the door leaves of Grand Ave.)
                let uv2 = if prop_mode(sname) == MatMode::PropGlass {
                    use mcla_formats::rage::drawable::ShaderParam;
                    let f = |h: u32, d: f32| {
                        sh.and_then(|s| s.params.iter().find_map(|(ph, p)| match p {
                            ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0].x),
                            _ => None,
                        })).unwrap_or(d)
                    };
                    if std::env::var("MCLA_PROP_HL_DUMP").is_ok() {
                        eprintln!("prop glass {}: Transparency {} ReflectionIntensity {}", prop.name, f(0x98d13f30, -1.0), f(0x73e4f90c, -1.0));
                    }
                    [f(0x98d13f30, 0.4).clamp(0.0, 1.0), f(0x73e4f90c, 0.5)]
                } else {
                    uv2
                };
                // (2026-10-14: the sun highlight of PSPropDefault / PSPropBumpSpec - the material's
                // SpecularExponent, SpecularIntensity and SpecularColorMixer, in the first colour word as
                // exponent, hundredths, hundredths (alpha stays 0); the program's HeightSpecularSampler as
                // the batch's third texture. city.wgsl `prop_hl`.)
                let prop_hl: [u8; 4] = {
                    use mcla_formats::rage::drawable::ShaderParam;
                    let f = |h: u32, d: f32| {
                        sh.and_then(|s| s.params.iter().find_map(|(ph, p)| match p {
                            ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0].x),
                            _ => None,
                        })).unwrap_or(d)
                    };
                    [f(0x93d520e8, 8.0).round().clamp(1.0, 255.0) as u8, (f(0xa95fc535, 0.33) * 100.0).round().clamp(0.0, 255.0) as u8, (f(0xbaf643f4, 0.7) * 100.0).round().clamp(0.0, 100.0) as u8,
                     // (bit 1 of the alpha: the material has a height / specular map - the bump-spec programs; city.wgsl `prop_nb`)
                     if sh.map_or(false, |s| s.textures().any(|(h, _)| h == crate::world::ph::HEIGHT_SPEC)) { 2 } else { 0 }]
                };
                let prop_bump = sh.and_then(|s| s.textures().find(|(h, _)| *h == crate::world::ph::HEIGHT_SPEC).map(|x| x.1.to_string()));
                if std::env::var("MCLA_PROP_HL_DUMP").is_ok() {
                    eprintln!("prop hl {} / {}: {:?} bump {:?} textures {:?}", prop.name, sname, prop_hl, prop_bump, sh.map(|s| s.textures().map(|(h, n)| format!("{:08x} {}", h, n)).collect::<Vec<_>>()));
                }
                // (for the fade with distance, city.wgsl vs_inst: the sphere's radius and 100 + kind
                // - 0 prop, 1 plant, 1.25 a plant's trunk, 2 hedge, 2.25 a hedge's low level - in the second UV set, which
                // only the signal heads use)
                let lname = prop.name.to_ascii_lowercase();
                let fade_uv = if sname.to_ascii_lowercase().contains("trafficsignal") || prop.radius <= 0.0 {
                    None
                } else {
                    Some([prop.radius, 100.0 + if !lname.contains("_gfx_") { 0.0 } else if lname.ends_with("#low") { 2.25 } else if lname.contains("hedge") { 2.0 } else if sname.to_ascii_lowercase().contains("trunk") { 1.25 } else { 1.0 }])
                };
                if std::env::var("MCLA_LEAF_DUMP").is_ok() && lname.contains("_gfx_") && !mesh.colors.is_empty() {
                    let m = mesh.colors.len() as f32;
                    let mean: Vec<f32> = (0..4).map(|k| mesh.colors.iter().map(|c| c[k] as f32).sum::<f32>() / m).collect();
                    eprintln!("leaf colours {}: {} vertices, byte means {:.0} {:.0} {:.0} {:.0}, first {:?}", prop.name, mesh.colors.len(), mean[0], mean[1], mean[2], mean[3], mesh.colors[0]);
                }
                let n = mesh.positions.len();
                let mut verts = Vec::with_capacity(n);
                for i in 0..n {
                    let nrm = mesh.normals.get(i).copied().unwrap_or(Vec3::Y);
                    let nrm = if nrm.is_finite() && nrm.length_squared() > 1e-4 { nrm.normalize() } else { Vec3::Y };
                    let uv0 = mesh.uv0.get(i).copied().unwrap_or_default();
                    // (plants: the leaves' baked occlusion. VSPropFoliageGenImpostorDay - and the leaf programs -
                    // multiply the light by the SECOND byte of the vertex's packed word (the fetch's swizzle
                    // 0x4c1 hands the program w = that byte; the other three are one value, the sway's): first
                    // three are the sway vector: 0.75 x that x (ambient + light x (0.1 + 0.9 n.l)). It was
                    // taken as 1 here. MCLA_LEAF_OCC=0: as before.)
                    let leaf_byte: Option<usize> = match std::env::var("MCLA_LEAF_OCC").ok().as_deref() { Some("0") => None, Some("b1") => Some(1), Some("b2") => Some(2), Some("b3") => Some(3), Some("b0") => Some(0), _ => Some(1) };
                    let leaf_occ = match leaf_byte { Some(k) if lname.contains("_gfx_") => mesh.colors.get(i).map_or(255, |c| c[k]), _ => 255 };
                    verts.push(Vertex { pos: mesh.positions[i].to_array(), normal: nrm.to_array(), uv0: uv0.to_array(), uv1: fade_uv.unwrap_or(uv0.to_array()), color: if prop_mode(sname) == MatMode::Object { prop_hl } else { [255, 255, 255, 0] }, color1: { let sw = |k: usize| match leaf_byte { Some(_) if lname.contains("_gfx_") => mesh.colors.get(i).map_or(0, |c| c[k]), _ => 0 }; [sw(0), sw(2), sw(3), leaf_occ] }, uv2 });
                }
                // (the binormal's sign of the bump-spec props, in the normal's length: world::mark_binormal)
                if let Some(s) = sh { crate::world::mark_binormal(&mut verts, &mesh.tangents, s); }
                // (diagnostics MCLA_PROP_TAN_DUMP: per bump-spec batch, triangles by the stored tangent's w, by the cosine of
                // the stored tangent against the direction u grows in - both laid into the face -, and by the sign of
                // (cross(T, N) x w) . dP/dv: what city.wgsl's frame, rebuilt from derivatives, takes for granted)
                if std::env::var("MCLA_PROP_TAN_DUMP").is_ok() && prop_bump.is_some() && mesh.tangents.len() == n {
                    let (mut wc, mut cc, mut bc) = ([0u32; 3], [0u32; 5], [0u32; 3]);
                    for tri in mesh.indices.chunks_exact(3) {
                        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
                        if i0 >= n || i1 >= n || i2 >= n || mesh.uv0.len() < n || mesh.normals.len() < n { continue; }
                        let (e1, e2) = (mesh.positions[i1] - mesh.positions[i0], mesh.positions[i2] - mesh.positions[i0]);
                        let (d1, d2) = (mesh.uv0[i1] - mesh.uv0[i0], mesh.uv0[i2] - mesh.uv0[i0]);
                        let det = d1.x * d2.y - d2.x * d1.y;
                        let nn = (mesh.normals[i0] + mesh.normals[i1] + mesh.normals[i2]).normalize_or_zero();
                        let tw = mesh.tangents[i0].w;
                        wc[if tw >= 1.5 { 1 } else if tw < 0.5 { 2 } else { 0 }] += 1;
                        if det.abs() < 1e-12 || nn == Vec3::ZERO { cc[4] += 1; continue; }
                        let (pu, pv) = ((e1 * d2.y - e2 * d1.y) / det, (e2 * d1.x - e1 * d2.x) / det);
                        let ts = (mesh.tangents[i0].truncate() + mesh.tangents[i1].truncate() + mesh.tangents[i2].truncate()).normalize_or_zero();
                        let c = (ts - nn * ts.dot(nn)).normalize_or_zero().dot((pu - nn * pu.dot(nn)).normalize_or_zero());
                        cc[if c > 0.9 { 0 } else if c > 0.7 { 1 } else if c > 0.0 { 2 } else { 3 }] += 1;
                        let sgn = if tw >= 1.5 { -1.0 } else if tw < 0.5 { 0.0 } else { 1.0 };
                        let bd = (ts.cross(nn) * sgn).dot(pv.normalize_or_zero());
                        bc[if bd < -0.2 { 0 } else if bd > 0.2 { 2 } else { 1 }] += 1;
                    }
                    eprintln!("prop tan {} / {} map {:?}: w +1/-1/0 {:?} cos(T, dP/du) >0.9 / >0.7 / >0 / <0 / degenerate {:?} B.dP/dv - / 0 / + {:?}", prop.name, sname, prop_bump, wc, cc, bc);
                }
                if verts.is_empty() || mesh.indices.is_empty() {
                    continue;
                }
                let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("prop vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
                let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("prop ib"), contents: bytemuck::cast_slice(&mesh.indices), usage: wgpu::BufferUsages::INDEX });
                let (bind, mode) = match &lamp {
                    Some(((lit, lk), _)) if *lk != 0 => (gpu.texture_bind_group(&tex, lit, k, *lk), MatMode::StoreSign),
                    _ => match (&prop_bump, prop_mode(sname)) {
                        (Some(bn), MatMode::Object) => {
                            // (read as stored, without the gamma decoding - the fetch constant asks for
                            // none, as on the walls' maps: a raw copy under its own key, world.rs RAW_KEY.
                            // The sign's l_prop_signs01_bs is a flat 128 195 128 128 in the running game.)
                            let bh = mcla_formats::hash::joaat(&bn.to_ascii_lowercase());
                            let bt = match (tex_res.as_ref(), tex_dict.as_ref()) {
                                (Some(r), Some(d)) => d.get(bh).and_then(|o| Texture::parse(&Rd::new(r), o).ok()).map(|t| gpu.upload_texture_raw(bh ^ 0x4853_5241, &t)),
                                _ => None,
                            };
                            match bt {
                                Some(bt) => (gpu.texture_bind_group3(&tex, &tex, &bt, k, k, bh ^ 0x4853_5241), MatMode::Object),
                                None => (gpu.texture_bind_group(&tex, &tex, k, k), MatMode::Object),
                            }
                        }
                        (_, MatMode::PropGlass) => {
                            // (what the pane mirrors: the game's fixed pictures `__WindowReflection` - ReflectSphereDay.dds
                            // by day (its ReflectionSampler; the same texture the shop panes' draws bind), ReflectSphereNight.dds
                            // under the night program - second and third texture, as on the shop panes)
                            let (hd, hn) = (mcla_formats::hash::joaat("reflectsphereday.dds"), mcla_formats::hash::joaat("reflectspherenight.dds"));
                            let day = world.texture_any(gpu, hd).unwrap_or_else(|| gpu.white.clone());
                            let night = world.texture_any(gpu, hn).unwrap_or_else(|| gpu.white.clone());
                            (gpu.texture_bind_group3(&tex, &day, &night, k, hd, hn), MatMode::PropGlass)
                        }
                        _ => (gpu.texture_bind_group(&tex, &tex, k, k), prop_mode(sname)),
                    },
                };
                batches.push(PropBatch { vb, ib, count: mesh.indices.len() as u32, bind, mode });
            }
            let res = if batches.is_empty() {
                None
            } else {
                let radius = prop.min.abs().max(prop.max.abs()).length();
                let sphere = if prop.radius > 0.0 { prop.radius } else { radius * 0.33 };
                models.push(PropModel { batches, radius, sphere, kind: 0 });
                Some(models.len() - 1)
            };
            model_of.insert(pi, res);
            res
        };
        let dump: Option<(f32, f32, f32)> = std::env::var("MCLA_PROP_DUMP").ok().and_then(|v| {
            let f: Vec<f32> = v.split(",").filter_map(|x| x.parse().ok()).collect();
            if f.len() == 3 { Some((f[0], f[1], f[2])) } else { None }
        });
        let dump_at = |name: &str, p: Vec3, extra: &str| {
            if let Some((x, z, r)) = dump {
                if (p.x - x).abs() < r && (p.z - z).abs() < r {
                    log::info!("prop near: {} at {:?} {}", name, p, extra);
                }
            }
        };
        // per model: instances bucketed per cell
        let mut per_model: HashMap<usize, HashMap<(i32, i32), Vec<Inst>>> = HashMap::new();
        // Plants. Each stands as the original's world matrix has it: its y axis along the
        // instance's up vector (lean and height), x and z scaled by the width, turned about its
        // own axis only where the type has rotation steps (hedges).
        let mut n_plants = 0usize;
        let mut plant_models: hashbrown::HashSet<usize> = Default::default();
        // plant model -> the model of its far card (None: could not be made)
        let mut card_of: HashMap<usize, Option<usize>> = HashMap::new();
        // the plant types with a card, for their impostors (props/impostor.rs)
        let mut imp_types: Vec<impostor::TypeInfo> = Vec::new();
        for ft in &foliage {
            let base = ft.name.trim_end_matches("_displayAsLowLOD").to_ascii_lowercase();
            let Some(&pi) = by_name.get(&base) else { continue };
            let Some(mi) = model_for(gpu, world, &mut models, pi) else { continue };
            plant_models.insert(mi);
            // Far off the original draws a plant as a card with a picture of it; near, the model.
            if !card_of.contains_key(&mi) {
                let sphere = models[mi].sphere;
                let radius = models[mi].radius;
                let card = bake_card(&all[pi], &mut |name: &str| {
                    let h = mcla_formats::hash::joaat(&name.to_ascii_lowercase());
                    let t = match (tex_res.as_ref(), tex_dict.as_ref().and_then(|d| d.get(h))) {
                        (Some(r), Some(o)) => Texture::parse(&Rd::new(r), o).ok(),
                        _ => None,
                    }
                    .or_else(|| world.shared.get(h))?;
                    Some((t.width as usize, t.height as usize, t.to_rgba8()))
                })
                .map(|(verts, indices, image)| {
                    let mut verts = verts;
                    for v in verts.iter_mut() {
                        v.uv1 = [sphere, 103.0];
                    }
                    let key = mcla_formats::hash::joaat(&format!("card:{}", all[pi].name));
                    let tex = gpu.upload_texture(key, &image);
                    let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("plant card vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
                    let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("plant card ib"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
                    let bind = gpu.texture_bind_group(&tex, &tex, key, key);
                    models.push(PropModel { batches: vec![PropBatch { vb, ib, count: indices.len() as u32, bind, mode: MatMode::Foliage }], radius, sphere, kind: 3 });
                    models.len() - 1
                });
                // (hedges have no card: their model is drawn to its own, shorter, limit)
                let hedge = base.contains("hedge");
                let card = if hedge { None } else { card };
                card_of.insert(mi, card);
                if let Some(ci) = card {
                    imp_types.push(impostor::TypeInfo { model: mi, baked: ci, name: all[pi].name.clone(), min: all[pi].min, max: all[pi].max, sphere, radius });
                }
                models[mi].kind = if hedge { 2 } else if card.is_some() { 1 } else { 0 };
                if let Some(li) = hedge_low_of.get(&pi).and_then(|&lp| model_for(gpu, world, &mut models, lp)) {
                    models[li].kind = 5;
                    hedge_low.insert(mi, li);
                }
            }
            let card = card_of[&mi];
            // (the `_displayAsLowLOD` placements are never more than their card)
            let key = match card {
                Some(ci) if ft.name.ends_with("_displayAsLowLOD") => ci,
                _ => mi,
            };
            let far = card.is_some() || models[mi].reach(2.0, FOV_REST, Lod::ORIGINAL).1 > NEAR_DIST;
            let cellmap = per_model.entry(key).or_default();
            for it in &ft.instances {
                let (w, h) = (it.width, it.height());
                if w < 0.01 || h < 0.01 {
                    continue;
                }
                dump_at(&ft.name, it.pos, &format!("width {:.3} height {:.3} turn {}", w, h, it.rot));
                // (the sign of the quaternion tells the shader a card-only placement from a plant's far card)
                let rot = it.rotation(ft.rot_steps);
                let rot = if (key != mi) == (rot.w < 0.0) { rot } else { -rot };
                cellmap.entry(if far { far_cell_of(it.pos) } else { cell_of(it.pos) }).or_default().push(Inst { fill: crate::fillmap::plant(it.fill), lamp_pos: [it.lamp.pos.x, it.lamp.pos.y, it.lamp.pos.z, it.lamp.strength], lamp_col: [it.lamp.color[0], it.lamp.color[1], it.lamp.color[2], it.lamp.cone[0]], lamp_dir: [it.lamp.dir[0], it.lamp.dir[1], it.lamp.dir[2], it.lamp.cone[1]], ..Inst::placed(it.pos, rot, w, h) });
                n_plants += 1;
            }
        }
                // static props (street lamps, bins, fences, signs ...)
        let by_off: HashMap<usize, usize> = all.iter().enumerate().map(|(i, p)| (p.off, i)).collect();
        let placed = mcla_formats::rage::props::parse_prop_instances(&pres);
        let mut n_static = 0usize;
        let mut solid_plants: hashbrown::HashSet<(u32, u32)> = Default::default();
        let mut posts: Vec<(usize, Vec3, Quat)> = Vec::new();
        let mut unseen: Vec<(usize, Vec3, Quat)> = Vec::new();
        let mut placed_kind: HashMap<usize, u32> = HashMap::new();
        // (first the wind-debris rails with their group's effect, then the particle props)
        let mut emitters: Vec<(Vec3, u8, Quat, Vec<(u32, Vec3)>)> = load_resource_file(&Path::new(data_dir).join("resources/city/sc/0xad0a8b94.r66")).map(|r| mcla_formats::rage::ptfx::trash_rail_effects(&r)).unwrap_or_default().into_iter().map(|(a, b, fx)| (a, EMIT_RAIL, Quat::IDENTITY, vec![(fx, b)])).collect();
        let mut pumps: Vec<Vec<Vec3>> = Vec::new();
        for it in &placed {
            let Some(&pi) = by_off.get(&it.frag) else { continue };
            if all[pi].name.contains("gas_pump") {
                match pumps.iter_mut().find(|g| g.iter().any(|p| (*p - it.pos).length() < 45.0)) {
                    Some(g) => g.push(it.pos),
                    None => pumps.push(vec![it.pos]),
                }
            }
            // Plants are placed by the foliage file; their entry here makes them a physical
            // object (2,175 of the 95,523 plants: the street trees one can hit).
            if all[pi].name.contains("_gfx_") {
                solid_plants.insert((it.pos.x.to_bits(), it.pos.z.to_bits()));
                continue;
            }
            dump_at(&all[pi].name, it.pos, &format!("static rot {:.3?} box {:.2?} .. {:.2?}", it.rot.to_array(), all[pi].min.to_array(), all[pi].max.to_array()));
            placed_kind.insert(pi, it.kind);
            if let Some((k, by_name)) = emitter_kind(&all[pi].name) {
                // the type's continuous events name its effects; a type without any: by its name
                let mut fx: Vec<(u32, Vec3)> = mcla_formats::rage::ptfx::prop_effects(&pres, it.frag).into_iter().map(|(n, at)| (mcla_formats::hash::joaat(&n), at)).collect();
                if fx.is_empty() {
                    fx.push((mcla_formats::hash::joaat(by_name), Vec3::ZERO));
                }
                emitters.push((it.pos, k, it.rot, fx));
            }
            let Some(mi) = model_for(gpu, world, &mut models, pi) else {
                // no model: the unseen posts that stand where a sector draws a column or a pole
                // (651 of them) still stop a car
                if all[pi].meshes.is_empty() && all[pi].post().is_some() {
                    posts.push((pi, it.pos, it.rot));
                } else if all[pi].meshes.is_empty() && it.kind == 1 && !all[pi].parts.is_empty() {
                    // the box sets (`l_prop_boxset_01x` / `_02x`, 89 placed): nothing to draw on
                    // the disc, but the prop list's constructor inserts every instance into the
                    // physics level whatever its drawable - loose piles of 6 / 9 boxes a car
                    // scatters without seeing them (work/map/behaviour_rule.md Q3)
                    unseen.push((pi, it.pos, it.rot));
                }
                continue;
            };
            let cell = if models[mi].reach(1.0, FOV_REST, Lod::ORIGINAL).1 > NEAR_DIST { far_cell_of(it.pos) } else { cell_of(it.pos) };
            per_model.entry(mi).or_default().entry(cell).or_default().push(Inst { pos_scale: [it.pos.x, it.pos.y, it.pos.z, 1.0], rot_tint: [it.rot.x, it.rot.y, it.rot.z, it.rot.w], fill: crate::fillmap::prop(it.pos.x, it.pos.z), ..Inst::NONE });
            n_static += 1;
        }
        // the hangouts' barriers
        // (made here, put out by `set_barriers`)
        let mut barriers: Vec<Barrier> = Vec::new();
        if std::env::var("MCLA_NO_BARRIERS").is_err() {
            for (name, p) in HANGOUT_BARRIERS {
                let Some(&pi) = by_name.get(name) else { continue };
                let Some(mi) = model_for(gpu, world, &mut models, pi) else { continue };
                let pos = Vec3::from(p);
                dump_at(name, pos, "hangout barrier");
                let mut tris: Vec<[Vec3; 3]> = Vec::new();
                for (mesh, _) in &all[pi].meshes {
                    for t in mesh.indices.chunks_exact(3) {
                        let g = |k: usize| mesh.positions.get(t[k] as usize).copied();
                        if let (Some(a), Some(b), Some(c)) = (g(0), g(1), g(2)) {
                            tris.push([pos + a, pos + b, pos + c]);
                        }
                    }
                }
                barriers.push(Barrier { model: mi, pos, local: LOCAL_BARRIERS.iter().any(|n| name.contains(n)), tris });
            }
            log::info!("props: {} hangout barriers ({} of local hangouts), out only during a chase", barriers.len(), barriers.iter().filter(|b| b.local).count());
        }
        let barrier_buf = (!barriers.is_empty()).then(|| {
            let insts: Vec<Inst> = barriers.iter().map(|b| Inst { pos_scale: [b.pos.x, b.pos.y, b.pos.z, 1.0], rot_tint: [0.0, 0.0, 0.0, 1.0], fill: [0; 4], ..Inst::NONE }).collect();
            gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("hangout barriers"), contents: bytemuck::cast_slice(&insts), usage: wgpu::BufferUsages::VERTEX })
        });
        let gas_stations: Vec<(Vec3, f32)> = pumps.iter().map(|g| {
            let c = g.iter().copied().sum::<Vec3>() / g.len() as f32;
            (c, g.iter().map(|p| (*p - c).length()).fold(0.0, f32::max) + 7.0)
        }).collect();
        log::info!("props: {} gas stations", gas_stations.len());
        // the props races bring along
        let mut race_models: HashMap<String, (usize, Vec<[Vec3; 3]>)> = HashMap::new();
        for name in RACE_PROPS {
            let Some(&pi) = by_name.get(name) else { continue };
            let Some(mi) = model_for(gpu, world, &mut models, pi) else { continue };
            let mut tris = Vec::new();
            for (mesh, _) in &all[pi].meshes {
                for t in mesh.indices.chunks_exact(3) {
                    let g = |k: usize| mesh.positions.get(t[k] as usize).copied();
                    if let (Some(a), Some(b), Some(c)) = (g(0), g(1), g(2)) {
                        tris.push([a, b, c]);
                    }
                }
            }
            race_models.insert(name.to_string(), (mi, tris));
        }
        // collision footprint per model
        let mut feet: Vec<Foot> = vec![Foot { kind: Solidity::None, center: Vec2::ZERO, half: Vec2::ZERO, y0: 0.0, height: 0.0, mass: 0.0, audio: 0, snd: 0, wide: 0.0, fixed: false }; models.len()];
        for (&pi, mi) in model_of.iter() {
            let Some(mi) = *mi else { continue };
            let prop = &all[pi];
            let name = prop.name.to_ascii_lowercase();
            let height = prop.max.y - prop.min.y;
            if !height.is_finite() {
                continue;
            }
            let foliage = name.contains("_gfx_");
            let mut mn = Vec2::splat(f32::INFINITY);
            let mut mx = Vec2::splat(f32::NEG_INFINITY);
            let mut has_trunk = false;
            for (mesh, si) in &prop.meshes {
                let sname = prop.shaders.get(*si as usize).map(|s| s.name.to_ascii_lowercase()).unwrap_or_default();
                let trunk = sname.contains("trunk");
                if foliage && !trunk {
                    continue;
                }
                has_trunk |= trunk;
                for v in &mesh.positions {
                    if v.y < prop.min.y + 1.2 {
                        mn = mn.min(Vec2::new(v.x, v.z));
                        mx = mx.max(Vec2::new(v.x, v.z));
                    }
                }
            }
            if !mn.is_finite() || !mx.is_finite() {
                continue;
            }
            let mut half = (mx - mn) * 0.5;
            let center = (mx + mn) * 0.5;
            let kind = if foliage {
                // only real trees stop a car; their trunk is roughly round
                if has_trunk && height > 4.0 && half.max_element() < 1.6 {
                    let r = (half.min_element() * 0.8).clamp(0.12, 0.7);
                    half = Vec2::splat(r);
                    Solidity::Solid
                } else {
                    Solidity::None
                }
            } else if name.contains("flare") || name.contains("glasspane") || name.contains("helicopter") {
                Solidity::None
            } else if name.contains("_static_") || name.contains("trailer_ramp") {
                Solidity::Solid
            } else {
                Solidity::Dynamic
            };
            let size = prop.max - prop.min;
            // thin panels (fences, signs) are light; boxes heavier
            let mass = (size.x.max(0.05) * size.y.max(0.05) * size.z.max(0.05) * 45.0).clamp(4.0, 350.0);
            feet[mi] = Foot { kind, center, half: half.max(Vec2::splat(0.04)), y0: prop.min.y, height, mass, snd: crate::sound::prop_sound(&name), audio: mcla_formats::hash::joaat(&name), wide: 0.0, fixed: kind == Solidity::Solid };
        }
        // ... which is only the fallback: the type's own collision shape and mass where it has one
        for (&pi, mi) in model_of.iter() {
            if let (Some(mi), Some(f)) = (*mi, physics_foot(&all[pi], placed_kind.get(&pi).copied())) {
                feet[mi] = f;
            }
        }
        // Props with a light file: where their lamp is - the middle of the lamp glass (the mesh
        // drawn with PropLampGlassTint), or of the topmost part of the model without one.
        // (Checked on the pedestrian light by the Hollywood garage: the original's table has its
        // lamp 10.3 m up and about 1.1 m out from the prop's origin.)
        let mut lamp_of: HashMap<usize, (Vec3, Vec3, f32, [f32; 2])> = HashMap::new();
        let mut lamp_box: HashMap<usize, (Vec3, f32)> = HashMap::new();
        let mut lamp_heads: HashMap<usize, Vec<Vec3>> = HashMap::new();
        for (pi, mi) in &model_of {
            let Some(mi) = mi else { continue };
            let prop = &all[*pi];
            let Some((color, intensity, cone)) = prop_light_file(data_dir, &prop.name) else { continue };
            // (a light file with Intensity 0 - la_prop_glow_warm_01x - lights nothing: the prop is
            // there for its glow in the cars' environment map alone; kept with intensity 0 for
            // `glow_lamps`, left out of `prop_lamps`)
            let (mut sum, mut n) = (Vec3::ZERO, 0.0f32);
            for (mesh, si) in &prop.meshes {
                if prop.shaders.get(*si as usize).map_or(false, |s| s.name.to_ascii_lowercase().contains("lampglass")) {
                    for v in &mesh.positions {
                        sum += *v;
                        n += 1.0;
                    }
                }
            }
            if n == 0.0 {
                let top = prop.max.y - (prop.max.y - prop.min.y) * 0.06;
                for (mesh, _) in &prop.meshes {
                    for v in mesh.positions.iter().filter(|v| v.y >= top) {
                        sum += *v;
                        n += 1.0;
                    }
                }
            }
            // (a prop's lamps one by one - the game's props carry one to three light places, a
            // glow each: taken here as the connected pieces of the lamp glass)
            {
                let mut heads: Vec<(Vec3, f32)> = Vec::new();
                for (mesh, si) in &prop.meshes {
                    if !prop.shaders.get(*si as usize).map_or(false, |s| s.name.to_ascii_lowercase().contains("lampglass")) {
                        continue;
                    }
                    let np = mesh.positions.len();
                    let mut par: Vec<usize> = (0..np).collect();
                    fn find(par: &mut [usize], mut a: usize) -> usize {
                        while par[a] != a {
                            par[a] = par[par[a]];
                            a = par[a];
                        }
                        a
                    }
                    let mut first: HashMap<[i32; 3], usize> = HashMap::new();
                    for (i, v) in mesh.positions.iter().enumerate() {
                        let k = [(v.x * 500.0).round() as i32, (v.y * 500.0).round() as i32, (v.z * 500.0).round() as i32];
                        match first.get(&k) {
                            Some(&j) => {
                                let (ra, rb) = (find(&mut par, i), find(&mut par, j));
                                par[ra] = rb;
                            }
                            None => {
                                first.insert(k, i);
                            }
                        }
                    }
                    for t in mesh.indices.chunks(3).filter(|t| t.len() == 3 && t.iter().all(|i| (*i as usize) < np)) {
                        for e in 1..3 {
                            let (ra, rb) = (find(&mut par, t[0] as usize), find(&mut par, t[e] as usize));
                            par[ra] = rb;
                        }
                    }
                    let mut acc: HashMap<usize, (Vec3, f32)> = HashMap::new();
                    for i in 0..np {
                        let r = find(&mut par, i);
                        let e = acc.entry(r).or_insert((Vec3::ZERO, 0.0));
                        e.0 += mesh.positions[i];
                        e.1 += 1.0;
                    }
                    let mut part: Vec<(Vec3, f32)> = acc.into_values().collect();
                    part.sort_by(|a, b| (a.0 / a.1).x.total_cmp(&(b.0 / b.1).x).then((a.0 / a.1).z.total_cmp(&(b.0 / b.1).z)));
                    heads.extend(part);
                }
                if std::env::var("MCLA_GLOW_DUMP").is_ok() {
                    eprintln!("lampheads {} sphere {:.3} n {} : {}", prop.name, models[*mi].sphere, heads.len(), heads.iter().map(|h| format!("({:.2} {:.2} {:.2} x{})", h.0.x / h.1, h.0.y / h.1, h.0.z / h.1, h.1)).collect::<Vec<_>>().join(" "));
                }
                // The game's own light places (2026-10-14): the type's locators `light0`, `light1` ..
                // are the places the running game keeps at prop + 156 (read out of it for nine lamp
                // types, work/lighting/lg/places.pkl: pedestrian single 0.016 10.638 0.849, the
                // three-light black lamp -1.297 / -0.910 / -0.541 x 7.926 / 7.864 / 7.805 ..). The
                // middle of the lamp glass stood in for them and lies up to 9 cm off - enough to
                // put the pedestrian light by the Hollywood garage's shop on the other side of a
                // half float's step in the lamp table (x 830.52 -> 830.5 where the game's 830.45 ->
                // 830.0) and take 30 % of its light off the shop wall. MCLA_LAMP_GLASS=1: as before.
                {
                    let mut l: Vec<&(String, Vec3)> = prop.locators.iter().filter(|l| l.0.to_ascii_lowercase().starts_with("light")).collect();
                    l.sort_by(|a, b| a.0.cmp(&b.0));
                    if std::env::var("MCLA_GLOW_DUMP").is_ok() {
                        eprintln!("lamplocators {} : {}", prop.name, l.iter().map(|l| format!("{} ({:.3} {:.3} {:.3})", l.0, l.1.x, l.1.y, l.1.z)).collect::<Vec<_>>().join(" "));
                    }
                    if !l.is_empty() && std::env::var("MCLA_LAMP_GLASS").is_err() {
                        heads = l.iter().map(|l| (l.1, 1.0)).collect();
                        // (the lamp table's ONE entry a prop - ground light, cone - stands at
                        // `light0`, not at the lights' mean: the original's table against the
                        // port's on the two-light types, work/lighting/lg/heads2.py)
                        sum = l[0].1;
                        n = 1.0;
                    }
                }
                lamp_heads.insert(*mi, heads.iter().map(|h| h.0 / h.1).collect());
            }
            if n > 0.0 {
                lamp_of.insert(*mi, (sum / n, color, intensity, cone));
                // (the prop's own sphere, prop + 96 in the game: the box round its collision parts,
                // centre and half diagonal - checked on nine lamp types against the running game)
                let (mut bmn, mut bmx) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                for part in &prop.parts {
                    bmn = bmn.min(part.min);
                    bmx = bmx.max(part.max);
                }
                if !bmn.cmple(bmx).all() {
                    bmn = prop.min;
                    bmx = prop.max;
                }
                lamp_box.insert(*mi, ((bmn + bmx) * 0.5, (bmx - bmn).length() * 0.5));
            }
        }
        let mut prop_lamps: Vec<PropLamp> = Vec::new();
        let mut glow_lamps: Vec<(Vec3, Vec3, f32)> = Vec::new();
        let mut glow_boxes: Vec<(Vec3, Vec3)> = Vec::new();
        let mut glow_cull: Vec<(Vec3, f32, f32, Vec3)> = Vec::new();
        let mut colliders: Vec<Collider> = Vec::new();
        // models that are traffic signal heads
        let signal_models: hashbrown::HashSet<usize> = model_of.iter().filter(|(pi, _)| all[**pi].name.contains("traffic_signal")).filter_map(|(_, mi)| *mi).collect();
        let mut head_list: Vec<(u32, Vec3, Vec3)> = Vec::new();
        for (mi, cellmap) in per_model {
            let radius = models[mi].radius;
            let card = card_of.get(&mi).copied().flatten();
            let split = card.is_some() || hedge_low.contains_key(&mi);
            for (c, mut list) in cellmap {
                if split {
                    list.sort_by_key(|i| cell_of(i.pos()));
                }
                let bounds = |part: &[Inst]| {
                    let mut mn = Vec3::splat(f32::INFINITY);
                    let mut mx = Vec3::splat(f32::NEG_INFINITY);
                    let mut scale = 0.0f32;
                    for i in part {
                        let p = i.pos();
                        let s = i.pos_scale[3] * i.rot().1.max(1.0);
                        mn = mn.min(p - Vec3::splat(radius * s));
                        mx = mx.max(p + Vec3::splat(radius * s));
                        scale = scale.max(s);
                    }
                    (mn, mx, scale)
                };
                let mut subs = Vec::new();
                if split {
                    let mut a = 0;
                    while a < list.len() {
                        let fc = cell_of(list[a].pos());
                        let b = a + list[a..].iter().take_while(|i| cell_of(i.pos()) == fc).count();
                        let (mn, mx, scale) = bounds(&list[a..b]);
                        subs.push(Run { model: mi, start: (insts.len() + a) as u32, count: (b - a) as u32, min: mn, max: mx, scale, card, subs: Vec::new() });
                        a = b;
                    }
                }
                let (mn, mx, scale) = bounds(&list);
                cells.entry(c).or_default().push(Run { model: mi, start: insts.len() as u32, count: list.len() as u32, min: mn, max: mx, scale, card, subs });
                total += list.len();
                if let Some((head, color, intensity, cone)) = lamp_of.get(&mi) {
                    for i in &list {
                        let q = Quat::from_xyzw(i.rot_tint[0], i.rot_tint[1], i.rot_tint[2], i.rot_tint[3]);
                        let pos = Vec3::new(i.pos_scale[0], i.pos_scale[1], i.pos_scale[2]) + q * (*head * i.pos_scale[3]);
                        dump_at("  its lamp", pos, "");
                        let (ip, ir) = (Vec3::new(i.pos_scale[0], i.pos_scale[1], i.pos_scale[2]), Vec3::splat(radius * i.pos_scale[3]));
                        let (bc, bw) = lamp_box.get(&mi).copied().unwrap_or((Vec3::ZERO, radius));
                        // (a glow for each of the prop's light places - one to three, the game's
                        // AddGlow once a light; the three lists in step)
                        let one = [*head];
                        let hs: &[Vec3] = match lamp_heads.get(&mi) {
                            Some(h) if h.len() > 1 => h,
                            _ => &one,
                        };
                        for h in hs {
                            glow_lamps.push((ip + q * (*h * i.pos_scale[3]), *color, models[mi].sphere * i.pos_scale[3]));
                            glow_boxes.push((ip - ir, ip + ir));
                            glow_cull.push((ip + q * (bc * i.pos_scale[3]), bw * i.pos_scale[3], models[mi].sphere * i.pos_scale[3], pos));
                        }
                        if *intensity <= 0.0 {
                            continue;
                        }
                        // (the light files point their spot down but for 8.5 degrees - Direction 90, 0, 171.47 - towards the prop's own +x: the original's table shows the props' four headings for it)
                        prop_lamps.push(PropLamp { pos, color: *color, intensity: *intensity, back: (q * Vec3::new(LAMP_BACK[0], LAMP_BACK[1], 0.0)).normalize(), cone: *cone });
                    }
                }
                if signal_models.contains(&mi) {
                    for (k, i) in list.iter().enumerate() {
                        let q = Quat::from_xyzw(i.rot_tint[0], i.rot_tint[1], i.rot_tint[2], i.rot_tint[3]);
                        let f = q * Vec3::Z;
                        head_list.push(((insts.len() + k) as u32, Vec3::new(i.pos_scale[0], i.pos_scale[1], i.pos_scale[2]), Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::Z)));
                    }
                }
                let foot = feet[mi];
                if foot.kind != Solidity::None {
                    let plant = plant_models.contains(&mi);
                    for (k, i) in list.iter().enumerate() {
                        if plant && !solid_plants.contains(&(i.pos_scale[0].to_bits(), i.pos_scale[2].to_bits())) {
                            continue;
                        }
                        colliders.push(Collider { inst: (insts.len() + k) as u32, foot, pos: i.pos(), rot: i.rot().0, scale: i.pos_scale[3], knocked: false });
                    }
                }
                insts.extend(list);
            }
        }
        for (pi, pos, rot) in &posts {
            if let Some(foot) = physics_foot(&all[*pi], Some(2)) {
                colliders.push(Collider { inst: u32::MAX, foot, pos: *pos, rot: *rot, scale: 1.0, knocked: false });
            }
        }
        let n_before = colliders.len();
        for (pi, pos, rot) in &unseen {
            let prop = &all[*pi];
            let snd = crate::sound::prop_sound(&prop.name.to_ascii_lowercase());
            for p in &prop.parts {
                let half = Vec2::new((p.max.x - p.min.x) * 0.5, (p.max.z - p.min.z) * 0.5).max(Vec2::splat(0.04));
                let y0 = p.min.y.max(0.0);
                let mass = if p.mass > 0.0 { p.mass } else { prop.mass / prop.parts.len() as f32 };
                let foot = Foot { kind: Solidity::Dynamic, center: Vec2::new((p.min.x + p.max.x) * 0.5, (p.min.z + p.max.z) * 0.5), half, y0, height: (p.max.y - y0).max(0.1), mass, snd, audio: mcla_formats::hash::joaat(&prop.name), wide: 0.0, fixed: false };
                colliders.push(Collider { inst: u32::MAX, foot, pos: *pos, rot: *rot, scale: 1.0, knocked: false });
            }
        }
        log::info!("props: {} unseen loose props (box sets), {} boxes", unseen.len(), colliders.len() - n_before);
        log::info!("props: {} static props placed ({} in the file), {} unseen posts", n_static, placed.len(), posts.len());
        log::info!("props: {} plants placed, {} of them physical objects in the prop list", n_plants, solid_plants.len());
        let inst_buf = if insts.is_empty() {
            None
        } else {
            Some(gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("prop instances"), contents: bytemuck::cast_slice(&insts), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST }))
        };
        let mut col_grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, c) in colliders.iter().enumerate() {
            col_grid.entry(col_cell(c.pos)).or_default().push(i as u32);
        }
        log::info!("props: {} colliders ({} solid)", colliders.len(), colliders.iter().filter(|c| c.foot.kind == Solidity::Solid).count());
        log::info!("props: {} foliage models, {} instances in {} cells", models.len(), total, cells.len());
        // city light sources
        let lights = load_resource_file(&Path::new(data_dir).join("resources/city/sc/0xaf69cf2f.r226")).map(|r| mcla_formats::rage::lights::parse_lights(&r)).unwrap_or_default();
        // These records are light sources (14,231 omni, 943 spots: what the original's night
        // light grid is filled from), not something one sees: at a lamp the original shows the
        // lamp's own glass or glow card and no halo (MAP_WORK.md D1). The sprite at every
        // record that earlier versions drew is kept behind MCLA_LIGHT_GLOWS.
        let glows: Vec<Inst> = lights
            .iter()
            .filter(|_| std::env::var("MCLA_LIGHT_GLOWS").is_ok())
            .map(|l| {
                let c = l.color.max(Vec3::splat(0.0));
                let peak = c.max_element().max(0.05);
                let size = (l.range * 0.045).clamp(0.5, 2.2);
                Inst { pos_scale: [l.pos.x, l.pos.y, l.pos.z, size], rot_tint: [c.x / peak, c.y / peak, c.z / peak, (0.6 + peak * 1.4) * l.intensity.clamp(0.2, 2.0)], fill: [0; 4], ..Inst::NONE }
            })
            .collect();
        let glow_buf = if glows.is_empty() { None } else { Some(gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("light glows"), contents: bytemuck::cast_slice(&glows), usage: wgpu::BufferUsages::VERTEX })) };
        log::info!("props: {} city lights, {} prop lamps ({} prop types with a light file)", lights.len(), prop_lamps.len(), lamp_of.len());
        let mut light_grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, l) in lights.iter().enumerate() {
            light_grid.entry(cell_of(l.pos)).or_default().push(i as u32);
        }
        // the junctions are the road network's (its 0x47a0c4 objects), the heads bound to their arms
        let junctions = load_resource_file(&Path::new(data_dir).join("resources/city/sc/0x9960fcd8.r66")).map(|r| mcla_formats::rage::navsys::parse_junctions(&r)).unwrap_or_default();
        let (signals, of_head) = crate::signals::Signals::build(&junctions, &head_list.iter().map(|h| (h.1, h.2)).collect::<Vec<_>>());
        let heads: Vec<crate::signals::SignalHead> = head_list.iter().zip(of_head.iter()).map(|(h, j)| crate::signals::SignalHead { inst: h.0, pos: h.1, facing: h.2, junction: j.0, arm: j.1, shown: 255 }).collect();
        log::info!("props: {} traffic signal heads, {} bound to the arms of {} junctions", heads.len(), heads.iter().filter(|h| h.junction != crate::signals::UNBOUND).count(), signals.junctions.len());
        // the lamp locators of every head's type, and the glow tuning
        let signal_glows = {
            let type_of: HashMap<usize, usize> = model_of.iter().filter_map(|(pi, mi)| mi.map(|m| (m, *pi))).collect();
            let runs: Vec<(u32, u32, usize)> = cells.values().flatten().filter(|r| signal_models.contains(&r.model)).map(|r| (r.start, r.count, r.model)).collect();
            let lamps: Vec<[[Option<Vec3>; 2]; 3]> = heads
                .iter()
                .map(|h| {
                    let prop = runs.iter().find(|r| h.inst >= r.0 && h.inst < r.0 + r.1).and_then(|r| type_of.get(&r.2)).map(|pi| &all[*pi]);
                    let at = |n: &str| prop.and_then(|p| p.locators.iter().find(|l| l.0.eq_ignore_ascii_case(n)).map(|l| l.1));
                    [[at("red"), at("red1")], [at("yellow"), at("yellow1")], [at("green"), at("green1")]]
                })
                .collect();
            let cull: Vec<(Vec3, f32, f32, (Vec3, Vec3))> = heads
                .iter()
                .map(|h| {
                    let run = runs.iter().find(|r| h.inst >= r.0 && h.inst < r.0 + r.1);
                    let i = &insts[h.inst as usize];
                    let (q, s) = (i.rot().0, i.pos_scale[3]);
                    let Some((prop, mi)) = run.and_then(|r| type_of.get(&r.2).map(|pi| (&all[*pi], r.2))) else { return (h.pos, 1.0, 1.0, (h.pos - Vec3::ONE, h.pos + Vec3::ONE)) };
                    let (mut bmn, mut bmx) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                    for part in &prop.parts {
                        bmn = bmn.min(part.min);
                        bmx = bmx.max(part.max);
                    }
                    if !bmn.cmple(bmx).all() {
                        bmn = prop.min;
                        bmx = prop.max;
                    }
                    let r = models[mi].radius * s;
                    (h.pos + q * ((bmn + bmx) * 0.5 * s), (bmx - bmn).length() * 0.5 * s, models[mi].sphere * s, (h.pos - Vec3::splat(r), h.pos + Vec3::splat(r)))
                })
                .collect();
            let text = std::fs::read_to_string(Path::new(data_dir).join("tune/lighting/GlowTuning.xml")).unwrap_or_default();
            let four = |tag: &str, or: [f32; 4]| -> [f32; 4] {
                let v: Vec<f32> = text.split(&format!("<{tag} ")).nth(1).and_then(|t| t.split_once('>')).and_then(|t| t.1.split('<').next()).map(|t| t.split_whitespace().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                if v.len() == 4 { [v[0], v[1], v[2], v[3]] } else { or }
            };
            log::info!("props: lamp locators on {} of {} signal heads", lamps.iter().filter(|l| l.iter().flatten().any(|x| x.is_some())).count(), heads.len());
            SignalGlows { lit: Vec::new(), cull, buf: None, count: 0, lamps, sizes: four("sm_fSignalGlowSizes", [4.52, 5.25, 8.7675, 10.0]), intensities: four("sm_fSignalGlowIntensitys", [4.52, 5.0, 5.0, 3.0]) }
        };
        let impostors = impostor::Impostors::new(gpu, data_dir, &mut models, &imp_types);
        Props { hedge_low, impostors, signal_glows, glow_buf, glow_count: glows.len() as u32, lights, prop_lamps, glow_lamps, glow_boxes, glow_cull, light_grid, models, inst_buf, cells, instances: total, insts, colliders, gas_stations, col_grid, debris: Vec::new(), hits: Vec::new(), fx: Vec::new(), defl_done: Default::default(), knocked: 0, rng: 12345, level_on: false, level_map: None, heads, signals, race_models, race_buf: None, race_runs: Vec::new(), barriers, barrier_buf, barrier_on: 0, emitters, trash: load_resource_file(&Path::new(data_dir).join("resources/city/sc/0xad0a8b94.r66")).map(|r| trash_rails(&r)).unwrap_or_default() }
    }

    /// Keep the effects of the particle props near the camera playing, and the litter on the
    /// wind at the player's car (call once a tick).
    pub fn emit(&self, fx: &mut crate::particles::Particles, cam: Vec3, tick: u64, player: Option<(Vec3, Vec3, Quat)>, stormy: f32) {
        // Wind debris: while the player's car is within 20 m of a trash rail the original runs
        // the effect of the rail's group on the car (`wind_debris_hw` / `_la` / `_beach`: flyers,
        // papers and leaves let out in a box ahead of it - `F-52018`, rule
        // `docs/reference/map/litter_evo_rule.md`): only while the Stormy weather's weight is exactly
        // 0, attached to the car's own matrix (`F-47301`, mode 2), evolution "speed" =
        // clamp(|v| / 90, 0, 1) every frame (`F-21480` A-62667); no stand-in
        if let Some((pos, vel, rot)) = player.filter(|_| stormy == 0.0) {
            let near = |a: Vec3, b: Vec3| {
                let ab = b - a;
                let t = ((pos - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                (a + ab * t).distance_squared(pos) < 400.0
            };
            let rail = self.emitters.iter().find(|e| e.1 == EMIT_RAIL && e.3.first().is_some_and(|f| near(e.0, f.1))).map(|e| e.3[0].0);
            // (the effect's frame is the car's: -z ahead)
            if let Some(h) = rail {
                fx.play_evo(u64::MAX, h, pos, rot, false, vel, 0.0, &[("speed", (vel.length() / 90.0).clamp(0.0, 1.0))]);
            }
        }
        for (i, (pos, kind, rot, effects)) in self.emitters.iter().enumerate() {
            if *kind == EMIT_RAIL {
                continue;
            }
            let d = pos.distance(cam);
            if d > 260.0 {
                continue;
            }
            // the effects the prop's type names, from the disc's rules
            let mut played = false;
            for (j, (h, at)) in effects.iter().enumerate() {
                played |= fx.play(((i as u64) << 8) | j as u64, *h, *pos + *rot * *at, *rot, Vec3::ZERO, d);
            }
            if played || (d > 90.0 && (tick + i as u64) % 2 != 0) {
                continue;
            }
            match *kind {
                0..=9 => fx.fountain(*pos, (*kind as f32 + 1.0) * 0.5),
                10 => fx.fountain(*pos, 0.25),
                11 => fx.waterfall(*pos),
                12 => {
                    if (tick + i as u64) % 4 == 0 {
                        fx.steam(*pos, 1.0);
                    }
                }
                _ => {
                    if (tick + i as u64) % 3 == 0 {
                        fx.steam(*pos, 3.0);
                    }
                }
            }
        }
    }

    /// The hangouts' barriers: out (drawn and solid) while the police are after the player -
    /// the original's `PoliceManager_CheckPoliceActivity(1) > 1` - and, at a local hangout, while
    /// the player is in it (`inside`: where that hangout is). Otherwise they are not in the world.
    pub fn set_barriers(&mut self, world: &mut World, chase: bool, inside: Option<Vec3>) {
        let up = std::env::var("MCLA_BARRIERS_UP").is_ok();
        let mut on = 0u16;
        for (k, b) in self.barriers.iter().enumerate() {
            if up || chase || (b.local && inside.is_some_and(|p| p.distance(b.pos) < 300.0)) {
                on |= 1 << k;
            }
        }
        if on == self.barrier_on {
            return;
        }
        self.barrier_on = on;
        world.collider.sectors.remove(&BARRIER_ID);
        let tris: Vec<[Vec3; 3]> = self.barriers.iter().enumerate().filter(|(k, _)| on >> k & 1 == 1).flat_map(|(_, b)| b.tris.iter().copied()).collect();
        if !tris.is_empty() {
            world.collider.sectors.insert(BARRIER_ID, crate::physics::collision::SectorMesh::build(tris));
        }
        log::info!("props: hangout barriers out: {:012b}", on);
    }

    /// Put a race's props out (an empty list takes them away again). Ramps are solid.
    pub fn set_race_props(&mut self, gpu: &Gpu, world: &mut World, items: &[(String, Vec3, glam::Quat)]) {
        self.race_runs.clear();
        self.race_buf = None;
        world.collider.sectors.remove(&RACE_PROP_ID);
        let mut insts: Vec<Inst> = Vec::new();
        let mut tris: Vec<[Vec3; 3]> = Vec::new();
        for (name, pos, rot) in items {
            let Some((mi, mesh)) = self.race_models.get(name.as_str()) else {
                log::debug!("race prop {} has no model", name);
                continue;
            };
            self.race_runs.push((*mi, insts.len() as u32));
            insts.push(Inst { pos_scale: [pos.x, pos.y, pos.z, 1.0], rot_tint: [rot.x, rot.y, rot.z, rot.w], fill: [0; 4], ..Inst::NONE });
            if name.contains("ramp") {
                tris.extend(mesh.iter().map(|t| [*pos + *rot * t[0], *pos + *rot * t[1], *pos + *rot * t[2]]));
            }
        }
        if !insts.is_empty() {
            self.race_buf = Some(gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("race props"), contents: bytemuck::cast_slice(&insts), usage: wgpu::BufferUsages::VERTEX }));
        }
        if !tris.is_empty() {
            world.collider.sectors.insert(RACE_PROP_ID, crate::physics::collision::SectorMesh::build(tris));
        }
    }

    /// Draw the foliage around the camera. The instanced pipeline must be set by the caller.
    /// (The shadow pass's call: the plants' far cards are their shadow impostors - a quad
    /// square to the light with the type's silhouette mask, props/impostor.rs - or, without
    /// them, the cards baked at load.)
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, cam: Vec3, frustum: &Frustum, occluded: Option<&crate::occluder::Selected>, draws: &mut usize) {
        self.draw_with(pass, gpu, cam, frustum, occluded, draws, false, false)
    }

    /// The same for the scene pass: the plants' far cards are their impostors where the type
    /// has one (props/impostor.rs; `update_impostors` must have run this frame).
    pub fn draw_scene<'a>(&'a self, pass: &mut wgpu::RenderPass<'_>, gpu: &'a Gpu, cam: Vec3, frustum: &Frustum, occluded: Option<&crate::occluder::Selected>, draws: &mut usize) {
        self.draw_with(pass, gpu, cam, frustum, occluded, draws, true, false)
    }

    /// The glass props' panes (PropGlass batches), for the scene's blended part: the caller sets the instanced
    /// blend pipeline. The original draws them near the end of the frame, after the shop panes, blended, both
    /// sides, without depth writes - and not at all into the shadow maps (no depth draw of the 6-index panes
    /// in the captures: only their metal frames cast), so `draw` / `draw_scene` leave them out.
    pub fn draw_glass<'a>(&'a self, pass: &mut wgpu::RenderPass<'_>, gpu: &'a Gpu, cam: Vec3, frustum: &Frustum, occluded: Option<&crate::occluder::Selected>, draws: &mut usize) {
        self.draw_with(pass, gpu, cam, frustum, occluded, draws, true, true)
    }

    /// Once a frame, before the shadow pass: the shadow impostors' masks that are due.
    pub fn update_shadow_impostors(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, light_vp: glam::Mat4, hour: f32) {
        if let Some(imp) = self.impostors.as_mut() {
            imp.update_shadow(gpu, enc, light_vp, hour, &self.models);
        }
    }

    /// Once a frame, before the scene pass: redraw the impostor pictures that are due.
    /// Diagnostics: see `Impostors::dump`.
    pub fn dump_impostors(&self, gpu: &Gpu, dir: &str) {
        if let Some(imp) = self.impostors.as_ref() {
            imp.dump(gpu, dir);
        }
    }

    pub fn update_impostors(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, globals: &crate::render::Globals) {
        if let Some(imp) = self.impostors.as_mut() {
            imp.update(gpu, enc, globals, &self.models);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_with<'a>(&'a self, pass: &mut wgpu::RenderPass<'_>, gpu: &'a Gpu, cam: Vec3, frustum: &Frustum, occluded: Option<&crate::occluder::Selected>, draws: &mut usize, scene: bool, glass: bool) {
        let Some(ibuf) = self.inst_buf.as_ref() else { return };
        // (the card a run names -> what is drawn for it)
        let imp = self.impostors.as_ref();
        let card_model = |card: usize| imp.map_or(card, |i| if scene { i.card(card) } else { i.shadow_card(card) });
        pass.set_vertex_buffer(1, ibuf.slice(..));
        let lens = lens();
        let k = Lod::now();
        // fine cells: what is only seen close by; coarse cells: lamps, masts, trees (both as far
        // again as the DRAW DISTANCE takes the reach: a cell's props were sorted by the original's)
        for (cell, size, reach) in [(cell_of(cam), CELL, NEAR_DIST * k.draw), (far_cell_of(cam), FAR_CELL, FAR_DIST * k.draw)] {
            let r = (reach / size).ceil() as i32;
            for dx in -r..=r {
                for dz in -r..=r {
                    let Some(runs) = self.cells.get(&(cell.0 + dx, cell.1 + dz)) else { continue };
                    for run in runs {
                        let model = &self.models[run.model];
                        let d = dist_to_box(cam, run.min, run.max);
                        let (near, far) = model.reach(run.scale, lens, k);
                        if d > far || !frustum.intersects_aabb(run.min, run.max) || occluded.map_or(false, |o| o.hides_aabb(run.min, run.max)) {
                            continue;
                        }
                        let mut put = |m: usize, r: &Run| {
                            for b in &self.models[m].batches {
                                if (b.mode == MatMode::PropGlass) != glass { continue; }
                                pass.set_bind_group(1, b.bind.as_ref(), &[]);
                                pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                                pass.set_vertex_buffer(0, b.vb.slice(..));
                                pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                                pass.draw_indexed(0..b.count, 0, r.start..r.start + r.count);
                                *draws += 1;
                            }
                        };
                        match run.card {
                            // a plant cell: all cards when it is far, else model or card by fine cell
                            // (lighting work, 2026-10-14: in the shadow pass a plant is ALWAYS its mask card.
                            // The original's depth pass draws the foliage effect only as
                            // VS_ShadowImpostorInstanceDepthAlpha - never the plant's mesh - and its collector at
                            // 07:00 shows one narrow frond band where the mesh threw a solid crown on the poster.
                            // MCLA_PLANT_MESH_SHADOW=1: the mesh near by, as before.)
                            Some(card) if d > near + 1.0 || (!scene && !plant_mesh_shadow()) => put(card_model(card), run),
                            Some(card) => {
                                for sub in &run.subs {
                                    let ds = dist_to_box(cam, sub.min, sub.max);
                                    let (near, far) = model.reach(sub.scale, lens, k);
                                    if ds > far || !frustum.intersects_aabb(sub.min, sub.max) || occluded.map_or(false, |o| o.hides_aabb(sub.min, sub.max)) {
                                        continue;
                                    }
                                    // (each plant fades by its own distance: where a cell reaches
                                    // across the change both are drawn)
                                    let ds_far = ds + Vec3::new(sub.max.x - sub.min.x, 0.0, sub.max.z - sub.min.z).length();
                                    if ds <= near {
                                        put(run.model, sub);
                                    }
                                    if ds_far >= near * 0.25 - 8.0 {
                                        put(card_model(card), sub);
                                    }
                                }
                            }
                            // a hedge: the high level to L, the low one from L - 16 (both in between,
                            // each faded by city.wgsl); the shadow passes keep the high level
                            None => match self.hedge_low.get(&run.model).filter(|_| scene) {
                                Some(&low) if d > near + 1.0 => put(low, run),
                                Some(&low) => {
                                    for sub in &run.subs {
                                        let ds = dist_to_box(cam, sub.min, sub.max);
                                        let (near, far) = model.reach(sub.scale, lens, k);
                                        if ds > far || !frustum.intersects_aabb(sub.min, sub.max) || occluded.map_or(false, |o| o.hides_aabb(sub.min, sub.max)) {
                                            continue;
                                        }
                                        let ds_far = ds + Vec3::new(sub.max.x - sub.min.x, 0.0, sub.max.z - sub.min.z).length();
                                        if ds <= near {
                                            put(run.model, sub);
                                        }
                                        if ds_far >= near - 16.0 {
                                            put(low, sub);
                                        }
                                    }
                                }
                                None => put(run.model, run),
                            },
                        }
                    }
                }
            }
        }
                // the running race's own props
        if let Some(rb) = self.race_buf.as_ref() {
            pass.set_vertex_buffer(1, rb.slice(..));
            for (mi, inst) in &self.race_runs {
                for b in &self.models[*mi].batches {
                    if (b.mode == MatMode::PropGlass) != glass { continue; }
                    pass.set_bind_group(1, b.bind.as_ref(), &[]);
                    pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                    pass.set_vertex_buffer(0, b.vb.slice(..));
                    pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..b.count, 0, *inst..*inst + 1);
                    *draws += 1;
                }
            }
        }
        // the hangouts' barriers that are out
        if let (Some(bb), true) = (self.barrier_buf.as_ref(), self.barrier_on != 0) {
            pass.set_vertex_buffer(1, bb.slice(..));
            for (k, bar) in self.barriers.iter().enumerate() {
                if self.barrier_on >> k & 1 == 0 {
                    continue;
                }
                for b in &self.models[bar.model].batches {
                    if (b.mode == MatMode::PropGlass) != glass { continue; }
                    pass.set_bind_group(1, b.bind.as_ref(), &[]);
                    pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                    pass.set_vertex_buffer(0, b.vb.slice(..));
                    pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..b.count, 0, k as u32..k as u32 + 1);
                    *draws += 1;
                }
            }
        }
    }

    /// Additive glows of the city lights. Sets its own pipeline.
    pub fn draw_glows<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu) {
        let Some(buf) = self.glow_buf.as_ref() else { return };
        pass.set_pipeline(&gpu.pipeline_glow);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.draw(0..4, 0..self.glow_count);
    }

    /// The glow at the lit lamp of every signal head within 250 m that faces the camera (call
    /// once a frame). As the original (work/map/signal_rule.md section 5): red 0xFFFF0000,
    /// yellow 0xFFE08040, green 0xFF00FF00 at both locators of the lit colour, only while the
    /// camera is in the head's +Z half space; a head no junction took shows green. The size
    /// and strength are the tuning's first set by day and its fourth at night (`night` 0..1),
    /// scaled to the port's sprite - that mapping is the port's own.
    pub fn update_signal_glows(&mut self, gpu: &Gpu, cam: Vec3, night: f32) {
        const MAX: usize = 384;
        const REACH: f32 = 250.0;
        const COLOURS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.878, 0.502, 0.251], [0.0, 1.0, 0.0]];
        let g = &mut self.signal_glows;
        let n = night.clamp(0.0, 1.0);
        let size = (g.sizes[0] + (g.sizes[3] - g.sizes[0]) * n) * 0.16;
        let strength = (g.intensities[0] + (g.intensities[3] - g.intensities[0]) * n) * 0.5;
        let mut out: Vec<Inst> = Vec::new();
        g.lit.clear();
        for (k, (h, lamps)) in self.heads.iter().zip(g.lamps.iter()).enumerate() {
            let d = h.pos - cam;
            let rot = self.insts[h.inst as usize].rot().0;
            let z = rot * Vec3::Z;
            // (the original, F-79347, once a head: no glow while the camera is behind the plane
            // through its origin facing z - equality draws; no reach of its own. Its view test, a 2 m
            // sphere in the main view or within 20 m, and the prop list's are main.rs's, with the frustum)
            if z.dot(d) > 0.0 {
                continue;
            }
            let near = d.length_squared() <= REACH * REACH;
            // 0 red, 1 yellow, 2 green
            let state = if h.junction == crate::signals::UNBOUND { 2 } else { self.signals.junctions[h.junction as usize].arm_state(h.arm as usize, 0.0) } as usize;
            for l in lamps[state.min(2)].iter().flatten() {
                // (a hand's width in front of the lens, so the sprite is not cut by the visor)
                let p = h.pos + rot * *l + z * 0.12;
                let dist = p.distance(cam);
                let fade = ((REACH - dist) / 40.0).clamp(0.0, 1.0);
                let c = COLOURS[state.min(2)];
                g.lit.push((h.pos + rot * *l, z, c, k as u32));
                // (the sprites below: MCLA_DEBUG_VIEW=115, the look of before - tuning x 0.16 / x 0.5, 250 m)
                if near && fade > 0.0 && out.len() < MAX {
                    // (a negative size: shown by day too)
                    out.push(Inst { pos_scale: [p.x, p.y, p.z, -size], rot_tint: [c[0], c[1], c[2], strength * fade], fill: [0; 4], ..Inst::NONE });
                }
            }
        }
        g.count = out.len() as u32;
        if out.is_empty() {
            return;
        }
        out.resize(MAX, Inst { pos_scale: [0.0; 4], rot_tint: [0.0; 4], fill: [0; 4], ..Inst::NONE });
        match g.buf.as_ref() {
            Some(b) => gpu.queue.write_buffer(b, 0, bytemuck::cast_slice(&out)),
            None => g.buf = Some(gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("signal glows"), contents: bytemuck::cast_slice(&out), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST })),
        }
    }

    /// The signal lamps lit this frame (after `update_signal_glows`): locator, the head's z axis, colour, head.
    pub fn signal_lamps_lit(&self) -> &[(Vec3, Vec3, [f32; 3], u32)] {
        &self.signal_glows.lit
    }

    /// A head's origin and what the game's prop list tests of it: box centre, half diagonal, radius, box.
    pub fn signal_head_cull(&self, head: u32) -> (Vec3, Vec3, f32, f32, (Vec3, Vec3)) {
        let c = self.signal_glows.cull[head as usize];
        (self.heads[head as usize].pos, c.0, c.1, c.2, c.3)
    }

    /// Draw the signal lamps' glows (day and night). Sets its own pipeline.
    pub fn draw_signal_glows<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu) {
        if let (Some(buf), true) = (self.signal_glows.buf.as_ref(), self.signal_glows.count > 0) {
            pass.set_pipeline(&gpu.pipeline_glow);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..4, 0..self.signal_glows.count);
        }
    }

    /// The city lights nearest to `focus` (at most `max`), as (position, range, colour).
    pub fn lights_near(&self, focus: Vec3, max: usize) -> Vec<(Vec3, f32, Vec3)> {
        let (cx, cz) = cell_of(focus);
        let mut v: Vec<(f32, u32)> = Vec::new();
        for dx in -2..=2 {
            for dz in -2..=2 {
                if let Some(list) = self.light_grid.get(&(cx + dx, cz + dz)) {
                    for &i in list {
                        let d = self.lights[i as usize].pos.distance(focus);
                        if d < 170.0 {
                            v.push((d, i));
                        }
                    }
                }
            }
        }
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        v.into_iter()
            .take(max)
            .map(|(_, i)| {
                let l = &self.lights[i as usize];
                (l.pos, l.range.clamp(8.0, 40.0), l.color.max(Vec3::ZERO) * l.intensity.clamp(0.2, 2.0))
            })
            .collect()
    }

    /// Standing props around `p` as (centre, radius) discs, for things that walk around them.
    pub fn obstacles_near(&self, p: Vec3, r: f32, out: &mut Vec<(Vec3, f32)>) {
        let (c0, c1) = (col_cell(p - Vec3::splat(r + 3.0)), col_cell(p + Vec3::splat(r + 3.0)));
        for cx in c0.0..=c1.0 {
            for cz in c0.1..=c1.1 {
                let Some(list) = self.col_grid.get(&(cx, cz)) else { continue };
                for &i in list {
                    let c = &self.colliders[i as usize];
                    if c.knocked || c.foot.height < 0.3 || (c.pos.y - p.y).abs() > 3.0 {
                        continue;
                    }
                    let centre = c.pos + c.rot * Vec3::new(c.foot.center.x, 0.0, c.foot.center.y) * c.scale;
                    let rad = c.foot.half.length().min(c.foot.half.max_element() * 1.1) * c.scale;
                    if centre.distance(p) < r + rad {
                        out.push((centre, rad));
                    }
                }
            }
        }
    }

    /// Standing props around `p` as footprint boxes: centre, the box's own x axis (flat,
    /// unit) and the half sizes along its x and z, for walkers that pass them by the corners.
    pub fn obstacle_boxes_near(&self, p: Vec3, r: f32, out: &mut Vec<(Vec3, Vec3, Vec2)>) {
        let (c0, c1) = (col_cell(p - Vec3::splat(r + 3.0)), col_cell(p + Vec3::splat(r + 3.0)));
        for cx in c0.0..=c1.0 {
            for cz in c0.1..=c1.1 {
                let Some(list) = self.col_grid.get(&(cx, cz)) else { continue };
                for &i in list {
                    let c = &self.colliders[i as usize];
                    if c.knocked || c.foot.height < 0.3 || (c.pos.y - p.y).abs() > 3.0 {
                        continue;
                    }
                    let centre = c.pos + c.rot * Vec3::new(c.foot.center.x, 0.0, c.foot.center.y) * c.scale;
                    let half = c.foot.half * c.scale;
                    if centre.distance(p) < r + half.length() {
                        let ax = c.rot * Vec3::X;
                        out.push((centre, Vec3::new(ax.x, 0.0, ax.z).normalize_or(Vec3::X), half));
                    }
                }
            }
        }
    }

    /// Solid things (trunks, masts, bus shelters) within `r` of a point: centre and radius.
    pub fn solid_near(&self, p: Vec3, r: f32, out: &mut Vec<(Vec3, f32)>) {
        let (c0, c1) = (col_cell(p - Vec3::splat(r + 3.0)), col_cell(p + Vec3::splat(r + 3.0)));
        for cx in c0.0..=c1.0 {
            for cz in c0.1..=c1.1 {
                let Some(list) = self.col_grid.get(&(cx, cz)) else { continue };
                for &i in list {
                    let c = &self.colliders[i as usize];
                    if c.knocked || c.foot.kind != Solidity::Solid || c.foot.height < 0.5 || (c.pos.y - p.y).abs() > 4.0 {
                        continue;
                    }
                    let centre = c.pos + c.rot * Vec3::new(c.foot.center.x, 0.0, c.foot.center.y) * c.scale;
                    let rad = c.foot.half.length().min(c.foot.half.max_element() * 1.1) * c.scale;
                    if centre.distance(p) < r + rad {
                        out.push((centre, rad));
                    }
                }
            }
        }
    }

    /// Diagnostics: what stands within `r` of a point.
    pub fn describe_near(&self, p: Vec3, r: f32) -> Vec<String> {
        self.colliders
            .iter()
            .filter(|c| !c.knocked && c.pos.distance(p) < r)
            .map(|c| format!("{} at ({:.1},{:.1}) {:.1} m away, {:.1} x {:.1} x {:.1} high, {:.0} kg", if c.foot.kind == Solidity::Solid { "solid" } else { "loose" }, c.pos.x, c.pos.z, c.pos.distance(p), c.foot.half.x * 2.0 * c.scale, c.foot.half.y * 2.0 * c.scale, c.foot.height * c.scale, c.foot.mass))
            .collect()
    }

    /// The fixed props (level state 2) within `r` of `p` (plan distance) as the object grid's prop
    /// objects (mcProp, kind 3): the instance's place, its forward -(matrix row 2), velocity 0, the
    /// footprint's full width and length. Each collider once (`seen` by collider index).
    pub fn fixed_objects_near(&self, p: Vec3, r: f32, seen: &mut hashbrown::HashSet<u32>, out: &mut Vec<crate::brainturn::ObjectInfo>) {
        let (c0, c1) = (col_cell(p - Vec3::splat(r)), col_cell(p + Vec3::splat(r)));
        for cx in c0.0..=c1.0 {
            for cz in c0.1..=c1.1 {
                let Some(list) = self.col_grid.get(&(cx, cz)) else { continue };
                for &i in list {
                    let c = &self.colliders[i as usize];
                    let d = c.pos - p;
                    if !c.foot.fixed || d.x * d.x + d.z * d.z > r * r || !seen.insert(i) {
                        continue;
                    }
                    let half = c.foot.half * c.scale;
                    out.push(crate::brainturn::ObjectInfo { pos: c.pos, vel: Vec3::ZERO, forward: -(c.rot * Vec3::Z), extent: (half.x * 2.0, half.y * 2.0), kind: crate::brainturn::KIND_PROP });
                }
            }
        }
    }

    /// Diagnostics: every collider standing in a box.
    pub fn describe_in(&self, mn: Vec3, mx: Vec3) -> Vec<String> {
        self.colliders
            .iter()
            .filter(|c| c.pos.x >= mn.x && c.pos.x <= mx.x && c.pos.z >= mn.z && c.pos.z <= mx.z)
            .map(|c| format!("{:.1} {:.1} {} half {:.2} x {:.2} high {:.1} fixed {} audio {:08x} knocked {}", c.pos.x, c.pos.z, if c.foot.kind == Solidity::Solid { "solid" } else { "loose" }, c.foot.half.x * c.scale, c.foot.half.y * c.scale, c.foot.height * c.scale, c.foot.fixed, c.foot.audio, c.knocked))
            .collect()
    }

    /// Everything solid (trees, posts, walls of props) standing in a box.
    pub fn solid_in(&self, mn: Vec3, mx: Vec3) -> Vec<Vec3> {
        self.colliders.iter().filter(|c| c.foot.kind == Solidity::Solid && !c.knocked && c.pos.x >= mn.x && c.pos.x <= mx.x && c.pos.z >= mn.z && c.pos.z <= mx.z).map(|c| c.pos).collect()
    }

    /// Test helper: solid / knockable props within `max` metres, nearest first.
    pub fn colliders_near(&self, p: Vec3, solid: bool, max: f32) -> Vec<Vec3> {
        let want = if solid { Solidity::Solid } else { Solidity::Dynamic };
        let mut v: Vec<(f32, Vec3)> = self
            .colliders
            .iter()
            .filter(|c| c.foot.kind == want && !c.knocked && (c.pos.y - p.y).abs() < 3.0 && c.foot.height > 0.7)
            .map(|c| (c.pos.distance(p), c.pos))
            .filter(|(d, _)| *d < max && *d > 6.0)
            .collect();
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        v.into_iter().map(|(_, q)| q).collect()
    }

    /// Every prop with a body: its type's audio object (joaat of the type's name - game.dat LA_PROP_<TYPE>) and where it
    /// is now (sound.rs: the props' distance loops).
    pub fn audio_places(&self) -> impl Iterator<Item = (u32, Vec3)> + '_ {
        self.colliders.iter().map(|c| (c.foot.audio, c.pos))
    }

    /// Bring the signal heads near `focus` up to date with their junction's phase.
    pub fn update_signals(&mut self, gpu: &Gpu, focus: Vec3, t: f32) {
        let Some(buf) = self.inst_buf.as_ref() else { return };
        for h in self.heads.iter_mut() {
            if (h.pos.x - focus.x).abs() > 450.0 || (h.pos.z - focus.z).abs() > 450.0 {
                continue;
            }
            // lamps on the z faces serve the street along the prop's z axis, lamps on the
            // x faces the cross street
            // (one state for every lamp of a head - the original sets one SignalColor per prop;
            // the shader takes a state for the z faces and one for the x faces: the same here.
            // A head no junction took shows green.)
            let one = if h.junction == crate::signals::UNBOUND { 2 } else { self.signals.junctions[h.junction as usize].arm_state(h.arm as usize, t) };
            let state = one * 4;
            if state == h.shown {
                continue;
            }
            h.shown = state;
            let inst = &mut self.insts[h.inst as usize];
            inst.pos_scale[3] = 1.0 + state as f32 * 0.001;
            gpu.queue.write_buffer(buf, h.inst as u64 * std::mem::size_of::<Inst>() as u64, bytemuck::bytes_of(inst));
        }
    }

    /// Collide the active cars with the props around them and advance knocked-over props.
    pub fn collide(&mut self, gpu: &Gpu, world: &World, cars: &mut [crate::vehicles::CarInstance], dt: f32) {
        self.hits.clear();
        self.fx.clear();
        // (the type's event set of a collider: the fragment type of its name, `FragType::type_events`)
        let loose_types = world.collider.loose.clone();
        let type_events = |audio: u32| loose_types.as_ref().and_then(|l| l.types.iter().find(|t| mcla_formats::hash::joaat(&t.name) == audio)).map(|t| t.type_events.clone()).unwrap_or_default();
        // (a level without the props - MCLA_NO_LOOSE_PHYSICS, the torn-off bumpers' alone - leaves them to this file)
        self.level_on = world.collider.loose.as_ref().is_some_and(|l| !l.instances.is_empty());
        if self.colliders.is_empty() || dt <= 0.0 {
            return;
        }
        for car in cars.iter_mut() {
            // (step 19f: MegaStealth - the car meets nothing but the map)
            if car.parked.is_some() || !car.placed || car.pu_ghost {
                continue;
            }
            let in_level = self.level_on && car.state.veh.is_some() && car.model.params.hull.is_some();
            let st = &mut car.state;
            let half = car.model.params.body_half;
            let human = car.is_player;
            let f3 = st.forward();
            let f = Vec2::new(f3.x, f3.z).normalize_or(Vec2::Y);
            let r = Vec2::new(-f.y, f.x);
            let ha = Vec2::new(half.x, half.z);
            let (cx, cz) = col_cell(st.pos);
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let Some(list) = self.col_grid.get(&(cx + dx, cz + dz)) else { continue };
                    for &ci in list {
                        let c = &mut self.colliders[ci as usize];
                        // (a loose prop - level state 1 - is the physics level's: the car meets it in its own step)
                        if c.foot.kind == Solidity::Dynamic && c.inst != u32::MAX && in_level {
                            continue;
                        }
                        if c.knocked {
                            continue;
                        }
                        let dy = st.pos.y - c.pos.y;
                        if dy < -1.5 || dy > c.foot.height * c.scale + 1.0 {
                            continue;
                        }
                        let reach = half.z + c.foot.half.max_element().max(c.foot.wide) * c.scale + 1.5;
                        if (st.pos.x - c.pos.x).abs() > reach || (st.pos.z - c.pos.z).abs() > reach {
                            continue;
                        }
                        // Deflection (mcCarBehavior::CollideObjects, F-35508; tune/vehicle/
                        // propDeflection: SpeedThreshold 25, SpeedClamp 25, MaxDetectionTime 0.5,
                        // MinGapDist 0.1, AccelScalar 1, AngAccScalar 1.5 - work/map/deflection_rule.md).
                        // A post is two capsules: the post, and a wide one that is never solid but
                        // brings the pair up early. In its reach the car is not collided with the
                        // post at all; it is shifted sideways so that it has cleared the post by the
                        // time it gets there - the player only from 25 m/s, every other car always.
                        if c.foot.wide > 0.0 {
                            let fwd = Vec3::new(f.x, 0.0, f.y);
                            // (h of the rule = the half box of the car's whole bound, max(-min, max) a side: the
                            // hull's box grown by 0.041 m as the running game holds it - R8: 1.04 x 2.40 m. In the
                            // recorded pass of the original the pushes were 0.193, 0.192, 0.178, 0.163, 0.108 m a
                            // tick; this gives 0.199, 0.196, 0.192, 0.185, 0.140 from the same states - MAP_WORK.md
                            // pass 9. A car without a hull: its body size and the R8's margins.)
                            let (hx, hz) = match car.model.params.hull.as_ref() {
                                Some(h) => (h.max.x.max(-h.min.x) + 0.041, h.max.z.max(-h.min.z) + 0.041),
                                None => (half.x + 0.05, half.z + 0.07),
                            };
                            let ext = |a: Vec3| (r.x * a.x + r.y * a.z).abs() * hx + (f.x * a.x + f.y * a.z).abs() * hz;
                            let v = Vec3::new(st.vel.x, 0.0, st.vel.z) + fwd * 0.1;
                            let d3 = Vec3::new(c.pos.x - st.pos.x, 0.0, c.pos.z - st.pos.z);
                            let dist = d3.length();
                            // (the pair exists while the car's box meets the wide capsule's)
                            if dist > 1e-3 && dist < c.foot.wide * c.scale + ext(d3 / dist) {
                                let dn = d3 / dist;
                                let closing = v.dot(dn);
                                // (the deflection pass's event, `F-35508` A-72075: once a pass, the type set's event 0 at the
                                // prop's matrix x its offset, the car's velocity; no speed test. The pass is armed again when the car
                                // is out of reach, A-11977.)
                                {
                                    let l0 = (d3 - v * (v.dot(d3) / v.length_squared().max(1e-6))).normalize_or_zero();
                                    let reach = (closing * 0.5).max(2.0 * (ext(l0) + c.foot.half.x * c.scale + 0.1));
                                    if dist <= reach && v.length() >= if human { 25.0 } else { 0.0 } {
                                        if self.defl_done.insert(ci) {
                                            if let Some(e) = type_events(c.foot.audio).first() {
                                                let at = c.pos + c.rot * Vec3::from(e.offset);
                                                self.fx.push(crate::physics::loose::FxPlay { effect: e.effect.clone(), at: crate::physics::loose::FxAt::World(at), vel: st.vel });
                                            }
                                        }
                                    } else if dist > reach {
                                        self.defl_done.remove(&ci);
                                    }
                                }
                                let l = (d3 - v * (v.dot(d3) / v.length_squared().max(1e-6))).normalize_or_zero();
                                let clear = ext(l) + c.foot.half.x * c.scale + 0.1;
                                if dist <= (closing * 0.5).max(2.0 * clear) && v.length() >= if human { 25.0 } else { 0.0 } {
                                    let lat = l.dot(d3);
                                    let overlap = clear - lat;
                                    if overlap > 0.0 && lat >= 0.0 {
                                        let t = (dist - (ext(dn) + c.foot.half.x * c.scale + 0.1)) / closing;
                                        let frac = if closing > 0.0 && t > 0.0 { (dt / t).clamp(0.01, 1.0) } else { 0.01 };
                                        let p = -l * (overlap * frac);
                                        let astern = -fwd.dot(dn);
                                        let w = { let c = clamp_length([v.x, v.y, v.z, 0.0], 0.0, 25.0); Vec3::new(c[0], c[1], c[2]) }.cross(p) * (-astern.signum() * astern.abs().powf(0.3) * 1.5);
                                        st.correct_position(p);
                                        st.vel += p * (60.0 * dt);
                                        st.ang_vel += w * dt;
                                        // (F-35508, after the push: a human's pad rumble 1 - unless a racing
                                        // wheel took the side push, F-17707; none in the port)
                                        if human {
                                            st.rumble.play(1);
                                        }
                                    }
                                    continue;
                                }
                            }
                        }
                        let bx3 = c.rot * Vec3::X;
                        let bz3 = c.rot * Vec3::Z;
                        let bx = Vec2::new(bx3.x, bx3.z).normalize_or(Vec2::X);
                        let bz = Vec2::new(bz3.x, bz3.z).normalize_or(Vec2::Y);
                        let off = c.rot * Vec3::new(c.foot.center.x, 0.0, c.foot.center.y) * c.scale;
                        let cb = Vec2::new(c.pos.x + off.x, c.pos.z + off.z);
                        let ca = Vec2::new(st.pos.x, st.pos.z);
                        let Some((n, depth)) = obb_overlap(ca, [r, f], ha, cb, [bx, bz], c.foot.half * c.scale) else { continue };
                        let n3 = Vec3::new(n.x, 0.0, n.y);
                        let vn = st.vel.dot(n3);
                        let speed = st.vel.length();
                        if c.foot.kind == Solidity::Dynamic && (speed > 1.5 || c.foot.mass < 30.0) {
                            // knock it away; the car loses a little momentum
                            let ratio = (c.foot.mass / car.model.params.mass.max(300.0)).min(0.07);
                            let dir = if speed > 0.5 { st.vel / speed } else { -n3 };
                            self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
                            let jitter = ((self.rng >> 8) & 0xffff) as f32 / 65535.0 - 0.5;
                            let side = Vec3::new(-dir.z, 0.0, dir.x);
                            let tall = c.foot.height > 2.2 * c.foot.half.max_element().max(0.2);
                            let vel = st.vel * 1.1 + dir * 1.5 + side * jitter * (1.0 + speed * 0.25) + Vec3::Y * (1.2 + speed * if tall { 0.05 } else { 0.11 }).min(7.0);
                            let spin_axis = Vec3::Y.cross(dir).normalize_or(Vec3::X);
                            let ang = spin_axis * (speed * if tall { 0.25 } else { 0.6 }).clamp(1.0, 9.0) + Vec3::Y * jitter * 6.0;
                            let h = c.foot.height * c.scale;
                            let com_local = Vec3::new(c.foot.center.x, c.foot.y0 + c.foot.height * 0.5, c.foot.center.y) * c.scale;
                            let rest = (c.foot.half.min_element().min(c.foot.height * 0.5) * c.scale).max(0.06).min(h * 0.5);
                            // (an unseen box has no instance to tumble: it is just out of the way)
                            if c.inst != u32::MAX {
                                self.debris.push(Debris { collider: ci, com: c.pos + c.rot * com_local, vel, rot: c.rot, ang, rest, age: 0.0 });
                            }
                            c.knocked = true;
                            self.knocked += 1;
                            self.hits.push(crate::sound::PropHit { pos: c.pos, kind: c.foot.snd, model: c.foot.audio, speed, knocked: true });
                            st.vel *= 1.0 - ratio * 0.9;
                            st.last_impact = st.last_impact.max(speed * ratio * 2.0);
                            // (mcCarDamage's contact callback, world_damage_rule.md: the change of
                            // velocity the touch gave the car, whole - a loose prop is not fixed -
                            // at the first contact's point, times +12 = 1)
                            let at = Vec3::new(c.pos.x, st.pos.y + 0.5, c.pos.z);
                            st.contact_hit(car.tuned.as_deref().unwrap_or(&car.model.params), speed * ratio * 0.9, 1.0, at);
                        } else {
                            // immovable: push the car out and bounce it off
                            st.correct_position(n3 * depth);
                            if vn < -1.5 {
                                self.hits.push(crate::sound::PropHit { pos: c.pos, kind: c.foot.snd, model: c.foot.audio, speed: -vn, knocked: false });
                            }
                            if -vn > 0.1 {
                                // (the impact report, `F-81755`: the type's events from their least speed, at the point on the prop
                                // nearest the car (its box's face along the normal, at the car's height) + the offset; I: the original's
                                // contact point is the hull's)
                                let ext = (bx.dot(n).abs() * c.foot.half.x + bz.dot(n).abs() * c.foot.half.y) * c.scale;
                                let y = (st.pos.y + 0.5).clamp(c.pos.y, c.pos.y + c.foot.height * c.scale);
                                let pt = Vec3::new(cb.x + n.x * ext, y, cb.y + n.y * ext);
                                for e in type_events(c.foot.audio) {
                                    if -vn >= e.min_speed {
                                        self.fx.push(crate::physics::loose::FxPlay { effect: e.effect.clone(), at: crate::physics::loose::FxAt::World(pt + Vec3::from(e.offset)), vel: Vec3::ZERO });
                                    }
                                }
                            }
                            if vn < 0.0 {
                                let tang = st.vel - n3 * vn;
                                let before = st.vel;
                                st.vel = tang * 0.92 - n3 * vn * 0.18;
                                // (the callback as above: a fixed prop's contact counts 0.4 of itself,
                                // a loose one that stands - the parked aircraft - whole)
                                let at = Vec3::new(cb.x, st.pos.y + 0.5, cb.y);
                                let k = if c.foot.fixed { f32::from_bits(0x3ecc_cccd) } else { 1.0 };
                                st.contact_hit(car.tuned.as_deref().unwrap_or(&car.model.params), (st.vel - before).length() * k, 1.0, at);
                                let arm = Vec3::new(cb.x - st.pos.x, 0.0, cb.y - st.pos.z);
                                let j = n3 * (-vn);
                                st.ang_vel.y += (arm.z * j.x - arm.x * j.z) * 0.28;
                                st.ang_vel.y = st.ang_vel.y.clamp(-6.0, 6.0);
                                st.last_impact = st.last_impact.max(-vn);
                            }
                        }
                    }
                }
            }
        }
        // tumbling props
        let Some(buf) = self.inst_buf.as_ref() else { return };
        let mut i = 0;
        while i < self.debris.len() {
            let d = &mut self.debris[i];
            let c = &self.colliders[d.collider as usize];
            d.age += dt;
            d.vel.y -= 9.81 * dt;
            d.com += d.vel * dt;
            let w = d.ang.length();
            if w > 1e-4 {
                d.rot = (Quat::from_axis_angle(d.ang / w, w * dt) * d.rot).normalize();
            }
            let mut resting = false;
            if let Some(hit) = world.collider.ground(d.com, 1.5, 3.0) {
                let floor = hit.point.y + d.rest;
                if d.com.y < floor {
                    d.com.y = floor;
                    if d.vel.y < 0.0 {
                        d.vel.y = -d.vel.y * 0.28;
                    }
                    let k = (1.0 - 5.0 * dt).max(0.0);
                    d.vel.x *= k;
                    d.vel.z *= k;
                    d.ang *= (1.0 - 4.0 * dt).max(0.0);
                    // tall props topple until they lie flat
                    let yw = d.rot * Vec3::Y;
                    let tall = c.foot.height > 2.2 * c.foot.half.max_element().max(0.2);
                    if tall && yw.y.abs() > 0.08 {
                        let axis = Vec3::Y.cross(yw);
                        let axis = if axis.length_squared() > 1e-5 { axis.normalize() } else { Vec3::X };
                        d.ang += axis * 14.0 * yw.y.abs() * dt * yw.y.signum();
                    } else if d.vel.length() < 0.35 && d.ang.length() < 0.6 {
                        resting = true;
                    }
                }
            } else if d.age > 6.0 {
                resting = true;
            }
            // write the instance back (origin = centre of mass - rotated local offset)
            let com_local = Vec3::new(c.foot.center.x, c.foot.y0 + c.foot.height * 0.5, c.foot.center.y) * c.scale;
            let pos = d.com - d.rot * com_local;
            let inst = Inst { pos_scale: [pos.x, pos.y, pos.z, c.scale], rot_tint: [d.rot.x, d.rot.y, d.rot.z, d.rot.w], fill: [0; 4], ..Inst::NONE };
            self.insts[c.inst as usize] = inst;
            gpu.queue.write_buffer(buf, c.inst as u64 * std::mem::size_of::<Inst>() as u64, bytemuck::bytes_of(&inst));
            if resting || d.age > 12.0 {
                self.debris.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
}

impl Props {
    /// The loose props as the cars' shared physics level has them (physics/loose.rs): a prop whose
    /// instance the level has hit is knocked (its sound once), and drawn at its body's place.
    pub fn follow_level(&mut self, gpu: &Gpu, level: &crate::physics::loose::LooseState, lw: &crate::physics::loose::LooseWorld) {
        let key = |p: Vec3| ((p.x * 20.0).round() as i32, (p.y * 20.0).round() as i32, (p.z * 20.0).round() as i32);
        if self.level_map.is_none() {
            let mut m = HashMap::new();
            for (ci, c) in self.colliders.iter().enumerate() {
                if c.inst != u32::MAX && c.foot.kind == Solidity::Dynamic {
                    m.insert(key(c.pos), ci as u32);
                }
            }
            self.level_map = Some(m);
        }
        let Some(buf) = self.inst_buf.as_ref() else { return };
        // (a prop that broke: the model follows its heaviest moving piece - the port draws a prop whole,
        // so the part that stays, a stump at most, is not drawn on its own)
        // (a piece taken out of the simulator asleep, `dormant`, lies at its last matrix and still counts)
        let mut best: HashMap<u32, (f32, usize)> = HashMap::new();
        for (k, o) in level.objs.iter().enumerate() {
            let Some(i) = o.inst.or(o.entry.and_then(|e| e.from)) else { continue };
            let Some(mass) = o.body.as_ref().map(|b| b.mass).or(if o.dormant { o.entry.map(|e| e.mass) } else { None }) else { continue };
            let Some((fi, _)) = lw.instances.get(i as usize) else { continue };
            let at = Vec3::new(fi.matrix[3][0], fi.matrix[3][1], fi.matrix[3][2]);
            let Some(&ci) = self.level_map.as_ref().unwrap().get(&key(at)) else { continue };
            if best.get(&ci).map_or(true, |&(m, _)| mass > m) {
                best.insert(ci, (mass, k));
            }
        }
        for (ci, (_, k)) in best {
            let o = &level.objs[k];
            let c = &mut self.colliders[ci as usize];
            if !c.knocked {
                c.knocked = true;
                self.knocked += 1;
                self.hits.push(crate::sound::PropHit { pos: c.pos, kind: c.foot.snd, model: c.foot.audio, speed: o.body.map_or(0.0, |b| b.v.length()), knocked: true });
            }
            let m = o.matrix;
            let rot = Quat::from_mat3(&glam::Mat3::from_cols(m[0].normalize_or(Vec3::X), m[1].normalize_or(Vec3::Y), m[2].normalize_or(Vec3::Z))).normalize();
            let mut inst = self.insts[c.inst as usize];
            inst.pos_scale = [m[3].x, m[3].y, m[3].z, c.scale];
            inst.rot_tint = [rot.x, rot.y, rot.z, rot.w];
            self.insts[c.inst as usize] = inst;
            gpu.queue.write_buffer(buf, c.inst as u64 * std::mem::size_of::<Inst>() as u64, bytemuck::bytes_of(&inst));
        }
    }
}

/// Side of a far card's picture (one view), in texels.
const CARD: usize = 256;

/// The far card of a plant: two upright quads through the model's axis (drawn from both sides),
/// each with a picture of the model seen square on - along z on the left half of the image,
/// along x on the right. The original renders such pictures every frame (one 128 x 128 target
/// per plant type) and turns one card per plant to the camera; here they are made once from
/// the model's own triangles and textures. Returns vertices, indices and the image.
fn bake_card(prop: &mcla_formats::rage::props::Prop, texture: &mut dyn FnMut(&str) -> Option<(usize, usize, Vec<u8>)>) -> Option<(Vec<Vertex>, Vec<u32>, Texture)> {
    let (mn, mx) = (prop.min, prop.max);
    let size = mx - mn;
    if prop.meshes.is_empty() || !size.is_finite() || size.min_element() <= 0.01 {
        return None;
    }
    let (w, h) = (CARD * 2, CARD);
    let mut rgba = vec![0u8; w * h * 4];
    let mut depth = vec![f32::NEG_INFINITY; w * h];
    // a 2 texel border keeps the two views out of each other's mip levels
    let span = (CARD - 4) as f32;
    for (mesh, si) in &prop.meshes {
        let sh = prop.shaders.get(*si as usize);
        let cutout = sh.map_or(true, |s| !s.name.to_ascii_lowercase().contains("trunk"));
        let Some((tw, th, tex)) = sh.and_then(|s| s.textures().next()).and_then(|t| texture(t.1)) else { continue };
        if tw == 0 || th == 0 {
            continue;
        }
        for view in 0..2 {
            // (across, up, towards the eye) of a model point, across and up in texels
            let project = |p: Vec3| -> Vec3 {
                let (a, lo, len, d) = if view == 0 { (p.x, mn.x, size.x, p.z) } else { (p.z, mn.z, size.z, p.x) };
                Vec3::new(2.0 + (a - lo) / len * span + (view * CARD) as f32, 2.0 + (mx.y - p.y) / size.y * span, d)
            };
            for t in mesh.indices.chunks_exact(3) {
                let idx = [t[0] as usize, t[1] as usize, t[2] as usize];
                if idx.iter().any(|i| *i >= mesh.positions.len() || *i >= mesh.uv0.len()) {
                    continue;
                }
                let p = idx.map(|i| project(mesh.positions[i]));
                let uv = idx.map(|i| mesh.uv0[i]);
                let area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[2].x - p[0].x) * (p[1].y - p[0].y);
                if area.abs() < 1e-6 {
                    continue;
                }
                let x0 = p.iter().map(|q| q.x).fold(f32::INFINITY, f32::min).floor().max((view * CARD) as f32) as usize;
                let x1 = (p.iter().map(|q| q.x).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min((view + 1) * CARD - 1);
                let y0 = p.iter().map(|q| q.y).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
                let y1 = (p.iter().map(|q| q.y).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min(h - 1);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                        let b0 = ((p[1].x - fx) * (p[2].y - fy) - (p[2].x - fx) * (p[1].y - fy)) / area;
                        let b1 = ((p[2].x - fx) * (p[0].y - fy) - (p[0].x - fx) * (p[2].y - fy)) / area;
                        let b2 = 1.0 - b0 - b1;
                        if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                            continue;
                        }
                        let d = p[0].z * b0 + p[1].z * b1 + p[2].z * b2;
                        let o = y * w + x;
                        if d <= depth[o] {
                            continue;
                        }
                        let u = uv[0] * b0 + uv[1] * b1 + uv[2] * b2;
                        let tx = ((u.x - u.x.floor()) * tw as f32) as usize % tw;
                        let ty = ((u.y - u.y.floor()) * th as f32) as usize % th;
                        let c = &tex[(ty * tw + tx) * 4..(ty * tw + tx) * 4 + 4];
                        if cutout && c[3] < 128 {
                            continue;
                        }
                        depth[o] = d;
                        rgba[o * 4..o * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                    }
                }
            }
        }
    }
    // the plant's mean colour under the empty texels, so that the edges do not darken
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for px in rgba.chunks_exact(4).filter(|px| px[3] > 0) {
        for c in 0..3 {
            sum[c] += px[c] as u64;
        }
        n += 1;
    }
    if n < 24 {
        return None;
    }
    let mean = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8];
    for px in rgba.chunks_exact_mut(4).filter(|px| px[3] == 0) {
        px[..3].copy_from_slice(&mean);
    }
    // mip levels (colour weighted by coverage), stored the way the disc's 8888 textures are: A R G B
    let mut levels: Vec<Vec<u8>> = Vec::new();
    let (mut lw, mut lh, mut cur) = (w, h, rgba);
    loop {
        levels.push(cur.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect());
        if lh <= 8 {
            break;
        }
        let (nw, nh) = (lw / 2, lh / 2);
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                let (mut c, mut a) = ([0u32; 3], 0u32);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let o = ((y * 2 + dy) * lw + x * 2 + dx) * 4;
                    let al = cur[o + 3] as u32;
                    for k in 0..3 {
                        c[k] += cur[o + k] as u32 * al.max(1);
                    }
                    a += al;
                }
                let o = (y * nw + x) * 4;
                for k in 0..3 {
                    next[o + k] = (c[k] / a.max(4)).min(255) as u8;
                }
                // (a little over the mean: the cut-out test at one half should keep the crown's outline)
                next[o + 3] = ((a as f32 / 4.0) * 1.25).min(255.0) as u8;
            }
        }
        (lw, lh, cur) = (nw, nh, next);
    }
    let image = Texture { name: format!("card:{}", prop.name), width: w as u32, height: h as u32, format: mcla_formats::rage::xenos::TexFormat::Rgba8, levels, fetch: [0; 6] };
    // the two quads, wound both ways
    let (u0, u1) = (2.0 / w as f32, (CARD - 2) as f32 / w as f32);
    let (v0, v1) = (2.0 / h as f32, (CARD - 2) as f32 / h as f32);
    let mut verts = Vec::new();
    let mut indices = Vec::new();
    let mut quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3, u: [f32; 2]| {
        let base = verts.len() as u32;
        for (p, uv) in [(a, [u[0], v1]), (b, [u[1], v1]), (c, [u[1], v0]), (d, [u[0], v0])] {
            verts.push(Vertex { pos: p.to_array(), normal: [0.0, 1.0, 0.0], uv0: uv, uv1: uv, color: [255, 255, 255, 0], color1: [0, 0, 0, 255], uv2: [0.0, 0.0] });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3, base, base + 2, base + 1, base, base + 3, base + 2]);
    };
    quad(Vec3::new(mn.x, mn.y, 0.0), Vec3::new(mx.x, mn.y, 0.0), Vec3::new(mx.x, mx.y, 0.0), Vec3::new(mn.x, mx.y, 0.0), [u0, u1]);
    quad(Vec3::new(0.0, mn.y, mn.z), Vec3::new(0.0, mn.y, mx.z), Vec3::new(0.0, mx.y, mx.z), Vec3::new(0.0, mx.y, mn.z), [0.5 + u0, 0.5 + u1]);
    Some((verts, indices, image))
}

fn plant_mesh_shadow() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MCLA_PLANT_MESH_SHADOW").map_or(false, |v| v != "0"))
}

/// The vector length clamp F-38357 in place on a vec4, as the original's code does it (the deflection pass calls it with
/// 0 and 25): the squared length in single precision from fused multiply-adds (each rounded to single), above max^2 the
/// vector is scaled by max x 1/length, below min^2 by min x 1/length, a (nearly) zero vector becomes (0, min, 0, 0), a
/// NaN or infinite squared length too; the 1/length is `1/sqrt` rounded to single, two Newton steps in double with fused
/// multiply-adds, rounded to single, and 0 when the squared length is not above zero. All four lanes, w too, are scaled.
/// Checked against the original's code run offline on 3,091 vectors (tools/clamp_length_reference.py).
pub fn clamp_length(v: [f32; 4], min: f32, max: f32) -> [f32; 4] {
    let (x, y, z) = (v[0] as f64, v[1] as f64, v[2] as f64);
    let yy = (y * y) as f32;
    let l1 = x.mul_add(x, yy as f64) as f32;
    let l = z.mul_add(z, l1 as f64) as f32;
    let unit_min = [0.0 * min, 1.0 * min, 0.0 * min, 0.0 * min];
    let scale = |k: f32| -> [f32; 4] {
        let ld = l as f64;
        let r0 = (1.0 / ld.sqrt()) as f32 as f64;
        let f6 = -(r0 * r0).mul_add(ld, -1.0);
        let f5 = f6.mul_add(r0 * 0.5, r0);
        let f1 = -(f5 * f5).mul_add(ld, -1.0);
        let r = f1.mul_add(f5 * 0.5, f5) as f32 as f64;
        let r = if -ld >= 0.0 { 0.0 } else { r };
        let s = (r * k as f64) as f32;
        [v[0] * s, v[1] * s, v[2] * s, v[3] * s]
    };
    if l > max * max {
        scale(max)
    } else if l < min * min {
        if l > 9.999999960041972e-13 {
            scale(min)
        } else {
            unit_min
        }
    } else if l.to_bits() & 0x7F80_0000 == 0x7F80_0000 {
        unit_min
    } else {
        v
    }
}

#[cfg(test)]
mod clamp_length_tests {
    use super::*;

    /// F-38357 against the original's own code run offline on 3,091 vectors and limit pairs (random magnitudes 1e-7 .. 1e3,
    /// zero lanes, a non-zero w, zero / tiny / NaN / infinite / huge vectors, equal and zero limits): every lane bit for bit
    /// (a NaN lane only needs to be a NaN).
    #[test]
    fn clamp_length_matches_the_original() {
        let h = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for l in include_str!("testdata/clamp_length_ref.txt").lines() {
            let (a, b) = l.split_once(" -> ").unwrap();
            let a: Vec<f32> = a.split(' ').map(h).collect();
            let want: Vec<f32> = b.split(' ').map(h).collect();
            let got = clamp_length([a[0], a[1], a[2], a[3]], a[4], a[5]);
            for k in 0..4 {
                assert!(got[k].to_bits() == want[k].to_bits() || (got[k].is_nan() && want[k].is_nan()), "{l}: lane {k} {:08x}", got[k].to_bits());
            }
            n += 1;
        }
        assert!(n > 3000);
    }
}

#[cfg(test)]
mod lod_option_tests {
    use super::*;
    /// lod_rule.md at the 25 degree lens: props 360.36 R (cap 2250), plants' model 20.31 R and cards 366.7 R;
    /// DRAW DISTANCE scales the far ends (and the caps), DETAIL LEVEL the plant's model distance.
    #[test]
    fn reach_by_the_options() {
        let m = |kind| PropModel { batches: Vec::new(), radius: 1.0, sphere: 1.0, kind };
        let fov = 25f32.to_radians();
        let (n, f) = m(0).reach(1.0, fov, Lod::ORIGINAL);
        assert!((f - 360.36).abs() < 0.05 && n == f, "{f}");
        let (_, f2) = m(0).reach(1.0, fov, Lod { draw: 2.0, detail: 1.0, ..Lod::ORIGINAL });
        assert!((f2 - 2.0 * f).abs() < 0.01);
        assert_eq!(m(0).reach(10.0, fov, Lod::ORIGINAL).1, 2250.0);
        assert_eq!(m(0).reach(10.0, fov, Lod { draw: 1.5, detail: 1.0, ..Lod::ORIGINAL }).1, 3375.0);
        let (n, f) = m(1).reach(1.0, fov, Lod::ORIGINAL);
        assert!((n - 20.31).abs() < 0.02 && (f - 366.7).abs() < 0.1, "{n} {f}");
        let (n3, f3) = m(1).reach(1.0, fov, Lod { draw: 1.0, detail: 3.0, ..Lod::ORIGINAL });
        assert!((n3 - 3.0 * n).abs() < 0.01 && f3 == f);
        // paused (F-31789): the model angle x 0.9 (0x82007FA8) - 2R / tan(12.5 x 0.9 deg x pi/400)
        let (np, _) = m(1).reach(1.0, fov, Lod { plant_angle: 0.9, ..Lod::ORIGINAL });
        assert!((np - 2.0 / (11.25f32 * std::f32::consts::PI / 400.0).tan()).abs() < 0.01 && (np - 22.58).abs() < 0.02, "{np}");
        // hedges: high level to 2R / tan(h pi/600) (30.5 R), gone at 2R / tan(h pi/2400) (122.2 R)
        let (l, hf) = m(2).reach(1.0, fov, Lod::ORIGINAL);
        assert!((l - 30.54).abs() < 0.05 && (hf - 122.2).abs() < 0.1, "{l} {hf}");
        assert_eq!(m(5).reach(1.0, fov, Lod::ORIGINAL), (0.0, hf));
        // the cards end at the main view's far clip, 6000 m (0x82037480)
        let big = PropModel { batches: Vec::new(), radius: 20.0, sphere: 20.0, kind: 1 };
        assert_eq!(big.reach(2.0, fov, Lod::ORIGINAL).1, 6000.0);
    }
}
