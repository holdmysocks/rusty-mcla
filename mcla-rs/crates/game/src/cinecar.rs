//! Cine car events: a scene's car is DRIVEN along its mover, not posed on it
//! (docs/reference/cutscenes/cine_car_event.md).
//!
//! The cine core script (game/CineScripts/source 0x95ca1d1b, 0xBBA..0xC2D) gives every car slot that
//! has a mover animation an mcBrainAnimated (BrainFactory_CreateBrainAnimated) unless the scene's
//! description sets its game-driven word (+1288: only `cut_cop_alert` does), launches its car event
//! (CineScript_Cars_LaunchEvent) and deletes the brain when the scene finishes (BrainFactory_DeleteBrain,
//! 0x1926). The event's update (`F-31853`, once a frame with the time before it advances) asks for
//! an acceleration and a curvature from the mover's path and the car's state; on its first frame and on
//! a cut in the path it sets the car down on the path (`F-86308`). The brain's update
//! (`F-91361` -> `F-28541`) turns the two into the chassis's steering, pedals and gear from the
//! car's calibration tables (`aivVehicleTuning`); the physics moves the car.
use crate::physics::car::{CarInput, CarState};
use crate::physics::exact::{ease, guest_atan2};
use crate::physics::vehsim::VehTune;
use glam::{Mat3, Mat4, Quat, Vec3};
use mcla_formats::rage::aitune::AiTune;
use mcla_formats::rage::anim::{Animation, KIND_MOVER_ROTATION, KIND_MOVER_TRANSLATION};
use std::sync::Arc;

/// mcBrainAnimated: what the car event last wrote (brain +12, +16) - kept when the event ends, until the
/// brain is deleted with the scene - and the car's calibration at its state of tune.
#[derive(Clone, Debug)]
pub struct AnimBrain {
    pub accel: f32,
    pub curv: f32,
    pub tune: Option<Arc<AiTune>>,
}

/// The car event instance (mcCineCarInstance): +156, the path time its cut test last stood on; whether
/// its first update has run; whether it has stopped (its time past the mover's duration).
#[derive(Clone, Debug, Default)]
pub struct CarEvent {
    prev: f32,
    pub started: bool,
    pub done: bool,
}

/// The pose, velocity and turn the car is set down with (`F-86308`'s arguments).
pub struct Snap {
    pub m: [Vec3; 4],
    pub vel: Vec3,
    pub ang_vel: Vec3,
}

/// `F-32060`: the signed xz angle turning `a` onto `b`, 0 when either is vertical.
fn sangle(a: Vec3, b: Vec3) -> f32 {
    let n = ((a.z * a.z + a.x * a.x) * (b.z * b.z + b.x * b.x)).sqrt();
    if n - 1e-12 < 0.0 {
        return 0.0;
    }
    let ang = (((b.z * a.z + b.x * a.x) / n).clamp(-1.0, 1.0) as f64).acos() as f32;
    if a.z * b.x - b.z * a.x >= 0.0 {
        ang
    } else {
        -ang
    }
}

/// `F-94425`: the angle between two rotations, 2 acos |q0 . q1| (0 outside (-1, 1)).
fn quat_angle(a: Quat, b: Quat) -> f32 {
    // (fmadds chain of the original: z first, then w, y, x; each a fused multiply-add)
    let d = b.x.mul_add(a.x, b.y.mul_add(a.y, b.w.mul_add(a.w, b.z * a.z)));
    if d > -1.0 && d < 1.0 {
        (d.abs() as f64).acos() as f32 * 2.0
    } else {
        0.0
    }
}

/// `F-41334`: a unit vector, or `other` when its squared length is not above `least`.
fn unit_or(v: Vec3, least: f32, other: Vec3) -> Vec3 {
    let l2 = v.x * v.x + v.y * v.y + v.z * v.z;
    if l2 > least {
        v / l2.sqrt()
    } else {
        other
    }
}

