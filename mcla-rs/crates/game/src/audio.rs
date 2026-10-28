//! Game audio from the original sound banks.
//!
//! `tools/convert_audio.py` unwraps the disc's XMA banks into `<data>/../audio_pc`:
//! one folder per bank with a WAV per wave (named after the wave) and one MP3 per
//! stream (music, ambience). This module mixes them: engines are cross-faded between
//! the per-rpm loops of the car's bank exactly like the game's `ENGINE_XFADER` objects,
//! everything else is one-shots and looping voices with distance attenuation and pan.

use glam::Vec3;
use hashbrown::HashMap;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RATE: u32 = 44100;
const TAB: char = '\u{9}';

/// Decoded PCM: mono, or interleaved stereo (folded streams).
pub struct Clip {
    pub data: Vec<f32>,
    pub rate: u32,
    /// 1 or 2
    pub channels: usize,
    /// the frame a looping wave returns to (the wave info's loop start; 0 for the old loops)
    pub loop_start: usize,
    /// a stream decoded while it plays (`sndobj::decode_mp3`): its samples, filled in by a thread; `data` is empty
    pub stream: Option<Arc<Progressive>>,
}

/// The samples of a stream being decoded on a thread of its own: the length is known before (the converted
/// stream's `.mp3.txt`), the samples arrive in order, about 2000 times faster than they play
/// (docs/PERFORMANCE.md, 2026-10-23: a whole stream decoded at once stopped the game for 100 - 700 ms).
pub struct Progressive {
    /// f32 bits, interleaved like `Clip::data`; 0 until decoded
    pub samples: Box<[std::sync::atomic::AtomicU32]>,
    pub frames: usize,
    /// set when the thread has finished
    pub done: std::sync::atomic::AtomicBool,
}

impl Progressive {
    pub fn zeroed(samples: usize, frames: usize) -> Progressive {
        let b: Box<[u32]> = vec![0u32; samples].into_boxed_slice();
        // (AtomicU32 has the same in-memory representation as u32)
        let samples = unsafe { Box::from_raw(Box::into_raw(b) as *mut [std::sync::atomic::AtomicU32]) };
        Progressive { samples, frames, done: std::sync::atomic::AtomicBool::new(false) }
    }
}

impl Clip {
    pub fn mono(data: Vec<f32>, rate: u32) -> Clip {
        Clip { data, rate, channels: 1, loop_start: 0, stream: None }
    }
    /// Frames (samples per channel).
    pub fn frames(&self) -> usize {
        match &self.stream {
            Some(s) => s.frames,
            None => self.data.len() / self.channels.max(1),
        }
    }
    /// The sample of channel `c` at frame `i` (a mono clip gives the same to both).
    fn at(&self, i: usize, c: usize) -> f32 {
        let k = i * self.channels + c.min(self.channels - 1);
        match &self.stream {
            Some(s) => s.samples.get(k).map_or(0.0, |v| f32::from_bits(v.load(std::sync::atomic::Ordering::Relaxed))),
            None => self.data[k],
        }
    }
}

pub fn load_wav_clip(p: &Path) -> Option<Clip> {
    load_wav(p)
}

fn load_wav(p: &Path) -> Option<Clip> {
    let d = std::fs::read(p).ok()?;
    if d.len() < 44 || &d[0..4] != b"RIFF" {
        return None;
    }
    let mut o = 12;
    let mut rate = RATE;
    let mut ch = 1usize;
    while o + 8 <= d.len() {
        let id = &d[o..o + 4];
        let n = u32::from_le_bytes(d[o + 4..o + 8].try_into().ok()?) as usize;
        let body = o + 8;
        if id == b"fmt " && body + 16 <= d.len() {
            ch = u16::from_le_bytes([d[body + 2], d[body + 3]]).max(1) as usize;
            rate = u32::from_le_bytes(d[body + 4..body + 8].try_into().ok()?);
        } else if id == b"data" {
            let end = (body + n).min(d.len());
            let data: Vec<f32> = d[body..end].chunks_exact(2 * ch).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
            return Some(Clip::mono(data, rate));
        }
        o = body + n + (n & 1);
    }
    None
}

struct Voice {
    /// 0 = one-shot, otherwise the id of a looping voice the game keeps updating
    id: u64,
    clip: Arc<Clip>,
    pos: f64,
    speed: f32,
    /// current and target gain per ear (smoothed to avoid clicks)
    gl: f32,
    gr: f32,
    tl: f32,
    tr: f32,
    looping: bool,
    /// loop voices fade out and die when the game stops updating them
    fresh: bool,
    /// held by a paused audio timer (`Audio::hold`): not mixed, its position kept
    held: bool,
}

#[derive(Default)]
struct Mixer {
    voices: Vec<Voice>,
    master: f32,
    /// (step 23b) the dynamic mixer's slow-down state: (gain, pitch) of the player's vehicle
    /// and the ambience's gain, ramped linearly from `from` to `to` over `dur` seconds
    slow: Option<SlowRamp>,
}

/// AudioManager_SetActiveTimeSlow's mixer state (DYNAMIC_MIXER_SLOWDOWN): the categories'
/// settings as the category controllers ramp them (sub_821EC620 / sub_821EC060 -> 0x82547908).
#[derive(Clone, Copy, Debug)]
pub struct SlowRamp {
    /// [player vehicle gain, player vehicle pitch, ambience gain]
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub start: std::time::Instant,
    pub dur: f32,
}

impl SlowRamp {
    pub fn now(&self) -> [f32; 3] {
        self.at(self.start.elapsed().as_secs_f32())
    }

    /// The values `t` seconds into the ramp. A category controller's ramp (sub_825484C8 /
    /// sub_82548768, audio thread) moves at a fixed speed, not over a fixed time: rate =
    /// K / min(time, 10 s) per second toward the target (K = 1 for gain - linear amplitude -
    /// and pitch ratio), snapped to it when passed; a time <= 0 sets the value at once.
    pub fn at(&self, t: f32) -> [f32; 3] {
        let d = self.dur.min(10.0);
        [0, 1, 2].map(|i| {
            let gap = self.to[i] - self.from[i];
            if d <= 0.0 {
                return self.to[i];
            }
            self.from[i] + gap.signum() * (t.max(0.0) / d).min(gap.abs())
        })
    }
}

/// DYNAMIC_MIXER_SLOWDOWN's settings: PLAYER_VEHICLES -1000 mB (10^(-10/20)) at pitch 0.5,
/// AMBIENCE -10000 mB (silent - sub_821493F8 gives 0 at -100 dB and under).
pub const SLOWDOWN_ON: [f32; 3] = [0.316_227_77, 0.5, 0.0];
pub const SLOWDOWN_OFF: [f32; 3] = [1.0, 1.0, 1.0];
/// the state's fade in (+30) and out (+34), seconds
pub const SLOWDOWN_FADE: (f32, f32) = (1.0, 1.0);

struct MixSource {
    mixer: Arc<Mutex<Mixer>>,
    buf: Vec<f32>,
    at: usize,
    /// samples the output has pulled (the output watch tells a stalled stream by it)
    pulled: Arc<AtomicU64>,
}

