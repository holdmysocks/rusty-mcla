//! Static world collision: triangle soup per sector in a uniform XZ grid.
//! Supports vertical/arbitrary raycasts and sphere-vs-triangle queries, which is
//! all the arcade car model needs.

use glam::Vec3;
use hashbrown::HashMap;

const CELL: f32 = 8.0;

/// What the ground is made of (the game's physics materials, `tune/physics/materials`:
/// each with its own tyre grip and drag).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Surface {
    Road = 0,
    Grass = 1,
    Gravel = 2,
    Sand = 3,
    River = 4,
    Sidewalk = 5,
    Dirt = 6,
    Alley = 7,
    Wood = 8,
    Brick = 9,
    Water = 10,
    WaterDeep = 11,
    Railroad = 12,
    Freeway = 13,
    Interior = 14,
    Exterior = 15,
    Tunnel = 16,
    Stairs = 17,
    Metal = 18,
    Cobblestone = 19,
    Mall = 20,
    Wall = 21,
    // (the rest of `tune/physics/materials/materials.list`: what the city's own collision names)
    CarRoof = 22,
    CarSide = 23,
    CarUndercarriage = 24,
    CarWheel = 25,
    Death = 26,
    Default = 27,
    ExteriorSmall = 28,
    Ice = 29,
    InteriorGrass = 30,
    InteriorRoad = 31,
    InteriorSidewalk = 32,
    InteriorSmall = 33,
    JumpDown = 34,
    JumpUp = 35,
    Manhole = 36,
    MetalCorrugated = 37,
    MetalGrate = 38,
    Pothole = 39,
    Prop = 40,
    Puddles = 41,
    Ragdoll = 42,
    Reflector = 43,
    RoadBumpy = 44,
    SandInland = 45,
    Sewer = 46,
    ShortcutGrass = 47,
    ShortcutRoad = 48,
    ShortcutSidewalk = 49,
    Shrub = 50,
    Snow = 51,
    SnowReverb = 52,
    Vertex = 53,
    WallBush = 54,
    RiverDeep = 55,
    PedOnlyWalk = 56,
    PedOnlyStand = 57,
    PropPushSphere = 58,
    PedRoad = 59,
    PedShortcut = 60,
    WallWood = 61,
    WallSoundPass = 62,
}

