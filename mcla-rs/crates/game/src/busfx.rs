//! The original's bus effects (BUS_EFFECTS_RULE.md): `audBiquadFilterEffectXenon`, `audReverbEffectXenon` and
//! `audWaveshaperEffectXenon`, the three classes effects.dat uses. Each processes 256 planar samples per channel in
//! place, on the channels of its mask. The operations follow the recompiled code step for step (every scalar step
//! rounded to f32, fmadds as one f64 fma then f32, the reverb's vector code with denormals flushed); the reference runs
//! are in work/audio2/busfx (test `busfx_is_the_originals`). Which bus carries which effect: BUS_GRAPH_RULE.md.

use crate::physics::exact::{guest_cos, guest_sin};
use mcla_formats::audcfg::Curve;

pub const BLOCK: usize = 256;

/// fmadds / fmsubs as the recomp evaluates them: one f64 fma, then f32
fn fma32(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// PowerPC fsel: a >= 0 ? b : c (NaN selects c)
fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 {
        b
    } else {
        c
    }
}

/// The inlined vector powf(10, y) (F-33229) as the recomp computes it: vexpte(f32(log2(10) * y))
fn pow10(y: f32) -> f32 {
    let x = f32::from_bits(0x4054_9A78) * y;
    let n = (x as f64).floor();
    let fr = (x as f64 - n) as f32;
    let mut p = 1.877_576_7e-3f32;
    for c in [8.989_339_7e-3f32, 5.582_631_8e-2, 2.401_536_1e-1, 6.931_530_8e-1, 1.0] {
        p = p * fr + c;
    }
    (p as f64 * 2f64.powi(n as i32)) as f32
}

/// F-42985: 10^(dB/20), 0 below -100 dB
pub(crate) fn db2lin(db: f32) -> f32 {
    fsel(db - -100.0, pow10(db * f32::from_bits(0x3D4C_CCCD)), 0.0)
}

/// zero or denormal (exponent field 0) -> +0.0
fn flush(v: f32) -> f32 {
    if v.to_bits() & 0x7F80_0000 == 0 {
        0.0
    } else {
        v
    }
}

fn active(nch: usize, mask: u8) -> usize {
    (0..nch).filter(|c| mask >> c & 1 != 0).count()
}

/// `audBiquadFilterEffectXenon` (section 2): types 0/1 low-pass, 2/3 high-pass, 4/5 band-pass, 6/7 band-reject,
/// 8/9 peaking, 10 peaking with A x 1.4142; types 1 3 5 7 9 10 run two equal stages.
pub struct Biquad {
    nch: usize,
    mask: u8,
    freq: f32,
    /// +52: the resonance (0 - 3) or the peak's A (8 - 10), linear
    q: f32,
    bw: f32,
    /// +60 output gain, linear
    og: f32,
    typ: u32,
    bypass: bool,
    fourth: bool,
    co: [f32; 5],
    /// s1 s2 t1 t2 per active channel
    st: Vec<[f32; 4]>,
    /// 2 pi / rate and 1 / rate (0x3909421F / 0x37AEC33E at the original's 48 kHz; `at_rate`)
    wk: f32,
    inv_sr: f32,
}

impl Biquad {
    /// The constructor and Initialize (F-41869 / F-10599): an active 4th-order low-pass at 23900 Hz.
    pub fn new(nch: usize, mask: u8) -> Biquad {
        let mut b = Biquad { nch, mask, freq: 23900.0, q: 0.0, bw: 500.0, og: 0.0, typ: 1, bypass: false, fourth: true, co: [0.0; 5], st: vec![[0.0; 4]; active(nch, mask)], wk: f32::from_bits(0x3909_421F), inv_sr: f32::from_bits(0x37AE_C33E) };
        b.q = pow10((0.0 - f32::from_bits(0x4040_A3D7)) * f32::from_bits(0x3D4C_CCCD));
        b.og = pow10(0.0 * f32::from_bits(0x3D4C_CCCD));
        b.design();
        b
    }

