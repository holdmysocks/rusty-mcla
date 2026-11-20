//! The animated props: the oil pumps, the Cessnas, the helicopters, the 737, the Ferris wheel and the gas-station ball
//! (docs/reference/map/prop_anim_rule.md). A type whose child 0 has an always-on event set with an animation event plays
//! the type's animation of that name whenever its cache entry is given (F-55624, `physics::loose::anim_start_time`), and
//! every game frame its entry's update (F-26321 -> F-42389, while the entry's effects run) steps it (F-85177, `step_time`)
//! and poses its skeleton with it (the clip time F-50482, the frame into the skeleton's locals F-34596), the world
//! matrices along the parent chain (F-97374). The two
//! blast props (the oil pumps and the gas-station ball, the list F-76379 makes) are stopped beyond 400 m of the player's
//! car and started again within 200 m, keeping their time (F-75502 -> the prop manager's F-96501, `lod`).
//!
//! The props' meshes are bound one vertex to one bone, in the type's frame at rest: a piece of each bone is drawn by the
//! bone's posed world matrix times the inverse of its rest one (`deltas`; props.rs draws the pieces).

use glam::{Mat4, Quat, Vec3};
use mcla_formats::rage::fragphys::{AnimEvent, FragType};

/// The step of an animation a game frame (the word F-85177 multiplies the event's rate by: 1/30, 0x3D088889; the running
/// original steps it once a frame, every second tick of its fragment pool).
pub const STEP: f32 = f32::from_bits(0x3D08_8889);

/// The blast props' switch (F-96501, the prop manager's tracked list): started again within this squared distance ...
pub const NEAR2: f32 = 40000.0;
/// ... stopped beyond this one.
pub const FAR2: f32 = 160000.0;

/// One animated prop instance.
#[derive(Debug, Clone)]
pub struct AnimProp {
    /// the level's instance (`LooseWorld::instances`)
    pub inst: u32,
    /// its type (`LooseWorld::types`) and the event / animation it plays
    pub ty: usize,
    pub event: usize,
    pub anim: usize,
    /// the animation's time (entry +416)
    pub time: f32,
    /// its effects are running (entry +16 bit 0x8): the frame steps it
    pub running: bool,
    /// on the prop manager's tracked list (the blast props): its place in it
    pub listed: bool,
    /// the time kept when the switch stopped it (manager +436)
    pub saved: f32,
    /// what each bone does to its piece at the present pose, in the type's frame (`deltas`)
    pub deltas: Vec<Mat4>,
}

/// F-85177's step of the time: + rate x STEP (fused); past the top (the event's end when start <= end < length, else the
/// length) a looping event (its random byte) goes back by the range - by STEP when the range is no longer than STEP -
/// and one that does not loop stays at the top.
pub fn step_time(e: &AnimEvent, length: f32, time: f32) -> f32 {
    let t = (e.rate as f64).mul_add(STEP as f64, time as f64) as f32;
    let top = if !(e.end < e.start) && e.end < length { e.end } else { length };
    if t > top {
        if e.random {
            let range = top - e.start;
            // (fsel(STEP - range, STEP, range))
            let back = if STEP - range >= 0.0 { STEP } else { range };
            return t - back;
        }
        return top;
    }
    t
}