impl Surface {
    pub const COUNT: usize = 63;
    pub const ALL: [Surface; Surface::COUNT] = [
        Surface::Road, Surface::Grass, Surface::Gravel, Surface::Sand, Surface::River, Surface::Sidewalk, Surface::Dirt, Surface::Alley, Surface::Wood, Surface::Brick, Surface::Water,
        Surface::WaterDeep, Surface::Railroad, Surface::Freeway, Surface::Interior, Surface::Exterior, Surface::Tunnel, Surface::Stairs, Surface::Metal, Surface::Cobblestone, Surface::Mall, Surface::Wall,
        Surface::CarRoof, Surface::CarSide, Surface::CarUndercarriage, Surface::CarWheel, Surface::Death, Surface::Default, Surface::ExteriorSmall, Surface::Ice, Surface::InteriorGrass,
        Surface::InteriorRoad, Surface::InteriorSidewalk, Surface::InteriorSmall, Surface::JumpDown, Surface::JumpUp, Surface::Manhole, Surface::MetalCorrugated, Surface::MetalGrate,
        Surface::Pothole, Surface::Prop, Surface::Puddles, Surface::Ragdoll, Surface::Reflector, Surface::RoadBumpy, Surface::SandInland, Surface::Sewer, Surface::ShortcutGrass,
        Surface::ShortcutRoad, Surface::ShortcutSidewalk, Surface::Shrub, Surface::Snow, Surface::SnowReverb, Surface::Vertex, Surface::WallBush, Surface::RiverDeep, Surface::PedOnlyWalk,
        Surface::PedOnlyStand, Surface::PropPushSphere, Surface::PedRoad, Surface::PedShortcut, Surface::WallWood, Surface::WallSoundPass,
    ];
    const NAMES: [&'static str; Surface::COUNT] = [
        "road", "grass", "gravel", "sandBeach", "river", "sidewalk", "dirt", "alley", "wood", "brick", "water", "water_deep", "railroad", "fwy", "interior", "exterior", "roadTunnel", "stairs", "metal",
        "cobblestone", "interior_mall", "wall", "car_roof", "car_side", "car_undercarriage", "car_wheel", "death", "default", "exterior_small", "ice", "interior_grass", "interior_road",
        "interior_sidewalk", "interior_small", "jump_down", "jump_up", "manhole", "metal_corrugated", "metal_grate", "pothole", "prop", "puddles", "ragdoll", "reflector", "road_bumpy", "sand",
        "sewer", "shortcut_grass", "shortcut_road", "shortcut_sidewalk", "shrub", "snow", "snow_reverb", "vertex", "wall_bush", "riverdeep", "pedOnlyWalk", "pedOnlyStand", "prop_push_sphere",
        "pedRoad", "pedShortcut", "wallWood", "wallSoundPass",
    ];
    /// name of the material file
    pub fn material(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn from_u8(v: u8) -> Surface {
        Self::ALL[(v as usize).min(Surface::COUNT - 1)]
    }
    /// The material of that name in the game's list.
    pub fn from_name(name: &str) -> Option<Surface> {
        Self::NAMES.iter().position(|n| n.eq_ignore_ascii_case(name)).map(|i| Self::ALL[i])
    }
    /// What it is like to drive on, for the things that only tell a handful of grounds apart
    /// (dust, spray, the tyres' sound): the short cuts, interiors and odd patches by their kind.
    pub fn kind(self) -> Surface {
        match self {
            Surface::SandInland => Surface::Sand,
            Surface::InteriorGrass | Surface::ShortcutGrass | Surface::Shrub => Surface::Grass,
            Surface::InteriorRoad | Surface::ShortcutRoad | Surface::RoadBumpy | Surface::Pothole | Surface::Sewer | Surface::Manhole | Surface::Reflector | Surface::Puddles | Surface::Default => Surface::Road,
            Surface::InteriorSidewalk | Surface::ShortcutSidewalk => Surface::Sidewalk,
            Surface::RiverDeep => Surface::River,
            Surface::MetalCorrugated | Surface::MetalGrate => Surface::Metal,
            Surface::WallBush | Surface::WallWood | Surface::WallSoundPass => Surface::Wall,
            Surface::ExteriorSmall => Surface::Exterior,
            Surface::InteriorSmall => Surface::Interior,
            s => s,
        }
    }
    /// The materials in the order of the game's table, which the city's collision counts in:
    /// "default" first, then `materials.list` without its "default" line.
    pub fn table(data_dir: &str) -> Vec<Surface> {
        let list = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/physics/materials/materials.list")).unwrap_or_default();
        let mut out = vec![Surface::Default];
        out.extend(list.lines().map(str::trim).filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("default")).map(|n| Surface::from_name(n).unwrap_or(Surface::Default)));
        out
    }
    /// From the shader and texture the ground is drawn with (when it names no art material).
    pub fn classify(shader: &str, texture: &str) -> Surface {
        let (s, t) = (shader.to_ascii_lowercase(), texture.to_ascii_lowercase());
        if s.contains("riverroad") || t.contains("river") {
            Surface::River
        } else if t.contains("sand") {
            Surface::Sand
        } else if t.contains("gravel") {
            Surface::Gravel
        } else if t.contains("dirt") || t.contains("mud") {
            Surface::Dirt
        } else if s == "citygrass" || s == "cityterrain" || t.contains("grass") {
            Surface::Grass
        } else if t.contains("sidewalk") {
            Surface::Sidewalk
        } else {
            Surface::Road
        }
    }
    /// From the art material a city shader was exported from - its `.mtlgeo` path, e.g.
    /// `t:/mc4/art/city/sc/beach/santa_monica_beach/mtl/ground/exterior/l_be_sb_ground_sand_01.mtlgeo`.
    /// The file's name says what the ground is (the first such word counts: "sand_trans_asphalt"
    /// is sand running out into asphalt), its folder what kind of thing it belongs to.
    pub fn from_material(path: &str, shader: &str, texture: &str) -> Surface {
        let p = path.to_ascii_lowercase().replace('\\', "/");
        if p.is_empty() {
            return Self::classify(shader, texture);
        }
        let folder = p.find("/mtl/").map_or("", |i| &p[i + 5..]);
        let name = p.rsplit('/').next().unwrap_or("").trim_end_matches(".mtlgeo");
        let ground = folder.starts_with("ground/") || folder.starts_with("road/") || folder.starts_with("organic/");
        // (materials filed under no kind - a few shared ones and the "sc_temp" exports - go by
        // their name alone)
        let filed = folder.contains('/');
        if filed && !ground {
            // walls, roofs, signs, fences, barriers: nothing a tyre holds on to (flat parts of
            // them are taken for plain ground by the caller)
            return Surface::Wall;
        }
        if !filed && ["wall", "bldg", "door", "fence", "shut", "roof", "sign", "window", "inst_", "extents", "geom"].iter().any(|w| name.contains(w)) {
            return Surface::Wall;
        }
        if name.contains("road_river") || name.contains("riverbasin") {
            return Surface::River;
        }
        const WORDS: [(&str, Surface); 34] = [
            ("water_ocean", Surface::WaterDeep),
            ("underwater", Surface::WaterDeep),
            ("water", Surface::Water),
            ("sand", Surface::Sand),
            ("gravel", Surface::Gravel),
            ("traintrack", Surface::Railroad),
            ("railyard", Surface::Railroad),
            ("dirt", Surface::Dirt),
            ("cliff", Surface::Dirt),
            ("terrain", Surface::Dirt),
            ("pallisades", Surface::Dirt),
            ("grass", Surface::Grass),
            ("groundcover", Surface::Grass),
            ("planter", Surface::Grass),
            ("treehole", Surface::Dirt),
            ("wood", Surface::Wood),
            ("brick", Surface::Brick),
            ("cobble", Surface::Cobblestone),
            ("alley", Surface::Alley),
            ("sidewalk", Surface::Sidewalk),
            ("promenade", Surface::Sidewalk),
            ("stair", Surface::Stairs),
            ("asphalt", Surface::Road),
            ("asphault", Surface::Road),
            ("crosswalk", Surface::Road),
            ("intersection", Surface::Road),
            ("parkinglot", Surface::Road),
            ("roadblend", Surface::Road),
            ("concrete", Surface::Road),
            ("mall", Surface::Mall),
            ("tile", Surface::Interior),
            ("carpet", Surface::Interior),
            ("rug", Surface::Interior),
            ("grate", Surface::Metal),
        ];
        let first = WORDS.iter().filter_map(|(w, s)| name.find(w).map(|i| (i, *s))).min_by_key(|(i, _)| *i).map(|(_, s)| s);
        let s = match first {
            // (tiles and carpets out of doors are just paving)
            Some(Surface::Interior) | Some(Surface::Mall) if !folder.starts_with("ground/interior") => Surface::Exterior,
            Some(s) => s,
            None if !filed => return Self::classify(shader, texture),
            None if folder.starts_with("ground/interior") => Surface::Interior,
            None if folder.starts_with("ground/exterior") => Surface::Exterior,
            None => Surface::Road,
        };
        match s {
            Surface::Road if name.contains("_tun") => Surface::Tunnel,
            Surface::Road if p.contains("/freeway/") => Surface::Freeway,
            Surface::Road if folder.starts_with("ground/interior") => Surface::Interior,
            s => s,
        }
    }
}

