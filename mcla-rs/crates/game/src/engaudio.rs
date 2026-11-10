//! The cars' engine sound as the original makes it (mcCarAudioEntity): the car's `_ENGINE_XFADER` table of recorded
//! loops cross-faded by a slewed, normalised rpm, the gain curve and load volume, the "kick" pitch chirp and amp-mod
//! bump, and the gear-shift and backfire one-shots. Rules and sources: `docs/reference/audio/ENGINE_AUDIO_RULE.md`.
//! Computer racers use their `_O_` objects with the opponent rules (`new_opponent`; VEHICLE_EXTRA_AUDIO_RULE.md
//! section 6); the turbo whine and blow-off read this sound's smoothed rpm and load volume (caraudio.rs).
//!
//! The start (F-44510, `start`) runs the ignition machine of the update (F-10725): the ignition one-shot, the crank,
//! the rev flare and the settle (ENGINE_AUDIO_RULE.md 2.2); a sound made with `new` starts running (state 4).
//! Not ported (open there): the stop states 2 / 3 (F-29108, vt+56: no caller in the port), the layer-B overlay (off on
//! every player car). The reverse whine (F-96287) is caraudio.rs's `reverse_whine`.

use crate::sndobj::{Handle, Req, SoundLib};
use glam::Vec3;
use mcla_formats::audcfg::{db2lin, lin2db};
use mcla_formats::hash::joaat;

fn be_u32(o: &[u8], at: usize) -> u32 {
    o.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).unwrap_or(0)
}
fn be_u16(o: &[u8], at: usize) -> u16 {
    o.get(at..at + 2).map(|b| u16::from_be_bytes(b.try_into().unwrap())).unwrap_or(0)
}
fn be_f32(o: &[u8], at: usize) -> f32 {
    f32::from_bits(be_u32(o, at))
}

/// The start's tune words (F-44510): XF+99 the ignition delay (u16 ms) x 0.001 and XF+105 the start boost (s16 dB x 100)
/// x 0.01, each converted to a float and multiplied in single precision.
fn xf_start(xf: &[u8]) -> (f32, f32) {
    (be_u16(xf, 99) as f32 * 0.001, be_u16(xf, 105) as i16 as f32 * 0.01)
}

/// The start's timer +288 (the delay with the ignition flag's low byte set, else 0) and boost +152 (dB to linear, F-42985).
fn start_values(ign_delay: f32, boost_db: f32, ignition: bool) -> (f32, f32) {
    (if ignition { ign_delay } else { 0.0 }, db2lin(boost_db))
}

/// Linear step toward a target, clamped there (F-90102).
fn approach(p: &mut f32, target: f32, step: f32) {
    approach_fma(p, target, step, 1.0);
}

/// F-90102 (audio2 2026-10-27): `p` steps toward `target` by `a * b` with the multiply and the add fused
/// (the original's fmadds / fnmsubs), clamped at the target. Verified bit for bit against the original (b1 evidence).
fn approach_fma(p: &mut f32, target: f32, a: f32, b: f32) {
    if *p < target {
        let v = a.mul_add(b, *p);
        *p = if v > target { target } else { v };
    } else if *p > target {
        // fnmsubs: -(a * b - p), so a step landing on zero gives -0, which the clamp keeps
        let v = -a.mul_add(b, -*p);
        *p = if v < target { target } else { v };
    }
}

/// What one update of the revs machine (F-74751) does besides moving the start boost: make the revs one-shot (F-60794),
/// play it (F-87666), stop it (the slot's release F-63886), or end the machine (+120 to the empty one).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RevsAct {
    pub make: bool,
    pub play: bool,
    pub stop: bool,
    pub done: bool,
}

/// One update of the revs machine (F-74751) on the start boost +152: `mode` byte 1950, `slot` the one-shot slot's state
/// (0 empty, 1 made, 5 playing, else another), `elapsed` the audio clock since it was played and `dur` its Duration (ms),
/// `dt` the real frame time (s). In the mode the boost moves at 1 a second (F-90102: the fused step, clamped):
/// to 1 when the slot is empty (the one-shot is made) or in another state, not at all when made (it is played), to 0 while
/// it plays and `elapsed < dur - 2000` (unsigned), else to 1. Out of the mode a one-shot in the slot is stopped and the
/// boost steps to 1 (a plain add, clamped); the machine ends when it is 1.
pub(crate) fn revs_machine(boost: &mut f32, mode: bool, slot: u8, elapsed: u32, dur: i32, dt: f32) -> RevsAct {
    let mut a = RevsAct::default();
    if !mode {
        a.stop = slot != 0;
        let v = *boost;
        if v < 1.0 {
            *boost = (v + dt).min(1.0);
        } else if v > 1.0 {
            *boost = (v - dt).max(1.0);
        }
        a.done = *boost == 1.0;
        return a;
    }
    match slot {
        0 => {
            a.make = true;
            approach_fma(boost, 1.0, 1.0, dt);
        }
        1 => a.play = true,
        5 => approach_fma(boost, if elapsed < (dur as u32).wrapping_sub(2000) { 0.0 } else { 1.0 }, 1.0, dt),
        _ => approach_fma(boost, 1.0, 1.0, dt),
    }
    a
}

#[derive(Clone, Copy)]
struct Rec {
    sound: u32,
    start: f32,
    end: f32,
    reference: f32,
}

/// Record `i`'s cross-fade at audio rpm `x`: full between the previous record's end and the next one's start, a
/// quarter sine in from its start, a quarter cosine out to its end.
/// F-98304: how far the rubber band holds the racer's throttle down, 1 - min((cap - MinThrottle) / (1 - MinThrottle), 1)
/// (brain +404, +428).
pub(crate) fn rubber_hold(cap: f32, min_throttle: f32) -> f32 {
    let f = (cap - min_throttle) / (1.0 - min_throttle);
    1.0 - if 1.0 - f >= 0.0 { f } else { 1.0 }
}

/// F-98304's blend target: the rubber band's hold, at most 3 x the speed ratio (this+232; on a tie the speed term).
fn blend_target(hold: f32, speed_ratio: f32) -> f32 {
    let r = speed_ratio * 3.0;
    if hold - r >= 0.0 {
        r
    } else {
        hold
    }
}

fn xfade(recs: &[Rec], i: usize, x: f32) -> f32 {
    let r = recs[i];
    let prev_end = if i == 0 { 1.0 } else { recs[i - 1].end };
    let next_start = if i + 1 == recs.len() { 100_000.0 } else { recs[i + 1].start };
    if x >= prev_end && x <= next_start {
        1.0
    } else if x > r.start && x <= r.end && x < prev_end {
        ((x - r.start) / (prev_end - r.start) * std::f32::consts::FRAC_PI_2).sin()
    } else if x <= r.end && x > next_start {
        ((x - next_start) / (r.end - next_start) * std::f32::consts::FRAC_PI_2).cos()
    } else {
        0.0
    }
}

/// What the engine sound reads from the car each frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct EngineIn {
    pub pos: Vec3,
    /// the player's audio camera mode (entity +1924); the computer racers' stays 2, which is the car's position
    pub view: Option<i32>,
    /// physics engine speed (rpm)
    pub rpm: f32,
    pub idle_rpm: f32,
    pub max_rpm: f32,
    /// engine +16: torque this frame (negative = engine braking); engine +20: full-throttle torque at peak power
    pub torque: f32,
    pub full_torque: f32,
    /// engine +24: pedal after the shift envelope
    pub pedal: f32,
    /// the driver's throttle input (this +188) and the sim throttle (car +136)
    pub throttle_in: f32,
    /// m/s
    pub speed: f32,
    /// the port's gear (-1 reverse, 0 neutral, 1.. forward)
    pub gear: i32,
    /// the transmission's shift state: +1 upshift, -1 downshift, 0 none (trans +24)
    pub shift: i8,
    /// the garage / cutscene rev mode (byte 1950): the loops follow idle
    pub idle_only: bool,
    /// the revs mode as the original sets byte 1950 (F-17772 from the brain's SetRevvingEnabled, F-58174): runs the revs
    /// machine (`idle_only` should be set with it)
    pub revs: bool,
    /// opponents: the car's driver is racing (byte 1951) and its speed over the gearbox's top-gear speed (this+232)
    pub racing: bool,
    pub speed_ratio: f32,
    /// opponents: how far the rubber band holds the throttle down, 1 - min((cap - MinThrottle) / (1 - MinThrottle), 1)
    /// (brain +404 the cap, +428 the brain's tuning copy's MinThrottle); 0 with the rubber band off (brain +380)
    pub rubber_hold: f32,
}

/// The car's audio DEFAULT object: `<CAR>_PROFILE` +10 for the player, +14 for an opponent; VPD_ cars fall back to their
/// VP_ profile, then DEFAULT_CAR / DEFAULT_BIKE_PROFILE. A mod car: its own profile (the mod's game.dat, merged into
/// `lib.game`), then its manifest's sound donor's, then the default (modfs.rs).
pub fn default_object<'a>(lib: &'a SoundLib, car: &str, bike: bool, player: bool) -> Option<&'a [u8]> {
    let g = &lib.game;
    let find = |c: &str| {
        let up = c.to_ascii_uppercase();
        g.get_name(&format!("{up}_PROFILE")).or_else(|| g.get_name(&format!("{}_PROFILE", up.replacen("VPD_", "VP_", 1))))
    };
    let profile = crate::modfs::car_names(crate::catalog::sound_car(car), crate::modfs::Donor::Sound)
        .iter()
        .find_map(|c| find(c))
        .or_else(|| g.get_name(if bike { "DEFAULT_BIKE_PROFILE" } else { "DEFAULT_CAR_PROFILE" }))?;
    g.get(be_u32(profile, if player { 10 } else { 14 }))
}

static NEXT_CHAIN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

