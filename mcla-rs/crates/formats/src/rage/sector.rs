//! City sector resources (`resources/city/sc/*.r1`, class 0x005cb710):
//! a texture dictionary, a shader group and a list of (shader, geometry) draw batches
//! whose vertices are already in world space.

use super::drawable::Shader;
use super::geometry::{parse_geometry, Mesh};
use super::texture::Texture;
use super::{vt, Dictionary, Rd};
use crate::rsc::Resource;
use crate::{bail, Result};
use hashbrown::HashMap;

#[derive(Debug, Clone)]
pub struct Batch {
    pub shader: usize,
    pub mesh: Mesh,
    /// low-nibble flags from the shader pointer (meaning TBD: 1/2/4 seen)
    pub flags: u32,
    /// the batch's box from the file (min, max): the table at +0x1C holds a sphere (16 bytes) for
    /// every batch, then a box (32 bytes) for every batch; the box is what the original's visibility
    /// walk tests each batch by (F-76875, called per batch by F-21213)
    pub bounds: Option<(glam::Vec3, glam::Vec3)>,
}

#[derive(Debug, Default)]
pub struct Sector {
    /// textures keyed by name hash (joaat of lower-case name including ".dds")
    pub textures: HashMap<u32, Texture>,
    pub shaders: Vec<Shader>,
    pub batches: Vec<Batch>,
    pub errors: Vec<String>,
}

impl Sector {
    pub fn parse(res: &Resource) -> Result<Sector> {
        let rd = Rd::new(res);
        if rd.vtable(0) != vt::CITY_SECTOR {
            bail!("not a city sector (vtable {:#x})", rd.vtable(0));
        }
        let mut sec = Sector::default();
        // textures
        if let Some(td) = rd.ptr(0x08) {
            match Dictionary::parse(&rd, td) {
                Ok(d) => {
                    for (h, v) in d.hashes.iter().zip(d.values.iter()) {
                        if let Some(o) = v {
                            match Texture::parse(&rd, *o) {
                                Ok(t) => {
                                    sec.textures.insert(*h, t);
                                }
                                Err(e) => sec.errors.push(format!("texture {:08x}: {:#}", h, e)),
                            }
                        }
                    }
                }
                Err(e) => sec.errors.push(format!("texture dict: {:#}", e)),
            }
        }
        // batches
        let count = rd.u16(0x24) as usize;
        let Some(arr) = rd.ptr(0x18) else {
            return Ok(sec);
        };
        let mut shader_index: HashMap<usize, usize> = HashMap::new();
        let boxes = rd.ptr(0x1c).map(|b| b + count * 16);
        for i in 0..count {
            // low nibble of the shader pointer carries per-batch flags
            let flags = rd.u32(arr + i * 8) & 0xF;
            let so = rd.res.ptr(rd.u32(arr + i * 8) & !0xF);
            let go = rd.res.ptr(rd.u32(arr + i * 8 + 4) & !0xF);
            if i == 0 {
                log::debug!("batch0 so={:?} go={:?} vs={:#x} ib_word={:#x}", so, go, res.virtual_size, go.map(|g| rd.u32(g + 0x18)).unwrap_or(0));
            }
            let (Some(so), Some(go)) = (so, go) else {
                continue;
            };
            if rd.vtable(so) != vt::SHADER || rd.vtable(go) != vt::GEOMETRY {
                sec.errors.push(format!("batch {} unexpected classes {:#x}/{:#x}", i, rd.vtable(so), rd.vtable(go)));
                continue;
            }
            let si = match shader_index.get(&so) {
                Some(&i) => i,
                None => {
                    let s = Shader::parse(&rd, so)?;
                    sec.shaders.push(s);
                    let i = sec.shaders.len() - 1;
                    shader_index.insert(so, i);
                    i
                }
            };
            match parse_geometry(&rd, go) {
                Ok(mesh) => {
                    let v = |o: usize| glam::Vec3::new(rd.f32(o), rd.f32(o + 4), rd.f32(o + 8));
                    let bounds = boxes.map(|b| (v(b + i * 32), v(b + i * 32 + 16))).filter(|(mn, mx)| mn.is_finite() && mx.is_finite() && mn.cmple(*mx).all());
                    sec.batches.push(Batch { shader: si, mesh, flags, bounds })
                }
                Err(e) => sec.errors.push(format!("batch {} geometry: {:#}", i, e)),
            }
        }
        Ok(sec)
    }

    /// Look up a texture by shader parameter name.
    pub fn texture_by_name(&self, name: &str) -> Option<&Texture> {
        let h = crate::hash::joaat(name);
        if let Some(t) = self.textures.get(&h) {
            return Some(t);
        }
        // try without extension
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        self.textures.get(&crate::hash::joaat(stem))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file's batch boxes (+0x1C, after the spheres) against the meshes' own bounds, over every sector of
    /// the city: equal to a few millimetres (the vertices are packed) except a handful, where the file's box
    /// is larger and holds the mesh's.
    #[test]
    fn batch_boxes_hold_the_meshes() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(dir) = std::fs::read_dir(std::path::Path::new(&root).join("resources/city/sc")) else { return };
        let (mut n, mut larger) = (0, 0);
        for e in dir.flatten() {
            let Ok(d) = std::fs::read(e.path()) else { continue };
            if d.len() < 16 || &d[0..4] != b"MCLR" {
                continue;
            }
            let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
            let data = d[16..].to_vec();
            let vs = (u(12) as usize).min(data.len());
            let res = Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data };
            if Rd::new(&res).vtable(0) != vt::CITY_SECTOR {
                continue;
            }
            let Ok(sec) = Sector::parse(&res) else { continue };
            for b in &sec.batches {
                let (mn, mx) = b.mesh.bbox();
                let (fa, fz) = b.bounds.expect("a box for every batch");
                let off = (fa - mn).abs().max((fz - mx).abs()).max_element();
                assert!(fa.cmple(mn + 0.01).all() && fz.cmpge(mx - 0.01).all(), "{:?}: the file's box does not hold the mesh", e.path());
                n += 1;
                if off > 0.05 {
                    larger += 1;
                }
            }
        }
        println!("{n} batches, {larger} with a larger box in the file");
        assert!(n == 0 || (n > 10_000 && larger * 1000 < n));
    }
}
