//! The cars' sounds beside the engine and the collisions, as the original's car audio entity makes them
//! (mcCarAudioEntity, base update F-36837): the tyres' rolling, skid and chirp sounds by the surface's MATERIALS_*
//! object, the suspension's damper and bottom-out one-shots, the hydraulics' rams and rocking, the wipeout and
//! two-wheel loops, the horn, the police siren, the turbo whine and blow-off, the nitrous loops and the damage loop;
//! the computer racers' engines (the 7 nearest, `_O_` objects) and the ambient traffic's engines, tyres, skids and
//! horns (mcAmbientVehicleAudioEntity). Rules: `docs/reference/audio/VEHICLE_EXTRA_AUDIO_RULE.md`.
//!
//! Also the police's sirens, warn, chirp, bullhorn and chatter (SIREN_TRIGGER_RULE.md), the traffic's selection,
//! horns, skids, alarms, driver speech and pass-by whooshes (TRAFFIC_AUDIO_RULE.md), the wet road (WEATHER_AUDIO_RULE.md)
//! and the bikes (BIKE_AUDIO_RULE.md). Approximate (written down there): the opponents' RPM_RACE_OPPONENT blend uses the
//! speed term only; the traffic brain's obstacle, lane limit and emergency brake are the port's stand-ins; the junction
//! warn uses the port's signalled junctions; the wipeout loops follow the port's wipeout flag; air ride is not played. The hood / trunk / bumper sounds follow the port's
//! moving damage parts (`damageparts.rs`, DAMAGE_PARTS_RULE.md 1.5, 3, 4); the fallen bumper's contacts are its
//! small body's against the ground (approximate).

use crate::engaudio::{default_object, EngineIn, EngineSound};
use crate::physics::collision::Surface;
use crate::sndobj::{Handle, Req, SoundLib};
use crate::vehicles::{CarInstance, Vehicles};
use glam::Vec3;
use hashbrown::HashMap;
use mcla_formats::audcfg::lin2db;
use mcla_formats::hash::joaat;
use std::path::Path;

const MPH: f32 = 2.237;

/// MCLA_CAR_AUDIO=1: a line a second on stderr with what the cars' sounds are running
fn trace() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| std::env::var_os("MCLA_CAR_AUDIO").is_some())
}

fn be_u32(o: &[u8], at: usize) -> u32 {
    o.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).unwrap_or(0)
}
fn be_f32(o: &[u8], at: usize) -> f32 {
    f32::from_bits(be_u32(o, at))
}
fn be_s16(o: &[u8], at: usize) -> f32 {
    o.get(at..at + 2).map(|b| i16::from_be_bytes(b.try_into().unwrap()) as f32).unwrap_or(0.0)
}
fn be_u16(o: &[u8], at: usize) -> f32 {
    o.get(at..at + 2).map(|b| u16::from_be_bytes(b.try_into().unwrap()) as f32).unwrap_or(0.0)
}

fn is_null(h: u32) -> bool {
    h == 0 || h == joaat("NULL_SOUND")
}

/// Linear step toward a target, clamped there (F-90102).
fn approach(p: &mut f32, target: f32, step: f32) {
    approach_fma(p, target, step, 1.0);
}

/// F-90102 (audio2 2026-10-27): `p` steps toward `target` by `a * b` with the multiply and the add fused
/// (the original's fmadds / fnmsubs), clamped at the target. Verified bit for bit against the original (b1 evidence).
fn approach_fma(p: &mut f32, target: f32, a: f32, b: f32) {
    if *p < target {
        *p = a.mul_add(b, *p).min(target);
    } else if *p > target {
        *p = (-a).mul_add(b, *p).max(target);
    }
}

/// `map(curve, lo, hi, x0, x1, x)` = F-82373.
fn map(lib: &SoundLib, curve: u32, lo: f32, hi: f32, x0: f32, x1: f32, x: f32) -> f32 {
    let t = if x <= x0 {
        0.0
    } else if x >= x1 {
        1.0
    } else {
        (x - x0) / (x1 - x0)
    };
    lo + (hi - lo) * lib.curve_eval(curve, t).unwrap_or(t)
}

fn cents(ratio: f32) -> i32 {
    (1200.0 * ratio.max(1e-6).log2()) as i32
}

/// F-46044: the original's log10 in double, `ln x * 0.4342944819032518` (F-96865 is its own ln routine).
fn log10_guest(x: f64) -> f64 {
    ln_guest(x) * 0.4342944819032518
}

/// The original's own natural logarithm (F-96865, a rational polynomial in double on the mantissa in [0.5, 1) and the
/// exponent), same operations and fused multiply-adds, for positive finite x (other inputs fall back to the library).
fn ln_guest(x: f64) -> f64 {
    if x == 1.0 {
        return 0.0;
    }
    if !(x > 0.0 && x <= f64::MAX) {
        return x.ln();
    }
    let (mut bits, mut e) = (x.to_bits(), 0i32);
    if x >= f64::MIN_POSITIVE {
        e = ((bits >> 52) & 0x7ff) as i32 - 1022;
    } else {
        bits = (x * 9007199254740992.0).to_bits();
        e = ((bits >> 52) & 0x7ff) as i32 - 1075 + e;
    }
    let m = f64::from_bits((bits & 0x800f_ffff_ffff_ffff) | 0x3fe0_0000_0000_0000);
    let (f, den) = if m > 0.7071067811865476 {
        ((m - 0.5) - 0.5, (m + 1.0) * 0.5)
    } else {
        e -= 1;
        let f = m - 0.5;
        (f, (f + 1.0) * 0.5)
    };
    let s = f / den;
    let z = s * s;
    let ef = e as f64;
    let (c28, c40) = (f64::from_bits(0x4030624a2016afed), f64::from_bits(0x40738083fa15267e));
    let (c70, c68, c60, c58, c50) = (f64::from_bits(0x3fe94415b356bd29), f64::from_bits(0x4041d5804b67ce0f), f64::from_bits(0x405007ff12b3b59a), f64::from_bits(0x40880bfe9c0d9077), f64::from_bits(0x3f2bd0105c610ca8));
    let t0 = ef * c50;
    let a = (-z).mul_add(c70, c28);
    let b = z - c68;
    let a = a.mul_add(z, -c60);
    let b = b.mul_add(z, c40);
    let a = a * z;
    let b = b.mul_add(z, -c58);
    let q = a / b + 1.0;
    let r = q.mul_add(s, -t0);
    ef.mul_add(0.693359375, r)
}

/// A pitch ratio as F-98474 turns it into the voice's cents (+80): log10 in double (F-46044 = ln x
/// 0.4342944819032518, 0x8205D018), rounded to single, times 3.321928 (0x82002C44) and 1200 (0x82002C40) in single,
/// truncated (fctiwz). (Recorded on the slipstream loop: 292 updates, every value equal - work/slipstream/audiorec.)
fn voice_cents(ratio: f32) -> i32 {
    pitch_cents_f32(ratio) as i32
}

/// The cents of a pitch ratio before the truncation: `float(log10) * 3.321928 * 1200`, single precision.
pub(crate) fn pitch_cents_f32(ratio: f32) -> f32 {
    let l = log10_guest(ratio.max(1e-6) as f64) as f32;
    l * f32::from_bits(0x4054_9A78) * 1200.0
}

/// MCLA_SST_REPLAY: the recording's update the player's sounds are fed this frame (-1: no replay)
static SST_REPLAY_AT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

/// MCLA_SST_AUDIO_LOG=<file>: the slipstream sounds' updates and events in the recorder's own line format
/// (work/slipstream/audiorec, recomp-trace src/sst_audio.cpp), for a line-by-line comparison.
fn sst_log(line: std::fmt::Arguments) {
    use std::io::Write;
    static F: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    if let Some(f) = F.get_or_init(|| std::env::var_os("MCLA_SST_AUDIO_LOG").and_then(|p| std::fs::File::create(p).ok()).map(std::sync::Mutex::new)) {
        let _ = writeln!(f.lock().unwrap(), "{line}");
    }
}

/// A persistent sound in a handle slot: start it when the slot is empty (or its sound ended), else update it.
fn keep(lib: &mut SoundLib, slot: &mut Option<Handle>, hash: u32, req: Req) {
    match *slot {
        Some(h) if lib.playing(h) => lib.set(h, req),
        _ => *slot = if is_null(hash) { None } else { lib.play_hash(hash, req) },
    }
}

fn stop(lib: &mut SoundLib, slot: &mut Option<Handle>) {
    if let Some(h) = slot.take() {
        lib.stop(h);
    }
}

fn free(lib: &SoundLib, slot: &Option<Handle>) -> bool {
    slot.map_or(true, |h| !lib.playing(h))
}

/// Physics material -> its audio material object (`AudioGameObject` of `tune/physics/materials`) and ptxthreshold[0].
pub struct Materials(HashMap<u8, (u32, f32)>);

impl Materials {
    pub fn load(data_dir: &Path) -> Materials {
        let mut m = HashMap::new();
        let dir = data_dir.join("tune").join("physics").join("materials");
        for s in Surface::ALL {
            let name = s.material();
            let txt = [dir.join(format!("0x{:08x}", joaat(&format!("{name}.mtl")))), dir.join(format!("0x{:08x}", joaat(name)))].iter().find_map(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
            let field = |k: &str| txt.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_string()));
            let obj = field("AudioGameObject:").unwrap_or_else(|| "MATERIALS_DEFAULT".into());
            let thr = field("ptxthreshold:").and_then(|v| v.split_whitespace().next().and_then(|x| x.parse().ok())).unwrap_or(0.3);
            m.insert(s as u8, (joaat(&obj), thr));
        }
        Materials(m)
    }

    fn get(&self, s: u8) -> (u32, f32) {
        self.0.get(&s).copied().unwrap_or((joaat("MATERIALS_DEFAULT"), 0.3))
    }
}

/// A wheel as the car's sounds read it.
#[derive(Clone, Copy, Default)]
pub struct WheelIn {
    /// it has a contact with a material (wheel +529, +464)
    pub contact: bool,
    pub surface: u8,
    /// |forward contact velocity| in mph (wheel +296)
    pub speed_mph: f32,
    /// the tyre's slide (wheel +520)
    pub slide: f32,
    pub pos: Vec3,
    /// suspension compression and its rate (wheel +496, +504); SuspensionLimit (tune +84) and the travel (+88 + +84)
    pub comp: f32,
    pub comp_rate: f32,
    pub limit: f32,
    pub travel: f32,
    /// on the driven axle (record +40)
    pub driven: bool,
}

/// What one car's sounds read each frame.
#[derive(Clone, Copy, Default)]
pub struct CarIn {
    pub pos: Vec3,
    /// the wheels in the original's record order (car +28: FL, FR, RL, RR - read from the running original; a bike front,
    /// rear) and how many there are
    pub wheels: [WheelIn; 4],
    pub n_wheels: usize,
    /// the weather's wet amount is not 0 (WEATHER_AUDIO_RULE.md 1.3)
    pub wet: bool,
    /// a motorcycle and its gyro's stunt state (gyro +16: 0, 1 wheelie, 99, -1 stoppie, -99)
    pub bike: bool,
    pub gyro: i16,
    /// burnout charge > 0 (car +580)
    pub burnout: bool,
    /// engine load = torque / full-throttle torque (this+164)
    pub load: f32,
    /// mcCarDamage normalised (2.2)
    pub damage: f32,
    pub hyd: Option<[i8; 4]>,
    pub hyd_on: bool,
    pub ang_vel: Vec3,
    pub air_time: f32,
    pub wipeout: bool,
    pub two_wheels: bool,
    pub horn: bool,
    pub nitro_firing: bool,
    pub nitro_empty: bool,
    /// the slipstream turbo (car +668, mcCarSSTurbo): its level (+36) and whether it burns (+40); None = no object
    pub sst: Option<(f32, bool)>,
    /// performance stages: intake, turbo, nitrous (mcCar+132 +30, +31, +34)
    pub parts: [u8; 3],
    /// the engine sound's smoothed rpm and load volume (this+204, +148), the limiter's rpm
    pub rpm_s: f32,
    pub load_vol: f32,
    pub max_rpm: f32,
    /// throttle input (this+188), pedal (this+180), the transmission's shift state
    pub throttle: f32,
    pub pedal: f32,
    pub shift: i8,
}

#[derive(Default)]
struct WheelRec {
    roll: Option<Handle>,
    skid: Option<Handle>,
    /// material object of the last update (0 = none)
    mat: u32,
    s: f32,
    prev: f32,
    t: f32,
    ratio: f32,
    gain: f32,
}

/// One racing car's sounds (player or computer racer).
pub struct CarSounds {
    player: bool,
    /// ONESHOT_PARAMS +92: the pass-by whoosh (F-50805) and when it last played (5 s apart)
    whoosh_hash: u32,
    whoosh_at: f32,
    skid_db: f32,
    skid_cents: i32,
    susp: Vec<u8>,
    turbo: Vec<u8>,
    damage_obj: Vec<u8>,
    horn_hash: u32,
    nitro_obj: Vec<u8>,
    police: Vec<u8>,
    w: [WheelRec; 5],
    clock: f32,
    chirp_at: f32,
    upshift_at: f32,
    prev_shift: i8,
    damper: [Option<Handle>; 4],
    bottom: [Option<Handle>; 4],
    hyd_prev: [i8; 4],
    hyd_loop: Option<(Handle, u8)>,
    hyd_timer: f32,
    two_loop: Option<Handle>,
    two_end: bool,
    prev_two: bool,
    horn: Option<Handle>,
    /// slot 1400 (alert / intersection / siren loop), siren flag (byte 1949), its pitch, the AI's start delay
    siren: Option<Handle>,
    siren_on: bool,
    siren_pitch: i32,
    siren_delay: Option<f32>,
    /// cop+1144: the junction warn runs (seconds)
    warn: f32,
    turbo_v: Option<Handle>,
    boost: f32,
    spool: f32,
    prev_throttle: f32,
    nitro: Option<(Option<Handle>, Option<Handle>)>,
    nitro_empty: Option<Handle>,
    /// GLOBAL_SPECIAL_ABILITIES (entity+1252) and the slipstream turbo's slots: 1408 the burn, 1412 the charge loop,
    /// 1416 the full charge's beep
    sst_obj: Vec<u8>,
    sst_burn: Option<Handle>,
    sst_loop: Option<Handle>,
    sst_full: Option<Handle>,
    /// this frame the player's burn asked for PLAYER_NITROUS (F-58518: mix slot 18 was not on yet)
    pub sst_speech: bool,
    /// MCLA_CAR_AUDIO trace: the slots last printed and when
    sst_seen: (u8, f32),
    /// the charge loop's last volume (dB) and pitch (cents)
    sst_vp: (f32, i32),
    /// the slipstream update's count (MCLA_SST_AUDIO_LOG lines)
    sst_f: u64,
    pings: Option<Handle>,
    /// the moving damage parts: the hood / trunk move loops, the bumpers' creak loops (rear, front), the fallen
    /// bumpers' scrape loops with the time of their last sliding contact (inst+96, +100), the last event heard
    flap_move: [Option<Handle>; 2],
    creak: [Option<Handle>; 2],
    scrape: [(Option<Handle>, f32); 2],
    parts_seen: Option<u64>,
    /// GLOBAL_CAR_PARTICLES +18 (CAR_BANGERS_GLASS_LARGE): a window cracking or breaking
    glass_hash: u32,
    /// the last window event heard (damagemap::CarDamage::glass)
    glass_seen: Option<u64>,
}

