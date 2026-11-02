//! Presentation only. Keep wheel offsets in chassis space while interpolating.
use crate::physics::CarState;
use glam::{Quat, Vec3};

#[derive(Clone, Copy)]
pub struct Pose {
    pub pos: Vec3,
    pub rot: Quat,
    pub hubs: [Vec3; 4],
    pub spin: [f32; 4],
    pub steer: [f32; 4],
    pub lean: f32,
}

impl Pose {
    pub fn capture(state: &CarState, lean: f32) -> Self {
        let inv = state.rot.inverse();
        Self {
            pos: state.pos, rot: state.rot, lean,
            hubs: state.wheels.map(|w| inv * (w.world_pos - state.pos)),
            spin: state.wheels.map(|w| w.spin),
            steer: state.wheels.map(|w| w.steer),
        }
    }

    pub fn interpolate(self, next: Self, alpha: f32) -> Self {
        let alpha = alpha.clamp(0.0, 1.0);
        if alpha == 1.0 { return next; }
        if alpha == 0.0 { return self; }
        Self {
            pos: self.pos.lerp(next.pos, alpha),
            rot: self.rot.slerp(next.rot, alpha),
            hubs: std::array::from_fn(|i| self.hubs[i].lerp(next.hubs[i], alpha)),
            // Spin is an unwrapped accumulator, so fast rotations retain their direction.
            spin: std::array::from_fn(|i| self.spin[i] + (next.spin[i] - self.spin[i]) * alpha),
            steer: std::array::from_fn(|i| self.steer[i] + (next.steer[i] - self.steer[i]) * alpha),
            lean: self.lean + (next.lean - self.lean) * alpha,
        }
    }
}

#[derive(Default)]
pub struct History {
    pair: Option<(Pose, Pose)>,
}

impl History {
    pub fn reset(&mut self) { self.pair = None; }

    pub fn begin(&mut self, state: &CarState, lean: f32) {
        let now = Pose::capture(state, lean);
        self.pair = Some((now, now));
    }

    pub fn finish(&mut self, state: &CarState, lean: f32) {
        let now = Pose::capture(state, lean);
        let previous = self.pair.map_or(now, |p| p.0);
        // Placement, road resets and cutscene jumps must not sweep through the city.
        let limit = 2.0 + state.vel.length() * crate::fixed_step::DT * 2.0;
        let previous = if previous.pos.distance(now.pos) > limit { now } else { previous };
        self.pair = Some((previous, now));
    }

    /// Each wheel's turn per second over the last fixed tick (0 before the second tick, and after a jump).
    pub fn spin_rate(&self) -> [f32; 4] {
        self.pair.map_or([0.0; 4], |(a, b)| std::array::from_fn(|i| (b.spin[i] - a.spin[i]) / crate::fixed_step::DT))
    }