fn pos_at(a: &Animation, t: f32) -> Option<Vec3> {
    a.track(KIND_MOVER_TRANSLATION, 0).map(|k| k.data.vec(a.frame_at(t)))
}

fn rot_at(a: &Animation, t: f32) -> Option<Quat> {
    a.track(KIND_MOVER_ROTATION, 0).map(|k| k.data.rot(a.frame_at(t)))
}

/// `F-13038`: the cut test. t0 = t on the 1/30 s grid (from where the last test stood plus 1/30 if it
/// skipped more than 0.05 s), t1 the grid step after the plain t0; a move over 2 m or a turn over 20
/// degrees of the mover between them is a cut, and the update goes on from t1. +156 = the plain t0.
fn cut_test(prev: &mut f32, a: &Animation, t: f32) -> (f32, bool) {
    cut_rule(prev, t, 2.0, 20.0, |s| pos_at(a, s), |s| rot_at(a, s))
}

/// The cut test's rule on its two samplers (the mover's place and rotation at a time), with the distance (m) and turn
/// (deg) limits the update passes in. Test: `cut_rule_matches_the_original`.
fn cut_rule(prev: &mut f32, t: f32, metres: f32, degrees: f32, pos: impl Fn(f32) -> Option<Vec3>, rot: impl Fn(f32) -> Option<Quat>) -> (f32, bool) {
    let step = 1.0f32 / 30.0;
    let t0 = t - (t as f64 % (1.0f32 / 30.0) as f64) as f32;
    let at = if t0 - *prev > 0.05 { *prev + step } else { t0 };
    let t1 = t0 + step;
    let mut d = pos(at).unwrap_or(Vec3::ZERO);
    if let Some(p1) = pos(t1) {
        d -= p1;
    }
    let q0 = rot(at).unwrap_or(Quat::IDENTITY);
    let turn = rot(t1).map_or(0.0, |q1| quat_angle(q0, q1));
    let cut = d.x.mul_add(d.x, d.y.mul_add(d.y, d.z * d.z)) > metres * metres || turn > degrees * 0.017_453_292;
    *prev = t0;
    (if cut { t1 } else { t }, cut)
}