    /// The same filter in a mix at `rate` Hz (the cutoffs keep their Hz; f32 2 pi / 48000 is the original's word)
    pub fn at_rate(mut self, rate: u32) -> Biquad {
        self.wk = std::f32::consts::TAU / rate as f32;
        self.inv_sr = 1.0 / rate as f32;
        self.design();
        self
    }

    /// SetParameters F-76373: frequency Hz, resonance / gain dB, bandwidth Hz, output gain dB, type, bypass.
    pub fn set(&mut self, freq: f32, g: f32, bw: f32, og: f32, typ: u32, bypass: u8) {
        let f = fsel(freq - 50.0, freq, 50.0);
        let f = fsel(f - 23900.0, 23900.0, f);
        self.freq = f;
        let g = fsel(g - -48.0, g, -48.0);
        let g = fsel(g - 24.0, 24.0, g);
        let bw = fsel(bw - 1.0, bw, 1.0);
        let bw = fsel(bw - 10000.0, 10000.0, bw);
        let f2 = f * 2.0;
        self.bw = fsel(bw - f2, f2, bw);
        let og = fsel(og - -40.0, og, -40.0);
        let og = fsel(og - 24.0, 24.0, og);
        self.typ = typ.min(10);
        self.bypass = bypass != 0;
        self.fourth = matches!(self.typ, 1 | 3 | 5 | 7 | 9 | 10);
        let k = f32::from_bits(0x3D4C_CCCD);
        self.q = pow10((g - f32::from_bits(0x4040_A3D7)) * k);
        self.og = pow10(og * k);
        self.design();
        if self.bypass {
            self.st.iter_mut().for_each(|s| *s = [0.0; 4]);
        }
    }

    /// The designer F-93822 (only while active)
    fn design(&mut self) {
        if self.bypass {
            return;
        }
        let t = self.typ;
        let w = self.freq * self.wk;
        self.co = match t {
            0..=3 => {
                let k = if self.fourth { 0.5 } else { 1.0 };
                let c = guest_cos(w as f64) as f32;
                let s = guest_sin(w as f64) as f32;
                let sh = ((0.5 / (self.q * k)) as f64).sinh() as f32;
                let al = sh * s;
                let (n1, n0) = if t < 2 { (1.0 - c, (1.0 - c) * 0.5) } else { (-1.0 - c, (c + 1.0) * 0.5) };
                let inv = 1.0 / (al + 1.0);
                [inv * n0, inv * n1, inv * n0, (c * -2.0) * inv, (1.0 - al) * inv]
            }
            4..=7 => {
                let tn = (((self.bw * self.inv_sr) * std::f32::consts::PI) as f64).tan() as f32;
                let c = guest_cos(w as f64) as f32;
                let d = c * 2.0;
                if t < 6 {
                    let cc = 1.0 / tn;
                    let b0 = 1.0 / (cc + 1.0);
                    [b0, 0.0, -b0, -((b0 * d) * cc), (cc - 1.0) * b0]
                } else {
                    let b0 = 1.0 / (tn + 1.0);
                    let m = -(b0 * d);
                    [b0, m, b0, m, (1.0 - tn) * b0]
                }
            }
            _ => {
                let q = self.freq / self.bw;
                let s = guest_sin(w as f64) as f32;
                let al = s / (q * 2.0);
                let c = guest_cos(w as f64) as f32;
                let a = if t < 10 { self.q } else { self.q * f32::from_bits(0x3FB5_0481) };
                let (ad, am) = (al / a, a * al);
                let inv = 1.0 / (ad + 1.0);
                let m1 = (c * -2.0) * inv;
                [(am + 1.0) * inv, m1, (1.0 - am) * inv, m1, (1.0 - ad) * inv]
            }
        };
    }

