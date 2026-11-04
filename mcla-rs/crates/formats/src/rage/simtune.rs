//! The original's car simulation tuning, read from a car's `.r50` by the member offsets
//! the executable registers (see `data/tune_schema.txt`): `vehSim` components hold a
//! `mcCarSimTune` with pointers to the wheel, engine, transmission, drivetrain and aero
//! blocks; `vehGyro` holds `mcGyroTune`; `vehInput` holds `mcPlayerInputTune`. Every car
//! has a base and a fully upgraded ("mods") set; the game blends them field by field.

use super::Rd;
use crate::rsc::Resource;

/// One word of a blend as the original's lerps do it: (b - a) rounded, then fused with t and a (F-94542 and the
/// sub-tunes' lerps).
fn lerp1(a: f32, b: f32, t: f32) -> f32 {
    (((b - a) as f64).mul_add(t as f64, a as f64)) as f32
}

/// An integer word blended as the original does it: the difference and a converted to float, fused with t, truncated.
fn lerp_i(a: i32, b: i32, t: f32) -> i32 {
    (b.wrapping_sub(a) as f32 as f64).mul_add(t as f64, a as f32 as f64) as f32 as i32
}

/// The same rounded to the nearest integer (half away from zero), mcCarTireFricTune's +24.
fn lerp_round(a: i32, b: i32, t: f32) -> i32 {
    let v = (((b as f32 - a as f32) as f64).mul_add(t as f64, a as f32 as f64)) as f32;
    (if v >= 0.0 { v + 0.5 } else { v - 0.5 }) as i32
}

