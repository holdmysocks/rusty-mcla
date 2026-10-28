//! The slipstream turbo, mcCarSSTurbo (vtable 0x820470E0; work/slipstream/report.txt): every car a
//! mcPlayer drives (the player and the race opponents - mcPlayer::CreateCar, 0x822A6560, builds it)
//! charges it in another such car's slipstream and fires it like a nitrous shot.
//!
//! One object per car, its update (0x823465C8) runs in the chassis update after mcCarNitro's: while it
//! burns the level runs down over turboUseTime and pushes the car; otherwise the strongest slipstream
//! around (0x82346438 over the player manager's list, 0x82346140 for each car) charges it.

use glam::Vec3;
use mcla_formats::rage::simtune::SsTurboTune;

use super::exact::{approach, fma, fnms, inverse_frame, point_through, unit_or_zero};

/// The object's state (offsets of mcCarSSTurbo).
#[derive(Clone, Copy, Debug)]
pub struct SsTurbo {
    /// +36: the charge (0..1) while it does not burn, the burn's level (1 -> 0) while it does
    pub level: f32,
    /// +40
    pub burning: bool,
    /// +41: the charge has reached 1 (cleared when it runs out or burns)
    pub full: bool,
    /// +44: seconds a full charge is still held out of the stream
    pub stay: f32,
    /// +20: degrees the lens is opened by (0 at rest, turboFOV while it burns)
    pub fov: f32,
    /// +24: the camera distance's scale (1 at rest, turboDist while it burns)
    pub dist: f32,
    /// +28: blurStartAmount while it burns, back to 1 at rest
    pub blur: f32,
    /// +64: the slipstream's strength this tick (0 out of one)
    pub strength: f32,
    /// +72: the car whose slipstream this one is in (index in the vehicle list), -1 none - also what
    /// picks the air's DragMin in mcCarSim::Update (0x82330024)
    pub leader: i32,
    /// +68: the car in this one's slipstream, -1 none
    pub drafter: i32,
    /// the sst_trail behind this car (mcCarModel +7972 block: +4 the target, +8 the faded value)
    pub trail_target: f32,
    pub trail: f32,
    /// this tick's search (0x82346438), made for every car before the cars are stepped: the strongest
    /// slipstream's strength and car
    pub found: (f32, i32),
    /// the drafter (+68) is still in a slipstream (its own search found one)
    pub drafter_drafting: bool,
    /// chassis +576 bit 0x10, set and cleared only by the script native Racer_LockFeature (sub_822ADF90; hash
    /// af5d85f0): the turbo neither charges, fades nor fires. None of the disc's 869 scripts calls it
    /// (`lock_feature_calls_of_the_disc`), so it stays false.
    pub locked: bool,
    /// the port's count of fires (0x82345E40), read for the player's stats gSST / nSST
    pub fires: u32,
}

impl Default for SsTurbo {
    fn default() -> Self {
        // (0x82345CC8)
        SsTurbo { level: 0.0, burning: false, full: false, stay: 0.0, fov: 0.0, dist: 1.0, blur: 1.0, strength: 0.0, leader: -1, drafter: -1, trail_target: 0.0, trail: 0.0, found: (0.0, -1), drafter_drafting: false, locked: false, fires: 0 }
    }
}

/// What 0x82346140 needs of a car.
#[derive(Clone, Copy, Debug)]
pub struct DraftCar {
    /// the car's place (chassis matrix row 3)
    pub pos: Vec3,
    /// the car's up (row 1)
    pub up: Vec3,
    /// the body's velocity (body +272)
    pub vel: Vec3,
    /// chassis +224 (the speed its update came in with)
    pub speed: f32,
}

impl SsTurbo {
    /// 0x82345E20 (the cine car's set-down and others): nothing charged, nothing burning.
    pub fn reset(&mut self) {
        self.burning = false;
        self.full = false;
        self.level = 0.0;
        self.stay = 0.0;
    }