const BLOCK: usize = 256;
/// the offline capture's stream voice ([`Audio::play_stream`])
const STREAM_VOICE: u64 = u64::MAX - 1;

impl MixSource {
    fn refill(&mut self) {
        self.buf.clear();
        self.buf.resize(BLOCK * 2, 0.0);
        self.at = 0;
        self.pulled.fetch_add((BLOCK * 2) as u64, Ordering::Relaxed);
        let Ok(mut m) = self.mixer.lock() else { return };
        let master = m.master;
        // (step 23b: the slow-down state on the player's vehicle - the engine's slot 0)
        let slow = m.slow.map(|r| r.now()).unwrap_or(SLOWDOWN_OFF);
        for v in m.voices.iter_mut() {
            if v.held {
                continue;
            }
            let mine = (v.id >= 0x1000_0000 && v.id < 0x1000_0040) || v.id & crate::sndobj::SLOWED != 0;
            let (cat_gain, cat_pitch) = if mine { (slow[0], slow[1]) } else { (1.0, 1.0) };
            let n = v.clip.frames();
            if n < 2 {
                v.tl = 0.0;
                v.tr = 0.0;
                v.looping = false;
                v.pos = n as f64;
                continue;
            }
            let step = (v.speed * cat_pitch * v.clip.rate as f32 / RATE as f32) as f64;
            for i in 0..BLOCK {
                v.gl += (v.tl - v.gl) * 0.004;
                v.gr += (v.tr - v.gr) * 0.004;
                let mut p = v.pos;
                if p >= (n - 1) as f64 {
                    if v.looping {
                        let ls = v.clip.loop_start.min(n - 2) as f64;
                        p = ls + (p - (n - 1) as f64) % ((n - 1) as f64 - ls).max(1.0);
                        v.pos = p;
                    } else {
                        break;
                    }
                }
                let i0 = p as usize;
                let f = (p - i0 as f64) as f32;
                let sl = v.clip.at(i0, 0) * (1.0 - f) + v.clip.at(i0 + 1, 0) * f;
                let sr = v.clip.at(i0, 1) * (1.0 - f) + v.clip.at(i0 + 1, 1) * f;
                self.buf[i * 2] += sl * v.gl * master * cat_gain;
                self.buf[i * 2 + 1] += sr * v.gr * master * cat_gain;
                v.pos += step;
            }
        }
        // drop finished one-shots and faded-out loops
        m.voices.retain(|v| {
            let n = v.clip.frames();
            let ended = !v.looping && v.pos >= n.saturating_sub(1) as f64;
            let silent = !v.held && v.id != 0 && v.tl == 0.0 && v.tr == 0.0 && v.gl.abs() < 1e-4 && v.gr.abs() < 1e-4;
            !(ended || silent)
        });
        // soft clip
        for s in self.buf.iter_mut() {
            *s = (*s).tanh();
        }
    }
}

impl Iterator for MixSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.at >= self.buf.len() {
            self.refill();
        }
        let s = self.buf[self.at];
        self.at += 1;
        Some(s)
    }
}

impl Source for MixSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Seconds between the output device checks (`Audio::watch_output`).
const WATCH_SECS: f32 = 3.0;

/// An opened output: the stream, its handle, the device's name and the default device's name at the time.
struct Output {
    stream: OutputStream,
    handle: OutputStreamHandle,
    name: String,
    default: Option<String>,
}

/// Open the default output device, else any other output device that opens (#13: a disabled or vanished default,
/// a format the default will not take); on each device its default format first, then the others it supports
/// (`open_device`). rodio converts the mix's 44.1 kHz stereo to the format. `verbose`: log every device tried.
fn open_output(verbose: bool) -> Option<Output> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    // (test hook MCLA_AUDIO_FAIL_OPENS=<n>: the first n opens fail, as with no usable device)
    static FAILS: AtomicU64 = AtomicU64::new(0);
    if let Some(n) = std::env::var("MCLA_AUDIO_FAIL_OPENS").ok().and_then(|v| v.parse::<u64>().ok()) {
        if FAILS.fetch_add(1, Ordering::Relaxed) < n {
            log::warn!("audio: output open failed (MCLA_AUDIO_FAIL_OPENS)");
            return None;
        }
    }
    let host = rodio::cpal::default_host();
    let default = host.default_output_device();
    let dname = default.as_ref().and_then(|d| d.name().ok());
    match &default {
        Some(d) => {
            if verbose {
                let cfg = match d.default_output_config() {
                    Ok(c) => format!("{} Hz, {} channels, {:?}", c.sample_rate().0, c.channels(), c.sample_format()),
                    Err(e) => format!("no default format: {e}"),
                };
                log::info!("audio: {} host, default output device {:?} ({cfg})", host.id().name(), dname.as_deref().unwrap_or("?"));
            }
            match open_device(d, verbose) {
                Ok((stream, handle)) => return Some(Output { stream, handle, name: dname.clone().unwrap_or_default(), default: dname }),
                Err(e) => {
                    if verbose {
                        log::warn!("audio: the default output device would not open: {e}");
                    }
                }
            }
        }
        None => {
            if verbose {
                log::warn!("audio: {} host has no default output device", host.id().name());
            }
        }
    }
    if let Ok(devs) = host.output_devices() {
        for d in devs {
            let n = d.name().unwrap_or_default();
            if Some(&n) == dname.as_ref() {
                continue;
            }
            match open_device(&d, verbose) {
                Ok((stream, handle)) => {
                    log::info!("audio: opened output device {n:?} instead of the default");
                    return Some(Output { stream, handle, name: n, default: dname });
                }
                Err(e) => {
                    if verbose {
                        log::warn!("audio: output device {n:?} would not open: {e}");
                    }
                }
            }
        }
    }
    None
}

/// One device: its default format (rodio's `try_from_device`), else each supported format range at 48 kHz, 44.1 kHz
/// or its highest rate, float and 16-bit ranges first.
fn open_device(d: &rodio::cpal::Device, verbose: bool) -> Result<(OutputStream, OutputStreamHandle), rodio::StreamError> {
    use rodio::cpal::traits::DeviceTrait;
    use rodio::cpal::{SampleFormat, SampleRate};
    let first = match OutputStream::try_from_device(d) {
        Ok(o) => return Ok(o),
        Err(e) => e,
    };
    let Ok(ranges) = d.supported_output_configs() else { return Err(first) };
    let mut ranges: Vec<_> = ranges.collect();
    ranges.sort_by_key(|r| match r.sample_format() {
        SampleFormat::F32 => 0,
        SampleFormat::I16 => 1,
        _ => 2,
    });
    for r in ranges {
        for rate in [48_000, 44_100] {
            if (r.min_sample_rate().0..=r.max_sample_rate().0).contains(&rate) {
                let c = r.clone().with_sample_rate(SampleRate(rate));
                if let Ok(o) = OutputStream::try_from_device_config(d, c.clone()) {
                    if verbose {
                        log::info!("audio: opened at {} Hz, {} channels, {:?}", rate, c.channels(), c.sample_format());
                    }
                    return Ok(o);
                }
            }
        }
        let c = r.with_max_sample_rate();
        if let Ok(o) = OutputStream::try_from_device_config(d, c.clone()) {
            if verbose {
                log::info!("audio: opened at {} Hz, {} channels, {:?}", c.sample_rate().0, c.channels(), c.sample_format());
            }
            return Ok(o);
        }
    }
    Err(first)
}

