//! Ambient traffic as the original drives it (`mcAmbientCar`, vtable 0x820c2b0c, with its
//! `mcAmbientBehaviorDrive2` 0x820d9dfc; per frame sub_826EA968: the behaviour sub_827A0138,
//! the acceleration sub_826EA420, the speed sub_826EA240). A car rides its lane kinematically
//! (the rail follower advances by speed x dt, sub_82798F18) with no pedals or steering. Met by
//! something it turns physical (state 4) and rolls to rest (state 5); in free roam it then
//! finds its way back onto a lane on a recovery rail (state 6), goes round a car standing in
//! its way (state 7) and backs off one it is stuck behind (state 8). A semi cab tows a trailer
//! that trails its hitch. Rules decoded 2026-10-19 / 20: docs/PEOPLE_TRAFFIC.md "Ambient
//! traffic driving".

use super::Vehicles;
use crate::physics::collision::WorldCollider;
use glam::{Quat, Vec2, Vec3};
use mcla_formats::rage::navsys::{LaneCurve, NavSys, RegionRect, DEFAULT_ROAD_SPEED};

const MPH: f32 = 0.44704;
/// The slowest a car is ever asked to drive (sub_827A2B38: 10 mph).
const MIN_WANT: f32 = 4.4704;
/// Braking below this stops the car (sub_826EA240: 0.5 mph).
const STOP_SPEED: f32 = 0.22352;
/// Occluders are taken within 400 m of the focus, as the pedestrian spawner's (peds.rs
/// `OCC_RANGE_SCALE`).
pub(super) const OCC_RANGE: f32 = crate::occluder::RANGE * 10.0;
/// The recovery rail's largest turn (2.44346 rad) and the passing rail's (130 degrees).
const RAIL_TURN: f32 = 2.44346;
const PASS_TURN: f32 = 2.26893;
/// The semi trailers (0x8282BDDC); a cab draws one of them.
pub const TRAILERS: [&str; 5] = ["va_semitruck_00_trailer", "va_semitruck_00_tanker", "va_semitruck_00_flatbed", "va_semitruck_00_flatloaded", "va_semitruck_dub1_00_trailer"];
/// The "hitch" bones (sub_82790E68): the cab's and every trailer's; the model's front is -z.
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
    /// `bIsTaxi` (type +113: never parked, sub_826F23A0)
    pub taxi: bool,
    /// `bIsProxy` (type +108): the start speed rule never removes such a car (sub_826E7FF8)
    pub proxy: bool,
    /// the bound's full width and length (the `.r155` root +8 bound, max +0x10 - min +0x20:
    /// obstacle vfuncs 4 and 6, sub_826D9818); 0: not read
    pub width: f32,
    pub length: f32,
    /// `mcAmbientCarTypeColor.xml` (render type +236): the 20 colours times `fScale`, and
    /// `iPaintType` (the CarPaint record: 0 DirtyMatte, 1 Matte, 2 Gloss, 3 Metallic, 4 Pearl,
    /// 5 Sparkle)
    pub colors: Option<[[f32; 3]; 20]>,
    pub paint_type: u8,
}

impl Default for AmbType {
    fn default() -> Self {
        // (the values every sedan of the set carries)
        AmbType { max_speed: 40.0, min_accel_band: 1.0, min_accel: 1.0, max_accel: 5.0, min_decel_band: 0.2, min_decel: 3.0, max_decel: 22.5, min_turn: 5.0, pivot_turn: 3.0, instances: 20, bound_group: 0, freeway_only: false, cab: false, trailer: false, taxi: false, proxy: false, width: 0.0, length: 0.0, colors: None, paint_type: 2 }
    }
}

impl AmbType {
    pub fn load(root: &std::path::Path, model: &str) -> AmbType {
        let _ = DATA_ROOT.set(root.to_path_buf());
        let d = AmbType::default();
        let (width, length) = bound_size(root, model).unwrap_or((0.0, 0.0));
        let (colors, paint_type) = type_colors(root, model);
        let Ok(t) = std::fs::read_to_string(root.join("tune/ambients/traffic").join(model).join("mcAmbientType.xml")) else { return AmbType { width, length, colors, paint_type, ..d } };
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
            colors,
            paint_type,
        }
    }

    /// The colour of colour index `i` (the spawner draws `rand % 20`, sub_826F6920; the
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

/// A traffic type's two LOD steps, `tune/ambients/traffic/<model>/mcAmbientRenderType.xml` (the render type's
/// parser table 0x82830414, filled by sub_82790530: fLodSwitchScale at +40, fHighToMedSwitch at +44,
/// fMedToLowSwitch at +48): LOD 0 below +44 x +40, LOD 1 below +48 x +40 (sub_827902F0). Every type has the file;
/// the steps differ by type (40 / 155 for most cars, 40 / 250 for buses and service trucks, 180 / 250 for the
/// flatbed semis, 50 / 155 and 40 / 200 for one type each).
pub fn lod_steps(root: &std::path::Path, model: &str) -> Option<[f32; 2]> {
    let t = std::fs::read_to_string(root.join("tune/ambients/traffic").join(model).join("mcAmbientRenderType.xml")).ok()?;
    let v = |tag: &str| crate::ambient::xml_value(&t, tag);
    let scale = v("fLodSwitchScale")?;
    Some([v("fHighToMedSwitch")? * scale, v("fMedToLowSwitch")? * scale])
}

/// The centre of a traffic model's bound (the root bound's centroid, +0x30), in the car's space: the middle of the
/// sphere the LOD choice tests (sub_826E9F88: the car's matrix times bound +48).
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

/// The drawn wheels turn only within 75 m of the focus, horizontally (sub_82794FA8: d^2 < 5625).
pub const SPIN_REACH2: f32 = 5625.0;

/// A drawn wheel's angle kept in [0, 2 pi] (sub_82794FA8: one 2 pi off above, one on below 0).
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

/// A parked car's drawn wheel turn (sub_82794FA8): 0.2 of a riding car's - it has no behaviour
/// (car +452 = 0) and is no trailer.
pub const PARKED_SPIN: f32 = 0.2;

/// A parked car's front wheels' turn (sub_826E86C8 with a spot: car +3584 = a Gaussian, mean 0, sd
/// pi / 8 (sub_821CE328), clamped to +-0.706858; drawn added to the front wheels' steer while the
/// car has no behaviour and is no trailer, sub_82794FA8). `u1` in (0, 1], `u2` in [0, 1).
pub fn parked_steer(u1: f32, u2: f32) -> f32 {
    let g = (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
    (g * std::f32::consts::FRAC_PI_8).clamp(-0.706858, 0.706858)
}

/// A traffic car's instance matrix (inst +16): its rotation, and the seat's place - the pose before
/// the drawn 0.025 m drop - while it rides; rows X, up, Z, place.
pub fn instance_matrix(c: &super::CarInstance) -> [Vec3; 4] {
    let m = glam::Mat3::from_quat(c.state.rot);
    let at = c.rail.as_ref().filter(|r| !r.active).and_then(|r| r.seat.as_ref()).map_or(c.state.pos, |s| s.at.0);
    [m.x_axis, m.y_axis, m.z_axis, at]
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
        // (sub_826E5B30)
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

    /// The point and direction `s` metres along a lane, and the link and metres along it.
    fn at(&self, nav: &NavSys, lane: usize, mut s: f32) -> (Vec3, Vec3, usize, f32) {
        let ls = &self.links[lane];
        for (i, &k) in ls.iter().enumerate() {
            let l = &nav.links[k];
            if s <= l.len || i + 1 == ls.len() {
                let s = s.clamp(0.0, l.len);
                let t = s / l.len.max(1e-3);
                return (l.a + (l.b - l.a) * t, l.tangent_at(t), k, s);
            }
            s -= l.len;
        }
        (Vec3::ZERO, Vec3::NEG_Z, 0, 0.0)
    }

    /// Metres along the lane of the point nearest to `p`.
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
/// heading off the curve at its phase, sub_827986C0 -> lane vfunc 9 sub_82245E00 ->
/// sub_82586A28), cut wherever the tangent has turned 2 degrees, with the curve's tangents.
/// Traffic keeps to this graph (its own link indices; lanes and successors as the shared one).
pub fn rail_nav(nav: &NavSys) -> &'static NavSys {
    static N: std::sync::OnceLock<NavSys> = std::sync::OnceLock::new();
    N.get_or_init(|| {
        let t = std::time::Instant::now();
        let cv = nav.curved(2f32.to_radians());
        log::info!("traffic: lanes as curves, {} links (control polygon {}), {:.2} s", cv.links.len(), nav.links.len(), t.elapsed().as_secs_f32());
        cv
    })
}

/// The lanes of the city, made once.
pub fn lanes(nav: &NavSys) -> &'static Lanes {
    static L: std::sync::OnceLock<Lanes> = std::sync::OnceLock::new();
    L.get_or_init(|| Lanes::build(nav))
}

/// A rail off the lanes (recovery, passing, backing up): points every half metre, the
/// distance to each, the rail's speed and the lane it ends on.
#[derive(Clone, Debug)]
pub struct Path {
    pts: Vec<Vec3>,
    cum: Vec<f32>,
    pub speed: f32,
    /// the link and metres along it where the car goes on along its lane
    end: Option<(usize, f32)>,
    /// ridden backwards (the car faces against the rail)
    reverse: bool,
    pub s: f32,
}

impl Path {
    fn new(pts: Vec<Vec3>, speed: f32, end: Option<(usize, f32)>) -> Path {
        let mut cum = Vec::with_capacity(pts.len());
        let mut d = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                d += p.distance(pts[i - 1]);
            }
            cum.push(d);
        }
        Path { pts, cum, speed, end, reverse: false, s: 0.0 }
    }
    pub fn len(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }
    /// The point and direction of travel `s` metres along.
    fn at(&self, s: f32) -> (Vec3, Vec3) {
        let n = self.pts.len();
        if n < 2 {
            return (self.pts.first().copied().unwrap_or(Vec3::ZERO), Vec3::NEG_Z);
        }
        let i = self.cum.partition_point(|&c| c <= s).clamp(1, n - 1);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        let seg = (self.cum[i] - self.cum[i - 1]).max(1e-4);
        let t = ((s - self.cum[i - 1]) / seg).clamp(0.0, 1.0);
        (a.lerp(b, t), (b - a).normalize_or(Vec3::NEG_Z))
    }
}

fn wrap_2pi(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let r = a.rem_euclid(t);
    if r >= t - 1e-5 { 0.0 } else { r }
}

/// Points along an arc, a line and an arc (sub_8279E518; `s` +1 turns towards the heading's
/// left normal (-sin, cos) in x/z, -1 the other way), every half metre; y runs from the
/// start's to the end's.
#[allow(clippy::too_many_arguments)]
fn sample_csc(p0: Vec3, p1: Vec3, c0: Vec2, s0: f32, phi0: f32, th0: f32, c1: Vec2, s1: f32, psi: f32, th1: f32, rr: f32) -> Vec<Vec3> {
    let on = |c: Vec2, s: f32, phi: f32| c + Vec2::new(phi.sin(), -phi.cos()) * (s * rr);
    let mut flat: Vec<Vec2> = Vec::new();
    let n0 = ((th0 * rr) / 0.5).ceil().max(1.0) as usize;
    for k in 0..=n0 {
        flat.push(on(c0, s0, phi0 + s0 * th0 * k as f32 / n0 as f32));
    }
    let (t0, t1) = (on(c0, s0, psi), on(c1, s1, psi));
    let nl = (t0.distance(t1) / 0.5).ceil() as usize;
    for k in 1..nl {
        flat.push(t0.lerp(t1, k as f32 / nl as f32));
    }
    let n1 = ((th1 * rr) / 0.5).ceil().max(1.0) as usize;
    for k in 0..=n1 {
        flat.push(on(c1, s1, psi + s1 * th1 * k as f32 / n1 as f32));
    }
    let mut total = 0.0;
    let mut cum = vec![0.0];
    for i in 1..flat.len() {
        total += flat[i].distance(flat[i - 1]);
        cum.push(total);
    }
    flat.iter().zip(cum).map(|(q, c)| Vec3::new(q.x, p0.y + (p1.y - p0.y) * if total > 0.0 { c / total } else { 1.0 }, q.y)).collect()
}

/// The four arc-line-arc ways from `p0` heading `h0` to `p1` heading `h1` at turning radius
/// `rr` that turn no more than `max_turn` in all: (length, points).
fn csc(p0: Vec3, h0: Vec3, p1: Vec3, h1: Vec3, rr: f32, max_turn: f32) -> Vec<(f32, Vec<Vec3>)> {
    let (a, b) = (Vec2::new(p0.x, p0.z), Vec2::new(p1.x, p1.z));
    let (phi0, phi1) = (h0.z.atan2(h0.x), h1.z.atan2(h1.x));
    let nrm = |phi: f32| Vec2::new(-phi.sin(), phi.cos());
    let mut out = Vec::new();
    for s0 in [1.0f32, -1.0] {
        for s1 in [1.0f32, -1.0] {
            let c0 = a + nrm(phi0) * (s0 * rr);
            let c1 = b + nrm(phi1) * (s1 * rr);
            // (sub_8279D3A8: no way when the target lies inside the start circle)
            if (c0 - b).length_squared() < rr * rr {
                continue;
            }
            let w = c1 - c0;
            let dd = w.length();
            let alpha = w.y.atan2(w.x);
            let (psi, line) = if s0 == s1 {
                (if dd < 1e-4 { phi0 } else { alpha }, dd)
            } else {
                if dd < 2.0 * rr {
                    continue;
                }
                let l = (dd * dd - 4.0 * rr * rr).sqrt();
                (alpha + (2.0 * s0 * rr).atan2(l), l)
            };
            let th0 = wrap_2pi(s0 * (psi - phi0));
            let th1 = wrap_2pi(s1 * (phi1 - psi));
            if th0 + th1 > max_turn {
                continue;
            }
            // (its length, sub_8279CB68: the arcs and the line between the tangent points, which
            // take the start's and the end's heights)
            out.push((rr * (th0 + th1) + (line * line + (p1.y - p0.y) * (p1.y - p0.y)).sqrt(), sample_csc(p0, p1, c0, s0, phi0, th0, c1, s1, psi, th1, rr)));
        }
    }
    out
}

/// An arc then a line to `p1` (the passing rail, sub_827A14C0): both turning senses.
fn cs(p0: Vec3, h0: Vec3, p1: Vec3, rr: f32, max_turn: f32) -> Vec<(f32, Vec<Vec3>)> {
    let (a, b) = (Vec2::new(p0.x, p0.z), Vec2::new(p1.x, p1.z));
    let phi0 = h0.z.atan2(h0.x);
    let mut out = Vec::new();
    for s in [1.0f32, -1.0] {
        let c = a + Vec2::new(-phi0.sin(), phi0.cos()) * (s * rr);
        let w = b - c;
        let dd = w.length();
        if dd < rr {
            continue;
        }
        let l = (dd * dd - rr * rr).sqrt();
        let psi = w.y.atan2(w.x) + (s * rr).atan2(l);
        let th = wrap_2pi(s * (psi - phi0));
        if th > max_turn {
            continue;
        }
        // (an arc to the line's heading and none at the end)
        out.push((rr * th + l, sample_csc(p0, p1, c, s, phi0, th, b - Vec2::new(psi.sin(), -psi.cos()) * (s * rr), s, psi, 0.0, rr)));
    }
    out
}

