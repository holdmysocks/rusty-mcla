//! Race pay-outs as the original's experience manager computes them (read from the
//! executable): reputation follows a curve of ten bands, prize money grows with the
//! career, the class of the player's car and the difficulty of the event.

use std::collections::HashMap;

/// Share of the career at which each of the thirteen reputation levels starts.
pub const LEVEL_FRAC: [f32; 13] = [0.0, 0.04, 0.1, 0.17, 0.25, 0.34, 0.43, 0.52, 0.6, 0.68, 0.76, 0.84, 1.0];
const BASE_EXPERIENCE: f32 = 100.0;
const EXPERIENCE_MODIFIER: f32 = 15.0;
/// experience bonus per event difficulty (Easy, Medium, Hard, Impossible)
const DIFF_EXP: [f32; 4] = [-1.0, 0.0, 1.0, 3.0];
/// money factor per event difficulty
const DIFF_MONEY: [f32; 4] = [0.8, 1.0, 1.2, 1.4];
/// money factor per performance class of the car (A, B, C, D)
const CLASS_MONEY: [f32; 4] = [2.0, 1.5, 1.2, 1.0];
pub const START_MONEY: i64 = 5000;
/// icon colours of racers by difficulty (Easy, Medium, Hard, Impossible)
pub const DIFF_COLORS: [[u8; 3]; 4] = [[153, 204, 0], [255, 204, 0], [255, 120, 0], [204, 0, 51]];

/// Reputation held at share `f` (0..1) of the career: each tenth is worth
/// (100 + 15 * band) * 20 points, 33 500 in all.
pub fn rep_curve(f: f32) -> f32 {
    let n = (f.min(1.0) * 100.0) as i32;
    let (tens, units) = (n / 10, n % 10);
    let band = |i: i32| (i as f32 * EXPERIENCE_MODIFIER + BASE_EXPERIENCE) * 20.0;
    let mut acc = 0.0;
    for i in 0..tens {
        acc += band(i);
    }
    acc + band(tens) * 0.1 * units as f32
}

/// Reputation level (0..12) and share of the career (0..1) for `rep` points.
pub fn progress(rep: i64) -> (usize, f32) {
    let thr = |l: usize| rep_curve(LEVEL_FRAC[l]);
    let mut level = 0;
    for l in (1..=12).rev() {
        if rep >= (thr(l) + 0.5) as i64 {
            level = l;
            break;
        }
    }
    let next = (level + 1).min(12);
    let (a, b) = (thr(level), thr(next));
    let span = LEVEL_FRAC[next] - LEVEL_FRAC[level];
    let extra = if b - a > 0.001 { (rep as f32 - a) / (b - a) * span } else { span };
    (level, (LEVEL_FRAC[level] + extra).clamp(0.0, 1.0))
}

fn floor10(v: f32) -> f32 {
    (v * 0.1) as i32 as f32 * 10.0
}

/// Experience an event is worth right now: a hundredth of the career at the current
/// point of the curve, shifted by the difficulty.
pub fn dynamic_exp(rep: i64, diff: usize) -> f32 {
    let p = progress(rep).1;
    let (a, b) = if p < 1.0 { (rep_curve(p), rep_curve(p + 0.0105)) } else { (rep_curve(0.99), rep_curve(1.0)) };
    floor10((DIFF_EXP[diff.min(3)] * EXPERIENCE_MODIFIER + (b - a)) * 0.55)
}

/// Performance class of a car from its performance value: 0 = A .. 3 = D.
pub fn perf_class(perf: f32) -> usize {
    if perf < 0.144 {
        3
    } else if perf < 0.38 {
        2
    } else if perf < 0.645 {
        1
    } else {
        0
    }
}

/// Prize money an event is worth (mcExperienceManager_CalculateRaceMoney).
pub fn race_money(rep: i64, class: usize, diff: usize, round: f32) -> f32 {
    let p = progress(rep).1;
    let v = (p * 3000.0 + 250.0) * CLASS_MONEY[class.min(3)] * DIFF_MONEY[diff.min(3)];
    (v / round) as i32 as f32 * round
}

