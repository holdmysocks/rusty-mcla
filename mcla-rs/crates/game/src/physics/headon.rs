//! The head-on wipeout (F-43509, mcCarInstance vtable slot 25, after mcCarDamage's contact call):
//! a car that drives into the nose of an ambient (traffic or parked) car, or straight into something
//! fixed, above 22.35 m/s with a run of contacts that changed its velocity by at least its
//! WipeoutThreshold is wiped out. Rule: docs/reference/cameras/carhit_wipeout_rule.md.

use glam::Vec3;
use super::artic::V4;

/// The other side of one contact of the run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Partner {
    /// an ambient car (mcAmbientPhysInst, class 6: traffic and parked cars), with its back axis
    /// (its matrix row 2)
    Ambient { back: Vec3 },
    /// an mcCarInstance (class 8) whose driver is the player (only an AI car's own test sees one),
    /// with its back axis
    PlayerCar { back: Vec3 },
    /// something of level state 2: the city, standing posts
    Fixed,
    /// anything else (a racer, a cop, a loose prop, a piece): nothing
    Other,
}

/// One enabled contact of the run, as the own car has it.
#[derive(Clone, Copy, Debug)]
pub struct Touch {
    /// the impulse on the own car (contact +112, negated on the pair's second side)
    pub impulse: Vec3,
    /// the contact normal pointing into the own car (contact +80, negated on the second side)
    pub normal: Vec3,
    /// the contact point on the own car
    pub point: Vec3,
    pub partner: Partner,
}

/// The own car as the rule reads it.
#[derive(Clone, Copy, Debug)]
pub struct Own {
    /// the run's velocity change (F-19931: the impulses over the mass, 0.4 of those against
    /// fixed things)
    pub dv: Vec3,
    /// chassis +224
    pub speed: f32,
    /// the body's velocity (collider +272) and place (+256)
    pub vel: Vec3,
    pub body_pos: Vec3,
    /// the instance matrix's row 2 (the back axis)
    pub back: Vec3,
    /// mcCarDamageTune WipeoutThreshold (+36), MaxDamage (+24); simTune HeadOnSpinMult (+284)
    pub wipeout_threshold: f32,
    pub max_damage: f32,
    pub spin_mult: f32,
    /// mcCarDamage byte +33 (no damage), and the player's own Agro running (ability +52 == 1, +48 == 3)
    pub no_damage: bool,
    pub agro: bool,
}

/// What the rule did over the run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeadOn {
    /// the angular impulses for the collider (+384, phCollider slot 38), one a qualifying ambient contact, in order
    pub spins: Vec<Vec3>,
    /// damage points added (MaxDamage x 0.1 per qualifying contact, F-15289)
    pub damage: f32,
    /// a contact qualified: the wipeout (F-69183), the slow motion (F-46437); from an
    /// ambient car also the tyres' melt = 1
    pub hit: bool,
    pub melt: bool,
    /// a fixed partner qualified: the CG goes to ModelOffset's height (ch+112 = ch+96)
    pub cg_drop: bool,
    /// how many contacts qualified (each adds 0.1 of MaxDamage)
    pub hits: u32,
}

/// The game's shared generator (0x82860890, multiply with carry, a = 0x5CDCFAA7). (approx.: the
/// state is the port's own: the original's is drawn by 53 other users)
static RNG: std::sync::Mutex<[u32; 2]> = std::sync::Mutex::new([0x2545_F491, 0x1234_5678]);

fn draw2(s: &mut [u32; 2]) -> (u32, u32) {
    let a = 0x5CDC_FAA7u64;
    let t1 = s[0] as u64 * a + s[1] as u64;
    let x1 = (t1 as u32) & 0x7F_FFFF;
    let t2 = (t1 as u32) as u64 * a + (t1 >> 32);
    let x2 = (t2 as u32) & 0x7F_FFFF;
    *s = [t2 as u32, (t2 >> 32) as u32];
    (x1, x2)
}

/// sgn(v) |v|^0.33 per axis: the power in double (F-21955, `pow_guest`), rounded, times the sign the two fsel make (1 above
/// 0, 0 at either zero, -1 below; a NaN takes -1).
fn soft(r: Vec3) -> Vec3 {
    let f = |v: f32| {
        let m = crate::caraudio::pow_guest(v.abs() as f64, f64::from_bits(0x3fd5_1eb8_6000_0000)) as f32;
        let s = if -v >= 0.0 { 0.0 } else { 1.0 };
        let s = if v >= 0.0 { s } else { -1.0 };
        m * s
    };
    Vec3::new(f(r.x), f(r.y), f(r.z))
}

