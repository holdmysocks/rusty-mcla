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
/// `F-77597`).
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

impl Scene {
    /// The length the event-time rule compares with: the scene camera's own clip (header +12), else the scene's.
    pub fn camera_len(&self) -> f32 {
        self.camera.as_ref().map_or(self.duration, |c| c.duration)
    }

    /// The cutscene form of the cascades (F-34053's second branch): the cine core script (0x95ca1d1b 0xD1D, 0xE1F) pushes
    /// every character and car event it launches into kill buffer 1 (CineScript_PushKillBuffer, F-73266), which sets the
    /// renderer's +389 until F-60657 clears the buffer at the scene's end; the form is skipped while the scene's name holds
    /// "racestart" but neither "red" nor "payback" (cine manager +319, F-81251; +335, CineScript_SetRaceStarterLighting, is
    /// called by no script).
    pub fn close_shadows(&self) -> bool {
        let n = self.name.to_ascii_lowercase();
        let racestart = n.contains("racestart") && !n.contains("red") && !n.contains("payback");
        (!self.actors.is_empty() || !self.movers.is_empty()) && !racestart
    }
}

/// The character sets of the hangouts' people (`Cine::set_ambient`).
const AMBIENT_FEMALE: [&str; 4] = ["ped_fa_001_set", "ped_fb_002_set", "ped_fc_001_set", "ped_fh_001_set"];
const AMBIENT_MALE: [&str; 5] = ["ped_ma_001_set", "ped_mb_001_set", "ped_mc_001_set", "ped_mh_001_set", "ped_ma_003_set"];
const AMBIENT_DRIVERS: [&str; 4] = ["drv_mb_02_set", "drv_mc_02_set", "drv_ma_02_set", "drv_mh_02_set"];

