//! The original's district ambience (the ambience manager, F-80049 and friends): the district whose circles
//! are nearest picks the bed (day / night), its distant one-shots and surround "quadshots" fire on random timers
//! 6 km away in random directions, the two nearest location emitters play at their places, faded by distance, and
//! a district's line emitter follows the listener along its polyline. The freeway / tunnel / interior / mall
//! districts come from the ground's material (the listener's probe), the garage's and a hangout's first; a district switch
//! starts its dynamic mixer scene and second bed. The hangout cores' landmark sets (HNG_LM_*) join the location emitters,
//! and the nearest of them drives DYNAMIC_MIXER_POSITIONAL_MUSIC. Rules: `docs/reference/audio/AMBIENCE_RULE.md`.
//!
//! The two slots' occluders (records with flag 0x2, Downtown's local hangout): `SlotOccluders`, `occlusion::System`.
//! Not ported (open there): the district cross-fade. Rain and thunder (here too): `docs/reference/audio/WEATHER_AUDIO_RULE.md`.

use crate::sndobj::{Handle, Req, SoundLib};
use glam::Vec3;
use mcla_formats::audcfg::{fsel, lin2db};
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
    /// the second bed by day / night (+18 / +22: DISTRICT_MALL's, DISTRICT_GARAGE's, the HNG_DIS_* crowd; NULL elsewhere)
    bed2: [u32; 2],
    /// +26: the district's dynamic mixer scene (DYNAMIC_MIXER_FREEWAY, _GARAGE, HNG_DIS_*'s _HANGOUT; 0 in the rest)
    scene: u32,
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
    let locs = (0..nd).map(|k| parse_loc(o, p + 41 * k)).collect();
    p += 41 * nd;
    let ne = o.get(p).copied().unwrap_or(0) as usize;
    p += 1;
    let line = (0..ne).map(|k| Vec3::new(be_f32(o, p + 12 * k), be_f32(o, p + 12 * k + 4), be_f32(o, p + 12 * k + 8))).collect();
    let [shots, quads] = shots;
    let weather_mix = (o.get(30).copied().unwrap_or(0) & 1 != 0).then(|| (be_u16(o, 31) as i16 as f32 * 0.01, be_u16(o, 35) as i16 as f32 * 0.01));
    Some(District { name: name.to_string(), bed: [be_u32(o, 10), be_u32(o, 14)], bed2: [be_u32(o, 18), be_u32(o, 22)], scene: be_u32(o, 26), line_dist: be_f32(o, 39), line_sound: be_u32(o, 43), circles, shots, quads, locs, line, weather_mix })
}

/// A location emitter's record: (district, list D record) or (landmark set by its id's hash, record).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Key {
    Loc(usize, usize),
    Lm(u32, usize),
}

/// A hangout's landmark set (F-24802, the script native AudioManager_InitHangout(id, state name, &entry line end)): the
/// game.dat objects `HNG_LM_<id>` and `HNG_LM_<id>_0` .. `_9` (type 6, up to the first missing; none without the first),
/// each a location record at +10; an object whose position (+11 / +15 / +19) is all 0 takes the argument's. Set `disabled`
/// (+264) by AudioManager_SuspendHangout (F-27681), dropped by AudioManager_DestroyHangout (F-85547). Kept in the
/// manager's +196 list in the order registered.
struct LmSet {
    id: u32,
    recs: Vec<(Vec3, Loc)>,
    disabled: bool,
}

fn parse_loc(o: &[u8], e: usize) -> Loc {
    Loc {
        flags: o[e],
        pos: Vec3::new(be_f32(o, e + 1), be_f32(o, e + 5), be_f32(o, e + 9)),
        outer: be_f32(o, e + 17),
        inner: be_f32(o, e + 21),
        sounds: [be_u32(o, e + 25), be_u32(o, e + 29), be_u32(o, e + 33), be_u32(o, e + 37)],
    }
}

fn load_lm_set(g: &mcla_formats::audcfg::Dat, id: &str, at: Vec3) -> Option<LmSet> {
    let rec = |o: &[u8]| {
        let l = (o.len() >= 51).then(|| parse_loc(o, 10))?;
        let pos = if l.pos == Vec3::ZERO { at } else { l.pos };
        Some((pos, l))
    };
    let mut recs = vec![rec(g.get_name(&format!("HNG_LM_{id}"))?)?];
    for i in 0..10 {
        let Some(r) = g.get_name(&format!("HNG_LM_{id}_{i}")).and_then(rec) else { break };
        recs.push(r);
    }
    Some(LmSet { id: joaat(id), recs, disabled: false })
}

struct Slot {
    key: Key,
    sound: Handle,
    companion: Option<Handle>,
    /// which of the manager's two slots (+156 + 20 i) it is: the slot's own occluder (+16)
    at: usize,
}

/// The ambience's two slot occluders as F-80314 and the slot stops use them (`occlusion::System`'s `amb_*`).
pub trait SlotOccluders {
    /// slot `k`'s occluder at `p`; its table slot + 1 for the requests (+32), 0 when there is none
    fn place(&mut self, k: usize, p: Vec3) -> u16;
    /// +119 clear
    fn updated(&self, k: usize) -> bool;
    /// the immediate update F-65914 (now, not local) while "not updated yet"
    fn now(&mut self, k: usize);
    /// F-65900 when updated
    fn reset(&mut self, k: usize);
}

/// No occluders (the offline tests, or GLOBAL_OCCLUSION missing): no record takes the flag 0x2 path's switch.
pub struct NoOccluders;

impl SlotOccluders for NoOccluders {
    fn place(&mut self, _: usize, _: Vec3) -> u16 {
        0
    }
    fn updated(&self, _: usize) -> bool {
        false
    }
    fn now(&mut self, _: usize) {}
    fn reset(&mut self, _: usize) {}
}

