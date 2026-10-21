//! The city's collision (`resources/city/sc/0x3e5fd91d.r1`, class 0x005c9a50): what the
//! original's wheels and bodies touch. It is its own, much coarser geometry than the drawn
//! sectors - 615,221 triangles for the whole map - and every triangle names its physics
//! material.
//!
//! ```text
//! root   +28 -> holder, holder +28 -> the grid bound
//! grid   (type byte +04 = 8)  +80 f32 cell size (700 m)  +88 i32 first column, last column,
//!        columns, first row, last row, rows   +a0 -> cell pointers   +a4 u16 cells
//! cell   (type byte +04 = 10) +8c -> triangles   +90 vec3 vertex scale   +a0 vec3 vertex centre
//!        +b0 -> vertices (3 x i16)   +c8 u32 vertices   +cc u32 triangles
//!        +d0 -> material ids (u32)   +d8 -> material number of each triangle (u8)   +dc u8 materials
//! triangle (16 bytes)  f32 area, 3 x u16 vertex, 3 x u16 neighbouring triangle (0xffff none)
//! ```
//! A material id's low byte is the material's number in the game's table: 0 is "default", then
//! the names of `tune/physics/materials/materials.list` in order, "default" left out. The bits
//! above it are marks (0x8000 on all ground, 0x10000 on walls, bits 8-11 the kind of way:
//! 1 pavement, 2 road, 4 open ground, 8 short cut).

use super::Rd;
use crate::rsc::Resource;
use crate::{bail, Result};

#[derive(Debug, Default, Clone)]
pub struct BoundCell {
    pub verts: Vec<[f32; 3]>,
    pub tris: Vec<[u32; 3]>,
    /// each triangle's neighbours across its three edges, as indices into `tris` (None: none)
    pub neighbours: Vec<[Option<u32>; 3]>,
    /// the material id of each triangle
    pub material: Vec<u32>,
    /// the first number of each triangle's record: its area (what the collision divides the
    /// corners' cross product by, twice, for the triangle's normal)
    pub area: Vec<f32>,
    /// which square of the grid this cell is (x, z index: the grid's +88 / +94 firsts and the
    /// cell's place in the list, index = (z - first z) * columns + (x - first x), 0x825CE120)
    pub grid: (i32, i32),
}

#[derive(Debug, Default)]
pub struct CityBound {
    pub cell_size: f32,
    /// the grid's +84 (one over the cell size, as the file holds it) and +70 (its margin)
    pub inv_size: f32,
    pub margin: f32,
    pub cells: Vec<BoundCell>,
}

