//! The original's district ambience (the ambience manager, sub_821EB810 and friends): the district whose circles
//! are nearest picks the bed (day / night), its distant one-shots and surround "quadshots" fire on random timers
//! 6 km away in random directions, the two nearest location emitters play at their places, faded by distance, and
//! a district's line emitter follows the listener along its polyline. Rules: `docs/reference/audio/AMBIENCE_RULE.md`.
//!
//! Not ported (open there): the bed duck near an emitter (a dynamic mixer, MC4_HANGOUT_DYNMIXER_APPLY), the
//! district cross-fade, the freeway / tunnel / interior / mall districts (no circles: chosen by something else),
//! map-placed emitters, rain / wind / thunder.

use crate::sndobj::{Handle, Req, SoundLib};
use glam::Vec3;
use mcla_formats::audcfg::fsel;
use mcla_formats::hash::joaat;

fn be_u32(o: &[u8], at: usize) -> u32 {
    o.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).unwrap_or(0)
}
fn be_f32(o: &[u8], at: usize) -> f32 {
    f32::from_bits(be_u32(o, at))
}
fn be_u16(o: &[u8], at: usize) -> u16 {
    o.get(at..at + 2).map(|b| u16::from_be_bytes(b.try_into().unwrap())).unwrap_or(0)
}

const NULL_SOUND: &str = "NULL_SOUND";

#[derive(Clone, Copy, Debug)]
struct Shot {
    flags: u8,
    sound: u32,
    /// dB
    volume: f32,
    /// degrees, None = any direction
    centre: Option<f32>,
    spread: f32,
    min: f32,
    max: f32,
}

#[derive(Clone, Copy, Debug)]
struct Loc {
    flags: u8,
    pos: Vec3,
    outer: f32,
    inner: f32,
    /// day sound, day companion, night sound, night companion
    sounds: [u32; 4],
}

#[derive(Clone, Debug)]
struct District {
    name: String,
    bed: [u32; 2],
    line_dist: f32,
    line_sound: u32,
    circles: Vec<(f32, f32, f32)>,
    shots: Vec<Shot>,
    quads: Vec<Shot>,
    locs: Vec<Loc>,
    line: Vec<Vec3>,
    /// +30 bit 0: the district sets the weather categories (+31 s16 rain 0.01 dB, +35 s16 thunder 0.01 dB; the LPFs +33 / +37
    /// are not applied by the port's mixer)
    weather_mix: Option<(f32, f32)>,
}

fn parse_district(name: &str, o: &[u8]) -> Option<District> {
    if o.first() != Some(&5) {
        return None;
    }
    let mut p = 59;
    let na = *o.get(p)? as usize;
    p += 1;
    let circles = (0..na).map(|k| (be_f32(o, p + 12 * k), be_f32(o, p + 12 * k + 4), be_f32(o, p + 12 * k + 8))).collect();
    p += 12 * na;
    let mut shots = [Vec::new(), Vec::new()];
    for list in shots.iter_mut() {
        let n = *o.get(p)? as usize;
        p += 1;
        for k in 0..n {
            let e = p + 19 * k;
            let c = be_u16(o, e + 7);
            list.push(Shot {
                flags: o[e],
                sound: be_u32(o, e + 1),
                volume: be_u16(o, e + 5) as i16 as f32 * 0.01,
                centre: (c != 0xFFFF).then_some(c as f32),
                spread: be_u16(o, e + 9) as i16 as f32,
                min: be_f32(o, e + 11),
                max: be_f32(o, e + 15),
            });
        }
        p += 19 * n;
    }
    let nd = *o.get(p)? as usize;
    p += 1;
    let locs = (0..nd)
        .map(|k| {
            let e = p + 41 * k;
            Loc {
                flags: o[e],
                pos: Vec3::new(be_f32(o, e + 1), be_f32(o, e + 5), be_f32(o, e + 9)),
                outer: be_f32(o, e + 17),
                inner: be_f32(o, e + 21),
                sounds: [be_u32(o, e + 25), be_u32(o, e + 29), be_u32(o, e + 33), be_u32(o, e + 37)],
            }
        })
        .collect();
    p += 41 * nd;
    let ne = o.get(p).copied().unwrap_or(0) as usize;
    p += 1;
    let line = (0..ne).map(|k| Vec3::new(be_f32(o, p + 12 * k), be_f32(o, p + 12 * k + 4), be_f32(o, p + 12 * k + 8))).collect();
    let [shots, quads] = shots;
    let weather_mix = (o.get(30).copied().unwrap_or(0) & 1 != 0).then(|| (be_u16(o, 31) as i16 as f32 * 0.01, be_u16(o, 35) as i16 as f32 * 0.01));
    Some(District { name: name.to_string(), bed: [be_u32(o, 10), be_u32(o, 14)], line_dist: be_f32(o, 39), line_sound: be_u32(o, 43), circles, shots, quads, locs, line, weather_mix })
}

