//! mcCarDamage's contact callback (F-11688, vtable slot 3): one call per run of a car's contacts in a tick the solver gave
//! them an impulse. It reads the run's change of velocity dv (F-19931) and decides the camera's jolt, the pad's rumbles,
//! the LandingDampFactor set by a roof-side contact, the point of the hit, the scrape and the zone damage of both kinds,
//! and whether the hit goes on to F-96323. Check: docs/reference/vehicle/contact_call_check.md.

use glam::Vec3;
use super::artic::V4;
use super::exact::fma;

/// The run as the callback reads it.
#[derive(Clone, Copy, Debug)]
pub struct RunIn {
    /// dv, the run's change of velocity (F-19931)
    pub dv: Vec3,
    /// chassis +224
    pub speed: f32,
    /// mcCarDamageTune +48 JoltThreshold, +32 ImpactThreshold, +52 (the jolt's scale)
    pub jolt: f32,
    pub impact: f32,
    pub jolt_scale: f32,
    /// F-31409 of the car (the local human's car) and the pause manager's byte +6 clear
    pub local_human: bool,
    pub quiet: bool,
    /// the racer's camera when it takes jolts (its +89 == 1): the shake tune's +36, +40, +44, +48
    pub cam: Option<[f32; 4]>,
    /// the collider's rows 0 and 1
    pub row0: Vec3,
    pub row1: Vec3,
    /// the run's first contact's normal (pair +80, negated when the car is the pair's second side) and, for a run of
    /// more than one, all of them summed in order with `add` (the vector unit's adds)
    pub n_first: Vec3,
    pub n_sum: Option<Vec3>,
    /// the run's last contact's own point (F-83617)
    pub last: Vec3,
    /// the point of the run's first enabled contact (pair +165 & 0x80; the first contact when none is)
    pub point: Vec3,
    /// the two velocities at the point (slot 21 of the two instances; zero without one)
    pub v1: Vec3,
    pub v2: Vec3,
    /// the car's body velocity (chassis [+128] +272) and the other instance's, when there is one
    pub own_c: Vec3,
    pub other_c: Option<Vec3>,
    /// F-14919 (a human's car), chassis slot 11 (wheels on the ground), sim tune +300 LandingDampFactor
    pub human: bool,
    pub wheels: i32,
    pub landing: f32,
    /// the own instance of the pair is there (the zone damage needs it)
    pub own: bool,
}

/// What the call does, in its order: the jolt (the racing wheel's effect, the camera's, the pad's rumble 1), the
/// aero set, the scrape's rumble 2 (with a draw of the game's generator 0x82860880), the zone damage kind 0 (then
/// rumble 1) and kind 1 (then rumble 2), the largest sqrt dv; then F-96323 at `point` unless gated by its caller.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunOut {
    /// |dv| over the speed's threshold for the local human's car: the wheel's effect's duration (ms) and side
    /// (270 / 90; 0 none) and the pad's rumble 1
    pub jolt: Option<(i32, i32)>,
    /// F-68650's two arguments (f1, f2)
    pub cam: Option<(f32, f32)>,
    /// aero +4 = LandingDampFactor
    pub aero: Option<f32>,
    /// the scrape's test passed: rumble 2 when local human and quiet, and one draw of the generator
    pub scrape: bool,
    pub point: Vec3,
    /// the summed normal (unit over 0.001) and the centres' relative velocity
    pub n: Vec3,
    pub rel: Vec3,
    pub zone0: Option<f32>,
    pub zone1: Option<f32>,
    /// sqrt |dv|, for the largest seen (0x8288DCC4); None without the own instance or under 0.1
    pub root: Option<f32>,
}

fn v4(v: Vec3) -> V4 {
    V4::new(v.x, v.y, v.z, 0.0)
}

fn v3(v: V4) -> Vec3 {
    Vec3::new(v.0[0], v.0[1], v.0[2])
}

/// One more normal into the run's sum (vaddfp).
pub fn add(sum: Vec3, n: Vec3) -> Vec3 {
    v3(v4(sum).add(v4(n)))
}

/// fsel(a >= 0, b, c)
fn sel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 { b } else { c }
}

