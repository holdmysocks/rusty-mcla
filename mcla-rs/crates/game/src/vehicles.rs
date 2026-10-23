//! Vehicle system: GPU car models shared between instances, player + traffic cars,
//! spawning/despawning around the player, car-car separation.

use crate::ai::AiDriver;
use crate::physics::{CarInput, CarParams, CarState};
use crate::render::{Gpu, GpuTexture, MatMode, ObjectBinding, ObjectUniform, Vertex};
use crate::vehicle_model::{self, CarAsset, CarConfig, Slot};
use crate::world::{GpuBatch, World};
use anyhow::Result;
use glam::{Mat4, Quat, Vec3};
use std::path::Path;
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[path = "traffic.rs"]
pub mod traffic;
#[path = "parked.rs"]
pub mod parked;

/// The glow table's kind of a `glow<name>0` bone of a car part's skeleton (every bone of that name
/// is the place of one glow of the kind): 0 brake, 1 tail, 2 signal, 3 head, 4 high beam,
/// 6 reversing, 8 fog, 12 third brake, 13 / 14 the police lamps, 15 .. 18 the turn signals front
/// left, front right, rear left, rear right - the executable's names by kind at 0x827E7638 (5, 7,
/// 9 .. 11 "noglow").
fn glow_bone_kind(name: &str) -> Option<u8> {
    let p = name.trim_end_matches(|c: char| c.is_ascii_digit());
    Some(match p {
        "glowbrake" => 0,
        "glowtail" => 1,
        "glowsignal" => 2,
        "glowhead" => 3,
        "glowhigh" => 4,
        "glowreverse" => 6,
        "glowfog" => 8,
        "glowthirdbrake" => 12,
        "coplightred" => 13,
        "coplightblue" => 14,
        "signalfrontleft" => 15,
        "signalfrontright" => 16,
        "signalrearleft" => 17,
        "signalrearright" => 18,
        _ => return None,
    })
}

pub struct CarModel {
    pub name: String,
    pub batches: Vec<GpuBatch>,
    /// the lamps: type (the id of the game's glow table: 0 brake, 1 tail, 3 head, 6 reverse,
    /// 12 third brake, 13 / 14 police) and the middle of each lamp face in car space
    pub lamps: Vec<(u8, Vec3)>,
    /// the lamps' glow places as the car's own part files give them (the bones under `glowbrake0`,
    /// `glowhead0` ... of each fitted part's skeleton - what the original's light set holds, one
    /// record a bone, read out of the running game for the R8): the kind of the game's glow table
    /// and the bone's place in car space. Render only (the glow pass, `halo_lamps`).
    pub glow_bones: Vec<(u8, Vec3, Vec3)>,
    pub params: CarParams,
    /// the same car with every performance upgrade fitted (the tuning file's "mods" set)
    pub params_mods: Option<CarParams>,
    /// the original's simulation tuning (base and mods), when the car has it
    pub simtunes: Option<Arc<mcla_formats::rage::simtune::SimTunes>>,
    /// what the computer drivers know of the car (the game's own calibration tables)
    pub aitunes: Option<Arc<mcla_formats::rage::aitune::AiTunes>>,
    /// the height the wheels' centres were modelled at (mean of front and rear)
    pub rest_height: Option<f32>,
    /// the car's own stock wheels (`resources/rims/whl_stk_<car>`), when the disc has them
    pub wheels: Option<Arc<WheelModel>>,
    /// the car's brakes behind them (`resources/brakes/brk_stk_<car>`, else `brk_car_base`)
    pub brakes: Option<Arc<BrakeModel>>,
    pub brake_dir: Option<std::path::PathBuf>,
    /// where those came from, and the rim's share of the wheel diameter (stock size)
    pub rim_dir: Option<std::path::PathBuf>,
    pub rim_ratio: f32,
    /// motorcycle: one front and one rear wheel on the centre line, leans into corners
    pub bike: bool,
    pub min: Vec3,
    pub max: Vec3,
    pub cfg: CarConfig,
    /// (slot, variant names) for the garage
    pub variants: Vec<(Slot, Vec<String>)>,
    /// display labels matching `variants`
    pub variant_labels: Vec<(Slot, Vec<String>)>,
    /// extra garage rows with a placed fragment on this car
    pub extra_variants: std::collections::HashSet<(String, usize)>,
    /// per batch: 0, or 1 / 2 for the left / right side window's glass (drawn with the car's
    /// window object, `CarInstance::window_bindings`), 3 hood, 4 trunk lid, 5 rear bumper, 6 front
    /// bumper and intercooler (drawn with the car's part objects, `CarInstance::part_bindings`)
    pub batch_window: Vec<u8>,
    /// the side windows' clips (left, right), where the base fragment has them
    pub windows: [Option<vehicle_model::WindowClip>; 2],
    /// the damage map's layout texture (`<car>_maxdamage`: width, height, RGBA8), when the car has it
    pub damage_layout: Option<Arc<(u32, u32, Vec<u8>)>>,
    /// the moving damage parts (`damageparts.rs`): the car's movement settings and bumper slot bones, and the bones
    /// the hood (slot "hood", tag 33332) and the trunk lid (`trunk_0`) turn about, in car space
    pub part_rig: crate::damageparts::PartRig,
    pub hood_slot: Option<Mat4>,
    pub trunk_bone: Option<Mat4>,
    /// a traffic model's own wheels (tyre, rim, disc in its body fragment, on its wheel
    /// bones `wheel_0` .. `wheel_5` - `CarPart::mesh_wheel`): their hubs; drawn as batch groups 7 - 12 turned about these -
    /// no rim set of `resources/rims` is put on
    pub own_wheels: Option<[Option<Vec3>; 6]>,
    /// a traffic model's LOD 1 and LOD 2 (its base fragments 0x7b0cd281 / 0x0de3ddb3: the type's drawables at
    /// type +164 / +168, sub_82791A50), each with its batches' groups as `batch_window`; empty for other cars
    pub lods: [(Vec<GpuBatch>, Vec<u8>); 2],
    /// the radius the LOD choice adds to the depth (car +492, sub_826EAD00: 0.5 x sqrt 2 x the larger of the
    /// bound's width and length - sub_826D9818: its box max - min, x and z; every "car" type, vtable 0x820C0FD4,
    /// answers 0 to the type's +28, so the 0.5 x width of the "ped" types never applies)
    pub lod_radius: f32,
    /// the middle of that sphere in the car's space (the bound's centroid, sub_826E9F88)
    pub lod_centre: Vec3,
    /// the depths of the LOD 0 -> 1 and 1 -> 2 steps (the type's `mcAmbientRenderType.xml`, `traffic::lod_steps`)
    pub lod_steps: [f32; 2],
}

pub struct WheelModel {
    pub batches: Vec<GpuBatch>,
    /// where the rim's spokes stand along the axle (the mean x of zone 1, in the unit
    /// wheel's own measure: -0.25 is the outer face)
    pub spoke_x: f32,
}

/// How far a brake is moved in along the axle to stand behind the spokes of a rim
/// (its own outer face is at -0.1875 of the unit wheel). (approx.) the game's rule for
/// it was not read; the gap is by eye.
fn brake_shift(spoke_x: f32) -> f32 {
    (spoke_x + 0.07 + 0.1875).clamp(0.0, 0.30)
}

/// A brake of `resources/brakes` (`brk_stk_<car>`, `brk_car_base` ...): the rotor, which
/// turns with the wheel, and the caliper, which does not.
pub struct BrakeModel {
    pub rotor: Vec<GpuBatch>,
    pub caliper: Vec<GpuBatch>,
}

/// (approx.) the rotor's diameter against the rim's, measured on the capture `g_rm_0`
const ROTOR_OF_RIM: f32 = 0.74;

/// A parked car's `settle` while its ground probe is due / after it found no ground (dropped).
const PARK_PROBE: f32 = -1.0;
const PARK_NO_GROUND: f32 = -2.0;

/// The brake model in `dir`, its unit-diameter meshes scaled to `k` of the wheel's.
fn load_brakes(gpu: &mut Gpu, world: &mut World, dir: &Path, k: f32, shift: f32) -> Option<BrakeModel> {
    let (mut r63, mut r83) = (None, None);
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        match p.extension().and_then(|x| x.to_str()) {
            Some("r63") => r63 = Some(p),
            Some("r83") => r83 = Some(p),
            _ => {}
        }
    }
    // (the shaders and the brake's own textures are in the .r83)
    let look = vehicle_model::load_simple(&r83?, None).ok();
    let mut model = vehicle_model::load_simple(&r63?, None).ok()?;
    if model.shaders.is_empty() {
        model.shaders = look.as_ref().map(|l| l.shaders.clone()).unwrap_or_default();
    }
    let xf = Mat4::from_translation(Vec3::new(shift, 0.0, 0.0)) * Mat4::from_scale(Vec3::new(1.0, k, k));
    let mut out = BrakeModel { rotor: Vec::new(), caliper: Vec::new() };
    let mut seen: Vec<(String, bool)> = Vec::new();
    for (mesh, si) in &model.meshes {
        if mesh.positions.is_empty() {
            continue;
        }
        // the rotor is the whole disc; the caliper sits on one side of it. The first of
        // each is the finest (the others are its coarser levels).
        let (lo, hi) = mesh.positions.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.z), a.1.max(p.z)));
        let is_rotor = lo < -0.4 && hi > 0.4;
        let list = if is_rotor { &mut out.rotor } else { &mut out.caliper };
        let sh = model.shaders.get(*si as usize);
        let sname = sh.map(|s| s.name.clone()).unwrap_or_default();
        // (one mesh of a kind: the caliper, its maker's mark)
        if seen.contains(&(sname.clone(), is_rotor)) {
            continue;
        }
        seen.push((sname.clone(), is_rotor));
        let name = sh.and_then(|s| s.diffuse().map(|n| n.to_ascii_lowercase())).unwrap_or_default();
        let keys = [mcla_formats::hash::joaat(&name), mcla_formats::hash::joaat(name.trim_end_matches(".dds"))];
        let own = [&model.textures, look.as_ref().map(|l| &l.textures).unwrap_or(&model.textures)].into_iter().find_map(|d| keys.iter().find_map(|k| d.get(k).map(|t| (*k, t))));
        let key = own.map_or(keys[0], |o| o.0);
        let tex = match own {
            Some((k, t)) => gpu.upload_texture(k, t),
            None if !name.is_empty() => keys.iter().find_map(|k| world.texture_any(gpu, *k)).unwrap_or_else(|| gpu.white.clone()),
            None => gpu.white.clone(),
        };
        if std::env::var("MCLA_WHEEL_DUMP").is_ok() {
            if let Some((_, t)) = own {
                let px = t.to_rgba8();
                let at = |u: f32, v: f32| { let (x, y) = (((u * t.width as f32) as usize).min(t.width as usize - 1), ((v * t.height as f32) as usize).min(t.height as usize - 1)); let i = (y * t.width as usize + x) * 4; [px[i], px[i + 1], px[i + 2], px[i + 3]] };
                let grid: Vec<Vec<[u8; 4]>> = (0..8).map(|y| (0..8).map(|x| at((x as f32 + 0.5) / 8.0, (y as f32 + 0.5) / 8.0)).collect()).collect();
                log::info!("brake texel {} at (0.943,0.942) {:?} flipped {:?} grid {:?}", name, at(0.943, 0.942), at(0.943, 0.058), grid);
            }
            log::info!("brake params {} rotor {}: {:?}", sname, is_rotor, sh.map(|s| s.params.iter().map(|(h, p)| format!("{:08x}={:?}", h, p)).collect::<Vec<_>>()));
            log::info!("brake mesh {} shader {} texture {:?} own {} rotor {} size {:?} means {:?} uv {:?}", dir.display(), sname, name, own.is_some(), is_rotor, own.map(|o| (o.1.width, o.1.height)), own.map(|o| o.1.level_means()), mesh.uv0.iter().fold((glam::Vec2::splat(9.0), glam::Vec2::splat(-9.0)), |a, u| (a.0.min(*u), a.1.max(*u))));
        }
        // The rotor's texture is its pattern in dark grey (the shader makes the metal of
        // it); a caliper of the rotor's shader has one texel and takes its colour at run
        // time - red on the R8 (`ref/g_rm_0`), which is the only one captured. Both go
        // through the rim's branch of the shader, marked by zone: 8 rotor, 9 caliper.
        let lower = sname.to_ascii_lowercase();
        let mode = if lower.contains("alpha") { MatMode::AlphaTest } else { MatMode::Rim };
        // (uv1.y 1: the caliper's colour is its texture's - the Project Mu sets' teal is a
        // patch of the rotor's texture; where that texel is plain dark grey, as on the R8,
        // the colour is the game's own: red)
        let plain = own.map_or(false, |(_, t)| {
            let (px, uv) = (t.to_rgba8(), mesh.uv0.first().copied().unwrap_or_default());
            let (x, y) = (((uv.x.clamp(0.0, 0.999)) * t.width as f32) as usize, ((uv.y.clamp(0.0, 0.999)) * t.height as f32) as usize);
            let i = (y * t.width as usize + x) * 4;
            px.get(i..i + 3).map_or(true, |c| c.iter().max().unwrap() - c.iter().min().unwrap() < 12 && c[0] < 60)
        });
        let (zone, paint, own_tex) = if is_rotor { (8.0, [255u8, 255, 255], 0.0) } else if lower.contains("rotor") && plain { (9.0, [150, 6, 6], 0.0) } else { (9.0, [255, 255, 255], 1.0) };
        let mut b = mesh_to_batch_xf(gpu, mesh, &sname, mode, tex.clone(), tex, key, key, xf);
        // (as on the rims the first colour channel is baked occlusion, the others empty:
        // the generic path would draw the brake black)
        let verts: Vec<Vertex> = (0..mesh.positions.len())
            .map(|i| {
                let p = xf.transform_point3(mesh.positions[i]);
                let nr = xf.transform_vector3(mesh.normals.get(i).copied().unwrap_or(Vec3::Y)).normalize_or(Vec3::Y);
                let occ = mesh.colors.get(i).map_or(255, |c| c[0]);
                let (uv1, color) = if mode == MatMode::Rim { ([zone, own_tex], [paint[0], paint[1], paint[2], occ]) } else { (mesh.uv1.get(i).copied().unwrap_or_default().to_array(), [255, 255, 255, 255]) };
                Vertex { pos: p.to_array(), normal: nr.to_array(), uv0: mesh.uv0.get(i).copied().unwrap_or_default().to_array(), uv1, color, color1: [0, 0, 0, 255], uv2: [0.0, 0.0] }
            })
            .collect();
        b.vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("brake vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
        list.push(b);
    }
    (!out.rotor.is_empty()).then_some(out)
}

pub struct CarInstance {
    pub model: Arc<CarModel>,
    pub binding: ObjectBinding,
    pub wheel_bindings: Vec<ObjectBinding>,
    /// the brake calipers' objects: as the wheels', without their turning
    pub caliper_bindings: Vec<ObjectBinding>,
    pub state: CarState,
    pub input: CarInput,
    pub color: [f32; 4],
    pub ai: Option<AiDriver>,
    /// a cine car event's mcBrainAnimated drives the car (cinecar.rs): from the scene's start to its end
    pub brain: Option<crate::cinecar::AnimBrain>,
    pub placed: bool,
    pub is_player: bool,
    /// aftermarket wheels fitted in the garage (player only)
    pub wheels: Option<Arc<WheelModel>>,
    /// visual lean angle of a motorcycle (rad)
    pub lean: f32,
    /// performance upgrade level 0..1 and the blended parameters for it
    pub perf: f32,
    pub tuned: Option<Box<CarParams>>,
    /// police cruiser
    pub cop: bool,
    /// seconds the car is knocked out (EMP / Roar): no throttle, no steering
    pub stunned: f32,
    /// parked at the kerb: index of its parking spot (`parked::ITEM_SPOT`: parked for a layout
    /// item, no spot)
    pub parked: Option<usize>,
    /// the layout item it stands for (car +40, parked.rs)
    pub item: Option<u32>,
    /// a layout item's car: when it was made and how long it lives at least (car +464 / +468,
    /// 8 + 4U s, sub_826EAD00)
    pub amb_born: f32,
    pub amb_life: f32,
    /// a car on its way out (car +500 bit 0x4, set with the reason at +488 by sub_826E9588 - only the parked
    /// unspawner sub_826F2A30 does): its fade (+484), falling by 2 a second until the car is removed
    /// (sub_826E9AD0 -> sub_826D50E8); meanwhile its LOD 1 is drawn as LOD 2 (`pick_lods`)
    pub removing: Option<f32>,
    /// seconds of physics left before a parked car is frozen in place
    pub settle: f32,
    /// roaming street racer (free roam): can be challenged
    pub racer: bool,
    /// takes part in the running event: never despawned, not counted as traffic
    pub pinned: bool,
    /// seconds of nitrous an AI car has been told to use (race scripts)
    pub boost: f32,
    /// character id of a named rival at the wheel ("GH_06"), else empty
    pub driver: String,
    /// licence plate text (set by whoever spawns the car; else a random Californian one)
    pub plate_text: String,
    /// plate texture with the text on it
    pub plate: Option<Arc<wgpu::BindGroup>>,
    /// car set-up whose livery the body wears ("GH_06_0_vp_dsn_280z_75"), else empty
    pub vinyl_name: String,
    pub vinyl: Option<Arc<wgpu::BindGroup>>,
    vinyl_done: bool,
    /// the wheels of this car's own set-up (a rival's `.xcc`), where it has one
    pub setup: Option<mcla_formats::rage::simtune::WheelFit>,
    pub pose_history: crate::vehicle_pose::History,
    /// step 19d: the factor on the damage this vehicle inflicts on another it hits
    /// (mcCarDamage +20, Racer_SetDamageInflictedFactor; 1 by default)
    pub inflict: f32,
    /// its number in the session (the order cars are made in: their place among the physics level's indices)
    pub uid: u32,
    /// one of the payback client's friends (made through the police manager without being
    /// police: no light bar, no siren)
    pub goon: bool,
    /// what the car's set-up paints on it (not the player's car: that is `player_*`)
    pub look: Option<Arc<CarLook>>,
    /// the brake model of its set-up
    pub brake: Option<Arc<BrakeModel>>,
    /// the set-up in `vinyl_name` has been put on the car (`Vehicles::dress_setups`)
    pub dressed: bool,
    /// the character set at the wheel ("drv_mb_009_set": Racer_SetCharacterName), else empty
    pub driver_set: String,
    /// step 19e: the power-up control override on the car's input (the original's vehicle
    /// +624; `powerups::apply_ctl`)
    pub pu_ctl: u32,
    /// step 19f: MegaStealth - the car's archetype includes only the map's type (2048) and its
    /// wheels' probes likewise (sub_8237F8B0): it meets no other car and no prop
    pub pu_ghost: bool,
    /// step 19g: Mirror - the paint's DL6.w (sub_823802D8 -> render object +4108 -> sub_823580D0:
    /// the player's car only), added to PS_CarPaint's mirror weight
    pub pu_mirror: f32,
    /// step 19g: Stealth - the car is drawn by pu_fx.rs with PS_Stealth instead of its own look
    pub pu_stealth: bool,
    /// the side windows' objects (left, right): the body's, moved by the window's clip; empty
    /// when the model has no window clips
    pub window_bindings: Vec<ObjectBinding>,
    /// the side windows' controllers (left, right)
    pub windows: [WindowCtl; 2],
    /// the moving damage parts' objects (hood, trunk lid, rear bumper, front bumper): the body's, moved by
    /// the part; empty when the model has none of them
    pub part_bindings: Vec<ObjectBinding>,
    /// ambient traffic riding its lane as the original's (traffic.rs; None: driven by the physics)
    pub rail: Option<traffic::Rider>,
    /// a traffic car's lamp states and glow levels (`AmbLamps`)
    pub amb_lamps: AmbLamps,
    /// a parked car's front wheels' turn (traffic.rs `parked_steer`)
    pub park_steer: f32,
    /// a parked car the physics level holds as its ambient instance (loose.rs `Amb`)
    pub park_held: bool,
    /// a traffic / parked car's own wheels drawn raised by their collider wheel's compression, by
    /// wheel bone (`ambient_out_of_level`; 0 beyond 30 m)
    pub amb_lift: [f32; 6],
}

/// A traffic car's lamps as the original's mcAmbientRenderCarInstance keeps them (sub_82792FD8,
/// run while the car is drawn): five states eased towards the car's flags (sub_82375C98: on
/// v += (1 - v) x 9 x dt, 1 from 0.99; off v -= v x 9 x dt, 0 under 0.01), the lamp programs'
/// state of each entry = the state table at its attachment (`traffic_state`), and each entry's
/// glow level = that x (1 - sat((d - 50) / 325))^2 x the type's fGlowIntensityScale, d the car's
/// depth along the camera's axis. A car not drawn (out of view, or past the lamps' reach) keeps
/// what it had; one never drawn has no levels and no glows. Read out of the running game
/// 2026-10-21 (docs/LIGHTING.md "Traffic's own lamp levels").
#[derive(Clone, Copy, Default)]
pub struct AmbLamps {
    /// head (lights on), brake, the two signal sides, reversing
    pub s: [f32; 5],
    /// the glows' levels by table entry, once drawn
    pub levels: Option<[f32; crate::render::car_lamps::IDS]>,
    /// the lamp programs' state by table entry (PSAmbientCarLight's cover row w)
    pub lens: [f32; crate::render::car_lamps::IDS],
    /// the glass's tint gCarAmbTintColor.w (sub_827914B8): 0.25 + 1.5 sat(min(d / 30, 1) - 0.5), d the
    /// game camera's distance from the car - set once a frame from that camera and used in every pass
    pub tint: f32,
    /// the render instance's four side sums (+560, gCarAmbDamageParams c126): what the post-solve callback
    /// sub_826FAF98 added from each contact with a car (vehsim.rs) - never reset, decayed or clamped while
    /// the car lives; zeroed when the instance is set up (sub_82792860)
    pub dmg: [f32; 4],
    /// its type's `_maxdamage` texture is bound as the vertex programs' DmgTex
    pub dmg_bound: bool,
    /// a set-up car's lamps switched off (mcCar +597 = 0, the racer's +689): `Racer_SetHeadlights(car, 0)`
    /// of the hangout / payback scripts, the cine events `headlights_off` / `car_off` (`cine_lamp_events`);
    /// no head, brake or reversing state while it is set (sub_82353E38)
    pub off: bool,
    /// a set-up car's shift light (the state at +392, the lamp programs' 20th entry, lamp id 19)
    pub shift: f32,
    /// a set-up car's siren target (+304): its police lamps glow while it is set (`police_glows`)
    pub siren_on: bool,
}

/// sub_82792FD8's state table by attachment, from the five eased states (head, brake, signal
/// sides a / b, reversing): brake 0.25 head + 0.75 brake, tail 0.25 head, 2 brake, 3 and 5 head,
/// 7 .. 9 always 1, 11 reversing, 13 / 15 side a, 14 / 16 side b, the rest 0.
pub fn traffic_state(s: [f32; 5], att: u8) -> f32 {
    let [h, b, sa, sb, r] = s;
    match att {
        0 => 0.25 * h + 0.75 * b,
        1 => 0.25 * h,
        2 => b,
        3 | 5 => h,
        7..=9 => 1.0,
        11 => r,
        13 | 15 => sa,
        14 | 16 => sb,
        _ => 0.0,
    }
}

/// sub_823523C0's state table by attachment for a set-up car (mcCar: the player, rivals, cops), from
/// its eased states in `AmbLamps::s` (head, brake, siren, -, reversing; sub_82353E38): 0 brake 0.25 head +
/// 0.75 brake, 1 tail 0.25 head, 2 brake, 3 head + flash (sat), 5 and 6 head, 7 .. 9 the siren state (+308,
/// the police lamps 13 / 14), 11 reversing, the rest 0.
/// The flash state (+248, attachments 3 / 4) has no target but the "always rain" debug byte
/// 0x8288E5B8, so 0; attachment 5 is the state at +188, eased from the head state's target, so equal
/// to it.
pub fn setup_state(s: [f32; 5], att: u8) -> f32 {
    let [h, b, siren, _, r] = s;
    match att {
        0 => 0.25 * h + 0.75 * b,
        1 => 0.25 * h,
        2 => b,
        3 | 5 | 6 => h,
        7..=9 => siren,
        11 => r,
        _ => 0.0,
    }
}

/// The garage's head lamp state (sub_82353E38 with the light manager's byte 14546 set: the byte +39229 of
/// the garage object, which the view change sub_826B34F8 sets for the views 3 and 10 .. 13 of the table
/// at 0x828292A0): on in the cabin views, off in every other.
pub fn garage_lamps_due(view: &str) -> bool {
    matches!(view, "Interior" | "FrontSeats" | "Stereo" | "SteeringWheel" | "PostGauges")
}

/// The cine scripts' lamp switching events (generic events 22 car_on, 23 car_off, 33 headlights_on,
/// 34 headlights_off of the table at 0x827EAD60, handled by sub_823D76A8 -> sub_822A85D8 on the racer of the
/// event's car slot; `car_restore` (24) takes the flag kept at the scene's start - used by no scene):
/// (scene, on, car slot, seconds). Read from every cine script (work/lighting2/lamps/lampev.py, the
/// hash-named ones decrypted first). The scene's start keeps every racer's switch (sub_823BBE80 via
/// BeginDescription) and FinishScene puts it back (sub_823BBEE0).
pub const CINE_LAMP_EVENTS: &[(&str, bool, u8, f32)] = &[
    ("cut_cop_arrest", false, 0, 0.0),
    ("cut_cop_arrest_02", false, 0, 0.0),
    ("cut_cop_arrest_03", false, 0, 0.0),
    ("cut_cop_arrest_08", false, 0, 0.0),
    ("cut_cop_arrest_bikestand", false, 0, 0.0),
    ("cut_cop_walk_plyr_02", false, 0, 0.0),
    ("cut_cop_walk_plyr_bike", false, 0, 0.0),
    ("hangout/sc_cut_hangout_01_ent_01", false, 0, 6.66667),
    ("hangout/sc_cut_hangout_01_ext_01", true, 0, 0.0),
    ("hangout/sc_cut_hangout_02_ent_01", false, 0, 6.66667),
    ("hangout/sc_cut_hangout_02_ext_01", true, 0, 0.0),
    ("racestart/cut_payback_get_keys", false, 0, 0.0),
    ("racestart/cut_payback_get_keys", true, 0, 14.5),
    ("story/cut_story_beat_sh_02_exotic", false, 0, 2.66667),
    ("story/cut_story_beat_sh_02_exotic", false, 1, 2.66667),
    ("story/cut_story_bestof_sh_05", false, 0, 8.33333),
    ("story/cut_story_bestof_sh_05", false, 1, 8.33333),
    ("story/cut_story_city_champ", false, 0, 8.33333),
    ("story/cut_story_city_champ_night", false, 0, 8.33333),
    ("story/cut_story_district_champs", false, 0, 8.33333),
    ("story/cut_story_district_champs", true, 0, 74.3333),
    ("story/cut_story_district_champs_night", false, 0, 8.33333),
    ("story/cut_story_district_champs_night", true, 0, 74.3333),
    ("story/cut_story_find_sh_02c", false, 0, 6.66667),
    ("story/cut_story_hangout_pinkslip_downtown", false, 0, 7.3),
    ("story/cut_story_hangout_pinkslip_downtown", false, 1, 7.3),
    ("story/cut_story_hangout_pinkslip_downtown", false, 2, 7.3),
    ("story/cut_story_hangout_pinkslip_hills", false, 0, 8.33333),
    ("story/cut_story_hangout_pinkslip_hills", false, 1, 8.33333),
    ("story/cut_story_hangout_series_downtown", false, 0, 7.23333),
    ("story/cut_story_hangout_series_downtown", false, 1, 7.23333),
    ("story/cut_story_hangout_series_downtown", false, 2, 7.23333),
    ("story/cut_story_hangout_series_hollywoodc", false, 0, 27.6667),
    ("story/cut_story_hollywood_garage_intro", false, 0, 4.33333),
    ("story/cut_story_hollywood_garage_intro", false, 1, 4.33333),
    ("story/cut_story_motorcycle_rep", true, 0, 5.33333),
    ("story/cut_story_motorcycle_rep", true, 1, 5.33333),
    ("story/cut_story_payback_intro", false, 0, 5.33333),
    ("story/cut_story_payback_intro", false, 1, 5.33333),
    ("story/cut_story_payback_intro", false, 2, 5.33333),
    ("story/cut_story_race_gh_06", false, 0, 7.06667),
    ("story/cut_story_race_gh_06", true, 0, 57.6333),
    ("story/cut_story_races_in_beach", false, 0, 4.83333),
    ("story/cut_story_races_in_beach", false, 1, 4.83333),
    ("story/cut_story_rep_on_strip_garage", false, 0, 8.66667),
    ("story/cut_story_rep_on_strip_garage", false, 1, 6.0),
    ("story/cut_story_rep_on_strip_garage", true, 0, 81.3333),
    ("story/cut_story_rep_on_strip_garage", true, 0, 6.0),
];

/// sub_82792FD8's fade of a traffic car's glow levels with its depth `d` along the camera's
/// axis: (1 - sat((d - 50) / 325))^2.
pub fn traffic_fade(d: f32) -> f32 {
    (1.0 - ((d - 50.0) / 325.0).clamp(0.0, 1.0)).powi(2)
}

/// sub_82375C98: one frame of a lamp state's easing at 9 a second.
pub fn traffic_ease(v: f32, on: bool, dt: f32) -> f32 {
    ease_at(v, on, 9.0, dt)
}

/// sub_82375C98 with the state record's own rate (+0: 9; the siren's +300 18).
pub fn ease_at(v: f32, on: bool, rate: f32, dt: f32) -> f32 {
    if on {
        let n = v + (1.0 - v) * rate * dt;
        if n > 0.99 { 1.0 } else { n }
    } else {
        let n = v - v * rate * dt;
        if n < 0.01 { 0.0 } else { n }
    }
}

/// how far (camera depth, m) a traffic car's lamps are drawn and its levels kept up to date: the
/// last update seen at 125.1 m, the model's LOD 1 seen to 140.8 m - taken as 125 (approximate)
pub const TRAFFIC_LAMP_REACH: f32 = 125.0;

/// The glow file's factors by kind (`tune/vehicle/glow`, 0x0f2b8cad: `<kind>_intensity` and
/// `<kind>_player_intensity`; sub_8237AC90 takes the second for the player's car).
pub const GLOW_FACTOR: [f32; 19] = [2.0, 2.0, 1.002, 2.0, 1.002, 1.0, 0.528, 1.0, 1.002, 1.0, 1.0, 1.0, 0.346, 1.002, 1.002, 0.728, 0.728, 0.3, 0.3];
pub const GLOW_FACTOR_PLAYER: [f32; 19] = [0.3, 1.004, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];

/// A side window's controller as the original's (vtable 0x82046958; open sub_8233C3B8, close
/// sub_8233C438, update sub_8233BEC0): `state` 1 closed (up), 2 open (down), 3 opening,
/// 4 closing; `time` runs at rate 0.5 (0x823409C8) over the clip, so rolling down takes twice
/// the clip's length; the clip is played at clamp(time, 0, duration).
#[derive(Debug, Clone, Copy)]
pub struct WindowCtl {
    pub state: u8,
    pub time: f32,
}

impl Default for WindowCtl {
    fn default() -> Self {
        WindowCtl { state: 1, time: 0.0 }
    }
}

impl WindowCtl {
    pub const RATE: f32 = 0.5;

    /// `open` only from closed, `close` only from fully open (an up request while the window is
    /// still going down is dropped, as the original's)
    pub fn request(&mut self, open: bool, duration: f32) {
        match (open, self.state) {
            (true, 1) => {
                self.state = 3;
                self.time = 0.0;
            }
            (false, 2) => {
                self.state = 4;
                self.time = duration;
            }
            _ => {}
        }
    }

    /// (the original holds the time while the window motor's sound is being prepared, +72;
    /// the port has no such wait)
    pub fn update(&mut self, dt: f32, duration: f32) {
        match self.state {
            3 => {
                self.time += Self::RATE * dt;
                if self.time >= duration {
                    self.state = 2;
                }
            }
            4 => {
                self.time -= Self::RATE * dt;
                if self.time < 0.0 {
                    self.state = 1;
                }
            }
            _ => {}
        }
    }

    /// The clip time the player is set to.
    pub fn clip_time(&self, duration: f32) -> f32 {
        self.time.clamp(0.0, duration)
    }
}

/// What a set-up file paints on a car that is not the player's, as the renderer takes it
/// for the player's (`Vehicles::player_*`): the body's paint type, its second colour (w the
/// fade), the window tint, the cabin's five colour groups, the five paint areas after the
/// body, and the rims' finish. Made by `mc4::setup_look`.
#[derive(Clone, Debug, PartialEq)]
pub struct CarLook {
    pub paint_type: u8,
    pub color2: [f32; 4],
    pub tint: [f32; 4],
    pub interior: [[f32; 4]; 5],
    pub area_a: [[f32; 4]; 5],
    pub area_b: [[f32; 4]; 5],
    pub rim_finish: [[f32; 4]; 3],
}

/// A set-up as `Vehicles::dress_setups` puts it on a car: the model's parts, the rim's
/// directory and size (0: the car's own), the tire set, the brake model, the body's first
/// colour, the look, and the share of performance upgrades (None: left as it is).
#[derive(Clone, Debug)]
pub struct Dress {
    pub cfg: CarConfig,
    pub rim_dir: Option<std::path::PathBuf>,
    pub inches: u32,
    pub tire: String,
    pub brake: Option<String>,
    pub color: [f32; 4],
    pub look: CarLook,
    pub perf: Option<f32>,
    /// step 19e: the set-up's licence plate text (0x847; empty: none)
    pub plate: String,
}

impl CarInstance {
    pub fn render_pose(&self, alpha: f32) -> crate::vehicle_pose::Pose {
        self.pose_history.sample(&self.state, self.lean, alpha)
    }

    pub fn render_state(&self, alpha: f32) -> CarState {
        let pose = self.render_pose(alpha);
        let mut state = self.state.clone();
        state.pos = pose.pos;
        state.rot = pose.rot;
        for i in 0..4 {
            state.wheels[i].world_pos = pose.pos + pose.rot * pose.hubs[i];
            state.wheels[i].steer = pose.steer[i];
            state.wheels[i].spin = pose.spin[i];
        }
        state
    }

    /// Wear another livery ("" for none); the texture is fetched on the next frame.
    pub fn set_vinyl(&mut self, name: &str) {
        if self.vinyl_name != name {
            self.vinyl_name = name.to_string();
            self.vinyl = None;
            self.vinyl_done = false;
        }
    }
}

