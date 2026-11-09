//! The game's cutscene animation (`resources/animation/cineanimpacks/*.r1`): every scene
//! is a set of animations sharing a name prefix -
//! `story/cut_story_race_gh_06/CSCamera` (camera position, rotation, field of view),
//! `.../Entity/mover_drv00_car/..` (a car's path: mover translation / rotation),
//! `.../Entity/booke_drv01_name_drv_mb_04_set/..` (a character: the set to use is named
//! after `_name_`, the root bone carries the position in the scene).
//! Story scenes are authored in world coordinates; race start scenes relative to the grid.
//! The camera looks along its local -Z with +Y up; the field of view is vertical degrees.
//! Scenes that can happen anywhere (arrests, the winner's scene) are relative to the
//! player's car: its mover gives the car's place in the scene, which fixes the origin.
//! Their people have no set in the name (`cut_drv01_cop`, `cut_drv01_f`, `player_drv00`):
//! police, the woman starting the race, the player.
//! Hangouts: `hangout/cut_hangout_NN/<f|m>_pedNN_loop` are the people standing around there
//! (looping, world coordinates), `.._ent_01` / `.._ext` the scenes of pulling in and leaving.
//! `<scene>_IM` holds the facial capture of the speaking characters (same entity names,
//! starting with the scene; it repeats the body in local space, so only the bones the
//! body animation leaves alone are taken from it), `hangout/<scene>_peds` the bystanders.

use crate::assets::load_resource_file;
use crate::render::Gpu;
use crate::skin::{Clip, FaceShapes, SkinBuffer, SkinModel};
use glam::{Mat4, Quat, Vec3};
use hashbrown::HashMap;
use crate::skin::{DOF_FLOAT, DOF_QUATERNION, DOF_VECTOR3};
use mcla_formats::rage::anim::{animation_named, pack_entries, Animation, KIND_FOV, KIND_MOVER_ROTATION, KIND_MOVER_TRANSLATION, KIND_ROTATION, KIND_TRANSLATION};
use mcla_formats::rage::Rd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Actor {
    label: String,
    model: Arc<SkinModel>,
    clip: Clip,
    /// the scene time the clip began (0; a slot animated again mid-scene: then - `reanimate`)
    t0: f32,
    /// facial animation
    face: Option<Clip>,
    /// the face's blend shapes and the facial pack's kind-21 track of every target (by the target's id)
    shapes: Option<(FaceShapes, Vec<Option<usize>>)>,
    /// a slot animated again (`reanimate`): the old clip's root at the switch, its height 0, added to the new clip's
    /// root - rotation on the left, translation summed (the AddSubtract over the root-bone filter)
    root_add: Option<(Quat, Vec3)>,
    /// and the old pose it blends out of over 0.25 s from that scene time
    fade: Option<(Vec<(Quat, Vec3)>, f32)>,
    /// a face's start (F-19950): the pose it blends in from over 0.25 s at scene time 0 - the body's frame 0 with the
    /// face bones at rest
    face_in: Option<Vec<(Quat, Vec3)>>,
}

/// The crmtNodeBlend step from `old` into `local` at weight `w` (clamped, F-27951): the frame blend with `old` the
/// destination and `local` the source (F-56920, skin.rs `blend_pose`), the result into `local`. At 1 the node is gone.
fn blend_in(local: &mut [(Quat, Vec3)], old: &[(Quat, Vec3)], w: f32) {
    let w = crate::skin::frame_weight(w);
    if w >= 1.0 {
        return;
    }
    let mut out = old.to_vec();
    crate::skin::blend_pose(&mut out, local, w);
    for (l, o) in local.iter_mut().zip(out) {
        *l = o;
    }
}

impl Actor {
    /// The pose at frame `f` of its clip, scene time `t`.
    /// The body at clip time `ct` (scene time `t`).
    fn pose(&self, ct: f32, t: f32, local: &mut Vec<(Quat, Vec3)>) {
        pose_at(&self.clip.anim, &self.model, ct, local);
        if let (Some((r, p)), Some(l)) = (self.root_add, local.first_mut()) {
            l.0 = (r * l.0).normalize();
            l.1 += p;
        }
        // (the crmtNodeBlend of ReAnimate, rate 4, F-51260: w += 4 dt clamped, linear; per DOF F-56920:
        // rotations nlerp with the sign fix, translations kept below w 0.001 and copied from 0.999)
        if let Some((old, t1)) = self.fade.as_ref() {
            blend_in(local, old, 4.0 * (t - t1));
        }
    }
}

/// Track kind of a blend-shape weight (`crCreatureComponentBlendShapes` reads kind 21 by target id,
/// `F-77597`).
const KIND_BLEND_SHAPE: u8 = 21;

struct Mover {
    /// "drv00", "henry", "omnibus"
    label: String,
    anim: Arc<Animation>,
}

pub struct Scene {
    pub name: String,
    camera: Option<Arc<Animation>>,
    actors: Vec<Actor>,
    movers: Vec<Mover>,
    pub duration: f32,
    pub t: f32,
    /// the scene time at the latest game tick's start (`Cine::begin_tick`): the cast is drawn between it and `t` by the render
    /// alpha (`draw_t`)
    pub t_prev: f32,
    /// scene space -> world
    pub origin: Mat4,
    /// CineScript_Camera_Kill: the game camera shows the rest
    camera_killed: bool,
    /// the camera's cut test state and the last (scene time, snapped time) it gave
    cam_cut: std::sync::Mutex<(CamCut, Option<(f32, f32)>)>,
}

impl Scene {
    /// The length the event-time rule compares with: the scene camera's own clip (header +12), else the scene's.
    /// The scene time a drawn frame shows: between the latest tick's start and end by the render alpha `a` (1: the tick's end,
    /// as the logic uses). This is the port's interpolation, not the original's (it draws each 30 Hz frame at its update's
    /// time): above the tick rate the cast would move in 60 Hz steps. A jump (a skip, a start part-way in, the warp's
    /// hand-over) or a time going back is drawn at once.
    pub fn draw_t(&self, a: f32) -> f32 {
        let d = self.t - self.t_prev;
        if (0.0..=0.1).contains(&d) { (a - 1.0).mul_add(d, self.t) } else { self.t }
    }

    pub fn camera_len(&self) -> f32 {
        self.camera.as_ref().map_or(self.duration, |c| c.duration)
    }

    /// The cutscene form of the cascades (F-34053's second branch): the cine core script (0x95ca1d1b 0xD1D, 0xE1F) pushes
    /// every character and car event it launches into kill buffer 1 (CineScript_PushKillBuffer, F-73266), which sets the
    /// renderer's +389 until F-60657 clears the buffer at the scene's end; the form is skipped while the scene's name holds
    /// "racestart" but neither "red" nor "payback" (cine manager +319, F-81251; +335, CineScript_SetRaceStarterLighting, is
    /// called by no script).
    pub fn close_shadows(&self) -> bool {
        let n = self.name.to_ascii_lowercase();
        let racestart = n.contains("racestart") && !n.contains("red") && !n.contains("payback");
        (!self.actors.is_empty() || !self.movers.is_empty()) && !racestart
    }
}

/// The scene camera's cut test limits (F-25555 passes the tune globals, 2 m and 20 degrees in the image).
const CAM_CUT_METRES: f32 = 2.0;
const CAM_CUT_DEGREES: f32 = 20.0;

/// The camera cut test's state (F-34419's globals): the previous grid time (-1: none), the last time it gave and the
/// one before, and whether the last test cut. A camera launch (F-22373 / F-62736) sets them to its start time, not cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamCut {
    prev: f32,
    last: f32,
    last2: f32,
    cut: bool,
}

impl Default for CamCut {
    fn default() -> Self {
        CamCut { prev: 0.0, last: 0.0, last2: 0.0, cut: false }
    }
}

/// `F-34419`: the camera cut test. t0 = t on the 1/30 s grid; from where the last test stood plus 1/30 if it skipped more
/// than 0.05 s (`at`), else t0. Between `at` and the next grid step t1 the camera's move d and, unless the last test cut,
/// the change of its step (the step before `at` minus d, times 900) give a cut when |d|^2 x |change|^2 x 900 > metres^2
/// (after a cut the distance never cuts); a turn over `degrees` cuts too. A cut gives `at`, else t. With `on` (the scene
/// camera's update) the state moves on: prev = t0, last = the result. Test: `cam_cut_matches_the_original`.
fn cam_cut_rule(s: &mut CamCut, on: bool, t: f32, metres: f32, degrees: f32, pos: impl Fn(f32) -> Option<Vec3>, rot: impl Fn(f32) -> Option<Quat>) -> f32 {
    if !(s.prev > -1.0) {
        s.prev = t;
    }
    let step = 1.0f32 / 30.0;
    let t0 = t - (t as f64 % (1.0f32 / 30.0) as f64) as f32;
    // (the image's 0.05 is 0x3D4CCCCE, one step above 0.05f)
    let at = if t0 - s.prev > f32::from_bits(0x3d4c_ccce) { s.prev + step } else { t0 };
    let (before, t1) = (at - step, t0 + step);
    let len2 = |v: Vec3| (v.x as f64).mul_add(v.x as f64, (v.y as f64).mul_add(v.y as f64, (v.z * v.z) as f64) as f32 as f64) as f32;
    let p_at = pos(at);
    let mut d = p_at.unwrap_or(Vec3::ZERO);
    if let Some(p) = pos(t1) {
        d -= p;
    }
    let mut k = 0.0f32;
    if !s.cut {
        let mut b = pos(before).unwrap_or(Vec3::ZERO);
        if let Some(p) = p_at {
            b -= p;
        }
        k = len2(b - d) * f32::from_bits(0x4460_fffe);
    }
    let q0 = rot(at).unwrap_or(Quat::IDENTITY);
    let turn = rot(t1).map_or(0.0, |q1| crate::cinecar::quat_angle(q0, q1));
    let cut = len2(d) * k > metres * metres || turn > degrees * 0.017_453_292;
    let out = if cut { at } else { t };
    if on {
        s.cut = cut;
        s.last2 = s.last;
        s.prev = t0;
        s.last = out;
    }
    out
}

/// The cine-to-game camera blend (the viewport's blend object, made at boot by F-20115: time +0 = -1 idle, the
/// stored camera +16 / fov +80, rate +84 = 1 per second). `start` is F-98908: it stores the camera the viewport last
/// showed and sets the time to 0, or straight to 1 (no blend) when the hand-off is a cut.
#[derive(Clone, Copy, Debug)]
pub struct CamBlend {
    pub t: f32,
    pub rate: f32,
    pub from: [f32; 16],
    pub from_fov: f32,
}

impl Default for CamBlend {
    fn default() -> Self {
        CamBlend { t: -1.0, rate: 1.0, from: [0.0; 16], from_fov: -1.0 }
    }
}

impl CamBlend {
    pub fn start(&mut self, cam: [f32; 16], fov: f32, cut: bool) {
        self.t = if cut { 1.0 } else { 0.0 };
        self.from = cam;
        self.from_fov = fov;
    }

    pub fn active(&self) -> bool {
        self.t > -1.0
    }

    /// `F-82215`: one update of `dt` toward the target camera (rows of 4: rotation rows, translation row 3). While the
    /// blend runs the result is F-45235 from the stored camera by (1 - ease(t)) / 2 and the fov's lerp, else the target.
    /// `out` keeps its rotation rows' fourth entries (the original writes only the 3x3 and row 3). Test:
    /// `cam_blend_matches_the_original`.
    pub fn step(&mut self, dt: f32, out: &mut [f32; 16], target: &[f32; 16], target_fov: f32) -> f32 {
        if self.t > -1.0 {
            self.t = (self.rate as f64).mul_add(dt as f64, self.t as f64) as f32;
        }
        let blending = if self.t >= 1.0 {
            self.t = -1.0;
            false
        } else {
            self.t > -1.0
        };
        if !blending {
            *out = *target;
            return target_fov;
        }
        let w = (1.0 - ease(self.t)) * 0.5;
        matrix_blend(out, &self.from, target, w);
        ((target_fov - self.from_fov) as f64).mul_add(w as f64, self.from_fov as f64) as f32
    }
}

