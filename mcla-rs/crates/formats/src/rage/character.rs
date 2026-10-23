//! Character sets (`resources/character/*_set/*.r6`): pedestrians, drivers, police.
//!
//! The resource starts with a 0x30-byte header followed by the bone array: `count`
//! (u32 at +0x20) bones of 0xE0 bytes each
//! `{+00 name ptr, +04 flags, +08 next sibling, +0c first child, +10 parent, +14 u16 index,
//! +16 u16 id, +20 local position, +40 local rotation quaternion (xyzw), +60 world position}`.
//! The skinned geometry lives in an ordinary drawable (found by scanning); its blend
//! indices address this bone array directly. Textures are embedded texture objects.

use super::geometry::Mesh;
use super::scan::{Drawable, ShaderGroup};
use super::texture::Texture;
use super::Rd;
use crate::rsc::Resource;
use crate::Result;
use glam::{Quat, Vec3};

#[derive(Debug, Clone)]
pub struct CharBone {
    pub name: String,
    /// id used by animation tracks
    pub id: u16,
    pub parent: Option<usize>,
    pub local_pos: Vec3,
    /// rest pose local rotation. The skinning bind pose itself has no rotations: every
    /// bone's bind matrix is a pure translation to `world_pos` (the stored world offsets
    /// are exactly the sums of the local offsets).
    pub local_rot: Quat,
    /// bind pose position in model space
    pub world_pos: Vec3,
}

pub struct Character {
    pub name: String,
    pub bones: Vec<CharBone>,
    /// every model of the drawable: (meshes with shader index)
    pub models: Vec<Vec<(Mesh, u16)>>,
    /// model indices per level of detail (high, medium, low)
    pub lods: [Vec<usize>; 3],
    /// diffuse texture name per shader
    pub shader_tex: Vec<String>,
    /// shader names
    pub shader_names: Vec<String>,
    /// the shaders with their material parameters (SpecExp, wrapAmount, the maps, ...)
    pub shaders: Vec<super::drawable::Shader>,
    pub textures: Vec<Texture>,
}

const BONE_STRIDE: usize = 0xE0;
const BONES_AT: usize = 0x30;
/// LE vtable of an embedded texture object
const TEX_VT: u32 = 0x0061_4064;

impl Character {
    pub fn parse(res: &Resource) -> Result<Character> {
        let rd = Rd::new(res);
        let name = rd.ptr(0x08).map(|p| rd.cstr(p)).unwrap_or_default();
        let count = (rd.u32(0x20) as usize).min(256);
        let mut bones = Vec::with_capacity(count);
        for i in 0..count {
            let o = BONES_AT + i * BONE_STRIDE;
            if !rd.has(o, BONE_STRIDE) {
                break;
            }
            let v = |k: usize| Vec3::new(rd.f32(o + k), rd.f32(o + k + 4), rd.f32(o + k + 8));
            let parent = rd.ptr(o + 0x10).filter(|p| *p >= BONES_AT).map(|p| (p - BONES_AT) / BONE_STRIDE).filter(|p| *p < count && *p != i);
            let q = Quat::from_xyzw(rd.f32(o + 0x40), rd.f32(o + 0x44), rd.f32(o + 0x48), rd.f32(o + 0x4c));
            bones.push(CharBone {
                name: rd.ptr(o).map(|p| rd.cstr(p)).unwrap_or_default(),
                id: rd.u16(o + 0x16),
                parent,
                local_pos: v(0x20),
                local_rot: if q.is_finite() && q.length_squared() > 0.5 { q.normalize() } else { Quat::IDENTITY },
                world_pos: v(0x60),
            });
        }
        let d = Drawable::parse(res)?;
        // the drawable itself hangs off root+0x10: +08 shader group, +40.. LOD model lists
        let mut shader_tex = Vec::new();
        let mut shader_names = Vec::new();
        let mut shaders = Vec::new();
        let mut lods: [Vec<usize>; 3] = Default::default();
        // usually at root+0x10; some sets (bike riders) keep it elsewhere: find the object
        // whose +08 points at a shader group {+08 -> shaders, +0c count == +0e} and that has
        // a LOD list at +40
        let looks_like = |o: usize| -> bool {
            let Some(sg) = rd.ptr(o + 0x08) else { return false };
            let n = rd.u16(sg + 0x0c);
            rd.ptr(sg + 0x08).is_some() && n > 0 && n < 64 && n == rd.u16(sg + 0x0e) && rd.ptr(o + 0x40).is_some() && rd.u32(o) & 0xff == 0
        };
        let drawable = rd.ptr(0x10).filter(|&o| looks_like(o)).or_else(|| (0..res.virtual_size.saturating_sub(0x50)).step_by(4).find(|&o| looks_like(o)));
        if let Some(dr) = drawable {
            if let Some(Ok(g)) = rd.ptr(dr + 0x08).map(|sg| ShaderGroup::parse(&rd, sg)) {
                shader_tex = g.shaders.iter().map(|s| s.diffuse().unwrap_or("").to_string()).collect();
                shader_names = g.shaders.iter().map(|s| s.name.clone()).collect();
                shaders = g.shaders;
            }
            for (l, lod) in lods.iter_mut().enumerate() {
                let Some(list) = rd.ptr(dr + 0x40 + l * 4) else { continue };
                let n = rd.u16(list + 4) as usize;
                let Some(arr) = rd.ptr(list) else { continue };
                for i in 0..n.min(64) {
                    if let Some(m) = rd.ptr(arr + i * 4) {
                        if let Some(mi) = d.models.iter().position(|dm| dm.off == m) {
                            lod.push(mi);
                        }
                    }
                }
            }
        }
        if lods[0].is_empty() {
            lods[0] = (0..d.models.len()).collect();
        }
        let models = d.models.into_iter().map(|m| m.parts).collect();
        // embedded textures: scan the virtual segment for texture objects
        let mut textures = Vec::new();
        let vs = res.virtual_size;
        let mut o = 0;
        while o + 0x30 <= vs {
            if rd.u32(o) == TEX_VT.swap_bytes() {
                if let Ok(t) = Texture::parse(&rd, o) {
                    if !t.name.is_empty() && t.width >= 8 {
                        textures.push(t);
                    }
                }
            }
            o += 4;
        }
        Ok(Character { name, bones, models, lods, shader_tex, shader_names, shaders, textures })
    }

    pub fn bone(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    }
}