/// A car's damage map in the pool (`Vehicles::damage_maps`).
struct DmgMap {
    uid: u32,
    px: crate::damagemap::MapPixels,
    tex: wgpu::Texture,
    _view: wgpu::TextureView,
}

#[derive(Default)]
pub struct Vehicles {
    pub render_alpha: f32,
    /// each car's LOD this frame (`pick_lods`, by the cars' index): 0 .. 2, -1 not drawn
    pub lod_pick: Vec<i8>,
    /// whether the glow pass draws a car's lamp glows this frame (`pick_lods`, by the cars' index)
    pub lod_glow: Vec<bool>,
    pub models: Vec<Arc<CarModel>>,
    pub wheels: Option<Arc<WheelModel>>,
    pub cars: Vec<CarInstance>,
    pub traffic_target: usize,
    /// the ambient manager's traffic density scale (+0x64240; None = its 1.0): an ARCADE race's
    /// TRAFFIC row, 0.5 for low (sub_8265D2B8); it multiplies the region list's target
    pub density_scale: Option<f32>,
    /// solid props around the racing cars (centre, radius), refreshed by the game a few times a second
    pub solids: Vec<(Vec3, f32)>,
    /// the fixed props (physics level state 2) around the racers as the object grid's kind-3
    /// objects (sub_82735580), for the racer brains' obstacle scan; refreshed with `solids`
    pub prop_objects: Vec<crate::brainturn::ObjectInfo>,
    /// the glows of the cars' lamps this frame (instances of the glow pipeline)
    glow_buf: Option<wgpu::Buffer>,
    glow_count: u32,
    /// rim size chosen in the garage for the player's car (inches; 0 = the car's stock size)
    pub player_rim_inches: u8,
    /// the player's lamp table: its garage record's (the save's copy of its set-up), which the
    /// original draws it with; None = by model / set-up as for any car (`lamp_rows`)
    pub player_lamps: Option<Arc<crate::render::car_lamps::LampRows>>,
    /// traffic density a race script asked for on a road (a point on it, 0..5), not yet resolved
    pub road_density_new: Vec<(Vec3, i32)>,
    /// the same per lane of the road network
    pub road_density: std::collections::HashMap<usize, i32>,
    pub spawn_timer: f32,
    pub rng: u32,
    /// the traffic types' driving values (`mcAmbientType.xml`), by model name
    pub amb_types: hashbrown::HashMap<String, traffic::AmbType>,
    /// the original's traffic spawner (traffic.rs; made with the road network)
    pub spawner: Option<traffic::Spawner>,
    /// the driver cars' lanes (sub_826D99B8 on the racer's obstacle; racer +640 / +644), by uid
    pub driver_lanes: hashbrown::HashMap<u32, traffic::DriverLanes>,
    /// a scene or the vehicle pick: physics and AI are held
    pub frozen: bool,
    /// the race's grid hold (N-VP-1): seconds to GO. The cars simulate, held by their brakes - the
    /// player's chassis brake forced and his front wheels locked (`CarInput::hold`), the computer
    /// drivers at their pre-race antics (`AiDriver::grid_input`) - so revving and burnouts happen as
    /// in the original.
    pub grid_hold: Option<f32>,
    /// under the grid hold, the racers are not placed yet (the port's dive in front of the start scene):
    /// held by the grid rules with no pedals, the computer drivers' antics not begun (main.rs PLACE_TICKS)
    pub grid_wait: bool,
    /// no car has its lamps on (VEHICLE SELECT: three cars standing in a lot)
    pub lights_off: bool,
    /// the finish of the player's rims by zone - spoke A, spoke B, lip (the rim meshes'
    /// uv1.x 1..3): colour, and in w 1 painted, 2 chrome, 3 brushed (0 = as the model is)
    pub player_rim_finish: [[f32; 4]; 3],
    /// the player's cabin colours A, B, C (w 1 = chosen)
    pub player_interior: [[f32; 4]; 5],
    /// the brakes fitted in the garage (BRAKES: a model of `resources/brakes`), else the car's own
    pub player_brake: Option<Arc<BrakeModel>>,
    /// the tires fitted in the garage (TIRES: a set of `resources/tires/textures`; empty =
    /// the plain set), the set the next wheels are built with, and the rim and size the
    /// player's wheels stand on
    pub player_tire: String,
    tire_now: String,
    player_rim: Option<(std::path::PathBuf, f32)>,
    player_brake_name: String,
    /// the bumper camera sits inside the bodywork: the player's car is not drawn
    pub hide_player: bool,
    /// the driver's camera (the original's POV view) is on: the player's windows are drawn with the
    /// in-car glass program (G+123 of the car's draw object; city.wgsl mode 12, `obj.params.w` bit 256)
    pub interior_glass: bool,
    /// the player's car is driven by the test driver (`--auto`): it passes traffic like the racers
    pub auto_player: bool,
    /// the city's parking spots and their quadtree (parked.rs), the active lists (M+1804) and
    /// the random numbers the parked-car spawner draws (the original's shared MWC)
    pub parking: mcla_formats::rage::parking::ParkingTree,
    park_lists: Vec<u32>,
    park_rng: parked::Mwc,
    park_dump_frame: u32,
    /// (diagnostics, MCLA_PARK_DUMP) parked spawn tries turned down, by reason
    park_why: [u32; 5],
    /// the layout items (sub_826E2070: a pool of 512) and each quadtree node's items in the order
    /// they were put in (node +16 / +20)
    pub layout: Vec<Option<parked::LayoutItem>>,
    pub node_items: Vec<Vec<u32>>,
    pub item_free: std::collections::VecDeque<u32>,
    /// the items whose person stands (peds.rs keeps it), and the items the spawner drew for a
    /// person (sub_826DFB08) or for a car following its race path (sub_826DF430): (item, its
    /// colour of the type's twenty)
    pub item_peds: Vec<u32>,
    pub item_peds_due: Vec<u32>,
    pub item_followers_due: Vec<(u32, usize)>,
    /// the items whose follower's request is still pending (item +80 >= 0; racescript.rs)
    pub item_pending: Vec<u32>,
    /// the people's list 2 as the unspawner sees it (main.rs fills it from peds.rs): each
    /// person's item, place and whether it is older than its least age; the item whose person
    /// it removes
    pub ped_list: Vec<(Option<u32>, Vec3, bool)>,
    pub ped_remove: Option<u32>,
    /// parked cars on (> 0) or off (0); the count itself is not used: the spawner fills a
    /// quarter of the spots of the nodes round the focus (parked.rs)
    pub parked_target: usize,
    /// AmbientMgr_SetParkedCarsEnable(0) in force (main.rs: CarSelection, the new career's opening)
    pub parked_held: bool,
    /// the ambient manager's cutscene mode (sub_826D4CA0: AmbientMgr_SetCutscene's flag, the race
    /// timer >= 0, no replay recording or playback): the cutscene camera's eye and forward - the
    /// focus and view of the spawners (sub_826F14E0), set by main.rs while a scene plays
    pub amb_cutscene: Option<(Vec3, Vec3)>,
    wheel_cache: hashbrown::HashMap<(std::path::PathBuf, String, u32), Arc<WheelModel>>,
    brake_cache: hashbrown::HashMap<(std::path::PathBuf, u32, i32), Arc<BrakeModel>>,
    /// police light bars are flashing
    pub sirens: bool,
    /// running time for light flashing (s)
    pub clock: f32,
    /// traffic signals (shared with the props) and the signal clock
    pub signals: crate::signals::Signals,
    /// payback target: takes heavy damage when the player rams it
    pub fragile: Option<usize>,
    /// Agro: the player shrugs off collisions with other cars
    pub agro: bool,
    /// step 19e: the power-up sources' glows (x, y, z, size, r, g, b, strength)
    pub pu_glows: Vec<[f32; 8]>,
    /// slipstream strength 0..1 on the player
    pub draft: f32,
    sst_log_t: f32,
    /// the physics level every car's step meets (physics/loose.rs): the props and pieces that have come
    /// loose, the other cars as bodies; handed to each car for its step (`level_swap`)
    pub level: crate::physics::loose::LooseState,
    next_uid: u32,
    /// the cars' damage maps (crate::damagemap): the game's pool of 8 render targets, each a car's
    /// (its uid) for as long as the car is there
    dmg_maps: Vec<DmgMap>,
    /// traffic types' DmgTex (the type's `<type>_maxdamage`, sub_82791A50: type +224, else its model's
    /// +24), by model name
    amb_dmg_tex: std::collections::HashMap<String, (wgpu::Texture, wgpu::TextureView)>,
    /// district traffic tables (`tune/ambients`) and the hour they are read at
    pub ambient: crate::ambient::Distribution,
    pub hour: f32,
    /// the paint type of the player's car (garage: 1 Matte .. 5 Sparkle)
    pub player_paint_type: u8,
    /// its second colour and, in w, the fade type (`garage.ChangePaintFadeType`)
    pub player_paint_b: [f32; 4],
    /// the player's paint areas after the body (`ObjectUniform::area_a`, `area_b`)
    pub player_area_a: [[f32; 4]; 5],
    pub player_area_b: [[f32; 4]; 5],
    /// its window tint (colour, level 1..10 or 0)
    pub player_tint: [f32; 4],
    /// diagnostics: car-frames braking for / standing at a red light
    pub held_at_red: u64,
    pub stopped_at_red: u64,
    trace_car: Option<(usize, u32)>,
    /// plate background and font (RGBA), and the finished plates by text
    plate_art: Option<(u32, u32, Vec<u8>, Vec<u8>)>,
    plates: hashbrown::HashMap<String, Arc<wgpu::BindGroup>>,
    plate_serial: u32,
    /// the background of the player's plate (a name of `resources/license_plates/backgrounds`:
    /// the vehicle record's LicenseBG - `mc4`; none = the California plate), and those loaded
    pub player_plate_bg: Option<String>,
    plate_bgs: hashbrown::HashMap<String, Option<(u32, u32, Vec<u8>)>>,
    /// liveries of the named opponents: texture file by set-up name, loaded ones
    vinyl_index: Option<hashbrown::HashMap<String, std::path::PathBuf>>,
    vinyl_painted: hashbrown::HashSet<String>,
    vinyls: hashbrown::HashMap<String, Option<Arc<wgpu::BindGroup>>>,
}

/// Trailers are towed, never driven.
fn is_trailer(n: &str) -> bool {
    n.contains("trailer") || n.contains("flatbed") || n.contains("flatloaded") || n.contains("tanker")
}

pub const PALETTE: [[f32; 4]; 10] = [
    [0.75, 0.05, 0.05, 1.0],
    [0.05, 0.05, 0.08, 1.0],
    [0.85, 0.85, 0.88, 1.0],
    [0.55, 0.57, 0.60, 1.0],
    [0.05, 0.15, 0.55, 1.0],
    [0.85, 0.65, 0.05, 1.0],
    [0.10, 0.40, 0.15, 1.0],
    [0.45, 0.10, 0.50, 1.0],
    [0.90, 0.35, 0.05, 1.0],
    [0.30, 0.30, 0.32, 1.0],
];

/// Everyday paint colours for ambient traffic.
pub const TRAFFIC_PALETTE: [[f32; 4]; 10] = [
    [0.62, 0.63, 0.65, 0.3],
    [0.05, 0.05, 0.06, 0.2],
    [0.80, 0.80, 0.78, 0.1],
    [0.30, 0.31, 0.33, 0.3],
    [0.35, 0.05, 0.05, 0.2],
    [0.06, 0.10, 0.25, 0.3],
    [0.45, 0.40, 0.30, 0.3],
    [0.10, 0.20, 0.12, 0.2],
    [0.55, 0.55, 0.50, 0.3],
    [0.20, 0.22, 0.28, 0.3],
];

/// A traffic or parked car's paint as VSAmbientCarPaint hands it to PSAmbientCarPaint: sat(4 x C^2.2),
/// C = the type's vColors[idx] x fScale (`traffic::AmbType::color`) - sub_827914B8 sets C^2.2 in both
/// paint rows (c124, c125: recorded (0.21764, 0.13321, 0) for a taxi, work/trafpaint/tp_caps.py), the
/// vertex program lerps them and takes sat(4 x) of it (work/trafpaint/tp_px.py: o6).
pub fn ambient_paint(c: [f32; 4]) -> [f32; 4] {
    let f = |x: f32| (4.0 * x.max(0.0).powf(2.2)).min(1.0);
    [f(c[0]), f(c[1]), f(c[2]), c[3]]
}

fn car_mode(shader: &str) -> MatMode {
    let s = shader.to_ascii_lowercase();
    if s.contains("paint") {
        MatMode::Paint
    } else if s.contains("blackmatte") {
        // (BlackMatte.fx: no reflection at all - the backing of grilles, the inside of pipes)
        MatMode::Rubber
    } else if s.contains("carbonfiber") {
        // (CarbonFiber.fx has no texture: a dark glossy weave made by the shader - the R8's
        // side blades, carbon hoods; drawn as black gloss)
        MatMode::BlackGloss
    } else if s == "grillopaque" && std::env::var("MCLA_CARPARTS_OLD").is_err() {
        // (grillopaque.fxc is the BumpSpec program again, opaque, its soft reflection times the
        // texture - the silver mesh panels, e.g. the R8's number plate recess; marked below)
        MatMode::Object
    } else if s.contains("blackopaque") || s.contains("grill") {
        MatMode::BlackGloss
    } else if s.contains("glass") {
        MatMode::CarGlass
    } else if s.contains("alphachrome") && std::env::var("MCLA_BADGE_OLD").is_ok() {
        MatMode::AlphaTest
    } else if s.contains("chrome") || s.contains("brushedmetal") || (s.contains("metal") && !s.contains("grime")) {
        MatMode::Chrome
    } else if s.contains("light") && !s.contains("lightmap") {
        MatMode::Light
    } else if s == "bumpspecalpha" && std::env::var("MCLA_CARPARTS_OLD").is_err() {
        // (bumpspecalpha.fxc is the BumpSpec program with a cut-out - the mirrors' glass, badges,
        // trim: a car part, not a wall; its vertices are marked below for city.wgsl)
        MatMode::Object
    } else if s.contains("alpha") || s.contains("grill") {
        MatMode::AlphaTest
    } else if s.contains("rubber") || s.contains("underbody") || s.contains("bumpersubframe") {
        MatMode::Rubber
    } else if s.starts_with("interiortrim") {
        MatMode::Interior
    } else {
        MatMode::Object
    }
}

fn mesh_to_batch(gpu: &mut Gpu, mesh: &mcla_formats::rage::geometry::Mesh, sname: &str, mode: MatMode, d: Arc<GpuTexture>, s2: Arc<GpuTexture>, k0: u32, k1: u32) -> GpuBatch {
    mesh_to_batch_xf(gpu, mesh, sname, mode, d, s2, k0, k1, Mat4::IDENTITY)
}

/// The stored tangent word's sign w (raw 1 = +1, raw 3 = -1; the game's programs hand the pixel
/// stage `cross(T, N) x w`) rides in the LENGTH of the vertex normal as on the walls
/// (`world::mark_binormal`): 2 for w = -1, 1.5 for w = 0, unchanged for +1 and for meshes without
/// tangents. city.wgsl `vs_body` takes it off (`bflip`) and normalises.
fn car_binormal_mark(mesh: &mcla_formats::rage::geometry::Mesh, i: usize, nrm: Vec3) -> Vec3 {
    match mesh.tangents.get(i) {
        Some(t) if mesh.tangents.len() == mesh.positions.len() => {
            let k = if t.w >= 1.5 { 2.0 } else if t.w < 0.5 { 1.5 } else { 1.0 };
            nrm * k
        }
        _ => nrm,
    }
}

fn mesh_to_batch_xf(gpu: &mut Gpu, mesh: &mcla_formats::rage::geometry::Mesh, sname: &str, mode: MatMode, d: Arc<GpuTexture>, s2: Arc<GpuTexture>, k0: u32, k1: u32, xf: Mat4) -> GpuBatch {
    let bind = gpu.texture_bind_group(&d, &s2, k0, k1);
    let n = mesh.positions.len();
    let mut verts = Vec::with_capacity(n);
    let identity = xf == Mat4::IDENTITY;
    let own = if mesh.normals.len() < n { face_normals(mesh) } else { Vec::new() };
    let mut mn = Vec3::splat(f32::INFINITY);
    let mut mx = Vec3::splat(f32::NEG_INFINITY);
    for i in 0..n {
        let nrm0 = mesh.normals.get(i).or(own.get(i)).copied().unwrap_or(Vec3::Y);
        let (pos, nrm) = if identity { (mesh.positions[i], nrm0) } else { (xf.transform_point3(mesh.positions[i]), xf.transform_vector3(nrm0).normalize_or(Vec3::Y)) };
        mn = mn.min(pos);
        mx = mx.max(pos);
        let nrm = car_binormal_mark(mesh, i, nrm);
        let uv0 = mesh.uv0.get(i).copied().unwrap_or_default();
        let uv1 = mesh.uv1.get(i).copied().unwrap_or_default();
        let c = mesh.colors.get(i).copied().unwrap_or([255, 255, 255, 255]);
        let c1 = mesh.colors1.get(i).copied().unwrap_or([c[0], 255, 255, 255]); // (no second colour set: its alpha slot carries the first set's alpha, the body's baked occlusion - the lighting reads it there; the first set's own alpha stays the baked-paint flag)
        verts.push(Vertex { pos: pos.to_array(), normal: nrm.to_array(), uv0: uv0.to_array(), uv1: uv1.to_array(), color: [c[1], c[2], c[3], 0], color1: [c1[1], c1[2], c1[3], c1[0]], uv2: [0.0, 0.0] });
    }
    let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
    let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&mesh.indices), usage: wgpu::BufferUsages::INDEX });
    GpuBatch { shader: sname.to_string(), vb, ib, count: mesh.indices.len() as u32, bind, mode, min: mn, max: mx }
}

/// The California plate background of the disc (`resources/license_plates/backgrounds`),
/// uploaded once. The files hold a single texture object behind the root pointer.
fn plate_texture(gpu: &mut Gpu, world: &World) -> (Arc<GpuTexture>, u32) {
    const KEY: u32 = 0xb59c_dcaf; // joaat("plate_ca_00")
    if let Some(t) = gpu.textures.get(&KEY) {
        return (t.clone(), KEY);
    }
    let dir = world.assets.root.join("resources/license_plates/backgrounds/plate_ca_00");
    let file = std::fs::read_dir(&dir).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.extension().map_or(false, |x| x == "r1")));
    if let Some(f) = file {
        if let Ok(res) = crate::assets::load_resource_file(&f) {
            let rd = mcla_formats::rage::Rd::new(&res);
            if let Some(o) = rd.ptr(0) {
                if let Ok(t) = mcla_formats::rage::texture::Texture::parse(&rd, o) {
                    return (gpu.upload_texture(KEY, &t), KEY);
                }
            }
        }
    }
    (gpu.white.clone(), 0)
}

/// (render only) The carbon weave's normal map of the disc: `__cfnormalmap` = `cf_bump2.dds` of
/// `textures/global/cars/globaltex` (256 x 256, x / y tilt in red / green, the strand in alpha),
/// uploaded as stored - no gamma decoding. White when the file is missing (the shader then takes a flat map).
fn carbon_weave(gpu: &mut Gpu, world: &World) -> (Arc<GpuTexture>, u32) {
    const KEY: u32 = 0x9499_fb74 ^ 0x0052_4157; // joaat("cf_bump2.dds"), kept apart from a gamma-decoded upload of the same file
    if let Some(t) = gpu.textures.get(&KEY) {
        return (t.clone(), KEY);
    }
    if let Some(p) = world.globaltex.get(&mcla_formats::hash::joaat("cf_bump2.dds")) {
        if let Some(t) = std::fs::read(p).ok().and_then(|d| mcla_formats::dds::parse_dds("cf_bump2.dds", &d).ok()) {
            return (gpu.upload_texture_raw(KEY, &t), KEY);
        }
    }
    (gpu.white.clone(), 0)
}

/// The windows' two global maps (damage_map_rule.md, "The windows' look"): `__detailmasktest`
/// = window_detail_mask.dds (DetailMaskSampler: its alpha keeps the shards of a broken pane) and
/// `__glassdmg` = glass_test.dds (GlassDmgSampler: the crack's normal in rgb, the crack in alpha) of
/// cars/globaltex.list - as stored, not gamma-decoded (a normal map and masks). None when missing.
fn glass_maps(gpu: &mut Gpu, world: &World) -> Option<((Arc<GpuTexture>, u32), (Arc<GpuTexture>, u32))> {
    let raw = |gpu: &mut Gpu, name: &str| -> Option<(Arc<GpuTexture>, u32)> {
        // (kept apart from a gamma-decoded upload of the same file)
        let key = mcla_formats::hash::joaat(name) ^ 0x0052_4157;
        if let Some(t) = gpu.textures.get(&key) {
            return Some((t.clone(), key));
        }
        let p = world.globaltex.get(&mcla_formats::hash::joaat(name))?;
        let t = std::fs::read(p).ok().and_then(|d| mcla_formats::dds::parse_dds(name, &d).ok())?;
        Some((gpu.upload_texture_raw(key, &t), key))
    };
    Some((raw(gpu, "window_detail_mask.dds")?, raw(gpu, "glass_test.dds")?))
}

/// The in-car glass program's noise (PS_InteriorGlass noiseColoredSampler): `__coloredNoise` of
/// cars/globaltex.list = 2dnoise4.dds, 128 x 128 A8R8G8B8 with its 8 levels, read as stored (the
/// bound texture of a recording is this file's green byte for byte). The windows' third texture.
fn glass_noise(gpu: &mut Gpu, world: &World) -> Option<(Arc<GpuTexture>, u32)> {
    let key = mcla_formats::hash::joaat("2dnoise4.dds") ^ 0x0052_4157;
    if let Some(t) = gpu.textures.get(&key) {
        return Some((t.clone(), key));
    }
    let p = world.globaltex.get(&mcla_formats::hash::joaat("2dnoise4.dds"))?;
    let t = std::fs::read(p).ok().and_then(|d| mcla_formats::dds::parse_dds("2dnoise4.dds", &d).ok())?;
    Some((gpu.upload_texture_raw(key, &t), key))
}

/// Normals for a mesh whose vertex format has none (the matte black parts are stored as
/// positions alone): the area-weighted mean of the faces around each vertex.
fn face_normals(mesh: &mcla_formats::rage::geometry::Mesh) -> Vec<Vec3> {
    let n = mesh.positions.len();
    let mut out = vec![Vec3::ZERO; n];
    for t in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        if a < n && b < n && c < n {
            let f = (mesh.positions[b] - mesh.positions[a]).cross(mesh.positions[c] - mesh.positions[a]);
            out[a] += f;
            out[b] += f;
            out[c] += f;
        }
    }
    out.iter().map(|v| v.normalize_or(Vec3::Y)).collect()
}

/// CPU-side accumulator for meshes that share a material.
struct MergeAcc {
    verts: Vec<Vertex>,
    indices: Vec<u32>,
    d: Arc<GpuTexture>,
    s2: Arc<GpuTexture>,
    sname: String,
    mode: MatMode,
    min: Vec3,
    max: Vec3,
}

impl MergeAcc {
    fn push(&mut self, mesh: &mcla_formats::rage::geometry::Mesh, xf: Mat4, dark_above: Option<f32>, lens: f32, amb: u8) {
        let base = self.verts.len() as u32;
        let identity = xf == Mat4::IDENTITY;
        let n = mesh.positions.len();
        let own = if mesh.normals.len() < n { face_normals(mesh) } else { Vec::new() };
        for i in 0..n {
            let nrm0 = mesh.normals.get(i).or(own.get(i)).copied().unwrap_or(Vec3::Y);
            let nrm0 = if nrm0.is_finite() { nrm0 } else { Vec3::Y };
            let (pos, nrm) = if identity { (mesh.positions[i], nrm0) } else { (xf.transform_point3(mesh.positions[i]), xf.transform_vector3(nrm0).normalize_or(Vec3::Y)) };
            self.min = self.min.min(pos);
            self.max = self.max.max(pos);
            let nrm = car_binormal_mark(mesh, i, nrm);
            let uv0 = mesh.uv0.get(i).copied().unwrap_or_default();
            let uv1 = mesh.uv1.get(i).copied().unwrap_or_default();
            let c = mesh.colors.get(i).copied().unwrap_or([255, 255, 255, 255]);
            let c1 = mesh.colors1.get(i).copied().unwrap_or([c[0], 255, 255, 255]); // (no second colour set: its alpha slot carries the first set's alpha, the body's baked occlusion - the lighting reads it there; the first set's own alpha stays the baked-paint flag)
            // (a window of the full glass program, lens -1: its stored tangent in the car's space in the colour,
            // alpha 255 - VS_Glass's T for the crack map's frame; the window's colour is not read)
            let tw = if lens < -0.5 { mesh.tangents.get(i).map(|t| xf.transform_vector3(t.truncate()).normalize_or_zero()).filter(|t| *t != Vec3::ZERO) } else { None };
            let c = match tw {
                Some(t) => [0, ((t.x * 0.5 + 0.5) * 255.0).round() as u8, ((t.y * 0.5 + 0.5) * 255.0).round() as u8, ((t.z * 0.5 + 0.5) * 255.0).round() as u8],
                None => c,
            };
            let ca = if tw.is_some() { 255 } else { 0 };
            // (a traffic model's vertex: its stored tangent in the car's frame in the second colour's rgb, t x 0.5
            // + 0.5 - the ambient programs bend the normal along it where the car is dented (city.wgsl vs_body);
            // its meshes have no second colour set, so those three bytes are otherwise unused; 0 0 0 - no unit
            // tangent comes out so - marks the tyre, whose program, VSAmbientTire, has no dent: amb 2)
            let at = if amb == 2 && mesh.colors1.is_empty() { Some(Vec3::splat(-1.0)) } else if amb == 1 && mesh.colors1.is_empty() { mesh.tangents.get(i).filter(|_| mesh.tangents.len() == n).map(|t| xf.transform_vector3(t.truncate()).normalize_or_zero()) } else { None };
            let c1 = match at {
                Some(t) => [c1[0], ((t.x * 0.5 + 0.5) * 255.0).round() as u8, ((t.y * 0.5 + 0.5) * 255.0).round() as u8, ((t.z * 0.5 + 0.5) * 255.0).round() as u8],
                None => c1,
            };
            self.verts.push(Vertex { pos: pos.to_array(), normal: nrm.to_array(), uv0: uv0.to_array(), uv1: uv1.to_array(), color: [c[1], c[2], c[3], ca], color1: [c1[1], c1[2], c1[3], c1[0]], uv2: [if dark_above.map_or(false, |y| pos.y >= y) { 1.0 } else { 0.0 }, lens] });
        }
        // (a mirrored instance - the second exhaust tip - keeps its faces outward: its winding
        // reversed, as the original flips the cull mode for it, sub_82359C10)
        let mirrored = xf.determinant() < 0.0;
        for t in mesh.indices.chunks_exact(3) {
            if t.iter().all(|&i| (i as usize) < n) {
                if mirrored {
                    self.indices.extend([t[0] + base, t[2] + base, t[1] + base]);
                } else {
                    self.indices.extend(t.iter().map(|&i| i + base));
                }
            }
        }
    }
}

/// Physics parameters derived from the art bounds.
pub fn car_params(car: &CarAsset) -> CarParams {
    let mut p = CarParams::default();
    let size = car.max - car.min;
    let center = (car.max + car.min) * 0.5;
    if let Some(h) = car.hubs {
        // hub height above the ground plane (y = 0) is the wheel radius
        p.wheel_radius = h.iter().map(|v| v.y).sum::<f32>() / 4.0;
        p.wheel_radius = p.wheel_radius.clamp(0.25, 0.5);
        p.wheel_pos = [h[0], h[1], h[2], h[3]].map(|v| Vec3::new(v.x, v.y + p.susp_rest, v.z));
        p.hub_bones = Some(h);
    } else {
        p.wheel_radius = (size.y * 0.16).clamp(0.30, 0.40);
        let hub_y = car.min.y + p.wheel_radius;
        let half_w = size.x * 0.5 - 0.10;
        let front_z = center.z - size.z * 0.31;
        let rear_z = center.z + size.z * 0.30;
        p.wheel_pos = [
            Vec3::new(-half_w, hub_y + p.susp_rest, front_z),
            Vec3::new(half_w, hub_y + p.susp_rest, front_z),
            Vec3::new(-half_w, hub_y + p.susp_rest, rear_z),
            Vec3::new(half_w, hub_y + p.susp_rest, rear_z),
        ];
    }
    p.body_half = Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5);
    p.mass = (900.0 + size.x * size.y * size.z * 90.0).clamp(1000.0, 2600.0);
    if let Some(t) = car.tune.as_ref() {
        p.mass = t.mass.clamp(600.0, 3500.0);
        if t.dims.iter().all(|d| *d > 0.5) && !car.bike {
            p.body_half = Vec3::new(t.dims[0] * 0.5, t.dims[1] * 0.5, t.dims[2] * 0.5);
        }
        if car.bike {
            // bike + rider
            p.body_half = Vec3::new(0.38, 0.6, (size.z * 0.5).max(0.9));
        }
        p.drive_front = match t.drivetrain {
            1 => 1.0,
            2 => 0.5,
            _ => 0.0,
        };
        p.top_speed = t.engine.top_speed.clamp(30.0, 120.0);
        let e = &t.engine;
        p.accel = e.accel.clone();
        if !e.gears.is_empty() {
            p.gear_count = e.gears.len().min(8);
            for (i, g) in e.gears.iter().take(8).enumerate() {
                p.gears[i] = *g;
            }
        }
        let peak = e.torque.iter().cloned().fold(0.0f32, f32::max);
        if e.torque.len() == 40 && peak > 0.0 {
            for i in 0..40 {
                p.torque[i] = (0.45 + 0.55 * e.torque[i] / peak).clamp(0.3, 1.0);
            }
        }
        if t.redline > 2000.0 {
            p.redline = t.redline.clamp(4000.0, 14000.0);
        } else if e.rpm_scale > 500.0 {
            p.redline = (e.rpm_scale * 2.5).clamp(5000.0, 10000.0);
        }
        // tyres, steering lock and wheel size straight from the car's axle data
        if let (Some(f), Some(h)) = (car.fit.as_ref(), car.hubs) {
            // (the original's wheels: at the hub bones' height and length, sideways where the
            // fenders put tyres of this width; drawn 6.35 mm larger than the tyre's physics)
            p.wheel_radius = f.radius[0] + 0.00635;
            for (i, w) in p.wheel_pos.iter_mut().enumerate() {
                let x = f.half_track[i / 2];
                *w = Vec3::new(if x > 0.0 { h[i].x.signum() * x } else { h[i].x }, h[i].y + p.susp_rest, h[i].z);
            }
        } else if t.front.radius > 0.2 && t.front.radius < 0.6 {
            // (the hub bones sit a few centimetres above this on some cars: droop)
            let hub = p.wheel_radius;
            p.wheel_radius = t.front.radius;
            for w in p.wheel_pos.iter_mut() {
                w.y += p.wheel_radius - hub;
            }
        }
        if t.front.grip > 0.3 && t.rear.grip > 0.3 {
            p.grip_front = t.front.grip;
            p.grip_rear = t.rear.grip;
        }
        if t.front.steer > 0.2 && t.front.steer < 1.2 {
            p.max_steer = t.front.steer;
        }
        if t.front.brake > 0.05 && t.rear.brake > 0.05 {
            p.brake_bias = t.front.brake / (t.front.brake + t.rear.brake);
        }
        // grip hint: the game's own lateral grip triple (Golf ~1.8, muscle cars ~1.2)
        let g = (t.grip_a[0] + t.grip_a[2]) * 0.5;
        if g > 0.3 {
            p.grip *= (0.88 + 0.12 * (g / 1.9)).clamp(0.85, 1.05);
        }
    }
    // The wheel models are as wide as half their diameter; the cars wear tyres of about
    // two thirds of that, and never wider than the body leaves room for.
    p.wheel_width = p.wheel_radius * 0.72;
    if let Some(h) = car.hubs {
        let room = 2.0 * (size.x * 0.5 - h[1].x.abs());
        if room > 0.12 {
            p.wheel_width = p.wheel_width.min(room);
        }
    }
    if car.bike {
        p.wheel_width = 0.17;
    }
    p.engine_force = p.mass * 11.0;
    // maximum braking deceleration of the car (the tyres still limit it)
    let decel = car.tune.as_ref().map(|t| t.engine.brake_decel).filter(|d| *d > 4.0 && *d < 30.0).unwrap_or(11.0);
    p.brake_force = p.mass * decel * 1.25;
    if car.tune.is_none() {
        p.grip_front = p.grip;
        p.grip_rear = p.grip;
    }
    p.spring = p.mass * 32.0;
    p.damper = p.mass * 3.2;
    p.finalize();
    p
}

/// Parameters of the fully upgraded car: the "mods" drivetrain table and mass.
pub fn car_params_mods(car: &CarAsset, base: &CarParams) -> Option<CarParams> {
    let t = car.tune.as_ref()?;
    let e = &t.engine_mods;
    if e.top_speed <= base.top_speed * 0.9 || e.accel.len() < 16 {
        return None;
    }
    let mut p = base.clone();
    p.top_speed = e.top_speed.clamp(30.0, 130.0);
    p.accel = e.accel.clone();
    if t.mass_mods > 300.0 {
        p.mass = t.mass_mods.clamp(600.0, 3500.0);
    }
    if !e.gears.is_empty() {
        p.gear_count = e.gears.len().min(8);
        for (i, g) in e.gears.iter().take(8).enumerate() {
            p.gears[i] = *g;
        }
    }
    if e.brake_decel > 4.0 && e.brake_decel < 30.0 {
        p.brake_force = p.mass * e.brake_decel * 1.25;
    }
    // upgraded tyres: the catalogue handling figure grows by about a quarter
    p.grip_front *= 1.12;
    p.grip_rear *= 1.12;
    p.finalize();
    Some(p)
}

impl CarInstance {
    /// The parameters this car drives with (stock, or blended towards the upgrades).
    pub fn params(&self) -> &CarParams {
        self.tuned.as_deref().unwrap_or(&self.model.params)
    }

    /// Give the car the wheels of a set-up (`mcCarConfig`: a rival's `.xcc`).
    pub fn set_setup(&mut self, cfg: &[u8]) {
        let Some(s) = self.model.simtunes.as_ref() else { return };
        let Some(fit) = (**s).clone().fit(cfg) else { return };
        if self.setup != Some(fit) {
            self.setup = Some(fit);
            let perf = self.perf;
            self.set_perf(perf);
        }
    }

