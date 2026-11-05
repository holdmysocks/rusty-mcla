//! Drivers and riders: the `drv_*_lod02_set` characters posed with each car's own driver
//! animations (`$/resources/animation/Drv/<Player|Opponent>/Male/<car>.xapk`: one pack per car with
//! `<CAR>_Drv_steer_sweep`, `_Drv_Idle_stp`, `_Drv_Idle_Low`, `_Drv_rev`, ...; `pack_path`). The
//! animations are authored in car space, so the root track seats the driver.

use crate::assets::load_resource_file;
use crate::render::{Frustum, Gpu};
use crate::drvanim as da;
use crate::drvmachine as dm;
use crate::skin::{blend_pose, Clip, FrameDof, SkinBuffer, SkinModel, DOF_NO_VALUE, DOF_QUATERNION, DOF_VECTOR3};
use glam::{Mat4, Quat, Vec3};
use hashbrown::HashMap;
use mcla_formats::rage::anim::{animation_named, pack_entries};
use mcla_formats::rage::Rd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What the driver of one car needs to know this frame.
pub struct Seat<'a> {
    pub id: usize,
    /// the car's number in the session (`CarInstance::uid`): fixed while the car lives, unlike its
    /// place in the list, which moves when an earlier car goes
    pub uid: u32,
    pub car: &'a str,
    pub mat: Mat4,
    pub pos: Vec3,
    /// -1 left .. 1 right
    pub steer: f32,
    pub speed: f32,
    pub reversing: bool,
    pub player: bool,
    pub cop: bool,
    pub bike: bool,
    /// step 19d: the character set this driver is ("drv_mb_009_set"), else empty
    pub set: &'a str,
    /// LoadDriver's hi-res set (F-39898): the local human, or a racer raised by UseRacerNativeCharacter (F-94066)
    pub hi: bool,
    /// the car's mcCarModelTune vDriverGaugeLightOffset / vDriverNeonLightOffset (`SimTunes::driver_lights`)
    pub lights: Option<[[f32; 3]; 2]>,
    /// the car's record's gauge colour A and interior neon colour (w: IntNeonIntensity) - None: its own set-up's
    pub colors: Option<[[f32; 4]; 2]>,
    /// the car's head lamp state (lamp object +168, eased: `AmbLamps::s[0]`)
    pub head: f32,
    /// the car body's velocity (body +272; its length is the chassis' +224 speed): the wind on a rider (`Wind::step`)
    pub vel: Vec3,
    /// a rider's: what the bike pose machine reads of the bike (its `ck` and `x` are set here)
    pub ride: RideIn,
    /// a car driver's: what the car machine's gatherer reads of the car (none for a rider)
    pub drv: Option<DrvIn>,
}

/// What a car driver's gatherer (F-28290, `drvmachine::gather`) reads of the port's car this frame (main.rs fills it), and the
/// steering part's fields it reads (D +1124 the wheel's eased angle, +1176 the racer's divisor, +1164 / +1136).
#[derive(Clone, Copy, Default)]
pub struct DrvIn {
    pub g: dm::Gather,
    pub needle: f32,
    pub divisor: f32,
    pub out: f32,
    pub scale: f32,
    pub racer: dm::Racer,
    pub cutscene: bool,
    /// the local human's look (the view's +112 x 0.8 through the rival look, -1 looking back: main.rs `rider_look`)
    pub look: f32,
    /// the damage object's +44 / +48 (the gatherer clears +48: `Drivers::hit_taken`)
    pub dmg: [f32; 2],
}

/// What the gatherer reads outside the car (main.rs): the race's over byte (race +3773; None without a race), the human's
/// horn sounding (the horn test F-26939: the car audio's +1404 slot, F-58539), the player's crash state (chassis +592's 0x10 / 0x04 bits and the damage
/// object's +44 timer; None for another car) and the replay stream's byte (+76: 0x80 recording, 0x40 playing).
#[derive(Clone, Copy, Debug, Default)]
pub struct DrvWorld {
    pub race_over: Option<bool>,
    pub horn: bool,
    pub crash: Option<(u8, f32)>,
    pub replay: u8,
}

impl DrvIn {
    /// The gatherer's reads from the port's car. The body rows and velocity, the pedals, the gearbox (+20 0 = reverse, the
    /// original's reverse-first order), the burnout charge, the nitrous, the wheels' slide (+520) and contact records, the
    /// grid and deactivated bits of chassis +592. Approximate (docs/reference/people/car_anim_machine.md "Wiring"): no
    /// race / GPS / damage / water / horn / powerup inputs, the view's camera test (0x400) off, the cockpit view as the
    /// view's mode 1 with the player manager's tests false, the racer's +656 and the sim's alarm value 0.
    pub fn of(st: &crate::physics::CarState, input: &crate::physics::CarInput, sp: &crate::steer_parts::SteerParts, player: bool, cutscene: bool, ckpt: bool, look: f32, w: &DrvWorld) -> DrvIn {
        let vs = st.veh.as_deref();
        let (r0, r1, r2) = (st.rot * Vec3::X, st.rot * Vec3::Y, st.rot * Vec3::Z);
        let stick = vs.filter(|v| v.pad.live).map_or(input.lean, |v| v.pad.stick_y);
        let g = dm::Gather {
            racer: true,
            human: player,
            player,
            gesture: 20,
            vehicle: true,
            ready: true,
            chassis: true,
            mgr: Some(false),
            view: player.then_some(dm::View { cam: None, mode: if ckpt { 1 } else { 0 }, dist: 0.0 }),
            cine_cam: cutscene,
            // (+592: 0x1 deactivated, 0x2 the grid hold, 0x4 damaged out, 0x10 wiped out - the player's from main.rs's
            // crash state, another car's from its damage at the limit and its upset)
            w592: input.deactivated as u32
                | (input.hold as u32) << 1
                | w.crash.map_or(((st.damage >= 0.999) as u32) << 2 | ((st.upset > 0.0 || vs.is_some_and(|v| v.wipe.is_some())) as u32) << 4, |c| (c.0 & 0x14) as u32),
            handbrake: if input.handbrake { 1.0 } else { 0.0 },
            burn: vs.map_or(0.0, |v| v.burn),
            throttle: input.throttle,
            brake: input.brake,
            steer: vs.map_or(input.steer, |v| v.steer),
            water: vs.and_then(|v| v.fluid.as_ref()).map(|f| f.inside),
            race: w.race_over,
            honk: w.horn && player,
            g76: w.replay,
            nitro: vs.map(|v| v.nitro_burning),
            wheel_n: 4,
            wheels: std::array::from_fn(|i| Some(vs.map_or(0.0, |v| v.wheels[i].slide))),
            down: vs.map_or(st.wheels.iter().filter(|w| w.grounded).count(), |v| v.wheels.iter().filter(|w| w.record).count()) as i32,
            trans: Some((vs.map_or(0, |v| v.gear_change.direction as i32), st.gear + 1)),
            body: Some(([r0.x, r0.y, r0.z, r1.x, r1.y, r1.z, r2.x, r2.y, r2.z], [st.vel.x, st.vel.y, st.vel.z])),
            stick,
            goal: vs.filter(|v| player && v.pad.live).map_or(input.steer, |v| v.pad.goal),
            aero: Some(false),
            wheel: true,
            ..dm::Gather::default()
        };
        // (the damage object, vehicle +20: +44 the wipe-out / damage-out timer - the player's crash timer -, +48 the visible hit)
        let dmg = [w.crash.map_or(0.0, |c| c.1), if st.visible_hit { 1.0 } else { 0.0 }];
        DrvIn { g, needle: sp.needle, divisor: sp.divisor, out: sp.out, scale: sp.scale, racer: dm::Racer { human: player, ..dm::Racer::default() }, cutscene, look, dmg }
    }
}

/// What a car driver's gatherer wrote into the car's steering part (D +1112 / +1108 / +1156 the wheel pair's rate and scales,
/// +1148 / +1152 the hold), for main.rs to hand the car's `SteerParts` before its next step.
#[derive(Clone, Copy, Debug, Default)]
pub struct SteerOut {
    pub needle_rate: f32,
    pub needle_scale: f32,
    pub needle_scale2: f32,
    pub hold_ease: f32,
    pub hold_rate: f32,
}

impl SteerOut {
    pub fn apply(&self, d: &mut crate::steer_parts::SteerParts) {
        d.needle_rate = self.needle_rate;
        d.needle_scale = self.needle_scale;
        d.needle_scale2 = self.needle_scale2;
        d.hold_ease = self.hold_ease;
        d.hold_rate = self.hold_rate;
    }
}

/// A car driver's machine (drvmachine) and anim player (drvanim): the gatherer's own state, the steering part's +1128 /
/// +1132 (the gatherer's alone), the context, the machine, the sequencer and the pan smoother (driver +48).
struct CarAnim {
    gm: dm::GatherMut,
    parts: (f32, f32),
    ctx: dm::Ctx,
    m: dm::Machine,
    seq: da::Seq,
    pan_p: Option<(f32, f32)>,
}

/// The composer's view of a rig (drvanim `Poser`): a frame is two DOFs per bone, the rotation (a Quaternion) and the
/// translation (a Vector3).
struct RigPoser<'a>(&'a Rig);

impl da::Poser for RigPoser<'_> {
    fn len(&self) -> usize {
        2 * self.0.model.bones.len()
    }

    fn blank(&self) -> Vec<FrameDof> {
        (0..self.len()).map(|i| FrameDof { flags: if i % 2 == 0 { DOF_QUATERNION } else { DOF_VECTOR3 } | DOF_NO_VALUE, v: [0.0; 4] }).collect()
    }

    fn sample(&self, clip: da::ClipId, t: f32, out: &mut [FrameDof]) {
        let Some(c) = self.0.clips.get(clip as usize) else { return };
        let a = &c.anim;
        let (i, fr) = da::sample_pos(a, t);
        let f = c.frame(t, false);
        for b in 0..self.0.model.bones.len() {
            for (slot, k) in [(2 * b, c.rot[b]), (2 * b + 1, c.pos[b])] {
                let Some(k) = k else { continue };
                let tr = &a.tracks[k];
                let d = &mut out[slot];
                if tr.chans.is_empty() {
                    // (a clip of several blocks: the decoder's own sampling - the sampler's block step is not ported)
                    d.v = if slot % 2 == 0 {
                        tr.data.rot(f).to_array()
                    } else {
                        let v = tr.data.vec(f);
                        [v.x, v.y, v.z, 0.0]
                    };
                    d.flags &= !DOF_NO_VALUE;
                } else {
                    da::sample_track(tr, i, fr, d);
                }
            }
        }
    }

    fn rest(&self, out: &mut [FrameDof]) {
        for (b, bone) in self.0.model.bones.iter().enumerate() {
            out[2 * b] = FrameDof { flags: DOF_QUATERNION, v: bone.rot.to_array() };
            out[2 * b + 1] = FrameDof { flags: DOF_VECTOR3, v: [bone.pos.x, bone.pos.y, bone.pos.z, 0.0] };
        }
    }

    fn weights(&self, f: da::Filter) -> &[f32] {
        &self.0.dofw[match f {
            da::Filter::Spine => 0,
            da::Filter::Neck => 1,
            da::Filter::NoFace => 2,
        }]
    }
}

/// The finish into the skeleton: a DOF with a value is the bone's, one without shows the rest pose.
fn finish(frame: &[FrameDof], bones: &[crate::skin::Bone], out: &mut Vec<(Quat, Vec3)>) {
    out.clear();
    for (b, bone) in bones.iter().enumerate() {
        let (q, p) = (&frame[2 * b], &frame[2 * b + 1]);
        out.push((
            if q.flags & DOF_NO_VALUE == 0 { Quat::from_array(q.v) } else { bone.rot },
            if p.flags & DOF_NO_VALUE == 0 { Vec3::new(p.v[0], p.v[1], p.v[2]) } else { bone.pos },
        ));
    }
}

impl CarAnim {
    fn new(set: &str, rng: [u32; 2]) -> CarAnim {
        CarAnim {
            gm: dm::GatherMut::default(),
            parts: (0.0, 0.0),
            ctx: dm::Ctx { gesture: 20, wheel_scale: 1.0, ..dm::Ctx::default() },
            m: dm::Machine::new(set, rng),
            seq: da::Seq::default(),
            pan_p: None,
        }
    }

