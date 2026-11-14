//! The driver's contact walker (F-83713, mcRacer and its kin): per contact of the car's run, the law word's bits for the
//! police (car +484) and the new-contact table (+492, F-60290). Check: docs/reference/vehicle/law_contact_check.md.

use glam::Vec3;
use super::artic::V4;
use super::exact::fma;

/// 4.47 m/s: either car must be faster for a car contact to count
pub const MOVING: f32 = 4.47;
/// 0.2235 m/s: a car slower than this stands (the other is the aggressor)
pub const STANDING: f32 = 0.2235;
/// 6.705 m/s: the own speed an ambient car's new contact needs
pub const AMBIENT: f32 = 6.705;

/// A car contact (class 8): None under the gate (neither speed over 4.47 m/s), else whether the own car is the
/// aggressor - the other when the own car stands (under 0.2235), the own when the other stands, else the one whose
/// velocity points more at the other ((v . d) / speed, d the other's place less the own, fused as the original).
/// `speed`: chassis +224 of each; `pos`: each racer's matrix place; `vel`: each body's +272.
pub fn aggressor(own_speed: f32, other_speed: f32, own_pos: Vec3, other_pos: Vec3, own_vel: Vec3, other_vel: Vec3) -> Option<bool> {
    let (f13, f0) = (own_speed, other_speed);
    if !(f13 > MOVING) && !(f0 > MOVING) {
        return None;
    }
    if !(f0 >= STANDING) {
        return Some(true);
    }
    if !(f13 > STANDING) {
        return Some(false);
    }
    let d = V4::new(other_pos.x, other_pos.y, other_pos.z, 0.0).sub(V4::new(own_pos.x, own_pos.y, own_pos.z, 0.0));
    let nd = V4::ZERO.sub(d);
    let (d, nd) = (d.0, nd.0);
    let a = fma(own_vel.x, d[0], fma(own_vel.z, d[2], own_vel.y * d[1])) / f13;
    let b = fma(other_vel.x, nd[0], fma(other_vel.y, nd[1], other_vel.z * nd[2])) / f0;
    Some(a > b)
}

/// The aggressor's law bit for a car contact: 2 against a police car (+465), 4 against the human's car (slot 10), else 8.
pub fn car_bit(victim_cop: bool, victim_human: bool) -> u32 {
    if victim_cop {
        2
    } else if victim_human {
        4
    } else {
        8
    }
}

/// An ambient car's contact (class 6) counts when it is new and the own speed is over 6.705 m/s: bit 8 and the table.
pub fn ambient_counts(new: bool, own_speed: f32) -> bool {
    new && own_speed > AMBIENT
}

#[cfg(test)]
mod offline {
    use super::*;
    use std::collections::HashMap;