    /// 0x82345D58 (mcCarSim's reset, slot +4; mcCar's update when a wiped-out car is set back, sim +592 bit
    /// 0x10 with its damage timer run out): everything back to rest.
    pub fn reset_all(&mut self) {
        self.burning = false;
        self.full = false;
        self.level = 0.0;
        self.stay = 0.0;
        self.strength = 0.0;
        self.drafter = -1;
        self.leader = -1;
        self.fov = 0.0;
        self.dist = 1.0;
        self.blur = 1.0;
    }

    /// 0x82345FE0: `answering` is the chassis's vfunc 12 (0x8232C368: car state +592 0, or 16 on a car); not
    /// with chassis +576 bit 0x10 (`locked`).
    pub fn can_fire(&self, answering: bool) -> bool {
        !self.locked && answering && self.full && self.level >= 1.0 && !self.burning && self.fov == 0.0
    }

    /// 0x82345E40: the level 1 and burning. (The camera's jolt, the pad's rumble 3 and the player's
    /// stats gSST / nSST are the callers' business.)
    pub fn fire(&mut self) {
        self.level = 1.0;
        self.burning = true;
        self.fires = self.fires.wrapping_add(1);
    }

    /// The burning half of 0x823465C8. `upset`: 0x823455B8 (air time <= 0 and up.y under 0.1). Returns
    /// -(turboBoostAmount x level^turboBoostExp), the factor of the push along the car's back row, before
    /// the throttle, the share of the wheels down and the mass.
    pub fn burn(&mut self, t: &SsTurboTune, upset: bool, dt: f32) -> f32 {
        self.level = fnms(1.0 / t.use_time, dt, self.level);
        if !(self.level > 0.0) || upset {
            self.level = 0.0;
            self.stay = 0.0;
            self.burning = false;
            self.full = false;
        }
        let p = (self.level as f64).powf(t.boost_exp as f64) as f32;
        let k = -(t.boost_amount * p);
        self.blur = t.blur_start_amount;
        if self.fov != t.fov {
            approach(&mut self.fov, t.fov, t.fov.abs() / t.fov_out_time, dt);
        }
        if self.dist != t.dist {
            approach(&mut self.dist, t.dist, (t.dist - 1.0).abs() / t.fov_out_time, dt);
        }
        k
    }

    /// The charging half of 0x823465C8, with this tick's search in `found` / `drafter_drafting`.
    /// `speed`: chassis +224.
    pub fn charge(&mut self, t: &SsTurboTune, speed: f32, dt: f32) {
        self.strength = 0.0;
        // (0x823467E4: with chassis +576 bit 0x10 the update ends here. Byte +48 - a forced full-strength charge
        // and the car's own trail at 1 - has no writer but the constructor, so it is left out.)
        if self.locked {
            return;
        }
        let fall = 1.0 / if self.full { t.charged_falloff_time } else { t.falloff_time };
        let (s, idx) = self.found;
        if !(s > 0.0) {
            if self.stay > 0.0 {
                self.stay -= dt;
                if self.stay < 0.0 {
                    self.stay = 0.0;
                }
            } else if self.level > 0.0 {
                self.trail_target = 0.0;
                self.level = fnms(dt, fall, self.level);
                if speed < t.lose_charge_speed {
                    // (ten times as much again under loseChargeSpeed: 10.0 at 0x82003A8C)
                    self.level = fnms(dt * fall, 10.0, self.level);
                }
                if !(self.level > 0.0) {
                    self.level = 0.0;
                    self.burning = false;
                    self.stay = 0.0;
                    self.full = false;
                }
            }
            self.leader = -1;
        } else {
            // (the leader's trail and its +68 are written by the search's caller, Vehicles::draft_pass)
            self.strength = s;
            self.level = fma((1.0 / t.regen_time) * dt, s, self.level);
            if self.level >= 1.0 {
                self.level = 1.0;
                self.full = true;
                self.stay = t.stay_full_time;
            }
            self.leader = idx;
        }
        if self.drafter == -1 || !self.drafter_drafting {
            self.drafter = -1;
            self.trail_target = 0.0;
        }
        if self.blur != 1.0 {
            self.blur = fma((1.0 / t.blur_off_time) * (1.0 - self.blur), dt, self.blur);
        }
        if self.fov != 0.0 {
            approach(&mut self.fov, 0.0, t.fov.abs() / t.fov_in_time, dt);
        }
        if self.dist != 1.0 {
            approach(&mut self.dist, 1.0, (t.dist - 1.0).abs() / t.fov_in_time, dt);
        }
    }

