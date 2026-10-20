//! Arcade vehicle simulation: rigid body + four raycast wheels + sphere body probes.
//! Car local space: +X right, +Y up, -Z forward (matches the art assets).

use super::collision::WorldCollider;
use glam::{Mat3, Quat, Vec3};

#[derive(Clone, Debug)]
pub struct CarParams {
    pub mass: f32,
    pub wheel_pos: [Vec3; 4], // FL, FR, RL, RR attachment points (local, at ride height)
    /// the skeleton's `hub_0` .. `hub_3` bones as the model has them (the deform pieces' places, 0x8235CBD0)
    pub hub_bones: Option<[Vec3; 4]>,
    pub wheel_radius: f32,
    /// tyre width (m)
    pub wheel_width: f32,
    pub susp_rest: f32,
    pub susp_travel: f32,
    pub spring: f32,
    pub damper: f32,
    pub max_steer: f32,
    pub engine_force: f32,
    pub brake_force: f32,
    pub handbrake_force: f32,
    pub drag: f32,
    pub roll_resist: f32,
    pub lat_stiffness: f32,
    pub grip: f32,
    pub top_speed: f32, // m/s
    pub nitro_boost: f32,
    pub body_half: Vec3,
    /// the body's hull for collisions, as the original shapes it (none: seven spheres)
    pub hull: Option<std::sync::Arc<super::hull::HullShape>>,
    pub drive_front: f32, // 0 = RWD, 1 = FWD, 0.5 = AWD
    /// gearbox: ratios first gear first, `gear_count` valid entries
    pub gears: [f32; 8],
    pub gear_count: usize,
    /// normalised torque curve (0..1), 40 samples idle..redline
    pub torque: [f32; 40],
    pub redline: f32,
    /// final drive, derived so that top gear reaches `top_speed` at the redline
    pub final_drive: f32,
    /// the car's own full-throttle acceleration table (m/s^2, evenly spaced over
    /// 0..top_speed); empty = use the generic engine model
    pub accel: Vec<f32>,
    /// tyre friction per axle
    pub grip_front: f32,
    pub grip_rear: f32,
    /// share of the braking done by the front axle
    pub brake_bias: f32,
    /// the original's simulation tuning for this car (None: the simple model below)
    pub veh: Option<std::sync::Arc<super::vehsim::VehTune>>,
    /// damage (mcCarDamageTune): points a car takes before it is a wreck, the points from which a
    /// hard blow finishes it, and the impact speeds (m/s) that count at all, that finish a
    /// battered car, and that wreck any car outright
    pub dmg: [f32; 5],
}

impl CarParams {
    /// Half the width and length of the box the zone damage measures a hit against (0x82356D98):
    /// the car instance's composite bound box (+0x10 max / +0x20 min), which `sub_82599FB0` makes
    /// the union of its parts' boxes - the hull's and each fitted wheel bound's at the wheel's place
    /// (recorded R8: hull x 1.0009, rear wheel 0.7775 + 0.2263 = 1.0038; z 2.1064 .. -2.3572, half
    /// 2.2318; the same box all through a crash). Without a hull: the body box.
    pub fn zone_box(&self) -> [f32; 2] {
        let (lo, hi) = self.composite_box();
        [(hi.x - lo.x) * 0.5, (hi.z - lo.z) * 0.5]
    }

    /// The box of the car's composite bound (0x8259ADA8 / 0x82599FB0): the hull's, margin included,
    /// and the wheels' crowned boxes. (lo, hi) in the car.
    pub fn composite_box(&self) -> (Vec3, Vec3) {
        let Some(h) = self.hull.as_ref() else { return (-self.body_half, self.body_half) };
        // (the hull bound's own box, its 4 cm margin included: R8 1.0009, 2.1064 .. -2.3572)
        let (mut hi, mut lo) = h.bbox.unwrap_or((h.max + Vec3::splat(0.041), h.min - Vec3::splat(0.041)));
        if let Some(vt) = self.veh.as_ref() {
            // (a wheel bound's box: half the crowned shape's width across, its round along - R8
            // front 0.2024 x 0.3072, rear 0.2263 x 0.3190 - at the wheel's place)
            let w = &self.wheel_pos;
            let wheels = if vt.bike { vec![((w[0] + w[1]) * 0.5, 0), ((w[2] + w[3]) * 0.5, 1)] } else { vec![(w[0], 0), (w[1], 0), (w[2], 1), (w[3], 1)] };
            for (at, k) in wheels {
                let (radius, width) = (if k == 0 { vt.t.wheel_front.radius } else { vt.t.wheel_back.radius }.max(0.1), vt.tyre_width[k]);
                if width <= 0.0 {
                    continue;
                }
                let e = Vec3::new(super::vehsim::crown_half_width(radius, width), 0.0, super::vehsim::crown(radius, width, 0.0));
                lo = lo.min(at - e);
                hi = hi.max(at + e);
            }
        }
        (lo, hi)
    }

    /// Derive the final drive from top speed / top gear / redline / wheel radius.
    pub fn finalize(&mut self) {
        let top = self.gears[self.gear_count.saturating_sub(1).min(7)].max(0.1);
        self.final_drive = self.redline * self.wheel_radius / (self.top_speed.max(1.0) * top * 9.5493);
    }
    pub fn rpm_at(&self, fwd_speed: f32, gear: usize) -> f32 {
        let w = fwd_speed.max(0.0) / self.wheel_radius;
        w * self.gears[gear.min(7)] * self.final_drive * 9.5493
    }
    /// Full-throttle acceleration (m/s^2) at a forward speed, from the car's table.
    pub fn accel_at(&self, v: f32) -> f32 {
        let n = self.accel.len();
        if n == 0 {
            return 0.0;
        }
        let x = (v.max(0.0) / self.top_speed.max(1.0)) * n as f32 - 0.5;
        if x >= (n - 1) as f32 {
            // past the last sample the table runs out to zero at the top speed
            let over = x - (n - 1) as f32;
            return (self.accel[n - 1] * (1.0 - over * 2.0)).max(0.0);
        }
        let x = x.max(0.0);
        let i = x.floor() as usize;
        let f = x - i as f32;
        self.accel[i] * (1.0 - f) + self.accel[(i + 1).min(n - 1)] * f
    }
    pub fn torque_at(&self, rpm: f32) -> f32 {
        let t = (rpm / self.redline.max(1.0) * 39.0).clamp(0.0, 39.0);
        let i = t.floor() as usize;
        let fr = t - i as f32;
        self.torque[i] * (1.0 - fr) + self.torque[(i + 1).min(39)] * fr
    }
}

