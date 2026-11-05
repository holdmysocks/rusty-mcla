//! A car's tune built part by part from its set-up's stages (F-55728), the opponent manager's random free character
//! (F-19687) and the car config's cached mod value (F-69770), as pure functions. Rules: docs/reference/ai/tune_build.md.
//!
//! The tune build runs when a car's set-up has no mod value (byte +60 = 255, i.e. -1: the player's car and opponents the
//! scripts give none): the car's update first blends base and mods with t = 0 (the base), then F-55728 writes about 150
//! words again, each blended with a weight that the set-up's stage bytes give through the `vehMods` records (five
//! readers: engine, transmission, suspension, brakes, tires), and scales the wheels' grip and two aero dampings by the
//! set-up's percent bytes +61..+63. The records come from `tune/vehicle/default.vehMods`, which is not on the disc: the
//! table keeps its constructor's records (`VehMods::default()`), whose part values are all 0, so every weight is 0.

#![allow(dead_code)]

use mcla_formats::rage::simtune::SimTune;

// ---------------------------------------------------------------------------------------------------------------------
// F-19687: OpponentManager_FindRandomAvailableCharacter

/// The manager's own multiply-with-carry state (manager +48 low word, +52 carry). Its constructor (F-67966) seeds it with 1:
/// low 1, carry 0x10001 (F-37317: low = s + (s == 0), carry = s ^ rot16(s)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManagerRng {
    pub lo: u32,
    pub hi: u32,
}

impl Default for ManagerRng {
    fn default() -> Self {
        ManagerRng { lo: 1, hi: 0x10001 }
    }
}

/// F-19687: one step of the manager's generator, `(lo & 0x7FFFFFFF) % count` (count = the character map's entry count,
/// manager +42), then the walk of the map in its order (buckets in turn, each chain from its head): the entry at that
/// place if it is free (byte +48 == 0), else the first free one after it; when none after it is free, the first free
/// one of the whole map; `None` when all are used. `used` = the used flags in the map's walk order. Returns the place in
/// that order. A count of 0 stops the original (a trap before the state is written); here `None`, the state kept.
pub fn find_random_available(rng: &mut ManagerRng, count: u16, used: &[bool]) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let x = rng.lo as u64 * 0x5CDC_FAA7 + rng.hi as u64;
    rng.lo = x as u32;
    rng.hi = (x >> 32) as u32;
    let mut skip = (rng.lo & 0x7FFF_FFFF) % count as u32;
    let mut first = None;
    for (i, &u) in used.iter().enumerate() {
        if first.is_none() && !u {
            first = Some(i);
        }
        if skip == 0 {
            if !u {
                return Some(i);
            }
        } else {
            skip -= 1;
        }
    }
    first
}

// ---------------------------------------------------------------------------------------------------------------------
// F-69770: mcCarConfigData_GetModValue

/// F-69770: the car config's cached mod value (config +76), 100 without a config. The value is written when the config
/// is picked (F-46809 GetCarConfigDataByVehicleName / F-70276 GetCarConfigData: the matched config, and each "nearest class" candidate as it becomes the
/// best so far, gets the band's mod value F-83091 of the player's car at that moment) and cleared to 0 with the used
/// marks (OpponentManager_ClearUsedCharacters, F-20163).
pub fn car_config_mod_value(cached: Option<i32>) -> i32 {
    cached.unwrap_or(100)
}

// ---------------------------------------------------------------------------------------------------------------------
// F-55728: the tune build per part

/// The `vehMods` records, each as the original holds it (16 bit words): word 0 kind, 1 stage count, 2..5 the share (%)
/// of stage 0..3, then the part's values (% of the upgrade each field group gets at a full share).
#[derive(Clone, Debug, PartialEq)]
pub struct VehMods {
    /// Exhaust, Headers, Intake, Turbo, Upgrade, Chip, Nitro: values EngineHP, EngineRPM, GearRatios, Drag, Wheelie,
    /// Burnout, Nitro, Wobble, Backfire, Mass
    pub engine: [[i16; 16]; 7],
    /// Clutch, Gears: Shift, Boost, NumGears, GearRatios, Mass
    pub trans: [[i16; 11]; 2],
    /// ShocksAndSpring, SwayBar: Spring, Shock, Drift, Turn, SSS, SimCG, Spin, Axle, Inertia, AngDamp
    pub suspension: [[i16; 16]; 2],
    /// Brakes: BrakeCoef, HandbrakeCoef
    pub brake: [[i16; 8]; 1],
    /// Tires: Tread, Sidewall, Compound, Friction
    pub tire: [[i16; 10]; 1],
}

