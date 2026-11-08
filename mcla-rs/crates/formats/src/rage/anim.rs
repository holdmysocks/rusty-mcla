//! RAGE `crAnimation` objects as embedded in ambient pedestrian resources
//! (`resources/ambients/<set>/*.r189`, `resources/ambients/*.r198`).
//!
//! Animation (0x30 bytes): `+04 u16 flags (bit 0 = looping), +08 u16 frame count,
//! +0c f32 duration, +14 -> block pointers, +18 u16 block count, +1c -> name
//! ("anim/Ped/Male/Mped_run.anim"), +20 -> track descriptors, +24 u16 track count`.
//! Block: `+04 -> track pointers, +08 u16 count, +10 u16 frames`. Track (0x18 bytes):
//! `u8 kind (0 bone translation, 1 bone rotation, 5 mover translation, 6 mover
//! rotation), u8 flags, u16 bone id, 4 channel pointers, u32 channel count`.
//! Channels: byte +5 is the type: 4 static float (+8), 6 quantised floats (+8 -> bit
//! stream, +0c bits per value, +10 count, +14 scale, +18 offset; values are packed
//! LSB first inside big-endian 32-bit words), 9 static quaternion (+8 -> xyzw),
//! 0x0d static vector (+8 -> xyz), 1 raw floats (+8 -> f32 list, +0c u16 count; the Beaches garage
//! scene's camera x, whose values start at the set-up's wait camera -3059.59).
//!
//! Long animations (cutscenes) are cut into blocks of 128 frames; the last frame of a
//! block is the first of the next. Track kind 10 is a camera's field of view (degrees,
//! one float channel).

use super::Rd;
use crate::rsc::Resource;
use glam::{Quat, Vec3};

#[derive(Debug, Clone)]
pub enum TrackData {
    Vec(Vec<Vec3>),
    Rot(Vec<Quat>),
}

/// One channel as the original's sampler reads it (F-12216 and the per-channel evaluators): type 4 one float, type 1 a float
/// per frame, type 6 the quantised integers with their scale and offset (the value is fma(q, scale, offset)), types 9 / 13
/// one 16-byte value (a quaternion, a vector and its fourth word).
#[derive(Debug, Clone)]
pub enum Chan {
    Static(f32),
    Raw(Vec<f32>),
    Quant(Vec<u32>, f32, f32),
    Static4([f32; 4]),
}

#[derive(Debug, Clone)]
pub struct Track {
    pub kind: u8,
    pub bone_id: u16,
    pub data: TrackData,
    /// the track's byte +1 (its high nibble 0x10: one channel per component)
    pub flags: u8,
    /// the channels (of the first block; empty for a clip of several blocks)
    pub chans: Vec<Chan>,
}

#[derive(Debug, Clone, Default)]
pub struct Animation {
    pub name: String,
    pub frames: usize,
    pub duration: f32,
    pub looped: bool,
    pub tracks: Vec<Track>,
    /// +0a frames per block, the block count (+18) and the first block's frame count (+10 of the block)
    pub block_len: u16,
    pub blocks: u16,
    pub block0_frames: u16,
}

pub const KIND_TRANSLATION: u8 = 0;
pub const KIND_ROTATION: u8 = 1;
pub const KIND_MOVER_TRANSLATION: u8 = 5;
pub const KIND_MOVER_ROTATION: u8 = 6;
pub const KIND_FOV: u8 = 10;

fn bits(rd: &Rd, p: usize, bit: usize, n: usize) -> u32 {
    let w = bit >> 5;
    let s = bit & 31;
    let lo = rd.u32(p + w * 4) as u64;
    let hi = if s + n > 32 { rd.u32(p + w * 4 + 4) as u64 } else { 0 };
    (((lo | (hi << 32)) >> s) & ((1u64 << n) - 1)) as u32
}