/// One frame of the car event (`F-31853`): `t` the event's time before this frame, `dt` the frame,
/// `scene` the scene's frame in the world (the coordinate-system binding, `F-88556`), `idle0` the
/// car's calibration [12] (IdleSpeed of its first row). Gives the brain's acceleration and curvature, and
/// the pose to set the car down in on the first frame or a cut.
pub fn update(ev: &mut CarEvent, a: &Animation, scene: Mat4, t: f32, dt: f32, car: &CarState, idle0: f32) -> (f32, f32, Option<Snap>) {
    let (t, cut) = cut_test(&mut ev.prev, a, t);
    let first = !std::mem::replace(&mut ev.started, true);
    let dur = a.duration;
    // the mover's pose (track 5 place, track 6 rotation) in the scene's frame
    let p0 = pos_at(a, t).unwrap_or(Vec3::ZERO);
    let rm = Mat3::from_quat(rot_at(a, t).unwrap_or(Quat::IDENTITY));
    let (mut rx, ry, mut rz) = (rm.x_axis, rm.y_axis, rm.z_axis);
    let back = |x: f32| if -x >= 0.0 { 0.0 } else { x };
    let pm1 = if !cut { pos_at(a, back(t - dt)).unwrap_or(p0) } else { p0 };
    let pm2 = if !cut { pos_at(a, back((-dt).mul_add(2.0, t))).unwrap_or(pm1) } else { pm1 };
    let ahead = |x: f32| if dur - x >= 0.0 { x } else { dur };
    let pp1 = pos_at(a, ahead(t + dt)).unwrap_or(p0);
    let pp2 = pos_at(a, ahead(dt.mul_add(2.0, t))).unwrap_or(pp1);
    let len = |d: Vec3| d.x.mul_add(d.x, d.z.mul_add(d.z, d.y * d.y)).sqrt();
    let step = if 1e-6 - dt >= 0.0 { 1e-6 } else { dt };
    let inv = 1.0 / step;
    let v = len(pp1 - p0) * inv;
    let v2 = len(pp2 - pp1) * inv;
    let vb = len(p0 - pm1) * inv;
    let vm = if 5.0 - v >= 0.0 { 5.0 } else { v };
    let mid = pp1 - pm1;
    let behind = p0 - pm2;
    let curv_ahead = sangle(mid, pp2 - p0) / vm;
    let curv_behind = sangle(behind, mid) / if 5.0 - vb >= 0.0 { 5.0 } else { vb };
    // the car in the scene's frame (the binding's rows dotted: its inverse turn)
    let (_, br, bt) = scene.to_scale_rotation_translation();
    let inv_r = br.inverse();
    let local = |w: Vec3| inv_r * w;
    let cm = Mat3::from_quat(car.rot);
    let car_x = local(cm.x_axis);
    let vel = local(car.vel);
    let fwd = local(-cm.z_axis);
    let spin = local(car.ang_vel);
    let org = local(car.pos - bt);
    let speed = car.vel.y.mul_add(car.vel.y, car.vel.x.mul_add(car.vel.x, car.vel.z * car.vel.z)).sqrt();
    let (accel, curv) = if v > 0.25 * idle0 {
        let err = p0 - org;
        let l = vm * 0.5;
        // the pose heads along the path, not the rotation track
        rz = unit_or(-mid, 0.0, Vec3::ZERO);
        rx = ry.cross(rz);
        let a_fwd = sangle(fwd, behind);
        let lat = guest_atan2(-err.z.mul_add(car_x.z, err.x.mul_add(car_x.x, car_x.y * err.y)) as f64, l as f64) as f32;
        let fwd_h = unit_or(Vec3::new(fwd.x, 0.0, fwd.z), 1e-5, Vec3::X);
        let vel_h = unit_or(Vec3::new(vel.x, 0.0, vel.z), 1.0, fwd_h);
        let a_vel = sangle(vel_h, behind);
        let along = err.x.mul_add(fwd.x, fwd.y.mul_add(err.y, err.z * fwd.z));
        let cos = vel_h.y.mul_add(fwd_h.y, vel_h.x.mul_add(fwd_h.x, vel_h.z * fwd_h.z));
        let mover_acc = (v2 - v) / step;
        let (mut f0, mut f13, mut f12, mut f11, mut lat, mut a_fwd) = (along * 8.0, (v - speed) * 2.0, curv_behind - spin.y / vm, a_vel * cos, lat, a_fwd);
        // (the port may start a scene part-way - a race start's handover: its first frame stands for the
        // original's t = 0)
        if t == 0.0 || cut || first {
            (f0, f13, f12, f11, lat, a_fwd) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        }
        ((f0 + mover_acc) + f13, ((f11 + lat) + a_fwd) / l + f12 + curv_ahead)
    } else {
        (-100.0, 0.0)
    };
    // the pose into the world
    let bm = Mat3::from_quat(br);
    let m = [bm * rx, bm * ry, bm * rz, bt + bm * p0];
    let snap = (t == 0.0 || cut || first).then(|| Snap { m, vel: m[2] * -v, ang_vel: Vec3::new(0.0, v * curv, 0.0) });
    (accel, curv, snap)
}