    /// F-83713 against the original run offline (tools/law_contact_reference.py): 6,000 walks of 1 .. 6 contacts - every
    /// class, frags with and without their bit / entry / parent chain, the table holding some of the instances, speeds at
    /// and about 0.2235, 4.47 and 6.705 m/s, both racers' matrices (with and without a car, either thread buffer), police
    /// and human victims. Every table add, slot 10 and slot 40 call (with its arguments) and every law word equal.
    #[test]
    fn law_contact_matches_the_original() {
        let text = include_str!("testdata/law_contact_ref.txt");
        let mut lines = text.lines();
        let hex = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let base: Vec<u32> = lines.next().unwrap().split_whitespace().map(hex).collect();
        const R: u32 = 0x1000;
        const TLS: u32 = 0x1B00;
        const LAWW: u32 = 0x1BA0;
        const IT: u32 = 0x1BC0;
        let o = |k: u32| 0x2000 + 0x100 * k;
        let orr = |k: u32| 0x5200 + 0x200 * k;
        let (mut n, mut cars, mut adds) = (0, 0, 0);
        for line in lines.filter(|l| !l.is_empty()) {
            let (pairs, out) = line.split_once(" -> ").unwrap();
            let (state, log) = out.split_once(" |").unwrap();
            let mut m: HashMap<u32, u32> = HashMap::new();
            let mut b8: HashMap<u32, u32> = HashMap::new();
            let mut ret = [0u32; 32];
            for p in base.chunks(2).chain(pairs.split_whitespace().map(hex).collect::<Vec<_>>().chunks(2)) {
                let (a, v) = (p[0], p[1]);
                if a >= 0xF000_0000 {
                    ret[(a - 0xF000_0000) as usize] = v;
                } else if a & 0x8000_0000 == 0 && a & 0x4000_0000 != 0 {
                    b8.insert(a & 0x3FFF_FFFF, v & 0xFF);
                } else {
                    m.insert(a, v);
                }
            }
            let g = |a: u32| *m.get(&a).unwrap_or(&0);
            let gf = |a: u32| f32::from_bits(g(a));
            let vec = |a: u32| Vec3::new(gf(a), gf(a + 4), gf(a + 8));
            let byte = |a: u32| *b8.get(&a).unwrap_or(&0);
            let fx = |x: f32| format!("{:x}", x.to_bits());
            let mut words: HashMap<u32, u32> = HashMap::new();
            words.insert(R, g(LAWW));
            for k in 0..5 {
                words.insert(orr(k), 0);
            }
            let mut want: Vec<String> = Vec::new();
            let table: Vec<u32> = (0..8).map(|j| g(R + 492 + 12 * j)).filter(|&x| x != 0).collect();
            let found = |x: u32| table.contains(&x);
            // (the racer's matrix place, F-35658)
            let place = |r: u32| {
                let car = g(r + 48);
                if car == 0 {
                    vec(r + 336)
                } else if byte(TLS + 72) != 0 {
                    vec(car + 320)
                } else {
                    vec(g(car + 28) + 64)
                }
            };
            let car = g(R + 48);
            let car_ok = car != 0 && g(car + 8) != 0 && g(car + 28) != 0;
            for c in 0..g(IT + 12) {
                let inst = ret[c as usize];
                if inst == 0 {
                    continue;
                }
                let k = (inst - 0x2000) / 0x100;
                let cls = ret[8 + k as usize] as i32;
                let speed = || gf(g(car + 8) + 224);
                match cls {
                    3 => {
                        if g(inst + 80) & 2 == 0 || g(inst + 84) == 0 || gf(g(inst + 84) + 420) != 0.0 {
                            continue;
                        }
                        let up = |x: u32| if g(x + 84) != 0 { g(g(x + 84) + 256) } else { x };
                        let mut key = inst;
                        if up(inst) != 0 {
                            loop {
                                if up(key) == key {
                                    break;
                                }
                                key = up(key);
                                if up(key) == 0 {
                                    break;
                                }
                            }
                        }
                        if !car_ok || found(key) {
                            continue;
                        }
                        *words.get_mut(&R).unwrap() |= 1;
                        want.push(format!("T,{:x},{:x},{}", R, key, fx(speed())));
                        adds += 1;
                    }
                    4 | 5 | 9 | 10 | 11 => {
                        if !car_ok || found(inst) {
                            continue;
                        }
                        *words.get_mut(&R).unwrap() |= 1;
                        want.push(format!("T,{:x},{:x},{}", R, inst, fx(speed())));
                        adds += 1;
                    }
                    6 => {
                        if !car_ok || !ambient_counts(!found(inst), speed()) {
                            continue;
                        }
                        *words.get_mut(&R).unwrap() |= 8;
                        want.push(format!("T,{:x},{:x},{}", R, inst, fx(speed())));
                        adds += 1;
                    }
                    8 => {
                        if found(inst) || !car_ok {
                            continue;
                        }
                        let oc = g(inst + 80);
                        if oc == 0 || g(oc + 8) == 0 || g(oc + 128) == 0 {
                            continue;
                        }
                        let (och, other) = (g(oc + 8), g(oc + 128));
                        let (f13, f0) = (speed(), gf(och + 224));
                        let own_vel = vec(g(g(car + 8) + 128) + 272);
                        let other_vel = vec(g(och + 128) + 272);
                        let Some(own_aggr) = aggressor(f13, f0, place(R), place(other), own_vel, other_vel) else { continue };
                        let own_coll = g(car + 28);
                        let (aggr, aggr_inst, victim, victim_inst, v) = if own_aggr { (R, own_coll, other, inst, f13) } else { (other, inst, R, own_coll, f0) };
                        let cop = byte(victim + 465) != 0;
                        let human = if cop {
                            false
                        } else {
                            want.push(format!("S,{:x}", victim));
                            ret[if victim == R { 16 } else { 17 + ((victim - 0x5200) / 0x200) as usize }] & 0xFF != 0
                        };
                        *words.get_mut(&aggr).unwrap() |= car_bit(cop, human);
                        want.push(format!("A,{:x},1,{:x},{}", aggr, victim_inst, fx(v)));
                        want.push(format!("A,{:x},0,{:x},{}", victim, aggr_inst, fx(v)));
                        cars += 1;
                    }
                    _ => {}
                }
            }
            let got = format!("{:x} {:x} {:x} {:x} {:x} {:x}", words[&R], words[&orr(0)], words[&orr(1)], words[&orr(2)], words[&orr(3)], words[&orr(4)]);
            assert_eq!(got, state, "{pairs}");
            assert_eq!(log.split_whitespace().collect::<Vec<_>>(), want, "{pairs}");
            n += 1;
        }
        assert_eq!((n, cars, adds), (6000, 644, 3299));
    }
}
