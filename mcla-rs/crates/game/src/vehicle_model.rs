//! Loads a player/traffic car from `resources/vehicle/<name>/`:
//! the main `.r83` supplies the shader group + textures, every `.r63` is a body
//! part. Parts carry their names internally (e.g. "hood0"/"hd00_stk"), so the
//! stock configuration is assembled from the parts marked `_stk` plus the always-on
//! base parts.

use crate::assets::{load_resource_file, Assets};
use anyhow::{Context, Result};
use glam::Vec3;
use hashbrown::HashMap;
use mcla_formats::rage::drawable::Shader;
use mcla_formats::rage::geometry::Mesh;
use mcla_formats::rage::scan::Drawable;
use mcla_formats::rage::skeleton::Skeleton;
use mcla_formats::rage::texture::Texture;
use glam::Mat4;
use mcla_formats::rage::{Dictionary, Rd};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

pub struct CarPart {
    pub file: PathBuf,
    pub names: Vec<String>,
    pub primary: String,
    pub stock: bool,
    pub meshes: Vec<(Mesh, u16)>,
    /// per-mesh bone transform inside the fragment (from the fragment's own skeleton)
    pub mesh_bone: Vec<Mat4>,
    /// per-mesh tag of that bone (the window glass rides on `win_l_anim` / `win_r_anim`)
    pub mesh_tag: Vec<u16>,
    /// per-mesh: the index i of a `wheel_<i>` bone it rides on (a traffic model's own wheels; the
    /// original finds them by that name, sub_82790E68 - their tags differ between models)
    pub mesh_wheel: Vec<Option<usize>>,
    pub tris: usize,
    pub min: Vec3,
    pub max: Vec3,
    /// placement in car space (identity for parts modelled in car space)
    pub transform: Mat4,
    pub placed: bool,
    /// this is the vehicle's base fragment (body + car skeleton)
    pub is_base: bool,
    pub bones: usize,
    /// part file borrowed from the parent car (its shader indices refer to `shaders_parent`)
    pub from_parent: bool,
    /// the exhaust outlets of the part's own skeleton (`exh_primary0`, `exh_secondary0`: rear
    /// bumpers, the wide body), in the part's space
    pub sockets: Vec<(String, Mat4)>,
}

pub struct CarAsset {
    pub name: String,
    pub shaders: Vec<Shader>,
    /// shader group of the parent car (for parts merged into a "_dub" edition)
    pub shaders_parent: Vec<Shader>,
    pub textures: HashMap<u32, Texture>,
    pub parts: Vec<CarPart>,
    pub min: Vec3,
    pub max: Vec3,
    pub skeleton: Option<Skeleton>,
    pub hubs: Option<[Vec3; 4]>,
    /// motorcycle: two wheels on the centre line
    pub bike: bool,
    /// per-car tuning (mass, dimensions, drivetrain, gears, torque curve, top speed)
    pub tune: Option<mcla_formats::rage::vehtune::VehTune>,
    /// the simulation tuning (base and fully upgraded)
    pub simtune: Option<std::sync::Arc<mcla_formats::rage::simtune::SimTunes>>,
    /// the wheels of the car's own set-up (`resources/vehicle/configs/<name>.xcc`), already
    /// written into `simtune`
    pub fit: Option<mcla_formats::rage::simtune::WheelFit>,
    /// what the computer drivers know of the car (stock and fully upgraded)
    pub aitune: Option<std::sync::Arc<mcla_formats::rage::aitune::AiTunes>>,
    /// the body's collision hull as the fragment file has it
    pub hull: Option<mcla_formats::rage::bound::Hull>,
    /// the side windows' roll-down clips (`win_l_open`, `win_r_open` of the base fragment), left
    /// then right
    pub windows: [Option<WindowClip>; 2],
    /// the damage map's layout (`<car>_maxdamage`, the largest of the car's .r83 files, decoded RGBA8; the map's G
    /// and A after a reset - damagemap.rs)
    pub damage_layout: Option<(u32, u32, Vec<u8>)>,
}

/// A side window's clip as the base fragment holds it: `win_l_open` / `win_r_open`, a plain
/// crAnimation (21 frames, 0.667 s on the DB9) with a translation and a rotation track on one
/// bone - `win_l_anim` (tag 0x4137) / `win_r_anim` (0xd554), the bone the glass mesh is bound
/// to. The car's anim object binds it (sub_8233D908 / sub_823404E0) and the window controller
/// sets the player to clamp(time, 0, duration) (sub_8233BEC0).
#[derive(Debug, Clone)]
pub struct WindowClip {
    pub tag: u16,
    pub duration: f32,
    pub anim: mcla_formats::rage::anim::Animation,
    /// the bone's parent in car space and the bone's own rest (bind) matrix in car space
    pub parent_world: Mat4,
    pub rest_world: Mat4,
    pub rest_local: Mat4,
}

impl WindowClip {
    pub fn load(res: &mcla_formats::rsc::Resource, sk: &Skeleton, name: &str) -> Option<WindowClip> {
        let anim = mcla_formats::rage::anim::animation_by_name(res, name)?;
        let tag = anim.tracks.first()?.bone_id;
        let bi = sk.bone_by_tag(tag)?;
        let b = &sk.bones[bi];
        let parent_world = sk.bones.get(b.parent).map_or(Mat4::IDENTITY, |p| p.world);
        Some(WindowClip { tag, duration: anim.duration, parent_world, rest_world: b.world, rest_local: b.local, anim })
    }

    /// The bone's local matrix at a clip time (clamped to the clip, never wrapped).
    pub fn local_at(&self, time: f32) -> Mat4 {
        use mcla_formats::rage::anim::{KIND_ROTATION, KIND_TRANSLATION};
        let last = (self.anim.frames.max(1) - 1) as f32;
        let f = if self.duration > 0.0 { (time / self.duration * last).clamp(0.0, last) } else { 0.0 };
        let (_, rest_r, rest_t) = self.rest_local.to_scale_rotation_translation();
        let t = self.anim.track(KIND_TRANSLATION, self.tag).map_or(rest_t, |tr| tr.data.vec(f));
        let r = self.anim.track(KIND_ROTATION, self.tag).map_or(rest_r, |tr| tr.data.rot(f));
        Mat4::from_rotation_translation(r, t)
    }

    /// What the bone's motion does to the glass, in car space: the meshes are baked at the
    /// rest pose, so this maps the rest pose to the pose at `time`.
    pub fn delta(&self, time: f32) -> Mat4 {
        self.parent_world * self.local_at(time) * self.rest_world.inverse()
    }
}

/// Slot classification used to pick the hinge bone for a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Hash)]
pub enum Slot {
    Hood,
    Trunk,
    BumperF,
    BumperR,
    DoorL,
    DoorR,
    Steer,
    Spoiler,
    Exhaust,
    Intercooler,
    Skirts,
    Seats,
    /// body shell / fender kit (fenders0 stock, widebody0 for the wide kit)
    Body,
    Wheels,
    Paint,
    /// door interior panels: ride on the door bones but are never customised
    DoorLInt,
    DoorRInt,
    CarSpace,
    Unknown,
}

/// Customisable slots shown in the garage, in display order.
pub const CUSTOM_SLOTS: [Slot; 10] = [Slot::Paint, Slot::Wheels, Slot::Body, Slot::Hood, Slot::Spoiler, Slot::BumperF, Slot::BumperR, Slot::Skirts, Slot::Exhaust, Slot::Seats];

impl Slot {
    pub fn label(&self) -> &'static str {
        match self {
            Slot::Paint => "Paint",
            Slot::Wheels => "Rims",
            Slot::Hood => "Hood",
            Slot::Spoiler => "Spoiler",
            Slot::BumperF => "Front bumper",
            Slot::BumperR => "Rear bumper",
            Slot::Skirts => "Side skirts",
            Slot::Exhaust => "Exhaust",
            Slot::Seats => "Seats",
            Slot::Steer => "Steering wheel",
            Slot::Intercooler => "Intercooler",
            Slot::Trunk => "Trunk",
            Slot::DoorL => "Left door",
            Slot::DoorR => "Right door",
            Slot::Body => "Body kit",
            Slot::DoorLInt => "Door interior L",
            Slot::DoorRInt => "Door interior R",
            Slot::CarSpace => "Body",
            Slot::Unknown => "Other",
        }
    }
}

