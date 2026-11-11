//! The drawn wheels' roll of a traffic car (F-37033's wheel loop) against the original's code run offline
//! (tools/wheel_roll_reference.py): 2,737 render updates of cars with 0 - 6 wheels, hubs of every height, velocities along,
//! across and against the car, zero and tiny, the lock bit, parked cars and trailers, the focus about 75 m off; each
//! wheel's angle after, from [`spin_near`] and [`wheel_roll`] with the update's own drawn steer (f25).

use super::*;

#[test]
fn wheel_roll_matches_the_original() {
    let (mut frames, mut rolled, mut front) = (0, 0, 0);
    for line in include_str!("testdata/wheel_roll_ref.txt").lines() {
        let (inp, out) = line.split_once(" -> ").unwrap();
        let w: Vec<&str> = inp.split_whitespace().collect();
        let int = |i: usize| w[i].parse::<u32>().unwrap();
        let f = |i: usize| f32::from_bits(u32::from_str_radix(w[i], 16).unwrap());
        let v3 = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
        let o: Vec<f32> = out.split_whitespace().map(|h| f32::from_bits(u32::from_str_radix(h, 16).unwrap())).collect();
        let (has_b, trailer, n, lock) = (int(1) != 0, int(3) != 0, int(5) as usize, int(6) != 0);
        // (c[]: 7 park, 8 hub z, 9/10 focus x z, 11..13 place, 14..22 rows, 29 dt, 30..32 velocity, 33..38 hub y, 39..44 angles)
        let focus = Vec3::new(f(9), 0.0, f(10));
        let near = spin_near(focus, v3(11));
        let rows = [v3(14), v3(17), v3(20)];
        let parked = !has_b && !trailer;
        for k in 0..6 {
            let a = f(39 + k);
            let got = if k < n && near {
                rolled += 1;
                front += (k < 2) as i32;
                wheel_roll(a, rows, v3(30), o[0], k < 2, f(33 + k), f(29), lock, parked)
            } else {
                a
            };
            assert_eq!(got.to_bits(), o[1 + k].to_bits(), "frame {frames} wheel {k}: {got} vs {}: {line}", o[1 + k]);
        }
        frames += 1;
    }
    assert!(frames >= 2700 && rolled > 5000 && front > 2000, "{frames} {rolled} {front}");
}
