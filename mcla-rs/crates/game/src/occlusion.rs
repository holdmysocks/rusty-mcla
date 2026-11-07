//! The sound occlusion of the original (GLOBAL_OCCLUSION, docs/AUDIO.md "Occlusion"): an occlusion group holds a
//! volume and a low-pass cutoff for the sounds of one owner, from an occlusion amount the owner's update measures with
//! two world probes between the listener and the emitter (the material depth between them times the materials' factor).
//! This file has the group's own rule: the tune (F-43021 with F-44963 / F-80129), the group's init F-23776, set F-69008,
//! smoothing F-80830 and latch F-43736, and the owner's update F-87712 with the probes and the material factor as
//! inputs, the occluder F-49147, the manager's round-robin F-96474, the radius test F-21927, the tick F-60633 and the
//! immediate update F-65914. Bit-exact offline (tools/occlusion_reference.py, test `occlusion_matches_the_original`).
//! [`System`] keeps the cars' occluders (sound.rs moves them and hands their outputs to the voices); the probe on the
//! port's collision (F-93873, `CityProbe`) is not compared with the original's yet.

#![allow(dead_code)] // (the group's fields the tests read)

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

/// The occlusion factor of each physics material: +226 of its audio material object (MATERIALS_*, the .mtl's
/// `AudioGameObject`; MATERIALS_DEFAULT without one), as F-87712 reads it through the hit's material; 0 when the
/// object is missing.
pub(crate) fn material_factors(data_dir: &std::path::Path, game: &mcla_formats::audcfg::Dat) -> std::collections::HashMap<u8, f32> {
    use crate::physics::collision::Surface;
    use mcla_formats::hash::joaat;
    let dir = data_dir.join("tune").join("physics").join("materials");
    let mut m = std::collections::HashMap::new();
    for s in Surface::ALL {
        let name = s.material();
        let txt = [dir.join(format!("0x{:08x}", joaat(&format!("{name}.mtl")))), dir.join(format!("0x{:08x}", joaat(name)))]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        let obj = txt.lines().find_map(|l| l.trim().strip_prefix("AudioGameObject:").map(|v| v.trim().to_string())).unwrap_or_else(|| "MATERIALS_DEFAULT".into());
        let f = game.get_name(&obj).and_then(|o| o.get(226..230)).map_or(0.0, |b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        m.insert(s as u8, f);
    }
    m
}

/// The world probe F-93873 on the port's city collision: the level's line probe with the city bound and city_roads
/// only (include 0x800, fixed objects), one-sided faces, at most `max` records nearest the start; each hit's factor
/// is its material's.
pub(crate) struct CityProbe<'a> {
    pub world: &'a crate::physics::collision::WorldCollider,
    pub factors: &'a std::collections::HashMap<u8, f32>,
}

impl Probe for CityProbe<'_> {
    fn probe(&mut self, from: [f32; 4], to: [f32; 4], _flags: u32, max: usize) -> Vec<Hit> {
        let o = glam::Vec3::new(from[0], from[1], from[2]);
        let d = glam::Vec3::new(to[0], to[1], to[2]) - o;
        self.world
            .probe_front_all(o, d, 1.0, max)
            .into_iter()
            .map(|h| Hit { pos: h.point.to_array(), factor: self.factors.get(&(h.surface as u8)).copied().unwrap_or(0.0) })
            .collect()
    }
}

/// An occluder as F-49147 builds it: its owner's group and position, the outputs (+4 the volume, +8 the cutoff in Hz),
/// the frame of its audio entity (+16: the entity's matrix, None once the entity is gone) and +119 "not updated yet".
/// A new one starts fully occluded (F-23776 into the outputs).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Occluder {
    pub owner: Owner,
    pub out: [f32; 2],
    pub frame: Option<[[f32; 4]; 4]>,
    pub fresh: bool,
}

impl Occluder {
    pub fn new(t: &Tune, pos: [f32; 4], frame: Option<[[f32; 4]; 4]>) -> Occluder {
        let (group, out) = Group::init(t);
        Occluder { owner: Owner { group, pos, ..Default::default() }, out, frame, fresh: true }
    }

    /// F-21927: the listener within `r` (GLOBAL_OCCLUSION +10) of the position (+80 when it is the world's, else the
    /// frame's origin; the offset is not added here); out of range the group starts over fully occluded.
    pub fn in_range(&mut self, t: &Tune, l: [f32; 4], r: f32) -> bool {
        let e = if self.owner.absolute { self.owner.pos } else { self.frame.map_or([0.0; 4], |f| f[3]) };
        let (dx, dy, dz) = (e[0] - l[0], e[1] - l[1], e[2] - l[2]);
        let d = fma32(dx, dx, fma32(dz, dz, dy * dy));
        if d < r * r {
            return true;
        }
        let (group, out) = Group::init(t);
        self.owner.group = group;
        self.out = out;
        false
    }