/// Tyre grip factor and drag (1/s) of every surface, read once from the material files.
static SURFACES: std::sync::OnceLock<[(f32, f32); Surface::COUNT]> = std::sync::OnceLock::new();

pub fn load_surfaces(data_dir: &str) {
    let dir = std::path::Path::new(data_dir).join("tune/physics/materials");
    let mut out = [(1.0f32, 0.0f32); Surface::COUNT];
    let mut bumps = [(1.0f32, 0.0f32, 0.0f32); Surface::COUNT];
    let mut depths = [0.0f32; Surface::COUNT];
    for (i, o) in out.iter_mut().enumerate() {
        let name = Surface::from_u8(i as u8).material();
        // (the files are named by the hash of the material's file name)
        let text = [format!("{name}.mtl"), name.to_string()].iter().find_map(|n| std::fs::read_to_string(dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(n)))).ok());
        if let Some(t) = text {
            let val = |key: &str| t.lines().find_map(|l| l.trim().strip_prefix(key).and_then(|v| v.trim().split_whitespace().next().and_then(|x| x.parse::<f32>().ok())));
            *o = (val("grip:").unwrap_or(1.0).clamp(0.01, 1.2), val("drag:").unwrap_or(0.0).clamp(0.0, 1.0));
            bumps[i] = (val("width:").unwrap_or(1.0).max(0.01), val("height:").unwrap_or(0.0), val("space:").unwrap_or(0.0));
            depths[i] = val("depth:").unwrap_or(0.0);
        }
    }
    log::info!("surfaces (grip, drag): {}", (0..Surface::COUNT).map(|i| format!("{} {:.2}/{:.3}", Surface::from_u8(i as u8).material(), out[i].0, out[i].1)).collect::<Vec<_>>().join(", "));
    let _ = SURFACES.set(out);
    let _ = SURFACE_BUMPS.set(bumps);
    let _ = SURFACE_DEPTHS.set(depths);
    // (what a body's contact with each is like: the records' +12 and +16)
    let mut touch = [(0.5f32, 0.5f32); Surface::COUNT];
    for (i, o) in touch.iter_mut().enumerate() {
        let name = Surface::from_u8(i as u8).material();
        let text = [format!("{name}.mtl"), name.to_string()].iter().find_map(|n| std::fs::read_to_string(dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(n)))).ok());
        if let Some(t) = text {
            let val = |key: &str| t.lines().find_map(|l| l.trim().strip_prefix(key).and_then(|v| v.trim().split_whitespace().next().and_then(|x| x.parse::<f32>().ok())));
            *o = (val("friction:").unwrap_or(0.5), val("elasticity:").unwrap_or(0.5));
        }
    }
    let _ = SURFACE_TOUCH.set(touch);
}

