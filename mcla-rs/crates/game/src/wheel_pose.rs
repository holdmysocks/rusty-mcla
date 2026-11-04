//! A set-up car's drawn wheel pose (F-47822, the visual wheel update): the wheel's matrix among the car's parts, kept
//! under the fender, leaned, and its turn cut by mcCarModelTune's TireSteerLimit with the shift TireSteerOffset.
//! Each step in the original's own operations (fused where it fuses, single rounding where it rounds).
use crate::physics::exact::{axle_disp, fma, guest_atan2, guest_sin, leaned};
use glam::Vec3;

/// What the pose reads of the car: is it a motorcycle (F-54299) and the front wheel's turn it then adds (+1164 of
/// the car's +84 object), the fender ceilings of the set-up's body (FenderCeilingFront / Rear, the pair's stock or
/// wide-body value by the set-up's byte +2203), TireSteerLimit and TireSteerOffset.
#[derive(Clone, Copy, Debug, Default)]
pub struct CarTerms {
    pub bike: bool,
    pub bike_turn: f32,
    pub ceiling: [f32; 2],
    pub steer_limit: f32,
    pub steer_offset: f32,
}

/// What it reads of the wheel: its place of rest (+384), steering axis (+400), radius and width (+420, +424), the
/// vehWheelTune's SteeringLimit and CamberLimit, the lean (+516), and what the axle's displacement (F-48387) needs:
/// compression (+496), TravelExtent / TravelLimit and the offset (+432).
#[derive(Clone, Copy, Debug, Default)]
pub struct WheelTerms {
    pub rest: Vec3,
    pub axis: Vec3,
    pub radius: f32,
    pub width: f32,
    pub steering_limit: f32,
    pub camber_limit: f32,
    pub lean: f32,
    pub comp: f32,
    pub travel_extent: f32,
    pub travel_limit: f32,
    pub offset: f32,
}

/// fsel's sign of x: 1 above 0, 0 at 0, -1 below
fn side(x: f32) -> f32 {
    let a = if -x >= 0.0 { 0.0 } else { 1.0 };
    if x >= 0.0 { a } else { -1.0 }
}

/// vmsum3fp128: the three products summed as the vector unit does, (x + y) + (z + 0)
fn dot3(a: Vec3, b: Vec3) -> f32 {
    (a.x * b.x + a.y * b.y) + (a.z * b.z + 0.0)
}

/// The wheel's matrix (rows axle, up, way; place) among the car's parts: the wheel's world matrix against the body's
/// (the original's products of the two, F-47822's start).
pub fn local(body: &[Vec3; 4], wheel: &[Vec3; 4]) -> [Vec3; 4] {
    let d = wheel[3] - body[3];
    let f = |v: Vec3| Vec3::new(dot3(v, body[0]), dot3(v, body[1]), dot3(v, body[2]));
    [f(wheel[0]), f(wheel[1]), f(wheel[2]), f(d)]
}