/// The turning radius of the recovery and passing rails (sub_8279E4C8): the pivot radius when
/// the car has stood 2 s, else the smallest radius growing with speed above 10 mph.
fn turn_radius(ty: &AmbType, v: f32, ready: bool) -> f32 {
    if ready { ty.pivot_turn } else { ty.min_turn * (1.0 + 6.0 * ((v - 4.4704) * 0.149129).clamp(0.0, 1.0)) }
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
    /// the speed offset (car +424: -6, 0 or +6 m/s) and seconds to its next draw
    pub off: f32,
    pub off_timer: f32,
    /// acceleration bonus (car +428: U - 0.5, drawn once)
    pub bonus: f32,
    /// the standing gap behind the car ahead (behaviour +460: 2 .. 5 m)
    pub gap: f32,
    /// start delay when moving off (car +444: U x 0.8 s) and the time asked to go (+440)
    pub wait: f32,
    waited: f32,
    /// the constraint entries switched on (hysteresis sub_826E91F8): car ahead, stop line, slower lane
    pub on: [bool; 3],
    /// the stop and the car ahead the entries were made for (a new one resets the hysteresis)
    stop_id: Option<usize>,
    obst_id: Option<usize>,
    /// the behaviour's state (sub_827A0138): 0 a trailer (no behaviour), 1 on its lane, 4
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
    /// the car ahead kept while both stand: (index, its position)
    kept: Option<(usize, Vec3)>,
    /// seconds at rest while physical (the physics' sleep)
    rest: f32,
    /// a cab's trailer or a trailer's cab (their ids, +3604)
    pub partner: Option<u32>,
    /// on the ground (the car update's probe and smoothing, car +1696 .. +2528); None: made
    /// anew from the car's pose
    pub seat: Option<Seat>,
    /// the drawn body's pitch and roll (sub_82794FA8)
    pub tilt: Tilt,
    /// the physics level holds the car (loose.rs `Amb`: inactive while it rides, active - state 4 -
    /// until its rest rule); `active`: its body is active; `wake`: the game made it physical
    /// (vehicles.rs hands that to the level: 0x8258D3F8)
    pub in_level: bool,
    pub active: bool,
    pub wake: bool,
    rng: u32,
    id: u32,
    /// the follower's curve speed |dP/dt| (+180) and the lane it was taken on (another
    /// lane: taken anew, as sub_82798B98 does when it sets a rail)
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
    /// the drawn wheels' lock (sub_826E5950, `Lock`)
    pub lock: Lock,
    /// when it was made and how long it lives at least on a listed road (car +464 / +468:
    /// 8 + 4U s)
    pub born: f32,
    pub life: f32,
    /// a race path's car on the path's rails (state 9; racescript.rs, `step_route`)
    pub route: Option<Box<Route>>,
    /// a race path's bPhysical (path +170): the instance's +140 bit 0x80 (sub_826EAD00 ->
    /// sub_826FA588), for the car's life
    pub phys_pair: bool,
    /// its place in its rail's car list (`RAIL_SEQ` when it took the rail: at its spawn, or when
    /// the rail it holds changed, `Vehicles::stamp_rails`), the rail it was stamped on, and
    /// whether it is still on the rail it was made on
    rail_seq: u64,
    rail_key: Option<RailKey>,
    rail_fresh: bool,
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
            rest: 0.0,
            partner: None,
            seat: None,
            tilt: Tilt::default(),
            in_level: false,
            active: false,
            wake: false,
            rng: seed.max(1),
            id: seed,
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
            phys_pair: false,
            rail_seq: next_rail_seq(),
            rail_key: None,
            rail_fresh: true,
        };
        r.place(nav, link, s);
        r.life = 8.0 + 4.0 * r.rand();
        r.bonus = r.rand() - 0.5;
        r.gap = 2.0 + 3.0 * r.rand();
        r.wait = r.rand() * 0.8;
        // (the speed offset, car +424, is 0 from the spawn - sub_826EAD00 - until its first draw:
        // +432 = 3 + 3U s of age, sub_826EA240)
        r.off = 0.0;
        r.off_timer = 3.0 + 3.0 * r.rand();
        r.choose_next(nav);
        // (the lane's speed; the spawner and the race paths set the first speed by the start
        // speed rule, `Vehicles::path_start_speed`)
        r.v = rail_speed(nav, link).0;
        r
    }

    /// A bPhysical car (inst +140 bit 0x80) whose pose was written this frame: the follower has
    /// a rail (sub_827986C0 sets the pose flag only then; car slot 20 sub_826E5D20 ->
    /// sub_826FA760 puts the inactive instance on the simulator's list, loose.rs `ambient_posed`).
    pub fn pairs_inactive(&self) -> bool {
        self.phys_pair && (matches!(self.state, 1 | 9) || (matches!(self.state, 6..=8) && self.path.is_some()))
    }

    /// a stable id for traces and for the cab / trailer link
    pub fn rng_id(&self) -> u32 {
        self.id
    }

    pub fn rand(&mut self) -> f32 {
        // (the car's own MWC is not reproduced: any uniform draw)
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / 16777216.0
    }

    /// sub_826EA240: every 3 + 3U s a new offset, -6, +6 or 0 m/s by thirds.
    fn draw_offset(&mut self) {
        let u = self.rand();
        self.off = if u < 1.0 / 3.0 { -6.0 } else if u < 2.0 / 3.0 { 6.0 } else { 0.0 };
        self.off_timer = 3.0 + 3.0 * self.rand();
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

    /// Which lane follows this one, chosen on entering a lane (slot 16 sub_8279FBB0 ->
    /// sub_8273BD80): offline the route is empty, so any lane starting where this one ends
    /// but itself, uniformly at random - no weighting, no look-ahead.
    fn choose_next(&mut self, nav: &NavSys) {
        let end = self.lane_end(nav);
        let lane = nav.links[end].lane;
        let mut succ: Vec<usize> = nav.links[end].next.iter().copied().filter(|&n| nav.links[n].lane != lane).collect();
        if succ.is_empty() {
            // (a lane end the port's graph left unconnected - the original's rails all join:
            // the lane starts within reach of it)
            succ = nav.links[end].hop.clone();
        }
        self.next = if succ.is_empty() { None } else { Some(succ[(self.rand() * succ.len() as f32) as usize % succ.len()]) };
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

    /// The car's point: the lane's curve at its phase (sub_827986C0 -> sub_82586A28).
    pub fn pos(&self, nav: &NavSys) -> Vec3 {
        let l = &nav.links[self.link];
        match nav.curves.get(l.lane) {
            Some(c) if c.pts.len() >= 2 => c.eval(self.phase(nav)).0,
            _ => l.a + (l.b - l.a) * (self.s / l.len.max(1e-3)),
        }
    }

    /// The heading: the curve's derivative at the phase, normalised (sub_827986C0).
    pub fn tangent(&self, nav: &NavSys) -> Vec3 {
        let l = &nav.links[self.link];
        let d = nav.curves.get(l.lane).map_or(Vec3::ZERO, |c| c.eval(self.phase(nav)).1);
        d.normalize_or(l.tangent_at(self.s / l.len.max(1e-3)))
    }

    /// The pose the follower stores (sub_827986C0): point, heading, and the curve speed
    /// +180 = the mean of |dP/dt| here and the last value (taken anew on another lane).
    fn pose(&mut self, nav: &NavSys) -> (Vec3, Vec3) {
        let l = &nav.links[self.link];
        let Some(c) = nav.curves.get(l.lane).filter(|c| c.pts.len() >= 2) else { return (self.pos(nav), self.tangent(nav)) };
        let (p, d) = c.eval(self.phase(nav));
        let sp = d.length();
        self.ds = if self.ds_lane == l.lane && self.ds > 0.0 { (sp + self.ds) * 0.5 } else { sp };
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

    /// The physics level put the car to rest (sub_826E68C0 -> behaviour vfunc 13 sub_8279F790: state 5).
    pub(super) fn came_to_rest(&mut self) {
        self.active = false;
        if self.state == 4 {
            self.state = 5;
            self.v = 0.0;
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

/// The speed of a lane (sub_82245A28 on the lane's owner, flags at owner +4) and whether it is
/// a junction lane (owner flags & 0x30, or lane flags & 0x1000).
/// A lane owned by a junction (sub_822452F8: the lane's owner, or the owner's parent, has the
/// junction flag 4).
fn junction_lane(l: &mcla_formats::rage::navsys::Link) -> bool {
    (l.owner_flags | l.parent_flags) & 4 != 0
}

/// The junction a lane belongs to (sub_822452F8: the lane's owner +08 when the owner's flags +04
/// have bit 4, else the owner's parent when its flags have bit 4; none else), as an index into
/// the navsys junction array (root +58; `NavSys::lane_junction`).
fn lane_junction(nav: &NavSys, lane: usize) -> Option<usize> {
    nav.lane_junction.get(lane).copied().flatten()
}

/// The port's signals' junction built from navsys junction `j` (the one with its centre +80).
/// None for the 12 junctions without arms (`parse_junctions` leaves them out): there
/// sub_8273A178 finds no arm at J +160 and reads the zero vector at 0x828608E0, so no lane of
/// theirs is ever green.
fn signals_junction(signals: &crate::signals::Signals, nav: &NavSys, j: usize) -> Option<usize> {
    let c = *nav.junction_centers.get(j)?;
    let bits = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
    signals.junction_at(c).filter(|&ji| bits(signals.junctions[ji].center) == bits(c))
}

/// sub_8273A178(J, rail): may a car enter `link`'s lane at junction `j` - the lane's first span
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
/// obstacle's entry is appended at its tail by sub_8273E160 when the follower sets the rail,
/// sub_82798B98 / sub_82798F18, and unlinked when it leaves, sub_8273E1A8).
static RAIL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_rail_seq() -> u64 {
    RAIL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A driver's car's lanes (its obstacle, mcRacer's +56: entries +20 + 24 k, at most 12, count
/// +308) as sub_826D99B8 keeps them, and the lane it reports (racer +640 lane, +644 squared
/// distance; sub_822A9D98).
#[derive(Clone, Debug, Default)]
pub struct DriverLanes {
    list: Vec<(usize, f32)>,
    pub best: Option<(usize, f32)>,
}

/// A lane's first two control points (sub_8273A178 reads P0 - P1); the link's ends without them.
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

/// What holds a junction's box for sub_8273A848 (`Vehicles::junction_blockers`).
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

/// The junction ahead of the player's lane (sub_826F4B98 + sub_826F0E30), as a navsys junction
/// index. None when the lane is a junction lane (owner or parent flag 4) or its owner has
/// flags & 0x30 or & 0x40. Else up to 8 rounds: at the lane's end (its first point when
/// `reverse`) take the lanes of that vertex in the vertex table's order, not the lane itself,
/// starting there (`reverse`: ending there); the first such junction lane gives its junction,
/// and the first one whose owner has no flag 0x30 / 0x40 is the next round's lane. (The
/// original goes on to a second junction for a record nothing reads.) The node order of the
/// lanes at a vertex is the port's (`lane_ends`, curve order), not checked against the file.
pub fn player_junction_ahead(nav: &NavSys, lane: usize, reverse: bool) -> Option<usize> {
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
                    return Some(j);
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

/// A junction's box as sub_8273A848 tests it (mcRnIntersectionAmbient): the centre +0x50 / +0x58,
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

/// The light a rider obeys (sub_827A29F0 -> sub_827A3570 / sub_827A3458 / sub_8273A178): the
/// junction lane it is to enter - its next lane when that is a junction lane, else the first
/// junction lane starting where the next lane ends (sub_822464F0; no further) - and its
/// junction's state for that lane (`Junction::lane_state`, the lane's first span against the
/// current arm). Returns (junction, xz metres from `pos` to the junction lane's first point =
/// the stop line, state 2 green / 1 yellow / 0 red). (Which junction lane sub_822464F0 takes
/// when several start at that point follows the node's lane order; here the graph's successor
/// order. The junction is the lane's, sub_822452F8; one without arms gives none here.)
fn signal_stop(env: &Env, r: &Rider, pos: Vec3) -> Option<(usize, f32, u8, usize)> {
    let nav = env.nav;
    let n = r.next?;
    let first = if junction_lane(&nav.links[n]) {
        n
    } else {
        let lane = nav.links[n].lane;
        let mut k = n;
        while let Some(&m) = nav.links[k].next.iter().find(|&&m| nav.links[m].lane == lane && m != k) {
            k = m;
        }
        *nav.links[k].next.iter().find(|&&m| nav.links[m].lane != lane && junction_lane(&nav.links[m]))?
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

/// The acceleration that brings speed `v` to `vt` at `dist - margin` metres (sub_82796658):
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
    pub vel: Vec3,
    /// half its bound's length and width
    pub half_len: f32,
    pub half_wid: f32,
    /// on a lane, kinematic: (its lane, its next lane, junction lane)
    pub on: Option<(usize, Option<usize>, bool)>,
    /// on a rail off the lanes (states 6 - 8: run-time rails, flags 0x1800 - no lane in the
    /// file carries those bits)
    pub railed: bool,
    pub player: bool,
    /// ambient traffic (parked cars too); else the player, police, racers
    pub ambient: bool,
    /// physically active (awake)
    pub active: bool,
    pub cab_or_trailer: bool,
    /// (diagnostics) its rider's id
    pub key: u32,
}

impl Other {
    fn corners(&self) -> [Vec3; 4] {
        let (x, z) = (self.right * self.half_wid, self.fwd * self.half_len);
        [self.pos + x + z, self.pos + x - z, self.pos - x - z, self.pos - x + z]
    }
    /// Distance in the ground plane from `q` to its box (0 inside).
    fn box_dist(&self, q: Vec3) -> f32 {
        let d = q - self.pos;
        let r = Vec3::new(self.right.x, 0.0, self.right.z).normalize_or(Vec3::X);
        let f = Vec3::new(self.fwd.x, 0.0, self.fwd.z).normalize_or(Vec3::NEG_Z);
        let (lx, lz) = ((d.dot(r).abs() - self.half_wid).max(0.0), (d.dot(f).abs() - self.half_len).max(0.0));
        (lx * lx + lz * lz).sqrt()
    }
}

/// Is the obstacle in the way of a rail point (sub_827A59A8; no lane width used): its corners'
/// offsets along the line from it to the rail point straddle that point, or come within half
/// our width of it.
fn in_the_way(o: &Other, q: Vec3, own_wid: f32) -> bool {
    let u = Vec2::new(q.x - o.pos.x, q.z - o.pos.z);
    if u.length_squared() < 1e-8 {
        return true;
    }
    let u = u.normalize();
    let (mut mn, mut mx) = (f32::MAX, f32::MIN);
    for c in o.corners() {
        let s = u.dot(Vec2::new(c.x - q.x, c.z - q.z));
        mn = mn.min(s);
        mx = mx.max(s);
    }
    (mn < 0.0 && mx > 0.0) || mn.abs() < 0.5 * own_wid || mx.abs() < 0.5 * own_wid
}

/// Is it a "blocking" obstacle (sub_827A5E28): one that stands for good.
fn blocking(o: &Other, fwd: Vec3) -> bool {
    if o.vel.length_squared() > 4.99612 {
        return false;
    }
    let speed = o.vel.length();
    if !o.ambient {
        // (the player and other drivers: turned more than 70 degrees off our heading; the
        // vehicle-state test +640 / +872 / +876 is not decoded)
        return o.fwd.dot(fwd) < 0.34202;
    }
    // (a car on its lane: never; on a rail off the lanes: standing and facing us)
    match o.on {
        Some(_) => false,
        None if o.railed => speed == 0.0 && !o.active && o.fwd.dot(fwd) < 0.0,
        None if o.active => speed < MPH && !o.cab_or_trailer,
        None => speed == 0.0,
    }
}

/// The nearest point of a rail (its segments) to `p`, unless it lies at the rail's start or
/// end: (point, metres along).
fn on_rail(segs: &[(Vec3, Vec3)], p: Vec3, open_start: bool) -> Option<(Vec3, f32)> {
    let (mut best, mut at, mut s0) = (f32::MAX, None, 0.0);
    let n = segs.len();
    for (i, &(a, b)) in segs.iter().enumerate() {
        let d = b - a;
        let l2 = d.length_squared().max(1e-6);
        let t = ((p - a).dot(d) / l2).clamp(0.0, 1.0);
        let q = a + d * t;
        let dd = q.distance_squared(p);
        if dd < best {
            best = dd;
            let inside = !(i + 1 == n && t >= 1.0) && (open_start || !(i == 0 && t <= 0.0));
            at = inside.then_some((q, s0 + t * l2.sqrt()));
        }
        s0 += l2.sqrt();
    }
    at
}

/// The car ahead (sub_827A6448): the nearest obstacle and the nearest "blocking" one.
struct Ahead {
    j: usize,
    d: f32,
    vt: f32,
    blocking: Option<usize>,
    player_across: bool,
}

/// Looks for the car ahead along `rails` (this rail and the next as segments), `lanes` the
/// lanes those rails are (cars riding them count without a test).
#[allow(clippy::too_many_arguments)]
fn find_ahead(nav: &NavSys, me: usize, pos: Vec3, fwd: Vec3, own_wid: f32, rails: &[Vec<(Vec3, Vec3)>], lanes: (Option<usize>, Option<usize>), next_start: Option<Vec3>, others: &[Other]) -> Ahead {
    let mut best: Option<(usize, f32)> = None;
    let mut block: Option<(usize, f32)> = None;
    let mut player_across = false;
    for (j, o) in others.iter().enumerate() {
        if j == me {
            continue;
        }
        let rel = o.pos - pos;
        if rel.dot(fwd) < 0.0 || rel.y.abs() > 4.0 || rel.length_squared() > 150.0 * 150.0 {
            continue;
        }
        let d = rel.length();
        if d <= 0.0 {
            continue;
        }
        let direct = match o.on {
            Some((ol, onext, _)) => {
                // (a car merging into our next rail: while it moves - its speed +412 - and we
                // are within 75 m of that rail's start)
                Some(ol) == lanes.0 || (lanes.1.is_some() && Some(ol) == lanes.1) || (o.vel != Vec3::ZERO && onext.is_some() && onext == lanes.1 && next_start.is_some_and(|s| s.distance_squared(pos) < 5625.0))
            }
            _ => false,
        };
        let mine = direct || rails.iter().enumerate().any(|(k, segs)| on_rail(segs, o.pos, k > 0).is_some_and(|(q, _)| in_the_way(o, q, own_wid)));
        if o.player && !mine {
            // (the player's car across our lane or the next: MaxDecel x 3; the original's test
            // is on the lanes the player is registered on - here within 3 m of our way)
            let side = (rel - fwd * rel.dot(fwd)).length();
            if side < 3.0 && o.fwd.dot(fwd).abs() < 0.7071 {
                player_across = true;
            }
        }
        if !mine {
            continue;
        }
        if best.map_or(true, |b| d < b.1) {
            best = Some((j, d));
        }
        if blocking(o, fwd) && block.map_or(true, |b| d < b.1) {
            block = Some((j, d));
        }
    }
    let Some((j, d)) = best else { return Ahead { j: usize::MAX, d: 0.0, vt: 0.0, blocking: block.map(|b| b.0), player_across } };
    let o = &others[j];
    // its speed along our heading, but not for a car turning through a junction
    let vt = match o.on {
        Some((_, _, true)) => o.vel.length(),
        _ => o.vel.dot(fwd),
    };
    let _ = nav;
    Ahead { j, d, vt: vt.max(0.0), blocking: block.map(|b| b.0), player_across }
}

/// The segments of a lane.
fn lane_segs(nav: &NavSys, lanes: &Lanes, lane: usize) -> Vec<(Vec3, Vec3)> {
    lanes.links[lane].iter().map(|&k| (nav.links[k].a, nav.links[k].b)).collect()
}

/// The world a rider looks at this frame.
pub struct Env<'a> {
    pub nav: &'a NavSys,
    pub lanes: &'a Lanes,
    pub col: &'a WorldCollider,
    pub others: &'a [Other],
    pub signals: &'a crate::signals::Signals,
    /// the frame's blockers per navsys junction (sub_8273A848's hits, `Vehicles::frame_blockers`)
    pub blockers: &'a hashbrown::HashMap<usize, Blockers>,
    pub clock: f32,
    /// a race or another event is on: no recovery, passing or backing up (byte +0x64251)
    pub event: bool,
    pub hold: bool,
    /// the frame's parity (the footprint probe's side, manager +0x6423C)
    pub parity: bool,
}

/// Is a rail clear (sub_8279D9D0 with the own half width): the city's bound and the cars.
fn sweep_clear(env: &Env, me: usize, pts: &[Vec3], hw: f32) -> bool {
    for (k, p) in pts.iter().enumerate() {
        if k % 2 == 1 && k + 1 != pts.len() {
            continue;
        }
        // (the bound swept above the kerbs: the sphere's bottom 0.35 m up)
        if env.col.sphere(*p + Vec3::Y * (hw + 0.35), hw).is_some() {
            return false;
        }
        if env.others.iter().enumerate().any(|(j, o)| j != me && (o.pos.y - p.y).abs() < 3.0 && o.box_dist(*p) < hw) {
            return false;
        }
    }
    true
}

/// The footprint probe (sub_8279FC80, states 6 and 7; sub_827A3AB0 in state 8): a segment
/// 0.75 m up, 0.675 of the half width to one side (the side alternates by frame), from -0.9
/// to +0.9 of the half length. Does it hit anything?
fn probe_hit(env: &Env, me: usize, pos: Vec3, fwd: Vec3, hl: f32, hw: f32) -> bool {
    let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
    let side = if env.parity { 1.0 } else { -1.0 };
    let base = pos + Vec3::Y * 0.75 + right * (side * 0.675 * hw);
    let (a, b) = (base - fwd * (0.9 * hl), base + fwd * (0.9 * hl));
    let len = a.distance(b);
    if len > 1e-3 && env.col.raycast(a, (b - a) / len, len).is_some() {
        return true;
    }
    let n = (len / 0.25).ceil().max(1.0) as usize;
    env.others.iter().enumerate().any(|(j, o)| j != me && (o.pos.y - pos.y).abs() < 2.0 && o.pos.distance_squared(pos) < (hl + o.half_len + o.half_wid + 2.0).powi(2) && (0..=n).any(|k| o.box_dist(a.lerp(b, k as f32 / n as f32)) < 0.01))
}

/// The recovery rail (sub_827A06D0): ten tries at a random point of a random lane within 25 m
/// (at or after the point nearest to us), four arc-line-arc ways to each that turn no more
/// than `max_turn` and sweep clear; the shortest.
#[allow(clippy::too_many_arguments)]
fn recovery_rail(r: &mut Rider, env: &Env, me: usize, start: Vec3, dir: Vec3, rr: f32, max_turn: f32, speed: f32) -> Option<Path> {
    let (nav, lanes) = (env.nav, env.lanes);
    let mut cand: Vec<usize> = Vec::new();
    for k in nav.links_near(start, 25.0) {
        let l = &nav.links[k];
        if l.point_at(l.project(start)).distance(start) <= 25.0 && !cand.contains(&l.lane) && lanes.len.get(l.lane).is_some_and(|&x| x > 0.5) {
            cand.push(l.lane);
            if cand.len() == 48 {
                break;
            }
        }
    }
    if cand.is_empty() {
        return None;
    }
    let hw = r.ty.width.max(1.8) * 0.5;
    let mut best: Option<(f32, Vec<Vec3>, usize, f32)> = None;
    for _ in 0..10 {
        let lane = cand[(r.rand() * cand.len() as f32) as usize % cand.len()];
        let total = lanes.len[lane];
        let t0 = lanes.project(nav, lane, start) / total;
        let t = t0 + r.rand() * (1.0 - t0);
        let (p, tan, link, s) = lanes.at(nav, lane, t * total);
        if (p.y - start.y).abs() > 4.0 {
            continue;
        }
        if tan.dot(dir) > 0.0 && dir.dot(p - start) < 0.0 {
            continue;
        }
        for (len, pts) in csc(start, dir, p, tan, rr, max_turn) {
            if best.as_ref().is_some_and(|b| len >= b.0) {
                continue;
            }
            if sweep_clear(env, me, &pts, hw) {
                best = Some((len, pts, link, s));
            }
        }
    }
    best.map(|(_, pts, link, s)| Path::new(pts, speed, Some((link, s))))
}

/// The recovery try (sub_827A3E08), in state 5 and every frame in state 6: not online, not a
/// semi, not upside down; about once a second; a new rail replaces one being ridden only when
/// it is shorter by a metre where more than 10 m are left.
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
    if r.rand() > 1.0 / 30.0 {
        return false;
    }
    let (start, dir) = match r.path.as_ref() {
        Some(p) => {
            let (q, d) = p.at(p.s);
            (q, if p.reverse { -d } else { d })
        }
        None => (pos, fwd),
    };
    let rr = turn_radius(&r.ty, r.v, ready);
    let speed = if ready { 1.1176 } else { r.v.max(4.0) };
    let Some(p) = recovery_rail(r, env, me, start, Vec3::new(dir.x, 0.0, dir.z).normalize_or(fwd), rr, RAIL_TURN, speed) else { return false };
    if let Some(cur) = r.path.as_ref() {
        let left = cur.len() - cur.s;
        if left > 10.0 && p.len() >= left - 1.0 {
            return false;
        }
    }
    r.take_path(p, 6);
    r.then = None;
    true
}

/// The passing rail (sub_827A4048 -> sub_82799728 -> sub_827A14C0): to beside the obstacle's
/// leftmost or rightmost corner, pushed out by half our width and `c` (1.1, 1.5, 0.8 m), on an
/// arc and a line of at most 130 degrees, swept clear; then a rail back onto a lane at 0.7 of
/// the lane's speed (sub_827A10B8).
fn passing_rail(r: &mut Rider, env: &Env, me: usize, pos: Vec3, fwd: Vec3, obst: usize, lane_speed: f32, ready: bool) -> Option<(Path, Path)> {
    let o = &env.others[obst];
    let corners = o.corners();
    let d0 = Vec3::new(o.pos.x - pos.x, 0.0, o.pos.z - pos.z);
    let side = |c: Vec3| {
        let c = Vec3::new(c.x - pos.x, 0.0, c.z - pos.z).normalize_or_zero();
        d0.x * c.z - d0.z * c.x
    };
    let left = corners.iter().copied().min_by(|a, b| side(*a).total_cmp(&side(*b)))?;
    let right = corners.iter().copied().max_by(|a, b| side(*a).total_cmp(&side(*b)))?;
    let hw = r.ty.width.max(1.8) * 0.5;
    let rr = turn_radius(&r.ty, r.v, ready);
    let flat_fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
    for c in [1.1f32, 1.5, 0.8] {
        let mut best: Option<(f32, Vec<Vec3>)> = None;
        for (corner, sgn) in [(left, 1.0f32), (right, -1.0)] {
            let v = Vec3::new(corner.x - pos.x, 0.0, corner.z - pos.z);
            let t = corner + Vec3::new(v.z, 0.0, -v.x).normalize_or_zero() * (sgn * (hw + c));
            let t = Vec3::new(t.x, pos.y, t.z);
            for (len, pts) in cs(pos, flat_fwd, t, rr, PASS_TURN) {
                if best.as_ref().is_some_and(|b| len >= b.0) {
                    continue;
                }
                if sweep_clear(env, me, &pts, hw) {
                    best = Some((len, pts));
                }
            }
        }
        let Some((_, pts)) = best else { continue };
        let n = pts.len();
        let end_dir = if n >= 2 { (pts[n - 1] - pts[n - 2]).normalize_or(flat_fwd) } else { flat_fwd };
        let speed = (0.7 * lane_speed).max(MIN_WANT);
        let back = recovery_rail(r, env, me, pts[n - 1], end_dir, rr, RAIL_TURN, speed)?;
        return Some((Path::new(pts, r.v.max(4.0), None), back));
    }
    None
}

/// The speed controller (sub_827A3708 -> sub_826EA420): the constraint entries (car ahead,
/// stop line, slower next lane), each with its hysteresis; the most negative wins, else the
/// positive one with the smallest gap, else free driving towards `want`. Returns the
/// acceleration and the entry chosen.
#[allow(clippy::too_many_arguments)]
fn control(r: &mut Rider, v: f32, want: f32, half_len: f32, ahead: &Ahead, others: &[Other], stop: Option<(usize, f32)>, slower: Option<(f32, f32)>, dt: f32) -> (f32, Option<usize>) {
    let ty = r.ty;
    let max_decel = if ahead.player_across { ty.max_decel * 3.0 } else { ty.max_decel };
    let up = ty.max_accel + r.bonus;
    let mph = v / MPH;
    let mut entries: [Option<(f32, f32, bool)>; 3] = [None; 3];
    // 0: the car ahead (sub_827A3128): centre to centre, half lengths and the standing gap
    // stretched with speed
    let obst = (ahead.j != usize::MAX).then_some(ahead.j);
    if obst != r.obst_id {
        r.on[0] = false;
        r.obst_id = obst;
    }
    if let Some(j) = obst {
        let o = &others[j];
        let margin = half_len + o.half_len + r.gap * (1.0 + 2.0 * (mph / 70.0).min(1.0));
        entries[0] = Some((needed(v, ahead.vt, ahead.d, margin, up, max_decel), ahead.d - margin, o.player));
        r.ahead = (ahead.d, margin);
    }
    // 1: the stop line while the light says stop (sub_827A3570), half our length short of it
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
    let (on_a, off_a) = (ty.min_accel + ty.min_accel_band * 0.5, ty.min_accel - ty.min_accel_band * 0.5);
    let (on_d, off_d) = (ty.min_decel + ty.min_decel_band * 0.5, ty.min_decel - ty.min_decel_band * 0.5);
    let mut best_neg: Option<(f32, usize)> = None;
    let mut best_pos: Option<(f32, f32, usize)> = None;
    for k in 0..3 {
        let Some((mut a, gap, player)) = entries[k] else {
            r.on[k] = false;
            continue;
        };
        // (sub_826E91F8; no room left forces it on)
        if !r.on[k] {
            r.on[k] = a > on_a || -a > on_d || gap <= 0.0;
        } else if gap > 0.0 && ((a >= 0.0 && a < off_a) || (a < 0.0 && -a < off_d)) {
            r.on[k] = false;
        }
        if !r.on[k] {
            continue;
        }
        if player && -a > 1.25 * max_decel {
            a = -1.5 * max_decel;
        } else {
            a = a.clamp(-max_decel, up);
        }
        if a < 0.0 {
            if best_neg.map_or(true, |b| a < b.0) {
                best_neg = Some((a, k));
            }
        } else if best_pos.map_or(true, |b| gap < b.1) {
            best_pos = Some((a, gap, k));
        }
    }
    let (mut a, chosen) = match (best_neg, best_pos) {
        (Some((n, k)), _) => (n, Some(k)),
        (None, Some((p, _, k))) => (p, Some(k)),
        _ => (((want - v) / 1.0).clamp(-(ty.min_decel + ty.max_decel) * 0.5, (ty.min_accel + ty.max_accel + r.bonus) * 0.5), None),
    };
    // moving off from standing: the start delay, counted while the controller asks to go
    if v == 0.0 {
        if a <= 0.0 {
            r.waited = 0.0;
        } else {
            r.waited += dt;
            if r.waited < r.wait {
                a = 0.0;
            }
        }
    }
    (a, chosen)
}

/// The integrator (sub_826EA240).
fn integrate(r: &mut Rider, mut a: f32, dt: f32) {
    r.a = a;
    a = a.min(r.ty.max_accel);
    let mut nv = (r.v + a * dt).min(r.ty.max_speed);
    if a < 0.0 && nv < STOP_SPEED {
        nv = 0.0;
    }
    r.v = nv.max(0.0);
}

/// One frame of a rider on its lane (state 1; sub_827A0138 / sub_826EA420 / sub_826EA240),
/// with the stuck and passing tests after it. Returns the new pose.
#[allow(clippy::too_many_arguments)]
pub fn step(r: &mut Rider, env: &Env, me: usize, half_len: f32, upright: bool, dt: f32) -> (Vec3, Vec3) {
    let nav = env.nav;
    let v = r.v;
    r.off_timer -= dt;
    if r.off_timer <= 0.0 {
        r.draw_offset();
    }
    // wanted speed (sub_827A2B38): the lane's speed and the next lane's, with the offset
    // except on junction lanes
    let (cur, junction) = rail_speed(nav, r.link);
    let want = if junction { cur.max(MIN_WANT) } else { (cur + r.off).max(MIN_WANT) };
    let next_want = match (junction, r.next) {
        (true, _) | (_, None) => want,
        (false, Some(n)) => {
            let (nv, nj) = rail_speed(nav, n);
            if nj { nv.max(MIN_WANT) } else { (nv + r.off).max(MIN_WANT) }
        }
    };
    let pos = r.pos(nav);
    let fwd = r.tangent(nav);
    let lane = nav.links[r.link].lane;
    let next_lane = r.next.map(|n| nav.links[n].lane);
    // the car ahead: kept while both stand (searched again with p = 0.025), else searched
    let kept = match r.kept.clone() {
        Some((j, at)) if v == 0.0 && env.others.get(j).is_some_and(|o| o.vel == Vec3::ZERO && o.pos.distance_squared(at) < 1e-6) && r.rand() > 0.025 => Some(j),
        _ => None,
    };
    let ahead = match kept {
        Some(j) => Ahead { j, d: env.others[j].pos.distance(pos), vt: 0.0, blocking: blocking(&env.others[j], fwd).then_some(j), player_across: false },
        None => {
            let mut rails = vec![lane_segs(nav, env.lanes, lane)];
            if let Some(nl) = next_lane {
                rails.push(lane_segs(nav, env.lanes, nl));
            }
            // (on this rail only from where we are)
            let here = env.lanes.project(nav, lane, pos);
            let mut s0 = 0.0;
            rails[0].retain(|&(a, b)| {
                let keep = s0 + a.distance(b) > here;
                s0 += a.distance(b);
                keep
            });
            find_ahead(nav, me, pos, fwd, r.ty.width.max(1.6), &rails, (Some(lane), next_lane), r.next.map(|n| nav.links[n].a), env.others)
        }
    };
    r.kept = (v == 0.0 && ahead.j != usize::MAX).then(|| (ahead.j, env.others[ahead.j].pos));
    // sub_827A3458: go = green for the lane and the junction's box not held by anything but this car
    // (`Blockers`); a refused car stops at the line as at red (and `waits` counts its demand)
    let stop = signal_stop(env, r, pos).filter(|x| !(x.2 == 2 && !env.blockers.get(&x.3).is_some_and(|b| b.blocks(me)))).map(|x| (x.0, x.1));
    let slower = (next_want <= v).then(|| {
        let end = nav.links[r.lane_end(nav)].b;
        (next_want, end.distance(pos))
    });
    let (mut a, chosen) = control(r, v, want, half_len, &ahead, env.others, stop, slower, dt);
    r.slot = chosen;
    if env.hold {
        a = -r.ty.max_decel;
    }
    integrate(r, a, dt);
    // along the lane (sub_82798F18): the phase moves by the distance over the curve speed
    // (+180, from the last pose); past the lane's end the rest of the distance goes on along
    // the next lane from its start, at that lane's speed there (vfunc 11), clamped to it
    let d = r.v * dt;
    if d > 0.0 {
        let lane = nav.links[r.link].lane;
        let c = &nav.curves[lane];
        let t0 = r.phase(nav);
        if r.ds_lane != lane {
            r.ds = c.speed(t0);
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
                    r.choose_next(nav);
                    let nl = nav.links[n].lane;
                    let c = &nav.curves[nl];
                    r.ds = c.speed(0.0);
                    r.ds_lane = nl;
                    if rest > 0.0 {
                        let t = mcla_formats::rage::navsys::LaneCurve::advance(0.0, rest, r.ds).clamp(0.0, 1.0);
                        r.set_phase(nav, env.lanes, t);
                    }
                }
            }
        }
    }
    let (p, f) = r.pose(nav);
    if !env.event {
        after_switch(r, env, me, p, f, &ahead, chosen, cur, upright, dt);
    }
    (p, f)
}

/// The tests after the state switch (free roam only): stuck behind a blocker (state 8),
/// passing it (state 7).
#[allow(clippy::too_many_arguments)]
fn after_switch(r: &mut Rider, env: &Env, me: usize, pos: Vec3, fwd: Vec3, ahead: &Ahead, chosen: Option<usize>, lane_speed: f32, upright: bool, dt: f32) {
    let semi = r.ty.trailer || (r.ty.cab && r.partner.is_some());
    let light_near = signal_stop(env, r, pos).is_some_and(|x| x.1 < 50.0);
    let blocker = ahead.blocking.filter(|&b| b == ahead.j);
    // stuck (sub_827A3810): standing more than 2 s behind the blocker that limits us: back
    // up 2 m at 4 m/s, three times at most until it is no longer stuck
    let can_stuck = upright && chosen != Some(1) && !semi && blocker.is_some() && chosen == Some(0) && !light_near;
    if can_stuck && r.v == 0.0 {
        r.stuck.0 += dt;
        if r.stuck.0 > 2.0 && r.stuck.2 < 3 {
            r.stuck.2 += 1;
            r.stuck.0 = 0.0;
            r.stuck.1 = 0.0;
            let back = pos - fwd * 2.0;
            let n = 5;
            let pts: Vec<Vec3> = (0..=n).map(|k| pos.lerp(back, k as f32 / n as f32)).collect();
            let mut p = Path::new(pts, 4.0, None);
            p.reverse = true;
            r.take_path(p, 8);
            return;
        }
    } else if !can_stuck {
        r.stuck = (0.0, 0.0, 0);
    }
    // passing (sub_827A42C0 / sub_827A4048)
    let Some(b) = blocker else {
        r.pass_stand = 0.0;
        return;
    };
    let o = &env.others[b];
    let close = if r.v == 0.0 { o.vel.length() < 2.2352 } else { ahead.d < 12.0 || ahead.d / r.v < if lane_speed > 60.0 * MPH { 4.0 } else { 3.0 } };
    if !close || chosen == Some(1) || !upright || semi {
        return;
    }
    // (the next lane's light at stop within 50 m)
    if signal_stop(env, r, pos).is_some_and(|x| x.2 != 2 && x.1 < 50.0) {
        return;
    }
    if r.v == 0.0 {
        r.pass_stand += dt;
    } else {
        r.pass_stand = 0.0;
    }
    let ready = r.pass_stand > 2.0;
    if o.ambient && r.rand() > 1.0 / 60.0 {
        return;
    }
    if let Some((pass, back)) = passing_rail(r, env, me, pos, fwd, b, lane_speed, ready) {
        r.take_path(pass, 7);
        r.then = Some(back);
        r.passing = Some((b, env.others[b].pos));
    }
}

/// One frame on a rail off the lanes (states 6 - 8). Returns the new pose, or None when the
/// car turns physical (the footprint probe hit something).
pub fn step_path(r: &mut Rider, env: &Env, me: usize, pos: Vec3, fwd: Vec3, half_len: f32, upright: bool, dt: f32) -> Option<(Vec3, Vec3)> {
    let hw = r.ty.width.max(1.6) * 0.5;
    let semi = r.ty.trailer || r.ty.cab;
    // state 6: re-planned every frame (on a rail or standing with none)
    if r.state == 6 && !env.event {
        try_recovery(r, env, me, pos, fwd, upright, dt);
    }
    // state 7: what is being passed moved off - a recovery rail instead
    if r.state == 7 {
        if let Some((j, at)) = r.passing {
            if env.others.get(j).map_or(true, |o| o.pos.distance_squared(at) > 0.110889) && try_recovery(r, env, me, pos, fwd, upright, dt) {
                r.passing = None;
            }
        }
    }
    let Some(path) = r.path.as_ref() else {
        // standing, waiting for a rail
        r.v = 0.0;
        return Some((pos, fwd));
    };
    let (q, d) = path.at(path.s);
    let dir = if path.reverse { -d } else { d };
    // the speed: the rail's, with the car ahead along the rail
    let ahead = if path.reverse {
        Ahead { j: usize::MAX, d: 0.0, vt: 0.0, blocking: None, player_across: false }
    } else {
        let segs: Vec<(Vec3, Vec3)> = path.pts.windows(2).zip(path.cum.iter().skip(1)).filter(|(_, &c)| c > path.s).map(|(w, _)| (w[0], w[1])).collect();
        find_ahead(env.nav, me, q, dir, r.ty.width.max(1.6), &[segs], (None, None), None, env.others)
    };
    let want = path.speed;
    let (mut a, chosen) = control(r, r.v, want, half_len, &ahead, env.others, None, None, dt);
    r.slot = chosen;
    if env.hold {
        a = -r.ty.max_decel;
    }
    integrate(r, a, dt);
    let path = r.path.as_mut().unwrap();
    path.s += r.v * dt;
    let (p, d) = path.at(path.s);
    let f = if path.reverse { -d } else { d };
    let done = path.s >= path.len();
    // the footprint probe
    if !semi && probe_hit(env, me, p, f, half_len, hw) {
        if r.state == 8 {
            r.path = None;
            r.state = 6;
            r.stuck.1 = 0.0;
            return Some((p, f));
        }
        return None;
    }
    if r.state == 8 {
        r.stuck.1 += dt;
        if r.stuck.1 > 4.0 || done {
            r.path = None;
            r.state = 6;
            r.stuck.1 = 0.0;
        }
        return Some((p, f));
    }
    if done {
        if r.state == 7 {
            // on to the rail back onto the lane
            match r.then.take() {
                Some(next) => r.take_path(next, 6),
                None => {
                    r.path = None;
                    r.state = 6;
                }
            }
            r.passing = None;
        } else if let Some((link, s)) = r.path.as_ref().and_then(|p| p.end) {
            r.path = None;
            r.state = 1;
            // (a rail set anew: its curve speed is taken anew, sub_82798B98)
            r.place(env.nav, link, s);
            r.on = [false; 3];
            r.choose_next(env.nav);
            return Some((r.pos(env.nav), f));
        } else {
            r.path = None;
        }
    }
    Some((p, f))
}

/// How a riding car stands on the ground (car update sub_826E5D20, decoded 2026-10-20): the
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

/// The drawn wheels' lock (sub_826E5950, written after the render update in sub_826E8EC8): set
/// when the speed controller's entry in charge is the car ahead (car +420 == 0), the mean of the
/// last 4 updates' asked acceleration (car +2336: count, next, sum, 4 entries, pushed by
/// sub_826E4DB0 from sub_826E5AB8 every update) is a deceleration of at least fMaxDecel, and
/// |car +208|^2 >= 4.99612 ((5 mph)^2, the 30 Hz frame's position change). Kept per update index
/// (owner byte +557 + U, U flipping each update): the render update reads the byte of its index
/// before it is written, so the drawn wheels see the bit of two updates before; while it is set
/// they do not turn. The car's skid sound reads the rule directly (sub_826DD748).
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
        self.since += dt;
        if self.since < 1.0 / 30.0 - 1e-4 {
            return;
        }
        let t = self.since;
        self.since = 0.0;
        let v = self.prev.map_or(Vec3::ZERO, |p| (pos - p) / t);
        self.prev = Some(pos);
        // (sub_826E4DB0)
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
        let mean = if self.n == 0 { 0.0 } else { self.sum / self.n as f32 };
        self.now = slot == Some(0) && -mean >= max_decel && v.length_squared() >= 4.99612;
        self.buf[self.u] = self.now;
        self.u ^= 1;
    }
    /// The bit the render update sees (two updates old).
    pub fn drawn(&self) -> bool {
        self.buf[self.u]
    }
    /// sub_826E5950 now (the skid sound).
    pub fn now(&self) -> bool {
        self.now
    }
}

/// The drawn body's pitch and roll from the car's change of velocity (render update
/// sub_82794FA8): a 30 Hz frame's velocity change dv (car +224: v - v of the last frame, v from
/// the positions, car +208), within 30 m (horizontal) of the focus pitch = clamp(dv . forward /
/// fMaxPitchDot) x fMaxPitchAngle (nose up when speeding up), roll = clamp(dv . right /
/// fMaxRollDot) x fMaxRollAngle (right side up: leaning out of a turn); beyond 30 m (0, 0). The
/// last ten samples averaged (sub_82794B78); drawn turned about the car's right axis by the
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
/// fMaxRollAngle (sub_82792518) - the same for every type.
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
                let v = (pos - p) / t;
                (v, v - v0)
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
    // (sub_826E5540 -> sub_827974F0: one-sided faces, the nearest hit - collision.rs `probe_front`)
    let h = col.probe_front(p + Vec3::Y * PROBE_HALF, Vec3::NEG_Y, 2.0 * PROBE_HALF)?;
    Some((h.point, h.normal.normalize_or(Vec3::Y)))
}

/// Should the car set up a probe this frame (sub_826E5D20): trailers every frame; a standing
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
    // (a car on a rail is drawn 0.025 m lower from its second frame on: skeleton +236)
    let drawn = if s.frames > 1 { pos - up * 0.025 } else { pos };
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

impl Vehicles {
    /// Move the traffic cars: along their lanes, on their rails, back from physics; the
    /// trailers behind their cabs.
    pub(super) fn ride_traffic(&mut self, nav: &NavSys, col: &WorldCollider, dt: f32) {
        if !self.cars.iter().any(|c| c.rail.is_some()) {
            return;
        }
        let nav = rail_nav(nav);
        let lanes = lanes(nav);
        let others: Vec<Other> = self
            .cars
            .iter()
            .map(|c| {
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
                    vel: c.state.vel,
                    half_len: hl,
                    half_wid: hw,
                    on,
                    railed: r.is_some_and(|r| matches!(r.state, 6..=9)),
                    player: c.is_player,
                    ambient: r.is_some() || c.parked.is_some(),
                    active: r.is_some_and(|r| r.state == 4) || (r.is_none() && c.parked.is_none()),
                    cab_or_trailer: r.is_some_and(|r| r.ty.cab || r.ty.trailer),
                    key: r.map_or(0, |r| r.rng_id()),
                }
            })
            .collect();
        let hold = self.frozen || self.grid_hold.is_some();
        let event = self.cars.iter().any(|c| c.racer || c.pinned);
        static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let frame = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let parity = frame & 1 == 1;
        let focus = self.cars.iter().find(|c| c.is_player).map_or(Vec3::ZERO, |c| c.state.pos);
        let blockers = self.frame_blockers(nav);
        let env = Env { nav, lanes, col, others: &others, signals: &self.signals, blockers: &blockers, clock: self.clock, event, hold, parity };
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
                    if let Some(w) = trace.as_mut() {
                        use std::io::Write;
                        let p = c.state.pos;
                        let _ = writeln!(w, "{:.2}	{}	{}	{:.2}	{:.2}	{:.2}	{:.3}	{:.2}	{}	0	000	{:.2}	0	0	0	{}	0	0", self.clock, r.rng_id(), c.model.name, p.x, p.y, p.z, c.state.speed(), c.input.brake, c.placed as u8, c.input.throttle, r.state);
                    }
                    // physical: the physics level's rest rule takes it out (state 5: vehicles.rs,
                    // loose.rs `ambient_frame`); a contact makes it active again
                    if r.in_level {
                        if r.state == 4 {
                            continue;
                        }
                    }
                    // (without the level: under 0.1 m/s and 0.1 rad/s for 0.5 s - the port's own)
                    let (sp, w) = (c.state.speed(), c.state.ang_vel.length());
                    if r.state == 4 && !r.in_level {
                        if sp < 0.1 && w < 0.1 {
                            r.rest += dt;
                            if r.rest > 0.5 {
                                r.state = 5;
                                r.v = 0.0;
                                r.stand = 0.0;
                            }
                        } else {
                            r.rest = 0.0;
                        }
                        continue;
                    }
                    if sp > 0.3 && !r.in_level {
                        r.state = 4;
                        r.rest = 0.0;
                        continue;
                    }
                    r.v = 0.0;
                    let fwd = c.state.forward();
                    if !try_recovery(r, &env, i, c.state.pos, fwd, upright, dt) {
                        continue;
                    }
                    // back on a rail: kinematic from here
                    let p = r.path.as_ref().unwrap();
                    p.at(0.0)
                }
                6..=8 => {
                    let (pos, fwd) = (c.state.pos, c.state.forward());
                    match step_path(r, &env, i, pos, fwd, half, upright, dt) {
                        Some(p) => p,
                        None => {
                            let back = matches!(r.path.as_ref(), Some(p) if p.reverse);
                            r.state = 4;
                            r.physical = true;
                            r.wake = true;
                            r.path = None;
                            r.then = None;
                            r.rest = 0.0;
                            c.state.vel = c.state.forward() * if back { -r.v } else { r.v };
                            continue;
                        }
                    }
                }
                // a race path's car let go of by its path (unload): no rail, not posed; its speed
                // +412 is what it measures, |car +208| (sub_826EA420 with no rail; +416 = 0)
                10 => {
                    r.v = c.state.vel.length();
                    c.state.vel = Vec3::ZERO;
                    continue;
                }
                // a race path's car on the path's rails
                9 => step_route(r, &env, i, half, dt),
                _ => {
                    let p = step(r, &env, i, half, upright, dt);
                    // refused by the light this frame (the stop entry is made only then): the
                    // junction counts it as waiting (sub_827A3458)
                    if let Some(ji) = r.stop_id {
                        waits.push((ji, c.state.pos));
                    }
                    p
                }
            };
            let (p, f) = pose;
            let v = r.v;
            let back = matches!(r.path.as_ref(), Some(pp) if pp.reverse);
            // on the ground (sub_826E5D20)
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
            // (the drawn wheels turn at 0.6 of the rolling rate, within 75 m: sub_82794FA8)
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
            let Some(r) = c.rail.as_mut() else { continue };
            let dist2 = Vec3::new(pos.x - focus.x, 0.0, pos.z - focus.z).length_squared();
            r.tilt.update(pos, fwd, right, dist2, dt);
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
                    if let Some(k) = pick {
                        *last = self.clock;
                        let (tp, tr, tv) = (self.cars[k].state.pos, self.cars[k].state.rot, if parked { 0.0 } else if semi { self.cars[k].state.vel.length() } else { self.cars[k].rail.as_ref().unwrap().v });
                        // (a parked car: from its right side)
                        let f = if parked { tr * Vec3::NEG_X } else { tr * Vec3::NEG_Z };
                        let p = &mut self.cars[pi];
                        // (as a reset puts a car: a fresh state, placed on the ground by the step)
                        p.state = crate::physics::CarState::new(tp - f * if semi { 13.0 } else { 9.0 }, (-f.x).atan2(-f.z));
                        p.placed = false;
                        p.state.vel = f * (tv + 12.0);
                        p.pose_history.reset();
                        log::info!("traffic ram {} {} at {:?}", self.cars[k].rail.as_ref().map_or(self.cars[k].uid, |r| r.rng_id()), self.cars[k].model.name, tp);
                    }
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
        for (ji, p) in waits {
            self.signals.wait(ji, p);
        }
    }

    /// The trailers (slot 28 sub_826FB400): the rear bogie trails the cab's hitch - the
    /// smallest turn that points the trailer from the hitch to where its bogie was, then moved
    /// so its hitch is on the cab's. A trailer whose cab is gone is let go (slot 29, reason 16:
    /// the spawner's next step removes it - the cars are not removed in the middle of the
    /// vehicles' update).
    fn tow_trailers(&mut self, col: &WorldCollider, dt: f32) {
        let focus = self.cars.iter().find(|c| c.is_player).map_or(Vec3::ZERO, |c| c.state.pos);
        let ids: hashbrown::HashMap<u32, usize> = self.cars.iter().enumerate().filter_map(|(i, c)| c.rail.as_ref().filter(|r| r.state != 0).map(|r| (r.rng_id(), i))).collect();
        for i in 0..self.cars.len() {
            let Some(r) = self.cars[i].rail.as_mut().filter(|r| r.state == 0) else { continue };
            let Some(&cab) = r.partner.and_then(|id| ids.get(&id)) else {
                r.partner = None;
                continue;
            };
            // (once physical a trailer never trails again: +500 bit 0x10, set by sub_826E96E0, is
            // cleared only at the car's init, sub_826E9BB8 - it lies where it came to rest)
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
                // (the drawn wheels at 0.6 of the rolling rate, as the riding cars': sub_82794FA8)
                w.spin = wrap_spin(w.spin + if near { 0.6 * speed / wr * dt } else { 0.0 });
                w.steer = 0.0;
                w.grounded = true;
            }
            t.placed = true;
        }
    }

    /// Hitch a trailer to a cab just made (slot 5 sub_826E86C8 -> sub_826FB778): with p 0.75
    /// one of the five trailers, its hitch on the cab's, facing the same way.
    pub(super) fn hitch_trailer(&mut self, gpu: &crate::render::Gpu, root: &std::path::Path, nav: &NavSys, cab: usize) {
        let Some(r) = self.cars[cab].rail.as_mut() else { return };
        if !r.ty.cab || r.rand() >= 0.75 {
            return;
        }
        let name = TRAILERS[(r.rand() * 5.0) as usize % 5];
        let shade = (r.rand() * 20.0) as usize % 20;
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
        self.cars[idx].placed = true;
        self.cars[idx].rail = Some(t);
        if let Some(r) = self.cars[cab].rail.as_mut() {
            r.partner = Some(seed);
        }
    }

    /// Something met a traffic car on a rail: from now on the physics has it (base slot 18
    /// sub_826E96E0, state 4). A trailer has no behaviour (car +452 = 0): it is physical from
    /// here (and never trails again); its cab is made active with it (sub_826E7340 ->
    /// sub_826EA0A8, the level's constraints join them).
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
        // (back from physics the seat starts from the car's pose, sub_826E5680)
        r.seat = None;
        c.state.vel = c.state.forward() * if back { -r.v } else { r.v };
    }
}