    /// One frame of the driver: the gatherer (with the car's steering-wheel shake), the context step, the machine, its
    /// commands into the sequencer, the pan smoother, the motion tree's update. Returns what the gatherer wrote into the
    /// steering part. (The order of these against each other and against the car's update is the harness's, not traced live.)
    #[allow(clippy::too_many_arguments)]
    fn step(&mut self, rig: &Rig, d: &DrvIn, k: &dm::Tune, dt: f32, odd: bool, culled: bool, rng: &mut [u32; 2], car: &str) -> Option<SteerOut> {
        self.ctx.culled = culled;
        self.ctx.opponent = !d.racer.human;
        self.gm.dmg = Some(d.dmg);
        self.gm.parts = Some(dm::Parts { f1124: d.needle, f1176: d.divisor, f1164: d.out, f1136: d.scale, f1128: self.parts.0, f1132: self.parts.1, ..dm::Parts::default() });
        let mut cc = self.ctx;
        let inp = dm::gather(
            &d.g,
            &self.ctx,
            k,
            &mut self.gm,
            dt,
            &mut |_, _| {},
            &mut |t, sm, v| {
                let (x, w) = dm::wheel_shake(&mut cc, k, t, sm, v, odd);
                (x, w, cc.side_vals[3])
            },
            &mut |_, _| d.look,
        );
        self.ctx.side_vals = cc.side_vals;
        if let Some(i) = inp {
            dm::context_step(&mut self.ctx, &i, k, dt);
        }
        let out = self.gm.parts.map(|p| {
            self.parts = (p.f1128, p.f1132);
            SteerOut { needle_rate: p.f1112, needle_scale: p.f1108, needle_scale2: p.f1156, hold_ease: p.f1148, hold_rate: p.f1152 }
        });
        // (the driver's update F-83924: the anim update F-32205 - the machine, its clips, the pan smoother, the anim player's
        // update - is not run on a frame of no time (dt under 1e-6) while the replay stream records or plays (+76 0x80 /
        // 0x40), unless the context's flags hold 0x40000 or 0x80000)
        if !(dt >= f32::from_bits(0x3586_37bd)) && self.ctx.flags & 0xC0000 == 0 && d.g.g76 & 0xC0 != 0 {
            return out;
        }
        let env = dm::Env { remaining: self.seq.remaining(), phase: self.seq.phase(), cutscene: d.cutscene };
        self.m.rng = *rng;
        let mut cmds = Vec::new();
        self.m.step(&mut self.ctx, k, &env, &d.racer, &mut cmds);
        *rng = self.m.rng;
        let id = |n: &str| -> Option<da::ClipId> {
            let r = rig.dict.get(&n.to_ascii_lowercase()).copied();
            if r.is_none() {
                static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
                let key = format!("{car} {n}");
                if let Ok(mut v) = SEEN.lock() {
                    if !v.contains(&key) {
                        log::info!("drivers: {car} has no clip {n}");
                        v.push(key);
                    }
                }
            }
            r
        };
        let dur = |c: da::ClipId| rig.clips.get(c as usize).map_or(0.0, |c| c.anim.duration);
        let look = da::Look { face: self.m.face.as_deref().and_then(id), pan_spine: None, pan: self.m.overlay.and_then(id), filters: true };
        for c in &cmds {
            let req = match c {
                dm::Cmd::Play(n, b, l, nb) => id(n).map(|x| da::play(&look, &dur, x, *b, *l, *nb)),
                dm::Cmd::PlayCut(n, b, l, nb) => id(n).map(|x| da::play_cut(x, *b, *l, *nb)),
                dm::Cmd::Pair(a, z, l, b) => id(a).zip(id(z)).map(|(x, y)| da::pair(&look, &dur, x, y, *l, *b)),
                dm::Cmd::Five(n, b, l) => {
                    let ids: Vec<_> = n.iter().filter_map(|x| id(x)).collect();
                    (ids.len() == 5).then(|| da::five(&look, &dur, [ids[0], ids[1], ids[2], ids[3], ids[4]], *b, *l))
                }
                dm::Cmd::Overlay(n, b, l) => id(n).and_then(|x| da::overlay(&mut self.seq, &look, x, *b, *l)),
                dm::Cmd::Phase(v) => {
                    self.seq.set_phase(*v);
                    None
                }
                dm::Cmd::Rate(v) => {
                    self.seq.set_rate(*v);
                    None
                }
                dm::Cmd::Rate5(v) => {
                    self.seq.set_rate5(*v);
                    None
                }
                dm::Cmd::Phase2(v) => {
                    self.seq.set_phase2(*v);
                    None
                }
                dm::Cmd::Phase5(v) => {
                    self.seq.set_phase5(*v);
                    None
                }
                dm::Cmd::PhaseAll(v) => {
                    self.seq.set_phase_all(*v);
                    None
                }
                dm::Cmd::Weight(v) => {
                    self.seq.set_weight(*v);
                    None
                }
                // (the engine and camera calls: not the driver's pose)
                dm::Cmd::EngineStart | dm::Cmd::EngineStop | dm::Cmd::HandBack => None,
            };
            if let Some(r) = req {
                self.seq.submit(&r, &dur);
            }
        }
        // the pan's smoother (driver +48, F-36491: centre 0.5, rate 5, hold 0.5) while +112 is set, then the pan clip's time
        if self.m.overlay.is_some() {
            let (p, hold) = self.pan_p.get_or_insert((0.5, -1.0));
            let target = crate::physics::exact::fma(-self.ctx.look, 0.5, 0.5);
            let eps = f32::from_bits(0x3ba3_d70a);
            if (target - 0.5).abs() < eps && (*p - 0.5).abs() < eps {
                *hold = 0.5;
            }
            if *hold >= 0.0 {
                *hold -= dt;
            }
            if *hold < 0.0 {
                *p = crate::physics::exact::fma((target - *p) * 5.0, dt, *p);
            }
            self.seq.set_pan_time(*p);
        }
        self.seq.update(dt);
        if std::env::var("MCLA_DRV_TRACE").is_ok() && d.racer.human {
            log::info!(
                "drvtrace {car} {:?} needle {:.3} div {:.3} add {:.4} speed {:.2} goal {:.3} steer {:.3} smooth {:.3} wheel {:.3} k {:?} flags {:08x} face {:?} cmds {:?}",
                self.m.state,
                d.needle,
                d.divisor,
                self.ctx.steer_add, self.ctx.speed, self.ctx.goal, self.ctx.steer, self.ctx.smooth, self.ctx.wheel_scale, self.ctx.smooth_k, self.ctx.flags, self.m.face, cmds
            );
        }
        out
    }
}

/// The wind on a rider's clothes (VSSkinned_Wind's windParams / windEmitterPlaneEq): what the draw hook keeps at +112 (the
/// phase), +116 (the strength) and +128 (the bike's velocity); the constructor (F-92182) sets all three to 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wind {
    pub phase: f32,
    pub speed: f32,
    pub vel: Vec3,
}

impl Wind {
    /// F-31704, each frame for every live driver in a vehicle (the hooks' update): on a bike the strength is
    /// clamp(speed x 2.237 x 0.0111111, 0, 1) - mph / 90 - (0 in a car, or with the debug bool "nowind" set: never in
    /// play); the phase gains dt x 1.5 x strength (fused), the velocity is kept as it is. Nothing moves while the game is
    /// paused (the game's pause byte: the hook is skipped; and the pause manager's +4 / +10 would zero the step). Not mapped:
    /// the instance's +129 byte and F-78526, which skip it too - taken as a drawn driver's.
    pub fn step(&mut self, speed: f32, vel: Vec3, bike: bool, dt: f32, paused: bool) {
        if paused {
            return;
        }
        let s = if bike {
            let k = speed * f32::from_bits(0x400f_2b02) * f32::from_bits(0x3c36_0b61);
            let k = if k >= 0.0 { k } else { 0.0 };
            if k - 1.0 >= 0.0 { 1.0 } else { k }
        } else {
            0.0
        };
        self.phase = dt.mul_add(s * 1.5, self.phase);
        self.speed = s;
        self.vel = vel;
    }

    /// F-42959's windEmitterPlaneEq: the velocity made unit by the vector unit (vmsum3fp as x + (y + z), vrsqrtefp - infinity to 0 - and
    /// one Newton step) when its square is over 1e-5, else (0, 0, -1); w = that . the car matrix's place (fused as
    /// x (z (y))).
    pub fn plane(&self, at: Vec3) -> [f32; 4] {
        let v = self.vel;
        let d = v.x * v.x + (v.y * v.y + v.z * v.z);
        let n = if d > f32::from_bits(0x3727_c5ac) {
            let e = 1.0 / d.sqrt();
            let e = if e == f32::INFINITY { 0.0 } else { e };
            v * ((1.0 - d * (e * e)) * (0.5 * e) + e)
        } else {
            Vec3::new(0.0, 0.0, -1.0)
        };
        [n.x, n.y, n.z, at.x.mul_add(n.x, at.z.mul_add(n.z, at.y * n.y))]
    }
}

/// The car's lights on its driver's draw (F-42959, the driver in a car: the vertex programs' gaugeLightPos_ConeAng
/// c209, gaugeLightColor_Falloff c210, gaugeLightConeDir c211, neonLightPos c212, neonLightColor_Falloff c213 and
/// floorPlaneEq c214). Recorded: `work/checkpoints2/cap/col4.bin` (the Saleen and an R8 at 01:06),
/// `work/intglass/cap/pov_noon.bin` (an R8 at noon), `work/police17/cap/rider_night.bin` / `rider_day.bin` (a ZX-14).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cabin {
    pub gauge_pos: Vec3,
    pub gauge_col: Vec3,
    pub cone_dir: Vec3,
    pub neon_pos: Vec3,
    pub neon_col: Vec3,
    pub floor: [f32; 4],
    /// windEmitterPlaneEq c215 and windParams c216 (phase, strength): `Wind::plane`, the hook's +112 / +116
    pub wind_eq: [f32; 4],
    pub wind: [f32; 2],
}

/// The lamp object's +408, the interior neon's flash (F-10129, every frame): (cos(rate x clock x 10) + 1) x 0.5 with the
/// record's IntNeonFlashRate (a signed byte) and the unpaused clock (0x827D7518: `App::wind_clock`), cos the guest
/// runtime's (F-63589) in double, rounded once. 1 at the rate 0.
pub fn neon_flash(rate: f32, clock: f32) -> f32 {
    (crate::physics::exact::guest_cos((rate * clock * 10.0) as f64) as f32 + 1.0) * 0.5
}

impl Cabin {
    /// From the car's matrix `m` (its body's, the vehicle's +144: the offsets are in its frame), the model tune's two
    /// offsets, the record's colours (w of the second: IntNeonIntensity), the head lamp state, whether it is a bike, the
    /// neon's flash (`neon_flash`) and the hook's wind:
    /// - gauge = m x vDriverGaugeLightOffset, colour = (c + (mean - c) x 0.5) x head x 1.0 (an image constant) with
    ///   c = DashGaugeColor and mean = (r + g) x 0.33333298 + b x 0.33333 (by day the head state and so the light are 0);
    /// - cone direction = minus the matrix's third row;
    /// - neon = m x vDriverNeonLightOffset, colour = IntNeonColor x ((lamp object +408 x +268) x IntNeonIntensity); +268
    ///   is set to 1 every update (F-10129), +408 is `flash`;
    /// - floor = (the matrix's up row, its place . up + 0.3); a bike's w is -1e5 (F-54299);
    /// - windEmitterPlaneEq = `Wind::plane` at the matrix's place, windParams = (phase, strength).
    pub fn of(m: Mat4, offs: [[f32; 3]; 2], colors: [[f32; 4]; 2], head: f32, bike: bool, flash: f32, wind: &Wind) -> Cabin {
        // (in the vector unit's order: the place, then x, y and z of the offset, each a product then a sum)
        let at = |o: [f32; 3]| m.z_axis.truncate() * o[2] + (m.y_axis.truncate() * o[1] + (m.x_axis.truncate() * o[0] + m.w_axis.truncate()));
        let up = m.y_axis.truncate();
        let p = m.w_axis.truncate();
        let c = Vec3::new(colors[0][0], colors[0][1], colors[0][2]);
        let mean = (c.x + c.y).mul_add(f32::from_bits(0x3eaa_aa9f), c.z * f32::from_bits(0x3eaa_aa3b));
        let g = (c + (Vec3::splat(mean) - c) * 0.5) * head;
        let n = Vec3::new(colors[1][0], colors[1][1], colors[1][2]) * (flash * 1.0 * colors[1][3]);
        let w = if bike { -1.0e5 } else { up.x.mul_add(p.x, p.y.mul_add(up.y, p.z * up.z)) + 0.3 };
        Cabin { gauge_pos: at(offs[0]), gauge_col: g, cone_dir: -m.z_axis.truncate(), neon_pos: at(offs[1]), neon_col: n, floor: [up.x, up.y, up.z, w], wind_eq: wind.plane(p), wind: [wind.phase, wind.speed] }
    }
}

struct Rig {
    model: Arc<SkinModel>,
    steer: Clip,
    idle_stop: Option<Clip>,
    reverse: Option<Clip>,
    /// the pack's gesture clips by their name after the car prefix ("drv_gest_listen_lft", "drv_win",
    /// "drv_gest_jawflap"); the car's own first, the shared Challenger ones otherwise
    gestures: HashMap<String, Clip>,
    /// DRV_HIT_HRD's length (the wreck's hit, F-47123's machine: then 4 s of DRV_IDLE_START)
    hit_hard: Option<f32>,
    /// a rider's eject clips by their name after the bike's prefix ("drv_eject_l", rider_eject.rs)
    ejects: HashMap<String, Clip>,
    /// a rider's clips of the bike pose machine's riding states (`BikeClips`)
    bike: Option<BikeClips>,
    /// a car driver's dictionary (machine +24, its lookup F-66148 case-blind): every clip of the pack by its name after the
    /// car's prefix (the car's own over the unprefixed ones over the Challenger's), and the clips
    dict: HashMap<String, u32>,
    clips: Vec<Clip>,
    /// the filters' weight per DOF (two per bone): +92 spine00 down, +96 neck down, +100 all but Facial_root's subtree
    dofw: [Vec<f32>; 3],
}

/// The clips of mcMotorcycleAnimMachine's riding states (`BikeClips::load` names them; `_ck`: the cockpit camera's
/// `_ckpt` variant): start (F-88016), cruise (F-27733), stopped idle and burnout centre (F-73617), tuck (F-14953), wheelie
/// (F-33885), endo (F-23193), burnout left / right (F-13481 / F-26376), nos boost (F-22152), shift (F-54589), reverse
/// (F-93384) and fidget (F-87252).
struct BikeClips {
    idle_low: Option<Clip>,
    low: Clip,
    fst: Clip,
    low_ck: Option<Clip>,
    fst_ck: Option<Clip>,
    stp: Option<Clip>,
    stp_ck: Option<Clip>,
    low_fwd: Option<Clip>,
    fst_fwd: Option<Clip>,
    wheelie: Option<Clip>,
    wheelie_ck: Option<Clip>,
    endo: Option<Clip>,
    endo_ck: Option<Clip>,
    burn_l: Option<Clip>,
    burn_r: Option<Clip>,
    nos: Option<Clip>,
    nos_ck: Option<Clip>,
    shft_low: Option<Clip>,
    shft_low_ck: Option<Clip>,
    rev: Option<Clip>,
    rev_ck: Option<Clip>,
    rev_over: Option<Clip>,
    /// the fidget's two clips (the picker's table: `drv_gest_look_lft`, `drv_gest_look_rgt`)
    look: [Option<Clip>; 2],
    /// `DRV_PAN_SWEEP_OVERLAY`, the head's pan (F-56423 looks it up for the local human only)
    pan: Option<Clip>,
}

impl BikeClips {
    fn load(clip: impl Fn(&[&str]) -> Option<Clip>) -> Option<BikeClips> {
        Some(BikeClips {
            idle_low: clip(&["drv_idle_low"]),
            low: clip(&["drv_low_lean_sweep"])?,
            fst: clip(&["drv_fst_lean_sweep"])?,
            low_ck: clip(&["drv_low_lean_sweep_ckpt"]),
            fst_ck: clip(&["drv_fst_lean_sweep_ckpt"]),
            stp: clip(&["drv_idle_stp_sweep"]),
            stp_ck: clip(&["drv_idle_stp_sweep_ckpt"]),
            low_fwd: clip(&["drv_low_leanfwd_sweep"]),
            fst_fwd: clip(&["drv_fst_leanfwd_sweep"]),
            wheelie: clip(&["drv_wheelie"]),
            wheelie_ck: clip(&["drv_wheelie_ckpt"]),
            endo: clip(&["drv_endo"]),
            endo_ck: clip(&["drv_endo_ckpt"]),
            burn_l: clip(&["drv_burnout_l"]),
            burn_r: clip(&["drv_burnout_r"]),
            nos: clip(&["drv_nos_lean_sweep"]),
            nos_ck: clip(&["drv_nos_lean_sweep_ckpt"]),
            shft_low: clip(&["drv_shft_low_overlay"]),
            shft_low_ck: clip(&["drv_shft_low_ckpt_overlay"]),
            rev: clip(&["drv_rev_sweep"]),
            rev_ck: clip(&["drv_rev_sweep_ckpt"]),
            rev_over: clip(&["drv_rev_overlay"]),
            look: [clip(&["drv_gest_look_lft"]), clip(&["drv_gest_look_rgt"])],
            pan: clip(&["drv_pan_sweep_overlay"]),
        })
    }
}