impl Default for VehMods {
    /// The table's constructor F-88535 (its element constructors): shares 0 / 50 / 100 / 100 (tires 0 / 33 / 67 / 100), every
    /// value 0. `tune/vehicle/default.vehMods` is not on the disc, so the game keeps these.
    fn default() -> Self {
        let rec = |kind: i16, n: usize| {
            let mut r = [0i16; 16];
            r[..6].copy_from_slice(&[kind, 2, 0, 50, 100, 100]);
            if kind == 4 {
                r[..6].copy_from_slice(&[4, 3, 0, 33, 67, 100]);
            }
            r[n..].fill(0);
            r
        };
        let cut = |r: [i16; 16], out: &mut [i16]| out.copy_from_slice(&r[..out.len()]);
        let mut m = VehMods { engine: [rec(0, 16); 7], trans: [[0; 11]; 2], suspension: [rec(3, 16); 2], brake: [[0; 8]; 1], tire: [[0; 10]; 1] };
        for t in m.trans.iter_mut() {
            cut(rec(1, 11), t);
        }
        cut(rec(2, 8), &mut m.brake[0]);
        cut(rec(4, 10), &mut m.tire[0]);
        m
    }
}

/// The set-up's bytes the build reads: the 13 stage bytes (+28..+40: engine 7, transmission 2, suspension 2, brakes,
/// tires) and the three percent bytes +61, +62, +63 (100 from the constructor: rear grip, front grip, aero damping).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stages {
    pub stages: [u8; 13],
    pub percent: [u8; 3],
}

impl Stages {
    /// From a set-up record (the 8176-byte mcCarConfig of a `.xcc` or a garage slot).
    pub fn of_setup(cfg: &[u8]) -> Option<Stages> {
        if cfg.len() < 64 {
            return None;
        }
        let mut s = Stages::default();
        s.stages.copy_from_slice(&cfg[28..41]);
        s.percent.copy_from_slice(&cfg[61..64]);
        Some(s)
    }
}

/// The five readers' sums (F-69310 engine, F-30859 transmission, F-88632 suspension, F-31497 brakes, F-46157 tires, each
/// with the scaling F-45608): for each field group the sum over the
/// category's parts of trunc(value x share x 0.01), share = the record's word 2 + its stage; 16 bit words.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PartSums {
    pub engine: [i16; 10],
    pub trans: [i16; 5],
    pub suspension: [i16; 10],
    pub brake: [i16; 2],
    pub tire: [i16; 4],
}

/// One reader: `recs` the category's records one after the other (`n` words each), one stage byte per record.
fn read_parts<const K: usize>(recs: &[i16], n: usize, stages: &[u8]) -> [i16; K] {
    let mut acc = [0u16; K];
    for (i, &st) in stages.iter().enumerate() {
        // (the share word is read from the record array at 2 + stage + n i: a stage over 3 reads past the four shares)
        let share = recs.get(2 + st as usize + n * i).copied().unwrap_or(0);
        let w = share as f32 * 0.01;
        for (k, a) in acc.iter_mut().enumerate() {
            let v = (recs[n * i + 6 + k] as f32 * w) as i32;
            *a = a.wrapping_add(v as u16);
        }
    }
    acc.map(|a| a as i16)
}