#[cfg(test)]
mod tests {
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
        let r = RegionRect { index: 0, rect_c: [0.0, 0.0], width: 10.0, len: 40.0, y_range: [0.0, 1.0], mid: Vec3::ZERO, dir: Vec3::X, side: Vec3::Z };
        assert!(region_meets_circle(&r, Vec3::new(0.0, 0.0, 24.9), 20.0));
        assert!(!region_meets_circle(&r, Vec3::new(0.0, 0.0, 25.1), 20.0));
        assert!(region_meets_circle(&r, Vec3::new(39.9, 0.0, 0.0), 20.0));
        assert!(!region_meets_circle(&r, Vec3::new(40.1, 0.0, 0.0), 20.0));
    }

    #[test]
    fn the_drawn_wheels_lock_two_updates_late() {
        // (sub_826E5950: the car ahead in charge, mean asked deceleration >= fMaxDecel, over 5 mph)
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

    #[test]
    fn offsets_are_minus_six_zero_plus_six() {
        let mut r = Rider { ty: AmbType::default(), link: 0, s: 0.0, next: None, v: 0.0, off: 0.0, off_timer: 0.0, bonus: 0.0, gap: 3.0, wait: 0.0, waited: 0.0, on: [false; 3], stop_id: None, obst_id: None, state: 1, physical: false, path: None, then: None, stand: 0.0, pass_stand: 0.0, stuck: (0.0, 0.0, 0), passing: None, kept: None, rest: 0.0, partner: None, seat: None, tilt: Tilt::default(), in_level: false, active: false, wake: false, rng: 7, id: 7, ds: 0.0, ds_lane: usize::MAX, t: 0.0, ahead: (0.0, 0.0), a: 0.0, slot: None, lock: Lock::default(), born: 0.0, life: 8.0, route: None, phys_pair: false, rail_seq: 0, rail_key: None, rail_fresh: true };
        let mut seen = [0usize; 3];
        for _ in 0..3000 {
            r.draw_offset();
            assert!(r.off_timer >= 3.0 && r.off_timer <= 6.0);
            seen[match r.off as i32 { -6 => 0, 6 => 1, 0 => 2, _ => unreachable!() }] += 1;
        }
        assert!(seen.iter().all(|&n| n > 850 && n < 1150), "{seen:?}");
    }

    /// sub_8273A848's box: |a70 . d| < 0.4 h3c and |a60 . d| < 0.4 h40, the height left out.
    #[test]
    fn the_junction_box_is_four_tenths_of_its_half_extents() {
        let b = JunctionBox { c: (10.0, -20.0), a60: [1.0, 5.0, 0.0], a70: [0.0, 5.0, 1.0], h3c: 10.0, h40: 20.0 };
        assert!(b.holds(Vec3::new(10.0, 100.0, -20.0)));
        assert!(b.holds(Vec3::new(10.0 + 7.9, 0.0, -20.0 + 3.9)));
        assert!(!b.holds(Vec3::new(10.0 + 8.0, 0.0, -20.0)));
        assert!(!b.holds(Vec3::new(10.0, 0.0, -20.0 - 4.0)));
    }

    /// bPhysical (instance +140 bit 0x80) only lets inactive pairs in while the car is posed by
    /// its rail (slot 20 / sub_826FA760: the follower has a rail).
    #[test]
    fn a_physical_path_car_pairs_only_while_on_a_rail() {
        let mut r = Rider { ty: AmbType::default(), link: 0, s: 0.0, next: None, v: 0.0, off: 0.0, off_timer: 0.0, bonus: 0.0, gap: 3.0, wait: 0.0, waited: 0.0, on: [false; 3], stop_id: None, obst_id: None, state: 1, physical: false, path: None, then: None, stand: 0.0, pass_stand: 0.0, stuck: (0.0, 0.0, 0), passing: None, kept: None, rest: 0.0, partner: None, seat: None, tilt: Tilt::default(), in_level: false, active: false, wake: false, rng: 7, id: 7, ds: 0.0, ds_lane: usize::MAX, t: 0.0, ahead: (0.0, 0.0), a: 0.0, slot: None, lock: Lock::default(), born: 0.0, life: 8.0, route: None, phys_pair: false, rail_seq: 0, rail_key: None, rail_fresh: true };
        assert!(!r.pairs_inactive());
        r.phys_pair = true;
        for (state, yes) in [(1u8, true), (9, true), (10, false), (0, false), (4, false), (5, false), (6, false)] {
            r.state = state;
            assert_eq!(r.pairs_inactive(), yes, "state {state}");
        }
    }

    #[test]
    fn arc_line_arc_reaches_the_target() {
        // a quarter turn left onto a lane 20 m ahead and 10 m across
        let (p0, h0) = (Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
        let (p1, h1) = (Vec3::new(-10.0, 0.0, -20.0), Vec3::new(-1.0, 0.0, 0.0));
        let ways = csc(p0, h0, p1, h1, 5.0, RAIL_TURN);
        assert!(!ways.is_empty());
        for (len, pts) in &ways {
            assert!(pts.first().unwrap().distance(p0) < 1e-3);
            assert!(pts.last().unwrap().distance(p1) < 1e-2, "{:?}", pts.last());
            let walked: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
            assert!((walked - len).abs() < 0.2, "{walked} {len}");
            // no jumps
            assert!(pts.windows(2).all(|w| w[0].distance(w[1]) < 0.6));
        }
        let ways = cs(p0, h0, Vec3::new(3.0, 0.0, -8.0), 5.0, PASS_TURN);
        assert!(ways.iter().all(|(_, pts)| pts.last().unwrap().distance(Vec3::new(3.0, 0.0, -8.0)) < 1e-2));
    }

    #[test]
    fn corners_in_the_way() {
        let o = |x: f32| Other { pos: Vec3::new(x, 0.0, -10.0), fwd: Vec3::NEG_Z, right: Vec3::X, vel: Vec3::ZERO, half_len: 2.2, half_wid: 0.9, on: None, railed: false, player: false, ambient: true, active: false, cab_or_trailer: false, key: 0 };
        let q = Vec3::new(0.0, 0.0, -10.0);
        // on the line, beside it within half our width, clear of it
        assert!(in_the_way(&o(0.0), q, 1.8));
        assert!(in_the_way(&o(1.5), q, 1.8));
        assert!(!in_the_way(&o(2.5), q, 1.8));
    }
}

// ---------------------------------------------------------------------------------------
// The spawner and unspawner (density manager `mcAmbientDensityTuning`, vtable 0x820C39C4;
// sub_826F79F0: the region list on odd manager frames, sub_826F4E90; spawning sub_826F6920
// and unspawning sub_826F4390 on even ones - 15 Hz each). Decoded 2026-10-19:
// docs/PEOPLE_TRAFFIC.md "Ambient traffic spawner".

/// A road as the spawner weighs it (`mcRnRoadAmbient`).
pub struct Road {
    pub mid: Vec3,
    pub dir: Vec3,
    pub half: glam::Vec2,
    pub speed: f32,
    pub density: i32,
    /// its lanes: first link and length
    pub lanes: Vec<(usize, f32)>,
    pub lane_len: f32,
    /// the box the occluders are asked about (road vfunc 2 sub_8224D130; empty: never hidden)
    pub bmin: Vec3,
    pub bmax: Vec3,
    pub no_box: bool,
    /// the rectangle the front-end mode's sphere and frustum tests take (sub_8224D4C0, sub_826F4E90)
    pub rect: RegionRect,
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
    /// this frame's region box: centre, unit length axis (x, z), full length and width
    /// (the parked-car lists take the same box, sub_826F17D8)
    pub region_box: (glam::Vec2, glam::Vec2, f32, f32),
    /// the cutscene mode this frame (sub_826D4CA0): the parked lists' box as wide as it is long
    pub cutscene: bool,
    /// the front-end mode this frame (sub_826D4C90): its camera and values
    pub frontend: Option<FrontEnd>,
    /// the junctions' rectangles (as `ints`), the junctions the front-end region list holds,
    /// and the front-end lane points (ambient S+48, sub_8273E7E8)
    pub int_rects: Vec<RegionRect>,
    pub fe_ints: Vec<usize>,
    pub fe_pts: FrontEndPoints,
    /// every lane's links on the rail graph, in order
    lane_links: Vec<Vec<usize>>,
}

fn lerp(a: [f32; 2], t: f32) -> f32 {
    a[0] + (a[1] - a[0]) * t
}

/// Do two oriented rectangles (centre, unit x axis, half sizes along it and across) meet
/// (sub_8261B200, a 2D separating-axis test)?
pub(super) fn rects_meet(c1: glam::Vec2, u1: glam::Vec2, h1: glam::Vec2, c2: glam::Vec2, u2: glam::Vec2, h2: glam::Vec2) -> bool {
    let d = c2 - c1;
    let (v1, v2) = (u1.perp(), u2.perp());
    for ax in [u1, v1, u2, v2] {
        let r1 = h1.x * u1.dot(ax).abs() + h1.y * v1.dot(ax).abs();
        let r2 = h2.x * u2.dot(ax).abs() + h2.y * v2.dot(ax).abs();
        if d.dot(ax).abs() > r1 + r2 {
            return false;
        }
    }
    true
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
            let lane_len = lanes.iter().map(|l| l.1).sum();
            let half = glam::Vec2::new(z.rect_len * 0.5, z.width * 0.5);
            // the occluder box: x / z the extremes of the four edge midpoints (not the
            // corners), y the road's range raised 2 m at the top
            let (d, sd) = (z.dir * (z.rect_len * 0.5), z.side * (z.width * 0.5));
            let mids = [z.mid + d, z.mid - d, z.mid + sd, z.mid - sd];
            let (mut bmin, mut bmax) = mids.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
            bmin.y = z.y_range[0];
            bmax.y = z.y_range[1] + 2.0;
            let rect = RegionRect { index: roads.len(), rect_c: z.rect_c, width: z.width, len: z.rect_len, y_range: z.y_range, mid: z.mid, dir: z.dir, side: z.side };
            roads.push(Road { mid: z.mid, dir: z.dir, half, speed: z.speed_limit_mps(), density: z.density, lanes, lane_len, bmin, bmax, no_box: z.no_box, rect });
        }
        let ints = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66"))
            .map(|r| mcla_formats::rage::navsys::parse_intersections(&r))
            .unwrap_or_default();
        let int_rects = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66"))
            .map(|r| mcla_formats::rage::navsys::parse_intersection_rects(&r))
            .unwrap_or_default();
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
        Spawner { roads, ints, tune: Tune::load(root), data_dir: root.to_string_lossy().into_owned(), list: Vec::new(), int_weight: 0.0, listed: vec![false; n], target: 0.0, current: 0, frame: 0, step: 0.0, script: Default::default(), why: [0; 8], player_road: None, region_box: (glam::Vec2::ZERO, glam::Vec2::NEG_Y, 0.0, 0.0), cutscene: false, frontend: None, int_rects, fe_ints: Vec::new(), fe_pts: FrontEndPoints::default(), lane_links }
    }

    /// The road a link belongs to (sub_82245340): its lane's owner when that is a road (flag
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

    /// The density value of a road (sub_8273B670): the script's index, else the road's own,
    /// else 3. In free roam with the player at 30 mph or more (mode byte +0x64250; the
    /// mode's meaning is a guess) the indices map 0 -> 0, 1-2 -> 0.55, 3-5 -> 1.25, 6 -> 0.10.
    fn value(&self, r: u32, free_fast: bool) -> f32 {
        let own = self.roads[r as usize].density;
        // (the front-end mode's index, 0x8282A7D8, before the road's own)
        let base = match self.frontend {
            Some(f) => f.index,
            None => if own >= 0 { own } else { 3 },
        };
        let idx = self.script.get(&r).copied().unwrap_or(base).clamp(0, 6) as usize;
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

/// Is a point seen from the focus (sub_826F0628 / sub_826F0870): nearer than 50 m always;
/// up to 250 m within the field of view either way along the view line (|cos|) and not
/// behind an occluder (sphere r 2, 1.5 m up); beyond, never.
pub(super) fn seen(t: &Tune, focus: Vec3, view: Vec3, mph: f32, p: Vec3, occ: Option<&crate::occluder::Selected>) -> bool {
    let d = Vec3::new(p.x - focus.x, 0.0, p.z - focus.z);
    let dist = d.length();
    if dist < t.spawn_inner {
        return true;
    }
    if dist > t.spawn_outer {
        return false;
    }
    let k = ((mph - t.fov_mph[0]) / (t.fov_mph[1] - t.fov_mph[0])).clamp(0.0, 1.0);
    let fov = lerp(t.fov_deg, k).to_radians();
    if (d / dist).dot(view).abs() < fov.cos() {
        return false;
    }
    !occ.is_some_and(|o| o.hides_sphere(p + Vec3::Y * 1.5, 2.0))
}

// ---------------------------------------------------------------------------------
// The ambient manager's front-end mode (sub_826D4C90: byte 0x829095FE, set by the press-start
// script's AmbientMgr_SetFrontEnd(1, a, b, c, d, n), sub_826D5D10, which also writes S+8312 = a,
// S+8316 = b, S+8304 = c, S+8308 = d and 0x8282A7D8 = n). No player exists on the start screen:
// sub_826D75D8 runs the manager once with the camera (sub_826F14E0: focus its eye, view its
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
    /// the roads' density index (0x8282A7D8, sub_8273B670)
    pub index: i32,
}

