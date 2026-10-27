//! The cars' engine sound as the original makes it (mcCarAudioEntity): the car's `_ENGINE_XFADER` table of recorded
//! loops cross-faded by a slewed, normalised rpm, the gain curve and load volume, the "kick" pitch chirp and amp-mod
//! bump, and the gear-shift and backfire one-shots. Rules and sources: `docs/reference/audio/ENGINE_AUDIO_RULE.md`.
//! Computer racers use their `_O_` objects with the opponent rules (`new_opponent`; VEHICLE_EXTRA_AUDIO_RULE.md
//! section 6); the turbo whine and blow-off read this sound's smoothed rpm and load volume (caraudio.rs).
//!
//! Not ported (open there): the start / stop state machine (ignition, crank, rev flare - the port's engine is always
//! running), the layer-B overlay (off on every player car), the reverse whine (NULL on the cars looked at).

use crate::sndobj::{Handle, Req, SoundLib};
use glam::Vec3;
use mcla_formats::audcfg::lin2db;
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

#[derive(Clone, Copy)]
struct Rec {
    sound: u32,
    start: f32,
    end: f32,
    reference: f32,
}

/// What the engine sound reads from the car each frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct EngineIn {
    pub pos: Vec3,
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
    /// opponents: the car's driver is racing (byte 1951) and its speed over the gearbox's top-gear speed (this+232)
    pub racing: bool,
    pub speed_ratio: f32,
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