impl CityBound {
    pub fn parse(res: &Resource) -> Result<CityBound> {
        let rd = Rd::new(res);
        let grid = rd.ptr(0x28).and_then(|holder| rd.ptr(holder + 0x28));
        let Some(grid) = grid.filter(|&g| rd.has(g, 0xb0) && rd.u8(g + 4) == 8) else {
            bail!("no grid bound in this resource");
        };
        let mut out = CityBound { cell_size: rd.f32(grid + 0x80), inv_size: rd.f32(grid + 0x84), margin: rd.f32(grid + 0x70), cells: Vec::new() };
        let (first_x, columns, first_z) = (rd.u32(grid + 0x88) as i32, (rd.u32(grid + 0x90) as i32).max(1), rd.u32(grid + 0x94) as i32);
        let Some(list) = rd.ptr(grid + 0xa0) else {
            return Ok(out);
        };
        for i in 0..rd.u16(grid + 0xa4) as usize {
            let Some(c) = rd.ptr(list + i * 4) else { continue };
            if !rd.has(c, 0xe0) || rd.u8(c + 4) != 10 {
                continue;
            }
            let (nv, nt, nm) = (rd.u32(c + 0xc8) as usize, rd.u32(c + 0xcc) as usize, rd.u8(c + 0xdc) as usize);
            let (Some(tris), Some(verts), Some(ids), Some(numbers)) = (rd.ptr(c + 0x8c), rd.ptr(c + 0xb0), rd.ptr(c + 0xd0), rd.ptr(c + 0xd8)) else { continue };
            if !rd.has(tris, nt * 16) || !rd.has(verts, nv * 6) || !rd.has(ids, nm * 4) || !rd.has(numbers, nt) {
                bail!("cell {} of the city bound runs past the resource", i);
            }
            let scale = [rd.f32(c + 0x90), rd.f32(c + 0x94), rd.f32(c + 0x98)];
            let centre = [rd.f32(c + 0xa0), rd.f32(c + 0xa4), rd.f32(c + 0xa8)];
            let mut cell = BoundCell { verts: Vec::with_capacity(nv), tris: Vec::with_capacity(nt), neighbours: Vec::with_capacity(nt), material: Vec::with_capacity(nt), area: Vec::with_capacity(nt), grid: (i as i32 % columns + first_x, i as i32 / columns + first_z) };
            // (a triangle left out moves the ones after it up: the file's numbers to ours)
            let mut kept: Vec<Option<u32>> = Vec::with_capacity(nt);
            let mut raw: Vec<[u16; 3]> = Vec::with_capacity(nt);
            for v in 0..nv {
                let o = verts + v * 6;
                let q = |k: usize| rd.u16(o + k * 2) as i16 as f32 * scale[k] + centre[k];
                cell.verts.push([q(0), q(1), q(2)]);
            }
            for t in 0..nt {
                let o = tris + t * 16;
                let v = [rd.u16(o + 4) as u32, rd.u16(o + 6) as u32, rd.u16(o + 8) as u32];
                if v.iter().any(|&x| x as usize >= nv) {
                    kept.push(None);
                    continue;
                }
                kept.push(Some(cell.tris.len() as u32));
                raw.push([rd.u16(o + 10), rd.u16(o + 12), rd.u16(o + 14)]);
                let number = rd.u8(numbers + t) as usize;
                cell.tris.push(v);
                cell.area.push(rd.f32(o));
                cell.material.push(if number < nm { rd.u32(ids + number * 4) } else { 0 });
            }
            cell.neighbours = raw.iter().map(|n| n.map(|i| kept.get(i as usize).copied().flatten())).collect();
            out.cells.push(cell);
        }
        Ok(out)
    }

    pub fn triangle_count(&self) -> usize {
        self.cells.iter().map(|c| c.tris.len()).sum()
    }
}

/// A vehicle's body hull: the polyhedron its fragment file (`.r83`) carries for collisions -
/// eighteen points and thirty-two triangles on a car, eleven and eighteen on a motorcycle -
/// in the car's own frame. Class 0x0059b464, type byte 4:
///
/// ```text
/// +10 / +20 box max / min   +70 margin (0.04)   +80 -> vertices (4 x f32 each)
/// +8c -> triangles (16 bytes, as the city's)   +c8 u32 vertices   +cc u32 triangles
/// +d0 -> material ids (4 car_side, 3 car_roof, 5 car_undercarriage)   +d8 -> u8 per triangle
/// ```
/// (The running game moves some of these points after loading; this is the file's shape.)
#[derive(Debug, Default, Clone)]
pub struct Hull {
    pub verts: Vec<[f32; 3]>,
    pub tris: Vec<[u16; 3]>,
    /// the material id of each triangle
    pub material: Vec<u32>,
    pub margin: f32,
    /// bound +0x50: its centre of gravity
    pub cg: [f32; 3],
    /// bound +186: how many of the corners the detector's scan takes
    pub scan: u16,
    /// its own box, +0x10 max / +0x20 min (the margin included)
    pub bbox: ([f32; 3], [f32; 3]),
    /// the box (max, min) of the composite bound (type 12) that holds this hull as a part, as
    /// the file has it (the running game moves the wheel parts to the wheels and the box with them)
    pub composite: Option<([f32; 3], [f32; 3])>,
    /// the places of that composite's other parts (the wheels: type 5) in the car, from the
    /// part matrices (+0x84, 64 bytes, translation at +48)
    pub wheel_parts: Vec<[f32; 3]>,
}

pub const HULL_CLASS: u32 = 0x0059_b464;

/// The class word of the same polyhedron bound in an ambient (traffic) vehicle's file (`.r155`).
pub const AMBIENT_HULL_CLASS: u32 = 0x0051_05ec;

/// The one hull among a fragment file's objects.
pub fn find_hull(res: &Resource) -> Option<Hull> {
    find_hull_of(res, HULL_CLASS)
}

