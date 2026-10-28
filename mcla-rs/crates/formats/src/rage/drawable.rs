//! Shader instances (`grmShaderFx`) and their parameters.

use super::Rd;
use crate::Result;
use glam::Vec4;

#[derive(Debug, Clone)]
pub enum ShaderParam {
    /// Texture reference by name (e.g. "l_do_bn_wall_concrete_03.dds")
    Texture { name: String },
    /// One or more float4 constants
    Vector(Vec<Vec4>),
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Shader {
    /// e.g. "CityGrime", "CityRoad", "UnlitRuntimeAlpha"
    pub name: String,
    pub name_hash: u32,
    pub source_path: String,
    /// (parameter name hash, value)
    pub params: Vec<(u32, ShaderParam)>,
    /// header byte +0x09: the draw bucket - grmModel's draw (F-73936) draws a geometry only in the pass whose bucket
    /// equals its shader's: 0 opaque, 1 the blended pass after the opaques, 2 LOD only (the cars, F-73627)
    pub bucket: u8,
}

impl Shader {
    pub fn parse(rd: &Rd, off: usize) -> Result<Shader> {
        let count = rd.u16(off + 0x18) as usize;
        let params_ptr = rd.ptr(off + 0x10);
        let types_ptr = rd.ptr(off + 0x1c);
        let hashes_ptr = rd.ptr(off + 0x24);
        let name_hash = rd.u32(off + 0x20);
        let bucket = rd.u8(off + 0x09);
        let name = rd.cstr_ptr(off + 0x30).unwrap_or_default();
        let source_path = rd.cstr_ptr(off + 0x38).unwrap_or_default();
        let mut params = Vec::with_capacity(count);
        if let Some(pp) = params_ptr {
            for i in 0..count.min(64) {
                let h = hashes_ptr.map(|hp| rd.u32(hp + i * 4)).unwrap_or(0);
                let t = types_ptr.map(|tp| rd.u8(tp + i)).unwrap_or(0);
                let value = match rd.ptr(pp + i * 4) {
                    None => ShaderParam::Unknown,
                    Some(vo) => {
                        if t == 0 {
                            // texture object: +0x18 → name string
                            match rd.cstr_ptr(vo + 0x18) {
                                Some(n) if !n.is_empty() => ShaderParam::Texture { name: n },
                                _ => ShaderParam::Unknown,
                            }
                        } else {
                            let n = (t as usize).max(1);
                            let mut v = Vec::with_capacity(n);
                            for k in 0..n {
                                let o = vo + k * 16;
                                v.push(Vec4::new(rd.f32(o), rd.f32(o + 4), rd.f32(o + 8), rd.f32(o + 12)));
                            }
                            ShaderParam::Vector(v)
                        }
                    }
                };
                params.push((h, value));
            }
        }
        Ok(Shader { name, name_hash, source_path, params, bucket })
    }

    /// First texture parameter name, usually the diffuse map.
    pub fn diffuse(&self) -> Option<&str> {
        self.params.iter().find_map(|(_, p)| match p {
            ShaderParam::Texture { name } => Some(name.as_str()),
            _ => None,
        })
    }

    pub fn textures(&self) -> impl Iterator<Item = (u32, &str)> {
        self.params.iter().filter_map(|(h, p)| match p {
            ShaderParam::Texture { name } => Some((*h, name.as_str())),
            _ => None,
        })
    }
}