struct Slot {
    /// (district, location record)
    key: (usize, usize),
    sound: Handle,
    companion: Option<Handle>,
}

pub struct Ambience {
    districts: Vec<District>,
    garage: Option<usize>,
    cur: Option<usize>,
    night: bool,
    bed: Option<Handle>,
    timers: Vec<f32>,
    quad_timers: Vec<f32>,
    shots: Vec<Handle>,
    slots: Vec<Slot>,
    line: Option<Handle>,
    landmark_curve: u32,
    null: u32,
    now: f32,
    rain: Rain,
}

/// The rain and thunder of the ambience manager (sub_821E8410; `reference/audio/WEATHER_AUDIO_RULE.md`).
#[derive(Default)]
struct Rain {
    /// AMBIENCE_GLOBAL +22 RAIN_INT, +26 RAIN_EXT, +30 THUNDER, +34 / +38 thunder interval, +58 ramp time, +62 covered rain dB
    int_snd: u32,
    ext_snd: u32,
    thunder_snd: u32,
    thunder_gap: (f32, f32),
    ramp: f32,
    covered_db: (f32, f32),
    /// slots +280 (RAIN_EXT), +836 (RAIN_INT), +284 (thunder); byte +1074; +824 the next thunder
    ext: Option<Handle>,
    int: Option<Handle>,
    thunder: Option<Handle>,
    raining: bool,
    next_thunder: f32,
    /// +1048 / +1052: the share of the player's wheels with something above them, and its smoothed value
    cover_target: f32,
    cover: f32,
    /// the player car's probe walk (this+2213 / +2214)
    probe_i: usize,
    probe_hits: usize,
    /// the mix last applied: (district, listener covered)
    mix: Option<(Option<usize>, bool)>,
}

/// Voices for the distant one-shots and quadshots at once (the manager's free-slot lists; size not decoded).
const SHOT_SLOTS: usize = 4;

impl Ambience {
    pub fn new(lib: &SoundLib) -> Option<Ambience> {
        let g = &lib.game;
        let glob = g.get_name("AMBIENCE_GLOBAL")?;
        let n = *glob.get(70)? as usize;
        let mut districts: Vec<District> = (0..n).filter_map(|k| {
            let h = be_u32(glob, 71 + 4 * k);
            parse_district(g.name_of(h)?, g.get(h)?)
        }).collect();
        let garage = g.get_name("DISTRICT_GARAGE").and_then(|o| parse_district("DISTRICT_GARAGE", o)).map(|d| {
            districts.push(d);
            districts.len() - 1
        });
        log::info!("audio: ambience - {} districts", districts.len());
        Some(Ambience {
            districts,
            garage,
            cur: None,
            night: false,
            bed: None,
            timers: Vec::new(),
            quad_timers: Vec::new(),
            shots: Vec::new(),
            slots: Vec::new(),
            line: None,
            landmark_curve: joaat("MC4_HANGOUT_LANDMARK_APPLY"),
            null: joaat(NULL_SOUND),
            now: 0.0,
            rain: Rain {
                int_snd: be_u32(glob, 22),
                ext_snd: be_u32(glob, 26),
                thunder_snd: be_u32(glob, 30),
                thunder_gap: (be_f32(glob, 34), be_f32(glob, 38)),
                ramp: be_f32(glob, 58),
                covered_db: (be_u16(glob, 62) as i16 as f32 * 0.01, be_u16(glob, 66) as i16 as f32 * 0.01),
                cover_target: 1.0,
                cover: 1.0,
                // (+824 starts at rand(0, 20), sub_821EAD98 @821EAF8C: drawn in the first update)
                next_thunder: -1.0,
                ..Default::default()
            },
        })
    }

    /// The player car's cover probe (sub_82326A78, from the car audio update while wet): one wheel a frame, a line 40 m
    /// straight up (or back down); after every wheel the share of hits is the cover target. `probe(from, to)`.
    pub fn cover_probe(&mut self, wet: bool, wheels: &[Vec3], probe: &dyn Fn(Vec3, Vec3) -> bool) {
        let r = &mut self.rain;
        if !wet || wheels.is_empty() {
            // (dry: the wheel index goes back to 0, the hit count is kept - the original's quirk)
            r.probe_i = 0;
            return;
        }
        let i = r.probe_i.min(wheels.len() - 1);
        let p = wheels[i];
        let q = p + Vec3::new(0.0, 40.0, 0.0);
        if probe(p, q) || probe(q, p) {
            r.probe_hits += 1;
        }
        r.probe_i += 1;
        if r.probe_i >= wheels.len() {
            r.cover_target = r.probe_hits as f32 / wheels.len() as f32;
            r.probe_i = 0;
            r.probe_hits = 0;
        }
    }

