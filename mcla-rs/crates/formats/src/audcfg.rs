//! The RAGE audio metadata of the game (`audio/X360/config/*.dat`): sound objects (`sounds.dat`), curves
//! (`curves.dat`), categories (`categories.dat`) and game objects (`game.dat`).
//!
//! Decoded from the original executable (static recompilation); the rule of every field and its source address are
//! in `docs/reference/audio/SOUND_FORMAT.md`. Big-endian throughout. Names hash with [`crate::hash::joaat`]
//! (A-Z lower-cased, `\` read as `/`).

use crate::hash::joaat;
use std::collections::HashMap;

fn be_u16(d: &[u8], o: usize) -> u16 {
    d.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0)
}
fn be_i16(d: &[u8], o: usize) -> i16 {
    be_u16(d, o) as i16
}
fn be_u32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}
fn be_i32(d: &[u8], o: usize) -> i32 {
    be_u32(d, o) as i32
}
fn be_f32(d: &[u8], o: usize) -> f32 {
    f32::from_bits(be_u32(d, o))
}

/// The original's 0.30103 is stored as 0.301 (0x82002B20).
const LOG10_2: f32 = 0.301;

/// Linear gain to dB as the original computes it: `20 * log2(x) * 0.301`, -100 dB below 1e-5.
pub fn lin2db(x: f32) -> f32 {
    if x - 1e-5 >= 0.0 {
        // (the original multiplies log2 by 0.301 first, then by 20: vmulfp128 with the table, then fmuls)
        // (log2 through f64 and one rounding: the f32 libm routines differ in the last bit between platforms, e.g. on
        // Apple Silicon; this gives the correctly rounded value everywhere)
        (x as f64).log2() as f32 * LOG10_2 * 20.0
    } else {
        -100.0
    }
}

/// dB to linear gain (F-42985): `10^(dB * 0.05)`, 0 below -100 dB.
pub fn db2lin(db: f32) -> f32 {
    if db - (-100.0) >= 0.0 {
        10f32.powf(db * 0.05)
    } else {
        0.0
    }
}

/// One `.dat` file: `u32 version, u32 size, data[size], u32 strsize, strings, u32 count, u32 namebytes`, then per object
/// `{u8 len, name, u32 offset (into data), u32 size}` (loader F-76779).
pub struct Dat {
    pub version: u32,
    data: Vec<u8>,
    /// (name, offset, size) in file order
    pub objects: Vec<(String, usize, usize)>,
    by_hash: HashMap<u32, usize>,
    /// sounds.dat: the wave-bank names of the string block (`VEHICLES\VEHICLE_AUX`)
    pub banks: Vec<String>,
}

impl Dat {
    pub fn parse(d: &[u8]) -> Option<Dat> {
        let version = be_u32(d, 0);
        let size = be_u32(d, 4) as usize;
        let data = d.get(8..8 + size)?.to_vec();
        let mut p = 8 + size;
        let nstr = be_u32(d, p) as usize;
        let strblock = d.get(p + 4..p + 4 + nstr)?;
        p += 4 + nstr;
        let count = be_u32(d, p) as usize;
        p += 8;
        let mut objects = Vec::with_capacity(count);
        let mut by_hash = HashMap::with_capacity(count);
        for i in 0..count {
            let n = *d.get(p)? as usize;
            let name: String = d.get(p + 1..p + 1 + n)?.iter().map(|&c| c as char).collect();
            p += 1 + n;
            let off = be_u32(d, p) as usize;
            let sz = be_u32(d, p + 4) as usize;
            p += 8;
            by_hash.insert(joaat(&name), i);
            objects.push((name, off, sz));
        }
        let mut banks = Vec::new();
        if strblock.len() >= 4 {
            let cnt = be_u32(strblock, 0) as usize;
            let base = 4 + 4 * cnt;
            if base <= strblock.len() {
                for k in 0..cnt {
                    let o = base + be_u32(strblock, 4 + 4 * k) as usize;
                    if let Some(rest) = strblock.get(o..) {
                        let e = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
                        banks.push(rest[..e].iter().map(|&c| c as char).collect());
                    }
                }
            }
        }
        Some(Dat { version, data, objects, by_hash, banks })
    }

