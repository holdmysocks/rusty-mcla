//! What the build of a character's ragdoll (the thrown bike rider's phArticulatedBody) reads of the character's
//! fragment type, as F-62449 / F-16970 / F-91939 / F-55164 / F-49221 / F-83435 read it (decode:
//! docs/reference/vehicle/ragdoll_link_joint_build.md, ragdoll_setup_decode.md 2.2 - 2.3). Every number is kept
//! as the file has it, to the bit (lane w of the vectors too: the build carries it along).
//!
//! ```text
//! set root +20 -> fragType T
//! T: +8 least principal moment of a link (F-91939), +12 most, +32 / +48 the two centre vectors (off = +32 - +48),
//!    +64 the centre shift of a young cache entry (F-13409),
//!    +180 -> drawable (+12 -> skeleton: +0 -> bones of 224 B, u16 count +20), +208 -> groups, +212 -> children,
//!    +240 -> per-child principal moments (16 B each), +248 the pointer the collider keeps at +748,
//!    +256 / +260 -> the two byte tables of part pairs (F-83435), +280 -> pose (+20 -> 64 B per bone),
//!    +300 pair count, +302 groups, +303 children, +316 float (constraints when != 0), +320 the contacts' elasticity,
//!    +0xec -> composite bound (one part per child: capsules and boxes for the rider; +0x50 its centre-of-gravity
//!    offset, a part's +0x50 its own)
//! group G: +20 softness k, +24 / +28 / +32 / +36 limit fractions, +40 target rate, +44 velocity gain,
//!    +48 position gain, +52 most torque, +56 -1 or > 0: the joint is parked, +60 mass,
//!    +69 parent group (255 none), +70 first child, +71 children
//! child: +4 mass, +12 group, +14 bone (i16), +80 / +96 / +112 rotation rows, +128 centre (model space)
//! bone (224 B): +0 -> name, +4 flags, +144 / +160 translation min / max, +176 / +192 rotation min / max (x y z)
//! ```

use super::Rd;
use crate::rsc::Resource;

/// A group record of the type (T+208[g]).
#[derive(Debug, Clone, Copy, Default)]
pub struct RagGroup {
    /// +20: softness k of the joint (F-49221 clamps it to [min, 1])
    pub k: f32,
    /// +24 / +28 / +32 / +36: limit fractions (a hinge reads +24 / +28, a swing-twist joint +28 / +32 / +36)
    pub frac: [f32; 4],
    /// +40 target rate
    pub target_rate: f32,
    /// +44 velocity gain
    pub vel_gain: f32,
    /// +48 position gain
    pub pos_gain: f32,
    /// +52 most torque
    pub max_torque: f32,
    /// +56: -1 or > 0 parks the joint (entry+424) instead of adding it to the body
    pub park: f32,
    /// +60
    pub mass: f32,
    /// +69 (255: none)
    pub parent: u8,
    /// +70
    pub first_child: u8,
    /// +71
    pub children: u8,
}

/// A child of the type (T+212[i]).
#[derive(Debug, Clone, Copy, Default)]
pub struct RagChild {
    /// +4
    pub mass: f32,
    /// +12
    pub group: u8,
    /// +14: its bone (<= 0: none)
    pub bone: i16,
    /// +80 / +96 / +112: the rotation rows (4 lanes each)
    pub rot: [[f32; 4]; 3],
    /// +128: its centre in model space (4 lanes)
    pub pos: [f32; 4],
    /// +16: the 4 x 4 matrix H of the pose exchange (F-93058)
    pub h: [[f32; 4]; 4],
    /// [+144] + 128: C, the part's place in its bone's frame (F-36901, F-46606; the bind of the bone)
    pub c: [[f32; 4]; 4],
    /// [T+248] + 64 i: O, the part's fixed place in its link's frame (F-43257)
    pub o: [[f32; 4]; 4],
}

/// A bone record of the skeleton (224 B).
#[derive(Debug, Clone, Default)]
pub struct RagBone {
    pub name: String,
    /// +4: DOF flags (F-41462: rot x 0x10 & 0x2, y 0x20 & 0x4, z 0x40 & 0x8; trans x 0x400 & 0x80, y 0x800 & 0x100,
    /// z 0x1000 & 0x200)
    pub flags: u32,
    /// +144 / +160
    pub trans_min: [f32; 3],
    pub trans_max: [f32; 3],
    /// +176 / +192 (radians)
    pub rot_min: [f32; 3],
    pub rot_max: [f32; 3],
}