/// The same with the vector unit (vsubfp, vmaddfp: the product rounded before the sum), mcCarSimTune's vectors +32..+108.
fn lerp_v(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

macro_rules! tune_struct {
    ($(#[$m:meta])* $name:ident { $($field:ident @ $off:expr),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default)]
        pub struct $name { $(pub $field: f32,)* }
        impl $name {
            pub fn read(rd: &Rd, base: usize) -> Self {
                Self { $($field: rd.f32(base + $off),)* }
            }
            pub fn lerp(a: &Self, b: &Self, t: f32) -> Self {
                Self { $($field: lerp1(a.$field, b.$field, t),)* }
            }
        }
    };
}

tune_struct!(
    /// vehWheelTune
    WheelTune {
        radius @ 4, width @ 8, disp_limit_lat @ 12, disp_limit_long @ 16, disp_limit_vert @ 20,
        damp_coef_lat @ 24, damp_coef_long @ 28, surface_drag_lat @ 32, surface_drag_long @ 36,
        steering_limit @ 40, camber_limit @ 44, wobble_limit @ 48, axle_limit @ 52, pivot_offset @ 56,
        brake_coef @ 60, handbrake_coef @ 64, steering_offset @ 68, travel_limit @ 72, travel_extent @ 76,
        suspension_offset @ 80, suspension_limit @ 84, suspension_extent @ 88, suspension_clamp @ 92,
        suspension_factor @ 96, suspension_damp_coef @ 100, suspension_damp_factor @ 104,
        bottomout_elasticity @ 108, optimum_slip @ 112, static_fric @ 116, sliding_fric @ 120,
    }
);

tune_struct!(
    /// vehEngineTune without its control value arrays
    EngineScalars {
        inertia @ 0, max_hp @ 4, idle_rpm @ 8, opt_rpm @ 12, max_rpm @ 16, engage_rpm @ 20, engage_cvt @ 24,
        gcl @ 28, upshift_throttle @ 32, downshift_throttle @ 36, boost_duration @ 40, boost_torque @ 44,
        boost_throttle_threshold @ 48,
    }
);

tune_struct!(
    /// vehTransTune (the gear count sits in front of it)
    TransScalars {
        rev_gear_mph @ 4, low_gear_mph @ 8, high_gear_mph @ 12, upshift_bias @ 16, upshift_bias_low @ 20,
        downshift_bias_min @ 24, downshift_bias_max @ 28, gear_bias @ 36, gear_change_delay @ 40,
        gear_change_inertia @ 44, gear_change_match_rpm @ 48, shift_modulator_approach @ 52, shift_modulator_decay @ 56,
    }
);

tune_struct!(
    /// vehDrivetrainTune
    DrivetrainTune {
        inertia @ 0, ang_drag @ 4, brake_dynamic_coef @ 8, brake_static_coef @ 12, diff_low_speed_limit @ 16,
        diff_high_speed_limit @ 20, diff_high_limit_speed @ 24,
    }
);

tune_struct!(
    /// mcCarAeroTune (vehAeroTune with the drag range; vehAeroTune's own Drag, +64, is not blended - the live tune keeps
    /// its constructor's 0.4 - and nothing reads it)
    AeroTune {
        ang_c_damp_x @ 16, ang_c_damp_y @ 20, ang_c_damp_z @ 24, ang_vel_damp_x @ 32, ang_vel_damp_y @ 36, ang_vel_damp_z @ 40,
        ang_vel2_damp_x @ 48, ang_vel2_damp_y @ 52, ang_vel2_damp_z @ 56, down @ 68, drag_min @ 88, drag_max @ 92,
    }
);

tune_struct!(
    /// mcCarFluidTune (vehFluidTune's damping and current with the buoyancy range)
    FluidTune { damp @ 4, current_scale @ 12, buoyancy_max @ 16, buoyancy_min @ 20, sink_rate @ 24 }
);

tune_struct!(
    /// mcCarTireFricTune
    TireFricTune { gear_threshold_bits @ 24, skidding_threshold @ 28, melt_rate @ 32, recover_rate @ 36, spinning_fric @ 40 }
);

tune_struct!(
    /// mcCarNitroTune
    NitroTune { boost_exp @ 4, boost_amount @ 8, boost_time @ 12, fov @ 16, fov_out_time @ 20, fov_in_time @ 24, flame_time @ 28, flame_size @ 32 }
);

tune_struct!(
    /// the scalar part of mcCarSimTune
    SimScalars {
        model_offset_x @ 32, model_offset_y @ 36, model_offset_z @ 40, com_x @ 48, com_y @ 52, com_z @ 56,
        size_x @ 64, size_y @ 68, size_z @ 72, inertia_x @ 80, inertia_y @ 84, inertia_z @ 88, mass @ 112,
        bound_friction @ 116, bound_elasticity @ 120, bound_gravity @ 124, sss_value @ 212, sss_threshold_min @ 216,
        sss_threshold_max @ 220, burn_threshold @ 224, burn_rev_threshold @ 228, burn_boost @ 232, burn_incval @ 236,
        burn_decval @ 240, burn_damage @ 244, burn_weightxfer @ 248, rev_limit_min @ 252, rev_limit_max @ 256,
        rev_limit_delay @ 260, rev_limit_throttle_rate @ 264, upset_cg @ 268, settle_cg @ 272, drift_cg @ 276,
        head_on_cg_offset @ 280, head_on_spin_mult @ 284, friction_handling_wipeout @ 292, friction_handling_standard @ 296,
        landing_damp_factor @ 300, aero_damp_factor @ 304, tire_melt_damp_factor @ 308, aero_damp_delay @ 312,
        landing_elasticity @ 316, bound_clearance @ 320, bound_angle @ 324, bound_bumper @ 328, bound_widebody @ 332,
        bound_bumper_width @ 336,
    }
);

tune_struct!(
    /// mcGyroTune: the arcade assists
    GyroTune {
        turn @ 24, drift @ 28, spin @ 32, rev_spin @ 36, yaw @ 40, pitch @ 44, roll @ 48, lean @ 52, lean_angle @ 56,
        wheelie @ 64, turn_factor @ 68, turn_limit @ 76, turn_damp @ 80, turn_bias @ 84, turn_oversteer @ 88,
        lean_factor @ 92, lean_angle_max @ 96, lean_rate_up @ 100, lean_rate_dn @ 104, lean_limit @ 108, lean_damp @ 112,
        lean_impulse_up @ 116, lean_impulse_dn @ 120, lean_speed_min @ 124, lean_steer_limit @ 128,
        lean_throttle_limit @ 132, lean_steer_bias @ 136, lean_offset_cg @ 140, nosie_angle @ 144, wheelie_angle @ 148,
        wheelie_rate @ 152, wheelie_limit @ 156, wheelie_damp @ 160, roll_limit @ 164, roll_damp @ 168, hill_torque @ 172,
        hill_grade @ 176, drift_torque @ 180, drift_thrust @ 184, drift_decay @ 188, wheelie_boost_acc @ 192,
        wheelie_boost_duration @ 196, wobble_frequency @ 200, wobble_amplitude @ 204, burnout_rate @ 208,
        burnout_limit @ 212, burnout_turn @ 216, burnout_turn_rev @ 220, burnout_lean @ 224, burnout_grip @ 228,
        reverse_acc @ 232, reverse_limit @ 236, gravity_pitch @ 240,
    }
);

tune_struct!(
    /// mcCarHydraulicsTune: how far each end rises and drops (m), how fast (m/s), and the
    /// suspension a car on hydraulics rides on
    HydraulicsTune {
        latch_time @ 0, front_rise @ 4, front_drop @ 8, back_rise @ 12, back_drop @ 16, rate_rise @ 20, rate_drop @ 24,
        suspension_extent @ 28, suspension_damp @ 32, suspension_stiffness @ 36, mass_scale_up @ 44,
    }
);

tune_struct!(
    /// mcCarSSTurboTune: the slipstream turbo
    SsTurboTune {
        boost_exp @ 0, boost_amount @ 4, use_time @ 8, regen_time @ 12, falloff_time @ 16, charged_falloff_time @ 20,
        stay_full_time @ 24, lose_charge_speed @ 28, min_speed @ 32, full_speed @ 36, fov_out_time @ 40, fov_in_time @ 44, fov @ 48, dist @ 52,
        blur_off_time @ 56, blur_start_amount @ 60, envelope_length @ 64, envelope_width1 @ 68, envelope_width2 @ 72,
        flame_time @ 76, flame_size @ 80, smoke_size @ 84,
    }
);

tune_struct!(
    /// mcCarDamageTune (component `vehDamage`): damage points and impact speeds (m/s)
    DamageTune {
        max_damage @ 24, med_damage @ 28, impact_threshold @ 32, wipeout_threshold @ 36, weightxfer_threshold @ 40,
        explode_threshold @ 44, jolt_threshold @ 48, fatal_blow @ 52, wipeout_delay @ 56, damage_out_delay @ 60,
        explosion_delay @ 64, explode_impulse @ 72,
    }
);

tune_struct!(
    /// mcCarCamShakeTune (component `vehCamShake`)
    CamShakeTune {
        magnitude_multiplier @ 24, rumble_threshold_speed_pct @ 28, rumble_max_shake_speed_pct @ 32,
        impact_amplitude_min @ 36, impact_duration_min @ 40, impact_amplitude_max @ 44, impact_duration_max @ 48,
    }
);

tune_struct!(
    /// mcPlayerInputTune
    InputTune {
        sss_threshold @ 24, sss_value @ 28, auto_reverse_speed @ 32, auto_reverse_input @ 36, countersteer_factor @ 72,
        steer_rate @ 76, return_rate @ 80, steer_input_exp @ 84, brake_input_exp @ 88, throttle_input_exp @ 92,
        throttle_input_velocity_threshold @ 96, cruise_throttle_min @ 100, cruise_throttle_max @ 104,
        cruise_activation_speed_range @ 108, steer_damage_bias @ 112, steer_damage_speed_threshold @ 116,
    }
);

#[derive(Clone, Debug, Default)]
pub struct EngineTune {
    pub s: EngineScalars,
    /// torque (as the game uses it, N m) at evenly spaced engine speeds: the first value one
    /// step below idle, the last at the maximum
    pub torque: Vec<f32>,
    /// torque off the throttle, same spacing (positive at idle, engine braking above)
    pub idle: Vec<f32>,
    /// the tune's own words: the node count (+52) and all eleven words of each table (+56, +100), which the blend
    /// takes whatever the count
    pub count: u32,
    pub words: [[i32; 11]; 2],
}

impl EngineTune {
    fn with_words(s: EngineScalars, count: u32, words: [[i32; 11]; 2]) -> Self {
        let n = (count as usize).min(11);
        let table = |k: usize| words[k][..n].iter().map(|&v| v as f32).collect();
        EngineTune { s, torque: table(0), idle: table(1), count, words }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TransTune {
    /// gears including reverse and neutral
    pub gears: u32,
    pub max_downshifts: u32,
    pub s: TransScalars,
}

/// One complete set (base or mods) of a car.
#[derive(Clone, Debug, Default)]
pub struct SimTune {
    pub s: SimScalars,
    /// 0 rear, 1 front, 2 all wheel drive
    pub drivetrain_type: u32,
    pub wheel_front: WheelTune,
    pub wheel_back: WheelTune,
    pub engine: EngineTune,
    pub trans: TransTune,
    pub powertrain: DrivetrainTune,
    pub freetrain: DrivetrainTune,
    /// anti-roll: (torque coefficient, damping) per axle
    pub axle_front: [f32; 2],
    pub axle_back: [f32; 2],
    pub aero: AeroTune,
    pub fluid: FluidTune,
    pub tire_fric: TireFricTune,
    pub nitro: NitroTune,
    pub gyro: GyroTune,
    pub input: InputTune,
    pub cam_shake: CamShakeTune,
    pub damage: DamageTune,
    pub ss_turbo: SsTurboTune,
    pub hydraulics: HydraulicsTune,
    /// mcCarNitroTune's shot count (a 16 bit count in front of its floats)
    pub nitro_shots: f32,
    /// a car with a live rear axle: how far along the car its model's `axle0` bone stands
    /// (not in the `.r50`: the car's loader reads it from that part's skeleton)
    pub axle_bone_z: Option<f32>,
    /// that bone's matrix as authored: its three rows and its place
    pub axle_bone: Option<[[f32; 3]; 4]>,
    /// the front wheel's steering axis in the car's frame where it is not straight up (a
    /// motorcycle's raked fork: the second row of its fork bone's matrix; wheel +400)
    pub steer_axis: Option<[f32; 3]>,
}

impl SimTune {
    /// Blend two sets (`t` = share of the upgrades fitted).
    pub fn lerp(a: &SimTune, b: &SimTune, t: f32) -> SimTune {
        let l2 = |x: [f32; 2], y: [f32; 2]| [lerp1(x[0], y[0], t), lerp1(x[1], y[1], t)];
        SimTune {
            // (the vectors with the vector unit, the bound's five words +320..+336 copied from a)
            s: SimScalars {
                model_offset_x: lerp_v(a.s.model_offset_x, b.s.model_offset_x, t),
                model_offset_y: lerp_v(a.s.model_offset_y, b.s.model_offset_y, t),
                model_offset_z: lerp_v(a.s.model_offset_z, b.s.model_offset_z, t),
                com_x: lerp_v(a.s.com_x, b.s.com_x, t),
                com_y: lerp_v(a.s.com_y, b.s.com_y, t),
                com_z: lerp_v(a.s.com_z, b.s.com_z, t),
                size_x: lerp_v(a.s.size_x, b.s.size_x, t),
                size_y: lerp_v(a.s.size_y, b.s.size_y, t),
                size_z: lerp_v(a.s.size_z, b.s.size_z, t),
                inertia_x: lerp_v(a.s.inertia_x, b.s.inertia_x, t),
                inertia_y: lerp_v(a.s.inertia_y, b.s.inertia_y, t),
                inertia_z: lerp_v(a.s.inertia_z, b.s.inertia_z, t),
                bound_clearance: a.s.bound_clearance,
                bound_angle: a.s.bound_angle,
                bound_bumper: a.s.bound_bumper,
                bound_widebody: a.s.bound_widebody,
                bound_bumper_width: a.s.bound_bumper_width,
                ..SimScalars::lerp(&a.s, &b.s, t)
            },
            drivetrain_type: lerp_i(a.drivetrain_type as i32, b.drivetrain_type as i32, t) as u32,
            wheel_front: WheelTune::lerp(&a.wheel_front, &b.wheel_front, t),
            wheel_back: WheelTune::lerp(&a.wheel_back, &b.wheel_back, t),
            // (the node count is the larger one; every word of both tables is blended)
            engine: EngineTune::with_words(
                EngineScalars::lerp(&a.engine.s, &b.engine.s, t),
                (a.engine.count as i32).max(b.engine.count as i32) as u32,
                std::array::from_fn(|k| std::array::from_fn(|i| lerp_i(a.engine.words[k][i], b.engine.words[k][i], t))),
            ),
            trans: TransTune {
                gears: lerp_i(a.trans.gears as i32, b.trans.gears as i32, t) as u32,
                max_downshifts: lerp_i(a.trans.max_downshifts as i32, b.trans.max_downshifts as i32, t) as u32,
                s: TransScalars::lerp(&a.trans.s, &b.trans.s, t),
            },
            powertrain: DrivetrainTune::lerp(&a.powertrain, &b.powertrain, t),
            freetrain: DrivetrainTune::lerp(&a.freetrain, &b.freetrain, t),
            axle_front: l2(a.axle_front, b.axle_front),
            axle_back: l2(a.axle_back, b.axle_back),
            // (the three damping vectors +16..+60 with the vector unit)
            aero: {
                let v = |f: fn(&AeroTune) -> f32| lerp_v(f(&a.aero), f(&b.aero), t);
                AeroTune {
                    ang_c_damp_x: v(|x| x.ang_c_damp_x),
                    ang_c_damp_y: v(|x| x.ang_c_damp_y),
                    ang_c_damp_z: v(|x| x.ang_c_damp_z),
                    ang_vel_damp_x: v(|x| x.ang_vel_damp_x),
                    ang_vel_damp_y: v(|x| x.ang_vel_damp_y),
                    ang_vel_damp_z: v(|x| x.ang_vel_damp_z),
                    ang_vel2_damp_x: v(|x| x.ang_vel2_damp_x),
                    ang_vel2_damp_y: v(|x| x.ang_vel2_damp_y),
                    ang_vel2_damp_z: v(|x| x.ang_vel2_damp_z),
                    ..AeroTune::lerp(&a.aero, &b.aero, t)
                }
            },
            fluid: FluidTune::lerp(&a.fluid, &b.fluid, t),
            // (+24 is an integer, rounded)
            tire_fric: TireFricTune {
                gear_threshold_bits: f32::from_bits(lerp_round(a.tire_fric.gear_threshold_bits.to_bits() as i32, b.tire_fric.gear_threshold_bits.to_bits() as i32, t) as u32),
                ..TireFricTune::lerp(&a.tire_fric, &b.tire_fric, t)
            },
            nitro: NitroTune::lerp(&a.nitro, &b.nitro, t),
            gyro: GyroTune::lerp(&a.gyro, &b.gyro, t),
            input: InputTune::lerp(&a.input, &b.input, t),
            cam_shake: a.cam_shake.clone(),
            damage: a.damage.clone(),
            ss_turbo: SsTurboTune::lerp(&a.ss_turbo, &b.ss_turbo, t),
            hydraulics: HydraulicsTune::lerp(&a.hydraulics, &b.hydraulics, t),
            // (a signed 16 bit word blended as an integer; its low half stored)
            nitro_shots: lerp_i(a.nitro_shots as u16 as i16 as i32, b.nitro_shots as u16 as i16 as i32, t) as u16 as f32,
            axle_bone_z: a.axle_bone_z,
            axle_bone: a.axle_bone,
            steer_axis: a.steer_axis,
        }
    }
}

/// Both sets of a car's `.r50`.
#[derive(Clone, Debug, Default)]
pub struct SimTunes {
    pub base: SimTune,
    pub mods: SimTune,
    /// mcCarModelTune's FenderEdgeFront / FenderEdgeRear (+88, +96): where the outer face of
    /// the tyres comes to, front and rear, each for the stock body and the wide one
    pub fender_edge: [[f32; 2]; 2],
    /// mcCarModelTune's FenderHeight, FenderWidth and FenderFloor (+56 / +64, +72 / +80, +104 / +112), front and rear, each
    /// for the stock body and the wide one: the garage's limits on a wheel (`fit_error`)
    pub fender_height: [[f32; 2]; 2],
    pub fender_width: [[f32; 2]; 2],
    pub fender_floor: [[f32; 2]; 2],
    /// mcCarModelTune's FenderCeilingFront / FenderCeilingRear (+40 / +48), each for the stock body and the wide one: how
    /// high a drawn wheel's top may stand (F-47822); 0 = no ceiling, as the ctor F-97670 leaves it
    pub fender_ceiling: [[f32; 2]; 2],
    /// mcCarModelTune's TireSteerLimit and TireSteerOffset (+136, +140; the ctor's 0, 0): the share of its turn a drawn
    /// steered wheel gives back, and the shift sideways with it (F-47822)
    pub tire_steer: [f32; 2],
    /// mcCarModelTune's cFrontBumperMovement, cRearBumperMovement (+184, +188: 0 NoMovement,
    /// 1 AllowLoose, 2 AllowBroken), cHoodMovement, cTrunkMovement (+192, +196: 0 NoMovement,
    /// 1 NormalMovement, 2 ReverseMovement) - sub_823404E0 @82340DB8; None without a vehModel
    pub part_movement: Option<[u32; 4]>,
    /// mcCarModelTune's fWindowClipPlaneHeight (+180): the side windows' clip plane, this high
    /// along the car's up axis from its origin (sub_82357DB8 -> ClipPlaneEq); None without a vehModel
    pub window_clip: Option<f32>,
    /// mcCarModelTune's bDoubleSidedTrunk (+200, a byte): the car's draw (F-92759) draws the base fragment's meshes on
    /// the `trunk_0` bone and the trunk_swap / trunk_wide slots without back-face culling; None without a vehModel
    pub double_sided_trunk: Option<bool>,
    /// mcCarModelTune's vDriverGaugeLightOffset / vDriverNeonLightOffset (+144, +160; the ctor F-97670's (-0.4, 1, -0.825) and
    /// 0): the gauge and neon lights of the driver's draw, in the car's frame (F-42959); None without a vehModel
    pub driver_lights: Option<[[f32; 3]; 2]>,
}

/// A wheel's radius and its ride's offset from the figures `d` of a set-up (ride, width, rim, profile, front then rear) for
/// `axle`: R = rim diameter x 0.5 + profile x width - 0.03175 in the original's fused single steps (F-82553, `fit`).
fn wheel_radius(d: &[u8; 8], axle: usize) -> (f32, f32) {
    const RIDE: [f32; 12] = [0.075, 0.05, 0.025, 0.0, -0.025, -0.05, -0.075, -0.1, -0.1125, -0.125, -0.1375, -0.15];
    let (ride, width, rim, profile) = (d[axle].min(11) as usize, d[2 + axle].min(16), d[4 + axle].clamp(12, 28), d[6 + axle].min(13));
    let w = (0.195f64 + 0.01 * width as f64) as f32;
    let p = (0.20f64 + 0.05 * profile as f64) as f32;
    let t = ((p as f64).mul_add(w as f64, -(f32::from_bits(0x3d02_0c4a) as f64))) as f32;
    let r = ((f32::from_bits(RIM_DIAMETER[rim as usize - 12]) as f64).mul_add(0.5, t as f64)) as f32;
    (r, RIDE[ride])
}

/// The rims' diameters (m) for 12..28 inches, as the executable holds them (0x827E9A50)
const RIM_DIAMETER: [u32; 17] = [
    0x3ea90ffa, 0x3eb61134, 0x3ec3126f, 0x3ed013a9, 0x3edd14e4, 0x3eea161f, 0x3ef71759, 0x3f020c4a, 0x3f088ce7, 0x3f0f0d84, 0x3f158e21,
    0x3f1c0ebe, 0x3f228f5c, 0x3f290ff9, 0x3f2f9096, 0x3f361134, 0x3f3c91d1,
];

/// What a car's set-up makes of its wheels (`SimTunes::fit`), front and rear.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelFit {
    pub radius: [f32; 2],
    pub width: [f32; 2],
    /// the wheels' sideways place in the car (0 = as the model has them)
    pub half_track: [f32; 2],
    pub offset: [f32; 2],
    /// the tyre's bead pull per axle, innerRadiusScale (F-43091 as the wheel draw F-11293 hands it to VS_ParallaxRubber2):
    /// (1 - sw / Rv) x 0.5 - 0.406, sw = profile x width - 0.0254, Rv = sw + rim diameter x 0.5 - the bead at the rim's radius
    pub bead: [f32; 2],
    /// the set-up's wide body (its byte +2203): which of mcCarModelTune's stock / wide pairs the car takes
    pub wide: bool,
}

impl SimTunes {
    /// The wheels a car set-up asks for. The original never drives on the tuning file's
    /// TireRadius / TireWidth / SuspensionOffset: when a car is made (0x82332440 ->
    /// 0x8236BC20, 0x82363210) they are written over from its `mcCarConfig` - the 8176-byte
    /// record of a `.xcc` file and of a garage slot in the save: +2106 ride height, +2108
    /// tyre width, +2110 rim size, +2112 tyre profile (a byte each, front then rear), +2203
    /// the wide body kit. Width = 0.195 + 0.01 i; radius = half of (rim + 1) inches + profile
    /// (0.20 + 0.05 i) x width - 0.03175; the wheels stand with their outer face at the
    /// model tuning's FenderEdge. Agrees with ten recorded wheels to 0.1 mm.
    pub fn fit(&mut self, cfg: &[u8]) -> Option<WheelFit> {
        const RIDE: [f32; 12] = [0.075, 0.05, 0.025, 0.0, -0.025, -0.05, -0.075, -0.1, -0.1125, -0.125, -0.1375, -0.15];
        if cfg.len() < 2204 {
            return None;
        }
        let wide = (cfg[2203] != 0) as usize;
        let mut fit = WheelFit { wide: wide != 0, ..Default::default() };
        for axle in 0..2 {
            let (ride, width, rim, profile) = (cfg[2106 + axle] as usize, cfg[2108 + axle] as usize, cfg[2110 + axle] as f32, cfg[2112 + axle] as usize);
            if ride > 11 || width > 16 || profile > 13 || !(12.0..=28.0).contains(&rim) {
                return None;
            }
            // (0x82392F08, to the bit: the three tables as the executable holds them - widths
            // and profiles the nearest single to 0.195 + 0.01 i and 0.20 + 0.05 i, the rims'
            // diameters at 0x827E9A50 - and two fused steps)
            const RIM: [u32; 17] = [
                0x3ea90ffa, 0x3eb61134, 0x3ec3126f, 0x3ed013a9, 0x3edd14e4, 0x3eea161f, 0x3ef71759, 0x3f020c4a, 0x3f088ce7,
                0x3f0f0d84, 0x3f158e21, 0x3f1c0ebe, 0x3f228f5c, 0x3f290ff9, 0x3f2f9096, 0x3f361134, 0x3f3c91d1,
            ];
            let w = (0.195f64 + 0.01 * width as f64) as f32;
            let p = (0.20f64 + 0.05 * profile as f64) as f32;
            let t = ((p as f64).mul_add(w as f64, -(f32::from_bits(0x3d02_0c4a) as f64))) as f32;
            fit.width[axle] = w;
            fit.radius[axle] = ((f32::from_bits(RIM[rim as usize - 12]) as f64).mul_add(0.5, t as f64)) as f32;
            fit.offset[axle] = RIDE[ride];
            // (F-43091: f13 = fmsubs(p, w, 0.0254), f12 = f13 + D x 0.5, f9 = 2 f13 / 2 f12, fmsubs(1 - f9, 0.5, 0.406))
            let sw = ((p as f64).mul_add(w as f64, -(0.0254f32 as f64))) as f32;
            let rv = sw + f32::from_bits(RIM[rim as usize - 12]) * 0.5;
            let f9 = (sw * 2.0) / (rv * 2.0);
            fit.bead[axle] = (((1.0 - f9) as f64).mul_add(0.5, -(0.406f32 as f64))) as f32;
            let edge = self.fender_edge[axle][wide];
            fit.half_track[axle] = if edge > 0.5 { edge - fit.width[axle] * 0.5 } else { 0.0 };
        }
        for set in [&mut self.base, &mut self.mods] {
            for (axle, wt) in [&mut set.wheel_front, &mut set.wheel_back].into_iter().enumerate() {
                wt.radius = fit.radius[axle];
                wt.width = fit.width[axle];
                wt.suspension_offset = fit.offset[axle];
            }
        }
        Some(fit)
    }

    /// The garage's wheel check (F-97327 -> F-82553) for one axle of a set-up's wheel figures `d` (+2106..+2114 of the
    /// record: ride, width, rim, profile, a byte each front then rear), `wide` the record's +2203: with R the wheel's
    /// radius as `fit` has it and h the ride's offset, 1 when R + h < FenderFloor (ride too low), 2 when R - h >
    /// FenderHeight (hitting the wheel well), 3 when R > FenderWidth (hitting the fender; FenderHeight when FenderWidth
    /// is not over 0), else 0.
    pub fn fit_error(&self, d: &[u8; 8], wide: bool, axle: usize) -> u8 {
        let (a, w) = (axle.min(1), wide as usize);
        let (r, h) = wheel_radius(d, a);
        let (floor, height, width) = (self.fender_floor[a][w], self.fender_height[a][w], self.fender_width[a][w]);
        let width = if width > 0.0 { width } else { height };
        if r + h < floor {
            1
        } else if r - h > height {
            2
        } else if r > width {
            3
        } else {
            0
        }
    }

    /// The garage setting one wheel figure (the DIMENSIONS rows, F-81554): `kind` 1 rim size, 2 tyre profile, 3 tyre
    /// width, 4 ride height set to `value` on `axle` (0 front, 1 rear, 2 both) of `d`, then the other figures moved until
    /// the wheel fits (F-40918); when nothing fits, the axle's four figures as they were. With both axles the front's
    /// figures go to the rear as well - not after a ride height that fitted, as in the original.
    pub fn set_dimension(&self, d: &mut [u8; 8], wide: bool, axle: usize, kind: u8, value: u8) {
        let a = if axle == 2 { 0 } else { axle };
        let old = [d[4 + a], d[2 + a], d[6 + a], d[a]];
        let at = match kind {
            1 => 4,
            2 => 6,
            3 => 2,
            _ => 0,
        };
        if axle == 2 {
            d[at] = value;
            d[at + 1] = value;
        } else {
            d[at + a] = value;
        }
        let fits = self.adjust_to_fit(d, wide, axle, kind);
        if !fits {
            [d[4 + a], d[2 + a], d[6 + a], d[a]] = old;
        }
        if axle == 2 && !(fits && kind == 4) {
            for k in [4, 0, 6, 2] {
                d[k + 1] = d[k];
            }
        }
    }

    /// F-40918: after figure `kind` changed, the others moved one step at a time in the order the original tries them
    /// (`step_figure`), each put back when it found no fit; false when none did.
    fn adjust_to_fit(&self, d: &mut [u8; 8], wide: bool, axle: usize, kind: u8) -> bool {
        let a = if axle == 2 { 0 } else { axle };
        let (rim0, prof0, ride0) = (d[4 + a], d[6 + a], d[a]);
        // (the figure's place and range: rim 12..28, profile 0..13, ride 0..11)
        const RIM: (usize, u8, u8) = (4, 12, 28);
        const PROF: (usize, u8, u8) = (6, 0, 13);
        const RIDE: (usize, u8, u8) = (0, 0, 11);
        let restore = |d: &mut [u8; 8], f: (usize, u8, u8)| d[f.0 + a] = match f.0 { 4 => rim0, 6 => prof0, _ => ride0 };
        // (the tries for each error, in order)
        let tries: &[((usize, u8, u8), i32)] = match (kind, self.fit_error(d, wide, a)) {
            (_, 0) => return true,
            (1, 1) => &[(PROF, 1), (RIDE, 1)],
            (1, 2) => &[(PROF, -1), (RIDE, 1)],
            (1, 3) => &[(PROF, -1)],
            (2, 1) => &[(RIDE, 1), (RIM, 1)],
            (2, 2) => &[(RIDE, 1), (RIM, -1)],
            (2, 3) => &[(RIM, -1)],
            (3, 1) => &[(PROF, 1), (RIDE, 1), (RIM, 1)],
            (3, 2) => &[(PROF, -1), (RIDE, 1), (RIM, -1)],
            (3, 3) => &[(PROF, -1), (RIM, -1), (PROF, -1)],
            (4, 1) | (4, 2) => &[(PROF, -1), (RIM, -1)],
            (4, 3) => return false,
            _ => return true,
        };
        for &(f, step) in tries {
            if self.step_figure(d, wide, a, f, step) {
                return true;
            }
            restore(d, f);
        }
        false
    }

    /// F-91663 / F-39778 / F-83143 (rim, profile, ride): figure `f` (place, lowest, highest) of axle `a` moved by `step`
    /// within its range until the wheel fits - true - or it stands at the end it moves to - false, the figure left there.
    fn step_figure(&self, d: &mut [u8; 8], wide: bool, a: usize, f: (usize, u8, u8), step: i32) -> bool {
        let (at, lo, hi) = f;
        loop {
            let v = d[at + a];
            if (step > 0 && v >= hi) || (step <= 0 && v <= lo) {
                return false;
            }
            d[at + a] = (v as i32 + step).clamp(lo as i32, hi as i32) as u8;
            if self.fit_error(d, wide, a) == 0 {
                return true;
            }
        }
    }

    pub fn parse(res: &Resource) -> Option<SimTunes> {
        let rd = Rd::new(res);
        let mut out = SimTunes::default();
        let (mut have_sim, mut have_gyro) = (false, false);
        let mut o = 0x50;
        while let Some(c) = rd.ptr(o) {
            o += 4;
            if o > 0x200 {
                break;
            }
            let (Some(np), Some(tp)) = (rd.ptr(c + 4), rd.ptr(c + 8)) else { continue };
            let (nm, ty) = (rd.cstr(np), rd.cstr(tp));
            if ty == "vehModel" {
                out.fender_edge = [[rd.f32(c + 88), rd.f32(c + 92)], [rd.f32(c + 96), rd.f32(c + 100)]];
                let pair = |o: usize| [[rd.f32(c + o), rd.f32(c + o + 4)], [rd.f32(c + o + 8), rd.f32(c + o + 12)]];
                (out.fender_height, out.fender_width, out.fender_floor) = (pair(56), pair(72), pair(104));
                out.fender_ceiling = pair(40);
                out.tire_steer = [rd.f32(c + 136), rd.f32(c + 140)];
                out.part_movement = Some([rd.u32(c + 184), rd.u32(c + 188), rd.u32(c + 192), rd.u32(c + 196)]);
                out.window_clip = Some(rd.f32(c + 180));
                out.double_sided_trunk = Some(rd.u8(c + 200) != 0);
                let v3 = |o: usize| [rd.f32(c + o), rd.f32(c + o + 4), rd.f32(c + o + 8)];
                out.driver_lights = Some([v3(144), v3(160)]);
                continue;
            }
            let set = if nm.ends_with("_mods") { &mut out.mods } else { &mut out.base };
            match ty.as_str() {
                "vehSim" => {
                    set.read_sim(&rd, c, &|off| rd.ptr(c + off));
                    if !nm.ends_with("_mods") {
                        have_sim = true;
                    }
                }
                "vehGyro" => {
                    set.gyro = GyroTune::read(&rd, c);
                    have_gyro = true;
                }
                "vehInput" => set.input = InputTune::read(&rd, c),
                "vehDamage" => {
                    out.base.damage = DamageTune::read(&rd, c);
                    out.mods.damage = out.base.damage.clone();
                }
                "vehCamShake" => {
                    out.base.cam_shake = CamShakeTune::read(&rd, c);
                    out.mods.cam_shake = out.base.cam_shake.clone();
                }
                _ => {}
            }
        }
        let b = &out.base;
        let sane = have_sim && have_gyro && b.s.mass > 50.0 && b.wheel_front.radius > 0.05 && b.wheel_front.static_fric > 0.1 && b.engine.s.max_rpm > 500.0 && b.trans.gears >= 3;
        if !sane {
            return None;
        }
        // (a car without a "mods" set keeps its base figures)
        if out.mods.s.mass <= 50.0 || out.mods.wheel_front.radius <= 0.05 {
            out.mods = out.base.clone();
        }
        Some(out)
    }
}

impl SimTune {
    /// mcCarSimTune at `c` and the sub-tunes its pointers lead to (`sub(off)`: the object the pointer at `c + off` names).
    fn read_sim(&mut self, rd: &Rd, c: usize, sub: &dyn Fn(usize) -> Option<usize>) {
        let set = self;
        set.s = SimScalars::read(rd, c);
        set.drivetrain_type = rd.u32(c + 132);
        if let Some(p) = sub(136) {
            set.wheel_front = WheelTune::read(rd, p);
        }
        if let Some(p) = sub(140) {
            set.wheel_back = WheelTune::read(rd, p);
        }
        if let Some(p) = sub(144) {
            let table = |at: usize| std::array::from_fn(|i| rd.u32(at + i * 4) as i32);
            set.engine = EngineTune::with_words(EngineScalars::read(rd, p), rd.u32(p + 52), [table(p + 56), table(p + 100)]);
        }
        if let Some(p) = sub(148) {
            set.trans = TransTune { gears: rd.u32(p), max_downshifts: rd.u32(p + 32), s: TransScalars::read(rd, p) };
        }
        if let Some(p) = sub(152) {
            set.powertrain = DrivetrainTune::read(rd, p);
        }
        if let Some(p) = sub(156) {
            set.freetrain = DrivetrainTune::read(rd, p);
        }
        if let Some(p) = sub(160) {
            set.axle_front = [rd.f32(p), rd.f32(p + 4)];
        }
        if let Some(p) = sub(164) {
            set.axle_back = [rd.f32(p), rd.f32(p + 4)];
        }
        if let Some(p) = sub(168) {
            set.aero = AeroTune::read(rd, p);
        }
        if let Some(p) = sub(172) {
            set.fluid = FluidTune::read(rd, p);
        }
        if let Some(p) = sub(192) {
            set.tire_fric = TireFricTune::read(rd, p);
        }
        if let Some(p) = sub(196) {
            set.nitro = NitroTune::read(rd, p);
            set.nitro_shots = rd.u16(p) as f32;
        }
        if let Some(p) = sub(200) {
            set.ss_turbo = SsTurboTune::read(rd, p);
        }
        if let Some(p) = sub(208) {
            set.hydraulics = HydraulicsTune::read(rd, p);
        }
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    /// The garage's check on the R8 (its .r50's fenders: height 0.38 / 0.39, width 0.35, floor 0.32): its own front wheel,
    /// R 0.2953, is under the floor already, so a lower ride finds nothing that fits and is refused, a higher one fits, and
    /// a 19 inch rim takes the next tyre profile up (R 0.32025) - F-81554 / F-40918 / F-82553 step by step.
    #[test]
    fn the_garage_keeps_the_wheels_in_the_fenders() {
        let t = SimTunes { fender_height: [[0.38, 0.38], [0.39, 0.39]], fender_width: [[0.35, 0.35], [0.35, 0.35]], fender_floor: [[0.32, 0.32], [0.32, 0.32]], ..Default::default() };
        let stock = [3u8, 3, 5, 11, 18, 18, 3, 2];
        assert_eq!(t.fit_error(&stock, false, 0), 1);
        let mut d = stock;
        t.set_dimension(&mut d, false, 0, 4, 4);
        assert_eq!(d, stock, "lowered: refused");
        t.set_dimension(&mut d, false, 0, 4, 2);
        assert_eq!(d[0], 2, "raised: kept");
        let mut d = stock;
        t.set_dimension(&mut d, false, 2, 1, 19);
        assert_eq!((d[4], d[5], d[6], d[7]), (19, 19, 4, 4), "both axles: the front's figures to the rear");
        // (a 23 inch rim, R 0.3588: under the wheel well, over the fender - error 3)
        assert_eq!(t.fit_error(&[3, 3, 5, 11, 23, 18, 3, 2], false, 0), 3);
    }

    /// The Audi R8's own set-up (18 inch rims, tyres 0.245 x 0.35 front and 0.305 x 0.30 rear)
    /// and its fenders (0.91 / 0.93): the wheels recorded in the running game.
    #[test]
    fn a_set_up_makes_the_wheels_the_running_game_has() {
        let mut tunes = SimTunes { fender_edge: [[0.91, 0.95], [0.93, 0.97]], ..Default::default() };
        let mut cfg = vec![0u8; 8176];
        cfg[2106..2114].copy_from_slice(&[3, 3, 5, 11, 18, 18, 3, 2]);
        let fit = tunes.fit(&cfg).expect("a set-up");
        let near = |a: f32, b: f32| (a - b).abs() < 5e-5;
        assert!(near(fit.radius[0], 0.2953) && near(fit.radius[1], 0.30105), "{fit:?}");
        assert!(near(fit.width[0], 0.245) && near(fit.width[1], 0.305), "{fit:?}");
        assert!(near(fit.half_track[0], 0.7875) && near(fit.half_track[1], 0.7775), "{fit:?}");
        assert_eq!(fit.offset, [0.0, 0.0]);
        assert!(near(tunes.base.wheel_back.radius, 0.30105) && near(tunes.mods.wheel_front.width, 0.245));
        // (lowered as far as the garage goes at the back, the wide body fitted)
        cfg[2107] = 11;
        cfg[2203] = 1;
        let fit = tunes.fit(&cfg).unwrap();
        assert!(near(fit.offset[1], -0.15) && near(fit.half_track[0], 0.95 - 0.1225), "{fit:?}");
        // (and nothing is touched by a record that is not one)
        cfg[2110] = 40;
        assert_eq!(tunes.fit(&cfg), None);
    }

    /// The tyre's bead pull (innerRadiusScale) of the 240SX's own set-up (15 inch rims, tyres 0.195 x 0.60): the
    /// running original hands VS_ParallaxRubber2 c209 = -0.061359555 (0xbd7b5428) on all four tyres
    /// (`work/police17/cap/s240_noon.gpu.bin`, draws 2169 - 2172).
    #[test]
    fn the_bead_pull_is_the_recorded_one() {
        let mut tunes = SimTunes::default();
        let mut cfg = vec![0u8; 8176];
        cfg[2106..2114].copy_from_slice(&[3, 3, 0, 0, 15, 15, 8, 8]);
        let fit = tunes.fit(&cfg).expect("a set-up");
        assert_eq!(fit.bead.map(f32::to_bits), [0xbd7b_5428; 2], "{fit:?}");
    }
}


#[cfg(test)]
mod movement_tests {
    /// (diagnostic: MCLA_MOVEMENT_DIR=<extracted vehicle dir> lists every car's part movement
    /// settings, mcCarModelTune +184..+196)
    #[test]
    fn part_movement_settings_are_enums() {
        let Ok(dir) = std::env::var("MCLA_MOVEMENT_DIR") else { return };
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            for f in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
                if f.path().extension().map_or(true, |x| x != "r50") {
                    continue;
                }
                let Ok(d) = std::fs::read(f.path()) else { continue };
                // (the cache's own wrapping, as the game's load_resource_file reads it)
                let res = if d.len() > 16 && &d[0..4] == b"MCLR" {
                    let w = |k: usize| u32::from_le_bytes(d[k..k + 4].try_into().unwrap());
                    let data = d[16..].to_vec();
                    let vs = (w(12) as usize).min(data.len());
                    crate::rsc::Resource { rtype: w(4), flags: w(8), virtual_size: vs, physical_size: data.len() - vs, data }
                } else {
                    let Ok(r) = crate::rsc::Resource::parse(&d) else { continue };
                    r
                };
                if let Some(t) = super::SimTunes::parse(&res) {
                    let m = t.part_movement;
                    println!("{} {:?} window clip {:?} driver lights {:?} damage max {} med {} (base) {} (mods)", e.file_name().to_string_lossy(), m, t.window_clip, t.driver_lights, t.base.damage.max_damage, t.base.damage.med_damage, t.mods.damage.max_damage);
                    if let Some(m) = m {
                        assert!(m.iter().all(|v| *v <= 2), "{m:?}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod blend_tests {
    use super::*;

    /// A base and a mods set blended by the upgrade share as the original's blend F-94542 does it, run offline on its own
    /// code (`work/physics2/d34/blend_cases.py`: both sets of the live tune filled with random words, t = 0, 1, 0.5 and
    /// random): every field the port reads, bit for bit.
    #[test]
    fn the_blend_is_the_originals() {
        let words = |l: &str| l.split_whitespace().skip(1).map(|x| u32::from_str_radix(x, 16).unwrap()).collect::<Vec<u32>>();
        let set = |w: &[u32], base: u32| {
            let data: Vec<u8> = w.iter().flat_map(|v| v.to_be_bytes()).collect();
            let len = data.len() as u32;
            let res = Resource { rtype: 0, flags: 0, virtual_size: data.len(), physical_size: 0, data };
            let rd = Rd::new(&res);
            let mut s = SimTune::default();
            s.read_sim(&rd, 0, &|off| {
                let v = rd.u32(off);
                (v >= base && v < base + len).then(|| (v - base) as usize)
            });
            s
        };
        let lines: Vec<&str> = include_str!("simtune_blend_cases.txt").lines().collect();
        let (mut cases, mut bad) = (0, Vec::new());
        for c in lines.chunks(4) {
            let h = words(c[0]);
            let t = f32::from_bits(h[0]);
            let (a, b, mut d) = (set(&words(c[1]), h[2]), set(&words(c[2]), h[3]), set(&words(c[3]), h[1]));
            let port = SimTune::lerp(&a, &b, t);
            // (the wheel lerp leaves radius, width and suspension offset: the car's set-up writes them, SimTunes::fit)
            for (w, p) in [(&mut d.wheel_front, &port.wheel_front), (&mut d.wheel_back, &port.wheel_back)] {
                (w.radius, w.width, w.suspension_offset) = (p.radius, p.width, p.suspension_offset);
            }
            let (port, orig) = (format!("{port:?}"), format!("{d:?}"));
            cases += 1;
            if port != orig {
                let at = port.split(", ").zip(orig.split(", ")).find(|(p, o)| p != o);
                bad.push(format!("t {t}: {at:?}"));
            }
        }
        assert!(cases >= 12 && bad.is_empty(), "{} of {cases} differ:\n{}", bad.len(), bad.join("\n"));
    }

    /// The gyro and input tunes the same way, by their own blends F-22878 (with F-56676) and F-41440.
    #[test]
    fn the_gyro_and_input_blends_are_the_originals() {
        fn check<T: std::fmt::Debug>(cases: &str, read: fn(&Rd, usize) -> T, lerp: fn(&T, &T, f32) -> T) {
            let words = |l: &str| l.split_whitespace().skip(1).map(|x| u32::from_str_radix(x, 16).unwrap()).collect::<Vec<u32>>();
            let set = |l: &str| {
                let data: Vec<u8> = words(l).iter().flat_map(|v| v.to_be_bytes()).collect();
                let res = Resource { rtype: 0, flags: 0, virtual_size: data.len(), physical_size: 0, data };
                read(&Rd::new(&res), 0)
            };
            let lines: Vec<&str> = cases.lines().collect();
            assert!(lines.len() >= 64);
            for c in lines.chunks(4) {
                let t = f32::from_bits(words(c[0])[0]);
                let (port, orig) = (format!("{:?}", lerp(&set(c[1]), &set(c[2]), t)), format!("{:?}", set(c[3])));
                assert_eq!(port, orig, "t {t}");
            }
        }
        check(include_str!("simtune_gyro_blend_cases.txt"), GyroTune::read, GyroTune::lerp);
        check(include_str!("simtune_input_blend_cases.txt"), InputTune::read, InputTune::lerp);
    }
}