    pub fn load(path: &std::path::Path) -> Option<Dat> {
        Dat::parse(&std::fs::read(path).ok()?)
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// The object's bytes by name hash.
    pub fn get(&self, hash: u32) -> Option<&[u8]> {
        let &i = self.by_hash.get(&hash)?;
        let (_, off, sz) = &self.objects[i];
        self.data.get(*off..*off + *sz)
    }
    pub fn get_name(&self, name: &str) -> Option<&[u8]> {
        self.get(joaat(name))
    }
    pub fn name_of(&self, hash: u32) -> Option<&str> {
        self.by_hash.get(&hash).map(|&i| self.objects[i].0.as_str())
    }
    pub fn contains(&self, hash: u32) -> bool {
        self.by_hash.contains_key(&hash)
    }

    /// Add the objects of `other` whose names this file does not have yet, and its banks not yet listed (a mod's
    /// sound objects joining the game's: docs/reference/mods/manifest.md). Objects refer to each other and to banks
    /// by name hash only, so they move as they are. Returns the names skipped because this file already has them.
    pub fn extend_from(&mut self, other: &Dat) -> Vec<String> {
        let mut skipped = Vec::new();
        for (name, off, sz) in &other.objects {
            let h = joaat(name);
            if self.by_hash.contains_key(&h) {
                skipped.push(name.clone());
                continue;
            }
            let Some(bytes) = other.data.get(*off..*off + *sz) else { continue };
            let at = self.data.len();
            self.data.extend_from_slice(bytes);
            self.by_hash.insert(h, self.objects.len());
            self.objects.push((name.clone(), at, *sz));
        }
        for b in &other.banks {
            if !self.banks.iter().any(|x| joaat(x) == joaat(b)) {
                self.banks.push(b.clone());
            }
        }
        skipped
    }

    /// Write a `.dat` file in the layout [`Dat::parse`] reads (the disc's: `version`, data, the bank string block,
    /// count, name bytes = the sum of the name lengths + 1 per object, the name table). Names are at most 255 bytes.
    pub fn build(version: u32, objects: &[(&str, &[u8])], banks: &[&str]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut table = Vec::new();
        let mut name_bytes = 0u32;
        for (name, body) in objects {
            let n = name.len().min(255);
            table.push(n as u8);
            table.extend_from_slice(&name.as_bytes()[..n]);
            table.extend_from_slice(&(data.len() as u32).to_be_bytes());
            table.extend_from_slice(&(body.len() as u32).to_be_bytes());
            name_bytes += n as u32 + 1;
            data.extend_from_slice(body);
        }
        let mut strs = (banks.len() as u32).to_be_bytes().to_vec();
        let mut text = Vec::new();
        for b in banks {
            strs.extend_from_slice(&(text.len() as u32).to_be_bytes());
            text.extend_from_slice(b.as_bytes());
            text.push(0);
        }
        strs.extend_from_slice(&text);
        let mut out = Vec::new();
        out.extend_from_slice(&version.to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&data);
        out.extend_from_slice(&(strs.len() as u32).to_be_bytes());
        out.extend_from_slice(&strs);
        out.extend_from_slice(&(objects.len() as u32).to_be_bytes());
        out.extend_from_slice(&name_bytes.to_be_bytes());
        out.extend_from_slice(&table);
        out
    }
}

// ------------------------------------------------------------------------------------------------ sound objects

/// Field sizes of the sound header by mask bit (F-65201).
const FIELD_SIZE: [usize; 24] = [2, 2, 2, 2, 2, 2, 2, 2, 4, 4, 2, 2, 2, 4, 4, 2, 1, 1, 4, 4, 4, 4, 4, 4];

/// The common header of a sound object, with the defaults of F-11939 for absent fields.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundHeader {
    pub kind: u8,
    /// 16 two-bit tristates: 0 off, 1 on, 2 inherit
    pub flags: u32,
    /// "do not inherit from the parent" bits (+9): 0x80 volume, 0x40 pitch, 0x20 pan, 0x10 predelay, 0x01 doppler
    pub no_inherit: u8,
    /// (+10): 0x40 curve scale, 0x20 speaker mask
    pub no_inherit2: u8,
    pub mask: u32,
    /// 0.01 dB
    pub volume: i16,
    pub volume_var: u16,
    /// cents
    pub pitch: i16,
    pub pitch_var: u16,
    /// degrees, -1 = none
    pub pan: i16,
    pub pan_var: u16,
    /// ms
    pub predelay: u16,
    pub predelay_var: u16,
    /// ms, or percent of the duration when tristate 0 is on
    pub start_offset: u32,
    pub start_offset_var: u32,
    pub attack: u16,
    /// ms, -1 = none
    pub release: i16,
    /// percent
    pub doppler: u16,
    /// category name hash (0 = inherit)
    pub category: u32,
    /// rolloff curve name hash (0 = inherit)
    pub curve: u32,
    /// percent
    pub curve_scale: u16,
    pub speaker_mask: u8,
    pub route: u8,
    pub volume_var_hash: u32,
    pub pitch_var_hash: u32,
    pub pan_var_hash: u32,
    pub predelay_var_hash: u32,
    pub start_offset_var_hash: u32,
    pub lpf_var_hash: u32,
}

impl SoundHeader {
    /// Tristate pair k: 0 off, 1 on, 2 inherit.
    pub fn tristate(&self, k: u32) -> u8 {
        ((self.flags >> (2 * k)) & 3) as u8
    }
    pub fn has(&self, bit: u32) -> bool {
        self.mask >> bit & 1 != 0
    }
}

/// Parse the header; returns it and the offset of the type payload.
pub fn parse_header(o: &[u8]) -> (SoundHeader, usize) {
    let mask = be_u32(o, 11);
    let mut vals = [0u32; 24];
    let mut p = 15;
    for (b, &sz) in FIELD_SIZE.iter().enumerate() {
        if mask >> b & 1 != 0 {
            vals[b] = match sz {
                1 => o.get(p).copied().unwrap_or(0) as u32,
                2 => be_u16(o, p) as u32,
                _ => be_u32(o, p),
            };
            p += sz;
        }
    }
    let get = |b: u32, default: u32| if mask >> b & 1 != 0 { vals[b as usize] } else { default };
    let h = SoundHeader {
        kind: o.first().copied().unwrap_or(0),
        flags: be_u32(o, 5),
        no_inherit: o.get(9).copied().unwrap_or(0),
        no_inherit2: o.get(10).copied().unwrap_or(0),
        mask,
        volume: get(0, 0) as u16 as i16,
        volume_var: get(1, 0) as u16,
        pitch: get(2, 0) as u16 as i16,
        pitch_var: get(3, 0) as u16,
        pan: get(4, 0xFFFF) as u16 as i16,
        pan_var: get(5, 0) as u16,
        predelay: get(6, 0) as u16,
        predelay_var: get(7, 0) as u16,
        start_offset: get(8, 0),
        start_offset_var: get(9, 0),
        attack: get(10, 0) as u16,
        release: get(11, 0xFFFF) as u16 as i16,
        doppler: get(12, 100) as u16,
        category: get(13, 0),
        curve: get(14, 0),
        curve_scale: get(15, 100) as u16,
        speaker_mask: get(16, 0) as u8,
        route: get(17, 0) as u8,
        volume_var_hash: get(18, 0),
        pitch_var_hash: get(19, 0),
        pan_var_hash: get(20, 0),
        predelay_var_hash: get(21, 0),
        start_offset_var_hash: get(22, 0),
        lpf_var_hash: get(23, 0),
    };
    (h, p)
}

/// A sound type's payload (section 4 of SOUND_FORMAT.md). Children are name hashes (0 / 0xFFFFFFFF = none).
#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    /// 1: loop count (-1 = infinite), restart offset ms, child
    Looping { count: i16, restart_ms: u16, child: u32 },
    /// 2: ADSR on the child's volume
    Envelope { attack: u16, decay: u16, sustain: u8, hold: i32, release: i32, curves: [u32; 3], child: u32 },
    /// 3: two looping instances cross-faded at random intervals
    TwinLoop { swap: (u16, u16), xfade: (u16, u16), curve: u32, children: Vec<u32> },
    /// 4: speech (content from the game)
    Speech,
    /// 5: child, the sound started on a stop request, a third (on finish?)
    OnStop { child: u32, on_stop: u32, third: u32 },
    /// 6
    Wrapper { child: u32 },
    /// 7
    Sequential { children: Vec<u32> },
    /// 8: duration ms, one Simple per stream channel
    Streaming { duration_ms: u32, children: Vec<u32> },
    /// 9: a new instance of the child every `delay` ms, `count` times (-1 = forever)
    Retriggered { count: i16, delay_ms: u16, child: u32 },
    /// 10: near / far children cross-faded by distance
    Crossfade { near: u32, far: u32, min: f32, max: f32, curve: u32 },
    /// 12: wave bank (name hash) and wave (name hash)
    Simple { bank: u32, wave: u32 },
    /// 13: all children at once
    Multitrack { children: Vec<u32> },
    /// 14: one child picked at Init (history kept in the object, see [`Randomized`])
    Randomized { step_var: u32, history_len: u8, children: Vec<(u32, f32)> },
    /// 22: variables for the subtree (name hash, initial value)
    VariableBlock { child: u32, vars: Vec<(u32, f32)> },
    /// 25: per-update ops (opcode, 3 x (constant, variable hash))
    MathOperation { child: u32, ops: Vec<(u8, [(f32, u32); 3], u32)> },
    Other(u8),
}

