//! The sound occlusion of the original (GLOBAL_OCCLUSION, docs/AUDIO.md "Occlusion"): an occlusion group holds a
//! volume and a low-pass cutoff for the sounds of one owner, from an occlusion amount the owner's update measures with
//! two world probes between the listener and the emitter (the material depth between them times the materials' factor).
//! This file has the group's own rule: the tune (F-43021 with F-44963 / F-80129), the group's init F-23776, set F-69008,
//! smoothing F-80830 and latch F-43736, and the owner's update F-87712 with the probes and the material factor as
//! inputs. Bit-exact offline (tools/occlusion_reference.py, test `occlusion_matches_the_original`). Not here yet: the
//! manager's round-robin over the groups, the listener radius test, the probe itself (F-93873) on the port's collision
//! and the owners (car audio, ambience, ...) applying the outputs.

#![allow(dead_code)] // (not wired to the emitters yet)

use crate::busfx::{db2lin, dp4, vadd};
use crate::caraudio::{ln_guest, pow_guest};
use mcla_formats::audcfg::{fsel, Curve};

/// fmadds as the recomp evaluates it: one f64 fma, then f32
fn fma32(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// F-82373 with a curve object: x clamped into [x0, x1] as a 0..1 position, the curve's value there (F-33490; an unset
/// curve reads 0), lo + (hi - lo) * that fused.
fn map(c: Option<&Curve>, lo: f32, hi: f32, x0: f32, x1: f32, x: f32) -> f32 {
    let t = if !(x > x0) {
        0.0
    } else if !(x < x1) {
        1.0
    } else {
        (x - x0) / (x1 - x0)
    };
    fma32(hi - lo, c.map_or(0.0, |c| c.eval(t)), lo)
}

/// F-90102: `p` steps toward `target` by `rate * dt` fused, clamped there; true when it is there.
fn approach(p: &mut f32, target: f32, rate: f32, dt: f32) -> bool {
    if !(*p >= target) {
        *p = fma32(rate, dt, *p);
        if *p > target {
            *p = target;
        }
    } else if *p > target {
        // (fnmsubs: -(rate * dt - p), so a step of 0 from +0 gives -0)
        *p = -((rate as f64).mul_add(dt as f64, -(*p as f64)) as f32);
        if *p < target {
            *p = target;
        }
    }
    *p == target
}

/// The cutoff in the smoothing's log domain: log2(hz / 23900) as F-23776 / F-80830 compute it (the original's ln in
/// double on hz x 1/23900 in single, then x log2 e in single).
fn hz_log(hz: f32) -> f32 {
    (ln_guest((hz * f32::from_bits(0x382F_7E70)) as f64) as f32) * f32::from_bits(0x3FB8_AA3B)
}

/// GLOBAL_OCCLUSION (game.dat) as F-43021 reads it into its two sets: the volume's (+38 the level in mB as dB2lin -
/// the depth where the volume curve starts, +40 where it ends, +48 the volume's rate per second; +44 the curve,
/// OCCLUSION_FACTOR_TO_LINEAR_VOLUME) and the cutoff's (+52 the lowest cutoff in Hz, +54 the depth where the cutoff
/// curve ends, +62 the log-domain rate per second; +58 the curve, OCCLUSION_DEPTH_TO_FILTER).
#[derive(Clone, Debug)]
pub(crate) struct Tune {
    pub vol: [f32; 3],
    pub filt: [f32; 3],
    pub vol_curve: Option<Curve>,
    pub filt_curve: Option<Curve>,
}

impl Tune {
    pub fn read(o: &[u8], curve: impl Fn(u32) -> Option<Curve>) -> Option<Tune> {
        let w = |at: usize| o.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        let f = |at: usize| w(at).map(f32::from_bits);
        let mb = i16::from_be_bytes([*o.get(38)?, *o.get(39)?]) as f32;
        let lowest = u16::from_be_bytes([*o.get(52)?, *o.get(53)?]) as f32;
        Some(Tune {
            vol: [db2lin(mb * f32::from_bits(0x3C23_D70A)), f(40)?, f(48)?],
            filt: [lowest, f(54)?, f(62)?],
            vol_curve: curve(w(44)?),
            filt_curve: curve(w(58)?),
        })
    }

    fn vol_at(&self, x: f32) -> f32 {
        map(self.vol_curve.as_ref(), 0.0, 1.0, self.vol[0], self.vol[1], x)
    }

    fn hz_at(&self, x: f32) -> f32 {
        map(self.filt_curve.as_ref(), self.filt[0], 23900.0, 0.0, self.filt[1], x)
    }
}

/// An occlusion group (+32 the amount last set, +36 / +40 the volume and cutoff out, +44 the cutoff in the log domain,
/// +48 / +52 their targets).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Group {
    pub amount: f32,
    pub vol: f32,
    pub hz: f32,
    pub hz_log: f32,
    pub vol_to: f32,
    pub hz_to: f32,
}