/// What finishing a race in `place` (1-based) of `field` pays: (money, reputation).
/// `factor` is 1 for a new event and 0.6 for one that was already won.
pub fn race_reward(rep: i64, perf: f32, diff: usize, place: usize, field: usize, factor: f32) -> (i64, i64) {
    let field = field.max(1);
    let share = (field + 1).saturating_sub(place.max(1)) as f32 / field as f32;
    let exp = floor10(share * factor * dynamic_exp(rep, diff));
    let money = if place > 3 { 0.0 } else { floor10(share * factor * race_money(rep, perf_class(perf), diff, 10.0)) };
    (if money > 1.0 { money as i64 } else { 0 }, if exp > 1.0 { exp as i64 } else { 0 })
}

/// Reputation for a pursuit escaped after `seconds` (COP_EVADE, sub_826BA238): a quarter
/// of a Medium event's experience for every minute begun (sub_823DBAA8 rounds up), five at
/// the most.
pub fn pursuit_exp(rep: i64, seconds: f32) -> i64 {
    let minutes = (seconds * (1.0 / 60.0)).ceil().clamp(0.0, 5.0);
    floor10(minutes * (dynamic_exp(rep, 1) * 0.25)) as i64
}

/// Reputation for the run to the start line (BEAT_ME_THERE_RACE_COMPLETE, sub_826B99A0):
/// half the experience of an event of that difficulty (sub_826B95C0), by the place among
/// the `field` cars - (field - place + 1) / field - cut to tens; given when above 1.
pub fn beat_me_there_exp(rep: i64, diff: usize, place: usize, field: usize) -> i64 {
    let field = field.max(1);
    let share = (field + 1).saturating_sub(place.max(1)) as f32 / field as f32;
    let exp = floor10(share * dynamic_exp(rep, diff) * 0.5);
    if exp > 1.0 { exp as i64 } else { 0 }
}

/// Share of the prize a payback race hands over after `wins` wins.
pub fn payback_money(rep: i64, diff: usize, wins: usize) -> i64 {
    (race_money(rep, 0, diff, 20.0) * (0.10 + 0.05 * wins as f32)) as i64
}

/// (sub_826BCB70 / sub_826BCAF0; the steps are the career's, set in its constructor at
/// 0x826C1960..0x826C1A08: -0.15, 0.025, 0.15, 0.3 and 0.45 for the step the script native
/// folds into Impossible.)
/// How hard a racer is for the player, from the gap between their cars performance
/// values: -1 = no contest, 0 Easy, 1 Medium, 2 Hard, 3 Impossible.
pub fn difficulty(racer_perf: f32, player_perf: f32) -> i32 {
    let d = racer_perf - player_perf;
    if d >= 0.30 {
        3
    } else if d >= 0.15 {
        2
    } else if d >= 0.025 {
        // (approx.) the step to Medium: it stood at 0.025, but a new career's first man -
        // a stock 280Z, 0.0916, against the stock SCIROCCO, 0.0593: 0.032 apart - is EASIEST
        // in the original (green ring, $100 for second of two: `ng_chase_010`, `ng_rewards`)
        1
    } else if d >= -0.15 {
        0
    } else {
        -1
    }
}

/// The opponents level (0..10, a step of the rubber band tuning) that adapts to how the
/// player does (RubberBandMgr): results add to a tally and every 18 points move the level.
#[derive(Clone, Copy, Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Adaptive {
    pub level: i32,
    pub points: i32,
}

impl Adaptive {
    /// Rubber band step for a race meant to be of difficulty `desired`; `odd` is the
    /// difficulty of the first racer that is not of that difficulty for the player.
    pub fn step(&self, desired: i32, odd: Option<i32>) -> usize {
        let l = if desired <= 0 {
            self.level - 2
        } else {
            match odd {
                Some(d) if d >= desired => self.level - 2,
                Some(_) => self.level + 3,
                None => self.level,
            }
        };
        l.clamp(0, 10) as usize
    }