pub fn parse_payload(kind: u8, pl: &[u8]) -> Payload {
    let list = |n_at: usize, first: usize| -> Vec<u32> {
        let n = pl.get(n_at).copied().unwrap_or(0) as usize;
        (0..n).map(|i| be_u32(pl, first + 8 * i)).collect()
    };
    match kind {
        1 => Payload::Looping { count: be_i16(pl, 0), restart_ms: be_u16(pl, 2), child: be_u32(pl, 4) },
        2 => Payload::Envelope {
            attack: be_u16(pl, 0),
            decay: be_u16(pl, 2),
            sustain: pl.get(4).copied().unwrap_or(100),
            hold: be_i32(pl, 5),
            release: be_i32(pl, 9),
            curves: [be_u32(pl, 13), be_u32(pl, 17), be_u32(pl, 21)],
            child: be_u32(pl, 45),
        },
        3 => Payload::TwinLoop { swap: (be_u16(pl, 0), be_u16(pl, 2)), xfade: (be_u16(pl, 4), be_u16(pl, 6)), curve: be_u32(pl, 8), children: list(28, 29) },
        4 => Payload::Speech,
        5 => Payload::OnStop { child: be_u32(pl, 0), on_stop: be_u32(pl, 4), third: be_u32(pl, 8) },
        6 => Payload::Wrapper { child: be_u32(pl, 0) },
        7 => Payload::Sequential { children: list(0, 1) },
        8 => Payload::Streaming { duration_ms: be_u32(pl, 0), children: list(4, 5) },
        9 => Payload::Retriggered { count: be_i16(pl, 0), delay_ms: be_u16(pl, 2), child: be_u32(pl, 12) },
        10 => Payload::Crossfade { near: be_u32(pl, 0), far: be_u32(pl, 4), min: be_f32(pl, 9), max: be_f32(pl, 13), curve: be_u32(pl, 37) },
        12 => Payload::Simple { bank: be_u32(pl, 4), wave: be_u32(pl, 8) },
        13 => Payload::Multitrack { children: list(0, 1) },
        14 => {
            let nh = pl.get(5).copied().unwrap_or(0) as usize;
            let n = pl.get(6 + nh).copied().unwrap_or(0) as usize;
            let b = 7 + nh;
            Payload::Randomized { step_var: be_u32(pl, 0), history_len: nh as u8, children: (0..n).map(|i| (be_u32(pl, b + 8 * i), be_f32(pl, b + 8 * i + 4))).collect() }
        }
        22 => {
            let n = pl.get(4).copied().unwrap_or(0) as usize;
            Payload::VariableBlock { child: be_u32(pl, 0), vars: (0..n).map(|i| (be_u32(pl, 5 + 9 * i), be_f32(pl, 9 + 9 * i))).collect() }
        }
        25 => {
            let n = pl.get(4).copied().unwrap_or(0) as usize;
            let ops = (0..n)
                .map(|i| {
                    // 29-byte records (F-90822): opcode, 3 x {f32 constant, u32 variable hash}, u32 output variable hash.
                    let q = 5 + 29 * i;
                    (pl.get(q).copied().unwrap_or(0), [0, 1, 2].map(|k| (be_f32(pl, q + 1 + 8 * k), be_u32(pl, q + 5 + 8 * k))), be_u32(pl, q + 25))
                })
                .collect();
            Payload::MathOperation { child: be_u32(pl, 0), ops }
        }
        k => Payload::Other(k),
    }
}

/// A parsed sound object.
#[derive(Clone, Debug)]
pub struct Sound {
    pub header: SoundHeader,
    pub payload: Payload,
}

pub fn parse_sound(o: &[u8]) -> Sound {
    let (header, p) = parse_header(o);
    let payload = parse_payload(header.kind, o.get(p..).unwrap_or(&[]));
    Sound { header, payload }
}

// ------------------------------------------------------------------------------------------------ curves

/// DEFAULT_ROLLOFF's polynomial constants (0x82006BBC..0x82006BEC, float32).
const ROLL: [f32; 11] = [63.95, 0.027_052_762, -17.042_1, 62.4395, 4.3724, -2.5018, 0.715, 0.9942, -0.2507, -0.5388, 0.2201];

/// The built-in DEFAULT_ROLLOFF (F-98578, curve type 13): 0 dB below 4.99, -100 dB from 128.
pub fn default_rolloff_db(d: f32) -> f32 {
    if d - 128.0 >= 0.0 {
        return -100.0;
    }
    if d - 4.99 < 0.0 {
        return 0.0;
    }
    let x = (d - ROLL[0]) * ROLL[1];
    let c = ROLL;
    x * c[2] - c[3] + c[4] * x.powi(2) + c[5] * x.powi(3) + c[6] * x.powi(4) + c[7] * x.powi(5) + c[8] * x.powi(6) + c[9] * x.powi(7) + c[10] * x.powi(8)
}

/// Distance fed to a rolloff curve: `s` = category rolloff scale * the sound's curve scale. `None` when s = 0
/// (no attenuation).
pub fn scaled_distance(d: f32, s: f32) -> Option<f32> {
    if s == 0.0 {
        None
    } else if s - 1.0 >= 0.0 {
        Some(if d - 5.0 < 0.0 { d } else { (d - 5.0) / s + 5.0 })
    } else {
        Some(d / s)
    }
}

/// A curves.dat object (Init F-57265, Eval F-33490).
#[derive(Clone, Debug)]
pub struct Curve {
    pub kind: u8,
    pub flags: u32,
    pub min: f32,
    pub max: f32,
    params: CurveParams,
}

#[derive(Clone, Debug)]
enum CurveParams {
    Const(f32),
    Points(Vec<(f32, f32)>),
    Sine { flip_in: bool, flip_out: bool },
    Table(Vec<f32>),
    Pow { flip_in: bool, flip_out: bool, exp: f32 },
    Gauss { k: f32, norm: bool },
    Clamp { db: f32, norm: bool },
    None,
}

