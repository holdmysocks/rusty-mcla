//! The original's sound objects at run time: a sound is started by its `sounds.dat` name and played as the game's
//! audio engine plays it (the tree of Wrapper / Multitrack / Randomized / Simple ... sounds, each level's volume,
//! pitch, pan, predelay, start offset, attack and release, the category's volume, the rolloff curve, the stereo pan
//! law and Doppler). Rules and sources: `docs/reference/audio/SOUND_FORMAT.md`.
//!
//! Approximate (written down there): Crossfade plays its near child; MathOperation / variables are not evaluated; the
//! "requested settings" the game passes are the subset in [`Req`].

use crate::audio::{Audio, Clip};
use glam::Vec3;
use hashbrown::HashMap;
use mcla_formats::audcfg::{self, db2lin, lin2db, Category, Curve, Dat, Payload, SoundHeader};
use mcla_formats::hash::joaat;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A wave of a converted bank: its file and the wave info the converter kept.
#[derive(Clone)]
struct WaveInfo {
    file: PathBuf,
    loop_start: i32,
    /// 0.01 dB
    headroom: i16,
    /// the wave's duration and loop start in whole ms from its header's sample count and rate (F-13862)
    dur_ms: Option<f32>,
    loop_ms: Option<f32>,
}

/// F-13862: a header sample count to whole ms, floor(n / (rate x 0.001)) in f32 (the fsel floor of the original)
fn wave_ms(samples: u32, rate: u32) -> f32 {
    (samples as f32 / (rate as f32 * 0.001)).floor()
}

/// What the game asks of a playing sound (the original's requested-settings slot, the part the port uses).
#[derive(Clone, Copy, Debug)]
pub struct Req {
    /// world position (positional sounds); `None` = at the listener, no rolloff
    pub pos: Option<Vec3>,
    /// dB added at the top (entry+64)
    pub volume: f32,
    /// dB added to the second level, after the voice's chain and with the category and rolloff (entry+68, F-63029;
    /// set by F-95814: the engine loops' XF+10 x 0.01)
    pub vol2: f32,
    /// cents added at the top (entry+80)
    pub pitch: i32,
    /// pan in degrees (entry+82), -1 = none
    pub pan: i16,
    /// Doppler factor (entry+84)
    pub doppler: f32,
    /// one of the player vehicle's sounds (the mixer's time-slow applies)
    pub player: bool,
    /// start offset (P+36): milliseconds, or, when `start_pct` is set (P+70 bit 0x40), percent of the root sound's duration
    /// (F-90115)
    pub start_ms: f32,
    pub start_pct: f32,
    /// low-pass and high-pass cutoffs in Hz (entry+88 / +90; 23900 and 0 = open)
    pub lpf: u16,
    pub hpf: u16,
    /// the dry level and the effect chain send (entry +76 / +72, set by F-17698: the engine loops' EFFECT_PARAMS +16 /
    /// +12), the chain (an engine's PLAYER_WAVESHAPER, `SoundLib::set_chain`; 0 = none) and whether this voice pans
    /// the chain's output (entry +95 bit 2, F-89149: F-74444 sets it on the first engine loop)
    pub dry: f32,
    pub fx_send: f32,
    pub chain: u32,
    pub pans_chain: bool,
    /// the request's category (create params +28, the category record; StartSound F-33564 stores its index at the root's
    /// init +82): the root sound's parent category, which a header category under it overrides; 0 = none (BASE). The
    /// engine loops are the only sounds that give one (their own headers have none)
    pub category: u32,
    /// the request's occluder (+32; F-30479 keeps it in the voice entry +180): its slot in [`SoundLib::set_occlusion`]
    /// plus one, 0 = none. Every frame the voice takes the occluder's volume and cutoff (F-19918 into entry +160 / +164),
    /// which the pan applies (F-56973: the speaker gains times the volume, the low-pass at most the cutoff)
    pub occluder: u16,
}

impl Default for Req {
    fn default() -> Self {
        Req { pos: None, volume: 0.0, vol2: 0.0, pitch: 0, pan: -1, doppler: 1.0, player: false, start_ms: 0.0, start_pct: 0.0, lpf: 23900, hpf: 0, dry: 1.0, fx_send: 0.0, chain: 0, pans_chain: false, category: 0, occluder: 0 }
    }
}

impl Req {
    pub fn at(pos: Vec3) -> Req {
        Req { pos: Some(pos), ..Default::default() }
    }
    pub fn vol(mut self, db: f32) -> Req {
        self.volume = db;
        self
    }
    pub fn pitch(mut self, cents: i32) -> Req {
        self.pitch = cents;
        self
    }
    /// the root sound's parent offset (+108 = P+36, the percent flag from P+70; F-99744)
    fn offset(&self) -> (u32, bool) {
        if self.start_pct > 0.0 { (self.start_pct as u32, true) } else { (self.start_ms as u32, false) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle(u64);

/// The original's random generator: multiply-with-carry, x = lo * 0x5CDCFAA7 + carry (F-62480 / F-60851).
struct Mwc {
    lo: u32,
    carry: u32,
}

impl Mwc {
    fn next(&mut self) -> u32 {
        let x = self.lo as u64 * 0x5CDC_FAA7 + self.carry as u64;
        self.lo = x as u32;
        self.carry = (x >> 32) as u32;
        self.lo
    }
    /// integer in [a, b]
    fn int(&mut self, a: i32, b: i32) -> i32 {
        if b <= a {
            return a;
        }
        ((self.next() & 0x7FFF_FFFF) % (b - a + 1) as u32) as i32 + a
    }
    /// float in [a, b)
    fn float(&mut self, a: f32, b: f32) -> f32 {
        // (F-60851: (b - a) * the 23 bits, then one fused multiply-add with the scale 2^-23 and a)
        ((b - a) * (self.next() & 0x7F_FFFF) as f32).mul_add(1.0 / 8_388_608.0, a)
    }
}

thread_local! {
    /// The audio's shared generator state (lo, carry): one state in the original (F-62480 / F-60851 step it), drawn by
    /// the sound objects' variances and Randomized picks and by the speech manager (its gate, script and take draws,
    /// F-95157 / F-70477). (approx.) The seed: the original's boot state is not known; the game's other callers of
    /// this state (outside the audio) are not modelled, so the sequence is the original's kind, never its numbers.
    static AUDIO_MWC: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((0x1234_5678, 0x9ABC)) };
}

/// The shared audio generator (`AUDIO_MWC`) as the sound library draws it.
struct SharedMwc;

impl SharedMwc {
    fn with<R>(&self, f: impl FnOnce(&mut Mwc) -> R) -> R {
        AUDIO_MWC.with(|c| {
            let (lo, carry) = c.get();
            let mut m = Mwc { lo, carry };
            let r = f(&mut m);
            c.set((m.lo, m.carry));
            r
        })
    }
    fn int(&self, a: i32, b: i32) -> i32 {
        self.with(|m| m.int(a, b))
    }
    fn float(&self, a: f32, b: f32) -> f32 {
        self.with(|m| m.float(a, b))
    }
}

/// F-62480 on the shared audio state: an integer in [lo, hi] = lo + (new lo & 0x7FFFFFFF) mod (hi - lo + 1). It always
/// draws (also for lo == hi), as the original.
pub fn audio_rand(lo: i32, hi: i32) -> i32 {
    SharedMwc.with(|m| lo + ((m.next() & 0x7FFF_FFFF) % (hi - lo + 1).max(1) as u32) as i32)
}

/// The Doppler ratio (F-42745): v = (dd x factor) / (ms x 0.001) (the caller's 30 ms), clamped to +-100 by fsel, then
/// 340 / (v + 340); 1.0 when `ms` is 0. (c = SpeedOfSound 340 and the limit MaxRelativeVelocityForDoppler 100 of the
/// audio xml, env +3668 / +3672.)
fn doppler_ratio(dd: f32, factor: f32, ms: u32) -> f32 {
    if ms == 0 {
        return 1.0;
    }
    let (c, vmax) = (340.0f32, 100.0f32);
    let v = (dd * factor) / (ms as f32 * 0.001);
    let hi = if vmax - v >= 0.0 { v } else { vmax };
    let lo = if -vmax - v >= 0.0 { -vmax } else { v };
    if v >= 0.0 { c / (hi + c) } else { c / (lo + c) }
}

/// A distance as the environment update measures it: sqrt(fmadds(x, x, fmadds(y, y, z z))).
fn env_len(v: Vec3) -> f32 {
    v.x.mul_add(v.x, v.y.mul_add(v.y, v.z * v.z)).sqrt()
}

/// One Doppler sample of the environment update (F-77682 at the call, F-42745, the smoothing in F-24161): `src` - `lis` = the source
/// minus the listener now, `ms` / `ml` their motions since the last sample; the distance change |rel| - |(src - ms) -
/// (lis - ml)| over 30 ms, and the voice's ratio = fmadds(old, 0.5, new x 0.5).
fn doppler_sample(ratio: f32, src: Vec3, lis: Vec3, ms: Vec3, ml: Vec3, factor: f32) -> f32 {
    let dd = env_len(src - lis) - env_len((src - ms) - (lis - ml));
    ratio.mul_add(0.5, doppler_ratio(dd, factor, 30) * 0.5)
}

/// A new sound's variance draws (F-40563), each with F-62480 only when its variance is not 0, in this order: volume
/// (fmadds(+15, 0.01, r x 0.01): the sound's +28), pitch (+19 + r, 16 bits: +32), pan (none when 0xFFFF and then no draw,
/// else (s16(r) + s16(+23) + 360) % 360 truncating: +10), predelay (+27 + r: the word +88) and start offset (+31 + r: the
/// word +104).
fn variances(h: &SoundHeader) -> (f32, i16, i16, u32, u32) {
    let draw = |v: u32| if v == 0 { 0 } else { SharedMwc.int(-(v as i32), v as i32) };
    let r = draw(h.volume_var as u32);
    let vol = (h.volume as f32).mul_add(0.01, r as f32 * 0.01);
    let pitch = (h.pitch as u16 as i32).wrapping_add(draw(h.pitch_var as u32)) as i16;
    let pan = if h.pan == -1 { -1 } else { ((draw(h.pan_var as u32) as i16 as i32 + h.pan as i32 + 360) % 360) as i16 };
    let pd = (h.predelay as u32).wrapping_add(draw(h.predelay_var as u32) as u32);
    let so = h.start_offset.wrapping_add(draw(h.start_offset_var) as u32);
    (vol, pitch, pan, pd, so)
}

/// The shared audio generator's state (lo, carry), and setting it (tests against recorded states).
#[cfg(test)]
pub fn audio_rng_state() -> (u32, u32) {
    AUDIO_MWC.with(|c| c.get())
}
#[cfg(test)]
pub fn set_audio_rng_state(s: (u32, u32)) {
    AUDIO_MWC.with(|c| c.set(s));
}

/// Settings handed down the tree each update (F-63029 / F-57176).
#[derive(Clone, Copy)]
struct Ctx {
    vol: f32,
    pitch: i32,
    pan: i16,
    category: u32,
    curve: u32,
    curve_scale: f32,
    doppler: f32,
    mask: u8,
    /// tristate 1 (positional) as inherited: 0 off, 1 on, 2 unset
    positional: u8,
    /// the chain's filters (F-63029): LPF the minimum, HPF the maximum down the tree, Hz
    lpf: u16,
    hpf: u16,
}

/// The runtime terms a node is updated with, its own (not inherited): +36 volume (the Envelope's, F-66882), +34 pitch,
/// +124 pan, and at the root the request entry's pitch / pan / filters (+80 / +82 / +88 / +90). The port's request volume
/// (entry +64) is taken as the root's +36 (I: where the original adds it is not decoded; it is a dB term either way).
#[derive(Clone, Copy, Debug)]
struct Rt {
    vol: f32,
    pitch: i32,
    pan: i16,
    rpan: i16,
    lpf: Option<u16>,
    hpf: Option<u16>,
}

impl Rt {
    const NONE: Rt = Rt { vol: 0.0, pitch: 0, pan: -1, rpan: -1, lpf: None, hpf: None };
}

/// A root's parent (none: 0 dB, 0 cents, no pan, 23900 / 0 Hz; F-63029) and its runtime terms from the request.
fn root_ctx(r: &Req) -> (Ctx, Rt) {
    let ctx = Ctx { vol: 0.0, pitch: 0, pan: -1, category: if r.category != 0 { r.category } else { joaat("BASE") }, curve: 0, curve_scale: 1.0, doppler: 1.0, mask: 0, positional: 2, lpf: 23900, hpf: 0 };
    (ctx, Rt { vol: r.volume, pitch: r.pitch, pan: r.pan, rpan: -1, lpf: Some(r.lpf), hpf: Some(r.hpf) })
}

/// The settings combine F-63029 of one node: volume = own (+28) + (runtime + the parent's unless do-not-inherit 0x80),
/// pitch = own + runtime + request + the parent's unless 0x40, as an s16; pan = the parent's (none under 0x20), own,
/// request, runtime pan combined (a sum where both are set, else the one set) and then mod 360 (truncating); LPF =
/// min(the request's or 23900, the parent's), HPF = max(the request's, the parent's). Variables are not ported.
fn settings(ni: u8, vol: f32, pitch: i32, pan: i16, p: &Ctx, rt: &Rt) -> (f32, i32, i16, u16, u16) {
    let v = vol + if ni & 0x80 != 0 { rt.vol } else { rt.vol + p.vol };
    let pi = (pitch + rt.pitch + if ni & 0x40 != 0 { 0 } else { p.pitch }) as i16 as i32;
    let comb = |a: i32, b: i32| if a == -1 { b } else if b == -1 { a } else { a + b };
    let mut c = if ni & 0x20 != 0 { -1 } else { p.pan as i32 };
    for b in [pan, rt.pan, rt.rpan] {
        c = comb(c, b as i32);
    }
    if c != -1 {
        c %= 360;
    }
    let lpf = rt.lpf.map_or(23900, |l| if l < p.lpf { l } else { p.lpf });
    let lpf = if lpf < p.lpf { lpf } else { p.lpf };
    let hpf = rt.hpf.map_or(p.hpf, |h| if h > p.hpf { h } else { p.hpf });
    (v, pi, c as i16, lpf, hpf)
}

/// The attack of F-37133: `ms` the AttackTime (+68), `po` the parent offset (+108, raw), `own_pd` the own predelay
/// (+88), `ppd` the parent predelay (+96).
#[derive(Clone, Copy, Debug)]
struct Attack {
    ms: u32,
    po: u32,
    own_pd: u32,
    ppd: u32,
}

impl Attack {
    /// The dB the attack adds at `now` to a sound played at `play_at`; `None` when it adds nothing: no attack, a parent
    /// offset past it, past its end (`ppd - (po - own_pd) + ms` after the play), or before `ppd`. Its fraction is the
    /// time since the play less `ppd`, plus `po - own_pd` (the start offset is not in it), taken as an unsigned 32-bit
    /// number (so a time still in the predelay counts as full volume), over `ms`, clamped to [0, 1].
    fn term(&self, play_at: u32, now: u32) -> Option<f32> {
        self.frac(play_at, now).map(lin2db)
    }

    /// the clamped fraction `term` takes in dB
    fn frac(&self, play_at: u32, now: u32) -> Option<f32> {
        if self.ms == 0 {
            return None;
        }
        let d = self.po.wrapping_sub(self.own_pd) as i32;
        if d > 0 && d > self.ms as i32 {
            return None;
        }
        let t = now.wrapping_sub(play_at);
        if t >= self.ppd.wrapping_sub(d as u32).wrapping_add(self.ms) || t < self.ppd {
            return None;
        }
        let x = (t.wrapping_sub(self.ppd).wrapping_add(d as u32) as f64 as f32) / (self.ms as f64 as f32);
        Some(x.clamp(0.0, 1.0))
    }
}

/// The timing of the original's sound object: +132 the play time, +66 waiting (1), +136 / +140 the release, +56 bit 0x04
/// stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Clock {
    play_at: Option<u32>,
    waiting: bool,
    rel: Option<(u32, u32)>,
    stopped: bool,
}

/// What a node's type does in an update: waits for its predelay, is played (its type's Play), runs (its Update), ends.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    Wait,
    Start,
    Run,
    End,
}

impl Clock {
    /// The common Play F-91431 (the first call: the play time is `now`, the attack applied, played at once without a
    /// total predelay `pd` (+80), else waiting) or the update driver F-68220: a stopped sound's release step
    /// (`release_step`, the end ends it when `flags` (+58) has 0x10), else the attack; then a waiting sound is played
    /// once `now >= play time + pd`. Returns what the type does and the volume after the attack or release.
    fn tick(&mut self, now: u32, pd: u32, at: Attack, flags: u8, vol: f32) -> (Act, f32) {
        let Some(t0) = self.play_at else {
            self.play_at = Some(now);
            self.waiting = pd != 0;
            let vol = at.term(now, now).map_or(vol, |d| d + vol);
            return (if self.waiting { Act::Wait } else { Act::Start }, vol);
        };
        let mut vol = vol;
        if let Some((a, b)) = self.rel {
            let (v, alive) = release_step(a, b, now, vol, flags);
            if !alive {
                return (Act::End, v);
            }
            vol = v;
        } else if !self.stopped {
            vol = at.term(t0, now).map_or(vol, |d| d + vol);
        }
        if self.waiting {
            if t0.wrapping_add(pd) > now {
                return (Act::Wait, vol);
            }
            self.waiting = false;
            return (Act::Start, vol);
        }
        (Act::Run, vol)
    }

    /// The stop request F-58485 at `now` with the release `rel` (the request's override else the sound's own
    /// ReleaseTime): from now to now + rel; true. Without one (-1): false (the sound ends once no child plays).
    fn stop(&mut self, now: u32, rel: i32) -> bool {
        self.stopped = true;
        if rel >= 0 {
            self.rel = Some((now, now.wrapping_add(rel as u32)));
            true
        } else {
            false
        }
    }
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum St {
    Waiting,
    Playing,
    Done,
}

enum Kind {
    Wave { wave: Option<WaveInfo>, clip: Option<Arc<Clip>>, voice: u64, pos_ms: f32, dur_ms: f32, looping: bool },
    Stream { clip: Option<Arc<Clip>>, headroom: f32, voice: u64, pos_ms: f32, dur_ms: f32, looping: bool },
    /// Wrapper, Multitrack, Streaming fallback, OnStop's child, approximations
    Group { children: Vec<Node> },
    /// `child_off`: the parent offset the child is played with (the start split of F-39812, or the restart offset)
    Looping { child: Box<Node>, child_hash: u32, count: i16, restart_ms: u16, plays: i32, child_off: Option<u32> },
    /// `ahead`: the children made at Init (all of them unless on demand, F-75822), in order; on demand (more than 8, or
    /// tristate 5) each is made when it is reached from `list[at]`. `cur_off`: the parent offset of the current child
    Sequential { list: Vec<u32>, at: usize, cur: Option<Box<Node>>, ahead: std::collections::VecDeque<Node>, on_demand: bool, cur_off: u32 },
    Retrig { child_hash: u32, count: i16, delay_ms: f32, next_ms: f32, made: i32, live: Vec<Node> },
    Envelope { a: f32, d: f32, s: f32, hold: f32, r: f32, curves: [u32; 3], child: Box<Node>, rel_from: Option<f32> },
    OnStop { child: Box<Node>, on_stop: u32, stopped: Option<Box<Node>> },
    /// TwinLoop: its two children cross-faded by `fade`; `clock` the ms it has been stepped (the update's time)
    Twin { fade: Twin, children: Vec<Node>, clock: f64, played: bool },
    Empty,
}

/// The rate limiter TwinLoop fades with (+208; reset F-96619, range F-94657, rates F-46548, step F-94292): moves `v`
/// toward a target by `up` / `down` per ms of the tick, clamped to [`lo`, `hi`]; flags 0x80 on, 0x40 snap, 0x20 clamp.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Fader {
    pub up: f32,
    pub down: f32,
    pub hi: f32,
    pub lo: f32,
    pub v: f32,
    pub last: u32,
    pub flags: u8,
}

impl Fader {
    /// F-94292: one step toward `target` at the tick `now` (ms)
    pub fn step(&mut self, target: f32, now: u32) -> f32 {
        if self.flags & 0x80 == 0 {
            return 0.0;
        }
        if self.flags & 0x40 != 0 {
            self.v = target;
            self.last = now;
            self.flags &= !0x40;
            return target;
        }
        let dt = now.wrapping_sub(self.last) as f64 as f32;
        let up = dt.mul_add(self.up, self.v);
        // fnmsubs: -(down x dt - v)
        let down = -self.down.mul_add(dt, -self.v);
        let mut f = if target - self.v >= 0.0 {
            if target - up >= 0.0 { up } else { target }
        } else if target - down >= 0.0 {
            target
        } else {
            down
        };
        if self.flags & 0x20 != 0 {
            f = if f - self.hi >= 0.0 { self.hi } else { f };
            f = if f - self.lo >= 0.0 { f } else { self.lo };
        }
        self.last = now;
        self.v = f;
        f
    }
}

/// A TwinLoop's cross-fade (Init F-73377, Play F-38194, Update F-52106, StartCrossfade F-27729): the fader goes to 1 or 0
/// and back, a new direction at each swap time, each fade lasting a random MinCrossfade..MaxCrossfade ms (F-46637), the
/// swaps a random MinSwap..MaxSwap ms apart (F-63616); child 0 gets the curve of the fader, child 1 of its complement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Twin {
    pub swap: (u32, u32),
    pub xfade: (u32, u32),
    /// +204: 0 EQUAL_POWER_RISE, 1 LINEAR_FALL
    pub linear: bool,
    /// +164: the next swap time
    pub next_swap: u32,
    /// +200: the fader's target
    pub target: f32,
    pub fader: Fader,
}

impl Twin {
    /// Init's part (F-73377): the fader reset (flags 0x40) and given its range (rates 1, clamp 0..1), then the first rate
    pub fn new(swap: (u32, u32), xfade: (u32, u32), linear: bool, mut draw: impl FnMut() -> u32) -> Twin {
        let fader = Fader { up: 1.0, down: 1.0, hi: 1.0, lo: 0.0, flags: 0x40 | 0xE0, ..Default::default() };
        let mut t = Twin { swap, xfade, linear, next_swap: 0, target: 0.0, fader };
        let r = t.rate(&mut draw);
        (t.fader.up, t.fader.down) = (r, r);
        t
    }

    /// F-46637: 1 / a random fade length in ms (F-60851), 1 when it is at most 1 ms
    fn rate(&self, draw: &mut impl FnMut() -> u32) -> f32 {
        let (a, b) = (self.xfade.0 as f32, self.xfade.1 as f32);
        let d = ((b - a) * (draw() & 0x7F_FFFF) as f32).mul_add(1.0 / 8_388_608.0, a);
        if d > 1.0 { 1.0 / d } else { 1.0 }
    }

    /// F-63616: `now` + a random whole ms in MinSwap..MaxSwap (F-62480: signed remainder by max - min + 1)
    fn swap_at(&self, now: u32, draw: &mut impl FnMut() -> u32) -> u32 {
        let (a, b) = (self.swap.0 as i32, self.swap.1 as i32);
        let v = (draw() & 0x7FFF_FFFF) as i32;
        (v.wrapping_rem((b - a).wrapping_add(1)) + a) as u32 + now
    }

