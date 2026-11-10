//! Small routines of the vehicle simulation in the form the original has them (the same operations in the same order,
//! the scalar unit's fused ones where the original fuses), compared with the generated code run offline
//! (`vehsim_small_tests`, tools/vehsim_small_reference.py). Where the play code has the same arithmetic it calls these.

use super::exact::{fma, fnms};
use super::loose_words::Words;
use super::vehsim::{builtin_torque, RPM};
use mcla_formats::rage::simtune::EngineScalars;

/// F-86064, the idle torque of an engine without an idle curve (+76 the idle speed, +72 the speed of best power, the tune's
/// +4 the power): sign(idle - w) x ((power x (1 / opt)) x 550) x ((1 / opt) x w), the sign by two `fsel`s - 0 where they are
/// equal, -1 for a NaN difference.
pub fn idle_fallback(max_hp: f32, opt: f32, idle: f32, w: f32) -> f32 {
    let d = idle - w;
    let k = 1.0 / opt;
    let below = if -d >= 0.0 { 0.0 } else { 1.0 };
    let sign = if d >= 0.0 { below } else { -1.0 };
    (sign * ((max_hp * k) * 550.0)) * (k * w)
}

/// F-48349, the torque for a pedal position: above half, ((pedal - 0.5) x 2) x (boost + multiplier x the torque at full
/// throttle), the boost (timer / duration) x boost torque where the duration is above 0; else the idle torque x (1 - 2 pedal).
pub fn pedal_torque(pedal: f32, timer: f32, mult: f32, boost_duration: f32, boost_torque: f32, torque: impl FnOnce() -> f32, idle: impl FnOnce() -> f32) -> f32 {
    if pedal > 0.5 {
        let base = mult * torque();
        let boost = if boost_duration > 0.0 { (timer / boost_duration) * boost_torque } else { 0.0 };
        ((pedal - 0.5) * 2.0) * (boost + base)
    } else {
        idle() * fnms(pedal, 2.0, 1.0)
    }
}

/// F-23005, the numbers of the engine's init: 1 / (max - opt)^2 (+84) and 1 / opt^3 (+88), from the speeds in rad/s.
pub fn init_reciprocals(opt: f32, max: f32) -> (f32, f32) {
    let span = max - opt;
    (1.0 / (span * span), 1.0 / ((opt * opt) * opt))
}

/// F-23005 on a tune: (+68 max speed, +72 speed of best power, +76 idle speed, +84, +88, +20 the torque there). With control
/// values (`torque`: the tune's integers, the curve through them) the best-power speed is found between idle and max
/// (Curve::peak), else it is OptRPM and the torque the built-in curve's.
pub fn engine_init(e: &EngineScalars, torque: &[f32]) -> [f32; 6] {
    let (idle, opt, max) = (e.idle_rpm * RPM, e.opt_rpm * RPM, e.max_rpm * RPM);
    let (c84, c88) = init_reciprocals(opt, max);
    let (best, t) = if torque.len() >= 3 {
        super::engine_curve::Curve::new(torque, idle, max, false).peak()
    } else {
        (opt, builtin_torque(e, opt))
    };
    [max, best, idle, c84, c88, t]
}

/// F-31519, the aero's drag numbers per top speed: 1 / (HighGearMPH x 0.447)^2, the square by the C library's pow
/// (F-21955, `caraudio::pow_guest`: its logarithm and exponential in double precision, then rounded to single). Where
/// the exact square lies half way between two floats the library's last bits decide the rounding, so an exact product
/// is not the same number there.
pub fn drag_over(high_gear_mph: f32) -> f32 {
    let top = high_gear_mph * f32::from_bits(0x3ee4dd2f);
    1.0 / (crate::caraudio::pow_guest(top as f64, 2.0) as f32)
}

/// F-31519's three stored numbers: (+24 Down, +28 DragMin, +32 DragMax) each times `drag_over`.
pub fn drag_numbers(high_gear_mph: f32, down: f32, drag_min: f32, drag_max: f32) -> [f32; 3] {
    let over = drag_over(high_gear_mph);
    [down * over, drag_min * over, drag_max * over]
}

/// F-75615, lying on the ground: the air time is not above 0 (a NaN is not above it) and the up row's y is under 0.1.
pub fn lying_on_ground(air_time: f32, up_y: f32) -> bool {
    !(air_time > 0.0) && up_y < 0.1
}

/// F-60656's load for each wheel of a car with hydraulics, before the wheel's own part: (stiffness - 1) kept between 0 and
/// 10 by two `fsel`s (a NaN gives 0) times the car's mass.
pub fn hyd_static_load(stiffness: f32, mass: f32) -> f32 {
    let a = stiffness - 1.0;
    let b = if a >= 0.0 { a } else { 0.0 };
    let c = b - 10.0;
    (if c >= 0.0 { 10.0 } else { b }) * mass
}

