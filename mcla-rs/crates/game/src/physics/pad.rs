//! The player's pad as the original reads it (mcPlayerInput::Update, 0x822A2988), the part
//! that makes the steering value the chassis is handed (input object +24).
//!
//! Per frame the original does: stick -> dead zone -> exponent -> target T (input +84, with the
//! damaged car's pull); then 0x822A1520 moves the held value P (input +24) and the result is
//! clamped to -1..1. mcCarSim::Update copies P to chassis +132 every tick and steers with
//! P x the chassis's own speed factor (0x8232D800).
//!
//! The original runs this once a frame with the frame's step (0x827D7558), between a tick's
//! vehicle update and its physics step, every other tick. Stage A2: so does the port in normal
//! play (`PadHeld`, vehsim.rs) - and the pedals go through `pedals` below. MCLA_DBG_A2: once a
//! tick with the tick's step and the port's older pedal rules.

use super::exact::{approach, ease, fma, fnms, guest_asin};
use mcla_formats::rage::simtune::InputTune;

/// 0x8260C5C8: nothing inside the dead zone, the rest stretched over what is left.
pub fn dead_zone(x: f32, zone: f32) -> f32 {
    if x > zone {
        (x - zone) / (1.0 - zone)
    } else if x < -zone {
        (x + zone) / (1.0 - zone)
    } else {
        0.0
    }
}

/// The mapper's value of a trigger (0x822A4BB8 -> 0x822A49D0, type 8): the pad's byte over 255
/// (0x82003C04) through a dead zone of 0.05. mcPlayerInput then takes 0.2 more off (`TRIGGER_ZONE`).
/// Stage R (D + V: every logged frame of sb.0 / sd.2 / sd.3 / sf.*: byte 150 -> throttle 0.6769).
pub fn trigger_axis(byte: u8) -> f32 {
    dead_zone(byte as f32 * f32::from_bits(0x3B80_8081), 0.05)
}

/// The mapper's value of a stick's axis (0x822A4BB8 -> 0x822A48F0, types 9 / 11 / 1 on a stick):
/// (byte - 127.5) x (1 / 127.5) (0x82008F34, 0x82035788) kept in -1..1, through a dead zone of 0.1.
/// (The steering then goes through `steer_target`'s own 0.1.)
pub fn stick_axis(byte: u8) -> f32 {
    let v = (byte as f32 - 127.5) * f32::from_bits(0x3C00_8081);
    let v = if v < -1.0 { -1.0 } else if v > 1.0 { 1.0 } else { v };
    dead_zone(v, 0.1)
}

