//! A car's wipeout and damage-out (mcCar::Update's crash flags, chassis +592 bits 0x10 / 0x04, and
//! mcCarDamage +44's timer) for the cars the game drives itself. The player's car runs the same transitions
//! in main.rs (`App::crash_bits` / `crash_timer`), together with what only the player has: the slow motion,
//! the DAMAGED OUT notice, the wreck's menu, the police. In the original the flags are each car's, so a
//! computer racer wipes out as the player does.
//!
//! The rules, as main.rs has them for the player:
//! - not crashed: a head-on (physics::headon, F-43509), a roll onto the side or roof on the ground (A-72058,
//!   0.2 of MaxDamage) or a motorcycle's blow of more than 1.8 WipeoutThresholds (F-54299) is a wipeout
//!   (F-69183): bit 0x10, the timer WipeoutDelay, the car upset (its centre of gravity raised, vehsim). Damage
//!   at 0.999 is a damage-out (F-70073): bits 0x14, the timer DamageOutDelay, or ExplosionDelay after a hit
//!   over ExplodeThreshold.
//! - crashed: the timer runs down while the car is under 4.47 m/s or all its wheels touch (F-32386); a
//!   motorcycle in the original's sim keeps its wipeout timer there (vehsim's `wipe`). The wipeout's end
//!   (A-23262) clears the bits and stands up a car lying over (up y under 0.7) where it is.
//!
//! - a damage-out's end (mcCar::Update F-53665, its damage-out branch, once the timer is under 0): a car that is not
//!   the local player's and not a police car (racer byte +465, 1 for police: their brain teleports them,
//!   F-14059) gets mcCar's vtable slot 1, the car reset F-46526 "Resetting car
//!   %s": the tune blend and wheel fit again, every part's reset - the damage object's F-15879 (points, timer and
//!   hit fields to 0), the chassis' (its +592 flags to 0, mcCarSim::Reset F-47310), the damage parts' repair
//!   F-75221 - with its place and heading kept. Here: `Outcome::reset`, the caller runs `vehsim::reset`.

use super::car::{CarParams, CarState};
use glam::Vec3;

/// chassis +592's crash bits and mcCarDamage +44 (seconds) of one car
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Crash {
    /// 0x10 wiped out, 0x04 damaged out (with 0x10)
    pub bits: u8,
    /// mcCarDamage +44: the time the crash still holds
    pub timer: f32,
}

impl Crash {
    pub fn wiped_out(&self) -> bool {
        self.bits & 0x10 != 0
    }
}

/// What a step of the crash flags did, for the caller (a car stood up must have its pose history reset).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    pub started: bool,
    pub damaged_out: bool,
    pub ended: bool,
    pub stood_up: bool,
    /// the damage-out is over and the car was repaired (F-46526): the caller resets its sim (`vehsim::reset`)
    pub reset: bool,
}