/// `F-39265`: 1 at or below 0, -1 at or above 1, else cos(pi t).
fn ease(t: f32) -> f32 {
    if !(t > 0.0) {
        1.0
    } else if !(t < 1.0) {
        -1.0
    } else {
        ((t * std::f32::consts::PI) as f64).cos() as f32
    }
}

/// `F-50812`: a rotation (rows 0..2 of a 4x4) as a quaternion (x, y, z, w), from the largest of the trace and the diagonal.
pub(crate) fn mat_quat(m: &[f32; 16]) -> [f32; 4] {
    let (m00, m11, m22) = (m[0], m[5], m[10]);
    let tr = m00 + m11 + m22;
    let case = if tr < m00 {
        if m00 < m11 {
            if m11 < m22 { 3 } else { 2 }
        } else if m00 < m22 {
            3
        } else {
            1
        }
    } else if tr < m11 {
        if m11 < m22 { 3 } else { 2 }
    } else if tr < m22 {
        3
    } else {
        0
    };
    let big = |d: f32| (((d as f64).mul_add(2.0, -(tr as f64)) as f32 + 1.0) as f64).sqrt() as f32 * 0.5;
    match case {
        0 => {
            let w = ((tr + 1.0) as f64).sqrt() as f32 * 0.5;
            let s = 0.25 / w;
            [(m[6] - m[9]) * s, (m[8] - m[2]) * s, (m[1] - m[4]) * s, w]
        }
        1 => {
            let x = big(m00);
            let s = 0.25 / x;
            [x, (m[4] + m[1]) * s, (m[8] + m[2]) * s, (m[6] - m[9]) * s]
        }
        2 => {
            let y = big(m11);
            let s = 0.25 / y;
            [(m[4] + m[1]) * s, y, (m[9] + m[6]) * s, (m[8] - m[2]) * s]
        }
        _ => {
            let z = big(m22);
            let s = 0.25 / z;
            [(m[8] + m[2]) * s, (m[9] + m[6]) * s, z, (m[1] - m[4]) * s]
        }
    }
}

/// `F-40856`: a quaternion as the rotation rows of `m` (each part scaled by sqrt 2 first; the fourth entries stay).
pub(crate) fn quat_mat(m: &mut [f32; 16], q: [f32; 4]) {
    let k = f32::from_bits(0x3fb5_04f3);
    let (x, y, z, w) = (q[0] * k, q[1] * k, q[2] * k, q[3] * k);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (y * x, z * x, z * y);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    m[2] = xz - wy;
    m[1] = xy + wz;
    m[4] = xy - wz;
    m[8] = wy + xz;
    m[6] = wx + yz;
    m[9] = yz - wx;
    m[10] = 1.0 - (yy + xx);
    m[0] = 1.0 - (zz + yy);
    m[5] = 1.0 - (zz + xx);
}

/// `F-25555` after its cut test: the camera matrix `m` and its two lens values from the camera clip's samples, each
/// left as it is when its track is missing: the translation is row 3, the rotation the rows by F-40856, the kind-10 focal
/// length (mm on a 35 mm gate) the horizontal angle 2 atan(0.7085 / (v 0.03937)) through the CRT's double atan
/// (cam+104), the kind-14 value cam+108 (`cam_sample_matches_the_original`).
fn cam_sample(m: &mut [f32; 16], lens: &mut f32, k14: &mut f32, pos: Option<Vec3>, rot: Option<[f32; 4]>, focal: Option<f32>, f14: Option<f32>) {
    if let Some(p) = pos {
        m[12..15].copy_from_slice(&p.to_array());
    }
    if let Some(q) = rot {
        quat_mat(m, q);
    }
    if let Some(v) = focal {
        *lens = ((f32::from_bits(0x3f35_6042) / (v * f32::from_bits(0x3d21_4270))) as f64).atan() as f32 * 2.0;
    }
    if let Some(v) = f14 {
        *k14 = v;
    }
}

/// `F-20340`: spherical interpolation of two quaternions by `u` (the angle by F-94425), normalised by a reciprocal
/// square root with two Newton steps (0 for a zero length); `a` itself when the angle's sine is 0.
pub(crate) fn slerp(a: [f32; 4], b: [f32; 4], u: f32) -> [f32; 4] {
    let qa = Quat::from_array(a);
    let qb = Quat::from_array(b);
    let th = crate::cinecar::quat_angle(qa, qb) * 0.5;
    let s = (th as f64).sin() as f32;
    if s == 0.0 {
        return a;
    }
    let tu = th * u;
    let inv = 1.0 / s;
    let c0 = ((th - tu) as f64).sin() as f32 * inv;
    let c1 = (tu as f64).sin() as f32 * inv;
    let fm = |p: f32, q: f32, r: f32| (p as f64).mul_add(q as f64, r as f64) as f32;
    let x = fm(a[0], c0, b[0] * c1);
    let y = fm(b[1], c1, a[1] * c0);
    let z = fm(b[2], c1, a[2] * c0);
    let w = fm(b[3], c1, a[3] * c0);
    let n = fm(w, w, fm(z, z, fm(x, x, y * y)));
    let nd = n as f64;
    let y0 = (1.0 / nd.sqrt()) as f32 as f64;
    let y1 = (-(y0 * y0).mul_add(nd, -1.0)).mul_add(y0 * 0.5, y0);
    let y2 = (-(y1 * y1).mul_add(nd, -1.0)).mul_add(y1 * 0.5, y1);
    let k = if -nd >= 0.0 { 0.0 } else { y2 as f32 };
    [x * k, y * k, z * k, w * k]
}

/// `F-45235`: two 4x4s blended by `u`: their rotations as quaternions (the first flipped when the dot is negative),
/// slerped, back to rows 0..2; row 3 lerped.
fn matrix_blend(out: &mut [f32; 16], a: &[f32; 16], b: &[f32; 16], u: f32) {
    let mut qa = mat_quat(a);
    let qb = mat_quat(b);
    let fm = |p: f32, q: f32, r: f32| (p as f64).mul_add(q as f64, r as f64) as f32;
    let d = fm(qb[3], qa[3], fm(qb[0], qa[0], fm(qa[1], qb[1], qa[2] * qb[2])));
    if d < 0.0 {
        qa = [-qa[0], -qa[1], -qa[2], -qa[3]];
    }
    quat_mat(out, slerp(qa, qb, u));
    for i in 12..16 {
        out[i] = (b[i] - a[i]) * u + a[i];
    }
}

/// The race-start scenes that hand the picture back with `cam_to_game` (event 78: F-91263 calls F-45212 with no cut,
/// so the viewport blends, `CamBlend`); the others' `bookmark_cut_to_game` cuts. From their generated scripts
/// (`work/driverlight/gen/compare.py` over all 200 race-start scripts, the hash-named ones too: every one with
/// `cam_to_game` has its `bookmark` at the same time).
const BLEND_TO_GAME: [&str; 68] = [
    "cut_racestart_1x1_01", "cut_racestart_1x1_02", "cut_racestart_1x1_03", "cut_racestart_1x2_02", "cut_racestart_1x2_03",
    "cut_racestart_1x2_04", "cut_racestart_1x2_05", "cut_racestart_1x8_01", "cut_racestart_1x8_02", "cut_racestart_2x4_split_01",
    "cut_racestart_2x4_split_02", "cut_racestart_3x3_01", "cut_racestart_3x3_02", "cut_racestart_3x3_03", "cut_racestart_3x3_04",
    "cut_racestart_3x3_split_01", "cut_racestart_3x3_split_02", "cut_racestart_taunt_hk_01", "cut_racestart_taunt_hk_plyr_01", "cut_racestart_taunt_hk_plyr_02",
    "cut_racestart_taunt_plyr_hk_01", "cut_racestart_taunt_plyr_hk_02", "cut_racestarter_1x8_01", "cut_racestarter_1x8_02", "cut_racestarter_2x4_01",
    "cut_racestarter_2x4_02", "cut_racestarter_2x4_03", "cut_racestarter_2x4_04", "cut_racestarter_2x4_split_01", "cut_racestarter_2x4_split_02",
    "cut_racestarter_3x3_01", "cut_racestarter_3x3_02", "cut_racestarter_3x3_03", "cut_racestarter_3x3_04", "cut_racestarter_3x3_05",
    "cut_racestarter_3x3_06", "cut_racestarter_3x3_split_01", "cut_racestarter_3x3_split_02", "cut_racestarter_4x2_01", "cut_racestarter_4x2_02",
    "cut_redlight_01", "cut_redlight_be_00", "cut_redlight_be_01", "cut_redlight_dt_00", "cut_redlight_hl_00",
    "cut_redlight_hl_01", "cut_redlight_hw_00", "cut_redlight_hw_01", "cut_redlight_hw_02", "cut_redlight_hw_03",
    "cut_story_hangout_pinkslip_hills", "gen_prerace_grid_4x2_01", "gen_prerace_starter_01", "gen_prerace_starter_04", "gen_prerace_starter_05",
    "gen_prerace_starter_07", "gen_prerace_starter_09", "gen_prerace_starter_1x1_01", "gen_prerace_taunt_01", "gen_prerace_taunt_02",
    "gen_prerace_taunt_03", "gen_prerace_taunt_07", "gen_prerace_taunt_09", "taunt_01", "taunt_02",
    "taunt_03", "taunt_04", "taunt_05",
];

/// Whether a scene hands the picture to the game camera by the 1 s blend (`BLEND_TO_GAME`).
pub fn blends_to_game(scene: &str) -> bool {
    let n = scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase();
    BLEND_TO_GAME.contains(&n.as_str())
}

/// A camera (eye, forward, up) as the original's rows: right, up, back (x cross y = z), the eye in row 3.
pub fn cam_rows(eye: Vec3, fwd: Vec3, up: Vec3) -> [f32; 16] {
    let z = (-fwd).normalize_or(Vec3::Z);
    let x = up.cross(z).normalize_or(Vec3::X);
    let y = z.cross(x);
    [x.x, x.y, x.z, 0.0, y.x, y.y, y.z, 0.0, z.x, z.y, z.z, 0.0, eye.x, eye.y, eye.z, 1.0]
}

/// The character sets of the hangouts' people (`Cine::set_ambient`).
const AMBIENT_FEMALE: [&str; 4] = ["ped_fa_001_set", "ped_fb_002_set", "ped_fc_001_set", "ped_fh_001_set"];
const AMBIENT_MALE: [&str; 5] = ["ped_ma_001_set", "ped_mb_001_set", "ped_mc_001_set", "ped_mh_001_set", "ped_ma_003_set"];
const AMBIENT_DRIVERS: [&str; 4] = ["drv_mb_02_set", "drv_mc_02_set", "drv_ma_02_set", "drv_mh_02_set"];