/// A trigger given as 0..1 (this port's pad layer) as the byte the original's pad holds.
pub fn trigger_byte(unit: f32) -> u8 {
    (unit.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A stick's axis given as -1..1 as the byte the original's pad holds (128 at rest).
pub fn stick_byte(unit: f32) -> u8 {
    (unit.clamp(-1.0, 1.0) * 127.5 + 127.5).round().clamp(0.0, 255.0) as u8
}

/// The throttle "stomp" (0x822A2EBC..0x822A2ED8, 0x822A3274..0x822A32A0; flag 0x2000 of input +40):
/// (max(new, 0.5) - max(old, 0.5)) x the frame's 1 / step above ThrottleInputVelocityThreshold.
/// `old`: input +12 as the frame before left it; `new`: this frame's. Stage R (V: the logged flag
/// of every frame: set for 0 -> 1 and 0.1 -> 1, clear for 0 -> 0.677 and 0.677 -> 1, threshold 10).
pub fn stomp(old: f32, new: f32, frame_rate: f32, t: &InputTune) -> bool {
    let a = if 0.5 - old >= 0.0 { 0.5 } else { old };
    let b = if 0.5 - new >= 0.0 { 0.5 } else { new };
    t.throttle_input_velocity_threshold > 0.0 && (b - a) * frame_rate > t.throttle_input_velocity_threshold
}

/// 0x8255FF78: mcPlayerInput's own factor for the car's speed (chassis +224): 1 with no
/// SSSThreshold, SSSValue from the threshold up, eased from 1 below it.
pub fn speed_factor(t: &InputTune, speed: f32) -> f32 {
    if t.sss_threshold == 0.0 {
        1.0
    } else if speed < t.sss_threshold {
        fma(ease(speed / t.sss_threshold), t.sss_value - 1.0, 1.0)
    } else {
        t.sss_value
    }
}

/// 0x822A2F74..0x822A3024: the stick (-1..1) through its dead zone (0.1; the sensitivity
/// setting `sens`, input +156, widens it when negative) and SteerInputExp (lowered by half the
/// setting's share, never under 0.8).
pub fn steer_target(stick: f32, sens: f32, t: &InputTune) -> f32 {
    let x = stick.clamp(-1.0, 1.0);
    let zone0 = 0.1f32;
    let zone1 = fnms(sens * zone0, 0.75, zone0);
    let zone = if zone0 - zone1 >= 0.0 { zone0 } else { zone1 };
    let x = dead_zone(x, zone);
    let exp0 = t.steer_input_exp;
    let exp1 = fnms(sens * exp0, 0.5, exp0);
    let exp = if 0.8 - exp1 >= 0.0 { 0.8 } else { exp1 };
    let size = (x.abs() as f64).powf(exp as f64) as f32;
    let sign = if x >= 0.0 { if -x >= 0.0 { 0.0 } else { 1.0 } } else { -1.0 };
    (size * sign).clamp(-1.0, 1.0)
}

/// What 0x822A1520 reads besides the held value and the target.
pub struct SteerIn {
    /// chassis +224: the car's speed as vehAero last wrote it
    pub speed: f32,
    /// the body's velocity along the instance's first row (its right), m/s
    pub sideways: f32,
    /// chassis +568: the chassis's own speed factor as its last update left it
    pub chassis_factor: f32,
    /// the front wheels' SteeringLimit
    pub steering_limit: f32,
    /// input +16 and +20: brake and handbrake
    pub brake: f32,
    pub handbrake: f32,
    /// input +156
    pub sens: f32,
    pub dt: f32,
    /// input byte +101: the cruise holds the road's speed (`cruise`)
    pub cruise: bool,
}

/// 0x822A1520: the held steering value `held` (input +24) moved towards the target.
/// Steering further out from the countersteer reference - the slip angle over the lock, times
/// CountersteerFactor - it approaches target x the speed factor at SteerRate (ReturnRate with
/// the brake or the handbrake fully on, between them in proportion); otherwise it goes back
/// towards the TARGET ITSELF at ReturnRate (SteerRate where there is none). Rates times
/// 1 + 0.33 x the sensitivity setting.
pub fn steer_step(held: f32, target: f32, t: &InputTune, i: &SteerIn) -> f32 {
    let k = speed_factor(t, i.speed);
    let slip = if i.speed > 1.0 { guest_asin((i.sideways / i.speed) as f64) as f32 } else { guest_asin(0.0) as f32 };
    let reference = ((i.chassis_factor * slip) / i.steering_limit).clamp(-1.0, 1.0) * t.countersteer_factor;
    let reference = if reference.is_nan() { 0.0 } else { reference };
    let harder = if i.brake - i.handbrake >= 0.0 { i.brake } else { i.handbrake };
    let rate_out = fma(harder, t.return_rate - t.steer_rate, t.steer_rate);
    let rate_back = if t.return_rate > 0.0 { t.return_rate } else { t.steer_rate };
    // (0x822A163C, stage R: while the cruise is on - input byte +101 - the steering goes out and comes back
    // at SteerRate whatever the brake, and the countersteer reference is nothing. V: sd.3, 11 frames.)
    let (reference, rate_out, rate_back) = if i.cruise { (0.0, t.steer_rate, t.steer_rate) } else { (reference, rate_out, rate_back) };
    let gain = fma(i.sens, 0.33, 1.0);
    let (rate_out, rate_back) = (gain * rate_out, gain * rate_back);
    let mut p = held;
    let out = (held <= reference && target <= reference && target <= held)
        || (held >= reference && target >= reference && target >= held);
    if out {
        approach(&mut p, k * target, rate_out, i.dt);
    } else if p < target {
        p = fma(i.dt, rate_back, p);
        if p > target {
            p = target;
        }
    } else if p > target {
        p = fnms(i.dt, rate_back, p);
        if p < target {
            p = target;
        }
    }
    p.clamp(-1.0, 1.0)
}

/// The stick's other axis, input +32 (0x822A30AC..0x822A3120; a motorcycle's rider leaning back / forward):
/// read only while the weight button was down the frame before (flag 1 of input +40; or flag 0x80) -
/// the mapper's value through a second dead zone of 0.2, approached at 5 a second; otherwise nothing at
/// once. Stage R (V: se.0's wheelie, sd.2's stick without the button).
pub fn lean_step(held: f32, byte: u8, button_was: bool, dt: f32) -> f32 {
    if !button_was {
        return 0.0;
    }
    let y = stick_axis(byte);
    let y = if y - -1.0 >= 0.0 { y } else { -1.0 };
    let y = if y - 1.0 >= 0.0 { 1.0 } else { y };
    let mut v = held;
    approach(&mut v, dead_zone(y, TRIGGER_ZONE), 5.0, dt);
    v
}

/// What the cruise keeps between frames: input byte +101 (on) and +108, the car's speed smoothed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CruiseHeld {
    pub on: bool,
    pub speed: f32,
}

/// What the cruise reads besides its own state.
pub struct CruiseIn {
    /// chassis +224, m/s
    pub speed: f32,
    /// input +104: the speed limit of the road the car is on, in mph (the world's; 40 and 55 seen). Nothing: 0.
    pub limit_mph: f32,
    /// the car answers its pad, its gearbox points forward (0x8255ADE0 > 0), no nitrous shot burns, the
    /// gyro's intent (+18) is nothing
    pub driving: bool,
    /// transmission +148: the downshift modulator (1 after a launch or wheelspin, 0 driving steadily)
    pub modulation: f32,
}

/// The cruise, mcPlayerInput 0x822A25E0 (stage R, D + V: sd.3's 598 frames, 23 of them cruising): called
/// after the pedals and the stomp test, ahead of the steering. The smoothed speed (+108) follows the car's
/// by a fifth a frame. Driving forward faster than 10 mph (4.47 m/s) on a road with a limit: while the
/// transmission's modulator is under 0.1 the cruise comes ON once the smoothed speed (x 2.237 = mph) is
/// within CruiseActivationSpeedRange x the limit of the limit; it goes off when the modulator passes 0.9,
/// or when not driving. While on, the throttle (input +12) is remade:
///   CruiseThrottleMin < throttle <= CruiseThrottleMax:  e = smoothed speed - limit x 0.447,
///       k = min(|e| x 0.745712, 1);  faster: 0.5 - 0.4 k;  slower: 0.5 + 0.4 k;  on it: 0.5   (and no stomp)
///   throttle <= Min:  throttle / Min x 0.5            throttle > Max:  0.5 + (throttle - Max) x 0.5 / (1 - Max)
/// so a half-pressed trigger holds the road's speed (0.5 is the engine's coasting pedal). Returns the
/// throttle and whether the stomp flag is to be cleared.
pub fn cruise(held: &mut CruiseHeld, throttle: f32, t: &InputTune, i: &CruiseIn) -> (f32, bool) {
    let mph = f32::from_bits(0x400F_2B02);
    held.speed = fma(i.speed - held.speed, f32::from_bits(0x3E4C_CCCD), held.speed);
    let limit = i.limit_mph;
    if i.driving && limit > 0.0 && i.speed > f32::from_bits(0x408F_0A3D) {
        if i.modulation < f32::from_bits(0x3DCC_CCCD) {
            // fmsubs: speed x 2.237 - limit, one rounding
            let off = super::exact::fms(held.speed, mph, limit);
            if off.abs() < t.cruise_activation_speed_range * limit {
                held.on = true;
            }
        } else if i.modulation > f32::from_bits(0x3F66_6666) {
            held.on = false;
        }
    } else {
        held.on = false;
    }
    if !held.on {
        return (throttle, false);
    }
    let (lo, hi) = (t.cruise_throttle_min, t.cruise_throttle_max);
    if throttle > lo && !(throttle > hi) {
        let e = fnms(limit, f32::from_bits(0x3EE4_DD2F), held.speed);
        let k = e.abs() * f32::from_bits(0x3F3E_E6FE);
        let k = if k >= 0.0 { k } else { 0.0 };
        let k = if k - 1.0 >= 0.0 { 1.0 } else { k };
        let out = if e > 0.0 {
            fnms(k, f32::from_bits(0x3ECC_CCCD), 0.5)
        } else if e < -0.0 {
            fma(k, f32::from_bits(0x3ECC_CCCC), 0.5)
        } else {
            0.5
        };
        (out, true)
    } else if !(throttle > lo) {
        let v = (throttle / lo) * 0.5;
        let v = if v >= 0.0 { v } else { 0.0 };
        (if v - 0.5 >= 0.0 { 0.5 } else { v }, false)
    } else {
        let v = ((throttle - hi) * 0.5) / (1.0 - hi) + 0.5;
        let v = if v - 0.5 >= 0.0 { v } else { 0.5 };
        (if v - 1.0 >= 0.0 { 1.0 } else { v }, false)
    }
}

/// What the pad's code keeps between frames for the pedals: the auto-reverse state (input +64:
/// 0 none, 1 forward, 2 reverse, 3 braking from forward, 4 braking from reverse) and what it
/// last handed the chassis - throttle (input +12 -> chassis +136), brake (+16 -> +140) and the
/// way the gearbox is to point (flag bits 2 / 4 of the input's word: +1 forward, -1 reverse).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pedals {
    pub state: u8,
    pub throttle: f32,
    pub brake: f32,
    pub way: i8,
}

