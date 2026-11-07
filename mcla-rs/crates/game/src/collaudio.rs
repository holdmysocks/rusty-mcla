//! The original's collision sounds (sub_82381FB0 and the prop audio entity): per contact call the strongest closing
//! contact picks an impact tier of the car's zone (roof / side / undercarriage / wheel by the car's own material), the
//! zone's L/R bangs play across the car, a frontal hit adds the hood damage sound, and the fastest sliding contact
//! drives a scrape loop chosen by the other side's material. Props: each type's game.dat object (impact by speed,
//! break sound). Rules: `docs/reference/audio/COLLISION_AUDIO_RULE.md`.
//!
//! Approximate (written down there): the port's car-to-car and car-to-prop contacts are not the original's (one
//! point per pair, the prop's own footprint test), so those sounds follow the rule on the port's contacts; the prop
//! scrape / deflection loops and the whoosh are not played yet.

use crate::physics::car::{AudioTouch, CarState};
use crate::physics::collision::Surface;
use crate::sndobj::{Handle, Req, SoundLib};
use glam::{Mat3, Vec3};
use hashbrown::HashMap;
use mcla_formats::audcfg::{db2lin, lin2db};
use mcla_formats::hash::joaat;
use std::path::Path;

const MPH: f32 = 2.237;

/// MCLA_COLL_AUDIO=1: each collision sound on stderr
fn trace() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| std::env::var_os("MCLA_COLL_AUDIO").is_some())
}

fn be_u32(o: &[u8], at: usize) -> u32 {
    o.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).unwrap_or(0)
}
fn be_f32(o: &[u8], at: usize) -> f32 {
    f32::from_bits(be_u32(o, at))
}
fn be_s16(o: &[u8], at: usize) -> f32 {
    o.get(at..at + 2).map(|b| i16::from_be_bytes(b.try_into().unwrap()) as f32).unwrap_or(0.0)
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Who owns the collision sounds of a car (owner +12): the player's car, a racer, anything else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Player,
    Opponent,
    Ambient,
}

/// Per car: the owner's state (owner +44 .. +124).
#[derive(Default)]
struct CarAudio {
    scrape: Option<Handle>,
    scrape_hash: u32,
    scrape_local: Vec3,
    scrape_time: f32,
    scrape_frame: u64,
    tier3_frame: u64,
    /// bangs that travel with the car (voice position mode 16): handle, position
    bangs: Vec<(Handle, Vec3)>,
    last_pos: Option<Vec3>,
}

pub struct CollisionAudio {
    /// physics material -> its audio material object (`AudioGameObject` of `tune/physics/materials`)
    materials: HashMap<u8, u32>,
    cars: HashMap<usize, CarAudio>,
    /// props: per instance position bucket, the time of its last break / impact (+116 / once-per-prop breaks)
    frame: u64,
    time: f32,
}