pub struct Ambience {
    districts: Vec<District>,
    garage: Option<usize>,
    cur: Option<usize>,
    night: bool,
    bed: Option<Handle>,
    /// the beds' sounds now (mgr +272 / +276: NULL when none started) and the second bed (slot +268)
    bed_snd: u32,
    bed2_snd: u32,
    bed2: Option<Handle>,
    /// the last district set (mgr +140, kept across `stop`) and the running district scene (slot +304)
    last: Option<usize>,
    /// mgr +828: no district switch before this time (each switch sets it 4 s on, F-93563); seconds on `now`
    hold: f32,
    /// the garage district is in (mgr +152 set by F-71902)
    garage_on: bool,
    scene: Option<u32>,
    timers: Vec<f32>,
    quad_timers: Vec<f32>,
    shots: Vec<Handle>,
    /// the quadshots' slots (mgr +296 / +300, F-73296), apart from the one-shots' (+288 / +292, F-57118)
    quad_shots: Vec<Handle>,
    slots: Vec<Slot>,
    line: Option<Handle>,
    landmark_curve: u32,
    /// MC4_HANGOUT_DYNMIXER_APPLY (manager +944): DYNAMIC_MIXER_POSITIONAL_MUSIC's strength by the emitter's place
    dynmixer_curve: u32,
    /// the hangout cores' landmark sets (+196)
    sets: Vec<LmSet>,
    /// the hangout state (F-35228 / F-19354): byte +1077 with the id's hash, and +152 = HNG_DIS_<id> (None: neither it
    /// nor the fallback DISTRICT_HANGOUT is in game.dat)
    hangout: Option<(u32, Option<usize>)>,
    /// mixer slot 11 (DYNAMIC_MIXER_POSITIONAL_MUSIC) on: mixer +1740 bit 0x800
    pos_music: bool,
    null: u32,
    now: f32,
    rain: Rain,
    /// the speed fade (F-54370): AMBIENCE_GLOBAL +10 / +14 the speeds (mph), +50 / +54 the curves (manager +864 / +904),
    /// +18 WIND_COMP_MASTER and its slot (+832)
    fade_mph: (f32, f32),
    fade_curves: (u32, u32),
    wind_snd: u32,
    wind: Option<Handle>,
}

/// The rain and thunder of the ambience manager (F-94235; `reference/audio/WEATHER_AUDIO_RULE.md`).
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

/// The distant one-shots' slots (mgr +292, F-57118) and the quadshots' (+300, F-73296): F-20021 sizes them by the wave slot
/// pools AMBIENCE_ONESHOT and AMBIENCE_STREAM, which F-22027 counts in waveslots.xml through F-97061 (`%s_%.2i` from _00, else from _01,
/// while the next one exists): AMBIENCE_ONESHOT_01..04 and AMBIENCE_STREAM_01..04 (test `shot_pools_match_the_wave_slots`).
const SHOT_SLOTS: usize = 4;
const QUAD_SLOTS: usize = 4;

/// MANAGER_MIXER entry 11, the slot the hangout landmarks drive
const POS_MUSIC: &str = "DYNAMIC_MIXER_POSITIONAL_MUSIC";