impl Default for Pedals {
    fn default() -> Self {
        Pedals { state: 0, throttle: 0.0, brake: 0.0, way: 1 }
    }
}

/// What the pedal code reads besides its own state.
pub struct PedalIn {
    /// the right and left triggers, 0..1, as the pad gives them
    pub rt: f32,
    pub lt: f32,
    /// chassis +224: the car's speed as vehAero last wrote it
    pub speed: f32,
    /// the body's velocity along the car (minus its third row), m/s
    pub forward: f32,
    /// 0x82332098: a motorcycle - no throttle from the brake trigger in reverse, 0.1 at most
    pub bike: bool,
}

/// The triggers' dead zone (the float ahead of the table 0x822A2CD8 reads through r26).
pub const TRIGGER_ZONE: f32 = 0.2;

/// mcPlayerInput's pedals for one frame (0x822A2CD8..0x822A2EC8 with the auto-reverse state
/// machine 0x822A06C0), operation for operation: triggers through their dead zone; x = RT - LT
/// in -1..1; the state machine gives throttle / brake / way; the trigger that points the
/// gearbox's way is the throttle whatever the machine says, the other one brakes with 0.75 of
/// itself at least; BrakeInputExp and ThrottleInputExp; both kept in 0..1.
pub fn pedals(held: Pedals, t: &InputTune, i: &PedalIn) -> Pedals {
    let rt = dead_zone(i.rt, TRIGGER_ZONE);
    let lt = dead_zone(i.lt, TRIGGER_ZONE);
    let x = -lt + rt;
    let x = if x - -1.0 >= 0.0 { x } else { -1.0 };
    let x = if x - 1.0 >= 0.0 { 1.0 } else { x };
    // ---- 0x822A06C0 ----
    let mut state = held.state;
    let mut way: i8 = if state == 2 { -1 } else { 1 };
    let x = if x.abs() < f32::from_bits(0x38D1_B717) { 0.0 } else { x };
    let slow = t.auto_reverse_speed;
    if i.speed < slow && x == 0.0 {
        state = 0;
    }
    let (mut throttle, mut brake) = (0.0f32, 0.0f32);
    // (0.01: with it the chassis's pedal is not "released" while the car brakes)
    let creep = f32::from_bits(0x3C23_D70A);
    match state {
        1 => {
            if x < 0.0 {
                brake = -x;
                state = 3;
                throttle = creep;
            } else {
                throttle = x;
            }
        }
        2 => {
            if x > 0.0 {
                state = 4;
                brake = x;
            } else if x < 0.0 {
                throttle = if i.bike { if -x - 0.1 >= 0.0 { 0.1 } else { -x } } else { -x };
            }
        }
        3 => {
            if x > 0.0 {
                throttle = x;
                state = 1;
            } else if x == 0.0 {
            } else if i.forward < slow {
                brake = if i.bike { 0.0 } else { -x };
                state = 0;
            } else {
                brake = -x;
                throttle = creep;
            }
        }
        4 => {
            if x < 0.0 {
                throttle = -x;
                state = 2;
            } else if x == 0.0 {
                state = 0;
                throttle = creep;
            } else if i.forward > -slow {
                state = 0;
                brake = if i.bike { 0.0 } else { x };
            } else {
                throttle = 1.0;
                brake = x;
            }
        }
        _ => {}
    }
    if state == 0 {
        if x < -t.auto_reverse_input {
            way = -1;
            state = 2;
        } else if x > 0.0 {
            way = 1;
            state = 1;
        }
    }
    // ---- back in 0x822A2988 ----
    let larger = |a: f32, b: f32| if a - b >= 0.0 { a } else { b };
    if way > 0 {
        throttle = larger(throttle, rt);
        brake = larger(brake, lt * 0.75);
    } else {
        if !i.bike {
            throttle = larger(throttle, lt);
        }
        brake = larger(brake, rt * 0.75);
    }
    let shaped = |v: f32, exp: f32| {
        let p = (v as f64).powf(exp as f64) as f32;
        let p = if p >= 0.0 { p } else { 0.0 };
        if p - 1.0 >= 0.0 { 1.0 } else { p }
    };
    Pedals { state, throttle: shaped(throttle, t.throttle_input_exp), brake: shaped(brake, t.brake_input_exp), way }
}

