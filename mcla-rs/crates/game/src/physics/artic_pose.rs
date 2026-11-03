//! The pose exchange between the thrown rider's ragdoll links and the rider's skeleton, as the original runs it.
//! Decode: docs/reference/vehicle/ragdoll_pose_sync.md (bike_rider_ragdoll.md 6c). Pure functions over plain data:
//! the link frames as [`super::artic::Articulated`] holds them ([`LinkFrame`]), the child -> link map of
//! [`super::artic_build::ArticBuild`] (collider +716), and the fragment type's child data, default pose and bone
//! parents of [`mcla_formats::rage::ragdoll::RagdollType`].
//!
//! Matrices are the guest's row-vector 4 x 4 (rows 0-2 the axes, row 3 the position); `mmul(a, b)` is "a, then b".
//! Every product is a rounded multiply then an add in the order of the vector code (no fused multiply-add), the
//! three-lane dot is the vector unit's ([`V4::dot3`]), all with the unit's denormal flush ([`V4`]).
//!
//! - Activation (GoRagdoll, F-81936): the eject clip's last frame under the character matrix, FK (F-97374); the
//!   part matrices of the fragment type (F-36901: `part_i = C_i x G[bone_i]` in rd+16's frame); each link's frame
//!   seeded from its first child (F-93058: `W = F_i x H_i x part_i x R`).
//! - Per physics tick (F-35284 -> F-43257): `part_i = O_i x link frame`, in rd+16's frame.
//! - Per frame (F-22847 -> F-75366 -> F-46606): the bone matrices of the drawn skeleton, world space: bone 0 =
//!   the default pose of bone 0 x R; a ragdoll child's bone = `inv(C_i) x part_i x R`; the others (face, fingers,
//!   rolls, toes) = their local x their parent's.
//! - RemoveRagdoll (F-98260): the hard snap to the default pose at the character matrix.
//!
//! Checked word for word against the original run offline (testdata/ragdoll_pose_ro0.bin, work/ragdoll/skin/
//! export_pose.py): activation 21 parts and 15 links, one tick 21 parts, one frame 34 bones; tests below.
//!
//! Open (decoded as absent for the rider, or not read):
//! - the part's alternative C ([child+148]+128, when the group's health is low) and the bone-0 point handed to the
//!   bound's centroid (F-36901's tail); the part rule without O (collider +748 = 0: [`PoseOpen::NoPartFrames`]);
//! - the root link's centre shift `(T+32 - T+48) x W` is written in the vector order of the other products; it is
//!   zero for the rider, so the order is not checked;
//! - F-26311's clean-up (non-child bones under a zeroed parent, the damaged-skeleton copy E+356) and the +36
//!   translation override of the locals: none applies to the rider;
//! - the locals during the ragdoll (the eject clip's last frame is assumed to stay), and the order of the
//!   character update inside the frame (decode 4.1 - 4.3).

use super::artic::{Link, V4};
use mcla_formats::rage::ragdoll::RagdollType;

/// A guest 4 x 4: four rows of four lanes.
pub type Mat = [V4; 4];

/// A float 4 x 4 as the formats crate holds it, as a guest matrix.
pub fn mat_of(m: &[[f32; 4]; 4]) -> Mat {
    [V4(m[0]), V4(m[1]), V4(m[2]), V4(m[3])]
}

/// A link's frame as the body keeps it: link +416 / +432 / +448 (the transposed rotation, w lanes the relative
/// position) and +464 (the position relative to the collider's place).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinkFrame {
    pub rows: [V4; 3],
    pub pos: V4,
}

impl LinkFrame {
    pub fn of(l: &Link) -> LinkFrame {
        LinkFrame { rows: l.rows, pos: l.pos }
    }
    pub fn put(&self, l: &mut Link) {
        l.rows = self.rows;
        l.pos = self.pos;
    }
}

/// What the pose rules do not cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseOpen {
    /// the type has no part frames O (collider +748 = 0): the other part rule of F-43257 is not decoded further
    NoPartFrames,
}

