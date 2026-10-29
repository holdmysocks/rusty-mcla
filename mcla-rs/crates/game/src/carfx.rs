//! The exhaust and nitrous effects of every car on the original's simulation (player, racers, police):
//! records 0 / 1 (`veh_exhaust`) and 2 / 3 (`veh_nitro`) of the car's effects object, kept playing at the two
//! effect anchors while their conditions hold - F-21480 (the update), F-87024 (start), F-21553 (anchors).
//! The original has no backfire effect: the engine's backfire is a sound only.
//! Rule: `docs/reference/carparts/car_exhaust_fx_rule.md`.

use crate::particles::Particles;
use crate::vehicles::CarInstance;
use glam::{Mat3, Mat4, Quat, Vec3};
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
        // exhaust: engine running (`engine_off`: a scene's car_off), not in stealth, the car drawn, below 1.5 m/s;
        // "drawn" = G+118, bit 0 of the racer's draw mode (F-95074, F-16712 reads it): 0 only for the player's car in
        // the bumper view - not during the crash camera, the Damageout event, a cutscene or a menu - which is the
        // port's `hide_player`; every other car always (no cull or distance test)
        // evolution "throttle" = the chassis' throttle +136 as the sim's tick left it (F-21480 copies it as it is):
        // the pedal after the pad's dead zone and the rev limiter's cut, 1 while the nitrous burns; no velocity
        let drawn = !(c.is_player && player_hidden);
        let exhaust_on = !gate && !c.engine_off && !c.pu_stealth && drawn && st.speed() < EXHAUST_MAX_SPEED;
        let throttle = match st.veh.as_deref() {
            Some(v) if v.by.exact => if v.nitro_burning { 1.0 } else { v.rev.left },
            _ => c.input.throttle,
        };
        let car_m = Mat4::from_rotation_translation(st.rot, st.pos);
        // (the original pushes a bone's matrix, one 60 Hz frame old, on by the body's velocity / 60 - with a side
        // exhaust it takes the car's matrix instead; both come to the car's matrix of this frame, which is what
        // the port has here)
        // (a bone of the rear bumper stands where the bumper hangs: its global follows the part's pose, F-14987 -
        // the same move as the bumper's draw, `Bumper::draw_delta`)
        let rear_m = car_m * c.state.parts.bumpers[0].as_ref().filter(|b| b.detached.is_none()).map_or(Mat4::IDENTITY, |b| b.draw_delta());
        for (k, (a, on_rear)) in c.model.fx_anchors.iter().take(2).enumerate() {
            let w = if *on_rear { rear_m } else { car_m } * *a;
            let pos = w.w_axis.truncate();
            let rot = Quat::from_mat3(&Mat3::from_cols(w.x_axis.truncate().normalize_or_zero(), w.y_axis.truncate().normalize_or_zero(), w.z_axis.truncate().normalize_or_zero())).normalize();
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