/// The frame's state of the player's pad in the physics crate: the pedals, the steering value P
/// that the next update is handed, the rider's smoothed stick (input +32, 5 a second) and which
/// of the two ticks of a frame this is.
#[derive(Clone, Copy, Debug, Default)]
pub struct PadHeld {
    pub pedals: Pedals,
    pub steer_next: f32,
    pub stick_y: f32,
    pub stick_next: f32,
    pub second_tick: bool,
    pub live: bool,
    /// stage R: the cruise's state (input byte +101, +108)
    pub cruise: CruiseHeld,
    /// stage R: the weight button as the frame before had it (flag 1 of input +40)
    pub button: bool,
    /// the handbrake as the frame wrote it to the chassis (+144): the button is read once a frame, like the pedals
    pub handbrake: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tune() -> InputTune {
        InputTune { sss_threshold: 40.0, sss_value: 0.5, countersteer_factor: 1.0, steer_rate: 3.0, return_rate: 6.0, steer_input_exp: 1.5, ..Default::default() }
    }

    fn at(speed: f32, forward: f32, rt: f32, lt: f32) -> PedalIn {
        PedalIn { rt, lt, speed, forward, bike: false }
    }

    #[test]
    fn the_brake_trigger_brakes_then_reverses_and_the_throttle_brings_the_car_back() {
        let t = InputTune { auto_reverse_speed: 1.0, auto_reverse_input: 0.1, brake_input_exp: 1.0, throttle_input_exp: 1.0, ..tune() };
        // under way on the throttle
        let mut p = pedals(Pedals::default(), &t, &at(20.0, 20.0, 1.0, 0.0));
        assert_eq!((p.state, p.throttle, p.brake, p.way), (1, 1.0, 0.0, 1));
        // the brake at speed: braking, with a hundredth of throttle so the pedal is not "released"
        p = pedals(p, &t, &at(20.0, 20.0, 0.0, 1.0));
        assert_eq!((p.state, p.way), (3, 1));
        assert_eq!((p.throttle, p.brake), (f32::from_bits(0x3C23_D70A), 1.0));
        // slow enough: reverse is taken - the brake trigger is now the throttle
        p = pedals(p, &t, &at(0.5, 0.5, 0.0, 1.0));
        assert_eq!((p.state, p.way, p.throttle), (2, -1, 1.0));
        p = pedals(p, &t, &at(3.0, -3.0, 0.0, 1.0));
        assert_eq!((p.state, p.way, p.throttle, p.brake), (2, -1, 1.0, 0.0));
        // the throttle trigger while rolling back: the brake, and full throttle against the roll
        p = pedals(p, &t, &at(3.0, -3.0, 1.0, 0.0));
        assert_eq!((p.state, p.way, p.brake), (4, -1, 1.0));
        p = pedals(p, &t, &at(3.0, -3.0, 1.0, 0.0));
        assert_eq!((p.state, p.throttle, p.brake), (4, 1.0, 1.0));
        // stopped: forward again
        p = pedals(p, &t, &at(0.2, -0.2, 1.0, 0.0));
        assert_eq!((p.state, p.way, p.throttle), (1, 1, 1.0));
    }