/// The fragment type's part matrices: bound +132 (current) and +136 (the previous tick's, a copy made at the start
/// of every update).
#[derive(Clone, Debug, Default)]
pub struct Parts {
    pub cur: Vec<Mat>,
    pub prev: Vec<Mat>,
}

impl Parts {
    pub fn new(n: usize) -> Parts {
        Parts { cur: vec![[V4::ZERO; 4]; n], prev: vec![[V4::ZERO; 4]; n] }
    }
    /// memcpy bound+136 <- bound+132.
    fn begin(&mut self) {
        self.prev.clone_from(&self.cur);
    }
}

/// `a` then `b`: row i = ((b0 a.i.x [+ b3 for i = 3]) + b1 a.i.y) + b2 a.i.z.
pub fn mmul(a: &Mat, b: &Mat) -> Mat {
    let mut o = [V4::ZERO; 4];
    for i in 0..4 {
        let mut t = b[0].mul(a[i].lane(0));
        if i == 3 {
            t = t.add(b[3]);
        }
        o[i] = t.add(b[1].mul(a[i].lane(1))).add(b[2].mul(a[i].lane(2)));
    }
    o
}

/// The rigid inverse the pose code writes inline (F-65945): rows 0-2 the columns of `c` (w lanes `c`'s position
/// x / y / z), row 3 = 0 - (c0.t, c1.t, c2.t, c0.t) with t = `c`'s position (dot3).
pub fn rinv(c: &Mat) -> Mat {
    let col = |j: usize| V4::new(c[0].0[j], c[1].0[j], c[2].0[j], c[3].0[j]);
    let d = [c[0].dot3(c[3]), c[1].dot3(c[3]), c[2].dot3(c[3])];
    [col(0), col(1), col(2), V4::ZERO.sub(V4::new(d[0].x(), d[1].x(), d[2].x(), d[0].x()))]
}

/// The three-lane dots of `v` with the first three rows of `i` in lanes x y z, `w` in lane w (what the part
/// matrices hold: F-43257 repeats the y dot, F-36901 the row's own x).
fn into_frame(v: V4, i: &Mat, w: Option<f32>) -> V4 {
    let (a, b, c) = (v.dot3(i[0]).x(), v.dot3(i[1]).x(), v.dot3(i[2]).x());
    V4::new(a, b, c, w.unwrap_or(b))
}

/// `m` carried into the frame `i` (rd+16): the rows rotated by it, the position relative to it. Lane w: the y dot
/// (`keep_x` false, F-43257) or the source row's x (`keep_x` true, F-36901: the position row's x is the source's,
/// not the relative one's).
fn in_frame(m: &Mat, i: &Mat, keep_x: bool) -> Mat {
    let w = |r: usize| keep_x.then(|| m[r].x());
    [into_frame(m[0], i, w(0)), into_frame(m[1], i, w(1)), into_frame(m[2], i, w(2)), into_frame(m[3].sub(i[3]), i, w(3))]
}

/// The link of child `i`: the collider's +716 entry, 0 when out of range.
pub fn link_of(child_link: &[i32], i: usize, n_links: usize) -> usize {
    match child_link.get(i) {
        Some(&k) if k >= 0 && (k as usize) < n_links => k as usize,
        _ => 0,
    }
}

/// Forward kinematics (F-97374): `G0 = L0 x P`, `Gk = Lk x G[parent k]`. The parents come before their children.
pub fn fk(locals: &[Mat], parent: &[i32], p: &Mat) -> Vec<Mat> {
    let mut g: Vec<Mat> = Vec::with_capacity(locals.len());
    for k in 0..locals.len() {
        let m = if k == 0 { mmul(&locals[0], p) } else { mmul(&locals[k], &g[parent[k] as usize]) };
        g.push(m);
    }
    g
}

fn is_set(flags: &[bool], i: usize) -> bool {
    flags.get(i).copied().unwrap_or(false)
}