/// What the rider machine's gatherer (F-28290, through F-71604) hands its states each frame, mapped onto the port's car (main.rs).
#[derive(Clone, Copy, Debug, Default)]
pub struct RideIn {
    /// CockpitCamActive (bit 21): the local human's drive view is the cockpit one
    pub ck: bool,
    /// ReadSpeed, ctx+96: the body's velocity along its forward axis (m/s)
    pub speed: f32,
    /// ReadSmoothedSteeringParam, ctx+28 (= ctx+44, the phase input): the car-parts object's eased steering over its
    /// scale, D +1164 / D +1136 (steer_parts.rs)
    pub x: f32,
    /// ctx+92: the throttle (chassis +136), negated while reversing
    pub throttle: f32,
    /// ReadGoalSteering, ctx+20: the racer's virtual +172 (a player: its pad's steering target, input +84, F-91694)
    pub goal_steer: f32,
    /// IsTuck (bit 23): the aero's byte +40 (a bike's DragMin lean), forward speed over 0.5 and no brake (chassis +140)
    pub tuck: bool,
    /// IsWheelie (bit 5) / IsEndo (bit 6): the body's back axis y below -sin 20 deg / above it, with the rider's stick
    /// (input +32) over 0.5 / under -0.5
    pub wheelie: bool,
    pub endo: bool,
    /// IsBurnout (bit 8): the handbrake (chassis +144) on and the burnout charge (chassis +580) at 1 or more
    pub burnout: bool,
    /// IsNitroBurn (bit 4): mcCarNitro +32
    pub nitro: bool,
    /// IsReversing (bit 7): the gearbox in reverse and forward speed under -0.5
    pub reversing: bool,
    /// AreAllWheelsOnGround (bit 30)
    pub all_down: bool,
    /// ctx+124: the gearbox's shift direction (transmission +24), +1 up, -1 down
    pub shift: i8,
    /// IsCutscenePlaying
    pub cutscene: bool,
    /// IsOpponentDefined, ctx byte +49: the driver's record byte 0 is clear (taken as: not the local human)
    pub opponent: bool,
    /// ctx+120, the look the pan-sweep overlay turns the head to (-1 .. 1; main.rs `rider_look` for the local human)
    pub look: f32,
}

impl RideIn {
    /// The gatherer's flags (F-28290) from the port's car: the body's axes and velocity, the pedals the car was handed, the
    /// gearbox, the burnout charge, the nitrous and the wheel records, and the steering of the car-parts object (`sp`: D +1164 /
    /// +1136, the bike branch of F-28290). Approximate (docs/PEOPLE_TRAFFIC.md): the aero's byte +40 from the stick and the
    /// car's air time (the chassis +584 of the tick before), a computer rider's goal steering from its steering (its input's
    /// +152 is not kept apart) and IsOpponentDefined from "not the local human".
    pub fn of(st: &crate::physics::CarState, input: &crate::physics::CarInput, sp: &crate::steer_parts::SteerParts, player: bool, cutscene: bool) -> RideIn {
        // (sin of the double 0.34906587 through the guest's sin, rounded to float: 20 degrees)
        let sin20 = crate::physics::exact::guest_sin(f64::from_bits(0x3fd6_5718_6000_0000)) as f32;
        let back = st.rot * Vec3::Z;
        let speed = -back.dot(st.vel);
        let vs = st.veh.as_deref();
        let stick = vs.filter(|v| v.pad.live).map_or(input.lean, |v| v.pad.stick_y);
        let reversing = st.gear < 0 && speed < -0.5;
        RideIn {
            ck: false,
            speed,
            x: sp.out / sp.scale,
            throttle: if reversing { -input.throttle } else { input.throttle },
            goal_steer: vs.filter(|v| player && v.pad.live).map_or(input.steer, |v| v.pad.goal),
            tuck: stick < -0.5 && st.airborne_time < 0.5 && speed > 0.5 && input.brake <= 0.0,
            wheelie: back.y < -sin20 && stick > 0.5,
            endo: back.y > sin20 && stick < -0.5,
            burnout: input.handbrake && vs.map_or(false, |v| v.burn >= 1.0),
            nitro: vs.map_or(false, |v| v.nitro_burning),
            reversing,
            all_down: vs.map_or(st.wheels[0].grounded && st.wheels[2].grounded, |v| v.wheels[0].record && v.wheels[2].record),
            shift: vs.map_or(0, |v| v.gear_change.direction),
            cutscene,
            opponent: !player,
            look: 0.0,
        }
    }
}

/// The states of the riding part of the graph (F-71745).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum St {
    #[default]
    Start,
    Stopped,
    Cruise,
    Tuck,
    Wheelie,
    Endo,
    BurnC,
    BurnL,
    BurnR,
    Nos,
    Shift,
    Reverse,
    Fidget,
}

/// A rider in the bike pose machine: its state, the cockpit flag of the last frame (its change is the edge the enters and the
/// self-transitions test), the crossfade of the last state change and the pose it fades from, the machine's timers A / B
/// (ctx+148 / +152), the time in the state (for the clips that play), the fidget counter (state +96) and the shift's clips.
#[derive(Default)]
struct Ride {
    state: St,
    ck: bool,
    fade: f32,
    rate: f32,
    /// the reverse enter's second request (its overlay) and its crossfade, over the tree the first request left
    fade2: Option<(f32, f32)>,
    /// the old tree under the crossfade: the start action's looping `drv_idle_low` (its time) or the frozen pose
    from_idle: Option<f32>,
    from: Vec<(Quat, Vec3)>,
    last: Vec<(Quat, Vec3)>,
    timer_a: f32,
    timer_b: f32,
    t: f32,
    fidgets: i32,
    /// the shift's base pair (the cruise pair's clips left in slots +44 / +48: their cockpit flag) and its overlay's flag
    shift_base_ck: bool,
    shift_ck: bool,
    /// the sequencer's +112: the pan-sweep overlay is part of the tree the last enter requested
    pan: bool,
    /// the driver's +48 smoother (F-45732: centre 0.5, rate 5, hold 0.5 s): its value +12 and hold timer +16, made on first use
    pan_p: Option<(f32, f32)>,
}

/// The human rider's look at a racer beside him (the driver's +380 state; F-70536 with the test F-48336), which the gatherer
/// (F-28290) makes ctx+120 from: the driver constructor's range 20 m, angle 45 degrees, look 1 s and hold 5 s.
#[derive(Default, Clone, Copy, Debug)]
pub struct RivalLook {
    /// +0: the racer looked at (an index of the race's cars), +4 the look to it, +8 the hold timer
    rival: Option<usize>,
    look: f32,
    timer: f32,
}

impl RivalLook {
    /// F-70536: with no racer held, the nearest other racer (by plan distance) within 20 m that lies more to a side than
    /// ahead or behind (|atan2(|back|, |right|)| under 45 degrees: F-48336, left side tested first) is held for 5 s and the
    /// look is +1 for one on the left, -1 on the right; else `look` (the camera's). Holding, the timer runs down by `dt`: the
    /// held look for its first second, then `look`, and the racer is let go when it is below 0. `me`: the car's place and
    /// rotation; `others`: the race's other racers that drive (state +656 == 3, F-90764), in the race's order.
    pub fn step(&mut self, look: f32, dt: f32, me: (Vec3, Quat), others: impl Iterator<Item = (usize, Vec3)>) -> f32 {
        if self.rival.is_none() {
            let (right, back) = (me.1 * Vec3::X, me.1 * Vec3::Z);
            let near = |p: Vec3, side: f32| -> f32 {
                let v = p - me.0;
                let (x, z) = (right.dot(v), back.dot(v));
                if x * side < 0.0 {
                    return -1.0;
                }
                let d2 = crate::physics::exact::fma(v.z, v.z, v.x * v.x);
                if d2 >= 20.0 * 20.0 {
                    return -1.0;
                }
                let a = crate::physics::exact::guest_atan2(z.abs() as f64, x.abs() as f64) as f32;
                if a.abs() >= f32::from_bits(0x3f49_0fdb) {
                    return -1.0;
                }
                d2
            };
            let (mut best, mut pick, mut out) = (f32::MAX, None, look);
            for (k, p) in others {
                for (side, l) in [(-1.0, 1.0), (1.0, -1.0)] {
                    let d = near(p, side);
                    if d > -1.0 && d < best {
                        (best, pick, out) = (d, Some(k), l);
                    }
                }
            }
            if best < f32::MAX {
                (self.rival, self.look, self.timer) = (pick, out, 5.0);
            }
            return out;
        }
        self.timer -= dt;
        if self.timer > 5.0 - 1.0 {
            return self.look;
        }
        if self.timer < 0.0 {
            self.rival = None;
        }
        look
    }
}

/// The cruise state's steering to sweep phase (F-25740): slope 0.75 within |x| <= 0.5, 0.25 outside.
fn ride_remap(x: f32) -> f32 {
    if x.abs() <= 0.5 {
        (x * 1.5 + 1.0) * 0.5
    } else {
        let side = if x >= 0.0 { 1.0 } else { -1.0 };
        (side * crate::physics::exact::fma(x.abs() - 0.5, 0.5, 0.75) + 1.0) * 0.5
    }
}

/// The weight of the fast sweep (the cruise, tuck and shift updates): clamp((speed - 17.8816) x 0.2236936, 0, 1).
fn ride_speed_weight(speed: f32) -> f32 {
    let k = (speed - 17.881_6) * 0.223_693_64;
    let k = if k >= 0.0 { k } else { 0.0 };
    if k - 1.0 >= 0.0 { 1.0 } else { k }
}

/// Add `ov` onto `base` on the bones `clip` animates (crmtRequestAddSubtract at weight 1, reanimate_slot.md: rotation
/// `ov * base`, translation summed).
fn add_pose(base: &mut [(Quat, Vec3)], ov: &[(Quat, Vec3)], clip: &Clip) {
    for i in 0..base.len().min(ov.len()) {
        if clip.rot.get(i).map_or(false, |r| r.is_some()) {
            base[i].0 = ov[i].0 * base[i].0;
        }
        if clip.pos.get(i).map_or(false, |p| p.is_some()) {
            base[i].1 += ov[i].1;
        }
    }
}

