//! The original's map: the minimap and the GPS map's icons, as the executable draws them.
//!
//! Rules read from the executable (documents in `work/mm`: `rule_frame.md`, `rule_renderers.md`,
//! `rule_elements.md`) and checked against frames recorded from the running original
//! (`work/mm/trace_idle.md`); log in UI_WORK.md, "Minimap and GPS map 1:1".
//!
//! - The map is a list of ELEMENTS in seven layers (`mcHudMapServer`, `F-72032`): gas
//!   stations, locations, vehicles, checkpoints, the player. Each kind has a RENDERER
//!   (`F-98280`) that turns the element into a few textured meshes in world space
//!   (`resources/ui/meshes`: `icon_ring_hangout`, `icon_ring_car`, `caricon`, `icon_ingame<N>` ...),
//!   sized from the camera height, coloured by the tune (`tune/ui` `HudMap`) or by the element,
//!   pulsing and cross-fading on a clock - or, on the minimap, into a dot on the rim when it lies
//!   outside the map.
//! - MINIMAP (mode 0): a 220x220 render target (`__mini_map`, 8888, 4x MSAA) cleared to the hudmap
//!   movie's background colour with alpha 0; the five road meshes, the queued icon meshes, then a
//!   round mask that takes alpha away (`mask0.dds`, reverse-subtract), then the rim dots. The
//!   hudmap movie shows that target in its clip `renderMe` at 90 % over its `miniBG` plate.
//!   The camera is the movie's `target0_cam`, turning with the car (heading up).
//!   The port draws the target in software ([`Target`]) and hands it to the movie as a texture.
//! - GPS MAP (mode 1): the same icons in the 3D scene of the map camera; drawn here through egui
//!   on top of the city ([`paint_world`]).

use glam::{Mat4, Vec2, Vec3, Vec4};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------------------------
// Tuning

/// `tune/ui` `HudMap` with the executable's defaults for what the file leaves out.
#[derive(Clone, Default)]
pub struct Tune {
    m: HashMap<String, Vec<f32>>,
}

impl Tune {
    pub fn new(m: &std::collections::HashMap<String, Vec<f32>>) -> Tune {
        Tune { m: m.iter().map(|(k, v)| (k.clone(), v.clone())).collect() }
    }

    /// The `HudMap` block of `tune/ui` ("Name v v v v" lines).
    pub fn load(data_dir: &str) -> Tune {
        let mut m = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(Path::new(data_dir).join("tune/ui")) {
            for e in rd.flatten() {
                let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
                if !t.contains("HudMap {") {
                    continue;
                }
                for l in t.lines() {
                    let mut tok = l.split_whitespace();
                    let Some(k) = tok.next() else { continue };
                    let v: Vec<f32> = tok.filter_map(|x| x.parse().ok()).collect();
                    if !v.is_empty() {
                        m.insert(k.to_string(), v);
                    }
                }
            }
        }
        Tune { m }
    }

    pub fn f(&self, k: &str, d: f32) -> f32 {
        self.m.get(k).and_then(|v| v.first().copied()).unwrap_or(d)
    }

    pub fn c(&self, k: &str, d: [f32; 4]) -> [f32; 4] {
        match self.m.get(k) {
            Some(v) if v.len() >= 3 => [v[0], v[1], v[2], v.get(3).copied().unwrap_or(d[3])],
            _ => d,
        }
    }

    pub fn v3(&self, k: &str, d: Vec3) -> Vec3 {
        match self.m.get(k) {
            Some(v) if v.len() >= 3 => Vec3::new(v[0], v[1], v[2]),
            _ => d,
        }
    }
}

/// The career's difficulty colours (`tune/career/0x1784b502` "Career": Easy, Medium, Hard,
/// Impossible), as floats (what `Racer_SetIconColorByDifficulty` gives a racer).
pub const DIFFICULTY: [[f32; 4]; 4] = [[0.576471, 0.980392, 0.0313726, 1.0], [1.0, 1.0, 0.0, 1.0], [0.988235, 0.611765, 0.0, 1.0], [0.8, 0.0, 0.2, 1.0]];

/// The same colours through an element's colour word (`SetColorByDifficulty`: bytes made by
/// truncation, `F-88563`).
pub fn difficulty_word(d: usize) -> [f32; 4] {
    word(DIFFICULTY[d.min(3)])
}

/// A colour through an 8-bit word (truncating, as `fctiwz` does).
pub fn word(c: [f32; 4]) -> [f32; 4] {
    let b = |x: f32| ((x * 255.0) as i32).clamp(0, 255) as f32 / 255.0;
    [b(c[0]), b(c[1]), b(c[2]), b(c[3])]
}

// ---------------------------------------------------------------------------------------------
// Pictures and meshes

/// A texture as straight RGBA in 0..1 with its box-filtered mip chain.
pub struct Img {
    levels: Vec<(usize, usize, Vec<[f32; 4]>)>,
    /// the top level as 8-bit RGBA (for egui)
    pub rgba8: Vec<u8>,
    pub w: usize,
    pub h: usize,
}

impl Img {
    pub fn from_rgba8(w: usize, h: usize, px: &[u8]) -> Img {
        let top: Vec<[f32; 4]> = px.chunks_exact(4).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0]).collect();
        let mut levels = vec![(w, h, top)];
        while levels.last().map_or(false, |l| l.0 > 1 || l.1 > 1) {
            let (pw, ph, prev) = levels.last().unwrap();
            let (nw, nh) = ((pw / 2).max(1), (ph / 2).max(1));
            let mut out = vec![[0.0f32; 4]; nw * nh];
            for y in 0..nh {
                for x in 0..nw {
                    let mut s = [0.0f32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = prev[(2 * y + dy).min(ph - 1) * pw + (2 * x + dx).min(pw - 1)];
                        for c in 0..4 {
                            s[c] += p[c] * 0.25;
                        }
                    }
                    out[y * nw + x] = s;
                }
            }
            levels.push((nw, nh, out));
        }
        Img { levels, rgba8: px.to_vec(), w, h }
    }

    fn bilinear(&self, l: usize, u: f32, v: f32) -> [f32; 4] {
        let (w, h, px) = &self.levels[l.min(self.levels.len() - 1)];
        let (w, h) = (*w, *h);
        // (texel centres at (i + 0.5) / size, clamped at the edges)
        let x = (u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
        let y = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let (a, b, c, d) = (px[y0 * w + x0], px[y0 * w + x1], px[y1 * w + x0], px[y1 * w + x1]);
        let mut o = [0.0f32; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bot = c[k] + (d[k] - c[k]) * fx;
            o[k] = top + (bot - top) * fy;
        }
        o
    }

    /// The mip chain (size and straight RGBA 0..1 per level), for the GPU's copy (minimap_gpu.rs).
    pub fn levels(&self) -> &[(usize, usize, Vec<[f32; 4]>)] {
        &self.levels
    }

    /// Trilinear sample at level of detail `lod` (log2 texels per pixel).
    pub fn sample(&self, u: f32, v: f32, lod: f32) -> [f32; 4] {
        let lod = lod.clamp(0.0, (self.levels.len() - 1) as f32);
        let l0 = lod.floor() as usize;
        let f = lod - l0 as f32;
        let a = self.bilinear(l0, u, v);
        if f < 1e-3 || l0 + 1 >= self.levels.len() {
            return a;
        }
        let b = self.bilinear(l0 + 1, u, v);
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, a[3] + (b[3] - a[3]) * f]
    }
}

/// One geometry of a UI mesh: its triangles and its texture.
pub struct Part {
    pub pos: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub idx: Vec<u32>,
    pub tex: Option<Arc<Img>>,
    pub tex_name: String,
}

pub struct IconMesh {
    pub parts: Vec<Part>,
    /// bounding radius in the xz plane (the culling test's sphere)
    pub radius: f32,
}

/// The map's meshes and their textures (`resources/ui/meshes`, `resources/ui/meshtextures.r9`),
/// loaded on first use.
pub struct Assets {
    data_dir: String,
    meshes: HashMap<String, Option<Arc<IconMesh>>>,
    dict: Option<HashMap<String, Arc<Img>>>,
    pub mask0: Option<Img>,
    /// egui textures of the pictures, for the GPS map's drawing
    handles: HashMap<String, egui::TextureHandle>,
    /// `mask0` shared with the GPU's minimap (minimap_gpu.rs)
    mask0_gpu: Option<Arc<Img>>,
}

impl Assets {
    pub fn new(data_dir: &str) -> Assets {
        let mask0 = std::fs::read(Path::new(data_dir).join("ui/hudmap/mask0.dds"))
            .ok()
            .and_then(|d| mcla_formats::dds::parse_dds("mask0", &d).ok())
            .map(|t| Img::from_rgba8(t.width as usize, t.height as usize, &t.to_rgba8()));
        if mask0.is_none() {
            log::warn!("hudmap: ui/hudmap/mask0.dds not found");
        }
        Assets { data_dir: data_dir.to_string(), meshes: HashMap::new(), dict: None, mask0, handles: HashMap::new(), mask0_gpu: None }
    }

    fn texture(&mut self, mesh: &str, name: &str) -> Option<Arc<Img>> {
        let dict = self.dict.get_or_insert_with(|| {
            use mcla_formats::rage::texture::Texture;
            use mcla_formats::rage::{Dictionary, Rd};
            let mut out = HashMap::new();
            if let Ok(res) = crate::assets::load_resource_file(&Path::new(&self.data_dir).join("resources/ui/meshtextures.r9")) {
                let rd = Rd::new(&res);
                if let Ok(d) = Dictionary::parse(&rd, 0) {
                    for o in d.values.iter().flatten() {
                        if let Ok(t) = Texture::parse(&rd, *o) {
                            out.insert(t.name.to_ascii_lowercase(), Arc::new(Img::from_rgba8(t.width as usize, t.height as usize, &t.to_rgba8())));
                        }
                    }
                }
            }
            out
        });
        let name = name.to_ascii_lowercase();
        let short = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string();
        // ("<mesh folder>/<file>", else any picture of that file name - the white 8x8 of
        // `icon_ring_car` is one of several identical ones)
        dict.get(&format!("{}/{}", mesh.to_ascii_lowercase(), short))
            .or_else(|| dict.iter().find(|(k, _)| k.rsplit(['/', '\\']).next() == Some(short.as_str())).map(|(_, v)| v))
            .cloned()
    }

    /// `mask0` for the GPU's minimap.
    pub fn mask0_arc(&mut self) -> Option<Arc<Img>> {
        if self.mask0_gpu.is_none() {
            self.mask0_gpu = self.mask0.as_ref().map(|m| Arc::new(Img::from_rgba8(m.w, m.h, &m.rgba8)));
        }
        self.mask0_gpu.clone()
    }

    /// The icon meshes the HUD draws, read and parsed ahead (at start-up, on the loading thread): read at its first
    /// draw, a race's first checkpoint icons took 20 - 45 ms of one frame (docs/PERFORMANCE.md 2026-10-27 (5)).
    pub fn warm(&mut self) {
        const MESHES: [&str; 16] = [
            // (the 3D icons - hookicons.rs `mesh_name` - and the racers' markers)
            "iconmesh", "icon_arrow", "icon_arrow2", "target_ripple_ingame", "ctf_bomb", "ctf_flag", "icon_racer",
            // (the minimap and the GPS map)
            "target_ripple", "checkpoint_sphere", "icon_ring_car", "icon_ring_hangout", "icon_gasStation", "icon_ingame1", "caricon", "icon_flag", "icon_flagBase",
        ];
        for m in MESHES {
            self.mesh(m);
        }
    }

    pub fn mesh(&mut self, name: &str) -> Option<Arc<IconMesh>> {
        if let Some(m) = self.meshes.get(name) {
            return m.clone();
        }
        let dir = Path::new(&self.data_dir).join("resources/ui/meshes");
        let plain = dir.join(format!("{name}.r102"));
        let hashed = dir.join(format!("0x{:08x}.r102", mcla_formats::hash::joaat(&format!("{name}.xdr"))));
        let path = if plain.exists() { plain } else { hashed };
        let mut parts = Vec::new();
        let mut radius = 0.0f32;
        if let Ok(res) = crate::assets::load_resource_file(&path) {
            if let Ok(d) = mcla_formats::rage::scan::Drawable::parse(&res) {
                for model in &d.models {
                    for (mesh, si) in &model.parts {
                        let tex_name = d.shader_group.shaders.get(*si as usize).and_then(|s| s.diffuse()).unwrap_or("").to_string();
                        let tex = self.texture(name, &tex_name);
                        for p in &mesh.positions {
                            radius = radius.max(Vec2::new(p.x, p.z).length());
                        }
                        parts.push(Part { pos: mesh.positions.clone(), uv: mesh.uv0.clone(), idx: mesh.indices.clone(), tex, tex_name });
                    }
                }
            }
        }
        let m = if parts.is_empty() {
            log::warn!("hudmap: mesh {} not found", name);
            None
        } else {
            Some(Arc::new(IconMesh { parts, radius }))
        };
        self.meshes.insert(name.to_string(), m.clone());
        m
    }

    fn handle(&mut self, ctx: &egui::Context, key: &str, img: &Img) -> egui::TextureHandle {
        self.handles
            .entry(key.to_string())
            .or_insert_with(|| ctx.load_texture(format!("hudmap:{key}"), crate::flash_ui::plain_image(img.w as u32, img.h as u32, &img.rgba8), egui::TextureOptions::LINEAR))
            .clone()
    }
}

// ---------------------------------------------------------------------------------------------
// Elements

/// What an element is (`+36` icon type, `+38` subtype; renderer table `F-98280`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// type 0: a checkpoint / destination flag; subtype 0 start (or next), 1 next, 2 next-next,
    /// 3 normal, 4 lap, 5 finish
    Checkpoint(u8),
    /// type 1 subtype 0
    Player,
    /// type 1 subtypes 1..5 and 10: hookmen and story racers - the ring with their icon
    Racer(u8),
    /// type 1 subtype 6 (and > 10): race opponents, remote players - the arrow
    OtherCar,
    /// type 1 subtype 7
    Cop,
    /// type 1 subtype 8: a payback's target car
    PaybackCar,
    /// type 1 subtype 9: a police-managed car that is not a cop
    RedCar,
    /// type 2 / 10: capture-the-flag flag and base
    Flag,
    FlagBase,
    /// type 6
    Garage,
    /// type 7 (local hangout: series)
    Series,
    /// type 8
    Wager,
    /// type 9: a story mission's place
    Story,
    /// type 11: a wager hangout with pink-slip profiles
    PinkSlip,
    /// type 12
    Gas,
    /// type 13
    Delivery,
    /// type 14
    Payback,
}

impl Kind {
    pub fn type_byte(&self) -> u8 {
        match self {
            Kind::Checkpoint(_) => 0,
            Kind::Player | Kind::Racer(_) | Kind::OtherCar | Kind::Cop | Kind::PaybackCar | Kind::RedCar => 1,
            Kind::Flag => 2,
            Kind::FlagBase => 10,
            Kind::Garage => 6,
            Kind::Series => 7,
            Kind::Wager => 8,
            Kind::Story => 9,
            Kind::PinkSlip => 11,
            Kind::Gas => 12,
            Kind::Delivery => 13,
            Kind::Payback => 14,
        }
    }

    /// The layer an element of this kind is listed in (`F-77867`); flag bits as `+41`.
    pub fn layer(&self, flags: u8) -> usize {
        match self {
            Kind::Checkpoint(_) => {
                if flags & 1 != 0 {
                    5
                } else if flags & (0x02 | 0x04 | 0x08 | 0x40 | 0x80) != 0 {
                    4
                } else {
                    3
                }
            }
            Kind::Player => 6,
            Kind::Racer(_) | Kind::OtherCar | Kind::Cop | Kind::PaybackCar | Kind::RedCar => {
                if flags & 2 != 0 {
                    6
                } else {
                    2
                }
            }
            Kind::Flag => 5,
            Kind::FlagBase => 4,
            Kind::Garage | Kind::Series | Kind::Wager | Kind::Story | Kind::PinkSlip => 2,
            Kind::Gas => 0,
            Kind::Delivery | Kind::Payback => 4,
        }
    }
}

/// Capacity of each layer (byte table 0x820289CC): a full layer drops further elements.
pub const LAYER_CAP: [usize; 7] = [32, 8, 100, 50, 20, 15, 4];

/// One map element as the renderers see it.
#[derive(Clone, Debug)]
pub struct Element {
    pub kind: Kind,
    /// position (`+80`, or the attached vehicle's matrix)
    pub pos: Vec3,
    /// vehicles: the matrix's row 2 (the car's BACKWARD axis); the heading is taken from it
    pub back: Vec3,
    /// colour: the element's word (`+128`) or the racer's own colour (`racer+96`)
    pub colour: [f32; 4],
    /// `+124` SetSelected
    pub selected: bool,
    /// `+126` an active mission points at it (pulse, the "!" cross-fade)
    pub mission: bool,
    /// `+41` bits: 0x08 red team, 0x10 blue team, 0x20 pulsate, 0x40 / 0x80 base team
    pub flags: u8,
    /// racers: `icon_ingame<N>` (`Racer_SetIconType`)
    pub icon: u8,
    /// `+32` the id (location UID, `vehicle+140`); the port's own numbers for the hookmen.
    /// The 3D icons (hookicons.rs) keep their slots by it.
    pub uid: u32,
}

impl Element {
    pub fn at(kind: Kind, pos: Vec3) -> Element {
        Element { kind, pos, back: Vec3::Z, colour: [1.0; 4], selected: false, mission: false, flags: 0, icon: 1, uid: 0 }
    }
}

// ---------------------------------------------------------------------------------------------
// Who is on the map (the original's producers, `rule_elements.md`, mapped onto the port's state)

/// The 27 service stations of Los Angeles (`script_obj/game/MiscTasks/0x53b4b66d`: one type-12
/// element each, UID 700 + i, for the life of the script).
pub const GAS_STATIONS: [[f32; 3]; 27] = [
    [241.0, 25.76, -918.4], [516.8, 31.76, -1368.3], [519.7, 30.76, -1290.6], [816.06, 31.76, -1283.56], [-259.4, -3.06, -32.4], [-732.2, 2.76, -157.7],
    [-559.9, 39.76, -1784.9], [-2768.9, -29.76, -979.4], [1030.06, 6.76, -307.56], [38.0, 6.76, -415.0], [-598.9, -10.76, 339.9], [-1834.4, 3.7, -216.6],
    [516.6, 20.4, -751.6], [2348.3, -3.5, -263.3], [-607.0, -22.1, 1700.7], [-1112.4, -21.8, 1650.3], [-1715.6, -12.0, 1580.3], [-1796.6, -10.8, 1506.3],
    [-1830.6, -10.48, 895.3], [-163.6, -14.8, 514.3], [-197.6, -14.8, 518.3], [520.6, -13.8, 421.3], [1206.6, -16.0, 1401.3], [974.9, -15.0, 1188.3],
    [-1248.9, -7.9, 1280.3], [-156.9, -21.6, 1555.9], [946.3, -14.6, 456.0],
];

/// The career's fixed places (`game/career/0x1529a945`: mcCareer_RegisterLocation + SetElement):
/// (UID, kind, position). Local hangouts are made yellow by their core script.
pub const PLACES: [(u32, Kind, [f32; 3]); 18] = [
    (505, Kind::Garage, [844.728, 18.28, -819.383]),
    (512, Kind::Garage, [-3095.061, -22.84, 165.897]),
    (543, Kind::Garage, [-51.7, -16.8, 1330.0]),
    (500, Kind::Series, [729.046, 9.391, -285.351]),
    (501, Kind::Series, [-470.347, 3.388, -171.286]),
    (502, Kind::Series, [-1683.348, -2.373, 261.934]),
    (510, Kind::Series, [-2437.672, -8.658, 332.416]),
    (520, Kind::Series, [-1012.003, 31.024, -1852.48]),
    (530, Kind::Series, [2099.811, -2.496, -254.723]),
    (540, Kind::Series, [-1387.7, -3.9, 1113.2]),
    (503, Kind::Wager, [-146.544, 8.162, -469.806]),
    (511, Kind::Wager, [-2747.15, -3.641, -203.055]),
    (521, Kind::Wager, [684.169, 77.288, -1507.205]),
    (531, Kind::Wager, [1847.427, -8.991, 606.823]),
    (541, Kind::Wager, [1386.6, -9.5, 1219.1]),
    // (the delivery and payback elements follow the car their core parks beside the garage -
    // SetRacerElement; where the running original has them, 100 % save, 2026-10-19)
    (504, Kind::Delivery, [834.47, 18.06, -843.32]),
    (542, Kind::Delivery, [-37.61, -19.67, 1316.65]),
    (513, Kind::Payback, [-3067.87, -24.82, 189.82]),
];

/// The port's state as the map's element lists, layer by layer (each list in the order its
/// producer adds; capacities enforced).
pub fn elements(game: &crate::game::Game, vehicles: &crate::vehicles::Vehicles, pursuit: bool) -> Vec<Element> {
    let mut layers: [Vec<Element>; 7] = Default::default();
    let mut push = |e: Element| {
        let l = e.kind.layer(e.flags);
        if layers[l].len() < LAYER_CAP[l] {
            layers[l].push(e);
        }
    };
    let player = vehicles.cars.iter().find(|c| c.is_player);
    let racing = game.race.is_some();
    // service stations: always listed
    for (i, g) in GAS_STATIONS.iter().enumerate() {
        let mut e = Element::at(Kind::Gas, Vec3::from(*g));
        e.uid = 700 + i as u32;
        push(e);
    }
    // (Script_SuspendAllOpponents: missions and races take the places and the hookmen off the
    // map; a new career shows them once the garage is reached - approx., the state scripts'
    // start is not modelled)
    let career = game.career.loaded();
    let places = !racing && (!career || game.career.reached("GARAGE_HOLLYWOOD"));
    let pick = game.gps_pick;
    if places {
        for (uid, kind, p) in PLACES.iter().filter(|x| !matches!(x.1, Kind::Delivery | Kind::Payback)) {
            let mut e = Element::at(*kind, Vec3::from(*p));
            e.uid = *uid;
            if matches!(kind, Kind::Series) {
                e.colour = [1.0, 1.0, 0.0, 1.0];
            }
            e.selected = pick.map_or(false, |q| q.distance(e.pos) < 30.0);
            push(e);
        }
    }
    // hookmen: the cruising slots' men (`Roamers::marks`: place, heading, difficulty) - the local
    // hookman (subtype 3, icon 4 = the flag); the current mission's man (subtype 2) with the
    // mission's pulse and "!" cross-fade, selected while he is the GPS target
    if !racing {
        let man = game.rival_mark.as_ref().map(|m| m.0);
        // (the slot men come with `map_men` when it is filled; `cruiser_marks` otherwise)
        let have_men = game.map_men.iter().any(|m| m.2 == 3);
        for (i, (p, f, d)) in game.cruiser_marks.iter().enumerate().filter(|_| !have_men) {
            if man.map_or(false, |m| m.distance(*p) < 60.0) {
                continue;
            }
            let mut e = Element::at(Kind::Racer(3), *p);
            e.uid = 3000 + i as u32;
            e.back = -*f;
            e.colour = DIFFICULTY[(*d).min(3)];
            e.icon = 4;
            push(e);
        }
        for (i, (p, f, sub, icon, d, mission)) in game.map_men.iter().enumerate() {
            if man.map_or(false, |m| m.distance(*p) < 60.0) {
                continue;
            }
            let mut e = Element::at(Kind::Racer(*sub), *p);
            e.uid = 2000 + i as u32;
            e.back = -*f;
            e.colour = DIFFICULTY[(*d).min(3)];
            e.icon = *icon;
            e.mission = *mission;
            push(e);
        }
        // the red-light racer waiting at a light (redlight.rs; the original's red-light hookmen,
        // subtype 1, icon 3 - `<district>_redlight_hk1` scripts)
        for (i, c) in vehicles.cars.iter().enumerate().filter(|(_, c)| c.driver == "redlight") {
            let mut e = Element::at(Kind::Racer(1), c.state.pos);
            e.uid = 4000 + i as u32;
            e.back = -c.state.forward();
            e.colour = DIFFICULTY[game.difficulty_of(&c.model.name).clamp(0, 3) as usize];
            e.icon = 3;
            push(e);
        }
        if let Some((p, _, _)) = game.rival_mark.as_ref() {
            let car = vehicles.cars.iter().filter(|c| c.racer).min_by(|a, b| a.state.pos.distance(*p).total_cmp(&b.state.pos.distance(*p)));
            let mut e = Element::at(Kind::Racer(2), *p);
            e.uid = 4999;
            if let Some(c) = car.filter(|c| c.state.pos.distance(*p) < 60.0) {
                e.pos = c.state.pos;
                e.back = -c.state.forward();
            }
            e.colour = DIFFICULTY[game.mc4_hookman_diff().unwrap_or(1).min(3)];
            e.icon = 4;
            // (flashing while his mission's current objective points at him - `Career::pulse_objective`)
            e.mission = game.career.rival_mission().map_or(false, |m| game.career.pulse_objective(&m) == Some(0));
            e.selected = game.mission_target.is_some();
            push(e);
        }
    }
    // the red light's and the freeway's man keep their racer element through the race, which the
    // racer renderer draws as the arrow in RacerColor while a race is current - work/hookmen2
    // `cap/rl_14`, `rl_15`, `fw_06` (the green arrow by the player's, a cop's blue one beside); the
    // career's races show no arrow for their opponents (work/ui/ref/ng_race_044..047, RACE_GH_06)
    let alt = game.active_def.as_ref().map_or(false, |d| crate::althk::is_alt_race(&d.id));
    if let Some(race) = game.race.as_ref().filter(|_| alt) {
        for (k, &i) in race.participants.iter().enumerate() {
            let Some(c) = vehicles.cars.get(i).filter(|c| !c.is_player) else { continue };
            let mut e = Element::at(Kind::OtherCar, c.state.pos);
            e.uid = 5000 + k as u32;
            e.back = -c.state.forward();
            push(e);
        }
    }
    // police (PoliceManager: listed within 500 m of a racer while no pursuit; always, pulsing, in
    // a pursuit)
    for c in vehicles.cars.iter().filter(|c| c.cop) {
        let near = vehicles.cars.iter().any(|r| (r.is_player || r.racer) && r.state.pos.distance_squared(c.state.pos) < 500.0 * 500.0);
        if !(pursuit || near) {
            continue;
        }
        let mut e = Element::at(Kind::Cop, c.state.pos);
        e.back = -c.state.forward();
        if pursuit {
            e.flags |= 0x20;
        }
        push(e);
    }
    // the race's checkpoints (OrderedCheckpointCore / the unordered core): the ones left in the
    // lap, the next one subtype 1, the others 3, the lap's last 4 (laps to go) or 5 (finish);
    // unordered: all left as 2, the finish (5) once it is the only one left
    if let (Some(race), Some(pi)) = (game.race.as_ref(), vehicles.cars.iter().position(|c| c.is_player)) {
        let n = race.checkpoints.len();
        let next = race.next.get(pi).copied().unwrap_or(0);
        let unordered = race.rivals_checkpoints.is_some();
        let lap_len = race.lap_len.max(1);
        let lap_end = ((next / lap_len) + 1) * lap_len;
        for i in next..n.min(if unordered { n } else { lap_end }) {
            let sub = if i + 1 == n {
                if unordered && next + 1 < n {
                    continue;
                }
                5
            } else if unordered {
                2
            } else if (i + 1) % lap_len == 0 {
                4
            } else if i == next {
                1
            } else {
                3
            };
            push(Element::at(Kind::Checkpoint(sub), race.checkpoints[i]));
        }
    }
    // deliveries and paybacks wait at their garages (layer 4)
    if places {
        for (uid, kind, p) in PLACES.iter().filter(|x| matches!(x.1, Kind::Delivery | Kind::Payback)) {
            let mut e = Element::at(*kind, Vec3::from(*p));
            e.uid = *uid;
            push(e);
        }
    }
    if let Some(c) = player {
        let mut e = Element::at(Kind::Player, c.state.pos);
        e.back = -c.state.forward();
        push(e);
    }
    layers.into_iter().flatten().collect()
}

/// What the map keeps between frames: its pictures, the minimap target and texture, the clocks
/// (`F-98009`: `HudMap+468` += dt, the strobe position, the pulse phase, the frame count).
pub struct State {
    pub tune: Tune,
    pub assets: Assets,
    pub target: Target,
    pub tex: Option<egui::TextureHandle>,
    pub t: f32,
    pub frame_no: u32,
    pub strobe: f32,
    pub pulse: f32,
    /// the GPS map's camera (for the cursor's size)
    pub gps_cam: Vec3,
    /// the city's surface for the GPS map's x-ray (from the city map, when it is ready)
    pub heights: Option<Heights>,
    /// MAP MODE's street names (NAVSYS2DMOVIE)
    pub street: Option<StreetNames>,
    /// MAP MODE: the road overlay on, and its classes' fades
    pub map_mode: bool,
    pub road_fades: [f32; 5],
    /// the egui pass the clocks last ran in (one tick a frame, whichever map draws)
    last_pass: u64,
    /// the overhead map's 749x430 target "__hud_map" (made when first shown) and its texture
    pub over: Option<Target>,
    pub over_tex: Option<egui::TextureHandle>,
}

impl State {
    pub fn new(data_dir: &str) -> State {
        State { tune: Tune::load(data_dir), assets: Assets::new(data_dir), target: Target::new(220), tex: None, t: 0.0, frame_no: 0, strobe: 0.0, pulse: 0.0, gps_cam: Vec3::new(0.0, 9000.0, 0.0), heights: None, street: None, map_mode: std::env::var("MCLA_MAP_MODE").is_ok(), road_fades: [0.0; 5], last_pass: u64::MAX, over: None, over_tex: None }
    }

    /// The map's tick; `checkpoints` = how many checkpoint elements there are (the strobe runs
    /// over them).
    pub fn tick(&mut self, dt: f32, checkpoints: usize) {
        self.t += dt;
        self.strobe += self.tune.f("StrobeSpeed", 4.6) * dt;
        if self.strobe > (checkpoints as f32 + 1.0) * self.tune.f("StrobePeriod", 1.2) {
            self.strobe = 0.0;
        }
        self.frame_no = self.frame_no.wrapping_add(1);
        self.pulse = (self.pulse + self.tune.f("PulseSpeed", 4.0) * dt) % PI;
    }

    /// `tick` once per egui pass.
    pub fn tick_once(&mut self, ctx: &egui::Context, dt: f32, checkpoints: usize) {
        let pass = ctx.cumulative_pass_nr();
        if pass != self.last_pass {
            self.last_pass = pass;
            self.tick(dt, checkpoints);
        }
    }