/// Activation, part matrices (F-36901, rd vtable +188): `Y = C_i x G[bone_i]`, `part_i` = Y in `r`'s frame, for each
/// child not in `broken` (a broken child's part stays). `globals` are the FK of the clip's last frame under `r`.
pub fn activate_parts(t: &RagdollType, globals: &[Mat], r: &Mat, broken: &[bool], parts: &mut Parts) {
    parts.begin();
    for (i, c) in t.children.iter().enumerate() {
        if is_set(broken, i) || c.bone < 0 {
            continue;
        }
        let y = mmul(&mat_of(&c.c), &globals[c.bone as usize]);
        parts.cur[i] = in_frame(&y, r, true);
    }
}

/// Activation, the links' frames (F-93058): each link once, from its first child (not in `broken`):
/// `W = F_i x H_i x part_i x R`; the root link's position moves by `(T+32 - T+48) x W`; rows = W's 3 x 3 transposed
/// (w lanes the relative position), position = W's position - `place`. `None` for a link no child seeds.
pub fn seed_links(t: &RagdollType, child_link: &[i32], n_links: usize, parts: &Parts, r: &Mat, place: V4, broken: &[bool]) -> Vec<Option<LinkFrame>> {
    let mut out: Vec<Option<LinkFrame>> = vec![None; n_links];
    for (i, c) in t.children.iter().enumerate() {
        if is_set(broken, i) {
            continue;
        }
        let k = link_of(child_link, i, n_links);
        if out[k].is_some() {
            continue;
        }
        let f = [V4(c.rot[0]), V4(c.rot[1]), V4(c.rot[2]), V4(c.pos)];
        let mut w = mmul(&mmul(&mmul(&f, &mat_of(&c.h)), &parts.cur[i]), r);
        if k == 0 {
            let off = V4(t.cg).sub(V4(t.cg_ref));
            let moved = w[0].mul(off.lane(0)).add(w[1].mul(off.lane(1))).add(w[2].mul(off.lane(2)));
            w[3] = w[3].add(moved);
        }
        let rel = w[3].sub(place);
        let row = |j: usize| V4::new(w[0].0[j], w[1].0[j], w[2].0[j], rel.0[j]);
        out[k] = Some(LinkFrame { rows: [row(0), row(1), row(2)], pos: rel });
    }
    out
}

/// One physics tick, the part matrices (F-35284 -> F-43257): `part_i = O_i x A`, A = the link's rows transposed
/// with its position, then into `r`'s frame (`r` = rd+16 after the tick's re-centre, `place` = collider +256).
/// `absent[i]`: the part's presence flag (bound +128) is 0 and the part is not touched.
pub fn tick_parts(t: &RagdollType, links: &[LinkFrame], child_link: &[i32], place: V4, r: &Mat, absent: &[bool], parts: &mut Parts) -> Result<(), PoseOpen> {
    if t.ptr248 == 0 {
        return Err(PoseOpen::NoPartFrames);
    }
    parts.begin();
    for (i, c) in t.children.iter().enumerate() {
        if is_set(absent, i) {
            continue;
        }
        let l = &links[link_of(child_link, i, links.len())];
        let col = |j: usize| V4::new(l.rows[0].0[j], l.rows[1].0[j], l.rows[2].0[j], 0.0);
        let a: Mat = [col(0), col(1), col(2), l.pos];
        let mut p = mmul(&mat_of(&c.o), &a);
        p[3] = p[3].add(place);
        parts.cur[i] = in_frame(&p, r, false);
    }
    Ok(())
}