pub struct EngineSound {
    /// the car's occluder (entity +96), which every request of the car audio carries at +32 (the loops: F-74444 /
    /// F-51617 store it there): a slot of the sound library's occlusion table plus one, 0 = none
    pub occ: u16,
    recs: Vec<Rec>,
    /// XF+12 cents, XF+14 mB as dB, EFFECT+10 mB as dB
    pitch_off: f32,
    /// the XF table's s16 +14 and the effect object's s16 +10, raw (hundredths of a dB)
    vol_a: f32,
    effect_db: f32,
    /// XF+10 x 0.01: every loop's request +68 (F-74444 / F-51617), dB on the voice's second level
    vol2: f32,
    /// where the loops sit in the two views from inside the car (F-96250, emitter type 1): XF +54 bumper, XF +42 driver
    /// (x, y, z in the camera's axes, z towards the viewer)
    view_place: [Vec3; 2],
    /// the loops' Doppler factor (the voice's +84): 0 in those two views, else back to 1 at 2 a second
    doppler: f32,
    /// XF+18 / +22 / +26 / +30 / +34
    load_brake: f32,
    load_floor: f32,
    load_rate: f32,
    load_slow: f32,
    load_rev: f32,
    slew: f32,
    gain_curve: u32,
    /// the ignition (XF +95 one-shot, +99 delay ms, +101 / +103 crank ms, +105 start boost mB, +107 flare rpm,
    /// +109 flare rate, +111 settle rate)
    ign_sound: u32,
    ign_delay: f32,
    crank: (i32, i32),
    boost_db: f32,
    flare: (f32, f32),
    settle: f32,
    /// the stop (F-29108): XF +113 the one-shot after the stop's wait (IGNITION_KAPUT_MASTER), +117 that wait in ms
    kaput_sound: u32,
    kaput_delay: f32,
    /// the revs machine (F-74751): the revs one-shot ONESHOT +88, its slot (+1464, emptied when it ends: F-30709), its
    /// time since played and Duration in ms (+1468 / +1472; None until it is played), and whether the machine runs (+120)
    revs_sound: u32,
    revs: Option<Handle>,
    revs_t: Option<(f32, i32)>,
    revs_made: bool,
    revs_on: bool,
    // one-shots (ONESHOT_PARAMS)
    gear_sound: u32,
    gear_special: u32,
    gear_vol: f32,
    gear_pitch: i32,
    gear_delay: f32,
    kick_throttle: f32,
    kick_rpm: f32,
    backfire: u32,
    backfire_time: f32,
    // LFO and amp-mod params
    lfo_freq: [f32; 3],
    lfo_freq_var: f32,
    lfo_phase: [f32; 3],
    lfo_amp: (i32, i32),
    lfo_hold: f32,
    lfo_release: f32,
    lfo_delay: f32,
    /// AMPMOD +10 the bump's curve, +14 its length, +22 its amount, +26 the random spread of both (F-75866)
    amp_curve: u32,
    amp_time: (f32, f32),
    amp_base: f32,
    /// the player's car (true) or a computer racer's (`_O_` objects, section 6 of VEHICLE_EXTRA_AUDIO_RULE.md)
    player: bool,
    /// opponents: AMPMOD +30/+34 wobble target range, +38/+42 its rate range
    wobble_target: (f32, f32),
    wobble_rate: (f32, f32),
    /// this frame's audio rpm (this+212) and each loop's gain (fade x G), for tests and the debug overlay
    pub audio_rpm: f32,
    pub gains: Vec<f32>,
    // state
    voices: Vec<Option<Handle>>,
    /// F-10725's state (this+1928: 0 ignition wait, 1 crank, 2 stop wait, 3 stopped, 4 running, 5 flare, 6 settle), its timer (this+288,
    /// seconds), the start boost (this+152) and the ignition one-shot (slot this+1460)
    state: u8,
    timer: f32,
    boost: f32,
    ign: Option<Handle>,
    /// set when a crank ends (the engine counts as running again: chassis engine +46 = 1); taken by `take_running`
    started: bool,
    s: f32,
    load_vol: f32,
    load_smooth: f32,
    prev_torque: f32,
    prev_rpm: f32,
    clock: f32,
    kick_at: f32,
    lfo: Lfo,
    /// the bump: time left (this+1204), its length (+1200) and this kick's amount (+1208)
    amp_left: f32,
    amp_len: f32,
    amp_amount: f32,
    prev_shift: i8,
    prev_gear: i32,
    pending_gear: Option<(f32, u32)>,
    backfire_armed: Option<f32>,
    /// opponents: the RPM_RACE_OPPONENT blend (this+216) and the wobble (this+220 target, +224 rate, +228 value)
    blend: f32,
    wobble: (f32, f32, f32),
    racing_curve: u32,
    /// the car's `VP_*_EFFECT_PARAMS` (entity +1232, the DEFAULT object +30) and this engine's PLAYER_WAVESHAPER
    /// chain (entity +1280: the player's one, an opponent's from the pool of 7), its id in the mixer
    eff: Vec<u8>,
    chain: u32,
    /// the waveshaper drive's input (entity +196, set with the load volume) and the drive (entity +1916)
    ws_in: f32,
    ws_drive: f32,
}

#[derive(Default)]
struct Lfo {
    delay: f32,
    hold: f32,
    release: f32,
    freq: [f32; 3],
    phase: [f32; 3],
    amp: f32,
    running: bool,
}

