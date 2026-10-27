//! Vinyls as the original builds them (mcCarVinylShapeLibrary / VinylGenerator, read from
//! the executable and checked against the finished opponent liveries on the disc).
//!
//! A livery is five zones of layers (top, left, right, front, rear). Every layer places
//! one shape bitmap (`resources/vinyls/bitmap`, named in `tune/vehicle/vinyl_shapes.list`)
//! with a size, position, rotation, skew and colour, in units of the car's paint unwrap.
//! The car's main resource says where each zone's origin lies in that unwrap and holds a
//! mask texture that tells which zone a texel belongs to.

use glam::Vec2;
use hashbrown::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const ZONE_NAMES: [&str; 5] = ["TOP", "LEFT", "RIGHT", "FRONT", "REAR"];
/// layers a zone can hold
pub const ZONE_CAPACITY: [usize; 5] = [64, 64, 64, 16, 16];
/// the zones' frames in the unwrap: texture direction of the zone's x and y axes
const FRAMES: [[f32; 4]; 5] = [[0.0, 1.0, 1.0, 0.0], [0.0, 1.0, 1.0, 0.0], [0.0, -1.0, -1.0, 0.0], [1.0, 0.0, 0.0, -1.0], [1.0, 0.0, 0.0, -1.0]];
/// rotation (degrees) at which a shape lies along the zone's x axis
const BASE_ROT: [f32; 5] = [270.0, 270.0, -270.0, 0.0, 0.0];

/// Rotation at which a layer of `zone` lies along the car.
pub fn base_rot(zone: usize) -> f32 {
    BASE_ROT[zone.min(4)]
}

/// How a layer is put on the ones below it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Blend {
    #[default]
    Normal,
    /// keeps what is below only inside the shape
    AlphaCutout,
    /// cuts the shape out of what is below
    NegativeCutout,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Layer {
    /// category * 1000 + index into the shape list
    pub shape: u16,
    /// half extents (negative = mirrored)
    pub scale: [f32; 2],
    pub pos: [f32; 2],
    /// degrees
    pub rot: f32,
    pub skew: f32,
    /// r, g, b, a
    pub color: [u8; 4],
    #[serde(default)]
    pub blend: Blend,
}