/// a x b + c, one rounding to single (the scalar fmadds).
fn fma(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// A quantised channel's value: fma(q, scale, offset), q converted exactly (F-89511).
pub fn dequant(q: u32, sc: f32, off: f32) -> f32 {
    fma(q as f32, sc, off)
}

/// A float channel between keys a and b at fraction fr: fma(b - a, fr, a) (F-18785, F-74386).
pub fn lerp_key(a: f32, b: f32, fr: f32) -> f32 {
    fma(b - a, fr, a)
}

/// A quaternion of one channel per component normalised as the sampler does (F-12216): the squares in the order
/// z y w x, the estimate 1 / sqrt in single with two Newton steps in double, 0 when the squared length is not above 0.
pub fn normalise4([x, y, z, w]: [f32; 4]) -> [f32; 4] {
    let n2 = fma(z, z, fma(y, y, fma(w, w, x * x)));
    let r = if -(n2 as f64) >= 0.0 {
        0.0
    } else {
        let n = n2 as f64;
        let mut e = (1.0 / n.sqrt()) as f32 as f64;
        for _ in 0..2 {
            let d = -(e * e).mul_add(n, -1.0);
            e = d.mul_add(e * 0.5, e);
        }
        e as f32
    };
    [r * x, r * y, r * z, r * w]
}

fn channel(rd: &Rd, ch: usize, frames: usize) -> Option<Vec<f32>> {
    match rd.u8(ch + 5) {
        4 => Some(vec![rd.f32(ch + 8)]),
        1 => {
            let p = rd.ptr(ch + 8)?;
            let cnt = (rd.u16(ch + 0x0c) as usize).min(frames.max(1));
            if cnt == 0 || !rd.has(p, cnt * 4) {
                return None;
            }
            Some((0..cnt).map(|i| rd.f32(p + i * 4)).collect())
        }
        6 => {
            let p = rd.ptr(ch + 8)?;
            let nb = rd.u32(ch + 0x0c) as usize;
            let cnt = (rd.u32(ch + 0x10) as usize).min(frames.max(1));
            let (sc, off) = (rd.f32(ch + 0x14), rd.f32(ch + 0x18));
            if nb == 0 || nb > 32 || !rd.has(p, (cnt * nb + 31) / 32 * 4) {
                return None;
            }
            Some((0..cnt).map(|i| dequant(bits(rd, p, i * nb, nb), sc, off)).collect())
        }
        _ => None,
    }
}

fn chan(rd: &Rd, ch: usize) -> Option<Chan> {
    Some(match rd.u8(ch + 5) {
        4 => Chan::Static(rd.f32(ch + 8)),
        1 => {
            let p = rd.ptr(ch + 8)?;
            let cnt = rd.u16(ch + 0x0c) as usize;
            if !rd.has(p, cnt * 4) {
                return None;
            }
            Chan::Raw((0..cnt).map(|i| rd.f32(p + i * 4)).collect())
        }
        6 => {
            let p = rd.ptr(ch + 8)?;
            let nb = rd.u32(ch + 0x0c) as usize;
            let cnt = rd.u32(ch + 0x10) as usize;
            if nb == 0 || nb > 32 || !rd.has(p, (cnt * nb + 31) / 32 * 4) {
                return None;
            }
            Chan::Quant((0..cnt).map(|i| bits(rd, p, i * nb, nb)).collect(), rd.f32(ch + 0x14), rd.f32(ch + 0x18))
        }
        9 | 0x0d => {
            let p = rd.ptr(ch + 8)?;
            Chan::Static4([rd.f32(p), rd.f32(p + 4), rd.f32(p + 8), rd.f32(p + 12)])
        }
        _ => return None,
    })
}

fn track(rd: &Rd, tr: usize, frames: usize) -> Option<Track> {
    let kind = rd.u8(tr);
    let bone_id = rd.u16(tr + 2);
    let n = rd.u32(tr + 0x14) as usize;
    let c0 = rd.ptr(tr + 4)?;
    let rot = matches!(kind, KIND_ROTATION | KIND_MOVER_ROTATION);
    let data = if n == 1 && matches!(rd.u8(c0 + 5), 4 | 6) {
        // a single float (field of view): kept in x
        TrackData::Vec(channel(rd, c0, frames)?.into_iter().map(|v| Vec3::new(v, 0.0, 0.0)).collect())
    } else if n == 1 {
        let p = rd.ptr(c0 + 8)?;
        match rd.u8(c0 + 5) {
            // the 16 bytes as they are (the sampler copies them)
            9 => TrackData::Rot(vec![Quat::from_xyzw(rd.f32(p), rd.f32(p + 4), rd.f32(p + 8), rd.f32(p + 12))]),
            0x0d => TrackData::Vec(vec![Vec3::new(rd.f32(p), rd.f32(p + 4), rd.f32(p + 8))]),
            _ => return None,
        }
    } else {
        let mut ch = Vec::with_capacity(n);
        for k in 0..n.min(4) {
            ch.push(channel(rd, rd.ptr(tr + 4 + k * 4)?, frames)?);
        }
        let len = ch.iter().map(|c| c.len()).max().unwrap_or(1);
        let at = |k: usize, i: usize| -> f32 { ch[k][i.min(ch[k].len() - 1)] };
        if rot && n == 4 {
            // the channels' keys as they are: the sampler lerps each channel, then normalises
            TrackData::Rot((0..len).map(|i| Quat::from_xyzw(at(0, i), at(1, i), at(2, i), at(3, i))).collect())
        } else if n == 3 {
            TrackData::Vec((0..len).map(|i| Vec3::new(at(0, i), at(1, i), at(2, i))).collect())
        } else {
            return None;
        }
    };
    let chans = (0..n.min(4)).filter_map(|k| chan(rd, rd.ptr(tr + 4 + k * 4)?)).collect();
    Some(Track { kind, bone_id, data, flags: rd.u8(tr + 1), chans })
}

/// Parse the animation object at `o` (found by scanning: the name has to be a path).
pub fn animation_at(rd: &Rd, o: usize) -> Option<Animation> {
    let name = rd.cstr(rd.ptr(o + 0x1c)?);
    if name.len() < 6 || !name.as_bytes()[..5].eq_ignore_ascii_case(b"anim/") {
        return None;
    }
    animation_named(rd, o)
}

/// Animation packs (`resources/animation/**.r2`): the root is a vtable followed by
/// pointers to the animations. Returns (offset, name) of each.
pub fn pack_entries(res: &Resource) -> Vec<(usize, String)> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let mut o = 4;
    while o + 4 <= res.virtual_size {
        let Some(a) = rd.ptr(o) else { break };
        o += 4;
        if let Some(n) = rd.ptr(a + 0x1c) {
            let name = rd.cstr(n);
            if !name.is_empty() && (1..=64).contains(&rd.u16(a + 0x18)) {
                out.push((a, name));
            }
        }
    }
    out
}