    /// Account for a finished race of difficulty `diff`.
    pub fn result(&mut self, won: bool, diff: i32) {
        let (delta, bound) = match (won, diff) {
            (true, 3) => (14, 10),
            (true, 2) => (9, 8),
            (true, 1) => (6, 7),
            (true, 0) => (4, 4),
            (false, 3) => (-4, 5),
            (false, 2) => (-6, 4),
            (false, 1) => (-9, 1),
            (false, 0) => (-14, 0),
            _ => (0, 5),
        };
        let mut points = self.points;
        if (won && self.level < bound) || (!won && self.level > bound) {
            points += delta;
        }
        if points >= 18 {
            if self.level < bound {
                self.points = points % 18 - 18;
                self.level = (self.level + 1).clamp(0, bound);
            }
        } else if points <= -18 {
            if self.level > bound {
                self.points = points % 18 + 18;
                self.level = (self.level - 1).max(bound).min(10);
            }
        } else {
            self.points = points;
        }
    }
}

/// The performance figures of every car at upgrade levels 0, 50 and 100
/// (`tune/vehicle/car_performance_sample_N.lst`).
#[derive(Default)]
pub struct Samples {
    cars: HashMap<String, [f32; 3]>,
}

impl Samples {
    pub fn load(data_dir: &str) -> Samples {
        let mut cars: HashMap<String, [f32; 3]> = HashMap::new();
        // (the files are stored under the hashes of their names)
        for f in ["0x90cdf4d3", "0xde74d3f4"] {
            let Ok(t) = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/vehicle").join(f)) else { continue };
            for s in t.split("sample").skip(1) {
                let tok: Vec<&str> = s.split_whitespace().collect();
                let field = |k: &str| tok.iter().position(|t| *t == k).and_then(|i| tok.get(i + 1)).copied();
                let (Some(name), Some(m), Some(p)) = (field("name"), field("mod").and_then(|v| v.parse::<i32>().ok()), field("performance").and_then(|v| v.parse::<f32>().ok())) else { continue };
                let e = cars.entry(name.to_ascii_lowercase()).or_insert([p; 3]);
                match m {
                    0 => e[0] = p,
                    50 => e[1] = p,
                    _ => e[2] = p,
                }
            }
        }
        log::info!("car performance samples: {} cars", cars.len());
        Samples { cars }
    }

    /// Performance value of `car` with `tuned` (0..1) of its upgrades fitted.
    pub fn value(&self, car: &str, tuned: f32) -> Option<f32> {
        let s = self.cars.get(&car.to_ascii_lowercase())?;
        let m = tuned.clamp(0.0, 1.0) * 2.0;
        Some(if m < 1.0 { s[0] + (s[1] - s[0]) * m } else { s[1] + (s[2] - s[1]) * (m - 1.0) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_matches_the_career_length() {
        assert_eq!(rep_curve(1.0) as i64, 33_500);
        assert_eq!(rep_curve(0.1) as i64, 2000);
        assert_eq!(progress(0), (0, 0.0));
        assert_eq!(progress(33_500).0, 12);
        assert_eq!(dynamic_exp(0, 1) as i64, 110);
        assert_eq!(race_money(0, 3, 1, 10.0) as i64, 250);
        assert_eq!(race_reward(0, 0.1, 1, 2, 4, 1.0), (180, 80));
        // (a new game's 260 / 210: the run to the first line gave 50 to the reputation alone)
        assert_eq!((beat_me_there_exp(0, 0, 1, 2), beat_me_there_exp(0, 1, 1, 2), beat_me_there_exp(0, 1, 2, 2)), (50, 50, 20));
        // (every minute begun, five at the most; nothing for no time at all)
        assert_eq!((pursuit_exp(0, 0.0), pursuit_exp(0, 30.0), pursuit_exp(0, 61.0), pursuit_exp(0, 900.0)), (0, 20, 50, 130));
        // (past the top of the career an event is still worth what the last hundredth is)
        assert_eq!((progress(119_080), dynamic_exp(119_080, 1) as i64), ((12, 1.0), 250));
        let mut a = Adaptive::default();
        for _ in 0..3 {
            a.result(true, 1);
        }
        assert_eq!(a, Adaptive { level: 1, points: -18 });
        assert_eq!(a.step(1, None), 1);
        assert_eq!(a.step(0, None), 0);
        assert_eq!(a.step(2, Some(1)), 4);
    }
}