impl CarSounds {
    pub fn new(lib: &SoundLib, car: &str, bike: bool, player: bool) -> CarSounds {
        let g = &lib.game;
        let def = default_object(lib, car, bike, player).map(|d| d.to_vec()).unwrap_or_default();
        let obj = |at: usize, fallback: &str| g.get(be_u32(&def, at)).or_else(|| g.get_name(fallback)).map(|o| o.to_vec()).unwrap_or_default();
        let skids = obj(22, if bike { "MOTORCYCLES_SKIDS" } else { "GLOBAL_SKIDS" });
        let oneshot = obj(18, "");
        CarSounds {
            player,
            skid_db: be_s16(&skids, 10) * 0.01,
            skid_cents: be_s16(&skids, 12) as i32,
            susp: obj(26, "TUNERS_SUSPENSION"),
            turbo: obj(46, "TUNERS_TURBO"),
            damage_obj: obj(54, "TUNERS_DAMAGE"),
            horn_hash: be_u32(&oneshot, 64),
            whoosh_hash: be_u32(&oneshot, 92),
            whoosh_at: -10.0,
            nitro_obj: obj(42, "TUNERS_NITRO"),
            police: g.get_name("GLOBAL_POLICE").map(|o| o.to_vec()).unwrap_or_default(),
            w: Default::default(),
            clock: 0.0,
            chirp_at: 0.0,
            upshift_at: -10.0,
            prev_shift: 0,
            damper: [None; 4],
            bottom: [None; 4],
            hyd_prev: [0; 4],
            hyd_loop: None,
            hyd_timer: 0.0,
            two_loop: None,
            two_end: false,
            prev_two: false,
            horn: None,
            siren: None,
            siren_on: false,
            siren_pitch: 0,
            siren_delay: None,
            warn: 0.0,
            turbo_v: None,
            boost: 0.0,
            spool: 0.0,
            prev_throttle: 0.0,
            nitro: None,
            nitro_empty: None,
            sst_obj: g.get_name("GLOBAL_SPECIAL_ABILITIES").map(|o| o.to_vec()).unwrap_or_default(),
            sst_burn: None,
            sst_loop: None,
            sst_full: None,
            sst_speech: false,
            sst_seen: (0, 0.0),
            sst_vp: (0.0, 0),
            sst_f: 0,
            pings: None,
            flap_move: [None; 2],
            creak: [None; 2],
            scrape: [(None, 0.0); 2],
            parts_seen: None,
            glass_hash: g.get_name("GLOBAL_CAR_PARTICLES").map_or(0, |o| be_u32(o, 18)),
            glass_seen: None,
        }
    }

    fn req(&self, pos: Vec3) -> Req {
        Req { player: self.player, ..Req::at(pos) }
    }

    /// Every sound of the car stops (the car left or its entity was released).
    pub fn stop_all(&mut self, lib: &mut SoundLib) {
        for r in self.w.iter_mut() {
            stop(lib, &mut r.roll);
            stop(lib, &mut r.skid);
        }
        for s in self.damper.iter_mut().chain(self.bottom.iter_mut()) {
            stop(lib, s);
        }
        if let Some((h, _)) = self.hyd_loop.take() {
            lib.stop(h);
        }
        for s in [&mut self.two_loop, &mut self.horn, &mut self.siren, &mut self.turbo_v, &mut self.nitro_empty, &mut self.pings, &mut self.sst_burn, &mut self.sst_loop, &mut self.sst_full] {
            stop(lib, s);
        }
        if self.player {
            lib.scene("DYNAMIC_MIXER_SST", false);
        }
        for s in self.flap_move.iter_mut().chain(self.creak.iter_mut()) {
            stop(lib, s);
        }
        for (s, _) in self.scrape.iter_mut() {
            stop(lib, s);
        }
        if let Some((a, b)) = self.nitro.take() {
            for mut s in [a, b] {
                stop(lib, &mut s);
            }
        }
    }

    /// The base update's parts in the original's order (tyres, suspension, hydraulics, turbo, horn, nitrous, damage).
    pub fn update(&mut self, lib: &mut SoundLib, mats: &Materials, c: &CarIn, cam_mode: i32, dt: f32) {
        self.clock += dt;
        if c.shift > 0 && self.prev_shift == 0 {
            // the upshift's start (this+260, F-33101 @A-66878)
            self.upshift_at = self.clock;
        }
        self.turbo(lib, c, dt);
        self.tyres(lib, mats, c, dt);
        self.suspension(lib, c);
        self.hydraulics(lib, c, dt);
        // (the base update calls vt+132 after the gear shift F-33101, before the horn F-21226 and the nitro vt+68)
        self.slipstream(lib, mats, c);
        self.horn(lib, c);
        self.nitrous(lib, mats, c, cam_mode);
        self.damage(lib, c, cam_mode);
        self.prev_shift = c.shift;
    }

    // ---- 1. tyres (F-21922)
    fn tyres(&mut self, lib: &mut SoundLib, mats: &Materials, c: &CarIn, dt: f32) {
        let mut max_slide = 0.0f32;
        let mut ground = 0;
        let mut car_level = None;
        let mut chirped = false;
        for i in 0..c.n_wheels {
            let w = c.wheels[i];
            if !w.contact {
                stop(lib, &mut self.w[i].roll);
                stop(lib, &mut self.w[i].skid);
                self.w[i].mat = 0;
                continue;
            }
            let (m, thr) = mats.get(w.surface);
            let prev = self.w[i].prev;
            self.wheel(lib, i, w.speed_mph, w.slide, m, w.pos, c.burnout && w.driven, c.wet, dt);
            max_slide = max_slide.max(w.slide);
            let o = lib.game.get(m).map(|o| o.to_vec()).unwrap_or_default();
            if w.slide > thr && (w.slide - prev) / dt.max(1e-6) > 10.0 && self.clock >= self.chirp_at + 1.0 && self.clock <= self.upshift_at + 0.5 && c.load >= 0.75 {
                let h = be_u32(&o, if self.player { 182 } else { 186 });
                if !is_null(h) {
                    lib.play_hash(h, self.req(w.pos));
                }
                chirped = true;
            }
            if !is_null(be_u32(&o, 18)) {
                car_level = Some(i);
            }
            ground += 1;
        }
        if chirped {
            self.chirp_at = self.clock;
        }
        match car_level {
            Some(i) => {
                let w = c.wheels[i];
                let (m, _) = mats.get(w.surface);
                self.wheel(lib, 4, w.speed_mph, max_slide, m, c.pos, false, c.wet, dt);
            }
            None => {
                stop(lib, &mut self.w[4].roll);
                stop(lib, &mut self.w[4].skid);
                self.w[4].mat = 0;
            }
        }
        if self.two_end && ground > 2 {
            stop(lib, &mut self.two_loop);
            let h = be_u32(&self.susp, 129);
            if !is_null(h) {
                lib.play_hash(h, self.req(c.pos));
            }
            self.two_end = false;
        }
    }

    /// update_wheel (F-84461); record 4 is the car-level one; `wet` picks the wet fields.
    #[allow(clippy::too_many_arguments)]
    fn wheel(&mut self, lib: &mut SoundLib, i: usize, v: f32, slide: f32, m: u32, pos: Vec3, burn: bool, wet: bool, dt: f32) {
        let Some(o) = lib.game.get(m).map(|o| o.to_vec()) else { return };
        let player = self.player;
        let car_level = i == 4;
        let f = |at: usize| be_f32(&o, at);
        let curve = be_u32(&o, 74);
        // rolling loop
        if v < f(78) {
            stop(lib, &mut self.w[i].roll);
        } else {
            let vol = map(lib, curve, f(86), f(90), f(78), f(82), v).max(f(86)).min(f(90));
            let ratio = map(lib, curve, f(106), f(110), f(98), f(102), v).max(f(106)).min(f(110));
            let req = Req { volume: lin2db(vol), pitch: cents(ratio), ..self.req(pos) };
            let old = self.w[i].mat;
            if m != old {
                stop(lib, &mut self.w[i].roll);
                let h = be_u32(&o, if player { 26 } else { 62 });
                if !is_null(h) {
                    lib.play_hash(h, req);
                }
                if let Some(oo) = lib.game.get(old).map(|o| o.to_vec()) {
                    let g = |at: usize| be_f32(&oo, at);
                    let h = be_u32(&oo, if player { 26 } else { 62 });
                    if !is_null(h) {
                        let vol = map(lib, curve, g(86), g(90), f(78), f(82), v).max(g(86)).min(g(90));
                        let ratio = map(lib, curve, g(106), g(110), f(98), f(102), v).max(g(106)).min(g(110));
                        lib.play_hash(h, Req { volume: lin2db(vol), pitch: cents(ratio), ..self.req(pos) });
                    }
                }
            }
            let w = if wet { 4 } else { 0 };
            let hash = match (car_level, player) {
                (true, true) => be_u32(&o, 18 + w),
                (true, false) => be_u32(&o, 46 + w),
                (false, true) => be_u32(&o, 10 + w),
                (false, false) => be_u32(&o, 38 + w),
            };
            // (the wheels' loops start at 0 / 25 / 50 / 75 % of the loop)
            let req = if car_level { req } else { Req { start_pct: 25.0 * i as f32, ..req } };
            keep(lib, &mut self.w[i].roll, hash, req);
        }
        // skid
        let slide = if burn { 1.0 } else { slide };
        let (thr, rate) = (f(198), f(202));
        let r = &mut self.w[i];
        if r.skid.is_none() && slide <= thr {
            r.s = thr;
        } else {
            approach_fma(&mut r.s, slide, rate, dt);
        }
        if r.s > thr {
            r.t = (r.s - thr) / (1.0 - thr).max(1e-6);
            r.gain = lib.curve_eval(be_u32(&o, 214), r.t).unwrap_or(r.t);
            r.ratio = f(190) + r.t * (f(194) - f(190));
            if m != r.mat {
                stop(lib, &mut r.skid);
            }
            let w = if wet { 4 } else { 0 };
            let hash = match (car_level, player) {
                (true, true) => be_u32(&o, 158 + w),
                (true, false) => be_u32(&o, 174 + w),
                (false, true) => be_u32(&o, 150 + w),
                (false, false) => be_u32(&o, 166 + w),
            };
            // skid_voice (F-75975): GLOBAL_SKIDS' offsets on top; the LPF (voice +88) is not applied by the mixer
            let req = Req { volume: lin2db(r.gain) + self.skid_db, pitch: cents(r.ratio) + self.skid_cents, start_pct: if car_level { 0.0 } else { 25.0 * i as f32 }, player, ..Req::at(pos) };
            keep(lib, &mut r.skid, hash, req);
        } else {
            stop(lib, &mut r.skid);
        }
        r.mat = m;
        r.prev = slide;
    }

    // ---- 2.2 suspension (F-61593)
    fn suspension(&mut self, lib: &mut SoundLib, c: &CarIn) {
        if self.susp.is_empty() {
            return;
        }
        let s = self.susp.clone();
        let k = 1.0 + c.damage * (be_u16(&s, 35) * 0.01 - 1.0);
        for i in 0..c.n_wheels {
            let w = c.wheels[i];
            if w.limit > 0.0 && w.comp >= w.limit && free(lib, &self.bottom[i]) {
                // (+37 below the load ratio +45, else +41: the same sound on the disc)
                self.bottom[i] = lib.play_hash(be_u32(&s, 41), self.req(w.pos));
            }
            let v = (w.comp_rate / w.travel.max(1e-3)).abs() * k;
            if v < be_f32(&s, 23) {
                stop(lib, &mut self.damper[i]);
                continue;
            }
            let (bit, at) = if v < be_f32(&s, 27) {
                (1, 11)
            } else if v < be_f32(&s, 31) {
                (2, 15)
            } else {
                (4, 19)
            };
            if s[10] & bit != 0 && !c.hyd_on {
                continue;
            }
            if free(lib, &self.damper[i]) {
                self.damper[i] = lib.play_hash(be_u32(&s, at), self.req(w.pos));
            }
        }
    }

    // ---- 2.3 / 2.4 hydraulics, rocking, wipeout loops (F-81129), two wheels
    fn hydraulics(&mut self, lib: &mut SoundLib, c: &CarIn, dt: f32) {
        if self.susp.is_empty() {
            return;
        }
        let s = self.susp.clone();
        let mut any_up = false;
        if let Some(order) = c.hyd {
            for i in 0..c.n_wheels {
                let (o, prev) = (order[i], self.hyd_prev[i]);
                if o != prev {
                    let at = match o {
                        1 => Some(53),
                        -1 => Some(49),
                        _ if prev != -1 => Some(49),
                        _ => None,
                    };
                    if let Some(at) = at {
                        lib.play_hash(be_u32(&s, at), self.req(c.wheels[i].pos));
                    }
                }
                any_up |= o == 1;
            }
            self.hyd_prev = order;
        }
        self.hyd_timer += dt;
        let w = c.ang_vel;
        let wxz = w.x * w.x + w.z * w.z;
        let sq = |at: usize| be_f32(&s, at) * be_f32(&s, at);
        // (sound offset, state, gate time offset) or None = stop; Some(None) = leave as is
        let want: Option<Option<(usize, u8, Option<usize>)>> = if c.wipeout {
            if c.air_time > 0.25 {
                Some((w.length_squared() >= sq(121)).then_some((117, 4, None)))
            } else if wxz >= sq(113) {
                Some(Some((109, 2, None)))
            } else if wxz >= sq(105) {
                Some(Some((97, 1, Some(101))))
            } else {
                Some(None)
            }
        } else if any_up && c.air_time <= 0.25 {
            if wxz < sq(69) {
                None
            } else {
                Some(Some((61, 3, Some(65))))
            }
        } else {
            Some(None)
        };
        match want {
            None => {}
            Some(None) => {
                if let Some((h, _)) = self.hyd_loop.take() {
                    lib.stop(h);
                }
            }
            Some(Some((at, state, gate))) => {
                if let Some((h, st)) = self.hyd_loop {
                    if st != state {
                        lib.stop(h);
                        self.hyd_loop = None;
                    }
                }
                let running = self.hyd_loop.is_some_and(|(h, _)| lib.playing(h));
                if running {
                    if let Some((h, _)) = self.hyd_loop {
                        lib.set(h, self.req(c.pos));
                    }
                } else if gate.map_or(true, |g| self.hyd_timer >= be_f32(&s, g)) {
                    self.hyd_timer = 0.0;
                    self.hyd_loop = lib.play_hash(be_u32(&s, at), self.req(c.pos)).map(|h| (h, state));
                }
            }
        }
        // two wheels (F-90315 / F-12477): the loop from the lean's start, the end once more than two wheels
        // touch again (tyres)
        if c.bike {
            // a motorcycle (@A-69806..FD44): the gyro's stunt state from 0 starts the loop, back to 0 stops it and plays
            // the DOWN sound; 1 <-> 99 and -1 <-> -99 change nothing
            let on = c.gyro != 0;
            if on && !self.prev_two {
                stop(lib, &mut self.two_loop);
                self.two_loop = lib.play_hash(be_u32(&s, 125), self.req(c.pos));
            } else if !on && self.prev_two {
                stop(lib, &mut self.two_loop);
                let h = be_u32(&s, 129);
                if !is_null(h) {
                    lib.play_hash(h, self.req(c.pos));
                }
            }
            if let Some(h) = self.two_loop {
                lib.set(h, self.req(c.pos));
            }
            self.prev_two = on;
            return;
        }
        if c.two_wheels && !self.prev_two {
            stop(lib, &mut self.two_loop);
            self.two_loop = lib.play_hash(be_u32(&s, 125), self.req(c.pos));
            self.two_end = false;
        } else if !c.two_wheels && self.prev_two {
            self.two_end = true;
        }
        if let Some(h) = self.two_loop {
            lib.set(h, self.req(c.pos));
        }
        self.prev_two = c.two_wheels;
    }

    // ---- 4. horn: ONESHOT_PARAMS +64 while the button is held
    fn horn(&mut self, lib: &mut SoundLib, c: &CarIn) {
        if c.horn {
            let req = self.req(c.pos);
            keep(lib, &mut self.horn, self.horn_hash, req);
        } else {
            stop(lib, &mut self.horn);
        }
    }