/// One step of a car's crash flags over `dt` seconds (the player's in main.rs, for every other car).
/// `cop`: a police car (racer +465), which is not reset at its damage-out's end.
pub fn step(crash: &mut Crash, st: &mut CarState, p: &CarParams, bike: bool, cop: bool, dt: f32) -> Outcome {
    let mut out = Outcome::default();
    let Some(vt) = p.veh.as_ref() else { return out };
    let dm = &vt.t.damage;
    let up_y = (st.rot * Vec3::Y).y;
    let owned = st.veh.as_deref().is_some_and(|vs| super::vehsim::wipe_owned(vs, vt));
    if crash.bits == 0 {
        // (read, not taken: the police's and the siren's "down" tests read it; it goes at the wipeout's end)
        let big = st.wipeout;
        let head_on = std::mem::take(&mut st.head_on);
        let rolled = up_y <= 0.1 && st.airborne_time <= 0.0 && st.damage < 0.999;
        if !head_on && rolled {
            st.damage = (st.damage + 0.2).min(1.0);
        }
        if st.damage >= 0.999 {
            let exploding = st.last_hit > dm.explode_threshold && dm.explode_threshold > 0.0;
            crash.bits = 0x14;
            crash.timer = if exploding { dm.explosion_delay } else { dm.damage_out_delay };
            out.started = true;
            out.damaged_out = true;
        } else if head_on || rolled || (bike && big > 1.8) {
            crash.bits = 0x10;
            crash.timer = dm.wipeout_delay;
            if let Some(vs) = st.veh.as_deref_mut().filter(|_| owned) {
                vs.wipe_hit = true;
            }
            st.upset = 1.0;
            out.started = true;
        }
        return out;
    }
    let speed = st.speed();
    let all_down = st.wheels.iter().enumerate().all(|(i, w)| w.grounded || (bike && i % 2 == 1));
    if owned && crash.bits & 4 == 0 {
        let vs = st.veh.as_deref().filter(|v| v.wipe.is_some() || v.wipe_hit);
        crash.timer = vs.map_or(0.0, |v| v.wipe.map_or(crash.timer, |w| w.0));
    } else if crash.timer > 0.0 && (speed < 4.47 || all_down) {
        crash.timer -= dt;
    }
    if crash.bits & 4 != 0 {
        if crash.timer < 0.0 && !cop {
            *crash = Crash::default();
            st.damage = 0.0;
            st.last_hit = 0.0;
            st.zones = [0.0; 4];
            st.dmg.repair();
            st.upset = 0.0;
            st.wipeout = 0.0;
            st.head_on = false;
            st.visible_hit = false;
            out.reset = true;
        }
    } else if crash.bits & 0x10 != 0 && crash.timer <= 0.0 {
        crash.bits = 0;
        st.upset = 0.0;
        st.wipeout = 0.0;
        out.ended = true;
        if up_y < 0.7 && !owned {
            let f = st.forward();
            let fresh = CarState::new(st.pos + Vec3::Y * 0.8, (-f.x).atan2(-f.z));
            let old = std::mem::replace(st, fresh);
            st.damage = old.damage;
            st.zones = old.zones;
            st.dmg = old.dmg;
            st.gear = old.gear;
            out.stood_up = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car() -> (CarParams, CarState) {
        let mut s = mcla_formats::rage::simtune::SimTune::default();
        s.damage.wipeout_delay = 3.0;
        s.damage.damage_out_delay = 5.0;
        let mut p = CarParams::default();
        p.veh = Some(std::sync::Arc::new(super::super::vehsim::VehTune::new(s)));
        (p, CarState::new(Vec3::ZERO, 0.0))
    }

    #[test]
    fn a_head_on_wipes_a_computer_car_out_and_it_ends() {
        let (p, mut st) = car();
        let mut c = Crash::default();
        st.head_on = true;
        assert!(step(&mut c, &mut st, &p, false, false, 1.0 / 60.0).started);
        assert_eq!((c.bits, c.timer, st.upset), (0x10, 3.0, 1.0));
        // (standing still: the timer runs down on real time)
        let mut ended = false;
        for _ in 0..181 {
            ended |= step(&mut c, &mut st, &p, false, false, 1.0 / 60.0).ended;
        }
        assert!(ended && c.bits == 0 && st.upset == 0.0);
    }

    #[test]
    fn a_wrecked_computer_car_is_reset_after_its_damage_out() {
        let (p, mut st) = car();
        let mut c = Crash::default();
        st.damage = 1.0;
        assert!(step(&mut c, &mut st, &p, false, false, 0.1).damaged_out);
        assert_eq!((c.bits, c.timer), (0x14, 5.0));
        let mut reset = 0;
        for _ in 0..100 {
            reset += step(&mut c, &mut st, &p, false, false, 0.1).reset as u32;
        }
        assert_eq!((reset, c, st.damage), (1, Crash::default(), 0.0));
        // (a police car keeps its damage-out: its brain teleports it)
        let (p, mut st) = car();
        let mut c = Crash::default();
        st.damage = 1.0;
        for _ in 0..100 {
            assert!(!step(&mut c, &mut st, &p, false, true, 0.1).reset);
        }
        assert_eq!(c.bits, 0x14);
    }
}
