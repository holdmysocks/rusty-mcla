//! The particle effects the props' events start (`work/map2/colfx/prop_hit_fx_rule.md`): the impact
//! reports and the deflection pass (an effect at a world point, unrotated, one shot), the break
//! events (an effect that follows a matrix of the part that stays), and the hydrant's water, whose
//! update functor (`sub_827364D8`) hands it the car's underside to bounce off and the "collision"
//! evolution each frame before the particle step.

use crate::particles::{Particles, Poly};
use crate::physics::loose::{xf, FxAt, FxPlay, LooseState, LooseWorld};
use glam::{Quat, Vec3};

/// An effect that is asked for again every frame (its matrix moves, or its evolutions change).
struct Active {
    key: u64,
    effect: u32,
    /// what it follows (none, a fixed matrix: `fixed`)
    at: FxAt,
    fixed: Option<[Vec3; 4]>,
    /// the last place it was put at (the matrix is gone when the object is)
    last: ([Vec3; 4], Vec3),
    /// the effect has the evolution "collision" (only `water_hydrant`): the update functor `sub_827364D8`
    hydrant: bool,
    /// it has been asked for once (then it is alive until it ends by itself: never asked for again after that)
    asked: bool,
}

pub struct PropFx {
    active: Vec<Active>,
    started: u64,
    hydrant: u32,
    log: bool,
}

impl PropFx {
    pub fn new() -> PropFx {
        PropFx { active: Vec::new(), started: 0, hydrant: mcla_formats::hash::joaat("water_hydrant"), log: std::env::var_os("MCLA_PROPFX_LOG").is_some() }
    }

    fn rot_of(m: &[Vec3; 4]) -> Quat {
        Quat::from_mat3(&glam::Mat3::from_cols(m[0].normalize_or(Vec3::X), m[1].normalize_or(Vec3::Y), m[2].normalize_or(Vec3::Z))).normalize()
    }

    /// Start the events the physics and the props queued this tick. Effects are never refused here but by the
    /// effect list itself (the original's pool of 250 instances; the port's own cap is `particles::MAX_EFFECTS`).
    pub fn start(&mut self, plays: Vec<FxPlay>, level: &LooseState, world: Option<&LooseWorld>, fx: &mut Particles, cam: Vec3) {
        for p in plays {
            let effect = mcla_formats::hash::joaat(&p.effect);
            if !fx.has_effect(effect) {
                if self.log {
                    log::info!("propfx: effect {} is not in the effect list", p.effect);
                }
                continue;
            }
            match p.at {
                FxAt::World(at) => {
                    // (identity matrix, the position parameter: at the point, unrotated; velocity as the caller gave)
                    let ok = fx.burst_evo(effect, at, Quat::IDENTITY, p.vel, at.distance(cam), &[]);
                    if self.log {
                        log::info!("propfx: {} at {:.2?} vel {:.2?} played {ok}", p.effect, at, p.vel);
                    }
                }
                FxAt::Follow { obj, part, offset } => {
                    let Some(m) = world.and_then(|w| level.fx_matrix(w, obj, part)) else { continue };
                    self.started += 1;
                    let key = 0x5052_4658_0000_0000 | self.started;
                    let a = Active { key, effect, at: p.at, fixed: None, last: (m, offset), hydrant: effect == self.hydrant, asked: false };
                    if self.log {
                        log::info!("propfx: {} follows object {obj} part {part:?} at {:.2?}", p.effect, xf(&m, offset));
                    }
                    self.active.push(a);
                }
            }
        }
    }

    pub fn has_fixed(&self) -> bool {
        self.active.iter().any(|a| a.fixed.is_some())
    }