    // ---- 5. siren
    /// F-10917: the alert, only when nothing runs in the siren's slot.
    pub fn siren_alert(&mut self, lib: &mut SoundLib, pos: Vec3) {
        if free(lib, &self.siren) {
            self.siren = lib.play_hash(be_u32(&self.police, 10), self.req(pos));
        }
    }

    /// The AI's start siren (F-60528): at once, or after `delay` s (the countdown +1140; F-39258 sets the
    /// siren flag even while the slot plays something else, the loop follows when it ends).
    pub fn siren_start(&mut self, delay: f32) {
        if delay <= 0.0 {
            self.siren_delay = None;
            if !self.siren_on {
                self.siren_on = true;
                self.siren_pitch = 0;
            }
        } else if !self.siren_on && self.siren_delay.is_none() {
            self.siren_delay = Some(delay);
        }
    }

    /// F-50805: the player passing this car (traffic/RULE.md 5) - a one-shot at most every 5 s.
    pub fn whoosh(&mut self, lib: &mut SoundLib, pos: Vec3, vol: f32, pitch: i32) {
        if self.clock - self.whoosh_at < 5.0 || is_null(self.whoosh_hash) {
            return;
        }
        self.whoosh_at = self.clock;
        lib.play_hash(self.whoosh_hash, Req::at(pos).vol(vol).pitch(pitch));
    }

    /// warn(cop, on) (F-12823): on - the INTERSECTION loop (+14) if the siren slot is free; off - the siren starts
    /// at once in a pursuit (the loop that held the slot stops first: whether it ends by itself is open, siren/RULE.md 8.4).
    pub fn siren_warn(&mut self, lib: &mut SoundLib, pos: Vec3, on: bool, pursuit: bool) {
        if on {
            if self.warn == 0.0 {
                if free(lib, &self.siren) {
                    self.siren = lib.play_hash(be_u32(&self.police, 14), self.req(pos));
                }
                self.warn = 1e-3;
            }
        } else if self.warn > 0.0 {
            self.warn = 0.0;
            if !self.siren_on {
                stop(lib, &mut self.siren);
            }
            if pursuit {
                self.siren_start(0.0);
            } else {
                self.siren_stop(lib);
            }
        }
    }

    /// F-59693: the slot stops, the siren flag clears.
    pub fn siren_stop(&mut self, lib: &mut SoundLib) {
        self.siren_delay = None;
        self.warn = 0.0;
        self.siren_on = false;
        stop(lib, &mut self.siren);
    }

    /// Per frame (F-79540 countdown, F-92699): the delayed start, then the loop kept running, wound down
    /// (600 cents/s to -2400, then -100 dB) while the car is wrecked, +10 dB on the player's car.
    pub fn siren_frame(&mut self, lib: &mut SoundLib, pos: Vec3, wrecked: bool, dt: f32) {
        // (a running warn pauses the delayed start)
        if self.warn > 0.0 {
            self.warn += dt;
        } else if let Some(d) = &mut self.siren_delay {
            *d -= dt;
            if *d <= 0.0 {
                self.siren_delay = None;
                self.siren_on = true;
                self.siren_pitch = 0;
            }
        }
        if !self.siren_on {
            if let Some(h) = self.siren {
                lib.set(h, self.req(pos));
            }
            return;
        }
        let base = if self.player { 10.0 } else { 0.0 };
        let mut vol = base;
        if wrecked {
            if self.siren_pitch > -2400 {
                self.siren_pitch -= (dt * 600.0) as i32;
            } else {
                vol = -100.0;
            }
        } else {
            self.siren_pitch = 0;
        }
        let req = Req { volume: vol, pitch: self.siren_pitch, ..self.req(pos) };
        if free(lib, &self.siren) {
            self.siren = lib.play_hash(be_u32(&self.police, 18), Req { volume: base, ..req });
        } else if let Some(h) = self.siren {
            lib.set(h, req);
        }
    }

    // ---- 8.2 / 8.3 turbo whine and blow-off
    fn turbo(&mut self, lib: &mut SoundLib, c: &CarIn, dt: f32) {
        let t = &self.turbo;
        if t.len() < 19 + 160 {
            return;
        }
        let level = c.parts[1].min(3) as usize;
        let r = 19 + 40 * level;
        let s = c.rpm_s;
        // blow-off and boost (F-32506)
        let thr = be_f32(t, r + 20);
        let fall = (c.throttle - self.prev_throttle) / dt.max(1e-6);
        let shift_event = c.shift != 0 && self.prev_shift == 0;
        if self.boost > thr && self.spool >= be_f32(t, r + 28) && (shift_event || fall <= be_f32(t, r + 32)) {
            let (a, b) = (be_u16(t, r + 36), be_u16(t, r + 38));
            let start_ms = ((1.0 - (self.boost - thr) / (1.0 - thr).max(1e-6)) * (b - a) + a).trunc();
            let h = be_u32(t, r + 12);
            if !is_null(h) {
                lib.play_hash(h, Req { start_ms, ..self.req(c.pos) });
            }
            self.boost = 0.0;
            self.spool = 0.0;
        }
        let rate = be_f32(t, r + 24) * dt;
        if s >= be_f32(t, r + 16) && c.throttle >= thr && c.shift == 0 {
            approach(&mut self.boost, c.pedal, rate);
            self.spool += dt;
        } else {
            approach(&mut self.boost, 0.0, rate);
        }
        self.prev_throttle = c.throttle;
        // whine (F-19395)
        let (hash, start) = if level != 0 {
            (be_u32(t, r), be_f32(t, r + 4))
        } else if c.parts[0] != 0 {
            (be_u32(t, 14), 3000.0)
        } else {
            stop(lib, &mut self.turbo_v);
            return;
        };
        if s < start || start <= 0.0 {
            stop(lib, &mut self.turbo_v);
            return;
        }
        let x = ((s - start) / (c.max_rpm - start).max(1.0)).clamp(0.0, 1.0);
        let g = lib.curve_eval(be_u32(t, 27), x).unwrap_or(x) * c.load_vol;
        let req = Req { volume: lin2db(g), pitch: cents(s / start), ..self.req(c.pos) };
        keep(lib, &mut self.turbo_v, hash, req);
    }

    /// The launch chirp (F-40027, the nitrous fire's first call and the slipstream burn's start): when the
    /// clock (this+156) is at least 0.5 s (0x82003A24) past the last chirp (this+264), TIRE_CHIRP_MASTER (the
    /// surface's MATERIALS_* +182) at every driven wheel (entity+1516+44i) with contact (+529) and a material (+464);
    /// this+264 = the clock once a wheel chirped.
    fn launch_chirp(&mut self, lib: &mut SoundLib, mats: &Materials, c: &CarIn) {
        if self.clock < self.chirp_at + 0.5 {
            return;
        }
        for (k, w) in c.wheels.iter().enumerate().take(c.n_wheels).filter(|(_, w)| w.driven && w.contact) {
            let (m, _) = mats.get(w.surface);
            let h = lib.game.get(m).map(|o| be_u32(o, 182)).unwrap_or(0);
            if !is_null(h) {
                lib.play_hash(h, self.req(w.pos));
            }
            let at = SST_REPLAY_AT.load(std::sync::atomic::Ordering::Relaxed);
            sst_log(format_args!("CHIRP f={} wheel {k} hash={h:08x} at {:.2},{:.2},{:.2}", if at >= 0 { at as u64 } else { self.sst_f }, w.pos.x, w.pos.y, w.pos.z));
            self.chirp_at = self.clock;
        }
    }

    // ---- the slipstream turbo (SST_AUDIO_RULE.md, recorded against the original: work/slipstream/audiorec):
    // GLOBAL_SPECIAL_ABILITIES +99 SST_LOOP_MASTER, +103 f32 charge start (0.05), +107 s16 mB there (-500), +109 f32 pitch
    // ratio there (0.4), +113 f32 full (1.0), +117 s16 mB at full (0), +119 f32 pitch ratio at full (1.5), +123
    // SST_BEEP_MASTER, +127 SLIPSTREAM_NITROUS_MASTER (player's burn), +131 SLIPSTREAM_NITROUS_MASTER_O (an opponent's burn)
    fn slipstream(&mut self, lib: &mut SoundLib, mats: &Materials, c: &CarIn) {
        self.sst_speech = false;
        self.sst_f += 1;
        // (log lines count this entity's updates; under MCLA_SST_REPLAY they carry the recording's update instead)
        let k = SST_REPLAY_AT.load(std::sync::atomic::Ordering::Relaxed);
        let f = if k >= 0 { k as u64 } else { self.sst_f };
        let p = &self.sst_obj;
        if p.len() < 135 {
            return;
        }
        let (level, burning) = c.sst.unwrap_or((0.0, false));
        // every start: 0 dB at the entity (P+20 = this+80), flags |0x20
        let at = Req { player: self.player, ..Req::at(c.pos) };
        let live = |lib: &SoundLib, h: &Option<Handle>| h.is_some_and(|h| lib.playing(h));
        // a slot's stop (F-31667) is only made when the slot holds a handle
        let halt = |lib: &mut SoundLib, slot: &mut Option<Handle>, what: &str| {
            if live(lib, slot) {
                sst_log(format_args!("STOP {what} f={f}"));
            }
            stop(lib, slot);
        };
        if !self.player {
            // mcOpponentAudioEntity vt+132 (F-47862): only the burn
            if c.sst.is_some_and(|(l, b)| b || l > 0.0) || live(lib, &self.sst_burn) {
                sst_log(format_args!("O f={f} L={level:.5} burn={} h={}", burning as u8, live(lib, &self.sst_burn) as u8));
            }
            if c.sst.is_some() && burning {
                let h = be_u32(p, 131);
                if free(lib, &self.sst_burn) {
                    sst_log(format_args!("START oppburn f={f} vol 0"));
                }
                keep(lib, &mut self.sst_burn, h, at);
            } else {
                halt(lib, &mut self.sst_burn, "oppburn");
            }
            return;
        }
        // mcPlayerAudioEntity vt+132 (F-58518)
        if c.sst.is_some_and(|(l, b)| b || l > 0.0) || live(lib, &self.sst_burn) || live(lib, &self.sst_loop) || live(lib, &self.sst_full) {
            sst_log(format_args!(
                "P f={f} clk={:.4} chirp={:.4} L={level:.5} burn={} burnh={} loop={} beep={}",
                self.clock,
                self.chirp_at,
                burning as u8,
                live(lib, &self.sst_burn) as u8,
                live(lib, &self.sst_loop) as u8,
                live(lib, &self.sst_full) as u8
            ));
        }
        if burning {
            // (the request's position is the entity's own (P+20 = this+80), so a playing burn follows the car)
            let first = free(lib, &self.sst_burn);
            if first {
                sst_log(format_args!("START burn f={f} vol 0"));
            }
            let h = be_u32(&self.sst_obj, 127);
            keep(lib, &mut self.sst_burn, h, at);
            if first {
                self.launch_chirp(lib, mats, c);
            }
            // mix slot 18 (mgr+1740 bit 0x40000) not on yet: the speech request PLAYER_NITROUS (if no speech line
            // plays - the caller's test), then the slot on at strength 1.0
            if !lib.scene_on("DYNAMIC_MIXER_SST") {
                self.sst_speech = true;
                sst_log(format_args!("SPEECHREQ f={f}"));
                lib.scene("DYNAMIC_MIXER_SST", true);
                sst_log(format_args!("MIXON f={f}"));
            }
        } else {
            halt(lib, &mut self.sst_burn, "burn");
            if lib.scene_on("DYNAMIC_MIXER_SST") {
                lib.scene("DYNAMIC_MIXER_SST", false);
                sst_log(format_args!("MIXOFF f={f}"));
            }
        }
        let p = &self.sst_obj;
        let (lmin, lfull) = (be_f32(p, 103), be_f32(p, 113));
        let (vol, pitch) = 'charge: {
            if burning || level < lmin {
                halt(lib, &mut self.sst_loop, "loop");
                halt(lib, &mut self.sst_full, "beep");
                break 'charge (0.0, 1.0);
            }
            let (vol, pitch);
            if level >= lfull {
                let beep = be_u32(p, 123);
                if level >= 1.0 {
                    if free(lib, &self.sst_full) {
                        sst_log(format_args!("START beep f={f} vol 0"));
                    }
                    keep(lib, &mut self.sst_full, beep, at);
                } else {
                    halt(lib, &mut self.sst_full, "beep");
                }
                vol = be_s16(p, 117) * 0.01;
                pitch = be_f32(p, 119);
            } else {
                halt(lib, &mut self.sst_full, "beep");
                let u = (level - lmin) / (lfull - lmin);
                let (v0, v1) = (be_s16(p, 107) * 0.01, be_s16(p, 117) * 0.01);
                vol = (v1 - v0) * u + v0;
                let (p0, p1) = (be_f32(p, 109), be_f32(p, 119));
                pitch = (p1 - p0) * u + p0;
            }
            if level >= 1.0 {
                halt(lib, &mut self.sst_loop, "loop");
            } else {
                // the loop's volume (dB, voice +64: F-46088) and pitch (cents, voice +80: F-98474) set every
                // frame; a new loop is started with the request's volume = this frame's
                let h = be_u32(p, 99);
                if free(lib, &self.sst_loop) {
                    sst_log(format_args!("START loop f={f} vol {vol:.4}"));
                }
                let cents = voice_cents(pitch);
                keep(lib, &mut self.sst_loop, h, Req { volume: vol, pitch: cents, ..at });
                self.sst_vp = (vol, cents);
                sst_log(format_args!("PITCH f={f} ratio {pitch:.9}"));
                sst_log(format_args!("VOL f={f} dB {vol:.6} (block pitch now {cents})"));
            }
            (vol, pitch)
        };
        if trace() {
            let k = live(lib, &self.sst_burn) as u8 | (live(lib, &self.sst_loop) as u8) << 1 | (live(lib, &self.sst_full) as u8) << 2;
            if k != self.sst_seen.0 || (k & 2 != 0 && self.clock >= self.sst_seen.1 + 0.5) {
                eprintln!("sst audio: level {level:.3} burn {} loop {} ({vol:.2} dB, {} cents) full {}", k & 1, k >> 1 & 1, voice_cents(pitch), k >> 2 & 1);
                self.sst_seen = (k, self.clock);
            }
        }
    }

    // ---- 8.4 nitrous: FIRE / FIRE_FRONT_END loops by the camera, the launch chirp, EMPTY
    fn nitrous(&mut self, lib: &mut SoundLib, mats: &Materials, c: &CarIn, cam_mode: i32) {
        let n = &self.nitro_obj;
        let base = 11 + 24 * c.parts[2].min(3) as usize;
        if n.len() < base + 24 {
            return;
        }
        let front = self.player && (cam_mode == 0 || cam_mode == 1);
        let (world_db, fe_db) = if front { (-100.0, 0.0) } else { (0.0, -100.0) };
        if c.nitro_firing {
            let first = self.nitro.is_none();
            let (fire, fire_fe) = (be_u32(n, base), be_u32(n, base + 4));
            let slots = self.nitro.get_or_insert((None, None));
            keep(lib, &mut slots.0, fire, Req { volume: world_db, ..Req { player: self.player, ..Req::at(c.pos) } });
            if self.player {
                keep(lib, &mut slots.1, fire_fe, Req { volume: fe_db, ..Req { player: true, ..Req::at(c.pos) } });
            }
            if first && self.player {
                self.launch_chirp(lib, mats, c);
            }
        } else if let Some((a, b)) = self.nitro.take() {
            for mut s in [a, b] {
                stop(lib, &mut s);
            }
        }
        if c.nitro_empty && self.player && free(lib, &self.nitro_empty) {
            let h = be_u32(&self.nitro_obj, base + if front { 12 } else { 8 });
            if !is_null(h) {
                self.nitro_empty = lib.play_hash(h, self.req(c.pos));
            }
        }
    }

    // ---- the moving damage parts (DAMAGE_PARTS_RULE.md): TUNERS_DAMAGE +15 hood / +43 trunk / +71 rear / +99 front
    // bumper records {move threshold, move loop, open-stop threshold, open-stop, shut threshold, shut, break-off}
    /// A window's hit sum passing 1 or 2 (F-66214 -> `F-45199` on the car's sound entity): the
    /// entity's GLOBAL_CAR_PARTICLES (+1256) +18 - CAR_BANGERS_GLASS_LARGE - once at the window's place,
    /// 0 dB, LPF field 23900, flags 4; heard whether or not the car is drawn.
    pub fn glass(&mut self, lib: &mut SoundLib, dmg: &crate::damagemap::CarDamage, pos: Vec3, rot: glam::Quat) {
        // (events from before this car's sounds were made are not heard)
        let seen = *self.glass_seen.get_or_insert(dmg.glass_serial);
        for &(n, local) in dmg.glass.iter().filter(|(n, _)| *n > seen) {
            self.glass_seen = Some(n);
            if !is_null(self.glass_hash) {
                let h = lib.play_hash(self.glass_hash, self.req(pos + rot * local));
                if std::env::var_os("MCLA_PARTS_LOG").is_some() {
                    log::info!("glass: event {n} sound {:08x} started {}", self.glass_hash, h.is_some());
                }
            }
        }
    }

    pub fn damage_parts(&mut self, lib: &mut SoundLib, parts: &crate::damageparts::DamageParts) {
        use crate::damageparts::{DamageParts, PartEvent};
        let d = self.damage_obj.clone();
        if d.len() < 183 {
            return;
        }
        let rec = |r: usize| 15 + 28 * r;
        let fdt = DamageParts::FRAME;
        let (clock, player) = (self.clock, self.player);
        // the flaps' move loops (F-32243): while loose, |drawn - last| / dt >= rec+0 (0.6 rad/s), at the particle
        for (i, f) in [&parts.hood, &parts.trunk].into_iter().enumerate() {
            let r = rec(i);
            let rate = (f.drawn() - f.drawn_prev()).abs() / fdt;
            if f.loose && rate >= be_f32(&d, r) {
                let req = self.req(f.p);
                keep(lib, &mut self.flap_move[i], be_u32(&d, r + 4), req);
            } else {
                stop(lib, &mut self.flap_move[i]);
            }
        }
        // the bumpers' creak loops (F-27481): the fastest loose end relative to the car >= rec+0 (0.4 m/s)
        for (i, b) in parts.bumpers.iter().enumerate() {
            let r = rec(2 + i);
            match b.as_ref().and_then(|b| b.creak(fdt)) {
                Some((x, pos)) if x >= be_f32(&d, r) => {
                    let req = self.req(pos);
                    keep(lib, &mut self.creak[i], be_u32(&d, r + 4), req);
                }
                _ => stop(lib, &mut self.creak[i]),
            }
        }
        // the one-shots of this frame's events
        let (imp_mph, imp_top, imp_lo, imp_hi) = (be_f32(&d, 131), be_f32(&d, 139), be_f32(&d, 135), be_f32(&d, 143));
        let (scr_lo_mph, scr_hi_mph, scr_lo, scr_hi) = (be_f32(&d, 151), be_f32(&d, 155), be_f32(&d, 159), be_f32(&d, 163));
        let (pit_lo_mph, pit_hi_mph, pit_lo, pit_hi) = (be_f32(&d, 167), be_f32(&d, 171), be_f32(&d, 175), be_f32(&d, 179));
        let lerp = |x: f32, x0: f32, x1: f32, y0: f32, y1: f32| y0 + (y1 - y0) * (x.clamp(x0, x1) - x0) / (x1 - x0).max(1e-6);
        // (events from before this car's sounds were made are not heard)
        let seen = *self.parts_seen.get_or_insert(parts.serial);
        for &(serial, e) in parts.events.iter().filter(|(n, _)| *n > seen) {
            self.parts_seen = Some(serial);
            match e {
                // (F-28953 / F-47714: shut at rate >= rec+16 plays rec+20, the open stop at >= rec+8 rec+12)
                PartEvent::FlapStop { flap, open, rate, pos } => {
                    let r = rec(flap as usize);
                    let (thr, h) = if open { (be_f32(&d, r + 8), be_u32(&d, r + 12)) } else { (be_f32(&d, r + 16), be_u32(&d, r + 20)) };
                    if rate >= thr && !is_null(h) {
                        lib.play_hash(h, self.req(pos));
                    }
                }
                // (F-82611: rec+24 BUMPER_BREAK_OFF at the piece, 0 dB)
                PartEvent::BumperOff { bumper, pos } => {
                    let h = be_u32(&d, rec(2 + bumper as usize) + 24);
                    if !is_null(h) {
                        lib.play_hash(h, self.req(pos));
                    }
                }
                // (F-80340: the largest approach of the new contacts, from +131 (3 mph): +127 at
                // dB(1.5 .. 2.0 over 3 .. 70 mph))
                PartEvent::BumperImpact { speed, pos, .. } => {
                    let mph = speed * MPH;
                    let h = be_u32(&d, 127);
                    if mph >= imp_mph && !is_null(h) {
                        let vol = lin2db(lerp(mph.min(imp_top), imp_mph, imp_top, imp_lo, imp_hi));
                        lib.play_hash(h, self.req(pos).vol(vol));
                    }
                }
                // (a slide from +151 (5 mph) keeps the scrape loop +147 going; its volume dB(0.25 .. 0.5 over 5 .. 60 mph)
                // and pitch 0.7 .. 1.5 over 5 .. 100 mph are set when it starts and never again)
                PartEvent::BumperScrape { bumper, speed, pos } => {
                    let mph = speed * MPH;
                    let h = be_u32(&d, 147);
                    let slot = &mut self.scrape[bumper as usize & 1];
                    if mph >= scr_lo_mph && !is_null(h) {
                        slot.1 = clock;
                        if slot.0.map_or(true, |s| !lib.playing(s)) {
                            let vol = lin2db(lerp(mph.min(scr_hi_mph), scr_lo_mph, scr_hi_mph, scr_lo, scr_hi));
                            let pitch = cents(lerp(mph, pit_lo_mph, pit_hi_mph, pit_lo, pit_hi));
                            slot.0 = lib.play_hash(h, Req { player, ..Req::at(pos) }.vol(vol).pitch(pitch));
                        }
                    }
                }
            }
        }
        // (F-44446 @A-78327: the scrape stops 1/15 s after the last slide)
        for (h, last) in self.scrape.iter_mut() {
            if h.is_some() && clock >= *last + 1.0 / 15.0 {
                stop(lib, h);
            }
        }
    }

    // ---- 8.6 the engine pings of a damaged car, in camera mode 0
    fn damage(&mut self, lib: &mut SoundLib, c: &CarIn, cam_mode: i32) {
        let d = &self.damage_obj;
        if d.len() > 14 && cam_mode == 0 && c.damage >= d[10] as f32 * 0.01 {
            let h = be_u32(d, 11);
            let req = self.req(c.pos);
            keep(lib, &mut self.pings, h, req);
        } else {
            stop(lib, &mut self.pings);
        }
    }
}

