//! The plant impostors' decisions as the original makes them, without the GPU: the tuning table, which types get pictures and how big,
//! the twins, the per-frame schedule, the shadow impostors' round robin, the card's bindings and the wind vector. Each rule is
//! checked against the original's code run offline (`tools/map_impostor_reference.py`, `testdata/map_impostor_ref.txt`).
//!
//! Picture rendering itself (the targets' formats, the multisampled passes) is the GPU's business and not compared here.

/// The suffix the loader cuts from a type's name to find its drawable; the mark of a twin (F-95538).
pub const LOW_LOD_SUFFIX: &str = "_displayAsLowLOD";

/// One item of `tune/MultiPropTuning.xml`: the type's name, MaxRots (item +4) and ScheduledFrame (item +8).
#[derive(Clone, Debug, PartialEq)]
pub struct TuneItem {
    pub name: String,
    pub max_rots: i32,
    pub sched: u32,
}

/// The items of the file in its order (an item missing a field is dropped; the shipped file has all three in all 29).
pub fn parse_tuning(text: &str) -> Vec<TuneItem> {
    let attr = |item: &str, tag: &str| item.split(tag).nth(1).and_then(|t| t.split('"').nth(1)).and_then(|v| v.trim().parse::<i64>().ok());
    let mut out = Vec::new();
    for item in text.split("<Item").skip(1) {
        let name = item.split("<Name").nth(1).and_then(|t| t.split_once('>')).and_then(|t| t.1.split('<').next()).map(|n| n.trim().to_string());
        if let (Some(name), Some(m), Some(s)) = (name, attr(item, "<MaxRots"), attr(item, "<ScheduledFrame")) {
            out.push(TuneItem { name, max_rots: m as i32, sched: s as u32 });
        }
    }
    out
}

/// F-29337: a type's (MaxRots, ScheduledFrame). The first item whose name equals the type's, ignoring case (a C `stricmp`); not
/// found: ScheduledFrame 3, and MaxRots 250 where the name holds `_gfx_hedge_`, else 1.
pub fn tuning_lookup(items: &[TuneItem], name: &str) -> (i32, u32) {
    match items.iter().find(|i| i.name.eq_ignore_ascii_case(name)) {
        Some(i) => (i.max_rots, i.sched),
        None => (if name.contains("_gfx_hedge_") { 250 } else { 1 }, 3),
    }
}

/// The pictures a type owns (F-28056): the shadow impostor's side, ImpostorColor's and ImpostorNormal's width x height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PicturePlan {
    pub shadow: u32,
    pub color: (u32, u32),
    pub normal: (u32, u32),
}

/// Side of the shared ImpostorDepth target (created with the first type's pictures).
pub const DEPTH_SIDE: u32 = 256;

/// F-28056 for a type with a drawable: none where MaxRots (signed) is 250 or more, or the type is a twin (it takes its base's);
/// otherwise ShadowImpostor is 256 x 256 where the drawable's bounding-sphere radius is over 2.0, else 128 x 128, and the colour and
/// normal pictures are MaxRots x 128 wide (the count shifted left 7, the low 7 bits cleared) and 128 high.
pub fn picture_plan(max_rots: i32, low: bool, radius: f32) -> Option<PicturePlan> {
    if max_rots >= 250 || low {
        return None;
    }
    let w = (max_rots as u32).rotate_left(7) & 0xFFFF_FF80;
    Some(PicturePlan { shadow: if radius > 2.0 { 256 } else { 128 }, color: (w, 128), normal: (w, 128) })
}

/// F-64555's second half: each twin (the low-LOD mark set) is linked to the first type, in array order, that is not a twin and whose name
/// equals the twin's name cut at the suffix (compared byte for byte, as `strcmp`; the copy of the name holds at most 255 bytes).
pub fn link_twins(names: &[&str], low: &[bool]) -> Vec<Option<usize>> {
    (0..names.len())
        .map(|i| {
            if !low[i] {
                return None;
            }
            let bytes = &names[i].as_bytes()[..names[i].len().min(255)];
            let cut = match find_sub(bytes, LOW_LOD_SUFFIX.as_bytes()) {
                Some(p) => &bytes[..p],
                None => bytes,
            };
            (0..names.len()).find(|&j| !low[j] && names[j].as_bytes() == cut)
        })
        .collect()
}

