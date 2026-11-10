//! Ambient traffic as the original drives it (`mcAmbientCar`, vtable 0x820c2b0c, with its
//! `mcAmbientBehaviorDrive2` 0x820d9dfc; per frame F-50624: the behaviour F-87488,
//! the acceleration F-30960, the speed F-58614). A car rides its lane kinematically
//! (the rail follower advances by speed x dt, F-50894) with no pedals or steering. Met by
//! something it turns physical (state 4) and rolls to rest (state 5); in free roam it then
//! finds its way back onto a lane on a recovery rail (state 6), goes round a car standing in
//! its way (state 7) and backs off one it is stuck behind (state 8). A semi cab tows a trailer
//! that trails its hitch. Rules decoded 2026-10-19 / 20: docs/PEOPLE_TRAFFIC.md "Ambient
//! traffic driving".

use super::Vehicles;
use crate::physics::collision::WorldCollider;
use glam::{Quat, Vec2, Vec3};
use crate::tcp_rail::TcpRail;
use mcla_formats::rage::navsys::{LaneCurve, NavNode, NavSys, RegionRect, DEFAULT_ROAD_SPEED};

const MPH: f32 = 0.44704;
/// The slowest a car is ever asked to drive (F-70324: 10 mph).
const MIN_WANT: f32 = 4.4704;
/// A freeway road's speed (F-31543 and the region list: road speed >= the image's word 0x41E81A69, 29.012896 m/s =
/// 65 mph; the literal 29.0129 rounds to the next float but one)
pub(super) const FREEWAY_SPEED: f32 = f32::from_bits(0x41E8_1A69);
/// Occluders are taken within 400 m of the focus, as the pedestrian spawner's (peds.rs
/// `OCC_RANGE_SCALE`).
pub(super) const OCC_RANGE: f32 = crate::occluder::RANGE * 10.0;
/// The recovery and rejoin rails' largest single turn (140 degrees, the image's 2.44346 word) and the passing rail's (130 degrees).
const RAIL_TURN: f32 = f32::from_bits(0x401C_61AA);
const PASS_TURN: f32 = f32::from_bits(0x4011_361E);
/// The semi trailers (0x8282BDDC); a cab draws one of them.
pub const TRAILERS: [&str; 5] = ["va_semitruck_00_trailer", "va_semitruck_00_tanker", "va_semitruck_00_flatbed", "va_semitruck_00_flatloaded", "va_semitruck_dub1_00_trailer"];
/// F-45988's picks on the cab's MWC (+384, right after its hitch draw): (lo & 0x7FFFFFFF) mod 5 into the table of the
/// five names (0x8282BDDC, `TRAILERS`' order), the type looked up by name (F-54839; none: nothing more), then mod 20 the
/// shade.
pub(crate) fn trailer_pick(mwc: &mut super::parked::Mwc, has: impl FnOnce(&str) -> bool) -> Option<(&'static str, usize)> {
    let name = TRAILERS[mwc.index(5) as usize];
    if !has(name) {
        return None;
    }
    Some((name, mwc.index(20) as usize))
}

/// The "hitch" bones (F-19769): the cab's and every trailer's; the model's front is -z.
const CAB_HITCH: Vec3 = Vec3::new(0.0, 1.132, 1.289);
const TRAILER_HITCH: Vec3 = Vec3::new(0.0, 1.174, -5.219);
/// The trailer's rear bogie (between its axles at 3.914 and 5.219), which trails the hitch.
const TRAILER_BOGIE: f32 = 4.57;
pub const CAB_HITCH_PUB: Vec3 = CAB_HITCH;
pub const TRAILER_HITCH_PUB: Vec3 = TRAILER_HITCH;

/// A traffic type's driving values (`mcAmbientType.xml`; type +124 .. +164) and its bound.
#[derive(Clone, Copy, Debug)]
pub struct AmbType {
    pub max_speed: f32,
    pub min_accel_band: f32,
    pub min_accel: f32,
    pub max_accel: f32,
    pub min_decel_band: f32,
    pub min_decel: f32,
    pub max_decel: f32,
    pub min_turn: f32,
    pub pivot_turn: f32,
    /// the type's pool (`iInstanceCount`, shared with the parked cars), its bound group
    /// (`iBoundGroup`, 2: buses, semis, limo, garbage truck) and `bFreewayOnly`
    pub instances: u32,
    pub bound_group: u32,
    pub freeway_only: bool,
    pub cab: bool,
    pub trailer: bool,
    /// `bIsTaxi` (type +113: never parked, F-74446)
    pub taxi: bool,
    /// `bIsProxy` (type +108): the start speed rule never removes such a car (F-30683)
    pub proxy: bool,
    /// the bound's full width and length (the `.r155` root +8 bound, max +0x10 - min +0x20:
    /// obstacle vfuncs 4 and 6, F-46284); 0: not read
    pub width: f32,
    pub length: f32,
    /// the same bound's height (max.y - min.y) and its word +0x0C: the type's +100 is that bound (F-58444 stores it and
    /// hands it to the physics archetype +104 as its bound +12); the traffic room test F-43944 adds +0x0C to a racer's
    /// extent. 0: not read
    pub height: f32,
    pub bound_r: f32,
    /// `mcAmbientCarTypeColor.xml` (render type +236): the 20 colours times `fScale`, and
    /// `iPaintType` (the CarPaint record: 0 DirtyMatte, 1 Matte, 2 Gloss, 3 Metallic, 4 Pearl,
    /// 5 Sparkle)
    pub colors: Option<[[f32; 3]; 20]>,
    pub paint_type: u8,
}

impl Default for AmbType {
    fn default() -> Self {
        // (the values every sedan of the set carries)
        AmbType { max_speed: 40.0, min_accel_band: 1.0, min_accel: 1.0, max_accel: 5.0, min_decel_band: 0.2, min_decel: 3.0, max_decel: 22.5, min_turn: 5.0, pivot_turn: 3.0, instances: 20, bound_group: 0, freeway_only: false, cab: false, trailer: false, taxi: false, proxy: false, width: 0.0, length: 0.0, height: 0.0, bound_r: 0.0, colors: None, paint_type: 2 }
    }
}

impl AmbType {
    /// The type's tune and bound (`read`), read once per data folder and type and kept: read for every type on a
    /// thread while the game loads (`ahead`), not in the first spawner frame (58 ms of it, docs/PERFORMANCE.md).
    pub fn load(root: &std::path::Path, model: &str) -> AmbType {
        type Cache = std::sync::Mutex<hashbrown::HashMap<(std::path::PathBuf, String), AmbType>>;
        static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let key = (root.to_path_buf(), model.to_ascii_lowercase());
        if let Some(t) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return *t;
        }
        let t = Self::read(root, model);
        cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, t);
        t
    }

    fn read(root: &std::path::Path, model: &str) -> AmbType {
        let _ = DATA_ROOT.set(root.to_path_buf());
        crate::cruisedrive::prewarm();
        let d = AmbType::default();
        let (width, length) = bound_size(root, model).unwrap_or((0.0, 0.0));
        let (height, bound_r) = bound_extra(root, model).unwrap_or((0.0, 0.0));
        let (colors, paint_type) = type_colors(root, model);
        let Ok(t) = std::fs::read_to_string(root.join("tune/ambients/traffic").join(model).join("mcAmbientType.xml")) else { return AmbType { width, length, height, bound_r, colors, paint_type, ..d } };
        let v = |tag: &str, def: f32| crate::ambient::xml_value(&t, tag).unwrap_or(def);
        let flag = |tag: &str| t.contains(&format!("<{tag} value=\"true\""));
        AmbType {
            max_speed: v("fMaxSpeed", d.max_speed),
            min_accel_band: v("fMinAccelBand", d.min_accel_band),
            min_accel: v("fMinAccel", d.min_accel),
            max_accel: v("fMaxAccel", d.max_accel),
            min_decel_band: v("fMinDecelBand", d.min_decel_band),
            min_decel: v("fMinDecel", d.min_decel),
            max_decel: v("fMaxDecel", d.max_decel),
            min_turn: v("fMinTurningRadius", d.min_turn),
            pivot_turn: v("fPivotTurningRadius", d.pivot_turn),
            instances: v("iInstanceCount", 20.0) as u32,
            bound_group: v("iBoundGroup", 0.0) as u32,
            freeway_only: flag("bFreewayOnly"),
            cab: flag("bIsCab"),
            trailer: flag("bIsTrailer") || TRAILERS.iter().any(|n| n.eq_ignore_ascii_case(model)),
            taxi: flag("bIsTaxi"),
            proxy: flag("bIsProxy"),
            width,
            length,
            height,
            bound_r,
            colors,
            paint_type,
        }
    }

    /// The colour of colour index `i` (the spawner draws `rand % 20`, F-92144; the
    /// render instance's +52): `vColors[i] x fScale`, as the player's paint is handed on.
    pub fn color(&self, i: usize) -> Option<[f32; 4]> {
        self.colors.map(|c| {
            let c = c[i % 20];
            [c[0], c[1], c[2], 1.0]
        })
    }
}

/// The data folder, known once a traffic type was loaded (the parked cars' colours).
pub static DATA_ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// `tune/ambients/traffic/<model>/mcAmbientCarTypeColor.xml`: (20 colours x fScale, iPaintType).
fn type_colors(root: &std::path::Path, model: &str) -> (Option<[[f32; 3]; 20]>, u8) {
    let Ok(t) = std::fs::read_to_string(root.join("tune/ambients/traffic").join(model).join("mcAmbientCarTypeColor.xml")) else { return (None, 2) };
    let scale = crate::ambient::xml_value(&t, "fScale").unwrap_or(1.0);
    let paint = crate::ambient::xml_value(&t, "iPaintType").map_or(2, |v| v as u8);
    let body = t.find("<vColors").and_then(|i| {
        let b = &t[i..];
        Some(&b[b.find('>')? + 1..b.find("</vColors>")?])
    });
    let v: Vec<f32> = body.map(|b| b.split_whitespace().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
    if v.len() < 60 {
        return (None, paint);
    }
    let mut c = [[0.0f32; 3]; 20];
    for (i, row) in c.iter_mut().enumerate() {
        *row = [v[i * 3] * scale, v[i * 3 + 1] * scale, v[i * 3 + 2] * scale];
    }
    (Some(c), paint)
}

/// The type's bound box: `resources/ambients/<model>/*.r155`, the root's +8 bound (a
/// polyhedron, or a composite for the trailers): full width (x) and length (z).
pub fn bound_size(root: &std::path::Path, model: &str) -> Option<(f32, f32)> {
    for e in std::fs::read_dir(root.join("resources/ambients").join(model)).ok()? {
        let p = e.ok()?.path();
        if p.extension().is_some_and(|x| x == "r155") {
            let res = crate::assets::load_resource_file(&p).ok()?;
            let rd = mcla_formats::rage::Rd::new(&res);
            let o = rd.ptr(8)?;
            let (w, l) = (rd.f32(o + 0x10) - rd.f32(o + 0x20), rd.f32(o + 0x18) - rd.f32(o + 0x28));
            return (w.is_finite() && l.is_finite() && w > 0.5 && l > 1.0).then_some((w, l));
        }
    }
    None
}

/// The type's bound (the `.r155` root's +8, as `bound_size`): its height (max +0x14 - min +0x24) and its word +0x0C.
pub fn bound_extra(root: &std::path::Path, model: &str) -> Option<(f32, f32)> {
    for e in std::fs::read_dir(root.join("resources/ambients").join(model)).ok()? {
        let p = e.ok()?.path();
        if p.extension().is_some_and(|x| x == "r155") {
            let res = crate::assets::load_resource_file(&p).ok()?;
            let rd = mcla_formats::rage::Rd::new(&res);
            let o = rd.ptr(8)?;
            return Some((rd.f32(o + 0x14) - rd.f32(o + 0x24), rd.f32(o + 0x0C)));
        }
    }
    None
}

/// The type's bound box (min, max) in the car's space: the `.r155` root's +8 bound, +0x10 max / +0x20 min. The traffic
/// occlusion blob's record (F-68081) takes its L and H from this box (z and y extents of max - min), read there through
/// the type's physics archetype; the drawn model's box is larger (the caravan's height 1.6775 against 1.4930).
pub fn bound_box(root: &std::path::Path, model: &str) -> Option<(glam::Vec3, glam::Vec3)> {
    for e in std::fs::read_dir(root.join("resources/ambients").join(model)).ok()? {
        let p = e.ok()?.path();
        if p.extension().is_some_and(|x| x == "r155") {
            let res = crate::assets::load_resource_file(&p).ok()?;
            let rd = mcla_formats::rage::Rd::new(&res);
            let o = rd.ptr(8)?;
            let v = |a: usize| glam::Vec3::new(rd.f32(a), rd.f32(a + 4), rd.f32(a + 8));
            let (mx, mn) = (v(o + 0x10), v(o + 0x20));
            return (mx.is_finite() && mn.is_finite() && (mx - mn).min_element() > 0.0).then_some((mn, mx));
        }
    }
    None
}

/// A traffic type's two LOD steps, `tune/ambients/traffic/<model>/mcAmbientRenderType.xml` (the render type's
/// parser table 0x82830414, filled by F-10142: fLodSwitchScale at +40, fHighToMedSwitch at +44,
/// fMedToLowSwitch at +48): LOD 0 below +44 x +40, LOD 1 below +48 x +40 (F-69447). Every type has the file;
/// the steps differ by type (40 / 155 for most cars, 40 / 250 for buses and service trucks, 180 / 250 for the
/// flatbed semis, 50 / 155 and 40 / 200 for one type each).
pub fn lod_steps(root: &std::path::Path, model: &str) -> Option<[f32; 2]> {
    let t = std::fs::read_to_string(root.join("tune/ambients/traffic").join(model).join("mcAmbientRenderType.xml")).ok()?;
    let v = |tag: &str| crate::ambient::xml_value(&t, tag);
    let scale = v("fLodSwitchScale")?;
    Some([v("fHighToMedSwitch")? * scale, v("fMedToLowSwitch")? * scale])
}

/// The centre of a traffic model's bound (the root bound's centroid, +0x30), in the car's space: the middle of the
/// sphere the LOD choice tests (F-41581: the car's matrix times bound +48).
pub fn bound_centre(root: &std::path::Path, model: &str) -> Option<glam::Vec3> {
    for e in std::fs::read_dir(root.join("resources/ambients").join(model)).ok()? {
        let p = e.ok()?.path();
        if p.extension().is_some_and(|x| x == "r155") {
            let res = crate::assets::load_resource_file(&p).ok()?;
            let rd = mcla_formats::rage::Rd::new(&res);
            let o = rd.ptr(8)?;
            let c = glam::Vec3::new(rd.f32(o + 0x30), rd.f32(o + 0x34), rd.f32(o + 0x38));
            return c.is_finite().then_some(c);
        }
    }
    None
}

/// The drawn wheels turn only within 75 m of the focus, horizontally (F-37033: d^2 < 5625).
pub const SPIN_REACH2: f32 = 5625.0;

/// A drawn wheel's angle kept in [0, 2 pi] (F-37033: one 2 pi off above, one on below 0).
pub fn wrap_spin(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    if a > tau {
        a - tau
    } else if a < 0.0 {
        a + tau
    } else {
        a
    }
}

/// A parked car's drawn wheel turn (F-37033): 0.2 of a riding car's - it has no behaviour
/// (car +452 = 0) and is no trailer.
pub const PARKED_SPIN: f32 = 0.2;

/// A parked car's front wheels' turn (F-77422 with a spot: car +3584 = a Gaussian, mean 0, sd
/// pi / 8 (F-66400), clamped to +-0.706858; drawn added to the front wheels' steer while the
/// car has no behaviour and is no trailer, F-37033). `u1` in (0, 1], `u2` in [0, 1).
pub fn parked_steer(u1: f32, u2: f32) -> f32 {
    let g = (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
    (g * std::f32::consts::FRAC_PI_8).clamp(-0.706858, 0.706858)
}

/// The drawn steer's ring (F-17429, the car's draw object +88: count, next, sum, 8 values). The draw objects are made
/// once with the pool's render instances (F-62509), and a car taking one empties it (count and next 0, F-46127 from the
/// activate hook F-25927; vehicles.rs `amb_pool_step`): its first sample starts the ring.
#[derive(Clone, Copy, Default, Debug)]
pub struct SteerRing {
    n: i32,
    next: i32,
    sum: f32,
    v: [f32; 8],
}

impl SteerRing {
    pub fn push(&mut self, x: f32) {
        if self.n == 0 {
            (self.sum, self.v[0], self.next, self.n) = (x, x, 1, 1);
        } else if self.n < 8 {
            self.v[self.next as usize] = x;
            self.sum += x;
            self.n += 1;
            self.next += 1;
        } else {
            if self.next == 8 {
                self.next = 0;
            }
            let k = self.next as usize;
            self.sum -= self.v[k];
            self.v[k] = x;
            self.sum += x;
            self.next += 1;
        }
    }

    /// The drawn steer (F-37033): sum / count, 0 while empty.
    pub fn mean(&self) -> f32 {
        if self.n > 0 { self.sum / self.n as f32 } else { 0.0 }
    }
}

/// Where on the lane the drawn steer looks (F-37033): the phase 0.4 x |wheel_0's hub z| on at the curve speed (rail
/// slot 6, F-77217), at most 1.
pub fn steer_look(t: f32, ds: f32, hub_z: f32) -> f32 {
    steer_clamp(mcla_formats::rage::navsys::LaneCurve::advance(t, hub_z.abs() * 0.4, ds))
}

fn steer_clamp(p: f32) -> f32 {
    let p = if 1.0 - p >= 0.0 { p } else { 1.0 };
    if p - 1.0 >= 0.0 { 1.0 } else { p }
}

/// The drawn steer's sample (F-37033) from the car's forward and the lane's derivative there (rail slot 10): the
/// derivative with y = 0, normalised as the code does (1 / sqrt, one Newton step; a zero length gives 0), then
/// -1.25 x (F.x N.z - F.z N.x). 0 when the flattened derivative is zero.
pub fn steer_of(fwd: Vec3, d: Vec3) -> f32 {
    if d.x == 0.0 && d.z == 0.0 {
        return 0.0;
    }
    // (the dot product as the vector unit adds it, z^2 + (y^2 + x^2) with y = 0; under the smallest normal it is flushed)
    let dd = d.z * d.z + d.x * d.x;
    let dd = if dd < f32::MIN_POSITIVE { 0.0 } else { dd };
    let mut y = 1.0 / dd.sqrt();
    if y == f32::INFINITY {
        y = 0.0;
    }
    let k = -(dd * (y * y) - 1.0) * (0.5 * y) + y;
    (fwd.x * (d.z * k) - fwd.z * (d.x * k)) * -1.25
}

/// A traffic car's instance matrix (inst +16): its rotation, and the seat's place - the pose before
/// the drawn 0.025 m drop - while it rides; rows X, up, Z, place.
pub fn instance_matrix(c: &super::CarInstance) -> [Vec3; 4] {
    let m = glam::Mat3::from_quat(c.state.rot);
    let at = c.rail.as_ref().filter(|r| seat_poses(r)).and_then(|r| r.seat.as_ref()).map_or(c.state.pos, |s| s.at.0);
    [m.x_axis, m.y_axis, m.z_axis, at]
}

/// Whether the rail's seat places the instance: only while a rail poses the car. Physical (state 4) or at
/// rest (5) the instance is where the collider left it - the car's place, which the body's pose kept (the
/// original's recording: no instance moves when its body goes, th.0 / th7.0, 10 of 10 at 0.000 - 0.001 m). A
/// seat from before the hit would move it there in one frame, and the next activation takes that jump over
/// the frame as its velocity (1.4 m: 78 m/s).
fn seat_poses(r: &Rider) -> bool {
    !r.active && !matches!(r.state, 4 | 5)
}

/// An ambient type's physics (loose.rs `AmbTune`): `mcAmbientTypePhysicsTuning.xml`,
/// `mcAmbientWheelTuning.xml`, `iWheelCount` / `bIsTrailer` of `mcAmbientType.xml`, the `.r155`
/// root bound, the model's wheel bones (`hubs`). Read once a type.
pub fn amb_tune(root: &std::path::Path, model: &str, hubs: Option<[Option<Vec3>; 6]>, ty: &AmbType) -> Option<std::sync::Arc<crate::physics::loose::AmbTune>> {
    use crate::physics::loose::AmbTune;
    use std::sync::{Arc, Mutex};
    static KEPT: std::sync::OnceLock<Mutex<hashbrown::HashMap<String, Option<Arc<AmbTune>>>>> = std::sync::OnceLock::new();
    let mut kept = KEPT.get_or_init(|| Mutex::new(hashbrown::HashMap::new())).lock().unwrap();
    if let Some(t) = kept.get(model) {
        return t.clone();
    }
    let made = (|| {
        let dir = root.join("tune/ambients/traffic").join(model);
        let phys = std::fs::read_to_string(dir.join("mcAmbientTypePhysicsTuning.xml")).ok()?;
        let wheel = std::fs::read_to_string(dir.join("mcAmbientWheelTuning.xml")).ok()?;
        let ty_xml = std::fs::read_to_string(dir.join("mcAmbientType.xml")).unwrap_or_default();
        let v = |t: &str, tag: &str| crate::ambient::xml_value(t, tag);
        let attr = |t: &str, tag: &str, a: &str| -> Option<f32> {
            let i = t.find(&format!("<{tag} "))?;
            let rest = &t[i..];
            let j = rest.find(&format!("{a}=\""))? + a.len() + 2;
            rest[j..].split('"').next()?.parse().ok()
        };
        let mass = v(&phys, "fMass")? * v(&phys, "fMassScale").unwrap_or(1.0);
        let scale = v(&phys, "fInertialBoxScale").unwrap_or(1.0);
        let bx = Vec3::new(attr(&phys, "vInertialBox", "x")?, attr(&phys, "vInertialBox", "y")?, attr(&phys, "vInertialBox", "z")?) * scale;
        let k = mass / 12.0;
        let mut inertia = Vec3::new(k * (bx.y * bx.y + bx.z * bx.z), k * (bx.x * bx.x + bx.z * bx.z), k * (bx.x * bx.x + bx.y * bx.y));
        let top = inertia.max_element();
        for i in 0..3 {
            if inertia[i] < top / 20.0 {
                inertia[i] = top * 0.05;
            }
        }
        let gravity = v(&phys, "fGravityFactor").unwrap_or(1.0);
        let (cog_sim, cog_off) = (v(&wheel, "fCogSim")?, v(&wheel, "fCogOffset")?);
        let (grip, ts, sd, travel, tt, td, st) = (v(&wheel, "fTireGrip")?, v(&wheel, "fSuspensionTension")?, v(&wheel, "fSuspensionDamping")?, v(&wheel, "fSuspensionTravel")?, v(&wheel, "fTireTension")?, v(&wheel, "fTireDamping")?, v(&wheel, "fTireStretch")?);
        // (F-82909)
        let wq = mass * gravity * 9.8 / 4.0;
        let ks = ts * wq / travel;
        let kt = 2.0 * wq * tt / st;
        let (cs, ct) = (2.0 * sd * (wq * ks).sqrt(), 2.0 * td * (wq * kt).sqrt());
        // (the bound)
        let file = std::fs::read_dir(root.join("resources/ambients").join(model)).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "r155"))?;
        let res = crate::assets::load_resource_file(&file).ok()?;
        let bound = mcla_formats::rage::bound::ambient_bound(&res)?;
        let shape = |h: &mcla_formats::rage::bound::Hull| -> &'static crate::physics::hull::HullShape {
            let mut sh = crate::physics::hull::HullShape::new(h.verts.iter().map(|v| Vec3::from(*v)).collect(), &h.tris);
            sh.mats = h.material.clone();
            sh.cg = Vec3::from(h.cg);
            sh.scan = (h.scan as usize).min(sh.verts.len());
            Box::leak(Box::new(sh))
        };
        let ident = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]];
        if bound.parts.iter().any(|p| p.1 != ident) {
            log::warn!("traffic {model}: a part matrix is not the identity - its bound is taken as it stands");
        }
        let cy = bound.centroid[1];
        // (the wheels: iWheelCount of the wheel bones; a trailer's from the third on are its first
        // ones with z turned - its support near the hitch)
        let count = v(&ty_xml, "iWheelCount").unwrap_or(4.0) as usize;
        let hubs = hubs?;
        let mut wheels = Vec::new();
        for i in 0..count.min(6) {
            let w = if ty.trailer && i >= 2 { hubs[i - 2].map(|h| Vec3::new(h.x, h.y, -h.z)) } else { hubs[i] };
            wheels.push(w?);
        }
        Some(Arc::new(AmbTune {
            shape: shape(&bound.parts[0].0),
            shape2: bound.parts.get(1).map(|p| shape(&p.0)),
            mass,
            inertia,
            gravity,
            cg: Vec3::new(0.0, cog_off * cy, 0.0),
            cog_sim: cog_sim * cy,
            k: ks,
            c: cs,
            kt,
            ct,
            travel,
            stretch: st,
            grip,
            wheels,
            hitch: if ty.cab { Some(CAB_HITCH) } else if ty.trailer { Some(TRAILER_HITCH) } else { None },
        }))
    })();
    if made.is_none() {
        log::warn!("traffic {model}: no ambient physics (tune, bound or wheel bones missing)");
    } else if let Some(t) = made.as_ref() {
        log::info!("traffic {model}: mass {} inertia {:?} gravity {} cg {:?} k {:.1} c {:.1} kt {:.1} ct {:.2} wheels {:?} parts {}", t.mass, t.inertia, t.gravity, t.cg, t.k, t.c, t.kt, t.ct, t.wheels, 1 + t.shape2.is_some() as u8);
    }
    kept.insert(model.to_string(), made.clone());
    made
}

/// The lanes as rails: each lane's links in order and its length.
pub struct Lanes {
    pub links: Vec<Vec<usize>>,
    pub len: Vec<f32>,
}

impl Lanes {
    fn build(nav: &NavSys) -> Lanes {
        let n = nav.links.iter().map(|l| l.lane + 1).max().unwrap_or(0);
        let mut of: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, l) in nav.links.iter().enumerate() {
            of[l.lane].push(i);
        }
        let mut links = Vec::with_capacity(n);
        let mut len = Vec::with_capacity(n);
        for group in of {
            // the lane's first link: none of its own lane leads to it
            let start = group.iter().copied().find(|&k| !group.iter().any(|&j| j != k && nav.links[j].next.contains(&k))).or(group.first().copied());
            let mut order = Vec::with_capacity(group.len());
            if let Some(mut k) = start {
                loop {
                    order.push(k);
                    match nav.links[k].next.iter().copied().find(|&x| nav.links[x].lane == nav.links[k].lane && !order.contains(&x)) {
                        Some(x) => k = x,
                        None => break,
                    }
                }
            }
            len.push(order.iter().map(|&k| nav.links[k].len).sum());
            links.push(order);
        }
        Lanes { links, len }
    }

    fn project(&self, nav: &NavSys, lane: usize, p: Vec3) -> f32 {
        let (mut best, mut s0, mut at) = (f32::MAX, 0.0, 0.0);
        for &k in &self.links[lane] {
            let l = &nav.links[k];
            let t = l.project(p);
            let d = l.point_at(t).distance_squared(p);
            if d < best {
                best = d;
                at = s0 + t * l.len;
            }
            s0 += l.len;
        }
        at
    }
}

/// The road network as the traffic rides it, made once: the lanes' curves (the original's
/// lanes are B-splines - `navsys::LaneCurve` - and its rail follower takes the car's point and
/// heading off the curve at its phase, F-74567 -> lane vfunc 9 F-42884 ->
/// F-87587), cut wherever the tangent has turned 2 degrees, with the curve's tangents.
/// Traffic keeps to this graph (its own link indices; lanes and successors as the shared one).
pub fn rail_nav(nav: &NavSys) -> &'static NavSys {
    RAIL_NAV.get_or_init(|| {
        let t = std::time::Instant::now();
        let cv = nav.curved(2f32.to_radians());
        log::info!("traffic: lanes as curves, {} links (control polygon {}), {:.2} s", cv.links.len(), nav.links.len(), t.elapsed().as_secs_f32());
        cv
    })
}

static RAIL_NAV: std::sync::OnceLock<NavSys> = std::sync::OnceLock::new();

/// The traffic's road network once it is made (`rail_nav`): what the riders' links index.
pub fn rail_nav_made() -> Option<&'static NavSys> {
    RAIL_NAV.get()
}

/// What the traffic's first spawner frames would make (the lanes as rails and their ends, the spawner's roads and junctions,
/// every traffic type's tune and bound, the riders' route net: 18 + 20 + 58 + 20 ms of the first ticks, docs/PERFORMANCE.md 2026-11-10), made on a
/// thread once the lane curves are: the same tables, only earlier.
pub fn ahead(nav: &'static NavSys, root: &std::path::Path) {
    let root = root.to_path_buf();
    let _ = std::thread::Builder::new().name("traffic tables".into()).spawn(move || {
        let _ = lanes(nav);
        let _ = lane_ends(nav);
        let sp = Spawner::new(nav, &root);
        *SPAWNER_AHEAD.lock().unwrap_or_else(|e| e.into_inner()) = Some((root.clone(), sp));
        let mut names: Vec<String> = std::fs::read_dir(root.join("tune/ambients/traffic")).map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default();
        names.sort();
        for n in names {
            let _ = AmbType::load(&root, &n);
        }
        // (the route net the riders' next lane is chosen on: read once the data folder is known, the first type's load)
        let _ = ghost_net();
    });
}

/// The spawner made by `ahead` (with its data folder), taken by the first `spawn_traffic`.
static SPAWNER_AHEAD: std::sync::Mutex<Option<(std::path::PathBuf, Spawner)>> = std::sync::Mutex::new(None);

/// The lanes of the city, made once.
pub fn lanes(nav: &NavSys) -> &'static Lanes {
    static L: std::sync::OnceLock<Lanes> = std::sync::OnceLock::new();
    L.get_or_init(|| Lanes::build(nav))
}

/// A rail off the lanes (recovery, passing, rejoin, backing up, a race path's join rail): one of the
/// original's dynamic rail classes, ridden by phase as the follower rides them (F-50894: slot 6 with the step
/// speed x dt, negated on a reverse rail), posed by the class's point and derivative (slot 9).
#[derive(Clone, Debug)]
pub enum Geom {
    /// mcRailAmbientTCP, the arc-line-arc rail (`tcp_rail.rs`): slot 6 F-70353 (the curve speed not used), slot 9
    /// F-31489 (its lateral output 0 goes to the follower's curve speed +180), slot 10 F-28568
    Tcp(TcpRail),
    /// mcRailAmbientDynamic, a B-spline with the lane curve's functions: slot 6 F-77217 (`LaneCurve::advance`),
    /// slot 9 F-42884, slot 10 F-26650 (`LaneCurve::eval(p).1`)
    Curve(LaneCurve),
}

#[derive(Clone, Debug)]
pub struct Path {
    pub geom: Geom,
    /// the rail's speed (slot 17, rail +304: set by the pool's slot 23 when the builder takes the rail)
    pub speed: f32,
    /// the lane and phase the car goes on along at the rail's end (the attach F-70405's holder +12 / +24)
    end: Option<(usize, f32)>,
    /// ridden backwards (the attach's reverse bit: the step negated; the car faces against the rail)
    reverse: bool,
    /// the follower's phase (+176) and curve speed (+180; 0: taken anew from the curve at the phase)
    pub t: f32,
    ds: f32,
    /// the attach's owner (holder +16, F-70405's r9): the road whose +160 counts the rail while it is
    /// attached - the passing rail's and the lane change rail's (the road of the lane the car was on, F-86691 cast to a
    /// road); none for the recovery, rejoin and backing rails
    pub owner: Option<usize>,
}

impl Path {
    fn tcp(rail: TcpRail, speed: f32, end: Option<(usize, f32)>) -> Path {
        Path { geom: Geom::Tcp(rail), speed, end, reverse: false, t: 0.0, ds: 0.0, owner: None }
    }

    /// The rail's length (slot 20: F-62306 for the arc-line-arc rail; the curve's chord length otherwise).
    pub fn len(&self) -> f32 {
        match &self.geom {
            Geom::Tcp(q) => q.len(),
            Geom::Curve(c) => c.length(),
        }
    }

    /// What is left of it: len - len x phase (the recovery's replace test F-99509, fused).
    fn left(&self) -> f32 {
        crate::physics::exact::fma(-self.len(), self.t, self.len())
    }

    /// The point and the derivative at phase `t` (a curve's slot 8: F-71868, `eval_pair` - dP/du).
    fn eval(&self, t: f32) -> (Vec3, Vec3) {
        match &self.geom {
            Geom::Tcp(q) => (q.point(t), q.tangent(t)),
            Geom::Curve(c) => c.eval_pair(t),
        }
    }

    /// The point at phase `t` and the unit direction along the rail there.
    fn at(&self, t: f32) -> (Vec3, Vec3) {
        let (p, d) = self.eval(t);
        (p, d.normalize_or(Vec3::NEG_Z))
    }

    /// Slot 6: the phase `d` metres on from `t` at the stored curve speed +180 (set at the attach, F-48384).
    fn advance(&self, t: f32, d: f32) -> f32 {
        match &self.geom {
            Geom::Tcp(q) => q.advance(t, d),
            Geom::Curve(_) => LaneCurve::advance(t, d, self.ds),
        }
    }

    /// The pose the follower stores (F-74567 -> slot 9): the point and the unit derivative; the curve speed +180 is
    /// the arc-line-arc rail's lateral output (0), or on a curve the mean of |dP/dt| here and the last value.
    fn pose(&mut self) -> (Vec3, Vec3) {
        // (a curve: slot 9, F-87587 - the point, dP/du and the speed averaged with +180)
        let (p, d) = match &self.geom {
            Geom::Tcp(_) => {
                self.ds = 0.0;
                self.eval(self.t)
            }
            Geom::Curve(c) => {
                let (p, d, s) = c.pose_slot9(self.t, self.ds);
                self.ds = s;
                (p, d)
            }
        };
        (p, d.normalize_or(Vec3::NEG_Z))
    }

    /// The follower's step (F-50894): the phase `d` metres on (back on a reverse rail); past 1 the metres left over,
    /// (phase - 1) x d / max(phase - old, 1e-6).
    fn step(&mut self, d: f32) -> Option<f32> {
        if d > 0.0 {
            let t0 = self.t;
            self.t = self.advance(t0, if self.reverse { -d } else { d });
            if self.t > 1.0 {
                let moved = self.t - t0;
                let k = if moved - 1e-6 >= 0.0 { moved } else { 1e-6 };
                return Some(((self.t - 1.0) * d) / k);
            }
        }
        None
    }

    /// The drawn steer's look (F-37033): slot 6 from the phase 0.4 x |hub z| on at the curve speed (+180), clamped
    /// to 1; slot 10 there.
    pub fn look(&self, hub_z: f32) -> Vec3 {
        let p = match &self.geom {
            Geom::Tcp(q) => q.advance(self.t, hub_z.abs() * 0.4),
            Geom::Curve(_) => LaneCurve::advance(self.t, hub_z.abs() * 0.4, self.ds),
        };
        match &self.geom {
            // (slot 10 on a curve: F-26180, `railroute::deriv`)
            Geom::Curve(c) => crate::railroute::deriv(c, steer_clamp(p)),
            Geom::Tcp(_) => self.eval(steer_clamp(p)).1,
        }
    }
}

/// The turning radius of the recovery, rejoin and passing rails (F-36195): the pivot radius when the car has stood
/// 2 s, else fMinTurningRadius x (1 + 6 clamp((v - 4.4704) x 0.149129, 0, 1)) (the products rounded, 1 + 6 x fused).
fn turn_radius(ty: &AmbType, v: f32, ready: bool) -> f32 {
    if ready { ty.pivot_turn } else { radius_at(v, ty.min_turn) }
}

pub(crate) fn radius_at(v: f32, min_turn: f32) -> f32 {
    let f9 = (v - 4.4704) * f32::from_bits(0x3E18_B54C);
    let f8 = if f9 >= 0.0 { f9 } else { 0.0 };
    let f5 = if f8 - 1.0 >= 0.0 { 1.0 } else { f8 };
    crate::physics::exact::fma(f5, 6.0, 1.0) * min_turn
}

/// A request for a rail onto a lane (the builder F-26466's request: +16 start, +32 heading, +48 speed, +52 the largest
/// single turn, +72 bit 0x80 ready).
#[derive(Clone, Copy, Debug)]
pub(crate) struct RailReq {
    pub p: Vec3,
    pub h0: Vec3,
    pub v: f32,
    pub limit: f32,
    pub ready: bool,
}

/// The builder's pick: the rail, the lane and phase it ends on, its speed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RailPick {
    pub rail: TcpRail,
    pub lane: usize,
    pub phase: f32,
    pub speed: f32,
}

/// The rail onto a lane (F-26466, the recovery and the rejoin rails): `cands` the lanes the query found (lane, the
/// phase nearest the start t0, the entry's valid byte); ten tries with the car's draws (`draw`: the MWC's new low
/// word): an entry by (x & 0x7FFFFFFF) mod n, skipped when not valid; a phase t0 + U (1 - t0) (U the low 23 bits,
/// fma(U (1 - t0), 2^-23, t0)); the lane's point and derivative there (`target`, lane slot 8); skipped when the heights
/// differ by more than 4 m or when the target heads our way (its unit derivative . H0 > 0) and lies behind
/// ((T - P) . H0 < 0); four arc-line-arc rails P, H0 -> T, unit derivative at the radius (senses 00, 01, 10, 11): a
/// valid one strictly shorter than the best, whose largest single turn is within the limit and which `clear` passes,
/// is the best. Speed: 1.1176 m/s when ready, else max(v, 4).
#[allow(clippy::type_complexity)]
pub(crate) fn rail_to_lane(req: &RailReq, min_turn: f32, pivot: f32, cands: &[(usize, f32, bool)], draw: &mut dyn FnMut() -> u32, target: &mut dyn FnMut(usize, f32) -> (Vec3, Vec3), clear: &mut dyn FnMut(&TcpRail) -> bool) -> Option<RailPick> {
    use crate::physics::exact::fma;
    let n = cands.len() as i32;
    if n == 0 {
        return None;
    }
    let rr = if req.ready { pivot } else { radius_at(req.v, min_turn) };
    let (p, h0) = (req.p, req.h0);
    let mut best_len = f32::MAX;
    let mut best: Option<(TcpRail, usize, f32)> = None;
    for _ in 0..10 {
        let (lane, t0, valid) = cands[((draw() & 0x7FFF_FFFF) as i32 % n) as usize];
        if !valid {
            continue;
        }
        let u = (draw() & 0x7F_FFFF) as f32;
        let ph = fma(u * (1.0 - t0), 1.0 / 8_388_608.0, t0);
        let (t, d) = target(lane, ph);
        if (p.y - t.y).abs() > 4.0 {
            continue;
        }
        let h1 = crate::tcp_rail::vnorm(d);
        let dot = |a: Vec3| fma(h0.x, a.x, fma(h0.y, a.y, h0.z * a.z));
        if dot(h1) > 0.0 && dot(crate::tcp_rail::vdiff(t, p)) < 0.0 {
            continue;
        }
        for (s0, s1) in [(false, false), (false, true), (true, false), (true, true)] {
            let q = crate::tcp_rail::build(p, h0, t, h1, rr, s0, s1);
            if !q.valid() {
                continue;
            }
            let len = q.len();
            if !(len < best_len) || q.max_turn().abs() > req.limit || !clear(&q) {
                continue;
            }
            best_len = len;
            best = Some((q, lane, ph));
        }
    }
    let (rail, lane, phase) = best?;
    let speed = if req.ready { 1.1176 } else if req.v - 4.0 >= 0.0 { req.v } else { 4.0 };
    Some(RailPick { rail, lane, phase, speed })
}

/// The port's stand-in for the car's own MWC (car +384): a 32-bit draw.
fn xorshift(x: &mut u32) -> u32 {
    *x ^= *x << 13;
    *x ^= *x >> 17;
    *x ^= *x << 5;
    *x
}

/// A traffic car on its lane.
#[derive(Clone, Debug)]
pub struct Rider {
    pub ty: AmbType,
    /// the link it is on and the metres along it
    pub link: usize,
    pub s: f32,
    /// the first link of the lane it takes after this one (chosen when the lane is entered)
    pub next: Option<usize>,
    /// speed (car +412)
    pub v: f32,
    /// the speed offset (car +424: -6, 0 or +6 m/s) and the car's age (manager clock - `born`) at its next draw (+432)
    pub off: f32,
    pub off_timer: f32,
    /// acceleration bonus (car +428: U - 0.5, drawn once)
    pub bonus: f32,
    /// the standing gap behind the car ahead (behaviour +460: 2 .. 5 m)
    pub gap: f32,
    /// start delay when moving off (car +444: U x 0.8 s) and the time asked to go (+440)
    pub wait: f32,
    waited: f32,
    /// the constraint entries switched on (hysteresis F-49294): car ahead, stop line, slower lane
    pub on: [bool; 3],
    /// the stop and the car ahead the entries were made for (a new one resets the hysteresis)
    stop_id: Option<usize>,
    obst_id: Option<usize>,
    /// the behaviour's state (F-87488): 0 a trailer (no behaviour), 1 on its lane, 4
    /// physical, 5 come to rest, 6 on a recovery rail, 7 on a passing rail, 8 backing up, 9 a
    /// race path's car on the path's rails, 10 a race path's car let go of by the path's unload
    /// (no rail: stands, not posed, until the unspawner takes it)
    pub state: u8,
    /// states 4 and 5: the physics has it
    pub physical: bool,
    /// the rail off the lanes (states 6 - 8) and the one after it (the passing rail's rejoin)
    pub path: Option<Path>,
    then: Option<Path>,
    /// the recovery try's standing timer (beh +472) and the passing try's
    stand: f32,
    pass_stand: f32,
    /// the stuck tracker (beh +532): seconds standing behind the blocker, seconds on the
    /// reverse rail, tries
    stuck: (f32, f32, u8),
    /// what the passing rail goes round: its index and where it stood
    passing: Option<(usize, Vec3)>,
    /// the last obstacle search's nearest and blocking obstacles (behaviour +432 / +436), for F-47175's head
    kept: Option<(usize, Option<usize>)>,
    /// F-37525's mark (behaviour +444, the result +12): the nearest obstacle that had us as its nearest and that we
    /// let go first (its rider's `rng_id`); F-10623 reads it the frame after
    mark: Option<u32>,
    /// seconds at rest while physical (the physics' sleep)
    rest: f32,
    /// a cab's trailer or a trailer's cab (their ids, +3604)
    pub partner: Option<u32>,
    /// on the ground (the car update's probe and smoothing, car +1696 .. +2528); None: made
    /// anew from the car's pose
    pub seat: Option<Seat>,
    /// the drawn body's pitch and roll (F-37033)
    pub tilt: Tilt,
    /// the physics level holds the car (loose.rs `Amb`: inactive while it rides, active - state 4 -
    /// until its rest rule); `active`: its body is active; `wake`: the game made it physical
    /// (vehicles.rs hands that to the level: F-24099)
    pub in_level: bool,
    pub active: bool,
    pub wake: bool,
    rng: u32,
    id: u32,
    /// the car's own MWC (car +384 / +388): after F-77422's init draws (`InitDraws::car`), drawn by the integrator's
    /// offset timer (F-58614), the recovery try (F-15490) and its rail (F-26466) and the passing try (F-28706)
    pub mwc: GMwc,
    /// the follower's curve speed |dP/dt| (+180) and the lane it was taken on (another
    /// lane: taken anew, as F-48384 does when it sets a rail)
    ds: f32,
    ds_lane: usize,
    /// the phase on the lane (follower +176): the car's point and heading are the curve's
    /// there; `link` / `s` are the graph's link holding it
    t: f32,
    /// (diagnostics) the car ahead last seen: centre distance and margin
    pub ahead: (f32, f32),
    /// the acceleration asked for (car +408), before the integrator's clamp
    pub a: f32,
    /// the speed controller's entry in charge of `a` (car +420: 0 the car ahead; None = free
    /// driving), kept while the car is not driven
    pub slot: Option<usize>,
    /// the drawn wheels' lock (F-37906, `Lock`)
    pub lock: Lock,
    /// when it was made and how long it lives at least on a listed road (car +464 / +468:
    /// 8 + 4U s)
    pub born: f32,
    pub life: f32,
    /// a race path's car on the path's rails (state 9; racescript.rs, `step_route`)
    pub route: Option<Box<Route>>,
    /// a race path's bPhysical (path +170): the instance's +140 bit 0x80 (F-92604 ->
    /// F-55165), for the car's life
    pub phys_pair: bool,
    /// a race path's car: the lane of its path (+64) when F-70324's slot 19 (F-74598 -> F-35476) counts it as one of the
    /// path's rails - the path has not both a curve (+176) and a join rail (+180), and the lane has no flag 0x800 / 0x1000
    /// (the curve rail's 0x1000 never matches) -; the path stays the car's for life (behaviour +468)
    pub race_lane: Option<usize>,
    /// the race path it was started on (behaviour +468, kept for the car's life - F-28180 is its only
    /// writer after the ctor) has bParked (+164, `PathRails::quiet`): behaviour slot 18, F-86955
    pub quiet: bool,
    /// that path has byte +188 (an "Ambient" trigger points at it, `ItemPath::referenced`): behaviour slot 22, F-90466
    pub path_referenced: bool,
    /// the car ahead the speed controller's entry 0 found (behaviour +432, the same obstacle as the
    /// entry's id +456): the length of its car's velocity (its slot 2, F-59415: car +208); None: no obstacle
    pub obst_v: Option<f32>,
    /// the front wheels' drawn steer (F-37033: the mean of the draw object's ring, `SteerRing`, within 45 m; else 0)
    pub steer: f32,
    /// its place in its rail's car list (`RAIL_SEQ` when it took the rail: at its spawn, or when
    /// the rail it holds changed, `Vehicles::stamp_rails`), the rail it was stamped on, and
    /// whether it is still on the rail it was made on
    rail_seq: u64,
    rail_key: Option<RailKey>,
    rail_fresh: bool,
    /// the cruising ghost's route, its speeds and its own random numbers (None: an ordinary car; `GhostRide`)
    pub ghost: Option<Box<GhostRide>>,
}

impl Rider {
    pub fn new(nav: &NavSys, ty: AmbType, link: usize, s: f32, seed: u32) -> Rider {
        let mut r = Rider {
            ty,
            link,
            s,
            next: None,
            v: 0.0,
            off: 0.0,
            off_timer: 0.0,
            bonus: 0.0,
            gap: 3.5,
            wait: 0.0,
            waited: 0.0,
            on: [false; 3],
            stop_id: None,
            obst_id: None,
            state: 1,
            physical: false,
            path: None,
            then: None,
            stand: 0.0,
            pass_stand: 0.0,
            stuck: (0.0, 0.0, 0),
            passing: None,
            kept: None,
            mark: None,
            rest: 0.0,
            partner: None,
            seat: None,
            tilt: Tilt::default(),
            in_level: false,
            active: false,
            wake: false,
            rng: seed.max(1),
            id: seed,
            mwc: mwc_seed(seed),
            ds: 0.0,
            ds_lane: usize::MAX,
            t: 0.0,
            ahead: (0.0, 0.0),
            a: 0.0,
            slot: None,
            lock: Lock::default(),
            born: 0.0,
            life: 8.0,
            route: None,
            race_lane: None,
            phys_pair: false,
            quiet: false,
            path_referenced: false,
            obst_v: None, steer: 0.0,
            rail_seq: next_rail_seq(),
            rail_key: None,
            rail_fresh: true,
            ghost: None,
        };
        r.place(nav, link, s);
        r.life = 8.0 + 4.0 * r.rand();
        r.bonus = r.rand() - 0.5;
        r.gap = 2.0 + 3.0 * r.rand();
        r.wait = r.rand() * 0.8;
        // (the speed offset, car +424, is 0 from the spawn - F-92604 - until its first draw:
        // +432 = 3 + 3U s of age, F-58614)
        r.off = 0.0;
        r.off_timer = 3.0 + 3.0 * r.rand();
        // (the next lane: `choose_next` by the caller, with the shared MWC)
        // (the lane's speed; the spawner and the race paths set the first speed by the start
        // speed rule, `Vehicles::path_start_speed`)
        r.v = rail_speed(nav, link).0;
        r
    }

    /// A bPhysical car (inst +140 bit 0x80) whose pose was written this frame: the follower has
    /// a rail (F-74567 sets the pose flag only then; car slot 20 F-22467 ->
    /// F-86858 puts the inactive instance on the simulator's list, loose.rs `ambient_posed`).
    pub fn pairs_inactive(&self) -> bool {
        self.phys_pair && (matches!(self.state, 1 | 9) || (matches!(self.state, 6..=8) && self.path.is_some()))
    }

    /// a stable id for traces and for the cab / trailer link
    pub fn rng_id(&self) -> u32 {
        self.id
    }

    pub fn rand(&mut self) -> f32 {
        // (the car's own MWC is not reproduced: any uniform draw)
        (xorshift(&mut self.rng) >> 8) as f32 / 16777216.0
    }

    /// The last link of the lane the car is on.
    fn lane_end(&self, nav: &NavSys) -> usize {
        let mut k = self.link;
        let lane = nav.links[k].lane;
        while let Some(&n) = nav.links[k].next.first() {
            if nav.links[n].lane != lane || n == k {
                break;
            }
            k = n;
        }
        k
    }

    /// Which lane follows this one, chosen on entering a lane (slot 16 F-48618 -> F-74277 on the behaviour's route
    /// +288, empty for an ordinary rider -> F-20639 with the mask byte 0xE0, `ghostroute::ordinary_next`): a lane other
    /// than this one at its end vertex (in the railnet's table order) that starts where it ends, by one index draw of the
    /// shared MWC (0x82860890); none: no draw, no next lane. Without the railnet tables (no data folder: tests) the
    /// port's link graph with the same draw.
    pub(crate) fn choose_next(&mut self, nav: &NavSys, shared: &mut GMwc) {
        let end = self.lane_end(nav);
        let lane = nav.links[end].lane;
        if let Some(net) = ghost_net() {
            let l = crate::ghostroute::ordinary_next(net, nav, lane as u32, shared);
            self.next = l.and_then(|l| lanes(nav).links.get(l as usize).and_then(|x| x.first().copied()));
            return;
        }
        let succ: Vec<usize> = nav.links[end].next.iter().copied().filter(|&n| nav.links[n].lane != lane).collect();
        self.next = if succ.is_empty() { None } else { Some(succ[shared.index(succ.len() as u32) as usize]) };
    }

    /// `choose_next` on the frame's shared MWC.
    fn choose_next_env(&mut self, env: &Env) {
        let mut m = env.shared.get();
        self.choose_next(env.nav, &mut m);
        env.shared.set(m);
    }

    /// The lane's phase where the car is (the follower's +176).
    fn phase(&self, _nav: &NavSys) -> f32 {
        self.t
    }

    /// Onto the lane at `s` metres along `link`: the phase of the curve's point nearest to
    /// the link's point there (a rail set from a point, as lane slot 3 finds it), and the
    /// curve speed taken anew.
    pub fn place(&mut self, nav: &NavSys, link: usize, s: f32) {
        let l = &nav.links[link];
        let q = l.a + (l.b - l.a) * (s / l.len.max(1e-3)).clamp(0.0, 1.0);
        self.link = link;
        self.s = s.clamp(0.0, l.len);
        self.t = match nav.curves.get(l.lane) {
            Some(c) if c.pts.len() >= 2 => c.nearest(q, l.t[0], l.t[1]),
            _ => l.phase_at(s),
        };
        self.ds_lane = usize::MAX;
    }

    /// The car's point: the lane's curve at its phase (F-74567 -> F-87587).
    pub fn pos(&self, nav: &NavSys) -> Vec3 {
        let l = &nav.links[self.link];
        match nav.curves.get(l.lane) {
            Some(c) if c.pts.len() >= 2 => c.pose_slot9(self.phase(nav), 0.0).0,
            _ => l.a + (l.b - l.a) * (self.s / l.len.max(1e-3)),
        }
    }

    /// The heading: the curve's derivative at the phase, normalised (F-74567).
    pub fn tangent(&self, nav: &NavSys) -> Vec3 {
        let l = &nav.links[self.link];
        let d = nav.curves.get(l.lane).filter(|c| c.pts.len() >= 2).map_or(Vec3::ZERO, |c| c.pose_slot9(self.phase(nav), 0.0).1);
        d.normalize_or(l.tangent_at(self.s / l.len.max(1e-3)))
    }

    /// The pose the follower stores (F-74567): point, heading, and the curve speed
    /// +180 = the mean of |dP/dt| here and the last value (taken anew on another lane).
    fn pose(&mut self, nav: &NavSys) -> (Vec3, Vec3) {
        let l = &nav.links[self.link];
        let Some(c) = nav.curves.get(l.lane).filter(|c| c.pts.len() >= 2) else { return (self.pos(nav), self.tangent(nav)) };
        // (slot 9, F-87587: the point, dP/du and the speed averaged with +180 - here only while on the same lane)
        let (p, d, s) = c.pose_slot9(self.phase(nav), if self.ds_lane == l.lane { self.ds } else { 0.0 });
        self.ds = s;
        self.ds_lane = l.lane;
        (p, d.normalize_or(l.tangent_at(self.s / l.len.max(1e-3))))
    }

    /// Put the car at phase `t` of its lane: the link holding it and the metres along it.
    fn set_phase(&mut self, nav: &NavSys, lanes: &Lanes, t: f32) {
        let ls = &lanes.links[nav.links[self.link].lane];
        for (i, &k) in ls.iter().enumerate() {
            let l = &nav.links[k];
            if t <= l.t[1] || i + 1 == ls.len() {
                self.link = k;
                self.s = l.s_at(t);
                self.t = t;
                return;
            }
        }
    }

    /// The physics level put the car to rest (F-38021 -> behaviour vfunc 13 F-58957: state 5; the car's speed +412 is
    /// left, the next update measures it).
    pub(super) fn came_to_rest(&mut self) {
        self.active = false;
        if self.state == 4 {
            self.state = 5;
            self.stand = 0.0;
            self.rest = 0.0;
        }
    }

    /// Off its lane onto a rail.
    fn take_path(&mut self, p: Path, state: u8) {
        self.path = Some(p);
        self.state = state;
        self.physical = false;
        self.kept = None;
    }
}

/// The speed of a lane (F-27898 on the lane's owner, flags at owner +4) and whether it is
/// a junction lane (owner flags & 0x30, or lane flags & 0x1000).
/// A lane owned by a junction (F-41621: the lane's owner, or the owner's parent, has the
/// junction flag 4).
fn junction_lane(l: &mcla_formats::rage::navsys::Link) -> bool {
    (l.owner_flags | l.parent_flags) & 4 != 0
}

/// The junction a lane belongs to (F-41621: the lane's owner +08 when the owner's flags +04
/// have bit 4, else the owner's parent when its flags have bit 4; none else), as an index into
/// the navsys junction array (root +58; `NavSys::lane_junction`).
fn lane_junction(nav: &NavSys, lane: usize) -> Option<usize> {
    nav.lane_junction.get(lane).copied().flatten()
}

/// The port's signals' junction built from navsys junction `j` (the one with its centre +80).
/// None for the 12 junctions without arms (`parse_junctions` leaves them out): there
/// F-78185 finds no arm at J +160 and reads the zero vector at 0x828608E0, so no lane of
/// theirs is ever green.
fn signals_junction(signals: &crate::signals::Signals, nav: &NavSys, j: usize) -> Option<usize> {
    let c = *nav.junction_centers.get(j)?;
    let bits = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
    signals.junction_at(c).filter(|&ji| bits(signals.junctions[ji].center) == bits(c))
}

/// F-78185(J, rail): may a car enter `link`'s lane at junction `j` - the lane's first span
/// within 25 degrees of the current arm and the timer J +164 under 6.5 s (`Junction::lane_state`
/// 2)? A junction without arms: never. (Online: always - not ported.)
fn junction_green(signals: &crate::signals::Signals, nav: &NavSys, j: usize, link: usize) -> bool {
    let Some(ji) = signals_junction(signals, nav, j) else { return false };
    let (p0, p1) = lane_entry(nav, link);
    signals.junctions[ji].lane_state(p0 - p1) == 2
}

/// The rail a car's follower holds (behaviour +188) as the ambient searches see it - a lane
/// (state 1) or a race path's rail (state 9) - and its phase there (behaviour +208).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RailKey {
    Lane(usize),
    Path(usize, usize),
}

fn rail_key(nav: &NavSys, r: &Rider) -> Option<(RailKey, f32)> {
    match (&r.route, r.state) {
        (Some(rt), 9) => {
            let (i, f) = rt.at();
            Some((RailKey::Path(rt.path, i), f))
        }
        (None, 1) => Some((RailKey::Lane(nav.links[r.link].lane), r.t)),
        _ => None,
    }
}

/// The order in which cars take rails: a rail's car list (rail +96, a linked list; the
/// obstacle's entry is appended at its tail by F-30206 when the follower sets the rail,
/// F-48384 / F-50894, and unlinked when it leaves, F-13436).
static RAIL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_rail_seq() -> u64 {
    RAIL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A driver's car's lanes (its obstacle, mcRacer's +56: entries +20 + 24 k, at most 12, count
/// +308) as F-58331 keeps them, and the lane it reports (racer +640 lane, +644 squared
/// distance; F-80793).
#[derive(Clone, Debug, Default)]
pub struct DriverLanes {
    /// (lane, phase, the order it was linked into the lane's car list: `next_rail_seq`)
    list: Vec<(usize, f32, u64)>,
    pub best: Option<(usize, f32)>,
    /// the best lane's phase t (the report's +648)
    pub best_t: f32,
    /// the best lane's unit tangent (the report's +624: slot 8's dP/du at the phase, the vector unit's normalisation)
    pub best_tan: Vec3,
}

/// A lane's first two control points (F-78185 reads P0 - P1); the link's ends without them.
fn lane_entry(nav: &NavSys, link: usize) -> (Vec3, Vec3) {
    let c = nav.curves.get(nav.links[link].lane);
    match c.and_then(|c| Some((*c.pts.first()?, *c.pts.get(1)?))) {
        Some(x) => x,
        None => (nav.links[link].a, nav.links[link].b),
    }
}

/// The navsys vertex table (root +0x38: per vertex, the lanes whose first or last control point
/// it is), keyed by the vertex's place (every lane end's place belongs to one vertex index only,
/// checked on the city file: 8437 places, none shared).
fn lane_ends(nav: &NavSys) -> &'static hashbrown::HashMap<[u32; 3], Vec<usize>> {
    static E: std::sync::OnceLock<hashbrown::HashMap<[u32; 3], Vec<usize>>> = std::sync::OnceLock::new();
    E.get_or_init(|| {
        let mut m: hashbrown::HashMap<[u32; 3], Vec<usize>> = hashbrown::HashMap::new();
        let key = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        for (lane, c) in nav.curves.iter().enumerate() {
            let (Some(a), Some(b)) = (c.pts.first(), c.pts.last()) else { continue };
            m.entry(key(*a)).or_default().push(lane);
            if key(*b) != key(*a) {
                m.entry(key(*b)).or_default().push(lane);
            }
        }
        m
    })
}

/// The junction a lane leads into (F-85925): at the vertex of its last control point, the first lane in the vertex's list
/// - not the lane itself - that starts there and belongs to a junction (its owner, or the owner's parent, with flag 4:
/// F-41621). The vertex list's order is the port's (`lane_ends`); the result equals the original's on every city lane
/// (test `lane_into_junction_matches_the_original`).
pub fn lane_into_junction(nav: &NavSys, lane: usize) -> Option<usize> {
    let end = *nav.curves.get(lane)?.pts.last()?;
    let key = [end.x.to_bits(), end.y.to_bits(), end.z.to_bits()];
    lane_ends(nav).get(&key)?.iter().filter(|&&o| o != lane).find_map(|&o| {
        let s = nav.curves[o].pts.first()?;
        if s.x == end.x && s.y == end.y && s.z == end.z { lane_junction(nav, o) } else { None }
    })
}

/// F-91078 with its direction forward (a lane's end to the next lane's start) and the given depth: does `lane` lead to
/// the record `target`? The lane's record (F-17030) is `target`: yes; `stay` (the railnet's byte +69: 1 from its ctor,
/// no other writer found) and the record is not `road`: no; past depth 5: no; else any other lane listed at its last
/// control point's vertex (root +0x38) that starts where it ends (xyz equal) and leads there one level deeper.
pub fn reaches_record(nav: &NavSys, lane: usize, road: mcla_formats::rage::navsys::NavNode, target: mcla_formats::rage::navsys::NavNode, stay: bool, depth: i32) -> bool {
    let rec = &nav.records;
    let r = rec.lane_record.get(lane).copied().flatten();
    if r == Some(target) {
        return true;
    }
    if stay && r != Some(road) {
        return false;
    }
    if depth > 5 {
        return false;
    }
    let (Some(&v), Some(end)) = (rec.last_vertex.get(lane), nav.curves.get(lane).and_then(|c| c.pts.last())) else { return false };
    let Some(list) = rec.vertex_lanes.get(v as usize) else { return false };
    list.iter().any(|&c| {
        c != lane
            && nav.curves.get(c).and_then(|x| x.pts.first()).is_some_and(|s| s.x == end.x && s.y == end.y && s.z == end.z)
            && reaches_record(nav, c, road, target, stay, depth + 1)
    })
}

/// The race traffic clearing's cut-off (the image's word 0.643, F-20685): a road whose arm's |dot| with the record's direction is
/// at most this is a cross road.
pub const CLEAR_DOT: f32 = f32::from_bits(0x3F24_9BA6);

/// The tracker's race traffic clearing (F-20685, docs/reference/people/race_traffic_clearing_rule.md): the cleared
/// roads (+2064, count +2080) and the lanes whose byte +104 it set (mcRailAmbient slot 19 F-88133 writes it, slot 18
/// F-73172 reads it: the road's spawn weight F-72428 and the spawner's lane pick F-10273 give such a lane weight 0).
#[derive(Debug, Clone, Default)]
pub struct RaceClearing {
    pub roads: Vec<usize>,
    pub off: hashbrown::HashSet<usize>,
}

impl RaceClearing {
    /// One run of F-20685 (every manager frame the manager runs). `on`: the -notrafficclearing switch off and (+2107,
    /// 0 from the ctor, or the race byte S+0x64251 with +2108, 1 from the ctor) - a race of type 0..4 is on.
    /// `recs`: the tracker's two junction records (+5888 / +5920 with their directions +5872 / +5904; F-95137 never
    /// sets the second). When `on`, collects from each record's junction the roads of its connection list (root +0x3c)
    /// with an arm (entry +8 >= 0) whose |dot| with the direction is <= 0.643, in that order. Then roads of the set not
    /// collected leave it (swapped with the last; the one swapped in is not looked at again this run), collected roads
    /// not in the set join it (until it holds 4); every lane of a road that left gets 0, and every lane of a road that
    /// joined that leads to one of the records' junctions (F-91078, forward, depth 0) gets 1 - in the order the roads
    /// left / joined, each road's lanes in F-61293's order. Returns those writes in order.
    pub fn step(&mut self, nav: &NavSys, on: bool, stay: bool, recs: &[Option<(usize, [f32; 3])>; 2]) -> Vec<(usize, bool)> {
        use mcla_formats::rage::navsys::NavNode;
        let rec = &nav.records;
        let mut new: Vec<usize> = Vec::new();
        if on {
            for &(j, d) in recs.iter().flatten() {
                let (Some(items), Some(dirs)) = (rec.junction_arms.get(j), rec.arm_dirs.get(j)) else { continue };
                for &(n, arm) in items {
                    // (the item's flags +4 bit 0x2: a road)
                    let NavNode::Road(r) = n else { continue };
                    let Some(a) = usize::try_from(arm).ok().and_then(|k| dirs.get(k)) else { continue };
                    let dot = d[0].mul_add(a[0], d[2].mul_add(a[2], a[1] * d[1]));
                    if dot.abs() > CLEAR_DOT {
                        continue;
                    }
                    new.push(r);
                }
            }
        }
        let mut left = Vec::new();
        let mut k = 0;
        while k < self.roads.len() {
            if !new.contains(&self.roads[k]) {
                left.push(self.roads.swap_remove(k));
            }
            k += 1;
        }
        let mut joined = Vec::new();
        for &r in &new {
            if self.roads.contains(&r) {
                continue;
            }
            joined.push(r);
            self.roads.push(r);
            if self.roads.len() >= 4 {
                break;
            }
        }
        let mut out = Vec::new();
        for &r in &left {
            for &l in rec.road_lanes.get(r).map_or(&[][..], |v| v.as_slice()) {
                out.push((l, false));
            }
        }
        for &r in &joined {
            for &l in rec.road_lanes.get(r).map_or(&[][..], |v| v.as_slice()) {
                if recs.iter().flatten().any(|&(j, _)| reaches_record(nav, l, NavNode::Road(r), NavNode::Junction(j), stay, 0)) {
                    out.push((l, true));
                }
            }
        }
        for &(l, v) in &out {
            if v {
                self.off.insert(l);
            } else {
                self.off.remove(&l);
            }
        }
        out
    }
}

/// What holds a junction's box for F-50959 (`Vehicles::junction_blockers`).
#[derive(Default, Clone, Debug)]
pub struct Blockers {
    /// the player's lane is one of the junction's and near
    pub players: bool,
    /// the cars on its rails that hold it (indices into `Vehicles::cars`)
    pub cars: Vec<usize>,
}

impl Blockers {
    /// Is the junction blocked for car `me` (the car itself left out)?
    pub fn blocks(&self, me: usize) -> bool {
        self.players || self.cars.iter().any(|&k| k != me)
    }
}

/// The junction ahead of the player's lane (F-94775 + F-95137), as a navsys junction
/// index. None when the lane is a junction lane (owner or parent flag 4) or its owner has
/// flags & 0x30 or & 0x40. Else up to 8 rounds: at the lane's end (its first point when
/// `reverse`) take the lanes of that vertex in the vertex table's order, not the lane itself,
/// starting there (`reverse`: ending there); the first such junction lane gives its junction,
/// and the first one whose owner has no flag 0x30 / 0x40 is the next round's lane. (The
/// original goes on to a second junction for a record nothing reads.) The node order of the
/// lanes at a vertex is the port's (`lane_ends`, curve order), not checked against the file.
///
/// Returned with the tracker's first junction record (+5872, F-95137): the unit
/// direction of the junction lane it was found on, its first span CV1 - CV0 (`reverse`: its last, CV[n-1] - CV[n-2]),
/// made unit through the vector unit's dot product z^2 + (y^2 + x^2), its reciprocal square root and one Newton step
/// (as `flat_forward`; not compared with the original here).
pub fn player_junction_record(nav: &NavSys, lane: usize, reverse: bool) -> Option<(usize, [f32; 3])> {
    let lanes = lanes(nav);
    let owner = |l: usize| lanes.links.get(l).and_then(|v| v.first()).map(|&k| &nav.links[k]);
    let l0 = owner(lane)?;
    if junction_lane(l0) || l0.owner_flags & 0x70 != 0 {
        return None;
    }
    let ends = lane_ends(nav);
    let key = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
    let mut cur = lane;
    for _ in 0..8 {
        let c = nav.curves.get(cur)?;
        let p = *(if reverse { c.pts.first() } else { c.pts.last() })?;
        let Some(list) = ends.get(&key(p)) else { break };
        let mut next = None;
        for &o in list {
            if o == cur {
                continue;
            }
            let oc = &nav.curves[o];
            let start = if reverse { oc.pts.last() } else { oc.pts.first() };
            if start.map(|v| key(*v)) != Some(key(p)) {
                continue;
            }
            let Some(l) = owner(o) else { continue };
            if junction_lane(l) {
                if let Some(j) = lane_junction(nav, o) {
                    let p = &oc.pts;
                    let d = match p.len() {
                        0 | 1 => Vec3::ZERO,
                        n if reverse => p[n - 1] - p[n - 2],
                        _ => p[1] - p[0],
                    };
                    let dd = d.z * d.z + (d.y * d.y + d.x * d.x);
                    let dd = if dd < f32::MIN_POSITIVE { 0.0 } else { dd };
                    let mut y = 1.0 / dd.sqrt();
                    if y == f32::INFINITY {
                        y = 0.0;
                    }
                    let k = -(dd * (y * y) - 1.0) * (0.5 * y) + y;
                    return Some((j, [d.x * k, d.y * k, d.z * k]));
                }
            }
            if l.owner_flags & 0x70 != 0 {
                continue;
            }
            next = Some(o);
            break;
        }
        cur = next?;
    }
    None
}

/// A junction's box as F-50959 tests it (mcRnIntersectionAmbient): the centre +0x50 / +0x58,
/// the axes +0x60 and +0x70 (their y terms times 0), the half extents +0x40 and +0x3c.
#[derive(Clone, Copy, Debug)]
struct JunctionBox {
    c: (f32, f32),
    a60: [f32; 3],
    a70: [f32; 3],
    h3c: f32,
    h40: f32,
}

impl JunctionBox {
    /// |a70 . d| < 0.4 x +0x3c and |a60 . d| < 0.4 x +0x40, d = the place less the centre.
    fn holds(&self, p: Vec3) -> bool {
        let (dz, dx) = (p.z - self.c.1, p.x - self.c.0);
        let k = f32::from_bits(0x3ecc_cccd);
        let u = self.a70[0].mul_add(dx, self.a70[2].mul_add(dz, self.a70[1] * 0.0));
        let w = self.a60[0].mul_add(dx, self.a60[2].mul_add(dz, self.a60[1] * 0.0));
        u.abs() < self.h3c * k && w.abs() < self.h40 * k
    }
}

/// The junction boxes, by the junction's centre (+0x80, the signals' `Junction::center`).
fn junction_box(center: Vec3) -> Option<JunctionBox> {
    static B: std::sync::OnceLock<hashbrown::HashMap<(u32, u32), JunctionBox>> = std::sync::OnceLock::new();
    let m = B.get_or_init(|| {
        let mut m = hashbrown::HashMap::new();
        let Some(root) = DATA_ROOT.get() else { return m };
        let Ok(res) = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")) else { return m };
        let rd = mcla_formats::rage::Rd::new(&res);
        let Some(arr) = rd.ptr(0x58) else { return m };
        let n = (rd.u32(0xc8) as usize).min(4096);
        for i in 0..n {
            let o = arr + i * 0xc0;
            if !rd.has(o, 0xc0) || rd.vtable(o) != 0x0047_a0c4 {
                continue;
            }
            let v3 = |p: usize| [rd.f32(p), rd.f32(p + 4), rd.f32(p + 8)];
            let b = JunctionBox { c: (rd.f32(o + 0x50), rd.f32(o + 0x58)), a60: v3(o + 0x60), a70: v3(o + 0x70), h3c: rd.f32(o + 0x3c), h40: rd.f32(o + 0x40) };
            m.insert((rd.f32(o + 0x80).to_bits(), rd.f32(o + 0x88).to_bits()), b);
        }
        m
    });
    m.get(&(center.x.to_bits(), center.z.to_bits())).copied()
}

/// The junction lane a rider whose next lane is `lane` obeys (F-55442): that lane when it is a junction lane; else, when
/// its flags +4 have neither 0x1000 nor 0x800, the first lane of the vertex list at its last control point (root +0x38,
/// `RecordData::vertex_lanes`, in the list's order) other than itself whose first control point equals that point (x, y,
/// z) and that is a junction lane (F-32663); else none. (0x1000 takes an override lane of the car's instead (its +16 ->
/// +452: byte +64 bit 0x80, the lane +48), not ported; no city lane has 0x1000 or 0x800 in the data.)
pub(crate) fn light_lane(nav: &NavSys, lane: usize) -> Option<usize> {
    let link = |l: usize| nav.links.get(nav.links.partition_point(|k| k.lane < l)).filter(|k| k.lane == l);
    let own = link(lane)?;
    if junction_lane(own) {
        return Some(lane);
    }
    if own.flags & 0x1800 != 0 {
        return None;
    }
    let end = *nav.curves.get(lane)?.pts.last()?;
    let v = *nav.records.last_vertex.get(lane)?;
    nav.records.vertex_lanes.get(v as usize)?.iter().copied().find(|&m| {
        m != lane
            && nav.curves.get(m).and_then(|c| c.pts.first()).is_some_and(|&p| p.x == end.x && p.y == end.y && p.z == end.z)
            && link(m).is_some_and(junction_lane)
    })
}

/// The light a rider obeys (F-55442 -> F-83573 / F-16850 / F-78185): the junction lane it is to enter (`light_lane`)
/// and its junction's state for that lane (`Junction::lane_state`, the lane's first span against the current arm).
/// Returns (junction, xz metres from `pos` to the junction lane's first point = the stop line, state 2 green / 1 yellow
/// / 0 red). (The junction is the lane's, F-41621; one without arms gives none here.)
fn signal_stop(env: &Env, r: &Rider, pos: Vec3) -> Option<(usize, f32, u8, usize)> {
    let nav = env.nav;
    let n = r.next?;
    let first = match light_lane(nav, nav.links[n].lane)? {
        l if l == nav.links[n].lane => n,
        l => nav.links.partition_point(|k| k.lane < l),
    };
    let c = nav.curves.get(nav.links[first].lane)?;
    let (p0, p1) = match (c.pts.first(), c.pts.get(1)) {
        (Some(a), Some(b)) => (*a, *b),
        _ => (nav.links[first].a, nav.links[first].b),
    };
    let nj = lane_junction(nav, nav.links[first].lane)?;
    let ji = signals_junction(env.signals, nav, nj)?;
    let d = Vec3::new(p0.x - pos.x, 0.0, p0.z - pos.z).length();
    Some((ji, d, env.signals.junctions[ji].lane_state(p0 - p1), nj))
}

pub fn rail_speed(nav: &NavSys, link: usize) -> (f32, bool) {
    let l = &nav.links[link];
    let f = l.owner_flags;
    let v = if f & 0x20 != 0 && l.owner_w10 == 0 {
        15.0 * MPH
    } else if f & 0x10 != 0 {
        15.0 * MPH
    } else if f & 0x20 != 0 {
        10.0 * MPH
    } else if f & 0x40 != 0 {
        6.0 * MPH
    } else {
        nav.speed_limit_mps(link).unwrap_or(DEFAULT_ROAD_SPEED)
    };
    (v, f & 0x30 != 0 || l.flags & 0x1000 != 0)
}

/// F-70324's wanted speeds (the car's, the next rail's): the rail's speed (slot 17, `rail_speed`) and the next rail's
/// (0 without one); on a junction rail (`rail_speed`'s flag) the next is the rail's own and no offset is added. Each, when
/// above 0, is the offset (car +424) plus the speed, at least 10 mph. `holds` (`race_holds`): the first stops both, the
/// second the next.
pub fn wanted(cur: (f32, bool), next: Option<f32>, off: f32, holds: (bool, bool)) -> (f32, f32) {
    let (want, junction) = cur;
    let (next, off) = if junction { (want, 0.0) } else { (next.unwrap_or(0.0), off) };
    let up = |v: f32| {
        let x = off + v;
        if MIN_WANT - x >= 0.0 { MIN_WANT } else { x }
    };
    let want = if want > 0.0 { up(want) } else { want };
    let next = if next > 0.0 { up(next) } else { next };
    match holds {
        (true, _) => (0.0, 0.0),
        (false, true) => (want, 0.0),
        _ => (want, next),
    }
}

/// A lane's flag words as F-70324 reads them: its owner's (+8 -> +4; 0 with none), the owner's parent's (+8 -> +4) and its
/// own (+4).
#[derive(Clone, Copy, Debug, Default)]
pub struct LaneBits {
    pub owner: u32,
    pub parent: u32,
    pub flags: u32,
}

impl LaneBits {
    fn of(l: &mcla_formats::rage::navsys::Link) -> Self {
        LaneBits { owner: l.owner_flags, parent: l.parent_flags, flags: l.flags }
    }

    /// F-50269: the lane belongs to a junction (its owner's or the owner's parent's flag 4).
    fn junction(self) -> bool {
        (self.owner | self.parent) & 4 != 0
    }
}

/// The event holds of F-70324 (only while a race is on, for a car not a proxy type and not on its race path's rails;
/// online and the `spawnaroundcamera` parameter off): (the car stops where it is - its speed and the next 0 -, the next
/// rail's speed 0). On a junction lane whose owner has flag 0x10 (F-73512) the car stops; a next lane of that kind
/// (F-50269 and F-15116) holds the next. When the car's heading `h` (instance row 2, x / z) is within 45 degrees of the
/// player's flat forward `d` or its reverse (|h.z d.z + d.x h.x| > 0.7071, fused) the lanes with flag 0x2000 that lead into
/// the junction ahead of the player's lane (`into`, F-85925; `player_j`, the tracker +5888 - its second record +5920
/// stays 0) are held too: the current one while its phase is under 0.3, the next one at any phase.
#[allow(clippy::too_many_arguments)]
pub fn race_holds(cur: LaneBits, next: Option<LaneBits>, phase: f32, h: (f32, f32), d: (f32, f32), into: (Option<usize>, Option<usize>), player_j: Option<usize>) -> (bool, bool) {
    let mut held_next = next.is_some_and(|n| n.junction() && n.owner & 0x10 != 0);
    if cur.owner & 0x10 != 0 && cur.junction() {
        return (true, held_next);
    }
    let dot = h.1.mul_add(d.1, d.0 * h.0);
    if !(dot.abs() > f32::from_bits(0x3F35_0481)) {
        return (false, held_next);
    }
    let ahead = |j: Option<usize>| j.is_some() && j == player_j;
    let held = cur.flags & 0x2000 != 0 && phase < f32::from_bits(0x3E99_999A) && ahead(into.0);
    held_next |= next.is_some_and(|n| n.flags & 0x2000 != 0) && ahead(into.1);
    (held, held_next)
}

/// F-70324's flat unit vector of the player's forward (the tracker +224, F-83397: row 2 with y = 0, through the
/// vector unit's dot product z^2 + (y^2 + x^2), its reciprocal square root and one Newton step; negated).
pub fn flat_forward(back: Vec3) -> (f32, f32) {
    let dd = back.z * back.z + back.x * back.x;
    let dd = if dd < f32::MIN_POSITIVE { 0.0 } else { dd };
    let mut y = 1.0 / dd.sqrt();
    if y == f32::INFINITY {
        y = 0.0;
    }
    let k = -(dd * (y * y) - 1.0) * (0.5 * y) + y;
    (-(back.x * k), -(back.z * k))
}

/// The acceleration that brings speed `v` to `vt` at `dist - margin` metres (F-20809):
/// over the time the gap takes at the average speed. Standing with the target standing: `up`
/// (MaxAccel + bonus) while there is room, else `-down`; no room left (or a negative time):
/// `-down`.
fn needed(v: f32, vt: f32, dist: f32, margin: f32, up: f32, down: f32) -> f32 {
    let gap = dist - margin;
    let avg = v + (vt - v) * 0.5;
    if avg == 0.0 {
        return if gap > 0.0 { up } else { -down };
    }
    let t = gap / avg;
    if t <= 0.0 {
        return -down;
    }
    (vt - v) / t
}

/// What another car is to a rider looking for the car ahead (an obstacle).
pub struct Other {
    pub pos: Vec3,
    pub fwd: Vec3,
    pub right: Vec3,
    /// its instance matrix's row 2 (`instance_matrix`: the back axis)
    pub back: Vec3,
    /// its instance matrix's row 0 (F-87404 reads it for the corners)
    pub row0: Vec3,
    /// its instance matrix as the frame starts (`instance_matrix`: rows X, up, Z, place) - what F-33765 / F-58860 hand
    /// the footprint probe
    pub inst: [Vec3; 4],
    pub vel: Vec3,
    /// half its bound's length and width
    pub half_len: f32,
    pub half_wid: f32,
    /// an ambient car's obstacle size (slot 3, F-46284: `obstacle_size`); None for the others
    pub size: Option<(f32, f32)>,
    /// its rider's speed (car +412; 0 without a rider)
    pub speed: f32,
    /// its behaviour (car +452; none for a trailer or a parked car): its rails +188 / +192
    pub beh: Option<(Option<SelRail>, Option<SelRail>)>,
    /// a driver's car (the player): its lane +640 (`DriverLanes::best`) with that lane's freeway test (F-31543), and
    /// its standing time +872 (F-16834)
    pub lane640: Option<(usize, bool)>,
    pub idle: f32,
    /// on a lane, kinematic: (its lane, its next lane, junction lane)
    pub on: Option<(usize, Option<usize>, bool)>,
    pub player: bool,
    /// ambient traffic (parked cars too); else the player, police, racers
    pub ambient: bool,
    /// physically active (awake)
    pub active: bool,
    pub cab_or_trailer: bool,
    /// (diagnostics) its rider's id
    pub key: u32,
    /// its grid object's kind byte +6 (`brainturn::ObjectInfo::from_car`: 0 the player, 1 racers, 2 the others)
    pub kind: u8,
    /// its rider's nearest obstacle of the last search (behaviour +432) as that obstacle's `key`
    pub nearest: Option<u32>,
}

/// A rail as F-47175's selection compares them (its context +12 / +16, a behaviour's +188 / +192): a navsys lane, or a
/// run-time rail (word +4 with 0x1800; each is one car's own, so it equals only itself: the owner's index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelRail {
    Lane(usize),
    RunTime(u32),
}

/// The player's +876 (mcPlayer's set-up F-96799): the sum of the three signal words (1.5, 2.0,
/// 6.5: the signal phase's lengths) times the image's 4.0, 40 s.
pub const IDLE_BLOCKS: f32 = (6.5 + 2.0 + 1.5) * 4.0;

/// What F-47175's selection, F-70116 and F-87404 read of an ambient obstacle's car (+16): its speed +412, its behaviour
/// +452 (None: none) with the rails +188 / +192, its type's bytes +111 / +112 (cab, trailer), physics active (F-17857).
#[derive(Clone, Copy, Debug)]
pub struct SelCar {
    pub speed: f32,
    pub beh: Option<(Option<SelRail>, Option<SelRail>)>,
    pub cab_or_trailer: bool,
    pub active: bool,
}

/// What F-47175's selection loop (with F-70116 and F-87404) reads of the world.
pub trait SelectWorld {
    /// an obstacle's matrix (slot 1): rows 0, 2 and 3, x / y / z
    fn rows(&self, o: usize) -> ([f32; 3], [f32; 3], [f32; 3]);
    /// slot 2: its velocity
    fn vel(&self, o: usize) -> [f32; 3];
    /// slot 3 (F-46284): its bound's extent along x and along z
    fn size(&self, o: usize) -> (f32, f32);
    /// its kind byte +6 (0 the player, 1 racers, 2 ambient cars, 9 an object with a driver's car at +312)
    fn kind(&self, o: usize) -> u8;
    /// a kind 2 obstacle's car (+16)
    fn ambient(&self, o: usize) -> Option<SelCar>;
    /// a driver's car (kind 0: the obstacle itself; kind 9: its +312): its lane +640 with F-31543's freeway answer,
    /// its standing time +872 and the limit +876
    fn driver(&self, o: usize) -> Option<(Option<bool>, f32, f32)>;
    /// a rail's slot 14 (a lane's F-24415) at an obstacle's place (slot 1 +48): the nearest point and its phase
    fn nearest(&self, rail: SelRail, o: usize) -> ([f32; 3], f32);
    /// a rail's slot 12 (a lane's first control point)
    fn start(&self, rail: SelRail) -> [f32; 3];
}

/// F-87404 (`o` the obstacle, `point` a rail's nearest point to it, `asker`): is the obstacle clear of that point? Its
/// corners (rows 0 and 2 times half its extents, y = 0: Z + X, Z - X, -Z + X, -Z - X) less (point - its place, y = 0),
/// each along the unit vector from its place to the point (y = 0, the vector unit's normalisation): the running least
/// and greatest of those; not clear (in the way) as soon as they straddle 0 or either is within half the asker's x
/// extent of it.
pub fn clear_of<W: SelectWorld>(w: &W, o: usize, point: [f32; 3], asker: usize) -> bool {
    use crate::physics::exact::fma;
    use crate::tcp_rail::{vadd3, vdiff, vmul3, vnorm};
    let half = f32::from_bits(0x3F00_0000);
    let (e0, e1) = w.size(o);
    let (hx, hz) = (e0 * half, e1 * half);
    let (r0, r2, r3) = w.rows(o);
    let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
    let z = flat(vmul3(Vec3::from(r2), hz));
    let x = flat(vmul3(Vec3::from(r0), hx));
    let nz = vdiff(Vec3::ZERO, z);
    let corners = [vadd3(z, x), vdiff(z, x), vadd3(nz, x), vdiff(nz, x)];
    let p = Vec3::from(point);
    let q = flat(vdiff(p, Vec3::from(r3)));
    let u = vnorm(q);
    let hw = w.size(asker).0 * half;
    let (mut mn, mut mx) = (f32::MAX, f32::MIN);
    for c in corners {
        let c = vdiff(c, q);
        let s = fma(c.y, u.y, fma(c.z, u.z, u.x * c.x));
        // (two `fsel`: a NaN difference takes s for the least, keeps the greatest)
        mn = if s - mn >= 0.0 { mn } else { s };
        mx = if s - mx >= 0.0 { s } else { mx };
        if (mn < 0.0 && mx > 0.0) || mn.abs() < hw || mx.abs() < hw {
            return false;
        }
    }
    true
}

/// F-70116 (`o`, `asker`): is the obstacle a blocking one? Not when |velocity|^2 (z^2 + (x^2 + y^2), fused) > the
/// image's 4.9961185. A driver's car (kind 0, kind 9's +312): its back axis within 70 degrees of the asker's (flat
/// dot, fused, < 0.34202015), or its lane +640 a freeway lane, or standing longer than +876 (+872 > +876). An ambient
/// car (kind 2, its +16): without a current rail (no behaviour or none at +188): standing (+412 == 0) and not physics
/// active, or slower than 0.44704 (1 mph) and a cab or trailer; then only on a run-time rail (0x1800), standing, not
/// active and its back axis against the asker's (dot < 0). Other kinds: no.
pub fn blocking_of<W: SelectWorld>(w: &W, o: usize, asker: usize) -> bool {
    use crate::physics::exact::fma;
    let v = w.vel(o);
    if fma(v[2], v[2], fma(v[0], v[0], v[1] * v[1])) > f32::from_bits(0x409F_E034) {
        return false;
    }
    match w.kind(o) {
        0 | 9 => {
            let Some((lane, idle, limit)) = w.driver(o) else { return false };
            let (_, ro, _) = w.rows(o);
            let (_, ra, _) = w.rows(asker);
            if fma(ro[2], ra[2], ro[0] * ra[0]) < f32::from_bits(0x3EAF_1D44) {
                return true;
            }
            if lane == Some(true) {
                return true;
            }
            idle > limit
        }
        2 => {
            let Some(c) = w.ambient(o) else { return false };
            let rail = c.beh.and_then(|b| b.0);
            if rail.is_none() {
                if c.speed == 0.0 && !c.active {
                    return true;
                }
                if c.speed < f32::from_bits(0x3EE4_E26D) && c.cab_or_trailer {
                    return true;
                }
            }
            if !matches!(rail, Some(SelRail::RunTime(_))) || c.speed != 0.0 || c.active {
                return false;
            }
            let (_, ra, _) = w.rows(asker);
            let (_, ro, _) = w.rows(o);
            fma(ra[0], ro[0], fma(ra[2], ro[2], ra[1] * ro[1])) < 0.0
        }
        _ => false,
    }
}

/// F-47175's take of a candidate that is ahead and nearer (the loop's tests after the distance): an ambient car riding
/// a lane (F-96472) whose +188 is the asker's current or next rail - or whose +192 is the next rail, while it moves and
/// the asker's place is within 75 m (flat, fused, < 5625) of that rail's start - is taken; else with no current rail
/// it is taken; else the current rail's nearest point when its phase is in (0, 1), else the next rail's when in (0, 1)
/// and not below `b_start` (context +32, the next rail's start phase +196), is tested with F-87404 (taken unless
/// clear); otherwise not taken.
fn takes<W: SelectWorld>(w: &W, o: usize, asker: usize, a: Option<SelRail>, b: Option<SelRail>, b_start: f32) -> bool {
    use crate::physics::exact::fma;
    if w.kind(o) == 2 {
        if let Some(c) = w.ambient(o) {
            if let Some((Some(r188 @ SelRail::Lane(_)), r192)) = c.beh {
                if Some(r188) == a || Some(r188) == b {
                    return true;
                }
                if r192.is_some() && r192 == b && c.speed != 0.0 {
                    let (_, _, pa) = w.rows(asker);
                    let s = w.start(r192.unwrap());
                    let (dz, dx) = (s[2] - pa[2], s[0] - pa[0]);
                    if fma(dx, dx, dz * dz) < 5625.0 {
                        return true;
                    }
                }
            }
        }
    }
    let Some(a) = a else { return true };
    let (pt, t) = w.nearest(a, o);
    if t > 0.0 && t < 1.0 {
        return !clear_of(w, o, pt, asker);
    }
    let Some(b) = b else { return false };
    let (pt, t) = w.nearest(b, o);
    if !(t > 0.0 && t < 1.0 && !(t < b_start)) {
        return false;
    }
    !clear_of(w, o, pt, asker)
}

/// F-47175's selection loop over the gather's candidates in their order: d = place - the asker's (vector, flushed),
/// f = -(d . the asker's back axis) (x last, fused); behind (f < 0) skipped; nearer than the nearest so far (from
/// FLT_MAX; `bge` skips, so a NaN is never nearer) and / or nearer than the blocking so far and blocking (F-70116); either:
/// taken by `takes` -> the result's word(s) and distance(s). Returns (nearest, blocking).
pub fn select_obstacles<W: SelectWorld>(w: &W, asker: usize, cands: &[usize], a: Option<SelRail>, b: Option<SelRail>, b_start: f32) -> (Option<usize>, Option<usize>) {
    use crate::physics::exact::fma;
    let (mut near_d, mut block_d) = (f32::MAX, f32::MAX);
    let mut out = (None, None);
    for &o in cands {
        let (_, ra, pa) = w.rows(asker);
        let (_, _, po) = w.rows(o);
        let d = crate::tcp_rail::vdiff(Vec3::from(po), Vec3::from(pa));
        let f = -fma(d.x, ra[0], fma(ra[1], d.y, ra[2] * d.z));
        if f < 0.0 {
            continue;
        }
        let near = f < near_d;
        let block = f < block_d && blocking_of(w, o, asker);
        if !near && !block {
            continue;
        }
        if !takes(w, o, asker, a, b, b_start) {
            continue;
        }
        if near {
            out.0 = Some(o);
            near_d = f;
        }
        if block {
            out.1 = Some(o);
            block_d = f;
        }
    }
    out
}

/// A run-time rail as the selection (F-47175) asks it: the arc-line-arc rail (mcRailAmbientTCP, vtable 0x820CD224:
/// slot 14 the thunk F-36154 -> F-16977 on the record at rail +128, slot 12 F-55960 -> F-33742 the record's start P) or
/// a pool curve (mcRailAmbientDynamic, vtable 0x820CD1BC: the lanes' slot 14 F-24415 and slot 12 F-88700, the curve's
/// first control point).
#[derive(Clone, Copy)]
pub(crate) enum RunRail<'a> {
    Tcp(&'a TcpRail),
    Curve(&'a LaneCurve),
}

impl RunRail<'_> {
    /// Slot 14 at a candidate's place (slot 1 +48): the nearest point and its phase (r3 out, r6 the phase; F-16977's
    /// squared distance r7 is not read by the selection).
    pub(crate) fn nearest(self, p: Vec3) -> ([f32; 3], f32) {
        match self {
            RunRail::Tcp(q) => {
                let (pt, t, _) = q.nearest(p);
                (pt.to_array(), t)
            }
            RunRail::Curve(c) => {
                let (t, _, q) = crate::railroute::nearest(c, p, 4, 10);
                (q.to_array(), t)
            }
        }
    }

    /// Slot 12: the rail's start.
    pub(crate) fn start(self) -> [f32; 3] {
        match self {
            RunRail::Tcp(q) => q.p.to_array(),
            RunRail::Curve(c) => c.pts.first().copied().unwrap_or(Vec3::ZERO).to_array(),
        }
    }
}

impl Geom {
    pub(crate) fn run(&self) -> RunRail<'_> {
        match self {
            Geom::Tcp(q) => RunRail::Tcp(q),
            Geom::Curve(c) => RunRail::Curve(c),
        }
    }
}

/// The car ahead (F-47175): the nearest obstacle and the nearest "blocking" one.
struct Ahead {
    j: usize,
    d: f32,
    vt: f32,
    blocking: Option<usize>,
    player_across: bool,
}

/// The world of F-47175's selection for a rider: the frame's cars (`Other`), the lanes' curves, the asker's own run-time
/// rail (states 6, 7, 9 and the ghost's lane change).
struct EnvSelect<'e, 'a> {
    env: &'e Env<'a>,
    me: usize,
    me_rows: ([f32; 3], [f32; 3], [f32; 3]),
    me_size: (f32, f32),
    me_rail: Option<RunRail<'e>>,
}

impl SelectWorld for EnvSelect<'_, '_> {
    fn rows(&self, o: usize) -> ([f32; 3], [f32; 3], [f32; 3]) {
        match self.env.others.get(o) {
            Some(x) if o != self.me => (x.row0.to_array(), x.back.to_array(), x.pos.to_array()),
            _ => self.me_rows,
        }
    }
    fn vel(&self, o: usize) -> [f32; 3] {
        self.env.others.get(o).map_or([0.0; 3], |x| x.vel.to_array())
    }
    fn size(&self, o: usize) -> (f32, f32) {
        match self.env.others.get(o) {
            Some(x) if o != self.me => x.size.unwrap_or((2.0 * x.half_wid, 2.0 * x.half_len)),
            _ => self.me_size,
        }
    }
    fn kind(&self, o: usize) -> u8 {
        self.env.others.get(o).map_or(2, |x| x.kind)
    }
    fn ambient(&self, o: usize) -> Option<SelCar> {
        let x = self.env.others.get(o)?;
        x.ambient.then_some(SelCar { speed: x.speed, beh: x.beh, cab_or_trailer: x.cab_or_trailer, active: x.active })
    }
    fn driver(&self, o: usize) -> Option<(Option<bool>, f32, f32)> {
        let x = self.env.others.get(o)?;
        x.player.then_some((x.lane640.map(|l| l.1), x.idle, IDLE_BLOCKS))
    }
    fn nearest(&self, rail: SelRail, o: usize) -> ([f32; 3], f32) {
        let p = Vec3::from(self.rows(o).2);
        match rail {
            SelRail::Lane(l) => {
                let (t, _, q) = crate::railroute::nearest(&self.env.nav.curves[l], p, 4, 10);
                (q.to_array(), t)
            }
            SelRail::RunTime(_) => self.me_rail.map_or((p.to_array(), 0.0), |q| q.nearest(p)),
        }
    }
    fn start(&self, rail: SelRail) -> [f32; 3] {
        match rail {
            SelRail::Lane(l) => self.env.nav.curves[l].pts.first().copied().unwrap_or(Vec3::ZERO).to_array(),
            SelRail::RunTime(_) => self.me_rail.map_or([0.0; 3], |q| q.start()),
        }
    }
}

/// The car ahead among the gather's candidates `cands` in their order (F-47175's selection, `select_obstacles`), on the
/// rails `a` (current; `run` the rail when it is a run-time rail) and `b` (next, entered at its start: phase 0).
/// The distance and the speed along our heading are the port's (F-10623 reads them).
#[allow(clippy::too_many_arguments)]
fn find_ahead(env: &Env, me: usize, me_rows: ([f32; 3], [f32; 3], [f32; 3]), me_size: (f32, f32), pos: Vec3, fwd: Vec3, run: Option<RunRail>, a: Option<SelRail>, b: Option<SelRail>, b_start: f32, cands: &[usize]) -> Ahead {
    let w = EnvSelect { env, me, me_rows, me_size, me_rail: run };
    let (near, block) = select_obstacles(&w, me, cands, a, b, b_start);
    let others = env.others;
    let mut player_across = false;
    for &j in cands {
        let Some(o) = others.get(j) else { continue };
        if !o.player || j == me {
            continue;
        }
        let rel = o.pos - pos;
        if rel.dot(fwd) < 0.0 || rel.y.abs() > 4.0 || takes(&w, j, me, a, b, b_start) {
            continue;
        }
        // (the player's car across our lane or the next: MaxDecel x 3; the original's test
        // is on the lanes the player is registered on - here within 3 m of our way)
        let side = (rel - fwd * rel.dot(fwd)).length();
        if side < 3.0 && o.fwd.dot(fwd).abs() < 0.7071 {
            player_across = true;
        }
    }
    let Some(j) = near else { return Ahead { j: usize::MAX, d: 0.0, vt: 0.0, blocking: block, player_across } };
    let o = &others[j];
    let d = (o.pos - pos).length();
    // its speed along our heading, but not for a car turning through a junction
    let vt = match o.on {
        Some((_, _, true)) => o.vel.length(),
        _ => o.vel.dot(fwd),
    };
    Ahead { j, d, vt: vt.max(0.0), blocking: block, player_across }
}

/// A rail as the obstacle gather sees it (F-47175's context +12 / +16 / +20): a navsys lane (rail word +4 without
/// 0x1800) or a run-time rail (with it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatherRail {
    Lane(usize),
    RunTime,
}

/// What the obstacle gather (F-47175, F-14781, F-25797, F-44673) reads of the world.
pub trait GatherWorld {
    /// the navsys vertex table (root +0x38): the lanes listed at a vertex, in the table's order
    fn vertex_lanes(&self, v: u32) -> &[usize];
    /// a lane's first and last control points' vertex indices (its curve, slot 15: +0x20 [0] and [+0x1c])
    fn lane_vertices(&self, lane: usize) -> (u32, u32);
    /// a lane's slot 12 (control point 0) and slot 13 (its end), x / y / z
    fn lane_ends(&self, lane: usize) -> ([f32; 3], [f32; 3]);
    /// the obstacles on a lane's car list (rail +96), in the list's order
    fn rail_cars(&self, lane: usize) -> &[usize];
    /// the objects of the 3 x 3 grid cells round an obstacle's place (F-88455 / F-99153), each cell's list in order
    fn grid_cells(&self, asker: usize) -> Vec<&[usize]>;
    /// an object's kind byte +6 (0 the player, 1 racers, 2 ambient cars, ...)
    fn kind(&self, o: usize) -> u8;
    /// an obstacle's matrix (slot 1): row 2 (+32) and row 3, its place (+48)
    fn rows(&self, o: usize) -> ([f32; 3], [f32; 3]);
}

/// The gather's room (F-47175's stack array: 64 obstacles).
pub const GATHER_CAP: usize = 64;

/// A VMX lane in the flush mode: a denormal is a zero of its sign.
fn vmx_flush(x: f32) -> f32 {
    if x.is_subnormal() {
        0.0f32.copysign(x)
    } else {
        x
    }
}

/// x / y / z equal as F-14781 / F-25797 compare two points (the mask of the image's run-time words = xyz, vcmpeqfp in
/// the flush mode: -0 equals 0, a denormal equals 0, a NaN equals nothing).
fn vmx_eq3(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| vmx_flush(a[i]) == vmx_flush(b[i]))
}

/// F-44673: a rail's car list into the candidates. Each obstacle on the list but the asker: d = its place - the asker's
/// (a vector subtract in the flush mode); skipped when fma(r2.x, d.x, fma(r2.y, d.y, r2.z d.z)) > 0 with r2 the asker's
/// row 2 (the back axis: behind); skipped when already a candidate; the walk ends when `cap` are held; else appended.
pub fn gather_rail<W: GatherWorld>(w: &W, lane: usize, asker: usize, out: &mut Vec<usize>, cap: usize) {
    use crate::physics::exact::fma;
    for &o in w.rail_cars(lane) {
        if o == asker {
            continue;
        }
        let (r2, pa) = w.rows(asker);
        let (_, po) = w.rows(o);
        let d: [f32; 3] = std::array::from_fn(|i| vmx_flush(vmx_flush(po[i]) - vmx_flush(pa[i])));
        let f10 = r2[2] * d[2];
        let f7 = fma(r2[1], d[1], f10);
        let f6 = fma(r2[0], d[0], f7);
        if f6 > 0.0 {
            continue;
        }
        if out.contains(&o) {
            continue;
        }
        if out.len() >= cap {
            return;
        }
        out.push(o);
    }
}

/// F-25797: one rail `r` (none: nothing) and its two vertices' lists: of the list at its start (`va`) the rails whose
/// end differs from r's start (the rails leading into r skipped; r itself is taken), of the list at its end (`vb`) the
/// rails but r whose start differs from r's end (r's successors skipped); each rail's car list by F-44673.
fn gather_one<W: GatherWorld>(w: &W, r: Option<usize>, va: u32, vb: u32, asker: usize, out: &mut Vec<usize>) {
    let Some(r) = r else { return };
    for &x in w.vertex_lanes(va) {
        if !vmx_eq3(w.lane_ends(r).0, w.lane_ends(x).1) {
            gather_rail(w, x, asker, out, GATHER_CAP);
        }
    }
    for &x in w.vertex_lanes(vb) {
        if x != r && !vmx_eq3(w.lane_ends(r).1, w.lane_ends(x).0) {
            gather_rail(w, x, asker, out, GATHER_CAP);
        }
    }
}

/// F-14781: the rail `a` and the next rail `b` that starts at a's end vertex: of a's start vertex list the rails whose
/// end differs from a's start, of the shared vertex's list every rail but a, of b's end vertex list the rails but b
/// whose start differs from b's end; each rail's car list by F-44673.
#[allow(clippy::too_many_arguments)]
fn gather_two<W: GatherWorld>(w: &W, a: usize, b: usize, l1: u32, l2: u32, l3: u32, asker: usize, out: &mut Vec<usize>) {
    for &x in w.vertex_lanes(l1) {
        if !vmx_eq3(w.lane_ends(a).0, w.lane_ends(x).1) {
            gather_rail(w, x, asker, out, GATHER_CAP);
        }
    }
    for &x in w.vertex_lanes(l2) {
        if x != a {
            gather_rail(w, x, asker, out, GATHER_CAP);
        }
    }
    for &x in w.vertex_lanes(l3) {
        if x != b && !vmx_eq3(w.lane_ends(b).1, w.lane_ends(x).0) {
            gather_rail(w, x, asker, out, GATHER_CAP);
        }
    }
}

/// The candidates of the obstacle search (F-47175's gather), in its order. `cur` / `next` / `other`: the behaviour's
/// rail (+188), next rail (+192) and the rail at +104 (its pass holder +92's +12). No current rail or a run-time one:
/// first the objects of the 3 x 3 grid cells round the asker of kind 0 or 2, not the asker, while fewer than 64 (no
/// duplicate test). Then the rail lists: A = the current rail if a lane, else `other` if a lane; B = the next rail if
/// a lane. A and B with A's end vertex B's start vertex: F-14781(A, B); else F-25797 for A, then for B.
pub fn gather_obstacles<W: GatherWorld>(w: &W, asker: usize, cur: Option<GatherRail>, next: Option<GatherRail>, other: Option<GatherRail>) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    if !matches!(cur, Some(GatherRail::Lane(_))) {
        for cell in w.grid_cells(asker) {
            for &o in cell {
                let k = w.kind(o);
                if (k == 2 || k == 0) && o != asker && out.len() < GATHER_CAP {
                    out.push(o);
                }
            }
        }
    }
    let a = match (cur, other) {
        (Some(GatherRail::Lane(l)), _) | (_, Some(GatherRail::Lane(l))) => Some(l),
        _ => None,
    };
    let b = match next {
        Some(GatherRail::Lane(l)) => Some(l),
        _ => None,
    };
    let (a0, a1) = a.map_or((u32::MAX, u32::MAX), |l| w.lane_vertices(l));
    let (b0, b1) = b.map_or((u32::MAX, u32::MAX), |l| w.lane_vertices(l));
    match (a, b) {
        (Some(a), Some(b)) if a1 == b0 && a1 != u32::MAX => gather_two(w, a, b, a0, a1, b1, asker, &mut out),
        _ => {
            gather_one(w, a, a0, a1, asker, &mut out);
            gather_one(w, b, b0, b1, asker, &mut out);
        }
    }
    out
}

/// F-47175's head: whether the last search stands this frame (no gather; the nearest and blocking words kept). Only
/// with a last nearest obstacle, when the asker's slot 2 (its velocity) and the last nearest's are zero in x / y / z
/// (flush mode: a denormal is zero): one draw of the shared MWC, kept when U23 x 2^-23 > 0.025 (0x3CCCCCCD).
pub fn search_kept(asker_vel: [f32; 3], last_vel: Option<[f32; 3]>, shared: &mut super::parked::Mwc) -> bool {
    let Some(lv) = last_vel else { return false };
    if !vmx_eq3(asker_vel, [0.0; 3]) || !vmx_eq3(lv, [0.0; 3]) {
        return false;
    }
    shared.float() > f32::from_bits(0x3CCC_CCCD)
}

/// The world a rider looks at this frame.
pub struct Env<'a> {
    pub nav: &'a NavSys,
    pub lanes: &'a Lanes,
    pub col: &'a WorldCollider,
    pub others: &'a [Other],
    pub signals: &'a crate::signals::Signals,
    /// the frame's blockers per navsys junction (F-50959's hits, `Vehicles::frame_blockers`)
    pub blockers: &'a hashbrown::HashMap<usize, Blockers>,
    pub clock: f32,
    /// a race is on (byte +0x64251, F-86550: the current race (F-27863), its type 0 - 4 - not a payback - and not over):
    /// no recovery, passing or backing up, and the holds of F-70324 (`race_holds`)
    pub event: bool,
    /// what F-70324's holds read of the player (the tracker +224 / +5888): its flat forward and the junction
    /// ahead of its lane; None without a player's car or junction
    pub player_ahead: Option<((f32, f32), usize)>,
    pub hold: bool,
    /// the frame's parity (the footprint probe's side, manager +0x6423C)
    pub parity: bool,
    /// per car: its race path (behaviour +468) as (referenced +188, fired +189) - a layout item's path
    pub paths: &'a [Option<(bool, bool)>],
    /// the game's shared MWC (`Vehicles::park_rng`, `parked::Mwc`), written back after the riders' step
    pub shared: &'a std::cell::Cell<super::parked::Mwc>,
    /// the obstacle gather's lists (`Vehicles::gather_lists`): the lanes' car lists, the grid's cells
    pub rail_cars: &'a hashbrown::HashMap<usize, Vec<usize>>,
    pub grid: &'a hashbrown::HashMap<(i32, i32), Vec<usize>>,
    /// the dynamic curve rails of the rail pool (F-76414 index 0) the other riders hold now (`holds_pool0`)
    pub pool0: &'a std::cell::Cell<usize>,
    /// the level's objects F-23371's sweeps meet (`RailLevel`)
    pub level: &'a RailLevel<'a>,
}

/// The ambient manager's rail pools (F-76414): index 0 = 256 dynamic curve rails (the backing rail F-54867, the ghost's
/// lane change), index 1 = 256 arc-line-arc rails (recovery, rejoin, passing). A take (F-88792) pops the free list's
/// head and is null when it is empty, i.e. when all 256 are held (tools/rail_pool_reference.py).
pub(crate) const RAIL_POOL: usize = 256;

/// F-88792 with `held` of the pool's rails held: a rail.
pub(crate) fn pool_take(held: usize) -> bool {
    held < RAIL_POOL
}

/// Does the rider hold a rail of pool 0 (its backing rail, or a ghost's lane-change rail)?
pub(crate) fn holds_pool0(r: &Rider) -> bool {
    r.path.as_ref().is_some_and(|p| p.reverse) || r.ghost.as_ref().is_some_and(|g| g.lc.is_some())
}

/// The world of one rider's obstacle gather: the frame's lists, the asker's matrix (its car's; a ghost without one: the
/// rider's place and back axis).
struct EnvGather<'e, 'a> {
    env: &'e Env<'a>,
    me: usize,
    me_rows: ([f32; 3], [f32; 3]),
}

impl GatherWorld for EnvGather<'_, '_> {
    fn vertex_lanes(&self, v: u32) -> &[usize] {
        self.env.nav.records.vertex_lanes.get(v as usize).map_or(&[], |x| x.as_slice())
    }
    fn lane_vertices(&self, lane: usize) -> (u32, u32) {
        let r = &self.env.nav.records;
        (r.first_vertex.get(lane).copied().unwrap_or(u32::MAX), r.last_vertex.get(lane).copied().unwrap_or(u32::MAX))
    }
    fn lane_ends(&self, lane: usize) -> ([f32; 3], [f32; 3]) {
        let c = &self.env.nav.curves[lane];
        let (a, b) = (c.pts.first().copied().unwrap_or(Vec3::ZERO), c.pts.last().copied().unwrap_or(Vec3::ZERO));
        (a.to_array(), b.to_array())
    }
    fn rail_cars(&self, lane: usize) -> &[usize] {
        self.env.rail_cars.get(&lane).map_or(&[], |v| v.as_slice())
    }
    fn grid_cells(&self, asker: usize) -> Vec<&[usize]> {
        let p = self.rows(asker).1;
        let Some((ix, iz)) = CrossGrid::UNBOUNDED.cell(p[0], p[2]) else { return Vec::new() };
        let mut out = Vec::with_capacity(9);
        for z in iz - 1..=iz + 1 {
            for x in ix - 1..=ix + 1 {
                out.push(self.env.grid.get(&(x, z)).map_or(&[][..], |v| v.as_slice()));
            }
        }
        out
    }
    fn kind(&self, o: usize) -> u8 {
        self.env.others.get(o).map_or(2, |x| x.kind)
    }
    fn rows(&self, o: usize) -> ([f32; 3], [f32; 3]) {
        if o == self.me {
            return self.me_rows;
        }
        let x = &self.env.others[o];
        (x.back.to_array(), x.pos.to_array())
    }
}

/// The obstacle search's head and gather (F-47175) before the port's selection (`find_ahead`): the last search kept
/// (`search_kept`: both stand, one draw of the shared MWC), else the candidates of `gather_obstacles` for the rails
/// `g` (current, next, the pass holder's). The result is stored for the next frame's head.
#[allow(clippy::too_many_arguments)]
/// `b_start`: the selection's context +32, the behaviour's +196 = the follower's +164: the start phase the follower took
/// with its next rail +160 (F-63069 hands +192 / +196 out; written by the follower's step at a change of rail from the
/// behaviour's slot 16, F-48618: 0 for an ordinary next lane, the route lane's phase +296 after a race path's join rail,
/// the override's +60 - a lane-change rail's target phase).
#[allow(clippy::too_many_arguments)]
fn search_ahead(r: &mut Rider, env: &Env, me: usize, pos: Vec3, fwd: Vec3, run: Option<RunRail>, lanes: (Option<usize>, Option<usize>), next_start: Option<Vec3>, g: [Option<GatherRail>; 3], b_start: f32) -> Ahead {
    let mine = env.others.get(me);
    let me_rows = mine.map_or(((-fwd).to_array(), pos.to_array()), |o| (o.back.to_array(), o.pos.to_array()));
    let vel = mine.map_or((fwd * r.v).to_array(), |o| o.vel.to_array());
    let last = r.kept.and_then(|(j, _)| env.others.get(j)).map(|o| o.vel.to_array());
    let mut m = env.shared.get();
    let keep = search_kept(vel, last, &mut m);
    env.shared.set(m);
    let ahead = match r.kept {
        Some((j, blocking)) if keep => Ahead { j, d: env.others[j].pos.distance(pos), vt: 0.0, blocking, player_across: false },
        _ => {
            let cands = gather_obstacles(&EnvGather { env, me, me_rows }, me, g[0], g[1], g[2]);
            // (the selection's rails: context +12 / +16 as the gather has them; a run-time rail is the car's own)
            let sel = |x: Option<GatherRail>| x.map(|x| match x {
                GatherRail::Lane(l) => SelRail::Lane(l),
                GatherRail::RunTime => SelRail::RunTime(me as u32),
            });
            let rows3 = mine.map_or((fwd.cross(Vec3::Y).to_array(), me_rows.0, me_rows.1), |o| (o.row0.to_array(), me_rows.0, me_rows.1));
            let size = mine.map_or_else(|| obstacle_size_of(&r.ty), |o| o.size.unwrap_or((2.0 * o.half_wid, 2.0 * o.half_len)));
            let _ = (lanes, next_start);
            find_ahead(env, me, rows3, size, pos, fwd, run, sel(g[0]), sel(g[1]), b_start, &cands)
        }
    };
    r.kept = (ahead.j != usize::MAX).then_some((ahead.j, ahead.blocking));
    // F-10623: a nearest equal to the mark of the frame before (result +12) is no nearest for the controller (the
    // result words stay); then F-37525 (F-33409 calls it after F-10623) marks anew from the nearest (result +0)
    let raw = ahead.j;
    let mut ahead = ahead;
    let key_of = |j: usize| env.others.get(j).filter(|o| o.ambient && o.beh.is_some()).map(|o| o.key);
    if raw != usize::MAX && r.mark.is_some() && key_of(raw) == r.mark {
        ahead = Ahead { j: usize::MAX, d: 0.0, vt: 0.0, blocking: ahead.blocking, player_across: ahead.player_across };
    }
    let our_lane = matches!(g[0], Some(GatherRail::Lane(_)));
    r.mark = None;
    if let (Some(o), Some(mine)) = (env.others.get(raw).filter(|o| o.kind == 2 && o.ambient), mine) {
        // (the cast to the car wrapper and its car; a behaviour - the rider's slot 0 answers 0 -; its nearest +432 our
        // obstacle)
        if let Some((rail, _)) = o.beh.filter(|_| o.nearest.is_some() && o.nearest == Some(r.rng_id())) {
            let other_lane = matches!(rail, Some(SelRail::Lane(_)));
            if mutual_mark(our_lane, (mine.back, mine.pos), other_lane, (o.back, o.pos)) {
                r.mark = Some(o.key);
            }
        }
    }
    ahead
}

/// F-37525's choice for two cars that each have the other as nearest obstacle (the caller's tests: the other an
/// ambient car with a behaviour whose nearest +432 is our obstacle): `our_lane` / `other_lane` the behaviours' +188 a
/// lane (none or a run-time rail: not), the rows 2 and places (slot 1 +32 / +48). M = (P other + P ours, y = 0) x 0.5,
/// D = the vector unit's normalisation of (row 2 other + row 2 ours, y = 0), c = fma(D.y, M.y, fma(M.z, D.z, M.x D.x)),
/// a = D . (P other, y = 0) - c, b = D . (P ours, y = 0) - c (vmsum3fp). Off a lane: always marked; on a lane the other
/// off one: not; else marked unless a > b (a NaN: marked).
pub fn mutual_mark(our_lane: bool, ours: (Vec3, Vec3), other_lane: bool, other: (Vec3, Vec3)) -> bool {
    use crate::physics::exact::fma;
    use crate::tcp_rail::{vadd3, vdot3, vmul3, vnorm};
    let mut m = vadd3(other.1, ours.1);
    m.y = 0.0;
    let m = vmul3(m, f32::from_bits(0x3F00_0000));
    let mut s = vadd3(other.0, ours.0);
    s.y = 0.0;
    let d = vnorm(s);
    let c = fma(d.y, m.y, fma(m.z, d.z, m.x * d.x));
    let flat = |p: Vec3| Vec3::new(p.x, 0.0, p.z);
    let a = vdot3(d, flat(other.1)) - c;
    let b = vdot3(d, flat(ours.1)) - c;
    if !our_lane {
        return true;
    }
    if !other_lane {
        return false;
    }
    !(a > b)
}

/// The level's objects as F-23371's queries meet them this frame (traffic_drive_rule.md 9.18 / 9.19): the city's grid
/// bound (fixed, the identity matrix), every car placed in the level - the player's (0x189) and the other mcCars
/// (racers, 0x185) with their composites as the set-up made them (`ProbeBound::of_car`), the ambient cars (0x1100,
/// parked or on a rail, in the level) with their file's bound (`of_ambient`) at their instance matrix -, the walkers
/// (0x20, `ProbeBound::walker`) at their matrices as the walkers' update of the frame before left them (the manager's
/// frame F-96156 updates the walkers after the cars), and the props: an unhit instance by its file state (1
/// inactive, 2 fixed, 3 not inserted) with its type's composite (`of_prop`), a hit one or a broken-off piece with its
/// entry's (`of_entry`, the absent groups' parts nulled). Each object's instance word is the port's own (nonzero;
/// the answer only asks whether the first record's is set); the own car's is the excluded one (count 1).
pub struct RailLevel<'a> {
    city: Option<&'a crate::physics::camprobe::ProbeBound>,
    loose: Option<&'a crate::physics::loose::LooseWorld>,
    /// the cars' count (instance words `rail_car_inst(0 ..)`)
    ncars: usize,
    built: RailObjs,
}

use std::sync::{Arc, Mutex};

type RailObj = (u32, u32, (u32, u32), [Vec3; 4], Arc<crate::physics::camprobe::ProbeBound>);

/// The objects F-23371 meets that are not the unhit props (those are looked up near each query).
struct RailObjs {
    objs: Vec<RailObj>,
    /// the hit props (their instances)
    moved: hashbrown::HashSet<u32>,
    props: Option<Arc<PropBounds>>,
}

/// Every prop type's composite (`of_prop`) and the largest |centroid| + radius among them (the reach of the near
/// search that finds every instance a query's culler could keep).
struct PropBounds {
    types: Vec<crate::physics::camprobe::ProbeBound>,
    reach: f32,
}

const INST_CITY: u32 = 1;
const INST_CAR: u32 = 0x1000;
const INST_WALKER: u32 = 0x10_0000;
const INST_PROP: u32 = 0x100_0000;
const INST_PIECE: u32 = 0x800_0000;

/// A car's instance word in the level of `RailLevel` (the excluded word of its own sweeps).
pub(crate) fn rail_car_inst(k: usize) -> u32 {
    INST_CAR + k as u32
}

/// The car file's composite (`fragphys::car_composite` of its `.r83`), read once a model.
fn car_file_composite(root: &std::path::Path, model: &str) -> Option<Arc<mcla_formats::rage::fragphys::CarComposite>> {
    type Kept = hashbrown::HashMap<String, Option<Arc<mcla_formats::rage::fragphys::CarComposite>>>;
    static KEPT: std::sync::OnceLock<Mutex<Kept>> = std::sync::OnceLock::new();
    let mut kept = KEPT.get_or_init(|| Mutex::new(hashbrown::HashMap::new())).lock().unwrap();
    kept.entry(model.to_string())
        .or_insert_with(|| {
            let mut files: Vec<_> = std::fs::read_dir(root.join("resources/vehicle").join(model)).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "r83")).collect();
            files.sort();
            files.iter().find_map(|p| mcla_formats::rage::fragphys::car_composite(&crate::assets::load_resource_file(p).ok()?)).map(Arc::new)
        })
        .clone()
}

/// An ambient model's level bound (`ProbeBound::of_ambient` of its `.r155`), read once a model.
fn ambient_level_bound(root: &std::path::Path, model: &str) -> Option<Arc<crate::physics::camprobe::ProbeBound>> {
    type Kept = hashbrown::HashMap<String, Option<Arc<crate::physics::camprobe::ProbeBound>>>;
    static KEPT: std::sync::OnceLock<Mutex<Kept>> = std::sync::OnceLock::new();
    let mut kept = KEPT.get_or_init(|| Mutex::new(hashbrown::HashMap::new())).lock().unwrap();
    kept.entry(model.to_string())
        .or_insert_with(|| {
            let file = std::fs::read_dir(root.join("resources/ambients").join(model)).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "r155"))?;
            let res = crate::assets::load_resource_file(&file).ok()?;
            crate::physics::camprobe::ProbeBound::of_ambient(&mcla_formats::rage::fragphys::ambient_frag(&res)?).map(Arc::new)
        })
        .clone()
}

/// The prop types' composites, made once a prop world.
fn prop_bounds(lw: &crate::physics::loose::LooseWorld) -> Arc<PropBounds> {
    static KEPT: std::sync::OnceLock<Mutex<Option<(usize, Arc<PropBounds>)>>> = std::sync::OnceLock::new();
    let mut kept = KEPT.get_or_init(|| Mutex::new(None)).lock().unwrap();
    let key = lw as *const _ as usize;
    if let Some((k, b)) = kept.as_ref() {
        if *k == key {
            return b.clone();
        }
    }
    let types: Vec<_> = lw.types.iter().map(|t| crate::physics::camprobe::ProbeBound::of_prop(t, &crate::physics::camprobe::game_materials)).collect();
    let reach = types.iter().map(|b| b.centroid.length() + b.radius).fold(0.0f32, f32::max);
    let b = Arc::new(PropBounds { types, reach });
    *kept = Some((key, b.clone()));
    b
}

/// A prop entry's composite (`of_entry`) for its type and absent groups, made once each.
fn entry_bound(lw: &crate::physics::loose::LooseWorld, ty: usize, absent: u64) -> Arc<crate::physics::camprobe::ProbeBound> {
    type Kept = hashbrown::HashMap<(usize, usize, u64), Arc<crate::physics::camprobe::ProbeBound>>;
    static KEPT: std::sync::OnceLock<Mutex<Kept>> = std::sync::OnceLock::new();
    let mut kept = KEPT.get_or_init(|| Mutex::new(hashbrown::HashMap::new())).lock().unwrap();
    let t = &lw.types[ty];
    kept.entry((lw as *const _ as usize, ty, absent))
        .or_insert_with(|| Arc::new(crate::physics::camprobe::ProbeBound::of_entry(t, &crate::physics::camprobe::game_materials, &|c| crate::physics::loose::part_present(t, absent, c))))
        .clone()
}

/// The player's or another mcCar's composite as the level holds it now (`ProbeBound::of_car`): the file's hull with
/// the set-up's reshaped vertices (a motorcycle's as the file has them), the fit's wheels with their tables and
/// their part matrices as the car's last update left them, the composite's box from the fit.
fn car_level_bound(root: &std::path::Path, c: &super::CarInstance) -> Option<crate::physics::camprobe::ProbeBound> {
    use crate::physics::camprobe::{CarWheel, ProbeBound};
    let p = &c.model.params;
    let vt = p.veh.as_ref()?;
    let hull = p.hull.as_ref()?;
    let cc = car_file_composite(root, &c.model.name)?;
    let fit = p.fit_wheels();
    if fit.len() != cc.wheel_tables.len() || cc.matrices.len() != fit.len() + 1 {
        return None;
    }
    let bbox = hull.bbox.map(|(hi, lo)| (lo, hi))?;
    let ws: Vec<_> = fit.iter().map(|x| x.1).collect();
    let fb = crate::physics::vehsim::fit_bound(bbox, &crate::physics::vehsim::fit_parts(&ws, vt.t.steer_axis.map(Vec3::from)));
    let v = |a: [f32; 3]| Vec3::from_array(a);
    let mut mats = vec![cc.matrices[0].map(v)];
    let mut wheels = Vec::with_capacity(fit.len());
    for (j, (i, w)) in fit.iter().enumerate() {
        let part = c.state.veh.as_ref().and_then(|vs| vs.wheels.get(*i)?.part);
        mats.push(part.unwrap_or([Vec3::X, Vec3::Y, Vec3::Z, w.rest]));
        wheels.push(CarWheel { front: w.front, radius: w.radius, width: w.width, materials: cc.wheel_tables[j].clone() });
    }
    let reshaped = (!vt.bike).then_some(hull.verts.as_slice());
    Some(ProbeBound::of_car(&cc.hull, reshaped, &wheels, &mats, &fb))
}

impl<'a> RailLevel<'a> {
    /// The frame's level: `cars` the vehicles' list, `walkers` the walkers' instance matrices, `state` the props' level.
    pub fn new(col: &'a WorldCollider, cars: &[super::CarInstance], root: &std::path::Path, walkers: &[[Vec3; 4]], state: &crate::physics::loose::LooseState) -> RailLevel<'a> {
        let loose = col.loose.as_deref();
        RailLevel { city: col.city.as_deref(), loose, ncars: cars.len(), built: Self::objs(loose, cars, root, walkers, state) }
    }

    fn objs(loose: Option<&crate::physics::loose::LooseWorld>, cars: &[super::CarInstance], root: &std::path::Path, walkers: &[[Vec3; 4]], state: &crate::physics::loose::LooseState) -> RailObjs {
        {
            use crate::physics::camprobe::level_words as lw;
            let mut objs: Vec<RailObj> = Vec::new();
            for (k, c) in cars.iter().enumerate() {
                let m = glam::Mat3::from_quat(c.state.rot);
                if c.rail.is_some() || c.parked.is_some() {
                    let in_level = c.rail.as_ref().map_or(c.park_held, |r| r.in_level);
                    if !in_level {
                        continue;
                    }
                    let Some(b) = ambient_level_bound(root, &c.model.name) else { continue };
                    let state = if c.rail.as_ref().is_some_and(|r| r.state == 4) || c.amb_hit { lw::ACTIVE } else { lw::INACTIVE };
                    objs.push((rail_car_inst(k), state, lw::AMBIENT, instance_matrix(c), b));
                } else if c.placed {
                    let Some(b) = car_level_bound(root, c) else { continue };
                    objs.push((rail_car_inst(k), lw::ACTIVE, if c.is_player { lw::PLAYER_CAR } else { lw::RACER }, [m.x_axis, m.y_axis, m.z_axis, c.state.pos], Arc::new(b)));
                }
            }
            static WALKER: std::sync::OnceLock<Arc<crate::physics::camprobe::ProbeBound>> = std::sync::OnceLock::new();
            let wb = WALKER.get_or_init(|| Arc::new(crate::physics::camprobe::ProbeBound::walker()));
            for (k, m) in walkers.iter().enumerate() {
                objs.push((INST_WALKER + k as u32, lw::INACTIVE, lw::WALKER, *m, wb.clone()));
            }
            let mut moved = hashbrown::HashSet::new();
            let props = loose.map(prop_bounds);
            if let (Some(w), Some(pb)) = (loose, props.as_ref()) {
                for (j, o) in state.objs.iter().enumerate() {
                    if o.veh.is_some() || o.amb.is_some() || o.ty >= w.types.len() {
                        continue;
                    }
                    let b = match o.entry {
                        Some(e) => entry_bound(w, o.ty, e.absent),
                        None => Arc::new(pb.types[o.ty].clone()),
                    };
                    let state = if o.body.is_some() && !o.dormant { lw::ACTIVE } else { lw::INACTIVE };
                    let inst = match o.inst {
                        Some(i) => {
                            moved.insert(i);
                            INST_PROP + i
                        }
                        None => INST_PIECE + j as u32,
                    };
                    objs.push((inst, state, lw::PROP, o.matrix, b));
                }
            }
            RailObjs { objs, moved, props }
        }
    }

    /// The objects a query about the ball (`c`, `r`) can meet: the city, the cars, walkers and hit props, then the unhit
    /// props found near it (every one whose bound's ball can touch it); `f` answers from them.
    pub(crate) fn near(&self, c: Vec3, r: f32, f: &mut dyn FnMut(&[crate::physics::camprobe::LevelObject<'_>]) -> bool) -> bool {
        use crate::physics::camprobe::{level_words as lw, LevelObject};
        let built = &self.built;
        let mut out: Vec<LevelObject<'_>> = Vec::with_capacity(built.objs.len() + 16);
        if let Some(b) = self.city {
            out.push(LevelObject { inst: INST_CITY, state: lw::FIXED, arch_type: lw::CITY.0, arch_include: lw::CITY.1, m: [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO], bound: b });
        }
        for (inst, state, words, m, b) in &built.objs {
            out.push(LevelObject { inst: *inst, state: *state, arch_type: words.0, arch_include: words.1, m: *m, bound: b });
        }
        if let (Some(w), Some(pb)) = (self.loose, built.props.as_ref()) {
            for i in w.near(c, r + pb.reach) {
                let (fi, ty) = &w.instances[i as usize];
                if built.moved.contains(&i) || fi.state == 3 || *ty >= pb.types.len() {
                    continue;
                }
                let state = if fi.state == 2 { lw::FIXED } else { lw::INACTIVE };
                out.push(LevelObject { inst: INST_PROP + i, state, arch_type: lw::PROP.0, arch_include: lw::PROP.1, m: crate::physics::loose::m4(&fi.matrix), bound: &pb.types[*ty] });
            }
        }
        f(&out)
    }
}

/// What a walker's sphere 2 meets (F-10083's input): the first record's object - its archetype's type bits (+16), its
/// matrix (slot 1: row 0, row 2, the place) and its bound's box width and length (x and z of max - min; the original's
/// obstacle slots 4 / 6 - the walker's / prop's obstacle sizes are not decoded, the bound's box stands for them).
#[derive(Clone, Copy, Debug)]
pub struct AheadHit {
    pub ty: u32,
    pub row0: Vec3,
    pub row2: Vec3,
    pub place: Vec3,
    pub w: f32,
    pub l: f32,
}

impl RailLevel<'_> {
    /// A walker's sphere 2 (F-59837 with k = 2: the shape type 512, include 0x103E, the states 7, 12 records) tested
    /// at the next manager frame's start (F-78763 -> F-66549 -> the driver F-49353, `ball_level`), walker `me`'s own
    /// instance excluded; the first record's object as F-10083 reads it.
    pub fn walker_ahead(&self, me: usize, c: Vec3, r: f32) -> Option<AheadHit> {
        use crate::physics::camprobe::{ball_level, LevelFilter};
        let excluded = [INST_WALKER + me as u32];
        let filter = LevelFilter::rail(0x103E, 7, &excluded);
        let mut out = None;
        self.near(c, r, &mut |objs| {
            let l = ball_level(c, r, 512 << crate::tcp_rail::MATERIAL_SHIFT, 12, [0; 3], &filter, objs.iter().copied()).list;
            if let Some(rec) = l.records.first().filter(|x| x.instance != 0) {
                if let Some(o) = objs.iter().find(|o| o.inst == rec.instance) {
                    let ext = o.bound.max - o.bound.min;
                    out = Some(AheadHit { ty: o.arch_type, row0: o.m[0], row2: o.m[2], place: o.m[3], w: ext.x, l: ext.z });
                }
            }
            true
        });
        out
    }
}

/// F-23371's level for one rider's sweep: the queries F-66549 / F-27454 (`tcp_rail::level_query`: the type 640's
/// material flags, the include mask - 0 is 0x812 -, the states 7) against `RailLevel`, the own car excluded.
struct LevelSweep<'e, 'a> {
    level: &'e RailLevel<'a>,
    include: u32,
    excluded: [u32; 1],
}

impl crate::tcp_rail::RailSweepWorld for LevelSweep<'_, '_> {
    fn sphere(&mut self, c: Vec3, r: f32) -> bool {
        use crate::physics::camprobe::{ball_level, LevelFilter};
        let q = crate::tcp_rail::level_query(false, 1, self.include);
        let filter = LevelFilter::rail(q.include, q.states as u32, &self.excluded);
        self.level.near(c, r, &mut |objs| {
            let l = ball_level(c, r, q.material, q.max as usize, [0; 3], &filter, objs.iter().copied()).list;
            l.records.first().is_some_and(|x| x.instance != 0)
        })
    }
    fn capsule(&mut self, a: Vec3, b: Vec3, r: f32, f30: f32) -> bool {
        use crate::physics::camprobe::{capsule_level, LevelFilter};
        let q = crate::tcp_rail::level_query(true, 1, self.include);
        let filter = LevelFilter::rail(q.include, q.states as u32, &self.excluded);
        let f30_high = ((f30 as f64).to_bits() >> 32) as u32;
        self.level.near((a + b) * 0.5, a.distance(b) * 0.5 + r, &mut |objs| {
            let l = capsule_level(a, b, r, q.material, q.max as usize, f30_high, &filter, objs.iter().copied()).list;
            l.records.first().is_some_and(|x| x.instance != 0)
        })
    }
}

/// F-23371 for one rider's rail: `width` the request's +0 (the car's own width), from phase 0 for the whole rail,
/// `lift` (+32), the include `mask` (+16); the excluded word the rider's car (`me`; none for a ghost without one).
/// True when it hits.
fn rail_blocked(env: &Env, me: usize, q: &TcpRail, width: f32, mask: u32, lift: f32) -> bool {
    let excl = if me < env.level.ncars { rail_car_inst(me) } else { 0 };
    let mut world = LevelSweep { level: env.level, include: if mask == 0 { 0x812 } else { mask }, excluded: [excl] };
    let mut left = -1.0;
    q.sweep(width, 0.0, &mut left, lift, &mut world)
}

/// The footprint probe (F-39379, states 6 and 7; F-82779 in state 8): the level's segment query F-57005 (r4 = 1,
/// `camprobe::segment_level`, traffic_drive_rule.md 9.20) from start to end of a segment 0.75 m up (row 1), 0.675 of
/// the half width to one side (row 0; + on an odd frame, - on an even one), from +0.9 to -0.9 of the half length along
/// row 2 (the back) - the madds unfused as the vector unit does them; the block's material word 512 (shifted by the
/// material manager's flag shift), the include mask 14366, the states 7, the own car excluded. A hit is the record's
/// instance word set. The rows and the place are the car's instance as the frame started (`Other::inst`). F-39379
/// answers 0 at once for a cab or a trailer (the type's +111 / +112: the caller's `semi`); F-82779 has no such test.
/// (The other excluded word of a car with +500 & 0x20 is not ported: the port's riders have no cab proxy.)
/// The footprint probe's segment (F-39379 / F-82779 before F-57005): from the instance's rows and place, the half
/// length and width (F-88657 slots 24 / 16, halved) and the frame's parity. Equal bit for bit to 162 probes recorded in
/// the running original (`probe_segment_matches_the_recording`).
fn probe_segment(inst: &[Vec3; 4], hl: f32, hw: f32, odd: bool) -> (Vec3, Vec3) {
    let [row0, row1, row2, pos] = *inst;
    let side = if odd { 0.675 * hw } else { -(0.675 * hw) };
    let base = row1 * 0.75 + pos;
    let base = row0 * side + base;
    let along = 0.9 * hl;
    (row2 * along + base, row2 * -along + base)
}

fn probe_hit(env: &Env, me: usize, inst: &[Vec3; 4], hl: f32, hw: f32) -> bool {
    use crate::physics::camprobe::{segment_level, LevelFilter};
    // (F-33765's rows and F-58860's place: the instance as the frame started - the state check F-94409 runs first in
    // the behaviour frame F-87488 and the new pose reaches the instance only after it, through slot 4 F-74063 in the
    // car update; F-82779 in state 8 follows and moves before its probe but the instance is still the frame's start)
    let (a, b) = probe_segment(inst, hl, hw, env.parity);
    let excluded = [if me < env.level.ncars { rail_car_inst(me) } else { 0 }];
    let filter = LevelFilter::rail(14366, 7, &excluded);
    let mask = 512u32 << crate::tcp_rail::MATERIAL_SHIFT;
    env.level.near((a + b) * 0.5, a.distance(b) * 0.5, &mut |objs| segment_level(a, b, mask, &filter, objs.iter().copied()).list.records[0].instance != 0)
}

/// The rail onto a lane (F-26466) for a request: the lanes within 25 m (the port's search, at most 48; the entry's
/// phase t0 the lane's point nearest the start), the lane's curve as the target (slot 8), the sweep F-23371 (`rail_blocked`: the own width, mask 2106).
fn recovery_rail(r: &mut Rider, env: &Env, me: usize, req: &RailReq) -> Option<Path> {
    let (nav, lanes) = (env.nav, env.lanes);
    let start = req.p;
    let mut cand: Vec<(usize, f32, bool)> = Vec::new();
    for k in nav.links_near(start, 25.0) {
        let l = &nav.links[k];
        let has_curve = nav.curves.get(l.lane).is_some_and(|c| c.pts.len() >= 2);
        if has_curve && l.point_at(l.project(start)).distance(start) <= 25.0 && !cand.iter().any(|c| c.0 == l.lane) && lanes.len.get(l.lane).is_some_and(|&x| x > 0.5) {
            cand.push((l.lane, lanes.project(nav, l.lane, start) / lanes.len[l.lane], true));
            if cand.len() == 48 {
                break;
            }
        }
    }
    // (F-23371's request: the own width, mask 2106, no lift; the own car excluded - traffic_drive_rule.md 9.18)
    let w_own = own_width(&r.ty);
    let (min_turn, pivot) = (r.ty.min_turn, r.ty.pivot_turn);
    // (F-26466's draws: the car's MWC)
    let mut rng = r.mwc;
    let pick = rail_to_lane(req, min_turn, pivot, &cand, &mut || rng.next(), &mut |lane, ph| nav.curves[lane].eval_pair(ph), &mut |q| !rail_blocked(env, me, q, w_own, 2106, 0.0));
    r.mwc = rng;
    let pick = pick?;
    Some(Path::tcp(pick.rail, pick.speed, Some((pick.lane, pick.phase))))
}

/// The recovery try (F-15490), in state 5 and every frame in state 6: not online, not a semi, not upside down; the
/// standing timer (ready past 2 s); a try when the car's draw is at most 1/30. The request (F-26749): from the
/// follower's rail at its phase (slot 8: the point, the derivative with y = 0 made unit), else the car's place and
/// forward (F-58860 and 0 - row 2, F-50349), plus the offset argument (the zero vector: a -0 turns +0); the car's speed,
/// 140 degrees (the image's 0x401C61AA). A new rail replaces one being ridden only when it is shorter by a metre
/// where more than 10 m are left (F-99509: len + 1 < len - len x phase). A rail taken zeroes the standing timer.
fn try_recovery(r: &mut Rider, env: &Env, me: usize, pos: Vec3, fwd: Vec3, upright: bool, dt: f32) -> bool {
    if env.event || r.ty.trailer || (r.ty.cab && r.partner.is_some()) || !upright {
        return false;
    }
    if r.v == 0.0 {
        r.stand += dt;
    } else {
        r.stand = 0.0;
    }
    let ready = r.stand > 2.0;
    // (F-15490: one draw of the car's MWC, U x 2^-23 > 1/30 (0x3D088889): no try)
    if u23(&mut r.mwc) * f32::from_bits(0x3400_0000) > f32::from_bits(0x3D08_8889) {
        return false;
    }
    let req = recovery_req(r.path.as_ref(), pos, fwd, r.v, ready);
    let Some(p) = recovery_rail(r, env, me, &req) else { return false };
    if let Some(cur) = r.path.as_ref() {
        let left = cur.left();
        if left > 10.0 && !(p.len() + 1.0 < left) {
            return false;
        }
    }
    r.take_path(p, 6);
    r.then = None;
    r.stand = 0.0;
    true
}

/// The recovery request (F-26749): from the rail at its phase (slot 8: the point and the derivative, y = 0, made unit)
/// or the car's place and `fwd` (0 - row 2), the place plus the zero offset.
fn recovery_req(path: Option<&Path>, pos: Vec3, fwd: Vec3, v: f32, ready: bool) -> RailReq {
    let (p, h0) = match path {
        Some(pp) => {
            let (q, d) = pp.eval(pp.t);
            (q, crate::tcp_rail::vnorm(Vec3::new(d.x, 0.0, d.z)))
        }
        None => (pos, fwd),
    };
    RailReq { p: crate::tcp_rail::vadd3(p, Vec3::ZERO), h0, v, limit: RAIL_TURN, ready }
}

/// The rejoin request (F-89026): from the passing rail's end T (slot 13) along its unit derivative at phase 1 (slot 10,
/// not flattened), 0.7 x its speed (slot 17) at least 4.4704 m/s (fsel), never ready, 140 degrees.
fn rejoin_req(q: &TcpRail, speed: f32) -> RailReq {
    let f13 = speed * 0.7;
    let v = if f13 - 4.4704 >= 0.0 { f13 } else { 4.4704 };
    RailReq { p: q.t, h0: crate::tcp_rail::vnorm(q.tangent(1.0)), v, limit: RAIL_TURN, ready: false }
}

/// The obstacle's four corners as the passing rail's builder (F-92334) makes them: its place (slot 1's matrix +48)
/// + row 0 x ((width x s) x 0.5) + row 2 x ((length x s) x 0.5), width and length its slots 4 and 6, the signs
/// (1, 1), (-1, 1), (-1, -1), (1, -1) (its table; s through the pair of fsel: 1, 0 or -1), each a vmaddfp.
pub(crate) fn pass_corners(place: Vec3, row0: Vec3, row2: Vec3, w: f32, l: f32) -> [Vec3; 4] {
    [(1.0f32, 1.0f32), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)].map(|(sx, sz)| {
        let c = crate::tcp_rail::vmadd3(row0, (w * sx) * 0.5, place);
        crate::tcp_rail::vmadd3(row2, (l * sz) * 0.5, c)
    })
}

/// The passing rail's request (F-10687 -> F-92334): the start and heading (the follower's rail at its phase as the
/// recovery request F-26749 makes them, plus the request's offset, the zero vector), the obstacle's matrix (row 0, row 2,
/// place) and its bound's width and length, the car's own width (its obstacle's slot 4, F-88657 -> F-46284), the
/// clearance (+28), the radius (F-36195 at the car's speed; the pivot radius when ready, +44 bit 0x10), the largest turn
/// (+36: 0x4011361E) and the flags (+44): 0x80 the left rails, 0x40 the right ones, 0x20 the lane test. Traffic's
/// F-10687 sets 0x80, 0x40 and 0x20; the walkers' F-79428 leaves 0x20 clear.
pub(crate) struct PassReq {
    pub p: Vec3,
    pub h0: Vec3,
    pub row0: Vec3,
    pub row2: Vec3,
    pub place: Vec3,
    pub w: f32,
    pub l: f32,
    pub w_own: f32,
    pub c: f32,
    pub radius: f32,
    pub gate: f32,
    pub flags: u8,
}

/// A lane the lane query F-56141 reports near a point (its 48-byte entry: +16 the lane's unit tangent there, +36 the
/// squared distance), `skip` when the lane's owner (+8) or the owner's parent (owner +8) has flag 0x4 (word +4).
pub(crate) struct LaneNear {
    pub tangent: Vec3,
    pub d2: f32,
    pub skip: bool,
}

/// F-50817: which way the lanes run at a rail's phase `ph`: the lanes within `radius` of the rail's point there
/// (F-56929; the query F-56141, 64 at most), of those not skipped the first of the least squared distance (strictly
/// below from f32::MAX); none: -1; else its tangent dotted with the rail's tangent there (F-28568; x, then y and z
/// fused on) above 0: 0 (the same way), else 1.
pub(crate) fn lane_side(q: &TcpRail, ph: f32, radius: f32, lanes: &mut dyn FnMut(Vec3, f32) -> Vec<LaneNear>) -> i32 {
    use crate::physics::exact::fma;
    let at = q.point(ph);
    let d = q.tangent(ph);
    let mut best = f32::MAX;
    let mut dir = None;
    for e in lanes(at, radius).iter().take(64) {
        if !e.skip && e.d2 < best {
            best = e.d2;
            dir = Some(e.tangent);
        }
    }
    let Some(t) = dir else { return -1 };
    if fma(d.z, t.z, fma(d.y, t.y, d.x * t.x)) > 0.0 { 0 } else { 1 }
}

/// F-92334, the passing rail's geometry and pick. The obstacle's corners (`pass_corners`); d0 = unit(O - P with y = O.y);
/// the corner of least and of greatest cross-y (d0 x unit(corner - P)) - the least strictly below f32::MAX, the greatest
/// strictly above -f32::MAX, each the first found, corner 0 when none; the targets `unit((d.z, 0, -d.x)) x fma(W_own,
/// 0.5, c) + corner` (left, d = corner - P) and `(-d.z, 0, d.x)` (right); H1 = unit(T - P with y = 0); four rails a
/// target (senses 00, 01, 10, 11; the left four when flag 0x80, the right four when 0x40). Of these in turn a valid rail
/// whose largest turn is not above the gate, not longer than both bests and f32::MAX, swept clear (`sweep`: true when it
/// hits; F-23371 with the own width): with the lane test (0x20) F-50817 at phase 1 within 2 x W_own: no lane (-1) - not
/// over the pavement at T (`pavement`, F-50927), not longer than best A, and the probe from T + 1 m to T - 1 m in y hits
/// (`ground`, F-57005) - is best A; a lane the same way (0) and not longer than best B is best B; the other way (1) is
/// no pick. Without the lane test a rail not longer than best A is best A. Ties replace. The pick: A, else B.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pass_pick(
    q: &PassReq,
    sweep: &mut dyn FnMut(&TcpRail) -> bool,
    lanes: &mut dyn FnMut(Vec3, f32) -> Vec<LaneNear>,
    pavement: &mut dyn FnMut(Vec3) -> bool,
    ground: &mut dyn FnMut(Vec3, Vec3) -> bool,
) -> Option<TcpRail> {
    use crate::physics::exact::fma;
    use crate::tcp_rail::{build, cross_y, vdiff, vmadd3, vnorm};
    let p = q.p;
    let corners = pass_corners(q.place, q.row0, q.row2, q.w, q.l);
    let mut d0 = vdiff(q.place, p);
    d0.y = q.place.y;
    let d0 = vnorm(d0);
    let (mut lo, mut hi) = (f32::MAX, -f32::MAX);
    let (mut li, mut ri) = (0usize, 0usize);
    for (k, &c) in corners.iter().enumerate() {
        let y = cross_y(d0, vnorm(vdiff(c, p)));
        if y < lo {
            lo = y;
            li = k;
        }
        if y > hi {
            hi = y;
            ri = k;
        }
    }
    let off = fma(q.w_own, 0.5, q.c);
    let push = |c: Vec3, left: bool| {
        let d = vdiff(c, p);
        let n = if left { Vec3::new(d.z, 0.0, -d.x) } else { Vec3::new(-d.z, 0.0, d.x) };
        vmadd3(vnorm(n), off, c)
    };
    let mut rails: Vec<TcpRail> = Vec::with_capacity(8);
    for (bit, t) in [(0x80u8, push(corners[li], true)), (0x40, push(corners[ri], false))] {
        if q.flags & bit == 0 {
            continue;
        }
        let mut dir = vdiff(t, p);
        dir.y = 0.0;
        let h1 = vnorm(dir);
        for (s0, s1) in [(false, false), (false, true), (true, false), (true, true)] {
            rails.push(build(p, q.h0, t, h1, q.radius, s0, s1));
        }
    }
    let (mut a_len, mut b_len) = (f32::MAX, f32::MAX);
    let (mut a, mut b): (Option<TcpRail>, Option<TcpRail>) = (None, None);
    for r in rails {
        if !r.valid() || r.max_turn() > q.gate {
            continue;
        }
        let len = r.len();
        if len > a_len && len > b_len && len > f32::MAX {
            continue;
        }
        if sweep(&r) {
            continue;
        }
        if q.flags & 0x20 == 0 {
            if len > a_len {
                continue;
            }
            a_len = len;
            a = Some(r);
            continue;
        }
        match lane_side(&r, 1.0, q.w_own * 2.0, lanes) {
            -1 => {
                if pavement(r.t) || len > a_len {
                    continue;
                }
                let (top, bottom) = (Vec3::new(r.t.x, r.t.y + 1.0, r.t.z), Vec3::new(r.t.x, r.t.y - 1.0, r.t.z));
                if !ground(top, bottom) {
                    continue;
                }
                a_len = len;
                a = Some(r);
            }
            0 => {
                if len > b_len {
                    continue;
                }
                b_len = len;
                b = Some(r);
            }
            _ => {}
        }
    }
    a.or(b)
}

/// The car's own width as its obstacle's slot 4 gives it (F-88657 -> F-46284): its bound's box width, 3 m when it has
/// no bound.
fn own_width(ty: &AmbType) -> f32 {
    if ty.width > 0.0 { ty.width } else { 3.0 }
}

/// The lanes the lane query F-56141 reports within `radius` of `at` (the port's search, as the recovery rail's: the
/// lanes' segments near the point, each lane once by its nearest point; the tangent the lane's there, made unit).
fn lanes_near(env: &Env, at: Vec3, radius: f32) -> Vec<LaneNear> {
    let nav = env.nav;
    let mut out: Vec<(usize, LaneNear)> = Vec::new();
    for k in nav.links_near(at, radius) {
        let l = &nav.links[k];
        let t = l.project(at);
        let d2 = l.point_at(t).distance_squared(at);
        if d2 > radius * radius {
            continue;
        }
        let e = LaneNear { tangent: l.tangent_at(t), d2, skip: (l.owner_flags | l.parent_flags) & 0x4 != 0 };
        match out.iter_mut().find(|x| x.0 == l.lane) {
            Some(x) if x.1.d2 > d2 => x.1 = e,
            Some(_) => {}
            None => out.push((l.lane, e)),
        }
    }
    out.into_iter().map(|x| x.1).collect()
}

/// The passing rail (F-28706 -> F-10687 -> F-92334) at clearance `c`, then the rejoin (F-89026 -> F-26466). The start
/// and heading as the recovery request's (the follower's lane at its phase); the request as traffic's F-10687 makes it
/// (left and right rails, the lane test, 130 degrees); the pick `pass_pick` with the port's world: the sweep its
/// F-23371 against the level (`rail_blocked`: the own width, mask -1), the lanes `lanes_near`, the pavement the navigation mesh's ai_sidewalk
/// polygons (F-50927's filter at the ambient manager +0x617F0 (F-24607), its slot 1 F-27305: material bit 0x1), the probe the
/// city's bound. Speed `max(0.7 v, 4.4704)` (fsel), 1.1176 when ready.
#[allow(clippy::too_many_arguments)]
fn passing_rail(r: &mut Rider, env: &Env, me: usize, p: Vec3, h0: Vec3, obst: usize, c: f32, ready: bool) -> Option<(Path, Path)> {
    let o = &env.others[obst];
    let w_own = own_width(&r.ty);
    let req = PassReq {
        p: crate::tcp_rail::vadd3(p, Vec3::ZERO),
        h0,
        row0: o.right,
        row2: o.back,
        place: o.pos,
        w: o.half_wid * 2.0,
        l: o.half_len * 2.0,
        w_own,
        c,
        radius: turn_radius(&r.ty, r.v, ready),
        gate: PASS_TURN,
        flags: 0xE0 | (ready as u8) << 4,
    };
    let rail = pass_pick(
        &req,
        // (F-23371's request: the own width, the request's mask -1 (+40) and its +32 word, 0 for cars)
        &mut |q| rail_blocked(env, me, q, w_own, u32::MAX, 0.0),
        &mut |at, rad| lanes_near(env, at, rad),
        &mut |t| crate::offroad::mesh().is_some_and(|m| m.on_flag(t, 0x1)),
        &mut |top, bottom| env.col.probe_front(top, Vec3::NEG_Y, top.y - bottom.y).is_some(),
    )?;
    let speed = if ready {
        1.1176
    } else {
        let f12 = r.v * 0.7;
        if f12 - 4.4704 >= 0.0 { f12 } else { 4.4704 }
    };
    let mut pass = Path::tcp(rail, speed, None);
    // (F-92334's attach: the owner is the current lane's road, F-86691 -> F-75805's cast; the rejoin rail has none)
    pass.owner = env.nav.links.get(r.link).and_then(|l| env.nav.lane_road(l.lane));
    let back = rejoin_rail(r, env, me, &pass)?;
    Some((pass, back))
}

/// The rejoin rail's request (F-89026): from the passing rail's end T (slot 13, record +112) along its unit derivative
/// at phase 1, at max(0.7 x its speed, 4.4704) - the radius at that speed, never the pivot one -, 140 degrees.
fn rejoin_rail(r: &mut Rider, env: &Env, me: usize, pass: &Path) -> Option<Path> {
    let Geom::Tcp(q) = &pass.geom else { return None };
    let req = rejoin_req(q, pass.speed);
    recovery_rail(r, env, me, &req)
}

/// The backing-up rail (F-56052 -> F-54867): a pool curve (index 0, four control points, knots 0 0 0 1 1 1) from
/// E = row 2 x 2 + the car's place to the car: E, (M + E) / 2, (M + P) / 2, P with M = (E + P) / 2 (the vector
/// unit's products and sums), 4 m/s, attached at phase 1 ridden back.
pub(crate) fn backing_rail(place: Vec3, row2: Vec3) -> Path {
    use crate::tcp_rail::{vadd3, vmadd3, vmul3};
    let e = vmadd3(row2, 2.0, place);
    let m = vmul3(vadd3(e, place), 0.5);
    let cps = [e, vmul3(vadd3(m, e), 0.5), vmul3(vadd3(m, place), 0.5), place];
    let c = path_curve(&cps);
    let ds = set_rail_speed(&c, 1.0);
    Path { geom: Geom::Curve(c), speed: 4.0, end: None, reverse: true, t: 1.0, ds, owner: None }
}

/// The turn signals: the render flags' bits 0x20 (F-41231, the lamp state `s[2]`) and 0x10 (F-81289, `s[3]`), for
/// a car at `pos` (F-58860: its matrix's place) whose blink (car +3572) is `blink` and offset +3580 `blink_on`. Both
/// are off past `blink_on` + 0.375 in the blink, and without a behaviour (a trailer). The rail tests read the owner
/// of the follower's rail (rail +8, its flags +4: the port's `owner_flags`): 0x20 is lit on a rail whose owner has
/// 0x10, or when the next rail's owner has 0x10 and the car is within 100 m (squared 10000) of the current rail's
/// end (rail slot 13, F-93886: the curve's last control point); 0x10 likewise with 0x20, except that on a rail
/// whose owner has 0x20 and +0x10 word 0 (F-29760) it is off past phase 0.5. A lane change rail (F-68395) lights
/// one side by itself; the original's traffic makes none offline (doc), so there is no such case here. Only a car
/// on a lane (state 1) is tested: the dynamic rails (recovery, passing, a race path's rails) are built with no
/// owner (+8 = 0 in their ctors F-33069 / F-36658, their reset slot 23 leaves it), so only the next-rail test can
/// light them, and what the follower's next rail is on them is not decoded (doc).
pub fn signals(nav: &NavSys, r: &Rider, pos: Vec3, blink: f32, blink_on: f32) -> (bool, bool) {
    if blink > blink_on + 0.375 || r.state != 1 {
        return (false, false);
    }
    let cur = &nav.links[r.link];
    let near = || {
        let Some(end) = nav.curves.get(cur.lane).and_then(|c| c.pts.last()) else { return false };
        let (dx, dy, dz) = (pos.x - end.x, pos.y - end.y, pos.z - end.z);
        dx.mul_add(dx, dz.mul_add(dz, dy * dy)) < 10000.0
    };
    let next = r.next.map(|n| nav.links[n].owner_flags).unwrap_or(0);
    let a = cur.owner_flags & 0x10 != 0 || (next & 0x10 != 0 && near());
    let b = if cur.owner_flags & 0x20 != 0 {
        !(cur.owner_w10 == 0 && r.t > 0.5)
    } else {
        next & 0x20 != 0 && near()
    };
    (a, b)
}

/// The car ahead's speed as F-56024 takes its velocity's length: y^2, then x and z fused on, in
/// single precision.
fn obst_speed(u: Vec3) -> f32 {
    u.z.mul_add(u.z, u.x.mul_add(u.x, u.y * u.y)).sqrt()
}

/// The brake lamp's flag 0x40 (F-56024; F-46062 writes the flags each frame): off for a car
/// without a behaviour (a trailer) or whose race path has bParked (+164; behaviour slot 18,
/// F-86955); on when the speed +412 is 0; when the entry in charge is the stop line (+420 == 1 -
/// its speed is always 0, so +416 is, F-30960) and the speed is under 6.7056 m/s; when the physics
/// has the car active (F-17857); or when it is faster than 1.5 x the car ahead's speed (behaviour
/// +432, F-59415); else off.
pub fn brake_lamp(r: &Rider) -> bool {
    if r.state == 0 || r.quiet {
        return false;
    }
    r.v == 0.0 || (r.slot == Some(1) && r.v < 6.7056) || r.active || r.obst_v.is_some_and(|o| r.v > o * 1.5)
}

/// A constraint slot's thresholds as F-30960 hands them to F-49294 (its stack copy of the controller's tune): the
/// decel band, MinDecel, this frame's MaxDecel, the accel band, MinAccel, MaxAccel + the car's bonus.
#[derive(Clone, Copy, Debug)]
pub struct SlotTune {
    pub band_d: f32,
    pub min_d: f32,
    pub max_d: f32,
    pub band_a: f32,
    pub min_a: f32,
    pub up: f32,
}

/// F-49294, a constraint slot's update for an active entry (its needed acceleration `a`, F-20809's result, and its
/// distance less the margin): the hysteresis by the sign of `a` (a NaN takes the accel side) - on, off below MinAccel -
/// band / 2 (a >= 0) or when -a < MinDecel - band / 2; off, on above MinAccel + band / 2 or when -a > MinDecel + band / 2 -;
/// on whenever the gap is not > 0; then at most `up`, and at least -MaxDecel, or for the player (the obstacle's type 9)
/// beyond 1.25 MaxDecel -1.5 MaxDecel. Returns the on bit, the slot's acceleration and its gap (fsel: 0 when -gap >= 0).
pub fn slot_update(on: bool, a: f32, gap: f32, player: bool, t: &SlotTune) -> (bool, f32, f32) {
    let mut on = on;
    let neg = a < 0.0;
    if on {
        if !neg {
            if a < (-t.band_a).mul_add(0.5, t.min_a) {
                on = false;
            }
        } else if -a < (-t.band_d).mul_add(0.5, t.min_d) {
            on = false;
        }
    } else if !neg {
        if a > t.band_a.mul_add(0.5, t.min_a) {
            on = true;
        }
    } else if -a > t.band_d.mul_add(0.5, t.min_d) {
        on = true;
    }
    if !(gap > 0.0) {
        on = true;
    }
    let mut a = if a > t.up { t.up } else { a };
    if player && -a > t.max_d * 1.25 {
        a = -(t.max_d * 1.5);
    } else if -a > t.max_d {
        a = -t.max_d;
    }
    (on, a, if -gap >= 0.0 { 0.0 } else { gap })
}

/// The speed controller (F-33409 -> F-30960): the constraint entries (car ahead,
/// stop line, slower next lane), each with its hysteresis; the most negative wins, else the
/// positive one with the smallest gap, else free driving towards `want`. Returns the
/// acceleration and the entry chosen.
#[allow(clippy::too_many_arguments)]
fn control(r: &mut Rider, v: f32, want: f32, half_len: f32, ahead: &Ahead, others: &[Other], stop: Option<(usize, f32)>, slower: Option<(f32, f32)>, dest: Option<(f32, f32)>, dt: f32) -> (f32, Option<usize>) {
    let ty = r.ty;
    let max_decel = if ahead.player_across { ty.max_decel * 3.0 } else { ty.max_decel };
    let up = ty.max_accel + r.bonus;
    let mph = v / MPH;
    let mut entries: [Option<(f32, f32, bool)>; 4] = [None; 4];
    // 0: the car ahead (F-10623): centre to centre, half lengths and the standing gap
    // stretched with speed
    let obst = (ahead.j != usize::MAX).then_some(ahead.j);
    if obst != r.obst_id {
        r.on[0] = false;
        r.obst_id = obst;
    }
    r.obst_v = obst.map(|j| obst_speed(others[j].vel));
    if let Some(j) = obst {
        let o = &others[j];
        let margin = half_len + o.half_len + r.gap * (1.0 + 2.0 * (mph / 70.0).min(1.0));
        entries[0] = Some((needed(v, ahead.vt, ahead.d, margin, up, max_decel), ahead.d - margin, o.player));
        r.ahead = (ahead.d, margin);
    }
    // 1: the stop line while the light says stop (F-83573), half our length short of it
    let sid = stop.map(|s| s.0);
    if sid != r.stop_id {
        r.on[1] = false;
        r.stop_id = sid;
    }
    if let Some((_, d)) = stop {
        entries[1] = Some((needed(v, 0.0, d, half_len, up, max_decel), d - half_len, false));
    }
    // 2: the next lane slower than we go: by its start (the lane's last vertex, straight on)
    if let Some((vt, d)) = slower {
        entries[2] = Some((needed(v, vt, d, 0.0, up, max_decel), d, false));
    }
    // 3: the cruising ghost's destination speed (F-25114: reach it at the destination point, margin 0)
    if let Some((vt, d)) = dest {
        entries[3] = Some((needed(v, vt, d, 0.0, up, max_decel), d, false));
    }
    let mut on = [r.on[0], r.on[1], r.on[2], r.ghost.as_ref().is_some_and(|g| g.on3)];
    let t = SlotTune { band_d: ty.min_decel_band, min_d: ty.min_decel, max_d: max_decel, band_a: ty.min_accel_band, min_a: ty.min_accel, up };
    let (a, chosen) = pick(&entries, &mut on, &t, want, v, dt, &mut r.waited, r.wait);
    r.on = [on[0], on[1], on[2]];
    if let Some(g) = r.ghost.as_mut() {
        g.on3 = on[3];
    }
    (a, chosen)
}

/// F-30960 from its slot updates on: each active entry through F-49294 ([`slot_update`]), the pick, free driving and the
/// start delay (+440 `waited` against +444 `wait`, counted while the speed +412 is 0). `entries`: the needed acceleration,
/// the distance less the margin and whether the obstacle is the player. Returns the acceleration and the entry chosen
/// (+420; none -1).
#[allow(clippy::too_many_arguments)]
pub fn pick(entries: &[Option<(f32, f32, bool)>; 4], on: &mut [bool; 4], t: &SlotTune, want: f32, v: f32, dt: f32, waited: &mut f32, wait: f32) -> (f32, Option<usize>) {
    // (F-30960's pick over the four slots: a > 0 the smallest stored gap - slot 3 keeps the gap it beats -, a <= 0 the
    // least; both start at FLT_MAX and take only a strictly smaller value, so a NaN never wins)
    let (mut pos_gap, mut pos_a, mut pos_k) = (f32::MAX, f32::MAX, None);
    let (mut neg_a, mut neg_k) = (f32::MAX, None);
    for k in 0..4 {
        // (an inactive entry clears only the slot's active bit: its on bit stays for when the entry comes back)
        let Some((a, dist_less_margin, player)) = entries[k] else {
            continue;
        };
        let (o, a, gap) = slot_update(on[k], a, dist_less_margin, player, t);
        on[k] = o;
        if !o {
            continue;
        }
        if a > 0.0 {
            if gap < pos_gap {
                if k < 3 {
                    pos_gap = gap;
                }
                (pos_a, pos_k) = (a, Some(k));
            }
        } else if a < neg_a {
            (neg_a, neg_k) = (a, Some(k));
        }
    }
    let (mut a, chosen) = if pos_a == f32::MAX && neg_a == f32::MAX {
        // free driving: F-54830 (want - v) / the controller's +156 (1.0), at most (MaxAccel + bonus + MinAccel) / 2, at
        // least -(MaxDecel + MinDecel) / 2 with this frame's MaxDecel (x 3 with the player across)
        let mut a = (want - v) / 1.0;
        let (upf, dnf) = ((t.up + t.min_a) * 0.5, (t.max_d + t.min_d) * 0.5);
        if a > upf {
            a = upf;
        }
        if -a > dnf {
            a = -dnf;
        }
        (a, None)
    } else if pos_a < neg_a {
        (pos_a, pos_k)
    } else {
        (neg_a, neg_k)
    };
    // moving off from standing: the start delay, counted while the controller asks to go
    if v == 0.0 {
        if !(a > 0.0) {
            *waited = 0.0;
        } else {
            *waited += dt;
            if *waited < wait {
                a = 0.0;
            }
        }
    }
    (a, chosen)
}

/// The follower's rail for F-83943 (+156): its word +4, the follower's byte +152 and phase +176, the arc-line-arc length
/// (F-62306, `tcp_rail::len`) and the points the rail's slots give: slot 7 at the phase, slots 12 / 13 its start / end,
/// the curve's (slot 15) last vertex (its +28 index through the curve's slot 2).
#[derive(Clone, Copy, Debug)]
pub struct LeftRail {
    pub flags: u32,
    pub byte: u8,
    pub phase: f32,
    pub len: f32,
    pub point: Vec3,
    pub start: Vec3,
    pub end: Vec3,
    pub last: Vec3,
}

/// F-83943, the distance left on the follower's rail: none 100000 (0x47C35000); a curve rail (word +4 bit 0x1000) the
/// length x (1 - phase); a reverse rail (byte +152 with 0x80 and 0x40) |end - start| x phase; else (a
/// lane) |point - last vertex|. Each |d| = sqrt(fma(x, x, fma(z, z, y y))).
pub fn distance_left(rail: Option<&LeftRail>) -> f32 {
    use crate::physics::exact::fma;
    let n = |d: Vec3| fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)).sqrt();
    let Some(r) = rail else { return f32::from_bits(0x47C3_5000) };
    if r.flags & 0x1000 != 0 {
        r.len * (1.0 - r.phase)
    } else if r.byte & 0x80 != 0 && r.byte & 0x40 != 0 {
        n(r.end - r.start) * r.phase
    } else {
        n(r.point - r.last)
    }
}

/// The integrator's speed (F-58614's first part, `speed_step`): the cap is fMaxSpeed (a race path's car: its
/// fPathMaxSpeed, behaviour +244 with +256 bit 0x40, put in `ty.max_speed` by `path_rider`). Its offset timer is
/// `rider_offset`, at the end of the car's frame.
fn integrate(r: &mut Rider, a: f32, dt: f32) {
    r.a = a;
    r.v = speed_step(r.v, a, dt, r.ty.max_speed, r.ty.max_accel);
}

/// F-58614's offset timer for an ordinary rider (`offset_step` on the car's MWC; no offset for a car of the proxy pool,
/// car +500 bit 0x20). The original runs F-58614 after the behaviour's update (F-50624's order: behaviour slot 3, the
/// asked acceleration, then F-58614), so its draws follow the frame's recovery / passing draws of the same MWC. The
/// cruising ghost's is in `ghost_integrate`.
fn rider_offset(r: &mut Rider, clock: f32) {
    if r.ghost.is_some() {
        return;
    }
    let mut s = CarSpeed { v: r.v, off: r.off, timer: r.off_timer, born: r.born, rng: r.mwc };
    offset_step(&mut s, clock, r.ty.proxy);
    (r.off, r.off_timer, r.mwc) = (s.off, s.timer, s.rng);
}

/// One frame of a rider on its lane (state 1; F-87488 / F-30960 / F-58614),
/// with the stuck and passing tests after it. Returns the new pose.
#[allow(clippy::too_many_arguments)]
pub fn step(r: &mut Rider, env: &Env, me: usize, half_len: f32, upright: bool, dt: f32) -> (Vec3, Vec3) {
    let nav = env.nav;
    if r.ghost.as_ref().is_some_and(|g| g.lc.is_some()) {
        return step_lane_change(r, env, me, half_len, dt);
    }
    let v = r.v;
    // (the speed offset's timer: F-58614 after the frame's behaviour update, `rider_offset` in `ride_traffic`)
    let lane = nav.links[r.link].lane;
    let next_lane = r.next.map(|n| nav.links[n].lane);
    // wanted speed (F-70324): the lane's speed and the next lane's, with the offset, and in a race its holds (the
    // car's heading from its instance matrix as the frame began)
    let holds = if env.event && !r.ty.proxy && r.race_lane != Some(lane) {
        let b = env.others[me].back;
        let (d, pj) = env.player_ahead.map_or(((0.0, 0.0), None), |(d, j)| (d, Some(j)));
        let into = match pj {
            Some(_) => (lane_into_junction(nav, lane), next_lane.and_then(|n| lane_into_junction(nav, n))),
            None => (None, None),
        };
        race_holds(LaneBits::of(&nav.links[r.link]), r.next.map(|n| LaneBits::of(&nav.links[n])), r.t, (b.x, b.z), d, into, pj)
    } else {
        (false, false)
    };
    let rail = rail_speed(nav, r.link);
    let cur = rail.0;
    let (want, next_want) = wanted(rail, r.next.map(|n| rail_speed(nav, n).0), r.off, holds);
    let pos = r.pos(nav);
    let fwd = r.tangent(nav);
    // the car ahead (F-47175): the last search kept while both stand (the shared MWC), else the gather's candidates
    // of this lane (+188) and the next (+192)
    let ahead = {
        let g = [Some(GatherRail::Lane(lane)), next_lane.map(GatherRail::Lane), None];
        search_ahead(r, env, me, pos, fwd, None, (Some(lane), next_lane), r.next.map(|n| nav.links[n].a), g, 0.0)
    };
    // F-16850: go = green for the lane (or a car of the bIsProxy pool while the red light race byte is set) and the
    // junction's box not held by anything but this car (`Blockers`); a refused car stops at the line as at red (and
    // `waits` counts its demand)
    let stop = signal_stop(env, r, pos)
        .filter(|x| !crate::signals::light_go(x.2 == 2, r.ty.proxy, env.signals.red_light_race, || env.blockers.get(&x.3).is_some_and(|b| b.blocks(me))))
        .map(|x| (x.0, x.1));
    // (entry 2: held where it is, its distance 0)
    let slower = (next_want <= v).then(|| {
        // (F-83943 on the lane: the point at the phase to the curve's last vertex - its last control point; the graph's
        // last link end when the lane has no curve)
        let last = nav.curves.get(lane).and_then(|c| c.pts.last().copied()).unwrap_or(nav.links[r.lane_end(nav)].b);
        let rail = LeftRail { flags: 0, byte: 0, phase: r.t, len: 0.0, point: pos, start: Vec3::ZERO, end: Vec3::ZERO, last };
        (next_want, if holds.0 { 0.0 } else { distance_left(Some(&rail)) })
    });
    let dest = r.ghost.as_ref().and_then(|g| g.dest_entry());
    let (mut a, chosen) = control(r, v, want, half_len, &ahead, env.others, stop, slower, dest, dt);
    r.slot = chosen;
    if env.hold {
        a = -r.ty.max_decel;
    }
    if r.ghost.is_some() {
        ghost_integrate(r, a, dt, env.clock);
    } else {
        integrate(r, a, dt);
    }
    // along the lane (F-50894): the phase moves by the distance over the curve speed
    // (+180, from the last pose); past the lane's end the rest of the distance goes on along
    // the next lane from its start, at that lane's speed there (vfunc 11), clamped to it
    let d = r.v * dt;
    if d > 0.0 {
        let lane = nav.links[r.link].lane;
        let c = &nav.curves[lane];
        let t0 = r.phase(nav);
        if r.ds_lane != lane {
            r.ds = set_rail_speed(c, t0);
            r.ds_lane = lane;
        }
        let t1 = mcla_formats::rage::navsys::LaneCurve::advance(t0, d, r.ds);
        if t1 <= 1.0 {
            r.set_phase(nav, env.lanes, t1);
        } else {
            let rest = (t1 - 1.0) * d / (t1 - t0).max(1e-6);
            match r.next {
                None => {
                    r.set_phase(nav, env.lanes, 1.0);
                    r.v = 0.0;
                }
                Some(n) => {
                    r.link = n;
                    r.s = 0.0;
                    r.t = 0.0;
                    // (the cruising ghost's next lane is the route's, F-74277: `Vehicles::ghost_frame` after the step)
                    match r.ghost.as_mut() {
                        Some(g) => {
                            r.next = None;
                            g.turned = true;
                        }
                        None => r.choose_next_env(env),
                    }
                    let nl = nav.links[n].lane;
                    let c = &nav.curves[nl];
                    r.ds = set_rail_speed(c, 0.0);
                    r.ds_lane = nl;
                    if rest > 0.0 {
                        r.set_phase(nav, env.lanes, carry_phase(c, 0.0, rest));
                    }
                }
            }
        }
    }
    let (p, f) = r.pose(nav);
    // F-87488's tail (in a race event too: the stuck test, the gate and the recovery try refuse there themselves); not
    // for the cruising ghost: its stuck and passing tests are open, docs/reference/people/cruise_ghost_hooks.md
    if r.ghost.is_none() {
        after_switch(r, env, me, p, f, chosen, cur, upright, dt);
    }
    (p, f)
}

/// The lane curve the car rides (state 1).
fn nav_curve<'a>(nav: &'a NavSys, r: &Rider) -> Option<&'a LaneCurve> {
    nav.links.get(r.link).and_then(|l| nav.curves.get(l.lane)).filter(|c| c.pts.len() >= 2)
}

/// What the stuck test (F-14023) reads of the rider, its car and the world.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StuckIn {
    /// the race event byte (S +0x64251) and the network test (F-24338)
    pub event: bool,
    pub net: bool,
    /// the car matrix's up y (F-33765 +20)
    pub up_y: f32,
    /// the speed controller's entry in charge (car +420)
    pub slot: i32,
    /// a cab (type +111) with a trailer id (+3604 >= 0); a trailer (type +112)
    pub cab_hitched: bool,
    pub trailer: bool,
    /// behaviour +432 / +436: the nearest obstacle and the blocking one (0 none)
    pub nearest: u32,
    pub blocking: u32,
    /// the car's behaviour (+452) holds a rail (+188)
    pub rail: bool,
    /// its next rail (+192): (has a junction (F-41621), green there (F-78185), its start - slot 12 - x and z)
    pub next: Option<(bool, bool, f32, f32)>,
    /// the car's place x, z (F-58860)
    pub pos: (f32, f32),
    /// speed (car +412)
    pub v: f32,
    /// the follower's reverse rail slot in use (behaviour +184 bit 0x80)
    pub reversing: bool,
    /// the car's body is active (F-17857)
    pub active: bool,
    /// the race path (behaviour +468): referenced (+188), fired (+189)
    pub path: Option<(bool, bool)>,
}

/// The stuck test (F-14023), every frame after the state switch (F-87488), on the tracker (behaviour +532: standing
/// time, backing time, tries). Refused, the tracker kept: the backing time running, a race event, a network game, the
/// car upside down, the stop line's entry in charge (+420 == 1), a cab with its trailer or a trailer, a blocking obstacle
/// that is not the nearest one (both set), the next rail's junction not green with its start within 50 m (squared
/// flat distance < 2500, the image's word). Then it counts while it stands (+412 == 0) with a blocking obstacle - or, off its
/// rail or on the reverse rail, with its body at rest - and its race path neither fired (F-60920) nor held (F-74273):
/// + dt; past 2 s (the image's 2.0) with fewer than 3 tries, a try more and the backing time 0 -> true (the caller starts the
/// 2 m reverse rail); the standing time is not reset here (the backing state F-82779 zeroes it). Otherwise the tracker
/// is zeroed.
pub(crate) fn stuck_test(st: &mut (f32, f32, u8), dt: f32, x: &StuckIn) -> bool {
    if st.1 > 0.0 || x.event || x.net || x.up_y < 0.0 || x.slot == 1 || x.cab_hitched || x.trailer {
        return false;
    }
    if x.blocking != 0 && x.nearest != 0 && x.blocking != x.nearest {
        return false;
    }
    if let Some((true, false, sx, sz)) = x.next {
        let (dx, dz) = (x.pos.0 - sx, x.pos.1 - sz);
        if crate::physics::exact::fma(dz, dz, dx * dx) < 2500.0 {
            return false;
        }
    }
    let stood = x.v == 0.0;
    let at_rest = stood && (!x.rail || x.reversing) && !x.active;
    let fired = x.path.is_some_and(|p| p.1);
    let held = x.path.is_some_and(|p| p.0 && !p.1);
    if (x.blocking != 0 || at_rest) && stood && !fired && !held {
        st.0 += dt;
        if st.0 > 2.0 && st.2 < 3 {
            st.1 = 0.0;
            st.2 += 1;
            return true;
        }
        return false;
    }
    *st = (0.0, 0.0, 0);
    false
}

/// An ambient car's fade (F-82632, every frame; car +484, `rising` +500 bit 0x8 set by the set-up F-92604, `falling` bit
/// 0x4 set by the mark F-79896): rising, + 2 dt (the image's 2.0, fused) and at 1 or more 1 and the rise done; else falling,
/// - 2 dt (fused) and at 0 or less 0, the fall done and the car removed (true; F-51512 with the reason +488). The value
/// goes to the draw holder (+456, slot 11 + the view).
pub(crate) fn fade_step(fade: &mut f32, rising: &mut bool, falling: &mut bool, dt: f32) -> bool {
    use crate::physics::exact::fma;
    if *rising {
        *fade = fma(dt, 2.0, *fade);
        if !(*fade < 1.0) {
            *fade = 1.0;
            *rising = false;
        }
    } else if *falling {
        *fade = -fma(dt, 2.0, -*fade);
        if !(*fade > 0.0) {
            *fade = 0.0;
            *falling = false;
            return true;
        }
    }
    false
}

/// A trailer's cab check (F-99638): not a trailer (type +112), nothing; else its cab id (+3604) looked up among the
/// ambient cars (F-10061) when it is not negative; no id or no car -> (none, removed with reason 16 - F-51512).
pub(crate) fn trailer_cab(trailer: bool, cab_id: i32, lookup: impl FnOnce(i32) -> Option<usize>) -> (Option<usize>, bool) {
    if !trailer {
        return (None, false);
    }
    match if cab_id < 0 { None } else { lookup(cab_id) } {
        Some(c) => (Some(c), false),
        None => (None, true),
    }
}

#[cfg(test)]
mod drive_rule_tests {
    use super::*;

    /// The wired F-23371 meets the level's walkers (their capsule at its matrix): a straight rail through one is
    /// blocked, past one 20 m aside clear; an empty level is clear.
    #[test]
    fn rail_sweep_meets_the_level() {
        let q = crate::tcp_rail::build(Vec3::ZERO, Vec3::Z, Vec3::new(0.0, 0.0, 12.0), Vec3::Z, 5.0, false, false);
        let q = if q.valid() { q } else { crate::tcp_rail::build(Vec3::ZERO, Vec3::Z, Vec3::new(0.0, 0.0, 12.0), Vec3::Z, 5.0, true, true) };
        assert!(q.valid());
        let col = WorldCollider::default();
        let state = crate::physics::loose::LooseState::default();
        let at = |x: f32| [[Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(x, 0.0, 6.0)]];
        for (walkers, want) in [(&at(0.0)[..], true), (&at(20.0)[..], false), (&[][..], false)] {
            let level = RailLevel::new(&col, &[], std::path::Path::new(""), walkers, &state);
            for (mask, lift) in [(2106, 0.0), (u32::MAX, 0.0)] {
                let mut world = LevelSweep { level: &level, include: mask, excluded: [0] };
                let mut left = -1.0;
                assert_eq!(q.sweep(2.0, 0.0, &mut left, lift, &mut world), want, "{walkers:?} {mask:x}");
            }
        }
    }

    /// A case's world for the obstacle gather (tools/traffic_gather_reference.py's layout).
    struct CaseWorld {
        obs: Vec<(u8, [f32; 3], [f32; 3])>,
        lanes: Vec<((u32, u32), [f32; 3], [f32; 3], Vec<usize>)>,
        verts: Vec<Vec<usize>>,
        grid: Vec<Vec<usize>>,
    }

    impl GatherWorld for CaseWorld {
        fn vertex_lanes(&self, v: u32) -> &[usize] {
            self.verts.get(v as usize).map_or(&[], |x| x.as_slice())
        }
        fn lane_vertices(&self, lane: usize) -> (u32, u32) {
            self.lanes[lane].0
        }
        fn lane_ends(&self, lane: usize) -> ([f32; 3], [f32; 3]) {
            (self.lanes[lane].1, self.lanes[lane].2)
        }
        fn rail_cars(&self, lane: usize) -> &[usize] {
            &self.lanes[lane].3
        }
        fn grid_cells(&self, _asker: usize) -> Vec<&[usize]> {
            self.grid.iter().map(|g| g.as_slice()).collect()
        }
        fn kind(&self, o: usize) -> u8 {
            self.obs[o].0
        }
        fn rows(&self, o: usize) -> ([f32; 3], [f32; 3]) {
            (self.obs[o].1, self.obs[o].2)
        }
    }

    /// The spawner's lane pick against the original's code run offline (tools/spawn_lane_reference.py ->
    /// testdata/spawn_lane_ref.txt): F-10273 whole with the record walks F-95725 / F-12622, the lanes' slots 18 / 20 and
    /// the shared MWC from the case; the lane, the phase's bits and the MWC after equal in every case.
    #[test]
    fn spawn_lane_pick_matches_the_original() {
        let (mut n, mut last, mut held) = (0, 0, 0);
        for line in include_str!("testdata/spawn_lane_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(&x.replace('_', ""), 16).unwrap()).collect();
            let mut mwc = super::super::parked::Mwc { lo: w[0], carry: w[1] };
            let m = w[2] as usize;
            let lanes: Vec<usize> = (0..m).map(|i| w[3 + 3 * i] as usize).collect();
            let wt = |l: usize| {
                let i = lanes.iter().position(|&x| x == l).unwrap();
                if w[4 + 3 * i] != 0 { 0.0 } else { f32::from_bits(w[5 + 3 * i]) }
            };
            let got = match pick_lane(&lanes, wt, &mut mwc) {
                Some((i, t)) => {
                    last += (i + 1 == m && m > 1) as usize;
                    held += (w[4 + 3 * i] != 0) as usize;
                    format!("{:x} {:x}", lanes[i], t.to_bits())
                }
                None => "ffffffff deadbeef".to_string(),
            };
            let got = format!("{got} {:04x}_{:04x} {:04x}_{:04x}", mwc.lo >> 16, mwc.lo & 0xFFFF, mwc.carry >> 16, mwc.carry & 0xFFFF);
            assert_eq!(got, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 5000);
        assert!(last > 300 && held > 10, "{last} {held}");
    }

    /// The obstacle gather against the original's code run offline (tools/traffic_gather_reference.py ->
    /// testdata/traffic_gather_ref.txt): F-47175 from its start through the gather (F-14781, F-25797, F-44673 inside),
    /// the grid's cell lists from the case; kept or not, the candidates in order, the shared MWC after and the result's
    /// two words equal in every case.
    #[test]
    fn gather_matches_the_original() {
        let (mut n, mut kept, mut cands, mut full) = (0, 0, 0, 0);
        for line in include_str!("testdata/traffic_gather_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let hw = |x: &str| u32::from_str_radix(&x.replace('_', ""), 16).unwrap();
            let w: Vec<u32> = input.split(' ').map(hw).collect();
            let mut k = 0;
            let mut next = || {
                k += 1;
                w[k - 1]
            };
            let mut mwc = super::super::parked::Mwc { lo: next(), carry: next() };
            let (nobs, asker, prev, prevb) = (next() as usize, next() as usize, next(), next());
            let rail = |x: u32| match x {
                0xFFFF_FFFF => None,
                0xFFFF_FFFE => Some(GatherRail::RunTime),
                l => Some(GatherRail::Lane(l as usize)),
            };
            let (cur, nxt, oth) = (rail(next()), rail(next()), rail(next()));
            let (nl, nv) = (next() as usize, next() as usize);
            let v3 = |f: &mut dyn FnMut() -> u32| [f32::from_bits(f()), f32::from_bits(f()), f32::from_bits(f())];
            let mut obs = Vec::new();
            let mut vel = Vec::new();
            for _ in 0..nobs {
                let kind = next() as u8;
                let r2 = v3(&mut next);
                let p = v3(&mut next);
                vel.push(v3(&mut next));
                obs.push((kind, r2, p));
            }
            let mut lanes = Vec::new();
            for _ in 0..nl {
                let (a, b) = (next(), next());
                let s = v3(&mut next);
                let e = v3(&mut next);
                let nc = next() as usize;
                lanes.push(((a, b), s, e, (0..nc).map(|_| next() as usize).collect()));
            }
            let verts: Vec<Vec<usize>> = (0..nv).map(|_| (0..next()).map(|_| next() as usize).collect()).collect();
            let grid: Vec<Vec<usize>> = (0..9).map(|_| (0..next()).map(|_| next() as usize).collect()).collect();
            let world = CaseWorld { obs, lanes, verts, grid };
            let last = (prev != 0xFFFF_FFFF).then(|| vel[prev as usize]);
            let ix = |x: u32| if x == 0xFFFF_FFFF { "ffffffff".to_string() } else { format!("{x:x}") };
            let mut got = if search_kept(vel[asker], last, &mut mwc) {
                kept += 1;
                format!("k {:04x}_{:04x} {:04x}_{:04x} {} {}", mwc.lo >> 16, mwc.lo & 0xFFFF, mwc.carry >> 16, mwc.carry & 0xFFFF, ix(prev), ix(prevb))
            } else {
                let c = gather_obstacles(&world, asker, cur, nxt, oth);
                cands += c.len();
                full += (c.len() == GATHER_CAP) as usize;
                let mut s = format!("g {}", c.len());
                for o in c {
                    s += &format!(" {o:x}");
                }
                s + &format!(" {:04x}_{:04x} {:04x}_{:04x} ffffffff ffffffff", mwc.lo >> 16, mwc.lo & 0xFFFF, mwc.carry >> 16, mwc.carry & 0xFFFF)
            };
            got = got.trim().to_string();
            assert_eq!(got, want.trim(), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
        assert!(kept > 300 && cands > 5000 && full > 5, "{kept} {cands} {full}");
    }

    /// A case's world for the selection (tools/traffic_select_reference.py's layout).
    struct SelCase {
        /// per object: kind, rows 0 / 2 / 3, velocity, size, has a car (+16 / +312), car, driver lane (0 none, 1, 2
        /// freeway), standing time, limit
        objs: Vec<(u8, [[f32; 3]; 3], [f32; 3], (f32, f32), bool, SelCar, u32, f32, f32)>,
        /// per rail: run-time, start, per object (nearest point, phase)
        rails: Vec<(bool, [f32; 3], Vec<([f32; 3], f32)>)>,
        /// rail 0 an arc-line-arc rail (tools/traffic_select_tcp_reference.py): its slots 12 / 14 are `RunRail`'s
        tcp: Option<TcpRail>,
    }

    impl SelCase {
        fn rail(&self, r: u32) -> Option<SelRail> {
            (r != 0xFFFF_FFFF).then(|| if self.rails[r as usize].0 { SelRail::RunTime(r) } else { SelRail::Lane(r as usize) })
        }
        fn index(r: SelRail) -> usize {
            match r {
                SelRail::Lane(l) => l,
                SelRail::RunTime(k) => k as usize,
            }
        }
    }

    impl SelectWorld for SelCase {
        fn rows(&self, o: usize) -> ([f32; 3], [f32; 3], [f32; 3]) {
            let r = self.objs[o].1;
            (r[0], r[1], r[2])
        }
        fn vel(&self, o: usize) -> [f32; 3] {
            self.objs[o].2
        }
        fn size(&self, o: usize) -> (f32, f32) {
            self.objs[o].3
        }
        fn kind(&self, o: usize) -> u8 {
            self.objs[o].0
        }
        fn ambient(&self, o: usize) -> Option<SelCar> {
            let x = &self.objs[o];
            (x.0 == 2 && x.4).then_some(x.5)
        }
        fn driver(&self, o: usize) -> Option<(Option<bool>, f32, f32)> {
            let x = &self.objs[o];
            (x.0 == 0 || (x.0 == 9 && x.4)).then(|| ((x.6 != 0).then_some(x.6 == 2), x.7, x.8))
        }
        fn nearest(&self, rail: SelRail, o: usize) -> ([f32; 3], f32) {
            match (&self.tcp, Self::index(rail)) {
                (Some(q), 0) => RunRail::Tcp(q).nearest(Vec3::from(self.objs[o].1[2])),
                (_, k) => self.rails[k].2[o],
            }
        }
        fn start(&self, rail: SelRail) -> [f32; 3] {
            match (&self.tcp, Self::index(rail)) {
                (Some(q), 0) => RunRail::Tcp(q).start(),
                (_, k) => self.rails[k].1,
            }
        }
    }

    /// The choice of the car ahead against the original's code run offline (tools/traffic_select_reference.py ->
    /// testdata/traffic_select_ref.txt): F-47175 from its start with the case's candidates in the gather's list (the
    /// result's two words: nearest and blocking), F-70116 and F-87404 called alone; every answer equal.
    #[test]
    fn select_matches_the_original() {
        let (mut n, mut near, mut block, mut btrue, mut clear) = (0, 0, 0, 0, 0);
        for line in include_str!("testdata/traffic_select_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let mut k = 0;
            let mut next = || {
                k += 1;
                w[k - 1]
            };
            let (mode, nobj, asker, nr, ra, rb, bs) = (next(), next() as usize, next() as usize, next() as usize, next(), next(), f32::from_bits(next()));
            let (mut cands, mut o, mut pt) = (Vec::new(), 0, [0.0; 3]);
            if mode == 0 {
                let c = next();
                cands = (0..c).map(|_| next() as usize).collect();
            } else {
                o = next() as usize;
                if mode == 2 {
                    pt = [f32::from_bits(next()), f32::from_bits(next()), f32::from_bits(next())];
                }
            }
            let v3 = |f: &mut dyn FnMut() -> u32| [f32::from_bits(f()), f32::from_bits(f()), f32::from_bits(f())];
            let mut objs = Vec::new();
            let mut behs = Vec::new();
            for _ in 0..nobj {
                let kind = next() as u8;
                let rows = [v3(&mut next), v3(&mut next), v3(&mut next)];
                let vel = v3(&mut next);
                let size = (f32::from_bits(next()), f32::from_bits(next()));
                let (hascar, speed, active, cab, trl, beh, r188, r192, lane, idle, limit) =
                    (next() != 0, f32::from_bits(next()), next() != 0, next() != 0, next() != 0, next() != 0, next(), next(), next(), f32::from_bits(next()), f32::from_bits(next()));
                behs.push((beh, r188, r192));
                objs.push((kind, rows, vel, size, hascar, SelCar { speed, beh: None, cab_or_trailer: cab || trl, active }, lane, idle, limit));
            }
            let mut rails = Vec::new();
            for _ in 0..nr {
                let rt = next() == 0x1800;
                let s = v3(&mut next);
                rails.push((rt, s, (0..nobj).map(|_| (v3(&mut next), f32::from_bits(next()))).collect::<Vec<_>>()));
            }
            let mut case = SelCase { objs, rails, tcp: None };
            for (i, &(beh, r188, r192)) in behs.iter().enumerate() {
                case.objs[i].5.beh = beh.then(|| (case.rail(r188), case.rail(r192)));
            }
            let ix = |x: Option<usize>| x.map_or("ffffffff".to_string(), |x| format!("{x:x}"));
            let got = match mode {
                0 => {
                    let (a, b) = select_obstacles(&case, asker, &cands, case.rail(ra), case.rail(rb), bs);
                    near += a.is_some() as usize;
                    block += b.is_some() as usize;
                    format!("{} {}", ix(a), ix(b))
                }
                1 => {
                    let b = blocking_of(&case, o, asker);
                    btrue += b as usize;
                    format!("{:x}", b as u8)
                }
                _ => {
                    let c = clear_of(&case, o, pt, asker);
                    clear += c as usize;
                    format!("{:x}", c as u8)
                }
            };
            assert_eq!(got, want.trim(), "{line}");
            n += 1;
        }
        assert_eq!(n, 5500);
        assert!(near > 500 && block > 100 && btrue > 150 && clear > 800 && clear < 2400, "{near} {block} {btrue} {clear}");
    }

    /// The selection with an arc-line-arc rail as the asker's current rail against the original's code run offline
    /// (tools/traffic_select_tcp_reference.py -> testdata/traffic_select_tcp_ref.txt): F-47175 whose slot 14 / 12 calls on
    /// that rail are the image's (F-36154 -> F-16977, F-55960 -> F-33742) on the record F-85722 built; the port's
    /// `RunRail` on `tcp_rail::build`'s rail. Both result words and the rail's valid flag equal in every case.
    #[test]
    fn select_tcp_matches_the_original() {
        let (mut n, mut near, mut block, mut valid) = (0, 0, 0, 0);
        for line in include_str!("testdata/traffic_select_tcp_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let mut k = 0;
            let mut next = || {
                k += 1;
                w[k - 1]
            };
            let (mode, nobj, asker, nr, ra, rb, bs) = (next(), next() as usize, next() as usize, next() as usize, next(), next(), f32::from_bits(next()));
            assert_eq!(mode, 3);
            let c = next();
            let cands: Vec<usize> = (0..c).map(|_| next() as usize).collect();
            let v3 = |f: &mut dyn FnMut() -> u32| [f32::from_bits(f()), f32::from_bits(f()), f32::from_bits(f())];
            let mut objs = Vec::new();
            let mut behs = Vec::new();
            for _ in 0..nobj {
                let kind = next() as u8;
                let rows = [v3(&mut next), v3(&mut next), v3(&mut next)];
                let vel = v3(&mut next);
                let size = (f32::from_bits(next()), f32::from_bits(next()));
                let (hascar, speed, active, cab, trl, beh, r188, r192, lane, idle, limit) =
                    (next() != 0, f32::from_bits(next()), next() != 0, next() != 0, next() != 0, next() != 0, next(), next(), next(), f32::from_bits(next()), f32::from_bits(next()));
                behs.push((beh, r188, r192));
                objs.push((kind, rows, vel, size, hascar, SelCar { speed, beh: None, cab_or_trailer: cab || trl, active }, lane, idle, limit));
            }
            let mut rails = Vec::new();
            for _ in 0..nr {
                let rt = next() == 0x1800;
                let s = v3(&mut next);
                rails.push((rt, s, (0..nobj).map(|_| (v3(&mut next), f32::from_bits(next()))).collect::<Vec<_>>()));
            }
            let (p, h0, t, h1) = (Vec3::from(v3(&mut next)), Vec3::from(v3(&mut next)), Vec3::from(v3(&mut next)), Vec3::from(v3(&mut next)));
            let (r, s0, s1) = (f32::from_bits(next()), next() != 0, next() != 0);
            let q = crate::tcp_rail::build(p, h0, t, h1, r, s0, s1);
            let mut case = SelCase { objs, rails, tcp: Some(q) };
            for (i, &(beh, r188, r192)) in behs.iter().enumerate() {
                case.objs[i].5.beh = beh.then(|| (case.rail(r188), case.rail(r192)));
            }
            let ix = |x: Option<usize>| x.map_or("ffffffff".to_string(), |x| format!("{x:x}"));
            let (a, b) = select_obstacles(&case, asker, &cands, case.rail(ra), case.rail(rb), bs);
            near += a.is_some() as usize;
            block += b.is_some() as usize;
            valid += q.valid() as usize;
            assert_eq!(format!("{} {} {}", ix(a), ix(b), q.valid() as u8), want.trim(), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
        assert!(near > 500 && block > 100 && valid > 2000, "{near} {block} {valid}");
    }

    /// The cab's trailer pick against the original's code run offline (tools/trailer_pick_reference.py ->
    /// testdata/trailer_pick_ref.txt): F-45988 whole on the cab's MWC, the type lookup answered from the case; the name,
    /// the shade and the MWC after equal in every case.
    #[test]
    fn trailer_pick_matches_the_original() {
        let (mut n, mut shades) = (0, 0);
        let h = |x: &str| u32::from_str_radix(&x.replace('_', ""), 16).unwrap();
        for line in include_str!("testdata/trailer_pick_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = input.split(' ').collect();
            let mut m = super::GMwc { lo: h(w[0]), carry: h(w[1]) };
            let mut asked = None;
            let got = trailer_pick(&mut m, |name| {
                asked = TRAILERS.iter().position(|&t| t == name);
                w[2] == "1"
            });
            shades += got.is_some() as usize;
            let k = asked.map_or(-1, |k| k as i32);
            let shade = got.map_or(0xFFFF, |g| g.1);
            let got = format!("{k} {shade:x} {:04x}_{:04x} {:04x}_{:04x}", m.lo >> 16, m.lo & 0xFFFF, m.carry >> 16, m.carry & 0xFFFF);
            assert_eq!(got, want.trim(), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
        assert!(shades > 2000, "{shades}");
    }

    /// The pass tracker's test against the original's code run offline (tools/traffic_passmoved_reference.py ->
    /// testdata/traffic_passmoved_ref.txt): F-73744 whole; the answer equal in every case.
    #[test]
    fn pass_moved_matches_the_original() {
        let (mut n, mut moved) = (0, 0);
        for line in include_str!("testdata/traffic_passmoved_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let v = |i: usize| Vec3::new(f32::from_bits(w[i]), f32::from_bits(w[i + 1]), f32::from_bits(w[i + 2]));
            let got = pass_moved(v(1), (w[0] != 0).then(|| v(4)));
            moved += got as usize;
            assert_eq!(format!("{}", got as u8), want.trim(), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
        assert!(moved > 800, "{moved}");
    }

    /// F-87488 against the original's code run offline (tools/rider_frame_reference.py -> testdata/rider_frame_ref.txt):
    /// the frame whole, its callees answered from the case and logged in call order, for every state 0 - 10. The switch
    /// part is written out here as decoded (states 1 - 3, 4, 5, 6, 7, 8; 0, 9 and 10 have none) with the port's
    /// `switch_clears_tracker` for its tracker clears; the tail is the port's `frame_tail` with the case's answers. Every
    /// call sequence and the state after equal.
    #[test]
    fn rider_frame_matches_the_original() {
        const NAMES: [&str; 15] = ["follow", "search", "lane", "speed", "recovery", "clear_tracker", "clear_recovery", "back_step", "zero_stuck", "unload", "stuck", "back_start", "gate", "tracker", "pass"];
        struct Log<'a> {
            a: &'a [u32],
            bt: f32,
            nrec: usize,
            out: Vec<usize>,
        }
        impl Log<'_> {
            fn l(&mut self, k: &str) {
                self.out.push(NAMES.iter().position(|n| *n == k).unwrap());
            }
            fn rec(&mut self) -> bool {
                self.l("recovery");
                let i = 1 + (self.nrec > 0) as usize;
                self.nrec += 1;
                self.a[i] != 0
            }
        }
        impl TailSteps for Log<'_> {
            fn stuck(&mut self) -> bool {
                self.l("stuck");
                self.a[4] != 0
            }
            fn back_start(&mut self) -> bool {
                self.l("back_start");
                self.a[5] != 0
            }
            fn backing_time(&self) -> f32 {
                self.bt
            }
            fn gate(&mut self) -> Option<usize> {
                self.l("gate");
                (self.a[6] != 0).then_some(self.a[6] as usize)
            }
            fn tracker(&mut self) -> bool {
                self.l("tracker");
                self.a[7] != 0
            }
            fn pass(&mut self, o: usize) -> bool {
                self.l("pass");
                assert_eq!(o, self.a[6] as usize);
                self.a[8] != 0
            }
            fn clear_recovery(&mut self) {
                self.l("clear_recovery");
            }
            fn recovery(&mut self) -> bool {
                self.rec()
            }
            fn clear_tracker(&mut self) {
                self.l("clear_tracker");
            }
        }
        let (mut n, mut tracked, mut cleared) = (0, [0usize; 11], 0);
        for line in include_str!("testdata/rider_frame_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let (mut st, ev) = (w[0] as u8, w[1] != 0);
            let mut g = Log { a: &w[5..14], bt: f32::from_bits(w[4]), nrec: 0, out: Vec::new() };
            let ended = g.a[0] != 0;
            match st {
                1..=3 => ["follow", "search", "lane", "speed"].iter().for_each(|k| g.l(k)),
                4 => {
                    assert!(switch_clears_tracker(4, ev, false));
                    g.l("clear_tracker");
                    g.l("clear_recovery");
                }
                5 if !ev => {
                    g.l("speed");
                    if g.rec() {
                        st = 6;
                    }
                }
                6 if !ev => {
                    ["follow", "search", "speed"].iter().for_each(|k| g.l(k));
                    if switch_clears_tracker(6, ev, ended) {
                        g.l("clear_tracker");
                        g.l("clear_recovery");
                        st = 1;
                    } else {
                        g.rec();
                    }
                }
                7 => {
                    ["follow", "search", "speed"].iter().for_each(|k| g.l(k));
                    if switch_clears_tracker(7, ev, ended) {
                        g.l("clear_tracker");
                        g.l("clear_recovery");
                        st = 6;
                    }
                }
                8 if !ev => {
                    g.l("follow");
                    g.l("speed");
                    g.l("back_step");
                    if g.a[3] != 0 {
                        g.l("zero_stuck");
                        g.l("unload");
                        st = 6;
                    }
                }
                _ => {}
            }
            cleared += g.out.contains(&5) as usize;
            let (near, block) = (w[2], w[3]);
            if let Some(s) = frame_tail(ev, near, block, &mut g) {
                st = s;
            }
            if g.out.contains(&13) {
                tracked[w[0] as usize] += 1;
            }
            let calls: String = g.out.iter().map(|k| format!("{k},")).collect();
            let (want_calls, want_state) = want.trim().split_once(' ').unwrap();
            assert_eq!(if calls.is_empty() { "-".to_string() } else { calls }, want_calls, "{line}");
            assert_eq!(st.to_string(), want_state, "{line}");
            n += 1;
        }
        assert_eq!(n, 4000);
        // (the tracker after the switch in every state)
        assert!(tracked.iter().all(|&t| t > 50), "{tracked:?}");
        assert!(cleared > 500, "{cleared}");
    }

    /// The rail pools and the reverse rail's refusal against the original's code run offline
    /// (tools/rail_pool_reference.py -> testdata/rail_pool_ref.txt): F-88792 on a free list of n rails gives n and then
    /// null every time (`pool_take` with 256 - n held, the pool's 256 from F-76414); F-56052 with F-54867's answer 0
    /// returns 0 after only the holder's destructor (the caller then keeps its state), with 1 returns 1 after the unload
    /// and the rail set.
    #[test]
    fn rail_pool_matches_the_original() {
        let (mut n, mut refused) = (0, 0);
        for line in include_str!("testdata/rail_pool_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let out: Vec<&str> = want.split_whitespace().collect();
            if w[0] == 0 {
                let free = w[1] as usize;
                // (the original: `free` takes, then null every time; the pool holds 256, so `free` left = 256 - free held)
                assert_eq!((out[0], out[1]), (free.to_string().as_str(), "1"), "{line}");
                if free <= RAIL_POOL {
                    let got = (0..free + 3).take_while(|&k| pool_take(RAIL_POOL - free + k)).count();
                    assert_eq!(got, free, "{line}");
                }
            } else {
                let built = w[1] != 0;
                assert_eq!(out, if built { vec!["1", "us"] } else { vec!["0", "d"] }, "{line}");
                refused += !built as usize;
            }
            n += 1;
        }
        assert_eq!((n, refused), (92, 20));
    }

    /// The region box getters F-81783 (length) and F-68633 (width) against the original's code run offline
    /// (tools/region_box_reference.py -> testdata/region_box_ref.txt; "which flag mph a b -> result", the pair the flag
    /// picks): `box_lerp(pair, box_t(mph))` equal to the bit in every case (the port's mph / 60 and unfused lerp differed
    /// in 297).
    #[test]
    fn region_box_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/region_box_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let got = super::box_lerp([f(3), f(4)], super::box_t(f(2)));
            assert_eq!(format!("{:08x}", got.to_bits()), want.trim(), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The mutual-nearest mark against the original's code run offline (tools/traffic_mark_reference.py ->
    /// testdata/traffic_mark_ref.txt): F-37525 whole, the cast and our obstacle answered from the case; the mark (the other
    /// or none) equal in every case.
    #[test]
    fn mutual_mark_matches_the_original() {
        let (mut n, mut marked) = (0, 0);
        for line in include_str!("testdata/traffic_mark_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let v = |i: usize| Vec3::new(f32::from_bits(w[i]), f32::from_bits(w[i + 1]), f32::from_bits(w[i + 2]));
            let pre = w[0] != 0 && w[1] != 0 && w[2] != 0 && w[3] != 0 && w[4] == 0 && w[5] != 0;
            let got = pre && mutual_mark(w[6] == 3, (v(14), v(17)), w[7] == 3, (v(8), v(11)));
            marked += got as usize;
            assert_eq!(format!("{}", got as u8), want.trim(), "{line}");
            n += 1;
        }
        assert_eq!(n, 4000);
        assert!(marked > 1000, "{marked}");
    }

    /// The trailer's cab check against the original's code run offline (tools/trailer_cab_reference.py ->
    /// testdata/trailer_cab_ref.txt): every type byte / id / lookup combination, the answer, the id asked and the removal equal.
    #[test]
    fn trailer_cab_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/trailer_cab_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let mut asked = None;
            let (cab, gone) = trailer_cab(w[0] & 0xFF != 0, w[1] as i32, |id| {
                asked = Some(id);
                (w[2] != 0).then_some(1)
            });
            let got = format!("{} {} {}", cab.unwrap_or(0), asked.map_or("-".into(), |a| format!("{:x}", a as u32)), if gone { "10" } else { "-" });
            assert_eq!(got, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 48);
    }

    /// The fade against the original's code run offline (tools/traffic_fade_reference.py -> testdata/traffic_fade_ref.txt):
    /// F-82632 whole, the removal stubbed; the flag byte, the fade, the removal and the holder's word equal in every case.
    #[test]
    fn fade_matches_the_original() {
        let (mut n, mut removed) = (0, 0);
        for line in include_str!("testdata/traffic_fade_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let o: Vec<&str> = want.split(' ').collect();
            let (mut fade, mut rising, mut falling) = (f32::from_bits(w[1]), w[0] & 8 != 0, w[0] & 4 != 0);
            let gone = fade_step(&mut fade, &mut rising, &mut falling, f32::from_bits(w[2]));
            let flags = (w[0] & !0xC) | if rising { 8 } else { 0 } | if falling { 4 } else { 0 };
            assert_eq!(format!("{flags:x}"), o[0], "{line}");
            assert_eq!(format!("{:x}", fade.to_bits()), o[1], "{line}");
            assert_eq!(gone, o[2] != "-", "{line}");
            let holder = if w[3] != 0 { fade.to_bits() } else { 0 };
            assert_eq!(format!("{holder:x}"), o[3], "{line}");
            n += 1;
            removed += gone as usize;
        }
        assert_eq!(n, 5000);
        assert!(removed > 500);
    }

    /// The stuck test against the original's code run offline (tools/traffic_stuck_reference.py ->
    /// testdata/traffic_stuck_ref.txt): F-14023 whole with the rider's path getters F-60920 / F-74273, the world's
    /// answers from the case; the answer and the tracker's three words equal in every case.
    #[test]
    fn stuck_test_matches_the_original() {
        let (mut n, mut yes) = (0, 0);
        for line in include_str!("testdata/traffic_stuck_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let o: Vec<u32> = want.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let b = w[4];
            let x = StuckIn {
                event: b & 1 != 0,
                net: b & 2 != 0,
                up_y: f(5),
                slot: w[6] as i32,
                cab_hitched: b & 4 != 0 && (w[7] as i32) >= 0,
                trailer: b & 8 != 0,
                nearest: w[8],
                blocking: w[9],
                rail: b & 16 != 0 && b & 32 != 0,
                next: (b & 16 != 0 && b & 64 != 0).then_some((b & 128 != 0, b & 256 != 0, f(11), f(12))),
                pos: (f(13), f(14)),
                v: f(10),
                reversing: b & 512 != 0,
                active: b & 1024 != 0,
                path: (b & 2048 != 0).then_some((b & 4096 != 0, b & 8192 != 0)),
            };
            let mut st = (f(0), f(1), w[2] as u8);
            let got = stuck_test(&mut st, f(3), &x);
            assert_eq!((got as u32, st.0.to_bits(), st.1.to_bits(), st.2 as u32), (o[0], o[1], o[2], o[3]), "{line}");
            n += 1;
            yes += got as usize;
        }
        assert_eq!(n, 6000);
        assert!(yes > 300);
    }
}

/// What F-87488's tail asks (`frame_tail`); the port's answers are `PortTail`'s.
pub(crate) trait TailSteps {
    /// the stuck test F-14023
    fn stuck(&mut self) -> bool;
    /// the backing rail F-56052 (through its thunk): taken
    fn back_start(&mut self) -> bool;
    /// the stuck tracker's backing time (behaviour +532 +4), read after the stuck test
    fn backing_time(&self) -> f32;
    /// the passing gate F-72757 on the blocking obstacle (+436): the obstacle when it is close
    fn gate(&mut self) -> Option<usize>;
    /// the pass tracker F-73744 (+480): its obstacle moved
    fn tracker(&mut self) -> bool;
    /// the passing try F-28706 round the gate's obstacle: taken
    fn pass(&mut self, o: usize) -> bool;
    /// the recovery's standing time (+472) zeroed
    fn clear_recovery(&mut self);
    /// the recovery try F-15490 (r5 = 0): taken
    fn recovery(&mut self) -> bool;
    /// the pass tracker (+480: obstacle, place, standing time) cleared
    fn clear_tracker(&mut self);
}

/// F-87488's tail, after the switch in every state (states 0 - 10; tools/rider_frame_reference.py): the stuck test
/// (a try: the backing rail, state 8 when it is taken, and nothing more this frame); the backing time running (> 0,
/// F-87488's own test): nothing more; the passing gate when the blocking obstacle (+436) is set and is the nearest
/// (+432) and no race event (S +0x64251); the pass tracker; with the gate's obstacle the passing try (taken: the recovery's
/// time zeroed, state 7) and nothing more; else when the tracker's obstacle moved the recovery try (taken: the tracker
/// cleared, state 6). Returns the state it sets.
pub(crate) fn frame_tail(event: bool, near: u32, block: u32, h: &mut dyn TailSteps) -> Option<u8> {
    if h.stuck() {
        return h.back_start().then_some(8);
    }
    if h.backing_time() > 0.0 {
        return None;
    }
    let gate = if block != 0 && near == block && !event { h.gate() } else { None };
    let moved = h.tracker();
    if let Some(o) = gate {
        if h.pass(o) {
            h.clear_recovery();
            return Some(7);
        }
        return None;
    }
    if moved && h.recovery() {
        h.clear_tracker();
        return Some(6);
    }
    None
}

/// F-87488's switch where it clears the pass tracker (F-73744's +480; tools/rider_frame_reference.py): state 4 every
/// frame; states 6 and 7 when the follower's step reaches the rail's end (state 6's case is skipped whole in a race
/// event).
pub(crate) fn switch_clears_tracker(state: u8, event: bool, ended: bool) -> bool {
    match state {
        4 => true,
        6 => !event && ended,
        7 => ended,
        _ => false,
    }
}

/// The passing gate F-72757 (r4 the car, r5 the blocking obstacle): the distance d = sqrt(fma(dx, dx, fma(dz, dz,
/// dy dy))) from the car's place (F-58860) to the obstacle's (slot 1 +48); standing (+412 == 0): the obstacle when its
/// velocity (slot 2) is shorter than 2.2352 (sqrt(fma(vz, vz, fma(vx, vx, vy vy)))); else when d < 12, or d / v under 4 s
/// when the car's rail (behaviour +452 -> +188) has a speed (slot 17) above 26.8224, under 3 s otherwise. A NaN: none.
pub(crate) fn pass_gate(our: Vec3, at: Vec3, v: f32, vel: Vec3, limit: Option<f32>) -> bool {
    use crate::physics::exact::fma;
    let (dy, dz, dx) = (our.y - at.y, our.z - at.z, our.x - at.x);
    let d = fma(dx, dx, fma(dz, dz, dy * dy)).sqrt();
    if v == 0.0 {
        return fma(vel.z, vel.z, fma(vel.x, vel.x, vel.y * vel.y)).sqrt() < 2.2352;
    }
    if d < 12.0 {
        return true;
    }
    let t = d / v;
    match limit {
        Some(s) if s > 26.8224 => t < 4.0,
        _ => t < 3.0,
    }
}

/// The pass tracker cleared (F-87488's +480 clear: the obstacle, its place and F-28706's standing time +32).
fn clear_tracker(r: &mut Rider) {
    r.passing = None;
    r.pass_stand = 0.0;
}

/// The port's answers to F-87488's tail.
struct PortTail<'a, 'e> {
    r: &'a mut Rider,
    env: &'a Env<'e>,
    me: usize,
    pos: Vec3,
    fwd: Vec3,
    chosen: Option<usize>,
    /// the speed of the car's rail (slot 17: a lane's speed, a run-time rail's +304 / +156)
    lane_speed: f32,
    upright: bool,
    dt: f32,
    /// +432 / +436 as words (0 none, else the obstacle's index + 1)
    near: u32,
    block: u32,
    /// the follower's rail at its phase: the passing request's start and heading (F-92334 as F-26749)
    start: (Vec3, Vec3),
    /// the stuck test's next rail (+192)
    next: Option<(bool, bool, f32, f32)>,
}

impl TailSteps for PortTail<'_, '_> {
    fn stuck(&mut self) -> bool {
        let o = &self.env.others[self.me];
        let r = &*self.r;
        let x = StuckIn {
            event: self.env.event,
            net: false,
            up_y: if self.upright { 0.0 } else { -1.0 },
            slot: self.chosen.map_or(-1, |s| s as i32),
            cab_hitched: r.ty.cab && r.partner.is_some(),
            trailer: r.ty.trailer,
            nearest: self.near,
            blocking: self.block,
            rail: r.path.is_some() || matches!(r.state, 1..=3),
            next: self.next,
            pos: (o.pos.x, o.pos.z),
            v: r.v,
            reversing: r.path.as_ref().is_some_and(|p| p.reverse),
            active: r.active,
            path: self.env.paths.get(self.me).copied().flatten(),
        };
        stuck_test(&mut self.r.stuck, self.dt, &x)
    }
    fn back_start(&mut self) -> bool {
        // (F-56052: refused when F-54867's take from pool 0 is null - all 256 held)
        if !pool_take(self.env.pool0.get() + holds_pool0(self.r) as usize) {
            return false;
        }
        // (F-54867: from the instance matrix as the frame began)
        let o = &self.env.others[self.me];
        self.r.take_path(backing_rail(o.pos, o.back), 8);
        true
    }
    fn backing_time(&self) -> f32 {
        self.r.stuck.1
    }
    fn gate(&mut self) -> Option<usize> {
        let b = self.block as usize - 1;
        let (us, o) = (&self.env.others[self.me], self.env.others.get(b)?);
        pass_gate(us.pos, o.pos, self.r.v, o.vel, Some(self.lane_speed)).then_some(b)
    }
    fn tracker(&mut self) -> bool {
        self.r.passing.is_some_and(|(j, at)| pass_moved(at, self.env.others.get(j).map(|o| o.pos)))
    }
    fn pass(&mut self, b: usize) -> bool {
        let (r, env) = (&mut *self.r, self.env);
        let semi = r.ty.trailer || (r.ty.cab && r.partner.is_some());
        if self.chosen == Some(1) || !self.upright || semi {
            return false;
        }
        // (the next lane's light at stop within 50 m)
        if signal_stop(env, r, self.pos).is_some_and(|x| x.2 != 2 && x.1 < 50.0) {
            return false;
        }
        if r.v == 0.0 {
            r.pass_stand += self.dt;
        } else {
            r.pass_stand = 0.0;
        }
        let ready = r.pass_stand > 2.0;
        // (F-28706: one draw of the car's MWC, U x 2^-23 > 1/60 (0x3C888889): no try)
        if env.others[b].ambient && u23(&mut r.mwc) * f32::from_bits(0x3400_0000) > f32::from_bits(0x3C88_8889) {
            return false;
        }
        let (p, h0) = self.start;
        // (F-28706: the clearances 1.1, 1.5, 0.8 m in turn, the first that gives a passing rail and its rejoin)
        for c in [1.1f32, 1.5, 0.8] {
            if let Some((pass, back)) = passing_rail(r, env, self.me, p, h0, b, c, ready) {
                r.take_path(pass, 7);
                r.then = Some(back);
                r.passing = Some((b, env.others[b].pos));
                return true;
            }
        }
        false
    }
    fn clear_recovery(&mut self) {
        self.r.stand = 0.0;
    }
    fn recovery(&mut self) -> bool {
        try_recovery(self.r, self.env, self.me, self.pos, self.fwd, self.upright, self.dt)
    }
    fn clear_tracker(&mut self) {
        clear_tracker(self.r);
    }
}

/// F-87488's tail with the port's answers (`frame_tail`), after the switch: the obstacle words +432 / +436 from the
/// last search (`Rider::kept`), the start of a passing rail the follower's rail at its phase (a lane's curve, or the
/// run-time rail: slot 8's point and its derivative flattened and made unit), the stuck test's next rail the next lane
/// (+192) while on a lane - none on a run-time rail (its +192 is not traced).
#[allow(clippy::too_many_arguments)]
fn after_switch(r: &mut Rider, env: &Env, me: usize, pos: Vec3, fwd: Vec3, chosen: Option<usize>, lane_speed: f32, upright: bool, dt: f32) {
    let (near, block) = match r.kept {
        Some((j, b)) => (j as u32 + 1, b.map_or(0, |b| b as u32 + 1)),
        None => (0, 0),
    };
    let on_lane = r.path.is_none() && matches!(r.state, 1..=3);
    let next = if on_lane {
        r.next.map(|n| match lane_junction(env.nav, env.nav.links[n].lane) {
            Some(j) => {
                let (p0, _) = lane_entry(env.nav, n);
                (true, junction_green(env.signals, env.nav, j, n), p0.x, p0.z)
            }
            None => (false, false, 0.0, 0.0),
        })
    } else {
        None
    };
    let start = match (r.path.as_ref(), nav_curve(env.nav, r)) {
        (Some(pp), _) => {
            let (q, d) = pp.eval(pp.t);
            (q, crate::tcp_rail::vnorm(Vec3::new(d.x, 0.0, d.z)))
        }
        (None, Some(c)) => {
            let (q, d) = c.eval_pair(r.t);
            (q, crate::tcp_rail::vnorm(Vec3::new(d.x, 0.0, d.z)))
        }
        (None, None) => (pos, fwd),
    };
    let event = env.event;
    let mut t = PortTail { r, env, me, pos, fwd, chosen, lane_speed, upright, dt, near, block, start, next };
    frame_tail(event, near, block, &mut t);
}

/// F-73744 (the pass tracker, behaviour +480): its obstacle (+0) when that moved more than 0.333 m from its place when the
/// pass began (+16): the squared distance fma(dx, dx, fma(dz, dz, dy dy)) of (now - then) > 0.110889 (0x3DE319C6; a NaN:
/// not); no obstacle (gone: the object-deleted clear F-57497 nulls +480): none.
pub fn pass_moved(at: Vec3, now: Option<Vec3>) -> bool {
    use crate::physics::exact::fma;
    let Some(p) = now else { return false };
    let (dy, dz, dx) = (p.y - at.y, p.z - at.z, p.x - at.x);
    fma(dx, dx, fma(dz, dz, dy * dy)) > f32::from_bits(0x3DE3_19C6)
}

/// One frame on a rail off the lanes (states 6 - 8). Returns the new pose, or None when the
/// car turns physical (the footprint probe hit something).
pub fn step_path(r: &mut Rider, env: &Env, me: usize, pos: Vec3, fwd: Vec3, half_len: f32, upright: bool, dt: f32) -> Option<(Vec3, Vec3)> {
    let out = step_path_switch(r, env, me, pos, fwd, half_len, upright, dt)?;
    // F-87488's tail after the switch (the pass tracker F-73744 in every state); the rail's speed (slot 17)
    if r.ghost.is_none() {
        let speed = r.path.as_ref().map_or(0.0, |p| p.speed);
        after_switch(r, env, me, out.0, out.1, r.slot, speed, upright, dt);
    }
    Some(out)
}

/// F-87488's switch in states 6 - 8 (the port's rail follower).
#[allow(clippy::too_many_arguments)]
fn step_path_switch(r: &mut Rider, env: &Env, me: usize, pos: Vec3, fwd: Vec3, half_len: f32, upright: bool, dt: f32) -> Option<(Vec3, Vec3)> {
    let hw = r.ty.width.max(1.6) * 0.5;
    let semi = r.ty.trailer || r.ty.cab;
    // state 6: re-planned every frame (on a rail or standing with none)
    if r.state == 6 && !env.event {
        try_recovery(r, env, me, pos, fwd, upright, dt);
    }
    let Some(path) = r.path.as_ref() else {
        // standing, waiting for a rail
        r.v = 0.0;
        return Some((pos, fwd));
    };
    let (q, d) = path.at(path.t);
    let dir = if path.reverse { -d } else { d };
    // the speed: the rail's, with the car ahead along the rail
    let want = path.speed;
    // (a run-time rail: F-47175's grid branch; the next rail +192 and the pass holder's +104 on these rails are not
    // traced - taken as none)
    let ahead = if path.reverse {
        Ahead { j: usize::MAX, d: 0.0, vt: 0.0, blocking: None, player_across: false }
    } else {
        let geom = path.geom.clone();
        search_ahead(r, env, me, q, dir, Some(geom.run()), (None, None), None, [Some(GatherRail::RunTime), None, None], 0.0)
    };
    let (mut a, chosen) = control(r, r.v, want, half_len, &ahead, env.others, None, None, None, dt);
    r.slot = chosen;
    if env.hold {
        a = -r.ty.max_decel;
    }
    integrate(r, a, dt);
    // the follower's step (F-50894): the phase on by the distance; past the end the rest carries onto the next rail
    let d = r.v * dt;
    let path = r.path.as_mut().unwrap();
    let over = path.step(d);
    let (p, dd) = path.pose();
    let f = if path.reverse { -dd } else { dd };
    // the footprint probe
    let inst = env.others.get(me).map(|o| o.inst);
    if (!semi || r.state == 8) && inst.is_some_and(|i| probe_hit(env, me, &i, half_len, hw)) {
        if r.state == 8 {
            r.path = None;
            r.state = 6;
            r.stuck.1 = 0.0;
            return Some((p, f));
        }
        return None;
    }
    if r.state == 8 {
        // (F-82779: the standing time zeroed, the backing timer; past 4 s the car waits for a recovery rail and the
        // tracker is zeroed, tries too - F-36541)
        r.stuck.0 = 0.0;
        r.stuck.1 += dt;
        if r.stuck.1 > 4.0 {
            r.path = None;
            r.state = 6;
            r.stuck = (0.0, 0.0, 0);
        }
        return Some((p, f));
    }
    let Some(rest) = over else { return Some((p, f)) };
    if r.state == 7 {
        // on to the rail back onto the lane, at its start and the rest on
        match r.then.take() {
            Some(mut next) => {
                next.t = 0.0;
                if rest > 0.0 {
                    next.t = next.advance(0.0, rest).clamp(0.0, 1.0);
                }
                r.take_path(next, 6);
            }
            None => {
                r.path = None;
                r.state = 6;
            }
        }
        clear_tracker(r);
        return Some(r.path.as_mut().map_or((p, f), |q| q.pose()));
    }
    // (state 6 at its rail's end: the tracker cleared - F-87488's switch)
    if switch_clears_tracker(r.state, env.event, true) {
        clear_tracker(r);
    }
    if let Some((lane, t)) = r.path.as_ref().and_then(|p| p.end) {
        r.path = None;
        r.state = 1;
        // onto the lane at the rail's end phase (a rail set anew: its curve speed taken at the phase), the rest on
        if let Some(&k) = env.lanes.links.get(lane).and_then(|ls| ls.first()) {
            r.link = k;
            r.set_phase(env.nav, env.lanes, t);
            let c = &env.nav.curves[lane];
            r.ds = set_rail_speed(c, t);
            r.ds_lane = lane;
            if rest > 0.0 {
                r.set_phase(env.nav, env.lanes, carry_phase(c, t, rest));
            }
        }
        r.on = [false; 3];
        r.choose_next_env(env);
        let (p, f) = r.pose(env.nav);
        return Some((p, f));
    }
    r.path = None;
    Some((p, f))
}

/// How a riding car stands on the ground (car update F-22467, decoded 2026-10-20): the
/// rail gives only x / z and the heading; the height and tilt come from a ground probe
/// (mask 0x800) set up one frame and read the next, and the up and forward are the means of
/// the last eight.
#[derive(Clone, Debug)]
pub struct Seat {
    /// the y the rail target takes (car +2324: the car's own y of the last update)
    y: f32,
    /// the probe set up last frame: the target then (position, travel direction)
    pending: Option<(Vec3, Vec3)>,
    /// what the car takes when no probe result comes (car +2272 .. +2320): position, up, travel
    raw: (Vec3, Vec3, Vec3),
    /// the last eight ups and forwards (rings at car +2368 / +2528)
    ups: [Vec3; 8],
    fwds: [Vec3; 8],
    k: usize,
    frames: u32,
    /// the car's position and up of the last update (before the drawn 0.025 m drop)
    at: (Vec3, Vec3),
}

/// The drawn wheels' lock (F-37906, written after the render update in F-46062): set
/// when the speed controller's entry in charge is the car ahead (car +420 == 0), the mean of the
/// last 4 updates' asked acceleration (car +2336: count, next, sum, 4 entries, pushed by
/// F-60856 from F-43567 every update) is a deceleration of at least fMaxDecel, and
/// |car +208|^2 >= 4.9961185 (0x409FE034, (5 mph)^2; car +208 the velocity F-84582 makes, `newton_velocity`). Kept per update index
/// (owner byte +557 + U, U flipping each update): the render update reads the byte of its index
/// before it is written, so the drawn wheels see the bit of two updates before; while it is set
/// they do not turn. The car's skid sound reads the rule directly (F-50163).
#[derive(Clone, Debug, Default)]
pub struct Lock {
    n: u8,
    i: u8,
    sum: f32,
    e: [f32; 4],
    since: f32,
    prev: Option<Vec3>,
    buf: [bool; 2],
    u: usize,
    now: bool,
}

impl Lock {
    /// One 30 Hz update: push `x` (car +408), work the bit out with the car's place change.
    pub fn update(&mut self, pos: Vec3, x: f32, slot: Option<usize>, max_decel: f32, dt: f32) {
        let Some(v) = self.track(pos, dt) else { return };
        // (F-60856)
        let i = self.i as usize;
        if self.n == 0 {
            self.e[0] = x;
            self.sum = x;
            self.i = 1;
            self.n = 1;
        } else if self.n < 4 {
            self.e[i] = x;
            self.sum += x;
            self.i += 1;
            self.n += 1;
        } else {
            self.sum -= self.e[i];
            self.e[i] = x;
            self.sum += x;
            self.i += 1;
        }
        if self.i >= 4 {
            self.i = 0;
        }
        self.now = lock_rule(slot, self.sum, self.n, max_decel, v);
        self.buf[self.u] = self.now;
        self.u ^= 1;
    }
    /// The car's velocity +208 on a 30 Hz update (F-53150: F-84582 from the place change, in every state); None
    /// between updates.
    pub fn track(&mut self, pos: Vec3, dt: f32) -> Option<Vec3> {
        self.since += dt;
        if self.since < 1.0 / 30.0 - 1e-4 {
            return None;
        }
        let t = self.since;
        self.since = 0.0;
        let v = self.prev.map_or(Vec3::ZERO, |p| newton_velocity(p, pos, t));
        self.prev = Some(pos);
        Some(v)
    }
    /// The bit the render update sees (two updates old).
    pub fn drawn(&self) -> bool {
        self.buf[self.u]
    }
    /// F-37906 now (the skid sound).
    pub fn now(&self) -> bool {
        self.now
    }
}

/// F-37906's rule: the entry in charge is the car ahead (+420 == 0); the ring's mean (sum x 1 / count by `fdivs`, 0 with
/// no entry) negated is not below fMaxDecel (type +148); |v|^2 = fma(z, z, fma(x, x, y y)) of the velocity +208 is not
/// below 4.9961185 (0x409FE034). Both tests `fcmpu` + `blt` (a NaN passes them).
pub fn lock_rule(slot: Option<usize>, sum: f32, n: u8, max_decel: f32, v: Vec3) -> bool {
    use crate::physics::exact::fma;
    if slot != Some(0) {
        return false;
    }
    let mean = if n == 0 { 0.0 } else { sum / n as f32 };
    if -mean < max_decel {
        return false;
    }
    !(fma(v.z, v.z, fma(v.x, v.x, v.y * v.y)) < f32::from_bits(0x409F_E034))
}

/// |car +208| as the car update stores it in +412 with no rail (F-50624 with the body active, F-30960 without a rail):
/// fsqrts(fmadds(z, z, fmadds(x, x, y y))).
pub fn measured_speed(v: Vec3) -> f32 {
    use crate::physics::exact::fma;
    fma(v.z, v.z, fma(v.x, v.x, v.y * v.y)).sqrt()
}

/// F-84582: the velocity (car +208) from the position change over `dt` as the vector unit makes it - the change times
/// the reciprocal of dt refined by one Newton step: r0 = 1 / dt (`vrefp`, the recompiled code's exact reciprocal), r =
/// r0 (-(r0 dt - 1)) + r0 with 1.0, each product and sum rounded on its own (`vnmsubfp` / `vmaddfp` as
/// the recompiled code computes them), denormals flushed. (dt <= 0: the original keeps the old velocity; never here.)
pub fn newton_velocity(old: Vec3, new: Vec3, dt: f32) -> Vec3 {
    let r0 = ftz(1.0 / ftz(dt));
    let e = -vs(vm(r0, dt), 1.0);
    let r = va(vm(r0, e), r0);
    Vec3::new(vm(vs(new.x, old.x), r), vm(vs(new.y, old.y), r), vm(vs(new.z, old.z), r))
}

/// The drawn body's pitch and roll from the car's change of velocity (render update
/// F-37033): a 30 Hz frame's velocity change dv (car +224: v - v of the last frame, v from
/// the positions, car +208), within 30 m (horizontal) of the focus pitch = clamp(dv . forward /
/// fMaxPitchDot) x fMaxPitchAngle (nose up when speeding up), roll = clamp(dv . right /
/// fMaxRollDot) x fMaxRollAngle (right side up: leaning out of a turn); beyond 30 m (0, 0). The
/// last ten samples averaged (F-14882); drawn turned about the car's right axis by the
/// pitch, then its back axis by the roll, within 30 m only; the wheels not turned.
#[derive(Clone, Debug, Default)]
pub struct Tilt {
    /// the position and velocity of the last sample, the time since it
    prev: Option<(Vec3, Vec3)>,
    since: f32,
    ring: [(f32, f32); 10],
    count: usize,
    idx: usize,
    sum: (f32, f32),
    /// within 30 m at the last sample
    near: bool,
}

/// `mcAmbientRenderCarType.xml`: +68 fMaxPitchDot, +72 fMaxRollDot, +76 fMaxPitchAngle, +80
/// fMaxRollAngle (F-58135) - the same for every type.
const PITCH_DOT: f32 = 0.5;
const ROLL_DOT: f32 = 0.1;
const PITCH_ANGLE: f32 = 2.0;
const ROLL_ANGLE: f32 = 3.0;

impl Tilt {
    /// One update: the car's position and axes now, its horizontal distance squared to the
    /// focus, the time step; a sample every 1/30 s (the original's manager frame).
    pub fn update(&mut self, pos: Vec3, fwd: Vec3, right: Vec3, dist2: f32, dt: f32) {
        self.since += dt;
        let frame = 1.0 / 30.0;
        if self.since < frame - 1e-4 {
            return;
        }
        let t = self.since;
        self.since = 0.0;
        let (vel, dv) = match self.prev {
            Some((p, v0)) => {
                // (F-84582: +208 the velocity, +224 = +208 - the old one)
                let v = newton_velocity(p, pos, t);
                (v, Vec3::new(vs(v.x, v0.x), vs(v.y, v0.y), vs(v.z, v0.z)))
            }
            None => (Vec3::ZERO, Vec3::ZERO),
        };
        self.prev = Some((pos, vel));
        self.near = dist2 < 900.0;
        let s = if self.near {
            ((dv.dot(fwd) / PITCH_DOT).clamp(-1.0, 1.0) * PITCH_ANGLE.to_radians(), (dv.dot(right) / ROLL_DOT).clamp(-1.0, 1.0) * ROLL_ANGLE.to_radians())
        } else {
            (0.0, 0.0)
        };
        if self.count < 10 {
            self.ring[self.idx] = s;
            self.count += 1;
            self.idx += 1;
        } else {
            if self.idx == 10 {
                self.idx = 0;
            }
            let o = self.ring[self.idx];
            self.sum = (self.sum.0 - o.0, self.sum.1 - o.1);
            self.ring[self.idx] = s;
            self.idx += 1;
        }
        self.sum = (self.sum.0 + s.0, self.sum.1 + s.1);
    }

    /// (diagnostics) the drawn pitch and roll in degrees
    pub fn degrees(&self) -> (f32, f32) {
        match self.quat() {
            Some(_) => (self.sum.0 / self.count as f32 * 57.29578, self.sum.1 / self.count as f32 * 57.29578),
            None => (0.0, 0.0),
        }
    }

    /// The turn to draw the body with (in the car's frame: x right, y up, -z forward).
    pub fn quat(&self) -> Option<Quat> {
        if !self.near || self.count == 0 {
            return None;
        }
        let n = self.count as f32;
        Some(Quat::from_rotation_x(self.sum.0 / n) * Quat::from_rotation_z(self.sum.1 / n))
    }
}

/// Half the probe segment (0x820390F4).
const PROBE_HALF: f32 = 1.75;

impl Seat {
    pub fn new(pos: Vec3, up: Vec3, fwd: Vec3) -> Seat {
        Seat { y: pos.y, pending: None, raw: (pos, up, fwd), ups: [up; 8], fwds: [fwd; 8], k: 0, frames: 0, at: (pos, up) }
    }
}

/// The ground under `p` (the probe: a vertical segment +- 1.75 m round it, the city's bound -
/// the original's collision class 0x800): contact point and its normal, facing up.
pub fn probe_ground(col: &WorldCollider, p: Vec3) -> Option<(Vec3, Vec3)> {
    // (F-18688 -> F-57005: one-sided faces, the nearest hit - collision.rs `probe_front`)
    let h = col.probe_front(p + Vec3::Y * PROBE_HALF, Vec3::NEG_Y, 2.0 * PROBE_HALF)?;
    Some((h.point, h.normal.normalize_or(Vec3::Y)))
}

/// Should the car set up a probe this frame (F-22467): trailers every frame; a standing
/// car never (offline); else on alternate frames by the car's id, every fourth beyond 80 m
/// from the focus unless it is tilted more than 5 degrees within 120 m.
fn probe_due(r: &Rider, frame: u32, dist2: f32, up_y: f32) -> bool {
    if r.ty.trailer {
        return true;
    }
    if r.v == 0.0 {
        return false;
    }
    let slot = r.id >> 5;
    if dist2 >= 6400.0 && !(up_y < 0.99619 && dist2 < 14400.0) {
        return (frame.wrapping_add(slot)) & 3 == 0;
    }
    (frame ^ slot) & 1 == 0
}

/// One frame of the seat: `p` / `dir` the rail's target (point, travel direction), `car` the
/// car's position and up now; `face` the way the car faces (the travel direction, or against
/// it backing up). Returns the car's position and rotation.
#[allow(clippy::too_many_arguments)]
fn seat_car(r: &mut Rider, col: &WorldCollider, p: Vec3, dir: Vec3, face: Vec3, car: (Vec3, Vec3, Vec3), due: bool) -> (Vec3, Quat) {
    let trailer = r.ty.trailer;
    let s = r.seat.get_or_insert_with(|| Seat::new(car.0, car.1, car.2));
    let t_pos = Vec3::new(p.x, s.y, p.z);
    // last frame's probe, read now
    let hit = s.pending.take().and_then(|(at, d)| probe_ground(col, at).map(|h| (h, d)));
    let (pos, up_raw, fwd_raw) = match hit {
        Some(((q, n), d)) => {
            let f = (d - n * d.dot(n)).normalize_or(d);
            // for the frames without a result: the contact moved towards the target in its plane
            let off = t_pos - q;
            s.raw = (q + off - n * off.dot(n), n, f);
            (q, n, f)
        }
        None => {
            let m = s.raw;
            // the next raw: the car's up kept, sliding to the target in that plane
            let (cp, cu) = s.at;
            let u = cu.normalize_or(Vec3::Y);
            let off = t_pos - cp;
            let f = (dir - u * dir.dot(u)).normalize_or(dir);
            s.raw = (cp + off - u * off.dot(u), u, f);
            m
        }
    };
    if due {
        s.pending = Some((t_pos, dir));
    }
    s.ups[s.k] = up_raw;
    s.fwds[s.k] = fwd_raw;
    s.k = (s.k + 1) % 8;
    let up = s.ups.iter().copied().sum::<Vec3>().normalize_or(Vec3::Y);
    // (a trailer's forward is not smoothed)
    let fwd = if trailer { fwd_raw } else { s.fwds.iter().copied().sum::<Vec3>().normalize_or(fwd_raw) };
    let f = if face.dot(dir) < 0.0 { -fwd } else { fwd };
    let right = f.cross(up).normalize_or(Vec3::X);
    let f = up.cross(right).normalize_or(f);
    s.y = pos.y;
    s.at = (pos, up);
    s.frames += 1;
    // (a car on a rail is drawn 0.025 m lower from its second frame on: F-37033 sets the skeleton's +236
    // on the first frame its instance has a rider (instance +452 -> +188) and from then subtracts 0.025 from
    // the drawn matrix's place y - the world's y, not along the car's up; no rider clears it. Recorded: the
    // taxi standing at a light, drawn c3.y 22.8026 against its instance's 22.8276; a parked car 0)
    let drawn = if s.frames > 1 { pos - Vec3::Y * 0.025 } else { pos };
    (drawn, Quat::from_mat3(&glam::Mat3::from_cols(right, up, -f)).normalize())
}

/// The rotation of a car facing `f` (3D) with the world's up as near up as it goes.
pub fn facing(f: Vec3) -> Quat {
    let right = f.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(f).normalize_or(Vec3::Y);
    Quat::from_mat3(&glam::Mat3::from_cols(right, up, -f)).normalize()
}

/// Half a car's bound: length and width. Traffic: its type's bound (`.r155`); the player:
/// the chassis bound; others: the model's extent.
/// F-46284 (the ambient obstacle's slot 3, GetSize): the car's physics instance (car +112; a proxy car with its owner's
/// instance: +52 +48 +28) -> its archetype (+4) -> the bound (+12): max (+16) - min (+32), x and z - the type's bound,
/// `AmbType::width` / `length` (`bound_size`); no archetype: 3 x 6 (0x40400000, 0x40C00000).
pub fn obstacle_size_of(ty: &AmbType) -> (f32, f32) {
    // (`bound_size`'s max - min of the file's bound: `bound_extents` without the flush, which no type's bound needs)
    if ty.width > 0.0 && ty.length > 0.0 {
        (ty.width, ty.length)
    } else {
        (3.0, 6.0)
    }
}

/// F-46284's arithmetic on the bound the instance's archetype holds (`Some((max, min))`; None: no archetype): the x and
/// z of max - min (`vsubfp`, flushed), else 3 x 6.
pub fn bound_extents(b: Option<(Vec3, Vec3)>) -> (f32, f32) {
    match b {
        Some((mx, mn)) => (vs(mx.x, mn.x), vs(mx.z, mn.z)),
        None => (3.0, 6.0),
    }
}

fn half_size(c: &super::CarInstance) -> (f32, f32) {
    if let Some(r) = c.rail.as_ref() {
        if r.ty.length > 0.0 {
            return (r.ty.length * 0.5, r.ty.width * 0.5);
        }
    }
    if c.is_player {
        let b = c.model.params.body_half;
        return (b.z.max(1.0), b.x.max(0.5));
    }
    let (l, w) = ((c.model.max.z - c.model.min.z) * 0.5, (c.model.max.x - c.model.min.x) * 0.5);
    (if l.is_finite() && l > 0.5 { l } else { c.model.params.body_half.z.max(1.0) }, if w.is_finite() && w > 0.3 { w } else { c.model.params.body_half.x.max(0.5) })
}

/// A parked car's render update's drawn steer (F-37033 with no behaviour, no trailer): car +3584 + 0 pushed into the
/// ring; the ring's mean while within 45 m of the focus ((dx, dz) the focus less the instance's place) or in the first
/// three updates, else 0.
pub fn parked_drawn_steer(ring: &mut SteerRing, park: f32, frames: u8, dx: f32, dz: f32) -> f32 {
    let near = dz.mul_add(dz, dx * dx) < 2025.0 || frames < 3;
    ring.push(park + 0.0);
    if near { ring.mean() } else { 0.0 }
}

/// Whether a car's render update runs this frame (F-37359, car slot 6's last step): always for a car with a behaviour,
/// one once physical (+500 bit 0x10, a parked car hit: `CarInstance::amb_hit`) or a trailer; a parked car only while its draw
/// object's update count +32 is under 3 (`CarInstance::amb_frames`): its first three updates after it took the instance,
/// then its wheels stay as drawn.
pub fn parked_render_runs(behaviour: bool, held: bool, frames: u8, trailer: bool) -> bool {
    behaviour || held || frames < 3 || trailer
}

impl Vehicles {
    /// The parked cars' render updates (F-37033 for a car with no behaviour that is no trailer, gated by
    /// `parked_render_runs`): dt != 0, the sample is car +3584 (+ 0, no rail), pushed into the draw object's ring; the
    /// front wheels turn by the ring's mean while the "near" byte holds: within 45 m of the focus, or the update count
    /// under 3; then the count goes up (F-14577). A held car's steer reaches +3584 within 8 updates and is 0 beyond 45 m.
    pub(super) fn parked_render(&mut self, dt: f32) {
        if dt == 0.0 {
            return;
        }
        let focus = self.amb_focus();
        for c in self.cars.iter_mut() {
            if c.rail.is_some() || (c.parked.is_none() && !c.park_held) || !parked_render_runs(false, c.amb_hit, c.amb_frames, false) {
                continue;
            }
            let m = instance_matrix(c);
            let s = parked_drawn_steer(&mut c.amb_steer, c.park_steer, c.amb_frames, focus.x - m[3].x, focus.z - m[3].z);
            for (i, w) in c.state.wheels.iter_mut().enumerate() {
                w.steer = if i <= 1 { s } else { 0.0 };
            }
            c.amb_frames = (c.amb_frames + 1).min(3);
        }
    }

    /// The level's objects as the original's level holds them (the city's grid, every car's composite with its state and
    /// type words, walkers, props) for the cameras' probes, and the instance word of car `k` in it.
    pub fn probe_level<'a>(&'a self, col: &'a WorldCollider) -> RailLevel<'a> {
        RailLevel::new(col, &self.cars, &self.data_root, &self.walkers, &self.level)
    }

    pub fn probe_inst(k: usize) -> u32 {
        rail_car_inst(k)
    }

    /// Move the traffic cars: along their lanes, on their rails, back from physics; the
    /// trailers behind their cabs.
    pub(super) fn ride_traffic(&mut self, nav: &NavSys, col: &WorldCollider, dt: f32) {
        // the player's standing time (mcPlayer +872, F-16834: speed under 2.2352 m/s adds the frame's time, else 0;
        // the port takes the car's |velocity| at the traffic's step)
        if let Some(p) = self.cars.iter().find(|c| c.is_player) {
            self.player_idle = if p.state.vel.length() < f32::from_bits(0x400F_0D84) { self.player_idle + dt } else { 0.0 };
        }
        // the cruising ghosts: the request pool's step and their rides (not in `cars`: not drawn, no physics)
        self.ghost_frame(nav, col, dt);
        if !self.cars.iter().any(|c| c.rail.is_some()) {
            return;
        }
        let root = self.data_root.clone();
        let level = &RailLevel::new(col, &self.cars, &root, &self.walkers, &self.level);
        let nav = rail_nav(nav);
        let lanes = lanes(nav);
        let others: Vec<Other> = self
            .cars
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let (hl, hw) = half_size(c);
                let r = c.rail.as_ref();
                let on = r.filter(|r| r.state == 1).map(|r| {
                    let l = &nav.links[r.link];
                    (l.lane, r.next.map(|n| nav.links[n].lane), rail_speed(nav, r.link).1)
                });
                Other {
                    pos: c.state.pos,
                    fwd: c.state.forward(),
                    right: c.state.right(),
                    back: glam::Mat3::from_quat(c.state.rot).z_axis,
                    row0: glam::Mat3::from_quat(c.state.rot).x_axis,
                    inst: instance_matrix(c),
                    vel: c.state.vel,
                    half_len: hl,
                    half_wid: hw,
                    size: self.obstacle_size(c),
                    speed: r.map_or(0.0, |r| r.v),
                    beh: self.sel_behaviour(nav, k, r),
                    lane640: if c.is_player { self.driver_lanes.get(&c.uid).and_then(|d| d.best).map(|(l, _)| (l, self.lane_freeway(nav, l))) } else { None },
                    idle: if c.is_player { self.player_idle } else { 0.0 },
                    on,
                    player: c.is_player,
                    ambient: r.is_some() || c.parked.is_some(),
                    active: r.is_some_and(|r| r.state == 4) || (r.is_none() && c.parked.is_none()),
                    cab_or_trailer: r.is_some_and(|r| r.ty.cab || r.ty.trailer),
                    key: r.map_or(0, |r| r.rng_id()),
                    kind: crate::brainturn::ObjectInfo::from_car(0, c).kind,
                    nearest: r.and_then(|r| r.kept).and_then(|(j, _)| self.cars.get(j)?.rail.as_ref().map(|x| x.rng_id())),
                }
            })
            .collect();
        let hold = self.frozen || self.grid_hold.is_some();
        let event = self.race_event;
        // (the player's flat forward, the tracker +224 as F-83397 writes it from the car's matrix this frame, and the
        // junction ahead found last frame - the original's order of the two against the car update is not traced)
        let player_ahead = self.player_junction.and_then(|j| self.cars.iter().find(|c| c.is_player).map(|c| (flat_forward(glam::Mat3::from_quat(c.state.rot).z_axis), j)));
        static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let frame = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let parity = frame & 1 == 1;
        let focus = self.amb_focus();
        let blockers = self.frame_blockers(nav);
        let paths: Vec<Option<(bool, bool)>> = self.cars.iter().map(|c| c.item.and_then(|h| self.layout.get(h as usize)?.as_ref()?.path.as_ref().map(|p| (p.referenced, p.fired)))).collect();
        let shared = std::cell::Cell::new(self.park_rng);
        let (rail_cars, grid) = self.gather_lists(nav);
        let pool0 = std::cell::Cell::new(0);
        let ghost_pool0 = self.ghosts.live.cars.iter().filter(|g| holds_pool0(&g.rider)).count();
        let env = Env { nav, lanes, col, others: &others, signals: &self.signals, blockers: &blockers, clock: self.clock, event, player_ahead, hold, parity, paths: &paths, shared: &shared, rail_cars: &rail_cars, grid: &grid, pool0: &pool0, level };
        // MCLA_TRAFFIC_TRACE=file: every riding car ten times a second (time, id, model, x, y,
        // z, speed, lane speed, junction lane, offset, entries on, standing gap, distance and
        // margin to the car ahead, acceleration, state, ...; the last column: the drawn heading in degrees)
        static TRACE: std::sync::Mutex<Option<(std::io::BufWriter<std::fs::File>, f32)>> = std::sync::Mutex::new(None);
        let mut guard = TRACE.lock().unwrap();
        if guard.is_none() {
            if let Ok(p) = std::env::var("MCLA_TRAFFIC_TRACE") {
                if let Ok(f) = std::fs::File::create(p) {
                    *guard = Some((std::io::BufWriter::new(f), -1.0));
                }
            }
        }
        let mut trace = match guard.as_mut() {
            Some((w, last)) if self.clock - *last >= std::env::var("MCLA_TRAFFIC_TRACE_DT").ok().and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.1) => {
                *last = self.clock;
                Some(w)
            }
            _ => None,
        };
        let mut waits: Vec<(usize, Vec3)> = Vec::new();
        for i in 0..self.cars.len() {
            let (half, _) = half_size(&self.cars[i]);
            // (the pool-0 rails the others hold as this car's update starts)
            pool0.set(ghost_pool0 + self.cars.iter().enumerate().filter(|&(j, c)| j != i && c.rail.as_ref().is_some_and(holds_pool0)).count());
            let c = &mut self.cars[i];
            let upright = (c.state.rot * Vec3::Y).y >= 0.0;
            let Some(r) = c.rail.as_mut() else { continue };
            let pose = match r.state {
                0 => {
                    if let Some(w) = trace.as_mut() {
                        use std::io::Write;
                        let p = c.state.pos;
                        let f = c.state.forward();
                        let _ = writeln!(w, "{:.2}	{}	{}	{:.2}	{:.2}	{:.2}	{:.3}	{:.3}	{:.3}	0	000	0	0	0	0	0	{}	0", self.clock, r.rng_id(), c.model.name, p.x, p.y, p.z, c.state.speed(), f.x, f.z, r.partner.unwrap_or(0));
                    }
                    continue;
                }
                4 | 5 => {
                    // (F-87488's switch: state 4 clears the pass tracker every frame)
                    if switch_clears_tracker(r.state, env.event, false) {
                        clear_tracker(r);
                    }
                    if let Some(w) = trace.as_mut() {
                        use std::io::Write;
                        let p = c.state.pos;
                        let _ = writeln!(w, "{:.2}	{}	{}	{:.2}	{:.2}	{:.2}	{:.3}	{:.2}	{}	0	000	{:.2}	0	0	0	{}	0	0", self.clock, r.rng_id(), c.model.name, p.x, p.y, p.z, c.state.speed(), c.input.brake, c.placed as u8, c.input.throttle, r.state);
                    }
                    // (F-50624: states 4 / 5 have no rail - behaviour vfuncs 12 / 13, F-79450 / F-58957, clear it:
                    // F-99671 -> F-78813, the follower's +156 -, so every update +412 = |car +208|: an active body
                    // (F-17857) at once and nothing more that update; else after the behaviour's step, F-30960's
                    // no-rail exit (a = 0, which F-58614 keeps))
                    let measured = r.lock.track(c.state.pos, dt).map(measured_speed);
                    // (F-17857: with the body active the update returns after +412 in every state - a woken car at rest
                    // takes no recovery try while the physics moves it)
                    let body_active = r.in_level && r.active;
                    if let Some(v) = measured.filter(|_| r.state == 4 || body_active) {
                        r.v = v;
                    }
                    // physical: the physics level's rest rule takes it out (state 5: vehicles.rs,
                    // loose.rs `ambient_frame`); a contact makes it active again
                    if r.in_level && (r.state == 4 || body_active) {
                        // (not active: the behaviour's state 4 and F-58614's offset timer)
                        if !r.active {
                            rider_offset(r, env.clock);
                        }
                        continue;
                    }
                    // (without the level: under 0.1 m/s and 0.1 rad/s for 0.5 s - the port's own)
                    let (sp, w) = (c.state.speed(), c.state.ang_vel.length());
                    if r.state == 4 && !r.in_level {
                        if sp < 0.1 && w < 0.1 {
                            r.rest += dt;
                            if r.rest > 0.5 {
                                r.state = 5;
                                r.stand = 0.0;
                            }
                        } else {
                            r.rest = 0.0;
                        }
                        // (no collider: never active, F-58614's offset timer)
                        rider_offset(r, env.clock);
                        continue;
                    }
                    if sp > 0.3 && !r.in_level {
                        r.state = 4;
                        r.rest = 0.0;
                        rider_offset(r, env.clock);
                        continue;
                    }
                    let fwd = c.state.forward();
                    let tried = try_recovery(r, &env, i, c.state.pos, fwd, upright, dt);
                    // (F-30960 after the behaviour's step: the measured speed; then F-58614's offset timer)
                    if let Some(v) = measured.filter(|_| r.state == 5) {
                        r.v = v;
                    }
                    // (F-58614 after the behaviour's update: the offset timer)
                    rider_offset(r, env.clock);
                    if !tried {
                        continue;
                    }
                    // back on a rail: kinematic from here
                    r.path.as_mut().unwrap().pose()
                }
                6..=8 => {
                    let (pos, fwd) = (c.state.pos, c.state.forward());
                    let stepped = step_path(r, &env, i, pos, fwd, half, upright, dt);
                    rider_offset(r, env.clock);
                    match stepped {
                        Some(p) => p,
                        None => {
                            let back = matches!(r.path.as_ref(), Some(p) if p.reverse);
                            r.state = 4;
                            r.physical = true;
                            r.wake = true;
                            r.path = None;
                            r.then = None;
                            r.rest = 0.0;
                            // (back from physics the seat starts from the car's pose, as `let_go`)
                            r.seat = None;
                            c.state.vel = c.state.forward() * if back { -r.v } else { r.v };
                            continue;
                        }
                    }
                }
                // a race path's car let go of by its path (unload): no rail, not posed; its speed
                // +412 is what it measures, |car +208| (F-30960 with no rail; +416 = 0), on the car update's
                // 30 Hz tick from the place change (F-53150)
                10 => {
                    if let Some(v) = r.lock.track(c.state.pos, dt) {
                        r.v = measured_speed(v);
                    }
                    c.state.vel = Vec3::ZERO;
                    continue;
                }
                // a race path's car on the path's rails
                9 => {
                    let p = step_route(r, &env, i, half, dt);
                    rider_offset(r, env.clock);
                    p
                }
                _ => {
                    let p = step(r, &env, i, half, upright, dt);
                    rider_offset(r, env.clock);
                    // refused by the light this frame (the stop entry is made only then): the
                    // junction counts it as waiting (F-16850)
                    if let Some(ji) = r.stop_id {
                        waits.push((ji, c.state.pos));
                    }
                    p
                }
            };
            let (p, f) = pose;
            let v = r.v;
            let back = matches!(r.path.as_ref(), Some(pp) if pp.reverse);
            // on the ground (F-22467)
            let dir = if back { -f } else { f };
            let dist2 = Vec3::new(c.state.pos.x - focus.x, 0.0, c.state.pos.z - focus.z).length_squared();
            let due = probe_due(r, frame, dist2, c.state.up().y);
            let car = (c.state.pos, c.state.up(), dir);
            let (p, rot) = seat_car(r, col, p, dir, f, car, due);
            c.state.relocate(p, rot);
            c.state.vel = dir * v;
            if let Some(w) = trace.as_mut() {
                use std::io::Write;
                let (lim, junction) = rail_speed(nav, r.link);
                let _ = writeln!(w, "{:.2}	{}	{}	{:.2}	{:.2}	{:.2}	{:.3}	{:.2}	{}	{:.0}	{}{}{}	{:.2}	{:.2}	{:.2}	{:.3}	{}	{}	{}	{:.3}	{:.2}	{:.2}	{:.1}", self.clock, r.rng_id(), c.model.name, c.state.pos.x, c.state.pos.y, c.state.pos.z, v, lim, junction as u8, r.off, r.on[0] as u8, r.on[1] as u8, r.on[2] as u8, r.gap, r.ahead.0, r.ahead.1, r.a, r.state, r.obst_id.and_then(|j| others.get(j)).map_or(0, |o| o.key), nav.links[r.link].flags, col.ground(c.state.pos, 3.0, 6.0).map_or(f32::NAN, |g| g.point.y - c.state.pos.y), r.tilt.degrees().0, r.tilt.degrees().1, c.state.forward().z.atan2(c.state.forward().x).to_degrees());
            }
            c.state.ang_vel = Vec3::ZERO;
            let wr = c.model.params.wheel_radius.max(0.2);
            // (the drawn wheels turn at 0.6 of the rolling rate, within 75 m: F-37033)
            let turn = if dist2 < SPIN_REACH2 && !r.lock.drawn() { 0.6 * if back { -v } else { v } / wr * dt } else { 0.0 };
            for w in c.state.wheels.iter_mut() {
                w.spin = wrap_spin(w.spin + turn);
                w.steer = 0.0;
                w.grounded = true;
            }
            c.placed = true;
        }
        drop(guard);
        self.tow_trailers(col, dt);
        // the rails' car lists and the driver cars' lanes
        self.stamp_rails(nav);
        self.update_driver_lanes(nav);
        // the drawn bodies' pitch and roll (every traffic car, trailers too)
        for c in self.cars.iter_mut() {
            let (pos, fwd, right) = (c.state.pos, c.state.forward(), c.state.right());
            let m = instance_matrix(c);
            let hub_z = c.model.own_wheels.and_then(|h| h[0]).map_or(0.0, |h| h.z);
            let Some(r) = c.rail.as_mut() else { continue };
            let dist2 = Vec3::new(pos.x - focus.x, 0.0, pos.z - focus.z).length_squared();
            r.tilt.update(pos, fwd, right, dist2, dt);
            // (the drawn steer, F-37033 with dt != 0: one sample a frame into the draw object's ring - within 45 m of the
            // focus and on a rail, the rail's derivative a little ahead (its slot 6 and slot 10: the lane, the arc-line-arc
            // rail or the curve of the backing and race path rails) against the car's heading; 0 otherwise (states 4 / 5
            // have no rail, F-78813; a trailer has no behaviour). The front wheels turn by the ring's mean within 45 m.)
            if dt != 0.0 {
                let (dx, dz) = (focus.x - m[3].x, focus.z - m[3].z);
                let near = dz.mul_add(dz, dx * dx) < 2025.0;
                let d = if !near {
                    None
                } else {
                    match r.state {
                        1 => nav.links.get(r.link).and_then(|l| nav.curves.get(l.lane)).filter(|cv| cv.pts.len() >= 2).map(|cv| crate::railroute::deriv(cv, steer_look(r.t, r.ds, hub_z))),
                        6..=8 => r.path.as_ref().map(|p| p.look(hub_z)),
                        9 => r.route.as_ref().map(|rt| rt.look(hub_z)),
                        _ => None,
                    }
                };
                let x = d.map_or(0.0, |d| steer_of(-m[2], d));
                c.amb_steer.push(x);
                r.steer = if near { c.amb_steer.mean() } else { 0.0 };
            }
            for (i, w) in c.state.wheels.iter_mut().enumerate() {
                w.steer = if i <= 1 { r.steer } else { 0.0 };
            }
            // (the lock's window: car +408 pushed every update, in every state - frozen while the
            // physics has the car, 0 resting (state 5) or in state 4 without its collider; a
            // trailer never works its bit out)
            if !r.ty.trailer {
                let x = if r.state == 5 || (r.state == 4 && !r.active) { 0.0 } else { r.a };
                let (slot, max_decel) = (r.slot, r.ty.max_decel);
                r.lock.update(pos, x, slot, max_decel, dt);
            }
        }
        // (test hook MCLA_TRAFFIC_RAM=s: every s seconds the player's car is put 9 m behind the
        // riding car nearest it within 80 m, on its heading, 12 m/s faster than it)
        if let Some(every) = std::env::var("MCLA_TRAFFIC_RAM").ok().and_then(|x| x.parse::<f32>().ok()) {
            static LAST: std::sync::Mutex<f32> = std::sync::Mutex::new(-100.0);
            let mut last = LAST.lock().unwrap();
            if self.clock - *last >= every {
                let me = self.cars.iter().position(|c| c.is_player);
                if let Some(pi) = me {
                    let m = self.cars[pi].state.pos;
                    let semi = std::env::var("MCLA_TRAFFIC_RAM_SEMI").is_ok();
                    // (MCLA_TRAFFIC_RAM_PARKED: a held parked car standing still instead)
                    let parked = std::env::var("MCLA_TRAFFIC_RAM_PARKED").is_ok();
                    let pick = (0..self.cars.len()).filter(|&i| if parked { self.cars[i].park_held && self.cars[i].state.vel.length() == 0.0 && self.cars[i].state.pos.distance(m) > 20.0 } else { self.cars[i].rail.as_ref().is_some_and(|r| (r.state == 1 && r.v > 2.0 && !semi && !r.ty.cab && !r.ty.trailer) || (semi && r.ty.trailer && r.state == 0 && !r.physical)) } && self.cars[i].state.pos.distance(m) < if parked { 400.0 } else { 150.0 }).min_by(|&a, &b| self.cars[a].state.pos.distance(m).total_cmp(&self.cars[b].state.pos.distance(m)));
                    // (the spot the player is put at and the run-up to the target clear of every other car: a car
                    // following within the 9 m would have the player dropped onto it)
                    let pick = pick.filter(|&k| {
                        let (tp, tr) = (self.cars[k].state.pos, self.cars[k].state.rot);
                        let from = std::env::var("MCLA_TRAFFIC_RAM_FROM").unwrap_or_default();
                        let f = if parked || from == "side" { tr * Vec3::NEG_X } else if from == "front" { tr * Vec3::Z } else { tr * Vec3::NEG_Z };
                        let reach = if semi { 13.0 } else { 9.0 };
                        (0..self.cars.len()).all(|j| j == k || self.cars[j].is_player || {
                            let d = self.cars[j].state.pos - (tp - f * reach);
                            let along = d.dot(f);
                            let across = (d - f * along).length();
                            !(along > -4.0 && along < reach && across < 3.5)
                        })
                    });
                    if let Some(k) = pick {
                        *last = self.clock;
                        let (tp, tr, tv) = (self.cars[k].state.pos, self.cars[k].state.rot, if parked { 0.0 } else if semi { self.cars[k].state.vel.length() } else { self.cars[k].rail.as_ref().unwrap().v });
                        // (a parked car: from its right side; MCLA_TRAFFIC_RAM_FROM=side / front: a riding car from its
                        // right side or head on, MCLA_TRAFFIC_RAM_DV: the speed over it, 12 m/s if not given)
                        let from = std::env::var("MCLA_TRAFFIC_RAM_FROM").unwrap_or_default();
                        let dv = std::env::var("MCLA_TRAFFIC_RAM_DV").ok().and_then(|x| x.parse::<f32>().ok()).unwrap_or(12.0);
                        let f = if parked || from == "side" { tr * Vec3::NEG_X } else if from == "front" { tr * Vec3::Z } else { tr * Vec3::NEG_Z };
                        let tv = if from == "side" { 0.0 } else if from == "front" { -tv } else { tv };
                        let p = &mut self.cars[pi];
                        // (as a reset puts a car: a fresh state, placed on the ground by the step)
                        p.state = crate::physics::CarState::new(tp - f * if semi { 13.0 } else { 9.0 }, (-f.x).atan2(-f.z));
                        p.placed = false;
                        p.state.vel = f * (tv + dv);
                        p.pose_history.reset();
                        log::info!("traffic ram {} {} at {:?}", self.cars[k].rail.as_ref().map_or(self.cars[k].uid, |r| r.rng_id()), self.cars[k].model.name, tp);
                    }
                }
            }
        }
        if std::env::var_os("MCLA_TRAFFIC_TRACE").is_some() {
            for c in self.cars.iter().filter(|c| c.rail.as_ref().is_some_and(|r| r.active || r.physical)) {
                let r = c.rail.as_ref().unwrap();
                log::info!("ttr {} {} t {:.3} st {} act {} phys {} at {:?} v {:?} w {:?} up {:.3}", c.model.name, c.uid, self.clock, r.state, r.active, r.physical, c.state.pos, c.state.vel, c.state.ang_vel, c.state.up().y);
            }
        }
        // (test hook MCLA_TRAFFIC_RAM: each frame an active riding car rises faster than 1 m/s or tips past 25 degrees)
        if std::env::var_os("MCLA_TRAFFIC_RAM").is_some() {
            for c in self.cars.iter().filter(|c| c.rail.as_ref().is_some_and(|r| r.active)) {
                if c.state.vel.y > 1.0 || c.state.up().y < 0.906 {
                    log::info!("traffic fly {} {} t {:.3} at {:?} v {:?} w {:?} up {:.3}", c.model.name, c.uid, self.clock, c.state.pos, c.state.vel, c.state.ang_vel, c.state.up().y);
                }
            }
        }
        // (test hook MCLA_TRAFFIC_KNOCK=s: every s seconds the riding car nearest the player
        // within 60 m is knocked sideways - physical - to watch it find its lane again)
        if let Some(every) = std::env::var("MCLA_TRAFFIC_KNOCK").ok().and_then(|x| x.parse::<f32>().ok()) {
            static LAST: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);
            let mut last = LAST.lock().unwrap();
            if self.clock - *last >= every {
                *last = self.clock;
                let me = self.cars.iter().find(|c| c.is_player).map(|c| c.state.pos);
                let pick = me.and_then(|m| (0..self.cars.len()).filter(|&i| self.cars[i].rail.as_ref().is_some_and(|r| r.state == 1) && self.cars[i].state.pos.distance(m) < 60.0).min_by(|&a, &b| self.cars[a].state.pos.distance(me.unwrap()).total_cmp(&self.cars[b].state.pos.distance(me.unwrap()))));
                if let Some(k) = pick {
                    let c = &mut self.cars[k];
                    let r = c.rail.as_mut().unwrap();
                    r.physical = true;
                    r.wake = true;
                    r.state = 4;
                    r.rest = 0.0;
                    c.state.vel = c.state.forward() * r.v + c.state.right() * 5.0;
                    c.state.ang_vel = Vec3::Y * 1.5;
                    log::info!("traffic knock {} {}", r.rng_id(), c.model.name);
                }
            }
        }
        self.park_rng = shared.get();
        for (ji, p) in waits {
            self.signals.wait(ji, p);
        }
    }

    /// The trailers (slot 28 F-16044): the rear bogie trails the cab's hitch - the
    /// smallest turn that points the trailer from the hitch to where its bogie was, then moved
    /// so its hitch is on the cab's. A trailer whose cab is gone is let go (slot 29, reason 16:
    /// the spawner's next step removes it - the cars are not removed in the middle of the
    /// vehicles' update).
    fn tow_trailers(&mut self, col: &WorldCollider, dt: f32) {
        let focus = self.amb_focus();
        let ids: hashbrown::HashMap<u32, usize> = self.cars.iter().enumerate().filter_map(|(i, c)| c.rail.as_ref().filter(|r| r.state != 0).map(|r| (r.rng_id(), i))).collect();
        for i in 0..self.cars.len() {
            let Some(r) = self.cars[i].rail.as_mut().filter(|r| r.state == 0) else { continue };
            // (F-99638: no cab id, or no car with it - removed, reason 16)
            let (Some(cab), _) = trailer_cab(true, r.partner.map_or(-1, |id| id as i32), |id| ids.get(&(id as u32)).copied()) else {
                r.partner = None;
                continue;
            };
            // (once physical a trailer never trails again: +500 bit 0x10, set by F-35377, is
            // cleared only at the car's init, F-40231 - it lies where it came to rest)
            if r.physical {
                continue;
            }
            let (cp, cr) = (self.cars[cab].state.pos, self.cars[cab].state.rot);
            let h = cp + cr * CAB_HITCH;
            let t = &mut self.cars[i];
            let (tp, tr) = (t.state.pos, t.state.rot);
            let a = tp + tr * Vec3::new(0.0, TRAILER_HITCH.y, TRAILER_BOGIE);
            let d = (a - h).normalize_or(tr * Vec3::Z);
            let z = (tr * Vec3::Z).normalize_or(Vec3::Z);
            let rot = (Quat::from_rotation_arc(z, d) * tr).normalize();
            let pos = h - rot * TRAILER_HITCH;
            // on the ground as every car (probed every frame, its forward not smoothed)
            let dir = rot * Vec3::NEG_Z;
            let car = (tp, tr * Vec3::Y, dir);
            let (pos, rot) = match t.rail.as_mut() {
                Some(r) => seat_car(r, col, pos, dir, dir, car, true),
                None => (pos, rot),
            };
            let v = if dt > 0.0 { (pos - tp) / dt } else { Vec3::ZERO };
            t.state.relocate(pos, rot);
            t.state.vel = v;
            t.state.ang_vel = Vec3::ZERO;
            let speed = v.dot(rot * Vec3::NEG_Z);
            let wr = t.model.params.wheel_radius.max(0.2);
            let near = Vec3::new(pos.x - focus.x, 0.0, pos.z - focus.z).length_squared() < SPIN_REACH2;
            for w in t.state.wheels.iter_mut() {
                // (the drawn wheels at 0.6 of the rolling rate, as the riding cars': F-37033)
                w.spin = wrap_spin(w.spin + if near { 0.6 * speed / wr * dt } else { 0.0 });
                w.steer = 0.0;
                w.grounded = true;
            }
            t.placed = true;
        }
    }

    /// Hitch a trailer to a cab just made (slot 5 F-77422 -> F-45988): with p 0.75
    /// one of the five trailers, its hitch on the cab's, facing the same way.
    /// The pool's spawn step and the car's init for the ambient car `idx` just made (`spot`: a parked spot's car):
    /// F-42724 draws the shared MWC (rec+832 = lo & 0x7FFFFFFF, the car MWC's seed); F-92604 gives a cab or trailer
    /// (F-42724 sets rec+836 for type +111 / +112; the spawners' requests carry the id -1, rec+808) an id from the
    /// same shared MWC: r = (lo & 0x7FFFFFFF) % 512, id = ((r << 7 | the type's index, F-54952) << 5) | the peer
    /// (F-78245), drawn again while a car holds that id (F-10061; at most 20 draws) - here: a car of the same type with
    /// the same r; then `init_draws` with the second shared MWC. Sets the car's life, id and lights' offsets.
    pub(super) fn amb_init(&mut self, idx: usize, ty: &AmbType, spot: bool) -> InitDraws {
        let seed = self.park_rng.next() & 0x7FFF_FFFF;
        if ty.cab || ty.trailer {
            let name = self.cars[idx].model.name.clone();
            let cars = &self.cars;
            let r = id_draws(&mut self.park_rng, |r| cars.iter().enumerate().any(|(k, c)| k != idx && c.amb_id == r && c.model.name == name));
            self.cars[idx].amb_id = r;
        }
        // (the manager's byte +0x64234, read by F-60230: its writer is not traced; 0 as everywhere in the port)
        let d = init_draws(seed, &mut self.amb_rng2, spot, ty.cab, ty.proxy, false);
        let c = &mut self.cars[idx];
        c.amb_born = self.clock;
        c.amb_life = d.life;
        c.amb_lamps.hour_offset = d.hour;
        c.amb_lamps.blink_wrap = d.blink_wrap;
        c.amb_lamps.blink_on = d.blink_on;
        d
    }

    /// `hitch`: the cab's init draw (`InitDraws::hitch`, F-77422: the car MWC's U x 2^-23 < 0.75).
    pub(super) fn hitch_trailer(&mut self, gpu: &crate::render::Gpu, root: &std::path::Path, nav: &NavSys, cab: usize, hitch: bool) {
        let Some(r) = self.cars[cab].rail.as_mut() else { return };
        if !r.ty.cab || !hitch {
            return;
        }
        let models = &self.models;
        let Some((name, shade)) = trailer_pick(&mut r.mwc, |n| models.iter().any(|m| m.name.eq_ignore_ascii_case(n))) else { return };
        let (cab_id, link, s) = (r.rng_id(), r.link, r.s);
        let Some(mi) = self.models.iter().position(|m| m.name.eq_ignore_ascii_case(name)) else { return };
        let (cp, cr) = (self.cars[cab].state.pos, self.cars[cab].state.rot);
        let pos = cp + cr * CAB_HITCH - cr * TRAILER_HITCH;
        let model = self.models[mi].clone();
        let tty = *self.amb_types.entry(model.name.clone()).or_insert_with(|| AmbType::load(root, &model.name));
        let color = tty.color(shade).unwrap_or(super::TRAFFIC_PALETTE[shade % super::TRAFFIC_PALETTE.len()]);
        let f = cr * Vec3::NEG_Z;
        let idx = self.spawn(gpu, model, pos, (-f.x).atan2(-f.z), color, false);
        self.cars[idx].state.relocate(pos, cr);
        let mname = self.cars[idx].model.name.clone();
        let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| AmbType::load(root, &mname));
        let seed = (self.rand() * 16777216.0) as u32 | 1;
        let mut t = Rider::new(nav, ty, link, s, seed);
        t.state = 0;
        t.v = 0.0;
        t.partner = Some(cab_id);
        t.born = self.clock;
        // (the trailer's request through the pool: its seed, id and init draws, F-42724 / F-92604 / F-77422)
        let d = self.amb_init(idx, &ty, false);
        (t.life, t.off_timer, t.wait, t.bonus, t.mwc) = (d.life, d.off_timer, d.wait, d.bonus, d.car);
        self.cars[idx].placed = true;
        self.cars[idx].rail = Some(t);
        if let Some(r) = self.cars[cab].rail.as_mut() {
            r.partner = Some(seed);
        }
    }

    /// Something met a traffic car on a rail: from now on the physics has it (base slot 18
    /// F-35377, state 4). A trailer has no behaviour (car +452 = 0): it is physical from
    /// here (and never trails again); its cab is made active with it (F-15252 ->
    /// F-10339, the level's constraints join them).
    pub(super) fn let_go(&mut self, k: usize, _world: &crate::world::World) {
        let Some(r) = self.cars[k].rail.as_mut() else { return };
        if r.state == 0 {
            if r.physical {
                return;
            }
            r.physical = true;
            r.wake = true;
            let partner = r.partner;
            let cab = partner.and_then(|id| self.cars.iter().position(|c| c.rail.as_ref().is_some_and(|x| x.rng_id() == id && x.state != 0)));
            if let Some(cab) = cab {
                self.let_go(cab, _world);
            }
            return;
        }
        let c = &mut self.cars[k];
        let r = c.rail.as_mut().unwrap();
        if r.physical {
            // (a car at rest made active again: PrepareForActivation, behaviour vfunc 12 F-79450 -> state 4)
            if r.state == 5 {
                r.state = 4;
                r.rest = 0.0;
            }
            return;
        }
        r.wake = true;
        let back = matches!(r.path.as_ref(), Some(p) if p.reverse);
        r.physical = true;
        r.state = 4;
        r.path = None;
        r.then = None;
        r.passing = None;
        r.rest = 0.0;
        // (back from physics the seat starts from the car's pose, F-47485)
        r.seat = None;
        c.state.vel = c.state.forward() * if back { -r.v } else { r.v };
    }
}

#[cfg(test)]
mod verify_lanes_tests {
    //! Per-case comparison with the original's unchanged generated code, run offline by
    //! work/verify-people/lanes/stage1.py (docs/reference/people/verify_people_lanes.md).
    use super::*;

    fn fixture(name: &str) -> String {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/verify-people/lanes");
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    /// F-41621 lane_junction, F-86691 road_of, F-50269 junction_lane: the original on every lane of
    /// the city's navsys resource (10,031 lanes) and on all owner / parent flag combinations.
    #[test]
    #[ignore = "requires user's extracted data (MCLA_DATA_DIR) and the original's reference csv"]
    fn lane_owner_functions_match_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).unwrap();
        let nav = NavSys::parse(&res).unwrap();
        let be = |o: usize| u32::from_be_bytes(res.data[o..o + 4].try_into().unwrap());
        let jarr = (be(0x58) & 0x0FFF_FFFF) as usize;
        // the zones' offsets, scanned as NavSys::parse does
        let (mut zone_offs, mut o) = (Vec::new(), 0usize);
        while o + 0x30 <= res.virtual_size {
            match u32::from_le_bytes(res.data[o..o + 4].try_into().unwrap()) {
                0x0047_9f0c => o += 0x70,
                0x0047_a13c => {
                    zone_offs.push(o);
                    o += 0xb0;
                }
                0x0047_a0c4 => o += 0xc0,
                _ => o += 4,
            }
        }
        let spawner = Spawner::new(&nav, &root);
        let mut first_link = vec![usize::MAX; nav.lane_junction.len()];
        for (k, l) in nav.links.iter().enumerate().rev() {
            first_link[l.lane] = k;
        }
        let (mut real, mut with_junction, mut with_road) = (0, 0, 0);
        for line in fixture("stage1_original.csv").lines().filter(|l| l.starts_with("R,")) {
            let f: Vec<u32> = line.split(',').skip(1).map(|v| v.parse().unwrap()).collect();
            let (lane, junction, road, isj) = (f[0] as usize, f[2], f[3], f[4]);
            let ptr = |p: u32| if p == 0 { None } else { Some((p & 0x0FFF_FFFF) as usize) };
            let want_j = ptr(junction).map(|p| (p - jarr) / 0xc0);
            assert_eq!(lane_junction(&nav, lane), want_j, "lane {lane}");
            let want_road = ptr(road).map(|p| zone_offs.binary_search(&p).expect("owner is a zone") as u32);
            let k = first_link[lane];
            assert_eq!(spawner.road_of(&nav, k), want_road, "lane {lane}");
            assert_eq!(junction_lane(&nav.links[k]), isj == 1, "lane {lane}");
            real += 1;
            with_junction += want_j.is_some() as usize;
            with_road += want_road.is_some() as usize;
        }
        assert_eq!((real, with_junction, with_road), (10031, 3567, 6464));
        // synthetic: owner flags -1 (no owner) / 0..15, parent flags -1 / 0..15
        let mut base = nav.links[0].clone();
        let mut n = 0;
        for line in fixture("stage1_original.csv").lines().filter(|l| l.starts_with("S,")) {
            let f: Vec<i32> = line.split(',').skip(1).map(|v| v.parse().unwrap()).collect();
            base.owner_flags = f[0].max(0) as u32;
            base.parent_flags = if f[0] < 0 { 0 } else { f[1].max(0) as u32 };
            assert_eq!(junction_lane(&base), f[4] == 1, "owner {} parent {}", f[0], f[1]);
            n += 1;
        }
        assert_eq!(n, 17 * 17);
    }

    /// The original's arithmetic for the lane green test (F-16432 after the Online byte): the
    /// entry flattened, normalised by 1/sqrt with one Newton step (dp_ps, sqrt, div, then
    /// mul / sub / mul / add), arm . entry by two fused multiply-adds, against 0.9063078; x87-free,
    /// denormals flushed as the original's vector unit does.
    fn original_green(arm: Option<[f32; 3]>, timer: f32, e: [f32; 3]) -> bool {
        let ftz = |x: f32| if x.is_subnormal() { 0.0f32.copysign(x) } else { x };
        let (x, z) = (ftz(e[0]), ftz(e[2]));
        let s = ftz(ftz(x * x) + ftz(z * z));
        let sq = s.sqrt();
        let mut r0 = ftz(1.0 / sq);
        if r0 == 0.0 {
            r0 = 0.0;
        }
        let v9 = ftz(r0 * r0);
        let v8 = ftz(0.5 * r0);
        let v7 = -ftz(ftz(s * v9) - 1.0);
        let v6 = ftz(ftz(v7 * v8) + r0);
        let n = [ftz(x * v6), ftz(0.0 * v6), ftz(z * v6)];
        let a = arm.unwrap_or([0.0; 3]);
        let f12 = a[1] * n[1];
        let f7 = a[2].mul_add(n[2], f12);
        let f6 = a[0].mul_add(n[0], f7);
        // (the port's lane_dot is this arithmetic)
        assert_eq!(f6.to_bits(), crate::signals::lane_dot(Vec3::from(a), Vec3::from(e)).to_bits());
        timer < 6.5 && f6 > f32::from_bits(0x3f6803ca)
    }

    /// F-78185 (via lane_entry + Junction::lane_state), F-16432 (Junction::lane_state): the
    /// original's lane green tests on 17,031 junction set-ups (every one of the city's 10,031
    /// lanes, 4,000 with the first span 0.0003 degrees round the threshold, 3,000 entry vectors,
    /// degenerate ones among them) and F-35280 (Signals::release) on 300 states.
    #[test]
    #[ignore = "requires user's extracted data (MCLA_DATA_DIR) and the original's reference csv"]
    fn lane_green_and_release_match_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).unwrap();
        let nav = NavSys::parse(&res).unwrap();
        let mut first_link = vec![usize::MAX; nav.lane_junction.len()];
        for (k, l) in nav.links.iter().enumerate().rev() {
            first_link[l.lane] = k;
        }
        let orig: Vec<(u32, u32)> = fixture("stage2_original.csv").lines().filter(|l| l.starts_with("C,")).map(|l| {
            let f: Vec<u32> = l.split(',').skip(2).map(|v| v.parse().unwrap()).collect();
            (f[0], f[1])
        }).collect();
        let cases = fixture("cases2.txt");
        let (mut n, mut d178, mut d360, mut replica_bad, mut greens) = (0usize, Vec::new(), Vec::new(), 0, 0);
        for (line, o) in cases.lines().zip(&orig) {
            let w: Vec<u32> = line.split(' ').map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let fl = |i: usize| f32::from_bits(w[i]);
            let narms = w[1] as usize;
            let arms: Vec<Vec3> = (0..narms).map(|k| Vec3::new(fl(2 + 3 * k), fl(3 + 3 * k), fl(4 + 3 * k))).collect();
            let cur = w[17] as i32;
            let (timer, ent) = (fl(18), Vec3::new(fl(19), fl(20), fl(21)));
            let j = crate::signals::Junction { center: Vec3::ZERO, arms: arms.clone(), radius: 10.0, reach: 10.0, forced: -1, current: if (0..256).contains(&cur) { cur as u8 } else { 255 }, timer, demand: vec![0; narms] };
            let link = first_link[w[0] as usize];
            let (p0, p1) = lane_entry(&nav, link);
            let port178 = j.lane_state(p0 - p1) == 2;
            let port360 = j.lane_state(ent) == 2;
            let arm = (cur >= 0 && (cur as usize) < narms).then(|| [arms[cur as usize].x, arms[cur as usize].y, arms[cur as usize].z]);
            let le = p0 - p1;
            let rep178 = original_green(arm, timer, [le.x, le.y, le.z]);
            let rep360 = original_green(arm, timer, [ent.x, ent.y, ent.z]);
            replica_bad += (rep178 != (o.0 == 1)) as usize + (rep360 != (o.1 == 1)) as usize;
            if port178 != (o.0 == 1) {
                d178.push(n);
            }
            if port360 != (o.1 == 1) {
                d360.push(n);
            }
            greens += (o.0 == 1) as usize;
            n += 1;
        }
        println!("lane green: {n} cases, {greens} green (A178); port differs A178 {} cases, A360 {}; replica differs {replica_bad}; first {:?} / {:?}", d178.len(), d360.len(), d178.iter().take(6).collect::<Vec<_>>(), d360.iter().take(6).collect::<Vec<_>>());
        let grp = |v: &Vec<usize>| (v.iter().filter(|&&i| i < 10031).count(), v.iter().filter(|&&i| (10031..14031).contains(&i)).count(), v.iter().filter(|&&i| i >= 14031).count());
        println!("port differences by group (real lanes / threshold band / entry vectors): A178 {:?}, A360 {:?}", grp(&d178), grp(&d360));
        // the online byte: the original says green regardless; not ported
        assert!(fixture("stage2_original.csv").lines().any(|l| l == "O,0,1,1"));
        // release
        let rel = fixture("rel2.txt");
        let out: Vec<Vec<i64>> = fixture("stage2_original.csv").lines().filter(|l| l.starts_with("R,")).map(|l| l.split(',').skip(2).map(|v| v.parse().unwrap()).collect()).collect();
        for (line, o) in rel.lines().zip(&out) {
            let w: Vec<u32> = line.split(' ').map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let mut s = crate::signals::Signals::default();
            s.junctions.push(crate::signals::Junction { center: Vec3::ZERO, arms: vec![Vec3::X], radius: 10.0, reach: 10.0, forced: (w[4] as i32).clamp(-2, 5) as i8, current: w[2] as u8, timer: f32::from_bits(w[3]), demand: (0..w[0]).map(|k| w[6 + k as usize] + 1).collect() });
            s.release();
            let j = &s.junctions[0];
            // original: +176 = -1, +160 = 0, +164 = 2 + 6.5, ints zeroed to the count, floats zeroed, byte +180 zeroed
            assert_eq!((j.forced as i64, j.current as i64, j.timer.to_bits() as i64), (o[0], o[1], o[2]));
            assert!(j.demand.len() == o[3] as usize && j.demand.iter().all(|&d| d == 0) && o[5] == 1 && o[6] == 1 && o[7] == 0 && o[8] == 1);
        }
        assert_eq!(replica_bad, 0, "the replica of the original's arithmetic reproduces the original");
        assert_eq!((n, d178.len(), d360.len()), (17031, 0, 0), "the port's lane_state is the original's");
    }

    /// `snap_to_lane` is the original's F-59579 without a heading: on the city's lanes (as parsed and as `rail_nav`
    /// curves them) it answers as `railroute::snap`, which equals the original's code run offline (20,000 of 20,000,
    /// railroute's tests): 20,000 points round control points, 5 / 30 / 50 m, lane, phase, squared distance, point and
    /// direction bit for bit.
    #[test]
    fn lane_snap_is_the_originals() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let Ok(res) = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")) else { return };
        let nav = NavSys::parse(&res).unwrap();
        let net = crate::railroute::RailNet::parse(&res);
        let curved = nav.curved(2f32.to_radians());
        let mut seed = 0x2545_F491u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        let (mut n, mut hits) = (0, 0);
        for k in 0..20000 {
            let c = &nav.curves[(rnd() * nav.curves.len() as f32) as usize % nav.curves.len()];
            let q = c.pts[(rnd() * c.pts.len() as f32) as usize % c.pts.len()];
            let p = q + Vec3::new(rnd() * 80.0 - 40.0, rnd() * 10.0 - 5.0, rnd() * 80.0 - 40.0);
            let r = [5.0, 30.0, 50.0][k % 3];
            let want = crate::railroute::snap(&net, &nav, p, r).map(|h| (h.lane as usize, h.t.to_bits(), h.d2.to_bits(), h.point.to_array().map(f32::to_bits), h.dir.to_array().map(f32::to_bits)));
            for nv in [&nav, &curved] {
                let got = snap_to_lane(nv, p, r).0.map(|h| (h.lane, h.t.to_bits(), h.d2.to_bits(), h.point.to_array().map(f32::to_bits), h.dir.to_array().map(f32::to_bits)));
                assert_eq!(got, want, "case {k} at {p:?} r {r}");
            }
            n += 1;
            hits += want.is_some() as usize;
        }
        println!("lane snap: {n} cases equal, {hits} with a lane");
        assert!(hits > 5000);
    }

    /// F-24415 (the 4-chord call) and F-53741 (n chords): `curve_nearest` against the original's
    /// nearest-point search on all 10,031 lanes, 3 positions each (0.5 - 30 m round a control
    /// point), 4 or 8 chords, 10 refinements. Bit-for-bit counts per output, by lane degree.
    #[test]
    #[ignore = "requires user's extracted data (MCLA_DATA_DIR) and the original's reference csv"]
    fn curve_nearest_against_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).unwrap();
        let nav = NavSys::parse(&res).unwrap();
        let cases = fixture("cases5.txt");
        let orig = fixture("stage5_original.csv");
        let u = |s: &str| f32::from_bits(s.parse::<u32>().unwrap());
        // [cases, all three outputs equal, phase equal, d2 equal] per (call, degree)
        let mut stat: std::collections::BTreeMap<(u8, u8), [usize; 4]> = Default::default();
        let mut max_phase = [0.0f32; 2];
        let mut max_d2_rel = [0.0f32; 2];
        for (c, o) in cases.lines().zip(orig.lines().skip(1)) {
            let w: Vec<u32> = c.split(' ').map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let lane = w[0] as usize;
            let p = Vec3::new(f32::from_bits(w[1]), f32::from_bits(w[2]), f32::from_bits(w[3]));
            let v: Vec<&str> = o.split(',').collect();
            let curve = &nav.curves[lane];
            for (call, n, base) in [(0u8, 4usize, 1usize), (1, w[4] as usize, 6)] {
                let (t, d2, q) = curve_nearest(curve, p, n, 10);
                let want = (u(v[base]), u(v[base + 1]), Vec3::new(u(v[base + 2]), u(v[base + 3]), u(v[base + 4])));
                let e = stat.entry((call, curve.degree)).or_default();
                e[0] += 1;
                e[1] += (t.to_bits() == want.0.to_bits() && d2.to_bits() == want.1.to_bits() && q.to_array().map(f32::to_bits) == want.2.to_array().map(f32::to_bits)) as usize;
                e[2] += (t.to_bits() == want.0.to_bits()) as usize;
                e[3] += (d2.to_bits() == want.1.to_bits()) as usize;
                max_phase[call as usize] = max_phase[call as usize].max((t - want.0).abs());
                max_d2_rel[call as usize] = max_d2_rel[call as usize].max(((d2 - want.1).abs() / want.1.max(1e-3)).min(1e9));
            }
        }
        println!("nearest [cases, all equal, phase equal, d2 equal] by (call 0 = 4 chords / 1 = n chords, degree): {stat:?}");
        println!("max |phase diff| {max_phase:?}, max relative d2 diff {max_d2_rel:?}");
    }
}

#[cfg(test)]
mod tests {
    /// F-15621 (one lane's answer to a point) run offline on the placed navsys for all 10,031 lanes, three points each (a
    /// point of the lane moved up to 3 / 12 / 40 m across and 2 m up or down, radius 0.5 - 6.5 / 20.5 / 45.5 m; harness
    /// work/traffic-detail/snap): `lane_answer` gives the same validity and, when valid, the same squared distance, phase,
    /// point and direction bit for bit, and the answer's lane.
    #[test]
    fn lane_answer_matches_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let Ok(res) = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")) else { return };
        let nav = NavSys::parse(&res).unwrap();
        let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let f = |s: &str| f32::from_bits(h(s));
        let (mut n, mut valid, mut bad) = (0, 0, Vec::new());
        for line in include_str!("testdata/snap_lane_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let w: Vec<&str> = line.split_whitespace().filter(|x| *x != "|").collect();
            let lane = w[0].parse::<usize>().unwrap();
            let (p, r) = (glam::Vec3::new(f(w[1]), f(w[2]), f(w[3])), f(w[4]));
            let got = super::lane_answer(&nav, lane, p, r);
            let ok = match (&got, w[5] == "1") {
                (None, false) => true,
                (Some(a), true) => {
                    valid += 1;
                    let bits = [a.d2, a.t, a.point.x, a.point.y, a.point.z, a.dir.x, a.dir.y, a.dir.z].map(f32::to_bits);
                    bits.iter().zip(&w[6..14]).all(|(g, e)| *g == h(e)) && w[14].parse::<usize>().ok() == Some(a.lane)
                }
                _ => false,
            };
            if !ok {
                bad.push(format!("{line} -> {got:?}"));
            }
            n += 1;
        }
        assert_eq!(n, 30093);
        assert!(valid > 15000);
        assert!(bad.is_empty(), "{} of {n} differ, first: {:?}", bad.len(), &bad[..bad.len().min(4)]);
    }

    /// F-55442 (with F-32663) run offline for every lane of the city as the next lane, on the placed navsys (harness and
    /// recipe: docs reference/map/signal_rule.md "Which lane, checked"): the junction lane chosen (or none) and the lane's
    /// flags equal `light_lane`'s.
    #[test]
    fn light_lane_matches_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let Ok(res) = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")) else { return };
        let nav = NavSys::parse(&res).unwrap();
        let (mut n, mut bad) = (0, Vec::new());
        for line in include_str!("testdata/light_lane_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let w: Vec<&str> = line.split_whitespace().collect();
            let (lane, flags, want) = (w[0].parse::<usize>().unwrap(), u32::from_str_radix(w[1], 16).unwrap(), w[2].parse::<i64>().unwrap());
            let link = &nav.links[nav.links.partition_point(|k| k.lane < lane)];
            let got = super::light_lane(&nav, lane).map_or(-1, |l| l as i64);
            if got != want || link.flags != flags {
                bad.push(format!("lane {lane}: flags {:08x} / {flags:08x}, lane {got} / {want}", link.flags));
            }
            n += 1;
        }
        assert_eq!(n, 10031);
        assert!(bad.is_empty(), "{} of {n} differ, first: {:?}", bad.len(), &bad[..bad.len().min(8)]);
    }

    /// F-91754 on the original's code run offline (tools/frontend_planes_reference.py, 3,000 cases): the six planes'
    /// words and the two atans equal.
    #[test]
    fn frontend_planes_match_the_original() {
        let f = |w: &str| f32::from_bits(u32::from_str_radix(w, 16).unwrap());
        let mut n = 0;
        for l in include_str!("testdata/frontend_planes_ref.txt").lines() {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<f32> = c.split(' ').map(f).collect();
            let v = |i: usize| glam::Vec3::new(c[i], c[i + 1], c[i + 2]);
            let pl = super::frontend_planes_of([v(0), v(3), v(6), v(9)], [c[12], c[13]], [c[14], c[15], c[16], c[17]]);
            let got: Vec<String> = pl.iter().flat_map(|p| p.to_array()).map(|x| format!("{:08x}", x.to_bits())).collect();
            let want: Vec<&str> = o.split(' ').collect();
            assert_eq!(got, want[2..], "{l}");
            let at = |t: f32| format!("{:08x}", (mcla_formats::guest_math::guest_atan(t as f64) as f32).to_bits());
            assert_eq!((at(c[13]), at(c[12])), (want[0].to_string(), want[1].to_string()), "{l}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// F-95989 on the original's code run offline (tools/seen_reference.py, 6,000 cases): the answer and the point the
    /// occluder is asked about equal, the query's arguments (r 2, kinds 2 / 2) as the port's.
    #[test]
    fn seen_matches_the_original() {
        let f = |w: &str| f32::from_bits(u32::from_str_radix(w, 16).unwrap());
        let (mut n, mut asked) = (0, 0);
        for l in include_str!("testdata/seen_ref.txt").lines() {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<&str> = c.split(' ').collect();
            let o: Vec<&str> = o.split(' ').collect();
            let from = super::SeenFrom {
                inner: f(c[0]),
                outer: f(c[1]),
                wide: f(c[2]),
                fov_mph: [f(c[3]), f(c[4])],
                fov_deg: [f(c[5]), f(c[6])],
                focus: glam::Vec3::new(f(c[7]), f(c[8]), f(c[9])),
                view: glam::Vec3::new(f(c[10]), 0.0, f(c[11])),
                mph: f(c[12]),
            };
            let p = glam::Vec3::new(f(c[13]), f(c[14]), f(c[15]));
            let word = |i: usize| u32::from_str_radix(c[i], 16).unwrap();
            let mut got = None;
            let occ = (word(18) & 0xFF != 0).then_some(|q: glam::Vec3| {
                got = Some(q);
                word(19)
            });
            let r = from.sees(p, f(c[16]), word(17) & 0xFF != 0, occ);
            assert_eq!(r, o[0] == "1", "{l}");
            match got {
                None => assert_eq!(o[1], "-", "{l}"),
                Some(q) => {
                    let w = [q.x, q.y, q.z].map(|v| format!("{:08x}", v.to_bits()));
                    assert_eq!(&o[1..7], &[w[0].as_str(), w[1].as_str(), w[2].as_str(), "2", "2", "40000000"], "{l}");
                    asked += 1;
                }
            }
            n += 1;
        }
        assert_eq!((n, asked), (6000, 1654));
    }

    /// The footprint probe's segment against the running original: 162 probes of riders in state 6 recorded at F-57005
    /// (work/traffic-detail/probe, the recorder's Q lines) with the instance at the car update's entry (U), which the
    /// recording shows is the matrix the probe reads (equal in all 162; the cars moved up to 0.27 m a frame).
    #[test]
    fn probe_segment_matches_the_recording() {
        let txt = include_str!("testdata/probe_seg_rec.txt");
        let mut n = 0;
        for l in txt.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let (inp, out) = l.split_once(" | ").unwrap();
            let v: Vec<f32> = inp.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let w: Vec<f32> = out.split_whitespace().map(|x| x.parse().unwrap()).collect();
            assert!(v.len() == 15 && w.len() == 6);
            let r = |k: usize| Vec3::new(v[3 * k], v[3 * k + 1], v[3 * k + 2]);
            let (a, b) = super::probe_segment(&[r(0), r(1), r(2), r(3)], v[12], v[13], v[14] != 0.0);
            let got = [a.x, a.y, a.z, b.x, b.y, b.z];
            for k in 0..6 {
                assert_eq!(got[k].to_bits(), w[k].to_bits(), "case {n} word {k}: {got:?} vs {w:?}");
            }
            n += 1;
        }
        assert_eq!(n, 162);
    }

    use super::*;

    #[test]
    fn front_end_frustum_and_lane_entries() {
        // shot 5's values (AmbientMgr_SetFrontEnd(1, 0, 0, 0.1, 226, 4)), a camera looking down -z,
        // tan up 0.3, across 0.3 x 16 / 9
        let fe = FrontEnd { eye: Vec3::new(10.0, 2.0, 5.0), fwd: Vec3::NEG_Z, tan: [0.3, 0.3 * 16.0 / 9.0], amb: [0.0, 0.0, 0.1, 226.0], index: 4 };
        let pl = frontend_planes(&fe);
        let inside = |p: Vec3| !pl.iter().any(|q| plane_out(*q, p));
        assert!(inside(Vec3::new(10.0, 2.0, -100.0)));
        // near 0.1, far 226 along the view
        assert!(!inside(Vec3::new(10.0, 2.0, 5.0 - 0.05)));
        assert!(inside(Vec3::new(10.0, 2.0, 5.0 - 225.9)));
        assert!(!inside(Vec3::new(10.0, 2.0, 5.0 - 226.1)));
        // the side planes at atan(0.5333) = 28.07 degrees across, atan(0.3) = 16.70 degrees up
        let at = |deg_x: f32, deg_y: f32| Vec3::new(10.0 + 100.0 * deg_x.to_radians().tan(), 2.0 + 100.0 * deg_y.to_radians().tan(), -95.0);
        assert!(inside(at(28.0, 0.0)) && !inside(at(28.2, 0.0)) && inside(at(-28.0, 0.0)) && !inside(at(-28.2, 0.0)));
        assert!(inside(at(0.0, 16.6)) && !inside(at(0.0, 16.8)) && !inside(at(0.0, -16.8)));
        // S+8312 widens the sides: 0.06 rad more (31.51 degrees)
        let wide = frontend_planes(&FrontEnd { amb: [0.06, 0.0, 0.1, 226.0], ..fe });
        assert!(!wide.iter().any(|q| plane_out(*q, at(31.45, 0.0))) && wide.iter().any(|q| plane_out(*q, at(31.6, 0.0))));
        // a straight lane across the view from the left (x -1000 .. 1000 at z -95): one entry,
        // across the left plane (index 1), at the middle of the 100 samples' step
        let c = path_curve(&[Vec3::new(-990.0, 2.0, -95.0), Vec3::new(1010.0, 2.0, -95.0)]);
        let e = lane_entries(&c, pl[1], 64);
        assert_eq!(e.len(), 1);
        let x_edge = 10.0 - 100.0 * (0.3f32 * 16.0 / 9.0);
        assert!((e[0].0.x - x_edge).abs() < 2000.0 / 99.0, "{:?} {}", e[0], x_edge);
        assert!(lane_entries(&c, pl[2], 64).is_empty());
        // the region's sphere test: a 40 x 10 m road along +x, 30 m off a circle of 20 m
        let r = RegionRect { index: 0, rect_c: [0.0, 0.0], width: 10.0, len: 40.0, y_range: [0.0, 1.0], mid: Vec3::ZERO, dir: Vec3::X, side: Vec3::Z, no_box: false };
        assert!(region_meets_circle(&r, Vec3::new(0.0, 0.0, 24.9), 20.0));
        assert!(!region_meets_circle(&r, Vec3::new(0.0, 0.0, 25.1), 20.0));
        assert!(region_meets_circle(&r, Vec3::new(39.9, 0.0, 0.0), 20.0));
        assert!(!region_meets_circle(&r, Vec3::new(40.1, 0.0, 0.0), 20.0));
    }

    #[test]
    fn the_drawn_wheels_lock_two_updates_late() {
        // (F-37906: the car ahead in charge, mean asked deceleration >= fMaxDecel, over 5 mph)
        let mut l = Lock::default();
        let dt = 1.0 / 30.0;
        let mut x = 0.0;
        let mut step = |l: &mut Lock, a: f32, slot: Option<usize>| {
            x += 0.2; // 6 m/s
            l.update(Vec3::new(x, 0.0, 0.0), a, slot, 22.5, dt);
        };
        step(&mut l, 0.0, Some(0));
        assert!(!l.now());
        // one -22.5 with 0 before: mean -11.25, not yet; a free-driving entry never
        step(&mut l, -22.5, Some(0));
        assert!(!l.now());
        for _ in 0..4 {
            step(&mut l, -22.5, None);
        }
        assert!(!l.now());
        // (update A works the bit out; the render of the update after the next - C - sees it:
        // read between B and C, `drawn` is what C's render update reads)
        step(&mut l, -22.5, Some(0));
        assert!(l.now() && !l.drawn());
        step(&mut l, -22.5, Some(0));
        assert!(l.drawn());
    }

    #[test]
    fn parked_front_wheels_and_spin() {
        // (car +3584: N(0, pi / 8), clamped to +-0.706858)
        assert_eq!(parked_steer(1.0, 0.0), 0.0);
        assert_eq!(parked_steer(1e-6, 0.0), 0.706858);
        assert_eq!(parked_steer(1e-6, 0.5), -0.706858);
        assert!((wrap_spin(7.0) - (7.0 - std::f32::consts::TAU)).abs() < 1e-6);
        assert!((wrap_spin(-0.5) - (std::f32::consts::TAU - 0.5)).abs() < 1e-6);
    }

    #[test]
    fn tilt_of_the_drawn_body() {
        // speeding up at 3 m/s2 (0.1 m/s a 30 Hz frame): nose up 0.4 degrees; 60 Hz updates
        let mut t = Tilt::default();
        let (fwd, right) = (Vec3::NEG_Z, Vec3::X);
        let (mut p, mut v) = (Vec3::ZERO, 5.0f32);
        for _ in 0..120 {
            v += 3.0 / 60.0;
            p += fwd * v / 60.0;
            t.update(p, fwd, right, 0.0, 1.0 / 60.0);
        }
        let (pitch, roll) = t.degrees();
        assert!((pitch - 0.4).abs() < 0.02 && roll.abs() < 0.01, "{pitch} {roll}");
        // a left turn, radius 12 m at 6.7 m/s (3.7 m/s2): leaning out - the right side down, -3
        let mut t = Tilt::default();
        for k in 0..120 {
            let a = k as f32 * 6.7 / 12.0 / 60.0;
            let fwd = Vec3::new(-a.sin(), 0.0, -a.cos());
            let right = Vec3::new(a.cos(), 0.0, -a.sin());
            let p = Vec3::new(-12.0 + 12.0 * a.cos(), 0.0, -12.0 * a.sin());
            t.update(p, fwd, right, 100.0, 1.0 / 60.0);
        }
        let (pitch, roll) = t.degrees();
        assert!((roll + 3.0).abs() < 0.01 && pitch.abs() < 0.05, "{pitch} {roll}");
        // beyond 30 m: not turned
        t.update(Vec3::new(100.0, 0.0, 0.0), Vec3::NEG_Z, Vec3::X, 1000.0, 1.0 / 30.0);
        assert!(t.quat().is_none());
    }

    #[test]
    fn needed_acceleration_like_the_original() {
        let ty = AmbType::default();
        let (up, down) = (ty.max_accel, ty.max_decel);
        // 10 m/s to a stop over 20 m: -2.5 m/s^2
        assert!((needed(10.0, 0.0, 25.0, 5.0, up, down) + 2.5).abs() < 1e-5);
        // no room left: full braking, whatever the speeds
        assert_eq!(needed(10.0, 0.0, 5.0, 5.0, up, down), -down);
        assert_eq!(needed(2.0, 8.0, 3.0, 5.0, up, down), -down);
        // standing with the target standing: go while there is room (the creep to the line)
        assert_eq!(needed(0.0, 0.0, 6.0, 5.0, up, down), up);
        assert_eq!(needed(0.0, 0.0, 5.0, 5.0, up, down), -down);
    }

    // ---- verify-people / traffic: the port against the original's unchanged generated code
    // (work/verify-people/traffic/run_traffic.py; evidence docs/reference/people/verify_people_traffic.md).
    // Run with: cargo test -p mcla --release verify_traffic -- --ignored --nocapture

    /// 0.14912909269 (the original's constant in turn radius; the port's literal 0.149129 is one ulp lower)
    const VTURN_K: u32 = 0x3E18_B54C;

    const VFIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../work/verify-people/traffic/fix/");

    /// The rows of a fixture file as numbers; `skip` leading tokens (the mode word) are dropped.
    fn vrows(name: &str, skip: usize) -> Vec<Vec<u32>> {
        let text = std::fs::read_to_string(format!("{VFIX}{name}")).unwrap_or_else(|_| panic!("fixture {name} missing: run run_traffic.py"));
        text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.split_whitespace().skip(skip).map(|t| t.parse::<u32>().unwrap()).collect()).collect()
    }

    /// Equal as floats (+0 and -0 equal), else bit patterns equal.
    fn veq(a: f32, b: f32) -> bool {
        a == b || a.to_bits() == b.to_bits()
    }

    fn vrider(ty: AmbType) -> Rider {
        Rider { ty, link: 0, s: 0.0, next: None, v: 0.0, off: 0.0, off_timer: 0.0, bonus: 0.0, gap: 3.0, wait: 0.0, waited: 0.0, on: [false; 3], stop_id: None, obst_id: None, state: 1, physical: false, path: None, then: None, stand: 0.0, pass_stand: 0.0, stuck: (0.0, 0.0, 0), passing: None, kept: None, rest: 0.0, partner: None, seat: None, tilt: Tilt::default(), in_level: false, active: false, wake: false, rng: 7, id: 7, mwc: mwc_seed(7), ds: 0.0, ds_lane: usize::MAX, t: 0.0, ahead: (0.0, 0.0), a: 0.0, slot: None, lock: Lock::default(), born: 0.0, life: 8.0, route: None, race_lane: None, phys_pair: false, quiet: false, path_referenced: false, obst_v: None, steer: 0.0, rail_seq: 0, rail_key: None, rail_fresh: true, ghost: None, mark: None }
    }

    /// The brake lamp's flag (F-56024 with F-86955, F-17857 and F-59415) against the original's code run offline on 4,000
    /// states the game can reach (tools/brake_lamp_reference.py -> testdata/brake_lamp_ref.txt): no behaviour (a trailer:
    /// state 0), the race path's bParked byte, the speed, the entry in charge, the physics instance's id and flags, the car
    /// ahead and its velocity.
    #[test]
    fn brake_lamp_matches_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let mut n = 0;
        for line in include_str!("testdata/brake_lamp_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let mut r = vrider(AmbType::default());
            r.state = if w[0] == "1" { 1 } else { 0 };
            r.quiet = w[1] == "1" && w[2] != "0";
            r.v = h(w[3]);
            let slot: i32 = w[5].parse().unwrap();
            r.slot = (slot >= 0).then_some(slot as usize);
            let (id, flags): (u32, u32) = (w[6].parse().unwrap(), w[7].parse().unwrap());
            r.active = id != 0xFFFF && flags & 3 == 0;
            r.obst_v = (w[8] == "1").then(|| obst_speed(Vec3::new(h(w[9]), h(w[10]), h(w[11]))));
            assert_eq!(brake_lamp(&r), o == "1", "{line}");
            n += 1;
        }
        assert_eq!(n, 4000);
    }

    #[test]
    fn drawn_steer_matches_the_original() {
        // (tools/drawn_steer_reference.py: F-37033 run whole with the rail's slots 6 / 10 stubbed; one frame a line, a
        // sequence's ring kept from frame to frame. A parked car - no behaviour, type byte +112 0 - goes through the
        // port's `parked_drawn_steer` with the draw object's +32 as its update count)
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let mut ring = SteerRing::default();
        let (mut n, mut sampled, mut parked) = (0, 0, 0);
        for line in include_str!("testdata/drawn_steer_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let o: Vec<&str> = o.split_whitespace().collect();
            let flag = |k: usize| w[k] == "1";
            if flag(0) {
                ring = SteerRing::default();
            }
            let (pos, focus) = (Vec3::new(h(w[9]), h(w[10]), h(w[11])), (h(w[7]), h(w[8])));
            let (dx, dz) = (focus.0 - pos.x, focus.1 - pos.z);
            let near = dz.mul_add(dz, dx * dx) < 2025.0;
            let looks = flag(1) && flag(2) && near;
            let mut k = 0;
            let mut x = 0.0;
            if looks {
                sampled += 1;
                assert_eq!([o[0], o[1], o[2]], [w[15], &format!("{:08x}", (h(w[6]).abs() * 0.4).to_bits()), w[16]], "{line}");
                assert_eq!(o[3], format!("{:08x}", steer_clamp(h(w[17])).to_bits()), "{line}");
                x = steer_of(Vec3::new(h(w[12]), h(w[13]), h(w[14])), Vec3::new(h(w[18]), h(w[19]), h(w[20])));
                k = 4;
            } else {
                assert_eq!((o[0], o[1]), ("-", "-"), "{line}");
                k += 2;
            }
            if !flag(1) && w[3] == "0" {
                parked_drawn_steer(&mut ring, h(w[5]), w[4].parse::<i32>().unwrap().clamp(0, 3) as u8, focus.0 - pos.x, focus.1 - pos.z);
                parked += 1;
            } else {
                ring.push(x);
            }
            let got = format!("{} {} {:08x} {} {:08x}", ring.n, ring.next, ring.sum.to_bits(), ring.v.map(|v| format!("{:08x}", v.to_bits())).join(" "), ring.mean().to_bits());
            assert_eq!(got, o[k..].join(" "), "{line}");
            n += 1;
        }
        assert_eq!((n, sampled, parked), (2381, 1160, 290));
    }

    /// F-37359 (does the render update run) and F-14577 (the draw object's update count) against the original's code
    /// run offline (tools/parked_render_reference.py -> testdata/parked_render_ref.txt): the gate for every case with an
    /// instance and a count the game can hold (0 - 3), the count after the tick for those.
    #[test]
    fn parked_render_gate_matches_the_original() {
        let (mut n, mut runs) = (0, 0);
        for line in include_str!("testdata/parked_render_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let o: Vec<&str> = o.split_whitespace().collect();
            if w[1] == 0 {
                assert_eq!(o[0], "0", "{line}");
                continue;
            }
            if w[3] > 3 {
                continue;
            }
            let frames = w[3] as u8;
            let run = parked_render_runs(w[0] != 0, w[2] & 0x10 != 0, frames, w[4] != 0);
            assert_eq!(o[0], if run { "1" } else { "0" }, "{line}");
            assert_eq!(o[1], format!("{:x}", (frames + 1).min(3)), "{line}");
            n += 1;
            runs += run as usize;
        }
        assert_eq!((n, runs), (1489, 1397));
    }

    /// F-85925 (the junction a lane leads into) against the original's code run offline on every city lane with the
    /// real navsys (work/traffic-detail/fix20/stage6.py -> testdata/lane_into_junction_ref.txt).
    #[test]
    fn lane_into_junction_matches_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let Ok(res) = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")) else { return };
        let nav = NavSys::parse(&res).unwrap();
        let want: hashbrown::HashMap<usize, usize> = include_str!("testdata/lane_into_junction_ref.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| {
                let mut f = l.split(' ').map(|x| x.parse::<usize>().unwrap());
                (f.next().unwrap(), f.next().unwrap())
            })
            .collect();
        assert_eq!((nav.curves.len(), want.len()), (10031, 2837));
        for lane in 0..nav.curves.len() {
            assert_eq!(lane_into_junction(&nav, lane), want.get(&lane).copied(), "lane {lane}");
        }
    }

    /// The race traffic clearing (F-20685 with F-95725 / F-61293 / F-12622, F-91078 and F-17030) against the original's
    /// code run whole offline on the real city navsys (tools/race_clearing_reference.py -> testdata/race_clearing_ref.txt):
    /// 4,000 runs - the switch, the bytes +2107 / +2108, the race byte, the railnet's +69, one or two junction records
    /// (directions along the junction's lanes and arms, at the 0.643 cut-off to the float, random, zero and NaN), 0 - 4
    /// roads already cleared. Every slot 19 write (lane, value, in order) and the set after the run must be equal.
    #[test]
    fn race_clearing_matches_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let Ok(res) = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")) else { return };
        let nav = NavSys::parse(&res).unwrap();
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let (mut n, mut writes, mut set, mut edge) = (0, 0, 0, 0);
        for l in include_str!("testdata/race_clearing_ref.txt").lines().filter(|l| !l.is_empty()) {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<u32> = c.split(' ').map(h).collect();
            let rec = |k: usize| (c[k + 3] != u32::MAX).then(|| (c[k + 3] as usize, [f32::from_bits(c[k]), f32::from_bits(c[k + 1]), f32::from_bits(c[k + 2])]));
            let recs = [rec(5), rec(9)];
            let prev: Vec<usize> = c[14..14 + c[13] as usize].iter().map(|&r| r as usize).collect();
            let mut rc = RaceClearing { roads: prev.clone(), off: Default::default() };
            let got = if c[0] != 0 {
                Vec::new()
            } else {
                let on = (c[1] & 0xFF) != 0 || ((c[2] & 0xFF) != 0 && (c[3] & 0xFF) != 0);
                rc.step(&nav, on, c[4] & 0xFF != 0, &recs)
            };
            let mut want = Vec::new();
            let (w, s) = o.split_once("N ").unwrap();
            let t: Vec<&str> = w.split_whitespace().collect();
            for k in t.chunks(3) {
                assert_eq!(k[0], "T");
                want.push((k[1].parse::<usize>().unwrap(), k[2] == "1"));
            }
            let s: Vec<usize> = s.split_whitespace().map(|x| x.parse().unwrap()).collect();
            assert_eq!(got, want, "case {n}: {l}");
            assert_eq!((s[0], &s[1..]), (rc.roads.len(), &rc.roads[..]), "case {n}: {l}");
            writes += want.len();
            set += want.iter().any(|x| x.1) as usize;
            for (j, d) in recs.iter().flatten() {
                for &(_, arm) in &nav.records.junction_arms[*j] {
                    if let Some(a) = usize::try_from(arm).ok().and_then(|k| nav.records.arm_dirs[*j].get(k)) {
                        edge += (d[0].mul_add(a[0], d[2].mul_add(a[2], a[1] * d[1])).abs() == CLEAR_DOT) as usize;
                    }
                }
            }
            n += 1;
        }
        assert_eq!(n, 4000);
        eprintln!("race clearing: {n} runs, {writes} slot 19 writes, {set} runs setting lanes, {edge} arms at |dot| = 0.643 exactly");
    }

    /// The road at a point (F-86195 with the quadtree query F-44023 and the roads' box test F-35347) and the density
    /// index (F-85897) against the original's code run whole offline on the real city navsys
    /// (tools/road_density_reference.py -> testdata/road_density_ref.txt): 6,000 points - in and about road boxes, on
    /// their edges and on the quadtree's splits and box edges to the float, the whole city and beyond, NaN, the two
    /// points the scripts give AmbientMgr_SetRoadDensity - with F-86195's road and every hit F-44023 counted (road and
    /// squared distance bits, in order); 600 index cases over the global, script, cutscene, front-end and own values;
    /// 1,200 runs of the setters AmbientMgr_SetRoadDensity (F-84942) and the race event's fire (F-36201): the road
    /// written and the index.
    #[test]
    fn road_density_matches_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let Ok(res) = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")) else { return };
        let nav = NavSys::parse(&res).unwrap();
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let (mut n, mut on, mut multi, mut d, mut s) = (0, 0, 0, 0, 0);
        for l in include_str!("testdata/road_density_ref.txt").lines().filter(|l| !l.is_empty()) {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<&str> = c.split(' ').collect();
            if c[0] == "S" {
                // the setters: AmbientMgr_SetRoadDensity (kind 0) and the race event's fire (kind 1, skipped while the
                // word 0x828CCE34 is set), both only with the ambient manager there: the road at the point gets the index
                let w: Vec<u32> = c[1..].iter().map(|t| h(t)).collect();
                let p = Vec3::new(f32::from_bits(w[1]), f32::from_bits(w[2]), f32::from_bits(w[3]));
                let on = w[5] != 0 && (w[0] == 0 || w[6] == 0);
                let got = if on { nav.road_at(p).map(|r| format!("{r} {:08x}", w[4])) } else { None };
                assert_eq!(got.unwrap_or_else(|| "-".into()), o, "{l}");
                s += 1;
                continue;
            }
            if c[0] == "D" {
                let w: Vec<u32> = c[1..].iter().map(|t| h(t)).collect();
                let got = density_index(w[0] as i32, w[1] as i32, w[2] & 0xFF != 0, (w[3] & 0xFF != 0).then_some(w[4] as i32), w[5] as i32);
                assert_eq!(got as u32, h(o), "{l}");
                d += 1;
                continue;
            }
            let p = Vec3::new(f32::from_bits(h(c[1])), f32::from_bits(h(c[2])), f32::from_bits(h(c[3])));
            let (r, hits) = o.split_once(" |").unwrap();
            let want: Vec<(usize, u32)> = hits.split_whitespace().map(|t| {
                let (a, b) = t.split_once(':').unwrap();
                (a.parse().unwrap(), h(b))
            }).collect();
            let got: Vec<(usize, u32)> = nav.road_hits(p, 8).into_iter().map(|(r, d2)| (r, d2.to_bits())).collect();
            assert_eq!(got, want, "hits: {l}");
            let r: i64 = r.parse().unwrap();
            assert_eq!(nav.road_at(p).map_or(-1, |x| x as i64), r, "road: {l}");
            on += (r >= 0) as usize;
            multi += (want.len() > 1) as usize;
            n += 1;
        }
        assert_eq!((n, d, s), (6000, 600, 1200));
        eprintln!("road density: {n} points, {on} on a road, {multi} with more than one hit; {d} index cases; {s} setter runs");
    }

    /// The removal message (F-99126 with the kind byte F-93575 / F-30914) and the enable natives (F-26788, F-74501,
    /// F-54264) against the original's code run whole offline (tools/ambient_remove_reference.py ->
    /// testdata/ambient_remove_ref.txt): 3,000 runs of 0 - 12 objects in the manager's six chains (spot word, type byte
    /// +108, vfunc 24 / 28 answers) under the used and random messages - the objects given a removal request (reason
    /// 11), in order -, and 1,500 native calls (argument, manager there or not, threaded, the stack byte): the byte
    /// stored and the message sent (straight or queued).
    #[test]
    fn ambient_remove_matches_the_original() {
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let (mut k, mut e, mut removed) = (0, 0, 0);
        for l in include_str!("testdata/ambient_remove_ref.txt").lines().filter(|l| !l.is_empty()) {
            let (c, o) = l.split_once(" -> ").unwrap();
            let w: Vec<u32> = c.split_whitespace().skip(1).map(h).collect();
            if l.starts_with('K') {
                // (the count is written in decimal)
                let (msg, n) = (w[0] as u8, c.split(' ').nth(2).unwrap().parse::<usize>().unwrap());
                let objs: Vec<&[u32]> = w[2..2 + 5 * n].chunks(5).collect();
                // the chains in the order F-99126 walks them: buckets 0..2 at +8, then at +32
                let mut got = Vec::new();
                for ch in 0..6 {
                    for (i, ob) in objs.iter().enumerate().filter(|(_, ob)| ob[0] == ch) {
                        if removal_hits(ambient_kind(ob[2] as u8, ob[1], ob[3], ob[4]), msg) {
                            got.push(format!("{i}/11"));
                        }
                    }
                }
                let want: Vec<String> = o.split_whitespace().map(|s| s.to_string()).collect();
                assert_eq!(got, want, "{l}");
                removed += want.len();
                k += 1;
            } else {
                let (native, arg, mgr, threaded, stack) = (w[0] as usize, w[1], w[2] != 0, w[3] & 0xFF != 0, w[4] as u8);
                let t: Vec<&str> = o.split_whitespace().collect();
                let stored = if mgr { format!("{:02x}", arg & 0xFF) } else { "--".into() };
                assert_eq!(t[0], stored, "{l}");
                let sent = if mgr { enable_message(native, arg) } else { None };
                match sent {
                    None => assert_eq!(t[1], "-", "{l}"),
                    Some(m) => {
                        assert_eq!(t[1], if threaded { "Q" } else { "M" }, "{l}");
                        assert_eq!(h(t[2]) as u8, m | (stack & 0x0F), "{l}");
                    }
                }
                e += 1;
            }
        }
        assert!(k >= 2990 && e >= 1490, "{k} {e}");
        eprintln!("ambient removal: {k} runs, {removed} objects removed; {e} native calls");
    }

    /// The race holds and the wanted speeds (F-70324 with F-73512, F-15116, F-50269 and F-85925 by the case) against the
    /// original's code run offline on 6,000 cases (tools/race_hold_reference.py -> testdata/race_hold_ref.txt): the gate
    /// (the event byte, a remote car, a proxy, the route's lane, the path), the two lanes' owners, parents and flags, the
    /// speeds and offset, the phase about 0.3, the headings' dot about 0.7071, the junctions the lanes lead into against
    /// the player's, the car's speed against the next wanted speed. The returned speed and the slower entry (+4 the
    /// distance, +8 the speed, byte +16 bit 0x80) must be equal bit for bit.
    #[test]
    fn race_holds_match_the_original() {
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let f = |t: &str| f32::from_bits(h(t));
        let mut n = 0;
        for l in include_str!("testdata/race_hold_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let (c, o) = l.split_once(" -> ").unwrap();
            let c: Vec<&str> = c.split(' ').collect();
            let o: Vec<&str> = o.split(' ').collect();
            let bits = |k: usize| LaneBits { owner: if h(c[k]) != 0 { h(c[k + 1]) } else { 0 }, parent: if h(c[k + 2]) != 0 { h(c[k + 3]) } else { 0 }, flags: h(c[k + 4]) };
            let cur = bits(5);
            let next = (h(c[11]) != 0).then(|| bits(12));
            let gate = h(c[0]) != 0 && h(c[1]) == 0 && h(c[2]) & 0x20 == 0 && h(c[3]) == 0 && h(c[4]) == 0;
            let j = |k: usize| match h(c[k]) { 0 => None, v => Some(v as usize) };
            let holds = if gate {
                race_holds(cur, next, f(c[19]), (f(c[20]), f(c[21])), (f(c[22]), f(c[23])), (j(24), j(25)), (h(c[26]) != 0).then_some(1))
            } else {
                (false, false)
            };
            let junction = |b: LaneBits| b.owner & 0x30 != 0 || b.flags & 0x1000 != 0;
            let (want, next_want) = wanted((f(c[10]), junction(cur)), next.map(|_| f(c[17])), f(c[18]), holds);
            let s = f(c[27]);
            let v = (s * s).sqrt();
            let entry = next_want <= v;
            let rec = if entry { (if holds.0 { 0.0 } else { f(c[28]) }, next_want, 0x80) } else { (0.0, 0.0, 0) };
            assert_eq!(
                (want.to_bits(), rec.0.to_bits(), rec.1.to_bits(), h(o[3]), rec.2),
                (h(o[0]), h(o[1]), h(o[2]), 0, h(o[4])),
                "{l}"
            );
            n += 1;
        }
        assert_eq!(n, 6000);
    }

    /// The turn signals (F-41231 bit 0x20, F-81289 bit 0x10, with the lane end F-93886 and the owner test F-29760)
    /// against the original's code run offline on 5,000 cases (tools/turn_signal_reference.py ->
    /// testdata/turn_signal_ref.txt): the blink at and around its offset + 0.375, the current lane's owner (none, its
    /// flags, its +0x10 word), the phase about 0.5, the next lane (none, its owner and flags), the car's place about
    /// 100 m from the lane's end.
    #[test]
    fn turn_signals_match_the_original() {
        use mcla_formats::rage::navsys::Link;
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let x = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let link = |lane: usize, owner_flags: u32, owner_w10: u32| Link { a: Vec3::ZERO, b: Vec3::X, len: 1.0, flags: 0, zone: None, owner_flags, owner_w10, owner_speed: 0.0, parent_flags: 0, parent_zone: None, lane, next: Vec::new(), prev: Vec::new(), side: Vec::new(), opp: Vec::new(), back: Vec::new(), hop: Vec::new(), hop_a: Vec::new(), node_a: 0, node_b: 0, ta: Vec3::ZERO, tb: Vec3::ZERO, t: [0.0; 2] };
        let mut n = 0;
        for line in include_str!("testdata/turn_signal_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let end = Vec3::new(h(w[12]), h(w[13]), h(w[14]));
            let mut nav = NavSys::default();
            // (no owner: flags 0, as the port's navsys keeps a lane without one)
            let (f1, w10) = if w[2] == "1" { (x(w[3]), w[4].parse().unwrap()) } else { (0, 0) };
            nav.links.push(link(0, f1, w10));
            nav.links.push(link(1, if w[7] == "1" { x(w[8]) } else { 0 }, 0));
            nav.curves.push(LaneCurve { pts: vec![Vec3::ZERO, Vec3::ONE, Vec3::X, end], ..Default::default() });
            let mut r = vrider(AmbType::default());
            r.t = h(w[5]);
            r.next = (w[6] == "1").then_some(1);
            let (a, b) = signals(&nav, &r, Vec3::new(h(w[9]), h(w[10]), h(w[11])), h(w[0]), h(w[1]));
            assert_eq!(format!("{} {}", a as u8, b as u8), o, "{line}");
            n += 1;
        }
        assert_eq!(n, 5000);
    }

    /// The needed acceleration (F-20809) on 800 cases.
    #[test]
    #[ignore]
    fn verify_traffic_needed() {
        let (inp, rf) = (vrows("needed_in.txt", 1), vrows("needed_ref.txt", 0));
        assert_eq!(inp.len(), rf.len());
        let mut bad = 0;
        for (c, o) in inp.iter().zip(&rf) {
            let f = |i: usize| f32::from_bits(c[i]);
            // the original reads the speed off the velocity (x, z); the cases keep z = 0 or an
            // exact Pythagorean pair, where sqrt(x^2 + z^2) is exact in f32 and in the original
            let v = (f(0) * f(0) + f(1) * f(1)).sqrt();
            let got = needed(v, f(2), f(3), f(4), f(5), f(6));
            if !veq(got, f32::from_bits(o[0])) {
                bad += 1;
                if bad < 6 {
                    println!("needed differs: v {v} vt {} dist {} margin {} -> port {got} original {}", f(2), f(3), f(4), f32::from_bits(o[0]));
                }
            }
        }
        println!("needed: {} cases, {} differ", inp.len(), bad);
        assert_eq!(bad, 0);
    }

    /// The turning radius (non-pivot branch) on 709 cases.
    #[test]
    #[ignore]
    fn verify_traffic_turn_radius() {
        let (inp, rf) = (vrows("turn_in.txt", 1), vrows("turn_ref.txt", 0));
        assert_eq!(inp.len(), rf.len());
        let (mut bad, mut maxulp) = (0, 0u32);
        for (c, o) in inp.iter().zip(&rf) {
            let ty = AmbType { min_turn: f32::from_bits(c[1]), ..AmbType::default() };
            let got = turn_radius(&ty, f32::from_bits(c[0]), false);
            if !veq(got, f32::from_bits(o[0])) {
                bad += 1;
                maxulp = maxulp.max(got.to_bits().abs_diff(o[0]));
                if bad < 6 {
                    println!("turn differs: v {} base {} -> port {got} original {}", f32::from_bits(c[0]), ty.min_turn, f32::from_bits(o[0]));
                }
            }
        }
        println!("turn_radius: {} cases, {} differ, max {} ulp", inp.len(), bad, maxulp);
        // (diagnostic only: the cause of the differences)
        let (mut bad_c, mut bad_cf) = (0, 0);
        for (c, o) in inp.iter().zip(&rf) {
            let (v, base) = (f32::from_bits(c[0]), f32::from_bits(c[1]));
            let k = f32::from_bits(VTURN_K);
            let x = ((v - 4.4704) * k).clamp(0.0, 1.0);
            if !veq(base * (1.0 + 6.0 * x), f32::from_bits(o[0])) { bad_c += 1; }
            if !veq(6.0f32.mul_add(x, 1.0) * base, f32::from_bits(o[0])) { bad_cf += 1; }
        }
        println!("  with the image's constant 0.14912909 instead of the literal: {} differ; with it and a fused 1 + 6x: {} differ", bad_c, bad_cf);
        // known difference (the literal constant and the unfused 1 + 6x), see the evidence file: 143 of 709
        assert_eq!((bad, bad_c, bad_cf), (143, 2, 0));
    }

    /// The speed integrator against the original's first block (offset draw excluded) on 2000 cases.
    #[test]
    #[ignore]
    fn verify_traffic_integrate() {
        let (inp, rf) = (vrows("integ_in.txt", 1), vrows("integ_ref.txt", 0));
        assert_eq!(inp.len(), rf.len());
        let (mut bad, mut maxulp, mut cap_nonpos, mut fma_only) = (0, 0u32, 0, 0);
        for (c, o) in inp.iter().zip(&rf) {
            let (a, v, dt) = (f32::from_bits(c[0]), f32::from_bits(c[1]), f32::from_bits(c[2]));
            let ty = AmbType { max_speed: f32::from_bits(c[3]), max_accel: f32::from_bits(c[4]), ..AmbType::default() };
            let mut r = vrider(ty);
            r.v = v;
            integrate(&mut r, a, dt);
            let want = f32::from_bits(o[0]);
            if !veq(r.v, want) {
                bad += 1;
                maxulp = maxulp.max(r.v.to_bits().abs_diff(o[0]));
                if a <= 0.0 && v > ty.max_speed {
                    cap_nonpos += 1;
                } else if (r.v - want).abs() <= want.abs() * 2.5e-7 {
                    fma_only += 1;
                }
                if bad < 8 {
                    println!("integrate differs: a {a} v {v} dt {dt} max_speed {} max_accel {} -> port {} original {want}", ty.max_speed, ty.max_accel, r.v);
                }
            }
        }
        // (diagnostic only: the original's rule, fused, capped only while accelerating)
        let mut bad_orig_rule = 0;
        for (c, o) in inp.iter().zip(&rf) {
            let (a0, v, dt) = (f32::from_bits(c[0]), f32::from_bits(c[1]), f32::from_bits(c[2]));
            let (ms, ma) = (f32::from_bits(c[3]), f32::from_bits(c[4]));
            let a = if a0 > 0.0 && a0 > ma { ma } else { a0 };
            let mut nv = a.mul_add(dt, v);
            if a < 0.0 && nv < 0.22352 { nv = 0.0 } else if a > 0.0 && nv > ms { nv = ms }
            if !veq(nv, f32::from_bits(o[0])) { bad_orig_rule += 1; }
        }
        println!("  the rule read off the original's code (fused, cap only while accelerating): {} differ", bad_orig_rule);
        println!("integrate: {} cases, {} differ (max {} ulp; {} of them the port capping a non-positive acceleration at max speed, {} within 2.5e-7 relative)", inp.len(), bad, maxulp, cap_nonpos, fma_only);
        // (270 before fix45: the port capped a non-positive acceleration; now `speed_step`, the original's rule)
        assert_eq!((bad, bad_orig_rule), (0, 0));
    }

    /// The 4-ring (F-60856) and the lock predicate (F-37906) on 1500 cases: N pushes, then the predicate.
    #[test]
    #[ignore]
    fn verify_traffic_lock_ring_and_predicate() {
        let (inp, rf) = (vrows("lock_in.txt", 1), vrows("lock_ref.txt", 0));
        assert_eq!(inp.len(), rf.len());
        let dt = 1.0f32 / 30.0;
        let (mut ring_bad, mut pred_bad, mut pred_true, mut pred_near, mut diag_bad, mut diag_bad_plain) = (0, 0, 0, 0, 0, 0);
        for (c, o) in inp.iter().zip(&rf) {
            let n = c[0] as usize;
            let xs: Vec<f32> = c[1..1 + n].iter().map(|&b| f32::from_bits(b)).collect();
            let k = 1 + n;
            let delta = Vec3::new(f32::from_bits(c[k]), f32::from_bits(c[k + 1]), f32::from_bits(c[k + 2]));
            let slot = c[k + 8] as usize;
            let md = f32::from_bits(o[7]);
            let mut l = Lock::default();
            for (i, &x) in xs.iter().enumerate() {
                if i + 1 == n {
                    l.prev = Some(Vec3::ZERO);
                    l.update(delta, x, Some(slot), md, dt);
                } else {
                    l.update(Vec3::ZERO, x, None, md, dt);
                }
            }
            // the velocity the port forms is the one the original was given
            let v = (delta - Vec3::ZERO) / dt;
            assert!(veq(v.x, f32::from_bits(c[k + 3])) && veq(v.y, f32::from_bits(c[k + 4])) && veq(v.z, f32::from_bits(c[k + 5])));
            let ring_ok = l.n as u32 == o[0] && l.i as u32 == o[1] % 4 && veq(l.sum, f32::from_bits(o[2])) && (0..4).all(|j| veq(l.e[j], f32::from_bits(o[3 + j])));
            if !ring_ok {
                ring_bad += 1;
                if ring_bad < 6 {
                    println!("ring differs: n {} i {} sum {} e {:?} | original {:?}", l.n, l.i, l.sum, l.e, o);
                }
            }
            let pred = o[8] != 0;
            if pred { pred_true += 1; }
            // (diagnostic only: the original's threshold constant is 4.9961185 = 0x409FE034 and |v|^2 is fused z*z + (x*x + y*y*...))
            let v2f = v.z.mul_add(v.z, v.x.mul_add(v.x, v.y * v.y));
            let mean = l.sum / l.n as f32;
            if (slot == 0 && -mean >= md && v2f >= f32::from_bits(0x409F_E034)) != pred { diag_bad += 1; }
            if (slot == 0 && -mean >= md && v.length_squared() >= f32::from_bits(0x409F_E034)) != pred { diag_bad_plain += 1; }
            if l.now() != pred {
                pred_bad += 1;
                if pred_bad < 6 {
                    println!("predicate differs: port {} original {pred}: v2 {} mean {} md {md}", l.now(), v.length_squared(), l.sum / l.n as f32);
                }
            }
            if (v.length_squared() - 4.99612).abs() < 1e-4 { pred_near += 1; }
        }
        println!("lock ring: {} cases, {} differ; predicate: {} differ ({} true, {} within 1e-4 of the speed threshold)", inp.len(), ring_bad, pred_bad, pred_true, pred_near);
        println!("  with the image's threshold 4.9961185: {} differ (fused |v|^2), {} differ (the port's own |v|^2)", diag_bad, diag_bad_plain);
        assert_eq!(ring_bad, 0);
        // (5 before fix45: the port's literal 4.99612 was one ulp above the image's 4.9961185; now `lock_rule`)
        assert_eq!((pred_bad, diag_bad, diag_bad_plain), (0, 0, 0));
    }

    /// The tilt's velocity step (F-84582) and 10-ring (F-14882). Step 1 (always rewrites the inputs,
    /// the port's side of every step is dumped): cargo test; then run_traffic.py run; step 2: cargo test again.
    #[test]
    #[ignore]
    fn verify_traffic_tilt_velocity_and_ring() {
        use std::fmt::Write;
        let mut rng = 0x1234_5678u32;
        let mut rnd = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng >> 8) as f32 / 16777216.0
        };
        // per step: (seq, step, old pos, new pos, old vel, dt, port vel, s, count, idx, sum, ring, fwd kind)
        struct Step { seq: u32, step: u32, old: Vec3, new: Vec3, oldv: Vec3, dt: f32, vel: Vec3, s: (f32, f32), count: usize, idx: usize, sum: (f32, f32), ring: [(f32, f32); 10], fwd_y: bool }
        let mut steps: Vec<Step> = Vec::new();
        for seq in 0..80u32 {
            let fwd_y = seq % 2 == 1;
            let (fwd, right) = if fwd_y { (Vec3::Y, Vec3::Z) } else { (Vec3::X, Vec3::Z) };
            let accel = if seq % 8 < 6 { 0.6 } else { 8.0 };
            let mut t = Tilt::default();
            let mut pos = Vec3::new(rnd() * 100.0 - 50.0, rnd() * 10.0, rnd() * 100.0 - 50.0);
            let mut vel = Vec3::new(rnd() * 6.0 - 3.0, rnd() - 0.5, rnd() * 6.0 - 3.0);
            for step in 0..45u32 {
                let dt = if seq % 5 == 0 { 1.0 / 30.0 + 0.015 * rnd() } else { 1.0 / 30.0 };
                vel += Vec3::new(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5) * accel * dt;
                pos += vel * dt;
                let before = t.prev;
                let (count0, idx0) = (t.count, t.idx);
                t.update(pos, fwd, right, 0.0, dt);
                let w = if count0 < 10 { idx0 } else if idx0 == 10 { 0 } else { idx0 };
                let (old, oldv) = before.unwrap_or((pos, Vec3::ZERO));
                let nv = t.prev.unwrap().1;
                steps.push(Step { seq, step, old, new: pos, oldv, dt, vel: nv, s: t.ring[w], count: t.count, idx: t.idx, sum: t.sum, ring: t.ring, fwd_y });
            }
        }
        let (mut vin, mut rin) = (String::new(), String::new());
        for s in &steps {
            let b = |f: f32| f.to_bits();
            if s.step > 0 {
                let _ = writeln!(vin, "vel {} {} {} {} {} {} {} {} {} {} {} {}", s.seq, s.step, b(s.old.x), b(s.old.y), b(s.old.z), b(s.new.x), b(s.new.y), b(s.new.z), b(s.oldv.x), b(s.oldv.y), b(s.oldv.z), b(s.dt));
            }
            let _ = writeln!(rin, "ring {} {} {} {}", s.seq, s.step, b(s.s.0), b(s.s.1));
        }
        std::fs::write(format!("{VFIX}vel_in.txt"), vin).unwrap();
        std::fs::write(format!("{VFIX}ring_in.txt"), rin).unwrap();
        let (vref, rref) = match (std::fs::read_to_string(format!("{VFIX}vel_ref.txt")), std::fs::read_to_string(format!("{VFIX}ring_ref.txt"))) {
            (Ok(a), Ok(b)) => (a, b),
            _ => {
                println!("inputs written; run run_traffic.py run, then run this test again");
                return;
            }
        };
        let parse = |t: &str| t.lines().map(|l| l.split_whitespace().map(|x| x.parse::<u32>().unwrap()).collect::<Vec<_>>()).collect::<Vec<_>>();
        let (vref, rref) = (parse(&vref), parse(&rref));
        let moving: Vec<&Step> = steps.iter().filter(|s| s.step > 0).collect();
        assert_eq!(vref.len(), moving.len());
        assert_eq!(rref.len(), steps.len());
        let (mut vel_bad, mut dv_bad, mut dv_unclamped, mut maxulp, mut newton_bad, mut dv_checked) = (0, 0, 0, 0u32, 0, 0);
        for (s, o) in moving.iter().zip(&vref) {
            assert_eq!((o[0], o[1]), (s.seq, s.step));
            let orig_v = Vec3::new(f32::from_bits(o[2]), f32::from_bits(o[3]), f32::from_bits(o[4]));
            if !(veq(orig_v.x, s.vel.x) && veq(orig_v.y, s.vel.y) && veq(orig_v.z, s.vel.z)) {
                vel_bad += 1;
                for (a, b) in [(orig_v.x, s.vel.x), (orig_v.y, s.vel.y), (orig_v.z, s.vel.z)] {
                    maxulp = maxulp.max(a.to_bits().abs_diff(b.to_bits()));
                }
                if vel_bad < 6 {
                    println!("velocity differs seq {} step {}: port {:?} original {:?}", s.seq, s.step, s.vel, orig_v);
                }
            }
            // (diagnostic only: the original's velocity is the position change times the reciprocal of dt
            // refined by one Newton step, r = r0 + r0 * -(r0 * dt - 1), r0 = 1 / dt, in f32)
            let r0 = 1.0 / s.dt;
            let r = r0.mul_add(-(r0 * s.dt - 1.0), r0);
            let dp = s.new - s.old;
            let nv = Vec3::new(dp.x * r, dp.y * r, dp.z * r);
            if !(veq(orig_v.x, nv.x) && veq(orig_v.y, nv.y) && veq(orig_v.z, nv.z)) {
                newton_bad += 1;
            }
            // dv: the original's dv is its velocity minus the old one (checked), the port keeps only s: the
            // original's dv through the port's formula must give s - compared where the velocities agree
            let dv = Vec3::new(f32::from_bits(o[5]), f32::from_bits(o[6]), f32::from_bits(o[7]));
            assert!(veq(dv.x, orig_v.x - s.oldv.x) && veq(dv.y, orig_v.y - s.oldv.y) && veq(dv.z, orig_v.z - s.oldv.z));
            let same_vel = veq(orig_v.x, s.vel.x) && veq(orig_v.y, s.vel.y) && veq(orig_v.z, s.vel.z);
            if !same_vel {
                continue;
            }
            dv_checked += 1;
            let (fwd, right) = if s.fwd_y { (Vec3::Y, Vec3::Z) } else { (Vec3::X, Vec3::Z) };
            let want = ((dv.dot(fwd) / PITCH_DOT).clamp(-1.0, 1.0) * PITCH_ANGLE.to_radians(), (dv.dot(right) / ROLL_DOT).clamp(-1.0, 1.0) * ROLL_ANGLE.to_radians());
            if (dv.dot(fwd) / PITCH_DOT).abs() < 1.0 && (dv.dot(right) / ROLL_DOT).abs() < 1.0 {
                dv_unclamped += 1;
            }
            if !(veq(want.0, s.s.0) && veq(want.1, s.s.1)) {
                dv_bad += 1;
                if dv_bad < 6 {
                    println!("dv differs seq {} step {}: port s {:?} from original dv {:?}", s.seq, s.step, s.s, want);
                }
            }
        }
        let mut ring_bad = 0;
        for (s, o) in steps.iter().zip(&rref) {
            assert_eq!((o[0], o[1]), (s.seq, s.step));
            let mut ok = o[2] as usize == s.count && o[3] as usize == s.idx && veq(s.sum.0, f32::from_bits(o[4])) && veq(s.sum.1, f32::from_bits(o[5]));
            for k in 0..10 {
                ok &= veq(s.ring[k].0, f32::from_bits(o[6 + 2 * k])) && veq(s.ring[k].1, f32::from_bits(o[7 + 2 * k]));
            }
            if !ok {
                ring_bad += 1;
                if ring_bad < 6 {
                    println!("ring differs seq {} step {}: port count {} idx {} sum {:?} | original {:?}", s.seq, s.step, s.count, s.idx, s.sum, &o[2..6]);
                }
            }
        }
        println!("tilt: {} velocity steps, velocity differs {} (max {} ulp); the reciprocal-and-Newton form differs {}; dv->s checked on {} steps ({} unclamped), differs {}; ring: {} steps, {} differ", moving.len(), vel_bad, maxulp, newton_bad, dv_checked, dv_unclamped, dv_bad, steps.len(), ring_bad);
        assert_eq!(ring_bad, 0);
        // (1,562 before fix45: the port divided; now `newton_velocity`, the reciprocal and its Newton step)
        assert_eq!((vel_bad, newton_bad, dv_bad), (0, 0, 0));
    }

    /// F-50959's box: |a70 . d| < 0.4 h3c and |a60 . d| < 0.4 h40, the height left out.
    #[test]
    fn a_physical_or_resting_car_is_not_put_back_on_its_seat() {
        // (a seat from before the hit: physical or at rest the instance stays where the collider left it)
        let mut r = Rider { ty: AmbType::default(), link: 0, s: 0.0, next: None, v: 0.0, off: 0.0, off_timer: 0.0, bonus: 0.0, gap: 3.0, wait: 0.0, waited: 0.0, on: [false; 3], stop_id: None, obst_id: None, state: 1, physical: false, path: None, then: None, stand: 0.0, pass_stand: 0.0, stuck: (0.0, 0.0, 0), passing: None, kept: None, rest: 0.0, partner: None, seat: None, tilt: Tilt::default(), in_level: false, active: false, wake: false, rng: 7, id: 7, mwc: mwc_seed(7), ds: 0.0, ds_lane: usize::MAX, t: 0.0, ahead: (0.0, 0.0), a: 0.0, slot: None, lock: Lock::default(), born: 0.0, life: 8.0, route: None, race_lane: None, phys_pair: false, quiet: false, path_referenced: false, obst_v: None, steer: 0.0, rail_seq: 0, rail_key: None, rail_fresh: true, ghost: None, mark: None };
        r.seat = Some(Seat::new(Vec3::ZERO, Vec3::Y, Vec3::NEG_Z));
        assert!(seat_poses(&r));
        for (state, active) in [(4, false), (4, true), (5, false), (5, true), (1, true)] {
            r.state = state;
            r.active = active;
            assert!(!seat_poses(&r), "state {state} active {active}");
        }
        r.active = false;
        for state in [1, 2, 3, 6, 7, 8] {
            r.state = state;
            assert!(seat_poses(&r), "state {state}");
        }
    }

    #[test]
    fn the_junction_box_is_four_tenths_of_its_half_extents() {
        let b = JunctionBox { c: (10.0, -20.0), a60: [1.0, 5.0, 0.0], a70: [0.0, 5.0, 1.0], h3c: 10.0, h40: 20.0 };
        assert!(b.holds(Vec3::new(10.0, 100.0, -20.0)));
        assert!(b.holds(Vec3::new(10.0 + 7.9, 0.0, -20.0 + 3.9)));
        assert!(!b.holds(Vec3::new(10.0 + 8.0, 0.0, -20.0)));
        assert!(!b.holds(Vec3::new(10.0, 0.0, -20.0 - 4.0)));
    }

    /// bPhysical (instance +140 bit 0x80) only lets inactive pairs in while the car is posed by
    /// its rail (slot 20 / F-86858: the follower has a rail).
    #[test]
    fn a_physical_path_car_pairs_only_while_on_a_rail() {
        let mut r = Rider { ty: AmbType::default(), link: 0, s: 0.0, next: None, v: 0.0, off: 0.0, off_timer: 0.0, bonus: 0.0, gap: 3.0, wait: 0.0, waited: 0.0, on: [false; 3], stop_id: None, obst_id: None, state: 1, physical: false, path: None, then: None, stand: 0.0, pass_stand: 0.0, stuck: (0.0, 0.0, 0), passing: None, kept: None, rest: 0.0, partner: None, seat: None, tilt: Tilt::default(), in_level: false, active: false, wake: false, rng: 7, id: 7, mwc: mwc_seed(7), ds: 0.0, ds_lane: usize::MAX, t: 0.0, ahead: (0.0, 0.0), a: 0.0, slot: None, lock: Lock::default(), born: 0.0, life: 8.0, route: None, race_lane: None, phys_pair: false, quiet: false, path_referenced: false, obst_v: None, steer: 0.0, rail_seq: 0, rail_key: None, rail_fresh: true, ghost: None, mark: None };
        assert!(!r.pairs_inactive());
        r.phys_pair = true;
        for (state, yes) in [(1u8, true), (9, true), (10, false), (0, false), (4, false), (5, false), (6, false)] {
            r.state = state;
            assert_eq!(r.pairs_inactive(), yes, "state {state}");
        }
    }

    /// The rail onto a lane (F-26466, the recovery and rejoin rails) against the original's builder run whole offline
    /// (tools/traffic_rails_reference.py --mode recovery -> testdata/traffic_rails_recovery_ref.txt): the car's own MWC,
    /// the lane picks and phases, every target asked for (lane, phase), every rail swept (turns, radius, flags) in order,
    /// the pick (the whole arc-line-arc record, the lane, the end phase), the speed and the MWC after.
    #[test]
    fn rail_to_lane_matches_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let u = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let x = |f: f32| format!("{:08x}", f.to_bits());
        let xv = |v: Vec3| format!("{} {} {}", x(v.x), x(v.y), x(v.z));
        let (mut n, mut built, mut swept, mut asked) = (0, 0, 0, 0);
        for line in include_str!("testdata/traffic_rails_recovery_ref.txt").lines() {
            let (i, o) = line.split_once(" ->").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let o: Vec<&str> = o.split_whitespace().collect();
            let v3 = |k: usize| Vec3::new(h(w[k]), h(w[k + 1]), h(w[k + 2]));
            let req = RailReq { p: v3(0), h0: v3(3), v: h(w[6]), limit: h(w[7]), ready: u(w[11]) != 0 };
            let nl = u(w[16]) as usize;
            let cands: Vec<(usize, f32, bool)> = (0..nl).map(|k| (k, h(w[17 + 8 * k + 6]), u(w[17 + 8 * k + 7]) != 0)).collect();
            // the original's calls in order
            let mut targets = std::collections::VecDeque::new();
            let mut sweeps = std::collections::VecDeque::new();
            let mut k = 0;
            let mut end = None;
            while k < o.len() {
                match o[k] {
                    "Q" => k += 6,
                    "T" => {
                        targets.push_back((o[k + 1].parse::<usize>().unwrap(), o[k + 2].to_string(), Vec3::new(h(o[k + 3]), h(o[k + 4]), h(o[k + 5])), Vec3::new(h(o[k + 6]), h(o[k + 7]), h(o[k + 8]))));
                        k += 9;
                    }
                    "S" => {
                        sweeps.push_back((o[k + 1..k + 5].join(" "), o[k + 5] == "1"));
                        k += 11;
                    }
                    "A" => {
                        end = Some(o[k + 2].to_string());
                        k += 4;
                    }
                    "R" => {
                        if o[k + 1] == "1" {
                            built += 1;
                            let m = o.iter().position(|&t| t == "M").unwrap();
                            let rest = o[k + 2..m].join(" ");
                            end = Some(format!("{} {}", end.take().unwrap(), rest));
                        }
                        break;
                    }
                    t => panic!("token {t}"),
                }
            }
            let m = o.iter().position(|&t| t == "M").unwrap();
            let mut mwc = super::super::parked::Mwc { lo: w[12].parse().unwrap(), carry: w[13].parse().unwrap() };
            let (nt, ns) = (targets.len(), sweeps.len());
            let pick = rail_to_lane(&req, h(w[8]), h(w[9]), &cands, &mut || mwc.next(), &mut |lane, ph| {
                let (l, p, a, d) = targets.pop_front().expect("a target the original did not ask for");
                assert_eq!((lane, x(ph)), (l, p), "{line}");
                (a, d)
            }, &mut |q| {
                let (s, hit) = sweeps.pop_front().expect("a sweep the original did not run");
                assert_eq!(format!("{} {} {} {:02x}", x(q.a1), x(q.a2), x(q.r), q.flags), s, "{line}");
                !hit
            });
            assert!(targets.is_empty() && sweeps.is_empty(), "{line}");
            asked += nt;
            swept += ns;
            assert_eq!(format!("{} {}", mwc.lo, mwc.carry), o[m + 1..m + 3].join(" "), "{line}");
            match pick {
                None => assert!(end.is_none(), "{line}"),
                Some(pk) => {
                    let q = pk.rail;
                    let got = format!(
                        "{} 00001580 {} 00000000 {} 80 {} {} {} {} {} {} {} {} {} {} {} {:02x}",
                        x(pk.speed), pk.lane, x(pk.phase), xv(q.a), xv(q.b), xv(q.c1), xv(q.c2), xv(q.p), xv(q.t), xv(q.h0), xv(q.h1), x(q.a1), x(q.a2), x(q.r), q.flags
                    );
                    assert_eq!(Some(got), end, "{line}");
                }
            }
            n += 1;
        }
        assert_eq!((n, built, asked, swept), (1497, 873, 12413, 3222));
    }

    /// A race path's join rail (F-45519) against the original's builder run whole offline (tools/traffic_rails_reference.py
    /// --mode join -> testdata/traffic_rails_join_ref.txt): the start heading from the file's vertices or the spawn
    /// matrix, the four arc-line-arc rails and the strictly shortest valid one, every word of its record.
    #[test]
    fn join_rail_matches_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let x = |f: f32| format!("{:08x}", f.to_bits());
        let xv = |v: Vec3| format!("{} {} {}", x(v.x), x(v.y), x(v.z));
        let (mut n, mut built) = (0, 0);
        for line in include_str!("testdata/traffic_rails_join_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let v3 = |k: usize| Vec3::new(h(w[k]), h(w[k + 1]), h(w[k + 2]));
            let nv = usize::from_str_radix(w[0], 16).unwrap();
            let verts: Vec<Vec3> = (0..nv).map(|k| v3(8 + 3 * k)).collect();
            let snap: Vec<Vec3> = (0..nv).map(|k| v3(32 + 3 * k)).collect();
            let h0 = join_heading(&verts, v3(5));
            let got = match join_rail(snap[nv - 2], h0, snap[nv - 1], v3(2), h(w[1])) {
                None => "R 0".to_string(),
                Some(q) => {
                    built += 1;
                    format!(
                        "R 1 {} {} {} {} {} {} {} {} {} {} {} {:02x}",
                        xv(q.a), xv(q.b), xv(q.c1), xv(q.c2), xv(q.p), xv(q.t), xv(q.h0), xv(q.h1), x(q.a1), x(q.a2), x(q.r), q.flags
                    )
                }
            };
            assert_eq!(got, o, "{line}");
            n += 1;
        }
        assert_eq!((n, built), (2000, 1795));
    }

    /// The backing-up rail (F-54867) against the original's builder run whole offline (tools/traffic_rails_reference.py
    /// --mode backing -> testdata/traffic_rails_backing_ref.txt): the four control points from the car's place and row 2,
    /// the pool (index 0, speed 4) and the attach (start phase 1, reversed).
    #[test]
    fn backing_rail_matches_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let x = |f: f32| format!("{:08x}", f.to_bits());
        let xv = |v: Vec3| format!("{} {} {}", x(v.x), x(v.y), x(v.z));
        let mut n = 0;
        for line in include_str!("testdata/traffic_rails_backing_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let v3 = |k: usize| Vec3::new(h(w[k]), h(w[k + 1]), h(w[k + 2]));
            let p = backing_rail(v3(0), v3(3));
            let Geom::Curve(cv) = &p.geom else { panic!("{line}") };
            assert!(p.end.is_none(), "{line}");
            let cps: Vec<String> = cv.pts.iter().map(|&q| xv(q)).collect();
            let got = format!("A 0 {} X 1 1 0 {} 0 {} 00000000 R 1 {}", x(p.speed), p.reverse as u8, x(p.t), cps.join(" "));
            assert_eq!(got, o, "{line}");
            n += 1;
        }
        assert_eq!(n, 2000);
    }

    /// The cruising ghost's lane-change rail F-11487 against the original's code run whole offline
    /// (tools/traffic_rails_reference.py --mode lanechange -> testdata/traffic_rails_lanechange_ref.txt): the calls it
    /// makes in order (the two curves' points at t and at the phase, the target's slot 17, the pool index 0 at that
    /// speed, the road and cast, the attach from phase 0 with the target lane at the phase, the obstacle's slot 12 on both
    /// lanes with -1), the four control points bit for bit and the holder's +32 .. +52.
    #[test]
    fn lane_change_rail_matches_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let x = |f: f32| format!("{:08x}", f.to_bits());
        let xv = |v: Vec3| format!("{} {} {}", x(v.x), x(v.y), x(v.z));
        let mut n = 0;
        for line in include_str!("testdata/traffic_rails_lanechange_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let v3 = |k: usize| Vec3::new(h(w[k]), h(w[k + 1]), h(w[k + 2]));
            let (t, ph, wd, sp, byte) = (h(w[12]), h(w[13]), h(w[14]), h(w[15]), w[16].trim_start_matches('0'));
            let p = lane_change_path([v3(0), v3(3), v3(6), v3(9)], wd, sp, 1, ph, None);
            let Geom::Curve(cv) = &p.geom else { panic!("{line}") };
            let (to, end_t) = p.end.unwrap();
            assert_eq!(to, 1);
            let cps: Vec<String> = cv.pts.iter().map(|&q| xv(q)).collect();
            let got = format!(
                "S15 0 E 1 {} S15 1 E 0 {} S17 1 A 1 0 {} O 1 C 1 0 X 1 1 1 0 1 1 {} {} V 5888 bf800000 V 5952 bf800000 R 1 {} H 1 1 1 {} {} {}",
                x(t), x(ph), x(p.speed), x(p.t), x(end_t), cps.join(" "), x(t), x(ph), if byte.is_empty() { "0" } else { byte }
            );
            assert_eq!(got, o, "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// The install F-32118 run whole with F-11487 and the attach F-70405 inside it (tools/traffic_rails_reference.py
    /// --mode lcinstall -> testdata/traffic_rails_lcinstall_ref.txt): the follower's rail unloaded (F-78813), each attached
    /// holder released (+92, +4, +36, +124), the new rail set at the port's start phase (F-48384 with `Path::t`), the holder copied in (+40 the
    /// pool, +44 the rail, +48 the target lane, +56 / +60 the start and end phases, +64 the attached bit, +68 .. +84 the
    /// car, both lanes, t and the phase), the next rail +160 = the target lane at +164 = `Path::end`'s phase.
    #[test]
    fn lane_change_install_matches_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let u = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let x = |f: f32| format!("{:08x}", f.to_bits());
        let xv = |v: Vec3| format!("{} {} {}", x(v.x), x(v.y), x(v.z));
        let mut n = 0;
        for line in include_str!("testdata/traffic_rails_lcinstall_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let v3 = |k: usize| Vec3::new(h(w[k]), h(w[k + 1]), h(w[k + 2]));
            let (t, ph, wd, sp, bits) = (h(w[12]), h(w[13]), h(w[14]), h(w[15]), u(w[17]));
            let p = lane_change_path([v3(0), v3(3), v3(6), v3(9)], wd, sp, 1, ph, None);
            let Geom::Curve(cv) = &p.geom else { panic!("{line}") };
            let (_, end_t) = p.end.unwrap();
            let cps: Vec<String> = cv.pts.iter().map(|&q| xv(q)).collect();
            // (released in the order +92, +4, +36, +124; the case's bit k belongs to holder [4, 36, 92, 124][k])
            let released: String = [(2, 92), (0, 4), (1, 36), (3, 124)].iter().filter(|&&(k, _)| bits >> k & 1 != 0).map(|(_, o)| format!(" D {o}")).collect();
            let got = format!(
                "A 0 {} V 5888 bf800000 V 5952 bf800000 U 1{released} B 1 1 {} R 1 {} F 00003800 00006000 00001a00 00001740 00001900 {} {} {} 00001200 00001700 00001740 {} {} 00000000 {} 00001740 {}",
                x(p.speed), x(p.t), cps.join(" "), x(p.t), x(end_t), if bits & 2 != 0 { "80000000" } else { "85000000" }, x(t), x(ph), w[18], x(end_t)
            );
            assert_eq!(got, o, "{line}");
            n += 1;
        }
        assert_eq!(n, 1000);
    }

    /// The recovery request (F-26749) and the rejoin request (F-89026) against the original's builders run whole offline
    /// (tools/traffic_rails_reference.py --mode request -> testdata/traffic_rails_request_ref.txt): the rail a real
    /// arc-line-arc record, F-26466 a stub printing the request.
    #[test]
    fn rail_requests_match_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let u = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let x = |f: f32| format!("{:08x}", f.to_bits());
        let xv = |v: Vec3| format!("{} {} {}", x(v.x), x(v.y), x(v.z));
        let mut kinds = [0; 3];
        for line in include_str!("testdata/traffic_rails_request_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let v3 = |k: usize| Vec3::new(h(w[k]), h(w[k + 1]), h(w[k + 2]));
            let q = crate::tcp_rail::build(v3(0), v3(3), v3(6), v3(9), h(w[12]), u(w[13]) != 0, u(w[14]) != 0);
            let kind = u(w[17]) as usize;
            let req = match kind {
                0 => rejoin_req(&q, h(w[15])),
                k => {
                    let mut pp = Path::tcp(q, h(w[15]), None);
                    pp.t = h(w[16]);
                    let fwd = crate::tcp_rail::vdiff(Vec3::ZERO, v3(21));
                    recovery_req(if k == 1 { Some(&pp) } else { None }, v3(18), fwd, h(w[24]), u(w[25]) != 0)
                }
            };
            let got = format!("K {} {} {} {} {} {:02x}", kind, xv(req.p), xv(req.h0), x(req.v), x(req.limit), if req.ready { 0x80 } else { 0 });
            assert_eq!(got, o, "{line}");
            kinds[kind] += 1;
        }
        assert_eq!(kinds, [1032, 1002, 966]);
    }

    /// The passing rail (F-92334 with the lane test F-50817) against the original's builder run whole offline
    /// (tools/traffic_rails_reference.py --mode pass -> testdata/traffic_rails_pass_ref.txt): the start from the follower's
    /// rail or the car's matrix, the obstacle's corners, the picks, the targets, the eight rails; every sweep (the rail
    /// and the request words), lane query (point, radius and the entries answered), pavement test and probe in order;
    /// the pick (every word of its record), the pool's speed and the attach.
    #[test]
    fn passing_rail_matches_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let u = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let x = |f: f32| format!("{:08x}", f.to_bits());
        let xv = |v: Vec3| format!("{} {} {}", x(v.x), x(v.y), x(v.z));
        let (mut n, mut built) = (0, 0);
        let calls = std::cell::RefCell::new([0usize; 4]);
        let mut tiers = [0usize; 2];
        for line in include_str!("testdata/traffic_rails_pass_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let o: Vec<&str> = o.split_whitespace().collect();
            let v3 = |k: usize| Vec3::new(h(w[k]), h(w[k + 1]), h(w[k + 2]));
            let flags = u(w[15]) as u8;
            let ready = flags & 0x10 != 0;
            let kind = u(w[19]);
            let rq = if kind == 1 {
                let q = crate::tcp_rail::build(v3(26), v3(29), v3(32), v3(35), h(w[38]), u(w[39]) != 0, u(w[40]) != 0);
                let mut pp = Path::tcp(q, 0.0, None);
                pp.t = h(w[41]);
                recovery_req(Some(&pp), Vec3::ZERO, Vec3::ZERO, 0.0, false)
            } else {
                recovery_req(None, v3(20), crate::tcp_rail::vdiff(Vec3::ZERO, v3(23)), 0.0, false)
            };
            let ty = AmbType { min_turn: h(w[17]), pivot_turn: h(w[18]), ..Default::default() };
            let req = PassReq {
                p: rq.p,
                h0: rq.h0,
                row0: v3(3),
                row2: v3(6),
                place: v3(0),
                w: h(w[9]),
                l: h(w[10]),
                w_own: h(w[11]),
                c: h(w[12]),
                radius: turn_radius(&ty, h(w[16]), ready),
                gate: h(w[14]),
                flags,
            };
            // the original's calls in order
            let mut seq: std::collections::VecDeque<(char, Vec<&str>)> = std::collections::VecDeque::new();
            let mut k = 0;
            let mut tail = None;
            while k < o.len() {
                let len = match o[k] {
                    "S" => 15,
                    "Q" => 8 + 5 * o[k + 7].parse::<usize>().unwrap(),
                    "N" => 7,
                    "G" => 11,
                    "A" | "R" => {
                        tail = Some(o[k..].join(" "));
                        break;
                    }
                    t => panic!("token {t}"),
                };
                seq.push_back((o[k].chars().next().unwrap(), o[k + 1..k + len].to_vec()));
                k += len;
            }
            let seq = std::cell::RefCell::new(seq);
            let next = |c: char| {
                let (t, a) = seq.borrow_mut().pop_front().unwrap_or_else(|| panic!("a call {c} the original did not make: {line}"));
                assert_eq!(t, c, "{line}");
                calls.borrow_mut()["SQNG".find(c).unwrap()] += 1;
                a
            };
            let pick = pass_pick(
                &req,
                &mut |q| {
                    let a = next('S');
                    // (the record's flag byte below 0x20 is a stack leftover in the original)
                    assert_eq!(format!("{} {:02x}", a[..3].join(" "), u(a[3]) & 0xE0), format!("{} {} {} {:02x}", x(q.a1), x(q.a2), x(q.r), q.flags), "{line}");
                    assert_eq!(a[5..].join(" "), format!("{} 00000001 00003070 00000000 ffffffff 00000000 00000000 bf800000 {}", w[11], w[13]), "{line}");
                    a[4] == "1"
                },
                &mut |at, rad| {
                    let a = next('Q');
                    assert_eq!(a[..6].join(" "), format!("{} {} 64 1", xv(at), x(rad)), "{line}");
                    let m: usize = a[6].parse().unwrap();
                    (0..m)
                        .map(|e| LaneNear { tangent: Vec3::new(h(a[7 + 5 * e]), h(a[8 + 5 * e]), h(a[9 + 5 * e])), d2: h(a[10 + 5 * e]), skip: matches!(a[11 + 5 * e], "2" | "3") })
                        .collect()
                },
                &mut |t| {
                    let a = next('N');
                    assert_eq!(a[..5].join(" "), format!("{} 41200000 42c80000", xv(t)), "{line}");
                    a[5] == "1"
                },
                &mut |top, bottom| {
                    let a = next('G');
                    assert_eq!(a[..9].join(" "), format!("{} {} 00000800 07 1", xv(top), xv(bottom)), "{line}");
                    a[9] == "1"
                },
            );
            assert!(seq.borrow().is_empty(), "{line}");
            let got = match pick {
                None => "R 0".to_string(),
                Some(q) => {
                    built += 1;
                    let speed = if ready {
                        1.1176f32
                    } else {
                        let f12 = h(w[16]) * 0.7;
                        if f12 - 4.4704 >= 0.0 { f12 } else { 4.4704 }
                    };
                    format!(
                        "A 1 {} 1 X 1 1 {} 00000000 00000000 0 0 R 1 {} {} {} {} {} {} {} {} {} {} {} {:02x}",
                        x(speed), kind, xv(q.a), xv(q.b), xv(q.c1), xv(q.c2), xv(q.p), xv(q.t), xv(q.h0), xv(q.h1), x(q.a1), x(q.a2), x(q.r), q.flags
                    )
                }
            };
            assert_eq!(Some(got), tail, "{line}");
            tiers[(flags & 0x20 == 0) as usize] += 1;
            n += 1;
        }
        assert_eq!((n, built, *calls.borrow(), tiers), (4000, 1394, [4323, 2625, 807, 589], [3194, 806]));
    }
}

// ---------------------------------------------------------------------------------------
// The spawner and unspawner (density manager `mcAmbientDensityTuning`, vtable 0x820C39C4;
// F-47889: the region list on odd manager frames, F-49549; spawning F-92144
// and unspawning F-65438 on even ones - 15 Hz each). Decoded 2026-10-19:
// docs/PEOPLE_TRAFFIC.md "Ambient traffic spawner".

/// A road as the spawner weighs it (`mcRnRoadAmbient`).
pub struct Road {
    pub mid: Vec3,
    pub dir: Vec3,
    pub speed: f32,
    pub density: i32,
    /// its lanes: first link and length
    pub lanes: Vec<(usize, f32)>,
    /// the record's lanes' lengths summed (F-72428 over F-95725's list, `RecordData::road_lanes`: its own lanes, then
    /// the sibling chain +0x0c with each one's child subtree; slot 20, the lane's +0x0c, added in order from 0)
    pub lane_len: f32,
    /// the box the occluders are asked about (road vfunc 2 F-76320; empty: never hidden)
    pub bmin: Vec3,
    pub bmax: Vec3,
    /// the rectangle the front-end mode's sphere and frustum tests take (F-46547, F-49549)
    pub rect: RegionRect,
}

impl Road {
    /// The road's lane length for its spawn weight (F-72428: over the record's lanes `RecordData::road_lanes`, each
    /// one's length +0x0c, 0 while the race traffic clearing holds it - its byte +104, mcRailAmbient slot 18 F-73172 -,
    /// `fadds` in order from 0).
    fn open_len(nav: &NavSys, road: usize, off: &hashbrown::HashSet<usize>) -> f32 {
        let lanes = nav.records.road_lanes.get(road).map_or(&[][..], |v| v.as_slice());
        lanes.iter().fold(0.0f32, |t, &l| (if off.contains(&l) { 0.0 } else { nav.lane_len(l) }) + t)
    }
}

/// F-10273 as the traffic spawner calls it (no direction: every lane a candidate): the road record's lanes (`lanes`:
/// its own, then its child's subtree - F-95725 / F-12622, `RecordData::road_lanes`) weighed by `weight` (slot 18 set -
/// the race traffic clearing holds it - 0, else slot 20, the lane's length +0x0c), the total added in order; u = (U23 x
/// total) x 2^-23 from the shared MWC; the first lane whose weight is not below what is left of u after the ones before
/// are taken off (a NaN stops), at most the last; a second draw U23 x 2^-23 is the phase along it. None (no draw)
/// without lanes. Returns the lane's index in `lanes` and the phase.
pub fn pick_lane(lanes: &[usize], weight: impl Fn(usize) -> f32, mwc: &mut super::parked::Mwc) -> Option<(usize, f32)> {
    if lanes.is_empty() {
        return None;
    }
    let w: Vec<f32> = lanes.iter().map(|&l| weight(l)).collect();
    let mut total = 0.0f32;
    for &x in &w {
        total = x + total;
    }
    let u23 = (mwc.next() & 0x7F_FFFF) as f32;
    let mut u = u23 * total * f32::from_bits(0x3400_0000);
    let mut i = 0;
    while i < w.len() && u > w[i] {
        u -= w[i];
        i += 1;
    }
    let i = i.min(w.len() - 1);
    let t = (mwc.next() & 0x7F_FFFF) as f32 * f32::from_bits(0x3400_0000);
    Some((i, t))
}

/// `density_tuning.xml`.
#[derive(Clone, Copy)]
pub struct Tune {
    pub box_len: [f32; 2],
    pub box_off: [f32; 2],
    pub box_width: [f32; 2],
    pub perp_len: [f32; 2],
    pub perp_off: [f32; 2],
    pub perp_width: [f32; 2],
    pub spacing: f32,
    pub fov_mph: [f32; 2],
    pub fov_deg: [f32; 2],
    pub density_mph: [f32; 2],
    pub density_at: [f32; 2],
    pub values: [f32; 7],
    pub spawn_inner: f32,
    pub spawn_outer: f32,
    pub unspawn_outer: f32,
    /// the parked cars (vehicles/parked.rs): the unspawner's inner radius (M+12), the radius
    /// inside which a spot must be unseen (M+68) and the share of a node's spots filled (M+152)
    pub unspawn_inner: f32,
    pub parked_radius: f32,
    pub parked_density: f32,
}

impl Tune {
    fn load(root: &std::path::Path) -> Tune {
        let t = std::fs::read_to_string(root.join("tune/ambients/density_tuning.xml")).unwrap_or_default();
        let g = |tag: &str, d: f32| crate::ambient::xml_value(&t, tag).unwrap_or(d);
        let mut values = [0.0, 0.02, 0.35, 0.55, 0.70, 1.25, 0.10];
        if let Some(i) = t.find("<DensityValues") {
            let body = &t[i..];
            if let (Some(a), Some(b)) = (body.find('>'), body.find("</DensityValues>")) {
                let v: Vec<f32> = body[a + 1..b].split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if v.len() == 7 {
                    values.copy_from_slice(&v);
                }
            }
        }
        Tune {
            box_len: [g("fTrafficBoxLengthAtLowSpeed", 700.0), g("fTrafficBoxLengthAtHighSpeed", 500.0)],
            box_off: [g("fTrafficBoxOffsetAtLowSpeed", 100.0), g("fTrafficBoxOffsetAtHighSpeed", 100.0)],
            box_width: [g("fTrafficBoxWidthAtLowSpeed", 100.0), g("fTrafficBoxWidthAtHighSpeed", 100.0)],
            perp_len: [g("fTrafficBoxLengthAtLowSpeedPerp", 400.0), g("fTrafficBoxLengthAtHighSpeedPerp", 400.0)],
            perp_off: [g("fTrafficBoxOffsetAtLowSpeedPerp", 0.0), g("fTrafficBoxOffsetAtHighSpeedPerp", 0.0)],
            perp_width: [g("fTrafficBoxWidthAtLowSpeedPerp", 400.0), g("fTrafficBoxWidthAtHighSpeedPerp", 400.0)],
            spacing: g("fMinSpawnTrafficSpacing", 15.0),
            fov_mph: [g("fFOVDegreesLowMPH", 25.0), g("fFOVDegreesHighMPH", 45.0)],
            fov_deg: [g("fFOVDegreesAtLowSpeed", 89.9), g("fFOVDegreesAtHighSpeed", 45.0)],
            density_mph: [g("fDensityLowMPH", 80.0), g("fDensityHighMPH", 200.0)],
            density_at: [g("fDensityAtLowSpeed", 1.0), g("fDensityAtHighSpeed", 0.4)],
            values,
            spawn_inner: g("fTrafficSpawnInnerRadius", 50.0),
            spawn_outer: g("fTrafficSpawnOuterRadius", 250.0),
            unspawn_outer: g("fTrafficUnspawnOuterRadius", 400.0),
            unspawn_inner: g("fTrafficUnspawnInnerRadius", 120.0),
            parked_radius: g("fParkedCarRadius", 100.0),
            parked_density: g("fParkedCarDensityFactor", 0.25),
        }
    }
}

/// The spawner's state: the roads, the junctions and this frame's region list.
pub struct Spawner {
    pub roads: Vec<Road>,
    /// junctions: centre, weight (+1c) and reach (+40)
    pub ints: Vec<(Vec3, f32, f32)>,
    pub tune: Tune,
    pub data_dir: String,
    /// the region list: roads with their weight (target share), and the junctions' weight
    pub list: Vec<(u32, f32)>,
    pub int_weight: f32,
    /// the whole region list in its order (M+1392): a road or a junction (by its index in the file) and its weight -
    /// what a spawn try picks from
    pub order: Vec<(NavNode, f32)>,
    pub listed: Vec<bool>,
    pub target: f32,
    pub current: u32,
    /// 30 Hz frames run and the time not yet run
    frame: u32,
    step: f32,
    /// script densities by road (`RoadDensity`, road +164)
    pub script: hashbrown::HashMap<u32, i32>,
    /// (diagnostics) spawn tries turned down, by reason
    pub why: [u32; 8],
    /// the player's road (M+320), kept while the player is on no road's lane
    pub player_road: Option<u32>,
    /// M+324: the square box across the road on the next pass (set at the end of each region list from the nearest
    /// listed road, F-49549)
    pub perp_next: bool,
    /// this frame's region box: centre, unit length axis (x, z), full length and width
    /// (the parked-car lists take the same box, F-51021)
    pub region_box: (glam::Vec2, glam::Vec2, f32, f32),
    /// the cutscene mode this frame (F-97032): the parked lists' box as wide as it is long
    pub cutscene: bool,
    /// the front-end mode this frame (F-95688): its camera and values
    pub frontend: Option<FrontEnd>,
    /// the junctions' rectangles (as `ints`), the junctions the front-end region list holds,
    /// and the front-end lane points (ambient S+48, F-56167)
    pub int_rects: Vec<RegionRect>,
    /// a junction's place in `int_rects` / `ints` by its index in the file (NavNode::Junction), and its occluder box
    /// (the roads' rule, F-76320)
    pub int_of: hashbrown::HashMap<usize, usize>,
    pub int_box: Vec<(Vec3, Vec3)>,
    pub fe_ints: Vec<usize>,
    pub fe_pts: FrontEndPoints,
    /// every lane's links on the rail graph, in order
    lane_links: Vec<Vec<usize>>,
}

fn lerp(a: [f32; 2], t: f32) -> f32 {
    a[0] + (a[1] - a[0]) * t
}

/// The region box getters' speed weight (F-81783 length, F-68633 width, F-97927 the centre's offset): mph x 0.016666668
/// (0x3C888889, a multiply), clamped by the pair of fsel (below 0 or NaN: 0; 1 or more: 1).
pub(crate) fn box_t(mph: f32) -> f32 {
    let x = mph * f32::from_bits(0x3C88_8889);
    let a = if x >= 0.0 { x } else { 0.0 };
    if a - 1.0 >= 0.0 { 1.0 } else { a }
}

/// F-15989: the distance from `p` to a road's rectangle (centre `c`, unit direction `d`, `len` along it, `width`
/// across) - the four edge midpoints c +- d len/2, c +- (-d.z, 0, d.x) width/2 by the vector unit's multiply then add,
/// each edge's plane offset n . m by the scalar fmuls / fmadds (y, z, x for d and -d; x, z, y for the sides), the
/// point's overshoot past each (vmsum3fp n . p - offset), the positive ones squared and summed (side, d, other side,
/// -d), the root.
pub(crate) fn road_box_dist(p: Vec3, c: Vec3, d: Vec3, len: f32, width: f32) -> f32 {
    let fma = |a: f32, b: f32, c: f32| (a as f64).mul_add(b as f64, c as f64) as f32;
    let (hw, hl) = (width * 0.5, len * 0.5);
    let side = Vec3::new(-d.z, 0.0, d.x);
    let nside = Vec3::new(d.z, 0.0, -d.x);
    let nd = Vec3::new(vs(0.0, d.x), vs(0.0, d.y), vs(0.0, d.z));
    let madd = |a: Vec3, k: f32, b: Vec3| Vec3::new(va(vm(a.x, k), b.x), va(vm(a.y, k), b.y), va(vm(a.z, k), b.z));
    let dot = |a: Vec3, b: Vec3| va(va(0.0, vm(a.z, b.z)), va(vm(a.y, b.y), vm(a.x, b.x)));
    let (m_d, m_nd, m_s, m_ns) = (madd(d, hl, c), madd(nd, hl, c), madd(side, hw, c), madd(nside, hw, c));
    let o_s = fma(m_s.y, 0.0, fma(m_s.z, d.x, m_s.x * -d.z));
    let o_d = fma(m_d.x, d.x, fma(m_d.z, d.z, m_d.y * d.y));
    let o_ns = fma(m_ns.y, 0.0, fma(m_ns.z, -d.x, m_ns.x * d.z));
    let o_nd = fma(m_nd.x, nd.x, fma(m_nd.z, nd.z, m_nd.y * nd.y));
    let mut s2 = 0.0f32;
    for (n, o) in [(side, o_s), (d, o_d), (nside, o_ns), (nd, o_nd)] {
        let e = dot(n, p) - o;
        if e > 0.0 {
            s2 = fma(e, e, s2);
        }
    }
    (s2 as f64).sqrt() as f32
}

/// The spawn box's centre (F-97927): focus + view x offset, the vector unit's multiply then add (not fused).
pub(crate) fn box_centre(focus: glam::Vec2, view: glam::Vec2, off: f32) -> glam::Vec2 {
    view * off + focus
}

/// Those getters' value of a pair at `t`: fma(t, b - a, a), fused.
pub(crate) fn box_lerp(p: [f32; 2], t: f32) -> f32 {
    crate::physics::exact::fma(t, p[1] - p[0], p[0])
}

/// Does the segment (x1, z1) - (x2, z2) meet the box +-hx, +-hz (F-43122, the original's parametric
/// clip: x then z, entry / exit parameters from -1 / 2, one flag set when an entry parameter is taken)?
/// Written with the original's branch order and tests, so that unordered values (NaN) go the way
/// the compares send them there; true when the clip leaves a segment that entered the box.
fn segment_meets_box(x1: f32, z1: f32, x2: f32, z2: f32, hx: f32, hz: f32) -> bool {
    let (mut tmin, mut tmax, mut entered) = (-1.0f32, 2.0f32, false);
    let dx = x2 - x1;
    let mut x_done = false;
    if dx == 0.0 {
        if x1.abs() > hx {
            return false;
        }
        x_done = true;
    }
    if !x_done {
        let inv = 1.0 / dx;
        let a = (hx - x1) * inv;
        if x1 < x2 {
            if a < 0.0 {
                return false;
            }
            if !(a > 1.0) {
                tmax = a;
            }
            let b = (-hx - x1) * inv;
            if b > 1.0 {
                return false;
            }
            if !(b < 0.0) {
                entered = true;
                tmin = b;
            }
        } else {
            if a > 1.0 {
                return false;
            }
            if !(a < 0.0) {
                entered = true;
                tmin = a;
            }
            let b = (-hx - x1) * inv;
            if b < 0.0 {
                return false;
            }
            if !(b > 1.0) {
                tmax = b;
            }
        }
        if tmin > tmax {
            return false;
        }
    }
    let dz = z2 - z1;
    if dz == 0.0 {
        if z1.abs() > hz {
            return false;
        }
    } else {
        let inv = 1.0 / dz;
        let a = (hz - z1) * inv;
        if z1 < z2 {
            if a < 0.0 {
                return false;
            }
            if !(a > 1.0) && a < tmax {
                tmax = a;
            }
            let b = (-hz - z1) * inv;
            if b > 1.0 {
                return false;
            }
            if !(b < 0.0) && b > tmin {
                entered = true;
                tmin = b;
            }
        } else {
            if a > 1.0 {
                return false;
            }
            if !(a < 0.0) && a > tmin {
                entered = true;
                tmin = a;
            }
            let b = (-hz - z1) * inv;
            if b < 0.0 {
                return false;
            }
            if !(b > 1.0) && b < tmax {
                tmax = b;
            }
        }
    }
    if tmin > tmax {
        return false;
    }
    entered
}

/// Do two oriented rectangles (centre, unit x axis, half sizes along it and across) meet (F-71037, the
/// original's test: each rectangle's corners in the other's frame, then the first one's four edges
/// clipped against the second, F-43122; the corner products are fused multiply-adds as the original's)?
pub(super) fn rects_meet(c1: glam::Vec2, u1: glam::Vec2, h1: glam::Vec2, c2: glam::Vec2, u2: glam::Vec2, h2: glam::Vec2) -> bool {
    let d = c2 - c1;
    let (u1x, u1z, u2x, u2z) = (u1.x, u1.y, u2.x, u2.y);
    let outside = |x: f32, z: f32, hx: f32, hz: f32| x > hx || x < -hx || z > hz || z < -hz;
    // the second rectangle's corners in the first's frame
    let c = u2z.mul_add(u1z, u2x * u1x);
    let sn = u2z.mul_add(u1x, -(u2x * u1z));
    let a = u1x.mul_add(d.x, u1z * d.y);
    let b = u1x.mul_add(d.y, -(u1z * d.x));
    let (p, q, r, t) = (h2.x * c, sn * h2.y, sn * h2.x, h2.y * c);
    let (ap, am, aq) = (a + p, a - p, a + q);
    let (br, bm) = (b + r, b - r);
    let cs = [((ap) - q, br + t), (aq + p, br - t), (am + q, bm - t), (am - q, bm + t)];
    if cs.iter().any(|&(x, z)| !outside(x, z, h1.x, h1.y)) {
        return true;
    }
    // the first rectangle's corners in the second's frame
    let (ndx, ndz) = (0.0 - d.x, 0.0 - d.y);
    let c2f = u1z.mul_add(u2z, u2x * u1x);
    let s2 = u1z.mul_add(u2x, -(u2z * u1x));
    let a2 = u2z.mul_add(ndz, u2x * ndx);
    let b2 = u2x.mul_add(ndz, -(u2z * ndx));
    let (e1, e2, e3, e4) = (h1.y * c2f, h1.x * c2f, h1.y * s2, h1.x * s2);
    let (am2, ap2, aq2) = (a2 - e2, a2 + e2, a2 + e3);
    let (bp2, bm2) = (b2 + e4, b2 - e4);
    let ds = [(ap2 - e3, bp2 + e1), (aq2 + e2, bp2 - e1), (am2 + e3, bm2 - e1), (am2 - e3, bm2 + e1)];
    if ds.iter().any(|&(x, z)| !outside(x, z, h2.x, h2.y)) {
        return true;
    }
    for (i, j) in [(3, 0), (0, 1), (1, 2), (2, 3)] {
        if segment_meets_box(ds[i].0, ds[i].1, ds[j].0, ds[j].1, h2.x, h2.y) {
            return true;
        }
    }
    false
}

/// A region's occluder box (road / junction vfunc 2, F-76320): x / z the extremes of the four edge midpoints (not the
/// corners), y the region's range raised 2 m at the top.
fn occluder_box(mid: Vec3, dir: Vec3, side: Vec3, len: f32, width: f32, y_range: [f32; 2]) -> (Vec3, Vec3) {
    let (d, sd) = (dir * (len * 0.5), side * (width * 0.5));
    let mids = [mid + d, mid - d, mid + sd, mid - sd];
    let (mut bmin, mut bmax) = mids.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
    bmin.y = y_range[0];
    bmax.y = y_range[1] + 2.0;
    (bmin, bmax)
}

impl Spawner {
    pub fn new(nav: &NavSys, root: &std::path::Path) -> Spawner {
        let mut roads = Vec::with_capacity(nav.zones.len());
        for z in nav.zones.iter() {
            let mut lanes: Vec<(usize, f32)> = Vec::new();
            let mut seen: hashbrown::HashMap<usize, usize> = hashbrown::HashMap::new();
            for &k in &z.links {
                let lane = nav.links[k].lane;
                let e = *seen.entry(lane).or_insert_with(|| {
                    lanes.push((k, 0.0));
                    lanes.len() - 1
                });
                lanes[e].1 += nav.links[k].len;
                if k < lanes[e].0 {
                    lanes[e].0 = k;
                }
            }
            let lane_len = Road::open_len(nav, roads.len(), &hashbrown::HashSet::new());
            let (bmin, bmax) = occluder_box(z.mid, z.dir, z.side, z.rect_len, z.width, z.y_range);
            let rect = RegionRect { index: roads.len(), rect_c: z.rect_c, width: z.width, len: z.rect_len, y_range: z.y_range, mid: z.mid, dir: z.dir, side: z.side, no_box: z.no_box };
            roads.push(Road { mid: z.mid, dir: z.dir, speed: z.speed_limit_mps(), density: z.density, lanes, lane_len, bmin, bmax, rect });
        }
        let ints = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66"))
            .map(|r| mcla_formats::rage::navsys::parse_intersections(&r))
            .unwrap_or_default();
        let int_rects = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66"))
            .map(|r| mcla_formats::rage::navsys::parse_intersection_rects(&r))
            .unwrap_or_default();
        let int_of: hashbrown::HashMap<usize, usize> = int_rects.iter().enumerate().map(|(k, r)| (r.index, k)).collect();
        let int_box: Vec<(Vec3, Vec3)> = int_rects.iter().map(|r| occluder_box(r.mid, r.dir, r.side, r.len, r.width, r.y_range)).collect();
        let mut lane_links: Vec<Vec<usize>> = Vec::new();
        for (k, l) in nav.links.iter().enumerate() {
            if lane_links.len() <= l.lane {
                lane_links.resize(l.lane + 1, Vec::new());
            }
            lane_links[l.lane].push(k);
        }
        for v in lane_links.iter_mut() {
            v.sort_by(|a, b| nav.links[*a].t[0].total_cmp(&nav.links[*b].t[0]));
        }
        let n = roads.len();
        Spawner { roads, ints, tune: Tune::load(root), data_dir: root.to_string_lossy().into_owned(), list: Vec::new(), int_weight: 0.0, listed: vec![false; n], target: 0.0, current: 0, frame: 0, step: 0.0, script: Default::default(), why: [0; 8], player_road: None, perp_next: false, order: Vec::new(), int_of, int_box, region_box: (glam::Vec2::ZERO, glam::Vec2::NEG_Y, 0.0, 0.0), cutscene: false, frontend: None, int_rects, fe_ints: Vec::new(), fe_pts: FrontEndPoints::default(), lane_links }
    }

    /// The road a link belongs to (F-86691): its lane's owner when that is a road (flag
    /// 2), else the owner's parent when that is one (a road's connector lanes); a junction's
    /// lanes have none.
    pub fn road_of(&self, nav: &NavSys, link: usize) -> Option<u32> {
        let l = nav.links.get(link)?;
        if l.owner_flags & 2 != 0 {
            l.zone.map(|z| z as u32)
        } else if l.parent_flags & 2 != 0 {
            l.parent_zone.map(|z| z as u32)
        } else {
            None
        }
    }

    /// The density value of a road: its index (F-85897, [`density_index`]), then in free roam
    /// with the player at 30 mph or more (mode byte +0x64250; the mode's meaning is a guess)
    /// the indices map 0 -> 0, 1-2 -> 0.55, 3-5 -> 1.25, 6 -> 0.10.
    fn value(&self, r: u32, free_fast: bool) -> f32 {
        let script = self.script.get(&r).copied().unwrap_or(-1);
        let idx = density_index(-1, script, self.cutscene, self.frontend.map(|f| f.index), self.roads[r as usize].density).clamp(0, 6) as usize;
        if free_fast {
            return match idx {
                0 => 0.0,
                1 | 2 => 0.55,
                3..=5 => 1.25,
                _ => 0.10,
            };
        }
        self.tune.values[idx]
    }
}

/// An ambient object's kind byte (F-30914, through F-93575): 0x80 the type's byte +108 bit 0, 0x20 a parking spot
/// (+44 not 0), 0x10 bit 0 of the type's vfunc 28 (a person's type), 0x40 without a spot when the low byte of the
/// type's vfunc 24 is not 0 (a car's type). (The low nibble is what the byte held before; the messages never test it.)
pub fn ambient_kind(flag108: u8, spot: u32, v24: u32, v28: u32) -> u8 {
    let mut k = (flag108 & 1) << 7 | ((v28 & 1) as u8) << 4;
    if spot != 0 {
        k |= 0x20;
    } else if v24 & 0xFF != 0 {
        k |= 0x40;
    }
    k
}

/// Does removal message `msg` take an object of kind `kind` (F-99126: any of the bits 0x80, 0x40, 0x20, 0x10 set in both)?
pub fn removal_hits(kind: u8, msg: u8) -> bool {
    kind & msg & 0xF0 != 0
}

/// The message the enable natives send when their byte is set to 0 (F-26788 traffic 0x60, F-74501 people 0x10,
/// F-54264 parked cars 0x20; the low nibble is a stack byte's, never tested): `arg`'s low byte 0, else none. (Each
/// first stores the byte - 0x8282A7C8 + native -, and does nothing at all while no ambient manager exists.)
pub fn enable_message(native: usize, arg: u32) -> Option<u8> {
    (arg & 0xFF == 0).then(|| [0x60, 0x10, 0x20][native])
}

/// A road's density index (F-85897): the global index when 0 or more (0x8282E930, written only by
/// the native RnRoadAmbient_SetGlobalRoadDensity F-79936, which no script calls - the port passes -1);
/// else the road's script index (+164, set by AmbientMgr_SetRoadDensity and the race scripts' road
/// density event, -1 none); else 5 in the cutscene mode (byte 0x829095FD); else the front-end
/// mode's index (byte 0x829095FE, index 0x8282A7D8); else the road's own (+152, from the data)
/// unless -1; else 3 (0x827DC4CC).
pub fn density_index(global: i32, script: i32, cutscene: bool, frontend: Option<i32>, own: i32) -> i32 {
    if global >= 0 {
        global
    } else if script != -1 {
        script
    } else if cutscene {
        5
    } else if let Some(n) = frontend {
        n
    } else if own != -1 {
        own
    } else {
        3
    }
}

/// Is a point seen from the focus (F-95989 / F-92805), from the spawn radii (50 / 250 m), one viewer, 1.5 m up for the
/// occluder (`SeenFrom::sees`).
pub(super) fn seen(t: &Tune, focus: Vec3, view: Vec3, mph: f32, p: Vec3, occ: Option<&crate::occluder::Selected>) -> bool {
    let from = SeenFrom { inner: t.spawn_inner, outer: t.spawn_outer, wide: 0.0, fov_mph: t.fov_mph, fov_deg: t.fov_deg, focus, view, mph };
    from.sees(p, 1.5, false, occ.map(|o| move |c: Vec3| if o.hides_sphere(c, 2.0) { OCC_HIDDEN } else { 0 }))
}

/// the occluder query's answer bit for a sphere wholly behind an occluder (F-87100's 0x04000000; F-95989 also takes
/// 0x10000000 as hidden)
const OCC_HIDDEN: u32 = 0x0400_0000;

/// What F-95989 reads off the ambient manager: the spawn radii (+4, +8), the flag's radius (+72), the field of view's
/// speeds (+108, +112) and degrees at them (+116, +120), the focus (+192), the view's x and z (+224, +232), the speed in mph
/// (+968).
#[derive(Clone, Copy)]
pub(crate) struct SeenFrom {
    pub inner: f32,
    pub outer: f32,
    pub wide: f32,
    pub fov_mph: [f32; 2],
    pub fov_deg: [f32; 2],
    pub focus: Vec3,
    pub view: Vec3,
    pub mph: f32,
}

impl SeenFrom {
    /// F-95989, as the original computes it. The flat distance squared d2 = dz dz + dx dx (fused). Without `flag`: nearer
    /// than the inner radius (or NaN) seen, beyond the outer one not; with it, beyond `wide` (or NaN) not seen, else the
    /// cone. The cone: the flat direction normalised as the vector unit does it (railroute::vnorm), c = view.z n.z +
    /// view.x n.x (fused), the angle acos(|c|) clamped by fsel (aiv::acos32); the speed's share t = (mph - lo) / (hi - lo),
    /// by fsel NaN or below 0 -> 0, from 1 -> 1; the field of view fov = t (deg_hi - deg_lo) + deg_lo (fused) times
    /// pi/180; an angle at or above it is not seen (a NaN angle is). Then, when the occluder test is on (`occ`, the
    /// manager's byte +2121), the query on the point `h` up (sphere r 2): bit 0x04000000 or 0x10000000 hides it.
    pub(crate) fn sees(&self, p: Vec3, h: f32, flag: bool, occ: Option<impl FnOnce(Vec3) -> u32>) -> bool {
        let (dx, dz) = (self.focus.x - p.x, self.focus.z - p.z);
        let d2 = dz.mul_add(dz, dx * dx);
        if flag {
            if !(d2 <= self.wide * self.wide) {
                return false;
            }
        } else {
            if !(d2 >= self.inner * self.inner) {
                return true;
            }
            if d2 > self.outer * self.outer {
                return false;
            }
        }
        let n = crate::railroute::vnorm(Vec3::new(p.x - self.focus.x, 0.0, p.z - self.focus.z));
        let c = self.view.z.mul_add(n.z, self.view.x * n.x);
        let a = crate::aiv::acos32(c.abs());
        let t = (self.mph - self.fov_mph[0]) / (self.fov_mph[1] - self.fov_mph[0]);
        let t0 = if t >= 0.0 { t } else { 0.0 };
        let k = if t0 - 1.0 >= 0.0 { 1.0 } else { t0 };
        let fov = k.mul_add(self.fov_deg[1] - self.fov_deg[0], self.fov_deg[0]) * f32::from_bits(0x3C8E_FA35);
        if a >= fov {
            return false;
        }
        match occ {
            None => true,
            Some(q) => {
                let b = q(Vec3::new(p.x, p.y + h, p.z));
                b & 0x0400_0000 == 0 && b & 0x1000_0000 == 0
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// The ambient manager's front-end mode (F-95688: byte 0x829095FE, set by the press-start
// script's AmbientMgr_SetFrontEnd(1, a, b, c, d, n), F-72572, which also writes S+8312 = a,
// S+8316 = b, S+8304 = c, S+8308 = d and 0x8282A7D8 = n). No player exists on the start screen:
// F-96156 runs the manager once with the camera (F-83397: focus its eye, view its
// forward made flat, speed 0, no player's road).

/// The world's frames run before the start screen's first frame is seen: the press-start script sets
/// the front-end mode (0x532), then waits for the city (City_DoneStreaming(250) polled every 10 ms)
/// and for Game_GetLoadingDone behind the loading logo; the game (the ambient manager and the cars)
/// runs meanwhile. The wait is the streaming's, not a constant of the code: in the recording
/// `work/title-cams/rec4` the boot movies end at f0184 and the first frame of the shot is f0224
/// (0.1 s a frame), and that first frame shows the freeway already full, the cars spread some 200 m
/// from the lane points they were made at: 4.0 s, i.e. 120 frames at 30 Hz (approximate - the
/// script's start inside that window was not measured).
pub const FRONT_END_PREROLL: u32 = 120;

/// The front-end mode this frame.
#[derive(Clone, Copy, Debug)]
pub struct FrontEnd {
    pub eye: Vec3,
    /// the camera's forward (unit)
    pub fwd: Vec3,
    /// tan(fov / 2) up and across (viewport +712, +716)
    pub tan: [f32; 2],
    /// S+8312 (widens the frustum across, radians), S+8316 (up), S+8304 (near), S+8308 (far)
    pub amb: [f32; 4],
    /// the roads' density index (0x8282A7D8, F-85897)
    pub index: i32,
}

/// The front-end lane points (ambient S+48, F-56167): the planes and region count they were
/// made for (the list is made again only when either changed), and at most 256 points - where a
/// lane goes into the frustum across one plane, inside the other five: the point, the lane, its
/// phase and the rail link and metres along it (none: a lane off the rail graph).
#[derive(Default, Clone)]
pub struct FrontEndPoints {
    planes: [glam::Vec4; 6],
    regions: Option<usize>,
    pub pts: Vec<(Vec3, usize, f32, Option<(usize, f32)>)>,
}

/// The front-end frustum (F-91754) from the port's front-end camera: the viewport's matrix as the port builds it
/// (z = -forward, x = right, y = up, the eye; not the original's own matrix - approximate input), `frontend_planes_of`.
pub fn frontend_planes(fe: &FrontEnd) -> [glam::Vec4; 6] {
    let z = -fe.fwd;
    let x = Vec3::Y.cross(z).normalize_or(Vec3::X);
    let y = z.cross(x);
    frontend_planes_of([x, y, z, fe.eye], fe.tan, fe.amb)
}

/// F-91754 as the original computes it: six planes, outward normal and w = normal . point (a point is outside when
/// normal . p - w > 0): near (at S+8304), left, right, far (at S+8308), top, bottom. `rows` the viewport's matrix rows
/// (+64 x, +80 y, +96 z, +112 the place), `tan` its +712 / +716, `amb` S+8312 / +8316 / +8304 / +8308. h = S+8312 +
/// atan(+716), v = S+8316 + atan(+712) (the guest atan F-79102, single); the local normals (0, 0, 1), (-cos h, 0, sin h),
/// (cos h, 0, sin h), (0, 0, -1), (0, cos v, sin v), (0, -cos v, sin v) (guest cos / sin, single), the points (0, 0, -near),
/// 0, 0, (0, 0, -far), 0, 0; each turned by the rows as the vector unit sums (vmsum3fp: (0 + z r2) + (y r1 + x r0); the
/// point vmsum4fp with w 1: (r3 + z r2) + (y r1 + x r0)), w = fma(p.x, n.x, fma(p.z, n.z, p.y n.y)).
pub fn frontend_planes_of(rows: [Vec3; 4], tan: [f32; 2], amb: [f32; 4]) -> [glam::Vec4; 6] {
    use mcla_formats::guest_math::{guest_atan, guest_cos, guest_sin};
    let [a, b, near, far] = amb;
    let h = a + guest_atan(tan[1] as f64) as f32;
    let v = b + guest_atan(tan[0] as f64) as f32;
    let (ch, sh) = (guest_cos(h as f64) as f32, guest_sin(h as f64) as f32);
    let (cv, sv) = (guest_cos(v as f64) as f32, guest_sin(v as f64) as f32);
    let local = [
        (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, -near)),
        (Vec3::new(-ch, 0.0, sh), Vec3::ZERO),
        (Vec3::new(ch, 0.0, sh), Vec3::ZERO),
        (Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 0.0, -far)),
        (Vec3::new(0.0, cv, sv), Vec3::ZERO),
        (Vec3::new(0.0, -cv, sv), Vec3::ZERO),
    ];
    let [r0, r1, r2, r3] = rows;
    let turn = |v: Vec3, base: f32, j: usize| (base + v.z * r2[j]) + (v.y * r1[j] + v.x * r0[j]);
    local.map(|(n, p)| {
        let nw = Vec3::new(turn(n, 0.0, 0), turn(n, 0.0, 1), turn(n, 0.0, 2));
        let pw = Vec3::new(turn(p, r3.x, 0), turn(p, r3.y, 1), turn(p, r3.z, 2));
        nw.extend(pw.x.mul_add(nw.x, pw.z.mul_add(nw.z, pw.y * nw.y)))
    })
}

fn plane_out(pl: glam::Vec4, p: Vec3) -> bool {
    pl.truncate().dot(p) - pl.w > 0.0
}

/// Where a lane goes into a plane's inside (F-78433): the lane at t = k x 0.01010101 (k 0 ..
/// 99, lane vfunc 7); a step from outside to inside gives the middle of the step and its phase;
/// at most `max`.
pub fn lane_entries(c: &LaneCurve, pl: glam::Vec4, max: usize) -> Vec<(Vec3, f32)> {
    let mut out = Vec::new();
    let mut last: Option<(bool, Vec3, f32)> = None;
    for k in 0..100 {
        let t = k as f32 * 0.010_101_01;
        let p = crate::railroute::point(c, t);
        let o = plane_out(pl, p);
        if let Some((true, q, tq)) = last {
            if !o {
                out.push(((p + q) * 0.5, (t + tq) * 0.5));
                if out.len() >= max {
                    break;
                }
            }
        }
        last = Some((o, p, t));
    }
    out
}

/// F-46547 (the regions' vfunc 3): does the circle (x / z) meet the region's rectangle -
/// first the length x 0.7071 + r round +34 / +38, then the squared distance from the centre,
/// turned by -atan2(dir.x, dir.z), to the rectangle (+3c across, +40 along) at most r^2.
pub fn region_meets_circle(r: &RegionRect, c: Vec3, rad: f32) -> bool {
    let (dx, dz) = (r.rect_c[0] - c.x, r.rect_c[1] - c.z);
    let reach = r.len * 0.707_106_77 + rad;
    if dx * dx + dz * dz > reach * reach {
        return false;
    }
    let ang = -r.dir.x.atan2(r.dir.z);
    let (s, co) = ang.sin_cos();
    let (px, pz) = (c.x - r.rect_c[0], c.z - r.rect_c[1]);
    let x = px * co + pz * s;
    let y = pz * co - px * s;
    let (hw, hl) = (r.width * 0.5, r.len * 0.5);
    let mut d = 0.0;
    if x < -hw {
        d = (x + hw) * (x + hw);
    } else if x > hw {
        d = (x - hw) * (x - hw);
    }
    if y < -hl {
        d += (y + hl) * (y + hl);
    } else if y > hl {
        d += (y - hl) * (y - hl);
    }
    d <= rad * rad
}

/// The 8 corners of a region's box (F-49549's front-end branch): centre +- direction x
/// length / 2 +- across x width / 2, at the height range's two ends.
fn region_corners(r: &RegionRect) -> [Vec3; 8] {
    let (d, s) = (r.dir * (r.len * 0.5), r.side * (r.width * 0.5));
    let xz = [r.mid + d + s, r.mid + d - s, r.mid - d - s, r.mid - d + s];
    let mut out = [Vec3::ZERO; 8];
    for (i, p) in xz.iter().enumerate() {
        out[i] = Vec3::new(p.x, r.y_range[0], p.z);
        out[i + 4] = Vec3::new(p.x, r.y_range[1], p.z);
    }
    out
}

/// Are all the points outside one of the planes?
fn all_out_of_one(planes: &[glam::Vec4; 6], pts: &[Vec3]) -> bool {
    planes.iter().any(|pl| pts.iter().all(|p| plane_out(*pl, *p)))
}

impl Spawner {
    /// F-49549's front-end branch: the regions (roads and junctions) whose rectangle meets
    /// the sphere ahead of the camera - D = S+8308, v = the flat forward x D, L = |v|, centre eye +
    /// v / 2, radius sqrt((L / 2)^2 + (L / tan across)^2); at most 200 (F-30720) - kept,
    /// at most 100, unless all 8 corners of the region's box are outside one plane of the
    /// front-end frustum; the target the sum of their weights (no density scale), 0 .. 1 -> 1.
    /// (The cutscene's own neighbours, F-63162, are not added in this mode.)
    fn region_list_frontend(&mut self, fe: &FrontEnd) {
        let t = self.tune;
        // (the parked lists' own box, F-97927, at the mode's zero speed and M+324 = 0)
        let vx = glam::Vec2::new(fe.fwd.x, fe.fwd.z).normalize_or(glam::Vec2::NEG_Y);
        let fx = glam::Vec2::new(fe.eye.x, fe.eye.z);
        self.region_box = (box_centre(fx, vx, box_lerp(t.box_off, box_t(0.0))), vx, box_lerp(t.box_len, box_t(0.0)), box_lerp(t.box_width, box_t(0.0)));
        let d = fe.amb[3];
        let v = Vec3::new(fe.fwd.x * d, 0.0, fe.fwd.z * d);
        let l = (v.x * v.x + v.z * v.z).sqrt();
        let centre = fe.eye + v.normalize_or(Vec3::NEG_Z) * (l * 0.5);
        let rad = ((l * 0.5) * (l * 0.5) + (l / fe.tan[1]) * (l / fe.tan[1])).sqrt();
        let planes = frontend_planes(fe);
        let speed_factor = lerp(t.density_at, ((0.0 - t.density_mph[0]) / (t.density_mph[1] - t.density_mph[0])).clamp(0.0, 1.0));
        self.list.clear();
        self.listed.iter_mut().for_each(|x| *x = false);
        self.fe_ints.clear();
        self.order.clear();
        self.int_weight = 0.0;
        let mut target = 0.0;
        let (mut found, mut kept) = (0usize, 0usize);
        // (the regions' query order is the city's region tree's, not decoded: roads, then
        // junctions - both caps are far from reached at the shots, see the doc)
        let n_roads = self.roads.len();
        for k in 0..n_roads + self.int_rects.len() {
            let r = if k < n_roads { self.roads[k].rect } else { self.int_rects[k - n_roads] };
            if !region_meets_circle(&r, centre, rad) {
                continue;
            }
            found += 1;
            if found > 200 || kept == 100 {
                break;
            }
            if all_out_of_one(&planes, &region_corners(&r)) {
                continue;
            }
            kept += 1;
            if k < n_roads {
                let w = self.roads[k].lane_len * 0.01 * self.value(k as u32, false) * speed_factor;
                self.list.push((k as u32, w));
                self.order.push((NavNode::Road(k), w));
                self.listed[k] = true;
                target += w;
            } else {
                let j = k - n_roads;
                let iw = self.ints.get(j).map_or(0.0, |x| x.1) * 0.01 * t.values[1] * speed_factor;
                self.order.push((NavNode::Junction(self.int_rects[j].index), iw));
                self.fe_ints.push(j);
                self.int_weight += iw;
                target += iw;
            }
        }
        if target > 0.0 && target < 1.0 {
            target = 1.0;
        }
        self.target = target;
        if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() && self.frame % 30 == 1 {
            log::info!("front-end regions: {} in the sphere (r {:.0}), {} kept ({} roads, {} junctions), target {:.2}", found, rad, kept, self.list.len(), self.fe_ints.len(), target);
        }
    }

    /// F-56167 (from F-96156 every frame in the front-end mode, before the managers
    /// run): the front-end planes and manager 0's region list; when either has changed (the
    /// planes compared exactly, the list by its count only) the points are made again - for every
    /// region whose lanes' box (lane +10 / +20) is not wholly outside one plane, every lane's
    /// entries across each plane (F-78433, 64 at most) that are inside the other five; at
    /// most 256.
    fn frontend_points(&mut self, nav: &NavSys, fe: &FrontEnd) {
        let planes = frontend_planes(fe);
        let count = self.list.len() + self.fe_ints.len();
        if self.fe_pts.regions == Some(count) && self.fe_pts.planes == planes {
            return;
        }
        self.fe_pts.planes = planes;
        self.fe_pts.regions = Some(count);
        self.fe_pts.pts.clear();
        let regions: Vec<Vec<usize>> = self
            .list
            .iter()
            .map(|&(ri, _)| self.roads[ri as usize].lanes.iter().map(|&(k, _)| nav.links[k].lane).collect())
            .chain(self.fe_ints.iter().map(|&j| self.int_rects.get(j).and_then(|r| nav.junction_lanes.get(r.index)).cloned().unwrap_or_default()))
            .collect();
        'regions: for lanes in regions {
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for &ln in &lanes {
                if let Some((a, b)) = nav.lane_box(ln) {
                    lo = lo.min(a);
                    hi = hi.max(b);
                }
            }
            let corners = [
                Vec3::new(lo.x, lo.y, lo.z), Vec3::new(lo.x, lo.y, hi.z), Vec3::new(lo.x, hi.y, lo.z), Vec3::new(lo.x, hi.y, hi.z),
                Vec3::new(hi.x, lo.y, lo.z), Vec3::new(hi.x, lo.y, hi.z), Vec3::new(hi.x, hi.y, lo.z), Vec3::new(hi.x, hi.y, hi.z),
            ];
            if all_out_of_one(&planes, &corners) {
                continue;
            }
            for &ln in &lanes {
                let Some(curve) = nav.curves.get(ln) else { continue };
                for i in 0..6 {
                    for (p, ph) in lane_entries(curve, planes[i], 64) {
                        if (0..6).any(|j| j != i && plane_out(planes[j], p)) {
                            continue;
                        }
                        let at = self.lane_links.get(ln).and_then(|ks| {
                            let k = *ks.iter().find(|&&k| ph <= nav.links[k].t[1]).or(ks.last())?;
                            Some((k, nav.links[k].s_at(ph)))
                        });
                        self.fe_pts.pts.push((p, ln, ph, at));
                        if self.fe_pts.pts.len() == 256 {
                            break 'regions;
                        }
                    }
                }
            }
        }
        if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
            log::info!("front-end lane points: {} from {} regions", self.fe_pts.pts.len(), count);
        }
    }
}

impl Vehicles {
    /// AmbientMgr_Reset (F-11140 -> F-91399 -> F-99126 with the message 0x40): every ambient car
    /// of kind 0x40 - a car type (type vt+24 F-49619 = 1; a person's type returns 0) without a parking
    /// spot (car +44 = 0) - goes at once, no fade: riding traffic, trailers, a layout item's car (its item stays).
    /// The cars in spots (kind 0x20) and the walkers (0x10) stay. Per car (F-76814 / F-75521 /
    /// F-29339 with no time): +500 bit 0x80, a request that takes the car over (F-10793), its
    /// behaviour (F-20553), then the car's vfunc 11 with reason 11 (F-12958: the car taken out) and
    /// back to its group's free list (F-72573, the type's counts F-39893). The spawn requests of kind
    /// 0x40 still waiting for ground are ended too (F-42105: state 7) - the port keeps none at this point.
    pub fn ambient_reset(&mut self) {
        let n = self.ambient_remove(0x40);
        log::info!("ambient: reset - {} cars taken out, {} left", n, self.cars.len());
    }

    /// The ambient manager's removal message (F-99126): every ambient car whose kind byte (F-30914) shares a bit of
    /// 0xf0 with `msg` goes at once (the people, kind 0x10, are not the cars' and stay here). Returns how many went.
    pub fn ambient_remove(&mut self, msg: u8) -> usize {
        let before = self.cars.len();
        self.cars.retain(|c| {
            let ambient = !c.is_player && !c.cop && !c.racer && !c.pinned && (c.rail.is_some() || c.parked.is_some());
            // (a car type: vfunc 24 = 1, vfunc 28 = 0; its spot +44: a parking spot, not a layout item's)
            let spot = c.parked.is_some_and(|s| s != super::parked::ITEM_SPOT);
            !ambient || !removal_hits(ambient_kind(0, spot as u32, 1, 0), msg)
        });
        before - self.cars.len()
    }

    /// AmbientMgr_SetTrafficEnable (F-26788) and AmbientMgr_SetParkedCarsEnable (F-54264) switched off: their message
    /// ([`enable_message`]) removes the riding and parked cars (0x60) or the parked ones (0x20) at once; the
    /// spawners stop by the byte (the port: `traffic_target` 0, `parked_held`).
    pub fn ambient_disable(&mut self, native: usize) {
        if let Some(m) = enable_message(native, 0) {
            let n = self.ambient_remove(m);
            log::info!("ambient: enable byte {native} off - {n} cars taken out, {} left", self.cars.len());
        }
    }

    /// The roads' part of the railnet's reset (F-44338, which also resets the junctions: `Signals::release`):
    /// every road's script density index +164 back to -1 (F-46490); script requests not yet taken go too.
    pub fn reset_road_density(&mut self) {
        self.road_density.clear();
        self.road_density_new.clear();
        if let Some(sp) = self.spawner.as_mut() {
            sp.script.clear();
        }
    }

    /// AmbientMgr_SetRoadDensity (F-84942) from a script the port runs by hand: the road at `p` (F-86195)
    /// gets `index` as its density index, -1 gives it back (taken at the spawner's next run).
    pub fn set_road_density(&mut self, p: Vec3, index: i32) {
        self.road_density_new.push((p, index));
    }

    /// Is car `i` ambient traffic the spawner owns?
    /// The cars on each lane's follower list as the region count reads them (rail slot 21, F-90597): every follower
    /// counted (t0), the ones with a layout item's car (+40) apart (t2). A port car is a follower of its navsys lane
    /// while it rides it (state 1).
    fn lane_cars(&self, nav: &NavSys) -> RegionCounts {
        let mut m = RegionCounts::default();
        for c in &self.cars {
            let Some(r) = c.rail.as_ref() else { continue };
            // (a road's +160: the rails attached with it as their owner - a passing rail, a lane change rail -, +1 at
            // the attach F-70405, -1 at the release F-29352)
            let lc = r.ghost.as_ref().and_then(|g| g.lc.as_ref());
            for owner in [r.path.as_ref().and_then(|p| p.owner), lc.and_then(|p| p.owner)].into_iter().flatten() {
                *m.roads.entry(owner).or_default() += 1;
            }
            // (on a lane change rail the follower is off its lane's list)
            if r.state != 1 || lc.is_some() {
                continue;
            }
            let Some(l) = nav.links.get(r.link) else { continue };
            let e = m.lanes.entry(l.lane).or_default();
            e.0 += 1;
            if c.item.is_some() {
                e.1 += 1;
            }
        }
        m
    }

    /// A region's cars (its vfunc 10: road F-64556, junction F-31442, both over F-85484 - its lanes, then its child's
    /// subtree; a road adds its +160 to t0, the rails it owns): max(0, t0 - t2).
    fn region_cars(nav: &NavSys, counts: &RegionCounts, node: NavNode) -> i32 {
        let lanes = match node {
            NavNode::Road(r) => nav.records.road_lanes.get(r),
            NavNode::Junction(j) => nav.junction_lanes.get(j),
        };
        let (mut t0, t2) = lanes.map_or((0, 0), |ls| ls.iter().fold((0, 0), |a, l| counts.lanes.get(l).map_or(a, |x| (a.0 + x.0, a.1 + x.1))));
        if let NavNode::Road(r) = node {
            t0 += counts.roads.get(&r).copied().unwrap_or(0);
        }
        (t0 - t2).max(0)
    }

    /// M+1800 (F-49549, every branch): the listed regions' cars summed in the list's order.
    fn listed_cars(&self, sp: &Spawner, nav: &NavSys) -> u32 {
        let lc = self.lane_cars(nav);
        let mut cur = 0.0f32;
        for &(node, _) in &sp.order {
            cur += Self::region_cars(nav, &lc, node) as f32;
        }
        cur as u32
    }

    fn ambient_car(&self, i: usize) -> bool {
        let c = &self.cars[i];
        // (a trailer has no behaviour: neither counted nor unspawned on its own)
        !c.is_player && !c.cop && !c.racer && !c.pinned && c.parked.is_none() && c.rail.as_ref().is_some_and(|r| r.state != 0)
    }

    /// The road of a car: of the lane it rides (or rode last).
    fn car_road(&self, sp: &Spawner, nav: &NavSys, i: usize) -> Option<u32> {
        sp.road_of(nav, self.cars[i].rail.as_ref()?.link)
    }

    /// The player's lane (F-58331 for the racer's obstacle): the nearest within the
    /// chassis bound's half diagonal + 2.25 m. (The original keeps a list of lanes changed by
    /// one random check a frame; the nearest of them is taken here at once.)
    fn player_lane(nav: &NavSys, c: &super::CarInstance) -> Option<usize> {
        let b = c.model.params.body_half;
        let r = (b.x * b.x + b.z * b.z).sqrt() + 2.25;
        let p = c.state.pos;
        nav.links_near(p, r)
            .map(|k| (k, nav.links[k].point_at(nav.links[k].project(p)).distance(p)))
            .filter(|x| x.1 < r)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| x.0)
    }

    /// The original's traffic spawner and unspawner, run from `manage_traffic`.
    pub(super) fn spawn_traffic(&mut self, gpu: &crate::render::Gpu, world: &crate::world::World, center: Vec3, dt: f32) {
        let Some(nav) = world.nav.as_ref().map(rail_nav) else { return };
        if self.spawner.is_none() {
            let made = SPAWNER_AHEAD.lock().unwrap_or_else(|e| e.into_inner()).take().filter(|(r, _)| *r == world.assets.root);
            self.spawner = Some(made.map_or_else(|| Spawner::new(nav, &world.assets.root), |m| m.1));
        }
        let mut sp = self.spawner.take().unwrap();
        // (test hook MCLA_TRAFFIC_PLACE=<type>,x,y,z,heading[,steer[,colour row[,drawn y]]]: one parked car of that type as a
        // script's layout item - AddAmbientToLayout, no visibility test - heading in degrees as a parking
        // spot's (z row (sin h, 0, cos h), the car facing -z), its front wheels turned `steer` radians when
        // given; for side-by-side shots with a recording of the original)
        if let Ok(v) = std::env::var("MCLA_TRAFFIC_PLACE") {
            static PLACED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let f: Vec<&str> = v.split(',').collect();
            let num = |i: usize| f.get(i).and_then(|x| x.trim().parse::<f32>().ok());
            if let (Some(name), Some(x), Some(y), Some(z), Some(h)) = (f.first(), num(1), num(2), num(3), num(4)) {
                let held = PLACED.load(std::sync::atomic::Ordering::Relaxed);
                if held == 0 {
                    if let Some(item) = self.add_layout_item(name.trim(), Vec3::new(x, y, z), h.to_radians(), None) {
                        PLACED.store(item + 1, std::sync::atomic::Ordering::Relaxed);
                        log::info!("MCLA_TRAFFIC_PLACE: layout item {item} {name} at ({x}, {y}, {z}) heading {h}");
                    }
                } else if let Some(steer) = num(5) {
                    for c in self.cars.iter_mut().filter(|c| c.item == Some(held - 1)) {
                        c.park_steer = steer;
                        for wh in c.state.wheels.iter_mut().take(2) {
                            wh.steer = steer;
                        }
                        // (eighth field: a drawn place y, for a recording of a car with a rider, 0.025 lower - F-37033)
                        if let Some(dy) = num(7) {
                            c.state.pos.y = dy;
                        }
                    }
                }
            }
        }
        // trailers whose cab is gone
        self.cars.retain(|c| c.rail.as_ref().map_or(true, |r| r.state != 0 || r.partner.is_some()));
        // the ambient cars' fade (F-82632, every car of the ambient update): up by 2 a second from the set-up, down by 2 a
        // second once marked (car +500 bit 0x4, `removing` holds the fade then); at 0 the car is removed (F-51512 with
        // its reason)
        let mut gone = vec![false; self.cars.len()];
        for (k, c) in self.cars.iter_mut().enumerate() {
            if c.rail.is_none() && c.parked.is_none() {
                continue;
            }
            let (mut rising, mut falling) = (!c.amb_lamps.faded_in, c.removing.is_some());
            gone[k] = fade_step(&mut c.amb_lamps.fade, &mut rising, &mut falling, dt);
            c.amb_lamps.faded_in = !rising;
            c.removing = falling.then_some(c.amb_lamps.fade);
        }
        let mut k = 0;
        self.cars.retain(|_| {
            k += 1;
            !gone[k - 1]
        });
        // roads a script set the density of (AmbientMgr_SetRoadDensity F-84942, the race scripts'
        // road density event F-36201): the road at the point (F-86195) gets the index as its +164,
        // -1 giving it back; no road there, nothing
        for (p, d) in std::mem::take(&mut self.road_density_new) {
            if let Some(r) = nav.road_at(p) {
                sp.script.insert(r as u32, d);
            }
        }
        // the focus: the player's car, its heading and speed, its lane and road (M+320: kept
        // while its lane belongs to no road)
        let (view, mph, player_link) = match self.cars.iter().find(|c| c.is_player) {
            Some(c) => {
                let f = Vec3::new(c.state.forward().x, 0.0, c.state.forward().z).normalize_or(Vec3::NEG_Z);
                (f, c.state.speed() / MPH, Self::player_lane(nav, c))
            }
            None => (Vec3::NEG_Z, 0.0, None),
        };
        // (the cutscene mode, F-97032: the focus the camera's eye, the view its forward made
        // flat, no speed - the velocity it takes, [0x828608D0], is zero - and no player's road)
        let cut = self.amb_cutscene;
        let (center, view, mph, player_link) = match cut {
            Some((eye, f)) => (eye, Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z), 0.0, None),
            None => (center, view, mph, player_link),
        };
        sp.cutscene = cut.is_some();
        // (the front-end mode, F-95688: no player on the start screen - F-83397 takes the
        // camera as in the cutscene mode: its eye, its forward made flat, speed 0, no player's road)
        let fe = self.amb_frontend.filter(|_| cut.is_none()).map(|f| FrontEnd { eye: f.eye, fwd: f.dir.normalize_or(Vec3::NEG_Z), tan: self.amb_frontend_tan, amb: f.amb, index: f.index });
        let (center, view, mph, player_link) = match fe {
            Some(f) => (f.eye, Vec3::new(f.fwd.x, 0.0, f.fwd.z).normalize_or(Vec3::NEG_Z), 0.0, None),
            None => (center, view, mph, player_link),
        };
        if sp.frontend.is_none() != fe.is_none() {
            sp.fe_pts = FrontEndPoints::default();
        }
        sp.frontend = fe;
        // (the player's car wiped out - its sim's +592 bit 0x10, set by mcCarDamage F-69183 for
        // WipeoutDelay: F-43424 - no spawning or unspawning, the lists still made)
        let wiped = fe.is_none() && self.cars.iter().find(|c| c.is_player).is_some_and(|c| c.state.upset > 0.0 || c.state.veh.as_ref().is_some_and(|v| v.wipe.is_some()));
        {
            static WAS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if WAS.swap(wiped, std::sync::atomic::Ordering::Relaxed) != wiped {
                log::info!("ambient spawners {} (the player's wipeout)", if wiped { "paused" } else { "resumed" });
            }
        }
        if let Some(r) = player_link.and_then(|l| sp.road_of(nav, l)) {
            sp.player_road = Some(r);
        }
        let free_fast = !self.cars.iter().any(|c| c.pinned) && mph >= 30.0;
        let enabled = self.traffic_target > 0;
        sp.step = (sp.step + dt).min(4.0 / 30.0);
        while sp.step >= 1.0 / 30.0 {
            sp.step -= 1.0 / 30.0;
            sp.frame = sp.frame.wrapping_add(1);
            // (F-96156: the front-end lane points from manager 0's region list, every frame
            // before the manager runs)
            if let Some(f) = fe {
                sp.frontend_points(nav, &f);
            }
            if sp.frame % 2 == 1 {
                self.region_list(&mut sp, nav, center, view, mph, free_fast);
                // the parked cars' lists right after (F-51021, vehicles/parked.rs)
                self.parked_lists(&sp);
                if let Some(f) = fe.filter(|_| sp.frame % 30 == 1 && std::env::var("MCLA_TRAFFIC_TRACE").is_ok()) {
                    // (diagnostics) the riding cars inside the drawn frustum (5 m .. 10 km, no widening)
                    let pl = frontend_planes(&FrontEnd { amb: [0.0, 0.0, 5.0, 10000.0], ..f });
                    let n = self.cars.iter().filter(|c| c.rail.is_some() && c.parked.is_none() && !c.is_player && !pl.iter().any(|p| plane_out(*p, c.state.pos))).count();
                    log::info!("front-end cars in view: {} (riding {})", n, self.cars.iter().filter(|c| c.rail.is_some() && c.parked.is_none() && !c.is_player).count());
                }
                if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() && sp.frame % 30 == 1 {
                    log::info!("traffic regions {} target {:.2} current {} enabled {} mph {:.0} turned down (type, pool, junction, freeway, one way, room, seen, no ground) {:?}", sp.list.len(), sp.target, sp.current, enabled, mph, sp.why);
                }
            } else if !wiped {
                // F-93348 (traffic, people, then the parked cars) and F-75661 (the
                // unspawners, only while the region list has roads); none while wiped out
                // (the type weights' use of the types, F-68379 -> F-83932: parked.rs)
                self.ambient_use(&world.assets.root);
                if enabled {
                    self.spawn_frame(&mut sp, gpu, world, nav, center, view, mph, player_link);
                }
                let parked = self.parked_target > 0;
                // (AmbientMgr_SetParkedCarsEnable(0): no parked car is made - the new career's CarSelection)
                if parked && !self.parked_held {
                    self.parked_spawn(gpu, &sp, center, view, mph);
                }
                // (F-75661: the four unspawners only while the region list has roads, M +1792 != 0 - its byte
                // +2086 is 1 from the ctor F-10264, its only writer -: the crossing unspawner F-54093, the
                // traffic's F-65438, the people's F-39594 (peds.rs, `Peds::traffic_regions`), the parked cars' F-53207)
                if !sp.list.is_empty() {
                    if enabled {
                        self.crossing_frame(&sp, world, nav, center, view, mph);
                        self.unspawn_frame(&sp, nav, center, view, mph);
                    }
                    if parked {
                        self.parked_unspawn(&sp, center);
                    }
                }
            }
            // F-20685 after either half of F-47889's frame (the wipeout does not stop it): the race traffic clearing at
            // the tracker's junction record (none in the cutscene or front-end mode: no player record), on while a race
            // of type 0..4 is on (the -notrafficclearing switch is off; +2107 / +2108 are 0 / 1 from the ctor)
            let rec = if cut.is_none() && fe.is_none() { self.player_record } else { None };
            self.race_clearing.step(nav, self.race_event, true, &[rec, None]);
        }
        self.spawner = Some(sp);
    }

    /// F-49549: the roads in the box ahead of the focus, their weights and the target.
    #[allow(clippy::too_many_arguments)]
    fn region_list(&mut self, sp: &mut Spawner, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32, free_fast: bool) {
        if let Some(f) = sp.frontend {
            sp.region_list_frontend(&f);
            sp.current = self.listed_cars(sp, nav);
            return;
        }
        let t = sp.tune;
        let k = box_t(mph);
        let (fx, vx) = (glam::Vec2::new(focus.x, focus.z), glam::Vec2::new(view.x, view.z).normalize_or(glam::Vec2::NEG_Y));
        // the square box round the focus when the last pass's nearest listed road ran across the view (M+324)
        let player_road = sp.player_road;
        let perp = sp.perp_next;
        // (F-81783 / F-68633 / F-97927's offset: `box_t`, `box_lerp`)
        let (len, off, width) = if perp { (box_lerp(t.perp_len, k), box_lerp(t.perp_off, k), box_lerp(t.perp_width, k)) } else { (box_lerp(t.box_len, k), box_lerp(t.box_off, k), box_lerp(t.box_width, k)) };
        let bc = box_centre(fx, vx, off);
        let bh = glam::Vec2::new(len * 0.5, width * 0.5);
        sp.region_box = (bc, vx, len, width);
        let on_freeway = player_road.is_some_and(|r| sp.roads[r as usize].speed >= FREEWAY_SPEED);
        // hidden: behind an occluder of the spawner's view only (F-45295 with flags 2 - no
        // frustum test): the eye 1.75 m above the focus, occluders within 400 m
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let speed_factor = lerp(t.density_at, ((mph - t.density_mph[0]) / (t.density_mph[1] - t.density_mph[0])).clamp(0.0, 1.0));
        sp.list.clear();
        sp.order.clear();
        sp.listed.iter_mut().for_each(|x| *x = false);
        sp.int_weight = 0.0;
        let mut target = 0.0;
        let mut nearest = (f32::MAX, None);
        // the candidates: the roads and junctions within half the box's length of its centre, at most 200, in the
        // region tree's order (F-30720 -> F-21575, mask 6)
        for node in nav.regions_near(Vec3::new(bc.x, 0.0, bc.y), len * 0.5, 200) {
            let (ri, jk, rect, (bmin, bmax), fw) = match node {
                NavNode::Road(ri) => {
                    let Some(r) = sp.roads.get(ri) else { continue };
                    (Some(ri), 0, r.rect, (r.bmin, r.bmax), r.speed >= FREEWAY_SPEED)
                }
                NavNode::Junction(j) => {
                    let Some(&k) = sp.int_of.get(&j) else { continue };
                    (None, k, sp.int_rects[k], sp.int_box[k], false)
                }
            };
            let rc = glam::Vec2::new(rect.mid.x, rect.mid.z);
            let ru = glam::Vec2::new(rect.dir.x, rect.dir.z).normalize_or(glam::Vec2::X);
            let overlap = rects_meet(bc, vx, bh, rc, ru, glam::Vec2::new(rect.len * 0.5, rect.width * 0.5));
            let hidden = || !rect.no_box && occ.as_ref().is_some_and(|o| o.hides_aabb(bmin, bmax));
            let accept = if on_freeway && fw {
                true
            } else if !overlap {
                false
            } else if on_freeway {
                !hidden()
            } else {
                let near = rc.distance(fx) <= 200.0;
                let aligned = rect.dir.dot(view) > 0.6;
                if fw && !(near || aligned) {
                    false
                } else {
                    (near && aligned) || !hidden()
                }
            };
            if !accept {
                continue;
            }
            let Some(ri) = ri else {
                // a junction: its weight (vfunc 11, F-46871: (+1c x 0.01) x the density value 1 x the speed factor)
                let iw = sp.ints.get(jk).map_or(0.0, |x| x.1) * 0.01 * t.values[1] * speed_factor;
                sp.order.push((node, iw));
                sp.int_weight += iw;
                target += iw;
                continue;
            };
            let r = &sp.roads[ri];
            // under 50 mph the listed road nearest the focus (F-15989 from M+176 to its rectangle, strictly nearer)
            if mph < 50.0 {
                let d = road_box_dist(focus, r.mid, r.dir, r.rect.len, r.rect.width);
                if d < nearest.0 {
                    nearest = (d, Some(ri));
                }
            }
            let w = Road::open_len(nav, ri, &self.race_clearing.off) * 0.01 * sp.value(ri as u32, free_fast) * speed_factor;
            sp.list.push((ri as u32, w));
            sp.order.push((NavNode::Road(ri), w));
            sp.listed[ri] = true;
            target += w;
        }
        // (the density scale, A-74605: on the sum, before the clamp - not in the cutscene branch)
        if !sp.cutscene {
            target *= self.density_scale.unwrap_or(1.0);
        }
        if target > 0.0 && target < 1.0 {
            target = 1.0;
        }
        sp.target = target;
        // the next pass's box: square when that road's direction is within 45 degrees of across the view (the view
        // M+224 . direction +96 by fmuls / fmadds y, z, x; |.| < 0.7071)
        sp.perp_next = nearest.1.is_some_and(|ri: usize| {
            let d = sp.roads[ri].dir;
            let fma = |a: f32, b: f32, c: f32| (a as f64).mul_add(b as f64, c as f64) as f32;
            fma(view.x, d.x, fma(view.z, d.z, view.y * d.y)).abs() < f32::from_bits(0x3F35_0481)
        });
        if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() && sp.frame % 300 == 1 {
            log::info!("traffic region list: perp {} next {} nearest {:?} view {:?} mph {:.1} listed {} of {} roads {:.3} junctions {:.3} target {:.3} box {:?} order {:?}", perp, sp.perp_next, nearest.1.map(|ri| (ri, nearest.0, sp.roads[ri].dir, sp.roads[ri].mid)), view, mph, sp.list.len(), sp.order.len(), sp.list.iter().map(|x| x.1).sum::<f32>(), sp.int_weight, sp.target, sp.region_box, sp.order);
        }
        // the cars on listed roads
        sp.current = self.listed_cars(sp, nav);
    }

    /// F-92144: up to 60 tries, at most min(target - current, 6) cars.
    #[allow(clippy::too_many_arguments)]
    fn spawn_frame(&mut self, sp: &mut Spawner, gpu: &crate::render::Gpu, world: &crate::world::World, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32, player_link: Option<usize>) {
        let mut n = (sp.target as i32 - sp.current as i32).min(6);
        if n <= 0 || (sp.list.is_empty() && sp.fe_ints.is_empty()) || self.models.is_empty() {
            return;
        }
        // (the front-end mode: one try a frame, A-67400)
        let fe = sp.frontend.is_some();
        // no spawning while the player is under 30 mph on a junction's lane (the lane's owner or
        // its parent flagged 4, F-41621)
        if mph * MPH < 13.41 && player_link.is_some_and(|l| (nav.links[l].owner_flags | nav.links[l].parent_flags) & 4 != 0) {
            return;
        }
        let t = sp.tune;
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let total: f32 = sp.list.iter().map(|x| x.1).sum::<f32>() + sp.int_weight;
        let mut made: Vec<(Vec3, Vec3, Vec3)> = Vec::new();
        for _ in 0..if fe { 1 } else { 60 } {
            if n <= 0 {
                break;
            }
            // a type (the focus district's list at this hour; trailers and full pools are skipped)
            let r = self.rand();
            let models = &self.models;
            let Some(name) = self.ambient.pick(focus, self.hour, r, &|m| models.iter().any(|x| x.name.eq_ignore_ascii_case(m))).map(|s| s.to_string()) else {
                sp.why[0] += 1;
                continue;
            };
            if super::is_trailer(&name) {
                sp.why[0] += 1;
                continue;
            }
            let Some(mi) = self.models.iter().position(|m| m.name.eq_ignore_ascii_case(&name)) else {
                sp.why[0] += 1;
                continue;
            };
            let mname = self.models[mi].name.clone();
            let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| AmbType::load(&world.assets.root, &mname));
            let used = self.cars.iter().filter(|c| c.model.name == mname).count() as u32;
            if used >= ty.instances {
                sp.why[1] += 1;
                continue;
            }
            // the front-end mode: a point of the front-end list (S+8240 of them, the global MWC %
            // count), its lane and phase; the lane's road (F-86691), else the try is wasted
            let fe_at = if fe {
                if sp.fe_pts.pts.is_empty() {
                    continue;
                }
                let k = (self.rand() * sp.fe_pts.pts.len() as f32) as usize % sp.fe_pts.pts.len();
                let Some((link, s)) = sp.fe_pts.pts[k].3 else { continue };
                let Some(ri) = sp.road_of(nav, link) else { continue };
                Some((ri, link, s))
            } else {
                None
            };
            // a region by weight (a junction wastes the try)
            let mut x = self.rand() * total;
            let mut pick = None;
            for &(node, w) in &sp.order {
                if x < w {
                    pick = match node {
                        NavNode::Road(r) => Some(r as u32),
                        NavNode::Junction(_) => None,
                    };
                    break;
                }
                x -= w;
            }
            let pick = match fe_at {
                Some((ri, _, _)) => Some(ri),
                None => pick,
            };
            let Some(ri) = pick else {
                sp.why[2] += 1;
                continue;
            };
            // a lane of the road and the phase along it (F-10273, `pick_lane`: the record's lanes by their length, the
            // race traffic clearing's weighing 0, two draws of the shared MWC); the phase's place on the port's links. (The
            // front-end mode has its point's lane and phase.)
            let (link, s) = match fe_at {
                Some((_, k, sk)) => (k, sk),
                None => {
                    let off = &self.race_clearing.off;
                    let rl = nav.records.road_lanes.get(ri as usize).map_or(&[][..], |v| v.as_slice());
                    let Some((li, t)) = pick_lane(rl, |l| if off.contains(&l) { 0.0 } else { nav.lane_len(l) }, &mut self.park_rng) else { continue };
                    let Some(at) = lane_link_at(nav, lanes(nav), rl[li], t) else { continue };
                    at
                }
            };
            let l = &nav.links[link];
            let pos = l.a + (l.b - l.a) * (s / l.len.max(1e-3));
            let dir = l.tangent_at(s / l.len.max(1e-3));
            let fw = sp.roads[ri as usize].speed >= FREEWAY_SPEED;
            if ty.freeway_only && !fw {
                sp.why[3] += 1;
                continue;
            }
            // one way of a road not crowded over the other
            if !fw {
                let (mut same, mut opp) = (0, 0);
                for i in 0..self.cars.len() {
                    if self.ambient_car(i) && self.car_road(sp, nav, i) == Some(ri) {
                        if self.cars[i].state.forward().dot(dir) > 0.0 {
                            same += 1
                        } else {
                            opp += 1
                        }
                    }
                }
                if same as f32 > 1.5 * opp as f32 && (same > 3 || opp > 3) {
                    sp.why[4] += 1;
                    continue;
                }
            }
            // room (F-92144's tests in its order; docs/reference/people/ambient_spawn_rule.md): the junction ahead of the
            // player (the tracker's record 0; record 1 is never written), the slot test (M+2115 is 1 from the ctor), this
            // frame's earlier requests, the room test (racers only)
            let size = Vec3::new(ty.width, ty.height, ty.length);
            let jrec = if fe { None } else { self.player_record.map(|r| r.0) };
            if jrec.is_some_and(|j| junction_ahead_blocks(pos, nav.junction_rects[j].c, nav.junction_centers[j].y, nav.junction_rects[j].len)) {
                sp.why[5] += 1;
                continue;
            }
            if !self.slot_clear(nav, &Rider::new(nav, ty, link, s, 1), (size.x, size.z), None) {
                sp.why[5] += 1;
                continue;
            }
            if made.iter().any(|&(d, sz, q)| {
                let dy = q.y - pos.y;
                !(dy > size.y || -sz.y > dy) && rects_meet(Vec2::new(pos.x, pos.z), Vec2::new(dir.x, dir.z), Vec2::new(0.5 * size.z, 0.5 * size.x), Vec2::new(q.x, q.z), Vec2::new(d.x, d.z), Vec2::new(0.5 * sz.z, 0.5 * sz.x))
            }) {
                sp.why[5] += 1;
                continue;
            }
            let racers: Vec<RoomCar> = self
                .cars
                .iter()
                .map(|c| {
                    let h = c.params().body_half;
                    RoomCar { kind: crate::brainturn::ObjectInfo::from_car(0, c).kind, pos: c.state.pos, vel: c.state.vel, extent: crate::physics::exact::fma(h.x, h.x, h.z * h.z).sqrt() }
                })
                .collect();
            if traffic_room_blocked(&CrossGrid::UNBOUNDED, pos, &racers, ty.bound_r, t.spacing) {
                sp.why[5] += 1;
                continue;
            }
            // unseen (a long type at both ends too; the front-end mode has no visibility test,
            // A-85419)
            if !fe && seen(&t, focus, view, mph, pos, occ.as_ref()) {
                sp.why[6] += 1;
                continue;
            }
            if ty.bound_group == 2 && !fe {
                let m = &self.models[mi];
                let h = if ty.length > 0.0 { ty.length * 0.5 } else { (m.max.z - m.min.z) * 0.5 };
                if seen(&t, focus, view, mph, pos + dir * h, occ.as_ref()) || seen(&t, focus, view, mph, pos - dir * h, occ.as_ref()) {
                    continue;
                }
            }
            // (the request made: F-92144 lists its direction, size and point for the frame's later tries)
            made.push((dir, size, pos));
            // on the ground: a probe +- 1.75 m at the point, none - no spawn ("could not spawn
            // on terrain", F-46111)
            let Some((ground, normal)) = probe_ground(&world.collider, pos) else {
                sp.why[7] += 1;
                continue;
            };
            let model = self.models[mi].clone();
            // its colour: one of the type's twenty (F-92144: the shared MWC's (lo & 0x7FFFFFFF) % 20)
            let shade = test_shade(self.park_rng.index(20) as usize);
            let color = ty.color(shade).unwrap_or(super::TRAFFIC_PALETTE[shade % super::TRAFFIC_PALETTE.len()]);
            let yaw = (-dir.x).atan2(-dir.z);
            let idx = self.spawn(gpu, model, ground, yaw, color, false);
            // (the request's seed and the car's init draws, `amb_init`: its life, start delay, the lights' hour offset
            // +3568 and the signal blink's offsets +3576 / +3580 - the blink +3572 starts at 0 -, the bonus, the hitch)
            let init = self.amb_init(idx, &ty, false);
            let f = (dir - normal * dir.dot(normal)).normalize_or(dir);
            let right = f.cross(normal).normalize_or(Vec3::X);
            self.cars[idx].state.relocate(ground, Quat::from_mat3(&glam::Mat3::from_cols(right, normal, -f)).normalize());
            let seed = (self.rand() * 16777216.0) as u32 | 1;
            let mut rider = Rider::new(nav, ty, link, s, seed);
            rider.choose_next(nav, &mut self.park_rng);
            rider.born = self.clock;
            (rider.life, rider.off_timer, rider.wait, rider.bonus, rider.mwc) = (init.life, init.off_timer, init.wait, init.bonus, init.car);
            rider.seat = Some(Seat::new(ground, normal, f));
            // the start speed (vfunc 84 F-30683, called by the spawn slot 5 F-77422
            // for every ambient car, path = none): removed (reason 17) when the rule says so
            let (hl, hw) = half_size(&self.cars[idx]);
            match self.path_start_speed(nav, &rider, ground, (2.0 * hw, 2.0 * hl), None, Some(idx)) {
                Some(v) => rider.v = v,
                None => {
                    if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
                        log::info!("traffic spawn {} removed by the start speed rule", self.cars[idx].model.name);
                    }
                    self.cars.remove(idx);
                    n -= 1;
                    continue;
                }
            }
            self.cars[idx].rail = Some(rider);
            if let Some(ai) = self.cars[idx].ai.as_mut() {
                ai.link = Some(link);
                ai.t = s / l.len.max(1e-3);
            }
            // a cab: its trailer
            self.hitch_trailer(gpu, &world.assets.root, nav, idx, init.hitch);
            if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
                let v = pos - focus;
                log::info!("traffic spawn {} dist {:.1} ahead {:.1} side {:.1} road {} target {:.2} current {}", self.cars[idx].model.name, Vec3::new(v.x, 0.0, v.z).length(), v.dot(view), (v.z * view.x - v.x * view.z).abs(), ri, sp.target, sp.current);
            }
            sp.current += 1;
            n -= 1;
        }
    }

    /// The traffic region list's count (M +1792; None before the spawner exists): the unspawners' dispatcher
    /// F-75661 runs none of its four unspawners, the people's F-39594 among them, while it is 0.
    pub fn traffic_regions(&self) -> Option<usize> {
        self.spawner.as_ref().map(|s| s.list.len())
    }

    /// The crossing unspawner (F-54093, first of the dispatcher F-75661): an unseen car whose next 5 s cross
    /// the player's next 2 s is removed (reason 6), at most one a run (`crossing_unspawn`). Not run in the
    /// cutscene (F-97032) or front-end (F-95688) modes, nor networked (F-24338, offline false); its enable byte
    /// M +2109 is 1 from the ctor F-10264, its only writer.
    fn crossing_frame(&mut self, sp: &Spawner, world: &crate::world::World, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32) {
        if sp.cutscene || sp.frontend.is_some() {
            return;
        }
        // the velocity M +208: the player's (F-83397, the player's obstacle slot 2), none without a player
        let vel = self.cars.iter().find(|c| c.is_player).map_or(Vec3::ZERO, |c| c.state.vel);
        let cars: Vec<CrossCar> = self.cars.iter().map(|c| {
            // (kind 2: the obstacles of the mcAmbientCars - riding and parked; the player, cops and racers are other kinds)
            let kind2 = !c.is_player && !c.cop && !c.racer && !c.pinned && (c.rail.is_some() || c.parked.is_some());
            let r = c.rail.as_ref().filter(|r| r.state != 0);
            CrossCar {
                kind2,
                pos: c.state.pos,
                vel: c.state.vel,
                // (car +500 bit 0x20: set by F-53584 for the bIsProxy pool's cars)
                proxy: r.is_some_and(|r| r.ty.proxy),
                // (F-23063 offline: F-92604 makes a car's id with F-78245's peer in its low 5 bits - the same value
                // F-23063 compares with - or takes the params' id or -1: never remote)
                remote: false,
                // (car +452: a trailer and a parked car have no behaviour)
                behaviour: r.is_some(),
                group2: r.is_some_and(|r| r.ty.bound_group == 2),
                item: c.item.is_some(),
                slot18: r.is_some_and(|r| r.quiet),
                slot19: r.is_some_and(|r| r.state == 9),
                slot22: r.is_some_and(|r| r.path_referenced),
                // (F-31543 on the follower's rail, behaviour +188: a navsys lane's road; a dynamic rail's owner +8 is 0
                // from its ctor F-33069 / F-36658, and a physical or let-go car has none)
                freeway: r.is_some_and(|r| r.state == 1 && sp.road_of(nav, r.link).is_some_and(|x| sp.roads[x as usize].speed >= FREEWAY_SPEED)),
                fwd: c.state.forward(),
                radius: c.model.lod_radius,
            }
        }).collect();
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let col = &world.collider;
        let run = crossing_unspawn(&CrossGrid::UNBOUNDED, focus, vel, view, &cars, |_, p| seen(&sp.tune, focus, view, mph, p, occ.as_ref()), |_, eye, end| {
            let d = end - eye;
            let len = d.length();
            len > 0.0 && col.probe_front(eye, d / len, len).is_some()
        });
        if let Some(i) = run.remove {
            if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
                let c = &self.cars[i];
                log::info!("traffic crossing unspawn {} dist {:.1} (of {} looked at)", c.model.name, Vec3::new(c.state.pos.x - focus.x, 0.0, c.state.pos.z - focus.z).length(), run.tested.len());
            }
            self.cars.remove(i);
        }
    }

    /// F-65438 (offline): 30 tries, at most one car removed (reason 8). Each try draws a car of the manager's list 0
    /// (F-92034: the shared MWC's index into the ambient cars, riding and parked) and passes over one that is not an
    /// mcAmbientCar's (obstacle +6 != 2), a proxy (+500 bit 0x20), another peer's (F-23063, offline never), without a
    /// behaviour (+452: a parked car, a trailer), whose behaviour's slot 19, slot 18 or slot 22 says so, or a layout
    /// item's (+40). Then: on a navsys lane (F-96472, r23): listed when the lane's region (F-17030: its road, else its
    /// junction) is on the list (M+1392, or the cutscene's neighbours M+988); off a lane: listed within +16 (the outer
    /// radius) of M+192. Stuck (r22): the stop line in charge (+420 == 1; its wanted speed +416 is 0) and the speed +412
    /// below 6.7056 m/s (15 mph), not in the cutscene. r24: its lane held by the race clearing (rail slot 18, F-73172). Listed and not held: kept while younger
    /// than its least life (+468). Crowded (r28): listed, on a lane of a region whose weight is above 0 and whose cars
    /// over that weight are above 1.5, not in the cutscene. r30: in a race, listed and within the junction ahead of the
    /// player (`race_junction_holds`). The seen test F-92805 only for a listed car (the front-end mode: seen = listed).
    /// Removed when not seen and not listed, held, stuck, at the junction, crowded or off a lane.
    fn unspawn_frame(&mut self, sp: &Spawner, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32) {
        let pool: Vec<usize> = (0..self.cars.len()).filter(|&i| self.ambient_listed(i)).collect();
        if pool.is_empty() {
            return;
        }
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let lc = self.lane_cars(nav);
        let jrec = if sp.frontend.is_some() { None } else { self.player_record.map(|r| r.0) };
        let listed_j: hashbrown::HashSet<usize> = sp.order.iter().filter_map(|&(n, _)| match n {
            NavNode::Junction(j) => Some(j),
            _ => None,
        }).collect();
        for _ in 0..30 {
            let i = pool[self.park_rng.index(pool.len() as u32) as usize];
            let c = &self.cars[i];
            let Some(r) = c.rail.as_ref().filter(|r| r.state != 0) else { continue };
            if r.ty.proxy || c.item.is_some() || r.quiet || r.state == 9 || r.path_referenced {
                continue;
            }
            let pos = c.state.pos;
            let on_lane = r.state == 1;
            let region = if on_lane {
                sp.road_of(nav, r.link).map(|x| NavNode::Road(x as usize)).or_else(|| nav.links.get(r.link).and_then(|l| nav.lane_junction.get(l.lane).copied().flatten()).map(NavNode::Junction))
            } else {
                None
            };
            let listed = if on_lane {
                match region {
                    Some(NavNode::Road(x)) => sp.listed[x],
                    Some(NavNode::Junction(j)) => listed_j.contains(&j),
                    None => false,
                }
            } else {
                Vec3::new(pos.x - focus.x, 0.0, pos.z - focus.z).length_squared() < sp.tune.unspawn_outer * sp.tune.unspawn_outer
            };
            let held = on_lane && nav.links.get(r.link).is_some_and(|l| self.race_clearing.off.contains(&l.lane));
            if listed && !held && self.clock - r.born < r.life {
                continue;
            }
            // (r28 only outside the cutscene mode, F-97032)
            let crowded = listed && on_lane && !sp.cutscene && region.is_some_and(|n| {
                let w = sp.order.iter().find(|x| x.0 == n).map_or(0.0, |x| x.1);
                w > 0.0 && (Self::region_cars(nav, &lc, n) as f32) / w > 1.5
            });
            let stuck = r.slot == Some(1) && r.v < 6.7056 && !sp.cutscene;
            // r30, the race clearing's junction: listed, M+2105 (0 from the ctor F-10264) or M+2106 (1 from it) with the
            // race byte S+0x64251, within the junction of one of the tracker's two records (only record 0 is written: the
            // player's junction ahead) - |dy| not over 5, fsqrts(fmadds(dx, dx, fmadds(dz, dz, dy dy))) under the
            // junction rect's length +0x40 -; not with -notrafficclearing (never set) nor in the cutscene mode (F-97032)
            let at_junction = listed && self.race_event && !sp.cutscene && jrec.is_some_and(|j| race_junction_holds(pos, nav.junction_rects[j].c, nav.junction_centers[j].y, nav.junction_rects[j].len));
            let seen_now = if sp.frontend.is_some() { listed } else { listed && seen(&sp.tune, focus, view, mph, pos, occ.as_ref()) };
            if seen_now {
                continue;
            }
            if !listed || held || stuck || at_junction || crowded || !on_lane {
                if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
                    let v = pos - focus;
                    log::info!("traffic unspawn {} dist {:.1} ahead {:.1} age {:.1} off_list {} crowded {} stuck {} off_lane {} junction {}", c.model.name, Vec3::new(v.x, 0.0, v.z).length(), v.dot(view), self.clock - r.born, !listed, crowded, stuck, !on_lane, at_junction);
                }
                self.cars.remove(i);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// The crossing unspawner (F-54093; docs/reference/people/crossing_unspawn_rule.md)

/// The object grid as the crossing unspawner reads it ([0x8286F1B4]; F-88455 -> F-59709 for a point's cell, F-99153 for
/// the 3 x 3 cells round it): 40 m cells, a point's cell ((x - min) x 0.025 truncated, the same for z) with the cell
/// number iz w + ix kept in 16 bits; off the grid (x < min or x >= max, max = min + 40 n) or a number of 0xFFFF or w h
/// and above: no cells at all. Unbounded: the port has no level bounds (brainturn.rs `ObjectGrid::unbounded`), the
/// cells are floor(x / 40).
#[derive(Clone, Copy, Debug)]
pub struct CrossGrid {
    pub min: [f32; 2],
    pub n: [i32; 2],
    pub bounded: bool,
}

impl CrossGrid {
    pub const UNBOUNDED: CrossGrid = CrossGrid { min: [0.0; 2], n: [0; 2], bounded: false };

    /// The cell (ix, iz) of a point (F-59709 and the cell number's checks of F-99153).
    pub fn cell(&self, x: f32, z: f32) -> Option<(i32, i32)> {
        if !self.bounded {
            return Some(((x * 0.025).floor() as i32, (z * 0.025).floor() as i32));
        }
        let max = [self.min[0] + 40.0 * self.n[0] as f32, self.min[1] + 40.0 * self.n[1] as f32];
        if x < self.min[0] || x >= max[0] || z < self.min[1] || z >= max[1] {
            return None;
        }
        let (ix, iz) = (((x - self.min[0]) * 0.025) as i32, ((z - self.min[1]) * 0.025) as i32);
        let id = (iz.wrapping_mul(self.n[0]).wrapping_add(ix) as u32 & 0xFFFF) as i32;
        if id == 0xFFFF || id >= self.n[0] * self.n[1] {
            return None;
        }
        Some((id % self.n[0], id / self.n[0]))
    }

    /// Whether cell (ix, iz) is one of the grid's (F-99153: a row or column outside gives an empty list).
    fn holds(&self, ix: i32, iz: i32) -> bool {
        !self.bounded || (ix >= 0 && ix < self.n[0] && iz >= 0 && iz < self.n[1])
    }
}

/// The counts a region's vfunc 10 reads: each lane's followers (rail slot 21, F-90597: all, those with a layout item's
/// car) and each road's attached rails (+160).
#[derive(Default)]
struct RegionCounts {
    lanes: hashbrown::HashMap<usize, (i32, i32)>,
    roads: hashbrown::HashMap<usize, i32>,
}

/// F-65438's r30 test: the car at `p` within the junction's length (rect +0x40, the whole of it) of the junction's point
/// (x / z +0x50 / +0x58, y +0x84), the height difference not over 5.0 (`fcmpu` + `bgt`: a NaN passes); the distance
/// fsqrts(fmadds(dx, dx, fmadds(dz, dz, dy dy))), `bge` (a NaN passes).
pub fn race_junction_holds(p: Vec3, c: [f32; 2], cy: f32, len: f32) -> bool {
    use crate::physics::exact::fma;
    let (dx, dy, dz) = (p.x - c[0], p.y - cy, p.z - c[1]);
    if dy.abs() > 5.0 {
        return false;
    }
    !(fma(dx, dx, fma(dz, dz, dy * dy)).sqrt() >= len)
}

/// F-92144's junction test: is the spawn point `p` within half the junction's length (rect +0x40) of the junction's
/// point - its region's x / z (+0x50 / +0x58: in the file the bits of +0x34 / +0x38 for all 351 junctions) and its
/// centre's y (+0x84) -, the height difference not over 5? The distance as the vector unit's: the squared length z^2 +
/// (y^2 + x^2), its reciprocal root and one Newton step, 0 for a squared length of 0 (flush to zero on).
pub fn junction_ahead_blocks(p: Vec3, c: [f32; 2], cy: f32, len: f32) -> bool {
    let d = Vec3::new(p.x - c[0], p.y - cy, p.z - c[1]);
    if d.y.abs() > 5.0 {
        return false;
    }
    let l2 = crate::physics::exact::dot3(d, d);
    let l2 = if l2.abs() < f32::MIN_POSITIVE { 0.0 } else { l2 };
    let dist = if l2 == 0.0 {
        0.0
    } else {
        let y = 1.0 / l2.sqrt();
        l2 * (-(l2 * (y * y) - 1.0) * (0.5 * y) + y)
    };
    dist - 0.5 * len < 0.0
}

/// What the traffic room test (F-43944) reads of one object of the grid's cells: its kind (obstacle +6; only 0 the
/// player and 1 the racers are looked at), its place (vfunc 1's matrix +48), its velocity (vfunc 2) and its extent
/// (vfunc 7).
#[derive(Clone, Copy, Debug, Default)]
pub struct RoomCar {
    pub kind: u8,
    pub pos: Vec3,
    pub vel: Vec3,
    pub extent: f32,
}

/// The traffic room test F-43944 (docs/reference/people/ambient_spawn_rule.md): the objects of the 3 x 3 grid cells
/// round the spawn point `p` (`CrossGrid::cell`, as the people's room test); of kind 0 / 1 only. Blocked by one within
/// the flat `spacing` (fma(dz, dz, dx dx) < spacing^2, d = its place - p), or else by one whose one-second motion meets
/// the sphere at `p` of radius its extent + `bound_r` (F-24126, `sphere_motion_hit`).
pub fn traffic_room_blocked(grid: &CrossGrid, p: Vec3, cars: &[RoomCar], bound_r: f32, spacing: f32) -> bool {
    use crate::physics::exact::fma;
    let Some((cx, cz)) = grid.cell(p.x, p.z) else { return false };
    cars.iter().any(|o| {
        if o.kind > 1 {
            return false;
        }
        match grid.cell(o.pos.x, o.pos.z) {
            Some((ox, oz)) if (ox - cx).abs() <= 1 && (oz - cz).abs() <= 1 && grid.holds(ox, oz) => {}
            _ => return false,
        }
        let (dx, dz) = (o.pos.x - p.x, o.pos.z - p.z);
        if fma(dz, dz, dx * dx) < spacing * spacing {
            return true;
        }
        sphere_motion_hit(p, o.extent + bound_r, o.pos, o.vel)
    })
}

/// F-24126: does a point moving from `s` by `d` meet the sphere (centre `c`, radius `r`)? The start inside (|s - c|^2 <
/// r^2, summed x^2 then z then y, fused), else the end s + d not outside (not >= r^2: a NaN counts as inside), else the
/// segment's crossing (F-47913, `ray_sphere`, > 0).
pub fn sphere_motion_hit(c: Vec3, r: f32, s: Vec3, d: Vec3) -> bool {
    use crate::physics::exact::fma;
    let rr = r * r;
    let l2 = |a: Vec3| {
        let (x, y, z) = (a.x - c.x, a.y - c.y, a.z - c.z);
        fma(y, y, fma(z, z, x * x))
    };
    if l2(s) < rr {
        return true;
    }
    if !(l2(s + d) >= rr) {
        return true;
    }
    ray_sphere(s - c, d, rr) > 0
}

/// F-47913 with no outputs and its flag clear: the segment o .. o + d against the sphere of squared radius `rr` round the
/// origin. 0: no meeting (d of no length, the discriminant below 0, from outside a crossing t1 outside [0, 1], from inside
/// an end inside or d under 1e-12); from outside 1 when the far crossing t2 is past 1, else 2; from inside 1.
pub fn ray_sphere(o: Vec3, d: Vec3, rr: f32) -> i32 {
    use crate::physics::exact::fma;
    let dd = fma(d.z, d.z, fma(d.x, d.x, d.y * d.y));
    if dd <= 0.0 {
        return 0;
    }
    let oo = fma(o.z, o.z, fma(o.x, o.x, o.y * o.y));
    let od = fma(o.z, d.z, fma(o.x, d.x, o.y * d.y));
    let disc = fma(rr - oo, dd, od * od);
    if disc < 0.0 {
        return 0;
    }
    if oo > rr {
        let inv = 1.0 / dd;
        let t1 = -((disc.sqrt() + od) * inv);
        if t1 < 0.0 || t1 > 1.0 {
            return 0;
        }
        let t2 = (disc.sqrt() - od) * inv;
        return if t2 > 1.0 { 1 } else { 2 };
    }
    let e = o + d;
    let ee = fma(e.z, e.z, fma(e.y, e.y, e.x * e.x));
    if ee <= rr || dd < f32::from_bits(0x2B8C_BCCC) {
        return 0;
    }
    1
}

/// What the crossing unspawner reads of one object of the grid's cells (an ambient car's obstacle and its car).
#[derive(Clone, Copy, Debug, Default)]
pub struct CrossCar {
    /// obstacle +6 == 2: an mcAmbientCar's obstacle
    pub kind2: bool,
    /// the obstacle's matrix row 3 (its slot 1, +48) - the car's place F-58860 is the same point
    pub pos: Vec3,
    /// the obstacle's slot 2: the car's velocity (+208)
    pub vel: Vec3,
    /// car +500 bit 0x20
    pub proxy: bool,
    /// F-23063
    pub remote: bool,
    /// car +452 set
    pub behaviour: bool,
    /// type +116 (iBoundGroup) == 2
    pub group2: bool,
    /// car +40 set (a layout item's car)
    pub item: bool,
    /// the behaviour's slots 18 (F-86955), 19 (F-74598 -> F-35476), 22 (F-90466)
    pub slot18: bool,
    pub slot19: bool,
    pub slot22: bool,
    /// F-31543 on the behaviour's rail (+188): a freeway lane
    pub freeway: bool,
    /// F-50349: -row 2 of the car's matrix
    pub fwd: Vec3,
    /// car +492: 0.5 sqrt 2 x the larger of the bound's width and length (`CarModel::lod_radius`)
    pub radius: f32,
}

/// One run of the crossing unspawner: the player's segment, the cars looked at (index, their segment's ends, crossed)
/// and the car removed.
#[derive(Clone, Debug, Default)]
pub struct CrossRun {
    /// M +400 / +408: the player's place and its place 2 s on (x, z)
    pub a: [f32; 2],
    pub b: [f32; 2],
    /// the point whose cells are looked at: 3/4 along that segment, at the focus height
    pub p: Vec3,
    /// M +416 + 8 i / +672 + 8 i / +928 + i (32 at most)
    pub tested: Vec<(usize, [f32; 2], [f32; 2], bool)>,
    pub remove: Option<usize>,
}

/// F-54093 after its exits: the player's next 2 s (focus + velocity x 2, flat) and, for each ambient car of the 3 x 3
/// grid cells round the point 3/4 along it (rows, then columns, then the cell's own order), unless 32 were already
/// looked at (then the run ends): kept when more than 3 m above or below the focus, a proxy, remote, without a
/// behaviour, of bound group 2 and more than 2 m above or below, an item's car, on a race path (slots 18 / 19 / 22) or
/// on a freeway lane; else its next 5 s (place + velocity x 5, flat) is tested against the player's (F-95591 -> F-66886,
/// both phases in [0, 1]); a crossing car is removed when not seen (`seen`, F-92805 at 1.5 m) or when both lines
/// from the eye (the focus 1.75 m up, plus the view x 1) to its ends 1.75 m up (place +- forward x radius) are blocked
/// (`blocked`, F-13384 -> F-57005); the first such car ends the run.
pub fn crossing_unspawn(grid: &CrossGrid, focus: Vec3, vel: Vec3, view: Vec3, cars: &[CrossCar], mut seen: impl FnMut(usize, Vec3) -> bool, mut blocked: impl FnMut(usize, Vec3, Vec3) -> bool) -> CrossRun {
    let a = [focus.x, focus.z];
    let b = [a[0] + vel.x * 2.0, a[1] + vel.z * 2.0];
    let p = Vec3::new((b[0] - a[0]) * 0.75 + a[0], focus.y, (b[1] - a[1]) * 0.75 + a[1]);
    let mut run = CrossRun { a, b, p, ..Default::default() };
    let Some((cx, cz)) = grid.cell(p.x, p.z) else { return run };
    let cells: Vec<Option<(i32, i32)>> = cars.iter().map(|c| grid.cell(c.pos.x, c.pos.z)).collect();
    let flat = |v: [f32; 2]| Vec3::new(v[0], 0.0, v[1]);
    for dz in -1..=1 {
        for dx in -1..=1 {
            let (ix, iz) = (cx + dx, cz + dz);
            if !grid.holds(ix, iz) {
                continue;
            }
            for (i, c) in cars.iter().enumerate() {
                if cells[i] != Some((ix, iz)) || !c.kind2 {
                    continue;
                }
                if run.tested.len() == 32 {
                    return run;
                }
                if (c.pos.y - p.y).abs() > 3.0 || c.proxy || c.remote || !c.behaviour {
                    continue;
                }
                if c.group2 && (c.pos.y - focus.y).abs() > 2.0 {
                    continue;
                }
                if c.slot18 || c.item || c.slot19 || c.slot22 || c.freeway {
                    continue;
                }
                let s = [c.pos.x, c.pos.z];
                let e = [s[0] + c.vel.x * 5.0, s[1] + c.vel.z * 5.0];
                // (F-95591: both phases within [0, 1]; a NaN compares as within)
                let hit = crate::checkpoints::cross_xz(flat(a), flat(b), flat(s), flat(e)).is_some_and(|(t, u)| !(t < 0.0 || t > 1.0 || u < 0.0 || u > 1.0));
                run.tested.push((i, s, e, hit));
                if !hit {
                    continue;
                }
                if !seen(i, c.pos) {
                    run.remove = Some(i);
                    return run;
                }
                let eye = Vec3::new(view.x + focus.x, view.y + (focus.y + 1.75), view.z + focus.z);
                let q = Vec3::new(c.pos.x, c.pos.y + 1.75, c.pos.z);
                let ends = [c.fwd * c.radius + q, c.fwd * -c.radius + q];
                if ends.iter().all(|&end| blocked(i, eye, end)) {
                    run.remove = Some(i);
                    return run;
                }
            }
        }
    }
    run
}

// ---------------------------------------------------------------------------------
// The races' ambient paths (`*_AmbientData.xml`, mcAmbientPath; decoded 2026-10-21,
// work/people2/re/ag16_report.txt A1 - A6, the vertex snap work/people2/racepaths/report.txt):
// path init (F-87998: the vertices snapped onto lanes, the curve rail, the join rail, the
// spawn matrix) and the car that follows the path (F-45833 with the path's params; its
// behaviour route F-28180): the curve rail, the join rail, then the path's lane as
// ordinary traffic, at most fPathMaxSpeed for its whole life.

/// A path vertex looks for a lane within 30 m (F-46970: the vertex's w, [0x82006BF0]).
pub const SNAP_RADIUS: f32 = 30.0;

/// The nearest point of a curve to `p` (F-53741): the original's search, `railroute::nearest` (bit-exact).
/// Returns (phase, squared distance, point) as the last halving wrote them.
pub fn curve_nearest(c: &LaneCurve, p: Vec3, n: usize, iters: usize) -> (f32, f32, Vec3) {
    crate::railroute::nearest(c, p, n as u32, iters as u32)
}

/// A lane's box (lane +16 / +32), its stored length (+0c) and its flags (+4), as the file holds them.
struct LaneBox {
    min: Vec3,
    max: Vec3,
    len: f32,
    flags: u32,
}

/// The lanes' boxes, made once (for `rail_nav`'s curves).
fn lane_boxes(nav: &NavSys) -> &'static [LaneBox] {
    static B: std::sync::OnceLock<Vec<LaneBox>> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        (0..nav.curves.len())
            .map(|i| {
                let (flags, len, min, max) = nav.lane_stored(i);
                LaneBox { min, max, len, flags }
            })
            .collect()
    })
}

/// A lane's answer to a point (lane slot 3 F-15621: the 48-byte record {point, tangent,
/// lane, squared distance, phase, valid}).
#[derive(Clone, Copy, Debug)]
pub struct LaneHit {
    pub lane: usize,
    pub t: f32,
    pub d2: f32,
    pub point: Vec3,
    /// the curve's derivative there (F-26180)
    pub dir: Vec3,
}

/// The lane nearest to `p` within `r` (F-66090 -> F-59579 with no heading): the lane
/// quadtree's nodes whose x / z box comes within r (F-21575 / F-80185), their lanes with
/// flags & 1 whose box comes within r (lane slot 5 F-56243) asked for their nearest curve
/// point (slot 3: F-53741 with 8 chords for a lane of 450 m or more, else 4, and 10 halvings),
/// valid when the squared distance is under r^2; the least squared distance wins (the first of
/// equals). Also the number of valid answers (the original keeps 64 at most).
pub fn snap_to_lane(nav: &NavSys, p: Vec3, r: f32) -> (Option<LaneHit>, usize) {
    let boxes = lane_boxes(nav);
    let mut best: Option<LaneHit> = None;
    let mut hits = 0;
    for (lane, b) in boxes.iter().enumerate() {
        if b.flags & 1 == 0 {
            continue;
        }
        let Some(h) = lane_answer(nav, lane, p, r) else { continue };
        hits += 1;
        if best.as_ref().map_or(true, |x| h.d2 < x.d2) {
            best = Some(h);
        }
    }
    (best, hits)
}

/// One lane's answer to a point within `r` (F-15621): the lane's box within r (slot 5, F-56243), its nearest curve
/// point with 8 chords for a lane of 450 m or more, else 4, and 10 halvings (F-53741), valid when the squared distance is
/// under r^2 (strict); then the curve's derivative at that phase (F-26180) as the answer's direction.
pub fn lane_answer(nav: &NavSys, lane: usize, p: Vec3, r: f32) -> Option<LaneHit> {
    let b = lane_boxes(nav).get(lane)?;
    let o = (b.min - p).max(p - b.max).max(Vec3::ZERO);
    if !(r * r > o.z * o.z + (o.y * o.y + o.x * o.x)) {
        return None;
    }
    let c = &nav.curves[lane];
    let n = if b.len >= 450.0 { 8 } else { 4 };
    let (t, d2, point) = curve_nearest(c, p, n, 10);
    if !(d2 < r * r) {
        return None;
    }
    Some(LaneHit { lane, t, d2, point, dir: crate::railroute::deriv(c, t) })
}

/// The curve rail of a path (F-86418 -> F-80887 -> F-42351; class F-33069,
/// flags 0x801): a clamped B-spline over `cps`, degree min(3, k - 1) (at least 1), the knots
/// handed over p zeros, 1 .. k - p - 1, then k - p repeated p times, with one more knot put
/// before and after (K0 = K1 - (K2 - K1), the last = the one before + its step; with four
/// knots both copied); TRange = K[count - p - 1] = k - p, TMax = TRange - 1e-4.
/// F-48384's +180 (the follower's set rail): 0 stored, then the rail's slot 9 (F-87587 on a curve) at the phase with
/// that 0 - |dP/du| x TRange, not averaged.
fn set_rail_speed(c: &LaneCurve, t: f32) -> f32 {
    c.pose_slot9(t, 0.0).2
}

/// F-50894's carry onto the rail just set at phase `t`: the rest of the step (only when above 0) at the rail's slot 11
/// speed (F-75223 with 0), slot 6 from `t`, the phase then clamped by fsel (below 0 or NaN: 0; 1 and up: 1). Once a
/// step: past the end of this rail too the phase stays at 1.
fn carry_phase(c: &LaneCurve, t: f32, rest: f32) -> f32 {
    let p = LaneCurve::advance(t, rest, crate::ghostroute::lane_speed(c, t, 0.0));
    let f0 = if p >= 0.0 { p } else { 0.0 };
    if f0 - 1.0 >= 0.0 { 1.0 } else { f0 }
}

pub fn path_curve(cps: &[Vec3]) -> LaneCurve {
    let k = cps.len();
    let p = (k as i32 - 1).clamp(1, 3) as usize;
    let nk = p + k - 1;
    let mut given = vec![0.0f32; nk];
    let mut v = 1.0f32;
    let mut i = p;
    while i + p < nk {
        given[i] = v;
        v += 1.0;
        i += 1;
    }
    for x in given.iter_mut().skip(i) {
        *x = v;
    }
    let count = k + p + 1;
    let mut kn = vec![0.0f32; count];
    kn[1..=nk].copy_from_slice(&given);
    let shift = kn[p];
    for x in kn[1..=nk].iter_mut() {
        *x -= shift;
    }
    kn[p] = 0.0;
    if count > 4 {
        kn[0] = kn[1] - (kn[2] - kn[1]);
        kn[count - 1] = kn[count - 2] + (kn[count - 2] - kn[count - 3]);
    } else if count == 4 {
        kn[0] = kn[1];
        kn[3] = kn[2];
    }
    let t_range = kn[count - p - 1];
    LaneCurve { degree: p as u8, pts: cps.to_vec(), knots: kn, t_range, t_max: t_range - 1e-4 }
}

/// A race path after its init (F-87998).
#[derive(Clone, Debug, Default)]
pub struct PathRails {
    /// +32: the vertices, those that found a lane moved onto it
    pub snapped: Vec<Vec3>,
    /// +64 / +68: the lane the last vertex found and the phase there; +48 that lane's flat
    /// direction there
    pub lane: Option<(usize, f32)>,
    pub lane_dir: Vec3,
    /// +176: the curve rail
    pub curve: Option<LaneCurve>,
    /// +180: the join rail onto the lane (an arc-line-arc rail, F-45519) and its heading at the start
    pub join: Option<TcpRail>,
    pub join_dir: Vec3,
    /// the spawn matrix after init (F-94337): its place and z row
    pub spawn: Vec3,
    pub row2: Vec3,
    /// both rails' speed: fPathMaxSpeed x 0.44704 m/s (F-69121)
    pub speed: f32,
    /// (diagnostics) the most valid lane answers a vertex had; the original keeps 64
    pub hits: usize,
    /// +170 bPhysical: the car's instance +140 bit 0x80 (`Rider::phys_pair`)
    pub physical: bool,
    /// +164 bParked, as behaviour slot 18 reads it (`Rider::quiet`)
    pub quiet: bool,
}

/// Path init (F-87998). `snap`: bSnapToRails per vertex (a single-vertex path that is not
/// parked snaps its vertex); `min_turn`: the TypeName type's fMinTurningRadius (type +160; no
/// type, no join rail); `spawn` / `row2`: the file's SpawnMtx; `max_mph`: fPathMaxSpeed.
#[allow(clippy::too_many_arguments)]
pub fn path_rails(nav: &NavSys, verts: &[Vec3], snap: &[bool], parked: bool, min_turn: Option<f32>, spawn: Vec3, row2: Vec3, max_mph: f32) -> PathRails {
    let n = verts.len();
    let mut flags: Vec<bool> = (0..n).map(|i| snap.get(i).copied().unwrap_or(false)).collect();
    if !parked && n == 1 && !flags[0] {
        flags[0] = true;
    }
    let mut out = PathRails { speed: max_mph * 0.44704, spawn, row2, ..Default::default() };
    // F-46970: each flagged vertex onto its nearest lane; the last one keeps the lane
    for i in 0..n {
        let mut q = verts[i];
        if flags[i] {
            let (hit, hits) = snap_to_lane(nav, verts[i], SNAP_RADIUS);
            out.hits = out.hits.max(hits);
            if let Some(h) = hit {
                q = h.point;
                if i + 1 == n {
                    out.lane = Some((h.lane, h.t));
                    out.lane_dir = Vec3::new(h.dir.x, 0.0, h.dir.z).normalize_or_zero();
                }
            }
        }
        out.snapped.push(q);
    }
    // the join rail (F-45519): the last vertex flagged and the one before not; from the
    // vertex before along the file's last step (two vertices: the spawn matrix's -z) to the last
    // along the lane; the shortest of the four arc - line - arc ways at fMinTurningRadius.
    // (With no lane found the lane's direction is unset: no rail is made here.)
    if n >= 2 && flags[n - 1] && !flags[n - 2] {
        if let (Some(rr), Some(_)) = (min_turn, out.lane) {
            let h0 = join_heading(&verts[..n], row2);
            if let Some(q) = join_rail(out.snapped[n - 2], h0, out.snapped[n - 1], out.lane_dir, rr) {
                out.join = Some(q);
                out.join_dir = h0;
            }
        }
    }
    // the curve rail: over the snapped vertices (the last left to the join rail); none for a
    // single vertex or two whose first is not flagged and the second is
    if n >= 2 && !(n == 2 && !flags[0] && flags[1]) {
        let k = out.snapped.len() - out.join.is_some() as usize;
        if k >= 2 {
            out.curve = Some(path_curve(&out.snapped[..k]));
        }
    }
    // the spawn matrix: at the first snapped vertex, its z row against the first rail's flat
    // tangent at its start (the curve, the join rail, else the lane at its phase)
    out.spawn = out.snapped.first().copied().unwrap_or(spawn);
    let tan = match (&out.curve, &out.join, out.lane) {
        // (F-94337: the first rail's slot 10; the curve rail's vtable has the lane's slots 6 - 11, F-26180 here too)
        (Some(c), _, _) => Some(crate::railroute::deriv(c, 0.0)),
        (None, Some(_), _) => Some(out.join_dir),
        // (F-94337: the first rail's slot 10 - a lane's F-26180)
        (None, None, Some((l, t))) => nav.curves.get(l).map(|c| crate::railroute::deriv(c, t)),
        _ => None,
    };
    if let Some(t) = tan {
        let f = Vec3::new(t.x, 0.0, t.z).normalize_or_zero();
        if f != Vec3::ZERO {
            out.row2 = -f;
        }
    }
    out
}

/// The join rail's start heading (F-45519): the file's last step before the last vertex (more than two vertices:
/// vertex n - 2 - vertex n - 3), else -row 2 of the spawn matrix (0 - row 2), y = 0, made unit by the vector unit.
pub(crate) fn join_heading(verts: &[Vec3], row2: Vec3) -> Vec3 {
    let n = verts.len();
    let mut d = if n > 2 { crate::tcp_rail::vdiff(verts[n - 2], verts[n - 3]) } else { crate::tcp_rail::vdiff(Vec3::ZERO, row2) };
    d.y = 0.0;
    crate::tcp_rail::vnorm(d)
}

/// The join rail (F-45519): the four arc-line-arc rails P, H0 -> T, H1 at the radius (senses 00, 01, 10, 11), the
/// valid one strictly shortest (F-62306); None when none is valid.
pub(crate) fn join_rail(p: Vec3, h0: Vec3, t: Vec3, h1: Vec3, rr: f32) -> Option<TcpRail> {
    let mut best: Option<TcpRail> = None;
    let mut best_len = f32::MAX;
    for (s0, s1) in [(false, false), (false, true), (true, false), (true, true)] {
        let q = crate::tcp_rail::build(p, h0, t, h1, rr, s0, s1);
        if q.valid() && q.len() < best_len {
            best_len = q.len();
            best = Some(q);
        }
    }
    best
}

/// F-10286: the heading of a matrix row (x, z): the guest C runtime's atan2 (F-65975, in double) rounded to single,
/// 2 pi (6.2831855) added while it is not above 0 and taken off while it is above 2 pi.
pub fn heading_of(x: f32, z: f32) -> f32 {
    const TAU: f32 = 6.2831855;
    let mut h = crate::physics::exact::guest_atan2(x as f64, z as f64) as f32;
    while h <= 0.0 {
        h += TAU;
    }
    while h > TAU {
        h -= TAU;
    }
    h
}

impl PathRails {
    /// The heading of the spawn matrix (F-10286: atan2(z row x, z row z) in (0, 2 pi]).
    pub fn heading(&self) -> f32 {
        heading_of(self.row2.x, self.row2.z)
    }

    /// The follower's route (F-28180): more than one vertex - the curve rail, then the
    /// join rail, at phase 0 - then the path's lane at its phase; one vertex: the lane at once.
    /// None without a lane.
    pub fn route(&self, path: usize) -> Option<Route> {
        let (lane, lane_t) = self.lane?;
        let mut rails = Vec::new();
        if self.snapped.len() > 1 {
            if let Some(c) = &self.curve {
                rails.push(RouteRail::Curve(c.clone()));
            }
            if let Some(j) = &self.join {
                rails.push(RouteRail::Turn(Path::tcp(*j, self.speed, None)));
            }
        }
        Some(Route { path, rails, i: 0, t: 0.0, ds: 0.0, speed: self.speed, lane, lane_t })
    }
}

/// A rail of a path's route: the curve rail or the join rail.
#[derive(Clone, Debug)]
pub enum RouteRail {
    Curve(LaneCurve),
    Turn(Path),
}

/// A race path car's route over the path's rails (behaviour +468 the path).
#[derive(Clone, Debug)]
pub struct Route {
    /// the path's index in the race's array
    pub path: usize,
    rails: Vec<RouteRail>,
    i: usize,
    /// the phase on a curve rail (follower +176) and its curve speed (+180, 0: taken anew)
    t: f32,
    ds: f32,
    /// the rails' speed (m/s)
    pub speed: f32,
    /// the lane it goes on along and where (path +64 / +68)
    pub lane: usize,
    pub lane_t: f32,
}

impl Route {
    /// The rail's index and the phase along it (0 .. 1).
    fn at(&self) -> (usize, f32) {
        let f = match &self.rails[self.i] {
            RouteRail::Curve(..) => self.t,
            RouteRail::Turn(p) => p.t,
        };
        (self.i, f)
    }

    /// The point and heading where the car is.
    fn peek(&self) -> (Vec3, Vec3) {
        match &self.rails[self.i] {
            // (the stored pose: slot 9's point and dP/du, which are slot 8's, F-71868)
            RouteRail::Curve(c) => {
                let (p, d) = c.eval_pair(self.t);
                (p, d.normalize_or(Vec3::NEG_Z))
            }
            RouteRail::Turn(p) => p.at(p.t),
        }
    }

    /// The pose the follower stores (F-74567 on a curve rail: the curve speed +180 the mean
    /// of |dP/dt| here and the last value).
    fn pose(&mut self) -> (Vec3, Vec3) {
        let (p, d, sp) = match &mut self.rails[self.i] {
            // (slot 9, F-87587: the point, dP/du and the speed averaged with +180)
            RouteRail::Curve(c) => {
                let (p, d, s) = c.pose_slot9(self.t, self.ds);
                (p, d.normalize_or(Vec3::NEG_Z), Some(s))
            }
            RouteRail::Turn(p) => {
                let (q, d) = p.pose();
                (q, d, None)
            }
        };
        if let Some(sp) = sp {
            self.ds = sp;
        }
        (p, d)
    }

    /// The drawn steer's look on the rail the car is on (F-37033: slot 6 then slot 10).
    fn look(&self, hub_z: f32) -> Vec3 {
        match &self.rails[self.i] {
            RouteRail::Curve(c) => crate::railroute::deriv(c, steer_look(self.t, self.ds, hub_z)),
            RouteRail::Turn(p) => p.look(hub_z),
        }
    }
}

/// The phase on a lane: the link holding it and the metres along that link.
fn lane_link_at(nav: &NavSys, lanes: &Lanes, lane: usize, t: f32) -> Option<(usize, f32)> {
    let ls = lanes.links.get(lane)?;
    for (i, &k) in ls.iter().enumerate() {
        let l = &nav.links[k];
        if t <= l.t[1] || i + 1 == ls.len() {
            return Some((k, l.s_at(t)));
        }
    }
    None
}

/// One frame of a race path's car on the path's rails (state 9): the follower as on a lane
/// (F-50894; the phase by distance over the curve speed on the curve rail, by arc length on
/// the join rail), wanted speed max(10 mph, rail speed + offset) (F-70324), the car ahead
/// along the rest of this rail and the next (the next path rail or the lane). Past the last rail
/// the rest of the distance goes on along the path's lane from its phase (the lane-end turn
/// F-48618 returns the route's lane), then ordinary traffic (state 1). (The stop-line and
/// slower-lane entries are not made on these rails, as on the other run-time rails here.)
pub fn step_route(r: &mut Rider, env: &Env, me: usize, half_len: f32, dt: f32) -> (Vec3, Vec3) {
    let nav = env.nav;
    // (the speed offset's timer: `rider_offset`, after the frame's step)
    let ahead = {
        let rt = r.route.as_ref().unwrap();
        let (pos, fwd) = rt.peek();
        let cur = rt.rails[rt.i].clone();
        let (next_lane, next_start, b_start) = if rt.i + 1 < rt.rails.len() {
            (None, None, 0.0)
        } else {
            (Some(rt.lane), nav.curves.get(rt.lane).map(|c| c.eval(rt.lane_t).0), rt.lane_t)
        };
        // (the path's curve and join rails taken as run-time rails - F-47175's grid branch; the lane after them +192 at
        // the route lane's phase +196 (F-48618's +296); between the path's own rails +196 is not traced: 0)
        let run = match &cur {
            RouteRail::Curve(c) => RunRail::Curve(c),
            RouteRail::Turn(p) => p.geom.run(),
        };
        search_ahead(r, env, me, pos, fwd, Some(run), (None, next_lane), next_start, [Some(GatherRail::RunTime), next_lane.map(GatherRail::Lane), None], b_start)
    };
    let want = (r.route.as_ref().unwrap().speed + r.off).max(MIN_WANT);
    let (mut a, chosen) = control(r, r.v, want, half_len, &ahead, env.others, None, None, None, dt);
    r.slot = chosen;
    if env.hold {
        a = -r.ty.max_decel;
    }
    integrate(r, a, dt);
    let mut d = r.v * dt;
    if d > 0.0 {
        loop {
            let rt = r.route.as_mut().unwrap();
            let i = rt.i;
            match &mut rt.rails[i] {
                RouteRail::Curve(c) => {
                    // (+180 unset: the rail just set, F-48384's slot 9 at the phase with 0)
                    if rt.ds <= 0.0 {
                        rt.ds = set_rail_speed(c, rt.t);
                    }
                    let t0 = rt.t;
                    let t1 = mcla_formats::rage::navsys::LaneCurve::advance(t0, d, rt.ds);
                    if t1 <= 1.0 {
                        rt.t = t1;
                        break;
                    }
                    d = (t1 - 1.0) * d / (t1 - t0).max(1e-6);
                }
                RouteRail::Turn(p) => match p.step(d) {
                    None => break,
                    Some(rest) => d = rest,
                },
            }
            if rt.i + 1 < rt.rails.len() {
                // the next rail, from its start (F-50894's carry: set at 0, F-48384, the rest once at its slot 11
                // speed, clamped; the arc-line-arc rail's slot 11 is not read: there the rest goes on as before)
                rt.i += 1;
                rt.t = 0.0;
                rt.ds = 0.0;
                match &mut rt.rails[rt.i] {
                    RouteRail::Turn(p) => p.t = 0.0,
                    RouteRail::Curve(c) => {
                        rt.ds = set_rail_speed(c, 0.0);
                        rt.t = carry_phase(c, 0.0, d);
                        break;
                    }
                }
                continue;
            }
            // onto the path's lane at its phase: ordinary traffic from here
            let (lane, lt) = (rt.lane, rt.lane_t);
            if std::env::var_os("MCLA_RACEPATH_LOG").is_some() {
                log::info!("race path {}: car {} at the end of its rails, onto lane {lane} at {lt:.3} at {:.1} m/s", rt.path, r.id, r.v);
            }
            r.route = None;
            if let Some((link, _)) = lane_link_at(nav, env.lanes, lane, lt) {
                r.link = link;
                r.set_phase(nav, env.lanes, lt);
                let c = &nav.curves[lane];
                r.ds = set_rail_speed(c, lt);
                r.ds_lane = lane;
                if d > 0.0 {
                    r.set_phase(nav, env.lanes, carry_phase(c, lt, d));
                }
            }
            r.state = 1;
            r.on = [false; 3];
            r.choose_next_env(env);
            return r.pose(nav);
        }
    }
    r.route.as_mut().unwrap().pose()
}

/// What became of a race path car's request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSpawn {
    Done,
    /// no ground under the spawn matrix yet (the request waits up to its 3 s, F-29339)
    NoGround,
    /// not made, or taken out at once (F-51512 reason 17)
    Removed,
}

impl Vehicles {
    /// The shared random numbers (the MWC at 0x82860890) for the race script's draws.
    pub fn shared_mwc(&mut self) -> &mut super::parked::Mwc {
        &mut self.park_rng
    }

    /// A type for a random ambient path (F-52741 -> F-91280 on the traffic list,
    /// S +8556): the focus district's list at this hour among the loaded types of bound group
    /// `group`, taxis only when `taxis`.
    pub fn path_type_draw(&mut self, root: &std::path::Path, focus: Vec3, group: u32, taxis: bool) -> Option<String> {
        let names: Vec<String> = self.models.iter().map(|m| m.name.clone()).collect();
        for n in &names {
            if !self.amb_types.contains_key(n) {
                let t = AmbType::load(root, n);
                self.amb_types.insert(n.clone(), t);
            }
        }
        let r = self.park_rng.float();
        let (models, types) = (&self.models, &self.amb_types);
        let ok = |m: &str| models.iter().find(|x| x.name.eq_ignore_ascii_case(m)).and_then(|x| types.get(&x.name)).is_some_and(|t| t.bound_group == group && (taxis || !t.taxi));
        self.ambient.pick(focus, self.hour, r, &ok).map(|s| s.to_string())
    }

    /// The start speed (car vfunc 84 F-30683, called by the spawn slot 5 F-77422 for
    /// every ambient car - traffic too - and at a race path's trigger; listings
    /// work/people2/racepaths/v84.txt, work/people2/racepaths2/m.txt). `start`: the path's
    /// fPathStartSpeed x 0.44704 (0 when bParked, or referenced and not fired); None for a car
    /// without a path. f25 = the rail's speed + the car's offset (+424). No rail, a junction lane
    /// (the rail's owner or its parent flagged 4) or online: v = start, else f25, removed when
    /// under 0. Else the cars on the rail (F-90496; phase = behaviour +208): the one ahead
    /// (least phase ahead, else the least on the next rail) caps it at sqrt(|v_a|^2 + fMaxDecel
    /// (gap - 2)) with our fMaxDecel, the one behind (greatest phase behind) asks at least
    /// sqrt(|v_b|^2 - fMaxDecel_b (gap - 2)), gap = the centres' distance less both half lengths,
    /// a negative term removes; the light (`light_start_cap`); the slot test (`slot_clear`), which
    /// removes a car that is no cab and no trailer (nor car +500 bit 0x20: the cars of the ambient
    /// manager's second pool, F-86868 -> F-53584 with 1, which serves the bIsProxy
    /// types, F-12577 -> F-21879; such a car is never removed anyway).
    /// Then f25 = max(f25, 0); removed when the least is over the cap, over f25 or over the
    /// light's speed; v = min(start or f25, cap, light), removed when under the least. A bIsProxy
    /// type (+108) is never removed. Removed: None (F-51512, reason 17). `size`: the car's
    /// bound (width, length); `me`: the car's own index when it is in the list already.
    #[allow(clippy::too_many_arguments)]
    fn path_start_speed(&self, nav: &NavSys, rider: &Rider, pos: Vec3, size: (f32, f32), start: Option<f32>, me: Option<usize>) -> Option<f32> {
        let log = std::env::var_os("MCLA_RACEPATH_LOG").is_some() || std::env::var_os("MCLA_START_LOG").is_some();
        let proxy = rider.ty.proxy;
        let half_len = size.1 * 0.5;
        // f25: the rail's speed (vfunc 68) + the offset (+424)
        let f25 = match (&rider.route, rider.state) {
            (Some(rt), 9) => rt.speed,
            _ => rail_speed(nav, rider.link).0,
        } + rider.off;
        // where the car is: (rail key, phase); the next rail
        use RailKey as Key;
        let Some((here, ph)) = rail_key(nav, rider) else {
            // (no rail: f25 is 0)
            let v = start.unwrap_or(0.0);
            return (v >= 0.0 || proxy).then_some(v);
        };
        if matches!(here, Key::Lane(_)) && junction_lane(&nav.links[rider.link]) {
            let v = start.unwrap_or(f25);
            if v < 0.0 && !proxy {
                if log {
                    log::info!("  start: on a junction lane at {v:.2} < 0: removed");
                }
                return None;
            }
            return Some(v);
        }
        let next = match (&rider.route, here) {
            (Some(rt), Key::Path(p, i)) => Some(if i + 1 < rt.rails.len() { Key::Path(p, i + 1) } else { Key::Lane(rt.lane) }),
            _ => rider.next.map(|n| Key::Lane(nav.links[n].lane)),
        };
        // F-90496: the rail's car list (F-83161: in the order the cars took the rail,
        // the first 64, the car itself among them and then passed over): the greatest phase
        // under ours behind, the least over ours ahead (the first of equals); none ahead: the
        // least phase on the next rail's list (behaviour +192)
        let (mut ahead, mut behind): (Option<(usize, f32)>, Option<(usize, f32)>) = (None, None);
        for (j, f) in self.rail_list(nav, here, 64) {
            if Some(j) == me {
                continue;
            }
            if f < ph && behind.map_or(true, |b| f > b.1) {
                behind = Some((j, f));
            }
            if f > ph && ahead.map_or(true, |a| f < a.1) {
                ahead = Some((j, f));
            }
        }
        if ahead.is_none() {
            if let Some(nk) = next {
                for (j, f) in self.rail_list(nav, nk, 64) {
                    if Some(j) != me && ahead.map_or(true, |a| f < a.1) {
                        ahead = Some((j, f));
                    }
                }
            }
        }
        let gap = |j: usize| self.cars[j].state.pos.distance(pos) - half_len - half_size(&self.cars[j]).0;
        let mut why: Option<String> = None;
        // f24: the car ahead's cap
        let mut cap = None::<f32>;
        if let Some((j, _)) = ahead {
            let va = self.cars[j].state.vel.length_squared();
            let x = va + rider.ty.max_decel * (gap(j) - 2.0);
            if x < 0.0 {
                why.get_or_insert_with(|| format!("the car ahead {} gap {:.1} speed {:.1}: {x:.1} < 0", self.cars[j].model.name, gap(j), va.sqrt()));
            } else {
                cap = Some(x.sqrt());
            }
        }
        // f28: what the car behind asks
        let mut least = None::<f32>;
        if let Some((j, _)) = behind {
            let o = self.cars[j].rail.as_ref().unwrap();
            let x = self.cars[j].state.vel.length_squared() - o.ty.max_decel * (gap(j) - 2.0);
            if x < 0.0 {
                why.get_or_insert_with(|| format!("the car behind {} gap {:.1} speed {:.1}: {x:.1} < 0", self.cars[j].model.name, gap(j), self.cars[j].state.vel.length()));
            } else {
                least = Some(x.sqrt());
            }
        }
        // f26: the light
        let light = self.light_start_cap(nav, rider, pos, half_len, me);
        // the slot test (exempt: cabs, trailers, car +500 bit 0x20 = a car of the bIsProxy pool)
        if !rider.ty.cab && !rider.ty.trailer && !rider.ty.proxy && !self.slot_clear(nav, rider, size, me) {
            why.get_or_insert_with(|| "the slot is not clear".to_string());
        }
        let want = if f25 <= 0.0 { 0.0 } else { f25 };
        let mut lo = 0.0f32;
        if let Some(l) = least {
            if cap.is_some_and(|c| l > c) {
                why.get_or_insert_with(|| format!("the car behind asks {l:.2} over the cap {:.2}", cap.unwrap_or(0.0)));
            }
            if l > want {
                why.get_or_insert_with(|| format!("the car behind asks {l:.2} over the wanted {want:.2}"));
            }
            if light.is_some_and(|s| s < l) {
                why.get_or_insert_with(|| format!("the car behind asks {l:.2} over the light's {:.2}", light.unwrap_or(0.0)));
            }
            lo = l;
        }
        let mut v = start.unwrap_or(want);
        if let Some(c) = cap {
            if v - c >= 0.0 {
                v = c;
            }
        }
        if let Some(s) = light {
            if v - s >= 0.0 {
                v = s;
            }
        }
        if v < lo {
            why.get_or_insert_with(|| format!("{v:.2} under the least {lo:.2}"));
        }
        if let Some(w) = why {
            if log {
                log::info!("  start: {w}{}", if proxy { " (bIsProxy: kept)" } else { ": removed" });
            }
            if !proxy {
                return None;
            }
        }
        Some(v)
    }

    /// The light's start speed (F-30683 at A-11634): when the next rail (behaviour +192)
    /// is a junction lane (F-50269: its owner or the owner's parent flagged 4), its junction
    /// J (F-41621) lets the car go when the lane is green (F-78185: the lane's first
    /// span within 25 degrees of the current arm, timer < 6.5 s; `Junction::lane_state` 2) and J
    /// is not blocked (F-50959, `junction_blocked`). d = |the lane's first control point (rail
    /// vfunc 12) - the car| less half its length, at least 0; v = sqrt(vn^2 + 2 (fMaxDecel / 2) d),
    /// vn the lane's speed when the car may go, else 0: the speed that still stops at the line
    /// (or reaches the lane's speed there). None: no such lane. (J of a junction without arms:
    /// never green - the car is held to the stopping speed.)
    fn light_start_cap(&self, nav: &NavSys, rider: &Rider, pos: Vec3, half_len: f32, me: Option<usize>) -> Option<f32> {
        let nlink = match (&rider.route, rider.state) {
            (Some(rt), 9) => {
                if rt.i + 1 < rt.rails.len() {
                    return None;
                }
                lane_link_at(nav, lanes(nav), rt.lane, rt.lane_t)?.0
            }
            (None, 1) => rider.next?,
            _ => return None,
        };
        if !junction_lane(&nav.links[nlink]) {
            return None;
        }
        let lane = nav.links[nlink].lane;
        let ji = lane_junction(nav, lane)?;
        let (p0, _) = lane_entry(nav, nlink);
        let go = junction_green(&self.signals, nav, ji, nlink) && !self.junction_blocked(nav, ji, me);
        let d = (p0 - pos).length() - half_len;
        let dist = if -d >= 0.0 { 0.0 } else { d };
        let vn = if go { rail_speed(nav, nlink).0 } else { 0.0 };
        let x = vn * vn + 2.0 * (rider.ty.max_decel * 0.5) * dist;
        if std::env::var_os("MCLA_START_LOG").is_some() {
            log::info!("  start light: junction {ji} lane {lane} go {go} d {dist:.1} vn {vn:.2} -> {:.2}", x.max(0.0).sqrt());
        }
        (x >= 0.0).then(|| x.sqrt())
    }

    /// F-50959: is navsys junction `ji` blocked for a car to enter? (1) A driver's car (the
    /// vehicle manager 0x82874374's list: the player, racers, police) whose lane (racer +640,
    /// `DriverLanes::best`) is a lane of the junction (F-41621) and whose squared distance
    /// (+644) is under (4.5 x 0.667)^2; (2) the cars on the junction's rails (F-42526: its
    /// lanes, then its children's, each rail's car list in order, 64 at most altogether), not
    /// trailers, the car itself left out: one whose lane is not green (F-78185) and that
    /// moves (+412 not 0); or one standing inside the junction's box (centre +0x50 / +0x58,
    /// |axis +0x70 . d| < 0.4 x +0x3c and |axis +0x60 . d| < 0.4 x +0x40) that has no next rail,
    /// or whose next rail is the junction's and not green, or that is headed (its forward,
    /// F-50349) off the green arm (F-16432).
    fn junction_blocked(&self, nav: &NavSys, ji: usize, me: Option<usize>) -> bool {
        let b = self.junction_blockers(nav, ji, None);
        if std::env::var_os("MCLA_START_LOG").is_some() && b.players {
            log::info!("  start light: junction {ji} blocked by the player on one of its lanes");
        }
        match me {
            Some(m) => b.blocks(m),
            None => b.players || !b.cars.is_empty(),
        }
    }

    /// Everything F-50959 would call a hit on navsys junction `ji`, collected: `players`
    /// (part 1: the player's lane is one of the junction's, within 4.5 x 0.667 m) and the cars
    /// of part 2 (see `junction_blocked`), as `cars` indexes. The original returns at the first
    /// hit with the asking car left out of the list; taking the set and leaving the asker out
    /// gives the same answer (the asker holds its place in the 64 cap, as in the original's
    /// buffer). `lists`: the rail lists by lane (`rail_lists`) when the caller has them.
    ///
    /// Part 1 looks at the player only (mcPlayerManager's list; racers, hookmen and police are
    /// not in it). Part 2's `R+4 & 0x1800` branch (a run-time rail goes straight to the box
    /// test, no green or moving test) is not written: the port has no such rails in a
    /// junction's list (`rail_key` gives only lanes and race-path rails, whose junction lists
    /// are lane rails), so there is nothing to carry the flags yet.
    fn junction_blockers(&self, nav: &NavSys, ji: usize, lists: Option<&hashbrown::HashMap<usize, Vec<(u64, usize)>>>) -> Blockers {
        let mut out = Blockers::default();
        let near = 4.5f32 * f32::from_bits(0x3f2a_c083);
        for c in self.cars.iter() {
            if !c.is_player || c.rail.is_some() || c.parked.is_some() {
                continue;
            }
            let Some((lane, d2)) = self.driver_lanes.get(&c.uid).and_then(|d| d.best) else { continue };
            if lane_junction(nav, lane) == Some(ji) && d2 < near * near {
                out.players = true;
            }
        }
        let sj = signals_junction(&self.signals, nav, ji);
        let jb = junction_box(nav.junction_centers[ji]);
        let mut list: Vec<usize> = Vec::new();
        for &lane in &nav.junction_lanes[ji] {
            if list.len() >= 64 {
                break;
            }
            let room = 64 - list.len();
            match lists {
                Some(m) => list.extend(m.get(&lane).into_iter().flat_map(|v| v.iter().take(room)).map(|x| x.1)),
                None => list.extend(self.rail_list(nav, RailKey::Lane(lane), room).into_iter().map(|x| x.0)),
            }
        }
        for k in list {
            let x = &self.cars[k];
            let Some(r) = x.rail.as_ref() else {
                out.cars.push(k);
                continue;
            };
            if r.ty.trailer {
                continue;
            }
            if junction_green(&self.signals, nav, ji, r.link) {
                continue;
            }
            if r.v != 0.0 {
                out.cars.push(k);
                continue;
            }
            if !jb.is_some_and(|b| b.holds(x.state.pos)) {
                continue;
            }
            let Some(nl) = r.next else {
                out.cars.push(k);
                continue;
            };
            let blocked = if lane_junction(nav, nav.links[nl].lane) == Some(ji) {
                !junction_green(&self.signals, nav, ji, nl)
            } else {
                !sj.is_some_and(|j| self.signals.junctions[j].lane_state(x.state.forward()) == 2)
            };
            if blocked {
                out.cars.push(k);
            }
        }
        out
    }

    /// The rail lists by lane (rail +96, `rail_list`'s order: the order the cars took the rail),
    /// for the lanes any car holds, made once for a frame.
    fn rail_lists(&self, nav: &NavSys) -> hashbrown::HashMap<usize, Vec<(u64, usize)>> {
        let mut m: hashbrown::HashMap<usize, Vec<(u64, usize)>> = hashbrown::HashMap::new();
        for (j, c) in self.cars.iter().enumerate() {
            let Some(r) = c.rail.as_ref() else { continue };
            let Some((RailKey::Lane(l), _)) = rail_key(nav, r) else { continue };
            let seq = if r.rail_fresh || Some(RailKey::Lane(l)) == r.rail_key { r.rail_seq } else { u64::MAX };
            m.entry(l).or_default().push((seq, j));
        }
        for v in m.values_mut() {
            v.sort();
        }
        m
    }

    /// What the obstacle gather (F-47175) reads, made once a frame: per lane its car list (rail +96) - the riders on it
    /// (`rail_lists`) and the driver cars' entries (F-94578 links one in when it adds the lane), in the order they were
    /// linked in; per grid cell (`CrossGrid::UNBOUNDED`, 40 m) its cars in the port's car order.
    fn gather_lists(&self, nav: &NavSys) -> (hashbrown::HashMap<usize, Vec<usize>>, hashbrown::HashMap<(i32, i32), Vec<usize>>) {
        let mut m = self.rail_lists(nav);
        for (j, c) in self.cars.iter().enumerate() {
            if let Some(dl) = self.driver_lanes.get(&c.uid) {
                for &(lane, _, seq) in &dl.list {
                    m.entry(lane).or_default().push((seq, j));
                }
            }
        }
        let lists = m
            .into_iter()
            .map(|(l, mut v)| {
                v.sort();
                (l, v.into_iter().map(|x| x.1).collect())
            })
            .collect();
        let mut grid: hashbrown::HashMap<(i32, i32), Vec<usize>> = hashbrown::HashMap::new();
        for (j, c) in self.cars.iter().enumerate() {
            if let Some(cell) = CrossGrid::UNBOUNDED.cell(c.state.pos.x, c.state.pos.z) {
                grid.entry(cell).or_default().push(j);
            }
        }
        (lists, grid)
    }

    /// The blockers of every junction that has a car on its lanes or the player's lane, once a
    /// frame, for the riders' light (F-16850 -> F-50959(J, car)).
    fn frame_blockers(&self, nav: &NavSys) -> hashbrown::HashMap<usize, Blockers> {
        let lists = self.rail_lists(nav);
        let mut js: Vec<usize> = lists.keys().filter_map(|&l| lane_junction(nav, l)).collect();
        for c in self.cars.iter().filter(|c| c.is_player) {
            if let Some((lane, _)) = self.driver_lanes.get(&c.uid).and_then(|d| d.best) {
                js.extend(lane_junction(nav, lane));
            }
        }
        js.sort_unstable();
        js.dedup();
        js.into_iter()
            .filter_map(|ji| {
                let b = self.junction_blockers(nav, ji, Some(&lists));
                (b.players || !b.cars.is_empty()).then_some((ji, b))
            })
            .collect()
    }

    /// One step of the junctions' lights (F-86376 for all, F-96156's player demand):
    /// `blocked` = F-50959(J, 0) for the junctions whose timer passes 10 s this step, and
    /// the player's demand at the junction ahead of its lane (`player_junction_record`; none in
    /// the cutscene mode). Priority (F-42082): a race exists and the player is over 10 mph.
    /// Approximate: evaluated once after the riders moved (the original's order against the
    /// ambient manager update is not traced).
    pub fn step_signals(&mut self, nav: &NavSys, dt: f32, race: Option<(f32, f32)>) {
        let nav = rail_nav(nav);
        let mut blocked = vec![false; self.signals.junctions.len()];
        let mut lists = None;
        for nj in 0..nav.junction_centers.len() {
            let Some(ji) = signals_junction(&self.signals, nav, nj) else { continue };
            let j = &self.signals.junctions[ji];
            // (F-50959 is asked only past 10 s, and not while forced or the red light race byte holds the timer)
            if j.forced != -1 || self.signals.red_light_race || j.timer + dt <= crate::signals::PHASE {
                continue;
            }
            let lists = lists.get_or_insert_with(|| self.rail_lists(nav));
            let b = self.junction_blockers(nav, nj, Some(lists));
            blocked[ji] = b.players || !b.cars.is_empty();
        }
        let mut demand = None;
        let mut player = None;
        let mut player_j = None;
        let mut player_rec = None;
        if let Some(c) = self.cars.iter().find(|c| c.is_player) {
            player = Some((c.state.pos, c.state.vel));
            if self.amb_cutscene.is_none() {
                if let Some(dl) = self.driver_lanes.get(&c.uid) {
                    if let Some((lane, _)) = dl.best {
                        // (the report's tangent +624)
                        let tangent = dl.best_tan;
                        let reverse = !(c.state.forward().dot(tangent) > 0.0);
                        player_rec = player_junction_record(nav, lane, reverse);
                        player_j = player_rec.map(|r| r.0);
                        if let Some(ji) = player_j.and_then(|nj| signals_junction(&self.signals, nav, nj)) {
                            let mph = c.state.vel.length() * 2.236_936_3;
                            demand = Some((ji, c.state.pos, race.is_some() && mph > 10.0));
                        }
                    }
                }
            }
        }
        self.player_junction = player_j;
        self.player_record = player_rec;
        self.signals.step(dt, &blocked, demand, player, race);
    }

    /// The cars on a rail in its list's order (rail +96: the order they took it - `rail_seq`;
    /// one that took it this frame, not stamped yet, last), the first `cap`, with their phases.
    fn rail_list(&self, nav: &NavSys, key: RailKey, cap: usize) -> Vec<(usize, f32)> {
        let mut v: Vec<(u64, usize, f32)> = self
            .cars
            .iter()
            .enumerate()
            .filter_map(|(j, c)| {
                let r = c.rail.as_ref()?;
                let (k, f) = rail_key(nav, r)?;
                (k == key).then(|| (if r.rail_fresh || Some(k) == r.rail_key { r.rail_seq } else { u64::MAX }, j, f))
            })
            .collect();
        v.sort_by_key(|x| (x.0, x.1));
        v.truncate(cap);
        v.into_iter().map(|x| (x.1, x.2)).collect()
    }

    /// Stamp the cars that took another rail this frame (the tail of the new rail's list).
    fn stamp_rails(&mut self, nav: &NavSys) {
        for c in self.cars.iter_mut() {
            let Some(r) = c.rail.as_mut() else { continue };
            let k = rail_key(nav, r).map(|x| x.0);
            if r.rail_fresh {
                r.rail_fresh = false;
                r.rail_key = k;
            } else if k != r.rail_key {
                r.rail_key = k;
                if k.is_some() {
                    r.rail_seq = next_rail_seq();
                }
            }
        }
    }

    /// The driver cars' lanes, once a frame (F-80793 -> the obstacle's vfunc 11
    /// F-58331 with the flag mcRacer vfunc 10 = 1): w = the car's radius (racer +480 =
    /// sqrt(hx^2 + hz^2), hx / hz the greater magnitude of the bound box's least and greatest x /
    /// z, F-57218) + 4.5 x 0.5; the lane quadtree's lanes within w at the car's place (flags
    /// & 1, 48 at most: F-53328); one entry of the car's list drawn (the shared MWC,
    /// (lo & 0x7FFFFFFF) % n: the racer's obstacle has no car at +16): when the query has its
    /// lane and the lane's nearest point (slot 3, F-15621) is within w its phase is kept,
    /// else the entry goes (the last moved into its place, F-59811); then one of the
    /// query's lanes drawn, unless it was the one just checked: in the list - kept or dropped
    /// as before; not - added with its phase when within w and the list has fewer than 12
    /// (F-94578). The lane reported: the entry whose point at its kept phase is nearest
    /// (squared distance, the first of equals).
    fn update_driver_lanes(&mut self, nav: &NavSys) {
        let mut live: Vec<u32> = Vec::new();
        for i in 0..self.cars.len() {
            let c = &self.cars[i];
            if c.rail.is_some() || c.parked.is_some() {
                continue;
            }
            live.push(c.uid);
            let (lo, hi) = c.model.params.composite_box();
            let (hx, hz) = (lo.x.abs().max(hi.x.abs()), lo.z.abs().max(hi.z.abs()));
            let w = 4.5f32.mul_add(0.5, hz.mul_add(hz, hx * hx).sqrt());
            let p = c.state.pos;
            let uid = c.uid;
            let found = nav.lanes_near(p, w, 1, 48);
            let near = |lane: usize| -> (f32, f32) {
                let cv = &nav.curves[lane];
                let n = if lane_boxes(nav)[lane].len >= 450.0 { 8 } else { 4 };
                let (t, d2, _) = curve_nearest(cv, p, n, 10);
                (t, d2)
            };
            let mut dl = self.driver_lanes.remove(&uid).unwrap_or_default();
            let mut checked: Option<usize> = None;
            if !dl.list.is_empty() {
                let k = self.park_rng.index(dl.list.len() as u32) as usize;
                let lane = dl.list[k].0;
                match found.iter().position(|&l| l == lane) {
                    Some(fi) => {
                        checked = Some(fi);
                        let (t, d2) = near(lane);
                        if d2 < w * w {
                            dl.list[k].1 = t;
                        } else {
                            dl.list.swap_remove(k);
                        }
                    }
                    None => {
                        dl.list.swap_remove(k);
                    }
                }
            }
            if !found.is_empty() {
                let fi = self.park_rng.index(found.len() as u32) as usize;
                if checked != Some(fi) {
                    let lane = found[fi];
                    let (t, d2) = near(lane);
                    let valid = d2 < w * w;
                    match dl.list.iter().position(|e| e.0 == lane) {
                        Some(k) => {
                            if valid {
                                dl.list[k].1 = t;
                            } else {
                                dl.list.swap_remove(k);
                            }
                        }
                        None => {
                            if valid && dl.list.len() < 12 {
                                dl.list.push((lane, t, next_rail_seq()));
                            }
                        }
                    }
                }
            }
            // (F-58331's report: each listed lane's slot 8 at its phase - F-71868, the point and dP/du -, d^2 to the car
            // as dy^2, dz fused, dx fused, the first strictly nearest from 3.4e38 in the list's order; its tangent the
            // vector unit's normalisation of dP/du)
            let mut best: Option<(usize, f32)> = None;
            let mut best_tan = Vec3::ZERO;
            for &(lane, t, _) in &dl.list {
                let (q, d) = nav.curves[lane].eval_pair(t);
                let (dy, dz, dx) = (q.y - p.y, q.z - p.z, q.x - p.x);
                let d2 = crate::physics::exact::fma(dx, dx, crate::physics::exact::fma(dz, dz, dy * dy));
                if d2 < best.map_or(f32::MAX, |b| b.1) {
                    best = Some((lane, d2));
                    best_tan = crate::tcp_rail::vnorm(d);
                }
            }
            dl.best_tan = best_tan;
            if std::env::var_os("MCLA_DRIVER_LANE_LOG").is_some() && best.map(|b| b.0) != dl.best.map(|b| b.0) {
                log::info!("driver car {uid}: lane {:?} ({} listed, {} near, w {w:.2})", best.map(|b| (b.0, b.1.sqrt())), dl.list.len(), found.len());
            }
            dl.best_t = best.and_then(|b| dl.list.iter().find(|e| e.0 == b.0)).map_or(0.0, |e| e.1);
            dl.best = best;
            self.driver_lanes.insert(uid, dl);
        }
        self.driver_lanes.retain(|k, _| live.contains(k));
    }

    /// The slot test F-12899 (with F-84149), only on a navsys lane (rail flags & 0x1800
    /// == 0; the path's rails pass): P = the lane's point at the car's phase (rail vfunc 7). The
    /// lanes of the navsys vertex table (root +0x38) at the lane's last control point's vertex -
    /// the lane itself among them - and at its first one but the lane itself: every ambient car
    /// registered on such a lane (its follower's rail, lane +96), the car itself left out, within
    /// 5 m of P in height, blocks when its xz distance to P is under the mean of the two cars'
    /// lengths - for this lane and a lane joined head to tail (one's first control point the
    /// other's last) - or of their widths (any other lane). True: clear.
    fn slot_clear(&self, nav: &NavSys, rider: &Rider, size: (f32, f32), me: Option<usize>) -> bool {
        if rider.state != 1 || rider.route.is_some() {
            return true;
        }
        let lane = nav.links[rider.link].lane;
        let Some(c) = nav.curves.get(lane).filter(|c| !c.pts.is_empty()) else { return true };
        let (first, last) = (c.pts[0], *c.pts.last().unwrap());
        let p = rider.pos(nav);
        let ends = lane_ends(nav);
        let key = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        let mut rails: Vec<usize> = ends.get(&key(last)).cloned().unwrap_or_default();
        rails.extend(ends.get(&key(first)).map_or(&[][..], |v| v.as_slice()).iter().copied().filter(|&o| o != lane));
        for o in rails {
            let oc = &nav.curves[o];
            let joined = o == lane || oc.pts.first() == Some(&last) || oc.pts.last() == Some(&first);
            for (k, x) in self.cars.iter().enumerate() {
                if Some(k) == me || !x.rail.as_ref().is_some_and(|r| r.state == 1 && r.route.is_none() && nav.links[r.link].lane == o) {
                    continue;
                }
                let q = x.state.pos;
                if (q.y - p.y).abs() > 5.0 {
                    continue;
                }
                let d2 = (q.x - p.x) * (q.x - p.x) + (q.z - p.z) * (q.z - p.z);
                let (hl, hw) = half_size(x);
                let (f1, f0) = if joined { (2.0 * hl, size.1) } else { (2.0 * hw, size.0) };
                let m = (f1 + f0) * 0.5;
                if d2 < m * m {
                    if std::env::var_os("MCLA_START_LOG").is_some() {
                        log::info!("  slot: {} on lane {o} ({}) {:.2} m from the spot", x.model.name, if joined { "joined" } else { "beside" }, d2.sqrt());
                    }
                    return false;
                }
            }
        }
        true
    }

    /// The rider of a race path's car on its route (state 9; on the lane at once when the route
    /// has no rails), its type's top speed fPathMaxSpeed for life (F-58614 reads behaviour
    /// +244 for the type's fMaxSpeed).
    fn path_rider(&mut self, nav: &NavSys, ty: AmbType, pr: &PathRails, path: usize) -> Option<Rider> {
        let lanes = lanes(nav);
        let route = pr.route(path)?;
        let (link, s) = lane_link_at(nav, lanes, route.lane, route.lane_t)?;
        let mut ty = ty;
        ty.max_speed = pr.speed;
        let seed = (self.rand() * 16777216.0) as u32 | 1;
        let mut r = Rider::new(nav, ty, link, s, seed);
        r.set_phase(nav, lanes, route.lane_t);
        r.ds_lane = usize::MAX;
        r.choose_next(nav, &mut self.park_rng);
        r.born = self.clock;
        r.race_lane = (route.rails.len() < 2 && nav.links[link].flags & 0x1800 == 0).then_some(route.lane);
        if !route.rails.is_empty() {
            r.route = Some(Box::new(route));
            r.state = 9;
        }
        r.phys_pair = pr.physical;
        r.quiet = pr.quiet;
        Some(r)
    }

    /// The follower of item `h` for its race path (F-45833 with the path's params: the
    /// type by the item's name, the colour `shade` of the type's twenty; placed at the spawn
    /// matrix with no distance or visibility test once the ground is found there), its start
    /// speed fPathStartSpeed (0 when `standing`: bParked, or referenced and not fired) within
    /// its limits.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_path_car(&mut self, gpu: &crate::render::Gpu, world: &crate::world::World, h: u32, shade: usize, pr: &PathRails, path: usize, start_mph: f32, standing: bool) -> PathSpawn {
        let Some(nav) = world.nav.as_ref().map(rail_nav) else { return PathSpawn::Removed };
        let Some(item) = self.layout.get(h as usize).cloned().flatten() else { return PathSpawn::Removed };
        let Some(mi) = self.models.iter().position(|m| m.name.eq_ignore_ascii_case(&item.name)) else { return PathSpawn::Removed };
        let Some((ground, normal)) = probe_ground(&world.collider, pr.spawn) else { return PathSpawn::NoGround };
        let mname = self.models[mi].name.clone();
        let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| AmbType::load(&world.assets.root, &mname));
        let Some(mut rider) = self.path_rider(nav, ty, pr, path) else { return PathSpawn::Removed };
        rider.path_referenced = item.path.is_some_and(|p| p.referenced);
        let model = self.models[mi].clone();
        let size = (if ty.width > 0.0 { ty.width } else { model.max.x - model.min.x }, if ty.length > 0.0 { ty.length } else { model.max.z - model.min.z });
        let v = if standing { 0.0 } else { start_mph * MPH };
        let Some(v) = self.path_start_speed(nav, &rider, pr.spawn, size, Some(v), None) else {
            if std::env::var_os("MCLA_RACEPATH_LOG").is_some() { log::info!("race path {path}: car removed at its start (the car ahead or behind)"); }
            return PathSpawn::Removed;
        };
        rider.v = v;
        let shade = test_shade(shade);
        let color = ty.color(shade).unwrap_or(super::TRAFFIC_PALETTE[shade % super::TRAFFIC_PALETTE.len()]);
        let dir = Vec3::new(-pr.row2.x, 0.0, -pr.row2.z).normalize_or(Vec3::NEG_Z);
        let yaw = (-dir.x).atan2(-dir.z);
        let idx = self.spawn(gpu, model, ground, yaw, color, false);
        let f = (dir - normal * dir.dot(normal)).normalize_or(dir);
        let right = f.cross(normal).normalize_or(Vec3::X);
        self.cars[idx].state.relocate(ground, Quat::from_mat3(&glam::Mat3::from_cols(right, normal, -f)).normalize());
        rider.seat = Some(Seat::new(ground, normal, f));
        // (the request's seed and the init draws, `amb_init`)
        let init = self.amb_init(idx, &ty, false);
        (rider.life, rider.off_timer, rider.wait, rider.bonus, rider.mwc) = (init.life, init.off_timer, init.wait, init.bonus, init.car);
        let c = &mut self.cars[idx];
        c.item = Some(h);
        c.rail = Some(rider);
        if let Some(ai) = c.ai.as_mut() {
            ai.link = None;
        }
        if std::env::var_os("MCLA_RACEPATH_LOG").is_some() { log::info!("race path {path}: item {h} {} follows its path from {:?} at {:.1} m/s (rails {}, top {:.1} m/s)", c.model.name, ground, v, c.rail.as_ref().map_or(0, |r| r.route.as_ref().map_or(0, |rt| rt.rails.len())), pr.speed); }
        PathSpawn::Done
    }

    /// The parked placement of item `h` (F-90920, as parked.rs places an item's car): at its
    /// place and heading, no behaviour.
    fn place_path_parked(&mut self, gpu: &crate::render::Gpu, root: &std::path::Path, h: u32) -> PathSpawn {
        let Some(item) = self.layout.get(h as usize).cloned().flatten() else { return PathSpawn::Removed };
        let Some(mi) = self.models.iter().position(|m| m.name.eq_ignore_ascii_case(&item.name)) else { return PathSpawn::Removed };
        let shade = test_shade(self.park_rng.index(20) as usize);
        let mname = self.models[mi].name.clone();
        let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| AmbType::load(root, &mname));
        let color = ty.color(shade).unwrap_or(super::TRAFFIC_PALETTE[shade % super::TRAFFIC_PALETTE.len()]);
        let model = self.models[mi].clone();
        let idx = self.spawn(gpu, model, item.pos, item.heading, color, false);
        // (the request's seed and the init draws, `amb_init`: its life and lights' offsets. An item has no spot, so
        // F-77422 draws no wheel turn and leaves +3584 as it was - what the pooled car holds there is not traced; the
        // port's own draw stays)
        self.amb_init(idx, &ty, false);
        let steer = parked_steer(1.0 - self.rand(), self.rand());
        let c = &mut self.cars[idx];
        c.ai = None;
        c.parked = Some(super::parked::ITEM_SPOT);
        c.item = Some(h);
        c.park_steer = steer;
        for wh in c.state.wheels.iter_mut().take(2) {
            wh.steer = steer;
        }
        c.settle = super::PARK_PROBE;
        PathSpawn::Done
    }

    /// A trigger fired race path `path` whose item is `h` (F-53279): the item's path is
    /// marked fired. A car there takes the path's params (vfunc 84): the waiting car placed
    /// parked (referenced, not fired, not bParked: it has its behaviour and stood) drives off on
    /// its route at fPathStartSpeed; a bParked car stays; a car following the path is set on its
    /// route anew. No car (and no request pending): the item's car at once - parked when
    /// bParked, else the follower at the spawn matrix when the path has its lane. Returns the
    /// colour drawn and what became of a new follower.
    #[allow(clippy::too_many_arguments)]
    pub fn fire_path(&mut self, gpu: &crate::render::Gpu, world: &crate::world::World, h: u32, pr: &PathRails, path: usize, start_mph: f32, parked: bool, pending: bool) -> Option<(usize, PathSpawn)> {
        if let Some(it) = self.layout_item_mut(h) {
            if let Some(p) = it.path.as_mut() {
                p.fired = true;
            }
        }
        if pending {
            return None;
        }
        let Some(nav) = world.nav.as_ref().map(rail_nav) else { return None };
        if let Some(k) = self.cars.iter().position(|c| c.item == Some(h)) {
            if parked && self.cars[k].rail.is_none() {
                return None;
            }
            let mname = self.cars[k].model.name.clone();
            let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| AmbType::load(&world.assets.root, &mname));
            let mut rider = match self.cars[k].rail.take() {
                // a follower: its route anew (its other state kept)
                Some(mut r) => match pr.route(path) {
                    Some(rt) => {
                        let lanes = lanes(nav);
                        if let Some((link, _)) = lane_link_at(nav, lanes, rt.lane, rt.lane_t) {
                            r.link = link;
                            r.set_phase(nav, lanes, rt.lane_t);
                            r.ds_lane = usize::MAX;
                            r.choose_next(nav, &mut self.park_rng);
                        }
                        r.path = None;
                        r.quiet = pr.quiet;
                        if rt.rails.is_empty() {
                            r.route = None;
                            r.state = 1;
                        } else {
                            r.route = Some(Box::new(rt));
                            r.state = 9;
                        }
                        r
                    }
                    None => {
                        self.cars[k].rail = Some(r);
                        return None;
                    }
                },
                None => match self.path_rider(nav, ty, pr, path) {
                    Some(mut r) => {
                        let c = &self.cars[k];
                        r.seat = Some(Seat::new(c.state.pos, c.state.up(), c.state.forward()));
                        r
                    }
                    None => return None,
                },
            };
            let pos = self.cars[k].state.pos;
            let (hl, hw) = half_size(&self.cars[k]);
            match self.path_start_speed(nav, &rider, pos, (2.0 * hw, 2.0 * hl), Some(start_mph * MPH), Some(k)) {
                Some(v) => rider.v = v,
                None => {
                    if std::env::var_os("MCLA_RACEPATH_LOG").is_some() { log::info!("race path {path}: fired car removed (the car ahead or behind)"); }
                    self.cars.remove(k);
                    return Some((0, PathSpawn::Removed));
                }
            }
            let c = &mut self.cars[k];
            c.parked = None;
            c.park_held = false;
            c.amb_hit = false;
            c.park_steer = 0.0;
            for w in c.state.wheels.iter_mut() {
                w.steer = 0.0;
            }
            if std::env::var_os("MCLA_RACEPATH_LOG").is_some() { log::info!("race path {path}: item {h} {} fired, drives off at {:.1} m/s", c.model.name, rider.v); }
            // (a trigger fired it: its path is referenced, byte +188)
            rider.path_referenced = true;
            c.rail = Some(rider);
            return Some((0, PathSpawn::Done));
        }
        if parked {
            return Some((0, self.place_path_parked(gpu, &world.assets.root, h)));
        }
        if pr.lane.is_none() {
            return None;
        }
        let shade = self.park_rng.index(20) as usize;
        Some((shade, self.spawn_path_car(gpu, world, h, shade, pr, path, start_mph, false)))
    }

    /// Race_UnloadAmbientPathArray (path vfunc F-47222: the item removed, reason 11): a car
    /// riding a lane goes with its item; any other (on the path's rails or a recovery rail,
    /// physical, placed parked) is let go of - no item, no path rails - and stays.
    pub fn unload_path_items(&mut self, handles: &[u32]) {
        for &h in handles {
            for c in self.cars.iter_mut().filter(|c| c.item == Some(h)) {
                if c.rail.as_ref().is_some_and(|r| r.state == 1) {
                    continue;
                }
                c.item = None;
                if let Some(r) = c.rail.as_mut() {
                    if r.route.take().is_some() || r.state == 9 {
                        // behaviour vfunc 11 F-59895 clears the follower's rails that were
                        // the path's (F-78813, F-55520): with no current rail
                        // F-50894 does not advance and F-74567 leaves the pose flag
                        // 0x80 clear, so the car is not posed (slot 20 / F-86858 skipped)
                        // and stands; nothing recovers it - the unspawner takes it (off a lane)
                        r.state = 10;
                        r.path = None;
                    }
                }
            }
            self.remove_layout_item(h);
        }
        self.item_followers_due.retain(|(h, _)| !handles.contains(h));
    }
}

#[cfg(test)]
mod road_box_dist_tests {
    /// The road rectangle distance against the original's F-15989 (tools/road_box_dist_reference.py, run offline):
    /// the result's bits equal.
    #[test]
    fn road_box_dist_matches_the_original() {
        use glam::Vec3;
        let f = |w: &str| f32::from_bits(u32::from_str_radix(&w.replace('_', ""), 16).unwrap());
        let mut n = 0;
        for line in include_str!("testdata/road_box_dist_ref.txt").lines() {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let c: Vec<f32> = inp.split(' ').map(f).collect();
            let v = |i: usize| Vec3::new(c[i], c[i + 1], c[i + 2]);
            let got = super::road_box_dist(v(0), v(3), v(6), c[9], c[10]);
            assert_eq!(got.to_bits(), f(out).to_bits(), "{line}: {got}");
            n += 1;
        }
        assert!(n >= 4000);
    }
}

#[cfg(test)]
mod box_centre_tests {
    /// The spawn box's centre against the original's F-97927 (tools/box_centre_reference.py, 4,000 cases: speeds and the
    /// clamp's edges, the across-the-road flag, shipped and random tune pairs, views of any length): the centre's x and z
    /// bits equal (the port's box is flat: y is not kept).
    #[test]
    fn box_centre_matches_the_original() {
        use super::{box_centre, box_lerp, box_t};
        let f = |w: &str| f32::from_bits(u32::from_str_radix(&w.replace('_', ""), 16).unwrap());
        let mut n = 0;
        for line in include_str!("testdata/box_centre_ref.txt").lines() {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let c: Vec<&str> = inp.split(' ').collect();
            let o: Vec<f32> = out.split(' ').map(f).collect();
            let pair = if c[1] == "00000001" { [f(c[4]), f(c[5])] } else { [f(c[2]), f(c[3])] };
            let off = box_lerp(pair, box_t(f(c[0])));
            let got = box_centre(glam::Vec2::new(f(c[9]), f(c[11])), glam::Vec2::new(f(c[6]), f(c[8])), off);
            assert_eq!((got.x.to_bits(), got.y.to_bits()), (o[0].to_bits(), o[2].to_bits()), "{line}");
            n += 1;
        }
        assert_eq!(n, 4000);
    }
}

#[cfg(test)]
mod verify_small_tests {
    //! FORMATS_TOOLS verification pass (docs/reference/formats/verify_small_evidence.md): the original's
    //! unchanged code (work/verify-small/FORMATS_TOOLS/stage1.py, offline) against the port. The original's
    //! outputs are held as 64-bit FNV-1a hashes of the u32 word streams (no game data in the repo); the
    //! synthetic cases come from a splitmix64 generator mirrored in the script (gen.py).
    use super::*;
    use glam::Vec2;

    fn fnv(h: &mut u64, w: u32) {
        for b in w.to_le_bytes() {
            *h = (*h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn uni(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * ((self.next() >> 11) as f64 / 9007199254740992.0)
        }
        fn randint(&mut self, a: i64, b: i64) -> i64 {
            a + (self.next() % (b - a + 1) as u64) as i64
        }
        fn angle(&mut self) -> Vec2 {
            let a = self.uni(-1.0, 1.0);
            let b = self.uni(-1.0, 1.0);
            let n = (a * a + b * b).sqrt();
            if n < 1e-3 {
                Vec2::new(1.0, 0.0)
            } else {
                Vec2::new((a / n) as f32, (b / n) as f32)
            }
        }
    }

    type Case = (Vec2, Vec2, Vec2, Vec2, Vec2, Vec2);

    /// The 33,000 rectangle pairs of gen.py (random, axis aligned touching +-1 ulp, crosses and nested,
    /// diamonds at the corner-touch distance).
    fn rect_cases() -> Vec<Case> {
        let mut r = Rng(0x4D434C4152454354);
        let mut v: Vec<Case> = Vec::new();
        let f = |x: f64| x as f32;
        for _ in 0..20000 {
            let s = [1.0f64, 10.0, 100.0, 1000.0][(r.next() % 4) as usize];
            let u1 = r.angle();
            let u2 = r.angle();
            let h1 = Vec2::new(f(r.uni(0.05, s)), f(r.uni(0.05, s)));
            let h2 = Vec2::new(f(r.uni(0.05, s)), f(r.uni(0.05, s)));
            let c1 = Vec2::new(f(r.uni(-s, s)), f(r.uni(-s, s)));
            let c2 = Vec2::new(f(r.uni(-s, s)), f(r.uni(-s, s)));
            v.push((c1, u1, h1, c2, u2, h2));
        }
        let up = |x: f32, k: i32| f32::from_bits((x.to_bits() as i32).wrapping_add(k) as u32);
        for _ in 0..3000 {
            let h1 = [r.randint(1, 20) as f64, r.randint(1, 20) as f64];
            let h2 = [r.randint(1, 20) as f64, r.randint(1, 20) as f64];
            let c1 = [r.randint(-50, 50) as f64, r.randint(-50, 50) as f64];
            let ax = (r.next() % 2) as usize;
            let sgn = [-1.0f64, 1.0][(r.next() % 2) as usize];
            let gap = if ax == 0 { h1[0] + h2[0] } else { h1[1] + h2[1] };
            let mut c2 = c1;
            let r5 = r.randint(-5, 5);
            c2[1 - ax] += [0.0, 1.0, -1.0, r5 as f64][(r.next() % 4) as usize];
            let e = [0.0, 0.0, 1e-5, -1e-5][(r.next() % 4) as usize];
            c2[ax] += sgn * (gap + e);
            let k = [-1, 0, 0, 1][(r.next() % 4) as usize];
            c2[ax] = up(f(c2[ax]), k) as f64;
            let x = Vec2::new(1.0, 0.0);
            v.push((Vec2::new(c1[0] as f32, c1[1] as f32), x, Vec2::new(h1[0] as f32, h1[1] as f32), Vec2::new(c2[0] as f32, c2[1] as f32), x, Vec2::new(h2[0] as f32, h2[1] as f32)));
        }
        for _ in 0..2000 {
            let hh = f(r.uni(1.0, 30.0));
            let w = f(r.uni(0.1, 1.0));
            let c = Vec2::new(f(r.uni(-9.0, 9.0)), f(r.uni(-9.0, 9.0)));
            let (x, y) = (Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0));
            v.push((c, x, Vec2::new(hh, w), c, y, Vec2::new(hh, w)));
            let dx = r.uni(-1.0, 1.0);
            let dy = r.uni(-1.0, 1.0);
            let c2 = Vec2::new(f(c.x as f64 + dx), f(c.y as f64 + dy));
            let u = r.angle();
            v.push((c, x, Vec2::new(hh, w), c2, u, Vec2::new(w, hh)));
            let u = r.angle();
            v.push((c, x, Vec2::new(hh, hh), c, u, Vec2::new(w, w)));
            let u = r.angle();
            v.push((c, x, Vec2::new(w, w), c, u, Vec2::new(hh, hh)));
        }
        let r2 = f(0.5f64.sqrt());
        let s2 = 2.0f64.sqrt();
        for _ in 0..1000 {
            let h = r.randint(1, 10) as f64;
            let e = [0.0, 1e-5, -1e-5, 1e-3, -1e-3][(r.next() % 5) as usize];
            let d = h + h * s2 + e;
            let hv = Vec2::new(h as f32, h as f32);
            let x = Vec2::new(1.0, 0.0);
            v.push((Vec2::ZERO, x, hv, Vec2::new(f(d), 0.0), Vec2::new(r2, r2), hv));
            v.push((Vec2::ZERO, x, hv, Vec2::new(f(d / s2), f(d / s2)), Vec2::new(r2, r2), hv));
        }
        v
    }

    /// F-27898 (the rail speed of a lane): the original on every lane object of the city
    /// navsys (10,031 lanes: the owner / road found by its own road_of, flags, +0x10 word, the road's
    /// limit) against `rail_speed` of the lane's first link, and on 128 owner-flag words x 2 values of
    /// the owner's +0x10 word with no road (port: no zone) and a lane without an owner.
    #[test]
    #[ignore = "requires user's extracted data (MCLA_DATA_DIR)"]
    fn rail_speed_matches_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).unwrap();
        let nav = NavSys::parse(&res).unwrap();
        let mut first_link = vec![usize::MAX; nav.curves.len()];
        for (k, l) in nav.links.iter().enumerate().rev() {
            first_link[l.lane] = k;
        }
        let mut h = 0xcbf29ce484222325u64;
        let mut differing = 0;
        for lane in 0..first_link.len() {
            fnv(&mut h, rail_speed(&nav, first_link[lane]).0.to_bits());
        }
        // lanes whose owner is not a road but has a road as parent: the original's road_of finds the road
        for (lane, &k) in first_link.iter().enumerate() {
            let l = &nav.links[k];
            if l.zone.is_none() && l.owner_flags & 2 == 0 && l.parent_zone.is_some() {
                differing += 1;
                let _ = lane;
            }
        }
        println!("rail_speed: {} lanes; {} with a parent road and no road of their own", first_link.len(), differing);
        assert_eq!(h, 0x0f61e061dfe2d429, "rail speed of the 10,031 lanes");
        // F-36686 (the lane box): min / max over the control vertices from the curve accessor
        let mut bh = 0xcbf29ce484222325u64;
        for b in lane_boxes(&nav) {
            for v in [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z] {
                fnv(&mut bh, v.to_bits());
            }
        }
        assert_eq!(bh, 0x2935e7d6e6ed9002, "lane boxes of the 10,031 lanes");
        let mut nav = nav;
        nav.links[0].zone = None;
        nav.links[0].parent_zone = None;
        let mut sh = 0xcbf29ce484222325u64;
        let mut n = 0;
        for f in 0..128u32 {
            for w in 0..2u32 {
                nav.links[0].owner_flags = f;
                nav.links[0].owner_w10 = w;
                fnv(&mut sh, rail_speed(&nav, 0).0.to_bits());
                n += 1;
            }
        }
        nav.links[0].owner_flags = 0;
        nav.links[0].owner_w10 = 0;
        fnv(&mut sh, rail_speed(&nav, 0).0.to_bits());
        assert_eq!(n, 256);
        assert_eq!(sh, 0x671d93d903813d8a, "synthetic owner flags");
    }

    /// F-43122 (the segment clip) is the helper of F-71037: the rectangle test
    /// `rects_meet` against the original's whole test (corners of each in the other's box, then the four
    /// edges against the box) on 33,000 pairs. Prints the number of pairs that differ.
    #[test]
    fn rects_meet_against_the_original() {
        let cases = rect_cases();
        assert_eq!(cases.len(), 33000);
        let (mut ch, mut oh) = (0xcbf29ce484222325u64, 0xcbf29ce484222325u64);
        let mut hits = 0;
        for (c1, u1, h1, c2, u2, h2) in &cases {
            for v in [c1.x, c1.y, u1.x, u1.y, h1.x, h1.y, c2.x, c2.y, u2.x, u2.y, h2.x, h2.y] {
                fnv(&mut ch, v.to_bits());
            }
            let m = rects_meet(*c1, *u1, *h1, *c2, *u2, *h2);
            hits += m as usize;
            fnv(&mut oh, m as u32);
        }
        println!("rects_meet: {hits} of {} pairs meet", cases.len());
        assert_eq!(ch, 0x59a082b80219e753, "case generator");
        assert_eq!(oh, 0x00c52807c8002dc4, "rects_meet outputs");
    }
}

/// F-86418 / F-80887 / F-42351: the path curve's builder, constructor and knot padding against the original's own code run offline
/// (tools/traffic_curve_reference.py -> testdata/traffic_curve_ref.txt; a closed path - flag word set - builds from one point less).
#[cfg(test)]
mod original_curve_tests {
    use super::*;

    #[test]
    fn path_curve_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/traffic_curve_ref.txt").lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let a: Vec<&str> = a.split(' ').skip(1).collect();
            let (k, flag): (usize, u32) = (a[0].parse().unwrap(), a[1].parse().unwrap());
            let w: Vec<u32> = a[2..].iter().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let pts: Vec<Vec3> = (0..k).map(|i| Vec3::new(f32::from_bits(w[4 * i]), f32::from_bits(w[4 * i + 1]), f32::from_bits(w[4 * i + 2]))).collect();
            let used = if flag != 0 { k - 1 } else { k };
            let c = path_curve(&pts[..used]);
            let o: Vec<&str> = b.split(' ').collect();
            let (deg, cnt): (u8, usize) = (o[0].parse().unwrap(), o[1].parse().unwrap());
            let kn: Vec<u32> = o[2..2 + cnt].iter().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let (tr, tm) = (u32::from_str_radix(o[2 + cnt], 16).unwrap(), u32::from_str_radix(o[3 + cnt], 16).unwrap());
            assert_eq!(c.degree, deg, "{line}");
            assert_eq!(c.knots.iter().map(|f| f.to_bits()).collect::<Vec<_>>(), kn, "{line}");
            assert_eq!((c.t_range.to_bits(), c.t_max.to_bits()), (tr, tm), "{line}");
            // the control points the constructor copied (16 bytes each; the port keeps x y z)
            let np: usize = o[4 + cnt].parse().unwrap();
            assert_eq!(np, used, "{line}");
            assert_eq!(c.pts.len(), np, "{line}");
            for (i, q) in c.pts.iter().enumerate() {
                let w: Vec<u32> = o[5 + cnt + 4 * i..9 + cnt + 4 * i].iter().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
                assert_eq!([q.x.to_bits(), q.y.to_bits(), q.z.to_bits()], [w[0], w[1], w[2]], "{line}");
            }
            n += 1;
        }
        assert_eq!(n, 126);
    }

    /// F-10286: the spawn matrix heading (guest atan2 wrapped to (0, 2 pi]) against the original's code run offline
    /// (tools/people_small_reference.py -> testdata/people_small_ref.txt).
    #[test]
    fn heading_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/people_small_ref.txt").lines().filter(|l| l.starts_with("HD ")) {
            let w: Vec<&str> = line.split(' ').collect();
            let hx = |x: &str| u32::from_str_radix(x, 16).unwrap();
            let got = heading_of(f32::from_bits(hx(w[1])), f32::from_bits(hx(w[2])));
            assert_eq!(got.to_bits(), hx(w[4]), "{line}");
            n += 1;
        }
        assert_eq!(n, 313);
    }

    /// F-71868 (point and unscaled derivative of the two-output evaluator) and F-75457 (B-spline basis and derivative) against the
    /// original's code run offline on path curves of degree 1 - 3 (tools/people_curve_reference.py -> testdata/people_curve_ref.txt).
    /// The fourth vector lane (the weight sum) is not compared.
    #[test]
    fn curve_pair_and_basis_match_the_original() {
        let hx = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut curve: Option<LaneCurve> = None;
        let (mut ev, mut ba) = (0, 0);
        for line in include_str!("testdata/people_curve_ref.txt").lines() {
            let w: Vec<&str> = line.split(' ').collect();
            if w[0] == "C" {
                let (k, flag): (usize, u32) = (w[1].parse().unwrap(), w[2].parse().unwrap());
                let f = |i: usize| f32::from_bits(hx(w[3 + i]));
                let pts: Vec<Vec3> = (0..k).map(|i| Vec3::new(f(4 * i), f(4 * i + 1), f(4 * i + 2))).collect();
                curve = Some(path_curve(&pts[..if flag != 0 { k - 1 } else { k }]));
            } else if w[0] == "EV" {
                let c = curve.as_ref().unwrap();
                let (p, d) = c.eval_pair(f32::from_bits(hx(w[1])));
                let want: Vec<u32> = w[3..].iter().map(|x| hx(x)).collect();
                assert_eq!([p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), d.x.to_bits(), d.y.to_bits(), d.z.to_bits()], [want[0], want[1], want[2], want[4], want[5], want[6]], "{line}");
                ev += 1;
            } else {
                let c = curve.as_ref().unwrap();
                let (n, d) = c.basis_pair(w[1].parse().unwrap(), w[2].parse().unwrap(), f32::from_bits(hx(w[3])));
                assert_eq!([n.to_bits(), d.to_bits()], [hx(w[5]), hx(w[6])], "{line}");
                ba += 1;
            }
        }
        assert_eq!((ev, ba), (1428, 840));
    }
}

/// (test hook: MCLA_TRAFFIC_PLACE's seventh field, the colour row 0..19 of the type's table, for every car
/// spawned while it is set - to match a recording of the original)
pub(crate) fn test_shade(shade: usize) -> usize {
    std::env::var("MCLA_TRAFFIC_PLACE").ok().and_then(|v| v.split(',').nth(6).and_then(|x| x.trim().parse::<usize>().ok())).map_or(shade, |s| s.min(19))
}

#[cfg(test)]
mod crossing_tests {
    use super::*;

    /// The crossing unspawner against the original's code run offline (tools/crossing_unspawn_reference.py ->
    /// testdata/crossing_unspawn_ref.txt): F-54093 whole on a fake object grid with the grid's cell code, the crossing
    /// test, the freeway test and the probe helper F-13384 as they are; per case the view tests and probes asked (their
    /// arguments), the removal and the words the run leaves in the manager (the 3 x 3 cells' first objects and numbers,
    /// the player's segment, each car's segment and crossing) - all equal.
    #[test]
    fn crossing_unspawn_matches_the_original() {
        let mut n = 0;
        let mut removals = 0;
        for line in include_str!("testdata/crossing_unspawn_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = input.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let v3 = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let (focus, vel, view) = (v3(4), v3(7), v3(10));
            let (gw, gh) = (w[13] as i32, w[14] as i32);
            let grid = CrossGrid { min: [f(15), f(16)], n: [gw, gh], bounded: true };
            let nc = w[17] as usize;
            let cars: Vec<CrossCar> = (0..nc).map(|k| {
                let o = 18 + 14 * k;
                let bits = w[o + 7];
                let (mode, speed) = (w[o + 8], f(o + 9));
                let lane = bits & 4 == 0 && bits & 256 == 0;
                CrossCar {
                    kind2: w[o] == 2,
                    pos: v3(o + 1),
                    vel: v3(o + 4),
                    proxy: bits & 1 != 0,
                    remote: bits & 2 != 0,
                    behaviour: bits & 4 == 0,
                    group2: bits & 8 != 0,
                    item: bits & 16 != 0,
                    slot18: bits & 32 != 0,
                    slot19: bits & 64 != 0,
                    slot22: bits & 128 != 0,
                    // (F-31543: the owner a road, or its parent; the road's speed >= FREEWAY_SPEED)
                    freeway: lane && (mode == 0 || mode == 1) && speed >= FREEWAY_SPEED,
                    fwd: v3(o + 10),
                    radius: f(o + 13),
                }
            }).collect();
            let mut got = String::new();
            if w[0] != 0 || w[1] != 0 || w[2] != 0 {
                got.push('X');
            } else {
                let mut idx = [-1i32; 9];
                let mut heads = [0u32; 9];
                let mut run = CrossRun::default();
                if w[3] != 0 {
                    let calls = std::cell::RefCell::new(String::new());
                    let mut rays = vec![0u32; nc];
                    run = crossing_unspawn(&grid, focus, vel, view, &cars, |k, p| {
                        *calls.borrow_mut() += &format!(" S {k} {:08x} {:08x} {:08x} 3fc00000 0 1", p.x.to_bits(), p.y.to_bits(), p.z.to_bits());
                        w[18 + 14 * k + 7] & 512 != 0
                    }, |k, eye, end| {
                        *calls.borrow_mut() += &format!(" Q {k} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} 00000280 00000800 04 1", eye.x.to_bits(), eye.y.to_bits(), eye.z.to_bits(), end.x.to_bits(), end.y.to_bits(), end.z.to_bits());
                        let b = if rays[k] == 0 { 1024 } else { 2048 };
                        rays[k] += 1;
                        w[18 + 14 * k + 7] & b != 0
                    });
                    let mut calls = calls.into_inner();
                    if let Some(k) = run.remove {
                        calls += &format!(" D {k} 6 1");
                        removals += 1;
                    }
                    got.push_str(calls.trim_start());
                    if !got.is_empty() {
                        got.push(' ');
                    }
                    // (F-99153 / F-49551: the 3 x 3 cells' first objects - inserted in car order - and numbers)
                    if let Some((cx, cz)) = grid.cell(run.p.x, run.p.z) {
                        for (j, (dz, dx)) in (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (dz, dx))).enumerate() {
                            let (ix, iz) = (cx + dx, cz + dz);
                            if grid.holds(ix, iz) {
                                idx[j] = iz * gw + ix;
                                heads[j] = cars.iter().position(|c| grid.cell(c.pos.x, c.pos.z) == Some((ix, iz))).map_or(0, |k| 0x2000 + 0x40 * k as u32);
                            }
                        }
                    }
                }
                got.push('M');
                for h in heads {
                    got += &format!(" {h:08x}");
                }
                for i in idx {
                    got += &format!(" {:08x}", i as u32);
                }
                for x in [run.a[0], run.a[1], run.b[0], run.b[1]] {
                    got += &format!(" {:08x}", x.to_bits());
                }
                let last = run.tested.iter().rposition(|t| t.1[0].to_bits() | t.1[1].to_bits() | t.2[0].to_bits() | t.2[1].to_bits() != 0 || t.3).map_or(0, |i| i + 1);
                got += &format!(" {last}");
                for t in &run.tested[..last] {
                    got += &format!(" {:08x} {:08x} {:08x} {:08x} {}", t.1[0].to_bits(), t.1[1].to_bits(), t.2[0].to_bits(), t.2[1].to_bits(), t.3 as u8);
                }
            }
            assert_eq!(got, want, "case {n}: {input}");
            n += 1;
        }
        assert!(n >= 2900 && removals > 300, "{n} cases, {removals} removals");
    }
}

// =====================================================================================================================
// The cruising ghost: the ambient request pool, the proxy car on its route, the release (the hooks of `cruiser.rs`'s
// `GhostTraffic`). Rules: docs/reference/ai/cruise_ghost.md (the decode); signatures, evidence and what is open:
// docs/reference/people/cruise_ghost_hooks.md.
// =====================================================================================================================

type GMwc = super::parked::Mwc;

/// The proxy type the brain's requests name (F-62453: the manager's first bIsProxy type; the only one shipped).
pub const PROXY_TYPE: &str = "0x0bccfb1d";

/// F-37317: an MWC seeded from `x` (lo = x + (x == 0), carry = x ^ rotl16(x); its byte +8 and float +12 zeroed).
pub fn mwc_seed(x: u32) -> GMwc {
    GMwc { lo: x.wrapping_add((x == 0) as u32), carry: x ^ x.rotate_left(16) }
}

/// The low 23 bits of an MWC's next word as the original converts them (`clrlwi 9`, `fcfid`, `frsp`: exact).
fn u23(m: &mut GMwc) -> f32 {
    (m.next() & 0x7F_FFFF) as f32
}

/// An ambient car's init draws (F-77422, which runs slot 5 F-60230 first, which runs the base F-92604 first) in the
/// original's order, from the car's own MWC (car +384 / +388, seeded by F-92604 from the pool record's draw rec+832)
/// and the second shared MWC (the 8 bytes 16 below the shared `park_rng` in the image; `Vehicles::amb_rng2`). Every constant is the
/// image's word.
#[derive(Clone, Copy, Debug, Default)]
pub struct InitDraws {
    /// +468 = fma(U, 2^-21, 8): the life on a listed road (F-92604, the car MWC's first draw)
    pub life: f32,
    /// +432 = fma(U, 3 x 2^-23, 3): the speed offset's first timer (F-92604, the second)
    pub off_timer: f32,
    /// +444 = U x 0.8 x 2^-23 (`fmuls` by 0x33CCCCCD): the start delay (F-60230; 0 without a draw for a proxy car or
    /// while the manager's byte +0x64234 is set)
    pub wait: f32,
    /// +3568 = U2 x 2^-25: the lights' hour offset (the second shared MWC)
    pub hour: f32,
    /// +3576 / +3580 = fmsubs(U2, 0x32CCCCCD, 0.1): the signal blink's offsets (the second shared MWC)
    pub blink_wrap: f32,
    pub blink_on: f32,
    /// +428 = fmsubs(U, 2^-23, 0.5): the acceleration bonus (0 without a draw for a proxy car or networked)
    pub bonus: f32,
    /// +3584: a spot's car (request +252 set): F-66400 on the car MWC, mean 0, sd pi / 8, clamped by two `fsel` to
    /// [-0.70685834, 0.70685834]; else not drawn (0)
    pub steer: f32,
    /// a cab (type +111, offline): U x 2^-23 < 0.75 hitches a trailer (F-45988)
    pub hitch: bool,
    /// the car's MWC after F-60230, and after F-77422
    pub placed: GMwc,
    pub car: GMwc,
}

/// F-66400 (`physics::vehsim::VehState::random_normal`, checked there against the original): the polar method on the
/// car's MWC with its spare cleared (F-37317 zeroes it), the result fma(value, deviation, mean).
fn car_normal(m: &mut GMwc, mean: f32, dev: f32) -> f32 {
    loop {
        let a = u23(m) * (1.0 / 4_194_304.0) - 1.0;
        let b = u23(m) * (1.0 / 4_194_304.0) - 1.0;
        let s = b.mul_add(b, a * a);
        if s < 1.0 && s != 0.0 {
            let f = ((crate::caraudio::ln_guest(s as f64) as f32 / s) * -2.0).sqrt();
            return (f * b).mul_add(dev, mean);
        }
    }
}

/// F-92604's id loop for a cab or trailer (request id < 0, rec+836 set): up to 20 draws of the shared MWC, each
/// r = (lo & 0x7FFFFFFF) % 512 (the id ((r << 7 | the type's index) << 5) | the peer), until `taken(r)` (F-10061 finds
/// a car with that id) is false; the last r stays when all 20 are taken.
pub fn id_draws(shared: &mut GMwc, mut taken: impl FnMut(i32) -> bool) -> i32 {
    let mut r = 0;
    for _ in 0..20 {
        r = ((shared.next() & 0x7FFF_FFFF) % 512) as i32;
        if !taken(r) {
            break;
        }
    }
    r
}

/// F-92604 + F-60230 + F-77422's draws (`InitDraws`): `seed` the pool record's rec+832, `spot` a parked spot's car,
/// `cab` the type's +111, `proxy` car +500 bit 0x20 (the bIsProxy pool), `green` the manager's byte +0x64234.
/// Offline: the networked tests are false. (Not here: the cab / trailer id draws of the shared MWC F-92604 makes
/// between its two draws, `Vehicles::amb_id`.)
pub fn init_draws(seed: u32, shared2: &mut GMwc, spot: bool, cab: bool, proxy: bool, green: bool) -> InitDraws {
    use crate::physics::exact::{fma, fms};
    let mut car = mwc_seed(seed);
    let mut d = InitDraws::default();
    // F-92604: +468, then +432
    d.life = fma(u23(&mut car), f32::from_bits(0x3500_0000), 8.0);
    d.off_timer = fma(u23(&mut car), f32::from_bits(0x34C0_0000), 3.0);
    // F-60230: +444
    d.wait = if green || proxy { 0.0 } else { u23(&mut car) * f32::from_bits(0x33CC_CCCD) };
    d.placed = car;
    // F-77422: the second shared MWC three times
    d.hour = u23(shared2) * f32::from_bits(0x3300_0000);
    d.blink_wrap = fms(u23(shared2), f32::from_bits(0x32CC_CCCD), f32::from_bits(0x3DCC_CCCD));
    d.blink_on = fms(u23(shared2), f32::from_bits(0x32CC_CCCD), f32::from_bits(0x3DCC_CCCD));
    d.bonus = if proxy { 0.0 } else { fms(u23(&mut car), f32::from_bits(0x3400_0000), 0.5) };
    if spot {
        let g = car_normal(&mut car, 0.0, f32::from_bits(0x3EC9_0FDB));
        let (lo, hi) = (f32::from_bits(0xBF34_F4AB), f32::from_bits(0x3F34_F4AB));
        let a = if g - lo >= 0.0 { g } else { lo };
        d.steer = if a - hi >= 0.0 { hi } else { a };
    }
    if cab {
        d.hitch = u23(&mut car) * f32::from_bits(0x3400_0000) < 0.75;
    }
    d.car = car;
    d
}

/// The car's speed fields the integrator works on: +412 speed, +424 offset, +432 offset timer (seconds of age), +464
/// born (manager clock), +384 / +388 the car's own MWC.
#[derive(Clone, Copy, Debug)]
pub struct CarSpeed {
    pub v: f32,
    pub off: f32,
    pub timer: f32,
    pub born: f32,
    pub rng: GMwc,
}

/// F-58614, the integrator: the cap is fMaxSpeed (type +124) or SetSpeed (behaviour +244 when its +256 has 0x40:
/// `set_speed`); a > fMaxAccel (+136) is clamped when a > 0; v' = a dt + v fused; a < 0 and v' under the stop speed ->
/// 0, a > 0 and v' over the cap -> the cap. Then the offset timer: when the car's age (manager clock - born) passes it,
/// one draw of the car's MWC sets it to age + 3 + 3U, and the offset is 0 for a networked car or one of the proxy pool
/// (`no_offset`: car +500 bit 0x20), else a second draw: -6, +6 or 0 by thirds.
#[allow(clippy::too_many_arguments)]
pub fn car_speed(s: &mut CarSpeed, a: f32, dt: f32, max_speed: f32, max_accel: f32, set_speed: Option<f32>, clock: f32, no_offset: bool) {
    s.v = speed_step(s.v, a, dt, set_speed.unwrap_or(max_speed), max_accel);
    offset_step(s, clock, no_offset);
}

/// F-58614's first part: the new speed +412 from the speed `v`, the asked acceleration `a` (+408), the cap (fMaxSpeed
/// or SetSpeed) and fMaxAccel.
pub fn speed_step(v: f32, a: f32, dt: f32, cap: f32, max_accel: f32) -> f32 {
    let a = if a > 0.0 && a > max_accel { max_accel } else { a };
    let v = a.mul_add(dt, v);
    if a < 0.0 && v < f32::from_bits(0x3E64_E26D) {
        0.0
    } else if a > 0.0 && v > cap {
        cap
    } else {
        v
    }
}

/// F-58614's second part: the speed offset's timer and draws on the car's MWC.
pub fn offset_step(s: &mut CarSpeed, clock: f32, no_offset: bool) {
    let age = clock - s.born;
    if age > s.timer {
        let u = (s.rng.next() & 0x7F_FFFF) as f32;
        s.timer = u.mul_add(f32::from_bits(0x34C0_0000), age) + 3.0;
        if no_offset {
            s.off = 0.0;
        } else {
            let u = (s.rng.next() & 0x7F_FFFF) as f32 * f32::from_bits(0x3400_0000);
            s.off = if u < f32::from_bits(0x3EAA_7EFA) {
                -6.0
            } else if u < f32::from_bits(0x3F2A_C083) {
                6.0
            } else {
                0.0
            };
        }
    }
}

/// The ghost rider's integrator: `car_speed` on its fields (proxy pool: no offset).
fn ghost_integrate(r: &mut Rider, a: f32, dt: f32, clock: f32) {
    r.a = a;
    let (max_speed, max_accel) = (r.ty.max_speed, r.ty.max_accel);
    let Some(g) = r.ghost.as_mut() else { return };
    let mut s = CarSpeed { v: r.v, off: r.off, timer: r.off_timer, born: r.born, rng: g.rng };
    car_speed(&mut s, a, dt, max_speed, max_accel, g.set_speed, clock, true);
    r.v = s.v;
    r.off = s.off;
    r.off_timer = s.timer;
    g.rng = s.rng;
}

/// The vector unit's flush of denormals (the recompiled code runs with FTZ / DAZ).
fn ftz(x: f32) -> f32 {
    if x.is_subnormal() {
        0.0f32.copysign(x)
    } else {
        x
    }
}
fn vm(a: f32, b: f32) -> f32 {
    ftz(ftz(a) * ftz(b))
}
fn va(a: f32, b: f32) -> f32 {
    ftz(ftz(a) + ftz(b))
}
fn vs(a: f32, b: f32) -> f32 {
    ftz(ftz(a) - ftz(b))
}

/// The pose block's normalisation: l2 = z^2 + (y^2 + x^2), y0 = 1 / sqrt(l2) (0 where that is +inf), one Newton step
/// s = (1 - l2 y0^2) (y0 / 2) + y0, each step rounded.
fn pose_norm(v: [f32; 3]) -> [f32; 3] {
    let l2 = va(vm(v[2], v[2]), va(vm(v[1], v[1]), vm(v[0], v[0])));
    let mut y0 = ftz(1.0 / ftz(l2).sqrt());
    if y0 == f32::INFINITY {
        y0 = 0.0;
    }
    let k = -vs(vm(l2, vm(y0, y0)), 1.0);
    let s = va(vm(k, vm(0.5, y0)), y0);
    [vm(v[0], s), vm(v[1], s), vm(v[2], s)]
}

/// F-18708 from the route: the request's matrix (rows a, b, c, place) at the start lane's point `p` and tangent: c =
/// (0 - tangent) with y = 0, normalised; b = (0, 1, 0); a = -(c.yzx b.zxy - c.zxy b.yzx), normalised; place = p.
pub fn spawn_pose(p: Vec3, tangent: Vec3) -> [Vec3; 4] {
    let c = pose_norm([vs(0.0, tangent.x), 0.0, vs(0.0, tangent.z)]);
    let b = [0.0f32, 1.0, 0.0];
    let m1 = [vm(c[1], b[2]), vm(c[2], b[0]), vm(c[0], b[1])];
    let m2 = [vm(c[2], b[1]), vm(c[0], b[2]), vm(c[1], b[0])];
    let a = pose_norm([-vs(m1[0], m2[0]), -vs(m1[1], m2[1]), -vs(m1[2], m2[2])]);
    [Vec3::from_array(a), Vec3::from_array(b), Vec3::from_array(c), p]
}

/// What the brain's request asks for (F-25539 / F-45833): the route, its start lane and phase, SetSpeed (req+68, flag
/// 0x40) and SetDestSpeed (req+76, flag 0x20).
#[derive(Clone, Debug)]
pub struct GhostOrder {
    pub route: crate::railroute::Route,
    pub start: (u32, f32),
    pub set_speed: Option<f32>,
    pub dest_speed: Option<f32>,
}

/// A record of the manager's request pool (manager +8800, 848 bytes; req+X = rec+16+X).
#[derive(Clone, Debug)]
pub struct PoolRec {
    /// rec+4: 1 free (the pool's ctor F-57400), 2 pending, 0 made, 3 timed out, 5 no ground, 7 cancelled
    pub state: u32,
    /// rec+0, set while the record is stepped
    pub busy: bool,
    /// req+284: 0x80 a behaviour car, 0x40 a removal
    pub kind: u8,
    /// req+242 (rec+258) the steps wanted and rec+308 the steps done
    pub wanted: u8,
    pub done: u8,
    /// rec+304 the pending time and req+0 its timeout (3.0 from the descriptor, else 0.0)
    pub time: f32,
    pub timeout: f32,
    /// req+8 the owner (the racer; notified at the end, F-90613)
    pub owner: u32,
    pub order: Option<Box<GhostOrder>>,
    /// rec+788 the car, rec+792 the behaviour
    pub car: u32,
    pub beh: u32,
    /// a removal's car (req+256) and reason (req+260)
    pub target: u32,
    pub reason: u32,
    /// rec+832: the shared MWC's draw at the spawn (the car's MWC is seeded from it, F-92604)
    pub seed: u32,
    /// rec+544: the spawn matrix (F-18708)
    pub matrix: [Vec3; 4],
}

impl PoolRec {
    fn free() -> PoolRec {
        PoolRec { state: 1, busy: false, kind: 0, wanted: 0, done: 0, time: 0.0, timeout: 0.0, owner: 0, order: None, car: 0, beh: 0, target: 0, reason: 0, seed: 0, matrix: [Vec3::ZERO; 4] }
    }
}

/// What the pool asks of the world, in the order the original's code does it (each is a call the offline harness
/// logs: L, C, c, B, b, D, P, I, K, N).
pub trait PoolWorld {
    /// the shared MWC (`Vehicles::shared_mwc`)
    fn shared(&mut self) -> &mut GMwc;
    /// the route's start lane's slot 8 at `t`: point and tangent
    fn lane_point(&mut self, lane: u32, t: f32) -> (Vec3, Vec3);
    /// a car of the proxy pool (F-21879; 0: none) and one given back (F-73520)
    fn car_get(&mut self) -> u32;
    fn car_back(&mut self, car: u32);
    /// a Drive2 behaviour (F-60281; 0: none) and one given back (F-24235)
    fn beh_get(&mut self) -> u32;
    fn beh_back(&mut self, beh: u32);
    /// a car's behaviour (car +452) and its vfunc 0
    fn car_beh(&mut self, car: u32) -> u32;
    fn beh_drop(&mut self, beh: u32);
    /// the type's slot 14 spawn probe (F-15062) at the record's matrix
    fn probe(&mut self, car: u32, seed: u32, m: &[Vec3; 4]) -> bool;
    /// car vfunc 5 (F-77422): the car made from the record
    fn car_init(&mut self, rec: usize, r: &PoolRec);
    /// the type's release of a car (F-38415 -> car vfunc 11, which tells the owner: vfunc 38)
    fn car_release(&mut self, car: u32, reason: u32);
    /// the owner's vfunc 35 (the attach: status, car) or 36 (a removal's: status, the car removed)
    fn notify(&mut self, rec: usize, owner: u32, vf: u8, status: u32, car: u32);
}

/// The manager's request pool (F-13129, F-52865, F-29339, F-11284, F-51512) with the proxy type's budget (type +4 and
/// +8, F-49084 / F-39893).
#[derive(Clone, Debug)]
pub struct GhostPool {
    pub recs: Vec<PoolRec>,
    /// pool +0 the pending count, +4 its most
    pub count: i32,
    pub max: i32,
    /// the type's budget left (+4, iInstanceCount at the start) and in use (+8)
    pub budget: i32,
    pub used: i32,
}

impl Default for GhostPool {
    fn default() -> Self {
        GhostPool::new(0)
    }
}

impl GhostPool {
    /// F-57400: 256 free records.
    pub fn new(budget: i32) -> GhostPool {
        GhostPool { recs: vec![PoolRec::free(); 256], count: 0, max: 0, budget, used: 0 }
    }

    fn take_budget(&mut self) -> bool {
        if self.budget > 0 {
            self.budget -= 1;
            self.used += 1;
            true
        } else {
            false
        }
    }

    fn give_budget(&mut self) {
        self.used -= 1;
        self.budget += 1;
    }

    /// F-13129 (with the record's init F-75521): a removal (0x40) of a car whose removal is pending gives that record
    /// (F-83456); else the first record neither pending nor busy, made pending; -1 when none.
    fn queue(&mut self, mut r: PoolRec) -> i32 {
        if r.kind & 0x40 != 0 && r.target != 0 {
            if let Some(i) = self.recs.iter().position(|o| o.state == 2 && o.target == r.target) {
                return i as i32;
            }
        }
        let Some(i) = self.recs.iter().position(|o| o.state != 2 && !o.busy) else { return -1 };
        r.state = 2;
        r.time = 0.0;
        r.done = 0;
        self.recs[i] = r;
        self.count += 1;
        self.max = self.max.max(self.count);
        i as i32
    }

    /// The brain's request (F-25539: F-86587, F-45833, SetSpeed / SetDestSpeed; F-59429): a behaviour car (kind 0x80,
    /// steps 0xA4, timeout 3 s) for `order`; None (the brain's idx -1) a request with neither descriptor nor route
    /// (kind 0, owner 0, timeout 0: it times out at the first step with dt > 0, and nobody is told).
    pub fn submit(&mut self, order: Option<GhostOrder>, owner: u32) -> i32 {
        let mut r = PoolRec::free();
        if let Some(o) = order {
            r.kind = 0x80;
            r.wanted = 0xA4;
            r.timeout = 3.0;
            r.owner = owner;
            r.order = Some(Box::new(o));
        }
        self.queue(r)
    }

    /// F-51512 -> F-76814: a removal of `car` (owner `owner`, car +52) for `reason`, served at the next step.
    pub fn submit_remove(&mut self, car: u32, reason: u32, owner: u32) -> i32 {
        let mut r = PoolRec::free();
        r.kind = 0x40;
        r.wanted = 0x50;
        r.target = car;
        r.reason = reason;
        r.owner = owner;
        self.queue(r)
    }

    /// F-11284: the record cancelled (state 7) and stepped at once (dt 0: the cleanup and the owner's notify with 7).
    pub fn cancel(&mut self, id: i32, w: &mut dyn PoolWorld) {
        // (the original indexes the pool with any id; the brain only cancels a pending request it holds)
        if !(0..256).contains(&id) {
            return;
        }
        self.recs[id as usize].state = 7;
        self.step_rec(id as usize, 0.0, w);
        self.count -= 1;
    }

    /// F-52865: every pending record in slot order, until as many as were pending have been seen.
    pub fn step(&mut self, dt: f32, w: &mut dyn PoolWorld) {
        let n = self.count;
        if n <= 0 {
            return;
        }
        let mut seen = 0;
        let mut i = 0;
        loop {
            if i >= 256 {
                break;
            }
            if self.recs[i].state == 2 {
                self.step_rec(i, dt, w);
                seen += 1;
                if self.recs[i].state != 2 {
                    self.count -= 1;
                }
            }
            i += 1;
            if seen >= n {
                break;
            }
        }
    }

    fn notify(&mut self, i: usize, w: &mut dyn PoolWorld) {
        let r = &self.recs[i];
        if r.owner == 0 {
            return;
        }
        if r.kind & 0x80 != 0 {
            w.notify(i, r.owner, 35, r.state, r.car);
        } else if r.kind & 0x40 != 0 {
            w.notify(i, r.owner, 36, r.state, r.target);
        }
    }

    /// F-29339: the record's stages, each only while it is pending, its step wanted and not done; the timeout; the end.
    fn step_rec(&mut self, i: usize, dt: f32, w: &mut dyn PoolWorld) {
        self.recs[i].busy = true;
        let pend = |r: &PoolRec, bit: u8| r.state == 2 && r.wanted & bit != 0 && r.done & bit == 0;
        // 0x04 F-18708: the pose from the route's start lane at its t
        if pend(&self.recs[i], 0x04) {
            if let Some((lane, t)) = self.recs[i].order.as_ref().map(|o| o.start) {
                let (p, tg) = w.lane_point(lane, t);
                let r = &mut self.recs[i];
                r.matrix = spawn_pose(p, tg);
                r.done |= 0x04;
            }
        }
        // 0x80 F-12577: one of the type's budget and a car of the proxy pool (none: the budget back, wait)
        if pend(&self.recs[i], 0x80) && self.take_budget() {
            let car = w.car_get();
            self.recs[i].car = car;
            if car != 0 {
                self.recs[i].done |= 0x80;
            } else {
                self.give_budget();
            }
        }
        // 0x20 F-19334: a behaviour (done even with none)
        {
            let r = &self.recs[i];
            if r.kind & 0x80 != 0 && pend(r, 0x20) && r.done & 0x80 != 0 {
                let b = w.beh_get();
                let r = &mut self.recs[i];
                r.beh = b;
                r.done |= 0x20;
            }
        }
        // F-42724: every wanted step done -> the shared MWC's draw, the probe (F-46111) and the car's init
        {
            let r = &self.recs[i];
            if r.state == 2 && r.kind & 0x80 != 0 && r.done & r.wanted == r.wanted {
                let seed = w.shared().next() & 0x7FFF_FFFF;
                self.recs[i].seed = seed;
                let (car, m) = (self.recs[i].car, self.recs[i].matrix);
                if w.probe(car, seed, &m) {
                    w.car_init(i, &self.recs[i]);
                    self.recs[i].state = 0;
                } else {
                    self.recs[i].state = 5;
                }
            }
        }
        // a removal: F-10793 (the car), then its behaviour's vfunc 0 and the behaviour given back, then F-73806 (the
        // type's release)
        {
            let r = &mut self.recs[i];
            if r.kind & 0x40 != 0 && pend(r, 0x40) {
                r.car = r.target;
                r.done |= 0x40;
            }
        }
        if self.recs[i].kind & 0x40 != 0 && pend(&self.recs[i], 0x10) {
            let b = w.car_beh(self.recs[i].target);
            if b != 0 {
                w.beh_drop(b);
                w.beh_back(b);
            }
            self.recs[i].done |= 0x10;
        }
        {
            let r = &self.recs[i];
            if r.state == 2 && r.kind & 0x40 != 0 && r.done & r.wanted == r.wanted && r.target != 0 {
                w.car_release(r.target, r.reason);
                self.recs[i].state = 0;
            }
        }
        // still pending: the timeout (3 s for the brain's request)
        {
            let r = &mut self.recs[i];
            if r.state == 2 {
                r.time += dt;
                if r.time > r.timeout {
                    r.state = 3;
                }
            }
        }
        // made: F-31134 (a removal's car back with its budget), the owner told
        if self.recs[i].state == 0 {
            let r = &mut self.recs[i];
            if r.kind & 0x80 != 0 {
                r.order = None;
            } else if r.kind & 0x40 != 0 && r.car != 0 {
                let car = r.car;
                w.car_back(car);
                self.give_budget();
            }
            self.notify(i, w);
        }
        // failed (3 .. 7): F-92217 -> F-59370 (a request's route, behaviour and car given back), the owner told
        if (3..=7).contains(&self.recs[i].state) {
            if self.recs[i].kind & 0x70 == 0 {
                self.recs[i].order = None;
                let b = self.recs[i].beh;
                w.beh_back(b);
                self.recs[i].beh = 0;
                let car = self.recs[i].car;
                if car != 0 {
                    w.car_back(car);
                    self.give_budget();
                    self.recs[i].car = 0;
                }
            }
            self.notify(i, w);
        }
        self.recs[i].busy = false;
    }
}

/// The cruising ghost's part of its rider (the behaviour's route object and flags, the car's own MWC).
#[derive(Clone, Debug)]
pub struct GhostRide {
    /// the route (behaviour +288, F-28180) and its two indices (+128 / +132)
    pub route: crate::railroute::Route,
    pub track: crate::ghostroute::Track,
    /// behaviour +256: 0x40 SetSpeed, 0x20 the destination speed
    pub flags: u8,
    /// SetSpeed (+244, with flag 0x40: the integrator's cap) and the destination speed (+252)
    pub set_speed: Option<f32>,
    pub dest_speed: f32,
    /// the car's own MWC (+384 / +388; the integrator's offset timer and the route's strict pick, F-52442)
    pub rng: GMwc,
    /// a lane was taken this frame: its next lane is the route's (F-74277), chosen after the step
    pub turned: bool,
    /// the controller's entry 3 (F-25114): its hysteresis, and (speed, distance) this frame
    pub on3: bool,
    pub dest: Option<(f32, f32)>,
    /// the route lane change's arguments this frame (F-24481), taken at once (F-32118)
    pub change: Option<crate::ghostroute::Change>,
    /// the follower on the lane-change rail (F-32118 / F-11487: follower +156 the rail, +160 the target lane, +164 its
    /// phase): ridden in state 1 until its end, then onto the target lane
    pub lc: Option<Path>,
}

impl GhostRide {
    fn dest_entry(&self) -> Option<(f32, f32)> {
        self.dest
    }
}

/// F-11487's four control points: the two lanes' unit derivatives (the vector unit's normalisation) T0 at the
/// current lane's point P0 and T1 at the target's P1; P0, P0 + W T0, P1 - W T1, P1 (vmaddfp: product, then sum).
pub fn lane_change_cps(p0: Vec3, d0: Vec3, p1: Vec3, d1: Vec3, w: f32) -> [Vec3; 4] {
    use crate::tcp_rail::{vmadd3, vnorm};
    let (t0, t1) = (vnorm(d0), vnorm(d1));
    [p0, vmadd3(t0, w, p0), vmadd3(t1, -w, p1), p1]
}

/// The route lane change F-32118 -> F-11487 (decode: docs/reference/people/cruise_ghost_hooks.md "Lane-change rail"):
/// the current lane's point and derivative at t (slot 15's curve, F-71868) and the target's at its phase, the four
/// control points (`lane_change_cps` with W = F-67514's), a rail of the pool's index 0 (mcRailAmbientDynamic: the B-spline
/// over them, `path_curve`) at the target lane's slot 17 speed (F-27898, `rail_speed`), attached (F-70405) from phase 0
/// with the target lane at the change's phase as the next rail. (L, F-36745, is passed but not read.)
pub fn lane_change_rail(nav: &NavSys, lanes: &Lanes, c: &crate::ghostroute::Change) -> Option<Path> {
    let (p0, d0) = nav.curves.get(c.from as usize)?.eval_pair(c.t);
    let (p1, d1) = nav.curves.get(c.to as usize)?.eval_pair(c.phase);
    let &link = lanes.links.get(c.to as usize)?.first()?;
    // (the attach's owner: the current lane's road, F-86691 cast to a road)
    Some(lane_change_path([p0, d0, p1, d1], c.w, rail_speed(nav, link).0, c.to as usize, c.phase, nav.lane_road(c.from as usize)))
}

/// F-11487's rail from the two lanes' points and derivatives `pd` (P0, D0, P1, D1): the pool's index 0 at `speed`, the
/// four control points, attached from phase 0 with (`to`, `phase`) as the next rail.
fn lane_change_path(pd: [Vec3; 4], w: f32, speed: f32, to: usize, phase: f32, owner: Option<usize>) -> Path {
    let cps = lane_change_cps(pd[0], pd[1], pd[2], pd[3], w);
    let c = path_curve(&cps);
    let ds = set_rail_speed(&c, 0.0);
    Path { geom: Geom::Curve(c), speed, end: Some((to, phase)), reverse: false, t: 0.0, ds, owner }
}

/// The cruising ghost's place and unit heading: on the lane-change rail its point and unit derivative, else its lane's.
fn ghost_point(r: &Rider, nav: &NavSys) -> (Vec3, Vec3) {
    match r.ghost.as_ref().and_then(|g| g.lc.as_ref()) {
        Some(p) => p.at(p.t),
        None => (r.pos(nav), r.tangent(nav)),
    }
}

/// State 1 of the cruising ghost while its follower is on the lane-change rail (F-50894 on a run-time rail): the wanted
/// speed F-70324 from the rail's speed (+304, the target lane's) and the next rail's (the target lane), the car ahead
/// along the rail and the target lane (F-47175's grid branch, as `step_route`), the destination entry, the integrator;
/// past the rail's end the follower takes the target lane at the change's phase (follower +160 / +164) with the rest of
/// the distance, and the route's next lane is asked (F-48618 -> F-74277, `turned`). (No stop-line or slower-lane entry
/// on a run-time rail, as the port's other run-time rails.)
fn step_lane_change(r: &mut Rider, env: &Env, me: usize, half_len: f32, dt: f32) -> (Vec3, Vec3) {
    let nav = env.nav;
    let path = r.ghost.as_ref().and_then(|g| g.lc.clone()).unwrap();
    let (to, to_t) = path.end.unwrap();
    let next_link = env.lanes.links.get(to).and_then(|l| l.first().copied());
    let (want, _) = wanted((path.speed, false), next_link.map(|k| rail_speed(nav, k).0), r.off, (false, false));
    let (q, dir) = path.at(path.t);
    let ahead = {
        let next_start = nav.curves.get(to).map(|c| c.eval(to_t).0);
        search_ahead(r, env, me, q, dir, Some(path.geom.run()), (None, Some(to)), next_start, [Some(GatherRail::RunTime), Some(GatherRail::Lane(to)), None], to_t)
    };
    let dest = r.ghost.as_ref().and_then(|g| g.dest_entry());
    let (mut a, chosen) = control(r, r.v, want, half_len, &ahead, env.others, None, None, dest, dt);
    r.slot = chosen;
    if env.hold {
        a = -r.ty.max_decel;
    }
    ghost_integrate(r, a, dt, env.clock);
    let d = r.v * dt;
    let g = r.ghost.as_mut().unwrap();
    let p = g.lc.as_mut().unwrap();
    let over = p.step(d);
    let pose = p.pose();
    let Some(rest) = over else { return pose };
    g.lc = None;
    g.turned = true;
    r.next = None;
    if let Some(k) = next_link {
        r.link = k;
        r.set_phase(nav, env.lanes, to_t);
        let c = &nav.curves[to];
        r.ds = set_rail_speed(c, to_t);
        r.ds_lane = to;
        if rest > 0.0 {
            r.set_phase(nav, env.lanes, carry_phase(c, to_t, rest));
        }
    }
    r.pose(nav)
}

/// What the cruising side is told (drained once a frame, `Vehicles::drain_ghost_events`).
#[derive(Clone, Debug, PartialEq)]
pub enum GhostEvent {
    /// the owner's attach (vfunc 35): the request made (status 0, with the ghost) or failed (3 timed out, 5 no ground)
    Attached { owner: u32, request: i32, status: i32, ghost: Option<u32> },
    /// the owner's event (vfunc 38, from the car's release): the ghost removed for `reason` (1 the route's end, 2 off
    /// the route, 5 the brain's own), `rail` the last ordinary lane it was on (car +400; None: none)
    Released { owner: u32, ghost: u32, reason: i32, rail: Option<u32> },
}

/// A ghost: a car of the proxy pool (not drawn, no collider, never unspawned) on its rider.
#[derive(Clone, Debug)]
pub struct GhostCar {
    pub id: u32,
    pub owner: u32,
    pub rider: Rider,
    /// car +400 (F-46062)
    pub rail: Option<u32>,
    /// made this frame: its start speed is still to be set (F-30683)
    fresh: bool,
}

/// The ghosts' side of the vehicles.
#[derive(Clone, Debug, Default)]
pub struct GhostLive {
    pub cars: Vec<GhostCar>,
    pub events: Vec<GhostEvent>,
    next_car: u32,
    next_beh: u32,
    ty: Option<AmbType>,
    /// (tests) the spawn probe answered yes without a world
    pub(crate) ground_always: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Ghosts {
    pub pool: GhostPool,
    pub live: GhostLive,
}

impl Ghosts {
    /// The proxy type and its budget, once the data folder is known.
    fn ensure_type(&mut self) {
        if self.live.ty.is_none() {
            if let Some(root) = DATA_ROOT.get() {
                let t = AmbType::load(root, PROXY_TYPE);
                self.pool.budget = t.instances as i32;
                self.live.ty = Some(t);
            }
        }
    }
}

/// The railnet tables of the city (for the route's lanes), made once the data folder is known.
pub fn ghost_net() -> Option<&'static crate::railroute::RailNet> {
    static NET: std::sync::OnceLock<Option<crate::railroute::RailNet>> = std::sync::OnceLock::new();
    let root = DATA_ROOT.get()?;
    NET.get_or_init(|| crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).ok().map(|r| crate::railroute::RailNet::parse(&r))).as_ref()
}

/// The pool's world in the game.
struct LiveWorld<'a> {
    g: &'a mut GhostLive,
    shared: &'a mut GMwc,
    /// the second shared MWC (F-77422's three draws for the ghost car too)
    shared2: &'a mut GMwc,
    nav: Option<(&'static NavSys, &'static Lanes, &'static crate::railroute::RailNet)>,
    col: Option<&'a WorldCollider>,
    clock: f32,
    /// inside the brain's cancel: its attach (status 7) is not sent (the brain sets its own state after it)
    cancelling: bool,
}

impl PoolWorld for LiveWorld<'_> {
    fn shared(&mut self) -> &mut GMwc {
        self.shared
    }
    fn lane_point(&mut self, lane: u32, t: f32) -> (Vec3, Vec3) {
        match self.nav.and_then(|(nav, _, _)| nav.curves.get(lane as usize)).filter(|c| c.pts.len() >= 2) {
            Some(c) => (crate::railroute::point(c, t), crate::railroute::deriv(c, t)),
            None => (Vec3::ZERO, Vec3::X),
        }
    }
    fn car_get(&mut self) -> u32 {
        // (the proxy pool's size is open: a car whenever the type's budget allows)
        self.g.next_car += 1;
        self.g.next_car
    }
    fn car_back(&mut self, _car: u32) {}
    fn beh_get(&mut self) -> u32 {
        self.g.next_beh += 1;
        self.g.next_beh
    }
    fn beh_back(&mut self, _beh: u32) {}
    fn car_beh(&mut self, car: u32) -> u32 {
        self.g.cars.iter().any(|c| c.id == car) as u32
    }
    fn beh_drop(&mut self, _beh: u32) {}
    fn probe(&mut self, _car: u32, _seed: u32, m: &[Vec3; 4]) -> bool {
        self.g.ground_always || self.col.is_some_and(|c| probe_ground(c, m[3]).is_some())
    }
    fn car_init(&mut self, _rec: usize, r: &PoolRec) {
        let (Some((nav, lanes, net)), Some(o)) = (self.nav, r.order.as_ref()) else { return };
        let Some(&link) = lanes.links.get(o.start.0 as usize).and_then(|l| l.first()) else { return };
        let ty = self.g.ty.unwrap_or_default();
        let mut rider = Rider::new(nav, ty, link, 0.0, r.car);
        rider.set_phase(nav, lanes, o.start.1);
        // F-92604 for the proxy pool: the car's MWC seeded from the record's draw, its life 8 + 4U and offset timer
        // 3 + 3U; no start delay, accel bonus or offset; the standing gap (2 + 5) / 2
        let mut rng = mwc_seed(r.seed);
        rider.life = ((rng.next() & 0x7F_FFFF) as f32).mul_add(f32::from_bits(0x3500_0000), 8.0);
        rider.off_timer = ((rng.next() & 0x7F_FFFF) as f32).mul_add(f32::from_bits(0x34C0_0000), 3.0);
        // F-77422's hour and blink offsets from the second shared MWC (a proxy car: no +444 / +428 draws; the ghost has
        // no lamps in the port, the three draws keep that generator's order)
        for _ in 0..3 {
            self.shared2.next();
        }
        rider.off = 0.0;
        rider.bonus = 0.0;
        rider.wait = 0.0;
        rider.gap = 3.5;
        rider.born = self.clock;
        let mut ride = GhostRide {
            route: o.route.clone(),
            track: crate::ghostroute::Track::default(),
            flags: if o.set_speed.is_some() { 0x40 } else { 0 } | if o.dest_speed.is_some() { 0x20 } else { 0 },
            set_speed: o.set_speed,
            dest_speed: o.dest_speed.unwrap_or(0.0),
            rng,
            turned: false,
            on3: false,
            dest: None,
            change: None,
            lc: None,
        };
        // behaviour slot 10 (F-28180) -> slot 16: the next lane by the route (F-74277), its indices -1
        let next = crate::ghostroute::route_next(&ride.route, net, nav, ride.track.last, o.start.0, &mut ride.rng, self.shared);
        rider.next = next.and_then(|l| lanes.links.get(l as usize).and_then(|x| x.first().copied()));
        rider.ghost = Some(Box::new(ride));
        self.g.cars.push(GhostCar { id: r.car, owner: r.owner, rider, rail: None, fresh: true });
    }
    fn car_release(&mut self, car: u32, reason: u32) {
        if let Some(k) = self.g.cars.iter().position(|c| c.id == car) {
            let c = self.g.cars.remove(k);
            self.g.events.push(GhostEvent::Released { owner: c.owner, ghost: c.id, reason: reason as i32, rail: c.rail });
        }
    }
    fn notify(&mut self, rec: usize, owner: u32, vf: u8, status: u32, car: u32) {
        // (vfunc 36, the removal's notify to the racer: its effect is open)
        if vf == 35 && !self.cancelling {
            self.g.events.push(GhostEvent::Attached { owner, request: rec as i32, status: status as i32, ghost: (status == 0 && car != 0).then_some(car) });
        }
    }
}

impl Vehicles {
    /// The cruising side's events since the last call.
    pub fn drain_ghost_events(&mut self) -> Vec<GhostEvent> {
        std::mem::take(&mut self.ghosts.live.events)
    }

    /// F-47175's view of a car's behaviour (+452: a trailer and a parked car have none): its rails +188 / +192 - a
    /// lane rider its lane and next lane; a car on a run-time rail (states 6 - 9) that rail (its own: its index) and no
    /// next (+192 there is not traced); otherwise none.
    fn sel_behaviour(&self, nav: &NavSys, k: usize, r: Option<&Rider>) -> Option<(Option<SelRail>, Option<SelRail>)> {
        let r = r.filter(|r| !r.ty.trailer)?;
        Some(match r.state {
            1 => (Some(SelRail::Lane(nav.links[r.link].lane)), r.next.map(|n| SelRail::Lane(nav.links[n].lane))),
            6..=9 => (Some(SelRail::RunTime(k as u32)), None),
            _ => (None, None),
        })
    }

    /// F-31543 on a lane: its road (F-86691) a freeway (speed >= FREEWAY_SPEED).
    fn lane_freeway(&self, nav: &NavSys, lane: usize) -> bool {
        let (Some(sp), Some(&k)) = (self.spawner.as_ref(), lanes(nav).links.get(lane).and_then(|l| l.first())) else { return false };
        sp.road_of(nav, k).is_some_and(|x| sp.roads[x as usize].speed >= FREEWAY_SPEED)
    }

    /// An ambient car's obstacle size (F-46284, `obstacle_size_of`: its type's bound - a rider's type, a parked car's by
    /// its model); None for the player, police and racers (their obstacles' slot 3 is not decoded).
    fn obstacle_size(&self, c: &super::CarInstance) -> Option<(f32, f32)> {
        match (c.rail.as_ref(), c.parked) {
            (Some(r), _) => Some(obstacle_size_of(&r.ty)),
            (None, Some(_)) => Some(self.amb_types.get(&c.model.name).map_or((3.0, 6.0), obstacle_size_of)),
            _ => None,
        }
    }

    /// The other cars as a rider looks at them (as `ride_traffic` makes them).
    fn ghost_others(&self, nav: &NavSys) -> Vec<Other> {
        self.cars
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let (hl, hw) = half_size(c);
                let r = c.rail.as_ref();
                let on = r.filter(|r| r.state == 1).map(|r| {
                    let l = &nav.links[r.link];
                    (l.lane, r.next.map(|n| nav.links[n].lane), rail_speed(nav, r.link).1)
                });
                Other {
                    pos: c.state.pos,
                    fwd: c.state.forward(),
                    right: c.state.right(),
                    back: glam::Mat3::from_quat(c.state.rot).z_axis,
                    row0: glam::Mat3::from_quat(c.state.rot).x_axis,
                    inst: instance_matrix(c),
                    vel: c.state.vel,
                    half_len: hl,
                    half_wid: hw,
                    size: self.obstacle_size(c),
                    speed: r.map_or(0.0, |r| r.v),
                    beh: self.sel_behaviour(nav, k, r),
                    lane640: if c.is_player { self.driver_lanes.get(&c.uid).and_then(|d| d.best).map(|(l, _)| (l, self.lane_freeway(nav, l))) } else { None },
                    idle: if c.is_player { self.player_idle } else { 0.0 },
                    on,
                    player: c.is_player,
                    ambient: r.is_some() || c.parked.is_some(),
                    active: r.is_some_and(|r| r.state == 4) || (r.is_none() && c.parked.is_none()),
                    cab_or_trailer: r.is_some_and(|r| r.ty.cab || r.ty.trailer),
                    key: r.map_or(0, |r| r.rng_id()),
                    kind: crate::brainturn::ObjectInfo::from_car(0, c).kind,
                    nearest: r.and_then(|r| r.kept).and_then(|(j, _)| self.cars.get(j)?.rail.as_ref().map(|x| x.rng_id())),
                }
            })
            .collect()
    }

    /// The ghosts' part of the ambient manager's frame: the request pool's step (F-52865, before the cars' updates),
    /// the new ghosts' start speed (F-30683), then each ghost's update on its lane with the route's hooks: the next lane
    /// at a change of rail (F-74277), the indices (F-18846), the route lane change's arguments (F-24481), the release
    /// tests (F-35182: 1 the route's end F-44936, 2 off the route F-20872 -> F-51512) and car +400 (F-46062).
    pub(super) fn ghost_frame(&mut self, nav: &NavSys, col: &WorldCollider, dt: f32) {
        if self.ghosts.pool.count <= 0 && self.ghosts.live.cars.is_empty() {
            return;
        }
        self.ghosts.ensure_type();
        let nav = rail_nav(nav);
        let lanes = lanes(nav);
        let net = ghost_net();
        let clock = self.clock;
        {
            let Vehicles { ghosts, park_rng, amb_rng2, .. } = self;
            let mut w = LiveWorld { g: &mut ghosts.live, shared: park_rng, shared2: amb_rng2, nav: net.map(|n| (nav, lanes, n)), col: Some(col), clock, cancelling: false };
            ghosts.pool.step(dt, &mut w);
        }
        for k in 0..self.ghosts.live.cars.len() {
            if !self.ghosts.live.cars[k].fresh {
                continue;
            }
            let r = &self.ghosts.live.cars[k].rider;
            let pos = r.pos(nav);
            // (bIsProxy: never removed by the rule; F-30683 on the ghost's own rail)
            let v = self.path_start_speed(nav, r, pos, (0.0, 0.0), None, None).unwrap_or(0.0);
            let g = &mut self.ghosts.live.cars[k];
            g.rider.v = v;
            g.fresh = false;
        }
        let Some(net) = net else { return };
        if self.ghosts.live.cars.is_empty() {
            return;
        }
        let others = self.ghost_others(nav);
        let blockers = self.frame_blockers(nav);
        let shared = std::cell::Cell::new(self.park_rng);
        let (rail_cars, grid) = self.gather_lists(nav);
        let pool0 = std::cell::Cell::new(self.cars.iter().filter(|c| c.rail.as_ref().is_some_and(holds_pool0)).count() + self.ghosts.live.cars.iter().filter(|g| holds_pool0(&g.rider)).count());
        let root = self.data_root.clone();
        let level = &RailLevel::new(col, &self.cars, &root, &self.walkers, &self.level);
        let env = Env { nav, lanes, col, others: &others, signals: &self.signals, blockers: &blockers, clock, event: self.race_event, player_ahead: None, hold: self.frozen || self.grid_hold.is_some(), parity: false, paths: &[], shared: &shared, rail_cars: &rail_cars, grid: &grid, pool0: &pool0, level };
        let mut removes: Vec<(u32, u32, u32)> = Vec::new();
        for g in self.ghosts.live.cars.iter_mut() {
            // F-58860: the proxy car's place is its owner's vehicle's (none: its own)
            let me = self.cars.iter().position(|c| c.uid == g.owner).unwrap_or(usize::MAX);
            let owner_pos = (me != usize::MAX).then(|| self.cars[me].state.pos);
            let r = &mut g.rider;
            if r.state != 1 {
                continue;
            }
            let pos = owner_pos.unwrap_or_else(|| ghost_point(r, nav).0);
            if let Some(ride) = r.ghost.as_mut() {
                ride.dest = (ride.flags & crate::ghostroute::FLAG_DEST_SPEED != 0 && crate::ghostroute::end_plane(&ride.route, pos) > 0.0).then(|| (ride.dest_speed, crate::ghostroute::dest_dist(&ride.route, pos)));
            }
            // (the proxy type has no bound: half length 0)
            step(r, &env, me, 0.0, true, dt);
            let lane = nav.links[r.link].lane as u32;
            let (t, v) = (r.t, r.v);
            let pos = owner_pos.unwrap_or_else(|| ghost_point(r, nav).0);
            let Some(ride) = r.ghost.as_mut() else { continue };
            // (on the lane-change rail the follower's rail is a run-time one: no node holds it, no change, not ordinary)
            let cur = ride.lc.is_none().then_some(lane);
            if ride.turned {
                ride.turned = false;
                let mut m = shared.get();
                let n = crate::ghostroute::route_next(&ride.route, net, nav, ride.track.last, lane, &mut ride.rng, &mut m);
                shared.set(m);
                r.next = n.and_then(|l| lanes.links.get(l as usize).and_then(|x| x.first().copied()));
            }
            crate::ghostroute::track(&ride.route, net, &mut ride.track, cur);
            let next = r.next.map(|n| nav.links[n].lane as u32);
            ride.change = cur.and_then(|l| crate::ghostroute::lane_change(&ride.route, net, nav, &ride.track, l, next, t, v));
            // F-32118: the change taken at once - the follower on the lane-change rail from phase 0, the target lane its
            // next rail (the pool's rail: none only when the pool is empty, the port's pool never is)
            if let Some(p) = ride.change.as_ref().and_then(|c| lane_change_rail(nav, lanes, c)) {
                r.next = p.end.and_then(|(l, _)| lanes.links.get(l).and_then(|x| x.first().copied()));
                ride.lc = Some(p);
            }
            let ordinary = ride.lc.is_none() && net.lanes.get(lane as usize).is_some_and(|l| l.flags & 0x1800 == 0);
            let reason = if crate::ghostroute::route_end(&ride.route, ride.track.last, pos, ride.flags, ride.dest_speed, v) {
                1
            } else if crate::ghostroute::off_route(&ride.route, &ride.track, ordinary) {
                2
            } else {
                0
            };
            if ordinary {
                g.rail = Some(lane);
            }
            if reason != 0 {
                removes.push((g.id, reason, g.owner));
            }
        }
        self.park_rng = shared.get();
        for (id, reason, owner) in removes {
            self.ghosts.pool.submit_remove(id, reason, owner);
        }
    }

    /// A ghost as the brain reads it: the matrix (its owner's vehicle's, F-58860 / the proxy pool; without one the
    /// spawn rule at its lane's point), the lane and phase it rides, its speed (+412).
    pub fn ghost_view(&self, ghost: u32) -> Option<crate::cruiser::GhostView> {
        let g = self.ghosts.live.cars.iter().find(|g| g.id == ghost)?;
        let nav = rail_nav_made()?;
        let r = &g.rider;
        let (p, f) = ghost_point(r, nav);
        let matrix = match self.cars.iter().find(|c| c.uid == g.owner) {
            Some(c) => instance_matrix(c),
            None => spawn_pose(p, f),
        };
        // (on the lane-change rail: no lane - the brain's state 5 reads that rail's slot 8 at its phase, `run_rail`)
        let lc = r.ghost.as_ref().and_then(|x| x.lc.as_ref());
        let rail = lc.is_none().then(|| (nav.links[r.link].lane as u32, r.t));
        let run_rail = lc.map(|p| match &p.geom {
            Geom::Curve(c) => c.eval_pair(p.t),
            Geom::Tcp(_) => p.eval(p.t),
        });
        Some(crate::cruiser::GhostView { matrix, rail, run_rail, speed: r.v })
    }
}

/// The ambient traffic's side of the cruising brain (cruiser.rs): `owner` is the owner car's `uid` in the vehicles.
impl crate::cruiser::GhostTraffic for Vehicles {
    fn present(&self) -> bool {
        ghost_net().is_some()
    }
    fn request_ghost(&mut self, r: crate::cruiser::GhostRequest) -> i32 {
        self.ghosts.ensure_type();
        let order = (!r.route.nodes.is_empty()).then(|| GhostOrder { route: r.route, start: r.start, set_speed: r.set_speed, dest_speed: r.dest_speed });
        let owner = if order.is_some() { r.owner } else { 0 };
        self.ghosts.pool.submit(order, owner)
    }
    fn cancel_request(&mut self, id: i32) {
        let clock = self.clock;
        let Vehicles { ghosts, park_rng, amb_rng2, .. } = self;
        let mut w = LiveWorld { g: &mut ghosts.live, shared: park_rng, shared2: amb_rng2, nav: None, col: None, clock, cancelling: true };
        ghosts.pool.cancel(id, &mut w);
    }
    fn release_ghost(&mut self, ghost: u32) {
        if let Some(owner) = self.ghosts.live.cars.iter().find(|g| g.id == ghost).map(|g| g.owner) {
            self.ghosts.pool.submit_remove(ghost, 5, owner);
        }
    }
    fn ghost(&self, ghost: u32) -> Option<crate::cruiser::GhostView> {
        self.ghost_view(ghost)
    }
    fn rng(&mut self) -> &mut GMwc {
        &mut self.park_rng
    }
}

#[cfg(test)]
mod ghost_tests {
    use super::*;
    use crate::cruiser::GhostTraffic;

    /// (a word that would read as a code address is split into 16-bit halves in the testdata: xxxx_xxxx)
    fn hx(s: &str) -> u32 {
        u32::from_str_radix(&s.replace('_', ""), 16).unwrap()
    }
    fn fx(s: &str) -> f32 {
        f32::from_bits(hx(s))
    }
    fn v3(w: &[&str]) -> Vec3 {
        Vec3::new(fx(w[0]), fx(w[1]), fx(w[2]))
    }

    /// The pool's world replayed from the original's log: each call takes the next line, which must be that call
    /// with the same arguments; the answers (a car, a behaviour, the probe's, the lane's point) are the log's.
    struct Replay<'a> {
        lines: Vec<Vec<&'a str>>,
        at: usize,
        shared: GMwc,
        beh: hashbrown::HashMap<u32, u32>,
        owner: hashbrown::HashMap<u32, u32>,
        calls: hashbrown::HashMap<&'static str, usize>,
    }

    impl<'a> Replay<'a> {
        fn take(&mut self, tag: &'static str) -> Vec<&'a str> {
            let l = self.lines.get(self.at).cloned().unwrap_or_default();
            assert_eq!(l.first().copied(), Some(tag), "line {}: the port called {tag}, the original logged {:?}", self.at + 1, l);
            self.at += 1;
            *self.calls.entry(tag).or_default() += 1;
            l
        }
    }

    impl PoolWorld for Replay<'_> {
        fn shared(&mut self) -> &mut GMwc {
            &mut self.shared
        }
        fn lane_point(&mut self, lane: u32, t: f32) -> (Vec3, Vec3) {
            let at = self.at + 1;
            let l = self.take("L");
            assert_eq!((l[1].parse::<u32>().unwrap(), hx(l[2])), (lane, t.to_bits()), "line {at}");
            (v3(&l[3..6]), v3(&l[6..9]))
        }
        fn car_get(&mut self) -> u32 {
            hx(self.take("C")[1])
        }
        fn car_back(&mut self, car: u32) {
            let at = self.at + 1;
            assert_eq!(hx(self.take("c")[1]), car, "line {at}");
        }
        fn beh_get(&mut self) -> u32 {
            hx(self.take("B")[1])
        }
        fn beh_back(&mut self, beh: u32) {
            let at = self.at + 1;
            assert_eq!(hx(self.take("b")[1]), beh, "line {at}");
        }
        fn car_beh(&mut self, car: u32) -> u32 {
            self.beh.get(&car).copied().unwrap_or(0)
        }
        fn beh_drop(&mut self, beh: u32) {
            let at = self.at + 1;
            assert_eq!(hx(self.take("D")[1]), beh, "line {at}");
        }
        fn probe(&mut self, car: u32, seed: u32, m: &[Vec3; 4]) -> bool {
            let at = self.at + 1;
            let l = self.take("P");
            let got: Vec<u32> = m.iter().flat_map(|v| v.to_array()).map(f32::to_bits).collect();
            let want: Vec<u32> = l[3..15].iter().map(|s| hx(s)).collect();
            assert_eq!((hx(l[1]), hx(l[2]), &got), (car, seed, &want), "line {at}: car, seed, matrix");
            l[15] == "1"
        }
        fn car_init(&mut self, _rec: usize, r: &PoolRec) {
            let at = self.at + 1;
            let l = self.take("I");
            assert_eq!((hx(l[1]), hx(l[2]), hx(l[3])), (r.car, r.beh, r.seed), "line {at}");
            self.beh.insert(r.car, r.beh);
            self.owner.insert(r.car, r.owner);
        }
        fn car_release(&mut self, car: u32, reason: u32) {
            let at = self.at + 1;
            let l = self.take("K");
            assert_eq!((hx(l[1]), hx(l[2])), (car, reason), "line {at}");
        }
        fn notify(&mut self, _rec: usize, owner: u32, vf: u8, status: u32, car: u32) {
            let at = self.at + 1;
            let l = self.take("N");
            assert_eq!((hx(l[1]), l[2].parse::<u8>().unwrap(), hx(l[3]), hx(l[4])), (owner, vf, status, car), "line {at}");
        }
    }

    /// The request pool against the original's code run offline (tools/ghost_pool_reference.py): 150 scenarios of
    /// random requests, cancels, removals and steps with the world's answers drawn at random, every call the code made
    /// with its arguments in order (the spawn matrix bit for bit, the shared MWC's draws) and the pool's count, most,
    /// the type's budget and the shared MWC after each step; then the integrator F-58614 on 2,000 random cars and the
    /// seed F-37317 on 500 values.
    #[test]
    fn ghost_pool_matches_the_original() {
        let text = include_str!("testdata/ghost_pool_ref.txt");
        let mut w = Replay { lines: text.lines().map(|l| l.split_whitespace().collect()).collect(), at: 0, shared: GMwc { lo: 0, carry: 0 }, beh: Default::default(), owner: Default::default(), calls: Default::default() };
        let mut pool = GhostPool::new(0);
        let (mut ops, mut steps, mut g_n, mut m_n) = (0usize, 0usize, 0usize, 0usize);
        while w.at < w.lines.len() {
            let n = w.at + 1;
            let l = w.lines[w.at].clone();
            w.at += 1;
            match l[0] {
                "Z" => {
                    pool = GhostPool::new(hx(l[1]) as i32);
                    w.shared = GMwc { lo: hx(l[2]), carry: hx(l[3]) };
                    w.beh.clear();
                    w.owner.clear();
                }
                "q" => {
                    let opt = |s: &str| (hx(s) != 0xFFFF_FFFF).then(|| fx(s));
                    let order = (l[2] == "1").then(|| GhostOrder { route: Default::default(), start: (l[3].parse().unwrap(), fx(l[4])), set_speed: opt(l[5]), dest_speed: opt(l[6]) });
                    let id = pool.submit(order, hx(l[1]));
                    assert_eq!(id, hx(l[9]) as i32, "line {n}");
                    ops += 1;
                }
                "x" => {
                    pool.cancel(hx(l[1]) as i32, &mut w);
                    ops += 1;
                }
                "r" => {
                    let car = hx(l[1]);
                    let owner = w.owner.get(&car).copied().unwrap_or(0);
                    pool.submit_remove(car, hx(l[2]), owner);
                    ops += 1;
                }
                "s" => {
                    pool.step(fx(l[1]), &mut w);
                    let at = w.at + 1;
                    let s = w.take("S");
                    let want: Vec<u32> = s[1..7].iter().map(|x| hx(x)).collect();
                    assert_eq!(vec![pool.count as u32, pool.max as u32, pool.budget as u32, pool.used as u32, w.shared.lo, w.shared.carry], want, "line {at}: count, max, budget, used, shared MWC");
                    steps += 1;
                }
                "G" => {
                    let i: Vec<u32> = l[1..16].iter().map(|x| if x.len() <= 2 { u32::from_str_radix(x, 16).unwrap() } else { hx(x) }).collect();
                    let f = |k: usize| f32::from_bits(i[k]);
                    let set = (i[5] != 0 && i[6] & 0x40 != 0).then(|| f(7));
                    let mut s = CarSpeed { v: f(0), off: f(14), timer: f(10), born: f(9), rng: GMwc { lo: i[11], carry: i[12] } };
                    car_speed(&mut s, f(1), f(2), f(3), f(4), set, f(8), i[13] & 0x20 != 0);
                    let want: Vec<u32> = l[17..22].iter().map(|x| hx(x)).collect();
                    assert_eq!(vec![s.v.to_bits(), s.off.to_bits(), s.timer.to_bits(), s.rng.lo, s.rng.carry], want, "line {n}: v, offset, timer, MWC");
                    g_n += 1;
                }
                "M" => {
                    let m = mwc_seed(hx(l[1]));
                    assert_eq!((m.lo, m.carry, hx(l[5]), hx(l[6])), (hx(l[3]), hx(l[4]), 0, 0), "line {n}");
                    m_n += 1;
                }
                t => panic!("line {n}: the original logged {t} where the port expected an operation"),
            }
        }
        let c = |k: &str| w.calls.get(k).copied().unwrap_or(0);
        eprintln!(
            "ghost pool: {ops} ops, {steps} steps; calls L {} C {} c {} B {} b {} D {} P {} I {} K {} N {}; {g_n} integrator cases, {m_n} seeds",
            c("L"), c("C"), c("c"), c("B"), c("b"), c("D"), c("P"), c("I"), c("K"), c("N")
        );
        assert!(steps > 1000 && c("P") > 400 && c("K") > 100 && g_n == 2000 && m_n == 500);
    }

    fn load() -> Option<(std::path::PathBuf, NavSys)> {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).ok()?;
        Some((root, NavSys::parse(&res).ok()?))
    }

    /// The play test's route (racer AI log c12, cruiser 1305): from lane 3542 at 0.5458 to (-129.559, 20.5996, -810.73),
    /// 17 nodes, the first (record 1001) with lanes 3542 3543 3548 3549 3550, then record 1067 with the junction lane
    /// 3890 - which starts only at the end of 3549. 3542 leads to 3548, whose end leads only to 3881 (off the route):
    /// the ghost must change from 3548 to 3549 (F-24481 -> F-32118). Before the lane-change rail was ported it rode
    /// 3542 -> 3548 -> 3881 and was released with reason 2; now it rides the rail onto 3549, then 3890, and reaches the
    /// route's end (reason 1).
    #[test]
    fn the_ghost_changes_lane_onto_the_junction_lane() {
        let Some((root, nav0)) = load() else { return };
        let _ = AmbType::load(&root, PROXY_TYPE);
        let nav = rail_nav(&nav0);
        let Some(net) = ghost_net() else { return };
        let col = WorldCollider::default();
        let route = crate::railroute::route(net, nav, 3542, 0.5458, Vec3::new(-129.559, 20.5996, -810.73));
        let firsts: Vec<(u32, Vec<u32>)> = route.nodes.iter().take(2).map(|n| (n.rec, n.lanes.clone())).collect();
        assert_eq!(firsts, vec![(1001, vec![3542, 3543, 3548, 3549, 3550]), (1067, vec![3890])]);
        assert_eq!(route.nodes.len(), 17);
        let mut v = Vehicles::new();
        v.ghosts.live.ground_always = true;
        let id = v.request_ghost(crate::cruiser::GhostRequest { owner: 77, route: route.clone(), start: (3542, 0.5458), set_speed: None, dest_speed: None });
        assert!(id >= 0);
        let dt = 1.0 / 60.0;
        let (mut ghost, mut done, mut lanes_seen, mut rail_frames) = (None, None, Vec::<u32>::new(), 0);
        for _ in 0..60 * 600 {
            v.clock += dt;
            v.ghost_frame(&nav0, &col, dt);
            for e in v.drain_ghost_events() {
                match e {
                    GhostEvent::Attached { ghost: g, .. } => ghost = g,
                    GhostEvent::Released { reason, rail, .. } => done = Some((reason, rail)),
                }
            }
            if done.is_some() {
                break;
            }
            let Some(gv) = ghost.and_then(|g| v.ghost(g)) else { continue };
            match gv.rail {
                Some((l, _)) => {
                    if lanes_seen.last() != Some(&l) {
                        lanes_seen.push(l);
                    }
                }
                None => {
                    assert!(gv.run_rail.is_some());
                    rail_frames += 1;
                }
            }
        }
        eprintln!("lanes {lanes_seen:?}, {rail_frames} frames on the lane-change rail, released {done:?}");
        assert!(rail_frames > 0, "the change was taken");
        let k = lanes_seen.iter().position(|&l| l == 3549).expect("onto 3549");
        assert_eq!(lanes_seen.get(k + 1), Some(&3890));
        assert!(!lanes_seen.contains(&3881));
        let last = route.nodes.last().unwrap();
        assert!(matches!(done, Some((1, Some(r))) if net.recs[last.rec as usize].lanes.contains(&r)), "{done:?}");
    }

    /// A ghost on a route through the vehicles' hooks: requested, attached in the next frame, riding the route's lanes
    /// (every lane it takes is a lane of the route's records, F-74277 / F-18846) until released at the route's end
    /// (reason 1, the rail its last lane) - or off the route (reason 2); lane changes the route asks for are taken
    /// (F-32118).
    #[test]
    fn a_ghost_follows_its_route_and_is_released_at_the_end() {
        let Some((root, nav0)) = load() else { return };
        let _ = AmbType::load(&root, PROXY_TYPE);
        let nav = rail_nav(&nav0);
        let lanes = lanes(nav);
        let Some(net) = ghost_net() else { return };
        let col = WorldCollider::default();
        let (mut ends, mut offs, mut tried, mut lanes_ridden) = (0, 0, 0, 0usize);
        let mut v = Vehicles::new();
        v.ghosts.live.ground_always = true;
        let mut shared = GMwc { lo: 0x1234_5678, carry: 0x9ABC };
        for start in (0..nav.curves.len() as u32).step_by(97) {
            if ends >= 5 || tried >= 60 {
                break;
            }
            if lanes.links.get(start as usize).map_or(true, |l| l.is_empty()) || net.lanes[start as usize].flags & 0x1800 != 0 {
                continue;
            }
            // the destination: four ordinary lanes on, half way along
            let mut l = start;
            for _ in 0..4 {
                match crate::ghostroute::ordinary_next(net, nav, l, &mut shared) {
                    Some(n) => l = n,
                    None => break,
                }
            }
            if l == start {
                continue;
            }
            let dest = crate::railroute::point(&nav.curves[l as usize], 0.5);
            let route = crate::railroute::route(net, nav, start, 0.1, dest);
            if route.nodes.len() < 2 {
                continue;
            }
            tried += 1;
            let id = v.request_ghost(crate::cruiser::GhostRequest { owner: 77, route: route.clone(), start: (start, 0.1), set_speed: Some(15.0), dest_speed: None });
            assert!(id >= 0);
            let dt = 1.0 / 60.0;
            let mut ghost = None;
            let mut done = None;
            let mut start_v = -1.0f32;
            for _ in 0..60 * 120 {
                v.clock += dt;
                v.ghost_frame(&nav0, &col, dt);
                for e in v.drain_ghost_events() {
                    match e {
                        GhostEvent::Attached { owner, request, status, ghost: g } => {
                            assert_eq!((owner, request, status), (77, id, 0));
                            ghost = g;
                        }
                        GhostEvent::Released { owner, ghost: g, reason, rail } => {
                            assert_eq!((owner, Some(g)), (77, ghost));
                            done = Some((reason, rail));
                        }
                    }
                }
                if done.is_some() {
                    break;
                }
                let Some(gv) = ghost.and_then(|g| v.ghost(g)) else { continue };
                if start_v < 0.0 {
                    start_v = gv.speed;
                }
                // (on the route lane change's rail: no lane)
                let Some((lane, _)) = gv.rail else { continue };
                // (SetSpeed caps the integrator's acceleration only: the start speed is the rail's, F-30683)
                assert!(gv.speed <= 15.0 || gv.speed <= start_v, "SetSpeed caps the speed");
                // a lane it rides while the route holds it is a lane of a route node's record
                let g = v.ghosts.live.cars.iter().find(|c| Some(c.id) == ghost).unwrap();
                let t = g.rider.ghost.as_ref().unwrap().track;
                if t.cur >= 0 {
                    assert!(net.recs[route.nodes[t.cur as usize].rec as usize].lanes.contains(&lane));
                    lanes_ridden += 1;
                }
            }
            match done {
                Some((1, rail)) => {
                    let last = route.nodes.last().unwrap();
                    assert!(rail.is_some_and(|r| net.recs[last.rec as usize].lanes.contains(&r)), "released at the end on a lane of the last node");
                    ends += 1;
                }
                Some((2, _)) => offs += 1,
                other => panic!("route from lane {start}: {other:?}"),
            }
        }
        eprintln!("ghost routes: {tried} tried, {ends} released at the end, {offs} off the route, {lanes_ridden} frames on route lanes");
        assert!(ends >= 3, "{ends} of {tried} routes ended");
    }
}

/// F-77422 (with F-60230, F-92604, F-37317, F-66400) run whole offline (tools/spawn_init_reference.py ->
/// testdata/spawn_init_ref.txt): the car MWC's and the two shared MWCs' draws, every word and both generators equal.
#[cfg(test)]
mod spawn_init_tests {
    use super::*;

    #[test]
    fn init_draws_match_the_original() {
        let hx = |t: &str| u32::from_str_radix(&t.replace('_', ""), 16).unwrap();
        let (mut n, mut spots, mut hitched, mut loops) = (0, 0, 0, 0);
        for line in include_str!("testdata/spawn_init_ref.txt").lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let i: Vec<u32> = a.split(' ').map(hx).collect();
            // (hex words; the hitch and the lookups decimal)
            let mut o: Vec<u32> = b.split(' ').map(hx).collect();
            o[19] = b.split(' ').nth(19).unwrap().parse().unwrap();
            let (flags, given, b292, found) = (i[5], i[6], i[7], i[8]);
            let mut shared2 = GMwc { lo: i[1], carry: i[2] };
            let mut shared = GMwc { lo: i[3], carry: i[4] };
            let (spot, cab, proxy, green) = (flags & 1 != 0, flags & 2 != 0, flags & 8 != 0, flags & 16 != 0);
            // F-92604's id: given (>= 0), else drawn when rec+836 is set (here the case's byte), else -1
            let mut looked = 0;
            let id = if flags & 32 != 0 {
                given
            } else if b292 != 0 {
                let r = id_draws(&mut shared, |_| {
                    looked += 1;
                    looked <= found
                });
                // (the stubs: type index 5, peer 0)
                (((r as u32) << 7) | 5) << 5
            } else {
                u32::MAX
            };
            let d = init_draws(i[0], &mut shared2, spot, cab, proxy, green);
            let want = [d.life.to_bits(), d.off_timer.to_bits(), d.wait.to_bits(), id, d.hour.to_bits(), 0, d.blink_wrap.to_bits(), d.blink_on.to_bits(), d.bonus.to_bits(), d.steer.to_bits(), d.hitch as u32, d.placed.lo, d.placed.carry, d.car.lo, d.car.carry, shared2.lo, shared2.carry, shared.lo, shared.carry, looked];
            assert_eq!(o, want, "line {}: {line}", n + 1);
            n += 1;
            spots += spot as u32;
            hitched += d.hitch as u32;
            loops += (looked > 1) as u32;
        }
        assert_eq!(n, 3000);
        eprintln!("spawn init: {n} cases, {spots} spots, {hitched} hitched, {loops} id loops past the first draw");
    }
    /// The traffic room test against the generated code run offline (tools/traffic_room_reference.py ->
    /// testdata/traffic_room_ref.txt): 6,000 cases - grids of 1 .. 8 x 1 .. 8 cells, the point on and off the grid,
    /// up to 16 objects of kinds 0 .. 7 about the spacing's edge or moving at, past, short of or along the sphere's edge
    /// a float step or two either way. The answer equal in every case.
    #[test]
    fn traffic_room_matches_original() {
        let txt = include_str!("testdata/traffic_room_ref.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let f = |s: &str| f32::from_bits(hex(s));
        let (mut cases, mut bad, mut blocked, mut by_motion) = (0, 0, 0, 0);
        for l in txt.lines().filter(|l| !l.is_empty()) {
            let (head, rest) = l.split_once(" | ").unwrap();
            let (objs, want) = rest.rsplit_once(" -> ").unwrap();
            let h: Vec<&str> = head.split_whitespace().collect();
            let grid = CrossGrid { min: [f(h[2]), f(h[3])], n: [hex(h[0]) as i32, hex(h[1]) as i32], bounded: true };
            let p = Vec3::new(f(h[4]), f(h[5]), f(h[6]));
            let o: Vec<&str> = objs.split_whitespace().collect();
            let list: Vec<RoomCar> = o
                .chunks(8)
                .map(|c| RoomCar { kind: hex(c[0]) as u8, pos: Vec3::new(f(c[1]), f(c[2]), f(c[3])), vel: Vec3::new(f(c[4]), f(c[5]), f(c[6])), extent: f(c[7]) })
                .collect();
            assert_eq!(list.len(), hex(h[9]) as usize);
            let got = traffic_room_blocked(&grid, p, &list, f(h[7]), f(h[8]));
            cases += 1;
            blocked += got as usize;
            let flat: Vec<RoomCar> = list.iter().map(|c| RoomCar { vel: Vec3::ZERO, extent: 0.0, ..*c }).collect();
            by_motion += (got && !traffic_room_blocked(&grid, p, &flat, 0.0, f(h[8]))) as usize;
            if got != (want.trim() == "1") {
                bad += 1;
                if bad <= 5 {
                    eprintln!("traffic room mismatch: {l} -> port {got}");
                }
            }
        }
        eprintln!("traffic room: {cases} cases ({blocked} no room, {by_motion} by a motion only), {bad} mismatches");
        assert_eq!(cases, 6000);
        assert_eq!(bad, 0);
    }
}

#[cfg(test)]
mod speed_rule_tests {
    use super::*;

    fn hexf(t: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(t, 16).unwrap())
    }

    /// F-58614's speed part (`speed_step`, the riders' `integrate`) against the original's first block run offline on
    /// 2,000 cases (work/verify-people/traffic run_traffic.py; accelerations -25 .. 12 and on the edges, speeds up to 1.3 x
    /// the cap, the stop speed's edge, dt 1/120 .. 1/20). The +244 cap (SetSpeed) and the offset timer are in
    /// `ghost_pool_matches_the_original` (`car_speed`, 2,000 more cases).
    #[test]
    fn integrator_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/traffic_integ_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<f32> = i.split_whitespace().map(hexf).collect();
            let v = speed_step(w[1], w[0], w[2], w[3], w[4]);
            assert_eq!(v.to_bits(), u32::from_str_radix(o, 16).unwrap(), "{line}");
            n += 1;
        }
        assert_eq!(n, 2000);
    }

    /// F-37906 (`lock_rule`) against the original's code on 1,500 cases (764 within 1e-4 of the speed threshold, the
    /// mean on fMaxDecel and one ulp either side, entries 0 / 1 / 2 / 7 in charge).
    #[test]
    fn lock_rule_matches_the_original() {
        let (mut n, mut yes) = (0, 0);
        for line in include_str!("testdata/traffic_lock_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let slot: usize = w[0].parse().unwrap();
            let v = Vec3::new(hexf(w[4]), hexf(w[5]), hexf(w[6]));
            let got = lock_rule(Some(slot), hexf(w[1]), w[2].parse().unwrap(), hexf(w[3]), v);
            assert_eq!(got, o == "1", "{line}");
            yes += got as u32;
            n += 1;
        }
        assert_eq!(n, 1500);
        assert!(yes > 100, "{yes}");
    }

    /// F-84582 (`newton_velocity`, the velocity +208 and its change +224) against the original's code on 3,520 steps
    /// (random walks of speed 0 .. 3 m/s, dt 1/30 and 1/30 .. 1/20).
    #[test]
    fn newton_velocity_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/traffic_vel_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<f32> = i.split_whitespace().map(hexf).collect();
            let want: Vec<u32> = o.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let (old, new, v0) = (Vec3::new(w[0], w[1], w[2]), Vec3::new(w[3], w[4], w[5]), Vec3::new(w[6], w[7], w[8]));
            let v = newton_velocity(old, new, w[9]);
            let dv = Vec3::new(vs(v.x, v0.x), vs(v.y, v0.y), vs(v.z, v0.z));
            let got = [v.x, v.y, v.z, dv.x, dv.y, dv.z].map(f32::to_bits);
            assert_eq!(got.to_vec(), want, "{line}");
            n += 1;
        }
        assert_eq!(n, 3520);
    }

    /// F-83943 (`distance_left`) against the original's code run whole (tools/traffic_left_reference.py) on 4,000 cases:
    /// no rail, rails with the curve bit 0x1000, 0x800 / 0x1800 and random words, the follower's byte with 0x80 / 0x40 /
    /// both / random, phases 0 / -0 / 1 / 1 - 1 ulp / out of range, lengths 0 .. 200, points equal and up to 150 m apart,
    /// last indices 0 .. 0xFFFF (the index asked for is the curve's +28 sign-extended).
    #[test]
    fn distance_left_matches_the_original() {
        let (mut n, mut lane) = (0, 0);
        for line in include_str!("testdata/traffic_left_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let o: Vec<u32> = o.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let f = |k: usize| f32::from_bits(w[k]);
            let v = |k: usize| Vec3::new(f(k), f(k + 1), f(k + 2));
            let rail = LeftRail { flags: w[1], byte: w[2] as u8, phase: f(3), len: f(4), point: v(5), start: v(8), end: v(11), last: v(14) };
            let got = distance_left((w[0] != 0).then_some(&rail));
            assert_eq!(got.to_bits(), o[0], "{line}");
            // the lane branch asks the curve for its last index (+28, sign-extended)
            let is_lane = w[0] != 0 && w[1] & 0x1000 == 0 && !(w[2] & 0xC0 == 0xC0);
            assert_eq!(o[1], if is_lane { w[17] as u16 as i16 as i32 as u32 } else { u32::MAX }, "{line}");
            lane += is_lane as u32;
            n += 1;
        }
        assert_eq!((n, lane), (4000, 1670));
    }

    /// F-46284 (the ambient obstacle's slot 3) against the original's code run whole in vector mode
    /// (tools/traffic_size_reference.py) on 2,000 cases: the proxy flag 0x20 with or without the holder (+52) and its
    /// owner (+48 +28), with or without an archetype, bounds with -0 and denormal corners. The instance as the original
    /// picks it, then `bound_extents`.
    #[test]
    fn obstacle_size_matches_the_original() {
        let (mut n, mut dflt) = (0, 0);
        for line in include_str!("testdata/traffic_size_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let o: Vec<u32> = o.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let v = |k: usize| Vec3::new(f32::from_bits(w[k]), f32::from_bits(w[k + 1]), f32::from_bits(w[k + 2]));
            let owner = w[0] & 0x20 != 0 && w[1] != 0 && w[2] != 0;
            let b = if owner { (w[4] != 0).then(|| (v(11), v(14))) } else { (w[3] != 0).then(|| (v(5), v(8))) };
            dflt += b.is_none() as u32;
            let (x, z) = bound_extents(b);
            assert_eq!(vec![x.to_bits(), z.to_bits()], o, "{line}");
            n += 1;
        }
        assert_eq!((n, dflt), (2000, 572));
    }
}

#[cfg(test)]
mod measured_speed_tests {
    //! F-30960's no-rail exit run offline (tools/measured_speed_reference.py): +412 = |car +208| for 3,000 velocities.
    use super::*;

    #[test]
    fn measured_speed_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/measured_speed_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = i.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let r: Vec<u32> = o.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let v = Vec3::new(f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]));
            assert_eq!(measured_speed(v).to_bits(), r[0], "{line}");
            assert_eq!((r[1], r[2]), (0, 0), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }
}

#[cfg(test)]
mod slot_update_tests {
    use super::*;

    /// F-49294 run offline on its generated code with its needed acceleration F-20809 stubbed (tools/slot_update_reference.py,
    /// 4,000 cases: on / off, the entry's reset bit, no obstacle / a car / the player, a near each threshold, at 0 and -0,
    /// the gap 0 and negative, a twentieth with one word a zero, a denormal, an infinity, a NaN or FLT_MAX): the on bit,
    /// the slot's acceleration and its gap equal to the bit.
    #[test]
    fn slot_update_matches_the_original() {
        let mut n = 0;
        for l in include_str!("testdata/slot_update_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let (a, b) = l.split_once(" -> ").unwrap();
            let w: Vec<u32> = a.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let o: Vec<u32> = b.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let f = |k: usize| f32::from_bits(w[k]);
            let t = SlotTune { band_d: f(6), min_d: f(7), max_d: f(8), band_a: f(9), min_a: f(10), up: f(11) };
            let (on, acc, gap) = slot_update(w[0] != 0 && w[1] == 0, f(3), f(4) - f(5), w[2] == 9, &t);
            assert_eq!((0x80 | if on { 0x40 } else { 0 }, acc.to_bits(), gap.to_bits()), (o[0], o[1], o[2]), "{l}");
            n += 1;
        }
        assert_eq!(n, 4000);
    }
}

#[cfg(test)]
mod controller_pick_tests {
    use super::*;

    /// F-30960 run offline from its slot updates on (tools/controller_pick_reference.py, 4,000 cases: four entries each
    /// active or not, with or without the reset bit, the player as entry 0's obstacle, a at 0 / -0 / MaxAccel + bonus /
    /// -MaxDecel / random / NaN / infinities, the slots' on bits, standing and moving, the start delay, the player across or
    /// along our rail): the acceleration, the entry chosen (+420), +440, every slot's on bit and, for the active ones, its
    /// +0 / +4 equal [`pick`]'s to the bit.
    #[test]
    fn controller_pick_matches_the_original() {
        let mut n = 0;
        for l in include_str!("testdata/controller_pick_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let (a, b) = l.split_once(" -> ").unwrap();
            let w: Vec<u32> = a.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let o: Vec<&str> = b.split_whitespace().collect();
            let h = |k: usize| u32::from_str_radix(o[k], 16).unwrap();
            let f = |k: usize| f32::from_bits(w[k]);
            let mut entries = [None; 4];
            let mut on = [false; 4];
            for e in 0..4 {
                let x = 7 * e;
                if w[x] != 0 {
                    entries[e] = Some((f(x + 3), f(x + 4) - f(x + 5), w[x + 2] != 0));
                }
                // (the entry's reset bit clears the slot's on bit before the update, F-49294's first test)
                on[e] = w[28 + e] != 0 && !(w[x] != 0 && w[x + 1] != 0);
            }
            let t0 = 32;
            let across = w[t0 + 12] == 1;
            let max_d = if across { f(t0 + 5) * 3.0 } else { f(t0 + 5) };
            let t = SlotTune { band_d: f(t0 + 3), min_d: f(t0 + 4), max_d, band_a: f(t0), min_a: f(t0 + 1), up: f(t0 + 6) + f(t0 + 2) };
            let mut waited = f(t0 + 9);
            let (acc, chosen) = pick(&entries, &mut on, &t, f(t0 + 8), f(t0 + 7), f(t0 + 11), &mut waited, f(t0 + 10));
            assert_eq!(acc.to_bits(), h(0), "a {l}");
            assert_eq!(chosen.map_or(-1, |k| k as i32), o[1].parse::<i32>().unwrap(), "chosen {l}");
            assert_eq!(waited.to_bits(), h(2), "waited {l}");
            for e in 0..4 {
                let flags = h(3 + e);
                assert_eq!(flags & 0x40 != 0, on[e], "on {e} {l}");
                assert_eq!(flags & 0x80 != 0, entries[e].is_some(), "active {e} {l}");
                if let Some((a, gap, player)) = entries[e] {
                    // (the slot's +0 / +4 as F-49294 leaves them, from the bit it had before)
                    let before = w[28 + e] != 0 && w[7 * e + 1] == 0;
                    let (_, sa, sg) = slot_update(before, a, gap, player, &t);
                    assert_eq!((sa.to_bits(), sg.to_bits()), (h(7 + 2 * e), h(8 + 2 * e)), "slot {e} {l}");
                }
            }
            n += 1;
        }
        assert_eq!(n, 4000);
    }
}