fn v4(v: Vec3) -> V4 {
    V4::new(v.x, v.y, v.z, 0.0)
}

fn v3(v: V4) -> Vec3 {
    Vec3::new(v.0[0], v.0[1], v.0[2])
}

/// One call of the rule for a run of contacts (the own car's run as A or as B), with the game's generator.
pub fn run(own: &Own, touches: &[Touch]) -> HeadOn {
    let mut s = RNG.lock().unwrap_or_else(|e| e.into_inner());
    run_with(own, touches, &mut s)
}

/// F-43509 for one run, the generator's state given (0x82860890). Checked against the original run offline:
/// tools/headon_reference.py, test `head_on_matches_the_original`.
pub fn run_with(own: &Own, touches: &[Touch], rng: &mut [u32; 2]) -> HeadOn {
    use super::exact::{dot_xyz, fma, fms};
    let mut out = HeadOn::default();
    if std::env::var_os("MCLA_DBG_HEADON").is_some() && own.dv.length() > 1.0 {
        // (test hook: the run's gates and each contact's two tests)
        let u = (own.vel - own.back).normalize_or_zero();
        eprintln!("HEADON dv {:.3} / {:.1} speed {:.2} agro {} touches {}", own.dv.length(), own.wipeout_threshold, own.speed, own.agro, touches.len());
        for t in touches {
            let b = match t.partner {
                Partner::Ambient { back } | Partner::PlayerCar { back } => back,
                _ => Vec3::ZERO,
            };
            eprintln!("  {:?} n.back {:.3} u.back {:.3} n.ownback {:.3} |J| {:.0}", t.partner, b.dot(t.normal), u.dot(b), own.back.dot(t.normal), t.impulse.length());
        }
    }
    if own.no_damage || own.agro {
        return out;
    }
    // |dv| as fsqrts of z z + (x x + y y), one fused chain
    let d = own.dv;
    if fma(d.z, d.z, fma(d.x, d.x, d.y * d.y)).sqrt() < own.wipeout_threshold {
        return out;
    }
    let guest_cos = |bits: u64| mcla_formats::guest_math::guest_cos(f64::from_bits(bits)) as f32;
    let fast = || own.speed > f32::from_bits(0x41b2_cccd);
    for t in touches {
        match t.partner {
            Partner::Ambient { back } | Partner::PlayerCar { back } => {
                if !fast() {
                    continue;
                }
                // the own car's way (0 - its back axis + the body's velocity), normalised as the vector unit does it:
                // vrsqrtefp (a zero length gives 0) and one Newton step
                let u = V4::ZERO.sub(v4(own.back)).add(v4(own.vel));
                let l = u.dot3(u);
                let y = V4(l.rsqrte().0.map(|x| if x == f32::INFINITY { 0.0 } else { x }));
                let k = l.nmsub(y.mul(y), V4::splat(1.0)).madd(V4::splat(0.5).mul(y), y);
                let way = v3(u.mul(k));
                // the own car in front of the other's nose, moving against its facing
                if !(dot_xyz(back, t.normal) < f32::from_bits(0xbf33_3333)) || !(dot_xyz(back, way) > guest_cos(0x3fe9_21fb_6000_0000)) {
                    continue;
                }
                let (x1, x2) = draw2(rng);
                let c = f32::from_bits(0x3480_0000);
                let (f7, f6) = (fms(x1 as f32, c, 1.0), fms(x2 as f32, c, 1.0));
                let ky = fma(f7, 0.25, 1.0);
                let kxz = (f6 + 1.0) * f32::from_bits(0x3e19_999a);
                let r = v4(soft(v3(v4(t.point).sub(v4(own.body_pos)))));
                let imp = v4(t.impulse);
                let w = imp.yzx().nmsub(r.zxy(), imp.zxy().mul(r.yzx()));
                let w = V4::new(kxz * w.0[0], ky * w.0[1], kxz * w.0[2], 0.0);
                out.spins.push(v3(w.mul(V4::splat(own.spin_mult))));
                out.damage += own.max_damage * f32::from_bits(0x3dcc_cccd);
                out.hits += 1;
                out.hit = true;
                out.melt = true;
            }
            Partner::Fixed => {
                if !(dot_xyz(own.back, t.normal) > guest_cos(0x3fdb_ecde_6000_0000)) || !fast() {
                    continue;
                }
                out.cg_drop = true;
                out.damage += own.max_damage * f32::from_bits(0x3dcc_cccd);
                out.hits += 1;
                out.hit = true;
            }
            Partner::Other => {}
        }
    }
    out
}