impl Default for CarParams {
    fn default() -> Self {
        CarParams {
            mass: 1300.0,
            hub_bones: None,
            wheel_pos: [
                Vec3::new(-0.78, 0.0, -1.35),
                Vec3::new(0.78, 0.0, -1.35),
                Vec3::new(-0.78, 0.0, 1.30),
                Vec3::new(0.78, 0.0, 1.30),
            ],
            wheel_radius: 0.33,
            wheel_width: 0.24,
            susp_rest: 0.32,
            susp_travel: 0.22,
            spring: 42000.0,
            damper: 4200.0,
            max_steer: 0.55,
            engine_force: 9000.0,
            brake_force: 14000.0,
            handbrake_force: 9000.0,
            drag: 0.42,
            roll_resist: 12.0,
            lat_stiffness: 11.0,
            grip: 1.35,
            top_speed: 62.0,
            nitro_boost: 1.8,
            body_half: Vec3::new(0.9, 0.7, 2.2),
            hull: None,
            gears: [3.4, 2.1, 1.5, 1.15, 0.92, 0.78, 0.0, 0.0],
            gear_count: 6,
            torque: {
                let mut t = [1.0f32; 40];
                for (i, v) in t.iter_mut().enumerate() {
                    let x = i as f32 / 39.0;
                    *v = 0.55 + 0.45 * (1.0 - (x - 0.55).abs() * 1.6).max(0.0);
                }
                t
            },
            redline: 7000.0,
            final_drive: 4.0,
            drive_front: 0.0,
            accel: Vec::new(),
            grip_front: 1.35,
            grip_rear: 1.35,
            brake_bias: 0.6,
            veh: None,
            dmg: [300.0, 250.0, 2.0, 14.0, 17.0],
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CarInput {
    pub throttle: f32,
    pub brake: f32,
    pub steer: f32, // -1 left .. 1 right
    pub handbrake: bool,
    pub nitro: bool,
    /// the rider's weight: 1 leaning back (wheelie) .. -1 forward
    pub lean: f32,
    /// from a computer driver: its throttle is a share of the engine's pull (the player's
    /// pedal has the original's meaning, where half throttle is coasting)
    pub ai: bool,
    /// hydraulics: bits 0-3 the wheel buttons (front left, front right, back left, back right),
    /// `HYD_FITTED` the car has them, `HYD_ON` they are switched on
    pub hyd: u8,
    /// in another car's slipstream: the air's drag falls to the tuning's DragMin
    pub draft: bool,
    /// the weight-transfer button: with the steering, at speed, it tips a car onto two wheels
    pub xfer: bool,
    /// manual gears (the pad's map binds both shifts): the gearbox does not choose
    pub manual: bool,
    /// a shift asked for this tick: 1 up, -1 down
    pub shift: i8,
    /// step 19e: the Ice power-up holds the car (its +624 bit 0x80): the tyres' grip a tenth
    pub ice: bool,
    /// the race's grid flag (chassis +592 bit 0x2, set on every racer from placement until GO;
    /// N-VP-1): mcCarSim::Update holds the gearbox while the handbrake is off and the burnout charge
    /// is under 0.95 (0x8232F098), uses a burnout weight transfer of 0.75 (0x8232F8E4) and locks the
    /// front wheels with ten times their grip (0x8232F304 - seen on the player's car only); the
    /// player's chassis brake is also written 1 by the race code. A computer driver's pedals with it
    /// are the chassis's own (no coasting share).
    pub hold: bool,
}

pub const HYD_FITTED: u8 = 0x40;
pub const HYD_ON: u8 = 0x80;

#[derive(Clone, Copy, Debug, Default)]
pub struct WheelState {
    pub compression: f32, // 0..1 of travel
    pub grounded: bool,
    pub contact: Vec3,
    pub normal: Vec3,
    pub spin: f32,  // rotation angle (rad)
    pub steer: f32, // current steer angle
    pub slip: f32,  // lateral slip magnitude (for effects)
    pub world_pos: Vec3,
    /// surface under the wheel (`collision::Surface`)
    pub surface: u8,
    /// debug: vertical load and applied longitudinal force (N)
    pub load: f32,
    pub f_lon: f32,
}

#[derive(Clone, Debug)]
pub struct CarState {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub ang_vel: Vec3,
    pub wheels: [WheelState; 4],
    pub steer: f32,
    pub rpm: f32,
    pub gear: i32,
    /// nitrous left as a share of the bottle (shots / most shots), for the dial
    pub nitro: f32,
    /// nitrous shots left and the most the car carries (mcCarNitro: no refill but at a gas
    /// station, on a race grid or at a reset)
    pub nitro_shots: u8,
    pub nitro_max: u8,
    pub airborne_time: f32,
    /// burnout: charge built up on the handbrake (0..1) and seconds of launch boost left
    pub burnout: f32,
    pub launch: f32,
    pub damage: f32,
    pub last_impact: f32,
    /// a hit above the wipeout threshold this frame, in thresholds (cleared by whoever acts on it)
    pub wipeout: f32,
    /// wiped out: 1 at the hit, running down to 0 over the tuning's WipeoutDelay (the centre
    /// of gravity is moved up for as long, see vehsim)
    pub upset: f32,
    /// body damage by zone - left, right, front, rear - as the game keeps it (each hit adds
    /// to the zones it lands in; 1 is a wrecked side), and the half width and length they
    /// are measured against
    pub zones: [f32; 4],
    pub zone_half: [f32; 2],
    /// the deform pieces, the damage map's queued stamps and the hits' effects (crate::damagemap)
    pub dmg: crate::damagemap::CarDamage,
    /// current engine output multiplier (torque curve x gear ratio), set in `step`
    pub engine_factor: f32,
    /// state of the ported simulation (drivetrain, tyres)
    pub veh: Option<Box<super::vehsim::VehState>>,
    /// this frame's contacts as the collision audio reads them (cleared by `Vehicles::update`), and a count of the
    /// solves that gave them
    pub touches: Vec<AudioTouch>,
    pub touch_calls: u32,
    /// the hood, trunk lid and bumpers that come loose (`crate::damageparts`), with this tick's queued hits
    pub parts: crate::damageparts::DamageParts,
}

/// What the contact callback reads for a scrape (0x82343CA8 .. 0x82344254): the car's velocity at the
/// point (`v1`) and the other's (`v2`, zero against the city), the centres' velocity difference
/// (`rel`) and the call's contact normals summed and made unit (`n`).
#[derive(Clone, Copy, Debug)]
pub struct ScrapeIn {
    pub v1: Vec3,
    pub v2: Vec3,
    pub rel: Vec3,
    pub n: Vec3,
}

impl ScrapeIn {
    /// The scrape's amount (0x82344254 / 0x82344394): over 3 m/s of closing and along the surface -
    /// the closing velocity's share against the normal strictly under 0.7 on a floor (n.y over
    /// 0.5), 0.6 else - the difference of the two points' squared speeds x 0.0001, at most 0.125.
    pub fn amount(&self) -> f32 {
        let r = self.rel.length();
        if r <= 3.0 {
            return 0.0;
        }
        let c = self.rel.dot(self.n) / r;
        let thr = if self.n.y > 0.5 { f32::from_bits(0x3f33_3333) } else { f32::from_bits(0x3f19_999a) };
        if !(-thr < c && c < thr) {
            return 0.0;
        }
        ((self.v1.length_squared() - self.v2.length_squared()).abs() * f32::from_bits(0x38d1_b717)).clamp(0.0, 0.125)
    }
}

/// A contact of the car as the original's collision audio reads it (sub_82381FB0, docs/reference/audio/COLLISION_AUDIO_RULE.md).
#[derive(Clone, Copy, Debug)]
pub struct AudioTouch {
    /// which solve it came from (`CarState::touch_calls` at the time): each solve is one call of the audio callback
    pub call: u32,
    pub pos: Vec3,
    /// from the car into what it touches
    pub normal: Vec3,
    /// the car's velocity at the point less the other's (before the solve answers it)
    pub vel: Vec3,
    /// the car's own material there (car_side / car_roof / car_undercarriage / car_wheel) and the other's
    pub own: super::collision::Surface,
    pub other: super::collision::Surface,
    /// the other is a racing vehicle
    pub vehicle: bool,
}

impl CarState {
    /// a nitrous shot is burning
    pub fn nitro_on(&self) -> bool {
        self.veh.as_ref().map_or(false, |v| v.nitro_level > 0.0)
    }

    pub fn new(pos: Vec3, yaw: f32) -> CarState {
        CarState {
            pos,
            rot: Quat::from_rotation_y(yaw),
            vel: Vec3::ZERO,
            ang_vel: Vec3::ZERO,
            wheels: [WheelState::default(); 4],
            steer: 0.0,
            rpm: 900.0,
            gear: 1,
            nitro: 1.0,
            nitro_shots: 1,
            nitro_max: 1,
            airborne_time: 0.0,
            burnout: 0.0,
            launch: 0.0,
            damage: 0.0,
            last_impact: 0.0, wipeout: 0.0, upset: 0.0, zones: [0.0; 4], zone_half: [0.9, 2.2], dmg: Default::default(),
            engine_factor: 1.0,
            veh: None,
            touches: Vec::new(),
            touch_calls: 0,
            parts: Default::default(),
        }
    }

    pub fn forward(&self) -> Vec3 {
        self.rot * -Vec3::Z
    }
    pub fn up(&self) -> Vec3 {
        self.rot * Vec3::Y
    }
    pub fn right(&self) -> Vec3 {
        self.rot * Vec3::X
    }
    /// Move the body out of another car after physics, carrying its hub poses.
    /// Ground contacts stay at the world samples used for this frame's forces.
    pub fn correct_position(&mut self, offset: Vec3) {
        self.pos += offset;
        for wheel in &mut self.wheels { wheel.world_pos += offset; }
    }

    /// Move a complete chassis pose without leaving its visual hubs behind.
    /// Ground contacts remain samples of the old world until the next physics tick.
    pub fn relocate(&mut self, pos: Vec3, rot: Quat) {
        let inverse = self.rot.inverse();
        for wheel in &mut self.wheels {
            let local = inverse * (wheel.world_pos - self.pos);
            wheel.world_pos = pos + rot * local;
        }
        self.pos = pos;
        self.rot = rot;
    }
    pub fn speed(&self) -> f32 {
        self.vel.length()
    }
    /// signed speed along forward
    pub fn fwd_speed(&self) -> f32 {
        self.vel.dot(self.forward())
    }

    fn inertia(p: &CarParams) -> Vec3 {
        let h = p.body_half * 2.0;
        let m = p.mass / 12.0;
        Vec3::new(m * (h.y * h.y + h.z * h.z), m * (h.x * h.x + h.z * h.z) * 1.3, m * (h.x * h.x + h.y * h.y))
    }

    /// An impact that changes the car's speed by `hit` m/s, counted as the original counts it
    /// (mcCarDamage, 0x82343378): under ImpactThreshold nothing; otherwise the hit itself is
    /// added to the damage points. At MaxDamage the car is a wreck - but only a blow above
    /// FatalBlow can finish it, a softer one leaves it one point short. A single hit above
    /// WipeoutThreshold is a wipeout (`wipeout` holds how many thresholds it was worth).
    pub fn take_hit(&mut self, p: &CarParams, hit: f32) {
        let [max, _med, impact, fatal, wipeout] = p.dmg;
        if hit <= impact {
            return;
        }
        // (a blow above the WeightxferThreshold, 10 on every car, knocks a car off its two wheels)
        if hit > 10.0 {
            if let Some(v) = self.veh.as_mut() {
                v.two = 0;
            }
        }
        let max = max.max(1.0);
        let mut points = self.damage * max + hit;
        if points >= max {
            points = if hit <= fatal { max - 1.0 } else { max };
        }
        self.damage = (points / max).min(1.0);
        if hit > wipeout && wipeout > 0.0 && self.damage < 1.0 {
            self.wipeout = self.wipeout.max(hit / wipeout);
        }
    }

    /// The mean of the zones (each at most 1).
    pub fn zone_mean(&self) -> f32 {
        self.zones.iter().map(|z| z.min(1.0)).sum::<f32>() / 4.0
    }

    /// What a delivery's customer sees (the game's DELIVERY_DAMAGE: the mean of the two sides
    /// and the front).
    pub fn delivery_damage(&self) -> f32 {
        self.zones[..3].iter().map(|z| z.min(1.0)).sum::<f32>() / 3.0
    }

    /// A blow of `hit` m/s landed at `point` (world): the zone damage there.
    pub fn add_dent(&mut self, point: Vec3, hit: f32) {
        // ((hit - 0.5) / 5, at most 1)
        let local = self.rot.inverse() * (point - self.pos);
        self.zone_hit(point, local, ((hit - 0.5) * 0.2).clamp(0.0, 1.0));
    }

    /// What the original's contact callback (mcCarDamage, 0x82343880) does with the tick's
    /// contacts of the car against the city: `dv` is the change of velocity they gave it - the sum
    /// of their impulses over the mass, each 0.4 of itself against something fixed (0x82567A08) -
    /// and `at` the first of them (on a car lying over, under 0.1 of upright, the last), `local` the
    /// same point in the car's pose after the tick's move (the instance's matrix then). A wrecked
    /// car takes nothing more. Over 0.1 m/s the zones and the bumpers' queue take
    /// clamp((sqrt dv - 0.5) x 0.2) at that point (0x82344394, zone damage kind 0); then the
    /// damage points take dv itself (0x82343378: a city hit counts 1.0 of it, mcCarDamage +12).
    pub(super) fn world_hit(&mut self, p: &CarParams, dv: f32, at: Vec3, local: Vec3, scrape: Option<ScrapeIn>) {
        self.damage_call(p, dv, 1.0, at, local, scrape);
    }

    /// The same callback for the car's contacts with another car or a loose prop (not fixed: their
    /// impulses count whole, 0x82567A08): `dv` the change of velocity they gave this car, `at` the
    /// first contact's point (world, in the car's pose now). The damage points take dv x `mult`
    /// (0x82342EA8): against another car this car's own +16 (1: Racer_SetDamageMultiplierFlag's
    /// factor, which nothing sets) times the other car's mcCarDamage +20 - its "damage inflicted"
    /// factor (Racer_SetDamageInflictedFactor; 1 but in a payback); against anything else +12 = 1.
    /// Recorded: sa.0 (R8 among loose pieces, 40 ticks) and sd.3 (R8 ramming traffic, 340 ticks)
    /// - mcCarDamage +56 is exactly this dv; the points rise by it x 1.
    pub fn contact_hit(&mut self, p: &CarParams, dv: f32, mult: f32, at: Vec3) {
        let local = self.rot.inverse() * (at - self.pos);
        self.damage_call(p, dv, mult, at, local, None);
    }

    /// `scrape`: the velocities and normal the callback reads for a scrape (None: the port's own
    /// contacts, which have none - the soft spheres between cars without the original's body, the
    /// props that stand fast).
    pub(super) fn damage_call(&mut self, p: &CarParams, dv: f32, mult: f32, at: Vec3, local: Vec3, scrape: Option<ScrapeIn>) {
        if self.damage >= 1.0 {
            return;
        }
        if dv > f32::from_bits(0x3dcc_cccd) {
            // (kind 1 first, 0x82344254 .. 0x82344470: a scrape along the surface)
            let amount = scrape.map_or(0.0, |s| s.amount());
            if amount != 0.0 {
                self.scrape_hit(at, local, amount, scrape.unwrap());
            }
            let amount = ((dv.sqrt() - 0.5) * 0.2).clamp(0.0, 1.0);
            if amount != 0.0 {
                self.zone_hit(at, local, amount);
            }
        }
        let hit = dv * mult;
        if hit > p.dmg[2] {
            self.take_hit(p, hit);
        }
    }

    /// The zone damage for a scrape (0x82356D98, kind 1, work/cardmg scrape_rule.md 2): above 0.12
    /// of the way up, the pieces (1000 on a car upside down) and the bumpers' queue (which take no
    /// scrape but keep its slot); the sparks; then eight stamps of half the amount into the map's
    /// blue, every 0.25 m back along the car's own velocity from the point - each that lies above
    /// 0.12 of the way up.
    fn scrape_hit(&mut self, point: Vec3, local: Vec3, amount: f32, s: ScrapeIn) {
        let upside_down = (self.rot * Vec3::Y).y < 0.0;
        if crate::damageparts::hit_counts(local) {
            let a = if upside_down { 1000.0 } else { amount };
            self.dmg.pieces_hit(local, a, 1);
            self.parts.queue(crate::damageparts::PartHit { point, amount: a, kind: 1 });
        }
        // (a car upside down makes them twice, the first with the amount, 0x82356F60 / 0x823571A4;
        // the port keeps 32 for the effects to take)
        if self.dmg.sparks.len() >= 32 {
            self.dmg.sparks.drain(..16);
        }
        if upside_down {
            self.dmg.sparks.push(crate::damagemap::Sparks { at: point, normal: s.n, rel: s.rel, amount });
        }
        self.dmg.sparks.push(crate::damagemap::Sparks { at: point, normal: s.n, rel: s.rel, amount: if upside_down { 1.0 } else { amount } });
        let back = self.rot.inverse() * self.vel.normalize_or_zero();
        for i in 0..8 {
            let q = (local - back * (i as f32 * 0.125 * 2.0)).normalize_or_zero();
            if q.y > f32::from_bits(0x3df5_c28f) {
                self.dmg.stamp(1, q, amount * 0.5);
            }
        }
    }

    /// The game's zone damage (0x82356d98, a collision: kind 0) with `amount` at `point` (world;
    /// `local` in the car). False when the hit is too low on the car to count.
    fn zone_hit(&mut self, point: Vec3, local: Vec3, amount: f32) -> bool {
        // (the game's zone damage, 0x82356d98: only a hit above 0.12 of the way up from the car's
        // origin counts - the deform pieces, the bumpers' queue @82356F60 and the zones @82357444 -
        // and on a car upside down the zones take 1.0 and the bumpers 1000, DAMAGE_PARTS_RULE.md 1.2)
        if std::env::var_os("MCLA_PARTS_LOG").is_some() {
            eprintln!("parts hit: {amount:.4} at car-local {local:.2?} counts {}", crate::damageparts::hit_counts(local));
        }
        if !crate::damageparts::hit_counts(local) {
            return false;
        }
        let upside_down = (self.rot * Vec3::Y).y < 0.0;
        // (the deform pieces: the windows crack, crate::damagemap)
        self.dmg.pieces_hit(local, if upside_down { 1000.0 } else { amount }, 0);
        // (to the front or rear zone for a hit in the outer two fifths of the length, to a side
        // zone for one in the outer half of the width; a collision: kind 0 - every caller here is one)
        self.parts.queue(crate::damageparts::PartHit { point, amount: if upside_down { 1000.0 } else { amount }, kind: 0 });
        let amount = if upside_down { 1.0 } else { amount };
        let [hx, hz] = self.zone_half;
        if local.z < -0.6 * hz {
            self.zones[2] += amount;
        }
        if local.z > 0.6 * hz {
            self.zones[3] += amount;
        }
        if local.x < -0.5 * hx {
            self.zones[0] += amount;
        }
        if local.x > 0.5 * hx {
            self.zones[1] += amount;
        }
        // (@82357514: a wrecked rear zone lets the trunk lid fall open, a wrecked front the hood)
        if self.zones[3] >= 1.0 {
            self.parts.trunk.loose = true;
        }
        if self.zones[2] >= 1.0 {
            self.parts.hood.loose = true;
        }
        // (a dent into the damage map's red at the hit's direction, 0x8230A180)
        self.dmg.stamp(0, local.normalize_or_zero(), amount);
        true
    }

    pub(super) fn resolve_body_pub(&mut self, p: &CarParams, world: &WorldCollider) {
        self.resolve_body(p, world);
    }

    /// What a blow of `speed` (m/s, closing) at a point does to the car besides stopping it.
    pub(super) fn blow(&mut self, p: &CarParams, speed: f32, at: Vec3) {
        if speed > 3.0 {
            self.take_hit(p, speed);
            self.add_dent(at, speed);
            self.last_impact = 0.5;
        }
    }

    pub fn step(&mut self, p: &CarParams, input: CarInput, world: &WorldCollider, dt: f32) {
        self.zone_half = p.zone_box();
        // (the deform pieces' places, once the model is in (0x8235CBD0): the hubs at the skeleton's hub bones,
        // the windows by the composite bound's box `sub_8255B9A8(car + 8)` - the one the collision's cell test
        // reads, set up there (vehsim.rs); V: the R8's as the original's DMAP record, test
        // the_r8_pieces_and_layout_are_the_originals. A car without the original's sim: the body box here.)
        if !self.dmg.pieces_set && (p.veh.is_none() || p.hull.is_none()) {
            let (lo, hi) = p.composite_box();
            self.dmg.set_up(p.hub_bones.unwrap_or(p.wheel_pos), lo, hi);
        }
        if let Some(vt) = p.veh.clone() {
            super::vehsim::step(self, p, &vt, input, world, dt);
            return;
        }
        // gearbox / engine: pick the gear from the current speed (with hysteresis),
        // then the torque multiplier from the car's own torque curve
        let v = self.fwd_speed().max(0.0);
        let n = p.gear_count.clamp(1, 8);
        let mut g = (self.gear.max(1) as usize - 1).min(n - 1);
        if g + 1 < n && p.rpm_at(v, g) > p.redline * 0.97 {
            g += 1;
        } else if g > 0 && p.rpm_at(v, g) < p.redline * 0.40 {
            g -= 1;
        }
        let mut rpm = p.rpm_at(v, g).max(p.redline * 0.14);
        if v < 2.0 && input.throttle > 0.1 {
            rpm = rpm.max(p.redline * (0.3 + 0.2 * input.throttle));
        }
        if input.nitro && self.nitro > 0.0 {
            rpm = (rpm * 1.05).min(p.redline);
        }
        if self.burnout > 0.0 {
            // wheels spinning against the handbrake
            rpm = rpm.max(p.redline * (0.78 + 0.1 * (self.burnout * 23.0).sin()));
        }
        self.engine_factor = p.torque_at(rpm) * (p.gears[g] / p.gears[0].max(0.1)).clamp(0.2, 1.0);
        self.rpm = rpm;
        self.gear = if self.fwd_speed() < -0.5 && input.brake > 0.1 { -1 } else { g as i32 + 1 };

        let sub = 4;
        let h = dt / sub as f32;
        // keep the last sane state: a degenerate contact must never poison the simulation
        let backup = (self.pos, self.rot, self.wheels);
        for _ in 0..sub {
            self.substep(p, input, world, h);
        }
        let sane = self.pos.is_finite() && self.rot.is_finite() && self.vel.is_finite() && self.ang_vel.is_finite() && self.vel.length() < 250.0 && self.ang_vel.length() < 60.0;
        if !sane {
            self.pos = backup.0;
            self.rot = backup.1;
            self.wheels = backup.2;
            self.vel = Vec3::ZERO;
            self.ang_vel = Vec3::ZERO;
            self.steer = 0.0;
        }
        self.last_impact = (self.last_impact - dt).max(0.0);
    }

    fn substep(&mut self, p: &CarParams, input: CarInput, world: &WorldCollider, dt: f32) {
        let mut hub_local = [Vec3::ZERO; 4];
        let fwd = self.forward();
        let up = self.up();
        let right = self.right();
        let inertia = Self::inertia(p);
        let mut force = Vec3::new(0.0, -9.81 * p.mass, 0.0);
        let mut torque = Vec3::ZERO;
        let speed = self.vel.length();
        let fwd_speed = self.vel.dot(fwd);

        // steering with speed-sensitive limit
        // full lock at parking speeds; above that only as much lock as the front tyres
        // can turn into cornering force (a little more, so the car can be thrown into a slide)
        let wheelbase = (p.wheel_pos[2].z - p.wheel_pos[0].z).abs().max(1.5);
        let a_max = p.grip_front * 9.81 * 1.35;
        let grip_lock = (wheelbase * a_max / (speed * speed).max(1.0)).atan();
        let steer_limit = p.max_steer.min(grip_lock.max(0.035));
        let target = input.steer.clamp(-1.0, 1.0) * steer_limit;
        let rate = 6.0 * dt;
        self.steer += (target - self.steer).clamp(-rate, rate);

        let mut grounded_count = 0;
        let nitro_on = input.nitro && self.nitro > 0.0;
        // Burnout: handbrake and gas from a stop spin the driven wheels (and let the car be
        // turned on the spot); letting go of the handbrake launches it.
        let burning = input.handbrake && input.throttle > 0.6 && speed < 3.5 && input.brake < 0.1;
        if burning {
            self.burnout = (self.burnout + dt / 1.3).min(1.0);
        } else if self.burnout > 0.0 {
            if !input.handbrake && input.throttle > 0.5 && self.burnout > 0.15 {
                self.launch = 0.5 + 1.3 * self.burnout;
            }
            self.burnout = 0.0;
        }
        self.launch = (self.launch - dt).max(0.0);
        let drive_force = if !p.accel.is_empty() {
            // the car's own acceleration curve; it is a net figure, so the resistances
            // this model applies separately are added back on top
            let v = fwd_speed.max(0.0);
            let mut a = p.accel_at(v);
            if nitro_on {
                // nitrous: a strong shove that also lifts the top speed
                a = a * 1.5 + 3.0 * (1.0 - (v / (p.top_speed * 1.25)).clamp(0.0, 1.0));
            }
            let resist = v * v * p.drag + v * p.roll_resist + v * v * 0.0;
            // (the wheel loop below hands out twice the drive force, see drive_share)
            input.throttle.clamp(0.0, 1.0) * (p.mass * a + resist) * 0.5
        } else {
            let mut f = input.throttle.clamp(0.0, 1.0) * p.engine_force * self.engine_factor;
            // top speed limiter
            let r = (fwd_speed.max(0.0) / p.top_speed).clamp(0.0, 1.0);
            f *= (1.0 - r * r * r * r).max(0.0);
            if nitro_on {
                f *= p.nitro_boost;
            }
            f
        };
        let drive_force = if burning { 0.0 } else if self.launch > 0.0 { drive_force * 1.6 } else { drive_force };
        let brake_force = input.brake.clamp(0.0, 1.0) * p.brake_force;
        let reverse = input.brake > 0.1 && fwd_speed < 1.0;

        for i in 0..4 {
            let front = i < 2;
            let local = p.wheel_pos[i];
            let attach = self.pos + self.rot * local;
            let ray_len = p.susp_rest + p.susp_travel + p.wheel_radius;
            let w = &mut self.wheels[i];
            w.steer = if front { self.steer } else { 0.0 };
            let hit = world.raycast(attach, -up, ray_len);
            let Some(hit) = hit else {
                w.grounded = false;
                w.compression = 0.0;
                hub_local[i] = local - Vec3::Y * (p.susp_rest + p.susp_travel * 0.5);
                w.world_pos = attach - up * (p.susp_rest + p.susp_travel * 0.5);
                continue;
            };
            grounded_count += 1;
            w.grounded = true;
            w.contact = hit.point;
            w.normal = hit.normal;
            let susp_len = (hit.t - p.wheel_radius).clamp(0.0, p.susp_rest + p.susp_travel);
            let comp = (p.susp_rest + p.susp_travel - susp_len) / (p.susp_rest + p.susp_travel);
            w.compression = comp;
            hub_local[i] = local - Vec3::Y * susp_len;
            w.world_pos = attach - up * susp_len;

            // suspension force
            let point_vel = self.vel + self.ang_vel.cross(attach - self.pos);
            let susp_speed = point_vel.dot(up);
            let x = (p.susp_rest - susp_len).max(-0.05);
            let mut fs = p.spring * x - p.damper * susp_speed;
            if hit.t < p.wheel_radius + 0.02 {
                fs += p.spring * 4.0 * (p.wheel_radius + 0.02 - hit.t); // bump stop
            }
            let fs = fs.max(0.0);
            let n = hit.normal;
            let f_susp = n * fs;
            force += f_susp;
            torque += (attach - self.pos).cross(f_susp);

            // tire frame
            let wheel_fwd = {
                let f = Quat::from_axis_angle(up, -w.steer) * fwd;
                (f - n * f.dot(n)).normalize_or(fwd)
            };
            let wheel_right = n.cross(wheel_fwd).normalize_or(right);
            let cvel = self.vel + self.ang_vel.cross(hit.point - self.pos);
            let v_lat = cvel.dot(wheel_right);
            let v_lon = cvel.dot(wheel_fwd);
            let load = fs.max(p.mass * 9.81 * 0.1);
            // what the wheel stands on: grass, gravel, sand and the river bed grip less,
            // sand and water also hold the car back
            let (surf_grip, surf_drag) = super::collision::surface(hit.surface);
            w.surface = hit.surface as u8;
            // (hot tyres bite: a burnout launch has a third more traction)
            let max_f = load * if front { p.grip_front } else { p.grip_rear } * if self.launch > 0.0 { 1.35 } else { 1.0 } * surf_grip;

            // longitudinal
            let mut f_lon = 0.0;
            let drive_share = if front { p.drive_front } else { 1.0 - p.drive_front } * 2.0;
            if reverse {
                f_lon -= p.engine_force * 0.5 * drive_share * 0.5;
            } else {
                f_lon += drive_force * drive_share * 0.5;
                let bias = if front { p.brake_bias } else { 1.0 - p.brake_bias };
                f_lon -= brake_force * bias * 0.5 * v_lon.signum() * (v_lon.abs().min(1.0));
            }
            let hb = input.handbrake && !front;
            if hb {
                f_lon -= p.handbrake_force * 0.5 * v_lon.signum() * (v_lon.abs().min(1.0));
            }
            // rolling resistance
            f_lon -= v_lon * p.roll_resist * 0.25;
            f_lon -= v_lon * surf_drag * p.mass * 0.25;

            // lateral (slip-angle based, saturating)
            let slip = if speed > 0.5 { (v_lat / (v_lon.abs() + 1.0)).clamp(-2.0, 2.0) } else { v_lat * 2.0 };
            // (in a burnout the tail slides round freely)
            let lat_grip = if burning { if front { 0.3 } else { 0.08 } } else if hb { 0.55 } else { 1.0 };
            let mut f_lat = -slip * p.lat_stiffness * load * lat_grip;
            // friction circle
            let total = (f_lon * f_lon + f_lat * f_lat).sqrt();
            if total > max_f {
                let s = max_f / total;
                f_lon *= s;
                f_lat *= s;
            }
            // low-speed lateral damping to kill creep
            if speed < 1.0 && !burning {
                f_lat -= v_lat * p.mass * 0.6;
            }
            w.slip = (total / max_f.max(1.0)).min(2.0);
            let driven = if front { p.drive_front > 0.05 } else { p.drive_front < 0.95 };
            if driven && (burning || self.launch > 0.9) {
                w.slip = 1.7;
                w.spin += 60.0 * dt;
            }
            w.load = load;
            w.f_lon = f_lon;
            let f = wheel_fwd * f_lon + wheel_right * f_lat;
            force += f;
            // apply tyre forces at roll-centre height (just below the CoM) to keep the
            // arcade car from flipping in hard corners
            let app = hit.point + up * (self.pos - hit.point).dot(up) * 0.85;
            torque += (app - self.pos).cross(f);
            w.spin += v_lon / p.wheel_radius * dt;
        }

        if burning && grounded_count >= 3 {
            // swing the tail round with the steering
            let want = -input.steer.clamp(-1.0, 1.0) * 1.5;
            let have = self.ang_vel.dot(up);
            torque += up * (want - have) * inertia.y * 14.0;
            // and stay put while doing it
            force -= Vec3::new(self.vel.x, 0.0, self.vel.z) * p.mass * 3.0;
        }
        if grounded_count == 0 {
            self.airborne_time += dt;
            // mild air control
            torque += right * (-input.throttle + input.brake) * 800.0;
        } else {
            self.airborne_time = 0.0;
        }

        // aero drag + downforce
        force -= self.vel * speed * p.drag;
        force -= up * speed * speed * 0.9;

        // integrate linear
        self.vel += force / p.mass * dt;
        self.pos += self.vel * dt;
        // integrate angular (body-space inertia)
        let rm = Mat3::from_quat(self.rot);
        let torque_local = rm.transpose() * torque;
        let ang_local = rm.transpose() * self.ang_vel;
        let acc_local = torque_local / inertia;
        let ang_local = ang_local + acc_local * dt;
        // angular damping: stronger on roll/pitch than on yaw
        let ang_local = Vec3::new(ang_local.x * (1.0 - 3.0 * dt), ang_local.y * (1.0 - 1.0 * dt), ang_local.z * (1.0 - 3.0 * dt));
        self.ang_vel = rm * ang_local;
        if self.ang_vel.length() > 0.0 {
            let dq = Quat::from_scaled_axis(self.ang_vel * dt);
            self.rot = (dq * self.rot).normalize();
        }
        // nitro usage
        if input.nitro && self.nitro > 0.0 && input.throttle > 0.1 {
            self.nitro = (self.nitro - dt * 0.25).max(0.0);
        } else {
            self.nitro = (self.nitro + dt * 0.03).min(1.0);
        }

        self.resolve_body(p, world);
        // Carry the sampled suspension lengths through the final body pose.
        for (wheel, local) in self.wheels.iter_mut().zip(hub_local) {
            wheel.world_pos = self.pos + self.rot * local;
        }
    }

    /// Push the body out of walls with sphere probes at the corners and sides.
    fn resolve_body(&mut self, p: &CarParams, world: &WorldCollider) {
        let r = 0.45;
        let h = p.body_half;
        let probes = [
            Vec3::new(-h.x + r, 0.3, -h.z + r),
            Vec3::new(h.x - r, 0.3, -h.z + r),
            Vec3::new(-h.x + r, 0.3, h.z - r),
            Vec3::new(h.x - r, 0.3, h.z - r),
            Vec3::new(-h.x + r, 0.3, 0.0),
            Vec3::new(h.x - r, 0.3, 0.0),
            Vec3::new(0.0, 0.9, 0.0),
        ];
        // What stands lower than a wheel can climb is the wheels' business: a recorded run of the
        // original takes a kerb at 24 m/s with a 20 kN knock on its suspension and no loss of
        // speed, where a body that met the kerb's face lost a third of it.
        let (origin, up) = (self.pos, self.up());
        for lp in probes {
            let wp = self.pos + self.rot * lp;
            if let Some((pen, n, _)) = world.sphere_where(wp, r, |q| (q - origin).dot(up) > 0.3) {
                // ignore ground-like contacts (handled by wheels) unless we are deep
                // (a bike's probes sit by its wheels: reared up on the back wheel they dip well
                // under the road without the bike touching it)
                let margin = if p.veh.as_ref().map_or(false, |v| v.bike) { 0.5 } else { 0.25 };
                if n.y > 0.6 && pen < margin {
                    continue;
                }
                // (the ground under the body: only what is deeper than the wheels' share is
                // pushed out, and the floor does not throw the car back up)
                let floor = n.y > 0.6;
                self.pos += n * if floor { pen - margin } else { pen };
                let rel = self.vel + self.ang_vel.cross(wp - self.pos);
                let vn = rel.dot(n);
                if vn < 0.0 {
                    let impulse = -vn * if floor { 1.0 } else { 1.0 + 0.15 };
                    self.vel += n * impulse * 0.9;
                    let arm = wp - self.pos;
                    self.ang_vel += arm.cross(n * impulse) * 0.25 / (p.body_half.length_squared().max(1.0));
                    // scrape friction (walls; the floor under a low body only holds it up)
                    let tangent = rel - n * vn;
                    if !floor {
                        self.vel -= tangent * 0.08;
                    }
                    if -vn > 3.0 {
                        self.take_hit(p, -vn);
                        self.add_dent(wp - n * r, -vn);
                        self.last_impact = 0.5;
                    }
                }
            }
        }
        self.catch_fall(world);
    }

    /// safety: never fall through the world
    pub(super) fn catch_fall(&mut self, world: &WorldCollider) {
        if let Some(g) = world.ground(self.pos, 1.0, 3.0) {
            // only genuine fall-through: the car's origin is a few centimetres under the road
            // as it stands (the wheels' places and sizes are the original's), and a kerb passing
            // under the middle of the car puts the ground 15-20 cm above it for a moment
            if self.pos.y < g.point.y - 0.6 {
                self.pos.y = g.point.y - 0.05;
                if self.vel.y < 0.0 {
                    self.vel.y = 0.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The R8's deform pieces where the original has them (work/dmgmap, the DMAP record: hubs (+-0.782, 0.343,
    /// -+1.325), glass 8 (0, 0.401, -2.357), 9 (0, 0.401, 2.106), 10 / 11 (-+1.004, 0.401, 0)) and its damage
    /// layout (`vp_aud_r8_08_maxdamage`, 256 x 256).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition vehicle resources"]
    fn the_r8_pieces_and_layout_are_the_originals() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
        let assets = crate::assets::Assets::new(&root);
        let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle/vp_aud_r8_08")).unwrap();
        assert_eq!(car.damage_layout.as_ref().map(|l| (l.0, l.1)), Some((256, 256)));
        let mut p = crate::vehicles::car_params(&car);
        // (the hull and tuning as the game sets them, vehicles.rs)
        let (s, h, hubs) = (car.simtune.as_ref().unwrap(), car.hull.as_ref().unwrap(), car.hubs.unwrap());
        let mut vt = super::super::vehsim::VehTune::new(s.base.clone());
        vt.seat((hubs[0].y + hubs[2].y) * 0.5);
        p.veh = Some(std::sync::Arc::new(vt));
        p.hull = Some(std::sync::Arc::new(super::super::hull::shaped(h, &s.base, (hubs[0].y + hubs[2].y) * 0.5, car.bike)));
        let mut world = WorldCollider::default();
        world.sectors.insert(0, super::super::collision::SectorMesh::build(vec![
            [Vec3::new(-64.0, 0.0, -64.0), Vec3::new(-64.0, 0.0, 64.0), Vec3::new(64.0, 0.0, -64.0)],
            [Vec3::new(64.0, 0.0, -64.0), Vec3::new(-64.0, 0.0, 64.0), Vec3::new(64.0, 0.0, 64.0)],
        ]));
        let mut st = CarState::new(Vec3::new(0.0, 0.3, 0.0), 0.0);
        st.step(&p, CarInput::default(), &world, 1.0 / 60.0);
        assert!(st.dmg.pieces_set);
        let want = [
            (0, Vec3::new(-0.782, 0.343, -1.325)), (1, Vec3::new(0.782, 0.343, -1.325)), (2, Vec3::new(-0.782, 0.343, 1.325)), (3, Vec3::new(0.782, 0.343, 1.325)),
            (8, Vec3::new(0.0, 0.401, -2.357)), (9, Vec3::new(0.0, 0.401, 2.106)), (10, Vec3::new(-1.004, 0.401, 0.0)), (11, Vec3::new(1.004, 0.401, 0.0)),
        ];
        for (i, w) in want {
            let at = st.dmg.pieces[i].local;
            eprintln!("piece {i}: {at:.4} (original {w:.3})");
            assert!(at.distance(w) < 0.003, "piece {i}: {at:.4} against {w:.3}");
        }
    }

    /// The R8 driven head-on into a wall at 26 m/s (recording sa.0 of the original, tick 153):
    /// the contacts change its velocity by 10.881046 m/s, landing at car-local (0.807251,
    /// 0.431519, -2.352051) - mcCarDamage +56 / +64 / +36 - and the next contact ticks by
    /// 0.334477 m/s at the other front corner. The zone box is the original's live bound box
    /// (half 1.0038 x 2.2318).
    #[test]
    fn a_wall_hit_counts_as_the_original() {
        let p = CarParams::default();
        let mut st = CarState::new(Vec3::ZERO, 0.0);
        st.zone_half = [1.0038, 2.2318];
        let at = Vec3::new(0.807251, 0.431519, -2.352051);
        st.world_hit(&p, 10.881046, at, at, None);
        assert!((st.damage * p.dmg[0] - 10.881046).abs() < 1e-4);
        // (sqrt 10.881046 - 0.5) x 0.2, to the right side and the front
        let a = (10.881046f32.sqrt() - 0.5) * 0.2;
        assert_eq!(st.zones, [0.0, a, a, 0.0]);
        assert!((a - 0.5597286).abs() < 1e-6);
        assert_eq!(st.parts.hits.len(), 1);
        // a small one adds to the zones but not to the damage points (under ImpactThreshold 2)
        let at = Vec3::new(-0.816284, 0.534737, -2.355713);
        st.world_hit(&p, 0.334477, at, at, None);
        assert!((st.zones[0] - 0.015668).abs() < 1e-5 && (st.zones[2] - a - 0.015668).abs() < 1e-5);
        assert!((st.damage * p.dmg[0] - 10.881046).abs() < 1e-4);
        // under 0.25 m/s (sqrt 0.5) the amount is 0; a hit low on the car (y under 0.12 of the
        // way out) leaves the zones alone
        let z = st.zones;
        st.world_hit(&p, 0.2, at, at, None);
        st.world_hit(&p, 5.0, Vec3::new(0.9, 0.1, -2.3), Vec3::new(0.9, 0.1, -2.3), None);
        assert_eq!(st.zones, z);
        assert!((st.damage * p.dmg[0] - 15.881046).abs() < 1e-3);
        // a wrecked car takes nothing more
        st.damage = 1.0;
        st.world_hit(&p, 30.0, at, at, None);
        assert_eq!(st.zones, z);
    }

    /// The R8 ramming traffic (recording sd.3 of the original): its contacts with another car
    /// count whole (no 0.4), and the points take |dv| x the other car's "damage inflicted" factor
    /// (1 here): k650 |dv| 2.998156 at car-local (-0.879272, 0.380019, -2.234070), points 23.848392
    /// -> 26.846548; k687 2.340403 at (0, 0.532755, -2.355995) -> 29.186951; k688 0.269759 adds none.
    #[test]
    fn a_car_hit_counts_as_the_original() {
        let p = CarParams::default();
        let max = p.dmg[0];
        let mut st = CarState::new(Vec3::ZERO, 0.0);
        st.zone_half = [1.0037985, 2.2318041];
        st.damage = 23.848392 / max;
        st.contact_hit(&p, 2.998156, 1.0, Vec3::new(-0.879272, 0.380019, -2.234070));
        assert!((st.damage * max - 26.846548).abs() < 1e-3);
        let a = (2.998156f32.sqrt() - 0.5) * 0.2;
        assert!((st.zones[0] - a).abs() < 1e-6 && (st.zones[2] - a).abs() < 1e-6 && st.zones[1] == 0.0 && st.zones[3] == 0.0);
        st.contact_hit(&p, 2.340403, 1.0, Vec3::new(0.0, 0.532755, -2.355995));
        assert!((st.damage * max - 29.186951).abs() < 1e-3);
        st.contact_hit(&p, 0.269759, 1.0, Vec3::new(0.000061, 0.532799, -2.356178));
        assert!((st.damage * max - 29.186951).abs() < 1e-3);
        // the payback's player inflicts 3x: a 1 m/s change, under the threshold alone, counts 3
        st.contact_hit(&p, 1.0, 3.0, Vec3::new(0.0, 0.53, -2.35));
        assert!((st.damage * max - 32.186951).abs() < 1e-3);
    }

    /// The scrape (kind 1, scrape_rule.md): along a wall at 20 m/s the amount is 20^2 x 1e-4; head on,
    /// or under 3 m/s of closing, none. Its eight stamps go back along the car's way, half the
    /// amount each; a collision stamps the dent's direction (damagemap.rs).
    #[test]
    fn a_scrape_marks_the_map() {
        let side = ScrapeIn { v1: Vec3::new(0.0, 0.0, -20.0), v2: Vec3::ZERO, rel: Vec3::new(0.0, 0.0, -20.0), n: Vec3::X };
        assert!((side.amount() - 0.04).abs() < 1e-6);
        assert_eq!(ScrapeIn { rel: Vec3::new(-20.0, 0.0, -1.0), ..side }.amount(), 0.0);
        assert_eq!(ScrapeIn { rel: Vec3::new(0.0, 0.0, -2.9), ..side }.amount(), 0.0);
        // (on a floor - n.y over 0.5 - the share may reach 0.7; capped at 0.125 from 35.4 m/s)
        let fast = ScrapeIn { v1: Vec3::new(0.0, 0.0, -40.0), rel: Vec3::new(0.0, -26.0, -30.0), n: Vec3::Y, ..side };
        assert_eq!(fast.amount(), 0.125);
        let p = CarParams::default();
        let mut st = CarState::new(Vec3::ZERO, 0.0);
        st.vel = Vec3::new(0.0, 0.0, -20.0);
        let at = Vec3::new(-0.95, 0.6, -0.5);
        st.world_hit(&p, 1.0, at, at, Some(side));
        assert_eq!(st.dmg.scrapes.len(), 8);
        assert!(st.dmg.scrapes.iter().all(|s| (s.amount - 0.02).abs() < 1e-6));
        // (the trail runs back - +z - from the point)
        assert!(st.dmg.scrapes[7].dir.z > st.dmg.scrapes[0].dir.z);
        assert_eq!(st.dmg.sparks.len(), 1);
        // the collision's own stamp: (sqrt 1 - 0.5) x 0.2 at the point's direction
        assert_eq!(st.dmg.dents.len(), 1);
        assert!((st.dmg.dents[0].amount - 0.1).abs() < 1e-6 && (st.dmg.dents[0].dir - at.normalize()).length() < 1e-6);
    }
}