impl RagBone {
    /// F-41462: the free DOFs (rx, ry, rz, tx, ty, tz). An axis is free when both its flag bits are set and its
    /// limits differ.
    pub fn dofs(&self) -> [bool; 6] {
        let f = |a: u32, b: u32, lo: f32, hi: f32| self.flags & a != 0 && self.flags & b != 0 && lo != hi;
        [
            f(0x10, 0x2, self.rot_min[0], self.rot_max[0]),
            f(0x20, 0x4, self.rot_min[1], self.rot_max[1]),
            f(0x40, 0x8, self.rot_min[2], self.rot_max[2]),
            f(0x400, 0x80, self.trans_min[0], self.trans_max[0]),
            f(0x800, 0x100, self.trans_min[1], self.trans_max[1]),
            f(0x1000, 0x200, self.trans_min[2], self.trans_max[2]),
        ]
    }
}

/// One part of the type's composite bound (T+0xec, type 12: +0x80 -> part bounds, +0x84 -> part matrices, u16 +0x92
/// count), what the narrow phase reads of it (docs: work/ragdoll/contact/rider_contacts.md). Every number as the file
/// has it.
#[derive(Debug, Clone, Default)]
pub struct RagPart {
    /// bound +4: 0 sphere, 1 capsule, 3 box, 4 polyhedron
    pub kind: u8,
    /// +8: the bounding sphere's radius
    pub radius: f32,
    /// +0x10 / +0x20: the part's box (margin included)
    pub max: [f32; 3],
    pub min: [f32; 3],
    /// +0x30
    pub centroid: [f32; 3],
    /// +0x70
    pub margin: f32,
    /// a sphere's or capsule's +0x80 (its radius) and a capsule's +0x90 (the length between the two centres)
    pub round: f32,
    pub length: f32,
    /// material id (low byte: the game's material table, 0 default): a sphere's +0x90, a capsule's +0xd0; None for
    /// a box or polyhedron (its id is not read here)
    pub material: Option<u32>,
    /// the file's part matrix (+0x84, 64 B; the run time rewrites its copy every tick)
    pub matrix: [[f32; 4]; 4],
    /// +0x50: the part's centre-of-gravity offset (4 lanes; F-68804 adds it to the part matrix's position)
    pub cg_offset: [f32; 4],
    /// the composite's part pointer is set (F-68804 skips a missing part)
    pub present: bool,
}

/// The fragment type of a character set, as the ragdoll build reads it.
#[derive(Debug, Clone, Default)]
pub struct RagdollType {
    /// offset of the type in the resource
    pub off: usize,
    /// +8: a link's smallest principal moment below this scales its mass and moments (F-91939)
    pub min_moment: f32,
    /// +12: bound on the largest scaled moment
    pub max_moment: f32,
    /// +32 / +48 (4 lanes): the build moves every centre by -(+32 - +48)
    pub cg: [f32; 4],
    pub cg_ref: [f32; 4],
    pub groups: Vec<RagGroup>,
    pub children: Vec<RagChild>,
    /// +240: principal moments per child (4 lanes, lane w as stored)
    pub inertia: Vec<[f32; 4]>,
    pub bones: Vec<RagBone>,
    /// skeleton data +4: the parent bone of each bone (bone 0's entry unused)
    pub bone_parent: Vec<i32>,
    /// skeleton data +16: the default pose, 64 B per bone (RemoveRagdoll's snap, F-98260; bone 0's is the entry's
    /// E+288, F-46606)
    pub bone_default: Vec<[[f32; 4]; 4]>,
    /// [[+280]+20]: the skeleton's pose, 64 B per bone (rows and position; the joint anchors start from row 3)
    pub bone_pose: Vec<[[f32; 4]; 4]>,
    /// +256 / +260 (+300 entries): the group pairs F-83435 turns into child pairs
    pub pairs: Vec<(u8, u8)>,
    /// +248 as stored (a resource pointer; the collider holds its loaded address at +748)
    pub ptr248: u32,
    /// +316
    pub f316: f32,
    /// +307
    pub flags: u8,
    /// +320: the elasticity the instance's contact callback writes into its contacts (F-22759)
    pub f320: f32,
    /// +0xec: the composite bound's parts, one per child (empty when the bound is missing or not a composite)
    pub parts: Vec<RagPart>,
    /// the composite bound's +0x50: its centre-of-gravity offset (4 lanes), what F-13409 sets the cache entry's
    /// bound to on an articulated type (F-83058)
    pub bound_cg: [f32; 4],
    /// +64 (4 lanes): F-13409 adds it to the bound's centre-of-gravity offset while the entry is young (< 0.25)
    pub cg_shift: [f32; 4],
}

