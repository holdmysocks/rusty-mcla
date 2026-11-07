//! The map effects' sounds (mcParticleAudioEntity): fountains, waterfalls, fire and the smoke stacks. The manager F-75446
//! keeps 64 entities; an effect instance that starts (the effect manager's start callback F-25011, set by F-28371) takes
//! one when game.dat has a `PTX_<effect rule name>` object for it (F-39411 / F-59390), plays the object's start sound
//! once, its loop while the listener is within the object's radius and its stop sound when the instance stops, each
//! through the entity's occluder. Rules: `docs/reference/audio/PTX_AUDIO_RULE.md`.

use crate::sndobj::{Handle, Req, SoundLib};
use glam::Vec3;
use mcla_formats::hash::joaat;
use std::collections::HashMap;

/// The manager's size (F-75446: 64 entities of 128 bytes).
pub const ENTITIES: usize = 64;

/// A `PTX_*` object of game.dat as the entity reads it: +10 the start sound, +14 the loop, +18 the stop sound, +22 the
/// radius (m), +26 / +30 / +34 the occluder's place (added to the instance's origin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PtxDef {
    pub start: u32,
    pub looped: u32,
    pub stop: u32,
    pub radius: f32,
    pub place: [f32; 3],
}

impl PtxDef {
    pub fn read(o: &[u8]) -> Option<PtxDef> {
        let w = |at: usize| o.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        let f = |at: usize| w(at).map(f32::from_bits);
        Some(PtxDef { start: w(10)?, looped: w(14)?, stop: w(18)?, radius: f(22)?, place: [f(26)?, f(30)?, f(34)?] })
    }
}

/// The occluders the entities use (`occlusion::System`'s `effect*`), behind a trait for the offline tests.
pub trait EffectOccluders {
    /// the entity `key`'s occluder at `origin` + `place` (not in the frame's space); from "not updated yet" the
    /// immediate update F-65914 when `now`; its table slot + 1, 0 when there is none
    fn bind(&mut self, key: u64, origin: Vec3, place: [f32; 3], now: bool) -> u16;
    /// the frame's origin moved (F-19929 copies the instance's matrix every update)
    fn follow(&mut self, key: u64, origin: Vec3);
    /// the occluder's volume (+4)
    fn volume(&self, key: u64) -> Option<f32>;
    /// the entity let go (F-83177: the occluder reset when it has been updated)
    fn release(&mut self, key: u64);
}

pub struct NoOccluders;

impl EffectOccluders for NoOccluders {
    fn bind(&mut self, _: u64, _: Vec3, _: [f32; 3], _: bool) -> u16 {
        0
    }
    fn follow(&mut self, _: u64, _: Vec3) {}
    fn volume(&self, _: u64) -> Option<f32> {
        None
    }
    fn release(&mut self, _: u64) {}
}

/// One entity (F-33172): +100 the object, +104 the instance, +96 the loop's handle, +108 the frame of its last
/// start / stop / keep-alive, +112 active, +114 its start waits for the audio update, +115 its stop does.
#[derive(Clone, Debug)]
struct Ent {
    inst: u32,
    def: PtxDef,
    pos: Vec3,
    looped: Option<Handle>,
    stamp: u64,
    active: bool,
    deferred: bool,
    stop_req: bool,
    occ: u16,
    /// +113: the start F-34716 has bound the instance's callbacks (the keep-alive F-11223 when the object has a loop)
    bound: bool,
}

#[derive(Default)]
pub struct PtxSounds {
    ents: Vec<Ent>,
    /// the instances seen last update
    seen: HashMap<u32, Vec3>,
    /// game.dat's object per effect rule name (None: no `PTX_` object)
    defs: HashMap<String, Option<PtxDef>>,
    /// the frame counter (the engine's frame count the entity stamps with)
    frame: u64,
}

/// The hash of NULL_SOUND (the global word the entity compares the object's sounds with).
fn null_sound() -> u32 {
    joaat("NULL_SOUND")
}

// (tests: what the entities played, (update, kind, instance, sound): O start sound, Q stop sound, L loop start, X loop stop)
#[cfg(test)]
thread_local! {
    static LOG: std::cell::RefCell<Vec<(u64, char, u32, u32)>> = const { std::cell::RefCell::new(Vec::new()) };
}
#[cfg(test)]
fn log(frame: u64, kind: char, inst: u32, sound: u32) {
    LOG.with(|l| l.borrow_mut().push((frame, kind, inst, sound)));
}
#[cfg(not(test))]
fn log(_: u64, _: char, _: u32, _: u32) {}