/// The front-end lane points (ambient S+48, sub_8273E7E8): the planes and region count they were
/// made for (the list is made again only when either changed), and at most 256 points - where a
/// lane goes into the frustum across one plane, inside the other five: the point, the lane, its
/// phase and the rail link and metres along it (none: a lane off the rail graph).
#[derive(Default, Clone)]
pub struct FrontEndPoints {
    planes: [glam::Vec4; 6],
    regions: Option<usize>,
    pub pts: Vec<(Vec3, usize, f32, Option<(usize, f32)>)>,
}

/// The front-end frustum (sub_8273E2F8): six planes, outward normal and w = normal . point (a
/// point is outside when normal . p - w > 0): near (at S+8304), left, right, far (at S+8308), top,
/// bottom; the sides at atan(tan across) + S+8312 and atan(tan up) + S+8316 off the forward axis.
pub fn frontend_planes(fe: &FrontEnd) -> [glam::Vec4; 6] {
    let [a, b, near, far] = fe.amb;
    let h = fe.tan[1].atan() + a;
    let v = fe.tan[0].atan() + b;
    // the viewport's matrix: z = -forward, x = right, y = up
    let z = -fe.fwd;
    let x = Vec3::Y.cross(z).normalize_or(Vec3::X);
    let y = z.cross(x);
    let local = [
        (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, -near)),
        (Vec3::new(-h.cos(), 0.0, h.sin()), Vec3::ZERO),
        (Vec3::new(h.cos(), 0.0, h.sin()), Vec3::ZERO),
        (Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 0.0, -far)),
        (Vec3::new(0.0, v.cos(), v.sin()), Vec3::ZERO),
        (Vec3::new(0.0, -v.cos(), v.sin()), Vec3::ZERO),
    ];
    local.map(|(n, p)| {
        let nw = x * n.x + y * n.y + z * n.z;
        let pw = fe.eye + x * p.x + y * p.y + z * p.z;
        nw.extend(nw.dot(pw))
    })
}

