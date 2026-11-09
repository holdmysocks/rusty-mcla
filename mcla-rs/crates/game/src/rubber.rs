//! Opponent rubber banding as the original's mcRubberBandMgr does it (decoded 2026-10-21, docs/AI.md
//! "Rubber banding"; rule file work/racerai/rubber.md).
//!
//! It is only a cap on the throttle a computer racer's driver asks for (mcBrainRacing sub_822BCFE0,
//! 0x822BD544..0x822BD668: `throttle = min(throttle, cap)`). Every frame (sub_82743E28) each racer's gap
//! to the best-placed human is measured along the chain of checkpoints, shifted by the racer's rank
//! difference to that human (AheadDistOffsets / BehindDistOffsets), and turned into the cap by the
//! tuning of the race's step (`RubberBandTune00` .. `RubberBandTune10` in `tune/career`).

use crate::police::Tuning;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RubberBand {
    pub max_throttle: f32,
    pub min_speed: f32,
    pub min_dist: f32,
    pub max_dist: f32,
    pub min_throttle: f32,
    pub behind_min_dist: f32,
    pub behind_max_dist: f32,
    pub behind_max_throttle: f32,
    pub dist_behind_before_shortcuts: f32,
    pub nitro_min_dist_from_start: f32,
    /// (x 2 at load, sub_82743A10)
    pub nitro_min_dist_from_finish: f32,
    pub nitro_min_dist_behind_player: f32,
    /// (x 2.5 at load)
    pub unlimited_nitro_min_dist_behind_player: f32,
    pub home_stretch_behind_max_throttle: f32,
    /// (x 2 at load)
    pub home_stretch_distance: f32,
    pub player_close_max_throttle: f32,
    pub player_close_distance: f32,
    pub ahead_offsets: [f32; 8],
    /// as the original keeps it: the file's [1..6] replaced by its [2..7] at load
    pub behind_offsets: [f32; 8],
}

fn float_array(text: &str, tag: &str) -> Vec<f32> {
    let Some(a) = text.find(&format!("<{tag} ")) else { return Vec::new() };
    let rest = &text[a..];
    let Some(s) = rest.find(">") else { return Vec::new() };
    let Some(e) = rest.find(&format!("</{tag}>")) else { return Vec::new() };
    rest[s + 1..e].split_whitespace().filter_map(|x| x.parse().ok()).collect()
}