/// MCLA_SST_REPLAY: the recorder's "P" lines (work/slipstream/audiorec) as (level, burning) per original update, from the
/// first update with a charge or a burn; updates the recorder skipped (nothing charged, nothing playing) are (0, false).
fn sst_replay_load(text: &str) -> Vec<(f32, bool)> {
    let field = |l: &str, k: &str| l.split(' ').find_map(|w| w.strip_prefix(k)).map(str::to_string);
    let mut rows: Vec<(usize, f32, bool)> = Vec::new();
    for l in text.lines().filter(|l| l.starts_with("P f=")) {
        let (Some(f), Some(lv), Some(b)) = (field(l, "f="), field(l, "L="), field(l, "burn=")) else { continue };
        let (Ok(f), Ok(lv)) = (f.parse::<usize>(), lv.parse::<f32>()) else { continue };
        rows.push((f, lv, b == "1"));
    }
    let Some(first) = rows.iter().find(|r| r.1 > 0.0 || r.2).map(|r| r.0) else { return Vec::new() };
    let last = rows.iter().map(|r| r.0).max().unwrap_or(first);
    let mut v = vec![(0.0, false); last + 1 - first];
    for (f, lv, b) in rows.into_iter().filter(|r| r.0 >= first) {
        v[f - first] = (lv, b);
    }
    v
}

/// A speech scenario's Probability (percent, clamped 0..100, 100 when missing) and Frequency (s, 10 when missing) as
/// the speech script loader F-66242 reads them (scenario +64, +20).
fn scenario_gate(xml: &str, name: &str) -> (i32, f32) {
    let tag = xml.split("<Scenario ").find(|s| s.starts_with(&format!("name=\"{name}\""))).and_then(|s| s.split('>').next()).unwrap_or("");
    let attr = |a: &str| tag.split(&format!(" {a}=\"")).nth(1).and_then(|r| r.split('"').next()).and_then(|v| v.trim().parse::<f32>().ok());
    (attr("Probability").map_or(100, |p| (p as i32).clamp(0, 100)), attr("Frequency").unwrap_or(10.0))
}

/// Record +40 (F-43725 @A-65200) of the port's wheel `i` (FL, FR, RL, RR): chassis +372 layout 1 or 3 -> +412 /
/// +420 (the front pair) are the driven ones, any other layout -> +416 / +424 (the rear pair). So an all-wheel-drive car
/// with layout 2 (the R8) chirps from its rear wheels only (recorded: records 2 and 3, work/slipstream/audiorec).
fn driven_record(drive: u8, i: usize) -> bool {
    if drive == 1 || drive == 3 {
        i < 2
    } else {
        i >= 2
    }
}

/// What a car's sounds read from the port's car (vehicle sim when it has one).
pub fn car_in(c: &CarInstance) -> CarIn {
    let st = &c.state;
    let vs = st.veh.as_deref();
    let tune = c.model.params.veh.as_deref();
    let drive = vs.map_or(0, |v| v.drive);
    // the original's record order: car +28 holds FL (+412), FR (+420), RL (+416), RR (+424) - read from the running original
    // (work/slipstream/audiorec: the R8's car+28 = +412, +420, +416, +424, wheels 2 and 3 on the rear axle), the port's
    // own order; a bike's records are its front and rear wheel (the port's 0 and 2)
    let order: &[usize] = if c.model.bike { &[0, 2] } else { &[0, 1, 2, 3] };
    let mut wheels = [WheelIn::default(); 4];
    for (k, w) in wheels.iter_mut().enumerate().take(order.len()) {
        let i = order[k];
        let ws = &st.wheels[i];
        let sim = vs.map(|v| &v.wheels[i]);
        let wt = tune.map(|t| if i < 2 { &t.t.wheel_front } else { &t.t.wheel_back });
        *w = WheelIn {
            contact: ws.grounded,
            surface: ws.surface,
            speed_mph: sim.map_or(st.speed(), |s| s.velocity_long.abs()) * MPH,
            slide: sim.map_or((ws.slip * 0.5).min(1.0), |s| s.slide),
            pos: ws.world_pos,
            comp: sim.map_or(0.0, |s| s.comp),
            comp_rate: sim.map_or(0.0, |s| s.comp_rate),
            limit: wt.map_or(0.0, |t| t.suspension_limit),
            travel: wt.map_or(1.0, |t| t.suspension_extent + t.suspension_limit),
            driven: driven_record(drive, i),
        };
    }
    let p = &c.model.params;
    let [max, med, ..] = p.dmg;
    let points = st.damage * max.max(1.0);
    let damage = if max > med { ((points - med) / (max - med)).clamp(0.0, 1.0) } else { 0.0 };
    let gyro = vs.map_or(0, |v| if v.bg.state != 0 { v.bg.state } else { v.wheelie as i16 });
    CarIn {
        pos: st.pos,
        wheels,
        n_wheels: order.len(),
        bike: c.model.bike,
        gyro,
        burnout: st.burnout > 0.0,
        load: vs.map_or(0.0, |v| v.engine_torque / tune.map_or(1.0, |t| t.sync_torque.max(1e-3))),
        damage,
        hyd: vs.filter(|v| v.hyd.on || v.hyd.order != [0; 4]).map(|v| [v.hyd.order[0], v.hyd.order[2], v.hyd.order[1], v.hyd.order[3]]),
        hyd_on: vs.is_some_and(|v| v.hyd.on),
        ang_vel: st.ang_vel,
        air_time: st.airborne_time,
        wipeout: st.wipeout > 0.0,
        two_wheels: vs.is_some_and(|v| v.two != 0) && !c.model.bike,
        nitro_firing: st.nitro_on(),
        sst: vs.map(|v| (v.sst.level, v.sst.burning)),
        throttle: c.input.throttle,
        pedal: vs.map_or(c.input.throttle, |v| v.engine_pedal),
        shift: vs.map_or(0, |v| if v.gear_change.active { v.gear_change.direction } else { 0 }),
        max_rpm: tune.map_or(7000.0, |t| t.t.engine.s.max_rpm),
        ..Default::default()
    }
}

/// The engine's inputs for a computer racer's engine sound.
fn engine_in(c: &CarInstance, racing: bool) -> EngineIn {
    let st = &c.state;
    let vs = st.veh.as_deref();
    let e = c.model.params.veh.as_ref().map(|t| &t.t.engine.s);
    let speed = st.speed();
    EngineIn {
        pos: st.pos,
        rpm: vs.map_or(st.rpm, |v| v.engine_w * 9.549_296),
        idle_rpm: e.map_or(800.0, |e| e.idle_rpm),
        max_rpm: e.map_or(7000.0, |e| e.max_rpm),
        torque: vs.map_or(0.0, |v| v.engine_torque),
        full_torque: c.model.params.veh.as_ref().map_or(1.0, |t| t.sync_torque),
        pedal: vs.map_or(c.input.throttle, |v| v.engine_pedal),
        throttle_in: c.input.throttle,
        speed,
        gear: vs.map_or(st.gear, |v| v.gear),
        shift: vs.map_or(0, |v| if v.gear_change.active { v.gear_change.direction } else { 0 }),
        idle_only: false,
        racing,
        speed_ratio: speed / c.model.params.veh.as_ref().map_or(c.model.params.top_speed, |t| t.top_speed).max(1.0),
    }
}

/// An ambient model's mcAmbientType.xml values the sounds read (traffic/RULE.md 1).
#[derive(Clone, Debug)]
struct TrafficType {
    audio: String,
    bound_group: i32,
    max_decel: f32,
    tire_stretch: f32,
}

/// One ambient car's sounds (mcAmbientVehicleAudioEntity; `reference/audio/TRAFFIC_AUDIO_RULE.md`).
struct Traffic {
    obj: Vec<u8>,
    ty: TrafficType,
    engine: Option<Handle>,
    state: u8,
    in_state: f32,
    accel_clock: f32,
    tyre: Option<Handle>,
    wet_tyre: Option<Handle>,
    skid: Option<Handle>,
    horn: Option<Handle>,
    alarm: Option<Handle>,
    speech: Option<Handle>,
    tgt: f32,
    prev_v: Option<f32>,
    /// E+244: the entity's age (only advances in its own update: a parked car's once something has hit it)
    age: f32,
    awake: bool,
    /// E+256 blocked time, E+260 next horn, E+264 next whoosh, E+268 next contact reaction, E+272 next speech
    wait: f32,
    next_horn: f32,
    next_whoosh: f32,
    next_hit: f32,
    next_speech: f32,
    /// E+224: the driver's voice (AMBIENT_VEHICLE_DRIVER_MGR entry's +10 sound)
    voice: u32,
    /// car+2336: the last 4 frames' commanded acceleration (m/s^2)
    cmd: std::collections::VecDeque<f32>,
}

/// What the Fleet works out for one ambient car's horn (traffic/RULE.md 3.1).
#[derive(Clone, Copy, Default)]
struct HornIn {
    blocked: bool,
    /// the obstacle has stood without input for 30 s or more
    idle: bool,
}