static SURFACE_TOUCH: std::sync::OnceLock<[(f32, f32); Surface::COUNT]> = std::sync::OnceLock::new();

/// (friction, elasticity) of a material where bodies touch it. A contact has the product of
/// its two materials' values (0x825A2C88).
pub fn surface_touch(s: Surface) -> (f32, f32) {
    SURFACE_TOUCH.get().map_or((0.5, 0.5), |t| t[s as usize])
}

/// How far a tyre sinks into a surface (`depth`, the material record's +40 - a wheel's +448):
/// its drag along the wheel grows by that share (the end of vehWheel::Update, 0x82566D38).
static SURFACE_DEPTHS: std::sync::OnceLock<[f32; Surface::COUNT]> = std::sync::OnceLock::new();

pub fn surface_depth(s: Surface) -> f32 {
    SURFACE_DEPTHS.get().map_or(0.0, |t| t[s as usize])
}

/// Roughness of a surface: the length of one bump (`width`), how high it is (`height`) and how
/// much its length varies (`space`) - the material record's +28, +32, +36, in the order the
/// files have them (then `depth`, +40).
static SURFACE_BUMPS: std::sync::OnceLock<[(f32, f32, f32); Surface::COUNT]> = std::sync::OnceLock::new();

pub fn surface_bump(s: Surface) -> (f32, f32, f32) {
    SURFACE_BUMPS.get().map_or((1.0, 0.0, 0.0), |t| t[s as usize])
}

/// (grip factor, drag) of a surface.
pub fn surface(s: Surface) -> (f32, f32) {
    SURFACES.get().map_or((1.0, 0.0), |t| t[s as usize])
}

pub struct SectorMesh {
    pub tris: Vec<[Vec3; 3]>,
    /// surface of each triangle (empty = all road)
    pub mats: Vec<u8>,
    /// cell (ix,iz) → triangle indices
    pub grid: HashMap<(i32, i32), Vec<u32>>,
    pub min: Vec3,
    pub max: Vec3,
    /// each triangle's normal as the original's collision forms it (0x825D0C18: from the
    /// area its record holds) and its neighbours across its three edges as its record names
    /// them - where the mesh is the city's own bound; empty otherwise
    pub faces: Vec<Vec3>,
    pub beside: Vec<[Option<u32>; 3]>,
    /// where the mesh is one cell of the city's grid bound: which, and the grid's numbers
    pub cell: Option<GridCell>,
}