    /// Rain and thunder (sub_821EB080 -> sub_821E8410): `wet` = the weather's wet amount is not 0, `cockpit` = audio camera
    /// mode 1 with a player, `covered` = the listener's probe 50 m up hit something.
    pub fn weather(&mut self, lib: &mut SoundLib, wet: bool, cockpit: bool, covered: bool, dt: f32) {
        let now = self.now;
        let level = |lib: &SoundLib, n: &str| lib.category(n).map_or(0.0, |c| c.volume);
        let r = &mut self.rain;
        if r.next_thunder < 0.0 {
            r.next_thunder = now + lib.rand_float(0.0, 20.0);
        }
        if level(lib, "AMBIENCE") <= -80.0 {
            // (sub_821E9C50: the rain bed's and the thunder's slots stop with everything else)
            for s in [&mut r.ext, &mut r.thunder] {
                if let Some(h) = s.take() {
                    lib.stop(h);
                }
            }
            return;
        }
        if level(lib, "AMBIENCE_BEDS") <= -80.0 {
            return;
        }
        if !wet {
            if let Some(h) = r.int.take() {
                lib.stop(h);
            }
            if r.raining {
                if let Some(h) = r.ext.take() {
                    lib.stop(h);
                }
            }
            r.raining = false;
            return;
        }
        if !r.raining {
            for s in [&mut r.int, &mut r.ext] {
                if let Some(h) = s.take() {
                    lib.stop(h);
                }
            }
            r.raining = true;
        }
        if r.ext.map_or(true, |h| !lib.playing(h)) {
            r.ext = lib.play_hash(r.ext_snd, Req::default());
        }
        if r.int.map_or(true, |h| !lib.playing(h)) {
            r.int = lib.play_hash(r.int_snd, Req::default().vol(-100.0));
        }
        // the weather categories by the district (+30 flag) or the listener's cover, ramped over AG+58
        let key = (self.cur, covered);
        if r.mix != Some(key) {
            let (rain_db, thunder_db) = match self.cur.and_then(|d| self.districts[d].weather_mix) {
                Some(m) => m,
                None if covered => r.covered_db,
                None => (0.0, 0.0),
            };
            lib.ramp_category("AMBIENCE_BEDS_WEATHER_RAIN", rain_db, r.ramp);
            lib.ramp_category("AMBIENCE_BEDS_WEATHER_THUNDER", thunder_db, r.ramp);
            r.mix = Some(key);
        }
        // the cover moves toward its target at 1 per second (G+92, I: unscaled dt)
        let step = dt;
        r.cover = if r.cover < r.cover_target { (r.cover + step).min(r.cover_target) } else { (r.cover - step).max(r.cover_target) };
        let (ext_db, int_db) = if cockpit {
            let open = 1.0 - r.cover;
            (-100.0, if open >= 1e-5 { mcla_formats::audcfg::lin2db(open) } else { -100.0 })
        } else {
            (0.0, -100.0)
        };
        if let Some(h) = r.ext {
            lib.set(h, Req::default().vol(ext_db));
        }
        if let Some(h) = r.int {
            lib.set(h, Req::default().vol(int_db));
        }
        if now >= r.next_thunder && r.thunder.map_or(true, |h| !lib.playing(h)) {
            r.thunder = lib.play_hash(r.thunder_snd, Req::default());
            r.next_thunder = now + lib.rand_float(r.thunder_gap.0, r.thunder_gap.1);
        }
    }

    /// The district the listener is in (sub_821EB810): the lowest squared distance over weight of all circles.
    fn pick(&self, l: Vec3) -> Option<usize> {
        let mut best = None;
        for (i, d) in self.districts.iter().enumerate() {
            for &(x, z, w) in &d.circles {
                let s = ((x - l.x).powi(2) + (z - l.z).powi(2)) / w;
                if best.map_or(true, |(b, _)| s < b) {
                    best = Some((s, i));
                }
            }
        }
        best.map(|b| b.1)
    }

    pub fn district_name(&self) -> Option<&str> {
        self.cur.map(|i| self.districts[i].name.as_str())
    }

