//! What a race's own script sets up besides the checkpoints (`script_obj/game/race/
//! <district>/<type>/<race>.sco` and its `<race>_AmbientData.xml`):
//!
//! * props placed for the event: stunt ramps, road flares;
//! * the scripted traffic of the race (`mcAmbientPathArray`: a spawn matrix, a path, speeds
//!   in mph, the vehicle type; parked cars, hazards, cars that pull out in front of you);
//! * triggers that start it - a timer, a cylinder, a ring whose firing distance depends on
//!   the speed of the car driving in, a line across the road - each with a random share of
//!   its actions;
//! * junctions whose lights are switched for the field, AI nitro points, road densities.
//!
//! The script is straight-line set-up code: `formats::sco` lists its native calls with
//! their constant arguments, which is all that is read here. Natives are known by the
//! hash of their name only; the ones used are told apart by the parameter names they are
//! called with ("oCenter", "rRadius", "iAmbientPathIndex", ...).

use glam::{Quat, Vec3};
use mcla_formats::sco::{Call, Val};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum Shape {
    /// fires this many seconds into the race (negative: before the start)
    Timer(f32),
    Cylinder { c: Vec3, r: f32, h: f32 },
    /// fires between `r` (driving in at `max_speed` mph or more) and `inner` (at `min_speed`)
    Ring { c: Vec3, r: f32, inner: f32, h: f32, min_speed: f32, max_speed: f32 },
    Line { a: Vec3, b: Vec3 },
}

#[derive(Clone, Debug)]
pub enum Action {
    /// start path n of the ambient data
    Ambient(usize),
    /// lights at the junction: green towards `toward`, or red all round
    Intersection { center: Vec3, toward: Vec3, green: bool },
    RoadDensity { center: Vec3, density: i32 },
    /// opponents light their nitrous here
    Nitro,
    /// a sound at a place: a whoosh (`WHOOSHES_*`) or a police radio call (`DISPATCH_*`)
    Sound { pos: Vec3, object: String },
}

#[derive(Clone, Debug)]
pub struct Trigger {
    pub shape: Shape,
    /// 0 = every lap
    pub lap: i32,
    pub randomness: f32,
    /// how many of its events are chosen at its reset (nMinTriggered / nMaxTriggered; the
    /// trigger's ctor sub_82686340 sets -1, -1 = all, and fRandomness 1)
    pub n_min: i32,
    pub n_max: i32,
    pub player_can: bool,
    pub by_opponent: bool,
    pub multiple: bool,
    pub actions: Vec<Action>,
}

/// A path of the race's `mcAmbientPathArray` (parser sub_82739570). (bHazard, bFire, bSmoke and
/// bDamage have no reader in the executable - only the parser's default and the unload's clear.)
#[derive(Clone, Debug, Default)]
pub struct AmbPath {
    /// SpawnMtx: its place, rotation and z row
    pub pos: Vec3,
    pub rot: Quat,
    pub row2: Vec3,
    pub verts: Vec<Vec3>,
    /// bSnapToRails, per vertex
    pub snap: Vec<bool>,
    /// fPathStartSpeed, fPathMaxSpeed, fPathEndSpeed (mph)
    pub speed: [f32; 3],
    pub parked: bool,
    /// bPhysical: the car's instance bit 0x80 (sub_826FA588 from sub_826EAD00) - a riding or
    /// parked car that meets the other inactive ambient cars (not ported: the physics level's pair
    /// test has no such flag yet)
    pub physical: bool,
    pub spawn_when_triggered: bool,
    pub random_large: bool,
    pub random_small: bool,
    pub type_name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Dressing {
    /// radius of every checkpoint (m); the last of a race is its finish line
    pub cp_radius: Vec<f32>,
    /// (prop name, position, orientation)
    pub props: Vec<(String, Vec3, Quat)>,
    pub triggers: Vec<Trigger>,
    pub paths: Vec<AmbPath>,
}

/// `dir/<name>` or `dir/0x<joaat(name)>` (files whose names the archive did not keep).
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    let p = dir.join(name);
    if p.is_file() {
        return Some(p);
    }
    let p = dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(name)));
    p.is_file().then_some(p)
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::from(a)
}

fn parse_paths(xml: &str) -> Vec<AmbPath> {
    let mut out = Vec::new();
    let floats = |s: &str| -> Vec<f32> { s.split_whitespace().filter_map(|x| x.parse().ok()).collect() };
    for item in xml.split("<Item>").skip(1) {
        let block = |tag: &str| -> Option<&str> {
            let a = item.find(&format!("<{tag}"))?;
            let b = item[a..].find('>')? + a + 1;
            let e = item[b..].find(&format!("</{tag}>"))? + b;
            Some(&item[b..e])
        };
        let flag = |tag: &str| item.find(&format!("<{tag} value=\"")).map_or(false, |a| item[a..].split('"').nth(1) == Some("true"));
        let value = |tag: &str| item.find(&format!("<{tag} value=\"")).and_then(|a| item[a..].split('"').nth(1)).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let m = block("SpawnMtx").map(floats).unwrap_or_default();
        if m.len() < 12 {
            continue;
        }
        let rot = Quat::from_mat3(&glam::Mat3::from_cols(Vec3::new(m[0], m[1], m[2]), Vec3::new(m[3], m[4], m[5]), Vec3::new(m[6], m[7], m[8]))).normalize();
        let verts: Vec<Vec3> = block("vVerts").map(floats).unwrap_or_default().chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
        let snap: Vec<bool> = block("bSnapToRails").map(|b| b.split_whitespace().map(|x| x.parse::<i32>().map_or(false, |v| v != 0)).collect()).unwrap_or_default();
        out.push(AmbPath {
            pos: Vec3::new(m[9], m[10], m[11]),
            rot,
            row2: Vec3::new(m[6], m[7], m[8]),
            verts,
            snap,
            speed: [value("fPathStartSpeed"), value("fPathMaxSpeed"), value("fPathEndSpeed")],
            parked: flag("bParked"),
            physical: flag("bPhysical"),
            spawn_when_triggered: flag("bSpawnWhenTriggered"),
            random_large: flag("bRandomLargeAmbient"),
            random_small: flag("bRandomSmallAmbient"),
            type_name: block("TypeName").unwrap_or("").trim().to_ascii_lowercase(),
        });
    }
    out
}

