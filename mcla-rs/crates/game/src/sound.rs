//! Maps what happens in the game onto the original sound banks.

use crate::ability::Abilities;
use crate::audio::Audio;
use crate::game::{Game, Mode};
use crate::police::{Heat, Police};
use crate::vehicles::Vehicles;
use glam::Vec3;

const AUX: &str = "vehicles/vehicle_aux";
const CAR_BANG: &str = "bangers/car_bangers";
const PROP_BANG: &str = "bangers/generic_prop_bangers";
const COLL: &str = "bangers/collisions";
const MENUS: &str = "ui/menus";
const FX: &str = "special_effects/bnk_special_effects";

/// Sound category of a prop (set from its name when the props load).
pub fn prop_sound(name: &str) -> u8 {
    let n = name.to_ascii_lowercase();
    if n.contains("gfx") || n.contains("bush") || n.contains("hedge") {
        9
    } else if n.contains("trash") || n.contains("dumpster") || n.contains("barrel") {
        2
    } else if n.contains("parking_meter") || n.contains("meter") {
        3
    } else if n.contains("news") || n.contains("vending") || n.contains("phone") || n.contains("utility_box") {
        4
    } else if n.contains("fence") || n.contains("barrier") {
        5
    } else if n.contains("hydrant") {
        6
    } else if n.contains("cone") || n.contains("plastic") {
        7
    } else if n.contains("light") || n.contains("signal") || n.contains("sign") || n.contains("pole") {
        8
    } else if n.contains("bollard") {
        10
    } else if n.contains("bus_") || n.contains("glass") || n.contains("booth") {
        11
    } else if n.contains("cardboard") || n.contains("box") {
        12
    } else if n.contains("bench") || n.contains("chair") || n.contains("table") || n.contains("palette") || n.contains("wood") || n.contains("crate") {
        1
    } else {
        0
    }
}

/// A prop was hit: (position, sound category, impact speed, knocked over?)
#[derive(Clone, Copy, Debug)]
pub struct PropHit {
    pub pos: Vec3,
    pub kind: u8,
    /// the prop type's name hash (its game.dat audio object)
    pub model: u32,
    pub speed: f32,
    pub knocked: bool,
}

#[derive(Default)]
pub struct Sound {
    /// each ambient traffic model's sound class (`szAudioType` of its mcAmbientType.xml):
    /// 0 sedan, 1 SUV, 2 pickup, 3 truck or bus
    pub amb_types: std::collections::HashMap<String, u8>,
    gear: i32,
    damage: f32,
    impact_cool: f32,
    nitro_on: bool,
    /// a count to vary the hydraulics' samples
    hyd_count: u32,
    throttle: f32,
    cp: usize,
    count_tick: i32,
    mode: Option<Mode>,
    menu_sel: usize,
    garage_row: usize,
    race_sel: usize,
    selected_car: usize,
    heat: Option<Heat>,
    radio: f32,
    /// the scripted police radio is running: no random chatter from here
    pub scripted_radio: bool,
    ability_on: bool,
    ability_ready: bool,
    rep: i64,
    money: i64,
    /// the HUD's overhead map was up
    map_big: bool,
    horn: bool,
    skid: f32,
    airborne: bool,
    backfire_cool: f32,
    traffic_horn: f32,
    started: bool,
    /// ambience bed name -> stream file (relative to the audio root)
    beds: Option<hashbrown::HashMap<String, String>>,
    /// the original's sound objects (sounds.dat ...) and whether loading them was tried
    pub lib: Option<crate::sndobj::SoundLib>,
    lib_tried: bool,
    /// the player car's engine sound, for the car it was made for
    engine: Option<(String, crate::engaudio::EngineSound)>,
    /// the player engine's start (`play_ignition`): 1 asked for, 2 cranking; and the crank's end, for `take_engine_running`
    ignition: u8,
    engine_running: bool,
    /// the player engine's stop asked for (`stop_engine`), applied at the next update
    engine_stop: bool,
    /// the player engine's sound set running at once (`engine_set_running`), applied at the next update
    engine_run: bool,
    /// the original's collision sounds (with `lib`)
    coll: Option<crate::collaudio::CollisionAudio>,
    /// the original's district ambience (with `lib`)
    amb: Option<crate::ambaudio::Ambience>,
    /// the AUDIO OPTIONS sliders (effects, music, voice, player vehicle, cutscene; 0..10) and the audio camera
    /// mode, set by the game each frame
    pub sliders: [u8; 5],
    pub cam_audio: i32,
    /// every car's sounds beside the engine and collisions (caraudio.rs, with `lib`)
    fleet: Option<crate::caraudio::Fleet>,
    /// the player car's performance stages the sounds read: intake, turbo, nitrous
    pub parts: [u8; 3],
    /// the countdown's last sounded step (3, 2, 1, 0 = GO; 4 = before it; -1 = no race)
    count_stage: i32,
    end_race: bool,
    /// the reward screen's rep bar sound waits 20 movie frames
    rep_fill: Option<f32>,
    ability_snd: Option<crate::sndobj::Handle>,
    nitro_btn: bool,
    /// a speech or police radio line is playing (set by the game each frame)
    pub speech_busy: bool,
    /// the career speech line said at the player's car (F-32206 puts it at the entity's position), followed while it plays
    speech_line: Option<crate::sndobj::Handle>,
    /// cruisers already reported disabled on the radio
    pub cop_disabled: std::collections::HashSet<usize>,
    /// set by the game each frame: the HUD movie's `damage_alpha`, a splash posted to the HUD's queue 1, the pause
    /// movie's `cur_camera`
    pub damage_alpha: f32,
    pub racer_found: bool,
    pub menu_cam: Option<i32>,
    /// a speech line the sounds ask for (PLAYER_IN_AIR); the game says it if no speech plays
    pub want_speech: Option<&'static str>,
    navmap: bool,
    /// set by the game: the start warp's step (startwarp.rs) - 0 none, 1 the climb, 2 held, 3 the way down
    pub warp: u8,
    /// the warp's step last frame, and whether its NAVMAP scene is the warp's (started by the climb, not yet stopped)
    warp_prev: u8,
    warp_navmap: bool,
    /// the audio camera mode last frame (a change applies the camera table, F-33257)
    cam_prev: i32,
    /// set by the game once the props are placed: every prop's audio object and place (`Props::audio_places`)
    pub prop_places: Vec<(u32, Vec3)>,
    /// the props with a distance loop (their game.dat object's +86 not NULL_SOUND): place, loop, max distance, and the
    /// loop while it plays
    prop_loops: Option<Vec<(Vec3, u32, f32, Option<crate::sndobj::Handle>)>>,
    phone: bool,
    tutorial: bool,
    dmg_prev: f32,
    dmg_armed: bool,
    pan_cam: Option<i32>,
    /// AIRTIME (F-22751): the wheel probed this frame, misses so far, on, time in the air
    air_i: usize,
    air_miss: usize,
    air_on: bool,
    air_t: f32,
    /// persistent sounds asked for by name (Audio::obj_loop)
    obj_loops: std::collections::HashMap<String, crate::sndobj::Handle>,
    /// loops asked for whose object the library does not have (said once)
    obj_loops_missing: std::collections::HashSet<String>,
    /// the story scene's audio (F-58551): its dialogue and effects sounds and its mixer scene
    cut: Vec<crate::sndobj::Handle>,
    /// set by the game: the player's car is the port's stand-in, hidden - a new career has no player vehicle until
    /// CarSelection gives one (AddVehicleToPlayer), so it has no engine sound
    pub no_player_car: bool,
    cut_scene: Option<String>,
    /// CopArrest.sco's mixer scenes (0x1010 / 0x1038, detriggered at 0x14f3 / 0x151b): POLICE_SCRIPT_TRIGGERED (not for
    /// the payback's goons) and MUTE_SHENANIGANS, from the arrest scene's start to the end of the script
    pub arrest_mix: (bool, bool),
}

/// The ambience beds' streams: bed name -> stream file (relative to the audio root).
pub fn load_beds(root: &std::path::Path) -> hashbrown::HashMap<String, String> {
    let mut m = hashbrown::HashMap::new();
    if let Ok(rd) = std::fs::read_dir(root.join("ambience_stream")) {
        for e in rd.flatten() {
            let p = e.path();
            let fname = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let Some(stem) = fname.strip_suffix(".mp3.txt") else { continue };
            let Ok(t) = std::fs::read_to_string(&p) else { continue };
            let Some(ch) = t.lines().next().and_then(|l| l.split(char::from(9u8)).nth(2)) else { continue };
            if let Some(bed) = ch.strip_prefix("AMB_BED_") {
                // drop the speaker suffix (_L, _R, _LS, _RS)
                let bed = bed.rsplit_once('_').map(|x| x.0).unwrap_or(bed);
                m.insert(bed.to_string(), format!("ambience_stream/{stem}.mp3"));
            }
        }
    }
    log::info!("audio: {} ambience beds", m.len());
    m
}