/// F-17224, what the wheel's setter F-51861 is given: the wheel's weight (+428) plus the load, not under 1.
pub fn wheel_weight_to_set(weight_428: f32, load: f32) -> f32 {
    let f = weight_428 + load;
    if f - 1.0 >= 0.0 { f } else { 1.0 }
}

/// F-11389, a wheel's share of the car's `weight` (wheel +428 as its set-up F-73903 and the weighing F-52728 give it to
/// F-51861): the weight over twice the wheel's distance along the car (+392, `at_z`) times its distance from the centre
/// of mass on the other side (`cg_z + at_z`); where the wheel stands wider (+384, `at_x`) than its tyre (+424, `width`)
/// halved the same way across. (The original first takes the chassis +104 from +392 when a global byte is set; that
/// byte is 0 in the image and nothing writes it.)
pub fn wheel_share(weight: f32, at_x: f32, at_z: f32, width: f32, cg_x: f32, cg_z: f32) -> f32 {
    let w = weight / (at_z.abs() * 2.0) * (cg_z + at_z).abs();
    if at_x.abs() > width { (cg_x + at_x).abs() / (at_x.abs() * 2.0) * w } else { w }
}

/// What F-73903 (the wheel's slot 1) weighs a wheel with: ((the sim tune's +124 x the body's mass +192) x 9.8).
pub fn wheel_init_weight(gravity: f32, mass: f32) -> f32 {
    (gravity * mass) * 9.8f32
}

/// F-60656 for one wheel, as the car's refresh F-92950 runs it at the hydraulics' switch-on (F-71695 raises the record's
/// dirty bit 0x80, F-53649): the wheel's slot 1 F-73903 weighs it afresh (`share`, F-11389), then F-17224 gives F-51861
/// that weight plus the hydraulics' load, not under 1.
pub fn hyd_refresh_weight(share: f32, stiffness: f32, mass: f32) -> f32 {
    wheel_weight_to_set(share, hyd_static_load(stiffness, mass))
}

/// F-60656: for each wheel (`weights`: their +428 words) the value its setter is given, empty where the routine does
/// nothing - the hydraulics fitted and (on, or a pattern running). The wheels' reset (their slot 1) comes first.
pub fn hyd_wheel_loads(fitted: bool, on: bool, pattern: bool, stiffness: f32, mass: f32, weights: &[f32]) -> Vec<f32> {
    if !(fitted && (on || pattern)) {
        return Vec::new();
    }
    let load = hyd_static_load(stiffness, mass);
    weights.iter().map(|&w| wheel_weight_to_set(w, load)).collect()
}

/// F-10823: what the hydraulics write into a wheel tune: (+76 and +88 the extent, +92 1.5, +100 the damp).
pub fn hyd_wheel_tune(extent: f32, damp: f32) -> [f32; 4] {
    [extent, extent, 1.5, damp]
}

/// F-42631, vehStuck's constructor on the words of its object: the vtable, chassis 0, flags 1, timer 0.0, the saved place
/// 0 0 0 0 (a vector store), state 0; the other words are left.
pub fn stuck_ctor<W: Words>(w: &mut W, at: u32) {
    w.set(at + 16, 0);
    w.set(at, (0x8207u32 << 16) | 0xEE64);
    w.set4(at + 32, [0.0; 4]);
    w.set(at + 8, 0);
    w.set(at + 12, 1);
    w.set(at + 56, 0);
}

/// F-96311, a force at a point: the body takes the force `f` (its slot 32), then the torque arm x f (slot 33) with the arm
/// from the centre of mass `cg` (chassis +112) carried through the chassis rows (+304: three rows and the place) to the
/// point `p`: p - (cg . rows + place), each coordinate (1 place + z row) + (y row + x row); the vector unit's lanes, the
/// w lane of the arm p.w - cg.x (the merge puts the centre's first word there). Returns (force, torque).
pub fn force_at_point(rows: &[[f32; 4]; 4], cg: [f32; 4], f: [f32; 4], p: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let at = |c: usize| (1.0 * rows[3][c] + cg[2] * rows[2][c]) + (cg[1] * rows[1][c] + cg[0] * rows[0][c]);
    let arm = [p[0] - at(0), p[1] - at(1), p[2] - at(2), p[3] - cg[0]];
    (f, cross_lanes(arm, f))
}