#[derive(Default)]
pub struct Cine {
    char_dir: PathBuf,
    /// scene ("story/cut_intro") -> (entry name, pack file)
    index: HashMap<String, Vec<(String, PathBuf)>>,
    models: HashMap<String, Option<Arc<SkinModel>>>,
    /// character sets read ahead on worker threads (`read_ahead_ambient`): the file and what it holds
    reading: HashMap<String, std::thread::JoinHandle<Option<(PathBuf, mcla_formats::rage::character::Character)>>>,
    /// a story scene's animation packs read ahead on a worker thread (`read_ahead_scene`), taken by `start`
    packs: HashMap<PathBuf, std::thread::JoinHandle<Option<mcla_formats::rsc::Resource>>>,
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

/// The story scenes' skip and cut events, from their generated scripts (`Game/CineScripts/generated/story/
/// cut_story_*`, CineScript_Events_LaunchEvent): the scene, its `bookmark` (73), `bookmark_cut_to_game` (72)
/// and `cut_to_game` (74) times. A scene not listed has none (cut_story_motorcycle_rep_A / _B, cut_intro).
pub const STORY_MARKS: &[(&str, &[(u8, f32)])] = &[
    ("story/cut_story_payback_intro", &[(73, 66.3333)]),
    ("story/cut_story_find_sh_04_alt", &[(73, 61.5)]),
    ("story/cut_story_hangout_series_hollywoodc", &[(72, 56.0)]),
    ("story/cut_story_hangout_pinkslip_downtown", &[(73, 43.3667)]),
    ("story/cut_story_city_champ_night", &[(73, 38.0)]),
    ("story/cut_story_district_champs_night", &[(72, 74.3333)]),
    ("story/cut_story_find_sh_02c", &[(73, 105.0)]),
    ("story/cut_story_race_sh_01", &[(73, 3.33333)]),
    ("story/cut_story_hangout_series_downtown", &[(73, 41.1)]),
    ("story/cut_story_motorcycle_rep", &[(72, 48.3333)]),
    ("story/cut_story_bestof_sh_05", &[(73, 41.6667)]),
    ("story/cut_story_district_champs", &[(72, 74.3333)]),
    ("story/cut_story_find_sh_04", &[(72, 58.3333)]),
    ("story/cut_story_rep_on_strip_garage", &[(73, 80.3333)]),
    ("story/cut_story_races_in_beach", &[(72, 41.8333)]),
    ("story/cut_story_hangout_pinkslip_hills", &[(72, 69.0)]),
    ("story/cut_story_city_champ", &[(73, 38.0)]),
    ("story/cut_story_hollywood_garage_intro", &[(73, 63.0)]),
    ("story/cut_story_race_gh_06", &[(73, 57.6333), (74, 64.1667)]),
    ("story/cut_story_beat_sh_02_exotic", &[(73, 35.2333)]),
];

/// An event's time as Events_LaunchEvent files it (`F-73781`): countdown (58), bookmark_cut_to_game (72)
/// and cut_to_game (74) at or past the camera clip's length (its header +12, `max(len, 0.1)`) are moved 0.1 s earlier.
fn mark_time(id: u8, t: f32, cam_len: f32) -> f32 {
    // (fsel: len - 0.1 >= 0 ? len : 0.1)
    if matches!(id, 58 | 72 | 74) && t >= if cam_len - 0.1 >= 0.0 { cam_len } else { 0.1 } {
        t - 0.1
    } else {
        t
    }
}

/// Where a skip takes a story scene (the cine manager's skip request +305: F-19408 -> F-61676):
/// the earliest bookmark / bookmark_cut_to_game / countdown of the scene, if the clock is not past it yet
/// (the clock jumps there and the scene plays on; with none, or past it, the skip does nothing).
pub fn skip_to(scene: &str, t: f32, cam_len: f32) -> Option<f32> {
    let marks = STORY_MARKS.iter().find(|m| m.0.eq_ignore_ascii_case(scene))?.1;
    let at = marks.iter().filter(|m| matches!(m.0, 58 | 72 | 73)).map(|m| mark_time(m.0, m.1, cam_len)).fold(f32::INFINITY, f32::min);
    (at.is_finite() && at > t).then_some(at)
}

/// When a story scene hands the picture to the game camera (bookmark_cut_to_game / cut_to_game: F-45212,
/// a hard cut); the scene's clock and cars run on to its end.
pub fn cut_to_game(scene: &str, cam_len: f32) -> Option<f32> {
    let marks = STORY_MARKS.iter().find(|m| m.0.eq_ignore_ascii_case(scene))?.1;
    marks.iter().filter(|m| matches!(m.0, 72 | 74)).map(|m| mark_time(m.0, m.1, cam_len)).reduce(f32::min)
}

/// The car movers that stand for the player's own car.
pub fn is_player_car(label: &str) -> bool {
    matches!(label, "drv00" | "player" | "player_drv00" | "playergarage" | "delivery")
}

/// The driver sets Boot.sco registers with a cutscene copy (CineScript_Characters_RegisterDriverMappings, its sixth
/// argument: "<set>_HackedScale" - native F-67461, the map at the manager's +116 / +128). The copy is the same
/// figure about 7.6 % taller, the rig the scenes' face packs were made on; every other set plays as itself.
const HACKED_SCALE: [&str; 14] = [
    "drv_fc_002_set", "drv_fc_003_set", "drv_ma_001_set", "drv_ma_03_set", "drv_mb_001_set", "drv_mb_008_set", "drv_mb_04_set",
    "drv_mc_001_set", "drv_mc_003_set", "drv_mc_02_set", "drv_mh_006_set", "drv_mh_02_set", "drv_mp_01_set", "mec_mc_01_set",
];

/// The set a cutscene draws for `set` (LookupDriverToCutscenePath / Name, F-58832 / F-81189). Not ported:
/// a slot marked to use the unscaled character (CineScript_Characters_GetSlotToUseUnscaledCharacter) and the jacket
/// copy (`drv_mp_jacket_01_set_hackedscale`, chosen by F-17373 - not decoded); GH_06 marks no slot.
fn cutscene_set(set: &str) -> String {
    if HACKED_SCALE.iter().any(|s| s.eq_ignore_ascii_case(set)) {
        format!("{}_hackedscale", set.to_ascii_lowercase())
    } else {
        set.to_string()
    }
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

/// A character set's first `.r6` read (on a worker thread: `read_ahead_ambient`, `read_ahead_scene`).
fn read_set(dir: &Path) -> Option<(PathBuf, mcla_formats::rage::character::Character)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "r6").unwrap_or(false)).collect()).unwrap_or_default();
    files.sort();
    let f = files.into_iter().next()?;
    let c = SkinModel::read(&f)?;
    Some((f, c))
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

    /// The hangouts' people's character sets (`set_ambient`) read on worker threads before the player reaches a
    /// hangout, so making them there does not stop the game; their models are made on first use as before
    /// (docs/PERFORMANCE.md 2026-10-23 (6)).
    pub fn read_ahead_ambient(&mut self) {
        for set in AMBIENT_FEMALE.iter().chain(AMBIENT_MALE.iter()).chain(AMBIENT_DRIVERS.iter()) {
            if self.models.contains_key(*set) || self.reading.contains_key(*set) {
                continue;
            }
            let dir = self.char_dir.join(set);
            if let Ok(h) = std::thread::Builder::new().name("people read".into()).spawn(move || read_set(&dir)) {
                self.reading.insert(set.to_string(), h);
            }
        }
    }

    /// A story scene's animation packs and its people's character sets read on worker threads before it starts, so
    /// that `start` finds them read (what it makes and when is unchanged - docs/PERFORMANCE.md 2026-10-25). Returns
    /// the labels of the cars it moves (`mover_<label>_car`), for the caller to read their cars ahead.
    pub fn read_ahead_scene(&mut self, scene: &str) -> Vec<String> {
        let key = scene.to_ascii_lowercase();
        let short = key.rsplit('/').next().unwrap_or("").to_string();
        let mut movers = Vec::new();
        let mut sets = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        for (part, faces) in [(key.clone(), false), (format!("hangout/{short}_peds"), false), (format!("{key}_im"), true)] {
            for (name, f) in self.index.get(&part).cloned().unwrap_or_default() {
                if !files.contains(&f) && !self.packs.contains_key(&f) {
                    files.push(f);
                }
                let l = name.to_ascii_lowercase();
                let ent = l.rsplit('/').next().unwrap_or("").to_string();
                if faces || l.ends_with("/cscamera") {
                    continue;
                }
                if let Some(label) = ent.strip_prefix("mover_").and_then(|r| r.strip_suffix("_car")) {
                    movers.push(label.to_string());
                    continue;
                }
                if ent.ends_with("_car") {
                    continue;
                }
                // (as `start` names them; an entity without a set of its own gets `default_set`'s by its place among
                // the actors, which only `start` knows. In the police scenes: every set that place could give it, the
                // officers' four - docs/PERFORMANCE.md 2026-10-27 (11))
                if let Some((_, s)) = ent.split_once("_name_") {
                    sets.push(cutscene_set(s));
                } else if key.contains("cut_cop") {
                    for n in 0..4 {
                        let s = cutscene_set(default_set(&key, &ent, n));
                        if !sets.contains(&s) {
                            sets.push(s);
                        }
                    }
                }
            }
        }
        for f in files {
            let p = f.clone();
            if let Ok(h) = std::thread::Builder::new().name("scene read".into()).spawn(move || load_resource_file(&p).ok()) {
                self.packs.insert(f, h);
            }
        }
        for set in sets {
            if self.models.contains_key(&set) || self.reading.contains_key(&set) {
                continue;
            }
            let dir = self.char_dir.join(&set);
            if let Ok(h) = std::thread::Builder::new().name("people read".into()).spawn(move || read_set(&dir)) {
                self.reading.insert(set, h);
            }
        }
        log::info!("cine: reading {} ahead ({} cars)", key, movers.len());
        movers
    }

    /// An animation pack: the one read ahead (`read_ahead_scene`), else read now.
    fn pack(&mut self, f: &Path) -> Option<mcla_formats::rsc::Resource> {
        match self.packs.remove(f) {
            Some(h) => h.join().ok().flatten(),
            None => load_resource_file(f).ok(),
        }
    }

    fn model(&mut self, gpu: &mut Gpu, set: &str) -> Option<Arc<SkinModel>> {
        if let Some(h) = self.reading.remove(set).filter(|_| !self.models.contains_key(set)) {
            let m = h.join().ok().flatten().and_then(|(f, c)| SkinModel::from_character(gpu, set, &f, c)).map(Arc::new);
            if m.is_none() {
                log::warn!("cine: could not load {}", set);
            }
            self.models.insert(set.to_string(), m);
        }
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
        let mut got: HashMap<PathBuf, Arc<mcla_formats::rsc::Resource>> = HashMap::new();
        for (part, faces) in parts {
            let Some(entries) = self.index.get(&part).cloned() else { continue };
            let mut by_file: HashMap<PathBuf, Vec<String>> = HashMap::new();
            for (name, f) in entries {
                by_file.entry(f).or_default().push(name);
            }
            for (f, names) in by_file {
                // (the scene and its faces are mostly in one pack: read once)
                let res = match got.get(&f) {
                    Some(r) => r.clone(),
                    None => {
                        let Some(r) = self.pack(&f) else { continue };
                        let r = Arc::new(r);
                        got.insert(f.clone(), r.clone());
                        r
                    }
                };
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
                    // (the cutscene loader script, 0x95ca1d1b 0x4f - 0x1d8: a slot not marked for the unscaled
                    // character takes its set's cutscene copy - LookupDriverToCutscenePath / Name)
                    let set = cutscene_set(&set);
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
        const FEMALE: [&str; 4] = AMBIENT_FEMALE;
        const MALE: [&str; 5] = AMBIENT_MALE;
        const DRIVERS: [&str; 4] = AMBIENT_DRIVERS;
        let t0 = std::time::Instant::now();
        let mut packs: std::collections::HashMap<std::path::PathBuf, _> = std::collections::HashMap::new();
        for (k, key) in self.scenes(&format!("{name}/")).into_iter().enumerate() {
            let Some((entry, f)) = self.index.get(&key).and_then(|e| e.first().cloned()) else { continue };
            // (the scenes of a hangout share their packs: each read once - docs/PERFORMANCE.md 2026-10-23 (5))
            if !packs.contains_key(&f) {
                let Ok(res) = load_resource_file(&f) else { continue };
                packs.insert(f.clone(), res);
            }
            let res = &packs[&f];
            let rd = Rd::new(res);
            let Some((o, _)) = pack_entries(res).into_iter().find(|(_, n)| *n == entry) else { continue };
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
        log::info!("cine: {} people at {} ({:.0} ms)", self.ambient.len(), name, t0.elapsed().as_secs_f32() * 1000.0);
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
    /// scene's camera at t 1.0 before the scene plays - `F-62707`): (eye, forward, up, vertical
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
        if self.at_game_camera() {
            return None;
        }
        Self::scene_camera(self.playing.as_ref()?)
    }

    /// A story scene past its cut_to_game: the game camera shows the rest (`STORY_MARKS`).
    pub fn at_game_camera(&self) -> bool {
        self.playing.as_ref().map_or(false, |sc| cut_to_game(&sc.name, sc.camera_len()).map_or(false, |c| sc.t >= c))
    }

    fn scene_camera(sc: &Scene) -> Option<(Vec3, Vec3, Vec3, f32)> {
        let cam = sc.camera.as_ref()?;
        let f = cam.frame_at(sc.t);
        let pos = cam.track(KIND_TRANSLATION, 0)?.data.vec(f);
        let rot = cam.track(KIND_ROTATION, 0).map(|t| t.data.rot(f)).unwrap_or(Quat::IDENTITY);
        // (the lens track is a focal length in mm on a 35 mm gate: cam+104 = 2 atan(0.7085 in /
        // (v mm * 0.03937)), the horizontal angle (F-25555); the render camera's vertical
        // angle is that divided by the viewport's aspect, 16:9 (F-79297, the warp's
        // target, cam_to_menu F-14139). Without the track the camera keeps its reset
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
                    // a target without a track keeps its weight (F-77597 writes only what the frame has)
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
            // (F-65008: a cine character whose name holds "ped" is drawn with the manager's own
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_skip_lands_on_the_bookmark() {
        // (the recomp's skip measured on district champs: its clock jumped to 74.233 s - bookmark_cut_to_game
        // 74.333 at the clip's end, filed 0.1 s earlier)
        let to = skip_to("story/cut_story_district_champs", 0.036, 74.3333).unwrap();
        assert!((to - 74.2333).abs() < 1e-3);
        // RACE_GH_06: the bookmark 57.633, the cut to the game camera 64.167; a mark is filed 0.1 s earlier only when it is at
        // or past the camera clip's length (F-73781: t >= max(len, 0.1)), so a clip ending exactly there moves it ...
        assert_eq!(skip_to("story/cut_story_race_gh_06", 10.0, 64.1667), Some(57.6333));
        assert_eq!(skip_to("story/cut_story_race_gh_06", 58.0, 64.1667), None);
        assert!((cut_to_game("story/cut_story_race_gh_06", 64.1667).unwrap() - 64.0667).abs() < 1e-3);
        // ... and a longer clip leaves it where the script put it
        assert!((cut_to_game("story/cut_story_race_gh_06", 64.2).unwrap() - 64.1667).abs() < 1e-3);
        // no marks: the skip does nothing
        assert_eq!(skip_to("story/cut_story_motorcycle_rep_A", 1.0, 40.0), None);
    }
}