/// One frame of the bike pose machine's riding states for a rider (`Ride`), the pose into `out`. The timers gain dt first
/// (the gatherer's context step), then the current state's transitions are tested in the graph's order (F-71745; impact,
/// damageout, gesture switch and suspended are left to rider_eject.rs, the scene gestures over the pose and the far freeze);
/// the first that holds runs its exit and the new state's enter; then the state's update poses:
/// - start: to cruise at once, its action playing `drv_idle_low` looped with no blend (F-88016); the cruise enter fades from it.
/// - cruise (enter F-27733: timer A = 0, 0.45 s; update F-61324): the low / fast lean sweeps as a pair at the fast weight, both
///   at the remapped steering phase; timer B = 0 while IsBurnout.
/// - stopped idle and burnout centre (enter F-73617: timers A and B = 0, 0.35 s): `drv_idle_stp_sweep` at clamp(x, -0.5, 0.5)
///   + 0.5 (F-63745) / at the remapped phase (burnout centre runs the cruise update).
/// - tuck (F-14953 / F-77590, 0.25 s): the leanFwd sweeps (cockpit: the lean sweeps) as a pair at the fast weight.
/// - wheelie (F-33885 / F-56690, 0.25 s, timer A = 0): `drv_wheelie` and the low sweep, B's weight timer A - 1 (0 under 1).
/// - endo (F-23193 / F-14383), burnouts left / right (F-13481 / F-26376; cockpit: the stopped sweep), nos boost (F-22152 /
///   F-23398): one clip at the remapped phase, 0.25 s.
/// - shift (F-54589 / F-32007, 0.25 s; the pair and overlay F-42342): the pair the cruise left, plus `drv_shft_low_overlay` (cockpit: `_ckpt_overlay`) added
///   over it, playing from 0 at rate 1 (its pair's weight stays 0, so the fast overlay never shows); leaves when less than
///   0.25 s of it is left (F-38907, F-70114).
/// - reverse (F-93384 / F-30945): `drv_rev_sweep` at (x + 1) x 0.5 over 0.35 s, then at once `drv_rev_overlay` added over it,
///   looped at rate 1.5, over 0.25 s on top of the first request (F-85022).
/// - fidget (F-87252, exit F-53813: timer A = 0): `drv_gest_look_rgt`, then `_lft`, alternating (F-61316), from 0 at rate 1, 0.25 s.
/// An enter's blend is 0.01 s when the cockpit flag changed this frame (all but the shift's and the reverse overlay's). A state
/// change fades linearly (rate 1 / blend time, the first frame at rate x dt) from the old tree, which stays frozen (the start's
/// idle keeps looping).
/// The pan-sweep overlay (F-18940's branch with +112): the cruise, stopped idle and burnout centre enters put it in the local
/// human's tree outside the cockpit (F-63058 with F-56423; the start action and the stopped idle and fidget exits arm it too,
/// but the next enter decides), every other enter takes it out. In the tree it is added (AddSubtract at weight 1) on the neck
/// and its children (filter +96) at a phase the driver's smoother sets each frame (F-32205: F-36491 towards F-66750's
/// 0.5 - ctx+120 x 0.5, then F-47264: time = clamp(p x duration, 0, duration)); the smoother runs only while it is armed.
#[allow(clippy::too_many_arguments)]
fn ride_pose<'a>(r: &mut Ride, b: &'a BikeClips, bones: &[crate::skin::Bone], neck: &[bool], i: &RideIn, time: f32, dt: f32, out: &mut Vec<(Quat, Vec3)>, tmp: &mut Vec<(Quat, Vec3)>) {
    const STOPPED: f32 = 0.15 * 0.44704;
    let ck = i.ck;
    let ck_before = r.ck;
    let edge = ck != r.ck && r.state != St::Start;
    r.ck = ck;
    r.timer_a += dt;
    r.timer_b += dt;
    r.t += dt;
    let pick = |c: &'a Option<Clip>, d: &'a Option<Clip>| if ck { c.as_ref().or(d.as_ref()) } else { d.as_ref() };
    // what is left of the fidget's clip / the shift's overlay (ReadTimeRemaining / ReadTimeRemainingOverlay: -1 with none)
    let left = |c: Option<&Clip>, t: f32| c.map_or(-1.0, |c| (c.anim.duration - t).max(0.0));
    let look = |r: &Ride| b.look[(r.fidgets % 2) as usize].as_ref();
    let shift_over = |r: &Ride| if r.shift_ck { b.shft_low_ck.as_ref().or(b.shft_low.as_ref()) } else { b.shft_low.as_ref() };
    let rolling = i.speed > STOPPED + 0.5 && !i.burnout;
    let burn_side = |i: &RideIn| if i.burnout && i.goal_steer < -0.5 { Some(St::BurnL) } else if i.burnout && i.goal_steer > 0.5 { Some(St::BurnR) } else { None };
    let next = match r.state {
        St::Start => Some(St::Cruise),
        St::Stopped => {
            if let Some(s) = burn_side(i) {
                Some(s)
            } else if i.burnout {
                Some(St::BurnC)
            } else if i.reversing {
                Some(St::Reverse)
            } else if rolling {
                Some(St::Cruise)
            } else if r.timer_a > 15.0 && i.x.abs() < 0.1 && !i.cutscene && !i.opponent && !ck {
                Some(St::Fidget)
            } else if edge {
                Some(St::Stopped)
            } else {
                None
            }
        }
        St::Cruise => {
            if i.nitro && r.timer_a > 0.25 && !i.opponent {
                Some(St::Nos)
            } else if i.wheelie {
                Some(St::Wheelie)
            } else if i.endo {
                Some(St::Endo)
            } else if i.tuck && r.timer_a > 1.5 {
                Some(St::Tuck)
            } else if let Some(s) = burn_side(i) {
                Some(s)
            } else if i.burnout {
                Some(St::BurnC)
            } else if i.shift != 0 && !ck {
                Some(St::Shift)
            } else if i.speed <= STOPPED {
                Some(St::Stopped)
            } else if edge {
                Some(St::Cruise)
            } else {
                None
            }
        }
        St::Tuck => if !i.tuck { Some(St::Cruise) } else if edge { Some(St::Tuck) } else { None },
        St::Wheelie => if i.all_down { Some(St::Cruise) } else if edge { Some(St::Wheelie) } else { None },
        St::Endo => if !i.endo { Some(St::Cruise) } else if edge { Some(St::Endo) } else { None },
        St::BurnC => {
            if i.burnout && i.goal_steer < -0.5 && r.timer_a > 0.75 {
                Some(St::BurnL)
            } else if i.burnout && i.goal_steer > 0.5 && r.timer_a > 0.75 {
                Some(St::BurnR)
            } else if !i.burnout && r.timer_b > 0.5 {
                Some(St::Cruise)
            } else if edge {
                Some(St::BurnC)
            } else {
                None
            }
        }
        St::BurnL => if i.burnout && i.goal_steer > -0.5 { Some(St::BurnC) } else if !i.burnout { Some(St::Cruise) } else if edge { Some(St::BurnL) } else { None },
        St::BurnR => if i.burnout && i.goal_steer < 0.5 { Some(St::BurnC) } else if !i.burnout { Some(St::Cruise) } else if edge { Some(St::BurnR) } else { None },
        St::Nos => if !i.nitro { Some(St::Cruise) } else if edge { Some(St::Nos) } else { None },
        St::Shift => if left(shift_over(r), r.t) < 0.25 || edge { Some(St::Cruise) } else { None },
        St::Reverse => {
            if i.throttle > 0.0 && i.speed > 0.0 {
                Some(St::Cruise)
            } else if !i.reversing && i.speed.abs() < 0.02 && i.x.abs() < 0.1 {
                Some(St::Stopped)
            } else if edge {
                Some(St::Reverse)
            } else {
                None
            }
        }
        St::Fidget => {
            if i.reversing {
                Some(St::Reverse)
            } else if left(look(r), r.t) < 0.033 || i.x.abs() > 0.1 || i.cutscene {
                Some(St::Stopped)
            } else if rolling || edge {
                Some(St::Cruise)
            } else {
                None
            }
        }
    };
    if let Some(n) = next {
        // (the fidget's exit: timer A = 0)
        if r.state == St::Fidget {
            r.timer_a = 0.0;
        }
        let blend = match n {
            St::Cruise => 0.45,
            St::Stopped | St::BurnC | St::Reverse => 0.35,
            _ => 0.25,
        };
        r.from_idle = if r.state == St::Start { Some(time) } else { None };
        r.from = r.last.clone();
        r.fade = 0.0;
        r.rate = 1.0 / if edge && n != St::Shift { 0.01 } else { blend };
        r.fade2 = if n == St::Reverse { Some((0.0, 1.0 / 0.25)) } else { None };
        r.t = 0.0;
        match n {
            St::Cruise | St::Wheelie => r.timer_a = 0.0,
            St::Stopped | St::BurnC => (r.timer_a, r.timer_b) = (0.0, 0.0),
            St::Fidget => r.fidgets += 1,
            // (the shift rebuilds the pair from the clips the cruise enter put in the slots - its cockpit flag then)
            St::Shift => {
                if r.state != St::Shift {
                    r.shift_base_ck = ck_before;
                }
                r.shift_ck = ck;
            }
            _ => {}
        }
        r.state = n;
        r.pan = matches!(n, St::Cruise | St::Stopped | St::BurnC) && !ck && !i.opponent && b.pan.is_some();
    }
    // the pan's smoother (F-36491 on the driver's +48: centre 0.5, rate 5, hold 0.5): back at the centre with the target there
    // the hold restarts; while it runs nothing moves
    let pan_t = if r.pan {
        let (p, hold) = r.pan_p.get_or_insert((0.5, -1.0));
        let target = crate::physics::exact::fma(-i.look, 0.5, 0.5);
        let eps = f32::from_bits(0x3ba3_d70a);
        if (target - 0.5).abs() < eps && (*p - 0.5).abs() < eps {
            *hold = 0.5;
        }
        if *hold >= 0.0 {
            *hold -= dt;
        }
        if *hold < 0.0 {
            *p = crate::physics::exact::fma((target - *p) * 5.0, dt, *p);
        }
        b.pan.as_ref().map(|c| {
            let d = c.anim.duration;
            let t = d * *p;
            let t = if t >= 0.0 { t } else { 0.0 };
            (c, if t - d >= 0.0 { d } else { t })
        })
    } else {
        None
    };
    if r.state == St::Cruise && i.burnout {
        r.timer_b = 0.0;
    }
    r.fade = (r.fade + r.rate * dt).min(1.0);
    if let Some((f, k)) = r.fade2.as_mut() {
        *f = (*f + *k * dt).min(1.0);
    }
    let ph = ride_remap(i.x);
    let w = ride_speed_weight(i.speed);
    let pair = |a: &Clip, c: &Clip, w: f32, out: &mut Vec<(Quat, Vec3)>, tmp: &mut Vec<(Quat, Vec3)>| {
        a.local(bones, ph * a.last_frame(), out);
        c.local(bones, ph * c.last_frame(), tmp);
        blend_pose(out, tmp, w);
    };
    let one = |c: &Clip, ph: f32, out: &mut Vec<(Quat, Vec3)>| c.local(bones, ph * c.last_frame(), out);
    let cruise_pair = |ck: bool| if ck { (b.low_ck.as_ref().unwrap_or(&b.low), b.fst_ck.as_ref().unwrap_or(&b.fst)) } else { (&b.low, &b.fst) };
    let stp = pick(&b.stp_ck, &b.stp).unwrap_or(&b.low);
    let mut under: Option<Vec<(Quat, Vec3)>> = None;
    match r.state {
        St::Start | St::Cruise => {
            let (lo, fs) = cruise_pair(ck);
            pair(lo, fs, w, out, tmp);
        }
        St::Stopped => one(stp, i.x.clamp(-0.5, 0.5) + 0.5, out),
        St::BurnC => one(stp, ph, out),
        St::Tuck => {
            let (lo, fs) = if ck { cruise_pair(true) } else { (b.low_fwd.as_ref().unwrap_or(&b.low), b.fst_fwd.as_ref().unwrap_or(&b.fst)) };
            pair(lo, fs, w, out, tmp);
        }
        St::Wheelie => {
            let (a, c) = if ck {
                let c = b.wheelie_ck.as_ref().unwrap_or(&b.low);
                (c, c)
            } else {
                (b.wheelie.as_ref().unwrap_or(&b.low), &b.low)
            };
            let k = r.timer_a - 1.0;
            pair(a, c, if k > 0.0 { k.min(1.0) } else { 0.0 }, out, tmp);
        }
        St::Endo => one(pick(&b.endo_ck, &b.endo).unwrap_or(&b.low), ph, out),
        St::BurnL => one(if ck { stp } else { b.burn_l.as_ref().unwrap_or(&b.low) }, ph, out),
        St::BurnR => one(if ck { stp } else { b.burn_r.as_ref().unwrap_or(&b.low) }, ph, out),
        St::Nos => one(pick(&b.nos_ck, &b.nos).unwrap_or(&b.low), ph, out),
        St::Shift => {
            let (lo, fs) = cruise_pair(r.shift_base_ck);
            pair(lo, fs, w, out, tmp);
            if let Some(o) = shift_over(r) {
                o.local(bones, o.frame(r.t, false), tmp);
                add_pose(out, tmp, o);
            }
        }
        St::Reverse => {
            one(pick(&b.rev_ck, &b.rev).unwrap_or(&b.low), (i.x + 1.0) * 0.5, out);
            under = Some(out.clone());
            if let Some(o) = b.rev_over.as_ref() {
                o.local(bones, o.frame(r.t * 1.5, true), tmp);
                add_pose(out, tmp, o);
            }
        }
        St::Fidget => match look(r) {
            Some(c) => c.local(bones, c.frame(r.t, false), out),
            None => one(stp, 0.5, out),
        },
    }
    if let Some((c, t)) = pan_t {
        c.local(bones, c.frame(t, false), tmp);
        for k in 0..out.len().min(tmp.len()).min(neck.len()) {
            if !neck[k] {
                continue;
            }
            if c.rot.get(k).map_or(false, |x| x.is_some()) {
                out[k].0 = tmp[k].0 * out[k].0;
            }
            if c.pos.get(k).map_or(false, |x| x.is_some()) {
                out[k].1 += tmp[k].1;
            }
        }
    }
    // the crossfades: from the frozen old tree (or the start's looping idle) into the new one; the reverse's first request
    // (its sweep alone) fades in under its second
    if r.fade < 1.0 || r.fade2.map_or(false, |(f, _)| f < 1.0) {
        match (r.from_idle, b.idle_low.as_ref()) {
            (Some(t0), Some(idle)) => idle.local(bones, idle.frame(time - t0, true), tmp),
            _ => tmp.clone_from(&r.from),
        }
        if tmp.len() == out.len() {
            if let (Some((f2, _)), Some(u)) = (r.fade2, under) {
                blend_pose(tmp, &u, r.fade);
                blend_pose(tmp, out, f2);
            } else {
                blend_pose(tmp, out, r.fade);
            }
            out.clone_from(tmp);
        }
    }
    r.last.clone_from(out);
}

/// A rider off his seat (GitHub #39, rider_eject.rs, bike_rider_ragdoll.md 2 / 5).
enum Off {
    /// "impact eject ...": the clip at its rate (F-53423), in the bike's space as the riding clips
    Eject { clip: String, t: f32, rate: f32 },
    /// the clip's time is up (F-41119): its last frame, and the character matrix then, wait for GoRagdoll (F-54692)
    Ready { local: Vec<(Quat, Vec3)>, mat: Mat4 },
    /// "impact ragdoll": the bones in the world from the ragdoll (F-46606), kept from frame to frame
    Ragdoll { world: Vec<Mat4> },
}

/// The gesture a scene event asks of a driver (racer+684; the event ids of table 0x827EAD60).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Neutral,
    Left,
    Right,
    TalkLeft,
    TalkRight,
    Mild,
    Win,
    ListenLeft,
    ListenRight,
    ThumbDownLeft,
    ThumbDownRight,
    /// the arrests' `gesture_cophandsup` (gesture 18): hands up
    CopHandsUp,
    /// the ticket scenes' `gesture_copfidget` (gesture 17, F-41149)
    CopFidget,
}

impl Gesture {
    /// From a scene event's name (the first entry of the event table the name contains).
    pub fn from_event(name: &str) -> Option<Gesture> {
        let n = name.to_ascii_lowercase();
        // (table order: neutral, left, right, talk_left, talk_right, mild, win, listen_left, listen_right,
        // thm_dwn_lft, thm_dwn_rgt - "gesture_talk_left" contains "left"; the original's first match is by
        // the full event names, so the longer names are tested first here)
        Some(match n.trim_start_matches("gesture_") {
            "neutral" => Gesture::Neutral,
            "left" => Gesture::Left,
            "right" => Gesture::Right,
            "talk_left" => Gesture::TalkLeft,
            "talk_right" => Gesture::TalkRight,
            "mild" => Gesture::Mild,
            "win" => Gesture::Win,
            "listen_left" => Gesture::ListenLeft,
            "listen_right" => Gesture::ListenRight,
            "thm_dwn_lft" => Gesture::ThumbDownLeft,
            "thm_dwn_rgt" => Gesture::ThumbDownRight,
            "cophandsup" => Gesture::CopHandsUp,
            "copfidget" => Gesture::CopFidget,
            _ => return None,
        })
    }
}

/// A gesture being played (the driver graph's "gesture switch" state, F-20767).
#[derive(Clone)]
struct Playing {
    clip: String,
    t: f32,
    /// the event's end time from now (its end call sets the gesture back to none), else infinite
    left: f32,
}

/// Per driver: the gesture's round-robin counters (ctx+68..+92), what plays, the jaw flap.
#[derive(Default, Clone)]
struct Acting {
    rr: [usize; 7],
    playing: Option<Playing>,
    /// a gesture asked for, its clip chosen when the driver is next posed
    asked: Option<(Gesture, f32, bool)>,
    /// fading out of a gesture that ended: (clip, its time - it plays on under the fade, weight)
    leaving: Option<(String, f32, f32)>,
    jaw: bool,
    jaw_k: f32,
    jaw_t: f32,
}

#[derive(Default, Clone, Copy)]
struct Pose {
    steer: f32,
    stopped: f32,
    reverse: f32,
    seen: u32,
}

#[derive(Default)]
pub struct Drivers {
    char_dir: PathBuf,
    /// driver sets: (name, bike rider, police, one the port picks for an unnamed driver)
    sets: Vec<(String, bool, bool, bool)>,
    models: HashMap<usize, Option<Arc<SkinModel>>>,
    /// (car key, the local human's pack, the female pack, driver set)
    rigs: HashMap<(String, bool, bool, usize), Option<Rig>>,
    poses: HashMap<usize, Pose>,
    buf: SkinBuffer,
    /// the player's head in car space (for the cockpit camera)
    pub player_head: Option<Vec3>,
    /// the player's driver as the view draws him (the draw mode, F-60712): 0 the full model, 1 the set's
    /// cockpit view model (F-56050), 2 none (looking back from the POV view, the bumper view - F-14542)
    pub cockpit: u8,
    /// the human's drive view is the cockpit one (the camera manager's view +80 == 1): the rider machine's CockpitCamActive
    pub ckpt_view: bool,
    rides: HashMap<usize, Ride>,
    time: f32,
    frame: u32,
    acting: HashMap<usize, Acting>,
    off: HashMap<usize, Off>,
    /// the character set each seat was last drawn with (its voice: CareerVoices.xml)
    pub set_of: HashMap<usize, String>,
    /// the colours of a car's own set-up (`<car>.xcc`) for a car that wears none (`Seat::colors`)
    own_colors: HashMap<String, [[f32; 4]; 2]>,
    data_dir: PathBuf,
    /// per car (`Seat::uid`): the distance LOD's freeze byte (input context +48, `freeze_step`)
    frozen: HashMap<u32, bool>,
    /// per car (`Seat::uid`): its driver's draw hook's wind (`Wind`), from the hook's creation on
    wind: HashMap<u32, Wind>,
    /// the unpaused clock (0x827D7518, `App::wind_clock`) and whether the game is paused - set before `prepare`
    pub clock: f32,
    pub paused: bool,
    /// per car (`Seat::uid`): its driver's machine and anim player (`MCLA_DRV_OLD=1`: the old direct pose)
    cars: HashMap<u32, CarAnim>,
    /// the shared generator F-84930 the machines draw from (a global of the original; its start value is not read: approximate)
    rng: [u32; 2],
    /// per car (`Seat::uid`): what its driver's gatherer wrote into the steering part this frame (main.rs hands it on)
    pub steer_out: HashMap<u32, SteerOut>,
    /// the cars whose visible hit (mcCarDamage +48) a driver's gatherer took this frame
    pub hit_taken: Vec<u32>,
}