    /// F-60633: the outputs step toward the group's targets by the frame's `dt`, once it has a frame and an update.
    pub fn tick(&mut self, t: &Tune, dt: f32) {
        if self.frame.is_some() && !self.fresh {
            self.out = self.owner.group.smooth(t, dt);
        }
    }

    /// F-65914: `local` into +117; with `now` and in range an update and the outputs straight to the targets; no longer
    /// "not updated yet" in every case.
    pub fn refresh(&mut self, t: &Tune, now: bool, local: bool, l: [f32; 4], r: f32, world: &mut impl Probe) {
        self.owner.local = local;
        if now && self.in_range(t, l, r) {
            self.update(t, l, world);
            self.out = self.owner.group.latch();
        }
        self.fresh = false;
    }

    /// F-65900: back to "not updated yet" and fully occluded (+118 / +117 cleared, +119 set, the group's init into
    /// the outputs).
    pub fn reset(&mut self, t: &Tune) {
        self.owner.placed = false;
        self.owner.local = false;
        self.fresh = true;
        let (group, out) = Group::init(t);
        self.owner.group = group;
        self.out = out;
    }

    fn update(&mut self, t: &Tune, l: [f32; 4], world: &mut impl Probe) {
        let frame = self.frame.unwrap_or([[0.0; 4]; 4]);
        self.owner.update(t, &frame, l, world);
    }
}

/// The occlusion manager's update F-96474: from the index it stopped at, round the table of occluders (wrapping at its
/// length), updating each one that is set, has had its first update and passes the radius test, at most 4 a frame;
/// it stops once 4 are done or it is back where it started.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Mgr {
    pub idx: usize,
}

impl Mgr {
    pub fn update(&mut self, t: &Tune, list: &mut [Option<Occluder>], l: [f32; 4], r: f32, world: &mut impl Probe) {
        if list.is_empty() {
            return;
        }
        let mut i = self.idx;
        let mut budget = 4;
        loop {
            if let Some(o) = list.get_mut(i).and_then(|o| o.as_mut()) {
                if !o.fresh && o.in_range(t, l, r) {
                    o.update(t, l, world);
                    budget -= 1;
                }
            }
            i += 1;
            if i == list.len() {
                i = 0;
            }
            if budget == 0 || i == self.idx {
                break;
            }
        }
        self.idx = i;
    }
}

/// The probe the game hands the occluders: the city's (`CityProbe`) when its collision is loaded, else none.
pub(crate) enum WorldProbe<'a> {
    City(CityProbe<'a>),
    None,
}

impl<'a> WorldProbe<'a> {
    pub fn of(world: Option<&'a crate::physics::collision::WorldCollider>, factors: &'a std::collections::HashMap<u8, f32>) -> WorldProbe<'a> {
        match world {
            Some(world) => WorldProbe::City(CityProbe { world, factors }),
            None => WorldProbe::None,
        }
    }
}

impl Probe for WorldProbe<'_> {
    fn probe(&mut self, from: [f32; 4], to: [f32; 4], flags: u32, max: usize) -> Vec<Hit> {
        match self {
            WorldProbe::City(c) => c.probe(from, to, flags, max),
            WorldProbe::None => Vec::new(),
        }
    }
}

/// A probe that hits nothing (no city collision loaded): every amount is 0.
pub(crate) struct NoProbe;

impl Probe for NoProbe {
    fn probe(&mut self, _: [f32; 4], _: [f32; 4], _: u32, _: usize) -> Vec<Hit> {
        Vec::new()
    }
}

/// The occluders as the game keeps them: the audio engine's table of 400 entities (engine +10408; an entity's
/// registration F-18654 takes the first free slot), each with the occluder its init F-37940 made, the manager
/// F-96474 over the table, and the tick F-60633 on each. Only the cars' occluders are made here (the player's and
/// the computer racers' car audio, the cars with `CarSounds`); the peds', props', map effects' and the ambience's
/// are not (docs/AUDIO.md, open).
pub(crate) struct System {
    pub tune: Tune,
    /// GLOBAL_OCCLUSION +10: the listener's radius (800 m)
    pub radius: f32,
    /// GLOBAL_OCCLUSION +14 / +18 / +22: the car occluder's place, added to the car's origin (0, 0.9, 0)
    pub car_offset: [f32; 3],
    /// GLOBAL_OCCLUSION +26 / +30 / +34: a walker's (0, 1.5, 0)
    pub ped_offset: [f32; 3],
    pub factors: std::sync::Arc<std::collections::HashMap<u8, f32>>,
    pub list: Vec<Option<Occluder>>,
    /// whose each slot is: (0, a car's `uid`), (1, a walker's key), (2, a prop entity's key), (3, an ambience slot) or (4, a map effect's entity)
    owner: Vec<Option<(u8, u64)>>,
    mgr: Mgr,
    /// the ambience's two location slots' occluders (manager +156 + 20 i + 16, made by its init F-20021)
    amb: [Option<usize>; 2],
}

