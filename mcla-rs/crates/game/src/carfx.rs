//! The exhaust and nitrous effects of every car on the original's simulation (player, racers, police):
//! records 0 / 1 (`veh_exhaust`) and 2 / 3 (`veh_nitro`) of the car's effects object, kept playing at the two
//! effect anchors while their conditions hold - F-21480 (the update), F-87024 (start), F-21553 (anchors).
//! The original has no backfire effect: the engine's backfire is a sound only.
//! Rule: `docs/reference/carparts/car_exhaust_fx_rule.md`.

use crate::particles::Particles;
use crate::vehicles::CarInstance;
use glam::{Mat4, Quat, Vec3};
use mcla_formats::hash::joaat;

/// The idle exhaust smoke stops at this speed (chassis +224, m/s).
const EXHAUST_MAX_SPEED: f32 = 1.5;

/// Whether a rear bumper end is broken off and the anchors are on the rear bumper (G+6483 with H+33):
/// then neither effect plays.
fn bumper_gate(c: &CarInstance) -> bool {
    c.model.fx_anchor_rear && c.state.parts.bumpers[0].as_ref().is_some_and(|b| b.a.state == 2 || b.b.state == 2)
}

/// One frame of the effects for every car (call every tick while the world runs). `player_hidden`: the player's
/// car is not drawn by the camera (the bumper view, `App::hide_player`).
pub fn step(fx: &mut Particles, cars: &[CarInstance], cam: Vec3, player_hidden: bool) {
    let (exhaust, nitro) = (joaat("veh_exhaust"), joaat("veh_nitro"));
    for (ci, c) in cars.iter().enumerate() {
        if c.state.veh.is_none() || c.parked.is_some() || !c.placed || c.model.fx_anchors.is_empty() {
            continue;
        }
        let st = &c.state;
        let gate = bumper_gate(c);
        // nitro: while the shot burns; never on a police unit (chassis +819, F-37253); with the car's velocity
        // (test hook MCLA_CARFX_FORCE=1: the player's nitro effect on all the time, also standing)
        let nitro_on = !gate && !c.cop && (st.nitro_on() || (c.is_player && std::env::var_os("MCLA_CARFX_FORCE").is_some()));
        // exhaust: engine running (`engine_off`: a scene's car_off), the model's switch +598 on (`exhaust`), not in stealth (F-54047: the effect's type, not its fade), the car drawn, below 1.5 m/s;
        // "drawn" = G+118, bit 0 of the racer's draw mode (F-95074, F-16712 reads it): 0 only for the player's car in
        // the bumper view - not during the crash camera, the Damageout event, a cutscene or a menu - which is the
        // port's `hide_player`; every other car always (no cull or distance test)
        // evolution "throttle" = the chassis' throttle +136 as the sim's tick left it (F-21480 copies it as it is):
        // the pedal after the pad's dead zone and the rev limiter's cut, 1 while the nitrous burns; no velocity
        let drawn = !(c.is_player && player_hidden);
        let exhaust_on = !gate && !c.engine_off && c.exhaust && !c.pu_stealth_on && drawn && st.speed() < EXHAUST_MAX_SPEED;
        let throttle = chassis_throttle(st, &c.input);
        let car_m = Mat4::from_rotation_translation(st.rot, st.pos);
        // (the original pushes a bone's matrix, one 60 Hz frame old, on by the body's velocity / 60 - with a side
        // exhaust it takes the car's matrix instead; both come to the car's matrix of this frame, which is what
        // the port has here)
        // (a bone of the rear bumper stands where the bumper hangs: its global follows the part's pose, F-14987 -
        // the same move as the bumper's draw, `Bumper::draw_delta`)
        let rear_m = car_m * c.state.parts.bumpers[0].as_ref().filter(|b| b.detached.is_none()).map_or(Mat4::IDENTITY, |b| b.draw_delta_on(c.model.slot_bones[0].as_ref().unwrap_or(&b.m_rest)));
        for (k, (a, on_rear)) in c.model.fx_anchors.iter().take(2).enumerate() {
            let w = if *on_rear { rear_m } else { car_m } * *a;
            let pos = w.w_axis.truncate();
            // (the matrix to quaternion F-16377 in the original's arithmetic, physics::exact_mat; the unit length is the port's)
            let axes = [w.x_axis, w.y_axis, w.z_axis].map(|c| c.truncate().normalize_or_zero().to_array());
            let q = crate::physics::exact_mat::quat_of_rows(&axes);
            let rot = Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize();
            let dist = pos.distance(cam);
            let key = 0x4341_5246_0000_0000 | ((ci as u64) << 4) | k as u64;
            if exhaust_on {
                fx.play_evo(key, exhaust, pos, rot, false, Vec3::ZERO, dist, &[("throttle", throttle)]);
            }
            if nitro_on {
                fx.play_evo(key | 2, nitro, pos, rot, false, st.vel, dist, &[]);
            }
        }
    }
}