impl Group {
    /// F-23776: the larger of the two curves' end depths as the amount, both outputs at it at once.
    pub fn init(t: &Tune) -> (Group, [f32; 2]) {
        let mut g = Group { amount: fsel(t.vol[1] - t.filt[1], t.vol[1], t.filt[1]), ..Default::default() };
        g.vol = t.vol_at(t.vol[1]);
        g.vol_to = g.vol;
        g.hz = t.hz_at(t.filt[1]);
        g.hz_to = g.hz;
        g.hz_log = hz_log(g.hz);
        (g, [g.vol, g.hz])
    }

    /// F-69008: a new amount sets both targets.
    pub fn set(&mut self, t: &Tune, amount: f32) {
        if amount != self.amount {
            self.vol_to = t.vol_at(amount);
            self.hz_to = t.hz_at(amount);
            self.amount = amount;
        }
    }

    /// F-80830: the volume steps to its target linearly, the cutoff in the log domain (2^ back, x 23900; the target
    /// itself once reached).
    pub fn smooth(&mut self, t: &Tune, dt: f32) -> [f32; 2] {
        if self.vol_to != self.vol {
            approach(&mut self.vol, self.vol_to, t.vol[2], dt);
        }
        if self.hz != self.hz_to {
            if approach(&mut self.hz_log, hz_log(self.hz_to), t.filt[2], dt) {
                self.hz = self.hz_to;
            } else {
                self.hz = (pow_guest(2.0, self.hz_log as f64) as f32) * 23900.0;
            }
        }
        [self.vol, self.hz]
    }

    /// F-43736: straight to the targets.
    pub fn latch(&mut self) -> [f32; 2] {
        self.vol = self.vol_to;
        self.hz = self.hz_to;
        [self.vol, self.hz]
    }
}

/// A probe's hit as F-87712 reads it from the hit table: +16 the position, and the factor (+226) of the material its
/// +72 names.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub pos: [f32; 3],
    pub factor: f32,
}

/// The world probe F-93873 from one point to another with the owner's +76 flags, at most 16 hits.
pub(crate) trait Probe {
    fn probe(&mut self, from: [f32; 4], to: [f32; 4], flags: u32, max: usize) -> Vec<Hit>;
}

/// The owner of an occlusion group as F-87712 has it: +20 the group, +76 the probe flags, +80 the position, +96 the
/// position in the world once worked out, +116 the position is the world's, +117 it is in the frame's space (+118: done
/// once, kept at +96), otherwise it is an offset from the frame's origin.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Owner {
    pub group: Group,
    pub flags: u32,
    pub pos: [f32; 4],
    pub world: [f32; 4],
    pub absolute: bool,
    pub local: bool,
    pub placed: bool,
}

/// The nearest hit with a nonzero factor to `p` by squared distance (strictly nearer than 1e8 m^2 and the ones before).
fn nearest(hits: &[Hit], p: [f32; 3]) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    let mut d_best = 100000000.0f32;
    for h in hits {
        if h.factor == 0.0 {
            continue;
        }
        let (dx, dy, dz) = (h.pos[0] - p[0], h.pos[1] - p[1], h.pos[2] - p[2]);
        let d = fma32(dx, dx, fma32(dz, dz, dy * dy));
        if d < d_best {
            d_best = d;
            best = Some((d, h.factor));
        }
    }
    best
}

impl Owner {
    /// The emitter's position (F-87712's head): `frame` is the owner's frame (F-91345, rows +16 .. +64, the origin last).
    pub fn emitter(&mut self, frame: &[[f32; 4]; 4]) -> [f32; 4] {
        if self.absolute {
            return self.pos;
        }
        if self.local {
            if !self.placed {
                // the position with w = 1 through the frame's columns, x y z, w left as the position's x
                let p = [self.pos[0], self.pos[1], self.pos[2], 1.0];
                let col = |k: usize| [frame[0][k], frame[1][k], frame[2][k], frame[3][k]];
                self.world = [dp4(&p, &col(0)), dp4(&p, &col(1)), dp4(&p, &col(2)), self.pos[0]];
                self.placed = true;
            }
            return self.world;
        }
        std::array::from_fn(|k| vadd(frame[3][k], self.pos[k]))
    }

