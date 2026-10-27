//! Drivers and riders: the `drv_*_lod02_set` characters posed with each car's own driver
//! animations (`resources/animation/<hash>/opponent/male/*.r2`: one pack per car with
//! `<CAR>_Drv_steer_sweep`, `_Drv_Idle_stp`, `_Drv_Idle_Low`, `_Drv_rev`, ...). The
//! animations are authored in car space, so the root track seats the driver.

use crate::assets::load_resource_file;
use crate::render::{Frustum, Gpu};
use crate::skin::{blend_pose, Clip, SkinBuffer, SkinModel};
use glam::{Mat4, Quat, Vec3};
use hashbrown::HashMap;
use mcla_formats::rage::anim::{animation_named, pack_entries};
use mcla_formats::rage::Rd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What the driver of one car needs to know this frame.
pub struct Seat<'a> {
    pub id: usize,
    pub car: &'a str,
    pub mat: Mat4,
    pub pos: Vec3,
    /// -1 left .. 1 right
    pub steer: f32,
    pub speed: f32,
    pub reversing: bool,
    pub player: bool,
    pub cop: bool,
    pub bike: bool,
    /// step 19d: the character set this driver is ("drv_mb_009_set"), else empty
    pub set: &'a str,
}

struct Rig {
    model: Arc<SkinModel>,
    steer: Clip,
    idle_stop: Option<Clip>,
    reverse: Option<Clip>,
    /// the pack's gesture clips by their name after the car prefix ("drv_gest_listen_lft", "drv_win",
    /// "drv_gest_jawflap"); the car's own first, the shared Challenger ones otherwise
    gestures: HashMap<String, Clip>,
    /// DRV_HIT_HRD's length (the wreck's hit, F-47123's machine: then 4 s of DRV_IDLE_START)
    hit_hard: Option<f32>,
}

/// The gesture a scene event asks of a driver (racer+684; the event ids of table 0x827EAD60).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Neutral,
    Left,
    Right,
    TalkLeft,
    TalkRight,
    Mild,
    Win,
    ListenLeft,
    ListenRight,
    ThumbDownLeft,
    ThumbDownRight,
}

impl Gesture {
    /// From a scene event's name (the first entry of the event table the name contains).
    pub fn from_event(name: &str) -> Option<Gesture> {
        let n = name.to_ascii_lowercase();
        // (table order: neutral, left, right, talk_left, talk_right, mild, win, listen_left, listen_right,
        // thm_dwn_lft, thm_dwn_rgt - "gesture_talk_left" contains "left"; the original's first match is by
        // the full event names, so the longer names are tested first here)
        Some(match n.trim_start_matches("gesture_") {
            "neutral" => Gesture::Neutral,
            "left" => Gesture::Left,
            "right" => Gesture::Right,
            "talk_left" => Gesture::TalkLeft,
            "talk_right" => Gesture::TalkRight,
            "mild" => Gesture::Mild,
            "win" => Gesture::Win,
            "listen_left" => Gesture::ListenLeft,
            "listen_right" => Gesture::ListenRight,
            "thm_dwn_lft" => Gesture::ThumbDownLeft,
            "thm_dwn_rgt" => Gesture::ThumbDownRight,
            _ => return None,
        })
    }
}

/// A gesture being played (the driver graph's "gesture switch" state, sub_823B3D70).
#[derive(Clone)]
struct Playing {
    clip: String,
    t: f32,
    /// the event's end time from now (its end call sets the gesture back to none), else infinite
    left: f32,
}

/// Per driver: the gesture's round-robin counters (ctx+68..+84), what plays, the jaw flap.
#[derive(Default, Clone)]
struct Acting {
    rr: [usize; 5],
    playing: Option<Playing>,
    /// a gesture asked for, its clip chosen when the driver is next posed
    asked: Option<(Gesture, f32, bool)>,
    /// fading out of a gesture that ended: (clip, frame, weight)
    leaving: Option<(String, f32, f32)>,
    jaw: bool,
    jaw_k: f32,
    jaw_t: f32,
}