fn find_sub(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

/// What the loader knows about one record of the type array: used for the per-frame schedule.
#[derive(Clone, Debug)]
pub struct SlotState {
    /// +168: the drawable
    pub has_drawable: bool,
    /// +196: the pictures (a twin has its base's)
    pub has_pictures: bool,
    /// +218: a twin
    pub low: bool,
    /// +144
    pub max_rots: i32,
    /// +152
    pub sched: u32,
    /// +217: the day / night byte the pictures were last drawn for
    pub last_night: u8,
    /// +212: the base type of a twin
    pub twin: Option<usize>,
    /// +156: the frame counter value of the last card draw (a twin's draws stamp its base)
    pub stamp: u32,
}

/// F-65228: the type is walked this frame when its cards were drawn last frame or this frame - its stamp (or its base's, for a type
/// with one) equals the counter or the counter minus one, compared as signed words.
pub fn in_view(stamp: u32, counter: u32) -> bool {
    let (s, c) = (stamp as i32, counter as i32);
    s == c.wrapping_sub(1) || s == c
}

/// F-27437: the frame bits a type's ScheduledFrame is tested against. `quad` (the renderer's byte +435): one of four frames,
/// `1 << (counter & 3)`; otherwise the bit of the frame's parity and the one two above it (5 on even frames, 10 on odd).
pub fn due_mask(counter: u32, quad: bool) -> u32 {
    if quad {
        1 << (counter & 3)
    } else {
        let b = 1u32 << (counter & 1);
        (b << 2) | b
    }
}

/// F-27437's test: the pictures are drawn again when ScheduledFrame has a bit of the frame's mask, or the day / night byte changed.
pub fn is_due(sched: u32, counter: u32, quad: bool, last_night: u8, night: u8) -> bool {
    sched & due_mask(counter, quad) != 0 || last_night != night
}

/// F-65228 with F-27437 for one frame: the (type, rotations drawn) in order, and the types' last-night byte updated. A type is skipped
/// unless it is not a twin, was seen (`in_view`), has its drawable and pictures, and is due; a drawn type draws one tile per rotation
/// (MaxRots of them, none when MaxRots is not positive).
pub fn frame_plan(slots: &mut [SlotState], counter: u32, quad: bool, night: u8) -> Vec<(usize, i32)> {
    let mut out = Vec::new();
    for i in 0..slots.len() {
        if slots[i].low {
            continue;
        }
        let stamp = match slots[i].twin {
            Some(b) => slots[b].stamp,
            None => slots[i].stamp,
        };
        if !in_view(stamp, counter) {
            continue;
        }
        let s = &mut slots[i];
        if !s.has_pictures || !s.has_drawable {
            continue;
        }
        if !is_due(s.sched, counter, quad, s.last_night, night) {
            continue;
        }
        s.last_night = night;
        if s.max_rots > 0 {
            out.push((i, s.max_rots));
        }
    }
    out
}

/// What one record does in the shadow impostors' round (F-61167).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visit {
    /// the silhouette is drawn
    Draw,
    /// a twin: only the card vectors are worked out
    Vectors,
}

/// F-80686: the shadow impostors' regeneration. The first call (the hour before it is -100) and any call whose hour is 1/120 h or more
/// from the last regenerates every record that has a drawable and pictures; a call within that of the last regenerates the next record
/// that has both (the index steps first, signed, modulo the count). The hour is not wrapped at 24.
#[derive(Clone, Debug)]
pub struct ShadowRound {
    pub index: i32,
    pub last_hour: f32,
}

impl Default for ShadowRound {
    fn default() -> Self {
        ShadowRound { index: 0, last_hour: -100.0 }
    }
}

impl ShadowRound {
    /// `slots`: per record (has drawable, has pictures, twin). The records visited, in order.
    pub fn turn(&mut self, slots: &[(bool, bool, bool)], hour: f32) -> Vec<(usize, Visit)> {
        let n = slots.len();
        let mut out = Vec::new();
        if n == 0 {
            return out;
        }
        let visit = |i: usize| if slots[i].2 { Visit::Vectors } else { Visit::Draw };
        // (a NaN hour fails the `less than` test too, so it takes the everything branch)
        if !((self.last_hour - hour).abs() < 1.0 / 120.0) {
            for (i, s) in slots.iter().enumerate() {
                if s.0 && s.1 {
                    out.push((i, visit(i)));
                }
            }
        } else {
            // (the original searches without a bound; with no qualifying record it would never return)
            for _ in 0..n {
                self.index = self.index.wrapping_add(1) % n as i32;
                let i = self.index as usize;
                if slots[i].0 && slots[i].1 {
                    out.push((i, visit(i)));
                    break;
                }
            }
        }
        self.last_hour = hour;
        out
    }
}

