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
/// The bones one kind of the light set holds (its constructor makes each table for 8)
const LIGHT_SET_KIND_CAP: usize = 8;

/// `glow_bone_kind` for the disc tests.
#[cfg(test)]
pub(crate) fn glow_bone_kind_pub(name: &str) -> Option<u8> {
    glow_bone_kind(name)
}

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
    /// each glow bone's light-set side (+24, F-27051): 0, 1 on the front bumper part, 2 on the rear one
    pub glow_sides: Vec<u8>,
    /// the cockpit readouts' bones (F-10977: `fontspeedo0` .. `fontclock0` of the base fragment's or a fitted part's
    /// skeleton, the first found): readout (0 speedo, 1 odometer, 2 trip, 3 gear, 4 clock), the bone's matrix in the
    /// model's frame, the box's width and height (its translation limits, max - min of x and y) - `readouts.rs`
    pub font_bones: Vec<(u8, Mat4, f32, f32)>,
    pub params: CarParams,
    /// the same car with every performance upgrade fitted (the tuning file's "mods" set)
    pub params_mods: Option<CarParams>,
    /// the original's simulation tuning (base and mods), when the car has it
    pub simtunes: Option<Arc<mcla_formats::rage::simtune::SimTunes>>,
    /// the car's upgrade table (vehicle_model.rs `CarAsset::vehmods`)
    pub vehmods: Option<Arc<crate::tunebuild::VehMods>>,
    /// the wheels its own set-up gives it (`configs/<name>.xcc`, `CarAsset::fit`)
    pub fit: Option<mcla_formats::rage::simtune::WheelFit>,
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
    /// a bike's front and rear tyre and rim are twice this wide: the span of the wheel_0 / wheel_2
    /// bones' translation limits (F-74884)
    pub bike_wheel_width: [f32; 2],
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
    /// bumper, 20 intercooler (drawn with the car's part objects, `CarInstance::part_bindings`)
    pub batch_window: Vec<u8>,
    /// the side windows' clips (left, right), where the base fragment has them
    pub windows: [Option<vehicle_model::WindowClip>; 2],
    /// the auto spoiler's clip (vehicle_model::BoneClip); its moved bones' meshes and parts are the groups 21 + j
    pub spoiler: Option<Arc<vehicle_model::BoneClip>>,
    /// the convertible roof's clips (`CarAsset::clamshell` / `skinned_top`): the three roof parts are the groups 29 (raised),
    /// 30 (folded), 31 (the skinned roof), the clamshell's moved bones' meshes and parts the groups 32 + j
    pub clamshell: Option<Arc<vehicle_model::BoneClip>>,
    pub skinned_top: Option<Arc<vehicle_model::BoneClip>>,
    /// the dashboard needles' clips (`CarAsset::needles`: tach, speedo; and the fitted add-on gauge's `tach_upgrade`, D+72,
    /// on that part's own skeleton); the j-th moved bone of needle k is the group NEEDLE_GROUP + NEEDLE_BONES k + j
    pub needles: [Option<Arc<vehicle_model::BoneClip>>; 3],
    /// where the add-on gauge stands (its placement in car space): its needle's deltas are P x D x P^-1
    pub needle_place: Mat4,
    /// the skinned roof's batches (group 31): batch index, the vertices at rest (car space) and each vertex's four bones of
    /// the roof's skeleton and weights; and where the roof part stands (its placement in car space)
    pub roof_skin: Vec<(usize, Vec<Vertex>, Vec<([u16; 4], [f32; 4])>)>,
    pub roof_place: Mat4,
    /// the police light bar's beacons (vehicle_model::CopLights)
    pub cop_lights: Option<Arc<vehicle_model::CopLights>>,
    /// the damage map's layout texture (`<car>_maxdamage`: width, height, RGBA8), when the car has it
    pub damage_layout: Option<Arc<(u32, u32, Vec<u8>)>>,
    /// the moving damage parts (`damageparts.rs`): the car's movement settings and bumper slot bones, and the bones
    /// the fitted hood (its matrix index; else the slot's tag 33332) and the trunk lid (`trunk_0`) turn about, in car space
    pub part_rig: crate::damageparts::PartRig,
    pub hood_slot: Option<Mat4>,
    pub trunk_bone: Option<Mat4>,
    /// the bones the fitted rear bumper, front bumper and intercooler hang on (their matrix index, F-14987), in car space
    pub slot_bones: [Option<Mat4>; 3],
    /// where the exhaust and nitrous effects come out (car space, at most two, each with whether it moves with the
    /// rear bumper) and whether the first is a
    /// bone of the rear bumper (`CarAsset::fx_anchors`, `carfx.rs`)
    pub fx_anchors: Vec<(Mat4, bool)>,
    pub fx_anchor_rear: bool,
    /// a traffic model's own wheels (tyre, rim, disc in its body fragment, on its wheel
    /// bones `wheel_0` .. `wheel_5` - `CarPart::mesh_wheel`): their hubs; drawn as batch groups 7 - 12 turned about these -
    /// no rim set of `resources/rims` is put on
    pub own_wheels: Option<[Option<Vec3>; 6]>,
    /// a traffic model's LOD 1 and LOD 2 (its base fragments 0x7b0cd281 / 0x0de3ddb3: the type's drawables at
    /// type +164 / +168, F-89353), each with its batches' groups as `batch_window`; empty for other cars
    pub lods: [(Vec<GpuBatch>, Vec<u8>); 2],
    /// the radius the LOD choice adds to the depth (car +492, F-92604: 0.5 x sqrt 2 x the larger of the
    /// bound's width and length - F-46284: its box max - min, x and z; every "car" type, vtable 0x820C0FD4,
    /// answers 0 to the type's +28, so the 0.5 x width of the "ped" types never applies)
    pub lod_radius: f32,
    /// the middle of that sphere in the car's space (the bound's centroid, F-41581)
    pub lod_centre: Vec3,
    /// the depths of the LOD 0 -> 1 and 1 -> 2 steps (the type's `mcAmbientRenderType.xml`, `traffic::lod_steps`)
    pub lod_steps: [f32; 2],
    /// a traffic type's bound box (min, max; `traffic::bound_box`): what its occlusion blob is sized from (F-68081)
    pub bound_box: Option<(Vec3, Vec3)>,
}

pub struct WheelModel {
    pub batches: Vec<GpuBatch>,
    /// `batches[rim_from..]` are the rim's (before them the tyre's)
    pub rim_from: usize,
    /// the wheel's back drawn in the rim's place when the camera is on the car's side of the wheel:
    /// `whl_backside` of vehicle/shared_utility (F-31291 loads it), the barrel's inside (RimMain, zone 2)
    /// and a black disc at -0.1723 (BlackMatte), scaled as the rim
    pub backside: Vec<GpuBatch>,
    /// `blur_wheel` of vehicle/shared_utility (F-31291 loads it): the disc at x = -0.25, radius 0.5, scaled as the
    /// rim, that F-49519 draws into the stencil only, with the wheel's own motion-blur id (`draw_wheel_blur`)
    pub blur: Option<GpuBatch>,
    /// the bead radius the tyre's vertices were pulled to at load (0.406, the model's own, when they were not): the
    /// car's wheel draw hands it to city.wgsl with the original's innerRadiusScale, which works from the model's radius
    pub tyre_bead: f32,
}

/// A brake's place (F-43091, for the brake draw F-57869): the frame scaled by (0.625, 1.25, 1.25) x the rim's radius and
/// moved 0.03 along its axle, inwards. The rotor's frame is its wheel's (turning, a right wheel's turned over), the
/// caliper's its hub's (a right one's turned over). +4036 1.0 and +4040 0.03 as set at the car's make (the image's
/// constants): x = 0.5 x +4036 x 1.25 rimR, y = z = 1.25 rimR. The move is along the caliper's scaled x row
/// made unit by the vector unit's reciprocal square root and one Newton step, as the recomp has them (vmsum3fp's sum
/// z^2 + (y^2 + x^2), 1 / sqrt, e + 0.5 e (1 - n e^2)), then times +4040 added to the place: unfused single steps.
pub fn brake_place(frame: Mat4, rim_r: f32) -> Mat4 {
    let sy = 1.25f32 * rim_r;
    let sx = 0.5f32 * 1.0 * sy;
    let x = frame.x_axis.truncate() * sx;
    let n = x.z * x.z + (x.y * x.y + x.x * x.x);
    let e = 1.0 / n.sqrt();
    let k = -(n * (e * e) - 1.0) * (0.5 * e) + e;
    let w = (x * k) * 0.03 + frame.w_axis.truncate();
    Mat4::from_cols(x.extend(frame.x_axis.w * sx), frame.y_axis * sy, frame.z_axis * sy, w.extend(1.0))
}

/// A motorcycle's brake places (F-43091's bike branch, F-54299 true): the caliper (+1600) is the hub frame scaled by
/// (+4036 b, b, b), b = the axle's +4100 / +4104, unturned on either side; rotor A (+1856) the wheel frame's three rows
/// scaled so, at the wheel's place; rotor B (+1920) rotor A with its x and y rows negated (the disc on the wheel's other
/// side). All three move along the caliper's scaled x row made unit (as `brake_place`) by t = 2 d x 0.255 + offset fused
/// (d = the axle's +4092 / +4096, the offset mcCarModelTune's fRotorOffset, front +128 / rear +132, on wheels 0 and 2),
/// the caliper and A forwards, B back; then the rotors' rows are scaled by fRotorSize (+120 / +124 on wheels 0 and 2).
/// Returns (caliper, rotor A, rotor B).
pub fn bike_brake_place(hub: Mat4, wheel: Mat4, b: f32, d: f32, offset: f32, rotor_size: f32) -> (Mat4, Mat4, Mat4) {
    let sx = 1.0f32 * b * 1.0;
    let x = hub.x_axis.truncate() * sx;
    let n = x.z * x.z + (x.y * x.y + x.x * x.x);
    let e = 1.0 / n.sqrt();
    let k = -(n * (e * e) - 1.0) * (0.5 * e) + e;
    let t = (d * 2.0).mul_add(0.255, offset);
    let u = (x * k) * t;
    let cal = Mat4::from_cols(x.extend(hub.x_axis.w * sx), hub.y_axis * b, hub.z_axis * b, (hub.w_axis.truncate() + u).extend(1.0));
    let (a0, a1, a2) = (wheel.x_axis * sx, wheel.y_axis * b, wheel.z_axis * b);
    let w = wheel.w_axis.truncate();
    let ra = Mat4::from_cols(a0 * rotor_size, a1 * rotor_size, a2 * rotor_size, (w + u).extend(1.0));
    let rb = Mat4::from_cols(-a0 * rotor_size, -a1 * rotor_size, a2 * rotor_size, (w - u).extend(1.0));
    (cal, ra, rb)
}

/// Which of a motorcycle's discs F-57869 draws on wheel `i` (0 front, 2 rear): [A, B] = [bHasFrontRight / RearRight,
/// bHasFrontLeft / RearLeft] of mcCarModelTune (+202 / +204, +201 / +203); without a vehModel the ctor's 1, 1, 0, 1.
fn bike_discs(m: &CarModel, i: usize) -> [bool; 2] {
    let a = (i >= 2) as usize;
    let has = m.simtunes.as_ref().and_then(|s| s.rotor).map_or([true, true, false, true], |r| r.has);
    [has[2 * a + 1], has[2 * a]]
}

/// A brake of `resources/brakes` (`brk_stk_<car>`, `brk_car_base` ...): the rotor, which
/// turns with the wheel, and the caliper, which does not.
pub struct BrakeModel {
    pub rotor: Vec<GpuBatch>,
    pub caliper: Vec<GpuBatch>,
}

/// A parked car's `settle` while its ground probe is due / after it found no ground (dropped).
const PARK_PROBE: f32 = -1.0;
const PARK_NO_GROUND: f32 = -2.0;

/// The brake model in `dir`, as modelled: `brake_place` puts it behind the wheel.
fn load_brakes(gpu: &mut Gpu, world: &mut World, dir: &Path) -> Option<BrakeModel> {
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
    let xf = Mat4::IDENTITY;
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
            Some((k, t)) => car_upload(gpu, k, t).0,
            None if !name.is_empty() => keys.iter().find_map(|k| car_any(gpu, world, *k)).unwrap_or_else(|| {
                crate::world::note_tex_miss("car", &format!("{name} ({sname})"));
                gpu.white.clone()
            }),
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
    /// the brake rotors' objects: with the wheels' turning (`brake_place`)
    pub rotor_bindings: Vec<ObjectBinding>,
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
    /// a computer car's wipeout / damage-out (chassis +592 0x10 / 0x04 and its timer, physics::wipeout); the
    /// player's are main.rs's `crash_bits` / `crash_timer`
    pub crash: crate::physics::wipeout::Crash,
    /// parked at the kerb: index of its parking spot (`parked::ITEM_SPOT`: parked for a layout
    /// item, no spot)
    pub parked: Option<usize>,
    /// the layout item it stands for (car +40, parked.rs)
    pub item: Option<u32>,
    /// a layout item's car: when it was made and how long it lives at least (car +464 / +468,
    /// 8 + 4U s, F-92604)
    pub amb_born: f32,
    pub amb_life: f32,
    /// a cab's or trailer's id draw (F-92604, offline: the shared MWC's (lo & 0x7FFFFFFF) % 512, the id made from it,
    /// the type's index and the peer; `Vehicles::amb_id`), -1 for every other car
    pub amb_id: i32,
    /// a car on its way out (car +500 bit 0x4, set with the reason at +488 by F-79896 - only the parked
    /// unspawner F-53207 does): its fade (+484), falling by 2 a second until the car is removed
    /// (F-82632 -> F-51512); meanwhile its LOD 1 is drawn as LOD 2 (`pick_lods`)
    pub removing: Option<f32>,
    /// seconds of physics left before a parked car is frozen in place
    pub settle: f32,
    /// roaming street racer (free roam): can be challenged
    pub racer: bool,
    /// takes part in the running event: never despawned, not counted as traffic
    pub pinned: bool,
    /// a deactivated racer (Racer_Deactivate, chassis +592 bit 0x1 - `CarInput::deactivated`): the new
    /// career's starters on VEHICLE SELECT. Stepped with that input whatever else holds the world.
    pub deactivated: bool,
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
    /// a car with no mod value (the player's, a racer whose script gives none: set-up byte +60 = 255): its set-up's
    /// stages, which F-55728 builds the tune from in place of the blend by `perf`
    pub stages: Option<crate::tunebuild::Stages>,
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
    /// the chassis's feature locks, +576 (set and cleared only by Racer_LockFeature F-17816): bit 0x8 nitrous,
    /// 0x10 the slipstream turbo, 0x20 the air ride's switch-on; `CarInput::lock`
    pub lock: u16,
    /// step 19f: MegaStealth - the car's archetype includes only the map's type (2048) and its
    /// wheels' probes likewise (F-83580): it meets no other car and no prop
    pub pu_ghost: bool,
    /// step 19g: Mirror - the paint's DL6.w (F-68530 -> render object +4108 -> F-13513:
    /// the player's car only), added to PS_CarPaint's mirror weight
    pub pu_mirror: f32,
    /// step 19g: Stealth - the car is drawn by pu_fx.rs with PS_Stealth instead of its own look
    pub pu_stealth: bool,
    /// the engine is not running (chassis engine +46 = 0): a scene's car_off / engine_off (F-91263) until the script
    /// starts it again (Racer_PlayIgnition) - no engine sound, no exhaust smoke
    pub engine_off: bool,
    /// the driver is drawn in a cine scene: racer +676 / its character instance's +420 (cinedriver.rs) - set by a
    /// scene's UseRacerNativeCharacter slots and showdriver, cleared by hidedriver and the racer's resets; kept after
    /// the scene
    pub show_driver: bool,
    /// racer +178: raised to the hi-res driver by a scene's UseRacerNativeCharacter on slot 0 / 1 (F-94066); LoadDriver
    /// (F-39898) then takes the hi-res set. Kept while the car lives.
    pub hi_driver: bool,
    /// the side windows' objects (left, right): the body's, moved by the window's clip; empty
    /// when the model has no window clips
    pub window_bindings: Vec<ObjectBinding>,
    /// the side windows' controllers (left, right)
    pub windows: [WindowCtl; 2],
    /// the auto spoiler's controller (D+864, the windows' class at rate 1.0)
    pub spoiler: WindowCtl,
    /// the convertible roof's players and requests (`RoofCtl`)
    pub roof: RoofCtl,
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
    /// the slipstream trail's points (mcCarModel +7200 block - sst_trail.rs)
    pub sst_trail: crate::sst_trail::Trail,
    /// a cruiser's siren target (+304), set by the police (F-78189's callers: on for the cops of a pull-over or
    /// pursuit and through a police scene, off after an escape or the scene; a cop in no tactic keeps it)
    pub siren: bool,
    /// the light bar clip's time (`CopLights`): from the car's making, looped
    pub light_t: f32,
    /// the car-parts object's eased steering (steer_parts.rs): a motorcycle's front wheel is drawn turned by it
    pub steer_parts: crate::steer_parts::SteerParts,
    /// a traffic or parked car's two light slot ids (render instance + 612 / + 660, F-99357; -1 = empty,
    /// 65545 = the hour's light) - main.rs `traffic_light_slots`
    pub amb_slot: [i32; 2],
    /// ... and the lights in them, eased (F-99357; all 0 at the start, F-19499)
    pub amb_light: AmbSlotLights,
    /// the draw object's drawn-steer ring (traffic.rs `SteerRing`; emptied when the car takes its instance, F-46127)
    pub amb_steer: traffic::SteerRing,
    /// the draw object's updates since the car took it (+32: 0 at the activate hook F-25927 -> F-65833, + 1 after each
    /// render update up to 3, F-14577; traffic.rs `parked_render_runs`)
    pub amb_frames: u8,
    /// a parked car once made active (+500 bit 0x10, F-37359 then runs its render update every frame)
    pub amb_hit: bool,
}

/// A traffic or parked car's two slot lights (render instance +576 / +624, F-99357): each slot's point
/// (the car's place + 100 m along the eased direction), colour and strength; and when they were last eased.
/// The ambient cars' pool as far as the light slots go. A render instance belongs to its pooled car for good
/// (made once at the title: F-38773 stores it at car +456), so what the instance holds at +576 passes on with the
/// car object. F-86868 builds two pools of cars: 98, and 50 for the bIsProxy types, linked in array order. F-60281 takes
/// the head of the free list, F-72573 gives a car back to the tail: first in, first out, so all 98 are used once before
/// a car inherits anything. Cars the port has made are noticed here once a frame by uid, not at the moment of the take.
#[derive(Default)]
pub struct AmbPool {
    built: bool,
    /// the free list of each pool; None = an instance never used (ids -1, F-19499)
    free: [std::collections::VecDeque<Option<([i32; 2], AmbSlotLights)>>; 2],
    /// the cars out, in the order they were taken: uid, pool, what the instance holds
    live: Vec<(u32, usize, [i32; 2], AmbSlotLights)>,
}

impl AmbPool {
    fn build(&mut self) {
        if !self.built {
            self.built = true;
            self.free = [std::iter::repeat(None).take(98).collect(), std::iter::repeat(None).take(50).collect()];
        }
    }
    /// The gone go back to the tail, then the new take the head. Returns what each new car inherits.
    fn step(&mut self, now: &[(u32, usize)]) -> Vec<(u32, Option<([i32; 2], AmbSlotLights)>)> {
        self.build();
        let mut back = Vec::new();
        self.live.retain(|l| {
            let keep = now.iter().any(|n| n.0 == l.0);
            if !keep {
                back.push((l.1, l.2, l.3));
            }
            keep
        });
        for (g, ids, light) in back {
            self.free[g].push_back(Some((ids, light)));
        }
        let mut got = Vec::new();
        for &(uid, g) in now {
            if self.live.iter().any(|l| l.0 == uid) {
                continue;
            }
            // (an empty pool: the spawner counts its cars, so the port should not get here; a fresh instance then)
            let old = self.free[g].pop_front().flatten();
            let (ids, light) = old.unwrap_or(([-1, -1], AmbSlotLights::default()));
            got.push((uid, old));
            self.live.push((uid, g, ids, light));
        }
        got
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct AmbSlotLights {
    pub point: [Vec3; 2],
    pub col: [Vec3; 2],
    pub int: [f32; 2],
    pub time: f32,
    /// drawn (eased) this frame / the frame before
    pub seen: bool,
    pub drawn_before: bool,
}

/// A traffic car's lamps as the original's mcAmbientRenderCarInstance keeps them (F-81135,
/// run while the car is drawn): five states eased towards the car's flags (F-24995: on
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
    /// the head lamps' flag (instance byte +557 + view, bit 0x4; `s[0]` eases towards it)
    pub head_on: bool,
    /// the glows' levels by table entry, once drawn
    pub levels: Option<[f32; crate::render::car_lamps::IDS]>,
    /// the lamp programs' state by table entry (PSAmbientCarLight's cover row w)
    pub lens: [f32; crate::render::car_lamps::IDS],
    /// the glass's tint gCarAmbTintColor.w (F-73627): 0.25 + 1.5 sat(min(d / 30, 1) - 0.5), d the
    /// game camera's distance from the car - set once a frame from that camera and used in every pass
    pub tint: f32,
    /// the render instance's four side sums (+560, gCarAmbDamageParams c126): what the post-solve callback
    /// F-13089 added from each contact with a car (vehsim.rs) - never reset, decayed or clamped while
    /// the car lives; zeroed when the instance is set up (F-93976)
    pub dmg: [f32; 4],
    /// its type's `_maxdamage` texture is bound as the vertex programs' DmgTex
    pub dmg_bound: bool,
    /// a set-up car's lamps switched off (mcCar +597 = 0, the racer's +689): `Racer_SetHeadlights(car, 0)`
    /// of the hangout / payback scripts, the cine events `headlights_off` / `car_off` (`cine_lamp_events`);
    /// no head, brake or reversing state while it is set (F-10129)
    pub off: bool,
    /// a set-up car's shift light (the state at +392, the lamp programs' 20th entry, lamp id 19)
    pub shift: f32,
    /// a set-up car's siren target (+304): its police lamps glow while it is set (`police_glows`)
    pub siren_on: bool,
    /// an ambient car's own hour offset for its lights (car +3568: the global MWC's 23 bits x 2^-25,
    /// 0 .. 0.25 h, drawn by the spawn slot F-77422; 0 for the others, F-20039)
    pub hour_offset: f32,
    /// an ambient car's signal blink (car +3572: seconds, + dt each update, back to 0 past 0.75 + `blink_wrap`;
    /// F-46062) and its two offsets drawn after the hour offset by F-77422 (+3576, +3580: 0.2 U - 0.1 s each);
    /// the signals are lit while `blink` <= `blink_on` + 0.375 (F-41231, F-81289)
    pub blink: f32,
    pub blink_wrap: f32,
    pub blink_on: f32,
    /// an ambient car's fade (car +484, 0 at its set-up F-92604) and its rise done (+500 bit 0x8 cleared): it rises by
    /// 2 a second to 1 after the set-up (`traffic::fade_step`, F-82632); the fall of a marked car is `removing`
    pub fade: f32,
    pub faded_in: bool,
}

/// An ambient car's lights (F-75961, bit 2 of the render flags): on when the hour is past the
/// light manager's sunset (+14568, 18.0) - 0.333 - the car's offset, or before its sunrise (+14564,
/// 6.0) + 0.5 + the offset, or when the weather term (the viewport table 0x827E1B10 +8 + +12, wet /
/// foggy weather - the port's `wet`) is over the offset; off on a car fading out (+484 < 0.1).
pub fn traffic_lights_due(hour: f32, offset: f32, wet: bool) -> bool {
    hour > 18.0 - 0.333 - offset || hour < offset + 6.0 + 0.5 || wet
}

#[cfg(test)]
#[test]
fn traffic_lights_due_hours() {
    // start-screen shot 2 at 6.498 h: on for every offset (before 6.5 + offset)
    assert!(traffic_lights_due(6.498, 0.0, false) && traffic_lights_due(6.498, 0.25, false));
    assert!(!traffic_lights_due(6.6, 0.0, false) && traffic_lights_due(6.6, 0.2, false));
    assert!(!traffic_lights_due(12.0, 0.25, false) && traffic_lights_due(12.0, 0.0, true));
    // 17.6 h: past 17.667 - offset only for offsets over 0.067
    assert!(!traffic_lights_due(17.6, 0.05, false) && traffic_lights_due(17.6, 0.1, false));
}

/// F-81135's state table by attachment, from the five eased states (head, brake, signal
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

/// F-96984's state table by attachment for a set-up car (mcCar: the player, rivals, cops), from
/// its eased states in `AmbLamps::s` (head, brake, siren, -, reversing; F-10129): 0 brake 0.25 head +
/// 0.75 brake, 1 tail 0.25 head, 2 brake, 3 head + flash (sat), 4 flash, 5 and 6 head, 7 .. 9 the siren
/// state (+308, the police lamps 13 / 14), 11 reversing, the rest 0.
/// The flash state (+248, `s[3]`) eases to the target +244 that the player's pad input writes from
/// HEADLIGHTS held (F-44589; F-10129 overwrites it only under the "always rain" debug byte); attachment 5
/// is the state at +188, eased from the head state's target, so equal to it.
pub fn setup_state(s: [f32; 5], att: u8) -> f32 {
    let [h, b, siren, flash, r] = s;
    match att {
        0 => 0.25 * h + 0.75 * b,
        1 => 0.25 * h,
        2 => b,
        3 => (h + flash).min(1.0),
        4 => flash,
        5 | 6 => h,
        7..=9 => siren,
        11 => r,
        _ => 0.0,
    }
}

/// The garage's head lamp state (F-10129 with the light manager's byte 14546 set: the byte +39229 of
/// the garage object, which the view change F-34768 sets for the views 3 and 10 .. 13 of the table
/// at 0x828292A0): on in the cabin views, off in every other.
pub fn garage_lamps_due(view: &str) -> bool {
    matches!(view, "Interior" | "FrontSeats" | "Stereo" | "SteeringWheel" | "PostGauges")
}

/// The cine scripts' engine events (F-91263 on the racer of the event's car slot): car_off (23) the engine word
/// (chassis+16) +46 = 0 and the lamps off; engine_off (36) Racer_Deactivate's state F-76651(racer, 1) -> F-30391(1):
/// +592 |= 1, the car stopped and +46 = 0; engine_on (35) F-76651(racer, 0) -> F-30391(0): +592 = 0, +46 = 1.
/// (scene, event, car slot, seconds), every such row of work/lighting2/lamps/lampev.tsv.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineEv {
    CarOff,
    EngineOff,
    EngineOn,
}
/// The story scenes with car_off / engine_off on the player whose caller brings the engine back only later, not
/// right after the scene: the downtown hangouts' and city_champ's at the race core's set-up and go (their race),
/// hollywood_garage_intro at the garage's exit (garage core 0x1557), find_sh_02c after its fade (its core 0x132c).
/// (Every racestart/ scene's comes back at the race's go too.)
pub const ENGINE_BACK_LATER: [&str; 6] = [
    "story/cut_story_hangout_pinkslip_downtown",
    "story/cut_story_hangout_series_downtown",
    "story/cut_story_city_champ",
    "story/cut_story_city_champ_night",
    "story/cut_story_hollywood_garage_intro",
    "story/cut_story_find_sh_02c",
];

pub const CINE_ENGINE_EVENTS: &[(&str, EngineEv, u8, f32)] = &[
    ("cut_cop_arrest", EngineEv::CarOff, 0, 0.0),
    ("cut_cop_arrest_02", EngineEv::CarOff, 0, 0.0),
    ("cut_cop_arrest_03", EngineEv::CarOff, 0, 0.0),
    ("cut_cop_arrest_07", EngineEv::EngineOff, 0, 0.0),
    ("cut_cop_arrest_08", EngineEv::CarOff, 0, 0.0),
    ("cut_cop_arrest_bikestand", EngineEv::CarOff, 0, 0.0),
    ("cut_cop_walk_plyr_02", EngineEv::CarOff, 0, 0.0),
    ("cut_cop_walk_plyr_bike", EngineEv::CarOff, 0, 0.0),
    ("racestart/cut_payback_get_keys", EngineEv::CarOff, 0, 0.0),
    ("racestart/cut_payback_get_keys", EngineEv::EngineOn, 0, 14.5),
    ("racestart/cut_story_hangout_pinkslip_downtown", EngineEv::EngineOff, 0, -7.3),
    ("racestart/cut_story_hangout_pinkslip_downtown", EngineEv::EngineOff, 1, -7.3),
    ("racestart/cut_story_hangout_pinkslip_downtown", EngineEv::EngineOff, 2, -7.3),
    ("story/cut_story_beat_sh_02_exotic", EngineEv::CarOff, 0, -2.66667),
    ("story/cut_story_beat_sh_02_exotic", EngineEv::CarOff, 1, -2.66667),
    ("story/cut_story_bestof_sh_05", EngineEv::CarOff, 0, -8.33333),
    ("story/cut_story_bestof_sh_05", EngineEv::CarOff, 1, -8.33333),
    ("story/cut_story_city_champ", EngineEv::CarOff, 0, -8.33333),
    ("story/cut_story_city_champ_night", EngineEv::CarOff, 0, -8.33333),
    ("story/cut_story_district_champs", EngineEv::CarOff, 0, -8.33333),
    ("story/cut_story_district_champs_night", EngineEv::CarOff, 0, -8.33333),
    ("story/cut_story_find_sh_02c", EngineEv::CarOff, 0, -6.66667),
    ("story/cut_story_hangout_pinkslip_downtown", EngineEv::CarOff, 0, -7.3),
    ("story/cut_story_hangout_pinkslip_downtown", EngineEv::CarOff, 1, -7.3),
    ("story/cut_story_hangout_pinkslip_downtown", EngineEv::CarOff, 2, -7.3),
    ("story/cut_story_hangout_pinkslip_hills", EngineEv::CarOff, 0, -8.33333),
    ("story/cut_story_hangout_pinkslip_hills", EngineEv::CarOff, 1, -8.33333),
    ("story/cut_story_hangout_pinkslip_hills", EngineEv::EngineOff, 0, -8.33333),
    ("story/cut_story_hangout_pinkslip_hills", EngineEv::EngineOff, 1, -8.33333),
    ("story/cut_story_hangout_series_downtown", EngineEv::CarOff, 0, -7.23333),
    ("story/cut_story_hangout_series_downtown", EngineEv::CarOff, 1, -7.23333),
    ("story/cut_story_hangout_series_downtown", EngineEv::CarOff, 2, -7.23333),
    ("story/cut_story_hangout_series_hollywoodc", EngineEv::CarOff, 0, -27.6667),
    ("story/cut_story_hollywood_garage_intro", EngineEv::CarOff, 0, -4.33333),
    ("story/cut_story_hollywood_garage_intro", EngineEv::CarOff, 1, -4.33333),
    ("story/cut_story_payback_intro", EngineEv::CarOff, 0, -5.33333),
    ("story/cut_story_payback_intro", EngineEv::CarOff, 1, -5.33333),
    ("story/cut_story_payback_intro", EngineEv::CarOff, 2, -5.33333),
    ("story/cut_story_race_gh_06", EngineEv::EngineOff, 0, -7.06667),
    ("story/cut_story_races_in_beach", EngineEv::CarOff, 0, -4.83333),
    ("story/cut_story_races_in_beach", EngineEv::CarOff, 1, -4.83333),
    ("story/cut_story_rep_on_strip_garage", EngineEv::CarOff, 0, 8.66667),
    ("story/cut_story_rep_on_strip_garage", EngineEv::CarOff, 1, -6.0),
];

/// The cine scripts' car_on events (generic event 22, F-91263 -> F-91984 on the racer of the event's car slot): the
/// lamps on (in `CINE_LAMP_EVENTS` too) and the engine sound's start with 0 (vt+52, F-44510): its ignition, crank and
/// flare. (scene, car slot, seconds), from work/lighting2/lamps/lampev.tsv - every car_on row; the hangout exits'
/// and rep_on_strip_garage's 6.0 "on" rows are headlights_on (33), which leave the engine alone.
pub const CINE_CAR_ON: &[(&str, u8, f32)] = &[
    ("racestart/cut_payback_get_keys", 0, 14.5),
    ("story/cut_story_district_champs", 0, 74.3333),
    ("story/cut_story_district_champs_night", 0, 74.3333),
    ("story/cut_story_rep_on_strip_garage", 0, 81.3333),
];

/// The cine scripts' lamp switching events (generic events 22 car_on, 23 car_off, 33 headlights_on,
/// 34 headlights_off of the table at 0x827EAD60, handled by F-91263 -> F-62687 on the racer of the
/// event's car slot; `car_restore` (24) takes the flag kept at the scene's start - used by no scene):
/// (scene, on, car slot, seconds). Read from every cine script (work/lighting2/lamps/lampev.py, the
/// hash-named ones decrypted first). The scene's start keeps every racer's switch (F-32153 via
/// BeginDescription) and FinishScene puts it back (F-62501).
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
    ("story/cut_story_beat_sh_02_exotic", false, 0, -2.66667),
    ("story/cut_story_beat_sh_02_exotic", false, 1, -2.66667),
    ("story/cut_story_bestof_sh_05", false, 0, -8.33333),
    ("story/cut_story_bestof_sh_05", false, 1, -8.33333),
    ("story/cut_story_city_champ", false, 0, -8.33333),
    ("story/cut_story_city_champ_night", false, 0, -8.33333),
    ("story/cut_story_district_champs", false, 0, -8.33333),
    ("story/cut_story_district_champs", true, 0, 74.3333),
    ("story/cut_story_district_champs_night", false, 0, -8.33333),
    ("story/cut_story_district_champs_night", true, 0, 74.3333),
    ("story/cut_story_find_sh_02c", false, 0, -6.66667),
    ("story/cut_story_hangout_pinkslip_downtown", false, 0, -7.3),
    ("story/cut_story_hangout_pinkslip_downtown", false, 1, -7.3),
    ("story/cut_story_hangout_pinkslip_downtown", false, 2, -7.3),
    ("story/cut_story_hangout_pinkslip_hills", false, 0, -8.33333),
    ("story/cut_story_hangout_pinkslip_hills", false, 1, -8.33333),
    ("story/cut_story_hangout_series_downtown", false, 0, -7.23333),
    ("story/cut_story_hangout_series_downtown", false, 1, -7.23333),
    ("story/cut_story_hangout_series_downtown", false, 2, -7.23333),
    ("story/cut_story_hangout_series_hollywoodc", false, 0, -27.6667),
    ("story/cut_story_hollywood_garage_intro", false, 0, -4.33333),
    ("story/cut_story_hollywood_garage_intro", false, 1, -4.33333),
    ("story/cut_story_motorcycle_rep", true, 0, -5.33333),
    ("story/cut_story_motorcycle_rep", true, 1, -5.33333),
    ("story/cut_story_payback_intro", false, 0, -5.33333),
    ("story/cut_story_payback_intro", false, 1, -5.33333),
    ("story/cut_story_payback_intro", false, 2, -5.33333),
    ("story/cut_story_race_gh_06", false, 0, -7.06667),
    ("story/cut_story_race_gh_06", true, 0, 57.6333),
    ("story/cut_story_races_in_beach", false, 0, -4.83333),
    ("story/cut_story_races_in_beach", false, 1, -4.83333),
    ("story/cut_story_rep_on_strip_garage", false, 0, 8.66667),
    ("story/cut_story_rep_on_strip_garage", false, 1, -6.0),
    ("story/cut_story_rep_on_strip_garage", true, 0, 73.7333),
    ("story/cut_story_rep_on_strip_garage", true, 0, 81.3333),
    ("story/cut_story_rep_on_strip_garage", true, 0, -6.0),
];