#[derive(Default)]
pub struct Cine {
    char_dir: PathBuf,
    /// the character sets the active racers wear (racer +144: the generic cast's in-use test F-68164), kept by `main`
    pub racer_sets: Vec<String>,
    /// the local player's car is a motorcycle (F-17373: the racer's car config and its catalogue class 3), kept by `main`
    pub player_bike: bool,
    /// the pause camera hold (F-83799 with -1, no time limit) taken at a scene's cam_gps_warp_out with the warper's
    /// pause pending (F-35052): the camera stays where it was while the scene plays on; cleared by `start` / `stop`
    pub held: Option<(Vec3, Vec3, Vec3, f32)>,
    /// scene ("story/cut_intro") -> (entry name, pack file)
    index: HashMap<String, Vec<(String, PathBuf)>>,
    models: HashMap<String, Option<Arc<SkinModel>>>,
    /// character sets read ahead on worker threads (`read_ahead_ambient`): the file and what it holds
    reading: HashMap<String, std::thread::JoinHandle<Option<(PathBuf, mcla_formats::rage::character::Character)>>>,
    /// a story scene's animation packs read ahead on a worker thread (`read_ahead_scene`), taken by `start`
    packs: HashMap<PathBuf, std::thread::JoinHandle<Option<mcla_formats::rsc::Resource>>>,
    pub playing: Option<Scene>,
    buf: SkinBuffer,
    /// people of the hangout nearby: each loops at its own length
    ambient: Vec<Actor>,
    ambient_name: String,
    clock: f32,
    /// `clock` at the latest tick's start (the hangout people drawn between, as `Scene::draw_t`)
    clock_prev: f32,
    /// MCLA_DRAW_STEPS: the drawn frame's number for the `dstep` lines
    steps: Option<u64>,
    /// a start scene after its cut to the game camera: its script runs on to the clip's end, so its people
    /// stay where they are and go on moving (work/career2/cap/README.md, finding 5: the starter is still in
    /// the game view for a few seconds after the cut)
    linger: Option<Scene>,
}

/// The in-game scenes whose generated scripts name an audio stream (their static block's upper-case scene name,
/// handed to core_rolling_prototype: CineScript_Audio_PrepareStream, F-27055 -> F-16992 looks up `CUTSCENES_<name>`).
/// The other generated scenes (cut_cop_arrest_04 - _06, the reward scenes, cut_cop_alert_02 ...) name none; CUT_INTRO
/// is left out, its audio is the story scene path's (`start_scene_audio`). cut_cop_fidget and cut_cop_walk_plyr_bike
/// have no object: no sound. The `racestart/` ones are named without their folder: the eleven starter scenes (stopped
/// early by their `stopaudiostreams` event) and the delivery / payback key hand-overs.
pub const SCENE_AUDIO: &[&str] = &[
    "cut_cop_alert", "cut_cop_arrest", "cut_cop_arrest_02", "cut_cop_arrest_03", "cut_cop_arrest_07", "cut_cop_arrest_08",
    "cut_cop_arrest_bikestand", "cut_cop_fidget", "cut_cop_fidget_02", "cut_cop_fidget_bike", "cut_cop_walk", "cut_cop_walk_02",
    "cut_cop_walk_plyr_02", "cut_cop_walk_plyr_bike", "cut_delivery_get_keys", "cut_loss_chung_hee", "cut_loss_lester",
    "cut_loss_pete", "cut_payback_get_keys", "gen_prerace_starter_2x4_01", "gen_prerace_starter_2x4_02",
    "gen_prerace_starter_2x4_03", "gen_prerace_starter_2x4_04", "gen_prerace_starter_2x4_05", "gen_prerace_starter_2x4_06",
    "gen_prerace_starter_2x4_07", "gen_prerace_starter_2x4_08", "gen_prerace_starter_2x4_09", "gen_prerace_starter_2x4_10",
    "gen_prerace_starter_2x4_11",
];

/// A scene's own fades that go down and come back up inside it: (scene, to black, scene seconds, speed). The handler
/// F-91263 starts game/misctasks/screen_fade_to_black / _to_scene with the event's f3 as the speed (1.0 when <= 0):
/// fade_speed_in = speed, held (fade_speed_out 0.01) / fade_speed_out = speed, which fades up only a fade that is down
/// or held (the script's TransitionOut == 0 && (TransitionIn || TransitionOutisReady) test). A fade to black that ends a
/// scene is held until the next scene's start (core_rolling_prototype fades up at its clock's start) or the caller's own
/// fade up; it is listed only where that caller is ported: the pull-over's walk and ticket scenes (CopPullover.sco,
/// `main.rs::ticket_tick`; the arrests' are `police::arrest_fade`), the garage intro's (REDLIGHT_GH_01_GARAGE_ON_ENTER)
/// and the payback intro's (GARAGE_BEACHES_PAYBACK_ON_ENTER; both `main.rs`). From work/driverlight/gen
/// (docs/reference/cutscenes/generated_description.md).
pub const SCENE_FADES: &[(&str, bool, f32, f32)] = &[
    ("story/cut_story_district_champs", true, 73.3, 1.0),
    ("story/cut_story_district_champs", false, 74.3333, 1.0),
    ("story/cut_story_district_champs_night", true, 73.3, 1.0),
    ("story/cut_story_district_champs_night", false, 74.3333, 1.0),
    ("story/cut_story_find_sh_04", true, 56.9333, 1.0),
    ("story/cut_story_find_sh_04", false, 58.3333, 1.0),
    ("story/cut_story_hangout_series_downtown", true, 1.26667, 1.0),
    ("story/cut_story_hangout_series_downtown", false, 2.26667, 2.0),
    ("story/cut_story_hollywood_garage_intro", true, 0.333333, 1.0),
    ("story/cut_story_hollywood_garage_intro", false, 2.0, 1.0),
    ("story/cut_story_hollywood_garage_intro", true, 62.0, 1.0),
    ("story/cut_story_payback_intro", true, 2.0, 1.0),
    ("story/cut_story_payback_intro", false, 3.66667, 1.0),
    ("story/cut_story_payback_intro", true, 65.3333, 1.0),
    ("cut_cop_walk_02", true, 5.86667, 1.0),
    ("cut_reward_gen_01", false, 1.0, 1.0),
    ("cut_reward_gen_01", true, 11.0, 1.0),
    ("cut_reward_gen_02", false, 1.0, 1.0),
    ("cut_reward_gen_02", true, 11.0, 1.0),
    ("cut_reward_gen_03", false, 1.0, 1.0),
    ("cut_reward_gen_03", true, 11.0, 1.0),
    ("cut_reward_gen_04", false, 1.0, 1.0),
    ("cut_reward_gen_04", true, 11.0, 1.0),
    ("cut_cop_fidget_02", true, 6.3, 1.0),
    ("sc_cut_cop_ticket_suv", true, 6.3, 1.0),
    ("cut_cop_fidget_bike", false, -9.03333, 1.0),
    ("cut_cop_fidget_bike", true, 6.3, 1.0),
];

/// The scenes whose script names a stream other than the scene's own name (the description's +8,
/// docs/reference/cutscenes/generated_description.md): the payback goons' walk-ups, whose objects are
/// CUTSCENES_CUT_PAYBACK_GOONS_WALKUP1 / 2 (game.dat; streams CUT_MISC_PAYBACK_GOONS_WALKUP1 / 2).
const SCENE_STREAM: [(&str, &str); 2] = [
    ("payback_goons_walkup_01", "CUT_PAYBACK_GOONS_WALKUP1"),
    ("payback_goons_walkup_02", "CUT_PAYBACK_GOONS_WALKUP2"),
];

/// The audio stream an in-game scene plays (`SCENE_AUDIO`, `SCENE_STREAM`), upper case as its script names it.
pub fn scene_audio(scene: &str) -> Option<String> {
    let n = scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase();
    if let Some(&(_, s)) = SCENE_STREAM.iter().find(|r| r.0 == n) {
        return Some(s.to_string());
    }
    SCENE_AUDIO.contains(&n.as_str()).then(|| n.to_ascii_uppercase())
}

/// The story scenes' skip and cut events, from their generated scripts (`Game/CineScripts/generated/story/
/// cut_story_*`, CineScript_Events_LaunchEvent): the scene, its `bookmark` (73), `bookmark_cut_to_game` (72)
/// and `cut_to_game` (74) times. A scene not listed has none (cut_story_motorcycle_rep_A / _B, cut_intro).
pub const STORY_MARKS: &[(&str, &[(u8, f32)])] = &[
    ("story/cut_story_payback_intro", &[(73, 66.3333)]),
    ("story/cut_story_find_sh_04_alt", &[(73, 61.5)]),
    ("story/cut_story_hangout_series_hollywoodc", &[(72, 56.0)]),
    ("story/cut_story_hangout_pinkslip_downtown", &[(73, 43.3667)]),
    ("story/cut_story_city_champ_night", &[(73, 38.0)]),
    ("story/cut_story_district_champs_night", &[(72, 74.3333)]),
    ("story/cut_story_find_sh_02c", &[(73, 105.0)]),
    ("story/cut_story_race_sh_01", &[(73, 3.33333)]),
    ("story/cut_story_hangout_series_downtown", &[(73, 41.1)]),
    ("story/cut_story_motorcycle_rep", &[(72, 48.3333)]),
    ("story/cut_story_bestof_sh_05", &[(73, 41.6667)]),
    ("story/cut_story_district_champs", &[(72, 74.3333)]),
    ("story/cut_story_find_sh_04", &[(72, 58.3333)]),
    ("story/cut_story_rep_on_strip_garage", &[(73, 80.3333)]),
    ("story/cut_story_races_in_beach", &[(72, 41.8333)]),
    ("story/cut_story_hangout_pinkslip_hills", &[(72, 69.0)]),
    ("story/cut_story_city_champ", &[(73, 38.0)]),
    ("story/cut_story_hollywood_garage_intro", &[(73, 63.0)]),
    ("story/cut_story_race_gh_06", &[(73, 57.6333), (74, 64.1667)]),
    ("story/cut_story_beat_sh_02_exotic", &[(73, 35.2333)]),
];

/// An event's time as Events_LaunchEvent files it (`F-73781`): countdown (58), bookmark_cut_to_game (72)
/// and cut_to_game (74) at or past the camera clip's length (its header +12, `max(len, 0.1)`) are moved 0.1 s earlier.
fn mark_time(id: u8, t: f32, cam_len: f32) -> f32 {
    // (fsel: len - 0.1 >= 0 ? len : 0.1)
    if matches!(id, 58 | 72 | 74) && t >= if cam_len - 0.1 >= 0.0 { cam_len } else { 0.1 } {
        t - 0.1
    } else {
        t
    }
}

/// Where a skip takes a story scene (the cine manager's skip request +305: F-19408 -> F-61676):
/// the earliest bookmark / bookmark_cut_to_game / countdown of the scene, if the clock is not past it yet
/// (the clock jumps there and the scene plays on; with none, or past it, the skip does nothing).
pub fn skip_to(scene: &str, t: f32, cam_len: f32) -> Option<f32> {
    let marks = STORY_MARKS.iter().find(|m| m.0.eq_ignore_ascii_case(scene))?.1;
    let at = marks.iter().filter(|m| matches!(m.0, 58 | 72 | 73)).map(|m| mark_time(m.0, m.1, cam_len)).fold(f32::INFINITY, f32::min);
    (at.is_finite() && at > t).then_some(at)
}

/// When a story scene hands the picture to the game camera (bookmark_cut_to_game / cut_to_game: F-45212,
/// a hard cut); the scene's clock and cars run on to its end.
pub fn cut_to_game(scene: &str, cam_len: f32) -> Option<f32> {
    let marks = STORY_MARKS.iter().find(|m| m.0.eq_ignore_ascii_case(scene))?.1;
    marks.iter().filter(|m| matches!(m.0, 72 | 74)).map(|m| mark_time(m.0, m.1, cam_len)).reduce(f32::min)
}

/// The car movers that stand for the player's own car.
pub fn is_player_car(label: &str) -> bool {
    matches!(label, "drv00" | "player" | "player_drv00" | "playergarage" | "delivery")
}