/// The rule over the car's contacts with the city alone (one run, as A): the solved contacts in
/// their manifolds' order (`reordered`: the solver's order back to that), 0.4 of each impulse in dv.
/// Nothing when no contact took an impulse (the callbacks do not run).
pub fn hull_run(contacts: &[super::solver::Contact], reordered: Option<&[usize]>, inv_mass: f32, own: impl Fn(Vec3) -> Own) -> Vec<HeadOn> {
    let mut order: Vec<usize> = (0..contacts.len()).collect();
    if let Some(r) = reordered {
        for (new, old) in r.iter().enumerate() {
            order[*old] = new;
        }
    }
    let rows: Vec<&super::solver::Contact> = order.iter().map(|&i| &contacts[i]).collect();
    if !rows.iter().any(|c| c.impulse != Vec3::ZERO) {
        return Vec::new();
    }
    let dv = rows.iter().fold(Vec3::ZERO, |s, c| s + c.impulse * inv_mass * f32::from_bits(0x3ecc_cccd));
    let touches: Vec<Touch> = rows.iter().map(|c| Touch { impulse: c.impulse, normal: c.normal, point: c.on_body, partner: Partner::Fixed }).collect();
    vec![run(&own(dv), &touches)]
}

/// The rule over a loose island's runs of the car (its A-sorted run, then its B-sorted run, as
/// mcCarDamage's calls take them): one result per run the car has, nothing done for a run without
/// an impulse. The partner's class from its level object (6: an ambient car) and its back axis from
/// that object's instance matrix (row 2, P+48).
pub fn island_runs(out: &super::loose::IslandOut, objs: &[super::loose::Obj], inv_mass: f32, own: impl Fn(Vec3) -> Own) -> Vec<HeadOn> {
    use super::loose::Who;
    // (the thrown rider's rows with the bike are not in its runs here: unverified, as the bike's shared island)
    let a_rows: Vec<usize> = (0..out.links.len()).filter(|&i| out.links[i].a == Some(Who::Car) && !out.links[i].rider).collect();
    let b_rows: Vec<usize> = out.by_b.iter().copied().filter(|&i| out.links[i].b == Some(Who::Car) && !out.links[i].rider).collect();
    let mut hits = Vec::new();
    for rows in [a_rows, b_rows] {
        if rows.is_empty() {
            continue;
        }
        let (mut dv, mut any, mut touches) = (Vec3::ZERO, false, Vec::new());
        for i in rows {
            let (l, t) = (&out.links[i], &out.touches[i]);
            let car_a = l.a == Some(Who::Car);
            let other = if car_a { l.b } else { l.a };
            let fixed = matches!(other, Some(Who::City) | None);
            let (mine, point, n_car) = if car_a { (t.impulse, t.a.point, t.normal) } else { (Vec3::ZERO - t.impulse, t.b.point, t.back) };
            dv += if fixed { mine * inv_mass * f32::from_bits(0x3ecc_cccd) } else { mine * inv_mass };
            any |= t.impulse != Vec3::ZERO;
            let partner = match other {
                Some(Who::Obj(o)) => match objs.get(o).and_then(|x| x.veh.map(|v| (v.class, x.matrix[2]))) {
                    Some((6, back)) => Partner::Ambient { back },
                    _ => Partner::Other,
                },
                _ if fixed => Partner::Fixed,
                _ => Partner::Other,
            };
            touches.push(Touch { impulse: mine, normal: n_car, point, partner });
        }
        hits.push(if any { run(&own(dv), &touches) } else { HeadOn::default() });
    }
    hits
}