/// Selected variant index per customisable slot (0 = stock).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CarConfig {
    pub paint: usize,
    pub rim: usize,
    pub hood: usize,
    pub spoiler: usize,
    pub bumper_f: usize,
    pub bumper_r: usize,
    pub skirts: usize,
    pub exhaust: usize,
    pub seats: usize,
    #[serde(default)]
    pub body: usize,
    /// The parts the garage's other lists change, by the section of the car's part file
    /// (`headlight`, `taillight`, `steer_whl`, `post_gauge`, `blower`, `side_exhaust`): the
    /// part's index there, which is the number at the end of the model part's name
    /// (`headlight2`, `blower3`, `exh_side3` ...). Absent = 0, the stock part - or none,
    /// where the car has no part 0 of the kind (blowers, side exhausts).
    #[serde(default)]
    pub extra: std::collections::BTreeMap<String, usize>,
    /// Parts painted by themselves (the paint shop's PAINT OPTIONS: PARTS; the record's six
    /// paint entries): the part's kind ("hood" ...) -> its colour, as the screen shows it.
    #[serde(default)]
    pub part_paint: std::collections::BTreeMap<String, [u8; 3]>,
}

/// The model parts of `CarConfig::extra`: (section of the part file, the parts' name stem).
pub const EXTRA_PARTS: [(&str, &str); 9] = [("headlight", "headlight"), ("taillight", "taillight"), ("steer_whl", "steer_whl"), ("post_gauge", "post_gauge"), ("blower", "blower"), ("side_exhaust", "exh_side"),
    // (the bikes' three body parts: models bike_tail0..2, bike_tank0..2, bike_cowl0..2)
    ("bike_tail", "bike_tail"), ("bike_tank", "bike_tank"), ("bike_cowl", "bike_cowl")];

/// Keys of `CarConfig::extra` that are switches, not part numbers: "intercooler" (the
/// record's UseIntercooler: the intercooler is fitted - which model, `intercooler<n>`, goes
/// by the front bumper's number) and "carbon_hood" (CarbonFiberHood: the hood is drawn in
/// carbon fibre).
pub const EXTRA_FLAGS: [&str; 2] = ["intercooler", "carbon_hood"];

/// `CarConfig::extra` key: the convertible top is down (the options' CONVERTIBLE TOP; put
/// there by `Game::sync_extra_parts`, not a part of the vehicle).
pub const ROOF_DOWN: &str = "roof_down";

/// A part of the convertible roof: 0 the raised roof (`conv_topup<n>`), 1 the folded one
/// (`conv_topdown<n>`), 2 the skinned roof that is drawn while it moves (`conv_top<n>`).
pub fn roof_part(primary: &str) -> Option<u8> {
    let p = primary.to_ascii_lowercase();
    let digits = |r: &str| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit());
    if p.strip_prefix("conv_topup").map_or(false, digits) {
        Some(0)
    } else if p.strip_prefix("conv_topdown").map_or(false, digits) {
        Some(1)
    } else if p.strip_prefix("conv_top").map_or(false, digits) {
        Some(2)
    } else {
        None
    }
}

/// If `primary` is a part of one of the `EXTRA_PARTS` kinds: its section and number.
pub fn extra_part(primary: &str) -> Option<(&'static str, usize)> {
    let p = primary.to_ascii_lowercase();
    EXTRA_PARTS.iter().find_map(|(sec, stem)| {
        let rest = p.strip_prefix(stem)?;
        (!rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())).then(|| (*sec, rest.parse().unwrap_or(0)))
    })
}

impl CarConfig {
    pub fn get(&self, slot: Slot) -> usize {
        match slot {
            Slot::Paint => self.paint,
            Slot::Wheels => self.rim,
            Slot::Hood => self.hood,
            Slot::Spoiler => self.spoiler,
            Slot::BumperF => self.bumper_f,
            Slot::BumperR => self.bumper_r,
            Slot::Skirts => self.skirts,
            Slot::Exhaust => self.exhaust,
            Slot::Seats => self.seats,
            Slot::Body => self.body,
            _ => 0,
        }
    }
    pub fn set(&mut self, slot: Slot, v: usize) {
        match slot {
            Slot::Paint => self.paint = v,
            Slot::Wheels => self.rim = v,
            Slot::Hood => self.hood = v,
            Slot::Spoiler => self.spoiler = v,
            Slot::BumperF => self.bumper_f = v,
            Slot::BumperR => self.bumper_r = v,
            Slot::Skirts => self.skirts = v,
            Slot::Exhaust => self.exhaust = v,
            Slot::Seats => self.seats = v,
            Slot::Body => self.body = v,
            _ => {}
        }
    }
}

pub fn classify_slot(primary: &str, names: &[String]) -> Slot {
    let p = primary.to_ascii_lowercase();
    // the base body fragment is never a slot part, whatever its meshes are called
    let base = p.starts_with("body_0x");
    let any = |pat: &str| !base && names.iter().any(|n| n.to_ascii_lowercase().contains(pat));
    if p.starts_with("hood") || p.starts_with("hd") || any("hood") && !p.contains("scoop") {
        Slot::Hood
    } else if p.starts_with("trunk") || p.starts_with("hatch") || p.starts_with("tailgate") {
        Slot::Trunk
    } else if p.starts_with("bumper_f") || p.starts_with("bumf") {
        Slot::BumperF
    } else if p.starts_with("bumper_r") || p.starts_with("bumr") {
        Slot::BumperR
    } else if (p.starts_with("door_l") && p.contains("int")) || p.starts_with("interior_door_l") {
        Slot::DoorLInt
    } else if (p.starts_with("door_r") && p.contains("int")) || p.starts_with("interior_door_r") {
        Slot::DoorRInt
    } else if p.starts_with("door_l") && !p.contains("int") {
        Slot::DoorL
    } else if p.starts_with("door_r") && !p.contains("int") {
        Slot::DoorR
    } else if p.starts_with("steer") {
        Slot::Steer
    } else if p.starts_with("spoiler") || p.starts_with("splr") || p.starts_with("wing") {
        Slot::Spoiler
    } else if p.starts_with("exh") || p.starts_with("exst") {
        Slot::Exhaust
    } else if p.starts_with("fenders") || p.starts_with("pfenders") || p.starts_with("widebody") {
        Slot::Body
    } else if p.starts_with("intercooler") || p.starts_with("ic0") {
        Slot::Intercooler
    } else if p.starts_with("skirts") || p.starts_with("ss0") {
        Slot::Skirts
    } else if p.starts_with("seats_f") {
        Slot::Seats
    } else if p.starts_with("interior") || p.starts_with("headlight") || p.starts_with("taillight") || p.starts_with("seats") || p.starts_with("roof") || p.starts_with("mirror") || p.starts_with("grill") || p.starts_with("body") || p.starts_with("chassis") {
        Slot::CarSpace
    } else {
        Slot::Unknown
    }
}

/// Global bone ids used by the vehicle skeletons for the customisation slots
/// (identical across every car; derived statistically from the whole car set).
/// the bone the add-on gauge stands on
pub const POST_GAUGE_TAG: u16 = 12783;
/// the stock spoiler that moves by itself (`spoiler_raise`; the Audi R8's) has its own bone
pub const ACTIVE_SPOILER_TAG: u16 = 60148;
/// the spoiler bone of the cars whose lid is no trunk (engine cover, hatch glass)
pub const SPOILER_TAG: u16 = 16868;

/// The bones a spoiler may stand on, first one the car has: the stock spoiler of the R8 on the
/// active spoiler's bone (reference: the original's draws of `spoiler0` have their origin at
/// that bone + the part's own bone, capture tg_12), any spoiler on the car's spoiler bone
/// (R8, Murcielago, Ford GT, Cobra), else on the trunk lid.
pub fn spoiler_tags(primary: &str) -> Vec<u16> {
    if std::env::var("MCLA_MOUNT_OLD").is_ok() {
        return vec![33531];
    }
    let p = primary.to_ascii_lowercase();
    if p == "spoiler0" {
        vec![ACTIVE_SPOILER_TAG, SPOILER_TAG, 33531]
    } else {
        vec![SPOILER_TAG, 33531]
    }
}