/// F-81135's fade of a traffic car's glow levels with its depth `d` along the camera's
/// axis: (1 - sat((d - 50) / 325))^2.
pub fn traffic_fade(d: f32) -> f32 {
    (1.0 - ((d - 50.0) / 325.0).clamp(0.0, 1.0)).powi(2)
}

/// F-24995: one frame of a lamp state's easing at 9 a second.
pub fn traffic_ease(v: f32, on: bool, dt: f32) -> f32 {
    ease_at(v, on, 9.0, dt)
}

/// F-24995 with the state record's own rate (+0: 9; the siren's +300 18).
pub fn ease_at(v: f32, on: bool, rate: f32, dt: f32) -> f32 {
    use crate::physics::exact::{fma, fnms};
    if on {
        // fmadds((1 - v) rate, dt, v); over 0.99 it is 1
        let n = fma((1.0 - v) * rate, dt, v);
        if n > 0.99 { 1.0 } else { n }
    } else {
        // fnmsubs(rate dt, v, v); under 0.01 it is 0 (bge: a NaN stays)
        let n = fnms(rate * dt, v, v);
        if n < 0.01 { 0.0 } else { n }
    }
}

/// how far (camera depth, m) a traffic car's lamps are drawn and its levels kept up to date: the
/// last update seen at 125.1 m, the model's LOD 1 seen to 140.8 m - taken as 125 (approximate)
pub const TRAFFIC_LAMP_REACH: f32 = 125.0;

/// The glow file's factors by kind (`tune/vehicle/glow`, 0x0f2b8cad: `<kind>_intensity` and
/// `<kind>_player_intensity`; F-96521 takes the second for the player's car).
pub const GLOW_FACTOR: [f32; 19] = [2.0, 2.0, 1.002, 2.0, 1.002, 1.0, 0.528, 1.0, 1.002, 1.0, 1.0, 1.0, 0.346, 1.002, 1.002, 0.728, 0.728, 0.3, 0.3];
pub const GLOW_FACTOR_PLAYER: [f32; 19] = [0.3, 1.004, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];

/// How many of the auto spoiler clip's moved bones get a draw group of their own (21 ..): the Murcielago's clip moves 4.
const SPOILER_GROUPS: usize = 8;

/// The convertible roof's draw groups: 29 the raised roof (`conv_topup`), 30 the folded one (`conv_topdown`), 31 the skinned
/// roof (`conv_top`); 32 + j the meshes and parts on the j-th bone the `clamshell` clip moves (at most CLAM_GROUPS).
const ROOF_GROUP: u32 = 29;
const CLAM_GROUP: u32 = 32;
const CLAM_GROUPS: usize = 48;

/// The dashboard needles' draw groups (after the clamshell's): NEEDLE_GROUP + NEEDLE_BONES k + j the meshes on the j-th
/// bone needle k's clip moves (k 0 the tach, 1 the speedo).
const NEEDLE_GROUP: u32 = CLAM_GROUP + CLAM_GROUPS as u32;
const NEEDLE_BONES: usize = 8;

/// A needle's clip time from the update F-35036 (the gauge records D+960 tach / D+992 speedo, their time +972 / +1004
/// written into the bound player and clamped to [0, the clip's length] there): the tach's phase is
/// rpm / ((L x 30 + 1) x 100), the speedo's mph / (L x 30 + 1), the time L x phase - one frame of the 30 fps clip per
/// 100 rpm or per mph. `len`: the clip's length L (the binder F-53023 copies it to the record's +8).
pub fn needle_time(len: f32, value: f32, tach: bool) -> f32 {
    let frames = len.mul_add(30.0, 1.0);
    let phase = if tach { value / (frames * 100.0) } else { value / frames };
    let t = len * phase;
    let t = if t >= 0.0 { t } else { 0.0 };
    if t - len >= 0.0 { len } else { t }
}

/// The convertible roof's two players on the car-parts object, both the windows' controller (`WindowCtl`) at the
/// constructor's rate 1.0: +480 the hatch (`clamshell`), +576 the top (`skinned_top`); and the requests +2159 raise /
/// +2160 lower, which applying the options sets (F-87944: DOWN -> lower, UP -> raise) and the update F-35036 takes for
/// the local player's car only, when both players stand at the request's start end. `opt`: the option as last seen
/// (None: the car was just made - F-53023 then sets a lowered roof's players, and the side windows', at their end).
#[derive(Debug, Clone, Copy, Default)]
pub struct RoofCtl {
    pub hatch: WindowCtl,
    pub top: WindowCtl,
    pub raise: bool,
    pub lower: bool,
    pub opt: Option<bool>,
}

impl RoofCtl {
    /// One frame of F-35036's roof part, then both players' updates (vt+12). `down`: the option; `player`: the car is the
    /// local human player's (F-65988 - the update's gate; the no-damage byte +435 is 0 off line); `lens`: the
    /// clamshell's and the skinned roof's lengths; `windows`: the side windows' controllers and clip lengths, opened and
    /// closed with the roof. (The port sets a request when the option changes; the original sets it at every apply of
    /// the options - one asking for the end the roof stands at is never taken and is cleared by the next one taken.)
    pub fn step(&mut self, down: bool, player: bool, dt: f32, lens: (f32, f32), windows: &mut [WindowCtl; 2], win_lens: [Option<f32>; 2]) {
        match self.opt {
            None => {
                if down && player {
                    self.hatch = WindowCtl { state: 2, time: lens.0 };
                    self.top = WindowCtl { state: 2, time: lens.1 };
                    for k in 0..2 {
                        if let Some(l) = win_lens[k] {
                            windows[k] = WindowCtl { state: 2, time: l };
                        }
                    }
                }
            }
            Some(was) if was != down => {
                if down {
                    self.lower = true;
                } else {
                    self.raise = true;
                }
            }
            _ => {}
        }
        self.opt = Some(down);
        if player {
            // (+2159: both at the open end -> close both, the windows up; +2160: both closed -> open both, the windows
            // down; the bytes are cleared once both players took it)
            let mut win: Option<bool> = None;
            if self.raise && self.hatch.state == 2 && self.top.state == 2 {
                self.top.request(false, lens.1);
                self.hatch.request(false, lens.0);
                win = Some(false);
                self.raise = false;
                self.lower = false;
            }
            if self.lower && self.hatch.state == 1 && self.top.state == 1 {
                self.top.request(true, lens.1);
                self.hatch.request(true, lens.0);
                win = Some(true);
                self.raise = false;
                self.lower = false;
            }
            if let Some(open) = win {
                for k in 0..2 {
                    if let Some(l) = win_lens[k] {
                        windows[k].request(open, l);
                    }
                }
            }
        }
        self.hatch.update_at(WindowCtl::SPOILER_RATE, dt, lens.0);
        self.top.update_at(WindowCtl::SPOILER_RATE, dt, lens.1);
    }

    /// Which roof shows (the body draw F-92759): the raised part while the top's time is below 0.01 (F-36938), the folded
    /// one while it is above the length less 0.01 (F-80163), the skinned roof while either player is between its ends
    /// (F-91671: 0 < time < length).
    pub fn shown(&self, lens: (f32, f32)) -> [bool; 3] {
        let moving = |c: &WindowCtl, l: f32| c.time > 0.0 && c.time < l;
        [self.top.time < 0.01, self.top.time > lens.1 - 0.01, moving(&self.top, lens.1) || moving(&self.hatch, lens.0)]
    }
}

/// A side window's controller as the original's (vtable 0x82046958; open F-85820, close
/// F-82826, update F-27851): `state` 1 closed (up), 2 open (down), 3 opening,
/// 4 closing; `time` runs at rate 0.5 (A-15115) over the clip, so rolling down takes twice
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
    /// the auto spoiler's (+68 as the controller's constructor sets it; F-53023 sets 0.5 only on the two windows)
    pub const SPOILER_RATE: f32 = 1.0;

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

    /// At rest the time is held at its end (0 closed, the clip's length open); moving, it runs by rate x dt in the
    /// original's fused operations and is set to the end it reaches. (The original holds the time while a motor
    /// sound flagged to be prepared is loading, +72; neither window's object has such a flag.)
    pub fn update(&mut self, dt: f32, duration: f32) {
        self.update_at(Self::RATE, dt, duration)
    }

    /// The same at the controller's own rate (+68).
    pub fn update_at(&mut self, rate: f32, dt: f32, duration: f32) {
        match self.state {
            1 => self.time = 0.0,
            2 => self.time = duration,
            3 => {
                self.time = crate::sndobj::fmadds(rate, dt, self.time);
                if self.time >= duration {
                    self.time = duration;
                    self.state = 2;
                }
            }
            4 => {
                // (fnmsubs: -(rate x dt - time), so an exact end is -0, not yet below 0)
                self.time = -crate::sndobj::fmadds(rate, dt, -self.time);
                if self.time < 0.0 {
                    self.time = 0.0;
                    self.state = 1;
                }
            }
            _ => {}
        }
    }

    /// The clip time the player is set to.
    /// (two fsel, as F-27851: 0 when below 0, the length when (time - length) >= 0)
    pub fn clip_time(&self, duration: f32) -> f32 {
        let t = if self.time >= 0.0 { self.time } else { 0.0 };
        if t - duration >= 0.0 { duration } else { t }
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
    /// the driver's gauge colour A and interior neon colour as the record holds them (w: IntNeonFlashRate, IntNeonIntensity)
    pub cabin_light: [[f32; 4]; 2],
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

/// The walker's aggressor test (F-83713) for the car "this" with velocity `va` and its partner with `vb`, `d` = partner position - this position:
/// this when the partner stands (< 0.2235 m/s), the partner when this stands, else the one whose velocity points at the other the
/// more (velocity . offset / speed).
fn first_is_aggressor(va: Vec3, vb: Vec3, d: Vec3) -> bool {
    const STANDING: f32 = 0.2235;
    let (sa, sb) = (va.length(), vb.length());
    if sb < STANDING {
        true
    } else if sa <= STANDING {
        false
    } else {
        va.dot(d) / sa > vb.dot(-d) / sb
    }
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

/// The chassis' scramble word applied to the pad's pedals (mcCarSim::Update after its copy of the input, before the
/// grid block and the burnout): 0x1 brake 0, else 0x2 brake 1; 0x100 handbrake off, else 0x200 on; 0x8 throttle 1,
/// else 0x4 throttle 0; 0x400 steer 0, else 0x10 -1, 0x20 +1, 0x40 negated; 0x800 throttle 0 and handbrake 0.25,
/// else 0x1000 steer negated and handbrake 0.25 (the port's handbrake is on or off: 0.25 counts as on).
pub fn scramble(mut i: CarInput, f: u32) -> CarInput {
    if f & 0x1 != 0 {
        i.brake = 0.0;
    } else if f & 0x2 != 0 {
        i.brake = 1.0;
    }
    if f & 0x100 != 0 {
        i.handbrake = false;
    } else if f & 0x200 != 0 {
        i.handbrake = true;
    }
    if f & 0x8 != 0 {
        i.throttle = 1.0;
    } else if f & 0x4 != 0 {
        i.throttle = 0.0;
    }
    if f & 0x400 != 0 {
        i.steer = 0.0;
    } else if f & 0x10 != 0 {
        i.steer = -1.0;
    } else if f & 0x20 != 0 {
        i.steer = 1.0;
    } else if f & 0x40 != 0 {
        i.steer = -i.steer;
    }
    if f & 0x800 != 0 {
        i.throttle = 0.0;
        i.handbrake = true;
    } else if f & 0x1000 != 0 {
        i.steer = -i.steer;
        i.handbrake = true;
    }
    i
}

#[derive(Default)]
pub struct Vehicles {
    pub render_alpha: f32,
    /// each car's LOD this frame (`pick_lods`, by the cars' index): 0 .. 2, -1 not drawn
    pub lod_pick: Vec<i8>,
    /// whether the glow pass draws a car's lamp glows this frame (`pick_lods`, by the cars' index)
    pub lod_glow: Vec<bool>,
    /// the camera's eye this frame (`set_wheel_eye`)
    wheel_eye: Vec3,
    /// by the cars' index, bit i: wheel i is drawn from behind - its back in the rim's place (F-57869)
    wheel_back: Vec<u8>,
    /// by the cars' index: the wheels F-49519 blurs this frame - each rim's matrix and its turn about the rim's own x
    /// (radians, unwrapped; render/mb.rs takes the records from them). Empty for a car without them.
    pub blur_wheels: Vec<Vec<(Mat4, f32)>>,
    pub models: Vec<Arc<CarModel>>,
    /// this frame's uniforms, written in one upload (`upload_uniforms`)
    uniforms: crate::render::UniformBatch,
    pub wheels: Option<Arc<WheelModel>>,
    pub cars: Vec<CarInstance>,
    pub traffic_target: usize,
    /// the ambient manager's traffic density scale (+0x64240; None = its 1.0): an ARCADE race's
    /// TRAFFIC row, 0.5 for low (F-15258); it multiplies the region list's target
    pub density_scale: Option<f32>,
    /// solid props around the racing cars (centre, radius), refreshed by the game a few times a second
    pub solids: Vec<(Vec3, f32)>,
    /// the fixed props (physics level state 2) around the racers as the object grid's kind-3
    /// objects (F-47614), for the racer brains' obstacle scan; refreshed with `solids`
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
    /// the driver cars' lanes (F-58331 on the racer's obstacle; racer +640 / +644), by uid
    pub driver_lanes: hashbrown::HashMap<u32, traffic::DriverLanes>,
    /// a scene or the vehicle pick: physics and AI are held
    pub frozen: bool,
    /// mcOpponentManager +4 (F-28020): the racer the frame's per-racer step starts at; it moves on by one
    /// every game frame (`oppmgr::update_order`)
    pub opp_start: i32,
    /// the game frame's clock of that step (`brain::frame_due`)
    pub opp_frame: f32,
    /// a race is on as the ambient manager's byte +0x64251 has it (F-86550): the current race (F-27863), not a payback
    /// (race type 0 - 4, race +3096), not over (+3773, `$IsRaceFinish`) - set by the game each frame
    pub race_event: bool,
    /// the player's standing time (mcPlayer +872, F-16834: under 2.2352 m/s it gains the frame's time, else 0), read by
    /// the traffic's blocking test F-70116 against +876 (`traffic::IDLE_BLOCKS`)
    pub player_idle: f32,
    /// the navsys junction ahead of the player's lane (F-94775 + F-95137, the tracker +5888), as `step_signals` found it
    pub player_junction: Option<usize>,
    /// the same record with its direction (+5888 junction, +5872 the junction lane's unit direction; F-95137)
    pub player_record: Option<(usize, [f32; 3])>,
    /// the race traffic clearing's roads and the lanes it took out of the spawner's choice (F-20685)
    pub race_clearing: traffic::RaceClearing,
    /// the race's grid hold (N-VP-1): seconds to GO. The cars simulate, held by their brakes - the
    /// player's chassis brake forced and his front wheels locked (`CarInput::hold`), the computer
    /// drivers at their pre-race antics (`AiDriver::grid_input`) - so revving and burnouts happen as
    /// in the original.
    pub grid_hold: Option<f32>,
    /// under the grid hold, the racers are not placed yet (the port's dive in front of the start scene):
    /// held by the grid rules with no pedals, the computer drivers' antics not begun (main.rs PLACE_TICKS)
    pub grid_wait: bool,
    /// the player's car held as on the grid (CopPullover: Racer_PreRaceBurnout over the officer's walk) - its
    /// pedals go through under the hold, so it can be spun to bolt
    pub player_hold: bool,
    /// HEADLIGHTS held (the pad input's +40 bit 0x2, F-56064): F-44589 writes it as the player car's flash
    /// lamp target (+244) each update - the high beams (`setup_state` 3 and 4) and their click (F-38670)
    pub player_flash: bool,
    /// the player's chassis scramble word (+624, Racer_SetScrambleInputFlags): pedals forced after the pad's copy
    /// (`scramble`)
    pub player_scramble: u32,
    /// no car has its lamps on (VEHICLE SELECT: three cars standing in a lot)
    pub lights_off: bool,
    /// the finish of the player's rims by zone - spoke A, spoke B, lip (the rim meshes'
    /// uv1.x 1..3): colour, and in w 1 painted, 2 chrome, 3 brushed (0 = as the model is)
    pub player_rim_finish: [[f32; 4]; 3],
    /// the player's cabin colours A, B, C (w 1 = chosen)
    pub player_interior: [[f32; 4]; 5],
    /// the player's gauge colour A and interior neon colour as its record holds them (w: IntNeonFlashRate, IntNeonIntensity) -
    /// what its driver is lit by (`Game::cabin_light_colors`)
    pub player_cabin_light: [[f32; 4]; 2],
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
    /// the player's engine sound's slewed rpm (`Sound::tach_rpm`), which the tach needle shows (F-35036)
    pub player_tach_rpm: Option<f32>,
    /// the player's car is driven by the test driver (`--auto`): it passes traffic like the racers
    pub auto_player: bool,
    /// the city's parking spots and their quadtree (parked.rs), the active lists (M+1804) and
    /// the random numbers the parked-car spawner draws (the original's shared MWC)
    pub parking: mcla_formats::rage::parking::ParkingTree,
    park_lists: Vec<u32>,
    park_rng: parked::Mwc,
    /// the second shared MWC (the 8 bytes 16 below `park_rng` in the image, a = 0x5CDCFAA7): F-77422 draws an ambient car's
    /// hour and blink offsets from it (`traffic::init_draws`); its seed and its other users are not modelled
    amb_rng2: parked::Mwc,
    /// the cruising ghosts: the ambient request pool and the proxy cars on their routes (traffic.rs `Ghosts`)
    pub ghosts: traffic::Ghosts,
    park_dump_frame: u32,
    /// (diagnostics, MCLA_PARK_DUMP) parked spawn tries turned down, by reason
    park_why: [u32; 5],
    /// the layout items (F-16770: a pool of 512) and each quadtree node's items in the order
    /// they were put in (node +16 / +20)
    pub layout: Vec<Option<parked::LayoutItem>>,
    pub node_items: Vec<Vec<u32>>,
    pub item_free: std::collections::VecDeque<u32>,
    /// the items whose person stands (peds.rs keeps it), and the items the spawner drew for a
    /// person (F-39915) or for a car following its race path (F-45833): (item, its
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
    /// the walkers' instance matrices (rows, place) as their last update left them (main.rs sets them after
    /// `Peds::update`): what the cars' rail sweeps meet in the next frame (F-96156 updates the walkers after the cars)
    pub walkers: Vec<[Vec3; 4]>,
    /// the data root the level's bound files are read from (`traffic::RailLevel`)
    pub data_root: std::path::PathBuf,
    /// parked cars on (> 0) or off (0); the count itself is not used: the spawner fills a
    /// quarter of the spots of the nodes round the focus (parked.rs)
    pub parked_target: usize,
    /// AmbientMgr_SetParkedCarsEnable(0) in force (main.rs: CarSelection, the new career's opening)
    pub parked_held: bool,
    /// the ambient manager's cutscene mode (F-97032: AmbientMgr_SetCutscene's flag, the race
    /// timer >= 0, no replay recording or playback): the cutscene camera's eye and forward - the
    /// focus and view of the spawners (F-83397), set by main.rs while a scene plays
    pub amb_cutscene: Option<(Vec3, Vec3)>,
    /// the ambient manager's front-end mode (F-95688, AmbientMgr_SetFrontEnd from the
    /// press-start script): the start screen's camera and the shot's four values and density
    /// index (traffic.rs `FrontEnd`), set by main.rs while the start screen is up
    pub amb_frontend: Option<crate::titlecam::FrontEndAmbient>,
    /// the start screen viewport's tan(fov / 2) up and across (viewport +712, +716)
    pub amb_frontend_tan: [f32; 2],
    amb_frontend_was: bool,
    wheel_cache: hashbrown::HashMap<(std::path::PathBuf, String, u32), Arc<WheelModel>>,
    brake_cache: hashbrown::HashMap<(std::path::PathBuf, u32, i32), Arc<BrakeModel>>,
    /// the cars' files as read (`vehicle_model::load_car`) by folder: parsed ahead on a worker thread (`prefetch`)
    /// and kept for the next build of the same car - a set-up's build follows the plain one (docs/PERFORMANCE.md,
    /// 2026-10-23 (4)); the last `PARSED_KEEP` kept, by the order they were taken
    parsed: hashbrown::HashMap<String, Parsed>,
    parsed_stamp: u64,
    /// car builds' CPU part (`prep_car`) made ahead on worker threads, by car folder and configuration, taken by
    /// `load_model_cfg`: the read-ahead's plain build (`prefetch`), the racers' set-ups (`dress_setups`) -
    /// docs/PERFORMANCE.md 2026-10-27 (14); the last `PREPS_KEEP`
    preps: Vec<(String, CarConfig, PrepSlot)>,
    /// the cars and set-ups whose build `dress_setups` has had made ahead
    dress_ahead: std::collections::HashSet<(String, String)>,
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
    /// the render instances' pool, for the light slots a new car inherits
    pub amb_pool: AmbPool,
    /// this frame's head-lamp cones (`pick_cones`), seven rows each
    pub cones: Vec<[[f32; 4]; 7]>,
    /// the cars' damage maps (crate::damagemap): the game's pool of 8 render targets, each a car's
    /// (its uid) for as long as the car is there
    dmg_maps: Vec<DmgMap>,
    /// traffic types' DmgTex (the type's `<type>_maxdamage`, F-89353: type +224, else its model's
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
    /// plate background and font (RGBA), and the finished plates by text (the cars hold them: every traffic car
    /// has a plate of its own, a plate of a car that has gone goes with `Gpu::trim_textures`)
    plate_art: Option<(Option<PlateBg>, Vec<u8>)>,
    plates: hashbrown::HashMap<String, std::sync::Weak<wgpu::BindGroup>>,
    plate_serial: u32,
    /// the background of the player's plate (a name of `resources/license_plates/backgrounds`:
    /// the vehicle record's LicenseBG - `mc4`; none = the California plate), and those loaded
    pub player_plate_bg: Option<String>,
    plate_bgs: hashbrown::HashMap<String, Option<PlateBg>>,
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
/// C = the type's vColors[idx] x fScale (`traffic::AmbType::color`) - F-73627 sets C^2.2 in both
/// paint rows (c124, c125: recorded (0.21764, 0.13321, 0) for a taxi, work/trafpaint/tp_caps.py), the
/// vertex program lerps them and takes sat(4 x) of it (work/trafpaint/tp_px.py: o6).
pub fn ambient_paint(c: [f32; 4]) -> [f32; 4] {
    let f = |x: f32| (4.0 * x.max(0.0).powf(2.2)).min(1.0);
    [f(c[0]), f(c[1]), f(c[2]), c[3]]
}

/// Traffic and parked cars cast no sun shadow (`Vehicles::draw_shadow`); MCLA_TRAFFIC_CASTS=1: they do, as before.
fn traffic_casts_none() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_TRAFFIC_CASTS").is_err())
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
    GpuBatch { shader: sname.to_string(), vb, ib, count: mesh.indices.len() as u32, bind, mode, min: mn, max: mx, bucket: 0, cull: 0 }
}

/// The California plate background of the disc (`resources/license_plates/backgrounds`),
/// uploaded once. The files hold a single texture object behind the root pointer.
fn plate_texture(gpu: &mut Gpu, world: &World) -> (Arc<GpuTexture>, u32) {
    const KEY0: u32 = 0xb59c_dcaf; // joaat("plate_ca_00")
    let raw = car_tex_raw();
    let key = if raw { KEY0 ^ 0x0052_4157 } else { KEY0 };
    if let Some(t) = gpu.textures.get(&key) {
        return (t.clone(), key);
    }
    let dir = world.assets.root.join("resources/license_plates/backgrounds/plate_ca_00");
    let file = std::fs::read_dir(&dir).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.extension().map_or(false, |x| x == "r1")));
    if let Some(f) = file {
        if let Ok(res) = crate::assets::load_resource_file(&f) {
            let rd = mcla_formats::rage::Rd::new(&res);
            if let Some(o) = rd.ptr(0) {
                if let Ok(t) = mcla_formats::rage::texture::Texture::parse(&rd, o) {
                    return (if raw { gpu.upload_texture_raw(key, &t) } else { gpu.upload_texture(key, &t) }, key);
                }
            }
        }
    }
    (gpu.white.clone(), 0)
}

/// The cars' maps are uploaded as stored, not gamma-decoded: no car program's fetch constant asks for gamma
/// (the recorded noon frame, every sampler of PS_BumpSpec, PS_Light, PS_Plate, PS_Emissive, PS_BumpedChrome ...:
/// word 0 bits 2 - 9 clear), so the original filters the stored texels and its rule follows; city.wgsl decodes
/// after the filter (`car_raw_tex`). Keyed `hash ^ 0x0052_4157`, apart from a gamma upload of the same file.
/// MCLA_DEBUG_VIEW=780: the gamma uploads as before (city.wgsl reads the same switch).
pub fn car_tex_raw() -> bool {
    std::env::var("MCLA_DEBUG_VIEW").map_or(true, |v| v.trim() != "780")
}

/// A car's map from its own dictionary, as stored or gamma-decoded by `car_tex_raw`, with the key it is under.
fn car_upload(gpu: &mut Gpu, key: u32, t: &mcla_formats::rage::texture::Texture) -> (Arc<GpuTexture>, u32) {
    if car_tex_raw() {
        (gpu.upload_texture_raw(key ^ 0x0052_4157, t), key ^ 0x0052_4157)
    } else {
        (gpu.upload_texture(key, t), key)
    }
}

/// ... and a global or shared one (`World::texture_any`).
fn car_any(gpu: &mut Gpu, world: &mut World, key: u32) -> Option<Arc<GpuTexture>> {
    if car_tex_raw() { world.texture_any_raw(gpu, key) } else { world.texture_any(gpu, key) }
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

/// The skinned roof posed at the top's clip time `t` into its batches' vertex buffers (`vehicle_model::skin_vertex`).
fn roof_skin_write(gpu: &Gpu, model: &CarModel, top: &vehicle_model::BoneClip, t: f32) {
    if model.roof_skin.is_empty() {
        return;
    }
    let nb = model.roof_skin.iter().flat_map(|(_, _, s)| s.iter().flat_map(|(b, _)| b.iter())).copied().max().unwrap_or(0) as usize + 1;
    let by_bone = vehicle_model::roof_bone_deltas(top, model.roof_place, nb, t);
    for (bi, rest, skin) in &model.roof_skin {
        let Some(batch) = model.batches.get(*bi) else { continue };
        let out: Vec<Vertex> = rest.iter().zip(skin).map(|(v, (b, w))| {
            let (q, n) = vehicle_model::skin_vertex(&by_bone, Vec3::from(v.pos), Vec3::from(v.normal), *b, *w);
            Vertex { pos: q.to_array(), normal: n.to_array(), ..*v }
        }).collect();
        gpu.queue.write_buffer(&batch.vb, 0, bytemuck::cast_slice(&out));
    }
}

/// CPU-side accumulator for meshes that share a material.
struct MergeAcc {
    verts: Vec<Vertex>,
    skin: Vec<([u16; 4], [f32; 4])>,
    indices: Vec<u32>,
    d: Arc<GpuTexture>,
    s2: Arc<GpuTexture>,
    sname: String,
    mode: MatMode,
    min: Vec3,
    max: Vec3,
}

impl MergeAcc {
    /// A mesh's geometry (`MeshGeo`, its indices from 0) after what the batch holds: as `MeshGeo::push` into the batch
    /// itself had put it.
    fn append(&mut self, g: MeshGeo) {
        let base = self.verts.len() as u32;
        self.min = self.min.min(g.min);
        self.max = self.max.max(g.max);
        self.verts.extend(g.verts);
        self.skin.extend(g.skin);
        self.indices.extend(g.indices.iter().map(|i| i + base));
    }
}

/// One mesh of a car build: its vertices and indices as `load_model_cfg` merges them, made apart from the GPU
/// (`Vehicles::prep_car`).
struct MeshGeo {
    verts: Vec<Vertex>,
    /// the skinned roof's vertices: each one's four bones of the roof's skeleton and weights (else empty)
    skin: Vec<([u16; 4], [f32; 4])>,
    indices: Vec<u32>,
    min: Vec3,
    max: Vec3,
}

/// A mesh of `prep_car`: its geometry and what the build needs to find its textures and batch.
struct MeshPrep {
    sname: String,
    mode: MatMode,
    /// the colour and second maps' names
    dn: Option<String>,
    sn: Option<String>,
    /// the plate's own map, the carbon weave, the windows' glass maps (`plate_texture`, `carbon_weave`, `glass_maps`)
    plate: bool,
    carbon: bool,
    win_cand: bool,
    /// the batch key's third part: mode, cull class (bits 12 - 14, `GpuBatch::cull`), moving group, LOD
    kbits: u32,
    /// with the glass maps (lens -1) where `win_cand`; `geo_alt` without them, where that differs
    geo: MeshGeo,
    geo_alt: Option<MeshGeo>,
}

/// A car build's CPU part (`Vehicles::prep_car`): the meshes in build order and what is gathered with them.
struct CarPrep {
    car_is_ambient: bool,
    meshes: Vec<MeshPrep>,
    lamp_acc: hashbrown::HashMap<(i32, bool), (Vec3, u32)>,
    own_hubs: [Option<Vec3>; 6],
    glow_bones: Vec<(u8, Vec3, Vec3)>,
    glow_sides: Vec<u8>,
    font_bones: Vec<(u8, Mat4, f32, f32)>,
    variants: Vec<(Slot, Vec<String>)>,
    variant_labels: Vec<(Slot, Vec<String>)>,
    extra_variants: std::collections::HashSet<(String, usize)>,
    part_rig: crate::damageparts::PartRig,
    hood_slot: Option<Mat4>,
    trunk_bone: Option<Mat4>,
    slot_bones: [Option<Mat4>; 3],
    roof_place: Mat4,
    /// the fitted add-on gauge's needle clip and placement (CarModel::needles [2], needle_place)
    upgrade: Option<(Arc<vehicle_model::BoneClip>, Mat4, std::path::PathBuf)>,
}

impl MeshGeo {
    fn new() -> MeshGeo {
        MeshGeo { verts: Vec::new(), skin: Vec::new(), indices: Vec::new(), min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) }
    }

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
            // (a traffic tyre, amb 2: colour alpha 128 - city.wgsl's mark for PSAmbientTire, and no dent)
            let ca = if tw.is_some() { 255 } else if amb == 2 { 128 } else { 0 };
            // (a traffic model's vertex: its stored tangent in the car's frame in the second colour's rgb, t x 0.5
            // + 0.5 - the ambient programs bend the normal along it where the car is dented (city.wgsl vs_body);
            // its meshes have no second colour set, so those three bytes are otherwise unused; 0 0 0 - no unit
            // tangent comes out so - marks the tyre, whose program, VSAmbientTire, has no dent: amb 2)
            // (a tyre keeps its tangent too since 2026-10-28: PSAmbientTire's normal map needs the frame; it is told by
            // the colour's alpha, 128, instead)
            let at = if amb >= 1 && mesh.colors1.is_empty() { mesh.tangents.get(i).filter(|_| mesh.tangents.len() == n).map(|t| xf.transform_vector3(t.truncate()).normalize_or_zero()) } else { None };
            let c1 = match at {
                Some(t) => [c1[0], ((t.x * 0.5 + 0.5) * 255.0).round() as u8, ((t.y * 0.5 + 0.5) * 255.0).round() as u8, ((t.z * 0.5 + 0.5) * 255.0).round() as u8],
                None => c1,
            };
            self.verts.push(Vertex { pos: pos.to_array(), normal: nrm.to_array(), uv0: uv0.to_array(), uv1: uv1.to_array(), color: [c[1], c[2], c[3], ca], color1: [c1[1], c1[2], c1[3], c1[0]], uv2: [if dark_above.map_or(false, |y| pos.y >= y) { 1.0 } else { 0.0 }, lens] });
        }
        // (a mirrored instance - the second exhaust tip - keeps its faces outward: its winding
        // reversed, as the original flips the cull mode for it, F-39836)
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

