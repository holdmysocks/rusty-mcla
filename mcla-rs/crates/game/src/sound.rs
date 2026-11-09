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
/// A prop near the player's car for the whoosh scan (F-12768): instance (`u32::MAX` none), type name hash, position,
/// velocity.
#[derive(Clone, Copy, Debug)]
pub struct NearProp {
    pub inst: u32,
    pub model: u32,
    pub pos: Vec3,
    pub vel: Vec3,
}

/// A post the deflection pass steers a car round (F-35508): the prop's instance, type name hash, place, the car's speed
/// (chassis +224, m/s) and whether the car is the player's (its loop then has no environment).
#[derive(Clone, Copy, Debug)]
pub struct PropDeflect {
    pub inst: u32,
    pub model: u32,
    pub pos: Vec3,
    pub speed: f32,
    pub player: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PropHit {
    pub pos: Vec3,
    /// the prop's instance (`Props::origin`), `u32::MAX` for none
    pub inst: u32,
    pub kind: u8,
    /// the prop type's name hash (its game.dat audio object)
    pub model: u32,
    pub speed: f32,
    pub knocked: bool,
}

/// A contact sliding along a prop this frame (F-99417's scrape): the prop's instance, place and type, the velocity of the
/// prop less the car's.
#[derive(Clone, Copy, Debug)]
pub struct PropScrape {
    pub inst: u32,
    pub pos: Vec3,
    pub model: u32,
    pub rel: Vec3,
}

/// A prop's audio entity (160 bytes in the original, F-44000): its instance, its one-shots, the scrape loop (+136) and
/// the age (+104) of its last update (+108).
struct PropEnt {
    key: u64,
    inst: u32,
    sounds: Vec<crate::sndobj::Handle>,
    scrape: Option<crate::sndobj::Handle>,
    scrape_age: f32,
    age: f32,
    /// +112: the age at the last whoosh (-FLT_MAX)
    whoosh: f32,
    /// its voices whose request +172 is set (impacts, breaks, the deflection loop): moved to the prop every update
    follow: Vec<crate::sndobj::Handle>,
}

const MUSIC_SUSPEND: &str = "DYNAMIC_MIXER_MUSIC_SUSPEND";

/// The music manager's calls (audio.rs `MusicEnv`) on the port's mixer, sounds and radio: the song is the radio's
/// sink (token 1), the jingle a sounds.dat voice, the suspend scene by name; no console player, no change callback.
struct MusicHere<'a> {
    lib: &'a mut crate::sndobj::SoundLib,
    a: &'a mut Audio,
    jingle: &'a mut Option<crate::sndobj::Handle>,
}

impl crate::audio::MusicEnv for MusicHere<'_> {
    fn music_user_db(&self) -> f32 {
        self.lib.category("MUSIC_USER").map_or(0.0, |c| c.volume)
    }
    fn scene_start(&mut self) -> Option<u32> {
        self.lib.scene_at(MUSIC_SUSPEND, true, 1.0);
        self.lib.scene_on(MUSIC_SUSPEND).then_some(1)
    }
    fn scene_stop(&mut self, _h: u32) {
        self.lib.scene(MUSIC_SUSPEND, false);
    }
    fn sound_stop(&mut self, _h: u32) {
        if let Some(h) = self.jingle.take() {
            self.lib.stop(h);
        }
    }
    fn jingle(&mut self, hash: u32) -> Option<u32> {
        *self.jingle = self.lib.play_hash(hash, crate::sndobj::Req::default());
        log::info!("music: Play's jingle {} ({})", self.lib.name_of(hash).unwrap_or("?"), if self.jingle.is_some() { "playing" } else { "not made" });
        self.jingle.map(|_| 1)
    }
    fn release(&mut self, _h: u32) {
        self.a.stop_song();
    }
    fn pick(&mut self) -> u32 {
        0
    }
    fn song_start(&mut self, _hash: u32, offset: u32, predelay: u32, _arg: u32) -> Option<u32> {
        self.a.start_song(predelay, offset).then_some(1)
    }
    fn place(&mut self, _a: u32, _b: u32) {}
    fn console_ready(&mut self) -> bool {
        false
    }
    fn console_start(&mut self, _entry: u32) {}
    fn console_stop(&mut self) {}
    fn changed(&mut self) {}
}