    /// Fit performance upgrades: 0 = stock, 1 = everything.
    pub fn set_perf(&mut self, level: f32) {
        self.perf = level.clamp(0.0, 1.0);
        self.tuned = None;
        if let Some(ai) = self.ai.as_mut() {
            ai.tune = None;
        }
        if self.perf <= 0.0 && self.setup.is_none() {
            return;
        }
        let b = &self.model.params;
        let f = self.perf;
        let mut p = b.clone();
        if let Some(s) = self.model.simtunes.as_ref() {
            // the game blends the base and the upgraded tuning field by field
            let mut tune = mcla_formats::rage::simtune::SimTune::lerp(&s.base, &s.mods, f);
            // (and then writes the set-up's wheels over the blend)
            if let Some(fit) = self.setup {
                for (axle, wt) in [&mut tune.wheel_front, &mut tune.wheel_back].into_iter().enumerate() {
                    wt.radius = fit.radius[axle];
                    wt.width = fit.width[axle];
                    wt.suspension_offset = fit.offset[axle];
                }
                for (i, w) in p.wheel_pos.iter_mut().enumerate() {
                    if fit.half_track[i / 2] > 0.0 {
                        w.x = w.x.signum() * fit.half_track[i / 2];
                    }
                }
            }
            let mut vt = crate::physics::vehsim::VehTune::new(tune);
            vt.bike = self.model.bike;
            if let Some(h) = self.model.rest_height {
                vt.seat(h);
            }
            p.veh = Some(Arc::new(vt));
        }
        let Some(m) = self.model.params_mods.as_ref() else {
            if p.veh.is_some() {
                self.tuned = Some(Box::new(p));
            }
            return;
        };
        p.top_speed = b.top_speed + (m.top_speed - b.top_speed) * f;
        p.mass = b.mass + (m.mass - b.mass) * f;
        p.brake_force = b.brake_force + (m.brake_force - b.brake_force) * f;
        p.grip_front = b.grip_front + (m.grip_front - b.grip_front) * f;
        p.grip_rear = b.grip_rear + (m.grip_rear - b.grip_rear) * f;
        // both tables span 0..their own top speed: resample at equal real speeds
        if !b.accel.is_empty() && !m.accel.is_empty() {
            let n = b.accel.len().max(m.accel.len());
            p.accel = (0..n)
                .map(|i| {
                    let v = (i as f32 + 0.5) / n as f32 * p.top_speed;
                    b.accel_at(v) + (m.accel_at(v) - b.accel_at(v)) * f
                })
                .collect();
        }
        p.spring = p.mass * 32.0;
        p.damper = p.mass * 3.2;
        p.finalize();
        self.tuned = Some(Box::new(p));
    }
}

impl Vehicles {
    pub fn new() -> Vehicles {
        Vehicles { traffic_target: 0, rng: 12345, ..Default::default() }
    }

