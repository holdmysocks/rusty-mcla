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

/// The player's rider off a motorcycle, as the game runs it each tick: the anim context once a frame (every other
/// tick), the impact switch at the wipeout's first frame (F-28290's rising edge 0x8000), the eject clip (drivers.rs),
/// GoRagdoll when the clip is over (F-54692 -> F-81936), the ragdoll's ticks (physics::artic_run), its velocity for
/// the wipeout's timer (F-32386 through slot 11, F-62429) and its bones for the draw (F-46606), and RemoveRagdoll when
/// the wipeout ends (the recovery states' exits, F-29164 / F-98260).
#[derive(Default)]
pub struct RiderOff {
    pub ctx: ImpactCtx,
    /// a tick of the frame's pair has run the context
    second: bool,
    wiped: bool,
    /// the eject taken, the velocity kept at the wipeout's start (chassis +800, F-69183) and DamageoutTiming then
    eject: Option<(Eject, Vec3, bool)>,
    seat: Option<usize>,
    pub ragdoll: Option<Box<crate::physics::artic_run::RiderRagdoll>>,
    /// where BIKER_BODYFALL_MASTER plays (F-92308), set on the tick of the ragdoll's first contact batch with no
    /// contact with its own bike (F-15373); the caller takes it and starts the sound ([`BODY_FALL_SOUND`])
    pub body_fall: Option<Vec3>,
    types: hashbrown::HashMap<std::path::PathBuf, Option<std::sync::Arc<mcla_formats::rage::ragdoll::RagdollType>>>,
    surfaces: Vec<crate::physics::collision::Surface>,
}

/// The one-shot F-92308 starts by name through the audio-entity request (F-15373 calls it once per ragdoll): requested
/// volume 0 dB, pitch 0, position = the ragdoll instance matrix's position (request +20 points at rd+64), LPF field 23900,
/// request flags 36 (0x04 | 0x20), +52 = 18 - the same request block as the other entity one-shots (tyre chirps, damage
/// parts), which the port plays through `Req::at`. The sound's tree is BIKER_BODYFALL_RAND + BIKER_BONE_CRACK_RAND +
/// BIKER_BODYFALL_VOCAL_RAND_01 (category CAR_BANGERS), so the tree decides everything else.
pub const BODY_FALL_SOUND: &str = "BIKER_BODYFALL_MASTER";

/// The angular inertia the rider's archetype answers at run time (BuildInput::inertia; how the game makes it is not
/// traced - not the .r6 archetype's): the drv_mp_01_mot_set rider's from the recording ro.0 (collider +160 at GoRagdoll).
/// (open: other character sets take this one too)
const RIDER_INERTIA: crate::physics::artic::V4 = crate::physics::artic::V4([f32::from_bits(0x438a_c2d0), f32::from_bits(0x429b_b1a4), f32::from_bits(0x434f_a81c), f32::from_bits(0x7fc0_0001)]);

/// A glam matrix as the guest's row-vector one (rows = glam's columns).
pub fn guest_of(m: glam::Mat4) -> crate::physics::artic_pose::Mat {
    use crate::physics::artic::V4;
    let c = |v: glam::Vec4| V4::new(v.x, v.y, v.z, v.w);
    [c(m.x_axis), c(m.y_axis), c(m.z_axis), c(m.w_axis)]
}

/// A guest matrix as glam's: its rows' lanes w are the guest's leftovers (the draw reads the 3 x 4 part, F-74033).
pub fn glam_of(m: &crate::physics::artic_pose::Mat) -> glam::Mat4 {
    let c = |v: crate::physics::artic::V4, w: f32| glam::Vec4::new(v.0[0], v.0[1], v.0[2], w);
    glam::Mat4::from_cols(c(m[0], 0.0), c(m[1], 0.0), c(m[2], 0.0), c(m[3], 1.0))
}