/// One square of the city's grid bound (type 8): its x / z index, the grid's 1 / cell size
/// (+132) and margin (+112).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridCell {
    pub x: i32,
    pub z: i32,
    pub inv_size: f32,
    pub margin: f32,
}

impl GridCell {
    /// 0x825CD4E8..0x825CD694: the grid hands a part only the cells its box's ball reaches -
    /// floor((c - R) / size) ..= floor((c + R) / size) on x and z, R = the grid's margin + the
    /// length of the box's half sizes. A polygon that reaches over from the cell next door (the
    /// file repeats those in both cells, rounded differently) is not met through that cell.
    pub fn reached(&self, ball: (Vec3, f32)) -> bool {
        let r = self.margin + ball.1;
        let lo = |c: f32| ((c - r) * self.inv_size).floor() as i32;
        let hi = |c: f32| ((c + r) * self.inv_size).floor() as i32;
        lo(ball.0.x) <= self.x && self.x <= hi(ball.0.x) && lo(ball.0.z) <= self.z && self.z <= hi(ball.0.z)
    }
}

#[inline]
fn cell_of(x: f32, z: f32) -> (i32, i32) {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}

impl SectorMesh {
    pub fn build(tris: Vec<[Vec3; 3]>) -> SectorMesh {
        Self::build_with(tris, Vec::new())
    }