/// The table's size (F-18654 / F-96474: 400 entities).
pub(crate) const SLOTS: usize = 400;

impl System {
    pub fn new(tune: Tune, o: &[u8], factors: std::collections::HashMap<u8, f32>) -> Option<System> {
        let f = |at: usize| o.get(at..at + 4).map(|b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        let mut sys = System { tune, radius: f(10)?, car_offset: [f(14)?, f(18)?, f(22)?], ped_offset: [f(26)?, f(30)?, f(34)?], factors: std::sync::Arc::new(factors), list: vec![None; SLOTS], owner: vec![None; SLOTS], mgr: Mgr::default(), amb: [None; 2] };
        // the ambience's init F-20021 (at the title, before any car): one occluder from the pool for each of its two
        // location slots (F-49147, no instance), its place the world's (+116 = 1), registered in the table (F-18654)
        for k in 0..2 {
            let mut o = Occluder::new(&sys.tune, [0.0; 4], None);
            o.owner.absolute = true;
            sys.list[k] = Some(o);
            sys.owner[k] = Some((3, k as u64));
            sys.amb[k] = Some(k);
        }
        Some(sys)
    }

    /// The ambience slot `k`'s occluder at `p` (F-80314: the emitter's place 2 m up, +80 / +84 / +88, for a record with
    /// flag 0x2) and its table slot.
    pub fn amb_place(&mut self, k: usize, p: [f32; 3]) -> Option<usize> {
        let i = self.amb.get(k).copied().flatten()?;
        let o = self.list[i].as_mut()?;
        o.owner.pos = [p[0], p[1], p[2], o.owner.pos[3]];
        Some(i)
    }

    /// The table slot of the ambience slot `k`'s occluder.
    pub fn amb_place_slot(&self, k: usize) -> Option<usize> {
        self.amb.get(k).copied().flatten()
    }

    /// Whether the ambience slot `k`'s occluder has been updated (+119 clear).
    pub fn amb_updated(&self, k: usize) -> bool {
        self.amb.get(k).copied().flatten().and_then(|i| self.list[i].as_ref()).is_some_and(|o| !o.fresh)
    }

    /// F-80314 after the slot's sound starts: the immediate update F-65914 (now, not local) while "not updated yet".
    pub fn amb_now(&mut self, k: usize, l: [f32; 4], world: &mut impl Probe) {
        if let Some(i) = self.amb.get(k).copied().flatten() {
            self.now(i, l, world);
        }
    }

    /// The slot stops or is not among the two nearest (F-10322 / F-12897 / F-10645): F-65900 when its occluder has been updated.
    pub fn amb_reset(&mut self, k: usize) {
        let t = &self.tune;
        if let Some(o) = self.amb.get(k).copied().flatten().and_then(|i| self.list[i].as_mut()) {
            if !o.fresh {
                o.reset(t);
            }
        }
    }

    /// The car's frame: the port keeps only its origin (the car's place), the one row a car occluder reads (its
    /// place is not in the frame's space and not the world's: the origin plus `car_offset`, F-87712).
    fn frame(origin: [f32; 3]) -> [[f32; 4]; 4] {
        [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [origin[0], origin[1], origin[2], 1.0]]
    }

    /// The car `uid`'s occluder slot, its frame moved to the car's `origin`. A new one is made as the car audio's
    /// start does it (F-43725): the entity's occluder (F-37940 with F-49147) in the first free slot, its place
    /// `car_offset`, then the immediate update F-65914 (now, not local).
    pub fn car(&mut self, uid: u32, origin: [f32; 3], l: [f32; 4], world: &mut impl Probe) -> Option<usize> {
        if let Some(i) = self.owner.iter().position(|o| *o == Some((0, uid as u64))) {
            let (t, r) = (&self.tune, self.radius);
            if let Some(o) = self.list[i].as_mut() {
                o.frame = Some(Self::frame(origin));
                // (an ambient car's switched-off occluder that has become a racer's: the car audio start's update)
                if o.fresh {
                    o.refresh(t, true, false, l, r, world);
                }
            }
            return Some(i);
        }
        let i = self.list.iter().position(|o| o.is_none())?;
        let [x, y, z] = self.car_offset;
        let mut o = Occluder::new(&self.tune, [x, y, z, 0.0], Some(Self::frame(origin)));
        o.refresh(&self.tune, true, false, l, self.radius, world);
        self.list[i] = Some(o);
        self.owner[i] = Some((0, uid as u64));
        Some(i)
    }

    /// The ambient car `uid`'s occluder slot (mcAmbientVehicleAudioEntity), its frame moved to the car's `origin`. A
    /// new one is the ambient entity's (F-37940 from the ctor F-15430): "not updated yet" and fully occluded, its place
    /// `car_offset` (the init F-89483), no update until the entity switches it on.
    pub fn ambient(&mut self, uid: u32, origin: [f32; 3]) -> Option<usize> {
        if let Some(i) = self.owner.iter().position(|o| *o == Some((0, uid as u64))) {
            if let Some(o) = self.list[i].as_mut() {
                o.frame = Some(Self::frame(origin));
            }
            return Some(i);
        }
        let i = self.list.iter().position(|o| o.is_none())?;
        let [x, y, z] = self.car_offset;
        self.list[i] = Some(Occluder::new(&self.tune, [x, y, z, 0.0], Some(Self::frame(origin))));
        self.owner[i] = Some((0, uid as u64));
        Some(i)
    }

    /// The ambient car's switch in its update (F-70175): on while one of its voices was placed within the last
    /// 0.1333 s - from "not updated yet" the immediate update F-65914 with no update (the manager takes it from
    /// there); off past it - F-65900 when it has been updated.
    pub fn switch(&mut self, i: usize, on: bool, l: [f32; 4], world: &mut impl Probe) {
        let (t, r) = (&self.tune, self.radius);
        if let Some(o) = self.list.get_mut(i).and_then(|o| o.as_mut()) {
            match (on, o.fresh) {
                (true, true) => o.refresh(t, false, false, l, r, world),
                (false, false) => o.reset(t),
                _ => {}
            }
        }
    }

    /// The pass-by whoosh's immediate update (F-74101: F-65914 now, not local, while "not updated yet").
    pub fn now(&mut self, i: usize, l: [f32; 4], world: &mut impl Probe) {
        let (t, r) = (&self.tune, self.radius);
        if let Some(o) = self.list.get_mut(i).and_then(|o| o.as_mut()) {
            if o.fresh {
                o.refresh(t, true, false, l, r, world);
            }
        }
    }

    /// The cars whose audio is gone give their slots back (the entity's release takes it out of the table).
    pub fn keep(&mut self, live: &[u32]) {
        for (o, u) in self.list.iter_mut().zip(self.owner.iter_mut()) {
            if u.is_some_and(|(k, u)| k == 0 && !live.contains(&(u as u32))) {
                *o = None;
                *u = None;
            }
        }
    }

    /// One frame: the manager's round of updates (F-96474, the engine frame F-88094), then every occluder's tick
    /// (F-60633). (approx.) The order of the two within the frame: the tick's caller is not found.
    pub fn step(&mut self, l: [f32; 4], dt: f32, world: &mut impl Probe) {
        self.mgr.update(&self.tune, &mut self.list, l, self.radius, world);
        for o in self.list.iter_mut().flatten() {
            o.tick(&self.tune, dt);
        }
    }

    /// A walker's reaction entity's occluder (mcAmbientPedAudioEntity, F-39461), its frame moved to the walker's
    /// `origin`. The reaction places it at `ped_offset`, its own instance excluded, and from "not updated yet" runs
    /// the immediate update F-65914 (now, not local); here a new one is made when the walker's reaction starts.
    pub fn person(&mut self, key: u64, origin: [f32; 3], l: [f32; 4], world: &mut impl Probe) -> Option<usize> {
        if let Some(i) = self.owner.iter().position(|o| *o == Some((1, key))) {
            if let Some(o) = self.list[i].as_mut() {
                o.frame = Some(Self::frame(origin));
            }
            return Some(i);
        }
        let i = self.list.iter().position(|o| o.is_none())?;
        let [x, y, z] = self.ped_offset;
        let mut o = Occluder::new(&self.tune, [x, y, z, 0.0], Some(Self::frame(origin)));
        o.refresh(&self.tune, true, false, l, self.radius, world);
        self.list[i] = Some(o);
        self.owner[i] = Some((1, key));
        Some(i)
    }

    /// A walker's entity let go once its sound is over (F-33574: the occluder reset, F-65900; here the slot is given
    /// back, which the manager skips the same way).
    pub fn release_person(&mut self, key: u64) {
        self.release(1, key);
    }

    /// A prop's entity (one of the 64 of F-44000) bound to a prop at `origin`: the bind F-95562 places its occluder at
    /// the prop type's +69 / +73 / +77 (`place`), the instance excluded. From "not updated yet" its sounds run the
    /// immediate update F-65914: `now` for a contact or pass-by one-shot (F-99417), with no update for the distance
    /// loop (F-32775). (approx.) +62 bit 0x08 (the place in the prop's frame, 2 of the 282 types) is taken as the
    /// world's: the port's props carry no rotation here.
    pub fn prop(&mut self, key: u64, origin: [f32; 3], place: [f32; 3], now: bool, l: [f32; 4], world: &mut impl Probe) -> Option<usize> {
        let i = match self.owner.iter().position(|o| *o == Some((2, key))) {
            Some(i) => i,
            None => {
                let i = self.list.iter().position(|o| o.is_none())?;
                self.list[i] = Some(Occluder::new(&self.tune, [place[0], place[1], place[2], 0.0], None));
                self.owner[i] = Some((2, key));
                i
            }
        };
        let (t, r) = (&self.tune, self.radius);
        let o = self.list[i].as_mut()?;
        o.frame = Some(Self::frame(origin));
        if o.fresh {
            o.refresh(t, now, false, l, r, world);
        }
        Some(i)
    }

    /// A map effect's entity (mcParticleAudioEntity, F-95929): its occluder at `place` (the PTX object's +26 / +30 /
    /// +34) on the instance's `origin`, not in the frame's space, and from "not updated yet" the immediate update
    /// F-65914 when `now` (the object has a start sound). (approx.) Made here when the entity is bound; the original's
    /// 64 entities keep theirs from the start.
    pub fn effect(&mut self, key: u64, origin: [f32; 3], place: [f32; 3], now: bool, l: [f32; 4], world: &mut impl Probe) -> Option<usize> {
        let i = match self.owner.iter().position(|o| *o == Some((4, key))) {
            Some(i) => i,
            None => {
                let i = self.list.iter().position(|o| o.is_none())?;
                self.list[i] = Some(Occluder::new(&self.tune, [place[0], place[1], place[2], 0.0], None));
                self.owner[i] = Some((4, key));
                i
            }
        };
        let (t, r) = (&self.tune, self.radius);
        let o = self.list[i].as_mut()?;
        o.owner.pos = [place[0], place[1], place[2], 0.0];
        o.frame = Some(Self::frame(origin));
        if o.fresh {
            o.refresh(t, now, false, l, r, world);
        }
        Some(i)
    }

    /// A map effect's entity follows its instance (F-19929: the instance's matrix into the entity's frame).
    pub fn effect_follow(&mut self, key: u64, origin: [f32; 3]) {
        if let Some(i) = self.owner.iter().position(|o| *o == Some((4, key))) {
            if let Some(o) = self.list[i].as_mut() {
                o.frame = Some(Self::frame(origin));
            }
        }
    }

    /// The table slot of a map effect's entity.
    pub fn effect_slot(&self, key: u64) -> Option<usize> {
        self.owner.iter().position(|o| *o == Some((4, key)))
    }

    /// A map effect's entity let go (F-83177: the occluder reset when updated; here the slot is given back).
    pub fn release_effect(&mut self, key: u64) {
        self.release(4, key);
    }

    /// A prop's entity let go (F-32518: the occluder reset when it has been updated; here the slot is given back).
    pub fn release_prop(&mut self, key: u64) {
        self.release(2, key);
    }

    fn release(&mut self, kind: u8, key: u64) {
        if let Some(i) = self.owner.iter().position(|o| *o == Some((kind, key))) {
            self.list[i] = None;
            self.owner[i] = None;
        }
    }

    /// The slot of the car `uid`.
    pub fn slot_of(&self, uid: u32) -> Option<usize> {
        self.owner.iter().position(|o| *o == Some((0, uid as u64)))
    }

    /// The outputs (volume, cutoff in Hz) of a slot.
    pub fn out(&self, i: usize) -> Option<[f32; 2]> {
        self.list.get(i).and_then(|o| o.as_ref()).map(|o| o.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    /// The disc's GLOBAL_OCCLUSION as the wiring reads it: radius 800 m, the car occluder's place (0, 0.9, 0), and the
    /// volume never 0 at any depth (the lowest is the fully occluded 0.2), so the car update's stop at volume 0
    /// (F-36837) is never reached with the disc's tune. Needs MCLA_DATA_DIR (extracted\cache).
    /// The probe and depth (F-87712 on `CityProbe`) against the original's: MCLA_OCC_REC = "<recording>;<data dir>",
    /// a recording of the occlusion recorder (work/recomp-trace `MCLA_OCC_TRACE`: each update's listener, emitter and
    /// the depth it gives the group's set). Prints how many depths are equal and the worst ones.
    #[test]
    #[ignore]
    fn probe_against_recording() {
        let Ok(spec) = std::env::var("MCLA_OCC_REC") else { return };
        let (rec, dir) = spec.split_once(';').expect("<recording>;<data dir>");
        let assets = crate::assets::Assets::new(dir);
        crate::physics::collision::load_surfaces(dir);
        let mut city = crate::physics::collision::WorldCollider::default();
        for (i, mesh) in crate::world::city_bound(&assets).expect("city bound").into_iter().enumerate() {
            city.sectors.insert(crate::world::BOUND_ID + i, mesh);
        }
        let a = crate::audio::Audio::new_offline(dir).expect("audio");
        let lib = crate::sndobj::SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
        let o = lib.game.get_name("GLOBAL_OCCLUSION").expect("GLOBAL_OCCLUSION").to_vec();
        let t = Tune::read(&o, |h| lib.curve(h).cloned()).expect("tune");
        let factors = material_factors(std::path::Path::new(dir), &lib.game);
        let mut probe = CityProbe { world: &city, factors: &factors };
        let f = |h: &str| f32::from_bits(u32::from_str_radix(h, 16).unwrap());
        let (mut n, mut same, mut near, mut both_zero, mut zero_port, mut zero_orig) = (0, 0, 0, 0, 0, 0);
        let mut worst: Vec<(f32, String)> = Vec::new();
        let (mut max_near, mut max_ulps) = (0.0f32, 0i64);
        for line in std::fs::read_to_string(rec).expect("recording").lines() {
            let kv: std::collections::HashMap<&str, &str> = line.split_whitespace().filter_map(|w| w.split_once('=')).collect();
            let (Some(l), Some(e), Some(d)) = (kv.get("L"), kv.get("E"), kv.get("d")) else { continue };
            let v = |s: &str| -> Option<[f32; 4]> {
                let p: Vec<&str> = s.split(',').collect();
                (p.len() == 3 && p.iter().all(|x| x.len() == 8)).then(|| [f(p[0]), f(p[1]), f(p[2]), 0.0])
            };
            let (Some(l), Some(e)) = (v(l), v(e)) else { continue };
            if d.len() != 8 {
                continue;
            }
            let want = f(d);
            let mut owner = Owner { absolute: true, pos: e, ..Default::default() };
            let got = owner.update(&t, &[[0.0; 4]; 4], l, &mut probe);
            n += 1;
            match (got == 0.0, want == 0.0) {
                (true, true) => both_zero += 1,
                (true, false) => zero_port += 1,
                (false, true) => zero_orig += 1,
                _ => {}
            }
            if got.to_bits() == want.to_bits() {
                same += 1;
            } else if (got - want).abs() <= 1e-3 * want.abs().max(1.0) {
                near += 1;
                let ulps = (got.to_bits() as i64 - want.to_bits() as i64).abs();
                max_near = max_near.max((got - want).abs());
                max_ulps = max_ulps.max(ulps);
            } else {
                worst.push(((got - want).abs(), format!("L {:?} E {:?} original {want} port {got}", &l[..3], &e[..3])));
            }
        }
        worst.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("{n} updates: {same} equal, {near} within 1e-3, {} differ; zero in both {both_zero}, only the port {zero_port}, only the original {zero_orig}", worst.len());
        println!("within 1e-3: largest difference {max_near} m, {max_ulps} ulps");
        for (_, w) in worst.iter().take(25) {
            println!("  {w}");
        }
    }

    #[test]
    #[ignore]
    fn disc_tune_for_the_wiring() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let lib = crate::sndobj::SoundLib::load(a.root(), &a.cfg.clone()).expect("sound objects");
        let o = lib.game.get_name("GLOBAL_OCCLUSION").expect("GLOBAL_OCCLUSION").to_vec();
        let t = Tune::read(&o, |h| lib.curve(h).cloned()).expect("tune");
        let s = System::new(t.clone(), &o, Default::default()).expect("system");
        assert_eq!((s.radius, s.car_offset), (800.0, [0.0, 0.9, 0.0]));
        let mut low = f32::MAX;
        for k in 0..=20000 {
            let (mut g, _) = Group::init(&t);
            g.set(&t, k as f32 * 0.01);
            let v = g.latch()[0];
            low = low.min(v);
        }
        println!("lowest volume {low}, init {:?}", Group::init(&t).1);
        assert!(low > 0.0, "{low}");
    }

    /// An ambient car's occluder (F-70175 / F-74101 on the reset F-65900 and the immediate update F-65914): made off and
    /// fully occluded; switched on it is no longer "not updated yet" but keeps its outputs until the manager updates
    /// it; switched off it is reset; the whoosh's update latches at once, only when off.
    #[test]
    fn ambient_switch() {
        // the disc's GLOBAL_OCCLUSION and its two curves, as the reference file holds them
        let text = include_str!("testdata/occlusion_ref.txt");
        let hexw = |t: &str| u32::from_str_radix(&t[1..], 16).unwrap();
        let unhex = |s: &str| (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<u8>>();
        let mut lines = text.lines();
        let c: Vec<&str> = lines.next().unwrap().split(' ').collect();
        let curves = [(hexw(c[1]), Curve::parse(&unhex(&c[2][1..]))), (hexw(c[3]), Curve::parse(&unhex(&c[4][1..])))];
        let o = unhex(&lines.next().unwrap()[3..]);
        let t = Tune::read(&o, |h| curves.iter().find(|c| c.0 == h).map(|c| c.1.clone())).unwrap();
        let mut s = System::new(t.clone(), &o, Default::default()).unwrap();
        assert_eq!((s.radius, s.car_offset), (800.0, [0.0, 0.9, 0.0]));
        let l = [0.0, 0.0, 0.0, 0.0];
        // the ambience's two slot occluders take the first two slots, the world's place, "not updated yet"; placed 2 m
        // above an emitter, updated at once after the start (the 0x2 record's path switch), reset when the slot is let go
        let init = Group::init(&t).1;
        for k in 0..2 {
            let o = s.list[k].unwrap();
            assert!(o.fresh && o.owner.absolute && o.frame.is_none());
            assert_eq!(s.amb_place_slot(k), Some(k));
        }
        assert_eq!(s.amb_place(1, [3.0, 2.0, 4.0]), Some(1));
        assert_eq!(s.list[1].unwrap().owner.pos[..3], [3.0, 2.0, 4.0]);
        assert!(!s.amb_updated(1));
        s.amb_now(1, l, &mut NoProbe);
        assert!(s.amb_updated(1) && !s.amb_updated(0));
        s.amb_now(1, [900.0, 0.0, 0.0, 0.0], &mut NoProbe);
        assert_eq!(s.out(1), Some(s.list[1].unwrap().owner.group.latch()));
        s.amb_reset(1);
        assert!(!s.amb_updated(1));
        assert_eq!(s.out(1), Some(init));
        s.keep(&[]);
        assert!(s.list[0].is_some() && s.list[1].is_some());
        let i = s.ambient(7, [10.0, 0.0, 0.0]).unwrap();
        assert_eq!(i, 2);
        assert!(s.list[i].unwrap().fresh);
        assert_eq!(s.out(i), Some(init));
        s.switch(i, true, l, &mut NoProbe);
        assert!(!s.list[i].unwrap().fresh);
        assert_eq!(s.out(i), Some(init));
        s.step(l, 1.0 / 60.0, &mut NoProbe);
        assert_ne!(s.out(i), Some(init));
        s.switch(i, false, l, &mut NoProbe);
        assert!(s.list[i].unwrap().fresh);
        assert_eq!(s.out(i), Some(init));
        s.now(i, l, &mut NoProbe);
        assert!(!s.list[i].unwrap().fresh);
        assert_eq!(s.out(i), Some(s.list[i].unwrap().owner.group.latch()));
        assert_eq!(s.slot_of(7), Some(i));
        // a walker's: (0, 1.5, 0), updated at once, its own key apart from the cars' uids; let go, the slot is free
        assert_eq!(s.ped_offset, [0.0, 1.5, 0.0]);
        let j = s.person(7, [0.0, 0.0, 5.0], l, &mut NoProbe).unwrap();
        assert_ne!(i, j);
        assert!(!s.list[j].unwrap().fresh);
        assert_eq!(s.list[j].unwrap().owner.pos, [0.0, 1.5, 0.0, 0.0]);
        s.keep(&[]);
        assert!(s.list[j].is_some() && s.list[i].is_none());
        s.release_person(7);
        assert!(s.list[j].is_none());
        // a prop's: the loop switches it on with no update (outputs kept), a hit latches at once; keys apart
        let a = s.prop(7, [0.0, 0.0, 9.0], [0.0, 3.0, 0.0], false, l, &mut NoProbe).unwrap();
        assert!(!s.list[a].unwrap().fresh);
        assert_eq!((s.out(a), s.list[a].unwrap().owner.pos), (Some(init), [0.0, 3.0, 0.0, 0.0]));
        let b = s.prop(8, [0.0, 0.0, 9.0], [0.0, 0.5, 0.0], true, l, &mut NoProbe).unwrap();
        assert_ne!(a, b);
        assert_eq!(s.out(b), Some(s.list[b].unwrap().owner.group.latch()));
        s.release_prop(7);
        assert!(s.list[a].is_none() && s.list[b].is_some());
    }

    /// tools/occlusion_reference.py ran the original's code offline: the pow F-21955 on 6,000 inputs, F-33490 on 2,000
    /// random Gauss curves, and on 150 tunes (GLOBAL_OCCLUSION or random) the reader then 20 random group ops each
    /// (init, set, smoothing, latch, the owner's update with random frames, positions and probe hits); every result,
    /// field, output and probe call as the original's; and the wiring on 200 cases of up to 6 occluders built by the
    /// original's ctor F-49147 (GLOBAL_OCCLUSION as on the disc), 14 random ops each: the manager F-96474 from a given
    /// index, the immediate update F-65914, the tick F-60633 and moves of the frames; every probe call, the manager's
    /// index and each occluder's outputs, flag and group as the original's
    #[test]
    fn occlusion_matches_the_original() {
        let text = include_str!("testdata/occlusion_ref.txt");
        let hexw = |t: &str| u32::from_str_radix(&t[1..], 16).unwrap();
        let unhex = |s: &str| (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<u8>>();
        let (mut curves, mut factors) = (Vec::new(), [0f32; 8]);
        let (mut tune, mut owner, mut bad, mut n) = (None::<Tune>, Owner::default(), Vec::new(), [0usize; 6]);
        let (mut wtune, mut radius, mut occ) = (None::<Tune>, 0f32, Vec::<Option<Occluder>>::new());
        let ostate = |s: &mut String, occ: &[Option<Occluder>]| {
            for o in occ.iter().flatten() {
                write!(s, " | o{:08x} o{:08x} u{} f{}{}", o.out[0].to_bits(), o.out[1].to_bits(), o.fresh as u8, o.owner.local as u8, o.owner.placed as u8).unwrap();
                let g = &o.owner.group;
                for v in [g.amount, g.vol, g.hz, g.hz_log, g.vol_to, g.hz_to] {
                    write!(s, " g{:08x}", v.to_bits()).unwrap();
                }
            }
        };
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
                "B" => {
                    let o = unhex(&t[1][1..]);
                    radius = f32::from_bits(u32::from_be_bytes([o[10], o[11], o[12], o[13]]));
                    wtune = Tune::read(&o, |h| curves.iter().find(|c| c.0 == h).map(|c| c.1.clone()));
                }
                "R" => {
                    let tn = wtune.as_ref().unwrap();
                    let k = t[1][1..].parse::<usize>().unwrap();
                    occ.clear();
                    s = t[..2 + 9 * k].join(" ");
                    for j in 0..k {
                        let w = |i: usize| t[2 + 9 * j + i];
                        let f = |i: usize| f32::from_bits(hexw(w(i)));
                        let mut frame = [[0f32; 4]; 4];
                        frame[3] = [f(6), f(7), f(8), 1.0];
                        let gone = w(1) != "B0";
                        let mut o = Occluder::new(tn, [f(2), f(3), f(4), f(5)], (!gone).then_some(frame));
                        o.owner.absolute = w(0) != "A0";
                        occ.push(Some(o));
                    }
                    ostate(&mut s, &occ);
                    n[4] += 1;
                }
                "O" => {
                    let tn = wtune.as_ref().unwrap();
                    let m = t[1][1..].parse::<u32>().unwrap();
                    let na = [2, 4, 3, 4, 2][m as usize];
                    let x = |i: usize| hexw(t[1 + i]);
                    let l = [f32::from_bits(x(na)), f32::from_bits(x(na + 1)), f32::from_bits(x(na + 2)), 1.0];
                    // the probe hits the original's run used, in call order (after its count)
                    let c_at = t.iter().position(|v| v.starts_with('c')).unwrap();
                    let mut lists = Vec::new();
                    let mut i = c_at + 1;
                    while i < t.len() && t[i].starts_with('h') {
                        let cnt = hexw(t[i]) as usize;
                        let g = |k: usize| f32::from_bits(hexw(t[i + 1 + k]));
                        lists.push((0..cnt).map(|k| Hit { pos: [g(4 * k), g(4 * k + 1), g(4 * k + 2)], factor: factors[(hexw(t[i + 4 + 4 * k]) & 7) as usize] }).collect::<Vec<_>>());
                        i += 1 + 4 * cnt;
                    }
                    struct Seq<'a> {
                        lists: Vec<Vec<Hit>>,
                        calls: &'a mut String,
                        k: usize,
                    }
                    impl Probe for Seq<'_> {
                        fn probe(&mut self, from: [f32; 4], to: [f32; 4], flags: u32, max: usize) -> Vec<Hit> {
                            *self.calls += " Q";
                            for v in from.iter().chain(to.iter()) {
                                write!(self.calls, " p{:08x}", v.to_bits()).unwrap();
                            }
                            write!(self.calls, " g{flags} n{max}").unwrap();
                            self.k += 1;
                            self.lists.get(self.k - 1).cloned().unwrap_or_default()
                        }
                    }
                    s = t[..na + 4].join(" ");
                    let mut world = Seq { lists, calls: &mut s, k: 0 };
                    match m {
                        0 => {
                            let mut mgr = Mgr { idx: x(1) as usize };
                            mgr.update(tn, &mut occ, l, radius, &mut world);
                            let k = world.k;
                            write!(s, " i{} c{k}", mgr.idx).unwrap();
                        }
                        1 => {
                            occ[x(1) as usize].as_mut().unwrap().refresh(tn, x(2) != 0, x(3) != 0, l, radius, &mut world);
                            let k = world.k;
                            write!(s, " c{k}").unwrap();
                        }
                        2 => {
                            occ[x(1) as usize].as_mut().unwrap().tick(tn, f32::from_bits(x(2)));
                            s += " c0";
                        }
                        4 => {
                            occ[x(1) as usize].as_mut().unwrap().reset(tn);
                            s += " c0";
                        }
                        _ => {
                            for o in occ.iter_mut().flatten() {
                                if let Some(f) = o.frame.as_mut() {
                                    f[3] = [f32::from_bits(x(1)), f32::from_bits(x(2)), f32::from_bits(x(3)), 1.0];
                                }
                            }
                            s += " c0";
                        }
                    }
                    for v in &t[c_at + 1..i] {
                        s += " ";
                        s += v;
                    }
                    ostate(&mut s, &occ);
                    n[5] += 1;
                }
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
        assert_eq!(n, [6000, 2000, 150, 3000, 200, 2800], "the reference's counts");
        assert!(bad.is_empty(), "{} lines differ, first:\n{}", bad.len(), bad[..bad.len().min(6)].join("\n"));
    }
}