/// x at least 0 (NaN 0) and at most 1, as the two fsel
fn unit_clamp(x: f32) -> f32 {
    let x = sel(x, x, 0.0);
    sel(x - 1.0, 1.0, x)
}

/// F-11688 after its gate (+36 under MaxDamage) and the other object's check, for one run.
pub fn run(r: &RunIn) -> RunOut {
    let mut o = RunOut { point: r.point, ..Default::default() };
    let d = r.dv;
    let len = fma(d.z, d.z, fma(d.x, d.x, d.y * d.y)).sqrt();
    let t = unit_clamp((r.speed - 5.0) * f32::from_bits(0x3d23_d70a));
    let thr = fma(t, r.impact - r.jolt, r.jolt);
    if len > thr && r.local_human && r.quiet {
        if let Some(c) = r.cam {
            let k = unit_clamp((len - thr) / r.jolt_scale);
            o.cam = Some((fma(c[3] - c[1], k, c[1]), fma(c[2] - c[0], k, c[0])));
        }
        let k = unit_clamp(len * f32::from_bits(0x3d4c_cccd));
        let ms = ((k + 1.0) * 50.0) as i32;
        let side = fma(r.row0.x, d.x, fma(r.row0.y, d.y, r.row0.z * d.z)) / len;
        // (a NaN is the left: the second compare is bge)
        let way = if side > f32::from_bits(0x3f19_999a) { 270 } else if !(side >= f32::from_bits(0xbf19_999a)) { 90 } else { 0 };
        o.jolt = Some((ms, way));
    }
    // (the normals of the run summed in order, unit over 0.001; one contact's as it is)
    let mut n = v4(r.n_first);
    if let Some(s) = r.n_sum {
        let s = v4(s);
        let l = fma(s.0[0], s.0[0], fma(s.0[1], s.0[1], s.0[2] * s.0[2])).sqrt();
        if l > f32::from_bits(0x3a83_126f) {
            n = s.mul(V4::splat(1.0 / l));
        }
    }
    let n = v3(n);
    o.n = n;
    if r.landing > 1.0 && r.human && r.wheels > 0 {
        let up = -fma(r.row1.x, n.x, fma(r.row1.z, n.z, r.row1.y * n.y));
        if up > f32::from_bits(0x3f49_0fdb) {
            o.aero = Some(r.landing);
        }
    }
    let rel = match r.other_c {
        Some(c) => v3(v4(r.own_c).sub(v4(c))),
        None => r.own_c,
    };
    o.rel = rel;
    if !(len > f32::from_bits(0x3dcc_cccd)) {
        return o;
    }
    if !(r.row1.y >= f32::from_bits(0x3dcc_cccd)) {
        o.point = r.last;
    }
    let mut scrape = 0.0f32;
    let vl = fma(rel.x, rel.x, fma(rel.y, rel.y, rel.z * rel.z)).sqrt();
    if vl > 3.0 {
        let c = fma(rel.x, n.x, fma(rel.z, n.z, rel.y * n.y)) / vl;
        let lim = if n.y > 0.5 { f32::from_bits(0x3f33_3333) } else { f32::from_bits(0x3f19_999a) };
        if c < lim && c > -lim {
            let a = fma(r.v1.y, r.v1.y, fma(r.v1.x, r.v1.x, r.v1.z * r.v1.z));
            let b = fma(r.v2.y, r.v2.y, fma(r.v2.x, r.v2.x, r.v2.z * r.v2.z));
            scrape = (a - b).abs() * f32::from_bits(0x38d1_b717);
            o.scrape = true;
        }
    }
    if !r.own {
        return o;
    }
    let s = sel(scrape, scrape, 0.0);
    let scrape = sel(s - 0.125, 0.125, s);
    let root = len.sqrt();
    o.root = Some(root);
    let a0 = unit_clamp((root - 0.5) * f32::from_bits(0x3e4c_cccd));
    if a0 != 0.0 {
        o.zone0 = Some(a0);
    }
    if scrape != 0.0 {
        o.zone1 = Some(scrape);
    }
    o
}

#[cfg(test)]
mod offline {
    use super::*;
    use std::collections::HashMap;