    #[test]
    fn triggers_have_a_fifth_of_dead_travel_and_the_other_pedal_three_quarters() {
        let t = InputTune { auto_reverse_speed: 1.0, auto_reverse_input: 0.1, brake_input_exp: 1.0, throttle_input_exp: 0.5, ..tune() };
        let p = pedals(Pedals::default(), &t, &at(0.0, 0.0, 0.15, 0.0));
        assert_eq!((p.throttle, p.brake, p.state), (0.0, 0.0, 0));
        // both down, going forward: the throttle whole, the brake at 0.75
        let p = pedals(Pedals { state: 1, way: 1, ..Default::default() }, &t, &at(20.0, 20.0, 1.0, 1.0));
        assert_eq!((p.throttle, p.brake), (1.0, 0.75));
        // a motorcycle in reverse: a tenth of throttle at most - 0.316 after ThrottleInputExp 0.5
        let p = pedals(Pedals { state: 2, way: -1, ..Default::default() }, &t, &PedalIn { rt: 0.0, lt: 1.0, speed: 3.0, forward: 3.0, bike: true });
        assert_eq!(p.throttle, ((0.1f32 as f64).powf(0.5)) as f32);
    }

    /// The recordings' pedals: every pad frame of a recording as `gjkfix/padcheck.py` found a trigger
    /// pair for it (env MCLA_PAD_CASES: state, RT, LT, chassis +224, forward speed, bike, AutoReverseSpeed,
    /// AutoReverseInput, BrakeInputExp, ThrottleInputExp; then state, throttle, brake, way after).
    #[test]
    #[ignore]
    fn recorded_pedals() {
        let path = std::env::var("MCLA_PAD_CASES").expect("MCLA_PAD_CASES");
        let text = std::fs::read_to_string(path).unwrap();
        let (mut n, mut bad) = (0, 0);
        for line in text.lines() {
            let v: Vec<&str> = line.split('#').next().unwrap().split_whitespace().collect();
            if v.len() < 14 { continue; }
            let f = |i: usize| f32::from_bits(u32::from_str_radix(v[i], 16).unwrap());
            let t = InputTune { auto_reverse_speed: f(6), auto_reverse_input: f(7), brake_input_exp: f(8), throttle_input_exp: f(9), ..tune() };
            let held = Pedals { state: v[0].parse().unwrap(), ..Default::default() };
            let p = pedals(held, &t, &PedalIn { rt: f(1), lt: f(2), speed: f(3), forward: f(4), bike: v[5] == "1" });
            n += 1;
            if (p.state as i32, p.throttle.to_bits(), p.brake.to_bits(), p.way as i32) != (v[10].parse().unwrap(), f(11).to_bits(), f(12).to_bits(), v[13].parse().unwrap()) {
                bad += 1;
                if bad < 5 { println!("differs: {line} -> {p:?}"); }
            }
        }
        println!("recorded pedal frames: {n}, not reproduced: {bad}");
        assert!(n > 0 && bad == 0);
    }