    /// Draw the minimap target for frame `f` and hand it out as a texture.
    pub fn minimap(&mut self, ctx: &egui::Context, f: &Frame, els: &[Element], roads: Option<&crate::mapmesh::RoadMap>) -> egui::TextureHandle {
        let q = build(&self.tune, &mut self.assets, f, els, &[], false);
        // (MCLA_HUDMAP_LOG=1: what is on the map, every 300 frames)
        if self.frame_no % 300 == 1 && std::env::var("MCLA_HUDMAP_LOG").is_ok() {
            let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
            for e in els {
                *kinds.entry(format!("{:?}", e.kind)).or_default() += 1;
            }
            log::info!("hudmap: {} elements {:?}, {} draws, {} rim dots, camera y {:.0}", els.len(), kinds, q.draws.len(), q.rim.len(), f.cam_pos.y);
        }
        // (on the GPU, into the same texture: minimap_gpu.rs; MCLA_MINIMAP_CPU=1 the CPU's target)
        if crate::minimap_gpu::enabled() {
            let (w, h) = (self.target.w, self.target.h);
            let tex = self.tex.get_or_insert_with(|| ctx.load_texture("hudmap:minimap", egui::ColorImage::new([w, h], egui::Color32::TRANSPARENT), egui::TextureOptions::LINEAR)).clone();
            crate::minimap_gpu::queue(w, h, &mut self.assets, &self.tune, roads, f, &q, [0.4, 0.4, 0.4], tex.id());
            // (MCLA_SELF_DUMP: the CPU's picture of the same frame beside the GPU's, as the texture holds it)
            if let Ok(dir) = std::env::var("MCLA_SELF_DUMP") {
                render_minimap(&mut self.target, &mut self.assets, &self.tune, roads, f, &q, [0.4, 0.4, 0.4]);
                let img = crate::flash_ui::plain_image(w as u32, h as u32, &self.target.resolve());
                let px: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Ok(file) = std::fs::File::create(format!("{dir}/minimap_cpu.png")) {
                    let mut e = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
                    e.set_color(png::ColorType::Rgba);
                    e.set_depth(png::BitDepth::Eight);
                    let _ = e.write_header().and_then(|mut wr| wr.write_image_data(&px));
                }
            }
            return tex;
        }
        render_minimap(&mut self.target, &mut self.assets, &self.tune, roads, f, &q, [0.4, 0.4, 0.4]);
        let px = self.target.resolve();
        let img = crate::flash_ui::plain_image(self.target.w as u32, self.target.h as u32, &px);
        match self.tex.as_mut() {
            Some(t) => {
                t.set(img, egui::TextureOptions::LINEAR);
                t.clone()
            }
            None => {
                let t = ctx.load_texture("hudmap:minimap", img, egui::TextureOptions::LINEAR);
                self.tex = Some(t.clone());
                t
            }
        }
    }

    /// Draw the overhead map's target "__hud_map" (749x430, mode 2) for frame `f` and hand it out as
    /// a texture (the hudmap movie shows it in `HUD.renderMe`, scaled to its 739 x 499 window).
    pub fn overhead(&mut self, ctx: &egui::Context, f: &Frame, els: &[Element], roads: Option<&crate::mapmesh::RoadMap>) -> egui::TextureHandle {
        let q = build(&self.tune, &mut self.assets, f, els, &[], false);
        // (on the GPU, as the minimap: minimap_gpu.rs; MCLA_MINIMAP_CPU=1 the CPU's target)
        if crate::minimap_gpu::enabled() {
            let tex = self.over_tex.get_or_insert_with(|| ctx.load_texture("hudmap:overhead", egui::ColorImage::new([OVERHEAD_W, OVERHEAD_H], egui::Color32::TRANSPARENT), egui::TextureOptions::LINEAR)).clone();
            crate::minimap_gpu::queue(OVERHEAD_W, OVERHEAD_H, &mut self.assets, &self.tune, roads, f, &q, [0.4, 0.4, 0.4], tex.id());
            // (MCLA_SELF_DUMP: the CPU's picture of the same frame beside the GPU's)
            if let Ok(dir) = std::env::var("MCLA_SELF_DUMP") {
                let t = self.over.get_or_insert_with(|| Target::rect(OVERHEAD_W, OVERHEAD_H));
                render_minimap(t, &mut self.assets, &self.tune, roads, f, &q, [0.4, 0.4, 0.4]);
                let img = crate::flash_ui::plain_image(OVERHEAD_W as u32, OVERHEAD_H as u32, &t.resolve());
                let px: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Ok(file) = std::fs::File::create(format!("{dir}/overhead_cpu.png")) {
                    let mut e = png::Encoder::new(std::io::BufWriter::new(file), OVERHEAD_W as u32, OVERHEAD_H as u32);
                    e.set_color(png::ColorType::Rgba);
                    e.set_depth(png::BitDepth::Eight);
                    let _ = e.write_header().and_then(|mut wr| wr.write_image_data(&px));
                }
            }
            return tex;
        }
        let t = self.over.get_or_insert_with(|| Target::rect(OVERHEAD_W, OVERHEAD_H));
        render_minimap(t, &mut self.assets, &self.tune, roads, f, &q, [0.4, 0.4, 0.4]);
        let img = crate::flash_ui::plain_image(t.w as u32, t.h as u32, &t.resolve());
        match self.over_tex.as_mut() {
            Some(t) => {
                t.set(img, egui::TextureOptions::LINEAR);
                t.clone()
            }
            None => {
                let t = ctx.load_texture("hudmap:overhead", img, egui::TextureOptions::LINEAR);
                self.over_tex = Some(t.clone());
                t
            }
        }
    }

    /// The frame values for mode `mode` from a camera (eye, view_proj) and the player.
    pub fn frame(&self, mode: u8, view_proj: Mat4, eye: Vec3, theta: f32, race: bool, police: i32, target_set: bool, strobe_on: bool) -> Frame {
        let (a, b) = Frame::bases(&self.tune, mode, eye.y);
        Frame {
            mode,
            view_proj,
            cam_pos: eye,
            theta,
            scale_a: a,
            scale_b: b,
            nav_scale: (mode == 1).then_some((7e-5, 0.04)),
            t468: self.t,
            tg: self.t,
            frame_no: self.frame_no,
            race,
            race_checkpoints: false,
            police,
            police0: police,
            target_set,
            strobe: self.strobe,
            pulse: self.pulse,
            cap_lift: false,
            h473: false,
            strobe_on,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The frame

/// Per-frame values every renderer reads (`F-64763`'s record).
#[derive(Clone, Copy)]
pub struct Frame {
    /// 0 minimap, 1 GPS map, 2 overhead map
    pub mode: u8,
    /// world -> clip of the map camera (D3D depth 0..1)
    pub view_proj: Mat4,
    pub cam_pos: Vec3,
    /// the map's turn (`thetaMap`): the player's heading on the minimap, 0 otherwise
    pub theta: f32,
    /// `+144`: ((3 camY + IconScalingOffset) / IconScalingFactor + 1) * 0.3 * GlobalIconScale
    pub scale_a: f32,
    /// `+148`: the checkpoints' base
    pub scale_b: f32,
    /// the GPS map's per-icon base: |p - camera| * k + c (`F-38206`; k 7e-05, c 0.04)
    pub nav_scale: Option<(f32, f32)>,
    /// `HudMap+468` (seconds of map time) and the game clock `0x827D7514`
    pub t468: f32,
    pub tg: f32,
    /// `HudMap+452`: frames drawn
    pub frame_no: u32,
    /// `+153`: a race is current (Race_HasCurrentRace)
    pub race: bool,
    /// `HudMap+24`: the race's checkpoints are being drawn (only the player and checkpoints pass)
    pub race_checkpoints: bool,
    /// `+156`: the player's police state (`F-35540(1)`)
    pub police: i32,
    /// `+160`
    pub police0: i32,
    /// `+180`: a GPS target is set (the minimap then shows the selected elements only)
    pub target_set: bool,
    /// `HudMap+460`: strobe position, `HudMap+464`: pulse phase
    pub strobe: f32,
    pub pulse: f32,
    /// `HudMap+472` lifts the minimap's checkpoint cap; `HudMap+473`
    pub cap_lift: bool,
    pub h473: bool,
    /// `HudMap+508` checkpoint strobe on (`Map_SetCheckpointStrobe`; races set it unless unordered)
    pub strobe_on: bool,
}

/// A mesh to draw (the deferred list's records: mesh, world matrix, colour).
#[derive(Clone)]
pub struct Draw {
    pub mesh: &'static str,
    /// world matrix (column vectors: axes and translation)
    pub m: Mat4,
    pub colour: [f32; 4],
}

/// A dot on the minimap's rim (`F-12952` records).
#[derive(Clone, Copy, Debug)]
pub struct Rim {
    pub angle: f32,
    pub colour: [f32; 4],
    /// 2 flag, 10 flag base, else the checkpoint sphere
    pub kind: u8,
    pub selected: bool,
}

/// `RotY(a)` scaled by `s` with translation `t`: rows (cos a, 0, -sin a), (0, 1, 0), (sin a, 0, cos a).
fn rot_y(a: f32, s: f32, t: Vec3) -> Mat4 {
    let (sn, cs) = a.sin_cos();
    Mat4::from_cols(Vec4::new(cs * s, 0.0, -sn * s, 0.0), Vec4::new(0.0, s, 0.0, 0.0), Vec4::new(sn * s, 0.0, cs * s, 0.0), t.extend(1.0))
}

fn diag(s: f32, t: Vec3) -> Mat4 {
    Mat4::from_cols(Vec4::new(s, 0.0, 0.0, 0.0), Vec4::new(0.0, s, 0.0, 0.0), Vec4::new(0.0, 0.0, s, 0.0), t.extend(1.0))
}

/// The heading of a matrix whose row 2 is `back` (`F-94892`).
pub fn heading(back: Vec3) -> f32 {
    let c = Vec3::new(back.x, 0.0, back.z).normalize_or(Vec3::Z);
    let h = c.z.clamp(-1.0, 1.0).acos();
    if c.x < 0.0 {
        -h
    } else {
        h
    }
}

impl Frame {
    /// The icon bases from the camera height (`F-64763` A-82768..A-68741).
    pub fn bases(tune: &Tune, mode: u8, cam_y: f32) -> (f32, f32) {
        let (off, fac) = (tune.f("IconScalingOffset", -280.0), tune.f("IconScalingFactor", 9000.0));
        let global = tune.f(["GlobalIconScaleMiniMap", "GlobalIconScaleGPSMap", "GlobalIconScaleOverheadMap"][mode.min(2) as usize], 1.0);
        let a = ((cam_y * 3.0 + off) / fac + 1.0) * 0.3 * global;
        let b = if mode == 1 { (cam_y * 7e-5 + 0.04) * 2.5 } else { ((cam_y + off) / fac + 1.0) * global };
        (a, b)
    }

    fn project(&self, p: Vec3) -> Vec3 {
        let c = self.view_proj * p.extend(1.0);
        if c.w.abs() < 1e-6 {
            return Vec3::splat(10.0);
        }
        c.truncate() / c.w
    }

    /// `F-72569`: inside the map when |projected (x, y, z)| < RimIconRadius - the depth (0..1)
    /// counts in the length.
    fn inside(&self, tune: &Tune, p: Vec3) -> bool {
        self.project(p).length() < tune.f("RimIconRadius", 1.25)
    }

    /// The per-icon base (`F+144`, or the GPS map's distance rule).
    fn base(&self, p: Vec3) -> f32 {
        match self.nav_scale {
            Some((k, c)) => (p - self.cam_pos).length() * k + c,
            None => self.scale_a,
        }
    }
}

/// Everything one frame of the map draws: meshes in order, and the rim dots.
#[derive(Default)]
pub struct Queue {
    pub draws: Vec<Draw>,
    pub rim: Vec<Rim>,
}

impl Queue {
    fn mesh(&mut self, mesh: &'static str, m: Mat4, colour: [f32; 4]) {
        // (a draw with alpha 0 is skipped, F-61799)
        if colour[3] > 0.0 {
            self.draws.push(Draw { mesh, m, colour });
        }
    }

    /// `F-40572`: a rim dot at the element's bearing.
    fn rim(&mut self, f: &Frame, p: Vec3, colour: [f32; 4], selected: bool, kind: u8) {
        let q = f.project(p);
        let angle = std::f32::consts::FRAC_PI_2 - q.y.atan2(q.x);
        self.rim.push(Rim { angle, colour: word(colour), kind, selected });
    }
}

const PI: f32 = std::f32::consts::PI;

/// The element walk of one frame (`F-64763` + `F-83806` + the renderers), elements
/// already in layer order. `checkpoints`: the race's own checkpoints (`HudMap+24` provider),
/// drawn after the layers; `circuit` = `HudMap+28`.
pub fn build(tune: &Tune, assets: &mut Assets, f: &Frame, elements: &[Element], checkpoints: &[Vec3], circuit: bool) -> Queue {
    let mut q = Queue::default();
    let mut f = *f;
    let mut cp_count = 0u32;
    for e in elements {
        // (filter 1: while the race draws its checkpoints only checkpoints and the player go on)
        if f.race_checkpoints && !matches!(e.kind, Kind::Checkpoint(_) | Kind::Player) {
            continue;
        }
        if f.race_checkpoints && matches!(e.kind, Kind::Checkpoint(_)) {
            continue;
        }
        // (filter 4: with a GPS target set the minimap keeps the player, cops, gas stations and
        // the selected)
        if !f.race && !matches!(e.kind, Kind::Player | Kind::Cop | Kind::Gas) && f.target_set && f.mode == 0 && !e.selected {
            continue;
        }
        draw_element(tune, assets, &f, e, &mut q, &mut cp_count);
    }
    // the race's checkpoints (A-20285): first = start, second = next, others normal, the last the
    // finish (or the start on a circuit); drawn as in the GPS map (record mode 1)
    if !checkpoints.is_empty() {
        let n = checkpoints.len();
        let mode = f.mode;
        f.mode = 1;
        let mut cp = 0u32;
        for (i, p) in checkpoints.iter().enumerate() {
            let sub = if i + 1 == n && n > 1 {
                if circuit {
                    0
                } else {
                    5
                }
            } else if i == 0 {
                0
            } else if i == 1 {
                1
            } else {
                3
            };
            let mut e = Element::at(Kind::Checkpoint(sub), *p);
            e.colour = [1.0; 4];
            checkpoint(tune, &f, mode, &e, true, &mut q, &mut cp);
        }
    }
    q
}

fn hidden_by_police(f: &Frame) -> bool {
    !f.race && f.police > 1
}

fn draw_element(tune: &Tune, _assets: &mut Assets, f: &Frame, e: &Element, q: &mut Queue, cp_count: &mut u32) {
    let minimap = f.mode == 0;
    // (positions flatten to y = 0 off the GPS map; renderers lift by 1)
    let flat = |p: Vec3| if f.mode != 1 { Vec3::new(p.x, 0.0, p.z) } else { p };
    let lift = |p: Vec3| p + Vec3::Y;
    let t = f.t468;
    match e.kind {
        Kind::Checkpoint(_) => checkpoint(tune, f, f.mode, e, false, q, cp_count),
        Kind::Player | Kind::OtherCar | Kind::Cop | Kind::RedCar => vehicle_arrow(tune, f, e, q),
        Kind::Racer(_) | Kind::PaybackCar => {
            // (racers during a race: the plain arrow in RacerColor, no rim)
            if f.race && matches!(e.kind, Kind::Racer(_)) {
                let mut a = e.clone();
                a.colour = tune.c("RacerColor", [0.529412, 0.796078, 0.0, 1.0]);
                vehicle_arrow_as(tune, f, &a, q, tune.f("IconVehicleScale", 35.0), false);
                return;
            }
            if hidden_by_police(f) {
                return;
            }
            let h = heading(e.back);
            let p = e.pos;
            let mut s0 = f.base(p);
            let mut colour = e.colour;
            let speed = tune.f("HookmanCrossFadeSpeed", 5.0);
            let mission = e.mission && matches!(e.kind, Kind::Racer(_));
            if mission {
                s0 *= 1.0 + 0.05 * (speed * 2.0 * t).sin();
            }
            let s = tune.f("IconVehicleScale", 35.0) * s0;
            let ring = rot_y(h - 3.0 * PI / 4.0, s, lift(p));
            let sub = rot_y(f.theta, s, lift(p));
            let ripple = matches!(e.kind, Kind::PaybackCar) || e.selected;
            if minimap && !f.inside(tune, p) {
                q.rim(f, p, colour, ripple, 1);
                return;
            }
            if ripple {
                let rc = tune.c("RippleColor", [1.0; 4]);
                q.mesh("target_ripple", diag(tune.f("RippleIconOtherScale", 12.0) * s0, lift(p)), mul(rc, colour));
            }
            if mission {
                colour[3] *= 0.8 + 0.2 * ((speed * 2.0 * t).sin() + 1.0) * 0.5;
            }
            q.mesh("icon_ring_car", ring, colour);
            let icon: &'static str = match e.kind {
                Kind::PaybackCar => "icon_payback",
                Kind::Racer(10) => "icon_series",
                _ => ingame(e.icon),
            };
            if mission {
                let a = ((speed * t).sin() + 1.0) * 0.5;
                q.mesh(icon, sub, [1.0, 1.0, 1.0, a]);
                q.mesh("icon_ingame1", sub, [1.0, 1.0, 1.0, 1.0 - a]);
            } else {
                q.mesh(icon, sub, [1.0; 4]);
            }
        }
        Kind::Flag | Kind::FlagBase => {
            let mut colour = if e.flags & 0x08 != 0 {
                tune.c("RedTeamColor", [0.7, 0.2, 0.2, 1.0])
            } else if e.flags & 0x10 != 0 {
                tune.c("BlueTeamColor", [0.2, 0.2, 0.7, 1.0])
            } else {
                tune.c("NeutralTeamColor", [1.0, 0.956863, 0.380392, 1.0])
            };
            let p = lift(flat(e.pos));
            let mut s0 = f.base(e.pos);
            if e.flags & 0x20 != 0 {
                let k = f.pulse.sin().abs();
                for c in 0..3 {
                    colour[c] = (colour[c] + tune.f("PulseColorFactor", 1.0) * k).min(1.0);
                }
                s0 *= 1.0 + tune.f("PulseSizeFactor", 0.25) * k;
            }
            let s = tune.f("IconFlagScale", 60.0) * s0;
            let off = if matches!(e.kind, Kind::Flag) { tune.v3("FlagOffset", Vec3::new(1.4, 0.0, -1.0)) * s } else { Vec3::ZERO };
            // (offset turned with the map, then the position added)
            let (sn, cs) = f.theta.sin_cos();
            let off = Vec3::new(off.x * cs + off.z * sn, off.y, -off.x * sn + off.z * cs);
            let m = rot_y(f.theta, s, p + off);
            if e.selected {
                colour[3] *= (5.0 * f.tg).sin().abs();
            }
            let mesh = if matches!(e.kind, Kind::Flag) { "icon_flag" } else { "icon_flagBase" };
            let kind = e.kind.type_byte();
            if minimap && !f.inside(tune, p + off) {
                q.rim(f, p + off, colour, e.selected, kind);
                return;
            }
            q.mesh(mesh, m, colour);
        }
        Kind::Gas => {
            let p = lift(flat(e.pos));
            let s = tune.f("IconGasStationScale", 100.0) * f.base(e.pos);
            let colour = tune.c("GasStationColor", [0.501961, 0.501961, 0.501961, 1.0]);
            if e.selected {
                q.mesh("target_ripple", diag(tune.f("RippleIconOtherScale", 12.0) * f.base(e.pos), p), mul(tune.c("RippleColor", [1.0; 4]), colour));
            }
            q.mesh("icon_gasStation", rot_y(f.theta, s, p), colour);
        }
        Kind::Garage | Kind::Series | Kind::Wager | Kind::Story | Kind::PinkSlip | Kind::Delivery | Kind::Payback => {
            if hidden_by_police(f) {
                return;
            }
            let p = lift(flat(e.pos));
            let garage_colour = matches!(e.kind, Kind::Garage | Kind::Delivery | Kind::Payback);
            let mut colour = if garage_colour { tune.c("GarageColor", [1.0; 4]) } else { [e.colour[0], e.colour[1], e.colour[2], 1.0] };
            let speed = match e.kind {
                Kind::Garage => tune.f("GarageCrossFadeSpeed", 5.0),
                Kind::Series => tune.f("SeriesCrossFadeSpeed", 5.0),
                Kind::Wager => tune.f("WagerCrossFadeSpeed", 5.0),
                Kind::PinkSlip => tune.f("PinkSlipCrossFadeSpeed", 1.75),
                Kind::Delivery => tune.f("DeliveryCrossFadeSpeed", 1.75),
                Kind::Payback => tune.f("PaybackCrossFadeSpeed", 1.75),
                _ => 5.0,
            };
            let mut s0 = f.base(e.pos);
            if e.mission {
                s0 *= 1.0 + 0.05 * (speed * 2.0 * t).sin();
                colour[3] *= 0.8 + 0.2 * ((speed * 2.0 * t).sin() + 1.0) * 0.5;
            }
            let scale = if garage_colour { tune.f("IconGarageScale", 139.0) } else { tune.f("IconHangoutScale", 139.0) };
            let m = rot_y(f.theta, scale * s0, p);
            if e.selected {
                q.mesh("target_ripple", diag(tune.f("RippleIconOtherScale", 12.0) * s0, p), mul(tune.c("RippleColor", [1.0; 4]), colour));
            }
            q.mesh("icon_ring_hangout", m, colour);
            // the icons: one (section 6), a two-icon cross-fade (7) or a three-icon cycle (8)
            let (one, two, three): (&'static str, Option<(&'static str, &'static str)>, Option<[&'static str; 3]>) = match e.kind {
                Kind::Garage => ("icon_garage", Some(("icon_ingame1", "icon_garage")), None),
                Kind::Series => ("icon_series", Some(("icon_ingame1", "icon_series")), None),
                Kind::Wager => ("icon_ingame6", Some(("icon_ingame1", "icon_ingame6")), None),
                Kind::Story => ("icon_ingame1", None, None),
                Kind::PinkSlip => ("", Some(("icon_pinkslip", "icon_ingame6")), Some(["icon_ingame6", "icon_pinkslip", "icon_ingame1"])),
                Kind::Delivery => ("", Some(("icon_delivery", "icon_garage")), Some(["icon_garage", "icon_delivery", "icon_ingame1"])),
                Kind::Payback => ("", Some(("icon_payback", "icon_garage")), Some(["icon_garage", "icon_payback", "icon_ingame1"])),
                _ => ("icon_ingame1", None, None),
            };
            let three_way = matches!(e.kind, Kind::PinkSlip | Kind::Delivery | Kind::Payback);
            if e.mission && three_way {
                let c = three.unwrap();
                let phi = speed * t / 3.0 - PI / 2.0;
                let (s, co) = phi.sin_cos();
                let (a_mesh, b_mesh, a) = if s < -0.5 {
                    (if co > 0.0 { c[0] } else { c[2] }, c[1], 2.0 * (s + 1.0))
                } else if s < 0.5 {
                    if co > 0.0 {
                        (c[2], c[0], s + 0.5)
                    } else {
                        (c[0], c[2], s + 0.5)
                    }
                } else {
                    (c[1], if co > 0.0 { c[2] } else { c[0] }, 2.0 * (s - 0.5))
                };
                q.mesh(a_mesh, m, [1.0, 1.0, 1.0, a]);
                q.mesh(b_mesh, m, [1.0, 1.0, 1.0, 1.0 - a]);
            } else if (e.mission && !three_way && two.is_some()) || three_way {
                let (r16, r8) = two.unwrap();
                let a = ((speed * t).sin() + 1.0) * 0.5;
                q.mesh(r16, m, [1.0, 1.0, 1.0, a]);
                q.mesh(r8, m, [1.0, 1.0, 1.0, 1.0 - a]);
            } else {
                q.mesh(one, m, [1.0; 4]);
            }
            if minimap && !f.inside(tune, p) {
                q.rim(f, p, colour, e.selected, e.kind.type_byte());
            }
        }
    }
}

fn ingame(n: u8) -> &'static str {
    const N: [&str; 19] = [
        "icon_ingame0", "icon_ingame1", "icon_ingame2", "icon_ingame3", "icon_ingame4", "icon_ingame5", "icon_ingame6", "icon_ingame7", "icon_ingame8", "icon_ingame9",
        "icon_ingame10", "icon_ingame11", "icon_ingame12", "icon_ingame13", "icon_ingame14", "icon_ingame15", "icon_ingame16", "icon_ingame17", "icon_ingame18",
    ];
    N[(n as usize).min(18)]
}

fn mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

/// `F-54162`: the arrow of a vehicle (player, cop, other car, red car).
fn vehicle_arrow(tune: &Tune, f: &Frame, e: &Element, q: &mut Queue) {
    let (scale, rim) = match e.kind {
        // (the player pulses off the minimap: IconPlayerPulseFrequency 12, amplitude 0.05)
        Kind::Player => (tune.f("IconPlayerScale", 139.0) * if f.mode == 0 { 1.0 } else { 1.0 + tune.f("IconPlayerPulseAmplitude", 0.05) * (tune.f("IconPlayerPulseFrequency", 12.0) * f.tg).sin() }, false),
        Kind::RedCar => (tune.f("IconVehicleScale", 35.0), true),
        _ => (tune.f("IconVehicleScale", 35.0), false),
    };
    let mut a = e.clone();
    a.colour = match e.kind {
        Kind::Player => tune.c("HumanColor", [0.45098, 0.960784, 1.0, 1.0]),
        Kind::OtherCar => {
            if e.flags & 0x08 != 0 {
                tune.c("RedTeamColor", [0.7, 0.2, 0.2, 1.0])
            } else if e.flags & 0x10 != 0 {
                tune.c("BlueTeamColor", [0.2, 0.2, 0.7, 1.0])
            } else {
                tune.c("RacerColor", [0.529412, 0.796078, 0.0, 1.0])
            }
        }
        Kind::Cop => cop_colour(tune, f, e),
        Kind::RedCar => {
            if matches!(f.police0, 2 | 5 | 6 | 7 | 8) {
                [((60.0 * f.t468).sin() + 1.0) * 0.375 + 0.25, 0.0, 0.0, 1.0]
            } else {
                tune.c("CopColorFlashA", [1.0, 0.0, 0.0, 1.0])
            }
        }
        _ => e.colour,
    };
    vehicle_arrow_as(tune, f, &a, q, scale, rim);
}

/// `F-40117`: a cop's colour - cruising, or flashing by the police state when pulsing.
fn cop_colour(tune: &Tune, f: &Frame, e: &Element) -> [f32; 4] {
    let cruise = tune.c("CopColorCruising", [0.0, 0.266667, 0.517647, 0.917647]);
    if e.flags & 0x20 == 0 {
        return cruise;
    }
    let (fa, fb) = (tune.c("CopColorFlashA", [1.0, 0.0, 0.0, 1.0]), tune.c("CopColorFlashB", [0.258824, 0.454902, 1.0, 1.0]));
    let black = [0.0, 0.0, 0.0, 1.0];
    let white = [1.0; 4];
    let k = (f.frame_no % 6) as usize;
    match f.police {
        0 | 1 => cruise,
        -1 | 3 => {
            if k < 3 {
                fb
            } else {
                black
            }
        }
        4 => {
            if k < 3 {
                fa
            } else {
                black
            }
        }
        2 | 5 | 6 | 7 => [fb, white, black, fa, white, black][k],
        _ => cruise,
    }
}

fn vehicle_arrow_as(tune: &Tune, f: &Frame, e: &Element, q: &mut Queue, scale: f32, rim: bool) {
    let h = heading(e.back);
    let p = e.pos;
    let s0 = f.base(p);
    let s = scale * s0;
    let m = rot_y(h, s, p + Vec3::Y);
    if rim && f.mode == 0 && !f.inside(tune, p) {
        q.rim(f, p, e.colour, e.selected, 1);
        return;
    }
    if e.selected {
        q.mesh("target_ripple", diag(tune.f("RippleIconOtherScale", 12.0) * s0, p + Vec3::Y), mul(tune.c("RippleColor", [1.0; 4]), e.colour));
    }
    q.mesh("caricon", m, e.colour);
}

/// `F-35159`: a checkpoint sphere. `mode`: the frame's real mode (the race's checkpoints are
/// drawn with the record's mode forced to 1, `provider`).
fn checkpoint(tune: &Tune, f: &Frame, mode: u8, e: &Element, provider: bool, q: &mut Queue, count: &mut u32) {
    let Kind::Checkpoint(sub) = e.kind else { return };
    let first = *count == 0;
    let mut colour = match sub {
        0 => {
            if provider {
                tune.c("CheckpointStartColor", [0.0, 1.0, 0.0, 1.0])
            } else {
                tune.c("CheckpointNextColor", [1.0, 0.941176, 0.0901961, 1.0])
            }
        }
        1 => tune.c("CheckpointNextColor", [1.0, 0.941176, 0.0901961, 1.0]),
        2 => tune.c("CheckpointNextNextColor", [1.0, 0.956863, 0.380392, 1.0]),
        3 => {
            if e.flags & 0x40 != 0 {
                tune.c("RedTeamColor", [0.7, 0.2, 0.2, 1.0])
            } else if e.flags & 0x80 != 0 {
                tune.c("BlueTeamColor", [0.2, 0.2, 0.7, 1.0])
            } else {
                tune.c("CheckpointNormalColor", [1.0, 0.956863, 0.380392, 1.0])
            }
        }
        4 => tune.c("CheckpointLapColor", [0.300781, 0.800781, 0.0, 1.0]),
        _ => tune.c("CheckpointFinishColor", [0.839216, 0.0588235, 0.0588235, 1.0]),
    };
    let ripple_next = tune.f("RippleNextCheckpoint", 1.0) != 0.0;
    // the effect: ripple, pulse, blink or none
    #[derive(PartialEq)]
    enum Fx {
        None,
        Ripple,
        Pulse,
        Blink,
    }
    let fx = match sub {
        0 | 1 => {
            if ripple_next && !provider {
                Fx::Ripple
            } else {
                Fx::Pulse
            }
        }
        4 if first => {
            if ripple_next {
                Fx::Ripple
            } else {
                Fx::Pulse
            }
        }
        5 if first => {
            if f.h473 || ripple_next {
                Fx::Ripple
            } else {
                Fx::Pulse
            }
        }
        5 if f.mode == 1 && !f.strobe_on => Fx::Blink,
        _ => Fx::None,
    };
    let selected = e.selected || first;
    *count += 1;
    let n = *count as f32;
    let b = f.scale_b;
    let mut scale = if sub == 0 { tune.f("IconStartCheckpointScale", 25.0) } else { tune.f("IconCheckpointScale", 20.0) } * b;
    if fx == Fx::Blink {
        colour[3] *= ((tune.f("NextCheckpointPulseSpeed", 4.0) * f.t468).sin() + 1.0) * 0.5;
    } else if (tune.f("StrobeInMiniMap", 0.0) != 0.0 || f.mode != 0) && f.strobe_on && !(sub == 5 && fx == Fx::Ripple) {
        let d = (n - f.strobe).abs();
        if d < 1.0 {
            let k = 1.0 - d;
            scale *= 1.0 + tune.f("StrobeScale", 0.7) * k.powf(tune.f("StrobeExponent", 1.0));
            let add = tune.f("StrobeColorScale", 1.0) * k.powf(tune.f("StrobeColorExponent", 0.4));
            for c in 0..3 {
                colour[c] = (colour[c] + add).min(1.0);
            }
        }
    } else if fx == Fx::Pulse && f.mode == 0 {
        let s = (tune.f("NextCheckpointPulseSpeed", 4.0) * f.t468).sin().abs();
        scale *= 1.0 + tune.f("NextCheckpointPulseSizeFactor", 0.25) * s.powf(tune.f("NextCheckpointScaleExp", 1.0));
        let add = tune.f("NextCheckpointPulseColorFactor", 0.5) * s.powf(tune.f("NextCheckpointColorExp", 1.0));
        for c in 0..3 {
            colour[c] = (colour[c] + add).min(1.0);
        }
    }
    // the minimap's cap on list checkpoints (not the race's own)
    if mode == 0 && f.mode == 0 && n > tune.f("MaxCheckpointsInMinimap", 2.0) && !f.cap_lift {
        return;
    }
    let p = if f.mode != 1 { Vec3::new(e.pos.x, 0.0, e.pos.z) } else { e.pos + Vec3::Y };
    if fx == Fx::Ripple {
        q.mesh("target_ripple", diag(tune.f("RippleIconScale", 12.0) * b, p + Vec3::Y), mul(tune.c("RippleColor", [1.0; 4]), colour));
    }
    if mode == 0 && !f.inside(tune, p) {
        q.rim(f, p, colour, selected || fx == Fx::Ripple, 0);
        return;
    }
    q.mesh("checkpoint_sphere", diag(scale, p), colour);
}

