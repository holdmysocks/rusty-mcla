//! Facial blend shapes: a character set's `.r3` (`#btm`), a `rage::grbTargetManager`
//! (Place `sub_826142B8`). Rules in `docs/reference/cutscenes/blend_shapes.md`.
//!
//! Manager: `+0c` atArray of mesh records, `+14` hash table (bucket pointers, u16 bucket count at
//! `+18`; node 16 bytes = u16 key, pointer to the grbTarget, next; bucket = key % count,
//! `sub_82613F68`). grbTarget (0x50): `+04` name, `+44` u32 slot, `+48` atArray of the blend
//! geometries it moves. Mesh record: `+00` name ("mesh_head.mesh"), `+80` u32 LOD, `+84` u32 the
//! model's place in that LOD's model list of the drawable, `+88` atArray of blend geometries, one per
//! geometry of the model. Blend geometry: `+44` atArray of target blobs by slot (null where a target
//! leaves the geometry alone), `+50` three delta vertex buffers (float4 per vertex, zero in the file),
//! `+68` atArray of the GPU-path records (count 0 in every file: the CPU path). Target blob: `+00` vertex
//! count, `+04` u8 1, `+05` u8 normals present (0 in every file), `+08` position deltas
//! (4 x s16 per touched vertex, / 4096), `+10` normal deltas, `+18` tangent deltas (4 x s8, / 128),
//! `+20` u32 vertex indices, `+24` u16 count.

use super::Rd;
use crate::rsc::Resource;
use crate::{bail, Result};

/// One target's deltas on one geometry.
#[derive(Debug, Clone, Default)]
pub struct TargetBlob {
    pub indices: Vec<u32>,
    /// raw s16 x 4 (x, y, z, w); the kernel scales by 1 / 4096
    pub deltas: Vec<[i16; 4]>,
}

#[derive(Debug, Clone, Default)]
pub struct BlendGeometry {
    pub vertex_count: usize,
    /// by target slot
    pub blobs: Vec<Option<TargetBlob>>,
}

#[derive(Debug, Clone, Default)]
pub struct BlendMesh {
    pub name: String,
    pub lod: u32,
    /// index into the drawable's model list of `lod`
    pub model: u32,
    pub geoms: Vec<BlendGeometry>,
}

#[derive(Debug, Clone)]
pub struct Target {
    /// the key of the hash table = the bone id of the facial pack's kind-21 track
    pub id: u16,
    pub name: String,
    /// slot in every geometry's blob array
    pub slot: usize,
    /// (mesh, geometry) pairs the target moves
    pub geoms: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Default)]
pub struct BlendShapes {
    pub meshes: Vec<BlendMesh>,
    /// sorted by id (the creature component's order, `sub_8253F590`)
    pub targets: Vec<Target>,
}

/// Position delta of a raw value as the kernel `sub_82615F78` makes it (`vcsxwfp ..,12`).
#[inline]
pub fn delta_scale(v: i16) -> f32 {
    v as f32 * (1.0 / 4096.0)
}

impl TargetBlob {
    /// The kernel `sub_82615F78`: every touched vertex's float4 += (s16 x 4 / 4096) x weight (vmaddfp:
    /// a product, then a sum, in single precision).
    pub fn apply(&self, buf: &mut [[f32; 4]], w: f32) {
        for (k, &vi) in self.indices.iter().enumerate() {
            let v = &mut buf[vi as usize];
            for c in 0..4 {
                v[c] += delta_scale(self.deltas[k][c]) * w;
            }
        }
    }
}