/// A set-up car's wheel `i` as F-47822 draws it, from the sim's last update: the turn (rows axle, up, way of the posed
/// matrix) and how far the pose moved the wheel's place. None for a car the sim does not run or without its `.r50`,
/// and with MCLA_WHEEL_POSE_OLD.
fn drawn_wheel_pose(c: &CarInstance, i: usize) -> Option<(Quat, Vec3)> {
    static OLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *OLD.get_or_init(|| std::env::var_os("MCLA_WHEEL_POSE_OLD").is_some()) {
        return None;
    }
    let tunes = c.model.simtunes.as_ref()?;
    let vs = c.state.veh.as_ref()?;
    let world = vs.wheels[i].world?;
    let p = c.params();
    let vt = p.veh.as_ref()?;
    let st = &c.state;
    let body = vs.body_now.unwrap_or_else(|| [st.rot * Vec3::X, st.rot * Vec3::Y, st.rot * Vec3::Z, st.pos]);
    let m = crate::wheel_pose::local(&body, &world);
    let wide = c.setup.or(c.model.fit).is_some_and(|f| f.wide) as usize;
    let car = crate::wheel_pose::CarTerms {
        bike: c.model.bike,
        bike_turn: c.steer_parts.out,
        ceiling: [tunes.fender_ceiling[0][wide], tunes.fender_ceiling[1][wide]],
        steer_limit: tunes.tire_steer[0],
        steer_offset: tunes.tire_steer[1],
    };
    let front = i < 2;
    let wt = if front { &vt.t.wheel_front } else { &vt.t.wheel_back };
    // (the wheel's +432: the hydraulics' or the air ride's last write, else the tune's SuspensionOffset)
    let w432 = vs.hyd.ofs.map_or(wt.suspension_offset, |o| o[i]);
    let (rest, offset) = match vs.rest_exact {
        Some(r) => (Vec3::new(r[i][0], r[i][1], r[i][2]), r[i][4]),
        None => (Vec3::new(p.wheel_pos[i].x, p.wheel_pos[i].y - p.susp_rest, p.wheel_pos[i].z), w432),
    };
    let w = crate::wheel_pose::WheelTerms {
        rest,
        axis: vt.t.steer_axis.filter(|_| front).map_or(Vec3::Y, Vec3::from),
        radius: wt.radius,
        width: vt.tyre_width[!front as usize],
        steering_limit: wt.steering_limit,
        camber_limit: wt.camber_limit,
        lean: crate::physics::exact::sign(p.wheel_pos[i].x) * vs.wheels[i].camber,
        comp: vs.wheels[i].comp,
        travel_extent: wt.travel_extent,
        travel_limit: wt.travel_limit,
        offset,
    };
    // (a motorcycle's front wheel, F-54299's wheel 0: the port's two front wheels, drawn on the centre line together)
    let posed = crate::wheel_pose::pose(m, &car, &w, front);
    let rot = Quat::from_mat3(&glam::Mat3::from_cols(posed[0], posed[1], posed[2])).normalize();
    Some((rot, posed[3] - m[3]))
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

    /// A car with no mod value: the tune is built from its set-up's stages (F-55728, tunebuild.rs); None goes back to
    /// the blend by `perf`.
    pub fn set_stages(&mut self, st: Option<crate::tunebuild::Stages>) {
        if self.stages != st {
            self.stages = st;
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
        if self.perf <= 0.0 && self.setup.is_none() && self.stages.is_none() {
            return;
        }
        let b = &self.model.params;
        let f = self.perf;
        let mut p = b.clone();
        if let Some(s) = self.model.simtunes.as_ref() {
            // the game blends the base and the upgraded tuning field by field (a mod value); a car with none (byte 255:
            // the player's) is built from its set-up's stages with the car's own vehMods weights (F-19699 -> F-55728)
            let mut tune = match (self.stages.as_ref(), self.model.vehmods.as_deref()) {
                (Some(st), Some(vm)) => crate::tunebuild::build(&s.base, &s.mods, st, vm, self.model.bike, [s.fender_height[0][0], s.fender_height[1][0]], s.air_floor),
                _ => mcla_formats::rage::simtune::SimTune::lerp(&s.base, &s.mods, f),
            };
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
            // (F-55728: the air ride - the base's Rise and rates, the drops from the wheels as the set-up left them)
            tune.set_air_ride(s);
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

/// The files of a set-up's rim, tyres and brakes read once on a worker thread and dropped, so that dressing a car
/// in it later (`Vehicles::dress_setups`) reads them from the system's file cache rather than the disk - what is
/// loaded is unchanged (docs/PERFORMANCE.md 2026-10-23 (6)). Each folder once a run.
pub fn warm_dress(root: &Path, d: &Dress) {
    static DONE: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());
    let mut dirs = vec![root.join("resources/tires/models/tir_car"), root.join("resources/tires/textures").join(&d.tire)];
    dirs.extend(d.rim_dir.clone());
    dirs.extend(d.brake.as_ref().map(|b| root.join("resources/brakes").join(b)));
    let Ok(mut done) = DONE.lock() else { return };
    dirs.retain(|p| !done.contains(p));
    if dirs.is_empty() {
        return;
    }
    done.extend(dirs.iter().cloned());
    let _ = std::thread::Builder::new().name("dress read".into()).spawn(move || {
        for d in dirs {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let _ = std::fs::read(e.path());
            }
        }
    });
}

/// A car's files being read on a worker thread, or read (`Vehicles::parsed`).
enum Parsed {
    /// (and when it was asked for; with the plain build's CPU part, `prep_car`)
    Reading(std::thread::JoinHandle<Result<(CarAsset, CarPrep)>>, u64),
    Read(Arc<CarAsset>, u64),
}

/// A car build's CPU part made ahead (`Vehicles::preps`).
enum PrepSlot {
    Ready(CarPrep),
    Making(std::thread::JoinHandle<CarPrep>),
}

/// builds made ahead and not taken, at most
const PREPS_KEEP: usize = 12;

const PARSED_KEEP: usize = 6;
/// read ahead but not taken yet, at most
const PREFETCH_KEEP: usize = 8;

/// `Vehicles::parsed`'s key: the car's folder name (the callers spell the path differently).
fn parsed_key(dir: &Path) -> String {
    dir.file_name().map_or_else(|| dir.to_string_lossy().to_string(), |n| n.to_string_lossy().to_ascii_lowercase())
}

impl Vehicles {
    /// Start reading the car in `dir` on a worker thread, if it is not read or being read (the model built from it
    /// later takes it from there - `load_model_cfg`).
    pub fn prefetch(&mut self, world: &World, dir: &Path) {
        let key = parsed_key(dir);
        // (a model of the car may be built already, but in other parts: the callers ask only when the build they want is not)
        if self.parsed.contains_key(&key) {
            return;
        }
        // (read ones nobody took - a race's guessed grid that was not drawn: the oldest go past PREFETCH_KEEP)
        loop {
            let idle: Vec<(String, u64)> = self.parsed.iter().filter_map(|(k, v)| if let Parsed::Reading(h, t) = v { h.is_finished().then(|| (k.clone(), *t)) } else { None }).collect();
            let Some(old) = idle.iter().min_by_key(|(_, t)| *t).filter(|_| idle.len() >= PREFETCH_KEEP) else { break };
            self.parsed.remove(&old.0);
        }
        self.parsed_stamp += 1;
        let stamp = self.parsed_stamp;
        let assets = crate::assets::Assets::new(world.assets.root.clone());
        let d = dir.to_path_buf();
        // (and its plain build's CPU part: the race start builds the grid so before it dresses it)
        let read = move || vehicle_model::load_car(&assets, &d).map(|c| {
            let p = Vehicles::prep_car(&c, &d, &CarConfig::default());
            (c, p)
        });
        if let Ok(h) = std::thread::Builder::new().name("car read".into()).spawn(read) {
            log::info!("vehicles: reading {} ahead", dir.display());
            self.parsed.insert(key, Parsed::Reading(h, stamp));
        }
    }

    /// True while the car in `dir` is still being read on its worker thread (`prefetch`).
    pub fn reading(&self, dir: &Path) -> bool {
        matches!(self.parsed.get(&parsed_key(dir)), Some(Parsed::Reading(h, _)) if !h.is_finished())
    }

    fn keep_prep(&mut self, key: String, cfg: CarConfig, slot: PrepSlot) {
        self.preps.retain(|(k, c, _)| !(*k == key && *c == cfg));
        self.preps.push((key, cfg, slot));
        if self.preps.len() > PREPS_KEEP {
            self.preps.remove(0);
        }
    }

    /// The CPU part of the build of `dir` in `cfg` made ahead (`preps`), waiting for its worker if it is still making it.
    fn take_prep(&mut self, dir: &Path, cfg: &CarConfig) -> Option<CarPrep> {
        let key = parsed_key(dir);
        let i = self.preps.iter().position(|(k, c, _)| *k == key && c == cfg)?;
        match self.preps.remove(i).2 {
            PrepSlot::Ready(p) => Some(p),
            PrepSlot::Making(h) => h.join().ok(),
        }
    }

    /// The build of `dir` in `cfg`: its CPU part (`prep_car`) made on a worker thread now, for `load_model_cfg` to
    /// take - when the car has been read (`parsed`) and the build is neither made nor being made.
    fn prepare_ahead(&mut self, dir: &Path, cfg: &CarConfig) {
        let key = parsed_key(dir);
        let name = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if self.models.iter().any(|m| m.name == name && m.cfg == *cfg) || self.preps.iter().any(|(k, c, _)| *k == key && c == cfg) {
            return;
        }
        let Some(Parsed::Read(car, _)) = self.parsed.get(&key) else { return };
        let (car, d, c) = (car.clone(), dir.to_path_buf(), cfg.clone());
        if let Ok(h) = std::thread::Builder::new().name("car prep".into()).spawn(move || Vehicles::prep_car(&car, &d, &c)) {
            self.keep_prep(key, cfg.clone(), PrepSlot::Making(h));
        }
    }

    /// The car in `dir` as read: from `parsed` (waiting for its worker if it is still reading), else read now.
    fn car_asset(&mut self, world: &World, dir: &Path) -> Result<Arc<CarAsset>> {
        self.parsed_stamp += 1;
        let stamp = self.parsed_stamp;
        let key = parsed_key(dir);
        let car = match self.parsed.remove(&key) {
            Some(Parsed::Read(c, _)) => c,
            Some(Parsed::Reading(h, _)) => {
                let (c, p) = h.join().map_err(|_| anyhow::anyhow!("car read thread panicked"))??;
                self.keep_prep(key.clone(), CarConfig::default(), PrepSlot::Ready(p));
                Arc::new(c)
            }
            None => Arc::new(vehicle_model::load_car(&world.assets, dir)?),
        };
        self.parsed.insert(key, Parsed::Read(car.clone(), stamp));
        // (the oldest read ones go past PARSED_KEEP)
        loop {
            let read: Vec<(String, u64)> = self.parsed.iter().filter_map(|(k, v)| if let Parsed::Read(_, t) = v { Some((k.clone(), *t)) } else { None }).collect();
            let Some(old) = read.iter().min_by_key(|(_, t)| *t).filter(|_| read.len() > PARSED_KEEP) else { break };
            self.parsed.remove(&old.0);
        }
        Ok(car)
    }

    pub fn new() -> Vehicles {
        Vehicles { traffic_target: 0, rng: 12345, ..Default::default() }
    }

    /// The camera's eye this frame, for the wheels' side test (`upload_uniforms`).
    pub fn set_wheel_eye(&mut self, eye: Vec3) {
        self.wheel_eye = eye;
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

    /// The model of `dir` with `cfg` as loaded before (`replace_player`'s lookup), else loaded now.
    pub fn model_cached(&mut self, gpu: &mut Gpu, world: &mut World, dir: &Path, cfg: &CarConfig) -> Result<Arc<CarModel>> {
        let name = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        match self.models.iter().find(|m| m.name == name && m.cfg == *cfg) {
            Some(m) => Ok(m.clone()),
            None => self.load_model_cfg(gpu, world, dir, cfg),
        }
    }

    pub fn load_model_cfg(&mut self, gpu: &mut Gpu, world: &mut World, dir: &Path, cfg: &CarConfig) -> Result<Arc<CarModel>> {
        let t_load = std::time::Instant::now();
        let car = self.car_asset(world, dir)?;
        let ms_parse = t_load.elapsed().as_secs_f32() * 1000.0;
        let CarPrep { car_is_ambient, meshes, lamp_acc, own_hubs, glow_bones, glow_sides, font_bones, variants, variant_labels, extra_variants, part_rig, hood_slot, trunk_bone, slot_bones, roof_place, upgrade } = match self.take_prep(dir, cfg) {
            Some(p) => p,
            None => Self::prep_car(&car, dir, cfg),
        };
        let mut batches = Vec::new();
        let mut merged: hashbrown::HashMap<(u32, u32, u32), MergeAcc> = hashbrown::HashMap::new();
        // (meshes sharing textures and material mode are merged into one draw, in build order: `prep_car`)
        for m in meshes {
            let resolve = |gpu: &mut Gpu, world: &mut World, name: Option<&str>| -> (Arc<GpuTexture>, u32) {
                let Some(name) = name else { return (gpu.white.clone(), 0) };
                let h = mcla_formats::hash::joaat(name);
                let hk = if car_tex_raw() { h ^ 0x0052_4157 } else { h };
                if let Some(t) = gpu.textures.get(&hk) {
                    return (t.clone(), hk);
                }
                if let Some(t) = car.textures.get(&h) {
                    log::debug!("car texture {} from car dict: {}x{} {:?} name {:?} levels {}", name, t.width, t.height, t.format, t.name, t.levels.len());
                    return car_upload(gpu, h, t);
                }
                if let Some(t) = car_any(gpu, world, h) {
                    return (t, hk);
                }
                log::debug!("car texture miss: {} ({:#010x})", name, h);
                crate::world::note_tex_miss("car", name);
                (gpu.white.clone(), 0)
            };
            let (d, k0) = if m.plate { plate_texture(gpu, world) } else { resolve(gpu, world, m.dn.as_deref()) };
            // (a badge's bump map, PS_BumpedChrome's NormalSampler: its fetch constant asks for no gamma - word 0 of the
            // trim atlas, 0x84000002, in the recorded frame - so the program filters the texels as stored; uploaded as
            // they are, under a key of its own. MCLA_BADGE_GAMMA_OLD=1: the gamma-decoded upload re-encoded in city.wgsl, as before)
            let (d, k0) = if m.sname.eq_ignore_ascii_case("BumpedAlphaChrome") && std::env::var("MCLA_BADGE_OLD").is_err() && std::env::var("MCLA_BADGE_GAMMA_OLD").is_err() {
                match m.dn.as_deref().map(mcla_formats::hash::joaat).and_then(|h| car.textures.get(&h).map(|t| (h, t))) {
                    Some((h, t)) => (gpu.upload_texture_raw(h ^ 0x0052_4157, t), h ^ 0x0052_4157),
                    None => (d, k0),
                }
            } else {
                (d, k0)
            };
            let (s2, k1) = resolve(gpu, world, m.sn.as_deref());
            let (s2, k1) = if m.carbon { carbon_weave(gpu, world) } else { (s2, k1) };
            let win_maps = if m.win_cand { glass_maps(gpu, world) } else { None };
            let ((d, k0), (s2, k1)) = win_maps.clone().unwrap_or(((d, k0), (s2, k1)));
            let geo = match (win_maps.is_none(), m.geo_alt) {
                (true, Some(g)) => g,
                _ => m.geo,
            };
            let (sname, mode) = (m.sname, m.mode);
            let acc = merged.entry((k0, k1, m.kbits)).or_insert_with(|| MergeAcc { verts: Vec::new(), skin: Vec::new(), indices: Vec::new(), d, s2, sname, mode, min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) });
            acc.append(geo);
        }
        let mut keys: Vec<(u32, u32, u32)> = merged.keys().copied().collect();
        let mut batch_window: Vec<u8> = Vec::new();
        let mut roof_skin: Vec<(usize, Vec<Vertex>, Vec<([u16; 4], [f32; 4])>)> = Vec::new();
        let mut lods: [(Vec<GpuBatch>, Vec<u8>); 2] = Default::default();
        keys.sort_by_key(|k| (k.2 & !0x7000, k.0, k.1, k.2 & 0x7000));
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
            // (the skinned roof's vertices are written again while it moves: `Vehicles::upload_uniforms`)
            let skinned = !acc.skin.is_empty() && acc.skin.len() == acc.verts.len() && (k.2 >> 24) & 3 == 0;
            let usage = if skinned { wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST } else { wgpu::BufferUsages::VERTEX };
            let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&acc.verts), usage });
            if skinned {
                roof_skin.push((batches.len(), acc.verts.clone(), acc.skin.clone()));
            }
            let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&acc.indices), usage: wgpu::BufferUsages::INDEX });
            // (bit 31: a traffic mesh of the blended bucket, bits 26 - 30 its place among them - `prep_car`)
            let (group, lod) = ((k.2 >> 16) as u8, ((k.2 >> 24) & 3) as usize);
            if group != 0 {
                log::debug!("car {}: moving group {} (1/2 windows, 3 hood, 4 trunk, 5/6 bumpers) batch shader {} tris {}", car.name, group, acc.sname, acc.indices.len() / 3);
            }
            let b = GpuBatch { shader: acc.sname, vb, ib, count: acc.indices.len() as u32, bind, mode: acc.mode, min: acc.min, max: acc.max, bucket: (k.2 >> 31) as u8, cull: ((k.2 >> 12) & 7) as u8 };
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
            // (F-55728: the air ride - the base's Rise and rates, the drops from the wheels)
            let mut base = s.base.clone();
            base.set_air_ride(s);
            let mut vt = crate::physics::vehsim::VehTune::new(base);
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
                params.jolt = d.jolt_threshold;
            }
        }
        log::info!("car {}: hubs {:?} bounds {:?}..{:?} wheel radius {:.3}", car.name, car.hubs, car.min, car.max, params.wheel_radius);
        let params_mods = car_params_mods(&car, &params);
        log::info!("car model {}: {} batches (LOD 1 {}, LOD 2 {}), mass {:.0}, built in {:.0} ms ({:.0} reading the files)", car.name, batches.len(), lods[0].0.len(), lods[1].0.len(), params.mass, t_load.elapsed().as_secs_f32() * 1000.0, ms_parse);
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
        // (a bike's rim: 0.82 of its tyre's diameter, the constructor F-60786's +4084 / +4088, F-43091)
        let rim_ratio = if car.bike { 0.82 } else { stock_rim_ratio(&world.assets.root, &car.name, params.wheel_radius) };
        let mut wheels = None;
        let mut rim_dir = None;
        // (a mod car's stock rims: its own folder or its manifest's `donors.rims` - modcars.rs)
        let mod_rims = crate::modcars::car(&car.name).and_then(|c| c.rims.clone());
        for dir in mod_rims.into_iter().chain(cands.iter().flatten().map(|c| rims_root.join(c))) {
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
        let brakes = {
            let root = world.assets.root.join("resources/brakes");
            let own = [base_name.strip_prefix("vpd_").map(|n| format!("brkd_stk_{n}")), base_name.strip_prefix("vp_").map(|n| format!("brk_stk_{n}"))];
            // (a mod car: its manifest's `donors.brake` - modcars.rs)
            let dir = crate::modcars::brake_dir(&car.name).into_iter().chain(own.iter().flatten().map(|n| root.join(n))).find(|d| d.is_dir()).unwrap_or_else(|| root.join("brk_car_base"));
            let key = (dir.clone(), 0, 0);
            brake_dir = Some(dir.clone());
            match self.brake_cache.get(&key) {
                Some(b) => Some(b.clone()),
                None => {
                    let b = load_brakes(gpu, world, &dir).map(Arc::new);
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
        let bound_box = if car_is_ambient { traffic::bound_box(&world.assets.root, &car.name) } else { None };
        let (fx_anchors, fx_anchor_rear) = car.fx_anchors(cfg);
        // (F-74884, a bike's set-up: +4092 / +4096 = |max - min| of the translation limits, crBoneData +144 / +160, of the
        // first bone named wheel_0 / wheel_2 in the base fragment's skeleton, then the parts' - F-20921)
        let bike_wheel_width = ["wheel_0", "wheel_2"].map(|n| car.skeleton.as_ref().and_then(|s| s.bones.iter().find(|b| b.name == n)).map_or(params.wheel_width, |b| (b.trans_limits.1 - b.trans_limits.0).length()));
        let m = Arc::new(CarModel { lods, lod_radius, lod_centre, lod_steps, bound_box, own_wheels, name: car.name.clone(), lamps, glow_bones, glow_sides, font_bones, batches, params, params_mods, simtunes, vehmods: car.vehmods.clone(), fit: car.fit, aitunes: car.aitune.clone(), rest_height: car.hubs.map(|h| (h[0].y + h[2].y) * 0.5), wheels, brakes, brake_dir, rim_dir, rim_ratio, bike: car.bike, bike_wheel_width, min: car.min, max: car.max, cfg: cfg.clone(), variants, variant_labels, extra_variants, batch_window, windows: car.windows.clone(), spoiler: car.spoiler.clone().map(Arc::new), clamshell: car.clamshell.clone().filter(|_| car.skinned_top.is_some()).map(Arc::new), skinned_top: car.skinned_top.clone().filter(|_| car.clamshell.is_some()).map(Arc::new), needles: [car.needles[0].clone().map(Arc::new), car.needles[1].clone().map(Arc::new), upgrade.as_ref().map(|u| u.0.clone())], needle_place: upgrade.as_ref().map_or(Mat4::IDENTITY, |u| u.1), roof_skin, roof_place, cop_lights: car.cop_lights.clone().map(Arc::new), damage_layout: car.damage_layout.clone().map(Arc::new), part_rig, hood_slot, trunk_bone, slot_bones, fx_anchors, fx_anchor_rear });
        self.models.push(m.clone());
        if self.wheels.is_none() {
            self.wheels = load_wheels(gpu, world).map(Arc::new);
        }
        Ok(m)
    }

    /// The CPU part of a car's build (`load_model_cfg`): its meshes placed, merged by program and marked, in build
    /// order, with the lamp places, the wheel hubs and glow bones gathered on the way - without the GPU or the world, so
    /// that it can run on a worker thread (docs/PERFORMANCE.md 2026-10-27 (14)). The textures are found, and the
    /// batches made and uploaded, by `load_model_cfg` on the main thread.
    fn prep_car(car: &CarAsset, dir: &Path, cfg: &CarConfig) -> CarPrep {
        let car_is_ambient = dir.file_name().map_or(false, |n| n.to_string_lossy().to_ascii_lowercase().starts_with("va_"));
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
        // (the light set's order: its slot rank, then the skeleton's order - see the cap below)
        let mut glow_ranked: Vec<(usize, u8, Vec3, Vec3, u8)> = Vec::new();
        let mut font_bones: Vec<(u8, Mat4, f32, f32)> = Vec::new();
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
        let trunk_tag = trunk_i.and_then(|i| sk.map(|s| s.bones[i].tag)).filter(|t| *t != 0);
        let trunk_tags: Vec<u16> = trunk_set.iter().filter_map(|&i| sk.map(|s| s.bones[i].tag)).filter(|t| *t != 0).collect();
        let trunk_worlds: Vec<Mat4> = trunk_set.iter().filter_map(|&i| sk.map(|s| s.bones[i].world)).collect();
        // (each moved slot's part is moved about the bone it hangs on, its matrix index - F-14987; a part without
        // one keeps the port's slot bone)
        let fitted = car.placed_parts_for(cfg);
        let own_bone = |f: &dyn Fn(&str) -> bool| fitted.iter().find(|(p, _)| !p.is_base && f(&p.primary.to_ascii_lowercase()) && sk.map_or(false, |s| (p.matrix_index as usize) < s.bones.len())).map(|(_, x)| *x);
        let hood_slot = own_bone(&|p| p.starts_with("hood") && !p.contains("scoop")).or_else(|| bone_world(vehicle_model::slot_tag(Slot::Hood).unwrap_or(0)));
        let trunk_bone = trunk_i.and_then(|i| sk.map(|s| s.bones[i].world));
        let part_rig = crate::damageparts::PartRig {
            movement: car.simtune.as_ref().and_then(|s| s.part_movement).unwrap_or(crate::damageparts::DEFAULT_MOVEMENT),
            bumper_rest: [bone_world(13699), bone_world(13687)],
        };
        let slot_bones = [
            own_bone(&|p| p.starts_with("bumper_r")).or(part_rig.bumper_rest[0]),
            own_bone(&|p| p.starts_with("bumper_f")).or(part_rig.bumper_rest[1]),
            own_bone(&|p| p.starts_with("intercooler")).or(part_rig.bumper_rest[1]),
        ];
        // (the exhaust tips are drawn at the outlet bones of the part that holds them, as those bones stand:
        // they move with that part - a hanging rear bumper takes them along)
        let outlet_src = car.outlet_part(cfg).map(|o| (o.primary.to_ascii_lowercase(), o.is_base, o.transform));
        let spoiler_bones: Vec<usize> = car.spoiler.as_ref().map(|c| c.bones.clone()).unwrap_or_default();
        // (a convertible whose roof moves - both clips there: the three roof parts and the clamshell's bones are groups)
        let roof_moves = car.clamshell.is_some() && car.skinned_top.is_some();
        let clam_bones: Vec<usize> = car.clamshell.as_ref().filter(|_| roof_moves).map(|c| c.bones.clone()).unwrap_or_default();
        let needle_bones: [Vec<usize>; 2] = std::array::from_fn(|k| car.needles[k].as_ref().map(|c| c.bones.clone()).unwrap_or_default());
        // (the fitted add-on gauge's needle: `tach_upgrade` of the post_gauge part's own file, on its own skeleton; of
        // the gauge's two files only one has the clip)
        let upgrade: Option<(Arc<vehicle_model::BoneClip>, Mat4, std::path::PathBuf)> = fitted.iter().filter(|(p, _)| !p.is_base && p.primary.to_ascii_lowercase().starts_with("post_gauge")).find_map(|(p, x)| {
            let psk = p.skeleton.as_ref()?;
            let res = crate::assets::load_resource_file(&p.file).ok()?;
            vehicle_model::BoneClip::load(&res, psk, "tach_upgrade").map(|c| (Arc::new(c), *x, p.file.clone()))
        });
        if let Some(u) = upgrade.as_ref() {
            log::info!("car {}: add-on gauge needle tach_upgrade {:.3} s, {} bones", car.name, u.0.duration, u.0.bones.len());
        }
        let part_group = |part: &vehicle_model::CarPart, part_xf: &Mat4, mi: usize| -> u32 {
            if let Some(kind) = vehicle_model::roof_part(&part.primary).filter(|_| roof_moves) {
                return ROOF_GROUP + kind as u32;
            }
            let mut p = part.primary.to_ascii_lowercase();
            let mut part_xf = *part_xf;
            if p.starts_with("exh_pipe") {
                match &outlet_src {
                    Some((name, false, xf)) => {
                        p = name.clone();
                        part_xf = *xf;
                    }
                    _ => return 0,
                }
            }
            let part_xf = &part_xf;
            // (a mesh of the add-on gauge on a bone its `tach_upgrade` moves)
            if let Some((cl, _, _)) = upgrade.as_ref().filter(|u| !part.is_base && u.2 == part.file) {
                let b = part.mesh_tag.get(mi).and_then(|&t| part.skeleton.as_ref().and_then(|s| s.bone_by_tag(t)));
                if let Some(j) = b.and_then(|b| cl.bones.iter().position(|&x| x == b)).filter(|&j| j < NEEDLE_BONES) {
                    return NEEDLE_GROUP + (2 * NEEDLE_BONES + j) as u32;
                }
            }
            // (on a bone the auto spoiler's clip moves: a base mesh bound to it, a part hung on it by its matrix index)
            let bone = if part.is_base { part.mesh_tag.get(mi).and_then(|&t| sk.and_then(|s| s.bone_by_tag(t))) } else { Some(part.matrix_index as usize) };
            if let Some(j) = bone.and_then(|b| spoiler_bones.iter().position(|&x| x == b)).filter(|&j| j < SPOILER_GROUPS) {
                return 21 + j as u32;
            }
            // (on a bone the roof's clamshell moves; a bone of the spoiler's too - the Gallardo's spoiler bone - stays the
            // spoiler's)
            if let Some(j) = bone.and_then(|b| clam_bones.iter().position(|&x| x == b)).filter(|&j| j < CLAM_GROUPS) {
                return CLAM_GROUP + j as u32;
            }
            // (on a bone a dashboard needle's clip moves)
            for (k, nb) in needle_bones.iter().enumerate() {
                if let Some(j) = bone.and_then(|b| nb.iter().position(|&x| x == b)).filter(|&j| j < NEEDLE_BONES) {
                    return NEEDLE_GROUP + (k * NEEDLE_BONES + j) as u32;
                }
            }
            if part.is_base {
                return if part.mesh_tag.get(mi).map_or(false, |t| trunk_tags.contains(t)) { 4 } else { 0 };
            }
            if p.starts_with("hood") && !p.contains("scoop") && hood_slot.is_some() {
                3
            } else if p.starts_with("bumper_r") && part_rig.bumper_rest[0].is_some() {
                5
            } else if p.starts_with("bumper_f") && part_rig.bumper_rest[1].is_some() {
                6
            } else if p.starts_with("intercooler") && part_rig.bumper_rest[1].is_some() {
                20
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
        let mut meshes: Vec<MeshPrep> = Vec::new();
        // (a traffic model's LOD 1 and LOD 2: its other two base fragments, built as LOD 0's body is)
        let mut todo: Vec<(&vehicle_model::CarPart, Mat4, u32)> = car.placed_parts_for(cfg).into_iter().map(|(p, x)| (p, x, 0)).collect();
        // (the roof parts the configuration leaves out: drawn by the roof players' times - `RoofCtl::shown`)
        if roof_moves {
            todo.extend(car.roof_rest(cfg).into_iter().map(|(p, x)| (p, x, 0)));
        }
        let roof_place = todo.iter().find(|(p, _, _)| vehicle_model::roof_part(&p.primary) == Some(2)).map_or(Mat4::IDENTITY, |(_, x, _)| *x);
        if car_is_ambient {
            for (lod, h) in [(1u32, "7b0cd281"), (2, "0de3ddb3")] {
                if let Some(p) = car.lod_parts.iter().find(|p| p.primary.contains(h)) {
                    todo.push((p, p.transform, lod));
                }
            }
        }
        // (a traffic model's meshes of the blended bucket, counted per LOD: their draw order - `Vehicles::draw`)
        let mut b1_seq = [0u32; 4];
        let hide_plate = cfg.no_plate.unwrap_or_else(|| crate::modcars::hides_plate(&car.name));
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
            if let Some(sk) = part.skeleton.as_deref().filter(|_| lod == 0) {
                for (bi, b) in sk.bones.iter().enumerate() {
                    let parent = sk.bones.get(b.parent).map_or(String::new(), |q| q.name.to_ascii_lowercase());
                    let w = (part_xf * b.world).w_axis.truncate();
                    if lamp_dump {
                        eprintln!("lampbone {} {} {} {} parent {} {:.4} {:.4} {:.4} z {:.3} {:.3} {:.3}", car.name, part.primary, bi, if b.name.is_empty() { "-" } else { b.name.as_str() }, if parent.is_empty() { "-" } else { parent.as_str() }, w.x, w.y, w.z, b.world.z_axis.x, b.world.z_axis.y, b.world.z_axis.z);
                    }
                    if let Some(kind) = glow_bone_kind(&b.name.to_ascii_lowercase()) {
                        // (the light set's +24 side, F-27051 through F-75563: 1 for a bone in slot 6 bumper_f, 2 in slot 3
                        // bumper_r, else 0 - the base skeleton is in no slot)
                        let pn = part.primary.to_ascii_lowercase();
                        let side = if part.is_base { 0 } else if pn.starts_with("bumper_f") { 1 } else if pn.starts_with("bumper_r") { 2 } else { 0 };
                        glow_ranked.push((vehicle_model::outlet_rank(part), kind, w, -(part_xf * b.world).z_axis.truncate().normalize_or_zero(), side));
                    }
                    if let Some(k) = crate::readouts::bone_kind(&b.name).filter(|k| !font_bones.iter().any(|f| f.0 == *k)) {
                        let (lo, hi) = b.trans_limits;
                        font_bones.push((k, part_xf * b.world, hi.x - lo.x, hi.y - lo.y));
                    }
                }
            }
            for (mi, (mesh, si)) in part.meshes.iter().enumerate() {
                let bone_m = part.mesh_bone.get(mi).copied().unwrap_or(Mat4::IDENTITY);
                let sh = if part.from_parent && !car.shaders_parent.is_empty() { car.shaders_parent.get(*si as usize) } else { car.shaders.get(*si as usize) };
                let sname = sh.map(|s| s.name.as_str()).unwrap_or("");
                // (CarbonFiberHood: the hood's painted meshes - part slots hood and hood_wide - are
                // drawn by the carbon fibre program in place of the paint, F-92759 at A-17085.
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
                // (NO PLATE, the user's deviation D28: the plate meshes are left out - `CarConfig::no_plate`, a mod car's
                // `hide_plate` by default)
                if hide_plate && sname.eq_ignore_ascii_case("LicensePlate") {
                    continue;
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
                // (a traffic tyre: PSAmbientTire reads its NormalSampler map - the second slot, city.wgsl)
                let sn = if car_is_ambient && sname.eq_ignore_ascii_case("AmbientTire") { sh.and_then(|s| s.textures().find(|(h, _)| NORMAL_SAMPLERS.contains(h)).map(|(_, n)| n.to_string())).or(sn) } else { sn };
                if log::log_enabled!(log::Level::Debug) && dn.is_none() && !all_tex.is_empty() {
                    log::debug!("shader {} has only normal maps: {:?}", sname, all_tex);
                }
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
                // (the maps are found at the build, on the main thread - `load_model_cfg`: which ones)
                let plate = sname.eq_ignore_ascii_case("LicensePlate") && dn.is_none();
                // (render only, blade round - PS_CarbonFiber's NormalSampler, the weave's normal map, rides as the CarbonFiber
                // meshes' second texture; city.wgsl reads it in the carbon branch. MCLA_CARBON_OLD=1: none)
                let carbon = sname.eq_ignore_ascii_case("CarbonFiber") && !car_is_ambient && std::env::var("MCLA_CARBON_OLD").is_err();
                // (the windows of a car with the full glass program, PS_GlassFront: the detail mask and the
                // crack map in the two slots, marked by lens -1 below; city.wgsl reads them for a cracked
                // or broken pane and draws the pane white as before)
                let win_cand = sname.eq_ignore_ascii_case("CarGlass") && !car_is_ambient && dn.is_none() && sn.is_none();
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
                // (the light bar's lens meshes: each bound to one of the cop_lights clip's pivots - groups 13 - 19)
                let win_side = match car.cop_lights.as_ref().filter(|_| part.is_base).and_then(|cl| cl.lamp_of_tag(*part.mesh_tag.get(mi)?)) {
                    Some(li) => 13 + li as u32,
                    None => win_side,
                };
                if win_side >= 3 && std::env::var("MCLA_PARTS_LOG").is_ok() {
                    log::info!("damage parts {}: {} mesh {} -> group {}", car.name, part.primary, mi, win_side);
                }
                // The cull class (`GpuBatch::cull`). A set-up car's draw (F-92759) culls back faces (render state 0
                // through F-33110: 2, or 1 in a mirrored view) but for the slots hood / hood_wide (2, 40), always; the
                // bumpers bumper_f / bumper_r (6, 3) while an end of theirs hangs or is torn (+512 / +516 of the bumper
                // object); trunk_swap / trunk_wide (4, 43) and the base fragment's meshes on the `trunk_0` bone itself
                // (the second mask its constructor F-60786 builds by F-86379, drawn by F-28289) when the model tune's
                // bDoubleSidedTrunk is set. BlackOpaqueGlass draws both sides wherever it is (cull 0x18000 on its every
                // draw, recorded: work/police17 cap/front240). (MCLA_CARCULL_OLD=1: both sides, as before)
                let cull = if car_is_ambient || std::env::var("MCLA_CARCULL_OLD").is_ok() {
                    0
                } else {
                    let slot = part.primary.to_ascii_lowercase();
                    let slot = slot.trim_end_matches(|c: char| c.is_ascii_digit());
                    let ds_trunk = car.simtune.as_ref().and_then(|t| t.double_sided_trunk).unwrap_or(false);
                    let on_trunk = part.is_base && trunk_tag.is_some_and(|t| part.mesh_tag.get(mi) == Some(&t));
                    if sname.eq_ignore_ascii_case("BlackOpaqueGlass") || slot == "hood" || slot == "hood_wide" || ds_trunk && (on_trunk || slot == "trunk_swap" || slot == "trunk_wide") {
                        2
                    } else if slot == "bumper_f" {
                        3
                    } else if slot == "bumper_r" {
                        4
                    } else {
                        1
                    }
                };
                if std::env::var("MCLA_PARTS_LOG").is_ok() && cull != 1 {
                    log::info!("cull class {} {}: {} mesh {} {} tris", cull, car.name, part.primary, mi, mesh.indices.len() / 3);
                }
                let kbits = mode as u32 | cull << 12 | win_side << 16 | lod << 24;
                // The draw bucket, header byte +0x09 of the shader (`Shader::bucket`): grmModel's draw (F-73936)
                // draws a geometry only in the pass whose bucket number equals its shader's, and the traffic car's
                // lit draw (F-73627) asks for bucket 0 in the main opaque pass (2 at LOD 2) and bucket 1 in the main
                // alpha pass (bucket lists F-80884 / F-48841). So a traffic mesh of bucket 1 - the decals, logos,
                // the diamond panels, the plate, the glass - is drawn after every opaque, blended, in its model's
                // order: such meshes are not merged with others (bit 31 + their place, bits 26 - 30, in the key;
                // the keys are drawn in sorted order). (MCLA_TRAFFIC_BUCKET_OLD=1: every mesh by its mode, as before)
                let bucket1 = car_is_ambient && sh.map_or(0, |s| s.bucket) == 1 && std::env::var("MCLA_TRAFFIC_BUCKET_OLD").is_err();
                // A set-up car (the player's, a racer's, a cop's) draws its bucket 1 the same way, straight after its
                // bucket 0 (recorded frame work/police17/cap/g.gpu.bin, the cruiser: draws 1883 - 1895 after 1867 - 1882):
                // blended SRC_ALPHA / INV_SRC_ALPHA, depth writes on, alpha test GREATER 4/255 (0x07060706 / 0x00708767 /
                // 0x8700000c, RB_ALPHA_REF 0x3c808081). The BumpSpec family's alpha is the texture's (o0.w = gLightAmbient.w
                // x gInvColorExpBias.x x A, the bias 1/16 undoing the target's x16), the badge's 2A - 1: the light bar's
                // domes (BumpSpecAlpha, A = 0.69) show the lit lenses under them. (MCLA_SETUP_BUCKET_OLD=1: cut out, opaque)
                let setup_b1 = !car_is_ambient
                    && sh.map_or(0, |s| s.bucket) == 1
                    && (["BumpSpec", "BumpSpecAlpha", "GrillOpaque"].iter().any(|n| sname.eq_ignore_ascii_case(n)) || badge)
                    && std::env::var("MCLA_SETUP_BUCKET_OLD").is_err();
                let bucket1 = bucket1 || setup_b1;
                let kbits = if bucket1 {
                    let li = (lod as usize).min(3);
                    b1_seq[li] += 1;
                    kbits | 1 << 31 | (b1_seq[li] - 1).min(31) << 26
                } else {
                    kbits
                };
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
                let lens_of = |win_maps: bool| {
                    if (glass || lamp) && (pn.contains("tail") || pn.contains("brake")) {
                        1.0
                    } else if glass {
                        2.0
                    } else if win_maps {
                        -1.0
                    } else {
                        0.0
                    }
                };
                // (the mesh's vertices as the batch takes them: placed, then marked by its program's rules)
                let make = |lens: f32| -> MeshGeo {
                    let mut acc = MeshGeo::new();
                    let first = 0;
                    acc.push(mesh, part_xf * bone_m, dark_above, lens, if !car_is_ambient { 0 } else if sname.eq_ignore_ascii_case("AmbientTire") { 2 } else { 1 });
                    // (render only - car parts drawn by their own programs' rules in city.wgsl, see
                    // work/lighting bs3.py / bmprobe.py / cglassprobe.py: the material's terms ride in
                    // the vertices. BumpSpec family: uv2 = (-(1 + 200 smoothness + 256 x 200
                    // SpecularColor), 1 + base r + 256 base g), uv1 = (base b + 256 x 32 SpecMapExp, 0),
                    // colour alpha = 10 metallic + 16 cut-out + 32 cabin part + 64 GrillOpaque + 128 blended bucket (its
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
                            let a = (by(m, 10.0).min(15.0) + if alpha_cut { 16.0 } else { 0.0 } + if cabin { 32.0 } else { 0.0 } + if spec_map { 64.0 } else { 0.0 } + if setup_b1 { 128.0 } else { 0.0 }) as u8;
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
                        } else if sname.to_ascii_lowercase().contains("blackmatte") && std::env::var("MCLA_BLACKMATTE_OLD").is_err() {
                            // (PS_BlackMatte of BlackMatte.fx writes only o0.w = gLightAmbient.w - black, as the traffic
                            // cars' PSAmbientBlackMatte; city.wgsl: uv2.x = 4)
                            for v in &mut acc.verts[first..] {
                                v.uv2 = [4.0, 0.0];
                            }
                        } else if sname.eq_ignore_ascii_case("BlackOpaqueGlass") {
                            for v in &mut acc.verts[first..] {
                                v.uv2 = [1.0, 0.0];
                            }
                        } else if badge {
                            // (BumpedAlphaChrome, PS_BumpedChrome of cars/0x1d68d869: PS_Chrome's mirror x
                            // (0.125 + 1.185 sat(collector - 0.25)) x baked occlusion, alpha = 2 x the
                            // texture's alpha - 1; city.wgsl cuts it out where that is not above 0 (uv2.y = 3))
                            // (uv2.x = -1: drawn in the blended bucket, alpha 2A - 1 - `setup_b1`)
                            // (and its stored tangent in the car's space in the colour's rgb, t x 0.5 + 0.5, as a window's:
                            // VS_BumpedChrome hands over N, T and cross(T, N) x w, the pixel program tilts the normal by
                            // the map in that frame; the badge's program reads no own-lamp weights from the colour)
                            let bxf = part_xf * bone_m;
                            for (i, v) in acc.verts[first..].iter_mut().enumerate() {
                                v.uv2 = [if setup_b1 { -1.0 } else { 0.0 }, 3.0];
                                if setup_b1 && mesh.tangents.len() == mesh.positions.len() && std::env::var("MCLA_BADGE_BUMP_OLD").is_err() {
                                    let t = bxf.transform_vector3(mesh.tangents[i].truncate()).normalize_or_zero();
                                    if t != Vec3::ZERO {
                                        let e = |x: f32| ((x * 0.5 + 0.5) * 255.0).round() as u8;
                                        v.color = [e(t.x), e(t.y), e(t.z), v.color[3]];
                                        // (-3: and its map uploaded as stored, load_model_cfg)
                                        v.uv2[0] = if std::env::var("MCLA_BADGE_GAMMA_OLD").is_err() { -3.0 } else { -2.0 };
                                    }
                                }
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
                    // (traffic: PSAmbientBumpSpec of cars/0x2a82f172, ported in city.wgsl - the material's terms in the
                    // vertices: uv2 = (-(1 + 255 smoothness + 256 x 255 metallic), -(1 + 4095 base r + 4096 x 4095 base g)),
                    // uv1 = (-(1 + 4095 base b + 4096 x 4095 SpecularColor), -(1 + 1023.75 SpecMapExp + 4096 x 1023.75
                    // ambientMod)), colour alpha 255 on a mesh of the blended bucket (its texture's alpha is then the
                    // pixel's); uv0 x detailTiling as VSAmbientBumpSpec hands it on (o4)
                    if car_is_ambient && sname.eq_ignore_ascii_case("AmbientBumpSpec") && std::env::var("MCLA_TRAFFIC_BS_OLD").is_err() {
                        use mcla_formats::rage::drawable::ShaderParam;
                        let pv = |h: u32, d: f32| sh.and_then(|s| s.params.iter().find_map(|(ph, p)| match p { ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0]), _ => None })).unwrap_or(glam::Vec4::splat(d));
                        let q = |x: f32, k: f32| if x.is_finite() { (x.abs() * k).round().clamp(0.0, k) } else { 0.0 };
                        let (bc, am, e, k, m, sm, dt) = (pv(0x1468b111, 1.0), pv(0xfd110c9f, 1.0).x, pv(0xb37a7946, 1.0).x, pv(0xa98e2243, 0.0).x, pv(0x83697767, 0.0).x, pv(0x5e315fd1, 0.0).x, pv(0x250f72fc, 1.0).x);
                        let blended = sh.map_or(0, |s| s.bucket) == 1;
                        for v in &mut acc.verts[first..] {
                            v.uv2 = [-(1.0 + q(sm, 255.0) + 256.0 * q(m, 255.0)), -(1.0 + q(bc.x, 4095.0) + 4096.0 * q(bc.y, 4095.0))];
                            v.uv1 = [-(1.0 + q(bc.z, 4095.0) + 4096.0 * q(k, 4095.0)), -(1.0 + q(e / 4.0, 4095.0) + 4096.0 * q(am / 4.0, 4095.0))];
                            v.color[3] = if blended { 255 } else { 0 };
                            if dt.is_finite() {
                                v.uv0 = [v.uv0[0] * dt, v.uv0[1] * dt];
                            }
                        }
                    }
                    // (traffic tyres: PSAmbientTire of cars/0xed26c1fb, ported in city.wgsl - its terms in the vertices:
                    // uv2.y = -(1 + 1023.75 AmbientIntensity + 4096 x 1023.75 DiffuseIntensity), uv1 = (-(1 + 63.98
                    // SpecularExponentLow + 4096 x 63.98 SpecularExponentHigh), -(1 + 1023.75 SpecularIntensity)))
                    if car_is_ambient && sname.eq_ignore_ascii_case("AmbientTire") {
                        use mcla_formats::rage::drawable::ShaderParam;
                        let pv = |h: u32, d: f32| sh.and_then(|s| s.params.iter().find_map(|(ph, p)| match p { ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0].x), _ => None })).filter(|x| x.is_finite()).unwrap_or(d);
                        let q = |x: f32, k: f32| (x.abs() * k).round().clamp(0.0, 4095.0);
                        let (ai, di, lo, hi, si) = (pv(0x3098c6f5, 1.0), pv(0xf7ba0d6e, 1.0), pv(0xe59064e1, 1.0), pv(0xea99b41e, 40.0), pv(0xa95fc535, 1.0));
                        for v in &mut acc.verts[first..] {
                            v.uv2[1] = -(1.0 + q(ai, 4095.0 / 4.0) + 4096.0 * q(di, 4095.0 / 4.0));
                            v.uv1 = [-(1.0 + q(lo, 4095.0 / 64.0) + 4096.0 * q(hi, 4095.0 / 64.0)), -(1.0 + q(si, 4095.0 / 4.0))];
                        }
                    }
                    // cabin trim: the colour group of the mesh's zone (uv1.x), by the vehicle's own
                    // table, rides in uv2.y as 100 + group (city.wgsl, mode 17; nothing else of that
                    // mode reads uv2.y) - the original builds the trim program's colour table from
                    // the same table every frame (F-19629)
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
                    acc
                };
                let mut geo = make(lens_of(win_cand));
                let mut geo_alt = (win_cand && lens_of(false) != lens_of(true)).then(|| make(lens_of(false)));
                // (the skinned roof: each vertex's bones of the roof's own skeleton)
                if roof_moves && lod == 0 && vehicle_model::roof_part(&part.primary) == Some(2) {
                    if let Some(skin) = vehicle_model::roof_vertex_bones(part, mi) {
                        if geo.verts.len() == skin.len() {
                            geo.skin = skin.clone();
                        }
                        if let Some(g) = geo_alt.as_mut().filter(|g| g.verts.len() == skin.len()) {
                            g.skin = skin;
                        }
                    }
                }
                meshes.push(MeshPrep { sname: sname.to_string(), mode, dn, sn, plate, carbon, win_cand, kbits, geo, geo_alt });
            }
        }
        // The light set's bone tables (F-70785: F-27051 for each of the 19 kinds, made by F-10977; the set's constructor
        // F-36859 gives every kind a table of 8, F-14578): the bones named `<kind>0` (the base skeleton's, then the drawn slots' in
        // slot order, each skeleton in its order - the iterator of the nitrous anchors), the first 8 kept. Three cars
        // carry more brake glows than that with the wide body and a taillight fitted (disc test `lamp_bone_names`).
        glow_ranked.sort_by_key(|g| g.0);
        let mut glow_bones: Vec<(u8, Vec3, Vec3)> = Vec::new();
        let mut glow_sides: Vec<u8> = Vec::new();
        for (_, kind, w, facing, side) in glow_ranked {
            if glow_bones.iter().filter(|g| g.0 == kind).count() < LIGHT_SET_KIND_CAP {
                glow_bones.push((kind, w, facing));
                glow_sides.push(side);
            }
        }
        CarPrep { car_is_ambient, meshes, lamp_acc, own_hubs, glow_bones, glow_sides, font_bones, variants, variant_labels, extra_variants, part_rig, hood_slot, trunk_bone, slot_bones, roof_place, upgrade }
    }

    /// Tyre + rim model for a rim directory, loaded once.
    /// The garage's BRAKES choice on the player's car (`None`: the car's own).
    pub fn set_player_brake(&mut self, gpu: &mut Gpu, world: &mut World, name: Option<&str>) {
        // (the car's own brake too: it stands behind the spokes of the rim now fitted)
        let Some(own) = self.player().filter(|c| !c.model.bike).map(|c| c.model.brake_dir.clone()) else { return };
        let Some(dir) = name.map(|n| world.assets.root.join("resources/brakes").join(n)).or(own) else { return };
        let tag = dir.display().to_string();
        if tag == self.player_brake_name {
            return;
        }
        let key = (dir.clone(), 0, 0);
        self.player_brake = match self.brake_cache.get(&key) {
            Some(b) => Some(b.clone()),
            None => {
                let b = load_brakes(gpu, world, &dir).map(Arc::new);
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
        let t_dress = std::time::Instant::now();
        let i = self.cars.iter().position(|c| !c.is_player && !c.dressed && !c.vinyl_name.is_empty() && c.parked.is_none())?;
        // (the builds of every car waiting to be dressed made ahead on workers, once each: the cars are still dressed one
        // a call, as before, and each finds its build's CPU part made - docs/PERFORMANCE.md 2026-10-27 (14))
        let waiting: Vec<(String, String)> = self.cars.iter().filter(|c| !c.is_player && !c.dressed && !c.vinyl_name.is_empty() && c.parked.is_none()).map(|c| (c.model.name.clone(), c.vinyl_name.clone())).collect();
        for (car, setup) in waiting {
            if self.dress_ahead.insert((car.clone(), setup.clone())) {
                if let Some(d) = resolve(&car, &setup) {
                    let dir = world.assets.root.join("resources/vehicle").join(&car);
                    self.prepare_ahead(&dir, &d.cfg);
                }
            }
        }
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
            if let Some(bdir) = d.brake.as_ref().map(|n| world.assets.root.join("resources/brakes").join(n)).filter(|p| p.is_dir()) {
                let key = (bdir.clone(), 0, 0);
                brake = match self.brake_cache.get(&key) {
                    Some(b) => Some(b.clone()),
                    None => {
                        let b = load_brakes(gpu, world, &bdir).map(Arc::new);
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
        log::info!("vehicles: {} dressed in {:.0} ms", name, t_dress.elapsed().as_secs_f32() * 1000.0);
        Some(i)
    }

    pub fn spawn(&mut self, gpu: &Gpu, model: Arc<CarModel>, pos: Vec3, yaw: f32, color: [f32; 4], is_player: bool) -> usize {
        // (the parts' uniforms in shared buffers, written once a frame by `upload_uniforms` - render::ObjPool)
        let binding = gpu.new_pooled_object(&ObjectUniform::default());
        let wheel_bindings = (0..4).map(|_| gpu.new_pooled_object(&ObjectUniform::default())).collect();
        let caliper_bindings = (0..4).map(|_| gpu.new_pooled_object(&ObjectUniform::default())).collect();
        let rotor_bindings = (0..4).map(|_| gpu.new_pooled_object(&ObjectUniform::default())).collect();
        let window_bindings = if model.windows.iter().any(|w| w.is_some()) { (0..2).map(|_| gpu.new_pooled_object(&ObjectUniform::default())).collect() } else { Vec::new() };
        let part_bindings = match model.batch_window.iter().copied().max().unwrap_or(0) {
            g if g >= 21 => (0..g as usize - 2).map(|_| gpu.new_pooled_object(&ObjectUniform::default())).collect(),
            g if g >= 20 => (0..18).map(|_| gpu.new_pooled_object(&ObjectUniform::default())).collect(),
            g if g >= 13 => (0..17).map(|_| gpu.new_pooled_object(&ObjectUniform::default())).collect(),
            g if g >= 7 => (0..10).map(|_| gpu.new_pooled_object(&ObjectUniform::default())).collect(),
            g if g >= 3 => (0..4).map(|_| gpu.new_pooled_object(&ObjectUniform::default())).collect(),
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
        self.cars.push(CarInstance { uid, model, binding, wheel_bindings, caliper_bindings, rotor_bindings, state: CarState::new(pos, yaw), input: CarInput::default(), color, ai, brain: None, placed: false, is_player, wheels: None, lean: 0.0, perf: 0.0, tuned: None, cop: false, stunned: 0.0, crash: Default::default(), parked: None, item: None, amb_born: 0.0, amb_life: 8.0, amb_id: -1, removing: None, settle: 0.0, racer: false, pinned: false, deactivated: false, boost: 0.0, driver: String::new(), plate_text: String::new(), plate: None, vinyl_name: String::new(), vinyl: None, vinyl_done: false, setup: None, stages: None, pose_history: Default::default(), inflict: 1.0, goon: false, look: None, brake: None, dressed: false, driver_set: String::new(), pu_ctl: 0, lock: 0, pu_ghost: false, pu_mirror: 0.0, pu_stealth: false, engine_off: false, show_driver: false, hi_driver: false, window_bindings, windows: Default::default(), spoiler: WindowCtl::default(), roof: RoofCtl::default(), part_bindings, rail: None, amb_lamps: AmbLamps::default(), park_steer: 0.0, park_held: false, amb_lift: [0.0; 6], sst_trail: Default::default(), siren: false, light_t: 0.0, steer_parts: Default::default(), amb_slot: amb_slot_start(), amb_light: AmbSlotLights::default(), amb_steer: Default::default(), amb_frames: 0, amb_hit: false });
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
            p.stages = None;
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

    /// A car with a slipstream turbo: the player's (mcPlayer::CreateCar F-51384) and every opponent's but a
    /// police car's (F-52772: not catalogue class 5) - race opponents and the street racers of free roam.
    /// Traffic and the police have none.
    pub fn has_sst(c: &CarInstance) -> bool {
        (c.is_player || c.racer || c.ai.as_ref().map_or(false, |a| a.brain.is_some())) && !c.cop && c.state.veh.is_some() && c.params().veh.is_some()
    }

    /// F-69277 for car `i` with the cars as they stand now: nothing if it is slower than its own tune +32;
    /// else over every car with a turbo but a motorcycle (catalogue class 3, SportBike: F-54299) and a car on
    /// two wheels (its gyro's +18, the side it is up on), the strongest slipstream (F-22877, the leader's
    /// tune), the first of equals. The follower's speed is its own chassis +224 of this tick (the speed it comes
    /// in with), the leader's the +224 its last update wrote.
    fn sst_search(&self, i: usize) -> (f32, i32) {
        use crate::physics::sst::{strength, DraftCar};
        let me = &self.cars[i];
        let Some(vt) = me.params().veh.as_ref() else { return (0.0, -1) };
        if !Self::has_sst(me) || me.state.vel.length() < vt.t.ss_turbo.min_speed {
            return (0.0, -1);
        }
        let mut best = (0.0f32, -1i32);
        for (j, o) in self.cars.iter().enumerate() {
            if j == i || o.model.bike || !Self::has_sst(o) {
                continue;
            }
            let (Some(ov), Some(ot)) = (o.state.veh.as_ref(), o.params().veh.as_ref()) else { continue };
            if ov.two != 0 {
                continue;
            }
            let st = &o.state;
            let s = strength(&ot.t.ss_turbo, &DraftCar { pos: st.pos, up: st.up(), vel: st.vel, speed: ov.last_speed }, me.state.pos);
            if s > best.0 {
                best = (s, j as i32);
            }
        }
        best
    }

    /// The search half of car `idx`'s mcCarSSTurbo::Update (F-60935), made right before the car's step - in
    /// the list's order, with the cars before it already moved this tick, as the original makes it in each car's
    /// update. A car that does not burn and finds a slipstream tells its leader so (+68 = this car if it is the player, else -1) and fades the
    /// leader's trail in (its model +7976 = clamp(strength + 0.75, 0, 1)); the car's own +68 is checked by the
    /// search of the car it names.
    fn sst_search_step(&mut self, idx: usize) {
        if !Self::has_sst(&self.cars[idx]) {
            return;
        }
        let found = self.sst_search(idx);
        let is_player = self.cars[idx].is_player;
        let d = self.cars[idx].state.veh.as_ref().map_or(-1, |v| v.sst.drafter);
        let drafting = d >= 0 && (d as usize) < self.cars.len() && self.sst_search(d as usize).0 > 0.0;
        let Some(vs) = self.cars[idx].state.veh.as_deref_mut() else { return };
        vs.sst.found = found;
        vs.sst.drafter_drafting = drafting;
        let (s, j) = found;
        if vs.sst.burning || vs.sst.locked || !(s > 0.0) {
            return;
        }
        if let Some(lv) = self.cars[j as usize].state.veh.as_deref_mut() {
            lv.sst.trail_target = (s + 0.75).clamp(0.0, 1.0);
            // +68 = the follower's player index (F-55794: the driver's +136), -1 for a computer racer - so an
            // AI-drafted car's own update drops the target again (recorded, sst_trail_evidence_2026-10-28 s.10)
            lv.sst.drafter = if is_player { idx as i32 } else { -1 };
        }
    }

    /// Test hook MCLA_SST_TRAIL_FOLLOW=<dist> (the port's copy of the recorder's hold, work/slipstream/trailrec
    /// hold2.py / MCLA_HOLD_BEHIND): before the player's step each tick his car is put `dist` m behind (along +z,
    /// the back row) the nearest other car with a slipstream turbo (the racers; no bike), turned as that car and with its
    /// velocity, so that he drafts it.
    fn sst_trail_follow(&mut self, idx: usize) {
        if !self.cars[idx].is_player {
            return;
        }
        let Some(dist) = std::env::var("MCLA_SST_TRAIL_FOLLOW").ok().and_then(|v| v.trim().parse::<f32>().ok()) else { return };
        let me = self.cars[idx].state.pos;
        let lead = (0..self.cars.len())
            .filter(|&j| j != idx && Self::has_sst(&self.cars[j]) && !self.cars[j].model.bike && self.cars[j].placed)
            .min_by(|&a, &b| self.cars[a].state.pos.distance(me).total_cmp(&self.cars[b].state.pos.distance(me)));
        let Some(j) = lead else { return };
        let (pos, rot, vel, ang) = { let s = &self.cars[j].state; (s.pos, s.rot, s.vel, s.ang_vel) };
        let c = &mut self.cars[idx];
        c.state.relocate(pos + rot * Vec3::Z * dist, rot);
        c.state.vel = vel;
        c.state.ang_vel = ang;
    }

    /// After the step: each turbo's trail fades (mcCarModel F-66688 - kept while the local player is this car,
    /// or the car in its slipstream with this car as its leader).
    fn sst_trails(&mut self, dt: f32) {
        // (the trail's points move first - sst_trail.rs)
        crate::sst_trail::update(&mut self.cars, dt);
        let n = self.cars.len();
        let leader_of: Vec<i32> = self.cars.iter().map(|c| c.state.veh.as_ref().map_or(-1, |v| v.sst.leader)).collect();
        let player: Vec<bool> = self.cars.iter().map(|c| c.is_player).collect();
        for i in 0..n {
            if !Self::has_sst(&self.cars[i]) {
                continue;
            }
            let Some(vs) = self.cars[i].state.veh.as_deref_mut() else { continue };
            let d = vs.sst.drafter;
            let keep = player[i] || (d >= 0 && (d as usize) < n && player[d as usize] && leader_of[d as usize] == i as i32);
            vs.sst.fade_trail(keep, dt);
        }
        // (MCLA_SST_LOG: each car's turbo every half second)
        self.sst_log_t += dt;
        if std::env::var_os("MCLA_SST_LOG").is_some() && self.sst_log_t >= 0.5 {
            self.sst_log_t = 0.0;
            for (i, c) in self.cars.iter().enumerate() {
                if let (Some(v), true) = (c.state.veh.as_ref(), Self::has_sst(c)) {
                    log::info!("sst car {i}{} speed {:.1} level {:.3} full {} burning {} strength {:.3} leader {} drafter {} fov {:.2} trail {:.2}", if c.is_player { "P" } else { "" }, c.state.speed(), v.sst.level, v.sst.full, v.sst.burning, v.sst.strength, v.sst.leader, v.sst.drafter, v.sst.fov, v.sst.trail);
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
        // (the start screen: the world has run behind the loading logo before its first frame -
        // traffic.rs FRONT_END_PREROLL frames of the spawners and the cars)
        let fe_new = self.amb_frontend.is_some() && !self.amb_frontend_was;
        self.amb_frontend_was = self.amb_frontend.is_some();
        if fe_new && world.nav.is_some() {
            for _ in 0..traffic::FRONT_END_PREROLL {
                self.spawn_traffic(gpu, world, center, 1.0 / 30.0);
                self.update(world, CarInput::default(), 1.0 / 30.0);
            }
        }
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
                let t_liv = std::time::Instant::now();
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
                log::info!("vehicles: livery {} {} ({:.0} ms)", name, if bind.is_some() { "loaded" } else { "not found" }, t_liv.elapsed().as_secs_f32() * 1000.0);
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
            // (the plate font is the second of F-11553's four, fnt_license_plate)
            let root = &world.assets.root;
            let font = std::fs::read(root.join("vehicle/shared_font/fnt_license_plate.dds")).ok().and_then(|d| mcla_formats::dds::parse_dds("fnt_license_plate", &d).ok()).filter(|t| t.width == 256 && t.height == 256).map(|t| t.to_rgba8());
            self.plate_art = Some(match font {
                Some(f) => (load_plate_bg(root, "plate_ca_00"), f),
                None => (None, Vec::new()),
            });
        }
        let Some((Some(ca), font)) = self.plate_art.clone() else {
            if self.plate_art.is_none() {
                return;
            }
            // no art: stop trying
            let white = gpu.white.clone();
            let b = gpu.texture_bind_group(&white, &white, 0, 0);
            for c in self.cars.iter_mut().filter(|c| c.plate.is_none()) {
                c.plate = Some(b.clone());
            }
            return;
        };
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
                let art = self.plate_bgs.entry(n.clone()).or_insert_with(|| load_plate_bg(&world.assets.root, &n));
                art.clone().map(|a| (n, a))
            });
            let (slot, bg) = match own_bg {
                Some((n, a)) => (format!("{text}@{n}"), a),
                None => (text.clone(), ca.clone()),
            };
            let made = self.plates.get(&slot).and_then(|b| b.upgrade());
            if made.is_none() {
                self.plates.retain(|_, b| b.strong_count() > 0);
                let (w, h) = (bg.w, bg.h);
                let px = compose_plate(&bg, &font, &text);
                // (as stored, like the car's other maps: city.wgsl decodes the sample once and PS_Plate's ^1.6 gets the
                // stored value back through car_raw. A gamma upload was decoded twice - the plate dark and saturated, its
                // blue 4, 17, 192 where the original's is 56, 100, 250. MCLA_PLATE_GAMMA_OLD=1: as before)
                let gamma = !car_tex_raw() || std::env::var("MCLA_PLATE_GAMMA_OLD").is_ok();
                let tex = Arc::new(crate::render::create_rgba_as(&gpu.device, &gpu.queue, w as u32, h as u32, &px, "plate", gamma));
                let key = mcla_formats::hash::joaat(&format!("plate:{slot}")) | 1;
                let bind = gpu.texture_bind_group(&tex, &tex, key, key);
                self.plates.insert(slot.clone(), Arc::downgrade(&bind));
                self.cars[i].plate = Some(bind);
            } else {
                self.cars[i].plate = made;
            }
        }
    }

    /// Physics + AI for every car.
    pub fn update(&mut self, world: &World, player_input: CarInput, dt: f32) {
        self.clock += dt;
        // (test hook MCLA_GROUND_PROBE=x,y,z;x,y,z;...: the parked cars' ground probe (traffic::probe_ground) at each
        // point, printed once all of them are loaded - for comparing with the original's probe records)
        if let Ok(v) = std::env::var("MCLA_GROUND_PROBE") {
            static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            let pts: Vec<Vec3> = v.split(';').filter_map(|t| { let f: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect(); (f.len() == 3).then(|| Vec3::new(f[0], f[1], f[2])) }).collect();
            if !DONE.load(std::sync::atomic::Ordering::Relaxed) && pts.iter().all(|p| world.ready_at(*p)) {
                DONE.store(true, std::sync::atomic::Ordering::Relaxed);
                for p in pts {
                    match traffic::probe_ground(&world.collider, p) {
                        Some((h, n)) => eprintln!("groundprobe at {:.4} {:.4} {:.4} hit {:.4} {:.4} {:.4} normal {:.4} {:.4} {:.4}", p.x, p.y, p.z, h.x, h.y, h.z, n.x, n.y, n.z),
                        None => eprintln!("groundprobe at {:.4} {:.4} {:.4} none", p.x, p.y, p.z),
                    }
                }
            }
        }
        for c in self.cars.iter_mut() {
            c.state.touches.clear();
        }
        // (test hook: MCLA_WINDOW_TEST=1 rolls the player's side windows down)
        static WINDOW_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let window_test = *WINDOW_TEST.get_or_init(|| std::env::var("MCLA_WINDOW_TEST").is_ok_and(|v| v == "1"));
        static ROOF_TEST: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
        let roof_test = *ROOF_TEST.get_or_init(|| std::env::var("MCLA_ROOF_TEST").ok().and_then(|v| v.parse::<f32>().ok()));
        static SPOILER_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let spoiler_test = *SPOILER_TEST.get_or_init(|| std::env::var("MCLA_SPOILER_TEST").is_ok_and(|v| v == "1"));
        for c in self.cars.iter_mut() {
            // (the light bar's clip: its own time from the car's making, the game frame's step each frame, back to 0 at
            // its end - F-35036 / F-27851; docs/reference/hookmen/cop_lamps.md)
            if let Some(cl) = c.model.cop_lights.as_ref() {
                c.light_t += dt;
                if c.light_t >= cl.duration {
                    c.light_t = 0.0;
                }
            }
            // (the convertible roof, F-35036: before the windows - it opens and closes them)
            if let (Some(cl), Some(top)) = (c.model.clamshell.as_ref(), c.model.skinned_top.as_ref()) {
                let down = c.model.cfg.extra.get(vehicle_model::ROOF_DOWN).copied().unwrap_or(0) != 0;
                let win_lens = [0, 1].map(|k| c.model.windows[k].as_ref().map(|w| w.duration));
                c.roof.step(down, c.is_player, dt, (cl.duration, top.duration), &mut c.windows, win_lens);
                // (test hook: MCLA_ROOF_TEST=<s> holds every convertible's roof players at that time, moving)
                if let Some(t) = roof_test {
                    c.roof.hatch = WindowCtl { state: 3, time: t.min(cl.duration) };
                    c.roof.top = WindowCtl { state: 3, time: t.min(top.duration) };
                }
            }
            for k in 0..2 {
                if let Some(clip) = c.model.windows[k].as_ref() {
                    if window_test && c.is_player {
                        c.windows[k].request(true, clip.duration);
                    }
                    c.windows[k].update(dt, clip.duration);
                }
            }
            // (the auto spoiler, F-35036: raised while the speed - chassis +224 - x 2.237 is above 60 mph, else lowered; a
            // request is taken only at the matching end, as the windows')
            if let Some(clip) = c.model.spoiler.as_ref() {
                // (test hook: MCLA_SPOILER_TEST=1 raises it at any speed)
                c.spoiler.request(c.state.speed() * 2.237 > 60.0 || (spoiler_test && c.is_player), clip.duration);
                c.spoiler.update_at(WindowCtl::SPOILER_RATE, dt, clip.duration);
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
        // (the ambient system's answers to the cruising brains, before they run)
        crate::cruisedrive::events(self, &world.collider);
        let n = self.cars.len();
        // AI inputs (with simple car-ahead avoidance)
        let positions: Vec<(Vec3, Vec3, f32)> = self.cars.iter().map(|c| (c.state.pos, c.state.forward(), c.state.speed())).collect();
        // (the racers - cars with a racing brain, in the order they were added - in mcOpponentManager's order F-28020:
        // from its start index round the list, the start moving on by one each game frame; they draw from the brains'
        // shared generator, so the order decides who gets which draw. The other cars after them, in list order.
        // approx.: the list's order taken as the cars' order; each brain keeps its own frame clock)
        let racers: Vec<usize> = (0..n).filter(|&i| self.cars[i].ai.as_ref().map_or(false, |a| a.brain.0.is_some())).collect();
        let (rot, next) = crate::oppmgr::update_order(self.opp_start, racers.len(), false);
        if dt > 0.0 && !self.frozen && crate::brain::frame_due(&mut self.opp_frame, dt) {
            self.opp_start = next;
        }
        let order: Vec<usize> = rot.iter().map(|&k| racers[k]).chain((0..n).filter(|i| !racers.contains(i))).collect();
        for i in order {
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
                // (worked out on the frame's first tick, held for its second - see `AnimBrain::fresh`)
                let mut s = match b.held {
                    Some(h) if !b.fresh => h,
                    _ => crate::cinecar::solve(b, &st, self.cars[i].params().veh.as_deref()),
                };
                if let Some(b) = self.cars[i].brain.as_mut() {
                    b.held = Some(s);
                }
                // (the player's air bags stay fitted under the brain - the chassis update F-97237 runs on through the
                // garage's drive-out; only the car's reset F-46526 resets the air ride, F-80081)
                if self.cars[i].is_player {
                    s.air = player_input.air;
                }
                s
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
                // (the steering is written 0 on the grid, A-88204 - only there)
                let hold = self.grid_hold.is_some() || self.player_hold;
                scramble(CarInput { hold, steer: if hold { 0.0 } else { player_input.steer }, ..player_input }, self.player_scramble)
            } else if crate::cruisedrive::is_cruiser(&self.cars[i]) {
                // (a cruising man: mcBrainCruising over his racing brain - cruisedrive.rs)
                crate::cruisedrive::tick(self, &world.collider, i, dt)
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
            } else if crate::hangout_scene::standing(&self.cars[i]) {
                // (a hangout set-up's car: an opponent with no race - its brain's pedals with no line, hangout_scene.rs)
                crate::hangout_scene::standing_input()
            } else {
                CarInput::default()
            };
            // (the brain's off-mesh teleport: F-86308 at the polygon's centre, brain.rs)
            if let Some(p) = self.cars[i].ai.as_mut().and_then(|a| a.teleport.take()) {
                let c = &mut self.cars[i];
                if let Some(vt) = c.params().veh.clone() {
                    let r = c.state.rot;
                    crate::physics::vehsim::set_pose(&mut c.state, &vt, &world.collider, [r * Vec3::X, r * Vec3::Y, r * Vec3::Z, p], Vec3::ZERO, Vec3::ZERO);
                    c.pose_history.reset();
                }
            }
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
            // (nor for a hangout set-up's standing car: the original's brain gives it no handbrake - hangout_scene.rs `standing`)
            if !self.cars[i].is_player && self.cars[i].parked.is_none() && !pursuing && !passing && self.grid_hold.is_none() && !crate::hangout_scene::standing(&self.cars[i]) {
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
                // (the player's car damaged out or wiped out, +592 bit 0x04 / 0x10: the pad's input goes on to the car,
                // whose update writes its own over it - vehsim.rs `step`; approx.: any other hold the port's own 0.15)
                let crash = self.cars[i].is_player && (self.cars[i].state.damage >= 0.999 || self.cars[i].state.upset > 0.0);
                if !crash {
                    input = CarInput { brake: 0.15, ..Default::default() };
                }
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
            self.cars[i].input = CarInput { ai: input.ai || !self.cars[i].is_player, lock: self.cars[i].lock, ..input };
        }
        // (a new traffic or parked car is bound to its pooled instance before its first render update: the activate hook)
        self.amb_pool_step();
        if self.data_root != world.assets.root {
            self.data_root = world.assets.root.clone();
        }
        if let Some(nav) = world.nav.as_ref() {
            self.ride_traffic(nav, &world.collider, dt.min(1.0 / 30.0));
        }
        self.parked_render(dt.min(1.0 / 30.0));
        // ---- one physics level for every car (physics/loose.rs): the loose props and pieces once a tick
        // before and after the cars, each car's step with the cars not yet stepped as bodies of its island
        // (each pair is solved in the earlier car's step) ----
        let phys_t0 = std::time::Instant::now();
        // (test hook MCLA_RAM_CAR=secs: at that time on the vehicles' clock the player's car is put 2.6 m to the right of the
        // nearest other placed car with the original's body, at its speed and 6 m/s towards it - a car x car contact for the
        // level step; MCLA_RAM_SIDE=<m> another distance, MCLA_RAM_WIPE=1 the player's wipeout blow with it)
        if let Some(at) = std::env::var("MCLA_RAM_CAR").ok().and_then(|v| v.parse::<f32>().ok()) {
            static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if self.clock >= at && !DONE.load(std::sync::atomic::Ordering::Relaxed) {
                let me = self.cars.iter().position(|c| c.is_player);
                let target = me.and_then(|m| {
                    let p = self.cars[m].state.pos;
                    (0..self.cars.len()).filter(|&j| j != m && self.cars[j].placed && self.cars[j].state.veh.is_some() && self.cars[j].rail.is_none() && self.cars[j].parked.is_none()).min_by(|&a, &b| (self.cars[a].state.pos - p).length().total_cmp(&(self.cars[b].state.pos - p).length()))
                });
                if let (Some(m), Some(j)) = (me, target) {
                    DONE.store(true, std::sync::atomic::Ordering::Relaxed);
                    let (tp, tr, tv) = (self.cars[j].state.pos, self.cars[j].state.rot, self.cars[j].state.vel);
                    let r = tr * Vec3::X;
                    let c = &mut self.cars[m];
                    let side = std::env::var("MCLA_RAM_SIDE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(2.6);
                    c.state.relocate(tp + r * side, tr);
                    c.state.vel = tv - r * 6.0;
                    if std::env::var_os("MCLA_RAM_WIPE").is_some() {
                        c.state.wipeout = 2.0;
                    }
                    c.state.ang_vel = Vec3::ZERO;
                    c.pose_history.reset();
                    log::info!("MCLA_RAM_CAR: player put beside car {} (uid {})", j, self.cars[j].uid);
                    RAM_TARGET.store(j * 1000 + m, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let t = RAM_TARGET.load(std::sync::atomic::Ordering::Relaxed);
            if t != usize::MAX {
                static LEFT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(90);
                if LEFT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) > 0 {
                    let (j, m) = (t / 1000, t % 1000);
                    let (a, b) = (&self.cars[m].state, &self.cars[j].state);
                    log::info!("MCLA_RAM_CAR: distance {:.2} speeds {:.1} {:.1} player placed {}", (a.pos - b.pos).length(), a.vel.length(), b.vel.length(), self.cars[m].placed);
                }
            }
        }
        let shared = world.collider.loose.clone().filter(|_| std::env::var_os("MCLA_DBG_NOSHARED").is_none());
        let step_dt = dt.min(1.0 / 30.0);
        if let Some(lw) = shared.as_deref() {
            self.level.live = true;
            self.ambient_into_level(step_dt);
            self.level.pre_collide(lw, step_dt);
        }
        // (the cars' steps of the tick - which car, its input, what follows its step: run together after this walk,
        // `step_cars`)
        let mut jobs: Vec<(usize, CarInput, StepPost)> = Vec::new();
        for idx in 0..self.cars.len() {
            self.sst_trail_follow(idx);
            self.sst_search_step(idx);
            let c = &mut self.cars[idx];
            // (traffic: the physics level's - inactive while it rides, an ambient collider while
            // physical; loose.rs `Amb`)
            if c.rail.as_ref().is_some_and(|r| !r.physical || r.in_level) {
                continue;
            }
            // a parked car: on the ground by the spawn probe once its ground is here (F-23470 /
            // F-72535: +-1.75 m round the spot, the place the hit, up its normal, the forward made
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
                // (the reset car's own pairs go with its old state)
                c.state = CarState::new(p, yaw);
                c.pose_history.reset();
                c.placed = false;
            }
            // Already placed cars can outlive their streamed collision sectors.
            // Suspend physical motion until they can be placed on resident ground
            // again. The existing distant-racer path still advances kinematically.
            // (a race opponent the racer brain drives keeps driving on the city's collision bound, which is loaded
            // whole - `World::own_bound`, one resource in the original too: far from the player it is neither
            // suspended nor carried along the old driver's line with its wheels still - AI.md NEW 2 / 3 / 5)
            let on_bound = world.own_bound && c.ai.as_ref().is_some_and(|a| a.brain.is_some());
            if c.placed && !on_bound && !world.ready_at(c.state.pos) {
                c.placed = false;
                c.pose_history.reset();
            }
            // Racers outside the streamed-in part of the city have no ground to drive on:
            // move them along the race line until the world around them is loaded.
            // (not on the grid: there the cars stand on their brakes while the world streams in)
            if !c.is_player && !on_bound && !self.frozen && self.grid_hold.is_none() && c.parked.is_none() && c.rail.is_none() && !world.ready_at(c.state.pos) {
                if let Some(ai) = c.ai.as_mut() {
                    if ai.path.is_some() {
                        // (a brain-driven racer has left the old driver's index behind: from where the car is)
                        if ai.brain.is_some() {
                            ai.seat_path(c.state.pos);
                        }
                        let v = ai.target_speed * 0.8;
                        if let Some(p) = ai.path_ahead(c.state.pos, v * dt) {
                            let d = p - c.state.pos;
                            if d.length() > 1e-3 {
                                let dir = d.normalize();
                                let gear = c.state.gear;
                                c.state = CarState::new(p, (-dir.x).atan2(-dir.z));
                                // (a cruising hookman has no physical car out here: the original poses it on its ghost,
                                // which rides the rail on the road - every placement through the +-5 m ground hit (F-29307).
                                // The line's points lie straight between lane points, under the road on a crest: the car
                                // stood under the street and could not be flashed, AI.md NEW 7)
                                if c.racer && world.own_bound {
                                    if let Some(g) = world.collider.ground(p, 5.0, 5.0) {
                                        let n = g.normal;
                                        if n.y > 0.8 {
                                            let f = (dir - n * dir.dot(n)).normalize_or(dir);
                                            let right = f.cross(n).normalize_or(Vec3::X);
                                            c.state.rot = Quat::from_mat3(&glam::Mat3::from_cols(right, n, -f)).normalize();
                                        }
                                        c.state.pos = g.point + Vec3::Y * 0.03;
                                    }
                                }
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
                    if c.racer {
                        // (a cruising hookman: set down on the nearest rail with ground under it, as the cruising brain's
                        // teleport, at the speed it had along the rail - `roamers::rail_place`; nothing within 80 m: it waits
                        // unplaced. The first surface from 52 m above put a car on whatever was highest there, and with
                        // none - a loop point inside a block - let it fall through the map, AI.md NEW 6)
                        let place = crate::roamers::rail_place(world, c.state.pos);
                        let Some((at, up, f, _)) = place else { continue };
                        let f = (f - up * f.dot(up)).normalize_or(f);
                        let right = f.cross(up).normalize_or(Vec3::X);
                        let speed = Vec3::new(c.state.vel.x, 0.0, c.state.vel.z).length();
                        c.state.rot = Quat::from_mat3(&glam::Mat3::from_cols(right, up, -f)).normalize();
                        c.state.pos = at;
                        c.state.vel = f * speed;
                        c.state.ang_vel = Vec3::ZERO;
                        c.placed = true;
                    }
                    let gg = if c.racer { None } else { near.or_else(|| world.collider.ground(c.state.pos + Vec3::Y * 2.0, 50.0, 200.0)) };
                    if let Some(g) = gg {
                        // straight onto its springs (the model's origin is at road level), tilted with the road
                        let fwd = c.state.forward();
                        let n = g.normal;
                        if n.y > 0.8 {
                            let f = (fwd - n * fwd.dot(n)).normalize_or(fwd);
                            let right = f.cross(n).normalize_or(Vec3::X);
                            c.state.rot = Quat::from_mat3(&glam::Mat3::from_cols(right, n, -f)).normalize();
                        }
                        // (the player's car with its origin on the ground point itself: the original's after a load
                        // stands at 17.59785, the port's ground hit there 17.5978 - recorded 2026-11-05, Impala at the
                        // Hollywood garage; the 3 cm lift made it drop 6.8 cm and bounce 2.1 cm up, the original 2.9 / 0.8)
                        c.state.pos = g.point + Vec3::Y * if c.is_player { 0.0 } else { 0.03 };
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
                    jobs.push((idx, CarInput { brake: 1.0, handbrake: true, ..Default::default() }, StepPost::Settle));
                } else {
                    c.state.vel = Vec3::ZERO;
                    c.state.ang_vel = Vec3::ZERO;
                }
                continue;
            }
            if c.deactivated {
                // (the steering is the default steer direction the start script gives the three,
                // Racer_SetDefaultSteerDirection 0.5 - chassis +656, F-69218. A scene's car - the player's after
                // the scene's engine_off, F-76651(racer, 1): the mover's inputs give way the same, and its +656 is
                // the 0 that F-30391(0) left)
                let steer = if c.brain.is_some() { 0.0 } else { 0.5 };
                let input = CarInput { deactivated: true, steer, ..Default::default() };
                jobs.push((idx, input, StepPost::None));
                continue;
            }
            if self.frozen || (self.grid_hold.is_some() && !c.is_player && !c.pinned) {
                // settle onto the ground but do not drive
                jobs.push((idx, CarInput { brake: 1.0, ..Default::default() }, StepPost::Frozen));
                continue;
            }
            jobs.push((idx, c.input, StepPost::Trace));
        }
        self.step_cars(&jobs, world, step_dt, shared.is_some());
        for &(idx, _, post) in &jobs {
            let c = &mut self.cars[idx];
            // (the crash flags of a car the game drives itself, as mcCar::Update keeps them for every car; the player's
            // are main.rs's)
            if !c.is_player && c.state.veh.is_some() {
                let (bike, cop) = (c.model.bike, c.cop);
                let CarInstance { state, tuned, model, crash, pose_history, .. } = c;
                let p: &CarParams = tuned.as_deref().unwrap_or(&model.params);
                // (stood up where it is, 0.8 m up, onto its wheels: not set down again by the port's placing, which
                // would put a racer on a rail)
                let out = crate::physics::wipeout::step(crash, state, p, bike, cop, step_dt);
                if out.stood_up {
                    pose_history.reset();
                }
                if out.reset {
                    if let Some(vt) = p.veh.clone() {
                        crate::physics::vehsim::reset(state, &vt, &world.collider);
                    }
                }
            }
            match post {
                StepPost::Settle => {
                    c.state.vel.x = 0.0;
                    c.state.vel.z = 0.0;
                    c.state.ang_vel.y = 0.0;
                }
                StepPost::Frozen => {
                    c.state.vel.x = 0.0;
                    c.state.vel.z = 0.0;
                    c.state.ang_vel = Vec3::ZERO;
                }
                StepPost::None => {}
                StepPost::Trace => {
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
            }
        }
        self.sst_trails(step_dt);
        if let Some(lw) = shared.as_deref() {
            // (the thrown rider whose island is one with the level's: his bike's step left it here - his build, solve,
            // push and post-push with that island's, his tick's answers back to the bike's state)
            let job = self.cars.iter_mut().find_map(|c| c.state.veh.as_deref_mut().and_then(|v| v.rider_job.take().map(|j| (j, v))));
            match job {
                Some((j, v)) if v.rider.is_some() => {
                    let rd = v.rider.as_deref_mut().unwrap();
                    let tick = v.rider_out.get_or_insert_with(Default::default);
                    self.level.step_rest_alone_rider(lw, step_dt, Some((rd, j, tick)));
                }
                _ => self.level.step_rest_alone(lw, step_dt),
            }
            self.level.end_tick_with(lw, step_dt, Some(&world.collider));
            self.ambient_out_of_level(step_dt, world);
        }
        if shared.is_some() {
            self.soft_push();
        }
        // (MCLA_PHYS_TIMING=1: the level's physics block - every car's step and the loose world - timed, logged every 1000 ticks)
        if std::env::var_os("MCLA_PHYS_TIMING").is_some() {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SUM: AtomicU64 = AtomicU64::new(0);
            static N: AtomicU64 = AtomicU64::new(0);
            let ns = phys_t0.elapsed().as_nanos() as u64;
            let us = SUM.fetch_add(ns, Ordering::Relaxed) + ns;
            let n = N.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 1000 == 0 {
                log::info!("phys timing: {} ticks, mean {:.1} us, cars {}, islands of several cars {}", n, us as f64 / n as f64 / 1000.0, self.cars.iter().filter(|c| c.state.veh.is_some()).count(), ISLANDS_OF_CARS.load(Ordering::Relaxed));
            }
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
                            // (F-61340: its own +16 x the other's +20)
                            let mult = self.cars[other].inflict * self.cars[mov].state.blow[1];
                            self.cars[mov].state.contact_hit(&p, -vn * 1.3, mult, at);
                        }
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
                    // (a computer car's side makes no sound for a touch with the player's car: A-26371)
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
                        // factor, mcCarDamage +20 (F-61340; the payback sets the player's to 3, the
                        // client's to 0.65 and, once he runs, 0.55), to the points above the
                        // ImpactThreshold (F-96323))
                        let at = (self.cars[i].state.pos + self.cars[j].state.pos) * 0.5 + Vec3::Y * 0.5;
                        for (k, o, dv) in [(i, j, dvi), (j, i, dvj)] {
                            let mut p = self.cars[k].tuned.as_deref().unwrap_or(&self.cars[k].model.params).clone();
                            // (the payback's client: Racer_SetFatalBlow(client,
                            // Racer_GetImpactThreshold(client)) - any blow that counts can finish him)
                            if self.fragile == Some(k) {
                                p.dmg[3] = p.dmg[2];
                            }
                            let mult = self.cars[o].inflict * self.cars[k].state.blow[1];
                            self.cars[k].state.contact_hit(&p, dv, mult, at);
                        }
                    }
                }
            }
        }

        // ---- the eased steering, every frame (F-98654 -> F-69494, steer_parts.rs): the racer's values as F-80793 copies
        // them in (a player: its pad's +36, +24 and +88 - F-13138, F-54401, F-51971; a computer driver: its input's +36 and
        // +24 and 1 - F-73863, F-72992, F-35724), the rider's rates (F-28290); a car's rates are what its driver's gatherer
        // wrote (drivers.rs `SteerOut`, handed on by main.rs) ----
        for c in self.cars.iter_mut() {
            let Some(vs) = c.state.veh.as_deref() else { continue };
            let Some(vt) = c.tuned.as_deref().unwrap_or(&c.model.params).veh.as_ref() else { continue };
            let d = &mut c.steer_parts;
            if c.model.bike {
                d.rider_rates();
            }
            if c.input.ai {
                // (approximate: the opponent input's +36 is the frame's steering out of its brain, F-17058's local
                // +100, which the port does not keep apart from +24)
                (d.gate, d.value, d.divisor) = (c.input.steer, c.input.steer, 1.0);
            } else {
                // (+36: the steering axis through its dead zones, towards which it goes at 8 a second while the weight
                // button was down the frame before, else 0 - A-60035 .. A-28077 of mcPlayerInput::Update (F-56064); its
                // flag 0x80 case, set at once, is not ported)
                d.pad_x = if vs.pad.button {
                    let v = crate::physics::pad::dead_zone(crate::physics::pad::stick_axis(crate::physics::pad::stick_byte(c.input.steer)), 0.1);
                    let mut x = d.pad_x;
                    crate::physics::exact::approach(&mut x, v, 8.0, dt);
                    x
                } else {
                    0.0
                };
                (d.gate, d.value, d.divisor) = (d.pad_x, vs.pad_steer, crate::physics::pad::speed_factor(&vt.t.input, vs.last_speed));
            }
            d.step(vs.last_speed, false, dt);
        }

        // ---- the moving damage parts (F-24651: every car, every frame, after its impacts are queued) ----
        let ground = |p: Vec3| world.collider.ground(p, 1.0, 3.0).map(|h| (h.point.y, h.normal));
        for c in self.cars.iter_mut() {
            let st = &mut c.state;
            if !st.parts.rigged {
                st.parts.rig(&c.model.part_rig);
            }
            let cm = Mat4::from_rotation_translation(st.rot, st.pos);
            // (the repair, F-75221, runs where the game fixes the car: here, a car whose damage is all gone)
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
    /// The simulator's soft push between vehicles (F-49821, after the level's update): physical
    /// cars of the shared level with a body of the original's. Cars the level does not hold (a
    /// ghost under MegaStealth, parked or lane-riding traffic, a car without the original's body)
    /// are not in it here; the original lists them too (their share of a push is overwritten by
    /// the lane follower or never applied while inactive) - the layer masks of the pair are taken
    /// as passed by any pair the level collides (not read from the archetypes).
    fn soft_push(&mut self) {
        let mut ids = Vec::new();
        let mut bodies = Vec::new();
        for (k, c) in self.cars.iter().enumerate() {
            if !c.placed || c.pu_ghost || c.parked.is_some() || c.park_held || c.rail.is_some() {
                continue;
            }
            let p = c.tuned.as_deref().unwrap_or(&c.model.params);
            let (Some(vt), Some(_)) = (p.veh.as_ref(), p.hull.as_ref()) else { continue };
            if let Some(b) = crate::physics::vehsim::soft_body(&c.state, vt) {
                ids.push(k);
                bodies.push(b);
            }
        }
        for (i, v) in crate::physics::softpush::pushes(&bodies, &|_, _| true) {
            crate::physics::vehsim::queue_push(&mut self.cars[ids[i]].state, v);
        }
    }

    /// The base driver's contact walker for a car-car contact of cars `a` and `b` (F-83713: the vehicle case of the
    /// walker; the port takes "in contact" as "the level gave an impulse in this step"). Gate: one of the two
    /// speeds (chassis +224) above 4.47 m/s (10 mph). The aggressor is the one whose partner stands (< 0.2235 m/s), else
    /// the one that stands itself is the victim, else the one whose velocity points at the other the more
    /// (velocity . offset / speed, compared); the victim's driver is told (hit with byte 0, the aggressor's speed), the
    /// racer turning that into its brain's 1.5 s collision timer when the aggressor is the player (F-30284: the other driver's
    /// slot 10 is true for mcPlayer only; the other driver classes return false). Both drivers are told in the original, the aggressor's call (byte 1) does nothing.
    /// The cruiser whose siren lights the world (F-88968 tests the siren's target, F-62430 takes the per-frame record
    /// nearest the camera within 100 m): its place 2 m above the car.
    pub fn siren_light(&self, cam: Vec3) -> Option<Vec3> {
        self.cars.iter().filter(|c| (c.cop && (c.siren || self.sirens)) || (c.is_player && c.siren)).map(|c| c.state.pos + Vec3::Y * 2.0).map(|p| (p.distance_squared(cam), p)).filter(|x| x.0 < 10000.0).min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1)
    }

    fn notify_contact(&mut self, a: usize, b: usize) {
        const MOVING: f32 = 4.47;
        let (sa, sb) = (self.cars[a].state.vel.length(), self.cars[b].state.vel.length());
        if !(sa > MOVING || sb > MOVING) {
            return;
        }
        let a_aggressor = first_is_aggressor(self.cars[a].state.vel, self.cars[b].state.vel, self.cars[b].state.pos - self.cars[a].state.pos);
        for (victim, aggressor) in [(b, a), (a, b)] {
            if a_aggressor == (victim == b) && self.cars[aggressor].is_player {
                if let Some(ai) = self.cars[victim].ai.as_mut().filter(|ai| ai.brain.is_some()) {
                    ai.hit_pending = true;
                }
                // (F-83713: the aggressor's law word gets 2 when the victim is a police car; a racer sets a bit no law
                // code reads. Traffic is class 6: `agro_contacts`)
                if self.cars[victim].cop {
                    let key = 2u64 << 32 | self.cars[victim].uid as u64;
                    let v = if aggressor == a { sa } else { sb };
                    if let Some(vs) = self.cars[aggressor].state.veh.as_deref_mut() {
                        if vs.agro.law_new_contact(key, v) {
                            vs.agro.law_word |= 2;
                        }
                    }
                }
            }
        }
    }

    /// Every car's step of the tick in the original's order for several cars (physics/vehsim.rs `LevelX`, F-93826 ->
    /// F-38310 -> F-96487): each car's update and move (to its first yield), then each car's collide with the level
    /// lent to it - the other cars bodies of it, each pair made by the lower car's step - then each island solved once:
    /// an island of one car by its own step, an island of several by one of them (the car with the thrown rider, else the
    /// first), the others lent into its level and answered, their steps ending with the island's answer. Without the
    /// shared level each car's whole step in turn.
    fn step_cars(&mut self, jobs: &[(usize, CarInput, StepPost)], world: &World, dt: f32, shared: bool) {
        use crate::physics::loose::{static_shape, LiveVeh, LooseState, PBody};
        use crate::physics::vehsim::{LevelX, Stage};
        use std::cell::RefCell;
        use std::future::Future;
        use std::pin::Pin;
        use std::task::{Context, Poll, Waker};
        // (each car the level can hold: its id, its place in the cars, the part of a LiveVeh that does not move, and its
        // body as the last tick left it moved by the step - for a car whose step does not hand one)
        struct Info {
            uid: u32,
            idx: usize,
            shape: &'static crate::physics::hull::HullShape,
            class: u8,
            inflict: f32,
            parked: bool,
            player: bool,
            tune: (f32, f32, f32),
            body: PBody,
            cg: Vec3,
            two: bool,
            wiped: bool,
        }
        fn uid_of(infos: &[Info], idx: usize) -> u32 {
            infos.iter().find(|i| i.idx == idx).map_or(u32::MAX, |i| i.uid)
        }
        fn find(root: &[usize], mut x: usize) -> usize {
            while root[x] != x {
                x = root[x];
            }
            x
        }
        let mut infos: Vec<Info> = Vec::new();
        for (j, o) in self.cars.iter().enumerate() {
            if !o.placed || o.pu_ghost || o.park_held || o.rail.as_ref().is_some_and(|r| r.in_level) {
                continue;
            }
            let p = o.tuned.as_deref().unwrap_or(&o.model.params);
            let (Some(vt), Some(hull)) = (p.veh.as_ref(), p.hull.as_ref()) else { continue };
            let Some((body, cg)) = crate::physics::vehsim::island_body(&o.state, p, vt, dt) else { continue };
            let ws = o.state.veh.as_ref();
            infos.push(Info {
                uid: o.uid,
                idx: j,
                shape: static_shape(hull),
                class: if o.rail.is_some() || o.parked.is_some() { 6 } else { 8 },
                inflict: o.inflict,
                // (a parked car or a riding one the level does not hold as an ambient car - no ambient tune: inactive in the
                // original (level state 1, no collider), immovable to the step's solve; F-27744)
                parked: o.parked.is_some() || o.rail.is_some(),
                player: o.is_player,
                tune: (vt.t.s.bound_friction, vt.t.s.bound_elasticity, vt.t.s.head_on_cg_offset),
                body,
                cg,
                two: ws.is_some_and(|w| w.two != 0),
                wiped: ws.is_some_and(|w| w.wipe.is_some()),
            });
        }
        // (which steps take the level: a car with the original's body, not under MegaStealth - that one meets the map and
        // nothing else)
        for &(idx, ..) in jobs {
            let ghost = self.cars[idx].pu_ghost;
            if let Some(vs) = self.cars[idx].state.veh.as_deref_mut() {
                vs.loose.ghost = ghost;
            }
        }
        let lend: Vec<bool> = jobs
            .iter()
            .map(|&(idx, ..)| {
                let c = &self.cars[idx];
                shared && c.state.veh.is_some() && !c.pu_ghost && c.tuned.as_deref().unwrap_or(&c.model.params).veh.is_some()
            })
            .collect();
        let xs: Vec<RefCell<LevelX>> = jobs.iter().map(|_| RefCell::new(LevelX::default())).collect();
        let mut level = std::mem::take(&mut self.level);
        level.live = true;
        level.skip_pre = true;
        level.skip_end = true;
        level.ghost = false;
        let death = crate::physics::collision::pass_death_set(false);
        crate::physics::collision::pass_death_set(death);
        let mut contacts: Vec<(usize, u32)> = Vec::new();
        {
            let collider = &world.collider;
            type Step<'s> = Pin<Box<dyn Future<Output = ()> + 's>>;
            let mut steps: Vec<Option<Step<'_>>> = Vec::new();
            {
                let mut cars: Vec<Option<&mut CarInstance>> = self.cars.iter_mut().map(Some).collect();
                for (j, &(idx, input, _)) in jobs.iter().enumerate() {
                    let CarInstance { state, tuned, model, .. } = cars[idx].take().unwrap();
                    let p: &CarParams = tuned.as_deref().unwrap_or(&model.params);
                    if lend[j] {
                        steps.push(Some(Box::pin(state.step_in_level(p, input, collider, dt, &xs[j]))));
                    } else {
                        // (no level: the whole step now, in the cars' order)
                        state.step(p, input, collider, dt);
                        steps.push(None);
                    }
                }
            }
            let mut cx = Context::from_waker(Waker::noop());
            // (one poll of a step with the level lent: the level back after it - the step gives it at each yield and
            // at its end)
            let mut poll = |j: usize, level: &mut LooseState, steps: &mut Vec<Option<Step<'_>>>| {
                xs[j].borrow_mut().level = Some(std::mem::take(level));
                let done = matches!(steps[j].as_mut().unwrap().as_mut().poll(&mut cx), Poll::Ready(()));
                *level = xs[j].borrow_mut().level.take().expect("the level back from a car's step");
                if done {
                    steps[j] = None;
                    xs[j].borrow_mut().stage = Stage::Done;
                }
            };
            // (the other cars as bodies of car j's level: moved by their own steps where they were; the lower car of each
            // two makes their pair)
            let list = |j: usize| -> Vec<LiveVeh> {
                let me = jobs[j].0;
                let me_uid = uid_of(&infos, me);
                infos
                    .iter()
                    .filter(|i| i.idx != me)
                    .map(|i| {
                        let moved = jobs.iter().position(|x| x.0 == i.idx).and_then(|k| xs[k].borrow().moved);
                        let (body, two, wiped) = moved.unwrap_or((i.body, i.two, i.wiped));
                        let rider_touch = if i.class == 8 { crate::physics::loose::rider_touch(i.player, two, body.rows[1].y, wiped, i.tune) } else { None };
                        LiveVeh { id: i.uid, shape: i.shape, class: i.class, gravity: 1.0, cg: i.cg, level: i.uid as i64 - me_uid as i64, body, inflict: i.inflict, parked: i.parked, rider_touch, pair: i.idx > me }
                    })
                    .collect()
            };
            let n = jobs.len();
            // ---- every car's update and move ----
            for j in 0..n {
                if steps[j].is_some() {
                    poll(j, &mut level, &mut steps);
                }
            }
            // ---- every car's collide ----
            let mut first = true;
            for j in 0..n {
                if steps[j].is_some() && xs[j].borrow().stage == Stage::Moved {
                    level.later = !first;
                    first = false;
                    level.set_vehicles(list(j));
                    poll(j, &mut level, &mut steps);
                }
            }
            level.later = true;
            // ---- the islands: cars joined by their rows with each other or with the same loose object ----
            let waiting: Vec<bool> = (0..n).map(|j| steps[j].is_some() && xs[j].borrow().stage == Stage::Collided).collect();
            let mut root: Vec<usize> = (0..n).collect();
            let mut obj_first: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            for j in (0..n).filter(|&j| waiting[j]) {
                let (veh, objs) = {
                    let g = xs[j].borrow();
                    (g.touch_veh.clone(), g.touch_obj.clone())
                };
                let mut join = |a: usize, b: usize| {
                    let (ra, rb) = (find(&root, a), find(&root, b));
                    if ra != rb {
                        root[ra.max(rb)] = ra.min(rb);
                    }
                };
                for uid in veh {
                    if let Some(k) = (0..n).find(|&k| waiting[k] && uid_of(&infos, jobs[k].0) == uid) {
                        join(j, k);
                    }
                }
                for o in objs {
                    match obj_first.get(&o) {
                        Some(&k) => join(j, k),
                        None => {
                            obj_first.insert(o, j);
                        }
                    }
                }
            }
            // ---- each island solved once ----
            let obj_of = |level: &LooseState, uid: u32| level.objs.iter().position(|o| o.amb.is_none() && o.veh.is_some_and(|v| v.index == uid));
            for j in 0..n {
                if !waiting[j] || find(&root, j) != j {
                    continue;
                }
                let group: Vec<usize> = (0..n).filter(|&k| waiting[k] && find(&root, k) == j).collect();
                let lead = group.iter().copied().find(|&k| xs[k].borrow().rider).unwrap_or(group[0]);
                level.set_vehicles(list(lead));
                let lead_obj = obj_of(&level, uid_of(&infos, jobs[lead].0));
                let mut lent = Vec::new();
                for &k in group.iter().filter(|&&k| k != lead) {
                    let uid = uid_of(&infos, jobs[k].0);
                    let (share, part) = {
                        let mut g = xs[k].borrow_mut();
                        (g.share.take(), g.part.take())
                    };
                    match (share, part, obj_of(&level, uid)) {
                        (Some(mut e), Some(part), Some(o)) => {
                            e.uid = uid;
                            e.obj = o;
                            level.lend_extra(e, part, lead_obj);
                            lent.push(k);
                        }
                        (_, part, _) => xs[k].borrow_mut().part = part,
                    }
                }
                level.lead = lead_obj.filter(|_| !lent.is_empty());
                if !lent.is_empty() {
                    ISLANDS_OF_CARS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    log::debug!("one island of {} cars, solved in car {}'s step", lent.len() + 1, jobs[lead].0);
                }
                poll(lead, &mut level, &mut steps);
                level.lead = None;
                for (uid, ..) in level.vehicle_answers() {
                    contacts.push((jobs[lead].0, uid));
                }
                for (e, part) in level.return_extras(lead_obj) {
                    if let Some(&k) = lent.iter().find(|&&k| uid_of(&infos, jobs[k].0) == e.uid) {
                        let mut g = xs[k].borrow_mut();
                        // (a car the island's solve did not take - its rows with the island all disabled: its own solve)
                        if e.view.is_some() {
                            g.answer = Some(e);
                        }
                        g.part = Some(part);
                    }
                }
                // (the other cars' steps end with the island's answer)
                for &k in group.iter().filter(|&&k| k != lead) {
                    level.set_vehicles(list(k));
                    poll(k, &mut level, &mut steps);
                    for (uid, ..) in level.vehicle_answers() {
                        contacts.push((jobs[k].0, uid));
                    }
                }
            }
            // (a step still waiting - none is expected: to its end)
            for j in 0..n {
                while steps[j].is_some() {
                    level.set_vehicles(list(j));
                    poll(j, &mut level, &mut steps);
                }
            }
        }
        crate::physics::collision::pass_death_set(death);
        self.level = level;
        for (k, uid) in contacts {
            if let Some(j) = self.cars.iter().position(|c| c.uid == uid) {
                self.notify_contact(k, j);
            }
        }
    }

    /// Traffic into the physics level before the tick (loose.rs `Amb`): the cars gone leave it; each
    /// riding (or otherwise kinematic) car's instance where it is this frame, inactive; a car the game
    /// made physical (its footprint probe, a knock) is made active there.
    fn ambient_into_level(&mut self, dt: f32) {
        let Some(root) = traffic::DATA_ROOT.get().cloned() else { return };
        let alive: std::collections::HashSet<u32> = self.cars.iter().filter(|c| c.rail.is_some() || c.park_held).map(|c| c.uid).collect();
        self.level.ambient_keep(&|u| alive.contains(&u));
        // parked cars: the same ambient car with no behaviour, an inactive instance where it stands
        // (F-23864 -> F-42724 -> F-92604); any contact activates it, it rolls as the
        // passive collider and rests where it stops, still parked (F-38021: no behaviour, no
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
            // (a parked car the level made active: car +500 |= 0x10, kept until the car is made again - its render
            // update runs every frame from now on, traffic.rs `parked_render_runs`)
            if woke {
                if let Some(c) = self.cars.iter_mut().find(|c| c.uid == uid && c.park_held) {
                    c.amb_hit = true;
                }
            }
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
        let focus = self.amb_focus();
        // the drawn wheels' compression (F-37033, within 30 m horizontally): the collider wheel's
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
            // F-37033 with car +208, the instance position's change over the frame, along the
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
                // (a riding car's front wheels: its drawn steer, traffic.rs `Rider::steer`)
                // (a parked one's: traffic.rs `parked_render`)
                wh.steer = if i > 1 { 0.0 } else if c.park_held { wh.steer } else { c.rail.as_ref().map_or(0.0, |r| r.steer) };
            }
            c.state.relocate(m[3], rot);
            c.state.vel = v;
            c.state.ang_vel = w;
            c.placed = true;
        }
    }

    /// Put a car somewhere by hand (cutscenes): wheels follow, it stands still.
    /// The ambient manager's focus (its +192): the player's car; on the start screen, in front-end mode, the still
    /// camera's eye (AmbientMgr_SetFrontEnd - PEOPLE_TRAFFIC.md "Front-end mode"). The drawn wheels' spin (75 m) and
    /// the bodies' tilt (30 m) are measured from it (F-37033).
    pub fn amb_focus(&self) -> Vec3 {
        match self.amb_frontend.as_ref() {
            Some(fe) => fe.eye,
            None => self.cars.iter().find(|c| c.is_player).map_or(Vec3::ZERO, |c| c.state.pos),
        }
    }

    pub fn pose_car(&mut self, idx: usize, pos: Vec3, rot: Quat) {
        let c = &mut self.cars[idx];
        // (a car the physics has not run yet - one a story scene brings, whose clock stops the physics - has its
        // hubs at the world's origin: it was drawn without wheels, the tyres somewhere under it. Its wheels go
        // under their mounts at the suspension's rest length. approx.: no static sag)
        let unset = c.state.wheels.iter().all(|w| w.world_pos == Vec3::ZERO);
        // (its drawn wheels roll with the way it was moved along its forward - a car the scene drives in the
        // original is a riding car whose wheels turn at the road's speed; approx.: the posed car's rolling is not
        // taken from the original, it is the plain rolling of the move)
        if !unset {
            let r = c.model.params.wheel_radius.max(0.2);
            let turn = (pos - c.state.pos).dot(rot * Vec3::NEG_Z) / r;
            if turn.abs() < 5.0 {
                for wh in c.state.wheels.iter_mut() {
                    wh.spin = traffic::wrap_spin(wh.spin + turn);
                }
            }
        }
        c.state.relocate(pos, rot);
        if unset {
            let (mounts, rest) = (c.params().wheel_pos, c.params().susp_rest);
            for (w, a) in c.state.wheels.iter_mut().zip(mounts) {
                w.world_pos = pos + rot * (a - Vec3::Y * rest);
            }
        }
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

    /// The damage maps (F-11107, once a frame as the car is drawn): a car made while one of the 8
    /// is free gets it (F-69766) and keeps it until it goes; the queued stamps go into it, a car
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
                c.state.dmg.dents.clear();
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
        let mut batch = std::mem::take(&mut self.uniforms);
        let mut wheel_back = std::mem::take(&mut self.wheel_back);
        wheel_back.clear();
        wheel_back.resize(self.cars.len(), 0);
        let mut blur_wheels = std::mem::take(&mut self.blur_wheels);
        blur_wheels.resize(self.cars.len(), Vec::new());
        blur_wheels.truncate(self.cars.len());
        blur_wheels.iter_mut().for_each(|w| w.clear());
        let eye = self.wheel_eye;
        let player_tach_rpm = self.player_tach_rpm;
        for (ci, c) in self.cars.iter_mut().enumerate() {
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
            let siren = if (c.cop && (c.siren || self.sirens)) || (c.is_player && c.siren) { 1.0 + ((self.clock * 5.0) as i32 % 2) as f32 } else { 0.0 };
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
            // (MCLA_CAR_POSE_DUMP=<type name part>: that traffic type's drawn body matrix too, one update in 30)
            if !c.is_player && std::env::var("MCLA_CAR_POSE_DUMP").is_ok_and(|v| v.len() > 1 && c.model.name.contains(v.as_str())) {
                static M: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let k = M.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if k % 30 == 0 {
                    eprintln!("car pose {} n {}: x {:?} y {:?} z {:?} at {:?} wheel-at {:?}", c.model.name, k, m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate(), m.w_axis.truncate(), wheel_m.w_axis.truncate());
                }
            }
            // (the car's own lamp colours: its set-up's, else its traffic type's, else the Audi R8's - render/car_lamps.rs)
            let mut lamp_rows = (if c.is_player { player_lamps.clone() } else { None }).or_else(|| crate::render::car_lamps::for_car(&c.model.name, &c.vinyl_name)).map_or_else(|| crate::render::car_lamps::R8.uniform(), |t| t.uniform());
            // (a traffic car's lamp programs take each entry's state in the cover row's w - `AmbLamps`)
            // (a set-up car's too - `setup_lamps`, F-96984 - and the two states its own light on the
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
            // (the fifth: the side windows' clip plane in the world, ClipPlaneEq of F-50822 - normal the
            // car's up, through its origin + up x h, h = mcCarModelTune fWindowClipPlaneHeight (-1 on a car
            // without it: nothing cut); VS_Glass sends its distance for the windows 2 and 3 and
            // PS_GlassFront drops what is below. The rolled-down glass, moved by its bone, sinks under it.)
            let up = m.y_axis.truncate().normalize_or(Vec3::Y);
            let h = c.model.simtunes.as_ref().and_then(|s| s.window_clip).unwrap_or(-1.0);
            body_slots[4] = up.extend(-up.dot(m.w_axis.truncate() + up * h)).to_array();
            // (traffic and parked cars - their lamp rows marked ambient: F-73627 hands the type's
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
            // (+ 256: the in-car glass, only the player's car - the camera's target, F-95074)
            let int_glass = if self.interior_glass && c.is_player { 256.0 } else { 0.0 };
            // (a traffic or parked car's own light-slot order, F-99357: x = 1 when the hour's light is in the
            // second slot, the first holding a lamp or nothing; w = 1: given. Other cars take the player's.)
            // (and its slots' lights, F-99357: row 1 / 2 the slot not holding the hour's light - direction from the
            // car to its point, colour x strength -, row 3 / 4 the one holding it, or the second; the shader takes
            // them at night, by day the scene's light as the player's car)
            let amb_slots = if c.is_ambient() {
                let hs = if c.amb_slot[0] == 65545 { 0 } else { 1 };
                let ls = 1 - hs;
                let l = &c.amb_light;
                let dir = |k: usize| (l.point[k] - c.state.pos).normalize_or(Vec3::Y);
                let (ld, lc, hd, hc) = (dir(ls), l.col[ls] * l.int[ls], dir(hs), l.col[hs] * l.int[hs]);
                [[if hs == 1 { 1.0 } else { 0.0 }, 0.0, 0.0, 1.0], [ld.x, ld.y, ld.z, 0.0], [lc.x, lc.y, lc.z, 0.0], [hd.x, hd.y, hd.z, 0.0], [hc.x, hc.y, hc.z, 0.0]]
            } else {
                [[0.0; 4]; 5]
            };
            let body_u = ObjectUniform { model: m.to_cols_array_2d(), color: body_color, params: [lights, c.input.brake, c.state.damage, siren + reversing + 16.0 + 32.0 * s19d_look.0 as f32 + int_glass], dents: body_slots, color2: s19d_look.1, tint: s19d_look.2, interior: s19d_look.3, fade_min: c.model.min.extend(c.pu_mirror).to_array(), fade_size: (c.model.max - c.model.min).max(Vec3::splat(0.01)).extend(c.amb_lamps.tint).to_array(), fill: car_fill, lamp_a: lamp_rows.0, lamp_b: lamp_rows.1, area_a: s19d_look.4, area_b: s19d_look.5, slots: amb_slots };
            batch.push_object(&c.binding, &body_u);
            // (the side windows: the body's object moved by the window bone's clip)
            for (k, wb) in c.window_bindings.iter().enumerate() {
                let Some(clip) = c.model.windows[k].as_ref() else { continue };
                let wm = m * clip.delta(c.windows[k].clip_time(clip.duration));
                batch.push_object(wb, &ObjectUniform { model: wm.to_cols_array_2d(), ..body_u });
            }
            // (the moving damage parts: hood, trunk lid, rear and front bumper - damageparts.rs)
            let spoiler_d = c.model.spoiler.as_ref().map(|cl| cl.deltas(c.spoiler.clip_time(cl.duration))).unwrap_or_default();
            // (the convertible roof: which of its three parts shows, the clamshell's bones at the hatch's time; a part not
            // shown is drawn collapsed to a point)
            let (roof_shown, clam_d) = match (c.model.clamshell.as_ref(), c.model.skinned_top.as_ref()) {
                (Some(cl), Some(top)) => (c.roof.shown((cl.duration, top.duration)), cl.deltas(c.roof.hatch.clip_time(cl.duration))),
                _ => ([true; 3], Vec::new()),
            };
            if roof_shown[2] {
                if let Some(top) = c.model.skinned_top.as_ref() {
                    roof_skin_write(gpu, &c.model, top, c.roof.top.clip_time(top.duration));
                }
            }
            // (the dashboard needles, F-35036: the tach by the engine sound's slewed rpm - [[car +16] +204], the HUD's
            // feed - the speedo by the chassis speed x 2.237)
            let needle_d: [Vec<Mat4>; 3] = std::array::from_fn(|k| c.model.needles[k].as_ref().map(|cl| {
                let rpm = if c.is_player { player_tach_rpm.unwrap_or(c.state.rpm) } else { c.state.rpm };
                let d = if k == 1 { cl.deltas(needle_time(cl.duration, c.state.speed() * 2.237, false)) } else { cl.deltas(needle_time(cl.duration, rpm, true)) };
                // (the add-on gauge's bones are its own skeleton's: moved about its placement)
                if k == 2 { d.iter().map(|x| c.model.needle_place * *x * c.model.needle_place.inverse()).collect() } else { d }
            }).unwrap_or_default());
            for (k, pb) in c.part_bindings.iter().enumerate() {
                let parts = &c.state.parts;
                let pm = match k {
                    // (a bone of a dashboard needle)
                    n if n + 3 >= NEEDLE_GROUP as usize => {
                        let i = n + 3 - NEEDLE_GROUP as usize;
                        needle_d[(i / NEEDLE_BONES).min(2)].get(i % NEEDLE_BONES).map_or(m, |d| m * *d)
                    }
                    // (the roof's parts and the clamshell's bones)
                    r @ 26..=28 => if roof_shown[r - 26] { m } else { Mat4::ZERO },
                    j if j >= 29 => clam_d.get(j - 29).map_or(m, |d| m * *d),
                    // (a bone the auto spoiler's clip moves)
                    s if s >= 18 => spoiler_d.get(s - 18).map_or(m, |d| m * *d),
                    // (the intercooler: the front bumper's pose, on its own bone)
                    17 => parts.bumpers[1].as_ref().map_or(m, |b| b.slot_draw(&m, c.model.slot_bones[2].as_ref().unwrap_or(&b.m_rest), b.detached.filter(|_| !(self.hide_player && c.is_player)).map(|l| Mat4::from_rotation_translation(l.rot, l.pos)))),
                    // (a light bar lens, on its turning pivot)
                    l if l >= 10 => c.model.cop_lights.as_ref().filter(|cl| l - 10 < cl.lamps.len()).map_or(m, |cl| m * cl.lens_delta(l - 10, c.light_t)),
                    0 => c.model.hood_slot.map_or(m, |s| m * parts.hood_delta(&s)),
                    1 => c.model.trunk_bone.map_or(m, |b| m * parts.trunk_delta(&b)),
                    // (a traffic model's own wheel: turned about its hub - steer, then the roll -
                    // with the body untilted; the original's wheel bones, F-37033)
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
                    // (the fallen piece is drawn where its body is)
                    _ => parts.bumpers[k - 2].as_ref().map_or(m, |b| b.slot_draw(&m, c.model.slot_bones[k - 2].as_ref().unwrap_or(&b.m_rest), b.detached.filter(|_| !(self.hide_player && c.is_player)).map(|l| Mat4::from_rotation_translation(l.rot, l.pos)))),
                };
                let mut pu = ObjectUniform { model: pm.to_cols_array_2d(), ..body_u };
                // (a traffic model's own wheel: its rim's and matte's programs dent it too, the vertex taken in its
                // wheel bone's frame - every va_ wheel bone is its hub with no turn or scale, so p - hub; the third
                // slot carries the hub)
                if (4..10).contains(&k) {
                    if let Some(h) = c.model.own_wheels.and_then(|h| h[k - 4]) {
                        pu.dents[2] = [h.x, h.y, h.z, 1.0];
                    }
                }
                batch.push_object(pb, &pu);
            }
            for (i, b) in c.wheel_bindings.iter().enumerate() {
                let left = i % 2 == 0;
                let mut steer = Quat::from_rotation_y(-pose.steer[i]);
                // (F-47822: a car's wheel is drawn from its matrix among the car's parts, kept under the fender, its
                // turn cut by TireSteerLimit and shifted by TireSteerOffset - see wheel_pose.rs; the shift is added to
                // the drawn hub. A bike's front wheel is turned by the eased steering, steer_parts.rs, instead of its
                // own. MCLA_WHEEL_POSE_OLD=1: the steer alone, as before)
                let mut shift = Vec3::ZERO;
                if let Some((rot, d)) = drawn_wheel_pose(c, i) {
                    steer = rot;
                    shift = d;
                }
                // the rim models face -x (hub cap at x = -0.24): left wheels as modelled, right
                // wheels turned around; rolling forward (-z) is a negative rotation about x
                let spin = Quat::from_rotation_x(if left { -pose.spin[i] } else { pose.spin[i] });
                let flip = if left { Quat::IDENTITY } else { Quat::from_rotation_y(std::f32::consts::PI) };
                let mut local = pose.hubs[i] + shift;
                if c.model.bike {
                    // Bike physical wheels and their visual mirrors share the center line.
                    let lo = pose.hubs[i ^ 1];
                    local = (local + lo) * 0.5;
                }
                let flip = if c.model.bike { Quat::IDENTITY } else { flip };
                let spin = if c.model.bike { Quat::from_rotation_x(-pose.spin[i]) } else { spin };
                // (the original draws each axle's own tyre: as wide as fitted, 6.35 mm larger than its physics radius.
                // A bike's (F-43091 with F-54299): x twice its axle's bone width, y / z its physics radius doubled, the
                // radius the wheel had at the set-up, F-74884 - the rim then 0.82 of that)
                let (tyre_w, tyre_r) = if c.model.bike {
                    let a = (i >= 2) as usize;
                    let r = c.params().veh.as_ref().map(|v| if a == 0 { v.t.wheel_front.radius } else { v.t.wheel_back.radius }).filter(|r| *r > 0.1).unwrap_or(c.model.params.wheel_radius);
                    (c.model.bike_wheel_width[a], r)
                } else {
                    c.params().veh.as_ref().filter(|v| v.tyre_width[0] > 0.0).map(|v| if i < 2 { (v.tyre_width[0], v.t.wheel_front.radius + 0.00635) } else { (v.tyre_width[1], v.t.wheel_back.radius + 0.00635) }).unwrap_or((c.model.params.wheel_width, c.model.params.wheel_radius))
                };
                let frame = wheel_m * Mat4::from_translation(local) * Mat4::from_quat(steer * flip * spin);
                let wm = frame * Mat4::from_scale(Vec3::new(tyre_w * 2.0, tyre_r * 2.0, tyre_r * 2.0));
                // (F-57869: the rim is drawn only while the camera is outside the wheel - the way from the eye to the
                // hub against the wheel's x axis, which points in on every wheel, over 0; at 0 or below the
                // wheel's back takes its place. Not on a bike. The threshold is 0.04 only when the car job's byte +41
                // (F-11352) is off: that needs a racer who is not the local player with racer+25 set, which only the
                // online remote-player paths do (F-62113) - 0 for every car offline, read live on four cars. The
                // mirror flag (k < 0) and the pool draws after it run only with that byte off too: not ported, online)
                // (F-49519: every wheel of a set-up car that has its wheel_N bone - the entries +4192 / +4208, F-13644's
                // lookups through F-59267. A bike (F-54299) has wheel_0 and wheel_2: the car's disc on its
                // two wheels - recorded, stencil 8 and 12 after the body's 4 - and `blur_wheel_bike`, which F-31291
                // loads, is never drawn: only its unload reads it)
                if (!c.model.bike || i % 2 == 0) && c.model.own_wheels.is_none() && i < 4 && !(self.hide_player && c.is_player) {
                    blur_wheels[ci].push((wm, if left { -pose.spin[i] } else { pose.spin[i] }));
                }
                if !c.model.bike {
                    let to_hub = (wm.w_axis.truncate() - eye).normalize_or_zero();
                    if to_hub.dot(wm.x_axis.truncate().normalize_or_zero()) <= 0.0 && i < 8 {
                        wheel_back[ci] |= 1 << i;
                    }
                }
                // (a wheel has no dents: the first three slots carry its rim's finish)
                let mut finish = [[0.0; 4]; 6];
                if c.is_player {
                    finish[..3].copy_from_slice(&self.player_rim_finish);
                } else if let Some(l) = c.look.as_ref() {
                    finish[..3].copy_from_slice(&l.rim_finish);
                }
                // (the fourth: the tyre's dirt - TireDirtColor.w, its hub's scrape sum at most 1 - and a
                // mark that this is a wheel)
                // (the third: tireMotionBlur as the tyre draw F-11293 sets it for this wheel, 0.08 x |w|, w = the wheel's
                // turn over an update / 2.0195954 (+440 of its block, F-47822); the update taken as a 1 / 30 s frame, as for
                // the blur pass (render/mb.rs). city.wgsl reads the tyre's maps with it. Traffic: 0, as recorded for PSAmbientTire)
                let tmb = if c.model.bike || i >= 4 || c.is_ambient() { 0.0 } else { 0.08 * (c.pose_history.spin_rate()[i] / 30.0 / 2.019_595_4).abs() };
                finish[3] = [c.state.dmg.rim_scuff(i), 1.0, tmb, 0.0];
                // (the fifth: the tyre's bead. F-11293 hands VS_ParallaxRubber2 innerRadiusScale, the car's +2368 / +2372 for
                // its front / rear axle, which F-43091 sets from the set-up - WheelFit::bead; a bike's 0 (+2372 = -0.0525 with
                // its byte +6481, not read). city.wgsl moves a tyre vertex at the model's radius r by sat(1 - (r - 0.406) / 0.094) k
                // outward, from the bead the tyre was built with (WheelModel::tyre_bead). A car without a set-up: its build's
                // bead, as before. Traffic: none - VSAmbientTire. MCLA_TYRE_BEAD_OLD=1: none)
                if let (Some(w), false, true) = (c.wheels.as_ref().or(c.model.wheels.as_ref()).or(self.wheels.as_ref()), c.is_ambient() || i >= 4, std::env::var("MCLA_TYRE_BEAD_OLD").is_err()) {
                    // (only where the rim drawn is the set-up's: the front axle's bead equals the one the wheel was built with, or
                    // the player's, whose set-up is its record's DIMENSIONS, the rim size chosen among them. city.wgsl also draws
                    // the rim at (0.406 + k) / built of its size: the original sizes a rim from its axle's diameter alone, 1.25 x
                    // rimR (F-43091), where the port's rim grows with the axle's tyre)
                    let fit = c.setup.filter(|_| c.is_player).or(c.setup.or(c.model.fit).filter(|f| (f.bead[0] + 0.406 - w.tyre_bead).abs() < 1e-4)).filter(|_| !c.model.bike);
                    let k = if c.model.bike { 0.0 } else { fit.map_or(w.tyre_bead - 0.406, |f| f.bead[(i >= 2) as usize]) };
                    finish[4] = [w.tyre_bead, k, 1.0, 0.0];
                }
                // (params.y: the rim effect's useGradient.x - F-91597 hands F-96332 "not a motorcycle" (F-54299): 100.0 on a
                // motorcycle, 0.0 on a car: city.wgsl's fade along the axle is taken at x - useGradient.x, so a bike's rims never fade)
                let use_gradient = if c.model.bike { 100.0 } else { 0.0 };
                batch.push_object(b, &ObjectUniform { model: wm.to_cols_array_2d(), color: [0.8, 0.8, 0.8, 1.0], params: [night, use_gradient, 0.0, 0.0], dents: finish, fill: car_fill, ..Default::default() });
                // the brake (F-43091 -> F-57869, `brake_place`): the rotor in the wheel's turning frame, the caliper in its
                // hub's, both sized by the axle's rim radius - the set-up's (WheelFit::rim_radius). (approx.) a car without a
                // set-up: the radius of the rim the port draws, its rim ratio of the tyre
                // A motorcycle (F-43091's bike branch, `bike_brake_place`): on wheels 0 and 2 the caliper in the hub's frame and
                // the two discs A / B either side of the wheel, b = +4100 / +4104 (the ctor's 0.35 / 0.31, never rewritten),
                // d = the axle's bone span, the axle's fRotorOffset / fRotorSize; rotor B goes in the unused odd wheel's binding
                if c.model.bike && i % 2 == 0 && (c.model.brakes.is_some() || c.brake.is_some() || (c.is_player && self.player_brake.is_some())) {
                    let axle = (i >= 2) as usize;
                    let tune = c.model.simtunes.as_ref().and_then(|s| s.rotor);
                    let (size, off) = tune.map_or((1.0, 0.0), |r| (r.size[axle], r.offset[axle]));
                    let hub = wheel_m * Mat4::from_translation(local) * Mat4::from_quat(steer);
                    let (cal, ra, rb) = bike_brake_place(hub, frame, [0.35f32, 0.31][axle], c.model.bike_wheel_width[axle], off, size);
                    let mut rf = finish;
                    rf[3][2] = 0.0;
                    rf[4] = [0.0; 4];
                    for (k, m) in [(i, ra), (i + 1, rb)] {
                        if let Some(rbd) = c.rotor_bindings.get(k) {
                            batch.push_object(rbd, &ObjectUniform { model: m.to_cols_array_2d(), color: [0.8, 0.8, 0.8, 1.0], params: [night, 0.0, 0.0, 0.0], dents: rf, fill: car_fill, ..Default::default() });
                        }
                    }
                    if let Some(cb) = c.caliper_bindings.get(i) {
                        let mut mark = [[0.0; 4]; 6];
                        mark[3] = [0.0, 1.0, 0.0, 0.0];
                        batch.push_object(cb, &ObjectUniform { model: cal.to_cols_array_2d(), color: [0.8, 0.8, 0.8, 1.0], params: [night, 0.0, 0.0, 0.0], dents: mark, fill: car_fill, ..Default::default() });
                    }
                } else if !c.model.bike && (c.model.brakes.is_some() || c.brake.is_some() || (c.is_player && self.player_brake.is_some())) {
                    let axle = (i >= 2) as usize;
                    let rim_r = c.setup.or(c.model.fit).map_or(c.model.rim_ratio * tyre_r, |f| f.rim_radius[axle]);
                    // (no bead pull and no tyre blur: those are the tyre's)
                    let mut rf = finish;
                    rf[3][2] = 0.0;
                    rf[4] = [0.0; 4];
                    if let Some(rb) = c.rotor_bindings.get(i) {
                        batch.push_object(rb, &ObjectUniform { model: brake_place(frame, rim_r).to_cols_array_2d(), color: [0.8, 0.8, 0.8, 1.0], params: [night, 0.0, 0.0, 0.0], dents: rf, fill: car_fill, ..Default::default() });
                    }
                    if let Some(cb) = c.caliper_bindings.get(i) {
                        // (a right hub turned over: x and y reversed)
                        let side = if left { Quat::IDENTITY } else { Quat::from_rotation_y(std::f32::consts::PI) * Quat::from_rotation_x(std::f32::consts::PI) };
                        let cm = brake_place(wheel_m * Mat4::from_translation(local) * Mat4::from_quat(steer * side), rim_r);
                        // (the wheel's mark, no dirt: its maps are decoded as the wheel's, city.wgsl car_raw_tex)
                        let mut mark = [[0.0; 4]; 6];
                        mark[3] = [0.0, 1.0, 0.0, 0.0];
                        batch.push_object(cb, &ObjectUniform { model: cm.to_cols_array_2d(), color: [0.8, 0.8, 0.8, 1.0], params: [night, 0.0, 0.0, 0.0], dents: mark, fill: car_fill, ..Default::default() });
                    }
                }
            }
        }
        gpu.flush_uniforms(&mut batch);
        self.uniforms = batch;
        self.wheel_back = wheel_back;
        self.blur_wheels = blur_wheels;
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
            let siren = if (c.cop && (c.siren || self.sirens)) || (c.is_player && c.siren) { 1 + (self.clock * 5.0) as i32 % 2 } else { 0 };
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
    /// camera's axis `axis` from `eye`. The flags as F-46062 sets them, for every traffic car each
    /// frame: lights on at night, the brake lamp (`traffic::brake_lamp`, F-56024), the turn signals
    /// with the car's blink (`traffic::signals`, F-41231 / F-81289), reversing; a trailer with its cab
    /// takes the cab's five flags instead (F-46062 copies 0x40, 0x20, 0x10, 0x8 and 0x4 from the cab's
    /// record). Parked cars have every flag off, as recorded.
    pub fn traffic_lamps(&mut self, eye: Vec3, axis: Vec3, frustum: &crate::render::Frustum, night: f32, dt: f32) {
        let alpha = self.render_alpha;
        let lights_off = self.lights_off;
        let hour = self.hour;
        let nav = traffic::rail_nav_made();
        // (the hour window of F-75961; `night` over 0.5 inside 6 .. 18 h is the weather's own term)
        let wet = night > 0.5 && (6.0..=18.0).contains(&hour);
        let mut flags: Vec<[bool; 5]> = Vec::with_capacity(self.cars.len());
        for c in self.cars.iter_mut() {
            // (the blink: + dt each update, back to 0 past 0.75 + the car's +3576 - F-46062)
            let a = &mut c.amb_lamps;
            a.blink += dt;
            if a.blink > a.blink_wrap + 0.75 {
                a.blink = 0.0;
            }
            let driven = c.parked.is_none();
            let due = traffic_lights_due(hour, c.amb_lamps.hour_offset, wet) && c.removing.map_or(true, |f| f >= 0.1);
            let (sa, sb) = match (nav, c.rail.as_ref()) {
                (Some(nav), Some(r)) if driven => traffic::signals(nav, r, c.state.pos, c.amb_lamps.blink, c.amb_lamps.blink_on),
                _ => (false, false),
            };
            flags.push([driven && !lights_off && due, driven && c.rail.as_ref().is_some_and(traffic::brake_lamp), sa, sb, driven && c.state.gear < 0]);
        }
        // a trailer with its cab: the cab's flags
        for i in 0..self.cars.len() {
            let Some(r) = self.cars[i].rail.as_ref().filter(|r| r.ty.trailer) else { continue };
            let Some(id) = r.partner else { continue };
            if let Some(k) = self.cars.iter().position(|c| c.rail.as_ref().is_some_and(|x| x.rng_id() == id && x.state != 0)) {
                flags[i] = flags[k];
            }
        }
        for (c, flags) in self.cars.iter_mut().zip(flags) {
            if c.is_player {
                continue;
            }
            let Some(rows) = crate::render::car_lamps::for_car(&c.model.name, &c.vinyl_name).filter(|t| t.ambient) else { continue };
            let pos = c.render_pose(alpha).pos;
            c.amb_lamps.tint = 0.25 + 1.5 * ((pos.distance(eye) / 30.0).min(1.0) - 0.5).clamp(0.0, 1.0);
            // (the flag is the car's, whether or not the car is in the view)
            c.amb_lamps.head_on = flags[0];
            let d = (pos - eye).dot(axis);
            let r = Vec3::splat(c.model.params.body_half.length() + 1.0);
            if d > TRAFFIC_LAMP_REACH || !frustum.intersects_aabb(pos - r, pos + r) {
                continue;
            }
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
    /// as mcCar's update keeps them (F-16712 -> F-10129 eases, F-96984 writes the light
    /// set's levels at +400 and the lamp programs' states), in `AmbLamps`. The targets of
    /// F-10129: head lamps `lights_due` (the hour past the light manager's 18.0 or before its 6.0,
    /// or the Stormy or Foggy weather's weight over 0.85 - the caller's); braking the driver input's
    /// brake (+16: the player's pad output, a computer driver's own) over 0.001; reversing the
    /// gearbox in its first (reverse) gear; in the garage (light manager byte 14546) the head lamps
    /// follow the view instead (the caller's `lights_due`, `garage_lamps_due`). All three need the car's
    /// switch (`AmbLamps::off`, +597) and no Stealth (F-54047: the power-up's state 3 or 14).
    pub fn setup_lamps(&mut self, lights_due: bool, dt: f32) {
        let lights_off = self.lights_off;
        let sirens = self.sirens;
        let flash = self.player_flash;
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
            // (all three need the car's switch +597; Stealth / MegaStealth - F-54047 - puts them out)
            let on = !c.amb_lamps.off && !c.pu_stealth;
            // (the flash, +240 at rate 9: the pad input's target, not switched by +597 - F-44589)
            let flags = [on && lights_due && !lights_off, on && brake > 0.001, false, c.is_player && flash, on && c.state.gear < 0];
            // (the siren state +300, rate 18: its target +304 is what the car's police tactic writes each update -
            // F-63286 -> F-78189, 1 in pull-over / pursuit / arrow trap, cleared at the tactic's end; the
            // port's police has one state for all its cars: `sirens` on a cruiser)
            // (the player's cruiser: its HEADLIGHTS toggle, main.rs)
            let siren = (c.cop && (c.siren || sirens)) || (c.is_player && c.siren);
            let a = &mut c.amb_lamps;
            for (k, (v, on)) in a.s.iter_mut().zip(flags).enumerate() {
                *v = if k == 2 { ease_at(*v, siren, 18.0, dt) } else { traffic_ease(*v, on, dt) };
            }
            a.siren_on = siren;
            for i in 0..crate::render::car_lamps::IDS {
                a.lens[i] = setup_state(a.s, att[i]);
            }
            a.levels = Some(a.lens);
            // (the 20th entry, the shift light: the engine's revs - F-12794, engine +8 x 9.5493 - at or past
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
    /// original's F-96521): the lamp's place, the way it faces, its lit colour, and its
    /// level x the kind's factor of the executable's two tables (0x827E7688 other cars: brake 2,
    /// tail 2, head 2, reversing 0.528, third brake 0.346; 0x827E76D8 the player's: brake 0.3,
    /// tail 1.004, head 2, the others 1).
    /// (place, facing, colour, level, the lamp's id for its light trail: the car's uid and the lamp's number)
    pub fn halo_lamps(&self, night: f32, cam: Vec3) -> Vec<(Vec3, Vec3, [f32; 3], f32, u64, Option<f32>)> {
        let mut out = Vec::new();
        let old_rule = std::env::var("MCLA_GLOW_OLD").is_ok();
        for (ci, c) in self.cars.iter().enumerate() {
            let pose = c.render_pose(self.render_alpha);
            if pose.pos.distance_squared(cam) > 260.0 * 260.0 || (self.hide_player && c.is_player) {
                continue;
            }
            let lights = if c.parked.is_some() || self.lights_off { 0.0 } else { night };
            let pl = c.is_player;
            // The original's light set (F-96521 walks it; the player's read out of the running game,
            // work/lighting/fe_lset.py): one record for every `glow<kind>0` bone of the fitted parts'
            // skeletons (the R8: four glowbrake0, two glowhead0, two glowhigh0, two glowreverse0, one
            // glowthirdbrake0), its facing the bone's z row. Levels of the standing player's car at 23:30:
            // brake 0.25, tail 0.25, signal 1, head 1, high beam 0, fog 1, reversing / third brake 0.
            // Traffic takes its own levels (`AmbLamps`, F-81135: by table entry, x the glow file's
            // factor of the kind) and only once it has been drawn near; its glows reach as far as the
            // original's test in F-62563: |camera - car|^2 - radius^2 <= 250^2.
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
            // (a set-up car: its light set's levels as F-96984 writes them - `setup_lamps` - x the
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
            let lamp = |n: usize| ((c.uid as u64) << 12) | n as u64;
            // (the reflection pass, mode 2 of F-27833: a traffic car whose lights are on and whose LOD is not -1 -
            // no 50 m rule - sets the world matrix to the mirror about its origin height and runs F-62563 ->
            // F-96521 with refl = 1; the player's car has no reflection rectangles in the recordings)
            let refl_y = (traffic.is_some() && self.lod_pick.get(ci).copied().unwrap_or(-1) >= 0).then_some(pose.pos.y);
            for (n, (id, local, facing)) in c.model.glow_bones.iter().enumerate() {
                let k = level(*id);
                if k <= 0.0 || old_rule {
                    continue;
                }
                // (F-96521 takes the record's bone from the car's posed skeleton (F-25478): a light bar's lamp is on its
                // turning matrix, as in F-73214's own glows - `police_glows`)
                let (local, facing) = match (*id, c.model.cop_lights.as_ref()) {
                    (13 | 14, Some(cl)) => cl.lamps.iter().position(|l| l.4.distance(*local) < 0.01)
                        .map(|li| cl.lamp_at(li, c.light_t))
                        .map_or((*local, *facing), |m| (m.w_axis.truncate(), -m.z_axis.truncate().normalize_or_zero())),
                    _ => (*local, *facing),
                };
                // (a lamp of a bumper part: F-96521 skips it while the light set's byte of its side is 0 - [1, +6482 == 0,
                // +6483 == 0], F-16712 setting +6482 / +6483 while an end of the front / rear bumper is broken; until
                // then its bone stands in the bumper's slot matrix as the bumper hangs, M_slot' = A M_slot)
                let side = c.model.glow_sides.get(n).copied().unwrap_or(0);
                let (local, facing) = match side {
                    1 | 2 => {
                        let i = if side == 1 { 1 } else { 0 };
                        match c.state.parts.bumpers[i].as_ref() {
                            Some(b) if b.a.state == 2 || b.b.state == 2 => continue,
                            Some(b) => {
                                let d = b.draw_delta_on(c.model.slot_bones[i].as_ref().unwrap_or(&b.m_rest));
                                (d.transform_point3(local), d.transform_vector3(facing).normalize_or_zero())
                            }
                            None => (local, facing),
                        }
                    }
                    _ => (local, facing),
                };
                out.push((pose.pos + pose.rot * local, pose.rot * facing, colour(*id, dflt(*id)), k, lamp(n), refl_y));
            }
            for (n, (id, local)) in c.model.lamps.iter().enumerate() {
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
                    out.push((pose.pos + pose.rot * Vec3::new(0.701 * sx, 0.747, 1.803), out_dir, colour(*id, dflt(*id)), k, lamp(2048 + 2 * n), refl_y));
                    out.push((pose.pos + pose.rot * Vec3::new(0.526 * sx, 0.773, 1.902), out_dir, colour(*id, dflt(*id)), k, lamp(2049 + 2 * n), refl_y));
                    continue;
                }
                out.push((pose.pos + pose.rot * *local, out_dir, colour(*id, dflt(*id)), k, lamp(1024 + n), refl_y));
            }
        }
        out
    }

    /// The police lamps' glows (F-73214 in the glow pass, not F-96521): while a car's siren target
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
            let spin_t = c.light_t;
            for (id, local, facing) in c.model.glow_bones.iter().filter(|g| g.0 == 13 || g.0 == 14) {
                let i = *id as usize;
                let col = rows.as_ref().map_or([1.0; 3], |t| [0, 1, 2].map(|k| (t.bulb[i][k] * t.cover[i][k] + 0.5).clamp(0.0, 1.0)));
                // (the glow on the lamp's turning matrix: its place and its -z, F-73214)
                let spun = c.model.cop_lights.as_ref().and_then(|cl| cl.lamps.iter().position(|l| l.4.distance(*local) < 0.01).map(|li| cl.lamp_at(li, spin_t)));
                let (l, f) = spun.map_or((*local, *facing), |m| (m.w_axis.truncate(), -m.z_axis.truncate().normalize_or_zero()));
                out.push((pose.pos + pose.rot * l, pose.rot * f, col, c.amb_lamps.s[2]));
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
        self.draw_x(pass, gpu, blend_pass, draws, range, frustum, false, None);
    }

    /// `draw` with the set-up cars' back faces culled by their batches' cull class (`GpuBatch::cull`), as the
    /// original's car draw (F-92759) does; `mirrored`: the view is mirrored, the other face culled. Into a pass of
    /// the variant in force (`ms::current`).
    pub fn draw_culled<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, blend_pass: bool, draws: &mut usize, range: Option<(Vec3, f32)>, frustum: Option<&crate::render::Frustum>, mirrored: bool) {
        self.draw_x(pass, gpu, blend_pass, draws, range, frustum, false, Some(mirrored));
    }

    /// The cars into a sun shadow pass (the cascades, the near map). Traffic and parked cars cast none: the ambient
    /// cars' pass dispatcher (F-27833) sends the shadow modes 4 / 8 to F-89353, which never ran in any recording
    /// (registry hits 0), and the recorded frames (work/traffic-detail/fix/shadowchk.py) hold no depth draw of the
    /// food truck's meshes - only its occlusion blob (PS_CalcShadowsQuad). (MCLA_TRAFFIC_CASTS=1: as before)
    pub fn draw_shadow<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, draws: &mut usize, range: Option<(Vec3, f32)>) {
        self.draw_x(pass, gpu, false, draws, range, None, traffic_casts_none(), None);
    }

    /// The wheels' blur discs (F-49519, after the cars' glass): for every wheel `blur_wheels` holds, the shared
    /// `blur_wheel` disc in the rim's place, into the stencil alone - colour writes off, depth tested, not written,
    /// no cull - with the wheel's motion-blur id (render/mb.rs `wheel_stencil_of`), so the streak pass moves the rim's
    /// face with the wheel's own record. None in the garage, none for the car in the driver's view (F-73214). The
    /// cars as `draw_culled` takes them (reach, box), the wheels' own reach aside: F-49519 has none. Into a pass of the
    /// variant in force. `stealth`: the cars under Stealth instead, whose discs F-73214 draws in their own phase
    /// (0x400000) after their bodies, in the garage and the driver's view too (pu_fx.rs).
    pub fn draw_wheel_blur<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, draws: &mut usize, range: Option<(Vec3, f32)>, frustum: Option<&crate::render::Frustum>, garage: bool, stealth: bool) {
        if (garage && !stealth) || wheel_blur_off() {
            return;
        }
        let mut set = false;
        for (ci, c) in self.cars.iter().enumerate() {
            let Some(blur) = self.blur_wheels.get(ci).filter(|w| !w.is_empty()) else { continue };
            if c.pu_stealth != stealth || (!stealth && c.is_player && self.interior_glass) {
                continue;
            }
            let Some(disc) = c.wheels.as_ref().or(c.model.wheels.as_ref()).or(self.wheels.as_ref()).and_then(|w| w.blur.as_ref()) else { continue };
            let pos = c.render_pose(self.render_alpha).pos;
            if range.is_some_and(|(eye, max)| pos.distance(eye) > max) {
                continue;
            }
            if let Some(f) = frustum {
                let r = Vec3::splat(c.model.params.body_half.length() + 1.0);
                if !f.intersects_aabb(pos - r, pos + r) {
                    continue;
                }
            }
            if !set {
                pass.set_pipeline(&gpu.pipeline_stencil_only);
                set = true;
            }
            pass.set_bind_group(1, disc.bind.as_ref(), &[]);
            pass.set_bind_group(2, &gpu.mat_bgs[&(disc.mode as u32)], &[]);
            pass.set_vertex_buffer(0, disc.vb.slice(..));
            pass.set_index_buffer(disc.ib.slice(..), wgpu::IndexFormat::Uint32);
            for i in 0..blur.len() {
                let Some(wb) = c.wheel_bindings.get(blur_binding(c.model.bike, i)) else { continue };
                pass.set_bind_group(3, &wb.bg, &[]);
                pass.set_stencil_reference(crate::render::mb::wheel_stencil_of(ci, i));
                pass.draw_indexed(0..disc.count, 0, 0..1);
                *draws += 1;
            }
        }
    }

    fn draw_x<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, blend_pass: bool, draws: &mut usize, range: Option<(Vec3, f32)>, frustum: Option<&crate::render::Frustum>, skip_ambient: bool, cull: Option<bool>) {
        // (the opaque pass of `draw_culled`: each batch with its material's own pipeline once it is built - `car_pipe`)
        let own = cull.is_some() && !blend_pass;
        let mut pick = (0u8, u32::MAX, false);
        for (ci, c) in self.cars.iter().enumerate() {
            // (step 19g: a car under Stealth is not drawn in either pass of its own - pu_fx.rs)
            if c.pu_stealth {
                continue;
            }
            if skip_ambient && c.is_ambient() {
                continue;
            }
            let Some((batches, groups)) = self.lod_batches(ci) else { continue };
            // (a bumper with an end loose or torn off - +512 / +516 of its object: drawn with both sides; 0 rear, 1 front)
            let hangs = |k: usize| c.state.parts.bumpers[k].as_ref().is_some_and(|b| b.a.state != 0 || b.b.state != 0);
            let two_sided = |b: &GpuBatch| match b.cull {
                1 => false,
                3 => hangs(1),
                4 => hangs(0),
                _ => true,
            };
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
            // (the car's stencil id for the motion blur: 4 k, + 1 when not a player's - render/mb.rs)
            pass.set_stencil_reference(crate::render::mb::stencil_of(ci));
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
                        if b.mode != MatMode::CarGlass || b.shader.to_ascii_lowercase().contains("bumped") || b.bucket == 1 {
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
            // A traffic car's blended bucket (`GpuBatch::bucket`, prep_car), in the main alpha pass, in its model's order
            // as F-73627 draws it: blended (SRC_ALPHA / INV_SRC_ALPHA), depth writes on, back faces culled (recorded
            // 0x07060706 / 0x00708767 / 0x00018002); city.wgsl drops the pixels whose alpha is not above 0 (alpha test
            // GREATER 0, 0x8700000c). Each window twice, back faces first (cull 0x00018006), then front faces.
            if blend_pass && batches.iter().any(|b| b.bucket == 1) {
                for (bi, b) in batches.iter().enumerate() {
                    if b.bucket != 1 {
                        continue;
                    }
                    // (test hook MCLA_AMB_GLASS_SKIP=1: the traffic glass left out, to compare the colour behind it with
                    // the original's - the recorder's "skip <PSAmbientGlassFront>" line)
                    static SKIP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                    if b.mode == MatMode::CarGlass && *SKIP.get_or_init(|| std::env::var_os("MCLA_AMB_GLASS_SKIP").is_some()) {
                        continue;
                    }
                    if amb_skip(&c.model.name, bi, b) {
                        continue;
                    }
                    pass.set_bind_group(3, c.group_object(groups[bi]), &[]);
                    pass.set_bind_group(1, b.bind.as_ref(), &[]);
                    pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                    pass.set_vertex_buffer(0, b.vb.slice(..));
                    pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                    if b.mode == MatMode::CarGlass {
                        pass.set_pipeline(&gpu.pipeline_glass_back);
                        pass.draw_indexed(0..b.count, 0, 0..1);
                        *draws += 1;
                    }
                    // (a set-up car's batch of a slot drawn with both sides - the hood's BumpSpecAlpha: cull 0x18000)
                    let both = cull.is_some() && b.cull != 0 && b.mode != MatMode::CarGlass && two_sided(b);
                    pass.set_pipeline(if both { &gpu.pipeline_pane } else { &gpu.pipeline_glass_front });
                    pass.draw_indexed(0..b.count, 0, 0..1);
                    *draws += 1;
                }
                pass.set_pipeline(&gpu.pipeline_blend);
            }
            for (bi, b) in batches.iter().enumerate() {
                if b.mode.blended() != blend_pass || b.bucket == 1 {
                    continue;
                }
                if let (Some(m), false) = (cull, blend_pass) {
                    let want = !two_sided(b);
                    car_pipe(pass, gpu, &mut pick, if !want { 0 } else if m { 2 } else { 1 }, b.mode);
                }
                if c.is_ambient() && amb_skip(&c.model.name, bi, b) {
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
                        // (seen from behind, the wheel's back in the rim's place - F-57869)
                        let back = !w.backside.is_empty() && self.wheel_back.get(ci).is_some_and(|m| wi < 8 && m & (1 << wi) != 0);
                        let rim_from = if back { w.rim_from.min(w.batches.len()) } else { w.batches.len() };
                        for b in w.batches[..rim_from].iter().chain(if back { &w.backside[..] } else { &w.batches[rim_from..] }) {
                            if own {
                                car_pipe(pass, gpu, &mut pick, 0, b.mode);
                            }
                            pass.set_bind_group(1, b.bind.as_ref(), &[]);
                            pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                            pass.set_vertex_buffer(0, b.vb.slice(..));
                            pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..b.count, 0, 0..1);
                            *draws += 1;
                        }
                        // the brake behind the rim: the rotor with the wheel, the caliper still
                        if let Some(br) = self.player_brake.as_ref().filter(|_| c.is_player).or(c.brake.as_ref()).or(c.model.brakes.as_ref()) {
                            // (a motorcycle, F-57869 with F-54299: the brake drawn once with [caliper, rotor B] when the tune's
                            // bHasFrontLeftRotor / bHasRearLeftRotor is set, once with [caliper, rotor A] for the Right one; the
                            // caliper's place is the same in both, so it is drawn once here)
                            let [disc_a, disc_b] = bike_discs(&c.model, wi);
                            let bike = c.model.bike;
                            for (list, bg, on) in [
                                (&br.rotor, c.rotor_bindings.get(wi).map(|b| &b.bg), !bike || disc_a),
                                (&br.rotor, c.rotor_bindings.get(wi + 1).map(|b| &b.bg), bike && disc_b),
                                (&br.caliper, c.caliper_bindings.get(wi).map(|b| &b.bg), !bike || disc_a || disc_b),
                            ] {
                                let (Some(bg), true) = (bg, on) else { continue };
                                pass.set_bind_group(3, bg, &[]);
                                for b in list {
                                    if own {
                                        car_pipe(pass, gpu, &mut pick, 0, b.mode);
                                    }
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
        if pick.0 != 0 || pick.2 {
            pass.set_pipeline(&gpu.pipeline_opaque);
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
    /// A traffic or parked car (an ambient render instance, a `va_` model that is not the player's).
    pub fn is_ambient(&self) -> bool {
        !self.is_player && self.model.name.to_ascii_lowercase().starts_with("va_")
    }

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
    /// The traffic cars' LOD (F-69447, from F-73443 into the instance's +28, in the main pass
    /// 0x10000 of F-27833): the car's sphere - the bound's centroid through the car's matrix, radius
    /// `CarModel::lod_radius` - tested against the main view (F-91781: outside -> -1) and its occluders
    /// (F-87100 -> F-38575, `occluder::Selected::hides_sphere`: hidden -> -1); the depth is plane 0 of
    /// the viewport's first block - the near plane, column 2 of view x projection normalised (F-53768) -
    /// through the centre, plus the radius: the distance ahead of the camera less the near distance (the boom
    /// file's `CameraNear`), plus the radius. Not drawn from 500 on (0x82003250), LOD 0 below type +44 x type +40,
    /// LOD 1 below type +48 x type +40, else LOD 2 (per type: `CarModel::lod_steps`). The -1
    /// keeps the car out of every pass: the shadow passes 4 and 8 draw through F-89353, which returns on
    /// it, and so does the main view's (`lod_batches`). Cars without LODs (set-up cars) keep 0. `full_detail`
    /// (OPTIONS > PERFORMANCE > HIGH-DETAIL TRAFFIC, the port's own - DECISIONS.md D19): LOD 0 wherever the
    /// original draws the car at all.
    ///
    /// The glow pass (0x8000 of F-27833 -> F-62563 -> F-96521) draws a car's glows when its lights
    /// are on (the instance's per-view byte +557 + view, bit 0x4, read live: all clear at noon, set on the
    /// driven cars at 23:00; here the view test of the sphere, the lamps' levels carrying the flag) and either its LOD is not -1 or instance
    /// +556 bit 0x20 is set: the depth below 50 (0x82006BF4, F-73443). The depth is written whether or
    /// not the sphere is in the view (F-91781 stores it before its answer), so a car hidden by an
    /// occluder or 500 deep keeps its glows within 50 - `lod_glow`.
    pub fn pick_lods(&mut self, eye: Vec3, axis: Vec3, near: f32, k: crate::props::Lod, frustum: &crate::render::Frustum, occluders: &crate::occluder::Selected) {
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
                let lod = Self::lod_of(c, d, seen && !occluders.hides_sphere(centre, r), k);
                // (F-27833, main pass: LOD 1 becomes LOD 2 while car +500 bit 0x4 - on its way out)
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

    /// F-69447's answer for depth `d`: -1 outside the view or hidden (`shown` false) or from 500 on
    /// (0x82003250); LOD 0 below the type's fHighToMedSwitch x fLodSwitchScale (+44 x +40, the scale
    /// is the f0 of A-18097), LOD 1 below fMedToLowSwitch x the scale (+48 x +40), else LOD 2.
    /// OPTIONS > PERFORMANCE (`k`, 1 = the original): DRAW DISTANCE multiplies the 500, DETAIL LEVEL
    /// the scale; DETAIL LEVEL 3X keeps every car at LOD 0 (what HIGH-DETAIL TRAFFIC ON did, D19).
    fn lod_of(c: &CarInstance, d: f32, shown: bool, k: crate::props::Lod) -> i8 {
        Self::lod_by(c.model.lod_steps, d, shown, k)
    }

    pub(crate) fn lod_by(steps: [f32; 2], d: f32, shown: bool, k: crate::props::Lod) -> i8 {
        if !shown || d >= 500.0 * k.draw {
            -1
        } else if k.detail >= 3.0 || d < steps[0] * k.detail {
            0
        } else if d < steps[1] * k.detail {
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
            if traffic_casts_none() && c.is_ambient() {
                continue;
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
            // (a traffic car's blob is sized from its type's bound box, F-68081; MCLA_BLOB_MODEL_BOX=1: the drawn model's box)
            static MODEL_BOX: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            let (min, max) = match c.model.bound_box {
                Some(b) if !*MODEL_BOX.get_or_init(|| std::env::var_os("MCLA_BLOB_MODEL_BOX").is_some()) => b,
                _ => (c.model.min, c.model.max),
            };
            out.push(crate::carblob::CarBlob { m, min, max, ground_y, ambient: c.model.name.to_ascii_lowercase().starts_with("va_"), wheels });
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
                if !b.mode.blended() && b.bucket != 1 {
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
            if c.model.bike || (self.hide_player && c.is_player) || (traffic_casts_none() && c.is_ambient()) {
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
            // (the wheels' blur discs: the car's depth draw F-35555 runs the blur pass F-49519 with its flag 0 after the
            // tyres - the disc alone in the rim's place, no motion-blur record; recorded in both of the car's shadow
            // tiles, four 234-index VS_CarShadowDepthLocal draws each, both sides (0x19800). The wheels as the blur
            // pass takes them, `blur_wheels`. A car under Stealth casts nothing here, as its body. MCLA_WHEEL_BLUR=0: none)
            let disc = c.wheels.as_ref().or(c.model.wheels.as_ref()).or(self.wheels.as_ref()).and_then(|w| w.blur.as_ref());
            if let (Some(disc), Some(blur), false) = (disc, self.blur_wheels.get(ci), c.pu_stealth || wheel_blur_off()) {
                pass.set_bind_group(1, disc.bind.as_ref(), &[]);
                pass.set_bind_group(2, &gpu.mat_bgs[&(disc.mode as u32)], &[]);
                pass.set_vertex_buffer(0, disc.vb.slice(..));
                pass.set_index_buffer(disc.ib.slice(..), wgpu::IndexFormat::Uint32);
                for wb in (0..blur.len()).filter_map(|i| c.wheel_bindings.get(blur_binding(c.model.bike, i))) {
                    pass.set_bind_group(3, &wb.bg, &[]);
                    pass.draw_indexed(0..disc.count, 0, 0..1);
                    *draws += 1;
                }
            }
        }
    }
}

/// The wheel binding of `blur_wheels` entry `i`: a bike's entries are its wheels 0 and 2 (the odd ones are their mirrors).
fn blur_binding(bike: bool, i: usize) -> usize {
    if bike { 2 * i } else { i }
}

/// MCLA_WHEEL_BLUR=0: no blur discs (the blur pass, the shadow passes), as before the wheel blur was ported.
fn wheel_blur_off() -> bool {
    std::env::var("MCLA_WHEEL_BLUR").is_ok_and(|v| v.trim() == "0")
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
    let d = std::fs::read(crate::modcars::setup_file(config).unwrap_or_else(|| root.join("resources/vehicle/configs").join(format!("{config}.xcc")))).ok()?;
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
        out.push(car_upload(gpu, key, &t));
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
    let mut rim_from = usize::MAX;
    let mut tyre_bead = 0.406f32;
    for (src_i, model) in sources.iter().enumerate() {
        // (per shader: the rim's badge, a few triangles on the hub with the rim's own texture, is not a
        // coarser level of the rim)
        let mut seen_by: hashbrown::HashMap<u32, usize> = hashbrown::HashMap::new();
        // Tyre and rim are both unit-diameter models. The rim is scaled to its size; the
        // tyre's bead (modelled at radius 0.406, a 21 inch rim on most cars) is pulled in
        // to meet it, which makes the side wall as tall as the rim leaves room for.
        let is_rim = src_i + 1 == sources.len() && sources.len() > 1;
        if is_rim {
            rim_from = batches.len();
        }
        let xf = if is_rim { Mat4::from_scale(Vec3::new(1.0, rim_ratio, rim_ratio)) } else { Mat4::IDENTITY };
        // (a bike's tyre keeps its bead: F-43091 gives it innerRadiusScale 0, recorded so on the ZX-14)
        let bead = if tyre == "tir_bike" { 0.406 } else { rim_ratio * 0.5 };
        for (mesh, si) in &model.meshes {
            let mut morphed;
            // (the tyre is drawn turned half round about the wheel's vertical against the rim: F-11293 turns the
            // wheel's matrix by pi before the tyre draw (rows 0 and 2 negated, F-25963), so the side wall that
            // carries the lettering - the model's +x, the rim's face being -x - faces out. Baked into the tyre's
            // vertices here, a turn, so the spin and the side's handedness stay. MCLA_TYRE_TURN_OLD: as before)
            let turn = !is_rim && std::env::var("MCLA_TYRE_TURN_OLD").is_err();
            // (the side wall's bulge, VS_ParallaxRubber2 with sidewallBulge = the car's +4048: 0.05 for a car, 0 for
            // a bike, set when the car is made. From the model's own radius r: t = (r - 0.406) / 0.094,
            // s = sat(t - 0.05) x 1.0526, x grows by 1 + b sqrt(s (1 - s)) and the normal's (y, z) gets the radial
            // unit x 10 b (s - 1), then is normalised. Constant for every car, so baked here. MCLA_TYRE_BULGE_OLD: none)
            let bulge = if is_rim || tyre.starts_with("tir_bike") || std::env::var("MCLA_TYRE_BULGE_OLD").is_ok() { 0.0 } else { 0.05f32 };
            let mesh = if !is_rim && ((bead - 0.406).abs() > 0.004 || turn || bulge > 0.0) {
                morphed = mesh.clone();
                tyre_bead = bead;
                if bulge > 0.0 {
                    for vi in 0..morphed.positions.len() {
                        let p = morphed.positions[vi];
                        let r = (p.y * p.y + p.z * p.z).sqrt();
                        let t = (r - 0.406) * 10.526_315;
                        let s = (t - 0.05).clamp(0.0, 1.0) * 1.052_631_6;
                        morphed.positions[vi].x = p.x * (1.0 + bulge * (s - s * s).abs().sqrt());
                        if let (Some(n), true) = (morphed.normals.get_mut(vi), r > 1e-6) {
                            let k = 10.0 * bulge * (s - 1.0) / r;
                            let len = n.length();
                            let bent = Vec3::new(n.x, n.y + k * p.y, n.z + k * p.z);
                            // (the normal's length carries the binormal's sign, bflip: kept)
                            *n = bent.normalize_or_zero() * len;
                        }
                    }
                }
                if turn {
                    for p in morphed.positions.iter_mut().chain(morphed.normals.iter_mut()) {
                        *p = Vec3::new(-p.x, p.y, -p.z);
                    }
                    for t in morphed.tangents.iter_mut() {
                        *t = glam::Vec4::new(-t.x, t.y, -t.z, t.w);
                    }
                }
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
                Some((key, t)) => car_upload(gpu, key, t).0,
                None if k != 0 => car_any(gpu, world, k).unwrap_or_else(|| {
                    crate::world::note_tex_miss("car part", &format!("{k:08x}"));
                    gpu.white.clone()
                }),
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
    // (the wheel's back, with the rim's scale; only for a wheel that has a rim)
    let backside = if rim_from < batches.len() { wheel_backside(gpu, &root, Mat4::from_scale(Vec3::new(1.0, rim_ratio, rim_ratio))) } else { Vec::new() };
    let rim_from = rim_from.min(batches.len());
    let blur = if rim_from < batches.len() { wheel_blur_disc(gpu, &root, Mat4::from_scale(Vec3::new(1.0, rim_ratio, rim_ratio))) } else { None };
    Some(WheelModel { batches, rim_from, backside, blur, tyre_bead })
}

/// `blur_wheel` of vehicle/shared_utility (F-31291): one material (TestRimBlur), the text mesh `blur_wheel.mesh` - 80
/// vertices, 234 indices, a disc in the plane x = -0.25 (the rim's outer face), radius 0.5. Only its places count: the
/// original draws it with colour writes off (recorded: register 0x2104 = 0). None when the file is missing.
fn wheel_blur_disc(gpu: &mut Gpu, root: &Path, xf: Mat4) -> Option<GpuBatch> {
    let dir = ["shared_utility", "0xd8bee6cc"].iter().map(|top| root.join("vehicle").join(top).join("blur_wheel")).find(|d| d.is_dir())?;
    let mesh = std::fs::read(dir.join("blur_wheel.mesh")).or_else(|_| std::fs::read(dir.join(format!("0x{:08x}", mcla_formats::hash::joaat("blur_wheel.mesh"))))).ok()?;
    let text = String::from_utf8_lossy(&mesh);
    let rest = &text[text.find("Idx ")? + 4..];
    let (o, e) = (rest.find('{')?, rest.find('}')?);
    let idx: Vec<u32> = rest[o + 1..e].split_whitespace().filter_map(|w| w.parse().ok()).collect();
    let rest = &rest[rest.find("Verts ")? + 6..];
    let n = rest.split_whitespace().next()?.parse::<usize>().ok()?;
    let mut m = mcla_formats::rage::geometry::Mesh { indices: idx, ..Default::default() };
    for line in rest.lines().skip(1).take(n) {
        // (position / weights / normal / ...)
        let p: Vec<f32> = line.split('/').next().unwrap_or("").split_whitespace().filter_map(|w| w.parse().ok()).collect();
        if p.len() < 3 {
            break;
        }
        m.positions.push(Vec3::new(p[0], p[1], p[2]));
        m.normals.push(Vec3::X);
        m.uv0.push(glam::Vec2::ZERO);
    }
    if m.positions.len() != n || m.indices.is_empty() || m.indices.iter().any(|&i| i as usize >= n) {
        log::warn!("blur_wheel: mesh unreadable");
        return None;
    }
    let white = gpu.white.clone();
    Some(mesh_to_batch_xf(gpu, &m, "BlackMatte", car_mode("BlackMatte"), white.clone(), white, 0, 0, xf))
}

/// `whl_backside` of vehicle/shared_utility (F-31291): its entity's two shaders in order (RimMain, BlackMatte)
/// and its text mesh `whl_backside_geo.mesh`, one material each - the barrel's inside, x -0.1723 .. 0.25 at
/// radius 0.461 .. 0.5 (zone 2), and the black disc at x -0.1723 (radius 0.4614). As the rim's own vertices: the
/// second UV set is the zone, the first colour's red the occlusion; v as the compiled resources keep it (1 - v,
/// the recorded vertex buffer). Empty when the files are missing.
fn wheel_backside(gpu: &mut Gpu, root: &Path, xf: Mat4) -> Vec<GpuBatch> {
    let dir = ["shared_utility", "0xd8bee6cc"].iter().map(|top| root.join("vehicle").join(top).join("whl_backside")).find(|d| d.is_dir());
    let Some(dir) = dir else { return Vec::new() };
    let mesh = std::fs::read(dir.join("whl_backside_geo.mesh")).or_else(|_| std::fs::read(dir.join(format!("0x{:08x}", mcla_formats::hash::joaat("whl_backside_geo.mesh")))));
    let (Ok(mesh), Ok(ty)) = (mesh, std::fs::read_to_string(dir.join("entity.type"))) else { return Vec::new() };
    let shaders: Vec<String> = ty.lines().filter_map(|l| l.trim().strip_prefix("Template ")).map(|s| s.trim().trim_matches('"').to_string()).collect();
    let text = String::from_utf8_lossy(&mesh);
    let mut out = Vec::new();
    let mut rest = &text[..];
    for sname in &shaders {
        // (each material: its indices, then its vertices)
        let Some(at) = rest.find("Idx ") else { break };
        rest = &rest[at + 4..];
        let (Some(o), Some(e)) = (rest.find('{'), rest.find('}')) else { break };
        let idx: Vec<u32> = rest[o + 1..e].split_whitespace().filter_map(|w| w.parse().ok()).collect();
        let Some(at) = rest.find("Verts ") else { break };
        rest = &rest[at + 6..];
        let Some(n) = rest.split_whitespace().next().and_then(|w| w.parse::<usize>().ok()) else { break };
        let mut m = mcla_formats::rage::geometry::Mesh { indices: idx, ..Default::default() };
        let f = |s: &str| -> Vec<f32> { s.split_whitespace().filter_map(|w| w.parse().ok()).collect() };
        for line in rest.lines().skip(1).take(n) {
            // (position / weights / normal / three colours / UV sets: count, u, v ... / tangent)
            let p: Vec<&str> = line.split('/').collect();
            if p.len() < 7 {
                break;
            }
            let (pos, nrm, col, uv) = (f(p[0]), f(p[2]), f(p[3]), f(p[6]));
            if pos.len() < 3 || nrm.len() < 3 || col.len() < 4 || uv.len() < 5 {
                break;
            }
            m.positions.push(Vec3::new(pos[0], pos[1], pos[2]));
            m.normals.push(Vec3::new(nrm[0], nrm[1], nrm[2]));
            m.colors.push([0, 1, 2, 3].map(|k| (col[k].clamp(0.0, 1.0) * 255.0).round() as u8));
            m.uv0.push(glam::Vec2::new(uv[1], 1.0 - uv[2]));
            m.uv1.push(glam::Vec2::new(uv[3], 1.0 - uv[4]));
        }
        if m.positions.len() != n || m.indices.iter().any(|&i| i as usize >= n) {
            log::warn!("whl_backside: material {sname} unreadable");
            return Vec::new();
        }
        let rim = sname.eq_ignore_ascii_case("RimMain");
        let white = gpu.white.clone();
        let mut b = mesh_to_batch_xf(gpu, &m, sname, if rim { MatMode::Rim } else { car_mode(sname) }, white.clone(), white, 0, 0, xf);
        if rim {
            // (as the rim's: the zone in uv1, the occlusion in the colour's alpha; uv2.y: its colours are the rim's x 0.125 -
            // F-94854 hands the entity the rim's hookups and its colours x 0.125, city.wgsl's PS_Multi branch)
            let verts: Vec<Vertex> = (0..n)
                .map(|i| {
                    let nr = xf.transform_vector3(m.normals[i]).normalize_or(Vec3::Y);
                    Vertex { pos: xf.transform_point3(m.positions[i]).to_array(), normal: nr.to_array(), uv0: m.uv0[i].to_array(), uv1: m.uv1[i].to_array(), color: [255, 255, 255, m.colors[i][0]], color1: [0, 0, 0, 255], uv2: [0.0, 0.125] }
                })
                .collect();
            b.vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("whl_backside vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
        } else if sname.to_ascii_lowercase().contains("blackmatte") && std::env::var("MCLA_BLACKMATTE_OLD").is_err() {
            // (the disc: PS_BlackMatte, black - recorded draws 2112 ... 2118 of the original, uv2.x = 4 as the car's BlackMatte parts)
            let verts: Vec<Vertex> = (0..n)
                .map(|i| {
                    let nr = xf.transform_vector3(m.normals[i]).normalize_or(Vec3::Y);
                    Vertex { pos: xf.transform_point3(m.positions[i]).to_array(), normal: nr.to_array(), uv0: m.uv0[i].to_array(), uv1: m.uv1[i].to_array(), color: [255, 255, 255, 255], color1: [0, 0, 0, 255], uv2: [4.0, 0.0] }
                })
                .collect();
            b.vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("whl_backside disc vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
        }
        out.push(b);
    }
    out
}

/// A plate background as F-34887 loads it (mcCarLicensePlateBGResource, the background folder's file whose object
/// has a texture behind +8): the texture (RGBA), the letters' tint (+16), place (+32, +36) and size (+40, +44).
#[derive(Clone)]
pub struct PlateBg {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
    pub tint: [f32; 3],
    pub pos: [f32; 2],
    pub div: [f32; 2],
}

/// The background `name` of `resources/license_plates/backgrounds` (none when its record is missing).
pub fn load_plate_bg(root: &std::path::Path, name: &str) -> Option<PlateBg> {
    let dir = root.join("resources/license_plates/backgrounds").join(name);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "r1")).collect();
    files.sort();
    files.into_iter().find_map(|f| {
        let res = crate::assets::load_resource_file(&f).ok()?;
        let rd = mcla_formats::rage::Rd::new(&res);
        // (the folder's other file is a texture alone; the plate draw uses the record's own)
        let t = rd.ptr(8).and_then(|o| mcla_formats::rage::texture::Texture::parse(&rd, o).ok())?;
        Some(PlateBg {
            w: t.width as usize,
            h: t.height as usize,
            px: t.to_rgba8(),
            tint: [rd.f32(16), rd.f32(20), rd.f32(24)],
            pos: [rd.f32(32), rd.f32(36)],
            div: [rd.f32(40), rd.f32(44)],
        })
    })
}

/// The plate's texture with its seven letters, as PS_Plate draws them over the background on each texel: F-64389
/// sets licensePlateFontPos = (pos.x, -pos.y), licensePlateFontScale = (1 / div.x, 1 / div.y), the tint, and
/// letterIdxs = the cells of the seven characters (F-79104: 0-9, A-Z, `[\]^` and `:;<=` 36..39, anything else
/// cell 63, empty).
/// With U = 7.4 sx sat(u - fx - 0.03) and V = 2 sy sat(v - fy - 0.37), column trunc(U) (0..6) and row trunc(V)
/// (0) hold a letter, sampled at its cell + letterSize (1/8) x the fractions in the 8-cell atlas; the colour is
/// lerp(background, tint, font alpha) where u, v are past the letters' corner. The program also multiplies by the
/// font's colour, which is white in the whole atlas. A divisor of 0 (the Rockstar plates) puts every column past
/// 6: no letters.
pub fn compose_plate(bg: &PlateBg, font: &[u8], text: &str) -> Vec<u8> {
    let mut px = bg.px.clone();
    if font.len() < 256 * 256 * 4 {
        return px;
    }
    let mut cells = [63usize; 7];
    for (k, c) in text.chars().take(7).enumerate() {
        cells[k] = match c.to_ascii_uppercase() {
            c @ '0'..='9' => c as usize - '0' as usize,
            c @ 'A'..='Z' => 10 + c as usize - 'A' as usize,
            '[' | ':' => 36,
            '\\' | ';' => 37,
            ']' | '<' => 38,
            '^' | '=' => 39,
            _ => 63,
        };
    }
    let (fx, fy) = (bg.pos[0] + 0.03, -bg.pos[1] + 0.37);
    let (sx, sy) = (1.0 / bg.div[0], 1.0 / bg.div[1]);
    // (the atlas sampled bilinearly, clamped at its edges. What depends on the column alone - the cell, the atlas
    // column, its weight - and on the row alone is worked out once, the texels' alpha / 255 read from a table: the
    // same operations in the same order as per pixel, so the same bytes - 1.4 ms a traffic car's plate before)
    let fa: Vec<f32> = (0..256 * 256).map(|i| font[i * 4 + 3] as f32 / 255.0).collect();
    let idx = |c: f32| c.clamp(0.0, 255.0) as usize;
    // per column: the cell's row in the atlas, the two texel columns and the weight
    let cols: Vec<Option<(usize, usize, usize, f32)>> = (0..bg.w)
        .map(|x| {
            let u = (x as f32 + 0.5) / bg.w as f32;
            if u - fx < 0.0 {
                return None;
            }
            let ru = 7.4 * sx * (u - fx).min(1.0);
            if !(ru.trunc() <= 6.0) {
                return None;
            }
            let cell = cells[ru.trunc() as usize];
            let ax = ((cell % 8) as f32 / 8.0 + 0.125 * ru.fract()) * 256.0 - 0.5;
            let x0 = ax.floor();
            Some((cell / 8, idx(x0), idx(x0 + 1.0), ax - x0))
        })
        .collect();
    let btab: Vec<f32> = (0..256).map(|b| b as f32 / 255.0).collect();
    for y in 0..bg.h {
        let v = (y as f32 + 0.5) / bg.h as f32;
        if v - fy < 0.0 {
            continue;
        }
        let rv = 2.0 * sy * (v - fy).min(1.0);
        if !(rv.trunc() <= 0.0) {
            continue;
        }
        // per atlas row of a cell: the two texel rows and the weight
        let rows: [(usize, usize, f32); 8] = std::array::from_fn(|cr| {
            let ay = (cr as f32 / 8.0 + 0.125 * rv.fract()) * 256.0 - 0.5;
            let y0 = ay.floor();
            (idx(y0) * 256, idx(y0 + 1.0) * 256, ay - y0)
        });
        for (x, col) in cols.iter().enumerate() {
            let Some((cr, x0, x1, tx)) = *col else { continue };
            let (y0, y1, ty) = rows[cr];
            let top = fa[y0 + x0] * (1.0 - tx) + fa[y0 + x1] * tx;
            let bot = fa[y1 + x0] * (1.0 - tx) + fa[y1 + x1] * tx;
            let a = top * (1.0 - ty) + bot * ty;
            if a <= 0.0 {
                continue;
            }
            let o = (y * bg.w + x) * 4;
            for j in 0..3 {
                let b = btab[px[o + j] as usize];
                px[o + j] = ((b + a * (bg.tint[j] - b)) * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    px
}

#[cfg(test)]
mod traffic_light_set_tests {
    use super::*;

    /// A traffic car's light set (the traffic object's reset F-14696 sets the set's +0 to 0 and +4 to its
    /// drawable, so F-70785 takes the second source F-74049): per kind the drawable's bones whose name is exactly
    /// `<kind>0` (F-90899: skeleton order, a byte compare), at most the table's 8, the side +24 never written (0).
    /// Every `va_` model on the disc: `prep_car`'s glow list equals that.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn traffic_glows_are_the_second_source() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let assets = crate::assets::Assets::new(&dir);
        let root = std::path::Path::new(&dir).join("resources/vehicle");
        let mut cars: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path())
            .filter(|p| p.is_dir() && p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("va_"))).collect();
        cars.sort();
        let (mut n, mut lamps, mut bad) = (0, 0, Vec::new());
        for p in cars {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let Ok(car) = vehicle_model::load_car(&assets, &p) else { continue };
            let Some(sk) = car.skeleton.as_ref() else { continue };
            let mut want: Vec<(u8, Vec3)> = Vec::new();
            for b in &sk.bones {
                // (the exact name: a bone spelt other than in lower case is not found)
                let Some(k) = glow_bone_kind(&b.name).filter(|_| b.name == b.name.to_ascii_lowercase()) else { continue };
                if want.iter().filter(|w| w.0 == k).count() < LIGHT_SET_KIND_CAP {
                    want.push((k, b.world.w_axis.truncate()));
                }
            }
            let prep = Vehicles::prep_car(&car, &p, &CarConfig::default());
            let got: Vec<(u8, Vec3)> = prep.glow_bones.iter().map(|g| (g.0, g.1)).collect();
            let same = got.len() == want.len() && got.iter().zip(&want).all(|(a, b)| a.0 == b.0 && a.1.distance(b.1) < 1e-4);
            if !same {
                bad.push(format!("{name}: port {:?} want {:?}", got.iter().map(|g| g.0).collect::<Vec<_>>(), want.iter().map(|g| g.0).collect::<Vec<_>>()));
            }
            if prep.glow_sides.iter().any(|&s| s != 0) {
                bad.push(format!("{name}: a side lamp {:?}", prep.glow_sides));
            }
            n += 1;
            lamps += want.len();
        }
        println!("{n} traffic models, {lamps} lamps; differ: {bad:?}");
        assert!(n > 30, "{n}");
        // (open: the garbage truck's LOD0 fragment has no skeleton; its lamp bones are in the LOD1 / LOD2 fragments'
        // 30-bone skeleton. Which skeleton the original hands F-74049 - the first word of entry 0 of the traffic type's
        // list +40, F-58302 - is not decoded, so the port's empty list is not settled either way)
        assert!(bad.len() == 1 && bad[0].starts_with("va_rstr_garbagetruck_07: port []"), "{}", bad.join("\n"));
    }
}

#[cfg(test)]
mod plate_tests {
    use super::*;

    fn bg(pos: [f32; 2], div: [f32; 2]) -> PlateBg {
        PlateBg { w: 64, h: 32, px: vec![255; 64 * 32 * 4], tint: [0.6706, 0.1216, 0.1216], pos, div }
    }

    /// A full atlas: every cell opaque but the empty one (63).
    fn font() -> Vec<u8> {
        let mut f = vec![255u8; 256 * 256 * 4];
        for y in 224..256 {
            for x in 224..256 {
                f[(y * 256 + x) * 4 + 3] = 0;
            }
        }
        f
    }

    /// plate_al_00's record: the tint inside the letters' box (columns from u = 0.045, the row from
    /// v = 0.285 for 1 / (2 x 1/0.97)), the background outside it, and none on a space.
    #[test]
    fn letters_take_the_record_tint_and_box() {
        let b = bg([0.015, 0.085], [0.97, 0.97]);
        let px = compose_plate(&b, &font(), "AAAAAAA");
        let at = |x: usize, y: usize| &px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 3];
        assert_eq!(at(10, 16), &[171, 31, 31]);
        assert_eq!(at(1, 16), &[255, 255, 255]);
        assert_eq!(at(10, 4), &[255, 255, 255]);
        assert_eq!(at(10, 29), &[255, 255, 255]);
        let sp = compose_plate(&b, &font(), " AAAAAA");
        assert_eq!(&sp[(16 * 64 + 4) * 4..(16 * 64 + 4) * 4 + 3], &[255, 255, 255]);
    }

    /// Time of one plate on the disc's own art (plate_ca_00, fnt_license_plate); `MCLA_DATA_DIR` or extracted/cache.
    #[test]
    #[ignore = "timing; needs the extracted disc"]
    fn plate_time_on_the_disc_art() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string()));
        let b = load_plate_bg(&root, "plate_ca_00").expect("plate_ca_00");
        let d = std::fs::read(root.join("vehicle/shared_font/fnt_license_plate.dds")).unwrap();
        let f = mcla_formats::dds::parse_dds("fnt_license_plate", &d).unwrap().to_rgba8();
        let t = std::time::Instant::now();
        let mut sum = 0u64;
        for k in 0..50 {
            let px = compose_plate(&b, &f, &format!("{}ABC{:03}", k % 8 + 1, k));
            sum += px[(100 * b.w + 200) * 4] as u64;
            assert!(px == compose_plate_whole(&b, &f, &format!("{}ABC{:03}", k % 8 + 1, k)));
        }
        eprintln!("plate {}x{} pos {:?} div {:?}: {:.3} ms a plate (incl. the reference) ({sum})", b.w, b.h, b.pos, b.div, t.elapsed().as_secs_f64() * 1e3 / 50.0);
        let t = std::time::Instant::now();
        for k in 0..50 {
            sum += compose_plate(&b, &f, &format!("{}ABC{:03}", k % 8 + 1, k))[0] as u64;
        }
        eprintln!("compose_plate alone: {:.3} ms a plate ({sum})", t.elapsed().as_secs_f64() * 1e3 / 50.0);
    }

    /// The loop over the letters' box only gives the same pixels as the loop over the whole plate.
    #[test]
    fn letter_box_loop_is_the_whole_loop() {
        let mut r = 0x9E37_79B9u32;
        let mut next = move || {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            r
        };
        let mut f = font();
        for k in 0..256 * 256 {
            f[k * 4 + 3] = (next() % 256) as u8;
        }
        let picks = [0.0f32, -1.0, 1.0, 0.015, 0.085, 0.3, 0.97, 1.2, -0.4, 2.5];
        for case in 0..400 {
            let mut b = bg([picks[(next() % 10) as usize], picks[(next() % 10) as usize]], [picks[(next() % 10) as usize], picks[(next() % 10) as usize]]);
            (b.w, b.h) = ([64, 512, 37][case % 3], [32, 256, 19][case % 3]);
            b.px = (0..b.w * b.h * 4).map(|_| (next() % 256) as u8).collect();
            let text: String = (0..7).map(|_| b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ [\\]^:;<=*"[(next() % 46) as usize] as char).collect();
            assert!(compose_plate(&b, &f, &text) == compose_plate_whole(&b, &f, &text), "case {case}: {:?} {:?} {text}", b.pos, b.div);
        }
    }

    fn compose_plate_whole(bg: &PlateBg, font: &[u8], text: &str) -> Vec<u8> {
        let mut px = bg.px.clone();
        if font.len() < 256 * 256 * 4 {
            return px;
        }
        let mut cells = [63usize; 7];
        for (k, c) in text.chars().take(7).enumerate() {
            cells[k] = match c.to_ascii_uppercase() {
                c @ '0'..='9' => c as usize - '0' as usize,
                c @ 'A'..='Z' => 10 + c as usize - 'A' as usize,
                '[' | ':' => 36,
                '\\' | ';' => 37,
                ']' | '<' => 38,
                '^' | '=' => 39,
                _ => 63,
            };
        }
        let (fx, fy) = (bg.pos[0] + 0.03, -bg.pos[1] + 0.37);
        let (sx, sy) = (1.0 / bg.div[0], 1.0 / bg.div[1]);
        // (the atlas sampled bilinearly, clamped at its edges)
        let alpha = |u: f32, v: f32| -> f32 {
            let (x, y) = (u * 256.0 - 0.5, v * 256.0 - 0.5);
            let (x0, y0) = (x.floor(), y.floor());
            let (tx, ty) = (x - x0, y - y0);
            let at = |x: f32, y: f32| font[((y.clamp(0.0, 255.0) as usize) * 256 + x.clamp(0.0, 255.0) as usize) * 4 + 3] as f32 / 255.0;
            let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1.0, y0) * tx;
            let bot = at(x0, y0 + 1.0) * (1.0 - tx) + at(x0 + 1.0, y0 + 1.0) * tx;
            top * (1.0 - ty) + bot * ty
        };
        for y in 0..bg.h {
            let v = (y as f32 + 0.5) / bg.h as f32;
            if v - fy < 0.0 {
                continue;
            }
            let rv = 2.0 * sy * (v - fy).min(1.0);
            for x in 0..bg.w {
                let u = (x as f32 + 0.5) / bg.w as f32;
                if u - fx < 0.0 {
                    continue;
                }
                let ru = 7.4 * sx * (u - fx).min(1.0);
                if !(ru.trunc() <= 6.0 && rv.trunc() <= 0.0) {
                    continue;
                }
                let cell = cells[ru.trunc() as usize];
                let a = alpha((cell % 8) as f32 / 8.0 + 0.125 * ru.fract(), (cell / 8) as f32 / 8.0 + 0.125 * rv.fract());
                if a <= 0.0 {
                    continue;
                }
                let o = (y * bg.w + x) * 4;
                for j in 0..3 {
                    let b = px[o + j] as f32 / 255.0;
                    px[o + j] = ((b + a * (bg.tint[j] - b)) * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        px
    }

    /// The Rockstar plates (pos -1, 1, divisors 0): no letters at all.
    #[test]
    fn rockstar_plates_have_no_letters() {
        let b = bg([-1.0, 1.0], [0.0, 0.0]);
        assert_eq!(compose_plate(&b, &font(), "RG45ABC"), b.px);
    }
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
        // (HEADLIGHTS held by day: the flash lights attachment 4 and sat(head + flash) 3; F-96984)
        let f = lv([0.0, 0.0, 0.0, 1.0, 0.0]);
        let want = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(f, want.to_vec());
        assert_eq!(lv([1.0, 0.0, 0.0, 0.5, 0.0])[3], 1.0);
    }
}

#[cfg(test)]
mod lod_option_tests {
    use crate::props::Lod;
    /// F-69447 (ORIGINAL): 500 from 0x82003250, steps 40 / 155 of an ordinary type; the PERFORMANCE scales on top.
    #[test]
    fn traffic_lod_by_the_options() {
        let by = super::Vehicles::lod_by;
        let s = [40.0, 155.0];
        let o = Lod::ORIGINAL;
        assert_eq!([by(s, 39.9, true, o), by(s, 40.0, true, o), by(s, 154.9, true, o), by(s, 155.0, true, o), by(s, 499.9, true, o), by(s, 500.0, true, o), by(s, 10.0, false, o)], [0, 1, 1, 2, 2, -1, -1]);
        let k = Lod { draw: 2.0, detail: 1.5, ..Lod::ORIGINAL };
        assert_eq!([by(s, 59.9, true, k), by(s, 60.0, true, k), by(s, 232.4, true, k), by(s, 232.5, true, k), by(s, 999.0, true, k), by(s, 1000.0, true, k)], [0, 1, 1, 2, 2, -1]);
        // DETAIL LEVEL 3X = the old HIGH-DETAIL TRAFFIC ON: LOD 0 wherever the car is drawn
        let k = Lod { draw: 1.0, detail: 3.0, ..Lod::ORIGINAL };
        assert_eq!([by(s, 300.0, true, k), by(s, 499.0, true, k), by(s, 500.0, true, k)], [0, 0, -1]);
    }
}

#[cfg(test)]
mod hit_tests {
    use super::*;

    #[test]
    fn aggressor_is_the_one_driving_at_the_other() {
        let d = Vec3::new(10.0, 0.0, 0.0);
        // the partner stands: this is the aggressor; this stands: the partner is
        assert!(first_is_aggressor(Vec3::new(20.0, 0.0, 0.0), Vec3::ZERO, d));
        assert!(!first_is_aggressor(Vec3::ZERO, Vec3::new(-20.0, 0.0, 0.0), d));
        // both move: the one whose heading points more at the other (the speeds divide out)
        assert!(first_is_aggressor(Vec3::new(30.0, 0.0, 0.0), Vec3::new(-5.0, 0.0, 5.0), d));
        assert!(!first_is_aggressor(Vec3::new(5.0, 0.0, 5.0), Vec3::new(-30.0, 0.0, 0.0), d));
    }
}

impl Vehicles {
    /// The traffic cars' head-lamp cones this frame (F-68081, second half; the same strength as the blob's,
    /// run also for a car whose LOD is -1 - only the blob needs a LOD). Lights on; a sphere of radius 3, 6 m ahead
    /// and 2 m up, in the view; the one 6 m ahead not behind an occluder; a ray 5 m down hits the world. The
    /// record: rows x = n x z, y = n, z = x x n scaled 15 x 0.33, 0.25, 15 x 0.75, place = hit + 0.1 n - 6 z; the cone
    /// at car + 1.89 y - 2.23 z along -(z c - y s) for the pitch -0.823 (F-52786), squared range 225,
    /// colour (1, 0.95, 0.75) x strength.
    pub fn pick_cones(&mut self, eye: Vec3, frustum: &crate::render::Frustum, occluders: &crate::occluder::Selected, world: &crate::physics::collision::WorldCollider) {
        let alpha = self.render_alpha;
        let mut cones = std::mem::take(&mut self.cones);
        cones.clear();
        // (test hooks: MCLA_CONE_FORCE=<file> of lines of 28 numbers: these records and no others, for a recording's
        // draws; MCLA_CONE_ALL=1: lit whatever the flag, for a placed car)
        if let Ok(p) = std::env::var("MCLA_CONE_FORCE") {
            for l in std::fs::read_to_string(p).unwrap_or_default().lines() {
                let f: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if f.len() == 28 {
                    let mut r = [[0.0; 4]; 7];
                    for (i, x) in f.iter().enumerate() {
                        r[i / 4][i % 4] = *x;
                    }
                    cones.push(r);
                }
            }
            self.cones = cones;
            return;
        }
        let all = std::env::var_os("MCLA_CONE_ALL").is_some();
        for c in &self.cars {
            if !c.is_ambient() || !(c.amb_lamps.head_on || all) {
                continue;
            }
            let pose = c.render_pose(alpha);
            // (F-27833: the distance from the viewport's eye to F-41581's point - the car's matrix times its bound's
            // centroid, the LOD sphere's centre - not to the car's origin)
            let strength = ((60.0 - (pose.pos + pose.rot * c.model.lod_centre).distance(eye)) / 30.0).clamp(0.0, 1.0);
            if strength <= 0.001 {
                continue;
            }
            let (y, z) = (pose.rot * Vec3::Y, pose.rot * Vec3::Z);
            let ahead = pose.pos - z * 6.0;
            let probe = ahead + y * 2.0;
            if !frustum.intersects_sphere(probe, 3.0) || occluders.hides_sphere(ahead, 3.0) {
                continue;
            }
            let Some(hit) = world.probe_front(probe, Vec3::NEG_Y, 5.0) else { continue };
            let n = hit.normal;
            let rx = n.cross(z).normalize_or(Vec3::X);
            let rz = rx.cross(n).normalize_or(Vec3::Z);
            let place = hit.point + n * 0.1;
            let (s, co) = (-0.823f32).sin_cos();
            let dir = -(z * co - y * s);
            let from = pose.pos + y * 1.89 - z * 2.23;
            let col = Vec3::new(1.0, 0.95, 0.75) * strength;
            if std::env::var_os("MCLA_CONE_DUMP").is_some() {
                eprintln!("cone {} car {:?} place {:?} n {:?} from {:?} dir {:?} col {:?}", c.model.name, pose.pos, place, n, from, dir, col);
            }
            cones.push([(rx * 4.95).extend(0.0).to_array(), (n * 0.25).extend(0.0).to_array(), (rz * 11.25).extend(0.0).to_array(), place.extend(1.0).to_array(), from.extend(225.0).to_array(), dir.extend(0.0).to_array(), col.extend(0.0).to_array()]);
        }
        self.cones = cones;
    }

    /// A new car takes its slots from the pool's instance (see `AmbPool`). Once a frame before the cars' render updates
    /// (the drawn steer, traffic.rs) and again before the slots run.
    pub fn amb_pool_step(&mut self) {
        let now: Vec<(u32, usize)> = self
            .cars
            .iter()
            .filter(|c| c.is_ambient())
            .map(|c| (c.uid, self.amb_types.get(&c.model.name).map_or(0, |t| t.proxy as usize)))
            .collect();
        for (uid, old) in self.amb_pool.step(&now) {
            if let Some(c) = self.cars.iter_mut().find(|c| c.uid == uid) {
                // (the activate hook F-25927 -> the draw object's F-46127: the update count, the drawn steer's ring count
                // and next and the wheel angles go to 0)
                c.amb_frames = 0;
                c.amb_steer = traffic::SteerRing::default();
                if let Some((ids, light)) = old {
                    c.amb_slot = ids;
                    c.amb_light = AmbSlotLights { seen: false, drawn_before: false, ..light };
                }
            }
        }
    }

    /// What the instances hold after this frame's slots.
    pub fn amb_pool_note(&mut self) {
        for l in self.amb_pool.live.iter_mut() {
            if let Some(c) = self.cars.iter().find(|c| c.uid == l.0) {
                l.2 = c.amb_slot;
                l.3 = c.amb_light;
            }
        }
    }
}

/// A new car's light slot ids: -1, -1 (F-19499). Test hook MCLA_AMB_SLOT_START=a,b: other ids, to stand for
/// the history an original's pooled render instance brings (e.g. -1,65545: the hour's light second, as a
/// car last chosen at night keeps it by day).
fn amb_slot_start() -> [i32; 2] {
    static S: std::sync::OnceLock<[i32; 2]> = std::sync::OnceLock::new();
    *S.get_or_init(|| {
        std::env::var("MCLA_AMB_SLOT_START")
            .ok()
            .and_then(|v| {
                let mut it = v.split(',').map(|x| x.trim().parse::<i32>().ok());
                Some([it.next()??, it.next()??])
            })
            .unwrap_or([-1, -1])
    })
}

/// Test hook MCLA_AMB_SKIP=<batch>,<batch>,...: those batches of every traffic car's drawn LOD left out (the recorder's
/// "skip" lines, to find which mesh paints a pixel); MCLA_AMB_BATCHES=1 lists each traffic model's batches once.
fn amb_skip(model: &str, bi: usize, b: &GpuBatch) -> bool {
    static LIST: std::sync::OnceLock<(Vec<usize>, bool)> = std::sync::OnceLock::new();
    static SEEN: std::sync::Mutex<Vec<(String, usize)>> = std::sync::Mutex::new(Vec::new());
    let (skip, list) = LIST.get_or_init(|| {
        (std::env::var("MCLA_AMB_SKIP").map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default(), std::env::var_os("MCLA_AMB_BATCHES").is_some())
    });
    if *list {
        let mut seen = SEEN.lock().unwrap();
        if !seen.iter().any(|(m, i)| m == model && *i == bi) {
            seen.push((model.to_string(), bi));
            eprintln!("ambbatch {model} {bi}: shader {} mode {:?} bucket {} indices {} box {:?} {:?}", b.shader, b.mode, b.bucket, b.count, b.min, b.max);
        }
    }
    skip.contains(&bi)
}

#[cfg(test)]
mod amb_pool_tests {
    use super::*;

    /// The new cars that inherit a used instance, with its ids.
    fn inherited(p: &mut AmbPool, now: &[(u32, usize)]) -> Vec<(u32, [i32; 2])> {
        p.step(now).into_iter().filter_map(|(u, o)| o.map(|o| (u, o.0))).collect()
    }

    /// First in, first out: a car given back is taken again only after every other car of its pool.
    #[test]
    fn fifo_carries_after_the_whole_pool() {
        let mut p = AmbPool::default();
        assert!(inherited(&mut p, &[(1, 0)]).is_empty());
        p.live[0].2 = [7, 65545];
        assert!(inherited(&mut p, &[]).is_empty());
        for last in 2..=98 {
            let out: Vec<_> = (2..=last).map(|u| (u, 0)).collect();
            assert!(inherited(&mut p, &out).is_empty(), "car {last} did not get a fresh instance");
        }
        // the 98th take empties the fresh ones; the next gets the first car's
        let got = inherited(&mut p, &(2..=99).map(|u| (u, 0)).collect::<Vec<_>>());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], (99, [7, 65545]));
        // the other pool is its own
        let mut out: Vec<_> = (2..=99).map(|u| (u, 0)).collect();
        out.push((200, 1));
        assert!(inherited(&mut p, &out).is_empty());
    }
}

/// The siren light's colour and strength (F-72231), by the wall clock in ms (the 32-bit tick): t = ms as a float x 0.01
/// (a float product); red when fmod(t, (0.5 + 0.5) x 3 x 2) > (0.5 + 0.5) x 3, else blue; strength 0 when
/// fmod(t, 0.5 + 0.5) > 0.5, else 200 (the manager ctor's values; fmod in double, its answer made a float). Checked on
/// 4,661 clocks against the original's code run offline (`tools/siren_flash_reference.py`, test `siren_flash_matches_the_original`).
pub fn siren_flash(ms: u64) -> ([f32; 3], f32) {
    let t = (ms as u32) as f32 * 0.01;
    let (on, cycle) = (0.5f32 + 0.5, (0.5f32 + 0.5) * 3.0);
    let red = ((t as f64) % ((cycle * 2.0) as f64)) as f32 > cycle;
    let off = ((t as f64) % (on as f64)) as f32 > 0.5;
    (if red { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] }, if off { 0.0 } else { 200.0 })
}

#[cfg(test)]
mod siren_tests {
    use super::siren_flash;

    #[test]
    fn siren_flash_matches_the_original() {
        let t = include_str!("testdata/siren_flash_reference.txt");
        let mut n = 0;
        for l in t.lines() {
            let (a, b) = l.split_once(" -> ").unwrap();
            let ms: u64 = a.trim_start_matches("CASE ").parse().unwrap();
            let w: Vec<u32> = b.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let (c, i) = siren_flash(ms);
            assert_eq!([i.to_bits(), c[0].to_bits(), c[1].to_bits(), c[2].to_bits()], [w[0], w[1], w[2], w[3]], "{l}");
            n += 1;
        }
        assert_eq!(n, 4661);
    }

    #[test]
    fn the_flash_by_the_clock() {
        assert_eq!(siren_flash(0), ([0.0, 0.0, 1.0], 200.0));
        assert_eq!(siren_flash(49), ([0.0, 0.0, 1.0], 200.0));
        assert_eq!(siren_flash(51), ([0.0, 0.0, 1.0], 0.0));
        assert_eq!(siren_flash(299), ([0.0, 0.0, 1.0], 0.0));
        assert_eq!(siren_flash(301), ([1.0, 0.0, 0.0], 200.0));
        assert_eq!(siren_flash(350), ([1.0, 0.0, 0.0], 200.0));
        assert_eq!(siren_flash(599), ([1.0, 0.0, 0.0], 0.0));
    }
}

/// A batch of `draw_x`'s opaque pass: `cull` 0 none, 1 the set-up car's, 2 the same mirrored; its material's own pipeline
/// once it is built (`Gpu::pipeline_opaque_mode` / `pipeline_opaque_cull_mode`; none for 2), else the pass's as before.
/// `pick` = (the cull, the material, its own pipeline set) of the pipeline set last.
fn car_pipe(pass: &mut wgpu::RenderPass<'_>, gpu: &Gpu, pick: &mut (u8, u32, bool), cull: u8, mode: MatMode) {
    if pick.0 == cull && pick.1 == mode as u32 {
        return;
    }
    let table = match cull {
        0 => Some(&gpu.pipeline_opaque_mode),
        1 => Some(&gpu.pipeline_opaque_cull_mode),
        _ => None,
    };
    let own = table.and_then(|t| t[mode as usize].as_ref()).and_then(|p| p.ready(crate::render::ms::current()));
    if own.is_some() || pick.2 || cull != pick.0 {
        let base: &wgpu::RenderPipeline = match cull {
            0 => &gpu.pipeline_opaque,
            1 => &gpu.pipeline_opaque_cull,
            _ => &gpu.pipeline_opaque_cull_m,
        };
        pass.set_pipeline(own.unwrap_or(base));
    }
    *pick = (cull, mode as u32, own.is_some());
}

/// What follows a car's step of the tick (vehicles.rs `step_cars`).
#[derive(Clone, Copy)]
enum StepPost {
    None,
    /// a parked car settling: it does not roll or turn
    Settle,
    /// frozen, or held on the grid: settles onto the ground but does not drive
    Frozen,
    /// a driven car (the MCLA_GRID_TRACE hook)
    Trace,
}

/// (MCLA_PHYS_TIMING) the islands of several cars solved so far (`step_cars`)
static ISLANDS_OF_CARS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// (MCLA_RAM_CAR) the rammed car and the player's car, target x 1000 + player
static RAM_TARGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

#[cfg(test)]
mod brake_tests {
    use super::brake_place;
    use glam::{Mat4, Vec4};

    /// The brake's place against F-43091 run offline on the recomp (testdata/brake_place_ref.txt, 40 cars x 4 wheels,
    /// random rims, yaw / tilt and spin, right wheel bones turned over): the rotor (+1856) from the wheel_N matrix, the
    /// caliper (+1600) from the hub_N matrix turned over on a right wheel - every word equal.
    #[test]
    fn brake_place_is_the_originals() {
        let rows = |w: &[u32]| {
            let f = |k: usize| f32::from_bits(w[k]);
            Mat4::from_cols(Vec4::new(f(0), f(1), f(2), f(3)), Vec4::new(f(4), f(5), f(6), f(7)), Vec4::new(f(8), f(9), f(10), f(11)), Vec4::new(f(12), f(13), f(14), f(15)))
        };
        let mut n = 0;
        for line in include_str!("testdata/brake_place_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let v: Vec<&str> = line.split_whitespace().collect();
            let rim_r = mcla_formats::rage::simtune::rim_radius(v[0].parse().unwrap()).unwrap();
            let left = v[1].parse::<usize>().unwrap() % 2 == 0;
            let w: Vec<u32> = v[2..].iter().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let (wm, hub, cal, rot) = (rows(&w[0..16]), rows(&w[16..32]), rows(&w[32..48]), rows(&w[48..64]));
            let side = if left { Mat4::IDENTITY } else { Mat4::from_diagonal(Vec4::new(-1.0, -1.0, 1.0, 1.0)) };
            for (got, want, what) in [(brake_place(wm, rim_r), rot, "rotor"), (brake_place(hub * side, rim_r), cal, "caliper")] {
                let (g, e) = (got.to_cols_array(), want.to_cols_array());
                assert!(g.iter().zip(&e).all(|(a, b)| a == b), "{what} {line}: {g:?} != {e:?}");
            }
            n += 1;
        }
        assert_eq!(n, 160);
    }

    /// A motorcycle's caliper and two discs against F-43091's bike branch run offline (tools/bike_brake_reference.py,
    /// testdata/bike_brake_ref.txt: 60 bikes x 4 wheels, random turns and places, b, d, fRotorOffset and fRotorSize):
    /// every word equal.
    #[test]
    fn bike_brake_place_is_the_originals() {
        let rows = |w: &[u32]| {
            let f = |k: usize| f32::from_bits(w[k]);
            Mat4::from_cols(Vec4::new(f(0), f(1), f(2), f(3)), Vec4::new(f(4), f(5), f(6), f(7)), Vec4::new(f(8), f(9), f(10), f(11)), Vec4::new(f(12), f(13), f(14), f(15)))
        };
        let mut n = 0;
        for line in include_str!("testdata/bike_brake_ref.txt").lines() {
            let v: Vec<&str> = line.split_whitespace().filter(|x| *x != "->").collect();
            let w: Vec<u32> = v[1..].iter().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let f = |k: usize| f32::from_bits(w[k]);
            let (wm, hub) = (rows(&w[4..20]), rows(&w[20..36]));
            let (cal, ra, rb) = super::bike_brake_place(hub, wm, f(0), f(1), f(2), f(3));
            for (got, want, what) in [(cal, rows(&w[36..52]), "caliper"), (ra, rows(&w[52..68]), "rotor A"), (rb, rows(&w[68..84]), "rotor B")] {
                let (g, e) = (got.to_cols_array(), want.to_cols_array());
                assert!(g.iter().zip(&e).all(|(a, b)| a.to_bits() == b.to_bits()), "{what} {line}: {g:?} != {e:?}");
            }
            n += 1;
        }
        assert_eq!(n, 240);
    }
}

#[cfg(test)]
mod needle_tests {
    use super::needle_time;

    /// The needles' time rule of F-35036: one frame of the 30 fps clip per 100 rpm / per mph (L x 30 + 1 frames over L
    /// seconds), clamped to [0, L]; the Vantage's 221-frame speedo at 220 mph and the 81-frame tach at 8000 rpm stand
    /// at the last frame x 220 / 221 and x 80 / 81.
    #[test]
    fn needle_time_steps() {
        let (speedo, tach) = (220.0f32 / 30.0, 80.0f32 / 30.0);
        assert_eq!(needle_time(speedo, 0.0, false), 0.0);
        assert_eq!(needle_time(speedo, -5.0, false), 0.0);
        assert_eq!(needle_time(speedo, 220.0, false), speedo * (220.0 / speedo.mul_add(30.0, 1.0)));
        assert!((needle_time(speedo, 220.0, false) * 30.0 - 220.0 * 220.0 / 221.0).abs() < 1e-3);
        assert!((needle_time(tach, 8000.0, true) * 30.0 - 80.0 * 80.0 / 81.0).abs() < 1e-3);
        assert_eq!(needle_time(speedo, 400.0, false), speedo);
        assert_eq!(needle_time(tach, 20000.0, true), tach);
        // (one mph / 100 rpm moves the needle by nearly one frame)
        let df = (needle_time(speedo, 51.0, false) - needle_time(speedo, 50.0, false)) * 30.0;
        assert!((df - 220.0 / 221.0).abs() < 1e-3, "{df}");
    }
}

#[cfg(test)]
mod roof_tests {
    use super::{RoofCtl, WindowCtl};

    /// The three roof draw tests against the original's F-36938 / F-80163 / F-91671 run offline on 4000 random and edge
    /// times (tools/roof_shown_reference.py): every answer equal.
    #[test]
    fn roof_shown_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for line in include_str!("testdata/roof_shown_ref.txt").lines() {
            let (a, b) = line.split_once('|').unwrap();
            let v: Vec<f32> = a.split_whitespace().map(f).collect();
            let want: Vec<bool> = b.split_whitespace().map(|x| x != "0").collect();
            let r = RoofCtl { top: WindowCtl { state: 3, time: v[0] }, hatch: WindowCtl { state: 3, time: v[2] }, ..Default::default() };
            assert_eq!(r.shown((v[3], v[1])).to_vec(), want, "{line}");
            n += 1;
        }
        assert_eq!(n, 4000);
    }

    /// F-35036's roof part: a request is taken only when both players stand at its start end, opens or closes the side
    /// windows with the roof, and waits otherwise; a lowered roof starts at its end with the windows down.
    #[test]
    fn roof_requests() {
        let lens = (5.0, 7.0);
        let wl = [Some(0.667), Some(0.667)];
        let mut w = [WindowCtl::default(); 2];
        // (made with the option down: at the end, windows down, no motion)
        let mut r = RoofCtl::default();
        r.step(true, true, 0.0, lens, &mut w, wl);
        assert_eq!((r.hatch.state, r.top.state, r.top.time, w[0].state, w[1].time), (2, 2, 7.0, 2, 0.667));
        // (not the player's car: nothing at the start)
        let (mut r2, mut w2) = (RoofCtl::default(), [WindowCtl::default(); 2]);
        r2.step(true, false, 0.0, lens, &mut w2, wl);
        assert_eq!((r2.top.state, w2[0].state), (1, 1));
        // (raised: both close, the windows go up; the hatch ends first)
        r.step(false, true, 1.0, lens, &mut w, wl);
        assert_eq!((r.hatch.state, r.top.state, r.raise, w[0].state), (4, 4, false, 4));
        r.step(false, true, 4.5, lens, &mut w, wl);
        assert_eq!((r.hatch.state, r.top.state), (1, 4));
        // (lowered while the top still closes: the request waits for both at 1)
        r.step(true, true, 0.5, lens, &mut w, wl);
        assert!(r.lower);
        assert_eq!(r.top.state, 4);
        r.step(true, true, 2.0, lens, &mut w, wl);
        assert_eq!((r.top.state, r.hatch.state), (1, 1));
        w = [WindowCtl::default(); 2];
        r.step(true, true, 0.1, lens, &mut w, wl);
        assert_eq!((r.top.state, r.hatch.state, r.lower, w[0].state), (3, 3, false, 3));
        // (the player's car only)
        let mut r3 = RoofCtl { opt: Some(false), ..Default::default() };
        r3.step(true, false, 0.1, lens, &mut w2, wl);
        assert_eq!((r3.top.state, r3.lower), (1, true));
    }
}