/// The driver sets Boot.sco registers with a cutscene copy (CineScript_Characters_RegisterDriverMappings, its sixth
/// argument: "<set>_HackedScale" - native F-67461, the map at the manager's +116 / +128). The copy is the same
/// figure about 7.6 % taller, the rig the scenes' face packs were made on; every other set plays as itself.
const HACKED_SCALE: [&str; 14] = [
    "drv_fc_002_set", "drv_fc_003_set", "drv_ma_001_set", "drv_ma_03_set", "drv_mb_001_set", "drv_mb_008_set", "drv_mb_04_set",
    "drv_mc_001_set", "drv_mc_003_set", "drv_mc_02_set", "drv_mh_006_set", "drv_mh_02_set", "drv_mp_01_set", "mec_mc_01_set",
];

/// The officers' face table of LaunchAnimEventWithFace (F-45542): `cut_cop_faces/Entity/<n>/<n>` by the first
/// entry containing the slot name.
const COP_FACES: [&str; 5] = [
    "black_face_pol_mb_01_set_drv02",
    "caumust_face_pol_mc_01_set_drv03",
    "cauhair_face_pol_mc_02_set_drv04",
    "caubuzz_face_pol_mc_03_set_drv05",
    "mexmust_face_pol_mc_04_set_drv06",
];

/// The cine manager's auxiliary pack (+364) while a scene plays: CopArrest.sco loads `cut_cop_faces` for its arrest
/// scenes (CopPullover's `cut_cop_alert_02` holds no faces, so the fallback finds nothing there).
fn aux_pack(scene: &str) -> Option<&'static str> {
    scene.starts_with("cut_cop_arrest").then_some("cut_cop_faces")
}

/// A vector track (kind, bone 0) of a scene clip at time `t`, sampled as F-15435 (`drvanim::sample_dof`).
fn dof_vec(a: &Animation, kind: u8, t: f32) -> Option<Vec3> {
    crate::drvanim::sample_dof(a, kind, 0, DOF_VECTOR3, t).map(|v| Vec3::new(v[0], v[1], v[2]))
}

/// A rotation track (kind, bone 0) of a scene clip at time `t`, sampled as F-15435.
fn dof_rot(a: &Animation, kind: u8, t: f32) -> Option<Quat> {
    crate::drvanim::sample_dof(a, kind, 0, DOF_QUATERNION, t).map(Quat::from_array)
}

/// A scene mover's place at time `t`: its mover track and its root's, each sampled as F-15435 (a missing one: zero / identity).
fn mover_at(a: &Animation, t: f32) -> (Vec3, Quat) {
    let p = dof_vec(a, KIND_MOVER_TRANSLATION, t).unwrap_or(Vec3::ZERO) + dof_vec(a, KIND_TRANSLATION, t).unwrap_or(Vec3::ZERO);
    let r = dof_rot(a, KIND_MOVER_ROTATION, t).unwrap_or(Quat::IDENTITY) * dof_rot(a, KIND_ROTATION, t).unwrap_or(Quat::IDENTITY);
    (p, r)
}

/// A scene character's body at clip time `t` as the original's clip sampler fills its frame (F-47152 -> F-12216: the block,
/// key and fraction of `sample_block`, each DOF from the block's own track, the mapped loop unless the character's frame
/// layout is the clip's) and the finish writes it: only the DOFs the character's frame has (the skeleton data's +27 / +26),
/// a bone without a value keeps its default local.
fn pose_at(a: &Animation, m: &SkinModel, t: f32, out: &mut Vec<(Quat, Vec3)>) {
    let mapped = a.layout != m.layout;
    out.clear();
    for (i, b) in m.bones.iter().enumerate() {
        let id = m.ids.get(i).copied().unwrap_or(0);
        let (has_pos, has_rot) = m.dofs.get(i).copied().unwrap_or((true, true));
        let r = has_rot.then(|| crate::drvanim::sample_dof_in(a, KIND_ROTATION, id, DOF_QUATERNION, t, mapped)).flatten().map_or(b.rot, Quat::from_array);
        let p = has_pos.then(|| crate::drvanim::sample_dof_in(a, KIND_TRANSLATION, id, DOF_VECTOR3, t, mapped)).flatten().map_or(b.pos, |v| Vec3::new(v[0], v[1], v[2]));
        out.push((r, p));
    }
}

/// A scene character's mover (its place in the scene) at clip time `t`, sampled as F-15435.
fn mover_of(a: &Animation, t: f32) -> (Vec3, Quat) {
    (dof_vec(a, KIND_MOVER_TRANSLATION, t).unwrap_or(Vec3::ZERO), dof_rot(a, KIND_MOVER_ROTATION, t).unwrap_or(Quat::IDENTITY))
}

/// The set a cutscene draws for `set` (LookupDriverToCutscenePath / Name, F-58832 / F-81189): the character manager's
/// map +116 (the HackedScale column of Boot.sco's RegisterDriverMappings), and when the local player's car is a motorcycle
/// (F-17373, which with a player there stands for the script's own argument) the map +140 (the jacket column) - in the 45
/// mappings they differ only for the player's `drv_mp_01_set`: `drv_mp_jacket_01_set_hackedscale` on a bike. Every slot: the
/// core keeps the plain set only for a record tied to a racer whose slot is marked unscaled (the byte F-99281 reads), and no shipped
/// scene marks one (the marks come from the generic casts, whose records have no racer; FinishScene clears them) - checked
/// over all 315 scene scripts, docs/reference/cutscenes/unscaled_slot.md.
fn cutscene_set(set: &str, bike: bool) -> String {
    if bike && set.eq_ignore_ascii_case("drv_mp_01_set") {
        return "drv_mp_jacket_01_set_hackedscale".to_string();
    }
    if HACKED_SCALE.iter().any(|s| s.eq_ignore_ascii_case(set)) {
        format!("{}_hackedscale", set.to_ascii_lowercase())
    } else {
        set.to_string()
    }
}

/// The officers' sets of CineScript_Characters_AssignCopToSlot (F-61680, its table).
const COPS: [&str; 5] = ["pol_mb_01_set", "pol_mc_01_set", "pol_mc_02_set", "pol_mc_03_set", "pol_mc_04_set"];
/// Its counter: one for the whole game, never reset, stepped before each officer it places.
static COP_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Character set of a scene entity that does not name one.
/// The generic cast's lists (AssignBoyToSlot F-40835: tables 0x827EA804 / 0x827EA820; AssignGirlToSlot F-49041: 0x827EA848 /
/// 0x827EA854): slot 0 takes a driver set, any other slot a pedestrian set.
const CAST_BOY_DRV: [&str; 7] = ["drv_mb_009_set", "drv_mb_02_set", "drv_mb_05_set", "drv_mh_01_set", "drv_mb_010_set", "drv_mh_04_set", "drv_ma_02_set"];
const CAST_BOY_PED: [&str; 10] = ["ped_ma_004_set", "ped_mh_002_set", "ped_mh_003_set", "ped_mb_004_set", "ped_mc_004_set", "ped_mc_001_set", "ped_mb_001_set", "ped_mb_002_set", "ped_mh_001_set", "ped_mh_004_set"];
const CAST_GIRL_DRV: [&str; 3] = ["drv_fb_01_set", "drv_fa_001_set", "drv_fc_003_set"];
const CAST_GIRL_PED: [&str; 6] = ["ped_fb_002_set", "ped_fb_005_set", "ped_fa_002_set", "ped_fh_001_set", "ped_fc_002_set", "ped_fa_001_set"];
/// The four counters of the generic cast (boy driver, girl driver, boy pedestrian, girl pedestrian): the session's, from 0
static CAST_COUNTERS: std::sync::Mutex<[i32; 4]> = std::sync::Mutex::new([0; 4]);

/// One AssignBoyToSlot / AssignGirlToSlot (F-40835 / F-49041): the counter is pre-incremented and picks from its list; on
/// slot 0 a set an active racer already wears (F-68164, `in_use`) moves it on, at most twice (the third pick is not tested).
/// (the counters never go negative in a session; `rem_euclid` stands for the guest's signed remainder on the positive side)
fn cast_pick(girl: bool, slot: i32, ctr: &mut [i32; 4], in_use: impl Fn(&str) -> bool) -> &'static str {
    if slot == 0 {
        let (list, i): (&[&'static str], usize) = if girl { (&CAST_GIRL_DRV, 1) } else { (&CAST_BOY_DRV, 0) };
        let n = list.len() as i32;
        let mut c = ctr[i].wrapping_add(1);
        let mut set = list[c.rem_euclid(n) as usize];
        let mut tries = 0;
        while in_use(set) {
            c = c.wrapping_add(1);
            tries += 1;
            set = list[c.rem_euclid(n) as usize];
            if tries >= 2 {
                break;
            }
        }
        ctr[i] = c;
        set
    } else {
        let (list, i): (&[&'static str], usize) = if girl { (&CAST_GIRL_PED, 3) } else { (&CAST_BOY_PED, 2) };
        let c = ctr[i].wrapping_add(1);
        ctr[i] = c;
        list[c.rem_euclid(list.len() as i32) as usize]
    }
}

/// A scene script's generic cast in its order (`cinecast::CASTS`): the set each named entity gets, with the session's counters.
pub fn cast_scene(scene: &str, in_use: impl Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    let Some((_, calls)) = crate::cinecast::CASTS.iter().find(|(s, _)| s.eq_ignore_ascii_case(scene)) else { return Vec::new() };
    let mut ctr = CAST_COUNTERS.lock().unwrap();
    calls.iter().map(|&(girl, slot, entity)| (entity, cast_pick(girl, slot, &mut ctr, &in_use))).collect()
}

fn default_set(scene: &str, entity: &str, n: usize) -> &'static str {
    const RACERS: [&str; 4] = ["drv_mb_02_set", "drv_mc_02_set", "drv_ma_02_set", "drv_mh_02_set"];
    let player = entity.starts_with("player") || entity == "cut_drv00";
    // (CopPullover.sco names its officer itself, AssignNameToSlot("pol_mc_01_set", 0) before the walk and again
    // before the ticket scene: one officer in every pull-over, the arrests' counter untouched)
    const PULLOVER: [&str; 5] = ["cut_cop_walk_02", "cut_cop_fidget_02", "sc_cut_cop_ticket_suv", "cut_cop_fidget_bike", "cut_cop_alert_02"];
    if !player && PULLOVER.iter().any(|s| scene.ends_with(s)) {
        "pol_mc_01_set"
    } else if entity.contains("cop") || (scene.contains("cut_cop") && !player) {
        // in the police scenes everybody but the player is an officer: the next of the five (F-61680:
        // ++counter % 5 - the first officer ever is pol_mc_01)
        COPS[(COP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1) % COPS.len()]
    } else if entity.ends_with("_f") {
        "drv_fa_001_set"
    } else if player {
        "drv_mp_01_set"
    } else {
        RACERS[n % RACERS.len()]
    }
}

/// A character set's first `.r6` read (on a worker thread: `read_ahead_ambient`, `read_ahead_scene`).
fn read_set(dir: &Path) -> Option<(PathBuf, mcla_formats::rage::character::Character)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "r6").unwrap_or(false)).collect()).unwrap_or_default();
    files.sort();
    let f = files.into_iter().next()?;
    let c = SkinModel::read(&f)?;
    Some((f, c))
}

/// "story/cut_intro/Entity/x/x" -> "story/cut_intro"
fn scene_of(entry: &str) -> String {
    let l = entry.to_ascii_lowercase();
    let cut = l.find("/entity/").or_else(|| l.find("/cscamera")).unwrap_or(l.len());
    l[..cut].to_string()
}