fn plane_out(pl: glam::Vec4, p: Vec3) -> bool {
    pl.truncate().dot(p) - pl.w > 0.0
}

/// Where a lane goes into a plane's inside (sub_822450A0): the lane at t = k x 0.01010101 (k 0 ..
/// 99, lane vfunc 7); a step from outside to inside gives the middle of the step and its phase;
/// at most `max`.
pub fn lane_entries(c: &LaneCurve, pl: glam::Vec4, max: usize) -> Vec<(Vec3, f32)> {
    let mut out = Vec::new();
    let mut last: Option<(bool, Vec3, f32)> = None;
    for k in 0..100 {
        let t = k as f32 * 0.010_101_01;
        let p = c.eval(t).0;
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

/// sub_8224D4C0 (the regions' vfunc 3): does the circle (x / z) meet the region's rectangle -
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

/// The 8 corners of a region's box (sub_826F4E90's front-end branch): centre +- direction x
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
    /// sub_826F4E90's front-end branch: the regions (roads and junctions) whose rectangle meets
    /// the sphere ahead of the camera - D = S+8308, v = the flat forward x D, L = |v|, centre eye +
    /// v / 2, radius sqrt((L / 2)^2 + (L / tan across)^2); at most 200 (sub_822491E8) - kept,
    /// at most 100, unless all 8 corners of the region's box are outside one plane of the
    /// front-end frustum; the target the sum of their weights (no density scale), 0 .. 1 -> 1.
    /// (The cutscene's own neighbours, sub_826F2000, are not added in this mode.)
    fn region_list_frontend(&mut self, fe: &FrontEnd) {
        let t = self.tune;
        // (the parked lists' own box, sub_826F13A8, at the mode's zero speed and M+324 = 0)
        let vx = glam::Vec2::new(fe.fwd.x, fe.fwd.z).normalize_or(glam::Vec2::NEG_Y);
        let fx = glam::Vec2::new(fe.eye.x, fe.eye.z);
        self.region_box = (fx + vx * lerp(t.box_off, 0.0), vx, lerp(t.box_len, 0.0), lerp(t.box_width, 0.0));
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
                self.listed[k] = true;
                target += w;
            } else {
                let j = k - n_roads;
                let iw = self.ints.get(j).map_or(0.0, |x| x.1) * 0.01 * t.values[1] * speed_factor;
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

    /// sub_8273E7E8 (from sub_826D75D8 every frame in the front-end mode, before the managers
    /// run): the front-end planes and manager 0's region list; when either has changed (the
    /// planes compared exactly, the list by its count only) the points are made again - for every
    /// region whose lanes' box (lane +10 / +20) is not wholly outside one plane, every lane's
    /// entries across each plane (sub_822450A0, 64 at most) that are inside the other five; at
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
    /// Is car `i` ambient traffic the spawner owns?
    fn ambient_car(&self, i: usize) -> bool {
        let c = &self.cars[i];
        // (a trailer has no behaviour: neither counted nor unspawned on its own)
        !c.is_player && !c.cop && !c.racer && !c.pinned && c.parked.is_none() && c.rail.as_ref().is_some_and(|r| r.state != 0)
    }

    /// The road of a car: of the lane it rides (or rode last).
    fn car_road(&self, sp: &Spawner, nav: &NavSys, i: usize) -> Option<u32> {
        sp.road_of(nav, self.cars[i].rail.as_ref()?.link)
    }

    /// The player's lane (sub_826D99B8 for the racer's obstacle): the nearest within the
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
            self.spawner = Some(Spawner::new(nav, &world.assets.root));
        }
        let mut sp = self.spawner.take().unwrap();
        // trailers whose cab is gone
        self.cars.retain(|c| c.rail.as_ref().map_or(true, |r| r.state != 0 || r.partner.is_some()));
        // cars on their way out (car +500 bit 0x4, sub_826E9AD0): the fade +484 falls by 2 a second
        // (0x82040E94); at 0 the car is removed (sub_826D50E8 with its reason)
        for c in self.cars.iter_mut() {
            if let Some(f) = c.removing.as_mut() {
                *f -= 2.0 * dt;
            }
        }
        self.cars.retain(|c| c.removing.map_or(true, |f| f > 0.0));
        // roads a race script set the density of
        for (p, d) in std::mem::take(&mut self.road_density_new) {
            for i in nav.links_near(p, 30.0) {
                let l = &nav.links[i];
                let ab = l.b - l.a;
                let t = ((p - l.a).dot(ab) / ab.length_squared().max(1e-3)).clamp(0.0, 1.0);
                let q = l.a + ab * t;
                if (q.y - p.y).abs() < 7.0 && Vec3::new(q.x - p.x, 0.0, q.z - p.z).length() < 13.0 {
                    self.road_density.insert(l.lane, d);
                    if let Some(r) = sp.road_of(nav, i) {
                        sp.script.insert(r, d);
                    }
                }
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
        // (the cutscene mode, sub_826D4CA0: the focus the camera's eye, the view its forward made
        // flat, no speed - the velocity it takes, [0x828608D0], is zero - and no player's road)
        let cut = self.amb_cutscene;
        let (center, view, mph, player_link) = match cut {
            Some((eye, f)) => (eye, Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z), 0.0, None),
            None => (center, view, mph, player_link),
        };
        sp.cutscene = cut.is_some();
        // (the front-end mode, sub_826D4C90: no player on the start screen - sub_826F14E0 takes the
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
        // (the player's car wiped out - its sim's +592 bit 0x10, set by mcCarDamage sub_823429B0 for
        // WipeoutDelay: sub_826F0548 - no spawning or unspawning, the lists still made)
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
            // (sub_826D75D8: the front-end lane points from manager 0's region list, every frame
            // before the manager runs)
            if let Some(f) = fe {
                sp.frontend_points(nav, &f);
            }
            if sp.frame % 2 == 1 {
                self.region_list(&mut sp, nav, center, view, mph, free_fast);
                // the parked cars' lists right after (sub_826F17D8, vehicles/parked.rs)
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
            } else {
                // sub_826F7988 (traffic, people, then the parked cars) and sub_826F6130 (the
                // unspawners, only while the region list has roads); none while wiped out
                if wiped {
                    continue;
                }
                if enabled {
                    self.spawn_frame(&mut sp, gpu, world, nav, center, view, mph, player_link);
                }
                let parked = self.parked_target > 0;
                // (AmbientMgr_SetParkedCarsEnable(0): no parked car is made - the new career's CarSelection)
                if parked && !self.parked_held {
                    self.parked_spawn(gpu, &sp, center, view, mph);
                }
                if enabled {
                    self.unspawn_frame(&sp, nav, center, view, mph);
                }
                if parked && !sp.list.is_empty() {
                    self.parked_unspawn(&sp, center);
                }
            }
        }
        self.spawner = Some(sp);
    }

    /// sub_826F4E90: the roads in the box ahead of the focus, their weights and the target.
    #[allow(clippy::too_many_arguments)]
    fn region_list(&mut self, sp: &mut Spawner, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32, free_fast: bool) {
        if let Some(f) = sp.frontend {
            sp.region_list_frontend(&f);
            sp.current = (0..self.cars.len()).filter(|&i| self.ambient_car(i) && self.car_road(sp, nav, i).is_some_and(|r| sp.listed[r as usize])).count() as u32;
            return;
        }
        let t = sp.tune;
        let k = (mph / 60.0).clamp(0.0, 1.0);
        let (fx, vx) = (glam::Vec2::new(focus.x, focus.z), glam::Vec2::new(view.x, view.z).normalize_or(glam::Vec2::NEG_Y));
        // the player's road; across the view at under 50 mph: the square box round the focus
        let player_road = sp.player_road;
        let perp = mph < 50.0 && player_road.is_some_and(|r| sp.roads[r as usize].dir.dot(view).abs() < 0.7071);
        let (len, off, width) = if perp { (lerp(t.perp_len, k), lerp(t.perp_off, k), lerp(t.perp_width, k)) } else { (lerp(t.box_len, k), lerp(t.box_off, k), lerp(t.box_width, k)) };
        let bc = fx + vx * off;
        let bh = glam::Vec2::new(len * 0.5, width * 0.5);
        sp.region_box = (bc, vx, len, width);
        let on_freeway = player_road.is_some_and(|r| sp.roads[r as usize].speed >= 29.0129);
        // hidden: behind an occluder of the spawner's view only (sub_82175508 with flags 2 - no
        // frustum test): the eye 1.75 m above the focus, occluders within 400 m
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let speed_factor = lerp(t.density_at, ((mph - t.density_mph[0]) / (t.density_mph[1] - t.density_mph[0])).clamp(0.0, 1.0));
        sp.list.clear();
        sp.listed.iter_mut().for_each(|x| *x = false);
        let mut target = 0.0;
        for ri in 0..sp.roads.len() {
            let r = &sp.roads[ri];
            let rc = glam::Vec2::new(r.mid.x, r.mid.z);
            if rc.distance(bc) > bh.length() + r.half.length() {
                continue;
            }
            let ru = glam::Vec2::new(r.dir.x, r.dir.z).normalize_or(glam::Vec2::X);
            let overlap = rects_meet(bc, vx, bh, rc, ru, r.half);
            let fw = r.speed >= 29.0129;
            let hidden = || !r.no_box && occ.as_ref().is_some_and(|o| o.hides_aabb(r.bmin, r.bmax));
            let accept = if on_freeway && fw {
                true
            } else if !overlap {
                false
            } else if on_freeway {
                !hidden()
            } else {
                let near = rc.distance(fx) <= 200.0;
                let aligned = r.dir.dot(view) > 0.6;
                if fw && !(near || aligned) {
                    false
                } else {
                    (near && aligned) || !hidden()
                }
            };
            if !accept {
                continue;
            }
            let w = r.lane_len * 0.01 * sp.value(ri as u32, free_fast) * speed_factor;
            sp.list.push((ri as u32, w));
            sp.listed[ri] = true;
            target += w;
        }
        // the junctions in the box
        sp.int_weight = 0.0;
        for &(c, w, _) in &sp.ints {
            let p = glam::Vec2::new(c.x, c.z) - bc;
            if p.dot(vx).abs() <= bh.x && p.dot(vx.perp()).abs() <= bh.y {
                let iw = w * 0.01 * t.values[1] * speed_factor;
                sp.int_weight += iw;
                target += iw;
            }
        }
        // (the density scale, 0x826F5504: on the sum, before the clamp - not in the cutscene branch)
        if !sp.cutscene {
            target *= self.density_scale.unwrap_or(1.0);
        }
        if target > 0.0 && target < 1.0 {
            target = 1.0;
        }
        sp.target = target;
        // the cars on listed roads
        sp.current = (0..self.cars.len()).filter(|&i| self.ambient_car(i) && self.car_road(sp, nav, i).is_some_and(|r| sp.listed[r as usize])).count() as u32;
    }

    /// sub_826F6920: up to 60 tries, at most min(target - current, 6) cars.
    #[allow(clippy::too_many_arguments)]
    fn spawn_frame(&mut self, sp: &mut Spawner, gpu: &crate::render::Gpu, world: &crate::world::World, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32, player_link: Option<usize>) {
        let mut n = (sp.target as i32 - sp.current as i32).min(6);
        if n <= 0 || (sp.list.is_empty() && sp.fe_ints.is_empty()) || self.models.is_empty() {
            return;
        }
        // (the front-end mode: one try a frame, 0x826F6C5C)
        let fe = sp.frontend.is_some();
        // no spawning while the player is under 30 mph on a junction's lane (the lane's owner or
        // its parent flagged 4, sub_822452F8)
        if mph * MPH < 13.41 && player_link.is_some_and(|l| (nav.links[l].owner_flags | nav.links[l].parent_flags) & 4 != 0) {
            return;
        }
        let t = sp.tune;
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let total: f32 = sp.list.iter().map(|x| x.1).sum::<f32>() + sp.int_weight;
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
            // count), its lane and phase; the lane's road (sub_82245340), else the try is wasted
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
            for &(ri, w) in &sp.list {
                if x < w {
                    pick = Some(ri);
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
            let road = &sp.roads[ri as usize];
            if road.lanes.is_empty() {
                continue;
            }
            // a lane by length, a point along it
            let mut x = self.rand() * road.lane_len;
            let mut lane = road.lanes[0];
            for &l in &road.lanes {
                lane = l;
                if x < l.1 {
                    break;
                }
                x -= l.1;
            }
            let mut s = self.rand() * lane.1;
            let mut link = lane.0;
            let lane_id = nav.links[link].lane;
            while s > nav.links[link].len {
                s -= nav.links[link].len;
                match nav.links[link].next.first() {
                    Some(&nk) if nav.links[nk].lane == lane_id => link = nk,
                    _ => {
                        s = nav.links[link].len;
                        break;
                    }
                }
            }
            let (link, s) = match fe_at {
                Some((_, k, sk)) => (k, sk),
                None => (link, s),
            };
            let l = &nav.links[link];
            let pos = l.a + (l.b - l.a) * (s / l.len.max(1e-3));
            let dir = l.tangent_at(s / l.len.max(1e-3));
            let fw = road.speed >= 29.0129;
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
            // room: no car within the spacing
            if self.cars.iter().any(|c| Vec3::new(c.state.pos.x - pos.x, 0.0, c.state.pos.z - pos.z).length() < t.spacing && (c.state.pos.y - pos.y).abs() < 5.0) {
                sp.why[5] += 1;
                continue;
            }
            // unseen (a long type at both ends too; the front-end mode has no visibility test,
            // 0x826F7658)
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
            // on the ground: a probe +- 1.75 m at the point, none - no spawn ("could not spawn
            // on terrain", sub_826E4A48)
            let Some((ground, normal)) = probe_ground(&world.collider, pos) else {
                sp.why[7] += 1;
                continue;
            };
            let model = self.models[mi].clone();
            // its colour: one of the type's twenty (global RNG % 20)
            let shade = (self.rand() * 20.0) as usize % 20;
            let color = ty.color(shade).unwrap_or(super::TRAFFIC_PALETTE[shade % super::TRAFFIC_PALETTE.len()]);
            let yaw = (-dir.x).atan2(-dir.z);
            let idx = self.spawn(gpu, model, ground, yaw, color, false);
            // (its lights' hour offset, car +3568: 0 .. 0.25 h - sub_826E86C8)
            self.cars[idx].amb_lamps.hour_offset = self.rand() * 0.25;
            let f = (dir - normal * dir.dot(normal)).normalize_or(dir);
            let right = f.cross(normal).normalize_or(Vec3::X);
            self.cars[idx].state.relocate(ground, Quat::from_mat3(&glam::Mat3::from_cols(right, normal, -f)).normalize());
            let seed = (self.rand() * 16777216.0) as u32 | 1;
            let mut rider = Rider::new(nav, ty, link, s, seed);
            rider.born = self.clock;
            rider.seat = Some(Seat::new(ground, normal, f));
            // the start speed (vfunc 84 sub_826E7FF8, called by the spawn slot 5 sub_826E86C8
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
            self.hitch_trailer(gpu, &world.assets.root, nav, idx);
            if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
                let v = pos - focus;
                log::info!("traffic spawn {} dist {:.1} ahead {:.1} side {:.1} road {} target {:.2} current {}", self.cars[idx].model.name, Vec3::new(v.x, 0.0, v.z).length(), v.dot(view), (v.z * view.x - v.x * view.z).abs(), ri, sp.target, sp.current);
            }
            sp.current += 1;
            n -= 1;
        }
    }

    /// sub_826F4390: 30 cars looked at, at most one removed.
    fn unspawn_frame(&mut self, sp: &Spawner, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32) {
        let amb: Vec<usize> = (0..self.cars.len()).filter(|&i| self.ambient_car(i)).collect();
        if amb.is_empty() {
            return;
        }
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let mut road_cars: hashbrown::HashMap<u32, u32> = hashbrown::HashMap::new();
        for &i in &amb {
            if let Some(r) = self.car_road(sp, nav, i) {
                *road_cars.entry(r).or_default() += 1;
            }
        }
        let weight: hashbrown::HashMap<u32, f32> = sp.list.iter().copied().collect();
        for _ in 0..30 {
            let i = amb[(self.rand() * amb.len() as f32) as usize % amb.len()];
            let c = &self.cars[i];
            // (a layout item's car is left to the parked unspawner's item branch: sub_826F4390
            // skips a car whose +40 is set)
            if c.item.is_some() {
                continue;
            }
            let pos = c.state.pos;
            let Some(r) = c.rail.as_ref() else { continue };
            if r.state != 1 && r.state != 0 {
                // (a trailer, state 0, has no behaviour: left to the rule below as before)
                // not on a navsys lane (sub_826E9870 false: the detached, the physical, the
                // recovery / passing / backing rails): listed when within +16 (the outer
                // radius) of the focus, and then kept while seen or young; otherwise removed
                // (reason 8) - sub_826F4390: remove if !r27 || r24 || stuck || r30 || crowded || !r23
                let near = Vec3::new(pos.x - focus.x, 0.0, pos.z - focus.z).length() < sp.tune.unspawn_outer;
                // (the front-end mode: seen = listed, here within the outer radius, 0x826F48A4)
                let seen_now = if sp.frontend.is_some() { near } else { seen(&sp.tune, focus, view, mph, pos, occ.as_ref()) };
                if near && (self.clock - r.born < r.life || seen_now) {
                    continue;
                }
                if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
                    log::info!("traffic unspawn {} off the lanes (state {}) near {}", c.model.name, r.state, near);
                }
                self.cars.remove(i);
                return;
            }
            let road = self.car_road(sp, nav, i);
            let listed = road.is_some_and(|x| sp.listed[x as usize]);
            // (the front-end mode: a listed car counts as seen - never removed - 0x826F48A4)
            if if sp.frontend.is_some() { listed } else { seen(&sp.tune, focus, view, mph, pos, occ.as_ref()) } {
                continue;
            }
            let stuck = if r.physical { c.state.speed() < 6.7056 } else { r.v == 0.0 && r.next.is_none() && r.on == [false; 3] };
            let crowded = road.is_some_and(|x| weight.get(&x).is_some_and(|w| road_cars.get(&x).copied().unwrap_or(0) as f32 > 1.5 * w));
            let off_list = match road {
                Some(_) => !listed,
                None => Vec3::new(pos.x - focus.x, 0.0, pos.z - focus.z).length() >= sp.tune.unspawn_outer,
            };
            let mut remove = off_list || crowded || stuck;
            if remove && listed && self.clock - r.born < r.life {
                remove = false;
            }
            if remove {
                if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
                    let v = pos - focus;
                    log::info!("traffic unspawn {} dist {:.1} ahead {:.1} age {:.1} off_list {} crowded {} stuck {}", c.model.name, Vec3::new(v.x, 0.0, v.z).length(), v.dot(view), self.clock - r.born, off_list, crowded, stuck);
                }
                self.cars.remove(i);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// The races' ambient paths (`*_AmbientData.xml`, mcAmbientPath; decoded 2026-10-21,
// work/people2/re/ag16_report.txt A1 - A6, the vertex snap work/people2/racepaths/report.txt):
// path init (sub_827393B8: the vertices snapped onto lanes, the curve rail, the join rail, the
// spawn matrix) and the car that follows the path (sub_826DF430 with the path's params; its
// behaviour route sub_8279FA00): the curve rail, the join rail, then the path's lane as
// ordinary traffic, at most fPathMaxSpeed for its whole life.

/// A path vertex looks for a lane within 30 m (sub_82738FB8: the vertex's w, [0x82006BF0]).
pub const SNAP_RADIUS: f32 = 30.0;

/// The nearest point of a curve to `p` (sub_825875F0): `n` chords at t = k / n, the one whose
/// own nearest point is nearest gives the phase bracket [ta, tb]; the side whose third point
/// (ta + h or tb - h, h = (tb - ta) / 3) is nearer is taken first, then `iters` halvings, each
/// keeping the end that has come nearest so far (ties: the lower end first, then the upper).
/// Returns (phase, squared distance, point) as the last halving wrote them.
pub fn curve_nearest(c: &LaneCurve, p: Vec3, n: usize, iters: usize) -> (f32, f32, Vec3) {
    let pt = |t: f32| c.eval(t).0;
    let d2 = |q: Vec3| (p - q).length_squared();
    let mut prev = pt(0.0);
    let mut tprev = 0.0f32;
    let step = 1.0 / n.max(1) as f32;
    let (mut best, mut a, mut b, mut ta, mut tb) = (f32::MAX, prev, prev, 0.0f32, 1.0f32);
    for k in 1..=n {
        let t = k as f32 * step;
        let q = pt(t);
        let d = q - prev;
        let len2 = d.length_squared();
        let u = if len2 == 0.0 { 0.0 } else { ((p - prev).dot(d) / len2).clamp(0.0, 1.0) };
        let e = d2(prev + d * u);
        if e < best {
            best = e;
            a = prev;
            b = q;
            ta = tprev;
            tb = t;
        }
        prev = q;
        tprev = t;
    }
    let h = (tb - ta) * 0.333_333_34;
    let (mut fa, mut fb) = (d2(a), d2(b));
    let mut side = d2(pt(ta + h)) < d2(pt(tb - h));
    let mut least = f32::MAX;
    let mut out = (0.0f32, 0.0f32, Vec3::ZERO);
    for _ in 0..iters {
        let mid = (tb + ta) * 0.5;
        if side {
            out = (ta, fa, a);
            tb = mid;
            b = pt(mid);
            fb = d2(b);
        } else {
            out = (tb, fb, b);
            ta = mid;
            a = pt(mid);
            fa = d2(a);
        }
        if fa <= least {
            least = fa;
            side = true;
        }
        if fb <= least {
            least = fb;
            side = false;
        }
    }
    out
}

/// A lane's box (lane +16 / +32: here the box of its control points, which holds its curve),
/// its stored length (+0c, the curve's: sub_82244E10) and its flags (+4).
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
        let mut flags = vec![0u32; nav.curves.len()];
        for l in &nav.links {
            if let Some(f) = flags.get_mut(l.lane) {
                *f = l.flags;
            }
        }
        nav.curves
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for p in &c.pts {
                    mn = mn.min(*p);
                    mx = mx.max(*p);
                }
                LaneBox { min: mn, max: mx, len: c.length(), flags: flags[i] }
            })
            .collect()
    })
}