// ---------------------------------------------------------------------------------------------
// The minimap's render target, in software

/// `PS_Ripple` (ui effect `0x13a285e4`, technique of the `target_ripple` mesh): rings running out
/// from the middle. p = uv * 2 - 1, r = |p|; alpha = sat(sin(t * speed - r * freq) * amplitude) *
/// (1 - r)^falloff * colour.a (Xenos log2 / exp2), colour = gSimpleHue. Recorded constants
/// (`work/mm/cap/gps_tgt2.bin`): freq 19.5 = RippleFrequency, speed 16 = RippleSpeed, falloff 0.4 =
/// RippleExponent, amplitude 1.37 (RippleAmplitude 5 eased to 1.37 by the camera's distance / 75),
/// t = the game clock.
#[derive(Clone, Copy)]
pub struct Ripple {
    pub t: f32,
    pub freq: f32,
    pub speed: f32,
    pub falloff: f32,
    pub amplitude: f32,
}

impl Ripple {
    pub fn new(tune: &Tune, t: f32, cam_dist: f32) -> Ripple {
        let a = tune.f("RippleAmplitude", 8.0);
        Ripple { t, freq: tune.f("RippleFrequency", 24.0), speed: tune.f("RippleSpeed", 20.0), falloff: tune.f("RippleExponent", 0.4), amplitude: a + (1.37 - a) * (cam_dist / 75.0).min(1.0) }
    }

    pub fn alpha(&self, u: f32, v: f32) -> f32 {
        let (x, y) = (u * 2.0 - 1.0, v * 2.0 - 1.0);
        let r = (x * x + y * y).sqrt();
        let ph = (self.t * self.speed - r * self.freq) * 0.159155 + 0.5;
        let w = ((ph - ph.floor()) * std::f32::consts::TAU - PI).sin();
        let fall = (1.0 - r).max(0.0).powf(self.falloff);
        (w * self.amplitude).clamp(0.0, 1.0) * fall
    }
}

/// The map's render target with four samples per pixel (the original's 4x MSAA: surface field 2): 220x220
/// "__mini_map" for the minimap, 749x430 "__hud_map" for the overhead map (`F-76527`).
pub struct Target {
    pub w: usize,
    pub h: usize,
    buf: Vec<[f32; 4]>,
    /// the overhead map's roads (its camera stands still once the movie's easing settles): the
    /// samples after the roads, and the view they were drawn from
    roads: Option<([u32; 16], Vec<[f32; 4]>)>,
    /// the mask pass's values per sample (they depend on the mask picture and the target's size only),
    /// by the picture they were read from
    mask_m: Option<(usize, Vec<f32>)>,
}

/// The x range (pixels, one more on each side for the rounding of the exact test) where the triangle `v`
/// meets the band of rows `b0..b1`; None when it does not.
fn band_span(v: &[(f32, f32, f32); 3], b0: f64, b1: f64) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for i in 0..3 {
        let (p, q) = (v[i], v[(i + 1) % 3]);
        let (px, py, qx, qy) = (p.0 as f64, p.1 as f64, q.0 as f64, q.1 as f64);
        if py >= b0 && py <= b1 {
            lo = lo.min(px);
            hi = hi.max(px);
        }
        if py != qy {
            for b in [b0, b1] {
                let t = (b - py) / (qy - py);
                if (0.0..=1.0).contains(&t) {
                    let x = px + (qx - px) * t;
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
        }
    }
    (lo <= hi).then(|| (lo.floor() - 1.0, hi.ceil() + 1.0))
}

/// Where the four samples of a pixel lie (pixel corner = 0). *(approx.)* The console's own 4x positions are not
/// decoded; these are D3D's standard 4x pattern (-2,-6) (6,-2) (-6,2) (2,6) / 16 px, which the recomp's captures use.
const SAMPLES: [(f32, f32); 4] = [(0.375, 0.125), (0.875, 0.375), (0.125, 0.625), (0.625, 0.875)];
const NS: usize = SAMPLES.len();

#[derive(Clone, Copy, PartialEq)]
enum Blend {
    /// colour: SRC_ALPHA / INV_SRC_ALPHA; alpha: max (RB_BLENDCONTROL 0x01610706)
    Alpha,
}

impl Target {
    pub fn new(size: usize) -> Target {
        Target::rect(size, size)
    }

    pub fn rect(w: usize, h: usize) -> Target {
        Target { w, h, buf: vec![[0.0; 4]; w * h * NS], roads: None, mask_m: None }
    }

    pub fn clear(&mut self, c: [f32; 4]) {
        self.buf.iter_mut().for_each(|p| *p = c);
    }

    /// One textured triangle; `v` = (pixel x, y, 1/w), `uv`.
    fn tri(&mut self, v: [(f32, f32, f32); 3], uv: [Vec2; 3], tex: Option<&Img>, colour: [f32; 4], _blend: Blend, ripple: Option<&Ripple>) {
        let (nw, nh) = (self.w as f32, self.h as f32);
        let (x0, y0, x1, y1) = (v[0].0.min(v[1].0).min(v[2].0), v[0].1.min(v[1].1).min(v[2].1), v[0].0.max(v[1].0).max(v[2].0), v[0].1.max(v[1].1).max(v[2].1));
        if x1 < 0.0 || y1 < 0.0 || x0 >= nw || y0 >= nh {
            return;
        }
        let area = (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[2].0 - v[0].0) * (v[1].1 - v[0].1);
        if area.abs() < 1e-9 {
            return;
        }
        // level of detail: texels per pixel over the triangle
        let lod = tex.map_or(0.0, |t| {
            let ua = ((uv[1].x - uv[0].x) * (uv[2].y - uv[0].y) - (uv[2].x - uv[0].x) * (uv[1].y - uv[0].y)).abs() * (t.w * t.h) as f32;
            if ua > 0.0 {
                0.5 * (ua / area.abs()).log2()
            } else {
                0.0
            }
        });
        let (px0, py0) = (x0.floor().max(0.0) as usize, y0.floor().max(0.0) as usize);
        let (px1, py1) = ((x1.ceil() as usize).min(self.w - 1), (y1.ceil() as usize).min(self.h - 1));
        let inv = 1.0 / area;
        // (each row of the box on its own - rows share no samples, so they run side by side with the same
        // result; only the pixels the triangle can reach in the row - docs/PERFORMANCE.md, 2026-10-23)
        let row = |py: usize, out: &mut [[f32; 4]]| {
            let Some((lo, hi)) = band_span(&v, py as f64, py as f64 + 1.0) else { return };
            let (sx0, sx1) = ((lo.max(px0 as f64)) as usize, (hi.min(px1 as f64)) as usize);
            for px in sx0..=sx1.min(px1) {
                for (s, (sx, sy)) in SAMPLES.iter().enumerate() {
                    let (x, y) = (px as f32 + sx, py as f32 + sy);
                    let w0 = ((v[1].0 - x) * (v[2].1 - y) - (v[2].0 - x) * (v[1].1 - y)) * inv;
                    let w1 = ((v[2].0 - x) * (v[0].1 - y) - (v[0].0 - x) * (v[2].1 - y)) * inv;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    // perspective-correct texture coordinates
                    let (a, b, c) = (w0 * v[0].2, w1 * v[1].2, w2 * v[2].2);
                    let sum = a + b + c;
                    let (u, vv) = ((a * uv[0].x + b * uv[1].x + c * uv[2].x) / sum, (a * uv[0].y + b * uv[1].y + c * uv[2].y) / sum);
                    let t = match ripple {
                        Some(r) => [1.0, 1.0, 1.0, r.alpha(u, vv)],
                        None => tex.map_or([1.0; 4], |t| t.sample(u, vv, lod)),
                    };
                    let src = [t[0] * colour[0], t[1] * colour[1], t[2] * colour[2], t[3] * colour[3]];
                    let d = &mut out[px * NS + s];
                    for k in 0..3 {
                        d[k] = src[k] * src[3] + d[k] * (1.0 - src[3]);
                    }
                    d[3] = d[3].max(src[3]);
                }
            }
        };
        let rl = self.w * NS;
        let rows = &mut self.buf[py0 * rl..(py1 + 1) * rl];
        if (px1 + 1 - px0) * (py1 + 1 - py0) >= 4096 {
            use rayon::prelude::*;
            mcla_flash::raster::pool().install(|| rows.par_chunks_mut(rl).enumerate().for_each(|(i, r)| row(py0 + i, r)));
        } else {
            rows.chunks_mut(rl).enumerate().for_each(|(i, r)| row(py0 + i, r));
        }
    }

    /// A mesh through `world` and `view_proj` (clip -> pixels: x right, y down).
    pub fn mesh(&mut self, mesh: &IconMesh, world: Mat4, view_proj: Mat4, colour: [f32; 4], ripple: Option<&Ripple>) {
        let (hw, hh) = (self.w as f32 * 0.5, self.h as f32 * 0.5);
        let wvp = view_proj * world;
        for part in &mesh.parts {
            let pts: Vec<Option<(f32, f32, f32)>> = part
                .pos
                .iter()
                .map(|p| {
                    let c = wvp * p.extend(1.0);
                    (c.w > 1e-4).then(|| (hw + hw * c.x / c.w, hh - hh * c.y / c.w, 1.0 / c.w))
                })
                .collect();
            for t in part.idx.chunks_exact(3) {
                let (Some(a), Some(b), Some(c)) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]) else { continue };
                let uv = |i: u32| part.uv.get(i as usize).copied().unwrap_or(Vec2::ZERO);
                self.tri([a, b, c], [uv(t[0]), uv(t[1]), uv(t[2])], part.tex.as_deref(), colour, Blend::Alpha, ripple);
            }
        }
    }

    /// Flat triangles on the ground (the road meshes: identity world, white texture).
    pub fn ground(&mut self, tris: &[[Vec3; 3]], view_proj: Mat4, colour: [f32; 4]) {
        let (hw, hh) = (self.w as f32 * 0.5, self.h as f32 * 0.5);
        for t in tris {
            let mut v = [(0.0, 0.0, 0.0); 3];
            let mut ok = true;
            for k in 0..3 {
                let c = view_proj * t[k].extend(1.0);
                if c.w <= 1e-4 {
                    ok = false;
                    break;
                }
                v[k] = (hw + hw * c.x / c.w, hh - hh * c.y / c.w, 1.0 / c.w);
            }
            if ok {
                self.tri(v, [Vec2::ZERO; 3], None, colour, Blend::Alpha, None);
            }
        }
    }

    /// The mask pass: alpha minus (1 - mask), the mask's red channel over the whole target
    /// (`AlphaModulate`, uv clamped to 0.05..0.95, reverse subtract).
    pub fn mask(&mut self, mask: &Img) {
        // (the mask's value at each sample, read once for this picture and size)
        let key = mask.rgba8.as_ptr() as usize;
        if self.mask_m.as_ref().map_or(true, |(k, m)| *k != key || m.len() != self.buf.len()) {
            let (nw, nh) = (self.w as f32, self.h as f32);
            let mut m = Vec::with_capacity(self.buf.len());
            for py in 0..self.h {
                for px in 0..self.w {
                    for (sx, sy) in SAMPLES.iter() {
                        let u = ((px as f32 + sx) / nw).clamp(0.05, 0.95);
                        let v = ((py as f32 + sy) / nh).clamp(0.05, 0.95);
                        m.push(mask.sample(u, v, 0.0)[0]);
                    }
                }
            }
            self.mask_m = Some((key, m));
        }
        let m = &self.mask_m.as_ref().unwrap().1;
        for (d, m) in self.buf.iter_mut().zip(m) {
            d[3] = (d[3] - (1.0 - m)).max(0.0);
        }
    }

    /// The rim dots, after the mask (`F-88946` -> `F-42900`): unselected from the last to
    /// the first, then the selected; a quad of the mesh scaled by 6 at 90 px from the centre.
    pub fn rim(&mut self, assets: &mut Assets, rim: &[Rim], tg: f32) {
        let (cx, cy) = (self.w as f32 * 0.5, self.h as f32 * 0.5);
        for sel in [false, true] {
            for r in rim.iter().rev().filter(|r| r.selected == sel) {
                let name = match r.kind {
                    2 => "icon_flag",
                    10 => "icon_flagBase",
                    _ => "checkpoint_sphere",
                };
                let Some(mesh) = assets.mesh(name) else { continue };
                let (sn, cs) = r.angle.sin_cos();
                let (ox, oz) = (90.0 * sn, 90.0 * cs);
                let a = if r.selected { ((8.0 * tg).sin() + 1.0) * 0.5 } else { 1.0 };
                let colour = [r.colour[0], r.colour[1], r.colour[2], a];
                for part in &mesh.parts {
                    let pts: Vec<(f32, f32, f32)> = part.pos.iter().map(|p| (cx + ox + 6.0 * p.x, cy - oz + 6.0 * p.z, 1.0)).collect();
                    for t in part.idx.chunks_exact(3) {
                        let uv = |i: u32| part.uv.get(i as usize).copied().unwrap_or(Vec2::ZERO);
                        self.tri([pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]], [uv(t[0]), uv(t[1]), uv(t[2])], part.tex.as_deref(), colour, Blend::Alpha, None);
                    }
                }
            }
        }
    }

    /// The resolved picture (the samples averaged), straight RGBA 8-bit.
    pub fn resolve(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.w * self.h * 4);
        for p in self.buf.chunks_exact(NS) {
            for k in 0..4 {
                out.push(((p.iter().map(|s| s[k]).sum::<f32>() / NS as f32).clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
        }
        out
    }
}

/// The road layers in the order and colours the map draws them (`F-85917`).
pub const ROAD_COLOURS: [(&str, [f32; 4]); 5] = [
    ("MinorRoadColor", [0.0470588, 0.0862745, 0.101961, 1.0]),
    ("MainRoadColor", [0.0470588, 0.0862745, 0.101961, 1.0]),
    ("MajorRoadColor", [0.0470588, 0.0862745, 0.101961, 1.0]),
    ("ShortcutColor", [0.0470588, 0.0862745, 0.101961, 1.0]),
    ("FreewayColor", [0.0470588, 0.0862745, 0.101961, 1.0]),
];

/// The minimap's camera (`target0_cam` in coordinate frame 1, `F-15166`): `eye` and `look`
/// in the frame turned by the heading `h` about the car's (x, 0, z); 35 degree lens, square,
/// near 10, far 9000.
pub fn minimap_view_proj(car: Vec3, h: f32, eye: Vec3, look: Vec3, fovy_deg: f32) -> (Mat4, Vec3) {
    let frame = rot_y(h, 1.0, Vec3::new(car.x, 0.0, car.z));
    let e = frame.transform_point3(eye);
    let l = frame.transform_point3(look);
    let view = Mat4::look_at_rh(e, l, Vec3::Y);
    let proj = Mat4::perspective_rh(fovy_deg.to_radians(), 1.0, 10.0, 9000.0);
    (proj * view, e)
}

/// The overhead map's target "__hud_map" (`F-76527`: 749x430, 32 bpp).
pub const OVERHEAD_W: usize = 749;
pub const OVERHEAD_H: usize = 430;

/// The overhead map's camera (mode 2, rule_frame.md 9): the hudmap movie's `target0_cam` in
/// coordinate frame 0 = the world (`target0_cam.coord = 0`), so it stands still over the city,
/// north up. South Central's city ("sc", the movie's `SC_map` 1 - the port's city): eye
/// (-300, 7400, 0) looking at (-300, 0, -180) as the movie asks, (-299, 7399, 1) -> (-299, 0, -179)
/// after its easing (`pos += int((cam - pos) / 2)` stops one short; z keeps the 1 left from the
/// minimap's easing down from 2500); the other city's (`SC_map` 0) (-300, 6000, 0) -> (-300, 0, -610).
/// Lens `target0_cam.fovy` (35), aspect `target0.aspect` = the movie's viewportWindow width / height
/// (widescreen: 740 x 500 asked, 739 / 499 settled), near 10, far 9000 (`F-27685` ->
/// `F-79482`); the up vector (0, 1, 0) (`F-97951`). Recorded from the original
/// (`work/mm/cap/ov_free1.bin`): eye (-299, 7399, 1), x scale 2.14158 = 3.17159 / (739 / 499).
pub fn overhead_view_proj(eye: Vec3, look: Vec3, fovy_deg: f32, aspect: f32) -> Mat4 {
    Mat4::perspective_rh(fovy_deg.to_radians(), aspect.max(0.1), 10.0, 9000.0) * Mat4::look_at_rh(eye, look, Vec3::Y)
}

/// `MCLA_OLD_BIGMAP=1`: the port's previous big map (its own minimap drawing, larger) instead of
/// the original's overhead map.
pub fn old_bigmap() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_OLD_BIGMAP").is_ok())
}

/// The delivery start scenes' generic events `big_map_up` (88) and `big_map_down` (89) - they send
/// the HUD mapGoBig / mapGoNormal (`F-91263`): scene, up and down in scene seconds
/// (`gen_prerace_delivery_1x1_0N_generated.sco`; `bookmark_cut_to_game` at the up time, the
/// countdown event 0.17 s after the down).
pub const CINE_BIG_MAP: [(&str, f32, f32); 3] = [("gen_prerace_delivery_1x1_01", 6.1, 9.1), ("gen_prerace_delivery_1x1_02", 5.5, 8.5), ("gen_prerace_delivery_1x1_03", 6.0, 9.0)];

/// Is the overhead map up at `t` seconds into the cine scene `scene`?
pub fn cine_big_map(scene: &str, t: f32) -> bool {
    CINE_BIG_MAP.iter().any(|(n, up, down)| scene.ends_with(n) && t >= *up && t < *down)
}

/// A delivery's intro scene (`racestart/gen_prerace_delivery_1x1_0N`) and its generic events in
/// scene seconds (`gen_prerace_delivery_1x1_0N_generated.sco`): `racename` (67: the race's badge,
/// form 70) at `racename` for `racename_len`, `bookmark_cut_to_game` (72: bookmark + cut_to_game -
/// the game camera from here on) with `big_map_up` at `up`, `big_map_down` at `down`, `countdown`
/// (58: starts `game/racetypes/countdown_321Go_3sec`, GO 3 s later = the camera's end) at
/// `countdown`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeliveryIntro {
    pub scene: &'static str,
    pub racename: f32,
    pub racename_len: f32,
    pub up: f32,
    pub down: f32,
    pub countdown: f32,
}

pub const DELIVERY_INTROS: [DeliveryIntro; 3] = [
    DeliveryIntro { scene: "racestart/gen_prerace_delivery_1x1_01", racename: 2.0, racename_len: 2.0, up: 6.1, down: 9.1, countdown: 9.26667 },
    DeliveryIntro { scene: "racestart/gen_prerace_delivery_1x1_02", racename: 2.66667, racename_len: 2.0, up: 5.5, down: 8.5, countdown: 8.66667 },
    DeliveryIntro { scene: "racestart/gen_prerace_delivery_1x1_03", racename: 2.0, racename_len: 2.0, up: 6.0, down: 9.0, countdown: 9.16667 },
];

/// The intro scene `scene` is, if it is one.
pub fn delivery_intro(scene: &str) -> Option<DeliveryIntro> {
    DELIVERY_INTROS.iter().copied().find(|e| scene.ends_with(e.scene))
}

/// The intro of each delivery leg: delivery_hangout_core (`Hangouts/0x37c63b19` 0x14CC)
/// `RAND_INT_RANGE(0, 2)` - both ends in - 0 -> 01, 1 -> 02, else 03 (test hook MCLA_DELIVERY_INTRO=n).
pub fn delivery_intro_pick(r: u32) -> DeliveryIntro {
    let n = std::env::var("MCLA_DELIVERY_INTRO").ok().and_then(|v| v.parse::<usize>().ok()).map_or(r as usize % 3, |n| n.clamp(1, 3) - 1);
    DELIVERY_INTROS[n]
}

/// Render the whole map target: roads, queued icons, mask, rim dots.
pub fn render_minimap(target: &mut Target, assets: &mut Assets, tune: &Tune, roads: Option<&crate::mapmesh::RoadMap>, f: &Frame, q: &Queue, clear_rgb: [f32; 3]) {
    target.clear([clear_rgb[0], clear_rgb[1], clear_rgb[2], 0.0]);
    // (no road is drawn in a frame without an icon - F-85917)
    if !q.draws.is_empty() || !q.rim.is_empty() {
        // the overhead map: every road triangle of the city (the view takes it all in); drawn again
        // only when the camera moves - the movie eases it in over a dozen frames, then it stands
        let key = (f.mode == 2).then(|| f.view_proj.to_cols_array().map(f32::to_bits));
        let cached = match (key, target.roads.take()) {
            (Some(k), Some((ck, b))) if ck == k && b.len() == target.buf.len() => {
                target.buf.copy_from_slice(&b);
                target.roads = Some((ck, b));
                true
            }
            _ => false,
        };
        if let (Some(roads), false, Some(k)) = (roads, cached, key) {
            for (layer, (ckey, d)) in roads.layers.iter().zip(ROAD_COLOURS) {
                let colour = tune.c(ckey, d);
                if colour[3] <= 0.0 {
                    continue;
                }
                let tris: Vec<[Vec3; 3]> = layer
                    .tris
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let y = layer.ys.get(i).copied().unwrap_or([0.0; 3]);
                        [Vec3::new(t[0].x, y[0], t[0].y), Vec3::new(t[1].x, y[1], t[1].y), Vec3::new(t[2].x, y[2], t[2].y)]
                    })
                    .collect();
                target.ground(&tris, f.view_proj, colour);
            }
            target.roads = Some((k, target.buf.clone()));
        }
        if let (Some(roads), None) = (roads, key) {
            // the triangles under the camera's view (it reaches 1891 x tan 17.5 = 596 m either side
            // at rest, 2401 x tan 17.5 = 757 m at full height; the look point is 110 m ahead)
            let reach = f.cam_pos.y * 0.33 + 160.0;
            let centre = Vec2::new(f.cam_pos.x, f.cam_pos.z);
            let mut ids = Vec::new();
            let mut tris: Vec<[Vec3; 3]> = Vec::new();
            for (layer, (key, d)) in roads.layers.iter().zip(ROAD_COLOURS) {
                let colour = tune.c(key, d);
                if colour[3] <= 0.0 {
                    continue;
                }
                layer.near(centre, reach, &mut ids);
                tris.clear();
                tris.extend(ids.iter().map(|&i| {
                    let t = layer.tris[i as usize];
                    let y = layer.ys.get(i as usize).copied().unwrap_or([0.0; 3]);
                    [Vec3::new(t[0].x, y[0], t[0].y), Vec3::new(t[1].x, y[1], t[1].y), Vec3::new(t[2].x, y[2], t[2].y)]
                }));
                target.ground(&tris, f.view_proj, colour);
            }
        }
        for d in &q.draws {
            if let Some(m) = assets.mesh(d.mesh) {
                let ripple = (d.mesh == "target_ripple").then(|| Ripple::new(tune, f.tg, (d.m.w_axis.truncate() - f.cam_pos).length()));
                target.mesh(&m, d.m, f.view_proj, d.colour, ripple.as_ref());
            }
        }
    }
    // (the mask of the mode: mask0 the disc for the minimap; the overhead map's mask2 is opaque
    // white - nothing taken away)
    if f.mode == 0 {
        if let Some(mask) = assets.mask0.take() {
            target.mask(&mask);
            assets.mask0 = Some(mask);
        }
    }
    target.rim(assets, &q.rim, f.tg);
}

/// The city's surface (the city map's coarse relief: building tops every 6 m) for the GPS map's
/// x-ray: the original draws the overlay and the icons against the scene's depth and at 25 %
/// (`gXrayTerm`) where geometry is in front (PS_TexturedAlternateDepth). The port tests each vertex:
/// behind the city when the ray from it to the camera passes under the surface (approx.: per
/// vertex, 6 m cells, instead of per pixel against the depth buffer).
#[derive(Clone)]
pub struct Heights {
    pub min_x: f32,
    pub min_z: f32,
    pub cell: f32,
    pub gw: usize,
    pub gh: usize,
    pub h: Arc<Vec<f32>>,
    /// the tallest roof in each 8 x 8 cells (built on first use), for the ray march to skip open ground
    pub coarse: Arc<std::sync::OnceLock<Vec<f32>>>,
}

const XRAY_BLOCK: usize = 8;

impl Heights {
    pub fn at(&self, x: f32, z: f32) -> f32 {
        let gx = (((x - self.min_x) / self.cell).round().max(0.0) as usize).min(self.gw - 1);
        let gz = (((z - self.min_z) / self.cell).round().max(0.0) as usize).min(self.gh - 1);
        self.h[gz * self.gw + gx]
    }

    fn coarse(&self) -> &Vec<f32> {
        self.coarse.get_or_init(|| {
            let (cw, ch) = (self.gw.div_ceil(XRAY_BLOCK), self.gh.div_ceil(XRAY_BLOCK));
            let mut c = vec![f32::MIN; cw * ch];
            for z in 0..self.gh {
                for x in 0..self.gw {
                    let k = (z / XRAY_BLOCK) * cw + x / XRAY_BLOCK;
                    c[k] = c[k].max(self.h[z * self.gw + x]);
                }
            }
            c
        })
    }

    /// the tallest roof within the coarse block around (x, z), its neighbours included (the fine test
    /// rounds to the nearest cell, so a point may read a cell of the next block)
    fn coarse_at(&self, x: f32, z: f32) -> f32 {
        let c = self.coarse();
        let (cw, ch) = (self.gw.div_ceil(XRAY_BLOCK), self.gh.div_ceil(XRAY_BLOCK));
        let bx = (((x - self.min_x) / self.cell).max(0.0) as usize / XRAY_BLOCK).min(cw - 1);
        let bz = (((z - self.min_z) / self.cell).max(0.0) as usize / XRAY_BLOCK).min(ch - 1);
        let mut m = f32::MIN;
        for zz in bz.saturating_sub(1)..=(bz + 1).min(ch - 1) {
            for xx in bx.saturating_sub(1)..=(bx + 1).min(cw - 1) {
                m = m.max(c[zz * cw + xx]);
            }
        }
        m
    }

    /// Is `p` hidden from `cam` by the city?
    pub fn hidden(&self, p: Vec3, cam: Vec3) -> bool {
        let d = cam - p;
        let flat = Vec2::new(d.x, d.z).length();
        // (march until the ray is above the tallest roof, ~330 m, one cell at a time)
        let top = 330.0f32;
        if p.y >= top {
            return false;
        }
        let rise = d.y.max(1e-3);
        let span = if flat < 1e-3 { 0.0 } else { ((top - p.y) / rise * flat).min(flat) };
        let n = ((span / self.cell).ceil() as usize).min(160);
        let mut k = 1;
        while k <= n.max(1) {
            let f = (k as f32 * self.cell / flat.max(1e-3)).min(1.0);
            let q = p + d * f;
            // (a whole block of steps over open ground: the ray only rises, so if it is above the
            // tallest roof around here now it stays above it for the block's steps)
            if k % XRAY_BLOCK == 1 && q.y >= self.coarse_at(q.x, q.z) && k + XRAY_BLOCK <= n {
                let fe = ((k + XRAY_BLOCK - 1) as f32 * self.cell / flat.max(1e-3)).min(1.0);
                let qe = p + d * fe;
                if qe.y >= self.coarse_at(qe.x, qe.z) {
                    k += XRAY_BLOCK;
                    continue;
                }
            }
            if q.y < self.at(q.x, q.z) - 0.5 {
                return true;
            }
            if f >= 1.0 {
                break;
            }
            k += 1;
        }
        flat < 1e-3 && p.y < self.at(p.x, p.z) - 0.5
    }
}

const XRAY: f32 = 0.25;

/// Cut a projected mesh's triangles into pieces of about 8 px on screen (positions and uvs taken in
/// the model's space, then projected, so the perspective holds; at most 6 x 6 a triangle) and
/// colour each piece by the x-ray test of its corners (hidden when two of the three are): a hard
/// edge where a building covers part of an icon. The grid's points are tested once each.
#[allow(clippy::too_many_arguments)]
fn xray_split(mesh: &egui::Mesh, part: &Part, m: &Mat4, wvp: &Mat4, (c, hw, hh): (egui::Pos2, f32, f32), hf: &Heights, cam: Vec3, (col, dim): (egui::Color32, egui::Color32)) -> egui::Mesh {
    let mut out = egui::Mesh::with_texture(mesh.texture_id);
    for t in mesh.indices.chunks_exact(3) {
        let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let (s0, s1, s2) = (mesh.vertices[i0].pos, mesh.vertices[i1].pos, mesh.vertices[i2].pos);
        let edge = s0.distance(s1).max(s1.distance(s2)).max(s2.distance(s0));
        let n = ((edge / 8.0).ceil() as usize).clamp(1, 6);
        if n == 1 {
            // (a small triangle: its corners' own test, as tested already)
            let base = out.vertices.len() as u32;
            out.vertices.extend([mesh.vertices[i0], mesh.vertices[i1], mesh.vertices[i2]]);
            out.indices.extend([base, base + 1, base + 2]);
            continue;
        }
        let (p0, p1, p2) = (part.pos[i0], part.pos[i1], part.pos[i2]);
        let uv = |i: usize| part.uv.get(i).copied().unwrap_or(Vec2::ZERO);
        let (u0, u1, u2) = (uv(i0), uv(i1), uv(i2));
        // grid point (a, b), a + b <= n: weights (n - a - b, a, b) / n
        let row = |a: usize| a * (2 * n + 3 - a) / 2;
        let mut pts: Vec<Option<(egui::Pos2, Vec2, bool)>> = Vec::with_capacity((n + 1) * (n + 2) / 2);
        for a in 0..=n {
            for b in 0..=(n - a) {
                let (wa, wb) = (a as f32 / n as f32, b as f32 / n as f32);
                let w0 = 1.0 - wa - wb;
                let p = p0 * w0 + p1 * wa + p2 * wb;
                let cl = *wvp * p.extend(1.0);
                pts.push((cl.w > 1e-4).then(|| {
                    let u = u0 * w0 + u1 * wa + u2 * wb;
                    (egui::pos2(c.x + hw * cl.x / cl.w, c.y - hh * cl.y / cl.w), u, hf.hidden(m.transform_point3(p), cam))
                }));
            }
        }
        let mut piece = |q: [(usize, usize); 3]| {
            let mut v = [(egui::Pos2::ZERO, Vec2::ZERO, false); 3];
            for (k, &(a, b)) in q.iter().enumerate() {
                match pts[row(a) + b] {
                    Some(x) => v[k] = x,
                    None => return,
                }
            }
            let colour = if v.iter().filter(|x| x.2).count() >= 2 { dim } else { col };
            let base = out.vertices.len() as u32;
            for (pos, u, _) in v {
                out.vertices.push(egui::epaint::Vertex { pos, uv: egui::pos2(u.x, u.y), color: colour });
            }
            out.indices.extend([base, base + 1, base + 2]);
        };
        for a in 0..n {
            for b in 0..(n - a) {
                piece([(a, b), (a + 1, b), (a, b + 1)]);
                if a + b + 1 < n {
                    piece([(a + 1, b), (a + 1, b + 1), (a, b + 1)]);
                }
            }
        }
    }
    out
}

