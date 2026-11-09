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
    /// the record's +26 / +27: the frame made from the skeleton holds a translation / a rotation DOF for the bone
    pub dof_trans: bool,
    pub dof_rot: bool,
    /// the record's default translation and rotation words as stored (+32 .. +47, +64 .. +79): what the rest pose op copies
    /// into a frame (F-26010's identity branch -> F-90682 -> F-51341, no normalising; the translation's w word too)
    pub rest_raw: [[f32; 4]; 2],
}

pub struct Character {
    pub name: String,
    pub bones: Vec<CharBone>,
    /// every model of the drawable: (meshes with shader index)
    pub models: Vec<Vec<(Mesh, u16)>>,
    /// model indices per level of detail (high, medium, low)
    pub lods: [Vec<usize>; 3],
    /// the cockpit view model (root+0x18: the type's +24, drawn by F-56050): the arms and hands the player sees from the
    /// driver's seat - its high LOD's model indices; empty when the set has none (16 of the 57 `_set_h` sets have one)
    pub cockpit: Vec<usize>,
    /// diffuse texture name per shader
    pub shader_tex: Vec<String>,
    /// shader names
    pub shader_names: Vec<String>,
    /// the shaders with their material parameters (SpecExp, wrapAmount, the maps, ...)
    pub shaders: Vec<super::drawable::Shader>,
    pub textures: Vec<Texture>,
    /// the layout signature of the frame the skeleton component makes (F-21688 -> F-66930: the bones' DOFs sorted, F-59824;
    /// `skeleton::layout_signature`); a clip whose +16 equals it is sampled on the same-layout path
    pub frame_layout: u32,
}

const BONE_STRIDE: usize = 0xE0;
const BONES_AT: usize = 0x30;
/// LE vtable of an embedded texture object
const TEX_VT: u32 = 0x0061_4064;

