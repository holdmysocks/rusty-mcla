//! The chassis update F-97237 (mcCarSim's update after the vehicle's), its own pieces as one function: decoded
//! 2026-11-12 from the generated code and checked against it offline (tools/chassis_update_reference.py, 6,000 chassis
//! states; docs/reference/vehicle/chassis_update_check.md). Its calls after the pieces - the drivetrain's F-79815, the
//! nitro F-84636, the SST F-60935, the hydraulics and air ride blocks, the end F-23458 - are their own rows.

use glam::Vec4;

/// What F-97237 reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct In {
    /// chassis +592 (0x10 wiped out, 0x04 damaged out)
    pub flags: u32,
    /// mcCarFluid's bit (F-12574)
    pub in_water: bool,
    /// the player options' byte +797 and chassis +576
    pub option: bool,
    pub f576: u16,
    /// tune +32 (ModelOffset) and +48 (CenterOfMass), four lanes each; +268 UpsetCG
    pub model_offset: Vec4,
    pub com: Vec4,
    pub upset_cg: f32,
    /// the damage object's wipeout timer (+44) and its WipeoutDelay (tune +56)
    pub wipe_timer: f32,
    pub wipe_delay: f32,
    /// chassis +112 (the centre of mass)
    pub cg: Vec4,
    pub dt: f32,
    /// slot 11 (wheels on the ground) and +24 (wheels)
    pub ground: u32,
    pub wheels: u32,
    /// mcCarStuck's +56 (None: no stuck object)
    pub stuck: Option<u32>,
    /// chassis +584 (air time), +640 (the place of the take-off), the car's position
    pub air: f32,
    pub takeoff: Vec4,
    pub position: Vec4,
    /// chassis +136 / +140 (throttle, brake), +224 (speed), the body's angular velocity (+288)
    pub throttle: f32,
    pub brake: f32,
    pub speed: f32,
    pub spin: [f32; 3],
    /// the hydraulics' wheel buttons (+48..51) and the air ride's (+120..123)
    pub hyd: [u8; 4],
    pub air_ride: [u8; 4],
    /// the gyro's halfwords +16 / +18 (None: no gyro)
    pub gyro: Option<(u16, u16)>,
    /// tune +300 LandingDampFactor, +304 AeroDampFactor, +312 AeroDampDelay
    pub landing: f32,
    pub aero_factor: f32,
    pub aero_delay: f32,
    /// chassis +260 (up.y), the aero object's +4 (the damping factor)
    pub up_y: f32,
    pub aero: f32,
    /// chassis +578 (signed byte), +580, and [+16] +8 / +72
    pub f578: i8,
    pub f580: f32,
    pub s16_8: f32,
    pub s16_72: f32,
    /// the transmission's +28
    pub trans28: f32,
}

/// What F-97237 writes (`landed`: the player's jump statistics would be counted - not ported).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Out {
    pub f576: u16,
    pub cg: Vec4,
    pub air: f32,
    pub still: f32,
    pub takeoff: Vec4,
    pub aero: f32,
    pub trans28: f32,
    pub s16_8: f32,
    pub landed: bool,
}

/// x (lane-wise mul then add, the generated code's vmaddfp)
fn madd(a: Vec4, b: f32, c: Vec4) -> Vec4 {
    a * Vec4::splat(b) + c
}

/// The body's turning speed as the still timer measures it: sqrt(fma(z, z, fma(x, x, y y))).
pub fn spin_len(w: [f32; 3]) -> f32 {
    w[2].mul_add(w[2], w[0].mul_add(w[0], w[1] * w[1])).sqrt()
}