    /// The children's levels (dB) at the fader's value (Play / Update): EQUAL_POWER_RISE sin(f pi/2) and
    /// sin((1 - f) pi/2), LINEAR_FALL f and 1 - f; 20 log10 as 20 x (log2 x 0.301), -100 dB under 1e-5.
    fn levels(&self, f: f32) -> (f32, f32) {
        use crate::physics::exact::guest_sin;
        let half_pi = f32::from_bits(0x3FC9_0FDB);
        let (g0, g1) = if self.linear {
            (f, 1.0 - f)
        } else {
            (guest_sin((f * half_pi) as f64) as f32, guest_sin(((1.0 - f) * half_pi) as f64) as f32)
        };
        let db = |g: f32| if g - f32::from_bits(0x3727_C5AC) >= 0.0 { (g.log2() * f32::from_bits(0x3E9A_1CAC)) * 20.0 } else { -100.0 };
        (db(g0), db(g1))
    }

    /// Play (F-38194) at `now`: the fader snaps to 1 (child 0 full, child 1 silent), the first swap time is drawn
    pub fn play(&mut self, now: u32, mut draw: impl FnMut() -> u32) -> (f32, f32) {
        self.target = 1.0;
        let f = self.fader.step(1.0, now);
        let l = self.levels(f);
        self.next_swap = self.swap_at(now, &mut draw);
        l
    }

    /// Update (F-52106) at `now`, while a child plays: past the swap time a new fade starts the other way (F-27729: a
    /// new rate, the target flipped, the next swap drawn); the fader steps and gives the children's levels
    pub fn update(&mut self, now: u32, mut draw: impl FnMut() -> u32) -> (f32, f32) {
        if now > self.next_swap {
            let r = self.rate(&mut draw);
            (self.fader.up, self.fader.down) = (r, r);
            self.target = 1.0 - self.target;
            self.next_swap = self.swap_at(now, &mut draw);
        }
        let f = self.fader.step(self.target, now);
        self.levels(f)
    }
}

struct Node {
    h: SoundHeader,
    vol: f32,
    pitch: i32,
    pan: i16,
    predelay_ms: f32,
    start_ms: f32,
    st: St,
    /// ms since it was played (now - +132) / since it started playing (from its start offset)
    t: f32,
    play_t: f32,
    /// the original object's timing (F-91431 / F-68220): play time, waiting, release, stopped
    clock: Clock,
    /// the envelope's time origin in `t` (own predelay - the parent's offset, F-82485 +176), set when it is played
    env0: f32,
    /// the total predelay and start offset of the node's play (+80 / +84, F-90115; 0 until it plays)
    tot: (u32, u32),
    /// Streaming: its type duration (+200 when set, else the bank's longest wave, else -1; F-96939)
    ty_ms: i32,
    kind: Kind,
}

struct Instance {
    id: u64,
    /// the sounds.dat object played (0: a converted wave)
    hash: u32,
    root: Node,
    req: Req,
    /// the source's place at the last Doppler sample, and the place the voice is heard at until the next drawn frame
    /// (the frame's: what the frame's camera was made against - SoundLib::frame_sample)
    prev_pos: Option<Vec3>,
    frame_pos: Option<Vec3>,
    doppler: f32,
    stop: bool,
    /// the request's release override (the request entry +192, -1 = none): a stop takes it instead of each sound's own
    /// ReleaseTime (F-58485)
    stop_ms: i32,
    /// the audio timer the sound runs on (sound +65 >> 2: request +56, -1 = the root sound's category's tristate pair 1 -
    /// docs/reference/audio/SOUND_TIMER_RULE.md; F-33564): every request but the speech take's has -1, so the root's
    /// header category's (`SoundLib::cat_timer`); `set_timer` for a request that gives one
    timer: Option<u8>,
}

pub struct SoundLib {
    root: PathBuf,
    snd: Dat,
    /// game.dat: the game objects (car audio profiles, cross-fader tables, districts ...)
    pub game: Dat,
    curves: HashMap<u32, Curve>,
    cats: HashMap<u32, Category>,
    /// bank name hash -> the bank's folder below the audio root (`vehicles/vehicle_aux`)
    bank_dir: HashMap<u32, String>,
    /// bank name hash -> a mod's wave bank folder on disk (banks the game's audio does not have; modfs.rs)
    mod_banks: HashMap<u32, PathBuf>,
    waves: HashMap<u32, Arc<HashMap<u32, WaveInfo>>>,
    /// stream channel hash -> (converted .mp3, the stream's lowest channel headroom)
    /// a stream channel: its file, headroom and its bank's longest wave in whole ms (F-46175)
    stream_of: Option<HashMap<u32, (PathBuf, i16, i32)>>,
    /// (with the count of `clip_of` at its last use)
    clips: HashMap<PathBuf, (Option<Arc<Clip>>, u64)>,
    clip_uses: u64,
    rng: SharedMwc,
    /// the audio controller's three timers (F-46276: engine+572 + 12 k: time, rate, paused): a sound on a paused timer is
    /// not updated (F-89113 skips it; its voice holds). Timer 0 is paused while the game is (F-99037, `sound.rs`).
    pub timer_paused: [bool; 3],
    /// their times in ms (the sounds' `now`)
    timer_ms: [f64; 3],
    /// each category's timer (`audcfg::category_timers`: categories.dat tristate pair 1 from BASE down)
    cat_timer: HashMap<u32, u8>,
    /// Randomized objects' history (kept per object, as the original writes it into the metadata)
    history: HashMap<u32, (usize, Vec<u8>)>,
    insts: Vec<Instance>,
    next: u64,
    next_voice: u64,
    /// each wave voice's slot state for the voice manager (1 playing, 2 stopping; waiting when absent)
    voice_state: HashMap<u64, u8>,
    pub listener: (Vec3, Vec3, Vec3),
    /// the game ticks since the last Doppler sample and the camera at it (frame_sample)
    sample_ticks: usize,
    sample_cam: Option<Vec3>,
    /// the reverb level and room size (env +5100 / +5104, set every frame by the probes of F-26357 -
    /// BUS_GRAPH_RULE.md 2.5)
    pub reverb_env: (f32, f32),
    /// the occluders' outputs this frame (volume, cutoff in Hz) by slot, as the requests name them (`Req::occluder`)
    occlusion: Vec<[f32; 2]>,
    /// effect chain settings set this frame, handed to the mixer by `update`
    chain_params: Vec<(u32, crate::audio::ChainParams)>,
    /// categories.dat, and the run-time controllers on its categories (dB by category hash): the five volume
    /// sliders and the camera table (docs/reference/audio/VOLUME_RULE.md). A voice at 0 dB in all reaches full scale.
    cat_dat: Dat,
    sliders: std::collections::HashMap<u32, f32>,
    camera: std::collections::HashMap<u32, f32>,
    /// the camera table's LPF / HPF per category (Hz; 23900 / 0 = open)
    camera_filt: std::collections::HashMap<u32, (u16, u16)>,
    /// the ambience manager's speed fade on the AMBIENCE category (its controller +840, F-54370), dB
    speed_fade: f32,
    /// ramped category controllers (category hash -> current dB, target dB, dB per second): the weather mix
    ramps: std::collections::HashMap<u32, (f32, f32, f32)>,
    /// (mode, bike) the camera table was last applied for
    cam_mode: Option<(i32, bool)>,
    /// dynamic mixer scenes (game.dat type 23): name hash -> state
    scenes: HashMap<u32, Scene>,
    /// MANAGER_OPPONENT_GAINS: the racers' presence, dB on its categories (applied at once, every frame)
    presence: (Vec<u32>, f32),
    presence_db: f32,
    /// MANAGER_PLAYER_SPEED (F-15595): each record and its controller's linear level
    speed: Option<Vec<(SpeedRec, f32)>>,
    /// DYNAMIC_MIXER_SLOWDOWN's SFX_HARD record (pitch ratio 0.1, its level unchanged): the controller's ratio now and
    /// its target (career2 `audioramp/notes.md`: 1 a second either way; the PLAYER_VEHICLES and AMBIENCE records are
    /// `Audio::set_time_slow`'s)
    slow_hard: (f32, f32),
    /// the scenes on at the end of the last update (the blocking test)
    scenes_on_hint: std::collections::HashSet<u32>,
}

/// A MANAGER_PLAYER_SPEED record (game.dat type 39, 28 bytes from +11): category, speed input A's and B's low / high,
/// the level at the low / high end (s16 mB) and the slew rate (dB per second).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpeedRec {
    pub cat: u32,
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub mb: [i16; 2],
    pub rate: f32,
}

impl SpeedRec {
    pub(crate) fn table(o: &[u8]) -> Vec<SpeedRec> {
        let f = |at: usize| f32::from_be_bytes(o[at..at + 4].try_into().unwrap());
        (0..o.len().saturating_sub(11) / 28)
            .map(|i| {
                let at = 11 + 28 * i;
                SpeedRec {
                    cat: u32::from_be_bytes(o[at..at + 4].try_into().unwrap()),
                    a: [f(at + 4), f(at + 8)],
                    b: [f(at + 12), f(at + 16)],
                    mb: [i16::from_be_bytes([o[at + 20], o[at + 21]]), i16::from_be_bytes([o[at + 22], o[at + 23]])],
                    rate: f(at + 24),
                }
            })
            .collect()
    }
}

/// F-15595 for one record: the inputs (0 in camera mode 5, the cinematic) at or under both lows give the low level, at
/// or over either high the high level, else low + max(tA, tB) x (high - low) with tA = (A - Alo) / (Ahi - Alo) and tB
/// = B - Blo / Bhi - Blo (the original's arithmetic); the controller's level (as dB, -100 under 1e-5) moves toward it
/// at rate x dt and is set again (SetVolumeDB F-10740: dB2lin). Returns the new linear level.
pub(crate) fn speed_step(r: &SpeedRec, lin: f32, mode: i32, sa: f32, sb: f32, dt: f32) -> f32 {
    let (a, b) = if mode == 5 { (0.0, 0.0) } else { (sa, sb) };
    let target = if !(a > r.a[0]) && !(b > r.b[0]) {
        r.mb[0] as f32 * 0.01
    } else if !(a >= r.a[1]) && !(b >= r.b[1]) {
        let ta = (a - r.a[0]) / (r.a[1] - r.a[0]);
        let tb = b - r.b[0] / r.b[1] - r.b[0];
        let t = if ta - tb >= 0.0 { ta } else { tb };
        let lo = r.mb[0] as f32;
        t.mul_add(r.mb[1] as f32 - lo, lo) * 0.01
    } else {
        r.mb[1] as f32 * 0.01
    };
    // (vlogefp as the recomp runs it: f32 log2, as the TwinLoop's levels)
    let cur = if lin - f32::from_bits(0x3727_C5AC) >= 0.0 { (lin.log2() * f32::from_bits(0x3E9A_1CAC)) * 20.0 } else { -100.0 };
    let db = if !(cur >= target) {
        let v = dt.mul_add(r.rate, cur);
        if !(v <= target) { target } else { v }
    } else if !(cur <= target) {
        let v = -dt.mul_add(r.rate, -cur);
        if !(v >= target) { target } else { v }
    } else {
        cur
    };
    crate::busfx::db2lin(db)
}

/// A dynamic mixer scene (game.dat `DYNAMIC_MIXER_*`, type 23: +14 enter, +18 exit, +22 loop, +26 stream sound, +30
/// fade in s, +34 fade out s, +38 flags, +39 time, +43 blocking scene, +47 count, 14-byte records {category, s16 mB,
/// f32 x, u16 LPF, u16 HPF}; `reference/audio/MIXER_SCENES_RULE.md`). A record's gain moves linearly from 1 to
/// `1 + (dB2lin - 1) x strength` over the fade.
struct Scene {
    enter: u32,
    exit: u32,
    looped: u32,
    stream: u32,
    fade_in: f32,
    fade_out: f32,
    /// 0x1 continuous (no fade, the strength re-applied every frame), 0x2 auto-stop after `time`, 0x8 exclusive
    flags: u8,
    time: f32,
    block: u32,
    recs: Vec<(u32, f32)>,
    /// the records' LPF and HPF (Hz): at strength s the controller takes 23900 + (LPF - 23900) s and HPF s
    filts: Vec<(u32, u16, u16)>,
    on: bool,
    strength: f32,
    elapsed: f32,
    /// 0..1: the fade
    level: f32,
    /// the enter, loop and stream sounds' slots
    playing: [Option<Handle>; 3],
}

/// The volume sliders' categories (F-22027 A-61259): effects, music, voice, player vehicle, cutscene.
pub const SLIDER_CATEGORIES: [&str; 5] = ["SFX", "MUSIC", "SPEECH", "PLAYER_VEHICLES", "CUTSCENES"];

/// Voice id bit for the player vehicle's sounds: the mixer's time-slow (Zone, race end) applies to them.
pub const SLOWED: u64 = 1 << 40;

/// F-39461's field of an AMBIENT_PED_<type> object for a reaction of `kind` (its switch): 0 +14 (QUICK), 1 +10 (SLOW),
/// 2 +18 (EXPENSIVE_CAR) or, when the player's entity is a bike (+1945), +26 (EXPENSIVE_BIKE), 3 +22 (DAMAGED) or,
/// on a bike, +30. Another kind plays the global default sound (not used: the callers pass 0 .. 3).
pub fn ped_reaction_field(kind: u32, bike: bool) -> Option<usize> {
    match kind {
        0 => Some(14),
        1 => Some(10),
        2 => Some(if bike { 26 } else { 18 }),
        3 => Some(if bike { 30 } else { 22 }),
        _ => None,
    }
}

/// F-39461's sound word from the object it found (its type's, else the fallback; none: the original reads through a
/// null object, the port plays nothing): the object's unaligned big-endian word at `ped_reaction_field`.
pub fn ped_reaction_pick(obj: Option<&[u8]>, kind: u32, bike: bool) -> Option<u32> {
    let k = ped_reaction_field(kind, bike)?;
    Some(u32::from_be_bytes(obj?.get(k..k + 4)?.try_into().ok()?))
}

/// The walkers' reaction sounds (F-39461's mcAmbientPedAudioEntity pool): each plays where its walker stands, the
/// entity's position pointer (+116) followed every frame.
#[derive(Default)]
pub struct PedVoices {
    /// (walker, sound, where it plays, the entity's occluder slot plus one)
    playing: Vec<((usize, u32), Handle, Vec3, u16)>,
}

impl PedVoices {
    /// Whether the walker has an entity: its sound still plays.
    pub fn busy(&self, lib: &SoundLib, key: (usize, u32)) -> bool {
        self.playing.iter().any(|(k, h, ..)| *k == key && lib.playing(*h))
    }

    /// A walker's reaction sound started (the sound's hash, `SoundLib::ped_reaction`) with its entity's occluder slot
    /// plus one (the request template +96 carries it at +32). F-48911 gives no entity to a walker that has one: here
    /// while its sound plays (approximate - the pool's size, the sound manager's +4, and when an entity lets its walker
    /// go are not decoded).
    pub fn start(&mut self, lib: &mut SoundLib, key: (usize, u32), hash: u32, pos: Vec3, occ: u16) -> Option<Handle> {
        if self.busy(lib, key) {
            return None;
        }
        let h = lib.play_hash(hash, Req { occluder: occ, ..Req::at(pos) })?;
        self.playing.push((key, h, pos, occ));
        Some(h)
    }

    /// Each playing sound follows its walker (`pos_of`; a walker gone leaves its sound where it last stood). The
    /// walkers whose sound is over are handed back.
    pub fn follow(&mut self, lib: &mut SoundLib, pos_of: impl Fn((usize, u32)) -> Option<Vec3>) -> Vec<(usize, u32)> {
        let mut gone = Vec::new();
        self.playing.retain(|(k, h, ..)| {
            let on = lib.playing(*h);
            if !on {
                gone.push(*k);
            }
            on
        });
        for (k, h, at, occ) in self.playing.iter_mut() {
            if let Some(p) = pos_of(*k).filter(|p| p != at) {
                *at = p;
                lib.set(*h, Req { occluder: *occ, ..Req::at(p) });
            }
        }
        gone.retain(|k| !self.playing.iter().any(|(j, ..)| j == k));
        gone
    }

    /// The walkers with an entity and where their sounds play.
    pub fn live(&self) -> impl Iterator<Item = ((usize, u32), Vec3)> + '_ {
        self.playing.iter().map(|(k, _, p, _)| (*k, *p))
    }
}

const VOICE_BASE: u64 = 0x4000_0000;
const NONE: u32 = 0xFFFF_FFFF;

impl SoundLib {
    /// `root` = the converted audio (`audio_pc`), `cfg` = `audio/X360/config` of the extracted cache.
    pub fn load(root: &Path, cfg: &Path) -> Option<SoundLib> {
        let mut snd = Dat::load(&cfg.join("sounds.dat"))?;
        let crv = Dat::load(&cfg.join("curves.dat"))?;
        let cat = Dat::load(&cfg.join("categories.dat"))?;
        let mut game = Dat::load(&cfg.join("game.dat"))?;
        // a mod's own sound and game objects join the game's; the game's keep their names (modfs.rs,
        // docs/reference/mods/manifest.md section 7)
        let mods = crate::modfs::set();
        for (rel, dat) in [(mcla_mods::SOUNDS_DAT, &mut snd), (mcla_mods::GAME_DAT, &mut game)] {
            for (id, b) in mods.map(|m| m.read_each(rel)).unwrap_or_default() {
                match Dat::parse(&b) {
                    Some(d) => {
                        let skipped = dat.extend_from(&d);
                        log::info!("audio: mod {id}: {} objects of {rel} added ({} already the game's: not used)", d.len() - skipped.len(), skipped.len());
                    }
                    None => log::warn!("audio: mod {id}: {rel} is not a readable .dat file"),
                }
            }
        }
        let curves = crv.objects.iter().filter_map(|(n, _, _)| crv.get_name(n).map(|o| (joaat(n), Curve::parse(o)))).collect();
        let cats: HashMap<u32, Category> = audcfg::resolve_categories(&cat).into_iter().collect();
        // bank folders: each part of the bank's path by name, or by its hash where the archive lost the name
        let mut bank_dir = HashMap::new();
        let mut mod_banks = HashMap::new();
        for b in &snd.banks {
            let mut dir = PathBuf::new();
            let mut rel = Vec::new();
            for part in b.split(['\\', '/']) {
                let low = part.to_ascii_lowercase();
                let hashed = format!("0x{:08x}", joaat(part));
                let pick = if root.join(&dir).join(&low).exists() { low } else { hashed };
                dir.push(&pick);
                rel.push(pick);
            }
            let rel = rel.join("/");
            // (a bank the game's audio does not have: a mod's wave bank, its folder on disk)
            if let Some(d) = mods.filter(|_| !root.join(&rel).is_dir()).and_then(|m| m.wave_bank_dir(b)) {
                log::info!("audio: bank {b} from a mod ({})", d.display());
                mod_banks.insert(joaat(b), d);
            }
            bank_dir.insert(joaat(b), rel);
        }
        log::info!("audio: sounds.dat {} sounds, {} curves, {} categories, {} banks", snd.len(), crv.len(), cat.len(), snd.banks.len());
        Some(SoundLib {
            root: root.to_path_buf(),
            snd,
            game,
            curves,
            cats,
            bank_dir,
            mod_banks,
            waves: HashMap::new(),
            stream_of: None,
            clips: HashMap::new(),
            clip_uses: 0,
            rng: SharedMwc,
            timer_paused: [false; 3],
            timer_ms: [0.0; 3],
            cat_timer: audcfg::category_timers(&cat).into_iter().collect(),
            history: HashMap::new(),
            insts: Vec::new(),
            next: 1,
            next_voice: 0,
            voice_state: HashMap::new(),
            listener: (Vec3::ZERO, Vec3::X, Vec3::NEG_Z),
            sample_ticks: 0,
            sample_cam: None,
            cat_dat: cat,
            sliders: Default::default(),
            camera: Default::default(),
            camera_filt: Default::default(),
            reverb_env: (0.0, 0.0),
            occlusion: Vec::new(),
            chain_params: Vec::new(),
            ramps: Default::default(),
            cam_mode: None,
            scenes: HashMap::new(),
            presence: (Vec::new(), 0.0),
            speed_fade: 0.0,
            presence_db: 0.0,
            speed: None,
            slow_hard: (1.0, 1.0),
            scenes_on_hint: Default::default(),
        })
    }

    pub fn exists(&self, name: &str) -> bool {
        self.snd.contains(joaat(name))
    }

    /// F-39461's sound for a walker's reaction of `kind`: its type's sound object in game.dat (the type's szAudioType,
    /// +176; else AMBIENT_PED_FEMALE_01 / AMBIENT_PED_MALE_01 by the type's sex +109 - names game.dat does not have,
    /// and every ambient type's own object is there) and that object's sound field `ped_reaction_field`.
    pub fn ped_reaction(&self, audio_type: &str, female: bool, kind: u32, bike: bool) -> Option<u32> {
        let own = Some(audio_type).filter(|t| !t.is_empty()).and_then(|t| self.game.get_name(t));
        let fallback = || self.game.get_name(if female { "AMBIENT_PED_FEMALE_01" } else { "AMBIENT_PED_MALE_01" });
        ped_reaction_pick(own.or_else(fallback), kind, bike)
    }
    pub fn exists_hash(&self, h: u32) -> bool {
        self.snd.contains(h)
    }
    pub fn name_of(&self, h: u32) -> Option<&str> {
        self.snd.name_of(h)
    }
    /// A curve of curves.dat by name hash.
    pub fn curve(&self, h: u32) -> Option<&Curve> {
        self.curves.get(&h)
    }
    pub fn category(&self, name: &str) -> Option<Category> {
        self.cats.get(&joaat(name)).copied()
    }
    /// The five AUDIO OPTIONS sliders, 0..10 each (effects, music, voice, player vehicle, cutscene): gain = value x 0.1
    /// on the slider's category (F-54340, F-29638), as dB (-100 at 0).
    pub fn set_sliders(&mut self, values: [u8; 5]) {
        let mut changed = false;
        for (name, v) in SLIDER_CATEGORIES.iter().zip(values) {
            let db = lin2db((v.min(10) as f32 * 0.1).clamp(0.0, 1.0));
            if self.sliders.insert(joaat(name), db) != Some(db) {
                changed = true;
            }
        }
        if changed {
            self.resolve();
        }
    }

    /// The audio camera mode (F-33257 with game.dat MANAGER_CAMERA): 0 bumper, 1 driver, 2 near, 3 medium, 4 far,
    /// 5 cinematic, 6 other cameras, -1 / 7 none; `bike` picks the bike groups of modes 0 / 1. Volume (F-10740), LPF and
    /// HPF (F-27208 / F-23310) of every record.
    pub fn set_camera_mode(&mut self, mode: i32, bike: bool) {
        if self.cam_mode == Some((mode, bike)) {
            return;
        }
        self.cam_mode = Some((mode, bike));
        self.camera.clear();
        self.camera_filt.clear();
        let group = match (mode, bike) {
            (0, false) => Some(0),
            (0, true) => Some(1),
            (1, false) => Some(2),
            (1, true) => Some(3),
            (2..=6, _) => Some(mode as usize + 2),
            _ => None,
        };
        if let (Some(g), Some(o)) = (group, self.game.get_name("MANAGER_CAMERA")) {
            let mut at = 19;
            while at + 58 <= o.len() {
                let h = u32::from_be_bytes(o[at..at + 4].try_into().unwrap());
                let mb = i16::from_be_bytes(o[at + 4 + 6 * g..at + 6 + 6 * g].try_into().unwrap());
                self.camera.insert(h, mb as f32 * 0.01);
                let u = |k: usize| u16::from_be_bytes([o[at + k + 6 * g], o[at + k + 1 + 6 * g]]);
                self.camera_filt.insert(h, (u(6), u(8)));
                at += 58;
            }
        }
        self.resolve();
    }