#[derive(Default)]
pub struct Sound {
    /// the reverb object the probes chose (F-26357; REVERB_DEFAULT until the first choice)
    reverb_obj: Option<u32>,
    /// the forced district it set (ambience manager +144)
    forced_district: Option<u32>,
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
    /// a cine scene plays (the scene byte: F-37136 .. F-65984), set by the game each frame
    pub scene: bool,
    /// every car's sounds beside the engine and collisions (caraudio.rs, with `lib`)
    fleet: Option<crate::caraudio::Fleet>,
    /// the sound occlusion's table of occluders (occlusion.rs, with `lib`) and each car's slot plus one by `uid`
    occl: Option<crate::occlusion::System>,
    occ_slots: std::collections::HashMap<u32, u16>,
    /// the hit props' entities (their occluder keys) and the sounds they still play
    /// each touched prop's entity (the pool of F-44000, here while its sounds play)
    prop_held: Vec<PropEnt>,
    /// the props near the player's car this frame, for its whoosh scan (main: `Props::near_props`)
    pub near_props: Vec<NearProp>,
    /// the posts the deflection pass steered a car round this frame (main: `Props::deflects`)
    pub prop_deflects: Vec<PropDeflect>,
    /// the map effects' sounds (ptxaudio.rs)
    ptx: crate::ptxaudio::PtxSounds,
    /// the player car's performance stages the sounds read: intake, turbo, nitrous
    pub parts: [u8; 3],
    /// the countdown's last sounded step (3, 2, 1, 0 = GO; 4 = before it; -1 = no race)
    count_stage: i32,
    /// the end sequences seen (`Game::end_seq_n`)
    end_race: u32,
    /// the reward screen's rep bar sound waits 20 movie frames
    ability_snd: Option<crate::sndobj::Handle>,
    nitro_btn: bool,
    /// a speech or police radio line is playing (set by the game each frame)
    pub speech_busy: bool,
    /// the career speech line said at the player's car (F-32206 puts it at the entity's position), followed while it plays
    speech_line: Option<crate::sndobj::Handle>,
    /// the career speech line last started (stopped by `ObjCmd::LineStop`)
    line_h: Option<crate::sndobj::Handle>,
    /// cruisers already reported disabled on the radio
    pub cop_disabled: std::collections::HashSet<usize>,
    /// set by the game each frame: the HUD movie's `damage_alpha`, a splash posted to the HUD's queue 1, the pause
    /// movie's `cur_camera`
    pub damage_alpha: f32,
    /// the names the UI movies started (`flash::Player::sounds_started`), played by name (F-42664: no position, the
    /// default parameters)
    pub movie_sounds: Vec<std::rc::Rc<str>>,
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
    prop_loops: Option<Vec<(Vec3, u32, Vec<u8>, Option<crate::sndobj::Handle>)>>,
    phone: bool,
    /// the music manager's jingle (its +500)
    music_jingle: Option<crate::sndobj::Handle>,
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
    /// CopPullover.sco's POLICE_SCRIPT_TRIGGERED (DYNAMICMIXER_TriggerState F-13109 at 0x7b5..0x88c, as the walk starts;
    /// DeTriggerState F-52510 at the bolt 0x9a7.. or once the ticket scene has ended 0xb8b..0xc3e)
    pub pullover_mix: bool,
    /// the story scripts' states, by the story scene that plays (main): MUTE_SHENANIGANS through
    /// CUT_STORY_REP_ON_STRIP_GARAGE (Story cores of RACE_GH_02 0x44e3..0x480f and RACE_SH_01 0x2506..0x265b: on before
    /// the warp to it, off once it has ended) and, for the first frame of CUT_STORY_RACE_GH_06, MUTE_SHENANIGANS +
    /// HENRY_DRIVEBY (RACE_GH_06_READY_TO_START_ON_ENTER: on at 0x910 / 0x938 right before the scene starts, off at 0x9fd /
    /// 0xa22 after the first 10 ms wait of its loop)
    pub story_mix: (bool, bool),
    /// SLOWMO_FORM, mixer slot 17: mcSlowMotionForm (form 108) shown (vt+400, F-22641: started unless running) until
    /// hidden (vt+756, F-21566: stopped if running), and AudioManager_SetActiveSlowMotionForm (1 starts / 0 stops the
    /// same way); main: the tutorial's form 108 up, or a hangout entry from the line crossing (form 108 shown) until the
    /// core's native (0) right after MISSION_EVENT (the hide at the ease from pause is followed at once by native (1))
    pub slowmo_form: bool,
    /// the story scene of the last update (for the first frame of a scene)
    pub story_cut: Option<String>,
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

/// One frame of the sound occlusion: an occluder for each car with a car audio entity (the player's and the computer
/// racers' and cruisers', `Fleet::opponent`; made at the car audio's start, F-43725) and for each ambient car (made off,
/// switched by its update: `switch_ambient`), the slots of cars gone given back,
/// the manager's updates and the ticks (`occlusion::System::step`), every occluder's outputs to the voices (F-19918).
/// The cars' slots plus one by `uid`.
fn occlude(oc: &mut crate::occlusion::System, lib: &mut crate::sndobj::SoundLib, vehicles: &Vehicles, dt: f32, world: &mut impl crate::occlusion::Probe) -> std::collections::HashMap<u32, u16> {
    let l = lib.listener.0;
    let l4 = [l.x, l.y, l.z, 0.0];
    let mut slots = std::collections::HashMap::new();
    for c in &vehicles.cars {
        let slot = if c.is_player || crate::caraudio::Fleet::opponent(c) {
            oc.car(c.uid, c.state.pos.to_array(), l4, world)
        } else {
            // (approx.) every ambient car has its entity here; the original makes one for the cars the ambient
            // manager gives audio to
            oc.ambient(c.uid, c.state.pos.to_array())
        };
        if let Some(i) = slot {
            slots.insert(c.uid, (i + 1) as u16);
        }
    }
    oc.keep(&slots.keys().copied().collect::<Vec<_>>());
    oc.step(l4, dt, world);
    for i in 0..crate::occlusion::SLOTS {
        if let Some(o) = oc.out(i) {
            lib.set_occlusion(i, o);
        }
    }
    slots
}

/// The ambience's two slot occluders on the table (`occlusion::System::amb_*`) for its update.
struct AmbOccluders<'a, 'w> {
    oc: &'a mut crate::occlusion::System,
    world: crate::occlusion::WorldProbe<'w>,
    l: [f32; 4],
}

impl crate::ambaudio::SlotOccluders for AmbOccluders<'_, '_> {
    fn place(&mut self, k: usize, p: Vec3) -> u16 {
        self.oc.amb_place(k, p.to_array()).map_or(0, |i| (i + 1) as u16)
    }
    fn updated(&self, k: usize) -> bool {
        self.oc.amb_updated(k)
    }
    fn now(&mut self, k: usize) {
        let l = self.l;
        self.oc.amb_now(k, l, &mut self.world);
    }
    fn reset(&mut self, k: usize) {
        self.oc.amb_reset(k);
    }
}

/// The map effects' entities' occluders on the table (`occlusion::System::effect*`).
struct EffectOccluders<'a, 'w> {
    oc: &'a mut crate::occlusion::System,
    world: crate::occlusion::WorldProbe<'w>,
    l: [f32; 4],
}

impl crate::ptxaudio::EffectOccluders for EffectOccluders<'_, '_> {
    fn bind(&mut self, key: u64, origin: Vec3, place: [f32; 3], now: bool) -> u16 {
        let l = self.l;
        self.oc.effect(key, origin.to_array(), place, now, l, &mut self.world).map_or(0, |i| (i + 1) as u16)
    }
    fn follow(&mut self, key: u64, origin: Vec3) {
        self.oc.effect_follow(key, origin.to_array());
    }
    fn volume(&self, key: u64) -> Option<f32> {
        self.oc.effect_slot(key).and_then(|i| self.oc.out(i)).map(|o| o[0])
    }
    fn release(&mut self, key: u64) {
        self.oc.release_effect(key);
    }
}

/// A prop type object's occluder place (+69 / +73 / +77, F-95562).
fn prop_place(o: &[u8]) -> [f32; 3] {
    let f = |at: usize| o.get(at..at + 4).map_or(0.0, |b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    [f(69), f(73), f(77)]
}

/// The ambient cars' switches (uid, on, the whoosh's immediate update) applied to their occluders, and every
/// occluder's outputs to the voices again.
fn switch_ambient(oc: &mut crate::occlusion::System, lib: &mut crate::sndobj::SoundLib, sw: &[(u32, bool, bool)], world: &mut impl crate::occlusion::Probe) {
    let l = lib.listener.0;
    let l4 = [l.x, l.y, l.z, 0.0];
    for &(uid, on, now) in sw {
        if let Some(i) = oc.slot_of(uid) {
            if now {
                oc.now(i, l4, world);
            }
            oc.switch(i, on, l4, world);
        }
    }
    for i in 0..crate::occlusion::SLOTS {
        if let Some(o) = oc.out(i) {
            lib.set_occlusion(i, o);
        }
    }
}

/// The sound library and what is made with it, read off the game loop (the start-up's loading thread,
/// boot.md "Loading behind the movie"; or at the first sound update as before).
pub struct Preloaded {
    pub lib: Option<crate::sndobj::SoundLib>,
    coll: Option<crate::collaudio::CollisionAudio>,
    amb: Option<crate::ambaudio::Ambience>,
    fleet: Option<crate::caraudio::Fleet>,
    occl: Option<crate::occlusion::System>,
    beds: Option<hashbrown::HashMap<String, String>>,
}

impl Preloaded {
    /// `root` = the audio root, `cfg` = `audio/X360/config`; `all`: the ambience beds and the stream index too
    /// (otherwise read when first needed).
    pub fn load(root: &std::path::Path, cfg: &std::path::Path, all: bool) -> Preloaded {
        let mut lib = crate::sndobj::SoundLib::load(root, cfg);
        let (mut coll, mut amb, mut fleet, mut occl) = (None, None, None, None);
        if let Some(l) = lib.as_mut() {
            // (cfg = <data>/audio/X360/config)
            let data = cfg.ancestors().nth(3).map(|p| p.to_path_buf()).unwrap_or_default();
            coll = Some(crate::collaudio::CollisionAudio::new(&data));
            amb = crate::ambaudio::Ambience::new(l);
            fleet = Some(crate::caraudio::Fleet::new(&data));
            // GLOBAL_OCCLUSION as F-43021 reads it, and the materials' factors
            occl = l.game.get_name("GLOBAL_OCCLUSION").map(|o| o.to_vec()).and_then(|o| {
                let t = crate::occlusion::Tune::read(&o, |h| l.curve(h).cloned())?;
                crate::occlusion::System::new(t, &o, crate::occlusion::material_factors(&data, &l.game))
            });
            if all {
                l.index_streams();
            }
        } else {
            log::warn!("audio: {} not loaded - the original's sound objects are not played", cfg.display());
        }
        Preloaded { lib, coll, amb, fleet, occl, beds: all.then(|| load_beds(root)) }
    }
}

impl Sound {
    /// The player engine's start (F-44510 with ignition 0, from CopArrest's Racer_PlayIgnition F-35902 / F-91984):
    /// the engine sound stops and plays the ignition, cranks and flares (engaudio.rs `start`); from the request on the
    /// control step holds the car and lifts `engine_off` (`engine_held`, F-17997).
    pub fn play_ignition(&mut self) {
        self.ignition = 1;
    }

    /// The player's wipeout slow motion starts (F-46437, offline, the local player): its audio entity's vt+36 (F-46824)
    /// sets +2200 = -1 (no PLAYER_WIPE_OUT from the wreck start's 0.25 s timer, which the port does not have) and,
    /// unless a speech line plays (+2256 and +2260), asks for PLAYER_WIPE_OUT_TIMEWARP ("PLAYER") on a car, not a bike.
    /// Not ported: then TAUNT_RACE_BIG_CRASH in a race of more than one racer through the race's +208 speaker (not
    /// identified).
    pub fn wipeout_warp(&mut self, bike: bool) {
        if !self.speech_busy && !bike {
            self.want_speech = Some("PLAYER_WIPE_OUT_TIMEWARP");
        }
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

    /// A cruiser's call sign.
    pub fn radio_callsign(&mut self, car: usize) -> String {
        self.fleet.as_mut().map(|f| f.callsign_of(car)).unwrap_or_default()
    }

    /// Take a [`Preloaded`] sound library (the first sound update does it itself when nothing was handed over).
    pub fn adopt(&mut self, p: Preloaded) {
        self.lib_tried = true;
        self.lib = p.lib;
        self.coll = p.coll;
        self.amb = p.amb;
        self.fleet = p.fleet;
        self.occl = p.occl;
        if p.beds.is_some() {
            self.beds = p.beds;
        }
    }

    /// The walkers' reaction sounds (F-39461, the mcAmbientPedAudioEntity pool): the type's AMBIENT_PED object, the
    /// kind's sound at the walker and following it (`bike`: the player entity's bike byte), each with its entity's
    /// occluder (`occlusion::System::person`: placed (0, 1.5, 0) on the walker, the immediate update at the start; let
    /// go once the sound is over).
    /// The map effects' sounds (mcParticleAudioEntity, F-83177 from the audio update): the effect instances playing now
    /// (`Particles::instances`), each entity through its occluder.
    pub fn effects(&mut self, live: &[(u32, String, Vec3)], collider: Option<&crate::physics::collision::WorldCollider>) {
        let Some(lib) = self.lib.as_mut() else { return };
        let l = lib.listener.0;
        match &mut self.occl {
            Some(oc) => {
                let factors = oc.factors.clone();
                let mut eo = EffectOccluders { oc, world: crate::occlusion::WorldProbe::of(collider, &factors), l: [l.x, l.y, l.z, 0.0] };
                self.ptx.update(lib, live, &mut eo);
                for i in 0..crate::occlusion::SLOTS {
                    if let Some(o) = eo.oc.out(i) {
                        lib.set_occlusion(i, o);
                    }
                }
            }
            None => self.ptx.update(lib, live, &mut crate::ptxaudio::NoOccluders),
        }
    }

    pub fn ped_reactions(&mut self, peds: &mut crate::peds::Peds, bike: bool, collider: Option<&crate::physics::collision::WorldCollider>) {
        let Some(lib) = self.lib.as_mut() else { return };
        let key = |k: (usize, u32)| ((k.0 as u64) << 32) | k.1 as u64;
        let l = lib.listener.0;
        let l4 = [l.x, l.y, l.z, 0.0];
        let factors = self.occl.as_ref().map(|o| o.factors.clone()).unwrap_or_default();
        let mut world = crate::occlusion::WorldProbe::of(collider, &factors);
        for sp in std::mem::take(&mut peds.speech) {
            let Some(h) = lib.ped_reaction(&sp.audio, sp.female, sp.kind, bike) else { continue };
            if peds.voices_playing.busy(lib, sp.key) {
                continue;
            }
            let p = sp.pos.to_array();
            let slot = self.occl.as_mut().and_then(|oc| oc.person(key(sp.key), p, l4, &mut world));
            let occ = slot.map_or(0, |i| (i + 1) as u16);
            if let (Some(oc), Some(i)) = (&self.occl, slot) {
                if let Some(o) = oc.out(i) {
                    lib.set_occlusion(i, o);
                }
            }
            peds.voices_playing.start(lib, sp.key, h, sp.pos, occ);
        }
        let mut v = std::mem::take(&mut peds.voices_playing);
        let gone = v.follow(lib, |k| peds.pos_of(k));
        if let Some(oc) = &mut self.occl {
            for k in gone {
                oc.release_person(key(k));
            }
            for (k, p) in v.live() {
                oc.person(key(k), p.to_array(), l4, &mut crate::occlusion::NoProbe);
            }
        }
        peds.voices_playing = v;
    }

    /// Whether the sound library is there (the object sounds can play).
    pub fn has_lib(&self) -> bool {
        self.lib.is_some()
    }

    /// A persistent object sound started outside the game loop (the start-up movie's loading music), kept as
    /// [`crate::audio::Audio::obj_loop`] keeps its own.
    pub fn keep_loop(&mut self, name: &str, h: crate::sndobj::Handle) {
        self.obj_loops.insert(name.to_string(), h);
    }
}

impl Sound {
    /// The car's horn sounds (its audio's +1404 slot, `Fleet::horn_sounding`).
    pub fn horn_sounding(&self, uid: u32) -> bool {
        self.fleet.as_ref().is_some_and(|f| f.horn_sounding(uid))
    }

    #[allow(clippy::too_many_arguments)]
    /// `rain`: the weather's rain share (wet = not 0); `probe(from, to)`: the city's collision has something on that line.
    /// `probe_at(from, to)`: the material (Surface) of the nearest hit on that line.
    /// The tach's rpm: the HUD's d_RPMvalue is the player's engine sound's slewed rpm (engine sound +204,
    /// F-86111 from F-10725's follower: at most its slew a second - 250 rpm a frame for most cars),
    /// not the engine's own. None while the car has no engine sound of the original's.
    pub fn tach_rpm(&self) -> Option<f32> {
        self.engine.as_ref().map(|(_, e)| e.smoothed_rpm())
    }

    pub fn update(&mut self, a: &mut Audio, vehicles: &Vehicles, game: &Game, police: &Police, ability: &Abilities, prop_hits: &[PropHit], prop_scrapes: &[PropScrape], prop_matrix: &dyn Fn(u32) -> Option<[[f32; 4]; 4]>, cam_pos: Vec3, cam_right: Vec3, cam_fwd: Vec3, rain: f32, horn: bool, night: bool, probe: &dyn Fn(Vec3, Vec3) -> bool, probe_at: &dyn Fn(Vec3, Vec3) -> Option<u8>, collider: Option<&crate::physics::collision::WorldCollider>, dt: f32) {
        let wet = rain > 0.0;
        a.set_listener(cam_pos, cam_right);
        if !self.lib_tried {
            self.adopt(Preloaded::load(a.root(), &a.cfg, false));
        }
        a.objects = self.lib.is_some();
        if let Some(lib) = &mut self.lib {
            // what the rest of the game asked for this frame (Audio::obj ...)
            let mut loops_now: Vec<String> = Vec::new();
            // (the music manager's slots: a jingle that ended, a suspend scene whose fade-out is over and a song that
            // ended empty theirs)
            if self.music_jingle.is_some_and(|h| !lib.playing(h)) {
                self.music_jingle = None;
            }
            a.mgr.jingle = self.music_jingle.map(|_| 1);
            if !lib.scene_on(MUSIC_SUSPEND) {
                a.mgr.suspend = None;
            }
            a.mgr.song = a.song_live().then_some(1);
            for c in std::mem::take(&mut a.obj_cmds) {
                match c {
                    crate::audio::ObjCmd::MusicPlay { arg } => {
                        let tune = lib.game.get_name("MUSIC_0_MANAGER").and_then(crate::audio::MusicTune::read);
                        if let Some(t) = tune {
                            let mut m = std::mem::take(&mut a.mgr);
                            m.play(&t, arg, &mut MusicHere { lib, a, jingle: &mut self.music_jingle });
                            a.mgr = m;
                        } else {
                            // (no manager object: the radio starts as it did)
                            a.mgr.handler = crate::audio::Handler::Playing;
                        }
                    }
                    crate::audio::ObjCmd::MusicSuspend => {
                        let tune = lib.game.get_name("MUSIC_0_MANAGER").and_then(crate::audio::MusicTune::read);
                        if let Some(t) = tune {
                            let mut m = std::mem::take(&mut a.mgr);
                            m.suspend(&t, &mut MusicHere { lib, a, jingle: &mut self.music_jingle });
                            a.mgr = m;
                        }
                    }
                    crate::audio::ObjCmd::Loop { name } => {
                        let alive = self.obj_loops.get(&name).is_some_and(|h| lib.playing(*h));
                        if !alive {
                            if let Some(h) = lib.play(&name, crate::sndobj::Req::default()) {
                                log::info!("audio: loop {name} started");
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
                            self.line_h = Some(h);
                            if pos.is_some() {
                                self.speech_line = Some(h);
                            }
                        }
                    }
                    crate::audio::ObjCmd::LineStop => {
                        if let Some(h) = self.line_h.take() {
                            lib.stop(h);
                        }
                        self.speech_line = None;
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
            // (test hook MCLA_LISTENER_TRACE=1: the player's car in the listener's axes and its pan, every frame)
            if std::env::var_os("MCLA_LISTENER_TRACE").is_some() {
                if let Some(p) = vehicles.player() {
                    let d = p.state.pos - cam_pos;
                    let v = Vec3::new(d.dot(cam_right), d.dot(cam_right.cross(cam_fwd)), d.dot(cam_fwd));
                    let (l, r) = crate::sndobj::pan_gains(v);
                    log::info!("listener: cam {:.3} {:.3} {:.3} fwd {:.4} {:.4} {:.4} car {:.3} {:.3} {:.3} rel {:.3} {:.3} {:.3} share {:.4} speed {:.2}", cam_pos.x, cam_pos.y, cam_pos.z, cam_fwd.x, cam_fwd.y, cam_fwd.z, p.state.pos.x, p.state.pos.y, p.state.pos.z, v.x, v.y, v.z, l / (l + r), p.state.speed());
                }
            }
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
            // port's stand.) Every update: HPF (+90) = u16 [98] + (1 - t1) * ([104] - [98]), t1 = clamp01((d - [94]) /
            // ([100] - [94])), LPF (+88) likewise from [110] / [116] / [106] / [112] (the CESSNA: 0 Hz within 400 m, 400 Hz
            // at 700 m; the LPF stays 23900)
            if self.prop_loops.is_none() && !self.prop_places.is_empty() {
                let null = mcla_formats::hash::joaat("NULL_SOUND");
                let mut v = Vec::new();
                for &(ty, pos) in &self.prop_places {
                    let Some(o) = lib.game.get(ty).filter(|o| o.first() == Some(&43) && o.len() >= 118) else { continue };
                    let h = u32::from_be_bytes(o[86..90].try_into().unwrap());
                    if h != 0 && h != null {
                        v.push((pos, h, o.to_vec(), None));
                    }
                }
                log::info!("audio: {} props with a distance loop", v.len());
                self.prop_loops = Some(v);
            }
            let factors = self.occl.as_ref().map(|o| o.factors.clone()).unwrap_or_default();
            let mut world = crate::occlusion::WorldProbe::of(collider, &factors);
            let l4 = [cam_pos.x, cam_pos.y, cam_pos.z, 0.0];
            for (k, (pos, h, o, playing)) in self.prop_loops.iter_mut().flatten().enumerate() {
                let f = |at: usize| f32::from_bits(u32::from_be_bytes(o[at..at + 4].try_into().unwrap()));
                let u = |at: usize| u16::from_be_bytes([o[at], o[at + 1]]) as f32;
                let d = pos.distance(cam_pos);
                let near = d <= f(90);
                let filt = |d0: usize, d1: usize, a: usize, b: usize| {
                    let t = ((d - f(d0)) / (f(d1) - f(d0))).max(0.0).min(1.0);
                    (1.0 - t).mul_add(u(b) - u(a), u(a)) as i64 as u16
                };
                // the prop entity's occluder while its loop plays (F-32775: switched on with no update)
                let key = (1u64 << 63) | k as u64;
                let occ = if near {
                    self.occl.as_mut().and_then(|oc| oc.prop(key, crate::occlusion::System::frame(pos.to_array()), prop_place(o), false, false, l4, &mut world)).map_or(0, |i| (i + 1) as u16)
                } else {
                    0
                };
                let req = crate::sndobj::Req { pos: Some(*pos), hpf: filt(94, 100, 98, 104), lpf: filt(106, 112, 110, 116), occluder: occ, ..Default::default() };
                match (near, *playing) {
                    (true, Some(p)) if lib.playing(p) => lib.set(p, req),
                    (true, _) => *playing = lib.play_hash(*h, req),
                    (false, Some(p)) => {
                        lib.stop(p);
                        *playing = None;
                        if let Some(oc) = &mut self.occl {
                            oc.release_prop(key);
                        }
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
            // (DYNAMIC_MIXER_GARAGE is the garage district's +26 scene: ambaudio.rs, F-93563)
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
            for n in std::mem::take(&mut self.movie_sounds) {
                if lib.play(&n, crate::sndobj::Req::default()).is_none() {
                    log::warn!("audio: no sound object {n} (started by a movie)");
                }
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
        // ---- sound occlusion (occlusion.rs): the cars' occluders, the manager's updates and the ticks; their outputs go
        // to the voices whose requests name them
        if let (Some(lib), Some(oc)) = (&mut self.lib, &mut self.occl) {
            let factors = oc.factors.clone();
            self.occ_slots = occlude(oc, lib, vehicles, dt, &mut crate::occlusion::WorldProbe::of(collider, &factors));
            if let Some(f) = &mut self.fleet {
                f.occ = self.occ_slots.iter().map(|(k, v)| (*k, *v)).collect();
            }
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
                coll.car(lib, i, &c.model.name, owner, c.model.bike, &c.state, c.is_player, self.occ_slots.get(&c.uid).copied().unwrap_or(0));
            }
            // the Agro crashes of the frame's strikes, on the struck ambient cars (F-78148)
            for &(uid, at) in ability.agro_crashes() {
                if let Some((i, c)) = vehicles.cars.iter().enumerate().find(|(_, c)| c.uid == uid) {
                    coll.agro_crash(lib, i, &c.model.name, crate::collaudio::Owner::Ambient, c.model.bike, &c.state, at, c.is_player, self.occ_slots.get(&c.uid).copied().unwrap_or(0));
                }
            }
            coll.retain(lib, |i| i < vehicles.cars.len());
            // each hit prop's entity: its occluder updated at once (F-99417), let go when its sounds are over
            let factors = self.occl.as_ref().map(|o| o.factors.clone()).unwrap_or_default();
            let mut world = crate::occlusion::WorldProbe::of(collider, &factors);
            let l = lib.listener.0;
            let l4 = [l.x, l.y, l.z, 0.0];
            // one entity per prop instance (F-95562 binds the instance); a prop without one by its type and place
            let key_of = |inst: u32, model: u32, pos: Vec3| {
                if inst != u32::MAX {
                    1u64 << 62 | inst as u64
                } else {
                    (((model as u64) << 31) ^ (pos.x.to_bits() as u64) ^ ((pos.z.to_bits() as u64) << 7)) & !(3u64 << 62)
                }
            };
            let frame_of = |inst: u32, pos: Vec3| prop_matrix(inst).unwrap_or_else(|| crate::occlusion::System::frame(pos.to_array()));
            // a contact's entity: bound (its occluder at the type's place, the immediate update with +62 bit 0x08 as the
            // local flag, F-99417 -> F-65914), its occluder slot plus one
            let mut bind = |occl: &mut Option<crate::occlusion::System>, lib: &mut crate::sndobj::SoundLib, held: &mut Vec<PropEnt>, inst: u32, model: u32, pos: Vec3| -> (usize, u16) {
                let key = key_of(inst, model, pos);
                let obj = lib.game.get(model).filter(|o| o.first() == Some(&43)).or_else(|| lib.game.get_name("LA_PROP_DEFAULT")).map(|o| o.to_vec());
                let slot = match (occl.as_mut(), &obj) {
                    (Some(oc), Some(o)) => oc.prop(key, frame_of(inst, pos), prop_place(o), true, o.get(62).is_some_and(|b| b & 0x08 != 0), l4, &mut world),
                    _ => None,
                };
                if let (Some(oc), Some(i)) = (occl.as_ref(), slot) {
                    if let Some(o) = oc.out(i) {
                        lib.set_occlusion(i, o);
                    }
                }
                let e = match held.iter().position(|e| e.key == key) {
                    Some(e) => e,
                    None => {
                        held.push(PropEnt { key, inst, sounds: Vec::new(), scrape: None, scrape_age: 0.0, age: 0.0, whoosh: f32::MIN, follow: Vec::new() });
                        held.len() - 1
                    }
                };
                (e, slot.map_or(0, |i| (i + 1) as u16))
            };
            for h in prop_hits {
                let (e, occ) = bind(&mut self.occl, lib, &mut self.prop_held, h.inst, h.model, h.pos);
                let hs = coll.prop(lib, h.model, h.pos, h.speed, h.knocked, occ);
                // (impact and break: request +172 = 1, F-99417 / the break loop F-53969)
                self.prop_held[e].follow.extend(hs.iter().copied());
                self.prop_held[e].sounds.extend(hs);
            }
            // the scrapes (F-99417): the type's loop on the entity's slot +136 at the prop, +108 = its age
            for sc in prop_scrapes {
                let (e, occ) = bind(&mut self.occl, lib, &mut self.prop_held, sc.inst, sc.model, sc.pos);
                let at = prop_matrix(sc.inst).map_or(sc.pos, |m| Vec3::new(m[3][0], m[3][1], m[3][2]));
                let ent = &mut self.prop_held[e];
                if coll.prop_scrape(lib, sc.model, sc.rel, at, occ, &mut ent.scrape) {
                    ent.scrape_age = ent.age;
                }
            }
            // the deflections (F-35508: on a post the pass steers a car round): the gate (F-25269) binds the prop's entity when its
            // type has a deflection loop ([58] != NULL_SOUND; an existing entity always), then F-94867 runs the loop on the
            // entity's slot +136, +108 = its age
            for dfl in self.prop_deflects.iter() {
                let key = key_of(dfl.inst, dfl.model, dfl.pos);
                let Some(obj) = lib.game.get(dfl.model).filter(|o| o.first() == Some(&43)).or_else(|| lib.game.get_name("LA_PROP_DEFAULT")).map(|o| o.to_vec()) else { continue };
                let null = mcla_formats::hash::joaat("NULL_SOUND");
                if !self.prop_held.iter().any(|e| e.key == key) && obj.get(58..62).map_or(null, |b| u32::from_be_bytes(b.try_into().unwrap())) == null {
                    continue;
                }
                let (e, occ) = bind(&mut self.occl, lib, &mut self.prop_held, dfl.inst, dfl.model, dfl.pos);
                let at = prop_matrix(dfl.inst).map_or(dfl.pos, |m| Vec3::new(m[3][0], m[3][1], m[3][2]));
                let ent = &mut self.prop_held[e];
                let was = ent.scrape;
                if coll.prop_deflect(lib, &obj, dfl.speed, at, if dfl.player { 0 } else { occ }, &mut ent.scrape) {
                    ent.scrape_age = ent.age;
                    // (a loop it started: request +172 = 1)
                    if let Some(h) = ent.scrape.filter(|h| Some(*h) != was) {
                        ent.follow.push(h);
                    }
                }
            }
            // the whooshes (F-12768): the player's car past each prop within the scan's reach; on a hit the gate (F-67165)
            // binds the prop's entity (only when its type has a whoosh, [81] != NULL_SOUND; an existing entity always),
            // then the whoosh (F-28919): +132 < [68], age >= +112 + 2 s, the one-shot [81] at the prop, +112 = age
            if let (Some(w), Some(p)) = (lib.game.get_name("GLOBAL_WHOOSHES").map(|o| o.to_vec()), vehicles.cars.iter().find(|c| c.is_player)) {
                let null = mcla_formats::hash::joaat("NULL_SOUND");
                for np in self.near_props.iter() {
                    let Some((vol, pitch)) = crate::collaudio::prop_whoosh(&w, p.state.pos, p.state.vel, np.pos, np.vel, p.input.throttle) else { continue };
                    let key = key_of(np.inst, np.model, np.pos);
                    let obj = lib.game.get(np.model).filter(|o| o.first() == Some(&43)).or_else(|| lib.game.get_name("LA_PROP_DEFAULT")).map(|o| o.to_vec());
                    let Some(obj) = obj else { continue };
                    let snd = obj.get(81..85).map_or(null, |b| u32::from_be_bytes(b.try_into().unwrap()));
                    if !self.prop_held.iter().any(|e| e.key == key) && snd == null {
                        continue;
                    }
                    let (e, occ) = bind(&mut self.occl, lib, &mut self.prop_held, np.inst, np.model, np.pos);
                    let ent = &mut self.prop_held[e];
                    let live = ent.sounds.iter().filter(|h| lib.playing(**h)).count() + ent.scrape.is_some_and(|h| lib.playing(h)) as usize;
                    if ent.age < ent.whoosh + 2.0 || live >= obj.get(68).copied().unwrap_or(0) as usize {
                        continue;
                    }
                    let at = prop_matrix(np.inst).map_or(np.pos, |m| Vec3::new(m[3][0], m[3][1], m[3][2]));
                    if let Some(h) = lib.play_hash(snd, crate::sndobj::Req { pos: Some(at), volume: vol, pitch: pitch as i32, occluder: occ, ..Default::default() }) {
                        ent.sounds.push(h);
                    }
                    ent.whoosh = ent.age;
                }
            }
            // F-20017, every frame after the occluders' update (above, `occlude`): age += dt, the instance's matrix again
            // (so the occluder of a knocked prop follows it a frame behind: recording q2, the emitter of an update in
            // frame F is the origin the entity took in frame F - 1 plus the type's place), the scrape loop stopped once
            // the age is 0.1 s past its last update
            for ent in self.prop_held.iter_mut() {
                ent.age += dt;
                if let (Some(oc), Some(m)) = (&mut self.occl, prop_matrix(ent.inst)) {
                    oc.prop_follow(ent.key, m);
                }
                // the entity's voice pass (vt slot 5, F-34529): with a prop instance (+96), each live voice whose request
                // +172 is set goes to the entity's place (+64, its matrix just refreshed)
                ent.follow.retain(|h| lib.playing(*h));
                if let Some(m) = prop_matrix(ent.inst) {
                    let at = Vec3::new(m[3][0], m[3][1], m[3][2]);
                    for h in &ent.follow {
                        lib.set_pos(*h, at);
                    }
                }
                if let Some(h) = ent.scrape {
                    if ent.age >= ent.scrape_age + 0.1 {
                        lib.stop(h);
                        ent.scrape = None;
                    }
                }
            }
            self.prop_held.retain_mut(|ent| {
                ent.sounds.retain(|h| lib.playing(*h));
                // (and 2 s past its last whoosh, F-20017's release rule)
                let on = !ent.sounds.is_empty() || ent.scrape.is_some_and(|h| lib.playing(h)) || ent.age < ent.whoosh + 2.0;
                if !on {
                    if let Some(oc) = &mut self.occl {
                        oc.release_prop(ent.key);
                    }
                }
                on
            });
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
            if self.lib.is_some() {
                // GARAGE_MONEY_MASTER at a purchase or sale in the garage
                if game.money != self.money && game.mode == Mode::Garage {
                    self.ui(a, "GARAGE_MONEY_MASTER", "", 0.0);
                }
            } else if game.money < self.money && in_menu {
                a.play(&format!("{MENUS}/ui_menu_select_l"), 0.7);
            }
        }
        // (without the sound library nothing plays them)
        self.movie_sounds.clear();
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
        // the race's end: the player's voice when its end sequence runs (`Game::end_race_speech`), then the ENDRACE
        // mixer, whose own sound is the sting (AudioManager_SetActiveEndRace)
        let ended = game.end_seq_n != self.end_race;
        if let Some(lib) = &mut self.lib {
            if ended {
                if let Some(g) = game.end_race_speech() {
                    lib.play(&format!("PLAYER_ENDRACE_GESTURE_{g}_MASTER"), crate::sndobj::Req::default());
                }
            }
            lib.scene("DYNAMIC_MIXER_ENDRACE", game.race_end_t.is_some_and(|t| t >= 0.2));
        }
        self.end_race = game.end_seq_n;

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
        // the mixer's speed inputs (F-99947 every frame: chassis +224 x 2.237 mph and the rpm fraction; 0 / 0 in the pause
        // branch of F-99037, taken as the pause as for the speed fade)
        if let Some(lib) = &mut self.lib {
            let paused = game.mode == Mode::Paused;
            lib.set_speed_inputs(if paused { 0.0 } else { speed * f32::from_bits(0x400F_2B02) }, if paused { 0.0 } else { crate::caraudio::mixer_rpm_frac(p) });
        }
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
                self.engine = crate::engaudio::EngineSound::new(lib, &name, p.model.bike).map(|mut e| {
                    e.start_at(crate::caraudio::engine_rpm(p));
                    (name.clone(), e)
                });
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
                revs: false,
                racing: false,
                speed_ratio: 0.0,
                rubber_hold: 0.0,
            };
            eng.occ = self.occ_slots.get(&p.uid).copied().unwrap_or(0);
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
            lib.scene("DYNAMIC_MIXER_POLICE_SCRIPT_TRIGGERED", self.arrest_mix.0 || self.pullover_mix);
            lib.scene("DYNAMIC_MIXER_MUTE_SHENANIGANS", self.arrest_mix.1 || self.story_mix.0);
            lib.scene("DYNAMIC_MIXER_HENRY_DRIVEBY", self.story_mix.1);
            lib.scene("DYNAMIC_MIXER_SLOWMO_FORM", self.slowmo_form);
            let press = p.input.nitro && !self.nitro_btn;
            let ctl = crate::caraudio::PlayerCtl {
                horn: horn && !in_menu,
                police_class: game.oppick.class_of(&p.model.name) == Some(crate::oppick::CLASS_COP),
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
                siren: crate::caraudio::SirenTune::of(&police.tun),
            };
            fleet.update(lib, vehicles, cam_pos, ctl, wet, game.mode == Mode::Race, pol, dt);
            // the ambient cars' occluder switches of their updates (F-70175, F-74101), their outputs again to the voices
            if let Some(oc) = &mut self.occl {
                let sw = fleet.traffic_occ();
                let factors = oc.factors.clone();
                switch_ambient(oc, lib, &sw, &mut crate::occlusion::WorldProbe::of(collider, &factors));
            }
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
            // the reverb level and room size and the forced district (F-26357, BUS_GRAPH_RULE.md 2.5): a hit 50 m up
            // from the listener -> REVERB_PROBE_UP; a hit 50 m down -> its material's audio object (MATERIALS_*): +230 bit 0
            // (with no hit above, or with +230 bit 1 too) names the reverb object +231 (REVERB_DEFAULT on the outdoor
            // grounds, REVERB_TUNNEL in tunnels and sewers), +235 bit 0 the forced district +236 (freeway, interior, mall,
            // tunnel); no hit at all -> REVERB_PROBE_AIR; otherwise the object stays. Level and size move toward the
            // object's +10 / +14 by its rate +18 a second.
            let mut forced = None;
            {
                let h = mcla_formats::hash::joaat;
                let mut obj = if probe(cam_pos, cam_pos + Vec3::new(0.0, 50.0, 0.0)) { h("REVERB_PROBE_UP") } else { 0 };
                let mut any = obj != 0;
                if let Some(surface) = probe_at(cam_pos, cam_pos - Vec3::new(0.0, 50.0, 0.0)) {
                    let mat = self.fleet.as_ref().map(|f| f.material_object(surface)).and_then(|m| lib.game.get(m));
                    if let Some(m) = mat {
                        let word = |at: usize| m.get(at..at + 4).map_or(0, |b| u32::from_be_bytes(b.try_into().unwrap()));
                        let flags = m.get(230).copied().unwrap_or(0);
                        if (obj == 0 || flags & 2 != 0) && flags & 1 != 0 && lib.game.contains(word(231)) {
                            obj = word(231);
                        }
                        if m.get(235).map_or(false, |b| b & 1 != 0) && lib.game.contains(word(236)) {
                            forced = Some(word(236));
                        }
                    }
                    any = true;
                }
                let cur = self.reverb_obj.unwrap_or(h("REVERB_DEFAULT"));
                let obj = if obj != 0 { obj } else if !any { h("REVERB_PROBE_AIR") } else { cur };
                if self.reverb_obj != Some(obj) || self.forced_district != forced {
                    log::info!("audio: reverb {} district {}", lib.game.name_of(obj).unwrap_or("?"), forced.and_then(|d| lib.game.name_of(d)).unwrap_or("-"));
                }
                self.reverb_obj = Some(obj);
                self.forced_district = forced;
                if let Some(o) = lib.game.get(obj) {
                    let f = |at: usize| o.get(at..at + 4).map_or(0.0, |b| f32::from_be_bytes(b.try_into().unwrap()));
                    let (level, size, rate) = (f(10), f(14), f(18));
                    let step = |v: &mut f32, t: f32| {
                        if *v < t {
                            *v = rate.mul_add(dt, *v).min(t);
                        } else if *v > t {
                            *v = (-rate).mul_add(dt, *v).max(t);
                        }
                    };
                    let mut env = lib.reverb_env;
                    step(&mut env.0, level);
                    step(&mut env.1, size);
                    lib.reverb_env = env;
                }
            }
            // the hangout cores' audio natives (AudioManager_InitHangout / _SuspendHangout / _DestroyHangout with the core,
            // _EnterHangoutState / _ExitHangoutState with the pull-in - `Game::hangout_audio`)
            let cores: Vec<(&str, Vec3, bool)> = crate::hangoutin::SETUPS.iter().filter(|s| game.mc4_hangout_core_running(s.key)).map(|s| (s.key, s.line[1], game.place_hold.contains_key(&s.uid))).collect();
            amb.hangouts(lib, &cores, game.hangout_audio.map(|(si, _)| crate::hangoutin::SETUPS[si].key));
            let mph = if game.mode == Mode::Paused { 0.0 } else { speed * 2.237 };
            // mgr +1079 (F-37136 clears it at a scene's start, F-65984 sets it at its finish)
            amb.positional = !self.scene;
            let factors = self.occl.as_ref().map(|o| o.factors.clone()).unwrap_or_default();
            let l = lib.listener.0;
            match &mut self.occl {
                Some(oc) => {
                    let mut so = AmbOccluders { oc, world: crate::occlusion::WorldProbe::of(collider, &factors), l: [l.x, l.y, l.z, 0.0] };
                    amb.update(lib, cam_pos, night, game.mode == Mode::Garage, forced, mph, game.mode == Mode::Race, dt, &mut so);
                    for k in 0..2 {
                        if let Some(i) = so.oc.amb_place_slot(k) {
                            if let Some(o) = so.oc.out(i) {
                                lib.set_occlusion(i, o);
                            }
                        }
                    }
                }
                None => amb.update(lib, cam_pos, night, game.mode == Mode::Garage, forced, mph, game.mode == Mode::Race, dt, &mut crate::ambaudio::NoOccluders),
            }
            // rain and thunder (WEATHER_AUDIO_RULE.md): the player's wheel cover probes, the listener's probe 50 m up
            let wheels: Vec<Vec3> = if p.model.bike { vec![st.wheels[0].world_pos, st.wheels[2].world_pos] } else { [0, 2, 1, 3].iter().map(|&i| st.wheels[i].world_pos).collect() };
            amb.cover_probe(wet, &wheels, probe);
            let covered = wet && probe(cam_pos, cam_pos + Vec3::new(0.0, 50.0, 0.0));
            amb.weather(lib, wet, self.cam_audio == 1, covered, dt);
            // the speed fade: the player's speed in mph as F-99947 keeps it (chassis +224 x 2.237); held at 0 by F-99037
            // in a branch of its own taken while the game is held (taken here as the pause: which states exactly is not decoded)
            amb.speed_fade(lib, if game.mode == Mode::Paused { 0.0 } else { speed * 2.237 });
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