    /// Stop everything (out of the city: menus without the garage, cutscenes ...).
    pub fn stop(&mut self, lib: &mut SoundLib) {
        for h in self.bed.take().into_iter().chain(self.line.take()) {
            lib.stop(h);
        }
        for s in self.slots.drain(..) {
            lib.stop(s.sound);
            if let Some(c) = s.companion {
                lib.stop(c);
            }
        }
        self.cur = None;
    }

    pub fn update(&mut self, lib: &mut SoundLib, listener: Vec3, night: bool, in_garage: bool, dt: f32) {
        self.now += dt;
        let now = self.now;
        let want = if in_garage { self.garage } else { self.pick(listener) };
        let Some(di) = want else {
            self.stop(lib);
            return;
        };
        let changed = self.cur != Some(di);
        if changed || night != self.night {
            // the bed of the district for the time of day (+10 / +14)
            if let Some(h) = self.bed.take() {
                lib.stop(h);
            }
            let snd = self.districts[di].bed[night as usize];
            if snd != self.null {
                self.bed = lib.play_hash(snd, Req::default());
                log::info!("audio: ambience bed {} {}", lib.name_of(snd).unwrap_or("?"), if self.bed.is_some() { "playing" } else { "missing" });
            }
        }
        if changed {
            let d = &self.districts[di];
            log::info!("audio: ambience district {}", d.name);
            // (the timers' start on entering a district is not decoded: each waits one random interval)
            let (shots, quads) = (d.shots.clone(), d.quads.clone());
            self.timers = shots.iter().map(|s| now + lib.rand_float(s.min, s.max)).collect();
            self.quad_timers = quads.iter().map(|s| now + lib.rand_float(s.min, s.max)).collect();
            if let Some(h) = self.line.take() {
                lib.stop(h);
            }
            self.cur = Some(di);
        }
        self.night = night;
        let d = self.districts[di].clone();
        // distant one-shots and quadshots (sub_821E8B18)
        self.shots.retain(|h| lib.playing(*h));
        for (list, timers) in [(&d.shots, 0usize), (&d.quads, 1)] {
            for (k, s) in list.iter().enumerate() {
                let t = if timers == 0 { &mut self.timers[k] } else { &mut self.quad_timers[k] };
                if now < *t {
                    continue;
                }
                *t = now + lib.rand_float(s.min, s.max);
                if self.shots.len() >= SHOT_SLOTS {
                    continue;
                }
                if s.flags & if night { 2 } else { 1 } == 0 {
                    continue;
                }
                let angle = match s.centre {
                    Some(c) => (lib.rand_float(-s.spread, s.spread) + c).to_radians(),
                    None => lib.rand_float(-std::f32::consts::PI, std::f32::consts::PI),
                };
                let pos = listener + Vec3::new(-6000.0 * angle.sin(), 0.0, -6000.0 * angle.cos());
                if let Some(h) = lib.play_hash(s.sound, Req { pos: Some(pos), volume: s.volume, ..Default::default() }) {
                    self.shots.push(h);
                }
            }
        }
        // location emitters (sub_821EA288 -> sub_821E9390): the two nearest inside their outer radius
        let mut cands: Vec<(f32, usize)> = Vec::new();
        for (k, l) in d.locs.iter().enumerate() {
            // (flag bit 0 takes another path, gated by manager +1028 / +1032: not decoded, none in the data)
            if l.flags & 1 != 0 {
                continue;
            }
            let snd = l.sounds[if night { 2 } else { 0 }];
            if snd == self.null || snd == 0 {
                continue;
            }
            let d2 = l.pos.distance_squared(listener);
            if d2 <= l.outer * l.outer {
                cands.push((d2, k));
            }
        }
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        cands.truncate(2);
        // slots whose emitter is no longer among the two stop
        self.slots.retain(|s| {
            let keep = s.key.0 == di && cands.iter().any(|c| c.1 == s.key.1) && lib.playing(s.sound);
            if !keep {
                lib.stop(s.sound);
                if let Some(c) = s.companion {
                    lib.stop(c);
                }
            }
            keep
        });
        for (d2, k) in cands {
            let l = d.locs[k];
            let dist = d2.sqrt();
            // (sub_821E9390 @821E9588: dist raised to the inner radius, then lowered to the outer one, both by fsel, over
            // outer - inner with no guard - an emitter with equal radii (342 of 409) gives 0 / 0 = NaN, which the
            // LANDMARK curve turns into 0, so it plays at -100 dB; one with the inner radius above the outer gives 1)
            let t = landmark_t(dist, l.inner, l.outer);
            let b = lib.curve_eval(self.landmark_curve, t).unwrap_or(1.0 - t);
            // (@821E9644: 20 x 0.301 x log2(b) when b >= 1e-5, else -100 dB - the constants at 0x820188E0)
            let vol = fsel(b - 1e-5, 20.0 * (b.log2() * 0.301), -100.0);
            let req = Req { pos: Some(l.pos), volume: vol, ..Default::default() };
            match self.slots.iter().find(|s| s.key == (di, k)) {
                Some(s) => {
                    lib.set(s.sound, req);
                    if let Some(c) = s.companion {
                        lib.set(c, req);
                    }
                }
                None => {
                    let n = night as usize * 2;
                    if let Some(h) = lib.play_hash(l.sounds[n], req) {
                        let companion = (l.sounds[n + 1] != self.null).then(|| lib.play_hash(l.sounds[n + 1], req)).flatten();
                        self.slots.push(Slot { key: (di, k), sound: h, companion });
                    }
                }
            }
        }
        // the line emitter (sub_821E9068): the polyline's nearest point, within +39 m
        if d.line.len() >= 2 && d.line_sound != self.null {
            let mut best = (f32::MAX, Vec3::ZERO);
            for w in d.line.windows(2) {
                let ab = w[1] - w[0];
                let t = ((listener - w[0]).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let p = w[0] + ab * t;
                let dd = p.distance(listener);
                if dd < best.0 {
                    best = (dd, p);
                }
            }
            if best.0 <= d.line_dist {
                let req = Req::at(best.1);
                match self.line {
                    Some(h) if lib.playing(h) => lib.set(h, req),
                    _ => self.line = lib.play_hash(d.line_sound, req),
                }
            } else if let Some(h) = self.line.take() {
                lib.stop(h);
            }
        }
    }
}

/// The location emitter's position between its radii (sub_821E9390 @821E9588..821E95B0), as the original computes it.
fn landmark_t(dist: f32, inner: f32, outer: f32) -> f32 {
    let x = fsel(dist - inner, dist, inner);
    let x = fsel(x - outer, outer, x);
    (x - inner) / (outer - inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The data's odd radii never panic: equal radii give NaN (silent through the LANDMARK curve), an inner radius
    /// above the outer gives 1 (DISTRICT_BUSINESS_OLD 4: 50 / 49.94).
    #[test]
    fn landmark_radii() {
        assert!((landmark_t(30.0, 10.0, 50.0) - 0.5).abs() < 1e-6);
        assert_eq!(landmark_t(5.0, 10.0, 50.0), 0.0);
        assert!(landmark_t(20.0, 50.0, 50.0).is_nan());
        assert_eq!(landmark_t(40.0, 50.0, 49.94), 1.0);
        assert_eq!(landmark_t(20.0, 50.0, 25.0), 1.0);
    }

    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn districts_from_disc() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let amb = Ambience::new(&lib).expect("ambience");
        assert_eq!(amb.districts.len(), 17);
        let skid = amb.districts.iter().find(|d| d.name == "DISTRICT_SKID_ROW").unwrap();
        assert_eq!(skid.circles.len(), 3);
        assert_eq!(skid.shots.len(), 15);
        assert_eq!(skid.quads.len(), 2);
        assert_eq!(skid.locs.len(), 10);
        assert_eq!(skid.locs[0].outer, 66.0);
        assert_eq!(lib.name_of(skid.bed[0]), Some("BED_SANTA_MONICA_CITY_MASTER"));
        // the circles' centres pick their own district
        for (i, d) in amb.districts.iter().enumerate() {
            for &(x, z, _) in &d.circles {
                assert_eq!(amb.pick(Vec3::new(x, 0.0, z)), Some(i), "{}", d.name);
            }
        }
        let sm = amb.districts.iter().find(|d| d.name == "DISTRICT_SANTA_MONICA").unwrap();
        assert_eq!(sm.line.len(), 7);
        // every location's t is a number or the equal-radii NaN, and the LANDMARK curve maps NaN to 0 (silent), t 0 to 1
        let bo = amb.districts.iter().find(|d| d.name == "DISTRICT_BUSINESS_OLD").unwrap();
        assert!(bo.locs[4].inner > bo.locs[4].outer);
        for d in &amb.districts {
            for l in &d.locs {
                for dist in [0.0, l.inner * 0.5, l.outer] {
                    let t = landmark_t(dist, l.inner, l.outer);
                    assert!(t.is_nan() == (l.inner == l.outer), "{} {:?} {t}", d.name, (l.inner, l.outer));
                }
            }
        }
        let c = amb.landmark_curve;
        assert_eq!(lib.curve_eval(c, f32::NAN), Some(0.0));
        assert!((lib.curve_eval(c, 0.0).unwrap() - 1.0).abs() < 1e-6);
    }
}