/// The vector unit's cross product of two vectors a and b as the original forms it (a vnmsubfp: minus the difference of
/// two rounded products, so an exact zero comes out as -0): lane i = -((b[j] a[i]) - (b[i] a[j])), (i, j) = (2, 1),
/// (0, 2), (1, 0); the w lane -(b.w a.w - b.w a.w).
fn cross_lanes(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let t = |i: usize, j: usize| -((b[j] * a[i]) - (b[i] * a[j]));
    [t(2, 1), t(0, 2), t(1, 0), -(b[3] * a[3] - b[3] * a[3])]
}

/// F-29993, the helper that sets a body's pending turn and push: the body's slot 28 gets `v`, its slot 27 gets the
/// vector `0 - (u x v)` (for the constant vector the first argument points at, and 0). Returns (slot 28's vector, slot 27's).
pub fn turn_and_push(v: [f32; 4], u: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    // v11 = the vnmsubfp of the two products (the cross above with v for a and u for b, the zero's sign included), then 0 - v11
    let c = cross_lanes(v, u);
    (v, c.map(|x| 0.0 - x))
}

/// F-61274, a suspension lever's two reciprocals (+100, +104): with a bone (its rows 0 and 2 as 16 words, the place at
/// +48) d = the wheel's vector - the place; a100 = 1 / (row 0 . d), a104 = 1 / (row 2 . d), each dot product
/// z (dz) fused with y (dy) fused with x (dx), and 0 where it is not above 0.01 in size; without a bone a100 = 1 / the wheel
/// vector's x and a104 = 1 / its z under the same rule.
pub fn lever_reciprocals(bone: Option<&[f32; 16]>, wheel: [f32; 4]) -> (f32, f32) {
    let rec = |x: f32| if x.abs() > 0.01 { 1.0 / x } else { 0.0 };
    match bone {
        Some(b) => {
            let d = [wheel[0] - b[12], wheel[1] - b[13], wheel[2] - b[14]];
            let dot = |r: usize| fma(b[r], d[0], fma(b[r + 1], d[1], b[r + 2] * d[2]));
            (rec(dot(0)), rec(dot(8)))
        }
        None => (rec(wheel[0]), rec(wheel[2])),
    }
}

/// F-68237's step for one fixed, all-but-upright contact: the centre of mass's height comes 0.8 of the way to
/// SettleCG x ModelOffset's height, fused (tune +272 and +36).
pub fn settle_step(cg_y: f32, settle_cg: f32, model_offset_y: f32) -> f32 {
    fma(super::exact::fms(settle_cg, model_offset_y, cg_y), f32::from_bits(0x3f4c_cccd), cg_y)
}

/// A contact as F-68237 meets it: enabled (the byte +165 has its top bit), against something fixed (an instance with a
/// valid id whose table entry's low two bits are 2) and the normal's y as it stands against the car.
#[derive(Clone, Copy, Debug)]
pub struct SettleTouch {
    pub enabled: bool,
    pub fixed: bool,
    pub normal_y: f32,
}

/// F-68237 over the contacts of the car in their order: for each enabled one - against something fixed with |normal y|
/// under 0.1 - the step above; the still timer (+588) is zeroed once any enabled contact is met while the instance's
/// flag word has bit 0x10. Returns the centre (x, y, z, w) and whether +588 was zeroed.
pub fn settle_cg(mut cg: [f32; 4], settle_cg: f32, model_offset_y: f32, touches: &[SettleTouch], flag_bit: bool) -> ([f32; 4], bool) {
    let mut reset = false;
    for t in touches.iter().filter(|t| t.enabled) {
        if t.fixed && t.normal_y.abs() < 0.1 {
            cg[1] = settle_step(cg[1], settle_cg, model_offset_y);
        }
        reset |= flag_bit;
    }
    (cg, reset)
}

/// F-72737, the speed along the car: -(row 2 . v), the sum z-term first, fused.
pub fn set_down_forward(row2: [f32; 3], v: [f32; 3]) -> f32 {
    -fma(row2[0], v[0], fma(row2[2], v[2], row2[1] * v[1]))
}

/// F-72737, a shaft's speed: minus the forward speed, over its first wheel's radius where it has a wheel.
pub fn set_down_shaft(fwd: f32, first_radius: Option<f32>) -> f32 {
    let s = -fwd;
    first_radius.map_or(s, |r| s / r)
}

/// F-72737, the engine's speed: minus gear ratio x shaft speed, not under the idle speed (a `fsel`: a NaN gives the idle
/// speed) and then not over the speed of best power (a `fsel` again).
pub fn set_down_engine(ratio: f32, shaft: f32, idle: f32, peak: f32) -> f32 {
    let w = -(ratio * shaft);
    let up = if w - idle >= 0.0 { w } else { idle };
    if up - peak >= 0.0 { peak } else { up }
}