/// What the drivers' distance LOD (F-12028) reads of one racer's driver.
#[derive(Clone, Copy, Default)]
pub struct FreezeIn {
    /// racer +421: the car's instance is up (set by the input gatherer F-28290)
    pub active: bool,
    /// the scene-playing byte (a story scene from its start to FinishScene)
    pub scene: bool,
    /// racer +145 / +147: the draw gate's show flags (F-14542)
    pub show: bool,
    pub show_scene: bool,
    /// the no-damage mode byte and racer +370 (the draw gate's other test)
    pub no_damage: bool,
    pub b370: bool,
    /// racer +96: the racer has a car
    pub car: bool,
    /// a cine script running (F-52539)
    pub cine: bool,
    /// the car is the player's (F-65988)
    pub player: bool,
    /// the car's state +592 has 0x10 (wiped out) or 0x04 (damaged out)
    pub wrecked: bool,
    /// the player's car's place (x, z), None without one: then the camera's
    pub reference: Option<[f32; 2]>,
    pub camera: [f32; 2],
    pub pos: [f32; 2],
}

/// F-12028: the freeze byte after one update and whether it reset the inputs (F-55405, once on freezing). The gates above
/// leave the byte as it is; no car freezes; a cine script or a wreck thaws; the player's car is never touched; otherwise
/// frozen while the car is more than 20 m from the reference (x / z, d^2 > 400: dz^2 fused onto dx^2 in double, rounded
/// to single - the fmadds).
pub fn freeze_step(frozen: bool, f: &FreezeIn) -> (bool, bool) {
    if !f.active || f.scene || !(f.show || f.show_scene) || (f.no_damage && !f.show_scene && f.b370) {
        return (frozen, false);
    }
    if !f.car {
        return (true, false);
    }
    if f.cine {
        return (false, false);
    }
    if f.player {
        return (frozen, false);
    }
    let r = f.reference.unwrap_or(f.camera);
    let (dx, dz) = (f.pos[0] - r[0], f.pos[1] - r[1]);
    let d2 = (dz as f64 * dz as f64 + (dx * dx) as f64) as f32;
    if !f.wrecked && d2 > 400.0 {
        return (true, !frozen);
    }
    (false, false)
}

/// The neutral / left / right / talk round robins (tables 0x827EA5F4, 0x827EA608, 0x827EA61C, 0x827EA630,
/// 0x827EA640).
const RR_NEUTRAL: [&str; 5] = ["drv_gest_mirror", "drv_win", "drv_gest_rspnd_lft", "drv_gest_rspnd_rgt", "drv_gest_fidget_01"];
const RR_LEFT: [&str; 5] = ["drv_gest_mild_lft_01", "drv_gest_agro_lft_01", "drv_gest_thm_dwn_lft", "drv_gest_thm_up_lft", "drv_gest_cas_lft_01"];
const RR_RIGHT: [&str; 5] = ["drv_gest_mild_rgt_01", "drv_gest_agro_rgt_01", "drv_gest_thm_dwn_rgt", "drv_gest_thm_up_rgt", "drv_gest_cas_rgt_01"];
const RR_TALK_LEFT: [&str; 4] = ["drv_gest_cas_lft_01", "drv_gest_agro_lft_01", "drv_gest_mild_lft_01", "drv_gest_rspnd_lft"];
const RR_TALK_RIGHT: [&str; 4] = ["drv_gest_cas_rgt_01", "drv_gest_agro_rgt_01", "drv_gest_mild_rgt_01", "drv_gest_rspnd_rgt"];
/// The mild and win pickers' two-clip tables (F-36583 with ctx+88, F-89025 with ctx+92).
const RR_MILD: [&str; 2] = ["drv_gest_mild_lft_01", "drv_gest_mild_rgt_01"];
const RR_WIN: [&str; 2] = ["drv_win", "drv_win_02"];
/// a gesture blends in over 0.25 s (the gesture state's enter plays it through F-24441 with 0.25 s) and back to
/// driving over 0.25 s (the cruising state's update requests its clip through F-85393 with 0.25 s), both linear
const GESTURE_BLEND: f32 = 0.25;

/// The clip a gesture plays (F-20767); `human`: the neutral round robin skips `_rspnd_rgt` for the human
/// player and `_rspnd_lft` for the others.
fn gesture_clip(g: Gesture, rr: &mut [usize; 7], human: bool) -> String {
    let mut next = |k: usize, list: &[&str]| -> String {
        let c = list[rr[k] % list.len()];
        rr[k] += 1;
        c.to_string()
    };
    match g {
        Gesture::Neutral => loop {
            let c = next(0, &RR_NEUTRAL);
            if (human && c == "drv_gest_rspnd_rgt") || (!human && c == "drv_gest_rspnd_lft") {
                continue;
            }
            break c;
        },
        Gesture::Left => next(1, &RR_LEFT),
        Gesture::Right => next(2, &RR_RIGHT),
        Gesture::TalkLeft => next(3, &RR_TALK_LEFT),
        Gesture::TalkRight => next(4, &RR_TALK_RIGHT),
        Gesture::Mild => next(5, &RR_MILD),
        Gesture::Win => next(6, &RR_WIN),
        Gesture::ListenLeft => "drv_gest_listen_lft".into(),
        Gesture::ListenRight => "drv_gest_listen_rgt".into(),
        Gesture::ThumbDownLeft => "drv_gest_thm_dwn_lft".into(),
        Gesture::ThumbDownRight => "drv_gest_thm_dwn_rgt".into(),
        // (event 53 sets racer+684 = 18; F-20767's case 18)
        Gesture::CopHandsUp => "drv_gest_rspnd".into(),
        // (case 17, F-57037: one clip, its counter unused)
        Gesture::CopFidget => "drv_gest_disapnt".into(),
    }
}

/// The bike machine's left / right pickers' five-clip tables (F-24572 with ctx+72, F-99511 with ctx+76) and its mild
/// picker's two (F-51244 with ctx+88).
const BIKE_LEFT: [&str; 5] = ["drv_gest_agro_lft", "drv_gest_thm_dwn_lft", "drv_gest_thm_up_lft", "drv_gest_look_lft", "drv_gest_cas_lft"];
const BIKE_RIGHT: [&str; 5] = ["drv_gest_agro_rgt", "drv_gest_thm_dwn_rgt", "drv_gest_thm_up_rgt", "drv_gest_look_rgt", "drv_gest_cas_rgt"];
const BIKE_MILD: [&str; 2] = ["drv_gest_agro_lft", "drv_gest_agro_rgt"];

/// The clip a gesture plays on a motorcycle (the bike machine's gesture switch enter F-32265 and its pickers): neutral
/// `drv_gest_disapnt` (F-38624: a one-clip table, its `_rspnd` tests never hold), left / right / mild round robins, talk
/// left / right `drv_gest_look_lft` / `_rgt` (F-44243 / F-57458), win `drv_win` (F-49380), the listen and thumb-down clips
/// by name, the ticket fidget `drv_gest_disapnt` (F-80672) and hands up `drv_gest_arrest`. Each picker steps its counter.
fn bike_gesture_clip(g: Gesture, rr: &mut [usize; 7]) -> String {
    let mut next = |k: usize, list: &[&str]| -> String {
        let c = list[rr[k] % list.len()];
        rr[k] += 1;
        c.to_string()
    };
    match g {
        Gesture::Neutral => next(0, &["drv_gest_disapnt"]),
        Gesture::Left => next(1, &BIKE_LEFT),
        Gesture::Right => next(2, &BIKE_RIGHT),
        Gesture::TalkLeft => next(3, &["drv_gest_look_lft"]),
        Gesture::TalkRight => next(4, &["drv_gest_look_rgt"]),
        Gesture::Mild => next(5, &BIKE_MILD),
        Gesture::Win => next(6, &["drv_win"]),
        Gesture::ListenLeft => "drv_gest_listen_lft".into(),
        Gesture::ListenRight => "drv_gest_listen_rgt".into(),
        Gesture::ThumbDownLeft => "drv_gest_thm_dwn_lft".into(),
        Gesture::ThumbDownRight => "drv_gest_thm_dwn_rgt".into(),
        Gesture::CopHandsUp => "drv_gest_arrest".into(),
        Gesture::CopFidget => "drv_gest_disapnt".into(),
    }
}

/// Blend `b` over `a` only on the bones `clip` animates (an overlay such as the jaw flap).
fn overlay(a: &mut [(Quat, Vec3)], b: &[(Quat, Vec3)], clip: &Clip, k: f32) {
    for i in 0..a.len().min(b.len()) {
        if clip.rot.get(i).map_or(false, |r| r.is_some()) {
            let fr = if a[i].0.dot(b[i].0) < 0.0 { -b[i].0 } else { b[i].0 };
            a[i].0 = a[i].0.lerp(fr, k).normalize();
        }
        if clip.pos.get(i).map_or(false, |p| p.is_some()) {
            a[i].1 = a[i].1.lerp(b[i].1, k);
        }
    }
}

/// A driver pack's file: LoadDriver (F-39898) asks for `$/resources/animation/Drv/%s/%s/%s` (F-62204) with
/// "Player" for the local human (the racer's vt+40: F-49619 true for mcPlayer, F-74264 false for mcOpponent; while the debug tune bool
/// `animplayerasopponent` is off) and "Opponent" for everyone else, the driver's sex, and the car's name with
/// its first three letters dropped unless it starts with "vpd", upper-cased and cut before a `_DUB` (F-51581); the
/// archive names the file by the hash of `<name>.xapk` (hashed lower-case). "Female" for a female driver (racer+177, `is_female`).
fn pack_path(root: &Path, player: bool, female: bool, key: &str) -> PathBuf {
    let h = mcla_formats::hash::joaat(&format!("{key}.xapk"));
    let (who, sex) = (if player { "player" } else { "opponent" }, if female { "female" } else { "male" });
    root.join(format!("resources/animation/0x{:08x}/{who}/{sex}/0x{h:08x}.r2", mcla_formats::hash::joaat("drv")))
}

/// The racer's character name as SetCharName (F-22036) keeps it: the two misnamed sets (the first 13 letters compared) become
/// the ones that exist.
fn char_name(name: &str) -> &str {
    if name.starts_with("drv_mc_04_set") {
        "drv_mp_01_set"
    } else if name.starts_with("drv_fh_01_set") {
        "drv_fh_001_set"
    } else {
        name
    }
}

/// Racer+177, the driver is a woman: SetCharName (F-22036) sets it when the kept name holds "_f" (case kept). An unnamed driver
/// is LoadDriver's default `drv_mp_01_set` (F-39898), a man.
fn is_female(name: &str) -> bool {
    char_name(name).contains("_f")
}

/// The pack name of a car: "vp_frd_mustang_06_dub" -> "frd_mustang_06"; DLC cars keep their "vpd_" prefix (F-62204 /
/// F-51581; the records spell every `_DUB` in capitals, so the cut is the name's end).
fn car_key(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    let n = n.trim_end_matches("_dub");
    if n.starts_with("vpd") { n.to_string() } else { n.get(3..).unwrap_or("").to_string() }
}

/// A driver pack's dictionary (F-99427): the holder's name table (+780, count +1548) against its clips (+8 + 4i), the keys
/// lower-cased (the lookup is case-blind), the first of equal keys kept.
fn holder_keys(rd: &Rd) -> Vec<(String, usize)> {
    let n = (rd.u32(1548) as usize).min(rd.u32(776) as usize).min(192);
    let mut out: Vec<(String, usize)> = Vec::new();
    for i in 0..n {
        let k = rd.ptr(780 + 4 * i).and_then(|a| rd.ptr(a)).map(|s| rd.cstr(s)).unwrap_or_default().to_ascii_lowercase();
        if let Some(o) = rd.ptr(8 + 4 * i) {
            if !out.iter().any(|x| x.0 == k) {
                out.push((k, o));
            }
        }
    }
    out
}

fn hash(a: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^ (h >> 13)
}

impl Drivers {
    /// The distance LOD on this frame's seats (F-12028 per drawn driver): a frozen driver's input context keeps what the
    /// reset (F-55405) gave it - no steer, no speed, not reversing - and its machine runs on those (F-28290 skips the
    /// gathering while the byte is set).
    pub fn freeze_far(&mut self, seats: &mut [Seat], f: impl Fn(&Seat) -> FreezeIn) {
        let mut next = HashMap::new();
        for s in seats.iter_mut() {
            let (on, _) = freeze_step(self.frozen.get(&s.uid).copied().unwrap_or(false), &f(s));
            if on {
                s.steer = 0.0;
                s.speed = 0.0;
                s.reversing = false;
            }
            next.insert(s.uid, on);
        }
        self.frozen = next;
    }

    pub fn load(&mut self, data_dir: &str) {
        let root = Path::new(data_dir);
        self.data_dir = root.to_path_buf();
        self.char_dir = root.join("resources/character");
        let mut names: Vec<String> = std::fs::read_dir(&self.char_dir).map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default();
        names.sort();
        for n in names {
            let l = n.to_ascii_lowercase();
            // every driver set a racer can be named (Racer_SetCharacterName, its in-car versions through
            // cinedriver::DRIVER_MAPS); an unnamed driver is one of the male in-car versions (`_lod02`, LoadDriver's hi-res `_set_h`)
            if !(l.starts_with("drv_") || l.starts_with("pol_")) {
                continue;
            }
            let pick = (l.ends_with("lod02_set") || l.ends_with("lod2_set") || l.ends_with("_set_h")) && (l.starts_with("drv_m") || l.starts_with("pol_"));
            self.sets.push((n, l.contains("_mot_"), l.starts_with("pol_"), pick));
        }
        log::info!("drivers: {} character sets", self.sets.len());
    }