/// A lane's answer to a point (lane slot 3 sub_82245388: the 48-byte record {point, tangent,
/// lane, squared distance, phase, valid}).
#[derive(Clone, Copy, Debug)]
pub struct LaneHit {
    pub lane: usize,
    pub t: f32,
    pub d2: f32,
    pub point: Vec3,
    /// the curve's derivative there (sub_82586BC8)
    pub dir: Vec3,
}

/// The lane nearest to `p` within `r` (sub_8224A868 -> sub_8224A4F8 with no heading): the lane
/// quadtree's nodes whose x / z box comes within r (sub_82246BF8 / sub_82246B78), their lanes with
/// flags & 1 whose box comes within r (lane slot 5 sub_82244DC0) asked for their nearest curve
/// point (slot 3: sub_825875F0 with 8 chords for a lane of 450 m or more, else 4, and 10 halvings),
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
        let o = (b.min - p).max(p - b.max).max(Vec3::ZERO);
        if !(r * r > o.length_squared()) {
            continue;
        }
        let c = &nav.curves[lane];
        let n = if b.len >= 450.0 { 8 } else { 4 };
        let (t, d2, point) = curve_nearest(c, p, n, 10);
        if !(d2 < r * r) {
            continue;
        }
        hits += 1;
        if best.as_ref().map_or(true, |x| d2 < x.d2) {
            best = Some(LaneHit { lane, t, d2, point, dir: c.eval(t).1 });
        }
    }
    (best, hits)
}