    /// F-87712: the occlusion amount from the listener `l` to the emitter, set into the group. Probes from the listener
    /// to the emitter and back; of each, the hit nearest its start (nonzero factor). Both: the depth between them
    /// (|E - L| less the two distances) times the larger factor; one: its distance times its factor; none: 0.
    pub fn update(&mut self, t: &Tune, frame: &[[f32; 4]; 4], l: [f32; 4], world: &mut impl Probe) -> f32 {
        let e = self.emitter(frame);
        let from_l = world.probe(l, e, self.flags, 16);
        let near_l = nearest(&from_l, [l[0], l[1], l[2]]);
        let from_e = world.probe(e, l, self.flags, 16);
        let near_e = nearest(&from_e, [e[0], e[1], e[2]]);
        let (dx, dy, dz) = (l[0] - e[0], l[1] - e[1], l[2] - e[2]);
        let amount = match (near_l, near_e) {
            (Some((dl, fl)), Some((de, fe))) => {
                let len = fma32(dx, dx, fma32(dz, dz, dy * dy)).sqrt();
                ((len - dl.sqrt()) - de.sqrt()) * fsel(fl - fe, fl, fe)
            }
            (Some((dl, fl)), None) => dl.sqrt() * fl,
            (None, Some((de, fe))) => de.sqrt() * fe,
            (None, None) => 0.0,
        };
        self.group.set(t, amount);
        amount
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    /// tools/occlusion_reference.py ran the original's code offline: the pow F-21955 on 6,000 inputs, F-33490 on 2,000
    /// random Gauss curves, and on 150 tunes (GLOBAL_OCCLUSION or random) the reader then 20 random group ops each
    /// (init, set, smoothing, latch, the owner's update with random frames, positions and probe hits); every result,
    /// field, output and probe call as the original's
    #[test]
    fn occlusion_matches_the_original() {
        let text = include_str!("testdata/occlusion_ref.txt");
        let hexw = |t: &str| u32::from_str_radix(&t[1..], 16).unwrap();
        let unhex = |s: &str| (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<u8>>();
        let (mut curves, mut factors) = (Vec::new(), [0f32; 8]);
        let (mut tune, mut owner, mut bad, mut n) = (None::<Tune>, Owner::default(), Vec::new(), [0usize; 4]);
        let grp = |s: &mut String, g: &Group| {
            for v in [g.amount, g.vol, g.hz, g.hz_log, g.vol_to, g.hz_to].iter().zip("avhlwz".chars()) {
                write!(s, " {}{:08x}", v.1, v.0.to_bits()).unwrap();
            }
        };
        let outs = |s: &mut String, o: [f32; 2]| write!(s, " o{:08x} o{:08x}", o[0].to_bits(), o[1].to_bits()).unwrap();
        struct World<'a> {
            hits: [Vec<Hit>; 2],
            calls: &'a mut String,
            k: usize,
        }
        impl Probe for World<'_> {
            fn probe(&mut self, from: [f32; 4], to: [f32; 4], flags: u32, max: usize) -> Vec<Hit> {
                *self.calls += " Q";
                for v in from.iter().chain(to.iter()) {
                    write!(self.calls, " p{:08x}", v.to_bits()).unwrap();
                }
                write!(self.calls, " g{flags} n{max}").unwrap();
                self.k += 1;
                self.hits[self.k - 1].clone()
            }
        }
        for line in text.lines() {
            let t: Vec<&str> = line.split(' ').collect();
            let mut s = String::new();
            match t[0] {
                "C" => curves = vec![(hexw(t[1]), Curve::parse(&unhex(&t[2][1..]))), (hexw(t[3]), Curve::parse(&unhex(&t[4][1..])))],
                "F" => factors = std::array::from_fn(|k| f32::from_bits(hexw(t[1 + k]))),
                "P" => {
                    let w = |k: usize| f64::from_bits(u64::from_str_radix(&t[k][1..], 16).unwrap());
                    n[0] += 1;
                    write!(s, "P {} {} r{:016x}", t[1], t[2], pow_guest(w(1), w(2)).to_bits()).unwrap();
                }
                "G" => {
                    let mut o = vec![9u8; 1];
                    o.extend([0u8; 4]);
                    o.extend(hexw(t[2]).to_be_bytes());
                    o.extend([0u8; 4]);
                    for k in [3, 4, 1] {
                        o.extend(hexw(t[k]).to_be_bytes());
                    }
                    o.extend([0u8; 8]);
                    n[1] += 1;
                    let r = Curve::parse(&o).eval(f32::from_bits(hexw(t[5])));
                    write!(s, "G {} {} {} {} {} r{:08x}", t[1], t[2], t[3], t[4], t[5], r.to_bits()).unwrap();
                }
                "T" => {
                    let mut o = vec![0u8; 66];
                    o[38..40].copy_from_slice(&(t[1][1..].parse::<u32>().unwrap() as u16).to_be_bytes());
                    o[40..44].copy_from_slice(&hexw(t[2]).to_be_bytes());
                    o[44..48].copy_from_slice(&curves[0].0.to_be_bytes());
                    o[48..52].copy_from_slice(&hexw(t[3]).to_be_bytes());
                    o[52..54].copy_from_slice(&(t[4][1..].parse::<u32>().unwrap() as u16).to_be_bytes());
                    o[54..58].copy_from_slice(&hexw(t[5]).to_be_bytes());
                    o[58..62].copy_from_slice(&curves[1].0.to_be_bytes());
                    o[62..66].copy_from_slice(&hexw(t[6]).to_be_bytes());
                    let tn = Tune::read(&o, |h| curves.iter().find(|c| c.0 == h).map(|c| c.1.clone())).unwrap();
                    s = t[..7].join(" ");
                    for v in tn.vol.iter().chain(tn.filt.iter()) {
                        write!(s, " s{:08x}", v.to_bits()).unwrap();
                    }
                    tune = Some(tn);
                    owner = Owner::default();
                    n[2] += 1;
                }
                op => {
                    let tn = tune.as_ref().unwrap();
                    n[3] += 1;
                    match op {
                        "I" => {
                            let (g, o) = Group::init(tn);
                            owner.group = g;
                            s += "I";
                            grp(&mut s, &owner.group);
                            outs(&mut s, o);
                        }
                        "S" => {
                            owner.group.set(tn, f32::from_bits(hexw(t[1])));
                            s = format!("S {}", t[1]);
                            grp(&mut s, &owner.group);
                        }
                        "M" => {
                            let o = owner.group.smooth(tn, f32::from_bits(hexw(t[1])));
                            s = format!("M {}", t[1]);
                            grp(&mut s, &owner.group);
                            outs(&mut s, o);
                        }
                        "L" => {
                            let o = owner.group.latch();
                            s += "L";
                            grp(&mut s, &owner.group);
                            outs(&mut s, o);
                        }
                        _ => {
                            let g: Vec<u32> = t[1..83].iter().map(|v| hexw(v)).collect();
                            let f = |k: usize| f32::from_bits(g[k]);
                            (owner.absolute, owner.local, owner.placed) = (g[0] != 0, g[1] != 0, g[2] != 0);
                            owner.pos = std::array::from_fn(|k| f(3 + k));
                            owner.world = std::array::from_fn(|k| f(7 + k));
                            let frame: [[f32; 4]; 4] = std::array::from_fn(|r| std::array::from_fn(|k| f(11 + 4 * r + k)));
                            let l = std::array::from_fn(|k| f(27 + k));
                            owner.flags = g[31];
                            let hits = |at: usize| (0..g[at] as usize).map(|k| Hit { pos: std::array::from_fn(|c| f(at + 1 + 4 * k + c)), factor: factors[(g[at + 4 + 4 * k] & 7) as usize] }).collect();
                            s = t[..83].join(" ");
                            let mut world = World { hits: [hits(32), hits(57)], calls: &mut s, k: 0 };
                            owner.update(tn, &frame, l, &mut world);
                            for v in owner.world {
                                write!(s, " W{:08x}", v.to_bits()).unwrap();
                            }
                            write!(s, " b{}", owner.placed as u8).unwrap();
                            grp(&mut s, &owner.group);
                        }
                    }
                }
            }
            if !s.is_empty() && s != line {
                bad.push(format!("want {line}\n got {s}"));
            }
        }
        assert_eq!(n, [6000, 2000, 150, 3000], "the reference's counts");
        assert!(bad.is_empty(), "{} lines differ, first:\n{}", bad.len(), bad[..bad.len().min(6)].join("\n"));
    }
}