fn parse_calls(calls: &[Call]) -> (Vec<(String, Vec3, Quat)>, Vec<Trigger>) {
    use std::collections::HashMap;
    let mut props = Vec::new();
    let mut triggers: Vec<Trigger> = Vec::new();
    // handle (offset of the creating call) -> trigger / (trigger, action)
    let mut trig_of: HashMap<usize, usize> = HashMap::new();
    let mut act_of: HashMap<usize, (usize, usize)> = HashMap::new();
    let blank = |shape: Shape| Trigger { shape, lap: 0, randomness: 1.0, n_min: -1, n_max: -1, player_can: true, by_opponent: false, multiple: false, actions: Vec::new() };
    for c in calls {
        let a = &c.args;
        // a prop: (group, position, euler angles, "props", name)
        if a.len() == 5 && a[3].str() == Some("props") {
            if let (Some(p), Some(r), Some(name)) = (a[1].vec3(), a[2].vec3(), a[4].str()) {
                let rot = Quat::from_euler(glam::EulerRot::YXZ, r[1], r[0], r[2]);
                props.push((name.to_ascii_lowercase(), v3(p), rot));
            }
            continue;
        }
        let Some(h) = a.first().and_then(|v| v.ret()) else { continue };
        let key = a.get(1).and_then(|v| v.str());
        match (key, a.len()) {
            // a child action of a trigger
            (Some(kind), 2) if trig_of.contains_key(&h) => {
                let ti = trig_of[&h];
                let act = match kind {
                    "Ambient" => Action::Ambient(usize::MAX),
                    "Intersection" => Action::Intersection { center: Vec3::ZERO, toward: Vec3::ZERO, green: true },
                    "RoadDensity" => Action::RoadDensity { center: Vec3::ZERO, density: 0 },
                    "Nitro" => Action::Nitro,
                    "Sound" => Action::Sound { pos: Vec3::ZERO, object: String::new() },
                    _ => continue,
                };
                triggers[ti].actions.push(act);
                act_of.insert(c.pc, (ti, triggers[ti].actions.len() - 1));
            }
            (Some(k), 3) => {
                let num = a[2].num().map(|v| v as f32);
                let vec = a[2].vec3().map(v3);
                if let Some(&(ti, ai)) = act_of.get(&h) {
                    match (&mut triggers[ti].actions[ai], k) {
                        (Action::Ambient(i), "iAmbientPathIndex") => *i = num.unwrap_or(-1.0).max(0.0) as usize,
                        (Action::Intersection { center, .. }, "vIntersectionCenter") => *center = vec.unwrap_or_default(),
                        (Action::Intersection { toward, .. }, "vDirection") => *toward = vec.unwrap_or_default(),
                        (Action::Intersection { green, .. }, "bGreenElseAllRed") => *green = num.unwrap_or(1.0) != 0.0,
                        (Action::RoadDensity { center, .. }, "RoadCenter") => *center = vec.unwrap_or_default(),
                        (Action::RoadDensity { density, .. }, "iDensity" | "Density") => *density = num.unwrap_or(0.0) as i32,
                        (Action::Sound { pos, .. }, "Position") => *pos = vec.unwrap_or_default(),
                        (Action::Sound { object, .. }, "GameObject") => *object = a[2].str().unwrap_or("").to_string(),
                        _ => {}
                    }
                    continue;
                }
                // a parameter of a trigger: the first one that only a certain kind has decides the kind
                let ti = match trig_of.get(&h) {
                    Some(&t) => t,
                    None => {
                        let shape = match k {
                            "rTimer" => Shape::Timer(0.0),
                            "oCenter" => Shape::Cylinder { c: Vec3::ZERO, r: 10.0, h: 20.0 },
                            "oPosStart" => Shape::Line { a: Vec3::ZERO, b: Vec3::ZERO },
                            _ => continue,
                        };
                        triggers.push(blank(shape));
                        trig_of.insert(h, triggers.len() - 1);
                        triggers.len() - 1
                    }
                };
                let t = &mut triggers[ti];
                match k {
                    "rTimer" => {
                        if let Shape::Timer(v) = &mut t.shape {
                            *v = num.unwrap_or(0.0);
                        }
                    }
                    "oCenter" => match &mut t.shape {
                        Shape::Cylinder { c, .. } | Shape::Ring { c, .. } => *c = vec.unwrap_or_default(),
                        _ => {}
                    },
                    "rRadius" => match &mut t.shape {
                        Shape::Cylinder { r, .. } | Shape::Ring { r, .. } => *r = num.unwrap_or(10.0),
                        _ => {}
                    },
                    "rHeight" => match &mut t.shape {
                        Shape::Cylinder { h, .. } | Shape::Ring { h, .. } => *h = num.unwrap_or(20.0),
                        _ => {}
                    },
                    "rRadiusInner" | "rMinSpeed" | "rMaxSpeed" => {
                        // a cylinder with these is a ring
                        if let Shape::Cylinder { c, r, h } = t.shape.clone() {
                            t.shape = Shape::Ring { c, r, inner: r * 0.3, h, min_speed: 0.0, max_speed: 100.0 };
                        }
                        if let Shape::Ring { inner, min_speed, max_speed, .. } = &mut t.shape {
                            match k {
                                "rRadiusInner" => *inner = num.unwrap_or(*inner),
                                "rMinSpeed" => *min_speed = num.unwrap_or(0.0),
                                _ => *max_speed = num.unwrap_or(100.0),
                            }
                        }
                    }
                    "oPosStart" | "oPosEnd" => {
                        if let Shape::Line { a, b } = &mut t.shape {
                            if k == "oPosStart" {
                                *a = vec.unwrap_or_default();
                            } else {
                                *b = vec.unwrap_or_default();
                            }
                        }
                    }
                    "lapActive" => t.lap = num.unwrap_or(0.0) as i32,
                    "fRandomness" => t.randomness = num.unwrap_or(1.0),
                    "nMinTriggered" => t.n_min = num.unwrap_or(0.0) as i32,
                    "nMaxTriggered" => t.n_max = num.unwrap_or(0.0) as i32,
                    "nTriggerByOpponent" => t.by_opponent = true,
                    "bPlayerCanTrigger" => t.player_can = num.unwrap_or(1.0) != 0.0,
                    "bTriggerMultipleTimes" => t.multiple = num.unwrap_or(0.0) != 0.0,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    triggers.retain(|t| !t.actions.is_empty());
    (props, triggers)
}

/// The sound triggers of the open city (`script_obj/game/race/global/0x2b7901de/*`):
/// lines across the road under overpasses, gantry signs and in tunnel mouths where a
/// car going past hears the whoosh. Returned as segments (cylinders as short ones).
pub fn whoosh_lines(data_dir: &str) -> Vec<(Vec3, Vec3)> {
    let dir = Path::new(data_dir).join("script_obj/game/race/global/0x2b7901de");
    let mut out: Vec<(Vec3, Vec3)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let Some(sco) = std::fs::read(e.path()).ok().and_then(|d| mcla_formats::sco::decode(&d).ok()) else { continue };
        let (_, triggers) = parse_calls(&sco.calls());
        for t in triggers {
            if !t.actions.iter().any(|a| matches!(a, Action::Sound { object, .. } if !object.starts_with("DISPATCH"))) {
                continue;
            }
            let seg = match t.shape {
                Shape::Line { a, b } => (a, b),
                Shape::Cylinder { c, r, .. } | Shape::Ring { c, r, .. } => (c - Vec3::X * r, c + Vec3::X * r),
                Shape::Timer(_) => continue,
            };
            // the files overlap: one of each
            if !out.iter().any(|o| o.0.distance(seg.0) < 1.0 && o.1.distance(seg.1) < 1.0) {
                out.push(seg);
            }
        }
    }
    out
}

/// Places of the open city where the police dispatcher reports reckless driving (the
/// same global scripts: `Sound` actions with a `DISPATCH_REPORT_VIOLATION_*` call):
/// centre, radius and the call.
pub fn dispatch_zones(data_dir: &str) -> Vec<(Vec3, f32, String)> {
    let dir = Path::new(data_dir).join("script_obj/game/race/global/0x2b7901de");
    let mut out: Vec<(Vec3, f32, String)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let Some(sco) = std::fs::read(e.path()).ok().and_then(|d| mcla_formats::sco::decode(&d).ok()) else { continue };
        let (_, triggers) = parse_calls(&sco.calls());
        for t in triggers {
            let Some(call) = t.actions.iter().find_map(|a| match a {
                Action::Sound { object, .. } if object.starts_with("DISPATCH") => Some(object.clone()),
                _ => None,
            }) else { continue };
            let (c, r) = match t.shape {
                Shape::Line { a, b } => ((a + b) * 0.5, a.distance(b) * 0.5),
                Shape::Cylinder { c, r, .. } | Shape::Ring { c, r, .. } => (c, r),
                Shape::Timer(_) => continue,
            };
            if !out.iter().any(|o| o.0.distance(c) < 1.0 && o.2 == call) {
                out.push((c, r.max(6.0), call));
            }
        }
    }
    out
}

/// The script fills an array with the checkpoint positions (x, y, z, 3, index, 3, base) and
/// right after it one with their radii (value, index, 1, base). The checkpoint array is
/// the one whose first entries are a few hundred metres apart and which is followed by
/// such a list of the same length.
fn checkpoint_radii(sco: &mcla_formats::sco::Sco, first: Option<Vec3>) -> Vec<f32> {
    use mcla_formats::sco::Tok;
    let t = sco.consts();
    let num = |i: usize| if let Some(Tok::Num(v)) = t.get(i) { Some(*v as f32) } else { None };
    let mut i = 0;
    while i + 9 < t.len() {
        // "base, count" of an array, then its first position entry with index 0
        if let (Some(base), Some(n), Some(three), Some(zero), Some(three2), Some(b2)) = (num(i), num(i + 1), num(i + 5), num(i + 6), num(i + 7), num(i + 8)) {
            let n = n as usize;
            // (the start grid is filled the same way, followed by its headings: tell them apart by position)
            let here = match (num(i + 2), num(i + 3), num(i + 4), first) {
                (Some(x), Some(y), Some(z), Some(f)) => Vec3::new(x, y, z).distance(f) < 6.0,
                (_, _, _, None) => true,
                _ => false,
            };
            if here && three == 3.0 && zero == 0.0 && three2 == 3.0 && b2 == base && (2..=80).contains(&n) {
                let after = i + 2 + n * 7;
                // the radii: "base2, n" then n x (radius, k, 1, base2)
                if let (Some(base2), Some(n2)) = (num(after), num(after + 1)) {
                    if n2 as usize == n {
                        let r: Vec<f32> = (0..n).filter_map(|k| match (num(after + 2 + k * 4), num(after + 3 + k * 4), num(after + 4 + k * 4), num(after + 5 + k * 4)) {
                            (Some(v), Some(kk), Some(one), Some(b)) if kk as usize == k && one == 1.0 && b == base2 && (1.0..=200.0).contains(&v) => Some(v),
                            _ => None,
                        }).collect();
                        if r.len() == n {
                            // (the finish line is given as 5 m: a line to cross, taken generously here)
                            return r;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    Vec::new()
}

/// The dressing of the race with this id ("game/Race/Hollywood/Circuit/hw_circuit_ds_02a").
pub fn load(data_dir: &str, race_id: &str, first_checkpoint: Option<Vec3>) -> Option<Dressing> {
    let rel = race_id.to_ascii_lowercase();
    let (folder, name) = rel.rsplit_once('/')?;
    let dir = Path::new(data_dir).join("script_obj").join(folder);
    let script = find(&dir, &format!("{name}.sco"))?;
    let sco = mcla_formats::sco::decode(&std::fs::read(script).ok()?).ok()?;
    let (props, triggers) = parse_calls(&sco.calls());
    // (the easy layout of a race shares the scripted traffic of the race itself)
    let base = name.strip_suffix("_easy").unwrap_or(name);
    let paths = find(&dir, &format!("{name}_ambientdata.xml"))
        .or_else(|| find(&dir, &format!("{name}_AmbientData.xml")))
        .or_else(|| find(&dir, &format!("{base}_ambientdata.xml")))
        .or_else(|| find(&dir, &format!("{base}_AmbientData.xml")))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|x| parse_paths(&x))
        .unwrap_or_default();
    Some(Dressing { cp_radius: checkpoint_radii(&sco, first_checkpoint), props, triggers, paths })
}

// ---------------------------------------------------------------------------------
// The course: checkpoint gates and the opponents' routes

/// One checkpoint as the race's script sets it up.
#[derive(Clone, Debug)]
pub struct CourseCp {
    pub pos: Vec3,
    pub radius: f32,
    /// "Line" checkpoints are a gate from `pos` to this point, crossed by the racing
    /// line at the given fraction of the way
    pub gate: Option<(Vec3, f32)>,
}

impl CourseCp {
    /// Where the road passes the checkpoint (and its marker stands).
    pub fn mark(&self) -> Vec3 {
        match self.gate {
            Some((b, f)) => self.pos + (b - self.pos) * f,
            None => self.pos,
        }
    }
}

/// A stretch the opponents may drive: `count` nodes from `start`, taken with the
/// given probability among the routes of the same stretch (0 = not during the race).
#[derive(Clone, Debug, Default)]
pub struct AiRoute {
    pub start: usize,
    pub count: usize,
    pub weight: f32,
    pub kind: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Course {
    pub cps: Vec<CourseCp>,
    /// position and width of every route node
    pub nodes: Vec<(Vec3, f32)>,
    pub routes: Vec<AiRoute>,
    /// where the police lie in wait along the race (`NumCopZones` of the race table):
    /// position and radius
    pub cop_zones: Vec<(Vec3, f32)>,
}

enum Arr {
    V(Vec<Vec3>),
    N(Vec<f32>),
    S(Vec<String>),
}

impl Arr {
    fn len(&self) -> usize {
        match self {
            Arr::V(v) => v.len(),
            Arr::N(v) => v.len(),
            Arr::S(v) => v.len(),
        }
    }
}

/// The run of tables that starts at token `i` ("base, count", then the entries: vectors
/// as x, y, z, 3, k, 3, base; single values as v, k, 1, base).
fn tables(t: &[mcla_formats::sco::Tok], mut i: usize) -> Vec<Arr> {
    use mcla_formats::sco::Tok;
    let num = |i: usize| if let Some(Tok::Num(v)) = t.get(i) { Some(*v as f32) } else { None };
    let mut out = Vec::new();
    loop {
        let (Some(base), Some(n)) = (num(i), num(i + 1)) else { break };
        let n = n as usize;
        if !(1..=400).contains(&n) {
            break;
        }
        let is_vec = |k: usize| {
            let o = i + 2 + k * 7;
            num(o).is_some() && num(o + 1).is_some() && num(o + 2).is_some() && num(o + 3) == Some(3.0) && num(o + 4) == Some(k as f32) && num(o + 5) == Some(3.0) && num(o + 6) == Some(base)
        };
        let is_one = |k: usize| {
            let o = i + 2 + k * 4;
            t.get(o).is_some() && num(o + 1) == Some(k as f32) && num(o + 2) == Some(1.0) && num(o + 3) == Some(base)
        };
        if (0..n).all(is_vec) {
            out.push(Arr::V((0..n).map(|k| Vec3::new(num(i + 2 + k * 7).unwrap(), num(i + 3 + k * 7).unwrap(), num(i + 4 + k * 7).unwrap())).collect()));
            i += 2 + n * 7;
        } else if (0..n).all(is_one) {
            if matches!(t.get(i + 2), Some(Tok::Str(_))) {
                out.push(Arr::S((0..n).map(|k| if let Some(Tok::Str(s)) = t.get(i + 2 + k * 4) { s.clone() } else { String::new() }).collect()));
            } else {
                out.push(Arr::N((0..n).map(|k| num(i + 2 + k * 4).unwrap_or(0.0)).collect()));
            }
            i += 2 + n * 4;
        } else {
            break;
        }
    }
    out
}

/// Read the checkpoints and opponent routes of a race from its script.
pub fn course(data_dir: &str, race_id: &str, first_checkpoint: Option<Vec3>) -> Option<Course> {
    use mcla_formats::sco::Tok;
    let rel = race_id.to_ascii_lowercase();
    let (folder, name) = rel.rsplit_once('/')?;
    let dir = Path::new(data_dir).join("script_obj").join(folder);
    let script = find(&dir, &format!("{name}.sco"))?;
    let sco = mcla_formats::sco::decode(&std::fs::read(script).ok()?).ok()?;
    let t = sco.consts();
    let num = |i: usize| if let Some(Tok::Num(v)) = t.get(i) { Some(*v as f32) } else { None };
    // the table of checkpoint positions: its first entry is the race's first checkpoint
    // (other tables may begin at the same spot: the right one is followed by the radii)
    let arrs = (0..t.len().saturating_sub(9)).filter(|&i| {
        matches!((num(i + 2), num(i + 3), num(i + 4)), (Some(x), Some(y), Some(z)) if first_checkpoint.map_or(true, |f| Vec3::new(x, y, z).distance(f) < 6.0)) && num(i + 5) == Some(3.0) && num(i + 6) == Some(0.0) && num(i + 7) == Some(3.0) && num(i + 8) == num(i)
    }).map(|at| tables(&t, at)).find(|a| {
        // positions, their radii, and (a few tables on) the checkpoint kinds
        matches!((a.first(), a.get(1)), (Some(Arr::V(p)), Some(Arr::N(r))) if p.len() == r.len()) && (first_checkpoint.is_some() || a.iter().take(12).any(|x| matches!(x, Arr::S(s) if s.len() == a[0].len() && s.iter().all(|k| k == "Normal" || k == "Line"))))
    })?;
    let Some(Arr::V(pos)) = arrs.first() else { return None };
    let n = pos.len();
    let Some(Arr::N(radius)) = arrs.get(1).filter(|a| a.len() == n) else { return None };
    // the gates: the other ends, how far along the road crosses, and which checkpoints are lines
    let kinds = arrs.iter().skip(2).find_map(|a| if let Arr::S(s) = a { Some(s) } else { None }).filter(|s| s.len() == n);
    let ends_at = arrs.iter().enumerate().skip(2).find(|(_, a)| matches!(a, Arr::V(_)) && a.len() == n).map(|(i, _)| i);
    let mut cps: Vec<CourseCp> = (0..n).map(|k| CourseCp { pos: pos[k], radius: radius[k], gate: None }).collect();
    if let (Some(kinds), Some(ei)) = (kinds, ends_at) {
        if let (Some(Arr::V(ends)), Some(Arr::N(frac))) = (arrs.get(ei), arrs.get(ei + 1)) {
            if frac.len() == n {
                for k in 0..n {
                    if kinds[k].eq_ignore_ascii_case("Line") && ends[k].length() > 1.0 && ends[k].distance(pos[k]) > 4.0 {
                        cps[k].gate = Some((ends[k], frac[k].clamp(0.0, 1.0)));
                    }
                }
            }
        }
    }
    // route nodes: the first table of positions after the checkpoint tables, with their widths
    let mut nodes: Vec<(Vec3, f32)> = Vec::new();
    let mut cop_zones: Vec<(Vec3, f32)> = Vec::new();
    let after = arrs.iter().position(|a| matches!(a, Arr::S(_))).unwrap_or(1) + 1;
    for i in after..arrs.len() {
        if let (Arr::V(p), Some(Arr::N(r))) = (&arrs[i], arrs.get(i + 1)) {
            if r.len() == p.len() && ends_at != Some(i) {
                nodes = p.iter().zip(r.iter()).map(|(p, r)| (*p, *r)).collect();
                // after the nodes (positions, widths, flags): the cop zones, radii first
                if let (Some(Arr::N(zr)), Some(Arr::V(zp))) = (arrs.get(i + 3), arrs.get(i + 4)) {
                    if zr.len() == zp.len() && zr.len() <= 16 {
                        cop_zones = zp.iter().zip(zr.iter()).map(|(p, r)| (*p, *r)).collect();
                    }
                }
                break;
            }
        }
    }
    // the routes are set up one call per value: (brain, route, value)
    let mut routes: Vec<AiRoute> = Vec::new();
    for c in sco.calls() {
        if !matches!(c.hash, 0x9e325f69 | 0x1555cd90 | 0x13eafb43 | 0x7bc98615) || c.args.len() != 3 {
            continue;
        }
        let (Some(r), Some(v)) = (c.args[1].num(), c.args[2].num()) else { continue };
        let r = r as usize;
        if r >= 512 {
            continue;
        }
        if routes.len() <= r {
            routes.resize(r + 1, AiRoute::default());
        }
        match c.hash {
            0x9e325f69 => routes[r].start = v as usize,
            0x1555cd90 => routes[r].count = v as usize,
            0x13eafb43 => routes[r].weight = v as f32,
            _ => routes[r].kind = v as i32,
        }
    }
    routes.retain(|r| r.count > 0 && r.start + r.count <= nodes.len());
    Some(Course { cps, nodes, routes, cop_zones })
}

fn seg_dist(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let (px, pz, ax, az, bx, bz) = (p.x, p.z, a.x, a.z, b.x, b.z);
    let (dx, dz) = (bx - ax, bz - az);
    let t = (((px - ax) * dx + (pz - az) * dz) / (dx * dx + dz * dz).max(1e-3)).clamp(0.0, 1.0);
    ((px - ax - dx * t).powi(2) + (pz - az - dz * t).powi(2)).sqrt()
}

impl Course {
    /// One opponent's choice of routes: for every leg (the stretch after checkpoint k)
    /// the nodes to drive through. `rnd` picks among alternatives by their weights;
    /// without it the likeliest route of each stretch is taken.
    pub fn pick(&self, circuit: bool, mut rnd: Option<&mut dyn FnMut() -> f32>) -> Vec<Vec<(Vec3, f32)>> {
        let n = self.cps.len();
        let marks: Vec<Vec3> = self.cps.iter().map(|c| c.mark()).collect();
        let legs = if circuit { n } else { n.saturating_sub(1) };
        let mut per_leg: Vec<Vec<(Vec3, f32)>> = vec![Vec::new(); n];
        if legs == 0 {
            return per_leg;
        }
        let leg_of = |p: Vec3| (0..legs).min_by(|&a, &b| seg_dist(p, marks[a], marks[(a + 1) % n]).partial_cmp(&seg_dist(p, marks[b], marks[(b + 1) % n])).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0);
        // which legs each route covers
        let mut spans: Vec<(usize, usize, usize, Vec<usize>)> = Vec::new();
        for (ri, r) in self.routes.iter().enumerate() {
            if r.weight <= 0.0 {
                continue;
            }
            let mut l: Vec<usize> = (r.start..r.start + r.count).map(|i| leg_of(self.nodes[i].0)).collect();
            for j in 1..l.len() {
                if l[j] < l[j - 1] {
                    l[j] = l[j - 1];
                }
            }
            spans.push((l[0], *l.last().unwrap(), ri, l));
        }
        spans.sort_by_key(|s| (s.0, s.2));
        let mut i = 0;
        while i < spans.len() {
            // routes of the same stretch are alternatives
            let mut j = i + 1;
            let mut hi = spans[i].1;
            while j < spans.len() && spans[j].0 <= hi {
                hi = hi.max(spans[j].1);
                j += 1;
            }
            let group = &spans[i..j];
            let sum: f32 = group.iter().map(|s| self.routes[s.2].weight).sum();
            let chosen = match rnd.as_mut() {
                // self-test: everybody takes the rarest route
                Some(_) if std::env::var("MCLA_SHORTCUTS").map_or(false, |v| v == "2") => group.iter().min_by(|a, b| self.routes[a.2].weight.partial_cmp(&self.routes[b.2].weight).unwrap_or(std::cmp::Ordering::Equal)),
                Some(f) => {
                    let mut x = f() * sum.max(1.0);
                    group.iter().find(|s| {
                        x -= self.routes[s.2].weight;
                        x < 0.0
                    })
                }
                None => group.iter().max_by(|a, b| self.routes[a.2].weight.partial_cmp(&self.routes[b.2].weight).unwrap_or(std::cmp::Ordering::Equal)),
            };
            if let Some(s) = chosen {
                let r = &self.routes[s.2];
                for (k, ni) in (r.start..r.start + r.count).enumerate() {
                    per_leg[s.3[k]].push(self.nodes[ni]);
                }
            }
            i = j;
        }
        per_leg
    }
}

// ---------------------------------------------------------------------------------
// Running it

use crate::render::Gpu;
use crate::signals::Signals;
use crate::vehicles::parked::{ItemPath, Mwc};
use crate::vehicles::traffic::{self, PathRails, PathSpawn};
use crate::vehicles::Vehicles;

/// Tag (in `CarInstance::driver`) the port's own earlier rule gave the race scripts' cars; no
/// car carries it now (the paths' cars are layout items, `CarInstance::item`).
pub const TAG: &str = "amb:";

/// A trigger's events chosen at its reset (trigger vfunc 13 sub_82685B30, from Reset sub_826854B8
/// for every trigger, TriggerManager_ResetTriggers): the bounds clamped (min >= count -> 0;
/// max >= count -> count; min < 0 with max >= 0 -> min 0; max < 0 with min >= 0 -> max count);
/// one draw of the shared MWC, u = (lo & 0x7FFFFF) / 2^23, u >= fRandomness disarms it (None);
/// both bounds negative: every event; else n = min + (lo & 0x7FFFFFFF) % (max - min + 1) on a
/// second draw, n = count: every event, else the first index r % count, each next
/// (previous + 1 + r % (count - 1)) % count, kept when not in yet, until n are in.
pub fn select(rng: &mut Mwc, count: usize, n_min: i32, n_max: i32, randomness: f32) -> Option<Vec<usize>> {
    let c = count as i32;
    let (mut lo, mut hi) = (n_min, n_max);
    if lo < 0 && hi >= 0 {
        lo = 0;
    } else if lo >= c {
        lo = 0;
    }
    if hi < 0 && lo >= 0 {
        hi = c;
    } else if hi >= c {
        hi = c;
    }
    let u = rng.float();
    if !(u < randomness) {
        return None;
    }
    if lo < 0 && hi < 0 {
        return Some((0..count).collect());
    }
    let span = hi - lo + 1;
    let r = (rng.next() & 0x7FFF_FFFF) as i32;
    // (a zero span traps in the original)
    let n = lo + if span != 0 { r % span } else { 0 };
    if n == c {
        return Some((0..count).collect());
    }
    let mut out: Vec<usize> = Vec::new();
    let mut prev: i32 = -1;
    // (n over the count would never end in the original; the bounds keep it within)
    while (out.len() as i32) < n.min(c) {
        let r = (rng.next() & 0x7FFF_FFFF) as i32;
        let idx = if prev < 0 {
            r % c
        } else if c > 1 {
            (r % (c - 1) + prev + 1) % c
        } else {
            0
        };
        prev = idx;
        if !out.contains(&(idx as usize)) {
            out.push(idx as usize);
        }
    }
    Some(out)
}

pub struct Runtime {
    pub d: Dressing,
    fired: Vec<f32>,
    /// each trigger's chosen events (trigger +40); None: disarmed (state 0)
    chosen: Vec<Option<Vec<usize>>>,
    prev: Vec<(usize, Vec3)>,
    /// the ambient updates since the race's FullReset (S +0x6423C): the triggers are reset at
    /// the first, the paths become layout items at the second (sub_826D60A0)
    count: u32,
    /// each path after its init, and its layout item (+184)
    rails: Vec<Option<PathRails>>,
    items: Vec<Option<u32>>,
    /// followers asked for and waiting for the ground (the record's 3 s, sub_826E0438): item,
    /// colour, age
    pending: Vec<(u32, usize, f32)>,
    /// counters for the log
    pub started: u32,
    pub lights: u32,
    pub boosts: u32,
    pub roads: u32,
    /// police radio calls set off this frame
    pub radio: Vec<String>,
}

impl Runtime {
    pub fn new(d: Dressing, _seed: u32) -> Runtime {
        let (nt, np) = (d.triggers.len(), d.paths.len());
        Runtime { d, fired: vec![-1.0; nt], chosen: vec![None; nt], prev: Vec::new(), count: 0, rails: vec![None; np], items: vec![None; np], pending: Vec::new(), started: 0, lights: 0, boosts: 0, roads: 0, radio: Vec::new() }
    }

    /// Vehicle types the paths ask for (to have their models in memory).
    pub fn types(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for p in &self.d.paths {
            if !p.type_name.is_empty() && !v.contains(&p.type_name) {
                v.push(p.type_name.clone());
            }
        }
        v
    }

    /// Road flares of the event (they burn).
    pub fn flares(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.d.props.iter().filter(|p| p.0.contains("flare")).map(|p| p.1)
    }

    /// The path of item `h`.
    fn path_of(&self, h: u32) -> Option<usize> {
        self.items.iter().position(|x| *x == Some(h))
    }

    /// TriggerManager_ResetTriggers: every trigger chooses its events.
    fn reset_triggers(&mut self, vehicles: &mut Vehicles) {
        for (ti, t) in self.d.triggers.iter().enumerate() {
            self.chosen[ti] = select(vehicles.shared_mwc(), t.actions.len(), t.n_min, t.n_max, t.randomness);
            if std::env::var_os("MCLA_RACEPATH_LOG").is_some() {
                log::info!("race trigger {ti}: {} events, randomness {} min {} max {} -> {:?}", t.actions.len(), t.randomness, t.n_min, t.n_max, self.chosen[ti]);
            }
        }
    }

    /// sub_826D60A0 (once, at the second ambient update): each path's init (its vertices onto
    /// the lanes, its rails), then its layout item unless an "Ambient" event points at it and
    /// none of those events' triggers chose it - type bRandomSmallAmbient: a draw of bound group
    /// 0, taxis too; bRandomLargeAmbient: group 1, no taxis; else TypeName; at the first snapped
    /// vertex, heading the spawn matrix's.
    fn make_items(&mut self, vehicles: &mut Vehicles, world: &crate::world::World, focus: Vec3) -> bool {
        let Some(nav) = world.nav.as_ref().map(traffic::rail_nav) else { return false };
        let root = world.assets.root.clone();
        let (mut made, mut most) = (0, 0);
        for i in 0..self.d.paths.len() {
            let p = self.d.paths[i].clone();
            let ty = (!p.type_name.is_empty()).then(|| traffic::AmbType::load(&root, &p.type_name));
            let mut pr = traffic::path_rails(nav, &p.verts, &p.snap, p.parked, ty.map(|t| t.min_turn), p.pos, p.row2, p.speed[1]);
            // bPhysical (path +170): instance +140 bit 0x80 for the car's life (sub_826FA588)
            pr.physical = p.physical;
            most = most.max(pr.hits);
            let mut referenced = false;
            let mut chosen = false;
            'trig: for (ti, t) in self.d.triggers.iter().enumerate() {
                for (ei, a) in t.actions.iter().enumerate() {
                    if matches!(a, Action::Ambient(k) if *k == i) {
                        referenced = true;
                        if self.chosen[ti].as_ref().is_some_and(|c| c.contains(&ei)) {
                            chosen = true;
                            break 'trig;
                        }
                    }
                }
            }
            if !referenced || chosen {
                let name = if p.random_small {
                    vehicles.path_type_draw(&root, focus, 0, true)
                } else if p.random_large {
                    vehicles.path_type_draw(&root, focus, 1, false)
                } else if !p.type_name.is_empty() {
                    Some(p.type_name.clone())
                } else {
                    None
                };
                if name.is_none() && std::env::var_os("MCLA_RACEPATH_LOG").is_some() {
                    log::info!("race path {i}: no type (small {} large {} TypeName '{}')", p.random_small, p.random_large, p.type_name);
                }
                if let Some(name) = name {
                    let path = ItemPath { index: i, parked: p.parked, referenced, fired: false, lane: pr.lane.is_some(), spawn_when_triggered: p.spawn_when_triggered };
                    self.items[i] = vehicles.add_layout_item(&name, pr.spawn, pr.heading(), Some(path));
                    made += self.items[i].is_some() as u32;
                    if std::env::var_os("MCLA_RACEPATH_LOG").is_some() {
                        log::info!("race path {i}: item {:?} {name} at {:?} parked {} referenced {} lane {:?} rails curve {} join {} start/max {}/{} mph", self.items[i], pr.spawn, p.parked, referenced, pr.lane, pr.curve.is_some(), pr.join.is_some(), p.speed[0], p.speed[1]);
                    }
                }
            } else if std::env::var_os("MCLA_RACEPATH_LOG").is_some() {
                log::info!("race path {i}: no item (referenced, not chosen)");
            }
            self.rails[i] = Some(pr);
        }
        log::info!("race paths: {} of {} made layout items (most lane answers at a vertex {most})", made, self.d.paths.len());
        true
    }

    /// The followers the parked spawner's item branch asked for (parked.rs `item_followers_due`)
    /// and those waiting for the ground.
    fn followers(&mut self, vehicles: &mut Vehicles, gpu: &Gpu, world: &crate::world::World, dt: f32) {
        for (h, shade) in std::mem::take(&mut vehicles.item_followers_due) {
            let Some(i) = self.path_of(h) else { continue };
            // (the spawner's item branch, sub_826F23A0, passes over an item whose path a trigger
            // points at once that has fired, or always with bSpawnWhenTriggered: such a path's car
            // comes only from its trigger - the guard here until parked.rs reads it)
            let ip = vehicles.layout.get(h as usize).and_then(|x| x.as_ref()).and_then(|x| x.path);
            if ip.is_some_and(|x| x.referenced && (x.fired || self.d.paths[i].spawn_when_triggered)) {
                continue;
            }
            if !self.pending.iter().any(|p| p.0 == h) && !vehicles.cars.iter().any(|c| c.item == Some(h)) {
                self.pending.push((h, shade, 0.0));
            }
        }
        let mut keep = Vec::new();
        for (h, shade, age) in std::mem::take(&mut self.pending) {
            let Some(i) = self.path_of(h) else { continue };
            let Some(pr) = self.rails[i].clone() else { continue };
            let p = &self.d.paths[i];
            let fired = vehicles.layout.get(h as usize).and_then(|x| x.as_ref()).and_then(|x| x.path).is_some_and(|x| x.fired);
            let referenced = vehicles.layout.get(h as usize).and_then(|x| x.as_ref()).and_then(|x| x.path).is_some_and(|x| x.referenced);
            let standing = p.parked || (referenced && !fired);
            if vehicles.spawn_path_car(gpu, world, h, shade, &pr, i, p.speed[0], standing) == PathSpawn::NoGround && age + dt <= 3.0 {
                keep.push((h, shade, age + dt));
            }
        }
        self.pending = keep;
        vehicles.item_pending = self.pending.iter().map(|p| p.0).collect();
    }

    /// A trigger's "Ambient" event (sub_82687F58).
    fn fire_path(&mut self, vehicles: &mut Vehicles, gpu: &Gpu, world: &crate::world::World, i: usize) {
        let (Some(Some(h)), Some(Some(pr))) = (self.items.get(i).copied(), self.rails.get(i).cloned()) else { return };
        self.started += 1;
        let pending = self.pending.iter().any(|p| p.0 == h);
        let p = &self.d.paths[i];
        if let Some((shade, PathSpawn::NoGround)) = vehicles.fire_path(gpu, world, h, &pr, i, p.speed[0], p.parked, pending) {
            self.pending.push((h, shade, 0.0));
        }
    }

    /// Race_UnloadAmbientPathArray (the core's teardown): the paths' items removed; cars riding
    /// lanes go with them, the others stay without their item.
    pub fn unload(self, vehicles: &mut Vehicles) {
        let handles: Vec<u32> = self.items.iter().flatten().copied().collect();
        vehicles.unload_path_items(&handles);
    }

    /// One frame of a running race. `time` is negative during the countdown;
    /// `subjects` are the racers (car indices), the player first.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, vehicles: &mut Vehicles, gpu: &Gpu, world: &crate::world::World, other_signals: &mut Signals, subjects: &[usize], time: f32, lap: i32, dt: f32) {
        let Some(&pi) = subjects.first() else { return };
        let Some(pp) = vehicles.cars.get(pi).map(|c| c.state.pos) else { return };
        // the triggers' reset, then (the ambient counter at 2) the paths' items
        if self.count == 0 {
            self.reset_triggers(vehicles);
            self.count = 1;
        } else if self.count == 1 && self.make_items(vehicles, world, pp) {
            self.count = 2;
        }
        self.followers(vehicles, gpu, world, dt);
        // triggers
        let mut fire: Vec<(usize, usize)> = Vec::new();
        for (ti, t) in self.d.triggers.iter().enumerate() {
            if self.chosen[ti].is_none() {
                continue;
            }
            if self.fired[ti] >= 0.0 && (!t.multiple || time - self.fired[ti] < 4.0) {
                continue;
            }
            if t.lap != 0 && t.lap != lap {
                continue;
            }
            let nitro = t.actions.iter().any(|a| matches!(a, Action::Nitro));
            for (k, &ci) in subjects.iter().enumerate() {
                if (k == 0 && !t.player_can) || (k > 0 && !(t.by_opponent || nitro)) {
                    continue;
                }
                let Some(c) = vehicles.cars.get(ci) else { continue };
                let p = c.state.pos;
                let hit = match &t.shape {
                    Shape::Timer(at) => k == 0 && time >= *at,
                    Shape::Cylinder { c: o, r, h } => (p.y - o.y).abs() < *h && Vec3::new(p.x - o.x, 0.0, p.z - o.z).length() < *r,
                    Shape::Ring { c: o, r, inner, h, min_speed, max_speed } => {
                        let v = c.state.speed() / 0.44704;
                        let f = ((v - min_speed) / (max_speed - min_speed).max(1.0)).clamp(0.0, 1.0);
                        (p.y - o.y).abs() < *h && Vec3::new(p.x - o.x, 0.0, p.z - o.z).length() < inner + (r - inner) * f
                    }
                    Shape::Line { a, b } => {
                        let q = self.prev.iter().find(|e| e.0 == ci).map(|e| e.1).unwrap_or(p);
                        let cross = |o: Vec3, u: Vec3, w: Vec3| (u.x - o.x) * (w.z - o.z) - (u.z - o.z) * (w.x - o.x);
                        (p.y - a.y).abs() < 10.0 && cross(*a, *b, q) * cross(*a, *b, p) < 0.0 && cross(q, p, *a) * cross(q, p, *b) < 0.0
                    }
                };
                if hit {
                    fire.push((ti, ci));
                    break;
                }
            }
        }
        self.prev = subjects.iter().filter_map(|&ci| vehicles.cars.get(ci).map(|c| (ci, c.state.pos))).collect();
        for (ti, ci) in fire {
            self.fired[ti] = time.max(0.0);
            let t = self.d.triggers[ti].clone();
            // the events the trigger chose at its reset
            let chosen = self.chosen[ti].clone().unwrap_or_default();
            for &ei in &chosen {
                let Some(a) = t.actions.get(ei) else { continue };
                match a {
                    Action::Ambient(i) => self.fire_path(vehicles, gpu, world, *i),
                    Action::Intersection { center, toward, green } => {
                        let a1 = vehicles.signals.force(*center, *toward - *center, *green);
                        let a2 = other_signals.force(*center, *toward - *center, *green);
                        if a1 || a2 {
                            self.lights += 1;
                        }
                    }
                    Action::Sound { object, .. } if object.starts_with("DISPATCH") => self.radio.push(object.clone()),
                    Action::RoadDensity { center, density } => {
                        vehicles.road_density_new.push((*center, *density));
                        self.roads += 1;
                    }
                    Action::Nitro => {
                        if let Some(c) = vehicles.cars.get_mut(ci) {
                            if !c.is_player {
                                c.boost = 2.5;
                                self.boosts += 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sub_82685B30: the bounds, the disarming draw, the count draw and the picks.
    #[test]
    fn trigger_selection_rule() {
        // fRandomness 1.0 never disarms; 0.0 always does (u >= 0)
        let mut m = Mwc { lo: 12345, carry: 7 };
        assert!(select(&mut m, 4, -1, -1, 0.0).is_none());
        // both bounds negative: every event, one draw only
        let mut m = Mwc { lo: 12345, carry: 7 };
        let mut m2 = m;
        assert_eq!(select(&mut m, 5, -1, -1, 1.0), Some(vec![0, 1, 2, 3, 4]));
        m2.next();
        assert_eq!((m.lo, m.carry), (m2.lo, m2.carry));
        // min 0, max 0: no event chosen (the trigger still fires, with nothing to do)
        let mut m = Mwc { lo: 99, carry: 3 };
        assert_eq!(select(&mut m, 5, 0, 0, 1.0), Some(vec![]));
        // min = max: exactly that many; a min at or over the count is 0
        let mut m = Mwc { lo: 99, carry: 3 };
        assert_eq!(select(&mut m, 4, 2, 2, 1.0).map(|v| v.len()), Some(2));
        let mut m = Mwc { lo: 99, carry: 3 };
        assert_eq!(select(&mut m, 4, 4, 0, 1.0).map(|v| v.len()), Some(0));
        // the picks by hand: n = 1 + r % 2, first r % 6, then (prev + 1 + r % 5) % 6
        for seed in 1..200u32 {
            let mut m = Mwc { lo: seed.wrapping_mul(2654435761), carry: seed };
            let mut e = m;
            let got = select(&mut m, 6, 1, 2, 1.0).unwrap();
            let u = e.float();
            assert!(u < 1.0);
            let n = 1 + ((e.next() & 0x7FFF_FFFF) % 2) as usize;
            let mut want: Vec<usize> = Vec::new();
            let mut prev: Option<usize> = None;
            while want.len() < n {
                let r = (e.next() & 0x7FFF_FFFF) as usize;
                let i = match prev {
                    None => r % 6,
                    Some(p) => (p + 1 + r % 5) % 6,
                };
                prev = Some(i);
                if !want.contains(&i) {
                    want.push(i);
                }
            }
            assert_eq!(got, want);
            assert_eq!((m.lo, m.carry), (e.lo, e.carry));
            // the second pick never repeats the first (it skips it by construction)
            if got.len() == 2 {
                assert_ne!(got[0], got[1]);
            }
        }
        // a negative min with a set max: the min is 0
        let mut m = Mwc { lo: 5, carry: 1 };
        let mut e = m;
        let got = select(&mut m, 4, -1, 1, 1.0).unwrap();
        e.float();
        let n = ((e.next() & 0x7FFF_FFFF) % 2) as usize;
        assert_eq!(got.len(), n);
    }

    /// The curve rail (sub_827383B8 / sub_825867F8 / sub_82587A78): clamped uniform knots, degree
    /// min(3, k - 1), through the first and last control points.
    #[test]
    fn path_curve_rail() {
        let pts: Vec<Vec3> = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 5.0), Vec3::new(30.0, 1.0, 15.0), Vec3::new(35.0, 1.0, 30.0)];
        let c = traffic::path_curve(&pts);
        assert_eq!(c.degree, 3);
        assert_eq!(c.knots, vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0, 2.0]);
        assert_eq!(c.t_range, 2.0);
        assert!((c.t_max - (2.0 - 1e-4)).abs() < 1e-6);
        assert!(c.eval(0.0).0.distance(pts[0]) < 1e-4);
        assert!(c.eval(1.0).0.distance(pts[4]) < 1e-2);
        // the start's tangent along the first leg
        let d = c.eval(0.0).1.normalize();
        assert!(d.dot(Vec3::X) > 0.999);
        // three points: degree 2, one span, knots 0 0 0 1 1 1
        let c3 = traffic::path_curve(&pts[..3]);
        assert_eq!(c3.degree, 2);
        assert_eq!(c3.knots, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        assert_eq!(c3.t_range, 1.0);
        let mid = c3.eval(0.5).0;
        // the quadratic's middle: (P0 + 2 P1 + P2) / 4
        assert!(mid.distance((pts[0] + pts[1] * 2.0 + pts[2]) / 4.0) < 1e-4);
        // four points: degree 3, knots 0 0 0 0 1 1 1 1
        let c4 = traffic::path_curve(&pts[..4]);
        assert_eq!(c4.knots, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
        // two points: the line
        let c2 = traffic::path_curve(&pts[..2]);
        assert_eq!(c2.degree, 1);
        assert_eq!(c2.knots, vec![0.0, 0.0, 1.0, 1.0]);
        assert!(c2.eval(0.25).0.distance(Vec3::new(2.5, 0.0, 0.0)) < 1e-4);
        // the nearest point (sub_825875F0) on the cubic finds a point placed on it
        let q = c.eval(0.37).0 + Vec3::new(0.0, 2.0, 0.0);
        let (t, d2, p) = traffic::curve_nearest(&c, q, 4, 10);
        assert!((t - 0.37).abs() < 0.02, "t {t}");
        assert!((d2 - 4.0).abs() < 0.2 && (p.distance_squared(q) - d2).abs() < 1e-3);
    }
}
