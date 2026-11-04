//! Drivers and riders: the `drv_*_lod02_set` characters posed with each car's own driver
//! animations (`resources/animation/<hash>/opponent/male/*.r2`: one pack per car with
//! `<CAR>_Drv_steer_sweep`, `_Drv_Idle_stp`, `_Drv_Idle_Low`, `_Drv_rev`, ...). The
//! animations are authored in car space, so the root track seats the driver.

use crate::assets::load_resource_file;
use crate::render::{Frustum, Gpu};
use crate::skin::{blend_pose, Clip, SkinBuffer, SkinModel};
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
}

/// The clips of mcMotorcycleAnimMachine's start / cruise / stopped idle states (start action F-88016, cruise enter F-27733,
/// stopped idle enter F-73617): the low and fast lean sweeps, the stopped-idle sweep and their cockpit-camera
/// (`_ckpt`) variants, and the start's idle.
struct BikeClips {
    idle_low: Option<Clip>,
    low: Clip,
    fst: Clip,
    low_ck: Option<Clip>,
    fst_ck: Option<Clip>,
    stp: Option<Clip>,
    stp_ck: Option<Clip>,
}

/// A rider's state in the bike pose machine (the riding part of its graph, F-71745): which state, the cockpit-camera flag of the
/// last frame (its change is the edge the enters and the self-transitions test), the crossfade of the last state change and the
/// pose it fades from.
#[derive(Default)]
struct Ride {
    /// 0 "start here", 1 "cruise", 2 "stopped idle"
    state: u8,
    ck: bool,
    fade: f32,
    rate: f32,
    /// the old tree under the crossfade: the start action's looping `drv_idle_low` (its time) or the frozen pose
    from_idle: Option<f32>,
    from: Vec<(Quat, Vec3)>,
    last: Vec<(Quat, Vec3)>,
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

/// One frame of the bike pose machine's riding states for a rider (`Ride`), the pose into `out`:
/// - "start here" goes to "cruise" at once, its action playing `drv_idle_low` looped with no blend (F-88016); the
///   cruise enter then fades from it.
/// - "cruise" (enter F-27733, update F-61324): the low and fast lean sweeps (`_ckpt` with the cockpit camera) as
///   a pair, the fast one's weight clamp((speed - 17.8816) x 0.2236936, 0, 1), both frozen at the remapped steering phase; entered
///   over 0.45 s, 0.01 s when the cockpit flag changed this frame. Leaves for "stopped idle" when speed <= 0.15 mph (tune +48 x
///   0.44704, F-49190), else re-enters itself on a cockpit flag change.
/// - "stopped idle" (enter F-73617, update F-63745): `drv_idle_stp_sweep` (`_ckpt`) frozen at phase
///   clamp(x, -0.5, 0.5) + 0.5; entered over 0.35 s (0.01 s on a cockpit flag change). Leaves for "cruise" when
///   speed > 0.15 mph + 0.5 (the graph's condition on StoppedRollingBandCutoff, F-78470), else re-enters itself on a change.
/// A state change fades linearly (rate 1 / blend time, the first frame at rate x dt) from the old tree, which stays frozen
/// (the start's idle keeps looping). The machine's other states (tuck, wheelie, endo, burnouts, nos, shift, reverse, impacts,
/// fidget, gesture switch) and the pan-sweep overlay are not ported (docs/PEOPLE_TRAFFIC.md).
#[allow(clippy::too_many_arguments)]
fn ride_pose<'a>(r: &mut Ride, b: &'a BikeClips, bones: &[crate::skin::Bone], ck: bool, speed: f32, x: f32, time: f32, dt: f32, out: &mut Vec<(Quat, Vec3)>, tmp: &mut Vec<(Quat, Vec3)>) {
    const STOPPED: f32 = 0.15 * 0.44704;
    let edge = ck != r.ck && r.state != 0;
    r.ck = ck;
    let enter = |r: &mut Ride, state: u8, blend: f32, from_idle: Option<f32>| {
        r.state = state;
        r.fade = 0.0;
        r.rate = 1.0 / if edge { 0.01 } else { blend };
        r.from_idle = from_idle;
        r.from = r.last.clone();
    };
    match r.state {
        0 => enter(r, 1, 0.45, Some(time)),
        1 if speed <= STOPPED => enter(r, 2, 0.35, None),
        1 if edge => enter(r, 1, 0.45, None),
        2 if speed > STOPPED + 0.5 => enter(r, 1, 0.45, None),
        2 if edge => enter(r, 2, 0.35, None),
        _ => {}
    }
    r.fade = (r.fade + r.rate * dt).min(1.0);
    let pick = |c: &'a Option<Clip>, d: &'a Clip| if ck { c.as_ref().unwrap_or(d) } else { d };
    if r.state == 1 {
        let (low, fst) = (pick(&b.low_ck, &b.low), pick(&b.fst_ck, &b.fst));
        let ph = ride_remap(x);
        let w = ((speed - 17.8816) * 0.223_693_64).max(0.0).min(1.0);
        low.local(bones, ph * low.last_frame(), out);
        fst.local(bones, ph * fst.last_frame(), tmp);
        blend_pose(out, tmp, w);
    } else {
        let stp = if ck { b.stp_ck.as_ref().or(b.stp.as_ref()) } else { b.stp.as_ref() };
        let c = stp.unwrap_or(&b.low);
        let ph = x.clamp(-0.5, 0.5) + 0.5;
        c.local(bones, ph * c.last_frame(), out);
    }
    if r.fade < 1.0 {
        match (r.from_idle, b.idle_low.as_ref()) {
            (Some(t0), Some(idle)) => idle.local(bones, idle.frame(time - t0, true), tmp),
            _ => tmp.clone_from(&r.from),
        }
        if tmp.len() == out.len() {
            blend_pose(tmp, out, r.fade);
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
    /// car key ("frd_mustang_06", "vpd_chv_impala_64") -> animation pack
    packs: HashMap<String, PathBuf>,
    /// driver sets: (name, bike rider, police)
    sets: Vec<(String, bool, bool)>,
    models: HashMap<usize, Option<Arc<SkinModel>>>,
    rigs: HashMap<(String, usize), Option<Rig>>,
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

/// "vp_frd_mustang_06_dub" -> "frd_mustang_06"; DLC cars keep their "vpd_" prefix.
fn car_key(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    let n = n.trim_end_matches("_dub");
    n.strip_prefix("vp_").unwrap_or(n).to_string()
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
            // the in-car versions (`_lod02`, and LoadDriver's hi-res `_set_h`, cinedriver::DRIVER_MAPS); male ones (the
            // packs used here are the male set)
            if !(l.ends_with("lod02_set") || l.ends_with("lod2_set") || l.ends_with("_set_h")) || !(l.starts_with("drv_m") || l.starts_with("pol_")) {
                continue;
            }
            self.sets.push((n, l.contains("_mot_"), l.starts_with("pol_")));
        }
        // which pack belongs to which car: read once, then kept in a small index file
        let index = root.join("driver_packs.tsv");
        if let Ok(t) = std::fs::read_to_string(&index) {
            for line in t.lines() {
                if let Some((k, f)) = line.split_once('\t') {
                    self.packs.insert(k.to_string(), PathBuf::from(f));
                }
            }
        }
        // (the index holds full paths: after the game folder was moved, or the data rebuilt elsewhere, they
        // point nowhere and no driver loads - every car seen empty, GitHub #24; then it is made again)
        if self.packs.values().any(|f| !f.is_file()) {
            log::warn!("drivers: {} names packs that are not there - indexing again", index.display());
            self.packs.clear();
        }
        if self.packs.is_empty() {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("resources/animation")).map(|d| d.flatten().map(|e| e.path().join("opponent/male")).filter(|p| p.is_dir()).collect()).unwrap_or_default();
            dirs.sort();
            let mut out = String::new();
            for dir in dirs {
                let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
                files.sort();
                for f in files {
                    let Ok(res) = load_resource_file(&f) else { continue };
                    let mut count: HashMap<String, u32> = HashMap::new();
                    for (_, name) in pack_entries(&res) {
                        let l = name.to_ascii_lowercase();
                        let Some(i) = l.find("_fdrv_").or_else(|| l.find("_drv_")) else { continue };
                        *count.entry(l[..i].to_string()).or_default() += 1;
                    }
                    // every pack also carries the shared Challenger gestures
                    let mut v: Vec<(String, u32)> = count.into_iter().collect();
                    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                    let key = v.iter().find(|(k, n)| k != "dge_challenger_70" && *n >= 10).or(v.first()).map(|(k, _)| k.clone());
                    if let Some(k) = key {
                        if !self.packs.contains_key(&k) {
                            out.push_str(&format!("{}\t{}\n", k, f.display()));
                            self.packs.insert(k, f);
                        }
                    }
                }
            }
            let _ = std::fs::write(&index, out);
        }
        log::info!("drivers: {} character sets, {} animation packs", self.sets.len(), self.packs.len());
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
            let want = crate::cinedriver::driver_set(s.set, false, s.bike).unwrap_or(s.set);
            if let Some(i) = self.sets.iter().position(|n| n.0.eq_ignore_ascii_case(want)) {
                return Some(i);
            }
        }
        let want = |n: &(String, bool, bool)| n.1 == s.bike && n.2 == (s.cop && !s.bike) && !n.0.ends_with("_set_h");
        if s.player {
            // "mp" is the player character
            if let Some(i) = self.sets.iter().position(|n| n.0.to_ascii_lowercase().starts_with("drv_mp_") && n.1 == s.bike && !n.0.to_ascii_lowercase().ends_with("_set_h")) {
                return Some(i);
            }
        }
        let c: Vec<usize> = (0..self.sets.len()).filter(|&i| want(&self.sets[i]) && !self.sets[i].0.to_ascii_lowercase().starts_with("drv_mp_")).collect();
        if c.is_empty() {
            return self.sets.iter().position(|n| n.1 == s.bike && !n.0.to_ascii_lowercase().ends_with("_set_h"));
        }
        // (by the car's own number: the place in the list moved when cars went, and the driver - his
        // face - changed with it; GitHub #17)
        Some(c[hash(s.uid + 17) as usize % c.len()])
    }

    /// The length of a car's DRV_HIT_HRD clip, once its driver is loaded.
    pub fn hit_hard_secs(&self, car: &str) -> Option<f32> {
        let k = car_key(car);
        self.rigs.iter().filter(|((c, _), _)| *c == k).find_map(|(_, r)| r.as_ref()?.hit_hard)
    }

    fn rig(&mut self, gpu: &mut Gpu, car: &str, si: usize) -> bool {
        let key = (car_key(car), si);
        if !self.rigs.contains_key(&key) {
            let rig = (|| -> Option<Rig> {
                let model = self.model(gpu, si)?;
                let pack = self.packs.get(&key.0).or_else(|| self.packs.get("dge_challenger_70"))?.clone();
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
                let bike = match (clip(&["drv_low_lean_sweep"]), clip(&["drv_fst_lean_sweep"])) {
                    (Some(low), Some(fst)) => Some(BikeClips {
                        idle_low: clip(&["drv_idle_low"]),
                        low,
                        fst,
                        low_ck: clip(&["drv_low_lean_sweep_ckpt"]),
                        fst_ck: clip(&["drv_fst_lean_sweep_ckpt"]),
                        stp: clip(&["drv_idle_stp_sweep"]),
                        stp_ck: clip(&["drv_idle_stp_sweep_ckpt"]),
                    }),
                    _ => None,
                };
                Some(Rig { steer, idle_stop: clip(&["drv_idle_stp"]), reverse: clip(&["drv_rev"]), model, gestures, hit_hard, ejects, bike })
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
            if !self.rig(gpu, s.car, si) {
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
            let Some(Some(rig)) = self.rigs.get(&(car_key(s.car), si)) else { continue };
            if let (true, Some(b)) = (s.bike, rig.bike.as_ref()) {
                let ride = self.rides.entry(s.id).or_default();
                ride_pose(ride, b, &rig.model.bones, s.player && self.ckpt_view, s.speed, p.steer.clamp(-1.0, 1.0), self.time, dt, &mut local, &mut other);
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
                    let name = gesture_clip(g, &mut a.rr, human);
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

    /// The riding states' transitions (F-71745): start -> cruise at once; cruise -> stopped idle at <= 0.15 mph; back to
    /// cruise only above 0.15 mph + 0.5 m/s; a cockpit flag change re-enters the state with the 0.01 s blend.
    #[test]
    fn ride_states() {
        let mk = || {
            let bones = vec![crate::skin::Bone { parent: None, pos: Vec3::ZERO, rot: Quat::IDENTITY, bind: Vec3::ZERO }];
            let anim = Arc::new(mcla_formats::rage::anim::Animation::default());
            let ids = vec![0u16];
            let c = || Clip::bind(&anim, &ids, false, false);
            (bones, BikeClips { idle_low: None, low: c(), fst: c(), low_ck: None, fst_ck: None, stp: None, stp_ck: None })
        };
        let (bones, b) = mk();
        let (mut out, mut tmp) = (Vec::new(), Vec::new());
        let mut r = Ride::default();
        let mut step = |r: &mut Ride, ck: bool, v: f32| ride_pose(r, &b, &bones, ck, v, 0.0, 0.0, 1.0 / 30.0, &mut out, &mut tmp);
        step(&mut r, false, 0.0);
        assert_eq!((r.state, r.rate), (1, 1.0 / 0.45));
        step(&mut r, false, 0.0);
        assert_eq!((r.state, r.rate), (2, 1.0 / 0.35));
        step(&mut r, false, 0.5);
        assert_eq!(r.state, 2);
        step(&mut r, false, 0.6);
        assert_eq!((r.state, r.rate), (1, 1.0 / 0.45));
        step(&mut r, true, 10.0);
        assert_eq!((r.state, r.rate), (1, 1.0 / 0.01));
        step(&mut r, true, 0.05);
        assert_eq!((r.state, r.rate), (2, 1.0 / 0.35));
    }
}