/// MAP MODE's road overlay on the GPS map (mcSimpleMap, `F-25904`; rule_frame.md 4.4): the
/// same five road meshes, lifted 2 + 98 t (t = the camera's height fraction), in the order shortcuts,
/// small, main, large, freeways. Colours and alphas as recorded with every class faded in
/// (`work/mm/cap/mapmode_low.bin`): shortcuts (1, 1, 1, 0.4), small (0.9, 0.9, 0.5, 0.4), main and
/// large (0.99, 0.99, 0.55, 0.436), freeways (0.8, 0.6, 0.1, 0.4). Each class's alpha fades by 0.1 a
/// frame toward 1 while the camera is under its height (shortcuts 1700, small 3000, main 5000,
/// large and freeways always) and toward 0 above it.
pub const OVERLAY: [(usize, [f32; 4], f32); 5] = [
    (3, [1.0, 1.0, 1.0, 0.4], 1700.0),
    (0, [0.9, 0.9, 0.5, 0.4], 3000.0),
    (1, [0.99, 0.99, 0.55, 0.436], 5000.0),
    (2, [0.99, 0.99, 0.55, 0.436], 100000.0),
    (4, [0.8, 0.6, 0.1, 0.4], 100000.0),
];

/// One frame of the overlay's fades (`fades` per OVERLAY entry, 30 steps a second).
pub fn overlay_fade(fades: &mut [f32; 5], cam_y: f32, dt: f32) {
    for (k, (_, _, top)) in OVERLAY.iter().enumerate() {
        let step = 0.1 * dt * 30.0;
        fades[k] = if cam_y < *top { (fades[k] + step).min(1.0) } else { (fades[k] - step).max(0.0) };
    }
}

pub fn paint_roads(painter: &egui::Painter, roads: &crate::mapmesh::RoadMap, view_proj: Mat4, screen: egui::Rect, cam: Vec3, fades: &[f32; 5], heights: Option<&Heights>) {
    let (hw, hh) = (screen.width() * 0.5, screen.height() * 0.5);
    let c = screen.center();
    let t = ((cam.y - NAV_LOW) / (NAV_HIGH - NAV_LOW)).clamp(0.0, 1.0);
    let lift = 2.0 + 98.0 * t;
    // (what the camera sees: its look point and the screen's reach on the ground, with room)
    let reach = cam.y * 0.45 + nav_off(cam.y) + 600.0;
    let centre = Vec2::new(cam.x, cam.z - nav_off(cam.y));
    let mut ids = Vec::new();
    for (k, (layer, col, _)) in OVERLAY.iter().enumerate() {
        let a = col[3] * fades[k];
        if a <= 0.01 {
            continue;
        }
        let Some(l) = roads.layers.get(*layer) else { continue };
        let colour = crate::flash_ui::plain_color((col[0] * 255.0) as u8, (col[1] * 255.0) as u8, (col[2] * 255.0) as u8, (a * 255.0 + 0.5) as u8);
        let dim = crate::flash_ui::plain_color((col[0] * 255.0) as u8, (col[1] * 255.0) as u8, (col[2] * 255.0) as u8, (a * XRAY * 255.0 + 0.5) as u8);
        l.near(centre, reach, &mut ids);
        let mut mesh = egui::Mesh::default();
        for &i in &ids {
            let tri = l.tris[i as usize];
            let y = l.ys.get(i as usize).copied().unwrap_or([0.0; 3]);
            let mut ok = true;
            let base = mesh.vertices.len() as u32;
            for j in 0..3 {
                let w = Vec3::new(tri[j].x, y[j] + lift, tri[j].y);
                let cl = view_proj * w.extend(1.0);
                if cl.w <= 1e-3 {
                    ok = false;
                    break;
                }
                let hidden = heights.map_or(false, |hf| hf.hidden(w, cam));
                mesh.vertices.push(egui::epaint::Vertex { pos: egui::pos2(c.x + hw * cl.x / cl.w, c.y - hh * cl.y / cl.w), uv: egui::epaint::WHITE_UV, color: if hidden { dim } else { colour } });
            }
            if ok {
                mesh.indices.extend([base, base + 1, base + 2]);
            } else {
                mesh.vertices.truncate(base as usize);
            }
        }
        if !mesh.indices.is_empty() {
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}

/// The GPS map's icons over the 3D scene: each queued mesh through the map camera's `view_proj`
/// onto `screen` (the 3D frame's rectangle).
pub fn paint_world(ctx: &egui::Context, painter: &egui::Painter, assets: &mut Assets, q: &Queue, view_proj: Mat4, screen: egui::Rect, ripple: Option<Ripple>, xray: Option<(&Heights, Vec3)>) {
    let (hw, hh) = (screen.width() * 0.5, screen.height() * 0.5);
    let c = screen.center();
    // (the ripple's rings for this frame, as a white picture with the program's alpha)
    let ripple_tex = match (ripple, q.draws.iter().any(|d| d.mesh == "target_ripple")) {
        (Some(r), true) => {
            const N: usize = 128;
            let mut px = Vec::with_capacity(N * N * 4);
            for y in 0..N {
                for x in 0..N {
                    let a = r.alpha((x as f32 + 0.5) / N as f32, (y as f32 + 0.5) / N as f32);
                    px.extend_from_slice(&[255, 255, 255, (a * 255.0 + 0.5) as u8]);
                }
            }
            let img = crate::flash_ui::plain_image(N as u32, N as u32, &px);
            let h = assets.handles.entry("ripple".into()).or_insert_with(|| ctx.load_texture("hudmap:ripple", img.clone(), egui::TextureOptions::LINEAR)).clone();
            let mut h2 = h.clone();
            h2.set(img, egui::TextureOptions::LINEAR);
            Some(h)
        }
        _ => None,
    };
    for d in &q.draws {
        let Some(m) = assets.mesh(d.mesh) else { continue };
        let wvp = view_proj * d.m;
        // (a draw wholly off the screen paints nothing: skip its x-ray tests)
        if xray.is_some() {
            let out = |e: [f32; 4]| m.parts.iter().all(|part| part.pos.iter().all(|p| {
                let cl = wvp * p.extend(1.0);
                cl.x * e[0] + cl.y * e[1] + cl.z * e[2] > cl.w * e[3]
            }));
            if out([1.0, 0.0, 0.0, 1.0]) || out([-1.0, 0.0, 0.0, 1.0]) || out([0.0, 1.0, 0.0, 1.0]) || out([0.0, -1.0, 0.0, 1.0]) {
                continue;
            }
        }
        for (k, part) in m.parts.iter().enumerate() {
            let handle = if d.mesh == "target_ripple" {
                let Some(h) = ripple_tex.clone() else { continue };
                h
            } else {
                let Some(tex) = part.tex.clone() else { continue };
                let key = format!("{}#{}", d.mesh, k);
                assets.handle(ctx, &key, &tex)
            };
            let col = crate::flash_ui::plain_color((d.colour[0].clamp(0.0, 1.0) * 255.0) as u8, (d.colour[1].clamp(0.0, 1.0) * 255.0) as u8, (d.colour[2].clamp(0.0, 1.0) * 255.0) as u8, (d.colour[3].clamp(0.0, 1.0) * 255.0) as u8);
            let dim = crate::flash_ui::plain_color((d.colour[0].clamp(0.0, 1.0) * 255.0) as u8, (d.colour[1].clamp(0.0, 1.0) * 255.0) as u8, (d.colour[2].clamp(0.0, 1.0) * 255.0) as u8, (d.colour[3].clamp(0.0, 1.0) * XRAY * 255.0) as u8);
            let mut mesh = egui::Mesh::with_texture(handle.id());
            let mut ok = true;
            for (i, p) in part.pos.iter().enumerate() {
                let cl = wvp * p.extend(1.0);
                if cl.w <= 1e-4 {
                    ok = false;
                    break;
                }
                let uv = part.uv.get(i).copied().unwrap_or(Vec2::ZERO);
                let hidden = xray.map_or(false, |(hf, cam)| hf.hidden(d.m.transform_point3(*p), cam));
                mesh.vertices.push(egui::epaint::Vertex { pos: egui::pos2(c.x + hw * cl.x / cl.w, c.y - hh * cl.y / cl.w), uv: egui::pos2(uv.x, uv.y), color: if hidden { dim } else { col } });
            }
            if !ok {
                continue;
            }
            mesh.indices.extend(part.idx.iter().copied());
            // (x-ray near per pixel: each triangle cut into pieces of about 8 px, each drawn whole or
            // at 25 % - a hard edge where a building covers part of an icon; MCLA_XRAY_VERTEX=1: per vertex)
            if let Some((hf, cam)) = xray {
                if std::env::var("MCLA_XRAY_VERTEX").is_err() {
                    mesh = xray_split(&mesh, part, &d.m, &wvp, (c, hw, hh), hf, cam, (col, dim));
                }
            }
            if d.mesh == "navsys_cursor_BG" && std::env::var("MCLA_HUDMAP_PAINTLOG").is_ok() {
                log::info!("hudmap paint: {} {:?} {:?}", d.mesh, mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>(), painter.clip_rect());
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}


// ---------------------------------------------------------------------------------------------
// The GPS map's camera (NavMap; `work/mm/rule_navcam.md`)

/// The map's middle for the "sc" city (outline polygon of `F-93744`, box overridden for sc).
pub const NAV_CENTRE: Vec3 = Vec3::new(-327.0, 0.0, 111.5);
const NAV_MIN: [f32; 2] = [-3627.0, -2488.5];
const NAV_MAX: [f32; 2] = [2973.0, 2386.5];
const NAV_LOW: f32 = 850.0;
const NAV_HIGH: f32 = 9000.0;

/// How far south of its look point the eye stands (`F-84102`).
pub fn nav_off(h: f32) -> f32 {
    let f = (h - NAV_LOW) / (5000.0 - NAV_LOW);
    if f > 1.0 {
        10.0
    } else {
        ((1.0 - f.clamp(0.0, 1.0)) * 1300.0).abs()
    }
}

/// `F-86323`.
fn nav_g(u: f32) -> f32 {
    if u <= 0.0 {
        0.0
    } else if u >= 1.0 {
        1.0
    } else {
        1.0 - (u * PI * 0.5).cos()
    }
}

/// The zoom parameter of a height (`acos(1 - t) * 0.63662`).
fn nav_u(h: f32) -> f32 {
    let t = ((h - NAV_LOW) / (NAV_HIGH - NAV_LOW)).clamp(0.0, 1.0);
    ((1.0 - t).clamp(-1.0, 1.0).acos() * 0.63662).min(1.0)
}

/// Where the eye may stand at height `h` (`F-94269`): (xmin, xmax, zmin, zmax).
pub fn nav_bounds(h: f32, aspect: f32) -> [f32; 4] {
    let off = nav_off(h);
    let a = off.atan2(h);
    let hf = 0.218166f32;
    let far = (a + hf).tan() * h;
    let near = (hf - a).tan() * h;
    let hw = (aspect * hf).tan() * h / (hf - a).cos() * 0.85;
    let m = hf.tan() * h * 0.1;
    let u = nav_u(h);
    let (mut xmin, mut xmax) = (NAV_MIN[0] + m + hw, NAV_MAX[0] - hw - m);
    let (mut zmin, mut zmax) = (NAV_MIN[1] + far - m - (1.0 - u) * 400.0, NAV_MAX[1] - near + m - (1.0 - u) * 400.0 * 0.5);
    if xmax < xmin {
        xmin = NAV_CENTRE.x;
        xmax = NAV_CENTRE.x;
    }
    if zmax < zmin {
        zmin = NAV_CENTRE.z;
        zmax = NAV_CENTRE.z;
    }
    [xmin, xmax, zmin, zmax]
}

/// The camera's moves (UITransition): the eye from `from` to `to` over `dur` seconds with the
/// piecewise-quadratic ease (Translate_Axis, no carried speed), on an ease-in clock for the opening.
#[derive(Clone, Copy)]
struct NavMove {
    from: Vec3,
    to: Vec3,
    from_look: Vec3,
    to_look: Vec3,
    t: f32,
    dur: f32,
    ease_in: bool,
}

#[derive(Clone)]
pub struct NavCam {
    /// the eye (x, H, z) and the stored look point (NM+720)
    pub eye: Vec3,
    pub look: Vec3,
    pub cursor: Vec3,
    bounds: [f32; 4],
    mv: Option<NavMove>,
    /// frame clock: the zoom steps once per 1/30 s
    acc: f32,
    /// the game camera the opening turns from (its rows)
    open_rows: Option<CamRows>,
    /// the way back to driving, once the map is closed
    pub exit: Option<NavExit>,
    /// the lens (NM+712)
    pub lens: NavLens,
    /// the render camera's velocity (playerMgr+0x51420, `F-31556`): the eye's move over the last
    /// 30 Hz frame / that frame's time (1/30 s), y = 0 - and the eye at the start of that frame, time
    /// since then
    vel: Vec3,
    vel_prev: Option<Vec3>,
    vel_t: f32,
}

/// The map camera's lens, NM+712: the Lerp_Float transition, a UITransition_Value (start
/// `F-13189`, update `F-34135`). Recorded from the original (rule_exitcam.md 10.9): the value
/// is eased, from + (to - from) (1 - cos(pi x)) / 2, x = (clock - delay) / duration, written before the
/// clock steps (the first frame shows `from`) and reaching `to` on the last write. Opening
/// (`F-39307`): from the game camera's lens to 25 degrees over 23 frames (0.767 s; "half the
/// opening's 1.5 s" as read, one frame longer as recorded). Closing (`F-88097` / `F-37814`):
/// from 25 to the game camera's lens (playerMgr+0x131F4), delay 0.2 T, over 0.7 T, T = the closing
/// flight's length (recorded with T 3.0: 0.6 s / 2.1 s).
#[derive(Clone, Copy, Debug)]
pub struct NavLens {
    from: f32,
    to: f32,
    delay: f32,
    dur: f32,
    clock: f32,
    pub deg: f32,
}

impl NavLens {
    pub fn new(from_deg: f32) -> NavLens {
        NavLens { from: from_deg, to: 25.0, delay: 0.0, dur: 23.0 / 30.0, clock: 0.0, deg: from_deg }
    }

    /// the closing fade back to the game camera's lens
    pub fn closing(to_deg: f32, delay: f32, dur: f32) -> NavLens {
        NavLens { from: 25.0, to: to_deg, delay, dur, clock: 0.0, deg: 25.0 }
    }

    /// the warper's lens (`F-98643` / `F-18267`: Lerp_Float 804 from, 812 to, 580 delay, 576
    /// length, written to warper+284)
    pub fn between(from_deg: f32, to_deg: f32, delay: f32, dur: f32) -> NavLens {
        NavLens { from: from_deg, to: to_deg, delay, dur, clock: 0.0, deg: from_deg }
    }

    /// already over (the lens at 25 degrees)
    pub fn done() -> NavLens {
        NavLens { from: 25.0, to: 25.0, delay: 0.0, dur: 0.75, clock: 0.75, deg: 25.0 }
    }

    /// One frame: the lens in degrees.
    pub fn step(&mut self, dt: f32) -> f32 {
        if self.clock >= self.delay && self.clock < self.delay + self.dur + 1e-4 {
            let x = ((self.clock - self.delay) / self.dur).min(1.0);
            self.deg = self.from + (self.to - self.from) * (1.0 - (PI * x).cos()) * 0.5;
        }
        self.clock += dt;
        self.deg
    }
}

impl Default for NavCam {
    fn default() -> NavCam {
        NavCam { eye: Vec3::new(NAV_CENTRE.x, NAV_HIGH, NAV_CENTRE.z), look: NAV_CENTRE, cursor: NAV_CENTRE, bounds: nav_bounds(NAV_HIGH, 16.0 / 9.0), mv: None, acc: 0.0, open_rows: None, exit: None, lens: NavLens::done(), vel: Vec3::ZERO, vel_prev: None, vel_t: 0.0 }
    }
}

impl NavCam {
    /// Opening (`F-39307` with EnterIntoMaxHeight, then activation `F-13388`): fly from
    /// the game camera to the whole city in 1.5 s; the stored look point and the cursor are the
    /// selection's place, else the player's - while the screen shows the whole city.
    pub fn open(&mut self, game_eye: Vec3, game_look: Vec3, at: Vec3, aspect: f32) {
        self.open_rows = Some(CamRows::look_at(game_eye, game_look, Vec3::Y));
        self.exit = None;
        let to = Vec3::new(NAV_CENTRE.x, NAV_HIGH, NAV_CENTRE.z);
        self.mv = Some(NavMove { from: game_eye, to, from_look: game_look, to_look: Vec3::new(to.x, 0.0, to.z - nav_off(NAV_HIGH)), t: 0.0, dur: 1.5, ease_in: true });
        self.eye = to;
        self.look = Vec3::new(at.x, 0.0, at.z);
        self.cursor = at;
        self.bounds = nav_bounds(NAV_HIGH, aspect);
        self.acc = 0.0;
        // (the render camera before the map's first frame is the game camera)
        self.vel = Vec3::ZERO;
        self.vel_prev = Some(game_eye);
        self.vel_t = 0.0;
    }

    /// The eye clamped and the look point made from it (`F-38897`).
    fn place(&mut self, mut e: Vec3) {
        e.y = e.y.clamp(NAV_LOW, NAV_HIGH);
        e.x = e.x.clamp(self.bounds[0], self.bounds[1]);
        e.z = e.z.clamp(self.bounds[2], self.bounds[3]);
        self.eye = e;
        self.look = Vec3::new(e.x, 0.0, e.z - nav_off(e.y));
    }

    /// Fly to an element (Change_Target, `F-79270`): eye over it at the same height, 10 m
    /// beyond the offset, in 1 s.
    pub fn fly_to(&mut self, p: Vec3) {
        let h = self.eye.y;
        let mut e = Vec3::new(p.x, h, p.z + nav_off(h) + 10.0);
        e.x = e.x.clamp(self.bounds[0], self.bounds[1]);
        e.z = e.z.clamp(self.bounds[2], self.bounds[3]);
        let (from, from_look) = self.view();
        self.mv = Some(NavMove { from, to: e, from_look, to_look: Vec3::new(e.x, 0.0, e.z - nav_off(h)), t: 0.0, dur: 1.0, ease_in: false });
        self.eye = e;
        self.look = Vec3::new(e.x, 0.0, e.z - nav_off(h));
        self.cursor = p;
    }

    /// One frame: RT (`zoom_in`) / LT (`zoom_out`) held, `cursor_move` = the left stick shaped
    /// (-1..1, x east, y south).
    pub fn update(&mut self, dt: f32, zoom_in: bool, zoom_out: bool, cursor_move: Vec2, aspect: f32) {
        if let Some(m) = self.mv.as_mut() {
            m.t += dt;
            if m.t >= m.dur {
                self.mv = None;
            }
        }
        self.acc += dt;
        self.vel_t += dt;
        let mut frame_done = false;
        while self.acc >= 1.0 / 30.0 {
            self.acc -= 1.0 / 30.0;
            frame_done = true;
            let hold = self.eye.y;
            if zoom_in || zoom_out {
                self.mv = None;
            }
            if zoom_in {
                // (F-50813: about the cursor)
                let u = (nav_u(hold) - 0.02).max(0.0);
                let h = NAV_LOW + (NAV_HIGH - NAV_LOW) * nav_g(u);
                let k = (hold - h) / h;
                let l = self.look;
                let e = Vec3::new(l.x + (self.cursor.x - l.x) * k, h, l.z + (self.cursor.z - l.z) * k + nav_off(h));
                self.place(e);
            } else if zoom_out {
                // (F-11681: toward the map's middle)
                let u = (nav_u(hold) + 0.02).min(1.0);
                let h = NAV_LOW + (NAV_HIGH - NAV_LOW) * nav_g(u);
                let k = (h - hold) / (NAV_HIGH + 250.0 - h);
                let l = self.look;
                let e = Vec3::new(l.x + (NAV_CENTRE.x - l.x) * k, h, l.z + (NAV_CENTRE.z - l.z) * k + nav_off(h));
                self.place(e);
            }
        }
        // the left stick moves the cursor at 0.5 H m/s at full tilt (F-62325)
        if cursor_move != Vec2::ZERO {
            let d = cursor_move * 0.5 * self.eye.y * dt;
            self.cursor += Vec3::new(d.x, 0.0, d.y);
        }
        // (the bounds of this frame's height, used by the next frame - F-51113)
        self.bounds = nav_bounds(self.eye.y, aspect);
        // the render camera's velocity (`F-31556`, once per rendered frame: (eye - previous eye) /
        // *(0x827D7558) = the timer's unscaled frame step, 1/30 s at the original's 30 Hz - written every
        // frame by `F-49323`; recorded: V = the eye's speed). Sampled on the same 30 Hz frames as
        // the zoom steps.
        if frame_done {
            let eye = self.view().0;
            if let Some(p) = self.vel_prev {
                if self.vel_t > 1e-6 {
                    let d = (eye - p) * (1.0 / self.vel_t);
                    self.vel = Vec3::new(d.x, 0.0, d.z);
                }
            }
            self.vel_prev = Some(eye);
            self.vel_t = 0.0;
        }
    }

    /// The velocity the closing flight starts with (playerMgr+0x51420 with y = 0, handed to the
    /// transition by `F-60031`): the eye's move over the last 30 Hz frame divided by its 1/30 s.
    pub fn eye_velocity(&self) -> Vec3 {
        self.vel
    }

    /// Where a world point lands on the screen (-1..1, y up) through the camera now.
    pub fn ndc(&self, p: Vec3, aspect: f32) -> Option<Vec2> {
        let (eye, look) = self.view();
        let vp = Mat4::perspective_rh(self.lens.deg.to_radians(), aspect, 1.0, 20000.0) * Mat4::look_at_rh(eye, look, Vec3::Y);
        let c = vp * p.extend(1.0);
        (c.w > 1e-3).then(|| Vec2::new(c.x / c.w, c.y / c.w))
    }

    /// The cursor pushed the view (`F-62325` at an edge limit): the eye moves by the cursor's
    /// own step on that axis.
    pub fn push(&mut self, d: Vec3) {
        self.mv = None;
        let e = self.eye + Vec3::new(d.x, 0.0, d.z);
        self.place(e);
    }

    /// Shape a stick as the NavMap does (`F-54604`): radial dead zone 0.2 rescaled, then each
    /// axis through 1 - cos(|a| pi / 2) with its sign.
    pub fn shape_stick(v: Vec2) -> Vec2 {
        let m = v.length();
        if m < 0.2 {
            return Vec2::ZERO;
        }
        let v = v * ((m.min(1.0) - 0.2) / 0.8) / m;
        let g = |a: f32| a.signum() * (1.0 - (a.abs().min(1.0) * PI * 0.5).cos());
        Vec2::new(g(v.x), g(v.y))
    }

    /// The camera now as rows (the opening turns the game camera's rows - Rotate children).
    pub fn view_rows(&self) -> CamRows {
        let (eye, look) = self.view();
        match (self.mv, self.open_rows) {
            (Some(m), Some(r)) if m.ease_in => {
                let mut c = nav_open_rows(r, m.t);
                c.eye = eye;
                c
            }
            _ => CamRows::look_at(eye, look, Vec3::Y),
        }
    }

    /// The camera now: eye and look-at point.
    pub fn view(&self) -> (Vec3, Vec3) {
        match self.mv {
            Some(m) => {
                let mut x = (m.t / m.dur).clamp(0.0, 1.0);
                if m.ease_in {
                    x = 1.0 - (x * PI * 0.5).cos();
                }
                let s = if x <= 0.5 { 2.0 * x * x } else { 1.0 - 2.0 * (1.0 - x) * (1.0 - x) };
                (m.from.lerp(m.to, s), m.from_look.lerp(m.to_look, s))
            }
            // (the pose is always built from the eye - F-38897; the stored look point NM+720
            // only feeds the zoom and may be stale)
            None => (self.eye, Vec3::new(self.eye.x, 0.0, self.eye.z - nav_off(self.eye.y))),
        }
    }
}

/// CHANGE TARGET (`F-99017`): the nearest place (3D distance) whose direction from the cursor
/// lies within 1 rad of `dir` (up = north = -z).
pub fn pick_in_direction(cursor: Vec3, dir: Vec3, places: &[Vec3]) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, p) in places.iter().enumerate() {
        let d = *p - cursor;
        let flat = Vec3::new(d.x, 0.0, d.z);
        let dist = d.length();
        if flat.length() < 1e-3 {
            continue;
        }
        let ang = flat.normalize().dot(dir.normalize()).clamp(-1.0, 1.0).acos();
        if ang < 1.0 && best.map_or(true, |b| dist < b.1) {
            best = Some((i, dist));
        }
    }
    best.map(|b| b.0)
}


// ---------------------------------------------------------------------------------------------
// Street names on the GPS map (MAP MODE; `work/mm/rule_labels.md`)

/// One label: a distinct street name (`record name + 6`) and the lanes it may sit on.
struct StreetLabel {
    text: String,
    imp: i32,
    lanes: Vec<(Vec3, Vec3)>,
    width: f32,
    slot: i32,
    fade: f32,
    active: bool,
    lane: Option<usize>,
    anchor: Vec3,
    rot: f32,
}

/// The street-name manager (`F-48222`): the `gps_sc` labels, 30 shown at most in the 60 slots
/// of the NAVSYS2DMOVIE movie, which draws them itself (Prototype 21 px, black over a 0xCFC87B
/// outline).
pub struct StreetNames {
    pub movie: crate::flash_ui::FlashMovie,
    labels: Vec<StreetLabel>,
    free: std::collections::VecDeque<i32>,
    acc: f32,
}

impl StreetNames {
    pub fn load(data_dir: &str, shared: &crate::flash_ui::UiShared) -> Option<StreetNames> {
        use mcla_formats::rage::Rd;
        let movie = crate::flash_ui::FlashMovie::load(data_dir, "resources/ui/0x94520e54/0xedfa681e.r27", shared)?;
        let res = crate::assets::load_resource_file(&Path::new(data_dir).join("resources/ui/navsys/0xeb965b06.r66")).ok()?;
        let rd = Rd::new(&res);
        let verts = rd.ptr(0x2c)?;
        let base = rd.ptr(116)?;
        let (stride, count) = (rd.u32(172) as usize, rd.u32(228) as usize);
        let vert = |i: u16| Vec3::new(rd.f32(verts + 16 * i as usize), rd.f32(verts + 16 * i as usize + 4), rd.f32(verts + 16 * i as usize + 8));
        let mut labels: Vec<StreetLabel> = Vec::new();
        let mut keys: Vec<String> = Vec::new();
        for i in 0..count {
            let o = base + i * stride;
            let Some(np) = rd.ptr(o + 144) else { continue };
            let raw = rd.cstr(np);
            if raw.len() <= 6 {
                continue;
            }
            let key = raw[6..].to_string();
            let mut lanes = Vec::new();
            if let Some(la) = rd.ptr(o + 20) {
                for k in 0..rd.u16(o + 24) as usize {
                    let Some(lo) = rd.ptr(la + 4 * k) else { continue };
                    let n = rd.u16(lo + 0x4c) as i16;
                    let Some(idx) = rd.ptr(lo + 0x50) else { continue };
                    if n < 0 {
                        continue;
                    }
                    lanes.push((vert(rd.u16(idx)), vert(rd.u16(idx + 2 * n as usize))));
                }
            }
            match keys.iter().position(|k| *k == key) {
                Some(j) => labels[j].lanes.extend(lanes),
                None => {
                    let text = key.strip_prefix('!').unwrap_or(&key).to_string();
                    let width = mcla_flash::player::measure_string(&movie.player.movie, "Prototype", &text, 21.0);
                    keys.push(key);
                    labels.push(StreetLabel { text, imp: rd.u32(o + 148) as i32, lanes, width, slot: -1, fade: 0.0, active: false, lane: None, anchor: Vec3::ZERO, rot: 0.0 });
                }
            }
        }
        log::info!("street names: {} labels, {} lanes", labels.len(), labels.iter().map(|l| l.lanes.len()).sum::<usize>());
        Some(StreetNames { movie, labels, free: (0..60).collect(), acc: 0.0 })
    }

    /// One frame (`F-91177` + `F-61493` + each label's `F-86093`) with the map camera
    /// `view_proj` onto the 1280x720 stage; `cam_y` its height, `u` the zoom parameter.
    pub fn update(&mut self, view_proj: Mat4, cam_y: f32, u: f32, dt: f32) {
        use mcla_flash::value::Value;
        let px = |p: Vec3| -> Option<Vec2> {
            let c = view_proj * p.extend(1.0);
            (c.w > 1e-3).then(|| Vec2::new(640.0 + 640.0 * c.x / c.w, 360.0 - 360.0 * c.y / c.w))
        };
        let on = |q: Option<Vec2>| q.map_or(false, |q| q.x > 0.0 && q.x < 1280.0 && q.y > 0.0 && q.y < 720.0);
        self.acc += dt;
        let mut steps = 0;
        while self.acc >= 1.0 / 30.0 && steps < 4 {
            self.acc -= 1.0 / 30.0;
            steps += 1;
        }
        let r = if cam_y > 8000.0 { 100 } else if cam_y > 5000.0 { 80 } else if cam_y > 3000.0 { 40 } else { 0 };
        let mut cands: Vec<usize> = Vec::new();
        for (li, l) in self.labels.iter_mut().enumerate() {
            let vis = |a: Vec3, b: Vec3| on(px(a)) as u8 + on(px(b)) as u8;
            let short = |a: Vec3, b: Vec3, w: f32| match (px(a), px(b)) {
                (Some(p), Some(q)) => w >= p.distance(q),
                _ => true,
            };
            if !l.lanes.iter().any(|(a, b)| vis(*a, *b) == 2) {
                l.lane = None;
            } else {
                let keep = l.active && l.lane.map_or(false, |k| vis(l.lanes[k].0, l.lanes[k].1) != 0 && !short(l.lanes[k].0, l.lanes[k].1, l.width));
                if !keep {
                    let ok: Vec<usize> = (0..l.lanes.len()).filter(|&k| vis(l.lanes[k].0, l.lanes[k].1) == 2 && !short(l.lanes[k].0, l.lanes[k].1, l.width)).collect();
                    l.lane = ok.get(ok.len() / 2).copied();
                }
                if let Some(k) = l.lane {
                    let (a, b) = l.lanes[k];
                    let mid = a + (b - a) * 0.5;
                    if mid.x != l.anchor.x || mid.z != l.anchor.z {
                        l.fade = 0.0;
                    }
                    l.anchor = mid;
                    if let (Some(pa), Some(pb)) = (px(a), px(b)) {
                        let d = if pa.x < pb.x { pb - pa } else { pa - pb };
                        let mut ang = d.y.atan2(d.x);
                        if ang < 0.0 {
                            ang += std::f32::consts::TAU;
                        }
                        l.rot = ang;
                    }
                }
            }
            if l.lane.is_some() && l.imp >= r && (r >= 100 || l.imp != 100) {
                cands.push(li);
            }
        }
        // (sorted by importance, descending; equal importance keeps the order - a stable sort)
        cands.sort_by(|a, b| self.labels[*b].imp.cmp(&self.labels[*a].imp));
        cands.truncate(30);
        for (li, l) in self.labels.iter_mut().enumerate() {
            let want = cands.contains(&li);
            if l.active && !want {
                l.active = false;
            } else if !l.active && want {
                if l.slot < 0 {
                    match self.free.pop_front() {
                        Some(s) => l.slot = s,
                        None => continue,
                    }
                }
                l.active = true;
            }
        }
        let scale = (1.0 - u) * 25.0 + 75.0;
        let p = &mut self.movie.player;
        for l in self.labels.iter_mut() {
            if l.slot < 0 {
                continue;
            }
            for _ in 0..steps {
                if !l.active {
                    l.fade -= 0.1;
                } else if l.fade < 1.0 {
                    l.fade = (l.fade + 0.1).min(1.0);
                }
            }
            if !l.active && l.fade <= 0.0 {
                l.fade = 0.0;
                p.set_path(&format!("streetnames.{}.alpha", l.slot), Value::Float(0.0));
                self.free.push_back(l.slot);
                l.slot = -1;
                continue;
            }
            let q = px(l.anchor).unwrap_or(Vec2::new(-9999.0, -9999.0));
            let s = l.slot;
            p.set_path(&format!("streetnames.{s}.x"), Value::Float(q.x));
            p.set_path(&format!("streetnames.{s}.y"), Value::Float(q.y));
            p.set_path(&format!("streetnames.{s}.scale"), Value::Float(scale));
            p.set_path(&format!("streetnames.{s}.name"), Value::str(&l.text));
            p.set_path(&format!("streetnames.{s}.rotation"), Value::Float(l.rot.to_degrees()));
            p.set_path(&format!("streetnames.{s}.alpha"), Value::Float(l.fade * 100.0));
        }
        p.set_global("streetname_update", Value::Int(1));
        p.set_global("cur_visibility", Value::Int(1));
        p.set_global("aspect", Value::Int(1));
        p.update(dt);
    }
}

/// The zoom parameter of a camera height (`NM+1392`).
pub fn nav_zoom_u(h: f32) -> f32 {
    nav_u(h)
}


// ---------------------------------------------------------------------------------------------
// The GPS map's camera moves in and out (UITransition; `work/mm/rule_exitcam.md`)

/// A camera as RAGE rows: right, up, BACK (it looks along -r2), and its place.
#[derive(Clone, Copy, Debug)]
pub struct CamRows {
    pub r0: Vec3,
    pub r1: Vec3,
    pub r2: Vec3,
    pub eye: Vec3,
}

impl CamRows {
    /// `F-97951`: r2 = n(eye - target), r0 = n(up x r2), r1 = r2 x r0.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> CamRows {
        let r2 = (eye - target).normalize_or(Vec3::Z);
        let r0 = up.cross(r2).normalize_or(Vec3::X);
        CamRows { r0, r1: r2.cross(r0), r2, eye }
    }

    pub fn look_point(&self) -> Vec3 {
        self.eye - self.r2 * 100.0
    }

    /// every row times roty(th) (`F-92665`: [[c,0,-s],[0,1,0],[s,0,c]]) - a yaw about world Y
    fn roty(mut self, th: f32) -> CamRows {
        self.r0 = roty_v(self.r0, th);
        self.r1 = roty_v(self.r1, th);
        self.r2 = roty_v(self.r2, th);
        self
    }

    fn rot(mut self, ax: Vec3, ang: f32) -> CamRows {
        let q = glam::Quat::from_axis_angle(ax, ang);
        self.r0 = q * self.r0;
        self.r1 = q * self.r1;
        self.r2 = q * self.r2;
        self
    }

    /// roll about its own r2 (`F-83846`)
    fn roll(mut self, th: f32) -> CamRows {
        let (s, c) = th.sin_cos();
        let (r0, r1) = (self.r0, self.r1);
        self.r0 = r0 * c + r1 * s;
        self.r1 = r1 * c - r0 * s;
        self
    }

    /// pitch about its own r0 (`F-52786`)
    fn pitch(mut self, th: f32) -> CamRows {
        let (s, c) = th.sin_cos();
        let (r1, r2) = (self.r1, self.r2);
        self.r1 = r1 * c + r2 * s;
        self.r2 = r2 * c - r1 * s;
        self
    }
}

fn roty_v(v: Vec3, th: f32) -> Vec3 {
    let (s, c) = th.sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}

fn flat_n(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z).normalize_or(Vec3::Z)
}

/// `F-52353`: a UITransition time (total_time, delay_start_time) snapped to whole frames of
/// *(0x827D7558) (the timer's frame step, 1/30 s), in single precision: n = trunc(t (1/F) + 0.5), t' =
/// n / (1/F). 1/F rounds to 29.999998, so a time on a half frame goes DOWN: 0.75 -> 22/30, 2.25 ->
/// 67/30, 1.05 -> 31/30 (recorded in the original's closing flights, rule_exitcam.md 10.9).
pub fn ui_time(t: f32) -> f32 {
    let r = 1.0f32 / (1.0f32 / 30.0);
    let x = r * t;
    let n = if x >= 0.0 { (x + 0.5) as i32 } else { (x - 0.5) as i32 };
    n as f32 / r
}

/// `s(u)`: the Translate_Axis / Rotate profile with no carried speed.
fn s_curve(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    if u <= 0.5 {
        2.0 * u * u
    } else {
        1.0 - 2.0 * (1.0 - u) * (1.0 - u)
    }
}

/// The carried-speed part of the Translate_Axis profile (`F-76061` / `F-58409` with
/// v1 = 0): `vm = 2d/T - v0/2, a1 = 2(vm - v0)/T, a2 = -2vm/T` give p(tau) = d s(tau/T) + v0 T g(tau/T),
/// g(u) = u - 1.5u^2 (u <= 1/2), 1/8 - (w - w^2)/2 with w = u - 1/2; 0 at both ends, slope v0 at 0.
fn ta_carry(v0: f32, t: f32, tau: f32) -> f32 {
    let u = (tau / t).clamp(0.0, 1.0);
    let g = if u <= 0.5 {
        u - 1.5 * u * u
    } else {
        let w = u - 0.5;
        0.125 - 0.5 * (w - w * w)
    };
    v0 * t * g
}

/// `F-32060`: the angle turning `a` onto `b` in the xz plane.
fn sangle(a: Vec3, b: Vec3) -> f32 {
    let (a, b) = (flat_n(a), flat_n(b));
    let ang = a.dot(b).clamp(-1.0, 1.0).acos();
    if a.z * b.x - a.x * b.z >= 0.0 {
        ang
    } else {
        -ang
    }
}

/// `F-73985`: the axis and angle turning `u` onto `v`.
fn align(u: Vec3, v: Vec3) -> (Vec3, f32) {
    let (u, v) = (u.normalize_or(Vec3::Y), v.normalize_or(Vec3::Y));
    let cr = u.cross(v);
    let d = u.dot(v);
    if d <= -0.998 {
        return (u.any_orthonormal_vector(), PI);
    }
    let sp = cr.length().min(1.0);
    let mut phi = sp.asin();
    if d <= 0.0 {
        phi = PI - phi;
    }
    (cr.normalize_or(Vec3::Y), phi)
}

/// LookAtTarget's blend (`F-99969`): from A's rows toward B's by weight w - yaw about world Y,
/// then the view direction, then the up vector (only in the second half with delay_up_rotate).
fn blend_rows(a: CamRows, b: CamRows, w: f32, delay_up: bool) -> CamRows {
    let (fa, fb) = (flat_n(a.r2), flat_n(b.r2));
    let mut th = fa.dot(fb).clamp(-1.0, 1.0).acos();
    let test = roty_v(fa, th);
    if test.dot(fb).clamp(-1.0, 1.0).acos() > 0.1 * th {
        th = -th;
    }
    let mut m = a.roty(w * th);
    let m2 = a.roty(th);
    let (ax, phi) = align(m2.r2, b.r2);
    m = m.rot(ax, w * phi);
    let m3 = m2.rot(ax, phi);
    let (ax2, psi) = align(m3.r1, b.r1);
    if delay_up {
        let w2 = 2.0 * (w - 0.5);
        if w2 > 0.0 {
            m = m.rot(ax2, psi * (1.0 - (w2 * PI * 0.5).cos()));
        }
    } else {
        m = m.rot(ax2, psi * w);
    }
    m.eye = a.eye;
    m
}

/// `F-32060` exactly: the signed xz angle turning `a` onto `b`, 0 when either is vertical.
fn sangle0(a: Vec3, b: Vec3) -> f32 {
    let n = ((a.x * a.x + a.z * a.z) * (b.x * b.x + b.z * b.z)).sqrt();
    if n - 1e-12 < 0.0 {
        return 0.0;
    }
    let ang = ((a.x * b.x + a.z * b.z) / n).clamp(-1.0, 1.0).acos();
    if a.z * b.x - a.x * b.z >= 0.0 {
        ang
    } else {
        -ang
    }
}

fn wrap_pi(mut a: f32) -> f32 {
    while a > PI {
        a -= std::f32::consts::TAU;
    }
    while a < -PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// The rage spline's Catmull-Rom segment (`F-85496` basis 0x82812730, `F-39947`).
fn catmull(p: [Vec3; 4], u: f32) -> Vec3 {
    let c0 = p[0] * -0.5 + p[1] * 1.5 - p[2] * 1.5 + p[3] * 0.5;
    let c1 = p[0] - p[1] * 2.5 + p[2] * 2.0 - p[3] * 0.5;
    let c2 = (p[2] - p[0]) * 0.5;
    c0 * (u * u * u) + c1 * (u * u) + c2 * u + p[1]
}

/// Which closing flight (`F-60031`, GPS branch, rule_exitcam.md 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitKind {
    /// Exiting_Map_GPS_HighAltitude (3.0 s)
    HighAltitude,
    /// Exiting_Map_GPS_LowAltitude_FarAway (3.5 s)
    LowAltitudeFarAway,
    /// Exiting_Map_GPS_AboveCar (2.0 s)
    AboveCar,
    /// Exiting_Map_GPS_SharpAngle (2.0 s)
    SharpAngle,
    /// Exiting_Map_GPS_SouthSideCloseCar (1.533 s: AxisStages' 1.05 snaps to 31/30)
    SouthSideCloseCar,
}

impl ExitKind {
    /// `F-60031` with start S = the map camera (playerMgr+0x513E0) and end E = the gameplay camera
    /// (playerMgr+0x13170, `F-88097`): h = S.y, dist = |flat(S - E)|, f1 = sangle(S.r2, flat(S - E))
    /// (where the map camera stands seen from the car's camera, 0 = due south since the map's r2 points
    /// south), f30 = sangle(E.r2, S.r2) (the chase camera's back against south; +-pi = car facing south).
    pub fn choose(s: &CamRows, e: &CamRows) -> ExitKind {
        let h = s.eye.y;
        let d = s.eye - e.eye;
        let flat = Vec3::new(d.x, 0.0, d.z);
        let dist = flat.length();
        let f30 = sangle0(e.r2, s.r2);
        let f1 = sangle0(s.r2, flat);
        if h > 6000.0 {
            ExitKind::HighAltitude
        } else if dist > 2000.0 {
            ExitKind::LowAltitudeFarAway
        } else if dist < 100.0 && h < 200.0 {
            // (Warp_CloseToTarget: never from the GPS map, its camera stays at 850 m or higher)
            ExitKind::AboveCar
        } else if h < 6000.0 && dist < 0.25 * h + 600.0 {
            ExitKind::AboveCar
        } else if f1 > 0.75 || f1 < -0.75 {
            ExitKind::AboveCar
        } else if f30.abs() > 2.5 {
            ExitKind::SharpAngle
        } else {
            ExitKind::SouthSideCloseCar
        }
    }

    /// the flight's length (= its real time, the Lean's ease_out maps elapsed T to clock T)
    pub fn total(self) -> f32 {
        match self {
            ExitKind::HighAltitude => 3.0,
            ExitKind::LowAltitudeFarAway => 3.5,
            ExitKind::AboveCar | ExitKind::SharpAngle => 2.0,
            // (max delay + total of the children: AxisStages' 1.05 + 0.5, the 1.05 snapped to 31/30)
            ExitKind::SouthSideCloseCar => ui_time(1.05) + 0.5,
        }
    }

    fn lean_factor(self) -> f32 {
        if self == ExitKind::HighAltitude {
            1.0
        } else {
            20.0
        }
    }
}

/// The way back to driving (`F-88097` -> `F-60031`): one of five UITransition states of
/// Main.sc.xml picked from the map camera's height, distance and angles to the driving camera.
#[derive(Clone)]
pub struct NavExit {
    kind: ExitKind,
    s: CamRows,
    e: CamRows,
    t: f32,
    out: CamRows,
    /// LookAtTarget's A (the render camera when it starts)
    lat_a: Option<CamRows>,
    /// Lean: history, last, previous out, its 30 Hz clock
    hist: [f32; 4],
    last: f32,
    prev: CamRows,
    acc: f32,
    lam: f32,
    /// the map camera's velocity handed in (vfuncs 736 / 744), y = 0, and its projections (vfunc 860
    /// `F-18512`) on along_final_look (3D: n(E.r3 + n(E.r2) target - S.r3)) and final_look
    /// (n(flat E.r2))
    vel: Vec3,
    v0_along: f32,
    v0_final: f32,
    /// the child clock of the frame before (a Translate_Axis steps only once its clock was past its
    /// delay before the frame: the frame that crosses the delay still shows 0)
    c_prev: f32,
}

impl NavExit {
    /// the flight's length: 3.0 s (HighAltitude), 3.5 (LowAltitude_FarAway), 2.0 (AboveCar, SharpAngle),
    /// 1.533 (SouthSideCloseCar)
    pub fn total(&self) -> f32 {
        self.kind.total()
    }

    pub fn kind(&self) -> ExitKind {
        self.kind
    }

    /// from a map camera at rest
    pub fn new(s: CamRows, e: CamRows) -> NavExit {
        NavExit::moving(s, e, Vec3::ZERO)
    }

    /// `F-60031` also hands the render camera's velocity (playerMgr+0x51420, y set to 0) to the
    /// transition: Lean / Parallel give it to every child, a Serial to its first child; each
    /// Translate_Axis projects it on its axis (vfunc 860) and starts its profile with that speed.
    /// The choice itself does not use it.
    pub fn moving(s: CamRows, e: CamRows, vel: Vec3) -> NavExit {
        let vel = Vec3::new(vel.x, 0.0, vel.z);
        let kind = ExitKind::choose(&s, &e);
        let target = if kind == ExitKind::HighAltitude { 50.0 } else { 100.0 };
        let v0_along = vel.dot((e.eye + e.r2.normalize_or_zero() * target - s.eye).normalize_or_zero());
        let v0_final = vel.dot(flat_n(e.r2));
        if std::env::var("MCLA_NAV_LOG").is_ok() {
            let d = s.eye - e.eye;
            log::info!("nav exit: {:?} ({:.2} s) h {:.1} dist {:.1} f1 {:.4} f30 {:.4} from {:?} to {:?} vel {:?}", kind, kind.total(), s.eye.y, Vec3::new(d.x, 0.0, d.z).length(), sangle0(s.r2, d), sangle0(e.r2, s.r2), s.eye, e.eye, vel);
        }
        NavExit { kind, s, e, t: 0.0, out: s, lat_a: None, hist: [0.0; 4], last: 0.0, prev: s, acc: 0.0, lam: 0.0, vel, v0_along, v0_final, c_prev: 0.0 }
    }

    /// What the carried velocity adds to the eye at child clock `c` (the first Translate_Axis of each
    /// group; second stages of a Serial start from the first one's end speed v1 = 0, the spline and
    /// the Y / target_velocity pieces get 0).
    fn carried(&self, c: f32, c_prev: f32) -> Vec3 {
        let (s, e, v) = (self.s, self.e, self.vel);
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
        match self.kind {
            ExitKind::HighAltitude | ExitKind::LowAltitudeFarAway => {
                // along_final_look 50 (.75 -> 22/30, 1.5) / 100 (0, 3): along n(P - flat(S)), flat
                let (target, d0, t) = if self.kind == ExitKind::HighAltitude { (50.0, ui_time(0.75), 1.5) } else { (100.0, 0.0, 3.0) };
                if d0 > 0.0 && c_prev < d0 {
                    return Vec3::ZERO;
                }
                let p = flat(e.eye) + flat_n(e.r2) * target;
                (p - flat(s.eye)).normalize_or_zero() * ta_carry(self.v0_along, t, c - d0)
            }
            ExitKind::AboveCar | ExitKind::SouthSideCloseCar => {
                // X, Z (0, 2 / 1.5); Forward's first final_look 'set' 300 (0, .5 / 1)
                let (txz, tf) = if self.kind == ExitKind::AboveCar { (2.0, 0.5) } else { (1.5, 1.0) };
                Vec3::new(ta_carry(v.x, txz, c), 0.0, ta_carry(v.z, txz, c)) + flat_n(e.r2) * ta_carry(self.v0_final, tf, c)
            }
            ExitKind::SharpAngle => Vec3::ZERO,
        }
    }

    /// One frame: the camera, or None once the move is over (the game camera takes over).
    pub fn step(&mut self, dt: f32) -> Option<CamRows> {
        self.t += dt;
        let total = self.kind.total();
        let lean_factor = self.kind.lean_factor();
        if self.t >= total - 1e-4 {
            return None;
        }
        // (ease_out clock of the Lean / Parallel: C = T sin(pi/2 t/T))
        let c = total * (self.t.min(total) / total * PI * 0.5).sin();
        let c_prev = self.c_prev;
        self.c_prev = c;
        let (s, e) = (self.s, self.e);
        let prev_eye = self.out.eye;
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
        // LookAtTarget (`F-47360`): from A (the camera when it starts) toward looking at L from
        // the camera's last place, up = E.r1, cosine weight over `ttl` from clock `d0`
        let look = |this: &mut NavExit, d0: f32, ttl: f32, target: f32, delay_up: bool| {
            let a = *this.lat_a.get_or_insert(this.out);
            let l = e.eye + e.r2.normalize_or(Vec3::Z) * target;
            let b = CamRows::look_at(prev_eye, l, e.r1);
            let w = (1.0 - (PI * ((c - d0) / ttl).min(1.0)).cos()) * 0.5;
            blend_rows(a, b, w, delay_up)
        };
        let mut out;
        match self.kind {
            ExitKind::HighAltitude => {
                // (times snapped to frames: the along_final_look delay and the X / Z stage 0.75 ->
                // 22/30 s, LookAtTarget delay 2.25 -> 67/30 for 22/30; the first stage shows 0 on the
                // frame that crosses its delay)
                let (d1, t2, d3) = (ui_time(0.75), ui_time(0.75), ui_time(2.25));
                let p = Vec3::new(e.eye.x, 0.0, e.eye.z) + flat_n(e.r2) * 50.0;
                let s1 = if c_prev < d1 { 0.0 } else { s_curve((c - d1) / 1.5) };
                let s2 = s_curve((c - d1 - 1.5) / t2);
                let x = s.eye.x + (p.x - s.eye.x) * s1 + (e.eye.x - p.x) * s2;
                let z = s.eye.z + (p.z - s.eye.z) * s1 + (e.eye.z - p.z) * s2;
                let y = s.eye.y + (e.eye.y - s.eye.y) * s_curve(c / 3.0);
                out = s;
                out.eye = Vec3::new(x, y, z);
                if c >= d3 {
                    let a = *self.lat_a.get_or_insert(self.out);
                    let l = e.eye - e.r2.normalize_or(Vec3::Z) * 15.0;
                    let b = CamRows::look_at(prev_eye, l, e.r1);
                    // (time_to_look 0.75 is not snapped, only delay and total are; complete - w = 1 -
                    // at the snapped delay + total, 67/30 + 22/30 s)
                    let w = if c >= d3 + t2 { 1.0 } else { (1.0 - (PI * ((c - d3) / 0.75).min(1.0)).cos()) * 0.5 };
                    let eye = out.eye;
                    out = blend_rows(a, b, w, true);
                    out.eye = eye;
                }
            }
            ExitKind::AboveCar => {
                let fwd = flat_n(e.r2) * (300.0 * s_curve(c.min(0.5) / 0.5) - 300.0 * s_curve((c - 0.5) / 1.5));
                let eye = s.eye + (e.eye - s.eye) * s_curve(c / 2.0) + fwd;
                let ey = |r: CamRows| r.r2.x.atan2(r.r2.z);
                let d = wrap_pi(ey(e) - ey(s));
                out = s.roty(d * s_curve(c / 1.5));
                if c >= 1.5 {
                    let a = *self.lat_a.get_or_insert(s.roty(d));
                    let l = e.eye - e.r2.normalize_or(Vec3::Z) * 10.0;
                    let b = CamRows::look_at(prev_eye, l, e.r1);
                    let w = (1.0 - (PI * ((c - 1.5) / 0.5).min(1.0)).cos()) * 0.5;
                    out = blend_rows(a, b, w, false);
                }
                out.eye = eye;
            }
            ExitKind::LowAltitudeFarAway => {
                // AxisStages('Y', 2500, 1, 2.5): to 2500 m above the end in 1, then down in 2.5
                let y = s.eye.y + (e.eye.y + 2500.0 - s.eye.y) * s_curve(c) - 2500.0 * s_curve((c - 1.0) / 2.5);
                // Serial[along_final_look 100 (0, 3); X, Z (0, .5)]: to 100 m behind the end, then onto it
                let (sf, ef) = (flat(s.eye), flat(e.eye));
                let p = ef + flat_n(e.r2) * 100.0;
                let xz = sf + (p - sf) * s_curve(c / 3.0) + (ef - p) * s_curve((c - 3.0) / 0.5);
                // Serial[Par[Rotate x to -1.370796 (0, 1.5), Rotate y final_look (0, 2.5)];
                //        LookAtTarget(0, 1, 1, -15)]
                if c >= 2.5 {
                    out = look(self, 2.5, 1.0, -15.0, false);
                } else {
                    let dx = -1.370_796 - (-s.r2.y).clamp(-1.0, 1.0).asin();
                    let ey = |r: CamRows| r.r2.x.atan2(r.r2.z);
                    let dy = wrap_pi(ey(e) - ey(s));
                    out = s.pitch(dx * s_curve(c / 1.5)).roty(dy * s_curve(c / 2.5));
                }
                out.eye = Vec3::new(xz.x, y, xz.z);
            }
            ExitKind::SouthSideCloseCar => {
                // X, Z (0, 1.5); AxisStages('Y', 100, 1.05, .5); Forward(+300 (0, 1), -300 (0, .5));
                // Axis2Stages('target_velocity', ..) moves nothing (no vector handed in, `F-66730`)
                // (AxisStages' 1.05 snaps to 31/30 s)
                let y1 = ui_time(1.05);
                let xz = flat(s.eye) + (flat(e.eye) - flat(s.eye)) * s_curve(c / 1.5);
                let y = s.eye.y + (e.eye.y + 100.0 - s.eye.y) * s_curve(c / y1) - 100.0 * s_curve((c - y1) / 0.5);
                let fwd = flat_n(e.r2) * (300.0 * s_curve(c) - 300.0 * s_curve((c - 1.0) / 0.5));
                // LookAtTarget(0, 1.5, 1.5, -15) from the map camera
                out = look(self, 0.0, 1.5, -15.0, false);
                out.eye = Vec3::new(xz.x, y, xz.z) + fwd;
            }
            ExitKind::SharpAngle => {
                // UITransition_Translate_Spline_ExitSharpAngle (`F-38895` / `F-34196` /
                // `F-43336`): x, z on a Catmull-Rom curve S -> P1 -> E, a segment per half of the
                // time; P1 = E + 100 n(E.r0) (to the side the chase camera turns from) - 150 m toward S
                let sign = if sangle0(e.r2, s.r2) >= 0.0 { 1.0 } else { -1.0 };
                let (p0, p2) = (s.eye, e.eye);
                let p1 = e.eye + e.r0.normalize_or(Vec3::X) * (100.0 * sign) - flat(e.eye - s.eye).normalize_or_zero() * 150.0;
                let x = c / 2.0;
                let q = if x < 0.9999 {
                    let seg = (2.0 * x) as i32;
                    let u = 2.0 * x - seg as f32;
                    if seg == 0 {
                        catmull([p0 * 2.0 - p1, p0, p1, p2], u)
                    } else {
                        catmull([p0, p1, p2, p2 * 2.0 - p1], u)
                    }
                } else {
                    p2
                };
                // Y (0, 2); LookAtTarget(0, .5, 2, -10) from the map camera
                let y = s.eye.y + (e.eye.y - s.eye.y) * s_curve(c / 2.0);
                out = look(self, 0.0, 0.5, -10.0, false);
                out.eye = Vec3::new(q.x, y, q.z);
            }
        }
        if self.vel != Vec3::ZERO {
            out.eye += self.carried(c, c_prev);
        }
        // Lean (`F-10316`): a roll into the turn, sampled at 30 Hz as the original's frames
        self.acc += dt;
        while self.acc >= 1.0 / 30.0 {
            self.acc -= 1.0 / 30.0;
            // (d = sangle(flat prev.r2, flat out.r2): `F-32060(r3 = prev, r4 = out)` - recorded:
            // the other order rolled the wrong way)
            let d = sangle(self.prev.r2, out.r2);
            self.prev = out;
            self.lam = if c >= total - 0.5 {
                (total - (c + 0.0333)).max(0.0) * 2.0 * self.last
            } else {
                let l = ((self.hist.iter().sum::<f32>() + lean_factor * d) / 5.0).clamp(-PI / 4.0, PI / 4.0);
                self.hist.rotate_right(1);
                self.hist[0] = l;
                self.last = l;
                l
            };
        }
        let mut k = 2.0 * (1.0 - out.r2.y);
        if !(0.0..=1.0).contains(&k) {
            k = 1.0;
        }
        out = out.roll(k * self.lam.clamp(-PI / 4.0, PI / 4.0));
        self.out = out;
        Some(out)
    }
}

// ---------------------------------------------------------------------------------------------
// The cinematic warps (race start from the map: mcWarper `F-32081` / `F-18267`,
// `work/career2/re_start/notes.md`, rule_exitcam.md "Cinematic warps (race start)")

/// The start scene begins (`F-31347` state 19 clears cineMgr+293 = IsGameReadyForScene) when the
/// flight's time reaches `total - CINE_HANDOVER` (`+260 + 1e-4 >= T - 30 * frame_dt` = T - 1.0 s at the
/// original's 30 Hz). The flight's end E is the scene camera at scene t = 1.0 (`F-62707`, req+3 = 0),
/// so scene t = flight time - (total - CINE_HANDOVER) and the flight lands on the scene camera exactly when
/// it is over; from then on the camera is the scene's.
pub const CINE_HANDOVER: f32 = 1.0;

/// Which cinematic warp (`F-60031` with req+9 = 1, the `Warp_*_Cinematically` states of
/// tune/ui/transitions/Main.sc.xml).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CineKind {
    /// Warp_HighAltitude_Cinematically (Main.sc.xml l.37; 3.0 s, no easing)
    HighAltitude,
    /// Warp_LowAltitude_FarAway_Cinematically (l.107; 5.25 s)
    LowAltitudeFarAway,
    /// Warp_CloseToTarget (l.601; 2.033 s, a UITransition_Lerp)
    CloseToTarget,
    /// Warp_AboveCar_Cinematically (l.316; 3.0 s)
    AboveCar,
    /// Warp_SharpAngle_Cinematically (l.268; 3.033 s)
    SharpAngle,
    /// Warp_SouthSideCloseCar_Cinematically (l.227; 3.033 s)
    SouthSideCloseCar,
    /// Warp_Target_Close (l.354, "used for story location cuts": a Serial with one UITransition_Lerp,
    /// ease_in_out, total 1, no Lean) - STORY_LOCATION_CUT_SIMPLE with the render camera within 100 of
    /// the destination (`F-32081` case 17); `CineWarp::target_close`
    TargetClose,
}