fn v4(rd: &Rd, o: usize) -> [f32; 4] {
    [rd.f32(o), rd.f32(o + 4), rd.f32(o + 8), rd.f32(o + 12)]
}

fn m4(rd: &Rd, o: usize) -> [[f32; 4]; 4] {
    [v4(rd, o), v4(rd, o + 16), v4(rd, o + 32), v4(rd, o + 48)]
}

fn v3(rd: &Rd, o: usize) -> [f32; 3] {
    [rd.f32(o), rd.f32(o + 4), rd.f32(o + 8)]
}

/// The fragment type at `t` (an offset into the resource).
pub fn parse_ragdoll_type(res: &Resource, t: usize) -> Option<RagdollType> {
    let rd = Rd::new(res);
    if !rd.has(t, 320) {
        return None;
    }
    let (ng, nc, np) = (rd.u8(t + 302) as usize, rd.u8(t + 303) as usize, rd.u8(t + 300) as usize);
    let mut out = RagdollType {
        off: t,
        min_moment: rd.f32(t + 8),
        max_moment: rd.f32(t + 12),
        cg: v4(&rd, t + 32),
        cg_ref: v4(&rd, t + 48),
        cg_shift: v4(&rd, t + 64),
        ptr248: rd.u32(t + 248),
        f316: rd.f32(t + 316),
        flags: rd.u8(t + 307),
        ..Default::default()
    };
    let groups = rd.ptr(t + 208)?;
    for g in 0..ng {
        let o = rd.ptr(groups + 4 * g)?;
        out.groups.push(RagGroup {
            k: rd.f32(o + 20),
            frac: v4(&rd, o + 24),
            target_rate: rd.f32(o + 40),
            vel_gain: rd.f32(o + 44),
            pos_gain: rd.f32(o + 48),
            max_torque: rd.f32(o + 52),
            park: rd.f32(o + 56),
            mass: rd.f32(o + 60),
            parent: rd.u8(o + 69),
            first_child: rd.u8(o + 70),
            children: rd.u8(o + 71),
        });
    }
    let children = rd.ptr(t + 212)?;
    let inertia = rd.ptr(t + 240)?;
    for i in 0..nc {
        let o = rd.ptr(children + 4 * i)?;
        out.children.push(RagChild {
            mass: rd.f32(o + 4),
            group: rd.u8(o + 12),
            bone: rd.u16(o + 14) as i16,
            rot: [v4(&rd, o + 80), v4(&rd, o + 96), v4(&rd, o + 112)],
            pos: v4(&rd, o + 128),
            h: m4(&rd, o + 16),
            c: rd.ptr(o + 144).map_or([[0.0; 4]; 4], |c| m4(&rd, c + 128)),
            o: rd.ptr(t + 248).map_or([[0.0; 4]; 4], |p| m4(&rd, p + 64 * i)),
        });
        out.inertia.push(v4(&rd, inertia + 16 * i));
    }
    if let Some(skel) = rd.ptr(t + 180).and_then(|d| rd.ptr(d + 12)) {
        if let Some(pa) = rd.ptr(skel + 4) {
            let n = rd.u16(skel + 20) as usize;
            if rd.has(pa, 4 * n) {
                out.bone_parent = (0..n).map(|k| rd.u32(pa + 4 * k) as i32).collect();
            }
        }
        if let Some(dm) = rd.ptr(skel + 16) {
            let n = rd.u16(skel + 20) as usize;
            if rd.has(dm, 64 * n) {
                out.bone_default = (0..n).map(|k| m4(&rd, dm + 64 * k)).collect();
            }
        }
        if let Some(bones) = rd.ptr(skel) {
            let n = rd.u16(skel + 20) as usize;
            for k in 0..n {
                let o = bones + 224 * k;
                if !rd.has(o, 224) {
                    break;
                }
                out.bones.push(RagBone {
                    name: rd.cstr_ptr(o).unwrap_or_default(),
                    flags: rd.u32(o + 4),
                    trans_min: v3(&rd, o + 144),
                    trans_max: v3(&rd, o + 160),
                    rot_min: v3(&rd, o + 176),
                    rot_max: v3(&rd, o + 192),
                });
            }
        }
    }
    if let Some(pose) = rd.ptr(t + 280).and_then(|p| rd.ptr(p + 20)) {
        for k in 0..out.bones.len() {
            let o = pose + 64 * k;
            if !rd.has(o, 64) {
                break;
            }
            out.bone_pose.push([v4(&rd, o), v4(&rd, o + 16), v4(&rd, o + 32), v4(&rd, o + 48)]);
        }
    }
    if let (Some(a), Some(b)) = (rd.ptr(t + 256), rd.ptr(t + 260)) {
        for p in 0..np {
            out.pairs.push((rd.u8(a + p), rd.u8(b + p)));
        }
    }
    out.f320 = rd.f32(t + 320);
    out.parts = composite_parts(&rd, t);
    if let Some(b) = rd.ptr(t + 0xec).filter(|&b| rd.has(b, 0x60)) {
        out.bound_cg = v4(&rd, b + 0x50);
    }
    Some(out)
}