    /// The lamp table a car is drawn and lit with (render/car_lamps.rs): the player's record's,
    /// else its set-up's, its traffic type's or its model's own; None = none on disc.
    pub fn lamp_rows(&self, c: &CarInstance) -> Option<Arc<crate::render::car_lamps::LampRows>> {
        match (&self.player_lamps, c.is_player) {
            (Some(t), true) => Some(t.clone()),
            _ => crate::render::car_lamps::for_car(&c.model.name, &c.vinyl_name),
        }
    }

    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x % 100000) as f32 / 100000.0
    }

    /// Load a car directory into a shared GPU model.
    pub fn load_model(&mut self, gpu: &mut Gpu, world: &mut World, dir: &Path) -> Result<Arc<CarModel>> {
        self.load_model_cfg(gpu, world, dir, &CarConfig::default())
    }

    pub fn load_model_cfg(&mut self, gpu: &mut Gpu, world: &mut World, dir: &Path, cfg: &CarConfig) -> Result<Arc<CarModel>> {
        let car = vehicle_model::load_car(&world.assets, dir)?;
        let car_is_ambient = dir.file_name().map_or(false, |n| n.to_string_lossy().to_ascii_lowercase().starts_with("va_"));
        let mut batches = Vec::new();
        let mut merged: hashbrown::HashMap<(u32, u32, u32), MergeAcc> = hashbrown::HashMap::new();
        // (which colour group each zone of the cabin trim takes: the vehicle's own table, its .r83)
        let zone_table = crate::mc4::zone_groups_dir(dir);
        // (lamp faces by type and side: the sum of their vertices and how many)
        let mut lamp_acc: hashbrown::HashMap<(i32, bool), (Vec3, u32)> = hashbrown::HashMap::new();
        let variants: Vec<(Slot, Vec<String>)> = vehicle_model::CUSTOM_SLOTS.iter().map(|&s| (s, car.variants(s))).collect();
        let variant_labels: Vec<(Slot, Vec<String>)> = vehicle_model::CUSTOM_SLOTS.iter().map(|&s| (s, car.variant_labels(s))).collect();
        let mut extra_variants: std::collections::HashSet<(String, usize)> = car.parts.iter().filter(|p| p.placed)
            .filter_map(|p| vehicle_model::extra_part(&p.primary))
            .map(|(sec, index)| (sec.to_string(), index)).collect();
        // (a convertible whose roof moves: the options' CONVERTIBLE TOP row is its)
        if car.has_roof_animation() {
            extra_variants.insert((vehicle_model::ROOF_DOWN.to_string(), 0));
        }
        let only_part = std::env::var("MCLA_ONLY_PART").ok();
        let hide_part = std::env::var("MCLA_HIDE_PART").ok();
        let lamp_dump = std::env::var("MCLA_LAMP_DUMP").is_ok();
        let mut glow_bones: Vec<(u8, Vec3, Vec3)> = Vec::new();
        // (the moving damage parts, DAMAGE_PARTS_RULE.md 1.4 / 2.3: the slot pass moves the parts of the slots
        // hood / hood_wide (2, 40), bumper_r (3), intercooler and bumper_f (5, 6); the trunk_0 bone turns with
        // whatever hangs on it - the base fragment's meshes bound to it or to its children, the parts placed there)
        let sk = car.skeleton.as_ref();
        let bone_world = |tag: u16| sk.and_then(|s| s.bone_by_tag(tag).map(|i| s.bones[i].world));
        let trunk_i = sk.and_then(|s| s.bones.iter().position(|b| b.name.eq_ignore_ascii_case("trunk_0")).or_else(|| s.bone_by_tag(33531)));
        let trunk_set: Vec<usize> = match (sk, trunk_i) {
            (Some(s), Some(t)) => (0..s.bones.len()).filter(|&i| {
                let mut j = i;
                for _ in 0..64 {
                    if j == t {
                        return true;
                    }
                    if j >= s.bones.len() || s.bones[j].parent == j {
                        return false;
                    }
                    j = s.bones[j].parent;
                }
                false
            }).collect(),
            _ => Vec::new(),
        };
        let trunk_tags: Vec<u16> = trunk_set.iter().filter_map(|&i| sk.map(|s| s.bones[i].tag)).filter(|t| *t != 0).collect();
        let trunk_worlds: Vec<Mat4> = trunk_set.iter().filter_map(|&i| sk.map(|s| s.bones[i].world)).collect();
        let hood_slot = bone_world(vehicle_model::slot_tag(Slot::Hood).unwrap_or(0));
        let trunk_bone = trunk_i.and_then(|i| sk.map(|s| s.bones[i].world));
        let part_rig = crate::damageparts::PartRig {
            movement: car.simtune.as_ref().and_then(|s| s.part_movement).unwrap_or(crate::damageparts::DEFAULT_MOVEMENT),
            bumper_rest: [bone_world(13699), bone_world(13687)],
        };
        let part_group = |part: &vehicle_model::CarPart, part_xf: &Mat4, mi: usize| -> u32 {
            let p = part.primary.to_ascii_lowercase();
            if part.is_base {
                return if part.mesh_tag.get(mi).map_or(false, |t| trunk_tags.contains(t)) { 4 } else { 0 };
            }
            if p.starts_with("hood") && !p.contains("scoop") && hood_slot.is_some() {
                3
            } else if p.starts_with("bumper_r") && part_rig.bumper_rest[0].is_some() {
                5
            } else if (p.starts_with("bumper_f") || p.starts_with("intercooler")) && part_rig.bumper_rest[1].is_some() {
                6
            } else if *part_xf != Mat4::IDENTITY && trunk_worlds.iter().any(|w| w.w_axis.distance(part_xf.w_axis) < 1e-3) {
                4
            } else {
                0
            }
        };
        if std::env::var("MCLA_PARTS_LOG").is_ok() {
            log::info!("damage parts {}: rig {:?} hood slot {:?} trunk bone {:?} (set {:?})", car.name, part_rig, hood_slot.map(|m| m.w_axis), trunk_bone.map(|m| m.w_axis), trunk_tags);
        }
        let mut own_hubs: [Option<Vec3>; 6] = [None; 6];
        // (a traffic model's LOD 1 and LOD 2: its other two base fragments, built as LOD 0's body is)
        let mut todo: Vec<(&vehicle_model::CarPart, Mat4, u32)> = car.placed_parts_for(cfg).into_iter().map(|(p, x)| (p, x, 0)).collect();
        if car_is_ambient {
            for (lod, h) in [(1u32, "7b0cd281"), (2, "0de3ddb3")] {
                if let Some(p) = car.lod_parts.iter().find(|p| p.primary.contains(h)) {
                    todo.push((p, p.transform, lod));
                }
            }
        }
        for (part, part_xf, lod) in todo {
            if let Some(f) = only_part.as_deref() {
                if !f.split(',').any(|n| part.primary == n) {
                    continue;
                }
            }
            if let Some(f) = hide_part.as_deref() {
                if f.split(',').any(|n| part.primary == n) {
                    continue;
                }
            }
            log::debug!("draw part {} ({} meshes, from_parent {})", part.primary, part.meshes.len(), part.from_parent);
            // (the glow places: bones of the part's own skeleton, children of the `glow<kind>0` bones)
            if let Some(sk) = (lod == 0).then(|| crate::assets::load_resource_file(&part.file).ok()).flatten().and_then(|res| mcla_formats::rage::skeleton::Skeleton::parse(&res).ok().flatten()) {
                for (bi, b) in sk.bones.iter().enumerate() {
                    let parent = sk.bones.get(b.parent).map_or(String::new(), |q| q.name.to_ascii_lowercase());
                    let w = (part_xf * b.world).w_axis.truncate();
                    if lamp_dump {
                        eprintln!("lampbone {} {} {} {} parent {} {:.4} {:.4} {:.4} z {:.3} {:.3} {:.3}", car.name, part.primary, bi, if b.name.is_empty() { "-" } else { b.name.as_str() }, if parent.is_empty() { "-" } else { parent.as_str() }, w.x, w.y, w.z, b.world.z_axis.x, b.world.z_axis.y, b.world.z_axis.z);
                    }
                    if let Some(kind) = glow_bone_kind(&b.name.to_ascii_lowercase()) {
                        glow_bones.push((kind, w, -(part_xf * b.world).z_axis.truncate().normalize_or_zero()));
                    }
                }
            }
            for (mi, (mesh, si)) in part.meshes.iter().enumerate() {
                let bone_m = part.mesh_bone.get(mi).copied().unwrap_or(Mat4::IDENTITY);
                let sh = if part.from_parent && !car.shaders_parent.is_empty() { car.shaders_parent.get(*si as usize) } else { car.shaders.get(*si as usize) };
                let sname = sh.map(|s| s.name.as_str()).unwrap_or("");
                // (CarbonFiberHood: the hood's painted meshes - part slots hood and hood_wide - are
                // drawn by the carbon fibre program in place of the paint, sub_8235D500 at 0x8235DEE0.
                // (approx.) by the port's branch for the CarbonFiber material; the hood's own
                // program, PS_HoodCarbonFiber of cars/0x0d96b4f8, has other constants - lighting work)
                let sname = if car_mode(sname) == MatMode::Paint && cfg.extra.get("carbon_hood").copied().unwrap_or(0) != 0 && vehicle_model::classify_slot(&part.primary, &part.names) == Slot::Hood { "CarbonFiber" } else { sname };
                let mode = car_mode(sname);
                // (lighting diagnostics: MCLA_MESH_DUMP lists a car's meshes - part, index, shader, size, box in the
                // part's space; MCLA_HIDE_MESH=part:index,... leaves meshes out)
                if std::env::var("MCLA_MESH_DUMP").is_ok() {
                    log::info!("meshdump-xf {} {} part placement {:?}", car.name, part.primary, part_xf);
                    let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                    for v in &mesh.positions {
                        let w = bone_m.transform_point3(*v);
                        mn = mn.min(w);
                        mx = mx.max(w);
                    }
                    log::info!("meshdump {} {}:{} shader {} #{} verts {} box {:.2?} .. {:.2?} tex {:?}", car.name, part.primary, mi, sname, si, mesh.positions.len(), mn.to_array(), mx.to_array(), sh.map(|s| s.textures().map(|(_, n)| n.to_string()).collect::<Vec<_>>()));
                }
                if let Ok(h) = std::env::var("MCLA_HIDE_MESH") {
                    if h.split(',').any(|e| e == format!("{}:{}", part.primary, mi)) {
                        continue;
                    }
                }
                if std::env::var("MCLA_TRIM_DUMP").is_ok() && (mode == MatMode::Interior || sname.to_ascii_lowercase().contains("carbon") || sname.to_ascii_lowercase().contains("caliper") || sname.to_ascii_lowercase().contains("brake")) {
                    let mut u1: std::collections::BTreeMap<i32, u32> = Default::default();
                    for u in &mesh.uv1 {
                        *u1.entry((u.x * 10.0).round() as i32).or_insert(0) += 1;
                    }
                    log::info!("trim: part {} shader {} verts {} uv1x10 {:?} tex {:?} params {:?}", part.primary, sname, mesh.positions.len(), u1, sh.map(|s| s.textures().map(|(_, n)| n.to_string()).collect::<Vec<_>>()), sh.map(|s| s.params.iter().filter(|(_, p)| format!("{:?}", p).len() < 90).map(|(h, p)| format!("{:08x}={:?}", h, p)).collect::<Vec<_>>()));
                }
                // (render only - every traffic car's glass, AmbientCarGlass, is drawn by its own program,
                // PSAmbientGlassFront: marked per vertex for city.wgsl, which tells windows from lamp
                // lenses by the zone in uv1.x; until 2026-10-20 only the glass above 0.82 m was marked)
                let dark_above = if mode == MatMode::CarGlass && car_is_ambient { Some(f32::NEG_INFINITY) } else { None };
                // pick the colour map by its sampler name; never a normal map
                const NORMAL_SAMPLERS: [u32; 2] = [0x8ac11cb0, 0x28038e05];
                // (the traffic lamps' program, PSAmbientCarLight, has ONE texture and it is a colour map - the car's trim sheet - bound to the sampler called NormalSampler)
                let amb_lamp_map = sname.eq_ignore_ascii_case("AmbientCarLight") && std::env::var("MCLA_CARPARTS_OLD").is_err();
                // (and the badges' program, PS_BumpedChrome of cars/0x1d68d869 (BumpedAlphaChrome): its one
                // texture, on NormalSampler, is a bump map whose alpha is the badge's outline - kept for its alpha)
                let badge = sname.eq_ignore_ascii_case("BumpedAlphaChrome") && std::env::var("MCLA_BADGE_OLD").is_err();
                let amb_lamp_map = amb_lamp_map || badge;
                let all_tex: Vec<(u32, String)> = sh.map(|s| s.textures().filter(|(h, _)| !NORMAL_SAMPLERS.contains(h) || amb_lamp_map).map(|(h, n)| (h, n.to_string())).collect()).unwrap_or_default();
                let is_normal = |n: &str| {
                    let l = n.to_ascii_lowercase();
                    l.ends_with("_n.dds") || l.ends_with("_nm.dds") || l.contains("normal") || l.ends_with("_bs.dds")
                };
                let dn = all_tex
                    .iter()
                    .find(|(h, n)| *h == crate::world::ph::DIFFUSE && !is_normal(n))
                    .or_else(|| all_tex.iter().find(|(_, n)| !is_normal(n)))
                    .map(|x| x.1.clone());
                let sn = all_tex.iter().find(|(_, n)| Some(n) != dn.as_ref() && !is_normal(n)).map(|x| x.1.clone()).or_else(|| dn.clone());
                if log::log_enabled!(log::Level::Debug) && dn.is_none() && !all_tex.is_empty() {
                    log::debug!("shader {} has only normal maps: {:?}", sname, all_tex);
                }
                let resolve = |gpu: &mut Gpu, world: &mut World, name: Option<&str>| -> (Arc<GpuTexture>, u32) {
                    let Some(name) = name else { return (gpu.white.clone(), 0) };
                    let h = mcla_formats::hash::joaat(name);
                    if let Some(t) = gpu.textures.get(&h) {
                        return (t.clone(), h);
                    }
                    if let Some(t) = car.textures.get(&h) {
                        log::debug!("car texture {} from car dict: {}x{} {:?} name {:?} levels {}", name, t.width, t.height, t.format, t.name, t.levels.len());
                        return (gpu.upload_texture(h, t), h);
                    }
                    if let Some(t) = world.texture_any(gpu, h) {
                        return (t, h);
                    }
                    log::debug!("car texture miss: {} ({:#010x})", name, h);
                    (gpu.white.clone(), 0)
                };
                log::debug!("mat {} mesh {} shader#{} {} mode {:?} tex {:?}", part.primary, mi, si, sname, mode, dn);
                if mode == MatMode::Paint {
                    if let Ok(f) = std::env::var("MCLA_VINYL_DUMP") {
                        // the painted body in car space with its unwrap: "v pos normal uv", "t indices"
                        use std::io::Write;
                        let m = part_xf * bone_m;
                        if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(&f) {
                            let _ = writeln!(out, "m {} {} {}", car.name, part.primary, mi);
                            for k in 0..mesh.positions.len() {
                                let p = m.transform_point3(mesh.positions[k]);
                                let n = m.transform_vector3(mesh.normals.get(k).copied().unwrap_or(Vec3::Y));
                                let u = mesh.uv0.get(k).copied().unwrap_or_default();
                                let u1 = mesh.uv1.get(k).copied().unwrap_or_default();
                                let u2 = mesh.uv2.get(k).copied().unwrap_or_default();
                                let c = mesh.colors.get(k).copied().unwrap_or_default();
                                let _ = writeln!(out, "v {:.4} {:.4} {:.4} {:.3} {:.3} {:.3} {:.5} {:.5} {:.5} {:.5} {:.5} {:.5} {} {} {} {}", p.x, p.y, p.z, n.x, n.y, n.z, u.x, u.y, u1.x, u1.y, u2.x, u2.y, c[0], c[1], c[2], c[3]);
                            }
                            for t in mesh.indices.chunks_exact(3) {
                                let _ = writeln!(out, "t {} {} {}", t[0], t[1], t[2]);
                            }
                        }
                    }
                }
                if mode == MatMode::Paint && std::env::var("MCLA_UV_LOG").is_ok() {
                    let (mut u0, mut u1) = (glam::Vec2::splat(9e9), glam::Vec2::splat(-9e9));
                    for u in &mesh.uv0 {
                        u0 = u0.min(*u);
                        u1 = u1.max(*u);
                    }
                    log::info!("paint uv {} {} mesh {} verts {} uv0 {} range {:?}..{:?}", car.name, part.primary, mi, mesh.positions.len(), mesh.uv0.len(), u0, u1);
                }
                if log::log_enabled!(log::Level::Debug) && std::env::var("MCLA_SHADER_DUMP").is_ok() {
                    if let Some(sh) = sh {
                        log::debug!("shaderdump {} #{} params {:?}", sname, si, sh.params.iter().map(|(h, p)| format!("{:08x}={:?}", h, p)).collect::<Vec<_>>());
                    }
                }
                if log::log_enabled!(log::Level::Debug) && mode == MatMode::Light {
                    if let Some(sh) = sh {
                        log::debug!("light shader {} params {:?}", sname, sh.params.iter().map(|(h, p)| format!("{:08x}={:?}", h, p)).collect::<Vec<_>>());
                    }
                    let mut cols: hashbrown::HashMap<[u8; 4], u32> = hashbrown::HashMap::new();
                    for c in &mesh.colors {
                        *cols.entry(*c).or_insert(0) += 1;
                    }
                    let mut c1: hashbrown::HashMap<[u8; 4], u32> = hashbrown::HashMap::new();
                    for c in &mesh.colors1 {
                        *c1.entry(*c).or_insert(0) += 1;
                    }
                    let (mut u0, mut u1) = (glam::Vec2::splat(9e9), glam::Vec2::splat(-9e9));
                    for u in &mesh.uv1 {
                        u0 = u0.min(*u);
                        u1 = u1.max(*u);
                    }
                    let mut cv: Vec<_> = cols.into_iter().collect();
                    cv.sort_by_key(|x| std::cmp::Reverse(x.1));
                    cv.truncate(6);
                    let mut c1v: Vec<_> = c1.into_iter().collect();
                    c1v.sort_by_key(|x| std::cmp::Reverse(x.1));
                    c1v.truncate(4);
                    log::debug!("light mesh {} #{} verts {} colors(argb) {:?} colors1 {:?} uv1 {:?}..{:?} names {:?}", part.primary, mi, mesh.positions.len(), cv, c1v, u0, u1, part.names.iter().take(12).collect::<Vec<_>>());
                }
                let (d, k0) = if sname.eq_ignore_ascii_case("LicensePlate") && dn.is_none() {
                    plate_texture(gpu, world)
                } else {
                    resolve(gpu, world, dn.as_deref())
                };
                let (s2, k1) = resolve(gpu, world, sn.as_deref());
                // (render only, blade round - PS_CarbonFiber's NormalSampler, the weave's normal map, rides as the CarbonFiber
                // meshes' second texture; city.wgsl reads it in the carbon branch. MCLA_CARBON_OLD=1: none)
                let (s2, k1) = if sname.eq_ignore_ascii_case("CarbonFiber") && !car_is_ambient && std::env::var("MCLA_CARBON_OLD").is_err() { carbon_weave(gpu, world) } else { (s2, k1) };
                // (the windows of a car with the full glass program, PS_GlassFront: the detail mask and the
                // crack map in the two slots, marked by lens -1 below; city.wgsl reads them for a cracked
                // or broken pane and draws the pane white as before)
                let win_maps = if sname.eq_ignore_ascii_case("CarGlass") && !car_is_ambient && dn.is_none() && sn.is_none() { glass_maps(gpu, world) } else { None };
                let ((d, k0), (s2, k1)) = win_maps.clone().unwrap_or(((d, k0), (s2, k1)));
                // meshes sharing textures and material mode are merged into one draw (the side
                // windows' glass apart, by side: it moves with its bone)
                let win_side = if part.is_base { car.windows.iter().position(|w| w.as_ref().map_or(false, |w| part.mesh_tag.get(mi) == Some(&w.tag))).map_or(0, |k| k as u32 + 1) } else { 0 };
                let win_side = if win_side != 0 { win_side } else { part_group(part, &part_xf, mi) };
                // (a traffic model's own wheel: tyre, rim and disc on one of its wheel bones -
                // groups 7 - 12, the bone's place its hub)
                let own_wheel = if car_is_ambient && !car.bike { part.mesh_wheel.get(mi).copied().flatten() } else { None };
                if let Some(i) = own_wheel.filter(|_| lod == 0) {
                    own_hubs[i] = Some((part_xf * bone_m).w_axis.truncate());
                }
                let win_side = own_wheel.map_or(win_side, |i| 7 + i as u32);
                if win_side >= 3 && std::env::var("MCLA_PARTS_LOG").is_ok() {
                    log::info!("damage parts {}: {} mesh {} -> group {}", car.name, part.primary, mi, win_side);
                }
                let acc = merged.entry((k0, k1, mode as u32 | win_side << 16 | lod << 24)).or_insert_with(|| MergeAcc { verts: Vec::new(), indices: Vec::new(), d, s2, sname: sname.to_string(), mode, min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) });
                // (the glass over a lamp is its lens: red over the tail lamps, clear elsewhere -
                // the game tints it from a table per lamp, BumpedTintColors)
                let pn = part.primary.to_ascii_lowercase();
                let (glass, lamp) = (sname.to_ascii_lowercase().contains("bumpedglass"), mode == MatMode::Light);
                if lamp && lod == 0 {
                    let xf = part_xf * bone_m;
                    // (MCLA_LAMP_DUMP: every lamp mesh in full - vertices with their lamp id, triangles; work/lighting/fe_lamps.py)
                    if std::env::var("MCLA_LAMP_DUMP").is_ok() {
                        for (k, p) in mesh.positions.iter().enumerate() {
                            let w = xf.transform_point3(*p);
                            eprintln!("lampv {} {} {} {} {} {:.4} {:.4} {:.4}", car.name, part.primary, mi, k, mesh.uv1.get(k).map_or(-1, |u| u.x.round() as i32), w.x, w.y, w.z);
                        }
                        for t in mesh.indices.chunks_exact(3) {
                            eprintln!("lampt {} {} {} {} {} {}", car.name, part.primary, mi, t[0], t[1], t[2]);
                        }
                    }
                    for (k, p) in mesh.positions.iter().enumerate() {
                        let id = mesh.uv1.get(k).map_or(-1, |u| u.x.round() as i32);
                        if matches!(id, 0 | 1 | 3 | 6 | 12 | 13 | 14) {
                            let w = xf.transform_point3(*p);
                            let e = lamp_acc.entry((id, w.x > 0.0)).or_insert((Vec3::ZERO, 0));
                            e.0 += w;
                            e.1 += 1;
                            if std::env::var("MCLA_GLOW_DUMP").is_ok() {
                                eprintln!("lampvert {} {} mesh {} id {} {:.4} {:.4} {:.4}", car.name, part.primary, mi, id, w.x, w.y, w.z);
                            }
                        }
                    }
                }
                let lens = if (glass || lamp) && (pn.contains("tail") || pn.contains("brake")) {
                    1.0
                } else if glass {
                    2.0
                } else if win_maps.is_some() {
                    -1.0
                } else {
                    0.0
                };
                let first = acc.verts.len();
                acc.push(mesh, part_xf * bone_m, dark_above, lens, if !car_is_ambient { 0 } else if sname.eq_ignore_ascii_case("AmbientTire") { 2 } else { 1 });
                // (render only - car parts drawn by their own programs' rules in city.wgsl, see
                // work/lighting bs3.py / bmprobe.py / cglassprobe.py: the material's terms ride in
                // the vertices. BumpSpec family: uv2 = (-(1 + 200 smoothness + 256 x 200
                // SpecularColor), 1 + base r + 256 base g), uv1 = (base b + 256 x 32 SpecMapExp, 0),
                // colour alpha = 10 metallic + 16 cut-out + 32 cabin part + 64 GrillOpaque (its
                // soft reflection goes by the texture). Brushed metal: uv2.x =
                // 1 + ReflectColorMod. Opaque black glass: uv2.x = 1. Cabin paint: colour alpha 32
                // (the cabin's own-lamp colours are the gauge colours, not the outside lamps').)
                // (MCLA_CARPARTS_OLD=1 with MCLA_DEBUG_VIEW=72: everything as it was before this, for comparison)
                if !car_is_ambient && std::env::var("MCLA_CARPARTS_OLD").is_err() {
                    use mcla_formats::rage::drawable::ShaderParam;
                    let pv = |h: u32, d: f32| sh.and_then(|s| s.params.iter().find_map(|(ph, p)| match p { ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0]), _ => None })).unwrap_or(glam::Vec4::splat(d));
                    let by = |x: f32, k: f32| if x.is_finite() { (x.abs() * k).round().clamp(0.0, 255.0) } else { 0.0 };
                    let cabin = pn.starts_with("interior") || pn.contains("_int") || pn.starts_with("seats") || pn.starts_with("steer_whl");
                    let alpha_cut = sname.eq_ignore_ascii_case("BumpSpecAlpha");
                    let spec_map = sname.eq_ignore_ascii_case("GrillOpaque");
                    if sname.eq_ignore_ascii_case("BumpSpec") || alpha_cut || spec_map {
                        let (bc, e, k, s, m) = (pv(0x1468b111, 1.0), pv(0xb37a7946, 1.0).x, pv(0xa98e2243, 0.0).x, pv(0x5e315fd1, 0.0).x, pv(0x83697767, 0.0).x);
                        let a = (by(m, 10.0).min(15.0) + if alpha_cut { 16.0 } else { 0.0 } + if cabin { 32.0 } else { 0.0 } + if spec_map { 64.0 } else { 0.0 }) as u8;
                        for v in &mut acc.verts[first..] {
                            v.uv2 = [-(1.0 + by(s, 200.0) + 256.0 * by(k, 200.0)), 1.0 + by(bc.x, 255.0) + 256.0 * by(bc.y, 255.0)];
                            v.uv1 = [by(bc.z, 255.0) + 256.0 * by(e, 32.0), 0.0];
                            v.color[3] = a;
                        }
                    } else if sname.eq_ignore_ascii_case("BrushedMetal") {
                        let rc = pv(0xaffa32ba, 1.0).x;
                        for v in &mut acc.verts[first..] {
                            v.uv2 = [1.0 + if rc.is_finite() { rc.clamp(0.0, 4.0) } else { 1.0 }, 0.0];
                        }
                    } else if sname.eq_ignore_ascii_case("BlackOpaqueGlass") {
                        for v in &mut acc.verts[first..] {
                            v.uv2 = [1.0, 0.0];
                        }
                    } else if badge {
                        // (BumpedAlphaChrome, PS_BumpedChrome of cars/0x1d68d869: PS_Chrome's mirror x
                        // (0.125 + 1.185 sat(collector - 0.25)) x baked occlusion, alpha = 2 x the
                        // texture's alpha - 1; city.wgsl cuts it out where that is not above 0 (uv2.y = 3))
                        for v in &mut acc.verts[first..] {
                            v.uv2 = [0.0, 3.0];
                        }
                    } else if sname.eq_ignore_ascii_case("CarbonFiber") && std::env::var("MCLA_CARBON_OLD").is_err() {
                        // (render only - PS_CarbonFiber of cars/0x84fc57d9, city.wgsl: uv2.x = 2)
                        for v in &mut acc.verts[first..] {
                            v.uv2 = [2.0, 0.0];
                        }
                    } else if cabin && mode == MatMode::Paint {
                        for v in &mut acc.verts[first..] {
                            v.color[3] = 32;
                        }
                    } else if sname.eq_ignore_ascii_case("LicensePlate") && std::env::var("MCLA_PLATE_OLD").is_err() {
                        // (render only - the number plate is drawn by its own program's rule,
                        // PS_Plate of cars/0x94eca68d, city.wgsl: marked like a BumpSpec part with
                        // the cut-out and the GrillOpaque bits both set, 16 + 64)
                        for v in &mut acc.verts[first..] {
                            v.uv2 = [-1.0, 1.0];
                            v.uv1 = [0.0, 0.0];
                            v.color[3] = 80;
                        }
                    }
                }
                // cabin trim: the colour group of the mesh's zone (uv1.x), by the vehicle's own
                // table, rides in uv2.y as 100 + group (city.wgsl, mode 17; nothing else of that
                // mode reads uv2.y) - the original builds the trim program's colour table from
                // the same table every frame (sub_82358AE0)
                if mode == MatMode::Interior {
                    if let Some(table) = zone_table.as_ref() {
                        for v in &mut acc.verts[first..] {
                            let z = v.uv1[0].round();
                            if (0.0..25.0).contains(&z) {
                                v.uv2[1] = 100.0 + table[z as usize] as f32;
                            }
                        }
                    }
                }
                // a part painted by itself: its colour in the vertices' own colour, marked by its
                // alpha - the paint shader takes it in place of the car's (city.wgsl, mode paint)
                if mode == MatMode::Paint {
                    let kind = match vehicle_model::classify_slot(&part.primary, &part.names) {
                        Slot::Hood => "hood",
                        Slot::Trunk => "trunk",
                        Slot::BumperF => "bumper_f",
                        Slot::BumperR => "bumper_r",
                        Slot::Spoiler => "spoiler",
                        Slot::Skirts => "skirts",
                        Slot::DoorL | Slot::DoorR => "doors",
                        _ => "",
                    };
                    // (the part's paint area rides in uv2.y, 8 + area, for the player's own
                    // entry per area - the original's table per part slot, 0x82050FD0: hood and
                    // hood_wide 2, bumper_r 4, bumper_f 3, skirts 5, the doors and their wide
                    // ones 1, every other slot the body's. Nothing else of the paint reads uv2.y.)
                    let area = match kind {
                        "doors" => 1.0,
                        "hood" => 2.0,
                        "bumper_f" => 3.0,
                        "bumper_r" => 4.0,
                        "skirts" => 5.0,
                        _ => 0.0,
                    };
                    if area > 0.0 && !pn.starts_with("subbum") {
                        for v in &mut acc.verts[first..] {
                            v.uv2[1] = 8.0 + area;
                        }
                    }
                    if let Some(c) = cfg.part_paint.get(kind) {
                        // (the colour is the screen's: the shader's paint is linear)
                        let lin = |v: u8| ((v as f32 / 255.0).powf(2.2) * 255.0).round() as u8;
                        for v in &mut acc.verts[first..] {
                            v.color = [lin(c[0]), lin(c[1]), lin(c[2]), 255];
                        }
                    }
                }
            }
        }
        let mut keys: Vec<(u32, u32, u32)> = merged.keys().copied().collect();
        let mut batch_window: Vec<u8> = Vec::new();
        let mut lods: [(Vec<GpuBatch>, Vec<u8>); 2] = Default::default();
        keys.sort_by_key(|k| (k.2, k.0, k.1));
        if std::env::var("MCLA_WHEEL_DUMP").is_ok() {
            if let Some(h) = car.hubs {
                // profile of the body over the front and rear wheel and at mid-car
                for (label, z) in [("front hub", h[0].z), ("rear hub", h[2].z), ("middle", 0.0)] {
                    let mut ys: Vec<f32> = Vec::new();
                    for acc in merged.values().filter(|a| a.mode == MatMode::Paint) {
                        for v in &acc.verts {
                            if (v.pos[2] - z).abs() < 0.05 && v.pos[0].abs() > h[1].x.abs() - 0.02 {
                                ys.push(v.pos[1]);
                            }
                        }
                    }
                    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    log::info!("body profile {} (z {:.2}): {} outer paint verts, lowest {:?}", label, z, ys.len(), ys.iter().take(6).collect::<Vec<_>>());
                }
                let mut lo = f32::INFINITY;
                let mut which = String::new();
                for acc in merged.values() {
                    for v in &acc.verts {
                        if v.pos[1] < lo {
                            lo = v.pos[1];
                            which = acc.sname.clone();
                        }
                    }
                }
                log::info!("body lowest vertex y {:.3} in shader {}", lo, which);
            }
        }
        for k in keys {
            let acc = merged.remove(&k).unwrap();
            if acc.indices.is_empty() {
                continue;
            }
            // (the windows with the glass maps also carry the in-car program's noise as their third texture)
            let win_noise = if k.0 == mcla_formats::hash::joaat("window_detail_mask.dds") ^ 0x0052_4157 && acc.mode == MatMode::CarGlass { glass_noise(gpu, world) } else { None };
            let bind = match win_noise {
                Some((t3, k2)) => gpu.texture_bind_group3(&acc.d, &acc.s2, &t3, k.0, k.1, k2),
                None => gpu.texture_bind_group(&acc.d, &acc.s2, k.0, k.1),
            };
            let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&acc.verts), usage: wgpu::BufferUsages::VERTEX });
            let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&acc.indices), usage: wgpu::BufferUsages::INDEX });
            let (group, lod) = ((k.2 >> 16) as u8, (k.2 >> 24) as usize);
            if group != 0 {
                log::debug!("car {}: moving group {} (1/2 windows, 3 hood, 4 trunk, 5/6 bumpers) batch shader {} tris {}", car.name, group, acc.sname, acc.indices.len() / 3);
            }
            let b = GpuBatch { shader: acc.sname, vb, ib, count: acc.indices.len() as u32, bind, mode: acc.mode, min: acc.min, max: acc.max };
            if lod == 0 {
                batch_window.push(group);
                batches.push(b);
            } else {
                lods[lod - 1].1.push(group);
                lods[lod - 1].0.push(b);
            }
        }
        let mut params = car_params(&car);
        // cars with the original's tuning drive on the ported simulation (MCLA_OLD_PHYSICS keeps the simple model)
        let simtunes = if std::env::var("MCLA_OLD_PHYSICS").is_ok() { None } else { car.simtune.clone() };
        if let Some(s) = simtunes.as_ref() {
            let mut vt = crate::physics::vehsim::VehTune::new(s.base.clone());
            vt.bike = car.bike;
            if let Some(hubs) = car.hubs {
                vt.seat((hubs[0].y + hubs[2].y) * 0.5);
            }
            params.veh = Some(Arc::new(vt));
            // (the body's collision hull, shaped as the game shapes it for this tuning)
            if let (Some(h), Some(hubs)) = (car.hull.as_ref(), car.hubs) {
                params.hull = Some(Arc::new(crate::physics::hull::shaped(h, &s.base, (hubs[0].y + hubs[2].y) * 0.5, car.bike)));
            }
            let d = &s.base.damage;
            if d.max_damage > 1.0 && d.wipeout_threshold > 1.0 {
                params.dmg = [d.max_damage, d.med_damage, d.impact_threshold, d.fatal_blow, d.wipeout_threshold];
            }
        }
        log::info!("car {}: hubs {:?} bounds {:?}..{:?} wheel radius {:.3}", car.name, car.hubs, car.min, car.max, params.wheel_radius);
        let params_mods = car_params_mods(&car, &params);
        log::info!("car model {}: {} batches (LOD 1 {}, LOD 2 {}), mass {:.0}", car.name, batches.len(), lods[0].0.len(), lods[1].0.len(), params.mass);
        // stock wheels: whl_stk_<car> for the base cars, whld_stk_<car> for the DLC set
        let base_name = car.name.to_ascii_lowercase().replace("_dub", "");
        let rims_root = world.assets.root.join("resources/rims");
        let cands = [
            base_name.strip_prefix("vpd_").map(|n| format!("whld_stk_{n}")),
            base_name.strip_prefix("vp_").map(|n| format!("whl_stk_{n}")),
            // a few are stored without the model year
            base_name.strip_prefix("vp_").and_then(|n| n.rsplit_once('_').map(|(a, _)| format!("whl_stk_{a}"))),
        ];
        let tyre = if car.bike { "tir_bike" } else { "tir_car" };
        let rim_ratio = if car.bike { 0.812 } else { stock_rim_ratio(&world.assets.root, &car.name, params.wheel_radius) };
        let mut wheels = None;
        let mut rim_dir = None;
        for c in cands.iter().flatten() {
            let dir = rims_root.join(c);
            if dir.is_dir() {
                wheels = self.wheel_set(gpu, world, &dir, tyre, rim_ratio);
                rim_dir = Some(dir);
                break;
            }
        }
        if std::env::var("MCLA_PARTS_DUMP").is_ok() {
            for p in &car.parts {
                let l = p.primary.to_ascii_lowercase();
                if ["gauge", "scissor", "door", "steer", "blower", "post"].iter().any(|k| l.contains(k)) {
                    log::info!("part {} placed {} stock {} meshes {} box {:?}..{:?} at {:?} names {:?}", p.primary, p.placed, p.stock, p.meshes.len(), p.min, p.max, p.transform.w_axis, p.names.iter().take(4).collect::<Vec<_>>());
                }
            }
        }
        // the brakes: the car's own where the disc has them, else the plain set
        let mut brake_dir = None;
        let brakes = if car.bike {
            None
        } else {
            let root = world.assets.root.join("resources/brakes");
            let own = [base_name.strip_prefix("vpd_").map(|n| format!("brkd_stk_{n}")), base_name.strip_prefix("vp_").map(|n| format!("brk_stk_{n}"))];
            let dir = own.iter().flatten().map(|n| root.join(n)).find(|d| d.is_dir()).unwrap_or_else(|| root.join("brk_car_base"));
            let k = rim_ratio * ROTOR_OF_RIM;
            let shift = brake_shift(wheels.as_ref().map_or(-0.2, |w: &Arc<WheelModel>| w.spoke_x));
            let key = (dir.clone(), (k * 200.0).round() as u32, (shift * 200.0).round() as i32);
            brake_dir = Some(dir.clone());
            match self.brake_cache.get(&key) {
                Some(b) => Some(b.clone()),
                None => {
                    let b = load_brakes(gpu, world, &dir, k, shift).map(Arc::new);
                    if let Some(b) = b.as_ref() {
                        self.brake_cache.insert(key, b.clone());
                    }
                    b
                }
            }
        };
        let lamps: Vec<(u8, Vec3)> = lamp_acc.iter().map(|((id, _), (sum, n))| (*id as u8, *sum / (*n).max(1) as f32)).collect();
        let own_wheels = own_hubs.iter().any(|h| h.is_some()).then_some(own_hubs);
        let (bw, bl) = if car_is_ambient { traffic::bound_size(&world.assets.root, &car.name) } else { None }.unwrap_or((car.max.x - car.min.x, car.max.z - car.min.z));
        let lod_radius = 0.5 * std::f32::consts::SQRT_2 * bw.max(bl);
        let lod_centre = if car_is_ambient { traffic::bound_centre(&world.assets.root, &car.name) } else { None }.unwrap_or((car.min + car.max) * 0.5);
        let lod_steps = if car_is_ambient { traffic::lod_steps(&world.assets.root, &car.name) } else { None }.unwrap_or([40.0, 155.0]);
        let m = Arc::new(CarModel { lods, lod_radius, lod_centre, lod_steps, own_wheels, name: car.name.clone(), lamps, glow_bones, batches, params, params_mods, simtunes, aitunes: car.aitune.clone(), rest_height: car.hubs.map(|h| (h[0].y + h[2].y) * 0.5), wheels, brakes, brake_dir, rim_dir, rim_ratio, bike: car.bike, min: car.min, max: car.max, cfg: cfg.clone(), variants, variant_labels, extra_variants, batch_window, windows: car.windows.clone(), damage_layout: car.damage_layout.clone().map(Arc::new), part_rig, hood_slot, trunk_bone });
        self.models.push(m.clone());
        if self.wheels.is_none() {
            self.wheels = load_wheels(gpu, world).map(Arc::new);
        }
        Ok(m)
    }

    /// Tyre + rim model for a rim directory, loaded once.
    /// The garage's BRAKES choice on the player's car (`None`: the car's own).
    pub fn set_player_brake(&mut self, gpu: &mut Gpu, world: &mut World, name: Option<&str>) {
        // (the car's own brake too: it stands behind the spokes of the rim now fitted)
        let Some((ratio, spoke_x, own)) = self.player().filter(|c| !c.model.bike).map(|c| (c.model.rim_ratio, c.wheels.as_ref().or(c.model.wheels.as_ref()).map_or(-0.2, |w| w.spoke_x), c.model.brake_dir.clone())) else { return };
        let k = ratio * ROTOR_OF_RIM;
        let shift = brake_shift(spoke_x);
        let Some(dir) = name.map(|n| world.assets.root.join("resources/brakes").join(n)).or(own) else { return };
        let tag = format!("{}@{:.3}@{:.3}", dir.display(), k, shift);
        if tag == self.player_brake_name {
            return;
        }
        let key = (dir.clone(), (k * 200.0).round() as u32, (shift * 200.0).round() as i32);
        self.player_brake = match self.brake_cache.get(&key) {
            Some(b) => Some(b.clone()),
            None => {
                let b = load_brakes(gpu, world, &dir, k, shift).map(Arc::new);
                if let Some(b) = b.as_ref() {
                    self.brake_cache.insert(key, b.clone());
                }
                b
            }
        };
        self.player_brake_name = tag;
    }

    /// The garage's TIRES choice on the player's car (a set's name; empty = the plain set).
    pub fn set_player_tire(&mut self, gpu: &mut Gpu, world: &mut World, name: &str) {
        if self.player_tire == name {
            return;
        }
        self.player_tire = name.to_string();
        let Some((bike, stock)) = self.player().map(|c| (c.model.bike, c.model.rim_dir.clone().map(|d| (d, c.model.rim_ratio)))) else { return };
        if bike {
            return;
        }
        if let Some((dir, ratio)) = self.player_rim.clone().or(stock) {
            self.tire_now = self.player_tire.clone();
            let w = self.wheel_set(gpu, world, &dir, "tir_car", ratio);
            self.tire_now.clear();
            if let Some(p) = self.player_mut() {
                p.wheels = w;
            }
        }
    }

    fn wheel_set(&mut self, gpu: &mut Gpu, world: &mut World, rim_dir: &Path, tyre: &str, rim_ratio: f32) -> Option<Arc<WheelModel>> {
        // (every tyre has a set of textures: the one chosen, else the first of the garage's
        // list - a zero in a vehicle's record)
        let set = if !self.tire_now.is_empty() { self.tire_now.clone() } else if tyre == "tir_bike" { "tir_bike_pir_corsa3".to_string() } else { "tir_car_bfg_kdw_blk".to_string() };
        let key = (rim_dir.to_path_buf(), format!("{tyre}+{set}"), (rim_ratio * 200.0).round() as u32);
        if let Some(w) = self.wheel_cache.get(&key) {
            return Some(w.clone());
        }
        let w = Arc::new(load_wheels_with(gpu, world, rim_dir, tyre, rim_ratio, Some(&set))?);
        self.wheel_cache.insert(key, w.clone());
        Some(w)
    }

    /// Step 19d: the cars that came with a set-up's name (`vinyl_name`: a rival's, a
    /// hookman's, the payback's cars) are put into it - one car a call, since a model may
    /// have to be built: the model with the set-up's parts, its rims at their size on its
    /// tires, its brakes, its paints (`CarLook`), its share of upgrades. `resolve(car,
    /// set-up)` reads the set-up (`Game::mc4_dress_of`). Returns the car dressed.
    pub fn dress_setups(&mut self, gpu: &mut Gpu, world: &mut World, resolve: &mut dyn FnMut(&str, &str) -> Option<Dress>) -> Option<usize> {
        let i = self.cars.iter().position(|c| !c.is_player && !c.dressed && !c.vinyl_name.is_empty() && c.parked.is_none())?;
        self.cars[i].dressed = true;
        let name = self.cars[i].model.name.clone();
        let d = resolve(&name, &self.cars[i].vinyl_name)?;
        let dir = world.assets.root.join("resources/vehicle").join(&name);
        let model = match self.models.iter().find(|m| m.name == name && m.cfg == d.cfg) {
            Some(m) => m.clone(),
            None if dir.is_dir() => match self.load_model_cfg(gpu, world, &dir, &d.cfg) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("vehicles: {} could not be built in its set-up {}: {}", name, self.cars[i].vinyl_name, e);
                    self.cars[i].model.clone()
                }
            },
            None => self.cars[i].model.clone(),
        };
        let mut wheels = None;
        let mut brake = None;
        if !model.bike {
            let ratio = if d.inches > 0 { rim_ratio_for(d.inches as f32, model.params.wheel_radius) } else { model.rim_ratio };
            if let Some(rd) = d.rim_dir.clone().or(model.rim_dir.clone()) {
                self.tire_now = d.tire.clone();
                wheels = self.wheel_set(gpu, world, &rd, "tir_car", ratio);
                self.tire_now.clear();
            }
            // (the brake stands behind the spokes of the rim fitted, as the player's)
            let spoke_x = wheels.as_ref().or(model.wheels.as_ref()).map_or(-0.2, |w| w.spoke_x);
            if let Some(bdir) = d.brake.as_ref().map(|n| world.assets.root.join("resources/brakes").join(n)).filter(|p| p.is_dir()) {
                let (k, shift) = (model.rim_ratio * ROTOR_OF_RIM, brake_shift(spoke_x));
                let key = (bdir.clone(), (k * 200.0).round() as u32, (shift * 200.0).round() as i32);
                brake = match self.brake_cache.get(&key) {
                    Some(b) => Some(b.clone()),
                    None => {
                        let b = load_brakes(gpu, world, &bdir, k, shift).map(Arc::new);
                        if let Some(b) = b.as_ref() {
                            self.brake_cache.insert(key, b.clone());
                        }
                        b
                    }
                };
            }
        }
        let c = &mut self.cars[i];
        c.model = model;
        if wheels.is_some() {
            c.wheels = wheels;
        }
        log::info!("vehicles: {} wears its set-up {}: rim {:?} at {} in, tires {:?}, brake {:?}, paint type {}, upgrades {:?}", name, c.vinyl_name, d.rim_dir.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()), d.inches, d.tire, d.brake, d.look.paint_type, d.perf);
        c.brake = brake;
        c.color = d.color;
        c.look = Some(Arc::new(d.look));
        // (step 19e: and the set-up's plate)
        if !d.plate.is_empty() && c.plate_text != d.plate {
            c.plate_text = d.plate.clone();
            c.plate = None;
        }
        // (a car that was given its share when it was made - a hookman's mod value - keeps it)
        if let Some(p) = d.perf.filter(|_| c.perf == 0.0) {
            c.set_perf(p);
        }
        Some(i)
    }

    pub fn spawn(&mut self, gpu: &Gpu, model: Arc<CarModel>, pos: Vec3, yaw: f32, color: [f32; 4], is_player: bool) -> usize {
        let binding = gpu.new_object(&ObjectUniform::default());
        let wheel_bindings = (0..4).map(|_| gpu.new_object(&ObjectUniform::default())).collect();
        let caliper_bindings = (0..4).map(|_| gpu.new_object(&ObjectUniform::default())).collect();
        let window_bindings = if model.windows.iter().any(|w| w.is_some()) { (0..2).map(|_| gpu.new_object(&ObjectUniform::default())).collect() } else { Vec::new() };
        let part_bindings = match model.batch_window.iter().copied().max().unwrap_or(0) {
            g if g >= 7 => (0..10).map(|_| gpu.new_object(&ObjectUniform::default())).collect(),
            g if g >= 3 => (0..4).map(|_| gpu.new_object(&ObjectUniform::default())).collect(),
            _ => Vec::new(),
        };
        let mut ai = None;
        if !is_player {
            let mut d = AiDriver::new(self.rng ^ 0x9e3779b9);
            // (ambient traffic keeps to the road's speed - 40 mph on nearly every road, which is
            // also every ambient type's fMaxSpeed - some a little under it)
            d.target_speed = 17.88 * (0.86 + self.rand() * 0.14);
            ai = Some(d);
        }
        self.next_uid += 1;
        let uid = self.next_uid;
        self.cars.push(CarInstance { uid, model, binding, wheel_bindings, caliper_bindings, state: CarState::new(pos, yaw), input: CarInput::default(), color, ai, brain: None, placed: false, is_player, wheels: None, lean: 0.0, perf: 0.0, tuned: None, cop: false, stunned: 0.0, parked: None, item: None, amb_born: 0.0, amb_life: 8.0, removing: None, settle: 0.0, racer: false, pinned: false, boost: 0.0, driver: String::new(), plate_text: String::new(), plate: None, vinyl_name: String::new(), vinyl: None, vinyl_done: false, setup: None, pose_history: Default::default(), inflict: 1.0, goon: false, look: None, brake: None, dressed: false, driver_set: String::new(), pu_ctl: 0, pu_ghost: false, pu_mirror: 0.0, pu_stealth: false, window_bindings, windows: Default::default(), part_bindings, rail: None, amb_lamps: AmbLamps::default(), park_steer: 0.0, park_held: false, amb_lift: [0.0; 6] });
        self.cars.len() - 1
    }

    /// Swap the player's model/colour/kit (garage).
    pub fn replace_player(&mut self, gpu: &mut Gpu, world: &mut World, dir: &Path, color: [f32; 4], cfg: &CarConfig, rim_dir: Option<&Path>) -> Result<()> {
        let name = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let model = match self.models.iter().find(|m| m.name == name && m.cfg == *cfg) {
            Some(m) => m.clone(),
            None => self.load_model_cfg(gpu, world, dir, cfg)?,
        };
        // aftermarket rims only when one is chosen; otherwise the car keeps its stock wheels
        let ratio = if self.player_rim_inches > 0 { rim_ratio_for(self.player_rim_inches as f32, model.params.wheel_radius) } else { model.rim_ratio };
        self.tire_now = self.player_tire.clone();
        self.player_rim = match rim_dir {
            Some(rd) if !rd.as_os_str().is_empty() && !model.bike => Some((rd.to_path_buf(), ratio)),
            _ => model.rim_dir.clone().filter(|_| !model.bike).map(|d| (d, ratio)),
        };
        let custom = match rim_dir {
            // (the stock rims with the tires chosen)
            None if !model.bike && !self.player_tire.is_empty() => match model.rim_dir.clone() {
                Some(rd) => self.wheel_set(gpu, world, &rd, "tir_car", ratio),
                None => None,
            },
            Some(rd) if rd.as_os_str().is_empty() && !model.bike && !self.player_tire.is_empty() => match model.rim_dir.clone() {
                Some(rd) => self.wheel_set(gpu, world, &rd, "tir_car", ratio),
                None => None,
            },
            Some(rd) if !rd.as_os_str().is_empty() && !model.bike => self.wheel_set(gpu, world, rd, "tir_car", ratio),
            // the stock rims in another size
            _ if !model.bike && (ratio - model.rim_ratio).abs() > 0.004 => match model.rim_dir.clone() {
                Some(rd) => self.wheel_set(gpu, world, &rd, "tir_car", ratio),
                None => None,
            },
            _ => None,
        };
        self.tire_now.clear();
        if let Some(p) = self.cars.iter_mut().find(|c| c.is_player) {
            let pos = p.state.pos + Vec3::Y * 0.5;
            let f = p.state.forward();
            let yaw = (-f.x).atan2(-f.z);
            // (the same vehicle built again keeps its damage; another one comes without any - its own
            // damage map, made again with its layout)
            let same = p.model.name == model.name;
            if !same {
                let uid = p.uid;
                self.dmg_maps.retain(|m| m.uid != uid);
            }
            p.model = model;
            p.wheels = custom;
            p.tuned = None;
            p.setup = None;
            let perf = p.perf;
            p.set_perf(perf);
            p.color = color;
            let old = std::mem::replace(&mut p.state, CarState::new(pos, yaw));
            if same {
                p.state.keep_damage(&old);
            }
            p.pose_history.reset();
            p.placed = false;
        }
        Ok(())
    }

    pub fn player(&self) -> Option<&CarInstance> {
        self.cars.iter().find(|c| c.is_player)
    }

    /// A car with a slipstream turbo: the player's (mcPlayer::CreateCar 0x822A6560) and every opponent's but a
    /// police car's (sub_82744358: not catalogue class 5) - race opponents and the street racers of free roam.
    /// Traffic and the police have none.
    pub fn has_sst(c: &CarInstance) -> bool {
        (c.is_player || c.racer || c.ai.as_ref().map_or(false, |a| a.brain.is_some())) && !c.cop && c.state.veh.is_some() && c.params().veh.is_some()
    }

    /// The slipstream search of every car with a turbo (mcCarSSTurbo 0x82346438 over the player manager's
    /// list, 0x82346140 for each car), made once before the cars are stepped - the original makes it in each
    /// car's update, after the cars ahead of it in the list have moved. A car slower than its own tune +32
    /// finds nothing; the others are every car with a turbo but a motorcycle (catalogue class 3, SportBike:
    /// 0x82332098), the strongest slipstream the one kept (the first of equals). (Not ported: a car whose
    /// mcCar +48 object has a u16 +18 set is passed over.) Its car is told it is followed (+68) and fades
    /// its trail in (clamp(strength + 0.75, 0, 1), its model +7976) - by a car that does not burn.
    fn draft_pass(&mut self, dt: f32) {
        use crate::physics::sst::{strength, DraftCar};
        let n = self.cars.len();
        let info: Vec<Option<(DraftCar, Arc<crate::physics::vehsim::VehTune>, bool)>> = self.cars.iter().map(|c| {
            if !Self::has_sst(c) {
                return None;
            }
            let vt = c.params().veh.clone()?;
            let st = &c.state;
            Some((DraftCar { pos: st.pos, up: st.up(), vel: st.vel, speed: st.vel.length() }, vt, c.model.bike))
        }).collect();
        let mut found = vec![(0.0f32, -1i32); n];
        for i in 0..n {
            let Some((me, vt, _)) = info[i].as_ref() else { continue };
            if me.speed < vt.t.ss_turbo.min_speed {
                continue;
            }
            for j in 0..n {
                let Some((o, ot, bike)) = info[j].as_ref() else { continue };
                if j == i || *bike {
                    continue;
                }
                let s = strength(&ot.t.ss_turbo, o, me.pos);
                if s > found[i].0 {
                    found[i] = (s, j as i32);
                }
            }
        }
        for i in 0..n {
            let Some(vs) = self.cars[i].state.veh.as_deref_mut() else { continue };
            vs.sst.found = found[i];
            let (s, j) = found[i];
            if vs.sst.burning || !(s > 0.0) || info[i].is_none() {
                continue;
            }
            if let Some(lv) = self.cars[j as usize].state.veh.as_deref_mut() {
                lv.sst.trail_target = if s > 0.0 { (s + 0.75).clamp(0.0, 1.0) } else { 0.0 };
                lv.sst.drafter = i as i32;
            }
        }
        let leader_of: Vec<i32> = self.cars.iter().map(|c| c.state.veh.as_ref().map_or(-1, |v| v.sst.leader)).collect();
        for i in 0..n {
            let me_player = self.cars[i].is_player;
            let Some(vs) = self.cars[i].state.veh.as_deref_mut() else { continue };
            let d = vs.sst.drafter;
            vs.sst.drafter_drafting = d >= 0 && (d as usize) < n && info[d as usize].is_some() && found[d as usize].0 > 0.0;
            // (0x82379D48: the trail is kept while the local player is this car, or the car in its slipstream
            // with this car as its leader)
            let keep = me_player || (d >= 0 && (d as usize) < n && self.cars[d as usize].is_player && leader_of[d as usize] == i as i32);
            let vs = self.cars[i].state.veh.as_deref_mut().unwrap();
            vs.sst.fade_trail(keep, dt);
        }
        // (MCLA_SST_LOG: each car's turbo every half second)
        self.sst_log_t += dt;
        if std::env::var_os("MCLA_SST_LOG").is_some() && self.sst_log_t >= 0.5 {
            self.sst_log_t = 0.0;
            for (i, c) in self.cars.iter().enumerate() {
                if let (Some(v), true) = (c.state.veh.as_ref(), info[i].is_some()) {
                    log::info!("sst car {i}{} speed {:.1} level {:.3} full {} burning {} strength {:.3} leader {} drafter {} fov {:.2}", if c.is_player { "P" } else { "" }, c.state.speed(), v.sst.level, v.sst.full, v.sst.burning, v.sst.strength, v.sst.leader, v.sst.drafter, v.sst.fov);
                }
            }
        }
    }
    pub fn player_mut(&mut self) -> Option<&mut CarInstance> {
        self.cars.iter_mut().find(|c| c.is_player)
    }

    /// Cars that left the world (through a hole, or off the edge of what is loaded).
    pub fn rescue_fallen(&mut self, world: &World, center: Vec3) {
        // a car that dropped through a hole in the world: traffic is taken away, anything that
        // matters goes back onto the road nearest to where it left the ground
        for c in self.cars.iter_mut() {
            if c.state.vel.y > -50.0 {
                continue;
            }
            let from = c.state.pos + Vec3::Y * (c.state.vel.y * c.state.vel.y / 27.0);
            let keep = c.is_player || c.pinned || c.racer || c.cop;
            let spot = world.nav.as_ref().and_then(|nav| nav.nearest_link(from).map(|(i, t, _)| (nav.links[i].point_at(t), nav.links[i].b - nav.links[i].a)));
if keep || c.state.pos.distance(center) < 450.0 {
                log::warn!("car {} ({:.0} m away, parked {}, vel {:.0} {:.0} {:.0}) fell out of the world at {:.0} {:.0} {:.0}: {}", c.model.name, c.state.pos.distance(center), c.parked.is_some(), c.state.vel.x, c.state.vel.y, c.state.vel.z, c.state.pos.x, c.state.pos.y, c.state.pos.z, if keep { "back on the road" } else { "removed" });
            }
            match spot {
                Some((p, dir)) if keep => {
                    c.state.relocate(p + Vec3::Y * 0.6, glam::Quat::from_rotation_y((-dir.x).atan2(-dir.z)));
                    c.state.vel = Vec3::ZERO;
                    c.state.ang_vel = Vec3::ZERO;
                    c.state.veh = None;
                    c.placed = false;
                    c.pose_history.reset();
                }
                _ => c.state.pos = center + Vec3::splat(1.0e5),
            }
        }
        self.cars.retain(|c| c.is_player || c.pinned || c.racer || c.cop || c.state.pos.distance(center) < 1.0e4);
    }

    /// Keep `traffic_target` AI cars alive around `center`.
    pub fn manage_traffic(&mut self, gpu: &Gpu, world: &World, center: Vec3, dt: f32) {
        // with the road network: the original's spawner and unspawner (traffic.rs)
        if world.nav.is_some() {
            self.spawn_traffic(gpu, world, center, dt);
            return;
        }
        // despawn far cars
        self.cars.retain(|c| c.is_player || c.pinned || c.parked.is_some() || c.state.pos.distance(center) < 420.0);
        self.spawn_timer -= dt;
        let n_traffic = self.cars.iter().filter(|c| !c.is_player && !c.cop && !c.racer && !c.pinned && c.parked.is_none()).count();
        // the roads empty out at night (density over the day from the ambient tables)
        let target = if self.traffic_target == 0 { 0 } else { ((self.traffic_target as f32 * self.ambient.density.at(self.hour)).round() as usize).max(1) };
        if n_traffic >= target || self.spawn_timer > 0.0 || self.models.is_empty() {
            return;
        }
        let Some(nav) = world.nav.as_ref() else { return };
        // pick a random connected link 120..300 m away
        let cands: Vec<usize> = nav.links_near(center, 300.0).filter(|&i| {
            let l = &nav.links[i];
            let d = l.a.distance(center);
            !l.next.is_empty() && !l.prev.is_empty() && d > 110.0 && d < 300.0 && l.len > 20.0
        }).collect();
        if cands.is_empty() {
            self.spawn_timer = 0.5;
            return;
        }
        // roads a race script has thinned out or filled up
        for (p, d) in std::mem::take(&mut self.road_density_new) {
            for i in nav.links_near(p, 30.0) {
                let l = &nav.links[i];
                let ab = l.b - l.a;
                let t = ((p - l.a).dot(ab) / ab.length_squared().max(1e-3)).clamp(0.0, 1.0);
                let q = l.a + ab * t;
                if (q.y - p.y).abs() < 7.0 && Vec3::new(q.x - p.x, 0.0, q.z - p.z).length() < 13.0 {
                    self.road_density.insert(l.lane, d);
                }
            }
        }
        let li = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
        let l = &nav.links[li];
        if let Some(&d) = self.road_density.get(&l.lane) {
            // 0 = kept clear, 3 and up = as busy as the city gets
            if self.rand() >= d as f32 * 0.34 {
                self.spawn_timer = 0.05;
                return;
            }
        }
        let t = 0.2 + self.rand() * 0.6;
        let pos = l.point_at(t) + Vec3::Y * 0.8;
        // keep distance from other cars
        if self.cars.iter().any(|c| c.state.pos.distance(pos) < 25.0) {
            self.spawn_timer = 0.2;
            return;
        }
        let dir = l.dir();
        let yaw = (-dir.x).atan2(-dir.z);
        // ambient traffic uses the game's "va_" vehicles when they are loaded
        let ambient: Vec<usize> = (0..self.models.len()).filter(|&i| self.models[i].name.starts_with("va_") && !is_trailer(&self.models[i].name)).collect();
        let pool: Vec<usize> = if ambient.is_empty() { (0..self.models.len()).filter(|&i| !self.models[i].name.contains("police")).collect() } else { ambient };
        if pool.is_empty() {
            return;
        }
        let mut mi = pool[(self.rand() * pool.len() as f32) as usize % pool.len()];
        // what drives in this district at this hour
        let r = self.rand();
        let models = &self.models;
        if let Some(name) = self.ambient.pick(pos, self.hour, r, &|n| !is_trailer(n) && models.iter().any(|m| m.name.eq_ignore_ascii_case(n))) {
            if let Some(i) = models.iter().position(|m| m.name.eq_ignore_ascii_case(name)) {
                mi = i;
            }
        }
        let model = self.models[mi].clone();
        let color = if model.name.contains("taxi") {
            [0.95, 0.72, 0.05, 0.0]
        } else {
            TRAFFIC_PALETTE[(self.rand() * TRAFFIC_PALETTE.len() as f32) as usize % TRAFFIC_PALETTE.len()]
        };
        let idx = self.spawn(gpu, model, pos, yaw, color, false);
        // it rides its lane as the original's traffic (traffic.rs)
        let name = self.cars[idx].model.name.clone();
        let ty = *self.amb_types.entry(name.clone()).or_insert_with(|| traffic::AmbType::load(&world.assets.root, &name));
        let seed = (self.rand() * 16777216.0) as u32 | 1;
        self.cars[idx].rail = Some(traffic::Rider::new(nav, ty, li, t * nav.links[li].len, seed));
        if std::env::var("MCLA_SPAWN_DUMP").is_ok() {
            log::info!("traffic spawn: link dir {:?} car forward {:?} dot {:.2}", dir, self.cars[idx].state.forward(), dir.dot(self.cars[idx].state.forward()));
        }
        if let Some(ai) = self.cars[idx].ai.as_mut() {
            ai.link = Some(li);
            ai.t = t;
        }
        self.spawn_timer = 0.3;
    }

    pub fn load_parking(&mut self, data_dir: &str) {
        let p = std::path::Path::new(data_dir).join("resources/city/sc/0x2e3038ce.r62");
        if let Ok(res) = crate::assets::load_resource_file(&p) {
            self.parking = mcla_formats::rage::parking::parse_parking(&res);
            log::info!("parking: {} spots in {} quadtree nodes", self.parking.spots.len(), self.parking.nodes.len());
        }
        let root = std::path::Path::new(data_dir);
        self.ambient = crate::ambient::Distribution::load(root, "va_");
        self.hour = 12.0;
        log::info!("ambient: {} traffic districts", self.ambient.types.len());
    }

    /// The parked cars are spawned and unspawned by the original's rules in the traffic
    /// spawner's manager frames (parked.rs); here only a car whose ground probe found nothing
    /// goes (its spot is free again: "could not spawn on terrain").
    pub fn manage_parked(&mut self, _gpu: &Gpu, _center: Vec3) {
        self.cars.retain(|c| c.parked.is_none() || c.pinned || c.settle != PARK_NO_GROUND);
    }

    /// Which livery file belongs to which car set-up (read once, then kept in a small index file).
    fn ensure_vinyl_index(&mut self, world: &World) {
        let root = &world.assets.root;
        if self.vinyl_index.is_none() {
            // which file is which set-up: read once, then kept in a small index file
            let mut map = hashbrown::HashMap::new();
            // (name, file, "1" if anything is painted on it: many set-ups are blank)
            let index = root.join("opponent_vinyls2.tsv");
            if let Ok(t) = std::fs::read_to_string(&index) {
                for line in t.lines() {
                    let mut it = line.split('\t');
                    if let (Some(k), Some(f)) = (it.next(), it.next()) {
                        map.insert(k.to_string(), std::path::PathBuf::from(f));
                        if it.next() == Some("1") {
                            self.vinyl_painted.insert(k.to_string());
                        }
                    }
                }
            }
            if map.is_empty() {
                let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("resources/opponent_vinyls")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
                files.sort();
                let mut out = String::new();
                for f in files {
                    let Ok(res) = crate::assets::load_resource_file(&f) else { continue };
                    let rd = mcla_formats::rage::Rd::new(&res);
                    let Some(t) = rd.ptr(8).and_then(|o| mcla_formats::rage::texture::Texture::parse(&rd, o).ok()) else { continue };
                    let name = t.name.rsplit(['/', '\\']).next().unwrap_or("").split('.').next().unwrap_or("").to_ascii_lowercase();
                    if !name.is_empty() {
                        let painted = t.to_rgba8().chunks_exact(4).filter(|p| p[3] > 40).count() > 600;
                        out.push_str(&format!("{}\t{}\t{}\n", name, f.display(), painted as u8));
                        if painted {
                            self.vinyl_painted.insert(name.clone());
                        }
                        map.insert(name, f);
                    }
                }
                let _ = std::fs::write(&index, out);
            }
            log::info!("vehicles: {} opponent liveries", map.len());
            self.vinyl_index = Some(map);
        }
    }

    /// Is a livery of this name ready (loaded or painted)?
    pub fn has_vinyl(&self, name: &str) -> bool {
        self.vinyls.get(&name.to_ascii_lowercase()).map_or(false, |v| v.is_some())
    }

    /// Forget the painted liveries whose name starts with `prefix`, except `keep`.
    pub fn drop_vinyls(&mut self, prefix: &str, keep: &str) {
        let (prefix, keep) = (prefix.to_ascii_lowercase(), keep.to_ascii_lowercase());
        self.vinyls.retain(|k, _| !k.starts_with(&prefix) || *k == keep);
    }

    /// Add a livery painted at run time (plain colour RGBA in the body's unwrap).
    pub fn add_vinyl_texture(&mut self, gpu: &mut Gpu, name: &str, w: u32, h: u32, px: &[u8]) {
        let name = name.to_ascii_lowercase();
        let tex = Arc::new(crate::render::create_rgba(&gpu.device, &gpu.queue, w, h, px, "vinyl"));
        let white = gpu.white.clone();
        let key = mcla_formats::hash::joaat(&format!("vinyl:{name}")) | 1;
        let bind = gpu.texture_bind_group3(&white, &white, &tex, key, key, key);
        self.vinyls.insert(name, Some(bind));
    }

    /// The ready-made liveries the disc has for a car (set-up names, sorted).
    pub fn vinyl_presets(&mut self, world: &World, car: &str) -> Vec<String> {
        self.ensure_vinyl_index(world);
        let tail = format!("_{}", car.to_ascii_lowercase());
        let mut v: Vec<String> = self.vinyl_index.as_ref().map(|m| m.keys().filter(|k| k.ends_with(&tail) && self.vinyl_painted.contains(*k)).cloned().collect()).unwrap_or_default();
        v.sort();
        v
    }

    /// Liveries of named opponents: `resources/opponent_vinyls` holds one finished vinyl
    /// texture per car set-up (stripes, sponsor logos), laid out in the body's unwrap
    /// (first UV set; unpainted-side parts all point at one empty corner texel).
    pub fn assign_vinyls(&mut self, gpu: &mut Gpu, world: &mut World) {
        if !self.cars.iter().any(|c| !c.vinyl_done && !c.vinyl_name.is_empty()) {
            return;
        }
        self.ensure_vinyl_index(world);
        for i in 0..self.cars.len() {
            if self.cars[i].vinyl_done || self.cars[i].vinyl_name.is_empty() {
                continue;
            }
            self.cars[i].vinyl_done = true;
            let name = self.cars[i].vinyl_name.to_ascii_lowercase();
            // the set-up's wheels: tyre sizes, track and ride height
            if let Ok(cfg) = std::fs::read(world.assets.root.join("resources/vehicle/configs").join(format!("{name}.xcc"))) {
                self.cars[i].set_setup(&cfg);
            }
            // the set-up's rim size (the rivals run 19 to 24 inch rims)
            let model = self.cars[i].model.clone();
            if let (Some(dir), false, Some(inches)) = (model.rim_dir.as_ref(), model.bike, config_rim_inches(&world.assets.root, &name)) {
                let ratio = rim_ratio_for(inches, model.params.wheel_radius);
                if (ratio - model.rim_ratio).abs() > 0.01 {
                    self.cars[i].wheels = self.wheel_set(gpu, world, dir, "tir_car", ratio);
                }
            }
            if !self.vinyls.contains_key(&name) {
                let tex = self.vinyl_index.as_ref().and_then(|m| m.get(&name)).and_then(|f| crate::assets::load_resource_file(f).ok()).and_then(|res| {
                    let rd = mcla_formats::rage::Rd::new(&res);
                    rd.ptr(8).and_then(|o| mcla_formats::rage::texture::Texture::parse(&rd, o).ok()).map(|t| (t.width, t.height, t.to_rgba8()))
                });
                let bind = tex.map(|(w, h, mut px)| {
                    // the finished liveries hold colour times alpha: back to plain colour
                    for p in px.chunks_exact_mut(4) {
                        let a = p[3] as u32;
                        if a > 0 && a < 255 {
                            for c in &mut p[..3] {
                                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                            }
                        }
                    }
                    let tex = Arc::new(crate::render::create_rgba(&gpu.device, &gpu.queue, w, h, &px, "vinyl"));
                    let white = gpu.white.clone();
                    let key = mcla_formats::hash::joaat(&format!("vinyl:{name}")) | 1;
                    gpu.texture_bind_group3(&white, &white, &tex, key, key, key)
                });
                log::info!("vehicles: livery {} {}", name, if bind.is_some() { "loaded" } else { "not found" });
                self.vinyls.insert(name.clone(), bind);
            }
            self.cars[i].vinyl = self.vinyls.get(&name).cloned().flatten();
        }
    }

    /// Give every car that has none a plate: the Californian background of the disc with
    /// the text set in the game's plate font (`vehicle/shared_font/fnt_license_plate.dds`:
    /// 8 x 5 cells of 32 px, "0-9A-Z" then hand, plus, star, heart for "[\\]^").
    pub fn assign_plates(&mut self, gpu: &mut Gpu, world: &World) {
        if !self.cars.iter().any(|c| c.plate.is_none() && c.placed) {
            return;
        }
        if self.plate_art.is_none() {
            let root = &world.assets.root;
            let font = std::fs::read(root.join("vehicle/shared_font/fnt_license_plate.dds")).ok().and_then(|d| mcla_formats::dds::parse_dds("fnt_license_plate", &d).ok()).filter(|t| t.width == 256 && t.height == 256).map(|t| t.to_rgba8());
            let dir = root.join("resources/license_plates/backgrounds/plate_ca_00");
            let file = std::fs::read_dir(&dir).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.extension().map_or(false, |x| x == "r1")));
            let bg = file.and_then(|f| crate::assets::load_resource_file(&f).ok()).and_then(|res| {
                let rd = mcla_formats::rage::Rd::new(&res);
                rd.ptr(0).and_then(|o| mcla_formats::rage::texture::Texture::parse(&rd, o).ok()).map(|t| (t.width, t.height, t.to_rgba8()))
            });
            self.plate_art = Some(match (bg, font) {
                (Some((w, h, px)), Some(f)) => (w, h, px, f),
                _ => (0, 0, Vec::new(), Vec::new()),
            });
        }
        let Some((w, h, bg, font)) = self.plate_art.clone() else { return };
        if w == 0 {
            // no art: stop trying
            let white = gpu.white.clone();
            let b = gpu.texture_bind_group(&white, &white, 0, 0);
            for c in self.cars.iter_mut().filter(|c| c.plate.is_none()) {
                c.plate = Some(b.clone());
            }
            return;
        }
        for i in 0..self.cars.len() {
            if self.cars[i].plate.is_some() || !self.cars[i].placed {
                continue;
            }
            if self.cars[i].plate_text.is_empty() {
                // "2BBE748": digit, three letters, three digits
                self.plate_serial = self.plate_serial.wrapping_mul(1664525).wrapping_add(1013904223);
                let mut r = self.plate_serial >> 4;
                let mut next = |n: u32| {
                    let v = r % n;
                    r = r / n ^ (r << 7);
                    v
                };
                let mut t = String::new();
                t.push((b'1' + next(8) as u8) as char);
                for _ in 0..3 {
                    t.push((b'A' + next(26) as u8) as char);
                }
                for _ in 0..3 {
                    t.push((b'0' + next(10) as u8) as char);
                }
                self.cars[i].plate_text = t;
            }
            let text = self.cars[i].plate_text.to_ascii_uppercase();
            // (the player's plate on its own background)
            let own_bg = self.player_plate_bg.clone().filter(|n| self.cars[i].is_player && n != "plate_ca_00").and_then(|n| {
                let art = self.plate_bgs.entry(n.clone()).or_insert_with(|| {
                    // (a background's folder holds two files; the texture is the one with an
                    // object behind its root pointer - not always the first of the two)
                    let dir = world.assets.root.join("resources/license_plates/backgrounds").join(&n);
                    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).ok().map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "r1")).collect()).unwrap_or_default();
                    files.sort();
                    files.into_iter().find_map(|f| {
                        let res = crate::assets::load_resource_file(&f).ok()?;
                        let rd = mcla_formats::rage::Rd::new(&res);
                        rd.ptr(0).and_then(|o| mcla_formats::rage::texture::Texture::parse(&rd, o).ok()).map(|t| (t.width, t.height, t.to_rgba8()))
                    })
                });
                art.clone().map(|a| (n, a))
            });
            let (slot, w, h, bg) = match own_bg {
                Some((n, (bw, bh, px))) => (format!("{text}@{n}"), bw, bh, px),
                None => (text.clone(), w, h, bg.clone()),
            };
            if !self.plates.contains_key(&slot) {
                let (w, h) = (w as usize, h as usize);
                let mut px = bg.clone();
                let chars: Vec<char> = text.chars().take(8).collect();
                // glyphs fill the middle band of the plate
                let ch = (h as f32 * 0.50) as usize;
                let cw = ((w as f32 * 0.86) / chars.len().max(6) as f32).min(ch as f32 * 0.72) as usize;
                let x0 = (w - cw * chars.len()) / 2;
                let y0 = (h as f32 * 0.30) as usize;
                for (k, c) in chars.iter().enumerate() {
                    let cell = match *c {
                        '0'..='9' => *c as usize - '0' as usize,
                        'A'..='Z' => 10 + *c as usize - 'A' as usize,
                        '[' => 36,
                        '\\' => 37,
                        ']' => 38,
                        '^' => 39,
                        _ => continue,
                    };
                    let (gx, gy) = (cell % 8 * 32, cell / 8 * 32);
                    for y in 0..ch {
                        for x in 0..cw {
                            let (sx, sy) = (gx + x * 32 / cw, gy + y * 32 / ch);
                            let a = font[(sy * 256 + sx) * 4 + 3] as u32;
                            if a == 0 {
                                continue;
                            }
                            let o = ((y0 + y) * w + x0 + k * cw + x) * 4;
                            // embossed dark blue characters (PS_Plate's letter colour, c109: 0.1412 0.1647 0.3412)
                            for (j, ink) in [36u32, 42, 87].iter().enumerate() {
                                px[o + j] = ((px[o + j] as u32 * (255 - a) + ink * a) / 255) as u8;
                            }
                        }
                    }
                }
                let tex = Arc::new(crate::render::create_rgba(&gpu.device, &gpu.queue, w as u32, h as u32, &px, "plate"));
                let key = mcla_formats::hash::joaat(&format!("plate:{slot}")) | 1;
                let bind = gpu.texture_bind_group(&tex, &tex, key, key);
                self.plates.insert(slot.clone(), bind);
            }
            self.cars[i].plate = self.plates.get(&slot).cloned();
        }
    }

    /// Physics + AI for every car.
    pub fn update(&mut self, world: &World, player_input: CarInput, dt: f32) {
        self.clock += dt;
        for c in self.cars.iter_mut() {
            c.state.touches.clear();
        }
        // (test hook: MCLA_WINDOW_TEST=1 rolls the player's side windows down)
        static WINDOW_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let window_test = *WINDOW_TEST.get_or_init(|| std::env::var("MCLA_WINDOW_TEST").is_ok_and(|v| v == "1"));
        for c in self.cars.iter_mut() {
            for k in 0..2 {
                if let Some(clip) = c.model.windows[k].as_ref() {
                    if window_test && c.is_player {
                        c.windows[k].request(true, clip.duration);
                    }
                    c.windows[k].update(dt, clip.duration);
                }
            }
        }
        // (diagnostics, N-VP-3: MCLA_WHEEL_GROUND="<update>[;x,y,z,R...]" prints at that update the
        // player's drawn wheels - middle, drawn radius - and how far each tyre's bottom stands off the
        // collision triangle under it, along its normal; then the same for the given middles)
        if let Ok(v) = std::env::var("MCLA_WHEEL_GROUND") {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let mut it = v.split(';');
            let at: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(300);
            if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == at {
                let off = |m: Vec3, r: f32| world.collider.ground(m, 1.0, 3.0).map(|h| ((m - h.point).dot(h.normal) - r, h.point.y, h.normal));
                for c in self.cars.iter().filter(|c| c.is_player || std::env::var("MCLA_WHEEL_GROUND_ALL").is_ok()) {
                    let up = c.state.rot * Vec3::Y;
                    eprintln!("wheel ground: car {} player {} setup {:?} pos {:?} up {:?} speed {:.2}", c.model.name, c.is_player, c.setup, c.state.pos, up, c.state.speed());
                    for i in 0..4 {
                        let r = c.params().veh.as_ref().map(|v| if i < 2 { v.t.wheel_front.radius } else { v.t.wheel_back.radius }).unwrap_or(c.model.params.wheel_radius);
                        let m = c.state.wheels[i].world_pos;
                        let local = c.state.rot.inverse() * (m - c.state.pos);
                        eprintln!("wheel ground: {i} middle {:?} local {:?} physics r {r:.4} drawn {:.4} bottom off {:?}", m, local, r + 0.00635, off(m, r + 0.00635));
                    }
                }
                for s in it {
                    let q: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if q.len() == 4 {
                        eprintln!("wheel ground: given {:?} R {:.4} bottom off {:?}", Vec3::new(q[0], q[1], q[2]), q[3], off(Vec3::new(q[0], q[1], q[2]), q[3]));
                    }
                }
            }
        }
        let n = self.cars.len();
        // AI inputs (with simple car-ahead avoidance)
        let positions: Vec<(Vec3, Vec3, f32)> = self.cars.iter().map(|c| (c.state.pos, c.state.forward(), c.state.speed())).collect();
        for i in 0..n {
            // (traffic has no pedals: on its lane it rides, physical it rolls to rest -
            // traffic.rs; the original's physical ambient car's brakes are not decoded)
            if let Some(r) = self.cars[i].rail.as_ref() {
                if r.physical {
                    self.cars[i].input = CarInput { brake: 1.0, handbrake: true, ai: true, ..Default::default() };
                }
                continue;
            }
            let (pos, fwd, _) = positions[i];
            let st = self.cars[i].state.clone();
            let mut input = if let Some(b) = self.cars[i].brain.as_ref() {
                // (a scene's car: its animated brain's pedals and steering, cinecar.rs)
                crate::cinecar::solve(b, &st, self.cars[i].params().veh.as_deref())
            } else if self.cars[i].parked.is_some() {
                CarInput { brake: 1.0, handbrake: true, ..Default::default() }
            } else if self.frozen {
                if let Some(ai) = self.cars[i].ai.as_mut() {
                    ai.stuck_timer = 0.0;
                    ai.reverse_timer = 0.0;
                }
                CarInput { brake: 1.0, ..Default::default() }
            } else if let (Some(_), false, false) = (self.grid_hold, self.cars[i].is_player, self.cars[i].pinned) {
                // (the rest of the world - traffic, police - stands while the racers wait for GO, as the
                // countdown held it before)
                if let Some(ai) = self.cars[i].ai.as_mut() {
                    ai.stuck_timer = 0.0;
                    ai.reverse_timer = 0.0;
                }
                CarInput { brake: 1.0, ..Default::default() }
            } else if let (Some(_), true) = (self.grid_hold, self.grid_wait) {
                CarInput { hold: true, ..Default::default() }
            } else if let (Some(to_go), false, true) = (self.grid_hold, self.cars[i].is_player, self.cars[i].ai.as_ref().map_or(false, |a| a.brain.is_some())) {
                // (a racer the original's brain drives: its revving and the grid burnout - brain.rs)
                crate::brain::tick(self, i, Some(to_go), dt)
            } else if let (Some(to_go), false) = (self.grid_hold, self.cars[i].is_player) {
                // (on the grid: a computer driver's pre-race antics - a burst at placement, revving
                // against the brake, burnouts, the one it holds to GO)
                let bike = self.cars[i].model.bike;
                match self.cars[i].ai.as_mut() {
                    Some(ai) => {
                        ai.stuck_timer = 0.0;
                        ai.reverse_timer = 0.0;
                        let input = ai.grid_input(to_go, dt, bike);
                        // (the end of the opening burst: the drivetrain's reset, vehsim.rs `grid_reset`)
                        if ai.grid.as_mut().map_or(false, |g| std::mem::take(&mut g.reset)) {
                            if let Some(vs) = self.cars[i].state.veh.as_mut() {
                                vs.grid_reset = true;
                            }
                        }
                        input
                    }
                    None => CarInput { brake: 1.0, ..Default::default() },
                }
            } else if self.cars[i].is_player {
                // (on the grid the player's own pedals go through under the hold: `CarInput::hold`)
                // (the steering is written 0 on the grid, 0x8232EC8C - only there)
                let hold = self.grid_hold.is_some();
                CarInput { hold, steer: if hold { 0.0 } else { player_input.steer }, ..player_input }
            } else if self.cars[i].ai.as_ref().map_or(false, |a| a.brain.is_some()) {
                // (a racer: the original's brain, aiv race line and mcOpponentInput - brain.rs)
                crate::brain::tick(self, i, None, dt)
            } else if let (Some(nav), true) = (world.nav.as_ref(), self.cars[i].ai.is_some()) {
                // (the driver knows its car from the game's calibration tables, at this car's state of tune)
                if self.cars[i].ai.as_ref().map_or(false, |a| a.tune.is_none()) && self.cars[i].state.veh.is_some() {
                    if let Some(t) = self.cars[i].model.aitunes.clone() {
                        let perf = self.cars[i].perf;
                        self.cars[i].ai.as_mut().unwrap().tune = Some(Arc::new(mcla_formats::rage::aitune::AiTune::lerp(&t.base, &t.mods, perf)));
                    }
                }
                self.cars[i].ai.as_mut().unwrap().drive(nav, &st, dt)
            } else {
                CarInput::default()
            };
            let pursuing = self.cars[i].cop && self.cars[i].ai.as_ref().map_or(false, |a| a.chase.is_some());
            // Racers do not queue behind traffic: they pull out and pass, on whichever side
            // is free of cars and walls.
            let racing = ((!self.cars[i].is_player && self.cars[i].pinned) || (self.cars[i].is_player && self.auto_player)) && self.cars[i].parked.is_none() && !self.frozen && self.grid_hold.is_none() && !self.cars[i].driver.starts_with(crate::racescript::TAG) && self.cars[i].ai.as_ref().map_or(false, |a| a.path.is_some());
            let mut passing = false;
            if racing {
                let v = positions[i].2;
                let right = Vec3::new(-fwd.z, 0.0, fwd.x).normalize_or(Vec3::X);
                let reach = 12.0 + v * 1.3;
                // the nearest thing in the way: a slower (or standing) car, a trunk, a mast
                // (how far ahead, how far to the side, and the room it needs)
                let mut block: Option<(f32, f32, f32)> = None;
                // (is what blocks standing still? then cars moving alongside do not close the gap)
                let mut standing = false;
                for j in 0..n {
                    if i == j {
                        continue;
                    }
                    let rel = positions[j].0 - pos;
                    if rel.y.abs() > 4.0 {
                        continue;
                    }
                    let ahead = rel.dot(fwd);
                    let side = rel.dot(right);
                    let closing = v - positions[j].1.dot(fwd) * positions[j].2;
                    if ahead > 1.5 && ahead < reach.max(if positions[j].2 < 1.0 { 30.0 } else { 0.0 }) && side.abs() < 2.7 && (closing > 1.5 || positions[j].2 < 2.5) && block.map_or(true, |b| ahead < b.0) {
                        block = Some((ahead, side, 3.3));
                        standing = positions[j].2 < 1.0;
                    }
                }
                for (c, rad) in self.solids.iter() {
                    let rel = *c - pos;
                    if rel.y.abs() > 5.0 {
                        continue;
                    }
                    let ahead = rel.dot(fwd);
                    let side = rel.dot(right);
                    if ahead > 0.5 && ahead < reach && side.abs() < 1.5 + rad && block.map_or(true, |b| ahead < b.0) {
                        block = Some((ahead, side, 2.2 + rad));
                        standing = true;
                    }
                }
                let cur = self.cars[i].ai.as_ref().map_or(0.0, |a| a.dodge);
                let mut want = 0.0;
                if let Some((ahead, side, room)) = block {
                    // both ways round, the nearer one first
                    let first = if side > 0.0 { side - room } else { side + room };
                    let second = if side > 0.0 { side + room } else { side - room };
                    let solids = &self.solids;
                    let free = |d: f32| -> bool {
                        // no car in that slot, nothing solid standing in it ...
                        let taken = (0..n).any(|k| {
                            if k == i {
                                return false;
                            }
                            let rel = positions[k].0 - pos;
                            let a = rel.dot(fwd);
                            // (round something that stands, a car running alongside only counts
                            // while it is right beside)
                            if standing && positions[k].2 > 3.0 && !(a > -4.5 && a < 5.5) {
                                return false;
                            }
                            rel.y.abs() < 4.0 && a > -3.0 && a < ahead + 14.0 && (rel.dot(right) - d).abs() < 2.1
                        }) || solids.iter().any(|(c, rad)| {
                            let rel = *c - pos;
                            let a = rel.dot(fwd);
                            rel.y.abs() < 5.0 && a > -1.0 && a < ahead + 10.0 && (rel.dot(right) - d).abs() < 1.3 + rad
                        });
                        // ... and no wall beside the road
                        let from = pos + Vec3::Y * 0.7 + fwd * ahead.min(9.0);
                        !taken && world.collider.raycast(from, right * d.signum(), d.abs() + 1.6).is_none() && world.collider.raycast(pos + Vec3::Y * 0.7, right * d.signum(), d.abs() + 1.2).is_none()
                    };
                    // one lane over either way, then two (a queue fills both lanes at a red light)
                    for d in [first, second, first + 3.3 * first.signum(), second + 3.3 * second.signum()] {
                        if free(d) {
                            want = d;
                            passing = true;
                            break;
                        }
                    }
                    if !passing && cur.abs() > 0.5 && free(cur) {
                        want = cur;
                        passing = true;
                    }
                    // too close to steer round at speed: lift (from a crawl just turn out)
                    if passing && ahead < 5.0 && (side - cur).abs() < 1.9 && v > 4.0 {
                        input.throttle = input.throttle.min(0.2);
                        input.brake = input.brake.max(0.4);
                    } else if passing && v < 4.0 && self.cars[i].ai.as_ref().map_or(true, |a| a.reverse_timer <= 0.0) {
                        // (unless the driver is backing off something he ran into)
                        input.throttle = input.throttle.max(0.5);
                        input.brake = 0.0;
                        input.handbrake = false;
                    }
                }
                if let Some(ai) = self.cars[i].ai.as_mut() {
                    let rate = if want.abs() > 0.1 { 7.0 } else { 2.5 } * dt;
                    ai.dodge += (want - ai.dodge).clamp(-rate, rate);
                }
            }
            let mut boxed_in = false;
            // (not on the grid: the field stands nose to tail there and the antics are the brain's own)
            if !self.cars[i].is_player && self.cars[i].parked.is_none() && !pursuing && !passing && self.grid_hold.is_none() {
                // brake for cars ahead
                for j in 0..n {
                    if i == j {
                        continue;
                    }
                    let rel = positions[j].0 - pos;
                    let ahead = rel.dot(fwd);
                    let side = (rel - fwd * ahead).length();
                    if ahead > 0.0 && ahead < 18.0 && side < 2.5 {
                        let closing = positions[i].2 - positions[j].2;
                        if ahead < 8.0 || closing > 2.0 {
                            input.throttle = 0.0;
                            input.brake = if ahead < 7.0 { 1.0 } else { 0.5 };
                            if positions[i].2 < 0.8 {
                                // queued up: stand still rather than back away
                                input.brake = 0.0;
                                input.handbrake = true;
                                boxed_in = true;
                            }
                        }
                    }
                }
            }
            // a racer does not queue: boxed in behind something that stands, it backs off after a
            // moment and comes round again with room to steer
            if racing {
                if let Some(ai) = self.cars[i].ai.as_mut() {
                    if boxed_in && ai.reverse_timer <= 0.0 {
                        ai.blocked += dt;
                        if ai.blocked > 1.2 {
                            ai.blocked = 0.0;
                            ai.reverse_timer = 1.4;
                        }
                    } else if !boxed_in {
                        ai.blocked = 0.0;
                    }
                    if ai.reverse_timer > 0.0 {
                        // (the driver's own backing off rules while it lasts)
                        let back = ai.reverse_timer;
                        input.handbrake = false;
                        input.throttle = 0.0;
                        input.brake = 1.0;
                        let _ = back;
                    }
                }
            }
            if std::env::var("MCLA_SIGNAL_TRACE").is_ok() && !self.cars[i].is_player && self.cars[i].parked.is_none() {
                // follow the first car that comes to rest at a red light
                let a = self.signals.ahead(pos, fwd, self.clock);
                if self.trace_car.is_none() && positions[i].2 < 0.3 && a.map_or(false, |x| x.2 == 0 && x.1 < 4.0) {
                    self.trace_car = Some((i, 0));
                }
                if let Some((ti, n)) = self.trace_car {
                    if ti == i && n < 1500 {
                        if n % 30 == 0 {
                            log::info!("trace {}: speed {:.1} signal {:?} throttle {:.2} brake {:.2}", n, positions[i].2, a.map(|x| (x.1, x.2)), input.throttle, input.brake);
                        }
                        self.trace_car = Some((ti, n + 1));
                    }
                }
            }
            // (free cars without a lane read no signal in the original: racers, hookmen, police and other brains never call the junction functions - docs signal_box_rule.md section 4)
            if self.cars[i].stunned > 0.0 {
                self.cars[i].stunned -= dt;
                input = CarInput { brake: 0.15, ..Default::default() };
            }
            // (no nitrous on the grid: a race script's boost waits for GO)
            if self.cars[i].boost > 0.0 && !self.cars[i].is_player && self.grid_hold.is_none() {
                self.cars[i].boost -= dt;
                if input.throttle > 0.3 {
                    input.nitro = true;
                    let st = &mut self.cars[i].state;
                    st.nitro_shots = st.nitro_shots.max(1);
                }
            }
            // (step 19e: a power-up's hold on the car - Ice, Reverse Steering, Stop)
            crate::powerups::apply_ctl(&mut input, self.cars[i].pu_ctl);
            self.cars[i].input = CarInput { ai: input.ai || !self.cars[i].is_player, ..input };
        }
        if let Some(nav) = world.nav.as_ref() {
            self.ride_traffic(nav, &world.collider, dt.min(1.0 / 30.0));
        }
        self.draft_pass(dt.min(1.0 / 30.0));
        // ---- one physics level for every car (physics/loose.rs): the loose props and pieces once a tick
        // before and after the cars, each car's step with the cars not yet stepped as bodies of its island
        // (each pair is solved in the earlier car's step) ----
        let shared = world.collider.loose.clone().filter(|_| std::env::var_os("MCLA_DBG_NOSHARED").is_none());
        let step_dt = dt.min(1.0 / 30.0);
        if let Some(lw) = shared.as_deref() {
            self.level.live = true;
            self.ambient_into_level(step_dt);
            self.level.pre_collide(lw, step_dt);
        }
        let mut level_car: Option<usize> = None;
        let mut level_first = true;
        for idx in 0..self.cars.len() {
            if shared.is_some() {
                self.level_swap(&mut level_car, Some(idx), step_dt, level_first, true);
                level_first = false;
            }
            let c = &mut self.cars[idx];
            // (traffic: the physics level's - inactive while it rides, an ambient collider while
            // physical; loose.rs `Amb`)
            if c.rail.as_ref().is_some_and(|r| !r.physical || r.in_level) {
                continue;
            }
            // a parked car: on the ground by the spawn probe once its ground is here (sub_826E6380 /
            // sub_826E5160: +-1.75 m round the spot, the place the hit, up its normal, the forward made
            // square to it; no hit: no car), then the physics level's (loose.rs `Amb`)
            if c.parked.is_some() && (c.park_held || c.settle == PARK_PROBE || c.settle == PARK_NO_GROUND) {
                if c.settle == PARK_PROBE && world.ready_at(c.state.pos) {
                    match traffic::probe_ground(&world.collider, c.state.pos) {
                        Some((at, up)) => {
                            let f = c.state.forward();
                            let right = f.cross(up).normalize_or(Vec3::X);
                            let f = up.cross(right).normalize_or(f);
                            c.state.relocate(at, Quat::from_mat3(&glam::Mat3::from_cols(right, up, -f)).normalize());
                            c.state.vel = Vec3::ZERO;
                            c.state.ang_vel = Vec3::ZERO;
                            c.pose_history.reset();
                            c.placed = true;
                            c.settle = 0.0;
                        }
                        None => {
                            c.placed = false;
                            c.settle = PARK_NO_GROUND;
                        }
                    }
                }
                continue;
            }
            if let Some((p, yaw)) = c.ai.as_mut().and_then(|a| a.reset_to.take()) {
                // (the car holds the shared level while it steps - `level_swap`: it goes back first,
                // not away with the old state; the reset car's own pairs with it are dropped)
                if level_car == Some(idx) {
                    if let Some(v) = c.state.veh.as_deref_mut() {
                        let mut l = std::mem::take(&mut v.loose);
                        l.take_car();
                        self.level = l;
                    }
                    level_car = None;
                }
                c.state = CarState::new(p, yaw);
                c.pose_history.reset();
                c.placed = false;
            }
            // Already placed cars can outlive their streamed collision sectors.
            // Suspend physical motion until they can be placed on resident ground
            // again. The existing distant-racer path still advances kinematically.
            if c.placed && !world.ready_at(c.state.pos) {
                c.placed = false;
                c.pose_history.reset();
            }
            // Racers outside the streamed-in part of the city have no ground to drive on:
            // move them along the race line until the world around them is loaded.
            // (not on the grid: there the cars stand on their brakes while the world streams in)
            if !c.is_player && !self.frozen && self.grid_hold.is_none() && c.parked.is_none() && c.rail.is_none() && !world.ready_at(c.state.pos) {
                if let Some(ai) = c.ai.as_mut() {
                    if ai.path.is_some() {
                        let v = ai.target_speed * 0.8;
                        if let Some(p) = ai.path_ahead(c.state.pos, v * dt) {
                            let d = p - c.state.pos;
                            if d.length() > 1e-3 {
                                let dir = d.normalize();
                                let gear = c.state.gear;
                                if level_car == Some(idx) {
                                    if let Some(v) = c.state.veh.as_deref_mut() {
                                        let mut l = std::mem::take(&mut v.loose);
                                        l.take_car();
                                        self.level = l;
                                    }
                                    level_car = None;
                                }
                                c.state = CarState::new(p, (-dir.x).atan2(-dir.z));
                                c.pose_history.reset();
                                c.state.vel = dir * v;
                                c.state.gear = gear;
                            }
                            ai.stuck_long = 0.0;
                            ai.stuck_anchor = p;
                        }
                        c.placed = false;
                        continue;
                    }
                }
            }
            if !c.placed {
                c.pose_history.reset();
                if world.ready_at(c.state.pos) {
                    // (the player's car, where roads are stacked: the first surface under the place asked for -
                    // a tunnel's road, not the street above it; with nothing under it, the drop from above as before.
                    // A set-up's standing car too (`hangout_scene::stand_car`): its place is the set-up's, at ground
                    // height - the payback client's car stands under Classic Autos' canopy, not on it)
                    let near = if c.is_player || c.driver.starts_with("hangout:") { world.collider.ground(c.state.pos, 2.0, 200.0) } else { None };
                    if let Some(g) = near.or_else(|| world.collider.ground(c.state.pos + Vec3::Y * 2.0, 50.0, 200.0)) {
                        // straight onto its springs (the model's origin is at road level), tilted with the road
                        let fwd = c.state.forward();
                        let n = g.normal;
                        if n.y > 0.8 {
                            let f = (fwd - n * fwd.dot(n)).normalize_or(fwd);
                            let right = f.cross(n).normalize_or(Vec3::X);
                            c.state.rot = Quat::from_mat3(&glam::Mat3::from_cols(right, n, -f)).normalize();
                        }
                        c.state.pos = g.point + Vec3::Y * 0.03;
                        c.state.vel.y = 0.0;
                    }
                    c.placed = true;
                } else {
                    continue;
                }
            }
            if c.parked.is_some() {
                // let the suspension settle, then leave the car where it stands
                if c.settle > 0.0 {
                    c.settle -= dt;
                    c.state.step(c.tuned.as_deref().unwrap_or(&c.model.params), CarInput { brake: 1.0, handbrake: true, ..Default::default() }, &world.collider, dt.min(1.0 / 30.0));
                    c.state.vel.x = 0.0;
                    c.state.vel.z = 0.0;
                    c.state.ang_vel.y = 0.0;
                } else {
                    c.state.vel = Vec3::ZERO;
                    c.state.ang_vel = Vec3::ZERO;
                }
                continue;
            }
            if self.frozen || (self.grid_hold.is_some() && !c.is_player && !c.pinned) {
                // settle onto the ground but do not drive
                c.state.step(c.tuned.as_deref().unwrap_or(&c.model.params), CarInput { brake: 1.0, ..Default::default() }, &world.collider, dt.min(1.0 / 30.0));
                c.state.vel.x = 0.0;
                c.state.vel.z = 0.0;
                c.state.ang_vel = Vec3::ZERO;
                continue;
            }
            c.state.step(c.tuned.as_deref().unwrap_or(&c.model.params), c.input, &world.collider, dt.min(1.0 / 30.0));
            // (N-VP-1 test hook: MCLA_GRID_TRACE=1 logs the racers through the grid hold and 10 s after it)
            if (c.pinned || c.is_player) && std::env::var("MCLA_GRID_TRACE").is_ok() {
                use std::sync::atomic::{AtomicU32, Ordering};
                static LEFT: AtomicU32 = AtomicU32::new(0);
                if self.grid_hold.is_some() {
                    LEFT.store(600, Ordering::Relaxed);
                }
                let left = LEFT.load(Ordering::Relaxed);
                if left > 0 {
                    if c.is_player && self.grid_hold.is_none() {
                        LEFT.store(left - 1, Ordering::Relaxed);
                    }
                    if let Some(vs) = c.state.veh.as_ref() {
                        let f = c.state.forward();
                        log::info!("grid {:<22} to_go {:6.2} thr {:.2} brk {:.2} hand {} st {} charge {:.2} rpm {:5.0} gear {} spd {:6.2} rear slide {:.2} {:.2} vlong {:6.1} {:6.1} front slide {:.2} {:.2} vlong {:6.1} pos {:.3} {:.3} nitro {} rspin {:.1} {:.1} shaft {:.1} {:.1} drive {} clutch {} held {} | F load {:.0} flong {:.0} disp {:.4} slide {:.2} | R load {:.0} flong {:.0} disp {:.4} slide {:.2} | v {:.3}",
                            if c.is_player { "player" } else { c.model.name.as_str() }, self.grid_hold.unwrap_or(0.0), c.input.throttle, c.input.brake, c.input.handbrake as u8, vs.burn_state, vs.burn, c.state.rpm, vs.gear, c.state.vel.dot(f),
                            vs.wheels[2].slide, vs.wheels[3].slide, vs.wheels[2].velocity_long, vs.wheels[3].velocity_long, vs.wheels[0].slide, vs.wheels[1].slide, vs.wheels[0].velocity_long, c.state.pos.x, c.state.pos.z, c.input.nitro as u8, vs.wheels[2].spin_rate, vs.wheels[3].spin_rate, vs.omega[0], vs.omega[1], vs.drive, vs.clutch as u8, vs.trans_inhibited as u8, vs.wheels[0].load, vs.wheels[0].f_long, vs.wheels[0].disp_long, vs.wheels[0].slide, vs.wheels[2].load, vs.wheels[2].f_long, vs.wheels[2].disp_long, vs.wheels[2].slide, c.state.vel.length());
                    }
                }
            }
        }
        if let Some(lw) = shared.as_deref() {
            self.level_swap(&mut level_car, None, step_dt, false, true);
            self.level.step_rest_alone(lw, step_dt);
            self.level.end_tick_with(lw, step_dt, Some(&world.collider));
            self.ambient_out_of_level(step_dt, world);
        }
        // car-car separation (soft spheres): the port's own rule, now only for a pair the shared level does
        // not take (a car without the original's body: no hull, no simulation tuning; MCLA_DBG_NOSHARED)
        for i in 0..n {
            for j in (i + 1)..n {
                let r = 2.2;
                let d = self.cars[j].state.pos - self.cars[i].state.pos;
                let dist = d.length();
                if dist < r && dist > 1e-3 {
                    let nrm = d / dist;
                    // traffic riding its lane: two of them never meet (each keeps its gap); met
                    // by anything else but a parked car it turns physical (traffic.rs)
                    let riding = |c: &CarInstance| c.rail.as_ref().is_some_and(|r| !r.physical);
                    let (ki, kj) = (riding(&self.cars[i]), riding(&self.cars[j]));
                    if (ki && kj) || ((ki || kj) && (self.cars[i].parked.is_some() || self.cars[j].parked.is_some())) {
                        continue;
                    }
                    // (traffic the physics level holds: its pairs are the level's - loose.rs `Amb`)
                    let held = |c: &CarInstance| c.park_held || c.rail.as_ref().is_some_and(|r| r.in_level);
                    if shared.is_some() && (held(&self.cars[i]) || held(&self.cars[j])) {
                        continue;
                    }
                    for (k, kin) in [(i, ki), (j, kj)] {
                        if kin {
                            self.let_go(k, world);
                        }
                    }
                    let (pi, pj) = (self.cars[i].parked.is_some(), self.cars[j].parked.is_some());
                    if pi && pj {
                        continue;
                    }
                    // (step 19f: MegaStealth - neither meets the other)
                    if self.cars[i].pu_ghost || self.cars[j].pu_ghost {
                        continue;
                    }
                    // (both in the shared level: their pair is the solver's, in the earlier one's step)
                    let in_level = |c: &CarInstance| c.state.veh.is_some() && c.tuned.as_deref().unwrap_or(&c.model.params).hull.is_some();
                    let solved = shared.is_some() && in_level(&self.cars[i]) && in_level(&self.cars[j]);
                    if pi || pj {
                        if solved {
                            continue;
                        }
                        // a parked car does not move: the other one takes the whole correction
                        let (mov, sign) = if pi { (j, 1.0) } else { (i, -1.0) };
                        self.cars[mov].state.correct_position(nrm * (r - dist) * sign);
                        let vn = self.cars[mov].state.vel.dot(nrm) * sign;
                        if vn < 0.0 {
                            self.cars[mov].state.vel -= nrm * sign * vn * 1.3;
                            // (mcCarDamage's contact callback, world_damage_rule.md: the change of
                            // velocity the touch gave it, whole - a parked car is not fixed - times
                            // the parked car's "damage inflicted" factor)
                            let p = self.cars[mov].tuned.as_deref().unwrap_or(&self.cars[mov].model.params).clone();
                            let other = if pi { i } else { j };
                            let at = (self.cars[mov].state.pos + self.cars[other].state.pos) * 0.5 + Vec3::Y * 0.5;
                            let mult = self.cars[other].inflict;
                            self.cars[mov].state.contact_hit(&p, -vn * 1.3, mult, at);
                        }
                        continue;
                    }
                    // Agro: the player is an unstoppable ram
                    let (ai, aj) = (self.agro && self.cars[i].is_player, self.agro && self.cars[j].is_player);
                    if ai || aj {
                        let (ram, hit, sign) = if ai { (i, j, 1.0) } else { (j, i, -1.0) };
                        let rv = self.cars[ram].state.vel;
                        self.cars[hit].state.correct_position(nrm * sign * (r - dist));
                        let closing = (rv - self.cars[hit].state.vel).dot(nrm * sign).max(0.0);
                        self.cars[hit].state.vel += nrm * sign * (closing * 1.4 + 4.0) + Vec3::Y * (1.5 + closing * 0.08);
                        self.cars[hit].state.ang_vel += Vec3::new(0.0, 2.5 * sign, 1.0);
                        self.cars[hit].state.damage = (self.cars[hit].state.damage + 0.2).min(1.0);
                        self.cars[hit].stunned = self.cars[hit].stunned.max(2.0);
                        continue;
                    }
                    if solved {
                        continue;
                    }
                    let push = (r - dist) * 0.5;
                    self.cars[i].state.correct_position(-nrm * push);
                    self.cars[j].state.correct_position(nrm * push);
                    let vi = self.cars[i].state.vel.dot(nrm);
                    let vj = self.cars[j].state.vel.dot(nrm);
                    // (the collision audio's view of the touch, for each car)
                    let at = (self.cars[i].state.pos + self.cars[j].state.pos) * 0.5 + Vec3::Y * 0.5;
                    let rel = self.cars[i].state.vel - self.cars[j].state.vel;
                    // (a computer car's side makes no sound for a touch with the player's car: 0x82382174)
                    for (k, o, n, v) in [(i, j, nrm, rel), (j, i, -nrm, -rel)] {
                        if self.cars[o].is_player {
                            continue;
                        }
                        let st = &mut self.cars[k].state;
                        st.touch_calls += 1;
                        let side = crate::physics::collision::Surface::CarSide;
                        st.touches.push(crate::physics::car::AudioTouch { call: st.touch_calls, pos: at, normal: n, vel: v, own: side, other: side, vehicle: true });
                    }
                    if vi - vj > 0.0 {
                        // (the closing speed taken out between the two by their masses, as a solved
                        // contact without bounce does: each car's change is the impulse over its own mass)
                        let mass = |c: &CarInstance| c.tuned.as_deref().unwrap_or(&c.model.params).mass.max(1.0);
                        let (mi, mj) = (mass(&self.cars[i]), mass(&self.cars[j]));
                        let (dvi, dvj) = ((vi - vj) * mj / (mi + mj), (vi - vj) * mi / (mi + mj));
                        self.cars[i].state.vel -= nrm * dvi;
                        self.cars[j].state.vel += nrm * dvj;
                        // (mcCarDamage's contact callback for each, world_damage_rule.md: its own change
                        // of velocity, whole - another car is not fixed - over 0.1 m/s to the zones by
                        // clamp((sqrt dv - 0.5) x 0.2), and dv times the other's "damage inflicted"
                        // factor, mcCarDamage +20 (0x82342EA8; the payback sets the player's to 3, the
                        // client's to 0.65 and, once he runs, 0.55), to the points above the
                        // ImpactThreshold (0x82343378))
                        let at = (self.cars[i].state.pos + self.cars[j].state.pos) * 0.5 + Vec3::Y * 0.5;
                        for (k, o, dv) in [(i, j, dvi), (j, i, dvj)] {
                            let mut p = self.cars[k].tuned.as_deref().unwrap_or(&self.cars[k].model.params).clone();
                            // (the payback's client: Racer_SetFatalBlow(client,
                            // Racer_GetImpactThreshold(client)) - any blow that counts can finish him)
                            if self.fragile == Some(k) {
                                p.dmg[3] = p.dmg[2];
                            }
                            let mult = self.cars[o].inflict;
                            self.cars[k].state.contact_hit(&p, dv, mult, at);
                        }
                    }
                }
            }
        }

        // ---- the moving damage parts (sub_823424F0: every car, every frame, after its impacts are queued) ----
        let ground = |p: Vec3| world.collider.ground(p, 1.0, 3.0).map(|h| (h.point.y, h.normal));
        for c in self.cars.iter_mut() {
            let st = &mut c.state;
            if !st.parts.rigged {
                st.parts.rig(&c.model.part_rig);
            }
            let cm = Mat4::from_rotation_translation(st.rot, st.pos);
            // (the repair, sub_8233E1B0, runs where the game fixes the car: here, a car whose damage is all gone)
            if st.damage == 0.0 && st.zones == [0.0; 4] && st.parts.damaged() {
                st.parts.repair(&cm);
            }
            let (speed, heavy) = (st.speed(), st.damage > 0.66);
            // (a torn-off bumper is a body of the loose-body level, damageparts.rs: the shared one when the cars
            // share one, else the car's own)
            let level = match shared.as_deref() {
                Some(lw) => Some((lw, &mut self.level)),
                None => world.collider.loose.as_deref().zip(st.veh.as_deref_mut().map(|v| &mut v.loose)),
            };
            st.parts.tick(cm, speed, heavy, dt, &ground, level);
            if c.is_player {
                parts_test(st, &cm, self.clock);
            }
        }
        if let Some(lw) = world.collider.loose.as_deref() {
            torn_pieces(&mut self.cars, lw, shared.is_some().then_some(&mut self.level));
        }
    }

    /// The shared physics level from car `cur` (its step done) to car `next`: what `cur`'s solves gave the
    /// other cars goes into their bodies, `cur` keeps its own manifolds; `next` gets its manifolds and the
    /// cars after it as bodies. `first`: `next` is the tick's first car (it makes the loose bodies' own pairs).
    fn level_swap(&mut self, cur: &mut Option<usize>, next: Option<usize>, dt: f32, first: bool, _step: bool) {
        use crate::physics::loose::{static_shape, LiveVeh};
        if let Some(k) = cur.take() {
            let mut answers = Vec::new();
            if let Some(vs) = self.cars.get_mut(k).and_then(|c| c.state.veh.as_deref_mut()) {
                answers = vs.loose.vehicle_answers();
                let part = vs.loose.take_car();
                std::mem::swap(&mut self.level, &mut vs.loose);
                vs.loose.put_car(part);
            }
            for (uid, jf, jt, push, turn) in answers {
                let Some(o) = self.cars.iter_mut().find(|c| c.uid == uid) else { continue };
                let p = o.tuned.as_deref().unwrap_or(&o.model.params).clone();
                if let Some(vt) = p.veh.clone() {
                    crate::physics::vehsim::take_answer(&mut o.state, &p, &vt, jf, jt, push, turn);
                }
            }
        }
        let Some(k) = next else { return };
        let ghost = self.cars[k].pu_ghost;
        if let Some(vs) = self.cars[k].state.veh.as_deref_mut() {
            vs.loose.ghost = ghost;
        }
        // (a car under MegaStealth meets the map and nothing else: it steps without the level)
        if self.cars[k].state.veh.is_none() || ghost {
            return;
        }
        let me = self.cars[k].uid;
        let mut list = Vec::new();
        for (j, o) in self.cars.iter().enumerate() {
            // (a car before this one has stepped: their pair was in its step; traffic the level holds
            // is an object of its own there - loose.rs `Amb`)
            if j <= k || !o.placed || o.pu_ghost || self.cars[k].pu_ghost || o.park_held || o.rail.as_ref().is_some_and(|r| r.in_level) {
                continue;
            }
            let p = o.tuned.as_deref().unwrap_or(&o.model.params);
            let (Some(vt), Some(hull)) = (p.veh.as_ref(), p.hull.as_ref()) else { continue };
            let Some((body, cg)) = crate::physics::vehsim::island_body(&o.state, p, vt, dt) else { continue };
            list.push(LiveVeh { id: o.uid, shape: static_shape(hull), class: if o.rail.is_some() || o.parked.is_some() { 6 } else { 8 }, gravity: 1.0, cg, level: o.uid as i64 - me as i64, body, inflict: o.inflict, parked: o.parked.is_some() });
        }
        let vs = self.cars[k].state.veh.as_deref_mut().unwrap();
        let part = vs.loose.take_car();
        std::mem::swap(&mut self.level, &mut vs.loose);
        vs.loose.put_car(part);
        vs.loose.live = true;
        vs.loose.skip_pre = true;
        vs.loose.skip_end = true;
        vs.loose.later = !first;
        vs.loose.set_vehicles(list);
        *cur = Some(k);
    }

    /// Traffic into the physics level before the tick (loose.rs `Amb`): the cars gone leave it; each
    /// riding (or otherwise kinematic) car's instance where it is this frame, inactive; a car the game
    /// made physical (its footprint probe, a knock) is made active there.
    fn ambient_into_level(&mut self, dt: f32) {
        let Some(root) = traffic::DATA_ROOT.get().cloned() else { return };
        let alive: std::collections::HashSet<u32> = self.cars.iter().filter(|c| c.rail.is_some() || c.park_held).map(|c| c.uid).collect();
        self.level.ambient_keep(&|u| alive.contains(&u));
        // parked cars: the same ambient car with no behaviour, an inactive instance where it stands
        // (sub_826DF700 -> sub_826DF260 -> sub_826EAD00); any contact activates it, it rolls as the
        // passive collider and rests where it stops, still parked (sub_826E68C0: no behaviour, no
        // recovery) - ag13_park_report.txt
        for k in 0..self.cars.len() {
            let c = &self.cars[k];
            if c.parked.is_none() || c.rail.is_some() || !c.placed || c.settle == PARK_PROBE || c.settle == PARK_NO_GROUND {
                continue;
            }
            let Some(ty) = self.amb_types.get(&c.model.name).copied() else { continue };
            let Some(tune) = traffic::amb_tune(&root, &c.model.name, c.model.own_wheels, &ty) else { continue };
            let m = glam::Mat3::from_quat(c.state.rot);
            self.level.ambient_place(c.uid, &tune, [m.x_axis, m.y_axis, m.z_axis, c.state.pos], None, c.inflict, dt);
            let c = &mut self.cars[k];
            if !c.park_held {
                c.park_held = true;
                c.state.vel = Vec3::ZERO;
                c.state.ang_vel = Vec3::ZERO;
            }
        }
        // (a semi's other car: a trailer's cab is the car its id names; a cab's trailer the one that names it)
        let cabs: std::collections::HashMap<u32, u32> = self.cars.iter().filter_map(|c| c.rail.as_ref().filter(|r| r.state != 0).map(|r| (r.rng_id(), c.uid))).collect();
        let trailers: std::collections::HashMap<u32, u32> = self.cars.iter().filter_map(|c| c.rail.as_ref().filter(|r| r.state == 0).and_then(|r| r.partner).map(|p| (p, c.uid))).collect();
        for k in 0..self.cars.len() {
            let c = &self.cars[k];
            let Some(r) = c.rail.as_ref() else { continue };
            let Some(tune) = traffic::amb_tune(&root, &c.model.name, c.model.own_wheels, &r.ty) else {
                self.cars[k].rail.as_mut().unwrap().in_level = false;
                continue;
            };
            let partner = if r.state == 0 { r.partner.and_then(|p| cabs.get(&p)).copied() } else if r.ty.cab { trailers.get(&r.rng_id()).copied() } else { None };
            let m = traffic::instance_matrix(c);
            self.level.ambient_place(c.uid, &tune, m, partner, c.inflict, dt);
            if r.pairs_inactive() {
                self.level.ambient_posed(c.uid);
            }
            let uid = c.uid;
            let r = self.cars[k].rail.as_mut().unwrap();
            r.in_level = true;
            if std::mem::take(&mut r.wake) {
                self.level.ambient_let_go(uid);
            }
        }
    }

    /// After the tick: the level's rest rule, what it made active or put to rest (traffic.rs `let_go`;
    /// state 5), and each active car's pose from its body.
    fn ambient_out_of_level(&mut self, dt: f32, world: &crate::world::World) {
        self.level.ambient_frame(dt);
        // (the post-solve callbacks' side sums, render instance +560)
        for (uid, side, w) in std::mem::take(&mut self.level.amb_dmg) {
            if let Some(c) = self.cars.iter_mut().find(|c| c.uid == uid) {
                c.amb_lamps.dmg[side] += w;
                // (diagnostics MCLA_AMB_DMG: each addition)
                if std::env::var_os("MCLA_AMB_DMG").is_some() {
                    log::info!("amb dmg {} {} side {} + {:.4} -> {:?}", c.model.name, uid, side, w, c.amb_lamps.dmg);
                }
            }
        }
        let ram = std::env::var("MCLA_TRAFFIC_RAM").is_ok();
        for (uid, woke, slept) in self.level.ambient_events() {
            if ram {
                if let Some(c) = self.cars.iter().find(|c| c.uid == uid && c.park_held) {
                    let me = self.cars.iter().find(|c| c.is_player).map(|c| (c.state.pos, c.state.vel.length())).unwrap_or_default();
                    log::info!("traffic level parked {} {} {} at {:?} d {:.2} player v {:.2}", if woke { "active" } else { "rest" }, c.model.name, uid, c.state.pos, c.state.pos.distance(me.0), me.1);
                }
            }
            let Some(k) = self.cars.iter().position(|c| c.uid == uid && c.rail.is_some()) else { continue };
            if ram {
                let me = self.cars.iter().find(|c| c.is_player).map(|c| (c.state.pos, c.state.vel.length())).unwrap_or_default();
                log::info!("traffic level {} {} {} d {:.2} player v {:.2}", if woke { "active" } else { "rest" }, self.cars[k].model.name, uid, self.cars[k].state.pos.distance(me.0), me.1);
            }
            if woke {
                self.let_go(k, world);
                let r = self.cars[k].rail.as_mut().unwrap();
                r.active = true;
                r.wake = false;
            }
            if slept && self.level.ambient_pose(uid).is_none() {
                self.cars[k].rail.as_mut().unwrap().came_to_rest();
            }
        }
        // (test: MCLA_TRAFFIC_RAM - every half second each active semi's hitch gap and the active cars' speeds)
        if ram {
            static T: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if T.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 15 == 0 {
                for c in self.cars.iter().filter(|c| c.rail.as_ref().is_some_and(|r| r.active)) {
                    let r = c.rail.as_ref().unwrap();
                    let gap = r.partner.and_then(|p| self.cars.iter().find(|o| o.rail.as_ref().is_some_and(|x| x.rng_id() == p))).and_then(|o| {
                        let (a, b) = (self.level.ambient_pose(c.uid)?, self.level.ambient_pose(o.uid)?);
                        let x = |m: &[Vec3; 4], p: Vec3| m[3] + m[0] * p.x + m[1] * p.y + m[2] * p.z;
                        let (ha, hb) = if r.ty.trailer { (traffic::TRAILER_HITCH_PUB, traffic::CAB_HITCH_PUB) } else { (traffic::CAB_HITCH_PUB, traffic::TRAILER_HITCH_PUB) };
                        Some(x(&a.0, ha).distance(x(&b.0, hb)))
                    });
                    log::info!("traffic active {} {} v {:.2} w {:.2} up {:.3} hitch gap {:?} spin0 {:.2} lock {} lift {:.3} {:.3} {:.3} {:.3}", c.model.name, c.uid, c.state.vel.length(), c.state.ang_vel.length(), c.state.up().y, gap, c.state.wheels[0].spin, r.lock.drawn(), c.amb_lift[0], c.amb_lift[1], c.amb_lift[2], c.amb_lift[3]);
                }
                let locked: Vec<String> = self.cars.iter().filter(|c| c.rail.as_ref().is_some_and(|r| r.lock.drawn())).map(|c| format!("{} v {:.1}", c.model.name, c.state.vel.length())).collect();
                if !locked.is_empty() {
                    log::info!("traffic locked wheels {:?}", locked);
                }
                for c in self.cars.iter().filter(|c| c.park_held && self.level.ambient_pose(c.uid).is_some()) {
                    log::info!("traffic active parked {} {} v {:.2} w {:.2} up {:.3} spin0 {:.2} steer {:.3} at {:?}", c.model.name, c.uid, c.state.vel.length(), c.state.ang_vel.length(), c.state.up().y, c.state.wheels[0].spin, c.state.wheels[0].steer, c.state.pos);
                }
            }
        }
        let focus = self.cars.iter().find(|c| c.is_player).map_or(Vec3::ZERO, |c| c.state.pos);
        // the drawn wheels' compression (sub_82794FA8, within 30 m horizontally): the collider wheel's
        // d - collider +636 + 24 i + 48 for wheel bone i, a trailer's bones 2.. reading wheel i - 2 -
        // kept between activations; beyond 30 m none
        for c in self.cars.iter_mut() {
            if c.rail.is_none() && !c.park_held {
                continue;
            }
            let near = Vec3::new(c.state.pos.x - focus.x, 0.0, c.state.pos.z - focus.z).length_squared() < 900.0;
            let trailer = c.rail.as_ref().is_some_and(|r| r.ty.trailer);
            c.amb_lift = match (near, self.level.ambient_wheel_d(c.uid)) {
                (true, Some(d)) => std::array::from_fn(|i| if trailer && i >= 2 { d[i - 2] } else { d[i] }),
                _ => [0.0; 6],
            };
        }
        for c in self.cars.iter_mut() {
            if !c.rail.as_ref().is_some_and(|r| r.active) && !c.park_held {
                continue;
            }
            let Some((m, v, w)) = self.level.ambient_pose(c.uid) else {
                if c.park_held {
                    c.state.vel = Vec3::ZERO;
                    c.state.ang_vel = Vec3::ZERO;
                }
                continue;
            };
            let rot = Quat::from_mat3(&glam::Mat3::from_cols(m[0], m[1], m[2])).normalize();
            // (the drawn wheels of a physical car: the same render update as a riding one's -
            // sub_82794FA8 with car +208, the instance position's change over the frame, along the
            // car's forward (the steer is 0 in states 4 / 5), x 0.6 / r, within 75 m)
            let moved = if dt > 0.0 { (m[3] - c.state.pos) / dt } else { Vec3::ZERO };
            let near = Vec3::new(m[3].x - focus.x, 0.0, m[3].z - focus.z).length_squared() < traffic::SPIN_REACH2;
            let wr = c.model.params.wheel_radius.max(0.2);
            let rate = if c.park_held { 0.6 * traffic::PARKED_SPIN } else { 0.6 };
            // (none while the lock holds: traffic.rs `Lock`)
            let locked = c.rail.as_ref().is_some_and(|r| r.lock.drawn());
            let turn = if near && !locked { rate * moved.dot(rot * Vec3::NEG_Z) / wr * dt } else { 0.0 };
            for (i, wh) in c.state.wheels.iter_mut().enumerate() {
                wh.spin = traffic::wrap_spin(wh.spin + turn);
                wh.steer = if c.park_held && i <= 1 { c.park_steer } else { 0.0 };
            }
            c.state.relocate(m[3], rot);
            c.state.vel = v;
            c.state.ang_vel = w;
            c.placed = true;
        }
    }

    /// Put a car somewhere by hand (cutscenes): wheels follow, it stands still.
    pub fn pose_car(&mut self, idx: usize, pos: Vec3, rot: Quat) {
        let c = &mut self.cars[idx];
        c.state.relocate(pos, rot);
        c.state.vel = Vec3::ZERO;
        c.state.ang_vel = Vec3::ZERO;
        c.placed = true;
        c.pose_history.reset();
    }

    pub fn begin_tick(&mut self) {
        for c in &mut self.cars {
            if c.placed { c.pose_history.begin(&c.state, c.lean); }
            else { c.pose_history.reset(); }
        }
    }

    pub fn finish_tick(&mut self, dt: f32) {
        for c in &mut self.cars {
            if c.model.bike {
                if let Some(vs) = c.state.veh.as_ref() { c.lean = vs.lean; }
                else {
                    let lat = c.state.fwd_speed() * c.state.ang_vel.dot(c.state.up());
                    let target = (lat / 9.81).atan().clamp(-0.85, 0.85);
                    c.lean += (target - c.lean) * (dt * 6.0).min(1.0);
                }
            }
            c.pose_history.finish(&c.state, c.lean);
        }
    }

    /// The damage maps (0x82309238, once a frame as the car is drawn): a car made while one of the 8
    /// is free gets it (0x823095A8) and keeps it until it goes; the queued stamps go into it, a car
    /// without one loses them.
    fn damage_maps(&mut self, gpu: &Gpu) {
        let cars = &mut self.cars;
        self.dmg_maps.retain(|m| cars.iter().any(|c| c.uid == m.uid));
        for c in cars.iter_mut() {
            // (a traffic or parked car - its lamp rows its type's - has no map: its programs read the type's
            // `_maxdamage` and the four side sums, render instance +560; docs/LIGHTING.md "Traffic dents")
            if !c.is_player && c.look.is_none() && crate::render::car_lamps::for_car(&c.model.name, &c.vinyl_name).is_some_and(|t| t.ambient) {
                if !c.amb_lamps.dmg_bound {
                    if let Some(l) = c.model.damage_layout.as_ref().filter(|l| l.0 == l.1 && l.0 > 0) {
                        let (_, view) = self.amb_dmg_tex.entry(c.model.name.clone()).or_insert_with(|| {
                            let t = gpu.new_layout_texture(l.0, &l.2);
                            let v = t.create_view(&Default::default());
                            (t, v)
                        });
                        for b in std::iter::once(&mut c.binding).chain(c.window_bindings.iter_mut()).chain(c.part_bindings.iter_mut()) {
                            gpu.object_damage_map(b, Some(view));
                        }
                    }
                    c.amb_lamps.dmg_bound = true;
                }
        // (a car the physics has not run yet - one a story scene brings, whose clock stops the physics - has its
        // hubs at the world's origin: it was drawn without wheels, the tyres somewhere under it. Its wheels go
        // under their mounts at the suspension's rest length. approx.: no static sag)
        let unset = c.state.wheels.iter().all(|w| w.world_pos == Vec3::ZERO);
                c.state.dmg.dents.clear();
        if unset {
            let (mounts, rest) = (c.params().wheel_pos, c.params().susp_rest);
            for (w, a) in c.state.wheels.iter_mut().zip(mounts) {
                w.world_pos = pos + rot * (a - Vec3::Y * rest);
            }
        }
                c.state.dmg.scrapes.clear();
                c.state.dmg.repair = false;
                continue;
            }
            let k = match self.dmg_maps.iter().position(|m| m.uid == c.uid) {
                Some(k) => Some(k),
                None if self.dmg_maps.len() < crate::damagemap::POOL => {
                    let tex = gpu.new_damage_map();
                    let view = tex.create_view(&Default::default());
                    let px = match &c.model.damage_layout {
                        Some(l) => crate::damagemap::MapPixels::with_layout(&l.2, l.0 as usize, l.1 as usize),
                        None => crate::damagemap::MapPixels::new(),
                    };
                    gpu.write_damage_map(&tex, &px.px);
                    for b in std::iter::once(&mut c.binding).chain(c.window_bindings.iter_mut()).chain(c.part_bindings.iter_mut()) {
                        gpu.object_damage_map(b, Some(&view));
                    }
                    self.dmg_maps.push(DmgMap { uid: c.uid, px, tex, _view: view });
                    Some(self.dmg_maps.len() - 1)
                }
                None => None,
            };
            let d = &mut c.state.dmg;
            match k {
                Some(k) => {
                    let m = &mut self.dmg_maps[k];
                    if m.px.apply(d) {
                        gpu.write_damage_map(&m.tex, &m.px.px);
                        // (diagnostics MCLA_DMG_DUMP=file.png: the player's map after each change)
                        if c.is_player {
                            if let Some(path) = std::env::var_os("MCLA_DMG_DUMP") {
                                m.px.dump(&path.to_string_lossy());
                            }
                        }
                    }
                }
                None => {
                    d.dents.clear();
                    d.scrapes.clear();
                    d.repair = false;
                }
            }
        }
    }

    /// Write every car's transform and lamp state to the GPU.
    pub fn upload_uniforms(&mut self, gpu: &Gpu, night: f32) {
        self.damage_maps(gpu);
        let player_lamps = self.player_lamps.clone();
        for c in self.cars.iter_mut() {
            let pose = c.render_pose(self.render_alpha);
            let mut m = Mat4::from_rotation_translation(pose.rot, pose.pos);
            if self.hide_player && c.is_player {
                m = m * Mat4::from_scale(Vec3::ZERO);
            }
            if c.model.bike {
                m = m * Mat4::from_rotation_z(pose.lean);
            }
            // (traffic: the body pitched and rolled by its change of velocity, the wheels not -
            // traffic::Tilt)
            let wheel_m = m;
            if let Some(q) = c.rail.as_ref().and_then(|r| r.tilt.quat()) {
                m = m * Mat4::from_quat(q);
            }
            // lamp state for the light shader: siren phase (0 off, 1 red, 2 blue) + 4 x reversing
            let siren = if c.cop && self.sirens { 1.0 + ((self.clock * 5.0) as i32 % 2) as f32 } else { 0.0 };
            let reversing = if c.state.gear < 0 { 4.0 } else { 0.0 };
            // parked cars are dark, everything else drives with its lights on at night
            let lights = if c.parked.is_some() || self.lights_off { 0.0 } else { night };
            // (the night fill light at the car's place, added to its ambient - fillmap.rs; the
            // player's car is left as it was: the original shows 0.4 of the sum there, unexplained)
            // (pass 9: the player's car takes the sum too, with the recorded factor 0.4 on its whole ambient)
            let car_fill = if c.is_player { crate::fillmap::player_now(m.w_axis.x, m.w_axis.z) } else { crate::fillmap::body_now(m.w_axis.x, m.w_axis.z) };
            // (lighting diagnostics: MCLA_CAR_POSE_DUMP prints the player's body matrix - columns - once, at the 250th update)
            if c.is_player && std::env::var("MCLA_CAR_POSE_DUMP").is_ok() {
                static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 250 {
                    eprintln!("car pose: x {:?} y {:?} z {:?} at {:?}", m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate(), m.w_axis.truncate());
                }
            }
            // (the car's own lamp colours: its set-up's, else its traffic type's, else the Audi R8's - render/car_lamps.rs)
            let mut lamp_rows = (if c.is_player { player_lamps.clone() } else { None }).or_else(|| crate::render::car_lamps::for_car(&c.model.name, &c.vinyl_name)).map_or_else(|| crate::render::car_lamps::R8.uniform(), |t| t.uniform());
            // (a traffic car's lamp programs take each entry's state in the cover row's w - `AmbLamps`)
            // (a set-up car's too - `setup_lamps`, sub_823523C0 - and the two states its own light on the
            // body takes: the brake lamps' table entry 0 in lamp_a[1].w, the head lamps' in lamp_a[2].w)
            let ambient_rows = lamp_rows.0[0][3] > 0.5 && !c.is_player;
            if ambient_rows || c.parked.is_none() {
                for i in 0..crate::render::car_lamps::IDS {
                    lamp_rows.1[i][3] = c.amb_lamps.lens[i];
                }
            }
            if !ambient_rows {
                lamp_rows.0[1][3] = setup_state(c.amb_lamps.s, 0);
                lamp_rows.0[2][3] = c.amb_lamps.s[0];
                // (the shift light's state, lamp id 19 - `setup_lamps`)
                lamp_rows.0[4][3] = c.amb_lamps.shift;
            }
            // (step 19d: a car in a set-up is painted by it, as the player's by its record)
            let s19d_look: (u8, [f32; 4], [f32; 4], [[f32; 4]; 5], [[f32; 4]; 5], [[f32; 4]; 5]) = match (c.is_player, c.look.as_ref()) {
                (true, _) => (self.player_paint_type.min(5), self.player_paint_b, self.player_tint, self.player_interior, self.player_area_a, self.player_area_b),
                (false, Some(l)) => (l.paint_type.min(5), l.color2, l.tint, l.interior, l.area_a, l.area_b),
                // (traffic and parked cars: their type's paint, `iPaintType`)
                (false, None) => (self.amb_types.get(&c.model.name).map_or(3, |t| t.paint_type.min(5)), [0.0; 4], [0.0; 4], [[0.0; 4]; 5], [[0.0; 4]; 5], [[0.0; 4]; 5]),
            };
            // (a body's first dent slot: its four windows' states for the glass program - WindowDmgVal
            // slots 0 - 3, damagemap::CarDamage::window)
            let mut body_slots = [[0.0; 4]; 6];
            body_slots[0] = c.state.dmg.windows();
            // (the fifth: the side windows' clip plane in the world, ClipPlaneEq of sub_82357DB8 - normal the
            // car's up, through its origin + up x h, h = mcCarModelTune fWindowClipPlaneHeight (-1 on a car
            // without it: nothing cut); VS_Glass sends its distance for the windows 2 and 3 and
            // PS_GlassFront drops what is below. The rolled-down glass, moved by its bone, sinks under it.)
            let up = m.y_axis.truncate().normalize_or(Vec3::Y);
            let h = c.model.simtunes.as_ref().and_then(|s| s.window_clip).unwrap_or(-1.0);
            body_slots[4] = up.extend(-up.dot(m.w_axis.truncate() + up * h)).to_array();
            // (traffic and parked cars - their lamp rows marked ambient: sub_827914B8 hands the type's
            // colour C = vColors[idx] x fScale on raised to 2.2 in both paint rows (c124 / c125), and
            // VSAmbientCarPaint paints with sat(4 x lerp(c124, c125, t)) = sat(4 C^2.2); the shader
            // takes that as it is - docs/LIGHTING.md "Traffic paint: the ambient cars' own program")
            let amb_paint = !c.is_player && c.look.is_none() && lamp_rows.0[0][3] > 0.5;
            let body_color = if amb_paint { ambient_paint(c.color) } else { c.color };
            // (their second slot: gCarAmbDamageParams c126, the side sums; enableVertDamage = c126 . c126 > 0)
            if amb_paint {
                body_slots[1] = c.amb_lamps.dmg;
                // (diagnostics MCLA_AMB_DMG_ALL=v: every traffic car's four sums v)
                if let Some(v) = std::env::var("MCLA_AMB_DMG_ALL").ok().and_then(|v| v.parse::<f32>().ok()) {
                    body_slots[1] = [v; 4];
                }
            }
            // (+ 256: the in-car glass, only the player's car - the camera's target, sub_822A9670)
            let int_glass = if self.interior_glass && c.is_player { 256.0 } else { 0.0 };
            let body_u = ObjectUniform { model: m.to_cols_array_2d(), color: body_color, params: [lights, c.input.brake, c.state.damage, siren + reversing + 16.0 + 32.0 * s19d_look.0 as f32 + int_glass], dents: body_slots, color2: s19d_look.1, tint: s19d_look.2, interior: s19d_look.3, fade_min: c.model.min.extend(c.pu_mirror).to_array(), fade_size: (c.model.max - c.model.min).max(Vec3::splat(0.01)).extend(c.amb_lamps.tint).to_array(), fill: car_fill, lamp_a: lamp_rows.0, lamp_b: lamp_rows.1, area_a: s19d_look.4, area_b: s19d_look.5 };
            gpu.update_object(&c.binding, &body_u);
            // (the side windows: the body's object moved by the window bone's clip)
            for (k, wb) in c.window_bindings.iter().enumerate() {
                let Some(clip) = c.model.windows[k].as_ref() else { continue };
                let wm = m * clip.delta(c.windows[k].clip_time(clip.duration));
                gpu.update_object(wb, &ObjectUniform { model: wm.to_cols_array_2d(), ..body_u });
            }
            // (the moving damage parts: hood, trunk lid, rear and front bumper - damageparts.rs)
            for (k, pb) in c.part_bindings.iter().enumerate() {
                let parts = &c.state.parts;
                let pm = match k {
                    0 => c.model.hood_slot.map_or(m, |s| m * parts.hood_delta(&s)),
                    1 => c.model.trunk_bone.map_or(m, |b| m * parts.trunk_delta(&b)),
                    // (a traffic model's own wheel: turned about its hub - steer, then the roll -
                    // with the body untilted; the original's wheel bones, sub_82794FA8)
                    w if w >= 4 => match c.model.own_wheels.and_then(|h| h[w - 4]) {
                        Some(h) => {
                            // (the second rear axle turns with the first)
                            let i = [0, 1, 2, 3, 2, 3][w - 4];
                            let q = Quat::from_rotation_y(-pose.steer[i]) * Quat::from_rotation_x(-pose.spin[i]);
                            // (raised by the collider wheel's compression d within 30 m: the bone's
                            // local [S M | h M + 2 o M - h - o], o = (0, d, 0), M the counter-tilt -
                            // the hub at h + 2 d up - d x the tilted body's up, in the untilted car)
                            let d = c.amb_lift[w - 4];
                            let tilted_up = c.rail.as_ref().and_then(|r| r.tilt.quat()).map_or(Vec3::Y, |t| t * Vec3::Y);
                            let lift = (Vec3::Y * 2.0 - tilted_up) * d;
                            wheel_m * Mat4::from_translation(h + lift) * Mat4::from_quat(q) * Mat4::from_translation(-h)
                        }
                        None => m,
                    },
                    _ => match parts.bumpers[k - 2].as_ref() {
                        // (the fallen piece is drawn where its body is: M_slot' = B * M_rest^-1 of the slot's mesh)
                        Some(b) if b.detached.is_some() && !(self.hide_player && c.is_player) => {
                            let l = b.detached.unwrap();
                            Mat4::from_rotation_translation(l.rot, l.pos) * b.m_rest.inverse()
                        }
                        Some(b) => m * b.draw_delta(),
                        None => m,
                    },
                };
                let mut pu = ObjectUniform { model: pm.to_cols_array_2d(), ..body_u };
                // (a traffic model's own wheel: its rim's and matte's programs dent it too, the vertex taken in its
                // wheel bone's frame - every va_ wheel bone is its hub with no turn or scale, so p - hub; the third
                // slot carries the hub)
                if k >= 4 {
                    if let Some(h) = c.model.own_wheels.and_then(|h| h[k - 4]) {
                        pu.dents[2] = [h.x, h.y, h.z, 1.0];
                    }
                }
                gpu.update_object(pb, &pu);
            }
            for (i, b) in c.wheel_bindings.iter().enumerate() {
                let left = i % 2 == 0;
                let steer = Quat::from_rotation_y(-pose.steer[i]);
                // the rim models face -x (hub cap at x = -0.24): left wheels as modelled, right
                // wheels turned around; rolling forward (-z) is a negative rotation about x
                let spin = Quat::from_rotation_x(if left { -pose.spin[i] } else { pose.spin[i] });
                let flip = if left { Quat::IDENTITY } else { Quat::from_rotation_y(std::f32::consts::PI) };
                let mut local = pose.hubs[i];
                if c.model.bike {
                    // Bike physical wheels and their visual mirrors share the center line.
                    let lo = pose.hubs[i ^ 1];
                    local = (local + lo) * 0.5;
                }
                let flip = if c.model.bike { Quat::IDENTITY } else { flip };
                let spin = if c.model.bike { Quat::from_rotation_x(-pose.spin[i]) } else { spin };
                // (the original draws each axle's own tyre: as wide as fitted, 6.35 mm larger than its physics radius)
                let (tyre_w, tyre_r) = c.params().veh.as_ref().filter(|v| v.tyre_width[0] > 0.0 && !c.model.bike).map(|v| if i < 2 { (v.tyre_width[0], v.t.wheel_front.radius + 0.00635) } else { (v.tyre_width[1], v.t.wheel_back.radius + 0.00635) }).unwrap_or((c.model.params.wheel_width, c.model.params.wheel_radius));
                let wm = wheel_m * Mat4::from_translation(local) * Mat4::from_quat(steer * flip * spin) * Mat4::from_scale(Vec3::new(tyre_w * 2.0, tyre_r * 2.0, tyre_r * 2.0));
                // (a wheel has no dents: the first three slots carry its rim's finish)
                let mut finish = [[0.0; 4]; 6];
                if c.is_player {
                    finish[..3].copy_from_slice(&self.player_rim_finish);
                } else if let Some(l) = c.look.as_ref() {
                    finish[..3].copy_from_slice(&l.rim_finish);
                }
                // (the fourth: the tyre's dirt - TireDirtColor.w, its hub's scrape sum at most 1 - and a
                // mark that this is a wheel)
                finish[3] = [c.state.dmg.rim_scuff(i), 1.0, 0.0, 0.0];
                gpu.update_object(b, &ObjectUniform { model: wm.to_cols_array_2d(), color: [0.8, 0.8, 0.8, 1.0], params: [night, 0.0, 0.0, 0.0], dents: finish, fill: car_fill, ..Default::default() });
                // the caliper: with the wheel but for its turning, on the rear of the disc on
                // both sides (the right ones turned over instead of around)
                if let (Some(cb), true) = (c.caliper_bindings.get(i), c.model.brakes.is_some() || c.brake.is_some()) {
                    let side = if left { Quat::IDENTITY } else { Quat::from_rotation_y(std::f32::consts::PI) * Quat::from_rotation_x(std::f32::consts::PI) };
                    let cm = wheel_m * Mat4::from_translation(local) * Mat4::from_quat(steer * side) * Mat4::from_scale(Vec3::new(tyre_w * 2.0, tyre_r * 2.0, tyre_r * 2.0));
                    gpu.update_object(cb, &ObjectUniform { model: cm.to_cols_array_2d(), color: [0.8, 0.8, 0.8, 1.0], params: [night, 0.0, 0.0, 0.0], fill: car_fill, ..Default::default() });
                }
            }
        }
    }

    /// The glow of every lit lamp near the camera, as the game's glow table rates them
    /// (`tune/vehicle`: head 2, tail 2 - the player's 1 - brake 2 / 0.3, reverse 0.53, third
    /// brake 0.35). A lamp glows towards where it points: head lamps ahead, the others astern.
    pub fn update_glows(&mut self, gpu: &Gpu, night: f32, cam: Vec3) {
        use crate::props::Inst;
        const MAX: usize = 320;
        let mut out: Vec<Inst> = Vec::new();
        for c in &self.cars {
            let pose = c.render_pose(self.render_alpha);
            if pose.pos.distance_squared(cam) > 260.0 * 260.0 || (self.hide_player && c.is_player) {
                continue;
            }
            let lights = if c.parked.is_some() || self.lights_off { 0.0 } else { night };
            let braking = (c.input.brake * 4.0).clamp(0.0, 1.0);
            let siren = if c.cop && self.sirens { 1 + (self.clock * 5.0) as i32 % 2 } else { 0 };
            for (id, local) in &c.model.lamps {
                let (k, col, size) = match *id {
                    3 => (2.0 * lights, [1.0, 0.96, 0.86], 0.85),
                    1 => ((if c.is_player { 1.0 } else { 2.0 }) * lights * 0.22, [1.0, 0.07, 0.04], 0.16),
                    0 => ((if c.is_player { 0.6 } else { 2.0 }) * braking * 0.6, [1.0, 0.07, 0.04], 0.34),
                    12 => (0.35 * braking, [1.0, 0.07, 0.04], 0.3),
                    6 => (0.53 * if c.state.gear < 0 { 1.0 } else { 0.0 }, [1.0, 1.0, 0.95], 0.4),
                    13 => (if siren == 1 { 2.5 } else { 0.0 }, [1.0, 0.08, 0.05], 0.9),
                    14 => (if siren == 2 { 2.5 } else { 0.0 }, [0.1, 0.25, 1.0], 0.9),
                    _ => (0.0, [0.0; 3], 0.0),
                };
                // (head, tail, brake and reversing lamps glow in the pass after the tone map now -
                // `halo_lamps`, halo.rs; MCLA_DEBUG_VIEW=115: these sprites as before)
                if k <= 0.02 || (matches!(*id, 0 | 1 | 3 | 6 | 12 | 13 | 14) && !crate::halo::old_look()) {
                    continue;
                }
                // (the glow takes the lamp's own lit colour - bulb through cover - where the car has a lamp table: render/car_lamps.rs)
                let col = if matches!(*id, 0 | 1 | 3 | 6 | 12) { self.lamp_rows(c).map_or(col, |t| t.lit(*id as usize)) } else { col };
                let p = pose.pos + pose.rot * *local;
                // (police lights show all round)
                let facing = if *id >= 13 {
                    1.0
                } else {
                    let out_dir = if *id == 3 { pose.rot * Vec3::NEG_Z } else { -(pose.rot * Vec3::NEG_Z) };
                    ((cam - p).normalize_or_zero().dot(out_dir) * 1.6).clamp(0.0, 1.0)
                };
                if facing > 0.02 && out.len() < MAX {
                    out.push(Inst { pos_scale: [p.x, p.y, p.z, size], rot_tint: [col[0], col[1], col[2], k * facing], fill: [0; 4], ..Inst::NONE });
                }
            }
        }
        // (step 19e: the power-up sources - `powerups::PowerUps::glows`)
        for g in &self.pu_glows {
            if out.len() < MAX && Vec3::new(g[0], g[1], g[2]).distance_squared(cam) < 260.0 * 260.0 {
                out.push(Inst { pos_scale: [g[0], g[1], g[2], g[3]], rot_tint: [g[4], g[5], g[6], g[7]], fill: [0; 4], ..Inst::NONE });
            }
        }
        self.glow_count = out.len() as u32;
        if out.is_empty() {
            return;
        }
        out.resize(MAX, Inst { pos_scale: [0.0; 4], rot_tint: [0.0; 4], fill: [0; 4], ..Inst::NONE });
        match self.glow_buf.as_ref() {
            Some(b) => gpu.queue.write_buffer(b, 0, bytemuck::cast_slice(&out)),
            None => {
                use wgpu::util::DeviceExt;
                self.glow_buf = Some(gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("lamp glows"), contents: bytemuck::cast_slice(&out), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST }));
            }
        }
    }

    /// Each frame before the cars' uniforms: the traffic cars' lamp states and glow levels
    /// (`AmbLamps`) - for the cars drawn now: in the view and within the lamps' reach along the
    /// camera's axis `axis` from `eye`. The flags as sub_826E8EC8 sets them, from what the port's
    /// traffic has: lights on at night, braking when the pedal is down or the car stands,
    /// reversing; the turn signals are not driven (the port's traffic does not signal; the
    /// original's rule is in the doc). Parked cars have every flag off, as recorded.
    pub fn traffic_lamps(&mut self, eye: Vec3, axis: Vec3, frustum: &crate::render::Frustum, night: f32, dt: f32) {
        let alpha = self.render_alpha;
        let lights_off = self.lights_off;
        for c in self.cars.iter_mut() {
            if c.is_player {
                continue;
            }
            let Some(rows) = crate::render::car_lamps::for_car(&c.model.name, &c.vinyl_name).filter(|t| t.ambient) else { continue };
            let pos = c.render_pose(alpha).pos;
            c.amb_lamps.tint = 0.25 + 1.5 * ((pos.distance(eye) / 30.0).min(1.0) - 0.5).clamp(0.0, 1.0);
            let d = (pos - eye).dot(axis);
            let r = Vec3::splat(c.model.params.body_half.length() + 1.0);
            if d > TRAFFIC_LAMP_REACH || !frustum.intersects_aabb(pos - r, pos + r) {
                continue;
            }
            let driven = c.parked.is_none();
            let flags = [driven && !lights_off && night > 0.5, driven && (c.input.brake > 0.0 || c.state.speed() < 0.1), false, false, driven && c.state.gear < 0];
            let a = &mut c.amb_lamps;
            for (v, on) in a.s.iter_mut().zip(flags) {
                *v = traffic_ease(*v, on, dt);
            }
            let fade = traffic_fade(d);
            let mut lv = [0.0; crate::render::car_lamps::IDS];
            for i in 0..crate::render::car_lamps::IDS {
                let st = traffic_state(a.s, rows.att[i]);
                a.lens[i] = st;
                lv[i] = st * fade * rows.glow_scale;
            }
            a.levels = Some(lv);
            // (diagnostics: MCLA_TLAMP_DUMP prints every drawn traffic car's depth, states and levels)
            if std::env::var("MCLA_TLAMP_DUMP").is_ok() {
                eprintln!("tlamp {} d {:.1} scale {:.3} s {:?} levels {:?}", c.model.name, d, rows.glow_scale, a.s, lv.map(|v| (v * 1e4).round() / 1e4));
            }
        }
    }

    /// Each frame: the set-up cars' (player, rivals, cops) lamp states, entry states and glow levels
    /// as mcCar's update keeps them (sub_823567F8 -> sub_82353E38 eases, sub_823523C0 writes the light
    /// set's levels at +400 and the lamp programs' states), in `AmbLamps`. The targets of
    /// sub_82353E38: head lamps `lights_due` (the hour past the light manager's 18.0 or before its 6.0,
    /// or the Stormy or Foggy weather's weight over 0.85 - the caller's); braking the driver input's
    /// brake (+16: the player's pad output, a computer driver's own) over 0.001; reversing the
    /// gearbox in its first (reverse) gear; in the garage (light manager byte 14546) the head lamps
    /// follow the view instead (the caller's `lights_due`, `garage_lamps_due`). All three need the car's
    /// switch (`AmbLamps::off`, +597) and no Stealth (sub_822A91E0: the power-up's state 3 or 14).
    pub fn setup_lamps(&mut self, lights_due: bool, dt: f32) {
        let lights_off = self.lights_off;
        let sirens = self.sirens;
        let player_rows = self.player_lamps.clone();
        for c in self.cars.iter_mut() {
            let rows = match (&player_rows, c.is_player) {
                (Some(t), true) => Some(t.clone()),
                _ => crate::render::car_lamps::for_car(&c.model.name, &c.vinyl_name),
            };
            if rows.as_ref().is_some_and(|t| t.ambient) || c.parked.is_some() {
                continue;
            }
            let att = rows.map_or(crate::render::car_lamps::R8.att, |t| t.att);
            let brake = match c.state.veh.as_ref() {
                Some(v) if c.is_player && v.pad.live => v.pad.pedals.brake,
                _ => c.input.brake,
            };
            // (all three need the car's switch +597; Stealth / MegaStealth - sub_822A91E0 - puts them out)
            let on = !c.amb_lamps.off && !c.pu_stealth;
            let flags = [on && lights_due && !lights_off, on && brake > 0.001, false, false, on && c.state.gear < 0];
            // (the siren state +300, rate 18: its target +304 is what the car's police tactic writes each update -
            // sub_82711628 -> sub_8270C7E8, 1 in pull-over / pursuit / arrow trap, cleared at the tactic's end; the
            // port's police has one state for all its cars: `sirens` on a cruiser)
            let siren = c.cop && sirens;
            let a = &mut c.amb_lamps;
            for (k, (v, on)) in a.s.iter_mut().zip(flags).enumerate() {
                *v = if k == 2 { ease_at(*v, siren, 18.0, dt) } else { traffic_ease(*v, on, dt) };
            }
            a.siren_on = siren;
            for i in 0..crate::render::car_lamps::IDS {
                a.lens[i] = setup_state(a.s, att[i]);
            }
            a.levels = Some(a.lens);
            // (the 20th entry, the shift light: the engine's revs - sub_825585A0, engine +8 x 9.5493 - at or past
            // the gear's upshift point, the transmission's +76 + 4 gear = `VehTune::shift[g][0]`; reverse and
            // neutral hold 0 there, so it is lit in them. Not switched by +597.)
            let shift_on = match (c.model.params.veh.as_ref(), c.state.veh.as_ref()) {
                (Some(vt), Some(vs)) => vs.gear <= 0 || vt.shift.get(vs.gear as usize).is_some_and(|s| vs.engine_w / 0.104_719_76 >= s[0]),
                _ => false,
            };
            a.shift = traffic_ease(a.shift, shift_on, dt);
        }
    }

    /// The cars' lit lamps for the glow pass after the tone map (render only; halo.rs, the
    /// original's sub_8237AC90): the lamp's place, the way it faces, its lit colour, and its
    /// level x the kind's factor of the executable's two tables (0x827E7688 other cars: brake 2,
    /// tail 2, head 2, reversing 0.528, third brake 0.346; 0x827E76D8 the player's: brake 0.3,
    /// tail 1.004, head 2, the others 1).
    pub fn halo_lamps(&self, night: f32, cam: Vec3) -> Vec<(Vec3, Vec3, [f32; 3], f32)> {
        let mut out = Vec::new();
        let old_rule = std::env::var("MCLA_GLOW_OLD").is_ok();
        for (ci, c) in self.cars.iter().enumerate() {
            let pose = c.render_pose(self.render_alpha);
            if pose.pos.distance_squared(cam) > 260.0 * 260.0 || (self.hide_player && c.is_player) {
                continue;
            }
            let lights = if c.parked.is_some() || self.lights_off { 0.0 } else { night };
            let pl = c.is_player;
            // The original's light set (sub_8237AC90 walks it; the player's read out of the running game,
            // work/lighting/fe_lset.py): one record for every `glow<kind>0` bone of the fitted parts'
            // skeletons (the R8: four glowbrake0, two glowhead0, two glowhigh0, two glowreverse0, one
            // glowthirdbrake0), its facing the bone's z row. Levels of the standing player's car at 23:30:
            // brake 0.25, tail 0.25, signal 1, head 1, high beam 0, fog 1, reversing / third brake 0.
            // Traffic takes its own levels (`AmbLamps`, sub_82792FD8: by table entry, x the glow file's
            // factor of the kind) and only once it has been drawn near; its glows reach as far as the
            // original's test in sub_82790B08: |camera - car|^2 - radius^2 <= 250^2.
            // A car without such bones keeps one glow a side at its lamp meshes' mean.
            let traffic = if !pl && self.lamp_rows(c).is_some_and(|t| t.ambient) {
                let Some(lv) = c.amb_lamps.levels else { continue };
                // (the glow pass's own test: in the view, and drawn or under 50 deep - `pick_lods`)
                if !self.lod_glow.get(ci).copied().unwrap_or(true) {
                    continue;
                }
                let r = c.model.params.body_half.length();
                if pose.pos.distance_squared(cam) - r * r > 250.0 * 250.0 {
                    continue;
                }
                Some(lv)
            } else {
                None
            };
            // (a set-up car: its light set's levels as sub_823523C0 writes them - `setup_lamps` - x the
            // player's or the others' factor)
            let own = if traffic.is_none() && c.parked.is_none() { c.amb_lamps.levels } else { None };
            let level = |id: u8| -> f32 {
                let k = (id as usize).min(crate::render::car_lamps::IDS - 1);
                if let Some(lv) = traffic {
                    return lv[k] * GLOW_FACTOR[k];
                }
                own.map_or(0.0, |lv| lv[k] * if pl { GLOW_FACTOR_PLAYER[k] } else { GLOW_FACTOR[k] })
            };
            // (the glow's colour is bulb x cover cut at 1, not brought to a full channel: the R8's
            // brake row 3.804 3.8 3.4 x 0.9804 0.0314 0 is recorded as 255 30 0)
            let rows = self.lamp_rows(c);
            let colour = |id: u8, dflt: [f32; 3]| -> [f32; 3] {
                rows.as_ref().map_or(dflt, |t| {
                    let i = (id as usize).min(crate::render::car_lamps::IDS - 1);
                    [(t.bulb[i][0] * t.cover[i][0]).min(1.0), (t.bulb[i][1] * t.cover[i][1]).min(1.0), (t.bulb[i][2] * t.cover[i][2]).min(1.0)]
                })
            };
            let dflt = |id: u8| -> [f32; 3] {
                match id {
                    3 | 8 => [1.0, 0.96, 0.86],
                    6 => [1.0, 1.0, 0.95],
                    2 => [1.0, 0.45, 0.0],
                    _ => [1.0, 0.07, 0.04],
                }
            };
            let has_bone = |id: u8| !old_rule && c.model.glow_bones.iter().any(|(k, _, _)| *k == id);
            for (id, local, facing) in &c.model.glow_bones {
                let k = level(*id);
                if k <= 0.0 || old_rule {
                    continue;
                }
                out.push((pose.pos + pose.rot * *local, pose.rot * *facing, colour(*id, dflt(*id)), k));
            }
            for (id, local) in &c.model.lamps {
                if has_bone(*id) || !matches!(*id, 0 | 1 | 3 | 6 | 12) {
                    continue;
                }
                let k = if old_rule && *id == 1 { (if pl { 1.004 } else { 2.0 }) * lights } else { level(*id) };
                if k <= 0.0 {
                    continue;
                }
                let out_dir = if *id == 3 { pose.rot * Vec3::NEG_Z } else { -(pose.rot * Vec3::NEG_Z) };
                // (MCLA_GLOW_OLD: the R8's four brake glows from the recorded table, as until 2026-10-15)
                if old_rule && *id == 0 && c.model.name == "vp_aud_r8_08" {
                    let sx = if local.x < 0.0 { -1.0 } else { 1.0 };
                    out.push((pose.pos + pose.rot * Vec3::new(0.701 * sx, 0.747, 1.803), out_dir, colour(*id, dflt(*id)), k));
                    out.push((pose.pos + pose.rot * Vec3::new(0.526 * sx, 0.773, 1.902), out_dir, colour(*id, dflt(*id)), k));
                    continue;
                }
                out.push((pose.pos + pose.rot * *local, out_dir, colour(*id, dflt(*id)), k));
            }
        }
        out
    }

    /// The police lamps' glows (sub_8235FB78 in the glow pass, not sub_8237AC90): while a car's siren target
    /// (+304) is set, one glow for every lamp of kinds 13 and 14 (the `coplightred0` / `coplightblue0` bones)
    /// with the kind's colour bulb x cover + 0.5 (cut to 1 when packed) and its siren state (+308); halo.rs
    /// `Tune::police` makes the glow. (place, facing, colour, state)
    pub fn police_glows(&self, cam: Vec3) -> Vec<(Vec3, Vec3, [f32; 3], f32)> {
        let mut out = Vec::new();
        for c in &self.cars {
            if !c.amb_lamps.siren_on || c.parked.is_some() {
                continue;
            }
            let pose = c.render_pose(self.render_alpha);
            if pose.pos.distance_squared(cam) > 260.0 * 260.0 {
                continue;
            }
            let rows = self.lamp_rows(c);
            for (id, local, facing) in c.model.glow_bones.iter().filter(|g| g.0 == 13 || g.0 == 14) {
                let i = *id as usize;
                let col = rows.as_ref().map_or([1.0; 3], |t| [0, 1, 2].map(|k| (t.bulb[i][k] * t.cover[i][k] + 0.5).clamp(0.0, 1.0)));
                out.push((pose.pos + pose.rot * *local, pose.rot * *facing, col, c.amb_lamps.s[2]));
            }
        }
        out
    }

    /// Draw the lamp glows (after the opaque scene). Sets its own pipeline.
    pub fn draw_glows<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu) {
        if let (Some(buf), true) = (self.glow_buf.as_ref(), self.glow_count > 0) {
            pass.set_pipeline(&gpu.pipeline_glow);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..4, 0..self.glow_count);
        }
    }

    /// Draw all cars. `blend_pass` selects glass-only.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, blend_pass: bool, draws: &mut usize, range: Option<(Vec3, f32)>, frustum: Option<&crate::render::Frustum>) {
        for (ci, c) in self.cars.iter().enumerate() {
            // (step 19g: a car under Stealth is not drawn in either pass of its own - pu_fx.rs)
            if c.pu_stealth {
                continue;
            }
            let Some((batches, groups)) = self.lod_batches(ci) else { continue };
            let mut wheels_visible = true;
            let pos = c.render_pose(self.render_alpha).pos;
            // (the main view's passes - the ones with a frustum: a traffic car is drawn by its LOD alone, which
            // has its view, occluder and 500 tests; the reach and box below are the port's, for the other cars)
            let by_lod = frustum.is_some() && !c.model.lods[0].0.is_empty();
            if let Some((eye, max)) = range.filter(|_| !by_lod) {
                let d = pos.distance(eye);
                if d > max {
                    continue;
                }
                // wheels are a few pixels beyond this
                wheels_visible = d < max * 0.6;
            }
            if let Some(f) = frustum.filter(|_| !by_lod) {
                let r = Vec3::splat(c.model.params.body_half.length() + 1.0);
                if !f.intersects_aabb(pos - r, pos + r) {
                    continue;
                }
            }
            pass.set_bind_group(3, &c.binding.bg, &[]);
            // (the windows in two passes, as the original: all back faces, then all front faces, with depth
            // writes; the lamps' lenses stay in the one pass. MCLA_GLASS_ONEPASS=1: as before. MCLA_GLASS_PASS=b / f:
            // one of the two only, to look at them)
            let glass2 = blend_pass && glass_two_pass() != 0;
            // (the player's car in the driver's view: interiorwindowglass, drawn twice by the original with the
            // back faces' cull and no depth writes - here once, city.wgsl blends it as the two draws)
            let inner = self.interior_glass && c.is_player;
            if glass2 {
                let passes: &[(u8, &wgpu::RenderPipeline)] = if inner { &[(3u8, &gpu.pipeline_glass_inner)] } else { &[(1u8, &gpu.pipeline_glass_back), (2u8, &gpu.pipeline_glass_front)] };
                for &(side, pl) in passes {
                    if glass_two_pass() & side == 0 {
                        continue;
                    }
                    pass.set_pipeline(pl);
                    for (bi, b) in batches.iter().enumerate() {
                        if b.mode != MatMode::CarGlass || b.shader.to_ascii_lowercase().contains("bumped") {
                            continue;
                        }
                        pass.set_bind_group(3, c.group_object(groups[bi]), &[]);
                        pass.set_bind_group(1, b.bind.as_ref(), &[]);
                        pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                        pass.set_vertex_buffer(0, b.vb.slice(..));
                        pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..b.count, 0, 0..1);
                        *draws += 1;
                    }
                }
                pass.set_pipeline(&gpu.pipeline_blend);
            }
            for (bi, b) in batches.iter().enumerate() {
                if b.mode.blended() != blend_pass {
                    continue;
                }
                if glass2 && b.mode == MatMode::CarGlass && !b.shader.to_ascii_lowercase().contains("bumped") {
                    continue;
                }
                // the car's own plate on the plate mesh
                let bind = match c.plate.as_ref() {
                    Some(p) if b.shader.eq_ignore_ascii_case("LicensePlate") => p,
                    _ => match c.vinyl.as_ref() {
                        Some(v) if b.mode == MatMode::Paint => v,
                        _ => &b.bind,
                    },
                };
                pass.set_bind_group(3, c.group_object(groups[bi]), &[]);
                pass.set_bind_group(1, bind.as_ref(), &[]);
                pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                pass.set_vertex_buffer(0, b.vb.slice(..));
                pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..b.count, 0, 0..1);
                *draws += 1;
            }
            if !blend_pass && wheels_visible && c.model.own_wheels.is_none() {
                // garage choice, else the car's stock wheels, else the generic set
                if let Some(w) = c.wheels.as_ref().or(c.model.wheels.as_ref()).or(self.wheels.as_ref()) {
                    for (wi, wb) in c.wheel_bindings.iter().enumerate() {
                        if c.model.bike && wi % 2 == 1 {
                            continue;
                        }
                        pass.set_bind_group(3, &wb.bg, &[]);
                        for b in &w.batches {
                            pass.set_bind_group(1, b.bind.as_ref(), &[]);
                            pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                            pass.set_vertex_buffer(0, b.vb.slice(..));
                            pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..b.count, 0, 0..1);
                            *draws += 1;
                        }
                        // the brake behind the rim: the rotor with the wheel, the caliper still
                        if let Some(br) = self.player_brake.as_ref().filter(|_| c.is_player).or(c.brake.as_ref()).or(c.model.brakes.as_ref()) {
                            for (list, bg) in [(&br.rotor, Some(&wb.bg)), (&br.caliper, c.caliper_bindings.get(wi).map(|b| &b.bg))] {
                                let Some(bg) = bg else { continue };
                                pass.set_bind_group(3, bg, &[]);
                                for b in list {
                                    pass.set_bind_group(1, b.bind.as_ref(), &[]);
                                    pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                                    pass.set_vertex_buffer(0, b.vb.slice(..));
                                    pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                                    pass.draw_indexed(0..b.count, 0, 0..1);
                                    *draws += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The cars' torn-off bumpers across the cars' levels (the original has one physics level, the port one per car): a
/// piece no bumper has any more comes out of the level holding it, each piece goes to the level of the car that reaches
/// it (`loose::hand_over_pieces`), and every car reads its pieces back from wherever they are (damageparts.rs).
fn torn_pieces(cars: &mut [CarInstance], world: &crate::physics::loose::LooseWorld, shared: Option<&mut crate::physics::loose::LooseState>) {
    if let Some(level) = shared {
        // (one level for every car: the pieces are its own, nothing to hand over)
        let mut dropped = Vec::new();
        for c in cars.iter_mut() {
            dropped.extend(c.state.parts.dropped());
        }
        for id in &dropped {
            level.remove_piece(*id);
        }
        let mut views: std::collections::HashMap<u64, crate::damageparts::PieceView> = std::collections::HashMap::new();
        for id in level.piece_ids() {
            if let Some((matrix, contacts, _)) = level.piece(id) {
                views.insert(id, crate::damageparts::PieceView { matrix, contacts: contacts.to_vec() });
            }
        }
        for c in cars.iter_mut() {
            let cm = Mat4::from_rotation_translation(c.state.rot, c.state.pos);
            c.state.parts.read_pieces(&cm, &|id| views.get(&id).cloned());
        }
        let _ = world;
        return;
    }
    let mut dropped = Vec::new();
    for c in cars.iter_mut() {
        dropped.extend(c.state.parts.dropped());
    }
    if !dropped.is_empty() {
        for c in cars.iter_mut() {
            if let Some(v) = c.state.veh.as_deref_mut() {
                for id in &dropped {
                    v.loose.remove_piece(*id);
                }
            }
        }
    }
    if cars.iter().any(|c| c.state.veh.as_ref().is_some_and(|v| !v.loose.pieces.is_empty())) {
        // (how far a car reaches: its hull's farthest corner, the piece's half diagonal 0.95 m and a metre - more than
        // a fast car closes in a tick)
        let mut levels: Vec<crate::physics::loose::CarLevel> = cars
            .iter_mut()
            .filter_map(|c| {
                let reach = c.model.params.hull.as_ref().map_or(2.5, |h| h.min.abs().max(h.max.abs()).length()) + 0.95 + 1.0;
                let at = c.state.pos;
                c.state.veh.as_deref_mut().map(|v| crate::physics::loose::CarLevel { at, reach, level: &mut v.loose })
            })
            .collect();
        crate::physics::loose::hand_over_pieces(world, &mut levels);
    }
    let mut views: std::collections::HashMap<u64, crate::damageparts::PieceView> = std::collections::HashMap::new();
    for c in cars.iter() {
        if let Some(v) = c.state.veh.as_deref() {
            for id in v.loose.piece_ids() {
                if let Some((matrix, contacts, _)) = v.loose.piece(id) {
                    views.insert(id, crate::damageparts::PieceView { matrix, contacts: contacts.to_vec() });
                }
            }
        }
    }
    for c in cars.iter_mut() {
        let cm = Mat4::from_rotation_translation(c.state.rot, c.state.pos);
        c.state.parts.read_pieces(&cm, &|id| views.get(&id).cloned());
    }
}


/// Test hooks for the moving damage parts on the player's car: MCLA_PARTS_TEST="hood,trunk,frontA,frontB,rearA,rearB"
/// (loose 0/1, end states 0..2) is put on the car once, two seconds in; MCLA_PARTS_POSE="hood,trunk" holds the
/// flaps at those angles (rad, <= 0 open) for pictures; MCLA_PARTS_LOG prints the parts twice a second.
fn parts_test(st: &mut CarState, cm: &Mat4, clock: f32) {
    static SET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if let Ok(v) = std::env::var("MCLA_PARTS_TEST") {
        if clock > 2.0 && !SET.swap(true, std::sync::atomic::Ordering::Relaxed) {
            // (with damage to match, or the repair test puts the parts straight back)
            st.damage = st.damage.max(0.7);
            st.zones[2] = st.zones[2].max(1.0);
            st.zones[3] = st.zones[3].max(1.0);
            let p = &mut st.parts;
            let n: Vec<u8> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            let g = |i: usize| n.get(i).copied().unwrap_or(0);
            p.hood.loose = g(0) != 0;
            p.trunk.loose = g(1) != 0;
            for (k, b) in [(1usize, 2usize), (0, 4)] {
                if let Some(bm) = p.bumpers[k].as_mut() {
                    bm.a.state = g(b);
                    bm.b.state = g(b + 1);
                }
            }
        }
    }
    if let Some(v) = std::env::var("MCLA_PARTS_POSE").ok() {
        let p = &mut st.parts;
        let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        p.hood.angle = n.first().copied().unwrap_or(0.0);
        p.trunk.angle = n.get(1).copied().unwrap_or(0.0);
    }
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if std::env::var("MCLA_PARTS_LOG").is_ok() && N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 30 == 0 {
        let p = &st.parts;
        let ci = cm.inverse();
        let b = |k: usize| p.bumpers[k].as_ref().map(|b| (b.a.state, b.b.state, ci.transform_point3(b.a.pos) - b.a.rest, ci.transform_point3(b.b.pos) - b.b.rest, b.detached.map(|l| l.pos)));
        eprintln!("parts t {:.2}: damage {:.3} zones {:.2?} hood {} {:.4} trunk {} {:.4} front {:?} rear {:?} events {}", clock, st.damage, st.zones, p.hood.loose, p.hood.angle, p.trunk.loose, p.trunk.angle, b(1), b(0), p.serial);
    }
}

impl CarInstance {
    /// The object (group 3) a batch of the car's model is drawn with: the body's, or a side
    /// window's for that window's glass.
    pub fn batch_object(&self, bi: usize) -> &wgpu::BindGroup {
        self.group_object(self.model.batch_window.get(bi).copied().unwrap_or(0))
    }

    /// The object of a batch group (`CarModel::batch_window`).
    pub fn group_object(&self, group: u8) -> &wgpu::BindGroup {
        match group {
            0 => &self.binding.bg,
            k @ 1..=2 => self.window_bindings.get(k as usize - 1).map_or(&self.binding.bg, |b| &b.bg),
            k => self.part_bindings.get(k as usize - 3).map_or(&self.binding.bg, |b| &b.bg),
        }
    }
}

impl Vehicles {
    /// The traffic cars' LOD (sub_827902F0, from sub_827932D8 into the instance's +28, in the main pass
    /// 0x10000 of sub_82793360): the car's sphere - the bound's centroid through the car's matrix, radius
    /// `CarModel::lod_radius` - tested against the main view (sub_8217C1D8: outside -> -1) and its occluders
    /// (sub_82175F20 -> sub_82175238, `occluder::Selected::hides_sphere`: hidden -> -1); the depth is plane 0 of
    /// the viewport's first block - the near plane, column 2 of view x projection normalised (sub_8217C740) -
    /// through the centre, plus the radius: the distance ahead of the camera less the near distance (the boom
    /// file's `CameraNear`), plus the radius. Not drawn from 500 on (0x82003250), LOD 0 below type +44 x type +40,
    /// LOD 1 below type +48 x type +40, else LOD 2 (per type: `CarModel::lod_steps`). The -1
    /// keeps the car out of every pass: the shadow passes 4 and 8 draw through sub_82791A50, which returns on
    /// it, and so does the main view's (`lod_batches`). Cars without LODs (set-up cars) keep 0. `full_detail`
    /// (OPTIONS > PERFORMANCE > HIGH-DETAIL TRAFFIC, the port's own - DECISIONS.md D19): LOD 0 wherever the
    /// original draws the car at all.
    ///
    /// The glow pass (0x8000 of sub_82793360 -> sub_82790B08 -> sub_8237AC90) draws a car's glows when it is
    /// visible in the view (the instance's per-view byte +557 + view, bit 0x4 - its setter, in the scene's
    /// cull, was not read: here the same view test of the sphere) and either its LOD is not -1 or instance
    /// +556 bit 0x20 is set: the depth below 50 (0x82006BF4, sub_827932D8). The depth is written whether or
    /// not the sphere is in the view (sub_8217C1D8 stores it before its answer), so a car hidden by an
    /// occluder or 500 deep keeps its glows within 50 - `lod_glow`.
    pub fn pick_lods(&mut self, eye: Vec3, axis: Vec3, near: f32, full_detail: bool, frustum: &crate::render::Frustum, occluders: &crate::occluder::Selected) {
        let alpha = self.render_alpha;
        // (diagnostics: MCLA_LOD_FORCE=1 / 2 draws every traffic car at that LOD)
        let force = std::env::var("MCLA_LOD_FORCE").ok().and_then(|v| v.parse::<i8>().ok());
        static FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let log_now = std::env::var("MCLA_LOD_LOG").is_ok() && FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 60 == 0;
        (self.lod_pick, self.lod_glow) = self
            .cars
            .iter()
            .map(|c| {
                if c.model.lods[0].0.is_empty() {
                    return (0, true);
                }
                if let Some(f) = force {
                    return (f, true);
                }
                let pose = c.render_pose(alpha);
                let (centre, r) = (pose.pos + pose.rot * c.model.lod_centre, c.model.lod_radius);
                let d = (centre - eye).dot(axis) - near + r;
                let seen = frustum.intersects_sphere(centre, r);
                let lod = Self::lod_of(c, d, seen && !occluders.hides_sphere(centre, r), full_detail);
                // (sub_82793360, main pass: LOD 1 becomes LOD 2 while car +500 bit 0x4 - on its way out)
                let lod = if lod == 1 && c.removing.is_some() { 2 } else { lod };
                (lod, seen && (lod >= 0 || d < 50.0))
            })
            .unzip();
        if log_now {
            // (diagnostics: MCLA_LOD_LOG=1 prints each second every traffic car drawn, glowing or under 60 deep)
            for (c, (&l, &g)) in self.cars.iter().zip(self.lod_pick.iter().zip(&self.lod_glow)) {
                let pose = c.render_pose(alpha);
                let centre = pose.pos + pose.rot * c.model.lod_centre;
                let d = (centre - eye).dot(axis) - near + c.model.lod_radius;
                if (l >= 0 || g || d < 60.0) && !c.model.lods[0].0.is_empty() {
                    eprintln!("lodpick {} lod {} glow {} d {:.1} r {:.2} dist {:.1}", c.model.name, l, g, d, c.model.lod_radius, centre.distance(eye));
                }
            }
        }
    }

    /// sub_827902F0's answer for depth `d`: -1 outside the view or hidden (`shown` false) or from 500 on.
    fn lod_of(c: &CarInstance, d: f32, shown: bool, full_detail: bool) -> i8 {
        if !shown || d >= 500.0 {
            -1
        } else if d < c.model.lod_steps[0] || full_detail {
            0
        } else if d < c.model.lod_steps[1] {
            1
        } else {
            2
        }
    }

    /// A car's batches and their groups at its LOD of this frame (None: not drawn).
    pub fn lod_batches(&self, ci: usize) -> Option<(&[GpuBatch], &[u8])> {
        let m = &self.cars[ci].model;
        match self.lod_pick.get(ci).copied().unwrap_or(0) {
            l if l < 0 => None,
            0 => Some((&m.batches, &m.batch_window)),
            l => m.lods.get(l as usize - 1).filter(|x| !x.0.is_empty()).map(|x| (&x.0[..], &x.1[..])).or(Some((&m.batches, &m.batch_window))),
        }
    }
}

impl Vehicles {
    /// Roll a side window of a car down (`open`) or up, as the original's window controller:
    /// down only from fully up, up only from fully down; it moves at half the clip's rate (the
    /// DB9's 0.667 s clip takes 1.33 s). Nothing happens on a car whose model has no clip.
    pub fn window(&mut self, car: usize, right: bool, open: bool) {
        let Some(c) = self.cars.get_mut(car) else { return };
        let k = right as usize;
        if let Some(clip) = c.model.windows[k].as_ref() {
            let d = clip.duration;
            c.windows[k].request(open, d);
        }
    }

    /// How far down a side window is: 0 up (closed) .. 1 fully down (open).
    pub fn window_open(&self, car: usize, right: bool) -> f32 {
        let Some(c) = self.cars.get(car) else { return 0.0 };
        let k = right as usize;
        match c.model.windows[k].as_ref() {
            Some(clip) if clip.duration > 0.0 => c.windows[k].clip_time(clip.duration) / clip.duration,
            _ => 0.0,
        }
    }
}

impl Vehicles {
    /// The cars' windows into a shadow pass (the caller's pipeline stays): the original's shadow passes draw
    /// every mesh of the car with VS_CarShadowDepthLocal, the six CarGlass meshes among them - the cabin lies
    /// in the car's own shade and the shadow on the road has no window holes. MCLA_GLASS_NOSHADOW=1: as before.
    pub fn draw_glass_casters<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, draws: &mut usize, range: Option<(Vec3, f32)>) {
        static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *OFF.get_or_init(|| std::env::var("MCLA_GLASS_NOSHADOW").is_ok()) {
            return;
        }
        for (ci, c) in self.cars.iter().enumerate() {
            let pos = c.render_pose(self.render_alpha).pos;
            if let Some((eye, max)) = range {
                if pos.distance(eye) > max {
                    continue;
                }
            }
            let Some((batches, groups)) = self.lod_batches(ci) else { continue };
            pass.set_bind_group(3, &c.binding.bg, &[]);
            for (bi, b) in batches.iter().enumerate() {
                if b.mode != MatMode::CarGlass || b.shader.to_ascii_lowercase().contains("bumped") {
                    continue;
                }
                pass.set_bind_group(3, c.group_object(groups[bi]), &[]);
                pass.set_bind_group(1, b.bind.as_ref(), &[]);
                pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                pass.set_vertex_buffer(0, b.vb.slice(..));
                pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..b.count, 0, 0..1);
                *draws += 1;
            }
        }
    }
}

impl Vehicles {
    /// What the occlusion blobs' pass needs of every car (carblob.rs): the body's matrix, the model's box, the
    /// wheels' foot, and the player's wheels (middle, axle, radius, width - as `upload_uniforms` draws them).
    pub fn blob_cars(&self) -> Vec<crate::carblob::CarBlob> {
        let mut out = Vec::new();
        for c in &self.cars {
            if c.model.bike || (self.hide_player && c.is_player) {
                continue;
            }
            let pose = c.render_pose(self.render_alpha);
            let m = Mat4::from_rotation_translation(pose.rot, pose.pos);
            let mut wheels = Vec::new();
            let mut ground_y = f32::MAX;
            for i in 0..4 {
                let (tyre_w, tyre_r) = c.params().veh.as_ref().filter(|v| v.tyre_width[0] > 0.0).map(|v| if i < 2 { (v.tyre_width[0], v.t.wheel_front.radius + 0.00635) } else { (v.tyre_width[1], v.t.wheel_back.radius + 0.00635) }).unwrap_or((c.model.params.wheel_width, c.model.params.wheel_radius));
                ground_y = ground_y.min(pose.hubs[i].y - tyre_r);
                if c.is_player {
                    let axle = m.transform_vector3(Quat::from_rotation_y(-pose.steer[i]) * Vec3::X).normalize_or(Vec3::X);
                    wheels.push((m.transform_point3(pose.hubs[i]), axle, tyre_r, tyre_w));
                }
            }
            out.push(crate::carblob::CarBlob { m, min: c.model.min, max: c.model.max, ground_y, ambient: c.model.name.to_ascii_lowercase().starts_with("va_"), wheels });
        }
        out
    }

    /// Every blended batch of the cars (windows, lenses) with the caller's pipeline: the blobs' pass draws the
    /// cars over its working value (the original's stencil test).
    pub fn draw_blended_plain<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, range: Option<(Vec3, f32)>) {
        for (ci, c) in self.cars.iter().enumerate() {
            let pos = c.render_pose(self.render_alpha).pos;
            if let Some((eye, max)) = range {
                if pos.distance(eye) > max {
                    continue;
                }
            }
            let Some((batches, groups)) = self.lod_batches(ci) else { continue };
            pass.set_bind_group(3, &c.binding.bg, &[]);
            for (bi, b) in batches.iter().enumerate() {
                if !b.mode.blended() {
                    continue;
                }
                pass.set_bind_group(3, c.group_object(groups[bi]), &[]);
                pass.set_bind_group(1, b.bind.as_ref(), &[]);
                pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                pass.set_vertex_buffer(0, b.vb.slice(..));
                pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..b.count, 0, 0..1);
            }
        }
    }
}

impl Vehicles {
    /// What else the original's shadow passes draw for a car (carshadow.rs): the occluder box under the body, a
    /// sphere in each wheel, and the lamps' lenses (its VS_Light meshes cast too). The caller's pipeline stays.
    pub fn draw_solid_casters<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, draws: &mut usize, range: Option<(Vec3, f32)>) {
        if crate::carshadow::off() {
            return;
        }
        for (ci, c) in self.cars.iter().enumerate() {
            let pose = c.render_pose(self.render_alpha);
            if let Some((eye, max)) = range {
                if pose.pos.distance(eye) > max {
                    continue;
                }
            }
            if c.model.bike || (self.hide_player && c.is_player) {
                continue;
            }
            let Some((batches, _)) = self.lod_batches(ci) else { continue };
            let Some(b0) = batches.iter().find(|b| !b.mode.blended()) else { continue };
            let wheel = |i: usize| c.params().veh.as_ref().filter(|v| v.tyre_width[0] > 0.0).map(|v| if i < 2 { (v.t.wheel_front.radius + 0.00635, v.tyre_width[0]) } else { (v.t.wheel_back.radius + 0.00635, v.tyre_width[1]) }).unwrap_or((c.model.params.wheel_radius, c.model.params.wheel_width));
            let s = crate::carshadow::get(gpu, &c.model.name, pose.hubs, [wheel(0), wheel(2)]);
            pass.set_bind_group(3, &c.binding.bg, &[]);
            for b in batches {
                if b.mode != MatMode::CarGlass || !b.shader.to_ascii_lowercase().contains("bumped") {
                    continue;
                }
                pass.set_bind_group(1, b.bind.as_ref(), &[]);
                pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                pass.set_vertex_buffer(0, b.vb.slice(..));
                pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..b.count, 0, 0..1);
                *draws += 1;
            }
            pass.set_bind_group(1, b0.bind.as_ref(), &[]);
            pass.set_bind_group(2, &gpu.mat_bgs[&(b0.mode as u32)], &[]);
            pass.set_vertex_buffer(0, s.vb.slice(..));
            pass.set_index_buffer(s.ib.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(s.ranges[0].0..s.ranges[0].1, 0, 0..1);
            *draws += 1;
            for (wi, wb) in c.wheel_bindings.iter().enumerate().take(4) {
                let r = s.ranges[if wi < 2 { 1 } else { 2 }];
                pass.set_bind_group(3, &wb.bg, &[]);
                pass.draw_indexed(r.0..r.1, 0, 0..1);
                *draws += 1;
            }
        }
    }
}