    /// Process F-34766: transposed direct form II, the output gain on every sample, the states flushed once a call.
    pub fn process(&mut self, buf: &mut [f32]) {
        if self.bypass {
            return;
        }
        let [b0, b1, b2, a1, a2] = self.co;
        let g = self.og;
        let mut k = 0;
        for ch in 0..self.nch {
            if self.mask >> ch & 1 == 0 {
                continue;
            }
            let [mut s1, mut s2, mut t1, mut t2] = self.st[k];
            for x in &mut buf[BLOCK * ch..BLOCK * ch + BLOCK] {
                if self.fourth {
                    let y1 = fma32(*x, b0, s1);
                    let y2 = fma32(y1, b0, t1);
                    s1 = fma32(*x, b1, -(y1 * a1)) + s2;
                    s2 = fma32(*x, b2, -(y1 * a2));
                    t1 = fma32(y1, b1, -(y2 * a1)) + t2;
                    t2 = fma32(y1, b2, -(y2 * a2));
                    *x = y2 * g;
                } else {
                    let y = fma32(*x, b0, s1);
                    s1 = fma32(*x, b1, -(y * a1)) + s2;
                    s2 = fma32(*x, b2, -(y * a2));
                    *x = y * g;
                }
            }
            self.st[k] = [flush(s1), flush(s2), flush(t1), flush(t2)];
            k += 1;
        }
    }
}

const COMB: [usize; 8] = [1116, 1188, 1276, 1356, 1424, 1491, 1556, 1616];
const AP: [usize; 4] = [556, 440, 340, 224];

/// vector ops with MXCSR FTZ|DAZ: denormal operands read as 0, denormal results written as 0
fn ftz(x: f32) -> f32 {
    if x != 0.0 && x.abs() < f32::MIN_POSITIVE {
        0.0f32.copysign(x)
    } else {
        x
    }
}
fn vadd(a: f32, b: f32) -> f32 {
    ftz(ftz(a) + ftz(b))
}
fn vsub(a: f32, b: f32) -> f32 {
    ftz(ftz(a) - ftz(b))
}
fn vmul(a: f32, b: f32) -> f32 {
    ftz(ftz(a) * ftz(b))
}
/// vmsum4fp128 as the recomp's dpps: (p3 + p2) + (p1 + p0)
fn dp4(v: &[f32; 4], x: &[f32]) -> f32 {
    let p: [f32; 4] = std::array::from_fn(|e| vmul(v[e], x[e]));
    vadd(vadd(p[3], p[2]), vadd(p[1], p[0]))
}

struct RevChannel {
    fs: [f32; 8],
    ci: [usize; 8],
    ai: [usize; 4],
    /// comb lines (+1: comb 5's vector at idx & ~3 reaches float 1491, one past the line)
    comb: [Vec<f32>; 8],
    ap: [Vec<f32>; 4],
}

/// `audReverbEffectXenon` (section 3): per channel 8 damped combs and 4 allpasses, channels independent.
pub struct Reverb {
    nch: usize,
    mask: u8,
    fb: f32,
    wet: f32,
    dry: f32,
    damp: f32,
    a28: [f32; 4],
    a36: [f32; 4],
    ch: Vec<RevChannel>,
}

impl Reverb {
    /// Initialize F-32898's defaults: feedback 0.84, wet 0, dry 2, damp 0.2 (outputs 2 x input until set)
    pub fn new(nch: usize, mask: u8) -> Reverb {
        let damp = f32::from_bits(0x3E4C_CCCD);
        let mut a28 = [f32::from_bits(0x3E2C_0831); 4];
        for i in 1..4 {
            a28[i] = a28[i - 1] * damp;
        }
        let mut a36 = [0.0, 0.0, 0.0, f32::from_bits(0x3F4C_CCCD)];
        for i in (0..3).rev() {
            a36[i] = a36[i + 1] * damp;
        }
        let ch = (0..active(nch, mask))
            .map(|_| RevChannel { fs: [0.0; 8], ci: [0; 8], ai: [0; 4], comb: COMB.map(|n| vec![0.0; n + 1]), ap: AP.map(|n| vec![0.0; n]) })
            .collect();
        Reverb { nch, mask, fb: f32::from_bits(0x3F57_0A3D), wet: 0.0, dry: 2.0, damp, a28, a36, ch }
    }