/// The composite bound's parts (T+0xec).
fn composite_parts(rd: &Rd, t: usize) -> Vec<RagPart> {
    let mut out = Vec::new();
    let Some(b) = rd.ptr(t + 0xec) else { return out };
    if !rd.has(b, 0x94) || rd.u8(b + 4) != 12 {
        return out;
    }
    let (Some(arr), Some(mats)) = (rd.ptr(b + 0x80), rd.ptr(b + 0x84)) else { return out };
    let n = rd.u16(b + 0x92) as usize;
    for k in 0..n {
        let Some(p) = rd.ptr(arr + 4 * k) else {
            out.push(RagPart::default());
            continue;
        };
        if !rd.has(p, 0xd4) {
            out.push(RagPart::default());
            continue;
        }
        let kind = rd.u8(p + 4);
        out.push(RagPart {
            kind,
            radius: rd.f32(p + 8),
            max: v3(rd, p + 0x10),
            min: v3(rd, p + 0x20),
            centroid: v3(rd, p + 0x30),
            margin: rd.f32(p + 0x70),
            round: if kind <= 1 { rd.f32(p + 0x80) } else { 0.0 },
            length: if kind == 1 { rd.f32(p + 0x90) } else { 0.0 },
            material: match kind {
                0 => Some(rd.u32(p + 0x90)),
                1 => Some(rd.u32(p + 0xd0)),
                _ => None,
            },
            matrix: if rd.has(mats + 64 * k, 64) { m4(rd, mats + 64 * k) } else { [[0.0; 4]; 4] },
            cg_offset: v4(rd, p + 0x50),
            present: true,
        });
    }
    out
}

