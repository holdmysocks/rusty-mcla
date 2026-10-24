//! The head-on wipeout (sub_82333CE8, mcCarInstance vtable slot 25, after mcCarDamage's contact call):
//! a car that drives into the nose of an ambient (traffic or parked) car, or straight into something
//! fixed, above 22.35 m/s with a run of contacts that changed its velocity by at least its
//! WipeoutThreshold is wiped out. Rule: docs/reference/cameras/carhit_wipeout_rule.md.

use glam::Vec3;

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
    /// the run's velocity change (sub_82567A08: the impulses over the mass, 0.4 of those against
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeadOn {
    /// the angular impulse for the collider (+384, phCollider slot 38), summed over the contacts
    pub spin: Vec3,
    /// damage points added (MaxDamage x 0.1 per qualifying contact, sub_82343178)
    pub damage: f32,
    /// a contact qualified: the wipeout (sub_823429B0), the slow motion (sub_82332A00); from an
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

fn draw2() -> (u32, u32) {
    let mut s = RNG.lock().unwrap_or_else(|e| e.into_inner());
    let a = 0x5CDC_FAA7u64;
    let t1 = s[0] as u64 * a + s[1] as u64;
    let x1 = (t1 as u32) & 0x7F_FFFF;
    let t2 = (t1 as u32) as u64 * a + (t1 >> 32);
    let x2 = (t2 as u32) & 0x7F_FFFF;
    *s = [t2 as u32, (t2 >> 32) as u32];
    (x1, x2)
}

/// sgn(v) |v|^0.33 per axis, the power in double (sub_823DC480).
fn soft(r: Vec3) -> Vec3 {
    let f = |v: f32| {
        let m = (v.abs() as f64).powf(0.330_000_013_113_021_85) as f32;
        if v > 0.0 {
            m
        } else if v < 0.0 {
            -m
        } else {
            0.0
        }
    };
    Vec3::new(f(r.x), f(r.y), f(r.z))
}

/// One call of the rule for a run of contacts (the own car's run as A or as B).
pub fn run(own: &Own, touches: &[Touch]) -> HeadOn {
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
    if own.dv.length() < own.wipeout_threshold {
        return out;
    }
    let cos25 = (0.436_332_315_206_527_7f64).cos() as f32;
    let cos45 = (0.785_398_185_253_143_3f64).cos() as f32;
    for t in touches {
        match t.partner {
            Partner::Ambient { back } | Partner::PlayerCar { back } => {
                if !(own.speed > 22.35) {
                    continue;
                }
                // the own car in front of the other's nose, moving against its facing
                let u = own.vel - own.back;
                let d = u.normalize_or_zero();
                if !(back.dot(t.normal) < -0.7) || !(d.dot(back) > cos45) {
                    continue;
                }
                let (x1, x2) = draw2();
                let f7 = x1 as f32 * f32::from_bits(0x3480_0000) - 1.0;
                let f6 = x2 as f32 * f32::from_bits(0x3480_0000) - 1.0;
                let ky = f7 * 0.25 + 1.0;
                let kxz = (f6 + 1.0) * 0.15;
                let w = soft(t.point - own.body_pos).cross(t.impulse);
                out.spin += Vec3::new(w.x * kxz, w.y * ky, w.z * kxz) * own.spin_mult;
                out.damage += own.max_damage * 0.1;
                out.hits += 1;
                out.hit = true;
                out.melt = true;
            }
            Partner::Fixed => {
                if !(own.back.dot(t.normal) > cos25) || !(own.speed > 22.35) {
                    continue;
                }
                out.cg_drop = true;
                out.damage += own.max_damage * 0.1;
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
    let a_rows: Vec<usize> = (0..out.links.len()).filter(|&i| out.links[i].a == Some(Who::Car)).collect();
    let b_rows: Vec<usize> = out.by_b.iter().copied().filter(|&i| out.links[i].b == Some(Who::Car)).collect();
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
/// +384 before the integrate): each qualifying contact adds 0.1 of MaxDamage (sub_82343178), the
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
        assert!(h.hit && h.melt && h.damage == 40.0 && !h.cg_drop && h.spin != Vec3::ZERO, "{h:?}");
        assert!(!run(&own(22.35, 25.0), &[nose]).hit);
        assert!(!run(&own(30.0, 18.9), &[nose]).hit);
        let tail = Touch { partner: Partner::Ambient { back: Vec3::Z }, ..nose };
        assert!(!run(&own(30.0, 25.0), &[tail]).hit);
        assert!(!run(&own(30.0, 25.0), &[Touch { partner: Partner::Other, ..nose }]).hit);
        assert!(!run(&Own { agro: true, ..own(30.0, 25.0) }, &[nose]).hit);
        // a wall straight ahead: the fixed rule (within 25 degrees of the back axis)
        let wall = Touch { partner: Partner::Fixed, ..nose };
        let h = run(&own(30.0, 25.0), &[wall]);
        assert!(h.hit && h.cg_drop && !h.melt && h.spin == Vec3::ZERO);
    }
}
