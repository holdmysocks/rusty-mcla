//! The bike rider's way off the seat when the motorcycle wipes out (GitHub #39, docs/reference/vehicle/bike_rider_ragdoll.md
//! sections 2 - 3, ragdoll_setup_decode.md section 4): the anim context's impact bits (F-71604, from the frame's body-frame
//! velocity F-28290 builds), the impact switch of the rider graph (F-71745) and the eject clip each branch plays (F-15344,
//! F-27063, F-74521, F-61419) with the velocity it leaves for GoRagdoll (seq +144).

use glam::Vec3;

/// The rider tune block (ctx +16, ctor F-40144): the running game keeps the ctor's values.
#[derive(Clone, Copy, Debug)]
pub struct ImpactTune {
    /// +136 ImpactHardThresholdAccel
    pub hard: f32,
    /// +140 ImpactSoftThresholdAccel
    pub soft: f32,
    /// +144 ImpactSideThresholdAccel
    pub side: f32,
    /// +148 / +152: d = +148 + +152 |v|
    pub base: f32,
    pub per_speed: f32,
}

impl Default for ImpactTune {
    fn default() -> Self {
        ImpactTune { hard: 100.0, soft: 25.0, side: 25.0, base: 0.0, per_speed: 0.05 }
    }
}

/// ctx +8 bit 0 ImpactFromLeft, bit 1 ImpactFromRight, bit 2 the soft, bit 3 the hard forward blow
pub const FROM_LEFT: u32 = 1;
pub const FROM_RIGHT: u32 = 2;
pub const SOFT: u32 = 4;
pub const HARD: u32 = 8;

/// The part of the anim context F-71604 keeps from frame to frame: the body-frame velocity (+100 x, +104 y, +96 z),
/// and the four impact bits it makes from its change.
#[derive(Clone, Copy, Debug, Default)]
pub struct ImpactCtx {
    /// +100, +104, +96 (x, y, z)
    pub vel: [f32; 3],
    /// +112, +116, +108: (new - old) / dt
    pub acc: [f32; 3],
    pub bits: u32,
}

/// F-28290's vector for F-71604: the bike collider's velocity (+272) in its rows (+208 / +224 / +240): x = row 0 . v,
/// y = row 1 . v, z = -(row 2 . v), each summed z, y, x as the code does; w = chassis +136 (the sim's throttle).
pub fn body_velocity(rows: [Vec3; 3], v: Vec3, throttle: f32) -> [f32; 4] {
    use crate::physics::exact::fma;
    let dot = |r: Vec3| fma(r.x, v.x, fma(r.z, v.z, r.y * v.y));
    let z = -fma(rows[2].x, v.x, fma(rows[2].z, v.z, rows[2].y * v.y));
    [dot(rows[0]), dot(rows[1]), z, throttle]
}

impl ImpactCtx {
    /// F-71604 once a frame with the frame's dt (<= 0: 1/30). The bits: x acceleration / d above +144 -> FROM_LEFT,
    /// below -+144 -> FROM_RIGHT; z acceleration / d below -+140 -> SOFT, below -+136 -> HARD; d = +148 + +152 |v|
    /// held in [1, 100], |v| over all four lanes of the vector (the throttle with them, vmsum4fp).
    /// (the context's first values are its ctor's, not read; they matter only to a wipeout in the first frame)
    pub fn update(&mut self, v: [f32; 4], dt: f32, t: &ImpactTune) {
        use crate::physics::exact::fma;
        let dt = if dt > 0.0 { dt } else { f32::from_bits(0x3d08_8889) };
        let inv = 1.0 / dt;
        let old = self.vel;
        self.vel = [v[0], v[1], v[2]];
        // (+108 = z, +112 = x, +116 = y)
        let az = (self.vel[2] - old[2]) * inv;
        let ax = (self.vel[0] - old[0]) * inv;
        let ay = (self.vel[1] - old[1]) * inv;
        self.acc = [ax, ay, az];
        use crate::physics::artic::V4;
        let v4 = V4::new(v[0], v[1], v[2], v[3]);
        let len = v4.dot4(v4).x().sqrt();
        let clamp = |d: f32| {
            let d = if d - 1.0 >= 0.0 { d } else { 1.0 };
            if d - 100.0 >= 0.0 { 100.0 } else { d }
        };
        let dx = clamp(fma(t.per_speed, len, t.base));
        let dz = clamp(fma(t.per_speed, len, t.base));
        let (x, z) = (ax / dx, az / dz);
        let mut b = self.bits & !0xF;
        if x > t.side {
            b |= FROM_LEFT;
        }
        if x < -t.side {
            b |= FROM_RIGHT;
        }
        if z < -t.soft {
            b |= SOFT;
        }
        if z < -t.hard {
            b |= HARD;
        }
        self.bits = b;
    }
}