impl CineKind {
    /// `F-60031`, req+9 = 1: S = the render camera (playerMgr+0x513E0), E = the start scene's camera
    /// at scene t 1.0; h = S.y, dist = |flat(S - E)|, f30 = sangle(E.r2, S.r2) as `ExitKind::choose`.
    /// Differs from the GPS tree: the f1 test is `f1 > 0.75 && f1 < -0.75` (never true) and an extra
    /// `E.z > S.z -> AboveCar` comes before the f30 test (`A-28643..A-41039`). req+4 (forces
    /// CloseToTarget) is 0 on the race start.
    pub fn choose(s: &CamRows, e: &CamRows) -> CineKind {
        let h = s.eye.y;
        let d = s.eye - e.eye;
        let dist = Vec3::new(d.x, 0.0, d.z).length();
        let f30 = sangle0(e.r2, s.r2);
        if h > 6000.0 {
            CineKind::HighAltitude
        } else if dist > 2000.0 {
            CineKind::LowAltitudeFarAway
        } else if dist < 100.0 && h < 200.0 {
            CineKind::CloseToTarget
        } else if (h < 6000.0 && dist < 0.25 * h + 600.0) || e.eye.z > s.eye.z {
            CineKind::AboveCar
        } else if f30.abs() > 2.5 {
            CineKind::SharpAngle
        } else {
            CineKind::SouthSideCloseCar
        }
    }

    /// The flight's length (the top node's real total, `F-66580`): max over the children of
    /// delay + total with the times snapped by `ui_time`; equal to the Lean's own length in every state.
    pub fn total(self) -> f32 {
        match self {
            CineKind::HighAltitude | CineKind::AboveCar => 3.0,
            // (AxisStages' 1.5 + 3.75; LookAtTarget after the 3.75 Rotate y: + 1.5)
            CineKind::LowAltitudeFarAway => 5.25,
            // (Axis2Stages 1.5 + .55, the .55 snapped to 16/30; the Lerp's 2)
            CineKind::CloseToTarget => 1.5 + ui_time(0.55),
            // (Y 3.05 -> 91/30; Axis2Stages 2 + 1.05 -> 2 + 31/30)
            CineKind::SharpAngle => ui_time(3.05),
            // (AxisStages 2.5025 + .533866 -> 2.5 + 16/30; Axis2Stages 2.002 + 1.03436667 -> 2 + 31/30)
            CineKind::SouthSideCloseCar => ui_time(2.5025) + ui_time(0.533_866),
            CineKind::TargetClose => 1.0,
        }
    }

    /// The Lean's child clock at real time `t` (`F-17563`): ease_out C = T sin(pi t / 2T)
    /// (LowAltitude, AboveCar, SharpAngle, SouthSide), ease_in_out C = T (1 - cos(pi t / T)) / 2
    /// (CloseToTarget), none (HighAltitude: neither its Parallel nor its Lean has an easing).
    fn clock(self, t: f32) -> f32 {
        let tl = self.total();
        let x = (t / tl).clamp(0.0, 1.0);
        match self {
            CineKind::HighAltitude | CineKind::TargetClose => t.min(tl),
            CineKind::CloseToTarget => tl * (1.0 - (PI * x).cos()) * 0.5,
            _ => tl * (x * PI * 0.5).sin(),
        }
    }

    fn lean_factor(self) -> f32 {
        if self == CineKind::CloseToTarget || self == CineKind::TargetClose {
            0.0
        } else {
            20.0
        }
    }
}

/// The end-velocity part of the Translate_Axis profile (`F-76061` / `F-58409` with d = 0,
/// v0 = 0, v1 = +404): `vm = -v1/2, a1 = -v1/T, a2 = 3v1/T` give p = v1 T h(u), h(u) = -u^2/2 (u <= 1/2),
/// -1/8 - w/2 + 1.5 w^2 with w = u - 1/2; 0 at both ends, slope v1 at u = 1.
fn ta_end(v1: f32, t: f32, tau: f32) -> f32 {
    let u = (tau / t).clamp(0.0, 1.0);
    let h = if u <= 0.5 {
        -0.5 * u * u
    } else {
        let w = u - 0.5;
        -0.125 - 0.5 * w + 1.5 * w * w
    };
    v1 * t * h
}