/// The box (max, min) of the composite bound (type byte 12; +0x80 -> parts, +0x90 u16 count)
/// that has the bound at `part` among its parts.
fn composite_box_of(rd: &Rd, size: usize, part: usize) -> Option<(([f32; 3], [f32; 3]), Vec<[f32; 3]>)> {
    let mut o = 0;
    while o + 0xa0 <= size {
        if rd.u8(o + 4) == 12 {
            let n = rd.u16(o + 0x90) as usize;
            if let Some(parts) = rd.ptr(o + 0x80).filter(|&p| n > 0 && n < 256 && rd.has(p, n * 4)) {
                if (0..n).any(|i| rd.ptr(parts + i * 4) == Some(part)) {
                    let v = |a: usize| [rd.f32(a), rd.f32(a + 4), rd.f32(a + 8)];
                    let mats = rd.ptr(o + 0x84).filter(|&m| rd.has(m, n * 64));
                    let wheels = (0..n)
                        .filter_map(|i| {
                            let b = rd.ptr(parts + i * 4)?;
                            (b != part && rd.u8(b + 4) == 5).then(|| mats.map(|m| v(m + i * 64 + 48))).flatten()
                        })
                        .collect();
                    return Some(((v(o + 0x10), v(o + 0x20)), wheels));
                }
            }
        }
        o += 16;
    }
    None
}

/// The one polyhedron bound of class word `class` among a file's objects (stage R: the traffic
/// vehicles' `resources/ambients/<name>/*.r155`, class `AMBIENT_HULL_CLASS`).
pub fn find_hull_of(res: &Resource, class: u32) -> Option<Hull> {
    let rd = Rd::new(res);
    let mut o = 0;
    while o + 0xe0 <= res.virtual_size {
        if rd.u32le(o) == class && rd.u8(o + 4) == 4 {
            if let Some(h) = hull_at(&rd, res.virtual_size, o) {
                return Some(h);
            }
        }
        o += 16;
    }
    None
}

/// An ambient (traffic) type's bound: the `.r155` root's +8 bound - one polyhedron, or a composite
/// (type 12: a semi trailer's deck and rear block) of polyhedra, each with its part matrix
/// (+0x84, 64 bytes a part; rows, translation at +48) - and the root bound's centroid (+0x30).
pub struct AmbientBound {
    pub parts: Vec<(Hull, [[f32; 3]; 4])>,
    pub centroid: [f32; 3],
}

pub fn ambient_bound(res: &Resource) -> Option<AmbientBound> {
    let rd = Rd::new(res);
    let o = rd.ptr(8)?;
    let centroid = [rd.f32(o + 0x30), rd.f32(o + 0x34), rd.f32(o + 0x38)];
    let ident = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]];
    match rd.u8(o + 4) {
        4 => Some(AmbientBound { parts: vec![(hull_at(&rd, res.virtual_size, o)?, ident)], centroid }),
        12 => {
            let n = rd.u16(o + 0x90) as usize;
            let (parts, mats) = (rd.ptr(o + 0x80)?, rd.ptr(o + 0x84)?);
            if n == 0 || n > 64 || !rd.has(parts, n * 4) || !rd.has(mats, n * 64) {
                return None;
            }
            let mut out = Vec::new();
            for i in 0..n {
                let b = rd.ptr(parts + i * 4)?;
                let m = mats + i * 64;
                let row = |k: usize| [rd.f32(m + k * 16), rd.f32(m + k * 16 + 4), rd.f32(m + k * 16 + 8)];
                if rd.u8(b + 4) != 4 {
                    return None;
                }
                out.push((hull_at(&rd, res.virtual_size, b)?, [row(0), row(1), row(2), row(3)]));
            }
            Some(AmbientBound { parts: out, centroid })
        }
        _ => None,
    }
}