    pub fn sample(&self, state: &CarState, lean: f32, alpha: f32) -> Pose {
        match self.pair {
            Some((a, b)) if b.pos == state.pos && b.rot == state.rot => a.interpolate(b, alpha),
            // An external pose/reset after the last tick is immediately visible.
            _ => Pose::capture(state, lean),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_body_keeps_all_four_axles_attached() {
        let hubs = [Vec3::new(-0.8, -0.4, -1.4), Vec3::new(0.8, -0.4, -1.4), Vec3::new(-0.8, -0.4, 1.4), Vec3::new(0.8, -0.4, 1.4)];
        let mut a = CarState::new(Vec3::new(1700.0, 3.0, -1000.0), 0.0);
        for (w, hub) in a.wheels.iter_mut().zip(hubs) { w.world_pos = a.pos + a.rot * hub; }
        let first = Pose::capture(&a, 0.0);
        a.pos += Vec3::new(1.5, 0.3, -1.5);
        a.rot = Quat::from_euler(glam::EulerRot::YXZ, 1.2, 0.2, -0.4);
        for (w, hub) in a.wheels.iter_mut().zip(hubs) { w.world_pos = a.pos + a.rot * hub; }
        let second = Pose::capture(&a, 0.3);
        for step in 0..=100 {
            let p = first.interpolate(second, step as f32 / 100.0);
            for (hub, expected) in p.hubs.into_iter().zip(hubs) {
                let world = p.pos + p.rot * hub;
                assert!((p.rot.inverse() * (world - p.pos) - expected).length() < 0.0002);
            }
        }
    }

    #[test]
    fn reset_and_external_pose_snap_without_changing_physics() {
        let mut a = CarState::new(Vec3::ZERO, 0.0);
        let mut h = History::default();
        h.begin(&a, 0.0);
        a.pos = Vec3::splat(100.0);
        h.finish(&a, 0.0);
        assert_eq!(h.sample(&a, 0.0, 0.0).pos, a.pos);
        a.pos += Vec3::Y;
        assert_eq!(h.sample(&a, 0.0, 0.1).pos, a.pos);
        h.reset();
        assert_eq!(h.sample(&a, 0.0, 0.0).pos, a.pos);
    }

    #[test]
    fn rescue_relocation_carries_hubs_and_retains_contact_samples() {
        let mut state = CarState::new(Vec3::new(1100.0, -96.0, 120.0), 0.8);
        let hubs = [Vec3::new(-0.8, -0.4, -1.4), Vec3::new(0.8, -0.4, -1.4), Vec3::new(-0.8, -0.4, 1.4), Vec3::new(0.8, -0.4, 1.4)];
        for (i, w) in state.wheels.iter_mut().enumerate() {
            w.world_pos = state.pos + state.rot * hubs[i];
            w.contact = Vec3::new(i as f32, -98.0, 100.0);
        }
        let contacts = state.wheels.map(|w| w.contact);
        let pos = Vec3::new(1079.0, 3.0, 119.0);
        let rot = Quat::from_rotation_y(-1.2);
        state.relocate(pos, rot);
        assert_eq!(state.pos, pos);
        assert_eq!(state.rot, rot);
        assert_eq!(state.wheels.map(|w| w.contact), contacts);
        let pose = Pose::capture(&state, 0.0);
        for (actual, expected) in pose.hubs.into_iter().zip(hubs) {
            assert!((actual - expected).length() < 0.0002);
        }
    }

    #[test]
    fn fallback_physics_keeps_hubs_on_axles_after_its_last_substep() {
        let params = crate::physics::CarParams::default();
        let mut world = crate::physics::WorldCollider::default();
        world.sectors.insert(0, crate::physics::SectorMesh::build(vec![
            [Vec3::new(-512.0, 0.0, -512.0), Vec3::new(-512.0, 0.0, 512.0), Vec3::new(512.0, 0.0, -512.0)],
            [Vec3::new(512.0, 0.0, -512.0), Vec3::new(-512.0, 0.0, 512.0), Vec3::new(512.0, 0.0, 512.0)],
        ]));
        for (speed, height) in [(0.0, 0.02), (30.0, 0.02), (90.0, 10.0)] {
            let mut state = CarState::new(Vec3::new(0.0, height, 0.0), 0.0);
            state.vel = state.forward() * speed;
            if height > 1.0 { state.ang_vel = Vec3::new(0.5, 0.8, 0.2); }
            for frame in 0..120 {
                let input = if frame < 60 { crate::physics::CarInput { throttle: 1.0, ..Default::default() } }
                    else { crate::physics::CarInput { brake: 1.0, steer: 0.25, ..Default::default() } };
                state.step(&params, input, &world, crate::fixed_step::DT);
                for i in 0..4 {
                    let hub = state.rot.inverse() * (state.wheels[i].world_pos - state.pos);
                    assert!((hub.x - params.wheel_pos[i].x).abs() < 0.001 && (hub.z - params.wheel_pos[i].z).abs() < 0.001,
                        "fallback speed {speed}, frame {frame}, wheel {i}: {hub:?}");
                }
            }
        }
    }
}