    fn model(&mut self, gpu: &mut Gpu, si: usize) -> Option<Arc<SkinModel>> {
        if !self.models.contains_key(&si) {
            let name = &self.sets[si].0;
            let dir = self.char_dir.join(name);
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "r6").unwrap_or(false)).collect()).unwrap_or_default();
            files.sort();
            let m = files.first().and_then(|f| SkinModel::load(gpu, name, f)).map(Arc::new);
            if m.is_none() {
                log::warn!("drivers: could not load {}", name);
            }
            self.models.insert(si, m);
        }
        self.models[&si].clone()
    }

    fn pick_set(&self, s: &Seat) -> Option<usize> {
        let lod = self.pick_lod_set(s)?;
        // (LoadDriver, F-39898: the racer's name through the hi-res maps for the local human and a raised racer)
        if s.hi {
            if let Some(i) = crate::cinedriver::driver_set(&self.sets[lod].0, true, s.bike).and_then(|h| self.sets.iter().position(|n| n.0.eq_ignore_ascii_case(h))) {
                return Some(i);
            }
        }
        Some(lod)
    }

    fn pick_lod_set(&self, s: &Seat) -> Option<usize> {
        // (step 19d: a driver who was named - Racer_SetCharacterName - is that character; a key goes through
        // Boot.sco's `_lod02` maps)
        if !s.set.is_empty() {
            let set = char_name(s.set);
            let want = crate::cinedriver::driver_set(set, false, s.bike).unwrap_or(set);
            if let Some(i) = self.sets.iter().position(|n| n.0.eq_ignore_ascii_case(want)) {
                return Some(i);
            }
        }
        let want = |n: &(String, bool, bool, bool)| n.3 && n.1 == s.bike && n.2 == (s.cop && !s.bike) && !n.0.ends_with("_set_h");
        if s.player {
            // "mp" is the player character
            if let Some(i) = self.sets.iter().position(|n| n.3 && n.0.to_ascii_lowercase().starts_with("drv_mp_") && n.1 == s.bike && !n.0.to_ascii_lowercase().ends_with("_set_h")) {
                return Some(i);
            }
        }
        let c: Vec<usize> = (0..self.sets.len()).filter(|&i| want(&self.sets[i]) && !self.sets[i].0.to_ascii_lowercase().starts_with("drv_mp_")).collect();
        if c.is_empty() {
            return self.sets.iter().position(|n| n.3 && n.1 == s.bike && !n.0.to_ascii_lowercase().ends_with("_set_h"));
        }
        // (by the car's own number: the place in the list moved when cars went, and the driver - his
        // face - changed with it; GitHub #17)
        Some(c[hash(s.uid + 17) as usize % c.len()])
    }

    /// The length of a car's DRV_HIT_HRD clip, once its driver is loaded.
    pub fn hit_hard_secs(&self, car: &str) -> Option<f32> {
        let k = car_key(car);
        self.rigs.iter().filter(|((c, _, _, _), _)| *c == k).find_map(|(_, r)| r.as_ref()?.hit_hard)
    }

    fn rig(&mut self, gpu: &mut Gpu, car: &str, bike: bool, player: bool, female: bool, si: usize) -> bool {
        let is_bike = bike;
        let key = (car_key(car), player, female, si);
        if !self.rigs.contains_key(&key) {
            let rig = (|| -> Option<Rig> {
                let model = self.model(gpu, si)?;
                // (the car's own pack, else the shared one of its kind - F-39898 / F-62204)
                let path = |k: &str| Some(pack_path(&self.data_dir, player, female, k)).filter(|p| p.is_file());
                let pack = path(&key.0).or_else(|| path(if bike { "kaw_ninja_zx14_06" } else { "dge_challenger_70" }))?;
                let res = load_resource_file(&pack).ok()?;
                let rd = Rd::new(&res);
                let entries = pack_entries(&res);
                let clip = |suffixes: &[&str]| -> Option<Clip> {
                    for s in suffixes {
                        // the car's own clip first, then the shared one
                        let mut hit = entries.iter().filter(|(_, n)| n.to_ascii_lowercase().ends_with(s)).collect::<Vec<_>>();
                        hit.sort_by_key(|(_, n)| !n.to_ascii_lowercase().starts_with(&key.0));
                        if let Some((o, _)) = hit.first() {
                            return animation_named(&rd, *o).map(|a| Clip::bind(&Arc::new(a), &model.ids, false, false));
                        }
                    }
                    None
                };
                let steer = clip(&["drv_steer_sweep", "drv_low_lean_sweep", "drv_idle_stp_sweep"])?;
                // the gestures the scenes ask for (case blind by name after the car prefix, F-99427)
                let mut gestures = HashMap::new();
                let mut names: Vec<&(usize, String)> = entries.iter().filter(|(_, n)| { let l = n.to_ascii_lowercase(); l.contains("_drv_gest_") || l.ends_with("_drv_win") || l.ends_with("_drv_win_02") }).collect();
                names.sort_by_key(|(_, n)| n.to_ascii_lowercase().starts_with(&key.0));
                for (o, n) in names {
                    let l = n.to_ascii_lowercase();
                    let Some(i) = l.find("drv_") else { continue };
                    if let Some(a) = animation_named(&rd, *o) {
                        gestures.insert(l[i..].to_string(), Clip::bind(&Arc::new(a), &model.ids, false, false));
                    }
                }
                let hit_hard = clip(&["drv_hit_hrd"]).map(|c| c.anim.duration);
                let mut ejects = HashMap::new();
                for n in ["drv_eject_l", "drv_eject_r", "drv_eject_bk", "drv_eject_over"] {
                    if let Some(c) = clip(&[n]) {
                        ejects.insert(n.to_string(), c);
                    }
                }
                let bike = BikeClips::load(&clip);
                let (mut dict, mut clips) = (HashMap::new(), Vec::new());
                if !is_bike {
                    // the pack's own dictionary (F-99427): the holder's name table (+780, count +1548) holds the keys
                    // ("Drv_steer_sweep"), entry i the clip at +8 + 4i; compared case-blind, the first equal one wins. The
                    // clips' own names carry the prefix of the car they were made for (the Police Car Pack's: CHV_IMPALA_96_ /
                    // DGE_CHARGER_06_, the faces dge_challenger_70_), the keys none.
                    for (k, o) in holder_keys(&rd) {
                        if let Some(a) = animation_named(&rd, o) {
                            dict.insert(k, clips.len() as u32);
                            clips.push(Clip::bind(&Arc::new(a), &model.ids, false, false));
                        }
                    }
                }
                let per_dof = |m: &dyn Fn(usize) -> bool| -> Vec<f32> { (0..model.bones.len()).flat_map(|b| [if m(b) { 1.0 } else { 0.0 }; 2]).collect() };
                let dofw = [per_dof(&|b| model.spine_bones[b]), per_dof(&|b| model.neck_bones[b]), per_dof(&|b| !model.face_bones[b])];
                Some(Rig { steer, idle_stop: clip(&["drv_idle_stp"]), reverse: clip(&["drv_rev"]), model, gestures, hit_hard, ejects, bike, dict, clips, dofw })
            })();
            if rig.is_none() {
                log::warn!("drivers: no animations for {}", car);
            }
            self.rigs.insert(key.clone(), rig);
        }
        self.rigs[&key].is_some()
    }

    /// A scene's gesture event for the driver of seat `id` (racer+684). Lost when a gesture is still playing
    /// (the "gesture switch" exit writes "none"). `dur` > 0: the event's end call stops it then.
    pub fn gesture(&mut self, id: usize, g: Gesture, dur: f32, human: bool) {
        let a = self.acting.entry(id).or_default();
        if a.playing.as_ref().map_or(true, |p| p.left <= 0.0) && a.asked.is_none() {
            a.asked = Some((g, if dur > 0.0 { dur } else { f32::INFINITY }, human));
        }
    }

    /// A scene's gesture event stopped (FinishScene F-65984 -> F-10326 -> F-81612 stops every active event; the gesture
    /// stop F-26895 sets racer +684 = 20, F-90946, for every gesture but copfidget): that seat's gesture leaves with its
    /// blend-out, its jaw and the other seats untouched; the seat takes a new gesture at once.
    pub fn end_gesture(&mut self, id: usize) {
        if let Some(a) = self.acting.get_mut(&id) {
            if let Some(p) = a.playing.as_mut() {
                p.left = 0.0;
            }
            a.asked = None;
        }
    }

    /// The jaw flap while the driver's speech line plays (racer+688; `DRV_GEST_JAWFLAP` in / out over 0.25 s).
    pub fn jaw(&mut self, id: usize, on: bool) {
        self.acting.entry(id).or_default().jaw = on;
    }

    /// The rider of seat `id` leaves his seat by an eject clip (`rider_eject::Eject::clip`) at `rate`.
    pub fn eject(&mut self, id: usize, clip: &str, rate: f32) {
        self.off.insert(id, Off::Eject { clip: clip.to_ascii_lowercase(), t: 0.0, rate });
    }

    /// The eject clip of seat `id` has played out: its last frame's local pose (the drawn model's bones), the
    /// character matrix it stood at and the set's `.r6` (its fragment type is the ragdoll's). Taken once.
    pub fn take_ragdoll_start(&mut self, id: usize) -> Option<(Vec<(Quat, Vec3)>, Mat4, PathBuf)> {
        if !matches!(self.off.get(&id), Some(Off::Ready { .. })) {
            return None;
        }
        let r6 = self.set_of.get(&id).and_then(|n| {
            let mut f: Vec<PathBuf> = std::fs::read_dir(self.char_dir.join(n)).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |e| e == "r6")).collect();
            f.sort();
            f.into_iter().next()
        })?;
        match self.off.insert(id, Off::Ragdoll { world: Vec::new() }) {
            Some(Off::Ready { local, mat }) => Some((local, mat, r6)),
            _ => None,
        }
    }

    /// The thrown rider's bones in the world this frame.
    pub fn ragdoll_world(&mut self, id: usize, world: Vec<Mat4>) {
        self.off.insert(id, Off::Ragdoll { world });
    }

    /// The rider back on his seat (RemoveRagdoll's snap, F-98260: the skeleton goes back to the animation), or a
    /// wipeout ended before the ragdoll.
    pub fn seat_back(&mut self, id: usize) {
        self.off.remove(&id);
    }

    /// Whether seat `id`'s rider is off his seat (an eject clip or the ragdoll).
    pub fn is_off(&self, id: usize) -> bool {
        self.off.contains_key(&id)
    }

    /// Scene over: no gestures, no jaw flap.
    pub fn stop_acting(&mut self) {
        for a in self.acting.values_mut() {
            a.playing = None;
            a.asked = None;
            a.jaw = false;
        }
    }

    /// Pose and skin the drivers of the cars near the camera.
    pub fn prepare(&mut self, gpu: &mut Gpu, seats: &[Seat], cam: Vec3, frustum: &Frustum, dt: f32) {
        self.buf.clear();
        self.time += dt;
        self.frame += 1;
        // (the hooks' update, F-31704: every driver in a vehicle, drawn or not; a car gone takes its hook with it)
        self.wind.retain(|u, _| seats.iter().any(|s| s.uid == *u));
        for s in seats {
            self.wind.entry(s.uid).or_default().step(s.vel.length(), s.vel, s.bike, dt, self.paused);
        }
        // (the car drivers' machines: every frame for each car whose rig is loaded, drawn or not)
        let machines = std::env::var("MCLA_DRV_OLD").is_err();
        let k = dm::Tune::original();
        self.cars.retain(|u, _| seats.iter().any(|s| s.uid == *u));
        self.steer_out.clear();
        self.hit_taken.clear();
        if machines {
            for s in seats {
                let (Some(d), false) = (s.drv.as_ref(), s.bike) else { continue };
                let Some(si) = self.pick_set(s) else { continue };
                let Some(Some(rig)) = self.rigs.get(&(car_key(s.car), s.player, is_female(s.set), si)) else { continue };
                // (the racer's +144, the character's name: the faces are `<name>_<set>`; the hi-res model is the set's `_h`)
                let set = self.sets[si].0.trim_end_matches("_h").to_string();
                let a = self.cars.entry(s.uid).or_insert_with(|| CarAnim::new(&set, [0x2f6b_6f5d, 0x0000_3039]));
                let culled = self.frozen.get(&s.uid).copied().unwrap_or(false);
                if let Some(o) = a.step(rig, d, &k, dt, self.frame & 1 == 1, culled, &mut self.rng, s.car) {
                    self.steer_out.insert(s.uid, o);
                }
                // (the gatherer took the damage object's +48: main.rs clears the car's visible hit)
                if d.dmg[1] != 0.0 && a.gm.dmg.is_some_and(|x| x[1] == 0.0) {
                    self.hit_taken.push(s.uid);
                }
            }
        }
        let mut local: Vec<(Quat, Vec3)> = Vec::new();
        let mut other: Vec<(Quat, Vec3)> = Vec::new();
        for s in seats {
            let dist = s.pos.distance(cam);
            // ambient drivers only matter close up
            let reach = if s.car.starts_with("va_") { 40.0 } else { 70.0 };
            if dist > reach || !frustum.intersects_aabb(s.pos - Vec3::splat(2.5), s.pos + Vec3::splat(2.5)) {
                continue;
            }
            if s.player && self.cockpit == 2 {
                continue;
            }
            let Some(si) = self.pick_set(s) else { continue };
            self.set_of.insert(s.id, self.sets[si].0.clone());
            let female = is_female(s.set);
            if !self.rig(gpu, s.car, s.bike, s.player, female, si) {
                continue;
            }
            let frame = self.frame;
            if self.poses.get(&s.id).map_or(true, |q| q.seen + 1 != frame) {
                // (a rider not drawn last frame starts over in "start here")
                self.rides.remove(&s.id);
            }
            let p = self.poses.entry(s.id).or_default();
            if p.seen + 1 != frame {
                *p = Pose { steer: s.steer, ..Default::default() };
            }
            p.seen = frame;
            let k = (dt * 8.0).min(1.0);
            p.steer += (s.steer - p.steer) * k;
            p.stopped += ((if s.speed.abs() < 0.6 { 1.0 } else { 0.0 }) - p.stopped) * (dt * 3.0).min(1.0);
            p.reverse += ((if s.reversing && s.speed.abs() > 0.3 { 1.0 } else { 0.0 }) - p.reverse) * (dt * 4.0).min(1.0);
            let p = *p;
            let Some(Some(rig)) = self.rigs.get(&(car_key(s.car), s.player, female, si)) else { continue };
            if let (true, Some(b)) = (s.bike, rig.bike.as_ref()) {
                let ride = self.rides.entry(s.id).or_default();
                let i = RideIn { ck: s.player && self.ckpt_view, ..s.ride };
                ride_pose(ride, b, &rig.model.bones, &rig.model.neck_bones, &i, self.time, dt, &mut local, &mut other);
            } else if let Some(fr) = self.cars.get(&s.uid).filter(|_| machines && !s.bike).and_then(|a| a.seq.compose(&RigPoser(rig))) {
                finish(&fr, &rig.model.bones, &mut local);
            } else {
            // the sweep runs from full left lock to full right lock
            let last = rig.steer.last_frame();
            rig.steer.local(&rig.model.bones, (0.5 + 0.5 * p.steer.clamp(-1.0, 1.0)) * last, &mut local);
            if let Some(idle) = rig.idle_stop.as_ref() {
                if p.stopped > 0.01 {
                    idle.local(&rig.model.bones, idle.frame(self.time + s.id as f32 * 0.37, true), &mut other);
                    blend_pose(&mut local, &other, p.stopped);
                }
            }
            if let Some(rev) = rig.reverse.as_ref() {
                if p.reverse > 0.01 {
                    rev.local(&rig.model.bones, rev.last_frame(), &mut other);
                    blend_pose(&mut local, &other, p.reverse);
                }
            }
            }
            // a scene's gesture over the driving pose, and the jaw flap over that
            if let Some(a) = self.acting.get_mut(&s.id) {
                if let Some((g, left, human)) = a.asked.take() {
                    let name = if s.bike { bike_gesture_clip(g, &mut a.rr) } else { gesture_clip(g, &mut a.rr, human) };
                    if rig.gestures.contains_key(&name) {
                        log::info!("drivers: seat {} plays {}", s.id, name);
                        // (a stopped gesture still on: it blends out under the new one, `end_gesture`)
                        let old = a.playing.take().and_then(|p| rig.gestures.get(&p.clip).map(|_| (p.clip.clone(), p.t, 1.0)));
                        a.playing = Some(Playing { clip: name, t: 0.0, left });
                        a.leaving = old;
                    } else {
                        log::info!("drivers: seat {} has no {}", s.id, name);
                    }
                }
                let mut over = false;
                if let Some(pl) = a.playing.as_mut() {
                    if let Some(c) = rig.gestures.get(&pl.clip) {
                        pl.t += dt;
                        pl.left -= dt;
                        let f = c.frame(pl.t, false);
                        c.local(&rig.model.bones, f, &mut other);
                        // (the gesture clips animate the arms, head and upper body only: laid over the
                        // driving pose on the bones they move)
                        overlay(&mut local, &other, c, (pl.t / GESTURE_BLEND).min(1.0));
                        // (the state leaves when under 0.033 s of the clip is left, a frame before its end - F-85393's
                        // caller, the gesture state's exit term)
                        over = c.anim.duration - pl.t < 0.033 || pl.left <= 0.0;
                        if over {
                            a.leaving = Some((pl.clip.clone(), pl.t, 1.0));
                        }
                    } else {
                        over = true;
                    }
                }
                if over {
                    a.playing = None;
                }
                if a.playing.is_none() {
                    if let Some((name, t, w)) = a.leaving.as_mut() {
                        *w -= dt / GESTURE_BLEND;
                        *t += dt;
                        if let Some(c) = rig.gestures.get(name.as_str()).filter(|_| *w > 0.0) {
                            c.local(&rig.model.bones, c.frame(*t, false), &mut other);
                            overlay(&mut local, &other, c, *w);
                        }
                    }
                    if a.leaving.as_ref().map_or(false, |l| l.2 <= 0.0) {
                        a.leaving = None;
                    }
                }
                a.jaw_k = (a.jaw_k + (if a.jaw { dt } else { -dt }) / GESTURE_BLEND).clamp(0.0, 1.0);
                if a.jaw {
                    a.jaw_t += dt;
                }
                if let (true, Some(c), Some(head)) = (a.jaw_k > 0.0, rig.gestures.get("drv_gest_jawflap"), rig.model.head) {
                    // (the shared jaw flap clip is the Challenger's whole driver; laid over the bones
                    // below the head only - the jaw. The original's overlay mask is not read.)
                    c.local(&rig.model.bones, c.frame(a.jaw_t, true), &mut other);
                    let bones = &rig.model.bones;
                    let below = |mut i: usize| -> bool {
                        while let Some(p) = bones.get(i).and_then(|b| b.parent) {
                            if p == head {
                                return true;
                            }
                            if p == i {
                                break;
                            }
                            i = p;
                        }
                        false
                    };
                    let mut masked = local.clone();
                    overlay(&mut masked, &other, c, a.jaw_k);
                    for i in 0..local.len() {
                        if below(i) {
                            local[i] = masked[i];
                        }
                    }
                }
            }
            // (a rider off his seat: the eject clip in place of the riding pose, then the ragdoll's bones)
            // (F-41119: the clip's time remaining at or under 0)
            let mut next: Option<Option<Off>> = None;
            match self.off.get_mut(&s.id) {
                Some(Off::Eject { clip, t, rate }) => match rig.ejects.get(clip.as_str()) {
                    Some(c) => {
                        *t += dt * *rate;
                        if *t >= c.anim.duration {
                            c.local(&rig.model.bones, c.last_frame(), &mut local);
                            next = Some(Some(Off::Ready { local: local.clone(), mat: s.mat }));
                        } else {
                            c.local(&rig.model.bones, c.frame(*t, false), &mut local);
                        }
                    }
                    None => {
                        log::info!("drivers: seat {} has no {}", s.id, clip);
                        next = Some(None);
                    }
                },
                Some(Off::Ready { local: l, .. }) => local.clone_from(l),
                Some(Off::Ragdoll { world }) if !world.is_empty() => self.buf.next_world = Some(world.clone()),
                _ => {}
            }
            match next {
                Some(Some(o)) => {
                    self.off.insert(s.id, o);
                }
                Some(None) => {
                    self.off.remove(&s.id);
                }
                None => {}
            }
            // (the POV view draws the set's cockpit view model - arms and hands - in place of the driver; a
            // set without one draws its full model, as the original does after its warning - F-56050)
            if s.player && self.cockpit == 1 {
                self.buf.next_cockpit = true;
            }
            // (the driver's stencil id: his car's + 1, a rider's his bike's - render/mb.rs, F-10884)
            self.buf.next_stencil = crate::render::mb::driver_stencil_of(s.id);
            // (the car's gauge and neon lights and its floor - F-42959; MCLA_DRV_CABIN_OLD=1: none, as outside a car)
            if let Some(offs) = s.lights.filter(|_| std::env::var("MCLA_DRV_CABIN_OLD").is_err()) {
                let colors = match s.colors {
                    Some(c) => c,
                    None => {
                        let dir = self.data_dir.clone();
                        *self.own_colors.entry(s.car.to_ascii_lowercase()).or_insert_with(|| {
                            crate::render::car_lamps::setup_file(&dir, s.car).and_then(|f| std::fs::read(f).ok()).and_then(|b| crate::mc4::setup_look(&b)).map_or([[0.0; 4]; 2], |l| l.look.cabin_light)
                        })
                    }
                };
                let wind = self.wind.get(&s.uid).copied().unwrap_or_default();
                self.buf.next_cabin = Some(Cabin::of(s.mat, offs, colors, s.head, s.bike, neon_flash(colors[0][3], self.clock), &wind));
            }
            self.buf.emit(&rig.model, 0, &local, s.mat);
            if s.player {
                self.player_head = rig.model.head.and_then(|h| self.buf.bone_pos(h)).map(|w| s.mat.inverse().transform_point3(w));
            }
        }
        if self.frame == 50 {
            log::info!("drivers: {} seats, {} rigs, {} poses", seats.len(), self.rigs.len(), self.poses.len());
            for s in seats {
                log::info!("  seat {} {} dist {:.1}", s.id, s.car, s.pos.distance(cam));
            }
        }
        self.buf.upload(gpu);
    }

    /// The lights the figures are drawn with (`CharLight`: the character programs), or none: the city's object look.
    pub fn set_light(&mut self, l: Option<crate::skin::CharLight>) {
        self.buf.char_light = l;
        self.buf.light_at_root = true;
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, draws: &mut usize) {
        self.buf.draw(pass, gpu, false, draws);
    }
}