/// A distant one-shot's (list B) or quadshot's (list C) offset from the listener at `angle` (F-14965).
fn shot_offset(quad: bool, angle: f32) -> Vec3 {
    crate::aiv::rot_y(Vec3::new(0.0, 0.0, if quad { 6000.0 } else { -6000.0 }), angle)
}

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
            bed_snd: joaat(NULL_SOUND),
            bed2_snd: joaat(NULL_SOUND),
            bed2: None,
            last: None,
            hold: 0.0,
            garage_on: false,
            scene: None,
            timers: Vec::new(),
            quad_timers: Vec::new(),
            shots: Vec::new(),
            quad_shots: Vec::new(),
            slots: Vec::new(),
            line: None,
            landmark_curve: joaat("MC4_HANGOUT_LANDMARK_APPLY"),
            dynmixer_curve: joaat("MC4_HANGOUT_DYNMIXER_APPLY"),
            sets: Vec::new(),
            hangout: None,
            pos_music: false,
            null: joaat(NULL_SOUND),
            now: 0.0,
            fade_mph: (be_f32(glob, 10), be_f32(glob, 14)),
            fade_curves: (be_u32(glob, 50), be_u32(glob, 54)),
            wind_snd: be_u32(glob, 18),
            wind: None,
            rain: Rain {
                int_snd: be_u32(glob, 22),
                ext_snd: be_u32(glob, 26),
                thunder_snd: be_u32(glob, 30),
                thunder_gap: (be_f32(glob, 34), be_f32(glob, 38)),
                ramp: be_f32(glob, 58),
                covered_db: (be_u16(glob, 62) as i16 as f32 * 0.01, be_u16(glob, 66) as i16 as f32 * 0.01),
                cover_target: 1.0,
                cover: 1.0,
                // (+824 starts at rand(0, 20), F-20021 @A-56478: drawn in the first update)
                next_thunder: -1.0,
                ..Default::default()
            },
        })
    }

    /// The player car's cover probe (F-83532, from the car audio update while wet): one wheel a frame, a line 40 m
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

    /// Rain and thunder (F-99455 -> F-94235): `wet` = the weather's wet amount is not 0, `cockpit` = audio camera
    /// mode 1 with a player, `covered` = the listener's probe 50 m up hit something.
    pub fn weather(&mut self, lib: &mut SoundLib, wet: bool, cockpit: bool, covered: bool, dt: f32) {
        let now = self.now;
        let level = |lib: &SoundLib, n: &str| lib.category(n).map_or(0.0, |c| c.volume);
        let r = &mut self.rain;
        if r.next_thunder < 0.0 {
            r.next_thunder = now + lib.rand_float(0.0, 20.0);
        }
        if level(lib, "AMBIENCE") <= -80.0 {
            // (F-79099: the rain bed's and the thunder's slots stop with everything else)
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

    /// The district the listener is in (F-80049): the lowest squared distance over weight of all circles.
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

    /// A district object by its name's hash, parsed the first time it is wanted.
    fn district_of(&mut self, lib: &SoundLib, h: u32) -> Option<usize> {
        let name = lib.game.name_of(h)?.to_string();
        match self.districts.iter().position(|d| d.name == name) {
            Some(i) => Some(i),
            None => {
                let d = parse_district(&name, lib.game.get(h)?)?;
                self.districts.push(d);
                Some(self.districts.len() - 1)
            }
        }
    }

    /// The hangout cores' audio natives, from the cores the port runs. `running`: each core's location id, its entry
    /// line's end (block +40) and whether it holds (Script_SuspendAllOpponents): a core's start registers its landmark
    /// set (AudioManager_InitHangout, F-24802), its end drops it (AudioManager_DestroyHangout, F-85547), its hold
    /// disables it (AudioManager_SuspendHangout(id, 1 / 0), F-27681: the local core 0x552 / 0x737, the wager core
    /// 0x7ff / 0x958). `hangout`: the hangout entered, AudioManager_EnterHangoutState(<location id>) (F-35228: +152 =
    /// game.dat `HNG_DIS_<id>`, else DISTRICT_HANGOUT; +1077 = 1; +308 = F-13171), None after ExitHangoutState (F-19354:
    /// +152 = 0, +1077 = 0, +308 = F-80049).
    pub fn hangouts(&mut self, lib: &SoundLib, running: &[(&str, Vec3, bool)], hangout: Option<&str>) {
        self.sets.retain(|s| running.iter().any(|r| joaat(r.0) == s.id));
        for &(id, at, held) in running {
            let h = joaat(id);
            match self.sets.iter_mut().find(|s| s.id == h) {
                Some(s) => s.disabled = held,
                None => {
                    if let Some(mut s) = load_lm_set(&lib.game, id, at) {
                        log::info!("audio: hangout landmarks {id} ({} records)", s.recs.len());
                        s.disabled = held;
                        self.sets.push(s);
                    }
                }
            }
        }
        let want = hangout.map(joaat);
        if want != self.hangout.map(|x| x.0) {
            self.hangout = hangout.map(|id| {
                let d = self.district_of(lib, joaat(&format!("HNG_DIS_{id}"))).or_else(|| self.district_of(lib, joaat("DISTRICT_HANGOUT")));
                log::info!("audio: hangout state {id} district {}", d.map_or("-", |d| self.districts[d].name.as_str()));
                (joaat(id), d)
            });
        }
    }

    /// Stop everything (out of the city: menus without the garage, cutscenes ...).
    /// The speed fade (F-54370, last in the frame's update F-99455): the player's speed in mph (chassis +224 x 2.237, 0
    /// while the game is held) between +10 and +14 picks the AMBIENCE category's level on the FALL curve and WIND_COMP's on
    /// the RISE curve (F-77479: WIND_COMP_MASTER started at that level, stopped at -100 dB); below the first speed
    /// the ambience is at 0 dB and the wind off, above the second the other way round.
    pub fn speed_fade(&mut self, lib: &mut SoundLib, mph: f32) {
        let Some((amb_db, wind_db)) = self.fade_db(lib, mph) else { return };
        lib.set_speed_fade(amb_db);
        if wind_db <= -100.0 {
            if let Some(h) = self.wind.take() {
                lib.stop(h);
            }
        } else {
            let req = Req { volume: wind_db, ..Default::default() };
            match self.wind {
                Some(h) if lib.playing(h) => lib.set(h, req),
                _ => self.wind = lib.play_hash(self.wind_snd, req),
            }
        }
    }

    /// The speed fade's two levels (dB): the AMBIENCE controller's and WIND_COMP's.
    fn fade_db(&self, lib: &SoundLib, mph: f32) -> Option<(f32, f32)> {
        let (lo, hi) = self.fade_mph;
        let (fall, rise) = if mph <= lo {
            (1.0, 0.0)
        } else if mph >= hi {
            (0.0, 1.0)
        } else {
            let t = (mph - lo) / (hi - lo);
            (lib.curve_eval(self.fade_curves.0, t)?, lib.curve_eval(self.fade_curves.1, t)?)
        };
        Some((lin2db(fall), lin2db(rise)))
    }

    pub fn stop(&mut self, lib: &mut SoundLib) {
        self.stop_pos_music(lib);
        self.bed_snd = self.null;
        self.bed2_snd = self.null;
        for h in self.bed.take().into_iter().chain(self.bed2.take()).chain(self.line.take()) {
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

    /// Mixer slot 11 off with its exit (F-10645 / F-89955 / F-80314: StopSlot(11, 1) while mixer +1740 bit 0x800 is set).
    fn stop_pos_music(&mut self, lib: &mut SoundLib) {
        if self.pos_music {
            lib.scene(POS_MUSIC, false);
            self.pos_music = false;
            log::info!("audio: positional music off");
        }
    }

    /// `forced`: the forced district (manager +144) the listener's probe set from the ground's material (F-26357:
    /// DISTRICT_FREEWAYS, _INTERIOR, _MALL, _TUNNEL), else the district by the circles (F-80049); in the garage or a
    /// hangout (`hangouts`) their district, +152 (F-13171). `mph`: the player's speed (mixer +1748, F-99947: chassis
    /// +224 x 2.237); `racing`: DYNAMIC_MIXER_RACE runs (mixer +1740 bit 0x4000, a race under way).
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, lib: &mut SoundLib, listener: Vec3, night: bool, in_garage: bool, forced: Option<u32>, mph: f32, racing: bool, dt: f32, occ: &mut dyn SlotOccluders) {
        self.update_slots(lib, listener, night, in_garage, forced, mph, racing, dt, occ);
        // the slot stops (F-10645, and F-10322 / F-12897 for a slot not among the two nearest, an empty one too):
        // each slot holding no emitter has its occluder reset when it has been updated
        for k in 0..2 {
            if !self.slots.iter().any(|s| s.at == k) {
                occ.reset(k);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update_slots(&mut self, lib: &mut SoundLib, listener: Vec3, night: bool, in_garage: bool, forced: Option<u32>, mph: f32, racing: bool, dt: f32, occ: &mut dyn SlotOccluders) {
        self.now += dt;
        let now = self.now;
        let forced = forced.and_then(|h| self.district_of(lib, h));
        if in_garage && !self.garage_on {
            // F-71902 (garage enter) calls F-10645(mgr, -1): the location slots stop with their own release, slot 11 off
            for sl in self.slots.drain(..) {
                lib.stop(sl.sound);
                if let Some(c) = sl.companion {
                    lib.stop(c);
                }
            }
            self.stop_pos_music(lib);
        }
        self.garage_on = in_garage;
        // (+152 null: F-93563 returns at once and the district stays)
        let want = match self.hangout {
            _ if in_garage => self.garage,
            Some((_, d)) => d.or(self.cur),
            None => forced.or_else(|| self.pick(listener)),
        };
        let Some(di) = want else {
            self.stop(lib);
            return;
        };
        // F-93563 returns at once while the same district stays (and the night flag is unchanged: byte +1073), and
        // while the hold +828 runs: the old district then plays on and the switch is tried again next frame
        // (the port's own stop - no player - clears the district; the next one then starts at once)
        // byte +1072 is the night flag and +1073 its change this frame, both written every frame (F-99455), so a night
        // change that falls in the hold is not switched later
        let night_changed = night != self.night;
        self.night = night;
        let switch = (self.cur != Some(di) || night_changed) && (now >= self.hold || self.cur.is_none());
        let di = if switch { di } else { self.cur.unwrap_or(di) };
        let changed = switch && self.cur != Some(di);
        if changed {
            // the district's dynamic mixer scene (F-93563, slot +304): the running one stops (with its exit) when the
            // last district's +26 differs; then, with none running, the new district's +26 starts at 1.0 (F-67134)
            let prev = self.last.map(|l| self.districts[l].scene);
            let want = self.districts[di].scene;
            if let (Some(s), Some(p)) = (self.scene, prev) {
                if p != want {
                    if let Some(n) = lib.game.name_of(s).map(|n| n.to_string()) {
                        lib.scene(&n, false);
                        log::info!("audio: district scene {n} off");
                    }
                    self.scene = None;
                }
            }
            if self.scene.is_none() && want != 0 {
                if let Some(n) = lib.game.name_of(want).map(|n| n.to_string()) {
                    lib.scene(&n, true);
                    log::info!("audio: district scene {n} on");
                    self.scene = Some(want);
                }
            }
            self.last = Some(di);
        }
        if switch {
            // the beds for the time of day (F-93563): the main one (+10 / +14) started at a random 0 - 90 % of its waves
            // (request +36 with +70 bit 0x40), the second one (+18 / +22) with a default request after a random draw of
            // 0 - 90000 (unused); each restarts only when its sound differs from the one playing (mgr +272 / +276)
            let snd = self.districts[di].bed[night as usize];
            if snd != self.bed_snd {
                let pct = lib.rand_int(0, 90) as f32;
                // the old bed stops with the release override AMBIENCE_GLOBAL +58 x 1000 (fctiwz) + 1000 ms (entry +192)
                if let Some(h) = self.bed.take() {
                    lib.stop_release(h, (self.rain.ramp * 1000.0) as i32 + 1000);
                }
                self.bed = if snd != self.null { lib.play_hash(snd, Req { start_pct: pct, ..Default::default() }) } else { None };
                self.bed_snd = if self.bed.is_some() { snd } else { self.null };
                log::info!("audio: ambience bed {} {}", lib.name_of(snd).unwrap_or("?"), if self.bed.is_some() { "playing" } else { "missing" });
            }
            let snd2 = self.districts[di].bed2[night as usize];
            if snd2 != self.bed2_snd {
                lib.rand_int(0, 90000);
                if let Some(h) = self.bed2.take() {
                    lib.stop(h);
                }
                self.bed2 = if snd2 != self.null { lib.play_hash(snd2, Req::default()) } else { None };
                self.bed2_snd = if self.bed2.is_some() { snd2 } else { self.null };
                if self.bed2.is_some() {
                    log::info!("audio: ambience second bed {}", lib.name_of(snd2).unwrap_or("?"));
                }
            }
        }
        if switch {
            // the playing one-shots stop with a 750 ms release (F-61925 lists +288 / +296), the next switch waits 4 s
            // (+828 = now + 4000 ms); then F-20727 sets every one-shot's and quadshot's timer to now + random(0, its
            // max interval)
            for h in self.shots.drain(..).chain(self.quad_shots.drain(..)) {
                lib.stop_release(h, 750);
            }
            self.hold = now + 4.0;
            let d = &self.districts[di];
            let (shots, quads) = (d.shots.clone(), d.quads.clone());
            self.timers = shots.iter().map(|s| now + lib.rand_float(0.0, s.max)).collect();
            self.quad_timers = quads.iter().map(|s| now + lib.rand_float(0.0, s.max)).collect();
        }
        if changed {
            log::info!("audio: ambience district {}", self.districts[di].name);
            if let Some(h) = self.line.take() {
                lib.stop(h);
            }
            self.cur = Some(di);
        }
        // F-99455 after the district: the categories' resolved volumes gate the rest (-80 dB at 0x82018A50)
        let level = |lib: &SoundLib, n: &str| lib.category(n).map_or(0.0, |c| c.volume);
        if level(lib, "AMBIENCE") <= -80.0 {
            // everything stops: the location slots with a 0 ms release override and slot 11 off (F-10645(mgr, 0)), the
            // beds (F-91693), rain and thunder (F-79099, in `weather`) and the one-shots (F-61925) with -1 = their own
            // release; the line emitter and the location emitters are not updated (the line sound plays on)
            for sl in self.slots.drain(..) {
                lib.stop_release(sl.sound, 0);
                if let Some(c) = sl.companion {
                    lib.stop_release(c, 0);
                }
            }
            self.stop_pos_music(lib);
            for h in self.bed.take().into_iter().chain(self.bed2.take()).chain(self.shots.drain(..)).chain(self.quad_shots.drain(..)) {
                lib.stop(h);
            }
            return;
        }
        if level(lib, "AMBIENCE_BEDS") > -80.0 {
            // an empty bed slot (stopped above, or ended) starts the district's bed for the time of day again: the
            // main one with request flag 0x20 only (no random start), the second with a default request
            let night = self.night;
            let beds = self.districts[di].clone();
            if self.bed.map_or(true, |h| !lib.playing(h)) {
                let snd = beds.bed[night as usize];
                self.bed = if snd != self.null { lib.play_hash(snd, Req::default()) } else { None };
                self.bed_snd = if self.bed.is_some() { snd } else { self.null };
            }
            if self.bed2.map_or(true, |h| !lib.playing(h)) {
                let snd2 = beds.bed2[night as usize];
                self.bed2 = if snd2 != self.null { lib.play_hash(snd2, Req::default()) } else { None };
                self.bed2_snd = if self.bed2.is_some() { snd2 } else { self.null };
            }
        } else {
            // F-91693(mgr, -1): both beds stop with their own release
            for h in self.bed.take().into_iter().chain(self.bed2.take()) {
                lib.stop(h);
            }
        }
        let d = self.districts[di].clone();
        // distant one-shots and quadshots (F-14965), while AMBIENCE_ONESHOTS is above -80 dB; else they stop with their
        // own release (F-61925(mgr, -1)) and the timers wait
        self.shots.retain(|h| lib.playing(*h));
        self.quad_shots.retain(|h| lib.playing(*h));
        let shots_on = level(lib, "AMBIENCE_ONESHOTS") > -80.0;
        if !shots_on {
            for h in self.shots.drain(..).chain(self.quad_shots.drain(..)) {
                lib.stop(h);
            }
        }
        for (list, timers) in [(&d.shots, 0usize), (&d.quads, 1)].into_iter().filter(|_| shots_on) {
            for (k, s) in list.iter().enumerate() {
                let t = if timers == 0 { &mut self.timers[k] } else { &mut self.quad_timers[k] };
                if now < *t {
                    continue;
                }
                *t = now + lib.rand_float(s.min, s.max);
                // a free slot of the list's own pool (F-57118 / F-73296); none -> this one is skipped
                if if timers == 0 { self.shots.len() >= SHOT_SLOTS } else { self.quad_shots.len() >= QUAD_SLOTS } {
                    continue;
                }
                if s.flags & if night { 2 } else { 1 } == 0 {
                    continue;
                }
                let angle = match s.centre {
                    Some(c) => (lib.rand_float(-s.spread, s.spread) + c).to_radians(),
                    None => lib.rand_float(-std::f32::consts::PI, std::f32::consts::PI),
                };
                // F-14965: list B (0, 0, -6000) through F-57591, list C (0, 0, +6000) rotated inline by the same sin / cos
                let pos = listener + shot_offset(timers == 1, angle);
                if let Some(h) = lib.play_hash(s.sound, Req { pos: Some(pos), volume: s.volume, ..Default::default() }) {
                    if timers == 0 { self.shots.push(h) } else { self.quad_shots.push(h) }
                }
            }
        }
        // location emitters (F-89955 -> F-80314): the two nearest inside their outer radius (3D), first over the hangout
        // cores' landmark sets (+196, not a disabled one; in a hangout - byte +1077 - only their records with flag 0x4),
        // then over the district's list D; a record whose sound for the time of day is NULL counts as outside
        let mut cands: Vec<(f32, Key, Vec3, Loc)> = Vec::new();
        let null = self.null;
        let silent = |l: &Loc| {
            let s = l.sounds[if night { 2 } else { 0 }];
            s == null || s == 0
        };
        for set in self.sets.iter().filter(|s| !s.disabled) {
            for (k, (pos, l)) in set.recs.iter().enumerate() {
                if self.hangout.is_some() && l.flags & 4 == 0 {
                    continue;
                }
                // (flag bit 0, the clock tower's chimes: in no record of the data)
                if l.flags & 1 != 0 || silent(l) {
                    continue;
                }
                let d2 = pos.distance_squared(listener);
                if d2 <= l.outer * l.outer {
                    cands.push((d2, Key::Lm(set.id, k), *pos, *l));
                }
            }
        }
        for (k, l) in d.locs.iter().enumerate() {
            // (flag bit 0 takes the clock tower's path, gated by manager +1028 / +1032: none in the data)
            if l.flags & 1 != 0 || silent(l) {
                continue;
            }
            let d2 = l.pos.distance_squared(listener);
            if d2 <= l.outer * l.outer {
                cands.push((d2, Key::Loc(di, k), l.pos, *l));
            }
        }
        // (only a strictly nearer one replaces: on equal distances the earlier record - a set's - stays first)
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        cands.truncate(2);
        // the positional-music path (F-89955): the nearest is a landmark set's record, DYNAMIC_MIXER_RACE is off and the
        // player is below 55 mph; otherwise slot 11 stops
        let pm = matches!(cands.first(), Some((_, Key::Lm(..), _, _))) && !racing && mph < 55.0;
        if !pm {
            self.stop_pos_music(lib);
        }
        // slots whose emitter is no longer among the two stop
        self.slots.retain(|s| {
            let keep = cands.iter().any(|c| c.1 == s.key) && lib.playing(s.sound);
            if !keep {
                lib.stop(s.sound);
                if let Some(c) = s.companion {
                    lib.stop(c);
                }
            }
            keep
        });
        for k in 0..2 {
            if !self.slots.iter().any(|s| s.at == k) {
                occ.reset(k);
            }
        }
        let landmarks_db = lib.category("AMBIENCE_LANDMARKS").map_or(0.0, |c| c.volume);
        for (d2, key, pos, l) in cands {
            // (F-80314: off the positional-music path a slot plays only while the AMBIENCE_LANDMARKS category is above
            // -80 dB, else its sounds stop)
            if !pm && landmarks_db <= -80.0 {
                self.slots.retain(|s| {
                    if s.key == key {
                        lib.stop(s.sound);
                        if let Some(c) = s.companion {
                            lib.stop(c);
                        }
                    }
                    s.key != key
                });
                continue;
            }
            // (F-80314: the volume is 0 dB - the sound's own distance roll-off only - except on the positional-music path:
            // there t between the radii (raised to the inner one, lowered to the outer one, both by fsel, over outer -
            // inner with no guard: equal radii give NaN, which the curves turn into 0), slot 11 at 1 - DYNMIXER(t), started
            // when off, else its strength (+940) clamped to 0 .. 1 - each of the two slots sets it, the second one last -
            // and the emitter at 20 x 0.301 x log2(LANDMARK(t)) dB when that is >= 1e-5, else -100 dB)
            // the slot (F-73347: the emitter's own, else the first empty one) and, for a record with flag 0x2, its
            // occluder: placed 2 m above the emitter (+80 / +84 / +88, the world's), in the requests (+32), and the path's
            // flag replaced by (updated and +112 < 10.0) - +112 is the ctor's 0.0, written by nothing else found - so such
            // a record takes the curves only once its occluder has been updated, whatever the nearest's path
            let at = self.slots.iter().find(|s| s.key == key).map(|s| s.at).or_else(|| (0..2).find(|k| !self.slots.iter().any(|s| s.at == *k))).unwrap_or(0);
            let (pm, occluder) = if l.flags & 2 != 0 {
                let o = occ.place(at, pos + Vec3::new(0.0, 2.0, 0.0));
                if o != 0 {
                    (occ.updated(at), o)
                } else {
                    (pm, 0)
                }
            } else {
                (pm, 0)
            };
            let vol = if pm {
                let t = landmark_t(d2.sqrt(), l.inner, l.outer);
                let a = lib.curve_eval(self.dynmixer_curve, t).unwrap_or(0.0);
                let b = lib.curve_eval(self.landmark_curve, t).unwrap_or(1.0 - t);
                lib.scene_at(POS_MUSIC, true, 1.0 - a);
                if !self.pos_music {
                    log::info!("audio: positional music on ({}, strength {:.2})", lib.name_of(l.sounds[night as usize * 2]).unwrap_or("?"), 1.0 - a);
                }
                self.pos_music = true;
                fsel(b - 1e-5, 20.0 * (b.log2() * 0.301), -100.0)
            } else {
                0.0
            };
            let req = Req { pos: Some(pos), volume: vol, occluder, ..Default::default() };
            match self.slots.iter().find(|s| s.key == key) {
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
                        self.slots.push(Slot { key, sound: h, companion, at });
                        // (F-80314 after the start: the immediate update while "not updated yet")
                        if occluder != 0 {
                            occ.now(at);
                        }
                    }
                }
            }
        }
        // the line emitter (F-49122): the polyline's nearest point, within +39 m
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

/// The location emitter's position between its radii (F-80314 @A-54704..A-45670), as the original computes it.
fn landmark_t(dist: f32, inner: f32, outer: f32) -> f32 {
    let x = fsel(dist - inner, dist, inner);
    let x = fsel(x - outer, outer, x);
    (x - inner) / (outer - inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The speed fade (F-54370) against the running original (work/audio2/cap/sf10.txt, recorder hooks mcla_sf_*): the
    /// manager's speed and the two levels handed on, every distinct case above 50 mph of a full-throttle run.
    #[test]
    fn speed_fade_is_the_originals() {
        let dir = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let Some(a) = crate::audio::Audio::new_offline(&dir) else { return };
        let Some(lib) = SoundLib::load(a.root(), &a.cfg.clone()) else { return };
        let amb = Ambience::new(&lib).expect("ambience");
        let (mut n, mut exact, mut worst) = (0, 0, 0);
        for line in include_str!("speed_fade_cases.txt").lines() {
            let v: Vec<f32> = line.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let (db_a, db_w) = amb.fade_db(&lib, v[0]).expect("curves");
            let ulp = |a: f32, b: f32| (a.to_bits() as i64 - b.to_bits() as i64).abs();
            worst = worst.max(ulp(db_a, v[1])).max(ulp(db_w, v[2]));
            exact += (ulp(db_a, v[1]) == 0 && ulp(db_w, v[2]) == 0) as usize;
            n += 1;
        }
        eprintln!("speed fade: {exact} of {n} cases bit-equal, worst {worst} ulp");
        assert!(n > 40 && worst <= 1);
    }

    /// F-22027's pool count (F-97061: `%s_%.2i` from _00, or from _01 when _00 is missing, while the next one exists) over the
    /// disc's waveslots.xml gives the two shot pools F-20021 sizes; and the two lists' offsets (F-14965): list B at
    /// (0, 0, -6000) through F-57591, list C at (0, 0, +6000) - the quadshots sit opposite the one-shots at one angle.
    #[test]
    fn shot_pools_match_the_wave_slots() {
        let dir = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let Ok(x) = std::fs::read_to_string(std::path::Path::new(&dir).join("audio/X360/config/waveslots.xml")) else { return };
        let names: Vec<&str> = x.split("<Name content=\"ascii\">").skip(1).filter_map(|t| t.split('<').next()).collect();
        let count = |pool: &str| {
            let has = |i: i32| names.iter().any(|n| *n == format!("{pool}_{i:02}"));
            let (mut n, mut skip) = (0, 0);
            let mut i = 0;
            loop {
                if has(i + skip) {
                    n += 1;
                } else if i == 0 && skip == 0 {
                    skip = 1;
                    continue;
                } else {
                    break;
                }
                i += 1;
            }
            n
        };
        assert_eq!(count("AMBIENCE_ONESHOT"), SHOT_SLOTS);
        assert_eq!(count("AMBIENCE_STREAM"), QUAD_SLOTS);
        let a = 0.7f32;
        let (b, c) = (shot_offset(false, a), shot_offset(true, a));
        assert_eq!((b.x, b.z), (-c.x, -c.z));
        assert!((c.x - 6000.0 * a.sin()).abs() < 0.01 && (c.z - 6000.0 * a.cos()).abs() < 0.01);
    }

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

    /// The districts' mixer scenes (+26) and second beds (+18 / +22) from the disc (F-93563): only FREEWAYS and
    /// GARAGE carry a scene, only GARAGE and MALL a second bed.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn district_scenes_and_second_beds() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let g = &lib.game;
        let glob = g.get_name("AMBIENCE_GLOBAL").unwrap();
        let mut names: Vec<String> = (0..16).filter_map(|k| g.name_of(be_u32(glob, 71 + 4 * k)).map(|s| s.to_string())).collect();
        for n in ["DISTRICT_GARAGE", "DISTRICT_FREEWAYS", "DISTRICT_INTERIOR", "DISTRICT_MALL", "DISTRICT_TUNNEL"] {
            names.push(n.into());
        }
        let null = joaat(NULL_SOUND);
        for n in names {
            let d = parse_district(&n, g.get_name(&n).unwrap()).unwrap();
            let scene = g.name_of(d.scene);
            match n.as_str() {
                "DISTRICT_FREEWAYS" => assert_eq!(scene, Some("DYNAMIC_MIXER_FREEWAY")),
                "DISTRICT_GARAGE" => assert_eq!(scene, Some("DYNAMIC_MIXER_GARAGE")),
                _ => assert_eq!(d.scene, 0, "{n}"),
            }
            match n.as_str() {
                "DISTRICT_GARAGE" => assert_eq!(d.bed2.map(|h| lib.name_of(h)), [Some("GARAGE_MUSIC_MASTER"); 2]),
                "DISTRICT_MALL" => assert_eq!(d.bed2, [0x444e7312; 2]),
                _ => assert_eq!(d.bed2, [null; 2], "{n}"),
            }
        }
    }

    /// Downtown's local hangout's records (flag 0x2) with their slot's occluder (F-80314): placed 2 m above the emitter
    /// and in the requests; while "not updated yet" the record is off the curves' path (0 dB) and the start runs the
    /// immediate update; once updated the path is on whatever the speed (here 60 mph, where another landmark's is off);
    /// a slot let go has its occluder reset.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn downtown_landmark_occluder() {
        #[derive(Default)]
        struct Fake {
            placed: [Option<Vec3>; 2],
            updated: [bool; 2],
            resets: usize,
        }
        impl SlotOccluders for Fake {
            fn place(&mut self, k: usize, p: Vec3) -> u16 {
                self.placed[k] = Some(p);
                (k + 1) as u16
            }
            fn updated(&self, k: usize) -> bool {
                self.updated[k]
            }
            fn now(&mut self, k: usize) {
                self.updated[k] = true;
            }
            fn reset(&mut self, k: usize) {
                if self.updated[k] {
                    self.resets += 1;
                }
                self.updated[k] = false;
            }
        }
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let mut amb = Ambience::new(&lib).expect("ambience");
        let dt = crate::hangoutin::SETUPS.iter().find(|s| s.key == "dt_local_hangout1").unwrap();
        let lm = load_lm_set(&lib.game, dt.key, dt.line[1]).unwrap().recs[0].1;
        assert_eq!(lm.flags & 2, 2);
        amb.hangouts(&lib, &[(dt.key, dt.line[1], false)], None);
        let at = lm.pos + Vec3::new(lm.inner, 0.0, 0.0);
        let mut f = Fake::default();
        amb.update(&mut lib, at, false, false, None, 60.0, false, 0.1, &mut f);
        let s = amb.slots.iter().find(|s| matches!(s.key, Key::Lm(_, 0))).expect("the landmark's slot");
        assert_eq!(f.placed[s.at], Some(lm.pos + Vec3::new(0.0, 2.0, 0.0)));
        assert!(f.updated[s.at] && !amb.pos_music);
        amb.update(&mut lib, at, false, false, None, 60.0, false, 0.1, &mut f);
        assert!(amb.pos_music && lib.scene_on(POS_MUSIC));
        amb.update(&mut lib, lm.pos + Vec3::new(5000.0, 0.0, 0.0), false, false, None, 60.0, false, 0.1, &mut f);
        assert!(amb.slots.is_empty() && f.updated == [false; 2] && f.resets >= 1);
    }

    /// The hangouts from the disc: each of the 12 hangout cores' location ids has its game.dat `HNG_DIS_<id>` district
    /// (scene DYNAMIC_MIXER_HANGOUT, beds, the crowd as second bed) and its landmark set `HNG_LM_<id>` + `_0` (the first with
    /// flag 0x4, played in a hangout too; `_0` without it; Downtown's local pair also 0x2, the positional-music object);
    /// EnterHangoutState picks the district over the circles, a landmark in reach starts POSITIONAL_MUSIC below 55 mph
    /// and not in a race, and ExitHangoutState gives the circles back.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn hangouts_from_disc() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let g = &lib.game;
        let null = joaat(NULL_SOUND);
        for s in crate::hangoutin::SETUPS.iter() {
            let n = format!("HNG_DIS_{}", s.key.to_ascii_uppercase());
            let d = parse_district(&n, g.get(joaat(&format!("HNG_DIS_{}", s.key))).unwrap_or_else(|| panic!("{n}"))).unwrap();
            assert_eq!(g.name_of(d.scene), Some("DYNAMIC_MIXER_HANGOUT"), "{n}");
            assert!(d.bed.iter().all(|&b| b != null && b != 0), "{n}");
            // (the second bed is the crowd's murmur, day and night)
            assert_eq!(d.bed2.map(|h| lib.name_of(h)), [Some("HNG_CROWD_MUMOR_RAND_MASTER"); 2], "{n}");
            let set = load_lm_set(g, s.key, s.line[1]).unwrap_or_else(|| panic!("HNG_LM_{}", s.key));
            assert_eq!(set.recs.len(), 2, "{}", s.key);
            let dt = s.key == "dt_local_hangout1";
            assert_eq!(set.recs.iter().map(|r| r.1.flags).collect::<Vec<_>>(), if dt { vec![6, 2] } else { vec![4, 0] }, "{}", s.key);
            for (pos, l) in &set.recs {
                // (every object has its own place: the script's argument is never taken)
                assert!(*pos == l.pos && l.pos != Vec3::ZERO && l.pos.distance(s.line[1]) < 150.0, "{} {:?}", s.key, l.pos);
                assert!(l.outer > l.inner && l.sounds[0] != null && l.sounds[2] != null, "{} {:?}", s.key, l);
            }
        }
        assert!(g.get_name("DISTRICT_HANGOUT").is_none());
        let mut amb = Ambience::new(&lib).expect("ambience");
        let hw = &crate::hangoutin::SETUPS[0];
        let lm = load_lm_set(&lib.game, hw.key, hw.line[1]).unwrap().recs[0].1;
        let cores = [(hw.key, hw.line[1], false)];
        amb.hangouts(&lib, &cores, None);
        assert_eq!(amb.sets.len(), 1);
        // cruising past the hangout: the circles' district, the landmark's set the nearest emitter
        let at = lm.pos + Vec3::new(lm.inner, 0.0, 0.0);
        amb.update(&mut lib, at, false, false, None, 30.0, false, 0.1, &mut NoOccluders);
        assert!(!amb.district_name().unwrap().starts_with("HNG_DIS"));
        assert!(amb.slots.iter().any(|s| matches!(s.key, Key::Lm(..))));
        assert!(amb.pos_music && lib.scene_on(POS_MUSIC));
        // too fast, then a race: slot 11 stops
        amb.update(&mut lib, at, false, false, None, 60.0, false, 0.1, &mut NoOccluders);
        assert!(!amb.pos_music && !lib.scene_on(POS_MUSIC));
        amb.update(&mut lib, at, false, false, None, 30.0, true, 0.1, &mut NoOccluders);
        assert!(!amb.pos_music);
        // suspended: not a candidate
        amb.hangouts(&lib, &[(hw.key, hw.line[1], true)], None);
        amb.update(&mut lib, at, false, false, None, 30.0, false, 0.1, &mut NoOccluders);
        assert!(!amb.slots.iter().any(|s| matches!(s.key, Key::Lm(..))) && !amb.pos_music);
        // in the hangout: its district and scene (stepping past the last switch's 4 s hold, +828)
        amb.hangouts(&lib, &cores, Some(hw.key));
        amb.update(&mut lib, at, false, false, None, 0.0, false, 4.1, &mut NoOccluders);
        assert_eq!(amb.district_name(), Some("HNG_DIS_HW_LOCAL_HANGOUT1"));
        assert!(lib.scene_on("DYNAMIC_MIXER_HANGOUT") && amb.pos_music);
        // out again: the circles' district, the hangout scene stopped
        amb.hangouts(&lib, &cores, None);
        amb.update(&mut lib, at, false, false, None, 0.0, false, 4.1, &mut NoOccluders);
        assert!(!amb.district_name().unwrap().starts_with("HNG_DIS") && !lib.scene_on("DYNAMIC_MIXER_HANGOUT"));
        // an id with no HNG_DIS_ (and no DISTRICT_HANGOUT): the district stays
        let before = amb.district_name().map(|s| s.to_string());
        amb.hangouts(&lib, &[], Some("nowhere"));
        amb.update(&mut lib, Vec3::ZERO, false, false, None, 0.0, false, 0.1, &mut NoOccluders);
        assert_eq!(amb.district_name().map(|s| s.to_string()), before);
        assert!(amb.sets.is_empty());
    }

    /// F-93563's hold: after a switch the next one waits until 4 s have passed (+828); the old bed's release override
    /// is AMBIENCE_GLOBAL +58 x 1000 + 1000 ms.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn district_switch_holds_four_seconds() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let mut amb = Ambience::new(&lib).expect("ambience");
        println!("AMBIENCE_GLOBAL +58 = {} -> release {} ms", amb.rain.ramp, (amb.rain.ramp * 1000.0) as i32 + 1000);
        let at = |amb: &Ambience, name: &str| {
            let d = amb.districts.iter().find(|d| d.name == name).unwrap();
            Vec3::new(d.circles[0].0, 0.0, d.circles[0].1)
        };
        let (sm, skid) = (at(&amb, "DISTRICT_SANTA_MONICA"), at(&amb, "DISTRICT_SKID_ROW"));
        let step = |amb: &mut Ambience, lib: &mut SoundLib, p: Vec3, t: f32| amb.update(lib, p, false, false, None, 0.0, false, t, &mut NoOccluders);
        step(&mut amb, &mut lib, sm, 0.1);
        assert_eq!(amb.district_name(), Some("DISTRICT_SANTA_MONICA"));
        // 3.9 s after the switch: held
        for _ in 0..38 {
            step(&mut amb, &mut lib, skid, 0.1);
        }
        assert_eq!(amb.district_name(), Some("DISTRICT_SANTA_MONICA"));
        for _ in 0..3 {
            step(&mut amb, &mut lib, skid, 0.1);
        }
        assert_eq!(amb.district_name(), Some("DISTRICT_SKID_ROW"));
        // night falls inside the hold: +1073 is set for that frame only, so the night bed never comes
        let bed = amb.districts.iter().find(|d| d.name == "DISTRICT_SKID_ROW").unwrap().bed;
        assert_ne!(bed[0], bed[1]);
        assert_eq!(amb.bed_snd, bed[0]);
        for _ in 0..50 {
            amb.update(&mut lib, skid, true, false, None, 0.0, false, 0.1, &mut NoOccluders);
        }
        assert_eq!(amb.bed_snd, bed[0]);
    }

    /// F-99455's category gates: AMBIENCE at or below -80 dB stops the beds, AMBIENCE_BEDS stops them too, and the
    /// beds start again (without a random start) once the category is back above.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn category_gates_stop_and_restart_the_beds() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let mut a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let mut amb = Ambience::new(&lib).expect("ambience");
        let d = amb.districts.iter().find(|d| d.name == "DISTRICT_SANTA_MONICA").unwrap();
        let (at, bed) = (Vec3::new(d.circles[0].0, 0.0, d.circles[0].1), d.bed[0]);
        let mut step = |amb: &mut Ambience, lib: &mut SoundLib, a: &mut crate::audio::Audio| {
            lib.update(0.1, a);
            amb.update(lib, at, false, false, None, 0.0, false, 0.1, &mut NoOccluders);
        };
        step(&mut amb, &mut lib, &mut a);
        assert!(amb.bed.is_some() && amb.bed_snd == bed);
        for cat in ["AMBIENCE", "AMBIENCE_BEDS"] {
            lib.ramp_category(cat, -100.0, 0.0);
            step(&mut amb, &mut lib, &mut a);
            assert!(amb.bed.is_none(), "{cat}");
            lib.ramp_category(cat, 0.0, 0.0);
            step(&mut amb, &mut lib, &mut a);
            assert!(amb.bed.is_some() && amb.bed_snd == bed, "{cat}");
        }
        lib.ramp_category("AMBIENCE_ONESHOTS", -100.0, 0.0);
        for _ in 0..20 {
            step(&mut amb, &mut lib, &mut a);
        }
        assert!(amb.shots.is_empty());
    }

    /// Into the garage and out: DISTRICT_GARAGE's scene and second bed start, then stop when the circles' district
    /// (no +26, NULL second bed) takes over after the hold.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn garage_scene_stops_on_leaving() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let mut amb = Ambience::new(&lib).expect("ambience");
        let at = Vec3::new(819.6, 0.0, -808.4);
        amb.update(&mut lib, at, false, true, None, 0.0, false, 0.1, &mut NoOccluders);
        assert_eq!(amb.district_name(), Some("DISTRICT_GARAGE"));
        assert!(lib.scene_on("DYNAMIC_MIXER_GARAGE") && amb.bed2.is_some());
        amb.update(&mut lib, at, false, false, None, 0.0, false, 4.1, &mut NoOccluders);
        assert_eq!(amb.district_name(), Some("DISTRICT_OLD_HOLLYWOOD"));
        assert!(!lib.scene_on("DYNAMIC_MIXER_GARAGE") && amb.bed2.is_none());
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