/// Which of the windows' two passes are drawn (bit 0 back faces, bit 1 front faces; 0 = the one unculled
/// pass of before): MCLA_GLASS_ONEPASS / MCLA_GLASS_PASS, lighting diagnostics.
fn glass_two_pass() -> u8 {
    static V: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        if std::env::var("MCLA_GLASS_ONEPASS").is_ok() {
            return 0;
        }
        match std::env::var("MCLA_GLASS_PASS").ok().as_deref().map(str::trim) {
            Some("b") => 1,
            Some("f") => 2,
            _ => 3,
        }
    })
}

fn load_wheels(gpu: &mut Gpu, world: &mut World) -> Option<WheelModel> {
    let rim = world.assets.root.join("resources/rims/whl_am_amer_aero");
    load_wheels_with(gpu, world, &rim, "tir_car", 0.812, Some("tir_car_bfg_kdw_blk"))
}

/// The rim's share of the wheel diameter for a rim of so many inches.
fn rim_ratio_for(inches: f32, wheel_radius: f32) -> f32 {
    // (leave at least 4.5 cm of tyre)
    (inches * 0.0254 / (2.0 * wheel_radius)).clamp(0.5, (1.0 - 0.045 / wheel_radius).min(0.9))
}

/// Rim size a car set-up asks for (`resources/vehicle/configs/<name>.xcc`: front and rear
/// rim diameter in inches at 0x83e).
fn config_rim_inches(root: &Path, config: &str) -> Option<f32> {
    let d = std::fs::read(root.join("resources/vehicle/configs").join(format!("{config}.xcc"))).ok()?;
    let v = *d.get(0x83e)? as f32;
    if (13.0..=26.0).contains(&v) { Some(v) } else { None }
}