/// The curve rail of a path (sub_827383B8 -> sub_825867F8 -> sub_82587A78; class sub_82737E80,
/// flags 0x801): a clamped B-spline over `cps`, degree min(3, k - 1) (at least 1), the knots
/// handed over p zeros, 1 .. k - p - 1, then k - p repeated p times, with one more knot put
/// before and after (K0 = K1 - (K2 - K1), the last = the one before + its step; with four
/// knots both copied); TRange = K[count - p - 1] = k - p, TMax = TRange - 1e-4.
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

/// A race path after its init (sub_827393B8).
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
    /// +180: the join rail onto the lane (arc - line - arc, points every half metre) and its
    /// heading at the start
    pub join: Option<Vec<Vec3>>,
    pub join_dir: Vec3,
    /// the spawn matrix after init (sub_82738E48): its place and z row
    pub spawn: Vec3,
    pub row2: Vec3,
    /// both rails' speed: fPathMaxSpeed x 0.44704 m/s (sub_82738A10)
    pub speed: f32,
    /// (diagnostics) the most valid lane answers a vertex had; the original keeps 64
    pub hits: usize,
    /// +170 bPhysical: the car's instance +140 bit 0x80 (`Rider::phys_pair`)
    pub physical: bool,
}

/// Path init (sub_827393B8). `snap`: bSnapToRails per vertex (a single-vertex path that is not
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
    // sub_82738FB8: each flagged vertex onto its nearest lane; the last one keeps the lane
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
    // the join rail (sub_82738558): the last vertex flagged and the one before not; from the
    // vertex before along the file's last step (two vertices: the spawn matrix's -z) to the last
    // along the lane; the shortest of the four arc - line - arc ways at fMinTurningRadius.
    // (With no lane found the lane's direction is unset: no rail is made here.)
    if n >= 2 && flags[n - 1] && !flags[n - 2] {
        if let (Some(rr), Some(_)) = (min_turn, out.lane) {
            let d = if n > 2 { verts[n - 2] - verts[n - 3] } else { -row2 };
            let h0 = Vec3::new(d.x, 0.0, d.z).normalize_or_zero();
            let (p0, p1) = (out.snapped[n - 2], out.snapped[n - 1]);
            if let Some((_, pts)) = csc(p0, h0, p1, out.lane_dir, rr, f32::INFINITY).into_iter().min_by(|a, b| a.0.total_cmp(&b.0)) {
                out.join = Some(pts);
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
        (Some(c), _, _) => Some(c.eval(0.0).1),
        (None, Some(_), _) => Some(out.join_dir),
        (None, None, Some((l, t))) => nav.curves.get(l).map(|c| c.eval(t).1),
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

impl PathRails {
    /// The heading of the spawn matrix (sub_8278F690: atan2(z row x, z row z) in (0, 2 pi]).
    pub fn heading(&self) -> f32 {
        let h = self.row2.x.atan2(self.row2.z);
        if h <= 0.0 { h + std::f32::consts::TAU } else { h }
    }

    /// The follower's route (sub_8279FA00): more than one vertex - the curve rail, then the
    /// join rail, at phase 0 - then the path's lane at its phase; one vertex: the lane at once.
    /// None without a lane.
    pub fn route(&self, path: usize) -> Option<Route> {
        let (lane, lane_t) = self.lane?;
        let mut rails = Vec::new();
        if self.snapped.len() > 1 {
            if let Some(c) = &self.curve {
                let n = (c.length().ceil() as usize).clamp(8, 800);
                let pts: Vec<(Vec3, f32)> = (0..=n).map(|k| {
                    let t = k as f32 / n as f32;
                    (c.eval(t).0, t)
                }).collect();
                rails.push(RouteRail::Curve(c.clone(), pts));
            }
            if let Some(j) = &self.join {
                rails.push(RouteRail::Turn(Path::new(j.clone(), self.speed, None)));
            }
        }
        Some(Route { path, rails, i: 0, t: 0.0, ds: 0.0, speed: self.speed, lane, lane_t })
    }
}

/// A rail of a path's route: the curve rail (with points along it for the car-ahead search) or
/// the join rail.
#[derive(Clone, Debug)]
pub enum RouteRail {
    Curve(LaneCurve, Vec<(Vec3, f32)>),
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
            RouteRail::Turn(p) => p.s / p.len().max(1e-3),
        };
        (self.i, f)
    }

    /// The point and heading where the car is.
    fn peek(&self) -> (Vec3, Vec3) {
        match &self.rails[self.i] {
            RouteRail::Curve(c, _) => {
                let (p, d) = c.eval(self.t);
                (p, d.normalize_or(Vec3::NEG_Z))
            }
            RouteRail::Turn(p) => p.at(p.s),
        }
    }

    /// The pose the follower stores (sub_827986C0 on a curve rail: the curve speed +180 the mean
    /// of |dP/dt| here and the last value).
    fn pose(&mut self) -> (Vec3, Vec3) {
        let (p, d, sp) = match &self.rails[self.i] {
            RouteRail::Curve(c, _) => {
                let (p, d) = c.eval(self.t);
                (p, d.normalize_or(Vec3::NEG_Z), Some(d.length()))
            }
            RouteRail::Turn(p) => {
                let (q, d) = p.at(p.s);
                (q, d, None)
            }
        };
        if let Some(sp) = sp {
            self.ds = if self.ds > 0.0 { (sp + self.ds) * 0.5 } else { sp };
        }
        (p, d)
    }

    /// Rail `k` as segments (from where the car is when `k` is its rail).
    fn segs(&self, k: usize) -> Vec<(Vec3, Vec3)> {
        let here = k == self.i;
        match &self.rails[k] {
            RouteRail::Curve(_, pts) => pts.windows(2).filter(|w| !here || w[1].1 > self.t).map(|w| (w[0].0, w[1].0)).collect(),
            RouteRail::Turn(p) => p.pts.windows(2).zip(p.cum.iter().skip(1)).filter(|(_, &c)| !here || c > p.s).map(|(w, _)| (w[0], w[1])).collect(),
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
/// (sub_82798F18; the phase by distance over the curve speed on the curve rail, by arc length on
/// the join rail), wanted speed max(10 mph, rail speed + offset) (sub_827A2B38), the car ahead
/// along the rest of this rail and the next (the next path rail or the lane). Past the last rail
/// the rest of the distance goes on along the path's lane from its phase (the lane-end turn
/// sub_8279FBB0 returns the route's lane), then ordinary traffic (state 1). (The stop-line and
/// slower-lane entries are not made on these rails, as on the other run-time rails here.)
pub fn step_route(r: &mut Rider, env: &Env, me: usize, half_len: f32, dt: f32) -> (Vec3, Vec3) {
    let nav = env.nav;
    r.off_timer -= dt;
    if r.off_timer <= 0.0 {
        r.draw_offset();
    }
    let ahead = {
        let rt = r.route.as_ref().unwrap();
        let (pos, fwd) = rt.peek();
        let mut rails = vec![rt.segs(rt.i)];
        let (next_lane, next_start) = if rt.i + 1 < rt.rails.len() {
            rails.push(rt.segs(rt.i + 1));
            (None, None)
        } else {
            rails.push(lane_segs(nav, env.lanes, rt.lane));
            (Some(rt.lane), nav.curves.get(rt.lane).map(|c| c.eval(rt.lane_t).0))
        };
        find_ahead(nav, me, pos, fwd, r.ty.width.max(1.6), &rails, (None, next_lane), next_start, env.others)
    };
    let want = (r.route.as_ref().unwrap().speed + r.off).max(MIN_WANT);
    let (mut a, chosen) = control(r, r.v, want, half_len, &ahead, env.others, None, None, dt);
    r.slot = chosen;
    r.kept = None;
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
                RouteRail::Curve(c, _) => {
                    if rt.ds <= 0.0 {
                        rt.ds = c.speed(rt.t);
                    }
                    let t0 = rt.t;
                    let t1 = mcla_formats::rage::navsys::LaneCurve::advance(t0, d, rt.ds);
                    if t1 <= 1.0 {
                        rt.t = t1;
                        break;
                    }
                    d = (t1 - 1.0) * d / (t1 - t0).max(1e-6);
                }
                RouteRail::Turn(p) => {
                    p.s += d;
                    if p.s <= p.len() {
                        break;
                    }
                    d = p.s - p.len();
                    p.s = p.len();
                }
            }
            if rt.i + 1 < rt.rails.len() {
                // the next rail, from its start (its curve speed taken anew)
                rt.i += 1;
                rt.t = 0.0;
                rt.ds = 0.0;
                if let RouteRail::Turn(p) = &mut rt.rails[rt.i] {
                    p.s = 0.0;
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
                r.ds = c.speed(lt);
                r.ds_lane = lane;
                if d > 0.0 {
                    let t = mcla_formats::rage::navsys::LaneCurve::advance(lt, d, r.ds).min(1.0);
                    r.set_phase(nav, env.lanes, t);
                }
            }
            r.state = 1;
            r.on = [false; 3];
            r.choose_next(nav);
            return r.pose(nav);
        }
    }
    r.route.as_mut().unwrap().pose()
}

/// What became of a race path car's request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSpawn {
    Done,
    /// no ground under the spawn matrix yet (the request waits up to its 3 s, sub_826E0438)
    NoGround,
    /// not made, or taken out at once (sub_826D50E8 reason 17)
    Removed,
}

impl Vehicles {
    /// The shared random numbers (the MWC at 0x82860890) for the race script's draws.
    pub fn shared_mwc(&mut self) -> &mut super::parked::Mwc {
        &mut self.park_rng
    }

    /// A type for a random ambient path (sub_82738B08 -> sub_826DA8D0 on the traffic list,
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

    /// The start speed (car vfunc 84 sub_826E7FF8, called by the spawn slot 5 sub_826E86C8 for
    /// every ambient car - traffic too - and at a race path's trigger; listings
    /// work/people2/racepaths/v84.txt, work/people2/racepaths2/m.txt). `start`: the path's
    /// fPathStartSpeed x 0.44704 (0 when bParked, or referenced and not fired); None for a car
    /// without a path. f25 = the rail's speed + the car's offset (+424). No rail, a junction lane
    /// (the rail's owner or its parent flagged 4) or online: v = start, else f25, removed when
    /// under 0. Else the cars on the rail (sub_826E7E28; phase = behaviour +208): the one ahead
    /// (least phase ahead, else the least on the next rail) caps it at sqrt(|v_a|^2 + fMaxDecel
    /// (gap - 2)) with our fMaxDecel, the one behind (greatest phase behind) asks at least
    /// sqrt(|v_b|^2 - fMaxDecel_b (gap - 2)), gap = the centres' distance less both half lengths,
    /// a negative term removes; the light (`light_start_cap`); the slot test (`slot_clear`), which
    /// removes a car that is no cab and no trailer (nor car +500 bit 0x20: the cars of the ambient
    /// manager's second pool, sub_826D7CE0 -> sub_826E99C8 with 1, which serves the bIsProxy
    /// types, sub_826DE670 -> sub_826D6B48; such a car is never removed anyway).
    /// Then f25 = max(f25, 0); removed when the least is over the cap, over f25 or over the
    /// light's speed; v = min(start or f25, cap, light), removed when under the least. A bIsProxy
    /// type (+108) is never removed. Removed: None (sub_826D50E8, reason 17). `size`: the car's
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
        // sub_826E7E28: the rail's car list (sub_8273E108: in the order the cars took the rail,
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

    /// The light's start speed (sub_826E7FF8 at 0x826E8394): when the next rail (behaviour +192)
    /// is a junction lane (sub_82245218: its owner or the owner's parent flagged 4), its junction
    /// J (sub_822452F8) lets the car go when the lane is green (sub_8273A178: the lane's first
    /// span within 25 degrees of the current arm, timer < 6.5 s; `Junction::lane_state` 2) and J
    /// is not blocked (sub_8273A848, `junction_blocked`). d = |the lane's first control point (rail
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

    /// sub_8273A848: is navsys junction `ji` blocked for a car to enter? (1) A driver's car (the
    /// vehicle manager 0x82874374's list: the player, racers, police) whose lane (racer +640,
    /// `DriverLanes::best`) is a lane of the junction (sub_822452F8) and whose squared distance
    /// (+644) is under (4.5 x 0.667)^2; (2) the cars on the junction's rails (sub_8273C1B8: its
    /// lanes, then its children's, each rail's car list in order, 64 at most altogether), not
    /// trailers, the car itself left out: one whose lane is not green (sub_8273A178) and that
    /// moves (+412 not 0); or one standing inside the junction's box (centre +0x50 / +0x58,
    /// |axis +0x70 . d| < 0.4 x +0x3c and |axis +0x60 . d| < 0.4 x +0x40) that has no next rail,
    /// or whose next rail is the junction's and not green, or that is headed (its forward,
    /// sub_826DDD90) off the green arm (sub_8273A360).
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

    /// Everything sub_8273A848 would call a hit on navsys junction `ji`, collected: `players`
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

    /// The blockers of every junction that has a car on its lanes or the player's lane, once a
    /// frame, for the riders' light (sub_827A3458 -> sub_8273A848(J, car)).
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

    /// One step of the junctions' lights (sub_8273ABC0 for all, sub_826D75D8's player demand):
    /// `blocked` = sub_8273A848(J, 0) for the junctions whose timer passes 10 s this step, and
    /// the player's demand at the junction ahead of its lane (`player_junction_ahead`; none in
    /// the cutscene mode). Priority (sub_826F61D8): a race exists and the player is over 10 mph.
    /// Approximate: evaluated once after the riders moved (the original's order against the
    /// ambient manager update is not traced).
    pub fn step_signals(&mut self, nav: &NavSys, dt: f32, race: Option<(f32, f32)>) {
        let nav = rail_nav(nav);
        let mut blocked = vec![false; self.signals.junctions.len()];
        let mut lists = None;
        for nj in 0..nav.junction_centers.len() {
            let Some(ji) = signals_junction(&self.signals, nav, nj) else { continue };
            let j = &self.signals.junctions[ji];
            if j.forced != -1 || j.timer + dt <= crate::signals::PHASE {
                continue;
            }
            let lists = lists.get_or_insert_with(|| self.rail_lists(nav));
            let b = self.junction_blockers(nav, nj, Some(lists));
            blocked[ji] = b.players || !b.cars.is_empty();
        }
        let mut demand = None;
        let mut player = None;
        if let Some(c) = self.cars.iter().find(|c| c.is_player) {
            player = Some((c.state.pos, c.state.vel));
            if self.amb_cutscene.is_none() {
                if let Some(dl) = self.driver_lanes.get(&c.uid) {
                    if let Some((lane, _)) = dl.best {
                        let t = dl.list.iter().find(|e| e.0 == lane).map_or(0.0, |e| e.1);
                        let tangent = nav.curves[lane].eval(t).1;
                        let reverse = !(c.state.forward().dot(tangent) > 0.0);
                        if let Some(ji) = player_junction_ahead(nav, lane, reverse).and_then(|nj| signals_junction(&self.signals, nav, nj)) {
                            let mph = c.state.vel.length() * 2.236_936_3;
                            demand = Some((ji, c.state.pos, race.is_some() && mph > 10.0));
                        }
                    }
                }
            }
        }
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

    /// The driver cars' lanes, once a frame (sub_822A9D98 -> the obstacle's vfunc 11
    /// sub_826D99B8 with the flag mcRacer vfunc 10 = 1): w = the car's radius (racer +480 =
    /// sqrt(hx^2 + hz^2), hx / hz the greater magnitude of the bound box's least and greatest x /
    /// z, sub_822AEBE0) + 4.5 x 0.5; the lane quadtree's lanes within w at the car's place (flags
    /// & 1, 48 at most: sub_82249210); one entry of the car's list drawn (the shared MWC,
    /// (lo & 0x7FFFFFFF) % n: the racer's obstacle has no car at +16): when the query has its
    /// lane and the lane's nearest point (slot 3, sub_82245388) is within w its phase is kept,
    /// else the entry goes (the last moved into its place, sub_826DA0A8); then one of the
    /// query's lanes drawn, unless it was the one just checked: in the list - kept or dropped
    /// as before; not - added with its phase when within w and the list has fewer than 12
    /// (sub_826DA058). The lane reported: the entry whose point at its kept phase is nearest
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
                                dl.list.push((lane, t));
                            }
                        }
                    }
                }
            }
            let mut best: Option<(usize, f32)> = None;
            for &(lane, t) in &dl.list {
                let q = nav.curves[lane].eval(t).0;
                let d2 = (q - p).length_squared();
                if best.map_or(f32::MAX, |b| b.1) > d2 {
                    best = Some((lane, d2));
                }
            }
            if std::env::var_os("MCLA_DRIVER_LANE_LOG").is_some() && best.map(|b| b.0) != dl.best.map(|b| b.0) {
                log::info!("driver car {uid}: lane {:?} ({} listed, {} near, w {w:.2})", best.map(|b| (b.0, b.1.sqrt())), dl.list.len(), found.len());
            }
            dl.best = best;
            self.driver_lanes.insert(uid, dl);
        }
        self.driver_lanes.retain(|k, _| live.contains(k));
    }

    /// The slot test sub_826F5FE0 (with sub_826F3A40), only on a navsys lane (rail flags & 0x1800
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
    /// has no rails), its type's top speed fPathMaxSpeed for life (sub_826EA240 reads behaviour
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
        r.choose_next(nav);
        r.born = self.clock;
        if !route.rails.is_empty() {
            r.route = Some(Box::new(route));
            r.state = 9;
        }
        r.phys_pair = pr.physical;
        Some(r)
    }

    /// The follower of item `h` for its race path (sub_826DF430 with the path's params: the
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
        let model = self.models[mi].clone();
        let size = (if ty.width > 0.0 { ty.width } else { model.max.x - model.min.x }, if ty.length > 0.0 { ty.length } else { model.max.z - model.min.z });
        let v = if standing { 0.0 } else { start_mph * MPH };
        let Some(v) = self.path_start_speed(nav, &rider, pr.spawn, size, Some(v), None) else {
            if std::env::var_os("MCLA_RACEPATH_LOG").is_some() { log::info!("race path {path}: car removed at its start (the car ahead or behind)"); }
            return PathSpawn::Removed;
        };
        rider.v = v;
        let color = ty.color(shade).unwrap_or(super::TRAFFIC_PALETTE[shade % super::TRAFFIC_PALETTE.len()]);
        let dir = Vec3::new(-pr.row2.x, 0.0, -pr.row2.z).normalize_or(Vec3::NEG_Z);
        let yaw = (-dir.x).atan2(-dir.z);
        let idx = self.spawn(gpu, model, ground, yaw, color, false);
        let f = (dir - normal * dir.dot(normal)).normalize_or(dir);
        let right = f.cross(normal).normalize_or(Vec3::X);
        self.cars[idx].state.relocate(ground, Quat::from_mat3(&glam::Mat3::from_cols(right, normal, -f)).normalize());
        rider.seat = Some(Seat::new(ground, normal, f));
        let c = &mut self.cars[idx];
        c.item = Some(h);
        c.rail = Some(rider);
        if let Some(ai) = c.ai.as_mut() {
            ai.link = None;
        }
        if std::env::var_os("MCLA_RACEPATH_LOG").is_some() { log::info!("race path {path}: item {h} {} follows its path from {:?} at {:.1} m/s (rails {}, top {:.1} m/s)", c.model.name, ground, v, c.rail.as_ref().map_or(0, |r| r.route.as_ref().map_or(0, |rt| rt.rails.len())), pr.speed); }
        PathSpawn::Done
    }

    /// The parked placement of item `h` (sub_826DF7F0, as parked.rs places an item's car): at its
    /// place and heading, no behaviour.
    fn place_path_parked(&mut self, gpu: &crate::render::Gpu, root: &std::path::Path, h: u32) -> PathSpawn {
        let Some(item) = self.layout.get(h as usize).cloned().flatten() else { return PathSpawn::Removed };
        let Some(mi) = self.models.iter().position(|m| m.name.eq_ignore_ascii_case(&item.name)) else { return PathSpawn::Removed };
        let shade = self.park_rng.index(20) as usize;
        let mname = self.models[mi].name.clone();
        let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| AmbType::load(root, &mname));
        let color = ty.color(shade).unwrap_or(super::TRAFFIC_PALETTE[shade % super::TRAFFIC_PALETTE.len()]);
        let model = self.models[mi].clone();
        let idx = self.spawn(gpu, model, item.pos, item.heading, color, false);
        let born = self.clock;
        let life = 8.0 + 4.0 * self.rand();
        let steer = parked_steer(1.0 - self.rand(), self.rand());
        let c = &mut self.cars[idx];
        c.ai = None;
        c.parked = Some(super::parked::ITEM_SPOT);
        c.item = Some(h);
        c.amb_born = born;
        c.amb_life = life;
        c.park_steer = steer;
        for wh in c.state.wheels.iter_mut().take(2) {
            wh.steer = steer;
        }
        c.settle = super::PARK_PROBE;
        PathSpawn::Done
    }

    /// A trigger fired race path `path` whose item is `h` (sub_82687F58): the item's path is
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
                            r.choose_next(nav);
                        }
                        r.path = None;
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
            c.park_steer = 0.0;
            for w in c.state.wheels.iter_mut() {
                w.steer = 0.0;
            }
            if std::env::var_os("MCLA_RACEPATH_LOG").is_some() { log::info!("race path {path}: item {h} {} fired, drives off at {:.1} m/s", c.model.name, rider.v); }
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

    /// Race_UnloadAmbientPathArray (path vfunc sub_82739228: the item removed, reason 11): a car
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
                        // behaviour vfunc 11 sub_8279F650 clears the follower's rails that were
                        // the path's (sub_82798C98, sub_827986A8): with no current rail
                        // sub_82798F18 does not advance and sub_827986C0 leaves the pose flag
                        // 0x80 clear, so the car is not posed (slot 20 / sub_826FA760 skipped)
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