impl Default for Layer {
    fn default() -> Self {
        Layer { shape: 0, scale: [0.12, 0.12], pos: [0.0, 0.0], rot: 0.0, skew: 0.0, color: [255; 4], blend: Blend::Normal }
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Livery {
    pub zones: [Vec<Layer>; 5],
}

/// The record's half float as the original unpacks it (F-38353): 0 is 0.0, anything else is the bit pattern
/// sign | (exponent + 112) << 23 | mantissa << 13 - so the original's "denormals" are normal numbers, 0x8000 is
/// -2^-15 and the exponent 31 is no infinity (all 65536 values compared, carparts_small_tests.rs).
pub(crate) fn half(b: &[u8]) -> f32 {
    let h = u32::from(u16::from_be_bytes([b[0], b[1]]));
    if h == 0 {
        return 0.0;
    }
    f32::from_bits((h & 0x8000) << 16 | (((h >> 10) & 0x1f) + 112) << 23 | (h & 0x3ff) << 13)
}

impl Livery {
    pub fn is_empty(&self) -> bool {
        self.zones.iter().all(|z| z.is_empty())
    }

    pub fn layer_count(&self) -> usize {
        self.zones.iter().map(|z| z.len()).sum()
    }

    /// The layers of a car set-up (`resources/vehicle/configs/*.xcc`): 20-byte records
    /// {half scale x, y; half position x, y; ARGB; half skew; half rotation; u16 shape;
    /// u8 blend; u8 hidden}, the zones one after the other from 0x904.
    pub fn from_xcc(d: &[u8]) -> Livery {
        let mut out = Livery::default();
        let mut o = 0x904;
        for (z, cap) in ZONE_CAPACITY.iter().enumerate() {
            for _ in 0..*cap {
                if o + 20 > d.len() {
                    return out;
                }
                let r = &d[o..o + 20];
                o += 20;
                let shape = u16::from_be_bytes([r[16], r[17]]);
                if shape == 0xffff || r[19] != 0 {
                    continue;
                }
                out.zones[z].push(Layer {
                    shape,
                    scale: [half(&r[0..]), half(&r[2..])],
                    pos: [half(&r[4..]), half(&r[6..])],
                    color: [r[9], r[10], r[11], r[8]],
                    skew: half(&r[12..]),
                    rot: half(&r[14..]),
                    blend: match r[18] {
                        1 => Blend::AlphaCutout,
                        2 => Blend::NegativeCutout,
                        _ => Blend::Normal,
                    },
                });
            }
        }
        out
    }

    /// One zone of a preset (`tune/garage/<preset>_<zone>.vgp`: "layer n", "shape id",
    /// "rotate", "scale x y", "position x y", "color r g b a", "negalpha", "skew").
    pub fn zone_from_vgp(text: &str) -> Vec<Layer> {
        let mut out: Vec<Layer> = Vec::new();
        let mut cur: Option<(Layer, i32)> = None;
        for line in text.lines() {
            let mut t = line.split_whitespace();
            let Some(key) = t.next() else { continue };
            let v: Vec<f32> = t.filter_map(|x| x.parse().ok()).collect();
            if key == "layer" {
                if let Some((mut l, shape)) = cur.take() {
                    if shape >= 0 {
                        l.shape = shape as u16;
                        out.push(l);
                    }
                }
                cur = Some((Layer::default(), -1));
                continue;
            }
            let Some((l, shape)) = cur.as_mut() else { continue };
            match (key, v.len()) {
                ("shape", 1..) => *shape = v[0] as i32,
                ("rotate", 1..) => l.rot = v[0],
                ("scale", 2..) => l.scale = [v[0], v[1]],
                ("position", 2..) => l.pos = [v[0], v[1]],
                ("color", 4..) => l.color = [(v[0] * 255.0) as u8, (v[1] * 255.0) as u8, (v[2] * 255.0) as u8, (v[3] * 255.0) as u8],
                ("negalpha", 1..) => {
                    l.blend = match v[0] as i32 {
                        1 => Blend::NegativeCutout,
                        2 => Blend::AlphaCutout,
                        _ => Blend::Normal,
                    }
                }
                ("skew", 1..) => l.skew = v[0],
                _ => {}
            }
        }
        if let Some((mut l, shape)) = cur.take() {
            if shape >= 0 {
                l.shape = shape as u16;
                out.push(l);
            }
        }
        out
    }
}

/// Where the zones of a car lie in its paint unwrap.
pub struct CarZones {
    pub centres: [Vec2; 5],
    /// zone of each texel of the mask (5 = none)
    mask: Vec<u8>,
    mw: usize,
    mh: usize,
}

impl CarZones {
    /// From the car's main resource (the larger `.r83`): the zone origins at root+0x20
    /// and the `<car>_zone` texture at root+0x14, whose grey level is (zone + 1) / 8.
    pub fn load(car_dir: &Path) -> Option<CarZones> {
        let main = std::fs::read_dir(car_dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |e| e == "r83")).max_by_key(|p| std::fs::metadata(p).map_or(0, |m| m.len()))?;
        let res = crate::assets::load_resource_file(&main).ok()?;
        let rd = mcla_formats::rage::Rd::new(&res);
        if !rd.has(0, 0x48) {
            return None;
        }
        let mut centres = [Vec2::splat(0.5); 5];
        for (i, c) in centres.iter_mut().enumerate() {
            *c = Vec2::new(rd.f32(0x20 + i * 8), rd.f32(0x24 + i * 8));
            if !(c.x > 0.0 && c.x < 1.0 && c.y > 0.0 && c.y < 1.0) {
                return None;
            }
        }
        let t = rd.ptr(0x14).and_then(|o| mcla_formats::rage::texture::Texture::parse(&rd, o).ok())?;
        let px = t.to_rgba8();
        let mask = px.chunks_exact(4).map(|p| ((p[1] as f32 / 32.0).round() as i32 - 1).clamp(-1, 5)).map(|z| if z < 0 { 5 } else { z as u8 }).collect();
        Some(CarZones { centres, mask, mw: t.width as usize, mh: t.height as usize })
    }

    fn zone_at(&self, u: f32, v: f32) -> usize {
        let x = ((u * self.mw as f32) as usize).min(self.mw - 1);
        let y = ((v * self.mh as f32) as usize).min(self.mh - 1);
        self.mask[y * self.mw + x] as usize
    }
}

pub struct Shape {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

impl Shape {
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = (u * self.w as f32 - 0.5).clamp(0.0, self.w as f32 - 1.0);
        let y = (v * self.h as f32 - 0.5).clamp(0.0, self.h as f32 - 1.0);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let p = |x: usize, y: usize, c: usize| self.rgba[(y * self.w + x) * 4 + c] as f32;
        let mut out = [0.0; 4];
        for (c, o) in out.iter_mut().enumerate() {
            *o = ((p(x0, y0, c) * (1.0 - fx) + p(x1, y0, c) * fx) * (1.0 - fy) + (p(x0, y1, c) * (1.0 - fx) + p(x1, y1, c) * fx) * fy) / 255.0;
        }
        out
    }
}

fn texture_of(res: &mcla_formats::rsc::Resource) -> Option<mcla_formats::rage::texture::Texture> {
    let rd = mcla_formats::rage::Rd::new(res);
    [rd.ptr(8), Some(0), rd.ptr(0), rd.ptr(4)].into_iter().flatten().find_map(|o| mcla_formats::rage::texture::Texture::parse(&rd, o).ok())
}

/// The shape bitmaps: categories of `vinyl_shapes.list`, the files found by the name
/// stored inside each texture.
#[derive(Default)]
pub struct ShapeLib {
    pub cats: Vec<(String, Vec<String>)>,
    files: HashMap<String, PathBuf>,
    cache: HashMap<u16, Option<Arc<Shape>>>,
}

impl ShapeLib {
    pub fn load(data_dir: &str) -> ShapeLib {
        let root = Path::new(data_dir);
        let mut lib = ShapeLib::default();
        if let Ok(t) = std::fs::read_to_string(root.join("tune/vehicle/vinyl_shapes.list")) {
            for l in t.lines().map(|l| l.trim()) {
                if l.starts_with("vnyl") {
                    if let Some(c) = lib.cats.last_mut() {
                        c.1.push(l.to_ascii_lowercase());
                    }
                } else if !l.is_empty() && l.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
                    lib.cats.push((l.to_string(), Vec::new()));
                }
            }
        }
        // the bitmap files are stored under hashes: index them by the texture's own name
        let index = root.join("vinyl_shapes.tsv");
        if let Ok(t) = std::fs::read_to_string(&index) {
            for l in t.lines() {
                if let Some((n, f)) = l.split_once("\t") {
                    lib.files.insert(n.to_string(), PathBuf::from(f));
                }
            }
        }
        if lib.files.is_empty() {
            let mut out = String::new();
            let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("resources/vinyls/bitmap")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            files.sort();
            for f in files {
                let Ok(res) = crate::assets::load_resource_file(&f) else { continue };
                let Some(t) = texture_of(&res) else { continue };
                let name = t.name.rsplit(['/', '\\']).next().unwrap_or("").split('.').next().unwrap_or("").to_ascii_lowercase();
                if !name.is_empty() {
                    out.push_str(&format!("{}\t{}\n", name, f.display()));
                    lib.files.insert(name, f);
                }
            }
            let _ = std::fs::write(&index, out);
        }
        log::info!("vinyls: {} shape categories, {} shapes, {} bitmaps", lib.cats.len(), lib.cats.iter().map(|c| c.1.len()).sum::<usize>(), lib.files.len());
        lib
    }