/// Stock rim size of a car: the starter cars have a set-up file; the others get the
/// size of their time (15 inch classics, 17 inch in the nineties, 19 inch since).
fn stock_rim_ratio(root: &Path, car: &str, wheel_radius: f32) -> f32 {
    let lower = car.to_ascii_lowercase();
    let base = lower.replace("_dub", "");
    let inches = config_rim_inches(root, &format!("starter_{base}")).unwrap_or_else(|| {
        if lower.ends_with("_dub") {
            return 22.0;
        }
        let yy: u32 = base.rsplit('_').next().and_then(|y| y.parse().ok()).unwrap_or(5);
        let year = if yy <= 25 { 2000 + yy } else { 1900 + yy };
        if year < 1990 { 15.0 } else if year < 2000 { 17.0 } else { 19.0 }
    });
    rim_ratio_for(inches, wheel_radius)
}

/// A tire set of `resources/tires/textures/<set>`: an object with two textures, `<set>_s`
/// (+8: the lettering's colour on black) and `<set>_n` (+12: the side wall's relief).
fn tire_textures(gpu: &mut Gpu, root: &Path, set: &str) -> Option<[(Arc<GpuTexture>, u32); 2]> {
    use mcla_formats::rage::{texture::Texture, Rd};
    let file = std::fs::read_dir(root.join("resources/tires/textures").join(set)).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().map_or(false, |x| x == "r1"))?;
    let res = crate::assets::load_resource_file(&file).ok()?;
    let rd = Rd::new(&res);
    let mut out = Vec::new();
    for off in [8usize, 12] {
        let t = Texture::parse(&rd, rd.ptr(off)?).ok()?;
        let key = mcla_formats::hash::joaat(&t.name.to_ascii_lowercase());
        out.push((gpu.upload_texture(key, &t), key));
    }
    let b = out.pop()?;
    Some([out.pop()?, b])
}