    /// Stage R: every logged pad frame (`stageR/agB/padcheck2.py` writes the cases from the PADF records:
    /// the input object before its update, the raw bytes, the chassis; and the object one frame later).
    /// Env MCLA_PAD_FRAMES. Pedals + state + way, the stomp flag, the stick's target, the speed factor,
    /// the steering value, the handbrake - bit for bit.
    #[test]
    #[ignore]
    fn recorded_frames() {
        let path = std::env::var("MCLA_PAD_FRAMES").expect("MCLA_PAD_FRAMES");
        let text = std::fs::read_to_string(path).unwrap();
        let mut n = 0;
        let mut bad = std::collections::BTreeMap::<&str, (u32, String)>::new();
        let mut skipped = 0;
        let mut cruising = 0;
        let mut leaning = 0;
        for line in text.lines() {
            let g: Vec<Vec<&str>> = line.split('|').map(|p| p.split_whitespace().collect()).collect();
            if g.len() < 8 { continue; }
            let h = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
            let bike = g[0][2] == "1";
            let tf: Vec<f32> = g[1].iter().map(|s| h(s)).collect();
            let t = InputTune { auto_reverse_speed: tf[0], auto_reverse_input: tf[1], brake_input_exp: tf[2], throttle_input_exp: tf[3], steer_rate: tf[4], return_rate: tf[5], steer_input_exp: tf[6], countersteer_factor: tf[7], sss_threshold: tf[8], sss_value: tf[9], throttle_input_velocity_threshold: tf[10], cruise_throttle_min: tf[12], cruise_throttle_max: tf[13], cruise_activation_speed_range: tf[14], ..Default::default() };
            let steering_limit = tf[11];
            let state: u8 = g[2][0].parse().unwrap();
            let (thr0, p0) = (h(g[2][1]), h(g[2][3]));
            let w80 = u32::from_str_radix(g[2][4], 16).unwrap();
            let raw: Vec<u8> = g[3].iter().map(|s| s.parse().unwrap()).collect();
            let c: Vec<f32> = g[4].iter().map(|s| h(s)).collect();
            let after_state: u8 = g[5][0].parse().unwrap();
            let a: Vec<u32> = g[5][1..].iter().map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
            let extra: Vec<u32> = g[6].iter().map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
            // (a car that does not answer its pad - the rider off, wrecked - or with the damage's pull or a
            // sensitivity setting: not what this test makes)
            if extra[0] != 0 || extra[1] != 0 || extra[2] & 0x38 != 0 || w80 & 0x10 == 0 { skipped += 1; continue; }
            n += 1;
            let (v, r0, r2) = (&c[0..3], &c[3..6], &c[6..9]);
            let forward = -fma(r2[0], v[0], fma(r2[2], v[2], r2[1] * v[1]));
            let sideways = fma(r0[0], v[0], fma(r0[2], v[2], r0[1] * v[1]));
            let (speed, factor) = (c[9], c[10]);
            let held = Pedals { state, ..Default::default() };
            let p = pedals(held, &t, &PedalIn { rt: trigger_axis(raw[2]), lt: trigger_axis(raw[3]), speed, forward, bike });
            let mut miss = |what: &'static str, ok: bool| {
                if !ok { let e = bad.entry(what).or_insert((0, line.to_string())); e.0 += 1; }
            };
            // (group 8: input +108, byte +101 before; +104, +108, bytes +101 / +102 after; transmission +148,
            // its gear, the gyro's +18, the nitrous burning as the frame found it)
            let q = &g[7];
            let hq = |k: usize| u32::from_str_radix(q[k], 16).unwrap();
            let mut ch = CruiseHeld { on: hq(1) != 0, speed: f32::from_bits(hq(0)) };
            let driving = extra[2] & 0x38 == 0 && hq(7) >= 2 && hq(8) == 0 && hq(9) == 0;
            let (throttle, quiet) = cruise(&mut ch, p.throttle, &t, &CruiseIn { speed, limit_mph: f32::from_bits(hq(2)), driving, modulation: f32::from_bits(hq(6)) });
            miss("cruise on", ch.on == (hq(4) != 0));
            miss("cruise speed", ch.speed.to_bits() == hq(3));
            miss("cruise quiet", !ch.on || quiet == (hq(5) != 0));
            if ch.on { cruising += 1; }
            miss("state", p.state == after_state);
            miss("throttle", throttle.to_bits() == a[0]);
            miss("brake", p.brake.to_bits() == a[1]);
            miss("way", (p.way < 0) == (a[7] & 4 != 0));
            let hand = if raw[4] > 127 { 1.0f32 } else { 0.0 };
            miss("handbrake", hand.to_bits() == a[2]);
            miss("stomp", (stomp(thr0, p.throttle, 30.0, &t) && !quiet) == (a[6] & 0x2000 != 0));
            let target = steer_target(stick_axis(raw[0]), 0.0, &t);
            miss("target", target.to_bits() == a[4] || (target == 0.0 && a[4] & 0x7fff_ffff == 0));
            miss("speed factor", speed_factor(&t, speed).to_bits() == a[5]);
            let frame = f32::from_bits(0x3C88_8889) + f32::from_bits(0x3C88_8889);
            let next = steer_step(p0, target, &t, &SteerIn { speed, sideways, chassis_factor: factor, steering_limit, brake: p.brake, handbrake: hand, sens: 0.0, dt: frame, cruise: ch.on });
            let lean = lean_step(h(g[2][5]), raw[1], hq(10) & 0x81 != 0, frame);
            miss("lean", lean.to_bits() == a[8] || (lean == 0.0 && a[8] & 0x7fff_ffff == 0));
            if a[8] & 0x7fff_ffff != 0 { leaning += 1; }
            miss("steering", next.to_bits() == a[3] || (next == 0.0 && a[3] & 0x7fff_ffff == 0));
        }
        println!("recorded pad frames: {n} (skipped {skipped}), cruising in {cruising}, leaning in {leaning}");
        for (what, (count, line)) in &bad {
            println!("  {what}: {count} not reproduced; first: {line}");
        }
        assert!(n > 0 && bad.is_empty());
    }

    #[test]
    fn the_pads_bytes_become_the_games_values() {
        // a trigger: byte 150 is the 0.6769 of throttle the recordings show (through 0.05, 0.2 and the root)
        let t = InputTune { auto_reverse_speed: 1.0, auto_reverse_input: 0.5, brake_input_exp: 2.0, throttle_input_exp: 0.5, throttle_input_velocity_threshold: 10.0, ..tune() };
        let p = pedals(Pedals { state: 1, ..Default::default() }, &t, &PedalIn { rt: trigger_axis(150), lt: 0.0, speed: 7.0, forward: 7.0, bike: false });
        assert!((p.throttle - 0.6769).abs() < 1e-4);
        assert_eq!(trigger_axis(12), 0.0);
        assert_eq!(pedals(Pedals::default(), &t, &PedalIn { rt: trigger_axis(60), lt: 0.0, speed: 0.0, forward: 0.0, bike: false }).throttle, 0.0);
        // a stick: 128 at rest is inside the dead zone; the ends are the ends
        assert_eq!(stick_axis(128), 0.0);
        assert_eq!(stick_axis(255), 1.0);
        assert_eq!(stick_axis(0), -1.0);
        assert_eq!((trigger_byte(1.0), trigger_byte(0.0), stick_byte(0.0), stick_byte(-1.0), stick_byte(1.0)), (255, 0, 128, 0, 255));
        // the stomp is a frame's test: 0 -> 0.677 and 0.677 -> 1 are none at 30 a second, 0 -> 1 is
        assert!(!stomp(0.0, p.throttle, 30.0, &t));
        assert!(!stomp(p.throttle, 1.0, 30.0, &t));
        assert!(stomp(0.0, 1.0, 30.0, &t));
        assert!(stomp(0.1, 1.0, 30.0, &t));
    }

    #[test]
    fn a_half_pressed_trigger_holds_the_roads_speed() {
        let t = InputTune { cruise_throttle_min: 0.2, cruise_throttle_max: 0.8, cruise_activation_speed_range: 0.33, ..tune() };
        let mut c = CruiseHeld { on: false, speed: 24.0 };
        let at = |speed: f32, modulation: f32| CruiseIn { speed, limit_mph: 55.0, driving: true, modulation };
        // 24 m/s = 53.7 mph on a 55 mph road, the gearbox settled: on; a little slow -> a little over the half
        let (thr, quiet) = cruise(&mut c, 0.6769, &t, &at(24.0, 0.0));
        assert!(c.on && quiet && thr > 0.5 && thr < 0.9);
        // faster than the limit: under the half - the engine brakes
        c.speed = 27.0;
        assert!(cruise(&mut c, 0.6769, &t, &at(27.0, 0.0)).0 < 0.5);
        // the trigger's ends still do what they do: a floored pedal is a floored pedal, a lifted one coasts less
        assert_eq!(cruise(&mut c, 1.0, &t, &at(27.0, 0.0)), (1.0, false));
        assert_eq!(cruise(&mut c, 0.1, &t, &at(27.0, 0.0)).0, 0.25);
        // wheelspin / a launch (the modulator at 1) ends it; no road speed, no cruise
        assert_eq!(cruise(&mut c, 0.6769, &t, &at(27.0, 1.0)), (0.6769, false));
        assert!(!c.on);
        let mut c = CruiseHeld { on: false, speed: 24.0 };
        assert_eq!(cruise(&mut c, 0.6769, &t, &CruiseIn { speed: 24.0, limit_mph: 0.0, driving: true, modulation: 0.0 }), (0.6769, false));
    }

    #[test]
    fn the_stick_has_a_dead_zone_and_keeps_its_ends() {
        let t = tune();
        assert_eq!(steer_target(0.05, 0.0, &t), 0.0);
        assert_eq!(steer_target(1.0, 0.0, &t), 1.0);
        assert_eq!(steer_target(-1.0, 0.0, &t), -1.0);
        assert!(steer_target(0.5, 0.0, &t) < 0.5);
    }

    #[test]
    fn steering_goes_out_at_the_steer_rate_to_the_scaled_target_and_back_to_the_target() {
        let t = tune();
        let i = SteerIn { speed: 40.0, sideways: 0.0, chassis_factor: 1.0, steering_limit: 0.6, brake: 0.0, handbrake: 0.0, sens: 0.0, dt: 1.0 / 60.0, cruise: false };
        let mut p = 0.0;
        p = steer_step(p, 1.0, &t, &i);
        assert!((p - 3.0 / 60.0).abs() < 1e-6);
        for _ in 0..120 {
            p = steer_step(p, 1.0, &t, &i);
        }
        assert_eq!(p, 0.5);
        let back = steer_step(p, 0.0, &t, &i);
        assert!((back - (0.5 - 6.0 / 60.0)).abs() < 1e-6);
    }
}