    /// The ambience manager's AMBIENCE controller (F-54370): set each frame, resolved only on a change.
    pub fn set_speed_fade(&mut self, db: f32) {
        if self.speed_fade != db {
            self.speed_fade = db;
            self.resolve();
        }
    }

    /// Turn a dynamic mixer scene on (strength 1) or off (with its exit sound) by name, as a fixed slot: starting a
    /// running scene does nothing.
    pub fn scene(&mut self, name: &str, on: bool) {
        self.scene_at(name, on, 1.0);
    }

    /// The same with a strength (0..1); a continuous scene (flag 0x1) takes a new strength while it runs.
    pub fn scene_at(&mut self, name: &str, on: bool, strength: f32) {
        let h = joaat(name);
        if !self.scenes.contains_key(&h) {
            if !on {
                return;
            }
            let Some(o) = self.game.get(h).filter(|o| o.first() == Some(&23)).map(|o| o.to_vec()) else { return };
            let u32at = |at: usize| o.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).unwrap_or(0);
            let n = o.get(47).copied().unwrap_or(0) as usize;
            let recs = (0..n).map(|i| (u32at(48 + 14 * i), i16::from_be_bytes([o[52 + 14 * i], o[53 + 14 * i]]) as f32 * 0.01)).collect();
            let u16at = |at: usize| o.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0);
            let filts = (0..n).map(|i| (u32at(48 + 14 * i), u16at(58 + 14 * i), u16at(60 + 14 * i))).filter(|r| r.1 != 23900 || r.2 != 0).collect();
            let null = joaat("NULL_SOUND");
            let snd = |at: usize| Some(u32at(at)).filter(|h| *h != 0 && *h != null).unwrap_or(NONE);
            self.scenes.insert(
                h,
                Scene {
                    enter: snd(14),
                    exit: snd(18),
                    looped: snd(22),
                    stream: snd(26),
                    fade_in: f32::from_bits(u32at(30)),
                    fade_out: f32::from_bits(u32at(34)),
                    flags: o.get(38).copied().unwrap_or(0),
                    time: f32::from_bits(u32at(39)),
                    block: u32at(43),
                    recs,
                    filts,
                    on: false,
                    strength: 1.0,
                    elapsed: 0.0,
                    level: 0.0,
                    playing: [None; 3],
                },
            );
        }
        if on {
            self.scene_start(h, strength);
        } else {
            self.scene_stop(h, true);
        }
    }

    /// A scene stopped without its exit sound (F-55770(h, 0): the camera table's apply on NAVMAP).
    pub fn scene_cut(&mut self, name: &str) {
        self.scene_stop(joaat(name), false);
    }

    pub fn scene_on(&self, name: &str) -> bool {
        self.scenes.get(&joaat(name)).is_some_and(|s| s.on)
    }

    /// F-38026: refused while its blocking scene runs or another exclusive scene runs; the enter, loop and stream
    /// sounds on the start; an exclusive scene stops the others.
    fn scene_start(&mut self, h: u32, strength: f32) {
        let sc = &self.scenes[&h];
        if sc.on {
            if sc.flags & 1 != 0 {
                self.scenes.get_mut(&h).unwrap().strength = strength.clamp(0.0, 1.0);
            }
            return;
        }
        let excl = sc.flags & 8 != 0;
        let (block, snds) = (sc.block, [sc.enter, sc.looped, sc.stream]);
        if self.scenes.get(&block).is_some_and(|b| b.on) || (!excl && self.scenes.values().any(|s| s.on && s.flags & 8 != 0)) {
            return;
        }
        let mut hs = [None; 3];
        for (k, s) in snds.into_iter().enumerate() {
            if s != NONE {
                hs[k] = self.play_hash(s, Req::default());
            }
        }
        let sc = self.scenes.get_mut(&h).unwrap();
        sc.on = true;
        sc.strength = strength.clamp(0.0, 1.0);
        sc.elapsed = 0.0;
        sc.playing = hs;
        if sc.flags & 1 != 0 {
            sc.level = 1.0;
        }
        if excl {
            let others: Vec<u32> = self.scenes.iter().filter(|(k, s)| **k != h && s.on && s.flags & 8 == 0).map(|x| *x.0).collect();
            for o in others {
                self.scene_stop(o, true);
            }
        }
    }

    /// F-55770: the enter, loop and stream sounds stop, the exit sound plays (`exit`), the records fade back.
    fn scene_stop(&mut self, h: u32, exit: bool) {
        let Some(sc) = self.scenes.get_mut(&h).filter(|s| s.on) else { return };
        sc.on = false;
        let ps = std::mem::take(&mut sc.playing);
        let x = sc.exit;
        if sc.flags & 1 != 0 {
            sc.level = 0.0;
        }
        for p in ps.into_iter().flatten() {
            self.stop(p);
        }
        if exit && x != NONE {
            self.play_hash(x, Req::default());
        }
    }

    /// MANAGER_OPPONENT_GAINS (F-40102 @A-21706): `w` = the racers' presence weight; above 1 its categories get
    /// -3 dB x log2(w) (the object's s16 mB per doubling), at once.
    pub fn set_presence(&mut self, w: f32) {
        if self.presence.0.is_empty() {
            let Some(o) = self.game.get_name("MANAGER_OPPONENT_GAINS").map(|o| o.to_vec()) else { return };
            let n = o.get(18).copied().unwrap_or(0) as usize;
            let mut cats = Vec::new();
            let mut mb = 0.0;
            for i in 0..n {
                let at = 19 + 6 * i;
                let Some(b) = o.get(at..at + 6) else { break };
                cats.push(u32::from_be_bytes(b[0..4].try_into().unwrap()));
                mb = i16::from_be_bytes([b[4], b[5]]) as f32;
            }
            self.presence = (cats, mb * 0.01);
            self.presence_db = mb * 0.01;
        }
        let db = if w > 1.0 { self.presence_db * w.log2() } else { 0.0 };
        if db != self.presence.1 {
            self.presence.1 = db;
            self.resolve();
        }
    }

    /// A category's runtime volume moved to `db` over `secs` (F-83232: linear in dB, I).
    pub fn ramp_category(&mut self, name: &str, db: f32, secs: f32) {
        let e = self.ramps.entry(joaat(name)).or_insert((0.0, 0.0, 0.0));
        if e.1 != db {
            e.1 = db;
            e.2 = if secs > 0.0 { (db - e.0).abs() / secs } else { f32::INFINITY };
        }
    }

    /// AudioManager_SetActiveTimeSlow for SFX_HARD (F-38026 / F-55770): its pitch ratio ramped to 0.1, or
    /// back to 1, at 1 a second (K / fade time, F-83565 / F-44036).
    pub fn set_time_slow(&mut self, on: bool) {
        self.slow_hard.1 = if on { 0.1 } else { 1.0 };
    }

    fn step_scenes(&mut self, dt: f32) {
        let mut changed = false;
        // MANAGER_PLAYER_SPEED (F-15595, every mixer update): the speed inputs (+1748 / +1752) are only ever set to 0 in
        // the game (F-35488 and the timer pause in F-99037), so every record follows its low end
        if self.speed.is_none() {
            self.speed = Some(self.game.get_name("MANAGER_PLAYER_SPEED").map_or(Vec::new(), |o| SpeedRec::table(o).into_iter().map(|r| (r, 1.0)).collect()));
        }
        let mode = self.cam_mode.map_or(-1, |m| m.0);
        for (r, lin) in self.speed.iter_mut().flatten() {
            let v = speed_step(r, *lin, mode, 0.0, 0.0, dt);
            if v != *lin {
                *lin = v;
                changed = true;
            }
        }
        let (cur, to) = self.slow_hard;
        if cur != to {
            self.slow_hard.0 = if to > cur { (cur + dt).min(to) } else { (cur - dt).max(to) };
            changed = true;
        }
        for e in self.ramps.values_mut() {
            if e.0 != e.1 {
                let step = e.2 * dt;
                e.0 = if e.1 > e.0 { (e.0 + step).min(e.1) } else { (e.0 - step).max(e.1) };
                changed = true;
            }
        }
        // the per-frame scene update (F-76740): auto-stop, blocking scene, the exclusive scene, loops kept alive
        let mut stops: Vec<(u32, bool)> = Vec::new();
        let excl = self.scenes.iter().find(|(_, s)| s.on && s.flags & 8 != 0).map(|x| *x.0);
        let mut alive: Vec<(u32, usize, u32)> = Vec::new();
        for (h, sc) in &mut self.scenes {
            if !sc.on {
                continue;
            }
            sc.elapsed += dt;
            if sc.flags & 2 != 0 && sc.elapsed >= sc.time {
                stops.push((*h, true));
            } else if self.scenes_on_hint.contains(&sc.block) {
                stops.push((*h, false));
            } else if excl.is_some_and(|e| e != *h) && sc.flags & 8 == 0 {
                stops.push((*h, true));
            }
            for (k, s) in [(1, sc.looped), (2, sc.stream)] {
                if s != NONE {
                    alive.push((*h, k, s));
                }
            }
            if sc.flags & 1 != 0 {
                changed = true;
            }
        }
        for (h, k, s) in alive {
            let cur = self.scenes[&h].playing[k];
            if cur.map_or(true, |p| !self.playing(p)) {
                let n = self.play_hash(s, Req::default());
                self.scenes.get_mut(&h).unwrap().playing[k] = n;
            }
        }
        for (h, exit) in stops {
            self.scene_stop(h, exit);
        }
        for sc in self.scenes.values_mut() {
            if sc.flags & 1 != 0 {
                continue;
            }
            let (target, time) = if sc.on { (1.0, sc.fade_in) } else { (0.0, sc.fade_out) };
            if sc.level != target {
                let step = if time > 0.0 { dt / time } else { 1.0 };
                sc.level = if target > sc.level { (sc.level + step).min(1.0) } else { (sc.level - step).max(0.0) };
                changed = true;
            }
        }
        self.scenes_on_hint = self.scenes.iter().filter(|(_, s)| s.on).map(|x| *x.0).collect();
        if changed {
            self.resolve();
        }
    }

    fn resolve(&mut self) {
        let mut rt = self.sliders.clone();
        for (h, db) in &self.camera {
            *rt.entry(*h).or_insert(0.0) += db;
        }
        for (h, e) in &self.ramps {
            *rt.entry(*h).or_insert(0.0) += e.0;
        }
        if self.speed_fade != 0.0 {
            *rt.entry(joaat("AMBIENCE")).or_insert(0.0) += self.speed_fade;
        }
        for (r, lin) in self.speed.iter().flatten() {
            *rt.entry(r.cat).or_insert(0.0) += lin2db(*lin);
        }
        for sc in self.scenes.values().filter(|s| s.level > 0.0) {
            for (h, db) in &sc.recs {
                // the controller's gain moves linearly from 1 to the scene's at its strength
                *rt.entry(*h).or_insert(0.0) += lin2db(1.0 + (db2lin(*db) - 1.0) * sc.strength * sc.level);
            }
        }
        if self.presence.1 != 0.0 {
            for h in &self.presence.0 {
                *rt.entry(*h).or_insert(0.0) += self.presence.1;
            }
        }
        let mut cents = std::collections::HashMap::new();
        if self.slow_hard.0 != 1.0 {
            cents.insert(joaat("SFX_HARD"), (1200.0 * self.slow_hard.0.log2()) as i32);
        }
        // the controllers' filters (F-27208 / F-23310 clamp to 20..23900 Hz; F-61464 takes the lowest LPF and the
        // highest HPF of a category's controllers, an HPF at 20 Hz or below is none)
        let mut filt: std::collections::HashMap<u32, (f32, f32)> = std::collections::HashMap::new();
        let mut put = |h: u32, l: f32, hp: f32| {
            let e = filt.entry(h).or_insert((23900.0, 20.0));
            e.0 = e.0.min(l.clamp(20.0, 23900.0));
            e.1 = e.1.max(hp.clamp(20.0, 23900.0));
        };
        for (h, (l, hp)) in &self.camera_filt {
            put(*h, *l as f32, *hp as f32);
        }
        for sc in self.scenes.values().filter(|s| s.level > 0.0) {
            let s = sc.strength * sc.level;
            for (h, l, hp) in &sc.filts {
                put(*h, 23900.0 + (*l as f32 - 23900.0) * s, *hp as f32 * s);
            }
        }
        // (back to Hz from the log2 domain, truncated)
        let hz = |v: f32| (23900.0 * (v / 23900.0).log2().exp2()) as u16;
        let filt = filt.into_iter().map(|(h, (l, hp))| (h, (hz(l), if hp > 20.0 { hz(hp) } else { 0 }))).collect();
        self.cats = audcfg::resolve_categories_rt(&self.cat_dat, &rt, &cents, &filt).into_iter().collect();
    }

    pub fn rand_float(&mut self, a: f32, b: f32) -> f32 {
        self.rng.float(a, b)
    }
    pub fn rand_int(&mut self, a: i32, b: i32) -> i32 {
        self.rng.int(a, b)
    }
    /// A curve's value by name hash; `None` when curves.dat has no such curve.
    pub fn curve_eval(&self, h: u32, x: f32) -> Option<f32> {
        self.curves.get(&h).map(|c| c.eval(x))
    }

    fn bank_waves(&mut self, bank: u32) -> Option<Arc<HashMap<u32, WaveInfo>>> {
        if let Some(w) = self.waves.get(&bank) {
            return Some(w.clone());
        }
        let dir = match self.mod_banks.get(&bank) {
            Some(d) => d.clone(),
            None => self.root.join(self.bank_dir.get(&bank)?),
        };
        let mut m = HashMap::new();
        if let Ok(t) = std::fs::read_to_string(dir.join("_index.tsv")) {
            for l in t.lines() {
                let c: Vec<&str> = l.split('\t').collect();
                let (Some(name), Some(h)) = (c.first(), c.get(1).and_then(|h| u32::from_str_radix(h, 16).ok())) else { continue };
                let loop_start = c.get(5).and_then(|v| v.parse().ok()).unwrap_or(-1);
                let headroom = c.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);
                let (samples, rate) = (c.get(2).and_then(|v| v.parse::<u32>().ok()), c.get(3).and_then(|v| v.parse::<u32>().ok()).filter(|r| *r > 0));
                let ms = |n: Option<u32>| Some(wave_ms(n?, rate?));
                let loop_ms = if loop_start >= 0 { ms(Some(loop_start as u32)) } else { None };
                m.insert(h, WaveInfo { file: dir.join(format!("{name}.wav")), loop_start, headroom, dur_ms: ms(samples), loop_ms });
            }
        }
        let a = Arc::new(m);
        self.waves.insert(bank, a.clone());
        Some(a)
    }

    /// Read the stream channels' index now (otherwise at the first stream played).
    pub fn index_streams(&mut self) {
        self.stream_index();
    }

    fn stream_index(&mut self) -> &HashMap<u32, (PathBuf, i16, i32)> {
        if self.stream_of.is_none() {
            let mut m = HashMap::new();
            fn walk(dir: &Path, m: &mut HashMap<u32, (PathBuf, i16, i32)>) {
                let Ok(rd) = std::fs::read_dir(dir) else { return };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, m);
                    } else if let Some(mp3) = p.to_str().and_then(|s| s.strip_suffix(".txt")).filter(|s| s.ends_with(".mp3")) {
                        let Ok(t) = std::fs::read_to_string(&p) else { continue };
                        let rows: Vec<(u32, i16, i32)> = t.lines().filter_map(|l| {
                            let c: Vec<&str> = l.split('\t').collect();
                            let n = |k: usize| c.get(k).and_then(|v| v.parse::<u32>().ok());
                            let ms = match (n(3), n(4)) {
                                (Some(s), Some(r)) if r > 0 => wave_ms(s, r) as i32,
                                _ => -1,
                            };
                            Some((u32::from_str_radix(c.get(1)?, 16).ok()?, c.get(5).and_then(|v| v.parse().ok()).unwrap_or(0), ms))
                        }).collect();
                        let h0 = rows.iter().map(|r| r.1).min().unwrap_or(0);
                        // the bank's longest wave (F-46175: the maximum, from -1, of each entry's whole ms)
                        let longest = rows.iter().map(|r| r.2).fold(-1, i32::max);
                        for (h, _, _) in rows {
                            m.insert(h, (PathBuf::from(mp3), h0, longest));
                        }
                    }
                }
            }
            walk(&self.root, &mut m);
            log::info!("audio: {} stream channels indexed", m.len());
            self.stream_of = Some(m);
        }
        self.stream_of.as_ref().unwrap()
    }

    fn clip_of(&mut self, file: &Path, loop_start: i32) -> Option<Arc<Clip>> {
        self.clip_uses += 1;
        if let Some(c) = self.clips.get_mut(file) {
            c.1 = self.clip_uses;
            return c.0.clone();
        }
        let t = std::time::Instant::now();
        let c = if file.extension().is_some_and(|e| e == "mp3") { decode_mp3(file) } else { crate::audio::load_wav_clip(file) };
        let ms = t.elapsed().as_secs_f32() * 1000.0;
        if ms > 20.0 {
            log::info!("audio: {} read in {:.0} ms", file.display(), ms);
        }
        let c = c.map(|mut c| {
            c.loop_start = loop_start.max(0) as usize;
            Arc::new(c)
        });
        self.clips.insert(file.to_path_buf(), (c.clone(), self.clip_uses));
        self.trim_clips();
        c
    }

    /// Keep what the clips nothing plays hold under a budget, the longest unused dropped first: the ambience
    /// and the streams alone are gigabytes decoded (docs/PERFORMANCE.md, 2026-10-30). A clip asked for again is read again.
    fn trim_clips(&mut self) {
        const IDLE_CLIPS: u64 = 256 << 20;
        let bytes = |c: &Clip| 4 * c.stream.as_ref().map_or(c.data.len(), |s| s.samples.len()) as u64;
        let mut idle: Vec<(u64, PathBuf, u64)> = self.clips.iter().filter_map(|(p, (c, used))| c.as_ref().filter(|c| Arc::strong_count(c) == 1).map(|c| (*used, p.clone(), bytes(c)))).collect();
        let mut total: u64 = idle.iter().map(|e| e.2).sum();
        if total <= IDLE_CLIPS {
            return;
        }
        idle.sort_unstable();
        for (_, p, b) in idle {
            if total <= IDLE_CLIPS {
                break;
            }
            self.clips.remove(&p);
            total -= b;
        }
    }

    // ---------------------------------------------------------------------------------------------- building

    fn make(&mut self, hash: u32, depth: usize) -> Option<Node> {
        if hash == 0 || hash == NONE || depth > 24 {
            return None;
        }
        let o = self.snd.get(hash)?.to_vec();
        let s = audcfg::parse_sound(&o);
        let h = s.header.clone();
        // variances, drawn once (F-40563); the predelay and start offset words are never negative on the disc's data (no
        // variance exceeds its value; how a negative one would be read is not decoded, so it is kept at 0 here)
        let (vol, pitch, pan, pd, so) = variances(&h);
        let pitch = pitch as i32;
        let predelay_ms = (pd as i32).max(0) as f32;
        let start_ms = (so as i32).max(0) as f32;
        let mut ty_ms = 0;
        let kind = match s.payload {
            Payload::Simple { bank, wave } => {
                let w = self.bank_waves(bank).and_then(|m| m.get(&wave).cloned());
                if let Some(w) = w {
                    let clip = self.clip_of(&w.file.clone(), w.loop_start);
                    let dur_ms = w.dur_ms.filter(|_| clip.is_some()).or_else(|| clip.as_ref().map(|c| c.frames() as f32 * 1000.0 / c.rate as f32)).unwrap_or(0.0);
                    Kind::Wave { looping: w.loop_start >= 0, wave: Some(w), clip, voice: self.voice_id(), pos_ms: 0.0, dur_ms }
                } else if let Some((mp3, h0, _)) = self.stream_index().get(&wave).cloned() {
                    // a stream channel: the converted file holds every channel folded to stereo
                    let clip = self.clip_of(&mp3, -1);
                    let dur_ms = clip.as_ref().map(|c| c.frames() as f32 * 1000.0 / c.rate as f32).unwrap_or(0.0);
                    Kind::Stream { clip, headroom: h0 as f32 * 0.01, voice: self.voice_id(), pos_ms: 0.0, dur_ms, looping: false }
                } else {
                    Kind::Wave { wave: None, clip: None, voice: 0, pos_ms: 0.0, dur_ms: 0.0, looping: false }
                }
            }
            Payload::Streaming { duration_ms, children } => {
                // all channels are in one folded file: play the first channel's stream once
                let looped = h.tristate(6) == 1;
                match children.iter().find_map(|&c| self.make(c, depth + 1).map(|n| (c, n))) {
                    Some((c, mut n)) => {
                        // the type duration (F-96939): the authored one, else the loaded bank's longest wave (F-46175)
                        let loaded = matches!(&n.kind, Kind::Stream { clip: Some(_), .. });
                        let longest = match self.snd.get(c).map(|o| audcfg::parse_sound(o).payload) {
                            Some(Payload::Simple { wave, .. }) => self.stream_index().get(&wave).map(|e| e.2),
                            _ => None,
                        };
                        ty_ms = stream_ms(duration_ms, longest.filter(|_| loaded));
                        if let Kind::Stream { looping, .. } = &mut n.kind {
                            *looping = looped;
                        }
                        Kind::Group { children: vec![n] }
                    }
                    None => Kind::Empty,
                }
            }
            Payload::Wrapper { child } => Kind::Group { children: self.make(child, depth + 1).into_iter().collect() },
            Payload::Multitrack { children } => Kind::Group { children: children.iter().filter_map(|&c| self.make(c, depth + 1)).collect() },
            // TwinLoop (F-73377): both children and a curve named EQUAL_POWER_RISE or LINEAR_FALL, else it is not made
            Payload::TwinLoop { swap, xfade, curve, children } => {
                let linear = match curve {
                    c if c == joaat("EQUAL_POWER_RISE") => Some(false),
                    c if c == joaat("LINEAR_FALL") => Some(true),
                    _ => None,
                };
                let a = children.first().and_then(|&c| self.make(c, depth + 1));
                let b = children.get(1).and_then(|&c| self.make(c, depth + 1));
                match (a, b, linear) {
                    (Some(a), Some(b), Some(linear)) => {
                        let fade = Twin::new((swap.0 as u32, swap.1 as u32), (xfade.0 as u32, xfade.1 as u32), linear, || self.rng.with(|m| m.next()));
                        Kind::Twin { fade, children: vec![a, b], clock: 0.0, played: false }
                    }
                    _ => Kind::Empty,
                }
            }
            // approximation: Crossfade picks by distance
            Payload::Crossfade { near, .. } => Kind::Group { children: self.make(near, depth + 1).into_iter().collect() },
            Payload::VariableBlock { child, .. } | Payload::MathOperation { child, .. } => Kind::Group { children: self.make(child, depth + 1).into_iter().collect() },
            Payload::Randomized { history_len, children, .. } => {
                let pick = self.pick(hash, &h, history_len as usize, &children);
                Kind::Group { children: pick.and_then(|c| self.make(c, depth + 1)).into_iter().collect() }
            }
            Payload::Looping { count, restart_ms, child } => match self.make(child, depth + 1) {
                Some(c) => Kind::Looping { child: Box::new(c), child_hash: child, count, restart_ms, plays: 0, child_off: None },
                None => Kind::Empty,
            },
            Payload::Sequential { children } => {
                // F-75822: on demand when it has more than 8 children or tristate 5 is on; otherwise every child is made
                // now, in order, and one that cannot be made fails the sound
                let on_demand = children.len() > 8 || h.tristate(5) == 1;
                if on_demand {
                    Kind::Sequential { list: children, at: 0, cur: None, ahead: Default::default(), on_demand, cur_off: 0 }
                } else {
                    let mut ahead = std::collections::VecDeque::new();
                    let mut ok = true;
                    for &c in &children {
                        match self.make(c, depth + 1) {
                            Some(n) => ahead.push_back(n),
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        let at = children.len();
                        Kind::Sequential { list: children, at, cur: None, ahead, on_demand, cur_off: 0 }
                    } else {
                        Kind::Empty
                    }
                }
            }
            Payload::Retriggered { count, delay_ms, child } => Kind::Retrig { child_hash: child, count, delay_ms: delay_ms as f32, next_ms: 0.0, made: 0, live: Vec::new() },
            Payload::Envelope { attack, decay, sustain, hold, release, curves, child } => match self.make(child, depth + 1) {
                Some(c) => Kind::Envelope { a: attack as f32, d: decay as f32, s: sustain as f32 * 0.01, hold: hold as f32, r: release as f32, curves, child: Box::new(c), rel_from: None },
                None => Kind::Empty,
            },
            Payload::OnStop { child, on_stop, .. } => match self.make(child, depth + 1) {
                Some(c) => Kind::OnStop { child: Box::new(c), on_stop, stopped: None },
                None => Kind::Empty,
            },
            Payload::Speech | Payload::Other(_) => Kind::Empty,
        };
        Some(Node { h, vol, pitch, pan, predelay_ms, start_ms, st: St::Waiting, t: 0.0, play_t: 0.0, clock: Clock::default(), env0: 0.0, tot: (0, 0), ty_ms, kind })
    }

    fn voice_id(&mut self) -> u64 {
        self.next_voice += 1;
        VOICE_BASE + self.next_voice
    }

    /// Randomized (F-15148 / F-39111): step mode with tristate 5, else a weighted pick avoiding the history.
    fn pick(&mut self, hash: u32, h: &SoundHeader, hlen: usize, children: &[(u32, f32)]) -> Option<u32> {
        let n = children.len();
        if n == 0 {
            return None;
        }
        let (idx, hist) = self.history.entry(hash).or_insert_with(|| (0, vec![0xFF; hlen])).clone();
        if h.tristate(5) == 1 {
            let i = (idx + 1) % n;
            self.history.insert(hash, (i, hist));
            return Some(children[i].0);
        }
        let excluded = |i: usize| hlen < n && hist.contains(&(i as u8));
        let total: f32 = children.iter().enumerate().filter(|(i, _)| !excluded(*i)).map(|(_, c)| c.1).sum();
        let mut r = self.rng.float(0.0, total);
        let mut chosen = 0;
        for (i, c) in children.iter().enumerate() {
            if excluded(i) {
                continue;
            }
            r -= c.1;
            if r <= 0.0 {
                chosen = i;
                break;
            }
        }
        if hlen > 0 {
            let mut hist = hist;
            let at = idx % hlen;
            hist[at] = chosen as u8;
            self.history.insert(hash, ((at + 1) % hlen, hist));
        }
        Some(children[chosen].0)
    }

    // ---------------------------------------------------------------------------------------------- game API

    /// Start a sound by name. Returns `None` when sounds.dat has no such sound.
    pub fn play(&mut self, name: &str, req: Req) -> Option<Handle> {
        self.play_hash(joaat(name), req)
    }

    pub fn play_hash(&mut self, hash: u32, req: Req) -> Option<Handle> {
        let root = self.make(hash, 0)?;
        let id = self.next;
        self.next += 1;
        let timer = Some(self.timer_of(&root.h));
        // (test hook MCLA_SOUND_STARTS=1: every sound object started, by name)
        if std::env::var_os("MCLA_SOUND_STARTS").is_some() {
            log::info!("sound start: {} vol {:.2} pitch {}", self.snd.name_of(hash).unwrap_or("?"), req.volume, req.pitch);
        }
        self.insts.push(Instance { id, hash, root, req, prev_pos: None, frame_pos: None, doppler: 1.0, stop: false, stop_ms: -1, timer });
        Some(Handle(id))
    }

    /// The timer of a root sound whose request leaves it to the category (request +56 = -1, sound +65 = 63): its header's
    /// category (none: BASE) by categories.dat pair 1 (docs/reference/audio/SOUND_TIMER_RULE.md).
    fn timer_of(&self, h: &audcfg::SoundHeader) -> u8 {
        let cat = if h.category != 0 { h.category } else { joaat("BASE") };
        self.cat_timer.get(&cat).copied().unwrap_or(0)
    }

    /// A converted wave file played as a one-shot voice of `category` (a speech take chosen by the speech system: the
    /// original plays it through its Speech sound, whose category sets the level). `key` = `<bank>/<wave>` below the root.
    pub fn play_clip(&mut self, key: &str, category: &str, req: Req) -> Option<Handle> {
        let file = self.root.join(format!("{key}.wav"));
        let clip = self.clip_of(&file, -1)?;
        // `category` may name the voice's Speech sound (type 4, e.g. CAREER_SPEECH_DRV_MP_01_SET_POS_YELL: F-70477 plays
        // the take through "<voice>_<SoundPostFix>"): its header (volume, Doppler, category, mask, tristates) is the voice's
        let speech = self.snd.get(joaat(category)).filter(|o| o.first() == Some(&4)).map(|o| audcfg::parse_sound(o).header);
        // else a header with every tristate inherited and only the category field (mask bit 13)
        let mut o = vec![12u8, 0, 0, 0, 0, 0xAA, 0xAA, 0xAA, 0xAA, 0, 0];
        o.extend_from_slice(&(1u32 << 13).to_be_bytes());
        o.extend_from_slice(&joaat(category).to_be_bytes());
        let (h, _) = match speech {
            Some(h) => (h, 0),
            None => audcfg::parse_header(&o),
        };
        let (vol, pitch) = (h.volume as f32 * 0.01, h.pitch as i32);
        let dur_ms = clip.frames() as f32 * 1000.0 / clip.rate as f32;
        let voice = self.voice_id();
        let root = Node { h, vol, pitch, pan: -1, predelay_ms: 0.0, start_ms: 0.0, st: St::Waiting, t: 0.0, play_t: 0.0, clock: Clock::default(), env0: 0.0, tot: (0, 0), ty_ms: 0, kind: Kind::Wave { wave: None, clip: Some(clip), voice, pos_ms: 0.0, dur_ms, looping: false } };
        let id = self.next;
        self.next += 1;
        let timer = Some(self.timer_of(&root.h));
        self.insts.push(Instance { id, hash: 0, root, req, prev_pos: None, frame_pos: None, doppler: 1.0, stop: false, stop_ms: -1, timer });
        Some(Handle(id))
    }

    /// Change what the game asks of a playing sound (position, volume, pitch).
    pub fn set(&mut self, h: Handle, req: Req) {
        if let Some(i) = self.insts.iter_mut().find(|i| i.id == h.0) {
            i.req = req;
        }
    }

    /// The audio timer a playing sound runs on (see `Instance::timer`).
    pub fn set_timer(&mut self, h: Handle, t: u8) {
        if let Some(i) = self.insts.iter_mut().find(|i| i.id == h.0) {
            i.timer = Some(t.min(2));
        }
    }

    /// Stop request: release times apply (OnStop sounds start their stop sound).
    pub fn stop(&mut self, h: Handle) {
        if let Some(i) = self.insts.iter_mut().find(|i| i.id == h.0) {
            i.stop = true;
        }
    }

    /// Stop request with a release override of `ms` (the request entry +192 written before the stop, as the ambience's
    /// district switch does): every sound of the tree the stop reaches fades over `ms` instead of its own ReleaseTime
    /// (F-58485).
    pub fn stop_release(&mut self, h: Handle, ms: i32) {
        if let Some(i) = self.insts.iter_mut().find(|i| i.id == h.0) {
            i.stop = true;
            i.stop_ms = ms;
        }
    }

    /// A story scene's audio (F-16992 prepares, F-58551 plays): game.dat `CUTSCENES_<scene>` (type 21) - +11 the
    /// dialogue (+15 instead while it is wet, audio manager +1044, unless NULL_SOUND), +19 the effects stream unless
    /// NULL_SOUND, +23 the mixer scene. Flags +10: 0x1 the speech stops, 0x2 the police speech stops (`main.rs`), 0x4 the
    /// garage district (not ported); +27 the music manager's bits (none set on a story scene). None: no such object.
    pub fn cutscene_parts(&self, scene: &str, wet: bool) -> Option<(Vec<u32>, Option<String>)> {
        let o = self.game.get_name(&format!("CUTSCENES_{scene}")).filter(|o| o.first() == Some(&21) && o.len() >= 27)?;
        let u32at = |at: usize| u32::from_be_bytes(o[at..at + 4].try_into().unwrap());
        let null = joaat("NULL_SOUND");
        let dialogue = if wet && u32at(15) != null { u32at(15) } else { u32at(11) };
        let parts = [dialogue, u32at(19)].into_iter().filter(|&h| h != 0 && h != null).collect();
        let mixer = self.game.name_of(u32at(23)).filter(|_| self.game.get(u32at(23)).and_then(|g| g.first()) == Some(&23)).map(|n| n.to_string());
        Some((parts, mixer))
    }

    /// Move a playing sound's streams to `ms` from their start (a story scene's skip to its bookmark): each stream
    /// takes a new voice there and its old voice fades as one no longer driven.
    pub fn seek(&mut self, h: Handle, ms: f32) {
        fn walk(n: &mut Node, ms: f32, ids: &mut dyn FnMut() -> u64) {
            match &mut n.kind {
                Kind::Stream { pos_ms, voice, dur_ms, .. } => {
                    *pos_ms = ms.min(*dur_ms);
                    *voice = ids();
                }
                Kind::Group { children } | Kind::Twin { children, .. } => children.iter_mut().for_each(|c| walk(c, ms, ids)),
                Kind::Looping { child, .. } | Kind::Envelope { child, .. } | Kind::OnStop { child, .. } => walk(child, ms, ids),
                Kind::Sequential { cur: Some(c), .. } => walk(c, ms, ids),
                _ => {}
            }
        }
        let mut next = self.next_voice;
        if let Some(i) = self.insts.iter_mut().find(|i| i.id == h.0) {
            walk(&mut i.root, ms, &mut || {
                next += 1;
                VOICE_BASE + next
            });
        }
        self.next_voice = next;
    }

    pub fn playing(&self, h: Handle) -> bool {
        self.insts.iter().any(|i| i.id == h.0)
    }

    /// A playing sound's Duration in ms (F-53189 on its root through F-73664; -1 unknown or endless).
    pub fn duration_ms(&self, h: Handle) -> i32 {
        self.insts.iter().find(|i| i.id == h.0).map_or(-1, |i| dur_total(&i.root).0)
    }

    pub fn count(&self) -> usize {
        self.insts.len()
    }

    /// Diagnostics: the playing sounds by object name (with a count where one plays more than once) and the scenes
    /// that are on with their fade level.
    pub fn summary(&self) -> String {
        let mut names: Vec<String> = self.insts.iter().map(|i| self.snd.name_of(i.hash).map(|n| n.to_string()).unwrap_or_else(|| if i.hash == 0 { "clip".into() } else { format!("{:08x}", i.hash) })).collect();
        names.sort();
        let mut out: Vec<String> = Vec::new();
        for n in names {
            match out.last_mut() {
                Some(l) if l.split(" x").next() == Some(n.as_str()) => {
                    let k = l.split(" x").nth(1).and_then(|v| v.parse::<u32>().ok()).unwrap_or(1) + 1;
                    *l = format!("{n} x{k}");
                }
                _ => out.push(n),
            }
        }
        let mut sc: Vec<String> = self.scenes.iter().filter(|(_, s)| s.on || s.level > 0.0).map(|(h, s)| format!("{}@{:.2}", self.game.name_of(*h).unwrap_or("?"), s.level)).collect();
        sc.sort();
        format!("sounds [{}] scenes [{}]", out.join(", "), sc.join(", "))
    }

    /// Advance every sound by `dt` and drive the mixer's voices.
    /// (tests) Advance one playing sound by `dt` and give the volume chain (dB) of every wave it plays now - the
    /// original's vol[40] with its attack / release / envelope terms; empty once it has ended.
    #[cfg(test)]
    pub(crate) fn test_leaf_vols(&mut self, h: Handle, dt: f32) -> Vec<f32> {
        let Some(at) = self.insts.iter().position(|i| i.id == h.0) else { return Vec::new() };
        let mut inst = self.insts.remove(at);
        let t = inst.timer.unwrap_or(0) as usize;
        self.timer_ms[t] += dt as f64 * 1000.0;
        let now = self.timer_ms[t] as u32;
        if inst.stop {
            inst.stop = false;
            stop_node(&mut inst.root, inst.stop_ms, now);
        }
        let (ctx, rt) = root_ctx(&inst.req);
        let mut out = Vec::new();
        self.step(&mut inst.root, dt * 1000.0, now, ctx, rt, inst.req.offset(), &mut out);
        if inst.root.st != St::Done {
            self.insts.insert(at, inst);
        }
        out.iter().map(|l| l.ctx.vol).collect()
    }

    /// An effect chain's settings for this frame (the engine's PLAYER_WAVESHAPER, F-65630).
    /// An occluder's outputs (volume, cutoff in Hz) for the voices whose request names its slot (F-19918, every frame).
    pub fn set_occlusion(&mut self, slot: usize, out: [f32; 2]) {
        if self.occlusion.len() <= slot {
            self.occlusion.resize(slot + 1, [1.0, 23900.0]);
        }
        self.occlusion[slot] = out;
    }

    pub fn set_chain(&mut self, id: u32, p: crate::audio::ChainParams) {
        self.chain_params.retain(|c| c.0 != id);
        self.chain_params.push((id, p));
    }

    /// Once per drawn frame, before its game ticks, with the camera of the frame drawn last (made after that frame's ticks
    /// from their result): the sounds are heard at the places of that moment until the next frame (`frame_pos`: pan,
    /// distance gain and reverb send in `emit` against `listener`, which is that camera), and every 1/30 s of game time
    /// (two 60 Hz ticks: the original's frame, T+8 = 1/30) the Doppler sample of the original's environment update
    /// (once per frame, on the update that brings the new position): `doppler_sample` with each source's and the
    /// listener's motion since the last sample. `ticks`: the game ticks of the frame drawn last. Before (#43) both were
    /// worked out every tick against a camera that moves once per frame: the distance to a followed car went up by a
    /// tick's travel and back, which the Doppler made a warble in the outside views at 30 fps.
    /// (A frame of more than two ticks - under 30 fps - gives one sample over all of them, its distance change scaled to
    /// 1/30 s; I: the original's own frame then is 1/30 of game time all the same.)
    pub fn frame_sample(&mut self, cam: Vec3, ticks: usize) {
        for inst in self.insts.iter_mut() {
            inst.frame_pos = inst.req.pos;
        }
        self.sample_ticks += ticks;
        if self.sample_ticks < 2 {
            return;
        }
        let scale = 2.0 / self.sample_ticks as f32;
        self.sample_ticks = 0;
        let ml = self.sample_cam.map_or(Vec3::ZERO, |p| cam - p);
        self.sample_cam = Some(cam);
        for inst in self.insts.iter_mut() {
            if inst.timer.is_some_and(|t| self.timer_paused[t as usize]) {
                continue;
            }
            if let Some(p) = inst.req.pos {
                let ms = inst.prev_pos.map_or(Vec3::ZERO, |q| p - q);
                let (ms, ml) = if scale == 1.0 { (ms, ml) } else { (ms * scale, ml * scale) };
                inst.doppler = doppler_sample(inst.doppler, p, cam, ms, ml, inst.req.doppler);
                inst.prev_pos = Some(p);
            }
        }
    }

    pub fn update(&mut self, dt: f32, audio: &mut Audio) {
        self.step_scenes(dt);
        for (id, p) in std::mem::take(&mut self.chain_params) {
            audio.set_chain(id, &p);
        }
        let ms = dt * 1000.0;
        // the timers' times (engine+572 + 12 k), in ms: a paused one stands
        for t in 0..3 {
            if !self.timer_paused[t] {
                self.timer_ms[t] += ms as f64;
            }
        }
        let mut insts = std::mem::take(&mut self.insts);
        let (lp, lr, lf) = self.listener;
        let mut drives = Vec::new();
        let mut states = HashMap::new();
        for inst in insts.iter_mut() {
            // a sound on a paused timer is not updated (F-89113): its voices hold where they are, silent, and go on from
            // there when the timer runs again
            if inst.timer.is_some_and(|t| self.timer_paused[t as usize]) {
                let slowed = if inst.req.player { SLOWED } else { 0 };
                hold_voices(&inst.root, &mut |v| {
                    audio.hold(v | slowed);
                    if let Some(s) = self.voice_state.get(&(v | slowed)) {
                        states.insert(v | slowed, *s);
                    }
                });
                continue;
            }
            let now = self.timer_ms[inst.timer.unwrap_or(0) as usize] as u32;
            if inst.stop {
                inst.stop = false;
                stop_node(&mut inst.root, inst.stop_ms, now);
            }
            let (ctx, rt) = root_ctx(&inst.req);
            let mut out = Vec::new();
            self.step(&mut inst.root, ms, now, ctx, rt, inst.req.offset(), &mut out);
            for v in out {
                drives.push(self.emit(v, inst, (lp, lr, lf)));
            }
        }
        // the voice manager (F-46122): the waves ranked by their levels, 96 a bucket; the first 76 (the config's 96 voices
        // less 20, F-68447 / F-11111) are heard: a waiting one starts while physical voices are free, past them the
        // playing ones stop. A wave not playing goes on (its position runs) and starts from there when picked again.
        let slots: Vec<VoiceSlot> = drives
            .iter()
            .map(|d| VoiceSlot { active: true, ended: false, state: self.voice_state.get(&d.id).copied().unwrap_or(0), group: 0, flags: d.flags, h: d.h })
            .collect();
        let banks: Vec<&[VoiceSlot]> = slots.chunks(96).collect();
        let picked = rank_pick(&banks, 76, 96u32.saturating_sub(audio.voices() as u32));
        let at = |id: u16| (id >> 8) as usize * 96 + (id & 0xff) as usize;
        let mut go = vec![false; drives.len()];
        for &id in &picked.start {
            go[at(id)] = true;
        }
        let mut halt = vec![false; drives.len()];
        for &id in &picked.stop {
            halt[at(id)] = true;
        }
        for (k, d) in drives.into_iter().enumerate() {
            match (slots[k].state, go[k], halt[k]) {
                // a stopped voice waits again once its physical voice has stopped (F-44224: the next update)
                (2, ..) => {}
                (1, _, true) => {
                    states.insert(d.id, 2);
                }
                (1, ..) | (0, true, _) => {
                    states.insert(d.id, 1);
                    audio.drive(d.id, &d.clip, d.mix, d.ratio, d.looping, d.start, d.filt);
                }
                _ => {}
            }
        }
        self.voice_state = states;
        insts.retain(|i| i.root.st != St::Done);
        // sounds started while updating (none today) stay
        insts.append(&mut self.insts);
        self.insts = insts;
    }

    /// One node: timing, its type's rule, the settings handed down; playing waves are collected in `out`.
    /// `po`: the parent offset the node is played with and whether it is in percent (+108 and +57 bit 0x08): the parent's
    /// start offset, or the request's.
    fn step(&mut self, n: &mut Node, ms: f32, now: u32, parent: Ctx, rt: Rt, po: (u32, bool), out: &mut Vec<Leaf>) {
        if n.st == St::Done {
            return;
        }
        // settings (F-63029)
        let (vol, pitch, pan, lpf, hpf) = settings(n.h.no_inherit, n.vol, n.pitch, n.pan, &parent, &rt);
        // the play (F-91431, the first update) or an update (F-68220): the release (F-31923) or the attack (F-37133), then
        // the predelay wait; the play offsets (F-90115) from the parent's offset when it is played
        if n.clock.play_at.is_none() {
            n.tot = play_offsets(n, po);
            n.env0 = n.predelay_ms - po.0 as f32;
        }
        let at = Attack { ms: n.h.attack as u32, po: po.0, own_pd: n.predelay_ms as u32, ppd: 0 };
        // +58 bit 0x10 (the base constructor F-79360 sets it, OnStop's Init clears it): the release's end ends the sound
        let flags = if matches!(n.kind, Kind::OnStop { .. }) { 0x40 } else { 0x50 };
        // stopped without a release: ends once no child plays (F-68220, F-13519), before its type's update
        let ending = n.clock.stopped && n.clock.rel.is_none();
        if ending && !matches!(n.kind, Kind::OnStop { .. } | Kind::Envelope { .. }) && !live_child(&n.kind) {
            n.st = St::Done;
            return;
        }
        let (act, vol) = n.clock.tick(now, n.tot.0, at, flags, vol);
        match act {
            Act::Wait => return,
            Act::End => {
                n.st = St::Done;
                return;
            }
            Act::Start => {
                n.st = St::Playing;
                n.play_t = n.tot.1 as f32;
            }
            Act::Run => n.play_t += ms,
        }
        n.t = now.wrapping_sub(n.clock.play_at.unwrap_or(now)) as f32;
        let h = &n.h;
        let ctx = Ctx {
            vol,
            pitch,
            pan,
            category: if h.has(13) { h.category } else { parent.category },
            curve: if h.has(14) { h.curve } else { parent.curve },
            curve_scale: parent.curve_scale * if h.no_inherit2 & 0x40 != 0 { 1.0 } else { 1.0 } * h.curve_scale as f32 * 0.01,
            doppler: if h.no_inherit & 0x01 != 0 { 1.0 } else { parent.doppler } * h.doppler as f32 * 0.01,
            mask: h.speaker_mask | if h.no_inherit2 & 0x20 != 0 { 0 } else { parent.mask },
            positional: match h.tristate(1) {
                2 => parent.positional,
                v => v,
            },
            lpf,
            hpf,
        };
        let offset = n.play_t;
        // what the node's children are played with: its start offset (+84, F-90115)
        let child_po = (n.tot.1, false);
        let releasing = n.clock.stopped;
        let done = match &mut n.kind {
            Kind::Wave { clip, wave, voice, pos_ms, dur_ms, looping } => {
                let Some(c) = clip.clone() else { n.st = St::Done; return };
                if *pos_ms == 0.0 {
                    *pos_ms = if *looping { offset % dur_ms.max(1.0) } else { offset };
                }
                if !*looping && *pos_ms >= *dur_ms {
                    true
                } else {
                    let ratio = 2f32.powf(ctx.pitch as f32 / 1200.0);
                    let headroom = wave.as_ref().map(|w| w.headroom as f32 * 0.01).unwrap_or(0.0);
                    out.push(Leaf { voice: *voice, clip: c.clone(), ctx, headroom, start_frame: (*pos_ms * 0.001 * c.rate as f32) as f64, looping: *looping, stereo: false });
                    *pos_ms += ms * ratio;
                    if *looping && *pos_ms >= *dur_ms {
                        let ls = wave.as_ref().and_then(|w| w.loop_ms).unwrap_or(c.loop_start as f32 * 1000.0 / c.rate as f32);
                        *pos_ms = ls + (*pos_ms - *dur_ms) % (*dur_ms - ls).max(1.0);
                    }
                    false
                }
            }
            Kind::Stream { clip, headroom, voice, pos_ms, dur_ms, looping } => {
                let Some(c) = clip.clone() else { n.st = St::Done; return };
                if !*looping && *pos_ms >= *dur_ms {
                    true
                } else {
                    let ratio = 2f32.powf(ctx.pitch as f32 / 1200.0);
                    out.push(Leaf { voice: *voice, clip: c.clone(), ctx, headroom: *headroom, start_frame: (*pos_ms * 0.001 * c.rate as f32) as f64, looping: *looping, stereo: true });
                    *pos_ms += ms * ratio;
                    if *looping && *pos_ms >= *dur_ms {
                        *pos_ms %= dur_ms.max(1.0);
                    }
                    false
                }
            }
            Kind::Group { children } => {
                for c in children.iter_mut() {
                    self.step(c, ms, now, ctx, Rt::NONE, child_po, out);
                }
                // Multitrack with tristate 5: one finished child stops the others
                if n.h.tristate(5) == 1 && n.h.kind == 13 && children.iter().any(|c| c.st == St::Done) {
                    children.iter_mut().for_each(|c| stop_node(c, -1, now));
                }
                children.iter().all(|c| c.st == St::Done)
            }
            Kind::Twin { fade, children, clock, played } => {
                // Play (F-38194) when it starts, then Update (F-52106) at the update's ms: each child plays on at its level
                if !*played {
                    *played = true;
                    fade.play(*clock as u32, || self.rng.with(|m| m.next()));
                } else {
                    *clock += ms as f64;
                }
                let lv = fade.update(*clock as u32, || self.rng.with(|m| m.next()));
                for (c, db) in children.iter_mut().zip([lv.0, lv.1]) {
                    if c.st != St::Done {
                        let mut c2 = ctx;
                        c2.vol += db;
                        self.step(c, ms, now, c2, Rt::NONE, child_po, out);
                    }
                }
                children.iter().all(|c| c.st == St::Done)
            }
            Kind::Looping { child, child_hash, count, restart_ms, plays, child_off } => {
                // played (F-39812): with a count, the start offset is split by the child's duration into the plays done
                // and the offset into the next one (no split for an endless loop, an unknown or endless child)
                let off = *child_off.get_or_insert_with(|| {
                    let (p, off) = looping_split(*count, child, child_po.0);
                    if let Some(p) = p {
                        *plays = p;
                    }
                    off
                });
                // a child still dead at the update is a remake that made nothing: it is not stepped
                let r = (child.st != St::Done).then(|| {
                    self.step(child, ms, now, ctx, Rt::NONE, (off, false), out);
                    child.st != St::Done
                });
                match looping_update(r, *count, plays) {
                    LoopStep::Done => true,
                    LoopStep::Run => false,
                    LoopStep::Remake if ending => true,
                    LoopStep::Remake => {
                        // made again from its reference (deferred to after the tree update, F-70509 / F-21229, so it
                        // first runs in the next update as here) and played with the restart offset as its parent
                        // offset (F-87646); none made leaves the dead child, and the Looping ends in the next update
                        if let Some(c) = self.make(*child_hash, 1) {
                            **child = c;
                        }
                        *child_off = Some(*restart_ms as u32);
                        false
                    }
                }
            }
            Kind::Sequential { list, at, cur, ahead, on_demand, cur_off } => 'seq: {
                if !*on_demand && cur.is_none() && ahead.len() == list.len() {
                    // played (F-66779): the child that holds the start offset plays from the offset into it; the ones
                    // before it are dropped, and nothing plays when the offset is past the end (F-71973)
                    let Some((i, off)) = seq_start(ahead, child_po.0) else { break 'seq true };
                    ahead.drain(..i);
                    *cur = ahead.pop_front().map(Box::new);
                    *cur_off = off;
                }
                loop {
                    if cur.is_none() && ending {
                        break;
                    }
                    if cur.is_none() {
                        // the next child is played without a parent offset (F-35671)
                        *cur_off = 0;
                        if let Some(c) = ahead.pop_front() {
                            *cur = Some(Box::new(c));
                            continue;
                        }
                        if *at >= list.len() {
                            break;
                        }
                        let hsh = list[*at];
                        *at += 1;
                        *cur = self.make(hsh, 1).map(Box::new);
                        continue;
                    }
                    let c = cur.as_mut().unwrap();
                    self.step(c, ms, now, ctx, Rt::NONE, (*cur_off, false), out);
                    if c.st == St::Done {
                        *cur = None;
                        continue;
                    }
                    break;
                }
                cur.is_none() && ahead.is_empty() && *at >= list.len()
            }
            Kind::Retrig { child_hash, count, delay_ms, next_ms, made, live } => {
                while offset >= *next_ms && (*count == -1 || *made < *count as i32) {
                    if let Some(c) = self.make(*child_hash, 1) {
                        live.push(c);
                    }
                    *made += 1;
                    *next_ms += delay_ms.max(1.0);
                }
                for c in live.iter_mut() {
                    self.step(c, ms, now, ctx, Rt::NONE, (0, false), out);
                }
                live.retain(|c| c.st != St::Done);
                live.is_empty() && *count != -1 && *made >= *count as i32
            }
            Kind::Envelope { a, d, s, hold, r, curves, child, rel_from } => {
                // ADSR on the child's volume (F-82485): value = a * d * r, linear. Its times count from its play
                // time + own predelay - the parent's offset (+176 = +88 - +108 + +96): the start offset is not in them
                // (recorded: SST_ENV_TAIL, PreDelay 250, StartOffset 250, rises as sin((t - 250)/500 * pi/2) from the burn's
                // start)
                let t = n.t - n.env0;
                let cv = |me: &SoundLib, k: usize, x: f32, default: f32| me.curves.get(&curves[k]).map(|c| c.eval(x)).unwrap_or(default);
                let att = if *a > 0.0 { cv(self, 0, (t / *a).clamp(0.0, 1.0), (t / *a).clamp(0.0, 1.0)) } else { 1.0 };
                let dec = if t <= *a {
                    1.0
                } else if *d > 0.0 && t < *a + *d {
                    let x = ((t - *a) / *d).clamp(0.0, 1.0);
                    *s + (1.0 - *s) * cv(self, 1, x, 1.0 - x)
                } else {
                    *s
                };
                if rel_from.is_none() && (releasing || (*hold >= 0.0 && t >= *a + *d + *hold)) {
                    *rel_from = Some(t);
                }
                let rel = match rel_from {
                    Some(t0) if *r > 0.0 => {
                        let x = ((t - *t0) / *r).clamp(0.0, 1.0);
                        cv(self, 2, x, 1.0 - x)
                    }
                    Some(_) => 0.0,
                    None => 1.0,
                };
                // into the child's runtime volume +36 (F-66882): its own term, kept under its do-not-inherit 0x80
                self.step(child, ms, now, ctx, Rt { vol: lin2db(att * dec * rel), ..Rt::NONE }, child_po, out);
                child.st == St::Done || (rel_from.is_some() && rel <= 0.0)
            }
            Kind::OnStop { child, on_stop, stopped } => {
                if n.clock.stopped && stopped.is_none() {
                    *stopped = self.make(*on_stop, 1).map(Box::new);
                }
                self.step(child, ms, now, ctx, Rt::NONE, child_po, out);
                if let Some(s) = stopped {
                    self.step(s, ms, now, ctx, Rt::NONE, (0, false), out);
                    s.st == St::Done
                } else {
                    child.st == St::Done
                }
            }
            Kind::Empty => true,
        };
        if done {
            n.st = St::Done;
        }
    }

    /// A playing wave to the mixer: the voice's gain chain (F-24161), the pan law (F-56973 / F-67363),
    /// pitch and Doppler.
    fn emit(&mut self, v: Leaf, inst: &Instance, listener: (Vec3, Vec3, Vec3)) -> Drive {
        let cat = self.cats.get(&v.ctx.category).copied().unwrap_or(Category { volume: 0.0, pitch: 0, lpf: 23900, hpf: 0, rolloff_scale: 1.0, reverb_amount: 1.0, reverb_dist: 1.0 });
        let (lp, lr, lf) = listener;
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let mut rolloff = 0.0;
        let occ = inst.req.occluder.checked_sub(1).and_then(|i| self.occlusion.get(i as usize).copied());
        // positional: unset counts as positional unless a pan or speaker mask is set (A-47267)
        // (heard at the frame's place: frame_sample)
        let heard = inst.req.pos.map(|p| inst.frame_pos.unwrap_or(p));
        let positional = heard.is_some() && if v.ctx.pan == -1 && v.ctx.mask == 0 { v.ctx.positional != 0 } else { v.ctx.positional == 1 };
        let rolloff_at = |d: f32, s: f32| match audcfg::scaled_distance(d, s) {
            Some(dd) => match self.curves.get(&v.ctx.curve) {
                Some(c) => c.eval(dd),
                None => audcfg::default_rolloff_db(dd),
            },
            None => 0.0,
        };
        // the reverb send (F-77682 / F-24640, BUS_GRAPH_RULE.md 2.4): positional voices only, w = amount x
        // dB2lin(rolloff at the reverb distance scale - rolloff), then x REVERB_SMALL / MEDIUM / LARGE (room size) x level
        let mut w = 0.0;
        let dir: Option<Vec3> = if positional {
            let p = heard.unwrap() - lp;
            let d = p.length();
            let s = cat.rolloff_scale * v.ctx.curve_scale;
            rolloff = rolloff_at(d, s);
            if cat.reverb_dist * s != 0.0 {
                w = cat.reverb_amount * db2lin(rolloff_at(d, cat.reverb_dist * s) - rolloff);
            }
            Some(Vec3::new(p.dot(lr), p.dot(lr.cross(lf)), p.dot(lf)))
        } else if v.ctx.pan != -1 {
            let a = (v.ctx.pan as f32).to_radians();
            Some(Vec3::new(a.sin(), 0.0, a.cos()))
        } else {
            None
        };
        let (gl, gr) = if v.stereo {
            (1.0, 1.0)
        } else if v.ctx.mask != 0 && !positional {
            // speaker mask in stereo (F-67363): front to its side, centre / surround add 0.7071, / sqrt(bits)
            let m = v.ctx.mask;
            let mut l = if m & 1 != 0 { 1.0 } else { 0.0 };
            let mut r = if m & 2 != 0 { 1.0 } else { 0.0 };
            if m & 4 != 0 {
                l += k;
                r += k;
            }
            if m & 16 != 0 {
                l += k;
            }
            if m & 32 != 0 {
                r += k;
            }
            let bits = (m & 0x3F).count_ones().max(1) as f32;
            (l / bits.sqrt(), r / bits.sqrt())
        } else if let Some(d) = dir {
            pan_gains_by(d, occ.map_or(1.0, |o| o[0]))
        } else {
            (k, k)
        };
        // A = chain - headroom, B = the request's vol2 + category + rolloff, the request's volume is in the chain's top
        let a = v.ctx.vol - v.headroom;
        let b = inst.req.vol2 + cat.volume + rolloff;
        let amp = db2lin(a) * db2lin(b);
        let ratio = 2f32.powf((v.ctx.pitch + cat.pitch) as f32 / 1200.0) * if positional { inst.doppler } else { 1.0 };
        // the voice's filters (F-24161): LPF = max(100, min(chain, category)), HPF = max(chain, category)
        // (the occluder's cutoff on the positional / pan path, F-56973: min(the chain's word, the cutoff truncated))
        let lpf = match (occ, dir) {
            (Some(o), Some(_)) if !v.stereo && !(v.ctx.mask != 0 && !positional) => occluded_lpf(v.ctx.lpf as u32, o[1]) as u16,
            _ => v.ctx.lpf,
        };
        let filt = (lpf.min(cat.lpf).max(100), v.ctx.hpf.max(cat.hpf));
        // the levels (F-88152): dry = pan x B x G30, G30 = entry +76 x dB2lin(A); reverb s = send_s x pan x B x G30;
        // the chain send G28 = entry +72 x dB2lin(A) (no B); the chain's own output = pan x B (and its sends)
        let (level, size) = self.reverb_env;
        let send: [f32; 3] = ["REVERB_SMALL", "REVERB_MEDIUM", "REVERB_LARGE"].map(|n| w * self.curves.get(&joaat(n)).map_or(0.0, |c| c.eval(size)) * level);
        let lb = db2lin(b);
        let g30 = inst.req.dry * db2lin(a);
        let rev_b = send.map(|x| (x * lb * gl, x * lb * gr));
        let mix = crate::audio::Mix {
            gains: (amp * gl * inst.req.dry, amp * gr * inst.req.dry),
            rev: rev_b.map(|(l, r)| (l * g30, r * g30)),
            chain: inst.req.chain,
            send: if inst.req.chain != 0 { inst.req.fx_send * db2lin(a) } else { 0.0 },
            pans_chain: (inst.req.chain != 0 && inst.req.pans_chain).then_some(((lb * gl, lb * gr), rev_b)),
        };
        // the record's levels as the voice manager ranks them (F-24161): +26 = dB2lin(B), +28 / +30 = entry +72 / +76 x
        // dB2lin(A), +32 = dB2lin(C) with C the request's volume (here in the chain's top, so taken out of A); flag 0x80 =
        // entry +95 bit 2
        let la = db2lin(a - inst.req.volume);
        let h = [level_half(lb), level_half(inst.req.fx_send * la), level_half(inst.req.dry * la), level_half(db2lin(inst.req.volume))];
        let id = v.voice | if inst.req.player { SLOWED } else { 0 };
        Drive { id, clip: v.clip, mix, ratio, looping: v.looping, start: v.start_frame, filt, h, flags: if inst.req.pans_chain { 0x80 } else { 0 } }
    }
}