    /// The shape groups of the editor: (name, shapes).
    pub fn categories(data_dir: &str) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = Vec::new();
        if let Ok(t) = std::fs::read_to_string(Path::new(data_dir).join("tune/vehicle/vinyl_shapes.list")) {
            for l in t.lines().map(|l| l.trim()) {
                if l.starts_with("vnyl") {
                    if let Some(c) = out.last_mut() {
                        c.1 += 1;
                    }
                } else if !l.is_empty() && l.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
                    out.push((l.to_string(), 0));
                }
            }
        }
        out
    }

    pub fn name(&self, id: u16) -> Option<&str> {
        self.cats.get(id as usize / 1000).and_then(|c| c.1.get(id as usize % 1000)).map(|s| s.as_str())
    }

    /// The bitmap of shape `id`, at most 256 texels a side.
    pub fn shape(&mut self, id: u16) -> Option<Arc<Shape>> {
        if let Some(s) = self.cache.get(&id) {
            return s.clone();
        }
        let s = self.name(id).and_then(|n| self.files.get(n)).and_then(|f| crate::assets::load_resource_file(f).ok()).and_then(|res| {
            let t = texture_of(&res)?;
            let (w, h) = (t.width as usize, t.height as usize);
            let px = t.to_rgba8();
            // box filter down
            let k = (w.max(h) / 256).max(1);
            let (nw, nh) = ((w / k).max(1), (h / k).max(1));
            let mut rgba = vec![0u8; nw * nh * 4];
            for y in 0..nh {
                for x in 0..nw {
                    let mut acc = [0u32; 4];
                    for yy in 0..k {
                        for xx in 0..k {
                            let i = ((y * k + yy) * w + x * k + xx) * 4;
                            for c in 0..4 {
                                acc[c] += px[i + c] as u32;
                            }
                        }
                    }
                    for c in 0..4 {
                        rgba[(y * nw + x) * 4 + c] = (acc[c] / (k * k) as u32) as u8;
                    }
                }
            }
            Some(Arc::new(Shape { w: nw, h: nh, rgba }))
        });
        self.cache.insert(id, s.clone());
        s
    }
}