impl RiderOff {
    /// One game tick. `seat` the player's car (the drivers' seat id), the damage bits and timer (main.rs).
    #[allow(clippy::too_many_arguments)]
    pub fn tick(&mut self, cars: &mut [crate::vehicles::CarInstance], seat: Option<usize>, drivers: &mut crate::drivers::Drivers, city: &crate::physics::collision::WorldCollider, crash_bits: u8, crash_timer: f32, data_dir: &str, dt: f32) {
        let Some(i) = seat.filter(|&i| cars.get(i).map_or(false, |c| c.model.bike)) else {
            if let Some(s) = self.seat.take() {
                drivers.seat_back(s);
            }
            self.ragdoll = None;
            self.eject = None;
            self.wiped = false;
            return;
        };
        if let Some(s) = self.seat.filter(|&s| s != i) {
            drivers.seat_back(s);
            self.ragdoll = None;
            self.eject = None;
        }
        self.seat = Some(i);
        let c = &mut cars[i];
        if std::env::var_os("MCLA_DBG_RIDER").is_some() {
            eprintln!("RIDER seat {i} bits {crash_bits:#x} wipe {:?} ctx {:#x} pos {:?} vel {:?}", c.state.veh.as_ref().map(|v| v.wipe), self.ctx.bits, c.state.pos, c.state.vel);
        }
        let wiped = crash_bits & 0x10 != 0 || c.state.veh.as_ref().map_or(false, |v| v.wipe.is_some());
        let rm = glam::Mat3::from_quat(c.state.rot);
        let rows = [rm.x_axis, rm.y_axis, rm.z_axis];
        // (F-28290 -> F-71604 once a frame, with the frame's dt)
        self.second = !self.second;
        if self.second {
            let throttle = c.state.veh.as_ref().map_or(c.input.throttle, |v| v.rev.left);
            self.ctx.update(body_velocity(rows, c.state.vel, throttle), dt + dt, &ImpactTune::default());
        }
        if wiped && !self.wiped && self.eject.is_none() {
            // (the impact switch on ImpactDriveable's rising edge; DamageoutTiming = the timer +44 above 0)
            let kept = c.state.vel;
            let timing = crash_timer > 0.0;
            if let Some(e) = choose(self.ctx.bits, nose_up(rows[2].y), c.state.in_water()) {
                let u = if e == Eject::Over && !timing { c.state.vel * 5.0 } else { kept };
                let uz = crate::physics::exact::fma(rows[2].x, u.x, crate::physics::exact::fma(rows[2].z, u.z, rows[2].y * u.y));
                log::info!("rider: {:?} (impact bits {:#x}), clip rate {:.3}", e, self.ctx.bits & 0xF, e.rate(uz));
                drivers.eject(i, e.clip(), e.rate(uz));
                self.eject = Some((e, kept, timing));
            }
        }
        self.wiped = wiped;
        if !wiped {
            if self.eject.take().is_some() | self.ragdoll.take().is_some() {
                // (the recovery's exit: RemoveRagdoll, the rider back to the animation)
                log::info!("rider: back on the seat");
                drivers.seat_back(i);
            }
            if let Some(v) = c.state.veh.as_deref_mut() {
                v.rider_vel = None;
            }
            return;
        }
        // GoRagdoll when the clip is over
        if let Some((local, mat, r6)) = drivers.take_ragdoll_start(i) {
            let ty = self
                .types
                .entry(r6.clone())
                .or_insert_with(|| {
                    let res = crate::assets::load_resource_file(&r6).ok()?;
                    mcla_formats::rage::ragdoll::parse_character_ragdoll(&res).map(std::sync::Arc::new)
                })
                .clone();
            if self.surfaces.is_empty() {
                self.surfaces = crate::physics::collision::Surface::table(data_dir);
            }
            match (ty, self.eject) {
                (Some(ty), Some((e, kept, timing))) if ty.bone_parent.len() == local.len() => {
                    use crate::physics::artic_build::{build, BuildInput, GuestAddrs};
                    let character = guest_of(mat);
                    // (the guest's addresses only tie the blocks together: any distinct values)
                    let addrs = GuestAddrs {
                        body: 0x4000_0000,
                        collider: 0x4100_0000,
                        links: (0..64).map(|k| 0x4200_0000 + 0x400 * k).collect(),
                        joints: (0..64).map(|k| 0x4300_0000 + 0x600 * k).collect(),
                        collider_arrays: [0x4400_0000, 0x4401_0000, 0x4402_0000, 0x4403_0000, 0x4404_0000, 0x4405_0000, 0x4406_0000],
                        inst: 0x4500_0000,
                        solver_obj: 0x4600_0000,
                        type248: 0x4700_0000,
                    };
                    match build(&ty, &BuildInput::new([character[0].0, character[1].0, character[2].0], RIDER_INERTIA.0), &addrs) {
                        Ok(b) => {
                            let locals: Vec<_> = local.iter().map(|(q, p)| guest_of(glam::Mat4::from_rotation_translation(*q, *p))).collect();
                            let launch = e.launch(kept, c.state.vel, timing);
                            log::info!("rider: GoRagdoll, {} links, launch {:.2} {:.2} {:.2}", b.links.len(), launch.x, launch.y, launch.z);
                            self.ragdoll = Some(Box::new(crate::physics::artic_run::RiderRagdoll::go(&ty, b, &locals, character, launch, &self.surfaces)));
                        }
                        Err(err) => log::warn!("rider: the ragdoll's build failed: {err:?}"),
                    }
                }
                (ty, _) => log::warn!("rider: no ragdoll (type {}, {} bones posed)", ty.map_or(0, |t| t.bone_parent.len()), local.len()),
            }
        }
        if let Some(rd) = self.ragdoll.as_mut() {
            let out = rd.tick(dt, city);
            if std::env::var_os("MCLA_DBG_RIDER").is_some() {
                let m = rd.matrix();
                eprintln!("RIDER pos {:?} v {:?} contacts {} island {}", m[3].0, rd.velocity(), out.contacts, out.island);
            }
            if let Some(p) = out.body_fall {
                log::info!("rider: BIKER_BODYFALL at {:.2} {:.2} {:.2}", p.x, p.y, p.z);
                self.body_fall = Some(p);
            }
            if let Some(v) = c.state.veh.as_deref_mut() {
                v.rider_vel = Some(rd.velocity());
            }
            drivers.ragdoll_world(i, rd.bones().iter().map(glam_of).collect());
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