pub fn slot_tag(slot: Slot) -> Option<u16> {
    Some(match slot {
        Slot::Hood => 33332,
        Slot::BumperF | Slot::Intercooler => 13687,
        Slot::BumperR | Slot::Exhaust => 13699,
        Slot::DoorL | Slot::DoorLInt => 27940,
        Slot::DoorR | Slot::DoorRInt => 25582,
        Slot::Steer => 18166,
        Slot::Spoiler | Slot::Trunk => 33531,
        _ => return None,
    })
}

/// Structural fallback: door bones are the parents of the mirror (0x4137 / 0xd554) and
/// door-handle (0x6da3 / 0xea01) bones, which keep their ids even when the door tag differs.
pub fn infer_slot_bone(slot: Slot, sk: &Skeleton) -> Option<usize> {
    let left = match slot {
        Slot::DoorL | Slot::DoorLInt => true,
        Slot::DoorR | Slot::DoorRInt => false,
        _ => return None,
    };
    // 0. the door hinge chain: root (tag 0x20f1/0x98d6 left, 0x20f7/0x0927 right) -> pivot -> door bone
    let root_tags: &[u16] = if left { &[0x20f1, 0x98d6] } else { &[0x20f7, 0x0927] };
    for tag in root_tags {
        if let Some(root) = sk.bone_by_tag(*tag) {
            let pivots: Vec<usize> = (0..sk.bones.len()).filter(|&i| sk.bones[i].parent == root).collect();
            for pv in pivots {
                let doors: Vec<usize> = (0..sk.bones.len()).filter(|&i| sk.bones[i].parent == pv).collect();
                // the door bone is the one that itself has children (handle / mirror); else the first
                if let Some(d) = doors.iter().copied().find(|&d| sk.bones.iter().any(|b| b.parent == d)).or(doors.first().copied()) {
                    return Some(d);
                }
            }
        }
    }
    // 1. the mirror / handle child ids used by most cars
    let child_tags: &[u16] = if left { &[0x4137, 0x6da3] } else { &[0xd554, 0xea01] };
    for tag in child_tags {
        if let Some(ci) = sk.bone_by_tag(*tag) {
            let p = sk.bones[ci].parent;
            if p < sk.bones.len() {
                return Some(p);
            }
        }
    }
    // 2. structural: every door bone owns a door-handle child whose rest rotation is the
    //    same fixed tilt (y axis = (0, 0.4, -0.92)); the handle's parent is the door bone
    let mut best: Option<(usize, f32)> = None;
    for (i, b) in sk.bones.iter().enumerate() {
        let y = b.world.y_axis.truncate();
        if (y.y - 0.4).abs() < 0.06 && (y.z + 0.92).abs() < 0.06 && y.x.abs() < 0.06 {
            let p = b.parent;
            if p < sk.bones.len() {
                let t = sk.bones[p].world.w_axis.truncate();
                if t.x.abs() > 0.5 && (t.x < 0.0) == left && t.y > 0.4 && t.y < 1.2 {
                    // prefer the hinge closest to the front door region
                    let score = (t.z + 0.8).abs();
                    if best.map_or(true, |(_, s)| score < s) {
                        best = Some((p, score));
                    }
                }
            }
        }
    }
    best.map(|b| b.0)
}

/// Candidate hinge bone translations for a slot (world bone positions matching the slot's region).
pub fn slot_candidates(slot: Slot, sk: &Skeleton) -> Vec<Vec3> {
    let ts: Vec<Vec3> = (0..sk.bones.len()).map(|i| sk.translation(i)).collect();
    let f: Box<dyn Fn(Vec3) -> bool> = match slot {
        Slot::Hood => Box::new(|t: Vec3| t.x.abs() < 0.08 && t.y > 0.6 && t.z < -0.3),
        Slot::Trunk | Slot::Spoiler => Box::new(|t: Vec3| t.x.abs() < 0.08 && t.y > 0.6 && t.z > 0.3),
        Slot::BumperF | Slot::Intercooler => Box::new(|t: Vec3| t.x.abs() < 0.08 && t.y < 0.7 && t.z < -1.0),
        Slot::BumperR | Slot::Exhaust => Box::new(|t: Vec3| t.x.abs() < 0.08 && t.y < 0.7 && t.z > 1.0),
        Slot::DoorL => Box::new(|t: Vec3| t.x < -0.5 && t.y > 0.45 && t.y < 1.05 && t.z.abs() < 1.0),
        Slot::DoorR => Box::new(|t: Vec3| t.x > 0.5 && t.y > 0.45 && t.y < 1.05 && t.z.abs() < 1.0),
        Slot::Steer => Box::new(|t: Vec3| t.x < -0.15 && t.x > -0.5 && t.y > 0.75 && t.y < 1.05 && t.z < -0.3 && t.z > -0.9),
        _ => Box::new(|_t: Vec3| false),
    };
    let mut v: Vec<Vec3> = ts.into_iter().filter(|t| f(*t)).collect();
    v.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap().then(a.y.partial_cmp(&b.y).unwrap()).then(a.z.partial_cmp(&b.z).unwrap()));
    v.dedup_by(|a, b| a.distance(*b) < 0.01);
    v
}

/// Pick the candidate whose placed bounding box fits the body volume best.
pub fn best_anchor(slot: Slot, cands: &[Vec3], pmin: Vec3, pmax: Vec3, body_min: Vec3, body_max: Vec3, sill: f32) -> Option<Vec3> {
    let mut best: Option<(f32, Vec3)> = None;
    for &a in cands {
        let mn = pmin + a;
        let mx = pmax + a;
        let out = (body_min - mn).max(Vec3::ZERO) + (mx - body_max).max(Vec3::ZERO);
        let mut score = out.x + out.y * 2.0 + out.z;
        // doors: bottom should sit at the sill
        if matches!(slot, Slot::DoorL | Slot::DoorR) {
            score += (mn.y - sill).abs() * 2.0;
        }
        if best.map_or(true, |b| score < b.0) {
            best = Some((score, a));
        }
    }
    best.map(|b| b.1)
}

/// The three LOD levels of the body fragment that carries the car skeleton (same hashes on every car).
const BASE_FRAGMENTS: &[&str] = &["0x0de3ddb3.r63", "0x7b0cd281.r63", "0xf9785b04.r63"];

const BASE_PARTS: &[&str] = &[
    "interior0", "door_l_int0", "door_r_int0", "interior_door_l", "interior_door_r", "arm_front_l", "arm_front_r", "arm_rear_l", "arm_rear_r",
    "guide_rear_l", "guide_rear_r", "fenders0", "taillight0", "headlight0", "exh_pipe0", "seats_f0", "seats_p0",
    "steer_00", "hood0", "door_l0", "door_r0", "bumper_f0", "bumper_r0", "skirts0", "trunk0", "roof0", "mirror_l0",
    "mirror_r0", "wing0", "grill0", "body0", "chassis0", "dash0", "engine0", "frame0", "underbody0",
];