/// The three oscillators (F-79529): 0 sine, 1 triangle in phase with the sine, 2 square (width 0.5).
fn osc(kind: usize, p: f32) -> f32 {
    match kind {
        0 => (p * std::f32::consts::TAU).sin(),
        1 => {
            let q = if p > 0.5 { p - 0.5 } else { p };
            let v = if (0.0..0.25).contains(&q) { 4.0 * q } else { 4.0 * (0.5 - q) };
            if p > 0.5 {
                -v
            } else {
                v
            }
        }
        _ => {
            if p <= 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
}

impl EngineSound {
    /// The car's audio objects: `<CAR>_PROFILE` -> player `_DEFAULT` -> cross-fader, one-shots, LFO, amp-mod.
    pub fn new(lib: &SoundLib, car: &str, bike: bool) -> Option<EngineSound> {
        Self::make(lib, car, bike, true)
    }

    /// A computer racer's engine: the profile's opponent DEFAULT (+14), `_O_ENGINE_XFADER`, TUNERS_AMPMOD_MAIN_OPPONENTS.
    pub fn new_opponent(lib: &SoundLib, car: &str, bike: bool) -> Option<EngineSound> {
        Self::make(lib, car, bike, false)
    }

    fn make(lib: &SoundLib, car: &str, bike: bool, player: bool) -> Option<EngineSound> {
        let g = &lib.game;
        let def = default_object(lib, car, bike, player)?;
        let xf = g.get(be_u32(def, 14))?;
        let one = g.get(be_u32(def, 18)).unwrap_or(&[]);
        let eff = g.get(be_u32(def, 30)).unwrap_or(&[]);
        let lfo = g.get(be_u32(def, 34)).unwrap_or(&[]);
        let amp = g.get(be_u32(def, 50)).or_else(|| g.get_name("TUNERS_AMPMOD_MAIN")).unwrap_or(&[]);
        let n = *xf.get(131)? as usize;
        let recs: Vec<Rec> = (0..n)
            .map(|i| {
                let o = 132 + 14 * i;
                Rec { sound: be_u32(xf, o), start: be_u16(xf, o + 8) as f32, end: be_u16(xf, o + 10) as f32, reference: be_u16(xf, o + 12) as f32 }
            })
            .collect();
        if recs.is_empty() {
            return None;
        }
        let s16 = |o: &[u8], at: usize| be_u16(o, at) as i16 as f32;
        log::info!("audio: engine {} - {} loops {}..{} rpm", lib.game.name_of(be_u32(def, 14)).unwrap_or("?"), n, recs[0].start, recs[n - 1].end);
        Some(EngineSound {
            occ: 0,
            player,
            wobble_target: (be_f32(amp, 30), be_f32(amp, 34)),
            wobble_rate: (be_f32(amp, 38), be_f32(amp, 42)),
            blend: 0.0,
            wobble: (1.0, 0.0, 1.0),
            racing_curve: joaat("RPM_RACE_OPPONENT"),
            pitch_off: s16(xf, 12),
            vol_a: s16(xf, 14),
            // (MCLA_ENGINE_VOL2_OLD=1: without it, as before #39)
            vol2: if std::env::var_os("MCLA_ENGINE_VOL2_OLD").is_some() { 0.0 } else { s16(xf, 10) * 0.01 },
            effect_db: if eff.is_empty() { 0.0 } else { s16(eff, 10) },
            view_place: [Vec3::new(be_f32(xf, 54), be_f32(xf, 58), be_f32(xf, 62)), Vec3::new(be_f32(xf, 42), be_f32(xf, 46), be_f32(xf, 50))],
            doppler: 1.0,
            load_brake: be_f32(xf, 18),
            load_floor: be_f32(xf, 22),
            load_rate: be_f32(xf, 26),
            load_slow: be_f32(xf, 30),
            load_rev: be_f32(xf, 34),
            slew: (100.0 - be_f32(xf, 38)) * 100.0,
            gain_curve: be_u32(xf, 119),
            ign_sound: be_u32(xf, 95),
            ign_delay: xf_start(xf).0,
            crank: (be_u16(xf, 101) as i32, be_u16(xf, 103) as i32),
            boost_db: xf_start(xf).1,
            flare: (be_u16(xf, 107) as f32, be_u16(xf, 109) as f32),
            settle: be_u16(xf, 111) as f32,
            kaput_sound: be_u32(xf, 113),
            kaput_delay: be_u16(xf, 117) as f32 * 0.001,
            revs_sound: be_u32(one, 88),
            revs: None,
            revs_t: None,
            revs_made: false,
            revs_on: false,
            gear_sound: be_u32(one, 10),
            gear_special: be_u32(one, 14),
            gear_vol: s16(one, 18) * 0.01,
            gear_pitch: s16(one, 20) as i32,
            gear_delay: be_u16(one, 22) as f32 * 0.001,
            kick_throttle: be_f32(one, 24),
            kick_rpm: be_f32(one, 28),
            backfire: be_u32(one, 32),
            backfire_time: be_u16(one, 38) as f32 * 0.01,
            lfo_freq: [be_f32(lfo, 10), be_f32(lfo, 14), be_f32(lfo, 18)],
            lfo_freq_var: be_f32(lfo, 22),
            lfo_phase: [be_f32(lfo, 26), be_f32(lfo, 30), be_f32(lfo, 34)],
            lfo_amp: (s16(lfo, 38) as i32, s16(lfo, 40) as i32),
            lfo_hold: be_f32(lfo, 46) + be_f32(lfo, 42),
            lfo_release: be_f32(lfo, 50),
            lfo_delay: be_f32(lfo, 62) + be_f32(lfo, 66),
            amp_curve: be_u32(amp, 10),
            amp_time: (be_f32(amp, 14), be_f32(amp, 26)),
            amp_base: be_f32(amp, 22),
            audio_rpm: 0.0,
            gains: vec![0.0; n],
            voices: vec![None; n],
            recs,
            state: 4,
            timer: 0.0,
            boost: 1.0,
            ign: None,
            started: false,
            s: 0.0,
            load_vol: 1.0,
            load_smooth: 0.0,
            prev_torque: 0.0,
            prev_rpm: 0.0,
            clock: 0.0,
            kick_at: -10.0,
            lfo: Lfo::default(),
            amp_left: 0.0,
            amp_len: 0.0,
            amp_amount: 0.0,
            prev_shift: 0,
            prev_gear: 0,
            pending_gear: None,
            backfire_armed: None,
            eff: eff.to_vec(),
            chain: if eff.is_empty() { 0 } else { NEXT_CHAIN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) },
            ws_in: 0.0,
            ws_drive: 0.0,
        })
    }

    /// The entity's constructor (F-43725) sets the rpm +200, the slewed rpm +204 and the previous rpm +208 to the car's
    /// engine rpm (F-12794) when it is made: a racer handed another car starts at its engine's rpm, not at 0.
    pub fn start_at(&mut self, rpm: f32) {
        self.s = rpm;
        self.prev_rpm = rpm;
    }

    /// The smoothed engine speed (this+204) and the load volume (this+148): the turbo whine reads both.
    pub fn smoothed_rpm(&self) -> f32 {
        self.s
    }
    pub fn load_volume(&self) -> f32 {
        self.load_vol
    }

    pub fn stop(&mut self, lib: &mut SoundLib) {
        self.stop_loops(lib);
        if let Some(h) = self.ign.take() {
            lib.stop(h);
        }
    }

    /// vt+112: every loop voice stops.
    fn stop_loops(&mut self, lib: &mut SoundLib) {
        for h in self.voices.iter_mut().filter_map(|h| h.take()) {
            lib.stop(h);
        }
    }

    /// The engine sound's start (F-44510, vt+52): state 0, the loops stop, the ignition after XF+99 ms when
    /// `ignition` is set (else on the next update), the start boost XF+105 (+6 dB on every car) that falls back to
    /// 0 dB at 1 per second once the engine runs. CopArrest's Racer_PlayIgnition and the scenes' car_on call it with 0.
    pub fn start(&mut self, lib: &mut SoundLib, ignition: bool) {
        self.state = 0;
        self.stop_loops(lib);
        (self.timer, self.boost) = start_values(self.ign_delay, self.boost_db, ignition);
    }

    /// The engine sound's stop (F-29108, vt+56): state 2, the loops stop, the wait XF+117 ms (2250 on every car) when
    /// `delay` is set (else 0), the start boost as in `start`. Once the wait has run out the slot's one-shot stops,
    /// XF+113 (IGNITION_KAPUT_MASTER) plays and the state is 3: silent until the next `start`. The driver's
    /// "impact start car" (F-80396) calls it with 1 after a damage-out.
    pub fn stop_engine(&mut self, lib: &mut SoundLib, delay: bool) {
        self.state = 2;
        self.stop_loops(lib);
        self.timer = if delay { self.kaput_delay } else { 0.0 };
        self.boost = db2lin(self.boost_db);
    }

    /// The engine sound set running at once (F-79364, vt+48): state 4 and the timer +288 = 0, no crank, no flare. The
    /// racer's restore F-39432 (the QUICK FIX, quickFixCar / quickFixResetCar) calls it; a new car's engine sound
    /// starts in state 4 too (its constructor).
    pub fn set_running(&mut self) {
        self.state = 4;
        self.timer = 0.0;
    }

    /// Whether the start machine holds the car (states 0 .. 3: the start's wait, the crank, the stop's wait, stopped):
    /// the control step F-17997 reads this+1928 < 4.
    pub fn held(&self) -> bool {
        self.state < 4
    }

    /// Whether the engine sound is stopped or waiting to stop (states 2 and 3).
    pub fn stopped(&self) -> bool {
        matches!(self.state, 2 | 3)
    }

    /// True once after a crank has ended: the engine counts as running again (F-10725 sets chassis engine +46 = 1).
    pub fn take_running(&mut self) -> bool {
        std::mem::take(&mut self.started)
    }

    /// Whether the ignition machine is between its start and the end of the crank (states 0 and 1).
    pub fn cranking(&self) -> bool {
        self.state <= 1
    }

    pub fn update(&mut self, lib: &mut SoundLib, e: &EngineIn, dt: f32) {
        self.clock += dt;
        // inputs (F-36837)
        let load = if e.full_torque > 0.0 { e.torque / e.full_torque } else { 0.0 };
        approach_fma(&mut self.load_smooth, load, 6.0, dt);
        let load_abs = load.abs().min(1.0);
        self.step_load_vol(e, load_abs, dt);
        // the kick: throttle re-applied (F-12996)
        let first = self.recs[0];
        let last = self.recs[self.recs.len() - 1];
        let audio_rpm_prev = {
            let norm = ((self.s - e.idle_rpm) / (e.max_rpm - e.idle_rpm).max(1.0)).min(1.0);
            norm * (last.end - first.start) + first.start
        };
        if e.torque > 0.0 && self.prev_torque <= 0.0 && e.throttle_in >= self.kick_throttle && audio_rpm_prev >= self.kick_rpm && e.speed >= 10.0 {
            if self.clock >= self.kick_at + 1.0 {
                self.kick(lib);
            }
            self.kick_at = self.clock;
        }
        if e.torque < 0.0 {
            self.lfo.running = false;
        }
        let lfo_cents = self.step_lfo(dt);
        self.step_revs(lib, e, dt);
        self.step_loops(lib, e, dt, lfo_cents);
    }

    /// The load volume (entity +148; player F-42554, opponent F-57529) and the waveshaper drive's input (+196) set by
    /// each branch. `load_abs` is |torque / full torque| clamped to 1 (entity +168).
    fn step_load_vol(&mut self, e: &EngineIn, load_abs: f32, dt: f32) {
        let reversing = e.gear == -1 && e.speed > 1.0;
        if reversing {
            approach_fma(&mut self.load_vol, self.load_rev, 0.8, dt);
            self.ws_in = e.throttle_in;
        } else if !self.player {
            let drive = load_abs.max(e.pedal).max(self.load_floor).max(self.load_brake);
            let full = if e.racing { 1.0 } else { drive };
            let (target, ws) = if self.load_smooth > 0.0 {
                (full, full)
            } else if e.speed < self.load_slow {
                (1.0, full)
            } else if e.pedal < 0.5 {
                (if e.racing { 1.0 } else { self.load_floor }, if e.racing { 1.0 } else { load_abs })
            } else if e.shift != 0 {
                (self.load_brake, load_abs)
            } else {
                (full, full)
            };
            approach_fma(&mut self.load_vol, target, self.load_rate, dt);
            self.ws_in = ws;
        } else {
            let drive = load_abs.max(e.throttle_in).max(e.pedal).max(self.load_floor).max(self.load_brake);
            if self.load_smooth > 0.0 {
                approach_fma(&mut self.load_vol, drive, self.load_rate, dt);
                self.ws_in = drive;
            } else if e.speed < self.load_slow {
                approach(&mut self.load_vol, 1.0, dt);
                self.ws_in = drive;
            } else if e.throttle_in < 0.5 {
                // (lifted: toward XF+22, the shift toward XF+18; recorded, work/audio2/cap2/eng6.txt)
                approach_fma(&mut self.load_vol, self.load_floor, self.load_rate, dt);
                self.ws_in = load_abs;
            } else if e.shift != 0 {
                approach_fma(&mut self.load_vol, self.load_brake, self.load_rate, dt);
                self.ws_in = load_abs;
            } else {
                approach_fma(&mut self.load_vol, drive, self.load_rate, dt);
                self.ws_in = drive;
            }
        }
    }

    /// The revs machine (F-74751), set going by the revs mode (F-17772). In the mode: with its slot empty the revs one-shot
    /// is made (F-60794) and the start boost +152 goes to 1; once made it is played (F-87666) with its start time and
    /// Duration taken; while it plays the boost goes to 0, back to 1 in its last 2 s (time < Duration - 2000, unsigned);
    /// otherwise to 1. Its slot empties when it ends, so it is made again. Out of the mode: the one-shot is stopped and the
    /// boost goes back to 1, then the machine ends. Steps of 1 a second.
    fn step_revs(&mut self, lib: &mut SoundLib, e: &EngineIn, dt: f32) {
        if e.revs {
            self.revs_on = true;
        }
        if !self.revs_on {
            return;
        }
        // the slot empties when the one-shot ends (F-30709)
        if self.revs.is_some_and(|h| !lib.playing(h)) {
            self.revs = None;
            self.revs_t = None;
        }
        let slot = match (self.revs, self.revs_made) {
            (Some(_), _) => 5,
            (None, true) => 1,
            (None, false) => 0,
        };
        let (elapsed, dur) = self.revs_t.map_or((0, -1), |(t, d)| ((t + dt * 1000.0) as u32, d));
        let a = revs_machine(&mut self.boost, e.revs, slot, elapsed, dur, dt);
        if a.stop {
            if let Some(h) = self.revs.take() {
                lib.stop(h);
            }
            self.revs_t = None;
            self.revs_made = false;
        }
        if a.done {
            self.revs_on = false;
        }
        if a.make {
            self.revs_made = true;
        } else if a.play {
            // its Duration taken before its first step (the total with +80 / +84 still 0)
            self.revs_made = false;
            self.revs = lib.play_hash(self.revs_sound, Req { pos: Some(e.pos), player: self.player, occluder: self.occ, ..Default::default() });
            self.revs_t = self.revs.map(|h| (0.0, lib.duration_ms(h)));
        } else if let Some((t, d)) = self.revs_t {
            self.revs_t = Some((t + dt * 1000.0, d));
        }
    }

    /// The rest of the update: the rpm slew, the start machine and the loops (F-10725).
    fn step_loops(&mut self, lib: &mut SoundLib, e: &EngineIn, dt: f32, lfo_cents: f32) {
        let first = self.recs[0];
        let last = self.recs[self.recs.len() - 1];
        // rpm slew and the start machine (F-10725)
        let target = if e.idle_only { e.idle_rpm } else { e.rpm };
        let rpm_rate = (target - self.s) / dt.max(1e-6);
        if self.state <= 1 {
            // 0 the wait, 1 the crank: the revs fall to 0, no loop sounds; the step that ends either returns too
            self.timer -= dt;
            approach_fma(&mut self.s, 0.0, self.slew, dt);
            if self.timer < 0.0 {
                if let Some(h) = self.ign.take() {
                    lib.stop(h);
                }
                if self.state == 0 {
                    self.ign = lib.play_hash(self.ign_sound, Req { pos: Some(e.pos), player: self.player, occluder: self.occ, ..Default::default() });
                    self.state = 1;
                    self.timer = lib.rand_int(self.crank.0, self.crank.1) as f32 * 0.001;
                    log::info!("engine: ignition at {:.3} s (one-shot {}), crank {:.3} s", self.clock, self.ign.is_some(), self.timer);
                } else {
                    self.state = 5;
                    self.started = true;
                    log::info!("engine: crank over at {:.3} s", self.clock);
                }
            }
            self.end_frame(lib, e, dt);
            return;
        }
        if self.state == 2 {
            // the stop's wait: the revs fall to 0, no loop sounds; at its end the kaput one-shot, then state 3
            self.timer -= dt;
            approach_fma(&mut self.s, 0.0, self.slew, dt);
            if self.timer < 0.0 {
                if let Some(h) = self.ign.take() {
                    lib.stop(h);
                }
                self.ign = lib.play_hash(self.kaput_sound, Req { pos: Some(e.pos), player: self.player, occluder: self.occ, ..Default::default() });
                self.state = 3;
            }
            self.end_frame(lib, e, dt);
            return;
        }
        if self.state == 3 {
            // stopped (vt+112 every update)
            self.stop_loops(lib);
            self.end_frame(lib, e, dt);
            return;
        }
        match self.state {
            // the flare to XF+107 at XF+109 rpm/s, then the settle down to the target at XF+111
            5 => {
                approach_fma(&mut self.s, self.flare.0, self.flare.1, dt);
                if self.s == self.flare.0 {
                    self.state = 6;
                }
            }
            6 => {
                if self.s > target {
                    approach_fma(&mut self.s, target, self.settle, dt);
                    if self.s == target {
                        self.state = 4;
                    }
                } else {
                    self.state = 4;
                }
            }
            _ => {
                approach_fma(&mut self.s, target, self.slew, dt);
                if !e.idle_only {
                    approach_fma(&mut self.boost, 1.0, 1.0, dt);
                }
            }
        }
        // (below 1 rpm every loop stops and nothing more is played this update)
        if self.s < 1.0 {
            self.stop_loops(lib);
            self.end_frame(lib, e, dt);
            return;
        }
        // normalised rpm, audio rpm, master gain
        let mut norm = ((self.s - e.idle_rpm) / (e.max_rpm - e.idle_rpm).max(1.0)).min(1.0);
        if !self.player {
            // the RPM_RACE_OPPONENT blend (F-98304: the rubber band's hold, at most 3 x the speed ratio) and the wobble
            // (F-10725 @A-74853)
            // (not racing: 0 at once)
            if e.racing {
                let want = blend_target(e.rubber_hold, e.speed_ratio);
                let rate = if want < self.blend { 0.125 } else { 0.25 };
                approach_fma(&mut self.blend, want, rate, dt);
            } else {
                self.blend = 0.0;
            }
            if e.racing {
                let c = lib.curve_eval(self.racing_curve, norm).unwrap_or(norm);
                norm += (c - norm) * self.blend;
                let (target, rate, mut w) = self.wobble;
                approach_fma(&mut w, target, rate, dt);
                if w == target {
                    self.wobble.0 = lib.rand_float(self.wobble_target.0, self.wobble_target.1);
                    self.wobble.1 = lib.rand_float(self.wobble_rate.0, self.wobble_rate.1);
                }
                self.wobble.2 = w;
                norm = (norm * (1.0 + (w - 1.0) * self.blend)).min(1.0);
            }
        }
        let x = norm * (last.end - first.start) + first.start;
        let mut g = lib.curve_eval(self.gain_curve, norm).unwrap_or(1.0) * self.boost * self.load_vol;
        if self.amp_left > 0.0 {
            let c = lib.curve_eval(self.amp_curve, self.amp_left / self.amp_len.max(1e-6)).unwrap_or(0.0);
            g = (g + c * self.amp_amount).max(0.0);
            self.amp_left -= dt;
        }
        self.step_doppler(e.view, dt);
        let loop_pos = self.place(lib.listener, e.view, e.pos);
        // per-record cross-fade and pitch
        let n = self.recs.len();
        for i in 0..n {
            let r = self.recs[i];
            let fade = xfade(&self.recs, i, x);
            self.gains[i] = fade * g;
            if !self.player {
                // opponents (F-51617): record 0 and the sounding records live, and the neighbour the revs are
                // moving towards fast
                let prev_start = if i == 0 { f32::MIN } else { self.recs[i - 1].start };
                let next_end = if i + 1 == n { f32::MAX } else { self.recs[i + 1].end };
                let active = fade > 0.0 || (rpm_rate < -2000.0 && prev_start < x && x < r.start) || (rpm_rate > 2000.0 && r.end < x && x < next_end);
                if !active && i != 0 {
                    if let Some(h) = self.voices[i].take() {
                        lib.stop(h);
                    }
                    continue;
                }
            }
            // F-74444: every record's voice runs, silent ones at -100 dB
            // (F-74444 / F-51617: dB = fmadds(xf14, 0.01, fmadds(effect10, 0.01, lin2db(gain))) - the order recorded in the
            // original is only fixed for zero offsets; the asm fuses both)
            let vol = crate::sndobj::fmadds(self.vol_a, 0.01, crate::sndobj::fmadds(self.effect_db, 0.01, lin2db(fade * g)));
            // (F-74444: cents = trunc(offset + float(log10 ratio) * 3.321928 * 1200); recorded in the running original)
            let cents = (self.pitch_off + crate::caraudio::pitch_cents_f32(x / r.reference) + lfo_cents) as i32;
            // F-74444 / F-17698: dry EFFECT_PARAMS +16, chain send +12; F-89149: record 0 pans the chain
            let (dry, fx_send) = if self.eff.is_empty() { (1.0, 0.0) } else { (be_f32(&self.eff, 16), be_f32(&self.eff, 12)) };
            // the loops' category (create params +28): the player's PLAYER_VEHICLES_ENGINES (entity +1696, set by F-30010),
            // a computer racer's OPPONENT_VEHICLES_ENGINE (F-51617); recorded in the running original (category index 128 / 25)
            let category = joaat(if self.player { "PLAYER_VEHICLES_ENGINES" } else { "OPPONENT_VEHICLES_ENGINE" });
            let req = Req { pos: Some(loop_pos), volume: vol, vol2: self.vol2, pitch: cents, doppler: self.doppler, player: self.player, dry, fx_send, chain: self.chain, pans_chain: i == 0, category, occluder: self.occ, ..Default::default() };
            match self.voices[i] {
                Some(h) if lib.playing(h) => lib.set(h, req),
                _ => self.voices[i] = lib.play_hash(r.sound, req),
            }
        }
        self.audio_rpm = x;
        self.end_frame(lib, e, dt);
    }

    /// The PLAYER_WAVESHAPER chain's settings this frame (F-65630, BUS_GRAPH_RULE.md 3.2), from EFFECT_PARAMS and the
    /// audio rpm (entity +212).
    fn step_chain(&mut self, lib: &mut SoundLib, e: &EngineIn, dt: f32) {
        if self.chain == 0 {
            return;
        }
        let o = &self.eff;
        let f = |a: usize| be_f32(o, a);
        let s16 = |a: usize| be_u16(o, a) as i16 as f32;
        let map = |c: usize, lo: f32, hi: f32, x0: f32, x1: f32, x: f32| crate::caraudio::map(lib, be_u32(o, c), lo, hi, x0, x1, x);
        let flags = be_u32(o, 5);
        let on = |b: u32| if matches!(b & 3, 1 | 2) { 1 } else { 0 };
        let rpm = self.audio_rpm;
        let g = ((rpm - f(76)) / (f(80) - f(76))).max(0.0).min(1.0);
        let (target, rate) = if e.torque >= 0.0 {
            (g * map(60, f(36), f(40), f(28), f(32), self.ws_in), f(64))
        } else {
            (g * map(60, f(52), f(56), f(44), f(48), self.ws_in), f(68))
        };
        let mut drive = self.ws_drive;
        approach_fma(&mut drive, target, rate, dt);
        let curve = lib.curve(be_u32(o, 24)).map(|c| (be_u32(o, 24), c.clone()));
        let formant = |b: usize| {
            let freq = map(b + 17, f(b + 1), f(b + 5), f(b + 9), f(b + 13), rpm);
            let q = map(b + 37, f(b + 21), f(b + 25), f(b + 29), f(b + 33), rpm);
            let gain = map(b + 53, s16(b + 41), s16(b + 43), f(b + 45), f(b + 49), rpm) * 0.01;
            (freq, gain, freq / q.clamp(0.5, 100.0), o.get(b).map_or(0, |x| x & 1))
        };
        let p = crate::audio::ChainParams {
            ws: (s16(20) * 0.01, s16(22) * 0.01, drive, curve, on(flags)),
            eq: [(f(88), s16(96) * 0.01, f(92), on(flags >> 2)), formant(98), formant(155), formant(212)],
        };
        self.ws_drive = drive;
        lib.set_chain(self.chain, p);
    }

    /// The loops' place and Doppler factor this frame (F-96250, emitter type 1). In the bumper and driver views they
    /// stay a fixed offset from the listener, whatever the car does; in every other view they are at the car.
    pub fn place(&self, listener: (Vec3, Vec3, Vec3), view: Option<i32>, car: Vec3) -> Vec3 {
        match view {
            Some(v @ 0..=1) => {
                let (p, right, fwd) = listener;
                let o = self.view_place[v as usize];
                p + right * o.x + right.cross(fwd) * o.y - fwd * o.z
            }
            _ => car,
        }
    }

    pub fn doppler(&self) -> f32 {
        self.doppler
    }

    fn step_doppler(&mut self, view: Option<i32>, dt: f32) {
        if matches!(view, Some(0..=1)) {
            self.doppler = 0.0;
        } else {
            approach_fma(&mut self.doppler, 1.0, 2.0, dt);
        }
    }

    /// The one-shots of the frame (gear shift, backfire) and the inputs kept for the next one: their own functions in
    /// the original, run whatever the start machine's state.
    fn end_frame(&mut self, lib: &mut SoundLib, e: &EngineIn, dt: f32) {
        self.step_chain(lib, e, dt);
        // gear-shift one-shot (F-33101 / F-40912): on the shift's start; upshift a little later
        if e.shift != 0 && self.prev_shift == 0 && e.gear != self.prev_gear {
            // original gear indices: reverse 0, neutral 1 - none into those
            let (old, new) = (self.prev_gear + 1, e.gear + 1);
            if new >= 2 {
                let snd = if (new - old).abs() > 1 { self.gear_special } else { self.gear_sound };
                let delay = if e.shift > 0 { self.gear_delay } else { 0.0 };
                self.pending_gear = Some((delay, snd));
            }
        }
        if e.shift != 0 || e.gear != self.prev_gear {
            self.prev_gear = e.gear;
        }
        if let Some((t, snd)) = self.pending_gear {
            if t <= 0.0 {
                lib.play_hash(snd, Req { pos: Some(e.pos), volume: self.gear_vol, pitch: self.gear_pitch, player: self.player, occluder: self.occ, ..Default::default() });
                self.pending_gear = None;
            } else {
                self.pending_gear = Some((t - dt, snd));
            }
        }
        self.prev_shift = e.shift;
        // backfire (F-91044): armed at full throttle, fires after 0.1 s of closed throttle with the revs falling
        if e.throttle_in >= 1.0 {
            self.backfire_armed = Some(self.backfire_time);
        } else if let Some(t) = self.backfire_armed {
            if e.throttle_in <= 0.0 && e.rpm < self.prev_rpm {
                let t = t - dt;
                if t <= 0.0 {
                    lib.play_hash(self.backfire, Req { player: self.player, occluder: self.occ, ..Req::at(e.pos) });
                    self.backfire_armed = None;
                } else {
                    self.backfire_armed = Some(t);
                }
            }
        }
        self.prev_torque = e.torque;
        self.prev_rpm = e.rpm;
    }

    /// The kick event (F-75866): restart LFO 1 with randomised rates and depth, and the amp-mod bump.
    fn kick(&mut self, lib: &mut SoundLib) {
        // (one draw for the three rates)
        let v = self.lfo_freq_var;
        let r = lib.rand_float(-v, v);
        for k in 0..3 {
            self.lfo.freq[k] = (self.lfo_freq[k] + r).max(0.0);
            self.lfo.phase[k] = self.lfo_phase[k];
        }
        self.lfo.amp = (self.lfo_amp.0 + lib.rand_int(-self.lfo_amp.1, self.lfo_amp.1)).max(0) as f32;
        self.lfo.delay = self.lfo_delay.max(0.0);
        self.lfo.hold = 0.0;
        self.lfo.release = 0.0;
        self.lfo.running = true;
        // (the amount AMPMOD +22 +- 26 drawn first; a bump only when it is above 0, its length +14 +- 26)
        let v = self.amp_time.1;
        self.amp_amount = self.amp_base + lib.rand_float(-v, v);
        if self.amp_amount > 0.0 {
            self.amp_len = (self.amp_time.0 + lib.rand_float(-v, v)).max(0.0);
            self.amp_left = self.amp_len;
        } else {
            self.amp_left = 0.0;
        }
    }

    /// LFO 1 (F-12996 @A-46053): delay, hold with the oscillators running, release; cents, upward only.
    fn step_lfo(&mut self, dt: f32) -> f32 {
        let l = &mut self.lfo;
        if !l.running {
            return 0.0;
        }
        if l.delay > 0.0 {
            l.delay -= dt;
            if l.delay <= 0.0 {
                l.hold = self.lfo_hold;
                l.phase = self.lfo_phase;
            }
            return 0.0;
        }
        let mut scale = 1.0;
        if l.hold > 0.0 {
            l.hold -= dt;
            if l.hold <= 0.0 {
                l.release = self.lfo_release;
            }
        } else if l.release > 0.0 {
            l.release -= dt;
            // DEFAULT_RELEASE_CURVE (0,1)->(1,0) of the time spent releasing
            scale = (l.release / self.lfo_release.max(1e-6)).clamp(0.0, 1.0);
            if l.release <= 0.0 {
                l.running = false;
                return 0.0;
            }
        } else {
            l.running = false;
            return 0.0;
        }
        let v = (0..3).map(|k| osc(k, l.phase[k])).sum::<f32>() / 3.0;
        for k in 0..3 {
            l.phase[k] = (l.phase[k] + dt * l.freq[k]).fract();
        }
        l.amp * v.max(0.0) * scale
    }
}

#[cfg(test)]
mod tests {

    /// The engine sound's start F-44510 against the original's code run offline (tools/engine_start_reference.py: XF+99 and
    /// XF+105 over 3,868 values each, the ignition flag 0 and 1): state +1928 0, the loops stopped once, timer +288 and
    /// boost +152 equal to the bit in every case (the boost through the shared db2lin, F-42985).
    #[test]
    fn engine_start_matches_the_original() {
        use super::*;
        let mut n = 0;
        for line in include_str!("testdata/engine_start_ref.txt").lines() {
            let w: Vec<&str> = line.split_whitespace().filter(|w| *w != "->").collect();
            let h = |i: usize| u32::from_str_radix(w[i], 16).unwrap();
            let mut xf = vec![0xABu8; 256];
            xf[99..101].copy_from_slice(&(h(1) as u16).to_be_bytes());
            xf[105..107].copy_from_slice(&(h(2) as u16).to_be_bytes());
            let (d, b) = xf_start(&xf);
            let (t, g) = start_values(d, b, w[0] == "1");
            assert_eq!((h(3), t.to_bits(), g.to_bits(), w[6]), (0, h(4), h(5), "1"), "{line}");
            n += 1;
        }
        assert_eq!(n, 7736);
    }

    /// The revs machine F-74751 against the original's code run offline on random update sequences
    /// (tools/revs_machine_reference.py): every update's boost bits and its make / play / stop / end
    #[test]
    fn revs_machine_matches_the_original() {
        use super::*;
        let mut n = 0;
        for line in include_str!("testdata/revs_machine_ref.txt").lines().filter(|l| l.contains("->")) {
            let v: Vec<u64> = line.split_whitespace().filter(|w| *w != "->").map(|w| w.parse::<i64>().unwrap() as u64).collect();
            let mut b = f32::from_bits(v[5] as u32);
            let a = revs_machine(&mut b, v[0] != 0, v[1] as u8, v[2] as u32, v[3] as i32, f32::from_bits(v[4] as u32));
            assert_eq!((b.to_bits(), a), (v[6] as u32, RevsAct { make: v[7] != 0, play: v[8] != 0, stop: v[9] != 0, done: v[10] != 0 }), "{line}");
            n += 1;
        }
        assert_eq!(n, 13070);
    }

    /// F-96250's emitter type 1: in the two inside views the loops are straight ahead of the listener at XF +54 / +42
    /// wherever the car is, in the others at the car; the Doppler factor 0 inside, back at 2 a second outside.
    #[test]
    fn loops_stay_ahead_of_the_listener_inside_the_car() {
        let dir = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let Some(a) = crate::audio::Audio::new_offline(&dir) else { return };
        let Some(lib) = SoundLib::load(a.root(), &a.cfg.clone()) else { return };
        let mut e = EngineSound::make(&lib, "vp_aud_r8_08", false, true).expect("engine");
        let (p, r) = (Vec3::new(5.0, 2.0, 7.0), Vec3::X);
        let f = Vec3::new(0.0, -0.6, -0.8);
        let car = Vec3::new(40.0, 0.0, 9.0);
        // in the listener's axes as `emit` takes them
        let (u, side) = (r.cross(f), |q: Vec3| (q - p).dot(r));
        for (view, ahead) in [(0, 1.0), (1, 2.0)] {
            let q = e.place((p, r, f), Some(view), car);
            assert!(side(q).abs() < 1e-5 && (q - p).dot(u).abs() < 1e-5 && ((q - p).dot(f) - ahead).abs() < 1e-5, "view {view}: {q}");
            e.step_doppler(Some(view), 0.1);
            assert_eq!(e.doppler(), 0.0);
        }
        assert_eq!(e.place((p, r, f), Some(3), car), car);
        assert_eq!(e.place((p, r, f), None, car), car);
        e.step_doppler(Some(3), 0.1);
        assert!((e.doppler() - 0.2).abs() < 1e-6);
    }

    /// F-51617's voice request (cents and dB handed to the voice) for the computer racers' engines, against pairs
    /// recorded in the running original in a street race (work/audio2/cap/eng2.txt, OEN / OEP / OEV lines): offsets XF+12
    /// 100..300 and effect+10 0 / -300 occur, XF+14 is 0
    #[test]
    fn opponent_voice_request_recorded() {
        let near = near_log2;
        let h = |x: &str| u32::from_str_radix(&x.replace('_', ""), 16).unwrap();
        let mut bad = 0;
        for line in include_str!("opp_voice_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<&str> = i.split(' ').collect();
            let w: Vec<&str> = o.split(' ').collect();
            let (pitch, gain) = (f32::from_bits(h(v[0])), f32::from_bits(h(v[1])));
            let (xf12, fx): (f32, f32) = (v[2].parse().unwrap(), v[3].parse().unwrap());
            let cents = (xf12 + crate::caraudio::pitch_cents_f32(pitch) + 0.0) as i32;
            let db = crate::sndobj::fmadds(0.0, 0.01, crate::sndobj::fmadds(fx, 0.01, lin2db(gain)));
            if cents != w[0].parse::<i32>().unwrap() || !w[1..].iter().any(|x| near(h(x), db)) {
                bad += 1;
            }
        }
        assert_eq!(bad, 0);
    }

    /// F-74444's voice request (layer A: cents and dB handed to the voice) against 7899 distinct (pitch, gain) pairs
    /// recorded in the running original (work/audio2/cap/eng1.txt, recomp-trace MCLA_SST_AUDIO ENG / EP / EV lines; R8,
    /// offsets XF+12, XF+14 and the effect's +10 all 0 there)
    #[test]
    fn engine_voice_request_recorded() {
        let near = near_log2;
        let h = |x: &str| u32::from_str_radix(&x.replace('_', ""), 16).unwrap();
        let mut bad = 0;
        for line in include_str!("eng_voice_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<&str> = i.split(' ').collect();
            let w: Vec<&str> = o.split(' ').collect();
            let (pitch, gain) = (f32::from_bits(h(v[0])), f32::from_bits(h(v[1])));
            let cents = (0.0 + crate::caraudio::pitch_cents_f32(pitch) + 0.0) as i32;
            let db = crate::sndobj::fmadds(0.0, 0.01, crate::sndobj::fmadds(0.0, 0.01, lin2db(gain)));
            if cents != w[0].parse::<i32>().unwrap() || !near(h(w[1]), db) {
                bad += 1;
            }
        }
        assert_eq!(bad, 0);
    }

    /// GitHub #33: the engine loops are in the request's category (PLAYER_VEHICLES_ENGINES for the player, as the running
    /// original gives them), so the player vehicle slider (VOLUME_RULE.md 1.1) reaches them: at 0 the 350Z's loops
    /// render below -80 dB of their level at 10; the music slider leaves them as they are.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn player_vehicle_slider_reaches_the_engine() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let rms = |sliders: [u8; 5]| {
            let mut a = crate::audio::Audio::new_offline(&dir).expect("audio");
            let mut lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
            lib.listener = (Vec3::new(0.0, 1.5, 6.0), Vec3::X, Vec3::NEG_Z);
            lib.set_sliders(sliders);
            let mut e = EngineSound::new(&lib, "vp_nsn_350z_06", false).expect("engine");
            let dt = 1.0 / 60.0;
            for f in 0..240 {
                let inp = EngineIn { pos: Vec3::ZERO, rpm: 900.0 + 20.0 * f as f32, idle_rpm: 900.0, max_rpm: 7000.0, torque: 300.0, full_torque: 400.0, pedal: 1.0, throttle_in: 1.0, speed: 20.0, gear: 3, view: Some(2), ..Default::default() };
                e.update(&mut lib, &inp, dt);
                lib.update(dt, &mut a);
                a.end_frame();
                a.render(dt);
            }
            let c = a.captured();
            (c.iter().map(|x| x * x).sum::<f32>() / c.len().max(1) as f32).sqrt()
        };
        let (on, off, music) = (rms([10; 5]), rms([10, 10, 10, 0, 10]), rms([10, 0, 10, 10, 10]));
        assert!(on > 1e-3, "{on}");
        assert!(off < on * 1e-4, "{off} against {on}");
        assert_eq!(music, on);
    }

    /// The loops carry the car's occluder (F-74444 stores entity +96 into request +32): with the slot's outputs at the
    /// fully occluded pair of the disc's GLOBAL_OCCLUSION (volume 0.2, 4002 Hz) the engine is quieter than with the
    /// open pair (1, 23900), the same as with no occluder. Needs MCLA_DATA_DIR (extracted\cache).
    #[test]
    #[ignore]
    fn occluder_reaches_the_engine_loops() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let rms = |occ: Option<[f32; 2]>| {
            let mut a = crate::audio::Audio::new_offline(&dir).expect("audio");
            let mut lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
            lib.listener = (Vec3::new(0.0, 1.5, 6.0), Vec3::X, Vec3::NEG_Z);
            let mut e = EngineSound::new(&lib, "vp_nsn_350z_06", false).expect("engine");
            if let Some(o) = occ {
                lib.set_occlusion(3, o);
                e.occ = 4;
            }
            let dt = 1.0 / 60.0;
            for f in 0..240 {
                let inp = EngineIn { pos: Vec3::ZERO, rpm: 900.0 + 20.0 * f as f32, idle_rpm: 900.0, max_rpm: 7000.0, torque: 300.0, full_torque: 400.0, pedal: 1.0, throttle_in: 1.0, speed: 20.0, gear: 3, view: Some(2), ..Default::default() };
                e.update(&mut lib, &inp, dt);
                lib.update(dt, &mut a);
                a.end_frame();
                a.render(dt);
            }
            let c = a.captured();
            (c.iter().map(|x| x * x).sum::<f32>() / c.len().max(1) as f32).sqrt()
        };
        let (none, open, shut) = (rms(None), rms(Some([1.0, 23900.0])), rms(Some([0.2, 4002.0])));
        assert!(none > 1e-3, "{none}");
        assert_eq!(open, none);
        assert!(shut < open * 0.2, "{shut} against {open}");
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

    /// The recorder's host `log2f` stands in for the console's log instruction and is not correctly rounded for a few
    /// inputs (and differs between platforms); the port takes the correctly rounded value, so a recorded dB is accepted
    /// within 4 ulp of it (the same split as audcfg's `curve3_log2_ambiguous_cases`).
    fn near_log2(want: u32, got: f32) -> bool {
        (want as i64 - got.to_bits() as i64).abs() <= 4
    }

    /// The Camaro's loops over an rpm sweep, rendered alone (`MCLA_ENGINE_WAV` = where to write the mix).
    /// The cut GT-R sounds like the 350Z (catalog::sound_car), not its own profile's R8.
    #[test]
    fn cut_gtr_uses_the_350z_engine() {
        let dir = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let Some(a) = crate::audio::Audio::new_offline(&dir) else { return };
        let Some(lib) = SoundLib::load(a.root(), &a.cfg.clone()) else { return };
        for player in [true, false] {
            let gtr = default_object(&lib, crate::catalog::CUT_GTR, false, player).expect("gt-r");
            assert_eq!(Some(gtr), default_object(&lib, "vp_nsn_350z_06", false, player));
            assert_ne!(Some(gtr), default_object(&lib, "vp_aud_r8_08", false, player));
        }
    }

    /// Every loop asks for XF+10 x 0.01 dB on its second level (F-74444 / F-51617 -> request +68): the 240SX +8 dB, its
    /// opponent's +8 dB, the Camaro +9 dB.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn loops_ask_for_the_xfaders_second_level() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
        assert_eq!(EngineSound::new(&lib, "vp_nsn_240sx_98", false).expect("240sx").vol2, 8.0);
        assert_eq!(EngineSound::new_opponent(&lib, "vp_nsn_240sx_98", false).expect("240sx o").vol2, 8.0);
        assert_eq!(EngineSound::new(&lib, "vp_chv_camaro_06", false).expect("camaro").vol2, 9.0);
    }

    /// The per-record gain F-10725 hands to F-74444 (`gains`: cross-fade x gain curve x load) against the running
    /// original's (work/audio2/cap/eng1.txt ENG lines: R8 player, the audio rpm this+212, the load this+2144 and each
    /// record's gain argument). `MCLA_ENG_CAP` = the capture, `MCLA_ENG_CAR` = the player's car (default the R8).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR and a recording (MCLA_ENG_CAP, default work/audio2/cap)"]
    fn player_record_gains_recorded() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let cap = std::env::var("MCLA_ENG_CAP").unwrap_or_else(|_| "../../../work/audio2/cap/eng1.txt".to_string());
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
        let car = std::env::var("MCLA_ENG_CAR").unwrap_or_else(|_| "vp_aud_r8_08".to_string());
        let e = EngineSound::new(&lib, &car, false).expect("engine");
        let (first, last) = (e.recs[0], e.recs[e.recs.len() - 1]);
        if std::env::var_os("MCLA_ENG_CAP_PRINT").is_some() {
            println!("gain curve {:?}; recs {:?}", lib.curve(e.gain_curve), e.recs.iter().map(|r| (r.start, r.end, r.reference)).collect::<Vec<_>>());
        }
        let field =|l: &str, k: &str| -> f32 { l.split(' ').find_map(|w| w.strip_prefix(k)).unwrap().parse().unwrap() };
        let text = std::fs::read_to_string(&cap).unwrap();
        let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("ENG ")).collect();
        let (mut ups, mut same, mut zero_same, mut worst) = (0, 0, 0, 0.0f32);
        let mut ratios: std::collections::BTreeMap<i32, usize> = Default::default();
        for g in lines.chunks(e.recs.len()) {
            if g.len() != e.recs.len() || field(g[0], "i=") != 0.0 {
                continue;
            }
            ups += 1;
            // (the audio rpm as logged, 6 digits; the pitch carries the kick chirp's cents too)
            let x = field(g[0], "rpm212=");
            // (half a unit of the last printed digit: a fade is accepted anywhere in that window)
            let digits = g[0].split(' ').find_map(|w| w.strip_prefix("rpm212=")).unwrap().split('.').nth(1).map_or(0, |d| d.len());
            let dx = 0.5 * 10f32.powi(-(digits as i32));
            let norm = ((x - first.start) / (last.end - first.start)).min(1.0);
            // (the load volume this+148, the start boost +152 and the amp-mod bump are not in the capture: their
            // product, one scalar per update, is the recorded gain over fade x curve, the same for every record)
            let gg = lib.curve_eval(e.gain_curve, norm).unwrap_or(1.0);
            let mut ok = true;
            // (the scale from the record with the largest fade, the least sensitive to the rpm's rounding)
            let top = (0..g.len()).max_by(|&a, &b| xfade(&e.recs, a, x).total_cmp(&xfade(&e.recs, b, x))).unwrap();
            let scale = (xfade(&e.recs, top, x) > 0.0).then(|| field(g[top], "gain=") / (xfade(&e.recs, top, x) * gg));
            for (i, l) in g.iter().enumerate() {
                let (want, got) = (field(l, "gain="), xfade(&e.recs, i, x) * gg);
                if (want == 0.0) != (got == 0.0) {
                    ok = false;
                } else if let (true, Some(s)) = (want != 0.0, scale) {
                    let (a, b) = (xfade(&e.recs, i, x - dx) * gg * s, xfade(&e.recs, i, x + dx) * gg * s);
                    let (lo, hi) = (a.min(b) * (1.0 - 1e-4), a.max(b) * (1.0 + 1e-4));
                    if want < lo || want > hi {
                        ok = false;
                    } else {
                        worst = worst.max((want / (got * s) - 1.0).abs());
                    }
                }
            }
            if let Some(s) = scale {
                *ratios.entry((s * 100.0).round() as i32).or_default() += 1;
            }
            if !ok && std::env::var_os("MCLA_ENG_CAP_PRINT").is_some() {
                println!("differs: x {x} | rec {:?} | port {:?}", g.iter().map(|l| field(l, "gain=")).collect::<Vec<_>>(), (0..g.len()).map(|i| xfade(&e.recs, i, x) * gg).collect::<Vec<_>>());
            }
            zero_same += g.iter().enumerate().all(|(i, l)| (field(l, "gain=") == 0.0) == (xfade(&e.recs, i, x) == 0.0)) as usize;
            same += ok as usize;
        }
        println!("updates {ups}: one scalar over fade x curve (rel 1e-4) {same}, same sounding records {zero_same}, worst rel {worst:e}");
        println!("that scalar (load volume x boost + bump, x100): {ratios:?}");
        assert!(ups > 200 && same == ups && zero_same == ups);
    }

    /// F-51617's records for the computer racers' engines against the running original (work/audio2/cap/eng2.txt OEN
    /// lines: per update and record the `active` flag F-10725 passes, the gain argument, the pitch ratio, the audio rpm
    /// this+212). A racer object is reused for other cars, so each update's car is found by its record count, record
    /// table and XF+12. Checked: the gains are one scalar over fade x curve, as for the player, and `active` is set for
    /// every sounding record and otherwise only for the neighbour the revs move towards (the rpm rate itself is not in
    /// the capture). Record 0's "always lives" is inside F-51617 and not seen here.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR and a recording (MCLA_ENG_CAP, default work/audio2/cap)"]
    fn opponent_records_recorded() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let cap = std::env::var("MCLA_ENG_CAP").unwrap_or_else(|_| "../../../work/audio2/cap/eng2.txt".to_string());
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
        let word = |l: &str, k: &str| -> String { l.split(' ').find_map(|w| w.strip_prefix(k)).unwrap().to_string() };
        let field = |l: &str, k: &str| -> f32 { word(l, k).parse().unwrap() };
        let text = std::fs::read_to_string(&cap).unwrap();
        // (an update = one racer's lines from an i=0 to its next; only the complete ones, records 0 .. n-1 in order)
        let mut open: std::collections::HashMap<String, Vec<&str>> = Default::default();
        let mut groups: Vec<Vec<&str>> = Vec::new();
        // (updates of an entity in the revs mode, byte +1950 on its last ES line: F-74751 fades +152 to 0 - not ported, left out)
        let mut revs_mode: std::collections::HashMap<String, bool> = Default::default();
        let mut revs = 0;
        for l in text.lines() {
            if l.starts_with("ES ") {
                revs_mode.insert(word(l, "this="), field(l, "b1950=") != 0.0);
                continue;
            }
            if !l.starts_with("OEN ") {
                continue;
            }
            if revs_mode.get(&word(l, "this=")).copied().unwrap_or(false) {
                revs += (field(l, "i=") == 0.0) as usize;
                continue;
            }
            let g = open.entry(word(l, "this=")).or_default();
            if field(l, "i=") == 0.0 && !g.is_empty() {
                groups.push(std::mem::take(g));
            }
            g.push(l);
        }
        groups.extend(open.into_values());
        let complete = |g: &Vec<&str>| g.len() > 1 && g.iter().enumerate().all(|(i, l)| field(l, "i=") == i as f32);
        let names: Vec<String> = std::fs::read_dir(std::path::Path::new(&dir).join("resources/vehicle")).unwrap().filter_map(|d| d.ok()?.file_name().into_string().ok()).collect();
        let cars: Vec<(String, EngineSound)> = names.into_iter().filter_map(|c| Some((c.clone(), EngineSound::new_opponent(&lib, &c, false)?))).collect();
        let shape = |e: &EngineSound| (e.gain_curve, e.recs.iter().map(|r| (r.start.to_bits(), r.end.to_bits())).collect::<Vec<_>>());
        let mut seen: std::collections::BTreeMap<(usize, i32), Vec<String>> = Default::default();
        let (mut ups, mut ambiguous, mut unknown, mut same, mut want_ok, mut neighbour) = (0, 0, 0, 0, 0, 0);
        let mut shown = 0;
        for g in groups.iter().filter(|g| complete(g)) {
            let n = g.len();
            let xf12 = field(g[0], "xf12=");
            let refs: Vec<f32> = g.iter().map(|l| (field(l, "rpm212=") / field(l, "pitch=")).round()).collect();
            let found: Vec<&(String, EngineSound)> = cars.iter().filter(|(_, e)| e.recs.len() == n && e.pitch_off == xf12 && e.recs.iter().zip(&refs).all(|(r, q)| r.reference == *q)).collect();
            let Some((_, e)) = found.first() else {
                unknown += 1;
                continue;
            };
            if found.iter().any(|(_, o)| shape(o) != shape(e)) {
                ambiguous += 1;
                continue;
            }
            seen.entry((n, xf12 as i32)).or_insert_with(|| found.iter().map(|f| f.0.clone()).collect());
            ups += 1;
            let (first, last) = (e.recs[0], e.recs[n - 1]);
            let x = field(g[0], "rpm212=");
            let digits = word(g[0], "rpm212=").split('.').nth(1).map_or(0, |d| d.len());
            let dx = 0.5 * 10f32.powi(-(digits as i32));
            let norm = ((x - first.start) / (last.end - first.start)).min(1.0);
            let gg = lib.curve_eval(e.gain_curve, norm).unwrap_or(1.0);
            let top = (0..n).max_by(|&a, &b| xfade(&e.recs, a, x).total_cmp(&xfade(&e.recs, b, x))).unwrap();
            let scale = (xfade(&e.recs, top, x) > 0.0).then(|| field(g[top], "gain=") / (xfade(&e.recs, top, x) * gg));
            let (mut ok, mut wok) = (true, true);
            for (i, l) in g.iter().enumerate() {
                let (want, got) = (field(l, "gain="), xfade(&e.recs, i, x) * gg);
                if (want == 0.0) != (got == 0.0) {
                    ok = false;
                } else if let (true, Some(s)) = (want != 0.0, scale) {
                    let (a, b) = (xfade(&e.recs, i, x - dx) * gg * s, xfade(&e.recs, i, x + dx) * gg * s);
                    if want < a.min(b) * (1.0 - 1e-4) || want > a.max(b) * (1.0 + 1e-4) {
                        ok = false;
                    }
                }
                // (F-10725 @A: falling fast, prevStart < x < start; rising fast, end < x < nextEnd)
                let r = e.recs[i];
                let prev_start = if i == 0 { 1.0 } else { e.recs[i - 1].start };
                let next_end = if i + 1 == n { 100_000.0 } else { e.recs[i + 1].end };
                let towards = (prev_start < x && x < r.start) || (r.end < x && x < next_end);
                let (wanted, sounding) = (field(l, "wanted=") != 0.0, xfade(&e.recs, i, x) > 0.0);
                if wanted && !sounding && towards {
                    neighbour += 1;
                } else if wanted != sounding {
                    wok = false;
                }
            }
            same += ok as usize;
            want_ok += wok as usize;
            if (!ok || !wok) && std::env::var_os("MCLA_ENG_CAP_PRINT").is_some() && shown < 12 {
                shown += 1;
                println!("  x {x} ok {ok} wok {wok} | wanted {:?} | rec {:?} | port {:?}", g.iter().map(|l| field(l, "wanted=") as u8).collect::<Vec<_>>(), g.iter().map(|l| field(l, "gain=")).collect::<Vec<_>>(), (0..n).map(|i| xfade(&e.recs, i, x) * gg).collect::<Vec<_>>());
            }
        }
        for (k, v) in &seen {
            println!("cars with {} records, XF+12 {}: {v:?}", k.0, k.1);
        }
        println!("updates {ups} (car unknown {unknown}, cars of different tables {ambiguous}): gains one scalar over fade x curve {same}, active as the rule {want_ok} (a neighbour active {neighbour} times); revs-mode updates left out {revs}");
        assert!(ups > 200 && same == ups && want_ok == ups);
    }

    /// The player's load volume F-42554 against the running original (work/audio2/cap2 LV / LVX lines: the routine's
    /// inputs at its entry - reverse byte +1946, +148, +172, +188, +168, the engine's pedal, the car's speed, the
    /// transmission's shift word, XF+18 .. +34 - and +148 / +196 at its exit; the tick from the ES line before it).
    /// `MCLA_ENG_CAR` = the player's car (default the R8; cap2/s240a.txt is the 240SX).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR and a recording (MCLA_ENG_CAP)"]
    fn player_load_volume_recorded() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let cap = std::env::var("MCLA_ENG_CAP").expect("MCLA_ENG_CAP");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
        let car = std::env::var("MCLA_ENG_CAR").unwrap_or_else(|_| "vp_aud_r8_08".to_string());
        let mut e = EngineSound::new(&lib, &car, false).expect("engine");
        let word = |l: &str, k: &str| -> String { l.split(' ').find_map(|w| w.strip_prefix(k)).unwrap().to_string() };
        let field = |l: &str, k: &str| -> f32 { word(l, k).parse().unwrap() };
        let text = std::fs::read_to_string(&cap).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let mut dt = 0.0f32;
        let (mut n, mut same, mut ws_same, mut xf_ok, mut shown) = (0, 0, 0, true, 0);
        let mut branches: std::collections::BTreeMap<&str, (u32, u32)> = Default::default();
        for (k, l) in lines.iter().enumerate() {
            if l.starts_with("ES ") {
                dt = field(l, "dt=");
                continue;
            }
            if !l.starts_with("LV ") {
                continue;
            }
            let Some(x) = lines[k + 1..].iter().find(|x| x.starts_with("LVX ") && word(x, "this=") == word(l, "this=")) else { continue };
            xf_ok &= [(e.load_brake, "xf18="), (e.load_floor, "xf22="), (e.load_rate, "xf26="), (e.load_slow, "xf30="), (e.load_rev, "xf34=")].iter().all(|(v, f)| *v == field(l, f));
            e.load_vol = field(l, "load=");
            e.load_smooth = field(l, "sm172=");
            let rev = field(l, "rev=") != 0.0;
            let inp = EngineIn { throttle_in: field(l, "th188="), pedal: field(l, "pedal="), speed: field(l, "speed="), shift: field(l, "shift=") as i8, gear: if rev { -1 } else { 1 }, ..Default::default() };
            let load_abs = field(l, "d168=");
            let branch = if rev { "reverse" } else if e.load_smooth > 0.0 { "load" } else if inp.speed < e.load_slow { "slow" } else if inp.throttle_in < 0.5 { "lifted" } else if inp.shift != 0 { "shift" } else { "drive" };
            e.step_load_vol(&inp, load_abs, dt);
            n += 1;
            let ok = e.load_vol.to_bits() == field(x, "load=").to_bits();
            let wok = e.ws_in.to_bits() == field(x, "ws=").to_bits();
            same += ok as usize;
            ws_same += wok as usize;
            let b = branches.entry(branch).or_default();
            b.0 += 1;
            b.1 += (ok && wok) as u32;
            if !(ok && wok) && shown < 12 && std::env::var_os("MCLA_ENG_CAP_PRINT").is_some() {
                shown += 1;
                println!("  {branch}: {l}\n    -> {x} | port load {} ws {}", e.load_vol, e.ws_in);
            }
        }
        println!("load-volume steps {n}: load bit-equal {same}, waveshaper input bit-equal {ws_same}, XF fields as the port's {xf_ok}; per branch (steps, equal) {branches:?}");
        assert!(n > 200 && same == n && ws_same == n && xf_ok);
    }

    /// F-10725's rpm slew and start boost against the running original (cap2 ES lines, one per entity update: state +1928,
    /// slewed rpm +204, target +200 or the idle rpm with byte +1950, boost +152, the tick dt, XF+38): in state 4 the next
    /// update's +204 / +152 are the port's step from this one's. Also the rpm rate the opponents' `active` test reads
    /// ((target - s) x the rate factor 0x827D750C) against the port's (target - s) / dt, at the +-2000 threshold.
    #[test]
    #[ignore = "requires a recording (MCLA_ENG_CAP)"]
    fn rpm_slew_recorded() {
        let cap = std::env::var("MCLA_ENG_CAP").expect("MCLA_ENG_CAP");
        let word = |l: &str, k: &str| -> String { l.split(' ').find_map(|w| w.strip_prefix(k)).unwrap().to_string() };
        let field = |l: &str, k: &str| -> f32 { word(l, k).parse().unwrap() };
        let text = std::fs::read_to_string(&cap).unwrap();
        let mut last: std::collections::HashMap<String, &str> = Default::default();
        let (mut n, mut s_same, mut b_same, mut rates, mut rate_same, mut shown) = (0, 0, 0, 0, 0, 0);
        let (mut swaps, mut swap_idle, mut revs) = (0, 0, 0);
        let mut recovering: std::collections::HashSet<String> = Default::default();
        for l in text.lines().filter(|l| l.starts_with("ES ")) {
            let this = word(l, "this=");
            if let Some(p) = last.insert(this, l) {
                let target = if field(p, "b1950=") != 0.0 { field(p, "f31=") } else { field(p, "tgt=") };
                let (s, dt) = (field(p, "s="), field(p, "dt="));
                let rate_orig = (target - s) * field(p, "k=");
                let rate_port = (target - s) / dt.max(1e-6);
                rates += 1;
                rate_same += ((rate_orig > 2000.0) == (rate_port > 2000.0) && (rate_orig < -2000.0) == (rate_port < -2000.0)) as usize;
                if field(p, "st=") != 4.0 || field(l, "st=") != 4.0 {
                    continue;
                }
                // (a racer object handed another car: a new entity, whose constructor F-43725 set +204 to the engine's rpm -
                // `start_at` - with the engine at idle then; +200 is this frame's rpm already, written by F-36837 first)
                if field(p, "f30=") != field(l, "f30=") {
                    swaps += 1;
                    swap_idle += (field(l, "s=") == field(l, "f31=")) as usize;
                    continue;
                }
                let slew = (100.0 - field(p, "xf38=")) * 100.0;
                let mut s2 = s;
                approach_fma(&mut s2, target, slew, dt);
                let mut b2 = field(p, "boost=");
                if field(p, "b1950=") == 0.0 {
                    approach_fma(&mut b2, 1.0, 1.0, dt);
                }
                n += 1;
                s_same += (s2.to_bits() == field(l, "s=").to_bits()) as usize;
                // (with byte +1950, and after it until +152 is back at 1, the revs machine F-74751 drives +152 at the real frame
                // time instead - not ported, counted apart)
                let this = word(l, "this=");
                if field(p, "b1950=") != 0.0 || field(l, "b1950=") != 0.0 {
                    recovering.insert(this.clone());
                }
                if recovering.contains(&this) && field(p, "b1950=") == 0.0 && field(p, "boost=") >= 1.0 {
                    recovering.remove(&this);
                }
                if recovering.contains(&this) {
                    revs += 1;
                    b2 = field(l, "boost=");
                }
                b_same += (b2.to_bits() == field(l, "boost=").to_bits()) as usize;
                if (s2.to_bits() != field(l, "s=").to_bits() || b2.to_bits() != field(l, "boost=").to_bits()) && shown < 12 && std::env::var_os("MCLA_ENG_CAP_PRINT").is_some() {
                    shown += 1;
                    println!("  {p}\n  {l}\n    port s {s2} boost {b2}");
                }
            }
        }
        println!("state-4 steps {n}: rpm bit-equal {s_same}, boost bit-equal {b_same}; rate past +-2000 the same {rate_same} of {rates}; car swaps {swaps} (rpm = the engine's idle rpm {swap_idle}); boost not compared in {revs} revs-mode steps");
        assert!(n > 1000 && s_same == n && b_same == n && rate_same == rates && swap_idle == swaps);
    }

    /// F-98304's RPM_RACE_OPPONENT blend step against the running original in a race (cap2 OB lines: brain +8 / +380 / +404 /
    /// +428, this+216 before the step, this+232 and the tick, logged after the racing brain is found; the next ES line of
    /// that entity holds +216 after it).
    #[test]
    #[ignore = "requires a recording (MCLA_ENG_CAP)"]
    fn opponent_blend_recorded() {
        let cap = std::env::var("MCLA_ENG_CAP").expect("MCLA_ENG_CAP");
        let word = |l: &str, k: &str| -> String { l.split(' ').find_map(|w| w.strip_prefix(k)).unwrap().to_string() };
        let field = |l: &str, k: &str| -> f32 { word(l, k).parse().unwrap() };
        let text = std::fs::read_to_string(&cap).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let (mut n, mut same, mut moving, mut shown) = (0, 0, 0, 0);
        for (k, l) in lines.iter().enumerate() {
            if !l.starts_with("OB ") || field(l, "b8=") == 0.0 {
                continue;
            }
            let this = word(l, "this=");
            let Some(x) = lines[k + 1..].iter().find(|x| x.starts_with("ES ") && word(x, "this=") == this) else { continue };
            let want = if field(l, "b380=") != 0.0 { blend_target(rubber_hold(field(l, "b404="), field(l, "b428=")), field(l, "r232=")) } else { 0.0 };
            let mut b = field(l, "bl216=");
            let rate = if want < b { 0.125 } else { 0.25 };
            approach_fma(&mut b, want, rate, field(l, "dt="));
            n += 1;
            moving += (b != field(l, "bl216=")) as usize;
            let ok = b.to_bits() == field(x, "bl216=").to_bits();
            same += ok as usize;
            if !ok && shown < 12 && std::env::var_os("MCLA_ENG_CAP_PRINT").is_some() {
                shown += 1;
                println!("  {l}
  -> {x}
    port {b} (target {want})");
            }
        }
        println!("blend steps {n} ({moving} moving): bit-equal {same}");
        assert!(n > 200 && moving > 20 && same == n);
    }

    /// The loops' start parameters against the running original (cap2 SPS / SPV / SPP lines at the setter calls F-17698,
    /// F-95814, F-89149 in F-74444 (player) and F-51617 (opponents), with the entity's own fields): the chain send and dry
    /// are EFFECT_PARAMS +12 / +16 of entity +1232, the second level s16 XF+10 x 0.01f, the chain pan bit set for record 0
    /// only - the values the port's loop request takes (`dry`, `fx_send`, `vol2`, `pans_chain`).
    #[test]
    #[ignore = "requires a recording (MCLA_ENG_CAP)"]
    fn loop_start_params_recorded() {
        let cap = std::env::var("MCLA_ENG_CAP").expect("MCLA_ENG_CAP");
        let word = |l: &str, k: &str| -> String { l.split(' ').find_map(|w| w.strip_prefix(k)).unwrap().to_string() };
        let field = |l: &str, k: &str| -> f32 { word(l, k).parse().unwrap() };
        let text = std::fs::read_to_string(&cap).unwrap();
        let (mut sends, mut send_ok, mut vols, mut vol_ok, mut pans, mut pan_ok) = (0, 0, 0, 0, 0, 0);
        let mut who: std::collections::BTreeMap<String, usize> = Default::default();
        for l in text.lines() {
            let kind = l.split(' ').next().unwrap_or("");
            if matches!(kind, "SPS" | "SPV" | "SPP") {
                *who.entry(format!("{kind} {}", l.split(' ').nth(1).unwrap_or(""))).or_default() += 1;
            }
            match kind {
                "SPS" => {
                    sends += 1;
                    send_ok += (field(l, "send=").to_bits() == field(l, "fx12=").to_bits() && field(l, "dry=").to_bits() == field(l, "fx16=").to_bits()) as usize;
                }
                "SPV" => {
                    vols += 1;
                    let port = field(l, "xf10=") * 0.01;
                    vol_ok += (u32::from_str_radix(&word(l, "vol2="), 16).unwrap() == port.to_bits()) as usize;
                }
                "SPP" => {
                    pans += 1;
                    pan_ok += ((field(l, "bit=") != 0.0) == (field(l, "rec=") == 0.0)) as usize;
                }
                _ => {}
            }
        }
        println!("send / dry {send_ok} of {sends}, second level {vol_ok} of {vols}, chain pan {pan_ok} of {pans}; lines {who:?}");
        assert!(sends > 20 && vols > 20 && pans > 20 && send_ok == sends && vol_ok == vols && pan_ok == pans);
    }

    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn camaro_sweep() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let mut a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
        lib.listener = (Vec3::new(0.0, 1.5, 6.0), Vec3::X, Vec3::NEG_Z);
        let mut e = EngineSound::new(&lib, "vp_chv_camaro_06", false).expect("engine");
        let dt = 1.0 / 60.0;
        let (idle, max) = (900.0, 6500.0);
        let mut rows = Vec::new();
        for f in 0..600 {
            let t = f as f32 * dt;
            // idle 1 s, then a rise to the limiter over 6 s, held, then off the throttle
            let rpm = if t < 1.0 { idle } else if t < 7.0 { idle + (max - idle) * (t - 1.0) / 6.0 } else if t < 8.5 { max } else { idle };
            let on = t >= 1.0 && t < 8.5;
            let inp = EngineIn { pos: Vec3::ZERO, rpm, idle_rpm: idle, max_rpm: max, torque: if on { 300.0 } else { -50.0 }, full_torque: 400.0, pedal: on as u8 as f32, throttle_in: on as u8 as f32, speed: 20.0, gear: 3, shift: 0, idle_only: false, ..Default::default() };
            e.update(&mut lib, &inp, dt);
            lib.update(dt, &mut a);
            a.end_frame();
            a.render(dt);
            rows.push((t, e.s, e.audio_rpm, e.load_vol, e.gains.clone()));
        }
        for r in rows.iter().step_by(30) {
            let g: Vec<String> = r.4.iter().map(|g| format!("{g:.2}")).collect();
            println!("t {:5.2} S {:6.0} x {:6.0} load {:.3} | {}", r.0, r.1, r.2, r.3, g.join(" "));
        }
        // the table's windows: on the Camaro three loops sound together between 4000 and 4020 (record 6 ends at 4020,
        // record 8 starts at 4000)
        for r in &rows {
            let live = r.4.iter().filter(|g| **g > 1e-4).count();
            assert!(live <= 3, "{live} loops at x {}", r.2);
        }
        let at = |x: f32| rows.iter().min_by(|a, b| (a.2 - x).abs().total_cmp(&(b.2 - x).abs())).unwrap();
        // idle: record 0 alone; the limiter: the top record alone
        assert!(at(900.0).4[1..].iter().all(|g| *g == 0.0));
        assert!(at(6065.0).4[..9].iter().all(|g| *g == 0.0));
        if let Ok(p) = std::env::var("MCLA_ENGINE_WAV") {
            a.write_capture(&p);
        }
    }

    /// The bus effects on the Camaro (BUS_GRAPH_RULE.md): the REVERB_* objects as the probes read them, the engine
    /// loops' dry 0.5 and chain send from VP_CHV_CAMARO_06's EFFECT_PARAMS, the waveshaper drive in [0, 1] and up
    /// under throttle, and the mix with the chain and the reverb buses against the dry loops alone.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn bus_effects_camaro() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let read = |lib: &SoundLib, n: &str| {
            let o = lib.game.get(joaat(n)).expect(n);
            (be_f32(o, 10), be_f32(o, 14), be_f32(o, 18))
        };
        let run = |effects: bool| {
            let mut a = crate::audio::Audio::new_offline(&dir).expect("audio");
            let mut lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
            assert_eq!(read(&lib, "REVERB_DEFAULT"), (0.125, 0.6, 10.0));
            assert_eq!(read(&lib, "REVERB_PROBE_UP"), (0.5, 0.6, 10.0));
            assert_eq!(read(&lib, "REVERB_PROBE_AIR"), (0.11, 0.7, 10.0));
            lib.listener = (Vec3::new(0.0, 1.5, 6.0), Vec3::X, Vec3::NEG_Z);
            let mut e = EngineSound::new(&lib, "vp_chv_camaro_06", false).expect("engine");
            assert!(e.chain != 0);
            println!("dry {} send {}", be_f32(&e.eff, 16), be_f32(&e.eff, 12));
            if !effects {
                e.chain = 0;
            } else {
                lib.reverb_env = (0.125, 0.6);
            }
            let dt = 1.0 / 60.0;
            let mut drive = Vec::new();
            for f in 0..240 {
                let on = f >= 60;
                let rpm = if on { 900.0 + (f - 60) as f32 * 25.0 } else { 900.0 };
                let inp = EngineIn { pos: Vec3::ZERO, rpm, idle_rpm: 900.0, max_rpm: 6500.0, torque: if on { 300.0 } else { -50.0 }, full_torque: 400.0, pedal: on as u8 as f32, throttle_in: on as u8 as f32, speed: 20.0, gear: 3, ..Default::default() };
                e.update(&mut lib, &inp, dt);
                lib.update(dt, &mut a);
                a.end_frame();
                a.render(dt);
                drive.push(e.ws_drive);
            }
            let c = a.captured();
            let rms = (c.iter().map(|s| s * s).sum::<f32>() / c.len().max(1) as f32).sqrt();
            (rms, drive, c.to_vec())
        };
        let (wet, drive, cw) = run(true);
        let (dry, _, cd) = run(false);
        println!("rms with effects {wet:.4}, dry only {dry:.4}; drive at 1 s {:.3}, at 4 s {:.3}", drive[59], drive[239]);
        assert!(drive.iter().all(|d| (0.0..=1.0).contains(d)));
        assert!(drive[239] > drive[59]);
        assert!(wet > 0.0 && dry > 0.0);
        let diff = cw.iter().zip(&cd).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(diff > 1e-3, "the effects change the mix ({diff})");
    }

    /// The start with ignition 0 (CopArrest's Racer_PlayIgnition) on the Camaro: the loops stop, the next update plays
    /// IGNITION_EXOTIC_MASTER, the crank lasts 750 - 1000 ms with no loop sounding and the revs falling at 7500 rpm/s,
    /// then the engine runs again (`take_running` once), the revs flare to 5000 at 8000 rpm/s, settle at 3000 rpm/s
    /// down to idle and the +6 dB start boost falls back to 0 dB at 1 per second.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn ignition_crank_flare() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
        let mut e = EngineSound::new(&lib, "vp_chv_camaro_06", false).expect("engine");
        assert_eq!((e.crank, e.flare, e.settle, e.boost_db), ((750, 1000), (5000.0, 8000.0), 3000.0, 6.0));
        assert_eq!(e.ign_sound, joaat("IGNITION_EXOTIC_MASTER"));
        let dt = 1.0 / 60.0;
        let idle = 900.0;
        let inp = EngineIn { pos: Vec3::ZERO, rpm: idle, idle_rpm: idle, max_rpm: 6500.0, full_torque: 400.0, speed: 0.0, gear: 1, ..Default::default() };
        for _ in 0..60 {
            e.update(&mut lib, &inp, dt);
        }
        assert!(e.voices[0].is_some() && e.state == 4);
        e.start(&mut lib, false);
        assert!(e.voices.iter().all(|v| v.is_none()) && e.state == 0);
        assert!((e.boost - 1.995_262).abs() < 1e-4);
        e.update(&mut lib, &inp, dt);
        assert_eq!(e.state, 1);
        assert!(e.ign.is_some() && (0.75..=1.0).contains(&e.timer));
        let crank = e.timer;
        let mut t = 0.0;
        while e.state == 1 {
            e.update(&mut lib, &inp, dt);
            t += dt;
            assert!(e.voices.iter().all(|v| v.is_none()), "no loop during the crank");
            assert!(t < 1.1);
        }
        assert!((t - crank).abs() <= dt + 1e-4, "crank {crank} ended at {t}");
        assert!(e.take_running() && !e.take_running());
        assert!(e.ign.is_none() && e.state == 5);
        // the flare: 0 -> 5000 at 8000 rpm/s (0.625 s), the boost held
        let mut t = 0.0;
        while e.state == 5 {
            e.update(&mut lib, &inp, dt);
            t += dt;
        }
        assert!((t - 0.625).abs() < 2.0 * dt, "flare {t}");
        assert!((e.boost - 1.995_262).abs() < 1e-4);
        // the settle: 5000 -> 900 at 3000 rpm/s (1.37 s), then running and the boost falling
        let mut t = 0.0;
        while e.state == 6 {
            e.update(&mut lib, &inp, dt);
            t += dt;
        }
        assert!((t - 4100.0 / 3000.0).abs() < 2.0 * dt, "settle {t}");
        assert_eq!((e.state, e.s), (4, idle));
        for _ in 0..30 {
            e.update(&mut lib, &inp, dt);
        }
        assert!((e.boost - (1.995_262 - 0.5)).abs() < 0.02, "{}", e.boost);
        assert!(e.voices[0].is_some());
    }

    /// The stop with 1 (the driver's "impact start car" after a damage-out) on the Camaro: the loops stop at once, the
    /// revs fall, after XF+117 = 2250 ms IGNITION_KAPUT_MASTER plays and the sound stays silent (state 3) until the
    /// start, which cranks as usual.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn stop_kaput_then_start() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
        let mut e = EngineSound::new(&lib, "vp_chv_camaro_06", false).expect("engine");
        assert_eq!((e.kaput_sound, e.kaput_delay), (joaat("IGNITION_KAPUT_MASTER"), 2.25));
        let dt = 1.0 / 60.0;
        let idle = 900.0;
        let inp = EngineIn { pos: Vec3::ZERO, rpm: idle, idle_rpm: idle, max_rpm: 6500.0, full_torque: 400.0, speed: 0.0, gear: 1, ..Default::default() };
        for _ in 0..60 {
            e.update(&mut lib, &inp, dt);
        }
        assert!(e.voices[0].is_some() && e.state == 4);
        e.stop_engine(&mut lib, true);
        assert!(e.voices.iter().all(|v| v.is_none()) && e.state == 2 && e.stopped());
        assert!((e.boost - 1.995_262).abs() < 1e-4);
        let mut t = 0.0;
        while e.state == 2 {
            e.update(&mut lib, &inp, dt);
            t += dt;
            assert!(e.voices.iter().all(|v| v.is_none()), "no loop in the stop's wait");
            assert!(t < 3.0);
        }
        assert!((t - 2.25).abs() <= dt + 1e-4, "kaput at {t}");
        assert!(e.ign.is_some() && e.state == 3 && e.s < 1.0);
        for _ in 0..120 {
            e.update(&mut lib, &inp, dt);
            assert!(e.voices.iter().all(|v| v.is_none()) && e.state == 3);
        }
        e.start(&mut lib, false);
        e.update(&mut lib, &inp, dt);
        assert_eq!(e.state, 1);
        // the restore's set-running (vt+48): state 4 at once, the loops back on the next update, no crank
        e.stop_engine(&mut lib, true);
        e.set_running();
        assert!(!e.held() && e.timer == 0.0);
        e.update(&mut lib, &inp, dt);
        assert!(e.state == 4 && e.voices[0].is_some());
    }
}