/// F-28290's NoseUp (ctx +8 bit 12): the bike collider's row 2 y (+244) under -sin(20 degrees) (the double at the
/// image's 0.34906587, through the guest's sin (exact::guest_sin) and frsp).
pub fn nose_up(row2_y: f32) -> bool {
    let s = crate::physics::exact::guest_sin(f64::from_bits(0x3FD6_5718_6000_0000)) as f32;
    row2_y < -s
}

/// The impact switch's way out (F-68636, F-87237, F-18011, F-29820 in that order; Submerged skips them all).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eject {
    /// thrown from the right: "impact eject left" (F-15344, drv_eject_L)
    Left,
    /// "impact eject right" (F-27063, drv_eject_R)
    Right,
    /// nose up: "impact fall backward" (F-74521, drv_eject_Bk)
    Back,
    /// "impact flip forward" (F-61419, drv_eject_over)
    Over,
}

pub fn choose(bits: u32, nose_up: bool, submerged: bool) -> Option<Eject> {
    if submerged {
        return None;
    }
    Some(if bits & FROM_RIGHT != 0 {
        Eject::Left
    } else if bits & FROM_LEFT != 0 {
        Eject::Right
    } else if nose_up {
        Eject::Back
    } else {
        Eject::Over
    })
}

impl Eject {
    /// the clip's name after the bike's pack prefix ("kaw_ninja_zx14_06_" + this)
    pub fn clip(self) -> &'static str {
        match self {
            Eject::Left => "drv_eject_L",
            Eject::Right => "drv_eject_R",
            Eject::Back => "drv_eject_Bk",
            Eject::Over => "drv_eject_over",
        }
    }

    /// seq +144, the velocity GoRagdoll gives the whole body (plus (0, 1, 0)): 0.5 x chassis +800 (the collider's
    /// velocity kept at the wipeout's start, F-69183) for left / right / back; for the flip 0.5 x u with u = 5 x the
    /// collider's velocity now, or chassis +800 while DamageoutTiming (ctx 0x40000) stands.
    pub fn launch(self, kept: Vec3, now: Vec3, damageout_timing: bool) -> Vec3 {
        match self {
            Eject::Over => (if damageout_timing { kept } else { now * 5.0 }) * 0.5,
            _ => kept * 0.5,
        }
    }

    /// the clip's rate (F-53423 -> anim +20): 0.5; the flip: max(-0.1 / 15 x u_local.z, 0.1) with u in the bike
    /// collider's rows (u itself when the bike is not active in the level)
    pub fn rate(self, u_local_z: f32) -> f32 {
        match self {
            Eject::Over => {
                let f = u_local_z * f32::from_bits(0xbd88_8889) * 0.1;
                if f - 0.1 >= 0.0 { f } else { 0.1 }
            }
            _ => 0.5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_blow_ejects_to_the_other_side() {
        let t = ImpactTune::default();
        let mut c = ImpactCtx::default();
        c.update([0.0, 0.0, 20.0, 1.0], 1.0 / 30.0, &t);
        // 1 m/s sideways in a frame at 20 m/s: 30 m/s^2 over d = 1 -> above 25
        c.update([-1.0, 0.0, 20.0, 1.0], 1.0 / 30.0, &t);
        assert_eq!(c.bits & 0xF, FROM_RIGHT);
        assert_eq!(choose(c.bits, false, false), Some(Eject::Left));
        assert_eq!(choose(c.bits, true, true), None);
        assert_eq!(choose(0, true, false), Some(Eject::Back));
        assert_eq!(choose(0, false, false), Some(Eject::Over));
    }

    #[test]
    fn nose_up_is_twenty_degrees() {
        assert!(nose_up(-0.35));
        assert!(!nose_up(-0.33));
    }
}