    /// The trail's fade (mcCarModel 0x82379D48 -> 0x82375D90): towards its target at 4.5 a second
    /// (0x820317C8), not within 0.01 (0x82002C60); then the target is dropped unless the local player is
    /// this car or the car in its slipstream (`keep`). Drawn above 0.001 (0x820022B4, 0x82379B30).
    pub fn fade_trail(&mut self, keep: bool, dt: f32) {
        let d = self.trail - self.trail_target;
        if d.abs() > 0.01 {
            let step = (4.5 * dt).min(d.abs());
            self.trail = if d < 0.0 { self.trail + step } else { self.trail - step };
        }
        if !keep {
            self.trail_target = 0.0;
        }
    }
}

/// 0x82346140: the strength of `leader`'s slipstream where `me` is, with the leader's tuning. The envelope
/// is laid along the leader's motion: rows right = cross(up, back) made unit, the car's own up, back = the
/// velocity's opposite made unit, its place; `me` in it at z (behind) 0..envelopeLength, |x| and |y| within
/// envelopeWidth2. Strength = (1 - (z/L)^2) (1 - min(|x| / w, 1)^2) times the leader's speed share, w
/// widening from envelopeWidth1 at the car to envelopeWidth2 at the end; the share (s - min) / full
/// between tune +32 and +36 (divided by +36 itself, as written), 1 from +36 up, 0 under +32.
pub fn strength(t: &SsTurboTune, leader: &DraftCar, me: Vec3) -> f32 {
    if leader.speed < t.min_speed {
        return 0.0;
    }
    let share = if leader.speed >= t.full_speed { 1.0 } else { (leader.speed - t.min_speed) / t.full_speed };
    let back = -unit_or_zero(leader.vel);
    let up = leader.up;
    let right = unit_or_zero(up.cross(back));
    // (F-43153 -> F-30106: the frame inverted with its place in the fourth row, in the original's cofactor
    // arithmetic; the point taken through it. The original answers 0 for a frame all but singular and then
    // leaves its output alone: no slipstream here.)
    let Some(inv) = inverse_frame(&[right, up, back], leader.pos) else { return 0.0 };
    let local = point_through(me, &inv);
    let (l, w1, w2) = (t.envelope_length, t.envelope_width1, t.envelope_width2);
    if local.z < 0.0 || local.z > l || local.x.abs() > w2 || local.y.abs() > w2 {
        return 0.0;
    }
    let z = local.z / l;
    let along = fnms(z, z, 1.0);
    let w = fma(w2 - w1, z, w1);
    let a = (local.x / w).abs().min(1.0);
    let across = fnms(a, a, 1.0);
    across * along * share
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tune() -> SsTurboTune {
        // (every playable car's, base and upgraded alike - `ss_turbo_tunes_of_the_disc`; distance 0.8 here to see it
        // move, the disc's is 1)
        SsTurboTune { boost_exp: 3.0, boost_amount: 25.0, use_time: 2.5, regen_time: 1.5, falloff_time: 15.0, charged_falloff_time: 0.89998, stay_full_time: 1.2, lose_charge_speed: 20.0, min_speed: 9.0, full_speed: 45.0, fov_out_time: 0.5, fov_in_time: 1.0, fov: 15.0, dist: 0.8, blur_off_time: 0.25, blur_start_amount: 0.25, envelope_length: 45.0, envelope_width1: 1.75, envelope_width2: 2.0, ..Default::default() }
    }

    #[test]
    fn envelope_strength() {
        let t = tune();
        let lead = DraftCar { pos: Vec3::ZERO, up: Vec3::Y, vel: Vec3::new(0.0, 0.0, -50.0), speed: 50.0 };
        // right behind (+z is the leader's back, against its motion): 1 - (z/L)^2
        let s = strength(&t, &lead, Vec3::new(0.0, 0.0, 15.0));
        assert!((s - (1.0 - (15.0f32 / 45.0).powi(2))).abs() < 1e-6, "{s}");
        // ahead of it, beyond the envelope, or too far aside or above: nothing
        assert_eq!(strength(&t, &lead, Vec3::new(0.0, 0.0, -1.0)), 0.0);
        assert_eq!(strength(&t, &lead, Vec3::new(0.0, 0.0, 45.5)), 0.0);
        assert_eq!(strength(&t, &lead, Vec3::new(2.1, 0.0, 10.0)), 0.0);
        assert_eq!(strength(&t, &lead, Vec3::new(0.0, 2.1, 10.0)), 0.0);
        // aside: the width grows from envelopeWidth1 to envelopeWidth2 along the length
        let s = strength(&t, &lead, Vec3::new(0.6, 0.0, 22.5));
        let w = 1.75 + (2.0 - 1.75) * 0.5;
        assert!((s - (1.0 - (0.6f32 / w).powi(2)) * 0.75).abs() < 1e-6, "{s}");
        // a slower leader: (speed - min) / full
        let slow = DraftCar { speed: 19.5, vel: Vec3::new(0.0, 0.0, -19.5), ..lead };
        let s = strength(&t, &slow, Vec3::new(0.0, 0.0, 0.0));
        assert!((s - (19.5 - 9.0) / 45.0).abs() < 1e-6, "{s}");
        assert_eq!(strength(&t, &DraftCar { speed: 8.9, ..slow }, Vec3::new(0.0, 0.0, 5.0)), 0.0);
    }

    #[test]
    fn charge_hold_and_fall() {
        let t = tune();
        let dt = 1.0 / 60.0;
        let mut s = SsTurbo::default();
        // full strength: full after turboRegenTime
        s.found = (1.0, 0);
        let mut n = 0;
        while !s.full {
            s.charge(&t, 40.0, dt);
            n += 1;
        }
        assert!((n as f32 * dt - t.regen_time).abs() <= dt, "{n}");
        assert_eq!(s.stay, t.stay_full_time);
        assert!(s.can_fire(true));
        assert!(!s.can_fire(false));
        // out of the stream: held for stayFullTime, then gone in chargedFalloffTime - ten times faster
        // under loseChargeSpeed
        s.found = (0.0, -1);
        let mut held = 0;
        while s.level >= 1.0 {
            s.charge(&t, 40.0, dt);
            held += 1;
        }
        assert!((held as f32 * dt - t.stay_full_time).abs() <= 3.0 * dt, "{held}");
        assert!(!s.can_fire(true), "below 1 it cannot be fired");
        let mut fall = 1;
        while s.level > 0.0 {
            s.charge(&t, 40.0, dt);
            fall += 1;
        }
        assert!((fall as f32 * dt - t.charged_falloff_time).abs() <= 2.0 * dt, "{fall}");
        assert!(!s.full);
        s.level = 0.5;
        let mut slow = 0;
        while s.level > 0.0 {
            s.charge(&t, 10.0, dt);
            slow += 1;
        }
        assert!((slow as f32 * dt - 0.5 * t.falloff_time / 11.0).abs() <= dt, "{slow}");
    }

    #[test]
    fn ai_drafted_trail_by_list_order() {
        // A computer racer in a slipstream writes -1 to its leader's +68 (its player index, F-55794), so the
        // leader's own update drops the trail target (sst_trail_evidence_2026-10-28 s.10). Leader after the
        // follower in the list: cleared before the model's fade, never drawn (5001 -> 5003 recorded: 0 in every
        // poll). Leader before the follower: set again after its update each tick, so it fades in at 4.5 a second
        // and out at 4.5 a second once the follower leaves (5003 -> 5002 recorded: 0.075 a frame both ways).
        let t = tune();
        let dt = 1.0 / 60.0;
        let follow = |l: &mut SsTurbo| {
            l.trail_target = (0.5f32 + 0.75).clamp(0.0, 1.0);
            l.drafter = -1;
        };
        let mut after = SsTurbo::default();
        for _ in 0..60 {
            follow(&mut after);
            after.charge(&t, 40.0, dt);
            after.fade_trail(false, dt);
            assert_eq!(after.trail, 0.0);
        }
        let mut before = SsTurbo::default();
        let mut seen = vec![];
        for _ in 0..16 {
            before.charge(&t, 40.0, dt);
            follow(&mut before);
            before.fade_trail(false, dt);
            seen.push(before.trail);
        }
        assert!((seen[0] - 0.075).abs() < 1e-6 && (seen[1] - 0.15).abs() < 1e-6 && seen[15] == 1.0, "{seen:?}");
        let mut n = 0;
        while before.trail > 0.0 {
            before.charge(&t, 40.0, dt);
            before.fade_trail(false, dt);
            n += 1;
        }
        assert_eq!(n, 14, "out at 0.075 a frame, the last step the rest");
    }

    #[test]
    fn burn_runs_down_and_lens() {
        let t = tune();
        let dt = 1.0 / 60.0;
        let mut s = SsTurbo { level: 1.0, full: true, ..Default::default() };
        s.fire();
        let k = s.burn(&t, false, dt);
        let l = 1.0 - dt / t.use_time;
        assert!((k + t.boost_amount * l * l * l).abs() < 1e-4, "{k}");
        assert_eq!(s.blur, t.blur_start_amount);
        let mut n = 1;
        while s.burning {
            s.burn(&t, false, dt);
            n += 1;
        }
        assert!((n as f32 * dt - t.use_time).abs() <= 2.0 * dt, "{n}");
        assert_eq!(s.fov, t.fov);
        assert_eq!(s.dist, t.dist);
        assert!(!s.full && s.level == 0.0);
        // the lens comes back over fovInTime, and only then can it be fired again
        s.found = (0.0, -1);
        s.charge(&t, 40.0, dt);
        assert!(s.fov < t.fov && s.fov > 0.0);
        // upset: out at once
        let mut u = SsTurbo { level: 1.0, full: true, ..Default::default() };
        u.fire();
        assert_eq!(u.burn(&t, true, dt), 0.0);
        assert!(!u.burning);
    }

    #[test]
    fn bike_follower_matches_recording() {
        // The player on the ZX-14 held 12 m behind racer 304 in his freeway race, the original polled every tick
        // (work/slipstream/bike/cap/b1.pkl, sst_trail_evidence_2026-10-28 s.11): level before, the update's strength
        // +64, level after - a motorcycle charges, fills, holds, falls and burns by the same object and tune as a car.
        let t = SsTurboTune { dist: 1.0, ..tune() };
        let dt = 1.0f32 / 60.0;
        let f = f32::from_bits;
        for (l0, s, l1) in [
            (0x3f425e78, 0x3d10091c, 0x3f427813),
            (0x3e212fa6, 0x3eee3bee, 0x3e267af0),
            (0x3ea067a0, 0x3f09b4e3, 0x3ea37706),
            (0x3f4fbb16, 0x3f2abd25, 0x3f51a0be),
            (0x3f5a35de, 0x3f692257, 0x3f5ccd01),
            (0x3ea072c2, 0x3f7f8957, 0x3ea6207a),
        ] {
            let mut st = SsTurbo { level: f(l0), found: (f(s), 5), ..Default::default() };
            st.charge(&t, 40.0, dt);
            assert_eq!(st.level.to_bits(), l1, "charge {} x {}", f(l0), f(s));
            assert_eq!((st.strength, st.leader), (f(s), 5));
        }
        // the filling tick: level 1, full, held stayFullTime
        let mut st = SsTurbo { level: 0.9988753795623779, found: (0.7006515264511108, 5), ..Default::default() };
        st.charge(&t, 40.0, dt);
        assert!(st.level == 1.0 && st.full && st.stay == 1.2);
        // held out of the stream: 1.2 -> 1.18333 -> 1.16667
        st.found = (0.0, -1);
        st.charge(&t, 57.0, dt);
        assert_eq!(st.stay.to_bits(), 0x3f977778);
        st.charge(&t, 57.0, dt);
        assert_eq!(st.stay.to_bits(), 0x3f955556);
        // a full charge falling at 1 / chargedFalloffTime (57 m/s)
        for (l0, l1) in [(0x3f800000, 0x3f7b4258), (0x3f7b4258, 0x3f7684b0), (0x3d3f3646, 0x3ce6b788), (0x3ce6b788, 0x3c1e0509)] {
            let mut st = SsTurbo { level: f(l0), full: true, ..Default::default() };
            st.charge(&t, 57.0, dt);
            assert_eq!(st.level.to_bits(), l1, "fall {}", f(l0));
        }
        // the burn: 1 / turboUseTime a tick, 150 ticks; the last one leaves 8.1e-7 and burns one tick more
        let mut st = SsTurbo { level: 1.0, full: true, ..Default::default() };
        st.fire();
        let mut seen = vec![];
        while st.burning {
            st.burn(&t, false, dt);
            seen.push(st.level.to_bits());
        }
        assert_eq!(&seen[..2], &[0x3f7e4b18, 0x3f7c9630]);
        assert_eq!(&seen[148..150], &[0x3bda7ae3, 0x355a8c60]);
        assert_eq!(seen.len(), 151);
        // the strength the bike got (+64) from 304's polled up row, place, body velocity and +224 and the bike's
        // matrix row 3 (b2.pkl, polls between two updates)
        let v3 = |a: [u32; 3]| Vec3::new(f(a[0]), f(a[1]), f(a[2]));
        for (up, pos, vel, speed, me, s) in [
            ([0xbc385c05, 0x3f7fce33, 0xbd18d83c], [0x445f13e0, 0x42158f26, 0xc4ce0a7b], [0x4189984f, 0x401fd2d1, 0x42828f66], 0x42871ba6, [0x445e398d, 0x4213768f, 0xc4cf9f9c], 0x3f69da7d),
            ([0xbc943030, 0x3f7ff368, 0xbbf76162], [0x4464fc95, 0x421dd47f, 0xc4c50069], [0x41dc3d07, 0x3f9cea2e, 0x428263f9], 0x428d8f62, [0x4463a77a, 0x421d5a3a, 0xc4c681c7], 0x3f668db7),
            ([0xbc9db897, 0x3f7ff3bf, 0xbae6a4ee], [0x4466b7e0, 0x421ed274, 0xc4c31f0f], [0x420273e6, 0x3f550ed5, 0x427d6898], 0x428e8478, [0x446545a1, 0x421e22af, 0xc4c49b46], 0x3f63d46d),
        ] {
            let lead = DraftCar { pos: v3(pos), up: v3(up), vel: v3(vel), speed: f(speed) };
            let got = strength(&t, &lead, v3(me));
            assert!((got - f(s)).abs() < 1e-6, "strength {got} recorded {}", f(s));
        }
    }

    /// Every script call of Racer_LockFeature on the disc (MCLA_DATA_DIR=...\extracted\cache).
    #[test]
    #[ignore]
    fn lock_feature_calls_of_the_disc() {
        fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() { walk(&p, out) } else if p.extension().map_or(false, |x| x == "sco") { out.push(p) }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")), &mut files);
        let hashes = [mcla_formats::hash::joaat_bytes(b"Racer_LockFeature"), mcla_formats::hash::joaat_bytes(b"racer_lockfeature")];
        let mut n = 0;
        for f in &files {
            let Some(sco) = std::fs::read(f).ok().and_then(|d| mcla_formats::sco::decode(&d).ok()) else { continue };
            // (also the raw bytes, in case the walk misses one)
            for h in hashes {
                let b = h.to_le_bytes();
                n += sco.code().windows(4).filter(|w| *w == b).count();
            }
            for c in sco.calls() {
                if hashes.contains(&c.hash) {
                    n += 1;
                    println!("{} pc {} hash {:08x} args {:?}", f.display(), c.pc, c.hash, c.args);
                }
            }
        }
        println!("{} scripts, {n} calls", files.len());
    }

    /// Who opens the slipstream tutorial forms 78 / 79: every Form_GetForm / Form_ShowForm call of the disc's
    /// scripts with a constant 78 or 79 (MCLA_DATA_DIR=...\extracted\cache).
    #[test]
    #[ignore]
    fn slipstream_tutorial_calls_of_the_disc() {
        fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() { walk(&p, out) } else if p.extension().map_or(false, |x| x == "sco") { out.push(p) }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")), &mut files);
        let names = ["Form_GetForm", "Form_ShowForm", "Form_CloseForm", "Form_GetFormVisible"];
        let hashes: Vec<u32> = names.iter().map(|n| mcla_formats::hash::joaat_bytes(n.as_bytes())).chain(names.iter().map(|n| mcla_formats::hash::joaat_bytes(n.to_lowercase().as_bytes()))).collect();
        let mut n = 0;
        let mut ids = std::collections::BTreeMap::new();
        for f in &files {
            let Some(sco) = std::fs::read(f).ok().and_then(|d| mcla_formats::sco::decode(&d).ok()) else { continue };
            for c in sco.calls() {
                let all = format!("{:?}", c.args);
                if all.contains("TUTORIAL_SST") {
                    println!("{} pc {} hash {:08x} args {:?}", f.display(), c.pc, c.hash, c.args);
                }
                if !hashes.contains(&c.hash) {
                    continue;
                }
                let form = c.args.get(1).map(|a| format!("{:?}", a)).unwrap_or_default();
                *ids.entry(form.clone()).or_insert(0) += 1;
                if form == "Num(78.0)" || form == "Num(79.0)" {
                    n += 1;
                    println!("{} pc {} hash {:08x} args {:?}", f.display(), c.pc, c.hash, c.args);
                }
            }
        }
        println!("form arguments of the calls: {:?}", ids);
        println!("{} scripts, {n} calls", files.len());
    }

    /// The disc's slipstream tunings, every car (MCLA_DATA_DIR=...\extracted\cache).
    #[test]
    #[ignore]
    fn ss_turbo_tunes_of_the_disc() {
        let dir = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")).join("resources/vehicle");
        let mut rows = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            for f in std::fs::read_dir(e.path()).unwrap().flatten() {
                if f.path().extension().map_or(false, |x| x == "r50") {
                    let Ok(res) = crate::assets::load_resource_file(&f.path()) else { continue };
                    let Some(st) = mcla_formats::rage::simtune::SimTunes::parse(&res) else { continue };
                    for (k, t) in [("base", &st.base.ss_turbo), ("mods", &st.mods.ss_turbo)] {
                        rows.push(format!("{} {k} {:?}", e.file_name().to_string_lossy(), t));
                    }
                }
            }
        }
        rows.sort();
        for r in &rows {
            println!("{r}");
        }
        assert!(!rows.is_empty());
    }
}