/// The skeleton's world matrices (in the type's frame) with `clip` at time `t`: each bone's local takes the clip's
/// translation (kind 0) and rotation (kind 1) of its id where the clip has them, its rest values else.
pub fn pose(ty: &FragType, clip: &mcla_formats::rage::anim::Animation, t: f32) -> Vec<Mat4> {
    use crate::skin::{FrameDof, DOF_NO_VALUE, DOF_QUATERNION, DOF_VECTOR3};
    use mcla_formats::rage::anim::{KIND_ROTATION, KIND_TRANSLATION};
    let Some(sk) = ty.skeleton.as_ref() else { return Vec::new() };
    let tt = crate::vehicle_model::clip_time(clip.frames, clip.duration, t);
    let (b, i, fr) = crate::drvanim::sample_block(clip, tt);
    let tracks = crate::drvanim::block_tracks(clip, b);
    let mut world: Vec<Mat4> = Vec::with_capacity(sk.bones.len());
    for bone in &sk.bones {
        let (_, mut r, mut p) = bone.local.to_scale_rotation_translation();
        for tr in tracks.iter().filter(|tr| tr.bone_id == bone.tag) {
            let kind = match tr.kind {
                KIND_TRANSLATION => DOF_VECTOR3,
                KIND_ROTATION => DOF_QUATERNION,
                _ => continue,
            };
            let mut d = FrameDof { flags: kind | DOF_NO_VALUE, v: [0.0; 4] };
            crate::drvanim::sample_track(tr, i, fr, &mut d);
            if d.flags & DOF_NO_VALUE != 0 {
                continue;
            }
            if kind == DOF_VECTOR3 {
                p = Vec3::new(d.v[0], d.v[1], d.v[2]);
            } else {
                r = Quat::from_array(d.v);
            }
        }
        let local = Mat4::from_rotation_translation(r, p);
        let w = match world.get(bone.parent) {
            Some(pw) => *pw * local,
            None => local,
        };
        world.push(w);
    }
    world
}

/// Each bone's posed world matrix times the inverse of its rest one: what moves the vertices bound to it.
pub fn deltas(ty: &FragType, posed: &[Mat4]) -> Vec<Mat4> {
    let Some(sk) = ty.skeleton.as_ref() else { return Vec::new() };
    sk.bones.iter().zip(posed).map(|(b, w)| *w * b.world.inverse()).collect()
}

/// The pose of `p` at its time (nothing when its type has no skeleton or clip).
pub fn repose(p: &mut AnimProp, types: &[FragType]) {
    let ty = &types[p.ty];
    p.deltas = match ty.anims.get(p.anim).and_then(|a| a.clip.as_ref()) {
        Some(clip) => deltas(ty, &pose(ty, clip, p.time)),
        None => Vec::new(),
    };
}