/// The disc's file name hash (`rubberbandtune%02d.xml`)
fn joaat(s: &str) -> u32 {
    let mut h: u32 = 0;
    for c in s.to_ascii_lowercase().bytes() {
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}

fn arr8(v: Vec<f32>) -> [f32; 8] {
    let mut a = [0.0; 8];
    for (i, x) in v.into_iter().take(8).enumerate() {
        a[i] = x;
    }
    a
}

/// The mcRubberBandTuning defaults a file's missing key keeps (F-36408), by offset +0 .. +72: the scalars in file order
/// (+32 / +36 OutsideTrafficBubble*, no reader). The offset arrays +76 / +108 get 0.0 in [0..7]; [7] is not written.
/// (All eleven shipped files set every key.)
const DEFAULTS: [(&str, f32); 19] = [
    ("MaxThrottle", 1.0), ("MinSpeed", 100.0), ("MinDist", 10000.0), ("MaxDist", 11000.0), ("MinThrottle", 0.7),
    ("BehindMinDist", 10000.0), ("BehindMaxDist", 11000.0), ("BehindMaxThrottle", 1.0), ("OutsideTrafficBubbleThresholdSpeed", 1.0),
    ("OutsideTrafficBubbleThrottlePenalty", 1.0), ("DistBehindBeforeTakingShortcuts", 100.0), ("UseNitroMinDistFromStart", 100.0),
    ("UseNitroMinDistFromFinish", 100.0), ("UseNitroMinDistBehindPlayer", 0.0), ("UnlimitedNitroMinDistBehindPlayer", 200.0),
    ("HomeStretchBehindMaxThrottle", 0.8), ("HomeStretchDistance", 200.0), ("PlayerCloseMaxThrottle", 1.0), ("PlayerCloseDistance", 0.0),
];

impl RubberBand {
    fn from_file(p: &Path) -> Option<RubberBand> {
        let text = std::fs::read_to_string(p).ok()?;
        if !text.contains("<mcRubberBandTuning>") {
            return None;
        }
        let t = Tuning::load(p);
        let g = |k: &str| t.get(k, DEFAULTS.iter().find(|d| d.0 == k).map_or(0.0, |d| d.1));
        let mut behind = arr8(float_array(&text, "BehindDistOffsets"));
        // (LoadTuning: Behind[1..6] = Behind[2..7]; [0] and [7] kept)
        for i in 1..7 {
            behind[i] = behind[i + 1];
        }
        Some(RubberBand {
            max_throttle: g("MaxThrottle"),
            min_speed: g("MinSpeed"),
            min_dist: g("MinDist"),
            max_dist: g("MaxDist"),
            min_throttle: g("MinThrottle"),
            behind_min_dist: g("BehindMinDist"),
            behind_max_dist: g("BehindMaxDist"),
            behind_max_throttle: g("BehindMaxThrottle"),
            dist_behind_before_shortcuts: g("DistBehindBeforeTakingShortcuts"),
            nitro_min_dist_from_start: g("UseNitroMinDistFromStart"),
            nitro_min_dist_from_finish: g("UseNitroMinDistFromFinish") * 2.0,
            nitro_min_dist_behind_player: g("UseNitroMinDistBehindPlayer"),
            unlimited_nitro_min_dist_behind_player: g("UnlimitedNitroMinDistBehindPlayer") * 2.5,
            home_stretch_behind_max_throttle: g("HomeStretchBehindMaxThrottle"),
            home_stretch_distance: g("HomeStretchDistance") * 2.0,
            player_close_max_throttle: g("PlayerCloseMaxThrottle"),
            player_close_distance: g("PlayerCloseDistance"),
            ahead_offsets: arr8(float_array(&text, "AheadDistOffsets")),
            behind_offsets: behind,
        })
    }

    /// The gap shifted by the racer's rank difference `k` to the human (+384): k places ahead
    /// `gap - Ahead[k-1]`, k places behind `gap + Behind[k-1]`.
    pub fn shifted_gap(&self, gap: f32, k: i32) -> f32 {
        if k < 0 {
            gap - self.ahead_offsets[((-k - 1) as usize).min(7)]
        } else if k > 0 {
            gap + self.behind_offsets[((k - 1) as usize).min(7)]
        } else {
            gap
        }
    }

    /// The throttle cap (+404) for a racer at shifted gap `g` (+384, metres ahead of the human), its own
    /// distance to the finish (+396) and the squared straight-line distance to the nearest human (+400),
    /// going at `speed` (car +224) with its driver's MaxSpeed (aivVehicleTuning [0]).
    /// As F-17058 computes it (brain +412 .. +484 = the tune from its +0): `ble` on the speed (not above: 1, a NaN speed
    /// too), the interpolations fused, the clamps and the min / max as fsel (a NaN takes the second value).
    pub fn cap(&self, g: f32, to_finish: f32, human_d2: f32, speed: f32, max_speed: f32) -> f32 {
        if !(speed > self.min_speed * max_speed) {
            return 1.0;
        }
        let fsel = |a: f32, b: f32, c: f32| if a >= 0.0 { b } else { c };
        let clamp01 = |t: f32| {
            let m = fsel(t, t, 0.0);
            fsel(m - 1.0, 1.0, m)
        };
        let mx = self.max_throttle;
        let mut cap = if g > 0.0 {
            let t = clamp01((g - self.min_dist) / (self.max_dist - self.min_dist));
            let c = t.mul_add(self.min_throttle - mx, mx);
            fsel(mx - c, c, mx)
        } else {
            let t = clamp01((-self.behind_min_dist - g) / (self.behind_max_dist - self.behind_min_dist));
            t.mul_add(self.behind_max_throttle - mx, mx)
        };
        if to_finish < self.home_stretch_distance {
            cap = fsel(cap - self.home_stretch_behind_max_throttle, self.home_stretch_behind_max_throttle, cap);
        } else if human_d2 < self.player_close_distance * self.player_close_distance {
            cap = fsel(mx - self.player_close_max_throttle, mx, self.player_close_max_throttle);
        }
        cap
    }
}

/// All eleven steps, `RubberBandTune00` first (an empty list if the files are missing).
pub fn load_all(data_dir: &str) -> Vec<RubberBand> {
    let dir = Path::new(data_dir).join("tune/career");
    let mut out = Vec::new();
    for i in 0..=10 {
        let p = dir.join(format!("0x{:08x}", joaat(&format!("rubberbandtune{:02}.xml", i))));
        match RubberBand::from_file(&p) {
            Some(r) => out.push(r),
            None => {
                log::warn!("rubber band: step {} missing ({})", i, p.display());
                return Vec::new();
            }
        }
    }
    log::info!("rubber band: {} difficulty steps", out.len());
    out
}

/// A racer's place on the chain of checkpoints: `p` = checkpoints passed (lap * n + next), `d` = its
/// distance to the checkpoint it heads for (0 once finished).
#[derive(Debug, Clone, Copy)]
pub struct ChainPos {
    pub p: usize,
    pub d: f32,
}

/// The distances along the chain (`seg[k]` from checkpoint k to k + 1; 200 m each in an unordered race).
pub struct Chain<'a> {
    pub seg: &'a [f32],
    /// checkpoints of the whole race (laps x per lap)
    pub total: usize,
}