impl Traffic {
    fn stop_all(&mut self, lib: &mut SoundLib) {
        for s in [&mut self.engine, &mut self.tyre, &mut self.wet_tyre, &mut self.skid, &mut self.horn, &mut self.alarm, &mut self.speech] {
            stop(lib, s);
        }
    }

    /// The update F-70175 of a car with a driver: engine (7.3), tyres (7.4), skid (RULE 2), blocked horn (3.1).
    fn update(&mut self, lib: &mut SoundLib, c: &CarInstance, selected: bool, wet: bool, horn: HornIn, dt: f32) {
        let a = self.obj.clone();
        let f = |at: usize| be_f32(&a, at);
        let st = &c.state;
        let pos = st.pos;
        let v = st.forward().dot(st.vel) * MPH;
        // E+236 = the AI's wanted speed of this frame, unsmoothed
        self.tgt = c.ai.as_ref().map_or(0.0, |ai| ai.target_speed) * MPH;
        // E+240: mph gained over one frame (the original's 30 Hz frame)
        let acc = self.prev_v.map_or(0.0, |p| (v - p) / dt.max(1e-6) / 30.0);
        self.prev_v = Some(v);
        self.age += dt;
        self.awake = true;
        // the commanded acceleration's 4-frame window (the port's brain commands pedals: full brake = fMaxDecel)
        let cmd = if c.input.brake > 0.0 { -c.input.brake * self.ty.max_decel } else { c.input.throttle };
        self.cmd.push_back(cmd);
        if self.cmd.len() > 4 {
            self.cmd.pop_front();
        }
        if selected {
            let tgt = self.tgt;
            let reference = if tgt < f(46) { f(46) } else { v };
            if v < f(14) {
                if self.state != 3 && self.in_state >= 1.0 {
                    self.set(lib, 3, v, pos);
                }
            } else if (reference - tgt).abs() <= (f(38) * tgt).abs() {
                self.set_checked(lib, 4, v, pos);
            } else if acc > 0.1 {
                if tgt > v && v >= f(14) {
                    self.set_checked(lib, 1, v, pos);
                }
            } else if self.state != 3 && acc < -0.1 && tgt < v * (1.0 - f(38)) && self.state != 2 && self.in_state >= 1.0 {
                self.set(lib, 2, v, pos);
            }
            if self.state == 0 && self.in_state >= 1.0 {
                self.set(lib, 3, v, pos);
            }
            let pitch = match self.state {
                1 => {
                    self.accel_clock += dt;
                    if self.accel_clock > 20.0 {
                        self.set_checked(lib, 4, v, pos);
                    }
                    0
                }
                3 => ((v - f(18)) * 100.0) as i32,
                4 => ((v - f(42)) * 15.0).clamp(be_s16(&a, 50), be_s16(&a, 52)) as i32,
                _ => 0,
            };
            if let Some(h) = self.engine {
                lib.set(h, Req::at(pos).pitch(pitch));
            }
            // tyres
            let speed = st.speed() * MPH;
            if speed >= f(70) {
                let x = speed.max(f(70)).min(f(74));
                let t = (x - f(70)) / (f(74) - f(70)).max(1e-3);
                let vol = (be_s16(&a, 62) + (be_s16(&a, 64) - be_s16(&a, 62)) * t) * 0.01;
                let pitch = (be_s16(&a, 66) + (be_s16(&a, 68) - be_s16(&a, 66)) * t) as i32;
                keep(lib, &mut self.tyre, be_u32(&a, 54), Req::at(pos).vol(vol).pitch(pitch));
                // wet: the water tyre loop A+58 beside it with the same volume and pitch
                if wet {
                    keep(lib, &mut self.wet_tyre, be_u32(&a, 58), Req::at(pos).vol(vol).pitch(pitch));
                } else {
                    stop(lib, &mut self.wet_tyre);
                }
            } else {
                stop(lib, &mut self.tyre);
                stop(lib, &mut self.wet_tyre);
            }
        } else {
            self.deselect(lib);
        }
        self.in_state += dt;
        // skid (F-50163): above 10 mph the largest tyre stretch of the sliding wheels - |contact-point velocity -
        // forward x |v|| without its normal part, over one 30 Hz frame, past fTireStretch - or the emergency brake
        let mut s = 0.0f32;
        if st.vel.length_squared() > 19.98 {
            let up = st.rot * Vec3::Y;
            let fwd = st.forward();
            let sp = st.vel.length();
            for w in &st.wheels {
                let pv = st.vel + st.ang_vel.cross(w.world_pos - st.pos);
                let mut slip = pv - fwd * sp;
                slip -= up * slip.dot(up);
                let x = slip.length() / 30.0;
                if x > self.ty.tire_stretch {
                    s = s.max(x);
                }
            }
        }
        // F-37906: the car ahead in charge of the speed, the mean of the last 4 updates' asked
        // acceleration a deceleration of fMaxDecel, at 5 mph or more (traffic.rs `Lock`)
        let emergency = c.rail.as_ref().is_some_and(|r| r.lock.now());
        if emergency || s > 0.15 {
            let pitch = (be_s16(&a, 86) + (be_s16(&a, 88) - be_s16(&a, 86)) * s) as i32;
            keep(lib, &mut self.skid, be_u32(&a, 78), Req::at(pos).pitch(pitch));
        } else {
            stop(lib, &mut self.skid);
        }
        // the blocked horn (F-90275): 3 s blocked, not at an obstacle that has stood without input for 30 s
        if horn.blocked {
            self.wait += dt;
            if self.wait >= 3.0 && !horn.idle {
                self.honk(lib, pos, 90, None);
                self.wait = 0.0;
            }
        } else {
            self.wait = 0.0;
        }
        for h in [self.horn, self.speech] {
            if let Some(h) = h {
                lib.set(h, Req::at(pos));
            }
        }
    }

    /// The deselected path (and no driver): after 1 s in its state, engine and tyre stop (the wet tyre slot is kept).
    fn deselect(&mut self, lib: &mut SoundLib) {
        if self.state != 0 && self.in_state >= 1.0 {
            stop(lib, &mut self.engine);
            stop(lib, &mut self.tyre);
            self.state = 0;
            self.in_state = 0.0;
        }
    }

    /// F-86335 (A+90 horn, next after rand(2, 6) s) / F-39477 (A+94 bystander horn, 6 s): the slot free and
    /// the hold-off over.
    fn honk(&mut self, lib: &mut SoundLib, pos: Vec3, at: usize, hold: Option<f32>) {
        if self.age < self.next_horn || !free(lib, &self.horn) {
            return;
        }
        self.horn = lib.play_hash(be_u32(&self.obj, at), Req::at(pos));
        self.next_horn = self.age + hold.unwrap_or_else(|| lib.rand_float(2.0, 6.0));
    }

    /// E vt+36 (F-58581): the car's contacts of the frame. With a driver: a vehicle hit gives the horn (A+115 %),
    /// the driver's line (33 %, under 35 mph, iBoundGroup < 2), 4 s hold-off. Parked: any contact may start the alarm
    /// (A+114 %), 4 s hold-off.
    fn contacts(&mut self, lib: &mut SoundLib, c: &CarInstance, speech_pct: u8, driver: bool) {
        let st = &c.state;
        if st.touches.is_empty() {
            return;
        }
        let pos = st.pos;
        if driver {
            if self.age < self.next_hit || !st.touches.iter().any(|t| t.vehicle) {
                return;
            }
            if lib.rand_int(0, 99) < self.obj.get(115).copied().unwrap_or(0) as i32 {
                self.honk(lib, pos, 90, None);
            }
            if lib.rand_int(0, 99) < speech_pct as i32 && st.vel.length_squared() < 244.81 && self.ty.bound_group < 2 && !is_null(self.voice) && free(lib, &self.speech) && self.age >= self.next_speech {
                self.speech = lib.play_hash(self.voice, Req::at(pos));
                self.next_speech = self.age + lib.rand_float(2.0, 6.0);
            }
            self.next_hit = self.age + 4.0;
        } else {
            // (a hit wakes the parked car: its entity starts updating, car vt+72, I)
            self.awake = true;
            if free(lib, &self.alarm) && self.age >= self.next_hit {
                let h = be_u32(&self.obj, 110);
                if lib.rand_int(0, 99) < self.obj.get(114).copied().unwrap_or(0) as i32 && !is_null(h) {
                    self.alarm = lib.play_hash(h, Req::at(pos));
                }
                self.next_hit = self.age + 4.0;
            }
        }
    }

    /// F-74101: the player passing; A+98 / +102 when the car moves faster than 10 mph, else A+106; 5 s apart.
    fn whoosh(&mut self, lib: &mut SoundLib, c: &CarInstance, vol: f32, pitch: i32) {
        if self.age < self.next_whoosh {
            return;
        }
        let at = if c.state.speed() > 4.47 { 98 } else { 106 };
        let h = be_u32(&self.obj, at);
        if !is_null(h) {
            lib.play_hash(h, Req::at(c.state.pos).vol(vol).pitch(pitch));
        }
        self.next_whoosh = self.age + 5.0;
    }

    /// set(s) of F-32926: a new state only after 1 s in the last; STEADY from ACCEL once its clock is through.
    fn set_checked(&mut self, lib: &mut SoundLib, s: u8, v: f32, pos: Vec3) {
        if s == self.state || self.in_state < 1.0 {
            return;
        }
        if s == 4 && self.state == 1 && self.accel_clock < be_f32(&self.obj, 26) {
            return;
        }
        self.set(lib, s, v, pos);
    }

    fn set(&mut self, lib: &mut SoundLib, s: u8, v: f32, pos: Vec3) {
        let a = &self.obj;
        stop(lib, &mut self.engine);
        let (at, pitch) = match s {
            1 => {
                self.accel_clock = if self.tgt > 0.0 { v / self.tgt * be_f32(a, 26) } else { 0.0 };
                (22, 0)
            }
            2 => (30, 0),
            3 => (10, ((v - be_f32(a, 18)) * 100.0) as i32),
            _ => (34, ((v - be_f32(a, 42)) * 15.0).clamp(be_s16(a, 50), be_s16(a, 52)) as i32),
        };
        let h = be_u32(a, at);
        self.engine = if is_null(h) { None } else { lib.play_hash(h, Req::at(pos).pitch(pitch)) };
        self.state = s;
        self.in_state = 0.0;
    }
}

/// The call signs the cruisers are given, in the order of the PoliceVoices OFFICER group (F-23825).
const CALLSIGNS: [&str; 6] = ["3ADAM5", "1LINCOLN4", "4ADAM8", "4ADAM13", "2ADAM6", "1EDWARD7"];

struct Entry {
    /// the car's `uid`: the entry follows its car when the list moves (cars before it removed)
    uid: u32,
    model: String,
    sounds: Option<CarSounds>,
    engine: Option<EngineSound>,
    traffic: Option<Traffic>,
}

/// Every non-player car's sounds, and the player car's extras.
pub struct Fleet {
    mats: Materials,
    /// traffic model -> its `szAudioType`
    types: HashMap<String, TrafficType>,
    cars: HashMap<usize, Entry>,
    /// the car that owns the worldized chatter (global 0x8288DCE0), its handle and timer (this+280)
    chatter: Option<(usize, Handle, f32)>,
    trace_t: f32,
    pol: PoliceAudio,
    /// the ambient cars chosen for engine and tyre loops at the end of the last frame (F-21924)
    sel: Vec<usize>,
    /// racing vehicles' seconds without driving input ((car+896)->+112)
    idle: HashMap<usize, f32>,
    /// the speech manager's gate for PLAYER_NITROUS (CareerScript.xml Probability, Frequency; F-95157): its
    /// clock (+2244) and the scenario's last start (+24, -FLT_MAX at load)
    nitrous_line: (i32, f32, f32),
    speech_clock: f32,
    /// MCLA_SST_REPLAY (test hook): the recorded level / burn per original update (30 a second) and the time into it
    sst_replay: Option<(Vec<(f32, bool)>, f32)>,
    /// a player line the cars' sounds ask for this frame (the game says it if no speech plays)
    pub want_speech: Option<&'static str>,
}

/// What the police tell the cars' sounds: heat 0 none, 1 pull-over, 2 pursuit, 3 search, 4 busted; the payback's
/// chasers (no sirens); the law-breaking score; a speech or radio line playing.
#[derive(Clone, Copy, Default)]
pub struct PoliceIn {
    pub heat: u8,
    pub goons: bool,
    pub score: f32,
    pub speech_busy: bool,
}

#[derive(Default)]
struct PoliceAudio {
    heat: u8,
    chirp_pending: bool,
    bullhorn_done: bool,
    /// the cruisers of the running tactic
    tactic: Vec<usize>,
    /// call signs handed out (car -> index into CALLSIGNS) and the round-robin cursor
    callsigns: HashMap<usize, usize>,
    cursor: usize,
    bullhorn: Option<Handle>,
    /// the junction warn (siren/RULE.md 2): tactic+116 down time, the cops not allowed to warn (+1148 = 0), the warning cop
    warn_down: f32,
    warn_barred: Vec<usize>,
    warn_cop: Option<usize>,
}

impl PoliceAudio {
    /// A cruiser's call sign (F-23825): handed out round robin the first time one is needed.
    fn callsign(&mut self, car: usize) -> &'static str {
        let n = CALLSIGNS.len();
        let cursor = &mut self.cursor;
        let k = *self.callsigns.entry(car).or_insert_with(|| {
            let k = *cursor % n;
            *cursor += 1;
            k
        });
        CALLSIGNS[k]
    }
}

/// The player side's inputs that the car itself does not hold.
#[derive(Clone, Copy, Default)]
pub struct PlayerCtl {
    pub horn: bool,
    pub nitro_empty: bool,
    pub parts: [u8; 3],
    pub cam_mode: i32,
    /// the player's engine sound: smoothed rpm, load volume
    pub engine: Option<(f32, f32)>,
}

impl Fleet {
    pub fn new(data_dir: &Path) -> Fleet {
        let mut types = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(data_dir.join("tune/ambients/traffic")) {
            for e in rd.flatten() {
                let Ok(mut t) = std::fs::read_to_string(e.path().join("mcAmbientType.xml")) else { continue };
                t += &std::fs::read_to_string(e.path().join("mcAmbientWheelTuning.xml")).unwrap_or_default();
                let tag = |n: &str| t.split(&format!("<{n}>")).nth(1).and_then(|r| r.split('<').next()).map(|s| s.trim().to_string());
                let num = |n: &str| tag(n).and_then(|s| s.trim_start_matches("value=").parse::<f32>().ok()).or_else(|| t.split(&format!("<{n} value=\"")).nth(1).and_then(|r| r.split('"').next()).and_then(|s| s.parse().ok()));
                if let Some(ty) = tag("szAudioType") {
                    types.insert(
                        e.file_name().to_string_lossy().to_ascii_lowercase(),
                        TrafficType { audio: ty, bound_group: num("iBoundGroup").unwrap_or(0.0) as i32, max_decel: num("fMaxDecel").unwrap_or(22.5), tire_stretch: num("fTireStretch").unwrap_or(0.15) },
                    );
                }
            }
        }
        let (prob, freq) = scenario_gate(&std::fs::read_to_string(data_dir.join("tune/audio/CareerScript.xml")).unwrap_or_default(), "PLAYER_NITROUS");
        Fleet {
            mats: Materials::load(data_dir),
            types,
            cars: HashMap::new(),
            chatter: None,
            trace_t: 0.0,
            pol: PoliceAudio::default(),
            sel: Vec::new(),
            idle: HashMap::new(),
            nitrous_line: (prob, freq, -f32::MAX),
            speech_clock: 0.0,
            // "<recording>[;<seconds to wait first>]"
            sst_replay: std::env::var("MCLA_SST_REPLAY").ok().and_then(|v| {
                let (p, wait) = v.split_once(';').map_or((v.as_str(), 0.0), |(p, w)| (p, w.parse::<f32>().unwrap_or(0.0)));
                std::fs::read_to_string(p).ok().map(|t| (sst_replay_load(&t), -wait))
            }),
            want_speech: None,
        }
    }