/// The drawn skeleton's world matrices of one frame (F-46606): bone 0 = the default pose of bone 0 x `r`; each
/// bone that is a ragdoll child's (the first child naming it) = `inv(C_i) x part_i x R`, all zero when the child
/// is in `broken`; every other bone = its local x its parent's. `locals` are the skeleton's local matrices.
pub fn bone_globals(t: &RagdollType, locals: &[Mat], parts: &Parts, r: &Mat, broken: &[bool]) -> Vec<Mat> {
    let n = locals.len();
    let mut first: Vec<Option<usize>> = vec![None; n];
    for (i, c) in t.children.iter().enumerate() {
        if c.bone > 0 && (c.bone as usize) < n && first[c.bone as usize].is_none() {
            first[c.bone as usize] = Some(i);
        }
    }
    let mut g: Vec<Mat> = Vec::with_capacity(n);
    for k in 0..n {
        let m = if k == 0 {
            mmul(&mat_of(&t.bone_default[0]), r)
        } else if let Some(i) = first[k] {
            if is_set(broken, i) {
                [V4::ZERO; 4]
            } else {
                mmul(&mmul(&rinv(&mat_of(&t.children[i].c)), &parts.cur[i]), r)
            }
        } else {
            mmul(&locals[k], &g[t.bone_parent[k] as usize])
        };
        g.push(m);
    }
    g
}