pub fn load_wheels_with(gpu: &mut Gpu, world: &mut World, rim_dir: &Path, tyre: &str, rim_ratio: f32, tire_set: Option<&str>) -> Option<WheelModel> {
    let root = world.assets.root.clone();
    let tire_tex = tire_set.and_then(|s| tire_textures(gpu, &root, s));
    let mut sources = Vec::new();
    if let Ok(t) = std::env::var("MCLA_TIRE_DUMP") {
        use mcla_formats::rage::{texture::Texture, Dictionary, Rd};
        if let Ok(rdir) = std::fs::read_dir(root.join("resources/tires/textures").join(&t)) {
            for e in rdir.flatten() {
                let Ok(r) = crate::assets::load_resource_file(&e.path()) else { continue };
                let rd = Rd::new(&r);
                log::info!("tire dict {} vtable {:08x}", e.path().display(), rd.vtable(0));
                for off in (0..64).step_by(4) {
                    let at = if off == 0 { Some(0) } else { rd.ptr(off) };
                    if let Some(Ok(tx)) = at.map(|o| Texture::parse(&rd, o)) {
                        if tx.width > 0 && tx.width <= 4096 && tx.height > 0 {
                            log::info!("tire texture at +{} -> {:?}: {} {}x{} {:?} means {:?}", off, at, tx.name, tx.width, tx.height, tx.format, tx.level_means().first());
                        }
                    }
                }
                match Dictionary::parse(&rd, 0) {
                    Ok(d) => {
                        log::info!("tire dict entries {} {:?}", d.len(), d.hashes.iter().map(|h| format!("{:08x}", h)).collect::<Vec<_>>());
                        for (h, v) in d.hashes.iter().zip(d.values.iter()) {
                            match v.map(|o| Texture::parse(&rd, o)) {
                                Some(Ok(tx)) => log::info!("tire texture {:08x} {} {}x{} means {:?}", h, tx.name, tx.width, tx.height, tx.level_means().first()),
                                Some(Err(e)) => log::info!("tire texture {:08x} error {:#}", h, e),
                                None => log::info!("tire texture {:08x} none", h),
                            }
                        }
                    }
                    Err(e) => log::info!("tire dict error {:#}", e),
                }
            }
        }
    }
    let dump_dir = std::env::var("MCLA_BRAKE_DUMP").ok().map(|d| root.join("resources/brakes").join(d));
    for dir in [root.join("resources/tires/models").join(tyre), dump_dir.unwrap_or_else(|| rim_dir.to_path_buf())] {
        let mut r63 = None;
        let mut r83 = None;
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                match p.extension().and_then(|x| x.to_str()) {
                    Some("r63") => r63 = Some(p),
                    Some("r83") => r83 = Some(p),
                    _ => {}
                }
            }
        }
        if let (Some(m), Some(sh)) = (r63, r83) {
            // (the shaders and the rim's own texture are in the .r83)
            let look = vehicle_model::load_simple(&sh, None).ok();
            if let Ok(mut model) = vehicle_model::load_simple(&m, None) {
                if let Some(look) = look {
                    if std::env::var("MCLA_WHEEL_DUMP").is_ok() {
                        log::info!("wheel look {}: model shaders {:?}, look shaders {:?}, look textures {:?}", dir.display(), model.shaders.iter().map(|s| (s.name.clone(), s.params.len(), s.textures().map(|t| format!("{:08x}={}", t.0, t.1)).collect::<Vec<_>>())).collect::<Vec<_>>(), look.shaders.iter().map(|s| (s.name.clone(), s.params.len(), s.textures().map(|t| format!("{:08x}={}", t.0, t.1)).collect::<Vec<_>>())).collect::<Vec<_>>(), look.textures.iter().map(|(h, t)| format!("{:08x}={} {}x{}", h, t.name, t.width, t.height)).collect::<Vec<_>>());
                    }
                    if model.shaders.is_empty() {
                        model.shaders = look.shaders;
                    }
                    for (h, t) in look.textures {
                        model.textures.entry(h).or_insert(t);
                    }
                }
                sources.push(model);
            }
        }
    }
    if sources.is_empty() {
        return None;
    }
    let mut batches = Vec::new();
    // (where the rim's spokes stand: the mean x of its zone 1)
    let spoke_x = sources.last().filter(|_| sources.len() > 1).and_then(|m| m.meshes.iter().max_by_key(|(mesh, _)| mesh.indices.len())).map_or(-0.2, |(mesh, _)| {
        let (sum, n) = mesh.positions.iter().zip(mesh.uv1.iter()).filter(|(_, u)| u.x.round() == 1.0).fold((0.0f32, 0u32), |a, (p, _)| (a.0 + p.x, a.1 + 1));
        if n > 0 { sum / n as f32 } else { -0.2 }
    });
    for (src_i, model) in sources.iter().enumerate() {
        // (per shader: the rim's badge, a few triangles on the hub with the rim's own texture, is not a
        // coarser level of the rim)
        let mut seen_by: hashbrown::HashMap<u32, usize> = hashbrown::HashMap::new();
        // Tyre and rim are both unit-diameter models. The rim is scaled to its size; the
        // tyre's bead (modelled at radius 0.406, a 21 inch rim on most cars) is pulled in
        // to meet it, which makes the side wall as tall as the rim leaves room for.
        let is_rim = src_i + 1 == sources.len() && sources.len() > 1;
        let xf = if is_rim { Mat4::from_scale(Vec3::new(1.0, rim_ratio, rim_ratio)) } else { Mat4::IDENTITY };
        let bead = rim_ratio * 0.5;
        for (mesh, si) in &model.meshes {
            let mut morphed;
            let mesh = if !is_rim && (bead - 0.406).abs() > 0.004 {
                morphed = mesh.clone();
                for p in morphed.positions.iter_mut() {
                    let rad = (p.y * p.y + p.z * p.z).sqrt();
                    if rad > 1e-4 {
                        let to = if rad >= 0.406 { 0.5 - (0.5 - rad) * (0.5 - bead) / (0.5 - 0.406) } else { rad * bead / 0.406 };
                        p.y *= to / rad;
                        p.z *= to / rad;
                    }
                }
                &morphed
            } else {
                mesh
            };
            let sname = model.shaders.get(*si as usize).map(|s| s.name.clone()).unwrap_or_default();
            if std::env::var("MCLA_WHEEL_DUMP").is_ok() {
                let mn = mesh.positions.iter().fold(Vec3::splat(1e9), |a, b| a.min(*b));
                let mx = mesh.positions.iter().fold(Vec3::splat(-1e9), |a, b| a.max(*b));
                log::info!("wheel mesh (all) src {} shader#{} {} tris {} bounds {:?}..{:?} uv0 {:?}..{:?}", src_i, si, sname, mesh.indices.len() / 3, mn, mx, mesh.uv0.iter().fold(glam::Vec2::splat(1e9), |a, b| a.min(*b)), mesh.uv0.iter().fold(glam::Vec2::splat(-1e9), |a, b| a.max(*b)));
            }
            let seen_tris = seen_by.entry(*si as u32).or_insert(0);
            if *seen_tris > 0 && mesh.indices.len() / 3 <= *seen_tris / 2 {
                continue;
            }
            *seen_tris = (*seen_tris).max(mesh.indices.len() / 3);
            if log::log_enabled!(log::Level::Debug) {
{
                    let mut cols: hashbrown::HashMap<[u8; 4], u32> = hashbrown::HashMap::new();
                    for c in &mesh.colors {
                        *cols.entry(*c).or_insert(0) += 1;
                    }
                    let mut cv: Vec<_> = cols.into_iter().collect();
                    cv.sort_by_key(|x| std::cmp::Reverse(x.1));
                    cv.truncate(8);
                    let mut u1: hashbrown::HashMap<(i32, i32), u32> = hashbrown::HashMap::new();
                    for u in &mesh.uv1 {
                        *u1.entry(((u.x * 10.0) as i32, (u.y * 10.0) as i32)).or_insert(0) += 1;
                    }
                    let mut uv: Vec<_> = u1.into_iter().collect();
                    uv.sort_by_key(|x| std::cmp::Reverse(x.1));
                    uv.truncate(8);
                    log::debug!("wheel attrs src {} verts {} colors {:?} uv1x10 {:?} uv0 n {}", src_i, mesh.positions.len(), cv, uv, mesh.uv0.len());
                }
                                log::debug!("wheel mesh src {} tris {} shader {} params {:?}", src_i, mesh.indices.len() / 3, sname, model.shaders.get(*si as usize).map(|s| s.params.iter().map(|(h, p)| format!("{:08x}={:?}", h, p)).collect::<Vec<_>>()));
            }
            if std::env::var("MCLA_WHEEL_DUMP").is_ok() {
                let mn = mesh.positions.iter().fold(Vec3::splat(1e9), |a, b| a.min(*b));
                let mx = mesh.positions.iter().fold(Vec3::splat(-1e9), |a, b| a.max(*b));
                log::info!("wheel mesh src {} shader {} tris {} bounds {:?}..{:?}", src_i, sname, mesh.indices.len() / 3, mn, mx);
                let mut zx = [(0.0f32, 0u32); 4];
                for (p, u) in mesh.positions.iter().zip(mesh.uv1.iter()) {
                    let z = (u.x.round() as usize).min(3);
                    zx[z].0 += p.x;
                    zx[z].1 += 1;
                }
                log::info!("wheel zones mean x {:?} of {}", zx.iter().map(|(s, n)| if *n > 0 { s / *n as f32 } else { 9.0 }).collect::<Vec<_>>(), rim_dir.display());
            }
            // (a tyre with its set of textures goes through the rim's branch as zones 10, 11)
            let rubber = sname.to_ascii_lowercase().contains("rubber");
            // (RimBadge: the maker's mark on the hub, a cut-out of the rim's own texture. MCLA_RIM_BADGE_OLD: a mirror as before)
            let badge = sname.eq_ignore_ascii_case("RimBadge") && std::env::var("MCLA_RIM_BADGE_OLD").is_err();
            let mode = if rubber && tire_tex.is_none() { MatMode::Rubber } else if rubber || sname.eq_ignore_ascii_case("RimMain") { MatMode::Rim } else if badge { MatMode::AlphaTest } else { MatMode::Chrome };
            let texname = model.shaders.get(*si as usize).and_then(|s| s.diffuse().map(|t| t.to_string()));
            let k = texname.as_deref().map(mcla_formats::hash::joaat).unwrap_or(0);
            // The model's own dictionary first (every rim's one picture, `whl_*.dds`, is only there),
            // under the bare name hash as the car bodies' are. One name sits in two rims with different
            // pictures (whl_am_bbs_lm.dds, in whl_am_bbs_rs_gt too): a picture that is not named after
            // its folder is keyed with the folder.
            // (MCLA_RIM_TEX_OLD: as it stood, for a comparison shot)
            let own = texname.as_deref().filter(|_| k != 0 && std::env::var("MCLA_RIM_TEX_OLD").is_err()).and_then(|n| {
                let l = n.to_ascii_lowercase();
                let stem = l.trim_end_matches(".dds");
                let t = model.textures.get(&k).or_else(|| model.textures.get(&mcla_formats::hash::joaat(stem)))?;
                let folder = if src_i == 0 { tyre.to_ascii_lowercase() } else { rim_dir.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase()).unwrap_or_default() };
                Some((if folder == stem { k } else { k ^ mcla_formats::hash::joaat(&folder) }, t))
            });
            let k = own.map_or(k, |o| o.0);
            let tex = match own {
                Some((key, t)) => gpu.upload_texture(key, t),
                None if k != 0 => world.texture_any(gpu, k).unwrap_or_else(|| gpu.white.clone()),
                None => gpu.white.clone(),
            };
            if std::env::var("MCLA_WHEEL_DUMP").is_ok() {
                log::info!("wheel texture src {} shader {} {:?} key {:08x} own {} {}x{}", src_i, sname, texname, k, own.is_some(), tex.width, tex.height);
            }
            let mut b = match tire_tex.as_ref().filter(|_| rubber) {
                Some([s, n]) => mesh_to_batch_xf(gpu, mesh, &sname, mode, s.0.clone(), n.0.clone(), s.1, n.1, xf),
                None => mesh_to_batch_xf(gpu, mesh, &sname, mode, tex.clone(), tex, k, k, xf),
            };
            if mode == MatMode::Rim || badge {
                // rims need the baked occlusion (vertex colour alpha) the generic path drops; the badge, as
                // the brakes' cut-outs, goes white (its colour channels are empty)
                let n = mesh.positions.len();
                let verts: Vec<Vertex> = (0..n)
                    .map(|i| {
                        let p = xf.transform_point3(mesh.positions[i]);
                        let nr = xf.transform_vector3(mesh.normals.get(i).copied().unwrap_or(Vec3::Y)).normalize_or(Vec3::Y);
                        let nr = car_binormal_mark(mesh, i, nr);
                        let c = mesh.colors.get(i).copied().unwrap_or([255, 255, 255, 255]);
                        let u1 = mesh.uv1.get(i).copied().unwrap_or_default();
                        let (uv1, alpha) = if rubber { ([10.0 + u1.x.round().clamp(0.0, 1.0), u1.y], 255) } else if badge { (u1.to_array(), 255) } else { (u1.to_array(), c[0]) };
                        Vertex { pos: p.to_array(), normal: nr.to_array(), uv0: mesh.uv0.get(i).copied().unwrap_or_default().to_array(), uv1, color: [255, 255, 255, alpha], color1: [0, 0, 0, 255], uv2: [0.0, 0.0] }
                    })
                    .collect();
                b.vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rim vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
            }
            batches.push(b);
        }
    }
    Some(WheelModel { batches, spoke_x })
}