impl CollisionAudio {
    pub fn new(data_dir: &Path) -> CollisionAudio {
        let mut materials = HashMap::new();
        let dir = data_dir.join("tune").join("physics").join("materials");
        for s in Surface::ALL {
            let name = s.material();
            let txt = [dir.join(format!("0x{:08x}", joaat(&format!("{name}.mtl")))), dir.join(format!("0x{:08x}", joaat(name)))].iter().find_map(|p| std::fs::read_to_string(p).ok());
            let obj = txt
                .as_deref()
                .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("AudioGameObject:").map(|v| v.trim().to_string())))
                .unwrap_or_else(|| "MATERIALS_DEFAULT".into());
            materials.insert(s as u8, joaat(&obj));
        }
        CollisionAudio { materials, cars: HashMap::new(), frame: 0, time: 0.0 }
    }

    /// The car's type-54 collision manager (vehicle audio DEFAULT +38, else by owner).
    fn manager<'a>(lib: &'a SoundLib, car: &str, owner: Owner, bike: bool) -> Option<&'a [u8]> {
        let g = &lib.game;
        let up = car.to_ascii_uppercase();
        if owner == Owner::Ambient {
            return g.get_name("AMBIENT_CAR_COLLISION_MGR");
        }
        let from_profile = g.get_name(&format!("{up}_PROFILE")).and_then(|p| g.get(be_u32(p, if owner == Owner::Player { 10 } else { 14 }))).and_then(|d| g.get(be_u32(d, 38))).filter(|o| o.first() == Some(&54));
        from_profile.or_else(|| {
            g.get_name(match (owner, bike) {
                (Owner::Player, true) => "PLAYER_BIKE_COLLISION_MGR",
                (Owner::Player, false) => "PLAYER_CAR_COLLISION_MGR",
                (_, true) => "OPPONENT_BIKE_COLLISION_MGR",
                (_, false) => "OPPONENT_CAR_COLLISION_MGR",
            })
        })
    }

    /// A frame's sounds of one car: every contact call's impact, then the scrape. `occ`: the car's occluder slot plus one
    /// (the collision owner's +36, the car's occluder: F-24302 puts it into every request), 0 = none.
    #[allow(clippy::too_many_arguments)]
    pub fn car(&mut self, lib: &mut SoundLib, idx: usize, car: &str, owner: Owner, bike: bool, st: &CarState, player: bool, occ: u16) {
        let Some(mgr) = Self::manager(lib, car, owner, bike).map(|m| m.to_vec()) else { return };
        let zone_of = |own: Surface| -> u32 {
            // sub_82381D60: car_side -> SIDE, car_undercarriage -> UNDER, car_roof -> ROOF, car_wheel -> WHEEL
            be_u32(&mgr, match own {
                Surface::CarRoof => 10,
                Surface::CarUndercarriage => 18,
                Surface::CarWheel => 22,
                _ => 14,
            })
        };
        let frame = self.frame;
        let time = self.time;
        let ca = self.cars.entry(idx).or_default();
        // the bangs travel with the car (sub_82383688: moved by the car's displacement)
        let disp = ca.last_pos.map_or(Vec3::ZERO, |p| st.pos - p);
        ca.last_pos = Some(st.pos);
        ca.bangs.retain(|(h, _)| lib.playing(*h));
        for (h, p) in ca.bangs.iter_mut() {
            *p += disp;
            lib.set(*h, Req { pos: Some(*p), player, occluder: occ, ..Default::default() });
        }
        let rot = Mat3::from_quat(st.rot);
        let mut calls: Vec<u32> = st.touches.iter().map(|t| t.call).collect();
        calls.dedup();
        for call in calls {
            let list: Vec<&AudioTouch> = st.touches.iter().filter(|t| t.call == call).collect();
            // 2.1: the strongest closing contact, the fastest sliding one
            let (mut best_imp, mut imp) = (0.0f32, None::<&AudioTouch>);
            let (mut best_scr2, mut scr) = (0.0f32, None::<&AudioTouch>);
            for t in &list {
                let v = t.vel;
                let vl = v.length();
                if vl > 1e-6 && t.normal.dot(v / vl) >= 0.1 {
                    if v.dot(t.normal) > best_imp {
                        best_imp = v.dot(t.normal);
                        imp = Some(t);
                    }
                } else if v.length_squared() > best_scr2 {
                    best_scr2 = v.length_squared();
                    scr = Some(t);
                }
            }
            // 2.2: the impact
            if let Some(t) = imp {
                let s = best_imp * MPH;
                if let Some(z) = lib.game.get(zone_of(t.own)).map(|z| z.to_vec()) {
                    let f = |o: usize| be_f32(&z, o);
                    let tier = if s < f(34) {
                        None
                    } else if s < f(62) {
                        Some((0usize, lerp(f(26), f(30), (s - f(34)) / (f(62) - f(34)))))
                    } else if s < f(90) {
                        Some((1, lerp(f(54), f(58), (s - f(62)) / (f(90) - f(62)))))
                    } else if s < f(118) {
                        Some((2, lerp(f(82), f(86), (s - f(90)) / (f(118) - f(90)))))
                    } else if ca.tier3_frame == frame {
                        // a second tier-3 hit in a frame: nothing at all this call, the scrape neither
                        continue;
                    } else {
                        ca.tier3_frame = frame;
                        Some((3, if s < f(122) { lerp(f(110), f(114), (s - f(90)) / (f(122) - f(118))) } else { f(114) }))
                    };
                    if let Some((k, gain)) = tier {
                        let b = 10 + 28 * k;
                        if trace() {
                            eprintln!("COLLAUDIO impact car {idx} {owner:?} zone {} {:.1} mph tier {k} gain {gain:.2} L {} R {}", lib.game.name_of(zone_of(t.own)).unwrap_or("?"), s, lib.name_of(be_u32(&z, b)).unwrap_or("?"), lib.name_of(be_u32(&z, b + 4)).unwrap_or("?"));
                        }
                        let vol = lin2db(gain);
                        let req = |p: Vec3| Req { pos: Some(p), volume: vol, player, occluder: occ, ..Default::default() };
                        lib.play_hash(be_u32(&z, b + 8), req(t.pos));
                        // the hood: a hit ahead of the car (n against the rear axis)
                        if t.normal.dot(rot.z_axis) < -std::f32::consts::FRAC_1_SQRT_2 {
                            if let Some(h) = lib.play_hash(be_u32(&z, b + 12), req(t.pos)) {
                                ca.bangs.push((h, t.pos));
                            }
                        }
                        // L / R across the line from the car's centre to the hit, +-half the zone's spacing
                        let local = rot.transpose() * (t.pos - st.pos);
                        let c = local.normalize_or(Vec3::Z);
                        let a = Vec3::Y.cross(c).normalize_or(Vec3::X);
                        let half = 0.5 * f(178);
                        for (off, snd) in [(half, be_u32(&z, b)), (-half, be_u32(&z, b + 4))] {
                            let p = st.pos + rot * (a * off);
                            if let Some(h) = lib.play_hash(snd, req(p)) {
                                ca.bangs.push((h, p));
                            }
                        }
                    }
                }
            }
            // 2.3: the scrape
            let Some(t) = scr else { continue };
            let s = best_scr2.sqrt() * 0.447;
            let Some(z) = lib.game.get(zone_of(t.own)).map(|z| z.to_vec()) else { continue };
            let f = |o: usize| be_f32(&z, o);
            if s < f(146) {
                continue;
            }
            let dt = time - ca.scrape_time;
            ca.scrape_time = time;
            ca.scrape_frame = frame;
            let ratio = lerp(f(170), f(174), (s - f(146)) / (f(150) - f(146))).clamp(f(170), f(174));
            let k = (dt * f(138)).min(1.0);
            let local = rot.transpose() * (t.pos - st.pos);
            ca.scrape_local = if ca.scrape.is_some() { ca.scrape_local.lerp(local, k) } else { local };
            let world = st.pos + rot * ca.scrape_local;
            let down = t.normal.dot(rot.y_axis) < -std::f32::consts::FRAC_1_SQRT_2;
            let Some(m) = self.materials.get(&(t.other as u8)).and_then(|h| lib.game.get(*h)).map(|m| m.to_vec()) else { continue };
            let hash = match (owner, down) {
                (Owner::Player, true) => be_u32(&m, 34),
                (Owner::Player, false) => be_u32(&m, 30),
                (Owner::Opponent, true) => be_u32(&m, 54),
                (Owner::Opponent, false) => be_u32(&m, 58),
                (Owner::Ambient, true) => be_u32(&m, 66),
                (Owner::Ambient, false) => be_u32(&m, 70),
            };
            if let Some(h) = ca.scrape {
                if hash != ca.scrape_hash {
                    lib.stop(h);
                    ca.scrape = None;
                }
            }
            let req = Req { pos: Some(world), pitch: (1200.0 * ratio.log2()) as i32, player, occluder: occ, ..Default::default() };
            match ca.scrape {
                Some(h) => lib.set(h, req),
                None => {
                    ca.scrape = lib.play_hash(hash, req);
                    ca.scrape_hash = hash;
                    if trace() {
                        eprintln!("COLLAUDIO scrape car {idx} {owner:?} {} at {s:.1} ratio {ratio:.2} {}", lib.name_of(hash).unwrap_or("?"), if ca.scrape.is_some() { "playing" } else { "no sound" });
                    }
                }
            }
        }
        // the scrape stops two frames after its last update (sub_82381F50)
        if let Some(h) = ca.scrape {
            if frame - ca.scrape_frame >= 2 {
                lib.stop(h);
                ca.scrape = None;
            }
        }
    }

    /// Cars that are gone: their loops stop.
    pub fn retain(&mut self, lib: &mut SoundLib, alive: impl Fn(usize) -> bool) {
        self.cars.retain(|i, ca| {
            let keep = alive(*i);
            if !keep {
                if let Some(h) = ca.scrape.take() {
                    lib.stop(h);
                }
            }
            keep
        });
    }

    pub fn begin_frame(&mut self, dt: f32) {
        self.frame += 1;
        self.time += dt;
    }

    /// A prop hit (3.1 / 3.4): the type's impact sound by speed; a break sound when it is knocked loose.
    pub fn prop(&mut self, lib: &mut SoundLib, model: u32, pos: Vec3, speed: f32, knocked: bool) {
        let o = match lib.game.get(model).filter(|o| o.first() == Some(&43)) {
            Some(o) => o.to_vec(),
            None => match lib.game.get_name("LA_PROP_DEFAULT") {
                Some(o) => o.to_vec(),
                None => return,
            },
        };
        let n = o.get(118).copied().unwrap_or(0) as usize;
        if n == 0 {
            return;
        }
        // entry 0 (the bound material choice needs the prop's own bound: approximate)
        let (brk, impact) = (be_u32(&o, 123), be_u32(&o, 127));
        let s = speed * MPH;
        if s >= be_f32(&o, 22) {
            let base = be_s16(&o, 63);
            let top = be_s16(&o, 32);
            let (lo, hi) = (be_f32(&o, 22), be_f32(&o, 28));
            let g = if s < hi { lerp(db2lin(base * 0.01), db2lin(top * 0.01), (s - lo) / (hi - lo)) } else { db2lin(top * 0.01) };
            let vol = lin2db(g) + base * 0.01;
            lib.play_hash(impact, Req { pos: Some(pos), volume: vol, ..Default::default() });
        }
        if knocked {
            lib.play_hash(brk, Req::at(pos));
        }
    }
}