    /// SetParameters F-91792: room, wet, dry, damp, each clamped to [0, 1]
    pub fn set(&mut self, p: [f32; 4]) {
        let p = p.map(|v| fsel(v, v, 0.0)).map(|v| fsel(v - 1.0, 1.0, v));
        self.fb = fma32(p[0], f32::from_bits(0x3E8F_5C29), f32::from_bits(0x3F33_3333));
        self.dry = p[2] * 2.0;
        self.wet = p[1] * 3.0;
        self.damp = p[3] * f32::from_bits(0x3ECC_CCCD);
        let d = self.damp;
        self.a28[0] = self.fb * d;
        for i in 1..4 {
            self.a28[i] = d * self.a28[i - 1];
        }
        self.a36 = [0.0, 0.0, 0.0, 1.0 - d];
        for i in (0..3).rev() {
            self.a36[i] = self.a36[i + 1] * d;
        }
    }

    /// Process F-13439
    pub fn process(&mut self, buf: &mut [f32]) {
        let a = self.a36;
        let taps = [a, [a[1], a[2], a[3], 0.0], [a[2], a[3], 0.0, 0.0], [a[3], 0.0, 0.0, 0.0]];
        let fixed = f32::from_bits(0x3C75_C28F);
        let mut k = 0;
        for c in 0..self.nch {
            if self.mask >> c & 1 == 0 {
                continue;
            }
            let st = &mut self.ch[k];
            let base = BLOCK * c;
            let mut sums = [[0.0f32; 4]; BLOCK / 4];
            for (v, acc) in sums.iter_mut().enumerate() {
                let xin: [f32; 4] = std::array::from_fn(|i| buf[base + 4 * v + i]);
                for j in 0..8 {
                    let q = st.ci[j] & !3;
                    let line = &mut st.comb[j];
                    let x = [line[q], line[q + 1], line[q + 2], line[q + 3]];
                    let lp: [f32; 4] = std::array::from_fn(|i| dp4(&taps[3 - i], &x));
                    let fsp = st.fs[j];
                    for i in 0..4 {
                        let t = vadd(vmul(lp[i], self.fb), vmul(self.a28[i], fsp));
                        line[q + i] = vadd(vmul(xin[i], fixed), t);
                    }
                    st.fs[j] = lp[3];
                    for i in 0..4 {
                        acc[i] = vadd(acc[i], x[i]);
                    }
                    st.ci[j] = (st.ci[j] + 4) % COMB[j];
                }
            }
            for (v, sum) in sums.iter().enumerate() {
                let mut y = *sum;
                for j in 0..4 {
                    let q = st.ai[j] & !3;
                    let line = &mut st.ap[j];
                    let b = [line[q], line[q + 1], line[q + 2], line[q + 3]];
                    for i in 0..4 {
                        line[q + i] = vadd(vmul(b[i], 0.5), y[i]);
                    }
                    y = std::array::from_fn(|i| vsub(b[i], y[i]));
                    st.ai[j] = (st.ai[j] + 4) % AP[j];
                }
                for i in 0..4 {
                    let s = base + 4 * v + i;
                    buf[s] = vadd(vmul(y[i], self.wet), vmul(buf[s], self.dry));
                }
            }
            k += 1;
        }
    }
}

/// `audWaveshaperEffectXenon` (section 4): input gain, a 201-entry shaping table over [-1, 1] (or the curve per
/// sample when `direct`), mix with the input, output gain.
pub struct Waveshaper {
    nch: usize,
    mask: u8,
    ing: f32,
    outg: f32,
    mix: f32,
    /// +24: the curve set, taken at the end of the next processed block; +68: the curve in use (by its hash)
    pending: Option<(u32, Curve)>,
    curve: Option<(u32, Curve)>,
    direct: bool,
    bypass: bool,
    table: [f32; 201],
}