    fn opponent(c: &CarInstance) -> bool {
        c.racer || c.pinned || c.cop || c.goon
    }

    /// F-90275's inputs for ambient car `i`: the racing vehicle its brain reacts to (the port's brake-for-car-ahead
    /// sensor: ahead within 18 m, 2.5 m aside), and whether it counts as blocking.
    fn horn_in(v: &Vehicles, i: usize, idle: &HashMap<usize, f32>) -> HornIn {
        let me = &v.cars[i];
        let (pos, fwd) = (me.state.pos, me.state.forward());
        let ob = v.cars.iter().enumerate().filter(|(j, o)| *j != i && o.placed && (o.is_player || Self::opponent(o))).filter_map(|(j, o)| {
            let rel = o.state.pos - pos;
            let ahead = rel.dot(fwd);
            let side = (rel - fwd * ahead).length();
            (ahead > 0.0 && ahead < 18.0 && side < 2.5).then_some((ahead, j))
        });
        let Some((_, j)) = ob.min_by(|a, b| a.0.total_cmp(&b.0)) else { return HornIn::default() };
        let o = &v.cars[j];
        let ofwd = o.state.forward();
        let oncoming = fwd.dot(ofwd) < 0.0;
        let ob_stopped = o.state.vel.length_squared() < 4.996;
        let me_stopped = me.state.vel.length_squared() < 4.996;
        // the obstacle's lane: its junction ahead (signals.rs); the limit is the original's 40 mph for a lane without one
        let (mut green, mut red) = (false, false);
        let lim = 40.0 * 0.44704;
        let slow = o.state.vel.length_squared() < (0.5 * lim) * (0.5 * lim);
        if let Some((ji, _, _)) = v.signals.ahead(o.state.pos, ofwd, v.clock) {
            // F-78185: the approach is within 25 degrees of the green phase, less than 6.5 s into it
            let jn = &v.signals.junctions[ji];
            let h = Vec3::new(ofwd.x, 0.0, ofwd.z).normalize_or_zero();
            let cur = jn.current as usize;
            if jn.arms.get(cur).is_some_and(|a| a.dot(-h) > 0.9063) && jn.timer < 6.5 {
                green = true;
            } else {
                red = true;
            }
        }
        let blocked = oncoming || (ob_stopped && me_stopped && (green || !red)) || (slow && !red);
        HornIn { blocked, idle: idle.get(&j).is_some_and(|t| *t >= 30.0) }
    }

    /// The player's pass-by scan (F-11431, GLOBAL_WHOOSHES): every car within 22 m passed at 40 mph or more
    /// (closer passes need less: ts >= (d - 10) / 12) whooshes; an ambient car may then honk at a player in front of
    /// its nose (45 deg, 30 %, else a 6 s hold-off).
    fn pass_by(&mut self, lib: &mut SoundLib, v: &Vehicles, pi: usize) {
        let Some(w) = lib.game.get_name("GLOBAL_WHOOSHES").map(|o| o.to_vec()) else { return };
        let p = &v.cars[pi];
        let (near, far, lo, hi) = (be_f32(&w, 10), be_f32(&w, 14), be_f32(&w, 18), be_f32(&w, 22));
        let (c0, c1) = (be_f32(&w, 26), be_f32(&w, 30));
        let (v0, v1, p0, p1) = (be_s16(&w, 34), be_s16(&w, 36), be_s16(&w, 38), be_s16(&w, 40));
        let (pct, cone) = (w.get(42).copied().unwrap_or(30) as i32, be_f32(&w, 43));
        // (entity+188: the player's 0..1 control value, I: the throttle)
        let tx = ((p.input.throttle - c0) / (c1 - c0).max(1e-6)).clamp(0.0, 1.0);
        let cell = |q: Vec3| ((q.x * 0.025).floor() as i32, (q.z * 0.025).floor() as i32);
        let (px, pz) = cell(p.state.pos);
        for (j, o) in v.cars.iter().enumerate() {
            if j == pi || !o.placed {
                continue;
            }
            let (ox, oz) = cell(o.state.pos);
            if (ox - px).abs() > 1 || (oz - pz).abs() > 1 {
                continue;
            }
            let d = o.state.pos.distance(p.state.pos);
            if d > far {
                continue;
            }
            let rel = (o.state.vel - p.state.vel).length() * MPH;
            if rel < lo {
                continue;
            }
            let ts = (rel.min(hi) - lo) / (hi - lo);
            if ts < (d - near) / (far - near) {
                continue;
            }
            let vol = (v0 + (v1 - v0) * ts.min(tx)) * 0.01;
            let pitch = (p0 + ts * (p1 - p0)) as i32;
            let Some(e) = self.cars.get_mut(&j) else { continue };
            if let Some(s) = &mut e.sounds {
                s.whoosh(lib, o.state.pos, vol, pitch);
            } else if let Some(t) = &mut e.traffic {
                t.whoosh(lib, o, vol, pitch);
                if t.age >= t.next_horn {
                    let to_p = (p.state.pos - o.state.pos).normalize_or_zero();
                    if to_p.dot(o.state.forward()) > cone {
                        if lib.rand_int(0, 99) < pct {
                            t.honk(lib, o.state.pos, 94, Some(6.0));
                        } else {
                            t.next_horn = t.age + 6.0;
                        }
                    }
                }
            }
        }
    }

    /// A frame of every car's sounds; `pol`: the police's state for the sirens, chirp, bullhorn and chatter.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, lib: &mut SoundLib, v: &Vehicles, listener: Vec3, ctl: PlayerCtl, wet: bool, racing: bool, pol: PoliceIn, dt: f32) {
        self.speech_clock += dt;
        // (the entries are kept by the car's place in the list: when cars before it go - traffic despawning - a car
        // moves down, and its sounds move with it rather than being dropped and loaded again on every move)
        let at: HashMap<u32, usize> = v.cars.iter().enumerate().map(|(i, c)| (c.uid, i)).collect();
        let moved: HashMap<usize, usize> = self.cars.iter().filter_map(|(&i, e)| at.get(&e.uid).map(|&j| (i, j))).filter(|(i, j)| i != j).collect();
        if !moved.is_empty() {
            let old = std::mem::take(&mut self.cars);
            for (_, mut e) in old {
                match at.get(&e.uid) {
                    Some(&j) => {
                        self.cars.insert(j, e);
                    }
                    None => Self::release(lib, &mut e),
                }
            }
            let m = |i: usize| moved.get(&i).copied().unwrap_or(i);
            self.sel = self.sel.iter().map(|&i| m(i)).collect();
            self.idle = self.idle.iter().map(|(&i, &t)| (m(i), t)).collect();
            if let Some(ch) = self.chatter.as_mut() {
                ch.0 = m(ch.0);
            }
            self.pol.tactic = self.pol.tactic.iter().map(|&i| m(i)).collect();
            self.pol.callsigns = self.pol.callsigns.iter().map(|(&i, &k)| (m(i), k)).collect();
            self.pol.warn_barred = self.pol.warn_barred.iter().map(|&i| m(i)).collect();
            self.pol.warn_cop = self.pol.warn_cop.map(m);
        }
        // cars that left
        let alive: Vec<usize> = self.cars.keys().copied().filter(|i| v.cars.get(*i).is_some_and(|c| c.placed && c.model.name == self.cars[i].model)).collect();
        let gone: Vec<usize> = self.cars.keys().copied().filter(|i| !alive.contains(i)).collect();
        for i in gone {
            if let Some(mut e) = self.cars.remove(&i) {
                Self::release(lib, &mut e);
            }
        }
        // the 7 nearest opponents get engine loops (F-25575), the 8 nearest traffic cars engine and tyres
        let mut opp: Vec<(f32, usize)> = Vec::new();
        let mut amb: Vec<(f32, usize)> = Vec::new();
        let mut parked: Vec<usize> = Vec::new();
        for (i, c) in v.cars.iter().enumerate() {
            if c.is_player || !c.placed {
                continue;
            }
            let d = c.state.pos.distance_squared(listener);
            if c.parked.is_some() {
                if !Self::opponent(c) {
                    parked.push(i);
                }
            } else if Self::opponent(c) {
                opp.push((d, i));
            } else if c.ai.is_some() {
                amb.push((d, i));
            }
        }
        // racing vehicles' time without input
        for (i, c) in v.cars.iter().enumerate() {
            if c.is_player || Self::opponent(c) {
                let t = self.idle.entry(i).or_insert(0.0);
                *t = if c.input.throttle == 0.0 && c.input.brake == 0.0 && c.input.steer == 0.0 { *t + dt } else { 0.0 };
            }
        }
        self.idle.retain(|i, _| v.cars.get(*i).is_some_and(|c| c.is_player || Self::opponent(c)));
        opp.sort_by(|a, b| a.0.total_cmp(&b.0));
        amb.sort_by(|a, b| a.0.total_cmp(&b.0));
        // the player's car
        if let Some((pi, p)) = v.cars.iter().enumerate().find(|(_, c)| c.is_player) {
            let e = entry(&mut self.cars, &self.types, lib, pi, p, true);
            let mut c = car_in(p);
            if let Some((v, t)) = &mut self.sst_replay {
                // (test hook) the recording's update at this time; 0 / not burning before and after it
                let k = (*t * 30.0).floor() as i64;
                SST_REPLAY_AT.store(k, std::sync::atomic::Ordering::Relaxed);
                *t += dt;
                c.sst = Some(usize::try_from(k).ok().and_then(|k| v.get(k).copied()).unwrap_or((0.0, false)));
            }
            c.wet = wet;
            c.horn = ctl.horn && !p.cop;
            c.nitro_empty = ctl.nitro_empty;
            c.parts = ctl.parts;
            if let Some((s, l)) = ctl.engine {
                c.rpm_s = s;
                c.load_vol = l;
            }
            if let Some(s) = &mut e.sounds {
                s.update(lib, &self.mats, &c, ctl.cam_mode, dt);
                // the slipstream burn's PLAYER_NITROUS request when no speech line plays (F-58518 @A-83150): the
                // speech manager (F-95157) refuses it within Frequency s of its last start, then unless
                // rand(0, 99) < Probability; the start time is kept when it passes
                if s.sst_speech && !pol.speech_busy {
                    let (prob, freq, last) = &mut self.nitrous_line;
                    sst_log(format_args!("SCEN clk {:.3} name PLAYER_NITROUS freq {freq:.3} last {last:.3} prob {prob}", self.speech_clock));
                    if self.speech_clock >= *last + *freq && lib.rand_int(0, 99) < *prob {
                        *last = self.speech_clock;
                        self.want_speech = Some("PLAYER_NITROUS");
                        sst_log(format_args!("SPEECHRET started"));
                    } else {
                        sst_log(format_args!("SPEECHRET 0"));
                    }
                }
                s.damage_parts(lib, &p.state.parts);
                s.glass(lib, &p.state.dmg, p.state.pos, p.state.rot);
                // a police car's horn button chirps the siren (F-44589)
                if p.cop && ctl.horn {
                    s.siren_alert(lib, c.pos);
                }
            }
        }
        // the police's events (siren/RULE.md section 7): the tactic's cruisers in spawn order, cop 0 leads
        let mut cops: Vec<usize> = opp.iter().map(|x| x.1).filter(|&i| v.cars[i].cop && !v.cars[i].goon).collect();
        cops.sort_unstable();
        let lead = cops.first().copied();
        let heat = if pol.goons { 0 } else { pol.heat };
        let mut starts: Vec<(usize, f32)> = Vec::new();
        let mut stop_all = false;
        let pa = &mut self.pol;
        if heat != pa.heat {
            match (pa.heat, heat) {
                (_, 1) => {
                    pa.chirp_pending = true;
                    pa.bullhorn_done = false;
                }
                // a pursuit begins: cop 0 at once, the others after their own rand x fMaxSirenDelayTime (3 s)
                (0 | 1, 2) => {
                    pa.tactic = cops.clone();
                    for (k, &i) in cops.iter().enumerate() {
                        starts.push((i, if k == 0 { 0.0 } else { lib.rand_float(0.0, 1.0) * 3.0 }));
                    }
                }
                // escaped, busted, ticket: the tactic ends and every siren stops (Search keeps them)
                (_, 0 | 4) => {
                    stop_all = true;
                    pa.tactic.clear();
                }
                _ => {}
            }
            pa.heat = heat;
        }
        if heat == 2 || heat == 3 {
            // cruisers that join the chase start theirs after their own delay
            for &i in &cops {
                if !pa.tactic.contains(&i) {
                    pa.tactic.push(i);
                    starts.push((i, lib.rand_float(0.0, 1.0) * 3.0));
                }
            }
        }
        // the junction warn (F-63286): the cop nearest the player, inside a junction or within its size / 2 + 10 m of
        // the next one ahead (the port's signalled junctions: centre, radius), 20 % -> warn, else 15 s without
        let mut warn_on = None;
        let mut warn_off = None;
        {
            let pa = &mut self.pol;
            let pp = v.player().map(|p| p.state.pos);
            if heat == 2 || heat == 3 {
                if pa.warn_down > 0.0 {
                    pa.warn_down -= dt;
                    if pa.warn_down <= 0.0 {
                        pa.warn_down = 0.0;
                        pa.warn_barred.clear();
                    }
                }
                let can = |i: usize, barred: &Vec<usize>| -> bool {
                    let Some(c) = v.cars.get(i) else { return false };
                    if barred.contains(&i) || !c.placed {
                        return false;
                    }
                    let p = c.state.pos;
                    let inside = v.signals.junctions.iter().any(|j| (j.center.y - p.y).abs() < 7.0 && Vec3::new(j.center.x - p.x, 0.0, j.center.z - p.z).length() < j.radius);
                    let dir = if c.state.speed() >= 2.235 { c.state.vel } else { c.state.forward() };
                    inside || v.signals.ahead(p, dir, v.clock).is_some_and(|(ji, _, _)| {
                        let j = &v.signals.junctions[ji];
                        Vec3::new(j.center.x - p.x, 0.0, j.center.z - p.z).length() < j.radius + 10.0
                    })
                };
                if let Some(w) = pa.warn_cop {
                    if !can(w, &Vec::new()) {
                        warn_off = Some(w);
                        pa.warn_cop = None;
                        pa.warn_down = 15.0;
                        pa.warn_barred = cops.clone();
                    }
                } else if let (Some(pp), true) = (pp, pa.warn_down <= 0.0) {
                    let near = cops.iter().copied().min_by(|a, b| v.cars[*a].state.pos.distance(pp).total_cmp(&v.cars[*b].state.pos.distance(pp)));
                    if let Some(l) = near.filter(|&l| can(l, &pa.warn_barred)) {
                        if lib.rand_float(0.0, 1.0) < 0.2 {
                            warn_on = Some(l);
                            pa.warn_cop = Some(l);
                            pa.warn_barred = cops.iter().copied().filter(|&c| c != l).collect();
                        } else {
                            pa.warn_down = 15.0;
                            pa.warn_barred = cops.clone();
                        }
                    }
                }
            } else {
                pa.warn_cop = None;
                pa.warn_down = 0.0;
                pa.warn_barred.clear();
            }
        }
        // computer racers and cruisers
        for (rank, &(_, i)) in opp.iter().enumerate() {
            let c = &v.cars[i];
            let e = entry(&mut self.cars, &self.types, lib, i, c, false);
            let mut ci = car_in(c);
            ci.wet = wet;
            if rank < 7 {
                if e.engine.is_none() {
                    e.engine = EngineSound::new_opponent(lib, &c.model.name, c.model.bike);
                }
                if let Some(en) = &mut e.engine {
                    let inp = engine_in(c, racing && !c.cop && c.ai.is_some());
                    en.update(lib, &inp, dt);
                }
            } else if let Some(mut en) = e.engine.take() {
                en.stop(lib);
            }
            if let Some(en) = &e.engine {
                ci.rpm_s = en.smoothed_rpm();
                ci.load_vol = en.load_volume();
            }
            if let Some(s) = &mut e.sounds {
                s.update(lib, &self.mats, &ci, -1, dt);
                s.damage_parts(lib, &c.state.parts);
                s.glass(lib, &c.state.dmg, c.state.pos, c.state.rot);
                if c.cop && !c.goon {
                    if stop_all {
                        s.siren_stop(lib);
                    }
                    if let Some(&(_, d)) = starts.iter().find(|x| x.0 == i) {
                        s.siren_start(d);
                    }
                    if warn_on == Some(i) {
                        s.siren_warn(lib, ci.pos, true, true);
                    } else if warn_off == Some(i) {
                        s.siren_warn(lib, ci.pos, false, heat == 2 || heat == 3);
                    }
                    // pull-over (F-22530): the chirp on cop 0 once no speech plays (after REPORT_PULL_SUSPECT_OVER);
                    // at law score 500 the bullhorn once - skipped for good if speech is playing then
                    if heat == 1 && Some(i) == lead {
                        if self.pol.chirp_pending && !pol.speech_busy {
                            s.siren_alert(lib, ci.pos);
                            self.pol.chirp_pending = false;
                        }
                        if pol.score >= 500.0 && !self.pol.bullhorn_done {
                            self.pol.bullhorn_done = true;
                            if !pol.speech_busy {
                                let k = self.pol.callsign(i);
                                // (the radio manager's one slot +224)
                                if let Some(h) = self.pol.bullhorn.take() {
                                    lib.stop(h);
                                }
                                self.pol.bullhorn = lib.play(&format!("POLICE_BULLHORN_{k}"), Req::at(ci.pos));
                            }
                        }
                    }
                    // (the wrecked word: set by a wipe-out or destruction, chassis +592 0x10 / 0x04)
                    s.siren_frame(lib, ci.pos, c.state.wipeout > 0.0 || c.state.damage >= 1.0, dt);
                }
            }
        }
        // call signs go back when their cruiser leaves
        self.pol.callsigns.retain(|i, _| v.cars.get(*i).is_some_and(|c| c.cop && c.placed));
        // the worldized chatter (F-65788 / F-33221), every frame in any state: the nearest police car within
        // 35 m of the listener (not the player's); started for 4 s, kept alive 0.15 s at a time while it stays nearest
        if let Some((i, h, t)) = &mut self.chatter {
            *t -= dt;
            if *t <= 0.0 || racing || !lib.playing(*h) {
                lib.stop(*h);
                self.chatter = None;
            } else if let Some(c) = v.cars.get(*i) {
                lib.set(*h, Req::at(c.state.pos));
            }
        }
        // MANAGER_OPPONENT_GAINS (siren/RULE.md 9.1): every racer-list car (computer racers and cruisers) within 9 m
        // adds 1, between 9 and 50 m (d - 9) / 41 (the original's literal formula)
        let w: f32 = v.cars.iter().filter(|c| !c.is_player && c.placed && (c.racer || c.cop)).map(|c| c.state.pos.distance(listener)).filter(|d| *d < 50.0).map(|d| if d <= 9.0 { 1.0 } else { (d - 9.0) / 41.0 }).sum();
        lib.set_presence(w);
        // (the chatter stops while DYNAMIC_MIXER_RACE runs: mixer flag 0x4000, a race in progress)
        let cand = v.cars.iter().enumerate().filter(|_| !racing).filter(|(_, c)| c.cop && !c.goon && !c.is_player && c.placed).map(|(i, c)| (c.state.pos.distance(listener), i)).filter(|(d, _)| *d <= 35.0).min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1);
        if let Some(i) = cand {
            match &mut self.chatter {
                Some((o, _, t)) if *o == i => *t = 0.15,
                Some(_) => {}
                None => {
                    let h = self.cars.get(&i).and_then(|e| e.sounds.as_ref()).map(|s| be_u32(&s.police, 22)).unwrap_or(0);
                    if let Some(h) = (!is_null(h)).then(|| lib.play_hash(h, Req::at(v.cars[i].state.pos))).flatten() {
                        self.chatter = Some((i, h, 4.0));
                    }
                }
            }
        }
        if let (Some(h), Some(i)) = (self.pol.bullhorn, lead) {
            if lib.playing(h) {
                lib.set(h, Req::at(v.cars[i].state.pos));
            }
        }
        // traffic (traffic/RULE.md): the cars with a driver update; every one hears its contacts
        let speech_pct = lib.game.get_name("AMBIENT_VEHICLE_DRIVER_MGR").and_then(|o| o.get(10).copied()).unwrap_or(0);
        for &(_, i) in &amb {
            let c = &v.cars[i];
            let horn = Self::horn_in(v, i, &self.idle);
            let selected = self.sel.contains(&i);
            let e = entry(&mut self.cars, &self.types, lib, i, c, false);
            if let Some(t) = &mut e.traffic {
                t.update(lib, c, selected, wet, horn, dt);
                t.contacts(lib, c, speech_pct, true);
            }
        }
        for &i in &parked {
            let c = &v.cars[i];
            if c.state.touches.is_empty() && !self.cars.contains_key(&i) {
                continue;
            }
            let e = entry(&mut self.cars, &self.types, lib, i, c, false);
            if let Some(t) = &mut e.traffic {
                t.deselect(lib);
                t.contacts(lib, c, speech_pct, false);
                if t.awake {
                    t.age += dt;
                }
                if let Some(h) = t.alarm {
                    lib.set(h, Req::at(c.state.pos));
                }
            }
        }
        // the player's pass-by scan (F-11431)
        if let Some(pi) = v.cars.iter().position(|c| c.is_player) {
            self.pass_by(lib, v, pi);
        }
        // the selection for the next frame (F-47008): the 3 x 3 40 m cells around the listener, rows (z) then
        // columns (x), each cell's newest entrant first; the first 8 fill the list, a later one replaces the first
        // listed car that is farther away
        let cell = |p: Vec3| ((p.x * 0.025).floor() as i32, (p.z * 0.025).floor() as i32);
        let (lx, lz) = cell(listener);
        let mut sel: Vec<(f32, usize)> = Vec::new();
        // (the port has no cell lists: a later-spawned car - higher index - stands for the newer entrant)
        let mut by_index = amb.clone();
        by_index.sort_by(|a, b| b.1.cmp(&a.1));
        for dz in -1..=1 {
            for dx in -1..=1 {
                for &(d, i) in &by_index {
                    if cell(v.cars[i].state.pos) != (lx + dx, lz + dz) {
                        continue;
                    }
                    if sel.len() < 8 {
                        sel.push((d, i));
                    } else if let Some(k) = sel.iter().position(|s| s.0 > d) {
                        sel[k] = (d, i);
                    }
                }
            }
        }
        self.sel = sel.into_iter().map(|x| x.1).collect();
        // cars whose sounds were not updated this frame (parked, left the lists) let theirs go
        let seen: Vec<usize> = opp.iter().chain(amb.iter()).map(|x| x.1).chain(parked.iter().copied()).chain(v.cars.iter().position(|c| c.is_player)).collect();
        let stale: Vec<usize> = self.cars.keys().copied().filter(|i| !seen.contains(i)).collect();
        for i in stale {
            if let Some(mut e) = self.cars.remove(&i) {
                Self::release(lib, &mut e);
            }
        }
        if trace() {
            self.trace_t -= dt;
            if self.trace_t <= 0.0 {
                self.trace_t = 1.0;
                let live = |h: &Option<Handle>| h.is_some_and(|h| lib.playing(h)) as u8;
                if let Some(s) = v.cars.iter().position(|c| c.is_player).and_then(|i| self.cars.get(&i)).and_then(|e| e.sounds.as_ref()) {
                    let rolls: u8 = s.w.iter().map(|r| live(&r.roll)).sum();
                    let skids: u8 = s.w.iter().map(|r| live(&r.skid)).sum();
                    eprintln!(
                        "car audio: player rolls {rolls} skids {skids} turbo {} horn {} nitro {} pings {} hyd {} | opponents {} engines {} | traffic {} engines {}",
                        live(&s.turbo_v),
                        live(&s.horn),
                        s.nitro.is_some() as u8,
                        live(&s.pings),
                        s.hyd_loop.is_some() as u8,
                        opp.len(),
                        self.cars.values().filter(|e| e.engine.is_some()).count(),
                        amb.len(),
                        self.cars.values().filter_map(|e| e.traffic.as_ref()).filter(|t| t.engine.is_some_and(|h| lib.playing(h))).count(),
                    );
                    let st: Vec<String> = amb.iter().take(8).filter_map(|(d, i)| self.cars.get(i)?.traffic.as_ref().map(|t| format!("{}{}{}:{:.0}/{:.0}@{:.0}m", if self.sel.contains(i) { "*" } else { "" }, t.state, if t.engine.is_some_and(|h| lib.playing(h)) { "+" } else { "-" }, t.prev_v.unwrap_or(0.0), t.tgt, d.sqrt()))).collect();
                    eprintln!("car audio: traffic states {}", st.join(" "));
                }
            }
        }
    }

    fn release(lib: &mut SoundLib, e: &mut Entry) {
        if let Some(s) = &mut e.sounds {
            s.stop_all(lib);
        }
        if let Some(en) = &mut e.engine {
            en.stop(lib);
        }
        if let Some(t) = &mut e.traffic {
            t.stop_all(lib);
        }
    }

    pub fn stop_all(&mut self, lib: &mut SoundLib) {
        for (_, mut e) in self.cars.drain() {
            Self::release(lib, &mut e);
        }
        if let Some((_, h, _)) = self.chatter.take() {
            lib.stop(h);
        }
    }

}