/// A wave's settings for the mixer this update, and its record's levels and flags for the voice manager.
struct Drive {
    id: u64,
    clip: Arc<Clip>,
    mix: crate::audio::Mix,
    ratio: f32,
    looping: bool,
    start: f64,
    filt: (u16, u16),
    h: [u16; 4],
    flags: u8,
}

/// Every wave / stream voice of a sound's tree.
fn hold_voices(n: &Node, f: &mut dyn FnMut(u64)) {
    match &n.kind {
        Kind::Wave { voice, .. } | Kind::Stream { voice, .. } => f(*voice),
        Kind::Group { children } | Kind::Twin { children, .. } => children.iter().for_each(|c| hold_voices(c, f)),
        Kind::Looping { child, .. } | Kind::Envelope { child, .. } => hold_voices(child, f),
        Kind::Sequential { cur, .. } => {
            if let Some(c) = cur {
                hold_voices(c, f)
            }
        }
        Kind::Retrig { live, .. } => live.iter().for_each(|c| hold_voices(c, f)),
        Kind::OnStop { child, stopped, .. } => {
            hold_voices(child, f);
            if let Some(s) = stopped {
                hold_voices(s, f)
            }
        }
        Kind::Empty => {}
    }
}

/// `fctidz` of a single, low word: what the original stores of a truncated 64-bit conversion.
fn fctidz_lo(x: f32) -> u32 {
    if x.is_nan() { 0 } else { x as i64 as u32 }
}