impl Character {
    pub fn parse(res: &Resource) -> Result<Character> {
        let rd = Rd::new(res);
        let name = rd.ptr(0x08).map(|p| rd.cstr(p)).unwrap_or_default();
        // (the count is the skeleton data's +20 too: the prop sets (prp_*) keep a pointer at the root's +0x20, not a count -
        // F-66930 run offline on all 290 sets with skeleton data reads S+20; the root's +0x20 only where the drawable has none)
        let sk_count = rd.ptr(0x10).and_then(|dr| rd.ptr(dr + 0x0c)).map(|sk| rd.u16(sk + 20) as usize).filter(|&n| n > 0 && n <= 256);
        let count = sk_count.unwrap_or_else(|| (rd.u32(0x20) as usize).min(256));
        // (the bones the character is drawn and animated with are the drawable's skeleton data's, drawable (root+0x10) +0x0c
        // -> +0: the creature's skeleton component, the frame it makes (F-66930) and the rest pose (F-51341) read those. The
        // root's own records at 0x30 are a second copy that differs in 7 driver sets - the player's drv_mp_01_set_h has no
        // translation DOF for its spine, arms and legs there, five others other face bones by up to 11 cm; the root's copy
        // only where the drawable has none)
        let base = rd
            .ptr(0x10)
            .and_then(|dr| rd.ptr(dr + 0x0c))
            .filter(|&sk| rd.u16(sk + 20) as usize == count)
            .and_then(|sk| rd.ptr(sk))
            .filter(|&b| rd.has(b, count * BONE_STRIDE))
            .unwrap_or(BONES_AT);
        let mut bones = Vec::with_capacity(count);
        for i in 0..count {
            let o = base + i * BONE_STRIDE;
            if !rd.has(o, BONE_STRIDE) {
                break;
            }
            let v = |k: usize| Vec3::new(rd.f32(o + k), rd.f32(o + k + 4), rd.f32(o + k + 8));
            let v4 = |k: usize| [rd.f32(o + k), rd.f32(o + k + 4), rd.f32(o + k + 8), rd.f32(o + k + 12)];
            let parent = rd.ptr(o + 0x10).filter(|p| *p >= base).map(|p| (p - base) / BONE_STRIDE).filter(|p| *p < count && *p != i);
            let q = Quat::from_xyzw(rd.f32(o + 0x40), rd.f32(o + 0x44), rd.f32(o + 0x48), rd.f32(o + 0x4c));
            bones.push(CharBone {
                name: rd.ptr(o).map(|p| rd.cstr(p)).unwrap_or_default(),
                id: rd.u16(o + 0x16),
                parent,
                local_pos: v(0x20),
                local_rot: if q.is_finite() && q.length_squared() > 0.5 { q.normalize() } else { Quat::IDENTITY },
                world_pos: v(0x60),
                dof_trans: rd.u8(o + 26) != 0,
                dof_rot: rd.u8(o + 27) != 0,
                rest_raw: [v4(0x20), v4(0x40)],
            });
        }
        // the DOFs' id: the record's +22 when the skeleton data's (drawable +0x0c) +28 has bit 1, else the bone's index
        let ids_on = rd.ptr(0x10).and_then(|dr| rd.ptr(dr + 0x0c)).map_or(true, |sk| rd.u32(sk + 28) & 2 != 0);
        let mut dofs: Vec<(u8, u8, u16)> = Vec::new();
        for (i, b) in bones.iter().enumerate() {
            let id = if ids_on { b.id } else { i as u16 };
            if b.dof_trans {
                dofs.push((0, 0, id));
            }
            if b.dof_rot {
                dofs.push((1, 1, id));
            }
        }
        dofs.sort_by_key(|&(_, k, id)| ((k as u32) << 16) | id as u32);
        let frame_layout = super::skeleton::layout_signature(&dofs);
        let d = Drawable::parse(res)?;
        // the drawable itself hangs off root+0x10: +08 shader group, +40.. LOD model lists
        let mut shader_tex = Vec::new();
        let mut shader_names = Vec::new();
        let mut shaders = Vec::new();
        let mut lods: [Vec<usize>; 3] = Default::default();
        let mut cockpit = Vec::new();
        let offs: Vec<usize> = d.models.iter().map(|m| m.off).collect();
        let mut models: Vec<Vec<(Mesh, u16)>> = d.models.into_iter().map(|m| m.parts).collect();
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
            let list_of = |dr: usize, l: usize| -> Vec<usize> {
                let mut out = Vec::new();
                let Some(list) = rd.ptr(dr + 0x40 + l * 4) else { return out };
                let n = rd.u16(list + 4) as usize;
                let Some(arr) = rd.ptr(list) else { return out };
                for i in 0..n.min(64) {
                    if let Some(m) = rd.ptr(arr + i * 4) {
                        if let Some(mi) = offs.iter().position(|&o| o == m) {
                            out.push(mi);
                        }
                    }
                }
                out
            };
            for (l, lod) in lods.iter_mut().enumerate() {
                *lod = list_of(dr, l);
            }
            // the cockpit view model hangs off root+0x18 (the character type's +24, placed by the type fixup
            // F-11329): a drawable of its own; with a shader group of its own its shaders follow the main ones
            if let Some(cd) = rd.ptr(0x18).filter(|&o| o != dr && looks_like(o)) {
                cockpit = list_of(cd, 0);
                let (sg0, sg1) = (rd.ptr(dr + 0x08), rd.ptr(cd + 0x08));
                if sg1 != sg0 {
                    if let Some(Ok(g)) = sg1.map(|sg| ShaderGroup::parse(&rd, sg)) {
                        let base = shaders.len() as u16;
                        shader_tex.extend(g.shaders.iter().map(|s| s.diffuse().unwrap_or("").to_string()));
                        shader_names.extend(g.shaders.iter().map(|s| s.name.clone()));
                        shaders.extend(g.shaders);
                        for &mi in &cockpit {
                            models[mi].iter_mut().for_each(|(_, si)| *si += base);
                        }
                    }
                }
            }
        }
        if lods[0].is_empty() {
            lods[0] = (0..offs.len()).filter(|i| !cockpit.contains(i)).collect();
        }
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
        Ok(Character { name, bones, models, lods, cockpit, shader_tex, shader_names, shaders, textures, frame_layout })
    }

    pub fn bone(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    }
}