/// A cinematic warp flight from the render camera S to the start scene's camera E (`F-18267`):
/// the chosen state with output warper+192, start S, end E, T.720(V = req+80, the scene camera's
/// velocity from `F-81729`), and the lens's Lerp_Float (warper+292, `F-59974`).
///
/// T.720 (Lean / Parallel `F-79404`: every child; Serial `F-90176`: 720 to its LAST child,
/// 724 to all) gives every Translate_Axis that gets it the vector (+448, the target_velocity direction)
/// and, with 720 (`F-16072`), the end speed v1 = its projection (vfunc 860) - so with V != 0 the
/// pieces arrive moving with the scene camera; the target_velocity Axis2Stages moves `-k` then `+k`
/// along n(V) (net 0, the second stage ending at |V|). V = 0 adds nothing.
/// Not handed in: the chooser's T.736 (the render camera's own velocity, playerMgr+0x51420) - assumed 0
/// (the map camera at rest).
#[derive(Clone)]
pub struct CineWarp {
    kind: CineKind,
    s: CamRows,
    e: CamRows,
    vel: Vec3,
    t: f32,
    out: CamRows,
    /// LookAtTarget's A (the render camera when it starts)
    lat_a: Option<CamRows>,
    /// Lean: history, last, previous out, its 30 Hz clock, roll
    hist: [f32; 4],
    last: f32,
    prev: CamRows,
    acc: f32,
    lam: f32,
    /// the outer Parallel's clock of the frame before (a delayed Translate_Axis shows 0 on the frame
    /// that crosses its delay)
    t_prev: f32,
    lens: NavLens,
}

impl CineWarp {
    /// `fov_from` = the current lens (warper+284), `fov_to` = the scene camera's (req+96); any unit.
    pub fn new(s: CamRows, e: CamRows, vel: Vec3, fov_from: f32, fov_to: f32) -> CineWarp {
        let kind = CineKind::choose(&s, &e);
        let total = kind.total();
        // Lerp_Float: 804(from), 812(to), 576(0.6 T) length, 580(0.3 T) delay, cosine ease (as NavLens)
        let lens = NavLens { from: fov_from, to: fov_to, delay: 0.3 * total, dur: 0.6 * total, clock: 0.0, deg: fov_from };
        if std::env::var("MCLA_NAV_LOG").is_ok() {
            let d = s.eye - e.eye;
            log::info!("cine warp: {:?} ({:.3} s) h {:.1} dist {:.1} f30 {:.4} from {:?} to {:?} vel {:?}", kind, total, s.eye.y, Vec3::new(d.x, 0.0, d.z).length(), sangle0(e.r2, s.r2), s.eye, e.eye, vel);
        }
        CineWarp { kind, s, e, vel, t: 0.0, out: s, lat_a: None, hist: [0.0; 4], last: 0.0, prev: s, acc: 0.0, lam: 0.0, t_prev: 0.0, lens }
    }

    pub fn kind(&self) -> CineKind {
        self.kind
    }

    /// STORY_LOCATION_CUT_SIMPLE (the garages' and hangouts' Warper_TransitionTo, `F-32081` case 17)
    /// with the render camera `s` within 100 of the destination `e`: Warp_Target_Close started by
    /// `F-47096` -> `F-98643`, whose lens (Lerp_Float, warper+292) goes from the current one to
    /// req+96 = 21.86547 (degrees; the 51 mm scene lens) with no delay over 0.8 T (0x820082E0).
    pub fn target_close(s: CamRows, e: CamRows, fov_from: f32, fov_to: f32) -> CineWarp {
        let kind = CineKind::TargetClose;
        let lens = NavLens { from: fov_from, to: fov_to, delay: 0.0, dur: 0.8 * kind.total(), clock: 0.0, deg: fov_from };
        CineWarp { kind, s, e, vel: Vec3::ZERO, t: 0.0, out: s, lat_a: None, hist: [0.0; 4], last: 0.0, prev: s, acc: 0.0, lam: 0.0, t_prev: 0.0, lens }
    }

    /// the flight's length (`CineKind::total`)
    pub fn total(&self) -> f32 {
        self.kind.total()
    }

    /// The eye at Lean clock `c`, outer (linear) clock `tl` (`tl_prev` = that clock a frame before).
    fn eye_at(&self, c: f32, tl: f32, tl_prev: f32) -> Vec3 {
        let (s, e, v) = (self.s, self.e, self.vel);
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
        let fl = flat_n(e.r2);
        let moving = v != Vec3::ZERO;
        let mut eye = match self.kind {
            CineKind::HighAltitude => {
                // Y (0, 3); Serial[along_final_look 500 (0, 2); Par[X, Z (0, 1)]]
                let p = flat(e.eye) + fl * 500.0;
                let xz = flat(s.eye) + (p - flat(s.eye)) * s_curve(c / 2.0) + (flat(e.eye) - p) * s_curve(c - 2.0);
                let mut q = Vec3::new(xz.x, s.eye.y + (e.eye.y - s.eye.y) * s_curve(c / 3.0), xz.z);
                if moving {
                    // (Y: v1 = V.y; the Serial's last child, the Parallel, gives X / Z theirs)
                    q += Vec3::new(ta_end(v.x, 1.0, c - 2.0), ta_end(v.y, 3.0, c), ta_end(v.z, 1.0, c - 2.0));
                }
                q
            }
            CineKind::LowAltitudeFarAway => {
                // AxisStages('Y', 2500, 1.5, 3.75); Serial[along_final_look 100 (0, 4.5); Par[X, Z (0, .75)]]
                // (.75 snaps to 22/30)
                let t2 = ui_time(0.75);
                let y = s.eye.y + (e.eye.y + 2500.0 - s.eye.y) * s_curve(c / 1.5) - 2500.0 * s_curve((c - 1.5) / 3.75);
                let p = flat(e.eye) + fl * 100.0;
                let xz = flat(s.eye) + (p - flat(s.eye)) * s_curve(c / 4.5) + (flat(e.eye) - p) * s_curve((c - 4.5) / t2);
                let mut q = Vec3::new(xz.x, y, xz.z);
                if moving {
                    q += Vec3::new(ta_end(v.x, t2, c - 4.5), ta_end(v.y, 3.75, c - 1.5), ta_end(v.z, t2, c - 4.5));
                }
                q
            }
            CineKind::SouthSideCloseCar => {
                // X, Z (0, 3.003 -> 3); AxisStages('Y', 100, 2.5025 -> 2.5, .533866 -> 16/30);
                // Forward(+300 (0, 2.002 -> 2), -300 (0, 1))
                let (txz, y1, y2, f1) = (ui_time(3.003), ui_time(2.5025), ui_time(0.533_866), ui_time(2.002));
                let xz = flat(s.eye) + (flat(e.eye) - flat(s.eye)) * s_curve(c / txz);
                let y = s.eye.y + (e.eye.y + 100.0 - s.eye.y) * s_curve(c / y1) - 100.0 * s_curve((c - y1) / y2);
                let mut q = Vec3::new(xz.x, y, xz.z) + fl * (300.0 * s_curve(c / f1) - 300.0 * s_curve(c - f1));
                if moving {
                    q += Vec3::new(ta_end(v.x, txz, c), ta_end(v.y, y2, c - y1), ta_end(v.z, txz, c)) + fl * ta_end(fl.dot(v), 1.0, c - f1);
                }
                q
            }
            CineKind::SharpAngle => {
                // UITransition_Translate_Spline_ExitSharpAngle total 3 (as ExitKind::SharpAngle, x = c / 3);
                // Y (0, 3.05 -> 91/30)
                let sign = if sangle0(e.r2, s.r2) >= 0.0 { 1.0 } else { -1.0 };
                let (p0, p2) = (s.eye, e.eye);
                let p1 = e.eye + e.r0.normalize_or(Vec3::X) * (100.0 * sign) - flat(e.eye - s.eye).normalize_or_zero() * 150.0;
                let x = c / 3.0;
                let q = if x < 0.9999 {
                    let seg = (2.0 * x) as i32;
                    let u = 2.0 * x - seg as f32;
                    if seg == 0 {
                        catmull([p0 * 2.0 - p1, p0, p1, p2], u)
                    } else {
                        catmull([p0, p1, p2, p2 * 2.0 - p1], u)
                    }
                } else {
                    p2
                };
                let ty = ui_time(3.05);
                let mut y = s.eye.y + (e.eye.y - s.eye.y) * s_curve(c / ty);
                if moving {
                    y += ta_end(v.y, ty, c);
                }
                Vec3::new(q.x, y, q.z)
            }
            CineKind::AboveCar => {
                // X, Y, Z (0, 3); Forward(+300 (0, .75 -> 22/30), -300 (0, 2.25 -> 67/30))
                let (f1, f2) = (ui_time(0.75), ui_time(2.25));
                let mut q = s.eye + (e.eye - s.eye) * s_curve(c / 3.0) + fl * (300.0 * s_curve(c / f1) - 300.0 * s_curve((c - f1) / f2));
                if moving {
                    q += Vec3::new(ta_end(v.x, 3.0, c), ta_end(v.y, 3.0, c), ta_end(v.z, 3.0, c)) + fl * ta_end(fl.dot(v), f2, c - f1);
                }
                q
            }
            CineKind::CloseToTarget | CineKind::TargetClose => {
                // UITransition_Lerp (start `F-50507`, update `F-33191`, `F-31424`): its own
                // ease_in_out clock over 2 s (1 s in Warp_Target_Close) on top of the Lean's; eye = S + (E - S) p
                s.eye + (e.eye - s.eye) * self.lerp_p(c)
            }
        };
        if moving {
            // Axis2Stages('target_velocity', ..): 'set' -k (D1, T1) then +k (D2, T2) along n(V), the
            // second stage ending at |V| (outside the Lean on the linear clock, inside it for SharpAngle
            // and CloseToTarget)
            let (k, d1, t1, d2, t2, ct, ct_prev) = match self.kind {
                CineKind::HighAltitude | CineKind::AboveCar => (10.0, 0.0, 2.0, 0.0, 1.0, tl, tl_prev),
                CineKind::LowAltitudeFarAway => (10.0, 0.0, 1.0, 3.25, 1.0, tl, tl_prev),
                CineKind::SouthSideCloseCar => (10.0, 0.0, ui_time(2.002), 0.0, ui_time(1.034_366_7), tl, tl_prev),
                CineKind::SharpAngle => (10.0, 0.0, 2.0, 0.0, ui_time(1.05), c, self.kind.clock(tl_prev)),
                CineKind::CloseToTarget => (2.0, 0.0, 1.5, 0.0, ui_time(0.55), c, self.kind.clock(tl_prev)),
                // (Warp_Target_Close has no target_velocity stages)
                CineKind::TargetClose => (0.0, 0.0, 1.0, 0.0, 1.0, c, c),
            };
            let s2 = d1 + t1 + d2;
            let tau2 = if d2 > 0.0 && ct_prev < s2 { 0.0 } else { ct - s2 };
            let p = -k * s_curve((ct - d1) / t1) + k * s_curve(tau2 / t2) + ta_end(v.length(), t2, tau2);
            eye += v.normalize_or_zero() * p;
        }
        eye
    }

    /// The Lerp's progress at Lean clock `c`: (1 - cos(pi min(1, c / T))) / 2 (its ease_in_out,
    /// `F-39265`, total T = 2; 1 in Warp_Target_Close).
    fn lerp_p(&self, c: f32) -> f32 {
        let tl = if self.kind == CineKind::TargetClose { 1.0 } else { 2.0 };
        (1.0 - (PI * (c / tl).clamp(0.0, 1.0)).cos()) * 0.5
    }

    /// One frame: the camera and the lens, or None once the flight is over (t >= total; the scene
    /// camera, now at scene t 1.0 = E, takes over).
    pub fn step(&mut self, dt: f32) -> Option<(CamRows, f32)> {
        self.t += dt;
        let total = self.kind.total();
        if self.t >= total - 1e-4 {
            return None;
        }
        let fov = self.lens.step(dt);
        let c = self.kind.clock(self.t);
        let t_prev = self.t_prev;
        self.t_prev = self.t;
        let (s, e) = (self.s, self.e);
        let prev_eye = self.out.eye;
        let cur = self.out;
        let eye = self.eye_at(c, self.t, t_prev);
        let ey = |r: CamRows| r.r2.x.atan2(r.r2.z);
        // LookAtTarget (`F-47360`): from A toward looking at L = E.r3 + n(E.r2) target (796 sets
        // +452 = 0: req+10 = 0) from the camera's last place, up E.r1, cosine weight over `ttl` (not
        // snapped) from `d0`, w = 1 once its snapped delay + total `done` is passed
        let look = |this: &mut CineWarp, a: CamRows, d0: f32, ttl: f32, done: f32, target: f32, delay_up: bool| {
            let a = *this.lat_a.get_or_insert(a);
            let l = e.eye + e.r2.normalize_or(Vec3::Z) * target;
            let b = CamRows::look_at(prev_eye, l, e.r1);
            let w = if c >= done { 1.0 } else { (1.0 - (PI * ((c - d0) / ttl).clamp(0.0, 1.0)).cos()) * 0.5 };
            blend_rows(a, b, w, delay_up)
        };
        let mut out = match self.kind {
            CineKind::HighAltitude => {
                // LookAtTarget(1, 1, 2, -50, final_up, final_look, 1, delay_up_rotate)
                if c >= 1.0 {
                    look(self, cur, 1.0, 1.0, 3.0, -50.0, true)
                } else {
                    s
                }
            }
            CineKind::LowAltitudeFarAway => {
                // Serial[Par[Rotate x -1.370796 (0, 2.25 -> 67/30), Rotate y final_look (0, 3.75)];
                //        LookAtTarget(0, 1.5, 1.5, -15)]
                if c >= 3.75 {
                    look(self, cur, 3.75, 1.5, 5.25, -15.0, false)
                } else {
                    let dx = -1.370_796 - (-s.r2.y).clamp(-1.0, 1.0).asin();
                    let dy = wrap_pi(ey(e) - ey(s));
                    s.pitch(dx * s_curve(c / ui_time(2.25))).roty(dy * s_curve(c / 3.75))
                }
            }
            CineKind::SouthSideCloseCar => {
                // LookAtTarget(0, 3.003, 3.003 -> 3, -50) from the render camera at the start
                look(self, cur, 0.0, 3.003, ui_time(3.003), -50.0, false)
            }
            CineKind::SharpAngle => {
                // LookAtTarget(0, 1.5015, 3.003 -> 3, -50)
                look(self, cur, 0.0, 1.5015, ui_time(3.003), -50.0, false)
            }
            CineKind::AboveCar => {
                // Serial[Rotate y final_look (0, 2.25 -> 67/30); LookAtTarget(0, .75, .75 -> 22/30, -50)]
                let (tr, tlat) = (ui_time(2.25), ui_time(0.75));
                let d = wrap_pi(ey(e) - ey(s));
                if c >= tr {
                    look(self, s.roty(d), tr, 0.75, tr + tlat, -50.0, false)
                } else {
                    s.roty(d * s_curve(c / tr))
                }
            }
            CineKind::CloseToTarget | CineKind::TargetClose => {
                // the Lerp's rows: slerp of the start / end quaternions (`F-50812`, shorter way,
                // `F-97287`)
                let q = |r: CamRows| glam::Quat::from_mat3(&glam::Mat3::from_cols(r.r0, r.r1, r.r2)).normalize();
                let m = glam::Mat3::from_quat(q(s).slerp(q(e), self.lerp_p(c)));
                CamRows { r0: m.x_axis, r1: m.y_axis, r2: m.z_axis, eye }
            }
        };
        out.eye = eye;
        // Lean (`F-10316`) at 30 Hz, as NavExit
        let lean_factor = self.kind.lean_factor();
        self.acc += dt;
        while self.acc >= 1.0 / 30.0 {
            self.acc -= 1.0 / 30.0;
            let d = sangle(self.prev.r2, out.r2);
            self.prev = out;
            self.lam = if c >= total - 0.5 {
                (total - (c + 0.0333)).max(0.0) * 2.0 * self.last
            } else {
                let l = ((self.hist.iter().sum::<f32>() + lean_factor * d) / 5.0).clamp(-PI / 4.0, PI / 4.0);
                self.hist.rotate_right(1);
                self.hist[0] = l;
                self.last = l;
                l
            };
        }
        let mut k = 2.0 * (1.0 - out.r2.y);
        if !(0.0..=1.0).contains(&k) {
            k = 1.0;
        }
        out = out.roll(k * self.lam.clamp(-PI / 4.0, PI / 4.0));
        self.out = out;
        Some((out, fov))
    }
}

/// The climb of Warper_Warp('DEFAULT_WITH_CUT') (request type 4, `F-32081` case A-30830): one
/// of three `Warp_*_Cinematically` states of tune/ui/transitions/Main.sc.xml picked by `F-81678`
/// (req+9 = 1, type not 8 / 9, the FromCarToLowWaitPosition distance 0): f1 = `F-32060(S.r2,
/// (0, 0, -1))` (S = the render camera, playerMgr+0x513E0; constants 0x82000ED4 = 0, 0x82003770 = -1),
/// 0 < f1 < 1.57 -> Sharp_Angle_Right, -1.57 < f1 < 0 -> Sharp_Angle_Left, else FocusOnCar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClimbKind {
    /// Warp_FocusOnCar_Cinematically (Main.sc.xml l.421; 1.30 s)
    FocusOnCar,
    /// Warp_Sharp_Angle_Right_Cinematically (l.467; 2.05 -> 61/30 s)
    SharpAngleRight,
    /// Warp_Sharp_Angle_Left_Cinematically (l.513; 61/30 s)
    SharpAngleLeft,
}

impl ClimbKind {
    pub fn choose(s: &CamRows) -> ClimbKind {
        let f1 = sangle0(s.r2, Vec3::new(0.0, 0.0, -1.0));
        if f1 < 1.57 && f1 > 0.0 {
            ClimbKind::SharpAngleRight
        } else if f1 > -1.57 && f1 < 0.0 {
            ClimbKind::SharpAngleLeft
        } else {
            ClimbKind::FocusOnCar
        }
    }

    /// The top Parallel's length (max of delay + total over the children, times snapped by
    /// `ui_time`): FocusOnCar's Y 1.30 (X, Z and LookAtTarget 1.25 -> 37/30, Axis2Stages 22/30 +
    /// 16/30, GPSJump .6); the Sharp_Angle ones' Y 2.05 -> 61/30.
    pub fn total(self) -> f32 {
        match self {
            ClimbKind::FocusOnCar => ui_time(1.30),
            _ => ui_time(2.05),
        }
    }

    /// ease_in on the top Parallel (`F-17563`): C = T (1 - cos(pi t / 2T)).
    fn clock(self, t: f32) -> f32 {
        let tl = self.total();
        tl * (1.0 - ((t / tl).clamp(0.0, 1.0) * PI * 0.5).cos())
    }
}

/// The climb's flight (`F-98643`: T.648(warper+192), 632(S = the render camera), 636(E = the
/// player car's matrix, `F-35658`: rows right, up, back), 736 / 744(V = the render camera's velocity,
/// playerMgr+0x51420 = its last frame's step over the unscaled frame time, y 0: the Translate_Axis
/// children carry it - `ta_carry`), no 720: the target_velocity Axis2Stages move nothing).
/// Lens (warper+292, Lerp_Float): from playerMgr+0x51430 to req+96 = 25, no delay, over 0.8 T
/// (0x820082E0). No Lean (no roll). When it is over (`F-99030`) the camera keeps its last matrix
/// and Warper_IsReadyToLoadStuff turns true.
#[derive(Clone)]
pub struct CineClimb {
    kind: ClimbKind,
    s: CamRows,
    e: CamRows,
    t: f32,
    c_prev: f32,
    out: CamRows,
    lens: NavLens,
    v0: Vec3,
}

impl CineClimb {
    /// `s` = the render camera, `e` = the player car's matrix, the lens from `fov_from` to `fov_to`
    /// (25 degrees; any unit), `v` = the render camera's velocity (y set to 0 here).
    pub fn new(s: CamRows, e: CamRows, fov_from: f32, fov_to: f32, v: Vec3) -> CineClimb {
        let kind = ClimbKind::choose(&s);
        let lens = NavLens { from: fov_from, to: fov_to, delay: 0.0, dur: 0.8 * kind.total(), clock: 0.0, deg: fov_from };
        CineClimb { kind, s, e, t: 0.0, c_prev: 0.0, out: s, lens, v0: Vec3::new(v.x, 0.0, v.z) }
    }

    pub fn kind(&self) -> ClimbKind {
        self.kind
    }

    /// the time flown
    pub fn t(&self) -> f32 {
        self.t
    }

    pub fn total(&self) -> f32 {
        self.kind.total()
    }

    /// The eye at child clock `c` (`c_prev` = that clock a frame before).
    fn eye_at(&self, c: f32, c_prev: f32) -> Vec3 {
        let (s, e) = (self.s, self.e);
        // Translate_Axis Y, 'world': d = 850 - S.y
        let ty = self.kind.total();
        let y = s.eye.y + (850.0 - s.eye.y) * s_curve(c / ty);
        match self.kind {
            ClimbKind::FocusOnCar => {
                // Axis.sc X (0, 1.25 -> 37/30) target 0, Z (0, 37/30) target 1300 (d = E - S + target)
                // (X and Z carry V.x / V.z over their 37/30)
                let (t, v) = (ui_time(1.25), self.v0);
                Vec3::new(s.eye.x + (e.eye.x - s.eye.x) * s_curve(c / t) + ta_carry(v.x, t, c), y, s.eye.z + (e.eye.z + 1300.0 - s.eye.z) * s_curve(c / t) + ta_carry(v.z, t, c))
            }
            ClimbKind::SharpAngleRight | ClimbKind::SharpAngleLeft => {
                // X (0, 2); Z (.2, 1.8) - shows 0 on the frame that crosses its delay
                let v = self.v0;
                let x = s.eye.x + (e.eye.x - s.eye.x) * s_curve(c / 2.0) + ta_carry(v.x, 2.0, c);
                let d = ui_time(0.2);
                let z = if c_prev < d { s.eye.z } else { s.eye.z + (e.eye.z + 1300.0 - s.eye.z) * s_curve((c - d) / 1.8) + ta_carry(v.z, 1.8, c - d) };
                // Axis2Stages('final_look', -+100, 0, .5, +-100, 0, 1.5) along n(flat E.r2) and
                // ('final_side', -+300, 0, .5, +-300, 0, 1.5) along n(E.r0), 'set' (d = target);
                // Right: -100 / -300 first, Left: +100 / +300
                let k = if self.kind == ClimbKind::SharpAngleRight { -1.0 } else { 1.0 };
                let st = k * (s_curve(c / 0.5) - s_curve((c - 0.5) / 1.5));
                // (their first stages carry V along their axes; the second ones nothing)
                let (look, side) = (flat_n(e.r2), e.r0.normalize_or(Vec3::X));
                Vec3::new(x, y, z) + look * (100.0 * st + ta_carry(look.dot(v), 0.5, c)) + side * (300.0 * st + ta_carry(side.dot(v), 0.5, c))
            }
        }
    }

    /// The rows: LookAtTarget(0, .1, T, target, 'Y', final_look) - from A = the render camera at the
    /// start toward looking from the camera's last place at L = E.r3 + n(E.r2) target with up (0, 1, 0);
    /// target -20 (FocusOnCar, total 37/30) / -10 (Sharp_Angle, total 2).
    fn rows_at(&self, c: f32, prev_eye: Vec3) -> CamRows {
        let e = self.e;
        let (target, done) = match self.kind {
            ClimbKind::FocusOnCar => (-20.0, ui_time(1.25)),
            _ => (-10.0, 2.0),
        };
        let l = e.eye + e.r2.normalize_or(Vec3::Z) * target;
        let b = CamRows::look_at(prev_eye, l, Vec3::Y);
        let w = if c >= done { 1.0 } else { (1.0 - (PI * (c / 0.1).clamp(0.0, 1.0)).cos()) * 0.5 };
        blend_rows(self.s, b, w, false)
    }

    /// One frame: the camera and the lens, or None once the climb is over (`end` holds from then on).
    pub fn step(&mut self, dt: f32) -> Option<(CamRows, f32)> {
        self.t += dt;
        if self.t >= self.kind.total() - 1e-4 {
            return None;
        }
        let fov = self.lens.step(dt);
        let c = self.kind.clock(self.t);
        let c_prev = std::mem::replace(&mut self.c_prev, c);
        let prev_eye = self.out.eye;
        let mut out = self.rows_at(c, prev_eye);
        out.eye = self.eye_at(c, c_prev);
        self.out = out;
        Some((out, fov))
    }

    /// The camera the climb leaves (its last update, the clock at T: every child complete) and the
    /// lens (at `to`, its 0.8 T being over).
    pub fn end(&self) -> (CamRows, f32) {
        let tl = self.kind.total();
        let mut out = self.rows_at(tl, self.out.eye);
        out.eye = self.eye_at(tl, tl);
        (out, self.lens.to)
    }
}