/// F-80898, F-40322, F-82819, F-95452, F-59643, F-36583, F-89025 (the gestures' round-robin pickers) against the original's code run offline
/// (tools/people_gesture_reference.py -> testdata/people_gesture_ref.txt): the clip name picked and the counter afterwards.
#[cfg(test)]
mod original_tests {
    use super::*;

    #[test]
    fn a_car_finds_its_own_driver_pack() {
        // (the archive's names: the Police Car Pack's five, the Impala's, the shared Challenger's)
        let r = Path::new("d");
        let f = |car: &str, player: bool| pack_path(r, player, false, &car_key(car)).to_string_lossy().replace('\\', "/");
        assert_eq!(f("vp_se_chv_plc_96", true), "d/resources/animation/0xe4f29864/player/male/0x8b348a1a.r2");
        assert_eq!(f("vp_se_chv_chp_96", false), "d/resources/animation/0xe4f29864/opponent/male/0xa7db038c.r2");
        assert_eq!(f("vp_se_dge_plc_06", true), "d/resources/animation/0xe4f29864/player/male/0xdaddd4cd.r2");
        assert_eq!(f("vp_se_dge_ubp_06", true), "d/resources/animation/0xe4f29864/player/male/0xf8dec655.r2");
        assert_eq!(f("vp_se_dge_chp_06", true), "d/resources/animation/0xe4f29864/player/male/0x39620546.r2");
        assert_eq!(f("vp_chv_impala_96", true), "d/resources/animation/0xe4f29864/player/male/0x394cb4ed.r2");
        assert_eq!(f("vpd_chv_impala_64", false), "d/resources/animation/0xe4f29864/opponent/male/0x0db60427.r2");
        assert_eq!(f("vp_dge_challenger_70", false), "d/resources/animation/0xe4f29864/opponent/male/0x953f13ee.r2");
        assert_eq!(car_key("vp_chv_camaro_06_DUB"), "chv_camaro_06");
        assert_eq!(car_key("vp_chv_police_96"), "chv_police_96");
        // a woman's: the same name in the Female folder (the disc has every car's pack in both)
        assert!(pack_path(r, false, true, "dge_challenger_70").ends_with("resources/animation/0xe4f29864/opponent/female/0x953f13ee.r2"));
    }

    /// The pack's own dictionary keys the clips by their plain names: the Police Car Pack's packs (clips named for the
    /// civilian Impala / Charger) give the machine every clip it asks for, as the civilian cars' do (GitHub #36).
    #[test]
    fn a_police_pack_keys_its_clips_by_plain_name() {
        let r = Path::new("D:/MCLA/extracted/cache");
        for (car, imp) in [("vp_se_chv_plc_96", "vp_chv_impala_96"), ("vp_se_dge_plc_06", "vp_dge_charger_06")] {
            for pl in [true, false] {
                let (Ok(a), Ok(b)) = (load_resource_file(&pack_path(r, pl, false, &car_key(car))), load_resource_file(&pack_path(r, pl, false, &car_key(imp)))) else {
                    eprintln!("no disc data, skipped");
                    return;
                };
                let (ka, kb) = (holder_keys(&Rd::new(&a)), holder_keys(&Rd::new(&b)));
                assert!(ka.iter().any(|k| k.0 == "drv_steer_sweep") && ka.iter().any(|k| k.0 == "drv_idle_stp"), "{car}");
                let names = |k: &[(String, usize)]| k.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
                assert_eq!(names(&ka), names(&kb), "{car} keys as {imp}'s");
                assert_eq!(ka.len(), if pl { 182 } else { 38 });
            }
        }
    }

    /// SetCharName's (F-22036) renames and its "_f" test, on the names the scripts give (the hookmen's sets).
    #[test]
    fn a_driver_named_a_woman_is_one() {
        assert_eq!(char_name("drv_mc_04_set"), "drv_mp_01_set");
        assert_eq!(char_name("drv_fh_01_set"), "drv_fh_001_set");
        assert_eq!(char_name("drv_fh_001_set"), "drv_fh_001_set");
        for (gh, f) in [("GH_29", true), ("GH_34", true), ("GH_35", true), ("GH_06", false), ("GH_36", false)] {
            assert_eq!(is_female(crate::cinedriver::hookman_set(gh).unwrap()), f, "{gh}");
        }
        assert!(is_female("drv_fh_01_set") && is_female("drv_fa_001_set") && !is_female("drv_mp_01_set") && !is_female(""));
    }

    /// The bike machine's pickers (F-24572, F-99511, F-51244: the counter's old value modulo the table, then counter + 1).
    #[test]
    fn bike_gestures() {
        let mut rr = [0usize; 7];
        let left: Vec<String> = (0..6).map(|_| bike_gesture_clip(Gesture::Left, &mut rr)).collect();
        assert_eq!(left, ["drv_gest_agro_lft", "drv_gest_thm_dwn_lft", "drv_gest_thm_up_lft", "drv_gest_look_lft", "drv_gest_cas_lft", "drv_gest_agro_lft"]);
        assert_eq!((bike_gesture_clip(Gesture::Mild, &mut rr), bike_gesture_clip(Gesture::Mild, &mut rr)), ("drv_gest_agro_lft".into(), "drv_gest_agro_rgt".into()));
        assert_eq!(bike_gesture_clip(Gesture::Neutral, &mut rr), "drv_gest_disapnt");
        assert_eq!(bike_gesture_clip(Gesture::CopHandsUp, &mut rr), "drv_gest_arrest");
        assert_eq!(rr, [1, 6, 0, 0, 0, 2, 0]);
    }

    #[test]
    fn gesture_round_robins_match_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/people_gesture_ref.txt").lines() {
            let w: Vec<&str> = line.split(' ').collect();
            let (kind, human, ctr): (usize, bool, usize) = (w[1].parse().unwrap(), w[2] == "1", w[3].parse().unwrap());
            let g = [Gesture::Neutral, Gesture::Left, Gesture::Right, Gesture::TalkLeft, Gesture::TalkRight, Gesture::Mild, Gesture::Win][kind];
            let mut rr = [0usize; 7];
            rr[kind] = ctr;
            let name = gesture_clip(g, &mut rr, human);
            assert_eq!((name.as_str(), rr[kind]), (w[5], w[6].parse::<usize>().unwrap()), "{line}");
            n += 1;
        }
        assert_eq!(n, 192);
    }

    #[test]
    fn driver_freeze_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/driver_freeze_ref.txt").lines() {
            let w: Vec<&str> = line.split(' ').collect();
            let b = |i: usize| w[i] != "0";
            let x = |i: usize| f32::from_bits(u32::from_str_radix(w[i], 16).unwrap());
            let fl: u32 = w[11].parse().unwrap();
            let f = FreezeIn {
                active: b(2),
                scene: b(3),
                show: b(4),
                show_scene: b(5),
                no_damage: b(6),
                b370: b(7),
                car: b(8),
                cine: b(9),
                player: b(10),
                wrecked: fl & 0x14 != 0,
                reference: (w[12] == "1").then(|| [x(13), x(14)]),
                camera: [x(15), x(16)],
                pos: [x(17), x(18)],
            };
            let got = freeze_step(b(1), &f);
            assert_eq!((got.0 as u8, got.1 as u8), (w[20].parse::<u8>().unwrap(), w[21].parse::<u8>().unwrap()), "{line}");
            n += 1;
        }
        assert_eq!(n, 4048);
    }
}

#[cfg(test)]
mod cabin_tests {
    use super::*;

    /// The car's matrix from a recording: up = floorPlaneEq.xyz (c214), z = -gaugeLightConeDir (c211), x = up x z,
    /// the place = the body's origin (a VS_TransformLit draw at it).
    fn car(up: [f32; 3], cone: [f32; 3], at: [f32; 3]) -> Mat4 {
        let (y, z) = (Vec3::from(up), -Vec3::from(cone));
        Mat4::from_cols(y.cross(z).extend(0.0), y.extend(0.0), z.extend(0.0), Vec3::from(at).extend(1.0))
    }

    fn near(a: Vec3, b: [f32; 3], e: f32) -> bool {
        (a - Vec3::from(b)).abs().max_element() < e
    }