impl Cine {
    pub fn load(data_dir: &str) -> Cine {
        let root = Path::new(data_dir);
        let mut c = Cine { char_dir: root.join("resources/character"), ..Default::default() };
        // which pack holds what: read once, then kept in a small index file
        let index = root.join("cine_index.tsv");
        let mut lines: Vec<(String, PathBuf)> = Vec::new();
        if let Ok(t) = std::fs::read_to_string(&index) {
            for line in t.lines() {
                if let Some((k, f)) = line.split_once('\t') {
                    lines.push((k.to_string(), PathBuf::from(f)));
                }
            }
        }
        if lines.is_empty() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("resources/animation/cineanimpacks")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            files.sort();
            let mut out = String::new();
            for f in files {
                let Ok(res) = load_resource_file(&f) else { continue };
                for (_, name) in pack_entries(&res) {
                    out.push_str(&format!("{}\t{}\n", name, f.display()));
                    lines.push((name, f.clone()));
                }
            }
            let _ = std::fs::write(&index, out);
        }
        for (name, f) in lines {
            c.index.entry(scene_of(&name)).or_default().push((name, f));
        }
        log::info!("cine: {} scenes", c.index.len());
        c
    }

    pub fn has(&self, scene: &str) -> bool {
        self.index.contains_key(&scene.to_ascii_lowercase())
    }

    /// Scenes whose name starts with `prefix` (sorted).
    pub fn scenes(&self, prefix: &str) -> Vec<String> {
        let p = prefix.to_ascii_lowercase();
        let mut v: Vec<String> = self.index.keys().filter(|k| k.starts_with(&p)).cloned().collect();
        v.sort();
        v
    }

    /// The hangouts' people's character sets (`set_ambient`) read on worker threads before the player reaches a
    /// hangout, so making them there does not stop the game; their models are made on first use as before
    /// (docs/PERFORMANCE.md 2026-10-23 (6)).
    pub fn read_ahead_ambient(&mut self) {
        for set in AMBIENT_FEMALE.iter().chain(AMBIENT_MALE.iter()).chain(AMBIENT_DRIVERS.iter()) {
            if self.models.contains_key(*set) || self.reading.contains_key(*set) {
                continue;
            }
            let dir = self.char_dir.join(set);
            if let Ok(h) = std::thread::Builder::new().name("people read".into()).spawn(move || read_set(&dir)) {
                self.reading.insert(set.to_string(), h);
            }
        }
    }

    /// A story scene's animation packs and its people's character sets read on worker threads before it starts, so
    /// that `start` finds them read (what it makes and when is unchanged - docs/PERFORMANCE.md 2026-10-25). Returns
    /// the labels of the cars it moves (`mover_<label>_car`), for the caller to read their cars ahead.
    pub fn read_ahead_scene(&mut self, scene: &str) -> Vec<String> {
        let key = scene.to_ascii_lowercase();
        let short = key.rsplit('/').next().unwrap_or("").to_string();
        let mut movers = Vec::new();
        let mut sets = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        let aux = aux_pack(&key).map(str::to_string);
        for (part, faces) in [(key.clone(), false), (format!("hangout/{short}_peds"), false), (format!("{key}_im"), true)].into_iter().chain(aux.map(|a| (a, true))) {
            for (name, f) in self.index.get(&part).cloned().unwrap_or_default() {
                if !files.contains(&f) && !self.packs.contains_key(&f) {
                    files.push(f);
                }
                let l = name.to_ascii_lowercase();
                let ent = l.rsplit('/').next().unwrap_or("").to_string();
                if faces || l.ends_with("/cscamera") {
                    continue;
                }
                if let Some(label) = ent.strip_prefix("mover_").and_then(|r| r.strip_suffix("_car")) {
                    movers.push(label.to_string());
                    continue;
                }
                if ent.ends_with("_car") {
                    continue;
                }
                // (as `start` names them; an entity without a set of its own gets `default_set`'s by its place among
                // the actors, which only `start` knows. In the police scenes: every set the officers' counter could give it,
                // the five - docs/PERFORMANCE.md 2026-10-27 (11))
                if let Some((_, s)) = ent.split_once("_name_") {
                    sets.push(cutscene_set(s, self.player_bike));
                } else if key.contains("cut_cop") {
                    for c in COPS {
                        let s = cutscene_set(c, self.player_bike);
                        if !sets.contains(&s) {
                            sets.push(s);
                        }
                    }
                }
            }
        }
        for f in files {
            let p = f.clone();
            if let Ok(h) = std::thread::Builder::new().name("scene read".into()).spawn(move || load_resource_file(&p).ok()) {
                self.packs.insert(f, h);
            }
        }
        for set in sets {
            if self.models.contains_key(&set) || self.reading.contains_key(&set) {
                continue;
            }
            let dir = self.char_dir.join(&set);
            if let Ok(h) = std::thread::Builder::new().name("people read".into()).spawn(move || read_set(&dir)) {
                self.reading.insert(set, h);
            }
        }
        log::info!("cine: reading {} ahead ({} cars)", key, movers.len());
        movers
    }

    /// An animation pack: the one read ahead (`read_ahead_scene`), else read now.
    fn pack(&mut self, f: &Path) -> Option<mcla_formats::rsc::Resource> {
        match self.packs.remove(f) {
            Some(h) => h.join().ok().flatten(),
            None => load_resource_file(f).ok(),
        }
    }

    fn model(&mut self, gpu: &mut Gpu, set: &str) -> Option<Arc<SkinModel>> {
        if let Some(h) = self.reading.remove(set).filter(|_| !self.models.contains_key(set)) {
            let m = h.join().ok().flatten().and_then(|(f, c)| SkinModel::from_character(gpu, set, &f, c)).map(Arc::new);
            if m.is_none() {
                log::warn!("cine: could not load {}", set);
            }
            self.models.insert(set.to_string(), m);
        }
        if !self.models.contains_key(set) {
            let dir = self.char_dir.join(set);
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "r6").unwrap_or(false)).collect()).unwrap_or_default();
            files.sort();
            let m = files.first().and_then(|f| SkinModel::load(gpu, set, f)).map(Arc::new);
            if m.is_none() {
                log::warn!("cine: could not load {}", set);
            }
            self.models.insert(set.to_string(), m);
        }
        self.models[set].clone()
    }

    /// Load a scene and start it at `origin` (identity for story scenes).
    pub fn start(&mut self, gpu: &mut Gpu, scene: &str, origin: Mat4) -> bool {
        self.held = None;
        let key = scene.to_ascii_lowercase();
        if !self.index.contains_key(&key) {
            return false;
        }
        let mut sc = Scene { name: key.clone(), camera: None, actors: Vec::new(), movers: Vec::new(), duration: 0.0, t: 0.0, t_prev: 0.0, origin, camera_killed: false, cam_cut: Default::default() };
        // the scene itself, its bystanders, then the faces
        let short = key.rsplit('/').next().unwrap_or("").to_string();
        let parts = [(key.clone(), false), (format!("hangout/{short}_peds"), false), (format!("{key}_im"), true)];
        let mut got: HashMap<PathBuf, Arc<mcla_formats::rsc::Resource>> = HashMap::new();
        // (each character's slot name, the set before its cutscene copy: the face fallback matches it)
        let mut slot_sets: Vec<(String, String)> = Vec::new();
        // (the scene script's AssignBoyToSlot / AssignGirlToSlot calls, run once at its start in their order)
        let cast = cast_scene(&key, |set| self.racer_sets.iter().any(|r| r == set));
        for (part, faces) in parts {
            let Some(entries) = self.index.get(&part).cloned() else { continue };
            let mut by_file: HashMap<PathBuf, Vec<String>> = HashMap::new();
            for (name, f) in entries {
                by_file.entry(f).or_default().push(name);
            }
            for (f, names) in by_file {
                // (the scene and its faces are mostly in one pack: read once)
                let res = match got.get(&f) {
                    Some(r) => r.clone(),
                    None => {
                        let Some(r) = self.pack(&f) else { continue };
                        let r = Arc::new(r);
                        got.insert(f.clone(), r.clone());
                        r
                    }
                };
                let rd = Rd::new(&res);
                for (o, name) in pack_entries(&res) {
                    if !names.contains(&name) {
                        continue;
                    }
                    let Some(anim) = animation_named(&rd, o) else {
                        log::warn!("cine: {} does not decode", name);
                        continue;
                    };
                    let anim = Arc::new(anim);
                    let l = name.to_ascii_lowercase();
                    // ".../Entity/<entity>/<entity>"
                    let ent = l.rsplit('/').next().unwrap_or("").to_string();
                    if faces {
                        if let Some((label, _)) = ent.split_once("_name_") {
                            if let Some(a) = sc.actors.iter_mut().find(|a| a.label == label) {
                                a.face = Some(Clip::bind(&anim, &a.model.ids, false, false));
                                if let Some(m) = a.model.morph.as_ref() {
                                    let tracks = m.shapes.targets.iter().map(|t| anim.tracks.iter().position(|k| k.kind == KIND_BLEND_SHAPE && k.bone_id == t.id)).collect();
                                    a.shapes = Some((FaceShapes::new(m), tracks));
                                }
                            }
                        }
                        continue;
                    }
                    sc.duration = sc.duration.max(anim.duration);
                    if l.ends_with("/cscamera") {
                        sc.camera = Some(anim);
                        continue;
                    }
                    if let Some(rest) = ent.strip_prefix("mover_") {
                        if let Some(label) = rest.strip_suffix("_car") {
                            sc.movers.push(Mover { label: label.to_string(), anim });
                            continue;
                        }
                    }
                    if ent.ends_with("_car") {
                        continue;
                    }
                    let (label, set) = match ent.split_once("_name_") {
                        Some((l, s)) => (l, s.to_string()),
                        None => (ent.as_str(), cast.iter().find(|c| c.0.eq_ignore_ascii_case(&ent)).map_or_else(|| default_set(&key, &ent, sc.actors.len()), |c| c.1).to_string()),
                    };
                    // (props - the arrests' pistols and shotguns, prp_handgun_001_set / prp_shotgun_001_set - are
                    // characters of the scene like the others, with their own root path next to their officer: the
                    // scene script attaches nothing to a bone; GitHub #17)
                    // (the cutscene loader script, 0x95ca1d1b 0x4f - 0x1d8: a slot not marked for the unscaled
                    // character takes its set's cutscene copy - LookupDriverToCutscenePath / Name)
                    slot_sets.push((label.to_string(), set.clone()));
                    let set = cutscene_set(&set, self.player_bike);
                    let Some(model) = self.model(gpu, &set) else { continue };
                    let clip = Clip::bind(&anim, &model.ids, false, false);
                    sc.actors.push(Actor { label: label.to_string(), model, clip, t0: 0.0, face: None, shapes: None, root_add: None, fade: None, face_in: None });
                }
            }
        }
        // (Characters_LaunchAnimEventWithFace's face fallback, F-45542: a character with no face in the scene's pack
        // whose slot name contains "pol" takes, from the auxiliary pack, the first entry of the officers' face table
        // containing its slot name - `CUT_COP_FACES/ENTITY/<n>/<n>`; played as a scene face, from 0 on the scene clock)
        if let Some(aux) = aux_pack(&key) {
            let entries = self.index.get(aux).cloned().unwrap_or_default();
            for a in sc.actors.iter_mut().filter(|a| a.face.is_none()) {
                let Some((_, set)) = slot_sets.iter().find(|s| s.0 == a.label).filter(|s| s.1.contains("pol")) else { continue };
                let Some(n) = COP_FACES.iter().find(|n| n.contains(set.as_str())) else { continue };
                let want = format!("{aux}/entity/{n}/{n}");
                let Some((name, f)) = entries.iter().find(|e| e.0.eq_ignore_ascii_case(&want)).cloned() else {
                    log::warn!("cine: could not find face anim {}", want);
                    continue;
                };
                let res = match got.get(&f) {
                    Some(r) => r.clone(),
                    None => {
                        let Some(r) = self.pack(&f) else { continue };
                        let r = Arc::new(r);
                        got.insert(f.clone(), r.clone());
                        r
                    }
                };
                let rd = Rd::new(&res);
                let Some(anim) = pack_entries(&res).into_iter().find(|e| e.1 == name).and_then(|(o, _)| animation_named(&rd, o)).map(Arc::new) else { continue };
                a.face = Some(Clip::bind(&anim, &a.model.ids, false, false));
                if let Some(m) = a.model.morph.as_ref() {
                    let tracks = m.shapes.targets.iter().map(|t| anim.tracks.iter().position(|k| k.kind == KIND_BLEND_SHAPE && k.bone_id == t.id)).collect();
                    a.shapes = Some((FaceShapes::new(m), tracks));
                }
            }
        }
        if sc.camera.is_none() && sc.actors.is_empty() && sc.movers.is_empty() {
            return false;
        }
        sc.actors.sort_by(|a, b| a.label.cmp(&b.label));
        log::info!("cine: {} ({:.1} s): camera {}, {} characters [{}] ({} with faces, {} with blend shapes: {} weight tracks), cars [{}]", key, sc.duration, sc.camera.is_some(), sc.actors.len(), sc.actors.iter().map(|a| a.label.as_str()).collect::<Vec<_>>().join(" "), sc.actors.iter().filter(|a| a.face.is_some()).count(), sc.actors.iter().filter(|a| a.shapes.is_some()).count(), sc.actors.iter().filter_map(|a| a.shapes.as_ref()).map(|s| s.1.iter().flatten().count()).sum::<usize>(), sc.movers.iter().map(|m| m.label.as_str()).collect::<Vec<_>>().join(" "));
        self.playing = Some(sc);
        true
    }

    /// Show the loops of a hangout ("hangout/cut_hangout_01"), or none.
    pub fn set_ambient(&mut self, gpu: &mut Gpu, name: Option<&str>) {
        let name = name.unwrap_or("").to_ascii_lowercase();
        if name == self.ambient_name {
            return;
        }
        self.ambient.clear();
        self.ambient_name = name.clone();
        if name.is_empty() {
            return;
        }
        const FEMALE: [&str; 4] = AMBIENT_FEMALE;
        const MALE: [&str; 5] = AMBIENT_MALE;
        const DRIVERS: [&str; 4] = AMBIENT_DRIVERS;
        let t0 = std::time::Instant::now();
        let mut packs: std::collections::HashMap<std::path::PathBuf, _> = std::collections::HashMap::new();
        for (k, key) in self.scenes(&format!("{name}/")).into_iter().enumerate() {
            let Some((entry, f)) = self.index.get(&key).and_then(|e| e.first().cloned()) else { continue };
            // (the scenes of a hangout share their packs: each read once - docs/PERFORMANCE.md 2026-10-23 (5))
            if !packs.contains_key(&f) {
                let Ok(res) = load_resource_file(&f) else { continue };
                packs.insert(f.clone(), res);
            }
            let res = &packs[&f];
            let rd = Rd::new(res);
            let Some((o, _)) = pack_entries(res).into_iter().find(|(_, n)| *n == entry) else { continue };
            let Some(anim) = animation_named(&rd, o) else { continue };
            let label = key.rsplit('/').next().unwrap_or("").to_string();
            // the racers' skeleton has more bones than the pedestrians'
            let set = if anim.tracks.len() > 80 {
                DRIVERS[k % DRIVERS.len()]
            } else if label.starts_with('f') {
                FEMALE[k % FEMALE.len()]
            } else {
                MALE[k % MALE.len()]
            };
            let Some(model) = self.model(gpu, set) else { continue };
            let clip = Clip::bind(&Arc::new(anim), &model.ids, false, false);
            self.ambient.push(Actor { label, model, clip, t0: 0.0, face: None, shapes: None, root_add: None, fade: None, face_in: None });
        }
        log::info!("cine: {} people at {} ({:.0} ms)", self.ambient.len(), name, t0.elapsed().as_secs_f32() * 1000.0);
    }

    /// A game tick starts: the times the drawn frames blend from (`Scene::draw_t`).
    pub fn begin_tick(&mut self) {
        self.clock_prev = self.clock;
        for sc in self.playing.iter_mut().chain(self.linger.iter_mut()) {
            sc.t_prev = sc.t;
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.clock += dt;
        if let Some(sc) = self.linger.as_mut() {
            sc.t += dt;
            if sc.t >= sc.duration {
                self.linger = None;
            }
        }
    }

    /// The scene hands the camera back to the game but its people play on to the clip's end.
    pub fn release(&mut self) {
        self.linger = self.playing.take().filter(|sc| !sc.actors.is_empty() && sc.t < sc.duration);
    }

    /// The lingering people go too (the race is over or another scene begins).
    pub fn drop_linger(&mut self) {
        self.linger = None;
    }

    /// The labels of a scene's car movers.
    pub fn mover_labels(&self, scene: &str) -> Vec<String> {
        let Some(entries) = self.index.get(&scene.to_ascii_lowercase()) else { return Vec::new() };
        entries.iter().filter_map(|(name, _)| name.to_ascii_lowercase().rsplit('/').next().and_then(|e| e.strip_prefix("mover_")).and_then(|r| r.strip_suffix("_car")).map(|l| l.to_string())).collect()
    }

    /// Where a scene puts a car (first or last frame), without playing it: the garage
    /// scenes know where the car stands inside, the hangout scenes where one pulls in.
    pub fn car_pose(&self, scene: &str, want: &dyn Fn(&str) -> bool, end: bool) -> Option<(Vec3, Quat)> {
        let entries = self.index.get(&scene.to_ascii_lowercase())?;
        for (name, f) in entries {
            let l = name.to_ascii_lowercase();
            let ent = l.rsplit('/').next().unwrap_or("");
            let Some(label) = ent.strip_prefix("mover_").and_then(|r| r.strip_suffix("_car")) else { continue };
            if !want(label) {
                continue;
            }
            let res = load_resource_file(f).ok()?;
            let rd = Rd::new(&res);
            let (o, _) = pack_entries(&res).into_iter().find(|(_, n)| n == name)?;
            let a = animation_named(&rd, o)?;
            let t = if end { a.duration } else { 0.0 };
            let (p, r) = mover_at(&a, t);
            return Some((p, r.normalize()));
        }
        None
    }

    /// Start a scene around the player's car (`car` = its world transform).
    pub fn start_bound(&mut self, gpu: &mut Gpu, scene: &str, car: Mat4) -> bool {
        if !self.start(gpu, scene, Mat4::IDENTITY) {
            return false;
        }
        let label = self.movers().into_iter().find(|l| is_player_car(l));
        let local = label.and_then(|l| self.mover(&l)).map(|(p, r)| Mat4::from_rotation_translation(r, p)).unwrap_or(Mat4::IDENTITY);
        if let Some(sc) = self.playing.as_mut() {
            sc.origin = car * local.inverse();
        }
        true
    }

    /// CineScript_Characters_ReAnimateSlot: the playing scene's character `label` plays `entity`'s clip of another
    /// scene's pack from now on (CopPullover: the officer's `cut_cop_alert_02/Entity/cut_drv01` when the suspect bolts).
    pub fn reanimate(&mut self, label: &str, scene: &str, entity: &str) -> bool {
        let Some(entries) = self.index.get(&scene.to_ascii_lowercase()).cloned() else { return false };
        let want = format!("/{}", entity.to_ascii_lowercase());
        let Some((name, f)) = entries.into_iter().find(|(n, _)| n.to_ascii_lowercase().ends_with(&want)) else { return false };
        let Some(res) = self.pack(&f) else { return false };
        let rd = Rd::new(&res);
        let Some(anim) = pack_entries(&res).into_iter().find(|(_, n)| *n == name).and_then(|(o, _)| animation_named(&rd, o)) else { return false };
        let Some(sc) = self.playing.as_mut() else { return false };
        let Some(a) = sc.actors.iter_mut().find(|a| a.label == label) else { return false };
        // (CineScript_Characters_ReAnimateSlot, F-56094 -> F-52646: the new clip from 0 added on the root bone to
        // the old clip frozen at its time with its root height 0, cross-faded in over 0.25 s; played once)
        let mut old = Vec::new();
        a.pose(sc.t - a.t0, sc.t, &mut old);
        a.root_add = old.first().map(|l| (l.0, Vec3::new(l.1.x, 0.0, l.1.z)));
        a.fade = Some((old, sc.t));
        a.clip = Clip::bind(&Arc::new(anim), &a.model.ids, false, false);
        a.t0 = sc.t;
        a.face = None;
        true
    }

    /// CineScript_Camera_Kill: the playing scene goes on under the game camera.
    pub fn kill_camera(&mut self) {
        if let Some(sc) = self.playing.as_mut() {
            sc.camera_killed = true;
        }
    }

    pub fn stop(&mut self) {
        self.held = None;
        self.playing = None;
        self.buf.clear();
    }

    /// A scene's camera at scene time `t` without starting it (the race-start warp flies to the start
    /// scene's camera at t 1.0 before the scene plays - `F-62707`): (eye, forward, up, vertical
    /// field of view in radians) through `origin`.
    pub fn peek_camera(&self, scene: &str, origin: Mat4, t: f32) -> Option<(Vec3, Vec3, Vec3, f32)> {
        let key = scene.to_ascii_lowercase();
        for (name, f) in self.index.get(&key)? {
            if !name.to_ascii_lowercase().ends_with("/cscamera") {
                continue;
            }
            let Ok(res) = load_resource_file(f) else { continue };
            let rd = Rd::new(&res);
            for (o, n) in pack_entries(&res) {
                if n != *name {
                    continue;
                }
                let anim = animation_named(&rd, o)?;
                let sc = Scene { name: key.clone(), camera: Some(Arc::new(anim)), actors: Vec::new(), movers: Vec::new(), duration: 0.0, t, t_prev: t, origin, camera_killed: false, cam_cut: Default::default() };
                return Self::scene_camera(&sc);
            }
        }
        None
    }

    /// Camera at the scene's time: (eye, forward, up, vertical field of view in radians).
    pub fn camera(&self) -> Option<(Vec3, Vec3, Vec3, f32)> {
        if self.held.is_some() && self.playing.is_some() {
            return self.held;
        }
        if self.at_game_camera() {
            return None;
        }
        Self::scene_camera(self.playing.as_ref()?)
    }

    /// The camera a drawn frame shows: `camera` drawn between the camera times of the last two updates by the render alpha `a`
    /// (`Scene::draw_t`; the port's interpolation, not the original's). Across a cut (the two ends as far apart as the cut
    /// test's metres or degrees) or a jump in time it is the latest update's, as `camera`.
    pub fn draw_camera(&self, a: f32) -> Option<(Vec3, Vec3, Vec3, f32)> {
        let now = self.camera()?;
        if self.held.is_some() {
            return Some(now);
        }
        let Some(sc) = self.playing.as_ref() else { return Some(now) };
        let Some(cam) = sc.camera.as_ref() else { return Some(now) };
        let (s, memo) = *sc.cam_cut.lock().unwrap_or_else(|e| e.into_inner());
        // (the memo is the latest update's (scene time, camera time); `last2` the update's before)
        let (Some((at, t1)), t0) = (memo, s.last2) else { return Some(now) };
        if at != sc.t || !(0.0..=0.1).contains(&(t1 - t0)) || a >= 1.0 {
            return Some(now);
        }
        let Some(before) = Self::camera_at(sc, cam, t0) else { return Some(now) };
        let turn = before.1.angle_between(now.1).max(before.2.angle_between(now.2));
        if before.0.distance(now.0) > CAM_CUT_METRES || turn > CAM_CUT_DEGREES * 0.017_453_292 {
            return Some(now);
        }
        Self::camera_at(sc, cam, (a - 1.0).mul_add(t1 - t0, t1))
    }

    /// A story scene past its cut_to_game: the game camera shows the rest (`STORY_MARKS`).
    pub fn at_game_camera(&self) -> bool {
        self.playing.as_ref().map_or(false, |sc| sc.camera_killed || cut_to_game(&sc.name, sc.camera_len()).map_or(false, |c| sc.t >= c))
    }

    fn scene_camera(sc: &Scene) -> Option<(Vec3, Vec3, Vec3, f32)> {
        let cam = sc.camera.as_ref()?;
        // (the scene camera update F-40303 runs the cut test once per update, its enable byte set around the call)
        let mut state = sc.cam_cut.lock().unwrap_or_else(|e| e.into_inner());
        let (mut cut, memo) = *state;
        let t = match memo {
            Some((at, snapped)) if at == sc.t => snapped,
            _ => {
                let pos = |s: f32| dof_vec(cam, KIND_TRANSLATION, s);
                let rot = |s: f32| dof_rot(cam, KIND_ROTATION, s);
                let snapped = cam_cut_rule(&mut cut, true, sc.t, CAM_CUT_METRES, CAM_CUT_DEGREES, pos, rot);
                *state = (cut, Some((sc.t, snapped)));
                snapped
            }
        };
        Self::camera_at(sc, cam, t)
    }

    /// The scene camera at a (snapped) camera time.
    fn camera_at(sc: &Scene, cam: &Animation, t: f32) -> Option<(Vec3, Vec3, Vec3, f32)> {
        let pos = dof_vec(cam, KIND_TRANSLATION, t)?;
        // (the scene camera update F-40303 starts the matrix as identity each update; the lens
        // track is a focal length in mm on a 35 mm gate, F-25555 (cam_sample); the render
        // camera's vertical angle is that divided by the viewport's aspect, 16:9 (F-79297, the
        // warp's target, cam_to_menu F-14139). Without the track the camera keeps its reset
        // lens, pi/4.)
        let mut m = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let (mut h, mut k14) = (std::f32::consts::FRAC_PI_4, 1.0);
        let rot = dof_rot(cam, KIND_ROTATION, t).map(|q| q.to_array());
        cam_sample(&mut m, &mut h, &mut k14, Some(pos), rot, crate::drvanim::sample_dof(cam, KIND_FOV, 0, DOF_FLOAT, t).map(|v| v[0]), None);
        let fov = h / (16.0 / 9.0);
        // (F-40303 then levels the scene origin, F-84619, unless the cine manager's +228 is set (the arrest's frame
        // sets it): not needed here - every origin the port builds is yaw-only or identity, which it leaves bit for bit,
        // and the tilted arrest frame is the case it skips. Then the camera's rows x the origin.)
        let row = |i: usize| Vec3::new(m[4 * i], m[4 * i + 1], m[4 * i + 2]);
        Some((sc.origin.transform_point3(row(3)), sc.origin.transform_vector3(-row(2)).normalize_or(Vec3::Z), sc.origin.transform_vector3(row(1)).normalize_or(Vec3::Y), fov))
    }

    /// Labels of the cars the scene moves.
    /// Where the playing scene's characters stand now: (label, root bone in world space).
    pub fn actor_roots(&self) -> Vec<(String, Vec3)> {
        let Some(sc) = self.playing.as_ref() else { return Vec::new() };
        let mut local: Vec<(Quat, Vec3)> = Vec::new();
        sc.actors.iter().map(|a| {
            a.pose(sc.t - a.t0, sc.t, &mut local);
            let (mt, mr) = mover_of(&a.clip.anim, sc.t - a.t0);
            let root = local.first().map_or(Vec3::ZERO, |l| l.1);
            (a.label.clone(), (sc.origin * Mat4::from_rotation_translation(mr, mt)).transform_point3(root))
        }).collect()
    }

    pub fn movers(&self) -> Vec<String> {
        self.playing.as_ref().map(|s| s.movers.iter().map(|m| m.label.clone()).collect()).unwrap_or_default()
    }

    /// The animation that moves a car of the playing scene, and the scene's frame in the world.
    pub fn mover_anim(&self, label: &str) -> Option<(Arc<Animation>, Mat4)> {
        let sc = self.playing.as_ref()?;
        sc.movers.iter().find(|m| m.label == label).map(|m| (m.anim.clone(), sc.origin))
    }

    /// World position and orientation of a moved car at the scene's time.
    pub fn mover(&self, label: &str) -> Option<(Vec3, Quat)> {
        let sc = self.playing.as_ref()?;
        let m = sc.movers.iter().find(|m| m.label == label)?;
        let (p, r) = mover_at(&m.anim, sc.t);
        let (_, or, _) = sc.origin.to_scale_rotation_translation();
        Some((sc.origin.transform_point3(p), (or * r).normalize()))
    }

    /// The light manager's state for the character programs this frame (main.rs); None: the
    /// figures keep the generic look (MCLA_CHAR_OLD=1).
    pub fn set_light(&mut self, l: Option<crate::skin::CharLight>) {
        self.buf.char_light = l;
    }

    /// Skin the characters at the scene's time.
    /// `alpha`: the render alpha the cast is drawn at (`Scene::draw_t`; 1 = the tick's end).
    pub fn prepare(&mut self, gpu: &Gpu, alpha: f32) {
        self.buf.clear();
        let mut local: Vec<(Quat, Vec3)> = Vec::new();
        let steps = self.steps.map(|n| n + 1).or_else(|| std::env::var_os("MCLA_DRAW_STEPS").map(|_| 0));
        self.steps = steps;
        let d = self.clock - self.clock_prev;
        let clock = if (0.0..=0.1).contains(&d) { (alpha - 1.0).mul_add(d, self.clock) } else { self.clock };
        for (k, a) in self.ambient.iter().enumerate() {
            let last = a.clip.last_frame();
            let f = if a.clip.anim.duration > 0.0 && last > 0.0 { (clock / a.clip.anim.duration * last).rem_euclid(last) } else { 0.0 };
            a.clip.local(&a.model.bones, f, &mut local);
            let (mt, mr) = a.clip.mover(f);
            // (the hangout's people: bystanders, the manager's own lights)
            self.buf.next_bystander = true;
            self.buf.emit(&a.model, 0, &local, Mat4::from_rotation_translation(mr, mt));
            if let (Some(n), Some(p)) = (steps, a.model.head.and_then(|h| self.buf.bone_pos(h))) {
                eprintln!("dstep {n} hang {k} {:.4} {:.4} {:.4} 0", p.x, p.y, p.z);
            }
        }
        for (si, sc) in self.playing.iter_mut().chain(self.linger.iter_mut()).enumerate() {
        let t = sc.draw_t(alpha);
        for (k, a) in sc.actors.iter_mut().enumerate() {
            a.pose(t - a.t0, t, &mut local);
            if let Some(face) = a.face.as_ref() {
                let fmapped = face.anim.layout != a.model.layout;
                if let (Some((shapes, tracks)), Some(m)) = (a.shapes.as_mut(), a.model.morph.as_ref()) {
                    // a target without a track keeps its weight (F-77597 writes only what the frame has)
                    for (i, tk) in tracks.iter().enumerate() {
                        if let Some(v) = tk.and_then(|k| face.anim.tracks.get(k)).and_then(|k| crate::drvanim::sample_dof_in(&face.anim, k.kind, k.bone_id, DOF_FLOAT, t, fmapped)) {
                            shapes.set(i, v[0]);
                        }
                    }
                    shapes.update(m);
                    if std::env::var("MCLA_BS_TRACE").is_ok() {
                        let (n, d) = shapes.stats();
                        log::info!("blend shapes {} t {:.2}: {} targets on, largest delta {:.4} m", a.label, sc.t, n, d);
                    }
                }
                // (F-19950: the face pair - the face clip on the Facial_root subtree, the body on the rest -
                // whatever the body clip has for those bones)
                for i in 0..local.len() {
                    let fb = a.model.face_bones.get(i).copied().unwrap_or(false);
                    let id = a.model.ids.get(i).copied().unwrap_or(0);
                    let (has_pos, has_rot) = a.model.dofs.get(i).copied().unwrap_or((true, true));
                    if has_rot && (fb || a.clip.rot[i].is_none()) && face.rot[i].is_some() {
                        if let Some(v) = crate::drvanim::sample_dof_in(&face.anim, KIND_ROTATION, id, DOF_QUATERNION, t, fmapped) {
                            local[i].0 = Quat::from_array(v);
                        }
                    }
                    if has_pos && (fb || a.clip.pos[i].is_none()) && face.pos[i].is_some() {
                        if let Some(v) = crate::drvanim::sample_dof_in(&face.anim, KIND_TRANSLATION, id, DOF_VECTOR3, t, fmapped) {
                            local[i].1 = Vec3::new(v[0], v[1], v[2]);
                        }
                    }
                }
            }
            // (F-19950: a character with a face blends into the face pair over 0.25 s from scene time 0, from the body's
            // frame 0 with the face bones at rest; the body alone starts without a blend, F-12285)
            if a.face.is_some() {
                if a.face_in.is_none() {
                    let mut old = Vec::new();
                    pose_at(&a.clip.anim, &a.model, 0.0, &mut old);
                    for (i, b) in a.model.bones.iter().enumerate() {
                        if a.model.face_bones.get(i).copied().unwrap_or(false) {
                            old[i] = (b.rot, b.pos);
                        }
                    }
                    a.face_in = Some(old);
                }
                if let Some(old) = a.face_in.as_ref() {
                    blend_in(&mut local, old, 4.0 * t);
                }
            }
            let (mt, mr) = mover_of(&a.clip.anim, t - a.t0);
            // (F-65008: a cine character whose name holds "ped" is drawn with the manager's own
            // lights - the scenes' bystanders pedsit1_ped00, benchsit_ped05 ...; the cast with three
            // lights of its own)
            self.buf.next_bystander = a.label.to_ascii_lowercase().contains("ped");
            self.buf.emit_shaped(&a.model, 0, &local, sc.origin * Mat4::from_rotation_translation(mr, mt), a.shapes.as_ref().map(|s| &s.0));
            if let (Some(n), Some(p)) = (steps, a.model.head.or((!a.model.bones.is_empty()).then_some(0)).and_then(|h| self.buf.bone_pos(h))) {
                eprintln!("dstep {n} cast {si}_{k} {:.4} {:.4} {:.4} 0", p.x, p.y, p.z);
            }
        }
        }
        self.buf.upload(gpu);
    }

    /// What the shadow passes draw of the characters (`draw` with `shadow`), apart from them (main.rs: `prepare` runs beside
    /// those passes).
    pub fn shadow_part(&self) -> Option<crate::skin::ShadowSkin> {
        (self.playing.is_some() || !self.ambient.is_empty()).then(|| self.buf.shadow_part()).flatten()
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, shadow: bool, draws: &mut usize) {
        if self.playing.is_some() || !self.ambient.is_empty() {
            self.buf.draw(pass, gpu, shadow, draws);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LookupDriverToCutsceneName's two maps as Boot.sco fills them: the HackedScale column, and on a bike the jacket one
    /// (the player's set only differs).
    #[test]
    fn cutscene_set_by_the_mappings() {
        assert_eq!(cutscene_set("drv_mp_01_set", false), "drv_mp_01_set_hackedscale");
        assert_eq!(cutscene_set("drv_mp_01_set", true), "drv_mp_jacket_01_set_hackedscale");
        assert_eq!(cutscene_set("drv_mb_001_set", true), "drv_mb_001_set_hackedscale");
        assert_eq!(cutscene_set("drv_mb_009_set", true), "drv_mb_009_set");
        assert_eq!(cutscene_set("pol_mc_01_set", false), "pol_mc_01_set");
    }

    #[test]
    fn every_cop_set_has_its_face() {
        // (F-45542: the first table entry containing the slot name; each officer his own row)
        for (i, c) in COPS.iter().enumerate() {
            let hits: Vec<usize> = COP_FACES.iter().enumerate().filter(|(_, n)| n.contains(c)).map(|(j, _)| j).collect();
            assert_eq!(hits, vec![i], "{c}");
        }
        assert_eq!(aux_pack("cut_cop_arrest_bikestand"), Some("cut_cop_faces"));
        assert_eq!(aux_pack("cut_cop_walk_02"), None);
    }

    #[test]
    fn story_skip_lands_on_the_bookmark() {
        // (the recomp's skip measured on district champs: its clock jumped to 74.233 s - bookmark_cut_to_game
        // 74.333 at the clip's end, filed 0.1 s earlier)
        let to = skip_to("story/cut_story_district_champs", 0.036, 74.3333).unwrap();
        assert!((to - 74.2333).abs() < 1e-3);
        // RACE_GH_06: the bookmark 57.633, the cut to the game camera 64.167; a mark is filed 0.1 s earlier only when it is at
        // or past the camera clip's length (F-73781: t >= max(len, 0.1)), so a clip ending exactly there moves it ...
        assert_eq!(skip_to("story/cut_story_race_gh_06", 10.0, 64.1667), Some(57.6333));
        assert_eq!(skip_to("story/cut_story_race_gh_06", 58.0, 64.1667), None);
        assert!((cut_to_game("story/cut_story_race_gh_06", 64.1667).unwrap() - 64.0667).abs() < 1e-3);
        // ... and a longer clip leaves it where the script put it
        assert!((cut_to_game("story/cut_story_race_gh_06", 64.2).unwrap() - 64.1667).abs() < 1e-3);
        // no marks: the skip does nothing
        assert_eq!(skip_to("story/cut_story_motorcycle_rep_A", 1.0, 40.0), None);
    }

    /// `SCENE_AUDIO` is every generated scene script that names its audio stream (besides CUT_INTRO and the story
    /// scenes), read from the scripts themselves in `generated/` and `generated/racestart/`; scene_audio gives the
    /// arrest's CUT_COP_ARREST.
    #[test]
    #[ignore]
    fn scene_audio_from_the_scripts() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "../extracted/cache".into());
        let dir = Path::new(&data).join("script_obj/game/CineScripts/generated");
        let mut named: Vec<String> = Vec::new();
        let files = std::fs::read_dir(&dir).unwrap().chain(std::fs::read_dir(dir.join("racestart")).unwrap()).flatten();
        for e in files {
            let Ok(b) = std::fs::read(e.path()) else { continue };
            let Ok(sco) = mcla_formats::sco::decode(&b) else { continue };
            let strs = sco.strings(3);
            // (the raw string scan keeps the bytes before a string when they are printable: the script's header and
            // the push's length byte for 33 - 126 characters)
            let scene = strs.iter().filter_map(|s| s.find("cutscene/").map(|i| &s[i + 9..])).find(|s| !s.contains('/') || (s.starts_with("racestart/") && s.matches('/').count() == 1));
            let Some(scene) = scene.map(|s| s.rsplit('/').next().unwrap().to_string()) else { continue };
            let up = scene.to_ascii_uppercase();
            if strs.iter().any(|s| s.ends_with(&up) && s.len() <= up.len() + 1) && scene != "cut_intro" {
                named.push(scene);
            }
        }
        named.sort();
        assert_eq!(named, SCENE_AUDIO.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(scene_audio("cut_cop_arrest").as_deref(), Some("CUT_COP_ARREST"));
        assert_eq!(scene_audio("cut_cop_arrest_05"), None);
    }

    #[test]
    fn goons_walkup_stream_by_its_script_name() {
        assert_eq!(scene_audio("payback_goons_walkup_02").as_deref(), Some("CUT_PAYBACK_GOONS_WALKUP2"));
    }
}

#[cfg(test)]
mod cam_blend_tests {
    use super::*;

    fn fl(h: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(h, 16).unwrap())
    }

    #[test]
    fn cam_blend_matches_the_original() {
        let text = include_str!("testdata/cam_blend_ref.txt");
        let (mut n, mut runs, mut worst, mut exact) = (0, 0, 0u32, 0);
        let mut first = String::new();
        for l in text.lines().filter(|l| l.starts_with("BLEND")) {
            let v: Vec<f32> = l.split(' ').skip(1).map(fl).collect();
            let mut b = CamBlend { t: v[0], rate: v[1], from: v[3..19].try_into().unwrap(), from_fov: v[19] };
            let target: [f32; 16] = v[20..36].try_into().unwrap();
            let mut out = [0.0f32; 16];
            let fov = b.step(v[2], &mut out, &target, v[36]);
            n += 1;
            assert_eq!(b.t.to_bits(), v[37].to_bits(), "{l}");
            let want = &v[38..54];
            let mut d = (fov.to_bits() as i64 - v[54].to_bits() as i64).unsigned_abs() as u32;
            for i in (0..16).filter(|i| !matches!(i, 3 | 7 | 11)) {
                d = d.max((out[i].to_bits() as i64 - want[i].to_bits() as i64).unsigned_abs() as u32);
            }
            runs += b.active() as usize;
            exact += (d == 0) as usize;
            if d > worst {
                first = format!("{l} -> {out:?} {fov}");
            }
            worst = worst.max(d);
        }
        assert!(n == 3000 && runs > 1000, "{n} cases, {runs} blending");
        assert!(exact == n, "{exact} of {n} bit-exact, worst {worst} ulp: {first}");
    }

    /// F-25555 run offline (tools/cam_sample_reference.py) with its track reads answered from each case, as the scene
    /// camera update F-40303 calls it (identity matrix in): matrix (the fourth entries of rows 0-2 and row 3's w aside,
    /// which the port does not use), lens and kind-14 value, word for word.
    #[test]
    fn cam_sample_matches_the_original() {
        let text = include_str!("testdata/cam_sample_ref.txt");
        let mut n = 0;
        for l in text.lines().filter(|l| l.starts_with("SMP")) {
            let w: Vec<u32> = l.split(' ').skip(1).map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let on = |b: u32| w[0] >> b & 1 != 0;
            let mut m = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
            let (mut lens, mut k14) = (f32::from_bits(0x0badf00d), f32::from_bits(0x0badf00d));
            super::cam_sample(&mut m, &mut lens, &mut k14, on(0).then(|| Vec3::new(f(1), f(2), f(3))), on(1).then(|| [f(4), f(5), f(6), f(7)]),
                on(2).then(|| f(8)), on(3).then(|| f(9)));
            for i in (0..15).filter(|i| !matches!(i, 3 | 7 | 11)) {
                assert_eq!(m[i].to_bits(), w[10 + i], "word {i}: {l}");
            }
            assert_eq!((lens.to_bits(), k14.to_bits()), (w[26], w[27]), "{l}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }
}

#[cfg(test)]
mod cam_cut_tests {
    use super::*;

    fn fl(h: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(h, 16).unwrap())
    }

    #[test]
    fn cam_cut_matches_the_original() {
        let text = include_str!("testdata/cam_cut_ref.txt");
        let (mut n, mut cuts, mut bad) = (0, 0, Vec::new());
        for l in text.lines().filter(|l| l.starts_with("CAM")) {
            let p: Vec<&str> = l.split(' ').skip(1).collect();
            let (t, m, deg) = (fl(p[0]), fl(p[1]), fl(p[2]));
            let mut s = CamCut { prev: fl(p[3]), last: fl(p[4]), last2: f32::from_bits(0x0bad_f00d), cut: p[5] == "00000001" };
            let (on, found) = (p[6] == "00000001", u32::from_str_radix(p[7], 16).unwrap());
            let k: Vec<f32> = (0..14).map(|i| fl(p[8 + i])).collect();
            let pos = |s: f32| (found & 1 != 0).then(|| Vec3::new(k[0] + k[1] * s, k[2] + k[3] * s, k[4] + k[5] * s * s));
            let rot = |s: f32| (found & 2 != 0).then(|| Quat::from_xyzw(k[6] + k[7] * s, k[8] + k[9] * s, k[10] + k[11] * s, k[12] + k[13] * s));
            let out = cam_cut_rule(&mut s, on, t, m, deg, pos, rot);
            let want = (p[22], p[23], p[24], p[25], p[26] == "1");
            let got = (format!("{:08x}", out.to_bits()), format!("{:08x}", s.prev.to_bits()), format!("{:08x}", s.last.to_bits()), format!("{:08x}", s.last2.to_bits()), s.cut);
            n += 1;
            cuts += (out.to_bits() != t.to_bits()) as usize;
            if (got.0.as_str(), got.1.as_str(), got.2.as_str(), got.3.as_str(), got.4) != want {
                bad.push(format!("{l} -> {got:?}"));
            }
        }
        assert!(n == 4000 && cuts > 1000, "{n} cases, {cuts} snapped");
        assert!(bad.is_empty(), "{} of {n} differ, first: {}", bad.len(), bad[0]);
    }
}

#[cfg(test)]
mod cast_tests {
    use super::*;

    #[test]
    fn generic_cast_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/cine_cast_ref.txt").lines() {
            let w: Vec<&str> = line.split(' ').collect();
            let (girl, slot): (bool, i32) = (w[1] == "1", w[2].parse().unwrap());
            let mut ctr: [i32; 4] = [w[3].parse().unwrap(), w[4].parse().unwrap(), w[5].parse().unwrap(), w[6].parse().unwrap()];
            let mask: u32 = w[7].parse().unwrap();
            let calls = std::cell::Cell::new(0u32);
            let set = cast_pick(girl, slot, &mut ctr, |_| {
                let k = calls.get();
                calls.set(k + 1);
                (mask >> k) & 1 != 0
            });
            let want: Vec<i32> = w[11..15].iter().map(|v| v.parse().unwrap()).collect();
            assert_eq!((set, w[10].parse::<i32>().unwrap(), ctr.to_vec()), (w[9], slot, want), "{line}");
            n += 1;
        }
        assert_eq!(n, 3472);
    }
}

#[cfg(test)]
mod draw_between_tests {
    use super::*;

    /// The drawn scene time runs between the tick's start and end by the render alpha; a jump or a time going back is drawn
    /// at once (the port's interpolation, `Scene::draw_t`).
    #[test]
    fn the_drawn_scene_time_lies_between_the_ticks() {
        let mut sc = Scene { name: String::new(), camera: None, actors: Vec::new(), movers: Vec::new(), duration: 10.0, t: 2.0, t_prev: 2.0, origin: Mat4::IDENTITY, camera_killed: false, cam_cut: Default::default() };
        sc.t_prev = sc.t;
        sc.t += 1.0 / 60.0;
        assert_eq!(sc.draw_t(1.0), sc.t);
        assert_eq!(sc.draw_t(0.0), sc.t_prev);
        assert!((sc.draw_t(0.25) - (2.0 + 0.25 / 60.0)).abs() < 1e-6);
        sc.t_prev = sc.t;
        sc.t = 7.5;
        assert_eq!(sc.draw_t(0.25), 7.5);
        sc.t_prev = sc.t;
        sc.t = 1.0;
        assert_eq!(sc.draw_t(0.25), 1.0);
    }
}
