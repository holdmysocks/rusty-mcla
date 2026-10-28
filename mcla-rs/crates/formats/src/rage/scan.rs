//! Generic drawable support: shader groups plus geometry/model discovery by
//! scanning the virtual block. This avoids depending on the exact class layout of
//! every drawable flavour (vehicles, parts, props, characters).

use super::drawable::Shader;
use super::geometry::{decode, IndexBuffer, Mesh, VertexBuffer};
use super::{Dictionary, Rd};
use crate::Result;
use hashbrown::{HashMap, HashSet};

/// `grmShaderGroup` as used by vehicles / props: +04 texture dictionary, +08 shader array, +0c count.
#[derive(Debug, Default)]
pub struct ShaderGroup {
    pub shaders: Vec<Shader>,
    pub textures: Dictionary,
}

impl ShaderGroup {
    pub fn parse(rd: &Rd, off: usize) -> Result<ShaderGroup> {
        let mut g = ShaderGroup::default();
        if let Some(td) = rd.ptr(off + 0x04) {
            g.textures = Dictionary::parse(rd, td).unwrap_or_default();
        }
        let count = rd.u16(off + 0x0c) as usize;
        if let Some(arr) = rd.ptr(off + 0x08) {
            for i in 0..count.min(512) {
                if let Some(so) = rd.ptr(arr + i * 4) {
                    // one unreadable shader must not cost the whole group
                    g.shaders.push(Shader::parse(rd, so).unwrap_or(Shader { name: String::new(), name_hash: 0, source_path: String::new(), params: vec![], bucket: 0 }));
                } else {
                    g.shaders.push(Shader { name: String::new(), name_hash: 0, source_path: String::new(), params: vec![], bucket: 0 });
                }
            }
        }
        Ok(g)
    }
}

/// A geometry located by scanning.
pub struct GeomRef {
    pub off: usize,
    pub vb: VertexBuffer,
    pub ib: IndexBuffer,
}

/// Does `off` look like a grmGeometry (vb at +0c, ib at +1c, index count at +2c)?
pub fn geometry_at(rd: &Rd, off: usize) -> Option<GeomRef> {
    let vt = rd.vtable(off);
    if vt & 0xFF00_0000 != 0 || vt < 0x0040_0000 {
        return None;
    }
    let vb_off = rd.ptr(off + 0x0c)?;
    let ib_off = rd.ptr(off + 0x1c)?;
    let vb = VertexBuffer::parse(rd, vb_off).ok()?;
    let ib = IndexBuffer::parse(rd, ib_off).ok()?;
    if vb.count == 0 || ib.count < 3 || vb.stride < 12 || vb.stride > 128 || !vb.fvf.has(0) {
        return None;
    }
    if rd.u32(off + 0x2c) as usize != ib.count {
        return None;
    }
    Some(GeomRef { off, vb, ib })
}

/// A model: geometries + shader index per geometry.
#[derive(Debug, Clone, Default)]
pub struct ModelRef {
    pub off: usize,
    pub geometries: Vec<usize>,
    pub shader_index: Vec<u16>,
    /// bone index into the fragment's own skeleton (from the model group at root+0x08)
    pub bone: u16,
    pub tag: u16,
}