    /// F-31704 then F-42959 on the same hook, run offline on the original's code (tools/driver_wind_reference.py, 6,000
    /// cases): the hook's wind and the eight values the driver's draw is given, bit for bit.
    #[test]
    fn wind_and_cabin_match_the_original() {
        let text = include_str!("testdata/driver_wind_ref.txt");
        let mut bad = [0usize; 12];
        let mut first: Option<String> = None;
        let mut n = 0;
        for line in text.lines() {
            let (i, o) = line.strip_prefix("DW ").unwrap().split_once(" -> ").unwrap();
            let x: Vec<u32> = i.split(' ').map(|w| u32::from_str_radix(w, 16).unwrap()).collect();
            let y: Vec<u32> = o.split(' ').map(|w| u32::from_str_radix(w, 16).unwrap()).collect();
            let f = |k: usize| f32::from_bits(x[k]);
            let v3 = |k: usize| Vec3::new(f(k), f(k + 1), f(k + 2));
            let m = Mat4::from_cols_array(&std::array::from_fn(|k| f(k)));
            let offs = [v3(16).to_array(), v3(19).to_array()];
            let colors = [[f(22), f(23), f(24), 0.0], [f(25), f(26), f(27), f(28)]];
            assert_eq!(f(30), 1.0);
            let mut w = Wind { phase: f(33), ..Default::default() };
            w.step(f(34), v3(35), x[32] != 0, f(40), x[39] != 0);
            let c = Cabin::of(m, offs, colors, f(31), x[32] != 0, f(29), &w);
            let b3 = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            let got: [&[u32]; 12] = [
                &[w.phase.to_bits()],
                &[w.speed.to_bits()],
                &b3(w.vel),
                &b3(c.neon_pos),
                &b3(c.neon_col),
                &b3(c.gauge_pos),
                &b3(c.gauge_col),
                &b3(c.cone_dir),
                &c.floor.map(f32::to_bits),
                &c.wind_eq.map(f32::to_bits),
                &c.wind.map(f32::to_bits),
                &[],
            ];
            let want: [&[u32]; 12] = [&y[0..1], &y[1..2], &y[2..5], &y[6..9], &y[10..13], &y[14..17], &y[18..21], &y[22..25], &y[26..30], &y[30..34], &y[34..36], &[]];
            for k in 0..11 {
                if got[k] != want[k] {
                    bad[k] += 1;
                    first.get_or_insert_with(|| format!("field {k}: {:08x?} against {:08x?} in {line}", got[k], want[k]));
                }
            }
            n += 1;
        }
        assert_eq!(n, 6000);
        assert!(bad.iter().all(|&b| b == 0), "mismatches by field {bad:?}; first {}", first.unwrap_or_default());
    }

    /// F-42959 against the running original: c209 / c210 / c211 / c212 / c214 of a driver's and a rider's draws.
    #[test]
    fn cabin_lights_match_the_recorded_draws() {
        // (an R8 at noon, work/intglass/cap/pov_noon.bin draw 1938, the body at draw 1923; its tune's offsets): the gauge
        // and neon places, the floor; by day the head state 0 - no gauge colour
        let m = car([0.0183608793, 0.998237431, 0.0564348772], [-0.474544317, -0.0409810767, 0.87927711], [819.610229, 17.5722237, -808.409119]);
        let c = Cabin::of(m, [[-0.375, 0.875, -1.325], [-0.375, 0.675, -0.85]], [[0.52941, 0.52941, 0.52941, 1.0], [0.0; 4]], 0.0, false, 1.0, &Wind::default());
        assert!(near(c.gauge_pos, [819.327515, 18.3752842, -807.017334], 2e-4), "{c:?}");
        assert!(near(c.neon_pos, [819.549255, 18.1951046, -807.446289], 2e-4), "{c:?}");
        assert!(near(c.cone_dir, [-0.474544317, -0.0409810767, 0.87927711], 1e-7) && c.gauge_col == Vec3::ZERO && c.neon_col == Vec3::ZERO);
        assert!(near(Vec3::new(c.floor[0], c.floor[1], c.floor[2]), [0.0183608793, 0.998237431, 0.0564348772], 1e-7) && (c.floor[3] + 12.7324543).abs() < 2e-4, "{c:?}");
        // (parked: no velocity - the plane (0, 0, -1) through the body, c215 (0 0 -1 808.409119); a car's windParams 0)
        assert!(c.wind_eq == [0.0, 0.0, -1.0, 808.409119] && c.wind == [0.0, 0.0], "{c:?}");
        // (the ZX-14 at 23:00, work/police17/cap/rider_night.bin draw 627, the body at draw 608): its record's DashGaugeColor
        // with the head lamps on, the floor of a bike, the neon (offset 0) at the bike's origin
        let m = car([-0.0173120443, 0.999082983, 0.0391364694], [-0.470790386, -0.0426769406, 0.881210923], [819.67041, 17.5936108, -808.20929]);
        let c = Cabin::of(m, [[0.0, 1.05, -1.275], [0.0; 3]], [[74.0 / 255.0, 93.0 / 255.0, 130.0 / 255.0, 1.0], [0.52, 0.52, 1.0, 0.0]], 1.0, true, 1.0, &Wind::default());
        assert!(near(c.gauge_pos, [819.051941, 18.5882359, -807.044678], 2e-4), "{c:?}");
        assert!(near(c.gauge_col, [0.339214742, 0.376469642, 0.449018657], 6e-8), "{c:?}");
        assert!(near(c.neon_pos, [819.67041, 17.5936108, -808.20929], 1e-5) && c.neon_col == Vec3::ZERO && c.floor[3] == -1.0e5, "{c:?}");
        // (its wind, c215 (0.880322516 -0.044405967 0.472292662 339.081726), c216 (0.246283069 0.00220525498): the bike
        // creeping at 0.0887 m/s - the strength is mph / 90; the plane along the velocity, w through the body's place)
        let n = Vec3::new(0.880322516, -0.044405967, 0.472292662);
        let mut w = Wind { phase: 0.246283069, ..Default::default() };
        w.step(0.00220525498 * 90.0 / 2.237, n * 0.0887, true, 0.0, false);
        let c = Cabin::of(m, [[0.0; 3]; 2], [[0.0; 4]; 2], 1.0, true, 1.0, &w);
        assert!(near(Vec3::new(c.wind_eq[0], c.wind_eq[1], c.wind_eq[2]), n.to_array(), 1e-7) && (c.wind_eq[3] - 339.081726).abs() < 2e-4, "{c:?}");
        assert!(c.wind[0] == 0.246283069 && (c.wind[1] - 0.00220525498).abs() < 1e-9, "{c:?}");
        // (the same in a car: no strength)
        w.step(10.0, n, false, 1.0 / 60.0, false);
        assert!(w.speed == 0.0 && w.phase == 0.246283069);
        // (work/checkpoints2/cap/col4.bin at 01:06: the player's Saleen, its record's colour; a racer's R8, its set-up's)
        let c = Cabin::of(Mat4::IDENTITY, [[0.0; 3]; 2], [[0.464063, 0.363281, 0.188281, 1.0], [0.0; 4]], 1.0, false, 1.0, &Wind::default());
        assert!(near(c.gauge_col, [0.401301622, 0.350911021, 0.263410985], 2e-6), "{c:?}");
        // (the set-up `vp_aud_r8_08.xcc`'s 0x3f078788 = 135 / 255)
        let c = Cabin::of(Mat4::IDENTITY, [[0.0; 3]; 2], [[f32::from_bits(0x3f07_8788), f32::from_bits(0x3f07_8788), f32::from_bits(0x3f07_8788), 1.0], [0.0; 4]], 1.0, false, 1.0, &Wind::default());
        assert!(near(c.gauge_col, [0.52941072; 3], 6e-8), "{c:?}");
    }
}

#[cfg(test)]
mod ride_tests {
    use super::*;

    /// F-25740 run offline on 3,012 steering values (work/cameras2/bikegraph/remap_ref.py; the bits as decimal words): equal.
    #[test]
    fn ride_remap_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/bike_ride_remap_ref.txt").lines() {
            let w: Vec<u32> = line.split(' ').map(|s| s.parse().unwrap()).collect();
            assert_eq!(ride_remap(f32::from_bits(w[0])).to_bits(), w[1], "{line}");
            n += 1;
        }
        assert_eq!(n, 3012);
    }

    /// The riding states' transitions (F-71745), in the graph's order: start -> cruise at once; cruise -> stopped idle at
    /// <= 0.15 mph; back only above 0.15 mph + 0.5 m/s; a cockpit flag change re-enters with the 0.01 s blend; wheelie before
    /// tuck; tuck after 1.5 s; burnout centre -> left after 0.75 s; the shift leaves when its overlay is missing; fidget after
    /// 15 s still, look right first.
    /// The pan overlay (F-63058 at the enters): in the human's cruise / stopped idle / burnout centre tree outside the cockpit
    /// only; its smoother waits 0.5 s at the centre, then moves at 5 x dt towards 0.5 - look x 0.5.
    #[test]
    fn ride_pan() {
        let bones = vec![crate::skin::Bone { parent: None, pos: Vec3::ZERO, rot: Quat::IDENTITY, bind: Vec3::ZERO }];
        let anim = Arc::new(mcla_formats::rage::anim::Animation::default());
        let ids = vec![0u16];
        let b = BikeClips::load(|_| Some(Clip::bind(&anim, &ids, false, false))).unwrap();
        let (mut out, mut tmp) = (Vec::new(), Vec::new());
        let dt = 1.0 / 30.0;
        let mut step = |r: &mut Ride, i: RideIn| ride_pose(r, &b, &bones, &[true], &i, 0.0, dt, &mut out, &mut tmp);
        let go = |v: f32| RideIn { speed: v, ..Default::default() };
        let mut r = Ride::default();
        step(&mut r, go(10.0));
        assert!(r.pan);
        // (at the centre with the look there: the hold starts and nothing moves)
        step(&mut r, go(10.0));
        assert_eq!(r.pan_p, Some((0.5, 0.5 - dt)));
        for _ in 0..13 {
            step(&mut r, RideIn { look: 1.0, ..go(10.0) });
        }
        assert_eq!(r.pan_p.unwrap().0, 0.5);
        for _ in 0..3 {
            step(&mut r, RideIn { look: 1.0, ..go(10.0) });
        }
        assert!(r.pan_p.unwrap().0 < 0.5);
        step(&mut r, RideIn { wheelie: true, look: 1.0, ..go(10.0) });
        assert_eq!(r.state, St::Wheelie);
        assert!(!r.pan);
        let held = r.pan_p;
        step(&mut r, RideIn { look: 1.0, ..go(10.0) });
        assert_eq!(r.pan_p, held);
        let mut o = Ride::default();
        step(&mut o, RideIn { opponent: true, ..go(10.0) });
        assert!(!o.pan);
        let mut c = Ride::default();
        step(&mut c, RideIn { ck: true, ..go(10.0) });
        assert!(!c.pan);
    }

    /// F-70536: a racer 5 m to the left is looked at (+1) for 1 s, then the camera's look until 5 s, then it is let go; one
    /// ahead is not looked at.
    #[test]
    fn rival_look() {
        let me = (Vec3::ZERO, Quat::IDENTITY);
        let mut l = RivalLook::default();
        assert_eq!(l.step(0.25, 0.25, me, [(3, Vec3::new(0.0, 0.0, -6.0))].into_iter()), 0.25);
        assert_eq!(l.rival, None);
        assert_eq!(l.step(0.25, 0.25, me, [(3, Vec3::new(0.0, 0.0, -6.0)), (4, Vec3::new(-5.0, 0.0, -1.0))].into_iter()), 1.0);
        assert_eq!(l.rival, Some(4));
        for _ in 0..3 {
            assert_eq!(l.step(0.25, 0.25, me, std::iter::empty()), 1.0);
        }
        assert_eq!(l.step(0.25, 0.25, me, std::iter::empty()), 0.25);
        for _ in 0..16 {
            l.step(0.25, 0.25, me, std::iter::empty());
        }
        assert_eq!(l.rival, Some(4));
        l.step(0.25, 0.25, me, std::iter::empty());
        assert_eq!(l.rival, None);
        assert_eq!(l.step(0.0, 0.25, me, [(5, Vec3::new(4.0, 0.0, 0.0))].into_iter()), -1.0);
    }

    #[test]
    fn ride_states() {
        let bones = vec![crate::skin::Bone { parent: None, pos: Vec3::ZERO, rot: Quat::IDENTITY, bind: Vec3::ZERO }];
        let anim = Arc::new(mcla_formats::rage::anim::Animation::default());
        let ids = vec![0u16];
        let b = BikeClips::load(|_| Some(Clip::bind(&anim, &ids, false, false))).unwrap();
        let (mut out, mut tmp) = (Vec::new(), Vec::new());
        let mut r = Ride::default();
        let dt = 1.0 / 30.0;
        let mut step = |r: &mut Ride, i: RideIn| ride_pose(r, &b, &bones, &[false], &i, 0.0, dt, &mut out, &mut tmp);
        let go = |v: f32| RideIn { speed: v, ..Default::default() };
        step(&mut r, go(0.0));
        assert_eq!((r.state, r.rate), (St::Cruise, 1.0 / 0.45));
        step(&mut r, go(0.0));
        assert_eq!((r.state, r.rate), (St::Stopped, 1.0 / 0.35));
        step(&mut r, go(0.5));
        assert_eq!(r.state, St::Stopped);
        step(&mut r, go(0.6));
        assert_eq!((r.state, r.rate), (St::Cruise, 1.0 / 0.45));
        step(&mut r, RideIn { ck: true, ..go(10.0) });
        assert_eq!((r.state, r.rate), (St::Cruise, 1.0 / 0.01));
        step(&mut r, RideIn { ck: true, ..go(0.05) });
        assert_eq!((r.state, r.rate), (St::Stopped, 1.0 / 0.35));
        // (the rolling rule comes before the self-transition: out to cruise, with the edge's blend)
        step(&mut r, go(10.0));
        assert_eq!((r.state, r.rate), (St::Cruise, 1.0 / 0.01));
        step(&mut r, go(10.0));
        assert_eq!(r.state, St::Cruise);
        step(&mut r, RideIn { wheelie: true, tuck: true, ..go(10.0) });
        assert_eq!((r.state, r.timer_a), (St::Wheelie, 0.0));
        step(&mut r, RideIn { all_down: true, ..go(10.0) });
        assert_eq!(r.state, St::Cruise);
        for _ in 0..50 {
            step(&mut r, RideIn { tuck: true, ..go(10.0) });
        }
        assert_eq!(r.state, St::Tuck);
        step(&mut r, go(10.0));
        step(&mut r, RideIn { burnout: true, ..go(0.0) });
        assert_eq!(r.state, St::BurnC);
        step(&mut r, RideIn { burnout: true, goal_steer: -0.6, ..go(0.0) });
        assert_eq!(r.state, St::BurnC);
        for _ in 0..30 {
            step(&mut r, RideIn { burnout: true, goal_steer: -0.6, ..go(0.0) });
        }
        assert_eq!(r.state, St::BurnL);
        step(&mut r, go(10.0));
        assert_eq!(r.state, St::Cruise);
        // (an empty clip: no overlay time left, the shift goes back at once)
        step(&mut r, RideIn { shift: 1, ..go(10.0) });
        assert_eq!(r.state, St::Shift);
        step(&mut r, go(10.0));
        assert_eq!(r.state, St::Cruise);
        step(&mut r, go(0.0));
        assert_eq!(r.state, St::Stopped);
        let mut n = 0;
        while r.fidgets == 0 {
            step(&mut r, go(0.0));
            n += 1;
        }
        // (after 15 s still: timer A over 15 - 450 frames of a float 1/30 s)
        assert_eq!((r.state, n), (St::Fidget, 450));
        step(&mut r, go(0.0));
        assert_eq!((r.state, r.timer_a), (St::Stopped, 0.0));
        step(&mut r, RideIn { reversing: true, ..go(-1.0) });
        assert_eq!((r.state, r.rate, r.fade2.map(|f| f.1)), (St::Reverse, 1.0 / 0.35, Some(4.0)));
        step(&mut r, RideIn { throttle: 1.0, ..go(0.5) });
        assert_eq!(r.state, St::Cruise);
    }
}