#[cfg(test)]
mod ambient_paint_tests {
    /// Traffic paint against the original (work/trafpaint): c124 recorded in the running game, o6 of
    /// VSAmbientCarPaint run on numbers.
    #[test]
    fn taxi_and_dark_sedan() {
        let taxi = super::ambient_paint([0.5, 0.4, 0.0, 1.0]);
        // (c124 of the taxi in car_b.gpu.bin: (0.21764, 0.13321, 0); o6 = 4 x that)
        assert!((taxi[0] - 4.0 * 0.21764).abs() < 1e-4 && (taxi[1] - 4.0 * 0.13321).abs() < 1e-4 && taxi[2] == 0.0);
        let grey = super::ambient_paint([0.1, 0.12, 0.14, 1.0]);
        assert!((grey[0] - 4.0 * 0.00631).abs() < 1e-4 && (grey[2] - 4.0 * 0.01323).abs() < 1e-4);
        // (white saturates)
        assert_eq!(super::ambient_paint([1.0, 1.0, 1.0, 1.0]), [1.0, 1.0, 1.0, 1.0]);
    }

    /// The traffic glass's zones: VSAmbientGlass reads the texcoord as one half4 over uv0 and uv1 (the
    /// recorded, patched fetch: 16_16_16_16_FLOAT at byte 20 of 32), its zone trunc(uv1.x); 3 and up (the
    /// lamp lenses) take no tint (city.wgsl). The meshes: windows 0, lenses 4 / 6, one zone per mesh.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition vehicle resources"]
    fn traffic_glass_zones() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
        let assets = crate::assets::Assets::new(&root);
        for (name, want) in [("va_chv_cobalt_05", vec![0, 4, 6]), ("va_cad_escalade_03", vec![0, 6]), ("va_350z_04", vec![0, 6])] {
            let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle").join(name)).unwrap();
            let mut zones = std::collections::BTreeSet::new();
            for part in &car.parts {
                for (mesh, si) in &part.meshes {
                    if car.shaders.get(*si as usize).map_or(false, |s| s.name.eq_ignore_ascii_case("AmbientCarGlass")) {
                        assert_eq!(mesh.raw_stride, 32);
                        zones.extend(mesh.uv1.iter().map(|u| u.x.trunc() as i32));
                    }
                }
            }
            assert_eq!(zones.into_iter().collect::<Vec<_>>(), want, "{}", name);
        }
    }

    /// Traffic's glow levels against the running original (2026-10-21, work/lighting2/lvcheck.py):
    /// two cars' recorded levels from their depth, type scale and eased states.
    #[test]
    fn traffic_levels_as_recorded() {
        use super::{traffic_ease, traffic_fade, traffic_state};
        let rows = crate::render::car_lamps::R8; // (attachments: every traffic table's order)
        let lv = |s: [f32; 5], d: f32, sc: f32| -> Vec<f32> { (0..19).map(|i| traffic_state(s, rows.att[i]) * traffic_fade(d) * sc).collect() };
        // a93ffc90: 95.1 m, fGlowIntensityScale 1.856, head on, brake on, side a easing in (0.51)
        let a = lv([1.0, 1.0, 0.51, 0.0, 0.0], 95.1, 1.856);
        // (the depths as printed, to 0.1 m: the levels to 5e-4; with the depth read unrounded the
        // check matched every drawn car to 2e-4)
        assert!((a[3] - 1.3764).abs() < 5e-4 && (a[0] - 1.3764).abs() < 5e-4 && (a[12] - 1.3764).abs() < 5e-4, "{:?}", a);
        // a9394100: 67.5 m, 1.758, head on: brake and tail 0.3934, head / side / fog 1.5736
        let b = lv([1.0, 0.0, 0.0, 0.0, 0.0], 67.5, 1.758);
        let want = [0.3934, 0.3934, 0.0, 1.5736, 0.0, 1.5736, 0.0, 0.0, 1.5736, 0.0, 0.0, 0.0, 0.0, 1.5736, 1.5736, 0.0, 0.0, 0.0, 0.0];
        for (g, w) in b.iter().zip(want) {
            assert!((g - w).abs() < 5e-4, "{:?}", b);
        }
        // the easing: 0.3, then 0.51 (the side lamp recorded at 0.51), off under 0.01
        let e1 = traffic_ease(0.0, true, 1.0 / 30.0);
        let e2 = traffic_ease(e1, true, 1.0 / 30.0);
        assert!((e1 - 0.3).abs() < 1e-6 && (e2 - 0.51).abs() < 1e-6);
        assert_eq!(traffic_ease(0.012, false, 1.0 / 30.0), 0.0);
    }

    /// The player's light set levels read out of the running original (work/lighting2/lamps/st.py,
    /// free roam at night): standing, then easing out of reversing with the brake state at 0.426.
    #[test]
    fn setup_levels_as_recorded() {
        use super::setup_state;
        let rows = crate::render::car_lamps::R8;
        let lv = |s: [f32; 5]| -> Vec<f32> { (0..19).map(|i| setup_state(s, rows.att[i])).collect() };
        let want = [0.25, 0.25, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(lv([1.0, 0.0, 0.0, 0.0, 0.0]), want.to_vec());
        let b = lv([1.0, 0.4260, 0.0, 0.0, 0.2556]);
        let want = [0.570, 0.250, 1.0, 1.0, 0.0, 1.0, 0.256, 0.0, 1.0, 0.0, 0.0, 0.0, 0.426, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        for (g, w) in b.iter().zip(want) {
            assert!((g - w).abs() < 1e-3, "{:?}", b);
        }
    }
}
