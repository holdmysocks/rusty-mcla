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
    /// each triangle's whole material id as the city's bound holds it (bound +208 [+216 [i]], what
    /// the bound's material call F-10817 answers): the table number in the low byte, the
    /// material flags above it (shifted by 8: 0x8000 terrain, 0x10000 buildings, 0x20000
    /// push_sphere ..., what a query's material mask +88 tests) - where the mesh is the city's own
    /// bound; empty otherwise
    pub ids: Vec<u32>,
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

/// One column of `segment_columns`: the rows the segment comes within 5 cm of, and the height range of its part over
/// the column (the column widened by 5 cm).
#[derive(Clone, Copy)]
struct SegColumn {
    lo: i32,
    hi: i32,
    ylo: f64,
    yhi: f64,
}

/// The cells of the box (c0 ..= c1) that the segment a .. b comes within 5 cm of on x / z, column by column: for each
/// column c0.0 ..= c1.0 the rows of the z range of the segment's part over it (the column widened by 5 cm), widened
/// again (empty when lo > hi), and the heights of that part.
fn segment_columns(a: Vec3, b: Vec3, c0: (i32, i32), c1: (i32, i32)) -> Vec<SegColumn> {
    const EPS: f64 = 0.05;
    let (ax, ay, az, bx, by, bz) = (a.x as f64, a.y as f64, a.z as f64, b.x as f64, b.y as f64, b.z as f64);
    let cell = CELL as f64;
    let mut out = Vec::with_capacity((c1.0 - c0.0 + 1).max(0) as usize);
    for cx in c0.0..=c1.0 {
        let (zl, zh, yl, yh) = if (bx - ax).abs() < 1e-9 {
            (az.min(bz), az.max(bz), ay.min(by), ay.max(by))
        } else {
            let t = |x: f64| ((x - ax) / (bx - ax)).clamp(0.0, 1.0);
            let (t0, t1) = (t(cx as f64 * cell - EPS), t((cx + 1) as f64 * cell + EPS));
            let (z0, z1) = (az + (bz - az) * t0, az + (bz - az) * t1);
            let (y0, y1) = (ay + (by - ay) * t0, ay + (by - ay) * t1);
            (z0.min(z1), z0.max(z1), y0.min(y1), y0.max(y1))
        };
        let lo = (((zl - EPS) / cell).floor() as i32).max(c0.1);
        let hi = (((zh + EPS) / cell).floor() as i32).min(c1.1);
        out.push(SegColumn { lo, hi, ylo: yl, yhi: yh });
    }
    out
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
        SectorMesh { tris, mats, grid, min, max, faces: Vec::new(), beside: Vec::new(), ids: Vec::new(), cell: None }
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

thread_local! {
    static PASS_DEATH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// While it lives, this thread's queries pass through the `death` material's triangles: what a
/// car's own suspension and checks read. The original's fluid callback (F-58580) takes every
/// contact on `death` out of the car's solve, so a car stands on what lies below it; its
/// contact manifolds still see them (`PassDeath::off`), for the callback to find.
pub struct PassDeath(bool);

impl PassDeath {
    pub fn on() -> PassDeath {
        PassDeath(PASS_DEATH.with(|c| c.replace(true)))
    }
    pub fn off() -> PassDeath {
        PassDeath(PASS_DEATH.with(|c| c.replace(false)))
    }
}

/// Sets whether this thread's queries pass through `death` (the game's level step, between the cars' steps), and what it was.
pub fn pass_death_set(on: bool) -> bool {
    PASS_DEATH.with(|c| c.replace(on))
}

impl Drop for PassDeath {
    fn drop(&mut self) {
        PASS_DEATH.with(|c| c.set(self.0));
    }
}

/// Is triangle `i` of `s` one this thread's queries see?
#[inline]
fn seen(s: &SectorMesh, i: usize) -> bool {
    !(PASS_DEATH.with(|c| c.get()) && s.mats.get(i) == Some(&(Surface::Death as u8)))
}

#[derive(Default)]
pub struct WorldCollider {
    // Stable traversal makes equal-distance contacts independent of hash seeds
    // and asynchronous sector insertion order. The cell lookup remains hashed.
    pub sectors: std::collections::BTreeMap<usize, SectorMesh>,
    /// the level's props as the physics reads them (None: the cars' hulls meet the city only)
    pub loose: Option<std::sync::Arc<super::loose::LooseWorld>>,
    /// the city's grid bound as the level queries meet it (`camprobe::ProbeBound::of_city`; None without the file)
    pub city: Option<std::sync::Arc<super::camprobe::ProbeBound>>,
}

/// Two colliders' sectors as one map would hold them: by id, `a`'s first on an equal id.
fn merged<'a>(a: &'a std::collections::BTreeMap<usize, SectorMesh>, b: &'a std::collections::BTreeMap<usize, SectorMesh>) -> impl Iterator<Item = (&'a usize, &'a SectorMesh)> + 'a {
    let (mut a, mut b) = (a.iter().peekable(), b.iter().peekable());
    std::iter::from_fn(move || match (a.peek(), b.peek()) {
        (Some(x), Some(y)) if y.0 < x.0 => b.next(),
        (Some(_), _) => a.next(),
        _ => b.next(),
    })
}

/// `WorldCollider::raycast` over the sectors `sectors()` walks (in their id order): the nearest hit, the first found
/// on an equal t.
fn raycast_in<'a, I: Iterator<Item = (&'a usize, &'a SectorMesh)>>(sectors: impl Fn() -> I, o: Vec3, d: Vec3, max_t: f32) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    let end = o + d * max_t;
    let mn = o.min(end);
    let mx = o.max(end);
    let (cx0, cz0) = cell_of(mn.x, mn.z);
    let (cx1, cz1) = cell_of(mx.x, mx.z);
    // limit the cell footprint for long near-horizontal rays
    if (cx1 - cx0 + 1) * (cz1 - cz0 + 1) > 400 {
        return raycast_coarse(sectors, o, d, max_t);
    }
    for (_, s) in sectors() {
        if mx.x < s.min.x || mn.x > s.max.x || mx.z < s.min.z || mn.z > s.max.z || mx.y < s.min.y || mn.y > s.max.y {
            continue;
        }
        for cx in cx0..=cx1 {
            for cz in cz0..=cz1 {
                if let Some(list) = s.grid.get(&(cx, cz)) {
                    for &i in list {
                        if !seen(s, i as usize) {
                            continue;
                        }
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

fn raycast_coarse<'a, I: Iterator<Item = (&'a usize, &'a SectorMesh)>>(sectors: impl Fn() -> I, o: Vec3, d: Vec3, max_t: f32) -> Option<Hit> {
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
        for (_, s) in sectors() {
            if let Some(list) = s.grid.get(&c) {
                for &ti in list {
                    if !seen(s, ti as usize) {
                        continue;
                    }
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

impl WorldCollider {
    /// Cast a ray against all loaded sectors (walks the XZ cells along the ray).
    pub fn raycast(&self, o: Vec3, d: Vec3, max_t: f32) -> Option<Hit> {
        raycast_in(|| self.sectors.iter(), o, d, max_t)
    }

    /// `raycast` against these sectors and `extra`'s together, in the order of their ids as one collider would hold
    /// them: the port's own users that read the drawn garage rooms (`World::set_collider`) besides the city.
    pub fn raycast_with(&self, extra: &WorldCollider, o: Vec3, d: Vec3, max_t: f32) -> Option<Hit> {
        raycast_in(|| merged(&self.sectors, &extra.sectors), o, d, max_t)
    }

    /// `ground` against these sectors and `extra`'s together (`raycast_with`).
    pub fn ground_with(&self, extra: &WorldCollider, p: Vec3, up: f32, down: f32) -> Option<Hit> {
        self.raycast_with(extra, p + Vec3::Y * up, -Vec3::Y, up + down)
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

    /// The same level probe with up to `max` result records (the result list of the original's line probe keeps the
    /// hits nearest the start): every one-sided hit of a city triangle on the segment o .. o + d max_t, each triangle
    /// once, nearest first (on equal t, the order they were found in).
    pub fn probe_front_all(&self, o: Vec3, d: Vec3, max_t: f32, max: usize) -> Vec<Hit> {
        let end = o + d * max_t;
        let mn = o.min(end);
        let mx = o.max(end);
        let (cx0, cz0) = cell_of(mn.x, mn.z);
        let (cx1, cz1) = cell_of(mx.x, mx.z);
        // Only the cells the segment comes within 5 cm of (a hit lies on the segment and inside its triangle's box,
        // so in one of them), not every cell of its box: the sounds' occlusion rays run to 400 m and met ~50,000
        // triangles each (1.4 ms). Equal t keep the order the whole box gave: the sector, the triangle's first cell
        // of the box (column, then row), its index - the same hits in the same order. Each triangle is tested once,
        // in the first of its cells the walk meets, and not where the segment passes over or under it.
        let cols = segment_columns(o, end, (cx0, cz0), (cx1, cz1));
        let mut hits: Vec<(Hit, (usize, i32, i32, u32))> = Vec::new();
        for (si, s) in self.sectors.values().enumerate() {
            if mx.x < s.min.x || mn.x > s.max.x || mx.z < s.min.z || mn.z > s.max.z || mx.y < s.min.y || mn.y > s.max.y {
                continue;
            }
            for (k, c) in cols.iter().enumerate() {
                let cx = cx0 + k as i32;
                for cz in c.lo..=c.hi {
                    let Some(list) = s.grid.get(&(cx, cz)) else { continue };
                    for &i in list {
                        let tri = &s.tris[i as usize];
                        let lo = tri[0].min(tri[1]).min(tri[2]);
                        let hi = tri[0].max(tri[1]).max(tri[2]);
                        let (x0, z0) = cell_of(lo.x, lo.z);
                        let (x1, z1) = cell_of(hi.x, hi.z);
                        // a triangle is in every cell of its box: tested in the first of them the walk meets (an
                        // earlier column, or an earlier row of this one, in its box means it was met there)
                        let first = (x0.max(cx0)..cx).all(|x| {
                            let e = &cols[(x - cx0) as usize];
                            e.lo.max(z0) > e.hi.min(z1)
                        }) && c.lo.max(z0) == cz;
                        if !first {
                            continue;
                        }
                        // nor where the segment's heights over the triangle's columns stay 5 cm clear of its own
                        // (a hit lies on the segment, inside the triangle's box)
                        let (mut yl, mut yh) = (f64::INFINITY, f64::NEG_INFINITY);
                        for e in &cols[(x0.max(cx0) - cx0) as usize..=(x1.min(cx1) - cx0) as usize] {
                            yl = yl.min(e.ylo);
                            yh = yh.max(e.yhi);
                        }
                        if (lo.y as f64) > yh + 0.05 || (hi.y as f64) < yl - 0.05 {
                            continue;
                        }
                        if let Some((t, n)) = ray_tri_front(o, d, tri, max_t) {
                            hits.push((Hit { t, point: o + d * t, normal: n, surface: Surface::from_u8(s.mats.get(i as usize).copied().unwrap_or(0)) }, (si, x0.max(cx0), z0.max(cz0), i)));
                        }
                    }
                }
            }
        }
        hits.sort_by(|a, b| a.0.t.total_cmp(&b.0.t).then(a.1.cmp(&b.1)));
        hits.truncate(max);
        hits.into_iter().map(|h| h.0).collect()
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
                if !self::seen(s, i as usize) {
                    continue;
                }
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
                if !self::seen(s, i as usize) {
                    continue;
                }
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
                            if !seen(s, i as usize) {
                                continue;
                            }
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

#[cfg(test)]
mod probe_all_tests {
    use super::*;

    /// `probe_front_all` before 2026-11-08: every cell of the segment's box (or, past 400 cells, a hash set of cells
    /// sampled along it).
    fn probe_front_all_box(w: &WorldCollider, o: Vec3, d: Vec3, max_t: f32, max: usize) -> Vec<Hit> {
        let mut hits: Vec<Hit> = Vec::new();
        let end = o + d * max_t;
        let mn = o.min(end);
        let mx = o.max(end);
        let (cx0, cz0) = cell_of(mn.x, mn.z);
        let (cx1, cz1) = cell_of(mx.x, mx.z);
        // the cells the segment crosses, with a one-cell margin (half-cell steps), or the whole box when it is small
        let cells: Vec<(i32, i32)> = if (cx1 - cx0 + 1) * (cz1 - cz0 + 1) <= 400 {
            (cx0..=cx1).flat_map(|cx| (cz0..=cz1).map(move |cz| (cx, cz))).collect()
        } else {
            let len = (end - o).length();
            let steps = (len / (CELL * 0.5)).ceil().max(1.0) as i32;
            let mut set = hashbrown::HashSet::new();
            for k in 0..=steps {
                let p = o.lerp(end, k as f32 / steps as f32);
                let (cx, cz) = cell_of(p.x, p.z);
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        set.insert((cx + dx, cz + dz));
                    }
                }
            }
            set.into_iter().collect()
        };
        for s in w.sectors.values() {
            if mx.x < s.min.x || mn.x > s.max.x || mx.z < s.min.z || mn.z > s.max.z || mx.y < s.min.y || mn.y > s.max.y {
                continue;
            }
            let mut seen = hashbrown::HashSet::new();
            for &(cx, cz) in &cells {
                {
                    if let Some(list) = s.grid.get(&(cx, cz)) {
                        for &i in list {
                            if !seen.insert(i) {
                                continue;
                            }
                            if let Some((t, n)) = ray_tri_front(o, d, &s.tris[i as usize], max_t) {
                                hits.push(Hit { t, point: o + d * t, normal: n, surface: Surface::from_u8(s.mats.get(i as usize).copied().unwrap_or(0)) });
                            }
                        }
                    }
                }
            }
        }
        hits.sort_by(|a, b| a.t.total_cmp(&b.t));
        hits.truncate(max);
        hits
    }


    /// The walk over the segment's cells gives the hits the walk over its whole box gave: random sectors of triangles
    /// (tiny to 120 m, both faces, in some worlds with the vertices on the cell lines) and segments (short, to 700 m,
    /// along x / z, on a cell line, steep). Where the box was walked whole (up to 400 cells): the same hits in the same order,
    /// to the bit; past that the old order came from a hash set, so there the same hits in any order.
    #[test]
    fn segment_cells_give_the_box_walks_hits() {
        let mut r = 0x2545_F491u32;
        let mut f = move || {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            r as f32 / u32::MAX as f32
        };
        let (mut cases, mut with_hits, mut big, mut ties) = (0, 0, 0, 0);
        for world in 0..12 {
            let mut w = WorldCollider { sectors: Default::default(), loose: None, city: None };
            for si in 0..3usize {
                let base = Vec3::new(si as f32 * 120.0 - 150.0, 0.0, (si % 2) as f32 * 90.0 - 60.0);
                let mut tris = Vec::new();
                let mut mats = Vec::new();
                for k in 0..900 {
                    let size = [0.3, 4.0, 8.0, 30.0, 120.0][k % 5];
                    let snap = |v: f32| if world % 3 == 0 { (v / 8.0).round() * 8.0 } else { v };
                    let c = base + Vec3::new(snap(f() * 200.0 - 100.0), f() * 20.0 - 5.0, snap(f() * 200.0 - 100.0));
                    let mut v = || c + Vec3::new(snap((f() - 0.5) * size), (f() - 0.5) * size * 0.5, snap((f() - 0.5) * size));
                    let t = [v(), v(), v()];
                    tris.push(t);
                    tris.push([t[0], t[2], t[1]]);
                    mats.push((k % 7) as u8);
                    mats.push((k % 5) as u8);
                }
                w.sectors.insert(si, SectorMesh::build_with(tris, mats));
            }
            for k in 0..400 {
                let mut o = Vec3::new(f() * 500.0 - 250.0, f() * 20.0 - 5.0, f() * 500.0 - 250.0);
                let mut e = Vec3::new(f() * 500.0 - 250.0, f() * 20.0 - 5.0, f() * 500.0 - 250.0);
                match k % 6 {
                    0 => e.x = o.x,
                    1 => e.z = o.z,
                    2 => e = o + Vec3::new(f() * 6.0 - 3.0, f() - 0.5, f() * 6.0 - 3.0),
                    3 => {
                        let x = (o.x / 8.0).round() * 8.0;
                        o.x = x;
                        e.x = x;
                    }
                    4 => {
                        // steep: from under the triangles to over them
                        o.y = -30.0 + f() * 10.0;
                        e.y = 30.0 + f() * 10.0;
                    }
                    _ => {}
                }
                let d = e - o;
                let a = w.probe_front_all(o, d, 1.0, 16);
                let b = probe_front_all_box(&w, o, d, 1.0, 16);
                let ((cx0, cz0), (cx1, cz1)) = (cell_of(o.min(e).x, o.min(e).z), cell_of(o.max(e).x, o.max(e).z));
                cases += 1;
                with_hits += (!b.is_empty()) as usize;
                ties += b.windows(2).any(|p| p[0].t == p[1].t) as usize;
                if (cx1 - cx0 + 1) * (cz1 - cz0 + 1) <= 400 {
                    assert_eq!(fmt(&a), fmt(&b), "world {world} case {k}");
                } else {
                    big += 1;
                    let (mut sa, mut sb) = (fmt(&a), fmt(&b));
                    sa.sort();
                    sb.sort();
                    assert_eq!(sa, sb, "world {world} case {k} (large box)");
                }
            }
        }
        eprintln!("{cases} segments, {with_hits} with hits, {big} past 400 cells, {ties} with equal t");
        assert!(with_hits > cases / 3 && big > 50);
    }

    /// `raycast_with` over two colliders gives what one collider holding all their sectors gives, to the bit: the
    /// nearest hit, the first in id order on an equal t (sectors split between the two at random, ids interleaved;
    /// short rays and long ones past 400 cells, which walk the coarse way; some vertices on the cell lines).
    #[test]
    fn raycast_with_is_one_colliders_raycast() {
        let mut r = 0x1234_5679u32;
        let mut f = move || {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            r as f32 / u32::MAX as f32
        };
        let (mut cases, mut hits, mut coarse, mut from_extra, mut ties) = (0, 0, 0, 0, 0);
        for world in 0..8 {
            let mut all = WorldCollider::default();
            let (mut a, mut b) = (WorldCollider::default(), WorldCollider::default());
            for si in 0..8usize {
                let base = Vec3::new((si % 4) as f32 * 120.0 - 200.0, 0.0, (si / 4) as f32 * 150.0 - 80.0);
                let mut tris = Vec::new();
                let mut mats = Vec::new();
                for k in 0..300 {
                    let size = [0.5, 4.0, 12.0, 40.0][k % 4];
                    let snap = |v: f32| if world % 2 == 0 { (v / 8.0).round() * 8.0 } else { v };
                    let c = base + Vec3::new(snap(f() * 160.0 - 80.0), f() * 10.0, snap(f() * 160.0 - 80.0));
                    let mut v = || c + Vec3::new(snap((f() - 0.5) * size), (f() - 0.5) * size * 0.3, snap((f() - 0.5) * size));
                    tris.push([v(), v(), v()]);
                    mats.push((k % 9) as u8);
                }
                // (a copy of a floor in another sector: equal t across the two colliders)
                if si == 3 {
                    tris.push([Vec3::new(-140.0, 2.0, -40.0), Vec3::new(-60.0, 2.0, -40.0), Vec3::new(-100.0, 2.0, 40.0)]);
                    mats.push(1);
                }
                if si == 6 {
                    tris.push([Vec3::new(-140.0, 2.0, -40.0), Vec3::new(-60.0, 2.0, -40.0), Vec3::new(-100.0, 2.0, 40.0)]);
                    mats.push(2);
                }
                let id = si * 7 + 3;
                all.sectors.insert(id, SectorMesh::build_with(tris.clone(), mats.clone()));
                if f() < 0.5 { &mut a } else { &mut b }.sectors.insert(id, SectorMesh::build_with(tris, mats));
            }
            for k in 0..500 {
                // (every fifth over the shared floor)
                let o = if k % 5 == 0 { Vec3::new(f() * 40.0 - 120.0, 2.5 + f() * 10.0, f() * 40.0 - 20.0) } else { Vec3::new(f() * 600.0 - 300.0, f() * 20.0 - 2.0, f() * 600.0 - 300.0) };
                let (d, t) = match k % 4 {
                    0 => (-Vec3::Y, 30.0),
                    1 => (Vec3::new(f() - 0.5, f() * 0.2 - 0.1, f() - 0.5).normalize(), 6.0 + f() * 40.0),
                    2 => (Vec3::new(f() - 0.5, -0.05, f() - 0.5).normalize(), 600.0),
                    _ => (Vec3::new(f() - 0.5, f() - 0.8, f() - 0.5).normalize(), 25.0),
                };
                let one = all.raycast(o, d, t);
                let two = a.raycast_with(&b, o, d, t);
                assert_eq!(fmt(&one.into_iter().collect::<Vec<_>>()), fmt(&two.into_iter().collect::<Vec<_>>()), "world {world} case {k}");
                assert_eq!(fmt(&all.ground(o, 5.0, 20.0).into_iter().collect::<Vec<_>>()), fmt(&a.ground_with(&b, o, 5.0, 20.0).into_iter().collect::<Vec<_>>()), "world {world} ground {k}");
                cases += 1;
                hits += one.is_some() as usize;
                coarse += (k % 4 == 2) as usize;
                ties += matches!((a.raycast(o, d, t), b.raycast(o, d, t)), (Some(x), Some(y)) if x.t == y.t) as usize;
                from_extra += (one.is_some() && a.raycast(o, d, t).map(|h| h.t.to_bits()) != one.map(|h| h.t.to_bits())) as usize;
            }
        }
        eprintln!("{cases} rays, {hits} with a hit, {coarse} long, {from_extra} hit nearest in the second collider, {ties} with an equal t in both");
        assert!(hits > 400 && from_extra > 200 && ties > 20);
    }

    fn fmt(h: &[Hit]) -> Vec<String> {
        h.iter().map(|h| format!("{:08x} {:?} {:?} {:?}", h.t.to_bits(), h.point.to_array().map(f32::to_bits), h.normal.to_array().map(f32::to_bits), h.surface)).collect()
    }
}