struct Prepared {
    shape: Arc<Shape>,
    pos: [f32; 2],
    cos: f32,
    sin: f32,
    inv: [f32; 2],
    skew: f32,
    color: [f32; 4],
    blend: Blend,
}

/// Paint a livery into the car's unwrap: `size` x `size` RGBA, plain (not multiplied) colour.
pub fn compose(zones: &CarZones, livery: &Livery, lib: &mut ShapeLib, size: usize) -> Vec<u8> {
    let prepared: Vec<Vec<Prepared>> = (0..5)
        .map(|z| {
            livery.zones[z]
                .iter()
                .filter_map(|l| {
                    let shape = lib.shape(l.shape)?;
                    if l.scale[0] == 0.0 || l.scale[1] == 0.0 {
                        return None;
                    }
                    let a = (l.rot - BASE_ROT[z]).to_radians();
                    Some(Prepared {
                        shape,
                        pos: l.pos,
                        cos: a.cos(),
                        sin: a.sin(),
                        inv: [1.0 / l.scale[0], -1.0 / l.scale[1]],
                        skew: l.skew,
                        color: [l.color[0] as f32 / 255.0, l.color[1] as f32 / 255.0, l.color[2] as f32 / 255.0, l.color[3] as f32 / 255.0],
                        blend: l.blend,
                    })
                })
                .collect()
        })
        .collect();
    let mut out = vec![0u8; size * size * 4];
    for y in 0..size {
        let v = (y as f32 + 0.5) / size as f32;
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32;
            let z = zones.zone_at(u, v);
            if z >= 5 || prepared[z].is_empty() {
                continue;
            }
            let (du, dv) = (u - zones.centres[z].x, v - zones.centres[z].y);
            let f = &FRAMES[z];
            let (qx, qy) = (f[0] * du + f[1] * dv, f[2] * du + f[3] * dv);
            // colour times alpha, and alpha
            let mut acc = [0.0f32; 4];
            for p in &prepared[z] {
                let (dx, dy) = (qx - p.pos[0], qy - p.pos[1]);
                let ly = (-p.sin * dx + p.cos * dy) * p.inv[1];
                let lx = (p.cos * dx + p.sin * dy) * p.inv[0] + p.skew * ly;
                let cov = if lx.abs() <= 1.0 && ly.abs() <= 1.0 { p.shape.sample((lx + 1.0) * 0.5, (ly + 1.0) * 0.5) } else { [0.0; 4] };
                let a = cov[3] * p.color[3];
                match p.blend {
                    Blend::Normal => {
                        if a > 0.0 {
                            for c in 0..3 {
                                acc[c] = acc[c] * (1.0 - a) + p.color[c] * cov[c] * a;
                            }
                            acc[3] = acc[3] * (1.0 - a) + a;
                        }
                    }
                    Blend::NegativeCutout => {
                        for c in &mut acc {
                            *c *= 1.0 - a;
                        }
                    }
                    Blend::AlphaCutout => {
                        for c in &mut acc {
                            *c *= a;
                        }
                    }
                }
            }
            if acc[3] > 0.002 {
                let o = (y * size + x) * 4;
                for c in 0..3 {
                    out[o + c] = (acc[c] / acc[3] * 255.0).clamp(0.0, 255.0) as u8;
                }
                out[o + 3] = (acc[3] * 255.0).clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// The presets of the garage (`tune/garage/*_preset_names.vgp` list them; one file per zone).
#[derive(Default)]
pub struct Presets {
    pub names: Vec<String>,
    pub bike_names: Vec<String>,
    dir: PathBuf,
}

impl Presets {
    pub fn load(data_dir: &str) -> Presets {
        let dir = Path::new(data_dir).join("tune/garage");
        let list = |lists: &[&str]| -> Vec<String> {
            let mut names = Vec::new();
            for l in lists {
                if let Ok(t) = std::fs::read_to_string(dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(l)))) {
                    names.extend(t.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && l.parse::<u32>().is_err()).map(|l| l.to_string()));
                }
            }
            names
        };
        let names = list(&["vinyl_preset_names.vgp", "sc_vinyl_preset_names.vgp"]);
        let bike_names = list(&["vinyl_bk_preset_names.vgp"]);
        log::info!("vinyls: {} presets, {} for bikes", names.len(), bike_names.len());
        Presets { names, bike_names, dir }
    }

    pub fn livery(&self, name: &str) -> Livery {
        let mut out = Livery::default();
        for (z, zn) in ["top", "left", "right", "front", "rear"].iter().enumerate() {
            let f = self.dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(&format!("{}_{}.vgp", name, zn))));
            if let Ok(t) = std::fs::read_to_string(f) {
                out.zones[z] = Livery::zone_from_vgp(&t);
            }
        }
        out
    }
}
