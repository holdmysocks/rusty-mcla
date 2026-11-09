//! Vehicle skeleton (found in one of the car's `.r63` files, the one that also holds
//! door/hood animations). Layout: root+0x24 → {…, +0x10 ptr → matrices, +0x14 u16 count}.
//! The matrix array holds `count` local matrices (4 rows of vec4, row 3 = translation,
//! big-endian; the w column is padding). lod+0x04 -> parent index per bone; world
//! matrices are accumulated along the parent chain. lod+0x20 -> (tag, index) table.

use super::Rd;
use crate::rsc::Resource;
use crate::Result;
use glam::{Mat4, Vec3, Vec4};

#[derive(Debug, Clone)]
pub struct Bone {
    pub local: Mat4,
    pub world: Mat4,
    pub name: String,
    /// global 16-bit bone id (same across all cars)
    pub tag: u16,
    /// parent bone index (usize::MAX for roots)
    pub parent: usize,
    /// the bone record's translation limits (crBoneData +144 / +160; lod+0x00 -> 224-byte
    /// records): a bike's tyre and rim are twice as wide as the distance between them (F-74884)
    pub trans_limits: (Vec3, Vec3),
    /// the bone record's +26 / +27 bytes: the frame made from the skeleton (F-66930) holds a translation DOF (kind 0, a
    /// Vector3) and a rotation DOF (kind 1, a Quaternion) for the bone; `dof_id` the id both carry (the record's +22 when
    /// the skeleton's +28 has bit 1, else the bone's index)
    pub dof_trans: bool,
    pub dof_rot: bool,
    pub dof_id: u16,
}

#[derive(Debug, Clone, Default)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

impl Skeleton {
    /// The layout signature of the frame made from the skeleton (F-66930 through the skeleton component F-21688, no mover
    /// and no filter): its DOFs sorted by kind << 16 | id (F-39725), summed by F-59824 (`layout_signature`). A clip whose
    /// +16 equals it is sampled on the same-layout path.
    pub fn frame_layout(&self) -> u32 {
        let mut d: Vec<(u8, u8, u16)> = Vec::new();
        for b in &self.bones {
            if b.dof_trans {
                d.push((0, 0, b.dof_id));
            }
            if b.dof_rot {
                d.push((1, 1, b.dof_id));
            }
        }
        d.sort_by_key(|&(_, k, id)| ((k as u32) << 16) | id as u32);
        layout_signature(&d)
    }
}

/// F-59824, a frame layout's signature: over the DOFs (type, kind, id) in the frame's order the two 32-bit sums of
/// Fletcher's checksum (both start 0xFFFF) take per DOF the word kind << 8 | 0x80 | type and then the id, folded to 16 bits
/// every 180 DOFs; the result is sum 2's fold in the high half (added twice over: (s2 << 16) + (s2 & 0xFFFF0000)) or'd
/// with sum 1's fold. The clips' own +16 is the same over their tracks.
pub fn layout_signature(dofs: &[(u8, u8, u16)]) -> u32 {
    if dofs.is_empty() {
        return 0;
    }
    let (mut s1, mut s2) = (0xFFFFu32, 0xFFFFu32);
    for chunk in dofs.chunks(180) {
        for &(t, k, id) in chunk {
            let w = ((t as u32 & 0xF) | 0x80) | ((k as u32) << 8);
            s1 = s1.wrapping_add(w);
            s2 = s2.wrapping_add(s1);
            s1 = s1.wrapping_add(id as u32);
            s2 = s2.wrapping_add(s1);
        }
        s1 = (s1 >> 16) + (s1 & 0xFFFF);
        s2 = (s2 >> 16) + (s2 & 0xFFFF);
    }
    (s2 << 16).wrapping_add(s2 & 0xFFFF_0000) | ((s1 >> 16) + (s1 & 0xFFFF))
}

fn read_mat(rd: &Rd, off: usize) -> Mat4 {
    let row = |k: usize| Vec4::new(rd.f32(off + k * 16), rd.f32(off + k * 16 + 4), rd.f32(off + k * 16 + 8), 0.0);
    let r0 = row(0);
    let r1 = row(1);
    let r2 = row(2);
    let t = row(3);
    // rows are basis vectors (x axis, y axis, z axis) + translation
    Mat4::from_cols(Vec4::new(r0.x, r0.y, r0.z, 0.0), Vec4::new(r1.x, r1.y, r1.z, 0.0), Vec4::new(r2.x, r2.y, r2.z, 0.0), Vec4::new(t.x, t.y, t.z, 1.0))
}

