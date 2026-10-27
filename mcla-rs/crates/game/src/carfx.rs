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

/// One frame of the effects for every car (call every tick while the world runs).
pub fn step(fx: &mut Particles, cars: &[CarInstance], cam: Vec3) {
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
        // exhaust: engine running (the port's engine always is), not in stealth, the car drawn, below 1.5 m/s;
        // evolution "throttle" = the throttle (chassis +136); no velocity
        // (approx.) "drawn" (G+118, the camera's value) is taken as always on: which views clear it is not known
        let exhaust_on = !gate && !c.pu_stealth && st.speed() < EXHAUST_MAX_SPEED;
        let car_m = Mat4::from_rotation_translation(st.rot, st.pos);
        // (the original pushes a bone's matrix, one 60 Hz frame old, on by the body's velocity / 60 - with a side
        // exhaust it takes the car's matrix instead; both come to the car's matrix of this frame, which is what
        // the port has here)
        for (k, a) in c.model.fx_anchors.iter().take(2).enumerate() {
            let w = car_m * *a;
            let pos = w.w_axis.truncate();
            let rot = Quat::from_mat3(&Mat3::from_cols(w.x_axis.truncate().normalize_or_zero(), w.y_axis.truncate().normalize_or_zero(), w.z_axis.truncate().normalize_or_zero())).normalize();
            let dist = pos.distance(cam);
            let key = 0x4341_5246_0000_0000 | ((ci as u64) << 4) | k as u64;
            if exhaust_on {
                fx.play_evo(key, exhaust, pos, rot, false, Vec3::ZERO, dist, &[("throttle", c.input.throttle)]);
            }
            if nitro_on {
                fx.play_evo(key | 2, nitro, pos, rot, false, st.vel, dist, &[]);
            }
        }
    }
}