fn entry<'a>(cars: &'a mut HashMap<usize, Entry>, types: &HashMap<String, TrafficType>, lib: &mut SoundLib, i: usize, c: &CarInstance, player: bool) -> &'a mut Entry {
        let traffic = !player && !Fleet::opponent(c);
        // (a car that changed kind - a racer let go into the traffic, a car taken into an event - gets a new entity)
        if cars.get(&i).is_some_and(|e| e.traffic.is_some() != traffic || (e.sounds.as_ref().is_some_and(|s| s.player) != player)) {
            if let Some(mut e) = cars.remove(&i) {
                Fleet::release(lib, &mut e);
            }
        }
        cars.entry(i).or_insert_with(|| {
            let amb = traffic.then(|| {
                // the type's object by name, else "<name>_0<0|1>" (F-89483)
                let tt = types.get(&c.model.name.to_ascii_lowercase()).cloned().unwrap_or(TrafficType { audio: "AMBIENT_SEDAN".into(), bound_group: 0, max_decel: 22.5, tire_stretch: 0.15 });
                let ty = tt.audio.clone();
                let pick = lib.rand_int(0, 1);
                let obj = lib.game.get_name(&ty).or_else(|| lib.game.get_name(&format!("{ty}_0{pick}"))).or_else(|| lib.game.get_name("AMBIENT_SEDAN_00")).map(|o| o.to_vec()).unwrap_or_default();
                // the driver's voice (F-89483): a random AMBIENT_VEHICLE_DRIVER_MGR entry, its +10 sound
                let mgr = lib.game.get_name("AMBIENT_VEHICLE_DRIVER_MGR").map(|o| o.to_vec()).unwrap_or_default();
                let n = mgr.get(11).copied().unwrap_or(0) as i32;
                let voice = if n > 0 { {
                    let k = lib.rand_int(0, n - 1) as usize;
                    lib.game.get(be_u32(&mgr, 12 + 4 * k))
                }.map_or(0, |o| be_u32(o, 10)) } else { 0 };
                Traffic {
                    obj,
                    ty: tt,
                    engine: None,
                    state: 0,
                    in_state: 0.0,
                    accel_clock: 0.0,
                    tyre: None,
                    wet_tyre: None,
                    skid: None,
                    horn: None,
                    alarm: None,
                    speech: None,
                    tgt: 0.0,
                    prev_v: None,
                    age: 0.0,
                    awake: false,
                    wait: 0.0,
                    next_horn: 0.0,
                    next_whoosh: 0.0,
                    next_hit: 0.0,
                    next_speech: 0.0,
                    voice,
                    cmd: Default::default(),
                }
            });
            Entry {
                uid: c.uid,
                model: c.model.name.clone(),
                sounds: (!traffic).then(|| CarSounds::new(lib, &c.model.name, c.model.bike, player)),
                engine: None,
                traffic: amb,
            }
        })
    }

#[cfg(test)]
mod tests {

    /// F-46044 against the original's own code on 20000 inputs (work/audio2/verify/ref6.py)
    #[test]
    fn log10_is_the_originals() {
        let mut bad = 0;
        let mut first = Vec::new();
        for line in include_str!("log10_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let x = f64::from_bits(u64::from_str_radix(i, 16).unwrap());
            let want = u64::from_str_radix(o, 16).unwrap();
            if log10_guest(x).to_bits() != want {
                bad += 1;
                if first.len() < 3 {
                    first.push((x, f64::from_bits(want), log10_guest(x)));
                }
            }
        }
        assert_eq!(bad, 0, "{first:?}");
    }

    /// F-90102 against the original's own code on 4000 cases (work/audio2/verify/ref1.py; evidence docs/reference/audio/verify_audio_b1.md)
    #[test]
    fn approach_is_the_originals() {
        let mut bad = 0;
        for line in include_str!("approach_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<u32> = i.split(' ').skip(1).map(|x| u32::from_str_radix(&x.replace('_', ""), 16).unwrap()).collect();
            let want: f32 = f32::from_bits({ let x = o.split(' ').nth(1).unwrap(); u32::from_str_radix(&x.replace('_', ""), 16).unwrap() });
            let mut p = f32::from_bits(v[0]);
            approach_fma(&mut p, f32::from_bits(v[1]), f32::from_bits(v[2]), f32::from_bits(v[3]));
            if p.to_bits() != want.to_bits() {
                bad += 1;
            }
        }
        assert_eq!(bad, 0);
    }
    use super::*;