    /// F-11688 against the original run offline (tools/contact_call_reference.py): 6,000 calls - the gate, the other
    /// object (none, active, not), mcCarStuck's call, the first enabled contact as A or B, the two sides' instances
    /// (none, the body, another), the hit-by masks, dv about the jolt's threshold and 0.1, speeds about 5 and 30 m/s,
    /// the camera there or not, rows lying over, roof-side normals, scrapes along and across, the cheat byte, slot 12.
    /// Every call the original makes (with its arguments) and the state it leaves equal what `run` and the caller's
    /// gates give.
    #[test]
    fn contact_call_matches_the_original() {
        let text = include_str!("testdata/contact_call_ref.txt");
        let mut lines = text.lines();
        let hex = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let base: Vec<u32> = lines.next().unwrap().split_whitespace().map(hex).collect();
        const THIS: u32 = 0x1000;
        const DT: u32 = 0x1080;
        const INST: u32 = 0x1100;
        const CH: u32 = 0x1300;
        const BODY: u32 = 0x1500;
        const ST: u32 = 0x1640;
        const I: u32 = 0x1840;
        const IT: u32 = 0x1880;
        const ARR: u32 = 0x18A0;
        const TBL: u32 = 0x2980;
        const PM: u32 = 0x2A00;
        const RAC: u32 = 0x3000;
        const CAM: u32 = 0x3400;
        const CT: u32 = 0x3580;
        let ents = [0x2700u32, 0x2780, 0x2800];
        let xe = [0x28A0u32, 0x28C0, 0x28E0];
        let cars = [0x3C00u32, 0x3C40, 0x3C80];
        let (mut n, mut jolts, mut zones, mut hits) = (0, 0, 0, 0);
        for line in lines.filter(|l| !l.is_empty()) {
            let (pairs, out) = line.split_once(" -> ").unwrap();
            let (state, log) = out.split_once(" |").unwrap();
            let mut m: HashMap<u32, u32> = HashMap::new();
            let mut ret = [0u32; 32];
            let mut b8: HashMap<u32, u32> = HashMap::new();
            let mut h16: HashMap<u32, u32> = HashMap::new();
            for p in base.chunks(2).chain(pairs.split_whitespace().map(hex).collect::<Vec<_>>().chunks(2)) {
                let (a, v) = (p[0], p[1]);
                if a >= 0xF000_0000 {
                    ret[(a - 0xF000_0000) as usize] = v;
                } else if a & 0x8000_0000 != 0 {
                    m.insert(a, v);
                } else if a & 0x4000_0000 != 0 {
                    b8.insert(a & 0x3FFF_FFFF, v & 0xFF);
                } else if a & 0x2000_0000 != 0 {
                    h16.insert(a & 0x1FFF_FFFF, v & 0xFFFF);
                } else {
                    m.insert(a, v);
                }
            }
            let g = |a: u32| *m.get(&a).unwrap_or(&0);
            let byte = |a: u32| *b8.get(&a).unwrap_or(&0);
            let half = |a: u32| *h16.get(&a).unwrap_or(&0);
            let gf = |a: u32| f32::from_bits(g(a));
            let vec = |a: u32| Vec3::new(gf(a), gf(a + 4), gf(a + 8));
            let mut want: Vec<String> = Vec::new();
            let (mut mask84, mut mask86, mut mask89) = (half(THIS + 84), half(THIS + 86), byte(THIS + 89));
            let mut aero = g(0x14C4);
            let mut stat = g(0x8288_DCC4);
            let mut gen = [g(0x8286_0880), g(0x8286_0884)];
            'call: {
                if !(gf(THIS + 36) < gf(DT + 24)) {
                    break 'call;
                }
                let count = g(IT + 12);
                let pair = |k: u32| g(ARR + 4 * k);
                let ab = |k: u32| g(pair(k) + 160);
                let own_a = |k: u32| g(ab(k)) == g(IT + 4);
                // (the other object of the pair at the iterator's cursor)
                let c0 = g(IT + 8);
                let other = if own_a(c0) { g(ab(c0) + 4) } else { g(ab(c0)) };
                if other == 0 {
                    break 'call;
                }
                let lv = half(other + 8);
                let active = lv != 0xFFFF && g(TBL + 8 * lv + 4) & 3 == 0;
                if !active {
                    want.push(format!("K,{:x},{:x},{:x},{:x}", ARR, g(IT + 4), c0, count));
                }
                // (the first enabled contact)
                let mut k = 0;
                while k < count && byte(pair(k) + 165) & 0x80 == 0 {
                    k += 1;
                }
                if k >= count {
                    k = 0;
                }
                let (pr, a) = (pair(k), ab(k));
                let oa = own_a(k);
                let straight = g(if oa { a + 8 } else { a + 12 }) == g(CH + 128) || g(IT + 4) == g(INST + 28);
                let (r26, r25, point, p26, p25) = if straight {
                    if oa { (g(a + 8), g(a + 12), vec(pr + 48), g(a + 28), g(a + 36)) } else { (g(a + 12), g(a + 8), vec(pr + 64), g(a + 36), g(a + 28)) }
                } else if oa {
                    (g(a + 12), g(a + 8), vec(pr + 64), g(a + 36), g(a + 28))
                } else {
                    (g(a + 8), g(a + 12), vec(pr + 48), g(a + 28), g(a + 36))
                };
                // (the racer index of an instance's vehicle: class 8 with flags 0xC; 0x80 with flag 4)
                let ent_of = |inst: u32| -> Option<usize> {
                    if inst == 0 {
                        return None;
                    }
                    let e = g(inst + 16);
                    if e == 0 || g(g(e + 4) + 16) & 0xC == 0 {
                        return None;
                    }
                    ents.iter().position(|&x| x == e)
                };
                let mut r29: i32 = -1;
                if let Some(e) = ent_of(r26) {
                    want.push(format!("Y{e}"));
                    if ret[1 + e] as i32 == 8 {
                        want.push(format!("X{}", cars.iter().position(|&c| c == g(ents[e] + 80)).unwrap()));
                        r29 = ret[17 + e] as i32;
                        if r29 >= 0 && g(xe[e] + 16) & 4 != 0 {
                            r29 |= 128;
                        }
                    }
                }
                if let Some(e) = ent_of(r25) {
                    want.push(format!("Y{e}"));
                    if ret[1 + e] as i32 == 8 {
                        want.push(format!("X{}", cars.iter().position(|&c| c == g(ents[e] + 80)).unwrap()));
                        let mut x = ret[17 + e] as i32;
                        if x >= 0 && g(xe[e] + 16) & 4 != 0 {
                            x |= 128;
                        }
                        if x >= 0 {
                            let shl = |s: u32| if s < 32 { 1u32 << s } else { 0 };
                            if x & 128 != 0 {
                                mask86 = (mask86 | shl((x ^ 128) as u32)) & 0xFFFF;
                            } else {
                                mask84 = (mask84 | shl(x as u32)) & 0xFFFF;
                                want.push(format!("H,{:x}", g(ents[e] + 80)));
                                mask89 |= ret[7] & 0xFF;
                            }
                        }
                    }
                }
                // (the velocities at the point, slot 21: the own instance's first)
                let mut calls = 0;
                let mut vel = |inst: u32, part: u32| -> Vec3 {
                    if inst == 0 {
                        return Vec3::ZERO;
                    }
                    want.push(format!("U,{:x},{:x},{:x},{:x},{:x}", inst, part, point.x.to_bits(), point.y.to_bits(), point.z.to_bits()));
                    let v = Vec3::new(f32::from_bits(ret[8 + 3 * calls]), f32::from_bits(ret[9 + 3 * calls]), f32::from_bits(ret[10 + 3 * calls]));
                    calls += 1;
                    v
                };
                let v1 = vel(r26, p26);
                let v2 = vel(r25, p25);
                want.push(format!("V,{:x},{:x}", g(INST + 28), k));
                let dv = Vec3::new(f32::from_bits(ret[20]), f32::from_bits(ret[21]), f32::from_bits(ret[22]));
                let normals: Vec<Vec3> = (0..count).map(|j| {
                    let v = vec(pair(j) + 80);
                    if own_a(j) { v } else { v3(V4::ZERO.sub(v4(v))) }
                }).collect();
                let n_sum = (count > 1).then(|| normals[1..].iter().fold(normals[0], |s, m| add(s, *m)));
                let last = vec(pair(count - 1) + if own_a(count - 1) { 48 } else { 64 });
                let cam_ok = g(RAC + 880) != 0 && byte(CAM + 89) == 1;
                let r = RunIn {
                    dv, speed: gf(CH + 224), jolt: gf(DT + 48), impact: gf(DT + 32), jolt_scale: gf(DT + 52),
                    local_human: ret[6] & 0xFF != 0, quiet: byte(PM + 6) == 0,
                    cam: cam_ok.then(|| [gf(CT + 36), gf(CT + 40), gf(CT + 44), gf(CT + 48)]),
                    row0: vec(g(INST + 28) + 16), row1: vec(g(INST + 28) + 32), n_first: normals[0], n_sum, last, point, v1, v2,
                    own_c: vec(g(CH + 128) + 272), other_c: (r25 != 0).then(|| vec(r25 + 272)),
                    human: ret[16] & 0xFF != 0, wheels: ret[4] as i32, landing: gf(ST + 300), own: r26 != 0,
                };
                let o = run(&r);
                // (the jolt's calls: the human test first, the racer, the camera, the wheel, the pad)
                let len = fma(dv.z, dv.z, fma(dv.x, dv.x, dv.y * dv.y)).sqrt();
                let thr = fma(unit_clamp((r.speed - 5.0) * f32::from_bits(0x3d23_d70a)), r.impact - r.jolt, r.jolt);
                if len > thr {
                    want.push(format!("H,{:x}", INST));
                    if r.local_human && r.quiet {
                        want.push(format!("G,{:x}", r29 as u32));
                    }
                }
                if let Some((ms, way)) = o.jolt {
                    if let Some((f1, f2)) = o.cam {
                        want.push(format!("C,{:x},{:x},{:x}", CAM, f1.to_bits(), f2.to_bits()));
                    }
                    want.push(format!("J,{:x},{:x},{:x}", g(RAC + 896), ms, way));
                    jolts += 1;
                    if ret[18] & 0xFF == 0 {
                        want.push("R1".into());
                    }
                }
                want.push("N".into());
                if r.landing > 1.0 {
                    want.push("F".into());
                }
                if let Some(a) = o.aero {
                    aero = a.to_bits();
                }
                if o.scrape {
                    // (the generator's step, its draw unused)
                    let x = (gen[0] as u64) * 0x5CDC_FAA7 + gen[1] as u64;
                    gen = [x as u32, (x >> 32) as u32];
                    if r.quiet {
                        want.push(format!("H,{:x}", INST));
                        if r.local_human {
                            want.push("R2".into());
                        }
                    }
                }
                if let Some(root) = o.root {
                    if root > f32::from_bits(stat) {
                        stat = root.to_bits();
                    }
                }
                let p = o.point;
                let pv = |v: Vec3| format!("{:x},{:x},{:x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
                if let Some(a) = o.zone0 {
                    want.push(format!("Z,{:x},0,{:x},{},0,3f800000,0,{}", 0x1800 - 16, a.to_bits(), pv(p), pv(vec(INST + 400))));
                    want.push("R1".into());
                    zones += 1;
                }
                if let Some(a) = o.zone1 {
                    want.push(format!("Z,{:x},1,{:x},{},{},{}", 0x1800 - 16, a.to_bits(), pv(p), pv(o.n), pv(o.rel)));
                    want.push("R2".into());
                    zones += 1;
                }
                if byte(THIS + 33) != 1 && g(0x8288_E5A4) & 0xFF == 0 {
                    want.push("M".into());
                    if ret[5] & 0xFF != 0 {
                        want.push(format!("T,{:x},1,0,ffffffff,{},{}", r25, pv(p), pv(dv)));
                        hits += 1;
                    }
                }
            }
            let got_state = format!("{:x} {:x} {:x} {:08x} {:08x} {:04x}:{:04x} {:04x}:{:04x}", mask84, mask86, mask89, aero, stat, gen[0] >> 16, gen[0] & 0xFFFF, gen[1] >> 16, gen[1] & 0xFFFF);
            assert_eq!(got_state, state, "{pairs}");
            let toks: Vec<&str> = log.split_whitespace().collect();
            assert_eq!(toks, want, "{pairs}");
            n += 1;
        }
        assert_eq!((n, jolts, zones, hits), (6000, 500, 2694, 1954));
    }
}