impl Skeleton {
    /// Try to parse a skeleton out of a part/fragment resource; returns None if the
    /// resource has no bone matrices.
    pub fn parse(res: &Resource) -> Result<Option<Skeleton>> {
        let rd = Rd::new(res);
        let Some(lod) = rd.ptr(0x24) else { return Ok(None) };
        let Some(mats) = rd.ptr(lod + 0x10) else { return Ok(None) };
        let count = rd.u16(lod + 0x14) as usize;
        if count == 0 || count > 512 || !rd.has(mats, count * 2 * 64) {
            return Ok(None);
        }
        // bone names (optional): root+0x14 → array of string objects
        let mut names = vec![String::new(); count];
        if let Some(nl) = rd.ptr(0x14) {
            for i in 0..count {
                if let Some(so) = rd.ptr(nl + i * 4) {
                    if let Some(sp) = rd.ptr(so) {
                        let s = rd.cstr(sp);
                        if s.len() < 48 && s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
                            names[i] = s;
                        }
                    }
                }
            }
        }
        // tag table: lod+0x20 → count pairs of (u16 tag, u16 bone index)
        let mut tags = vec![0u16; count];
        if let Some(tt) = rd.ptr(lod + 0x20) {
            for i in 0..count {
                let tag = rd.u16(tt + i * 4);
                let idx = rd.u16(tt + i * 4 + 2) as usize;
                if idx < count {
                    tags[idx] = tag;
                }
            }
        }
        // parent indices: lod+0x04 -> u32 per bone (root points at itself)
        let mut parents = vec![usize::MAX; count];
        if let Some(pa) = rd.ptr(lod + 0x04) {
            if rd.has(pa, count * 4) {
                for i in 0..count {
                    let p = rd.u32(pa + i * 4) as usize;
                    if p < count && p != i {
                        parents[i] = p;
                    }
                }
            }
        }
        let locals: Vec<Mat4> = (0..count).map(|i| read_mat(&rd, mats + i * 64)).collect();
        // bone records: lod+0x00 -> count x 224 bytes
        let vec3 = |o: usize| Vec3::new(rd.f32(o), rd.f32(o + 4), rd.f32(o + 8));
        let ids_on = rd.u32(lod + 28) & 2 != 0;
        let dofs: Vec<(bool, bool, u16)> = match rd.ptr(lod).filter(|&r| rd.has(r, count * 224)) {
            Some(r) => (0..count)
                .map(|i| (rd.u8(r + i * 224 + 26) != 0, rd.u8(r + i * 224 + 27) != 0, if ids_on { rd.u16(r + i * 224 + 22) } else { i as u16 }))
                .collect(),
            None => vec![(false, false, 0); count],
        };
        let limits: Vec<(Vec3, Vec3)> = match rd.ptr(lod).filter(|&r| rd.has(r, count * 224)) {
            Some(r) => (0..count).map(|i| (vec3(r + i * 224 + 144), vec3(r + i * 224 + 160))).collect(),
            None => vec![(Vec3::ZERO, Vec3::ZERO); count],
        };
        // world (bind) matrices are not stored; accumulate up the parent chain
        let mut world = vec![None::<Mat4>; count];
        fn resolve(i: usize, locals: &[Mat4], parents: &[usize], world: &mut [Option<Mat4>], depth: usize) -> Mat4 {
            if let Some(w) = world[i] {
                return w;
            }
            let w = if parents[i] == usize::MAX || depth > 64 {
                locals[i]
            } else {
                resolve(parents[i], locals, parents, world, depth + 1) * locals[i]
            };
            world[i] = Some(w);
            w
        }
        let mut bones = Vec::with_capacity(count);
        for i in 0..count {
            let w = resolve(i, &locals, &parents, &mut world, 0);
            bones.push(Bone { local: locals[i], world: w, name: names[i].clone(), tag: tags[i], parent: parents[i], trans_limits: limits[i],
                dof_trans: dofs[i].0, dof_rot: dofs[i].1, dof_id: dofs[i].2 });
        }
        Ok(Some(Skeleton { bones }))
    }

    pub fn bone_by_tag(&self, tag: u16) -> Option<usize> {
        self.bones.iter().position(|b| b.tag == tag && tag != 0)
    }

    pub fn translation(&self, i: usize) -> Vec3 {
        self.bones[i].world.w_axis.truncate()
    }

    /// Bones whose world translation satisfies `pred`.
    pub fn find(&self, pred: impl Fn(Vec3) -> bool) -> Vec<usize> {
        (0..self.bones.len()).filter(|&i| pred(self.translation(i))).collect()
    }

    /// Motorcycle hubs: the front-most and rear-most bone on the centre line at wheel
    /// height. Returns (front, rear).
    pub fn bike_hubs(&self) -> Option<[Vec3; 2]> {
        let c: Vec<Vec3> = self.find(|t| t.x.abs() < 0.06 && t.y > 0.2 && t.y < 0.45 && t.z.abs() > 0.45).into_iter().map(|i| self.translation(i)).collect();
        let front = c.iter().copied().filter(|p| p.z < 0.0).min_by(|a, b| a.z.partial_cmp(&b.z).unwrap())?;
        let rear = c.iter().copied().filter(|p| p.z > 0.0).max_by(|a, b| a.z.partial_cmp(&b.z).unwrap())?;
        Some([front, rear])
    }

    /// Hub bones: four bones with |x| > 0.4, 0.2 < y < 0.6 forming two axles at the same height.
    pub fn hubs(&self) -> Option<[Vec3; 4]> {
        let mut all: Vec<Vec3> = self
            .find(|t| t.x.abs() > 0.45 && t.y > 0.15 && t.y < 0.65 && t.z.abs() > 0.6)
            .into_iter()
            .map(|i| self.translation(i))
            .collect();
        all.sort_by(|a, b| a.z.partial_cmp(&b.z).unwrap().then(a.x.partial_cmp(&b.x).unwrap()));
        all.dedup_by(|a, b| a.distance(*b) < 0.01);
        // keep the height level that has the most candidates on both axles
        let mut best: Option<(usize, f32)> = None;
        for p in &all {
            let n = all.iter().filter(|q| (q.y - p.y).abs() < 0.04).count();
            let both = all.iter().any(|q| (q.y - p.y).abs() < 0.04 && q.z < 0.0) && all.iter().any(|q| (q.y - p.y).abs() < 0.04 && q.z > 0.0);
            if both && best.map_or(true, |b| n > b.0 || (n == b.0 && p.y < b.1)) {
                best = Some((n, p.y));
            }
        }
        let c: Vec<Vec3> = match best {
            Some((_, y)) => all.into_iter().filter(|q| (q.y - y).abs() < 0.04).collect(),
            None => all,
        };
        if c.len() < 4 {
            return None;
        }
        // front axle = most negative z
        let front: Vec<Vec3> = c.iter().copied().filter(|p| p.z < 0.0).collect();
        let rear: Vec<Vec3> = c.iter().copied().filter(|p| p.z > 0.0).collect();
        if front.len() < 2 || rear.len() < 2 {
            return None;
        }
        let pick = |v: &Vec<Vec3>, left: bool| {
            let mut best = v[0];
            for p in v {
                if (left && p.x < best.x) || (!left && p.x > best.x) {
                    best = *p;
                }
            }
            best
        };
        Some([pick(&front, true), pick(&front, false), pick(&rear, true), pick(&rear, false)])
    }
}

#[cfg(test)]
mod tests {
    /// The ZX-14's base fragment: wheel_0's translation limits are +-0.06396 on x, wheel_2's +-0.11 - the bike's
    /// front and rear widths (F-74884), recorded as the rims' x scale 0.25584 / 0.44000 (CAR_PARTS.md).
    #[test]
    fn zx14_wheel_limits() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/vehicle/vp_kaw_ninja_zx14_06/0xf9785b04.r63")) else { return };
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        let res = crate::rsc::Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data };
        let sk = super::Skeleton::parse(&res).unwrap().unwrap();
        let span = |n: &str| sk.bones.iter().find(|b| b.name == n).map(|b| (b.trans_limits.1 - b.trans_limits.0).length()).unwrap();
        assert!((span("wheel_0") - 0.12792).abs() < 1e-5, "{}", span("wheel_0"));
        assert!((span("wheel_2") - 0.22).abs() < 1e-5, "{}", span("wheel_2"));
        assert_eq!(span("hub_0"), 0.0);
    }
}