/// The opening's turn (Entering_Map_MaxHeight's Rotate children, rule_exitcam.md 7): from the game
/// camera's rows `s`, at real time `t` of the 1.5 s move.
pub fn nav_open_rows(s: CamRows, t: f32) -> CamRows {
    let c = 1.5 * (1.0 - (PI * t.min(1.5) / 3.0).cos());
    let dz = -s.r0.y.atan2(s.r1.y);
    let dx = -1.569_685_2 - (-s.r2.y).clamp(-1.0, 1.0).asin();
    let mut dy = -s.r2.x.atan2(s.r2.z);
    while dy > PI {
        dy -= std::f32::consts::TAU;
    }
    while dy < -PI {
        dy += std::f32::consts::TAU;
    }
    s.roll(dz * s_curve(c / 0.4)).pitch(dx * s_curve(c / 0.4)).roty(dy * s_curve(c / 0.75))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded frame (`work/mm/trace_idle.md`): the icon bases at camera height 1891 and
    /// the scales they give each kind.
    #[test]
    fn icon_scales_match_the_recorded_frame() {
        let mut m = std::collections::HashMap::new();
        m.insert("IconScalingOffset".to_string(), vec![-44.0]);
        m.insert("IconScalingFactor".to_string(), vec![8140.0]);
        m.insert("GlobalIconScaleMiniMap".to_string(), vec![1.0]);
        let t = Tune::new(&m);
        let (a, b) = Frame::bases(&t, 0, 1891.0);
        assert!((b - 1.22690).abs() < 1e-4);
        for (kind, want) in [(100.0, 50.746), (139.0, 70.537), (130.0, 65.969), (158.0, 80.178)] {
            assert!((kind * a - want).abs() < 2e-3, "{kind}: {} vs {want}", kind * a);
        }
        // (the GPS map: per icon |p - camera| * 7e-05 + 0.04; the hangout at 9104.5 m)
        assert!((139.0f32 * (9104.5 * 7e-5 + 0.04) - 94.157).abs() < 0.02);
    }

    /// Draw the recorded frame of `work/mm/trace_idle.md` (camera, elements and clock as recorded)
    /// to `MCLA_HUDMAP_OUT` (PNG, the 220 target over grey) for comparison with the original's
    /// screenshot `work/mm/cap/mm_idle.png` (the target sits at 77..297 x 463..683 there).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the extracted disc"]
    fn render_the_recorded_frame() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let tune = Tune::load(&dir);
        let mut assets = Assets::new(&dir);
        let roads = crate::mapmesh::RoadMap::load(&dir);
        let eye = Vec3::new(820.085, 1891.0, -809.289);
        let fwd = -Vec3::new(0.0276, 0.9983, -0.0511);
        let view_proj = Mat4::perspective_rh(35f32.to_radians(), 1.0, 10.0, 9000.0) * Mat4::look_at_rh(eye, eye + fwd, Vec3::Y);
        let back = Vec3::new(0.475, 0.0, -0.880);
        let (a, b) = Frame::bases(&tune, 0, eye.y);
        let f = Frame { mode: 0, view_proj, cam_pos: eye, theta: heading(back), scale_a: a, scale_b: b, nav_scale: None, t468: 5.101, tg: 5.101, frame_no: 0, race: false, race_checkpoints: false, police: 0, police0: 0, target_set: false, strobe: 0.0, pulse: 0.0, cap_lift: false, h473: false, strobe_on: true };
        let deg = |d: f32| Vec3::new(d.to_radians().sin(), 0.0, d.to_radians().cos());
        let mut els = Vec::new();
        for p in [(241.0, -918.4), (516.8, -1368.3), (519.7, -1290.6), (816.06, -1283.56), (1030.06, -307.56), (38.0, -415.0), (516.6, -751.6)] {
            els.push(Element::at(Kind::Gas, Vec3::new(p.0, 0.0, p.1)));
        }
        els.push(Element::at(Kind::Garage, Vec3::new(844.728, 18.28, -819.383)));
        let mut h = Element::at(Kind::Series, Vec3::new(729.046, 9.391, -285.351));
        h.colour = [1.0, 1.0, 0.0, 1.0];
        els.push(h);
        let mut r = Element::at(Kind::Racer(10), Vec3::new(396.1, 19.84, -727.6));
        r.back = deg(270.2);
        r.colour = DIFFICULTY[1];
        r.mission = true;
        els.push(r);
        let mut c = Element::at(Kind::Cop, Vec3::new(417.3, 14.9, -582.5));
        c.back = deg(-88.3);
        els.push(c);
        els.push(Element::at(Kind::Delivery, Vec3::new(844.728, 18.28, -819.383)));
        let mut p = Element::at(Kind::Player, Vec3::new(819.6, 17.6, -808.4));
        p.back = back;
        els.push(p);
        let q = build(&tune, &mut assets, &f, &els, &[], false);
        for d in &q.draws {
            let s = d.m.x_axis.truncate().length();
            eprintln!("{:<20} pos ({:8.1} {:6.1} {:8.1}) scale {:7.3} colour {:?}", d.mesh, d.m.w_axis.x, d.m.w_axis.y, d.m.w_axis.z, s, d.colour);
        }
        let mut t = Target::new(220);
        render_minimap(&mut t, &mut assets, &tune, roads.as_ref(), &f, &q, [0.4, 0.4, 0.4]);
        let px = t.resolve();
        // over mid grey, as straight alpha
        let mut out = Vec::with_capacity(px.len());
        for p in px.chunks_exact(4) {
            let a = p[3] as f32 / 255.0 * 0.9;
            for k in 0..3 {
                out.push((p[k] as f32 * a + 60.0 * (1.0 - a)) as u8);
            }
            out.push(255);
        }
        let path = std::env::var("MCLA_HUDMAP_OUT").unwrap_or_else(|_| "hudmap_frame.png".into());
        let file = std::fs::File::create(&path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 220, 220);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&out).unwrap();
    }

    /// The minimap pass's camera against the recorded idle frame (`work/mm/cap/mm_idle.bin`, draw 1515, an
    /// identity-world road draw: VS c8..c11 is the camera's view-projection, c12..c15 its ViewInverse).
    /// `minimap_view_proj` with the recorded eye and a look point along the recorded back vector
    /// (35 degree lens, square, near 10, far 9000) must give the recorded matrix.
    #[test]
    fn minimap_camera_matches_the_recorded_frame() {
        let eye = Vec3::new(820.085144, 1891.0, -809.289124);
        let back = Vec3::new(0.027579, 0.998312, -0.051105);
        let (vp, e) = minimap_view_proj(Vec3::ZERO, 0.0, eye, eye - back * 500.0, 35.0);
        assert!((e - eye).length() < 1e-3);
        let rec = [[-2.791112, -1.503681, -0.02761, -0.027579], [0.0, 0.184181, -0.999423, -0.998312], [-1.506222, 2.786402, 0.051162, 0.051105], [1069.980347, 3139.864258, 1943.944946, 1951.785034]];
        let got = vp.to_cols_array_2d();
        for c in 0..4 {
            for r in 0..4 {
                let tol = if c == 3 { 0.05 } else { 2e-4 };
                assert!((got[c][r] - rec[c][r]).abs() < tol, "col {c} row {r}: {} vs {}", got[c][r], rec[c][r]);
            }
        }
    }

    /// The icons of the recorded idle frame (`work/mm/cap/mm_idle.bin`, draws 1530..1552, R8 outside the
    /// Hollywood garage at night, speed 0, `docs/reference/minimap/trace_idle.md`): position, uniform
    /// scale, turn about y, colour and the target pixel of every draw, against the numbers read from the
    /// recording (`work/mm/mmdraws.py`). The scales are b x kind x 0.41361 (gas 100, hangout / garage 139,
    /// cop 158, player 130, racer 158 pulsing). Two entity headings (racer, cop) are inputs rounded to
    /// 0.1 degree, T468 is the recorded 5.101 (3 digits): the racer's pulse and cross-fade alphas carry that.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the extracted disc"]
    fn recorded_idle_frame_icons_match_the_original() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let tune = Tune::load(&dir);
        let mut assets = Assets::new(&dir);
        let eye = Vec3::new(820.085144, 1891.0, -809.289124);
        let back_cam = Vec3::new(0.027579, 0.998312, -0.051105);
        let (view_proj, _) = minimap_view_proj(Vec3::ZERO, 0.0, eye, eye - back_cam * 500.0, 35.0);
        let back = Vec3::new(0.47491, 0.0, -0.880034);
        let (a, b) = Frame::bases(&tune, 0, eye.y);
        let frame = |t468: f32| Frame { mode: 0, view_proj, cam_pos: eye, theta: heading(back), scale_a: a, scale_b: b, nav_scale: None, t468, tg: t468, frame_no: 0, race: false, race_checkpoints: false, police: 0, police0: 0, target_set: false, strobe: 0.0, pulse: 0.0, cap_lift: false, h473: false, strobe_on: true };
        let deg = |d: f32| Vec3::new(d.to_radians().sin(), 0.0, d.to_radians().cos());
        let mut els = Vec::new();
        for p in [(241.0, -918.4), (516.8, -1368.3), (519.7, -1290.6), (816.06, -1283.56), (1030.06, -307.56), (38.0, -415.0), (516.6, -751.6)] {
            els.push(Element::at(Kind::Gas, Vec3::new(p.0, 0.0, p.1)));
        }
        els.push(Element::at(Kind::Garage, Vec3::new(844.72803, 18.28, -819.383)));
        let mut h = Element::at(Kind::Series, Vec3::new(729.04602, 9.391, -285.35101));
        h.colour = [1.0, 1.0, 0.0, 1.0];
        els.push(h);
        let mut r = Element::at(Kind::Racer(10), Vec3::new(396.14804, 19.84267, -727.62256));
        r.back = deg(270.2);
        r.colour = DIFFICULTY[1];
        r.mission = true;
        els.push(r);
        let mut c = Element::at(Kind::Cop, Vec3::new(417.31116, 14.88972, -582.53674));
        c.back = deg(-88.3);
        els.push(c);
        els.push(Element::at(Kind::Delivery, Vec3::new(844.72803, 18.28, -819.383)));
        let mut p = Element::at(Kind::Player, Vec3::new(819.61023, 17.57223, -808.40912));
        p.back = back;
        els.push(p);
        // T468 was recorded to 3 digits (5.101): take the value within +-0.0005 for which the racer's three
        // alphas (ring 0.96784, cross-fade 0.6821 / 0.3179) all agree with the recording - two different
        // functions of the one clock, so a wrong rule would not find a common time.
        let mut found = None;
        for i in 0..=100 {
            let t = 5.1005 + i as f32 * 1e-5;
            let q = build(&tune, &mut assets, &frame(t), &els, &[], false);
            let al: Vec<f32> = q.draws[11..14].iter().map(|d| d.colour[3]).collect();
            if (al[0] - 0.96784).abs() < 1.5e-4 && (al[1] - 0.6821).abs() < 1.5e-4 && (al[2] - 0.3179).abs() < 1.5e-4 {
                found = Some(t);
                break;
            }
        }
        let t468 = found.expect("no clock value fits the racer's three alphas");
        let f = frame(t468);
        let q = build(&tune, &mut assets, &f, &els, &[], false);
        // mesh, x, y, z, scale, yaw (deg), pixel x, pixel y, rgba
        #[rustfmt::skip]
        let rec: [(&str, [f32; 3], f32, f32, [f32; 2], [f32; 4]); 19] = [
            ("icon_gasStation", [241.0, 1.0, -918.40002], 50.7457, -151.6465, [213.24139, 97.32384], [0.50196, 0.50196, 0.50196, 1.0]),
            ("icon_gasStation", [516.8, 1.0, -1368.3], 50.7457, -151.6465, [209.50398, 195.4308], [0.50196, 0.50196, 0.50196, 1.0]),
            ("icon_gasStation", [519.7, 1.0, -1290.6], 50.7457, -151.6465, [201.93849, 182.7774], [0.50196, 0.50196, 0.50196, 1.0]),
            ("icon_gasStation", [816.06, 1.0, -1283.56], 50.7457, -151.6465, [152.84954, 208.24082], [0.50196, 0.50196, 0.50196, 1.0]),
            ("icon_gasStation", [1030.06, 1.0, -307.56], 50.7457, -151.6465, [32.58941, 67.64325], [0.50196, 0.50196, 0.50196, 1.0]),
            ("icon_gasStation", [38.0, 1.0, -415.0], 50.7457, -151.6465, [200.63382, 0.11283], [0.50196, 0.50196, 0.50196, 1.0]),
            ("icon_gasStation", [516.6, 1.0, -751.6], 50.7457, -151.6465, [154.05315, 94.41169], [0.50196, 0.50196, 0.50196, 1.0]),
            ("icon_ring_hangout", [844.72803, 1.0, -819.383], 70.53652, -151.6465, [106.87449, 134.10946], [1.0, 1.0, 1.0, 1.0]),
            ("icon_garage", [844.72803, 1.0, -819.383], 70.53652, -151.6465, [106.87449, 134.10946], [1.0, 1.0, 1.0, 1.0]),
            ("icon_ring_hangout", [729.04602, 1.0, -285.35101], 70.53652, -151.6465, [79.28269, 38.31456], [1.0, 1.0, 0.0, 1.0]),
            ("icon_series", [729.04602, 1.0, -285.35101], 70.53652, -151.6465, [79.28269, 38.31456], [1.0, 1.0, 1.0, 1.0]),
            ("icon_ring_car", [396.14804, 20.84267, -727.62256], 82.89776, -135.1786, [171.94134, 79.58726], [1.0, 1.0, 0.0, 0.96784]),
            ("icon_series", [396.14804, 20.84267, -727.62256], 82.89776, -151.6465, [171.94134, 79.58726], [1.0, 1.0, 1.0, 0.6821]),
            ("icon_ingame1", [396.14804, 20.84267, -727.62256], 82.89776, -151.6465, [171.94134, 79.58726], [1.0, 1.0, 1.0, 0.3179]),
            ("caricon", [417.31116, 15.88972, -582.53674], 80.17821, 88.3355, [155.43933, 58.20567], [0.0, 0.26667, 0.51765, 0.91765]),
            ("icon_ring_hangout", [844.72803, 1.0, -819.383], 70.53652, -151.6465, [106.87449, 134.10946], [1.0, 1.0, 1.0, 1.0]),
            ("icon_delivery", [844.72803, 1.0, -819.383], 70.53652, -151.6465, [106.87449, 134.10946], [1.0, 1.0, 1.0, 0.73877]),
            ("icon_garage", [844.72803, 1.0, -819.383], 70.53652, -151.6465, [106.87449, 134.10946], [1.0, 1.0, 1.0, 0.26123]),
            ("caricon", [819.61023, 18.57223, -808.40912], 65.96941, -151.6465, [110.00001, 130.10724], [0.45098, 0.96078, 1.0, 1.0]),
        ];
        assert_eq!(q.draws.len(), rec.len(), "draw count");
        for (d, r) in q.draws.iter().zip(rec.iter()) {
            let pos = d.m.w_axis.truncate();
            let scale = d.m.x_axis.truncate().length();
            let yaw = d.m.x_axis.z.atan2(d.m.x_axis.x).to_degrees();
            let c = view_proj * pos.extend(1.0);
            let px = [110.0 + 110.0 * c.x / c.w, 110.0 - 110.0 * c.y / c.w];
            let tag = format!("{} at {pos:?}", d.mesh);
            assert_eq!(d.mesh, r.0, "{tag}: mesh");
            assert!((pos - Vec3::from(r.1)).length() < 2e-3, "{tag}: pos {pos:?} vs {:?}", r.1);
            assert!((scale - r.2).abs() < r.2 * 1e-4, "{tag}: scale {scale} vs {}", r.2);
            assert!((yaw - r.3).abs() < 0.06, "{tag}: yaw {yaw} vs {}", r.3);
            assert!((px[0] - r.4[0]).abs() < 0.02 && (px[1] - r.4[1]).abs() < 0.02, "{tag}: pixel {px:?} vs {:?}", r.4);
            for k in 0..4 {
                assert!((d.colour[k] - r.5[k]).abs() < 2e-4, "{tag}: colour {:?} vs {:?}", d.colour, r.5);
            }
        }
    }

    /// The GPS map's icon sizes against the recorded frame `work/mm/cap/gps_whole.bin` (whole city, camera
    /// (-327, 9000, 111.5) straight down; draws 4283..4337): every icon is kind x (|position - camera| x 7e-5 +
    /// 0.04) (`F-38206`) - the 27 gas stations (kind 100), the Hollywood garage and a hangout (139) and a
    /// cop (158), at the recorded world positions (y = the element's height + 1) and scales.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the extracted disc"]
    fn recorded_gps_frame_icon_scales_match_the_original() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let tune = Tune::load(&dir);
        let mut assets = Assets::new(&dir);
        let cam = Vec3::new(-327.0, 9000.0, 111.5);
        let view_proj = Mat4::perspective_rh(25f32.to_radians(), 16.0 / 9.0, 100.0, 20000.0) * Mat4::look_at_rh(cam, cam + Vec3::new(0.0, -1.0, -0.00111), Vec3::new(0.0, 0.0, -1.0));
        let (a, b) = Frame::bases(&tune, 1, cam.y);
        let f = Frame { mode: 1, view_proj, cam_pos: cam, theta: 0.0, scale_a: a, scale_b: b, nav_scale: Some((7e-5, 0.04)), t468: 5.0, tg: 5.0, frame_no: 0, race: false, race_checkpoints: false, police: 0, police0: 0, target_set: false, strobe: 0.0, pulse: 0.0, cap_lift: false, h473: false, strobe_on: true };
        // x, y, z (as drawn), scale
        let gas: [(f32, f32, f32, f32); 27] = [
(241.0000, 26.7600, -918.4000, 67.3569),
(516.8000, 32.7600, -1368.3000, 67.9001),
(519.7000, 31.7600, -1290.6000, 67.8230),
(816.0600, 32.7600, -1283.5601, 68.0346),
(-259.4000, -2.0600, -32.4000, 67.0312),
(-732.2000, 3.7600, -157.7000, 67.0727),
(-559.9000, 40.7600, -1784.9000, 68.1318),
(-2768.8999, -28.7600, -979.4000, 69.9226),
(1030.0601, 7.7600, -307.5600, 67.7329),
(38.0000, 7.7600, -415.0000, 67.1122),
(-598.9000, -9.7600, 339.9000, 67.1243),
(-1834.4000, 4.7000, -216.6000, 67.8933),
(516.6000, 21.4000, -751.6000, 67.4224),
(2348.3000, -2.5000, -263.3000, 69.8003),
(-607.0000, -21.1000, 1700.7000, 68.1569),
(-1112.4000, -20.8000, 1650.3000, 68.3001),
(-1715.6000, -11.0000, 1580.3000, 68.6513),
(-1796.6000, -9.8000, 1506.3000, 68.6505),
(-1830.6000, -9.4800, 895.3000, 68.1805),
(-163.6000, -13.8000, 514.3000, 67.1769),
(-197.6000, -13.8000, 518.3000, 67.1743),
(520.6000, -12.8000, 421.3000, 67.4120),
(1206.6000, -15.0000, 1401.3000, 68.6520),
(974.9000, -14.0000, 1188.3000, 68.2037),
(-1248.9000, -6.9000, 1280.3000, 67.9105),
(-156.9000, -20.6000, 1555.9000, 67.9666),
(946.3000, -13.6000, 456.0000, 67.7742)
        ];
        let mut els: Vec<Element> = gas.iter().map(|g| Element::at(Kind::Gas, Vec3::new(g.0, g.1 - 1.0, g.2))).collect();
        els.push(Element::at(Kind::Garage, Vec3::new(844.728, 18.28, -819.383)));
        els.push(Element::at(Kind::Cop, Vec3::new(1039.5265, 25.5811, -1165.9615)));
        let q = build(&tune, &mut assets, &f, &els, &[], false);
        let mut want: Vec<(f32, f32, f32, f32)> = gas.to_vec();
        want.push((844.728, 19.28, -819.383, 94.1569));
        want.push((844.728, 19.28, -819.383, 94.1569));
        want.push((1039.5265, 26.5811, -1165.9615, 107.7104));
        let got: Vec<&Draw> = q.draws.iter().collect();
        assert_eq!(got.len(), want.len(), "draws: {:?}", got.iter().map(|d| d.mesh).collect::<Vec<_>>());
        for (d, w) in got.iter().zip(&want) {
            let pos = d.m.w_axis.truncate();
            let s = d.m.x_axis.truncate().length();
            assert!((pos - Vec3::new(w.0, w.1, w.2)).length() < 2e-3, "{}: pos {pos:?} vs {w:?}", d.mesh);
            assert!((s - w.3).abs() < w.3 * 2e-4, "{} at {pos:?}: scale {s} vs {}", d.mesh, w.3);
        }
    }

    /// The overhead map's camera against the original's recorded frame (`work/mm/cap/ov_free1.bin`,
    /// free roam at the Hollywood garage, D-pad up): the pass's world-view-projection (VS c8..c11)
    /// and the 749x430 target pixels of four recorded icon draws. North up, east right.
    #[test]
    fn overhead_camera_matches_the_recorded_frame() {
        let vp = overhead_view_proj(Vec3::new(-299.0, 7399.0, 1.0), Vec3::new(-299.0, 0.0, -179.0), 35.0, 739.0 / 499.0);
        let rec = [[2.14158, 0.0, 0.0, 0.0], [0.0, 0.07713, -1.00082, -0.9997], [0.0, -3.17066, -0.02435, -0.02432], [640.33167, -567.54749, 7395.05225, 7396.83545]];
        let got = vp.to_cols_array_2d();
        for c in 0..4 {
            for r in 0..4 {
                let tol = if c == 3 { 0.05 } else { 2e-4 };
                assert!((got[c][r] - rec[c][r]).abs() < tol, "col {c} row {r}: {} vs {}", got[c][r], rec[c][r]);
            }
        }
        let px = |p: Vec3| {
            let c = vp * p.extend(1.0);
            (374.5 + 374.5 * c.x / c.w, 215.0 - 215.0 * c.y / c.w)
        };
        for (p, want) in [
            (Vec3::new(0.0, 0.0, 0.0), (406.9, 231.5)),
            (Vec3::new(844.7, 1.0, -819.4), (498.2, 156.1)),
            (Vec3::new(-3095.1, 1.0, 165.9), (71.1, 246.8)),
            (Vec3::new(-51.7, 1.0, 1330.0), (401.4, 354.7)),
        ] {
            let g = px(p);
            assert!((g.0 - want.0).abs() < 0.1 && (g.1 - want.1).abs() < 0.1, "{p:?}: {g:?} vs {want:?}");
        }
    }

    /// The icon bases on the overhead map (mode 2: GlobalIconScaleOverheadMap 1, camera y 7399).
    #[test]
    fn overhead_icon_bases() {
        let mut m = std::collections::HashMap::new();
        m.insert("IconScalingOffset".to_string(), vec![-44.0]);
        m.insert("IconScalingFactor".to_string(), vec![8140.0]);
        m.insert("GlobalIconScaleOverheadMap".to_string(), vec![1.0]);
        let t = Tune::new(&m);
        let (a, b) = Frame::bases(&t, 2, 7399.0);
        assert!((a - 1.116450).abs() < 1e-4, "{a}");
        assert!((b - 1.903563).abs() < 1e-4, "{b}");
    }

    /// A w x h target maps clip space onto its own width and height.
    #[test]
    fn rect_target_maps_clip_to_its_pixels() {
        let mut t = Target::rect(OVERHEAD_W, OVERHEAD_H);
        t.clear([0.4, 0.4, 0.4, 0.0]);
        // a small triangle round clip (0.5, 0.5) -> pixel (561.75, 107.5)
        let c = Vec3::new(0.5, 0.5, 0.5);
        t.ground(&[[c + Vec3::new(-0.01, -0.02, 0.0), c + Vec3::new(0.01, -0.02, 0.0), c + Vec3::new(0.0, 0.02, 0.0)]], Mat4::IDENTITY, [1.0, 0.0, 0.0, 1.0]);
        let px = t.resolve();
        assert_eq!(px.len(), OVERHEAD_W * OVERHEAD_H * 4);
        let at = |x: usize, y: usize| &px[(y * OVERHEAD_W + x) * 4..(y * OVERHEAD_W + x) * 4 + 4];
        assert_eq!(at(561, 107), &[255, 0, 0, 255]);
        assert_eq!(at(374, 215)[3], 0);
        assert_eq!(at(561, 300)[3], 0);
    }

    /// The delivery start scenes' big_map_up / big_map_down times.
    #[test]
    fn delivery_scene_map_events() {
        assert!(!cine_big_map("racestart/gen_prerace_delivery_1x1_01", 6.0));
        assert!(cine_big_map("racestart/gen_prerace_delivery_1x1_01", 6.1));
        assert!(cine_big_map("racestart/gen_prerace_delivery_1x1_01", 9.0));
        assert!(!cine_big_map("racestart/gen_prerace_delivery_1x1_01", 9.1));
        assert!(cine_big_map("racestart/gen_prerace_delivery_1x1_02", 5.6));
        assert!(!cine_big_map("racestart/gen_prerace_grid_1x1_01", 7.0));
        // (the intro table agrees with the map table; the countdown ends with the camera:
        // 12.267 / 11.667 / 12.167 s)
        for (e, (n, up, down)) in DELIVERY_INTROS.iter().zip(CINE_BIG_MAP.iter()) {
            assert!(e.scene.ends_with(n) && e.up == *up && e.down == *down);
            assert!(e.countdown > e.down && e.racename + e.racename_len < e.up);
        }
        assert!(((DELIVERY_INTROS[0].countdown + 3.0) - 12.267).abs() < 0.01);
        assert!(((DELIVERY_INTROS[1].countdown + 3.0) - 11.667).abs() < 0.01);
        assert_eq!(delivery_intro("racestart/gen_prerace_delivery_1x1_02").map(|e| e.up), Some(5.5));
        assert_eq!(delivery_intro("racestart/gen_prerace_grid_1x1_01"), None);
        assert_eq!(delivery_intro("racestart/cut_delivery_get_keys"), None);
    }

    /// The five recorded zoom steps of the original (rule_navcam.md 4.4): opened over the whole
    /// city with the look point left on the player, RT held 9, 9, 9, 10, 10 frames.
    #[test]
    fn nav_zoom_matches_the_recording() {
        let mut n = NavCam::default();
        let player = Vec3::new(819.6, 0.0, -808.4);
        n.open(Vec3::ZERO, Vec3::ZERO, player, 16.0 / 9.0);
        n.mv = None;
        let want = [(-81.7, 6726.2, -709.9), (208.8, 4633.0, -633.4), (448.9, 2886.6, -110.0), (632.8, 1520.3, 300.0), (714.2, 886.2, 490.6)];
        for (frames, w) in [9, 9, 9, 10, 10].iter().zip(want) {
            for _ in 0..*frames {
                n.update(1.0 / 30.0, true, false, Vec2::ZERO, 16.0 / 9.0);
            }
            let e = n.eye;
            assert!((e.x - w.0).abs() < 0.5 && (e.y - w.1).abs() < 0.5 && (e.z - w.2).abs() < 0.5, "{e:?} vs {w:?}");
        }
    }

    /// List the map's meshes: parts, extents, textures (MCLA_DATA_DIR).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the extracted disc"]
    fn list_meshes() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let mut a = Assets::new(&dir);
        for name in ["navsys_cursor_BG", "navsys_cursor", "target_ripple", "checkpoint_sphere", "icon_ring_car", "caricon"] {
            match a.mesh(name) {
                Some(m) => {
                    for p in &m.parts {
                        let (mn, mx) = p.pos.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), q| (a.min(*q), b.max(*q)));
                        eprintln!("{name}: {} verts {} idx tex {} ({}) min {mn:?} max {mx:?}", p.pos.len(), p.idx.len(), p.tex_name, p.tex.as_ref().map_or("none".to_string(), |t| format!("{}x{}", t.w, t.h)));
                        eprintln!("   pos {:?} uv {:?}", p.pos, p.uv);
                    }
                }
                None => eprintln!("{name}: none"),
            }
        }
    }

    /// The whole-city exit at the Hollywood garage (rule_exitcam.md 5, the 30 fps table).
    /// The opening lens as recorded from the original (work/mm/exitcam/c7.jsonl: driving lens 25.452
    /// degrees, one value per 30 Hz frame).
    #[test]
    fn map_lens_eases_to_25_like_the_recording() {
        let rec = [25.452255, 25.45015, 25.44387, 25.433535, 25.419336, 25.401537, 25.380472, 25.356531, 25.33016, 25.301853, 25.272135, 25.24156,
            25.210695, 25.18012, 25.150402, 25.122095, 25.095724, 25.071783, 25.050718, 25.032919, 25.01872, 25.008385, 25.002106, 25.0];
        let mut l = NavLens::new(25.452255);
        for (k, want) in rec.iter().enumerate() {
            let v = l.step(1.0 / 30.0);
            assert!((v - want).abs() < 2e-5, "frame {k}: {v} vs {want}");
        }
        assert_eq!(l.step(1.0 / 30.0), 25.0);
    }

    #[test]
    fn xray_block_skip_changes_nothing() {
        // (a made-up city: towers on a grid; the skipping march against a plain one)
        let (gw, gh) = (200usize, 160usize);
        let mut h = vec![0.0f32; gw * gh];
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16777216.0
        };
        for v in h.iter_mut() {
            let r = rnd();
            *v = if r > 0.97 { 40.0 + 250.0 * rnd() } else if r > 0.8 { 15.0 * rnd() } else { 0.0 };
        }
        let hf = Heights { min_x: -600.0, min_z: -480.0, cell: 6.0, gw, gh, h: Arc::new(h), coarse: Default::default() };
        let plain = |p: Vec3, cam: Vec3| -> bool {
            let d = cam - p;
            let flat = Vec2::new(d.x, d.z).length();
            if p.y >= 330.0 {
                return false;
            }
            let span = if flat < 1e-3 { 0.0 } else { ((330.0 - p.y) / d.y.max(1e-3) * flat).min(flat) };
            let n = ((span / hf.cell).ceil() as usize).min(160);
            for k in 1..=n.max(1) {
                let f = (k as f32 * hf.cell / flat.max(1e-3)).min(1.0);
                let q = p + d * f;
                if q.y < hf.at(q.x, q.z) - 0.5 {
                    return true;
                }
                if f >= 1.0 {
                    break;
                }
            }
            flat < 1e-3 && p.y < hf.at(p.x, p.z) - 0.5
        };
        let mut hidden = 0;
        for _ in 0..20000 {
            let p = Vec3::new(-550.0 + 1100.0 * rnd(), 60.0 * rnd(), -430.0 + 860.0 * rnd());
            let cam = Vec3::new(p.x + 400.0 * (rnd() - 0.5), 850.0 + 8000.0 * rnd(), p.z + 1300.0 * rnd());
            let a = hf.hidden(p, cam);
            assert_eq!(a, plain(p, cam), "{p:?} {cam:?}");
            hidden += a as usize;
        }
        assert!(hidden > 100, "{hidden}");
    }

    /// The closing lens as recorded (HighAltitude, T 3.0, work/mm/exitcam/fx_lens.txt: driving lens
    /// 25.3995): 25 until 0.6 s, then eased over 2.1 s.
    #[test]
    fn closing_lens_matches_the_recording() {
        let r = exit_rec(EXIT_RECS[2].1);
        let mut l = NavLens::closing(25.39951, 0.6, 2.1);
        for (k, (_, _, want)) in r.frames.iter().enumerate() {
            let v = l.step(1.0 / 30.0);
            assert!((v - want).abs() < 1e-4, "frame {k}: {v} vs {want}");
        }
        assert_eq!(r.frames.len(), 89);
    }

    /// The worked case (rule_exitcam.md 5) against the original's recorded frames (fx_high.txt: the same
    /// start and end).
    #[test]
    fn exit_flight_matches_the_decoded_table() {
        let s = CamRows { r0: Vec3::X, r1: Vec3::new(0.0, 0.00111, -1.0), r2: Vec3::new(0.0, 1.0, 0.00111), eye: Vec3::new(-327.0, 9000.0, 111.5) };
        let e = CamRows { r0: Vec3::new(-0.880, 0.017, -0.474), r1: Vec3::new(-0.018, 0.997, 0.070), r2: Vec3::new(0.474, 0.070, -0.878), eye: Vec3::new(823.773, 19.520, -816.039) };
        let mut x = NavExit::new(s, e);
        let mut rows = Vec::new();
        for _ in 0..89 {
            rows.push(x.step(1.0 / 30.0).expect("still flying"));
        }
        // (frame k = t (k + 1) / 30)
        let at = |t: f32| rows[(t * 30.0).round() as usize - 1];
        for (t, want) in [(0.5, Vec3::new(-327.0, 7796.844, 111.5)), (0.533, Vec3::new(-317.857, 7635.398, 103.938)), (0.833, Vec3::new(-28.704, 5792.060, -135.236)),
                          (1.633, Vec3::new(847.459, 1100.185, -859.872)), (2.333, Vec3::new(825.699, 84.843, -819.602)), (2.933, Vec3::new(823.774, 19.526, -816.039))] {
            let c = at(t);
            assert!((c.eye - want).length() < 0.1, "t {t}: {:?} vs {want:?}", c.eye);
        }
        let pitch = |c: CamRows| (-c.r2.y).clamp(-1.0, 1.0).asin().to_degrees();
        assert!((pitch(at(0.533)) + 89.94).abs() < 0.2);
        assert!((pitch(at(2.333)) + 77.55).abs() < 0.1, "{}", pitch(at(2.333)));
        assert!((pitch(at(2.933)) + 4.16).abs() < 0.1, "{}", pitch(at(2.933)));
        assert!(x.step(1.0 / 30.0).is_none(), "over at 3.0 s");
    }

    /// the GPS map camera at (x, h, z): looking down, north up (r2 = n(0, 1, 0.00111))
    fn map_cam(x: f32, h: f32, z: f32) -> CamRows {
        let eye = Vec3::new(x, h, z);
        CamRows::look_at(eye, eye - Vec3::new(0.0, 1.0, 0.00111), Vec3::Y)
    }

    /// the chase camera at the Hollywood garage (rule_exitcam.md 5): looks SSW, the car faces south
    fn chase_south() -> CamRows {
        CamRows { r0: Vec3::new(-0.880, 0.017, -0.474), r1: Vec3::new(-0.018, 0.997, 0.070), r2: Vec3::new(0.474, 0.070, -0.878), eye: Vec3::new(823.773, 19.520, -816.039) }
    }

    /// the same place, looking NNE (the car faces north)
    fn chase_north() -> CamRows {
        let eye = Vec3::new(823.773, 19.520, -816.039);
        CamRows::look_at(eye, eye - Vec3::new(-0.474, 0.070, 0.878), Vec3::Y)
    }

    fn fly(x: &mut NavExit, n: usize) -> Vec<CamRows> {
        (0..n).map(|_| x.step(1.0 / 30.0).expect("still flying")).collect()
    }

    fn pitch_of(c: CamRows) -> f32 {
        (-c.r2.y).clamp(-1.0, 1.0).asin().to_degrees()
    }

    /// heading of the view: 0 = north (-z), + east
    fn look_hdg(c: CamRows) -> f32 {
        (-c.r2.x).atan2(c.r2.z).to_degrees()
    }

    /// `F-60031`'s choice (rule_exitcam.md 10; numbers from work/mm/exitcam/sim_exit_low.py)
    #[test]
    fn exit_choice_follows_the_tree() {
        let e = chase_south();
        // whole city: HighAltitude; right above the car: AboveCar
        assert_eq!(ExitKind::choose(&map_cam(-327.0, 9000.0, 111.5), &e), ExitKind::HighAltitude);
        assert_eq!(ExitKind::choose(&map_cam(820.0, 1891.0, -809.0), &e), ExitKind::AboveCar);
        // 2500 m away at 3000 m: LowAltitude_FarAway (f1 -0.6435, f30 -2.6466)
        assert_eq!(ExitKind::choose(&map_cam(823.773 - 1500.0, 3000.0, -816.039 + 2000.0), &e), ExitKind::LowAltitudeFarAway);
        // 1204 m south at 1500 m (> 0.25 h + 600 = 975), f1 0.0831: SharpAngle when the car faces
        // south (|f30| 2.6466 > 2.5), SouthSideCloseCar when it faces north (f30 0.4950)
        let s = map_cam(823.773 + 100.0, 1500.0, -816.039 + 1200.0);
        assert!((sangle0(s.r2, s.eye - e.eye) - 0.0831).abs() < 1e-3);
        assert!((sangle0(e.r2, s.r2) + 2.6466).abs() < 1e-3);
        assert_eq!(ExitKind::choose(&s, &e), ExitKind::SharpAngle);
        assert_eq!(ExitKind::choose(&s, &chase_north()), ExitKind::SouthSideCloseCar);
        // 1217 m east (|f1| > 0.75): AboveCar
        assert_eq!(ExitKind::choose(&map_cam(823.773 + 1200.0, 1500.0, -816.039 - 200.0), &e), ExitKind::AboveCar);
        // 1300 m from 3000 m height (< 0.25 h + 600 = 1350): AboveCar
        assert_eq!(ExitKind::choose(&map_cam(823.773, 3000.0, -816.039 + 1300.0), &e), ExitKind::AboveCar);
    }

    /// Exiting_Map_GPS_LowAltitude_FarAway, 30 fps (sim_exit_low.py low)
    #[test]
    fn exit_low_altitude_far_away() {
        let mut x = NavExit::new(map_cam(823.773 - 1500.0, 3000.0, -816.039 + 2000.0), chase_south());
        assert_eq!(x.kind(), ExitKind::LowAltitudeFarAway);
        assert_eq!(x.total(), 3.5);
        let r = fly(&mut x, 104);
        for (k, want, p, h) in [
            (19, Vec3::new(-310.229, 2518.719, 690.133), -80.76, -51.64),
            (29, Vec3::new(116.589, 2304.369, 114.242), -78.54, -104.90),
            (52, Vec3::new(782.992, 830.250, -784.913), -78.54, -151.63),
            (74, Vec3::new(862.337, 115.630, -887.471), -55.78, -151.55),
            (95, Vec3::new(824.154, 20.323, -816.745), -8.17, -151.63),
            (103, Vec3::new(823.773, 19.520, -816.039), -4.02, -151.64),
        ] {
            assert!((r[k].eye - want).length() < 0.3, "frame {k}: {:?} vs {want:?}", r[k].eye);
            assert!((pitch_of(r[k]) - p).abs() < 0.3, "frame {k}: pitch {}", pitch_of(r[k]));
            assert!((look_hdg(r[k]) - h).abs() < 0.3, "frame {k}: heading {}", look_hdg(r[k]));
        }
        assert!(x.step(1.0 / 30.0).is_none(), "over at 3.5 s");
    }

    /// Exiting_Map_GPS_SharpAngle, 30 fps (sim_exit_low.py sharp; spline P1 (924.258, -, -619.142))
    #[test]
    fn exit_sharp_angle() {
        let mut x = NavExit::new(map_cam(823.773 + 100.0, 1500.0, -816.039 + 1200.0), chase_south());
        assert_eq!(x.kind(), ExitKind::SharpAngle);
        assert_eq!(x.total(), 2.0);
        let r = fly(&mut x, 59);
        for (k, want, p, h) in [
            (8, Vec3::new(929.866, 1338.637, -131.223), -61.58, -8.49),
            (19, Vec3::new(924.258, 759.760, -619.142), -73.29, -26.29),
            (35, Vec3::new(866.707, 127.520, -777.179), -64.52, -57.90),
            (52, Vec3::new(827.193, 20.350, -809.882), -12.85, -95.63),
            (58, Vec3::new(823.842, 19.520, -815.904), -4.18, -148.68),
        ] {
            assert!((r[k].eye - want).length() < 0.3, "frame {k}: {:?} vs {want:?}", r[k].eye);
            assert!((pitch_of(r[k]) - p).abs() < 0.3, "frame {k}: pitch {}", pitch_of(r[k]));
            assert!((look_hdg(r[k]) - h).abs() < 0.5, "frame {k}: heading {}", look_hdg(r[k]));
        }
        assert!(x.step(1.0 / 30.0).is_none(), "over at 2.0 s");
    }

    /// Exiting_Map_GPS_SouthSideCloseCar, 30 fps (sim_exit_low.py south)
    #[test]
    fn exit_south_side_close_car() {
        let e = chase_north();
        let mut x = NavExit::new(map_cam(823.773 + 100.0, 1500.0, -816.039 + 1200.0), e);
        assert_eq!(x.kind(), ExitKind::SouthSideCloseCar);
        // (1.05 + 0.5, the 1.05 snapped to 31/30 s)
        assert!((x.total() - 46.0 / 30.0).abs() < 1e-6);
        let r = fly(&mut x, 45);
        for (k, want, p) in [
            (9, Vec3::new(825.286, 818.243, 241.735), -76.87),
            (20, Vec3::new(702.865, 121.217, -293.675), -29.98),
            (29, Vec3::new(785.867, 59.383, -701.426), -19.39),
        ] {
            assert!((r[k].eye - want).length() < 0.3, "frame {k}: {:?} vs {want:?}", r[k].eye);
            assert!((pitch_of(r[k]) - p).abs() < 0.3, "frame {k}: pitch {}", pitch_of(r[k]));
        }
        assert!((r[44].eye - e.eye).length() < 1.0, "{:?}", r[44].eye);
        assert!(x.step(1.0 / 30.0).is_none(), "over at 1.533 s");
    }

    /// The map camera still moving when the map closes (rule_exitcam.md 10.8; numbers from
    /// work/mm/exitcam/sim_exit_vel.py, V = (300, 0, -400)): the eye differs from the at-rest flight by
    /// the carried part of each Translate_Axis profile, gone again at the end.
    #[test]
    fn exit_carries_the_map_camera_velocity() {
        let v = Vec3::new(300.0, 25.0, -400.0); // (y is dropped, as F-60031 does)
        let ss = map_cam(823.773 + 100.0, 1500.0, -816.039 + 1200.0);
        for (name, s, e, kind, n, want) in [
            ("high", map_cam(-327.0, 9000.0, 111.5), chase_south(), ExitKind::HighAltitude, 89,
             vec![(9, Vec3::ZERO), (14, Vec3::ZERO), (15, Vec3::new(5.3183, 0.0, -4.3992)), (19, Vec3::new(12.9812, 0.0, -10.7377)), (29, Vec3::new(11.2395, 0.0, -9.2970)), (44, Vec3::new(0.2622, 0.0, -0.2169))]),
            ("above", map_cam(823.773 + 1200.0, 1500.0, -816.039 - 200.0), chase_south(), ExitKind::AboveCar, 59,
             vec![(0, Vec3::new(25.4565, 0.0, -39.3224)), (4, Vec3::new(76.3950, 0.0, -108.8210)), (14, Vec3::new(97.8081, 0.0, -130.4108)), (19, Vec3::new(75.0, 0.0, -100.0)), (44, Vec3::new(1.7383, 0.0, -2.3177))]),
            ("low", map_cam(823.773 - 1500.0, 3000.0, -816.039 + 2000.0), chase_south(), ExitKind::LowAltitudeFarAway, 104,
             vec![(0, Vec3::new(9.9891, 0.0, -13.4779)), (9, Vec3::new(75.5422, 0.0, -101.9265)), (19, Vec3::new(97.8587, 0.0, -132.0373)), (44, Vec3::new(21.8370, 0.0, -29.4638))]),
            ("south", ss, chase_north(), ExitKind::SouthSideCloseCar, 45,
             vec![(0, Vec3::new(26.2149, 0.0, -40.8342)), (4, Vec3::new(95.0811, 0.0, -146.1247)), (9, Vec3::new(102.7478, 0.0, -151.4257)), (29, Vec3::new(3.6058, 0.0, -4.8077)), (44, Vec3::ZERO)]),
            ("sharp", ss, chase_south(), ExitKind::SharpAngle, 59, vec![(4, Vec3::ZERO), (29, Vec3::ZERO)]),
        ] {
            let (mut a, mut b) = (NavExit::new(s, e), NavExit::moving(s, e, v));
            assert_eq!(b.kind(), kind, "{name}");
            let (ra, rb) = (fly(&mut a, n), fly(&mut b, n));
            for (k, off) in want {
                let d = rb[k].eye - ra[k].eye;
                assert!((d - off).length() < 2e-2, "{name} frame {k}: {d:?} vs {off:?}");
            }
            // (it lands on the chase camera all the same)
            assert!((rb[n - 1].eye - ra[n - 1].eye).length() < 0.05, "{name}: {:?} vs {:?}", rb[n - 1].eye, ra[n - 1].eye);
            assert!(a.step(1.0 / 30.0).is_none() && b.step(1.0 / 30.0).is_none(), "{name}");
        }
        // the projections: along_final_look of HighAltitude is 3D (mostly down: 81.37 of 500),
        // final_look = n(flat E.r2).V
        let b = NavExit::moving(map_cam(-327.0, 9000.0, 111.5), chase_south(), v);
        assert!((b.v0_along - 81.3701).abs() < 0.01, "{}", b.v0_along);
        let b = NavExit::moving(ss, chase_north(), v);
        assert!((b.v0_final + 494.499).abs() < 0.01, "{}", b.v0_final);
    }

    /// The profile's carried part against the literal step (sim_exit_vel.py `ta_literal`):
    /// d = 0, v0 = 100, T = 2 -> p = (a1 tau/2 + v0) tau with a1 = -150, then 25 + (25 tau' - 50) tau'.
    #[test]
    fn translate_axis_carried_speed() {
        for (tau, want) in [(0.0, 0.0), (0.2, 17.0), (0.5, 31.25), (1.0, 25.0), (1.4, 9.0), (2.0, 0.0), (2.5, 0.0)] {
            assert!((ta_carry(100.0, 2.0, tau) - want).abs() < 1e-4, "tau {tau}: {}", ta_carry(100.0, 2.0, tau));
        }
        // slope v0 at the start
        assert!((ta_carry(100.0, 2.0, 1e-3) / 1e-3 - 100.0).abs() < 0.2);
    }

    /// NavCam's eye velocity: the eye's move over the last 30 Hz frame / (1/30 s), y dropped.
    #[test]
    fn map_camera_velocity_per_30hz_frame() {
        let mut n = NavCam::default();
        n.open(Vec3::new(0.0, 20.0, 0.0), Vec3::new(0.0, 19.0, -10.0), Vec3::ZERO, 16.0 / 9.0);
        for _ in 0..120 {
            n.update(1.0 / 60.0, false, false, Vec2::ZERO, 16.0 / 9.0);
        }
        assert_eq!(n.eye_velocity(), Vec3::ZERO);
        n.fly_to(Vec3::new(1500.0, 0.0, 800.0));
        let mut eyes = vec![n.view().0];
        for _ in 0..21 {
            n.update(1.0 / 60.0, false, false, Vec2::ZERO, 16.0 / 9.0);
            eyes.push(n.view().0);
        }
        // (21 frames at 60 Hz: the last 30 Hz frame is frames 18..20 or 19..21)
        let v = n.eye_velocity();
        let ok = |i: usize| {
            let d = (eyes[i] - eyes[i - 2]) * 30.0;
            (v - Vec3::new(d.x, 0.0, d.z)).length() < 1e-2 * d.length().max(1.0)
        };
        assert!(ok(20) || ok(21), "{v:?} vs {:?} / {:?}", (eyes[20] - eyes[18]) * 30.0, (eyes[21] - eyes[19]) * 30.0);
        assert!(v.length() > 100.0);
        // a zoom step per 30 Hz frame: 30 x its move
        let mut n = NavCam::default();
        n.open(Vec3::new(0.0, 20.0, 0.0), Vec3::new(0.0, 19.0, -10.0), Vec3::ZERO, 16.0 / 9.0);
        for _ in 0..120 {
            n.update(1.0 / 60.0, false, false, Vec2::ZERO, 16.0 / 9.0);
        }
        n.cursor = Vec3::new(400.0, 0.0, -300.0);
        let e0 = n.view().0;
        n.update(1.0 / 60.0, true, false, Vec2::ZERO, 16.0 / 9.0);
        n.update(1.0 / 60.0, true, false, Vec2::ZERO, 16.0 / 9.0);
        let d = (n.view().0 - e0) * 30.0;
        assert!(d.length() > 1.0 && (n.eye_velocity() - Vec3::new(d.x, 0.0, d.z)).length() < 1e-2 * d.length(), "{:?} vs {d:?}", n.eye_velocity());
    }

    /// A closing flight recorded from the original (work/mm/exitcam/rec_exit.py + fixture.py): start S
    /// (transition +112), end E (+176), the render camera's velocity V when it began, and each rendered
    /// frame (transition elapsed, render camera rows, NavMap lens NM+712).
    struct ExitRec {
        s: CamRows,
        e: CamRows,
        v: Vec3,
        frames: Vec<(f32, CamRows, f32)>,
    }

    fn rows12(v: &[f32]) -> CamRows {
        CamRows { r0: Vec3::new(v[0], v[1], v[2]), r1: Vec3::new(v[3], v[4], v[5]), r2: Vec3::new(v[6], v[7], v[8]), eye: Vec3::new(v[9], v[10], v[11]) }
    }

    fn exit_rec(text: &str) -> ExitRec {
        let mut r = ExitRec { s: map_cam(0.0, 0.0, 0.0), e: map_cam(0.0, 0.0, 0.0), v: Vec3::ZERO, frames: Vec::new() };
        for l in text.lines() {
            let mut it = l.split_whitespace();
            let tag = it.next().unwrap_or("");
            let n: Vec<f32> = it.map(|x| x.parse().unwrap()).collect();
            match tag {
                "S" => r.s = rows12(&n),
                "E" => r.e = rows12(&n),
                "V" => r.v = Vec3::new(n[0], n[1], n[2]),
                "F" => r.frames.push((n[0], rows12(&n[1..13]), n[13])),
                _ => {}
            }
        }
        r
    }

    const EXIT_RECS: [(&str, &str); 10] = [
        ("high", include_str!("../testdata/exitcam/fx_high.txt")),
        ("high2", include_str!("../testdata/exitcam/fx_high2.txt")),
        ("lens", include_str!("../testdata/exitcam/fx_lens.txt")),
        ("above", include_str!("../testdata/exitcam/fx_above.txt")),
        ("above2", include_str!("../testdata/exitcam/fx_abovemove.txt")),
        ("south", include_str!("../testdata/exitcam/fx_south.txt")),
        ("south_moving", include_str!("../testdata/exitcam/fx_move.txt")),
        ("low", include_str!("../testdata/exitcam/fx_low.txt")),
        ("low_moving", include_str!("../testdata/exitcam/fx_lowmove.txt")),
        ("sharp", include_str!("../testdata/exitcam/fx_sharp.txt")),
    ];

    fn ang_deg(a: Vec3, b: Vec3) -> f32 {
        a.normalize().dot(b.normalize()).clamp(-1.0, 1.0).acos().to_degrees()
    }

    /// The port's flight from the recorded S, E, V at 30 fps against the recorded frames: the largest
    /// eye distance (m), view direction and up vector angles (degrees), and the kind and frame count.
    fn exit_rec_diff(name: &str, text: &str, print: bool) -> (ExitKind, usize, f32, f32, f32) {
        let r = exit_rec(text);
        let mut x = NavExit::moving(r.s, r.e, r.v);
        let (mut de, mut dl, mut du) = (0.0f32, 0.0f32, 0.0f32);
        for (k, (t, f, _lens)) in r.frames.iter().enumerate() {
            let c = x.step(1.0 / 30.0).unwrap_or(r.e);
            let (a, b, u) = ((c.eye - f.eye).length(), ang_deg(c.r2, f.r2), ang_deg(c.r1, f.r1));
            let roll_d = c.r1.cross(f.r1).dot(c.r2).atan2(c.r1.dot(f.r1)).to_degrees();
            let mut kk = 2.0 * (1.0 - c.r2.y);
            if !(0.0..=1.0).contains(&kk) {
                kk = 1.0;
            }
            let roll_p = (kk * x.lam.clamp(-PI / 4.0, PI / 4.0)).to_degrees();
            if print {
                println!("{name} {k:3} roll port {roll_p:8.3} rec {:8.3} k {kk:.3}", roll_p + roll_d);
                println!("{name} {k:3} t {t:.3} rec ({:9.3} {:9.3} {:9.3}) p {:7.2} h {:8.2} | port ({:9.3} {:9.3} {:9.3}) p {:7.2} h {:8.2} | d {a:7.3} m {b:6.3} {u:6.3} deg",
                    f.eye.x, f.eye.y, f.eye.z, pitch_of(*f), look_hdg(*f), c.eye.x, c.eye.y, c.eye.z, pitch_of(c), look_hdg(c));
            }
            de = de.max(a);
            dl = dl.max(b);
            du = du.max(u);
        }
        (x.kind(), r.frames.len(), de, dl, du)
    }

    /// Every closing flight recorded from the original (rule_exitcam.md 10.9; started from rest and
    /// from a moving map camera, all five kinds): the port's flight from the same start, end and
    /// velocity lands on each recorded frame (eye within 1 cm, view and up within 0.05 degrees - the
    /// rest is float noise) and ends on the same frame.
    #[test]
    fn exit_recordings_match_the_original() {
        let kinds = [ExitKind::HighAltitude, ExitKind::HighAltitude, ExitKind::HighAltitude, ExitKind::AboveCar, ExitKind::AboveCar,
            ExitKind::SouthSideCloseCar, ExitKind::SouthSideCloseCar, ExitKind::LowAltitudeFarAway, ExitKind::LowAltitudeFarAway, ExitKind::SharpAngle];
        for ((name, text), kind) in EXIT_RECS.iter().zip(kinds) {
            let (k, n, de, dl, du) = exit_rec_diff(name, text, false);
            assert_eq!(k, kind, "{name}");
            assert!(de < 0.01 && dl < 0.05 && du < 0.05, "{name}: eye {de} m, look {dl} deg, up {du} deg");
            // (the flight is over on the frame after the last recorded one)
            let r = exit_rec(text);
            let mut x = NavExit::moving(r.s, r.e, r.v);
            for _ in 0..n {
                assert!(x.step(1.0 / 30.0).is_some(), "{name}");
            }
            assert!(x.step(1.0 / 30.0).is_none(), "{name}");
        }
        // (two of them start from a moving map camera)
        assert!(exit_rec(EXIT_RECS[6].1).v.length() > 1000.0 && exit_rec(EXIT_RECS[8].1).v.length() > 1000.0);
    }

    /// Prints every recorded closing flight next to the port's (cargo test exit_recordings_print -- --ignored --nocapture).
    #[test]
    #[ignore]
    fn exit_recordings_print() {
        for (name, text) in EXIT_RECS {
            let (kind, n, de, dl, du) = exit_rec_diff(name, text, true);
            println!("== {name}: {kind:?}, {n} frames, max eye {de:.3} m, look {dl:.3} deg, up {du:.3} deg");
        }
    }

    #[test]
    fn heading_follows_row_two() {
        // the recorded player: row 2 (0.475, 0, -0.880) -> 151.6 degrees
        let h = heading(Vec3::new(0.475, 0.0, -0.880));
        assert!((h.to_degrees() - 151.6).abs() < 0.1);
        let m = rot_y(h, 1.0, Vec3::ZERO);
        let z = m.z_axis.truncate();
        assert!((z - Vec3::new(0.475, 0.0, -0.880).normalize()).length() < 1e-3);
    }

    /// st1 (work/career2/mk/rec/st1.txt): the map camera S and the start scene's camera at t 1.0 E
    fn cine_st1() -> (CamRows, CamRows) {
        let se = Vec3::new(819.61, 850.0, 491.59);
        let ee = Vec3::new(-624.57, 28.22, -649.17);
        (CamRows::look_at(se, se + Vec3::new(0.0, -0.539, -0.842), Vec3::Y), CamRows::look_at(ee, ee + Vec3::new(-0.722, -0.482, 0.497), Vec3::Y))
    }

    #[test]
    fn cine_warp_choice_st1() {
        let (s, e) = cine_st1();
        assert_eq!(CineKind::choose(&s, &e), CineKind::SouthSideCloseCar);
        let w = CineWarp::new(s, e, Vec3::ZERO, 40.0, 51.0);
        assert!((w.total() - 91.0 / 30.0).abs() < 1e-6, "{}", w.total());
        // (the other branches)
        let ab = CamRows::look_at(Vec3::new(0.0, 850.0, 500.0), Vec3::new(0.0, 0.0, 400.0), Vec3::Y);
        let e0 = CamRows::look_at(Vec3::ZERO, Vec3::new(0.0, 0.0, -10.0), Vec3::Y);
        assert_eq!(CineKind::choose(&ab, &e0), CineKind::AboveCar);
        let hi = CamRows::look_at(Vec3::new(0.0, 7000.0, 500.0), Vec3::new(0.0, 0.0, 400.0), Vec3::Y);
        assert_eq!(CineKind::choose(&hi, &e0), CineKind::HighAltitude);
        let far = CamRows::look_at(Vec3::new(0.0, 850.0, 2500.0), Vec3::new(0.0, 0.0, 2400.0), Vec3::Y);
        assert_eq!(CineKind::choose(&far, &e0), CineKind::LowAltitudeFarAway);
        let near = CamRows::look_at(Vec3::new(30.0, 50.0, 30.0), Vec3::ZERO, Vec3::Y);
        assert_eq!(CineKind::choose(&near, &e0), CineKind::CloseToTarget);
        // (E north of S and looking south-ish: |f30| > 2.5)
        let south = CamRows::look_at(Vec3::new(0.0, 850.0, 1300.0), Vec3::new(0.0, 0.0, 1200.0), Vec3::Y);
        let es = CamRows::look_at(Vec3::ZERO, Vec3::new(1.0, -0.5, 10.0), Vec3::Y);
        assert_eq!(CineKind::choose(&south, &es), CineKind::SharpAngle);
        // (E south of S: the extra E.z > S.z test)
        let en = CamRows::look_at(Vec3::new(0.0, 0.0, 1500.0), Vec3::new(1.0, -0.5, 1490.0), Vec3::Y);
        let sn = CamRows::look_at(Vec3::new(0.0, 850.0, 0.0), Vec3::new(0.0, 0.0, 100.0), Vec3::Y);
        assert_eq!(CineKind::choose(&sn, &en), CineKind::AboveCar);
    }

    /// The eye path of st1 against the original's recorded render camera (flight start 4.68 s, samples
    /// 4.721..7.749): nearest distance to the port's path (sim_warp.py's path mode). One sample (6.993:
    /// car + (0, 2, 0) looking +z for one frame, not part of the flight) is off.
    #[test]
    fn cine_warp_eye_path_st1() {
        let rec = [
            (817.45, 847.47, 487.88), (814.75, 844.31, 483.25), (806.12, 834.21, 468.45), (800.20, 827.29, 458.30),
            (785.20, 809.73, 432.57), (776.13, 799.12, 417.01), (754.92, 774.30, 380.63), (729.69, 744.77, 337.35),
            (715.60, 728.29, 313.19), (684.57, 691.98, 259.96), (667.67, 672.19, 230.96), (631.17, 629.49, 168.37),
            (611.46, 606.62, 134.96), (566.24, 557.98, 66.18), (540.70, 532.26, 30.94), (484.05, 478.37, -41.10),
            (453.04, 451.94, -77.84), (385.93, 402.81, -152.62), (349.96, 380.05, -190.59), (274.90, 338.08, -266.45),
            (274.90, 338.08, -266.45), (203.31, 300.66, -336.31), (169.24, 283.61, -368.70), (104.50, 252.67, -428.57),
            (73.79, 238.74, -456.11), (15.60, 213.81, -506.63), (-11.91, 202.76, -529.67), (-63.92, 183.35, -571.51),
            (-89.30, 174.92, -589.80), (-114.79, 167.31, -606.05), (-165.98, 154.40, -632.74), (-191.62, 149.05, -643.34),
            (-217.25, 144.39, -652.22), (-268.32, 137.02, -665.16), (-293.70, 134.24, -669.37), (-318.94, 132.03, -672.18),
            (-344.01, 130.36, -673.69), (-368.87, 129.19, -673.97), (-417.84, 128.23, -671.15), (-441.24, 127.76, -668.67),
            (-482.30, 123.47, -664.30), (-500.15, 119.82, -662.40), (-530.98, 109.96, -659.12), (-555.93, 97.27, -656.47),
            (-566.44, 90.07, -655.35), (-638.91, 20.21, -636.81), (-591.12, 67.36, -652.73), (-602.71, 55.54, -651.49),
            (-607.28, 50.74, -651.01), (-614.37, 43.06, -650.25), (-617.01, 40.04, -649.97), (-620.83, 35.41, -649.57),
            (-622.13, 33.69, -649.43), (-623.77, 31.18, -649.25), (-624.21, 30.31, -649.21), (-624.57, 29.16, -649.17),
            (-624.57, 28.81, -649.17), (-624.57, 28.41, -649.17), (-624.57, 28.31, -649.17), (-624.57, 28.23, -649.17),
            (-624.57, 28.22, -649.17),
        ];
        let (s, e) = cine_st1();
        let mut w = CineWarp::new(s, e, Vec3::ZERO, 40.0, 51.0);
        let mut path = vec![s.eye];
        while let Some((c, _)) = w.step(1.0 / 3000.0) {
            path.push(c.eye);
        }
        path.push(e.eye);
        let seg_d = |p: Vec3, a: Vec3, b: Vec3| {
            let ab = b - a;
            let u = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            (a + ab * u - p).length()
        };
        let mut off = 0;
        let mut worst = 0.0f32;
        for (x, y, z) in rec {
            let p = Vec3::new(x, y, z);
            let d = path.windows(2).map(|q| seg_d(p, q[0], q[1])).fold(f32::MAX, f32::min);
            if d > 0.1 {
                off += 1;
            } else {
                worst = worst.max(d);
            }
        }
        assert!(off <= 1, "{off} samples off the path");
        assert!(worst < 0.1, "{worst}");
    }

    /// Every kind lands exactly on E (eye at the Lean's last clock; rows and lens on the last frame),
    /// at rest and with a scene camera velocity handed in.
    #[test]
    fn cine_warp_every_kind_ends_on_e() {
        let (st, est) = cine_st1();
        let e0 = CamRows::look_at(Vec3::ZERO, Vec3::new(0.0, 0.0, -10.0), Vec3::Y);
        let es = CamRows::look_at(Vec3::ZERO, Vec3::new(1.0, -0.5, 10.0), Vec3::Y);
        let cases = [
            (CamRows::look_at(Vec3::new(0.0, 7000.0, 500.0), Vec3::new(0.0, 0.0, 400.0), Vec3::Y), e0, CineKind::HighAltitude),
            (CamRows::look_at(Vec3::new(0.0, 850.0, 2500.0), Vec3::new(0.0, 0.0, 2400.0), Vec3::Y), e0, CineKind::LowAltitudeFarAway),
            (CamRows::look_at(Vec3::new(30.0, 50.0, 30.0), Vec3::ZERO, Vec3::Y), e0, CineKind::CloseToTarget),
            (CamRows::look_at(Vec3::new(0.0, 850.0, 500.0), Vec3::new(0.0, 0.0, 400.0), Vec3::Y), e0, CineKind::AboveCar),
            (CamRows::look_at(Vec3::new(0.0, 850.0, 1300.0), Vec3::new(0.0, 0.0, 1200.0), Vec3::Y), es, CineKind::SharpAngle),
            (st, est, CineKind::SouthSideCloseCar),
        ];
        for (s, e, kind) in cases {
            for vel in [Vec3::ZERO, Vec3::new(3.0, 0.5, -2.0)] {
                let mut w = CineWarp::new(s, e, vel, 40.0, 51.0);
                assert_eq!(w.kind(), kind);
                let tt = w.total();
                let end = w.eye_at(kind.clock(tt), tt, tt);
                assert!((end - e.eye).length() < 1e-3, "{kind:?} {vel}: {end} vs {}", e.eye);
                let mut last = None;
                let mut n = 0;
                while let Some(o) = w.step(1.0 / 600.0) {
                    last = Some(o);
                    n += 1;
                }
                assert!((n as f32 / 600.0 - tt).abs() < 2.0 / 600.0, "{kind:?}: {n} frames for {tt}");
                let (c, fov) = last.unwrap();
                assert!((c.eye - e.eye).length() < 0.05, "{kind:?} {vel}: {} vs {}", c.eye, e.eye);
                for (a, b) in [(c.r0, e.r0), (c.r1, e.r1), (c.r2, e.r2)] {
                    assert!(a.dot(b) > 0.9999, "{kind:?} {vel}: rows {a} vs {b}");
                }
                assert!((fov - 51.0).abs() < 1e-4, "{kind:?}: fov {fov}");
            }
        }
    }

    /// DEFAULT_WITH_CUT's climb: the chooser's f1 against south, the three lengths, and where it
    /// ends - 850 m world height, 1300 m south of the car (+z), looking at the point 20 m (FocusOnCar)
    /// / 10 m (Sharp_Angle) ahead of it; the lens at 25 after 0.8 T.
    #[test]
    fn default_with_cut_climb() {
        // the car at (100, 5, -200) facing -z (back +z), the camera behind it looking at it
        let car = CamRows { r0: Vec3::X, r1: Vec3::Y, r2: Vec3::Z, eye: Vec3::new(100.0, 5.0, -200.0) };
        let north = CamRows::look_at(Vec3::new(100.0, 7.0, -192.0), Vec3::new(100.0, 5.0, -210.0), Vec3::Y);
        let south = CamRows::look_at(Vec3::new(100.0, 7.0, -208.0), Vec3::new(100.0, 5.0, -190.0), Vec3::Y);
        let sw = CamRows::look_at(Vec3::new(100.0, 7.0, -208.0), Vec3::new(90.0, 5.0, -190.0), Vec3::Y);
        let se = CamRows::look_at(Vec3::new(100.0, 7.0, -208.0), Vec3::new(110.0, 5.0, -190.0), Vec3::Y);
        assert_eq!(ClimbKind::choose(&north), ClimbKind::FocusOnCar);
        // (exactly south: f1 = 0, neither 0 < f1 nor f1 < 0)
        assert_eq!(ClimbKind::choose(&south), ClimbKind::FocusOnCar);
        assert_eq!(ClimbKind::choose(&sw), ClimbKind::SharpAngleRight);
        assert_eq!(ClimbKind::choose(&se), ClimbKind::SharpAngleLeft);
        assert!((ClimbKind::FocusOnCar.total() - 1.3).abs() < 1e-5);
        assert!((ClimbKind::SharpAngleRight.total() - 61.0 / 30.0).abs() < 1e-4);
        for (s, kind, ahead) in [(north, ClimbKind::FocusOnCar, 20.0), (sw, ClimbKind::SharpAngleRight, 10.0), (se, ClimbKind::SharpAngleLeft, 10.0)] {
            let mut c = CineClimb::new(s, car, 50.0, 25.0, Vec3::ZERO);
            assert_eq!(c.kind(), kind);
            let mut n = 0;
            let mut last = None;
            while let Some(o) = c.step(1.0 / 30.0) {
                last = Some(o);
                n += 1;
            }
            assert_eq!(n as f32, (kind.total() * 30.0).round() - 1.0, "{kind:?}");
            let (end, fov) = c.end();
            assert!((end.eye - Vec3::new(100.0, 850.0, 1100.0)).length() < 1e-3, "{kind:?}: {}", end.eye);
            assert_eq!(fov, 25.0);
            let l = Vec3::new(100.0, 5.0, -200.0 - ahead);
            assert!((-end.r2).dot((l - end.eye).normalize()) > 0.9999, "{kind:?}: looks {}", -end.r2);
            assert!(end.r0.y.abs() < 1e-4, "{kind:?}: no roll");
            let (o, f) = last.unwrap();
            assert!((o.eye - end.eye).length() < 5.0, "{kind:?}: last frame {}", o.eye);
            // (the lens's last write falls short of its end by under a frame)
            assert!((f - 25.0).abs() < 0.05, "{kind:?}: lens {f}");
        }
        // FocusOnCar halfway (t 0.65, C = 1.3 (1 - cos(pi/4)) = 0.3808): y = 7 + 843 s(0.3808 / 1.3)
        let mut c = CineClimb::new(north, car, 50.0, 25.0, Vec3::ZERO);
        let mut o = None;
        for _ in 0..39 {
            o = c.step(1.0 / 60.0).or(o);
        }
        let y = 7.0 + 843.0 * s_curve(1.3 * (1.0 - (PI * 0.25).cos()) / 1.3);
        assert!((o.unwrap().0.eye.y - y).abs() < 0.01, "{} vs {y}", o.unwrap().0.eye.y);
        // the render camera's velocity (y dropped) bends the path, never the end: FocusOnCar's X / Z carry
        // V over 37/30 - v0 T g(u) - and the Sharp_Angle ones' X, Z, final_look / final_side first stages
        for s0 in [north, sw, se] {
            let v = Vec3::new(30.0, 50.0, -20.0);
            let (mut a, mut b) = (CineClimb::new(s0, car, 50.0, 25.0, Vec3::ZERO), CineClimb::new(s0, car, 50.0, 25.0, v));
            let mut most = 0f32;
            while let (Some(x), Some(y)) = (a.step(1.0 / 30.0), b.step(1.0 / 30.0)) {
                assert!((x.0.eye.y - y.0.eye.y).abs() < 1e-3, "no carry on Y");
                most = most.max((x.0.eye - y.0.eye).length());
            }
            assert!(most > 1.0, "{:?}: carried {most}", a.kind());
            assert!((a.end().0.eye - b.end().0.eye).length() < 1e-3);
        }
        let t = ui_time(1.25);
        assert!((ta_carry(30.0, t, 1e-3) / 1e-3 - 30.0).abs() < 0.1);
    }

    #[test]
    fn difficulty_words_truncate() {
        let g = difficulty_word(0);
        assert!((g[1] - 249.0 / 255.0).abs() < 1e-6);
        let o = difficulty_word(2);
        assert!((o[0] - 251.0 / 255.0).abs() < 1e-6 && (o[1] - 156.0 / 255.0).abs() < 1e-6);
    }
}