/// F-47822 on one wheel's matrix among the car's parts: what the wheel is drawn with before its roll (and the half
/// turn of the right-hand wheels). `front_wheel` = the bike's wheel 0.
pub fn pose(m: [Vec3; 4], car: &CarTerms, w: &WheelTerms, front_wheel: bool) -> [Vec3; 4] {
    let [mut r0, mut r1, mut r2, mut p] = m;
    // (under the fender: the wheel's top, its way along the steering axis from rest plus its radius, no higher than
    // the ceiling of its end - front where the wheel stands ahead of the middle, z below 0)
    let ceiling = car.ceiling[(p.z >= 0.0) as usize];
    if ceiling > 0.0 {
        let d = p - w.rest;
        let h = fma(d.x, w.axis.x, fma(w.axis.y, d.y, w.axis.z * d.z)) + w.radius;
        if h > ceiling {
            let e = h - ceiling;
            if e > 0.0 {
                p = w.axis * -e + p;
            }
        }
    }
    if w.camber_limit < 0.0 {
        // (a negative CamberLimit moves the wheel out along its axle as the axle is pressed in)
        let d = axle_disp(w.comp, w.travel_extent, w.travel_limit, w.offset);
        if d > 0.0 {
            p = r0 * (side(p.x) * w.camber_limit * d) + p;
        }
    } else if w.lean != 0.0 {
        // (the lean about the wheel's way, the place moved to keep the tyre's foot)
        [r0, r1, r2] = leaned(&[r0, r1, r2], r2, w.lean);
        let s = guest_sin(w.lean as f64) as f32;
        p = r0 * (w.radius * s * -3.5) + p;
        p = r1 * -(s.abs() * w.width) + p;
    }
    let turn = guest_atan2(r2.x as f64, r2.z as f64) as f32;
    if car.bike {
        if front_wheel {
            [r0, r1, r2] = leaned(&[r0, r1, r2], r1, -car.bike_turn - turn);
        }
    } else {
        // (the cut on the steered wheels only - SteeringLimit above 0.1; the shift on every wheel, by its own turn)
        if w.steering_limit > f32::from_bits(0x3dcccccd) && car.steer_limit > 0.0 {
            [r0, r1, r2] = leaned(&[r0, r1, r2], r1, -(car.steer_limit * turn));
        }
        if car.steer_offset > 0.0 {
            let a = turn.abs() * (1.0 - car.steer_limit);
            let s = guest_sin(a as f64) as f32;
            let k = side(p.x) * (w.radius * s * a);
            p.x = -fma(k, car.steer_offset, -p.x);
        }
    }
    [r0, r1, r2, p]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drawn_turn_is_the_sims_cut_by_the_tyre_steer() {
        // (a front wheel as the sim builds it, F-57353: turned by `angle` about up, carried through a yawed body; the old
        // draw turned it by rot_y(-steer) with steer = -angle - the pose gives that back at TSL 0, and (1 - TSL) of it)
        use crate::physics::exact::{row_through, wheel_rebuilt_about};
        let yaw = glam::Quat::from_rotation_y(0.7);
        let body = [yaw * Vec3::X, yaw * Vec3::Y, yaw * Vec3::Z, Vec3::new(100.0, 2.0, -50.0)];
        let _ = row_through;
        for angle in [-0.5f32, -0.1, 0.0, 0.2, 0.45] {
            let world = wheel_rebuilt_about(&body, Vec3::new(-0.78, 0.2, -1.4), 0.03, 0.0, true, angle, Vec3::Y);
            let w = WheelTerms { axis: Vec3::Y, radius: 0.33, width: 0.25, steering_limit: 0.6, ..Default::default() };
            for (tsl, share) in [(0.0f32, 1.0f32), (0.4, 0.6)] {
                let car = CarTerms { steer_limit: tsl, ..Default::default() };
                let p = pose(local(&body, &world), &car, &w, false);
                let q = glam::Quat::from_mat3(&glam::Mat3::from_cols(p[0], p[1], p[2]));
                let want = glam::Quat::from_rotation_y(angle * share);
                assert!(q.dot(want).abs() > 0.99999, "angle {angle} tsl {tsl}: {q:?} against {want:?}");
            }
        }
    }

    #[test]
    fn drawn_wheel_matches_the_original() {
        // (tools/drawn_wheel_reference.py: F-47822 run whole, offline, its roll and half turn stubbed; one car a line,
        // the car's terms and per wheel 22 words, then per wheel the pose's rows and place)
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let (mut cars, mut bad) = (0, 0);
        for line in include_str!("testdata/drawn_wheel_ref.txt").lines() {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let i: Vec<&str> = inp.split(' ').collect();
            let o: Vec<f32> = out.split(' ').map(f).collect();
            let v = |k: usize| Vec3::new(f(i[k]), f(i[k + 1]), f(i[k + 2]));
            let wide = i[1] == "1";
            let car = CarTerms { bike: i[0] == "1", bike_turn: f(i[2]), ceiling: [f(i[15]), f(i[16])], steer_limit: f(i[17]), steer_offset: f(i[18]) };
            let _ = wide;
            let body = [v(3), v(6), v(9), v(12)];
            for k in 0..4 {
                let b = 19 + 22 * k;
                let wheel = [v(b), v(b + 3), v(b + 6), v(b + 9)];
                let w = WheelTerms { rest: v(b + 12), axis: Vec3::Y, radius: f(i[b + 15]), width: f(i[b + 16]), steering_limit: f(i[b + 17]), camber_limit: f(i[b + 18]), lean: f(i[b + 19]), comp: f(i[b + 20]), travel_extent: 0.25, travel_limit: 0.25, offset: f(i[b + 21]) };
                let got = pose(local(&body, &wheel), &car, &w, k == 0);
                let want = &o[12 * k..12 * k + 12];
                let same = (0..4).all(|r| (0..3).all(|c| got[r][c].to_bits() == want[3 * r + c].to_bits()));
                if !same {
                    if bad < 5 {
                        eprintln!("car {cars} wheel {k}: got {got:?} want {want:?}");
                    }
                    bad += 1;
                }
            }
            cars += 1;
        }
        assert!(cars >= 600);
        assert_eq!(bad, 0, "{bad} of {} wheels differ", cars * 4);
    }
}