impl PartSums {
    pub fn of(mods: &VehMods, s: &Stages) -> PartSums {
        let flat = |v: &[i16]| v.to_vec();
        PartSums {
            engine: read_parts(&flat(&mods.engine.concat()), 16, &s.stages[0..7]),
            trans: read_parts(&flat(&mods.trans.concat()), 11, &s.stages[7..9]),
            suspension: read_parts(&flat(&mods.suspension.concat()), 16, &s.stages[9..11]),
            brake: read_parts(&flat(&mods.brake.concat()), 8, &s.stages[11..12]),
            tire: read_parts(&flat(&mods.tire.concat()), 10, &s.stages[12..13]),
        }
    }
}

/// The original's blend of one word: (b - a) rounded, then fused with t and a.
fn lerp1(a: f32, b: f32, t: f32) -> f32 {
    (((b - a) as f64).mul_add(t as f64, a as f64)) as f32
}

/// The vector unit's blend (vsubfp, vmaddfp: the product rounded before the sum).
fn lerp_v(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// An integer word: float(b - a) and float(a) fused with t, truncated.
fn lerp_i(a: i32, b: i32, t: f32) -> i32 {
    (b.wrapping_sub(a) as f32 as f64).mul_add(t as f64, a as f32 as f64) as f32 as i32
}

/// An integer word blended as floats and rounded half away from zero (+0.5 when not below 0, else -0.5, truncated).
fn lerp_round(a: i32, b: i32, t: f32) -> i32 {
    let v = lerp1(a as f32, b as f32, t);
    (if v >= 0.0 { v + 0.5 } else { v - 0.5 }) as i32
}

/// A sum word as a weight: (s16) x 0.01.
fn weight(v: i16) -> f32 {
    v as f32 * 0.01
}

/// F-55728: the tune of a car whose set-up has no mod value (+60 = 255), from the base and mods sets as `SimTunes::parse`
/// reads them (the set-up's wheels written over both by `SimTunes::fit` already), the set-up's stage bytes, the vehMods
/// records, whether the car is a motorcycle (F-54299), and mcCarModelTune's stock FenderHeight front / rear (+56, +64)
/// and +176 for the air ride's drops. Returns the built tune: the t = 0 blend (the base) with the words F-55728 writes.
/// Not in `SimTune` and so not returned: mcCarSimTune +288 (Backfire), vehAeroTune +64 Drag (EngineDrag), the vectors'
/// fourth words.
#[allow(clippy::too_many_arguments)]
pub fn build(base: &SimTune, mods: &SimTune, s: &Stages, vm: &VehMods, bike: bool, fender_height: [f32; 2], air_floor: f32) -> SimTune {
    let p = PartSums::of(vm, s);
    let mut o = SimTune::lerp(base, mods, 0.0);
    let (b, m) = (base, mods);
    macro_rules! l {
        ($w:expr; $($($f:ident).+),+) => { $( o.$($f).+ = lerp1(b.$($f).+, m.$($f).+, $w); )+ };
    }
    macro_rules! v {
        ($w:expr; $($($f:ident).+),+) => { $( o.$($f).+ = lerp_v(b.$($f).+, m.$($f).+, $w); )+ };
    }
    let [e_hp, e_rpm, e_gear, _e_drag, e_wheelie, e_burnout, e_nitro, e_wobble, _e_backfire, e_mass] = p.engine;
    let [t_shift, t_boost, t_gears, t_gear, t_mass] = p.trans;
    let [s_spring, s_shock, s_drift, s_turn, s_sss, s_cg, s_spin, s_axle, s_inertia, s_angdamp] = p.suspension;
    let [b_brake, b_handbrake] = p.brake;
    let [r_tread, r_sidewall, r_compound, r_friction] = p.tire;

    // EngineHP: the engine's torque tables over the base's node count, the count itself the base's
    let w = weight(e_hp);
    let n = (b.engine.count as usize).min(11);
    o.engine.count = b.engine.count;
    for k in 0..2 {
        for i in 0..n {
            o.engine.words[k][i] = lerp_i(b.engine.words[k][i], m.engine.words[k][i], w);
        }
    }
    let nn = (o.engine.count as usize).min(11);
    o.engine.torque = o.engine.words[0][..nn].iter().map(|&v| v as f32).collect();
    o.engine.idle = o.engine.words[1][..nn].iter().map(|&v| v as f32).collect();
    l!(w; engine.s.max_hp, engine.s.inertia, gyro.hill_torque, gyro.drift_torque);
    l!(weight(e_rpm); engine.s.idle_rpm, engine.s.opt_rpm, engine.s.max_rpm);
    // (EngineDrag: vehAeroTune +64, not in the port's tune)
    l!(weight(e_wheelie); gyro.nosie_angle, gyro.wheelie_angle, gyro.wheelie_rate, gyro.wheelie, gyro.wheelie_limit, gyro.wheelie_damp,
        gyro.wheelie_boost_acc, gyro.wheelie_boost_duration);
    l!(weight(e_burnout); s.burn_threshold, s.burn_boost, s.burn_decval, gyro.burnout_rate, gyro.burnout_limit, gyro.burnout_turn,
        gyro.burnout_turn_rev, gyro.burnout_lean, gyro.burnout_grip);
    if !bike {
        l!(weight(e_wobble); gyro.wobble_amplitude, gyro.wobble_frequency);
    }
    // Nitro: the shot count an integer rounded, its low half stored
    let w = weight(e_nitro);
    o.nitro_shots = lerp_round(b.nitro_shots as u16 as i16 as i32, m.nitro_shots as u16 as i16 as i32, w) as u16 as f32;
    l!(w; nitro.boost_amount, nitro.boost_exp, nitro.boost_time);
    // (Backfire: mcCarSimTune +288, not in the port's tune)

    let w = weight(t_shift);
    l!(w; engine.s.gcl, trans.s.gear_change_delay, engine.s.upshift_throttle, engine.s.downshift_throttle, engine.s.engage_rpm,
        trans.s.upshift_bias, trans.s.upshift_bias_low, trans.s.downshift_bias_min, trans.s.downshift_bias_max);
    o.trans.max_downshifts = lerp_round(b.trans.max_downshifts as i32, m.trans.max_downshifts as i32, w) as u32;
    l!(w; trans.s.gear_change_inertia, trans.s.gear_change_match_rpm);
    l!(weight(t_boost); engine.s.boost_torque, engine.s.boost_duration);
    // NumGears: rounded, its low half sign-extended
    o.trans.gears = lerp_round(b.trans.gears as i32, m.trans.gears as i32, weight(t_gears)) as i16 as i32 as u32;

    let w = weight(b_brake);
    l!(w; wheel_front.brake_coef, wheel_back.brake_coef);
    if bike {
        l!(w; gyro.wobble_amplitude, gyro.wobble_frequency);
    }
    l!(weight(b_handbrake); wheel_front.handbrake_coef, wheel_back.handbrake_coef);

    l!(weight(s_spring); wheel_front.suspension_extent, wheel_front.travel_extent, wheel_front.suspension_limit, wheel_front.travel_limit,
        wheel_front.suspension_factor, wheel_back.suspension_extent, wheel_back.travel_extent, wheel_back.suspension_limit,
        wheel_back.travel_limit, wheel_back.suspension_factor);
    l!(weight(s_shock); wheel_front.suspension_damp_coef, wheel_front.suspension_damp_factor, wheel_back.suspension_damp_coef,
        wheel_back.suspension_damp_factor);
    l!(weight(s_drift); gyro.drift_thrust, gyro.drift_decay, gyro.drift);
    l!(weight(s_turn); gyro.turn_factor, gyro.turn, gyro.turn_limit, gyro.turn_damp, gyro.turn_bias, gyro.turn_oversteer, input.steer_rate,
        input.return_rate, input.countersteer_factor, input.steer_input_exp, wheel_front.steering_limit);
    l!(weight(s_sss); s.sss_value, s.sss_threshold_min, s.sss_threshold_max);
    let w = weight(s_cg);
    v!(w; s.com_x, s.com_y, s.com_z);
    l!(w; s.drift_cg);
    l!(weight(s_spin); gyro.spin, gyro.rev_spin);
    let w = weight(s_axle);
    o.axle_front = [lerp1(b.axle_front[0], m.axle_front[0], w), lerp1(b.axle_front[1], m.axle_front[1], w)];
    o.axle_back = [lerp1(b.axle_back[0], m.axle_back[0], w), lerp1(b.axle_back[1], m.axle_back[1], w)];
    v!(weight(s_inertia); s.size_x, s.size_y, s.size_z, s.inertia_x, s.inertia_y, s.inertia_z);
    let w = weight(s_angdamp);
    v!(w; aero.ang_c_damp_x, aero.ang_c_damp_y, aero.ang_c_damp_z, aero.ang_vel_damp_x, aero.ang_vel_damp_y, aero.ang_vel_damp_z,
        aero.ang_vel2_damp_x, aero.ang_vel2_damp_y, aero.ang_vel2_damp_z);
    l!(w; aero.down, aero.drag_min, aero.drag_max);

    l!(weight(r_tread); s.friction_handling_standard);
    l!(weight(r_friction); tire_fric.skidding_threshold, tire_fric.spinning_fric, tire_fric.melt_rate, tire_fric.recover_rate, s.tire_melt_damp_factor);
    l!(weight(r_sidewall); wheel_front.disp_limit_lat, wheel_front.disp_limit_long, wheel_front.damp_coef_lat, wheel_front.damp_coef_long,
        wheel_back.disp_limit_lat, wheel_back.disp_limit_long, wheel_back.damp_coef_lat, wheel_back.damp_coef_long);
    l!(weight(r_compound); wheel_front.static_fric, wheel_front.sliding_fric, wheel_front.optimum_slip, wheel_back.static_fric,
        wheel_back.sliding_fric, wheel_back.optimum_slip);
    // (the hydraulics and the air ride's rises and rates blended with 0: the base's, as the t = 0 blend left them)
    // the air ride's drops from the wheels as the set-up left them: the smaller of FenderHeight - radius and
    // radius - floor (the first unless it is greater), plus SuspensionOffset
    for axle in 0..2 {
        let wt = if axle == 0 { &o.wheel_front } else { &o.wheel_back };
        let (x, y) = (fender_height[axle] - wt.radius, wt.radius - air_floor);
        let d = if x > y { y } else { x } + wt.suspension_offset;
        if axle == 0 {
            o.air_ride.front_drop = d;
        } else {
            o.air_ride.back_drop = d;
        }
    }

    // GearRatios: the engine's and the transmission's sums together; Mass the same
    let w = (e_gear as i32 + t_gear as i32) as f32 * 0.01;
    l!(w; trans.s.low_gear_mph, trans.s.high_gear_mph, trans.s.rev_gear_mph, trans.s.gear_bias, engine.s.engage_cvt);
    let w = (e_mass as i32 + t_mass as i32) as f32 * 0.01;
    l!(w; s.mass);

    // the set-up's percent bytes: +62 the front wheels' grip, +61 the rear's, +63 two aero dampings
    let pct = |k: usize| s.percent[k] as i8 as f32 * 0.01;
    let (rear, front, aero) = (pct(0), pct(1), pct(2));
    o.wheel_front.static_fric *= front;
    o.wheel_front.sliding_fric *= front;
    o.wheel_back.static_fric *= rear;
    o.wheel_back.sliding_fric *= rear;
    o.aero.ang_c_damp_y *= aero;
    o.aero.ang_vel2_damp_y *= aero;
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcla_formats::rage::simtune::SimTunes;
    use mcla_formats::rsc::Resource;

    /// F-19687 and F-69770 run offline on their own code (work/racerai/tunebuild/cpick: random maps of 1 - 40 characters
    /// in 1 - 13 buckets, random used marks and generator states): the place picked and the state after, bit for bit.
    #[test]
    fn pick_and_mod_value_match_the_original() {
        let (mut picks, mut mods) = (0, 0);
        for line in include_str!("testdata/tunebuild_pick_ref.txt").lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            match f[0] {
                "pick" => {
                    let n = |i: usize| f[i].parse::<i64>().unwrap();
                    let mut rng = ManagerRng { lo: n(1) as u32, hi: n(2) as u32 };
                    let used: Vec<bool> = f[4].bytes().map(|c| c == b'1').collect();
                    let got = find_random_available(&mut rng, n(3) as u16, &used).map_or(-1, |i| i as i64);
                    assert_eq!((got, rng.lo, rng.hi), (n(5), n(6) as u32, n(7) as u32), "{line}");
                    picks += 1;
                }
                "mod" => {
                    let cached = (f[1] == "1").then(|| f[2].parse::<i64>().unwrap() as u32 as i32);
                    assert_eq!(car_config_mod_value(cached) as u32, f[3].parse::<i64>().unwrap() as u32, "{line}");
                    mods += 1;
                }
                _ => {}
            }
        }
        assert!(picks >= 100 && mods >= 20);
    }

    /// A word of the built tune by the original's place: region (the live sim tune, a sub-tune by its pointer's offset, the
    /// gyro or input tune) and offset. None for words the port's tune does not hold.
    fn word(t: &SimTune, region: &str, off: u32) -> Option<u32> {
        let f = |v: f32| Some(v.to_bits());
        let wheel = |w: &mcla_formats::rage::simtune::WheelTune| match off {
            4 => f(w.radius),
            12 => f(w.disp_limit_lat),
            16 => f(w.disp_limit_long),
            24 => f(w.damp_coef_lat),
            28 => f(w.damp_coef_long),
            40 => f(w.steering_limit),
            60 => f(w.brake_coef),
            64 => f(w.handbrake_coef),
            72 => f(w.travel_limit),
            76 => f(w.travel_extent),
            80 => f(w.suspension_offset),
            84 => f(w.suspension_limit),
            88 => f(w.suspension_extent),
            96 => f(w.suspension_factor),
            100 => f(w.suspension_damp_coef),
            104 => f(w.suspension_damp_factor),
            112 => f(w.optimum_slip),
            116 => f(w.static_fric),
            120 => f(w.sliding_fric),
            _ => panic!("wheel +{off}"),
        };
        let (s, e, tr, a, g) = (&t.s, &t.engine, &t.trans, &t.aero, &t.gyro);
        match region {
            "sim" => match off {
                48 => f(s.com_x),
                52 => f(s.com_y),
                56 => f(s.com_z),
                64 => f(s.size_x),
                68 => f(s.size_y),
                72 => f(s.size_z),
                80 => f(s.inertia_x),
                84 => f(s.inertia_y),
                88 => f(s.inertia_z),
                112 => f(s.mass),
                212 => f(s.sss_value),
                216 => f(s.sss_threshold_min),
                220 => f(s.sss_threshold_max),
                224 => f(s.burn_threshold),
                232 => f(s.burn_boost),
                240 => f(s.burn_decval),
                276 => f(s.drift_cg),
                296 => f(s.friction_handling_standard),
                308 => f(s.tire_melt_damp_factor),
                60 | 76 | 92 | 288 => None,
                _ => panic!("sim +{off}"),
            },
            "s136" => wheel(&t.wheel_front),
            "s140" => wheel(&t.wheel_back),
            "s144" => match off {
                0 => f(e.s.inertia),
                4 => f(e.s.max_hp),
                8 => f(e.s.idle_rpm),
                12 => f(e.s.opt_rpm),
                16 => f(e.s.max_rpm),
                20 => f(e.s.engage_rpm),
                24 => f(e.s.engage_cvt),
                28 => f(e.s.gcl),
                32 => f(e.s.upshift_throttle),
                36 => f(e.s.downshift_throttle),
                40 => f(e.s.boost_duration),
                44 => f(e.s.boost_torque),
                52 => Some(e.count),
                56..=96 => Some(e.words[0][(off as usize - 56) / 4] as u32),
                100..=140 => Some(e.words[1][(off as usize - 100) / 4] as u32),
                _ => panic!("engine +{off}"),
            },
            "s148" => match off {
                0 => Some(tr.gears),
                4 => f(tr.s.rev_gear_mph),
                8 => f(tr.s.low_gear_mph),
                12 => f(tr.s.high_gear_mph),
                16 => f(tr.s.upshift_bias),
                20 => f(tr.s.upshift_bias_low),
                24 => f(tr.s.downshift_bias_min),
                28 => f(tr.s.downshift_bias_max),
                32 => Some(tr.max_downshifts),
                36 => f(tr.s.gear_bias),
                40 => f(tr.s.gear_change_delay),
                44 => f(tr.s.gear_change_inertia),
                48 => f(tr.s.gear_change_match_rpm),
                _ => panic!("trans +{off}"),
            },
            "s160" => f(t.axle_front[off as usize / 4]),
            "s164" => f(t.axle_back[off as usize / 4]),
            "s168" => match off {
                16 => f(a.ang_c_damp_x),
                20 => f(a.ang_c_damp_y),
                24 => f(a.ang_c_damp_z),
                32 => f(a.ang_vel_damp_x),
                36 => f(a.ang_vel_damp_y),
                40 => f(a.ang_vel_damp_z),
                48 => f(a.ang_vel2_damp_x),
                52 => f(a.ang_vel2_damp_y),
                56 => f(a.ang_vel2_damp_z),
                68 => f(a.down),
                88 => f(a.drag_min),
                92 => f(a.drag_max),
                28 | 44 | 60 | 64 => None,
                _ => panic!("aero +{off}"),
            },
            "s192" => match off {
                28 => f(t.tire_fric.skidding_threshold),
                32 => f(t.tire_fric.melt_rate),
                36 => f(t.tire_fric.recover_rate),
                40 => f(t.tire_fric.spinning_fric),
                _ => panic!("tire +{off}"),
            },
            "s196" => match off {
                // (the shot count is the high half of the word; the original's sth leaves the low half)
                0 => Some(t.nitro_shots as u32),
                4 => f(t.nitro.boost_exp),
                8 => f(t.nitro.boost_amount),
                12 => f(t.nitro.boost_time),
                _ => panic!("nitro +{off}"),
            },
            "s204" => {
                let r = &t.air_ride;
                f([r.front_rise, r.front_drop, r.back_rise, r.back_drop, r.rate_drop, r.rate_rise][off as usize / 4])
            }
            "s208" => {
                let h = &t.hydraulics;
                f([h.latch_time, h.front_rise, h.front_drop, h.back_rise, h.back_drop, h.rate_rise, h.rate_drop, h.suspension_extent, h.suspension_damp][off as usize / 4])
            }
            "gyro" => f(match off {
                24 => g.turn,
                28 => g.drift,
                32 => g.spin,
                36 => g.rev_spin,
                64 => g.wheelie,
                68 => g.turn_factor,
                76 => g.turn_limit,
                80 => g.turn_damp,
                84 => g.turn_bias,
                88 => g.turn_oversteer,
                144 => g.nosie_angle,
                148 => g.wheelie_angle,
                152 => g.wheelie_rate,
                156 => g.wheelie_limit,
                160 => g.wheelie_damp,
                172 => g.hill_torque,
                180 => g.drift_torque,
                184 => g.drift_thrust,
                188 => g.drift_decay,
                192 => g.wheelie_boost_acc,
                196 => g.wheelie_boost_duration,
                200 => g.wobble_frequency,
                204 => g.wobble_amplitude,
                208 => g.burnout_rate,
                212 => g.burnout_limit,
                216 => g.burnout_turn,
                220 => g.burnout_turn_rev,
                224 => g.burnout_lean,
                228 => g.burnout_grip,
                _ => panic!("gyro +{off}"),
            }),
            "input" => f(match off {
                72 => t.input.countersteer_factor,
                76 => t.input.steer_rate,
                80 => t.input.return_rate,
                84 => t.input.steer_input_exp,
                _ => panic!("input +{off}"),
            }),
            _ => panic!("{region}"),
        }
    }

    /// One case of the offline run of F-55728 (work/racerai/tunebuild/ctune): the resource the base / mods / gyro / input /
    /// model tunes were laid out in, the set-up's bytes, the vehMods records (none: the constructor's), and every word
    /// the original wrote (decimal).
    fn run_cases(text: &str) -> (usize, usize, Vec<String>) {
        let (mut cases, mut words, mut bad) = (0, 0, Vec::new());
        let mut lines = text.lines();
        while let Some(head) = lines.next() {
            let h: Vec<&str> = head.split_whitespace().collect();
            if h.first() != Some(&"case") {
                continue;
            }
            let res = lines.next().unwrap().strip_prefix("res ").unwrap();
            let out = lines.next().unwrap();
            let data: Vec<u8> = (0..res.len() / 2).map(|i| u8::from_str_radix(&res[2 * i..2 * i + 2], 16).unwrap()).collect();
            let r = Resource { rtype: 0, flags: 0, virtual_size: data.len(), physical_size: 0, data };
            let st = SimTunes::parse(&r).expect("tunes");
            let bike = h[4] == "1";
            let sb: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&h[5][2 * i..2 * i + 2], 16).unwrap()).collect();
            let mut s = Stages::default();
            s.stages.copy_from_slice(&sb[..13]);
            s.percent.copy_from_slice(&sb[13..16]);
            let vm = if h[6] == "-" {
                VehMods::default()
            } else {
                let v: Vec<i16> = h[6].split(',').map(|x| x.parse::<u32>().unwrap() as u16 as i16).collect();
                let mut m = VehMods::default();
                let mut it = v.into_iter();
                for r in m.engine.iter_mut() {
                    r.iter_mut().for_each(|w| *w = it.next().unwrap());
                }
                for r in m.trans.iter_mut() {
                    r.iter_mut().for_each(|w| *w = it.next().unwrap());
                }
                for r in m.suspension.iter_mut() {
                    r.iter_mut().for_each(|w| *w = it.next().unwrap());
                }
                m.brake[0].iter_mut().for_each(|w| *w = it.next().unwrap());
                m.tire[0].iter_mut().for_each(|w| *w = it.next().unwrap());
                m
            };
            let t = build(&st.base, &st.mods, &s, &vm, bike, [st.fender_height[0][0], st.fender_height[1][0]], st.air_floor);
            cases += 1;
            for w in out.split_whitespace().skip(2) {
                let (k, v) = w.split_once('=').unwrap();
                let (region, off) = k.split_once(':').unwrap();
                let (off, mut v) = (off.parse::<u32>().unwrap(), v.parse::<u64>().unwrap() as u32);
                if region == "s196" && off == 0 {
                    v >>= 16;
                }
                let Some(p) = word(&t, region, off) else { continue };
                words += 1;
                if p != v {
                    bad.push(format!("case {} ({}) {region}+{off}: port {p:08x} original {v:08x}", h[1], h[2]));
                }
            }
        }
        (cases, words, bad)
    }

    /// F-55728 against its own code run offline: real cars' base / mods tunes (some with the mods perturbed), random
    /// stage and percent bytes, motorcycles and cars, the constructor's vehMods records and random ones. A few cases
    /// here; all of them from work/racerai/tunebuild/ctune/cases.txt when present.
    #[test]
    fn the_tune_build_is_the_originals() {
        let (cases, words, bad) = run_cases(include_str!("testdata/tunebuild_tune_ref.txt"));
        assert!(cases >= 3 && words > 300 && bad.is_empty(), "{} of {words} words differ in {cases} cases:\n{}", bad.len(), bad.join("\n"));
        let full = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../work/racerai/tunebuild/ctune/cases.txt");
        let Ok(text) = std::fs::read_to_string(full) else { return };
        let (cases, words, bad) = run_cases(&text);
        println!("tune build: {cases} cases, {words} words, {} differ", bad.len());
        assert!(bad.is_empty(), "{} of {words} words differ in {cases} cases:\n{}", bad.len(), bad.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
    }

    /// With the constructor's records (the disc has no default.vehMods) every weight is 0: the build is the base set
    /// (and the percent bytes' scaling, the air ride's drops, the base's engine node count).
    #[test]
    fn without_records_the_build_is_the_base() {
        let p = PartSums::of(&VehMods::default(), &Stages { stages: [2; 13], percent: [100; 3] });
        assert_eq!(p, PartSums::default());
    }
}