fn key(inst: u32) -> u64 {
    0x5054_5800_0000_0000 | inst as u64
}

impl PtxSounds {
    /// How many entities are bound (manager +256).
    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.ents.len()
    }

    fn def(&mut self, lib: &SoundLib, name: &str) -> Option<PtxDef> {
        // (F-59390: "PTX_%s" with the effect rule's name, +136 -> +248, looked up in game.dat by name; the name hash is
        // case-blind)
        *self.defs.entry(name.to_string()).or_insert_with(|| lib.game.get(joaat(&format!("PTX_{name}"))).and_then(PtxDef::read))
    }

    /// One audio update: the instances playing now (`Particles::instances`). Those new are the effect manager's start
    /// callbacks (F-57036), those gone their stop callbacks (F-93114); then the manager's frame (F-83177).
    pub fn update(&mut self, lib: &mut SoundLib, live: &[(u32, String, Vec3)], occ: &mut dyn EffectOccluders) {
        self.frame += 1;
        let listener = lib.listener.0;
        let now: HashMap<u32, Vec3> = live.iter().map(|(i, _, p)| (*i, *p)).collect();
        // stops: the instance's stop callback (F-93114) off the audio thread sets +115; the update stops it (F-26129)
        for e in self.ents.iter_mut().filter(|e| e.active) {
            if !now.contains_key(&e.inst) {
                e.stop_req = true;
            }
        }
        // starts (F-57036): the instance's entity (F-62801), else a free one (F-39411: none when 64 are bound or game.dat
        // has no object); (approx.) the port's effects start in the particles' step, off the audio update: the start is
        // left to it (+114), F-95929's occluder place and F-34716 run there
        let null = null_sound();
        let frame = self.frame;
        for (inst, name, pos) in live {
            if self.seen.contains_key(inst) {
                continue;
            }
            // F-57036 found the instance's entity (F-62801: an instance object the effect library has reused while its
            // stopped entity is still bound): unless its start waits (+114), the start F-34716 runs at once on it - the
            // stamp, its object's start sound, the callbacks - and it stays not active (only the bind sets +112), so it
            // is let go once untouched
            if let Some(e) = self.ents.iter_mut().find(|e| e.inst == *inst) {
                if !e.deferred {
                    e.pos = *pos;
                    Self::start(lib, e, lib.listener.0, null, frame, occ);
                }
                continue;
            }
            if self.ents.len() >= ENTITIES {
                continue;
            }
            let Some(def) = self.def(lib, name) else { continue };
            self.ents.push(Ent { inst: *inst, def, pos: *pos, looped: None, stamp: self.frame, active: true, deferred: true, stop_req: false, occ: 0, bound: false });
        }
        // the keep-alive callback (F-11223, bound by the start when the object has a loop): +108 = the frame while the
        // instance plays
        for e in self.ents.iter_mut() {
            if let Some(p) = now.get(&e.inst) {
                e.pos = *p;
                if e.bound && e.def.looped != null {
                    e.stamp = frame;
                }
            }
        }
        self.seen = now;
        // the manager's frame (F-83177): an entity whose instance is gone, or not active and untouched for 2 frames, is
        // let go (F-33172's reset: the occluder reset when updated); the others update (F-19929)
        let mut kept = Vec::with_capacity(self.ents.len());
        for mut e in std::mem::take(&mut self.ents) {
            if frame > e.stamp + 2 && !e.active && !e.deferred {
                occ.release(key(e.inst));
                continue;
            }
            Self::step(lib, &mut e, listener, null, frame, occ);
            kept.push(e);
        }
        self.ents = kept;
    }

    /// F-19929 on one entity.
    fn step(lib: &mut SoundLib, e: &mut Ent, listener: Vec3, null: u32, frame: u64, occ: &mut dyn EffectOccluders) {
        let k = key(e.inst);
        occ.follow(k, e.pos);
        if !e.active {
            return;
        }
        if e.deferred {
            // F-95929: the occluder at the object's place on the instance's origin, the immediate update while "not
            // updated yet" when the object has a start sound; then the start F-34716
            e.occ = occ.bind(k, e.pos, e.def.place, e.def.start != null);
            Self::start(lib, e, listener, null, frame, occ);
            e.deferred = false;
        }
        if e.stop_req {
            // F-26129: the loop stops; the stop sound plays once within the radius (and not fully muted); the entity
            // stays bound, not active, until F-83177 lets it go
            e.stamp = frame;
            if let Some(h) = e.looped.take() {
                lib.stop(h);
                log(frame, 'X', e.inst, e.def.looped);
            }
            Self::one_shot(lib, e, e.def.stop, listener, occ, frame, 'Q');
            e.active = false;
            e.bound = false;
            e.stop_req = false;
            return;
        }
        if e.def.looped == null || occ.volume(k) == Some(0.0) {
            return;
        }
        let d2 = e.pos.distance_squared(listener);
        let r2 = e.def.radius * e.def.radius;
        match e.looped {
            // (F-19929: a loop already started stops beyond the radius; one that ended keeps its handle until then)
            Some(h) => {
                if d2 > r2 {
                    lib.stop(h);
                    log(frame, 'X', e.inst, e.def.looped);
                    e.looped = None;
                }
            }
            None => {
                if d2 <= r2 {
                    log(frame, 'L', e.inst, e.def.looped);
                    e.looped = lib.play_hash(e.def.looped, Self::req(e));
                }
            }
        }
    }

    /// F-34716: +108 = the frame, the start sound once (F-23916), the instance's callbacks bound (+113).
    fn start(lib: &mut SoundLib, e: &mut Ent, listener: Vec3, null: u32, frame: u64, occ: &dyn EffectOccluders) {
        e.stamp = frame;
        if e.def.start != null {
            Self::one_shot(lib, e, e.def.start, listener, occ, frame, 'O');
        }
        e.bound = true;
    }

    /// The default request (F-35212) with the instance's place (+20, inst +224) and the occluder (+32); request flags
    /// +70 = 0x24.
    fn req(e: &Ent) -> Req {
        Req { pos: Some(e.pos), occluder: e.occ, ..Default::default() }
    }

    /// F-23916 / F-26129's one-shot: not when the occluder's volume is 0, nor beyond the radius.
    fn one_shot(lib: &mut SoundLib, e: &Ent, hash: u32, listener: Vec3, occ: &dyn EffectOccluders, frame: u64, kind: char) {
        if occ.volume(key(e.inst)) == Some(0.0) {
            return;
        }
        if e.pos.distance_squared(listener) > e.def.radius * e.def.radius {
            return;
        }
        log(frame, kind, e.inst, hash);
        let _ = lib.play_hash(hash, Self::req(e));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 14 `PTX_*` objects of game.dat as the entity reads them, and the effect rules of the disc's list that have one.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn ptx_objects_from_disc() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let names = ["AMB_SMOKE_STACK", "FIRE", "WATER_FOUNTAIN", "WATER_FOUNTAIN_BEVERLY", "WATER_FOUNTAIN_BEVERLY_NO_LOOP", "WATER_FOUNTAIN_BEVERLY_SPROUT", "WATER_FOUNTAIN_CC_M", "WATER_FOUNTAIN_MAC", "WATER_FOUNTAIN_PROMENADE", "WATER_FOUNTAIN_PROMENADE_B", "WATER_FOUNTAIN_RODEO", "WATER_FOUNTAIN_SKINNY", "WATER_FOUNTAIN_SPASH", "WATER_WATERFALL"];
        let null = null_sound();
        for n in names {
            let o = lib.game.get_name(&format!("PTX_{n}")).unwrap_or_else(|| panic!("PTX_{n}"));
            let d = PtxDef::read(o).unwrap();
            let nm = |h: u32| if h == null { "NULL".to_string() } else { lib.name_of(h).unwrap_or("?").to_string() };
            println!("PTX_{n}: start {} loop {} stop {} radius {} place {:?}", nm(d.start), nm(d.looped), nm(d.stop), d.radius, d.place);
            assert!(d.radius > 0.0 && d.radius < 1000.0, "PTX_{n} {d:?}");
            assert!([d.start, d.looped, d.stop].iter().all(|&h| h == null || lib.name_of(h).is_some()), "PTX_{n} {d:?}");
        }
    }

    /// The entities against a recording of the running original (work/recomp-trace `MCLA_PTX_TRACE`; MCLA_PTX_REC =
    /// "<recording>;<data dir>"): each manager frame (F) is one update, with the instances started (B) and not yet
    /// stopped (C) and the recorded listener; what the port plays (start / stop sounds, loop starts / stops) is set
    /// against what the original played, per instance and in order, with the frame difference.
    #[test]
    #[ignore]
    fn entities_against_recording() {
        struct Open;
        impl EffectOccluders for Open {
            fn bind(&mut self, _: u64, _: Vec3, _: [f32; 3], _: bool) -> u16 {
                1
            }
            fn follow(&mut self, _: u64, _: Vec3) {}
            fn volume(&self, _: u64) -> Option<f32> {
                Some(1.0)
            }
            fn release(&mut self, _: u64) {}
        }
        let Ok(spec) = std::env::var("MCLA_PTX_REC") else { return };
        let (rec, dir) = spec.split_once(';').expect("<recording>;<data dir>");
        let a = crate::audio::Audio::new_offline(dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap_or(0);
        let v3 = |x: &str| -> Option<Vec3> {
            let p: Vec<f32> = x.split(',').map(|w| f32::from_bits(u32::from_str_radix(w, 16).unwrap_or(0))).collect();
            (p.len() == 3).then(|| Vec3::new(p[0], p[1], p[2]))
        };
        let mut live: Vec<(u32, String, Vec3)> = Vec::new();
        let mut ent_inst: HashMap<u32, u32> = HashMap::new();
        let mut orig: Vec<(u64, char, u32, u32)> = Vec::new();
        let mut s = PtxSounds::default();
        let mut occ = Open;
        let mut frames = 0u64;
        // (an F line's update runs once its entity lines (E, the state the frame starts with) are read: they give a
        // new instance its place, which is still 0 at its start callback)
        let mut pending = false;
        // the bound count at each frame's start (manager +256) against the port's after the update before
        let (mut counts_same, mut counts_diff, mut first_diff) = (0usize, 0usize, None);
        LOG.with(|l| l.borrow_mut().clear());
        for line in std::fs::read_to_string(rec).expect("recording").lines().chain(std::iter::once("END x x")) {
            let w: Vec<&str> = line.split_whitespace().collect();
            if w.len() < 3 {
                continue;
            }
            if pending && w[0] != "E" {
                s.update(&mut lib, &live, &mut occ);
                frames += 1;
                pending = false;
            }
            match w[0] {
                "B" if w.len() >= 5 => {
                    let pos = w[4].strip_prefix("P=").and_then(v3).unwrap_or(Vec3::ZERO);
                    live.retain(|e| e.0 != h(w[2]));
                    live.push((h(w[2]), w[3].to_string(), pos));
                }
                "C" => {
                    if let Some(i) = ent_inst.get(&h(w[2])) {
                        live.retain(|e| e.0 != *i);
                    }
                }
                "E" if w.len() >= 5 => {
                    let (ent, inst) = (h(w[2]), h(w[4]));
                    if inst != 0 {
                        ent_inst.insert(ent, inst);
                        if let Some(p) = w.iter().find_map(|x| x.strip_prefix("P=")).and_then(v3) {
                            if let Some(e) = live.iter_mut().find(|e| e.0 == inst) {
                                e.2 = p;
                            }
                        }
                    }
                }
                "S" if w.len() >= 4 => {
                    ent_inst.insert(h(w[2]), h(w[3]));
                }
                "O" | "Q" | "L" | "X" if w.len() >= 4 => {
                    let ent = h(w[2]);
                    // X: entity, handle (the loop's sound is the object's)
                    let inst = ent_inst.get(&ent).copied().unwrap_or(0);
                    let sound = if w[0] == "X" { 0 } else { h(w[3]) };
                    orig.push((frames.saturating_sub(1), w[0].chars().next().unwrap(), inst, sound));
                }
                "F" if w.len() >= 4 => {
                    if let Some(l) = w[3].strip_prefix("L=").and_then(v3) {
                        lib.listener.0 = l;
                    }
                    if let Ok(n) = w[2].parse::<usize>() {
                        if n == s.count() {
                            counts_same += 1;
                        } else {
                            counts_diff += 1;
                            first_diff.get_or_insert((frames, n, s.count()));
                            println!("  count frame {frames}: original {n} port {} (live {})", s.count(), live.len());
                        }
                    }
                    pending = true;
                }
                _ => {}
            }
        }
        let port: Vec<(u64, char, u32, u32)> = LOG.with(|l| l.borrow().iter().map(|&(f, k, i, x)| (f - 1, k, i, if k == 'X' { 0 } else { x })).collect());
        // per instance and kind, in order
        let mut by: HashMap<(u32, char), (Vec<(u64, u32)>, Vec<(u64, u32)>)> = HashMap::new();
        for &(f, k, i, x) in &orig {
            by.entry((i, k)).or_default().0.push((f, x));
        }
        for &(f, k, i, x) in &port {
            by.entry((i, k)).or_default().1.push((f, x));
        }
        let mut offs: std::collections::BTreeMap<i64, usize> = Default::default();
        let (mut same, mut sound_diff, mut only_o, mut only_p) = (0, 0, 0, 0);
        let mut odd: Vec<String> = Vec::new();
        for ((i, k), (o, p)) in &by {
            for j in 0..o.len().max(p.len()) {
                match (o.get(j), p.get(j)) {
                    (Some(a), Some(b)) => {
                        *offs.entry(b.0 as i64 - a.0 as i64).or_default() += 1;
                        if a.1 == b.1 { same += 1 } else { sound_diff += 1 }
                    }
                    (Some(a), None) => {
                        only_o += 1;
                        odd.push(format!("only the original: {k} inst {i:08X} frame {} sound {:08X}", a.0, a.1));
                    }
                    (None, Some(b)) => {
                        only_p += 1;
                        odd.push(format!("only the port: {k} inst {i:08X} frame {} sound {:08X}", b.0, b.1));
                    }
                    _ => {}
                }
            }
        }
        let kinds = |v: &[(u64, char, u32, u32)]| ['O', 'Q', 'L', 'X'].map(|c| v.iter().filter(|e| e.1 == c).count());
        println!("{frames} frames; original O Q L X {:?}, port {:?}", kinds(&orig), kinds(&port));
        println!("paired {}: same sound {same}, other sound {sound_diff}; only the original {only_o}, only the port {only_p}; frame offsets (port - original) {offs:?}", same + sound_diff);
        println!("bound count at a frame's start: {counts_same} equal, {counts_diff} differ (first: frame, original, port {first_diff:?})");
        odd.sort();
        for o in odd.iter().take(30) {
            println!("  {o}");
        }
    }

    /// The entity's life on the disc's PTX_WATER_FOUNTAIN: bound at the instance's start, the loop within the radius
    /// only, the stop when the instance is gone, let go 3 frames later; an effect without an object takes none.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn fountain_entity_life() {
        #[derive(Default)]
        struct Fake {
            bound: Vec<u64>,
            released: Vec<u64>,
        }
        impl EffectOccluders for Fake {
            fn bind(&mut self, k: u64, _: Vec3, _: [f32; 3], _: bool) -> u16 {
                self.bound.push(k);
                1
            }
            fn follow(&mut self, _: u64, _: Vec3) {}
            fn volume(&self, _: u64) -> Option<f32> {
                Some(1.0)
            }
            fn release(&mut self, k: u64) {
                self.released.push(k);
            }
        }
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let def = PtxDef::read(lib.game.get_name("PTX_WATER_FOUNTAIN").unwrap()).unwrap();
        assert_ne!(def.looped, null_sound());
        let mut s = PtxSounds::default();
        let mut f = Fake::default();
        let at = Vec3::new(100.0, 0.0, 0.0);
        lib.listener.0 = at + Vec3::new(def.radius * 2.0, 0.0, 0.0);
        let live = vec![(7, "water_fountain".to_string(), at), (8, "wheel_s_asphalt".to_string(), at)];
        s.update(&mut lib, &live, &mut f);
        assert_eq!((s.count(), f.bound.len()), (1, 1));
        assert!(s.ents[0].looped.is_none());
        lib.listener.0 = at + Vec3::new(def.radius * 0.5, 0.0, 0.0);
        s.update(&mut lib, &live, &mut f);
        assert!(s.ents[0].looped.is_some());
        s.update(&mut lib, &[], &mut f);
        assert!(s.ents[0].looped.is_none() && !s.ents[0].active);
        for _ in 0..3 {
            s.update(&mut lib, &[], &mut f);
        }
        assert_eq!((s.count(), f.released.len()), (0, 1));
    }
}