/// F-48104: what a card's draw binds. `shadow` (the byte of the argument): ShadowImpostor as the diffuse picture and none as the normal
/// one; otherwise ImpostorColor and ImpostorNormal.
pub fn card_pictures<T: Copy>(shadow: u32, color: T, normal: T, shadow_pic: T, none: T) -> (T, T) {
    if shadow & 0xFF != 0 {
        (shadow_pic, none)
    } else {
        (color, normal)
    }
}

/// gWindVector as F-65228 builds it each frame (the clock `t` is the time block's word at +24): lane k is `amp * sin(2 pi frac(t / P))`
/// for the periods P = 4, 4, 5 and the amplitude 0.075, lane w the constant 0.4. The original's vector code: the reciprocal of
/// the period (an estimate and one Newton step: 0.25 and 0.2 rounded to single, what the offline run gives), the fraction, the angle
/// brought to [-pi/2, pi/2] by the nearest multiple of pi (k) and a Taylor polynomial of the sine in single precision (no fused
/// multiply-add), negated for odd k as `0 - x`.
pub fn wind_vector(t: f32) -> [f32; 4] {
    const AMP: f32 = 0.075;
    const TWO_PI: f32 = f32::from_bits(0x40c9_0fdb);
    const INV_PI: f32 = f32::from_bits(0x3ea2_f983);
    const NEG_PI: f32 = f32::from_bits(0xc049_0fdb);
    const C9: f32 = f32::from_bits(0x3638_ef1d);
    const C7: f32 = f32::from_bits(0xb950_0d01);
    const C5: f32 = f32::from_bits(0x3c08_8889);
    const C3: f32 = f32::from_bits(0xbe2a_aaab);
    let lane = |recip: f32| -> f32 {
        let x = t * recip;
        let fr = x - x.trunc();
        let v0 = fr * TWO_PI;
        let k = (v0 * INV_PI).round_ties_even();
        let v4 = NEG_PI * k + v0;
        let sq = v4 * v4;
        let v2 = sq * C9 + C7;
        let v1 = sq * v2 + C5;
        let v31 = sq * v1 + C3;
        let v30 = sq * v31 + 1.0;
        let v48 = v4 * v30;
        let s = if (k as i32) & 1 == 0 { v48 } else { 0.0 - v48 };
        AMP * s
    };
    let (a, b) = (lane(0.25), lane(f32::from_bits(0x3e4c_cccd)));
    [a, a, b, 0.4]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(s: &str) -> u32 {
        u32::from_str_radix(s, 16).unwrap()
    }

    fn lines(tag: &str) -> Vec<(&'static str, &'static str)> {
        include_str!("../../testdata/map_impostor_ref.txt")
            .lines()
            .filter(|l| l.starts_with(tag) && l[tag.len()..].starts_with(' '))
            .map(|l| {
                let (i, o) = l.split_once(" -> ").unwrap();
                (&i[tag.len() + 1..], o)
            })
            .collect()
    }

    /// The tune table lines: "n name maxrots sched ..." then the rest.
    fn read_tune<'a>(t: &'a [&'a str]) -> (Vec<TuneItem>, &'a [&'a str]) {
        let n: usize = t[0].parse().unwrap();
        let items = (0..n).map(|k| TuneItem { name: t[1 + 3 * k].to_string(), max_rots: t[2 + 3 * k].parse().unwrap(), sched: t[3 + 3 * k].parse().unwrap() }).collect();
        (items, &t[1 + 3 * n..])
    }

    /// F-29337 run offline from the original's code over 1,713 tables and names: every shipped name in upper, lower and mixed case and
    /// with a suffix, a letter off, names with `_gfx_hedge_` in them, and random tables with repeated names.
    #[test]
    fn tuning_lookup_matches_the_original() {
        let mut n = 0;
        for (i, o) in lines("TUN") {
            let t: Vec<&str> = i.split(' ').collect();
            let (items, rest) = read_tune(&t);
            assert_eq!(rest.len(), 1, "{i}");
            let (m, s) = tuning_lookup(&items, rest[0]);
            assert_eq!(format!("{m} {s}"), o, "{i}");
            n += 1;
        }
        assert!(n > 1700);
    }

    /// F-28056 and F-64555 over type arrays as the loader builds them (the shipped 53 records with the shipped tuning, and 3,000 small
    /// random arrays: twins with and without a base, hedges, radii around 2.0, MaxRots from 0 to 400): what each type is given, the
    /// size and format of each target the device is asked for (in order), and the twins' links.
    #[test]
    fn pictures_and_twins_match_the_original() {
        let mut n = 0;
        for (i, o) in lines("SET") {
            let t: Vec<&str> = i.split(' ').collect();
            let (tune, rest) = read_tune(&t);
            let nt: usize = rest[0].parse().unwrap();
            let types: Vec<(&str, bool, f32)> = (0..nt).map(|k| (rest[1 + 3 * k], rest[2 + 3 * k] == "1", f32::from_bits(bits(rest[3 + 3 * k])))).collect();
            let names: Vec<&str> = types.iter().map(|t| t.0).collect();
            let low: Vec<bool> = names.iter().map(|n| n.contains(LOW_LOD_SUFFIX)).collect();
            let rots: Vec<(i32, u32)> = names.iter().map(|n| tuning_lookup(&tune, n)).collect();
            let links = link_twins(&names, &low);
            let mut creations: Vec<String> = Vec::new();
            let mut own: Vec<Option<[usize; 3]>> = vec![None; nt];
            let mut depth = false;
            for k in 0..nt {
                if !types[k].1 {
                    continue;
                }
                // (the shared depth target is made with the first record that has a drawable, before the MaxRots and twin tests)
                if !depth {
                    creations.push(format!("ImpostorDepth:2:{0}:{0}:32:0:2", DEPTH_SIDE));
                    depth = true;
                }
                if let Some(p) = picture_plan(rots[k].0, low[k], types[k].2) {
                    let base = creations.len() + 1;
                    creations.push(format!("ShadowImpostor:3:{0}:{0}:8:1000000:d", p.shadow));
                    creations.push(format!("ImpostorColor:3:{}:{}:32:1000000:2", p.color.0, p.color.1));
                    creations.push(format!("ImpostorNormal:3:{}:{}:16:1000000:e", p.normal.0, p.normal.1));
                    own[k] = Some([base, base + 1, base + 2]);
                }
            }
            let rows: Vec<String> = (0..nt)
                .map(|k| {
                    let pics = match (own[k], links[k]) {
                        (Some(p), _) => Some(p),
                        (None, Some(b)) => own[b],
                        _ => None,
                    };
                    let pics = pics.unwrap_or([0, 0, 0]);
                    format!("{} {} {} {} {} {} {}", rots[k].0, rots[k].1, low[k] as u8, pics[0], pics[1], pics[2], links[k].map_or(-1, |b| b as i64))
                })
                .collect();
            // (a twin's link is made for every twin whatever its own drawable; its pictures are its base's)
            assert_eq!(format!("{} | {}", creations.join(" "), rows.join(" ; ")), o, "{i}");
            n += 1;
        }
        assert!(n > 3000);
    }

    fn ref_frames(o: &str) -> Vec<(Vec<(usize, i32)>, Vec<u8>)> {
        o.split(" ;")
            .filter(|f| !f.trim().is_empty())
            .map(|f| {
                let (ev, st) = f.split_once('/').unwrap();
                let mut draws: Vec<(usize, i32)> = Vec::new();
                for e in ev.split_whitespace() {
                    let (i, _rot) = e[1..].split_once(':').unwrap();
                    let i: usize = i.parse().unwrap();
                    match draws.last_mut() {
                        Some(l) if l.0 == i => l.1 += 1,
                        _ => draws.push((i, 1)),
                    }
                }
                (draws, st.split_whitespace().map(|x| x.parse().unwrap()).collect())
            })
            .collect()
    }

    /// F-65228 with F-27437 and the vector pass run offline for 2,500 type arrays over up to 8 frames each (twins with and without
    /// bases, types without pictures or a drawable, MaxRots 0 to 7, every ScheduledFrame mask, the day / night byte 0, 1, 2, 255,
    /// frame counters across the signed wrap, the quarter-rate byte; the cards seen between frames stamp the counter): every picture
    /// drawn, its tiles, the order and each type's last-night byte after each frame are equal.
    #[test]
    fn frame_schedule_matches_the_original() {
        let mut n = 0;
        for (i, o) in lines("DUE") {
            let t: Vec<&str> = i.split(' ').collect();
            let nt: usize = t[0].parse().unwrap();
            let mut slots: Vec<SlotState> = (0..nt)
                .map(|k| {
                    let r = &t[1 + 7 * k..8 + 7 * k];
                    let tw: i32 = r[6].parse().unwrap();
                    SlotState { has_drawable: r[0] == "1", has_pictures: r[1] == "1", low: r[2] == "1", max_rots: r[3].parse().unwrap(), sched: r[4].parse().unwrap(), last_night: r[5].parse().unwrap(), twin: (tw >= 0).then_some(tw as usize), stamp: 0 }
                })
                .collect();
            let rest = &t[1 + 7 * nt..];
            let quad = rest[0] == "1";
            let counter: u32 = rest[1].parse().unwrap();
            let nf: usize = rest[2].parse().unwrap();
            let want = ref_frames(o);
            assert_eq!(want.len(), nf, "{i}");
            for f in 0..nf {
                let night: u8 = rest[3 + 2 * f].parse().unwrap();
                let visible = u32::from_str_radix(rest[4 + 2 * f], 16).unwrap();
                let c = counter.wrapping_add(f as u32);
                let plan = frame_plan(&mut slots, c, quad, night);
                let last: Vec<u8> = slots.iter().map(|s| s.last_night).collect();
                assert_eq!(plan, want[f].0, "frame {f} {i}");
                assert_eq!(last, want[f].1, "frame {f} {i}");
                for k in 0..nt {
                    if visible >> k & 1 == 1 {
                        let b = slots[k].twin.unwrap_or(k);
                        slots[b].stamp = c;
                    }
                }
            }
            n += 1;
        }
        assert!(n > 2000);
    }

    /// F-80686 with F-61167 run offline: 2,000 arrays over up to 13 calls each (the hour from -100, steps of 0, 0.001, 1/120 and a unit
    /// in the last place either side, 0.5, backwards, a day, a wrap past midnight, NaN; the index from 0 up to 2^20 and -1): the wind
    /// zeroed at every call, the records visited with a picture drawn or only the vectors, the index and the stored hour.
    #[test]
    fn shadow_round_matches_the_original() {
        let mut n = 0;
        for (i, o) in lines("RRB") {
            let t: Vec<&str> = i.split(' ').collect();
            let nt: usize = t[0].parse().unwrap();
            let slots: Vec<(bool, bool, bool)> = (0..nt).map(|k| (t[1 + 3 * k] == "1", t[2 + 3 * k] == "1", t[3 + 3 * k] == "1")).collect();
            let rest = &t[1 + 3 * nt..];
            let mut round = ShadowRound { index: rest[0].parse::<u32>().unwrap() as i32, last_hour: f32::from_bits(bits(rest[1])) };
            let nh: usize = rest[2].parse().unwrap();
            let calls: Vec<&str> = o.split(" | ").collect();
            assert_eq!(calls.len(), nh, "{i}");
            for h in 0..nh {
                let visits = round.turn(&slots, f32::from_bits(bits(rest[3 + h])));
                let mut got = vec!["W0".to_string()];
                got.extend(visits.iter().map(|(k, v)| format!("B{k}:{}", if *v == Visit::Draw { 'd' } else { 'v' })));
                got.push(format!("idx={}", round.index));
                got.push(format!("last={:08x}", if round.last_hour.is_nan() { f32::NAN.to_bits() } else { round.last_hour.to_bits() }));
                assert_eq!(got.join(" "), calls[h].trim(), "call {h} {i}");
            }
            n += 1;
        }
        assert!(n > 1900);
    }

    /// F-48104 run offline: the samplers set for the shadow byte 0, 1, 2, 255 and 256 (low byte 0), with and without the effect.
    #[test]
    fn card_bindings_match_the_original() {
        for (i, o) in lines("BND") {
            let t: Vec<&str> = i.split(' ').collect();
            let sh = bits(t[0]);
            let (c, n, s) = (bits(t[2]), bits(t[3]), bits(t[4]));
            let (d, nn) = card_pictures(sh, c, n, s, 0);
            let ev: Vec<&str> = o.split(' ').collect();
            assert_eq!(ev[0], format!("S:11:{d:x}"), "{i}");
            assert_eq!(ev[1], format!("S:22:{nn:x}"), "{i}");
        }
    }

    /// F-65228's wind vector run offline for 1,417 clocks (0 to 8, 0 to 4000, to 1e6, negative; exact multiples of the periods).
    #[test]
    fn wind_matches_the_original() {
        let mut n = 0;
        for (i, o) in lines("WND") {
            let w = wind_vector(f32::from_bits(bits(i)));
            let want: Vec<u32> = o.split(':').nth(4).unwrap().split(',').map(|x| bits(x.trim())).collect();
            let got: Vec<u32> = w.iter().map(|x| x.to_bits()).collect();
            assert_eq!(got, want, "{i}");
            n += 1;
        }
        assert!(n > 1400);
    }

    #[test]
    fn shadow_round_starts_with_everything() {
        let mut r = ShadowRound::default();
        let slots = [(true, true, false), (true, true, true), (false, true, false)];
        assert_eq!(r.turn(&slots, 12.0), vec![(0, Visit::Draw), (1, Visit::Vectors)]);
        assert_eq!(r.turn(&slots, 12.001), vec![(1, Visit::Vectors)]);
        assert_eq!(r.turn(&slots, 12.002), vec![(0, Visit::Draw)]);
    }
}