#[derive(Default, Clone, Copy)]
struct Pose {
    steer: f32,
    stopped: f32,
    reverse: f32,
    seen: u32,
}

#[derive(Default)]
pub struct Drivers {
    char_dir: PathBuf,
    /// car key ("frd_mustang_06", "vpd_chv_impala_64") -> animation pack
    packs: HashMap<String, PathBuf>,
    /// driver sets: (name, bike rider, police)
    sets: Vec<(String, bool, bool)>,
    models: HashMap<usize, Option<Arc<SkinModel>>>,
    rigs: HashMap<(String, usize), Option<Rig>>,
    poses: HashMap<usize, Pose>,
    buf: SkinBuffer,
    /// the player's head in car space (for the cockpit camera)
    pub player_head: Option<Vec3>,
    /// the camera sits in the player's head
    pub headless: bool,
    time: f32,
    frame: u32,
    acting: HashMap<usize, Acting>,
    /// the character set each seat was last drawn with (its voice: CareerVoices.xml)
    pub set_of: HashMap<usize, String>,
}

/// The neutral / left / right / talk round robins (tables 0x827EA5F4, 0x827EA608, 0x827EA61C, 0x827EA630,
/// 0x827EA640).
const RR_NEUTRAL: [&str; 5] = ["drv_gest_mirror", "drv_win", "drv_gest_rspnd_lft", "drv_gest_rspnd_rgt", "drv_gest_fidget_01"];
const RR_LEFT: [&str; 5] = ["drv_gest_mild_lft_01", "drv_gest_agro_lft_01", "drv_gest_thm_dwn_lft", "drv_gest_thm_up_lft", "drv_gest_cas_lft_01"];
const RR_RIGHT: [&str; 5] = ["drv_gest_mild_rgt_01", "drv_gest_agro_rgt_01", "drv_gest_thm_dwn_rgt", "drv_gest_thm_up_rgt", "drv_gest_cas_rgt_01"];
const RR_TALK_LEFT: [&str; 4] = ["drv_gest_cas_lft_01", "drv_gest_agro_lft_01", "drv_gest_mild_lft_01", "drv_gest_rspnd_lft"];
const RR_TALK_RIGHT: [&str; 4] = ["drv_gest_cas_rgt_01", "drv_gest_agro_rgt_01", "drv_gest_mild_rgt_01", "drv_gest_rspnd_rgt"];
/// a gesture blends in over 0.25 s (sub_823A5C10); the way back to driving is not read - the same 0.25 s
const GESTURE_BLEND: f32 = 0.25;

/// The clip a gesture plays (sub_823B3D70); `human`: the neutral round robin skips `_rspnd_rgt` for the human
/// player and `_rspnd_lft` for the others.
fn gesture_clip(g: Gesture, rr: &mut [usize; 5], human: bool) -> String {
    let mut next = |k: usize, list: &[&str]| -> String {
        let c = list[rr[k] % list.len()];
        rr[k] += 1;
        c.to_string()
    };
    match g {
        Gesture::Neutral => loop {
            let c = next(0, &RR_NEUTRAL);
            if (human && c == "drv_gest_rspnd_rgt") || (!human && c == "drv_gest_rspnd_lft") {
                continue;
            }
            break c;
        },
        Gesture::Left => next(1, &RR_LEFT),
        Gesture::Right => next(2, &RR_RIGHT),
        Gesture::TalkLeft => next(3, &RR_TALK_LEFT),
        Gesture::TalkRight => next(4, &RR_TALK_RIGHT),
        // (mild and win pick between two; the selector is not read - the first)
        Gesture::Mild => "drv_gest_mild_lft_01".into(),
        Gesture::Win => "drv_win".into(),
        Gesture::ListenLeft => "drv_gest_listen_lft".into(),
        Gesture::ListenRight => "drv_gest_listen_rgt".into(),
        Gesture::ThumbDownLeft => "drv_gest_thm_dwn_lft".into(),
        Gesture::ThumbDownRight => "drv_gest_thm_dwn_rgt".into(),
    }
}

