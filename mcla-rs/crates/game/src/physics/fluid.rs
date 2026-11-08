//! Deep water: mcCarFluid (chassis +12) and the car's fluid contact callback.
//!
//! The rule (docs/reference/vehicle/water_rule.md, recorded in the original in work/physics2/rec wa.0):
//! every tick the callback F-58580 walks the car's contacts. A material deeper than 0.09
//! (`water_deep` 0.1, `death` 1.0) costs a fifth of MaxDamage a second; `death` (depth 1 or more)
//! costs MaxDamage + 1 at once, takes the contact out of the solve and gives the water's level, the
//! contact's world point; any other game material under 0.09 says the car is out of the water. In
//! the water a lattice of 4 x 4 x 4 points over the hull's box is held up by the buoyancy and dragged
//! towards the current (F-87118).

use super::collision::{surface_depth, Surface};
use glam::Vec3;
use mcla_formats::rage::simtune::FluidTune;

/// The level a dry contact sets (0x82002C94): "not in the water".
pub const DRY: f32 = -1000.0;
/// The callback's fixed step (0x827D7508) and the fluid update's.
const STEP: f32 = f32::from_bits(0x3c88_8889);

#[derive(Clone, Debug)]
pub struct Fluid {
    /// in the water (+12 bit 0)
    pub inside: bool,
    /// the water's level (+16)
    pub level: f32,
    /// the current (+32)
    pub flow: Vec3,
    /// the lattice's points in the car's frame (+48, 64 of them)
    pub points: Vec<Vec3>,
    /// +1072, +1076
    pub buoyancy: f32,
    pub damp: f32,
    /// +1088: how long before it may be entered again
    pub timer: f32,
    pub tune: FluidTune,
}

impl Default for Fluid {
    fn default() -> Fluid {
        // (the base's constructor, F-59913's caller F-79372 then the init)
        Fluid { inside: false, level: 0.0, flow: Vec3::ZERO, points: Vec::new(), buoyancy: 0.5, damp: 0.1, timer: 0.0, tune: FluidTune::default() }
    }
}

impl Fluid {
    /// F-79372 -> F-59913 and the init F-62399: the lattice over the hull's box (min, max: the
    /// composite's first part, +32 / +16), the damping and the buoyancy the car starts with.
    pub fn new(tune: &FluidTune, min: Vec3, max: Vec3) -> Fluid {
        let third = f32::from_bits(0x3eaa_aaab);
        let along = [0.0, third, f32::from_bits(0x3f2a_aaab), 1.0];
        let d = max - min;
        let mut points = Vec::with_capacity(64);
        for i in 0..4 {
            for j in 0..4 {
                for z in along {
                    points.push(Vec3::new((d.x * i as f32).mul_add(third, min.x), (d.y * j as f32).mul_add(third, min.y), d.z.mul_add(z, min.z)));
                }
            }
        }
        let mut f = Fluid { points, damp: tune.damp, tune: tune.clone(), ..Fluid::default() };
        f.buoyancy = f.start();
        f
    }

    /// The buoyancy on entering (F-62399, F-77072): BuoyancyMin when it rises, else BuoyancyMax.
    fn start(&self) -> f32 {
        if self.tune.sink_rate > 0.0 { self.tune.buoyancy_min } else { self.tune.buoyancy_max }
    }

    /// vt+16, F-77072: in, unless the timer still runs.
    pub fn enter(&mut self) {
        if self.timer > 0.0 {
            return;
        }
        self.inside = true;
        self.buoyancy = self.start();
    }

    /// The update, F-84852, once a tick: the timer, then in the water the lattice's forces
    /// (F-87118) and the buoyancy's change. `rows`: the car's matrix (chassis +240..+288);
    /// `com`: its centre of mass in the car's frame (+112), what the torques turn about once
    /// through that matrix; `place`: the chassis' copy of the body's place (+352), what the
    /// points' velocities are taken about; the body's velocity and turning. Each point under
    /// the level adds its force and torque into `force` / `torque` (the body's sums +336 / +352,
    /// through its AddForce / AddTorque, F-17506 / F-93816) one point after another.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, rows: &[Vec3; 4], com: Vec3, place: Vec3, vel: Vec3, ang_vel: Vec3, mass: f32, force: &mut Vec3, torque: &mut Vec3) {
        if self.timer > 0.0 {
            self.timer -= STEP;
        }
        if !(self.inside && self.buoyancy > 0.0) {
            return;
        }
        // (a point (x, y, z, 1) against the matrix's columns, vmsum4fp: (x r0 + y r1) + (z r2 + r3))
        let at = |p: Vec3| {
            let c = |k: usize| (p.x * rows[0][k] + p.y * rows[1][k]) + (p.z * rows[2][k] + rows[3][k]);
            Vec3::new(c(0), c(1), c(2))
        };
        let centre = at(com);
        let up = Vec3::new(0.0, self.buoyancy * mass, 0.0);
        for &p in &self.points {
            let w = at(p);
            if w.y > self.level {
                continue;
            }
            let v = cross(ang_vel, w - place) + vel;
            let f = up - ((v - self.flow) * self.damp) * mass;
            *force += f;
            *torque += cross(w - centre, f);
        }
        let b = self.tune.sink_rate.mul_add(STEP, self.buoyancy);
        self.buoyancy = b.max(self.tune.buoyancy_min).min(self.tune.buoyancy_max);
    }
}

