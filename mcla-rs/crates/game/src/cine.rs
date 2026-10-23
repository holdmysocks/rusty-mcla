//! The game's cutscene animation (`resources/animation/cineanimpacks/*.r1`): every scene
//! is a set of animations sharing a name prefix -
//! `story/cut_story_race_gh_06/CSCamera` (camera position, rotation, field of view),
//! `.../Entity/mover_drv00_car/..` (a car's path: mover translation / rotation),
//! `.../Entity/booke_drv01_name_drv_mb_04_set/..` (a character: the set to use is named
//! after `_name_`, the root bone carries the position in the scene).
//! Story scenes are authored in world coordinates; race start scenes relative to the grid.
//! The camera looks along its local -Z with +Y up; the field of view is vertical degrees.
//! Scenes that can happen anywhere (arrests, the winner's scene) are relative to the
//! player's car: its mover gives the car's place in the scene, which fixes the origin.
//! Their people have no set in the name (`cut_drv01_cop`, `cut_drv01_f`, `player_drv00`):
//! police, the woman starting the race, the player.
//! Hangouts: `hangout/cut_hangout_NN/<f|m>_pedNN_loop` are the people standing around there
//! (looping, world coordinates), `.._ent_01` / `.._ext` the scenes of pulling in and leaving.
//! `<scene>_IM` holds the facial capture of the speaking characters (same entity names,
//! starting with the scene; it repeats the body in local space, so only the bones the
//! body animation leaves alone are taken from it), `hangout/<scene>_peds` the bystanders.

use crate::assets::load_resource_file;
use crate::render::Gpu;
use crate::skin::{Clip, FaceShapes, SkinBuffer, SkinModel};
use glam::{Mat4, Quat, Vec3};
use hashbrown::HashMap;
use mcla_formats::rage::anim::{animation_named, pack_entries, Animation, KIND_FOV, KIND_MOVER_ROTATION, KIND_MOVER_TRANSLATION, KIND_ROTATION, KIND_TRANSLATION};
use mcla_formats::rage::Rd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Actor {
    label: String,
    model: Arc<SkinModel>,
    clip: Clip,
    /// facial animation
    face: Option<Clip>,
    /// the face's blend shapes and the facial pack's kind-21 track of every target (by the target's id)
    shapes: Option<(FaceShapes, Vec<Option<usize>>)>,
}

/// Track kind of a blend-shape weight (`crCreatureComponentBlendShapes` reads kind 21 by target id,
/// `sub_8253EE48`).
const KIND_BLEND_SHAPE: u8 = 21;

struct Mover {
    /// "drv00", "henry", "omnibus"
    label: String,
    anim: Arc<Animation>,
}

pub struct Scene {
    pub name: String,
    camera: Option<Arc<Animation>>,
    actors: Vec<Actor>,
    movers: Vec<Mover>,
    pub duration: f32,
    pub t: f32,
    /// scene space -> world
    pub origin: Mat4,
}

#[derive(Default)]
pub struct Cine {
    char_dir: PathBuf,
    /// scene ("story/cut_intro") -> (entry name, pack file)
    index: HashMap<String, Vec<(String, PathBuf)>>,
    models: HashMap<String, Option<Arc<SkinModel>>>,
    pub playing: Option<Scene>,
    buf: SkinBuffer,
    /// people of the hangout nearby: each loops at its own length
    ambient: Vec<Actor>,
    ambient_name: String,
    clock: f32,
    /// a start scene after its cut to the game camera: its script runs on to the clip's end, so its people
    /// stay where they are and go on moving (work/career2/cap/README.md, finding 5: the starter is still in
    /// the game view for a few seconds after the cut)
    linger: Option<Scene>,
}

/// The car movers that stand for the player's own car.
pub fn is_player_car(label: &str) -> bool {
    matches!(label, "drv00" | "player" | "player_drv00" | "playergarage" | "delivery")
}