/// The rpm loops of one car's engine bank.
struct EngineBank {
    /// (rpm, clip), ascending
    loops: Vec<(f32, Arc<Clip>)>,
}

pub struct Audio {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    _sink: Option<Sink>,
    /// offline rendering (tests): the mix is pulled by the game loop and kept here
    offline: Option<(MixSource, Vec<f32>)>,
    /// the output: the device's name, the default device's name when it was opened, the samples pulled, when it was
    /// last checked, and the mix drained in real time while no device is open (so voices still run out)
    device: String,
    default_seen: Option<String>,
    pulled: Arc<AtomicU64>,
    watch: Instant,
    drain: Option<(MixSource, Instant)>,
    voice_vol: f32,
    mixer: Arc<Mutex<Mixer>>,
    root: PathBuf,
    /// the original's audio metadata (`audio/X360/config` of the extracted data: sounds.dat, curves.dat ...)
    pub cfg: PathBuf,
    clips: HashMap<String, Option<Arc<Clip>>>,
    engines: HashMap<String, Option<Arc<EngineBank>>>,
    /// wave names per bank folder (from `_index.tsv`)
    index: HashMap<String, Arc<Vec<String>>>,
    music: Option<Sink>,
    /// one-shot dialogue stream (story scenes)
    voice: Option<Sink>,
    /// what `voice` plays (diagnostics)
    stream_name: Option<String>,
    music_files: Vec<PathBuf>,
    music_next: usize,
    pub music_on: bool,
    pub music_volume: f32,
    /// the playing song's own level (its headroom)
    music_gain: f32,
    pub now_playing: String,
    /// per music file: the song's title and genre (0 eclectic .. 6 west coast rap) from the
    /// game's string table; None for what is not one of its songs
    music_meta: Vec<Option<(String, u8)>>,
    /// the playlist: one genre, or all of them
    pub playlist: Option<u8>,
    /// the original's music lists (`mc4`): with them the next song is theirs
    pub lists: Option<crate::mc4::MusicShared>,
    /// set when a new track starts (the HUD shows the title once)
    pub track_changed: bool,
    /// the song that started, as the music manager's callback hands it on (F-17609): the playing
    /// list (+52) and the place in it (+48); None for a track the lists did not choose
    pub song_place: Option<(usize, usize)>,
    /// looping stereo bed (city ambience): (path, sink)
    bed: Option<(PathBuf, Sink)>,
    /// listener
    ear: Vec3,
    ear_right: Vec3,
    rng: u32,
    pub available: bool,
    /// requests for the original's sound objects, played by `sound::Sound` (which owns the sound library) this frame;
    /// `objects` = the library is loaded (requests are accepted)
    pub obj_cmds: Vec<ObjCmd>,
    pub objects: bool,
}

/// A request for the sound-object player (`sndobj.rs`).
#[derive(Clone, Debug)]
pub enum ObjCmd {
    /// a sounds.dat object by name: at a place (positional) or not, with a requested volume (dB)
    Play { name: String, pos: Option<Vec3>, volume: f32 },
    /// a converted wave (`<bank>/<wave>` below the audio root) played as a voice of the category (speech lines:
    /// the speech system picks the take, the category and the scenario's mixer set the level)
    Clip { key: String, category: &'static str, pos: Option<Vec3> },
    /// a career speech line's take (`speech.rs`): played through `object` (the voice's Speech sound, or a category name)
    /// on the audio timer 0 (it holds while the game is paused); `player`: at the player's car, followed while it plays
    Line { key: String, object: String, player: bool },
    /// a dynamic mixer scene on / off
    Scene { name: String, on: bool },
    /// the sound (or, for a type-23 object, the scene) whose hash sits at `offset` of a game.dat object
    Field { object: String, offset: usize, pos: Option<Vec3>, on: bool },
    /// a persistent sound by name, kept while it is asked for every frame (stopped in the first frame it is not)
    Loop { name: String },
    /// a story scene's audio (game.dat `CUTSCENES_<name>`: its dialogue and effects streams and its mixer scene)
    /// started (`sounds` false: its mixer scene only), or (`on` false) the scene's audio ended
    Cutscene { name: String, on: bool, sounds: bool },
    /// the story scene's streams moved to `secs` from their start (the skip to the scene's bookmark)
    CutsceneSeek { secs: f32 },
}

/// Gains for a sound at `pos`: inverse-distance roll-off with a reference distance,
/// constant-power pan from the listener's right vector.
fn spatial(ear: Vec3, right: Vec3, pos: Vec3, vol: f32, ref_dist: f32) -> (f32, f32) {
    let d = pos - ear;
    let dist = d.length();
    let att = (ref_dist / dist.max(ref_dist)).powf(1.3);
    let pan = if dist > 0.5 { (d / dist).dot(right).clamp(-1.0, 1.0) * 0.8 } else { 0.0 };
    let a = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    (vol * att * a.cos() * 1.414, vol * att * a.sin() * 1.414)
}

impl Audio {
    pub fn new(data_dir: &str) -> Option<Audio> {
        Self::create(data_dir, false)
    }

    /// No audio device: the game loop pulls the mix with [`Audio::render`] and
    /// [`Audio::write_capture`] saves it.
    pub fn new_offline(data_dir: &str) -> Option<Audio> {
        Self::create(data_dir, true)
    }