/// a x b as F-87118 forms it: lane x -(b.y a.z - b.z a.y) and so on (vnmsubfp as the generated
/// code has it, the products and the subtraction each rounded, the sign flipped).
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(-(b.y * a.z - b.z * a.y), -(b.z * a.x - b.x * a.z), -(b.x * a.y - b.y * a.x))
}

/// What F-58580 makes of the tick's contacts.
#[derive(Clone, Copy, Debug)]
pub struct Scan {
    pub level: f32,
    pub damage: f32,
}

impl Scan {
    /// The walk's start: the level the car is in, or `DRY`.
    pub fn new(fluid: &Fluid) -> Scan {
        Scan { level: if fluid.inside { fluid.level } else { DRY }, damage: 0.0 }
    }

    /// One contact on `surface` whose world point is `point`; `max_damage` the damage tune's
    /// MaxDamage. Returns false where the contact is taken out of the solve (`death`).
    pub fn contact(&mut self, surface: Surface, point: Vec3, max_damage: f32) -> bool {
        // (the material record's +4: 2 for every game material, 0 for the table's own default)
        if surface == Surface::Default {
            return true;
        }
        let depth = surface_depth(surface);
        let deep = f32::from_bits(0x3db8_51ec);
        if depth > deep {
            self.damage = self.damage.max((max_damage * 0.2) * STEP);
            if depth >= 1.0 {
                self.damage = self.damage.max(max_damage + 1.0);
                self.level = self.level.max(point.y);
                return false;
            }
        } else if depth < deep {
            self.level = DRY;
        }
        true
    }

    /// The walk's end: in or out of the water. `drivable`: the chassis' vt+48 (F-71660: its
    /// flags +592 are 0 or 0x10), without which a car does not leave.
    pub fn finish(&self, fluid: &mut Fluid, drivable: bool) {
        if self.level == DRY {
            if fluid.inside && drivable {
                // (vt+20, F-29560: the bit alone - the timer is not set)
                fluid.inside = false;
            }
        } else {
            if !fluid.inside {
                fluid.enter();
            }
            fluid.level = self.level;
            // (F-98483: the direction (1, 0, 0) times CurrentScale)
            fluid.flow = Vec3::X * fluid.tune.current_scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tune() -> FluidTune {
        FluidTune { damp: 0.15, current_scale: 2.0, buoyancy_max: 0.2, buoyancy_min: 0.1, sink_rate: -0.1 }
    }

    #[test]
    fn the_lattice_spans_the_hull_box() {
        let f = Fluid::new(&tune(), Vec3::new(-0.955, 0.311, -2.286), Vec3::new(0.955, 1.3, 2.4));
        assert_eq!(f.points.len(), 64);
        assert_eq!(f.points[0], Vec3::new(-0.955, 0.311, -2.286));
        assert!((f.points[63] - Vec3::new(0.955, 1.3, 2.4)).length() < 1e-5);
        assert!((f.buoyancy - 0.2).abs() < 1e-7 && (f.damp - 0.15).abs() < 1e-7);
    }

    #[test]
    fn buoyancy_sinks_towards_its_least() {
        let mut f = Fluid::new(&tune(), Vec3::splat(-1.0), Vec3::splat(1.0));
        f.inside = true;
        f.level = 10.0;
        let rows = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO];
        let (mut force, mut torque) = (Vec3::ZERO, Vec3::ZERO);
        f.update(&rows, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 1000.0, &mut force, &mut torque);
        // (all 64 points under the level: 64 x 0.2 x the mass, no turn about the box's centre)
        assert!((force.y - 64.0 * 0.2 * 1000.0).abs() < 1.0, "{force:?}");
        assert!(torque.length() < 1e-2, "{torque:?}");
        for _ in 0..120 {
            f.update(&rows, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 1000.0, &mut force, &mut torque);
        }
        assert!((f.buoyancy - 0.1).abs() < 1e-7);
    }

    /// F-84852 with F-87118 and the body's AddForce / AddTorque run offline
    /// (tools/fluid_reference.py): the sums, the buoyancy and the timer, every word.
    #[test]
    fn update_matches_the_original() {
        let w = |s: &str| u32::from_str_radix(&s.replace('_', ""), 16).unwrap();
        let f = |s: &str| f32::from_bits(w(s));
        let v = |a: &[&str]| Vec3::new(f(a[0]), f(a[1]), f(a[2]));
        let mut n = 0;
        for line in include_str!("../testdata/fluid_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let a: Vec<&str> = i.split_whitespace().collect();
            let o: Vec<u32> = o.split_whitespace().map(w).collect();
            let t = FluidTune { damp: 0.0, current_scale: 0.0, buoyancy_max: f(a[9]), buoyancy_min: f(a[8]), sink_rate: f(a[10]) };
            let points = (0..64).map(|p| v(&a[42 + 3 * p..])).collect();
            let mut fl = Fluid { inside: w(a[0]) & 1 != 0, level: f(a[4]), flow: v(&a[5..]), points, buoyancy: f(a[1]), damp: f(a[2]), timer: f(a[3]), tune: t };
            let rows = [v(&a[24..]), v(&a[27..]), v(&a[30..]), v(&a[33..])];
            let (mut force, mut torque) = (v(&a[36..]), v(&a[39..]));
            fl.update(&rows, v(&a[21..]), v(&a[18..]), v(&a[12..]), v(&a[15..]), f(a[11]), &mut force, &mut torque);
            let got = [force.x, force.y, force.z, torque.x, torque.y, torque.z, fl.buoyancy, fl.timer].map(f32::to_bits);
            assert_eq!(got.to_vec(), o, "{line:.120}");
            n += 1;
        }
        assert_eq!(n, 1500);
    }
}