/// Scan the virtual block for model objects (`+04` → array of geometry pointers,
/// `+08` u16 count, `+10` → u16 shader indices).
pub fn scan_models(rd: &Rd) -> Vec<ModelRef> {
    let vs = rd.res.virtual_size;
    let mut out = Vec::new();
    let mut o = 0usize;
    while o + 0x20 <= vs {
        let cnt = rd.u16(o + 0x08) as usize;
        if cnt > 0 && cnt < 512 {
            if let Some(arr) = rd.ptr(o + 0x04) {
                if rd.has(arr, cnt * 4) {
                    let mut geoms = Vec::with_capacity(cnt);
                    let mut ok = true;
                    for i in 0..cnt {
                        match rd.ptr(arr + i * 4) {
                            Some(g) if geometry_at(rd, g).is_some() => geoms.push(g),
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        let mut shader_index = Vec::with_capacity(cnt);
                        if let Some(si) = rd.ptr(o + 0x10) {
                            for i in 0..cnt {
                                shader_index.push(rd.u16(si + i * 2));
                            }
                        }
                        out.push(ModelRef { off: o, geometries: geoms, shader_index, bone: 0, tag: 0 });
                        o += 0x10;
                        continue;
                    }
                }
            }
        }
        o += 4;
    }
    out
}

pub struct DrawModel {
    pub off: usize,
    pub parts: Vec<(Mesh, u16)>,
    pub bone: u16,
    pub tag: u16,
}

/// Fully decoded drawable: models with meshes, shader indices, shader group.
#[derive(Default)]
pub struct Drawable {
    pub shader_group: ShaderGroup,
    pub models: Vec<DrawModel>,
    /// local texture objects by name hash (offsets into this resource)
    pub texture_offsets: HashMap<u32, usize>,
}

impl Drawable {
    /// Parse any drawable-like resource.
    pub fn parse(res: &crate::rsc::Resource) -> Result<Drawable> {
        let rd = Rd::new(res);
        let mut d = Drawable::default();
        for slot in [0x0c, 0x08, 0x10, 0x14] {
            if let Some(sg) = rd.ptr(slot) {
                if let Ok(g) = ShaderGroup::parse(&rd, sg) {
                    if !g.shaders.is_empty() && g.shaders.iter().any(|s| !s.name.is_empty()) {
                        d.shader_group = g;
                        break;
                    }
                }
            }
        }
        for (h, v) in d.shader_group.textures.hashes.iter().zip(d.shader_group.textures.values.iter()) {
            if let Some(o) = v {
                d.texture_offsets.insert(*h, *o);
            }
        }
        // Fragment model group: root+0x08 -> {+00 -> model ptrs, +04 u16 count,
        // +08 -> u32 bone index per model, +0c u16 count}. This is the only place
        // that binds a model to a bone of the fragment's skeleton.
        let mut bone_of: HashMap<usize, u16> = HashMap::new();
        if let Some(g) = rd.ptr(0x08) {
            let n = rd.u16(g + 4) as usize;
            if n > 0 && n < 256 && rd.u16(g + 0x0c) as usize == n {
                if let (Some(a1), Some(a2)) = (rd.ptr(g), rd.ptr(g + 8)) {
                    if rd.has(a1, n * 4) && rd.has(a2, n * 4) {
                        for i in 0..n {
                            if let Some(m) = rd.ptr(a1 + i * 4) {
                                let b = rd.u32(a2 + i * 4);
                                if b < 512 {
                                    bone_of.insert(m, b as u16);
                                }
                            }
                        }
                    }
                }
            }
        }
        let models = scan_models(&rd);
        let mut seen = HashSet::new();
        for mut m in models {
            m.bone = bone_of.get(&m.off).copied().unwrap_or(0);
            m.tag = 0;
            let mut parts = Vec::new();
            for (i, &g) in m.geometries.iter().enumerate() {
                if !seen.insert(g) {
                    continue;
                }
                if let Some(gr) = geometry_at(&rd, g) {
                    if let Ok(mut mesh) = decode(&rd, &gr.vb, &gr.ib) {
                        mesh.bone_palette = super::geometry::bone_palette(&rd, g);
                        parts.push((mesh, m.shader_index.get(i).copied().unwrap_or(0)));
                    }
                }
            }
            if !parts.is_empty() {
                d.models.push(DrawModel { off: m.off, parts, bone: m.bone, tag: m.tag });
            }
        }
        Ok(d)
    }

    pub fn triangle_count(&self) -> usize {
        self.models.iter().flat_map(|m| m.parts.iter()).map(|(m, _)| m.indices.len() / 3).sum()
    }
}