/// Parse the animation object at `o`, whatever it is called.
pub fn animation_named(rd: &Rd, o: usize) -> Option<Animation> {
    let np = rd.ptr(o + 0x1c)?;
    let name = rd.cstr(np);
    let frames = rd.u16(o + 8) as usize;
    let duration = rd.f32(o + 0x0c);
    let nblk = rd.u16(o + 0x18) as usize;
    if frames == 0 || frames > 8000 || !(duration > 0.0 && duration < 300.0) || nblk == 0 || nblk > 64 {
        return None;
    }
    let bl = rd.ptr(o + 0x14)?;
    let mut tracks: Vec<Track> = Vec::new();
    let mut block0_frames = 0;
    for b in 0..nblk {
        let blk = rd.ptr(bl + b * 4)?;
        let ta = rd.ptr(blk + 4)?;
        let nt = rd.u16(blk + 8) as usize;
        if nt == 0 || nt > 1024 || !rd.has(ta, nt * 4) {
            return None;
        }
        let bf = match rd.u16(blk + 0x10) as usize {
            0 => frames,
            n => n,
        };
        if b == 0 {
            block0_frames = rd.u16(blk + 0x10);
        }
        let cur: Vec<Track> = (0..nt).filter_map(|t| track(rd, rd.ptr(ta + t * 4)?, bf)).collect();
        if nblk == 1 {
            tracks = cur;
            break;
        }
        // several blocks: every track spelled out per frame, blocks joined at the shared frame
        let skip = if b == 0 { 0 } else { 1 };
        if b == 0 {
            tracks = cur.iter().map(|t| Track { kind: t.kind, bone_id: t.bone_id, flags: t.flags, chans: Vec::new(), data: match t.data { TrackData::Vec(_) => TrackData::Vec(Vec::new()), TrackData::Rot(_) => TrackData::Rot(Vec::new()) } }).collect();
        }
        for t in tracks.iter_mut() {
            let src = cur.iter().find(|c| c.kind == t.kind && c.bone_id == t.bone_id);
            match (&mut t.data, src.map(|c| &c.data)) {
                (TrackData::Vec(out), Some(TrackData::Vec(v))) if !v.is_empty() => out.extend((skip..bf).map(|i| v[i.min(v.len() - 1)])),
                (TrackData::Rot(out), Some(TrackData::Rot(v))) if !v.is_empty() => out.extend((skip..bf).map(|i| v[i.min(v.len() - 1)])),
                (TrackData::Vec(out), _) => {
                    let last = out.last().copied().unwrap_or(Vec3::ZERO);
                    out.extend((skip..bf).map(|_| last));
                }
                (TrackData::Rot(out), _) => {
                    let last = out.last().copied().unwrap_or(Quat::IDENTITY);
                    out.extend((skip..bf).map(|_| last));
                }
            }
        }
    }
    Some(Animation { name, frames, duration, looped: rd.u16(o + 4) & 1 != 0, tracks, block_len: rd.u16(o + 0x0a), blocks: nblk as u16, block0_frames })
}