    /// The Camaro's tyre, suspension, horn and siren objects from the disc, a frame of rolling and sliding, and the
    /// ambient sedan's object.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn car_sounds_from_disc() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let mats = Materials::load(Path::new(&dir));
        let (road, thr) = mats.get(Surface::Road as u8);
        assert_eq!(lib.game.name_of(road), Some("MATERIALS_ROAD"));
        assert!((thr - 0.9).abs() < 1e-3, "{thr}");
        let mut s = CarSounds::new(&lib, "vp_chv_camaro_06", false, true);
        assert_eq!(s.horn_hash, joaat("MUSTANG_HORN_FIN"));
        assert_eq!(be_u32(&s.susp, 41), joaat("SUSPENSION_MASTER"));
        assert_eq!(be_u32(&s.police, 18), joaat("POLICE_SIREN_LOOP_CROSSFADE"));
        assert!((s.skid_db - 2.0).abs() < 1e-3);
        let w = WheelIn { contact: true, surface: Surface::Road as u8, speed_mph: 55.0, slide: 0.6, travel: 0.3, limit: 0.1, ..Default::default() };
        let c = CarIn { wheels: [w; 4], n_wheels: 4, horn: true, ..Default::default() };
        for _ in 0..30 {
            s.update(&mut lib, &mats, &c, 2, 1.0 / 30.0);
        }
        let rolls = s.w[..4].iter().filter(|r| r.roll.is_some_and(|h| lib.playing(h))).count();
        let skids = s.w[..4].iter().filter(|r| r.skid.is_some_and(|h| lib.playing(h))).count();
        println!("rolls {rolls} skids {skids} skid t {:.3} gain {:.3} ratio {:.3}", s.w[0].t, s.w[0].gain, s.w[0].ratio);
        assert_eq!((rolls, skids), (4, 4));
        // ROAD: threshold 0.25, slide 0.6 -> t = 0.467, t^0.1, pitch 0.85 + 0.3 t
        assert!((s.w[0].t - (0.6 - 0.25) / 0.75).abs() < 0.01);
        assert!((s.w[0].gain - s.w[0].t.powf(0.1)).abs() < 0.01);
        assert!(s.horn.is_some_and(|h| lib.playing(h)));
        let types = Fleet::new(Path::new(&dir)).types;
        assert!(types.values().any(|t| t.audio == "AMBIENT_SEDAN"), "{types:?}");
        // (the sedans: iBoundGroup 0, fMaxDecel and fTireStretch from the tune files)
        assert!(types.values().all(|t| t.max_decel > 0.0 && t.tire_stretch > 0.0), "{types:?}");
        assert!(lib.game.get_name("AMBIENT_SEDAN_00").is_some());
        let opp = EngineSound::new_opponent(&lib, "vp_chv_camaro_06", false);
        assert!(opp.is_some());
    }

    /// The slipstream turbo's sounds (work/slipstream/audio/audio_rule.md): GLOBAL_SPECIAL_ABILITIES' fields, the charge
    /// loop's level / pitch along the charge, the beep at full, the burn (player and opponent), mix slot 18 and the
    /// PLAYER_NITROUS gate (Probability 30, Frequency 120 s).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn slipstream_sounds_from_disc() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let mats = Materials::load(Path::new(&dir));
        let mut s = CarSounds::new(&lib, "vp_chv_camaro_06", false, true);
        let p = s.sst_obj.clone();
        println!(
            "GLOBAL_SPECIAL_ABILITIES: +99 {:08x} +103 {} +107 {} +109 {} +113 {} +117 {} +119 {} +123 {:08x} +127 {:08x} +131 {:08x}",
            be_u32(&p, 99),
            be_f32(&p, 103),
            be_s16(&p, 107),
            be_f32(&p, 109),
            be_f32(&p, 113),
            be_s16(&p, 117),
            be_f32(&p, 119),
            be_u32(&p, 123),
            be_u32(&p, 127),
            be_u32(&p, 131)
        );
        assert_eq!(be_u32(&p, 99), joaat("SST_LOOP_MASTER"));
        assert_eq!(be_u32(&p, 123), joaat("SST_BEEP_MASTER"));
        assert_eq!(be_u32(&p, 127), joaat("SLIPSTREAM_NITROUS_MASTER"));
        assert_eq!(be_u32(&p, 131), joaat("SLIPSTREAM_NITROUS_MASTER_O"));
        assert_eq!((be_f32(&p, 103), be_s16(&p, 107), be_f32(&p, 109)), (0.05, -500.0, 0.4));
        assert_eq!((be_f32(&p, 113), be_s16(&p, 117), be_f32(&p, 119)), (1.0, 0.0, 1.5));
        let dt = 1.0 / 30.0;
        let w = WheelIn { contact: true, driven: true, surface: Surface::Road as u8, ..Default::default() };
        let mut c = CarIn { wheels: [w; 4], n_wheels: 4, sst: Some((0.0, false)), ..Default::default() };
        let live = |lib: &SoundLib, h: &Option<Handle>| h.is_some_and(|h| lib.playing(h));
        // below 0.05: nothing; along the charge: the loop at lerp(-5, 0) dB and 1200 log2(lerp(0.4, 1.5)) cents
        for (l, db, ct) in [(0.04, None, 0), (0.05, Some(-5.0), -1586), (0.525, Some(-2.5), -88), (0.99, Some(-0.0526), 688)] {
            c.sst = Some((l, false));
            s.update(&mut lib, &mats, &c, 2, dt);
            let req = live(&lib, &s.sst_loop).then_some(s.sst_vp);
            println!("level {l}: loop {req:?} full {}", live(&lib, &s.sst_full));
            match db {
                None => assert!(!live(&lib, &s.sst_loop)),
                Some(db) => {
                    let (v, p) = req.expect("loop");
                    assert!((v - db).abs() < 1e-3 && p == ct, "{v} {p}");
                }
            }
            assert!(!live(&lib, &s.sst_full));
        }
        // full: the loop stops, the beep (a looping wave) runs
        c.sst = Some((1.0, false));
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!(!live(&lib, &s.sst_loop) && live(&lib, &s.sst_full));
        // the burn: both stop, the burn sound, mix slot 18, the speech request once, the launch chirp
        c.sst = Some((1.0, true));
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!(!live(&lib, &s.sst_loop) && !live(&lib, &s.sst_full) && live(&lib, &s.sst_burn));
        assert!(lib.scene_on("DYNAMIC_MIXER_SST") && s.sst_speech);
        // (no launch chirp yet: this+264 starts at 0 with the entity's clock and the clock is not 0.5 s on - recorded)
        assert!(s.clock < 0.5 && s.chirp_at == 0.0);
        c.sst = Some((0.5, true));
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!(!s.sst_speech && live(&lib, &s.sst_burn));
        c.sst = Some((0.0, false));
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!(!live(&lib, &s.sst_burn) && !lib.scene_on("DYNAMIC_MIXER_SST"));
        // an opponent: only the burn, _O_
        let mut o = CarSounds::new(&lib, "vp_chv_camaro_06", false, false);
        c.sst = Some((0.7, false));
        o.update(&mut lib, &mats, &c, -1, dt);
        assert!(!live(&lib, &o.sst_loop) && !live(&lib, &o.sst_burn));
        c.sst = Some((0.7, true));
        o.update(&mut lib, &mats, &c, -1, dt);
        assert!(live(&lib, &o.sst_burn) && !lib.scene_on("DYNAMIC_MIXER_SST"));
        let xml = std::fs::read_to_string(Path::new(&dir).join("tune/audio/CareerScript.xml")).unwrap();
        assert_eq!(scenario_gate(&xml, "PLAYER_NITROUS"), (30, 120.0));
        assert_eq!(scenario_gate(&xml, "PLAYER_HORN"), (30, 10.0));
    }

    /// F-98474's pitch arithmetic against the running original (work/slipstream/audiorec, r5: the charge loop's
    /// ratio as passed and the cents found in the voice block right after): the recorded pairs, and a ratio where the
    /// plain single-precision 1200 log2 would be one cent off.
    #[test]
    fn voice_cents_recorded() {
        for (r, c) in [(0.406076461f32, -1560), (0.703386903, -609), (1.04511762, 76), (1.42090929, 608), (0.829858959, -322), (1.37913656, 556), (0.642281413, -766)] {
            assert_eq!(voice_cents(r), c, "{r}");
        }
        assert_eq!((voice_cents(0.411_082_54), cents(0.411_082_54)), (-1538, -1539));
    }

    /// The driven records (+40): layout 0 / 2 / 4 the rear pair, 1 / 3 the front pair (the recorded R8, layout 2: records 2
    /// and 3 only).
    #[test]
    fn driven_records() {
        let pick = |d: u8| (0..4).map(|i| driven_record(d, i)).collect::<Vec<_>>();
        for d in [0, 2, 4, 255] {
            assert_eq!(pick(d), [false, false, true, true]);
        }
        for d in [1, 3] {
            assert_eq!(pick(d), [true, true, false, false]);
        }
    }

    /// The slipstream sounds against recordings of the running original (recomp-trace's sst_audio recorder,
    /// work/slipstream/audiorec cap/r1, r3, r5): the loop's dB and cents at recorded levels, the beep's start and stop
    /// with the level, the burn's chirp from the driven rear records only, its restart chirp 0.5 s after the last, the
    /// envelopes of SLIPSTREAM_NITROUS_MASTER (SST_ENV_ATTACK: 1 for 250 ms, then linear to 0 over 500 ms; SST_ENV_TAIL:
    /// after its 250 ms predelay sin(t/500 * pi/2) up to 1, held) and its stop: the wrapper's 2 s release over the
    /// envelopes as they were.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn slipstream_matches_recording() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let mats = Materials::load(Path::new(&dir));
        let mut s = CarSounds::new(&lib, "vp_aud_r8_08", false, true);
        let dt = 1.0 / 30.0;
        let live = |lib: &SoundLib, h: &Option<Handle>| h.is_some_and(|h| lib.playing(h));
        let mut c = CarIn { n_wheels: 4, sst: Some((0.0, false)), ..Default::default() };
        for (i, w) in c.wheels.iter_mut().enumerate() {
            *w = WheelIn { contact: true, driven: driven_record(2, i), surface: Surface::Road as u8, ..Default::default() };
        }
        // (L, dB, cents) recorded (r1 f955, f981, f1057, f445; L printed to 5 decimals)
        for (l, db, ct) in [(0.05064f32, -4.99664f32, -1583), (0.56536, -2.28756, -5), (0.96296, -0.19494, 651), (0.99307, -0.03646, 692)] {
            c.sst = Some((l, false));
            s.update(&mut lib, &mats, &c, 2, dt);
            let (v, p) = s.sst_vp;
            assert!(live(&lib, &s.sst_loop) && (v - db).abs() < 3e-5 && p == ct, "{l}: {v} {p}");
        }
        // below 0.05 (r3 f888: 0.01850): the loop stops; at 1: the loop stops and the beep starts (r3 f552)
        c.sst = Some((0.0185, false));
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!(!live(&lib, &s.sst_loop) && !live(&lib, &s.sst_full));
        c.sst = Some((1.0, false));
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!(!live(&lib, &s.sst_loop) && live(&lib, &s.sst_full));
        // the stay-full time ran out (r3 f862: 0.98148): the beep stops and the loop starts again in the same update
        c.sst = Some((0.98148, false));
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!(live(&lib, &s.sst_loop) && !live(&lib, &s.sst_full) && (s.sst_vp.0 + 0.0975).abs() < 1e-3);
        // the fire (r3 f641, burn = 1 at L 0.98667): the loop / beep stop, the burn starts, the chirp from records 2 and 3
        s.clock = 21.3667 - dt;
        s.chirp_at = 15.1001;
        c.sst = Some((0.98667, true));
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!(live(&lib, &s.sst_burn) && !live(&lib, &s.sst_loop) && s.sst_speech && lib.scene_on("DYNAMIC_MIXER_SST"));
        assert!((s.chirp_at - 21.3667).abs() < 1e-4);
        // a restarted burn (the slot found empty, r3 f700 at clock 23.3333, last chirp 21.3667): it chirps again; with the
        // wheels off the ground (r1 f940) the chirp time stays
        let h = s.sst_burn.take().unwrap();
        lib.stop(h);
        s.clock = 23.3333 - dt;
        c.sst = Some((0.16, true));
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!((s.chirp_at - 23.3333).abs() < 1e-4 && !s.sst_speech);
        let h = s.sst_burn.take().unwrap();
        lib.stop(h);
        s.clock = 31.3332 - dt;
        s.chirp_at = 29.2666;
        for w in c.wheels.iter_mut() {
            w.contact = false;
        }
        s.update(&mut lib, &mats, &c, 2, dt);
        assert!((s.chirp_at - 29.2666).abs() < 1e-4 && live(&lib, &s.sst_burn));
        // the burn's envelopes (r5, on the audio clock): the two leaves of SST_MULTI less their fixed chain dB
        let h = lib.play_hash(joaat("SLIPSTREAM_NITROUS_MASTER"), Req::default()).expect("burn");
        let attack = |t: f32| if t < 250.0 { 1.0 } else { (1.0 - (t - 250.0) / 500.0).max(1e-5) };
        let tail = |t: f32| ((t - 250.0).clamp(0.0, 500.0) / 500.0 * std::f32::consts::FRAC_PI_2).sin().max(1e-5);
        let mut base: Option<(f32, f32)> = None;
        for t in 1..=1200 {
            let v = lib.test_leaf_vols(h, 0.001);
            let tf = t as f32;
            if t == 300 {
                assert_eq!(v.len(), 2, "{v:?}");
                base = Some((v[0] - lin2db(attack(tf)), v[1] - lin2db(tail(tf))));
            }
            if let (Some((b0, b1)), true) = (base, t > 300 && t % 50 == 0 && t < 700) {
                println!("t {t}: attack {:.3} dB (model {:.3}), tail {:.3} dB (model {:.3})", v[0] - b0, lin2db(attack(tf)), v[1] - b1, lin2db(tail(tf)));
                assert!((v[0] - b0 - lin2db(attack(tf))).abs() < 0.1, "attack at {t}: {v:?}");
                assert!((v[1] - b1 - lin2db(tail(tf))).abs() < 0.1, "tail at {t}: {v:?}");
            }
        }
        // the stop: SLIPSTREAM_NITROUS_MASTER's ReleaseTime 2000 ms fades the whole tree; the tail envelope stays at 1
        let (_, b1) = base.unwrap();
        lib.stop(h);
        for k in 1..=2100 {
            let v = lib.test_leaf_vols(h, 0.001);
            if k == 1000 {
                println!("1 s into the release: {v:?} (tail alone {b1:.3})");
                assert!(v.len() == 1 && (v[0] - b1 - lin2db(0.5)).abs() < 0.1, "half-way: {v:?}");
            }
            if k == 2100 {
                assert!(v.is_empty(), "{v:?}");
            }
        }
    }


    /// TUNERS_DAMAGE's records as DAMAGE_PARTS_RULE.md 4.3 reads them, and the parts' sounds started by their
    /// events: the hood's slam, the break-off, the fallen bumper's impact and scrape (stopped 1/15 s after the slide).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn damage_part_sounds_from_disc() {
        use crate::damageparts::{DamageParts, PartEvent};
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let mut s = CarSounds::new(&lib, "vp_aud_r8_08", false, true);
        let d = s.damage_obj.clone();
        let f = |o: usize| be_f32(&d, o);
        assert_eq!((f(15), f(23), f(31)), (0.6f32, 1.0f32, 1.0f32));
        assert_eq!(be_u32(&d, 19), joaat("DAMAGE_HOOD_MOVE_MASTER"));
        assert_eq!(be_u32(&d, 35), joaat("DAMAGE_HOOD_IMP_HARD_MASTER"));
        assert_eq!(be_u32(&d, 27), joaat("DAMAGE_HOOD_IMP_LIGHT_MASTER"));
        assert_eq!(be_u32(&d, 55), joaat("DAMAGE_TRUNK_LIGHT_MASTER"));
        assert_eq!((f(71), f(99)), (0.4f32, 0.4f32));
        assert_eq!(be_u32(&d, 75), joaat("DAMAGE_BUMPER_CREAK_MASTER"));
        assert_eq!(be_u32(&d, 95), joaat("DAMAGE_BUMPER_BREAK_OFF"));
        assert_eq!(be_u32(&d, 127), joaat("DAMAGE_BUMPER_IMP_HARD_MASTER"));
        assert_eq!(be_u32(&d, 147), joaat("DAMAGE_BUMPER_SCRAPE_MASTER"));
        assert_eq!([f(131), f(135), f(139), f(143)], [3.0f32, 1.5, 70.0, 2.0]);
        assert_eq!([f(151), f(155), f(159), f(163), f(167), f(171), f(175), f(179)], [5.0f32, 60.0, 0.25, 0.5, 5.0, 100.0, 0.7, 1.5]);
        let mut parts = DamageParts::default();
        parts.events = vec![(1, PartEvent::FlapStop { flap: 0, open: false, rate: 2.0, pos: Vec3::ZERO }), (2, PartEvent::BumperOff { bumper: 1, pos: Vec3::ZERO }), (3, PartEvent::BumperImpact { bumper: 1, speed: 10.0, pos: Vec3::ZERO }), (4, PartEvent::BumperScrape { bumper: 1, speed: 10.0, pos: Vec3::ZERO })];
        s.parts_seen = Some(0);
        s.damage_parts(&mut lib, &parts);
        assert_eq!(s.parts_seen, Some(4));
        assert!(s.scrape[1].0.is_some_and(|h| lib.playing(h)));
        s.clock += 0.1;
        s.damage_parts(&mut lib, &parts);
        assert!(s.scrape[1].0.is_none());
    }
}