/// Character set of a scene entity that does not name one.
fn default_set(scene: &str, entity: &str, n: usize) -> &'static str {
    const COPS: [&str; 4] = ["pol_mc_01_set", "pol_mb_01_set", "pol_mc_02_set", "pol_mc_03_set"];
    const RACERS: [&str; 4] = ["drv_mb_02_set", "drv_mc_02_set", "drv_ma_02_set", "drv_mh_02_set"];
    let player = entity.starts_with("player") || entity == "cut_drv00";
    if entity.contains("cop") || (scene.contains("cut_cop") && !player) {
        // in the police scenes everybody but the player is an officer
        COPS[n % COPS.len()]
    } else if entity.ends_with("_f") {
        "drv_fa_001_set"
    } else if player {
        "drv_mp_01_set"
    } else {
        RACERS[n % RACERS.len()]
    }
}

/// "story/cut_intro/Entity/x/x" -> "story/cut_intro"
fn scene_of(entry: &str) -> String {
    let l = entry.to_ascii_lowercase();
    let cut = l.find("/entity/").or_else(|| l.find("/cscamera")).unwrap_or(l.len());
    l[..cut].to_string()
}

impl Cine {
    pub fn load(data_dir: &str) -> Cine {
        let root = Path::new(data_dir);
        let mut c = Cine { char_dir: root.join("resources/character"), ..Default::default() };
        // which pack holds what: read once, then kept in a small index file
        let index = root.join("cine_index.tsv");
        let mut lines: Vec<(String, PathBuf)> = Vec::new();
        if let Ok(t) = std::fs::read_to_string(&index) {
            for line in t.lines() {
                if let Some((k, f)) = line.split_once('\t') {
                    lines.push((k.to_string(), PathBuf::from(f)));
                }
            }
        }
        if lines.is_empty() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("resources/animation/cineanimpacks")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            files.sort();
            let mut out = String::new();
            for f in files {
                let Ok(res) = load_resource_file(&f) else { continue };
                for (_, name) in pack_entries(&res) {
                    out.push_str(&format!("{}\t{}\n", name, f.display()));
                    lines.push((name, f.clone()));
                }
            }
            let _ = std::fs::write(&index, out);
        }
        for (name, f) in lines {
            c.index.entry(scene_of(&name)).or_default().push((name, f));
        }
        log::info!("cine: {} scenes", c.index.len());
        c
    }

    pub fn has(&self, scene: &str) -> bool {
        self.index.contains_key(&scene.to_ascii_lowercase())
    }

    /// Scenes whose name starts with `prefix` (sorted).
    pub fn scenes(&self, prefix: &str) -> Vec<String> {
        let p = prefix.to_ascii_lowercase();
        let mut v: Vec<String> = self.index.keys().filter(|k| k.starts_with(&p)).cloned().collect();
        v.sort();
        v
    }

    fn model(&mut self, gpu: &mut Gpu, set: &str) -> Option<Arc<SkinModel>> {
        if !self.models.contains_key(set) {
            let dir = self.char_dir.join(set);
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "r6").unwrap_or(false)).collect()).unwrap_or_default();
            files.sort();
            let m = files.first().and_then(|f| SkinModel::load(gpu, set, f)).map(Arc::new);
            if m.is_none() {
                log::warn!("cine: could not load {}", set);
            }
            self.models.insert(set.to_string(), m);
        }
        self.models[set].clone()
    }

    /// Load a scene and start it at `origin` (identity for story scenes).
    pub fn start(&mut self, gpu: &mut Gpu, scene: &str, origin: Mat4) -> bool {
        let key = scene.to_ascii_lowercase();
        if !self.index.contains_key(&key) {
            return false;
        }
        let mut sc = Scene { name: key.clone(), camera: None, actors: Vec::new(), movers: Vec::new(), duration: 0.0, t: 0.0, origin };
        // the scene itself, its bystanders, then the faces
        let short = key.rsplit('/').next().unwrap_or("").to_string();
        let parts = [(key.clone(), false), (format!("hangout/{short}_peds"), false), (format!("{key}_im"), true)];
        for (part, faces) in parts {
            let Some(entries) = self.index.get(&part).cloned() else { continue };
            let mut by_file: HashMap<PathBuf, Vec<String>> = HashMap::new();
            for (name, f) in entries {
                by_file.entry(f).or_default().push(name);
            }
            for (f, names) in by_file {
                let Ok(res) = load_resource_file(&f) else { continue };
                let rd = Rd::new(&res);
                for (o, name) in pack_entries(&res) {
                    if !names.contains(&name) {
                        continue;
                    }
                    let Some(anim) = animation_named(&rd, o) else {
                        log::warn!("cine: {} does not decode", name);
                        continue;
                    };
                    let anim = Arc::new(anim);
                    let l = name.to_ascii_lowercase();
                    // ".../Entity/<entity>/<entity>"
                    let ent = l.rsplit('/').next().unwrap_or("").to_string();
                    if faces {
                        if let Some((label, _)) = ent.split_once("_name_") {
                            if let Some(a) = sc.actors.iter_mut().find(|a| a.label == label) {
                                a.face = Some(Clip::bind(&anim, &a.model.ids, false, false));
                                if let Some(m) = a.model.morph.as_ref() {
                                    let tracks = m.shapes.targets.iter().map(|t| anim.tracks.iter().position(|k| k.kind == KIND_BLEND_SHAPE && k.bone_id == t.id)).collect();
                                    a.shapes = Some((FaceShapes::new(m), tracks));
                                }
                            }
                        }
                        continue;
                    }
                    sc.duration = sc.duration.max(anim.duration);
                    if l.ends_with("/cscamera") {
                        sc.camera = Some(anim);
                        continue;
                    }
                    if let Some(rest) = ent.strip_prefix("mover_") {
                        if let Some(label) = rest.strip_suffix("_car") {
                            sc.movers.push(Mover { label: label.to_string(), anim });
                            continue;
                        }
                    }
                    if ent.ends_with("_car") {
                        continue;
                    }
                    let (label, set) = match ent.split_once("_name_") {
                        Some((l, s)) => (l, s.to_string()),
                        None => (ent.as_str(), default_set(&key, &ent, sc.actors.len()).to_string()),
                    };
                    // hand props without a path of their own (guns) hang off a hand bone
                    // in the original; on their own they would float where the scene starts
                    if set.starts_with("prp_") && anim.tracks.len() <= 4 {
                        continue;
                    }
                    let Some(model) = self.model(gpu, &set) else { continue };
                    let clip = Clip::bind(&anim, &model.ids, false, false);
                    sc.actors.push(Actor { label: label.to_string(), model, clip, face: None, shapes: None });
                }
            }
        }
        if sc.camera.is_none() && sc.actors.is_empty() && sc.movers.is_empty() {
            return false;
        }
        sc.actors.sort_by(|a, b| a.label.cmp(&b.label));
        log::info!("cine: {} ({:.1} s): camera {}, {} characters [{}] ({} with faces, {} with blend shapes: {} weight tracks), cars [{}]", key, sc.duration, sc.camera.is_some(), sc.actors.len(), sc.actors.iter().map(|a| a.label.as_str()).collect::<Vec<_>>().join(" "), sc.actors.iter().filter(|a| a.face.is_some()).count(), sc.actors.iter().filter(|a| a.shapes.is_some()).count(), sc.actors.iter().filter_map(|a| a.shapes.as_ref()).map(|s| s.1.iter().flatten().count()).sum::<usize>(), sc.movers.iter().map(|m| m.label.as_str()).collect::<Vec<_>>().join(" "));
        self.playing = Some(sc);
        true
    }

    /// Show the loops of a hangout ("hangout/cut_hangout_01"), or none.
    pub fn set_ambient(&mut self, gpu: &mut Gpu, name: Option<&str>) {
        let name = name.unwrap_or("").to_ascii_lowercase();
        if name == self.ambient_name {
            return;
        }
        self.ambient.clear();
        self.ambient_name = name.clone();
        if name.is_empty() {
            return;
        }
        const FEMALE: [&str; 4] = ["ped_fa_001_set", "ped_fb_002_set", "ped_fc_001_set", "ped_fh_001_set"];
        const MALE: [&str; 5] = ["ped_ma_001_set", "ped_mb_001_set", "ped_mc_001_set", "ped_mh_001_set", "ped_ma_003_set"];
        const DRIVERS: [&str; 4] = ["drv_mb_02_set", "drv_mc_02_set", "drv_ma_02_set", "drv_mh_02_set"];
        for (k, key) in self.scenes(&format!("{name}/")).into_iter().enumerate() {
            let Some((entry, f)) = self.index.get(&key).and_then(|e| e.first().cloned()) else { continue };
            let Ok(res) = load_resource_file(&f) else { continue };
            let rd = Rd::new(&res);
            let Some((o, _)) = pack_entries(&res).into_iter().find(|(_, n)| *n == entry) else { continue };
            let Some(anim) = animation_named(&rd, o) else { continue };
            let label = key.rsplit('/').next().unwrap_or("").to_string();
            // the racers' skeleton has more bones than the pedestrians'
            let set = if anim.tracks.len() > 80 {
                DRIVERS[k % DRIVERS.len()]
            } else if label.starts_with('f') {
                FEMALE[k % FEMALE.len()]
            } else {
                MALE[k % MALE.len()]
            };
            let Some(model) = self.model(gpu, set) else { continue };
            let clip = Clip::bind(&Arc::new(anim), &model.ids, false, false);
            self.ambient.push(Actor { label, model, clip, face: None, shapes: None });
        }
        log::info!("cine: {} people at {}", self.ambient.len(), name);
    }

    pub fn tick(&mut self, dt: f32) {
        self.clock += dt;
        if let Some(sc) = self.linger.as_mut() {
            sc.t += dt;
            if sc.t >= sc.duration {
                self.linger = None;
            }
        }
    }

    /// The scene hands the camera back to the game but its people play on to the clip's end.
    pub fn release(&mut self) {
        self.linger = self.playing.take().filter(|sc| !sc.actors.is_empty() && sc.t < sc.duration);
    }

    /// The lingering people go too (the race is over or another scene begins).
    pub fn drop_linger(&mut self) {
        self.linger = None;
    }

    /// The labels of a scene's car movers.
    pub fn mover_labels(&self, scene: &str) -> Vec<String> {
        let Some(entries) = self.index.get(&scene.to_ascii_lowercase()) else { return Vec::new() };
        entries.iter().filter_map(|(name, _)| name.to_ascii_lowercase().rsplit('/').next().and_then(|e| e.strip_prefix("mover_")).and_then(|r| r.strip_suffix("_car")).map(|l| l.to_string())).collect()
    }

    /// Where a scene puts a car (first or last frame), without playing it: the garage
    /// scenes know where the car stands inside, the hangout scenes where one pulls in.
    pub fn car_pose(&self, scene: &str, want: &dyn Fn(&str) -> bool, end: bool) -> Option<(Vec3, Quat)> {
        let entries = self.index.get(&scene.to_ascii_lowercase())?;
        for (name, f) in entries {
            let l = name.to_ascii_lowercase();
            let ent = l.rsplit('/').next().unwrap_or("");
            let Some(label) = ent.strip_prefix("mover_").and_then(|r| r.strip_suffix("_car")) else { continue };
            if !want(label) {
                continue;
            }
            let res = load_resource_file(f).ok()?;
            let rd = Rd::new(&res);
            let (o, _) = pack_entries(&res).into_iter().find(|(_, n)| n == name)?;
            let a = animation_named(&rd, o)?;
            let fr = if end { (a.frames.max(1) - 1) as f32 } else { 0.0 };
            let p = a.track(KIND_MOVER_TRANSLATION, 0).map(|t| t.data.vec(fr)).unwrap_or(Vec3::ZERO) + a.track(KIND_TRANSLATION, 0).map(|t| t.data.vec(fr)).unwrap_or(Vec3::ZERO);
            let r = a.track(KIND_MOVER_ROTATION, 0).map(|t| t.data.rot(fr)).unwrap_or(Quat::IDENTITY) * a.track(KIND_ROTATION, 0).map(|t| t.data.rot(fr)).unwrap_or(Quat::IDENTITY);
            return Some((p, r.normalize()));
        }
        None
    }

    /// Start a scene around the player's car (`car` = its world transform).
    pub fn start_bound(&mut self, gpu: &mut Gpu, scene: &str, car: Mat4) -> bool {
        if !self.start(gpu, scene, Mat4::IDENTITY) {
            return false;
        }
        let label = self.movers().into_iter().find(|l| is_player_car(l));
        let local = label.and_then(|l| self.mover(&l)).map(|(p, r)| Mat4::from_rotation_translation(r, p)).unwrap_or(Mat4::IDENTITY);
        if let Some(sc) = self.playing.as_mut() {
            sc.origin = car * local.inverse();
        }
        true
    }

    pub fn stop(&mut self) {
        self.playing = None;
        self.buf.clear();
    }

    /// A scene's camera at scene time `t` without starting it (the race-start warp flies to the start
    /// scene's camera at t 1.0 before the scene plays - `sub_823C0D10`): (eye, forward, up, vertical
    /// field of view in radians) through `origin`.
    pub fn peek_camera(&self, scene: &str, origin: Mat4, t: f32) -> Option<(Vec3, Vec3, Vec3, f32)> {
        let key = scene.to_ascii_lowercase();
        for (name, f) in self.index.get(&key)? {
            if !name.to_ascii_lowercase().ends_with("/cscamera") {
                continue;
            }
            let Ok(res) = load_resource_file(f) else { continue };
            let rd = Rd::new(&res);
            for (o, n) in pack_entries(&res) {
                if n != *name {
                    continue;
                }
                let anim = animation_named(&rd, o)?;
                let sc = Scene { name: key.clone(), camera: Some(Arc::new(anim)), actors: Vec::new(), movers: Vec::new(), duration: 0.0, t, origin };
                return Self::scene_camera(&sc);
            }
        }
        None
    }

    /// Camera at the scene's time: (eye, forward, up, vertical field of view in radians).
    pub fn camera(&self) -> Option<(Vec3, Vec3, Vec3, f32)> {
        Self::scene_camera(self.playing.as_ref()?)
    }

    fn scene_camera(sc: &Scene) -> Option<(Vec3, Vec3, Vec3, f32)> {
        let cam = sc.camera.as_ref()?;
        let f = cam.frame_at(sc.t);
        let pos = cam.track(KIND_TRANSLATION, 0)?.data.vec(f);
        let rot = cam.track(KIND_ROTATION, 0).map(|t| t.data.rot(f)).unwrap_or(Quat::IDENTITY);
        // (the lens track is a focal length in mm on a 35 mm gate: cam+104 = 2 atan(0.7085 in /
        // (v mm * 0.03937)), the horizontal angle (sub_823CDAE8); the render camera's vertical
        // angle is that divided by the viewport's aspect, 16:9 (sub_822C0320, the warp's
        // target, cam_to_menu sub_82206F18). Without the track the camera keeps its reset
        // lens, pi/4.)
        let h = cam.track(KIND_FOV, 0).map(|t| t.data.vec(f).x).filter(|v| *v > 0.0).map_or(std::f32::consts::FRAC_PI_4, |v| 2.0 * (0.7085 / (v * 0.03937)).atan());
        let fov = h / (16.0 / 9.0);
        Some((sc.origin.transform_point3(pos), sc.origin.transform_vector3(rot * -Vec3::Z).normalize_or(Vec3::Z), sc.origin.transform_vector3(rot * Vec3::Y).normalize_or(Vec3::Y), fov))
    }

    /// Labels of the cars the scene moves.
    /// Where the playing scene's characters stand now: (label, root bone in world space).
    pub fn actor_roots(&self) -> Vec<(String, Vec3)> {
        let Some(sc) = self.playing.as_ref() else { return Vec::new() };
        let mut local: Vec<(Quat, Vec3)> = Vec::new();
        sc.actors.iter().map(|a| {
            let f = a.clip.anim.frame_at(sc.t);
            a.clip.local(&a.model.bones, f, &mut local);
            let (mt, mr) = a.clip.mover(f);
            let root = local.first().map_or(Vec3::ZERO, |l| l.1);
            (a.label.clone(), (sc.origin * Mat4::from_rotation_translation(mr, mt)).transform_point3(root))
        }).collect()
    }

    pub fn movers(&self) -> Vec<String> {
        self.playing.as_ref().map(|s| s.movers.iter().map(|m| m.label.clone()).collect()).unwrap_or_default()
    }

    /// The animation that moves a car of the playing scene, and the scene's frame in the world.
    pub fn mover_anim(&self, label: &str) -> Option<(Arc<Animation>, Mat4)> {
        let sc = self.playing.as_ref()?;
        sc.movers.iter().find(|m| m.label == label).map(|m| (m.anim.clone(), sc.origin))
    }

    /// World position and orientation of a moved car at the scene's time.
    pub fn mover(&self, label: &str) -> Option<(Vec3, Quat)> {
        let sc = self.playing.as_ref()?;
        let m = sc.movers.iter().find(|m| m.label == label)?;
        let f = m.anim.frame_at(sc.t);
        let p = m.anim.track(KIND_MOVER_TRANSLATION, 0).map(|t| t.data.vec(f)).unwrap_or(Vec3::ZERO) + m.anim.track(KIND_TRANSLATION, 0).map(|t| t.data.vec(f)).unwrap_or(Vec3::ZERO);
        let r = m.anim.track(KIND_MOVER_ROTATION, 0).map(|t| t.data.rot(f)).unwrap_or(Quat::IDENTITY) * m.anim.track(KIND_ROTATION, 0).map(|t| t.data.rot(f)).unwrap_or(Quat::IDENTITY);
        let (_, or, _) = sc.origin.to_scale_rotation_translation();
        Some((sc.origin.transform_point3(p), (or * r).normalize()))
    }

    /// True if the scene shows the player's character on foot.
    pub fn has_actor(&self, prefix: &str) -> bool {
        self.playing.as_ref().map_or(false, |s| s.actors.iter().any(|a| a.label.starts_with(prefix)))
    }

    /// The light manager's state for the character programs this frame (main.rs); None: the
    /// figures keep the generic look (MCLA_CHAR_OLD=1).
    pub fn set_light(&mut self, l: Option<crate::skin::CharLight>) {
        self.buf.char_light = l;
    }

    /// Skin the characters at the scene's time.
    pub fn prepare(&mut self, gpu: &Gpu) {
        self.buf.clear();
        let mut local: Vec<(Quat, Vec3)> = Vec::new();
        for a in &self.ambient {
            let last = a.clip.last_frame();
            let f = if a.clip.anim.duration > 0.0 && last > 0.0 { (self.clock / a.clip.anim.duration * last).rem_euclid(last) } else { 0.0 };
            a.clip.local(&a.model.bones, f, &mut local);
            let (mt, mr) = a.clip.mover(f);
            // (the hangout's people: bystanders, the manager's own lights)
            self.buf.next_bystander = true;
            self.buf.emit(&a.model, 0, &local, Mat4::from_rotation_translation(mr, mt));
        }
        for sc in self.playing.iter_mut().chain(self.linger.iter_mut()) {
        for a in &mut sc.actors {
            let f = a.clip.anim.frame_at(sc.t);
            a.clip.local(&a.model.bones, f, &mut local);
            if let Some(face) = a.face.as_ref() {
                let ff = face.anim.frame_at(sc.t);
                if let (Some((shapes, tracks)), Some(m)) = (a.shapes.as_mut(), a.model.morph.as_ref()) {
                    // a target without a track keeps its weight (sub_8253EE48 writes only what the frame has)
                    for (i, t) in tracks.iter().enumerate() {
                        if let Some(t) = *t {
                            shapes.set(i, face.anim.tracks[t].data.vec(ff).x);
                        }
                    }
                    shapes.update(m);
                    if std::env::var("MCLA_BS_TRACE").is_ok() {
                        let (n, d) = shapes.stats();
                        log::info!("blend shapes {} t {:.2}: {} targets on, largest delta {:.4} m", a.label, sc.t, n, d);
                    }
                }
                for i in 0..local.len() {
                    if a.clip.rot[i].is_none() {
                        if let Some(t) = face.rot[i] {
                            local[i].0 = face.anim.tracks[t].data.rot(ff);
                        }
                    }
                    if a.clip.pos[i].is_none() {
                        if let Some(t) = face.pos[i] {
                            local[i].1 = face.anim.tracks[t].data.vec(ff);
                        }
                    }
                }
            }
            let (mt, mr) = a.clip.mover(f);
            // (sub_823A10A8: a cine character whose name holds "ped" is drawn with the manager's own
            // lights - the scenes' bystanders pedsit1_ped00, benchsit_ped05 ...; the cast with three
            // lights of its own)
            self.buf.next_bystander = a.label.to_ascii_lowercase().contains("ped");
            self.buf.emit_shaped(&a.model, 0, &local, sc.origin * Mat4::from_rotation_translation(mr, mt), a.shapes.as_ref().map(|s| &s.0));
        }
        }
        self.buf.upload(gpu);
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, shadow: bool, draws: &mut usize) {
        if self.playing.is_some() || !self.ambient.is_empty() {
            self.buf.draw(pass, gpu, shadow, draws);
        }
    }
}