impl Waveshaper {
    /// Defaults (F-81855): 0 dB, 0 dB, mix 1, no curve: a hard clip at +-1
    pub fn new(nch: usize, mask: u8) -> Waveshaper {
        let mut w = Waveshaper { nch, mask, ing: db2lin(0.0), outg: db2lin(0.0), mix: 1.0, pending: None, curve: None, direct: false, bypass: false, table: [0.0; 201] };
        w.build_table();
        w
    }

    /// F-32778: table[i] = map(curve, -1, 1, -1, 1, i * 0.01 - 1), or the identity
    fn build_table(&mut self) {
        for i in 0..201 {
            let x = fma32(i as f32, f32::from_bits(0x3C23_D70A), -1.0);
            self.table[i] = match &self.curve {
                Some((_, c)) => {
                    let t = if !(x > -1.0) {
                        0.0
                    } else if !(x < 1.0) {
                        1.0
                    } else {
                        (x + 1.0) / 2.0
                    };
                    fma32(2.0, c.eval(t), -1.0)
                }
                None => x,
            };
        }
    }

    /// SetParameters F-67481: input / output gain dB (clamped to +-96), mix, curve (by hash), direct, bypass
    pub fn set(&mut self, ing: f32, outg: f32, mix: f32, curve: Option<(u32, Curve)>, direct: bool, bypass: u8) {
        let ing = fsel(ing - -96.0, ing, -96.0);
        let ing = fsel(ing - 96.0, 96.0, ing);
        let outg = fsel(outg - -96.0, outg, -96.0);
        let outg = fsel(outg - 96.0, 96.0, outg);
        self.ing = db2lin(ing);
        self.outg = db2lin(outg);
        self.mix = mix;
        self.pending = curve;
        self.direct = direct;
        self.bypass = bypass != 0;
    }