/// F-97237's pieces; `still` the chassis' +588 as it comes in.
pub fn update(i: &In, still: f32) -> Out {
    // (+576 bit 2: the player options' byte)
    let f576 = if i.option { i.f576 | 2 } else { i.f576 & !2 };
    // (the centre of mass: wiped out, damaged out, else its height back to the tune's at 0.5 m/s)
    let mut cg = i.cg;
    let span = i.model_offset - i.com;
    if i.flags & 0x10 != 0 && i.upset_cg > 0.0 && !i.in_water {
        let q = i.wipe_timer / i.wipe_delay;
        let q = if q >= 0.0 { q } else { 0.0 };
        let q = if q - 1.0 >= 0.0 { 1.0 } else { q };
        cg = madd(span, q * i.upset_cg, i.com);
    } else if i.flags & 0x04 != 0 && i.upset_cg > 0.0 && !i.in_water {
        cg = madd(span, i.upset_cg, i.com);
    } else {
        let target = i.com.y;
        let mut y = cg.y;
        if y < target {
            y = i.dt.mul_add(0.5, y);
            if y > target {
                y = target;
            }
        } else if y > target {
            y = -(i.dt.mul_add(0.5, -y));
            if y < target {
                y = target;
            }
        }
        cg.y = y;
    }
    // (the air time: counted while no wheel is down and mcCarStuck rests; ending, LandingDampFactor after a quarter
    // second and the time negated for one tick; else 0 and the take-off's place is the car's)
    let mut air = i.air;
    let mut aero = i.aero;
    let mut takeoff = i.takeoff;
    let mut landed = false;
    if i.ground == 0 && i.stuck.is_none_or(|s| s == 0) {
        air += i.dt;
    } else if air > 0.0 {
        landed = true;
        if air > 0.25 {
            aero = i.landing;
        }
        air = -air;
    } else {
        air = 0.0;
        takeoff = i.position;
    }
    // (the still timer)
    let spin = spin_len(i.spin);
    let held = i.hyd.iter().any(|&b| b != 0) || i.air_ride.iter().any(|&b| b != 0);
    let still = if i.throttle == 0.0 && i.brake == 0.0 && i.ground == i.wheels && i.speed <= 0.05 && spin <= 0.5 && !held { still + i.dt } else { 0.0 };
    // (slot 13 below 0 - the landing tick: no change of gear for GearChangeDelay)
    let trans28 = if air < 0.0 { 0.0 } else { i.trans28 };
    // (the aero factor: on four wheels with a gyro, out of the water, long in the air or upside down; else towards 1)
    if !i.in_water && i.gyro == Some((0, 0)) && (air > i.aero_delay || i.up_y < 0.0) {
        aero = i.aero_factor;
    } else if aero != 1.0 {
        if aero < 1.0 {
            aero += i.dt;
            if aero > 1.0 {
                aero = 1.0;
            }
        } else {
            aero -= i.dt;
            if aero < 1.0 {
                aero = 1.0;
            }
        }
    }
    // (+578 > 0: [+16] +8 at least +580 x [+16] +72)
    let mut s16_8 = i.s16_8;
    if i.f578 > 0 {
        let v = i.f580 * i.s16_72;
        if s16_8 < v {
            s16_8 = v;
        }
    }
    Out { f576, cg, air, still, takeoff, aero, trans28, s16_8, landed }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-97237 against the original run offline (tools/chassis_update_reference.py).
    #[test]
    fn chassis_update_matches_the_original() {
        let text = include_str!("testdata/chassis_update_ref.txt");
        let mut n = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = inp.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let o: Vec<&str> = out.split_whitespace().collect();
            let mut i = In::default();
            i.flags = w[0];
            i.in_water = w[1] != 0;
            i.option = w[2] != 0;
            i.f576 = w[3] as u16;
            let r = &w[4..];
            let v4 = |s: &[u32]| Vec4::new(f32::from_bits(s[0]), f32::from_bits(s[1]), f32::from_bits(s[2]), f32::from_bits(s[3]));
            i.model_offset = v4(&r[0..4]);
            i.com = v4(&r[4..8]);
            let g = |j: usize| f32::from_bits(r[j]);
            i.upset_cg = g(8);
            i.wipe_timer = g(9);
            i.wipe_delay = g(10);
            i.cg = v4(&r[11..15]);
            i.dt = g(15);
            i.ground = r[16];
            i.wheels = r[17];
            i.stuck = (r[18] != 0).then_some(r[19]);
            i.air = g(20);
            i.takeoff = v4(&r[21..25]);
            i.position = Vec4::new(g(25), g(26), g(27), 0.0);
            i.throttle = g(28);
            i.brake = g(29);
            i.speed = g(30);
            i.spin = [g(31), g(32), g(33)];
            i.hyd = [r[34] as u8, r[35] as u8, r[36] as u8, r[37] as u8];
            i.air_ride = [r[38] as u8, r[39] as u8, r[40] as u8, r[41] as u8];
            // (r[42]: the air ride's +16, r[43]: slot 12 - neither read by these pieces)
            i.gyro = (r[44] != 0).then_some((r[45] as u16, r[46] as u16));
            i.landing = g(47);
            i.aero_factor = g(48);
            i.aero_delay = g(49);
            i.up_y = g(50);
            i.aero = g(51);
            i.f578 = r[52] as u8 as i8;
            i.f580 = g(53);
            i.s16_8 = g(54);
            i.s16_72 = g(55);
            i.trans28 = g(56);
            let x = update(&i, 0.0);
            let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
            let got = [x.f576 as u32, x.cg.x.to_bits(), x.cg.y.to_bits(), x.cg.z.to_bits(), x.cg.w.to_bits(), x.air.to_bits(), x.still.to_bits(), x.takeoff.x.to_bits(), x.takeoff.y.to_bits(), x.takeoff.z.to_bits(), x.takeoff.w.to_bits(), x.aero.to_bits(), x.trans28.to_bits(), x.s16_8.to_bits()];
            let want: Vec<u32> = o[..14].iter().map(|s| h(s)).collect();
            assert_eq!(got.to_vec(), want, "case {n}: {inp}");
            assert_eq!(x.landed, o[14].starts_with('s'), "case {n}: the statistics' gate");
            n += 1;
        }
        assert_eq!(n, 6000);
    }
}