impl Chain<'_> {
    fn sum(&self, from: usize, to_excl: usize) -> f32 {
        (from..to_excl).map(|k| self.seg.get(k).copied().unwrap_or(0.0)).sum()
    }

    /// How far `a` is ahead of `l` along the route (+388, sub_82743448).
    pub fn gap(&self, a: ChainPos, l: ChainPos) -> f32 {
        let last = self.total.saturating_sub(1);
        let da = if a.p < self.total { a.d } else { 0.0 };
        l.d - self.sum(a.p, l.p.min(last)) + self.sum(l.p, a.p.min(last)) - da
    }

    /// The racer's distance to the finish (+396, sub_82743798).
    pub fn to_finish(&self, a: ChainPos) -> f32 {
        a.d + self.sum(a.p, self.total.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults against the original's F-36408 run offline (`work/racerai/rbdef/defaults.bin`: the 140-byte tuning after
    /// it, memory filled with 0xAB before): +0 .. +72 word for word, +76 .. +100 and +108 .. +132 zero, +104 / +136 untouched.
    #[test]
    fn defaults_are_the_originals() {
        let Ok(b) = std::fs::read("D:/MCLA/work/racerai/rbdef/defaults.bin") else { return };
        let w = |i: usize| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
        for (i, d) in DEFAULTS.iter().enumerate() {
            assert_eq!(w(i), d.1.to_bits(), "{} at +{}", d.0, 4 * i);
        }
        for i in (19..26).chain(27..34) {
            assert_eq!(w(i), 0, "+{}", 4 * i);
        }
        assert_eq!((w(26), w(34)), (0xABAB_ABAB, 0xABAB_ABAB));
    }

    fn tune(step: usize) -> Option<RubberBand> {
        let dir = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        load_all(&dir).into_iter().nth(step)
    }

    /// Step 04's numbers as decoded: ahead 20..65 m from 0.85 to 0.29, behind 3..16 m to 0.95, home
    /// stretch 2 x 225 m at 0.78, player close 20 m -> 0.995; no cap while slow.
    #[test]
    fn step04_cap() {
        let Some(r) = tune(4) else { return };
        assert_eq!(r.home_stretch_distance, 450.0);
        assert_eq!(r.behind_offsets, [0.0, 24.0, 48.0, 85.0, 140.0, 195.0, 265.0, 265.0]);
        let far = 1e8;
        assert!((r.cap(10.0, 2000.0, far, 40.0, 60.0) - 0.85).abs() < 1e-6);
        assert!((r.cap(42.5, 2000.0, far, 40.0, 60.0) - 0.57).abs() < 1e-5);
        assert!((r.cap(100.0, 2000.0, far, 40.0, 60.0) - 0.29).abs() < 1e-6);
        assert!((r.cap(-9.5, 2000.0, far, 40.0, 60.0) - 0.90).abs() < 1e-5);
        assert!((r.cap(-50.0, 400.0, far, 40.0, 60.0) - 0.78).abs() < 1e-6);
        assert!((r.cap(100.0, 2000.0, 100.0, 40.0, 60.0) - 0.995).abs() < 1e-6);
        assert_eq!(r.cap(100.0, 2000.0, far, 30.0, 61.0), 1.0);
        assert_eq!(r.shifted_gap(100.0, -2), 35.0);
        assert_eq!(r.shifted_gap(-10.0, 2), 14.0);
    }

    /// The cap against the original's F-17058 run offline (tools/rubber_cap_reference.py, 6,000 cases: brain +404 after
    /// the frame, with the tune at +412 .. +484): word for word. Off (+380 clear) the original writes 1.
    #[test]
    fn cap_matches_the_original() {
        let u = |s: &str| u32::from_str_radix(&s.replace('_', ""), 16).unwrap();
        let f = |s: &str| f32::from_bits(u(s));
        let (mut n, mut bad) = (0, 0);
        for l in include_str!("testdata/rubber_cap_ref.txt").lines() {
            let (lhs, rhs) = l.split_once(" -> ").unwrap();
            let s: Vec<&str> = lhs.split_whitespace().skip(1).collect();
            let t = |i: usize| f(s[6 + i]);
            let r = RubberBand {
                max_throttle: t(0),
                min_speed: t(1),
                min_dist: t(2),
                max_dist: t(3),
                min_throttle: t(4),
                behind_min_dist: t(5),
                behind_max_dist: t(6),
                behind_max_throttle: t(7),
                dist_behind_before_shortcuts: 0.0,
                nitro_min_dist_from_start: 0.0,
                nitro_min_dist_from_finish: 0.0,
                nitro_min_dist_behind_player: 0.0,
                unlimited_nitro_min_dist_behind_player: 0.0,
                home_stretch_behind_max_throttle: t(15),
                home_stretch_distance: t(16),
                player_close_max_throttle: t(17),
                player_close_distance: t(18),
                ahead_offsets: [0.0; 8],
                behind_offsets: [0.0; 8],
            };
            let cap = if u(s[0]) != 0 { r.cap(f(s[1]), f(s[2]), f(s[3]), f(s[4]), f(s[5])) } else { 1.0 };
            n += 1;
            if cap.to_bits() != u(rhs) {
                if bad < 5 {
                    eprintln!("{l}\n   port {:08x}", cap.to_bits());
                }
                bad += 1;
            }
        }
        assert_eq!((n, bad), (6000, 0));
    }

    #[test]
    fn chain_gap() {
        let seg = [100.0, 50.0, 80.0, 0.0];
        let c = Chain { seg: &seg, total: 4 };
        // a heads for cp 3 (20 m off), l for cp 1 (30 m off): a is 50 + 80 - 20 + 30 ahead
        let g = c.gap(ChainPos { p: 3, d: 20.0 }, ChainPos { p: 1, d: 30.0 });
        assert!((g - 140.0).abs() < 1e-4);
        let g2 = c.gap(ChainPos { p: 1, d: 30.0 }, ChainPos { p: 3, d: 20.0 });
        assert!((g2 + 140.0).abs() < 1e-4);
        assert!((c.to_finish(ChainPos { p: 1, d: 30.0 }) - 160.0).abs() < 1e-4);
    }
}