/// The animation called exactly `name` inside a resource whose animations carry plain names
/// (a vehicle's base fragment `.r63`: `win_l_open`, `win_r_open`, `door_l_open`, `tach` ...),
/// found by the name pointer at its `+1c`.
pub fn animation_by_name(res: &Resource, name: &str) -> Option<Animation> {
    let rd = Rd::new(res);
    let mut o = 0x1c;
    while o + 0x14 + 4 <= res.virtual_size {
        if let Some(np) = rd.ptr(o) {
            if np + name.len() < res.virtual_size && rd.u8(np + name.len()) == 0 && rd.cstr(np) == name {
                if let Some(a) = animation_named(&rd, o - 0x1c) {
                    return Some(a);
                }
            }
        }
        o += 4;
    }
    None
}

/// Every animation embedded in a resource (found by scanning the virtual segment).
pub fn find_animations(res: &Resource) -> Vec<Animation> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let mut o = 0;
    while o + 0x30 <= res.virtual_size {
        if let Some(a) = animation_at(&rd, o) {
            out.push(a);
            o += 0x30;
        } else {
            o += 4;
        }
    }
    out
}

impl Animation {
    /// Short name: "anim/Ped/Male/Mped_run.anim" -> "mped_run".
    pub fn short(&self) -> String {
        let s = self.name.rsplit('/').next().unwrap_or("");
        s.trim_end_matches(".anim").to_ascii_lowercase()
    }

    pub fn track(&self, kind: u8, bone_id: u16) -> Option<&Track> {
        self.tracks.iter().find(|t| t.kind == kind && t.bone_id == bone_id)
    }

    /// Frame position (fractional) for a time, wrapping or clamping.
    pub fn frame_at(&self, time: f32) -> f32 {
        let last = (self.frames.max(1) - 1) as f32;
        if self.duration <= 0.0 || last <= 0.0 {
            return 0.0;
        }
        let f = time / self.duration * last;
        if self.looped { f.rem_euclid(last) } else { f.clamp(0.0, last) }
    }
}

impl TrackData {
    /// The value at frame position `f`: key i = floor(f) (at most the last), fraction f - i.
    pub fn vec(&self, f: f32) -> Vec3 {
        let i = f as usize;
        self.vec_at(i, f - i as f32)
    }
    /// The value between keys i and i + 1 at fraction fr, each component as the float channel's evaluator
    /// (fma(b - a, fr, a)); one key: that key.
    pub fn vec_at(&self, i: usize, fr: f32) -> Vec3 {
        match self {
            TrackData::Vec(v) if v.len() == 1 => v[0],
            TrackData::Vec(v) if !v.is_empty() => {
                let i = i.min(v.len() - 1);
                let (a, b) = (v[i], v[(i + 1).min(v.len() - 1)]);
                Vec3::new(lerp_key(a.x, b.x, fr), lerp_key(a.y, b.y, fr), lerp_key(a.z, b.z, fr))
            }
            _ => Vec3::ZERO,
        }
    }
    pub fn rot(&self, f: f32) -> Quat {
        let i = f as usize;
        self.rot_at(i, f - i as f32)
    }
    /// The rotation between keys i and i + 1 at fraction fr: each of the four channels lerped as a float channel,
    /// then normalised (normalise4); no sign flip (F-12216). One key (a static quaternion): its 16 bytes.
    pub fn rot_at(&self, i: usize, fr: f32) -> Quat {
        match self {
            TrackData::Rot(v) if v.len() == 1 => v[0],
            TrackData::Rot(v) if !v.is_empty() => {
                let i = i.min(v.len() - 1);
                let (a, b) = (v[i].to_array(), v[(i + 1).min(v.len() - 1)].to_array());
                Quat::from_array(normalise4(std::array::from_fn(|k| lerp_key(a[k], b[k], fr))))
            }
            _ => Quat::IDENTITY,
        }
    }
}