/// A sound's own duration in whole ms (-1 = unknown) and whether it is endless or loops: its type's method from the
/// table of F-25376. Children count with their totals (`dur_total`).
fn dur_type(n: &Node) -> (i32, bool) {
    let first = |v: &[Node], none: (i32, bool)| v.first().map(dur_total).unwrap_or(none);
    let max_of = |v: &[Node], start: (i32, bool)| v.iter().fold(start, |(m, f), c| {
        let (d, cf) = dur_total(c);
        (m.max(d), f | cf)
    });
    match &n.kind {
        // Simple (F-51499): the voice's length, looping when the wave loops
        Kind::Wave { dur_ms, looping, .. } | Kind::Stream { dur_ms, looping, .. } => (*dur_ms as i32, *looping),
        Kind::Group { children } => match n.h.kind {
            // Streaming (F-96939): its loop tristate
            8 => (n.ty_ms, n.h.tristate(6) == 1),
            // Multitrack (F-80575), Crossfade (F-57182): the longest child, from 0
            13 | 10 => max_of(children, (0, false)),
            // TwinLoop (F-81098): the longer of its two (also `Kind::Twin` below)
            3 => match children.split_first() {
                Some((c, rest)) => max_of(rest, dur_total(c)),
                None => (0, false),
            },
            // MathOperation (F-87138): -1 without a child
            25 => first(children, (-1, false)),
            // Wrapper (F-52913), Randomized / VariableBlock (F-67475)
            _ => first(children, (0, false)),
        },
        // TwinLoop (F-81098): the longer of its two
        Kind::Twin { children, .. } => match children.split_first() {
            Some((c, rest)) => max_of(rest, dur_total(c)),
            None => (0, false),
        },
        // Looping (F-53436): the child's times the count; endless with no count or an endless child
        Kind::Looping { child, count, .. } => {
            let (d, f) = dur_total(child);
            if *count == -1 || f {
                (d, true)
            } else if d == -1 {
                (-1, false)
            } else {
                (d.wrapping_mul(*count as i32), false)
            }
        }
        // Sequential (F-88072): the sum of the children it holds, from -1; unknown on demand or with an unknown child
        Kind::Sequential { cur, ahead, on_demand, .. } => {
            if *on_demand {
                return (-1, false);
            }
            let mut sum = -1i32;
            let mut f = false;
            for c in cur.iter().map(|c| &**c).chain(ahead.iter()) {
                let (d, cf) = dur_total(c);
                f |= cf;
                if d == -1 {
                    return (-1, f);
                }
                sum = sum.wrapping_add(d);
            }
            (sum, f)
        }
        // RetriggeredOverlapped (F-38643): (count - 1) x delay plus its child (the port's first live instance); endless
        // with no count
        Kind::Retrig { count, delay_ms, live, .. } => {
            let c = *count as i32;
            let base = if c == -1 { 0 } else { (c - 1).wrapping_mul(*delay_ms as i32) };
            let d = live.first().map_or(-1, |l| dur_total(l).0);
            (if d == -1 { base } else { base.wrapping_add(d) }, c == -1)
        }
        // Envelope (F-79791, times from F-57506): attack + decay + hold (+ release) when it holds, cut to a shorter finite
        // child
        Kind::Envelope { a, d, hold, r, child, .. } => {
            let (c, cf) = dur_total(child);
            let hold = *hold as i32;
            if hold < 0 {
                return (c, cf);
            }
            let mut t = (*a as i32).wrapping_add(*d as i32).wrapping_add(hold);
            if *r as i32 >= 0 {
                t = t.wrapping_add(*r as i32);
            }
            if !cf && c >= 0 && c < t {
                t = c;
            }
            if t < 0 { (c, cf) } else { (t, false) }
        }
        // OnStop (F-23434): its main child
        Kind::OnStop { child, .. } => dur_total(child),
        Kind::Empty => (0, false),
    }
}

/// F-53189: a sound's total duration, its own type's with its play's predelay and start offset (+80 / +84; 0 before it
/// plays): the start offset in percent of the duration with tristate 0, else what is left after it; -1 stays.
fn dur_total(n: &Node) -> (i32, bool) {
    let (d, f) = dur_type(n);
    if d == -1 {
        return (-1, f);
    }
    let (pd, so) = n.tot;
    let t = if n.h.tristate(0) == 1 {
        let x = fmadds(so as f32 * 0.01, d as f32, pd as f32);
        if x.is_nan() { i32::MIN } else { x as i32 }
    } else if d as u32 <= so {
        pd as i32
    } else {
        (d as u32).wrapping_sub(so).wrapping_add(pd) as i32
    };
    (t, f)
}

