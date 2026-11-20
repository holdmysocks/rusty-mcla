//! `grmGeometry` / `grcVertexBuffer` / `grcIndexBuffer` / `grcFvf` decoding.

use super::Rd;
use crate::{bail, Result};
use glam::{Vec2, Vec3, Vec4};

/// Vertex channel ids (bit positions in the fvf mask).
pub mod chan {
    pub const POSITION: usize = 0;
    pub const BLENDWEIGHTS: usize = 1;
    pub const BLENDINDICES: usize = 2;
    pub const NORMAL: usize = 3;
    pub const DIFFUSE: usize = 4;
    pub const SPECULAR: usize = 5;
    pub const TEXCOORD0: usize = 6;
    pub const TEXCOORD1: usize = 7;
    pub const TEXCOORD2: usize = 8;
    pub const TANGENT: usize = 14;
    pub const BINORMAL: usize = 15;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChanType {
    None,
    Half2,
    Float1,
    Half4,
    Float1b,
    Float2,
    Float3,
    Float4,
    UByte4,
    Color,
    Dec3N,
    Unknown(u8),
}

impl ChanType {
    fn from_nibble(n: u8) -> ChanType {
        match n {
            0 => ChanType::None,
            1 => ChanType::Half2,
            2 => ChanType::Float1,
            3 => ChanType::Half4,
            4 => ChanType::Float1b,
            5 => ChanType::Float2,
            6 => ChanType::Float3,
            7 => ChanType::Float4,
            8 => ChanType::UByte4,
            9 => ChanType::Color,
            10 => ChanType::Dec3N,
            o => ChanType::Unknown(o),
        }
    }
    pub fn size(&self) -> usize {
        match self {
            ChanType::None => 0,
            ChanType::Half2 => 4,
            ChanType::Float1 | ChanType::Float1b => 4,
            ChanType::Half4 => 8,
            ChanType::Float2 => 8,
            ChanType::Float3 => 12,
            ChanType::Float4 => 16,
            ChanType::UByte4 | ChanType::Color | ChanType::Dec3N => 4,
            ChanType::Unknown(_) => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Fvf {
    pub mask: u32,
    pub stride: usize,
    pub channels: [(ChanType, usize); 16], // (type, byte offset)
}

impl Fvf {
    pub fn parse(rd: &Rd, off: usize) -> Fvf {
        let mask = rd.u32(off);
        let stride = rd.u8(off + 4) as usize;
        let types = ((rd.u32(off + 8) as u64) << 32) | rd.u32(off + 12) as u64;
        let mut channels = [(ChanType::None, 0usize); 16];
        let mut o = 0usize;
        for i in 0..16 {
            if mask & (1 << i) != 0 {
                let t = ChanType::from_nibble(((types >> (i * 4)) & 0xf) as u8);
                channels[i] = (t, o);
                o += t.size();
            }
        }
        Fvf { mask, stride: if stride == 0 { o } else { stride }, channels }
    }
    pub fn has(&self, c: usize) -> bool {
        self.mask & (1 << c) != 0
    }
}

#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub tangents: Vec<Vec4>,
    pub uv0: Vec<Vec2>,
    pub uv1: Vec<Vec2>,
    pub uv2: Vec<Vec2>,
    pub colors: Vec<[u8; 4]>,
    pub colors1: Vec<[u8; 4]>,
    pub blend_weights: Vec<Vec4>,
    pub blend_indices: Vec<[u8; 4]>,
    pub indices: Vec<u32>,
    pub raw_stride: usize,
    /// skinned meshes: blend indices address this list of skeleton bone indices
    pub bone_palette: Vec<u16>,
}

impl Mesh {
    /// The mesh of the triangles `tris` (indices into the triangle list) alone, its vertices those they use, in first-use
    /// order, with every attribute the mesh has.
    pub fn subset(&self, tris: &[usize]) -> Mesh {
        let mut map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut order: Vec<usize> = Vec::new();
        let mut indices = Vec::with_capacity(tris.len() * 3);
        for &t in tris {
            for k in 0..3 {
                let v = self.indices[t * 3 + k];
                let n = *map.entry(v).or_insert_with(|| {
                    order.push(v as usize);
                    order.len() as u32 - 1
                });
                indices.push(n);
            }
        }
        fn pick<T: Clone>(a: &[T], order: &[usize]) -> Vec<T> {
            if a.is_empty() { Vec::new() } else { order.iter().map(|&i| a[i].clone()).collect() }
        }
        Mesh {
            positions: pick(&self.positions, &order),
            normals: pick(&self.normals, &order),
            tangents: pick(&self.tangents, &order),
            uv0: pick(&self.uv0, &order),
            uv1: pick(&self.uv1, &order),
            uv2: pick(&self.uv2, &order),
            colors: pick(&self.colors, &order),
            colors1: pick(&self.colors1, &order),
            blend_weights: pick(&self.blend_weights, &order),
            blend_indices: pick(&self.blend_indices, &order),
            indices,
            raw_stride: self.raw_stride,
            bone_palette: self.bone_palette.clone(),
        }
    }

    /// The bone a vertex is bound to: the blend index of its largest weight (through the palette when there is one).
    pub fn vertex_bone(&self, v: usize) -> Option<u16> {
        let (w, i) = (self.blend_weights.get(v)?, self.blend_indices.get(v)?);
        let a = w.to_array();
        let k = (0..4).max_by(|&x, &y| a[x].total_cmp(&a[y]))?;
        let b = i[k] as usize;
        Some(if self.bone_palette.is_empty() { b as u16 } else { *self.bone_palette.get(b)? })
    }

    pub fn bbox(&self) -> (Vec3, Vec3) {
        let mut mn = Vec3::splat(f32::INFINITY);
        let mut mx = Vec3::splat(f32::NEG_INFINITY);
        for p in &self.positions {
            mn = mn.min(*p);
            mx = mx.max(*p);
        }
        (mn, mx)
    }
}

fn half(bits: u16) -> f32 {
    let s = ((bits >> 15) & 1) as u32;
    let e = ((bits >> 10) & 0x1f) as i32;
    let m = (bits & 0x3ff) as u32;
    let f = if e == 0 {
        if m == 0 {
            0.0
        } else {
            (m as f32 / 1024.0) * 2f32.powi(-14)
        }
    } else if e == 31 {
        if m == 0 {
            f32::INFINITY
        } else {
            f32::NAN
        }
    } else {
        (1.0 + m as f32 / 1024.0) * 2f32.powi(e - 15)
    };
    if s == 1 {
        -f
    } else {
        f
    }
}

fn dec3n(v: u32) -> Vec4 {
    // 10:10:10:2 signed normalized, x in the low bits
    let sx = ((v & 0x3ff) as i32) << 22 >> 22;
    let sy = (((v >> 10) & 0x3ff) as i32) << 22 >> 22;
    let sz = (((v >> 20) & 0x3ff) as i32) << 22 >> 22;
    let w = (v >> 30) as f32;
    Vec4::new(sx as f32 / 511.0, sy as f32 / 511.0, sz as f32 / 511.0, w)
}

fn read_chan(rd: &Rd, off: usize, t: ChanType) -> Vec4 {
    match t {
        ChanType::None => Vec4::ZERO,
        ChanType::Half2 => Vec4::new(half(rd.u16(off)), half(rd.u16(off + 2)), 0.0, 0.0),
        ChanType::Half4 => Vec4::new(
            half(rd.u16(off)),
            half(rd.u16(off + 2)),
            half(rd.u16(off + 4)),
            half(rd.u16(off + 6)),
        ),
        ChanType::Float1 | ChanType::Float1b => Vec4::new(rd.f32(off), 0.0, 0.0, 0.0),
        ChanType::Float2 => Vec4::new(rd.f32(off), rd.f32(off + 4), 0.0, 0.0),
        ChanType::Float3 => Vec4::new(rd.f32(off), rd.f32(off + 4), rd.f32(off + 8), 0.0),
        ChanType::Float4 => Vec4::new(rd.f32(off), rd.f32(off + 4), rd.f32(off + 8), rd.f32(off + 12)),
        ChanType::UByte4 | ChanType::Color | ChanType::Unknown(_) => Vec4::new(
            rd.u8(off) as f32,
            rd.u8(off + 1) as f32,
            rd.u8(off + 2) as f32,
            rd.u8(off + 3) as f32,
        ),
        ChanType::Dec3N => dec3n(rd.u32(off)),
    }
}

#[derive(Debug, Clone)]
pub struct VertexBuffer {
    pub count: usize,
    pub stride: usize,
    pub data_off: usize,
    pub fvf: Fvf,
}

impl VertexBuffer {
    pub fn parse(rd: &Rd, off: usize) -> Result<VertexBuffer> {
        let count = rd.u16(off + 4) as usize;
        let data_off = rd.ptr(off + 8).or_else(|| rd.ptr(off + 0x18)).ok_or_else(|| crate::anyhow!("vb without data"))?;
        // stride is a u32 at +0x0c (observed "00 00 00 24")
        let stride = (rd.u32(off + 0xc) & 0xffff) as usize;
        let fvf_off = rd.ptr(off + 0x10).ok_or_else(|| crate::anyhow!("vb without fvf"))?;
        let mut fvf = Fvf::parse(rd, fvf_off);
        if stride != 0 {
            fvf.stride = stride;
        }
        if !rd.has(data_off, count * fvf.stride) {
            bail!("vertex data out of range");
        }
        Ok(VertexBuffer { count, stride: fvf.stride, data_off, fvf })
    }
}

#[derive(Debug, Clone)]
pub struct IndexBuffer {
    pub count: usize,
    pub data_off: usize,
}

impl IndexBuffer {
    pub fn parse(rd: &Rd, off: usize) -> Result<IndexBuffer> {
        let count = rd.u32(off + 4) as usize;
        let data_off = rd.ptr(off + 8).ok_or_else(|| crate::anyhow!("ib without data"))?;
        if !rd.has(data_off, count * 2) {
            bail!("index data out of range");
        }
        Ok(IndexBuffer { count, data_off })
    }
}

/// Parse a `grmGeometry` object into a decoded [`Mesh`].
pub fn parse_geometry(rd: &Rd, off: usize) -> Result<Mesh> {
    let vb_off = rd.ptr(off + 0x0c).ok_or_else(|| crate::anyhow!("geometry without vb"))?;
    // vertex buffers occupy +0x0c..+0x18 (4 slots), index buffers +0x1c..+0x28
    let ib_off = rd.ptr(off + 0x1c).ok_or_else(|| crate::anyhow!("geometry without ib"))?;
    let vb = VertexBuffer::parse(rd, vb_off)?;
    let ib = IndexBuffer::parse(rd, ib_off)?;
    decode(rd, &vb, &ib)
}

pub fn decode(rd: &Rd, vb: &VertexBuffer, ib: &IndexBuffer) -> Result<Mesh> {
    let f = &vb.fvf;
    let mut m = Mesh { raw_stride: vb.stride, ..Default::default() };
    let n = vb.count;
    m.positions.reserve(n);
    for i in 0..n {
        let base = vb.data_off + i * vb.stride;
        let get = |c: usize| read_chan(rd, base + f.channels[c].1, f.channels[c].0);
        if f.has(chan::POSITION) {
            m.positions.push(get(chan::POSITION).truncate());
        }
        if f.has(chan::NORMAL) {
            m.normals.push(get(chan::NORMAL).truncate());
        }
        if f.has(chan::TANGENT) {
            m.tangents.push(get(chan::TANGENT));
        }
        if f.has(chan::TEXCOORD0) {
            let v = get(chan::TEXCOORD0);
            m.uv0.push(Vec2::new(v.x, v.y));
        }
        if f.has(chan::TEXCOORD1) {
            let v = get(chan::TEXCOORD1);
            m.uv1.push(Vec2::new(v.x, v.y));
        }
        if f.has(chan::TEXCOORD2) {
            let v = get(chan::TEXCOORD2);
            m.uv2.push(Vec2::new(v.x, v.y));
        }
        if f.has(chan::DIFFUSE) {
            let o = base + f.channels[chan::DIFFUSE].1;
            m.colors.push([rd.u8(o), rd.u8(o + 1), rd.u8(o + 2), rd.u8(o + 3)]);
        }
        if f.has(chan::SPECULAR) {
            let o = base + f.channels[chan::SPECULAR].1;
            m.colors1.push([rd.u8(o), rd.u8(o + 1), rd.u8(o + 2), rd.u8(o + 3)]);
        }
        if f.has(chan::BLENDWEIGHTS) {
            let v = get(chan::BLENDWEIGHTS);
            let t = f.channels[chan::BLENDWEIGHTS].0;
            m.blend_weights.push(if matches!(t, ChanType::Color | ChanType::UByte4) { v / 255.0 } else { v });
        }
        if f.has(chan::BLENDINDICES) {
            let o = base + f.channels[chan::BLENDINDICES].1;
            m.blend_indices.push([rd.u8(o), rd.u8(o + 1), rd.u8(o + 2), rd.u8(o + 3)]);
        }
    }
    m.indices.reserve(ib.count);
    for i in 0..ib.count {
        m.indices.push(rd.u16(ib.data_off + i * 2) as u32);
    }
    Ok(m)
}

/// Bone palette of a skinned `grmGeometry`: +0x38 -> u16 bone indices, count at +0x3e.
pub fn bone_palette(rd: &Rd, geom: usize) -> Vec<u16> {
    let Some(p) = rd.ptr(geom + 0x38) else { return Vec::new() };
    let n = rd.u16(geom + 0x3e) as usize;
    if n == 0 || n > 256 || !rd.has(p, n * 2) {
        return Vec::new();
    }
    (0..n).map(|i| rd.u16(p + i * 2)).collect()
}