fn part_names(res: &mcla_formats::rsc::Resource) -> Vec<String> {
    let v = &res.data[..res.virtual_size];
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if v[i].is_ascii_lowercase() {
            let start = i;
            while i < v.len() && (v[i].is_ascii_alphanumeric() || v[i] == b'_') {
                i += 1;
            }
            if i < v.len() && v[i] == 0 && i - start >= 4 && i - start <= 48 {
                let s = String::from_utf8_lossy(&v[start..i]).to_string();
                if !s.ends_with(".dds") && !s.contains("sampler") && !out.contains(&s) {
                    out.push(s);
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Every NUL-terminated identifier (any case) in a part catalog resource.
fn catalog_names(res: &mcla_formats::rsc::Resource) -> Vec<String> {
    let v = &res.data[..];
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if v[i].is_ascii_alphabetic() {
            let start = i;
            while i < v.len() && (v[i].is_ascii_alphanumeric() || v[i] == b'_') {
                i += 1;
            }
            if i < v.len() && v[i] == 0 && i - start >= 4 && i - start <= 48 {
                out.push(String::from_utf8_lossy(&v[start..i]).to_string());
            }
        } else {
            i += 1;
        }
    }
    out
}

/// `root`: the name of the part's own root bone, which is the part's name (`exh_side2`,
/// `spoiler3`, `subbumf0`). It is taken where the scan of the file finds no part name: the scan
/// reads stray bytes into names (`aspoiler3`, `psubbumf0`) or meets a bone first (`nitro0` in
/// the side exhausts), and such parts were never drawn.
fn primary_name(names: &[String], root: &str) -> String {
    let slot_re = |s: &str| {
        let ok = s.starts_with("hood")
            || s.starts_with("door_")
            || s.starts_with("bumper_")
            || s.starts_with("fenders")
            || s.starts_with("widebody")
            || s.starts_with("pfenders")
            || s.starts_with("skirts")
            || s.starts_with("spoiler")
            || s.starts_with("headlight")
            || s.starts_with("taillight")
            || s.starts_with("interior")
            || s.starts_with("seats_")
            || s.starts_with("steer_")
            || s.starts_with("exh_pipe")
            || s.starts_with("arm_")
            || s.starts_with("guide_")
            || s.starts_with("intercooler")
            || s.starts_with("post_gauge")
            || s.starts_with("occluder")
            || s.starts_with("trunk")
            || s.starts_with("roof")
            || s.starts_with("mirror")
            || s.starts_with("wing")
            || s.starts_with("grill")
            || s.starts_with("body")
            || s.starts_with("chassis")
            || s.starts_with("hood_wide")
            || s.starts_with("neon")
            || s.starts_with("scoop")
            || s.starts_with("nitrous")
            || s.starts_with("hydraulics");
        ok && !s.contains("anim") && !s.starts_with("glow")
    };
    let root_ok = !root.is_empty() && root.len() >= 4 && root.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') && std::env::var("MCLA_MOUNT_OLD").is_err();
    names.iter().find(|s| slot_re(s)).cloned().unwrap_or_else(|| if root_ok { root.to_string() } else { names.first().cloned().unwrap_or_default() })
}

pub fn load_car(assets: &Assets, car_dir: &Path) -> Result<CarAsset> {
    let name = car_dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut r83 = Vec::new();
    let mut r63 = Vec::new();
    let mut r50 = Vec::new();
    let mut r19 = Vec::new();
    for e in std::fs::read_dir(car_dir).with_context(|| format!("list {}", car_dir.display()))? {
        let p = e?.path();
        match p.extension().and_then(|x| x.to_str()) {
            Some("r83") => r83.push(p),
            Some("r63") => r63.push(p),
            Some("r50") => r50.push(p),
            Some("r19") => r19.push(p),
            _ => {}
        }
    }
    // "_dub" editions only ship the parts that differ; everything else comes from the parent car
    let mut parent_files: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut parent_r83: Vec<PathBuf> = Vec::new();
    if let Some(parent_name) = name.strip_suffix("_dub") {
        let parent = car_dir.with_file_name(parent_name);
        if parent.is_dir() {
            let have: std::collections::HashSet<std::ffi::OsString> = r63.iter().filter_map(|p| p.file_name().map(|n| n.to_os_string())).collect();
            if let Ok(rd) = std::fs::read_dir(&parent) {
                for e in rd.flatten() {
                    let p = e.path();
                    match p.extension().and_then(|x| x.to_str()) {
                        Some("r63") if !p.file_name().map_or(false, |n| have.contains(n)) => {
                            parent_files.insert(p.clone());
                            r63.push(p);
                        }
                        Some("r83") => parent_r83.push(p),
                        _ => {}
                    }
                }
            }
            log::info!("car {}: merged {} part files from parent {}", name, parent_files.len(), parent_name);
        }
    }
    let tune = r50.iter().find_map(|p| load_resource_file(p).ok().and_then(|res| mcla_formats::rage::vehtune::VehTune::parse(&res)));
    let mut simtune = r50.iter().find_map(|p| load_resource_file(p).ok().and_then(|res| mcla_formats::rage::simtune::SimTunes::parse(&res)));
    // (the car's stock set-up decides its wheels, as it does in the original for every car)
    let fit = simtune.as_mut().and_then(|s| {
        let cfg = car_dir.parent().map(|v| v.join("configs").join(format!("{name}.xcc"))).and_then(|p| std::fs::read(p).ok())?;
        s.fit(&cfg)
    });
    if let Some(f) = fit.as_ref() {
        log::info!("car {}: wheels as set up: {:?}", name, f);
    }
    // (a live rear axle: the part whose skeleton has the bone `axle0` - 0x82357B80 looks it up
    // by that name and hands its matrix to the rear vehAxle)
    if let Some(s) = simtune.as_mut() {
        let bone = r63.iter().filter(|p| p.file_name().map_or(false, |n| n.to_string_lossy().eq_ignore_ascii_case("0x038c6bf5.r63"))).find_map(|p| {
            let sk = Skeleton::parse(&load_resource_file(p).ok()?).ok().flatten()?;
            let b = sk.bones.iter().find(|b| b.name.eq_ignore_ascii_case("axle0")).or(sk.bones.first().filter(|_| sk.bones.len() == 1))?;
            Some([b.local.x_axis.truncate().to_array(), b.local.y_axis.truncate().to_array(), b.local.z_axis.truncate().to_array(), b.local.w_axis.truncate().to_array()])
        });
        if let Some(m) = bone {
            log::info!("car {}: live rear axle, its bone at z {:.4}", name, m[3][2]);
        }
        s.base.axle_bone_z = bone.map(|m| m[3][2]);
        s.mods.axle_bone_z = bone.map(|m| m[3][2]);
        s.base.axle_bone = bone;
        s.mods.axle_bone = bone;
        // (Z3: a steered wheel turns about its part's own up, wheel +400 - straight up on a car; a
        // motorcycle's front wheel hangs in a raked fork: the first skeleton with a bone whose
        // second row leans along the car gives the axis)
        let axis = r63.iter().find_map(|p| {
            let sk = Skeleton::parse(&load_resource_file(p).ok()?).ok().flatten()?;
            // (a hub on the centre line: only a motorcycle has one)
            let b = sk.bones.iter().find(|b| b.name.to_ascii_lowercase().starts_with("hub") && b.local.w_axis.x.abs() < 0.06 && b.local.y_axis.x == 0.0 && b.local.y_axis.z.abs() > 0.05 && b.local.y_axis.y > 0.5 && b.local.x_axis.x.abs() > 0.99)?;
            Some((b.name.clone(), b.local.y_axis.truncate().to_array()))
        });
        if let Some((bone, a)) = axis.as_ref() {
            log::info!("car {}: steering axis {:?} from bone {} ", name, a, bone);
        }
        s.base.steer_axis = axis.as_ref().map(|a| a.1);
        s.mods.steer_axis = axis.as_ref().map(|a| a.1);
    }
    let simtune = simtune.map(std::sync::Arc::new);
    let aitune = r50.iter().find_map(|p| load_resource_file(p).ok().and_then(|res| mcla_formats::rage::aitune::AiTunes::parse(&res))).map(std::sync::Arc::new);
    if let Some(s) = simtune.as_ref() {
        let b = &s.base;
        log::info!(
            "car {}: sim tune mass {:.0} gravity {:.2} fric {:.2}/{:.2} slide {:.2}/{:.2} opt slip {:.2}/{:.2} hp {:.0} gears {} drivetrain {}",
            name, b.s.mass, b.s.bound_gravity, b.wheel_front.static_fric, b.wheel_back.static_fric, b.wheel_front.sliding_fric, b.wheel_back.sliding_fric, b.wheel_front.optimum_slip, b.wheel_back.optimum_slip, b.engine.s.max_hp, b.trans.gears, b.drivetrain_type
        );
    }
    if let Some(t) = tune.as_ref() {
        log::info!("car {}: tuning mass {} dims {:?} drivetrain {} top {:.1} m/s gears {:?} redline {:.0} hp {:.0} grip {:.2}/{:.2} steer {:.2} accel samples {}", name, t.mass, t.dims, t.drivetrain, t.engine.top_speed, t.engine.gears, t.redline, t.horsepower, t.front.grip, t.rear.grip, t.front.steer, t.engine.accel.len());
    } else {
        log::warn!("car {}: no tuning resource", name);
    }
    // (the collision hull: any of the car's own fragment files carries it; a "_dub" edition's
    // may be its parent's)
    let hull = r83.iter().chain(parent_r83.iter()).find_map(|p| load_resource_file(p).ok().and_then(|res| mcla_formats::rage::bound::find_hull(&res)));
    // main: the r83 whose shader group has the most named shaders
    let mut shaders = Vec::new();
    let mut textures: HashMap<u32, Texture> = HashMap::new();
    let mut damage_layout: Option<(u32, u32, Vec<u8>)> = None;
    for p in &r83 {
        let res = load_resource_file(p)?;
        let d = Drawable::parse(&res)?;
        if d.shader_group.shaders.len() > shaders.len() {
            shaders = d.shader_group.shaders;
        }
        let rd = Rd::new(&res);
        for t in Texture::find_by_suffix(&rd, "_maxdamage") {
            if damage_layout.as_ref().map_or(true, |l| t.width > l.0) {
                damage_layout = Some((t.width, t.height, t.to_rgba8_gpu()));
            }
        }
        for (h, o) in &d.texture_offsets {
            if let Ok(t) = Texture::parse(&rd, *o) {
                textures.entry(*h).or_insert(t);
            }
        }
    }
    // parent shader group (+ its textures) for the merged parts
    let mut shaders_parent: Vec<Shader> = Vec::new();
    for p in &parent_r83 {
        if let Ok(res) = load_resource_file(p) {
            if let Ok(d) = Drawable::parse(&res) {
                if d.shader_group.shaders.len() > shaders_parent.len() {
                    shaders_parent = d.shader_group.shaders;
                }
                let rd = Rd::new(&res);
                for (h, o) in &d.texture_offsets {
                    if let Ok(t) = Texture::parse(&rd, *o) {
                        textures.entry(*h).or_insert(t);
                    }
                }
            }
        }
    }
    let parts: Vec<CarPart> = r63
        .par_iter()
        .filter_map(|p| {
            let res = load_resource_file(p).ok()?;
            let names = part_names(&res);
            let d = Drawable::parse(&res).ok()?;
            let frag_sk = Skeleton::parse(&res).ok().flatten();
            let nbones = frag_sk.as_ref().map(|s| s.bones.len()).unwrap_or(0);
            let mut meshes = Vec::new();
            let mut mesh_bone = Vec::new();
            let mut mesh_tag = Vec::new();
            let mut mesh_wheel = Vec::new();
            let mut mn = Vec3::splat(f32::INFINITY);
            let mut mx = Vec3::splat(f32::NEG_INFINITY);
            let mut tris = 0;
            for m in d.models {
                let bm = frag_sk.as_ref().and_then(|s| s.bones.get(m.bone as usize)).map(|b| b.world).unwrap_or(Mat4::IDENTITY);
                let btag = frag_sk.as_ref().and_then(|s| s.bones.get(m.bone as usize)).map_or(0, |b| b.tag);
                let wheel = frag_sk.as_ref().and_then(|s| s.bones.get(m.bone as usize)).and_then(|b| b.name.to_ascii_lowercase().strip_prefix("wheel_").and_then(|k| k.parse::<usize>().ok())).filter(|&i| i < 6);
                for (mesh, si) in m.parts {
                    let (a, b) = mesh.bbox();
                    let (a, b) = if bm == Mat4::IDENTITY {
                        (a, b)
                    } else {
                        let mut lo = Vec3::splat(f32::INFINITY);
                        let mut hi = Vec3::splat(f32::NEG_INFINITY);
                        for i in 0..8 {
                            let c = Vec3::new(if i & 1 == 0 { a.x } else { b.x }, if i & 2 == 0 { a.y } else { b.y }, if i & 4 == 0 { a.z } else { b.z });
                            let w = bm.transform_point3(c);
                            lo = lo.min(w);
                            hi = hi.max(w);
                        }
                        (lo, hi)
                    };
                    mn = mn.min(a);
                    mx = mx.max(b);
                    tris += mesh.indices.len() / 3;
                    meshes.push((mesh, si));
                    mesh_bone.push(bm);
                    mesh_tag.push(btag);
                    mesh_wheel.push(wheel);
                }
            }
            if meshes.is_empty() {
                return None;
            }
            let primary = primary_name(&names, frag_sk.as_ref().and_then(|s| s.bones.first()).map_or("", |b| b.name.as_str()));
            let is_anim = names.iter().any(|n| n.contains("_anim") || n.ends_with("_open") || n.ends_with("_close"));
            let fname = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let is_base = BASE_FRAGMENTS.contains(&fname.as_str());
            let stock = (!is_anim || is_base) && (is_base || names.iter().any(|n| n.ends_with("_stk") || n.to_ascii_lowercase().ends_with("_dub")) || BASE_PARTS.contains(&primary.as_str()));
            let sockets: Vec<(String, Mat4)> = frag_sk.as_ref().map(|s| s.bones.iter().filter(|b| { let n = b.name.to_ascii_lowercase(); n.starts_with("exh_primary") || n.starts_with("exh_secondary") }).map(|b| (b.name.to_ascii_lowercase(), b.world)).collect()).unwrap_or_default();
            Some(CarPart { file: p.clone(), names, primary: if is_base { format!("body_{}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()) } else { primary }, stock, meshes, mesh_bone, mesh_tag, mesh_wheel, tris, min: mn, max: mx, transform: Mat4::IDENTITY, placed: true, is_base, bones: nbones, from_parent: parent_files.contains(p), sockets })
        })
        .collect();
    // skeleton: the LOD0 base fragment; otherwise the part file with the most bone matrices
    let mut skeleton: Option<Skeleton> = None;
    if let Some(p) = r63.iter().find(|p| p.file_name().map_or(false, |n| n.to_string_lossy().eq_ignore_ascii_case("0xf9785b04.r63"))) {
        if let Ok(res) = load_resource_file(p) {
            if let Ok(Some(sk)) = Skeleton::parse(&res) {
                if sk.bones.len() >= 8 {
                    skeleton = Some(sk);
                }
            }
        }
    }
    let need_scan = skeleton.is_none();
    for p in r63.iter().filter(|_| need_scan) {
        if let Ok(res) = load_resource_file(p) {
            if let Ok(Some(sk)) = Skeleton::parse(&res) {
                if sk.bones.len() >= 8 && skeleton.as_ref().map_or(true, |s| sk.bones.len() > s.bones.len()) {
                    skeleton = Some(sk);
                }
            }
        }
    }
    // (the side windows' clips: in every base fragment; the LOD0 one first)
    let mut windows: [Option<WindowClip>; 2] = [None, None];
    if let Some(sk) = skeleton.as_ref() {
        for f in BASE_FRAGMENTS.iter().rev() {
            if windows.iter().all(|w| w.is_some()) {
                break;
            }
            let Some(p) = r63.iter().find(|p| p.file_name().map_or(false, |n| n.to_string_lossy().eq_ignore_ascii_case(f))) else { continue };
            let Ok(res) = load_resource_file(p) else { continue };
            for (k, n) in ["win_l_open", "win_r_open"].iter().enumerate() {
                if windows[k].is_none() {
                    windows[k] = WindowClip::load(&res, sk, n);
                }
            }
        }
        for (k, w) in windows.iter().enumerate() {
            if let Some(w) = w {
                log::debug!("car {}: window {} clip {:.3} s, {} frames, bone tag {:#06x}, glass drops {:.3}", name, k, w.duration, w.anim.frames, w.tag, (w.delta(w.duration).transform_point3(w.rest_world.w_axis.truncate()) - w.rest_world.w_axis.truncate()).length());
            }
        }
    }
    let mut hubs = skeleton.as_ref().and_then(|s| s.hubs());
    // motorcycles: two hubs on the centre line; the physics rides on a narrow virtual axle pair
    let mut bike = false;
    if hubs.is_none() {
        if let Some([f, r]) = skeleton.as_ref().and_then(|s| s.bike_hubs()) {
            bike = true;
            let w = 0.14;
            hubs = Some([Vec3::new(-w, f.y, f.z), Vec3::new(w, f.y, f.z), Vec3::new(-w, r.y, r.z), Vec3::new(w, r.y, r.z)]);
        }
    }
    // several base fragments = LOD levels; keep the most detailed one
    for (i, p) in parts.iter().enumerate() {
        if p.is_base {
            log::info!("base fragment #{} {:?}: {} bones, {} tris, bbox ({:.2},{:.2},{:.2})..({:.2},{:.2},{:.2})", i, p.file.file_name().unwrap(), p.bones, p.tris, p.min.x, p.min.y, p.min.z, p.max.x, p.max.y, p.max.z);
        }
    }
    if let Some(sk) = skeleton.as_ref() {
        log::info!("car {}: skeleton {} bones, hubs {:?}", name, sk.bones.len(), hubs);
    }
    let mut parts = parts;
    // the three base fragments are LOD levels (LOD0 ~7k tris, LOD1 ~4k, LOD2 = a 54-tri box);
    // keep only the most detailed one
    let best_base = parts.iter().filter(|p| p.is_base).map(|p| p.tris).max().unwrap_or(0);
    for p in parts.iter_mut() {
        if p.is_base && p.tris < best_base {
            p.stock = false;
            p.placed = false;
        }
    }
    let lod_drop: Vec<std::path::PathBuf> = parts.iter().filter(|p| p.is_base && !p.stock).map(|p| p.file.clone()).collect();
    parts.retain(|p| !lod_drop.contains(&p.file));
    // parts borrowed from the parent car must be listed in the edition's own part catalog
    // (the catalog names every mesh the edition uses, e.g. "exh_pipe0_lod_0", "seats_f0_lod_0")
    if parts.iter().any(|p| p.from_parent) {
        let mut catalog: std::collections::HashSet<String> = std::collections::HashSet::new();
        for p in &r19 {
            if let Ok(res) = load_resource_file(p) {
                for n in catalog_names(&res) {
                    catalog.insert(n.trim_end_matches("_lod_0").to_ascii_lowercase());
                }
            }
        }
        if !catalog.is_empty() {
            let mut cn: Vec<&String> = catalog.iter().collect();
            cn.sort();
            log::debug!("catalog: {:?}", cn);
            for p in parts.iter().filter(|p| p.from_parent).take(3) {
                log::debug!("parent part {:?} names {:?}", p.primary, p.names);
            }
            let before = parts.len();
            parts.retain(|p| !p.from_parent || p.names.iter().any(|n| catalog.contains(&n.trim_end_matches("_lod_0").to_ascii_lowercase())));
            log::info!("car {}: dropped {} parent parts not in the edition's catalog ({} catalog names)", name, before - parts.len(), catalog.len());
        }
    }
    // a "_dub" edition's own parts win over the parent's for the same slot (wide doors etc.)
    if parts.iter().any(|p| p.from_parent) {
        let own_slots: std::collections::HashSet<Slot> = parts
            .iter()
            .filter(|p| !p.from_parent && !p.is_base)
            .map(|p| classify_slot(&p.primary, &p.names))
            .filter(|s| !matches!(s, Slot::Unknown | Slot::CarSpace))
            .collect();
        let before = parts.len();
        parts.retain(|p| !(p.from_parent && own_slots.contains(&classify_slot(&p.primary, &p.names))));
        log::info!("car {}: dropped {} parent parts shadowed by the edition's own slots", name, before - parts.len());
    }
    // body volume from the car-space parts (shell/interior)
    let mut body_min = Vec3::splat(f32::INFINITY);
    let mut body_max = Vec3::splat(f32::NEG_INFINITY);
    for p in parts.iter().filter(|p| p.stock && (p.min + p.max).y * 0.5 > 0.12 && p.min.y > -0.2) {
        body_min = body_min.min(p.min);
        body_max = body_max.max(p.max);
    }
    if !body_min.is_finite() {
        body_min = Vec3::new(-0.9, 0.0, -2.3);
        body_max = Vec3::new(0.9, 1.5, 2.3);
    }
    if std::env::var("MCLA_PARTS_DUMP").is_ok() {
        if let Some(sk) = skeleton.as_ref() {
            for (i, b) in sk.bones.iter().enumerate() {
                let t = b.world.w_axis.truncate();
                if t.y > 0.55 && t.z < 0.3 && t.z > -1.4 {
                    log::info!("bone {} tag {} ({:#06x}) at {:.3} {:.3} {:.3}", i, b.tag, b.tag, t.x, t.y, t.z);
                }
            }
        }
    }
    // (exhaust tips are drawn at the rear end's outlet bones, wherever the car's skeleton has a
    // bumper bone or not - the DUB editions have none: `CarAsset::placed_parts_for`)
    let has_outlets = parts.iter().any(|p| !p.sockets.is_empty()) && std::env::var("MCLA_MOUNT_OLD").is_err();
    for part in parts.iter_mut() {
        let slot = classify_slot(&part.primary, &part.names);
        // is the part already in car space? (its box overlaps the body volume)
        let center = (part.min + part.max) * 0.5;
        let in_car_space = part.is_base || (center.y > 0.12 && part.min.y > -0.2);
        let mut t = Mat4::IDENTITY;
        let mut placed = in_car_space || slot == Slot::CarSpace;
        if !in_car_space {
            if let Some(sk) = skeleton.as_ref() {
                // exact: the slot's global bone id
                // (the add-on gauge - `post_gauge<n>`, a tachometer on a foot - stands on the bone
                // tagged 12783 on top of the dash, right of the dials: reference `g_pg_1`)
                let tags: Vec<u16> = if bike && part.primary.starts_with("armfl") {
                    // The front arm's root is at the origin in its own file; the
                    // bike's inclined steer bone gives the fork its height/rake.
                    vec![18265]
                } else if part.primary.to_ascii_lowercase().starts_with("post_gauge") {
                    // The Bel Air DLC skeleton omits the common gauge mount. Its
                    // authored dash-side bone sits just left of the steering wheel.
                    if name == "vpd_chv_belair_55" { vec![POST_GAUGE_TAG, 16970] }
                    else { vec![POST_GAUGE_TAG] }
                } else if slot == Slot::Spoiler {
                    spoiler_tags(&part.primary)
                } else if slot == Slot::Steer {
                    // Several cars use either of these steering-column tags instead
                    // of the common 18166 (including right-hand-drive Skylines).
                    vec![18166, 9610, 38417]
                } else {
                    slot_tag(slot).into_iter().collect()
                };
                if let Some(bi) = tags.iter().find_map(|&tag| sk.bone_by_tag(tag)).or_else(|| infer_slot_bone(slot, sk)) {
                    t = sk.bones[bi].world;
                    placed = true;
                } else {
                    let cands = slot_candidates(slot, sk);
                    let sill = hubs.map(|h| h.iter().map(|v| v.y).sum::<f32>() / 4.0 * 0.9).unwrap_or(body_min.y + 0.3);
                    if let Some(a) = best_anchor(slot, &cands, part.min, part.max, body_min, body_max, sill) {
                        t = Mat4::from_translation(a);
                        placed = true;
                    }
                }
            }
        }
        part.transform = t;
        part.placed = placed || (has_outlets && part.primary.to_ascii_lowercase().starts_with("exh_pipe"));
        log::debug!("part {:<24} slot {:?} car_space {} placed {} t {:?}", part.primary, slot, in_car_space, placed, t.w_axis.truncate());
    }
    let _ = assets;
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for p in parts.iter().filter(|p| p.stock) {
        min = min.min(p.min);
        max = max.max(p.max);
    }
    log::info!("car {}: {} shaders, {} textures, {} parts ({} stock)", name, shaders.len(), textures.len(), parts.len(), parts.iter().filter(|p| p.stock).count());
    Ok(CarAsset { name, shaders, shaders_parent, textures, parts, min, max, skeleton, hubs, bike, tune, simtune, fit, aitune, hull, windows, damage_layout })
}

/// Display name of a part from its internal names, e.g. "exstPipe05_Borla_single_round"
/// -> "BORLA SINGLE ROUND", "hd00_stk" -> "STOCK", "bumf02" -> "STYLE 2".
pub fn pretty_part(names: &[String], primary: &str) -> String {
    // catalogue codes: <slot code><2 digits>[_<brand and model>]
    const CODES: [&str; 18] = ["hd", "hdwide", "bumf", "bumr", "ss", "splr", "splrwide", "exstpipe", "exstpipes", "side_exst", "seat", "seat_pass", "str", "hl", "tl", "blwr", "widebody", "ic"];
    for n in names {
        if n == primary {
            continue;
        }
        let b = n.as_bytes();
        let Some(pos) = (0..b.len().saturating_sub(1)).find(|&i| b[i].is_ascii_digit() && b[i + 1].is_ascii_digit()) else { continue };
        let code = n[..pos].to_ascii_lowercase();
        if !CODES.contains(&code.as_str()) {
            continue;
        }
        let rest = &n[pos + 2..];
        if rest.is_empty() {
            let k: u32 = n[pos..pos + 2].parse().unwrap_or(0);
            return if k == 0 { "STOCK".to_string() } else { format!("STYLE {}", k) };
        }
        if let Some(r) = rest.strip_prefix('_') {
            let r = r.trim_end_matches("_shared");
            let rl = r.to_ascii_lowercase();
            if rl == "stk" || rl == "stck" {
                return "STOCK".to_string();
            }
            if rl == "dub" {
                return "DUB".to_string();
            }
            return r.replace('_', " ").to_uppercase();
        }
    }
    // "hood_wide0" -> "WIDE", "spoiler3" -> "STYLE 3"
    let digits: String = primary.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
    let k: u32 = digits.parse().unwrap_or(0);
    if primary.contains("wide") {
        "WIDE BODY".to_string()
    } else if k == 0 {
        "STOCK".to_string()
    } else {
        format!("STYLE {}", k)
    }
}

impl CarAsset {
    /// Display labels matching [`CarAsset::variants`].
    pub fn variant_labels(&self, slot: Slot) -> Vec<String> {
        self.variants(slot)
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if slot == Slot::Spoiler && i == 5 && v.is_empty() { return "NO SPOILER".to_string(); }
                if v.is_empty() { return "STOCK".to_string(); }
                let names = self.parts.iter().filter(|p| &p.primary == v).max_by_key(|p| p.tris).map(|p| p.names.clone()).unwrap_or_default();
                pretty_part(&names, v)
            })
            .collect()
    }

    /// Variant names indexed by the part number in the car's garage file. An empty
    /// entry is a stock part built into the base fragment, with no separate mesh.
    pub fn variants(&self, slot: Slot) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        // (the parts of the garage's other lists - side exhausts `exh_side<n>` beside the tips
        // `exh_pipe<n>` - are chosen by `CarConfig::extra`, never as a variant of the slot)
        let own_list = |p: &CarPart| extra_part(&p.primary).is_some() && std::env::var("MCLA_MOUNT_OLD").is_err();
        for p in self.parts.iter().filter(|p| p.placed && classify_slot(&p.primary, &p.names) == slot && !own_list(p)) {
            let name = p.primary.as_str();
            let number = match slot {
                Slot::Body if name.starts_with("fenders") || name.starts_with("pfenders") => Some(0),
                Slot::Body if name.starts_with("widebody") => Some(if p.stock { 0 } else { 1 }),
                Slot::Hood => name.strip_prefix("hood").and_then(|s| s.parse::<usize>().ok()),
                Slot::Spoiler => name.strip_prefix("spoiler").and_then(|s| s.parse::<usize>().ok())
                    .or_else(|| name.strip_prefix("spoiler_b").and_then(|s| s.parse::<usize>().ok())),
                Slot::BumperF => name.strip_prefix("bumper_f").and_then(|s| s.parse::<usize>().ok()),
                Slot::BumperR => name.strip_prefix("bumper_r").and_then(|s| s.parse::<usize>().ok()),
                Slot::Skirts => name.strip_prefix("skirts").and_then(|s| s.parse::<usize>().ok()),
                Slot::Exhaust => name.strip_prefix("exh_pipe").and_then(|s| s.parse::<usize>().ok()),
                Slot::Seats => name.strip_prefix("seats_f").and_then(|s| s.parse::<usize>().ok()),
                _ => None,
            };
            if let Some(i) = number.filter(|i| *i < 32) {
                if v.len() <= i { v.resize(i + 1, String::new()); }
                let old_stock = self.parts.iter().any(|q| q.primary == v[i] && q.stock);
                if v[i].is_empty() || (p.stock && !old_stock)
                    || (slot == Slot::Spoiler && v[i].starts_with("spoiler_b") && !name.starts_with("spoiler_b"))
                    || (p.stock == old_stock && name.ends_with("_stk") && !v[i].ends_with("_stk")) {
                    v[i] = name.to_string();
                }
            }
        }
        // The disc's zero-cost spoiler_5 row occurs on cars with a separate
        // spoiler0, but no spoiler5 fragment. It selects the spoiler-free shell.
        if slot == Slot::Spoiler && v.first().is_some_and(|n| n.starts_with("spoiler0"))
            && v.get(5).is_none() {
            v.resize(6, String::new());
        }
        v
    }

    /// The vehicle is a convertible whose roof moves: it has the skinned roof, `conv_top<n>`
    /// (eight on the disc: Vantage, Mustang '06, Gallardo, SL65, 350Z, Solstice, XLR-V,
    /// Eclipse '09; the Cobra and the Murcielagos have a raised roof part only).
    pub fn has_roof_animation(&self) -> bool {
        self.parts.iter().any(|p| roof_part(&p.primary) == Some(2))
    }

    /// Parts for a configuration: customisable slots pick their variant, everything else stock.
    pub fn parts_for(&self, cfg: &CarConfig) -> Vec<&CarPart> {
        let mut chosen: HashMap<String, &CarPart> = HashMap::new();
        let mut want: HashMap<Slot, String> = HashMap::new();
        let fitted_seat = self.variants(Slot::Seats).get(cfg.seats)
            .filter(|name| !name.is_empty()).map(|_| cfg.seats).unwrap_or(0);
        for slot in CUSTOM_SLOTS.iter().copied() {
            if matches!(slot, Slot::Paint | Slot::Wheels) {
                continue;
            }
            let vs = self.variants(slot);
            if !vs.is_empty() {
                want.insert(slot, vs.get(cfg.get(slot)).unwrap_or(&vs[0]).clone());
            }
        }
        let wide_body = want.get(&Slot::Body).is_some_and(|n| n.starts_with("widebody"));
        if wide_body {
            for (slot, prefix) in [(Slot::DoorL, "door_l"), (Slot::DoorR, "door_r")] {
                if let Some(p) = self.parts.iter().find(|p| p.placed && p.primary == format!("{prefix}_widebody"))
                    .or_else(|| self.parts.iter().find(|p| p.placed && p.primary == format!("{prefix}_wide0"))) {
                    want.insert(slot, p.primary.clone());
                }
            }
            for (slot, primary) in [(Slot::Hood, "hood_wide0"), (Slot::Spoiler, "spoiler_wide0")] {
                if cfg.get(slot) == 0 && self.parts.iter().any(|p| p.placed && p.primary == primary) {
                    want.insert(slot, primary.to_string());
                }
            }
        }
        // hinge slots that are not customisable (doors, trunk, steering wheel, door interiors)
        // still need exactly one part even when none of the variants carries a stock name
        for slot in [Slot::DoorL, Slot::DoorR, Slot::DoorLInt, Slot::DoorRInt, Slot::Trunk, Slot::Steer, Slot::Hood, Slot::Intercooler, Slot::Body] {
            if want.contains_key(&slot) {
                continue;
            }
            let has_stock = self.parts.iter().any(|p| p.placed && p.stock && classify_slot(&p.primary, &p.names) == slot);
            if !has_stock {
                let vs = self.variants(slot);
                if let Some(first) = vs.first() {
                    want.insert(slot, first.clone());
                }
            }
        }
        let roof_down = cfg.extra.get(ROOF_DOWN).copied().unwrap_or(0) != 0 && self.has_roof_animation();
        for p in self.parts.iter().filter(|p| p.placed && !p.primary.starts_with("occluder")) {
            let slot = classify_slot(&p.primary, &p.names);
            let passenger_seat = p.primary.strip_prefix("seats_p").and_then(|s| s.parse::<usize>().ok());
            // (the convertible roof at rest: raised -> conv_topup, lowered -> conv_topdown where the
            // model has one; the skinned roof is drawn only while it moves - sub_8233B5E0,
            // sub_8233B608, sub_8233EEF0. Only a vehicle with the roof's animation lowers it.)
            let take = if let Some(kind) = roof_part(&p.primary) {
                match kind {
                    0 => !roof_down,
                    1 => roof_down,
                    _ => false,
                }
            } else if let Some(index) = passenger_seat {
                index == fitted_seat
            } else { match (extra_part(&p.primary), want.get(&slot)) {
                // (headlights, taillights, steering wheels, blowers ...: the one of the number chosen)
                (Some((sec, n)), _) => cfg.extra.get(sec).copied().unwrap_or(0) == n,
                (None, Some(name)) => *name == p.primary,
                (None, None) => p.stock || always_on(&p.primary, cfg),
            }};
            if !take {
                continue;
            }
            let e = chosen.entry(p.primary.clone()).or_insert(p);
            if p.tris > e.tris {
                *e = p;
            }
        }
        let mut v: Vec<&CarPart> = chosen.into_values().collect();
        // exactly one base body: the LOD0 fragment (0xf9785b04) when present
        if v.iter().filter(|p| p.is_base).count() > 1 {
            let keep = v
                .iter()
                .filter(|p| p.is_base)
                .max_by_key(|p| (p.primary.contains("f9785b04"), p.tris))
                .map(|p| p.primary.clone());
            v.retain(|p| !p.is_base || Some(&p.primary) == keep.as_ref());
        }
        v.sort_by(|a, b| a.primary.cmp(&b.primary));
        v
    }

    /// The parts of a configuration with where each one is drawn. A part is drawn once at its
    /// own place, but for the exhaust tips (`exh_pipe<n>`: ONE tip, modelled at the origin with
    /// its outlet there and its body along +z): one at every outlet bone of the fitted rear end
    /// - `exh_primary0` / `exh_secondary0` in the skeleton of the rear bumper (or of the wide
    /// body, the fenders, the body itself), turned as the bone is (half a turn about the
    /// vertical on most cars: the tip's body runs forward under the bumper). The outlet bone
    /// stands in for the tip's own root bone (the Murcielago's tips have theirs turned half
    /// round: with both, its half-metre pipes would stand out of the back of the car).
    pub fn placed_parts_for(&self, cfg: &CarConfig) -> Vec<(&CarPart, Mat4)> {
        let parts = self.parts_for(cfg);
        let rank = |p: &CarPart| match p.primary.to_ascii_lowercase() {
            n if n.starts_with("widebody") => 0,
            n if n.starts_with("bumper_r") => 1,
            _ => 2,
        };
        let outlets: Vec<Mat4> = if std::env::var("MCLA_MOUNT_OLD").is_ok() {
            Vec::new()
        } else {
            parts.iter().filter(|p| !p.sockets.is_empty()).min_by_key(|p| rank(p)).map(|p| p.sockets.iter().map(|(_, m)| p.transform * *m).collect()).unwrap_or_default()
        };
        if log::log_enabled!(log::Level::Debug) {
            for p in parts.iter().filter(|p| !p.sockets.is_empty()) {
                log::debug!("exhaust outlets of {} (at {:?}): {:?}", p.primary, p.transform.w_axis.truncate(), p.sockets.iter().map(|(n, m)| (n.as_str(), m.w_axis.truncate(), m.z_axis.truncate())).collect::<Vec<_>>());
            }
        }
        let mut out = Vec::with_capacity(parts.len() + 2);
        for p in parts {
            if !outlets.is_empty() && p.primary.to_ascii_lowercase().starts_with("exh_pipe") {
                let root = p.mesh_bone.first().copied().unwrap_or(Mat4::IDENTITY).inverse();
                out.extend(outlets.iter().map(|m| (p, *m * root)));
            } else {
                out.push((p, p.transform));
            }
        }
        out
    }

    /// Stock parts, de-duplicated by primary name (highest triangle count wins = LOD0).
    pub fn stock_parts(&self) -> Vec<&CarPart> {
        let mut best: HashMap<&str, &CarPart> = HashMap::new();
        for p in self.parts.iter().filter(|p| p.stock && p.placed && !p.primary.starts_with("occluder")) {
            let e = best.entry(p.primary.as_str()).or_insert(p);
            if p.tris > e.tris {
                *e = p;
            }
        }
        let mut v: Vec<&CarPart> = best.into_values().collect();
        v.sort_by(|a, b| a.primary.cmp(&b.primary));
        v
    }
}

/// Parts that belong on every car although their names carry no stock marker: the raised
/// convertible roof, suspension arms / shocks / axles (the whole rear end of a bike), and
/// the lower valances that go with the stock bumpers.
fn always_on(primary: &str, cfg: &CarConfig) -> bool {
    let p = primary.to_ascii_lowercase();
    if p.starts_with("conv_topup") || p.starts_with("shockrr") || p.starts_with("shockfr") || p.starts_with("axle") {
        return true;
    }
    if p.starts_with("armrl") || p.starts_with("armrr") || p.starts_with("armfl") || p.starts_with("armfr") {
        return true;
    }
    // (the intercooler: fitted by its own switch, and the model is the front bumper's number -
    // the set-part mirrors the bumper's byte into the intercooler's, sub_82391E78; the body
    // draw shows slot 5 when UseIntercooler is set, sub_8235D500 at 0x8235DA14)
    if let Some(n) = p.strip_prefix("intercooler").and_then(|r| r.parse::<usize>().ok()) {
        return cfg.extra.get("intercooler").copied().unwrap_or(0) != 0 && n == cfg.get(Slot::BumperF);
    }
    if p.starts_with("subbumf") {
        return cfg.get(Slot::BumperF) == 0;
    }
    if p.starts_with("subbumr") {
        return cfg.get(Slot::BumperR) == 0;
    }
    false
}

/// Load a shared drawable (rims, tires, brakes, props) with its own shader group.
pub struct SimpleModel {
    pub shaders: Vec<Shader>,
    pub textures: HashMap<u32, Texture>,
    pub meshes: Vec<(Mesh, u16)>,
    pub min: Vec3,
    pub max: Vec3,
}

pub fn load_simple(path: &Path, shared_textures: Option<&Dictionary>) -> Result<SimpleModel> {
    let res = load_resource_file(path)?;
    let d = Drawable::parse(&res)?;
    let rd = Rd::new(&res);
    let mut textures = HashMap::new();
    for (h, o) in &d.texture_offsets {
        if let Ok(t) = Texture::parse(&rd, *o) {
            textures.insert(*h, t);
        }
    }
    let _ = shared_textures;
    let mut meshes = Vec::new();
    let mut mn = Vec3::splat(f32::INFINITY);
    let mut mx = Vec3::splat(f32::NEG_INFINITY);
    for m in d.models {
        for (mesh, si) in m.parts {
            let (a, b) = mesh.bbox();
            mn = mn.min(a);
            mx = mx.max(b);
            meshes.push((mesh, si));
        }
    }
    Ok(SimpleModel { shaders: d.shader_group.shaders, textures, meshes, min: mn, max: mx })
}