/// The prop manager's switch of the tracked props for one frame, `refp` the player's car (the camera without one): a
/// stopped one within 200 m starts again (F-80348) - its start draws G2 (`draw`) and the kept time replaces the drawn one - and is
/// posed; a running one beyond 400 m keeps its time and stops (F-29976). Returns whether `p` changed.
pub fn lod(p: &mut AnimProp, at: Vec3, refp: Vec3, types: &[FragType], draw: impl FnOnce(&AnimProp, &[FragType])) -> bool {
    if !p.listed {
        return false;
    }
    let d = refp - at;
    // (the squared distance as the code sums it: y first, then z, then x, fused)
    let d2 = (d.x as f64).mul_add(d.x as f64, (d.z as f64).mul_add(d.z as f64, (d.y * d.y) as f64) as f32 as f64) as f32;
    if !p.running {
        if d2 < NEAR2 {
            draw(p, types);
            p.time = p.saved;
            p.running = true;
            repose(p, types);
            return true;
        }
    } else if d2 > FAR2 {
        p.saved = p.time;
        p.running = false;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(start: f32, end: f32, random: bool, rate: f32) -> AnimEvent {
        AnimEvent { child: 0, anim: "cycle".into(), rate, start, end, random }
    }

    /// F-85177's step: rate / 30 a frame; a looping event past its length back by the range (or by the step when the
    /// range is shorter), one that does not loop held at its top.
    #[test]
    fn step_time_rule() {
        assert_eq!(STEP, 1.0 / 30.0);
        let e = ev(0.0, -1.0, true, 0.5);
        assert_eq!(step_time(&e, 13.333333, 4.639262), ((0.5f64).mul_add(STEP as f64, 4.639262f32 as f64)) as f32);
        // (the Ferris wheel of the running original: 4.639262 -> 4.655928, one frame)
        assert!((step_time(&e, 13.333333, 4.639262) - 4.655928).abs() < 2e-6);
        let t = step_time(&e, 1.9666667, 1.96);
        assert_eq!(t, ((0.5f64).mul_add(STEP as f64, 1.96f32 as f64) as f32) - 1.9666667);
        assert_eq!(step_time(&ev(0.0, -1.0, false, 0.5), 1.9666667, 1.96), 1.9666667);
        assert_eq!(step_time(&ev(0.0, 0.01, true, 1.0), 4.0, 0.0), ((1.0f64).mul_add(STEP as f64, 0.0) as f32) - STEP);
    }

    /// The pose against the running original (work/carparts2/animrec/poses.py at the title, two samples a second apart:
    /// testdata/prop_poses_title.txt): each animated prop's time (entry +416), its instance matrix and its skeleton's
    /// world matrices; the port's instance x `pose` at the recorded time gives the same matrices. Needs the disc
    /// (MCLA_DATA_DIR).
    #[test]
    fn pose_matches_the_running_original() {
        let Ok(root) = std::env::var("MCLA_DATA_DIR") else { return };
        let lw = crate::physics::loose::LooseWorld::load(&std::path::Path::new(&root).join("resources/prop/0x0e66973c.r131")).expect("prop file");
        let rec = include_str!("../testdata/prop_poses_title.txt");
        let mat = |f: &[f32]| Mat4::from_cols_array(&[f[0], f[1], f[2], 0.0, f[4], f[5], f[6], 0.0, f[8], f[9], f[10], 0.0, f[12], f[13], f[14], 1.0]);
        let mut cur: Option<(usize, Mat4, Vec<Mat4>)> = None;
        let (mut props, mut bones, mut worst_axis, mut worst_pos) = (0, 0, 0.0f32, 0.0f32);
        for line in rec.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let num = |s: &[&str]| s.iter().map(|x| x.parse::<f32>().unwrap()).collect::<Vec<f32>>();
            match w[0] {
                "P" => {
                    let (name, t) = (w[3], w[7].parse::<f32>().unwrap());
                    let inst = mat(&num(&w[9..25]));
                    let at = inst.w_axis.truncate();
                    let (k, ty) = lw.instances.iter().enumerate().map(|(k, (fi, ty))| (k, fi, *ty)).find(|(_, fi, ty)| lw.types[*ty].name.contains(name) && (Vec3::from(fi.matrix[3]) - at).length() < 0.01).map(|(k, _, ty)| (k, ty)).unwrap_or_else(|| panic!("{name} at {at:?}"));
                    let t_ = &lw.types[ty];
                    let e = &t_.anim_events[0];
                    let a = t_.anims.iter().position(|a| a.name.eq_ignore_ascii_case(&e.anim)).unwrap();
                    let posed = pose(t_, t_.anims[a].clip.as_ref().unwrap(), t);
                    let _ = k;
                    cur = Some((ty, inst, posed));
                    props += 1;
                }
                "B" => {
                    let (_, inst, posed) = cur.as_ref().unwrap();
                    let b: usize = w[3].parse().unwrap();
                    let want = mat(&num(&w[4..20]));
                    let got = *inst * posed[b];
                    for c in 0..3 {
                        worst_axis = worst_axis.max((got.col(c) - want.col(c)).abs().max_element());
                    }
                    worst_pos = worst_pos.max((got.w_axis - want.w_axis).abs().max_element());
                    bones += 1;
                }
                _ => {}
            }
        }
        eprintln!("{props} props, {bones} bones: worst axis {worst_axis:e}, worst place {worst_pos:e} m");
        assert_eq!((props, bones), (66, 344));
        assert!(worst_axis < 1e-4 && worst_pos < 2e-3, "axis {worst_axis} place {worst_pos}");
    }
}