/// F-90115: the total predelay and start offset a sound plays with (+80 / +84), from its own (drawn) ones and the parent
/// offset `po` (+108, in percent of the sound's total when `po.1`, +57 bit 0x08): the parent's offset first eats the
/// predelay. Tristate 0 makes the own start offset a percent of the type's duration (an unknown one: the own predelay, no
/// offset); do-not-inherit 0x10 keeps the own predelay, 0x08 the own start offset (taken in percent once more with
/// tristate 0, as the original does).
fn play_offsets(n: &Node, po: (u32, bool)) -> (u32, u32) {
    let pd = n.predelay_ms as u32;
    let mut so = n.start_ms as u32;
    let pct = n.h.tristate(0) == 1;
    let scaled = |so: u32| match dur_type(n).0 {
        -1 => None,
        d => Some(fctidz_lo(so as f32 * d as f32 * 0.01)),
    };
    if pct {
        match scaled(so) {
            Some(v) => so = v,
            None => return (pd, 0),
        }
    }
    let p = if po.1 { fctidz_lo(dur_total(n).0 as f32 * po.0 as f32 * 0.01) } else { po.0 };
    let (mut a, mut b) = if p >= pd { (0, p.wrapping_sub(pd).wrapping_add(so)) } else { (pd.wrapping_sub(p), so) };
    if n.h.no_inherit & 0x10 != 0 {
        a = pd;
    }
    if n.h.no_inherit & 0x08 != 0 {
        if pct {
            match scaled(so) {
                Some(v) => so = v,
                None => return (pd, 0),
            }
        }
        b = so;
    }
    (a, b)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LoopStep {
    Done,
    Run,
    Remake,
}

/// One update of a Looping (F-67073): `child` its child's update (`None`: it holds no child, a remake that made nothing),
/// `ends` the plays ended (the original counts the plays begun, +196, in its +32 F-87646 - the same at every test).
/// No child ends it; a child that ended is remade unless the LoopCount is reached.
fn looping_update(child: Option<bool>, count: i16, ends: &mut i32) -> LoopStep {
    match child {
        None => LoopStep::Done,
        Some(true) => LoopStep::Run,
        Some(false) => {
            *ends += 1;
            if count != -1 && *ends >= count as i32 { LoopStep::Done } else { LoopStep::Remake }
        }
    }
}

/// F-39812: a Looping's start offset `so` split by its child's total into the plays done (`None`: left as they are) and
/// the child's parent offset; no split for an endless loop or an unknown or endless child, none at all for a child of 0 ms.
fn looping_split(count: i16, child: &Node, so: u32) -> (Option<i32>, u32) {
    if count == -1 {
        return (None, so);
    }
    match dur_total(child) {
        (-1, _) | (_, true) => (None, so),
        (0, _) => (None, 0),
        (d, _) => (Some((so as i32).wrapping_div(d)), (so as i32).wrapping_rem(d) as u32),
    }
}

/// F-96939: a Streaming sound's own duration, the authored one (+200) when set, else the longest wave of its bank when the
/// bank is loaded (F-46175), else -1.
fn stream_ms(duration_ms: u32, longest: Option<i32>) -> i32 {
    if duration_ms != 0 { duration_ms as i32 } else { longest.unwrap_or(-1) }
}

/// F-71973: the child of a Sequential that holds `so` and the offset into it, summing the children's totals; the first
/// with no offset when a child's is unknown, `None` past the end.
fn seq_start(children: &std::collections::VecDeque<Node>, so: u32) -> Option<(usize, u32)> {
    let mut cum = 0u32;
    for (i, c) in children.iter().enumerate() {
        let (d, f) = dur_total(c);
        if d == -1 {
            return Some((0, 0));
        }
        cum = cum.wrapping_add(d as u32);
        if cum >= so || f {
            return Some((i, (d as u32).wrapping_sub(cum).wrapping_add(so)));
        }
    }
    None
}

/// `fmadds`: the fused multiply-add in double, rounded once to single (how the recompiled code and the console do it).
pub(crate) fn fmadds(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// F-41334 (normalise `v` if its squared length is above `eps`, else `fallback`): the reciprocal root is `frsqrte` plus
/// two Newton steps in double, then `frsp`; the components are multiplied by it in single.
fn norm_or(v: [f32; 3], eps: f32, fallback: [f32; 3]) -> [f32; 3] {
    let y2 = v[1] * v[1];
    let l = fmadds(v[2], v[2], fmadds(v[0], v[0], y2));
    if !(l > eps) {
        return fallback;
    }
    let x = l as f64;
    let y0 = (1.0 / x.sqrt()) as f32 as f64;
    // (fnmsub: 1 - y*y*x rounded once, as -fma(y*y, x, -1))
    let e0 = -((y0 * y0).mul_add(x, -1.0));
    let y1 = e0.mul_add(y0 * 0.5, y0);
    let e1 = -((y1 * y1).mul_add(x, -1.0));
    let y2 = e1.mul_add(y1 * 0.5, y1);
    let k = y2 as f32;
    [v[0] * k, v[1] * k, v[2] * k]
}

/// The stereo pan of a direction in listener space (x right, y up, z forward; F-56973 with the stereo fold
/// L = FL + 0.7071 SL, R = FR + 0.7071 SR; the voice's +176 factor, unset, is 1). Same operations in the same order
/// as the original (verified bit for bit by `pan_gains_is_the_originals`).
pub fn pan_gains(d: Vec3) -> (f32, f32) {
    pan_gains_by(d, 1.0)
}

/// The voice's low-pass word with its occlusion pair's cutoff (F-56973 with +176 set): min(lpf, the cutoff truncated
/// toward zero, low 32 bits), unsigned.
pub fn occluded_lpf(lpf: u32, hz: f32) -> u32 {
    lpf.min((hz as i64) as u32)
}

/// [`pan_gains`] with the voice's +176 factor `f` (the occlusion volume, F-56973): every speaker's gain times `f`
/// before the stereo fold (verified bit for bit by `pan_occluded_is_the_originals`).
pub fn pan_gains_by(d: Vec3, f: f32) -> (f32, f32) {
    let kn = -0.707_106_77_f32;
    let kp = 0.707_106_77_f32;
    let v = norm_or([d.x, 0.0, d.z], 9.999_999_7e-6, [1.0, 0.0, 0.0]);
    let w = norm_or([d.x, d.y, d.z], 9.999_999_7e-6, [1.0, 0.0, 0.0]);
    let h = fmadds(w[1], v[1], fmadds(v[2], w[2], v[0] * w[0]));
    let e = (1.0 - h) * 0.5;
    let f8 = v[1] * 0.0;
    let (f6, f5, f4, f3) = (v[0] * kn, v[0] * kp, v[2] * kn, v[2] * kp);
    let a = (f8 + f6) + f3;
    let b = (f5 + f8) + f3;
    let c = (f4 + f8) + f6;
    let dd = (f4 + f5) + f8;
    let m = |x: f32| if -x >= 0.0 { 0.0 } else { x };
    let fl = fmadds(m(a), h, e);
    let fr = fmadds(m(b), h, e);
    let sl = fmadds(m(c), h, e);
    let sr = fmadds(m(dd), h, e);
    (fmadds(sl * f, kp, fl * f), fmadds(sr * f, kp, fr * f))
}

struct Leaf {
    voice: u64,
    clip: Arc<Clip>,
    ctx: Ctx,
    headroom: f32,
    start_frame: f64,
    looping: bool,
    stereo: bool,
}

/// One wave voice as the voice manager ranks it (F-91203): its slot is in use and has not ended (position field -8), the
/// slot's state (+16 top bits: 0 waiting, 1 playing, 2 stopping, F-60799 / F-71842 / F-44224), the voice group +4 (F-61065;
/// 0 = none, and no sound of the data sets the group tristate), the record's flag byte +24 and its levels +26 / +28 / +30 /
/// +32 as stored ([`level_half`]).
#[derive(Clone, Copy, Default)]
pub(crate) struct VoiceSlot {
    pub active: bool,
    pub ended: bool,
    pub state: u8,
    pub group: u32,
    pub flags: u8,
    pub h: [u16; 4],
}

/// What the voice manager does this update (F-46122): the counts of the ranking (voices, groups, forced to the front,
/// voices in no group), the voices to start and the playing voices to stop, as (bank << 8 | slot).
#[derive(Debug, PartialEq)]
pub(crate) struct Picked {
    pub counts: [u16; 4],
    pub start: Vec<u16>,
    pub stop: Vec<u16>,
}

/// The voice update's store of a level (F-24161): negative and NaN as 0, under 6.1e-5 as 0, at most 65504, the half float
/// cut off (not rounded).
pub(crate) fn level_half(x: f32) -> u16 {
    let x = if x >= 0.0 { x } else { 0.0 };
    if !(x >= 6.103_515_6e-5) {
        return 0;
    }
    let b = x.min(65504.0).to_bits();
    ((((b >> 23) & 0x1ff) + 16) << 10 | (b >> 13) & 0x3ff | (b >> 16) & 0x8000) as u16
}

/// The voice manager's ranking and pick (F-91203, F-96767). Every slot in use is ranked by its loudness: key = !(trunc
/// ((h28 + h30) x h26 x h32 x 2^32)), the smallest first, and 0 (the front) for a voice that pans an effect chain (+24 bit
/// 0x80), one with bit 0x08, or one with bit 0x20 that is not silent; the voices of a group go together, ranked by their
/// loudest. Down the ranking the first `budget` voices are kept: a waiting one starts while `free` physical voices last
/// (a group only whole); past the budget the playing ones stop. Ties keep the slots' order backwards, as the original's
/// lists do (each new voice at the front, then a stable radix sort).
pub(crate) fn rank_pick(banks: &[&[VoiceSlot]], budget: u32, free: u32) -> Picked {
    let half = |h: u16| {
        let h = h as u32;
        if h == 0 {
            0.0
        } else {
            f32::from_bits((h & 0x8000) << 16 | (h & 0x3ff) << 13 | (((h >> 10) & 31) + 112) << 23)
        }
    };
    // (key, id, state); `lone` in list order, each group's members newest first
    let mut recs: Vec<(u64, u16, u8)> = Vec::new();
    let mut lone: Vec<usize> = Vec::new();
    let mut groups: Vec<(u32, u64, Vec<usize>)> = Vec::new();
    let mut forced = 0u16;
    for (b, bank) in banks.iter().enumerate() {
        for (i, s) in bank.iter().enumerate().take(96) {
            if !s.active || s.ended {
                continue;
            }
            let [h26, h28, h30, h32] = s.h.map(half);
            let p = (h28 + h30) * h26 * h32;
            let x = p * 4_294_967_296.0;
            let t = if x.is_nan() { i64::MIN } else { x as i64 };
            let mut key = !(t as u64);
            if (s.flags & 0x20 != 0 && p > 0.0) || s.flags & 0x08 != 0 || s.flags & 0x80 != 0 {
                key = 0;
                forced += 1;
            }
            recs.push((key, (i as u16) | ((b as u16) & 0xff) << 8, s.state));
            let r = recs.len() - 1;
            if s.group == 0 {
                lone.insert(0, r);
            } else {
                let g = match groups.iter().position(|g| g.0 == s.group) {
                    Some(g) => g,
                    None => {
                        groups.push((s.group, key, Vec::new()));
                        groups.len() - 1
                    }
                };
                let g = &mut groups[g];
                if g.2.is_empty() || g.1 > key {
                    g.1 = key;
                }
                g.2.insert(0, r);
            }
        }
    }
    let counts = [recs.len() as u16, groups.len() as u16, forced, lone.len() as u16];
    lone.sort_by_key(|&r| recs[r].0);
    let mut order: Vec<usize> = Vec::new();
    for g in 0..groups.len() {
        if order.is_empty() || groups[g].1 <= groups[order[0]].1 {
            order.insert(0, g);
        } else {
            let mut p = 0;
            while p + 1 < order.len() && groups[order[p + 1]].1 < groups[g].1 {
                p += 1;
            }
            order.insert(p + 1, g);
        }
    }
    let (mut budget, mut free) = (budget, free);
    let (mut r, mut gi) = (0, 0);
    let (mut start, mut stop) = (Vec::new(), Vec::new());
    while r < lone.len() || gi < order.len() {
        if budget != 0 {
            if r < lone.len() && (gi >= order.len() || recs[lone[r]].0 < groups[order[gi]].1) {
                let (_, id, st) = recs[lone[r]];
                if st == 0 && free != 0 {
                    free -= 1;
                    start.push(id);
                }
                r += 1;
                budget -= 1;
            } else {
                let m = &groups[order[gi]].2;
                let mut left = m.len() as u32;
                if budget >= left {
                    for &x in m {
                        let (_, id, st) = recs[x];
                        if st == 0 {
                            if free < left {
                                break;
                            }
                            free -= 1;
                            left -= 1;
                            start.push(id);
                        }
                        budget -= 1;
                    }
                }
                gi += 1;
            }
        } else {
            if r < lone.len() {
                if recs[lone[r]].2 == 1 {
                    stop.push(recs[lone[r]].1);
                }
                r += 1;
            }
            if gi < order.len() {
                stop.extend(groups[order[gi]].2.iter().filter(|&&x| recs[x].2 == 1).map(|&x| recs[x].1));
                gi += 1;
            }
        }
    }
    Picked { counts, start, stop }
}

/// The release ramp of F-31923 on the original's integer clock: `ratio = (end - now) / (end - start)` (0.0 when `end <= start`
/// or `now >= end`, which also marks the sound ended: -100 dB), `vol += 20 * log2(ratio) * 0.301` in dB (-100 below 1e-5). Returns the
/// new volume and whether the sound goes on (an ended sound stops only when `flags & 0x10` is clear in the original's +58).
/// Verified bit for bit by `release_step_is_the_originals`; `Clock::tick` steps a stopped sound's release with it.
pub(crate) fn release_step(start: u32, end: u32, now: u32, vol: f32, flags: u8) -> (f32, bool) {
    let (ratio, ended) = if end <= start || now >= end { (0.0, true) } else { ((end - now) as f32 / (end - start) as f32, false) };
    (vol + lin2db(ratio), !(ended && flags & 0x10 != 0))
}

/// A stop request down the tree (F-58485): the release time, or done at once without one. `ovr`: the request's release
/// override (entry +192), which replaces the sound's own ReleaseTime unless -1.
fn stop_node(n: &mut Node, ovr: i32, now: u32) {
    if n.st == St::Done || n.clock.stopped {
        return;
    }
    let rel = if ovr != -1 { ovr } else { n.h.release as i32 };
    if n.clock.stop(now, rel) {
        // F-58485: a sound with a ReleaseTime only marks its own release (start = now, end = now + release) and
        // returns; its children are not stopped and play on under the fading parent until the release ends (recorded:
        // the slipstream burn SLIPSTREAM_NITROUS_MASTER, ReleaseTime 2000, kept both SST envelopes at their full value
        // for the 2 s after the stop - work/slipstream/audiorec)
        return;
    } else if matches!(n.kind, Kind::Wave { .. } | Kind::Stream { .. } | Kind::Empty) {
        n.st = St::Done;
        return;
    }
    // no release time (F-58485: +57 bit 0x10, then F-84762 asks each playing child to stop - its ChildStop, by default a
    // stop request of its own with its own release, F-58485 again): the children stop, the sound ends at the first update
    // with none playing (F-68220 with F-13519) and makes none (Looping, Sequential and Retriggered test the bit); OnStop
    // plays its stop sound, Envelope its release stage
    match &mut n.kind {
        Kind::Group { children } | Kind::Twin { children, .. } => children.iter_mut().for_each(|c| stop_node(c, ovr, now)),
        // Envelope: its own release stage fades the child (the step above starts it)
        Kind::Looping { child, .. } => stop_node(child, ovr, now),
        Kind::Sequential { cur, .. } => {
            if let Some(c) = cur {
                stop_node(c, ovr, now)
            }
        }
        Kind::Retrig { live, count, made, .. } => {
            *count = *made as i16;
            live.iter_mut().for_each(|c| stop_node(c, ovr, now))
        }
        Kind::OnStop { child, .. } => stop_node(child, ovr, now),
        _ => {}
    }
}

/// Whether a node has a child still playing (F-13519: a direct child in state 2).
fn live_child(k: &Kind) -> bool {
    let up = |c: &Node| c.st != St::Done;
    match k {
        Kind::Group { children } | Kind::Twin { children, .. } => children.iter().any(up),
        Kind::Looping { child, .. } => up(child),
        Kind::Sequential { cur, .. } => cur.as_deref().is_some_and(up),
        Kind::Retrig { live, .. } => live.iter().any(up),
        Kind::Envelope { child, .. } => up(child),
        Kind::OnStop { child, stopped, .. } => up(child) || stopped.as_deref().is_some_and(up),
        _ => false,
    }
}

/// A node played again from the start (Looping's restart).
fn reset_node(n: &mut Node) {
    n.st = St::Waiting;
    n.t = 0.0;
    n.play_t = 0.0;
    n.clock = Clock::default();
    n.tot = (0, 0);
    match &mut n.kind {
        Kind::Wave { pos_ms, .. } | Kind::Stream { pos_ms, .. } => *pos_ms = 0.0,
        Kind::Group { children } => children.iter_mut().for_each(reset_node),
        Kind::Twin { children, clock, played, .. } => {
            (*clock, *played) = (0.0, false);
            children.iter_mut().for_each(reset_node)
        }
        Kind::Looping { child, plays, child_off, .. } => {
            *plays = 0;
            *child_off = None;
            reset_node(child)
        }
        Kind::Envelope { child, rel_from, .. } => {
            *rel_from = None;
            reset_node(child)
        }
        Kind::Sequential { at, cur, ahead, .. } => {
            // (made again from the list as it goes)
            *at = 0;
            *cur = None;
            ahead.clear();
        }
        Kind::Retrig { next_ms, made, live, .. } => {
            *next_ms = 0.0;
            *made = 0;
            live.clear();
        }
        Kind::OnStop { child, stopped, .. } => {
            *stopped = None;
            reset_node(child)
        }
        Kind::Empty => {}
    }
}

/// A converted stream (stereo MP3): decoded while it plays when its length is known (`<name>.mp3.txt`, the
/// stream's frames at its rate - the same as decoded, checked on the disc's streams; a rate the MP3 cannot hold,
/// the landmark bells' 4800 Hz, was resampled by the converter to 8000 Hz, so the frames are counted at the file's
/// rate), else decoded whole.
pub(crate) fn decode_mp3(p: &Path) -> Option<Clip> {
    use rodio::Source;
    let first = std::fs::read_to_string(format!("{}.txt", p.display())).ok().and_then(|t| t.lines().next().map(str::to_string));
    let col = |k: usize| first.as_deref().and_then(|l| l.split('\t').nth(k)).and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0);
    let f = std::fs::File::open(p).ok()?;
    let dec = rodio::Decoder::new(std::io::BufReader::new(f)).ok()?;
    let channels = dec.channels() as usize;
    let rate = dec.sample_rate();
    let frames = col(3).map(|n| match col(4) {
        Some(r) if r != rate as usize => ((n as u64 * rate as u64).div_ceil(r as u64)) as usize,
        _ => n,
    });
    if let Some(frames) = frames {
        // (stereo kept, otherwise the first channel - as the whole decode below)
        let out = if channels == 2 { 2 } else { 1 };
        let buf = Arc::new(crate::audio::Progressive::zeroed(frames * out, frames));
        let b = buf.clone();
        let name = p.display().to_string();
        let spawned = std::thread::Builder::new().name("mp3 decode".into()).spawn(move || {
            let mut k = 0usize;
            for (j, s) in dec.enumerate() {
                if out == 1 && j % channels.max(1) != 0 {
                    continue;
                }
                let Some(v) = b.samples.get(k) else { break };
                v.store((s as f32 / 32768.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
                k += 1;
            }
            if k != b.samples.len() {
                log::warn!("audio: {} decoded {} frames, its .txt says {}", name, k / out, b.frames);
            }
            b.done.store(true, std::sync::atomic::Ordering::Release);
        });
        if spawned.is_ok() {
            return Some(Clip { data: Vec::new(), rate, channels: out, loop_start: 0, stream: Some(buf) });
        }
        return decode_mp3_whole(p);
    }
    decode_mp3_whole_from(dec, channels, rate)
}

fn decode_mp3_whole(p: &Path) -> Option<Clip> {
    use rodio::Source;
    let f = std::fs::File::open(p).ok()?;
    let dec = rodio::Decoder::new(std::io::BufReader::new(f)).ok()?;
    let (channels, rate) = (dec.channels() as usize, dec.sample_rate());
    decode_mp3_whole_from(dec, channels, rate)
}

fn decode_mp3_whole_from(dec: rodio::Decoder<std::io::BufReader<std::fs::File>>, channels: usize, rate: u32) -> Option<Clip> {
    let data: Vec<f32> = dec.map(|s| s as f32 / 32768.0).collect();
    if channels == 2 {
        Some(Clip { data, rate, channels: 2, loop_start: 0, stream: None })
    } else {
        // mono / other: keep the first channel
        Some(Clip::mono(data.chunks(channels.max(1)).map(|c| c[0]).collect(), rate))
    }
}

#[cfg(test)]
mod tests {

    /// #43: a car at 40 m/s with the camera 6 m behind it, made once per drawn frame (30 fps: two 60 Hz ticks a frame).
    /// Sampled once per 1/30 s against that frame's camera with both motions (frame_sample) the distance never changes
    /// and the ratio stays 1. Sampled every tick against the frame's camera (the port before) the ratio swung by
    /// several per cent from tick to tick - the engine's warble in the outside views. The same at 60 fps (one tick a
    /// frame, a sample every other frame).
    #[test]
    fn a_followed_car_keeps_its_doppler() {
        use super::*;
        let tick = 1.0 / 60.0f32;
        let vel = Vec3::new(40.0, 0.0, 0.0);
        for ticks_per_frame in [2usize, 1] {
            let (mut car, mut ratio, mut acc) = (Vec3::new(3.0, 0.5, -2.0), 1.0f32, 0usize);
            let mut cam = car - Vec3::new(6.0, -1.5, 0.0);
            let (mut prev_car, mut prev_cam) = (None::<Vec3>, None::<Vec3>);
            for _frame in 0..120 {
                acc += ticks_per_frame;
                if acc >= 2 {
                    acc = 0;
                    let ms = prev_car.map_or(Vec3::ZERO, |q| car - q);
                    let ml = prev_cam.map_or(Vec3::ZERO, |q| cam - q);
                    ratio = doppler_sample(ratio, car, cam, ms, ml, 1.0);
                    (prev_car, prev_cam) = (Some(car), Some(cam));
                }
                for _ in 0..ticks_per_frame {
                    car += vel * tick;
                }
                cam = car - Vec3::new(6.0, -1.5, 0.0);
                assert!((ratio - 1.0).abs() < 2e-4, "{ticks_per_frame}: {ratio}");
            }
        }
        // the port before: every tick against the frame's camera
        let (mut car, mut ratio, mut lo, mut hi) = (0.0f32, 1.0f32, 1.0f32, 1.0f32);
        let mut cam = car - 6.0;
        let mut pd = None::<f32>;
        for _frame in 0..60 {
            for _ in 0..2 {
                car += 40.0 * tick;
                let d = car - cam;
                if let Some(pd) = pd {
                    let v = ((d - pd) / tick).clamp(-100.0, 100.0);
                    ratio = 0.5 * ratio + 0.5 * (340.0 / (340.0 + v));
                    (lo, hi) = (lo.min(ratio), hi.max(ratio));
                }
                pd = Some(d);
            }
            cam = car - 6.0;
        }
        assert!(hi - lo > 0.1, "{lo} .. {hi}");
    }

    /// The Doppler ratio F-42745 against the original's code run offline (tools/doppler_ratio_reference.py: 4,000 distance
    /// changes, factors and time steps, the clamp's edges and signed zeros), bit for bit.
    #[test]
    fn doppler_ratio_is_the_originals() {
        use super::*;
        let mut n = 0;
        for l in include_str!("testdata/doppler_ratio_ref.txt").lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            let h = |i: usize| u32::from_str_radix(&w[i][1..], 16).unwrap();
            let t: u32 = w[2][1..].parse().unwrap();
            assert_eq!(doppler_ratio(f32::from_bits(h(0)), f32::from_bits(h(1)), t).to_bits(), h(4), "{l}");
            n += 1;
        }
        assert_eq!(n, 4000);
    }

    /// A new sound's variance draws (F-40563) against the original's code run offline with the game's rand
    /// (tools/sound_variance_reference.py): the 934 distinct field sets of the disc's sounds.dat and 3,000 random ones, the
    /// generator's state chained through them - the sound's five values and the state after, bit for bit.
    #[test]
    fn variances_match_the_original() {
        use super::*;
        let t = include_str!("testdata/sound_variance_ref.txt");
        let mut n = 0;
        for l in t.lines() {
            let w: Vec<u32> = l.split_whitespace().filter(|x| *x != "->").map(|x| u32::from_str_radix(&x[1..], 16).unwrap()).collect();
            // an object with the ten fields present (mask bits 0 - 9), as parse_header reads it
            let mut o = vec![0u8; 11];
            o.extend_from_slice(&0x3FFu32.to_be_bytes());
            for &f in &w[2..10] {
                o.extend_from_slice(&(f as u16).to_be_bytes());
            }
            o.extend_from_slice(&w[10].to_be_bytes());
            o.extend_from_slice(&w[11].to_be_bytes());
            let (h, _) = audcfg::parse_header(&o);
            set_audio_rng_state((w[0], w[1]));
            let (v, p, pan, pd, so) = variances(&h);
            let st = audio_rng_state();
            assert_eq!((v.to_bits(), p as u16 as u32, pan as u16 as u32, pd, so, st.0, st.1), (w[12], w[13], w[14], w[15], w[16], w[17], w[18]), "{l}");
            n += 1;
        }
        assert_eq!(n, 3934);
    }

    /// The voice manager's ranking and pick (F-91203, F-96767) against the original's code run offline on random voice
    /// slots (tools/voice_rank_reference.py): the counts, the voices to start and to stop, in order
    #[test]
    fn voice_pick_matches_the_original() {
        use super::*;
        let mut n = 0;
        for line in include_str!("testdata/voice_rank_ref.txt").lines() {
            let (case, out) = line.split_once(" -> ").unwrap();
            let (head, slots) = case.split_once(" | ").unwrap();
            let h: Vec<u32> = head.split(' ').map(|x| x.parse().unwrap()).collect();
            let mut banks = vec![vec![VoiceSlot::default(); 96]; h[0] as usize];
            for s in slots.split(' ').filter(|s| !s.is_empty()) {
                let v: Vec<u32> = s.split(',').map(|x| x.parse().unwrap()).collect();
                banks[v[0] as usize][v[1] as usize] = VoiceSlot {
                    active: true,
                    ended: v[2] != 0,
                    state: v[3] as u8,
                    group: v[4],
                    flags: v[5] as u8,
                    h: [v[6] as u16, v[7] as u16, v[8] as u16, v[9] as u16],
                };
            }
            let refs: Vec<&[VoiceSlot]> = banks.iter().map(|b| b.as_slice()).collect();
            let mut part = out.split('|');
            let nums = |s: &str| -> Vec<u16> { s.split_whitespace().map(|x| x.parse().unwrap()).collect() };
            let c = nums(part.next().unwrap());
            let want = Picked { counts: [c[0], c[1], c[2], c[3]], start: nums(part.next().unwrap()), stop: nums(part.next().unwrap()) };
            assert_eq!(rank_pick(&refs, h[1], h[2]), want, "{head}");
            n += 1;
        }
        assert_eq!(n, 600);
    }

    /// MANAGER_PLAYER_SPEED against the original's F-15595 run offline (tools/player_speed_reference.py): the game's table
    /// and 60 random ones, 5,802 frames of camera mode, both speed inputs and the frame time; every controller's new
    /// linear level, bit for bit.
    #[test]
    fn player_speed_matches_the_original() {
        use super::*;
        let hx = |t: &str| u32::from_str_radix(&t[1..], 16).unwrap();
        let num = |t: &str| t[1..].parse::<u32>().unwrap();
        let (mut recs, mut lins, mut n) = (Vec::new(), Vec::new(), 0);
        for l in include_str!("testdata/player_speed_ref.txt").lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            match w.first() {
                Some(&"T") => {
                    recs.clear();
                    lins.clear();
                }
                Some(&"r") => {
                    let f = |i: usize| f32::from_bits(hx(w[i]));
                    recs.push(SpeedRec { cat: num(w[1]), a: [f(2), f(3)], b: [f(4), f(5)], mb: [num(w[6]) as u16 as i16, num(w[7]) as u16 as i16], rate: f(8) });
                    lins.push(1.0f32);
                }
                Some(&"F") => {
                    let (mode, sa, sb, dt) = (num(w[1]) as i32, f32::from_bits(hx(w[2])), f32::from_bits(hx(w[3])), f32::from_bits(hx(w[4])));
                    let want: Vec<u32> = w[6..].iter().map(|t| hx(t)).collect();
                    assert_eq!(want.len(), recs.len(), "{l}");
                    for (k, r) in recs.iter().enumerate() {
                        lins[k] = speed_step(r, lins[k], mode, sa, sb, dt);
                        assert_eq!(lins[k].to_bits(), want[k], "record {k} {r:?}: {l}");
                    }
                    n += 1;
                }
                _ => {}
            }
        }
        assert_eq!(n, 5802);
    }

    #[test]
    fn twinloop_fade_matches_the_original() {
        // tools/twinloop_reference.py: the original's TwinLoop Init, Play and Update on random TwinLoops and times; every
        // call's two child levels, next swap time, target, fader value and the generator's state, bit for bit
        use super::*;
        let mut twin = None;
        let mut m = Mwc { lo: 0, carry: 0 };
        let mut n = 0;
        for l in include_str!("testdata/twinloop_ref.txt").lines() {
            // (each number is led by a letter)
            let w: Vec<&str> = l.split_whitespace().map(|x| if x.len() > 1 { &x[1..] } else { x }).collect();
            match w.first().copied() {
                // the original's hashes of the two curve names, as make() compares them
                Some("H") => assert_eq!((joaat("EQUAL_POWER_RISE"), joaat("LINEAR_FALL")), (w[1].parse().unwrap(), w[2].parse().unwrap())),
                Some("T") => {
                    let v: Vec<u32> = w[1..].iter().map(|x| x.parse().unwrap()).collect();
                    m = Mwc { lo: v[5], carry: v[6] };
                    twin = Some(Twin::new((v[0], v[1]), (v[2], v[3]), v[4] == 1, || m.next()));
                }
                Some(k @ ("P" | "U")) => {
                    let t = twin.as_mut().unwrap();
                    let now: u32 = w[1].parse().unwrap();
                    let (a, b) = if k == "P" { t.play(now, || m.next()) } else { t.update(now, || m.next()) };
                    let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
                    let got = (a.to_bits(), b.to_bits(), t.next_swap, t.target.to_bits(), t.fader.v.to_bits(), m.lo, m.carry);
                    let want = (hex(w[3]), hex(w[4]), w[5].parse().unwrap(), hex(w[6]), hex(w[7]), w[8].parse().unwrap(), w[9].parse().unwrap());
                    assert_eq!(got, want, "{l}");
                    n += 1;
                }
                _ => {}
            }
        }
        assert_eq!(n, 26_198);
    }

    #[test]
    fn looping_remake_matches_the_original() {
        // tools/looping_remake_reference.py: the original's Looping update and deferred remake on 1 - 14 Loopings; whether a
        // remake made a child (the make's result, or the 13th request of an update dropped) is read from the reference
        use super::*;
        let (mut ends, mut holds, mut alive) = (Vec::new(), Vec::new(), Vec::new());
        let mut counts: Vec<i16> = Vec::new();
        let mut n = 0;
        for l in include_str!("testdata/looping_remake_ref.txt").lines() {
            if let Some(c) = l.strip_prefix('c') {
                counts = c.split_whitespace().map(|v| v.parse().unwrap()).collect();
                (ends, holds, alive) = (vec![0; counts.len()], vec![true; counts.len()], vec![true; counts.len()]);
                continue;
            }
            let Some((i, o)) = l.split_once(" -> ") else { continue };
            let (i, o): (Vec<&str>, Vec<&str>) = (i.split_whitespace().collect(), o.split_whitespace().collect());
            for j in 0..counts.len() {
                if !alive[j] {
                    assert_eq!((i[j], o[j]), ("x", "x"), "{l}");
                    continue;
                }
                assert_eq!(i[j] == "-", !holds[j], "{l}");
                let r = (i[j] != "-").then(|| i[j] == "1");
                let e: Vec<i32> = o[j].split('/').map(|v| v.parse().unwrap()).collect();
                let step = looping_update(r, counts[j], &mut ends[j]);
                if step == LoopStep::Remake {
                    holds[j] = e[2] == 1;
                }
                let run = step != LoopStep::Done;
                assert_eq!((run as i32, e[1]), (e[0], ends[j] + (run && holds[j]) as i32), "{l} #{j}");
                alive[j] = run;
                n += 1;
            }
        }
        assert_eq!(n, 39_240);
    }

    /// The durations of the sound tree (F-53189 with each type's method), the play offsets (F-90115), the Looping split
    /// (F-39812) and the Sequential start (F-71973) against the original's code run offline on random trees
    /// (tools/sound_duration_reference.py; a case the original faults on is left out)
    #[test]
    fn durations_match_the_original() {
        use super::*;
        type Spec = (Vec<i64>, Vec<usize>);
        fn build(i: usize, specs: &[Spec], lw: i64) -> Node {
            let (f, kids) = &specs[i];
            let (t, p) = (f[0] as u8, &f[6..11]);
            let mut b = vec![t, 0, 0, 0, 0];
            b.extend_from_slice(&((f[3] as u32) | (f[5] as u32) << 12).to_be_bytes());
            b.extend_from_slice(&[f[4] as u8, 0, 0, 0, 0, 0]);
            let (h, _) = audcfg::parse_header(&b);
            let mut ch: Vec<Node> = kids.iter().map(|&k| build(k, specs, lw)).collect();
            let mut one = || Box::new(ch.remove(0));
            let mut ty_ms = 0;
            let kind = match t {
                12 => Kind::Wave { wave: None, clip: None, voice: 0, pos_ms: 0.0, dur_ms: p[0] as u32 as i32 as f32, looping: p[1] != 0 },
                1 => Kind::Looping { child: one(), child_hash: 0, count: p[0] as i16, restart_ms: 0, plays: 0, child_off: None },
                2 => Kind::Envelope { a: p[0] as f32, d: p[1] as f32, s: p[4] as f32 * 0.01, hold: p[2] as f32, r: p[3] as f32, curves: [0; 3], child: one(), rel_from: None },
                5 => Kind::OnStop { child: one(), on_stop: 0, stopped: None },
                7 => Kind::Sequential { list: vec![0; ch.len()], at: ch.len(), cur: None, ahead: ch.drain(..).collect(), on_demand: p[0] != 0, cur_off: 0 },
                8 => {
                    ty_ms = stream_ms(p[0] as u32, (p[2] != 0 && p[3] & 1 != 0).then_some(lw as i32));
                    Kind::Group { children: Vec::new() }
                }
                9 => Kind::Retrig { child_hash: 0, count: p[0] as i16, delay_ms: p[1] as f32, next_ms: 0.0, made: 0, live: ch.drain(..).collect() },
                _ => Kind::Group { children: ch.drain(..).collect() },
            };
            Node { h, vol: 0.0, pitch: 0, pan: -1, predelay_ms: 0.0, start_ms: 0.0, st: St::Waiting, t: 0.0, play_t: 0.0, clock: Clock::default(), env0: 0.0, tot: (f[1] as u32, f[2] as u32), ty_ms, kind }
        }
        let mut n = 0;
        for l in include_str!("testdata/sound_duration_ref.txt").lines() {
            let (c, o) = l.split_once(" -> ").unwrap();
            if o == "fault" {
                continue;
            }
            let (tree, root) = c.split_once(" | ").unwrap();
            let nums = |s: &str| -> Vec<i64> { s.split(',').filter(|v| !v.is_empty()).map(|v| v.parse().unwrap()).collect() };
            let specs: Vec<Spec> = tree.split(' ').map(|s| {
                let (f, k) = s.split_once('/').unwrap();
                (nums(f), nums(k).into_iter().map(|v| v as usize).collect())
            }).collect();
            let lw = specs.iter().find(|s| s.0[0] == 8).map_or(0, |s| s.0[10]);
            let r: Vec<i64> = root.split(' ').map(|v| v.parse().unwrap()).collect();
            let mut node = build(0, &specs, lw);
            node.predelay_ms = r[0] as f32;
            node.start_ms = r[1] as f32;
            let o: Vec<&str> = o.split(' ').collect();
            let (t, f) = dur_total(&node);
            assert_eq!(format!("{t} {}", f as u8), format!("{} {}", o[0], o[1]), "total: {l}");
            if let Kind::Looping { child, count, .. } = &node.kind {
                let (p, off) = looping_split(*count, child, node.tot.1);
                assert_eq!(format!("{} {off}", p.unwrap_or(0x77)), format!("{} {}", o[2], o[3]), "Looping split: {l}");
            }
            if let Kind::Sequential { ahead, on_demand: false, .. } = &node.kind {
                let got = match seq_start(ahead, node.tot.1) {
                    Some((i, off)) => format!("{i} {off}"),
                    None => "-1".into(),
                };
                let want = if o[4] == "-1" { "-1".to_string() } else { format!("{} {}", o[4], o[5]) };
                assert_eq!(got, want, "Sequential start: {l}");
            }
            let (pd, so) = play_offsets(&node, (r[2] as u32, r[3] != 0));
            assert_eq!(format!("{pd} {so}"), format!("{} {}", o[6], o[7]), "play offsets: {l}");
            n += 1;
        }
        assert!(n > 3800, "{n}");
    }

    /// F-13862: the wave's whole ms against the original's own sequence (its constants 0.001, +-2^23 and 1.0 read from the
    /// image): fcfid + frsp of the count and the rate, fmuls, fdivs, the fsel round to nearest, then -1 when it rounded up
    #[test]
    fn wave_ms_is_the_originals() {
        fn orig(n: u32, rate: u32) -> f32 {
            let q = (n as f64) as f32 / ((rate as f64) as f32 * f32::from_bits(0x3A83_126F));
            let m = if q >= 0.0 { -8_388_608.0f32 } else { 8_388_608.0 };
            let r = (q - m) + m;
            if q - r >= 0.0 { r } else { r - 1.0 }
        }
        assert_eq!(f32::from_bits(0x3A83_126F), 0.001);
        let mut x = 0x1234_5678u32;
        for i in 0..200_000u32 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let n = if i < 1000 { i * 48 } else { x % 40_000_000 };
            for rate in [48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 11_025] {
                assert_eq!(super::wave_ms(n, rate).to_bits(), orig(n, rate).to_bits(), "{n} {rate}");
            }
        }
        assert_eq!(super::wave_ms(78_329, 48_000), 1631.0);
    }

    /// F-56973 (pan of a direction, stereo fold) against the original's own code on 4000 cases
    /// (work/audio2/verify/ref5.py, vector shim; the voice's +176 factor unset)
    #[test]
    fn pan_gains_is_the_originals() {
        let h = |x: &str| u32::from_str_radix(&x.replace('_', ""), 16).unwrap();
        let mut bad = 0;
        let mut first = Vec::new();
        for line in include_str!("pan_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<f32> = i.split(' ').map(|x| f32::from_bits(h(x))).collect();
            let w: Vec<u32> = o.split(' ').map(h).collect();
            let (l, r) = pan_gains(Vec3::new(v[0], v[1], v[2]));
            if l.to_bits() != w[0] || r.to_bits() != w[1] {
                bad += 1;
                if first.len() < 4 {
                    first.push((v.clone(), (l, r), (f32::from_bits(w[0]), f32::from_bits(w[1]))));
                }
            }
        }
        assert_eq!(bad, 0, "{first:?}");
    }

    /// F-56973 with the occlusion pair at +176 and the low-pass word +180, against the original's own code on 4000
    /// cases (work/audio2/verify/ref7.py, vector shim): both gains and the word
    #[test]
    fn pan_occluded_is_the_originals() {
        let h = |x: &str| u32::from_str_radix(x.trim_start_matches('x'), 16).unwrap();
        let mut bad = Vec::new();
        for line in include_str!("pan_occ_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<u32> = i.split(' ').map(h).collect();
            let w: Vec<u32> = o.split(' ').map(h).collect();
            let d = Vec3::new(f32::from_bits(v[0]), f32::from_bits(v[1]), f32::from_bits(v[2]));
            let (f, lpf) = if v[6] != 0 { (f32::from_bits(v[3]), occluded_lpf(v[5], f32::from_bits(v[4]))) } else { (1.0, v[5]) };
            let (l, r) = pan_gains_by(d, f);
            if [l.to_bits(), r.to_bits(), lpf] != [w[0], w[1], w[2]] {
                bad.push(line.to_string());
            }
        }
        assert!(bad.is_empty(), "{} differ, first: {:?}", bad.len(), bad.first());
    }

    /// A sound's play F-91431 and updates F-68220 (settings F-63029, play offsets F-90115, attack F-37133, release F-31923,
    /// stop F-58485) against the original's own code (tools/sound_node_reference.py): per call the state, the waiting
    /// flag, what the type does, the volume +40 and the pitch, pan, LPF and HPF handed down.
    #[test]
    fn node_play_and_updates_are_the_originals() {
        use super::*;
        let mut cases = 0;
        let mut calls = 0;
        let mut nears = 0;
        let mut bad = Vec::new();
        let mut lines = include_str!("testdata/sound_node_ref.txt").lines().peekable();
        while let Some(l) = lines.next() {
            let Some(nl) = l.strip_prefix("N") else { continue };
            let f: HashMap<u32, u32> = nl
                .split_whitespace()
                .map(|kv| {
                    let (k, v) = kv.split_once(':').unwrap();
                    (k.parse().unwrap(), v.parse().unwrap())
                })
                .collect();
            let g = |o: u32, d: u32| f.get(&(0x200000 + o)).copied().unwrap_or(d);
            let gp = |o: u32, d: u32| f.get(&(0x200100 + o)).copied().unwrap_or(d);
            let gr = |o: u32, d: u32| f.get(&(0x300000 + o)).copied().unwrap_or(d);
            let has_par = g(5, 255) != 255;
            let has_req = g(4, 255) != 255;
            let b9 = g(8, 0) as u8;
            let mut hb = vec![12u8, 0, 0, 0, 0];
            hb.extend_from_slice(&0xaaaa_aaaau32.to_be_bytes());
            hb.extend_from_slice(&[b9, 0, 0, 0, 0, 0]);
            let (h, _) = audcfg::parse_header(&hb);
            let (pd, so, po) = (g(88, 0), g(104, 0), g(108, 0));
            let n = Node { h, vol: f32::from_bits(g(28, 0)), pitch: g(32, 0) as u16 as i16 as i32, pan: g(10, 0) as u16 as i16, predelay_ms: pd as f32, start_ms: so as f32, st: St::Waiting, t: 0.0, play_t: 0.0, clock: Clock::default(), env0: 0.0, tot: (0, 0), ty_ms: 0, kind: Kind::Empty };
            let (mut parent, _) = root_ctx(&Req::default());
            if has_par {
                parent.vol = f32::from_bits(gp(40, 0));
                parent.lpf = gp(48, 23900) as u16;
                parent.hpf = gp(50, 0) as u16;
                parent.pan = gp(52, 0xffff) as u16 as i16;
                parent.pitch = gp(54, 0) as u16 as i16 as i32;
            }
            let rt = Rt {
                vol: f32::from_bits(g(36, 0)),
                pitch: g(34, 0) as u16 as i16 as i32 + if has_req { gr(80, 0) as u16 as i16 as i32 } else { 0 },
                pan: if has_req { gr(82, 0) as u16 as i16 } else { -1 },
                rpan: g(124, 0xffff) as u16 as i16,
                lpf: has_req.then(|| gr(88, 0) as u16),
                hpf: has_req.then(|| gr(90, 0) as u16),
            };
            let tot = play_offsets(&n, (po, false));
            let at = Attack { ms: g(68, 0), po, own_pd: pd, ppd: 0 };
            let flags = g(58, 0x50) as u8;
            let rel = if has_req && gr(192, u32::MAX) as i32 != -1 { gr(192, 0) as i32 } else { g(136, u32::MAX) as i32 };
            let mut clock = Clock::default();
            let mut ended_at_stop = false;
            cases += 1;
            while let Some(&c) = lines.peek() {
                if c == "=" {
                    lines.next();
                    break;
                }
                lines.next();
                let (head, o) = c.split_once(" -> ").unwrap_or((c, ""));
                let mut hw = head.split(' ');
                let (k, now) = (hw.next().unwrap(), hw.next().unwrap().parse::<u32>().unwrap());
                if k == "s" {
                    ended_at_stop = !clock.stop(now, rel);
                    continue;
                }
                calls += 1;
                let w: Vec<&str> = o.split(' ').collect();
                // (the play returns nothing: its r3 is whatever was left)
                let (r, st, wt) = (if k == "p" { 1 } else { w[0].parse::<i32>().unwrap() }, w[1], w[2]);
                let vol40 = u32::from_str_radix(w[3], 16).unwrap();
                let called = w.get(9).copied().unwrap_or("");
                let (v, pi, pa, lp, hp) = settings(b9, n.vol, n.pitch, n.pan, &parent, &rt);
                let before = clock;
                let (act, v) = if ended_at_stop { (Act::End, v) } else { clock.tick(now, tot.0, at, flags, v) };
                // the one rounding the port may differ in: the recomp's vector log2 (vlogefp) is the platform's f32 log2,
                // lin2db rounds the f64 one (correctly rounded everywhere); a mismatch must be exactly that
                let f32db = |x: f32| if x - 1e-5 >= 0.0 { x.log2() * 0.301 * 20.0 } else { -100.0 };
                let base = settings(b9, n.vol, n.pitch, n.pan, &parent, &rt).0;
                let alt = match (before.play_at, before.rel) {
                    (Some(_), Some((a, b))) if !ended_at_stop => {
                        let x = if b <= a || now >= b { 0.0 } else { (b - now) as f32 / (b - a) as f32 };
                        Some(base + f32db(x))
                    }
                    (Some(t0), None) if !before.stopped => at.frac(t0, now).map(|x| base + f32db(x)),
                    (None, _) => at.frac(now, now).map(|x| base + f32db(x)),
                    _ => None,
                };
                let near = v.to_bits() != vol40 && alt.is_some_and(|a| a.to_bits() == vol40);
                if near {
                    nears += 1;
                }
                let v = if near { f32::from_bits(vol40) } else { v };
                let ok_act = match act {
                    Act::Wait => r == 1 && wt == "1" && called.is_empty(),
                    Act::Start => r == 1 && wt == "2" && called == "P",
                    Act::Run => called == "U" && (r == 1 || st == "3"),
                    Act::End => r == 0 && st == "3" && called.ends_with('K'),
                };
                let same = v.to_bits() == vol40
                    && pi == w[5].parse::<i32>().unwrap()
                    && pa as i32 == w[6].parse::<i32>().unwrap()
                    && lp as u32 == w[7].parse::<u32>().unwrap()
                    && hp as u32 == w[8].parse::<u32>().unwrap();
                if !(ok_act && same) && bad.len() < 20 {
                    bad.push(format!("{nl} | {c} | port {act:?} {:08x} {pi} {pa} {lp} {hp}", v.to_bits()));
                }
                if !(ok_act && same) && bad.len() >= 20 {
                    bad.push(String::new());
                }
            }
        }
        assert!(cases > 1000 && calls > 40_000, "{cases} {calls}");
        assert!(bad.is_empty(), "{} wrong of {calls}:\n{}", bad.len(), bad.join("\n"));
        println!("{cases} cases, {calls} calls, {nears} differ only in the log2 rounding");
        assert!(nears <= 40, "{nears}");
    }

    /// A stop without a release (F-58485 with F-84762 / F-13519): the group's children stop with their own releases, the
    /// group ends at the first update with none playing.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn a_stop_without_release_waits_for_the_children() {
        use super::*;
        let node = |release: i16, kind: Kind| {
            let mut hb = vec![12u8, 0, 0, 0, 0];
            hb.extend_from_slice(&0xaaaa_aaaau32.to_be_bytes());
            hb.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
            let (mut h, _) = audcfg::parse_header(&hb);
            h.release = release;
            Node { h, vol: 0.0, pitch: 0, pan: -1, predelay_ms: 0.0, start_ms: 0.0, st: St::Waiting, t: 0.0, play_t: 0.0, clock: Clock::default(), env0: 0.0, tot: (0, 0), ty_ms: 0, kind }
        };
        // (the group's own update, F-29161, ends it in the update its last child ends)
        // a child that plays until stopped: a Retriggered with no child to make and no end
        let alive = |release: i16| node(release, Kind::Retrig { child_hash: 0, count: -1, delay_ms: 1e9, next_ms: 1e9, made: 0, live: Vec::new() });
        let mut g = node(-1, Kind::Group { children: vec![alive(100), alive(-1)] });
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let (ctx, rt) = root_ctx(&Req::default());
        let mut out = Vec::new();
        for now in (0..200).step_by(10) {
            if now == 50 {
                stop_node(&mut g, -1, now);
            }
            lib.step(&mut g, 10.0, now, ctx, rt, (0, false), &mut out);
            let Kind::Group { children } = &g.kind else { unreachable!() };
            let st: Vec<St> = children.iter().map(|c| c.st).collect();
            match now {
                0..=40 => assert!(g.st == St::Playing && st == [St::Playing, St::Playing], "{now} {st:?}"),
                // the release-less child ends at the stop's update, the other releases until 150
                50..=140 => assert!(g.st == St::Playing && st == [St::Playing, St::Done], "{now} {:?} {st:?}", g.st),
                _ => assert_eq!(g.st, St::Done, "{now}"),
            }
        }
    }

    /// F-31923 (release step) against the original's own code on 4000 cases (work/audio2/verify/ref4.py, vector shim)
    #[test]
    fn release_step_is_the_originals() {
        let h = |x: &str| u32::from_str_radix(&x.replace('_', ""), 16).unwrap();
        let mut bad = 0;
        for line in include_str!("release_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<&str> = i.split(' ').collect();
            let w: Vec<&str> = o.split(' ').collect();
            let (vol, alive) = release_step(v[0].parse().unwrap(), v[1].parse().unwrap(), v[2].parse().unwrap(), f32::from_bits(h(v[3])), v[4].parse().unwrap());
            if vol.to_bits() != h(w[0]) || alive != (w[1] == "1") {
                bad += 1;
            }
        }
        assert_eq!(bad, 0);
    }

    /// F-60851 against the original's own code on 4000 cases (work/audio2/verify/ref2.py): value and next state
    #[test]
    fn mwc_float_is_the_originals() {
        let mut bad = 0;
        for line in include_str!("mwc_float_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<u32> = i.split(' ').map(|x| u32::from_str_radix(&x.replace('_', ""), 16).unwrap()).collect();
            let w: Vec<u32> = o.split(' ').map(|x| u32::from_str_radix(&x.replace('_', ""), 16).unwrap()).collect();
            let mut m = Mwc { lo: v[0], carry: v[1] };
            let f = m.float(f32::from_bits(v[2]), f32::from_bits(v[3]));
            if f.to_bits() != w[0] || m.lo != w[1] || m.carry != w[2] {
                bad += 1;
            }
        }
        assert_eq!(bad, 0);
    }
    use super::*;

    /// A walker's reaction sound against F-39461 run offline (tools/ped_reaction_reference.py ->
    /// testdata/ped_reaction_ref.txt): 4,000 cases of kind 0 .. 3, bike byte 0 / 1 / 2 / 0xFF, the type's sex byte
    /// 0 / 1 / 0x80, with and without the type's object and the fallback: the lookups made (the type's name, then
    /// AMBIENT_PED_FEMALE_01 when +109 is set, else AMBIENT_PED_MALE_01) and the sound word started equal; where the
    /// original reads through a null object (66 cases) the port plays nothing.
    #[test]
    fn ped_reaction_matches_the_original() {
        let (mut n, mut none) = (0, 0);
        for l in include_str!("testdata/ped_reaction_ref.txt").lines() {
            let (ci, o) = l.split_once(" -> ").unwrap();
            let c: Vec<u32> = ci.split(' ').map(|t| u32::from_str_radix(&t.replace('_', ""), 16).unwrap()).collect();
            let (log, word) = o.split_once(' ').unwrap();
            let bytes = |at: usize| c[at..at + 9].iter().flat_map(|w| w.to_be_bytes()).take(34).collect::<Vec<u8>>();
            let (own, fb) = (bytes(6), bytes(15));
            let (kind, bike, female, named, has_own, has_fb) = (c[0], c[1] != 0, c[2] != 0, c[3] != 0, c[4] != 0, c[5] != 0);
            let own = (named && has_own).then_some(own.as_slice());
            let want_log = format!("{}{}", if named { "T" } else { "" }, if own.is_some() { "" } else if female { "F" } else { "M" });
            assert_eq!(log, want_log, "{l}");
            let got = ped_reaction_pick(own.or(has_fb.then_some(fb.as_slice())), kind, bike);
            match word {
                "X" => {
                    assert_eq!(got, None, "{l}");
                    none += 1;
                }
                w => assert_eq!(got, Some(u32::from_str_radix(&w.replace('_', ""), 16).unwrap()), "{l}"),
            }
            n += 1;
        }
        assert_eq!((n, none), (4000, 66));
    }

    /// The streamed decode gives the same samples as the whole decode (MCLA_AUDIO_DIR = the converted audio,
    /// `<data>/../audio_pc`): every 25th stream of the stream folders, and the landmark bells (4800 Hz on the disc,
    /// 8000 Hz in the MP3: their length had been taken at the disc's rate, so the end of every bell was cut off).
    #[test]
    #[ignore = "requires MCLA_AUDIO_DIR"]
    fn streamed_decode_matches_whole() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_AUDIO_DIR").expect("MCLA_AUDIO_DIR"));
        let mut n = 0;
        for dir in ["ambience_stream", "vehicles_stream", "ambience_landmarks", "music"] {
            let mut files: Vec<_> = std::fs::read_dir(root.join(dir)).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "mp3")).collect();
            files.sort();
            let bell = |p: &std::path::Path| std::fs::read_to_string(format!("{}.txt", p.display())).is_ok_and(|t| t.contains("	BELL_"));
            for p in files.iter().enumerate().filter(|(i, p)| i % 25 == 0 || bell(p)).map(|(_, p)| p) {
                let a = decode_mp3(p).unwrap();
                let b = decode_mp3_whole(p).unwrap();
                let Some(s) = a.stream.as_ref() else { continue };
                while !s.done.load(std::sync::atomic::Ordering::Acquire) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                assert_eq!((a.frames(), a.channels, a.rate), (b.frames(), b.channels, b.rate), "{}", p.display());
                for (k, v) in b.data.iter().enumerate() {
                    assert_eq!(s.samples[k].load(std::sync::atomic::Ordering::Relaxed), v.to_bits(), "{} sample {k}", p.display());
                }
                n += 1;
            }
        }
        assert!(n > 20, "{n}");
        eprintln!("{n} streams equal");
    }

    #[test]
    fn pan_law() {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        // straight ahead: 0.7071 each side
        let (l, r) = pan_gains(Vec3::Z);
        assert!((l - k).abs() < 1e-6 && (r - k).abs() < 1e-6);
        // right: FR = k, SR = k -> R = k + k*k; L = 0
        let (l, r) = pan_gains(Vec3::X);
        assert!(l.abs() < 1e-6 && (r - (k + 0.5)).abs() < 1e-6, "{l} {r}");
        // behind: SL = SR = k -> 0.5 each
        let (l, r) = pan_gains(Vec3::NEG_Z);
        assert!((l - 0.5).abs() < 1e-6 && (r - 0.5).abs() < 1e-6);
        // straight up: h = 0 -> every speaker 0.5
        let (l, r) = pan_gains(Vec3::Y);
        assert!((l - (0.5 + 0.5 * k)).abs() < 1e-6 && (r - l).abs() < 1e-6);
    }

    /// The slow-down state's SFX_HARD record (career2 `audioramp/notes.md`): its pitch ratio falls at 1 a second to
    /// 0.1 (-3986 cents at the end) and rises back the same way; its children (opponents' engines, bangers, tyres)
    /// take it, the player's vehicle categories do not.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn slowdown_sfx_hard_pitch() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let p = |lib: &SoundLib, n: &str| lib.category(n).unwrap().pitch;
        let (hard, opp, eng) = (p(&lib, "SFX_HARD"), p(&lib, "OPPONENT_VEHICLES_ENGINE"), p(&lib, "PLAYER_VEHICLES_ENGINES"));
        lib.set_time_slow(true);
        lib.step_scenes(0.5);
        assert_eq!(p(&lib, "SFX_HARD") - hard, (1200.0 * 0.5f32.log2()) as i32);
        lib.step_scenes(1.0);
        assert_eq!(p(&lib, "SFX_HARD") - hard, -3986);
        assert_eq!(p(&lib, "OPPONENT_VEHICLES_ENGINE") - opp, -3986);
        assert_eq!(p(&lib, "PLAYER_VEHICLES_ENGINES"), eng);
        lib.set_time_slow(false);
        lib.step_scenes(0.4);
        assert_eq!(p(&lib, "SFX_HARD") - hard, (1200.0 * 0.5f32.log2()) as i32);
        lib.step_scenes(1.0);
        assert_eq!(p(&lib, "SFX_HARD"), hard);
    }

    /// A story scene's audio (F-16992): the new career's two scenes - CUT_INTRO has its stream and no effects,
    /// GH_06 its dialogue and its effects stream; GH_06's mixer scene takes the cars out (-100 dB) and ambient traffic
    /// down 8 dB.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn story_scene_audio() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let (p, s) = lib.cutscene_parts("CUT_INTRO", false).unwrap();
        assert_eq!(p, vec![joaat("CUT_INTRO_MASTER")]);
        assert_eq!(s.as_deref(), Some("DYNAMIC_MIXER_CUTSCENE_INTRO"));
        let (p, s) = lib.cutscene_parts("CUT_STORY_RACE_GH_06", true).unwrap();
        assert_eq!(p, vec![joaat("CUT_STORY_RACE_GH_06_MASTER"), joaat("SFX_CUT_STORY_RACE_GH_06_MASTER")]);
        let s = s.unwrap();
        assert_eq!(s, "DYNAMIC_MIXER_CUTSCENE_CUT_STORY_RACE_GH_06");
        let v = |lib: &SoundLib, n: &str| lib.category(n).unwrap().volume;
        let (pv, av) = (v(&lib, "PLAYER_VEHICLES"), v(&lib, "AMBIENT_VEHICLES"));
        lib.scene(&s, true);
        lib.step_scenes(10.0);
        assert!(v(&lib, "PLAYER_VEHICLES") - pv < -99.0);
        assert!((v(&lib, "AMBIENT_VEHICLES") - av + 8.0).abs() < 0.01);
        assert!(lib.cutscene_parts("CUT_STORY_NO_SUCH", false).is_none());
    }

    /// The arrest's audio (core_rolling_prototype's stream, F-27055 -> F-16992): CUTSCENES_CUT_COP_ARREST plays the
    /// officers' lines (one of the five takes) and the scene's effects stream, no mixer scene.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn arrest_scene_audio() {
        fn streams(n: &Node, out: &mut Vec<f32>) {
            match &n.kind {
                Kind::Stream { clip: Some(_), dur_ms, .. } => out.push(*dur_ms),
                Kind::Group { children } | Kind::Twin { children, .. } => children.iter().for_each(|c| streams(c, out)),
                Kind::Looping { child, .. } | Kind::Envelope { child, .. } | Kind::OnStop { child, .. } => streams(child, out),
                Kind::Sequential { cur: Some(c), .. } => streams(c, out),
                _ => {}
            }
        }
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let (p, s) = lib.cutscene_parts("CUT_COP_ARREST", false).unwrap();
        assert_eq!(p, vec![joaat("CUT_COP_ARREST_MASTER"), joaat("CUT_COP_ARREST_SFX_MASTER")]);
        assert_eq!(s, None);
        let mut clips = Vec::new();
        for h in p {
            let h = lib.play_hash(h, Req::default()).expect("plays");
            streams(&lib.insts.iter().find(|i| i.id == h.0).unwrap().root, &mut clips);
        }
        // (the lines' take and the effects: each a converted stream of the scene's length)
        assert!(clips.len() >= 2 && clips.iter().all(|&ms| ms > 3000.0), "{clips:?}");
    }

    /// VOLUME_RULE.md: slider 5 = -6.02 dB on its category and its children; the camera table's engine +3 dB in the
    /// chase views; PAUSE fades in by its own time.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn sliders_camera_scenes() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let v = |lib: &SoundLib, n: &str| lib.category(n).unwrap().volume;
        let (sfx, amb, eng, music) = (v(&lib, "SFX"), v(&lib, "AMBIENCE"), v(&lib, "PLAYER_VEHICLES_ENGINES"), v(&lib, "MUSIC_USER"));
        assert!((music + 17.5).abs() < 0.05, "{music}");
        lib.set_sliders([5, 10, 10, 10, 10]);
        assert!((v(&lib, "SFX") - sfx + 6.0206).abs() < 0.01);
        assert!((v(&lib, "AMBIENCE") - amb + 6.0206).abs() < 0.01);
        assert!((v(&lib, "MUSIC_USER") - music).abs() < 1e-4);
        lib.set_sliders([10, 0, 10, 10, 10]);
        assert!(v(&lib, "MUSIC_USER") < -100.0);
        lib.set_sliders([10; 5]);
        lib.set_camera_mode(2, false);
        assert!((v(&lib, "PLAYER_VEHICLES_ENGINES") - eng - 3.0).abs() < 0.01);
        lib.set_camera_mode(1, false);
        assert!((v(&lib, "PLAYER_VEHICLES_ENGINES") - eng - 4.0).abs() < 0.01);
        lib.set_camera_mode(-1, false);
        assert!((v(&lib, "PLAYER_VEHICLES_ENGINES") - eng).abs() < 1e-4);
        lib.scene("DYNAMIC_MIXER_COP_FORM", true);
        lib.step_scenes(10.0);
        assert!(v(&lib, "MUSIC_USER") < -100.0);
        lib.scene("DYNAMIC_MIXER_COP_FORM", false);
        lib.step_scenes(10.0);
        assert!((v(&lib, "MUSIC_USER") - music).abs() < 1e-4);
        // (MIXER_SCENES_RULE.md 1.3: ENDRACE is exclusive - it stops PAUSE and refuses it; ROAR stops itself after 1.75 s)
        lib.scene("DYNAMIC_MIXER_PAUSE", true);
        assert!(lib.scene_on("DYNAMIC_MIXER_PAUSE"));
        lib.scene("DYNAMIC_MIXER_ENDRACE", true);
        lib.step_scenes(0.1);
        assert!(!lib.scene_on("DYNAMIC_MIXER_PAUSE"));
        lib.scene("DYNAMIC_MIXER_PAUSE", true);
        assert!(!lib.scene_on("DYNAMIC_MIXER_PAUSE"));
        lib.scene("DYNAMIC_MIXER_ENDRACE", false);
        lib.scene("DYNAMIC_MIXER_ROAR", true);
        lib.step_scenes(1.0);
        assert!(lib.scene_on("DYNAMIC_MIXER_ROAR"));
        lib.step_scenes(1.0);
        assert!(!lib.scene_on("DYNAMIC_MIXER_ROAR"));
        // MANAGER_OPPONENT_GAINS: four racers close by -> -6 dB on PLAYER_VEHICLES_ENGINES
        lib.step_scenes(10.0);
        let e0 = v(&lib, "PLAYER_VEHICLES_ENGINES");
        lib.set_presence(4.0);
        assert!((v(&lib, "PLAYER_VEHICLES_ENGINES") - e0 + 6.0).abs() < 0.01);
        lib.set_presence(0.5);
        assert!((v(&lib, "PLAYER_VEHICLES_ENGINES") - e0).abs() < 1e-4);
    }

    #[test]
    fn mwc_ranges() {
        let mut g = Mwc { lo: 1, carry: 0 };
        for _ in 0..1000 {
            let i = g.int(-3, 3);
            assert!((-3..=3).contains(&i));
            let f = g.float(2.0, 5.0);
            assert!((2.0..5.0).contains(&f));
        }
    }

    /// Which timer a sound runs on (docs/reference/audio/SOUND_TIMER_RULE.md): categories.dat pair 1 from BASE down puts
    /// MUSIC, AMBIENCE (not its weather beds), UI, the end-of-race and replay effects, SPEECH_ENDRACE and ZZZ_DEBUG and all below
    /// them on timer 1, the other 117 on timer 0; with timer 0 paused a UI sound plays on and a game sound holds.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn sound_timers_by_category() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let mut a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let one: Vec<&str> = lib.cat_dat.objects.iter().map(|o| o.0.as_str()).filter(|n| lib.cat_timer.get(&joaat(n)) == Some(&1)).collect();
        assert_eq!(lib.cat_timer.len(), 145, "every category below BASE (one of the 146 is in no tree; the original builds 145 records)");
        assert_eq!(one.len(), 29, "{one:?}");
        for n in ["MUSIC", "MUSIC_USER", "AMBIENCE", "AMBIENCE_BEDS_NIGHT", "UI", "UI_MENU", "UI_SAVE", "SPECIAL_EFFECTS_END_OF_RACE", "SPECIAL_EFFECTS_REPLAY", "SPEECH_ENDRACE", "ZZZ_DEBUG"] {
            assert_eq!(lib.cat_timer.get(&joaat(n)), Some(&1), "{n}");
        }
        for n in ["BASE", "AMBIENCE_BEDS_WEATHER", "SPEECH_CAREER", "SPEECH_CAREER_POS", "PLAYER_VEHICLES_ENGINES", "SFX"] {
            assert_eq!(lib.cat_timer.get(&joaat(n)).copied().unwrap_or(0), 0, "{n}");
        }
        let key = format!("career_speech/0x0624a35b/{:08x}", joaat("PLAYER_NITROUS_07"));
        let ui = lib.play_clip(&key, "UI_MENU", Req::default()).expect("clip");
        let game = lib.play_clip(&key, "SPEECH_CAREER", Req::default()).expect("clip");
        let pos = |lib: &SoundLib, h: Handle| match &lib.insts.iter().find(|i| i.id == h.0).unwrap().root.kind {
            Kind::Wave { pos_ms, .. } => *pos_ms,
            _ => -1.0,
        };
        assert_eq!(lib.insts.iter().map(|i| i.timer).collect::<Vec<_>>(), vec![Some(1), Some(0)]);
        lib.update(0.1, &mut a);
        let (u0, g0) = (pos(&lib, ui), pos(&lib, game));
        lib.timer_paused[0] = true;
        for _ in 0..5 {
            lib.update(0.1, &mut a);
        }
        assert_eq!(pos(&lib, game), g0, "the game sound holds");
        assert!((pos(&lib, ui) - u0 - 500.0).abs() < 1.0, "the UI sound plays on");
    }

    /// A career speech line as the original plays it (work/slipstream/audio4/cap/n2.txt): PLAYER_NITROUS through the player
    /// voice's Speech sound CAREER_SPEECH_DRV_MP_01_SET_POS_YELL (SND line: request defaults, category SPEECH_CAREER_POS -> timer 0), whose
    /// category SPEECH_CAREER_POS resolves to 3.4997 dB at the sliders 10 (CAT 98) and own volume -3 dB, Doppler 0; on the
    /// paused timer 0 (F-89113 skips it; recorded: t0 held at 93.4585 through a 10.26 s pause) the line's wave does not move.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn speech_line_object_and_timer() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let mut a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        lib.set_sliders([10; 5]);
        let c = lib.category("SPEECH_CAREER_POS").expect("category").volume;
        assert!((c - 3.4997).abs() < 0.0015, "SPEECH_CAREER_POS {c}");
        let key = format!("career_speech/0x0624a35b/{:08x}", joaat("PLAYER_NITROUS_07"));
        let h = lib.play_clip(&key, "CAREER_SPEECH_DRV_MP_01_SET_POS_YELL", Req::at(Vec3::new(10.0, 0.0, 0.0))).expect("clip");
        lib.set_timer(h, 0);
        {
            let i = lib.insts.iter().find(|i| i.id == h.0).unwrap();
            assert_eq!((i.root.h.category, i.root.vol, i.root.h.doppler), (joaat("SPEECH_CAREER_POS"), -3.0, 0));
        }
        let pos = |lib: &SoundLib| match &lib.insts.iter().find(|i| i.id == h.0).unwrap().root.kind {
            Kind::Wave { pos_ms, .. } => *pos_ms,
            _ => -1.0,
        };
        lib.update(0.1, &mut a);
        let p0 = pos(&lib);
        lib.timer_paused[0] = true;
        for _ in 0..30 {
            lib.update(0.1, &mut a);
        }
        assert_eq!(pos(&lib), p0, "held in the pause");
        lib.timer_paused[0] = false;
        lib.update(0.1, &mut a);
        assert!((pos(&lib) - p0 - 100.0).abs() < 1.0, "goes on after the pause");
        // (still playing)
        assert!(lib.playing(h));
    }

    /// F-39461's sounds: every ambient type's szAudioType (tune/ambients/peds/*/mcAmbientType.xml) has its AMBIENT_PED
    /// object in game.dat, and each of its six fields names a sound of sounds.dat (<type>_QUICK / _SLOW /
    /// _EXPENSIVE_CAR / _DAMAGED / _EXPENSIVE_BIKE / _DAMAGED) that starts with a wave of the converted banks.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn ped_reaction_sounds() {
        fn waves(n: &Node) -> usize {
            match &n.kind {
                Kind::Wave { wave: Some(_), .. } => 1,
                Kind::Group { children } | Kind::Twin { children, .. } => children.iter().map(waves).sum(),
                Kind::Looping { child, .. } | Kind::Envelope { child, .. } | Kind::OnStop { child, .. } => waves(child),
                Kind::Sequential { cur: Some(c), .. } => waves(c),
                _ => 0,
            }
        }
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let mut types: Vec<String> = std::fs::read_dir(Path::new(&dir).join("tune/ambients/peds"))
            .unwrap()
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("mcAmbientType.xml")).ok())
            .filter_map(|t| Some(t.split("<szAudioType>").nth(1)?.split('<').next()?.trim().to_string()))
            .collect();
        types.sort();
        types.dedup();
        assert_eq!(types.len(), 39);
        let (mut missing, mut odd) = (Vec::new(), Vec::new());
        let tails = [(0, false, "QUICK"), (1, false, "SLOW"), (2, false, "EXPENSIVE_CAR"), (3, false, "DAMAGED"), (2, true, "EXPENSIVE_BIKE"), (3, true, "DAMAGED")];
        for t in &types {
            for (kind, bike, tail) in tails {
                let h = lib.ped_reaction(t, false, kind, bike).unwrap_or_else(|| panic!("{t} {kind}"));
                let name = lib.name_of(h).unwrap_or_else(|| panic!("{t} {tail}: not in sounds.dat")).to_string();
                if name != format!("{}_{tail}", t.trim_start_matches("AMBIENT_PED_")) {
                    odd.push(format!("{t} {tail} -> {name}"));
                }
                let s = lib.play_hash(h, Req::at(Vec3::ZERO)).unwrap_or_else(|| panic!("{t} {tail} plays"));
                if waves(&lib.insts.iter().find(|i| i.id == s.0).unwrap().root) == 0 {
                    missing.push(format!("{t} {tail}"));
                }
                lib.stop(s);
            }
        }
        // (the data's own: five fields name another type's or a plural sound)
        assert_eq!(
            odd,
            [
                "AMBIENT_PED_FC_005 DAMAGED -> FC_001_DAMAGED",
                "AMBIENT_PED_FC_006 EXPENSIVE_CAR -> FC_006_EXPENSIVE_CARS",
                "AMBIENT_PED_FC_006 EXPENSIVE_BIKE -> FC_006_EXPENSIVE_BIKES",
                "AMBIENT_PED_MC_003 QUICK -> MC_003_EXPENSIVE_QUICK",
                "AMBIENT_PED_MC_003 SLOW -> MC_003_EXPENSIVE_SLOW"
            ]
        );
        // (the disc has no AMBIENT_PEDS\FC_006 wave bank - 82 banks in sfx/AMBIENT_PEDS, none of them FC_006: its
        // sounds play nothing in the original too)
        assert!(missing.iter().all(|m| m.starts_with("AMBIENT_PED_FC_006 ")) && missing.len() == 6, "{missing:?}");
        // (no type's object: the fallbacks, which game.dat has not got)
        assert_eq!(lib.ped_reaction("", true, 2, false), None);
        assert_eq!(lib.ped_reaction("AMBIENT_PED_NONE", false, 3, false), None);
    }
}