    /// Start an effect on a fixed matrix (the test hook).
    pub fn start_fixed(&mut self, effect: &str, at: Vec3) {
        self.started += 1;
        let m = [Vec3::X, Vec3::Y, Vec3::Z, at];
        self.active.push(Active { key: 0x5052_4658_0000_0000 | self.started, effect: mcla_formats::hash::joaat(effect), at: FxAt::World(at), fixed: Some(m), last: (m, Vec3::ZERO), hydrant: effect == "water_hydrant", asked: false });
    }

    /// Every frame before the particles step: the followed effects to their matrices, the hydrant's water its
    /// collision polygon and evolution. `car`: the player's vehicle's place and its underside quad.
    pub fn step(&mut self, level: &LooseState, world: Option<&LooseWorld>, fx: &mut Particles, cam: Vec3, car: Option<(Vec3, [Vec3; 4])>) {
        let mut i = 0;
        while i < self.active.len() {
            let a = &mut self.active[i];
            if a.asked && !fx.is_alive(a.key) {
                self.active.swap_remove(i);
                continue;
            }
            if let (None, FxAt::Follow { obj, part, offset }) = (a.fixed, a.at) {
                if let Some(m) = world.and_then(|w| level.fx_matrix(w, obj, part)) {
                    a.last = (m, offset);
                }
            }
            let (m, off) = a.last;
            let pos = xf(&m, off);
            let dist = pos.distance(cam);
            let mut coll = 0.0f32;
            let mut poly = None;
            if a.hydrant {
                // (`sub_827364D8`: the quad only within 10 m of the car, 3D; the collision evolution 1 when the segment from
                // 0.5 m below the effect to 2 m above it crosses the quad)
                if let Some((car_pos, q)) = car.filter(|(c, _)| (*c - pos).length_squared() <= 100.0) {
                    let _ = car_pos;
                    let (s0, s1) = (pos - Vec3::Y * 0.5, pos + Vec3::Y * 2.0);
                    if seg_tri(s0, s1, q[0], q[1], q[2]) || seg_tri(s0, s1, q[0], q[2], q[3]) {
                        coll = 1.0;
                    }
                    poly = Some(Poly { v: q, n: Vec3::NEG_Y });
                }
            }
            let evo: &[(&str, f32)] = if a.hydrant { &[("collision", coll)] } else { &[] };
            let alive = fx.play_oneshot_evo(a.key, a.effect, pos, Self::rot_of(&m), Vec3::ZERO, dist, evo);
            fx.set_collision(a.key, poly);
            a.asked = true;
            // (the rule cannot be played: dropped)
            if !alive {
                self.active.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
}

/// Does the segment a -> b cross the triangle (v0, v1, v2) (`sub_8261A3F0`; either side)?
fn seg_tri(a: Vec3, b: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> bool {
    let d = b - a;
    let (e1, e2) = (v1 - v0, v2 - v0);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 7.63e-6 {
        return false;
    }
    let inv = 1.0 / det;
    let t0 = a - v0;
    let u = t0.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = t0.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    (0.0..=1.0).contains(&(e2.dot(q) * inv))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_through_the_underside() {
        let q = [Vec3::new(1.0, 0.2, -2.0), Vec3::new(1.0, 0.2, 2.0), Vec3::new(-1.0, 0.2, 2.0), Vec3::new(-1.0, 0.2, -2.0)];
        let hit = |p: Vec3| seg_tri(p - Vec3::Y * 0.5, p + Vec3::Y * 2.0, q[0], q[1], q[2]) || seg_tri(p - Vec3::Y * 0.5, p + Vec3::Y * 2.0, q[0], q[2], q[3]);
        assert!(hit(Vec3::ZERO) && hit(Vec3::new(0.9, 0.0, 1.9)));
        // (beside the quad; the segment ends 0.2 m short of it below; starts above it)
        assert!(!hit(Vec3::new(1.1, 0.0, 0.0)) && !hit(Vec3::new(0.0, -2.0, 0.0)) && !hit(Vec3::new(0.0, 0.8, 0.0)) && hit(Vec3::new(0.0, 0.69, 0.0)));
    }
}