/// The fragment type of a character set (`.r6`): root+20.
pub fn parse_character_ragdoll(res: &Resource) -> Option<RagdollType> {
    let t = Rd::new(res).ptr(20)?;
    parse_ragdoll_type(res, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rider() -> Option<Resource> {
        let root = std::env::var_os("MCLA_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("D:/MCLA/extracted/cache"));
        let d = std::fs::read(root.join("resources/character/drv_mp_01_mot_set/0x5eb8d142.r6")).ok()?;
        if d.len() < 16 || &d[0..4] != b"MCLR" {
            return None;
        }
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        Some(Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data })
    }

    /// The rider's table (ragdoll_setup_decode.md 2.3): 21 children = 21 groups, 12 pairs, masses 115.76 kg in all,
    /// hinges on the elbows / knees / ankles, hip_r without a DOF.
    #[test]
    fn rider_ragdoll_table() {
        let Some(res) = rider() else { return };
        let t = parse_character_ragdoll(&res).unwrap();
        assert_eq!((t.groups.len(), t.children.len(), t.pairs.len(), t.bones.len(), t.bone_pose.len()), (21, 21, 12, 98, 98));
        assert_eq!(t.min_moment.to_bits(), 0x3a83126f);
        assert_eq!(t.max_moment, 1.0);
        let bones: Vec<i16> = t.children.iter().map(|c| c.bone).collect();
        assert_eq!(bones, [1, 2, 3, 4, 5, 6, 43, 44, 46, 48, 64, 65, 67, 69, 85, 86, 88, 89, 92, 94, 95]);
        let masses: Vec<f32> = t.children.iter().map(|c| c.mass).collect();
        assert_eq!(masses, [11.0, 5.76, 11.0, 5.0, 1.0, 8.0, 6.0, 5.0, 4.0, 2.0, 6.0, 5.0, 4.0, 2.0, 5.0, 9.0, 7.0, 4.0, 4.0, 7.0, 4.0]);
        assert_eq!((t.groups[0].parent, t.groups[1].parent, t.groups[2].parent), (255, 255, 0));
        assert_eq!(t.bones[1].name, "spine00");
        let ndof = |b: i16| t.bones[b as usize].dofs()[..3].iter().filter(|&&d| d).count();
        assert_eq!((ndof(46), ndof(88), ndof(89), ndof(92), ndof(3), ndof(48)), (1, 1, 1, 0, 0, 2));
        assert!(t.bones.iter().all(|b| !b.dofs()[3..].iter().any(|&d| d)));
        assert_eq!(t.pairs, [(10, 4), (10, 2), (10, 1), (13, 4), (13, 2), (13, 1), (17, 19), (15, 16), (10, 8), (13, 8), (2, 9), (2, 12)]);
        assert_eq!(t.f316, 0.0);
    }

    /// The rider's composite bound (work/ragdoll/contact/rider_contacts.md, the bound table): 17 capsules and four
    /// boxes (wrists, feet), the capsules of the "ragdoll" material (id 31), margins 0.04 / 0.01 / 0.015.
    #[test]
    fn rider_bound_table() {
        let Some(res) = rider() else {
            eprintln!("skipped: the rider's .r6 is not in the extracted data");
            return;
        };
        let t = parse_character_ragdoll(&res).unwrap();
        assert_eq!(t.parts.len(), 21);
        let kinds: Vec<u8> = t.parts.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [1, 1, 1, 1, 1, 1, 1, 1, 1, 3, 1, 1, 1, 3, 1, 1, 1, 3, 1, 1, 3]);
        assert_eq!(t.f320, 0.0);
        for p in &t.parts {
            match p.kind {
                1 => {
                    assert_eq!(p.material, Some(31));
                    assert_eq!(p.margin.to_bits(), 0x3d23_d70a);
                    // (the box: the radius across, half the length plus the radius along y)
                    assert_eq!(p.max[0], p.round);
                    assert!((p.max[1] - (p.round + 0.5 * p.length)).abs() < 1e-6);
                }
                _ => assert_eq!(p.material, None),
            }
        }
        // spine00 (0.11787 / 0.06739), the thighs (0.07 / 0.4), a wrist box 0.04 x 0.15 x 0.08 with margin 0.01
        assert_eq!((t.parts[0].round.to_bits(), t.parts[0].length.to_bits()), (0x3df1_66e0, 0x3d8a_0557));
        assert_eq!((t.parts[15].round, t.parts[15].length), (0.07, 0.4));
        assert_eq!((t.parts[9].max, t.parts[9].min, t.parts[9].margin), ([0.02, 0.075, 0.04], [-0.02, -0.075, -0.04], 0.01));
        assert_eq!((t.parts[17].max, t.parts[17].margin), ([0.05, 0.03, 0.135], 0.015));
        // (the file's part matrix of spine00: the capsule's axis along the model's x)
        assert_eq!(t.parts[0].matrix[1][..3], [-1.0, 0.0, 0.0]);
        // the centre-of-gravity offsets the archetype's inertia is taken about (F-68804): the bound's (0, 0.0039,
        // -0.029), the parts' none, no shift
        assert_eq!(t.bound_cg[..3].iter().map(|x| x.to_bits()).collect::<Vec<_>>(), [0, 0x3b7f_9724, 0xbced_9168]);
        assert!(t.parts.iter().all(|p| p.present && p.cg_offset[..3] == [0.0; 3]));
        assert_eq!(t.cg_shift[..3], [0.0; 3]);
    }
}