/// What the rule's result does to the car besides its spin (which the caller adds to the tick's
/// +384 before the integrate): each qualifying contact adds 0.1 of MaxDamage (F-15289), the
/// tyres melt (mcCarTireFric +24 = 1), the CG drops for a fixed partner, and the game is told of
/// the wipeout.
pub fn apply(st: &mut super::car::CarState, p: &super::car::CarParams, h: &HeadOn, melt: &mut f32, cg: &mut [f32; 2], model_offset_y: f32) {
    if !h.hit {
        return;
    }
    if h.cg_drop {
        *cg = [model_offset_y, 0.0];
    }
    for _ in 0..h.hits {
        st.add_damage(p, p.dmg[0] * 0.1);
    }
    if h.melt {
        *melt = 1.0;
    }
    st.head_on = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own(speed: f32, dv: f32) -> Own {
        // (the R8: WipeoutThreshold 19, MaxDamage 400, HeadOnSpinMult 2.5; driving along -z)
        Own { dv: Vec3::new(0.0, 0.0, dv), speed, vel: Vec3::new(0.0, 0.0, -speed), body_pos: Vec3::ZERO, back: Vec3::Z, wipeout_threshold: 19.0, max_damage: 400.0, spin_mult: 2.5, no_damage: false, agro: false }
    }

    /// Into the nose of a traffic car coming the other way: the wipeout, 40 points, a spin; not
    /// under 22.35 m/s, not with |dv| under 19, not into its tail (rear-ending), not a racer.
    #[test]
    fn head_on_traffic() {
        // the traffic car faces +z (its back axis -z); the normal into the own car points +z
        let nose = Touch { impulse: Vec3::new(0.0, 0.0, 9000.0), normal: Vec3::Z, point: Vec3::new(0.4, 0.3, -2.2), partner: Partner::Ambient { back: Vec3::NEG_Z } };
        let h = run(&own(30.0, 25.0), &[nose]);
        assert!(h.hit && h.melt && h.damage == 40.0 && !h.cg_drop && h.spins.len() == 1 && h.spins[0] != Vec3::ZERO, "{h:?}");
        assert!(!run(&own(22.35, 25.0), &[nose]).hit);
        assert!(!run(&own(30.0, 18.9), &[nose]).hit);
        let tail = Touch { partner: Partner::Ambient { back: Vec3::Z }, ..nose };
        assert!(!run(&own(30.0, 25.0), &[tail]).hit);
        assert!(!run(&own(30.0, 25.0), &[Touch { partner: Partner::Other, ..nose }]).hit);
        assert!(!run(&Own { agro: true, ..own(30.0, 25.0) }, &[nose]).hit);
        // a wall straight ahead: the fixed rule (within 25 degrees of the back axis)
        let wall = Touch { partner: Partner::Fixed, ..nose };
        let h = run(&own(30.0, 25.0), &[wall]);
        assert!(h.hit && h.cg_drop && !h.melt && h.spins.is_empty());
    }
}

#[cfg(test)]
mod offline {
    use super::*;