/// The chassis' throttle +136 as the sim's tick left it (F-21480 copies it as it is): the pedal after the pad's dead zone
/// and the rev limiter's cut, 1 while the nitrous burns; the input handed to the sim where the exact sim does not run.
pub fn chassis_throttle(st: &crate::physics::CarState, input: &crate::physics::CarInput) -> f32 {
    match st.veh.as_deref() {
        Some(v) if v.by.exact => if v.nitro_burning { 1.0 } else { v.rev.left },
        _ => input.throttle,
    }
}

/// The braking smoke of a traffic car's wheels - its render instance's update F-66358, every frame for each of the
/// car's wheels: on while the car's brake-lock bit is set (the byte +557 of the render update's index, bit 7: traffic.rs
/// `Lock::drawn`) or the wheel's collider record +20, the ambient wheel's slide (physics/loose.rs `AmbWheel::slide`, kept
/// between activations as its compression is), the effect "wheel_s_asphalt_ambient" plays at the wheel bone's world
/// place (its matrix row 3: the hub raised by the compression, traffic.rs `roll_wheels`' frame) lowered by the y of the
/// bone's rest place (F-91983 +4, the hub height `roll_wheels` reads too), unturned (F-11877), with no velocity or
/// evolution; off, the effect is stopped (F-50844: no more particles, those out live on - `Particles::play`'s
/// rule for a key not asked for). The effect rule's own distances are the only cull.
/// `cars` with `slide(uid, wheel)`; `alpha` the render blend.
pub fn traffic_brake_smoke(fx: &mut Particles, cars: &[CarInstance], slide: impl Fn(u32, usize) -> bool, cam: Vec3, alpha: f32) {
    let smoke = joaat("wheel_s_asphalt_ambient");
    for (ci, c) in cars.iter().enumerate() {
        // (the traffic cars' render instances: a riding car, or a parked one held in the pool - no lock bit, so only
        // a sliding wheel)
        if c.rail.is_none() && !c.park_held {
            continue;
        }
        let Some(hubs) = c.model.own_wheels else { continue };
        let locked = c.rail.as_ref().is_some_and(|r| r.lock.drawn());
        let pose = c.render_pose(alpha);
        let m = Mat4::from_rotation_translation(pose.rot, pose.pos);
        let tilted_up = c.rail.as_ref().and_then(|r| r.tilt.quat()).map_or(Vec3::Y, |t| t * Vec3::Y);
        for i in 0..c.state.wheels.len().min(6) {
            let Some(h) = hubs[i] else { continue };
            if !(locked || slide(c.uid, i)) {
                continue;
            }
            let lift = (Vec3::Y * 2.0 - tilted_up) * c.amb_lift[i];
            let mut p = m.transform_point3(h + lift);
            p.y -= h.y;
            let key = 0x5442_524B_0000_0000 | ((ci as u64) << 4) | i as u64;
            // (MCLA_TRAFFIC_SMOKE_LOG=1: a line when a wheel's smoke starts)
            static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *LOG.get_or_init(|| std::env::var_os("MCLA_TRAFFIC_SMOKE_LOG").is_some()) && !fx.is_alive(key) {
                log::info!("traffic smoke {} wheel {} at {:.2} {:.2} {:.2} lock {} slide {} v {:.1}", c.model.name, i, p.x, p.y, p.z, locked, slide(c.uid, i), c.state.vel.length());
            }
            fx.play_tinted(key, smoke, p, [1.0; 3], [0.0; 3], 1.0, p.distance(cam), &[]);
        }
    }
}