    fn create(data_dir: &str, offline: bool) -> Option<Audio> {
        let mixer = Arc::new(Mutex::new(Mixer { voices: Vec::new(), master: 0.9, slow: None }));
        let pulled = Arc::new(AtomicU64::new(0));
        let src = MixSource { mixer: mixer.clone(), buf: Vec::new(), at: 0, pulled: pulled.clone() };
        let mut device = String::new();
        let mut default_seen = None;
        let mut drain = None;
        let (stream, handle, sink, off) = if offline {
            (None, None, None, Some((src, Vec::new())))
        } else {
            // (no device is no reason to drop the game's sound for the session: the mix is drained and the output
            // tried again every few seconds - `watch_output`)
            match open_output(true) {
                Some(o) => {
                    let sink = Sink::try_new(&o.handle).ok();
                    if let Some(s) = &sink {
                        s.append(src);
                        s.play();
                    }
                    device = o.name;
                    default_seen = o.default;
                    (Some(o.stream), Some(o.handle), sink, None)
                }
                None => {
                    log::warn!("audio: no output device could be opened - the game runs silent and tries again every {} s", WATCH_SECS);
                    drain = Some((src, Instant::now()));
                    (None, None, None, None)
                }
            }
        };
        // <data>/../audio_pc  (data = .../extracted/cache)
        let root = Path::new(data_dir).parent().map(|p| p.join("audio_pc")).unwrap_or_else(|| PathBuf::from("audio_pc"));
        let available = root.is_dir();
        if !available {
            log::warn!("audio: {} not found - run tools/convert_audio.py to convert the original sound banks", root.display());
        }
        // the radio: full songs only (short files are stingers and menu loops)
        let mut music_files: Vec<PathBuf> = std::fs::read_dir(root.join("music"))
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "mp3") && p.metadata().map_or(false, |m| m.len() > 1_500_000)).collect())
            .unwrap_or_default();
        music_files.sort();
        log::info!("audio: bank root {} ({}), {} music tracks", root.display(), if available { "ok" } else { "missing" }, music_files.len());
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7) | 1;
        Some(Audio {
            _stream: stream,
            handle,
            _sink: sink,
            offline: off,
            device,
            default_seen,
            pulled,
            watch: Instant::now(),
            drain,
            voice_vol: 1.0,
            mixer,
            root,
            cfg: Path::new(data_dir).join("audio").join("X360").join("config"),
            clips: HashMap::new(),
            engines: HashMap::new(),
            index: HashMap::new(),
            music: None,
            music_next: (seed as usize) % music_files.len().max(1),
            music_files,
            music_on: true,
            music_volume: 0.35,
            music_gain: 1.0,
            now_playing: String::new(),
            music_meta: Vec::new(),
            playlist: None,
            lists: None,
            track_changed: false,
            song_place: None,
            bed: None,
            voice: None,
            stream_name: None,
            ear: Vec3::ZERO,
            ear_right: Vec3::X,
            rng: seed,
            available,
            obj_cmds: Vec::new(),
            objects: false,
        })
    }

    /// Offline mode: advance the mix by `dt` seconds.
    pub fn render(&mut self, dt: f32) {
        if let Some((src, out)) = self.offline.as_mut() {
            let n = (dt * RATE as f32).round() as usize * 2;
            for _ in 0..n {
                out.push(src.next().unwrap_or(0.0));
            }
        }
    }

    /// Offline mode: write everything rendered so far as a 16-bit stereo WAV.
    pub fn write_capture(&self, path: &str) {
        let Some((_, out)) = self.offline.as_ref() else { return };
        let mut d = Vec::with_capacity(44 + out.len() * 2);
        let n = (out.len() * 2) as u32;
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(36 + n).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&RATE.to_le_bytes());
        d.extend_from_slice(&(RATE * 4).to_le_bytes());
        d.extend_from_slice(&4u16.to_le_bytes());
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(b"data");
        d.extend_from_slice(&n.to_le_bytes());
        for s in out {
            d.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        let _ = std::fs::write(path, d);
        log::info!("audio: wrote {} ({:.1} s)", path, out.len() as f32 / (RATE as f32 * 2.0));
    }

    /// Play a sounds.dat object by name (non-positional, 0 dB requested: the frontend path sub_821E6800). False when the
    /// sound objects are not loaded, so the caller can fall back.
    pub fn has_objects(&self) -> bool {
        self.objects
    }
    pub fn obj(&mut self, name: &str) -> bool {
        self.obj_ex(name, None, 0.0)
    }
    /// A persistent sound by name, asked for every frame it should run (non-positional).
    pub fn obj_loop(&mut self, name: &str) -> bool {
        if self.objects {
            self.obj_cmds.push(ObjCmd::Loop { name: name.to_string() });
        }
        self.objects
    }
    pub fn obj_at(&mut self, name: &str, pos: Vec3) -> bool {
        self.obj_ex(name, Some(pos), 0.0)
    }
    pub fn obj_ex(&mut self, name: &str, pos: Option<Vec3>, volume: f32) -> bool {
        if self.objects {
            self.obj_cmds.push(ObjCmd::Play { name: name.to_string(), pos, volume });
        }
        self.objects
    }
    /// A speech take as a voice of `category` (see [`ObjCmd::Clip`]).
    pub fn obj_clip(&mut self, key: &str, category: &'static str, pos: Option<Vec3>) -> bool {
        if self.objects {
            self.obj_cmds.push(ObjCmd::Clip { key: key.to_string(), category, pos });
        }
        self.objects
    }
    /// A career speech line's take (see [`ObjCmd::Line`]).
    pub fn obj_line(&mut self, key: &str, object: &str, player: bool) -> bool {
        if self.objects {
            self.obj_cmds.push(ObjCmd::Line { key: key.to_string(), object: object.to_string(), player });
        }
        self.objects
    }
    /// The sound named by a game.dat object's field (`on` false: a scene field switched off).
    pub fn obj_field(&mut self, object: &str, offset: usize, pos: Option<Vec3>, on: bool) -> bool {
        if self.objects {
            self.obj_cmds.push(ObjCmd::Field { object: object.to_string(), offset, pos, on });
        }
        self.objects
    }
    /// A story scene's audio by the scene's name ("CUT_STORY_RACE_GH_06"); false when the sound objects are not loaded.
    pub fn obj_cutscene(&mut self, name: &str, on: bool, sounds: bool) -> bool {
        if self.objects {
            self.obj_cmds.push(ObjCmd::Cutscene { name: name.to_string(), on, sounds });
        }
        self.objects
    }
    pub fn obj_cutscene_seek(&mut self, secs: f32) {
        if self.objects {
            self.obj_cmds.push(ObjCmd::CutsceneSeek { secs });
        }
    }
    pub fn obj_scene(&mut self, name: &str, on: bool) {
        if self.objects {
            self.obj_cmds.push(ObjCmd::Scene { name: name.to_string(), on });
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn random(&mut self) -> u32 {
        self.rand()
    }

    fn rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    pub fn set_listener(&mut self, pos: Vec3, right: Vec3) {
        self.ear = pos;
        self.ear_right = right.normalize_or(Vec3::X);
    }

    /// (step 23b) AudioManager_SetActiveTimeSlow(on) (sub_821E7470 -> sub_821E68D0): the dynamic
    /// mixer's state 16 switched on - its enter sound, the categories ramped in over 1 s - or
    /// off (no exit sound: +18 is NULL_SOUND), ramped back over 1 s. Only a change acts.
    pub fn set_time_slow(&mut self, on: bool) {
        let Ok(mut m) = self.mixer.lock() else { return };
        let was = m.slow.map_or(false, |r| r.to == SLOWDOWN_ON);
        if was == on {
            return;
        }
        let from = m.slow.map(|r| r.now()).unwrap_or(SLOWDOWN_OFF);
        let (to, dur) = if on { (SLOWDOWN_ON, SLOWDOWN_FADE.0) } else { (SLOWDOWN_OFF, SLOWDOWN_FADE.1) };
        m.slow = Some(SlowRamp { from, to, start: std::time::Instant::now(), dur });
        drop(m);
        if on {
            self.play("special_effects/bnk_special_effects/stinger_01_enter_l", 0.8);
        }
        log::info!("audio: time slow {}", if on { "on (DYNAMIC_MIXER_SLOWDOWN)" } else { "off" });
    }

    /// The ambience's share under the slow-down state (AMBIENCE's category gain).
    pub fn ambience_gain(&self) -> f32 {
        self.mixer.lock().ok().and_then(|m| m.slow.map(|r| r.now()[2])).unwrap_or(1.0)
    }

    pub fn set_master(&self, v: f32) {
        if let Ok(mut m) = self.mixer.lock() {
            m.master = v;
        }
    }

    /// Number of voices currently mixing (debug).
    pub fn voices(&self) -> usize {
        self.mixer.lock().map(|m| m.voices.len()).unwrap_or(0)
    }

    /// Wave names of a bank folder.
    pub fn bank_index(&mut self, bank: &str) -> Arc<Vec<String>> {
        if let Some(i) = self.index.get(bank) {
            return i.clone();
        }
        let names: Vec<String> = std::fs::read_to_string(self.root.join(bank).join("_index.tsv")).map(|t| t.lines().filter_map(|l| l.split(TAB).next().map(|s| s.to_string())).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        let a = Arc::new(names);
        self.index.insert(bank.to_string(), a.clone());
        a
    }

    /// `bank/wave` (lower case, as written by the converter).
    pub fn clip(&mut self, key: &str) -> Option<Arc<Clip>> {
        if let Some(c) = self.clips.get(key) {
            return c.clone();
        }
        let c = load_wav(&self.root.join(format!("{key}.wav"))).map(Arc::new);
        if c.is_none() && self.available {
            log::warn!("audio: missing clip {}", key);
        }
        self.clips.insert(key.to_string(), c.clone());
        c
    }

    /// A random wave of `bank` whose name starts with `prefix`.
    pub fn pick(&mut self, bank: &str, prefix: &str) -> Option<String> {
        let idx = self.bank_index(bank);
        let m: Vec<&String> = idx.iter().filter(|n| n.starts_with(prefix)).collect();
        if m.is_empty() {
            return None;
        }
        let k = self.rand() as usize % m.len();
        Some(format!("{}/{}", bank, m[k]))
    }

    /// A voice that is not one of the 96 game voices (the offline stream).
    fn push_any(&mut self, v: Voice) {
        if let Ok(mut m) = self.mixer.lock() {
            m.voices.push(v);
        }
    }

    /// The stream [`Audio::play_stream`] last started, if it still plays (diagnostics).
    pub fn stream_playing(&self) -> Option<&str> {
        let on = match (&self.voice, self.offline.is_some()) {
            (Some(s), _) => !s.empty(),
            (None, true) => self.mixer.lock().is_ok_and(|m| m.voices.iter().any(|v| v.id == STREAM_VOICE)),
            _ => false,
        };
        self.stream_name.as_deref().filter(|_| on)
    }

    /// The radio's song, if it plays (diagnostics).
    pub fn music_playing(&self) -> bool {
        self.music.as_ref().is_some_and(|s| !s.is_paused() && !s.empty())
    }

    fn push(&mut self, v: Voice) {
        if let Ok(mut m) = self.mixer.lock() {
            if m.voices.len() < 96 {
                m.voices.push(v);
            }
        }
    }

    /// Non-positional one-shot (UI, the player's own effects).
    pub fn play(&mut self, key: &str, vol: f32) {
        self.play_ex(key, vol, 1.0);
    }

    pub fn play_ex(&mut self, key: &str, vol: f32, speed: f32) {
        if let Some(clip) = self.clip(key) {
            self.push(Voice { id: 0, clip, pos: 0.0, speed, gl: vol, gr: vol, tl: vol, tr: vol, looping: false, fresh: true, held: false });
        }
    }

    /// Positional one-shot.
    pub fn play_at(&mut self, key: &str, pos: Vec3, vol: f32, ref_dist: f32) {
        if let Some(clip) = self.clip(key) {
            let (l, r) = spatial(self.ear, self.ear_right, pos, vol, ref_dist);
            if l + r > 0.004 {
                self.push(Voice { id: 0, clip, pos: 0.0, speed: 1.0, gl: l, gr: r, tl: l, tr: r, looping: false, fresh: true, held: false });
            }
        }
    }

    /// Create or update a looping voice. Loops that are not updated for a frame fade out
    /// (see [`Audio::end_frame`]).
    pub fn set_loop(&mut self, id: u64, key: &str, gains: (f32, f32), speed: f32) {
        let Some(clip) = self.clip(key) else { return };
        self.set_loop_clip(id, clip, gains, speed);
    }

    fn set_loop_clip(&mut self, id: u64, clip: Arc<Clip>, gains: (f32, f32), speed: f32) {
        let Ok(mut m) = self.mixer.lock() else { return };
        if let Some(v) = m.voices.iter_mut().find(|v| v.id == id) {
            v.tl = gains.0;
            v.tr = gains.1;
            v.speed = speed;
            v.fresh = true;
            return;
        }
        if gains.0 + gains.1 < 0.002 || m.voices.len() >= 96 {
            return;
        }
        // start at a random phase so identical loops do not comb
        let start = (id.wrapping_mul(2654435761) % clip.frames().max(1) as u64) as f64;
        m.voices.push(Voice { id, clip, pos: start, speed, gl: 0.0, gr: 0.0, tl: gains.0, tr: gains.1, looping: true, fresh: true, held: false });
    }

    /// Create or update a voice that lives while it is driven every frame (the sound objects' waves): `start` is the
    /// frame a new voice starts at; a one-shot voice ends at its last frame.
    pub fn drive(&mut self, id: u64, clip: &Arc<Clip>, gains: (f32, f32), speed: f32, looping: bool, start: f64) {
        let Ok(mut m) = self.mixer.lock() else { return };
        if let Some(v) = m.voices.iter_mut().find(|v| v.id == id) {
            v.tl = gains.0;
            v.tr = gains.1;
            v.speed = speed;
            v.fresh = true;
            v.held = false;
            return;
        }
        // a one-shot the mixer has just finished is not started again from its last frames
        if m.voices.len() >= 96 || (!looping && start > 0.0 && start + clip.rate as f64 * 0.05 >= clip.frames() as f64) {
            return;
        }
        m.voices.push(Voice { id, clip: clip.clone(), pos: start, speed, gl: gains.0, gr: gains.1, tl: gains.0, tr: gains.1, looping, fresh: true, held: false });
    }

    /// Hold a driven voice where it is, silent, for this frame (its sound is on a paused audio timer: F-89113 skips the
    /// sound and the type's pause method stops its voice); the next `drive` goes on from there.
    pub fn hold(&mut self, id: u64) {
        let Ok(mut m) = self.mixer.lock() else { return };
        if let Some(v) = m.voices.iter_mut().find(|v| v.id == id) {
            v.held = true;
            v.fresh = true;
        }
    }

    pub fn gains_at(&self, pos: Vec3, vol: f32, ref_dist: f32) -> (f32, f32) {
        spatial(self.ear, self.ear_right, pos, vol, ref_dist)
    }

    /// Fade out every loop the game did not touch this frame.
    /// The output device kept working (#13): every few seconds, reopen it when none is open, when the stream has
    /// stopped pulling the mix (a device unplugged, its format changed, taken exclusively) or when Windows' default
    /// output device changed; while none is open the mix is drained in real time. Cheap between the checks.
    pub fn watch_output(&mut self) {
        if self.offline.is_some() {
            return;
        }
        if let Some((src, at)) = &mut self.drain {
            let n = (at.elapsed().as_secs_f32() * RATE as f32) as usize * 2;
            *at = Instant::now();
            for _ in 0..n.min(RATE as usize * 2) {
                src.next();
            }
        }
        let el = self.watch.elapsed().as_secs_f32();
        if el < WATCH_SECS {
            return;
        }
        self.watch = Instant::now();
        let pulled = self.pulled.swap(0, Ordering::Relaxed) as f32;
        let reason = if self.handle.is_none() {
            Some("no device open".to_string())
        } else if pulled < el * RATE as f32 * 2.0 * 0.25 {
            Some(format!("the stream on {:?} stopped ({:.0} of {:.0} samples pulled)", self.device, pulled, el * RATE as f32 * 2.0))
        } else {
            use rodio::cpal::traits::{DeviceTrait, HostTrait};
            let now = rodio::cpal::default_host().default_output_device().and_then(|d| d.name().ok());
            (now.is_some() && now != self.default_seen).then(|| format!("the default output device changed to {:?}", now.unwrap_or_default()))
        };
        if let Some(r) = reason {
            self.reopen_output(&r);
        }
    }

    fn reopen_output(&mut self, why: &str) {
        let first = self.handle.is_some();
        let Some(o) = open_output(first) else {
            if first {
                log::warn!("audio: {why}; no output device could be opened - silent, trying again every {} s", WATCH_SECS);
                self.drop_output();
                self.drain = Some((self.mix_source(), Instant::now()));
            }
            return;
        };
        // what the streams were playing, to carry on where they were
        let music = self.music.as_ref().map(|s| (s.get_pos(), s.is_paused()));
        let voice = self.voice.as_ref().map(|s| s.get_pos()).zip(self.stream_name.clone());
        self.drop_output();
        let Ok(sink) = Sink::try_new(&o.handle) else { return };
        sink.append(self.mix_source());
        sink.play();
        log::info!("audio: output on {:?} ({why})", o.name);
        self.device = o.name;
        self.default_seen = o.default;
        self._sink = Some(sink);
        self.handle = Some(o.handle);
        self._stream = Some(o.stream);
        self.drain = None;
        self.pulled.store(0, Ordering::Relaxed);
        self.watch = Instant::now();
        if let Some((pos, paused)) = music {
            self.start_track();
            self.track_changed = false;
            if let Some(s) = self.music.as_ref() {
                let _ = s.try_seek(pos);
                if paused {
                    s.pause();
                }
            }
        }
        if let Some((pos, name)) = voice {
            let v = self.voice_vol;
            self.play_stream_at(Some(&name), v, pos.as_secs_f32());
        }
        // (the ambience bed is asked for every frame and starts again on its own)
    }

    fn drop_output(&mut self) {
        for s in [self.music.take(), self.voice.take(), self._sink.take()].into_iter().flatten() {
            s.stop();
        }
        if let Some((_, s)) = self.bed.take() {
            s.stop();
        }
        self.handle = None;
        self._stream = None;
    }

    fn mix_source(&self) -> MixSource {
        MixSource { mixer: self.mixer.clone(), buf: Vec::new(), at: 0, pulled: self.pulled.clone() }
    }

    pub fn end_frame(&mut self) {
        self.watch_output();
        if let Ok(mut m) = self.mixer.lock() {
            for v in m.voices.iter_mut().filter(|v| v.id != 0 && v.id != STREAM_VOICE) {
                if !v.fresh {
                    v.held = false;
                    v.tl = 0.0;
                    v.tr = 0.0;
                }
                v.fresh = false;
            }
        }
    }

    fn engine_bank(&mut self, car: &str) -> Option<Arc<EngineBank>> {
        if let Some(b) = self.engines.get(car) {
            return b.clone();
        }
        let lc = car.to_ascii_lowercase();
        // folders are named after the car; a few only survive as the hash of that name
        // cars whose own bank is not on the disc borrow the closest relative
        let relative = match lc.as_str() {
            "vp_frd_boss302_69" => "vp_frd_mustang_06",
            "vpd_am_db9_08" => "vp_am_vantage_07",
            "vpd_aud_s5_08" => "vp_aud_rs4_07",
            "vpd_cad_xlrv_08" => "vp_chv_corvettez06_07",
            "vpd_chv_chevelle_70" => "vp_chv_camaross_69",
            "vpd_duc_1098_07" => "vp_duc_999r_06",
            "vpd_lr_hse_08" | "vpd_lr_sport_08" => "vp_cry_300c_06",
            "vpd_mbz_cl65amg_08" => "vp_mbz_cl65_06",
            "vpd_mbz_clk63amg_08" => "vp_mbz_sl65_07",
            "vpd_mit_eclipse_09" => "vp_mit_eclipse_99",
            "vpd_mit_evox_08" => "vp_mit_evo_06",
            _ => "",
        };
        let as_vp = lc.replacen("vpd_", "vp_", 1);
        let cands = [
            format!("player_vehicles/0x{:08x}", mcla_formats::hash::joaat(&as_vp)),
            format!("player_vehicles/{relative}"),
            format!("player_vehicles/0x{:08x}", mcla_formats::hash::joaat(relative)),
            format!("player_vehicles/{lc}"),
            format!("player_vehicles/0x{:08x}", mcla_formats::hash::joaat(&lc)),
            format!("player_vehicles/{}", lc.replace("_dub", "")),
            // opponent_vehicles/<car>_o
            format!("0xb31871af/0x{:08x}", mcla_formats::hash::joaat(&format!("{lc}_o"))),
        ];
        let mut bank = None;
        for folder in cands {
            let idx = self.bank_index(&folder);
            let mut loops: Vec<(f32, Arc<Clip>)> = Vec::new();
            for name in idx.iter() {
                // "..._<rpm>" or "..._<rpm>_fin"
                let base = name.trim_end_matches("_fin");
                let Some(num) = base.rsplit('_').next().and_then(|t| t.parse::<f32>().ok()) else { continue };
                if !(500.0..=14000.0).contains(&num) || num % 250.0 != 0.0 {
                    continue;
                }
                if let Some(c) = self.clip(&format!("{folder}/{name}")) {
                    loops.push((num, c));
                }
            }
            if loops.len() >= 3 {
                loops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                log::info!("audio: engine bank {} with {} rpm loops ({}..{})", folder, loops.len(), loops[0].0, loops[loops.len() - 1].0);
                bank = Some(Arc::new(EngineBank { loops }));
                break;
            }
        }
        self.engines.insert(car.to_string(), bank.clone());
        bank
    }

    /// Drive one engine: cross-fade between the two recorded loops around `rpm`, each
    /// pitched to the exact engine speed. `slot` distinguishes simultaneous engines.
    pub fn engine(&mut self, slot: u64, car: &str, rpm: f32, throttle: f32, gains: (f32, f32)) -> bool {
        let Some(bank) = self.engine_bank(car) else { return false };
        let n = bank.loops.len();
        let lo = bank.loops[0].0;
        let hi = bank.loops[n - 1].0;
        let r = rpm.clamp(lo * 0.8, hi * 1.12);
        let mut i = 0;
        while i + 2 < n && bank.loops[i + 1].0 <= r {
            i += 1;
        }
        let (r0, r1) = (bank.loops[i].0, bank.loops[i + 1].0);
        let f = ((r - r0) / (r1 - r0).max(1.0)).clamp(0.0, 1.0);
        // off-throttle the engine is quieter
        let load = 0.55 + 0.45 * throttle.clamp(0.0, 1.0);
        for (k, (lr, clip)) in bank.loops.iter().enumerate() {
            let w = if k == i { (1.0 - f).sqrt() } else if k == i + 1 { f.sqrt() } else { 0.0 };
            let id = 0x1000_0000 + slot * 64 + k as u64;
            if w > 0.0 {
                self.set_loop_clip(id, clip.clone(), (gains.0 * w * load, gains.1 * w * load), r / lr);
            }
        }
        true
    }

    /// Looping stereo background stream (city ambience). `None` stops it.
    pub fn set_bed(&mut self, rel: Option<&str>, volume: f32) {
        let Some(handle) = self.handle.as_ref() else { return };
        let want = rel.map(|r| self.root.join(r));
        // (step 23b: AMBIENCE under the slow-down state)
        let volume = volume * self.ambience_gain();
        if let (Some((cur, sink)), Some(w)) = (self.bed.as_ref(), want.as_ref()) {
            if cur == w {
                sink.set_volume(volume);
                return;
            }
        }
        if let Some((_, s)) = self.bed.take() {
            s.stop();
        }
        let Some(p) = want else { return };
        let Ok(f) = std::fs::File::open(&p) else { return };
        let Ok(dec) = rodio::Decoder::new(std::io::BufReader::new(f)) else { return };
        if let Ok(s) = Sink::try_new(handle) {
            s.set_volume(volume);
            s.append(dec.repeat_infinite());
            s.play();
            self.bed = Some((p, s));
        }
    }

    /// Play a stereo stream once (a film's sound, story scene dialogue without the sound objects); `None` stops it.
    pub fn play_stream(&mut self, rel: Option<&str>, volume: f32) {
        self.play_stream_at(rel, volume, 0.0);
    }

    /// The same from `at` seconds into the stream.
    pub fn play_stream_at(&mut self, rel: Option<&str>, volume: f32, at: f32) {
        if let Some(s) = self.voice.take() {
            s.stop();
        }
        self.stream_name = rel.map(|r| r.to_string());
        self.voice_vol = volume;
        // (offline: a mixer voice, so the capture holds it)
        if self.offline.is_some() {
            if let Ok(mut m) = self.mixer.lock() {
                m.voices.retain(|v| v.id != STREAM_VOICE);
            }
            if let Some(c) = rel.and_then(|r| crate::sndobj::decode_mp3(&self.root.join(r))) {
                let pos = (at.max(0.0) * c.rate as f32) as f64;
                self.push_any(Voice { id: STREAM_VOICE, clip: Arc::new(c), pos, speed: 1.0, gl: volume, gr: volume, tl: volume, tr: volume, looping: false, fresh: true, held: false });
            }
            return;
        }
        let (Some(handle), Some(rel)) = (self.handle.as_ref(), rel) else { return };
        let Ok(f) = std::fs::File::open(self.root.join(rel)) else { return };
        let Ok(dec) = rodio::Decoder::new(std::io::BufReader::new(f)) else { return };
        if let Ok(s) = Sink::try_new(handle) {
            s.set_volume(volume);
            s.append(dec.skip_duration(Duration::from_secs_f32(at.max(0.0))));
            s.play();
            self.voice = Some(s);
        }
    }

    /// Keep the radio playing: starts the next track when the current one ends.
    pub fn update_music(&mut self, playing: bool) {
        self.watch_output();
        if !self.music_on || !playing || self.music_files.is_empty() {
            if let Some(s) = self.music.as_ref() {
                s.pause();
            }
            return;
        }
        if let Some(s) = self.music.as_ref() {
            s.set_volume(self.music_volume * self.music_gain);
            if s.is_paused() {
                s.play();
            }
            if !s.empty() {
                return;
            }
        }
        self.next_track();
    }

    /// A converted stream's level: the original subtracts each channel's headroom (wave info +26); the converter
    /// folds the channels relative to the lowest one, which is subtracted here.
    fn stream_gain(p: &Path) -> f32 {
        let txt = std::fs::read_to_string(format!("{}.txt", p.display())).unwrap_or_default();
        let h0 = txt.lines().filter_map(|l| l.split(TAB).nth(5)?.trim().parse::<i32>().ok()).min().unwrap_or(0);
        mcla_formats::audcfg::db2lin(-(h0 as f32) * 0.01)
    }

    /// Title of a track from the converter's channel list ("ARTIST_SONG_LEFT").
    fn track_title(p: &Path) -> String {
        let txt = std::fs::read_to_string(format!("{}.txt", p.display())).unwrap_or_default();
        let name = txt.lines().next().and_then(|l| l.split(TAB).nth(2)).unwrap_or("?").to_string();
        if name == "?" {
            return String::new();
        }
        name.trim_end_matches("_LEFT").trim_end_matches("_RIGHT").trim_end_matches("_L").trim_end_matches("_R").replace('_', " ")
    }

    /// Name the songs: the string table's `Music_<Genre>_<Artist>_<Song>` keys against the
    /// asset names of the converted tracks ("NAS_HERO").
    pub fn set_music_catalog(&mut self, strings: &mcla_formats::strtbl::StringTable) {
        const GENRES: [(&str, u8); 9] = [("eclectic_", 0), ("electronic_", 1), ("hardrock_", 2), ("hard_rock_", 2), ("hiphop_", 3), ("rock_", 4), ("techno_", 5), ("westcoastrap_", 6), ("west_rap_", 6)];
        let squash = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
        let mut songs: HashMap<String, (String, u8)> = HashMap::new();
        let mut by_song: HashMap<String, Option<(String, u8)>> = HashMap::new();
        for k in &strings.keys {
            let low = k.to_ascii_lowercase();
            let Some(rest) = low.strip_prefix("music_") else { continue };
            let Some((g, id)) = GENRES.iter().find(|(g, _)| rest.starts_with(g)) else { continue };
            if let Some(title) = strings.get(k) {
                songs.entry(squash(&rest[g.len()..])).or_insert((title.to_string(), *id));
                // (some assets carry another artist's name: the song's own name finds them)
                let song = squash(rest.rsplit('_').next().unwrap_or(""));
                if song.len() >= 4 {
                    by_song.entry(song).and_modify(|e: &mut Option<(String, u8)>| *e = None).or_insert(Some((title.to_string(), *id)));
                }
            }
        }
        self.music_meta = self
            .music_files
            .iter()
            .map(|p| {
                let txt = std::fs::read_to_string(format!("{}.txt", p.display())).unwrap_or_default();
                let asset = txt.lines().next().and_then(|l| l.split('\t').nth(2)).unwrap_or("").to_string();
                let asset = asset.trim_end_matches("_LEFT").trim_end_matches("_RIGHT").trim_end_matches("_L").trim_end_matches("_R");
                if asset.is_empty() || asset == "?" || asset.starts_with("MUSIC_") {
                    return None;
                }
                // (a song the table does not name plays with all genres only, under its asset's name)
                songs
                    .get(&squash(asset))
                    .cloned()
                    .or_else(|| by_song.get(&squash(asset.rsplit('_').next().unwrap_or(""))).cloned().flatten())
                    .or_else(|| Some((asset.replace('_', " "), 255)))
            })
            .collect();
        log::info!("audio: {} of {} music tracks are songs of the game's playlists, {} more play with all genres", self.music_meta.iter().flatten().filter(|m| m.1 != 255).count(), self.music_files.len(), self.music_meta.iter().flatten().filter(|m| m.1 == 255).count());
    }

    /// The songs of the game's playlists: "ARTIST - TITLE" and the genre (0..6; 255 = no
    /// genre of its own), for the pause menu's song lists.
    pub fn songs(&self) -> Vec<(String, u8)> {
        self.music_meta.iter().flatten().cloned().collect()
    }

    /// The converted tracks' asset names ("NIN_DISCIPLINE"; empty for none), file by file.
    pub fn music_assets(&self) -> Vec<String> {
        self.music_files
            .iter()
            .map(|p| {
                let txt = std::fs::read_to_string(format!("{}.txt", p.display())).unwrap_or_default();
                let asset = txt.lines().next().and_then(|l| l.split('\t').nth(2)).unwrap_or("").to_string();
                asset.trim_end_matches("_LEFT").trim_end_matches("_RIGHT").trim_end_matches("_L").trim_end_matches("_R").to_string()
            })
            .collect()
    }

    /// The song before, by the music lists' order (the pad's MUSIC_DOWN).
    pub fn prev_track(&mut self) {
        if let Some(f) = self.lists.as_ref().and_then(|l| l.lock().ok().and_then(|mut l| l.prev_file())).filter(|f| *f < self.music_files.len()) {
            self.music_next = f;
            self.start_track();
        }
    }

    /// The song the music lists say is on starts (PLAY in the playlist menu).
    pub fn play_listed(&mut self) {
        if let Some(f) = self.lists.as_ref().and_then(|l| l.lock().ok().and_then(|l| l.now_file())).filter(|f| *f < self.music_files.len()) {
            self.music_next = f;
            self.start_track();
        }
    }

    pub fn next_track(&mut self) {
        if self.music_files.is_empty() || self.handle.is_none() {
            return;
        }
        // (the original's lists: the next place of the playing list's order)
        if let Some(f) = self.lists.as_ref().and_then(|l| l.lock().ok().and_then(|mut l| l.next_file())).filter(|f| *f < self.music_files.len()) {
            self.music_next = f;
            self.start_track();
            return;
        }
        let step = 1 + self.rand() as usize % 7;
        self.music_next = (self.music_next + step) % self.music_files.len();
        // (the radio plays the songs of the chosen playlist; tracks the string table does not
        // name are menu and garage music)
        if self.music_meta.iter().any(|m| m.is_some()) {
            let ok = |m: &Option<(String, u8)>, pl: Option<u8>| m.as_ref().map_or(false, |(_, g)| pl.map_or(true, |p| p == *g));
            let pl = if self.music_meta.iter().any(|m| ok(m, self.playlist)) { self.playlist } else { None };
            for _ in 0..self.music_files.len() {
                if ok(&self.music_meta[self.music_next], pl) {
                    break;
                }
                self.music_next = (self.music_next + 1) % self.music_files.len();
            }
        }
        self.start_track();
    }

    /// The track `music_next` starts.
    fn start_track(&mut self) {
        if self.music_files.is_empty() || self.handle.is_none() {
            return;
        }
        let p = self.music_files[self.music_next].clone();
        let Ok(f) = std::fs::File::open(&p) else { return };
        let Ok(dec) = rodio::Decoder::new(std::io::BufReader::new(f)) else {
            log::warn!("audio: cannot decode {}", p.display());
            return;
        };
        if let Some(old) = self.music.take() {
            old.stop();
        }
        let Some(handle) = self.handle.as_ref() else { return };
        if let Ok(s) = Sink::try_new(handle) {
            self.music_gain = Self::stream_gain(&p);
            s.set_volume(self.music_volume * self.music_gain);
            s.append(dec);
            s.play();
            self.now_playing = match self.music_meta.get(self.music_next).cloned().flatten() {
                Some((title, _)) => title,
                None => Self::track_title(&p),
            };
            // (a song of the lists: the title the string table has for it)
            if let Some(t) = self.lists.as_ref().and_then(|l| l.lock().ok().and_then(|l| l.title_of_file(self.music_next))) {
                self.now_playing = t;
            }
            self.track_changed = true;
            self.song_place = self.lists.as_ref().and_then(|l| l.lock().ok().and_then(|l| (l.now_file() == Some(self.music_next)).then_some((l.genre, l.cur))));
            log::info!("audio: now playing {} ({})", self.now_playing, p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            self.music = Some(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #13: with no device at the start the game keeps its audio (silent, the mix drained) and opens the output at
    /// the next check; a stream that stops pulling is reopened. Needs an output device and MCLA_DATA_DIR.
    #[test]
    #[ignore = "requires an audio output device and MCLA_DATA_DIR"]
    fn output_retries_and_reopens() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        std::env::set_var("MCLA_AUDIO_FAIL_OPENS", "1");
        let mut a = Audio::new(&dir).expect("audio without a device");
        assert!(a.handle.is_none() && a.drain.is_some());
        a.play("ui/menus/ui_menu_select_l", 0.5);
        a.watch = Instant::now() - Duration::from_secs(4);
        a.watch_output();
        assert!(a.handle.is_some() && a.drain.is_none(), "opened at the retry");
        // a stalled stream: nothing pulled over the check's window
        std::thread::sleep(Duration::from_millis(300));
        a.drop_output();
        a.handle = Some(open_output(false).unwrap().handle);
        a.pulled.store(0, Ordering::Relaxed);
        a.watch = Instant::now() - Duration::from_secs(4);
        a.watch_output();
        assert!(a._sink.is_some() && a._stream.is_some(), "reopened");
        std::thread::sleep(Duration::from_millis(500));
        assert!(a.pulled.load(Ordering::Relaxed) > 10_000, "the new stream pulls the mix");
    }

    /// The slow-down state's ramp (step 23b, work/career2/audioramp/notes.md): every value
    /// moves at 1 a second - the engines' gain reaches 0.316 at 0.684 s, their pitch 0.5 at
    /// 0.5 s, the ambience 0 at 1 s; switched off, back up at the same speed.
    #[test]
    fn slowdown_ramp_moves_at_a_fixed_speed() {
        let r = SlowRamp { from: SLOWDOWN_OFF, to: SLOWDOWN_ON, start: std::time::Instant::now(), dur: SLOWDOWN_FADE.0 };
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        assert!(near(r.at(0.25), [0.75, 0.75, 0.75]));
        assert!(near(r.at(0.5), [0.5, 0.5, 0.5]));
        assert!(near(r.at(0.683_772), [0.316_227_77, 0.5, 0.316_228]));
        assert!(near(r.at(1.5), SLOWDOWN_ON));
        let back = SlowRamp { from: r.at(0.5), to: SLOWDOWN_OFF, start: std::time::Instant::now(), dur: SLOWDOWN_FADE.1 };
        assert!(near(back.at(0.25), [0.75, 0.75, 0.75]));
        assert!(near(back.at(0.5), SLOWDOWN_OFF));
    }
}
