//! The motion part of PSStreakMotionBlur (LIGHTING.md "Motion blur"): the previous-frame matrices the pass
//! reprojects each pixel with (Mc4MotionBlurVehicleMtxs, 24 of them, c69 + 4 k), and the stencil ids the scene
//! pass writes so that a pixel finds its own (k = stencil / 4).
//!
//! The original (all read from the recompiled code):
//! - F-94650 (renderer, the object's matrix, its previous matrix, its velocity) registers an object for the
//!   frame: at most 24 (else -1), record k = (camera matrix, world matrix) at renderer +320 + 128 k / +384 + 128 k,
//!   returns 4 k - the stencil reference the object is drawn with. The car draw (F-10884, its main opaque phase)
//!   keeps it at car +6028 and draws every part of the car - body, wheels, glass, lamps - with it; it hands the
//!   driver (F-17918, the character's id) +6028 + 1, or +6028 when F-54299 says the car is a motorbike. Recorded
//!   (work/slipstream/trailrec g.0.bin, three frames, and the paused g.1.bin): the player's body and glass 4, his
//!   driver 5, the other racer's body and glass 8, its driver 9, the far traffic 12 .. 48 (their stencil-only cheap
//!   draws). Until 2026-10-28 the port read the + 1 as the other cars' bodies and gave the drivers 0.
//!   F-46851 (frame end) makes record 0 the world (identity) for the next frame.
//! - F-46851 (frame start) sets each record's camera and world on the device and reads the composite
//!   (device +512 = world x inverse(camera) x projection) into the array; F-47680 copies the 24 into the
//!   post object (+1712) and F-12598 uploads them (SetMatrixArray, handle +3248).
//! - The records a frame's streak pass reads were registered during the frame BEFORE (the count is reset and the
//!   world re-registered at the end of F-46851; the cars register while they are drawn): the matrices are the
//!   previous frame's camera and the cars' previous poses, under this frame's projection - recorded frames in
//!   motion show the camera's turn in them (far points move: checkpoints2 col5 2.6 px, lighting car_r 23 px) and a
//!   car's own record follows the car (its pixels 2 px where the world's are 70 px; mb_fit.py).
//! - With the renderer's byte +374 set (1 from the constructor F-76712, never cleared) and the time scale
//!   (`0x827D7500` +84, the port's mc4_time_scale) not below 1, each record's camera is pulled back by a quarter of
//!   the camera's last step (+272 = 0.25 x (this - last), F-46851 / F-94650) and a car's place by a
//!   quarter of its velocity x the frame time (`0x827D7500` +8). With the time scale below 1: the plain previous
//!   camera and the caller's previous matrix. With the pause / replay condition of 0x828CCFC4 +76 bit 0x40 (and
//!   0x828CCEA0): this frame's camera, no motion.
//! So against this frame: the camera 1.25 steps back and turned one frame back, a car likewise.
//! *(approx.)* the original's frame is 1 / 30 s (two simulation ticks a frame); the port measures the camera's and
//! the cars' speeds over its own shorter frames and takes the steps of a 1 / 30 s frame. Checked on recorded
//! frames only as far as the recordings allow: checkpoints2 f4 (17 mph in reverse) has the previous camera 0.269 m
//! back - 1.06 steps of a 1 / 30 s frame, 1.25 steps of a 1 / 35 s one; the recorder's frame time is not known.

use glam::{Mat4, Quat, Vec3};
use std::sync::Mutex;

pub const RECORDS: usize = 24;
/// The original's frame time (TIME +8 at its 30 frames a second).
pub const FRAME: f32 = 1.0 / 30.0;

/// What the scene pass draws a car with: its stencil reference and its driver's (index into the frame's car list;
/// 0 = none / world).
static STENCIL: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());

/// The stencil reference of car `ci`'s body this frame (0 when it has no record).
pub fn stencil_of(ci: usize) -> u32 {
    STENCIL.lock().ok().and_then(|s| s.get(ci).copied()).map_or(0, |r| r.0)
}

/// The stencil reference car `ci`'s driver draws with this frame: the body's + 1, a rider the body's (0 when the car
/// has no record).
pub fn driver_stencil_of(ci: usize) -> u32 {
    STENCIL.lock().ok().and_then(|s| s.get(ci).copied()).map_or(0, |r| r.1)
}

static MB: Mutex<MotionBlur> = Mutex::new(MotionBlur { prev_cam: None, prev_cars: Vec::new() });

/// [`MotionBlur::frame`] on the game's one history.
pub fn frame(view: Mat4, proj: Mat4, cars: &[Option<Car>], dt: f32, slow: bool, still: bool) -> [[[f32; 4]; 4]; RECORDS] {
    MB.lock().map(|mut m| m.frame(view, proj, cars, dt, slow, still)).unwrap_or([Mat4::IDENTITY.to_cols_array_2d(); RECORDS])
}

#[derive(Default)]
pub struct MotionBlur {
    /// last frame's camera: place, rotation (camera -> world)
    prev_cam: Option<(Vec3, Quat)>,
    prev_cars: Vec<Option<(Vec3, Quat)>>,
}

/// Record `k`'s stencil ids: the body's 4 k (F-94650), the driver's 4 k + 1, a rider's 4 k (F-10884).
pub fn ids(k: usize, bike: bool) -> (u32, u32) {
    (4 * k as u32, 4 * k as u32 + if bike { 0 } else { 1 })
}

/// A car as the frame sees it: its place, rotation, velocity (m/s), whether it is a motorbike.
pub struct Car {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub bike: bool,
}