/// The vector powf (F-33229) of e (single, 0x402DF854) to `y` as the recomp computes it: vexptefp(log2f(e) * y) with
/// denormals flushed - 2^floor times the SDK's fourth-order polynomial of the fraction, the power of two built in the
/// exponent field (wrapping as the SDK's integer add and shift do) - and 1 for y == 0.
fn vpow_e(y: f32) -> f32 {
    let ftz = |v: f32| if v.is_subnormal() { 0.0f32.copysign(v) } else { v };
    if y == 0.0 {
        return 1.0;
    }
    let x = ftz(f32::from_bits(0x3FB8_AA3B) * ftz(y));
    let n = x.floor();
    let f = ftz(x - n);
    let mut p = 1.877_576_7e-3f32;
    for c in [8.989_339_7e-3f32, 5.582_631_8e-2, 2.401_536_1e-1, 6.931_530_8e-1, 1.0] {
        p = ftz(ftz(p * f) + c);
    }
    let ni = if n >= -2147483648.0 && n < 2147483648.0 { n as i32 } else { i32::MIN };
    ftz(p * ftz(f32::from_bits((ni.wrapping_add(127) as u32).wrapping_shl(23))))
}

/// The PowerPC `fsel`: `a` when `c >= 0`, else `b` (a NaN `c` picks `b`).
pub fn fsel(c: f32, a: f32, b: f32) -> f32 {
    if c >= 0.0 {
        a
    } else {
        b
    }
}

impl Curve {
    pub fn parse(o: &[u8]) -> Curve {
        let kind = o.first().copied().unwrap_or(0);
        let flags = be_u32(o, 5);
        let b21 = o.get(21).copied().unwrap_or(0);
        let params = match kind {
            1 => CurveParams::Const(be_f32(o, 21)),
            2 | 3 => CurveParams::Points(vec![(be_f32(o, 21), be_f32(o, 25)), (be_f32(o, 29), be_f32(o, 33))]),
            4 => {
                let n = be_u32(o, 21) as usize;
                CurveParams::Points((0..n).map(|i| (be_f32(o, 25 + 8 * i), be_f32(o, 29 + 8 * i))).collect())
            }
            5 => CurveParams::Sine { flip_in: b21 & 1 != 0, flip_out: b21 & 2 != 0 },
            6 => {
                let n = be_u16(o, 21) as usize;
                CurveParams::Table((0..n).map(|i| be_f32(o, 23 + 4 * i)).collect())
            }
            7 => CurveParams::Pow { flip_in: b21 & 1 != 0, flip_out: b21 & 2 != 0, exp: be_f32(o, 22) },
            9 => CurveParams::Gauss { k: be_f32(o, 21), norm: flags & 0xC != 0 },
            14 => CurveParams::Clamp { db: be_i16(o, 21) as f32 * 0.01, norm: flags & 0xC == 4 },
            _ => CurveParams::None,
        };
        Curve { kind, flags, min: be_f32(o, 13), max: be_f32(o, 17), params }
    }