impl BlendShapes {
    pub fn parse(res: &Resource) -> Result<BlendShapes> {
        let rd = Rd::new(res);
        let arr = |o: usize| -> Vec<Option<usize>> {
            let n = rd.u16(o + 4) as usize;
            match rd.ptr(o) {
                Some(p) => (0..n).map(|i| rd.ptr(p + i * 4)).collect(),
                None => Vec::new(),
            }
        };
        let mut out = BlendShapes::default();
        let mut geom_at: Vec<(usize, usize, usize)> = Vec::new();
        for m in arr(0x0c).into_iter().flatten() {
            let mi = out.meshes.len();
            let mut mesh = BlendMesh { name: rd.cstr(m), lod: rd.u32(m + 0x80), model: rd.u32(m + 0x84), geoms: Vec::new() };
            for g in arr(m + 0x88).into_iter().flatten() {
                if rd.u16(g + 0x6c) != 0 {
                    bail!("blend geometry {:#x}: GPU-path records", g);
                }
                let mut geom = BlendGeometry::default();
                for b in arr(g + 0x44) {
                    let Some(b) = b else {
                        geom.blobs.push(None);
                        continue;
                    };
                    if rd.u8(b + 5) != 0 {
                        bail!("target blob {:#x} has normal deltas", b);
                    }
                    geom.vertex_count = rd.u32(b) as usize;
                    let n = rd.u16(b + 0x24) as usize;
                    let (Some(dp), Some(ip)) = (rd.ptr(b + 8), rd.ptr(b + 0x20)) else { bail!("target blob {:#x} without data", b) };
                    let indices: Vec<u32> = (0..n).map(|i| rd.u32(ip + i * 4)).collect();
                    let deltas = (0..n).map(|i| std::array::from_fn(|k| rd.u16(dp + i * 8 + k * 2) as i16)).collect();
                    if indices.iter().any(|&i| i as usize >= geom.vertex_count) {
                        bail!("target blob {:#x}: vertex index out of range", b);
                    }
                    geom.blobs.push(Some(TargetBlob { indices, deltas }));
                }
                geom_at.push((g, mi, mesh.geoms.len()));
                mesh.geoms.push(geom);
            }
            out.meshes.push(mesh);
        }
        let buckets = rd.u16(0x18) as usize;
        if let Some(ht) = rd.ptr(0x14) {
            for i in 0..buckets {
                let mut node = rd.ptr(ht + i * 4);
                while let Some(n) = node {
                    if let Some(t) = rd.ptr(n + 4) {
                        let geoms = arr(t + 0x48).into_iter().flatten().filter_map(|g| geom_at.iter().find(|x| x.0 == g).map(|x| (x.1, x.2))).collect();
                        out.targets.push(Target { id: rd.u16(n), name: rd.cstr(t + 4), slot: rd.u32(t + 0x44) as usize, geoms });
                    }
                    node = rd.ptr(n + 8);
                }
            }
        }
        out.targets.sort_by_key(|t| t.id);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(path: &std::path::Path) -> Option<Resource> {
        let d = std::fs::read(path).ok()?;
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        Some(Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data })
    }