    /// F-43509 against the original run offline (tools/headon_reference.py): 6,000 runs of 0 .. 6 contacts - the damage
    /// and Agro gates, |dv| about WipeoutThreshold, speeds about 22.35 m/s, the own car as A or B, ambient cars (class 6
    /// or a player's car) facing the own car or not, fixed partners ahead or not, disabled contacts, lever-arm lanes at 0,
    /// the generator's state given. Every angular impulse (slot 38) with its lanes, every AddDamage with its value,
    /// F-69183 and F-46437, the CG copy, the melt and the generator's state after equal `run_with`'s.
    #[test]
    fn head_on_matches_the_original() {
        let text = include_str!("testdata/headon_ref.txt");
        let mut lines = text.lines();
        let hex = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let base: Vec<u32> = lines.next().unwrap().split_whitespace().map(hex).collect();
        let (c, d, dt, ch, st, tf, i, it, pair, ab, q, k, tbl) = (0x1040u32, 0x1100u32, 0x1140u32, 0x1200u32, 0x1380u32, 0x13C0u32, 0x1400u32, 0x1500u32, 0x1800u32, 0x1E00u32, 0x2000u32, 0x3000u32, 0x3480u32);
        let (mut n, mut spins, mut hits) = (0, 0, 0);
        for line in lines.filter(|l| !l.is_empty()) {
            let (pairs, out) = line.split_once(" -> ").unwrap();
            let (state, log) = out.split_once(" |").unwrap();
            let mut m = std::collections::HashMap::new();
            let mut ret = [0u32; 32];
            for p in base.chunks(2).chain(pairs.split_whitespace().map(hex).collect::<Vec<_>>().chunks(2)) {
                if p[0] >= 0xF000_0000 { ret[(p[0] - 0xF000_0000) as usize] = p[1]; } else { m.insert(p[0], p[1]); }
            }
            let g = |a: u32| *m.get(&a).unwrap_or(&0);
            let byte = |a: u32| g(0x4000_0000 | a) & 0xFF;
            let gf = |a: u32| f32::from_bits(g(a));
            let vec = |a: u32| Vec3::new(gf(a), gf(a + 4), gf(a + 8));
            let a = g(c + 72);
            let own = Own {
                dv: Vec3::new(f32::from_bits(ret[20]), f32::from_bits(ret[21]), f32::from_bits(ret[22])),
                speed: gf(ch + 224), vel: vec(k + 272), body_pos: vec(k + 256), back: vec(i + 48),
                wipeout_threshold: gf(dt + 36), max_damage: gf(dt + 24), spin_mult: gf(st + 284),
                no_damage: byte(d + 33) != 0,
                agro: ret[0] & 0xFF != 0 && a != 0 && g(a + 52) as i32 == 1 && g(a + 48) as i32 == 3,
            };
            let mut touches = Vec::new();
            for kk in 0..(g(it + 12) as i32).max(0) as u32 {
                let (pr, abk, qk) = (pair + 0xC0 * kk, ab + 0x20 * kk, q + 0x80 * kk);
                if byte(pr + 165) & 0x80 == 0 {
                    continue;
                }
                let own_a = g(abk) == i;
                let side = |v: Vec3| if own_a { v } else { v3(V4::ZERO.sub(v4(v))) };
                let cls = ret[1 + kk as usize] as i32;
                let back = vec(qk + 48);
                let partner = if cls == 6 {
                    Partner::Ambient { back }
                } else if cls == 8 && ret[8 + kk as usize] & 0xFF != 0 {
                    Partner::PlayerCar { back }
                } else if g(tbl + 8 * kk + 4) & 3 == 2 {
                    Partner::Fixed
                } else {
                    Partner::Other
                };
                touches.push(Touch { impulse: side(vec(pr + 112)), normal: side(vec(pr + 80)), point: vec(pr + if own_a { 48 } else { 64 }), partner });
            }
            let mut rng = [g(0x8286_0890), g(0x8286_0894)];
            let h = run_with(&own, &touches, &mut rng);
            let cg = if h.cg_drop { [g(ch + 96), g(ch + 100), g(ch + 104), g(ch + 108)] } else { [0; 4] };
            let melt = if h.melt { 0x3f80_0000 } else { g(tf + 24) };
            let want = format!("{:08x} {:08x} {:08x} {:08x} {:08x} {:04x}:{:04x} {:04x}:{:04x}", cg[0], cg[1], cg[2], cg[3], melt, rng[0] >> 16, rng[0] & 0xFFFF, rng[1] >> 16, rng[1] & 0xFFFF);
            assert_eq!(want, state, "{pairs}");
            let toks: Vec<&str> = log.split_whitespace().collect();
            let got_a: Vec<String> = toks.iter().filter(|t| t.starts_with("A,")).map(|t| t.to_string()).collect();
            let want_a: Vec<String> = h.spins.iter().map(|s| format!("A,0,{:x},{:x},{:x}", s.x.to_bits(), s.y.to_bits(), s.z.to_bits())).collect();
            assert_eq!(got_a, want_a, "{pairs}");
            let dmg = format!("D,{:x},{:x}", d, (own.max_damage * f32::from_bits(0x3dcc_cccd)).to_bits());
            assert_eq!(toks.iter().filter(|t| t.starts_with("D,")).map(|t| t.to_string()).collect::<Vec<_>>(), vec![dmg; h.hits as usize], "{pairs}");
            assert_eq!(toks.iter().filter(|t| t.starts_with("W,")).count(), h.hits as usize, "{pairs}");
            assert_eq!(toks.iter().filter(|t| **t == "S").count(), h.hits as usize, "{pairs}");
            n += 1;
            spins += h.spins.len();
            hits += h.hits as usize;
        }
        assert_eq!((n, spins, hits), (6000, 169, 426));
    }
}