    pub fn eval(&self, x: f32) -> f32 {
        if let (4, CurveParams::Points(p)) = (self.kind, &self.params) {
            // piecewise linear (F-37998); fmadds as one f64 fma, then f32
            let fma = |a: f32, b: f32, c: f32| (a as f64).mul_add(b as f64, c as f64) as f32;
            if p.len() <= 3 {
                // the 3-point form set up at Init (F-39474): missing points continue the last level
                let (x0, y0) = p.first().copied().unwrap_or((0.0, 0.0));
                let (x1, y1) = if p.len() <= 1 { (x0 + 1.0, y0) } else { p[1] };
                let (x2, y2) = match p.len() {
                    3 => p[2],
                    2 => (x1 + 2.0, y1),
                    _ => (x0 + 2.0, y1),
                };
                let (i01, i12) = (1.0 / (x1 - x0), 1.0 / (x2 - x1));
                let (d0, d1, d2) = (x - x0, x - x1, x - x2);
                let a = fma(i01 * d0, y1 - y0, y0);
                let b = fma(i12 * d1, y2 - y1, y1);
                return fsel(d2, y2, fsel(d1, b, fsel(d0, a, y0)));
            }
            // the general search: clamp to the first / last x, the first point with x_i >= x
            let (xf, xl) = (p[0].0, p[p.len() - 1].0);
            let x = fsel(x - xl, xl, x);
            let x = fsel(xf - x, xf, x);
            let mut i = 1;
            while i < p.len() - 1 && !(p[i].0 >= x) {
                i += 1;
            }
            let ((xa, ya), (xb, yb)) = (p[i - 1], p[i]);
            return fma((x - xa) / (xb - xa), yb - ya, ya);
        }
        // (F-33490 @A-60868: fsel against the range's min, then its max - a NaN input becomes the min)
        let x = if self.flags & 3 == 1 { fsel(fsel(x - self.min, x, self.min) - self.max, self.max, fsel(x - self.min, x, self.min)) } else { x };
        match &self.params {
            CurveParams::Const(c) => *c,
            CurveParams::Points(p) => {
                let ((x0, y0), (x1, y1)) = (p[0], p[1]);
                let y = if x <= x0 {
                    y0
                } else if x >= x1 {
                    y1
                } else if y0 == y1 {
                    y0
                } else {
                    // (F-11926: the divide, then one fused multiply-add)
                    ((x - x0) / (x1 - x0)).mul_add(y1 - y0, y0)
                };
                if self.kind == 2 {
                    y
                } else {
                    lin2db(y)
                }
            }
            CurveParams::Sine { flip_in, flip_out } => {
                // (F-87850: fsel to 0, then to 1 - a NaN input becomes 0)
                let xi = if *flip_in { 1.0 - x } else { x };
                let xi = fsel(xi, xi, 0.0);
                let xi = fsel(xi - 1.0, 1.0, xi);
                let y = (xi * std::f32::consts::FRAC_PI_2).sin();
                if *flip_out {
                    1.0 - y
                } else {
                    y
                }
            }
            CurveParams::Table(t) => {
                if t.is_empty() {
                    0.0
                } else {
                    t[(x.floor().max(0.0) as usize).min(t.len() - 1)]
                }
            }
            CurveParams::Pow { flip_in, flip_out, exp } => {
                let xi = if *flip_in { 1.0 - x } else { x };
                let y = if xi > 0.0 { xi.powf(*exp) } else { 0.0 };
                if *flip_out {
                    1.0 - y
                } else {
                    y
                }
            }
            CurveParams::Gauss { k, norm } => {
                // (F-49411: e^-(k x)^2 by the vector powf F-33229 of e in single; normalised by e^-k^2 when F-67749 set +8)
                let a = vpow_e((k * k) * -1.0);
                let b = vpow_e(((k * x) * (k * x)) * -1.0);
                if *norm {
                    (b - a) / (1.0 - a)
                } else {
                    b
                }
            }
            CurveParams::Clamp { db, norm } => {
                let y = default_rolloff_db(x).min(*db);
                if *norm {
                    y - db
                } else {
                    y
                }
            }
            CurveParams::None => {
                if self.kind == 13 {
                    default_rolloff_db(x)
                } else {
                    0.0
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ categories

/// A category after resolution from BASE down (F-52968).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Category {
    /// dB
    pub volume: f32,
    /// cents
    pub pitch: i32,
    pub lpf: u16,
    pub hpf: u16,
    pub rolloff_scale: f32,
    /// the reverb send's amount and distance scale (+25 / +27 percent, products down the tree; b9 0x04 / 0x02 stop the
    /// parent's part - F-41684 / F-52968; BUS_GRAPH_RULE.md 2.4)
    pub reverb_amount: f32,
    pub reverb_dist: f32,
}

/// The inlined vector powf(10, y) (F-33229) as the recomp computes it (as busfx.rs): vexpte(f32(log2(10) * y)).
fn pow10_vec(y: f32) -> f32 {
    let x = f32::from_bits(0x4054_9A78) * y;
    let n = (x as f64).floor();
    let fr = (x as f64 - n) as f32;
    let mut p = 1.877_576_7e-3f32;
    for c in [8.989_339_7e-3f32, 5.582_631_8e-2, 2.401_536_1e-1, 6.931_530_8e-1, 1.0] {
        p = p * fr + c;
    }
    (p as f64 * 2f64.powi(n as i32)) as f32
}

/// A category record's own volume (dB) and rolloff scale (F-31694, called by the loader F-41684 with the global blend factor,
/// always 1.0 - SOUND_FORMAT.md 5.1): `vol = lin2dB(fmadds(dB2lin(+11 * 0.01) - dB2lin(+13 * 0.01), f, dB2lin(+13 * 0.01)))`,
/// `scale = fmadds(+21 * 0.01 - +23 * 0.01, f, +23 * 0.01)`; dB2lin F-42985 with the vector powf, lin2dB the vector log2.
/// (With the factor 1 the blend is not +11 itself: (a - b) + b rounds.)
pub fn category_record(v11: i16, v13: i16, s21: u16, s23: u16) -> (f32, f32) {
    let (k21, k23) = (s21 as f32 * 0.01, s23 as f32 * 0.01);
    (lin2db(category_lin(v11, v13)), (k21 - k23).mul_add(1.0, k23))
}

/// The linear gain of [`category_record`]'s volume, before lin2dB.
fn category_lin(v11: i16, v13: i16) -> f32 {
    let d2l = |db: f32| if db - (-100.0) >= 0.0 { pow10_vec(db * f32::from_bits(0x3D4C_CCCD)) } else { 0.0 };
    let (a11, a13) = (d2l(v11 as f32 * 0.01), d2l(v13 as f32 * 0.01));
    (a11 - a13).mul_add(1.0, a13)
}

/// Resolve every category (by name hash) from BASE: volume and pitch add, LPF min, HPF max, rolloff scale multiplies;
/// bits of +9 stop the parent's part (0x80 volume, 0x40 pitch, 0x20 LPF, 0x10 HPF, 0x08 rolloff scale).
pub fn resolve_categories(cats: &Dat) -> HashMap<u32, Category> {
    resolve_categories_with(cats, &HashMap::new())
}

/// As [`resolve_categories`] with the run-time volume of the categories' controllers (the volume sliders, the camera
/// table ...): `runtime[hash]` dB (20 log10 of the product of the controllers' gains, F-61464) is added to that
/// category's own volume and so passed down to its children.
pub fn resolve_categories_with(cats: &Dat, runtime: &HashMap<u32, f32>) -> HashMap<u32, Category> {
    resolve_categories_rt(cats, runtime, &HashMap::new(), &HashMap::new())
}

/// The same with the controllers' pitch too: `cents[hash]` (trunc(1200 log2 of the product of the controllers'
/// ratios), F-61464) is added to that category's own pitch and passed down.
/// `filters`: the category's runtime LPF / HPF in Hz (+70 / +72: its controllers' minimum LPF and maximum HPF, F-61464),
/// taken with the category's own as LPF min and HPF max (a category without an entry: 23900 / 0, the record's start values,
/// F-26947). Pitch sums are s16 and wrap (F-52968).
pub fn resolve_categories_rt(cats: &Dat, runtime: &HashMap<u32, f32>, cents: &HashMap<u32, i32>, filters: &HashMap<u32, (u16, u16)>) -> HashMap<u32, Category> {
    let mut out = HashMap::new();
    #[allow(clippy::too_many_arguments)]
    fn walk(cats: &Dat, runtime: &HashMap<u32, f32>, cents: &HashMap<u32, i32>, filters: &HashMap<u32, (u16, u16)>, h: u32, parent: Option<Category>, out: &mut HashMap<u32, Category>, depth: usize) {
        let Some(o) = cats.get(h) else { return };
        if depth > 32 {
            return;
        }
        let b9 = o.get(9).copied().unwrap_or(0);
        let (vol, scale) = category_record(be_i16(o, 11), be_i16(o, 13), be_u16(o, 21), be_u16(o, 23));
        let own_vol = vol + runtime.get(&h).copied().unwrap_or(0.0);
        let own = Category { volume: own_vol, pitch: (be_u16(o, 15).wrapping_add(cents.get(&h).copied().unwrap_or(0) as u16)) as i16 as i32, lpf: be_u16(o, 17), hpf: be_u16(o, 19), rolloff_scale: scale, reverb_amount: be_u16(o, 25) as f32 * 0.01, reverb_dist: be_u16(o, 27) as f32 * 0.01 };
        // (the record's runtime LPF / HPF start at 23900 / 0, F-26947, and the walk always takes them: F-52968)
        let (l, hp) = filters.get(&h).copied().unwrap_or((23900, 0));
        let own = Category { lpf: own.lpf.min(l), hpf: own.hpf.max(hp), ..own };
        let c = match parent {
            None => own,
            Some(p) => Category {
                volume: own.volume + if b9 & 0x80 != 0 { 0.0 } else { p.volume },
                // (s16 sums, F-52968: lhz / add / extsh)
                pitch: (own.pitch as u16).wrapping_add(if b9 & 0x40 != 0 { 0 } else { p.pitch as u16 }) as i16 as i32,
                lpf: if b9 & 0x20 != 0 { own.lpf } else { own.lpf.min(p.lpf) },
                hpf: if b9 & 0x10 != 0 { own.hpf } else { own.hpf.max(p.hpf) },
                rolloff_scale: own.rolloff_scale * if b9 & 0x08 != 0 { 1.0 } else { p.rolloff_scale },
                reverb_amount: own.reverb_amount * if b9 & 0x04 != 0 { 1.0 } else { p.reverb_amount },
                reverb_dist: own.reverb_dist * if b9 & 0x02 != 0 { 1.0 } else { p.reverb_dist },
            },
        };
        out.insert(h, c);
        let n = o.get(35).copied().unwrap_or(0) as usize;
        for i in 0..n {
            walk(cats, runtime, cents, filters, be_u32(o, 36 + 4 * i), Some(c), out, depth + 1);
        }
    }
    walk(cats, runtime, cents, filters, joaat("BASE"), None, &mut out, 0);
    out
}

/// The audio timer of every category (by name hash), from BASE down: tristate pair 1 of the object's +5 word (the run-time
/// record's +76, copied by F-41684 and read by F-24707 - docs/reference/audio/SOUND_TIMER_RULE.md): 1 =
/// timer 1, 2 = the parent's, 0 / 3 = timer 0; BASE's parent is timer 0.
pub fn category_timers(cats: &Dat) -> HashMap<u32, u8> {
    let mut out = HashMap::new();
    fn walk(cats: &Dat, h: u32, parent: u8, out: &mut HashMap<u32, u8>, depth: usize) {
        let Some(o) = cats.get(h) else { return };
        if depth > 32 {
            return;
        }
        let t = match (be_u32(o, 5) >> 2) & 3 {
            1 => 1,
            2 => parent,
            _ => 0,
        };
        out.insert(h, t);
        let n = o.get(35).copied().unwrap_or(0) as usize;
        for i in 0..n {
            walk(cats, be_u32(o, 36 + 4 * i), t, out, depth + 1);
        }
    }
    walk(cats, joaat("BASE"), 0, &mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn math_operation_records_are_29_bytes() {
        // ZZZ_TEST_MATH's payload from sounds.dat: one op, opcode 12, output variable 0x49DC73B3.
        let hex = "ffffffff010c00000000ffd9332d000000009bf98846000000000000000049dc73b3";
        let pl: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let Payload::MathOperation { child, ops } = parse_payload(25, &pl) else { panic!() };
        assert_eq!(child, 0xFFFF_FFFF);
        assert_eq!(ops, vec![(12, [(0.0, 0xFFD9_332D), (0.0, 0x9BF9_8846), (0.0, 0)], 0x49DC_73B3)]);
    }


    /// F-90404 (type 3 curve: segment, then dB) against the original's own code on 4000 cases, run with the
    /// vector shim (work/audio2/verify/ref3.py); the shim was checked against an independent f32 evaluation
    #[test]
    fn curve3_is_the_originals() {
        let mut bad = 0;
        let h = |x: &str| u32::from_str_radix(&x.replace('_', ""), 16).unwrap();
        for line in include_str!("fixtures/curve3_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<f32> = i.split(' ').map(|x| f32::from_bits(h(x))).collect();
            let c = Curve { kind: 3, flags: 0, min: 0.0, max: 0.0, params: CurveParams::Points(vec![(v[1], v[2]), (v[3], v[4])]) };
            if c.eval(v[0]).to_bits() != h(o) {
                bad += 1;
            }
        }
        assert_eq!(bad, 0);
    }

    /// The 12 of the 4000 cases above where the recorder's host `log2f` is not correctly rounded (the shim stands in for
    /// the console's log instruction with the host library, which differs from platform to platform, e.g. Apple Silicon):
    /// the port takes the correctly rounded log2, the recorded value is within 4 ulp of it. The console's own log
    /// instruction is an estimate whose bits were not recorded, so these stay unverified to the bit.
    #[test]
    fn curve3_log2_ambiguous_cases() {
        let h = |x: &str| u32::from_str_radix(&x.replace('_', ""), 16).unwrap();
        let mut n = 0;
        for line in include_str!("fixtures/curve3_log2_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<f32> = i.split(' ').map(|x| f32::from_bits(h(x))).collect();
            let c = Curve { kind: 3, flags: 0, min: 0.0, max: 0.0, params: CurveParams::Points(vec![(v[1], v[2]), (v[3], v[4])]) };
            let (got, want) = (c.eval(v[0]).to_bits() as i64, h(o) as i64);
            assert!((got - want).abs() <= 4, "{line}");
            n += 1;
        }
        assert_eq!(n, 12);
    }

    /// F-11926 (type 2 curve: segment eval) against the original's own code on 4000 cases (work/audio2/verify/ref1.py)
    #[test]
    fn curve2_is_the_originals() {
        let mut bad = 0;
        for line in include_str!("fixtures/curve2_cases.txt").lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let v: Vec<f32> = i.split(' ').skip(1).map(|x| f32::from_bits(u32::from_str_radix(&x.replace('_', ""), 16).unwrap())).collect();
            let want: u32 = { let x = o.split(' ').nth(1).unwrap(); u32::from_str_radix(&x.replace('_', ""), 16).unwrap() };
            let c = Curve { kind: 2, flags: 0, min: 0.0, max: 0.0, params: CurveParams::Points(vec![(v[1], v[2]), (v[3], v[4])]) };
            if c.eval(v[0]).to_bits() != want {
                bad += 1;
            }
        }
        assert_eq!(bad, 0);
    }
    use super::*;

    #[test]
    fn rolloff_values() {
        // SOUND_FORMAT.md 6.1: 5: -0.41, 10: -14.1, 20: -30.7, 40: -48.9, 64: -62.5, 100: -76.2
        for (d, want) in [(5.0, -0.41), (10.0, -14.1), (20.0, -30.7), (40.0, -48.9), (64.0, -62.5), (100.0, -76.2)] {
            assert!((default_rolloff_db(d) - want).abs() < 0.06, "{d}: {}", default_rolloff_db(d));
        }
        assert_eq!(default_rolloff_db(4.0), 0.0);
        assert_eq!(default_rolloff_db(128.0), -100.0);
        assert_eq!(scaled_distance(30.0, 5.0), Some(10.0));
        assert_eq!(scaled_distance(3.0, 5.0), Some(3.0));
    }

    #[test]
    fn db_helpers() {
        assert!((lin2db(0.5) + 6.02).abs() < 0.01);
        assert_eq!(lin2db(0.0), -100.0);
        assert_eq!(db2lin(-101.0), 0.0);
        assert!((db2lin(-6.0) - 0.501).abs() < 0.001);
    }

    #[test]
    fn header_fields_in_bit_order() {
        // type 6, mask bits 0, 2, 3, 13 (+16): -600 mB, -500 cents, 200, category hash, speaker mask 0
        let mut o = vec![6u8, 0, 0, 0, 0, 0, 0, 2, 0xAA, 0, 0];
        o.extend_from_slice(&0x0001_200Du32.to_be_bytes());
        o.extend_from_slice(&(-600i16).to_be_bytes());
        o.extend_from_slice(&(-500i16).to_be_bytes());
        o.extend_from_slice(&200u16.to_be_bytes());
        o.extend_from_slice(&0x8856_f65au32.to_be_bytes());
        o.push(0);
        o.extend_from_slice(&0x1234_5678u32.to_be_bytes());
        let s = parse_sound(&o);
        assert_eq!((s.header.volume, s.header.pitch, s.header.pitch_var, s.header.category), (-600, -500, 200, 0x8856_f65a));
        assert_eq!((s.header.pan, s.header.release, s.header.doppler, s.header.curve_scale), (-1, -1, 100, 100));
        assert_eq!(s.payload, Payload::Wrapper { child: 0x1234_5678 });
    }

    /// A built file reads back; merging keeps the first file's objects and adds the rest with their banks.
    #[test]
    fn build_and_extend() {
        let a = Dat::parse(&Dat::build(0x16, &[("ONE", &[1, 2, 3]), ("TWO", &[4])], &[])).unwrap();
        assert_eq!((a.version, a.len(), a.get_name("one"), a.banks.len()), (0x16, 2, Some(&[1u8, 2, 3][..]), 0));
        let b = Dat::parse(&Dat::build(0x10, &[("TWO", &[9, 9]), ("THREE", &[7, 8])], &["PLAYER_VEHICLES\\VP_X_08", "VEHICLES\\VEHICLE_AUX"])).unwrap();
        assert_eq!(b.banks, ["PLAYER_VEHICLES\\VP_X_08", "VEHICLES\\VEHICLE_AUX"]);
        let mut m = a;
        m.banks.push("vehicles/vehicle_aux".into());
        assert_eq!(m.extend_from(&b), ["TWO"]);
        assert_eq!((m.get_name("TWO"), m.get_name("THREE"), m.get_name("ONE")), (Some(&[4u8][..]), Some(&[7u8, 8][..]), Some(&[1u8, 2, 3][..])));
        assert_eq!(m.banks, ["vehicles/vehicle_aux", "PLAYER_VEHICLES\\VP_X_08"], "the same bank by hash is not listed twice");
    }

    /// F-31694 against the original's code run offline (tools/category_record_reference.py, through the loader F-41684: every
    /// category of the disc and 3,000 random field sets): the record's +8 rolloff scale and +12 / +16 reverb sends bit for bit; the +4 volume bit for bit except where the
    /// harness's stand-in for the console's log instruction (the host log2f, not correctly rounded) and the port's
    /// correctly rounded log2 part (as curve3_log2_ambiguous_cases): 3 distinct gains (+11 = -1, 29, 404; -1 is an edge
    /// value the random sets draw often, 43 cases), 1 - 2 ulp. For those the gain before the log is bit exact, and on the
    /// host the harness ran on (windows-gnu) the f32 log2f path gives the reference word exactly.
    #[test]
    fn category_record_is_the_originals() {
        let t = include_str!("testdata/category_record_ref.txt");
        let (mut n, mut disc, mut gains) = (0, 0, std::collections::HashSet::new());
        for l in t.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            let num = |i: usize| w[i][1..].parse::<i64>().unwrap();
            let hex = |i: usize| u32::from_str_radix(&w[i][1..], 16).unwrap();
            let (v, k) = category_record(num(0) as i16, num(1) as i16, num(2) as u16, num(3) as u16);
            // (the loader's reverb sends, as resolve_categories_rt reads them)
            let (ra, rd) = (num(4) as u16 as f32 * 0.01, num(5) as u16 as f32 * 0.01);
            assert_eq!((k.to_bits(), ra.to_bits(), rd.to_bits()), (hex(8), hex(9), hex(10)), "{l}");
            if v.to_bits() != hex(7) {
                assert!((v.to_bits() as i64 - hex(7) as i64).abs() <= 4, "{l}");
                assert!(n >= 146, "a category of the disc: {l}");
                let lin = category_lin(num(0) as i16, num(1) as i16);
                if cfg!(all(target_os = "windows", target_env = "gnu")) {
                    assert_eq!((lin.log2() * LOG10_2 * 20.0).to_bits(), hex(7), "only the log2 rounding differs: {l}");
                }
                gains.insert(lin.to_bits());
            } else if n < 146 {
                disc += 1;
            }
            n += 1;
        }
        assert_eq!((n, disc), (3146, 146));
        assert!(gains.len() <= 3, "{gains:x?}");
    }

    /// The category tree against the original's code run offline (tools/category_tree_reference.py): the loader F-45518 whole
    /// (the .dat reader, the build from BASE F-48971 / F-31533 / F-26947 / F-41684, the sort) on the disc's categories.dat and
    /// copies with random own fields, the runtime volume / pitch / LPF / HPF set through the setters F-87261 / F-60329 /
    /// F-99639 / F-29180, the walk F-52968 through F-85160. Every resolved category's volume, rolloff scale, reverb sends,
    /// pitch, LPF and HPF equal resolve_categories_rt's (volumes as category_record_is_the_originals: the log2 rounding).
    #[test]
    fn category_tree_is_the_originals() {
        let t = include_str!("testdata/category_tree_ref.txt");
        let hx = |w: &str| u32::from_str_radix(w, 16).unwrap();
        let mut dat: Option<Dat> = None;
        let (mut rt, mut cents, mut filt) = (HashMap::new(), HashMap::new(), HashMap::new());
        let mut got: Option<HashMap<u32, Category>> = None;
        let (mut n, mut near, mut bad) = (0, 0, Vec::new());
        let mut objs: Vec<(u32, Vec<u8>)> = Vec::new();
        for l in t.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            match w[0] {
                "L" => {
                    objs.clear();
                    dat = None;
                }
                "O" => objs.push((hx(w[1]), (0..w[2].len() / 2).map(|i| u8::from_str_radix(&w[2][2 * i..2 * i + 2], 16).unwrap()).collect())),
                "C" => {
                    if dat.is_none() {
                        let mut d = Dat { version: 15, data: Vec::new(), objects: Vec::new(), by_hash: HashMap::new(), banks: Vec::new() };
                        for (h, b) in &objs {
                            d.by_hash.insert(*h, d.objects.len());
                            d.objects.push((String::new(), d.data.len(), b.len()));
                            d.data.extend_from_slice(b);
                        }
                        dat = Some(d);
                    }
                    rt.clear();
                    cents.clear();
                    filt.clear();
                    got = None;
                }
                "R" => {
                    let h = hx(w[1]);
                    rt.insert(h, f32::from_bits(hx(w[2])));
                    cents.insert(h, w[3].parse::<u32>().unwrap() as u16 as i16 as i32);
                    filt.insert(h, (w[4].parse().unwrap(), w[5].parse().unwrap()));
                }
                "T" => {
                    let g = got.get_or_insert_with(|| resolve_categories_rt(dat.as_ref().unwrap(), &rt, &cents, &filt));
                    let h = hx(w[1]);
                    n += 1;
                    let Some(c) = g.get(&h) else {
                        bad.push(format!("missing {l}"));
                        continue;
                    };
                    let want = (hx(w[3]), hx(w[4]), hx(w[5]), hx(w[9]) as u16 as i16 as i32, hx(w[10]) as u16, hx(w[11]) as u16);
                    let have = (c.rolloff_scale.to_bits(), c.reverb_amount.to_bits(), c.reverb_dist.to_bits(), c.pitch, c.lpf, c.hpf);
                    let dv = (c.volume.to_bits() as i64 - hx(w[2]) as i64).abs();
                    if have != want || dv > 4 {
                        bad.push(format!("{l} | port vol {:08x} {have:x?}", c.volume.to_bits()));
                    } else if dv != 0 {
                        near += 1;
                    }
                }
                _ => {}
            }
        }
        eprintln!("{n} records, {near} volumes within 4 ulp, {} different", bad.len());
        assert!(bad.is_empty(), "{} of {n}:\n{}", bad.len(), bad[..bad.len().min(20)].join("\n"));
    }

    /// Against the disc's own files (MCLA_DATA_DIR = extracted/cache), when present.
    #[test]
    fn disc_config() {
        let Ok(dir) = std::env::var("MCLA_DATA_DIR") else { return };
        let cfg = std::path::Path::new(&dir).join("audio/X360/config");
        let (Some(snd), Some(crv), Some(cat)) = (Dat::load(&cfg.join("sounds.dat")), Dat::load(&cfg.join("curves.dat")), Dat::load(&cfg.join("categories.dat"))) else { return };
        assert_eq!((snd.len(), crv.len(), cat.len()), (14_683, 86, 146));
        assert_eq!(snd.banks.len(), 1341);
        // DUMPSTER_SCRAPE_MASTER: wrapper, +39 dB under PROPS
        let s = parse_sound(snd.get_name("DUMPSTER_SCRAPE_MASTER").unwrap());
        assert_eq!((s.header.kind, s.header.volume), (6, 3900));
        let cats = resolve_categories(&cat);
        let c = |n: &str| cats[&joaat(n)];
        assert!((c("BASE").volume + 12.0).abs() < 0.01 && (c("BASE").rolloff_scale - 5.0).abs() < 1e-4);
        assert!((c("PROPS").volume + 59.0).abs() < 0.01 && (c("PROPS").rolloff_scale - 10.0).abs() < 1e-4);
        assert!((c("PLAYER_VEHICLES_ENGINES").volume + 25.0).abs() < 0.01);
        let rise = Curve::parse(crv.get_name("EQUAL_POWER_RISE").unwrap());
        assert!((rise.eval(0.5) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        // every Simple's bank is one of the string block's banks
        let banks: std::collections::HashSet<u32> = snd.banks.iter().map(|b| joaat(b)).collect();
        let mut simple = 0;
        for (name, _, _) in &snd.objects {
            if let Payload::Simple { bank, .. } = parse_sound(snd.get_name(name).unwrap()).payload {
                assert!(banks.contains(&bank), "{name}");
                simple += 1;
            }
        }
        assert_eq!(simple, 10_078);
    }
}

/// F-17925 / F-28348 (lookup by hash) and F-86504 / F-73434 (by name): the container's binary search against the original's own code run
/// offline (tools/audio_container_reference.py -> testdata/audio_container_ref.txt: 237 containers of 1 to 100 random names, every
/// hash and name present, other letter cases, key 0, absent hashes next to present ones and random names). The original's table is the
/// sorted (offset, hash) array and its +60 the last index; the port keeps a hash map. Same answer in every query.
#[cfg(test)]
mod original_container_tests {
    use super::*;

    fn dat_of(names: &[&str]) -> Dat {
        let mut d = Vec::new();
        d.extend_from_slice(&15u32.to_be_bytes());
        d.extend_from_slice(&((names.len() * 8) as u32).to_be_bytes());
        for i in 0..names.len() {
            d.extend_from_slice(&(i as u32).to_be_bytes());
            d.extend_from_slice(&[0; 4]);
        }
        d.extend_from_slice(&0u32.to_be_bytes());
        d.extend_from_slice(&(names.len() as u32).to_be_bytes());
        d.extend_from_slice(&0u32.to_be_bytes());
        for (i, nm) in names.iter().enumerate() {
            d.push(nm.len() as u8);
            d.extend_from_slice(nm.as_bytes());
            d.extend_from_slice(&((i * 8) as u32).to_be_bytes());
            d.extend_from_slice(&8u32.to_be_bytes());
        }
        Dat::parse(&d).unwrap()
    }

    #[test]
    fn container_lookups_match_the_original() {
        let (mut containers, mut queries) = (0, 0);
        for line in include_str!("testdata/audio_container_ref.txt").lines() {
            let (a, r) = line.strip_prefix("CNT ").unwrap().split_once(" ->").unwrap();
            let (head, qs) = a.split_once(" | ").unwrap();
            let mut it = head.split(' ');
            let n: usize = it.next().unwrap().parse().unwrap();
            let names: Vec<&str> = it.collect();
            assert_eq!(names.len(), n);
            let dat = dat_of(&names);
            let want: Vec<i32> = r.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let qs: Vec<&str> = qs.split(' ').collect();
            assert_eq!(qs.len(), want.len(), "{line}");
            for (q, w) in qs.iter().zip(want) {
                let got = match q.as_bytes()[0] {
                    b'H' => dat.get(u32::from_str_radix(&q[1..], 16).unwrap()),
                    _ => dat.get_name(&q[1..]),
                };
                let got = got.map_or(-1, |o| u32::from_be_bytes([o[0], o[1], o[2], o[3]]) as i32);
                assert_eq!(got, w, "query {q} in {line}");
                queries += 1;
            }
            containers += 1;
        }
        assert_eq!((containers, queries), (237, 5415));
    }
}