    pub fn build_with(tris: Vec<[Vec3; 3]>, mats: Vec<u8>) -> SectorMesh {
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for (i, t) in tris.iter().enumerate() {
            let tmin = t[0].min(t[1]).min(t[2]);
            let tmax = t[0].max(t[1]).max(t[2]);
            min = min.min(tmin);
            max = max.max(tmax);
            let (x0, z0) = cell_of(tmin.x, tmin.z);
            let (x1, z1) = cell_of(tmax.x, tmax.z);
            // very large triangles (sky domes, water planes) are skipped from fine cells
            if (x1 - x0) * (z1 - z0) > 4096 {
                continue;
            }
            for cx in x0..=x1 {
                for cz in z0..=z1 {
                    grid.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        SectorMesh { tris, mats, grid, min, max, faces: Vec::new(), beside: Vec::new(), cell: None }
    }

    /// The same with what the city's bound holds of each triangle: its area and neighbours.
    pub fn of_bound(tris: Vec<[Vec3; 3]>, mats: Vec<u8>, areas: &[f32], beside: Vec<[Option<u32>; 3]>) -> SectorMesh {
        let mut mesh = Self::build_with(tris, mats);
        if areas.len() == mesh.tris.len() && beside.len() == mesh.tris.len() {
            mesh.faces = mesh.tris.iter().zip(areas).map(|(t, a)| super::gjk::polygon_normal(t, *a)).collect();
            mesh.beside = beside;
        }
        mesh
    }
}

/// How far the city's collision stands proud of its triangles (the mesh bound's margin).
pub const WORLD_MARGIN: f32 = 0.005;

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub surface: Surface,
}

/// Möller–Trumbore, returns distance along the ray.
#[inline]
pub fn ray_tri(o: Vec3, d: Vec3, tri: &[Vec3; 3], max_t: f32) -> Option<(f32, Vec3)> {
    let e1 = tri[1] - tri[0];
    let e2 = tri[2] - tri[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-7 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - tri[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    if t <= 1e-4 || t > max_t {
        return None;
    }
    let mut n = e1.cross(e2).normalize_or_zero();
    if n.dot(d) > 0.0 {
        n = -n;
    }
    Some((t, n))
}

/// One-sided segment test of the level's probe (sub_82619AE0, query +160 = 1): only a face whose
/// front the segment crosses from in front to behind hits (d0 >= 0, d1 < 0); the normal is the
/// face's own, unflipped; `max_t` itself is no hit (the probe drops a value of 0 from the end).
#[inline]
pub fn ray_tri_front(o: Vec3, d: Vec3, tri: &[Vec3; 3], max_t: f32) -> Option<(f32, Vec3)> {
    let e1 = tri[1] - tri[0];
    let e2 = tri[2] - tri[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    // (det = -d . (e1 x e2): a front face has det > 0)
    if det < 1e-7 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - tri[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    if t < 0.0 || t >= max_t {
        return None;
    }
    Some((t, e1.cross(e2).normalize_or_zero()))
}

#[derive(Default)]
pub struct WorldCollider {
    // Stable traversal makes equal-distance contacts independent of hash seeds
    // and asynchronous sector insertion order. The cell lookup remains hashed.
    pub sectors: std::collections::BTreeMap<usize, SectorMesh>,
    /// the level's props as the physics reads them (None: the cars' hulls meet the city only)
    pub loose: Option<std::sync::Arc<super::loose::LooseWorld>>,
}

impl WorldCollider {
    /// Cast a ray against all loaded sectors (walks the XZ cells along the ray).
    pub fn raycast(&self, o: Vec3, d: Vec3, max_t: f32) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        let end = o + d * max_t;
        let mn = o.min(end);
        let mx = o.max(end);
        let (cx0, cz0) = cell_of(mn.x, mn.z);
        let (cx1, cz1) = cell_of(mx.x, mx.z);
        // limit the cell footprint for long near-horizontal rays
        if (cx1 - cx0 + 1) * (cz1 - cz0 + 1) > 400 {
            return self.raycast_coarse(o, d, max_t);
        }
        for s in self.sectors.values() {
            if mx.x < s.min.x || mn.x > s.max.x || mx.z < s.min.z || mn.z > s.max.z || mx.y < s.min.y || mn.y > s.max.y {
                continue;
            }
            for cx in cx0..=cx1 {
                for cz in cz0..=cz1 {
                    if let Some(list) = s.grid.get(&(cx, cz)) {
                        for &i in list {
                            let tri = &s.tris[i as usize];
                            let lim = best.map_or(max_t, |h| h.t);
                            if let Some((t, n)) = ray_tri(o, d, tri, lim) {
                                best = Some(Hit { t, point: o + d * t, normal: n, surface: Surface::from_u8(s.mats.get(i as usize).copied().unwrap_or(0)) });
                            }
                        }
                    }
                }
            }
        }
        best
    }

    /// The level's probe against the city bound (sub_827974F0 -> sub_822C6110, include 0x800: the
    /// city bound and city_roads only, fixed objects): one-sided faces, the nearest hit kept (one
    /// result slot, the smallest t; on a tie the earlier one).
    pub fn probe_front(&self, o: Vec3, d: Vec3, max_t: f32) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        let end = o + d * max_t;
        let mn = o.min(end);
        let mx = o.max(end);
        let (cx0, cz0) = cell_of(mn.x, mn.z);
        let (cx1, cz1) = cell_of(mx.x, mx.z);
        for s in self.sectors.values() {
            if mx.x < s.min.x || mn.x > s.max.x || mx.z < s.min.z || mn.z > s.max.z || mx.y < s.min.y || mn.y > s.max.y {
                continue;
            }
            for cx in cx0..=cx1 {
                for cz in cz0..=cz1 {
                    if let Some(list) = s.grid.get(&(cx, cz)) {
                        for &i in list {
                            let tri = &s.tris[i as usize];
                            let lim = best.map_or(max_t, |h| h.t);
                            if let Some((t, n)) = ray_tri_front(o, d, tri, lim) {
                                best = Some(Hit { t, point: o + d * t, normal: n, surface: Surface::from_u8(s.mats.get(i as usize).copied().unwrap_or(0)) });
                            }
                        }
                    }
                }
            }
        }
        best
    }

    fn raycast_coarse(&self, o: Vec3, d: Vec3, max_t: f32) -> Option<Hit> {
        // step along the ray cell by cell
        let mut best: Option<Hit> = None;
        let steps = (max_t / CELL).ceil().max(1.0) as i32;
        let mut visited = hashbrown::HashSet::new();
        for i in 0..=steps {
            let p = o + d * (i as f32 * CELL).min(max_t);
            let c = cell_of(p.x, p.z);
            if !visited.insert(c) {
                continue;
            }
            for s in self.sectors.values() {
                if let Some(list) = s.grid.get(&c) {
                    for &ti in list {
                        let lim = best.map_or(max_t, |h| h.t);
                        if let Some((t, n)) = ray_tri(o, d, &s.tris[ti as usize], lim) {
                            best = Some(Hit { t, point: o + d * t, normal: n, surface: Surface::from_u8(s.mats.get(ti as usize).copied().unwrap_or(0)) });
                        }
                    }
                }
            }
            if best.is_some() && best.unwrap().t < (i as f32 + 1.0) * CELL {
                break;
            }
        }
        best
    }

    /// Ground height below a point (looks up to `up` above and `down` below).
    pub fn ground(&self, p: Vec3, up: f32, down: f32) -> Option<Hit> {
        self.raycast(p + Vec3::Y * up, -Vec3::Y, up + down)
    }

    /// Closest-point sphere test: returns (penetration depth, normal) for the deepest contact.
    pub fn sphere(&self, c: Vec3, r: f32) -> Option<(f32, Vec3, Vec3)> {
        self.sphere_where(c, r, |_| true)
    }

    /// Every triangle that reaches into a box, once each.
    pub fn triangles_in(&self, mn: Vec3, mx: Vec3, mut f: impl FnMut(&[Vec3; 3], Surface)) {
        self.triangles_of(mn, mx, |s, i| f(&s.tris[i], Surface::from_u8(s.mats.get(i).copied().unwrap_or(0))));
    }

    /// The same by mesh and number.
    pub fn triangles_of<'a>(&'a self, mn: Vec3, mx: Vec3, f: impl FnMut(&'a SectorMesh, usize)) {
        self.triangles_of_ball(mn, mx, None, f)
    }

    /// The same; `ball`: the part's box's ball (centre, radius) by which the city's grid picks
    /// its cells (`GridCell::reached`).
    pub fn triangles_of_ball<'a>(&'a self, mn: Vec3, mx: Vec3, ball: Option<(Vec3, f32)>, mut f: impl FnMut(&'a SectorMesh, usize)) {
        let (cx0, cz0) = cell_of(mn.x, mn.z);
        let (cx1, cz1) = cell_of(mx.x, mx.z);
        let mut seen: Vec<u32> = Vec::new();
        for s in self.sectors.values() {
            if let (Some(b), Some(c)) = (ball, s.cell.as_ref()) {
                if !c.reached(b) { continue; }
            }
            if mx.x < s.min.x || mn.x > s.max.x || mx.z < s.min.z || mn.z > s.max.z || mx.y < s.min.y || mn.y > s.max.y {
                continue;
            }
            seen.clear();
            for cx in cx0..=cx1 {
                for cz in cz0..=cz1 {
                    if let Some(list) = s.grid.get(&(cx, cz)) {
                        seen.extend_from_slice(list);
                    }
                }
            }
            seen.sort_unstable();
            seen.dedup();
            for &i in &seen {
                let tri = &s.tris[i as usize];
                let lo = tri[0].min(tri[1]).min(tri[2]);
                let hi = tri[0].max(tri[1]).max(tri[2]);
                if lo.x > mx.x || hi.x < mn.x || lo.y > mx.y || hi.y < mn.y || lo.z > mx.z || hi.z < mn.z {
                    continue;
                }
                f(s, i as usize);
            }
        }
    }

    /// The same with the cell's number: cells in their order, each one's triangles in theirs.
    pub fn triangles_by_cell<'a>(&'a self, mn: Vec3, mx: Vec3, ball: Option<(Vec3, f32)>, mut f: impl FnMut(usize, &'a SectorMesh, usize)) {
        let (cx0, cz0) = cell_of(mn.x, mn.z);
        let (cx1, cz1) = cell_of(mx.x, mx.z);
        let mut seen: Vec<u32> = Vec::new();
        for (cell, s) in self.sectors.iter() {
            if let (Some(b), Some(c)) = (ball, s.cell.as_ref()) {
                if !c.reached(b) { continue; }
            }
            if mx.x < s.min.x || mn.x > s.max.x || mx.z < s.min.z || mn.z > s.max.z || mx.y < s.min.y || mn.y > s.max.y {
                continue;
            }
            seen.clear();
            for cx in cx0..=cx1 {
                for cz in cz0..=cz1 {
                    if let Some(list) = s.grid.get(&(cx, cz)) {
                        seen.extend_from_slice(list);
                    }
                }
            }
            seen.sort_unstable();
            seen.dedup();
            for &i in &seen {
                let tri = &s.tris[i as usize];
                let lo = tri[0].min(tri[1]).min(tri[2]);
                let hi = tri[0].max(tri[1]).max(tri[2]);
                if lo.x > mx.x || hi.x < mn.x || lo.y > mx.y || hi.y < mn.y || lo.z > mx.z || hi.z < mn.z {
                    continue;
                }
                f(*cell, s, i as usize);
            }
        }
    }

    /// The deepest contact of a sphere among the points `keep` lets through.
    pub fn sphere_where(&self, c: Vec3, r: f32, keep: impl Fn(Vec3) -> bool) -> Option<(f32, Vec3, Vec3)> {
        let (cx0, cz0) = cell_of(c.x - r, c.z - r);
        let (cx1, cz1) = cell_of(c.x + r, c.z + r);
        let mut best: Option<(f32, Vec3, Vec3)> = None;
        for s in self.sectors.values() {
            if c.x + r < s.min.x || c.x - r > s.max.x || c.z + r < s.min.z || c.z - r > s.max.z {
                continue;
            }
            for cx in cx0..=cx1 {
                for cz in cz0..=cz1 {
                    if let Some(list) = s.grid.get(&(cx, cz)) {
                        for &i in list {
                            let tri = &s.tris[i as usize];
                            let q = closest_point_tri(c, tri);
                            let dv = c - q;
                            let d2 = dv.length_squared();
                            if d2 < r * r && d2 > 1e-8 && keep(q) {
                                let d = d2.sqrt();
                                let pen = r - d;
                                if best.map_or(true, |b| pen > b.0) {
                                    best = Some((pen, dv / d, q));
                                }
                            }
                        }
                    }
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod deterministic_contact_tests {
    use super::*;

    #[test]
    fn the_probe_meets_front_faces_only() {
        // (sub_82619AE0: from in front to behind; the face's own normal)
        let up = [Vec3::new(-4.0, 0.0, -4.0), Vec3::new(-4.0, 0.0, 4.0), Vec3::new(4.0, 0.0, -4.0)];
        let (t, n) = ray_tri_front(Vec3::new(-1.0, 1.0, -1.0), Vec3::NEG_Y, &up, 2.0).unwrap();
        assert!((t - 1.0).abs() < 1e-6 && (n - Vec3::Y).length() < 1e-6);
        // its back: none, from below or with the face turned over
        assert!(ray_tri_front(Vec3::new(-1.0, -1.0, -1.0), Vec3::Y, &up, 2.0).is_none());
        let down = [up[0], up[2], up[1]];
        assert!(ray_tri_front(Vec3::new(-1.0, 1.0, -1.0), Vec3::NEG_Y, &down, 2.0).is_none());
        // a hit exactly at the end is none
        assert!(ray_tri_front(Vec3::new(-1.0, 1.0, -1.0), Vec3::NEG_Y, &up, 1.0).is_none());
    }

    #[test]
    fn coincident_contacts_do_not_depend_on_sector_load_order() {
        let flat = [Vec3::new(-4.0, 0.0, -4.0), Vec3::new(-4.0, 0.0, 4.0), Vec3::new(4.0, 0.0, -4.0)];
        let make = |reversed| {
            let mut world = WorldCollider::default();
            let ids = if reversed { [9, 2] } else { [2, 9] };
            for id in ids {
                let material = if id == 2 { Surface::Road } else { Surface::Freeway };
                world.sectors.insert(id, SectorMesh::build_with(vec![flat], vec![material as u8]));
            }
            world
        };
        for _ in 0..16 {
            let a = make(false);
            let b = make(true);
            let start = Vec3::new(-1.0, 1.0, -1.0);
            let ah = a.ground(start, 1.0, 4.0).unwrap();
            let bh = b.ground(start, 1.0, 4.0).unwrap();
            assert_eq!((ah.t, ah.normal, ah.surface), (bh.t, bh.normal, bh.surface));
            assert_eq!(ah.surface, Surface::Freeway);
            assert_eq!(a.sphere(start * Vec3::new(1.0, 0.2, 1.0), 0.5), b.sphere(start * Vec3::new(1.0, 0.2, 1.0), 0.5));
        }
    }
}

/// Ericson, Real-Time Collision Detection 5.1.5
pub fn closest_point_tri(p: Vec3, t: &[Vec3; 3]) -> Vec3 {
    let (a, b, c) = (t[0], t[1], t[2]);
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    a + ab * v + ac * w
}