/// The polyhedron bound (type 4) at `o`.
fn hull_at(rd: &Rd, size: usize, o: usize) -> Option<Hull> {
    {
        {
            let (nv, nt, nm) = (rd.u32(o + 0xc8) as usize, rd.u32(o + 0xcc) as usize, rd.u8(o + 0xdc) as usize);
            if let (Some(verts), Some(tris)) = (rd.ptr(o + 0x80), rd.ptr(o + 0x8c)) {
                if nv >= 4 && nv < 4096 && nt >= 4 && nt < 8192 && rd.has(verts, nv * 16) && rd.has(tris, nt * 16) {
                    let ids = rd.ptr(o + 0xd0).filter(|&p| rd.has(p, nm * 4));
                    let numbers = rd.ptr(o + 0xd8).filter(|&p| rd.has(p, nt));
                    let mut hull = Hull { margin: rd.f32(o + 0x70), cg: [rd.f32(o + 0x50), rd.f32(o + 0x54), rd.f32(o + 0x58)], scan: rd.u16(o + 186), ..Default::default() };
                    for v in 0..nv {
                        hull.verts.push([rd.f32(verts + v * 16), rd.f32(verts + v * 16 + 4), rd.f32(verts + v * 16 + 8)]);
                    }
                    for t in 0..nt {
                        let p = tris + t * 16;
                        let v = [rd.u16(p + 4), rd.u16(p + 6), rd.u16(p + 8)];
                        if v.iter().any(|&x| x as usize >= nv) {
                            continue;
                        }
                        hull.tris.push(v);
                        let number = numbers.map_or(0, |n| rd.u8(n + t) as usize);
                        hull.material.push(ids.filter(|_| number < nm).map_or(0, |i| rd.u32(i + number * 4)));
                    }
                    if let Some((b, w)) = composite_box_of(rd, size, o) {
                        hull.composite = Some(b);
                        hull.wheel_parts = w;
                    }
                    hull.bbox = ([rd.f32(o + 0x10), rd.f32(o + 0x14), rd.f32(o + 0x18)], [rd.f32(o + 0x20), rd.f32(o + 0x24), rd.f32(o + 0x28)]);
                    return Some(hull);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    /// Every car's hull sits in a composite bound with the wheels as its other parts
    /// (`MCLA_BOX_LOG=1 MCLA_BOX_PARTS=1` prints them).
    #[test]
    fn car_composite_boxes() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(dirs) = std::fs::read_dir(std::path::Path::new(&root).join("resources/vehicle")) else { return };
        let mut seen = 0;
        for d in dirs.flatten().filter(|d| d.file_name().to_string_lossy().starts_with("vp")) {
            let Ok(files) = std::fs::read_dir(d.path()) else { continue };
            for f in files.flatten().filter(|f| f.path().extension().map_or(false, |e| e == "r83")) {
                let Ok(b) = std::fs::read(f.path()) else { continue };
                let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
                let data = b[16..].to_vec();
                let vs = (u(12) as usize).min(data.len());
                let res = crate::rsc::Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data };
                let Some(h) = super::find_hull(&res) else { continue };
                let (lo, hi) = h.verts.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), v| ([lo[0].min(v[0]), lo[1].min(v[1]), lo[2].min(v[2])], [hi[0].max(v[0]), hi[1].max(v[1]), hi[2].max(v[2])]));
                if std::env::var("MCLA_BOX_LOG").is_ok() {
                    println!("{} hull y {:.3}..{:.3} margin {:.3} composite {:?}", d.file_name().to_string_lossy(), lo[1], hi[1], h.margin, h.composite);
                    if std::env::var("MCLA_BOX_PARTS").is_ok() {
                        let rd = super::Rd::new(&res);
                        let mut o = 0;
                        while o + 0xa0 <= res.virtual_size {
                            if rd.u8(o + 4) == 12 {
                                let n = rd.u16(o + 0x90) as usize;
                                if let (Some(parts), Some(mats)) = (rd.ptr(o + 0x80), rd.ptr(o + 0x84)) {
                                    if n > 0 && n < 64 && rd.has(parts, n * 4) && rd.has(mats, n * 64) {
                                        for i in 0..n {
                                            let Some(b) = rd.ptr(parts + i * 4) else { continue };
                                            let m = mats + i * 64;
                                            println!("   part {i} type {} radius {:.3} max ({:.3} {:.3} {:.3}) min ({:.3} {:.3} {:.3}) margin {:.3} at ({:.3} {:.3} {:.3})", rd.u8(b + 4), rd.f32(b + 8), rd.f32(b + 0x10), rd.f32(b + 0x14), rd.f32(b + 0x18), rd.f32(b + 0x20), rd.f32(b + 0x24), rd.f32(b + 0x28), rd.f32(b + 0x70), rd.f32(m + 48), rd.f32(m + 52), rd.f32(m + 56));
                                        }
                                    }
                                }
                            }
                            o += 16;
                        }
                    }
                }
                assert!(h.composite.is_some(), "{:?}", f.path());
                seen += 1;
                break;
            }
        }
        if seen > 0 {
            println!("{seen} cars");
        }
    }
}