/// The sound library and what is made with it, read off the game loop (the start-up's loading thread,
/// boot.md "Loading behind the movie"; or at the first sound update as before).
pub struct Preloaded {
    pub lib: Option<crate::sndobj::SoundLib>,
    coll: Option<crate::collaudio::CollisionAudio>,
    amb: Option<crate::ambaudio::Ambience>,
    fleet: Option<crate::caraudio::Fleet>,
    beds: Option<hashbrown::HashMap<String, String>>,
}

impl Preloaded {
    /// `root` = the audio root, `cfg` = `audio/X360/config`; `all`: the ambience beds and the stream index too
    /// (otherwise read when first needed).
    pub fn load(root: &std::path::Path, cfg: &std::path::Path, all: bool) -> Preloaded {
        let mut lib = crate::sndobj::SoundLib::load(root, cfg);
        let (mut coll, mut amb, mut fleet) = (None, None, None);
        if let Some(l) = lib.as_mut() {
            // (cfg = <data>/audio/X360/config)
            let data = cfg.ancestors().nth(3).map(|p| p.to_path_buf()).unwrap_or_default();
            coll = Some(crate::collaudio::CollisionAudio::new(&data));
            amb = crate::ambaudio::Ambience::new(l);
            fleet = Some(crate::caraudio::Fleet::new(&data));
            if all {
                l.index_streams();
            }
        } else {
            log::warn!("audio: {} not loaded - the original's sound objects are not played", cfg.display());
        }
        Preloaded { lib, coll, amb, fleet, beds: all.then(|| load_beds(root)) }
    }
}

impl Sound {
    /// The player engine's start (F-44510 with ignition 0, from CopArrest's Racer_PlayIgnition F-35902 / F-91984):
    /// the engine sound stops and plays the ignition, cranks and flares (engaudio.rs `start`); from the request on the
    /// control step holds the car and lifts `engine_off` (`engine_held`, F-17997).
    pub fn play_ignition(&mut self) {
        self.ignition = 1;
    }

    /// The player engine's stop after a damage-out (the player's vt+56, F-17073, with 1): the engine sound's stop
    /// (engaudio.rs `stop_engine`: silent, IGNITION_KAPUT_MASTER after 2.25 s) and the DAMAGE_OUT speech request
    /// (F-30989, "PLAYER") unless a speech line plays (+2256 and +2260).
    pub fn stop_engine(&mut self) {
        self.engine_stop = true;
        if !self.speech_busy {
            self.want_speech = Some("DAMAGE_OUT");
        }
    }

    /// The player engine's sound set running at once (the racer's restore F-39432 after a QUICK FIX -> vt+48,
    /// engaudio.rs `set_running`): a start or stop asked for and not yet applied is dropped with it.
    pub fn engine_set_running(&mut self) {
        self.engine_run = true;
    }

    /// Whether the player's engine sound is stopped or waiting to stop (its states 2 and 3, or a stop asked for).
    pub fn engine_stopped(&self) -> bool {
        self.engine_stop || self.engine.as_ref().is_some_and(|(_, e)| e.stopped())
    }

    /// Whether the player's engine sound holds the car (its state below 4, or a start / stop asked for and not yet
    /// applied): the control step F-17997 then sets the engine word +46 to 1 if it is 0, puts the gearbox in first and
    /// sets throttle 0, brake 1, handbrake 0.
    pub fn engine_held(&self) -> bool {
        self.ignition == 1 || self.engine_stop || self.engine.as_ref().is_some_and(|(_, e)| e.held())
    }

    /// True once when the started engine's crank has ended: the engine runs again (F-10725 sets chassis engine
    /// +46 = 1 there) - the caller lifts the car's `engine_off`.
    pub fn take_engine_running(&mut self) -> bool {
        std::mem::take(&mut self.engine_running)
    }

    /// The call signs of the lead cruiser and of a second one, for the police radio (`Fleet::radio_units`).
    pub fn radio_units(&mut self) -> (Option<String>, Option<String>) {
        self.fleet.as_mut().map(|f| f.radio_units()).unwrap_or_default()
    }

    /// Take a [`Preloaded`] sound library (the first sound update does it itself when nothing was handed over).
    pub fn adopt(&mut self, p: Preloaded) {
        self.lib_tried = true;
        self.lib = p.lib;
        self.coll = p.coll;
        self.amb = p.amb;
        self.fleet = p.fleet;
        if p.beds.is_some() {
            self.beds = p.beds;
        }
    }

    /// A persistent object sound started outside the game loop (the start-up movie's loading music), kept as
    /// [`crate::audio::Audio::obj_loop`] keeps its own.
    pub fn keep_loop(&mut self, name: &str, h: crate::sndobj::Handle) {
        self.obj_loops.insert(name.to_string(), h);
    }
}

impl Sound {
    #[allow(clippy::too_many_arguments)]
    /// `rain`: the weather's rain share (wet = not 0); `probe(from, to)`: the city's collision has something on that line.
    /// The tach's rpm: the HUD's d_RPMvalue is the player's engine sound's slewed rpm (engine sound +204,
    /// F-86111 from F-10725's follower: at most its slew a second - 250 rpm a frame for most cars),
    /// not the engine's own. None while the car has no engine sound of the original's.
    pub fn tach_rpm(&self) -> Option<f32> {
        self.engine.as_ref().map(|(_, e)| e.smoothed_rpm())
    }