#[cfg(test)]
mod tests {
    /// The float-channel evaluators against the original's (tools/anim_channel_reference.py, 6,000 random channels): a
    /// quantised value of frame i clamped to the count (F-89511), a quantised and a raw channel lerped between keys i and
    /// i + 1 (F-74386, F-18785); every result's bits equal.
    #[test]
    fn anim_channels_match_the_original() {
        use super::{bits, dequant, Rd, TrackData};
        let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let mut n = 0;
        for line in include_str!("testdata/anim_channel_ref.txt").lines() {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let t: Vec<u32> = inp.split(' ').map(h).collect();
            let (kind, nb, count, sc, off, i, fr) =
                (t[0], t[1] as usize, t[2] as usize, f32::from_bits(t[3]), f32::from_bits(t[4]), t[5] as usize, f32::from_bits(t[6]));
            let words = &t[8..8 + t[7] as usize];
            let res = crate::rsc::Resource {
                rtype: 0,
                flags: 0,
                virtual_size: words.len() * 4,
                physical_size: 0,
                data: words.iter().flat_map(|w| w.to_be_bytes()).collect(),
            };
            let rd = Rd::new(&res);
            let q = |k: usize| dequant(bits(&rd, 0, k * nb, nb), sc, off);
            let got = match kind {
                0 => q(i.min(count - 1)),
                1 => TrackData::Vec((0..count).map(|k| glam::Vec3::splat(q(k))).collect()).vec_at(i, fr).x,
                _ => TrackData::Vec(words[..count].iter().map(|w| glam::Vec3::splat(f32::from_bits(*w))).collect()).vec_at(i, fr).x,
            };
            let want = h(out);
            assert!(got.to_bits() == want || (got.is_nan() && f32::from_bits(want).is_nan()), "{line}: {:08x}", got.to_bits());
            n += 1;
        }
        assert_eq!(n, 6000);
    }

    /// The side windows' clips of a car's base fragment (DB9): 21 frames over 0.667 s, a
    /// translation and a rotation track on the window bone (tag 0x4137 left, 0xd554 right),
    /// starting at the bone's rest place; the glass ends 0.32 m lower.
    #[test]
    fn db9_window_clips() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/vehicle/vpd_am_db9_08/0x0de3ddb3.r63")) else { return };
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        let res = crate::rsc::Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data };
        let sk = super::super::skeleton::Skeleton::parse(&res).unwrap().unwrap();
        for (name, tag) in [("win_l_open", 0x4137u16), ("win_r_open", 0xd554)] {
            let a = super::animation_by_name(&res, name).unwrap();
            assert_eq!((a.frames, (a.duration * 1000.0).round() as i32, a.tracks.len()), (21, 667, 2));
            assert!(a.tracks.iter().all(|t| t.bone_id == tag));
            let t = a.track(super::KIND_TRANSLATION, tag).unwrap();
            let b = &sk.bones[sk.bone_by_tag(tag).unwrap()];
            assert!((t.data.vec(0.0) - b.local.w_axis.truncate()).length() < 1e-3);
            assert!((t.data.vec(0.0).y - t.data.vec(20.0).y - 0.320).abs() < 0.002);
            let r0 = a.track(super::KIND_ROTATION, tag).unwrap().data.rot(0.0);
            assert!(r0.angle_between(glam::Quat::from_mat4(&b.local)) < 1e-3);
        }
    }
}