pub struct EngineSound {
    recs: Vec<Rec>,
    /// XF+12 cents, XF+14 mB as dB, EFFECT+10 mB as dB
    pitch_off: f32,
    vol_a: f32,
    effect_db: f32,
    /// XF+18 / +22 / +26 / +30 / +34
    load_brake: f32,
    load_floor: f32,
    load_rate: f32,
    load_slow: f32,
    load_rev: f32,
    slew: f32,
    gain_curve: u32,
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
    amp_curve: u32,
    amp_time: (f32, f32),
    amp_amount: f32,
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
    s: f32,
    load_vol: f32,
    load_smooth: f32,
    prev_torque: f32,
    prev_rpm: f32,
    clock: f32,
    kick_at: f32,
    lfo: Lfo,
    amp_left: f32,
    amp_len: f32,
    prev_shift: i8,
    prev_gear: i32,
    pending_gear: Option<(f32, u32)>,
    backfire_armed: Option<f32>,
    /// opponents: the RPM_RACE_OPPONENT blend (this+216) and the wobble (this+220 target, +224 rate, +228 value)
    blend: f32,
    wobble: (f32, f32, f32),
    racing_curve: u32,
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
            player,
            wobble_target: (be_f32(amp, 30), be_f32(amp, 34)),
            wobble_rate: (be_f32(amp, 38), be_f32(amp, 42)),
            blend: 0.0,
            wobble: (1.0, 0.0, 1.0),
            racing_curve: joaat("RPM_RACE_OPPONENT"),
            pitch_off: s16(xf, 12),
            vol_a: s16(xf, 14) * 0.01,
            effect_db: if eff.is_empty() { 0.0 } else { s16(eff, 10) * 0.01 },
            load_brake: be_f32(xf, 18),
            load_floor: be_f32(xf, 22),
            load_rate: be_f32(xf, 26),
            load_slow: be_f32(xf, 30),
            load_rev: be_f32(xf, 34),
            slew: (100.0 - be_f32(xf, 38)) * 100.0,
            gain_curve: be_u32(xf, 119),
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
            amp_amount: be_f32(amp, 22) - be_f32(amp, 26),
            audio_rpm: 0.0,
            gains: vec![0.0; n],
            voices: vec![None; n],
            recs,
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
            prev_shift: 0,
            prev_gear: 0,
            pending_gear: None,
            backfire_armed: None,
        })
    }

    /// The smoothed engine speed (this+204) and the load volume (this+148): the turbo whine reads both.
    pub fn smoothed_rpm(&self) -> f32 {
        self.s
    }
    pub fn load_volume(&self) -> f32 {
        self.load_vol
    }

    pub fn stop(&mut self, lib: &mut SoundLib) {
        for h in self.voices.iter_mut().filter_map(|h| h.take()) {
            lib.stop(h);
        }
    }

    pub fn update(&mut self, lib: &mut SoundLib, e: &EngineIn, dt: f32) {
        self.clock += dt;
        // inputs (F-36837)
        let load = if e.full_torque > 0.0 { e.torque / e.full_torque } else { 0.0 };
        approach_fma(&mut self.load_smooth, load, 6.0, dt);
        let load_abs = load.abs().min(1.0);
        let reversing = e.gear == -1 && e.speed > 1.0;
        // load volume (player F-42554, opponent F-57529)
        if reversing {
            approach_fma(&mut self.load_vol, self.load_rev, 0.8, dt);
        } else if !self.player {
            let drive = load_abs.max(e.pedal).max(self.load_floor).max(self.load_brake);
            let full = if e.racing { 1.0 } else { drive };
            let target = if self.load_smooth > 0.0 || e.speed < self.load_slow {
                full
            } else if e.pedal < 0.5 {
                if e.racing { 1.0 } else { self.load_floor }
            } else if e.shift != 0 {
                self.load_brake
            } else {
                full
            };
            approach_fma(&mut self.load_vol, target, self.load_rate, dt);
        } else {
            let drive = load_abs.max(e.throttle_in).max(e.pedal).max(self.load_floor).max(self.load_brake);
            if self.load_smooth > 0.0 {
                approach_fma(&mut self.load_vol, drive, self.load_rate, dt);
            } else if e.speed < self.load_slow {
                approach(&mut self.load_vol, 1.0, dt);
            } else if e.throttle_in < 0.5 || e.shift != 0 {
                approach_fma(&mut self.load_vol, self.load_brake, self.load_rate, dt);
            } else {
                approach_fma(&mut self.load_vol, drive, self.load_rate, dt);
            }
        }
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
        // rpm slew (F-10725, state 4 = running)
        let target = if e.idle_only { e.idle_rpm } else { e.rpm };
        let rpm_rate = (target - self.s) / dt.max(1e-6);
        approach_fma(&mut self.s, target, self.slew, dt);
        // normalised rpm, audio rpm, master gain
        let mut norm = ((self.s - e.idle_rpm) / (e.max_rpm - e.idle_rpm).max(1.0)).min(1.0);
        if !self.player {
            // the RPM_RACE_OPPONENT blend (F-98304; the brain's share, brain +380 / +404 / +428, is not decoded:
            // the speed term alone) and the wobble (F-10725 @A-74853)
            let want = if e.racing { (e.speed_ratio * 3.0).min(1.0) } else { 0.0 };
            let rate = if want < self.blend { 0.125 } else { 0.25 } * dt;
            approach(&mut self.blend, want, rate);
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
        let mut g = lib.curve_eval(self.gain_curve, norm).unwrap_or(1.0) * self.load_vol;
        if self.amp_left > 0.0 {
            let c = lib.curve_eval(self.amp_curve, self.amp_left / self.amp_len.max(1e-6)).unwrap_or(0.0);
            g = (g + c * self.amp_amount).max(0.0);
            self.amp_left -= dt;
        }
        // per-record cross-fade and pitch
        let n = self.recs.len();
        for i in 0..n {
            let r = self.recs[i];
            let prev_end = if i == 0 { 1.0 } else { self.recs[i - 1].end };
            let next_start = if i + 1 == n { 100_000.0 } else { self.recs[i + 1].start };
            let fade = if x >= prev_end && x <= next_start {
                1.0
            } else if x > r.start && x <= r.end && x < prev_end {
                ((x - r.start) / (prev_end - r.start) * std::f32::consts::FRAC_PI_2).sin()
            } else if x <= r.end && x > next_start {
                ((x - next_start) / (r.end - next_start) * std::f32::consts::FRAC_PI_2).cos()
            } else {
                0.0
            };
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
            let vol = lin2db(fade * g) + self.effect_db + self.vol_a;
            let cents = (self.pitch_off + 1200.0 * (x / r.reference).log2() + lfo_cents) as i32;
            let req = Req { pos: Some(e.pos), volume: vol, pitch: cents, player: self.player, ..Default::default() };
            match self.voices[i] {
                Some(h) if lib.playing(h) => lib.set(h, req),
                _ => self.voices[i] = lib.play_hash(r.sound, req),
            }
        }
        self.audio_rpm = x;
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
                lib.play_hash(snd, Req { pos: Some(e.pos), volume: self.gear_vol, pitch: self.gear_pitch, player: self.player, ..Default::default() });
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
                    lib.play_hash(self.backfire, Req { player: self.player, ..Req::at(e.pos) });
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
        for k in 0..3 {
            let v = self.lfo_freq_var;
            self.lfo.freq[k] = (self.lfo_freq[k] + lib.rand_float(-v, v)).max(0.0);
            self.lfo.phase[k] = self.lfo_phase[k];
        }
        self.lfo.amp = (self.lfo_amp.0 + lib.rand_int(-self.lfo_amp.1, self.lfo_amp.1)).max(0) as f32;
        self.lfo.delay = self.lfo_delay.max(0.0);
        self.lfo.hold = 0.0;
        self.lfo.release = 0.0;
        self.lfo.running = true;
        if self.amp_amount > 0.0 {
            let v = self.amp_time.1;
            self.amp_len = (self.amp_time.0 + lib.rand_float(-v, v)).max(0.0);
            self.amp_left = self.amp_len;
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
}