/// Blend `b` over `a` only on the bones `clip` animates (an overlay such as the jaw flap).
fn overlay(a: &mut [(Quat, Vec3)], b: &[(Quat, Vec3)], clip: &Clip, k: f32) {
    for i in 0..a.len().min(b.len()) {
        if clip.rot.get(i).map_or(false, |r| r.is_some()) {
            let fr = if a[i].0.dot(b[i].0) < 0.0 { -b[i].0 } else { b[i].0 };
            a[i].0 = a[i].0.lerp(fr, k).normalize();
        }
        if clip.pos.get(i).map_or(false, |p| p.is_some()) {
            a[i].1 = a[i].1.lerp(b[i].1, k);
        }
    }
}

/// "vp_frd_mustang_06_dub" -> "frd_mustang_06"; DLC cars keep their "vpd_" prefix.
fn car_key(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    let n = n.trim_end_matches("_dub");
    n.strip_prefix("vp_").unwrap_or(n).to_string()
}

fn hash(a: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^ (h >> 13)
}

impl Drivers {
    pub fn load(&mut self, data_dir: &str) {
        let root = Path::new(data_dir);
        self.char_dir = root.join("resources/character");
        let mut names: Vec<String> = std::fs::read_dir(&self.char_dir).map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default();
        names.sort();
        for n in names {
            let l = n.to_ascii_lowercase();
            // the in-car versions; male ones (the packs used here are the male set)
            if !(l.ends_with("lod02_set") || l.ends_with("lod2_set")) || !(l.starts_with("drv_m") || l.starts_with("pol_")) {
                continue;
            }
            self.sets.push((n, l.contains("_mot_"), l.starts_with("pol_")));
        }
        // which pack belongs to which car: read once, then kept in a small index file
        let index = root.join("driver_packs.tsv");
        if let Ok(t) = std::fs::read_to_string(&index) {
            for line in t.lines() {
                if let Some((k, f)) = line.split_once('\t') {
                    self.packs.insert(k.to_string(), PathBuf::from(f));
                }
            }
        }
        if self.packs.is_empty() {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("resources/animation")).map(|d| d.flatten().map(|e| e.path().join("opponent/male")).filter(|p| p.is_dir()).collect()).unwrap_or_default();
            dirs.sort();
            let mut out = String::new();
            for dir in dirs {
                let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
                files.sort();
                for f in files {
                    let Ok(res) = load_resource_file(&f) else { continue };
                    let mut count: HashMap<String, u32> = HashMap::new();
                    for (_, name) in pack_entries(&res) {
                        let l = name.to_ascii_lowercase();
                        let Some(i) = l.find("_fdrv_").or_else(|| l.find("_drv_")) else { continue };
                        *count.entry(l[..i].to_string()).or_default() += 1;
                    }
                    // every pack also carries the shared Challenger gestures
                    let mut v: Vec<(String, u32)> = count.into_iter().collect();
                    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                    let key = v.iter().find(|(k, n)| k != "dge_challenger_70" && *n >= 10).or(v.first()).map(|(k, _)| k.clone());
                    if let Some(k) = key {
                        if !self.packs.contains_key(&k) {
                            out.push_str(&format!("{}\t{}\n", k, f.display()));
                            self.packs.insert(k, f);
                        }
                    }
                }
            }
            let _ = std::fs::write(&index, out);
        }
        log::info!("drivers: {} character sets, {} animation packs", self.sets.len(), self.packs.len());
    }

    fn model(&mut self, gpu: &mut Gpu, si: usize) -> Option<Arc<SkinModel>> {
        if !self.models.contains_key(&si) {
            let name = &self.sets[si].0;
            let dir = self.char_dir.join(name);
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "r6").unwrap_or(false)).collect()).unwrap_or_default();
            files.sort();
            let m = files.first().and_then(|f| SkinModel::load(gpu, name, f)).map(Arc::new);
            if m.is_none() {
                log::warn!("drivers: could not load {}", name);
            }
            self.models.insert(si, m);
        }
        self.models[&si].clone()
    }

    fn pick_set(&self, s: &Seat) -> Option<usize> {
        // (step 19d: a driver who was named - Racer_SetCharacterName - is that character)
        if !s.set.is_empty() {
            if let Some(i) = self.sets.iter().position(|n| n.0.eq_ignore_ascii_case(s.set)) {
                return Some(i);
            }
        }
        let want = |n: &(String, bool, bool)| n.1 == s.bike && n.2 == (s.cop && !s.bike);
        if s.player {
            // "mp" is the player character
            if let Some(i) = self.sets.iter().position(|n| n.0.to_ascii_lowercase().starts_with("drv_mp_") && n.1 == s.bike) {
                return Some(i);
            }
        }
        let c: Vec<usize> = (0..self.sets.len()).filter(|&i| want(&self.sets[i]) && !self.sets[i].0.to_ascii_lowercase().starts_with("drv_mp_")).collect();
        if c.is_empty() {
            return self.sets.iter().position(|n| n.1 == s.bike);
        }
        Some(c[hash(s.id as u32 + 17) as usize % c.len()])
    }

    /// The length of a car's DRV_HIT_HRD clip, once its driver is loaded.
    pub fn hit_hard_secs(&self, car: &str) -> Option<f32> {
        let k = car_key(car);
        self.rigs.iter().filter(|((c, _), _)| *c == k).find_map(|(_, r)| r.as_ref()?.hit_hard)
    }

    fn rig(&mut self, gpu: &mut Gpu, car: &str, si: usize) -> bool {
        let key = (car_key(car), si);
        if !self.rigs.contains_key(&key) {
            let rig = (|| -> Option<Rig> {
                let model = self.model(gpu, si)?;
                let pack = self.packs.get(&key.0).or_else(|| self.packs.get("dge_challenger_70"))?.clone();
                let res = load_resource_file(&pack).ok()?;
                let rd = Rd::new(&res);
                let entries = pack_entries(&res);
                let clip = |suffixes: &[&str]| -> Option<Clip> {
                    for s in suffixes {
                        // the car's own clip first, then the shared one
                        let mut hit = entries.iter().filter(|(_, n)| n.to_ascii_lowercase().ends_with(s)).collect::<Vec<_>>();
                        hit.sort_by_key(|(_, n)| !n.to_ascii_lowercase().starts_with(&key.0));
                        if let Some((o, _)) = hit.first() {
                            return animation_named(&rd, *o).map(|a| Clip::bind(&Arc::new(a), &model.ids, false, false));
                        }
                    }
                    None
                };
                let steer = clip(&["drv_steer_sweep", "drv_low_lean_sweep", "drv_idle_stp_sweep"])?;
                // the gestures the scenes ask for (case blind by name after the car prefix, sub_823B7D68)
                let mut gestures = HashMap::new();
                let mut names: Vec<&(usize, String)> = entries.iter().filter(|(_, n)| { let l = n.to_ascii_lowercase(); l.contains("_drv_gest_") || l.ends_with("_drv_win") || l.ends_with("_drv_win_02") }).collect();
                names.sort_by_key(|(_, n)| n.to_ascii_lowercase().starts_with(&key.0));
                for (o, n) in names {
                    let l = n.to_ascii_lowercase();
                    let Some(i) = l.find("drv_") else { continue };
                    if let Some(a) = animation_named(&rd, *o) {
                        gestures.insert(l[i..].to_string(), Clip::bind(&Arc::new(a), &model.ids, false, false));
                    }
                }
                let hit_hard = clip(&["drv_hit_hrd"]).map(|c| c.anim.duration);
                Some(Rig { steer, idle_stop: clip(&["drv_idle_stp"]), reverse: clip(&["drv_rev"]), model, gestures, hit_hard })
            })();
            if rig.is_none() {
                log::warn!("drivers: no animations for {}", car);
            }
            self.rigs.insert(key.clone(), rig);
        }
        self.rigs[&key].is_some()
    }

    /// A scene's gesture event for the driver of seat `id` (racer+684). Lost when a gesture is still playing
    /// (the "gesture switch" exit writes "none"). `dur` > 0: the event's end call stops it then.
    pub fn gesture(&mut self, id: usize, g: Gesture, dur: f32, human: bool) {
        let a = self.acting.entry(id).or_default();
        if a.playing.is_none() && a.asked.is_none() {
            a.asked = Some((g, if dur > 0.0 { dur } else { f32::INFINITY }, human));
        }
    }

    /// The jaw flap while the driver's speech line plays (racer+688; `DRV_GEST_JAWFLAP` in / out over 0.25 s).
    pub fn jaw(&mut self, id: usize, on: bool) {
        self.acting.entry(id).or_default().jaw = on;
    }

    /// Scene over: no gestures, no jaw flap.
    pub fn stop_acting(&mut self) {
        for a in self.acting.values_mut() {
            a.playing = None;
            a.asked = None;
            a.jaw = false;
        }
    }

    /// Pose and skin the drivers of the cars near the camera.
    pub fn prepare(&mut self, gpu: &mut Gpu, seats: &[Seat], cam: Vec3, frustum: &Frustum, dt: f32) {
        self.buf.clear();
        self.time += dt;
        self.frame += 1;
        let mut local: Vec<(Quat, Vec3)> = Vec::new();
        let mut other: Vec<(Quat, Vec3)> = Vec::new();
        for s in seats {
            let dist = s.pos.distance(cam);
            // ambient drivers only matter close up
            let reach = if s.car.starts_with("va_") { 40.0 } else { 70.0 };
            if dist > reach || !frustum.intersects_aabb(s.pos - Vec3::splat(2.5), s.pos + Vec3::splat(2.5)) {
                continue;
            }
            let Some(si) = self.pick_set(s) else { continue };
            self.set_of.insert(s.id, self.sets[si].0.clone());
            if !self.rig(gpu, s.car, si) {
                continue;
            }
            let frame = self.frame;
            let p = self.poses.entry(s.id).or_default();
            if p.seen + 1 != frame {
                *p = Pose { steer: s.steer, ..Default::default() };
            }
            p.seen = frame;
            let k = (dt * 8.0).min(1.0);
            p.steer += (s.steer - p.steer) * k;
            p.stopped += ((if s.speed.abs() < 0.6 { 1.0 } else { 0.0 }) - p.stopped) * (dt * 3.0).min(1.0);
            p.reverse += ((if s.reversing && s.speed.abs() > 0.3 { 1.0 } else { 0.0 }) - p.reverse) * (dt * 4.0).min(1.0);
            let p = *p;
            let Some(Some(rig)) = self.rigs.get(&(car_key(s.car), si)) else { continue };
            // the sweep runs from full left lock to full right lock
            let last = rig.steer.last_frame();
            rig.steer.local(&rig.model.bones, (0.5 + 0.5 * p.steer.clamp(-1.0, 1.0)) * last, &mut local);
            if let Some(idle) = rig.idle_stop.as_ref() {
                if p.stopped > 0.01 {
                    idle.local(&rig.model.bones, idle.frame(self.time + s.id as f32 * 0.37, true), &mut other);
                    blend_pose(&mut local, &other, p.stopped);
                }
            }
            if let Some(rev) = rig.reverse.as_ref() {
                if p.reverse > 0.01 {
                    rev.local(&rig.model.bones, rev.last_frame(), &mut other);
                    blend_pose(&mut local, &other, p.reverse);
                }
            }
            // a scene's gesture over the driving pose, and the jaw flap over that
            if let Some(a) = self.acting.get_mut(&s.id) {
                if let Some((g, left, human)) = a.asked.take() {
                    let name = gesture_clip(g, &mut a.rr, human);
                    if rig.gestures.contains_key(&name) {
                        a.playing = Some(Playing { clip: name, t: 0.0, left });
                        a.leaving = None;
                    }
                }
                let mut over = false;
                if let Some(pl) = a.playing.as_mut() {
                    if let Some(c) = rig.gestures.get(&pl.clip) {
                        pl.t += dt;
                        pl.left -= dt;
                        let f = c.frame(pl.t, false);
                        c.local(&rig.model.bones, f, &mut other);
                        // (the gesture clips animate the arms, head and upper body only: laid over the
                        // driving pose on the bones they move)
                        overlay(&mut local, &other, c, (pl.t / GESTURE_BLEND).min(1.0));
                        over = pl.t >= c.anim.duration || pl.left <= 0.0;
                        if over {
                            a.leaving = Some((pl.clip.clone(), f, 1.0));
                        }
                    } else {
                        over = true;
                    }
                }
                if over {
                    a.playing = None;
                }
                if a.playing.is_none() {
                    if let Some((name, f, w)) = a.leaving.as_mut() {
                        *w -= dt / GESTURE_BLEND;
                        if let Some(c) = rig.gestures.get(name.as_str()).filter(|_| *w > 0.0) {
                            c.local(&rig.model.bones, *f, &mut other);
                            overlay(&mut local, &other, c, *w);
                        }
                    }
                    if a.leaving.as_ref().map_or(false, |l| l.2 <= 0.0) {
                        a.leaving = None;
                    }
                }
                a.jaw_k = (a.jaw_k + (if a.jaw { dt } else { -dt }) / GESTURE_BLEND).clamp(0.0, 1.0);
                if a.jaw {
                    a.jaw_t += dt;
                }
                if let (true, Some(c), Some(head)) = (a.jaw_k > 0.0, rig.gestures.get("drv_gest_jawflap"), rig.model.head) {
                    // (the shared jaw flap clip is the Challenger's whole driver; laid over the bones
                    // below the head only - the jaw. The original's overlay mask is not read.)
                    c.local(&rig.model.bones, c.frame(a.jaw_t, true), &mut other);
                    let bones = &rig.model.bones;
                    let below = |mut i: usize| -> bool {
                        while let Some(p) = bones.get(i).and_then(|b| b.parent) {
                            if p == head {
                                return true;
                            }
                            if p == i {
                                break;
                            }
                            i = p;
                        }
                        false
                    };
                    let mut masked = local.clone();
                    overlay(&mut masked, &other, c, a.jaw_k);
                    for i in 0..local.len() {
                        if below(i) {
                            local[i] = masked[i];
                        }
                    }
                }
            }
            // (the cockpit view shows the arms and the wheel, not the driver's own head -
            // `ref/cockpit_view.png`)
            if s.player && self.headless {
                self.buf.hide_from = rig.model.head;
            }
            self.buf.emit(&rig.model, 0, &local, s.mat);
            if s.player {
                self.player_head = rig.model.head.and_then(|h| self.buf.bone_pos(h)).map(|w| s.mat.inverse().transform_point3(w));
            }
        }
        if self.frame == 50 {
            log::info!("drivers: {} seats, {} rigs, {} poses", seats.len(), self.rigs.len(), self.poses.len());
            for s in seats {
                log::info!("  seat {} {} dist {:.1}", s.id, s.car, s.pos.distance(cam));
            }
        }
        self.buf.upload(gpu);
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, draws: &mut usize) {
        self.buf.draw(pass, gpu, false, draws);
    }
}