/// The brain's inputs (`F-91361` with `F-28541`): the steering for the curvature at this speed
/// (SteerScalar), the pedals for the acceleration against what the car can do in its gear (the engine's
/// pull and braking tables, more pull while sliding or steering - DriftTorque over the engine's +20), the
/// drag the tables model (DragConstant1..3), the nitrous / slipstream push and the slope; no handbrake;
/// the gearbox out of reverse or neutral (`CarInput::animated`).
pub fn solve(b: &AnimBrain, st: &CarState, vt: Option<&VehTune>) -> CarInput {
    let mut out = CarInput { animated: true, ..Default::default() };
    let Some(tn) = b.tune.as_deref() else { return out };
    let vs = st.veh.as_deref();
    // the original's gear numbering (0 reverse, 1 neutral, 2 first): at least first
    let row = vs.map_or(1, |v| v.gear.max(1)) as usize;
    let m = Mat3::from_quat(st.rot);
    let (x, fwd, vel) = (m.x_axis, -m.z_axis, st.vel);
    let v = fwd.x.mul_add(vel.x, vel.z.mul_add(fwd.z, vel.y * fwd.y));
    let ss = tn.table_at(&tn.steer_scalar, v.abs());
    let steer = (-(ss * b.curv)).clamp(-1.0, 1.0);
    let mut pull = tn.table_at(&tn.accel_rows[row.min(tn.accel_rows.len() - 1)], v).max(0.1);
    let hold = tn.table_at(&tn.decel[row.min(tn.decel.len() - 1)], v).max(0.1);
    let k = steer / ss;
    let d = (k.abs().mul_add(tn.drag[2], tn.drag[1])).mul_add(v, tn.drag[0]);
    let (nitro, turbo) = vs.map_or((0.0, 0.0), |v| (v.nitro_level.max(0.0), v.sst.level.max(0.0)));
    let (nitro_amount, turbo_amount, gravity, drift) = vt.map_or((0.0, 0.0, 9.8, 0.0), |t| (t.t.nitro.boost_amount, t.t.ss_turbo.boost_amount, t.gravity, t.t.gyro.drift_torque / t.sync_torque));
    let extra = ((turbo * turbo) * turbo).mul_add(turbo_amount, (nitro * nitro) * nitro_amount);
    let d = d.max(0.0);
    let need = ((d * d) * d).mul_add(d, b.accel);
    let need = gravity.mul_add(fwd.y, need);
    if drift != 0.0 && v > 1.0 {
        let u = unit_or(vel, 0.0, Vec3::ZERO);
        let side = x.x.mul_add(u.x, x.y.mul_add(u.y, x.z * u.z)).abs().min(1.0);
        pull *= ease(side).max(steer.abs()).mul_add(drift, 1.0);
    }
    let (throttle, brake) = if need >= extra * 0.5 {
        ((pull + need) / pull.mul_add(2.0, extra), 0.0)
    } else if need >= -hold {
        ((hold + need) / hold.mul_add(2.0, extra), 0.0)
    } else {
        (0.0, -((hold + need) / tn.max_brake_decel))
    };
    out.steer = steer;
    out.throttle = throttle.clamp(0.0, 1.0);
    out.brake = brake.clamp(0.0, 1.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(x: &str) -> u32 {
        u32::from_str_radix(x, 16).unwrap()
    }
    fn fl(x: &str) -> f32 {
        f32::from_bits(word(x))
    }

    /// F-94425 against the original's code run offline (`tools/cutscenes_small_reference.py`, QANG lines: random pairs of
    /// unit rotations, near-equal pairs, +-1 dots, zero and NaN quaternions): the result bit for bit.
    #[test]
    fn quat_angle_matches_the_original() {
        let text = include_str!("../tests/data/cutscenes_small_reference.txt");
        let mut n = 0;
        let mut bad = Vec::new();
        for l in text.lines().filter(|l| l.starts_with("QANG")) {
            let p: Vec<&str> = l.split(' ').collect();
            let q = |i: usize| Quat::from_xyzw(fl(p[i]), fl(p[i + 1]), fl(p[i + 2]), fl(p[i + 3]));
            let got = quat_angle(q(1), q(5));
            n += 1;
            if got.to_bits() != word(p[9]) && !(got.is_nan() && fl(p[9]).is_nan()) {
                bad.push((l.to_string(), got.to_bits()));
            }
        }
        assert!(n > 800);
        assert!(bad.is_empty(), "{} of {n} differ, first {:?}", bad.len(), bad.first());
    }

    /// F-13038 against the original's code run offline (CUT lines: the cut rule with the mover sampler replaced by a stub that
    /// returns synthetic place / rotation samples, 2000 cases of time, previous time and path): the returned time, the cut
    /// flag and the stored previous time bit for bit. The sampler is not compared here.
    #[test]
    fn cut_rule_matches_the_original() {
        let text = include_str!("../tests/data/cutscenes_small_reference.txt");
        let (mut n, mut cuts) = (0, 0);
        let mut bad = Vec::new();
        for l in text.lines().filter(|l| l.starts_with("CUT")) {
            let p: Vec<&str> = l.split(' ').collect();
            let (t, prev, found) = (fl(p[1]), fl(p[2]), p[3].parse::<u32>().unwrap());
            let k: Vec<f32> = (0..14).map(|i| fl(p[4 + i])).collect();
            let pos = |s: f32| (found & 1 != 0).then(|| Vec3::new(k[0] + k[1] * s, k[2] + k[3] * s, k[4] + k[5] * s * s));
            let rot = |s: f32| (found & 2 != 0).then(|| Quat::from_xyzw(k[6] + k[7] * s, k[8] + k[9] * s, k[10] + k[11] * s, k[12] + k[13] * s));
            let mut pv = prev;
            let (nt, cut) = cut_rule(&mut pv, t, 2.0, 20.0, pos, rot);
            n += 1;
            cuts += cut as u32;
            if nt.to_bits() != word(p[18]) || cut != (p[19] == "1") || pv.to_bits() != word(p[20]) {
                bad.push(l.to_string());
            }
        }
        assert!(n >= 2000 && cuts > 500 && n - cuts > 300, "{n} cases, {cuts} cuts");
        assert!(bad.is_empty(), "{} of {n} differ, first {:?}", bad.len(), bad.first());
    }

    #[test]
    fn sangle_signs() {
        // turning right from straight ahead (-z) is negative, as the original's
        let a = sangle(Vec3::NEG_Z, Vec3::new(1.0, 0.0, -1.0));
        assert!((a + std::f32::consts::FRAC_PI_4).abs() < 1e-5, "{a}");
        assert_eq!(sangle(Vec3::Y, Vec3::X), 0.0);
    }

    /// The brain's pedals (F-28541): asked for more than the gear's pull in a straight line at rest,
    /// full throttle; asked to slow harder than engine braking, the brakes for the rest of MaxBrakeDecel.
    #[test]
    fn pedals_from_the_tables() {
        let tune = AiTune {
            max_speed: 127.0, min_curvature: 0.0, max_curvature: 0.2, max_total_accel: 10.0, max_brake_decel: 10.0, idle_speed: [1.0; 7],
            accel: Vec::new(), decel: vec![vec![2.0; 128]; 7], speed_high_curv: Vec::new(), speed_low_curv: Vec::new(),
            steer_scalar: vec![5.0; 128], drag: [0.0; 3], accel_rows: vec![vec![4.0; 128]; 8],
        };
        let b = |accel: f32, curv: f32| AnimBrain { accel, curv, tune: Some(Arc::new(tune.clone())) };
        let st = CarState::new(Vec3::ZERO, 0.0);
        let go = solve(&b(8.0, 0.0), &st, None);
        assert_eq!((go.throttle, go.brake, go.steer, go.animated), (1.0, 0.0, 0.0, true));
        // need 0: half throttle (the chassis's coasting point)
        assert_eq!(solve(&b(0.0, 0.0), &st, None).throttle, 0.5);
        // -7 against engine braking 2: brakes (7 - 2) / 10
        let stop = solve(&b(-7.0, 0.0), &st, None);
        assert_eq!((stop.throttle, stop.brake), (0.0, 0.5));
        // the curvature through SteerScalar, the other way round, held to full lock
        assert_eq!(solve(&b(0.0, 0.1), &st, None).steer, -0.5);
        assert_eq!(solve(&b(0.0, -1.0), &st, None).steer, 1.0);
    }
}