    fn root() -> std::path::PathBuf {
        std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()).into()
    }

    /// The player's set: one mesh (model 6 of LOD 0), one geometry of 2320 vertices, 100 targets of which
    /// 94 move it (17007 vertex entries), keys as the hash table holds them.
    #[test]
    fn drv_mp_01_targets() {
        let Some(res) = load(&root().join("resources/character/drv_mp_01_set/0x29016322.r3")) else { return };
        let bs = BlendShapes::parse(&res).unwrap();
        assert_eq!(bs.meshes.len(), 1);
        let m = &bs.meshes[0];
        assert_eq!((m.name.as_str(), m.lod, m.model, m.geoms.len()), ("mesh_head.mesh", 0, 6, 1));
        let g = &m.geoms[0];
        assert_eq!((g.vertex_count, g.blobs.len(), g.blobs.iter().flatten().count()), (2320, 100, 94));
        assert_eq!(g.blobs.iter().flatten().map(|b| b.indices.len()).sum::<usize>(), 17007);
        assert_eq!(bs.targets.len(), 100);
        assert!(bs.targets.windows(2).all(|w| w[0].id < w[1].id));
        let t = bs.targets.iter().find(|t| t.id == 36167).unwrap();
        assert_eq!((t.name.as_str(), t.slot, t.geoms.as_slice()), ("headGEO_IMBS_left_brow_up", 81, &[(0usize, 0usize)][..]));
        let mut slots: Vec<usize> = bs.targets.iter().map(|t| t.slot).collect();
        slots.sort();
        assert_eq!(slots, (0..100).collect::<Vec<_>>());
    }

    /// The port's kernel against the original `sub_82615F78` run on the same targets and weights
    /// (work/blendshapes/bs_reference.py: 40 applications into one buffer): bit for bit.
    #[test]
    fn kernel_matches_original() {
        let dir: std::path::PathBuf = std::env::var("MCLA_BS_REFERENCE").unwrap_or_else(|_| "D:/MCLA/work/blendshapes/ref".into()).into();
        let (Ok(cases), Ok(reference)) = (std::fs::read_to_string(dir.join("cases.txt")), std::fs::read_to_string(dir.join("bs_reference.txt"))) else { return };
        let Some(res) = load(&root().join("resources/character/drv_mp_01_set/0x29016322.r3")) else { return };
        let bs = BlendShapes::parse(&res).unwrap();
        let g = &bs.meshes[0].geoms[0];
        let mut buf = vec![[0.0f32; 4]; g.vertex_count];
        for l in cases.lines().filter(|l| !l.trim().is_empty()) {
            let (k, w) = l.split_once(' ').unwrap();
            let w = f32::from_bits(u32::from_str_radix(w.trim(), 16).unwrap());
            g.blobs[k.parse::<usize>().unwrap()].as_ref().unwrap().apply(&mut buf, w);
        }
        let mut moved = 0;
        for (i, l) in reference.lines().enumerate() {
            let want: Vec<u32> = l.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let got: Vec<u32> = buf[i].iter().map(|v| v.to_bits()).collect();
            assert_eq!(got, want, "vertex {}", i);
            moved += (want.iter().any(|&x| x != 0)) as usize;
        }
        assert_eq!((reference.lines().count(), moved), (2320, 1484));
    }

    /// The facial pack of the intro: the player's entry carries one kind-21 track per target, keyed by the
    /// targets' ids, each a single float channel.
    #[test]
    fn intro_face_tracks() {
        let Some(res) = load(&root().join("resources/character/drv_mp_01_set/0x29016322.r3")) else { return };
        let bs = BlendShapes::parse(&res).unwrap();
        let Some(pack) = load(&root().join("resources/animation/cineanimpacks/0x3866f241.r1")) else { return };
        let rd = Rd::new(&pack);
        let name = "story/cut_intro_IM/Entity/player_drv00_name_drv_mp_01_set/player_drv00_name_drv_mp_01_set";
        let (o, _) = super::super::anim::pack_entries(&pack).into_iter().find(|(_, n)| n == name).unwrap();
        let a = super::super::anim::animation_named(&rd, o).unwrap();
        let k21: Vec<&super::super::anim::Track> = a.tracks.iter().filter(|t| t.kind == 21).collect();
        assert_eq!(k21.len(), 100);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for t in &bs.targets {
            let tr = k21.iter().find(|k| k.bone_id == t.id).unwrap();
            for f in 0..a.frames {
                let v = tr.data.vec(f as f32).x;
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        let moving = k21.iter().filter(|t| (0..a.frames).any(|f| t.data.vec(f as f32).x != t.data.vec(0.0).x)).count();
        // 30 tracks move, from frame 192 on (the player speaks in the scene's second half), e.g. left_blink1 (707) 0 .. 0.98
        assert_eq!((a.frames, moving), (361, 30));
        assert!(lo >= 0.0 && (hi - 0.98).abs() < 0.005);
    }

    /// All 16 sets with targets: the mesh record's (LOD, model) names a model of the drawable whose
    /// geometries have exactly the blend geometries' vertex counts; the 231 others are empty.
    #[test]
    fn all_sets_bind() {
        let Ok(dirs) = std::fs::read_dir(root().join("resources/character")) else { return };
        let (mut with, mut empty) = (0, 0);
        for d in dirs.flatten() {
            let files: Vec<std::path::PathBuf> = std::fs::read_dir(d.path()).unwrap().flatten().map(|e| e.path()).collect();
            let Some(r3) = files.iter().find(|p| p.extension().map_or(false, |e| e == "r3")) else { continue };
            let bs = BlendShapes::parse(&load(r3).unwrap()).unwrap();
            if bs.targets.is_empty() {
                empty += 1;
                continue;
            }
            with += 1;
            let r6 = files.iter().find(|p| p.extension().map_or(false, |e| e == "r6")).unwrap();
            let c = super::super::character::Character::parse(&load(r6).unwrap()).unwrap();
            for m in &bs.meshes {
                let mi = c.lods[m.lod as usize][m.model as usize];
                for (gi, g) in m.geoms.iter().enumerate() {
                    assert_eq!(c.models[mi][gi].0.positions.len(), g.vertex_count, "{}", r3.display());
                }
            }
            assert!(bs.targets.iter().all(|t| !t.geoms.is_empty()));
        }
        assert_eq!((with, empty), (16, 231));
    }
}