    /// Process F-80723
    pub fn process(&mut self, buf: &mut [f32]) {
        if self.bypass {
            return;
        }
        for ch in 0..self.nch {
            if self.mask >> ch & 1 == 0 {
                continue;
            }
            for v in &mut buf[BLOCK * ch..BLOCK * ch + BLOCK] {
                let mut x = self.ing * *v;
                let y = match (&self.curve, self.direct) {
                    (Some((_, c)), true) => c.eval(x.abs()) * fsel(x, 1.0, -1.0),
                    _ => {
                        x = fsel(x - -1.0, x, -1.0);
                        x = fsel(x - 1.0, 1.0, x);
                        let pos = (x + 1.0) * 100.0;
                        let k = pos as usize;
                        let fr = pos - k as f32;
                        let ta = self.table[k];
                        // (k = 200 reads the word at +880, the direct byte: fr is 0 there)
                        let tb = if k < 200 { self.table[k + 1] } else { f32::from_bits((self.direct as u32) << 24) };
                        fma32(tb - ta, fr, ta)
                    }
                };
                *v = fma32(y - x, self.mix, x) * self.outg;
            }
        }
        if self.pending.as_ref().map(|p| p.0) != self.curve.as_ref().map(|c| c.0) {
            self.curve = self.pending.clone();
            if !self.direct {
                self.build_table();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUS_EFFECTS_RULE.md: the recompiled biquad, reverb and waveshaper run offline (work/audio2/busfx, sequence made
    /// by mkseq.py): every output sample, the filter / comb / allpass states and the waveshaper table, bit for bit.
    /// The biquad's NaN outputs (degenerate settings) only need to be NaN.
    #[test]
    #[ignore = "requires the reference runs in work/audio2/busfx (MCLA_BUSFX_SEQ, default D:/MCLA/work/audio2/busfx/seq.txt)"]
    fn busfx_is_the_originals() {
        let path = std::env::var("MCLA_BUSFX_SEQ").unwrap_or_else(|_| "D:/MCLA/work/audio2/busfx/seq.txt".into());
        let text = std::fs::read_to_string(path).expect("seq.txt");
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let same = |a: u32, b: u32| a == b || (f32::from_bits(a).is_nan() && f32::from_bits(b).is_nan());
        enum Fx {
            B(Biquad),
            R(Reverb),
            W(Waveshaper),
        }
        let mut fx: Option<Fx> = None;
        let mut curve: Option<Curve> = None;
        let mut n = [0usize; 3];
        for (ln, line) in text.lines().enumerate() {
            let w: Vec<&str> = line.split(' ').collect();
            match w[0] {
                "N" => {
                    let (nch, mask) = (w[2].parse().unwrap(), w[3].parse().unwrap());
                    fx = Some(match w[1] {
                        "biquad" => Fx::B(Biquad::new(nch, mask)),
                        "reverb" => Fx::R(Reverb::new(nch, mask)),
                        _ => Fx::W(Waveshaper::new(nch, mask)),
                    });
                }
                "C" => {
                    let b: Vec<u8> = (0..w[1].len() / 2).map(|i| u8::from_str_radix(&w[1][2 * i..2 * i + 2], 16).unwrap()).collect();
                    curve = Some(Curve::parse(&b));
                }
                "S" => {
                    let v: Vec<u32> = w[1..].iter().map(|t| h(t)).collect();
                    let f = |i: usize| f32::from_bits(v[i]);
                    match fx.as_mut().unwrap() {
                        Fx::B(b) => {
                            b.set(f(0), f(1), f(2), f(3), v[4], (v[5] >> 24) as u8);
                            let mine = [b.co[0], b.co[1], b.co[2], b.co[3], b.co[4], b.freq, b.q, b.bw, b.og].map(f32::to_bits);
                            assert!(mine.iter().zip(&v[6..15]).all(|(a, b)| same(*a, *b)), "line {ln}: object {mine:08x?} vs {:08x?}", &v[6..15]);
                        }
                        Fx::R(r) => r.set([f(0), f(1), f(2), f(3)]),
                        Fx::W(ws) => {
                            let c = if v[3] != 0 { curve.clone().map(|c| (v[3], c)) } else { None };
                            ws.set(f(0), f(1), f(2), c, v[4] >> 24 != 0, (v[4] >> 16) as u8);
                        }
                    }
                }
                _ => {
                    let inp: Vec<f32> = w[1].split(',').map(|t| f32::from_bits(h(t))).collect();
                    let want: Vec<u32> = w[2].split(',').map(h).collect();
                    let st: Vec<u32> = w[3..].iter().map(|t| h(t)).collect();
                    let mut buf = inp.clone();
                    let mine_st: Vec<u32> = match fx.as_mut().unwrap() {
                        Fx::B(b) => {
                            b.process(&mut buf);
                            n[0] += 1;
                            (0..4).flat_map(|j| b.st.iter().map(move |s| s[j].to_bits())).collect()
                        }
                        Fx::R(r) => {
                            r.process(&mut buf);
                            n[1] += 1;
                            let mut s: Vec<u32> = [r.fb, r.wet, r.dry, r.damp].map(f32::to_bits).to_vec();
                            for c in &r.ch {
                                s.extend(c.fs.map(f32::to_bits));
                                s.extend(c.ci.map(|i| i as u32));
                                s.extend(c.ai.map(|i| i as u32));
                            }
                            s.extend(r.a28.map(f32::to_bits));
                            s.extend(r.a36.map(f32::to_bits));
                            s
                        }
                        Fx::W(ws) => {
                            ws.process(&mut buf);
                            n[2] += 1;
                            ws.table.map(f32::to_bits).to_vec()
                        }
                    };
                    let bad = buf.iter().zip(&want).filter(|(a, b)| !same(a.to_bits(), **b)).count();
                    assert_eq!(bad, 0, "line {ln}: {bad} samples differ");
                    let st = match fx.as_ref().unwrap() {
                        Fx::W(_) => st[16..217].to_vec(),
                        _ => st,
                    };
                    assert!(mine_st.len() == st.len() && mine_st.iter().zip(&st).all(|(a, b)| same(*a, *b)), "line {ln}: states differ");
                }
            }
        }
        assert_eq!(n, [187, 98, 71]);
    }
}