/// `rot` turned back by `s` times the turn from `prev` to it.
fn back(rot: Quat, prev: Quat, s: f32) -> Quat {
    let d = (prev * rot.inverse()).normalize();
    let (axis, angle) = d.to_axis_angle();
    let angle = if angle > std::f32::consts::PI { angle - 2.0 * std::f32::consts::PI } else { angle };
    (Quat::from_axis_angle(axis, angle * s) * rot).normalize()
}

impl MotionBlur {
    /// This frame's 24 matrices (column-major, the port's clip space: world -> the original's previous clip) and the
    /// cars' stencil references. `view` / `proj`: this frame's; `dt`: the frame's real seconds; `slow`: the time
    /// scale is below 1; `still`: paused (no motion).
    pub fn frame(&mut self, view: Mat4, proj: Mat4, cars: &[Option<Car>], dt: f32, slow: bool, still: bool) -> [[[f32; 4]; 4]; RECORDS] {
        let inv = view.inverse();
        let cam = inv.w_axis.truncate();
        let cam_rot = Quat::from_mat4(&inv).normalize();
        // the port's frames as steps of the original's
        let s = (FRAME / dt.max(1e-4)).min(30.0);
        let pull = if slow { 0.0 } else { 0.25 };
        let mut out = [(proj * view).to_cols_array_2d(); RECORDS];
        let cam_view = match self.prev_cam.filter(|_| !still) {
            Some((pc, pr)) => {
                let step = (cam - pc) * s;
                let c2 = cam - step - pull * step;
                let r2 = back(cam_rot, pr, s);
                Mat4::from_rotation_translation(r2, c2).inverse()
            }
            None => view,
        };
        let pv = proj * cam_view;
        out[0] = pv.to_cols_array_2d();
        let mut refs = vec![(0u32, 0u32); cars.len()];
        let mut k = 1usize;
        self.prev_cars.resize(cars.len(), None);
        for (ci, c) in cars.iter().enumerate() {
            let Some(c) = c else {
                self.prev_cars[ci] = None;
                continue;
            };
            if k < RECORDS {
                let world_step = match self.prev_cars[ci].filter(|_| !still) {
                    Some((pp, pr)) => {
                        let p2 = c.pos - (c.pos - pp) * s - pull * c.vel * FRAME;
                        let r2 = back(c.rot, pr, s);
                        Mat4::from_rotation_translation(r2, p2) * Mat4::from_rotation_translation(c.rot, c.pos).inverse()
                    }
                    None => Mat4::IDENTITY,
                };
                out[k] = (pv * world_step).to_cols_array_2d();
                refs[ci] = ids(k, c.bike);
                k += 1;
            }
            self.prev_cars[ci] = Some((c.pos, c.rot));
        }
        self.prev_cam = Some((cam, cam_rot));
        if let Ok(mut st) = STENCIL.lock() {
            *st = refs;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_camera_gives_this_frames_matrix() {
        let mut m = MotionBlur::default();
        let view = Mat4::look_at_rh(Vec3::new(800.0, 20.0, -800.0), Vec3::new(810.0, 20.0, -800.0), Vec3::Y);
        let proj = Mat4::perspective_infinite_reverse_rh(1.0, 16.0 / 9.0, 0.1);
        let a = m.frame(view, proj, &[], 1.0 / 60.0, false, false);
        let b = m.frame(view, proj, &[], 1.0 / 60.0, false, false);
        let vp = (proj * view).to_cols_array();
        for (x, y) in Mat4::from_cols_array_2d(&b[0]).to_cols_array().iter().zip(vp.iter()) {
            assert!((x - y).abs() < 1e-4 * y.abs().max(1.0));
        }
        let _ = a;
    }

    #[test]
    fn camera_is_a_30_fps_frame_and_a_quarter_back() {
        let mut m = MotionBlur::default();
        let proj = Mat4::perspective_infinite_reverse_rh(1.0, 16.0 / 9.0, 0.1);
        let v0 = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -1.0), Vec3::Y);
        // 30 m/s along -z, frames of 1/60 s: 0.5 m a frame, 1 m in the original's frame, 1.25 m back
        let v1 = Mat4::look_at_rh(Vec3::new(0.0, 0.0, -0.5), Vec3::new(0.0, 0.0, -1.5), Vec3::Y);
        m.frame(v0, proj, &[], 1.0 / 60.0, false, false);
        let r = m.frame(v1, proj, &[], 1.0 / 60.0, false, false);
        let pv = Mat4::from_cols_array_2d(&r[0]);
        let want = proj * Mat4::look_at_rh(Vec3::new(0.0, 0.0, 0.75), Vec3::new(0.0, 0.0, -0.25), Vec3::Y);
        let p = Vec3::new(1.0, 0.5, -10.0);
        let (a, b) = (pv.project_point3(p), want.project_point3(p));
        assert!((a - b).length() < 1e-5, "{a} {b}");
    }

    #[test]
    /// The ids recorded in the original (work/slipstream/trailrec g.0.bin): the player's body 4 and driver 5, the
    /// next racer's 8 and 9; a rider keeps his bike's id.
    fn cars_get_four_k_and_drivers_one_more() {
        let mut m = MotionBlur::default();
        let proj = Mat4::perspective_infinite_reverse_rh(1.0, 16.0 / 9.0, 0.1);
        let car = |bike| Some(Car { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::new(30.0, 0.0, 0.0), bike });
        m.frame(Mat4::IDENTITY, proj, &[car(false), None, car(false), car(true)], 1.0 / 60.0, false, false);
        assert_eq!((ids(1, false), ids(2, false), ids(3, true)), ((4, 5), (8, 9), (12, 12)));
    }
}