    pub fn update(&mut self, a: &mut Audio, vehicles: &Vehicles, game: &Game, police: &Police, ability: &Abilities, prop_hits: &[PropHit], cam_pos: Vec3, cam_right: Vec3, cam_fwd: Vec3, rain: f32, horn: bool, night: bool, probe: &dyn Fn(Vec3, Vec3) -> bool, dt: f32) {
        let wet = rain > 0.0;
        a.set_listener(cam_pos, cam_right);
        if !self.lib_tried {
            self.adopt(Preloaded::load(a.root(), &a.cfg, false));
        }
        a.objects = self.lib.is_some();
        if let Some(lib) = &mut self.lib {
            // what the rest of the game asked for this frame (Audio::obj ...)
            let mut loops_now: Vec<String> = Vec::new();
            for c in std::mem::take(&mut a.obj_cmds) {
                match c {
                    crate::audio::ObjCmd::Loop { name } => {
                        let alive = self.obj_loops.get(&name).is_some_and(|h| lib.playing(*h));
                        if !alive {
                            if let Some(h) = lib.play(&name, crate::sndobj::Req::default()) {
                                self.obj_loops.insert(name.clone(), h);
                            } else if self.obj_loops_missing.insert(name.clone()) {
                                log::warn!("audio: no sound object {name} for a loop");
                            }
                        }
                        loops_now.push(name);
                    }
                    crate::audio::ObjCmd::Play { name, pos, volume } => {
                        if lib.play(&name, crate::sndobj::Req { pos, volume, ..Default::default() }).is_none() {
                            log::warn!("audio: no sound object {name}");
                        }
                    }
                    crate::audio::ObjCmd::Line { key, object, player } => {
                        // a speech line's request (F-70477): the defaults of F-35212 - audio timer 0, so it holds in a pause
                        let pos = if player { vehicles.player().map(|c| c.state.pos) } else { None };
                        if let Some(h) = lib.play_clip(&key, &object, crate::sndobj::Req { pos, ..Default::default() }) {
                            lib.set_timer(h, 0);
                            if pos.is_some() {
                                self.speech_line = Some(h);
                            }
                        }
                    }
                    crate::audio::ObjCmd::Clip { key, category, pos } => {
                        lib.play_clip(&key, category, crate::sndobj::Req { pos, ..Default::default() });
                    }
                    crate::audio::ObjCmd::Scene { name, on } => {
                        // (a speech line's scene, F-77322: strength = dB2lin(the SPEECH category's runtime volume))
                        let s = if name.starts_with("DYNAMIC_MIXER_SPEECH") || name == "DYNAMIC_MIXER_PRERACE_TAUNT" { lib.category("SPEECH").map_or(1.0, |c| mcla_formats::audcfg::db2lin(c.volume)) } else { 1.0 };
                        lib.scene_at(&name, on, s)
                    }
                    crate::audio::ObjCmd::Cutscene { name, on, sounds } => {
                        // the scene's audio ends (F-27469): its sounds stop, its mixer scene stops with its exit
                        for h in self.cut.drain(..) {
                            lib.stop(h);
                        }
                        if let Some(sc) = self.cut_scene.take() {
                            lib.scene(&sc, false);
                        }
                        // starts: the dialogue and effects non-positional at 0 dB, the mixer scene at strength 1.0
                        // (0x82001D14) - `SoundLib::cutscene_parts`
                        if let (true, Some((parts, scene))) = (on, lib.cutscene_parts(&name, wet)) {
                            for h in parts.into_iter().filter(|_| sounds) {
                                self.cut.extend(lib.play_hash(h, crate::sndobj::Req::default()));
                            }
                            if let Some(sc) = scene {
                                lib.scene_at(&sc, true, 1.0);
                                self.cut_scene = Some(sc);
                            }
                            log::info!("cutscene audio: CUTSCENES_{name} ({} sounds, scene {:?})", self.cut.len(), self.cut_scene);
                        }
                    }
                    crate::audio::ObjCmd::CutsceneSeek { secs } => {
                        for &h in &self.cut {
                            lib.seek(h, secs * 1000.0);
                        }
                    }
                    crate::audio::ObjCmd::Field { object, offset, pos, on } => {
                        let h = lib.game.get_name(&object).and_then(|o| o.get(offset..offset + 4)).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
                        if let Some(h) = h {
                            match lib.game.name_of(h).map(|n| n.to_string()) {
                                Some(scene) if lib.game.get(h).and_then(|o| o.first()) == Some(&23) => lib.scene(&scene, on),
                                _ if on => {
                                    lib.play_hash(h, crate::sndobj::Req { pos, ..Default::default() });
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            self.obj_loops.retain(|n, h| {
                let keep = loops_now.contains(n);
                if !keep {
                    lib.stop(*h);
                }
                keep
            });
            // the listener is the render camera's matrix (F-37562 keeps it, F-99037 hands it over every frame), pitch included
            lib.listener = (cam_pos, cam_right, cam_fwd);
            lib.set_sliders(self.sliders);
            let bike = vehicles.player().is_some_and(|p| p.model.bike);
            lib.set_camera_mode(self.cam_audio, bike);
            // the dynamic mixer scenes (MIXER_SCENES_RULE.md 2)
            // NAVMAP: the GPS camera rises into the map (GPS_UP_HIGH, GLOBAL_UI_NAVMAP +66) / comes down (+70)
            if game.show_map != self.navmap {
                let f = lib.game.get_name("GLOBAL_UI_NAVMAP").and_then(|o| o.get(if game.show_map { 66..70 } else { 70..74 })).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
                if let Some(h) = f {
                    lib.play_hash(h, crate::sndobj::Req::default());
                }
                lib.scene("DYNAMIC_MIXER_NAVMAP", game.show_map);
                self.navmap = game.show_map;
            }
            // the start warp's navmap audio (mcWarper -> F-52643(event)): Warper_Warp('MAXHEIGHT') (F-32081 case
            // 9, A-24009) sends event 0 - GLOBAL_UI_NAVMAP +66 (GPS_UP_HIGH) and, unless it already runs, NAVMAP (stream
            // GPS_SKY_AMBIENCE; beds, SFX_HARD - every car, the people, the tyres - and PLAYER_VEHICLES to -100 dB, fade in
            // 1 s); the way down (USER_WAIT_NO_CUT_DOWN: F-31347 after the GPS chooser F-18267) sends event 1 from
            // the held 9000 m (> 6000, 0x82018A20) - +70 (GPS_DOWN_LOW) and NAVMAP stopped with its exit (fade out 2 s).
            // A new career has no way down: the scene stays until the camera table is next applied for another mode than 5
            // (F-33257 from F-79297 stops NAVMAP without its exit)
            if self.warp != self.warp_prev {
                let field = |lib: &crate::sndobj::SoundLib, at: usize| lib.game.get_name("GLOBAL_UI_NAVMAP").and_then(|o| o.get(at..at + 4)).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
                if self.warp == 1 && self.warp_prev == 0 {
                    if let Some(h) = field(lib, 66) {
                        lib.play_hash(h, crate::sndobj::Req::default());
                    }
                    if !lib.scene_on("DYNAMIC_MIXER_NAVMAP") {
                        lib.scene("DYNAMIC_MIXER_NAVMAP", true);
                        self.warp_navmap = true;
                    }
                } else if self.warp == 3 {
                    if let Some(h) = field(lib, 70) {
                        lib.play_hash(h, crate::sndobj::Req::default());
                    }
                    lib.scene("DYNAMIC_MIXER_NAVMAP", false);
                    self.warp_navmap = false;
                }
                self.warp_prev = self.warp;
            }
            if self.cam_audio != self.cam_prev {
                if self.warp == 0 && self.warp_navmap && self.cam_audio != 5 {
                    lib.scene_cut("DYNAMIC_MIXER_NAVMAP");
                    self.warp_navmap = false;
                }
                self.cam_prev = self.cam_audio;
            }
            // the props' distance loops (F-39846 -> F-32775, COLLISION_AUDIO_RULE.md 3.5): a prop whose type
            // object (LA_PROP_<type>, type 43) has a loop at +86 - the parked and flying aircraft (CESSNA, CESSNA_BEACH,
            // the 737 JET, HELICOPTER) and road flares - plays it at its place while the listener is within +90 m
            // (700 / 800 / 780 / 50), and stops beyond. (The aircraft's flag +85 bit 0 keeps it while the prop moves; the
            // port's stand. The request's +88 LPF / +90 curves are not applied by the port's mixer.)
            if self.prop_loops.is_none() && !self.prop_places.is_empty() {
                let null = mcla_formats::hash::joaat("NULL_SOUND");
                let mut v = Vec::new();
                for &(ty, pos) in &self.prop_places {
                    let Some(o) = lib.game.get(ty).filter(|o| o.first() == Some(&43) && o.len() >= 94) else { continue };
                    let h = u32::from_be_bytes(o[86..90].try_into().unwrap());
                    if h != 0 && h != null {
                        v.push((pos, h, f32::from_bits(u32::from_be_bytes(o[90..94].try_into().unwrap())), None));
                    }
                }
                log::info!("audio: {} props with a distance loop", v.len());
                self.prop_loops = Some(v);
            }
            for (pos, h, max, playing) in self.prop_loops.iter_mut().flatten() {
                let near = pos.distance(cam_pos) <= *max;
                match (near, *playing) {
                    (true, Some(p)) if lib.playing(p) => {}
                    (true, _) => *playing = lib.play_hash(*h, crate::sndobj::Req { pos: Some(*pos), ..Default::default() }),
                    (false, Some(p)) => {
                        lib.stop(p);
                        *playing = None;
                    }
                    (false, None) => {}
                }
            }
            // BINK_DEFAULT: a film (BINK_INTRO / BINK_ATTRACT +10) while it plays - the Bink player's F-45360 starts
            // it at strength 1.0, its cleanup F-66695 stops it (MUSIC, SPEECH, CUTSCENES, PLAYER_VEHICLES, SFX -100 dB)
            lib.scene("DYNAMIC_MIXER_BINK_DEFAULT", game.movie.is_some());
            // PAUSE (F-99037): paused, not the map, not the replay viewer playing outside the race end
            let endrace = lib.scene_on("DYNAMIC_MIXER_ENDRACE") || lib.scene_on("DYNAMIC_MIXER_ENDRACE_BEATMETHERE");
            let viewer = game.replay.as_ref().is_some_and(|r| !r.paused);
            lib.scene("DYNAMIC_MIXER_PAUSE", game.mode == Mode::Paused && !lib.scene_on("DYNAMIC_MIXER_NAVMAP") && !(viewer && !endrace));
            // the audio timer 0 (F-99037: engine+580): paused while the game is, unless the replay viewer is open outside the
            // race end; (approx.) only the speech lines run on it in the port, the original's other game sounds do too
            lib.timer_paused[0] = game.mode == Mode::Paused && !(game.replay.is_some() && !endrace);
            // the line at the player's car follows it (F-32206 each update: the entity's position)
            if let Some(h) = self.speech_line {
                match vehicles.player().map(|c| c.state.pos) {
                    Some(p) if lib.playing(h) => lib.set(h, crate::sndobj::Req { pos: Some(p), ..Default::default() }),
                    _ => self.speech_line = None,
                }
            }
            lib.scene("DYNAMIC_MIXER_GARAGE", game.mode == Mode::Garage);
            // REPLAY_*: the replay viewer's speed (F-42247; the port's viewer plays at 1 or is paused)
            match &game.replay {
                Some(r) if r.paused => {
                    lib.scene("DYNAMIC_MIXER_REPLAY_FRAME_ADVANCE", false);
                    lib.scene("DYNAMIC_MIXER_REPLAY_PAUSED", true);
                    lib.scene_at("DYNAMIC_MIXER_REPLAY_SCALED", true, 1.0);
                }
                _ => {
                    for n in ["DYNAMIC_MIXER_REPLAY_PAUSED", "DYNAMIC_MIXER_REPLAY_FRAME_ADVANCE", "DYNAMIC_MIXER_REPLAY_SCALED"] {
                        lib.scene(n, false);
                    }
                }
            }
            // MUSIC (F-42730): while a song plays, strength = dB2lin(MUSIC + MUSIC_USER), continuous; off when the
            // song stops or MUSIC_USER is at -95 dB or below
            let vol = |n: &str| lib.category(n).map_or(0.0, |c| c.volume);
            let (mv, mu) = (vol("MUSIC"), vol("MUSIC_USER"));
            let song = a.music_on && !a.now_playing.is_empty() && mu > -95.0;
            lib.scene_at("DYNAMIC_MIXER_MUSIC", song, mcla_formats::audcfg::db2lin(mv + mu));
            // event sounds (MIXER_SCENES_RULE.md 3): the phone rings when it comes up and closes when it goes
            let phone = game.flash_phone.as_ref().is_some_and(|p| p.visible());
            if phone != self.phone {
                lib.play(if phone { "UI_CELLPHONE_RING" } else { "UI_CELLPHONE_CLOSE" }, crate::sndobj::Req::default());
                self.phone = phone;
            }
            // a tutorial box opens / closes: its form's OnShow / OnHide (mcForm slots 400 / 756) play
            // UI_MENU_TUTORIAL_OPEN_MASTER / _CLOSE_MASTER - all but the forms whose slots play none: 79 slipstream_use
            // (F-61402's class), 82 challenge / 83 flash (one OnShow, no OnHide), 85 gas_station, 89 telephone_challenge
            // (work/slipstream/tut/forms2.py; forms 94 .. 105 not read)
            const SILENT: [&str; 5] = ["tutorial_slipstream_use_title", "tutorial_challenge_title", "tutorial_flash_title", "tutorial_gas_station_title", "tutorial_telephone_challenge_title"];
            let tut = game.tutorial_box.as_ref().is_some_and(|t| t.2 <= 0.0 && !SILENT.contains(&t.0.as_str()));
            if tut != self.tutorial {
                lib.play(if tut { "UI_MENU_TUTORIAL_OPEN_MASTER" } else { "UI_MENU_TUTORIAL_CLOSE_MASTER" }, crate::sndobj::Req::default());
                self.tutorial = tut;
            }
            // DAMAGE_METER_BEEP (F-89429): one beep per rise of the HUD's damage_alpha, re-armed when it falls
            let v = if matches!(game.mode, Mode::FreeRoam | Mode::Race) && game.flash_hud.is_some() { self.damage_alpha } else { 0.0 };
            if v < self.dmg_prev {
                self.dmg_armed = true;
            } else if self.dmg_armed && v != 0.0 {
                lib.play("DAMAGE_METER_BEEP", crate::sndobj::Req::default());
                self.dmg_armed = false;
            }
            self.dmg_prev = v;
            // RACER_FOUND_MASTER: the HUD movie's splash queue 1 begins
            if std::mem::take(&mut self.racer_found) {
                lib.play("RACER_FOUND_MASTER", crate::sndobj::Req::default());
            }
            // END_OF_RACE_CAMERA_PAN (F-63176): the menu camera changes while the end-race scene is on
            if self.menu_cam != self.pan_cam {
                if self.pan_cam.is_some() && self.menu_cam.is_some() && endrace {
                    lib.play("END_OF_RACE_CAMERA_PAN", crate::sndobj::Req::default());
                }
                self.pan_cam = self.menu_cam;
            }
            // the songs: a MUSIC_USER stream at 0 dB (3.1), less their headroom (Audio::stream_gain)
            a.music_volume = lib.category("MUSIC_USER").map_or(0.0, |c| mcla_formats::audcfg::db2lin(c.volume));
        }
        // ---- collisions (collaudio.rs): every car's contacts of the frame, the props' hits
        if let (Some(lib), Some(coll)) = (&mut self.lib, &mut self.coll) {
            coll.begin_frame(dt);
            for (i, c) in vehicles.cars.iter().enumerate() {
                if c.state.touches.is_empty() && !c.is_player {
                    continue;
                }
                let owner = if c.is_player {
                    crate::collaudio::Owner::Player
                } else if c.racer {
                    crate::collaudio::Owner::Opponent
                } else {
                    crate::collaudio::Owner::Ambient
                };
                coll.car(lib, i, &c.model.name, owner, c.model.bike, &c.state, c.is_player);
            }
            coll.retain(lib, |i| i < vehicles.cars.len());
            for h in prop_hits {
                coll.prop(lib, h.model, h.pos, h.speed, h.knocked);
            }
        }
        let in_menu = game.in_menu();
        let driving = matches!(game.mode, Mode::FreeRoam | Mode::Race);
        self.impact_cool = (self.impact_cool - dt).max(0.0);
        self.backfire_cool = (self.backfire_cool - dt).max(0.0);

        // ---- front end ------------------------------------------------------------
        if self.started {
            if self.mode != Some(game.mode) {
                let back = matches!((self.mode, game.mode), (Some(Mode::Garage), Mode::MainMenu) | (Some(Mode::RaceSelect), Mode::MainMenu) | (Some(Mode::Paused), Mode::FreeRoam) | (Some(Mode::Paused), Mode::Race) | (Some(Mode::Garage), Mode::Paused));
                self.ui(a, if back { "UI_MENU_BACK" } else { "UI_MENU_SELECT" }, &format!("{MENUS}/{}", if back { "ui_menu_back_l" } else { "ui_menu_select_l" }), 0.6);
                // (with the sound objects the sting is the ENDRACE mixer's own sound, below)
                if game.mode == Mode::Results && self.lib.is_none() {
                    let won = game.last_results.first().map_or(false, |r| r.0.starts_with("YOU"));
                    a.play(&format!("{FX}/{}", if won { "end_of_race_01.l" } else { "end_of_race_05.l" }), 0.8);
                }
            } else if in_menu && (self.menu_sel != game.menu_sel || self.garage_row != game.garage_row || self.race_sel != game.race_sel || self.selected_car != game.selected_car) {
                // (MenuInput.sc.xml: up -> NAV_UP, down -> NAV_DOWN; MenuHorizontalInput: right -> NAV_DOWN, left -> NAV_UP)
                let down = game.menu_sel > self.menu_sel || game.garage_row > self.garage_row || game.race_sel > self.race_sel || game.selected_car > self.selected_car;
                self.ui(a, if down { "UI_MENU_NAV_DOWN" } else { "UI_MENU_NAV_UP" }, &format!("{MENUS}/ui_menu_nav_l"), 0.5);
            }
            // the overhead map up / down (hud.sc.xml: the transitions' playSound UI_MENU_SELECT /
            // UI_MENU_BACK - D-pad up and the scripts' mapGoBig / mapGoNormal alike)
            if game.hud_map_big != self.map_big {
                self.ui(a, if game.hud_map_big { "UI_MENU_SELECT" } else { "UI_MENU_BACK" }, &format!("{MENUS}/{}", if game.hud_map_big { "ui_menu_select_l" } else { "ui_menu_back_l" }), 0.6);
            }
            // the reward: a new rank RANK_UP_MASTER, else the bar filling, 20 movie frames in (AnimateReward)
            if game.rep > self.rep && game.rep / 2000 != self.rep / 2000 {
                self.ui(a, "RANK_UP_MASTER", &format!("{MENUS}/rank_up"), 0.7);
            } else if game.rep > self.rep && self.lib.is_some() {
                self.rep_fill = Some(20.0 / 30.0);
            }
            if self.lib.is_some() {
                // GARAGE_MONEY_MASTER at a purchase or sale in the garage
                if game.money != self.money && game.mode == Mode::Garage {
                    self.ui(a, "GARAGE_MONEY_MASTER", "", 0.0);
                }
            } else if game.money < self.money && in_menu {
                a.play(&format!("{MENUS}/ui_menu_select_l"), 0.7);
            }
        }
        if let Some(t) = &mut self.rep_fill {
            *t -= dt;
            if *t <= 0.0 {
                self.rep_fill = None;
                self.ui(a, "REP_BAR_FILL_MASTER", "", 0.0);
            }
        }
        self.started = true;
        self.mode = Some(game.mode);
        self.menu_sel = game.menu_sel;
        self.garage_row = game.garage_row;
        self.race_sel = game.race_sel;
        self.selected_car = game.selected_car;
        self.rep = game.rep;
        self.money = game.money;
        self.map_big = game.hud_map_big;

        // ---- race flow ------------------------------------------------------------
        if let Some(r) = game.race.as_ref() {
            if self.lib.is_some() {
                // countdown_321Go_3sec: 3 at the start, 2 at -2050 ms, 1 at -1050, GO at -50
                let cd = r.countdown;
                let stage = if cd > 3.0 {
                    4
                } else if cd > 2.05 {
                    3
                } else if cd > 1.05 {
                    2
                } else if cd > 0.05 {
                    1
                } else {
                    0
                };
                if stage < 4 && stage != self.count_stage && game.mode == Mode::Race && (stage < self.count_stage || stage == 3) {
                    let n = if stage == 0 { "HUD_CNTDWN_GO_MASTER".to_string() } else { format!("HUD_CNTDWN_{stage}_MASTER") };
                    self.ui(a, &n, "", 0.0);
                }
                self.count_stage = stage;
            } else {
                let tick = if r.countdown > 0.0 { r.countdown.ceil() as i32 } else { 0 };
                if tick != self.count_tick && game.mode == Mode::Race {
                    if tick == 0 {
                        a.play(&format!("{MENUS}/321go_big_go_04_fin.l"), 0.8);
                    } else if tick <= 3 {
                        a.play(&format!("{MENUS}/321go_big_04_fin.l"), 0.7);
                    }
                }
                self.count_tick = tick;
            }
            let pi = vehicles.cars.iter().position(|c| c.is_player).unwrap_or(0);
            let cp = r.next.get(pi).copied().unwrap_or(0);
            // (the player's checkpoint, not the finish: HUD_CHECKPOINT_MASTER - OrderedCheckpointCore
            // 0x2c9a / 0x2d6a, the unordered core 0x1f4a; sounds.dat: CHECKPOINT_BIGWAY_MULTI =
            // UI_MENUS_CHECKPOINT_BIG_WAY_2 twice - their own fields differ, not decoded - and
            // UI_MENUS_CHECK_POIMT_SWTNR)
            if cp > self.cp && cp < r.checkpoints.len() && game.mode == Mode::Race && game.flag.is_none() && r.kind != crate::game::RaceKind::Payback {
                if self.lib.is_some() {
                    self.ui(a, "HUD_CHECKPOINT_MASTER", "", 0.0);
                } else {
                    a.play(&format!("{MENUS}/checkpoint_big_way_2"), 0.6);
                    a.play(&format!("{MENUS}/check_poimt_swtnr"), 0.4);
                }
            }
            self.cp = cp;
        } else {
            self.cp = 0;
            self.count_tick = 0;
            self.count_stage = -1;
        }
        // the race's end: the player's voice (race core 0x422e62e2: 1st win, 2nd within 0.3 s close, else lose), then
        // the ENDRACE mixer, whose own sound is the sting (AudioManager_SetActiveEndRace)
        let ended = game.race_end_t.is_some();
        if let Some(lib) = &mut self.lib {
            if ended && !self.end_race {
                let res = &game.last_results;
                let place = res.iter().position(|r| r.0.starts_with("YOU"));
                let close = place == Some(1) && matches!((res.first().and_then(|r| r.1), res.get(1).and_then(|r| r.1)), (Some(a), Some(b)) if b - a <= 0.3);
                let g = match place {
                    Some(0) => "WIN",
                    _ if close => "CLOSE",
                    _ => "LOSE",
                };
                lib.play(&format!("PLAYER_ENDRACE_GESTURE_{g}_MASTER"), crate::sndobj::Req::default());
            }
            lib.scene("DYNAMIC_MIXER_ENDRACE", game.race_end_t.is_some_and(|t| t >= 0.2));
        }
        self.end_race = ended;

        // ---- the player's car -----------------------------------------------------
        let Some(p) = vehicles.player() else {
            if let (Some(lib), Some((_, e))) = (&mut self.lib, &mut self.engine) {
                e.stop(lib);
            }
            self.engine = None;
            if let (Some(lib), Some(amb)) = (&mut self.lib, &mut self.amb) {
                amb.stop(lib);
            }
            if let (Some(lib), Some(f)) = (&mut self.lib, &mut self.fleet) {
                f.stop_all(lib);
            }
            self.finish(a, dt);
            return;
        };
        let st = &p.state;
        let speed = st.speed();
        let grounded = st.wheels.iter().any(|w| w.grounded);
        // AIRTIME (F-22751): in the air and a 10 m probe down from each wheel in turn (one a frame) hits nothing;
        // after 0.5 s in the air PLAYER_IN_AIR is asked for once
        if let Some(lib) = &mut self.lib {
            let order: &[usize] = if p.model.bike { &[0, 2] } else { &[0, 2, 1, 3] };
            if st.airborne_time > 0.0 && !in_menu {
                let w = st.wheels[order[self.air_i % order.len()]].world_pos;
                if probe(w, w - Vec3::new(0.0, 10.0, 0.0)) {
                    self.air_miss = 0;
                    if self.air_on {
                        lib.scene("DYNAMIC_MIXER_AIRTIME", false);
                        self.air_on = false;
                    }
                } else {
                    self.air_miss += 1;
                }
                self.air_i = (self.air_i + 1) % order.len();
                if self.air_miss >= order.len() && !self.air_on {
                    lib.scene("DYNAMIC_MIXER_AIRTIME", true);
                    self.air_on = true;
                    self.air_t = 0.0;
                }
                if self.air_on && self.air_t >= 0.0 {
                    self.air_t += dt;
                    if self.air_t > 0.5 && !self.speech_busy {
                        self.want_speech = Some("PLAYER_IN_AIR");
                        self.air_t = -1.0;
                    }
                }
            } else {
                self.air_miss = 0;
                if self.air_on {
                    lib.scene("DYNAMIC_MIXER_AIRTIME", false);
                    self.air_on = false;
                }
            }
        }
        let name = p.model.name.clone();
        let throttle = if in_menu { 0.0 } else { p.input.throttle };
        let eng_vol = if in_menu { 0.28 } else { 0.62 };
        // the original's engine sound (engaudio.rs); the port's own cross-fade where the car's audio objects are missing;
        // none while the engine is off (a scene's car_off, F-91263) unless it is being started (`play_ignition`)
        if self.no_player_car || (p.engine_off && self.ignition == 0) {
            if let (Some(lib), Some((_, e))) = (&mut self.lib, &mut self.engine) {
                e.stop(lib);
            }
            self.engine = None;
        } else if let Some(lib) = &mut self.lib {
            if self.engine.as_ref().map_or(true, |(n, _)| *n != name) {
                if let Some((_, e)) = &mut self.engine {
                    e.stop(lib);
                }
                self.engine = crate::engaudio::EngineSound::new(lib, &name, p.model.bike).map(|e| (name.clone(), e));
            }
            if self.ignition == 1 {
                if let Some((_, e)) = &mut self.engine {
                    e.start(lib, false);
                    self.ignition = 2;
                    log::info!("audio: engine start (ignition 0): loops stopped, the ignition next");
                }
            }
            if std::mem::take(&mut self.engine_stop) {
                if let Some((_, e)) = &mut self.engine {
                    e.stop_engine(lib, true);
                    log::info!("audio: engine stop (1): loops stopped, the kaput after the wait");
                }
            }
            if self.engine_run {
                if let Some((_, e)) = &mut self.engine {
                    e.set_running();
                    log::info!("audio: engine set running (restore)");
                }
                self.ignition = 0;
            }
        }
        self.engine_stop = false;
        self.engine_run = false;
        if self.ignition != 0 && self.engine.is_none() {
            // (no engine objects for this car: nothing to crank, it runs at once)
            self.ignition = 0;
            self.engine_running = true;
        }
        let orig_engine = self.engine.is_some();
        if let (Some(lib), Some((_, eng))) = (&mut self.lib, &mut self.engine) {
            let vs = st.veh.as_deref();
            let e = p.model.params.veh.as_ref().map(|t| &t.t.engine.s);
            let inp = crate::engaudio::EngineIn {
                pos: st.pos,
                view: Some(self.cam_audio),
                rpm: vs.map_or(st.rpm, |v| v.engine_w * 9.549_296),
                idle_rpm: e.map_or(800.0, |e| e.idle_rpm),
                max_rpm: e.map_or(7000.0, |e| e.max_rpm),
                torque: vs.map_or(0.0, |v| v.engine_torque),
                full_torque: p.model.params.veh.as_ref().map_or(1.0, |t| t.sync_torque),
                pedal: vs.map_or(throttle, |v| v.engine_pedal),
                throttle_in: throttle,
                speed,
                gear: vs.map_or(st.gear, |v| v.gear),
                shift: vs.map_or(0, |v| if v.gear_change.active { v.gear_change.direction } else { 0 }),
                idle_only: in_menu,
                racing: false,
                speed_ratio: 0.0,
            };
            eng.update(lib, &inp, dt);
            if eng.take_running() {
                self.ignition = 0;
                self.engine_running = true;
                log::info!("audio: engine crank over - running again, flare");
            }
        } else if !self.no_player_car && !p.engine_off {
            // engine sits a little towards the centre of the mix whatever the camera does
            a.engine(0, &name, st.rpm, throttle, (eng_vol, eng_vol));
        }
        // police events for the cars' sounds: a pull-over chirps and speaks, the chase runs the sirens, the radio
        // timer asks for the worldized chatter (the original's AI triggers: VEHICLE_EXTRA_AUDIO_RULE.md section 5)
        let fleet_on = self.fleet.is_some();
        if fleet_on {
            if self.heat != Some(police.heat) && !police.goons && police.heat == Heat::Busted {
                self.ui(a, "COP_ARREST_FORM_OPEN_MASTER", "", 0.0);
            }
            self.heat = Some(police.heat);
        }
        if let (Some(lib), Some(fleet)) = (&mut self.lib, &mut self.fleet) {
            // COP_FORM (F-37653): the ticket form with its prepared audio stream (CopPullover); the arrest form has none
            lib.scene("DYNAMIC_MIXER_COP_FORM", game.ticket_for.is_some() && game.arrest_panel.is_some_and(|p| p.0 <= 0.0));
            lib.scene("DYNAMIC_MIXER_POLICE_SCRIPT_TRIGGERED", self.arrest_mix.0);
            lib.scene("DYNAMIC_MIXER_MUTE_SHENANIGANS", self.arrest_mix.1);
            let press = p.input.nitro && !self.nitro_btn;
            let ctl = crate::caraudio::PlayerCtl {
                horn: horn && !in_menu,
                nitro_empty: press && !st.nitro_on() && st.nitro_shots == 0,
                parts: self.parts,
                cam_mode: self.cam_audio,
                loop_place: self.engine.as_ref().map(|(_, e)| (e.place(lib.listener, Some(self.cam_audio), st.pos), e.doppler())),
                engine: self.engine.as_ref().map(|(_, e)| (e.smoothed_rpm(), e.load_volume())),
            };
            let pol = crate::caraudio::PoliceIn {
                heat: match police.heat {
                    Heat::None => 0,
                    Heat::PullOver => 1,
                    Heat::Pursuit => 2,
                    Heat::Search => 3,
                    Heat::Busted => 4,
                },
                goons: police.goons,
                score: police.score,
                speech_busy: self.speech_busy,
            };
            fleet.update(lib, vehicles, cam_pos, ctl, wet, game.mode == Mode::Race, pol, dt);
            // (the slipstream burn's PLAYER_NITROUS)
            if let Some(l) = fleet.want_speech.take() {
                self.want_speech = Some(l);
            }
        }
        self.nitro_btn = p.input.nitro;

        if driving {
            // gear changes
            if !orig_engine && st.gear != self.gear && self.gear > 0 && st.gear > 0 {
                let k = if st.rpm > 5000.0 { "gearshift_hi_ck_1" } else { "gearshift_ck_1" };
                a.play(&format!("{AUX}/{k}"), 0.5);
                if st.gear > self.gear && throttle > 0.7 && p.model.params.accel.first().copied().unwrap_or(0.0) > 4.5 {
                    // forced-induction cars vent on the shift
                    if let Some(k) = a.pick(AUX, "blow_off_valve_new") {
                        a.play(&k, 0.35);
                    }
                }
            }
            // lifting off at high revs: the exhaust pops
            if !orig_engine && self.throttle > 0.8 && throttle < 0.2 && st.rpm > p.model.params.redline * 0.75 && self.backfire_cool <= 0.0 {
                if let Some(k) = a.pick(AUX, "backfire_0") {
                    a.play(&k, 0.45);
                    self.backfire_cool = 2.5;
                }
            }
            // hydraulics: the pump as the car is switched up or down, each ram as it goes
            // (the events are cleared by the caller once heard)
            if let Some(h) = st.veh.as_ref().map(|v| v.hyd).filter(|_| !fleet_on) {
                if h.ev_switch > 0 {
                    a.play(&format!("{AUX}/hydro_transition_up"), 0.6);
                } else if h.ev_switch < 0 {
                    a.play(&format!("{AUX}/hydro_transition_down"), 0.6);
                }
                if h.ev_rise != 0 {
                    a.play(&format!("{AUX}/hydraulic_rise_sweetener_0{}", 1 + self.hyd_count % 2), 0.6);
                    self.hyd_count += 1;
                }
                if h.ev_drop != 0 {
                    a.play(&format!("{AUX}/hydraulic_drop_sweetener_02"), 0.55);
                }
                if h.ev_end != 0 && h.on {
                    a.play(&format!("{AUX}/hydraulic_end_sweetener_0{}", 1 + self.hyd_count % 3), 0.35);
                }
            }
            // nitrous
            let nitro = st.nitro_on();
            if nitro && !self.nitro_on && !fleet_on {
                a.play(&format!("{AUX}/nitrous_01_fin_l"), 0.7);
            }
            if !nitro && self.nitro_on && !fleet_on {
                a.play(&format!("{AUX}/nitrous_tail"), 0.4);
            }
            self.nitro_on = nitro;
            // tyres, road, wind, landings, horn: the port's own sounds where the sound objects are missing
            // (the original has no wind loop and no landing sound of its own: caraudio.rs)
            let old = !fleet_on;
            let slip = st.wheels.iter().filter(|w| w.grounded).map(|w| (w.slip - 0.92).max(0.0)).fold(0.0, f32::max);
            let skid_t = if grounded && speed > 4.0 { (slip * 2.2).min(1.0) } else { 0.0 };
            if old && skid_t > 0.25 && self.skid < 0.05 {
                if let Some(k) = a.pick(AUX, "tire_chirp") {
                    a.play(&k, 0.4);
                }
            }
            self.skid += (skid_t - self.skid) * (dt * 10.0).min(1.0);
            if old && self.skid > 0.02 {
                a.set_loop(1, &format!("{AUX}/skids_01"), (self.skid * 0.5, self.skid * 0.5), 0.9 + (speed / 60.0).min(0.35));
            }
            // road and wind
            let roll = (speed / 55.0).clamp(0.0, 1.0);
            if old && grounded && roll > 0.03 {
                // (the tyre loop of what the car is rolling on - the terrain recordings of the game)
                use crate::physics::collision::Surface;
                let under = st.wheels.iter().find(|w| w.grounded).map(|w| Surface::from_u8(w.surface).kind());
                let surface = match under {
                    Some(Surface::Grass) => "terrain_grass",
                    Some(Surface::Gravel) | Some(Surface::Dirt) | Some(Surface::Railroad) => "terrain_gravel",
                    Some(Surface::Sand) => "terrain_sand",
                    Some(Surface::Sidewalk) => "terrain_sidewalk",
                    Some(Surface::Brick) | Some(Surface::Cobblestone) => "terrain_brick_01_fin",
                    Some(Surface::Wood) => "terrain_woodloop_01_fin",
                    Some(Surface::Stairs) => "terrain_stair_loop_fin_01",
                    Some(Surface::River) => "terrain_river_",
                    Some(Surface::Water) => "terrain_water_tireloop_fin",
                    Some(Surface::WaterDeep) => "terrain_ocean_deep_01",
                    _ if rain > 0.5 => "terrain_water_tireloop_fin",
                    _ => "gnr_terrain_1",
                };
                a.set_loop(2, &format!("{AUX}/{surface}"), (roll * 0.3, roll * 0.3), 0.7 + roll * 0.6);
            }
            let wind = ((speed - 18.0) / 60.0).clamp(0.0, 1.0);
            if old && wind > 0.01 {
                a.set_loop(3, &format!("{AUX}/wind_01_l"), (wind * 0.35, wind * 0.2), 0.8 + wind * 0.5);
                a.set_loop(4, &format!("{AUX}/wind_01_r"), (wind * 0.2, wind * 0.35), 0.8 + wind * 0.5);
            }
            if old && wind > 0.6 {
                let w = (wind - 0.6) * 2.0;
                a.set_loop(5, &format!("{AUX}/wind_high_01_l"), (w * 0.3, w * 0.3), 1.0);
            }
            // landings
            if old && self.airborne && grounded && st.airborne_time < 0.05 {
                if let Some(k) = a.pick(AUX, "suspension_car_down") {
                    a.play(&k, 0.6);
                }
            }
            self.airborne = !grounded && st.airborne_time > 0.35;
            // crashes
            let dd = st.damage - self.damage;
            if self.coll.is_none() && (dd > 0.004 || st.last_impact > 0.45) && self.impact_cool <= 0.0 {
                let hard = dd > 0.04 || speed > 22.0;
                let k = if hard { a.pick(CAR_BANG, "vehicle_bangers_impact") } else { a.pick(CAR_BANG, "jw_mc4_body_dent") };
                if let Some(k) = k {
                    a.play(&k, if hard { 0.9 } else { 0.6 });
                }
                if hard {
                    if let Some(k) = a.pick(CAR_BANG, "vehicle_debris") {
                        a.play(&k, 0.5);
                    }
                    if dd > 0.08 {
                        if let Some(k) = a.pick(CAR_BANG, "vehicle_glass_break") {
                            a.play(&k, 0.5);
                        }
                    }
                    a.play("sweeteners/low_end/swtnr_crnch_gnrc_01", 0.5);
                }
                self.impact_cool = 0.35;
            }
            // horn
            if horn && old {
                let key = format!("{AUX}/mustang_horn_fin");
                a.set_loop(6, &key, (0.5, 0.5), 1.0);
            }
            self.horn = horn;
        }
        self.gear = st.gear;
        self.damage = st.damage;
        self.throttle = throttle;

        // ---- props ----------------------------------------------------------------
        for h in prop_hits.iter().filter(|_| self.coll.is_none()) {
            let vol = (0.35 + h.speed / 40.0).min(1.0);
            let key = match h.kind {
                1 => a.pick(PROP_BANG, "bngr_prop_wood_0"),
                2 => a.pick(PROP_BANG, "bngr_prop_grbgcan").or_else(|| a.pick(PROP_BANG, "bngr_prop_dmpstr")),
                3 => a.pick(PROP_BANG, "bngr_prop_prkngmtr"),
                4 => a.pick(PROP_BANG, "bngr_prop_nwsstnd"),
                5 => a.pick(PROP_BANG, "fence_0"),
                6 => a.pick(PROP_BANG, "firehydrant"),
                7 => a.pick(PROP_BANG, "trfk_cone").or_else(|| a.pick(PROP_BANG, "plastic_slap")),
                8 => a.pick(COLL, "strt_lit_col").or_else(|| a.pick(PROP_BANG, "bngr_prop_stopsgn")),
                9 => a.pick(PROP_BANG, "bush_impact"),
                10 => Some(format!("{PROP_BANG}/yellow_bollard")),
                11 => a.pick(COLL, "glasspane").or_else(|| a.pick(COLL, "bus_stop")),
                12 => Some("bangers/prop_scrapes/crdbrdscrp_01".to_string()),
                _ => a.pick(COLL, "metal_generic").or_else(|| a.pick(COLL, "metal_reson")),
            };
            if let Some(k) = key {
                a.play_at(&k, h.pos, vol, 9.0);
            }
        }

        // ---- other vehicles (the port's own sounds where the sound objects are missing) -------------
        if driving && !fleet_on {
            let mut near: Vec<(f32, usize)> = vehicles.cars.iter().enumerate().filter(|(_, c)| !c.is_player && c.parked.is_none() && c.placed).map(|(i, c)| (c.state.pos.distance(cam_pos), i)).filter(|(d, _)| *d < 90.0).collect();
            near.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
            for (slot, (_, i)) in near.iter().take(5).enumerate() {
                let c = &vehicles.cars[*i];
                let g = a.gains_at(c.state.pos, 0.9, 7.0);
                let thr = c.input.throttle;
                // racers and cruisers have their own banks; traffic shares the ambient set
                if !a.engine(1 + slot as u64 + (*i as u64 % 7) * 8, &c.model.name, c.state.rpm, thr, g) {
                    let v = c.state.speed();
                    // (the engine recorded for the model's sound class)
                    let key = match (self.amb_types.get(&c.model.name.to_ascii_lowercase()).copied(), *i % 3) {
                        (Some(1), 0) | (None, 1) => "ambients/vehicles/ambveh_esc_steady",
                        (Some(1), _) | (None, 2) => "ambients/vehicles/ambveh_rav4_intosteady_fin",
                        (Some(2), _) => "ambients/vehicles/ambveh_f150_into_steady_fin",
                        (Some(3), _) => "ambients/vehicles/penske_intosteady_fin",
                        (_, 0) => "ambients/vehicles/ambveh_cblt_steady",
                        (_, 1) => "ambients/vehicles/ambveh_volvo_intosteady_fin",
                        _ => "ambients/vehicles/lotus_intosteady_fin",
                    };
                    let k = 0.45 + (v / 30.0).min(1.0) * 0.55;
                    a.set_loop(0x2000 + *i as u64, key, (g.0 * k, g.1 * k), 0.65 + (v / 28.0).min(1.2) * 0.6);
                }
            }
            // sirens
            if police.lights_on() {
                for (i, c) in vehicles.cars.iter().enumerate().filter(|(_, c)| c.cop) {
                    let g = a.gains_at(c.state.pos, 1.1, 18.0);
                    a.set_loop(0x3000 + i as u64, &format!("{AUX}/police_siren_loop_01_fin"), g, 1.0);
                }
            }
            // an impatient driver now and then
            self.traffic_horn -= dt;
            if self.traffic_horn <= 0.0 {
                self.traffic_horn = 9.0 + (a.pick(AUX, "horn_by").map_or(0, |k| k.len()) % 7) as f32 * 2.0;
                if let Some((_, i)) = near.iter().find(|(d, i)| *d > 12.0 && !vehicles.cars[*i].cop && vehicles.cars[*i].state.speed() < 3.0) {
                    if let Some(k) = a.pick(AUX, "horn_by") {
                        a.play_at(&k, vehicles.cars[*i].state.pos, 0.6, 10.0);
                    }
                }
            }
        }

        // ---- police ---------------------------------------------------------------
        // (step 19d: the payback client's friends have no siren and no radio)
        if self.heat != Some(police.heat) && !police.goons && !fleet_on {
            match police.heat {
                Heat::PullOver => {
                    a.play(&format!("{AUX}/police_siren_alert_02"), 0.6);
                    self.bullhorn(a);
                }
                Heat::Pursuit => {
                    a.play(&format!("{AUX}/police_siren_alert_01"), 0.7);
                    self.radio = 2.0;
                }
                Heat::Busted => a.play(&format!("{AUX}/radio_cutoff_squelch_01"), 0.6),
                _ => {}
            }
        }
        self.heat = Some(police.heat);
        if matches!(police.heat, Heat::Pursuit | Heat::Search) && !police.goons && !fleet_on {
            self.radio -= dt;
            if self.radio <= 0.0 {
                self.radio = 9.0 + (a.pick(AUX, "radio_cutoff_squelch").map_or(0, |k| k.len()) % 5) as f32 * 2.0;
                if !self.scripted_radio {
                    self.police_radio(a);
                }
            }
        }

        // ---- abilities ------------------------------------------------------------
        let on = ability.active > 0.0;
        if let Some(lib) = &mut self.lib {
            // GLOBAL_SPECIAL_ABILITIES: 17-byte blocks {flag, +1 start, +5 exit, +9 extra} at +10 Zone, +27 Agro,
            // +44 Roar, +61 Pulse; the exit stops the start sound (F-43041); the ability's dynamic mixer while on
            let block = match ability.kind {
                crate::ability::Kind::Zone => 10,
                crate::ability::Kind::Agro => 27,
                crate::ability::Kind::Roar => 44,
                crate::ability::Kind::Emp => 61,
            };
            let field = |lib: &crate::sndobj::SoundLib, at: usize| lib.game.get_name("GLOBAL_SPECIAL_ABILITIES").and_then(|o| o.get(block + at..block + at + 4)).map(|b| u32::from_be_bytes(b.try_into().unwrap())).unwrap_or(0);
            if on && !self.ability_on {
                let h = field(lib, 1);
                self.ability_snd = lib.play_hash(h, crate::sndobj::Req::default());
            }
            if !on && self.ability_on {
                if let Some(h) = self.ability_snd.take() {
                    lib.stop(h);
                }
                let h = field(lib, 5);
                lib.play_hash(h, crate::sndobj::Req::default());
            }
            for (k, n) in [(crate::ability::Kind::Zone, "DYNAMIC_MIXER_ZONE"), (crate::ability::Kind::Agro, "DYNAMIC_MIXER_AGRO"), (crate::ability::Kind::Roar, "DYNAMIC_MIXER_ROAR"), (crate::ability::Kind::Emp, "DYNAMIC_MIXER_PULSE")] {
                lib.scene(n, on && ability.kind == k);
            }
        } else if on && !self.ability_on {
            let key = match ability.kind {
                crate::ability::Kind::Zone => "special_abilities/zone/in_the_zone_enter_l",
                crate::ability::Kind::Agro => "special_abilities/agro/agro_ignite_oneshots_01.l",
                crate::ability::Kind::Roar => "special_abilities/roar/roar_ignite_master.l",
                crate::ability::Kind::Emp => "special_abilities/pulse/pulse_in.l",
            };
            a.play(key, 0.8);
        }
        if !on && self.ability_on && self.lib.is_none() {
            let key = match ability.kind {
                crate::ability::Kind::Zone => "special_abilities/zone/in_the_zone_exit_l",
                crate::ability::Kind::Emp => "special_abilities/pulse/pulse_exit.l",
                _ => "special_abilities/agro/roar_exit.l",
            };
            a.play(key, 0.6);
        }
        self.ability_on = on;
        let ready = ability.ready();
        // (the original's ability-ready alert has no caller found: no sound with the sound objects)
        if ready && !self.ability_ready && driving && self.lib.is_none() {
            a.play(&format!("{MENUS}/rep_bar"), 0.5);
        }
        self.ability_ready = ready;

        // ---- weather --------------------------------------------------------------
        if rain > 0.03 && !in_menu && self.lib.is_none() {
            a.set_loop(7, &format!("{AUX}/amb_bed_rain_int_fin"), (rain * 0.45, rain * 0.45), 1.0);
        }

        // ---- district ambience -----------------------------------------------------
        if self.beds.is_none() {
            self.beds = Some(load_beds(a.root()));
        }
        let district = game.district_at(st.pos);
        let base = if game.mode == Mode::Garage {
            "GARAGE_01"
        } else if rain > 0.5 {
            "RAIN_EXT_01"
        } else {
            match (district.as_str(), night) {
                ("Hollywood", false) => "OLD_HOLLYWOOD",
                ("Hollywood", true) => "OLD_HOLLYWOOD_NIGHT",
                ("Beaches", false) => "SANTA_MONICA",
                ("Beaches", true) => "SANTA_MONICA_NIGHT",
                ("Downtown", false) => "BUSINESS_NEW",
                ("Downtown", true) => "BUSINESS_NEW_NIGHT",
                ("Hills", false) => "MULLHOLLAND",
                ("Hills", true) => "MULHOLLAND_NIGHT",
                ("SouthCentral", false) => "SOUTH_PARK",
                ("SouthCentral", true) => "SOUTH_PARK_NIGHT",
                (_, false) => "WILSHIRE_01",
                (_, true) => "WILSHIRE_NIGHT",
            }
        };
        if let (Some(lib), Some(amb)) = (&mut self.lib, &mut self.amb) {
            // the original's district ambience (ambaudio.rs) at the listener; the port's rain bed stays until the
            // original's rain sounds are decoded
            amb.update(lib, cam_pos, night, game.mode == Mode::Garage, dt);
            // rain and thunder (WEATHER_AUDIO_RULE.md): the player's wheel cover probes, the listener's probe 50 m up
            let wheels: Vec<Vec3> = if p.model.bike { vec![st.wheels[0].world_pos, st.wheels[2].world_pos] } else { [0, 2, 1, 3].iter().map(|&i| st.wheels[i].world_pos).collect() };
            amb.cover_probe(wet, &wheels, probe);
            let covered = wet && probe(cam_pos, cam_pos + Vec3::new(0.0, 50.0, 0.0));
            amb.weather(lib, wet, self.cam_audio == 1, covered, dt);
            a.set_bed(None, 0.0);
        } else {
            let bed = self.beds.as_ref().and_then(|m| m.get(base)).cloned();
            a.set_bed(bed.as_deref(), if in_menu { 0.25 } else { 0.4 });
        }

        a.update_music((driving || game.mode == Mode::Paused) && !game.radio_held);
        self.finish(a, dt);
    }

    /// A frontend sound by name (F-42664: 0 dB, no position; the tree's speaker masks place it); the port's own
    /// sample `old` where the sound objects are missing.
    fn ui(&mut self, a: &mut Audio, name: &str, old: &str, vol: f32) {
        match &mut self.lib {
            Some(lib) => {
                if lib.play(name, crate::sndobj::Req::default()).is_none() {
                    log::warn!("audio: no sound object {name}");
                }
            }
            None if !old.is_empty() => a.play(old, vol),
            None => {}
        }
    }

    /// The sound objects drive their voices, then every voice not driven this frame fades.
    fn finish(&mut self, a: &mut Audio, dt: f32) {
        if let Some(lib) = &mut self.lib {
            lib.update(dt, a);
        }
        a.end_frame();
    }

    /// The patrol car's loudhailer.
    fn bullhorn(&mut self, a: &mut Audio) {
        // police_bullhorn: one bank per officer, lines "<unit>_bullhorn_NN"
        let root = "0xec3de1f1";
        if let Some(bank) = first_bank(a, root) {
            let idx = a.bank_index(&bank);
            if let Some(n) = idx.first() {
                let prefix = n.rsplit_once('_').map(|x| x.0.to_string()).unwrap_or_default();
                if let Some(k) = a.pick(&bank, &prefix) {
                    a.play(&k, 0.8);
                }
            }
        }
    }

    /// Dispatch chatter during a pursuit.
    fn police_radio(&mut self, a: &mut Audio) {
        a.play(&format!("{AUX}/radio_cutoff_squelch_02"), 0.4);
        if let Some(bank) = random_bank(a, "police_radio") {
            if let Some(k) = a.pick(&bank, "") {
                a.play(&k, 0.6);
            }
        }
    }
}

fn banks_of(a: &Audio, root: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(a.root().join(root)).map(|rd| rd.flatten().filter(|e| e.path().is_dir()).map(|e| format!("{}/{}", root, e.file_name().to_string_lossy())).collect()).unwrap_or_default();
    v.sort();
    v
}

fn first_bank(a: &Audio, root: &str) -> Option<String> {
    banks_of(a, root).into_iter().next()
}

fn random_bank(a: &mut Audio, root: &str) -> Option<String> {
    let v = banks_of(a, root);
    if v.is_empty() {
        return None;
    }
    let k = a.random() as usize % v.len();
    Some(v[k].clone())
}

/// The sound class of every ambient traffic model, from `tune/ambients/traffic/<model>/mcAmbientType.xml`.
pub fn load_amb_types(data_dir: &str) -> std::collections::HashMap<String, u8> {
    let mut out = std::collections::HashMap::new();
    let Ok(rd) = std::fs::read_dir(std::path::Path::new(data_dir).join("tune/ambients/traffic")) else { return out };
    for e in rd.flatten() {
        let Ok(t) = std::fs::read_to_string(e.path().join("mcAmbientType.xml")) else { continue };
        let Some(ty) = t.split("<szAudioType>").nth(1).and_then(|r| r.split('<').next()) else { continue };
        let class = match ty.trim() {
            "AMBIENT_SUV" => 1,
            "AMBIENT_PICKUP" => 2,
            "AMBIENT_SEDAN" => 0,
            _ => 3,
        };
        out.insert(e.file_name().to_string_lossy().to_ascii_lowercase(), class);
    }
    out
}