/// RemoveRagdoll (F-98260): the locals back to the default pose (skeleton data +16) and the globals the FK of it
/// under the character matrix (ctrl+32, which the skeleton's parent pointer goes back to).
pub fn remove_pose(t: &RagdollType, character: &Mat) -> (Vec<Mat>, Vec<Mat>) {
    let locals: Vec<Mat> = t.bone_default.iter().map(mat_of).collect();
    let globals = fk(&locals, &t.bone_parent, character);
    (locals, globals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// testdata/ragdoll_pose_ro0.bin (work/ragdoll/skin/export_pose.py): sections of u32 words.
    struct Fix(HashMap<String, Vec<u32>>);

    impl Fix {
        fn load() -> Fix {
            let d: &[u8] = include_bytes!("testdata/ragdoll_pose_ro0.bin");
            assert_eq!(&d[..8], b"RGPOSE01");
            let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
            let mut o = 12;
            let mut m = HashMap::new();
            for _ in 0..u(8) {
                let name = String::from_utf8(d[o..o + 8].iter().copied().take_while(|&b| b != 0).collect()).unwrap();
                let n = u(o + 8) as usize;
                m.insert(name, (0..n).map(|i| u(o + 12 + 4 * i)).collect());
                o += 12 + 4 * n;
            }
            Fix(m)
        }
        fn w(&self, n: &str) -> &[u32] {
            &self.0[n]
        }
        fn mat(&self, n: &str, i: usize) -> Mat {
            let w = &self.w(n)[16 * i..16 * i + 16];
            [0, 1, 2, 3].map(|r| V4::from_bits([w[4 * r], w[4 * r + 1], w[4 * r + 2], w[4 * r + 3]]))
        }
        fn mats(&self, n: &str) -> Vec<Mat> {
            (0..self.w(n).len() / 16).map(|i| self.mat(n, i)).collect()
        }
        fn v4(&self, n: &str) -> V4 {
            let w = self.w(n);
            V4::from_bits([w[0], w[1], w[2], w[3]])
        }
        fn ints(&self, n: &str) -> Vec<i32> {
            self.w(n).iter().map(|&x| x as i32).collect()
        }
        /// The fragment type as the .r6 gives it, rebuilt from the fixture (so the tests run without game data).
        fn ty(&self) -> RagdollType {
            let f = |n: &str, i: usize| -> [[f32; 4]; 4] { self.mat(n, i).map(|r| r.0) };
            let cg = self.w("cg");
            let v = |o: usize| [0, 1, 2, 3].map(|i| f32::from_bits(cg[o + i]));
            let mut t = RagdollType { cg: v(0), cg_ref: v(4), ptr248: 1, ..Default::default() };
            for (i, b) in self.ints("cbone").into_iter().enumerate() {
                let fm = f("cF", i);
                t.children.push(mcla_formats::rage::ragdoll::RagChild { bone: b as i16, rot: [fm[0], fm[1], fm[2]], pos: fm[3], h: f("cH", i), c: f("cC", i), o: f("cO", i), ..Default::default() });
            }
            t.bone_default = (0..self.w("dflt").len() / 16).map(|k| f("dflt", k)).collect();
            t.bone_parent = self.ints("parent");
            t
        }
    }

    /// Words equal in lanes x y z, and in all four lanes, of the 16.
    fn same(a: &Mat, b: &Mat) -> (usize, usize) {
        let mut xyz = 0;
        let mut all = 0;
        for r in 0..4 {
            for l in 0..4 {
                if a[r].0[l].to_bits() == b[r].0[l].to_bits() {
                    all += 1;
                    if l < 3 {
                        xyz += 1;
                    }
                }
            }
        }
        (xyz, all)
    }

    fn frame_of(f: &Fix, n: &str, k: usize) -> LinkFrame {
        let m = f.mat(n, k);
        LinkFrame { rows: [m[0], m[1], m[2]], pos: m[3] }
    }

    /// GoRagdoll: the 21 parts (F-36901) and the 15 links' frames (F-93058) from the clip's last frame under rd+16.
    #[test]
    fn activation_matches_recording() {
        let f = Fix::load();
        let t = f.ty();
        let (locals, r, place) = (f.mats("locals"), f.mat("r_act", 0), f.v4("cpos_act"));
        let clink = f.ints("clink");
        let g = fk(&locals, &t.bone_parent, &r);
        let mut parts = Parts::new(t.children.len());
        activate_parts(&t, &g, &r, &[], &mut parts);
        let want = f.mats("comp_act");
        let (mut xyz, mut all) = (0, 0);
        for i in 0..want.len() {
            let (a, b) = same(&parts.cur[i], &want[i]);
            xyz += a;
            all += b;
        }
        eprintln!("activation parts: xyz words {xyz} / {}, all words {all} / {}", 12 * want.len(), 16 * want.len());
        assert_eq!(xyz, 12 * want.len());
        assert_eq!(all, 16 * want.len());
        let links = seed_links(&t, &clink, 15, &parts, &r, place, &[]);
        let (mut lx, mut la, mut n) = (0, 0, 0);
        for (k, l) in links.iter().enumerate() {
            let Some(l) = l else { continue };
            let w = f.mat("lnk_act", k);
            let have: Mat = [l.rows[0], l.rows[1], l.rows[2], l.pos];
            let (a, b) = same(&have, &w);
            lx += a;
            la += b;
            n += 1;
        }
        eprintln!("activation links: {n} links, xyz words {lx} / {}, all words {la} / {}", 12 * n, 16 * n);
        assert_eq!(n, 15);
        assert_eq!(lx, 12 * n);
        assert_eq!(la, 16 * n);
    }

    /// One physics tick: part matrices from the links after the post-push and rd+16 of the next stage 0 (F-43257).
    #[test]
    fn tick_parts_match_recording() {
        let f = Fix::load();
        let t = f.ty();
        let links: Vec<LinkFrame> = (0..15).map(|k| frame_of(&f, "lnk_tck", k)).collect();
        let mut parts = Parts::new(t.children.len());
        tick_parts(&t, &links, &f.ints("clink"), f.v4("cpos_tck"), &f.mat("r_tick", 0), &[], &mut parts).unwrap();
        let want = f.mats("comp_tck");
        let (mut xyz, mut all) = (0, 0);
        for i in 0..want.len() {
            let (a, b) = same(&parts.cur[i], &want[i]);
            xyz += a;
            all += b;
        }
        eprintln!("tick parts: xyz words {xyz} / {}, all words {all} / {}", 12 * want.len(), 16 * want.len());
        assert_eq!(xyz, 12 * want.len());
        assert_eq!(all, 16 * want.len());
    }

    /// One frame: the 34 bone matrices the snapshots hold (F-46606), from the activation's parts and locals.
    #[test]
    fn frame_globals_match_recording() {
        let f = Fix::load();
        let t = f.ty();
        let mut parts = Parts::new(t.children.len());
        parts.cur = f.mats("comp_act");
        let locals = f.mats("locals");
        let r = f.mat("r_act", 0);
        let g = bone_globals(&t, &locals, &parts, &r, &[]);
        let (have, want) = (f.w("g_have"), f.mats("g_exp"));
        let (mut xyz, mut all, mut n, mut plain) = (0, 0, 0, 0);
        let fkg = fk(&locals, &t.bone_parent, &r);
        for k in 0..g.len() {
            if have[k] == 0 {
                continue;
            }
            n += 1;
            let (a, b) = same(&g[k], &want[k]);
            xyz += a;
            all += b;
            plain += same(&fkg[k], &want[k]).0;
        }
        eprintln!("frame: {n} bones, xyz words {xyz} / {}, all words {all} / {}, plain FK {plain}", 12 * n, 16 * n);
        assert_eq!(n, 34);
        assert_eq!(xyz, 12 * n);
    }

    /// The default pose at the character matrix: the snap of RemoveRagdoll is plain FK of the defaults.
    #[test]
    fn remove_snaps_to_default_pose() {
        let f = Fix::load();
        let t = f.ty();
        let id: Mat = [V4::new(1.0, 0.0, 0.0, 0.0), V4::new(0.0, 1.0, 0.0, 0.0), V4::new(0.0, 0.0, 1.0, 0.0), V4::new(0.0, 0.0, 0.0, 1.0)];
        let (locals, g) = remove_pose(&t, &id);
        assert_eq!(locals.iter().flatten().map(|v| v.bits()).collect::<Vec<_>>(), f.mats("dflt").iter().flatten().map(|v| v.bits()).collect::<Vec<_>>());
        assert_eq!(g.len(), 98);
        // bone 0 under the identity is its default pose
        assert_eq!(g.len(), locals.len());
        assert!((0..3).all(|r| (0..3).all(|l| g[0][r].0[l] == locals[0][r].0[l])));
        // a child bone's global is its local times its parent's
        let k = t.children[3].bone as usize;
        assert_eq!(same(&g[k], &mmul(&locals[k], &g[t.bone_parent[k] as usize])).1, 16);
    }

    /// The type data the .r6 gives (C, H, F, O, defaults, parents) equals what the recording held in memory.
    #[test]
    fn rider_type_matches_recording() {
        let root = std::env::var_os("MCLA_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("D:/MCLA/extracted/cache"));
        let p = root.join("resources/character/drv_mp_01_mot_set/0x5eb8d142.r6");
        let Some(t) = crate::assets::load_resource_file(&p).ok().and_then(|r| mcla_formats::rage::ragdoll::parse_character_ragdoll(&r)) else {
            eprintln!("skipped: no extracted game data with drv_mp_01_mot_set (set MCLA_DATA)");
            return;
        };
        let f = Fix::load();
        let want = f.ty();
        let bits = |m: &[[f32; 4]; 4]| m.map(|r| r.map(f32::to_bits));
        assert_eq!(t.children.len(), want.children.len());
        for (a, b) in t.children.iter().zip(&want.children) {
            assert_eq!((a.bone, bits(&a.c), bits(&a.h), bits(&a.o)), (b.bone, bits(&b.c), bits(&b.h), bits(&b.o)));
            assert_eq!((a.rot.map(|r| r.map(f32::to_bits)), a.pos.map(f32::to_bits)), (b.rot.map(|r| r.map(f32::to_bits)), b.pos.map(f32::to_bits)));
        }
        assert_eq!(t.bone_parent[1..], want.bone_parent[1..]);
        assert_eq!(t.bone_default.len(), want.bone_default.len());
        for (a, b) in t.bone_default.iter().zip(&want.bone_default) {
            assert_eq!(bits(a), bits(b));
        }
        assert_eq!((t.cg.map(f32::to_bits), t.cg_ref.map(f32::to_bits)), (want.cg.map(f32::to_bits), want.cg_ref.map(f32::to_bits)));
    }
}
