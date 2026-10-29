//! Midnight Club: Los Angeles – Rust runtime.


mod ability;
mod boot;
mod bootload;
mod figlight;
mod hooks;
mod mc4;
mod save_gaps;
mod pads;
mod rewards;
mod vinyl;
#[cfg(test)]
mod carparts_small_tests;
mod ai;
mod ambient;
mod assets;
mod audio;
mod boomcam;
mod camera;
mod citymap;
mod career;
mod catalog;
mod modcars;
mod modfs;
mod modgarage;
mod garagecam;
mod garagein;
mod hangoutin;
mod radio;
mod offroad;
mod occluder;
mod racescript;
mod checkpoints;
mod powerups;
mod pu_draw;
mod pu_fx;
mod sst_trail;
mod pu_bolt;
mod redlight;
mod althk;
mod flags;
mod fixed_step;
mod vehicle_pose;
mod video;
mod cine;
mod cinecar;
mod cinedriver;
mod damageparts;
mod damagemap;
mod cutscene;
mod drivers;
mod game;
mod glyphs;
mod keybinds;
#[cfg(test)]
mod controls_ref;
mod hud;
mod flash_ui;
mod selfmovie;
mod flashpic;
mod minimap_gpu;
mod gpuprof;
mod passprof;
mod uipass;
mod aamode;
mod particles;
mod propfx;
mod modelfx;
mod tyrefx;
mod carfx;
mod part_audit;
mod rain;
mod halo;
mod siglrefl;
mod sunglow;
mod ticktimes;
mod carblob;
mod carshadow;
mod peds;
mod physics;
mod police;
mod props;
mod races;
mod replay;
mod roamers;
mod signals;
mod skin;
mod sky;
mod sound;
mod sndobj;
mod engaudio;
mod collaudio;
mod ambaudio;
mod caraudio;
mod speech;
mod render;
mod rimshop;
mod dimensions;
mod menu3d;
mod prerace;
mod racemark;
mod mapmesh;
mod oppick;
mod hudmap;
mod hudmap_gpu;
mod hookicons;
mod ground_sheet;
mod fillmap;
mod rearview;
mod floormirror;
mod titlecam;
mod startwarp;

/// Where the pause cameras' frame sits in the car (x right, y up, z rear from the
/// car's origin). Measured in the original (CAREER_RACES.md "Race end: camera and
/// timing", second run): the settled PauseCamera3 and PauseCamera2 both stand at their
/// tuned place + (0, 1.42, 0.08) from the S281's instance origin, the look the tune's to
/// 0.003. (approx.: the port's car origin taken as the original's instance origin; was
/// (0, 1.38, -0.25), fitted by eye)
/// VEHICLE SELECT's music (CarSelection 0xedb: mcMusicManager_PlayCustomMusic('MUSIC_CARSELECT_MASTER'))
const CARSELECT_MUSIC: &str = "MUSIC_CARSELECT_MASTER";
/// INTRO FAST (DECISIONS.md D24): the film `intro720.bik` (68.47 s) ends with the arrival CUT_INTRO shows (10.3 s), the
/// two ending together - the film's time of the scene's first frame (the close-up's cut: film 66.30 s, scene 8.1 s;
/// CUTSCENES.md "The opening scenes' audio (session `cut-audio`, 2026-10-23)")
const FILM_CUT_INTRO_AT: f32 = 58.2;
const PAUSE_FRAME: Vec3 = Vec3::new(0.0, 1.42, 0.08);
mod rivals;
mod rubber;
mod rumble;
mod aiv;
mod brain;
mod brainpath;
mod brainturn;
mod vehicle_model;
mod vehicles;
mod hangout_scene;
mod layout;
mod mc4b;
mod photoalbum;
mod saveflow;
mod world;

use anyhow::Result;
use assets::Assets;
use camera::{Camera, ChaseCam};
use glam::Vec3;
use physics::CarInput;
use render::{Frustum, Globals, Gpu, MatMode};
use std::sync::Arc;
use std::time::Instant;
use vehicles::Vehicles;
use game::{Game, HudInfo, MenuAction, Mode};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};
use world::World;

#[derive(Default)]
struct Input {
    keys: hashbrown::HashSet<KeyCode>,
    pending_keys: hashbrown::HashSet<KeyCode>,
    mouse_dx: f32,
    mouse_dy: f32,
    rmb: bool,
}

impl Input {
    fn down(&self, k: KeyCode) -> bool {
        self.keys.contains(&k) || self.pending_keys.contains(&k)
    }
}

struct Egui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    /// the menus' 4x UI pass (uipass.rs) and its renderer: 4 samples, no depth attachment
    renderer4: egui_wgpu::Renderer,
    ui4: Option<uipass::Ui4>,
    /// the same pass at 2 samples (ANTI-ALIASING 2X, aamode.rs), drawn with `renderer2`
    ui2: Option<uipass::Ui4>,
    /// the self movies' pictures (selfmovie.rs): 2 samples, 960 x 640
    renderer2: egui_wgpu::Renderer,
    selfs: std::sync::Arc<std::sync::Mutex<selfmovie::Shared>>,
}

#[derive(Clone, Debug)]
struct ShotCfg {
    path: String,
    frames: u64,
}

#[derive(Default, Clone, Debug)]
struct Options {
    data_dir: String,
    /// `--save-dir`: where the save lives (default: see `game::savedir`)
    save_dir: Option<String>,
    /// `--settings <file>`: the settings file (default: see `hooks`)
    settings_file: Option<String>,
    shot: Option<ShotCfg>,
    pos: Option<Vec3>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    time: Option<f32>,
    only: Option<String>,
    hide: Option<String>,
    car_dir: Option<String>,
    drive: bool,
    auto: bool,
    traffic: usize,
    fly: bool,
    menu: bool,
    race: bool,
    race_name: Option<String>,
    no_props: bool,
    audio_dump: Option<String>,
    no_post: bool,
    find_straight: bool,
    map: bool,
    weather: Option<sky::Weather>,
    aim_prop: Option<String>,
    no_shadows: bool,
    race_select: bool,
    /// fixed simulation step for headless runs (0 = real time)
    fixed_dt: f32,
    garage: bool,
    wot: bool,
    /// initial garage camera angle (radians)
    orbit: f32,
}

struct App {
    opt: Options,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    world: Option<World>,
    egui: Option<Egui>,
    cam: Camera,
    chase: ChaseCam,
    orbit: f32,
    sim_time: f32,
    tick_no: u64,
    /// frames the race-over menu has offered RACE BACK (test hooks, althk.rs)
    alt_menu_frames: u32,
    /// the freeway men's scripts have made the police units (althk.rs)
    alt_units_made: bool,
    sim_clock: fixed_step::Clock,
    /// Test-only render delta; still exercises the ordinary accumulator path.
    test_render_dt: Option<f64>,
    input: Input,
    last: Instant,
    fps: f32,
    frame_ms: f32,
    // (MCLA_FRAMELOG=<ms>: the worst-frame log - docs/PERFORMANCE.md)
    framelog: Option<f32>,
    fl_prev: [f32; 9],
    fl_ui: [f32; 5],
    fl_stream_ms: f32,
    draws: usize,
    time_of_day: f32,
    sky: sky::Sky,
    weather: sky::Weather,
    /// weather being blended in: (sky, weather, progress 0..1)
    sky_next: Option<(sky::Sky, sky::Weather, f32)>,
    /// The shadow map object's slice and angle scales (+160 / +164 of F-87623's object), eased each
    /// frame by F-55997 towards (0.3, 0.8) - the shadow state's +100 / +104 (F-28724) - while the
    /// garage's lighting is on (renderer +390 = the light manager's +14546), else towards (1, 1):
    /// a step of max(|d|, 0.2) x rate 1.0 x the frame's unscaled time.
    casc_scale: [f32; 2],
    /// the weather's change (F-18159): seconds of game time to the next draw (0x8287E12C,
    /// 0 at boot), the auto change (0x8287E138: off until hw_local_hangout2_setup runs), the
    /// draws (the shared MWC, 0x82860890 - the port's own instance) and a scene seen last frame
    weather_timer: f32,
    weather_auto: bool,
    weather_rng: vehicles::parked::Mwc,
    weather_scene: bool,
    cycle_weather: bool,
    props: props::Props,
    sky_bind: Option<Arc<wgpu::BindGroup>>,
    show_ui: bool,
    frame_no: u64,
    vehicles: Vehicles,
    drive: bool,
    /// SDL's gamepad layer (`pads.rs`); none when it did not start
    gamepad: Option<pads::Pads>,
    /// the pad being played with: the last one whose button or stick was used (Windows can
    /// list several - one controller as XInput and as a raw HID device, a dongle's spare slots)
    pad_id: Option<pads::PadId>,
    /// the pad's rumble: the player car's feedback object (`rumble`), made on first use
    pad_rumble: Option<rumble::PadRumble>,
    /// when each pad was last used (a button, or a stick or trigger past half way)
    pad_used: std::collections::HashMap<pads::PadId, std::time::Instant>,
    game: Game,
    /// the race checkpoints' looks and which are open (`checkpoints.rs`)
    cp_markers: checkpoints::Markers,
    quit: bool,
    pending_action: MenuAction,
    audio: Option<audio::Audio>,
    sound: sound::Sound,
    police: police::Police,
    fx: particles::Particles,
    /// the tyres' effects of the cars on the original's simulation (tireptx)
    tyre_fx: Option<tyrefx::TyreFx>,
    /// the effects the props' events start (propfx.rs)
    propfx: propfx::PropFx,
    peds: peds::Peds,
    roam: roamers::Roamers,
    /// where the car was before it was put into a garage (position, yaw)
    garage_return: Option<(Vec3, f32)>,
    /// free radius around the car inside the garage
    garage_clear: f32,
    /// the garage camera views of the car on the floor (its name, the views) and where the camera is between them
    garage_views: Option<(String, garagecam::Views)>,
    garage_cam: Option<(Vec3, Vec3, f32)>,
    /// the garage camera's move between views: where it came from, where it goes, the
    /// seconds gone and the up vector of this frame
    garage_tr: Option<(garagecam::CamState, garagecam::CamState, f32)>,
    /// the cine scene whose lamp events run (vehicles.rs `CINE_LAMP_EVENTS`): its name, the clock already
    /// passed, every car's lamp switch as the scene found it (uid, off)
    cine_lamps: Option<(String, f32, Vec<(u32, bool)>)>,
    /// the cars a showdriver event of the playing scene has shown (uid; cinedriver.rs)
    cine_showdriver: Vec<u32>,
    /// the race's end: the standings form's pause camera (picked when it comes up) and the
    /// view held once the game is paused
    race_end_cam: Option<(menu3d::PauseCam, menu3d::PauseCam)>,
    race_end_from: Option<(Vec3, Vec3, f32)>,
    race_end_way: Option<f32>,
    /// the parts the car in the garage was last built with: a change shows at once
    garage_built: Option<(usize, vehicle_model::CarConfig, u8)>,
    garage_up: Vec3,
    garage_spots: hashbrown::HashMap<usize, Option<(Vec3, f32)>>,
    /// where the car stands inside each garage (from the garage scenes)
    garage_poses: Option<Vec<(Vec3, glam::Quat)>>,
    news: std::collections::VecDeque<(String, Option<String>)>,
    speech: speech::Speech,
    radio: radio::Radio,
    /// where the dispatcher reports reckless driving (centre, radius, call), and the pause between reports
    dispatch_zones: Vec<racescript::GlobalDispatch>,
    /// the player's place last frame (the global triggers' line test)
    dispatch_prev: Option<Vec3>,
    /// seconds until the next bit of pursuit chatter
    radio_next: f32,
    /// vinyl shapes (loaded when a livery is first painted) and the cars' zone layouts
    shape_lib: Option<vinyl::ShapeLib>,
    car_zones: hashbrown::HashMap<String, Option<Arc<vinyl::CarZones>>>,
    cuts: cutscene::Cutscenes,
    cine: cine::Cine,
    /// a scene is waiting for its surroundings to load: (dialogue stream, frames waited)
    cine_hold: Option<(Option<String>, u32)>,
    /// the story scene's audio was started through the sound objects (`start_scene_audio`)
    cut_audio_objects: bool,
    /// the in-game scene whose audio stream plays (`cine::scene_audio`)
    cine_audio: Option<String>,
    /// the running scene keeps its own time (start-line cameras) instead of a story scene's
    cine_free: bool,
    /// the arrest scene's hands-up has been given (police::arrest_hands_up)
    arrest_hands: bool,
    /// the pull-over scene playing and how many of its acts (police::pullover_acts) have been given
    pullover_acts: (String, usize),
    /// the arrest's form waits for its scene to end: (the pursuit's length, the fine)
    arrest_form: Option<(f32, i64)>,
    /// the ARRESTED form is up: when it closes CopArrest.sco puts the car at the police station (frames left to wait
    /// for its ground)
    arrest_station: Option<u32>,
    /// CopArrest.sco 0x6e4 AmbientMgr_SetTrafficEnable(0) for the scene: the traffic target to give back after it
    arrest_traffic: Option<usize>,
    /// CopArrest.sco's TRANSITIONMOVIE black: 1 s down before the scene (waited), up 1 s at its start, the scene's
    /// fade_to_black, held over the form, up 1 s after the station reset (waited)
    arrest_mask: boomcam::CutMask,
    /// the arrest's scene starts once the black is full
    arrest_wait: bool,
    /// the running arrest scene's fade_to_black start (scene seconds)
    arrest_fade: Option<f32>,
    /// the station reset is done: the car gets control once the fade up has ended
    arrest_activate: bool,
    /// the police's game-frame clock (brain::frame_due)
    police_acc: f32,
    /// CopPullover.sco running: (the fine, the walk's time of a bolt, the ticket scene begun)
    ticket_run: Option<(i64, Option<f32>, bool)>,
    /// the playing scene's car events (cinecar.rs): the scene's name and each moved car's event
    cine_events: (String, Vec<(String, cinecar::CarEvent)>),
    /// tips waiting for a quiet moment: speech scenario, title, text
    tips: std::collections::VecDeque<(String, String, String)>,
    /// a mission's reminder (caller, text) waiting for its story scene to end
    pending_call: Option<(String, String)>,
    /// a mission whose READY_TO_START_ON_ENTER script plays its story scene first: it enters
    /// the log (mcMission_ChangeState(3)) once the scene is over - RACE_GH_06 0xadb
    pending_log: Option<String>,
    /// whoosh lines of the city, by 128 m cell; where the player was last frame
    whoosh: hashbrown::HashMap<(i32, i32), Vec<(Vec3, Vec3)>>,
    whoosh_prev: Vec3,
    whoosh_cool: f32,
    whooshes_heard: u32,
    /// the hidden collectable barrels (`script_obj/game/MiscTasks/Collectables.sco`)
    barrels: Vec<Vec3>,
    /// the placed props have to be rebuilt (a race began or ended, a barrel was taken)
    props_dirty: bool,
    /// what the running race's script put on the road
    dress: Option<racescript::Runtime>,
    /// step 19e: the race's power-ups (an ARCADE race with POWER UPS on)
    powerups: Option<powerups::PowerUps>,
    redlights: redlight::RedLights,
    /// start exactly here (the city's default start) instead of on the nearest road
    exact_start: Option<(Vec3, f32)>,
    /// the free-running scene is a start-line fly-by (holds the countdown, ends with the race)
    cine_prerace: bool,
    /// what to do when the free-running scene is over
    cine_after: Option<MenuAction>,
    /// hangouts: (scene name, where one pulls in)
    hangouts: Vec<(String, Vec3)>,
    /// the hangout the player is at (no new pull-in scene until he has left)
    hangout_in: Option<usize>,
    mode_before: Mode,
    /// the free-running scene belongs to the results screen and ends with it
    cine_results: bool,
    /// a tournament attempt's field (its event, the opponents): TournamentCore creates them once
    /// (0x20d..0x4cf) and `nextRace` / `restart` keep them; a new attempt draws again
    tour_field: Option<(String, Vec<rivals::Rival>)>,
    /// line shown in the letterbox of a free-running scene
    cine_caption: Option<String>,
    /// test hook MCLA_SHOT_AT_GO: taken
    shot_at_go: bool,
    /// the original's start-scene picker (its shuffle bags last the session)
    prerace_picker: Option<prerace::Picker>,
    /// the start scene's events on the racers (prerace.rs `slot_events`): its script runs to the clip's end
    /// after the cut to the game camera
    prerace_acts: Option<prerace::Acts>,
    /// the scene the last start played, and a RESTART asked: the original replays it without a new pick (no
    /// bag cursor moves over six restarts of three races, work/career2/cap/README.md)
    prerace_last: Option<(Option<usize>, String)>,
    prerace_replay: bool,
    /// seconds without input on the main menu (the attract movie starts after a while)
    idle: f32,
    /// start-line camera waiting for the grid's surroundings: frames waited
    prerace_wait: u32,
    /// the grid the opening shot comes down on (its front, its heading)
    /// the race-start warp (hudmap::CineWarp), its elapsed time, its view (eye, look, up, fov radians)
    warp: Option<hudmap::CineWarp>,
    warp_t: f32,
    warp_view: Option<(Vec3, Vec3, Vec3, f32)>,
    /// Warper_Warp('DEFAULT_WITH_CUT')'s climb (hudmap::CineClimb) and the action it holds until
    /// it is over (IsReadyToLoadStuff); its view (eye, look, up, fov radians), kept at its end until
    /// the dive takes over; the climb's last camera and lens (the dive's start); frames held
    climb: Option<(hudmap::CineClimb, MenuAction)>,
    climb_view: Option<(Vec3, Vec3, Vec3, f32)>,
    climb_end: Option<(hudmap::CamRows, f32)>,
    climb_hold: u32,
    /// the climb is over and its camera held: the time left until the race is set up and the dive
    /// starts, and the action held until then
    climb_wait: Option<(f32, MenuAction)>,
    /// the camera on screen last frame and its velocity (playerMgr+0x51420: the step over the unscaled
    /// frame time, `F-31556`) - what a climb is handed
    screen_eye: Vec3,
    screen_v: Vec3,
    /// the start scene chosen when the pre-race begins (the warp lands on its camera)
    prerace_plan: Option<(Vec<String>, glam::Mat4)>,
    starter_was: bool,
    /// the delivery's place opens its panel (again once the car has been away)
    delivery_armed: bool,
    /// The delivery core's LOSE_COPS count (100 polls of 0.1 s between splashes), seconds left.
    delivery_cops: f32,
    /// The delivery's ease to pause under way: (seconds, its hangout).
    delivery_ease: Option<(f32, usize)>,
    /// The delivery's visit: 1 its panel is up, 2 answered (the police come up once back in the cruise).
    delivery_visit: u8,
    /// (p080) a delivery's intro still to come after the walk to the car: (scene, grid slot 0, yaw)
    delivery_intro: Option<(String, Vec3, f32)>,
    /// (p080) a delivery intro after its cut to the game camera: (scene, scene time) - its map and
    /// countdown events go on (the scene script runs to its end in the original)
    delivery_tail: Option<(String, f32)>,
    /// the garage's door takes the car in (not straight after leaving it)
    /// driving into a garage: the warp to its sign, the scene, the fade up (`garagein`)
    garage_in: Option<garagein::Entry>,
    /// GO TO GARAGE's garage (state 4): garage_core's next poll in Cruising_Offline goes straight to its drive-in
    goto_garage: Option<usize>,
    /// pulling into a hangout: the ease to pause, the warp to its wait camera (`hangoutin`)
    hangout_entry: Option<hangoutin::Entry>,
    /// the place's band of a hangout entry: the clock from the warp's start
    hangout_title: Option<(usize, f32)>,
    /// the player's position a frame before (the entry lines' move: racer+400 -> the matrix)
    hangout_prev: Option<Vec3>,
    /// the camera of that warp and the wait after it (eye, look point, up, lens in radians)
    place_view: Option<(Vec3, Vec3, Vec3, f32)>,
    /// ACCEPT on the payback's panel (form 68): the warp to the place's wait camera - (time, the warp, where
    /// the car is held); then `payback_hold` until `cut_payback_get_keys` starts
    payback_in: Option<(f32, Option<hudmap::CineWarp>, (Vec3, glam::Quat))>,
    payback_hold: bool,
    /// its black (the scene's fade_to_black, garage_core's fade up)
    garage_mask: boomcam::CutMask,
    /// leaving the garage for the street: seconds into the fade to black before the drive-out scene
    garage_out: Option<f32>,
    /// seconds of cruising until a racer rings with a challenge (`game/race/TelephoneChallenge`)
    phone_challenge_in: f32,
    /// the racer whose telephone challenge is up (GH_03 ..): his voice says the invite and the decline
    phone_caller: &'static str,
    /// the racer's countdown has run out: his call waits for the phone's gates
    phone_challenge_due: bool,
    /// the phone form's +788 (held: sidekickDelay, after each call) and the delayed sidekickResume that clears it
    /// (Cruising.sc.xml: 8 s after entering Cruising_Offline; after a call SidekickDelayTime)
    phone_hold: bool,
    phone_resume: Option<f32>,
    /// in Cruising_Offline last tick; a call was on the phone last tick; a call went away and its close (F-37697's
    /// tail: the hold and the resume) waits for the gates
    phone_cruise: bool,
    phone_was_up: bool,
    phone_done: bool,
    /// UILogic's SidekickDelayTime (read when first needed)
    phone_delay: Option<f32>,
    /// a payback call lifted the hold with Sidekick_Delay(0): Sidekick_Delay(1) follows once it is announced
    phone_redelay: bool,
    /// a new career's opening (the film, the arrival) has been started - before VEHICLE SELECT
    intro_played: bool,
    /// the opening's step: 1 fading to black, 2 the film and the wait for the city, 3 VEHICLE SELECT, 4 black
    /// until the first story scene; 0 none (and `intro_t`, seconds in the step)
    intro_stage: u8,
    intro_t: f32,
    /// MCLA_AUDIO_TRACE: seconds since the last line of the audio trace
    audio_trace_t: f32,
    /// the opening's black (CarSelection's TRANSITIONMOVIE fades)
    intro_mask: boomcam::CutMask,
    /// the opening's `Graphics_WarpToTimeOfDay(17, 100)`: the hour aimed at, while the clock is moved there
    intro_tod: Option<f32>,
    /// VEHICLE SELECT: where the first of the three starters stands
    starters_made: bool,
    /// the pending race whose guessed grid was read ahead (`grid_ahead`)
    grid_warm: Option<(usize, bool)>,
    /// a new career's opening read ahead on worker threads (`opening_read_ahead`): 1 the three starters, 2 the
    /// first story scene's cars and people
    opening_read: u8,
    /// where the player's hidden car is held through VEHICLE SELECT (`place_player_at_kerb`)
    starter_stand: Option<(Vec3, glam::Quat)>,
    /// VEHICLE SELECT's camera (CarSelection 0x1069): eye, look, their speeds, the time to the next 30 ms step
    sel_cam: Option<(Vec3, Vec3, f32, f32, f32)>,
    settle_frames: u32,
    news_timer: f32,
    /// photo mode: (mode to return to, was driving), picture to write this frame, pictures taken
    photo: Option<(Mode, bool)>,
    photo_request: Option<String>,
    photo_msg: f32,
    /// the car is wrecked: seconds until it is put back on the road (the port's repair; it counts
    /// from the end of the damage-out timer)
    wreck_timer: f32,
    /// the driver's damage-out states for the engine sound: 1 the stop due at "impact start car", 2 stopped (the
    /// start due at the exit of "impact start car done"), 3 in "impact start car done" with no pause coming (its
    /// time in `wreck_done`), 0 none
    wreck_engine: u8,
    wreck_done: f32,
    /// the game's time scale from the time warpers (min of them; `boomcam::TimeWarper`):
    /// "vehicle\wipeout" (`wipeout.timewarper`) and "vehicle\explosion"
    wipe_scale: f32,
    warp_wipe: boomcam::TimeWarper,
    warp_expl: boomcam::TimeWarper,
    /// the look-back slow motion in the air ("vehicle\lookback", F-38892) and whether this side
    /// asked for its mixer scene (mgr +1740 bit 5)
    warp_look: boomcam::TimeWarper,
    look_scene: bool,
    /// the ability camera (mcPlayerCamera +68)
    ability_cam: boomcam::AbilityCam,
    /// the player car's crash state (chassis +592: 0x10 wiped out, 0x04 damaged out; crash_camera_rule.md
    /// 4): the bits, mcCarDamage +44 (the timer), an exploding car (+82), the speed and velocity
    /// of the frame before
    crash_bits: u8,
    crash_timer: f32,
    crash_exploding: bool,
    /// the wipeout camera (mcPlayerCamera +72)
    crash_cam: boomcam::CrashCam,
    /// standing still: seconds so far, the car's "Idle" view once looked up, how far it is blended in
    idle_time: f32,
    /// the idle camera: the replay's views of the standing car
    /// the idle camera: the car's replay files and the mount held
    /// the player's drive view in use this frame (`cam_mode`, or the replay director's;
    /// 255 when another camera is on)
    view_mode: u8,
    /// the replay's clock at the last frame (test hook MCLA_REPLAY_SHOTS)
    replay_shot_t: f32,
    /// the race's start (`Game::prerace`) kept for its replay's opening
    replay_prerace: Option<(Vec3, f32, String)>,
    /// seconds since VIEW REPLAY (test hook MCLA_REPLAY_SHOTS)
    replay_wall: f32,
    /// the camera shake (boomcam::CamShake, camFX's blocks), the last impact seen, the time in
    /// the air, a wipeout already shaken for
    shake: Option<boomcam::CamShake>,
    shake_impact: f32,
    shake_air: f32,
    /// the wipeout camera's start asks for its jolt (F-37740)
    shake_wipe: bool,
    /// the landing shake held while a wheel is bottomed out (car +473), the slipstream turbo
    /// running last frame, and one-shots asked for outside the camera code (duration, scale)
    shake_land: bool,
    shake_sst: bool,
    /// the player's SsTurbo::fires last frame (stats gSST / nSST)
    sst_fires: u32,
    /// the player's VehState::nitro_fires last frame (stats gNIT / nNIT)
    nitro_fires: u32,
    shake_jolts: Vec<(f32, f32)>,
    district_timer: f32,
    /// the GPS target last routed (place bits, mission): a change re-makes the route at once
    gps_key: (Option<[u32; 3]>, Option<String>),
    flash: bool,
    enter_garage: bool,
    /// 0 chase, 1 hood, 2 cockpit
    cam_mode: u8,
    /// the driving cameras of each car (tune/camera) and the camera in use
    booms: std::collections::HashMap<String, Option<Arc<boomcam::BoomSet>>>,
    boom: boomcam::BoomCam,
    /// the near distance of this frame's view for the traffic's LOD (`Vehicles::pick_lods`): the boom file's
    /// `CameraNear` (base +172 - viewport +704) while a boom view drives the camera, else the port's own
    lod_near: f32,
    /// the cinematic view: each car's event camera views, and the camera
    event_views: std::collections::HashMap<String, Option<Arc<boomcam::EventViews>>>,
    event_cam: boomcam::EventCam,
    /// the change of view: blend, cut mask, look-back turn
    cam_switch: boomcam::Switch,
    /// the event camera's burnout view
    event_dir: boomcam::EventDirector,
    boom_up: Option<Vec3>,
    look_back: bool,
    hide_player: bool,
    /// the start screen's cameras (titlecam.rs: the press-start script's front-end shots)
    titlecam: titlecam::TitleCam,
    /// the camera move after START (startwarp.rs)
    startwarp: Option<startwarp::StartWarp>,
    /// its clock (s since START)
    startwarp_t: f32,
    /// the camera's outer wall rays: the distance each last found (infinite: nothing)
    cam_side_t: [f32; 4],
    /// the pad's buttons a frame ago (for presses), its right stick, its horn button
    pad_prev: u32,
    pad_pending: u32,
    pad_look: (f32, f32),
    /// the cruise assist: the speed it watches (smoothed), switched on, holding the limit now
    cruise_v: f32,
    cruise_limit_mph: f32,
    cruise_on: bool,
    cruise_hold: bool,
    /// the driving camera's look-around (boomcam::LookAround) and CAMERA_UP from the pad
    look: boomcam::LookAround,
    pad_look_up: f32,
    /// the event camera's pad edges this frame: CAM_PAN_LEFT / RIGHT and CAM_UP / CAM_DOWN pressed
    pad_pan: (bool, bool),
    /// CAM_PAN_LEFT held -1, CAM_PAN_RIGHT +1 (Hydraulics' look yaw, F-38892)
    pad_pan_side: f32,
    pad_cam_ud: bool,
    /// the player's own handbrake this frame (mcPlayerInput +20), before the port's parked hold
    own_handbrake: bool,
    /// mcCarSSTurbo +20: the turbo's lens ramp (F-60935)
    sst_view: f32,
    /// tune/camera/default.camfx, read once
    cam_fx: Option<boomcam::CamFx>,
    pad_horn: bool,
    /// HEADLIGHTS held on the pad (the input's +40 bit 0x2, F-56064)
    pad_flash: bool,
    /// the pad's actions that were down a frame ago (by the active map), a shift asked for and
    /// not yet handed to the car, and the last shift with its time
    pad_act_prev: u32,
    pad_shift: i8,
    pad_shift_last: Option<(i8, std::time::Instant)>,
    /// the gas station the car stands in
    at_station: Option<usize>,
    /// the GasStations script's visit (`mc4b::GasVisit`)
    gas: mc4b::GasVisit,
    /// (step 23b) the hangouts' scenes: their cars, the panel's camera, the pink slip choice
    hscene: hangout_scene::HangoutScene,
    /// the layout items' sources: the hangout cores and the story scenes (layout.rs)
    layout: layout::Layout,
    was_racing: bool,
    nitro_lap: usize,
    /// hydraulics switched on (L, or a click of the left stick)
    hyd_on: bool,
    hyd_toggle_was: bool,
    auto_last_speed: f32,
    /// the city drawn from above for the map screen: being made, and on the GPU
    citymap_rx: Option<std::sync::mpsc::Receiver<citymap::CityMap>>,
    citymap: Option<CityMapGpu>,
    /// the map screen's camera: what it looks at, how far out (0 near .. 3 the whole city), the rise
    map_target: Vec3,
    map_zoom: f32,
    map_zoom_want: f32,
    /// the GPS map's camera as the NavMap flies it (`nav_camera`): zoom parameter u (0 = the whole
    /// city at the top height, 1 = the lowest), and the look point on the ground
    map_u: f32,
    map_look: Vec3,
    /// the original's GPS camera (`hudmap::NavCam`)
    nav: hudmap::NavCam,
    /// the GPS cursor was free last frame / the stick is held
    nav_free: bool,
    /// the GPS camera of the last map frame (the exit flies from it)
    nav_rows: Option<hudmap::CamRows>,
    nav_free_moving: bool,
    map_open: f32,
    map_vp: Option<glam::Mat4>,
    drivers: drivers::Drivers,
    fx_impact: f32,
    /// the last drawn frame's view frustum (the glass break plays only for a car in view)
    last_frustum: Option<Frustum>,
    ability: ability::Abilities,
    fire_ability: bool,
    cycle_ability: bool,
}

/// The payback place's wait camera (`Hangouts/0xbaa0ed14` +3468 / +3480 / +3492) is the Beaches garage's
const PAYBACK_PLACE_CAM: usize = 1;
/// The deliveries' trigger points (the set-ups' static 3276: `game/Hangouts/0x4dd86889` 0x256,
/// `sc/Hangouts/0xc8c6051d` 0x259) - Hollywood's, South Central's.
/// The delivery's ease to pause before its panel (delivery_hangout_core 0x4b0: 0.25 s, form 67).
const DELIVERY_EASE_IN: f32 = 0.25;
const DELIVERY_POINT: [[f32; 3]; 2] = [[834.47, 18.07, -843.32], [-37.614, -19.674, 1316.65]];

impl App {
    /// The race's start scene from the original's picker and table (prerace.rs) and where it is staged.
    fn prerace_choose(&mut self, front: Vec3, yaw: f32, grid: &str) -> (Vec<String>, glam::Mat4) {
        let grid = grid.to_string();
        // the scene from the original's picker and table (prerace.rs): the grid's list or a taunt,
        // the Safety list for SafetyCam races; none for a grid the table does not have
        if self.prerace_picker.is_none() {
            self.prerace_picker = Some(prerace::Picker::new(prerace::GridTable::load(&self.opt.data_dir), (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u32)) ^ self.tick_no as u32));
        }
        let (mut args, slots) = self.game.prerace_args.take().unwrap_or_else(|| (prerace::PickArgs { grid: grid.clone(), first_race: true, ..Default::default() }, [front, front]));
        args.front_lr = slots.iter().any(|s| self.vehicles.cars.iter().any(|c| (c.state.pos - *s).with_y(0.0).length() < 1.5 && c.model.name.contains("_lr_")));
        let picker = self.prerace_picker.as_mut().unwrap();
        // (test hooks: MCLA_PRERACE=n plays the grid's n-th scene, MCLA_PRERACE_TAUNT=n the n-th taunt)
        let forced = std::env::var("MCLA_PRERACE").ok().and_then(|v| v.parse::<usize>().ok()).map(|n| (picker.list(&args.grid), n))
            .or_else(|| std::env::var("MCLA_PRERACE_TAUNT").ok().and_then(|v| v.parse::<usize>().ok()).map(|n| (picker.list("Taunt"), n)));
        let replay = std::mem::take(&mut self.prerace_replay).then(|| self.prerace_last.clone()).flatten().filter(|l| l.0 == self.game.current_race).map(|l| l.1);
        let chosen = match (forced, replay) {
            (Some((l, n)), _) => l.get(n % l.len().max(1)).cloned(),
            (None, Some(again)) => Some(again),
            (None, None) => picker.pick(&args),
        };
        if let Some(c) = chosen.as_ref() {
            self.prerace_last = Some((self.game.current_race, c.clone()));
        }
        log::info!("prerace pick: {:?} -> {:?}", args, chosen);
        let mut scenes: Vec<String> = chosen.into_iter().filter(|s| !self.cine.scenes(s).is_empty()).collect();
        let pick = 0usize;
        let mut origin = glam::Mat4::from_rotation_translation(glam::Quat::from_rotation_y(yaw), front);
        if let Some(sc) = self.game.prerace_scene.take() {
            // a scene staged at this very place (a delivery's intro: the race core binds the
            // scene's coordinates to RaceGrid_GetPosition(grid, 0) -
            // CineScript_SetGameCoordSysBinding, OrderedCheckpointCore 0x0F50)
            if hudmap::delivery_intro(&sc).is_none() {
                origin = glam::Mat4::IDENTITY;
            }
            scenes = vec![sc];
        }
        (scenes, origin)
    }
    /// A start scene begins (`BeginDescription` / `EndDescriptionAndStartClock`): its racer events armed, a
    /// taunt scene's speech prepared - the hookman (slot 1) speaks in the voice of his driver's character
    /// (CareerVoices.xml); when it is prepared, slot 0's left and slot 1's right windows go down at once.
    fn prerace_acts_begin(&mut self, scene: &str) {
        let events = prerace::slot_events(scene);
        let duration = self.cine.playing.as_ref().map_or(0.0, |sc| sc.duration);
        let racers = self.game.race.as_ref().map(|r| r.participants.clone()).unwrap_or_default();
        let mut taunt = false;
        if let (Some(kind), Some(&hook)) = (prerace::taunt_kind(scene), racers.get(1)) {
            let set = self.drivers.set_of.get(&hook).cloned().unwrap_or_default();
            let voice = prerace::voice_of(&self.opt.data_dir, &set).unwrap_or_default();
            taunt = !voice.is_empty() && self.speech.prepare_taunt(kind, &voice);
            log::info!("prerace scene {}: taunt {} hookman car {} set {} voice {:?} prepared {}", scene, kind, hook, set, voice, taunt);
        }
        self.prerace_acts = Some(prerace::Acts { scene: scene.to_string(), fired: vec![false; events.len()], events, t: 0.0, duration, racers, ends: Vec::new(), taunt });
        if taunt {
            self.prerace_window(0, false, true);
            self.prerace_window(1, true, true);
        }
    }

    /// The window of the racer in scene slot `slot` (the car's `win_l_open` / `win_r_open`).
    fn prerace_window(&mut self, slot: usize, right: bool, open: bool) {
        let Some(&ci) = self.prerace_acts.as_ref().and_then(|a| a.racers.get(slot)) else { return };
        log::info!("prerace scene: car {} {} window {}", ci, if right { "right" } else { "left" }, if open { "down" } else { "up" });
        self.vehicles.window(ci, right, open);
    }

    /// Fire the start scene's racer events as its clock passes them; the jaw flap follows the speech lines; at
    /// the clip's end `FinishScene` rolls the two windows up.
    /// CopPullover.sco after COP_PULLOVER (GitHub #17): over the walk, spinning the held car (Racer_IsBurningOut,
    /// F-48805: chassis +578, the burnout state) from 1 s in until 1 s before the walk's camera ends is a bolt -
    /// the car let go, the officer re-animated with `cut_cop_alert_02`'s run back to his car, the scene camera killed
    /// 1.5 s later, and PoliceManager_SuspectFleeing (police::suspect_fled) with the bolt's time; no ticket. Else the
    /// ticket scene, then the ticket form (the TICKETED panel; the fine is taken there), the police record cleared
    /// (ResetAllPolice) and the car let go.
    fn ticket_tick(&mut self, gpu: &mut Gpu, world: &mut World) {
        let Some((fine, bolt, second)) = self.ticket_run else { return };
        let playing = self.cine.playing.as_ref().map(|sc| (sc.name.clone(), sc.t, sc.camera_len()));
        match playing {
            Some((name, t, len)) if name.ends_with("cut_cop_walk_02") => {
                let spun = self.vehicles.player().and_then(|c| c.state.veh.as_ref()).map_or(false, |v| v.burn_state != 0);
                if bolt.is_none() && spun && t >= 1.0 && t < len - 1.0 {
                    log::info!("police: the suspect bolts {:.2} s into the walk", t);
                    self.ticket_run = Some((fine, Some(t), false));
                    self.vehicles.player_hold = false;
                    // (the bail: Racer_SetScrambleInputFlags 0x100, the handbrake let off, Racer_Activate)
                    self.vehicles.player_scramble = 0x100;
                    self.cine.reanimate("cut_drv01", "cut_cop_alert_02", "cut_drv01");
                    let at = self.vehicles.player().map(|c| c.state.pos).unwrap_or_default();
                    self.police.suspect_fled(t, at);
                    if !self.speech.busy() {
                        self.speech.say("PLAYER_COPS_ESCAPE_TICKET");
                    }
                    if self.radio.loaded() {
                        let rctx = radio_ctx(&self.vehicles, world, self.time_of_day);
                        self.radio.say("REPORT_SUSPECT_FLEES", true, &rctx);
                        self.radio_next = 9.0;
                    }
                } else if bolt.is_some_and(|b| t >= b + 1.5) {
                    self.cine.kill_camera();
                }
            }
            Some(_) => {}
            None if bolt.is_some() => {
                self.ticket_run = None;
                self.vehicles.player_scramble = 0;
                // (the walk runs to its end after a bolt too: FinishScene stops its gesture)
                if let Some(i) = self.vehicles.cars.iter().position(|c| c.is_player) {
                    self.drivers.end_gesture(i);
                }
            }
            None if !second => {
                // (the walk's end: 0x404, no throttle and no steering)
                self.vehicles.player_scramble = 0x404;
                // (the walk's FinishScene stops its gesture_left, F-65984 -> F-26895: the player's gesture blends out and the
                // ticket scene's copfidget is taken)
                if let Some(i) = self.vehicles.cars.iter().position(|c| c.is_player) {
                    self.drivers.end_gesture(i);
                }
                // the walk is over: the ticket scene (the car stays held, Racer_Deactivate)
                self.ticket_run = Some((fine, None, true));
                let (car, bike) = self.vehicles.player().map_or((String::new(), false), |c| (c.model.name.clone(), c.model.bike));
                let scene = police::ticket_scene(&car, bike);
                let frame = police::scene_frame(&mut self.vehicles, world, &self.police.safe_zones);
                if let (Some(frame), true) = (frame, self.cine.has(scene) && std::env::var("MCLA_NO_CINE").is_err()) {
                    if self.cine.start_bound(gpu, scene, frame) {
                        self.cine_spawn_cars(gpu, world);
                        self.cine_free = true;
                        self.cine_caption = None;
                    }
                }
            }
            None => {
                // the ticket form: the TICKETED panel, the fine
                self.ticket_run = None;
                self.vehicles.player_hold = false;
                self.vehicles.player_scramble = 0;
                self.game.money = (self.game.money - fine).max(0);
                self.game.ticket_for = Some(self.police.ticket_for.max(1));
                self.game.arrest_panel = Some((0.0, 0.0, fine));
                self.police.ticket_done();
                self.game.write_save(&self.opt.data_dir);
            }
        }
    }

    fn prerace_acts_tick(&mut self, dt: f32) {
        let Some(mut a) = self.prerace_acts.take() else { return };
        if self.game.race.is_none() {
            self.drivers.stop_acting();
            return;
        }
        // (after the cut the scene's script runs on to the clip's end)
        if !(self.cine_prerace && self.cine.playing.is_some()) {
            a.t += dt;
        }
        let mut windows: Vec<(usize, bool, bool)> = Vec::new();
        for i in 0..a.events.len() {
            let e = a.events[i];
            if a.fired[i] || e.t > a.t {
                continue;
            }
            a.fired[i] = true;
            let car = usize::try_from(e.slot).ok().and_then(|s| a.racers.get(s).copied());
            match e.event {
                "playspeech" => {
                    if a.taunt {
                        self.speech.play_taunt();
                    }
                }
                // (the stream player's four slots: the starter scenes' own dialogue stream)
                "stopaudiostreams" => {}
                "window_down_left" | "window_down_right" | "window_up_left" | "window_up_right" => {
                    if let Ok(slot) = usize::try_from(e.slot) {
                        windows.push((slot, e.event.ends_with("right"), e.event.contains("down")));
                    }
                }
                name => {
                    if let (Some(g), Some(ci)) = (drivers::Gesture::from_event(name), car) {
                        let human = self.vehicles.cars.get(ci).map_or(false, |c| c.is_player);
                        self.drivers.gesture(ci, g, e.dur, human);
                    }
                }
            }
        }
        // the speakers' jaws: the player's line - slot 0, the hookman's - slot 1
        let line = self.speech.line();
        for (slot, player) in [(0usize, true), (1usize, false)] {
            if let Some(&ci) = a.racers.get(slot) {
                self.drivers.jaw(ci, line == Some(player) && a.taunt);
            }
        }
        let over = a.t >= a.duration;
        let racers_n = a.racers.len();
        self.prerace_acts = Some(a);
        for (slot, right, open) in windows {
            self.prerace_window(slot, right, open);
        }
        if over {
            // (FinishScene, F-62618 mode 1: slot 0's left and slot 1's right windows up)
            if racers_n > 0 {
                self.prerace_window(0, false, false);
                self.prerace_window(1, true, false);
            }
            self.prerace_acts = None;
        }
    }

    fn new(opt: Options) -> App {
        let orbit = opt.orbit;
        let mut cam = Camera::default();
        let mut exact_start: Option<(Vec3, f32)> = None;
        if let Some(p) = opt.pos {
            cam.pos = p;
        } else if let Some((p, yaw)) = races::default_start(&opt.data_dir) {
            // the city's own default start (tune/city/city.lst): the gate of the Hollywood
            // garage, where the drive-out scene leaves the car
            let fwd = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
            cam.pos = p - fwd * 8.0 + Vec3::Y * 2.0;
            if opt.yaw.is_none() {
                cam.yaw = -yaw;
                exact_start = Some((p, yaw));
            }
        }
        if let Some(y) = opt.yaw {
            cam.yaw = y.to_radians();
        }
        if let Some(p) = opt.pitch {
            cam.pitch = p.to_radians();
        }
        App {
            time_of_day: opt.time.unwrap_or(14.0),
            sky: sky::Sky::load_weather(&opt.data_dir, opt.weather.unwrap_or(sky::Weather::Nice)),
            weather: opt.weather.unwrap_or(sky::Weather::Nice),
            sky_next: None,
            casc_scale: [1.0, 1.0],
            weather_timer: 0.0,
            weather_auto: false,
            weather_rng: Default::default(),
            weather_scene: false,
            cycle_weather: false,
            props: props::Props::empty(),
            sky_bind: None,
            drive: opt.drive,
            opt,
            window: None,
            gpu: None,
            world: None,
            egui: None,
            cam,
            chase: ChaseCam::default(),
            orbit,
            sim_time: 0.0,
            tick_no: 0,
            alt_menu_frames: 0,
            alt_units_made: false,
            sim_clock: fixed_step::Clock::default(),
            test_render_dt: std::env::var("MCLA_RENDER_HZ").ok().and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 10.0).map(|hz| 1.0 / hz),
            input: Input::default(),
            last: Instant::now(),
            fps: 0.0,
            frame_ms: 0.0,
            framelog: std::env::var("MCLA_FRAMELOG").ok().map(|v| v.parse::<f32>().unwrap_or(25.0)),
            fl_prev: [0.0; 9],
            fl_ui: [0.0; 5],
            fl_stream_ms: 0.0,
            draws: 0,
            show_ui: false,
            frame_no: 0,
            vehicles: Vehicles::new(),
            gamepad: open_pads(),
            pad_id: None,
            pad_rumble: None,
            pad_used: Default::default(),
            game: Game::new(Vec::new(), 0),
            cp_markers: Default::default(),
            quit: false,
            pending_action: MenuAction::None,
            audio: None,
            sound: sound::Sound::default(),
            police: police::Police::new(),
            fx: particles::Particles::new(),
            tyre_fx: None,
            propfx: propfx::PropFx::new(),
            peds: peds::Peds::new(),
            roam: roamers::Roamers::new(),
            garage_return: None,
            garage_clear: 0.0,
            garage_views: None,
            garage_cam: None,
            garage_tr: None,
            cine_lamps: None,
            race_end_cam: None,
            cine_showdriver: Vec::new(),
            race_end_from: None,
            race_end_way: None,
            garage_built: None,
            garage_up: Vec3::Y,
            garage_spots: Default::default(),
            garage_poses: None,
            news: Default::default(),
            speech: Default::default(),
            radio: radio::Radio::default(),
            dispatch_zones: Vec::new(),
            dispatch_prev: None,
            radio_next: 0.0,
            shape_lib: None,
            car_zones: Default::default(),
            cuts: Default::default(),
            cine: Default::default(),
            cine_hold: None,
            cut_audio_objects: false,
            cine_audio: None,
            cine_free: false,
            arrest_hands: false,
            pullover_acts: (String::new(), 0),
            arrest_form: None,
            arrest_station: None,
            arrest_traffic: None,
            arrest_mask: Default::default(),
            arrest_wait: false,
            arrest_fade: None,
            arrest_activate: false,
            police_acc: 0.0,
            ticket_run: None,
            cine_events: (String::new(), Vec::new()),
            tips: Default::default(),
            pending_call: None,
            pending_log: None,
            whoosh: Default::default(),
            whoosh_prev: Vec3::ZERO,
            whoosh_cool: 0.0,
            whooshes_heard: 0,
            barrels: Vec::new(),
            props_dirty: true,
            dress: None,
            powerups: None,
            redlights: Default::default(),
            exact_start,
            cine_prerace: false,
            cine_after: None,
            hangouts: Vec::new(),
            hangout_in: None,
            mode_before: Mode::MainMenu,
            cine_results: false,
            tour_field: None,
            cine_caption: None,
            shot_at_go: false,
            prerace_picker: None,
            prerace_acts: None,
            prerace_last: None,
            prerace_replay: false,
            idle: 0.0,
            prerace_wait: 0,
            warp: None,
            warp_t: 0.0,
            warp_view: None,
            climb: None,
            climb_view: None,
            climb_end: None,
            climb_hold: 0,
            climb_wait: None,
            screen_eye: Vec3::ZERO,
            screen_v: Vec3::ZERO,
            prerace_plan: None,
            starter_was: false,
            delivery_armed: true,
            delivery_cops: 0.0,
            delivery_ease: None,
            delivery_visit: 0,
            delivery_intro: None,
            delivery_tail: None,
            garage_in: None,
            goto_garage: None,
            hangout_entry: None,
            hangout_title: None,
            hangout_prev: None,
            place_view: None,
            payback_in: None,
            payback_hold: false,
            garage_mask: Default::default(),
            garage_out: None,
            phone_challenge_in: 0.0,
            phone_caller: "",
            phone_challenge_due: false,
            phone_hold: false,
            phone_resume: None,
            phone_cruise: false,
            phone_was_up: false,
            phone_done: false,
            phone_delay: None,
            phone_redelay: false,
            intro_played: false,
            intro_stage: 0,
            intro_t: 0.0,
            audio_trace_t: 0.0,
            intro_mask: Default::default(),
            intro_tod: None,
            starters_made: false,
            grid_warm: None,
            opening_read: 0,
            starter_stand: None,
            sel_cam: None,
            settle_frames: 0,
            news_timer: 0.0,
            photo: None,
            photo_request: None,
            photo_msg: 0.0,
            wreck_timer: 0.0,
            wreck_engine: 0,
            wreck_done: 0.0,
            wipe_scale: 1.0,
            warp_wipe: boomcam::TimeWarper::new([1.25, 0.1, 5.0, 10.0]),
            warp_expl: boomcam::TimeWarper::new([0.5, 0.1, 3.0, 1.0]),
            // (F-49780's defaults: Duration 0, TimeWarp 0.239, Rise 2.009, Fall 3.009)
            warp_look: boomcam::TimeWarper::new([0.0, 0.239, 2.009, 3.009]),
            look_scene: false,
            ability_cam: Default::default(),
            crash_bits: 0,
            crash_timer: 0.0,
            crash_exploding: false,
            crash_cam: Default::default(),
            idle_time: 0.0,
            view_mode: 0,
            replay_shot_t: f32::MIN,
            replay_prerace: None,
            replay_wall: 0.0,
            shake: None,
            shake_impact: 0.0,
            shake_air: 0.0,
            shake_wipe: false,
            shake_land: false,
            shake_sst: false,
            sst_fires: 0,
            nitro_fires: 0,
            shake_jolts: Vec::new(),
            district_timer: 0.0,
            gps_key: (None, None),
            flash: false,
            enter_garage: false,
            cam_mode: std::env::var("MCLA_CAM_MODE").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
            booms: Default::default(),
            boom: Default::default(),
            lod_near: 0.3,
            event_views: Default::default(),
            event_cam: Default::default(),
            cam_switch: Default::default(),
            event_dir: Default::default(),
            boom_up: None,
            look_back: false,
            hide_player: false,
            titlecam: Default::default(),
            startwarp: None,
            startwarp_t: 0.0,
            cam_side_t: [f32::INFINITY; 4],
            pad_prev: 0,
            pad_pending: 0,
            pad_look: (0.0, 0.0),
            cruise_v: 0.0,
            cruise_limit_mph: 40.0,
            cruise_on: false,
            cruise_hold: false,
            look: Default::default(),
            pad_look_up: 0.0,
            pad_pan: (false, false),
            pad_pan_side: 0.0,
            pad_cam_ud: false,
            own_handbrake: false,
            sst_view: 0.0,
            cam_fx: None,
            pad_horn: false,
            pad_flash: false,
            pad_act_prev: 0,
            pad_shift: 0,
            pad_shift_last: None,
            at_station: None,
            gas: mc4b::GasVisit::default(),
            hscene: Default::default(),
            layout: Default::default(),
            was_racing: false,
            nitro_lap: 0,
            hyd_on: false,
            hyd_toggle_was: false,
            auto_last_speed: 0.0,
            citymap_rx: None,
            citymap: None,
            map_target: Vec3::ZERO,
            map_zoom: 1.0,
            map_zoom_want: 1.0,
            map_u: 0.0,
            map_look: Vec3::ZERO,
            nav: hudmap::NavCam::default(),
            nav_free: false,
            nav_rows: None,
            nav_free_moving: false,
            map_open: 0.0,
            map_vp: None,
            drivers: drivers::Drivers::default(),
            fx_impact: 0.0,
            last_frustum: None,
            ability: ability::Abilities::new(),
            fire_ability: false,
            cycle_ability: false,
        }
    }

    /// Rain intensity 0..1 (storms only), following weather cross-fades.
    fn rain_amount(&self) -> f32 {
        self.weather_weight(sky::Weather::Stormy)
    }

    /// A weather's weight 0..1 (the original's w[k], WEATHER_AUDIO_RULE.md), following cross-fades.
    /// The weather's change, once a frame (F-18159; notes `work/career2/weather/notes.md`).
    /// W (0x8287E134) is the weather blended towards - `sky_next`'s, else the one in force.
    /// - hw_local_hangout2_setup's first instruction, Weather_GetAndSetAutoWeatherChange(1): the
    ///   auto change on for the rest of the session (the flag is not saved).
    /// - A cine scene (CineScripts 0x95ca1d1b): at its start Stormy becomes Cloudy unless the
    ///   scene is a race start's; during it the auto change and the time of day's update are off
    ///   - no countdown, the weights snap -, both back at its end.
    /// - The garage's lighting (garage +393) forces Nice (W = 0), which stays after.
    /// - With the time of day's update on (+14545; off with a clock fixed on the command line):
    ///   the countdown by the game's frame step while the game is not paused; at 0 the next draw
    ///   300 s on and, with the auto change on, r = the MWC's 23-bit float: Nice -> Nice below
    ///   0.45, else Cloudy; Cloudy -> Nice below 0.70, Cloudy below 0.75, else Foggy; Stormy ->
    ///   Foggy; Foggy -> Cloudy below 0.53, else Stormy. The weights then move to W's at
    ///   dt x d x 0.0625, snapped within 0.05 (0 -> 1 in 47.9 s); with it off they snap.
    fn weather_tick(&mut self, dt: f32) {
        use sky::Weather::*;
        if self.opt.weather.is_some() {
            return;
        }
        // (HANGOUT_SERIES_HOLLYWOODB's mission script starts it as the mission enters the log,
        // and again with a loaded save)
        // (test hook: MCLA_WEATHER_TEST="timer,seed" - the next draw that many seconds on, the
        // draws' MWC from that seed)
        if self.tick_no == 60 {
            if let Ok(v) = std::env::var("MCLA_WEATHER_TEST") {
                let n: Vec<u32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                self.weather_timer = n.first().copied().unwrap_or(10) as f32;
                self.weather_rng = vehicles::parked::Mwc { lo: n.get(1).copied().unwrap_or(1), carry: 1 };
            }
        }
        if !self.weather_auto && self.tick_no % 30 == 0 && self.game.career.mission_started("HANGOUT_SERIES_HOLLYWOODB") {
            self.weather_auto = true;
            log::info!("weather: hw_local_hangout2_setup - the auto change on");
        }
        let target = |a: &Self| a.sky_next.as_ref().map_or(a.weather, |n| n.1);
        let scene = self.cine.playing.as_ref().map(|s| s.name.clone());
        if let (Some(name), false) = (scene.as_ref(), self.weather_scene) {
            if target(self) == Stormy && !name.contains("racestart") {
                self.weather_set(Cloudy);
            }
        }
        self.weather_scene = scene.is_some();
        if self.game.mode == Mode::Garage && target(self) != Nice {
            self.weather_set(Nice);
        }
        let game_dt = dt * self.game.mc4_time_scale();
        let update = self.opt.time.is_none() && scene.is_none();
        if std::env::var("MCLA_WEATHER_TRACE").is_ok() && self.tick_no % 120 == 0 {
            log::info!("weather: {} -> {} blend {:.3} timer {:.1} auto {} scene {:?} mode {:?} menu {}", self.weather.name(), target(self).name(), self.sky_next.as_ref().map_or(0.0, |n| n.2), self.weather_timer, self.weather_auto, scene, self.game.mode, self.game.in_menu());
        }
        if !update {
            if let Some((sky, w, _)) = self.sky_next.take() {
                self.sky = sky;
                self.weather = w;
            }
            return;
        }
        if matches!(self.game.mode, Mode::FreeRoam | Mode::Race) && !self.game.in_menu() {
            self.weather_timer -= game_dt;
            if self.weather_timer <= 0.0 {
                self.weather_timer = 300.0;
                if self.weather_auto {
                    let r = self.weather_rng.float();
                    let w = match target(self) {
                        Nice => if r < 0.45 { Nice } else { Cloudy },
                        Cloudy => if r < 0.70 { Nice } else if r < 0.75 { Cloudy } else { Foggy },
                        Stormy => Foggy,
                        _ => if r < 0.53 { Cloudy } else { Stormy },
                    };
                    log::info!("weather: draw {:.3} -> {}", r, w.name());
                    self.weather_set(w);
                }
            }
        }
        if let Some((_, _, f)) = self.sky_next.as_mut() {
            *f += game_dt * (1.0 - *f) * 0.0625;
            if 1.0 - *f < 0.05 {
                let (sky, w, _) = self.sky_next.take().unwrap();
                self.sky = sky;
                self.weather = w;
            }
        }
    }

    /// W = `w`, blended (the weights move towards it); back to the weather in force drops the blend.
    fn weather_set(&mut self, w: sky::Weather) {
        if self.sky_next.as_ref().map_or(self.weather, |n| n.1) == w {
            return;
        }
        if w == self.weather {
            self.sky_next = None;
        } else {
            self.sky_next = Some((sky::Sky::load_weather(&self.opt.data_dir, w), w, 0.0));
        }
    }

    fn weather_weight(&self, k: sky::Weather) -> f32 {
        let cur = if self.weather == k { 1.0 } else { 0.0 };
        match self.sky_next.as_ref() {
            Some((_, w, f)) => {
                let nx = if *w == k { 1.0 } else { 0.0 };
                cur + (nx - cur) * f
            }
            None => cur,
        }
    }

    /// The tone map's tuning at this hour: the weather's keys, as the original feeds them to
    /// its post effect (LIGHTING_WORK.md).
    fn post_tune(&self, st: &sky::SkyState) -> render::post::Tune {
        let mut p = render::post::Tune::default();
        if let Some(t) = st.tod {
            p.middle_grey = t.luminance[0];
            p.white = t.luminance[1] * t.luminance[1];
            p.bright = [t.bright[0], t.bright[1], t.bright[3] * t.bright[3]];
            p.limits = [t.adaption[1], t.adaption[2]];
            p.lambda = t.adaption[0] * 20.0;
            p.saturation = t.saturation;
            p.color_correct = [t.color_correct.x, t.color_correct.y, t.color_correct.z];
        }
        p
    }

    /// In the garage: its own light state (sky.rs `garage_tod` - the TODGarage keys, the area
    /// locators' colour correction and fog) and the light manager's garage figures. None
    /// outside it, without the files, or with MCLA_GARAGE_OLD=1 (the hand-set rig of before).
    fn garage_light(&self) -> Option<(sky::GarageRig, sky::Tod)> {
        if self.game.mode != game::Mode::Garage || std::env::var("MCLA_GARAGE_OLD").is_ok() {
            return None;
        }
        let at = self.vehicles.player().map_or(self.cam.pos, |c| c.state.pos);
        Some((sky::garage_rig(&self.opt.data_dir)?, sky::garage_tod(&self.opt.data_dir, self.weather, at)?))
    }

    /// The sky's state at this moment (the weather cross-fade included).
    fn sky_now(&self) -> sky::SkyState {
        // (the garage: the sky's keys at the garage's own hour, the scene's from its own file)
        let gar = self.garage_light();
        let t = gar.as_ref().map_or(self.time_of_day, |g| g.0.hour);
        let mut st = self.sky.at(t);
        if let Some((next, _, f)) = self.sky_next.as_ref() {
            st = st.lerp(&next.at(t), *f);
        }
        if let Some((_, gt)) = gar {
            st.tod = Some(gt);
        }
        st
    }

    fn lighting(&self, vp: glam::Mat4) -> Globals {
        let t = self.time_of_day;
        // (the garage - garage round, 2026-10-13: the scene's keys are TODGarage<Weather>'s, the sky's
        // and the fog's are read at sm_fGarageTOD, whatever the clock says; `garage_light`)
        let gar = self.garage_light();
        let ts = gar.as_ref().map_or(t, |g| g.0.hour);
        let mut st = self.sky.at(ts);
        let mut fogp = self.weather.fog();
        let mut wet = self.weather.wet();
        let mut cover = self.weather.cloud_cover();
        if let Some((next, w, f)) = self.sky_next.as_ref() {
            st = st.lerp(&next.at(ts), *f);
            let nf = w.fog();
            fogp = (fogp.0 + (nf.0 - fogp.0) * f, fogp.1 + (nf.1 - fogp.1) * f, fogp.2 + (nf.2 - fogp.2) * f);
            wet += (w.wet() - wet) * f;
            cover += (w.cloud_cover() - cover) * f;
        }
        if let Some((_, gt)) = gar.as_ref() {
            st.tod = Some(*gt);
        }
        let day = st.day;
        // The dome shows its keyframes brightened by a fixed gain (checked against the
        // disc's own footage: midday zenith 197 228 254 = the "clear" key x 1.5). The
        // horizon is the zenith tinted by the azimuth colours: the first away from the
        // sun, the second on the sun's side.
        let g22 = st.gain.powf(2.2);
        let show = |c: Vec3| (c * g22).min(Vec3::splat(0.92));
        let s_far = st.az_strength;
        let s_sun = (st.az_strength * 1.25).min(0.9);
        let zen_d = show(st.zenith);
        let hor_d = show(st.zenith.lerp(st.horizon, s_far));
        let hor_sun_d = show(st.zenith.lerp(st.horizon2, s_sun));
        // scene-referred values that come out as those colours after the tone curve
        let untone = |l: Vec3, k: f32| -> Vec3 {
            let f = |x: f32| (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
            let one = |t: f32| {
                let (mut a, mut b) = (0.0f32, 12.0f32);
                for _ in 0..28 {
                    let m = (a + b) * 0.5;
                    if f(m) < t { a = m } else { b = m }
                }
                (a + b) * 0.5 / k
            };
            Vec3::new(one(l.x), one(l.y), one(l.z))
        };
        let zen = untone(zen_d, 1.15 * 0.7);
        let hor = untone(hor_d, 1.15 * 0.7);
        let hor_sun = untone(hor_sun_d, 1.15 * 0.7);
        let low = (1.0 - (st.true_sun.y.abs() * 3.5).min(1.0)).max(0.0);
        let mut sun_col = st.sun_color * 5.0;
        let mut sun_int = day * 2.1 + (1.0 - day) * 0.06;
        let mut sky_amb = zen * (0.25 + 0.23 * day) + Vec3::new(0.035, 0.04, 0.06) * (1.0 - day);
        let mut ground = (hor * 0.38 + Vec3::new(0.05, 0.045, 0.04)) * (0.12 + 0.38 * day) + Vec3::new(0.03, 0.03, 0.04) * (1.0 - day);
        if let Some((light, amb)) = st.light {
            // the game's own keyed sun light and ambient (TODSun* in tune/lighting),
            // brought to this renderer's exposure
            const K: f32 = 1.7;
            let peak = light.max_element();
            if peak > 0.02 {
                sun_col = light / peak;
                sun_int = peak * K;
            } else {
                // moonlight
                sun_col = Vec3::new(0.55, 0.65, 0.9);
                sun_int = 0.05;
            }
            sky_amb = amb * K * 1.15;
            // sky lighting ground colour of the disc (0.42, 0.36, 0.40)
            ground = amb * K * 0.85 * Vec3::new(1.06, 0.92, 1.01);
        }
        // The original's light state, as measured on the running game (LIGHTING_WORK.md): the
        // weather's keys go to the shaders as they are - light colour, ambient, shadow depth -
        // and the scene is exposed by the tone map alone. (By night the game draws with other
        // programs; until those are ported the older night lighting above stands in.)
        let tod = st.tod;
        let tod_day = tod.filter(|t| t.nfl <= 0.0);
        let tod_night = tod.filter(|t| t.nfl > 0.0);
        if let Some(t) = tod_night {
            // (night: the key ambient; the directional light is a grey of 0.05 from below the
            // horizon in the original - left out)
            sun_col = t.light;
            sun_int = 0.0;
            sky_amb = t.ambient;
            ground = t.ambient;
        }
        if let Some(t) = tod_day {
            sun_col = t.light;
            sun_int = 1.0;
            sky_amb = t.ambient;
            ground = t.ambient;
            st.sun_dir = t.to_light;
        }
        // (test hooks, to compare one term at a time with the original: MCLA_AMB / MCLA_LIGHT = "r,g,b")
        let rgb_env = |name: &str| -> Option<Vec3> {
            let f: Vec<f32> = std::env::var(name).ok()?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (f.len() == 3).then(|| Vec3::new(f[0], f[1], f[2]))
        };
        if let Some(c) = rgb_env("MCLA_AMB") {
            sky_amb = c;
            ground = c;
        }
        if let Some(c) = rgb_env("MCLA_LIGHT") {
            sun_col = c;
            sun_int = 1.0;
        }
        // (the garage is lit by its own lamps, whatever the hour: an even, warm working light)
        let in_garage = self.game.mode == game::Mode::Garage;
        // (only without the garage's own light state: `garage_light`, MCLA_GARAGE_OLD=1)
        if in_garage && gar.is_none() {
            // (as the game's garage looks: dim, the car under ceiling lamps that leave soft
            // white highlights on its paint - one light from above and ahead stands for them)
            // (the figures were set by eye under the port's old tone curve; the original's curve
            // and display ramp, which now follow, lift the bright end about twice as far - so
            // they are scaled to land where they did. The garage's own lights are not read yet.)
            sky_amb = Vec3::new(0.15, 0.145, 0.14) * 0.45;
            ground = Vec3::new(0.10, 0.095, 0.09) * 0.45;
            sun_col = Vec3::new(1.0, 0.97, 0.92);
            sun_int = 0.42 * 0.45;
            st.sun_dir = Vec3::new(0.18, 0.93, 0.32).normalize();
        }
        // clouds: the sky with the cloud colour scattered in on top (white wisps at noon,
        // glowing at sunset); their undersides lean towards the top layer's colour
        let raw = |c: Vec3| Vec3::new(c.x.max(0.0).powf(1.0 / 2.2), c.y.max(0.0).powf(1.0 / 2.2), c.z.max(0.0).powf(1.0 / 2.2));
        let lin = |c: Vec3| Vec3::new(c.x.powf(2.2), c.y.powf(2.2), c.z.powf(2.2));
        let cloud_d = lin(((raw(st.zenith) + raw(st.cloud) * 1.15) * st.gain).min(Vec3::splat(0.985)));
        let shade_d = lin(((raw(st.zenith) * 0.62 + raw(st.cloud_top) * 0.55) * st.gain).min(Vec3::splat(0.9)));
        let cloud_c = untone(cloud_d.min(Vec3::splat(0.95)), 1.15 * 0.7);
        let cloud_s = untone(shade_d, 1.15 * 0.7);
        // distance haze takes the colour of the horizon
        let fog = untone(hor_d.lerp(hor_sun_d, 0.35), 1.15);
        let stars = (1.0 - day) * 1.2;
        // the tone map's figures for what is still composed in display terms (sky, glows), and
        // the fog as the weather keys it: its colour leans a quarter towards the sky's horizon
        // colours (the dome's two azimuth keys, as they are: west first, east second)
        let post = self.post_tune(&st);
        let lum_now = if tod.map_or(false, |t| t.nfl > 0.5) { post.limits[0] } else { post.limits[1] };
        let tone = [post.middle_grey / (lum_now + 0.001), post.white, post.saturation, if tod.map_or(false, |t| t.nfl > 0.0) { 1.0 } else { 0.0 }];
        // (test hook: MCLA_OCCW = OcclusionInfo.w, 1 switches the vertex occlusion off)
        let occ_w = std::env::var("MCLA_OCCW").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(tod_day.map_or(0.0, |t| 1.0 - t.occlusion[0]));
        let grade = [2.0 * post.color_correct[0], 2.0 * post.color_correct[1], 2.0 * post.color_correct[2], occ_w];
        let (az_west, az_east) = (self.sky.azimuth.rgb(ts), self.sky.azimuth2.rgb(ts));
        // (2026-10-15, the fog pass's colour constant c64 against the keys in 18 stored captures of the
        // nice weather and six of the others - work/lighting/tw_fogfit.py, tw_fogwx.py: its alpha is
        // sm_KeyMaxFogColor's as it is, its rgb
        //     key rgb x lerp(m_KeyColor rgb + m_KeyAmbLight rgb, 1, m_KeyColor.w)
        // to 1.4e-7 - the light file's fourth light channel: Nice 0.447, Cloudy 1, Stormy / Foggy
        // 0.502 by day, Stormy 0 at night. And the keys hold AT NIGHT as well (alpha 0.196 at 05:45,
        // 0.085 at 02:00, 0 from 18:42 to midnight in nice weather; 0.59 on a stormy night): the
        // older haze stood in under the night programs - at 05:45 a yellow veil of 0.64 over the
        // far towers where the original lays 0.18 of a dark grey-blue. MCLA_DEBUG_VIEW=460: as it
        // stood - the key's colour as it is by day, the older haze at night.)
        let fog_old = std::env::var("MCLA_DEBUG_VIEW").ok().map_or(false, |v| v.trim() == "460");
        let (fog, fogp, fog_lean) = match (if fog_old { tod_day } else { tod }) {
            Some(d) => {
                let k = if fog_old { Vec3::ONE } else { (d.light + d.ambient).lerp(Vec3::ONE, d.light_w) };
                (Vec3::new(d.fog_color[0], d.fog_color[1], d.fog_color[2]) * k, (d.fog_near, d.fog_far, d.fog_color[3]), 0.25)
            }
            None => (fog, fogp, 0.0),
        };
        let shadow_strength = if tod_night.is_some() { 0.0 } else { tod_day.map_or(st.shadow_strength(), |d| d.shadow_fade.clamp(0.0, 1.0)) };
        // the dome by its own keys (the game's Skyhat program; HDRExposure 0.8 in every sky file)
        let dome = st.dome.filter(|_| tod.is_some());
        let (zen, hor, hor_sun, cloud_c, cloud_s) = match dome {
            Some((c, _)) => (c[0], c[1], c[2], c[0] + c[3], c[0] * 0.6 + c[4] * 0.5),
            None => (zen, hor, hor_sun, cloud_c, cloud_s),
        };
        if std::env::var("MCLA_TOD_DUMP").is_ok() {
            if let Some(t) = tod.as_ref() { eprintln!("tod {:.2}: to_light {:?} light {:?} ambient {:?} nfl {}", self.time_of_day, t.to_light, t.light, t.ambient, t.nfl); }
        }
        let (mut car_key, mut car_key_col, mut car_key2, mut car_key2_col) = self.car_key_light(
            tod.as_ref().map_or(0.0, |t| t.nfl),
            tod_night.map_or((st.sun_dir, sun_col * sun_int), |t| (Vec3::new(t.to_light.x, t.to_light.y, t.to_light.z), Vec3::new(t.light.x, t.light.y, t.light.z))),
        );
        // The garage's lights on the car (recorded: gLightPosDir / gLightColor of every car program in
        // work/lighting/cap/gar_*.bin): the programs' first light is a point 4 m above the car, colour
        // sm_vGarageLight0Color x sm_fGarageLight0Intensity (0 on the disc: off); the second a point 1 m
        // above the CAMERA, sm_vGarageLight1Color x sm_fGarageLight1Intensity; both with range 1000.
        // The cars' ambient is sm_vGarageAmbientLighting (c57 ambYPos; their gLightAmbient holds it cut to
        // 8 bits, 0.19216), the blurred mirror map's imgExposure is 1, and the collector is the 8 x 8 grey.
        // car_key2.w = 2: a position (city.wgsl car_kd / car_kc). The first light is handed over as the
        // direction straight up - exact only above the car's origin; it has no strength on the disc.
        let mut car_amb = [0.0f32; 4];
        if let Some((rig, _)) = gar.as_ref() {
            let (l0, l1) = (rig.light0 * rig.int0, rig.light1 * rig.int1);
            car_key = [0.0, 1.0, 0.0, 1.0];
            car_key_col = [l0.x, l0.y, l0.z, 1.0];
            car_key2 = [self.cam.pos.x, self.cam.pos.y + 1.0, self.cam.pos.z, 2.0];
            car_key2_col = [l1.x, l1.y, l1.z, 0.0];
            car_amb = [rig.ambient.x, rig.ambient.y, rig.ambient.z, 1.0];
        }
        Globals {
            car_amb,
            car_key,
            car_key_col,
            car_key2,
            car_key2_col,
            // (recorded: 18:30 colour (0.390, 0.184, 0.047) from (-0.988, 0.152, 0.003); 23:30 a grey
            // 0.051 from below the horizon, (-0.591, -0.806, -0.014))
            // (w of the two: m_RoadReflection's two channels - gReflectionParams.xy of the road's programs)
            key_dir: tod_night.map_or([0.0, 1.0, 0.0, tod.map_or(0.0948, |t| t.road_reflection[0])], |t| [t.to_light.x, t.to_light.y, t.to_light.z, t.road_reflection[0]]),
            key_col: tod_night.map_or([0.0, 0.0, 0.0, tod.map_or(0.0948, |t| t.road_reflection[1])], |t| [t.light.x, t.light.y, t.light.z, t.road_reflection[1]]),
            dome: dome.map_or([1.0, 4.0, 0.8, 0.0], |(_, k)| [k[0], k[1], 0.8, 1.0]),
            tone,
            grade,
            fog_east: [az_east.x, az_east.y, az_east.z, fog_lean],
            fog_west: [az_west.x, az_west.y, az_west.z, 0.0],
            view_proj: vp.to_cols_array_2d(),
            cam_pos: [self.cam.pos.x, self.cam.pos.y, self.cam.pos.z, t],
            sun_dir: [st.sun_dir.x, st.sun_dir.y, st.sun_dir.z, sun_int],
            sun_color: [sun_col.x, sun_col.y, sun_col.z, self.time_of_day],
            ambient_sky: [sky_amb.x, sky_amb.y, sky_amb.z, st.shadow.unwrap_or(-1.0)],
            // (w: the HDR weathers' reference light level - sun plus ambient as they are now - for the
            // display-space lighting of the shader; 0 = the older linear look)
            // (with the weather's keys: x = gPropNFLColor.w = 1.5 x m_NFLFade, y = OcclusionInfo.x =
            // the camera's height + m_OcclusionHeights.x (5), z = m_OcclusionFades.y; w = -1 in the garage)
            ambient_ground: match tod {
                Some(t) if !in_garage || gar.is_some() => [1.5 * t.nfl, self.cam.pos.y + 5.0, t.occlusion[1], 0.0],
                _ => [ground.x, ground.y, ground.z, if in_garage { -1.0 } else if st.shadow.is_some() { (sun_col * sun_int + sky_amb).dot(Vec3::new(0.2126, 0.7152, 0.0722)) } else { 0.0 }],
            },
            fog_color: [fog.x, fog.y, fog.z, fogp.2],
            params: [fogp.0, fogp.1, 1.15, 0.0],
            shadow_params: [shadow_strength, 0.0, wet, 0.0],
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            sky_zenith: [zen.x, zen.y, zen.z, day],
            sky_horizon: [hor.x, hor.y, hor.z, stars],
            // (horizon on the sun's side; a = how far up the glow reaches)
            sky_sunset: [hor_sun.x, hor_sun.y, hor_sun.z, 0.55 + 0.45 * low],
            true_sun: [st.true_sun.x, st.true_sun.y, st.true_sun.z, 0.0],
            moon_dir: [-0.072, 0.215, 0.974, 0.0],
            cloud_color: [cloud_c.x, cloud_c.y, cloud_c.z, cover],
            cloud_shade: [cloud_s.x, cloud_s.y, cloud_s.z, self.sim_time * 0.0022],
            lod: { let k = crate::props::Lod::now(); [k.draw, k.detail, k.plant_angle, 0.0] },
            ..Globals::default()
        }
    }

    /// The lamp that lights the cars at night. The game gives its car programs two lights
    /// (gLightPosDir / gLightColor 0 and 1): by day the sun, by night the strongest of the
    /// city's baked lights where the car stands - its colour, the direction to it and the
    /// strength sat(Intensity / distance squared). (Recorded at the Hollywood garage's gate,
    /// 23:30: the light at (817.4, 26.3, -805.2), colour (1, 0.806, 0.333), intensity 40,
    /// 9.56 m off: strength 0.437.) Taken for the player's car and used for every car.
    fn car_key_light(&self, nfl: f32, hour_light: (Vec3, Vec3)) -> ([f32; 4], [f32; 4], [f32; 4], [f32; 4]) {
        // (w of the first: imgExposure of the blurred environment map - 0.473 while m_NFLFade is
        // at most a half, 1.29 above; recorded round the clock: 5:00 1.29, 5:30 0.473, 18:30
        // 0.473, 18:45 1.29. The game eases between the two over a few seconds; here a step.)
        // CORRECTED 2026-10-12: it is no step. Recorded at thirteen hours it runs in a straight
        // line from 1.29 at 05:30 to 0.473 at 06:30 and back from 17:30 to 18:30 (05:45 1.0857,
        // 05:54 0.9632, 06:00 0.8815, 06:15 0.6772; 17:45 0.6772, 18:00 0.8815, 18:15 1.0857) -
        // by the clock, half an hour either side of sunrise and sunset, not by m_NFLFade.
        // (At exactly 05:30 and 18:30 the game shows 0.473: an edge of its own, not copied.)
        let _ = nfl > 0.5;
        let hour = self.time_of_day.rem_euclid(24.0);
        let img_exposure = if hour <= 5.5 || hour >= 18.5 {
            1.29
        } else if hour < 6.5 {
            1.29 + (0.473 - 1.29) * (hour - 5.5)
        } else if hour <= 17.5 {
            0.473
        } else {
            0.473 + (1.29 - 0.473) * (hour - 17.5)
        };
        // (w of the second: which of the car's two lights comes first - the only one the fast
        // shadow collector weighs. The game settles it when the car is made: the lamp first for
        // a car made at night, the hour's light first for one made by day, and it stays so
        // through later days and nights. Here: by the hour the session starts at;
        // MCLA_CAR_SLOT=0 / 1 forces the hour's light / the lamp first, for comparisons.)
        // 2026-10-14, car round 10 - the game's own routine, read from the executable:
        // F-56246 (every car, every frame) asks F-24398 for the two strongest lights
        // where the car stands. Candidates and their weights:
        //   - from dusk to dawn (hour > 18 or < 6) the lamps of the lamp grid's cell: weight =
        //     sat(Intensity / d^2) x sat(cone) (F-60154), which is also the strength the
        //     slot then carries (F-77922);
        //   - the hour's light (the light manager's + 64): 2 x (0.30 r + 0.59 g + 0.11 b) of
        //     its colour (weights 0x827E25E0, the 2 at 0x827E264C), strength 1.
        //   (also the dynamic lights - the player's head lamps for a car in their cone - and a
        //   second manager light that is black in every capture: not taken here.)
        // The best goes to the first slot, the runner-up to the second - unless one of them
        // already sits in the other slot: a light keeps its slot while it stays chosen (ids at
        // car + 6336 / + 6388). So the order is the car's history: the hour's light first for a
        // car made by day, the lamp first for one made at night. Each slot is then eased, per
        // frame of the 30 Hz game: the direction turns 2.5 % of the angle left (F-27981),
        // colour and strength go 6 % of the way; the light is handed to the programs as a point
        // 100 m off along that direction with range 1000 (the programs' 0.9).
        // MCLA_CAR_SLOT=0 / 1 starts with the hour's light in the first / second slot (a
        // capture's history), otherwise the slots start empty at the session's first frame.
        *LAST_HOUR.lock().unwrap() = Some((nfl, hour_light));
        const HOUR: i32 = 65545;
        struct Slots { held: [i32; 2], dir: [Vec3; 2], col: [Vec3; 2], int: [f32; 2], frame: u64, time: f32, init: bool }
        static SLOTS: std::sync::Mutex<Option<Slots>> = std::sync::Mutex::new(None);
        let mut guard = SLOTS.lock().unwrap();
        let sl = guard.get_or_insert_with(|| Slots {
            held: match std::env::var("MCLA_CAR_SLOT").ok().as_deref() {
                Some("0") => [HOUR, -2],
                Some("1") => [-2, HOUR],
                _ => [-1, -1],
            },
            dir: [Vec3::Y; 2],
            col: [Vec3::ZERO; 2],
            int: [0.0; 2],
            frame: u64::MAX,
            time: 0.0,
            init: false,
        });
        if sl.frame != self.frame_no || !sl.init {
            let at = self.vehicles.player().filter(|_| self.drive).map(|c| c.state.pos).unwrap_or(self.cam.pos);
            let cand = self.slot_candidates(at, nfl, hour_light, None);
            let hw = cand.last().map_or(0.0, |c| c.1);
            let (b0, w0, b1, w1) = pick_two(&cand);
            let swap = slot_swap(sl.held, b0, b1);
            // (a forced start keeps the hour's light where it was put until it is chosen once)
            let forced = sl.held.contains(&-2);
            sl.held = if forced && (b0 == HOUR || b1 == HOUR) {
                let other = if b0 == HOUR { b1 } else { b0 };
                if sl.held[0] == HOUR { [HOUR, other] } else { [other, HOUR] }
            } else if forced {
                // (the hour's light lost its slot to a second lamp: the first lamp stays where it was)
                if sl.held[0] == HOUR || sl.held[0] == -2 && sl.held[1] != HOUR { [b1, b0] } else { [b0, b1] }
            } else if swap {
                [b1, b0]
            } else {
                [b0, b1]
            };
            // the game's frames since the last call
            let n = if !sl.init { 0.0 } else { let dt = self.sim_time - sl.time; if dt > 0.0 { (dt * 30.0).min(300.0) } else { 1.0 } };
            let (kc, kd) = (1.0 - 0.94f32.powf(n), 1.0 - 0.975f32.powf(n));
            for k in 0..2 {
                let t = cand.iter().find(|c| c.0 == sl.held[k]);
                let (td, tc, ti) = t.map_or((sl.dir[k], Vec3::ZERO, 0.0), |c| (c.2, c.3, c.4));
                if !sl.init {
                    sl.dir[k] = td;
                    sl.col[k] = tc;
                    sl.int[k] = ti;
                } else {
                    let dot = sl.dir[k].dot(td);
                    sl.dir[k] = if dot.abs() < 0.001 || dot > 0.999_999 {
                        td
                    } else {
                        // turn by kd of the angle between them
                        let ang = dot.clamp(-1.0, 1.0).acos();
                        let axis = sl.dir[k].cross(td);
                        if axis.length_squared() < 1e-12 { td } else { (glam::Quat::from_axis_angle(axis.normalize(), ang * kd) * sl.dir[k]).normalize() }
                    };
                    sl.col[k] += (tc - sl.col[k]) * kc;
                    sl.int[k] += (ti - sl.int[k]) * kc;
                }
            }
            sl.init = true;
            sl.frame = self.frame_no;
            sl.time = self.sim_time;
            if std::env::var("MCLA_TOD_DUMP").is_ok() && self.frame_no == 60 {
                eprintln!("car slots: at {:?} held {:?} best {} {:.4} second {} {:.4} hour w {:.4}; slot0 {:?} {:?} x {:.4}; slot1 {:?} {:?} x {:.4}", at, sl.held, b0, w0, b1, w1, hw, sl.dir[0], sl.col[0], sl.int[0], sl.dir[1], sl.col[1], sl.int[1]);
            }
        }
        // To the shader: `car_key` = the slot that does not hold the hour's light (the lamp),
        // `car_key2` = the one that does (or, where two lamps beat it, the second of them), and
        // which of the two is the programs' first light (the one the collector weighs).
        let hs = if sl.held[0] == HOUR { 0 } else { 1 };
        let ls = 1 - hs;
        let night = nfl > 0.0;
        let lamp_first = if hs == 1 { 1.0 } else { 0.0 };
        let (lc, hc) = (sl.col[ls] * sl.int[ls], sl.col[hs] * sl.int[hs]);
        (
            [sl.dir[ls].x, sl.dir[ls].y, sl.dir[ls].z, img_exposure],
            [lc.x, lc.y, lc.z, lamp_first],
            [sl.dir[hs].x, sl.dir[hs].y, sl.dir[hs].z, if night { 1.0 } else { 0.0 }],
            [hc.x, hc.y, hc.z, 0.0],
        )
    }

    /// The candidates for a car's two light slots where it stands (F-24398): (id, weight, direction
    /// towards it, colour, strength) - from dusk to dawn the lamps and the checkpoint's light, and
    /// always the hour's light, last.
    fn slot_candidates(&self, at: Vec3, nfl: f32, hour_light: (Vec3, Vec3), dynamic: Option<&figlight::Dynamic>) -> Vec<(i32, f32, Vec3, Vec3, f32)> {
        const HOUR: i32 = 65545;
        let mut cand: Vec<(i32, f32, Vec3, Vec3, f32)> = Vec::new();
        if nfl > 0.0 {
            for (i, l) in self.props.lights.iter().enumerate() {
                let d = l.pos - at;
                let d2 = d.length_squared();
                if d2 > 6400.0 || d2 < 1e-4 {
                    continue;
                }
                // (the weight takes the record's cone as the prop lamps' below - F-60154)
                let cone = l.table_cone();
                let s = if l.on() { (l.range / d2).clamp(0.0, 1.0) * ((d / d2.sqrt()).dot(-l.dir) * cone[0] + cone[1]).clamp(0.0, 1.0) } else { 0.0 };
                cand.push((i as i32, s, d / d2.sqrt(), l.color.max(Vec3::ZERO), s));
            }
            // (the lamps the props carry - the street lights. Recorded at the Selma / Highland
            // crossing, car at (735.91, 20.77, -850.46): colour (1, 0.75, 0.55), strength 0.3847,
            // direction (0.382, 0.903, -0.196) - the lamp at (740.6, 31.6, -852.8), Intensity 55,
            // 11.96 m off; at 02:00 the second slot holds a second one, (0.9, 0.8, 0.65) x 0.0983,
            // Intensity 80 at 28.5 m. MCLA_CAR_LAMP_CITY=1: the baked lights only.)
            if std::env::var("MCLA_CAR_LAMP_CITY").is_err() {
                for (i, l) in self.props.prop_lamps.iter().enumerate() {
                    let d = l.pos - at;
                    let d2 = d.length_squared();
                    if d2 > 6400.0 || d2 < 1e-4 || l.intensity <= 0.0 {
                        continue;
                    }
                    let dir = d / d2.sqrt();
                    let s = (l.intensity / d2).clamp(0.0, 1.0) * (dir.dot(l.back) * l.cone[0] + l.cone[1]).clamp(0.0, 1.0);
                    cand.push((30000 + i as i32, s, dir, l.color.max(Vec3::ZERO), s));
                }
            }
        }
        // (a traffic car's candidates - `dynamic`, F-24398's loop over the light manager's dynamic list,
        // ids 65536 + entry, inside the same dusk-to-dawn gate: entries 0 and 1 the player's head lamps,
        // weighed by F-60154 - sat(strength / d^2) x their cone - which is also the strength the slot
        // takes (F-77922); entry 2 the key light, weighed and carried by the luminance of its colour,
        // 0.30 r + 0.59 g + 0.11 b, from 100 km off; the player's own car takes none of them here)
        if let Some(d) = dynamic.filter(|_| nfl > 0.0) {
            for (j, h) in d.heads.iter().enumerate() {
                let Some(h) = h else { continue };
                let s = figlight::light_score(true, h.pos, -h.back, h.strength, h.cone[0], h.cone[1], at);
                let v = h.pos - at;
                cand.push((65536 + j as i32, s, v.normalize_or(Vec3::Y), h.color.max(Vec3::ZERO), s));
            }
            if let Some((col, dir)) = d.key {
                let s = col.max(Vec3::ZERO).dot(Vec3::new(0.30, 0.59, 0.11));
                cand.push((65538, s, dir.normalize_or(Vec3::Y), col.max(Vec3::ZERO), s));
            }
        }
        // (the light manager's dynamic list at night: its fourth record is the race checkpoint's
        // light, id 65539, weighed sat(40 / d squared) all round like the lamps - checkpoints.rs)
        if nfl > 0.0 {
            if let Some((p, c)) = self.cp_markers.light {
                let d = p - at;
                let d2 = d.length_squared();
                if d2 > 1e-4 {
                    let s = (checkpoints::LIGHT_INTENSITY / d2).clamp(0.0, 1.0);
                    cand.push((65539, s, d / d2.sqrt(), Vec3::from(c).max(Vec3::ZERO), s));
                }
            }
        }
        let hw = 2.0 * hour_light.1.max(Vec3::ZERO).dot(Vec3::new(0.30, 0.59, 0.11));
        cand.push((HOUR, hw, hour_light.0, hour_light.1.max(Vec3::ZERO), 1.0));
        cand
    }

    /// The traffic and parked cars' light slots, each car its own (F-99357, run by the ambient cars' pass
    /// dispatcher F-27833 for an instance drawn in the main pass - LOD 0 or more - and again in the blended
    /// bucket's pass). The ids at render instance + 612 / + 660 start at -1 (F-19499, once per instance when
    /// the title builds the pool) and are kept by the player car's rule (F-56246): the two best of F-24398
    /// where the car stands - lamps, the dynamic list (the player's head lamps, the key light, the
    /// checkpoint's light) from dusk to dawn, the hour's light always - a light keeping its slot while it
    /// stays chosen. Each slot is then built by F-77922 (the light's place, colour and strength) and eased
    /// once a call: the direction from the car to the slot's stored point turns 2.5 % of the way to the
    /// light, the point is put 100 m off along it, colour and strength go 6 % of the way; an empty slot (-1)
    /// heads for the world's origin with colour and strength 0. A car not drawn keeps all of it. GAP: the
    /// original's pool - an instance keeps ids and lights from one traffic car to the next, a new car here
    /// starts at -1 and 0 as an instance never drawn. Recorded live: `work/traffic-detail/fix10`.
    fn traffic_light_slots(&mut self) {
        let Some((nfl, hour_light)) = *LAST_HOUR.lock().unwrap() else { return };
        let dynamic = self.peds.night_lights.as_ref().map(|d| d.0);
        let now = self.sim_time;
        let calls = amb_slot_calls();
        let picks: Vec<(usize, Vec<(i32, f32, Vec3, Vec3, f32)>)> = self
            .vehicles
            .cars
            .iter()
            .enumerate()
            .filter(|(i, c)| c.is_ambient() && self.vehicles.lod_pick.get(*i).is_some_and(|&l| l >= 0))
            .map(|(i, c)| (i, self.slot_candidates(c.state.pos, nfl, hour_light, dynamic.as_ref())))
            .collect();
        let drawn: Vec<usize> = picks.iter().map(|p| p.0).collect();
        // (test hook MCLA_AMB_CAND_AT=x,y,z with MCLA_AMB_SLOT_DUMP: the six strongest candidates F-24398 weighs for
        // a car standing there - for comparisons with the slots read from the running original)
        if let (true, Ok(w)) = (std::env::var("MCLA_AMB_SLOT_DUMP").is_ok() && self.frame_no % 30 == 0, std::env::var("MCLA_AMB_CAND_AT")) {
            let v: Vec<f32> = w.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if v.len() == 3 {
                let mut cd = self.slot_candidates(Vec3::new(v[0], v[1], v[2]), nfl, hour_light, dynamic.as_ref());
                cd.sort_by(|a, b| b.1.total_cmp(&a.1));
                for x in cd.iter().take(6) {
                    eprintln!("ambslot-cand {} w {:.4} dir {:?} col {:?} s {:.4}", x.0, x.1, x.2, x.3, x.4);
                }
            }
        }
        for c in self.vehicles.cars.iter_mut() {
            c.amb_light.seen = false;
        }
        for (i, cand) in picks {
            let c = &mut self.vehicles.cars[i];
            let (b0, _, b1, _) = pick_two(&cand);
            c.amb_slot = if slot_swap(c.amb_slot, b0, b1) { [b1, b0] } else { [b0, b1] };
            // (the calls since the last: one a drawn 30 Hz frame and pass; a car drawn again after a gap gets one)
            let l = &mut c.amb_light;
            let frames = if l.drawn_before { ((now - l.time) * 30.0).clamp(0.0, 300.0) } else { 1.0 };
            let n = frames * calls;
            let (kc, kd) = (1.0 - 0.94f32.powf(n), 1.0 - 0.975f32.powf(n));
            let pos = c.state.pos;
            for k in 0..2 {
                // (F-77922's light: a lamp at its place, the hour's light 100 km off along it - here its
                // direction; an empty slot is the default record of F-91871: place 0, colour 0, strength 0)
                let (td, tc, ti) = match cand.iter().find(|x| x.0 == c.amb_slot[k] && c.amb_slot[k] >= 0) {
                    Some(x) => (x.2, x.3, x.4),
                    None => ((-pos).normalize_or(Vec3::Y), Vec3::ZERO, 0.0),
                };
                let from = (l.point[k] - pos).normalize_or(td);
                let d = figlight::turn(from, td, kd);
                l.point[k] = pos + d * 100.0;
                l.col[k] += (tc - l.col[k]) * kc;
                l.int[k] += (ti - l.int[k]) * kc;
            }
            l.time = now;
            l.seen = true;
        }
        for c in self.vehicles.cars.iter_mut() {
            c.amb_light.drawn_before = c.amb_light.seen;
        }
        if std::env::var("MCLA_AMB_SLOT_DUMP").is_ok() && self.frame_no % 30 == 0 {
            for &i in &drawn {
                let c = &self.vehicles.cars[i];
                let l = &c.amb_light;
                eprintln!("ambslot {} {} at {:?} ids {:?} p0 {:?} c0 {:?} s0 {:.4} p1 {:?} c1 {:?} s1 {:.4}", c.model.name, c.uid, c.state.pos, c.amb_slot, l.point[0], l.col[0], l.int[0], l.point[1], l.col[1], l.int[1]);
            }
        }
    }

    /// A key was pressed that no fixed key of the window's handler took: the action the
    /// keyboard's table has for it (`keybinds`; the defaults are the keys named here).
    fn key_action(&mut self, code: KeyCode) {
        use crate::keybinds as kb;
        let Some(act) = self.game.opt_x.ctrl.kbd.binds.action_of(code) else { return };
        // (the actions the port had no key for are heard where the pad's are: driving, no menu)
        let driving = self.drive && !self.game.in_menu() && !self.game.show_map;
        match act {
            // (N)
            kb::MUSIC_UP => {
                if let Some(a) = self.audio.as_mut() {
                    a.next_track();
                }
            }
            kb::MUSIC_DOWN if driving => {
                if let Some(a) = self.audio.as_mut() {
                    a.prev_track();
                }
            }
            // (B)
            kb::MUSIC_PAUSE => {
                if let Some(a) = self.audio.as_mut() {
                    a.music_on = !a.music_on;
                }
            }
            // (F)
            kb::HEADLIGHTS => self.flash = true,
            // (O)
            kb::MAP => self.game.map_button(),
            // (G)
            kb::GARAGE => self.enter_garage = true,
            // (L)
            kb::HYDRAULICS => {
                if self.game.hydraulics && self.drive && !self.game.in_menu() {
                    self.hyd_on = !self.hyd_on;
                    let msg = if self.hyd_on {
                        let b = &self.game.opt_x.ctrl.kbd.binds;
                        let name = |a: usize| b.keys[a][0].and_then(kb::key_name).unwrap_or("-");
                        format!("HYDRAULICS ON   {} {} {} {}", name(kb::WHEEL_FRONT_LEFT), name(kb::WHEEL_FRONT_RIGHT), name(kb::WHEEL_BACK_LEFT), name(kb::WHEEL_BACK_RIGHT))
                    } else {
                        "HYDRAULICS OFF".to_string()
                    };
                    self.game.show_message(&msg);
                }
            }
            // (V) the previous / the next place of the original's list of five (F-67519;
            // the HUD's camera bar shows the new one)
            kb::CAM_UP => self.camera_button(true),
            kb::CAM_DOWN if driving => self.camera_button(false),
            // (E, Q)
            kb::IN_ZONE => self.fire_ability = true,
            kb::ABILITY_NEXT => self.cycle_ability = true,
            // (M, Tab)
            kb::GPS_MAP => self.game.nav_map = true,
            // (R)
            kb::RESET_CAR => {
                if let Some(c) = self.vehicles.player_mut() {
                    let f = c.state.forward();
                    let yaw = (-f.x).atan2(-f.z);
                    c.state = physics::CarState::new(c.state.pos + Vec3::Y * 1.0, yaw);
                    c.pose_history.reset();
                }
            }
            // (no key by default: manual gears when both have one)
            kb::SHIFT_UP if driving => self.game.opt_x.ctrl.kbd.shift_press(1),
            kb::SHIFT_DOWN if driving => self.game.opt_x.ctrl.kbd.shift_press(-1),
            // (the roof the other way, and the option with it - as the pad's action)
            kb::CONVERTIBLE_UP_DN if driving => {
                crate::mc4::option_step(&mut self.game.mc4, &mut self.game.opt_x, 50, true, self.game.fresh_game);
                self.game.dirty = true;
            }
            // (a second key for the pause menu; Escape stays)
            kb::PAUSE if driving && matches!(self.game.mode, Mode::FreeRoam | Mode::Race) => self.game.nav_back = true,
            _ => {}
        }
    }

    fn keyboard_input(&self) -> CarInput {
        // (the keyboard's table, `keybinds`; by default W S A D or the arrows, Space, Left Shift,
        // X / Y the rider's lean, T the weight transfer, U I J K the hydraulics' wheels)
        use crate::keybinds as kb;
        let b = &self.game.opt_x.ctrl.kbd.binds;
        let down = |a: usize| b.down(a, |k| self.input.down(k));
        let mut inp = CarInput::default();
        inp.throttle = if down(kb::GAS) { 1.0 } else { 0.0 };
        inp.brake = if down(kb::BRAKE) { 1.0 } else { 0.0 };
        inp.steer = (if down(kb::STEER_RIGHT) { 1.0 } else { 0.0 }) - (if down(kb::STEER_LEFT) { 1.0 } else { 0.0 });
        inp.handbrake = down(kb::HANDBRAKE);
        inp.nitro = down(kb::NITRO);
        // (X: the rider leans back)
        // (Y: the rider leans forward - on the brake, a stoppie; in the air, nose down)
        inp.lean = if down(kb::LEAN_BACK) { 1.0 } else if down(kb::LEAN_FORWARD) { -1.0 } else { 0.0 };
        // (T: weight transfer - with the steering, at speed, the car goes up on two wheels)
        inp.xfer = down(kb::WEIGHT_XFER);
        // the hydraulics' wheel buttons, laid out like the car: U I in front, J K behind
        for (i, a) in [kb::WHEEL_FRONT_LEFT, kb::WHEEL_FRONT_RIGHT, kb::WHEEL_BACK_LEFT, kb::WHEEL_BACK_RIGHT].into_iter().enumerate() {
            if down(a) {
                inp.hyd |= 1 << i;
            }
        }
        inp
    }

    /// CAM_UP (`up`) / CAM_DOWN (F-67519): the previous / the next of the five views.
    /// Nothing while looking back or while a wreck is watched (the reverse and wipeout cameras
    /// hold the buttons). The new view is the save's DEFAULT CAMERA as well (F-93502).
    fn camera_button(&mut self, up: bool) {
        // (and not while the ability camera has the view, F-38892)
        if self.look_back || self.crash_bits != 0 || self.ability_cam.active() {
            return;
        }
        // (while the event camera runs CAM_UP only ends it and CAM_DOWN does nothing; the
        // Hydraulics, AirRide and Idle views are ended by the chooser's own tests)
        if self.event_dir.running() {
            if up && !matches!(self.event_dir.event(), Some(11..=13)) {
                self.event_dir.leave();
            }
            return;
        }
        self.cam_mode = boomcam::next_view(self.cam_mode, up);
    }

    /// The pad as the original's poll has it (`mc4::PadState`).
    fn pad_state(&self) -> Option<crate::mc4::PadState> {
        let pad = active_pad(self.gamepad.as_ref()?, self.pad_id)?;
        use pads::{Axis as A, Button as B};
        let mut buttons = 0u16;
        for (i, b) in [B::LeftTrigger, B::RightTrigger, B::LeftShoulder, B::RightShoulder, B::North, B::East, B::South, B::West, B::Back, B::LeftStick, B::RightStick, B::Start, B::DPadUp, B::DPadRight, B::DPadDown, B::DPadLeft].into_iter().enumerate() {
            if pad.pressed(b) {
                buttons |= 1 << i;
            }
        }
        Some(crate::mc4::PadState::new([pad.axis(A::LeftX), pad.axis(A::LeftY), pad.axis(A::RightX), pad.axis(A::RightY)], [pad.axis(A::LeftTrigger), pad.axis(A::RightTrigger)], buttons))
    }

    /// The pad's driving input, through the active controller map (`mc4`: the style of the
    /// save, the custom mapping, the hydraulics' own map while they are switched on).
    fn gamepad_input(&mut self, inp: &mut CarInput) {
        let pending = std::mem::take(&mut self.pad_pending);
        let shift = std::mem::take(&mut self.pad_shift);
        if let Some(p) = self.pad_state() {
            let slots = self.game.mc4_controls(self.hyd_on && self.game.hydraulics);
            if !slots.is_empty() {
                // (the look-back rule of F-75699)
                let live = crate::mc4::pad_live(&p, &slots);
                let value = |a: &str| crate::mc4::pad_analog(&p, &live, a);
                let down = |a: &str| crate::mc4::pad_down(&p, &live, a);
                // (a held steering key wins over the pad's stick)
                let steer = value("STEER_AXIS");
                if steer != 0.0 && inp.steer == 0.0 {
                    inp.steer = steer;
                }
                // (the lean axis: the stick pulled back leans the rider back)
                let lean = value("STEER_AXIS_UP_DN");
                if lean != 0.0 && inp.lean == 0.0 {
                    inp.lean = lean;
                }
                let (gas, brake) = (value("GAS"), value("BRAKE"));
                inp.throttle = inp.throttle.max(gas);
                inp.brake = inp.brake.max(brake);
                inp.handbrake |= down("HANDBRAKE") || pending & 1 != 0;
                inp.xfer |= down("WEIGHT_XFER") || pending & 2 != 0;
                inp.nitro |= down("NITRO") || pending & 4 != 0;
                for (i, a) in ["WHEEL_FRONT_LEFT", "WHEEL_FRONT_RIGHT", "WHEEL_BACK_LEFT", "WHEEL_BACK_RIGHT"].into_iter().enumerate() {
                    if down(a) || pending & (8 << i) != 0 {
                        inp.hyd |= 1 << i;
                    }
                }
                // (manual gears are the map's: both shifts bound - F-31769)
                inp.manual = crate::mc4::pad_manual(&slots);
                inp.shift = shift;
                return;
            }
        }
        if let Some(g) = self.gamepad.as_ref() {
            if let Some(pad) = active_pad(g, self.pad_id) {
                use pads::{Axis as A, Button as B};
                let sx = pad.axis(A::LeftX);
                if sx.abs() > 0.12 && inp.steer == 0.0 {
                    inp.steer = sx;
                }
                // (pulling the stick back leans the rider back)
                let sy = pad.axis(A::LeftY);
                if sy.abs() > 0.3 && inp.lean == 0.0 {
                    inp.lean = -sy;
                }
                let rt = pad.axis(A::RightTrigger);
                let lt = pad.axis(A::LeftTrigger);
                if rt > 0.05 {
                    inp.throttle = inp.throttle.max(rt);
                }
                if lt > 0.05 {
                    inp.brake = inp.brake.max(lt);
                }
                // (the buttons as the game's default layout has them - `xenon_controls_trigger`
                // of tune/controller: A handbrake, X nitrous, B weight transfer)
                if self.hyd_on && self.game.hydraulics {
                    // (switched on, the four face buttons are the four wheels, as on the original's pad)
                    for (i, b) in [B::West, B::North, B::South, B::East].into_iter().enumerate() {
                        let bit = match b { B::South => 0, B::East => 1, B::West => 2, _ => 3 };
                        if pad.pressed(b) || pending & (1 << bit) != 0 {
                            inp.hyd |= 1 << i;
                        }
                    }
                } else {
                    if pad.pressed(B::South) || pending & 1 != 0 {
                        inp.handbrake = true;
                    }
                    if pad.pressed(B::West) || pending & 4 != 0 {
                        inp.nitro = true;
                    }
                    if pad.pressed(B::East) || pending & 2 != 0 {
                        inp.xfer = true;
                    }
                }
            }
        }
    }

    /// The pad's other buttons, once a frame, as the game's default layout has them
    /// (`xenon_controls_trigger`: Y headlights, LB special ability, RB camera, the right stick
    /// pulled back looks behind, its click sounds the horn, D-pad down the hydraulics, D-pad
    /// right the next song, Start pauses, Back opens the map); in the menus the D-pad or the
    /// left stick moves, A picks and B goes back.
    fn gamepad_buttons(&mut self) {
        self.pad_flash = false;
        let (now, look, pressed) = {
            let Some(g) = self.gamepad.as_mut() else { return };
            use pads::{Axis as A, Button as B, PadEvent as E};
            let list = [B::South, B::East, B::West, B::North, B::LeftShoulder, B::RightShoulder, B::Back, B::Start, B::LeftStick, B::RightStick, B::DPadUp, B::DPadDown, B::DPadLeft, B::DPadRight];
            let events = g.poll();
            // (the pad played with: of the pads used in the last second - a button, or a stick
            // or trigger past half way - the best ranked, the current one on a tie: one with
            // rumble (XInput, SDL's HIDAPI drivers) first, then one SDL knows the kind of.)
            let now_t = std::time::Instant::now();
            for (id, ev) in &events {
                match ev {
                    E::Connected => {
                        if let Some(p) = g.get(*id) {
                            let (v, d) = p.usb_id();
                            log::info!("pad: {} \"{}\" connected ({}, {v:04x}:{d:04x}, rumble {}): {}", id.0, p.name(), p.kind(), p.has_rumble(), p.mapping());
                        }
                    }
                    E::Pressed(..) => {
                        self.pad_used.insert(*id, now_t);
                    }
                    E::Moved(_, v) if v.abs() > 0.5 => {
                        self.pad_used.insert(*id, now_t);
                    }
                    E::Disconnected => {
                        log::info!("pad: {} disconnected", id.0);
                        self.pad_used.remove(id);
                        if self.pad_id == Some(*id) {
                            self.pad_id = None;
                        }
                    }
                    _ => {}
                }
            }
            let recent = |id: &pads::PadId| self.pad_used.get(id).map_or(false, |t| now_t.duration_since(*t).as_secs_f32() < 1.0);
            let cur_rank = self.pad_id.filter(|id| recent(id)).and_then(|id| g.get(id)).map(|p| p.rank());
            let best = self.pad_used.keys().copied().filter(|id| recent(id)).filter_map(|id| g.get(id).map(|p| (p.rank(), id))).max_by_key(|(r, _)| *r);
            if let Some((rank, id)) = best {
                if cur_rank.map_or(true, |c| rank > c) && self.pad_id != Some(id) {
                    log::info!("pad: playing with {} \"{}\"", id.0, g.get(id).map(|p| p.name()).unwrap_or_default());
                    self.pad_id = Some(id);
                }
            }
            // (the prompts show the glyphs of the pad played with, once it is used - `glyphs`)
            let used = |e: &pads::PadEvent| matches!(e, E::Pressed(..)) || matches!(e, E::Moved(_, v) if v.abs() > 0.5);
            if let Some(pad) = self.pad_id.filter(|id| events.iter().any(|(e, ev)| e == id && used(ev))).and_then(|id| g.get(id)) {
                if crate::glyphs::use_pad(pad.playstation()) {
                    log::info!("prompts: {:?} glyphs ({})", crate::glyphs::device(), pad.kind());
                }
            }
            let mut pressed = 0u32;
            for (id, ev) in &events {
                if Some(*id) == self.pad_id {
                    if let E::Pressed(button) = ev {
                        if let Some(i) = list.iter().position(|b| b == button) {
                            pressed |= 1 << i;
                        }
                    }
                }
            }
            let Some(pad) = active_pad(g, self.pad_id) else { return };
            let mut now = 0u32;
            for (i, b) in list.iter().enumerate() {
                if pad.pressed(*b) {
                    now |= 1 << i;
                }
            }
            let (lx, ly) = (pad.axis(A::LeftX), pad.axis(A::LeftY));
            // (on the GPS map the left stick moves the cursor itself - not the d-pad's CHANGE TARGET)
            let stick_free = self.game.show_map && self.game.hm.is_some();
            for (i, on) in [ly > 0.6 && !stick_free, ly < -0.6 && !stick_free, lx < -0.6 && !stick_free, lx > 0.6 && !stick_free].into_iter().enumerate() {
                if on {
                    now |= 1 << (14 + i);
                }
            }
            (now, (pad.axis(A::RightX), pad.axis(A::RightY)), pressed)
        };
        let hit = (now & !self.pad_prev) | pressed;
        self.pad_prev = now;
        self.pad_look = look;
        self.pad_look_up = 0.0;
        self.pad_horn = now >> 9 & 1 != 0;
        self.pad_flash = now >> 3 & 1 != 0;
        let p = |i: u32| hit >> i & 1 != 0;
        if hit != 0 {
            self.idle = 0.0;
        }
        let watching = self.cine.playing.is_some() || self.cuts.playing.is_some() || self.game.movie.is_some();
        self.game.nav_slow = (now >> 17 & 1) as f32 - (now >> 16 & 1) as f32;
        // (`mc4` step 13b: the pad as it stands, for the mapping editor)
        self.game.opt_x.pad = self.pad_state();
        // (a box that takes the menu buttons outside a menu mode is read as a menu: the keyboard's
        // Enter / Esc reach it in every mode, so the pad's A / B must as well - GitHub #11)
        if self.game.in_menu() || self.game.show_map || watching || self.game.takes_menu_input() {
            self.pad_pending = 0;
            // (the map's actions keep their "down the frame before" while a menu is up: the
            // original's driver update still runs the pad's own, F-75699, when input is shut -
            // A-60769 - so a button held as the menu closes is no fresh press)
            if let Some(st) = self.game.opt_x.pad {
                let slots = self.game.mc4_controls(self.hyd_on && self.game.hydraulics);
                if !slots.is_empty() {
                    self.pad_act_prev = crate::mc4::pad_acts_down(&st, &slots);
                }
            }
            self.game.nav_x |= p(2);
            self.game.nav_y |= p(3);
            self.game.nav_select |= p(6);
            self.game.nav_start |= p(7);
            self.game.nav_up |= p(10) || p(14);
            self.game.nav_down |= p(11) || p(15);
            self.game.nav_left |= p(12) || p(16);
            self.game.nav_right |= p(13) || p(17);
            // LB / RB (the shoulder buttons, indices 4 and 5) step the rim shop's
            // brand tabs, as `uinput.l1` / `uinput.r1` do in the game's own Rim.sc.xml
            self.game.nav_tab_prev |= p(4);
            self.game.nav_tab_next |= p(5);
            // (Start is the start screen's own button - its prompt is "PRESS (Start)"; elsewhere in
            // the menus it backs out, as it closes the pause menu. It used to back out of the
            // start screen too, which quits the game.)
            let title = self.game.mode == Mode::MainMenu;
            self.game.nav_enter |= p(0) || (p(7) && title);
            // (B does nothing there either: the original's start screen has no way out)
            self.game.nav_back |= ((p(1) || p(7)) && !title) && !watching;
            // (Back answers a call on the phone over a menu too - the keyboard's map key does)
            self.game.nav_map |= p(6) && (self.game.show_map || self.game.phone_call_up());
            return;
        }
        // (the active controller map, `mc4`: the driving buttons are its actions. START and BACK
        // stay what they are - the original's editor cannot give them away.)
        let slots = self.game.mc4_controls(self.hyd_on && self.game.hydraulics);
        if let (Some(st), false) = (self.pad_state(), slots.is_empty()) {
            // (`mc4::PAD_ACTS`, through the look-back rule of F-75699: while looking back what
            // shares a button with CAM_PAN_* reads nothing)
            let down = crate::mc4::pad_acts_down(&st, &slots);
            let live = crate::mc4::pad_live(&st, &slots);
            let acted = down & !self.pad_act_prev;
            self.pad_act_prev = down;
            let a = |i: u32| acted >> i & 1 != 0;
            // (LOOKBACK is "is down"; both CAMERA_LEFT and CAMERA_RIGHT down look back too -
            // F-75699; the side look is the two as values)
            let (left, right) = (crate::mc4::pad_analog(&st, &live, "CAMERA_LEFT"), crate::mc4::pad_analog(&st, &live, "CAMERA_RIGHT"));
            let back = down >> 19 & 1 != 0 || (crate::mc4::pad_down(&st, &slots, "CAMERA_LEFT") && crate::mc4::pad_down(&st, &slots, "CAMERA_RIGHT"));
            self.pad_look = (right - left, if back { -1.0 } else { 0.0 });
            self.pad_look_up = crate::mc4::pad_analog(&st, &live, "CAMERA_UP");
            let (pl, pr) = (crate::mc4::pad_down(&st, &live, "CAM_PAN_LEFT"), crate::mc4::pad_down(&st, &live, "CAM_PAN_RIGHT"));
            let pan = pl || pr;
            self.pad_pan = (pan && !self.pad_pan.1, pan);
            self.pad_pan_side = if pl { -1.0 } else if pr { 1.0 } else { 0.0 };
            self.pad_cam_ud = a(10) || a(11);
            self.pad_horn = down >> 18 & 1 != 0;
            self.pad_flash = down >> 9 & 1 != 0;
            self.pad_pending |= acted & 127;
            if !self.drive {
                return;
            }
            self.game.nav_back |= p(7);
            self.game.nav_map |= p(6);
            if a(7) {
                self.game.map_button();
            }
            if a(8) && self.game.hydraulics {
                self.hyd_on = !self.hyd_on;
            }
            if a(9) {
                self.flash = true;
            }
            // (CAM_UP / CAM_DOWN: the next / the previous view - F-67519)
            // (the idle camera is ended by the pad - F-24075, its Idle view - and the press of
            // CAM_UP / CAM_DOWN that ends it does not change the view as well: F-67519 runs
            // after it and passes the press over while that camera goes)
            let woke = self.event_dir.idle() && crate::mc4::pad_wakes_idle(&st, &live);
            if woke {
                self.idle_time = 0.0;
            }
            if (a(10) || a(11)) && !woke {
                self.camera_button(a(10));
            }
            // (CAM_PAN_*: the look-back camera's turn - F-38892, actions 22..25 down)
            self.cam_switch.pan_in = (
                crate::mc4::pad_down(&st, &live, "CAM_PAN_RIGHT") as i32 as f32 - crate::mc4::pad_down(&st, &live, "CAM_PAN_LEFT") as i32 as f32,
                crate::mc4::pad_down(&st, &live, "CAM_PAN_UP") as i32 as f32 - crate::mc4::pad_down(&st, &live, "CAM_PAN_DOWN") as i32 as f32,
            );
            if a(12) || a(13) {
                if let Some(au) = self.audio.as_mut() {
                    if a(12) { au.next_track() } else { au.prev_track() }
                }
            }
            if a(14) {
                self.fire_ability = true;
            }
            // (a shift is a press; one the other way within 0.4 s of the last is passed over -
            // F-56064 at A-82189)
            if crate::mc4::pad_manual(&slots) && (a(15) || a(16)) {
                let way: i8 = if a(15) { 1 } else { -1 };
                if !self.pad_shift_last.map_or(false, |(w, t)| w != way && t.elapsed().as_secs_f32() < 0.4) {
                    self.pad_shift = way;
                    self.pad_shift_last = Some((way, std::time::Instant::now()));
                }
            }
            // (CONVERTIBLE_UP_DN: the roof the other way, and the option with it)
            if a(17) {
                crate::mc4::option_step(&mut self.game.mc4, &mut self.game.opt_x, 50, true, self.game.fresh_game);
                self.game.dirty = true;
            }
            return;
        }
        self.pad_pending |= hit & 15;
        if !self.drive {
            return;
        }
        self.game.nav_back |= p(7);
        self.game.nav_map |= p(6);
        if p(10) {
            self.game.map_button();
        }
        if p(11) && self.game.hydraulics {
            self.hyd_on = !self.hyd_on;
        }
        if p(3) && !(self.hyd_on && self.game.hydraulics) {
            self.flash = true;
        }
        if p(5) {
            self.camera_button(true);
        }
        if p(13) {
            if let Some(a) = self.audio.as_mut() {
                a.next_track();
            }
        }
    }

    fn fly_camera(&mut self, dt: f32) {
        let fast = if self.input.down(KeyCode::ShiftLeft) { 6.0 } else { 1.0 };
        let slow = if self.input.down(KeyCode::ControlLeft) { 0.2 } else { 1.0 };
        let speed = 25.0 * fast * slow;
        let f = self.cam.forward();
        let r = self.cam.right();
        let mut mv = Vec3::ZERO;
        if self.input.down(KeyCode::KeyW) {
            mv += f;
        }
        if self.input.down(KeyCode::KeyS) {
            mv -= f;
        }
        if self.input.down(KeyCode::KeyD) {
            mv += r;
        }
        if self.input.down(KeyCode::KeyA) {
            mv -= r;
        }
        if self.input.down(KeyCode::KeyE) || self.input.down(KeyCode::Space) {
            mv += Vec3::Y;
        }
        if self.input.down(KeyCode::KeyQ) {
            mv -= Vec3::Y;
        }
        // (photo mode's free camera - DECISIONS.md D35 - on the pad as its prompt bar says, through the
        // active map's PHOTO_CAM_* actions: ROTATE (MOVE_*, the right stick) turns it at the original
        // photo camera's rates, 2 and 1.25 rad/s (F-84170); ZOOM IN / OUT (the triggers) moves along the
        // view and PAN (PAN_*, the left stick) in the screen's plane, at the keyboard's speed)
        if self.photo.is_some() && self.game.album.camera_input() {
            let slots = self.game.mc4_controls(false);
            if let (Some(st), false) = (self.pad_state(), slots.is_empty()) {
                let v = |a: &str| crate::mc4::pad_analog(&st, &slots, a);
                self.cam.yaw += 2.0 * dt * (v("PHOTO_CAM_MOVE_RT") - v("PHOTO_CAM_MOVE_LT"));
                self.cam.pitch = (self.cam.pitch + 1.25 * dt * (v("PHOTO_CAM_MOVE_UP") - v("PHOTO_CAM_MOVE_DN"))).clamp(-1.5, 1.5);
                let up = r.cross(f).normalize_or(Vec3::Y);
                mv += f * (v("PHOTO_CAM_ZOOM_IN") - v("PHOTO_CAM_ZOOM_OUT")) + r * (v("PHOTO_CAM_PAN_RT") - v("PHOTO_CAM_PAN_LT")) + up * (v("PHOTO_CAM_PAN_UP") - v("PHOTO_CAM_PAN_DN"));
            }
        }
        self.cam.pos += mv * speed * dt;
    }

    fn update_gameplay(&mut self, dt: f32) {
        self.tick_no += 1;
        self.vehicles.begin_tick();
        // --- input / simulation -------------------------------------------------
        if self.input.rmb {
            self.cam.yaw += self.input.mouse_dx * 0.0025;
            self.cam.pitch = (self.cam.pitch - self.input.mouse_dy * 0.0025).clamp(-1.5, 1.5);
        }
        self.input.mouse_dx = 0.0;
        self.input.mouse_dy = 0.0;
        if self.input.down(KeyCode::BracketRight) {
            self.time_of_day = (self.time_of_day + dt * 2.0) % 24.0;
        }
        if self.input.down(KeyCode::BracketLeft) {
            self.time_of_day = (self.time_of_day - dt * 2.0 + 24.0) % 24.0;
        }
        // the clock runs while driving: one game hour per minute (a day in 24 minutes),
        // unless a time was fixed on the command line
        // (CarSelection holds the clock - Graphics_GetAndSetAutoUpdateTimeOfDay(0) at its start, (1) once ACCEPT's
        // black is up - and the parked cars, AmbientMgr_SetParkedCarsEnable 0 / 1)
        let car_selection = (1..=3).contains(&self.intro_stage) || self.game.starter_pick.is_some();
        self.vehicles.parked_held = car_selection;
        if self.opt.time.is_none() && matches!(self.game.mode, Mode::FreeRoam | Mode::Race) && !car_selection {
            let dh = dt / 60.0;
            self.time_of_day = (self.time_of_day + dh) % 24.0;
        }
        self.weather_tick(dt);
        if std::mem::take(&mut self.cycle_weather) && self.sky_next.is_none() {
            const CYCLE: [sky::Weather; 4] = [sky::Weather::Nice, sky::Weather::Cloudy, sky::Weather::Foggy, sky::Weather::Stormy];
            let i = CYCLE.iter().position(|w| *w == self.weather).unwrap_or(3);
            let w = CYCLE[(i + 1) % CYCLE.len()];
            self.sky_next = Some((sky::Sky::load_weather(&self.opt.data_dir, w), w, 0.0));
            self.game.show_message(&format!("WEATHER: {}", w.name().to_uppercase()));
        }
        let mut player_input = CarInput::default();
        let menu_open = self.game.in_menu();
        if self.drive && !menu_open {
            player_input = self.keyboard_input();
            self.gamepad_input(&mut player_input);
            // (the keyboard's manual gears, `keybinds`: both shift keys bound - beside the pad's
            // rule, which stands as it is)
            let (kbd_manual, kbd_shift) = self.game.opt_x.ctrl.kbd.take_shift();
            if kbd_manual {
                player_input.manual = true;
                if kbd_shift != 0 {
                    player_input.shift = kbd_shift;
                }
            }
            // ---- the cruise assist (mcPlayerInput F-43388): cruising within
            // CruiseActivationSpeedRange of the speed limit with the pedal between
            // CruiseThrottleMin and Max, the game holds the car at the limit - "press the gas
            // lightly to stay under the limit". Under Min the pedal coasts, over Max it is the
            // upper half of the engine's range; a hard brake lets go. ----
            self.cruise_hold = false;
            // Original road accessor F-87157; keep the last limit through
            // junctions without a road owner (F-43388).
            if let (Some(nav), Some(c)) = (self.world.as_ref().and_then(|w| w.nav.as_ref()), self.vehicles.player()) {
                if let Some(speed) = nav.speed_limit_at(c.state.pos) {
                    if (speed / 0.44704 - self.cruise_limit_mph).abs() > 0.01 && std::env::var_os("MCLA_CRUISE_TRACE").is_some() {
                        log::info!("cruise road limit: {:.0} mph at {:?}", speed / 0.44704, c.state.pos);
                    }
                    self.cruise_limit_mph = speed / 0.44704;
                }
            }
            let limit = self.cruise_limit_mph;
            let cruising = self.game.mode == Mode::FreeRoam && self.game.race.is_none() && self.roam.challenge.is_none();
            if let Some(c) = self.vehicles.player() {
                let v = c.state.speed();
                self.cruise_v += (v - self.cruise_v) * 0.2;
                let it = c.model.simtunes.as_ref().map(|s| s.base.input.clone());
                let ok = cruising && c.state.gear > 0 && !c.state.nitro_on() && v > 4.47 && limit > 0.0 && c.state.wheels.iter().any(|w| w.grounded);
                if let (true, Some(it)) = (ok, it) {
                    if player_input.brake < 0.1 {
                        if (self.cruise_v * 2.237 - limit).abs() < it.cruise_activation_speed_range * limit {
                            self.cruise_on = true;
                        }
                    } else if player_input.brake > 0.9 {
                        self.cruise_on = false;
                    }
                    if self.cruise_on && it.cruise_throttle_max > it.cruise_throttle_min && it.cruise_throttle_min > 0.0 {
                        let p = player_input.throttle;
                        let (lo, hi) = (it.cruise_throttle_min, it.cruise_throttle_max.min(0.99));
                        player_input.throttle = if p > lo && p <= hi {
                            self.cruise_hold = true;
                            let e = self.cruise_v - limit * 0.447;
                            let k = (e.abs() * 0.745712).clamp(0.0, 1.0) * 0.4;
                            if e > 0.0 { 0.5 - k } else { 0.5 + k }
                        } else if p <= lo {
                            (p / lo * 0.5).clamp(0.0, 0.5)
                        } else {
                            ((p - hi) * 0.5 / (1.0 - hi) + 0.5).clamp(0.5, 1.0)
                        };
                    }
                } else {
                    self.cruise_on = false;
                }
            }
            // (test hook: MCLA_TWO=1 drives off and goes up on two wheels after 700 ticks; a bike
            // pulls the stick back as well - a wheelie wants the button and stickY > 0.8)
            if std::env::var("MCLA_TWO").is_ok() {
                player_input.throttle = 1.0;
                if self.tick_no > 700 {
                    player_input.xfer = true;
                    player_input.steer = if self.tick_no < 720 { 0.3 } else { 0.0 };
                    if self.vehicles.player().is_some_and(|c| c.model.bike) {
                        (player_input.steer, player_input.lean) = (0.0, 1.0);
                    }
                }
            }
            // (test hook: MCLA_HYD=1 fits the hydraulics, switches them on and drops the front)
            if std::env::var("MCLA_HYD").is_ok() {
                self.game.hydraulics = true;
                self.hyd_on = self.tick_no > 60;
                if self.tick_no > 200 && self.tick_no < 208 {
                    player_input.hyd |= 0b0011;
                }
            }
            if self.game.hydraulics {
                player_input.hyd |= physics::car::HYD_FITTED | if self.hyd_on { physics::car::HYD_ON } else { 0 };
            } else {
                player_input.hyd = 0;
                self.hyd_on = false;
            }
        // (test hook MCLA_COP_TEST=bolt: the throttle alone from 2.5 s into the officer's walk - a burnout, a bolt -
        // then full throttle away)
        if std::env::var("MCLA_COP_TEST").is_ok_and(|v| v == "bolt") && self.ticket_run.is_some() && self.cine.playing.as_ref().is_some_and(|sc| sc.t >= 2.5) {
            player_input.throttle = 1.0;
        }
        // (test hook MCLA_TEST_THROTTLE=<0..1>[,<from tick>]: that much throttle from that tick on - a slow car for the cops)
        if let Ok(v) = std::env::var("MCLA_TEST_THROTTLE") {
            let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if self.tick_no as f32 >= n.get(1).copied().unwrap_or(0.0) {
                player_input.throttle = n.first().copied().unwrap_or(0.0);
            }
        }
        // (test hook: MCLA_GRID_NITRO=1 holds the nitrous button all the time)
        if std::env::var("MCLA_GRID_NITRO").is_ok() {
            player_input.nitro = true;
        }
        // (test hook MCLA_GO_GAS=1: the throttle held down through a race's start)
        if std::env::var("MCLA_GO_GAS").is_ok() && self.game.race.is_some() {
            player_input.throttle = 1.0;
        }
        // (N-VP-1: on the grid, from the intro to GO, the player can only burn out and rev - throttle,
        // brake, handbrake, the steering for the wheels; no nitrous, slipstream turbo, weight shift,
        // hydraulics or gear keys. Special abilities and power-ups are held further down.)
        if self.game.race.as_ref().map_or(false, |r| r.countdown > 0.0) {
            player_input = CarInput { throttle: player_input.throttle, brake: player_input.brake, handbrake: player_input.handbrake, steer: player_input.steer, ai: player_input.ai, ..Default::default() };
        }
        } else if !self.drive {
            self.fly_camera(dt);
        }
        self.own_handbrake = player_input.handbrake;
        // standing with the pedals untouched, the car holds itself (it used to roll away on hills)
        if self.drive && !menu_open && player_input.throttle.abs() < 0.02 && player_input.brake.abs() < 0.02 {
            if self.vehicles.player().map_or(false, |c| c.state.speed() < 1.2) {
                player_input.handbrake = true;
            }
        }
        if self.police.heat == police::Heat::Busted || self.arrest_wait || self.arrest_activate {
            player_input = CarInput { brake: 1.0, handbrake: true, ..Default::default() };
        }
        // (the warp after START: the car is not the player's to drive until it lands - Racer_Activate)
        if self.startwarp.is_some() {
            player_input = CarInput { handbrake: true, ..Default::default() };
        }
        if menu_open && self.game.mode != Mode::Paused {
            // parked behind the menu (a held brake would select reverse at standstill)
            player_input.handbrake = true;
        }
        if self.opt.wot && self.drive {
            // full-throttle straight-line test
            player_input = physics::car::CarInput { throttle: 1.0, ..Default::default() };
            // self-test of the burnout: "seconds on the handbrake, steering"
            if let Ok(v) = std::env::var("MCLA_BURNOUT_TEST") {
                let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                let hold = n.first().copied().unwrap_or(2.0);
                if self.sim_time < 1.0 {
                    player_input = physics::car::CarInput { handbrake: true, ..Default::default() };
                } else if self.sim_time < 1.0 + hold {
                    player_input.handbrake = true;
                    player_input.steer = n.get(1).copied().unwrap_or(0.0);
                }
            } else if self.sim_time < 1.0 {
                player_input = physics::car::CarInput { handbrake: true, ..Default::default() };
            }
        }
        // take world/gpu out of self for the simulation borrow
        let mut world = self.world.take().unwrap();
        let mut gpu = self.gpu.take().unwrap();
        self.vehicles.auto_player = self.drive && self.opt.auto;
        let frozen_now = self.vehicles.frozen || self.vehicles.grid_hold.is_some();
        if self.drive && self.opt.auto {
            if let (Some(nav), Some(car)) = (world.nav.as_ref(), self.vehicles.player_mut()) {
                let route = self.game.race.as_ref().map(|r| r.route.clone());
                let drv = car.ai.get_or_insert_with(|| {
                    let mut d = ai::AiDriver::new(7);
                    d.target_speed = 26.0;
                    d
                });
                if drv.route_override.is_none() && route.as_ref().map_or(false, |r| !r.is_empty()) {
                    drv.route_override = route;
                    drv.link = None;
                }
                let race_path = self.game.race.as_ref().and_then(|r| r.path.clone());
                match (race_path, drv.path.is_some()) {
                    (Some(p), false) => {
                        drv.path = Some(p);
                        drv.path_i = 0;
                        drv.target_speed = std::env::var("MCLA_AUTO_SPEED").ok().and_then(|v| v.parse().ok()).unwrap_or(38.0);
                    }
                    (None, true) => drv.path = None,
                    _ => {}
                }
                if frozen_now {
                    // (held on the grid: that is not being stuck)
                    drv.stuck_long = 0.0;
                    drv.stuck_anchor = car.state.pos;
                }
                if drv.tune.is_none() && car.state.veh.is_some() && std::env::var("MCLA_AI_OLD").is_err() {
                    if let Some(t) = car.model.aitunes.clone() {
                        drv.tune = Some(Arc::new(mcla_formats::rage::aitune::AiTune::lerp(&t.base, &t.mods, car.perf)));
                    }
                }
                player_input = physics::CarInput { ai: true, ..drv.drive(nav, &car.state, dt) };
                if car.state.fwd_speed() < self.auto_last_speed - 8.0 && std::env::var("MCLA_AI_TRACE").is_ok() {
                    // a sudden stop: what stands there?
                    let f = car.state.forward();
                    log::info!("trace: stopped dead at {:.1} {:.1} {:.1} (from {:.1} m/s), wall ahead {:?}, wall low {:?}, props {:?}", car.state.pos.x, car.state.pos.y, car.state.pos.z, self.auto_last_speed, world.collider.raycast(car.state.pos + Vec3::Y * 0.6, f, 6.0).map(|h| (h.t, h.normal)), world.collider.raycast(car.state.pos + Vec3::Y * 0.15, f, 6.0).map(|h| (h.t, h.normal)), self.props.describe_near(car.state.pos, 7.0));
                }
                self.auto_last_speed = car.state.fwd_speed();
                if self.tick_no % 20 == 0 && drv.path.is_some() && std::env::var("MCLA_AI_TRACE").is_ok() {
                    log::info!("trace: pos {:.0} {:.0} {:.0} v {:.1} aim {:.1} thr {:.2} brk {:.2} steer {:.2} pt {} impact {:.2} slide {:.2}", car.state.pos.x, car.state.pos.y, car.state.pos.z, car.state.fwd_speed(), drv.last_v_target, player_input.throttle, player_input.brake, player_input.steer, drv.path_i, car.state.last_impact, car.state.veh.as_ref().map_or(0.0, |v| v.wheels.iter().map(|w| w.slide).fold(0.0, f32::max)));
                }
                if let Some((p, yaw)) = drv.reset_to.take() {
                    log::info!("auto driver: put back on the line");
                    car.state = physics::CarState::new(p, yaw);
                    car.pose_history.reset();
                    car.placed = false;
                }
            }
        }
        if self.opt.find_straight && self.tick_no == 5 {
            // test helper: the longest straight runs of the lane graph
            if let Some(nav) = world.nav.as_ref() {
                let mut best: Vec<(f32, Vec3, f32)> = Vec::new();
                for (i, l) in nav.links.iter().enumerate() {
                    if l.len < 20.0 {
                        continue;
                    }
                    let d0 = l.dir();
                    let mut len = l.len;
                    let mut cur = i;
                    let mut guard = 0;
                    while guard < 200 {
                        guard += 1;
                        let c = &nav.links[cur];
                        let Some(&n) = c.next.iter().find(|&&n| nav.links[n].dir().dot(d0) > 0.9995 && (nav.links[n].a.y - l.a.y).abs() < 6.0) else { break };
                        len += nav.links[n].len;
                        cur = n;
                    }
                    best.push((len, l.a, d0.x.atan2(-d0.z).to_degrees()));
                }
                best.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                for (len, p, yaw) in best.iter().take(8) {
                    log::info!("straight: {:.0} m from {:.1},{:.1},{:.1} yaw {:.1}", len, p.x, p.y + 1.0, p.z, yaw);
                }
            }
        }
        if self.opt.map && self.tick_no == 30 {
            self.game.show_map = true;
            if let Some(c) = self.vehicles.player() {
                for (i, sp, d) in self.game.races_near(c.state.pos, 3) {
                    log::info!("race near: {} at {:?} ({:.0} m)", self.game.race_defs[i].title(), sp, d);
                }
            }
        }
        if self.opt.race && self.tick_no == 90 {
            self.pending_action = MenuAction::StartRace;
        }
        let focus = if self.drive { self.vehicles.player().map(|c| c.state.pos).unwrap_or(self.cam.pos) } else { self.cam.pos };
        // a cutscene plays wherever it was staged
        let focus = self.cine.camera().map(|c| c.0).unwrap_or(focus);
        // (the start screen: the city streamed round its shot's camera - City_DoneStreaming)
        let focus = self.titlecam.eye().unwrap_or(focus);
        // (the ambient manager's cutscene mode, F-97032: a scene playing - AmbientMgr_SetCutscene
        // from its load to its closing transition - and no replay recorded (the port records a race
        // as it runs) or played)
        let recording = self.game.mode == Mode::Race && self.game.race.is_some();
        self.vehicles.amb_cutscene = self.cine.camera().filter(|_| self.game.replay.is_none() && !recording).map(|c| (c.0, c.1));
        // (the ambient manager's front-end mode, F-95688: AmbientMgr_SetFrontEnd from the press-start
        // script while the start screen is up; the viewport's tan(fov / 2) up and across)
        self.vehicles.amb_frontend = self.titlecam.ambient().filter(|_| self.game.mode == Mode::MainMenu);
        if let Some((_, _, lens)) = self.titlecam.view() {
            let ty = (lens.to_radians() * 0.5).tan();
            self.vehicles.amb_frontend_tan = [ty, ty * gpu.config.width as f32 / gpu.config.height.max(1) as f32];
        }
        let t_stream = Instant::now();
        crate::ticktimes::mark(crate::ticktimes::S::World);
        world.update(&mut gpu, focus, 6.0);
        crate::ticktimes::mark(crate::ticktimes::S::Other);
        self.fl_stream_ms += t_stream.elapsed().as_secs_f32() * 1000.0;
        // special abilities (real time); Zone slows the whole simulation down
        let mut fire = std::mem::take(&mut self.fire_ability);
        if let Some(g) = self.gamepad.as_ref() {
            if let Some(pad) = active_pad(g, self.pad_id) {
                fire |= pad.pressed(pads::Button::LeftShoulder);
            }
        }
        // (N-VP-1: nothing fires on the grid before GO - abilities and power-ups wait for the race)
        if self.game.race.as_ref().map_or(false, |r| r.countdown > 0.0) {
            fire = false;
        }
        // (a career has no special ability before its INSTALL_ABILITY mission: the cluster's
        // two dials are empty rings in a new career - `ng_c4`)
        // (or the save says they are fitted: `mc4`, abilities_unlocked)
        // (step 19e: IN_ZONE with a power-up in hand fires it, backwards while LOOKBACK is held;
        // the special ability only gets the press when there is nothing to fire - F-39212)
        if fire {
            if let (Some(pu), Some(p)) = (self.powerups.as_mut(), self.vehicles.cars.iter().position(|c| c.is_player)) {
                if pu.fire(p, self.look_back, &mut self.vehicles, &mut self.ability) {
                    fire = false;
                }
            }
        }
        let sa_ok = (self.game.mc4_abilities_unlocked() && !self.game.mc4_ability_none()) || std::env::var("MCLA_ABILITY").is_ok();
        fire &= sa_ok;
        if let Ok(v) = std::env::var("MCLA_ABILITY") {
            // test helper: trigger the named ability at frame 200
            if self.tick_no == 200 {
                if let Some(k) = ability::Kind::ALL.iter().find(|k| k.name().eq_ignore_ascii_case(&v)) {
                    self.ability.kind = *k;
                    self.ability.meter = 1.0;
                    fire = true;
                }
            }
            if self.tick_no % 60 == 0 && self.tick_no >= 180 {
                let stunned = self.vehicles.cars.iter().filter(|c| c.stunned > 0.0).count();
                log::info!("ability: {} meter {:.2} active {:.1} scale {:.2} stunned cars {}", self.ability.kind.name(), self.ability.meter, self.ability.active, self.ability.time_scale(), stunned);
            }
        }
        if std::mem::take(&mut self.cycle_ability) {
            self.ability.cycle();
            self.game.show_message(&format!("ABILITY: {}", self.ability.kind.name()));
        }
        if matches!(self.game.mode, Mode::FreeRoam | Mode::Race) && !self.vehicles.frozen {
            // the abilities grow with use (levels of the career's ability systems)
            for (i, k) in ability::Kind::ALL.iter().enumerate() {
                let (system, stat) = k.system();
                self.ability.levels[i] = self.game.career.ability_level(system, stat);
            }
            self.ability.cam_step(dt);
            if let Some(k) = self.ability.update(&mut self.vehicles, fire, !self.props.hits.is_empty(), self.crash_bits != 0, dt) {
                // (F-77604: Roar and EMP shake the player's camera as they go off - their
                // tunes' ShakeTime / ShakeScale, roar 0.75 s 0.365, pulse 0.5 s 1)
                match k {
                    ability::Kind::Roar => self.shake_jolts.push((0.75, 0.365)),
                    ability::Kind::Emp => self.shake_jolts.push((0.5, 1.0)),
                    _ => {}
                }
                self.game.show_message(k.name());
                self.game.career.event(career::Ev::Ability);
                let (system, stat) = k.system();
                if let Some(level) = self.game.career.ability_used(system, stat) {
                    // "ZONE LEVEL 2" is the text for reaching level 1, and so on
                    let key = format!("{}_LEVEL_{}", system.to_ascii_uppercase(), level);
                    let text = self.game.career.strings.get(&key).map(|s| s.to_string()).unwrap_or_else(|| format!("{} LEVEL {}", k.name(), level + 1));
                    self.news.push_back((text, None));
                    self.game.write_save(&self.opt.data_dir);
                }
            }
            // (Agro's strikes, F-70024: the user's camera jolt - the tune's ShakeTime x ShakeScale - and the stat gAGH, +1 each)
            let (jolts, strikes) = self.ability.take_agro();
            self.shake_jolts.extend(jolts);
            for _ in 0..strikes {
                self.game.mc4_stat_add("gAGH", 0, 1.0);
            }
        }
        // (step 19e: the race's power-ups - gone with the race, the Racing state's RemovePowerUps)
        if self.game.race.is_none() || !matches!(self.game.mode, Mode::Race) {
            if let Some(mut pu) = self.powerups.take() {
                pu.clear(&mut self.vehicles, &mut self.ability);
            }
            self.vehicles.pu_glows.clear();
        }
        if let Some(pu) = self.powerups.as_mut() {
            let player = self.vehicles.cars.iter().position(|c| c.is_player);
            if !self.vehicles.frozen {
                // (test hook MCLA_POWERUP_TEST="<type>[,<frame>]": the player is handed one)
                if let (Some((kind, at, fire_at, back)), Some(p)) = (powerups::test_hook(), player) {
                    if self.tick_no as u64 == at {
                        if let Some(h) = pu.holders.iter_mut().find(|h| h.car == p) {
                            h.slots = [kind, powerups::NONE];
                            log::info!("powerups: test - the player holds {}", powerups::NAMES.get(kind as usize).unwrap_or(&"?"));
                        }
                    }
                    if fire_at == Some(self.tick_no as u64) {
                        let fired = pu.fire(p, back, &mut self.vehicles, &mut self.ability);
                        log::info!("powerups: test - IN_ZONE pressed, fired {}", fired);
                    }
                }
                // (step 19f: a racer that has finished takes nothing more - F-37974)
                if let Some(r) = self.game.race.as_ref() {
                    let ft = &r.finish_time;
                    pu.set_finished(&|car| ft.get(car).map_or(false, |t| t.is_some()));
                }
                let w = &world;
                pu.update(&mut self.vehicles, &mut self.ability, &|p, up, down| w.ready_at(p).then(|| w.collider.ground(p, up, down).map(|h| h.point)), dt);
                // (step 19g: the sparks at every landing of the last 2 s - pu_fx::sparks)
                pu_fx::sparks(pu, &self.vehicles, &mut self.fx, self.cam.pos);
            }
            self.vehicles.pu_glows = pu.glows();
            if self.tick_no % 300 == 0 {
                if let Some(c) = player.and_then(|p| self.vehicles.cars.get(p)) {
                    let (placed, missed, waiting, near) = pu.census(c.state.pos);
                    log::info!("powerups: {} placed, {} missed, {} waiting; nearest {:.0} m; {} glows", placed, missed, waiting, near, self.vehicles.pu_glows.len());
                }
            }
            for n in std::mem::take(&mut pu.notes) {
                log::info!("powerups: tick {} car {} {:?} {}", self.tick_no, n.car, n.snd, powerups::NAMES.get(n.kind as usize).unwrap_or(&"none"));
                // (only the player's car has the power-up sounds: PACKAGE_POWERUP_PLAYER)
                if Some(n.car) == player {
                    if let Some(a) = self.audio.as_mut() {
                        let pos = self.vehicles.cars.get(n.car).map(|c| c.state.pos);
                        let orig = powerups::object(n.kind, n.snd).is_some_and(|(o, f)| {
                            // the effect's mixer with its in / out sounds
                            if matches!(n.snd, powerups::Snd::In | powerups::Snd::Out) {
                                a.obj_field(&o, 50, None, n.snd == powerups::Snd::In);
                            }
                            a.obj_field(&o, f, pos, true)
                        });
                        if !orig {
                            if let Some(wv) = powerups::wave(n.kind, n.snd) {
                                a.play(&wv, 0.8);
                            }
                        }
                    }
                }
            }
            if let (Some(p), Some(a)) = (player, self.audio.as_mut()) {
                if let Some(h) = pu.holder(p) {
                    if let Some(wv) = h.shot.as_ref().and_then(|s| powerups::loop_wave(s.kind, true)) {
                        a.set_loop(0x5019_0001, &wv, (0.5, 0.5), 1.0);
                    }
                    if let Some(wv) = h.effect.as_ref().and_then(|e| powerups::loop_wave(e.kind, false)) {
                        a.set_loop(0x5019_0002, &wv, (0.5, 0.5), 1.0);
                    }
                }
            }
        }
        self.vehicles.agro = self.ability.agro();
        if let Some(c) = self.vehicles.cars.iter_mut().find(|c| c.is_player) {
            c.state.agro = self.ability.agro();
        }
        self.vehicles.draft = self.vehicles.player().and_then(|c| c.state.veh.as_ref()).map_or(0.0, |v| v.sst.strength);
        // mcCarSSTurbo::Fire F-68670: the player's car (F-14919, its player's +48) counts "gSST" and "nSST", 1 each
        let fires = self.vehicles.player().and_then(|c| c.state.veh.as_ref()).map_or(0, |v| v.sst.fires);
        if fires > self.sst_fires {
            for _ in self.sst_fires..fires {
                self.game.mc4_stat_add("gSST", 0, 1.0);
                self.game.mc4_stat_add("nSST", 0, 1.0);
            }
        }
        self.sst_fires = fires;
        // mcCarNitro's Fire (F-62883): the player's car counts "gNIT" and "nNIT", 1 each - the same gate as the turbo's
        let fires = self.vehicles.player().and_then(|c| c.state.veh.as_ref()).map_or(0, |v| v.nitro_fires);
        if fires > self.nitro_fires {
            for _ in self.nitro_fires..fires {
                self.game.mc4_stat_add("gNIT", 0, 1.0);
                self.game.mc4_stat_add("nNIT", 0, 1.0);
            }
        }
        self.nitro_fires = fires;
        // ---- story scenes ----
        // (once the car stands on its wheels: the world is frozen while a scene runs)
        let on_wheels = self.vehicles.player().map_or(false, |c| c.placed && c.state.wheels.iter().filter(|w| w.grounded).count() >= 3 && c.state.vel.y.abs() < 0.3);
        self.settle_frames = if on_wheels { self.settle_frames + 1 } else { 0 };
        let settled = self.settle_frames > 75;
        // diagnostics: play one scene by name
        if self.tick_no == 5 {
            if let Some(i) = std::env::var("MCLA_CUT").ok().and_then(|n| self.cuts.by_name(&n)) {
                self.cuts.clear_pending();
                self.cuts.queue(i);
            }
        }
        // diagnostics: leave the garage at a given frame
        if let Some(n) = std::env::var("MCLA_GARAGE_EXIT").ok().and_then(|v| v.parse::<u64>().ok()) {
            if self.tick_no as u64 == n && self.game.mode == Mode::Garage {
                self.game.mode = Mode::FreeRoam;
            }
        }
        // diagnostics: Enter pressed at the given ticks (MCLA_NAV_ENTER="300,320")
        if let Ok(v) = std::env::var("MCLA_NAV_ENTER") {
            if v.split(',').any(|t| t.trim().parse::<u64>().ok() == Some(self.tick_no as u64)) {
                self.game.nav_enter = true;
            }
        }
        // diagnostics: lay out a race in the editor and run it
        if std::env::var("MCLA_EDITOR_TEST").is_ok() {
            let f = self.tick_no as i64 - 150;
            match f {
                0 => {
                    self.game.show_map = true;
                    self.game.nav_edit = true;
                }
                10 | 60 | 110 => self.game.nav_enter = true,
                20..=50 if f % 3 == 0 => self.game.nav_up = true,
                70..=100 if f % 3 == 0 => self.game.nav_right = true,
                200 => self.game.nav_go = true,
                _ => {}
            }
        }
        // diagnostics: MCLA_DAMAGE=0..1 sets the player's damage
        if let Some(d) = std::env::var("MCLA_DAMAGE").ok().and_then(|v| v.parse::<f32>().ok()) {
            if let Some(c) = self.vehicles.player_mut() {
                c.state.damage = d;
                // (and a hard knock on the left front corner and one on the right rear door)
                if !c.state.dmg.stamped {
                    let (p, f, r) = (c.state.pos, c.state.forward(), c.state.right());
                    c.state.add_dent(p + f * 1.7 - r * 0.6 + Vec3::Y * 0.5, 20.0);
                    c.state.add_dent(p - f * 0.6 + r * 0.8 + Vec3::Y * 0.6, 12.0);
                }
            }
        }
        // diagnostics: MCLA_WINDOWS=f,r,l,r (0 whole, 1 cracked, 2 broken) sets the player's windows' hit
        // sums (4 and 6: damagemap::CarDamage::window) and MCLA_TYRE_DIRT=0..1 its hubs' scrape sums
        if let Some(c) = self.vehicles.player_mut() {
            if let Ok(v) = std::env::var("MCLA_WINDOWS") {
                // (through the pieces' own hit, so the glass effect and sound play as the sums pass 1 and 2)
                for (i, x) in v.split(',').take(4).enumerate() {
                    let k = x.trim().parse::<u32>().unwrap_or(0);
                    let (want, p) = ([0.0, 4.0, 6.0][k.min(2) as usize], c.state.dmg.pieces[8 + i]);
                    if c.state.dmg.pieces_set && p.hits < want && self.frame_no >= 120 {
                        c.state.dmg.pieces_hit(p.local, want - p.hits, 0);
                    }
                }
            }
            if let Some(d) = std::env::var("MCLA_TYRE_DIRT").ok().and_then(|v| v.parse::<f32>().ok()) {
                for i in 0..4 {
                    c.state.dmg.pieces[i].scrapes = d;
                }
            }
        }
        // ---- crashes (crash_camera_rule.md 4): the player's car wiped out (chassis +592 bit 0x10) or
        // damaged out (0x04); a wreck is put back on the road, repaired, a moment later ----
        {
            // (slow motion: the warpers on real time; the game's scale is the least of them)
            self.warp_wipe.update(dt);
            self.warp_expl.update(dt);
            self.warp_look.update(dt);
            self.wipe_scale = self.warp_wipe.cur.min(self.warp_expl.cur).min(self.warp_look.cur);
            let mission = self.game.race.as_ref().map_or(false, |r| r.kind != game::RaceKind::Race);
            if self.wreck_engine == 3 {
                // "impact start car done" with no pause: its exit after more than 5 s in it (F-38554, +156 += dt by
                // F-71604) starts the engine sound with 0 (F-85478, vt+52). (approx.: counted through scenes and
                // menus too - the driver update's dt there was not traced)
                self.wreck_done += dt;
                if self.wreck_done > 5.0 {
                    self.wreck_engine = 0;
                    self.sound.play_ignition();
                }
            }
            if matches!(self.game.mode, Mode::FreeRoam | Mode::Race) && !mission && self.cine.playing.is_none() {
                if self.wreck_timer > 0.0 {
                    self.wreck_timer -= dt;
                    // (still damaged out: 0x04 stands until the reset below)
                    if let Some(c) = self.vehicles.player_mut() {
                        c.stunned = c.stunned.max(0.5);
                    }
                    // the driver's "impact start car" (F-80396): entered when DRV_HIT_HRD has under 0.033 s left
                    // (F-84327), it lasts 4 s (F-76894). Its enter stops the engine sound with 1 (vt+56) unless the
                    // police bust is on (chassis +592 bit 0x20)
                    if self.wreck_engine == 1 && self.wreck_timer < 4.0 + 0.033 {
                        self.wreck_engine = 0;
                        if self.crash_bits & 0x20 == 0 {
                            self.sound.stop_engine();
                            self.wreck_engine = 2;
                        }
                    }
                    // start car's exit (F-47123) fires the wreck's "what next" - the pause menu - unless the police
                    // manager's target (+68) is the player (F-79134). "impact start car done" then ends at its first
                    // update with the game paused (F-38554: the flags' 0x20000000, F-28290 = the cine layer's +320,
                    // set while the pause manager pauses, F-94276), else after 5 s in it (+156 += dt, F-71604); its
                    // exit (F-85478) is the engine sound's start with 0 (vt+52). The reset below stands for that
                    // pause, so the start goes with it; with the police on the player there is neither and the car
                    // stays damaged out
                    if self.wreck_timer <= 0.0 && self.game.police_on_player {
                        if self.wreck_engine == 2 {
                            self.wreck_engine = 3;
                            self.wreck_done = 0.0;
                        }
                    } else if self.wreck_timer <= 0.0 {
                        if std::mem::take(&mut self.wreck_engine) == 2 {
                            self.sound.play_ignition();
                        }
                        // the wreck event's end (F-79134 -> F-89773). (approx.: the original then opens the
                        // PauseMenu in the cruise - its QUICK FIX row, free - or ends the race into the
                        // race-over menu; the port has neither row yet, so the car is set going here,
                        // free, as that QUICK FIX would)
                        if let Some(c) = self.vehicles.player_mut() {
                            let f = c.state.forward();
                            let gear = c.state.gear;
                            c.state = physics::CarState::new(c.state.pos + Vec3::Y * 0.8, (-f.x).atan2(-f.z));
                            c.pose_history.reset();
                            c.state.gear = gear;
                            c.placed = false;
                            // (the reset's F-30391(0): the flags +592 to 0, so the damaged-out hold ends; the
                            // engine word +46 = 1)
                            c.stunned = 0.0;
                            c.engine_off = false;
                            c.deactivated = false;
                        }
                        self.crash_bits = 0;
                        self.crash_exploding = false;
                        // (the player's reset puts the warpers back, F-86999)
                        self.warp_wipe.reset();
                        self.warp_expl.reset();
                        self.warp_look.reset();
                    }
                } else if let Some(c) = self.vehicles.player_mut() {
                    let dm = c.model.simtunes.as_ref().map(|s| s.base.damage.clone()).unwrap_or_default();
                    let speed = c.state.speed();
                    let up_y = (c.state.rot * Vec3::Y).y;
                    if let Ok(v) = std::env::var("MCLA_CRASH_TEST") {
                        // (test hook "secs,w|d|x": at that time a wipeout blow, a fatal blow, or an
                        // exploding one)
                        let mut it = v.split(',');
                        let at = it.next().and_then(|x| x.parse::<f32>().ok()).unwrap_or(5.0);
                        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                        if self.sim_time >= at && !DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                            match it.next().unwrap_or("w") {
                                "d" => {
                                    c.state.damage = 1.0;
                                    c.state.last_hit = dm.fatal_blow + 1.0;
                                }
                                "x" => {
                                    c.state.damage = 1.0;
                                    c.state.last_hit = dm.explode_threshold + 1.0;
                                }
                                _ => c.state.wipeout = 2.0,
                            }
                        }
                    }
                    let big = std::mem::take(&mut c.state.wipeout);
                    if self.crash_bits == 0 {
                        // a head-on into the nose of a traffic car or into something fixed above
                        // 22.35 m/s (F-43509, physics::headon, which added its damage): a wipeout
                        // and the slow motion
                        let head_on = std::mem::take(&mut c.state.head_on);
                        // rolled onto its side or roof on the ground (mcCar::Update A-72058): 0.2 of
                        // MaxDamage and a wipeout
                        let rolled = up_y <= 0.1 && c.state.airborne_time <= 0.0 && c.state.damage < 0.999;
                        if head_on {
                            log::info!("crash: head-on wipeout at {:.2} s, speed {:.2}, damage {:.3}", self.sim_time, speed, c.state.damage);
                            let now = self.wipe_scale;
                            self.warp_wipe.start(now, 0.0);
                        } else if rolled {
                            c.state.damage = (c.state.damage + 0.2).min(1.0);
                        }
                        if c.state.damage >= 0.999 {
                            // damaged out (F-70073 after F-69183): its timer DamageOutDelay, or
                            // ExplosionDelay when the destroying hit was over ExplodeThreshold
                            self.crash_exploding = c.state.last_hit > dm.explode_threshold && dm.explode_threshold > 0.0;
                            self.crash_bits = 0x14;
                            self.crash_timer = if self.crash_exploding { dm.explosion_delay } else { dm.damage_out_delay };
                            // (held while 0x04 stands - the branch below; the reset or QUICK FIX clears it)
                            c.stunned = c.stunned.max(0.5);
                            let p = c.state.pos;
                            self.fx.sparks(p + Vec3::Y * 0.6, Vec3::Y, 60);
                        } else if head_on || rolled || (c.model.bike && big > 1.8) {
                            // a wipeout (F-69183): its timer WipeoutDelay. (the hit's: a bike's
                            // blow of more than 1.8 times the WipeoutThreshold, F-54299 - a car
                            // wipes out on a hit only through the head-on rule)
                            self.crash_bits = 0x10;
                            self.crash_timer = dm.wipeout_delay;
                            c.stunned = c.stunned.max(1.2);
                            c.state.upset = 1.0;
                        }
                    } else {
                        // mcCarDamage +44 runs down on real time while the car is under 4.47 m/s or
                        // all its wheels touch (F-32386)
                        // (only while it is above 0: it goes below 0 once)
                        let all_down = !c.state.wheels.is_empty() && c.state.wheels.iter().all(|w| w.grounded);
                        if self.crash_timer > 0.0 && (speed < 4.47 || all_down) {
                            self.crash_timer -= dt;
                        }
                        if self.crash_bits & 4 != 0 {
                            c.stunned = c.stunned.max(0.5);
                            if self.crash_timer < 0.0 {
                                // the wreck tick (mcCar::Update F-53665): the timer to 0; the
                                // wrecked notice (F-58200 -> F-16795: DAMAGED OUT in the splash slot
                                // for 2 s); the driver's hit (DRV_HIT_HRD to its end) then 4 s of
                                // DRV_IDLE_START, at whose end the wreck's "what next" fires (F-47123
                                // -> F-79134) - not while the police have the player as their target, tested
                                // there; the driver's states run either way
                                self.crash_timer = 0.0;
                                let m = self.game.career.strings.get("splash_damaged_out").map(|s| s.to_uppercase()).unwrap_or_else(|| "DAMAGED OUT".into());
                                self.game.show_message(&m);
                                self.game.message_timer = 2.0;
                                // (approx.: a driver not loaded leaves the hit clip at 0)
                                self.wreck_timer = self.drivers.hit_hard_secs(&c.model.name).unwrap_or(0.0) + 4.0;
                                // the driver goes to "impact damageout" only right side up (IsUpsideDown, the
                                // flags' 0x800 from F-28290: [?] read as the car's up axis y <= 0); upside down
                                // it takes "impact damageout inverted", which never stops the engine
                                self.wreck_engine = if up_y > 0.0 { 1 } else { 0 };
                            }
                        } else if self.crash_bits & 0x10 != 0 && self.crash_timer <= 0.0 {
                            // the wipeout is over (mcCar::Update A-23262): a car lying over (up.y
                            // under 0.7) is stood up where it is
                            self.crash_bits = 0;
                            c.state.upset = 0.0;
                            if up_y < 0.7 {
                                let f = c.state.forward();
                                let fresh = physics::CarState::new(c.state.pos + Vec3::Y * 0.8, (-f.x).atan2(-f.z));
                                let old = std::mem::replace(&mut c.state, fresh);
                                c.state.damage = old.damage;
                                c.state.zones = old.zones;
                                c.state.dmg = old.dmg;
                                c.state.gear = old.gear;
                                c.pose_history.reset();
                                c.placed = false;
                            }
                        }
                    }
                }
            } else {
                self.wreck_timer = 0.0;
                // (approx.: a scene, a mission or a menu cutting "impact start car" short - the original's driver
                // goes back to cruising on IsRaceFinish without the start. The engine sound's state is written only by
                // its start (vt+52), its stop (vt+56), the restore's set-running (vt+48, the QUICK FIX) and a new car's
                // constructor, so it stays stopped until one of them; which of them follows the race's end was not
                // traced - here the start)
                match self.wreck_engine {
                    1 => self.wreck_engine = 0,
                    2 => {
                        self.wreck_engine = 0;
                        self.sound.play_ignition();
                    }
                    _ => {}
                }
                self.crash_bits &= 0x28;
            }
            // the bust (0x20, with 0x08: no damage hits) is cleared by the original's flag reset after the
            // arrest or the ticket [?: which one]; here when the arrest's hold, its scene and the
            // ticket / arrest panel are all over (approx.)
            if self.crash_bits & 0x20 != 0 && self.police.heat != police::Heat::Busted && self.cine.playing.is_none() && self.game.arrest_panel.is_none() {
                self.crash_bits &= !0x28;
            }
        }
        // diagnostics: enter photo mode and take a picture
        // self-test: fit the rim size of MCLA_RIM as the garage would
        if self.tick_no == 100 && std::env::var("MCLA_RIM").is_ok() {
            self.game.selected_car = self.game.current_car;
            self.pending_action = MenuAction::ApplyCar;
        }
        // parts bought in the performance shop go onto the car at once
        if std::mem::take(&mut self.game.perf_dirty) {
            let tuned = self.game.perf_frac(self.game.current_car);
            if let Some(p) = self.vehicles.player_mut() {
                p.set_perf(tuned);
            }
            self.game.write_save(&self.opt.data_dir);
        }
        if let Some(k) = std::env::var("MCLA_PERF_SHOP").ok().and_then(|v| v.parse::<usize>().ok()) {
            if self.game.mode == Mode::Garage && self.tick_no > 60 {
                self.game.perf_shop = Some(k);
            }
        }
        // self-test: MCLA_VINYL_EDIT="zone,row" opens the vinyl editor on a preset
        if let Ok(v) = std::env::var("MCLA_VINYL_EDIT") {
            if self.game.mode == Mode::Garage && self.tick_no > 60 && self.game.vinyl_edit.is_none() && !self.game.vinyl_for.is_empty() {
                let f: Vec<usize> = v.split(",").filter_map(|x| x.trim().parse().ok()).collect();
                let car = self.game.vinyl_for.clone();
                if let Some(p) = self.game.vinyls.get(&car).and_then(|v| v.strip_prefix("preset:").map(|p| p.to_string())) {
                    self.game.liveries.insert(car.clone(), self.game.vinyl_presets.livery(&p));
                    self.game.livery_rev += 1;
                    self.game.vinyls.insert(car, "custom".to_string());
                }
                self.game.vinyl_edit = Some(game::VinylEdit { page: if std::env::var("MCLA_VINYL_PAGE").is_ok() { 0 } else { 9 }, zone: f.first().copied().unwrap_or(1).min(4), layer: 0, row: f.get(1).copied().unwrap_or(0), ..Default::default() });
            }
        }
        // self-test: highlight a row of the garage menu
        if let Some(r) = std::env::var("MCLA_GARAGE_ROW").ok().and_then(|v| v.parse::<usize>().ok()) {
            if self.game.mode == Mode::Garage {
                self.game.garage_row = r;
                if let (Some(v), Some(slot)) = (std::env::var("MCLA_GARAGE_VAL").ok().and_then(|v| v.parse::<usize>().ok()), vehicle_model::CUSTOM_SLOTS.get(r.wrapping_sub(1))) {
                    self.game.cfg.set(*slot, v);
                }
            }
        }
        // self-test: open the pause menu on a tab / page ("tab,page,sel")
        // (the original's music lists, `mc4`: made once, with or without an audio device)
        if self.game.opt_x.music.is_none() {
            let _ = self.game.mc4_music();
        }
        // (test hook for the keyboard's table, `keybinds`: MCLA_KEY_TEST="from-to:NAME,..." - the key
        // NAME (as the keyboard page names it, `_` for a space) is pressed at tick `from` and let
        // go after tick `to`; where the player's car is then is logged)
        // (test hook MCLA_NAV_TEST="frame:a,frame:y,...": the pad's A / Y / START / B / up / down / left / right pressed at that frame)
        // (at the first tick on or after that frame: in real time a frame may run no tick)
        if let Ok(v) = std::env::var("MCLA_NAV_TEST") {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEEN: AtomicU64 = AtomicU64::new(0);
            let seen = SEEN.swap(self.frame_no, Ordering::Relaxed);
            for step in v.split(',') {
                let Some((at, b)) = step.trim().split_once(':') else { continue };
                if at.trim().parse::<u64>().ok().is_some_and(|at| at > seen && at <= self.frame_no) {
                    log::info!("nav test: {} at frame {}", b, self.frame_no);
                    match b.trim() {
                        "a" => self.game.nav_enter = true,
                        "y" => self.game.nav_y = true,
                        "start" => self.game.nav_start = true,
                        "b" => self.game.nav_back = true,
                        "up" => self.game.nav_up = true,
                        "down" => self.game.nav_down = true,
                        "left" => self.game.nav_left = true,
                        "right" => self.game.nav_right = true,
                        _ => {}
                    }
                }
            }
        }
        if let Ok(v) = std::env::var("MCLA_KEY_TEST") {
            for step in v.split(',') {
                let Some((at, name)) = step.trim().split_once(':') else { continue };
                let Some(code) = crate::keybinds::key_from_name(&name.replace('_', " ")) else { continue };
                let (from, to) = at.split_once('-').unwrap_or((at, at));
                let (Ok(from), Ok(to)) = (from.trim().parse::<u64>(), to.trim().parse::<u64>()) else { continue };
                if self.tick_no == from {
                    self.input.keys.insert(code);
                    self.key_action(code);
                }
                if self.tick_no == to + 1 {
                    self.input.keys.remove(&code);
                    let (pos, speed, input) = self.vehicles.player().map_or((Vec3::ZERO, 0.0, CarInput::default()), |c| (c.state.pos, c.state.speed(), c.input));
                    log::info!("key test: {} held {}..{}: car at {:.1} {:.1} {:.1}, speed {:.2}, throttle {:.1} brake {:.1} steer {:.1} handbrake {} nitro {} manual {} | camera {} hyd {} map big {}", name, from, to, pos.x, pos.y, pos.z, speed, input.throttle, input.brake, input.steer, input.handbrake, input.nitro, input.manual, self.cam_mode, self.hyd_on, self.game.hud_map_big);
                }
            }
        }
        if let Ok(v) = std::env::var("MCLA_PAUSE_TEST") {
            // ("tab,page,row[,tick]": the pause at that tick, 60 by default)
            let n: Vec<usize> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if self.tick_no >= n.get(3).copied().unwrap_or(60) as u64 && matches!(self.game.mode, Mode::FreeRoam | Mode::Race) && !DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                self.game.mode = Mode::Paused;
                self.game.pause_tab = n.first().copied().unwrap_or(0);
                self.game.pause_page = n.get(1).copied().unwrap_or(0) as u8;
                self.game.menu_sel = n.get(2).copied().unwrap_or(0);
            }
        }
        // (test hook: MCLA_TEST_DRIVE_CAR=<vehicle> - the showroom's TEST DRIVE in that vehicle at
        // tick 200, as its X does)
        if self.tick_no == 200 {
            if let Ok(name) = std::env::var("MCLA_TEST_DRIVE_CAR") {
                let i = self.game.car_dirs.iter().position(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&name)));
                if i.is_some() {
                    self.game.test_drive = i;
                    self.pending_action = MenuAction::TestDrive;
                }
            }
        }
        if let Ok(path) = std::env::var("MCLA_PHOTO_TEST") {
            if self.tick_no == 200 {
                self.pending_action = MenuAction::PhotoMode;
            }
            if self.tick_no == 230 {
                self.cam.pos += Vec3::Y * 2.0;
                // (step 23b: MCLA_PHOTO_TEST=album takes the picture into the album)
                self.photo_request = Some(if path == "album" { self.game.mc4b_photo_path() } else { path });
            }
        }
        // (test hook: MCLA_ALBUM_TEST="<presses>" - photo mode at tick 200, then the presses of
        // `photoalbum`'s hook; MCLA_ALBUM_VIEW starts in the album, as the MODES tab's PHOTO ALBUM)
        if std::env::var("MCLA_ALBUM_TEST").is_ok() && self.tick_no == 200 && self.photo.is_none() {
            self.pending_action = if std::env::var("MCLA_ALBUM_VIEW").is_ok() { MenuAction::PhotoAlbum } else { MenuAction::PhotoMode };
        }
        // ---- photo mode ----
        // (the album's flow asks for the frame, sounds and the way out - `photoalbum`)
        if self.photo.is_some() {
            if self.photo_request.is_none() {
                self.photo_request = self.game.album.capture.take().or_else(|| self.game.album.debug_shot.take());
            }
            for n in self.game.album.sounds.drain(..) {
                if let Some(a) = self.audio.as_mut() {
                    a.obj(n);
                }
            }
            if std::mem::take(&mut self.game.album.leave) {
                if let Some((_, drive)) = self.photo.take() {
                    self.drive = drive;
                    self.game.photo_on = false;
                    self.game.photo = None;
                }
            }
        }
        if self.photo.is_some() {
            self.photo_msg -= dt;
            self.game.photo = if self.photo_request.is_some() {
                None
            } else if self.photo_msg > 0.0 {
                Some("PHOTO SAVED".to_string())
            } else {
                Some("WASD  MOVE      Q / E  DOWN / UP      RIGHT MOUSE  LOOK      ENTER  TAKE PHOTO      ESC  BACK".to_string())
            };
        }
        // ---- movies ----
        if std::mem::take(&mut self.game.movie_done) {
            if let Some(a) = self.audio.as_mut() {
                a.play_stream(None, 0.0);
            }
            self.idle = 0.0;
        }
        if self.tick_no == 5 {
            if let Ok(name) = std::env::var("MCLA_MOVIE") {
                if !name.is_empty() && name != "1" {
                    self.start_movie(&name, Vec::new(), name == "attract");
                }
            }
        }
        // the start-up movie (boot.rs): its frames, loading done (`Game_SetLoadingDone` after `City_DoneStreaming`:
        // the game is loaded - bootload.rs held the movie until then - and no sector is waiting to be read), the logo
        // hidden while the attract movie plays, the loading music from its frame 340 on
        if self.game.mode != Mode::MainMenu {
            self.game.boot = None;
        }
        let attract_on = self.game.movie.is_some();
        let streamed = world.streaming() == 0;
        if let Some(b) = self.game.boot.as_mut() {
            b.update(dt, streamed, attract_on);
            if b.music() && !attract_on {
                if let Some(a) = self.audio.as_mut() {
                    a.obj_loop(boot::LOADING_MUSIC);
                }
            }
        }
        // After START the loading music goes on (the press-start script does not stop it) until the career stops it:
        // with a save the career script's AudioManager_StopLoadingMusic (0x64d) just before the flight down - the radio
        // (mcMusicManager_Play, 0x6e0) after it; a new career's CarSelection stops it (0x8a4) just before the film, and
        // plays MUSIC_CARSELECT_MASTER from VEHICLE SELECT's fade up (PlayCustomMusic, 0xef4) to its end
        // (StopCustomMusic, 0x149b, after AmbientMgr_Reset)
        let loading_on = self.startwarp.as_ref().is_some_and(|w| self.game.fresh_game || w.phase() != startwarp::Phase::Down);
        let carselect_on = self.intro_stage == 3 || (self.intro_stage == 4 && self.game.starter_pick.is_some());
        if let Some(a) = self.audio.as_mut() {
            if loading_on {
                a.obj_loop(boot::LOADING_MUSIC);
            }
            if carselect_on {
                a.obj_loop(CARSELECT_MUSIC);
            }
        }
        // (the radio waits: through the flight down, and for a new career until its opening is over)
        // (and through a story scene: the cutscene's own sound only)
        self.game.radio_held = self.startwarp.is_some() || self.game.fresh_game || self.intro_stage != 0 || self.cuts.playing.is_some();
        // nobody at the start screen: the attract movie (the press-start script: TIMERA > 110000 ms, the timer
        // started when the start screen takes START and again after each attract movie; other buttons do not reset it)
        let title_up = self.game.boot.as_ref().map_or(true, |b| b.title);
        self.idle = if self.game.mode == Mode::MainMenu && self.game.movie.is_none() && title_up { self.idle + dt } else { 0.0 };
        if self.idle > boot::ATTRACT_IDLE && self.opt.shot.is_none() {
            self.idle = 0.0;
            self.start_movie("attract", Vec::new(), true);
        }
        // (a new career's first scene waits for ACCEPT's black and the car's hand-over - `opening_tick`)
        if self.game.mode == Mode::FreeRoam && self.game.race.is_none() && settled && self.roam.challenge.is_none() && self.game.movie.is_none() && self.game.starter_pick.is_none() {
            if let Some(stream) = self.cuts.start_pending() {
                self.speech.stop();
                // the scene's own animation (camera, characters, cars), if the disc has it
                let scene = self.cuts.playing.as_ref().map(|p| format!("story/{}", p.def.name.to_ascii_lowercase())).unwrap_or_default();
                if std::env::var("MCLA_NO_CINE").is_err() && self.cine.start(&mut gpu, &scene, glam::Mat4::IDENTITY) {
                    self.cine_spawn_cars(&mut gpu, &mut world);
                    // the scene lasts as long as its animation
                    if let (Some(p), Some(sc)) = (self.cuts.playing.as_mut(), self.cine.playing.as_ref()) {
                        p.def.length = sc.duration;
                        // diagnostics: start part-way in
                        if let Some(t) = std::env::var("MCLA_CUT_T").ok().and_then(|v| v.parse::<f32>().ok()) {
                            p.t = t;
                        }
                    }
                    self.cine_hold = Some((stream, 0));
                } else {
                    self.start_scene_audio(stream.as_deref());
                }
            }
        }
        if let Some((stream, n)) = self.cine_hold.take() {
            // black until the place has streamed in, then the dialogue starts with the picture
            if (n >= 30 && world.streaming() == 0) || n > 400 {
                self.start_scene_audio(stream.as_deref());
            } else {
                self.cine_hold = Some((stream, n + 1));
            }
        }
        if self.cuts.playing.is_some() && self.cine_hold.is_none() {
            // (the player's skip is UI event 63 - Y, prompt button 4 - and does not end the scene: the cine
            // manager's request +305 jumps its clock to the scene's bookmark and it plays on from there
            // (F-86493 -> F-80555 -> F-19408 / F-61676, `cine::skip_to`); A does nothing)
            std::mem::take(&mut self.game.nav_enter);
            if std::mem::take(&mut self.game.nav_y) {
                let at = self.cine.playing.as_ref().and_then(|sc| cine::skip_to(&sc.name, sc.t, sc.camera_len()));
                if let (Some(at), Some(p)) = (at, self.cuts.playing.as_mut()) {
                    log::info!("cutscene: skip from {:.2} s to the bookmark at {:.2} s", p.t, at);
                    p.t = at;
                    // (the scene's streams go to the same time; the stand-in film sound of INTRO FAST stops)
                    if let Some(a) = self.audio.as_mut() {
                        a.obj_cutscene_seek(at);
                        a.play_stream(None, 0.0);
                    }
                }
            }
            let skip = self.game.mode != Mode::FreeRoam;
            let name = self.cuts.playing.as_ref().map(|p| p.def.name.clone()).unwrap_or_default();
            if !self.cuts.update(dt, skip) {
                // (the scene's audio ends with it, F-27469)
                if let Some(a) = self.audio.as_mut() {
                    if std::mem::take(&mut self.cut_audio_objects) {
                        a.obj_cutscene(&name, false, false);
                    }
                    a.play_stream(None, 0.0);
                }
            }
        }
        self.opening_tick(dt, world.streaming());
        // ---- hangouts: the people there, pulling in, leaving ----
        self.cine.tick(dt);
        // an in-game scene's audio (core_rolling_prototype): its stream prepared by the name its script gives
        // (F-27055), played as the scene plays (F-16075 -> F-58551: the speech and the police radio stop, flags 0x3
        // on every one of them), stopped with the scene (F-19749)
        let want = self.cine.playing.as_ref().and_then(|s| cine::scene_audio(&s.name));
        if want != self.cine_audio && self.cuts.playing.is_none() {
            if let Some(a) = self.audio.as_mut() {
                if let Some(old) = self.cine_audio.take() {
                    a.obj_cutscene(&old, false, false);
                }
                if let Some(n) = &want {
                    if a.obj_cutscene(n, true, true) {
                        self.speech.stop();
                        self.radio.stop();
                    }
                }
            }
            self.cine_audio = want;
        }
        {
            let pp = self.vehicles.player().map(|c| (c.state.pos, c.state.speed()));
            let near = pp.and_then(|(p, _)| self.hangouts.iter().enumerate().map(|(i, h)| (i, h.1.distance(p))).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()));
            let name = near.filter(|n| n.1 < 170.0).map(|n| self.hangouts[n.0].0.clone());
            // (their people's character sets read ahead from 250 m - cine.rs `read_ahead_ambient`)
            if near.is_some_and(|n| n.1 < 250.0) {
                self.cine.read_ahead_ambient();
            }
            self.cine.set_ambient(&mut gpu, name.as_deref());
            let free = self.game.mode == Mode::FreeRoam && self.game.race.is_none() && self.roam.challenge.is_none() && self.cine.playing.is_none() && self.cuts.playing.is_none() && self.game.movie.is_none();
            // (the hangout cores' entry lines: the player's move of this frame - `hangoutin::crossed`; the
            // port's hangout of that set-up by its scenes' name)
            let cur = pp.map(|(p, _)| p);
            let line = match (self.hangout_prev, cur) {
                (Some(a), Some(b)) => hangoutin::crossed_any(a, b),
                _ => None,
            };
            self.hangout_prev = cur;
            // (port guard: only a hangout whose scene parks the car near its line - South Central's scenes' car
            // poses are not in the city's frame in the port, CUTSCENES.md)
            let line = line.and_then(|s| self.hangouts.iter().position(|h| hangoutin::SETUPS[s].scene.starts_with(&format!("{}_", h.0))).filter(|&j| self.hangouts[j].1.distance(hangoutin::SETUPS[s].line[0]) < 60.0).map(|j| (s, j)));
            if let (Some((i, d)), Some((_, speed))) = (near, pp) {
                if self.tick_no % 120 == 0 && std::env::var("MCLA_HANGOUT_TRACE").is_ok() {
                    log::info!("hangout trace: nearest {} (spot {:?}) at {:.1} m, speed {:.1}, free {} (race {} challenge {} cine {} cuts {} movie {}), in {:?}, armed {}, mode {:?}", self.hangouts[i].0, self.hangouts[i].1, d, speed, free, self.game.race.is_some(), self.roam.challenge.is_some(), self.cine.playing.is_some(), self.cuts.playing.is_some(), self.game.movie.is_some(), self.hangout_in, self.delivery_armed, self.game.mode as u8);
                }
                if let Some(h) = self.hangout_in {
                    if free && self.mode_before == Mode::RaceSelect && std::env::var("MCLA_NO_CINE").is_err() {
                        // back on the road
                        let sc = [format!("{}_ext", self.hangouts[h].0), format!("{}_ext_01", self.hangouts[h].0)].into_iter().find(|s| self.cine.has(s));
                        if let Some(sc) = sc {
                            if self.cine.start(&mut gpu, &sc, glam::Mat4::IDENTITY) {
                                self.cine_free = true;
                                self.cine_caption = Some(String::new());
                            }
                        }
                    }
                    if h != i || d > 40.0 {
                        self.hangout_in = None;
                        self.delivery_armed = true;
                        // (the hangout's set-up starts again once the player is away: step 20b)
                        if let Some(k) = mc4::hangout_key_of_scene(&self.hangouts[h].0) {
                            self.game.mc4_hangout_left(k);
                        } else if self.hangouts[h].0.starts_with("delivery") {
                            // (step 21b: the delivery's set-up ends with the visit too)
                            let k = if self.hangouts[h].0.ends_with("_sc") { "sc_delivery" } else { "hw_delivery" };
                            self.game.mc4_hangout_left(k);
                        }
                    }
                } else if free && self.hangouts[i].0.starts_with("delivery") && cur.is_some_and(|p| p.distance(Vec3::from(DELIVERY_POINT[self.hangouts[i].0.ends_with("_sc") as usize])) < 12.0) {
                    // (delivery_hangout_core 0x3f5..0x552: within 12 of the set-up's point (static 3276),
                    // no speed test; with police activity above 1 the LOSE_COPS splash, again after 100
                    // polls of 0.1 s; else SetAllPoliceUnderground(1, 1), Racer_Deactivate - its panel)
                    let chased = matches!(self.game.opt_x.s23.heat_was, 2 | 3);
                    self.delivery_cops = (self.delivery_cops - dt).max(0.0);
                    if chased && self.delivery_armed {
                        if self.delivery_cops <= 0.0 {
                            let m = self.game.career.strings.get("LOSE_COPS").map(|s| s.to_uppercase()).unwrap_or_else(|| "LOSE THE COPS".into());
                            self.game.show_message(&m);
                            self.delivery_cops = 10.0;
                        }
                    } else if self.game.flash_hud.is_some() && self.delivery_armed && self.delivery_ease.is_none() && self.delivery_visit == 0 {
                        // (0x4a7: the police underground, then "ease to pause" - form 67 shown,
                        // PerspectiveForm_SetSlowMotion(67, 0, 1 / 0.25, 0, 1, 1), WAITUNWARPED(250))
                        self.delivery_armed = false;
                        self.police.units_down(&mut self.vehicles);
                        self.game.hangout_slow.set(0.0, 1.0 / DELIVERY_EASE_IN, false, true, 1.0);
                        self.game.hangout_slow.visible = true;
                        self.delivery_ease = Some((0.0, i));
                        log::info!("delivery: triggered at {:?}, ease to pause", cur);
                    }
                } else if let Some((si, j)) = line.filter(|&(_, j)| free && self.cuts.idle() && self.hangout_entry.is_none() && std::env::var("MCLA_NO_CINE").is_err() && self.game.mc4_hangout_open(&self.hangouts[j].0) && !matches!(self.police.heat, police::Heat::Pursuit | police::Heat::Search | police::Heat::Busted)) {
                    // the core's entry line crossed - no speed test, not with the police activity above 1 (the
                    // barrier's test): "ease to pause" (0.2 s), then the warp to the wait camera (`hangoutin`)
                    log::info!("hangout entry: {} line crossed at {:?}, speed {:.1}", hangoutin::SETUPS[si].key, cur, speed);
                    self.game.hangout_slow.set(0.0, 1.0 / hangoutin::SLOW_IN, false, true, 1.0);
                    self.game.hangout_slow.visible = true;
                    self.hangout_entry = Some(hangoutin::Entry { setup: si, hangout: j, stage: hangoutin::Stage::Slow, t: 0.0, warp: None });
                }
            }
            // ---- the delivery's visit (delivery_hangout_core): after the ease to pause Racer_Deactivate and
            // the panel (form 67, 0x7b6); its answer - BACK or ACCEPT - is followed by "ease from pause" (form 108,
            // 0.5 s, 0x831); the police come up at the end of the visit (0x351d: at once after BACK, after
            // the delivery when it was accepted) ----
            if let Some((t, i)) = self.delivery_ease.as_mut() {
                *t += dt;
                if *t >= DELIVERY_EASE_IN {
                    let i = *i;
                    self.delivery_ease = None;
                    if let Some(c) = self.vehicles.player_mut() {
                        c.state.vel = Vec3::ZERO;
                        c.state.ang_vel = Vec3::ZERO;
                    }
                    self.hangout_in = Some(i);
                    self.game.wager_here = false;
                    self.game.hangout_key = Some(if self.hangouts[i].0.ends_with("_sc") { "sc_delivery" } else { "hw_delivery" }.to_string());
                    // (the delivery panel keeps its last choice - mcDeliveryForm, F-17823)
                    self.game.hangout_diff = self.game.delivery_diff;
                    self.pending_action = MenuAction::OpenRaceSelect;
                    self.delivery_visit = 1;
                }
            } else if self.delivery_visit == 1 && self.mode_before == Mode::RaceSelect && self.game.mode != Mode::RaceSelect {
                self.game.hangout_slow.set(1.0, 1.0 / 0.5, false, true, 1.0);
                self.game.hangout_slow.visible = false;
                self.delivery_visit = 2;
                log::info!("delivery: panel answered, ease from pause");
            } else if self.delivery_visit == 2 && self.game.mode == Mode::FreeRoam && self.game.race.is_none() && self.cuts.idle() && self.cine.playing.is_none() && self.game.movie.is_none() {
                self.police.units_up();
                self.delivery_visit = 0;
                log::info!("delivery: visit over, police up");
            }
            // ---- pulling into a hangout (`hangoutin`): the ease to pause, the warp, the event, the scene ----
            let mut enter: Option<(usize, usize)> = None;
            if let Some(mut he) = self.hangout_entry.take() {
                he.t += dt;
                let su = &hangoutin::SETUPS[he.setup];
                let mut keep = true;
                match he.stage {
                    hangoutin::Stage::Slow => {
                        // WAITUNWARPED(200), then Warper_TransitionTo('STORY_LOCATION_CUT_SIMPLE', ..): from the render
                        // camera to the wait camera (Warp_Target_Close within 100 of it, as the garages')
                        if he.t >= hangoutin::SLOW_IN {
                            let e = hangoutin::wait_rows(su);
                            let look = (self.chase.look - self.chase.pos).normalize_or(Vec3::NEG_Z);
                            let s = hudmap::CamRows::look_at(self.cam.pos, self.cam.pos + look, self.boom_up.unwrap_or(Vec3::Y));
                            he.warp = (s.eye.distance(e.eye) < garagein::CLOSE).then(|| hudmap::CineWarp::target_close(s, e, self.cam.fov_y, garagein::LENS_DEG.to_radians()));
                            log::info!("hangout entry: {} (location {}) warp from {:?} ({})", su.key, su.uid, s.eye, he.warp.is_some());
                            he.stage = hangoutin::Stage::Warp;
                            he.t = 0.0;
                            // (ShowLocationName runs on the warp's Lerp: 0.1 s in, for 2 s)
                            self.hangout_title = Some((he.setup, 0.0));
                        }
                    }
                    hangoutin::Stage::Warp => {
                        let warp_len = he.warp.as_ref().map_or(0.0, |w| w.total());
                        let (rows, fov) = match he.warp.as_mut().and_then(|w| w.step(dt)) {
                            Some(v) => v,
                            None => (hangoutin::wait_rows(su), garagein::LENS_DEG.to_radians()),
                        };
                        self.place_view = Some((rows.eye, rows.look_point(), rows.r1, fov));
                        self.game.dive_k = 1.0;
                        // IsReadyToLoadStuff at the warp's end: Racer_Deactivate, "ease from pause" (0.5 s, form 108
                        // hidden), LOCATION_EVENT, MISSION_EVENT <KEY>_ENTER - the scene
                        if he.t >= warp_len {
                            self.game.hangout_slow.set(1.0, 1.0 / hangoutin::SLOW_OUT, false, true, 1.0);
                            self.game.hangout_slow.visible = false;
                            self.place_view = None;
                            enter = Some((he.setup, he.hangout));
                            keep = false;
                        }
                    }
                }
                if keep {
                    self.hangout_entry = Some(he);
                }
            }
            if let Some((si, i)) = enter {
                // (step 21b: <HANGOUT>_ENTER - a mission's event script plays its story scene in
                // place of the hangout's own entry; the hangout follows once it is over)
                let night = self.time_of_day > 18.5 || self.time_of_day < 5.5;
                let story = self.game.career.location_enter(hangoutin::SETUPS[si].enter).and_then(|id| self.cuts.for_mission(&id, night)).filter(|_| std::env::var("MCLA_NO_CUTS").is_err() && (self.opt.shot.is_none() || std::env::var("MCLA_CUTS").is_ok()));
                if let Some(s) = story {
                    self.cuts.queue(s);
                } else if self.cine.start(&mut gpu, &hangoutin::SETUPS[si].scene, glam::Mat4::IDENTITY) {
                    self.hangout_in = Some(i);
                    self.cine_free = true;
                    self.cine_caption = Some(String::new());
                    self.cine_after = Some(MenuAction::OpenRaceSelect);
                    // (four of the hangouts take bets: Mack's Burger, the rooftop parking in
                    // Santa Monica, the Diamond Lot and the Hollywood Lookout - the scenes
                    // their own scripts play)
                    // (the set-up's core: wager_hangout_core takes bets)
                    self.game.wager_here = hangoutin::SETUPS[si].key.contains("_wager_");
                    // (the location's key: its name in the string table, `<key>_state` in the registry)
                    self.game.hangout_key = Some(hangoutin::SETUPS[si].key.to_string());
                    // (series and wager panels open on MEDIUM: mcLocalHangoutForm F-75411 /
                    // F-48938, mcWagerForm F-90876 / F-82914 - combo 1)
                    self.game.hangout_diff = 1;
                    // the list opens at the events around here
                    let here = self.hangouts[i].1;
                    let mission = self.game.mission_races.clone();
                    let best = (0..self.game.race_defs.len()).min_by(|&a, &b| {
                        let key = |k: usize| self.game.race_defs[k].beacon.distance(here) - if mission.contains(&k) { 5000.0 } else { 0.0 };
                        key(a).partial_cmp(&key(b)).unwrap()
                    });
                    if let Some(b) = best {
                        self.game.race_sel = b;
                    }
                } else {
                    self.hangout_in = Some(i);
                }
            }
            // (test hook MCLA_HANGOUT_LINE=0..11: at 4 s and again at 10 s the car is put 15 m outside that set-up's entry
            // line, facing across its middle, and is pushed in at 8 m/s until the entry starts)
            if let Some(n) = std::env::var("MCLA_HANGOUT_LINE").ok().and_then(|v| v.parse::<usize>().ok()).filter(|n| *n < hangoutin::SETUPS.len()) {
                let su = &hangoutin::SETUPS[n];
                let m = (su.line[0] + su.line[1]) * 0.5;
                let d = su.line[1] - su.line[0];
                let out = Vec3::new(-d.z, 0.0, d.x).normalize_or(Vec3::X);
                if self.tick_no == 240 || self.tick_no == 600 {
                    if let Some(c) = self.vehicles.player_mut() {
                        c.state.relocate(m + out * 15.0 + Vec3::Y * 0.5, glam::Quat::from_rotation_y(out.x.atan2(out.z)));
                        c.state.vel = Vec3::ZERO;
                        c.pose_history.reset();
                    }
                    self.hangout_prev = None;
                } else if self.tick_no > 600 && self.hangout_entry.is_none() && self.hangout_title.is_none() && self.cine.playing.is_none() && self.game.mode == Mode::FreeRoam {
                    if let Some(c) = self.vehicles.player_mut() {
                        if c.state.pos.distance(m) < 20.0 {
                            c.state.vel = -out * 8.0;
                        }
                    }
                }
            }
            // ShowLocationName (popup template 63): the place's name and district, 0.1 s into the warp for 2 s
            if let Some((si, c)) = self.hangout_title.as_mut() {
                *c += dt;
                let (si, c) = (*si, *c);
                if c >= garagein::TITLE_FROM + garagein::TITLE_LEN {
                    self.game.place_title = None;
                    self.hangout_title = None;
                } else if c >= garagein::TITLE_FROM {
                    let k = hangoutin::SETUPS[si].key;
                    let st = |k: &str| self.game.career.strings.get(k).map(|v| v.to_string()).unwrap_or_default();
                    self.game.place_title = Some((st(k), st(&format!("{}_desc", k))));
                }
            }
            self.mode_before = self.game.mode;
            // (step 23b: the hangouts' scenes - the cars of their set-ups, the panel's camera, the
            // pink slip choice's camera on the profiles' cars)
            crate::ticktimes::mark(crate::ticktimes::S::Hangouts);
            {
                let near_h = near.map(|(i, d)| (self.hangouts[i].0.clone(), d));
                let scene = self.cine.playing.as_ref().map(|s| s.name.clone());
                let cam = self.cine.camera();
                self.hscene.update(&mut self.game, &mut self.vehicles, &mut gpu, &mut world, near_h.as_ref().map(|(n, d)| (n.as_str(), *d)), scene.as_deref(), cam, dt);
                self.layout.update(&self.game, &mut self.vehicles, &self.opt.data_dir, scene.as_deref(), dt);
            }
            crate::ticktimes::mark(crate::ticktimes::S::Other);
            // (a story hookman waits at his place: within his radius, not chased, he is met -
            // the CORE scripts' loop, step 20b)
            if self.tick_no % 15 == 0 && self.game.mode == Mode::FreeRoam && self.game.race.is_none() {
                if let Some(p) = self.vehicles.player().map(|c| c.state.pos) {
                    // (step 21b: the restarters of the places left wait for the player at their points)
                    self.game.mc4_hangout_restart_check([p.x, p.y, p.z]);
                    let pursued = matches!(self.police.heat, police::Heat::Pursuit | police::Heat::Search);
                    // (step 23: FIND_SH_02C waits at his place with the money box)
                    self.game.mc4_find02c_meet([p.x, p.y, p.z], pursued);
                    if self.game.career.story_meet_at([p.x, p.y, p.z], pursued).is_some() {
                        self.game.dirty = true;
                    }
                }
            }
        }
        // ---- the race's start: the original's warp onto the start scene, then the scene ----
        if self.game.race.is_none() {
            self.game.prerace = None;
            self.game.dive_k = 0.0;
            self.warp = None;
            self.warp_view = None;
            self.prerace_plan = None;
        }
        // DEFAULT_WITH_CUT's climb (`hudmap::CineClimb`): when it is over the camera keeps its last
        // matrix (`F-99030`), IsReadyToLoadStuff turns true and the script sets the race up - the
        // action held; the dive below starts from `climb_end`
        if let Some((c, act)) = self.climb.as_mut() {
            let act = *act;
            let (rows, fov) = match c.step(dt) {
                Some(v) => v,
                None => {
                    // (held: the script's WAITUNWARPED polls every 100 ms from the climb's start, then the
                    // race and its start scene are set up; warper state 4 counts 3 frames
                    // (m_CityPrestreamWaitFrames), 1 for the sectors, 1 for the scene camera, then 0.1 s -
                    // CAREER_RACES.md "The climb's hand-over")
                    let total = c.total();
                    let poll = ((total - 1e-4) / 0.1).ceil() * 0.1;
                    let wait = poll - total + 5.0 / 30.0 + 0.1;
                    let end = c.end();
                    log::info!("DEFAULT_WITH_CUT climb over: {:?} lens {:.2}, held {:.3} s", end.0.eye, end.1.to_degrees(), wait);
                    self.climb = None;
                    self.climb_end = Some(end);
                    self.climb_wait = Some((wait, act));
                    end
                }
            };
            self.climb_view = Some((rows.eye, rows.look_point(), rows.r1, fov));
            // (test hook MCLA_CLIMB_SHOTS=<prefix>,t1,t2,..: a screenshot <prefix>_<t>.png as the climb's
            // clock passes each t)
            if let (Ok(v), Some((c, _))) = (std::env::var("MCLA_CLIMB_SHOTS"), self.climb.as_ref()) {
                let mut it = v.split(',');
                let prefix = it.next().unwrap_or("climb").to_string();
                for x in it {
                    let at = x.parse::<f32>().unwrap_or(f32::MAX);
                    if c.t() - dt < at && c.t() >= at {
                        self.photo_request = Some(format!("{prefix}_{x}.png"));
                    }
                }
            }
        } else if let Some((left, act)) = self.climb_wait.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                self.pending_action = *act;
                self.climb_wait = None;
            }
        } else if self.climb_view.is_some() && self.warp.is_none() && self.game.prerace.is_none() {
            // (held until the dive takes over; let go if no race came of it)
            self.climb_hold += 1;
            if self.climb_hold > 2 {
                self.climb_view = None;
                self.climb_end = None;
            }
        }
        self.game.climb_on = self.climb.is_some() || self.climb_view.is_some();
        // The flight (CAREER_RACES.md "Race start: the warp"): mcWarper (`F-32081`, the cinematic
        // request of ARCADE's START RACE) waits for the start scene, then `F-18267` flies the render
        // camera to the scene's camera at scene t 1.0 (`F-62707`) with one of the cinematic
        // transitions (`hudmap::CineWarp`); 1 s before it lands the scene may start (state 19), so the
        // scene's clock runs at flight time - (T - 1) and the flight ends on its camera.
        if let Some(w) = self.warp.as_mut() {
            self.warp_t += dt;
            match w.step(dt) {
                Some((rows, fov)) => self.warp_view = Some((rows.eye, rows.look_point(), rows.r1, fov)),
                None => {
                    self.warp = None;
                    self.warp_view = None;
                }
            }
        }
        self.game.dive_k = if self.warp_view.is_some() { 1.0 } else { 0.0 };
        // The payback's warp (ACCEPT on form 68, `Hangouts/0xfc2c6c0a` 0x0c3d; CAREER_RACES.md "The payback's
        // warp"): the car deactivated where it is, Warp_Target_Close to the place's wait camera (within 100 of
        // it; else the camera is put there), Warper_IsReadyToLoadStuff polled every 30 ms - then PaybackCore: the
        // car changed, `cut_payback_get_keys` (Warper_SetIsEnterOnly(1): no flight onto it), the first leg
        if let Some((mut t, mut warp, hold)) = self.payback_in.take() {
            t += dt;
            if let Some(c) = self.vehicles.player_mut() {
                c.state.pos = hold.0;
                c.state.rot = hold.1;
                c.state.vel = Vec3::ZERO;
                c.state.ang_vel = Vec3::ZERO;
            }
            let len = warp.as_ref().map_or(0.0, |w| w.total());
            let (rows, fov) = match warp.as_mut().and_then(|w| w.step(dt)) {
                Some(v) => v,
                None => (garagein::wait_rows(&garagein::SETUPS[PAYBACK_PLACE_CAM]), garagein::LENS_DEG.to_radians()),
            };
            self.place_view = Some((rows.eye, rows.look_point(), rows.r1, fov));
            self.game.dive_k = 1.0;
            if t + 1e-4 >= ((len - 1e-4).max(0.0) / 0.03).ceil() * 0.03 {
                log::info!("payback warp: ready at {:.3} s (warp {:.3} s), tick {}", t, len, self.tick_no);
                self.payback_hold = true;
                self.pending_action = MenuAction::StartRace;
            } else {
                self.payback_in = Some((t, warp, hold));
            }
        } else if self.payback_hold {
            // (the camera waits at the wait camera until the keys scene is on)
            if self.cine.playing.as_ref().map_or(false, |sc| sc.name.contains("cut_payback_get_keys")) || self.game.race.is_none() {
                self.payback_hold = false;
                self.place_view = None;
            } else {
                let rows = garagein::wait_rows(&garagein::SETUPS[PAYBACK_PLACE_CAM]);
                self.place_view = Some((rows.eye, rows.look_point(), rows.r1, garagein::LENS_DEG.to_radians()));
                self.game.dive_k = 1.0;
            }
        }
        if let Some((front, yaw, grid)) = self.game.prerace.clone() {
            self.prerace_wait += 1;
            if let Some(r) = self.game.race.as_mut() {
                r.countdown = r.countdown.max(4.4);
            }
            if self.prerace_plan.is_none() {
                if self.game.replay.is_none() {
                    self.replay_prerace = Some((front, yaw, grid.clone()));
                }
                let plan = self.prerace_choose(front, yaw, &grid);
                // (PaybackCore's Warper_SetIsEnterOnly(1): the keys scene comes in from the wait camera, no flight)
                let enter_only = plan.0.first().map_or(false, |sc| sc.contains("cut_payback_get_keys"));
                if std::env::var("MCLA_NO_DIVE").is_err() && !enter_only {
                    if let Some((eye, fwd, up, fov)) = plan.0.first().and_then(|sc| self.cine.peek_camera(sc, plan.1, hudmap::CINE_HANDOVER)) {
                        let look = (self.chase.look - self.chase.pos).normalize_or(Vec3::NEG_Z);
                        let mut s = hudmap::CamRows::look_at(self.cam.pos, self.cam.pos + look, Vec3::Y);
                        // (test hook MCLA_WARP_FROM=x,y,z,fx,fy,fz: the flight starts from that camera)
                        let wf: Vec<f32> = std::env::var("MCLA_WARP_FROM").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                        if wf.len() == 6 {
                            let eye = Vec3::new(wf[0], wf[1], wf[2]);
                            s = hudmap::CamRows::look_at(eye, eye + Vec3::new(wf[3], wf[4], wf[5]), Vec3::Y);
                        }
                        let e = hudmap::CamRows::look_at(eye, eye + fwd, up);
                        // (after DEFAULT_WITH_CUT's climb: from the camera and lens it left - state 4 =
                        // ARCADE's state 7 handler, A-41914)
                        let fov_from = match self.climb_end.take() {
                            Some((rows, lens)) => {
                                s = rows;
                                lens
                            }
                            None => self.cam.fov_y,
                        };
                        self.climb_view = None;
                        let w = hudmap::CineWarp::new(s, e, Vec3::ZERO, fov_from, fov);
                        log::info!("race start warp: {:?} ({:.3} s) from {:?} to {:?} ({})", w.kind(), w.total(), s.eye, e.eye, plan.0[0]);
                        self.warp = Some(w);
                        self.warp_t = 0.0;
                    }
                }
                self.prerace_plan = Some(plan);
            }
            let ready = match self.warp.as_ref() {
                Some(w) => self.warp_t >= w.total() - hudmap::CINE_HANDOVER,
                None => (self.prerace_wait > 25 && world.streaming() == 0) || self.prerace_wait > 425,
            };
            if ready {
                self.game.prerace = None;
                self.prerace_wait = 0;
                let (scenes, origin) = self.prerace_plan.clone().unwrap_or_default();
                let pick = 0usize;
                let p080_sc = scenes.get(pick % scenes.len().max(1)).cloned().unwrap_or_default();
                if !scenes.is_empty() && std::env::var("MCLA_NO_CINE").is_err() && self.cuts.playing.is_none() && self.cine.start(&mut gpu, &scenes[pick % scenes.len()], origin) {
                    self.cine_free = true;
                    self.cine_prerace = true;
                    self.prerace_acts_begin(&p080_sc);
                    self.cine_caption = self.game.race.as_ref().map(|r| r.name.to_uppercase().replace('_', " "));
                    // (p080: the walk to the delivery car has no badge - walkthrough 2:44:20; the intro
                    // shows it at its racename event only)
                    if p080_sc.contains("delivery") || p080_sc.contains("cut_payback_get_keys") || self.game.active_def.as_ref().map_or(false, |d| d.id == althk::REDLIGHT_RACE_ID) {
                        self.cine_caption = None;
                    }
                    log::info!("prerace scene: {} (delivery intro {})", p080_sc, hudmap::delivery_intro(&p080_sc).is_some());
                    if let (Some(w), Some(sc)) = (self.warp.as_ref(), self.cine.playing.as_mut()) {
                        sc.t = (self.warp_t - (w.total() - hudmap::CINE_HANDOVER)).max(0.0);
                    }
                }
            }
        }
        if self.cine.playing.is_some() {
            // (the red light's script runs its own clock: Race_SetTimer(-7000) with the timer off over
            // the start scene, then Race_SetTimer(-4000) on - CruisingHookmen 0x18ac539b; the scene's
            // countdown and racename events are not its)
            let red_alt = self.game.active_def.as_ref().map_or(false, |d| d.id == althk::REDLIGHT_RACE_ID);
            let t = if self.cine_free {
                let skip = (self.cine_caption.is_some() && std::mem::take(&mut self.game.nav_enter)) || (self.cine_prerace && self.game.race.is_none()) || (self.cine_results && self.game.mode != Mode::Results);
                // (a scene without its own countdown event holds the count; the race-start scripts all
                // have one - prerace.rs)
                let own = !red_alt && self.cine.playing.as_ref().map_or(false, |sc| prerace::events(&sc.name).is_some());
                if let (true, false, Some(r)) = (self.cine_prerace, own, self.game.race.as_mut()) {
                    r.countdown = if red_alt { 4.0 } else { r.countdown.max(4.4) };
                }
                self.cine.playing.as_mut().and_then(|sc| {
                    sc.t += dt;
                    // (p080: a delivery intro's bookmark_cut_to_game - the game camera from there)
                    let cut = hudmap::delivery_intro(&sc.name).map_or(false, |e| sc.t >= e.up)
                        || prerace::events(&sc.name).map_or(false, |e| sc.t >= e.cut);
                    if sc.t >= sc.duration || skip || cut { None } else { Some(sc.t) }
                })
            } else {
                self.cuts.playing.as_ref().map(|p| p.t)
            };
            match t {
                Some(t) => {
                    if let Some(sc) = self.cine.playing.as_mut() {
                        sc.t = t;
                    }
                    // (p080) a delivery intro: its racename event shows the race's badge (form 70);
                    // the race clock runs to GO 3 s after its countdown event (countdown_321Go_3sec)
                    if let Some(e) = self.cine.playing.as_ref().and_then(|sc| hudmap::delivery_intro(&sc.name)) {
                        if let Some(r) = self.game.race.as_mut() {
                            r.countdown = (e.countdown + 3.0 - t).max(0.0);
                        }
                        self.cine_caption = (t >= e.racename && t < e.racename + e.racename_len).then(|| self.game.race.as_ref().map(|r| r.name.to_uppercase().replace('_', " ")).unwrap_or_default());
                    }
                    // a grid / starter / taunt scene: GO 3 s after its countdown event, the badge from its
                    // racename event for that event's own duration (2 - 2.033 s; form 70 shown at its start,
                    // closed at its end - as the delivery intros), no HUD before the countdown (prerace.rs)
                    // (the payback's keys scene plays before PaybackCore starts the leg: no race HUD over it)
                    if self.cine_prerace && self.cine.playing.as_ref().map_or(false, |sc| sc.name.contains("cut_payback_get_keys")) {
                        self.game.scene_hud_off = true;
                    }
                    // (the red light's scene - its own clock held, no badge - has no HUD over it:
                    // `cap/rl_07..10`)
                    if self.cine_prerace && red_alt {
                        self.game.scene_hud_off = true;
                    }
                    if let Some(e) = self.cine.playing.as_ref().filter(|_| self.cine_prerace && !red_alt).and_then(|sc| prerace::events(&sc.name)) {
                        if let Some(r) = self.game.race.as_mut() {
                            r.countdown = (e.go() - t).max(0.0);
                        }
                        self.cine_caption = (e.racename >= 0.0 && t >= e.racename && t < e.racename + e.racename_len).then(|| self.game.race.as_ref().map(|r| r.name.to_uppercase().replace('_', " ")).unwrap_or_default());
                        self.game.scene_hud_off = t < e.countdown;
                    }
                    // (test hook MCLA_CINE_OVERLAP=1: a start scene's people standing inside a racer's car -
                    // root within 1.0 m of its sides and 2.4 m of its centre along it, a typical car)
                    if self.cine_prerace && self.tick_no % 6 == 0 && std::env::var("MCLA_CINE_OVERLAP").is_ok() {
                        let racers = self.game.race.as_ref().map(|r| r.participants.clone()).unwrap_or_default();
                        for (label, p) in self.cine.actor_roots() {
                            for &ci in &racers {
                                let Some(c) = self.vehicles.cars.get(ci) else { continue };
                                let f = c.state.forward();
                                let f = Vec3::new(f.x, 0.0, f.z).normalize_or_zero();
                                let d = p - c.state.pos;
                                let (along, side) = (d.dot(f), d.dot(Vec3::new(-f.z, 0.0, f.x)));
                                if along.abs() < 2.4 && side.abs() < 1.0 {
                                    log::info!("cine overlap: t {:.2} {} in car {} ({}) along {:.2} side {:.2}", t, label, ci, if c.is_player { "player" } else { c.model.name.as_str() }, along, side);
                                }
                            }
                        }
                    }
                    if self.cine_prerace {
                        if let Some(a) = self.prerace_acts.as_mut() {
                            a.t = t;
                        }
                    }
                    // the cars: where the game's physics runs with the scene, each moved car is driven along its
                    // mover by its car event and animated brain (cinecar.rs); elsewhere - a story scene stops the
                    // physics, a race start's scene hands its racers to the grid (not ported: posed) - the car is
                    // put where the scene has it. (MCLA_CINE_POSE=1: every scene's cars posed, as before)
                    let drive = self.cine_free && !self.cine_prerace && std::env::var_os("MCLA_CINE_POSE").is_none();
                    let scene_name = self.cine.playing.as_ref().map(|sc| sc.name.clone()).unwrap_or_default();
                    if self.cine_events.0 != scene_name {
                        self.cine_events = (scene_name, Vec::new());
                    }
                    for label in self.cine.movers() {
                        let Some((pos, rot)) = self.cine.mover(&label) else { continue };
                        let idx = if cine::is_player_car(&label) {
                            self.vehicles.cars.iter().position(|c| c.is_player)
                        } else if self.cine_prerace && label == "drv01" {
                            // the other car at the lights
                            self.game.race.as_ref().and_then(|r| r.participants.get(1).copied())
                        } else {
                            let tag = format!("cine:{label}");
                            self.vehicles.cars.iter().position(|c| c.driver == tag)
                        };
                        let driven = idx.filter(|&i| drive && self.vehicles.cars[i].state.veh.is_some() && self.vehicles.cars[i].model.aitunes.is_some());
                        if let (Some(i), Some((anim, frame))) = (driven, self.cine.mover_anim(&label)) {
                            let k = match self.cine_events.1.iter().position(|e| e.0 == label) {
                                Some(k) => k,
                                None => {
                                    self.cine_events.1.push((label.clone(), cinecar::CarEvent::default()));
                                    self.cine_events.1.len() - 1
                                }
                            };
                            let c = &mut self.vehicles.cars[i];
                            if c.brain.is_none() {
                                let tune = c.model.aitunes.as_ref().map(|a| Arc::new(mcla_formats::rage::aitune::AiTune::lerp(&a.base, &a.mods, c.perf)));
                                c.brain = Some(cinecar::AnimBrain { accel: 0.0, curv: 0.0, tune });
                            }
                            let ev = &mut self.cine_events.1[k].1;
                            if !ev.done {
                                // (the event's update has the time before this frame's step - every port frame:
                                // updated at 30 Hz with dt 1/30 the drive-out ends 0.46 m off the recording,
                                // at the port's 60 Hz 0.21 m)
                                let t0 = (t - dt).max(0.0);
                                let idle0 = c.brain.as_ref().and_then(|b| b.tune.as_ref()).map_or(0.0, |a| a.idle_speed[0]);
                                let (accel, curv, snap) = cinecar::update(ev, &anim, frame, t0, dt, &c.state, idle0);
                                let snapped = snap.is_some();
                                if let Some(sn) = snap {
                                    if let Some(vt) = c.params().veh.clone() {
                                        physics::vehsim::set_pose(&mut c.state, &vt, &world.collider, sn.m, sn.vel, sn.ang_vel);
                                    }
                                    c.placed = true;
                                    c.pose_history.reset();
                                }
                                if let Some(b) = c.brain.as_mut() {
                                    b.accel = accel;
                                    b.curv = curv;
                                }
                                if anim.duration - t <= 0.0 {
                                    ev.done = true;
                                }
                                if std::env::var_os("MCLA_CINE_CAR_TRACE").is_some() {
                                    let m = glam::Mat3::from_quat(c.state.rot);
                                    log::info!("cine car: {} t {:.3} snap {} accel {:.3} curv {:.4} in {:?} pos {:.3} {:.3} {:.3} fwd {:.4} {:.4} speed {:.3} mover {:.3} {:.3}", label, t0, snapped, accel, curv, c.input, c.state.pos.x, c.state.pos.y, c.state.pos.z, -m.z_axis.x, -m.z_axis.z, c.state.speed(), pos.x, pos.z);
                                }
                            }
                            continue;
                        }
                        if let Some(i) = idx {
                            self.vehicles.pose_car(i, pos, rot);
                            // (a replay's opening: nothing is simulated, the pose drawn is this one)
                            if self.game.replay.is_some() {
                                let c = &mut self.vehicles.cars[i];
                                c.pose_history.begin(&c.state, c.lean);
                            }
                        }
                    }
                }
                None => {
                    // (p080) a delivery intro cut to the game camera (or skipped to its bookmark): its
                    // events go on; the walk to the delivery car: the intro next, after the warp
                    // (walkthrough 2:44:27..2:44:34 - the city from the air, down onto the car)
                    if let Some((name, st)) = self.cine.playing.as_ref().map(|sc| (sc.name.clone(), sc.t)) {
                        if let Some(e) = hudmap::delivery_intro(&name) {
                            if self.game.race.is_some() {
                                self.delivery_tail = Some((name, st.max(e.up)));
                            }
                        } else if let (Some(e), true) = (prerace::events(&name), self.cine_prerace && self.game.race.is_some()) {
                            // (cut to the game camera, or skipped: the script's countdown event still comes)
                            self.delivery_tail = Some((name, st.max(e.cut)));
                        } else if name.contains("cut_delivery_get_keys") && self.game.race.is_some() {
                            if let Some((sc, g0, yaw)) = self.delivery_intro.take() {
                                if let Some(i) = self.vehicles.cars.iter().position(|c| c.is_player) {
                                    self.vehicles.cars[i].state = crate::physics::CarState::new(g0 + Vec3::Y * 0.9, yaw);
                                    self.vehicles.cars[i].placed = false;
                                }
                                self.game.prerace_scene = Some(sc);
                                self.game.prerace = Some((g0, yaw, String::new()));
                                // (#10: the walk's start plan goes with it - else the walk was played
                                // again from that plan and the intro never came, the race starting where
                                // the walk parks the car; the intro is planned anew at grid slot 0)
                                self.prerace_plan = None;
                                self.prerace_wait = 0;
                                log::info!("delivery start: walk over, intro {} at grid slot 0 {:?} yaw {:.3}", self.game.prerace_scene.as_deref().unwrap_or(""), g0, yaw);
                            }
                        }
                    }
                    // over: the scene's extra cars leave with it; a start scene's people stay on to the
                    // clip's end (its script runs on after the cut)
                    let linger = self.cine_prerace && self.game.race.is_some();
                    self.cine_free = false;
                    self.cine_prerace = false;
                    self.cine_results = false;
                    self.cine_caption = None;
                    if let Some(a) = self.cine_after.take() {
                        self.pending_action = a;
                    }
                    if linger {
                        self.cine.release();
                    } else {
                        self.cine.stop();
                    }
                    // (a cruiser the scene borrowed stays in the world: CopPullover gives it back to the police)
                    self.vehicles.cars.retain(|c| !c.driver.starts_with("cine:") || c.cop);
                    for c in self.vehicles.cars.iter_mut().filter(|c| c.cop && c.driver.starts_with("cine:")) {
                        c.driver.clear();
                        // (the scene took its driver: the police drive it again - left without one it only crept on
                        // at idle, GitHub #17 "the cops don't know how to drive" after a pull-over)
                        if c.ai.is_none() {
                            c.ai = Some(ai::AiDriver::new(c.uid ^ 0x9e3779b9));
                        }
                    }
                }
            }
        }
        if self.game.race.is_none() {
            self.cine.drop_linger();
        }
        // ---- the overhead map's script triggers (rule_frame.md 9.2). A red light race's start
        // script sets the race clock to -4 s once its start scene is over and sends the HUD mapGoBig
        // with the landmark's name on the map control (the title bar), then mapGoNormal at -1 s
        // (CruisingHookmen 0x18ac539b 0x5CED..0x5E2D); the port's clock starts at -4.4 s. The
        // delivery start scenes send them by their events big_map_up / big_map_down. mapGoBig and
        // mapGoNormal move the same HUD state D-pad up toggles. ----
        {
            // (from -4 s, as the script sends it, to -1 s: 3 s up whatever the port's clock starts at)
            let red = self.game.race.as_ref().map_or(false, |r| r.countdown > 1.0 && r.countdown <= 4.0)
                && self.game.prerace.is_none()
                && self.cine.playing.is_none()
                && self.game.current_race.and_then(|i| self.game.race_defs.get(i)).map_or(false, |d| d.sub_type == "RedLight");
            // (p080: after the cut the intro's events go on - its tail - until the countdown event)
            if self.game.race.is_none() {
                self.delivery_tail = None;
                self.delivery_intro = None;
            }
            if let Some((name, t)) = self.delivery_tail.as_mut() {
                *t += dt;
                let (t, e) = (*t, hudmap::delivery_intro(name).map(|e| (e.countdown, e.scene)).or_else(|| prerace::events(name).map(|e| (e.countdown, e.scene))));
                if let (Some((countdown, scene)), Some(r)) = (e, self.game.race.as_mut()) {
                    if t < countdown {
                        r.countdown = (countdown + 3.0 - t).max(0.0);
                    } else {
                        // (the countdown runs on by itself: GO at e.countdown + 3)
                        r.countdown = r.countdown.min((countdown + 3.0 - t).max(0.0));
                        log::info!("prerace scene: countdown event at {:.2} s of {}, GO in {:.2} s", t, scene, r.countdown);
                        self.delivery_tail = None;
                    }
                } else {
                    self.delivery_tail = None;
                }
            }
            let scene = self.cine.playing.as_ref().map_or(false, |sc| hudmap::cine_big_map(&sc.name, sc.t))
                || self.delivery_tail.as_ref().map_or(false, |(n, t)| hudmap::cine_big_map(n, *t));
            let go = red || scene;
            if go != self.game.map_go {
                self.game.map_go = go;
                self.game.hud_map_big = go;
                // (the red light's script turns the map control's property 0 on with the landmark's
                // name - 0x5D7A -, but the original's big map has no title bar there: work/hookmen2
                // `cap/rl_11`. The control (HUD+2624) is not made visible by the red light's HUD
                // behaviour, F-88341; who shows it for a delivery's intro is not traced.)
                let _ = &self.redlights.title_key;
                self.game.overhead_title = None;
                // (p080) a delivery intro's title bar: the race's name, then its kind "Delivery" and
                // the district's emblem (walkthrough 2:46:30; the code that fills mcMapCtrl's
                // +196 / +200 for it is not traced - the badge's own values are used)
                self.game.overhead_sub = None;
                if scene && go {
                    if let Some(d) = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).filter(|d| d.race_type == "Delivery") {
                        let name = self.game.career.strings.get(&d.id).map(|v| v.to_string()).unwrap_or_else(|| d.name.to_uppercase().replace('_', " "));
                        let kind = self.game.career.strings.get("Delivery").map(|v| v.to_string()).unwrap_or_else(|| "Delivery".to_string());
                        let dl = format!("{} {}", d.district, d.id).to_ascii_lowercase();
                        let icon = std::env::var("MCLA_OVERHEAD_ICON").ok().and_then(|v| v.parse::<i32>().ok()).unwrap_or(if dl.contains("south") || dl.contains("sc_") { 4 } else { 0 });
                        self.game.overhead_title = Some(name);
                        self.game.overhead_sub = Some((icon, kind));
                    }
                }
                log::info!("overhead map: {} (red light {}, scene {}, title {:?})", if go { "mapGoBig" } else { "mapGoNormal" }, red, scene, self.game.overhead_title);
            }
            // (test hook MCLA_SHOT_ON_MAP=n: the screenshot n frames after a script raised the map)
            static UP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if go {
                let k = UP.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if std::env::var("MCLA_SHOT_ON_MAP").ok().and_then(|v| v.parse::<u32>().ok()) == Some(k) {
                    self.game.shot_now = true;
                }
            } else {
                UP.store(0, std::sync::atomic::Ordering::Relaxed);
            }
            // (test hook MCLA_SHOT_AT_GO=s: the screenshot once the race is s seconds from GO)
            if let (Some(s), Some(r)) = (std::env::var("MCLA_SHOT_AT_GO").ok().and_then(|v| v.parse::<f32>().ok()), self.game.race.as_ref()) {
                if r.countdown > 0.0 && r.countdown <= s && self.game.prerace.is_none() && !std::mem::replace(&mut self.shot_at_go, true) {
                    log::info!("shot at {:.2} s to GO (scene {:?})", r.countdown, self.cine.playing.as_ref().map(|sc| (sc.name.clone(), sc.t)));
                    self.game.shot_now = true;
                }
            }
            // (test hook MCLA_TITLE_SHOTS=<dir>;<s>: a picture every s seconds on the start screen)
            if let (Ok(v), true) = (std::env::var("MCLA_TITLE_SHOTS"), self.game.mode == Mode::MainMenu) {
                static CLOCK: std::sync::Mutex<(f32, u32)> = std::sync::Mutex::new((0.0, 0));
                let mut c = CLOCK.lock().unwrap();
                c.0 += dt;
                let (dir, s) = v.split_once(';').unwrap_or((".", v.as_str()));
                let s: f32 = s.parse().unwrap_or(1.0);
                if c.0 >= s * c.1 as f32 && self.photo_request.is_none() {
                    self.photo_request = Some(format!("{}/ts_{:03}.png", dir, c.1));
                    c.1 += 1;
                }
            }
            // (test hook MCLA_RACE_START_SHOTS=<dir>;t1,t2,...: pictures t seconds after the race-start warp
            // began - or the race, when it has none -, the camera logged)
            if let Ok(v) = std::env::var("MCLA_RACE_START_SHOTS") {
                static NEXT_S: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                static CLOCK: std::sync::Mutex<Option<f32>> = std::sync::Mutex::new(None);
                let mut clock = CLOCK.lock().unwrap();
                if (self.warp.is_some() || self.game.race.is_some()) && clock.is_none() {
                    // (a race without a warp - the freeway race's rolling start - from its first frame)
                    *clock = Some(if self.warp.is_some() { self.warp_t } else { 0.0 });
                } else if let Some(c) = clock.as_mut() {
                    *c += dt;
                }
                if let Some(t) = *clock {
                    let (dir, ts) = v.split_once(';').unwrap_or((".", v.as_str()));
                    let ts: Vec<f32> = ts.split(',').filter_map(|x| x.parse().ok()).collect();
                    let n = NEXT_S.load(std::sync::atomic::Ordering::Relaxed);
                    if let Some(&due) = ts.get(n) {
                        if t >= due && self.photo_request.is_none() {
                            NEXT_S.store(n + 1, std::sync::atomic::Ordering::Relaxed);
                            let car = self.vehicles.player().map(|c| c.state.pos).unwrap_or_default();
                            log::info!("race start shot {} at {:.3} s: eye {:.2} {:.2} {:.2} (car + {:.2} {:.2} {:.2}) look {:.2} {:.2} {:.2} fov {:.1} scene {:?} countdown {:.2}", n, t, self.chase.pos.x, self.chase.pos.y, self.chase.pos.z, self.chase.pos.x - car.x, self.chase.pos.y - car.y, self.chase.pos.z - car.z, self.chase.look.x, self.chase.look.y, self.chase.look.z, self.cam.fov_y.to_degrees(), self.cine.playing.as_ref().map(|sc| (sc.name.clone(), sc.t)), self.game.race.as_ref().map_or(-1.0, |r| r.countdown));
                            self.photo_request = Some(format!("{}/rs_{:02}_{:05}.png", dir, n, (t * 1000.0) as u32));
                        }
                    }
                }
            }
            // (test hook MCLA_CINE_SHOTS=<dir>;t1,t2,...: pictures t seconds into the scenes' clock - the first of
            // the run whose name contains MCLA_CINE_SHOTS_SCENE, any by default)
            if let (Ok(v), Some(sc)) = (std::env::var("MCLA_CINE_SHOTS"), self.cine.playing.as_ref()) {
                static NEXT_C: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                if sc.name.contains(std::env::var("MCLA_CINE_SHOTS_SCENE").unwrap_or_default().as_str()) {
                    let (dir, ts) = v.split_once(';').unwrap_or((".", v.as_str()));
                    let ts: Vec<f32> = ts.split(',').filter_map(|x| x.parse().ok()).collect();
                    let n = NEXT_C.load(std::sync::atomic::Ordering::Relaxed);
                    if let (Some(&due), true) = (ts.get(n), self.photo_request.is_none()) {
                        if sc.t >= due {
                            NEXT_C.store(n + 1, std::sync::atomic::Ordering::Relaxed);
                            log::info!("cine shot {} of {} at {:.3} s", n, sc.name, sc.t);
                            self.photo_request = Some(format!("{}/cs_{:02}_{:05}.png", dir, n, (sc.t * 1000.0) as u32));
                        }
                    }
                }
            }
            // (test hook MCLA_RACE_END_SHOTS=<dir>;t1,t2,...: pictures t seconds after the line,
            // the camera logged in the car's frame - x right, y up, z rear)
            if let (Ok(v), Some(t)) = (std::env::var("MCLA_RACE_END_SHOTS"), self.game.race_end_t) {
                static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let (dir, ts) = v.split_once(';').unwrap_or((".", v.as_str()));
                let ts: Vec<f32> = ts.split(',').filter_map(|x| x.parse().ok()).collect();
                let n = NEXT.load(std::sync::atomic::Ordering::Relaxed);
                if let (Some(&due), Some(car)) = (ts.get(n), self.vehicles.player()) {
                    if t >= due && self.photo_request.is_none() {
                        NEXT.store(n + 1, std::sync::atomic::Ordering::Relaxed);
                        let st = car.render_state(1.0);
                        let inv = st.rot.inverse();
                        let (e, l) = (inv * (self.chase.pos - st.pos), inv * (self.chase.look - st.pos));
                        let f = (l - e).normalize_or(Vec3::Z);
                        log::info!("race end shot {} at {:.3} s: eye {:.2} {:.2} {:.2} view {:.3} {:.3} {:.3} fov {:.1} scale {:.3}", n, t, e.x, e.y, e.z, f.x, f.y, f.z, self.cam.fov_y.to_degrees(), self.game.mc4_time_scale());
                        self.photo_request = Some(format!("{}/re_{:02}_{:04}.png", dir, n, (t * 1000.0) as u32));
                    }
                }
            }
            // (test hook MCLA_SHOT_AFTER_GO=s: the screenshot once the race clock passes s seconds)
            if let (Some(s), Some(r)) = (std::env::var("MCLA_SHOT_AFTER_GO").ok().and_then(|v| v.parse::<f32>().ok()), self.game.race.as_ref()) {
                if r.countdown <= 0.0 && r.time >= s && !std::mem::replace(&mut self.shot_at_go, true) {
                    log::info!("shot at race clock {:.2} s", r.time);
                    self.game.shot_now = true;
                }
            }
        }
        self.game.cine_black = self.cine_hold.is_some();
        let subtitles = self.game.opts.subtitles;
        self.game.cine = self.cuts.playing.as_ref().map(|_| self.cuts.subtitle().filter(|_| subtitles).unwrap_or("").to_string());
        if self.cine_free && self.cine.playing.is_some() {
            // letterbox with the name of the race / what happened
            if let Some(c) = self.cine_caption.clone() {
                self.game.cine = Some(c);
            }
        }
        if self.tick_no % 60 == 0 {
            if let Some(p) = self.cuts.playing.as_ref() {
                log::debug!("cutscene t {:.1}: {:?}", p.t, self.game.cine);
            }
        }
        self.game.race_slow_tick(dt);
        self.game.hangout_slow.update(dt);
        // (the bike gyro's T[+84]: the global scale as the main loop leaves it, never under 0.1)
        crate::physics::vehsim::TIME_SCALE84.store(self.game.mc4_time_scale().max(0.1).to_bits(), std::sync::atomic::Ordering::Relaxed);
        let sim_dt = if self.game.mode == Mode::Paused || self.cuts.playing.is_some() || self.game.movie.is_some() || self.game.show_map || self.game.tutorial_hold() || self.photo.is_some() { 0.0 } else { dt * self.wipe_scale.min(self.ability.time_scale()).max(0.1) * self.game.mc4_time_scale() };
        self.vehicles.rescue_fallen(&world, focus);
        // (VEHICLE SELECT: the three starters stand deactivated in the lot - the screen's loop calls no racer native;
        // the traffic and the people about them go on. The player has no car yet - AddVehicleToPlayer comes after
        // ACCEPT - so the port's player car is kept out of sight at the kerb until then)
        if self.game.starter_pick.is_some() {
            self.vehicles.lights_off = true;
            // (out of sight at the kerb from the first frame of the screen: the city about the lot is streamed by then)
            if !std::mem::replace(&mut self.starter_was, true) {
                self.place_player_at_kerb();
                self.starter_stand = self.vehicles.player().map(|c| (c.state.pos, c.state.rot));
            }
            // (test hook: MCLA_STARTER_LOG=1 logs where the starters stand, once a second)
            if std::env::var("MCLA_STARTER_LOG").is_ok() {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                if N.fetch_add(1, Ordering::Relaxed) % 60 == 0 {
                    for c in self.vehicles.cars.iter().filter(|c| c.driver == "starter") {
                        log::info!("starter hold: {} {:.4} {:.4} {:.4} speed {:.4}", c.model.name, c.state.pos.x, c.state.pos.y, c.state.pos.z, c.state.vel.length());
                    }
                }
            }
        } else if std::mem::take(&mut self.starter_was) {
            self.vehicles.lights_off = false;
            self.sel_cam = None;
            // (CarSelection 0x1402 on: the two not taken get their streaming unlocked and are marked
            // Opponent_ToggleDeleteOnEviction - they go when the opponent manager evicts them, below; the one taken
            // is given to the player (AddVehicleToPlayer) and its racer is SetUID(208) - it stays in the lot, and
            // the original's story scene shows all three behind the player)
            for c in self.vehicles.cars.iter_mut().filter(|c| c.driver == "starter") {
                c.driver = "starter_left".into();
            }
            if let Some(dir) = self.game.car_dirs.get(self.game.current_car).cloned() {
                let (color, cfg) = (self.game.paint_color(), self.game.cfg.clone());
                if let Err(e) = self.vehicles.replace_player(&mut gpu, &mut world, &dir, color, &cfg, None) {
                    log::warn!("opening: the chosen starter {}: {:#}", dir.display(), e);
                }
            }
            if let Some(c) = self.vehicles.player_mut() {
                c.deactivated = false;
            }
            // (CarSelection 0x1494: AmbientMgr_Reset, the last thing it does - `Vehicles::ambient_reset`)
            self.vehicles.ambient_reset();
        }
        // (the hidden car held where it was put, still - its ground need not be streamed in - through VEHICLE SELECT;
        // after ACCEPT it is let go onto its wheels once that ground is here: the first story scene waits for it)
        if self.cine.playing.is_some() || (self.game.starter_pick.is_none() && self.starter_stand.is_some_and(|(p, _)| world.ready_at(p))) {
            self.starter_stand = None;
        }
        if let (Some((p, r)), Some(c)) = (self.starter_stand, self.vehicles.player_mut()) {
            c.state.relocate(p, r);
            c.state.vel = Vec3::ZERO;
            c.state.ang_vel = Vec3::ZERO;
        }
        // (the starters not taken, deleted on eviction: the opponent manager's walk, F-31748 - a car in the world
        // whose horizontal distance to the player less M+896 (100 m) is over M+892 (600 m) is evicted, and one marked
        // DeleteOnEviction is deleted (F-28020); the constructor's values, docs/reference/ai/brain.md 2.9. Out of
        // the slots (8) is not reached by two cars and the hookman)
        if let Some(me) = self.vehicles.player().map(|c| c.state.pos) {
            self.vehicles.cars.retain(|c| c.driver != "starter_left" || (Vec3::new(c.state.pos.x - me.x, 0.0, c.state.pos.z - me.z).length() - 100.0).max(0.0) <= 600.0);
        }
        // (a new career's starters and first story scene read ahead on worker threads - docs/PERFORMANCE.md 2026-10-25)
        self.opening_read_ahead(&world);
        self.grid_ahead(&world);
        // CarSelection (game/career/0xebb682f7, 0x214 - 0x35a) makes the three starters before the film: opponents
        // loaded into the lot (OpponentManager_LoadOpponent, AddOpponentToWorld), deactivated, their icons hidden;
        // VEHICLE SELECT only moves the camera between them (`sel_cam`), and the one taken stays in the lot. The
        // places are the original's three car objects on this screen (the body's origin and its z axis - the
        // script's own placements are (-173.088, 18.169, -762.769) 34.025, (-176.355, 18.199, -761.178) 35.091,
        // (-179.104, 18.203, -759.119) 36.12, before the cars settle)
        // (made under the fade's black, the frame it is full, and the film waits for them (`opening_tick`): the original
        // makes them just before its fade, but their models (~90 ms here) would stop the fade's first frame;
        // nothing can see them until VEHICLE SELECT - docs/PERFORMANCE.md 2026-10-27)
        if self.game.fresh_game && self.intro_stage >= 1 && (self.intro_mask.full() || self.intro_stage >= 2) && !self.starters_made {
            self.starters_made = true;
            const LOT: [([f32; 3], [f32; 2]); 3] = [([-173.088, 18.137, -762.768], [0.559, 0.829]), ([-176.354, 18.147, -761.177], [0.575, 0.818]), ([-179.103, 18.192, -759.120], [0.589, 0.808])];
            self.vehicles.cars.retain(|c| c.driver != "starter");
            for (k, name) in game::STARTERS.iter().enumerate() {
                let Some(j) = self.game.car_dirs.iter().position(|d| d.file_name().map_or(false, |n| n.to_string_lossy().eq_ignore_ascii_case(name))) else { continue };
                let dir = self.game.car_dirs[j].clone();
                let was = self.game.current_car;
                self.game.current_car = j;
                self.game.set_starter_paint(name);
                let (color, cfg) = (self.game.paint_color(), self.game.cfg.clone());
                self.game.current_car = was;
                if let Ok(model) = self.vehicles.model_cached(&mut gpu, &mut world, &dir, &cfg) {
                    let (p, z) = LOT[k];
                    let z = Vec3::new(z[0], 0.0, z[1]).normalize();
                    let idx = self.vehicles.spawn(&gpu, model, Vec3::from(p), 0.0, color, false);
                    // (posed with its wheels under their mounts: the arrival scene that follows stops the physics, so
                    // in real time the car may get no step before it is seen)
                    self.vehicles.pose_car(idx, Vec3::from(p), glam::Quat::from_mat3(&glam::Mat3::from_cols(Vec3::Y.cross(z), Vec3::Y, z)));
                    let c = &mut self.vehicles.cars[idx];
                    c.ai = None;
                    c.pinned = true;
                    // (Racer_Deactivate: chassis +592 bit 0x1 - work/ui2/starter_hold/notes.md)
                    c.deactivated = true;
                    c.driver = "starter".into();
                }
            }
            if let Some(c) = self.vehicles.player_mut() {
                c.deactivated = true;
            }
            log::info!("opening: the three starters in the lot");
        }
        // VEHICLE SELECT: the starter under the cursor is the car the player will be given
        if let Some(name) = self.game.starter_swap.take() {
            if let Some(i) = self.game.car_dirs.iter().position(|d| d.file_name().map_or(false, |n| n.to_string_lossy().eq_ignore_ascii_case(&name))) {
                self.game.owned.retain(|o| !game::STARTERS.iter().any(|s| o.eq_ignore_ascii_case(s)));
                self.game.current_car = i;
                self.game.selected_car = i;
                self.game.ensure_owned();
                self.game.set_starter_paint(&name);
            }
        }
        // VIEW REPLAY: the race's cars and the traffic about the player where the recording has
        // them (nothing is simulated in the results); the recorded cars that have left the road
        // are made again, the traffic it does not have is taken off it; leaving the replay puts
        // every car back as it stood
        if let Some(pb) = self.game.replay.as_mut() {
            if !std::mem::replace(&mut pb.started, true) {
                let rec = &mut self.game.replay_rec;
                // (not from a running race's pause: its traffic is still wanted)
                if self.game.mode == Mode::Results {
                    self.vehicles.cars.retain(|c| c.is_player || c.pinned || c.parked.is_some() || rec.has(c.uid));
                }
                rec.saved = self.vehicles.cars.iter().map(|c| (c.uid, c.state.clone(), c.lean)).collect();
                let gone: Vec<(u32, std::sync::Arc<vehicles::CarModel>, [f32; 4])> = rec.gone(&self.vehicles.cars).iter().map(|t| (t.uid, t.model.clone(), t.color)).collect();
                log::info!("replay: {} tracks ({} whole) over {:.1} s, {} cars made again", rec.tracks.len(), rec.tracks.iter().filter(|t| !t.full.is_empty()).count(), rec.len(), gone.len());
                for (uid, model, color) in gone {
                    let i = self.vehicles.spawn(&gpu, model, Vec3::new(0.0, -1.0e4, 0.0), 0.0, color, false);
                    let c = &mut self.vehicles.cars[i];
                    c.uid = uid;
                    c.driver = replay::GHOST.to_string();
                    c.ai = None;
                    c.pinned = true;
                    c.placed = true;
                }
            }
            // the opening: the race's own start again - the warp from the camera of the moment
            // onto the grid, the start scene with the race's badge and its 3-2-1 (the original's
            // mcWarper starts "replay recording/playback" alike); the replay's clock runs from GO
            match pb.opening {
                1 => {
                    pb.opening_frames = 0;
                    pb.opening = match self.replay_prerace.clone().filter(|_| self.game.race.is_some() && std::env::var("MCLA_REPLAY_NO_OPENING").is_err()) {
                        Some(p) => {
                            self.cine.stop();
                            self.cine_prerace = false;
                            self.warp = None;
                            self.warp_view = None;
                            self.prerace_plan = None;
                            self.prerace_wait = 0;
                            // (the scene the race played, not a new pick)
                            self.prerace_replay = true;
                            self.game.prerace = Some(p);
                            2
                        }
                        None => 0,
                    };
                }
                2 => {
                    pb.opening_frames += 1;
                    let busy = self.game.prerace.is_some() || self.warp.is_some() || (self.cine_prerace && self.cine.playing.is_some());
                    let go = self.game.race.as_ref().map_or(0.0, |r| r.countdown);
                    // (the recording runs with the countdown: the cars as they stood at this count)
                    pb.t = (self.game.replay_rec.countdown0 - go).max(0.0);
                    if pb.opening_frames > 3 && (!busy || go <= 0.0) {
                        pb.opening = 0;
                    }
                }
                _ => {}
            }
            self.game.replay_rec.apply(pb.t, &mut self.vehicles.cars);
        } else if std::mem::take(&mut self.game.replay_rec.restore) {
            // (the race is over: its clock's countdown as the opening left it goes)
            if let Some(r) = self.game.race.as_mut() {
                r.countdown = 0.0;
            }
            self.game.prerace = None;
            self.warp = None;
            self.warp_view = None;
            if self.cine_prerace {
                self.cine.stop();
                self.cine_prerace = false;
                self.cine_free = false;
                self.cine_caption = None;
            }
            self.vehicles.cars.retain(|c| !c.driver.starts_with(replay::GHOST));
            let saved = std::mem::take(&mut self.game.replay_rec.saved);
            for c in self.vehicles.cars.iter_mut() {
                if let Some((_, s, lean)) = saved.iter().find(|x| x.0 == c.uid) {
                    c.state = s.clone();
                    c.lean = *lean;
                    c.pose_history.begin(&c.state, c.lean);
                }
            }
        }
        if self.game.mode != Mode::Results {
            // (races are run in traffic too; their cars are pinned and never despawned)
            self.vehicles.hour = self.time_of_day;
            // each vehicle its own paint, and the paint on the car as it stands now
            self.game.sync_car_paint();
            self.game.sync_extra_parts();
            self.game.sync_part_paint();
            // (the camera in use and the clock are the save's options: `mc4`)
            if std::env::var("MCLA_CAM_MODE").is_err() {
                self.game.mc4_camera_sync(&mut self.cam_mode);
            }
            self.game.mc4_clock_sync(&mut self.time_of_day, self.opt.time.is_some());
            // (F-54212: a Pull Over tactic (type 1) or a pursuit (types 2 - 4) on the player - payback
            // goons' StartPursuit included; Search / Busted taken as the pursuit's tactic still running)
            self.game.police_on_player = matches!(self.police.heat, police::Heat::PullOver | police::Heat::Pursuit | police::Heat::Search | police::Heat::Busted);
            self.game.police_fine = if self.police.goons { 0 } else { self.police.fine as i64 };
            // (step 23: the slow motion, the tutorials, the statistics and achievements)
            self.game.mc4_s23_tick(dt, mc4::S23In { heat: if self.police.goons { 0 } else { self.police.heat as u8 }, near: self.roam.near.is_some(), flashed: self.roam.accepted, kind: self.roam.near_type, station: self.at_station.is_some(), drive: mc4b::drive_of(self.vehicles.player()), sst: self.vehicles.player().filter(|c| vehicles::Vehicles::has_sst(c)).and_then(|c| c.state.veh.as_ref()).map(|v| v.sst.level) });
            // (step 23b: AudioManager_SetActiveTimeSlow - the dynamic mixer's slow-down state while
            // a tutorial holds the game)
            if let Some(a) = self.audio.as_mut() {
                a.set_time_slow(self.game.mc4b_time_slow());
            }
            if let Some(l) = self.sound.lib.as_mut() {
                l.set_time_slow(self.game.mc4b_time_slow());
            }
            // (step 22b: Downtown's scripted pink slip - its race starts after its scene; the
            // opponent is deactivated after his steps, Racer_Deactivate)
            if self.game.mc4_s23_take_start() {
                self.pending_action = MenuAction::StartRace;
            }
            if self.game.mode == Mode::Race && self.cuts.playing.is_none() && self.game.mc4_dt_pink_step() {
                if let Some(i) = self.game.race.as_ref().and_then(|r| r.participants.get(1).copied()) {
                    if let Some(c) = self.vehicles.cars.get_mut(i) {
                        c.ai = None;
                    }
                }
            }
            // (a stored vehicle keeps its damage: `mc4`)
            if !self.game.borrowed && self.game.test_drive.is_none() {
                if let Some(p) = self.vehicles.player_mut() {
                    let name = p.model.name.clone();
                    self.game.mc4_damage(&name, &mut p.state.damage, &mut p.state.zones);
                }
            }
            // (step 19d: a borrowed vehicle in a set-up wears the set-up's own paints, rims'
            // finish, cabin, tires and brakes - `mc4_borrowed_look` - in place of the record's
            // of the player's own vehicle)
            let s19d_look = self.game.mc4_borrowed_look();
            self.vehicles.player_rim_finish = self.game.rim_finish();
            // (the five colour groups, INTERIOR COLOR A..E: `mc4`)
            self.vehicles.player_interior = self.game.interior_groups();
            let tire = s19d_look.as_ref().map_or_else(|| self.game.tire_set(), |l| l.1.clone());
            self.vehicles.set_player_tire(&mut gpu, &mut world, &tire);
            let brake = match s19d_look.as_ref() {
                Some(l) => l.2.clone(),
                None => self.game.brake_model(),
            };
            self.vehicles.set_player_brake(&mut gpu, &mut world, brake.as_deref());
            // In the garage the part under a list's cursor is on the car at once (the game's
            // Install...OnVehicle as a row is entered): the car is built again when its parts,
            // rim or rim size differ from what it stands in - where it stands.
            if self.game.mode == Mode::Garage && self.game.selected_car == self.game.current_car && !self.game.borrowed && self.cuts.playing.is_none() {
                let now = (self.game.current_car, self.game.cfg.clone(), self.game.rim_inches);
                match self.garage_built.as_ref() {
                    None => self.garage_built = Some(now),
                    Some(b) if *b != now => {
                        if let Some(dir) = self.game.car_dirs.get(self.game.current_car).cloned() {
                            let color = self.game.paint_color();
                            let rim = self.game.rim_dirs.get(self.game.cfg.rim % self.game.rim_dirs.len().max(1)).cloned();
                            self.vehicles.player_rim_inches = self.game.rim_inches;
                            let stand = self.vehicles.player().map(|c| (c.state.pos, c.state.rot));
                            if self.vehicles.replace_player(&mut gpu, &mut world, &dir, color, &now.1, rim.as_deref()).is_ok() {
                                if let (Some((pos, rot)), Some(c)) = (stand, self.vehicles.player_mut()) {
                                    let old = std::mem::replace(&mut c.state, physics::car::CarState::new(pos, 0.0));
                                    c.state.keep_damage(&old);
                                    c.pose_history.reset();
                                    c.state.rot = rot;
                                    c.placed = true;
                                }
                                let tuned = self.game.perf_frac(self.game.current_car);
                                if let Some(p) = self.vehicles.player_mut() {
                                    p.set_perf(tuned);
                                }
                            }
                        }
                        self.garage_built = Some(now);
                    }
                    _ => {}
                }
            } else if self.game.mode != Mode::Garage {
                self.garage_built = None;
            }
            if !self.game.borrowed && self.game.selected_car == self.game.current_car {
                let c = self.game.paint_color();
                if let Some(p) = self.vehicles.player_mut() {
                    if p.color != c {
                        p.color = c;
                    }
                }
            }
            // (MCLA_PAINT_TYPE=1..5: a paint type for screenshots, not saved)
            // (the six paint areas as the vehicle's record has them and the shop changes them: `mc4`)
            self.game.mc4_paint_sync();
            let po = self.game.paint_out();
            self.vehicles.player_paint_type = std::env::var("MCLA_PAINT_TYPE").ok().and_then(|v| v.parse().ok()).unwrap_or(po.ty);
            // (MCLA_PAINT_FADE=1..4 likewise)
            let fade = std::env::var("MCLA_PAINT_FADE").ok().and_then(|v| v.parse().ok()).unwrap_or(po.fade);
            let c2 = po.b;
            self.vehicles.player_paint_b = [c2[0], c2[1], c2[2], fade as f32];
            self.vehicles.player_area_a = po.area_a;
            self.vehicles.player_area_b = po.area_b;
            self.vehicles.player_tint = self.game.tint_rgba();
            if let Some((l, _, _)) = s19d_look.as_ref() {
                self.vehicles.player_rim_finish = l.rim_finish;
                self.vehicles.player_interior = l.interior;
                self.vehicles.player_paint_type = l.paint_type;
                self.vehicles.player_paint_b = l.color2;
                self.vehicles.player_area_a = l.area_a;
                self.vehicles.player_area_b = l.area_b;
                self.vehicles.player_tint = l.tint;
            }
            crate::ticktimes::mark(crate::ticktimes::S::Traffic);
            self.vehicles.manage_traffic(&gpu, &world, focus, sim_dt);
            crate::ticktimes::mark(crate::ticktimes::S::Other);
            self.vehicles.manage_parked(&gpu, focus);
        }
        if self.tick_no == 20 && std::env::var("MCLA_WHOOSH_STAT").is_ok() {
            if let Some(nav) = world.nav.as_ref() {
                let cross = |o: Vec3, u: Vec3, w: Vec3| (u.x - o.x) * (w.z - o.z) - (u.z - o.z) * (w.x - o.x);
                let (mut n, mut crossing, mut along, mut lens) = (0, 0, 0, 0.0f32);
                for list in self.whoosh.values() {
                    for (a, b) in list {
                        n += 1;
                        lens += a.distance(*b);
                        let m = (*a + *b) * 0.5;
                        let d = (*b - *a).normalize_or(Vec3::X);
                        let mut c = false;
                        let mut al = false;
                        for li in nav.links_near(m, a.distance(*b) * 0.5 + 30.0) {
                            let l = &nav.links[li];
                            if (l.a.y - a.y).abs() > 14.0 {
                                continue;
                            }
                            if cross(*a, *b, l.a) * cross(*a, *b, l.b) < 0.0 && cross(l.a, l.b, *a) * cross(l.a, l.b, *b) < 0.0 {
                                c = true;
                            }
                            if l.dir().dot(d).abs() > 0.9 && l.point_at(l.project(m)).distance(m) < 12.0 {
                                al = true;
                            }
                        }
                        crossing += c as i32;
                        along += (al && !c) as i32;
                    }
                }
                log::info!("whoosh stat: {} lines, mean length {:.1}, {} cross a lane, {} only run along one", n, lens / n.max(1) as f32, crossing, along);
            }
        }
        if self.tick_no % 120 == 0 && std::env::var("MCLA_WHOOSH_TEST").is_ok() {
            if let Some(pp) = self.vehicles.player().map(|c| c.state.pos) {
                let mut best = (f32::MAX, Vec3::ZERO, Vec3::ZERO);
                for list in self.whoosh.values() {
                    for (a, b) in list {
                        let d = ((*a + *b) * 0.5).distance(pp);
                        if d < best.0 {
                            best = (d, *a, *b);
                        }
                    }
                }
                log::info!("whoosh test: player {:.0?} nearest line {:.0} m away: {:.0?} - {:.0?}; prev {:.0?} cool {:.1}", pp, best.0, best.1, best.2, self.whoosh_prev, self.whoosh_cool);
            }
        }
        // ---- rushing under an overpass, a gantry sign, into a tunnel: the whoosh ----
        // driving hard through one of the dispatcher's places puts a report on the air
        // (the city's global triggers, run by the trigger manager every frame, F-55598: a DISPATCH sound of a type-3
        // object always starts a Dispatch to Location on the player, F-94691 -> F-86831, which plays the call)
        let pp = self.vehicles.player().map(|c| c.state.pos);
        if let (Some(q), Some(pp)) = (self.dispatch_prev, pp) {
            if self.police.enabled && matches!(self.game.mode, Mode::FreeRoam | Mode::Race) {
                if let Some(call) = racescript::dispatch_test(&mut self.dispatch_zones, q, pp) {
                    log::info!("police radio: global trigger {}", call);
                    self.police.dispatch_call = Some(call);
                }
            }
        }
        self.dispatch_prev = pp;
        self.whoosh_cool -= dt;
        if let Some((pp, speed)) = self.vehicles.player().map(|c| (c.state.pos, c.state.speed())) {
            let q = self.whoosh_prev;
            self.whoosh_prev = pp;
            if (speed > 16.0 || (speed > 3.0 && std::env::var("MCLA_WHOOSH_TEST").is_ok())) && self.whoosh_cool <= 0.0 && q.distance(pp) < 12.0 && matches!(self.game.mode, Mode::FreeRoam | Mode::Race) {
                let (cx, cz) = ((pp.x / 128.0).floor() as i32, (pp.z / 128.0).floor() as i32);
                let cross = |o: Vec3, u: Vec3, w: Vec3| (u.x - o.x) * (w.z - o.z) - (u.z - o.z) * (w.x - o.x);
                let mut hit = false;
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = self.whoosh.get(&(cx + dx, cz + dz)) {
                            // crossing the line, or brushing past one that runs beside the road
                            let near = |p: Vec3, a: Vec3, b: Vec3| {
                                let ab = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
                                let t = (Vec3::new(p.x - a.x, 0.0, p.z - a.z).dot(ab) / ab.length_squared().max(0.01)).clamp(0.0, 1.0);
                                Vec3::new(a.x + ab.x * t - p.x, 0.0, a.z + ab.z * t - p.z).length()
                            };
                            hit |= list.iter().any(|(a, b)| (pp.y - a.y).abs() < 12.0 && ((cross(*a, *b, q) * cross(*a, *b, pp) < 0.0 && cross(q, pp, *a) * cross(q, pp, *b) < 0.0) || (near(pp, *a, *b) < 3.5 && near(q, *a, *b) >= 3.5)));
                        }
                    }
                }
                if hit {
                    self.whoosh_cool = 0.6;
                    if let Some(a) = self.audio.as_mut() {
                        if let Some(k) = a.pick("special_effects/bnk_special_effects", "whoosh_prop_large") {
                            a.play_ex(&k, (0.25 + speed / 70.0).min(0.9), 0.9 + speed / 200.0);
                        }
                    }
                    self.whooshes_heard += 1;
                    log::debug!("whoosh at {:?} ({:.0} m/s)", pp, speed);
                }
            }
        }
        // ---- collectables: drive through a barrel to take it ----
        if matches!(self.game.mode, Mode::FreeRoam | Mode::Race) {
            if let Some(pp) = self.vehicles.player().map(|c| c.state.pos) {
                let hit = self.barrels.iter().enumerate().find(|(i, b)| !self.game.collected.contains(&(*i as u16)) && Vec3::new(b.x - pp.x, (b.y - pp.y) * 0.5, b.z - pp.z).length() < 2.6).map(|(i, b)| (i, *b));
                if let Some((i, b)) = hit {
                    self.game.collected.push(i as u16);
                    self.props_dirty = true;
                    self.fx.sparks(b + Vec3::Y * 0.6, Vec3::Y, 40);
                    let msg = self.game.career.strings.get("COLLECTABLE_FOUND_MSG").map(|s| s.to_string()).unwrap_or_else(|| "COLLECTABLE FOUND".to_string());
                    self.game.show_message(&format!("{}   {} / {}", msg.to_uppercase(), self.game.collected.len(), self.barrels.len()));
                    if let Some(a) = self.audio.as_mut() {
                        // (MiscTasks/Collectables @0x7e3: AudioManager_PlayFrontendSound)
                        if !a.obj("COLLECTIBLES_PICKUP_MASTER") {
                            a.play("ui/menus/rank_up", 0.8);
                        }
                    }
                    self.game.write_save(&self.opt.data_dir);
                }
            }
        }
        if std::mem::take(&mut self.props_dirty) {
            let mut items: Vec<(String, Vec3, glam::Quat)> = self.dress.as_ref().map(|d| d.d.props.clone()).unwrap_or_default();
            for (i, b) in self.barrels.iter().enumerate() {
                if !self.game.collected.contains(&(i as u16)) {
                    items.push(("la_prop_col_barrel_01x".to_string(), *b, glam::Quat::IDENTITY));
                }
            }
            self.props.set_race_props(&gpu, &mut world, &items);
        }
        // (the hangouts' barriers shut the lots only while the police are after the player, and
        // a local hangout's while one is in it - MAP_WORK.md pass 6)
        {
            let chase = matches!(self.police.heat, police::Heat::Pursuit | police::Heat::Search | police::Heat::Busted);
            let inside = self.hangout_in.filter(|_| self.game.mode == Mode::RaceSelect).map(|h| self.hangouts[h].1);
            self.props.set_barriers(&mut world, chase, inside);
        }
        // ---- the race's script: its traffic, lights and nitro points ----
        if self.game.race.is_none() && self.dress.is_some() {
            // (Race_UnloadAmbientPathArray: the paths' items go, racescript.rs `Runtime::unload`)
            if let Some(rt) = self.dress.take() {
                rt.unload(&mut self.vehicles);
            }
            self.props_dirty = true;
            self.vehicles.signals.release();
                self.vehicles.road_density.clear();
                self.vehicles.road_density_new.clear();
            self.props.signals.release();
        }
        if let (Some(rt), Some(race), true) = (self.dress.as_mut(), self.game.race.as_ref(), self.game.mode == Mode::Race && sim_dt > 0.0) {
            let lap = (race.next.first().copied().unwrap_or(0) / race.lap_len.max(1)) as i32 + 1;
            let time = if race.countdown > 0.0 { -race.countdown } else { race.time };
            let subjects = race.participants.clone();
            crate::ticktimes::mark(crate::ticktimes::S::RaceScript);
            rt.update(&mut self.vehicles, &gpu, &world, &mut self.props.signals, &subjects, time, lap, sim_dt);
            crate::ticktimes::mark(crate::ticktimes::S::Other);
            // (a race script's DISPATCH sound starts a Dispatch to Location too, F-94691 -> F-86831)
            for call in std::mem::take(&mut rt.radio) {
                self.police.dispatch_call = Some(call);
            }
            if self.tick_no % 3 == 0 {
                let cam = self.cam.pos;
                for f in rt.flares().filter(|f| f.distance(cam) < 160.0) {
                    self.fx.flare_smoke(f + Vec3::Y * 0.1, [1.0, 0.16, 0.05], 0.8);
                }
            }
            if self.tick_no % 600 == 0 {
                log::info!("race script: {} scripted cars started, {} junctions held, {} nitro cues, {} whooshes", rt.started, rt.lights, rt.boosts, self.whooshes_heard);
                log::info!("race script: {} road densities set ({} lanes)", rt.roads, self.vehicles.road_density.len());
            }
        }
        // (the red light / freeway man's taunts in the race, and RACE BACK - althk.rs)
        if self.game.mode == Mode::Race && sim_dt > 0.0 {
            let chased = matches!(self.police.heat, police::Heat::Pursuit | police::Heat::Search);
            if let Some(sc) = self.game.alt_taunt_tick(&self.vehicles, chased, self.speech.busy()) {
                if self.speech.say_as(sc, &self.game.althk.voice) {
                    self.game.alt_taunt_said(sc);
                }
            }
        }
        // (test hooks: MCLA_ALT_SHOT=menu - the screenshot once the race-over menu offers RACE
        // BACK; MCLA_RACE_BACK=1 - RACE BACK picked there)
        if self.game.mode == Mode::Results && self.game.alt_race_back_offered() && !self.game.reward_panel && self.game.mission_results.is_empty() {
            self.alt_menu_frames += 1;
            if self.alt_menu_frames == 90 && std::env::var("MCLA_ALT_SHOT").map_or(false, |v| v == "menu") {
                self.game.shot_now = true;
            }
            if self.alt_menu_frames == 120 && std::env::var("MCLA_RACE_BACK").is_ok() {
                if std::env::var("MCLA_RACE_BACK").map_or(false, |v| v == "cruise") {
                    log::info!("test: RETURN TO CRUISE picked");
                    self.pending_action = MenuAction::StartFreeRoam;
                } else {
                    log::info!("test: RACE BACK picked");
                    self.game.althk.back_req = true;
                }
            }
        } else {
            self.alt_menu_frames = 0;
        }
        if std::mem::take(&mut self.game.althk.back_req) && self.pending_action == MenuAction::None {
            let men = self.roam.men.clone();
            if let Some(st) = self.game.alt_race_back(&self.roam.alt, &mut self.redlights, &self.vehicles, &men) {
                let id = st.def.id.clone();
                match self.game.race_defs.iter().position(|d| d.id == id) {
                    Some(i) => self.game.race_defs[i] = st.def,
                    None => self.game.race_defs.push(st.def),
                }
                self.game.current_race = self.game.race_defs.iter().position(|d| d.id == id);
                self.game.hookman_race = st.mission;
                self.game.prerace_scene = st.scene;
                self.game.menu_sel = 0;
                self.pending_action = MenuAction::StartRace;
                if let Some(t) = st.announce.and_then(|k| self.game.career.strings.get(&k).map(|s| s.to_string())) {
                    self.news.push_back((t.to_uppercase(), None));
                }
                if let Some(sp) = st.speech {
                    self.speech.say_as(&sp, &self.game.althk.voice);
                }
            }
        }
        let mut race_finished = false;
        // ---- flag games ----
        if self.game.mode == Mode::Race && sim_dt > 0.0 {
            let done = self.game.flag.as_mut().and_then(|fg| fg.update(&mut self.vehicles, sim_dt));
            if let Some(res) = done {
                let won = flags::FlagGame::player_won(&res);
                let sides = res.len().max(2);
                self.game.award_race(if won { 1 } else { sides }, sides);
                // (a team game's panel is form 58, the end of battle, index 3 of the race-over list: up for its
                // WAITUNWARPED(4400) instead of the standings' 2200 - F-34603)
                if let Some(t) = self.game.flag.as_ref().and_then(|fg| fg.team_scores()) {
                    self.game.eob = Some(t);
                    self.game.place_splash = self.game.place_splash.map(|(p, left)| (p, left + 2.2));
                }
                self.game.career.event(career::Ev::Rep(self.game.rep));
                // (the results screen stamps a win when the first row starts with "YOU")
                let mut res = res;
                if !won {
                    if let Some(i) = res.iter().position(|r| r.0.starts_with("YOU")).filter(|&i| i == 0) {
                        // a draw is no win
                        let j = 1.min(res.len() - 1);
                        res.swap(i, j);
                    }
                }
                self.game.last_results = res;
                self.game.results_plain = true;
                self.game.flag = None;
                self.vehicles.frozen = false;
                self.game.mode = Mode::Results;
                self.game.write_save(&self.opt.data_dir);
            }
        }
        if self.game.mode == Mode::Race && sim_dt > 0.0 {
            // (for VIEW REPLAY: the race's cars as they run)
            if let Some(r) = self.game.race.as_ref() {
                self.game.replay_rec.record(sim_dt, r.time, r.countdown, &self.vehicles.cars, &r.participants);
            }
        }
        if self.game.mode == Mode::Race {
            let ready = self.vehicles.player().map_or(true, |c| world.ready_at(c.state.pos));
            race_finished = self.game.update_race(&mut self.vehicles, sim_dt, ready);
            // (the race scripts' CalculateRankOrdered / Unordered for racer 0 while the player has not finished and the
            // race is not over: the lead-change count kLED - F-37138, F-90700)
            if let Some(r) = self.game.race.as_ref() {
                let pi = self.vehicles.cars.iter().position(|c| c.is_player);
                let done = pi.map_or(true, |i| r.finish_time.get(i).copied().flatten().is_some());
                if !done && !race_finished && self.game.race_end_t.is_none() {
                    let (rank, _) = self.game.player_position(&self.vehicles);
                    self.game.mc4_lead_rank(rank as i32);
                }
            }
            self.game.ui_sim_dt += sim_dt;
            // (test hook MCLA_GO_TRACE=1: the race clock and the player's car around GO, every tick)
            if std::env::var("MCLA_GO_TRACE").is_ok() {
                if let (Some(r), Some(c)) = (self.game.race.as_ref(), self.vehicles.player()) {
                    if r.countdown < 4.0 && r.time < 1.0 && self.game.prerace.is_none() {
                        log::info!("go trace: tick {} countdown {:.4} time {:.4} hold {:?} speed {:.3} thr {:.2} brk {:.2} scene {:?}", self.tick_no, r.countdown, r.time, self.vehicles.grid_hold.map(|h| (h * 1000.0).round() / 1000.0), c.state.vel.length(), c.input.throttle, c.input.brake, self.cine.playing.as_ref().map(|sc| (sc.name.clone(), (sc.t * 1000.0).round() / 1000.0)));
                    }
                }
            }
            if !(self.cine_prerace && self.cine.playing.is_some()) {
                self.game.scene_hud_off = false;
            }
            // (the original places the racers 0.8 s before its start scene begins - the player's engine
            // starts with the scene, grid_hold_rule.md 1; the port's dive in front of the scene stands for
            // the arrival, so the racers are held with no pedals until 0.8 s before its end)
            self.vehicles.grid_wait = self.game.prerace.is_some() && self.warp.is_some();
            if std::env::var("MCLA_STUCK_DUMP").is_ok() && self.tick_no % 60 == 0 {
                if let Some(race) = self.game.race.as_ref().filter(|r| r.time > 6.0) {
                    for &ci in race.participants.iter().skip(1) {
                        let Some(c) = self.vehicles.cars.get(ci) else { continue };
                        if c.state.speed() < 1.5 && race.finish_time.get(ci).map_or(true, |f| f.is_none()) {
                            let f = c.state.forward();
                            let ahead = world.collider.raycast(c.state.pos + Vec3::Y * 0.5, f, 12.0).map(|h| h.t);
                            let cars: Vec<String> = self.vehicles.cars.iter().enumerate().filter(|(j, o)| *j != ci && o.state.pos.distance(c.state.pos) < 9.0).map(|(j, o)| format!("car {} {} {:.1} m speed {:.1} parked {} racer {} pinned {} input {:?}", j, o.model.name, o.state.pos.distance(c.state.pos), o.state.speed(), o.parked.is_some(), o.racer, o.pinned, (o.input.throttle, o.input.brake, o.input.handbrake))).collect();
log::info!("stuck2: car {} {} grounded {} up.y {:.2} speed {:.2} rpm {:.0} veh {:?} loads {:?}", ci, c.model.name, c.state.wheels.iter().filter(|w| w.grounded).count(), c.state.up().y, c.state.speed(), c.state.rpm, c.state.veh.as_ref().map(|v| (v.gear, v.omega, v.clutch, v.auto_brake)), c.state.wheels.iter().map(|w| w.load as i32).collect::<Vec<_>>());
                                                        log::info!("stuck: car {} at ({:.0},{:.0},{:.0}) t {:.0} input thr {:.1} brk {:.1} hb {} steer {:.2} gear {} dodge {:.1} wall ahead {:?} cars {:?} props {:?}", ci, c.state.pos.x, c.state.pos.y, c.state.pos.z, race.time, c.input.throttle, c.input.brake, c.input.handbrake, c.input.steer, c.state.gear, c.ai.as_ref().map_or(0.0, |a| a.dodge), ahead, cars, self.props.describe_near(c.state.pos, 5.0));
                        }
                    }
                }
            }
            // (diagnostics: MCLA_PROPS_BOX=x0,z0,x1,z1 - the props standing in that box, once)
            if self.tick_no == 200 {
                if let Some(b) = std::env::var("MCLA_PROPS_BOX").ok().map(|v| v.split(',').filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>()).filter(|b| b.len() == 4) {
                    for d in self.props.describe_in(Vec3::new(b[0], -1e9, b[1]), Vec3::new(b[2], 1e9, b[3])) {
                        log::info!("props box: {d}");
                    }
                }
            }
            // what stands beside the road around the racers (they steer round it)
            if self.tick_no % 8 == 0 {
                let mut solids = std::mem::take(&mut self.vehicles.solids);
                solids.clear();
                if let Some(race) = self.game.race.as_ref() {
                    for &ci in race.participants.iter().skip(1) {
                        if let Some(c) = self.vehicles.cars.get(ci) {
                            if world.ready_at(c.state.pos) {
                                self.props.solid_near(c.state.pos + c.state.forward() * 18.0, 34.0, &mut solids);
                            }
                        }
                    }
                }
                self.vehicles.solids = solids;
                // the fixed props around the racers, for their brains' obstacle scan (kind 3 of the
                // object grid, filed while the prop is in level state 2 - F-47614)
                // (approx.: the original files the props of the streamed-in city; here every fixed
                // prop within 250 m of a racer - the scan reaches ~6 s of line ahead + 25 m)
                let mut objs = std::mem::take(&mut self.vehicles.prop_objects);
                objs.clear();
                if let Some(race) = self.game.race.as_ref() {
                    let mut seen = hashbrown::HashSet::new();
                    for &ci in race.participants.iter().skip(1) {
                        if let Some(c) = self.vehicles.cars.get(ci).filter(|c| c.ai.as_ref().map_or(false, |a| a.brain.is_some())) {
                            self.props.fixed_objects_near(c.state.pos, 250.0, &mut seen, &mut objs);
                        }
                    }
                }
                self.vehicles.prop_objects = objs;
            }
            // the race's cop zone (F-88125, F-71212): one zone of the race's, on one lap; the first
            // racer in it sets it off and the pursuit starts 5.5 s later (police.rs `zone_step`)
            if self.game.race.is_none() {
                self.police.zone = None;
                self.police.zone_picked = false;
            } else if self.police.enabled && !self.vehicles.frozen {
                let race = self.game.race.as_mut().unwrap();
                if !self.police.zone_picked {
                    self.police.zone_picked = true;
                    self.police.pick_zone(race.cop_zones.len(), race.laps);
                }
                if let Some(z) = self.police.zone.clone().filter(|_| race.countdown <= 0.0) {
                    let lap = race.next.first().map_or(0, |&n| n / race.lap_len.max(1));
                    let (zp, zr) = (race.cop_zones[z.idx].0, race.cop_zones[z.idx].1);
                    let inside = race.participants.iter().find_map(|&ci| {
                        self.vehicles.cars.get(ci).filter(|c| c.state.pos.distance_squared(zp) < zr * zr).map(|c| c.is_player)
                    });
                    if let Some(line) = self.police.zone_step(lap, inside, sim_dt) {
                        if !self.speech.busy() {
                            self.speech.say(line);
                        }
                    }
                    // (approx.: a cruiser waits at the armed zone once the player comes near - which
                    // unit the original sends, from where, was not read)
                    let pp = self.vehicles.player().map(|c| c.state.pos).unwrap_or(Vec3::ZERO);
                    if z.state == 1 && !z.stationed && self.tick_no % 30 == 0 && zp.distance(pp) < 320.0 && zp.distance(pp) > 90.0 && world.ready_at(zp) {
                        let ok = self.police.station(&gpu, &world, &mut self.vehicles, zp, z.idx);
                        if let Some(zz) = self.police.zone.as_mut() {
                            zz.stationed = ok;
                        }
                    }
                }
            }
            // Off-road stretches of the racing lines get their real line as soon as the city
            // around them is loaded (one per frame).
            if let (Some(race), Some(nav)) = (self.game.race.as_mut(), world.nav.as_ref()) {
                let now = race.time.max(0.0) + (4.5 - race.countdown.clamp(0.0, 4.5));
                let line_of = |vehicles: &vehicles::Vehicles, race: &game::Race, car: usize| if car == usize::MAX { race.path.clone() } else { vehicles.cars.get(car).and_then(|c| c.ai.as_ref()).and_then(|a| a.path.clone()) };
                let pick = race.shortcuts.iter().position(|s| {
                    if s.done || now < s.retry {
                        return false;
                    }
                    line_of(&self.vehicles, race, s.car).and_then(|p| p.get(s.from).copied()).map_or(false, |a| world.ready_at(a))
                });
                if let Some(k) = pick {
                    let s = race.shortcuts[k].clone();
                    if let Some(old) = line_of(&self.vehicles, race, s.car).filter(|p| s.to < p.len() && s.from < s.to) {
                        let (a, b) = (old[s.from], old[s.to]);
                        let b_ready = world.ready_at(b);
                        let mn = a.min(b) - Vec3::splat(50.0);
                        let mx = a.max(b) + Vec3::splat(50.0);
                        let solid = self.props.solid_in(mn, mx);
                        let t0 = std::time::Instant::now();
                        let key = [(a.x * 4.0) as i32, (a.z * 4.0) as i32, (b.x * 4.0) as i32, (b.z * 4.0) as i32];
                        let planned = match race.shortcut_lines.get(&key) {
                            Some(l) => Some((l.clone(), true)),
                            None => offroad::plan(&world.collider, &solid, &[a, b]),
                        };
                        if let Some((l, true)) = planned.as_ref() {
                            race.shortcut_lines.entry(key).or_insert_with(|| l.clone());
                        }
                        let (new, done, how): (Option<Vec<Vec3>>, bool, &str) = match planned {
                            Some((l, true)) => (Some(l), true, "through"),
                            Some((mut l, false)) if !b_ready => {
                                // as far as the city is loaded; the rest stays straight for now
                                let e = *l.last().unwrap();
                                let steps = (e.distance(b) / 8.0).ceil().max(1.0) as usize;
                                for q in 1..steps {
                                    l.push(e.lerp(b, q as f32 / steps as f32));
                                }
                                l.push(b);
                                (Some(l), false, "part of the way")
                            }
                            None if !b_ready => (None, false, "not yet"),
                            _ => {
                                // no way through (a wall the model has and the route does not): by road
                                let d = b - a;
                                let (l, missing) = nav.route_through(a, Vec3::new(d.x, 0.0, d.z).normalize_or(Vec3::X), &[b], 9.0);
                                (if missing == 0 && l.len() >= 2 { Some(l) } else { None }, true, "round by road")
                            }
                        };
                        if done || std::env::var("MCLA_COURSE_DUMP").is_ok() {
                            log::info!("race line: off-road stretch of {:.0} m for car {}: {} ({} points, {:.1} ms)", a.distance(b), if s.car == usize::MAX { -1 } else { s.car as i64 }, how, new.as_ref().map_or(0, |n| n.len()), t0.elapsed().as_secs_f32() * 1000.0);
                        }
                        if let Some(new) = new {
                            let n_new = new.len();
                            let delta = n_new as isize - (s.to - s.from + 1) as isize;
                            let mut path: Vec<Vec3> = old[..s.from].to_vec();
                            path.extend(new);
                            path.extend_from_slice(&old[s.to + 1..]);
                            let path = std::sync::Arc::new(path);
                            // whoever drives this line: keep his place on it
                            let target = if s.car == usize::MAX { self.vehicles.cars.iter().position(|c| c.is_player && c.ai.as_ref().map_or(false, |a| a.path.as_ref().map_or(false, |p| std::sync::Arc::ptr_eq(p, &old)))) } else { Some(s.car) };
                            if s.car == usize::MAX {
                                race.path = Some(path.clone());
                            }
                            if let Some(c) = target.and_then(|i| self.vehicles.cars.get_mut(i)) {
                                let pos = c.state.pos;
                                let is_player = c.is_player;
                                if let Some(ai) = c.ai.as_mut() {
                                    if ai.path_i > s.to {
                                        ai.path_i = (ai.path_i as isize + delta).max(0) as usize;
                                    } else if ai.path_i >= s.from {
                                        // in the middle of it: onto the nearest point of the new line
                                        let near = (s.from..s.from + n_new).min_by(|x, y| path[*x].distance_squared(pos).partial_cmp(&path[*y].distance_squared(pos)).unwrap()).unwrap_or(s.from);
                                        ai.path_i = near.min(path.len().saturating_sub(2));
                                        let q = path[ai.path_i];
                                        if !is_player && Vec3::new(q.x - pos.x, 0.0, q.z - pos.z).length() > 6.0 {
                                            let d = path[ai.path_i + 1] - q;
                                            ai.reset_to = Some((q + Vec3::Y * 0.3, (-d.x).atan2(-d.z)));
                                        }
                                    }
                                    ai.path = Some(path);
                                }
                            }
                            for o in race.shortcuts.iter_mut().filter(|o| o.car == s.car && o.from >= s.to) {
                                o.from = (o.from as isize + delta) as usize;
                                o.to = (o.to as isize + delta) as usize;
                            }
                            race.shortcuts[k].to = s.from + n_new - 1;
                        }
                        race.shortcuts[k].done = done;
                        race.shortcuts[k].retry = now + 2.0;
                    } else {
                        race.shortcuts[k].done = true;
                    }
                }
            }
        } else {
            self.vehicles.frozen = false;
        }
        if self.game.race.is_none() && self.tick_no % 60 == 0 {
            // cruisers still waiting for a race that is over go home
            self.vehicles.cars.retain(|c| !(c.cop && c.pinned && c.parked.is_some()));
        }
        if sim_dt > 0.0 {
            // (each cruiser has its own siren target now, `CarInstance::siren` set by the police; this one forces them all)
            self.vehicles.sirens = false;
            // (test hook MCLA_SIREN_TEST: the player's car is a cruiser with its siren on - the police lamps' states and glows)
            if std::env::var("MCLA_SIREN_TEST").is_ok() {
                self.vehicles.sirens = true;
                if let Some(c) = self.vehicles.player_mut() {
                    c.cop = true;
                }
            }
            if self.vehicles.signals.junctions.is_empty() && !self.props.signals.junctions.is_empty() {
                self.vehicles.signals = self.props.signals.clone();
            }
            {
                // the junctions' own state (signals.rs): stepped on the vehicles' copy, shown by the props'
                // (traffic on lanes reports itself: `Signals::wait` when the light refuses it)
                // (the box hold and the player's demand: `Vehicles::step_signals`)
                let race = self.game.race.as_ref().map(|r| (r.countdown, r.time));
                match world.nav.as_ref() {
                    Some(nav) => self.vehicles.step_signals(nav, sim_dt, race),
                    None => self.vehicles.signals.step(sim_dt, &[], None, self.vehicles.player().map(|c| (c.state.pos, c.state.vel)), race),
                }
                self.props.signals.sync_from(&self.vehicles.signals);
            }
            self.props.update_signals(&gpu, focus, self.vehicles.clock);
            // (the particle props: fountains, steam, smoke - MAP_WORK.md)
            let stormy = self.weather_weight(sky::Weather::Stormy);
            self.props.emit(&mut self.fx, self.cam.pos, self.tick_no, self.vehicles.player().map(|c| (c.state.pos, c.state.vel, c.state.rot)), stormy);
            props::set_lens(self.cam.fov_y);
            self.vehicles.hide_player = self.hide_player || (self.game.fresh_game && self.intro_stage != 0) || self.game.starter_pick.is_some();
            // (the scene is over: its animated brains are deleted with it, BrainFactory_DeleteBrain)
            static AFTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if self.cine.playing.is_none() && !self.cine_events.0.is_empty() {
                self.cine_events = (String::new(), Vec::new());
                for c in self.vehicles.cars.iter_mut() {
                    c.brain = None;
                }
                AFTER.store(90, std::sync::atomic::Ordering::Relaxed);
            }
            // (MCLA_CINE_CAR_TRACE: the player's car for 1.5 s after a scene)
            if AFTER.load(std::sync::atomic::Ordering::Relaxed) > 0 && std::env::var_os("MCLA_CINE_CAR_TRACE").is_some() {
                let left = AFTER.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                if let Some(c) = self.vehicles.player().filter(|_| left % 10 == 1) {
                    log::info!("cine car after: {} pos {:.3} {:.3} {:.3} speed {:.3} in {:?}", 90 - left, c.state.pos.x, c.state.pos.y, c.state.pos.z, c.state.speed(), c.input);
                }
            }
            // the control step's engine rules (F-17997): while the engine sound's start machine is below state 4 (the
            // start's wait, the crank, the stop) the engine word +46 is set to 1 and the car is held - throttle 0,
            // brake 1, handbrake 0, the gearbox to first (F-16736, `first_gear`); otherwise with +46 = 0 (a scene's
            // car_off) the throttle is 0
            if self.sound.engine_held() {
                player_input.throttle = 0.0;
                player_input.brake = 1.0;
                player_input.handbrake = false;
                player_input.first_gear = true;
                if let Some(c) = self.vehicles.player_mut() {
                    c.engine_off = false;
                }
            } else if self.vehicles.player().is_some_and(|c| c.engine_off) {
                player_input.throttle = 0.0;
            }
            crate::ticktimes::mark(crate::ticktimes::S::Vehicles);
            self.vehicles.update(&world, player_input, sim_dt);
            crate::ticktimes::mark(crate::ticktimes::S::Other);
            if std::env::var_os("MCLA_STEER_DBG").is_some() && self.tick_no % 30 == 0 {
                log::info!("steer dbg: tick {} drive {} menu {} mode {:?} in steer {:.2} thr {:.2} -> car input steer {:?} state steer {:?} veh steer {:?}", self.tick_no, self.drive, menu_open, self.game.mode, player_input.steer, player_input.throttle, self.vehicles.player().map(|c| c.input.steer), self.vehicles.player().map(|c| c.state.steer), self.vehicles.player().and_then(|c| c.state.veh.as_ref().map(|v| (v.pad_steer, v.steer))));
            }
            crate::ticktimes::mark(crate::ticktimes::S::Props);
            self.props.collide(&gpu, &world, &mut self.vehicles.cars, sim_dt);
            crate::ticktimes::mark(crate::ticktimes::S::Other);
            // ---- the pad's rumble (crate::rumble): the player car's tick asked its effects (hits, landing,
            // nitrous, turbo, posts...), Agro's strikes theirs; then mcFeedback's update F-25579 ----
            {
                let pad = self.gamepad.as_ref().map_or(false, |g| active_pad(g, self.pad_id).is_some());
                let data_dir = self.opt.data_dir.clone();
                let r = self.pad_rumble.get_or_insert_with(|| rumble::PadRumble::new(std::path::Path::new(&data_dir)));
                if let Some(c) = self.vehicles.cars.iter_mut().find(|c| c.is_player) {
                    if let Some(vs) = c.state.veh.as_deref_mut() {
                        for _ in 0..std::mem::take(&mut vs.agro.out.rumble) {
                            c.state.rumble.play(3);
                        }
                    }
                    r.tick(&mut c.state, pad, false);
                }
            }
            // (the loose props as the cars' shared physics level moved them, physics/loose.rs)
            crate::ticktimes::mark(crate::ticktimes::S::Props);
            if let Some(lw) = world.collider.loose.as_deref() {
                self.props.follow_level(&gpu, &self.vehicles.level, lw);
            }
            crate::ticktimes::mark(crate::ticktimes::S::Other);
            // ---- pulling up at a garage opens it ----
            {
                let want = std::mem::take(&mut self.enter_garage);
                if self.game.mode == Mode::FreeRoam && self.game.race.is_none() && self.roam.challenge.is_none() {
                    if let Some(c) = self.vehicles.player() {
                        let p = c.state.pos;
                        let at = world.set_bounds.values().any(|(mn, mx)| p.x > mn.x - 8.0 && p.x < mx.x + 8.0 && p.z > mn.z - 8.0 && p.z < mx.z + 8.0 && p.y > mn.y - 4.0 && p.y < mx.y + 4.0);
                        // (the original takes the car in as it drives up to the door - no key, no
                        // line: driven at walking pace from the lot towards the door the recomp
                        // was in the garage, `ref/gar110_1.png`. The door is armed again once the
                        // car has been well away, or leaving would lead straight back in.
                        // approx.: nine metres from the door.)
                        // (garage_core: within 8 m of the set-up's door point, the police doing nothing with
                        // the player - no speed test; `garagein`. The door is armed again once the car has
                        // been well away - the port's guard against driving straight back in.)
                        // (garage_core 0x37e..0x428: only while the game state is Cruising_Offline (its listener, local 25)
                        // and the core is not holding back after Script_SuspendAllOpponents (0x1e4..0x301: until 8 m
                        // from the door) - which is what keeps a car that has just come out from going straight back in;
                        // the drive-out scene leaves it about 26 m from the door)
                        let held = garagein::at_door(p).map_or(false, |g| self.game.garage_held(g));
                        // (0x3ca..0x3ee: PoliceManager_CheckPoliceActivity(1) > 1 waits 100 ms - a pursuit, a search or
                        // a bust; a pull-over or a dispatch on its way does not stop it - `Police::activity`, F-35540)
                        let quiet = self.police.activity().0 <= 1;
                        let door = garagein::at_door(p).filter(|&g| !self.game.career.loaded() || self.game.garage_running(g));
                        let drive_in = self.game.flash_hud.is_some() && self.game.cruising_offline() && !held && self.garage_in.is_none() && quiet && door.is_some() && self.opt.shot.is_none() | std::env::var("MCLA_DOOR_IN").is_ok();
                        // (state 4, GO TO GARAGE: 0x3a9 goes straight to the drive-in at 0x42d from wherever the car is -
                        // the listener state still first)
                        let goto = self.goto_garage.filter(|_| self.game.cruising_offline() && self.garage_in.is_none());
                        if drive_in || goto.is_some() {
                            // 0x42d: PoliceManager_DestroyAllPolice, ThrowEvent 'garage', Warper_SetIsWarpingToCutscene(1),
                            // Warper_TransitionTo('STORY_LOCATION_CUT_SIMPLE', ..): from the render camera to the garage's
                            // wait camera - Warp_Target_Close within 100 of it (F-47096); else the climb over the car
                            // (type 17's state 15, F-98643) and, the scene loaded, the dive onto it (state 26)
                            let g = goto.or(door).unwrap_or(0);
                            self.goto_garage = None;
                            self.police.destroy_all(&mut self.vehicles);
                            let c = self.vehicles.player().unwrap();
                            let e = garagein::wait_rows(&garagein::SETUPS[g]);
                            let look = (self.chase.look - self.chase.pos).normalize_or(Vec3::NEG_Z);
                            let s = hudmap::CamRows::look_at(self.cam.pos, self.cam.pos + look, self.boom_up.unwrap_or(Vec3::Y));
                            let close = s.eye.distance(e.eye) < garagein::CLOSE;
                            let warp = close.then(|| hudmap::CineWarp::target_close(s, e, self.cam.fov_y, garagein::LENS_DEG.to_radians()));
                            // (far, Warp() sets the request's +96 to 25 and picks the climb as DEFAULT_WITH_CUT does, F-32081;
                            // the climb eases the lens to it)
                            let car = hudmap::CamRows { r0: c.state.rot * Vec3::X, r1: c.state.rot * Vec3::Y, r2: c.state.rot * Vec3::Z, eye: c.state.pos };
                            let climb = (!close).then(|| hudmap::CineClimb::new(s, car, self.cam.fov_y, garagein::FAR_LENS_DEG.to_radians(), self.screen_v));
                            log::info!("garage entry: {} from {:?} (warp {}, climb {:?}, go to garage {})", garagein::SETUPS[g].key, s.eye, warp.is_some(), climb.as_ref().map(|c| c.kind()), goto.is_some());
                            self.garage_in = Some(garagein::Entry { garage: g, stage: garagein::Stage::Warp, t: 0.0, clock: 0.0, warp, hold: (c.state.pos, c.state.rot), opened: false, climb, dive: None });
                        } else if self.game.car_x.garage_after_scene && self.cuts.idle() {
                            self.game.car_x.garage_after_scene = false;
                            self.pending_action = MenuAction::OpenGarage;
                        } else if at && c.state.speed() < 12.0 {
                            if want {
                                self.pending_action = MenuAction::OpenGarage;
                            } else if self.tick_no % 60 == 0 && self.news.is_empty() && self.game.flash_hud.is_none() {
                                self.game.show_message("G  ENTER GARAGE");
                            }
                        }
                    }
                }
            }
            // ---- driving into a garage (`garagein`): the warp to the sign, the event, the scene, the fade up ----
            if let Some(mut gi) = self.garage_in.take() {
                gi.t += dt;
                gi.clock += dt;
                let su = &garagein::SETUPS[gi.garage];
                let mut keep = true;
                match gi.stage {
                    garagein::Stage::Warp => {
                        // (the car stands where the warp found it: Racer_Deactivate)
                        if let Some(c) = self.vehicles.player_mut() {
                            c.state.pos = gi.hold.0;
                            c.state.rot = gi.hold.1;
                            c.state.vel = Vec3::ZERO;
                            c.state.ang_vel = Vec3::ZERO;
                        }
                        // the Lerp, then the camera waits at the wait camera (the scene's first); far, the climb and
                        // then its last matrix held (F-99030)
                        let warp_len = gi.warp.as_ref().map_or(0.0, |w| w.total()).max(gi.climb.as_ref().map_or(0.0, |c| c.total()));
                        let (rows, fov) = match (gi.climb.as_mut(), gi.warp.as_mut().and_then(|w| w.step(dt))) {
                            (Some(c), _) => c.step(dt).unwrap_or_else(|| c.end()),
                            (None, Some(v)) => v,
                            (None, None) => (garagein::wait_rows(su), garagein::LENS_DEG.to_radians()),
                        };
                        self.place_view = Some((rows.eye, rows.look_point(), rows.r1, fov));
                        self.game.dive_k = 1.0;
                        // IsReadyToLoadStuff at the warp's end: LOCATION_EVENT, 500 ms, then <X>_GARAGE_ENTER - a
                        // mission's event script may play its story scene (REDLIGHT_GH_01_GARAGE_ON_ENTER), else
                        // <x>_garage_enter_cine plays the `_ent` scene
                        if gi.t >= warp_len + garagein::EVENT_WAIT {
                            // ("Reached neutral point": garage_core 0x517..0x52a sets the stat tGAR to 1 unless its place
                            // is element 506 - none of the three garages)
                            self.game.mc4_stat_set("tGAR", 0, 1.0);
                            self.game.career.event(career::Ev::Garage);
                            let night = self.time_of_day > 18.5 || self.time_of_day < 5.5;
                            let story = self.game.career.location_enter(su.enter).and_then(|id| self.cuts.for_mission(&id, night)).filter(|_| std::env::var("MCLA_NO_CUTS").is_err() && (self.opt.shot.is_none() || std::env::var("MCLA_CUTS").is_ok()));
                            self.place_view = None;
                            // (far: warper state 15 waits for the scene, hides Career_Loading, then state 26 dives onto
                            // the scene's camera at its t 1.0 - F-62707 - from the held climb's end)
                            let dive_to = gi.climb.as_ref().filter(|_| story.is_none() && std::env::var("MCLA_NO_CINE").is_err()).and_then(|c| {
                                let (eye, fwd, up, fov) = self.cine.peek_camera(su.scene, glam::Mat4::IDENTITY, hudmap::CINE_HANDOVER)?;
                                let (s, lens) = c.end();
                                Some(hudmap::CineWarp::new(s, hudmap::CamRows::look_at(eye, eye + fwd, up), Vec3::ZERO, lens, fov))
                            });
                            if story.is_some() || dive_to.is_none() {
                                self.game.career_loading = false;
                            }
                            if let Some(i) = story {
                                self.cuts.queue(i);
                                self.game.car_x.garage_after_scene = true;
                                keep = false;
                            } else if let Some(w) = dive_to {
                                log::info!("garage entry: dive {:?} ({:.3} s) onto {}", w.kind(), w.total(), su.scene);
                                self.place_view = Some((rows.eye, rows.look_point(), rows.r1, fov));
                                gi.dive = Some(w);
                                gi.stage = garagein::Stage::Dive;
                                gi.t = 0.0;
                            } else if std::env::var("MCLA_NO_CINE").is_err() && self.cine.start(&mut gpu, su.scene, glam::Mat4::IDENTITY) {
                                self.cine_free = true;
                                self.cine_caption = Some(String::new());
                                gi.stage = garagein::Stage::Scene;
                                gi.t = 0.0;
                            } else {
                                self.pending_action = MenuAction::OpenGarage;
                                keep = false;
                            }
                        }
                    }
                    garagein::Stage::Dive => {
                        if let Some(c) = self.vehicles.player_mut() {
                            c.state.pos = gi.hold.0;
                            c.state.rot = gi.hold.1;
                            c.state.vel = Vec3::ZERO;
                            c.state.ang_vel = Vec3::ZERO;
                        }
                        let (s, lens) = gi.climb.as_ref().map(|c| c.end()).unwrap_or((garagein::wait_rows(su), garagein::LENS_DEG.to_radians()));
                        let mut view = (s, lens);
                        if gi.t >= garagein::LOADING_OFF {
                            self.game.career_loading = false;
                        }
                        if gi.t >= garagein::DIVE_WAIT {
                            if let Some(w) = gi.dive.as_mut() {
                                if let Some(v) = w.step(dt) {
                                    view = v;
                                }
                                // (state 19: the scene starts 1 s before the flight lands, which then ends on its camera)
                                if gi.t - garagein::DIVE_WAIT >= w.total() - hudmap::CINE_HANDOVER {
                                    if self.cine.start(&mut gpu, su.scene, glam::Mat4::IDENTITY) {
                                        self.cine_free = true;
                                        self.cine_caption = Some(String::new());
                                        gi.stage = garagein::Stage::Scene;
                                        gi.t = 0.0;
                                    } else {
                                        self.pending_action = MenuAction::OpenGarage;
                                        keep = false;
                                    }
                                }
                            }
                        }
                        self.place_view = Some((view.0.eye, view.0.look_point(), view.0.r1, view.1));
                        self.game.dive_k = 1.0;
                    }
                    garagein::Stage::Scene => {
                        // (the dive still flying over the scene's first second)
                        let flying = gi.dive.as_mut().and_then(|w| w.step(dt));
                        self.place_view = flying.map(|(rows, fov)| (rows.eye, rows.look_point(), rows.r1, fov));
                        if flying.is_none() {
                            gi.dive = None;
                        }
                        // (F-31347 state 17: Career_Loading hidden once the scene's time is >= 0)
                        self.game.career_loading = false;
                        // the scene's fade_to_black (game/misctasks/screen_fade_to_black: held until the fade up)
                        if gi.t >= garagein::FADE_AT {
                            self.garage_mask.start(garagein::FADE_LEN, 0.01, f32::MAX);
                        }
                        if self.cine.playing.is_none() {
                            gi.stage = garagein::Stage::Poll;
                            gi.t = 0.0;
                        }
                    }
                    garagein::Stage::Poll => {
                        // <x>_garage_state 5: garage_core's next poll fades up and opens the garage. (The
                        // original's scene script has streamed the garage in by then - PrestreamManager_UnlockAll
                        // before the state; the port's interior comes in with the garage, so the fade up waits
                        // for the car to stand in it, at most 10 s.)
                        if gi.t >= garagein::STATE_POLL && !gi.opened {
                            gi.opened = true;
                            self.pending_action = MenuAction::OpenGarage;
                        }
                        if gi.opened && (self.garage_return.is_some() || gi.t > 10.0) {
                            self.garage_mask.release(garagein::FADE_UP);
                            keep = false;
                        }
                    }
                }
                // ShowLocationName: popup template 63 with the place's name and district (opened by Warp_Target_Close's
                // Lerp only - transitions/Main.sc.xml: the far warp's climb and dive open none)
                let shown = gi.climb.is_none() && gi.clock >= garagein::TITLE_FROM && gi.clock < garagein::TITLE_FROM + garagein::TITLE_LEN && keep;
                self.game.place_title = shown.then(|| {
                    let st = |k: &str| self.game.career.strings.get(k).map(|v| v.to_string()).unwrap_or_default();
                    (st(su.key), st(&format!("{}_desc", su.key)))
                });
                if keep {
                    self.garage_in = Some(gi);
                }
            }
            // leaving: LeaveGarage's 20 updates and garage_core's poll, then out to the street (the drive-out
            // scene starts below) and its fade_to_scene (at 0, the default 1 s - game/misctasks/screen_fade_to_scene)
            if let Some(t) = self.garage_out.as_mut() {
                *t += dt;
                if *t >= garagein::EXIT_WAIT || self.game.mode != Mode::Garage {
                    self.garage_out = None;
                    if self.game.mode == Mode::Garage {
                        self.game.mode = self.game.prev_mode;
                    }
                    self.garage_mask.release(garagein::FADE_UP);
                }
            }
            if self.garage_in.is_none() && self.hangout_entry.is_none() && self.hangout_title.is_none() && self.payback_in.is_none() && !self.payback_hold {
                self.place_view = None;
                self.game.place_title = None;
            }
            self.game.place_mask = self.garage_mask.update(dt).max(self.arrest_mask.update(dt));
            // (test hook MCLA_ARREST_TRACE=1: the arrest's black each frame it is on)
            if self.arrest_mask.is_on() && std::env::var_os("MCLA_ARREST_TRACE").is_some() {
                log::info!("arrest mask tick {} opacity {:.3} full {} wait {} scene {:?} form {} station {:?} activate {}", self.tick_no, self.game.place_mask, self.arrest_mask.full(), self.arrest_wait, self.cine.playing.as_ref().map(|sc| (sc.name.clone(), sc.t)), self.game.arrest_panel.is_some(), self.arrest_station, self.arrest_activate);
            }
            // (test hook MCLA_GARAGE_TEST=0..2: at 4 s the car is put 14 m from that garage's door point, towards
            // its wait camera, facing away from the door, at 14 s 7 m from it - the place streamed in by then)
            if let Some(g) = std::env::var("MCLA_GARAGE_TEST").ok().and_then(|v| v.parse::<usize>().ok()).filter(|g| *g < 3) {
                // (and MCLA_GARAGE_BACK=tick: BACK out of the garage at that tick)
                if std::env::var("MCLA_GARAGE_BACK").ok().and_then(|v| v.parse::<u64>().ok()) == Some(self.tick_no as u64) && self.game.mode == Mode::Garage {
                    self.pending_action = MenuAction::Back;
                }
                if self.tick_no == 240 || self.tick_no == 840 {
                    let su = &garagein::SETUPS[g];
                    let far = if self.tick_no == 240 { 14.0 } else { 7.0 };
                    let d = Vec3::new(su.cam[0].x - su.door.x, 0.0, su.cam[0].z - su.door.z).normalize_or(Vec3::X);
                    if let Some(c) = self.vehicles.player_mut() {
                        c.state.relocate(su.door + d * far + Vec3::Y * 0.5, glam::Quat::from_rotation_y((-d.x).atan2(-d.z)));
                        c.state.vel = Vec3::ZERO;
                        c.pose_history.reset();
                    }
                }
            }
            // (test hook MCLA_GOTO_GARAGE=tick: GO TO GARAGE at that tick in free roam)
            if std::env::var("MCLA_GOTO_GARAGE").ok().and_then(|v| v.parse::<u64>().ok()) == Some(self.tick_no as u64) {
                log::info!("go to garage test: mode {:?}", self.game.mode);
                if self.game.mode == Mode::FreeRoam {
                    self.pending_action = MenuAction::GoToGarage;
                }
            }
            // (test hook MCLA_GARAGE_SHOTS=dir: a frame every 6 ticks while a garage entry runs and its black is up)
            if let Ok(dir) = std::env::var("MCLA_GARAGE_SHOTS") {
                if (self.garage_in.is_some() || self.hangout_entry.is_some() || self.hangout_title.is_some() || self.game.place_mask > 0.0 || self.cine.playing.is_some()) && self.tick_no % 6 == 0 {
                    log::info!("garage shot {} stage {:?} mask {:.2} title {} placed {:?} car {:?}", self.tick_no, self.garage_in.as_ref().map(|g| (g.stage, g.clock)), self.game.place_mask, self.game.place_title.is_some(), self.garage_return.is_some(), self.vehicles.player().map(|c| c.state.pos));
                    self.photo_request = Some(format!("{}/g_{:05}.png", dir, self.tick_no));
                }
            }
            // ---- garage: the car stands inside the nearest garage interior ----
            if self.game.mode == Mode::Garage && self.garage_return.is_none() && self.drive {
                if let Some((ppos, pfwd)) = self.vehicles.player().map(|c| (c.state.pos, c.state.forward())) {
                    let near = world.set_bounds.iter().map(|(k, b)| (*k, *b)).min_by(|a, b| ((a.1 .0 + a.1 .1) * 0.5).distance_squared(ppos).partial_cmp(&((b.1 .0 + b.1 .1) * 0.5).distance_squared(ppos)).unwrap());
                    if let Some((k, (mn, mx))) = near {
                        // (a set that is still loading may yield nothing yet: only keep hits)
                        let spot = match self.garage_spots.get(&k) {
                            Some(s) => *s,
                            None => {
                                // (every tick while a drive-in waits on it: the fade up waits for the car's place)
                                let s = if self.tick_no % 20 == 0 || self.garage_in.is_some() { garage_spot(&world.collider, mn, mx) } else { None };
                                if s.is_some() {
                                    self.garage_spots.insert(k, s);
                                }
                                s
                            }
                        };
                        if let Some((mut p, mut clear)) = spot {
                            // the game's own spot for the car in this garage, if its scene tells
                            let cine = &self.cine;
                            let poses = self.garage_poses.get_or_insert_with(|| {
                                ["cut_la_hw_oh_garage_01x_ent", "cut_la_be_smb_garage_01x_ent", "sc_cut_garage_01_ent"].iter().filter_map(|s| cine.car_pose(s, &|l| l.contains("garage"), false)).collect()
                            });
                            let mid = (mn + mx) * 0.5;
                            let known = poses.iter().copied().filter(|q| q.0.distance(mid) < 80.0).min_by(|a, b| a.0.distance_squared(mid).partial_cmp(&b.0.distance_squared(mid)).unwrap());
                            let mut rot = glam::Quat::from_rotation_y(0.6);
                            if let Some((kp, kr)) = known {
                                if let Some(g) = world.collider.raycast(kp + Vec3::Y * 1.5, -Vec3::Y, 4.0) {
                                    p = g.point;
                                    rot = kr;
                                    // (the game's own pose in the Hollywood garage, read from the running
                                    // game's car matrix in step 34: 3.3 degrees off the cutscene's)
                                    let seen = (Vec3::new(861.773, 17.005, -809.715), Vec3::new(0.79122, 0.0, -0.61153));
                                    if (kp - seen.0).length() < 2.0 {
                                        p = Vec3::new(seen.0.x, p.y, seen.0.z);
                                        rot = glam::Quat::from_rotation_y((-seen.1.z).atan2(seen.1.x));
                                    }
                                    // room around the car for the camera
                                    clear = 9.0;
                                    for k in 0..12 {
                                        let a = k as f32 / 12.0 * std::f32::consts::TAU;
                                        if let Some(hit) = world.collider.raycast(p + Vec3::Y * 0.9, Vec3::new(a.cos(), 0.0, a.sin()), clear) {
                                            clear = clear.min(hit.t);
                                        }
                                    }
                                    clear = clear.max(3.0);
                                }
                            }
                            self.garage_return = Some((ppos, (-pfwd.x).atan2(-pfwd.z)));
                            // (the garage's camera starts on its view of the car where it now stands, not with a glide
                            // from where the car was outside - the original places the car before the garage shows)
                            self.garage_tr = None;
                            self.garage_cam = None;
                            self.garage_clear = clear;
                            if let Some(c) = self.vehicles.player_mut() {
                                // (the car goes in with the damage it has: the garage asks about it)
                                let old = std::mem::replace(&mut c.state, physics::car::CarState::new(p + Vec3::Y * 0.6, 0.6));
                                c.state.keep_damage(&old);
                                c.pose_history.reset();
                                c.state.rot = rot;
                                // already on the floor: the usual drop from above would land on the roof
                                c.placed = true;
                            }
                            // nothing else belongs in here
                            self.vehicles.cars.retain(|c| c.is_player || c.state.pos.distance(p) > 40.0);
                        }
                    }
                }
            } else if !matches!(self.game.mode, Mode::Garage) {
                if let Some((p, yaw)) = self.garage_return.take() {
                    // the game's scene of driving out of this garage, if we are in one of them
                    let here = self.vehicles.player().map(|c| c.state.pos).unwrap_or(p);
                    let out = ["cut_la_hw_oh_garage_01x_ext", "cut_la_be_smb_garage_01x_ext", "sc_cut_garage_01_ext"]
                        .iter()
                        .filter_map(|s| self.cine.car_pose(s, &cine::is_player_car, false).map(|q| (*s, q.0.distance(here))))
                        .filter(|q| q.1 < 90.0)
                        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                    let leaving = self.game.mode == Mode::FreeRoam && self.game.race.is_none() && self.cine.playing.is_none() && self.cuts.playing.is_none() && std::env::var("MCLA_NO_CINE").is_err();
                    let played = match out {
                        Some((sc, _)) if leaving => self.cine.start(&mut gpu, sc, glam::Mat4::IDENTITY),
                        _ => false,
                    };
                    if played {
                        self.cine_free = true;
                        self.cine_caption = Some(String::new());
                    } else if let Some(c) = self.vehicles.player_mut() {
                        c.state = physics::car::CarState::new(p + Vec3::Y * 0.3, yaw);
                        c.pose_history.reset();
                        c.placed = false;
                    }
                    self.garage_clear = 0.0;
                }
            }
            // ---- career news: new missions, completions, prize cars ----
            // (a new career's first call waits for VEHICLE SELECT to be over)
            // A new career's opening is the original's `game/Career/CarSelection` (0xebb682f7), then
            // CareerSetupPlayer and RACE_GH_06_READY_TO_START_ON_ENTER: the screen fades to black (1 s), the
            // film (intro720.bik: the arrival by bus is in it), black until the city has streamed in, the fade
            // up (1 s) onto VEHICLE SELECT; ACCEPT fades to black (1 s) and the black is held until the first
            // story scene CUT_STORY_RACE_GH_06 comes up (its core's own 1 s fade up, 0x95ca1d1b 0x1012). No
            // script plays CUT_INTRO (only the recomp's cutscene gallery lists it); the game camera is never seen
            // in between. `intro_stage`, `intro_mask`.
            // (CarSelection runs once the start-up checks are over - mc.loadSuccess after the save boxes,
            // `Game::save_boxes_step` - and, after START, under the camera held over the city: `StartWarp::stay`)
            let boxes_done = self.game.message_box.is_none() && (self.game.save_box == 2 || self.game.flash_popup.is_none() || std::env::var("MCLA_NO_SAVE_BOX").is_ok());
            let warp_held = self.startwarp.as_ref().map_or(true, |w| w.phase() == startwarp::Phase::Hold);
            if self.game.fresh_game && self.game.mode == Mode::FreeRoam && !self.intro_played && boxes_done && warp_held {
                self.intro_played = true;
                self.intro_stage = 1;
                self.intro_t = 0.0;
                // (CarSelection 0x456: fade down, waited for, 1 s)
                self.intro_mask.start(1.0, 0.01, f32::MAX);
            }
            self.game.hold_starter = self.game.fresh_game && (!self.intro_played || self.intro_stage < 3);
            // (also on the start screen: a new career's first call must not go out - and bring its story scene - before
            // VEHICLE SELECT is over)
            let held = self.game.fresh_game;
            for n in if held { Vec::new() } else { std::mem::take(&mut self.game.career.news) } {
                match n {
                    career::News::Started { id, title, who, .. } => {
                        // (the first call of a new career has gone out: the next one brings the
                        // MISSION LOG box)
                        if let Some(v) = self.game.career.stats.get_mut("tut:sidekick") {
                            *v = 2;
                        }
                        self.news.push_back((if who.is_empty() { format!("NEW MISSION   {}", title) } else { format!("{}   {}", who, title) }, Some(format!("SIDEKICK_{}", id))));
                    }
                    career::News::Completed { id, title } => {
                        if self.game.mission_target.as_deref() == Some(id.as_str()) {
                            self.game.mission_target = None;
                        }
                        // (after an event the game says so on its MISSION RESULTS panel)
                        if self.game.mode == Mode::Results && self.game.flash_menu.is_some() {
                            let objectives: Vec<String> = self.game.career.objective_texts(&id);
                            log::info!("mission results: {} {:?} (completed views {})", title, objectives, self.game.career.completed_views().len());
                            self.game.mission_results.push((title, objectives, true));
                            self.game.career_over = true;
                        } else {
                            self.news.push_back((format!("MISSION COMPLETE   {}", title), None));
                        }
                    }
                    career::News::Tip { key, title, text } => {
                        log::info!("career: tip {} ({} characters)", key, text.len());
                        self.tips.push_back((key, title, text));
                    }
                    // (step 21b: a story scene played by a CORE at a completion / meeting, or by
                    // an event script - no call, no line)
                    career::News::Scene(id) => {
                        // (step 22b: the CORE's own test - Graphics_GetTimeOfDay(), the day scene when
                        // 6 < hour < 18: CITY_CHAMP_CORE 0x7c0 / 0x9c3, FIND_SH_01B_CORE 0x5c7)
                        let night = !(self.time_of_day > 6.0 && self.time_of_day < 18.0);
                        let scene = self.cuts.for_mission(&id, night);
                        log::info!("career: story scene of {} ({:?})", id, scene.map(|i| self.cuts.scenes[i].name.clone()));
                        if let Some(i) = scene.filter(|_| std::env::var("MCLA_NO_CUTS").is_err() && (self.opt.shot.is_none() || std::env::var("MCLA_CUTS").is_ok())) {
                            self.cuts.queue(i);
                        }
                    }
                    career::News::Money(m) => {
                        // event prizes: the money of a race now (tournaments half as much again)
                        let class = rewards::perf_class(self.game.player_perf());
                        let m = ((rewards::race_money(self.game.rep, class, self.game.race_diff, 10.0) * if m >= 8000 { 1.5 } else { 1.0 }) as i64 / 10) * 10;
                        self.game.money += m;
                        self.news.push_back((format!("PRIZE MONEY   ${}", m), None));
                    }
                    career::News::Car(name) => {
                        // DUB edition prizes are stored with an upper-case suffix
                        let name = self.game.car_dirs.iter().filter_map(|d| d.file_name().map(|f| f.to_string_lossy().to_string())).find(|f| f.eq_ignore_ascii_case(&name)).unwrap_or(name);
                        let known = self.game.car_dirs.iter().any(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(&name)));
                        if known && !self.game.owned.iter().any(|o| o.eq_ignore_ascii_case(&name)) {
                            // (the slot's record, from the set-up file the reward names: `mc4`)
                            let config = self.game.career.car_configs.get(&name.to_ascii_lowercase()).cloned();
                            self.game.mc4_car_added(&name, config.as_deref());
                            self.game.owned.push(name.clone());
                            self.news.push_back((format!("PRIZE CAR   {}", game::pretty_car_name(&name).to_uppercase()), None));
                        }
                    }
                }
            }
            // (test hook: MCLA_SAVE_AT=n writes the save - the MCLA_SAVE_FILE copy - at frame n,
            // even in a screenshot run: step 20b)
            if let Some(n) = std::env::var("MCLA_SAVE_AT").ok().and_then(|v| v.parse::<u64>().ok()) {
                static SAVED_AT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                // (step 22b: by the frame, as --frames counts - a scene stops the ticks)
                if self.frame_no as u64 >= n && std::env::var("MCLA_SAVE_FILE").is_ok() && !SAVED_AT.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    let keep = self.game.no_save;
                    self.game.no_save = false;
                    self.game.write_save(&self.opt.data_dir);
                    self.game.no_save = keep;
                    log::info!("save: written at frame {} (MCLA_SAVE_AT)", n);
                }
            }
            if self.tick_no == 40 {
                if let Ok(k) = std::env::var("MCLA_TIP_TEST") {
                    let text = self.game.career.strings.get(&k).map(|s| s.to_string()).unwrap_or_default();
                    let title = self.game.career.strings.get("SIDEKICK_TITLE_TIP").map(|s| s.to_string()).unwrap_or_default();
                    self.tips.push_back((k, title, text));
                    self.news.clear();
                    self.news_timer = -5.0;
                }
            }
            // (the calls and tips are the career's mission scripts', which run once the career is loaded: none during the
            // start warp or before its loadSuccess, 1 s after the landing - LOADING still up)
            let career_up = self.startwarp.is_none() && self.game.loading_tag <= 0.0;
            // (a police tactic started: StartTactic F-33550 -> the phone's interrupt F-51256 hides the phone and marks
            // its call done - dropped, not queued again)
            if std::mem::take(&mut self.police.phone_cut) && self.game.tip.is_some() {
                log::info!("phone: call put away, a police tactic started");
                // (TelephoneChallenge.sco ends while the police level is over 1 (0x359, 0x437): no decline line)
                if self.police.activity().0 > 1 {
                    self.game.phone_challenge = false;
                }
                self.game.tip = None;
                self.game.phone_prompt.clear();
            }
            // (Cruising.sc.xml, Cruising_Offline: on enter sidekickClean, sidekickDelay and sidekickResume in 8 s; on exit the
            // resume removed, sidekickDelay, sidekickInterrupt (the call up dropped, F-51256) and sidekickClean. The pause
            // menu and the scenes are no state of their own there; a beat me there run is Racing_BeatMeThere. The story
            // scenes are the mission scripts': only RACE_GH_06_READY_TO_START_ON_ENTER throws state events round its scene -
            // mcGame_ThrowEvent (F-96962) 'intro' at 0x9de, 'cruising' at 0xb09 -, so CUT_STORY_RACE_GH_06 alone leaves
            // Cruising_Offline (`pending_log` spans it); Cruising_Offline swallows a repeated mc.cruising, the police's after
            // an arrest (F-22523). Every mc event reaches the chart by name (F-22523, mcGame_ThrowEvent from 96 calls in the
            // race, hangout, garage, hookman and story scripts, the UI charts' throwEvent) except mc.cutsceneDone: F-17084
            // throws it by index at every scene's end (the cine manager's finish callback, F-65984 -> +232, set by F-91527);
            // only an online dialog listens. Nothing throws mc.cutscene)
            let cruising = self.game.cruising_offline() && self.pending_log.is_none() && !self.out_of_cruise_warp();
            if cruising != self.phone_cruise {
                self.phone_cruise = cruising;
                self.phone_hold = true;
                self.phone_resume = cruising.then_some(8.0);
                if !cruising && self.game.tip.is_some() {
                    self.game.tip = None;
                    self.game.phone_prompt.clear();
                }
            }
            if let Some(t) = self.phone_resume.as_mut() {
                *t -= dt;
                if *t <= 0.0 {
                    self.phone_resume = None;
                    self.phone_hold = false;
                }
            }
            if self.phone_was_up && self.game.tip.is_none() {
                self.phone_done = true;
            }
            let gates = self.phone_held();
            // (F-37697's tail, once its gates let it run: the call cleaned away, +788 = 1 and, in Cruising_Offline,
            // sidekickResume after SidekickDelayTime)
            if self.phone_done && !gates && !self.phone_hold {
                self.phone_done = false;
                self.phone_hold = true;
                if cruising {
                    let data_dir = self.opt.data_dir.clone();
                    self.phone_resume = Some(*self.phone_delay.get_or_insert_with(|| menu3d::UiLogic::load(&data_dir).sidekick_delay));
                }
            }
            let phone_held = gates || self.phone_hold || self.phone_done;
            // (PaybackLegCore's calls: the script lifts the phone's hold itself - Sidekick_Delay(0) (F-53206: the phone's +788 =
            // its argument), or UIManager_ReplaceDelayedEvent(sidekickResume, 0.1) (F-93433) - and the call waits for the phone's other gates; once the
            // opening call is announced the script holds the phone again with Sidekick_Delay(1))
            match self.game.opt_x.arcade.d.phone_lift.take() {
                Some(0) => {
                    self.phone_hold = false;
                    self.phone_resume = None;
                    self.phone_redelay = true;
                }
                Some(_) => self.phone_resume = Some(0.1),
                None => {}
            }
            if self.game.tip.is_none() && !gates && !self.phone_hold && !self.phone_done {
                if let Some((who, text, secs, scenario)) = self.game.opt_x.arcade.d.phone_wait.take() {
                    if !text.is_empty() {
                        self.game.tip = Some((who, text, secs));
                        self.game.phone_prompt.clear();
                    }
                    self.game.opt_x.arcade.d.say.push(scenario);
                    self.phone_hold |= std::mem::take(&mut self.phone_redelay);
                }
            }
            // the mechanic rings when nothing else is going on
            if career_up && !phone_held && self.news.is_empty() && self.game.tip.is_none() && self.news_timer <= -4.0 && matches!(self.game.mode, Mode::FreeRoam) {
                if let Some((key, title, text)) = self.tips.pop_front() {
                    let secs = (6.0 + text.len() as f32 * 0.055).min(22.0);
                    self.game.tip = Some((title, text, secs));
                    self.game.phone_prompt.clear();
                    // (the call is the phone's: UI_CELLPHONE_RING when it comes up - sound.rs)
                    if let Some(a) = self.audio.as_mut().filter(|a| !a.has_objects()) {
                        a.play("ui/menus/tutorial_window_fin", 0.7);
                    }
                    self.speech.say(&key);
                    self.news_timer = 0.0;
                }
            }
            self.news_timer = (self.news_timer - dt).max(-30.0);
            // (a splash line is no call: only the calls wait for the phone)
            if career_up && self.news_timer <= 0.0 && matches!(self.game.mode, Mode::FreeRoam) && self.news.front().map_or(false, |n| n.1.is_none() || !phone_held) {
                if let Some((m, call)) = self.news.pop_front() {
                    log::info!("news: {:?} call {:?}", m, call);
                    // (a mission's call is on the phone alone - no line across the top:
                    // `ng_tut_missionlog`, `ng_c4`)
                    if call.is_none() || self.game.flash_phone.is_none() {
                        self.game.show_message(&m);
                    }
                    self.news_timer = 3.4;
                    // (a splash line has no sound of its own; a call rings the phone - sound.rs)
                    if let Some(a) = self.audio.as_mut().filter(|a| !a.has_objects()) {
                        a.play(if m.starts_with("MISSION COMPLETE") || m.starts_with("PRIZE") { "ui/menus/rank_up" } else { "ui/menus/tutorial_window_fin" }, 0.7);
                    }
                    // the briefing is a story scene where the game has one, else a phone call
                    if let Some(c) = call {
                        // (the call made: the READY_TO_START_ON_ENTER script moves the mission on
                        // to CHECKING_OBJECTIVES - step 20b)
                        let night = self.time_of_day > 18.5 || self.time_of_day < 5.5;
                        let scene = if career::Career::scene_elsewhere(c.trim_start_matches("SIDEKICK_")) { None } else { self.cuts.for_mission(c.trim_start_matches("SIDEKICK_"), night) };
                        // (RACE_GH_06's script plays CUT_STORY_RACE_GH_06 and changes the mission to
                        // CHECKING_OBJECTIVES only after it - 0x9cc..0xadb -, so its own script and the
                        // slot request of his man come after the scene; the other ON_ENTER scripts
                        // play no scene of their own)
                        let after_scene = c == "SIDEKICK_RACE_GH_06" && scene.is_some() && std::env::var("MCLA_NO_CUTS").is_err() && (self.opt.shot.is_none() || std::env::var("MCLA_CUTS").is_ok());
                        if after_scene {
                            self.pending_log = Some(c.trim_start_matches("SIDEKICK_").to_string());
                        } else {
                            self.game.career.call_delivered(c.trim_start_matches("SIDEKICK_"));
                        }
                        // (after its story scene a mission is on the phone as its reminder -
                        // "REMEMBER TO GO RACE HENRY ON THE SUNSET STRIP.", `ng_c4` - once the scene
                        // is over)
                        if scene.is_some() {
                            let text = self.game.career.strings.get(&c.replace("SIDEKICK_", "REMINDER_")).or_else(|| self.game.career.strings.get(&c)).map(|s| s.to_string());
                            if let (Some(text), true) = (text, self.game.flash_phone.is_some()) {
                                let who = m.split("   ").next().unwrap_or("").to_string();
                                let who = if who.starts_with("NEW MISSION") { self.game.career.strings.get("SIDEKICK_TITLE_TIP").map(|s| s.to_string()).unwrap_or_default() } else { who };
                                self.pending_call = Some((who, text));
                            }
                        }
                        match scene.filter(|_| std::env::var("MCLA_NO_CUTS").is_err() && (self.opt.shot.is_none() || std::env::var("MCLA_CUTS").is_ok())) {
                            // (the film plays only in CarSelection, before VEHICLE SELECT; RACE_GH_06's
                            // READY_TO_START_ON_ENTER plays its scene alone)
                            Some(i) => self.cuts.queue(i),
                            None if self.pending_call.is_some() => {}
                            None => {
                                if !self.speech.say(&c) {
                                    self.speech.say(&c.replace("SIDEKICK_", "REMINDER_"));
                                }
                                // the call on the phone: the caller, the game's `SIDEKICK_<mission>`
                                // text and the way to the mission log
                                let text = self.game.career.strings.get(&c).or_else(|| self.game.career.strings.get(&c.replace("SIDEKICK_", "REMINDER_"))).map(|s| s.to_string());
                                if let (Some(text), true) = (text, self.game.flash_phone.is_some()) {
                                    let who = m.split("   ").next().unwrap_or("").to_string();
                                    let who = if who.starts_with("NEW MISSION") { self.game.career.strings.get("SIDEKICK_TITLE_TIP").map(|s| s.to_string()).unwrap_or_default() } else { who };
                                    let secs = (6.0 + text.len() as f32 * 0.055).min(22.0);
                                    self.game.tip = Some((who, text, secs));
                                    self.game.phone_prompt = self.mission_log_prompt();
                                    // (a new career's second call explains the mission log, as the call
                                    // comes up - `ng_tut_missionlog`. It had been tied to the mission's
                                    // start, which falls into the results screens: the box never came.)
                                    let st = &mut self.game.career.stats;
                                    // (RACE_GH_02_READY_TO_START_ON_ENTER: after the call, form 76 in slow
                                    // motion unless GetNoTutorialCommandLine - 0x3d5..0x586; step 21b)
                                    if c == "SIDEKICK_RACE_GH_02" && !st.contains_key("tut:mission_log") && self.game.tutorial_box.is_none() {
                                        st.insert("tut:mission_log".into(), 1);
                                        self.game.tutorial_box = Some(("tutorial_mission_log_title".into(), "tutorial_mission_log_desc".into(), 0.3, secs));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // A racer rings: "LET'S RACE.  YOU PICK THE LOCATION." with ACCEPT on Back
            // (`ref/call_challenge.png`; the script `game/race/TelephoneChallenge`: one of GH_03,
            // GH_07, GH_13, GH_16, GH_22 after 2000..2500 of its 100 ms rounds of cruising).
            // approx.: asked of a career that has its garage; the call stands twelve seconds.
            if matches!(self.game.mode, Mode::FreeRoam) && self.game.race.is_none() && self.roam.challenge.is_none() && self.game.flash_phone.is_some() && self.game.career.loaded() && self.game.career.reached("GARAGE_HOLLYWOOD") && !self.game.fresh_game {
                if self.phone_challenge_in <= 0.0 {
                    self.phone_challenge_in = std::env::var("MCLA_PHONE_CHALLENGE").ok().and_then(|v| v.parse().ok()).unwrap_or(200.0 + (self.tick_no % 500) as f32 * 0.1);
                }
                if self.game.tip.is_none() && self.news.is_empty() && self.tips.is_empty() && self.cuts.idle() && !self.game.show_map {
                    // (the script's rounds count only while (0x131..0x162) the game is not paused (Game_IsPaused F-49028:
                    // the pause manager's flag, F-50459), the police level is under 2, the idle camera is off
                    // (Player_IsInIdleCam F-80662: camera mode 13), no map target is selected (mcUINavMap_IsTargetSelected
                    // F-15670: the nav map's +628, set only in its GPS selection - under the map's pause) and the camera
                    // is not held (UILogic_IsCameraPaused F-23025); the call it appends then waits for the phone's gates)
                    let paused = self.game.mode == Mode::Paused || self.game.show_map || self.game.album.stage != crate::photoalbum::Stage::Off;
                    if !self.phone_challenge_due && !paused && self.police.activity().0 < 2 && !self.game.idle_cam && !self.camera_held() {
                        self.phone_challenge_in -= sim_dt;
                        self.phone_challenge_due = self.phone_challenge_in <= 0.0;
                        self.phone_challenge_in = self.phone_challenge_in.max(f32::MIN_POSITIVE);
                    }
                    if self.phone_challenge_due && !phone_held {
                        self.phone_challenge_due = false;
                        self.phone_challenge_in = 0.0;
                        const WHO: [&str; 5] = ["GH_03", "GH_07", "GH_13", "GH_16", "GH_22"];
                        let caller = WHO[(self.tick_no / 7) as usize % 5];
                        let who = self.game.career.name_of(caller);
                        self.phone_caller = caller;
                        let st = |k: &str, d: &str| self.game.career.strings.get(k).map(|s| s.to_string()).unwrap_or_else(|| d.to_string());
                        self.game.tip = Some((who, st("Lets Race", "LET'S RACE.  YOU PICK THE LOCATION."), 12.0));
                        self.game.phone_prompt = st("Back To Accept", "ACCEPT").to_uppercase();
                        self.game.phone_challenge = true;
                        self.speech.say_as("HOOKMAN_PHONE_INVITE_PLAYER", caller);
                    }
                }
            }
            if self.game.tip.is_none() && std::mem::take(&mut self.game.phone_challenge) {
                // (not taken)
                self.speech.say_as("HOOKMAN_PHONE_PLAYER_DECLINES", self.phone_caller);
            }
            self.phone_was_up = self.game.tip.is_some();
            if std::mem::take(&mut self.game.phone_accept) {
                self.pending_action = MenuAction::OpenRaceSelect;
            }
            // (the mission whose scene is over enters the log)
            if self.pending_log.is_some() && self.cuts.idle() && self.game.movie.is_none() {
                if let Some(id) = self.pending_log.take() {
                    log::info!("career: {} enters the log after its scene", id);
                    self.game.career.call_delivered(&id);
                }
            }
            // the reminder kept for after a story scene
            if self.pending_call.is_some() && self.cuts.idle() && self.game.movie.is_none() && self.game.tip.is_none() && matches!(self.game.mode, Mode::FreeRoam) && !self.game.fresh_game && !phone_held {
                if let Some((who, text)) = self.pending_call.take() {
                    let secs = (6.0 + text.len() as f32 * 0.055).min(22.0);
                    self.game.tip = Some((who, text, secs));
                    self.game.phone_prompt = self.mission_log_prompt();
                }
            }
            // (HEADLIGHTS held - pad or key - is the player car's flash lamp target: the high beams, F-44589)
            let held = (self.pad_flash || self.game.opt_x.ctrl.kbd.binds.down(crate::keybinds::HEADLIGHTS, |k| self.input.down(k))) && self.drive && !self.game.in_menu();
            if held != self.vehicles.player_flash {
                let lv = self.vehicles.player().map(|c| (c.amb_lamps.s[3], c.amb_lamps.lens));
                log::info!("headlights: high beams {} (flash state, lamp levels {:?})", if held { "on" } else { "off" }, lv);
            }
            self.vehicles.player_flash = held;
            // (HEADLIGHTS pressed - its edge, F-52044; `flash` - in a car whose vehicle-table class is the police's (5,
            // F-64470) toggles the input flag 0x400 (F-56064), which F-44589 copies into the car's siren byte +304,
            // starting or stopping the siren (F-39258 / F-59693): the player's cruiser's light bar. MCLA_PLAYER_SIREN_OLD=1: none)
            if self.flash && self.drive && std::env::var("MCLA_PLAYER_SIREN_OLD").is_err() {
                let name = self.vehicles.player().map(|c| c.model.name.clone()).unwrap_or_default();
                if self.game.oppick.car(&name).is_some_and(|p| p.info.class == oppick::CLASS_COP) {
                    if let Some(c) = self.vehicles.player_mut() {
                        c.siren = !c.siren;
                        log::info!("headlights pressed in a police car: light bar {}", if c.siren { "on" } else { "off" });
                    }
                }
            }
            // ---- roaming racers ----
            crate::ticktimes::mark(crate::ticktimes::S::Roamers);
            {
                let mut flash = std::mem::take(&mut self.flash) || (std::env::var("MCLA_ROAM_TEST").map_or(false, |v| v != "once" || self.game.career.completed() == 0) && self.tick_no % 30 < 2 && self.roam.challenge.is_none());
                // (a hookman who only races a certain class of car says so)
                // (step 19d: nobody can be flashed while the police are doing something with the
                // player - CruisingHookmanCore 0x745: PoliceManager_CheckPoliceActivity(1) < 2)
                // (each flash the gates below drop is logged with the nearest racer: GitHub #22)
                let pressed = flash && self.drive;
                let refused = |why: &str, roam: &roamers::Roamers, v: &vehicles::Vehicles, w: &world::World| log::info!("headlights flashed, no challenge ({why}): {}", roam.flash_report(v, w));
                if flash && self.roam.challenge.is_none() && !matches!(self.police.heat, police::Heat::None) {
                    if pressed { refused(&format!("police activity {:?}", self.police.heat), &self.roam, &self.vehicles, &world); }
                    flash = false;
                }
                if flash && self.roam.near_is_rival && self.roam.challenge.is_none() && std::env::var("MCLA_ROAM_TEST").is_err() {
                    if let Some(text) = self.game.class_restriction() {
                        if pressed { refused(&format!("class restriction: {text}"), &self.roam, &self.vehicles, &world); }
                        self.game.show_message(&text);
                        flash = false;
                    }
                }
                if flash && pressed && !(self.game.mode == Mode::FreeRoam && self.game.race.is_none() && self.vehicles.traffic_target > 0 && !self.vehicles.frozen) {
                    refused(&format!("not cruising: mode {:?}, race {}, traffic {}, frozen {}", self.game.mode, self.game.race.is_some(), self.vehicles.traffic_target, self.vehicles.frozen), &self.roam, &self.vehicles, &world);
                }
                if self.game.mode == Mode::FreeRoam && self.game.race.is_none() && self.vehicles.traffic_target > 0 && !self.vehicles.frozen {
                    // (no flash while Warper_Warp's climb holds the race about to start - the red light's)
                    let held_up = self.climb.is_some() || self.climb_end.is_some() || self.climb_wait.is_some() || self.pending_action != MenuAction::None || self.game.bmt_end.is_some();
                    if flash && pressed && held_up {
                        refused(&format!("a race about to start or its end: climb {}, pending {:?}, beat me there end {}", self.climb.is_some() || self.climb_end.is_some() || self.climb_wait.is_some(), self.pending_action, self.game.bmt_end.is_some()), &self.roam, &self.vehicles, &world);
                    }
                    let flash = flash && self.climb.is_none() && self.climb_end.is_none() && self.climb_wait.is_none() && self.pending_action == MenuAction::None;
                    let mut warped = false;
                    // (nothing of the cruise runs while Warper_Warp's climb holds a race about to start: the
                    // flashed man's CruisingHookmanCore is over - no tutorial form, no new man on the road)
                    let climbing = self.climb.is_some() || self.climb_end.is_some() || self.climb_wait.is_some();
                    if climbing {
                        self.roam.near = None;
                        self.roam.near_is_rival = false;
                    } else if let Some(race) = (if self.game.bmt_end.is_some() { None } else { self.roam.update(&mut self.game, &mut self.vehicles, &mut gpu, &mut world, flash, sim_dt) }).filter(|&race| {
                        // (a beat me there run's end comes first: its race-over sequence or menu - `Game::bmt_end`)
                        let Some((place, rep, wrecked)) = self.roam.bmt_over.take() else { return true };
                        self.game.bmt_end = Some(game::BmtEnd { race, hookman: self.roam.won_against_rival, place, rep, wrecked, t: 0.0, sel: 0 });
                        log::info!("beat me there: the run is over - place {}, {} REP, wrecked {} (frame {})", place, rep, wrecked, self.tick_no);
                        if !wrecked {
                            self.game.race_slow_wait = Some(game::Game::RACE_END_PANEL);
                            self.game.race_end_t = Some(0.0);
                        }
                        false
                    }).or_else(|| {
                        // (... and then the race: BEAT_ME_THERE_WITH_CUT / SKIP_BEAT_ME_THERE_WITH_CUT)
                        let (race, hookman) = self.game.bmt_go.take()?;
                        self.roam.won_against_rival = hookman;
                        Some(race)
                    }) {
                        warped = self.roam.warp_greeting.is_some();
                        self.game.current_race = Some(race);
                        // only a race against the hookman himself counts for his mission
                        self.game.hookman_race = self.roam.won_against_rival;
                        self.pending_action = MenuAction::StartRace;
                        // (a story core that warps at once - `roamers::core_warp`, RACE_GH_06_CORE 0x4e4: its
                        // greeting, then Warper_Warp('DEFAULT_WITH_CUT'): the climb, the race set up after it)
                        if let Some(g) = self.roam.warp_greeting.take() {
                            self.game.warp_with_cut = true;
                            // (mcCareerSpeech_PlayScenario(scenario, Racer_GetVehicleDriverName(the man), 'PLAYER'))
                            let who = self.game.man_mod.as_ref().map_or(String::new(), |m| m.0.clone());
                            if !g.is_empty() && !self.speech.say_as(&g, &who) {
                                log::info!("speech: no scenario {}", g);
                            }
                        } else if let Some(id) = self.game.career.hookman_mission() {
                            self.speech.say(&format!("TAUNT_PRE_{}", id));
                        }
                    }
                    if self.roam.accepted && !warped {
                        if let Some(id) = self.game.career.hookman_mission() {
                            self.speech.say(&format!("GREETING_{}", id));
                        }
                    }
                    // ---- the red light and freeway racers on their loops (althk.rs) ----
                    if let Some(man) = self.roam.alt_flash.take() {
                        if let Some(st) = self.game.alt_race(&self.roam.alt, &mut self.redlights, &self.vehicles, &man) {
                            if let Some(slot) = st.def.grid.first() {
                                self.roam.alt_raced(man.uid, *slot, man.kind == roamers::ManKind::Freeway);
                            }
                            let id = st.def.id.clone();
                            match self.game.race_defs.iter().position(|d| d.id == id) {
                                Some(i) => self.game.race_defs[i] = st.def,
                                None => self.game.race_defs.push(st.def),
                            }
                            self.game.current_race = self.game.race_defs.iter().position(|d| d.id == id);
                            self.game.hookman_race = st.mission;
                            // Warper_TransitionTo('REDLIGHT') (type 24, `F-32081` A-10573): with the
                            // camera within 100 m of the target, or 300 m in the same district (F-55639),
                            // Warp_RedlightCloseToTarget (Main.sc.xml 612; approx.: the port's dive); else it
                            // goes on as EXIT_TO_CUT - DEFAULT_WITH_CUT's climb from the camera over the
                            // player's car, then the dive onto the start scene (`cap/rl_03..07`)
                            if st.scene.is_some() {
                                let target = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).map_or(Vec3::ZERO, |d| d.grid.first().map_or(d.beacon, |g| g.0));
                                let d = self.cam.pos.distance(target);
                                let close = d < 100.0 || (d < 300.0 && self.game.district_at(self.cam.pos) == self.game.district_at(target));
                                self.game.warp_with_cut = !close;
                                log::info!("red light warp: {:.0} m from the start, {}", d, if close { "close" } else { "climb (EXIT_TO_CUT)" });
                            }
                            self.game.prerace_scene = st.scene;
                            self.pending_action = MenuAction::StartRace;
                            if let Some(t) = st.announce.and_then(|k| self.game.career.strings.get(&k).map(|s| s.to_string())) {
                                self.news.push_back((t.to_uppercase(), None));
                            }
                            if let Some(sp) = st.speech {
                                self.speech.say_as(&sp, &self.game.althk.voice);
                            }
                        }
                    }
                    // ---- red light races ----
                    // (the original's red light racers cruise their loops - althk.rs; the port's first
                    // one, waiting at the nearest light, only with MCLA_OLD_REDLIGHT=1)
                    if althk::old_redlight() && self.roam.challenge.is_none() && self.pending_action == MenuAction::None && self.cine.playing.is_none() {
                        let go = flash || std::env::var("MCLA_REDLIGHT_TEST").is_ok();
                        // (step 19d: the red light racer's vehicle is read before he comes)
                        if let Some(car) = self.redlights.want.take() {
                            if !self.vehicles.models.iter().any(|m| m.name.eq_ignore_ascii_case(&car)) {
                                let dir = world.assets.path("resources/vehicle").join(&car);
                                if dir.is_dir() {
                                    let _ = self.vehicles.load_model(&mut gpu, &mut world, &dir);
                                }
                            }
                        }
                        if let Some((def, scene)) = self.redlights.update(&mut self.game, &mut self.vehicles, &gpu, go, sim_dt) {
                            // (one slot at the end of the list is reused for these)
                            match self.game.race_defs.iter().position(|d| d.id == def.id) {
                                Some(i) => self.game.race_defs[i] = def,
                                None => self.game.race_defs.push(def),
                            }
                            self.game.current_race = self.game.race_defs.iter().position(|d| d.id.ends_with("red_light_race"));
                            self.game.hookman_race = true;
                            self.game.prerace_scene = Some(scene);
                            self.pending_action = MenuAction::StartRace;
                            // "Race to Tower Records"
                            if let Some(t) = self.redlights.announce.take().and_then(|id| self.game.career.strings.get(&id).map(|s| s.to_string())) {
                                self.news.push_back((t.to_uppercase(), None));
                            }
                        }
                    }
                    if let Some(name) = self.roam.near.as_ref() {
                        if self.tick_no % 45 == 0 {
                            // the career may be waiting for exactly this: racing one of the hookmen
                            // (the game's own line, `tutorial_flash_desc`: no keyboard key in it; the
                            // port had "F  FLASH HEADLIGHTS TO RACE <car>")
                            let _ = name;
                            // (the game shows nothing beside a racer - `ng_chase_012`..`016`: only his
                            // flag on the map; the line stays for the port's own HUD)
                            if self.game.flash_hud.is_none() {
                                let line = self.game.career.strings.get("tutorial_flash_desc").map(|s| s.trim_end_matches('.').to_uppercase()).unwrap_or_else(|| "FLASH HEADLIGHTS TO RACE".into());
                                self.game.show_message(&line);
                            }
                            if self.roam.near_is_rival {
                                self.game.career.event(career::Ev::FoundHookman);
                            }
                        }
                    }
                    // where we are driving matters to "go to ..." objectives
                    // The career's cruising racers, three a district (flags in green, yellow and
                    // orange rings on the maps - `rk95_map`): each goes round its loop. approx.:
                    // they are places on the map only until near (the port's roamers are the cars),
                    // at 14 m/s round the loop's junctions in a straight line.
                    {
                        let t = self.vehicles.clock as f32;
                        // (step 19b: the marks are the slot system's men - `Roamers::marks`; the
                        // first version's stand-in is computed and dropped)
                        let s19h_marks = self.roam.marks(&self.vehicles);
                        self.game.cruiser_marks = self.roam.cruisers.iter().enumerate().map(|(n, (d, pts))| {
                            let len: f32 = (0..pts.len()).map(|i| pts[i].distance(pts[(i + 1) % pts.len()])).sum();
                            let mut at = (t * 14.0 + n as f32 * 733.0) % len.max(1.0);
                            let mut out = (pts[0], Vec3::NEG_Z, *d);
                            for i in 0..pts.len() {
                                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                                let l = a.distance(b);
                                if at <= l {
                                    out = (a.lerp(b, at / l.max(1e-3)), (b - a).normalize_or(Vec3::NEG_Z), *d);
                                    break;
                                }
                                at -= l;
                            }
                            out
                        }).collect();
                        self.game.cruiser_marks = s19h_marks;
                        // (the map's hookmen, all of them: the slot men - subtype 3, the flag; a man an
                        // active mission points at carries the mission's pulse and "!" (element +126) -
                        // and the time trial and tournament men, subtype 4, icons 5 / 7)
                        self.game.map_men = self
                            .roam
                            .men
                            .iter()
                            .filter(|m| !m.hidden)
                            .map(|m| {
                                let (p, d) = match self.vehicles.cars.iter().find(|c| c.racer && c.vinyl_name == m.config) {
                                    Some(c) => (c.state.pos, c.state.forward()),
                                    None => {
                                        let (p, d, _) = roamers::loop_at(&m.pts, m.at);
                                        (p, d)
                                    }
                                };
                                // (the flash of the icon: the man the current objective of his mission points at -
                                // `Career::pulse_objective`, `mc4::target_objective`)
                                let mission = !m.mission.is_empty() && self.game.career.pulse_objective(&m.mission) == Some(crate::mc4::target_objective(&m.mission, &m.name));
                                match m.kind {
                                    roamers::ManKind::Slot => (p, d, 3, 4, m.difficulty.clamp(0, 3) as usize, mission),
                                    roamers::ManKind::Tournament => (p, d, 4, 7, m.difficulty.clamp(0, 3) as usize, mission),
                                    roamers::ManKind::TimeTrial => (p, d, 4, 5, m.difficulty.clamp(0, 3) as usize, mission),
                                    // (rule_elements.md 5c: SetVehicleElement subtype 1 + Racer_SetIconType 3 -
                                    // `<d>_redlight_hk1`; subtype 5 + icon 8 - `freeway_hookman<n>`)
                                    roamers::ManKind::RedLight => (p, d, 1, 3, m.difficulty.clamp(0, 3) as usize, mission),
                                    roamers::ManKind::Freeway => (p, d, 5, 8, m.difficulty.clamp(0, 3) as usize, mission),
                                }
                            })
                            .collect();
                    }
                    // (the GPS map's cursor names the mission's man: TREVOR / 1975 280Z - `ng_gps_1`)
                    {
                        let want = self.game.career.rival_id();
                        self.game.rival_mark = self.vehicles.cars.iter().find(|c| c.racer && !c.driver.is_empty() && want.as_ref().map_or(true, |w| c.driver.eq_ignore_ascii_case(w))).map(|c| (c.state.pos, self.game.career.name_of(&c.driver), c.model.name.clone()));
                    }
                    self.district_timer -= sim_dt;
                    // (a target set or cleared on the GPS map takes effect at once, as F-83952 hands the arrow the new
                    // target when it is set: the route is made again now, not at the next 3 s round)
                    let key = (self.game.gps_pick.map(|p| p.to_array().map(f32::to_bits)), self.game.mission_target.clone());
                    let fresh = key != self.gps_key;
                    if fresh {
                        self.gps_key = key;
                    }
                    if fresh || self.district_timer <= 0.0 {
                        self.district_timer = 3.0;
                        let wanted = self.game.career.mission_races();
                        self.game.mission_races = (0..self.game.race_defs.len()).filter(|&i| wanted.iter().any(|(r, _)| self.game.race_defs[i].id.to_ascii_lowercase().ends_with(r.as_str()))).collect();
                        // (the GPS map's events: those open, and those done - to be driven again)
                        let done = self.game.career.done_races();
                        self.game.map_races = (0..self.game.race_defs.len()).filter(|&i| self.game.mission_races.contains(&i) || done.iter().any(|r| self.game.race_defs[i].id.to_ascii_lowercase().ends_with(r.as_str()))).collect();
                        // GPS: to the rival if the mission is about meeting him (the mission's own man, `rival_mark` - not
                        // whichever racer comes first), else to the nearest career event
                        let rival = self.game.rival_mark.as_ref().map(|m| m.0);

                        let event = self.game.mission_races.iter().filter_map(|&i| self.game.race_start(i)).min_by(|a, b| a.distance_squared(focus).partial_cmp(&b.distance_squared(focus)).unwrap());
                        // (the target set on the GPS map comes first: the place, or the mission's man / event)
                        let target = match self.roam.challenge.as_ref() {
                            Some(ch) => Some(ch.goal),
                            None => self.game.gps_target_pos().or(rival).or(event),
                        };
                        self.game.gps = match (target, world.nav.as_ref(), self.vehicles.player()) {
                            (Some(t), Some(nav), Some(c)) if t.distance(c.state.pos) > 60.0 => nav.route_points(c.state.pos, Some(c.state.forward()), t).map(std::sync::Arc::new),
                            _ => None,
                        };
                        let d = self.game.district_at(focus);
                        self.game.career.event(career::Ev::District(&d));
                    }
                    if let Some(ch) = self.roam.challenge.as_ref() {
                        if self.tick_no % 45 == 0 {
                            let d = self.vehicles.player().map_or(0.0, |c| c.state.pos.distance(ch.goal));
                            // (in the game's words: BEAT ME THERE - FLASH HEADLIGHTS TO MEET AT THE STARTING LINE)
                            let st = |k: &str, fb: &str| self.game.career.strings.get(k).map(|s| s.to_uppercase()).unwrap_or_else(|| fb.to_string());
                            let m = format!("{}   {:.1} KM   -   {}", st("raceToStart", "BEAT ME THERE"), d / 1000.0, st("CANCEL_BEAT_ME_THERE", "FLASH HEADLIGHTS TO MEET AT THE STARTING LINE"));
                            self.game.show_message(&m);
                        }
                    }
                } else if self.roam.challenge.is_some() {
                    self.roam.cancel(&mut self.vehicles);
                }
            }
            // (the run's race is current while it lasts - `Game::race_current`)
            self.game.bmt_goal = self.roam.challenge.as_ref().map(|c| c.goal);
            // (Script_SuspendAllOpponents: the hookmen's cores and the place cores - `Roamers::suspend`,
            // `Game::update_place_holds`)
            self.roam.suspend(self.game.suspend_all_opponents(), self.game.race.is_some(), &mut self.vehicles);
            let pp = self.vehicles.player().map(|c| c.state.pos);
            self.game.update_place_holds(pp, sim_dt);
            crate::ticktimes::mark(crate::ticktimes::S::Other);
            if std::env::var("MCLA_SIGNAL_TEST").is_ok() && self.tick_no % 600 == 0 {
                let moving = self.vehicles.cars.iter().filter(|c| !c.is_player && c.parked.is_none() && c.state.speed() > 3.0).count();
                log::info!("signals: car-frames braking for red {}, standing at red {}, traffic moving now {}", self.vehicles.held_at_red, self.vehicles.stopped_at_red, moving);
            }
            // diagnostics: MCLA_VINYL=<n> wears the n-th livery of the car
            if let Some(n) = std::env::var("MCLA_VINYL").ok().and_then(|v| v.parse::<usize>().ok()) {
                if let Some(v) = self.game.vinyl_list.get(n).cloned() {
                    let car = self.game.vinyl_for.clone();
                    self.game.vinyls.insert(car, v);
                }
            }
            // the player's livery follows the garage's choice for this car
            if let Some(pcar) = self.vehicles.player().map(|c| c.model.name.clone()) {
                if self.game.vinyl_for != pcar {
                    // the player's own, the presets of the disc, then the opponents' finished ones
                    let bike = self.vehicles.player().map_or(false, |c| c.model.bike);
                    let mut list: Vec<String> = Vec::new();
                    if self.game.liveries.get(&pcar).map_or(false, |l| !l.is_empty()) {
                        list.push("custom".to_string());
                    }
                    let presets = if bike { &self.game.vinyl_presets.bike_names } else { &self.game.vinyl_presets.names };
                    list.extend(presets.iter().map(|p| format!("preset:{}", p)));
                    list.extend(self.vehicles.vinyl_presets(&world, &pcar));
                    self.game.vinyl_list = list;
                    self.game.vinyl_for = pcar.clone();
                }
                let mut want = self.game.vinyls.get(&pcar).cloned().unwrap_or_default();
                // (step 19c: a borrowed vehicle wears its set-up's layers - `mc4_setup_livery`)
                let s19a_setup = self.game.mc4_setup_livery(&pcar);
                if let Some(k) = s19a_setup.as_ref() {
                    want = k.clone();
                }
                if want.starts_with("preset:") || want == "custom" || s19a_setup.is_some() {
                    // painted here from its layers, as the original's vinyl generator does
                    // (half size while the editor is open: it is painted again at every change)
                    let size: usize = if self.game.vinyl_edit.is_some() { 512 } else { 1024 };
                    let key = if want == "custom" { format!("custom@{}#{}x{}", pcar, self.game.livery_rev, size) } else { format!("{}@{}", want, pcar) };
                    if !self.vehicles.has_vinyl(&key) {
                        self.vehicles.drop_vinyls(&format!("custom@{}#", pcar), &key);
                        let s19a_file = want.strip_prefix("setup:").and_then(|c| render::car_lamps::setup_file(std::path::Path::new(&self.opt.data_dir), c)).and_then(|f| std::fs::read(f).ok());
                        let livery = match (s19a_file, want.strip_prefix("preset:")) {
                            (Some(d), _) => vinyl::Livery::from_xcc(&d),
                            (None, Some(p)) => self.game.vinyl_presets.livery(p),
                            (None, None) => self.game.liveries.get(&pcar).cloned().unwrap_or_default(),
                        };
                        let dir = if s19a_setup.is_some() { Some(std::path::Path::new(&self.opt.data_dir).join("resources/vehicle").join(&pcar)) } else { self.game.car_dirs.get(self.game.current_car).cloned() };
                        let zones = dir.as_deref().and_then(|d| self.car_zones.entry(pcar.clone()).or_insert_with(|| vinyl::CarZones::load(d).map(Arc::new)).clone());
                        let lib = self.shape_lib.get_or_insert_with(|| vinyl::ShapeLib::load(&self.opt.data_dir));
                        match zones {
                            Some(z) => {
                                let t = Instant::now();
                                let px = vinyl::compose(&z, &livery, lib, size);
                                log::info!("vinyls: painted {} ({} layers) in {:.0} ms", key, livery.layer_count(), t.elapsed().as_secs_f32() * 1000.0);
                                self.vehicles.add_vinyl_texture(&mut gpu, &key, size as u32, size as u32, &px);
                            }
                            None => {
                                log::warn!("vinyls: {} has no zone data", pcar);
                                self.vehicles.add_vinyl_texture(&mut gpu, &key, 1, 1, &[0, 0, 0, 0]);
                            }
                        }
                    }
                    want = key;
                }
                if let Some(c) = self.vehicles.player_mut() {
                    c.set_vinyl(&want);
                }
            }
            self.vehicles.assign_plates(&mut gpu, &world);
            self.vehicles.assign_vinyls(&mut gpu, &mut world);
            // (step 19d: a car that came with a set-up's name is put into it - parts, rims,
            // tires, brakes, paints, upgrades: one car a frame)
            {
                let game = &self.game;
                self.vehicles.dress_setups(&mut gpu, &mut world, &mut |car, cfg| game.mc4_dress_of(car, cfg));
            }
            // (step 19d: what the rules of `mc4` want said and played)
            {
                let (say, sounds, tick) = self.game.mc4_say_take();
                for s in say {
                    if !self.speech.say(&s) {
                        log::debug!("speech: no scenario {}", s);
                    }
                }
                if let Some(a) = self.audio.as_mut() {
                    for s in sounds {
                        // (the clock's beep: RACE_TIMER_SECOND_BEEP_MASTER, F-86111)
                        if !(s == "ui/menus/beep_fin" && a.obj("RACE_TIMER_SECOND_BEEP_MASTER")) {
                            a.play(s, 0.8);
                        }
                    }
                    // (step 19d: the race clock's tick loop - `mc4_clock_sounds`: RACE_TIMER_TICK_LOOP_MASTER)
                    if tick && self.game.mode != Mode::Paused && !a.obj_loop("RACE_TIMER_TICK_LOOP_MASTER") {
                        a.set_loop(0x5449_434B_0019_000D, "ui/menus/stopwatch_fin", (0.8, 0.8), 1.0);
                    }
                }
            }
            // ---- pedestrians ----
            if let Some(nav) = world.nav.as_ref() {
                // (and on the start screen, the ambient manager's front-end mode: the camera's view)
                let frontend = self.vehicles.amb_frontend;
                self.peds.set_frontend(frontend.is_some());
                if matches!(self.game.mode, Mode::FreeRoam | Mode::Race) || frontend.is_some() {
                    let threats: Vec<peds::Threat> = self.vehicles.cars.iter().filter(|c| c.parked.is_none() && c.placed).map(|c| peds::Threat { pos: c.state.pos, vel: c.state.vel }).collect();
                    let props = &self.props;
                    let signals = if self.vehicles.signals.junctions.is_empty() { None } else { Some((&self.vehicles.signals, self.vehicles.clock)) };
                    // the original's spawner looks along the player car (speed in mph)
                    match self.vehicles.player().filter(|_| self.drive && frontend.is_none()) {
                        Some(c) => self.peds.set_view(c.state.forward(), c.state.speed() * 2.236_936),
                        None => self.peds.set_view(frontend.map_or(self.cam.forward(), |f| f.dir), 0.0),
                    }
                    self.peds.read_ahead_also(frontend.and(self.vehicles.player().map(|c| c.state.pos)).map(|p| (p, self.titlecam.saved_hour().unwrap_or(self.time_of_day))));
                    self.peds.update(&mut gpu, nav, &world.collider, &|p, r, out| props.obstacle_boxes_near(p, r, out), focus, &threats, signals, self.time_of_day, sim_dt);
                    // (the layout items' people: placed, removed, and what the spawners see next)
                    if frontend.is_none() {
                        layout::Layout::people(&mut self.vehicles, &mut self.peds, &mut gpu, &world);
                    }
                    if self.tick_no % 90 == 0 && std::env::var("MCLA_PED_DUMP").is_ok() {
                        log::info!("peds: {} alive, {} evades, {} crossings, {} waiting, kerbs {:?}", self.peds.count(), self.peds.evades, self.peds.crossings, self.peds.waiting(), self.peds.curb_stats());
                        for (p, f, n) in self.peds.nearest(focus, 8) {
                            log::info!("  ped {} at {:.1},{:.1},{:.1} facing {:.2},{:.2} dist {:.0}", n, p.x, p.y, p.z, f.x, f.z, p.distance(focus));
                        }
                    }
                }
            }
            // ---- particle effects ----
            crate::ticktimes::mark(crate::ticktimes::S::Particles);
            {
                let rain = self.rain_amount();
                let cam = self.cam.pos;
                for c in self.vehicles.cars.iter() {
                    if c.parked.is_some() || !c.placed || c.state.pos.distance_squared(cam) > 150.0 * 150.0 {
                        continue;
                    }
                    let st = &c.state;
                    let speed = st.speed();
                    // (the cars on the original's simulation: their tyres' effects are the original's,
                    // `tyrefx.rs` below; the others keep the port's own puffs)
                    for (wi, w) in st.wheels.iter().enumerate().filter(|_| c.state.veh.is_none()) {
                        if !w.grounded {
                            continue;
                        }
                        // spinning or sliding tyres smoke; the driven axle most of all
                        let over = (w.slip - 1.0).max(0.0);
                        let mut burn = over * if speed > 3.0 || c.input.throttle > 0.6 { 1.0 } else { 0.0 };
                        // hard launches light up the driven tyres; the handbrake locks the rears
                        let pr = c.tuned.as_deref().unwrap_or(&c.model.params);
                        let driven = if wi < 2 { pr.drive_front > 0.4 } else { pr.drive_front < 0.6 };
                        if driven && c.input.throttle > 0.9 && speed < 11.0 && pr.accel_at(speed) > 5.2 {
                            burn = burn.max(0.7 * (1.0 - speed / 11.0));
                        }
                        if wi >= 2 && c.input.handbrake && speed > 6.0 {
                            burn = burn.max(0.6);
                        }
                        if burn > 0.12 && (self.tick_no + wi as u64) % 2 == 0 {
                            self.fx.tyre_smoke(w.contact, st.vel, (burn * 1.6).min(1.0));
                        }
                        // loose ground: dust off sand and gravel, a little off grass, spray in the river
                        if speed > 5.0 && (self.tick_no + wi as u64) % 3 == 0 {
                            use physics::collision::Surface;
                            let amount = (speed / 28.0).min(1.0) * (0.5 + over.min(1.0));
                            match Surface::from_u8(w.surface).kind() {
                                Surface::Sand => self.fx.dust(w.contact, st.vel, amount, [0.86, 0.78, 0.6]),
                                Surface::Gravel | Surface::Railroad => self.fx.dust(w.contact, st.vel, amount * 0.8, [0.62, 0.56, 0.47]),
                                Surface::Dirt => self.fx.dust(w.contact, st.vel, amount * 0.9, [0.58, 0.47, 0.36]),
                                Surface::Water | Surface::WaterDeep => self.fx.spray(w.contact, st.vel, (speed / 20.0).min(1.0)),
                                Surface::Grass if (self.tick_no / 3 + wi as u64) % 2 == 0 => self.fx.dust(w.contact, st.vel, amount * 0.45, [0.5, 0.48, 0.34]),
                                Surface::River => self.fx.spray(w.contact, st.vel, 0.7 * (speed / 25.0).min(1.0)),
                                _ => {}
                            }
                        }
                        if rain > 0.4 && speed > 8.0 && (self.tick_no + wi as u64) % 3 == 0 {
                            self.fx.spray(w.contact, st.vel, rain * (speed / 30.0).min(1.0));
                        }
                    }
                    // (the exhaust and nitrous effects: carfx.rs; the original has no backfire effect)
                    let h = c.model.params.body_half;
                    if c.is_player {
                        if st.last_impact > self.fx_impact + 0.2 {
                            let n = ((st.last_impact * 4.0) as usize).clamp(6, 30);
                            self.fx.sparks(st.pos + st.forward() * h.z * 0.8 + Vec3::Y * 0.4, -st.vel.normalize_or(Vec3::Y) * 0.4 + Vec3::Y * 0.5, n);
                        }
                        self.fx_impact = st.last_impact;
                    }
                }
                // (a window's hit sum passing 1 or 2 (F-66214): `damage_glassbreak` at the window's place,
                // moving with the car's body - effect +2028 of the car's effects object, started only while
                // the car is drawn (G+116 != 0, F-44722: radius r = the larger of the composite bound box's
                // full x / z sizes, the car's origin in the viewport's sphere test and no more than 450 m + r from
                // the camera; here the last frame's view frustum); its sound is caraudio's)
                let glass_fx = mcla_formats::hash::joaat("damage_glassbreak");
                let cam_at = self.cam.pos;
                let view = self.last_frustum;
                for c in self.vehicles.cars.iter_mut() {
                    let d = &mut c.state.dmg;
                    let seen = d.glass_fx_seen;
                    if !d.glass.iter().any(|(n, _)| *n > seen) {
                        continue;
                    }
                    let (lo, hi) = c.model.params.composite_box();
                    let r = (hi.x - lo.x).max(hi.z - lo.z);
                    let o = c.state.pos;
                    let drawn = o.distance(cam_at) - r <= 450.0 && view.map_or(true, |f| f.intersects_sphere(o, r));
                    for &(n, local) in d.glass.iter().filter(|(n, _)| *n > seen) {
                        let at = c.state.pos + c.state.rot * local;
                        let played = drawn && self.fx.burst(glass_fx, at, c.state.vel, at.distance(cam_at));
                        if std::env::var_os("MCLA_PARTS_LOG").is_some() {
                            log::info!("glass: event {n} at {at:.2?} damage_glassbreak played {played}");
                        }
                        d.glass_fx_seen = n;
                    }
                }
                // (a scrape's sparks, damagemap::Sparks - `F-85632`: the effect damage_scrape_sparks at
                // the point, its z along the contact normal, x = normalize(up x z), y = z x x
                // (`F-97951`, up = the identity's row 1), moving with the closing velocity,
                // evolution "impact" = min(10 x amount, 1); the port's own sparks only when the effect
                // cannot be played)
                let scrape_fx = mcla_formats::hash::joaat("damage_scrape_sparks");
                for c in self.vehicles.cars.iter_mut() {
                    for k in std::mem::take(&mut c.state.dmg.sparks) {
                        let z = k.normal.normalize_or(Vec3::Y);
                        let x = Vec3::Y.cross(z).normalize_or(Vec3::X);
                        let rot = glam::Quat::from_mat3(&glam::Mat3::from_cols(x, z.cross(x), z));
                        if !self.fx.burst_evo(scrape_fx, k.at, rot, k.rel, k.at.distance(self.cam.pos), &[("impact", (k.amount * 10.0).min(1.0))]) {
                            let n = ((k.amount * 10.0).min(1.0) * 12.0) as usize + 2;
                            self.fx.sparks(k.at, (k.normal * 0.5 - k.rel.normalize_or_zero() * 0.3).normalize_or(Vec3::Y), n);
                        }
                    }
                }
                // (the props' events: the impact reports and deflection passes, the breaks - physics/loose.rs, props.rs;
                // the hydrant's water gets the car's underside and its "collision" evolution before the particle step -
                // propfx.rs. The original has no generic spark on a prop hit: the sparks of a pole are col_metal_pole's.)
                {
                    let mut plays = std::mem::take(&mut self.props.fx);
                    plays.extend(std::mem::take(&mut self.vehicles.level.fx_events));
                    if let Ok(t) = std::env::var("MCLA_PROPFX_TEST") {
                        // test helper: "effect@x,y,z[;...]": the effect at that point (a fixed matrix) from tick 20, every 15 ticks (the hydrant's once)
                        if self.tick_no >= 20 && self.tick_no % 15 == 0 {
                            for item in t.split(';') {
                                let Some((name, at)) = item.split_once('@') else { continue };
                                let v: Vec<f32> = at.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                                if v.len() == 3 {
                                    if name == "water_hydrant" {
                                        if !self.propfx.has_fixed() {
                                            self.propfx.start_fixed(name, Vec3::new(v[0], v[1], v[2]));
                                        }
                                    } else if name != "water_hydrant" {
                                        plays.push(crate::physics::loose::FxPlay { effect: name.to_string(), at: crate::physics::loose::FxAt::World(Vec3::new(v[0], v[1], v[2])), vel: Vec3::ZERO });
                                    }
                                }
                            }
                        }
                    }
                    let lw = world.collider.loose.as_deref();
                    self.propfx.start(plays, &self.vehicles.level, lw, &mut self.fx, self.cam.pos);
                    let mut car = self.vehicles.player().and_then(|c| c.state.underside.map(|q| (c.state.pos, q)));
                    if let Ok(t) = std::env::var("MCLA_PROPFX_QUAD") {
                        // test helper "x,y,z": a car's underside there (2 x 4 m, level)
                        let v: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        if v.len() == 3 {
                            let c = Vec3::new(v[0], v[1], v[2]);
                            car = Some((c, [c + Vec3::new(1.0, 0.0, -2.0), c + Vec3::new(1.0, 0.0, 2.0), c + Vec3::new(-1.0, 0.0, 2.0), c + Vec3::new(-1.0, 0.0, -2.0)]));
                        }
                    }
                    self.propfx.step(&self.vehicles.level, lw, &mut self.fx, self.cam.pos, car);
                }
                let scrape_every = if std::env::var("MCLA_SCRAPE_TEST").map_or(false, |v| v.contains(',')) { 2 } else { 20 };
                if std::env::var("MCLA_SCRAPE_TEST").is_ok() && self.tick_no > 90 && self.tick_no % scrape_every == 0 {
                    // test helper: a scrape on the player's right side every 20 ticks (damagemap::Sparks,
                    // played as above), the closing velocity the car's own; "x,y,z" instead puts it at that
                    // point, normal +x, moving 8 m/s along -z (a still camera can watch it)
                    let at: Vec<f32> = std::env::var("MCLA_SCRAPE_TEST").unwrap_or_default().split(',').filter_map(|v| v.trim().parse().ok()).collect();
                    if at.len() == 3 {
                        // (no car needed: the frame for normal +x is x = up x z = -z, y = z x x = up)
                        let p = Vec3::new(at[0], at[1], at[2]);
                        let rot = glam::Quat::from_mat3(&glam::Mat3::from_cols(Vec3::NEG_Z, Vec3::Y, Vec3::X));
                        self.fx.burst_evo(mcla_formats::hash::joaat("damage_scrape_sparks"), p, rot, Vec3::new(0.0, 0.0, -8.0), p.distance(self.cam.pos), &[("impact", 1.0)]);
                    } else if let Some(c) = self.vehicles.cars.iter_mut().find(|c| c.is_player) {
                        let (pos, right, vel) = (c.state.pos, c.state.right(), c.state.vel);
                        c.state.dmg.sparks.push(crate::damagemap::Sparks { at: pos + right * 1.0 + Vec3::Y * 0.3, normal: -right, rel: vel, amount: 0.1 });
                    }
                }
                if std::env::var("MCLA_FX_TEST").is_ok() && self.tick_no > 90 {
                    // test helper: every effect at once on the player's car
                    if let Some(c) = self.vehicles.player() {
                        let st = &c.state;
                        for w in &st.wheels[2..] {
                            self.fx.tyre_smoke(w.contact, st.vel, 1.0);
                        }
                        if self.tick_no % 20 == 0 {
                            self.fx.sparks(st.pos + st.right() * 1.5 + Vec3::Y * 0.4, Vec3::Y, 20);
                        }
                    }
                }
                // (the tyres' effects: decided once a game frame - the original's 30 Hz, `F-67671` -
                // and moved with the wheels on the tick between; the road's wetness eases to the weather's)
                if let Some(t) = self.tyre_fx.as_mut() {
                    let mut wet = self.weather.wet();
                    if let Some((_, w, f)) = self.sky_next.as_ref() {
                        wet += (w.wet() - wet) * f;
                    }
                    t.step(&mut self.fx, &self.vehicles.cars, wet, sim_dt * 2.0, self.tick_no % 2 == 0, self.cam.pos);
                }
                carfx::step(&mut self.fx, &self.vehicles.cars, self.cam.pos, self.hide_player);
                self.fx.hour = self.time_of_day;
                self.fx.update(sim_dt);
            }
            crate::ticktimes::mark(crate::ticktimes::S::Police);
            if matches!(self.game.mode, Mode::FreeRoam | Mode::Race) && !self.vehicles.frozen {
                if let Ok(v) = std::env::var("MCLA_COP_TEST") {
                    // ("park": a cruiser waiting at the kerb ahead of the player, no pursuit - to look at it)
                    // ("pull": the same cruiser, free to drive, and the score at 260 - the pull-over starts when it sees the car)
                    let pull = v == "pull" || v == "bolt";
                    if (v == "park" || pull) && self.tick_no >= 150 && !self.vehicles.cars.iter().any(|c| c.cop) && self.tick_no % 30 == 0 {
                        if let Some(c) = self.vehicles.player().map(|c| c.state.pos + c.state.forward() * 14.0) {
                            let ok = self.police.station(&gpu, &world, &mut self.vehicles, c, 0);
                            log::info!("police: test cruiser parked: {}", ok);
                            // (held where it stands with its driver seated: not a parked car)
                            if let Some(c) = self.vehicles.cars.iter_mut().rev().find(|c| c.cop).filter(|_| ok) {
                                c.parked = None;
                                if pull {
                                    self.police.score = 260.0;
                                } else {
                                    c.ai = None;
                                }
                            }
                        }
                    } else if v == "units" && self.tick_no == 600 {
                        self.police.debug_units();
                    } else if v != "park" && v != "units" && !pull && self.tick_no == 150 {
                        self.police.debug_force(&gpu, &world, &mut self.vehicles);
                    }
                    // ("flip": the player's car laid on its roof first - the arrest of a car upside down)
                    if v == "flip" && self.tick_no == 140 {
                        if let Some(c) = self.vehicles.player_mut() {
                            c.state.pos += Vec3::Y * 1.5;
                            c.state.rot = c.state.rot * glam::Quat::from_rotation_z(std::f32::consts::PI);
                        }
                    }
                    if self.tick_no % 60 == 0 {
                        let pp = self.vehicles.player().map(|c| c.state.pos).unwrap_or_default();
                        let cops: Vec<String> = self.vehicles.cars.iter().filter(|c| c.cop).map(|c| format!("{:.0}m@{:.0}", c.state.pos.distance(pp), c.state.speed())).collect();
                        log::info!("police: heat {:?} score {:.0} fine {:.0} search {:.1} cops {:?} player speed {:.1}", self.police.heat, self.police.score, self.police.fine, self.police.search, cops, self.vehicles.player().map_or(0.0, |c| c.state.speed()));
                    }
                }
                let heat_before = self.police.heat;
                let score_before = self.police.score;
                if self.police.enabled {
                    self.police.update_units(&gpu, &world, &mut self.vehicles);
                }
                self.police.progress = rewards::progress(self.game.rep).1;
                self.police.radio_busy = self.radio.busy();
                self.police.in_race = self.game.race.is_some();
                // (the police manager and its tactics run once a game frame, 1/30 s - the chassis ticks twice in
                // between; their timers and per-frame tests (the +144 "still" step) take that dt. The level probe
                // meets the walkers' capsules too, F-20855)
                let ev = if brain::frame_due(&mut self.police_acc, sim_dt) {
                    self.police.peds = self.peds.capsule_feet();
                    self.police.update(&gpu, &world, &mut self.vehicles, self.props.knocked, brain::FRAME)
                } else {
                    None
                };
                // (F-22530: an arrest by the cops ends a race running - raceOverCommand `returnToCruise`,
                // raceOverTrigger 1: back to the cruise as RETURN TO CRUISE does)
                if std::mem::take(&mut self.police.arrest_over) && self.game.race.is_some() {
                    log::info!("police: arrested during the race - back to the cruise");
                    self.pending_action = MenuAction::StartFreeRoam;
                }
                // (step 19d: the payback's chase - the pursuers are the client's friends, their
                // two results the leg's own: no radio, no fine, no arrest form - `mc4`)
                let s19d_goons = self.police.goons;
                let ev = self.game.mc4_payback_chase(&mut self.police, &mut self.vehicles, &mut gpu, &mut world, ev, sim_dt);
                // ---- the radio traffic that goes with it ----
                if self.radio.loaded() && !s19d_goons && !self.police.goons {
                    let rctx = radio_ctx(&self.vehicles, &world, self.time_of_day);
                    match ev {
                        Some(police::Event::PullOver) => {
                            self.radio.say("REPORT_PULL_SUSPECT_OVER", true, &rctx);
                        }
                        // (siren/RULE.md section 7: hitting the cop adds iDamagePoliceVehPoints 600 at once; the
                        // score reaching 600 by itself; driving off during the ticket)
                        Some(police::Event::Pursuit) if heat_before == police::Heat::PullOver => {
                            if self.police.score - score_before >= 599.0 {
                                self.radio.say("REPORT_HIT_COP_CAR_HARD", true, &rctx);
                            } else if self.police.score >= 600.0 {
                                self.radio.say("REPORT_PULL_OVER_PURSUIT", true, &rctx);
                            } else {
                                if !self.speech.busy() {
                                    self.speech.say("PLAYER_COPS_ESCAPE_TICKET");
                                }
                                self.radio.say("REPORT_SUSPECT_FLEES", true, &rctx);
                            }
                            self.radio_next = 9.0;
                        }
                        Some(police::Event::Pursuit) => {
                            self.radio.say("REPORT_VIOLATION_PURSUIT", true, &rctx);
                            self.radio_next = 9.0;
                        }
                        Some(police::Event::Escaped) => {
                            self.radio.say("REPORT_END_PURSUIT", true, &rctx);
                        }
                        // busted and ticket: the radio line is cut, no scenario
                        Some(police::Event::Ticket(_)) | Some(police::Event::Busted(_)) => {
                            self.radio.stop();
                        }
                        Some(police::Event::ArrestScene) | None => {}
                    }
                    // (the tactics' lines: the search's LOST when its clock starts, SPOTTED / REACQUIRE on a sighting after
                    // 2 s (F-40917); the dispatch's DISPATCH call and REPORT_ACCEPT_CALL; a flank's REPORT_MANEUVER_CLOSE)
                    for cue in std::mem::take(&mut self.police.radio_cue) {
                        if cue == "REPORT_SUSPECT_LOST" {
                            self.radio.stop();
                        }
                        self.radio.say(&cue, !cue.starts_with("DISPATCH"), &rctx);
                    }
                    // a cruiser wrecked or destroyed: REPORT_COP_CAR_DISABLED once (per car)
                    for (i, c) in self.vehicles.cars.iter().enumerate() {
                        let down = c.cop && !c.goon && (c.state.wipeout > 0.0 || c.state.damage >= 1.0);
                        if down && self.sound.cop_disabled.insert(i) {
                            self.radio.say("REPORT_COP_CAR_DISABLED", true, &rctx);
                        } else if !c.cop {
                            self.sound.cop_disabled.remove(&i);
                        }
                    }
                    if self.police.heat == police::Heat::Pursuit {
                        self.radio_next -= sim_dt;
                        if self.radio_next <= 0.0 {
                            const CHATTER: [&str; 7] = ["REPORT_LOCATION_UPDATE", "REPORT_PURSUIT_STATEMENT", "REPORT_SUSPECT_SPEED", "REPORT_MANEUVER", "REPORT_OFFICER_TO_OFFICER", "REPORT_REQUEST_BACKUP", "REPORT_JOIN_PURSUIT"];
                            let fast = self.vehicles.player().map_or(false, |c| c.state.speed() > 40.0);
                            let pick = if fast && self.tick_no % 3 == 0 { "REPORT_SUSPECT_SPEED" } else { CHATTER[(self.tick_no as usize / 7) % CHATTER.len()] };
                            self.radio.say(pick, false, &rctx);
                            self.radio_next = 8.0 + (self.tick_no % 5) as f32;
                        }
                    }
                }
                match ev {
                    // (the scenes that may follow - the officer's walk after a pull-over, the arrest after a chase - read
                    // ahead on workers: their packs and officers' sets had stopped the scene's first frame for 16 - 90 ms,
                    // docs/PERFORMANCE.md 2026-10-27 (11). Which one plays and when is unchanged.)
                    Some(police::Event::PullOver) => {
                        for s in ["cut_cop_walk_02", "cut_cop_alert_02", "cut_cop_fidget_02"] {
                            if self.cine.has(s) {
                                self.cine.read_ahead_scene(s);
                            }
                        }
                    }
                    Some(police::Event::Pursuit) => {
                        for s in self.cine.scenes("cut_cop_arrest").into_iter().filter(|s| !s.contains("bike")) {
                            self.cine.read_ahead_scene(&s);
                        }
                    }
                    Some(police::Event::Escaped) => {
                        let exp = rewards::pursuit_exp(self.game.rep, std::mem::take(&mut self.police.chase_time));
                        if exp > 0 {
                            // (no upper limit: F-55689)
                            self.game.rep += exp;
                            self.game.show_message(&format!("YOU ESCAPED   REP +{}", exp));
                        } else {
                            self.game.show_message("YOU ESCAPED");
                        }
                        self.game.career.event(career::Ev::EscapedPolice);
                    }
                    Some(police::Event::Ticket(f)) => {
                        // (StateTicket's enter, F-14125: the bust bits 0x28 on a car not damaged out -
                        // the wipeout camera behind the car)
                        if self.vehicles.player().is_some_and(|c| c.state.damage < 0.999) {
                            self.crash_bits |= 0x28;
                        }
                        // COP_PULLOVER -> CopPullover.sco: the car put upright and held as on the grid, the officer's
                        // walk from the cruiser 10 m behind (always cut_cop_walk_02), no skipping; the ticket scene and
                        // the TICKETED panel (the ticket form) come after it - `ticket_tick`
                        let car = police::scene_frame(&mut self.vehicles, &world, &self.police.safe_zones);
                        self.ticket_run = Some((f, None, false));
                        if let (Some(car), true, true) = (car, self.cine.has("cut_cop_walk_02"), self.cine.playing.is_none() && std::env::var("MCLA_NO_CINE").is_err()) {
                            if self.cine.start_bound(&mut gpu, "cut_cop_walk_02", car) {
                                self.cine_spawn_cars(&mut gpu, &mut world);
                                self.cine_free = true;
                                self.cine_caption = None;
                                self.vehicles.player_hold = true;
                                // (CopPullover.sco after Racer_PreRaceBurnout: scramble 0x600 - the handbrake held on and no
                                // steering, so the throttle alone spins the held car)
                                self.vehicles.player_scramble = 0x600;
                            }
                        }
                    }
                    Some(police::Event::Busted(_)) => {
                        // (StateArrest's enter, F-14125: the bust bits 0x28 on a car not damaged out; the fine is taken
                        // when the tactic succeeds - COP_ARREST, below)
                        if self.vehicles.player().is_some_and(|c| c.state.damage < 0.999) {
                            self.crash_bits |= 0x28;
                        }
                    }
                    // (one CopArrest.sco at a time: an arrest posted while one runs is not started again)
                    Some(police::Event::ArrestScene) if self.arrest_wait || self.game.arrest_scene == 1 => {}
                    Some(police::Event::ArrestScene) => {
                        // (after StateArrest's fArrestStateTime: COP_ARREST, F-22530, runs CopArrest.sco - the scene,
                        // then the panel with the pursuit's length and the fine)
                        // (F-22530 when the tactic succeeds: a cops' tactic - not the payback's goons, whose first car has
                        // no +872 - fills arrest_duration / cop_fine, takes the fine and opens form 46 after the scene)
                        let goons = self.police.goons;
                        if !goons {
                            let f = self.police.fine as i64;
                            let chase = std::mem::take(&mut self.police.chase_time);
                            self.game.money = (self.game.money - f).max(0);
                            self.game.write_save(&self.opt.data_dir);
                            // (Form_ShowForm(46) opens after the scene, CopArrest.sco 0x106f: the panel waits for it)
                            self.arrest_form = Some((chase, f));
                            let st = |k: &str, fb: &str| self.game.career.strings.get(k).map(|s| s.to_uppercase()).unwrap_or_else(|| fb.to_string());
                            let m = format!("{}   {} ${}", st("arrest_title", "ARRESTED"), st("cop_fine_header", "FINE:"), f);
                            self.game.show_message(&m);
                        }
                        // (CopArrest.sco 0x36f: fn 0x37c(1, 1, 1.0, 0) - a 1 s fade to black, waited, before anything else;
                        // the scene starts under the full black, below)
                        self.arrest_mask.start(1.0, 0.01, f32::MAX);
                        self.arrest_wait = true;
                        self.game.arrest_scene = 0;
                        self.police.hold_busted(1.5);
                    }
                    None => {}
                }
            }
            if self.arrest_wait && (self.arrest_mask.full() || !self.arrest_mask.is_on()) {
                self.arrest_wait = false;
                let goons = self.police.goons;
                let mut started_scene = false;
                        // the arrest (CopArrest.sco): the wiped-out biker's scene first (manager +64, F-22530), then the
                        // payback's (race type 5), else the scene by the number of cops in the chase
                        // (PoliceManager_GetNumCopsInLatestTactic)
                        let bike = self.vehicles.player().is_some_and(|c| c.model.bike && c.state.wipeout > 0.0);
                        let cops = self.police.tactic_cops();
                        let scene = police::arrest_scene(cops, (self.tick_no as u32).wrapping_mul(2654435761) >> 8, bike, goons);
                        // (0x6d0: OpponentManager_HideOpponents - only with a race current, before 0x953 / 0xa39 read the
                        // player's place. A cops' bust ends every other race first (F-22530 -> the race scripts'
                        // Race_ResetCurrentRace, 1 - 2 frames, inside CopArrest's waited fade down): only a payback is
                        // still current, and its entry 0 - the player - goes to the lot too)
                        if let Some(pb) = self.game.race.as_ref().map(|r| r.kind == game::RaceKind::Payback) {
                            police::hide_opponents(&mut self.vehicles, &world, pb);
                        }
                        // (placed as the pull-over's: police::scene_frame - never the car's own roll; GitHub #17)
                        let car = police::scene_frame(&mut self.vehicles, &world, &self.police.safe_zones);
                        self.arrest_hands = false;
                        if let (Some(car), true, true) = (car, self.cine.has(scene), self.cine.playing.is_none() && std::env::var("MCLA_NO_CINE").is_err()) {
                            // (the scene is bound to the ground frame itself, SetGameCoordSysBindingCopyRacer; its car
                            // event snaps the player's car to frame x mover at t 0 - the bikestand's mover_player_car is
                            // 10.36 m from the origin, the bike is moved there. The car scenes' movers are at the origin.)
                            let started = if bike { self.cine.start(&mut gpu, scene, car) } else { self.cine.start_bound(&mut gpu, scene, car) };
                            if started {
                                started_scene = true;
                                self.arrest_fade = police::arrest_fade(scene);
                                // (the scene's t 0 event on slot 0: car_off in cut_cop_arrest, _02, _03, _08 and the
                                // bikestand, engine_off in _07 (F-91263) - the engine stays off until CopArrest's
                                // Racer_PlayIgnition after the form)
                                // (0x6e4: the traffic off for the scene - on again at 0x109f, after it; 0x1010 / 0x1038: the
                                // mixer scenes, POLICE_SCRIPT_TRIGGERED only on the cops' path)
                                self.arrest_traffic = Some(self.vehicles.traffic_target);
                                self.vehicles.traffic_target = 0;
                                self.sound.arrest_mix = (!goons, true);
                                if police::arrest_engine_off(scene) {
                                    if let Some(c) = self.vehicles.player_mut() {
                                        c.engine_off = true;
                                    }
                                }
                                self.cine_spawn_cars(&mut gpu, &mut world);
                                self.cine_free = true;
                                self.cine_caption = Some(String::new());
                                let secs = self.cine.playing.as_ref().map_or(0.0, |sc| sc.duration);
                                self.police.hold_busted(secs);
                            }
                        }
                // (core 0xfb3: EndDescriptionAndStartClock, WAITUNWARPED(10), the fade up (0, 1, 1.0, 0) not waited)
                self.arrest_mask.release(1.0);
                self.game.arrest_scene = if started_scene { 1 } else { 2 };
            }
            self.ticket_tick(&mut gpu, &mut world);
            crate::ticktimes::mark(crate::ticktimes::S::Other);
            if let (Ok(t), 90) = (std::env::var("MCLA_PROPFX_AIM"), self.tick_no) {
                // test helper "x,y,z[,run]": line the player up with the prop at that place, `run` (14) m away on the first clear line
                let v: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let (true, Some(c)) = (v.len() >= 3, self.vehicles.player_mut()) {
                    let t = Vec3::new(v[0], v[1], v[2]);
                    let run = v.get(3).copied().unwrap_or(14.0);
                    for turn in 0..16 {
                        let dir = glam::Quat::from_rotation_y(turn as f32 * std::f32::consts::FRAC_PI_8) * Vec3::X;
                        let start = t - dir * run + Vec3::Y * 0.7;
                        let side = Vec3::new(-dir.z, 0.0, dir.x) * 1.1;
                        if [Vec3::ZERO, side, -side].iter().any(|&o| world.collider.raycast(start + o, dir, run - 1.0).is_some()) {
                            continue;
                        }
                        let Some(g) = world.collider.ground(start, 1.0, 2.0) else { continue };
                        if (g.point.y - t.y).abs() > 0.4 {
                            continue;
                        }
                        c.state = physics::car::CarState::new(start, (-dir.x).atan2(-dir.z));
                        c.pose_history.reset();
                        log::info!("propfx aim: from {:?} towards {:?}", start, t);
                        break;
                    }
                }
            }
            if let Some(kind) = self.opt.aim_prop.clone() {
                // test helper: line the player up with the nearest prop of the given kind
                if self.tick_no == 90 {
                    if let Some(c) = self.vehicles.player_mut() {
                        let from = c.state.pos;
                        'found: for t in self.props.colliders_near(from, kind == "solid", 400.0) {
                          // ("wall": straight at the first loose prop with a clear middle line - the car ends
                          // against what stands behind or beside it)
                          let wall = kind == "wall";
                          for turn in 0..if wall { 1 } else { 8 } {
                            let d = (t - from) * Vec3::new(1.0, 0.0, 1.0);
                            let dir = glam::Quat::from_rotation_y(turn as f32 * std::f32::consts::FRAC_PI_4) * d.normalize_or(Vec3::X);
                            let start = t - dir * 16.0 + Vec3::Y * 0.7;
                            // need a clear, level run-up as wide as the car, to and past the prop
                            let side = Vec3::new(-dir.z, 0.0, dir.x) * 1.1;
                            if wall && world.collider.raycast(start, dir, 15.0).is_some() {
                                continue;
                            }
                            if !wall && [Vec3::ZERO, side, -side].iter().any(|&o| world.collider.raycast(start + o, dir, 19.0).is_some()) {
                                continue;
                            }
                            let Some(g) = world.collider.ground(start, 1.0, 2.0) else { continue };
                            if (g.point.y - t.y).abs() > 0.3 {
                                continue;
                            }
                            let yaw = (-dir.x).atan2(-dir.z);
                            c.state = physics::car::CarState::new(start, yaw);
                            c.pose_history.reset();
                            log::info!("aim-prop: {} prop at {:?}, starting at {:?}", kind, t, start);
                            break 'found;
                          }
                        }
                    }
                }
                if self.tick_no % 30 == 0 {
                    log::info!("aim-prop: knocked {} speed {:.1}", self.props.knocked, self.vehicles.player().map_or(0.0, |c| c.state.speed()));
                }
            }
        }
        if self.game.mode == Mode::Race {
            if race_finished {
                // player finished: gather results
                let mission = self.game.race.as_ref().map(|r| (r.kind, r.failed, r.time)).filter(|m| m.0 != game::RaceKind::Race);
                let res = if let Some((kind, failed, time)) = mission {
                    let what = if kind == game::RaceKind::Delivery { "DELIVERY" } else { "PAYBACK" };
                    let delivered = (kind == game::RaceKind::Delivery).then(|| self.vehicles.player().map_or(0.0, |c| c.state.delivery_damage()));
                    // (step 19d: a leg in which the client's friends were lost is no win and
                    // pays nothing - the run goes on: `mc4_payback_take_lost`)
                    let s19d_lost = kind == game::RaceKind::Payback && failed.is_none() && self.game.mc4_payback_take_lost();
                    if !s19d_lost {
                        self.game.award_mission(failed.is_none(), delivered);
                    }
                    // (the delivery's results panel, form 51 - F-88581)
                    if let (Some(d), true) = (delivered, self.game.flash_menu.is_some()) {
                        let limit = self.game.race.as_ref().map_or(0.0, |r| r.time_limit);
                        self.game.delivery_panel(failed.is_none().then_some(time), limit, d);
                    }
                    // (PAYBACK_COMPLETE comes with the last of the legs asked for: `mc4_mission_pay`)
                    if failed.is_none() && (kind == game::RaceKind::Delivery || std::mem::take(&mut self.game.career.payback_done)) {
                        self.game.career.event(if kind == game::RaceKind::Delivery { career::Ev::Delivery } else { career::Ev::Payback });
                    }
                    match failed {
                        // (in the game's words: DELIVERY SUCCESSFUL / DELIVERY FAILURE, PAYBACK SUCCESSFUL / PAYBACK FAILED)
                        None => {
                            let key = if kind == game::RaceKind::Delivery { "DELIVERY_SUCCESS" } else if s19d_lost { "PB_Lost" } else { "Payback_SUCCESS" };
                            let t = self.game.career.strings.get(key).map(|s| s.to_uppercase()).unwrap_or_else(|| format!("{} SUCCESSFUL", what));
                            vec![(format!("YOU - {}", t), Some(time))]
                        }
                        Some(why) => {
                            let key = if kind == game::RaceKind::Delivery { "DELIVERY_FAILURE" } else { "PAYBACK_FAILURE" };
                            let t = self.game.career.strings.get(key).map(|s| s.to_uppercase()).unwrap_or_else(|| format!("{} FAILED", what));
                            vec![(format!("{} - {}", t, why), None)]
                        }
                    }
                } else {
                    let standings = self.game.standings(&self.vehicles);
                    let mut res = Vec::new();
                    for (ci, t, _) in standings {
                        let c = &self.vehicles.cars[ci];
                        let name = if c.is_player {
                            format!("YOU ({})", game::pretty_car_name(&c.model.name))
                        } else if !c.driver.is_empty() {
                            format!("{} ({})", self.game.career.name_of(&c.driver), game::pretty_car_name(&c.model.name))
                        } else {
                            game::pretty_car_name(&c.model.name)
                        };
                        res.push((name, t));
                    }
                    let field = res.len();
                    let place = res.iter().position(|r| r.0.starts_with("YOU")).map_or(field, |p| p + 1);
                    // (level with another car to the hundredth: TIED)
                    let mine = res.iter().find(|r| r.0.starts_with("YOU")).and_then(|r| r.1);
                    self.game.place_tied = mine.map_or(false, |m| res.iter().filter(|r| !r.0.starts_with("YOU")).filter_map(|r| r.1).any(|t| (t - m).abs() < 0.005));
                    // (a time trial: the best time, the target beaten or not, the result's panel - `mc4`)
                    let s18_id = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).map(|d| d.id.clone()).unwrap_or_default();
                    self.game.mc4_tt_end(&s18_id, mine);
                    self.game.award_race(place, field);
                    if let Some(sp) = self.game.alt_finished(place) {
                        log::info!("alt hookman: {} after the race", sp);
                        self.speech.say_as(sp, &self.game.althk.voice);
                    }
                    // (a hookman who had to be beaten was not: his mission's panel, stamped FAIL -
                    // the rewards movie's own look; approx., the original's was not captured)
                    if self.game.hookman_race && place > 1 && self.game.flash_menu.is_some() {
                        if let Some(id) = self.game.career.hookman_mission().filter(|id| id.starts_with("BEAT_") || id.starts_with("REDLIGHT_")).map(|s| s.to_string()) {
                            if let Some(title) = self.game.career.title_of(&id) {
                                self.game.mission_results.push((title, self.game.career.objective_texts(&id), false));
                                self.game.career_over = true;
                            }
                        }
                    }
                    // (no scene after a race: the original's reward scenes - cut_reward_gen_01,
                    // _marcel, _nikolai - are played by three story scripts for their own
                    // missions only. The race ends in slow motion: `race_slow_tick`)
                    if std::env::var("MCLA_NO_RACE_SLOW").is_err() {
                        self.game.race_slow_wait = Some(game::Game::RACE_END_PANEL);
                        self.game.race_end_t = Some(0.0);
                    }
                    let district = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).map(|d| d.district.clone()).unwrap_or_default();
                    let class = self.vehicles.player().and_then(|c| self.game.catalog.get(&c.model.name)).map(|c| c.class.clone()).unwrap_or_default();
                    let hookman = std::mem::take(&mut self.game.hookman_race);
                    let (race_id, goal) = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).map(|d| (d.id.clone(), d.goal_time)).unwrap_or_default();
                    let my_time = res.iter().find(|r| r.0.starts_with("YOU")).and_then(|r| r.1);
                    let beat_goal = goal > 0.0 && my_time.map_or(false, |t| t <= goal);
                    // (a time trial's target is the save's, not the race file's goal: `mc4_tt_end`)
                    let beat_goal = self.game.career.tt_last.unwrap_or(beat_goal);
                    // part of a career event (series, tournament)? then the next race follows
                    let lid = race_id.to_ascii_lowercase();
                    let mine: Vec<String> = self.game.career.mission_races().into_iter().filter(|(r, _)| lid.ends_with(r.as_str())).map(|(_, t)| t).collect();
                    // (a tournament race pays points by place; the event is decided after its last race)
                    let order: Vec<String> = res.iter().map(|r| if r.0.starts_with("YOU") { "YOU".to_string() } else { r.0.clone() }).collect();
                    // (step 21b: a pink slip race is the wager core's own, no event's leg)
                    // (form 49's standings are the last tournament race's: none after any other)
                    self.game.career.tour_table = None;
                    let tournament = if self.game.mc4_pink_was() { None } else { self.game.career.tournament_race(&race_id, &order).or_else(|| order.first().and_then(|w| self.game.career.series_race(&race_id, w))) };
                    // (TournamentCore's EndOfRaceSequence: the standings (form 49) after every race,
                    // before REWARDS, until CONTINUE - as a series' results)
                    if self.game.career.tour_table.is_some() {
                        self.game.series_first = true;
                    }
                    self.game.career.event(career::Ev::Race { district: &district, place, hookman, class: &class, race: &race_id, beat_goal });
                    // (a finished race is an unlocked race: `mc4`)
                    self.game.mc4_race_finished(&race_id);
                    // (step 23: the race-end statistics, F-98556)
                    self.game.mc4_race_stats(place, &class, my_time);
                    // (step 23b: the race's goals - F-96807 from F-98556)
                    let s23b_dmg = self.vehicles.player().map_or(0.0, |c| c.state.damage);
                    self.game.mc4_race_goals(place, my_time, s23b_dmg);
                    // (GOAL ATTACK: the race's end hook F-84893 - SetAllPoliceUnderground(0, 1))
                    if self.game.goal_attack {
                        self.police.units_up();
                    }
                    self.game.event_line.clear();
                    self.game.next_event_race = None;
                    // (a lost tournament is over: no next race - the man cruises again, bFailure)
                    let tour_lost = self.game.career.tour_table.as_ref().is_some_and(|t| t.decided == Some(false));
                    if let Some(title) = mine.first().filter(|_| !tour_lost) {
                        let left: Vec<String> = self.game.career.mission_races().into_iter().filter(|(_, t)| t == title).map(|(r, _)| r).collect();
                        if left.is_empty() {
                            self.game.event_line = format!("{}   EVENT WON", title.to_uppercase());
                        } else {
                            // (a lost race stays on the list and is simply run again)
                            let next = left.iter().find(|r| !lid.ends_with(r.as_str())).or(left.first()).cloned().unwrap_or_default();
                            self.game.next_event_race = (0..self.game.race_defs.len()).find(|&i| self.game.race_defs[i].id.to_ascii_lowercase().ends_with(next.as_str()));
                            self.game.event_line = format!("{}   {} TO GO", title.to_uppercase(), if left.len() == 1 { "1 RACE".to_string() } else { format!("{} RACES", left.len()) });
                        }
                    }
                    // a hangout's series: the winner's tally; three wins end it, else the next race
                    let mut s15_series_done: Option<(String, usize)> = None;
                    if let Some(sr) = self.game.series.as_mut() {
                        let winner = res.first().map(|r| if r.0.starts_with("YOU") { "YOU".to_string() } else { r.0.clone() }).unwrap_or_default();
                        match sr.wins.iter_mut().find(|w| w.0 == winner) {
                            Some(w) => w.1 += 1,
                            None => sr.wins.push((winner, 1)),
                        }
                        self.game.series_first = true;
                        let over = sr.wins.iter().find(|w| w.1 >= game::SERIES_WINS).map(|w| w.0.clone());
                        match over {
                            Some(w) => {
                                self.game.next_event_race = None;
                                if w == "YOU" {
                                    // (SERIES_COMPLETE: the event of the hangout completes at the
                                    // difficulty chosen there - its reward is the panel's
                                    // COMPLETION REWARD; below, `mc4_event_complete`)
                                    s15_series_done = Some((sr.key.clone(), sr.diff));
                                }
                                // (form 56 for 3.5 s after the place's ring: SERIES WON / SERIES LOST -
                                // EndOfRaceSequence, OrderedSeriesCore's bComplete / bFailure)
                                self.game.opt_x.arcade.d.banner = Some((3.5, w == "YOU"));
                                self.game.opt_x.arcade.d.banner_kind = 3;
                                log::info!("series {}: won by {} ({:?})", sr.key, w, sr.wins);
                            }
                            None => {
                                sr.at = (sr.at + 1) % sr.races.len().max(1);
                                self.game.next_event_race = sr.races.get(sr.at).copied();
                                self.game.career.stats.insert(format!("series_at:{}", sr.key.trim_end_matches("_series")), ((sr.at + 1) % sr.races.len().max(1)) as i64);
                                log::info!("series {}: tally {:?}, next race {:?}", sr.key, sr.wins, self.game.next_event_race);
                            }
                        }
                    }
                    // the events that just completed: their counters and their rewards (F-67065)
                    let mut s15_done: Vec<(String, usize)> = s15_series_done.into_iter().collect();
                    for (key, diff) in std::mem::take(&mut self.game.career.completed_events) {
                        // (at the difficulty the tournament was entered with at its panel)
                        s15_done.push((key, diff));
                    }
                    for (key, diff) in s15_done {
                        let pay = self.game.mc4_event_complete(&key, diff, 1.5);
                        if pay.money > 0 {
                            self.game.money += pay.money;
                            self.game.last_reward.0 += pay.money;
                            self.game.reward_panel = true;
                        }
                    }
                    // (a car given by what the race completed has its page on the rewards screen)
                    if self.game.mc4_rewards_listed() {
                        self.game.reward_panel = true;
                    }
                    // (form 56 after the last race of a tournament: TOURNAMENT WON / TOURNAMENT LOST, 3.5 s -
                    // TournamentCore's EndOfRaceSequence, F-66541's bComplete / bFailure)
                    if let Some(won) = self.game.career.tour_decided.take() {
                        self.game.opt_x.arcade.d.banner = Some((3.5, won));
                        self.game.opt_x.arcade.d.banner_kind = 4;
                    }
                    if let Some(line) = tournament {
                        // (the points decide: the career hears of it with the next event)
                        self.game.event_line = if self.game.event_line.is_empty() { line } else { format!("{}   -   {}", self.game.event_line, line) };
                    }
                    res
                };
                if self.game.wager_paid {
                    self.game.career.event(career::Ev::Money(self.game.last_reward.0));
                }
                self.game.career.event(career::Ev::Rep(self.game.rep));
                self.game.last_results = res;
                self.game.mode = Mode::Results;
                self.game.write_save(&self.opt.data_dir);
                // (a payback goes on with its next client at once, without a menu - PaybackCore,
                // `script_obj/game/0x3ae26fc8/0x422e62e2`: `mc4_payback_chain`)
                let s15b_from = self.vehicles.player().map(|c| c.state.pos).unwrap_or(self.cam.pos);
                let s15b_paid = self.game.last_reward.0;
                if self.game.mc4_payback_chain(s15b_from) {
                    let text = self.game.career.strings.get("PB_Payment").map(|s| { let a = s.find("<D>").unwrap_or(s.len()); format!("{}{}", &s[..a], s15b_paid).to_uppercase() }).unwrap_or_else(|| format!("NICE JOB PAYMENT RECEIVED: $ {}", s15b_paid));
                    // (step 19d: nothing was paid for a leg the chasers were lost in)
                    if s15b_paid > 0 {
                        self.game.show_message(&text);
                    }
                    self.pending_action = MenuAction::StartRace;
                }
            }
        }
        // race checkpoints as the original shows them: the road flare (checkpoint_ctf_generic)
        // tinted with the look's m_ParticleColor and, for the finish looks, the light column -
        // only the player's open checkpoints (`checkpoints.rs`)
        {
            use checkpoints::Shown;
            // (test hook MCLA_CP_NEXT=<k>: the player's next checkpoint is k from the start - a
            // circuit's lap end and finish lie by its grid; for screenshots of the looks)
            if let (Ok(v), Some(r)) = (std::env::var("MCLA_CP_NEXT"), self.game.race.as_mut()) {
                if let (Ok(k), Some(pi)) = (v.parse::<usize>(), self.vehicles.cars.iter().position(|c| c.is_player)) {
                    if r.time < 0.05 && pi < r.next.len() && !r.checkpoints.is_empty() {
                        r.next[pi] = k.min(r.checkpoints.len() - 1);
                    }
                }
            }
            let mut shown: Vec<Shown> = Vec::new();
            if self.game.mode == Mode::Race {
                if let Some(fg) = self.game.flag.as_ref() {
                    // (the port's offline flag games: the flags wear the original's flag pick-up
                    // look, the base the drop-off look - ctfpickupnoteam / ctfdropoffnoteam)
                    for (k, (p, kind)) in fg.marker_looks(&self.vehicles).into_iter().enumerate() {
                        shown.push(Shown { key: 0xf1a0_0000 + ((kind as u64) << 8) + k as u64, pos: p, kind, height: 100.0 });
                    }
                } else if let Some(race) = self.game.race.as_ref().filter(|r| r.kind != game::RaceKind::Payback) {
                    let next = race.next.first().copied().unwrap_or(0);
                    let n = race.checkpoints.len();
                    let at = |c: Vec3| c - Vec3::Y * 0.2;
                    let pkey = |c: Vec3, kind: u8| (((c.x * 4.0) as i64 as u64) & 0xff_ffff) | ((((c.z * 4.0) as i64 as u64) & 0xff_ffff) << 24) | ((kind as u64) << 56);
                    if race.rivals_checkpoints.is_some() && next < n {
                        // (an unordered race: every checkpoint left is open, unorderedcp; the finish
                        // opens as lightcolumncp when they are done - the unordered core 0x745, 0x16e0)
                        if next + 1 < n {
                            for c in &race.checkpoints[next..n - 1] {
                                shown.push(Shown { key: pkey(*c, checkpoints::UNORDERED), pos: at(*c), kind: checkpoints::UNORDERED, height: 100.0 });
                            }
                        } else {
                            let c = race.checkpoints[n - 1];
                            shown.push(Shown { key: pkey(c, checkpoints::LIGHT_COLUMN), pos: at(c), kind: checkpoints::LIGHT_COLUMN, height: 100.0 });
                        }
                    } else if next < n && race.finish_time.first().map_or(true, |t| t.is_none()) {
                        // (ordered: the next one only - OrderedCheckpointCore 0x824)
                        let ll = race.lap_len.max(1);
                        let kind = checkpoints::ordered_kind(next % ll, ll, next / ll, race.laps);
                        shown.push(Shown { key: 0xc0de_0000 + next as u64, pos: at(race.checkpoints[next]), kind, height: 100.0 });
                    }
                }
            } else if self.game.mode == Mode::FreeRoam {
                // (beat me there: one checkpoint at the race's start beacon, finalcp -
                // generic_local_hookman.sco 0x11c1..0x11e9)
                if let Some(ch) = self.roam.challenge.as_ref() {
                    shown.push(Shown { key: 0xbee7_0000, pos: ch.goal, kind: checkpoints::FINAL, height: 100.0 });
                }
            }
            let cp3_raw: Vec<Vec3> = shown.iter().map(|s| s.pos).collect();
            // (the script's height of each - the flare's "Height" evolution - and +112: the marker
            // stands on the ground found from 2 m above to 2 m below it, F-34461)
            if let Some(race) = self.game.race.as_ref() {
                for s in shown.iter_mut() {
                    if let Some(sp) = checkpoints::spec_at(&race.cp_specs, s.pos + Vec3::Y * 0.2) {
                        s.height = sp.height;
                    }
                }
            }
            for s in shown.iter_mut() {
                if let Some(h) = world.collider.raycast(s.pos + Vec3::Y * 2.0, -Vec3::Y, 4.0) {
                    s.pos = h.point;
                }
            }
            // (test hook MCLA_CP_KIND=<0..17>: every open checkpoint wears that look - screenshots)
            if let Some(k) = std::env::var("MCLA_CP_KIND").ok().and_then(|v| v.parse::<u8>().ok()).filter(|k| *k < 18) {
                for s in shown.iter_mut() {
                    s.kind = k;
                }
            }
            // (the arrows projected onto the low smoke - checkpoints::arrow_plan; none in a flag
            // game: the CTF game object 0x8286EC44 turns them off, F-45072)
            self.cp_markers.arrows = None;
            if self.game.mode == Mode::Race && self.game.flag.is_none() {
                if let Some(race) = self.game.race.as_ref().filter(|r| r.kind != game::RaceKind::Payback) {
                    self.cp_markers.arrows = checkpoints::arrow_plan(race, &shown, &cp3_raw, race.rivals_checkpoints.is_some());
                }
            }
            if self.game.mode != Mode::Paused && std::env::var("MCLA_NO_MARKERS").is_err() {
                self.cp_markers.update(&shown, dt, &mut self.fx, self.cam.pos);
            }
        }
        self.sim_time += dt;
        if self.tick_no % 240 == 0 {
            log::info!("perf: {:.0} fps, {:.2} ms frame, {} draws", self.fps, self.frame_ms, self.draws);
        }
        if self.tick_no % 240 == 1 || (self.opt.wot && self.tick_no % 60 == 1) {
            if let Some(c) = self.vehicles.player() {
                if std::env::var("MCLA_CADENCE_TRACE").is_ok() {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    format!("{:?}", c.state).hash(&mut h);
                    log::info!("cadence tick {} state {:016x}", self.tick_no, h.finish());
                }
                let cp = self.game.race.as_ref().map(|r| (r.next.clone(), r.checkpoints.len()));
                if std::env::var("MCLA_WHEEL_DUMP").is_ok() {
                    log::info!("burnout {:.2} launch {:.2} yaw {:.2} slip {:.2}", c.state.burnout, c.state.launch, { let f = c.state.forward(); f.x.atan2(-f.z) }, c.state.wheels[0].slip);
                    log::info!("wheels local: {:?} up {:?} fwd {:?}", c.state.wheels.iter().map(|w| c.state.rot.inverse() * (w.world_pos - c.state.pos)).collect::<Vec<_>>(), c.state.up(), c.state.forward());
                    log::info!("wheels: body y {:.3}, hubs y {:?}, contact y {:?}, compression {:?}, wheel_pos {:?} r {:.3} rest {:.2} travel {:.2}", c.state.pos.y, c.state.wheels.iter().map(|w| w.world_pos.y).collect::<Vec<_>>(), c.state.wheels.iter().map(|w| w.contact.y).collect::<Vec<_>>(), c.state.wheels.iter().map(|w| w.compression).collect::<Vec<_>>(), c.model.params.wheel_pos, c.model.params.wheel_radius, c.model.params.susp_rest, c.model.params.susp_travel);
                }
                log::info!("player t={:.2} {:?} speed {:.1} rpm {:.0} gear {} grounded {} on {:?} cars {} race {:?}", self.sim_time, c.state.pos, c.state.fwd_speed(), c.state.rpm, c.state.gear, c.state.wheels.iter().filter(|w| w.grounded).count(), physics::collision::Surface::from_u8(c.state.wheels[0].surface), self.vehicles.cars.len() - 1, cp);
            }
        }
        let rain_snd = self.rain_amount();
        // (the start scene's events on the racers: gestures, windows, the taunt's speech and jaws)
        self.prerace_acts_tick(dt);
        // the arrests' gesture_cophandsup on racer slot 0, the player (event 53: racer+684 = 18 until its end call)
        match self.cine.playing.as_ref().and_then(|sc| police::arrest_gesture(&sc.name).map(|h| (h, sc.t))) {
            Some(((g, t, dur), now)) if now >= t && !self.arrest_hands => {
                self.arrest_hands = true;
                if let Some(i) = self.vehicles.cars.iter().position(|c| c.is_player) {
                    self.drivers.gesture(i, g, dur, true);
                }
            }
            Some(_) => {}
            None => self.arrest_hands = false,
        }
        // the pull-over scenes' acts on the player's racer as the scene clock passes them
        if let Some((name, now)) = self.cine.playing.as_ref().map(|sc| (sc.name.clone(), sc.t)) {
            if self.pullover_acts.0 != name {
                self.pullover_acts = (name.clone(), 0);
            }
            let acts = police::pullover_acts(&name);
            // (an event at 0 fires on the scene clock's first tick after its start, a frame after the last scene's
            // FinishScene at the earliest: the walk's stopped gesture has left by then, F-12285 / F-26895)
            while let Some(&(_, act)) = acts.get(self.pullover_acts.1).filter(|a| now >= a.0 && now > 0.0) {
                self.pullover_acts.1 += 1;
                if let Some(i) = self.vehicles.cars.iter().position(|c| c.is_player) {
                    match act {
                        police::SceneAct::Gesture(g) => self.drivers.gesture(i, g, 0.0, true),
                        police::SceneAct::LeftWindow(down) => self.vehicles.window(i, false, down),
                    }
                }
            }
        } else {
            self.pullover_acts = (String::new(), 0);
            // (CopArrest.sco: the form after the scene - 0x1084 fn 0x37c(1, 1, 1.0, 0), waited: a no-op when the scene's
            // black is full or coming, a 1 s fade down on a clear screen)
            if !self.arrest_wait && self.arrest_form.is_some() {
                self.arrest_mask.start(1.0, 0.01, f32::MAX);
                if self.arrest_mask.full() {
                    if let Some((chase, f)) = self.arrest_form.take() {
                        self.game.arrest_panel = Some((0.0, chase, f));
                        self.arrest_station = Some(0);
                    }
                }
            }
        }
        // (the arrest scene's fade_to_black, F-91263: game/misctasks/screen_fade_to_black (0, 1, 1.0, 0), held)
        if let (Some(at), Some(now)) = (self.arrest_fade, self.cine.playing.as_ref().map(|sc| sc.t)) {
            if now >= at {
                self.arrest_fade = None;
                self.arrest_mask.start(1.0, 0.01, f32::MAX);
            }
        } else if self.cine.playing.is_none() {
            self.arrest_fade = None;
        }
        // (PaybackCore 0xd50: the fade up over the goons' walk-up's black; a black left with no race to lift it goes too)
        if std::mem::take(&mut self.game.pb_fade_up) || (self.arrest_mask.is_on() && !self.arrest_wait && self.cine.playing.is_none() && self.arrest_form.is_none() && self.arrest_station.is_none() && !self.arrest_activate && self.game.race.is_none() && self.game.arrest_scene == 2) {
            self.arrest_mask.release(1.0);
        }
        // (CopArrest.sco 0x1178 once the form is closed: Racer_InitializeResetPosition(player, (-604.5, 7, -340.5), 5 deg)
        // + ResetWithDamageIntact - the police station, upright, standing, the damage kept; PoliceManager_CutSceneDone
        // F-30655 then puts a damaged-out car's damage at 0.95 of its MaxDamage)
        // (CopArrest.sco after the scene: 0x109f the traffic on again; the goons' path ends there - its mixer detriggered)
        if self.cine.playing.is_none() {
            if let Some(n) = self.arrest_traffic.take() {
                self.game.arrest_scene = 2;
                self.vehicles.traffic_target = n;
                if !self.sound.arrest_mix.0 {
                    self.sound.arrest_mix = (false, false);
                }
            }
        }
        // (test hook MCLA_ARREST_CLOSE=1: the ARRESTED form closed by itself on a 2 s tick)
        if std::env::var_os("MCLA_ARREST_CLOSE").is_some() && self.arrest_station == Some(0) && self.tick_no % 120 == 0 {
            self.game.arrest_panel = None;
        }
        if let (Some(n), true) = (self.arrest_station, self.game.arrest_panel.is_none()) {
            self.arrest_station = police::station_reset(&mut self.vehicles, &world, n);
            if self.arrest_station.is_none() {
                // (0x1199: fn 0x11a6(1, 1, 1.0, 0) - the 1 s fade up, waited, before the car is given back)
                self.arrest_mask.release(1.0);
                self.arrest_activate = true;
            }
        }
        if self.arrest_activate && !self.arrest_mask.is_on() {
            self.arrest_activate = false;
            self.sound.arrest_mix = (false, false);
            // (CopArrest.sco 0x14ee: the mixer detriggered, 0x1524 Racer_PlayIgnition - the engine sound's start, its
            // crank ends with the engine running again -, Racer_Activate, and 0x1552 after CutSceneDone:
            // SetAllPoliceUnderground(0, 1))
            self.sound.play_ignition();
            self.police.units_up();
        }
        // (F-10725: the crank's end sets the engine running again, chassis engine +46 = 1)
        if self.sound.take_engine_running() {
            if let Some(c) = self.vehicles.player_mut() {
                c.engine_off = false;
            }
        }
        if let Some(a) = self.audio.as_mut() {
            let fwd = self.cam.forward();
            let right = Vec3::new(-fwd.z, 0.0, fwd.x).normalize_or(Vec3::X);
            let horn = (self.game.opt_x.ctrl.kbd.binds.down(crate::keybinds::HORN, |k| self.input.down(k)) || self.pad_horn) && !self.game.in_menu();
            // Zone slows the world: sounds drop in pitch with it
            let o = self.game.opts;
            if self.sound.lib.is_some() {
                // the original's levels: the five sliders are category controllers, nothing after the voices
                // (docs/reference/audio/VOLUME_RULE.md); the pause is the PAUSE scene
                a.set_master(1.0);
                self.sound.sliders = [o.master, o.music, o.voice, o.vehicle, o.cutscene];
                // the audio camera mode (F-79297): 0 bumper, 1 driver, 2 near, 3 medium, 4 far, 5 cinematic, 6 other
                self.sound.cam_audio = if self.cuts.playing.is_some() || self.cine.playing.is_some() || self.game.replay.is_some() {
                    6
                } else if self.vehicles.player().is_none() || self.game.in_menu() && self.game.mode != Mode::Paused {
                    -1
                } else {
                    [4, 3, 2, 1, 0, 5].get(self.cam_mode as usize).copied().unwrap_or(2)
                };
                // the performance stages the car sounds read: intake, turbo, nitrous (shop parts 2, 3, 6)
                let st = self.game.perf_stages(self.game.current_car);
                self.sound.parts = [st[2], st[3], st[6]];
            } else {
                a.set_master(if self.game.mode == Mode::Paused { 0.35 } else { 0.9 } * o.master as f32 / 10.0);
                a.music_volume = o.music as f32 * 0.05 * o.master as f32 / 10.0;
            }
            // (steering sensitivity: the middle of the scale is the tuning's own steer rate)
            // (the save's own step where it lies between two of the port's: `mc4`)
            physics::vehsim::STEER_SENS.store((0.5 + self.game.sens_exact() * 0.1).to_bits(), std::sync::atomic::Ordering::Relaxed);
            // (the speech clock adds G+92, the frame's real time, not clamped - F-85023; a line holds while timer 0 is paused)
            self.speech.paused = self.game.mode == Mode::Paused && self.game.replay.is_none();
            let real_dt = if self.opt.fixed_dt > 0.0 || self.test_render_dt.is_some() { dt } else { self.frame_ms * 0.001 };
            self.speech.update(a, real_dt);
            self.radio.units = self.sound.radio_units();
            self.radio.update(a, dt);
            // pedestrians shouting as they jump clear (their speech banks looked up ahead)
            self.peds.scan_voices_ahead(a.root());
            for sp in std::mem::take(&mut self.peds.speech) {
                let root = a.root().to_path_buf();
                if let Some(bank) = self.peds.voice_bank(&root, &sp.voice, sp.female) {
                    let line = a.pick(&bank, if sp.fast { "ped_quick_evade" } else { "ped_slow_evade" }).or_else(|| a.pick(&bank, "ped_"));
                    if let Some(key) = line {
                        a.play_at(&key, sp.pos, 1.0, 9.0);
                    }
                }
            }
            // (PLAYER_NITROUS / PLAYER_IN_AIR: F-30989 requests, gated by the speech manager - `Speech::request`)
            if let Some(l) = self.sound.want_speech.take() {
                self.speech.request(l);
            }
            // the speech manager's "a line plays" test (+2256 and +2260, F-58518 / F-68821 and the police chirp): the
            // career speech only, the police radio has a manager of its own
            self.sound.speech_busy = self.speech.busy();
            // (nor on the start screen and the start-up movie before it: the career script makes the player's car only at
            // loadSuccess, after START - the port's car stands at the garage from the start)
            self.sound.no_player_car = self.game.fresh_game || self.game.starter_pick.is_some() || self.game.mode == Mode::MainMenu;
            if self.sound.prop_places.is_empty() && self.props.instances > 0 {
                self.sound.prop_places = self.props.audio_places().collect();
            }
            self.sound.warp = match self.startwarp.as_ref().map(|w| w.phase()) {
                None => 0,
                Some(startwarp::Phase::Climb) => 1,
                Some(startwarp::Phase::Hold) => 2,
                Some(startwarp::Phase::Down) => 3,
            };
            self.sound.damage_alpha = self.game.flash_hud.as_ref().and_then(|h| h.movie.player.get_global("damage_alpha")).map_or(0.0, |v| v.to_f32());
            self.sound.racer_found |= self.game.flash_hud.as_mut().is_some_and(|h| std::mem::take(&mut h.racer_found));
            self.sound.menu_cam = self.game.flash_menu.as_ref().filter(|_| self.game.in_menu()).and_then(|m| m.movie.player.get_global("cur_camera")).map(|v| v.to_f32() as i32);
            crate::ticktimes::mark(crate::ticktimes::S::Sound);
            self.sound.update(a, &self.vehicles, &self.game, &self.police, &self.ability, &self.props.hits, self.cam.pos, right, rain_snd, horn, self.time_of_day > 19.5 || self.time_of_day < 6.0, &|p: Vec3, q: Vec3| { let d = q - p; let l = d.length(); l > 0.0 && world.collider.raycast(p, d / l, l).is_some() }, dt);
            crate::ticktimes::mark(crate::ticktimes::S::Other);
            if let Some(v) = self.vehicles.player_mut().and_then(|c| c.state.veh.as_mut()) {
                (v.hyd.ev_rise, v.hyd.ev_drop, v.hyd.ev_end, v.hyd.ev_switch) = (0, 0, 0, 0);
            }
            // (test hook: MCLA_AUDIO_TRACE=1 logs once a second what sounds: the stream, the radio, the sound objects
            // and the mixer scenes, with the opening's step and the story scene)
            if std::env::var("MCLA_AUDIO_TRACE").is_ok() {
                self.audio_trace_t += dt;
                if self.audio_trace_t >= 1.0 {
                    self.audio_trace_t -= 1.0;
                    let cut = self.cuts.playing.as_ref().map(|p| format!("{} {:.1} s", p.def.name, p.t));
                    let lib = self.sound.lib.as_ref().map(|l| l.summary()).unwrap_or_default();
                    log::info!("audio trace: frame {} stage {} movie {} cut {:?} hold {} radio_held {} | stream {:?} radio {} speech {} | {}", self.frame_no, self.intro_stage, self.game.movie.is_some(), cut, self.cine_hold.is_some(), self.game.radio_held, a.stream_playing(), a.music_playing(), self.speech.busy(), lib);
                }
            }
            a.render(dt);
            // (the original's music lists, `mc4`: handed to the audio once; PLAY in the playlist
            // menu starts its song - the genre it brings is not a change to start another for)
            if a.lists.is_none() {
                if let Some(l) = self.game.mc4_music() {
                    if let Ok(mut m) = l.lock() {
                        m.bind_files(&a.music_assets());
                    }
                    a.lists = Some(l);
                }
            }
            if self.game.mc4_music_take_play() {
                a.playlist = self.game.music_genre.checked_sub(1);
                a.play_listed();
            }
            // (the pause menu's playlist: a change of genre starts a song of it)
            let want = self.game.music_genre.checked_sub(1);
            if a.playlist != want {
                a.playlist = want;
                a.next_track();
            }
            if self.game.songs.is_empty() {
                self.game.songs = a.songs();
            }
            // (the music manager's playing state, F-61726: the song line's clock while its song plays)
            if a.music_playing() && !a.track_changed {
                self.game.mc4_song_tick(dt, a.song_place);
            }
            if std::mem::take(&mut a.track_changed) && !a.now_playing.is_empty() {
                let t = a.now_playing.clone();
                // (the song line's statistics, F-73849: tMTC, tMTP of list x 256 + place)
                if let Some((g, k)) = a.song_place {
                    self.game.mc4_song_line(g as i32, k as i32);
                }
                // the HUD movie's bottom splash shows the song ("BAD RELIGION - 21st
                // Century Digital Boy" is its sample text)
                // (the original runs the song across the cluster's info line - references
                // `u02`, `u04`: "KILL MEMORY", "... ARTISTES (SWITCH REMIX)" - not as a splash)
                match self.game.flash_hud.as_ref().filter(|_| matches!(self.game.mode, Mode::FreeRoam | Mode::Race)) {
                    Some(_) => self.game.song_line = Some((t.to_uppercase(), 0.0)),
                    None => self.game.show_message(&t),
                }
            }

        }
        // ---- the garage's repair (`game.repairCar` = F-12181: the full reset F-74884 of the
        // model - zones, pieces, the map's dents AND scrapes - and the parts' repair F-75221; the car
        // is not moved) ----
        if std::mem::take(&mut self.game.repair_car) {
            if let Some(p) = self.vehicles.player_mut() {
                p.state.damage = 0.0;
                p.state.zones = [0.0; 4];
                p.state.dmg.reset();
            }
            // (F-12181 clears the six paint areas' unpainted flags; F-74884 kills the tyres' effects)
            self.game.mc4_set_unpainted([false; 6]);
            self.kill_player_tyre_fx();
        }
        // ---- the race-over menu's quick fix ----
        if std::mem::take(&mut self.game.quick_fix) {
            if let Some(a) = self.audio.as_mut() {
                a.obj("QUICK_FIX_MASTER");
            }
            // (the restore reads the zones before it resets them: a badly hit area may come back unpainted, F-39432)
            if let Some(z) = self.vehicles.player().map(|p| p.state.zones) {
                let mut rng = brain::rng();
                self.game.mc4_restore_primer(z, &mut || rng.next() & 0x7FFF_FFFF);
            }
            // (the restore's model reset F-11744 kills the tyres' effects, F-83592 per wheel)
            self.kill_player_tyre_fx();
            if let Some(p) = self.vehicles.player_mut() {
                // (the racer's restore after a repair, mcRacer#18 F-39432 - its vtable +72, called by F-32850 with 0 -
                // resets the damage through F-11744, whose map repair takes the dents and keeps the scrapes; F-27947
                // there is the chassis' reset of a car on its roof, not the map's)
                p.state.damage = 0.0;
                p.state.dmg.repair();
                p.state.zones = [0.0; 4];
                // (F-32850: set upright where it stands, its heading kept, then the racer's
                // reset F-39432(0))
                let f = p.state.forward();
                let f = Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z);
                let right = f.cross(Vec3::Y).normalize_or(Vec3::X);
                let pos = p.state.pos;
                p.state.relocate(pos, glam::Quat::from_mat3(&glam::Mat3::from_cols(right, Vec3::Y, -f)).normalize());
                p.state.vel = Vec3::ZERO;
                p.state.ang_vel = Vec3::ZERO;
                // (and the flags +592 set to 0, F-30391(0): no longer damaged out, the hold off; the engine word +46 = 1)
                p.stunned = 0.0;
                p.engine_off = false;
                p.deactivated = false;
                self.crash_bits = 0;
            }
            // (F-39432 also sets the engine sound running at once, vt+48 F-79364: state 4, no crank)
            self.sound.engine_set_running();
        }
        // ---- nitrous: none until the garage's nitrous part is fitted, then as many canisters
        // as the tuning counts (up to 4 fully fitted). They are filled when the part changes
        // and at the beginning and end of a race; a service station fills them all in cruise
        // and one when a race rolls through it (the game's own tutorial text) ----
        {
            let stage = self.game.perf_stages(self.game.current_car).get(6).copied().unwrap_or(0).min(2) as f32;
            let on_grid = self.game.race.as_ref().map_or(false, |r| r.countdown > 0.0);
            let mut refilled = false;
            let mut menu = false;
            if let Some(p) = self.vehicles.player_mut() {
                let (a, b) = p.model.simtunes.as_ref().map_or((1.0, 4.0), |s| (s.base.nitro_shots, s.mods.nitro_shots));
                let max = if stage > 0.0 { ((a + (b - a) * stage / 2.0) as u8).max(1) } else { 0 };
                let racing = self.game.race.is_some();
                let ended = self.was_racing && !racing;
                self.was_racing = racing;
                // (a circuit fills them again at the beginning of each lap)
                let lap = self.game.race.as_ref().filter(|r| r.laps > 1).map_or(0, |r| r.next.first().copied().unwrap_or(0) / r.lap_len.max(1));
                let new_lap = racing && lap > self.nitro_lap;
                self.nitro_lap = lap;
                if p.state.nitro_max != max || on_grid || ended || new_lap {
                    p.state.nitro_max = max;
                    p.state.nitro_shots = max;
                }
                // a gas station (`game/MiscTasks/GasStations`, career2 `gasphoto/notes.md`): rolling
                // through above 7.1 m/s fills one canister, once a visit, in any race too; stopping below
                // 7.0 in the cruise or a payback race with no police about opens the SERVICE STATION menu
                // (form 61), once a visit; the visit ends 10 m outside the station's radius
                if self.drive && p.state.damage < 1.0 {
                    let paused = !matches!(self.game.mode, Mode::FreeRoam | Mode::Race);
                    let police = matches!(self.police.heat, police::Heat::Pursuit | police::Heat::Search);
                    let state = self.game.race.as_ref().map_or(self.game.flag.is_none(), |r| r.kind == game::RaceKind::Payback);
                    let menu_open = self.game.hangout_key.as_deref() == Some(game::GAS_MENU);
                    match self.gas.step(dt, p.state.pos, p.state.vel.length(), menu_open, paused, police, state) {
                        mc4b::GasStep::RefillOne => {
                            // (mcCareer_RefillOneNitro, F-21838: one more while below the most)
                            if p.state.nitro_shots < p.state.nitro_max {
                                p.state.nitro_shots += 1;
                                refilled = true;
                            }
                        }
                        mc4b::GasStep::Menu => menu = true,
                        mc4b::GasStep::None => {}
                    }
                    self.at_station = self.gas.at;
                }
            }
            if refilled {
                if let Some(a) = self.audio.as_mut() {
                    a.obj("NITROUS_REFILL_MASTER");
                }
            }
            if menu {
                let nitro = self.vehicles.player().map_or(false, |c| c.state.nitro_max > 0);
                self.game.gas_menu_open(nitro);
                self.pending_action = MenuAction::OpenRaceSelect;
            }
            if let Some(n) = self.game.opt_x.s23b.ui_obj.take() {
                if let Some(a) = self.audio.as_mut() {
                    a.obj(n);
                }
            }
        }
        // ---- cheats (earned with the collectables) ----
        if self.game.cheats.iter().any(|c| *c) {
            let c = self.game.cheats;
            if let Some(p) = self.vehicles.player_mut() {
                if c[0] {
                    p.state.nitro_shots = p.state.nitro_max.max(1);
                }
                if c[1] {
                    p.state.damage = p.state.damage.min(0.9);
                }
            }
            if c[2] {
                self.ability.meter = 1.0;
            }
            if c[5] {
                self.police.fine = 0.0;
            }
        }
        // (and none before the career has unlocked them: `mc4`, flags.police_unlocked)
        // (and none in a red light race: its script destroys them all - althk.rs)
        let police_on = !self.game.cheats[4] && self.game.mc4_police_unlocked() && !self.game.alt_police_off();
        // (the player-load flow script's CreatePolice x 3, units 400 - 402, each sent underground: the free roam's police)
        if police_on && self.police.units.is_empty() {
            self.police.create_units(true);
        }
        if self.police.enabled != police_on {
            log::info!("police: {}", if police_on { "on" } else { "off" });
            self.police.enabled = police_on;
            if !self.police.enabled {
                if self.game.alt_police_off() {
                    self.police.destroy_all(&mut self.vehicles);
                } else {
                    self.police.reset(&mut self.vehicles);
                }
            }
        }
        // (the three cruisers of the hookmen's scripts: made by the freeway men's scripts when they
        // start, after a red light race brought up and made again - althk.rs, police.rs)
        if !self.alt_units_made && self.roam.men.iter().any(|m| m.kind == roamers::ManKind::Freeway) {
            self.alt_units_made = true;
            self.police.create_units(self.game.mc4_police_unlocked());
        }
        if std::mem::take(&mut self.game.althk.police_after) {
            self.police.units_up();
            self.police.create_units(self.game.mc4_police_unlocked());
        }

        self.vehicles.finish_tick(sim_dt);
        self.input.pending_keys.clear();
        self.world = Some(world);
        self.gpu = Some(gpu);
        let action = std::mem::replace(&mut self.pending_action, MenuAction::None);
        self.handle_action(action);
    }

    fn render(&mut self) {
        let Some(window) = self.window.clone() else { return };
        if self.gpu.is_none() || self.world.is_none() {
            return;
        }
        let now = Instant::now();
        let wall_seconds = (now - self.last).as_secs_f64();
        let wall_dt = wall_seconds as f32;
        let render_seconds = self.test_render_dt.unwrap_or(wall_seconds.min(0.1));
        let dt = if self.opt.fixed_dt > 0.0 { self.opt.fixed_dt } else { render_seconds as f32 };
        self.last = now;
        // --fast fixes simulation time, not the measured rendering throughput.
        self.fps = self.fps * 0.95 + (1.0 / wall_dt.max(1e-4)) * 0.05;
        self.frame_ms = wall_dt * 1000.0;
        // (MCLA_FRAMELOG=<ms>: a frame whose whole interval passed the limit, with the phases of the frame
        // that took it - the interval of frame N is known when frame N+1 starts)
        if let Some(limit) = self.framelog {
            if self.frame_ms > limit && self.frame_no > 0 {
                let p = self.fl_prev;
                let u = self.fl_ui;
                log::info!("slowframe: #{} {:.1} ms (limit {:.0}) | ticks {:.0} sim {:.2} (streaming {:.2}) prep {:.2} acquire {:.2} ui {:.2} (run {:.2} tessellate {:.2} textures {:.2} movies {:.2} buffers {:.2}) passes {:.2} submit+present {:.2} | loaded {} pending {} draws {}",
                    self.frame_no, self.frame_ms, limit, p[0], p[1], p[2], p[3], p[4], p[5], u[0], u[1], u[2], u[3], u[4], p[6], p[7], p[8] as i64, self.world.as_ref().map_or(0, |w| w.pending.len()), self.draws);
            }
        }
        self.fl_stream_ms = 0.0;
        self.frame_no += 1;
        // (start-up timing: the long frames of the first minute - docs/reference/ui/boot.md)
        if self.frame_no < 3600 && wall_dt > 0.05 {
            log::info!("startup: frame {} took {:.0} ms", self.frame_no, wall_dt * 1000.0);
        }
        self.screen_v = (self.cam.pos - self.screen_eye) / dt.max(1e-4);
        self.screen_eye = self.cam.pos;

        self.gamepad_buttons();
        // (what the prompts' keyboard keys are on this screen, and the GPS map's key - `glyphs`)
        crate::glyphs::set_screen(if self.photo.is_some() {
            crate::glyphs::Screen::Photo
        } else if self.game.show_map {
            crate::glyphs::Screen::Map
        } else if self.game.replay.is_some() {
            crate::glyphs::Screen::Replay
        } else {
            crate::glyphs::Screen::Menu
        });
        crate::glyphs::set_gps_key(self.game.opt_x.ctrl.kbd.binds.keys[keybinds::GPS_MAP][0].and_then(keybinds::key_name).unwrap_or("M"));
        // Do not spend the launch/countdown clock before the initial physics world
        // exists. Sector decode completion is asynchronous and can take different
        // numbers of render frames. UI/rendering continue while it loads.
        // the start screen's cameras (titlecam.rs, the press-start script): the shot up, its hour (the clock's
        // update off meanwhile) and its weather (snapped); after the start screen the hour before it
        let title_mask;
        {
            let attract = self.game.movie.as_ref().filter(|m| m.any_key).map(|m| (m.time(), m.duration()));
            // (the climb after START is still the press-start script's: its hour and streaming - startwarp.rs)
            let climbing = self.startwarp.as_ref().map_or(false, |w| w.climbing());
            if let Some(a) = self.titlecam.frame(self.game.mode == Mode::MainMenu || climbing, attract, self.time_of_day) {
                self.title_weather(a);
            }
            if let Some(i) = self.titlecam.shot {
                self.time_of_day = titlecam::SHOTS[i].hour;
            }
            if let Some(h) = self.titlecam.take_saved_hour() {
                self.time_of_day = h;
            }
            // (the black around the attract movie: 1 s down, the movie, then up after the next shot is
            // streamed - titlecam::attract_fade; over the garage entry's own mask)
            let streamed = self.world.as_ref().map_or(true, |w| w.streaming() == 0);
            title_mask = self.titlecam.attract_fade(dt, &mut self.idle, boot::ATTRACT_IDLE, self.game.movie.is_some(), streamed);
        }
        let loading = if self.tick_no == 0 && self.drive {
            if let Some(car) = self.vehicles.player() {
                let pos = car.state.pos;
                let world = self.world.as_mut().unwrap();
                if !world.ready_at(pos) { world.update(self.gpu.as_mut().unwrap(), pos, 6.0); }
                !world.ready_at(pos)
            } else { false }
        } else { false };
        let ticks = if loading { self.sim_clock.reset(); 0 }
            else if self.opt.fixed_dt > 0.0 { 1 }
            else { self.sim_clock.advance(render_seconds) };
        let sim_t0 = Instant::now();
        if let Some(w) = self.world.as_mut() {
            w.new_frame(6.0);
        }
        for _ in 0..ticks {
            crate::ticktimes::start();
            self.update_gameplay(fixed_step::DT);
            crate::ticktimes::end(self.vehicles.level.objs.len());
        }
        // ---- the pad's rumble to the pad (crate::rumble): game.SetForceFeedback as the game state has it
        // (GameState.sc.xml: off in the hangout and the garage, on for its test drive), the stops while the
        // game's tick is held (the pause menu F-54667 / F-77444, recorded; the port's other holds alike) ----
        if let Some(r) = self.pad_rumble.as_mut() {
            let ff = !(self.game.mode == Mode::Garage && self.game.test_drive.is_none()) && self.hangout_in.is_none();
            let held = self.game.mode == Mode::Paused || self.cuts.playing.is_some() || self.game.movie.is_some() || self.game.show_map || self.game.tutorial_hold() || self.photo.is_some() || self.game.in_menu();
            let vibration = self.game.mc4.as_ref().map_or(true, |m| m.options.vibration);
            let pad = self.gamepad.as_ref().and_then(|g| active_pad(g, self.pad_id));
            r.frame(pad, ff, vibration, held);
        }
        let sim_ms = sim_t0.elapsed().as_secs_f32() * 1000.0;
        // (the start screen's black around the attract movie, over the garage entry's own mask - set by the ticks)
        self.game.title_mask = title_mask;
        if self.frame_no < 3600 && sim_ms > 50.0 {
            log::info!("startup: frame {} - its {} game ticks took {:.0} ms", self.frame_no, ticks, sim_ms);
        }
        self.vehicles.render_alpha = if self.opt.fixed_dt > 0.0 { 1.0 } else { self.sim_clock.alpha() };
        let world = self.world.take().unwrap();
        let mut gpu = self.gpu.take().unwrap();
        // The cars' lamps. Under the weather's keys they are on as the game has them: while
        // m_NFLFade is above zero (18:00 to 06:00 in every weather's file - recorded: on at
        // 05:45 and 18:15, off at 06:15 and 17:45) and all day long in rain and fog (recorded on
        // at noon in Stormy and Foggy, off in Cloudy).
        let night = match self.sky_now().tod {
            // (the garage's own keys hold m_NFLFade 1, but the car stands there with its lamps off - the
            // original's garage screens, work/lighting/cap/gar_*.png)
            Some(_) if self.garage_light().is_some() => 0.0,
            Some(t) => if t.nfl > 0.0 || matches!(self.weather, sky::Weather::Stormy | sky::Weather::Foggy) { 1.0 } else { 0.0 },
            None => if self.time_of_day > 18.5 || self.time_of_day < 5.5 { 1.0 } else { 0.0 },
        };
        let sim_dt = if self.game.mode == Mode::Paused || self.cuts.playing.is_some() || self.game.movie.is_some() || self.game.show_map || self.game.tutorial_hold() || self.photo.is_some() { 0.0 } else { dt * self.wipe_scale.min(self.ability.time_scale()).max(0.1) * self.game.mc4_time_scale() };
        if let (Some(path), Some(cfg)) = (self.opt.audio_dump.as_ref(), self.opt.shot.as_ref()) {
            if self.frame_no == cfg.frames {
                if let Some(a) = self.audio.as_mut() { a.write_capture(path); }
            }
        }
        if self.drive {
            // the plate the player wrote in the garage (LICENSE PLATES) is the car's
            // (step 19e: a borrowed vehicle in a set-up shows the set-up's plate)
            if let Some(t) = self.game.player_plate().or_else(|| self.game.mc4_borrowed_plate()) {
                if let Some(c) = self.vehicles.player_mut() {
                    if c.plate_text != t {
                        c.plate_text = t;
                        c.plate = None;
                    }
                }
            }
            // (and its background is the one the vehicle's record names: `mc4`)
            let bg = self.game.mc4_plate_bg_name();
            if bg != self.vehicles.player_plate_bg {
                log::info!("plates: the player's background is {:?}", bg);
                self.vehicles.player_plate_bg = bg;
                if let Some(c) = self.vehicles.player_mut() {
                    c.plate = None;
                }
            }
            // (the damage callback's dv since the last frame, for the impact shake)
            let shake_dv = self.vehicles.player_mut().map_or(0.0, |c| std::mem::take(&mut c.state.shake_dv));
            if let Some(car) = self.vehicles.player() {
                let render_state = car.render_state(self.vehicles.render_alpha);
                // the level the cameras' probes see (physics/camprobe.rs): the city and every other
                // placed car's hull where it is drawn; the camera's own car is not in it
                let cam_level = physics::camprobe::CamLevel {
                    city: &world.collider,
                    cars: self
                        .vehicles
                        .cars
                        .iter()
                        .filter(|c| c.placed && !c.is_player && !c.pu_ghost)
                        .filter_map(|c| {
                            let h = c.tuned.as_deref().unwrap_or(&c.model.params).hull.as_ref()?;
                            let s = c.render_state(self.vehicles.render_alpha);
                            Some(physics::camprobe::CamObj::of_hull(h, s.pos, s.rot))
                        })
                        .collect(),
                };
                // the garage's own cameras: the view that goes with the highlighted row
                let mut own_view = false;
                // (the race's end before its menu: the driving camera, swung to the standings form's)
                // (and a beat me there run's end in free roam: the same EndOfRaceSequence - `Game::bmt_end`)
                let bmt_run_end = self.game.mode == Mode::FreeRoam && self.game.bmt_end.as_ref().map_or(false, |e| !e.wrecked);
                let race_end_on = (self.game.mode == Mode::Results || bmt_run_end) && self.game.replay.is_none() && self.cuts.playing.is_none() && self.game.race_end_t.is_some();
                let race_end_swing = race_end_on && self.game.race_end_t.map_or(false, |t| t < game::Game::RACE_END_MENU);
                if !race_end_on {
                    self.race_end_cam = None;
                    self.race_end_from = None;
                    self.race_end_way = None;
                }
                if self.game.mode == Mode::Garage && self.cuts.playing.is_none() && std::env::var("MCLA_NO_GARAGE_CAM").is_err() {
                    if self.garage_views.as_ref().map_or(true, |v| v.0 != car.model.name) {
                        let v = garagecam::Views::load(&self.opt.data_dir, &car.model.name);
                        // (a mod car without its own lines: its manifest's `donors.garage` - modcars.rs)
                        let v = v.or_else(|| crate::modcars::car(&car.model.name).and_then(|c| c.garage.as_deref()).and_then(|g| garagecam::Views::load(&self.opt.data_dir, g)));
                        log::info!("garage: {} camera views for {}", v.as_ref().map_or(0, |v| v.len()), car.model.name);
                        self.garage_views = Some((car.model.name.clone(), v.unwrap_or_default()));
                        self.game.dyno_rect = self.garage_views.as_ref().map(|v| v.1.dyno).filter(|d| d[2] > 0.0);
                    }
                    let name = self.game.garage_view();
                    if std::env::var("MCLA_MENU3D_LOG").is_ok() && self.frame_no % 120 == 0 {
                        log::info!("menu3d: garage view {} (car {} selected {} current {})", name, car.model.name, self.game.selected_car, self.game.current_car);
                    }
                    if let Some((_, views)) = self.garage_views.as_ref() {
                        if let Some(view) = views.get(name).or_else(|| views.get("Rotate")) {
                            // the turntable view drifts slowly round the car
                            self.orbit += dt * 0.12;
                            let turn = if name == "Rotate" { self.orbit } else { 0.0 };
                            // with the garage movie the views are the game's own, untouched, and the
                            // menu panel hangs where the view says (see menu3d)
                            let original = self.game.flash_garage.is_some() && std::env::var("MCLA_FLAT_MENU").is_err();
                            let turn = if original { 0.0 } else { turn };
                            // (approx.) where the views' frame sits in the car: test hook MCLA_GARAGE_OFF=x,y,z
                            let off: Vec<f32> = std::env::var("MCLA_GARAGE_OFF").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                            let origin = render_state.pos + render_state.rot * if off.len() == 3 && original { Vec3::new(off[0], off[1], off[2]) } else { Vec3::ZERO };
                            let (mut pos, mut look, fov) = view.place(origin, render_state.rot, turn);
                            if original && std::env::var("MCLA_GARAGE_ORBIT").map_or(false, |v| v == "origin") {
                                // test hook: orbit the car's origin (at the view's height) instead of the interest point
                                let centre = origin + render_state.rot * Vec3::new(0.0, view.height, 0.0);
                                pos = centre + (pos - look);
                            }
                            self.game.garage_panel = original.then(|| (view.menu_rot, view.menu_offset, view.menu_scale, glam::Mat4::from_rotation_translation(render_state.rot, origin) * garagecam::file_frame()));
                            // the menu covers the left of the screen (the original's small menus float
                            // beside the car): show the car in what is left, a little wider
                            let d = look - pos;
                            let far = d.length() > 4.0;
                            // (approx.) The views' `FOV` comes out as 4/3 of itself on the 16:9 screen
                            // (measured on the reference captures; the camera's use of the figure,
                            // +168 of the camera object, was not read).
                            // (the flat pages - paint shop, hydraulics, vinyl editor - show the figure as it
                            // is: references `t33`, `t45`, the car a good third larger than 4/3 would make it)
                            // (measured again in step 32 on eight area views of the recomp: the file's FOV
                            // as it is fits the top menu, the side and wheel views; 4/3 made every car
                            // about a quarter too small. On settled captures the cars still came out a tenth
                            // too large, hence 1.1 (approx.). MCLA_GARAGE_FOV tries another factor.)
                            // (step 34: with the camera placed as the running game's own - eye within a
                            // centimetre of it - the file's FOV as it stands reproduces the captures; the
                            // tenth was the old placement's error)
                            let fov_k: f32 = std::env::var("MCLA_GARAGE_FOV").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
                            let fov = fov * if original && (matches!(self.game.garage_menu, 3 | 13 | 15) || self.game.vinyl_edit.is_some() || self.game.cart_open.is_some()) { 1.0 } else if original { fov_k } else { 1.3 };
                            // turn the camera so that what the view looks at stands at two thirds of
                            // the screen's width
                            let aspect = gpu.config.width as f32 / gpu.config.height.max(1) as f32;
                            let ang = (if far { 0.36 } else { 0.5 } * (fov * 0.5).tan() * aspect).atan();
                            if !original {
                                look = pos + glam::Quat::from_rotation_y(ang) * d;
                            }
                            // glide from view to view
                            let k = 1.0 - (-dt * 3.2 / views.transition.max(0.1)).exp();
                            let cur = self.garage_cam.get_or_insert((pos, look, fov));
                            if original {
                                // the game's own move (F-53958): the camera's figures eased from the
                                // view left to the view entered, the azimuth the short way round
                                let to = garagecam::CamState::of(&view, origin, render_state.rot, turn);
                                let tr = self.garage_tr.get_or_insert((to, to, 1.0e3));
                                let same = (tr.1.target - to.target).length() < 1e-3 && (tr.1.dist - to.dist).abs() < 1e-4 && (tr.1.azimuth - to.azimuth).abs() < 1e-4 && (tr.1.height - to.height).abs() < 1e-4;
                                if !same {
                                    let now = tr.0.eased(&tr.1, tr.2 / views.transition.max(0.01));
                                    *tr = (now, to, 0.0);
                                }
                                tr.2 += dt;
                                let (e, l, up, f) = tr.0.eased(&tr.1, tr.2 / views.transition.max(0.01)).place();
                                *cur = (e, l, f * if fov > 0.0 { fov / view.fov.to_radians().max(1e-3) } else { 1.0 });
                                self.garage_up = up;
                            } else {
                                cur.0 = cur.0.lerp(pos, k);
                                cur.1 = cur.1.lerp(look, k);
                                cur.2 += (fov - cur.2) * k;
                                self.garage_up = Vec3::Y;
                            }
                            self.chase.pos = cur.0;
                            self.chase.look = cur.1;
                            if std::env::var("MCLA_MENU3D_LOG").is_ok() && self.frame_no % 120 == 0 {
                                let inv = render_state.rot.inverse();
                                log::info!("menu3d: view {:?} eye(local) {:?} look(local) {:?} fov {:.1}", view, inv * (cur.0 - render_state.pos), inv * (cur.1 - render_state.pos), cur.2.to_degrees());
                                log::info!("menu3d: car at {:?} x axis {:?} eye {:?}", render_state.pos, render_state.rot * Vec3::X, cur.0);
                            }
                            self.cam.fov_y = cur.2;
                            own_view = true;
                        }
                    }
                } else {
                    self.garage_cam = None;
                    self.garage_tr = None;
                }
                // the pause menu's camera: the movie picks it from the tuned pause cameras
                // (`UILogic` PauseCamera<n>, see menu3d) in the car's frame - x right, y up,
                // z rear, level with the ground. (approx.) The frame's height above the
                // car's origin is set by eye against the reference capture.
                // the replay's own cameras (not under its PAUSE panel)
                // (the original's director, replay::Director: its own cameras here, a drive view
                // is left to the boom cameras below)
                let aspect = gpu.config.width as f32 / gpu.config.height.max(1) as f32;
                let car_view = replay::CarView::of(&render_state, car.model.max.y);
                let mut probe = ReplayProbe(&cam_level);
                if let Some(p) = self.game.replay.as_mut() {
                    // (the right stick turns the polar camera, F-79918; the triggers' zoom is not wired)
                    p.stick(self.pad_look.0, self.pad_look.1, 0.0, dt);
                }
                let was = self.game.replay.as_ref().and_then(|p| p.view());
                // (test hook MCLA_REPLAY_SHOTS=<prefix>,t1,t2,..: a screenshot <prefix>_<t>.png as the
                // replay's clock passes each t)
                // (o<secs>: seconds since VIEW REPLAY, the opening included)
                if let (Ok(v), Some(p)) = (std::env::var("MCLA_REPLAY_SHOTS"), self.game.replay.as_ref()) {
                    let wall = self.replay_wall + dt;
                    let mut it = v.split(',');
                    let prefix = it.next().unwrap_or("replay").to_string();
                    for x in it {
                        let (o, at) = match x.strip_prefix('o') { Some(r) => (true, r.parse::<f32>().unwrap_or(f32::MAX)), None => (false, x.parse::<f32>().unwrap_or(f32::MAX)) };
                        let (was, now) = if o { (self.replay_wall, wall) } else { (self.replay_shot_t, p.t) };
                        if was < at && now >= at {
                            self.photo_request = Some(format!("{prefix}_{x}.png"));
                        }
                    }
                    if std::env::var("MCLA_REPLAY_DEBUG").is_ok() && self.frame_no % 15 == 0 {
                        log::info!("replay clock: wall {:.2} t {:.2} opening {} warp {} scene {:?} countdown {:.2}", wall, p.t, p.opening, self.warp.is_some(), self.cine.playing.as_ref().map(|sc| (sc.name.clone(), sc.t)), self.game.race.as_ref().map_or(-1.0, |r| r.countdown));
                    }
                    self.replay_shot_t = p.t;
                    self.replay_wall = wall;
                }
                let shot = self.game.replay.as_mut().filter(|p| p.menu.is_none()).and_then(|p| p.camera(&car_view, aspect, &mut probe));
                if std::env::var("MCLA_REPLAY_DEBUG").is_ok() {
                    let now = self.game.replay.as_ref().and_then(|p| p.view());
                    if now != was || self.frame_no % 30 == 0 {
                        log::info!("replay director: state {:?} -> {:?} at t {:.2} left {:.2} dt {:.4}", was, now, self.game.replay.as_ref().map_or(0.0, |p| p.t), self.game.replay.as_ref().and_then(|p| p.director.as_ref()).map_or(0.0, |d| d.time_left()), dt);
                    }
                }
                let replay_cut = self.game.replay.as_ref().and_then(|p| p.director.as_ref()).map_or(false, |d| d.cut);
                let replay_drive = match shot {
                    Some(replay::Shot::Drive { view, back }) => Some((view, back)),
                    _ => None,
                };
                let replay_view = shot.and_then(|s| s.view());
                // (eye, look, fov) and the camera's own up
                let (replay_view, replay_up) = match replay_view {
                    Some((eye, look, up, fov)) => (Some((eye, look, fov)), Some(up)),
                    None => (None, None),
                };
                // (the race's opening shot from the air)
                // (CarSelection's camera: Game_SetCamera(eye, look, 25.5) - `sel_cam`, stepped in `opening_tick`)
                let sel_view = self.sel_cam.filter(|_| self.cuts.playing.is_none()).map(|(eye, look, ..)| (eye, look, 25.5));
                let replay_view = sel_view.or(replay_view);
                let replay_up = replay_up.filter(|_| sel_view.is_none());
                // (the race-start warp: hudmap::CineWarp, roll included)
                let replay_view = match self.warp_view.filter(|_| self.game.race.is_some()).or(self.place_view) {
                    Some((eye, look, _, fov)) => Some((eye, look, fov.to_degrees())),
                    None => replay_view,
                };
                // (the start screen's still camera: Game_SetCamera(eye, eye + dir, lens, 0, 5, 10000) - titlecam.rs)
                let title_view = self.titlecam.view().filter(|_| self.game.mode == Mode::MainMenu && self.cuts.playing.is_none());
                let replay_view = title_view.or(replay_view);
                if let Some((eye, look, fov)) = replay_view {
                    self.chase.pos = eye;
                    self.chase.look = look;
                    self.cam.fov_y = fov.to_radians();
                    own_view = true;
                } else if (matches!(self.game.mode, Mode::Paused) || (self.game.mode == Mode::Results && !race_end_swing) || (self.game.mode == Mode::FreeRoam && self.game.bmt_end.as_ref().map_or(false, |e| e.wrecked)) || (self.game.mode == Mode::RaceSelect && self.game.hangout_key.as_deref().map_or(false, |k| k.contains("delivery")))) && self.cuts.playing.is_none() {
                    if let Some((eye, look, fov)) = self.game.pause_cam {
                        let fwd = render_state.forward();
                        let fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
                        let right = Vec3::new(-fwd.z, 0.0, fwd.x);
                        let off: Vec<f32> = std::env::var("MCLA_PAUSE_OFF").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                        let off = if off.len() == 3 { Vec3::new(off[0], off[1], off[2]) } else { PAUSE_FRAME };
                        let origin = render_state.pos + right * off.x + Vec3::Y * off.y - fwd * off.z;
                        let place = |p: Vec3| origin + right * p.x + Vec3::Y * p.y - fwd * p.z;
                        self.chase.pos = place(eye);
                        self.chase.look = place(look);
                        self.cam.fov_y = fov.to_radians();
                        own_view = true;
                    }
                }
                let mut boom_view = false;
                self.lod_near = if title_view.is_some() { titlecam::NEAR } else { self.cam.near };
                self.boom_up = None;
                if own_view {
                    // the garage views' roll (`<View>Roll`): the camera's up turned about its line of sight
                    if self.game.mode == Mode::Garage {
                        self.boom_up = Some(self.garage_up);
                    }
                    if let Some(up) = replay_up {
                        self.boom_up = Some(up);
                    }
                    if let Some((_, _, up, _)) = self.warp_view.filter(|_| self.game.race.is_some()).or(self.place_view) {
                        self.boom_up = Some(up);
                    }
                } else if self.game.mode == Mode::Garage || (self.cuts.playing.is_some() && !self.cine.at_game_camera()) {
                    // showroom orbit: 3/4 view slowly circling the car
                    self.orbit += dt * 0.25;
                    let a = self.orbit + 0.6;
                    let fwd = render_state.forward();
                    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
                    let dir = fwd * a.cos() + right * a.sin();
                    let ext = (car.model.max - car.model.min).length().max(4.0);
                    let mut dist = (ext * 1.65).max(8.0);
                    let mut up = (0.34 * ext).max(1.6);
                    self.cam.fov_y = 50f32.to_radians();
                    if self.garage_clear > 0.0 {
                        // indoors: stay between the car and the walls
                        dist = dist.min(self.garage_clear - 0.5).max(4.2);
                        up = up.min(1.7);
                        self.cam.fov_y = 62f32.to_radians();
                    }
                    self.chase.pos = render_state.pos + dir * dist + Vec3::Y * up;
                    self.chase.look = render_state.pos + Vec3::Y * (0.13 * ext).max(0.6);
                    if self.game.mode == Mode::Garage {
                        // the menu covers the left of the screen: show the car beside it
                        let view = (self.chase.look - self.chase.pos).normalize_or(Vec3::Z);
                        let side = view.cross(Vec3::Y).normalize_or(Vec3::X) * dist * 0.2;
                        self.chase.pos -= side;
                        self.chase.look -= side;
                    }
                } else {
                    // the car's own cameras (boom1..3 behind it, POV at the driver's eyes, bumper)
                    let data_dir = self.opt.data_dir.clone();
                    let set = self
                        .booms
                        .entry(car.model.name.clone())
                        .or_insert_with_key(|name| {
                            boomcam::BoomSet::load(&data_dir, name).or_else(|| boomcam::BoomSet::load(&data_dir, if name.contains("_duc_") || name.contains("_kaw_") { "vp_duc_999r_06" } else { "vp_vw_golf_83" })).map(Arc::new)
                        })
                        .clone();
                    let nitro = render_state.nitro_on();
                    // (the replay director's drive view: its view and the reverse camera; its
                    // changes are cuts, F-53309)
                    let mode = replay_drive.map_or(self.cam_mode, |(v, _)| boomcam::PORT_OF_VIEW[(v as usize).min(4)]);
                    self.view_mode = mode;
                    if replay_cut {
                        self.boom.reset();
                        self.cam_switch.reset();
                        // (a cut to a fixed mount - bumper or cockpit, camBoomCS vt+60 - puts the
                        // TRANSITIONMOVIE mask up, F-53309 -> F-28091)
                        if replay_drive.map_or(false, |(v, _)| v < 2) {
                            self.cam_switch.mask.start(0.01, 0.25, 0.01);
                        }
                    }
                    let cine_views = if mode == 5 {
                        self.event_views
                            .entry(car.model.name.clone())
                            .or_insert_with_key(|name| boomcam::EventViews::load(&data_dir, name).or_else(|| boomcam::EventViews::load(&data_dir, "vp_vw_golf_83")).map(Arc::new))
                            .clone()
                    } else {
                        None
                    };
                    if let Some(views) = cine_views {
                        // the event that rules now
                        let vs = render_state.veh.as_ref();
                        let event = if vs.map_or(false, |v| v.burn_state == 1 || v.burn_state == 2) {
                            "Burnout"
                        } else if vs.map_or(false, |v| v.wheelie == 1) {
                            "Wheelie"
                        } else if render_state.airborne_time > 0.35 {
                            "Air"
                        } else if vs.map_or(nitro, |v| v.nitro_level > 0.0) {
                            "Nitrous"
                        } else if render_state.gear < 0 && render_state.fwd_speed() < -2.0 {
                            "Reverse"
                        } else if vs.map_or(false, |v| v.hyd.on) {
                            "Hydraulics"
                        } else {
                            ""
                        };
                        let top = car.model.params.veh.as_ref().map_or(60.0, |v| v.top_speed.max(20.0));
                        let out = self.event_cam.update(&views, event, render_state.pos, render_state.forward(), render_state.speed() / top, dt);
                        self.chase.pos = out.pos;
                        self.chase.look = out.look;
                        self.boom_up = Some(out.up);
                        self.cam.fov_y = out.fov.to_radians();
                        boom_view = true;
                        self.boom.reset();
                    } else if let Some(set) = set {
                        self.event_cam.reset();
                        let view = (mode as usize).min(set.views.len() - 1);
                        // (C looks behind)
                        self.look_back = match replay_drive {
                            Some((_, back)) => back,
                            None => (self.game.opt_x.ctrl.kbd.binds.down(crate::keybinds::LOOKBACK, |k| self.input.down(k)) || self.cam_mode == 9 || self.pad_look.1 < -0.6) && matches!(self.game.mode, Mode::FreeRoam | Mode::Race),
                        };
                        let (tune, rev) = boomcam::tune_for(&set, mode, self.look_back);
                        // (a change of view blends or cuts, a look back cuts - mcPlayerCamera; the
                        // CINEMATIC CAMERA option turns the look back of a moving car to one side)
                        let cinematic = self.game.mc4.as_ref().map_or(true, |m| m.options.idle_camera);
                        self.cam_switch.frame(&set, mode, self.look_back, cinematic, render_state.speed() > 0.0, &mut self.boom, dt);
                        // the look-around (F-36289): the stick turns the view camera, CAMERA_UP
                        // lifts it; looking back from a chase view is the reverse camera's own turn
                        let chase = boomcam::VIEW_OF_PORT[(mode as usize).min(5)] >= 2;
                        // the look-back slow motion (F-38892, after the reverse camera's turn): while the
                        // reverse camera shows, the car over 0.25 s in the air, the CINEMATIC CAMERA
                        // option on, the camera turned and the line 2 s along the velocity (falling 5 m)
                        // clear, "vehicle\lookback" starts every frame and the LOOKBACK scene with it;
                        // landed (air <= 0) both are released every frame; the look back's end releases
                        // them too (F-39925). Not in a replay (Open: whether its camera can be the reverse one).
                        let scale_now = self.wipe_scale.min(self.ability.time_scale());
                        if self.cam_switch.look_off {
                            self.warp_look.release(scale_now);
                            lookback_scene(&mut self.look_scene, self.audio.as_mut(), false);
                        }
                        if self.look_back && chase && replay_drive.is_none() && self.crash_bits == 0 && !self.crash_cam.active() {
                            let air = render_state.airborne_time;
                            if air > 0.25 {
                                if cinematic && self.cam_switch.pan != (0.0, 0.0) {
                                    let a = render_state.pos + render_state.rot * Vec3::Y * 0.5;
                                    let b = a + (render_state.vel - Vec3::new(0.0, 2.5, 0.0)) * 2.0;
                                    if !cam_level.line(a, b) {
                                        self.warp_look.start(scale_now, dt);
                                        lookback_scene(&mut self.look_scene, self.audio.as_mut(), true);
                                    }
                                }
                            } else if air <= 0.0 {
                                self.warp_look.release(scale_now);
                                lookback_scene(&mut self.look_scene, self.audio.as_mut(), false);
                            }
                        }
                        let fx = self.cam_fx.get_or_insert_with(|| boomcam::CamFx::load(&data_dir)).clone();
                        let input = self.drive && !self.game.in_menu() && self.game.replay.is_none() && matches!(self.game.mode, Mode::FreeRoam | Mode::Race);
                        let (side, up) = if self.look_back && chase { (0.0, 0.0) } else { (self.pad_look.0, self.pad_look_up) };
                        // (the cockpit's parked free look: no pedal and the car still for over
                        // LookaroundDelayTime - the pad's idle time and chassis +588 - then the left
                        // stick, raw, its up negative)
                        let still = render_state.veh.as_ref().map_or(0.0, |v| v.still_timer);
                        let free = (self.idle_time > fx.look_delay && still > fx.look_delay)
                            .then(|| self.gamepad.as_ref().and_then(|g| active_pad(g, self.pad_id)).map(|p| (p.axis(pads::Axis::LeftX), -p.axis(pads::Axis::LeftY))))
                            .flatten();
                        self.look.frame(&fx, boomcam::VIEW_OF_PORT[(mode as usize).min(5)] as u8, car.model.bike, side, up, input, self.look_back && !chase, free, dt);
                        let target = boomcam::BoomTarget {
                            pos: render_state.pos,
                            rot: render_state.rot,
                            vel: render_state.vel,
                            ang_vel: render_state.ang_vel,
                            grounded: render_state.wheels.iter().filter(|w| w.grounded).count(),
                            gear: render_state.gear,
                            rev,
                            nitro: render_state.veh.as_ref().map_or(nitro, |v| v.nitro_level > 0.0),
                            nitro_fov: match car.model.simtunes.as_ref() {
                                Some(s) => (s.base.nitro.fov, s.base.nitro.fov_out_time, s.base.nitro.fov_in_time),
                                None => (13.0, 0.5, 1.0),
                            },
                            ability: if chase { (0.0, 1.0) } else { self.ability.camera() },
                            sst: render_state.veh.as_ref().map_or((0.0, 1.0), |v| (v.sst.fov, v.sst.dist)),
                            pan: if self.look_back && chase { self.cam_switch.pan_for(mode, true) } else { (self.look.yaw, self.look.pitch) },
                            rise: if self.look_back && chase { 0.0 } else { self.look.rise },
                            fixed_mount: !chase,
                            model_offset: car.model.simtunes.as_ref().map_or(Vec3::ZERO, |s| Vec3::new(s.base.s.model_offset_x, s.base.s.model_offset_y, s.base.s.model_offset_z)),
                        };
                        self.lod_near = tune.near();
                        let out = self.boom.update(tune, &target, dt);
                        let out = self.cam_switch.blend.apply(&boomcam::BoomTarget { pan: (0.0, 0.0), ..target }, out, dt);
                        // the burnout camera: with the CINEMATIC CAMERA option on and a chase view in
                        // use, a burnout spun on the handbrake brings in the event camera's Burnout
                        // view (F-24075; boomcam::EventDirector)
                        let allowed = cinematic
                            && boomcam::VIEW_OF_PORT[(mode as usize).min(5)] >= 2
                            && !self.look_back
                            && !self.cam_switch.blend.active()
                            && !self.ability_cam.active()
                            && matches!(self.game.mode, Mode::FreeRoam | Mode::Race);
                        let inv = render_state.rot.inverse();
                        let front_z = render_state.wheels.iter().map(|w| (inv * (w.world_pos - render_state.pos)).z).fold(f32::INFINITY, f32::min);
                        let track_car = boomcam::TrackCar {
                            pos: render_state.pos,
                            rot: render_state.rot,
                            vel: render_state.vel,
                            speed: render_state.speed(),
                            reverse_gear: render_state.gear < 0,
                            steer: car.input.steer,
                            front_z: if front_z.is_finite() { front_z } else { 0.0 },
                            slope: boomcam::TrackCar::slope_of(render_state.rot, &render_state.wheels),
                            air_time: render_state.airborne_time,
                            // (mcCarAero +4: AeroDampFactor while wiped out and over 4.47 m/s)
                            aero: if self.crash_bits & 0x10 != 0 && render_state.speed() > 4.47 { car.model.simtunes.as_ref().map_or(1.0, |s| s.base.s.aero_damp_factor) } else { 1.0 },
                            model_offset_y: car.model.simtunes.as_ref().map_or(0.75, |s| s.base.s.model_offset_y),
                            sst: render_state.veh.as_ref().map_or((0.0, 1.0), |v| (v.sst.fov, v.sst.dist)),
                            nitro_view: render_state.veh.as_ref().map_or(0.0, |v| v.nitro_view),
                            grounded: render_state.wheels.iter().filter(|w| w.grounded).count(),
                            wheels: render_state.wheels.len(),
                            // (body +320: the moments in the body's axes times the spin there, back to the world)
                            ang_mom: car.params().veh.as_ref().map_or(Vec3::ZERO, |vt| {
                                let i = crate::physics::vehsim::moments(car.params(), &vt.t);
                                render_state.rot * (i * (render_state.rot.inverse() * render_state.ang_vel))
                            }),
                        };
                        let charge = render_state.veh.as_ref().map_or(0.0, |v| v.burn);
                        // the chooser's inputs (F-24075; boomcam::EventIn)
                        let vs = render_state.veh.as_ref();
                        let bike = car.model.bike;
                        // the slipstream turbo's lens (mcCarSSTurbo +20, F-60935): to turboFOV at
                        // |FOV| / FovOutTime a second while it runs, back at FovInTime - physics/sst.rs
                        let sst_on = vs.map_or(false, |v| v.sst.burning);
                        let sfov = render_state.veh.as_ref().zip(car.params().veh.as_ref()).map_or(0.0, |(_, t)| t.t.ss_turbo.fov);
                        self.sst_view = vs.map_or(0.0, |v| v.sst.fov);
                        let inv = render_state.rot.inverse();
                        let zs: Vec<f32> = render_state.wheels.iter().map(|w| (inv * (w.world_pos - render_state.pos)).z).collect();
                        let (zmin, zmax) = zs.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &z| (a.min(z), b.max(z)));
                        // (the chassis' wheel 0 is the first front wheel, wheel 1 the first back one)
                        let first_down = |z0: f32| zs.iter().position(|&z| (z - z0).abs() < 0.05).map_or(true, |i| render_state.wheels[i].grounded);
                        let air_time = render_state.airborne_time;
                        let ein = boomcam::EventIn {
                            allowed,
                            view: boomcam::VIEW_OF_PORT[(mode as usize).min(5)] as u8,
                            bike,
                            stunt: vs.map_or(0, |v| if bike { v.bg.state } else { v.wheelie as i16 }),
                            g18: vs.map_or(0, |v| if bike { v.bg.intent } else { v.two as i16 }),
                            sst_on,
                            sst_full: sst_on && self.sst_view == sfov,
                            nitro_on: vs.map_or(false, |v| v.nitro_burning),
                            nitro_full: vs.map_or(false, |v| v.nitro_burning) && car.model.simtunes.as_ref().map_or(false, |s| vs.map_or(false, |v| v.nitro_view == s.base.nitro.fov)),
                            tuck: bike && car.input.lean < -0.5 && air_time < 0.5,
                            brake: car.input.brake,
                            handbrake: if self.own_handbrake { 1.0 } else { 0.0 },
                            hb_sim: if car.input.handbrake { 1.0 } else { 0.0 },
                            stick_y: car.input.lean,
                            stick_x: car.input.steer,
                            front_down: first_down(zmin),
                            rear_down: first_down(zmax),
                            speed: render_state.speed(),
                            charge,
                            yaw_rate: render_state.ang_vel.y,
                            hyd_on: vs.map_or(false, |v| v.hyd.on),
                            pan_pressed: self.pad_pan.0,
                            pan_side: self.pad_pan_side,
                            cam_pressed: self.pad_cam_ud,
                            reverse_gear: render_state.gear < 0,
                            air_time,
                            idle: self.idle_time,
                            ..Default::default()
                        };
                        self.pad_pan.0 = false;
                        self.pad_cam_ud = false;
                        // the wipeout camera (F-38892 / F-37740): taken while the crash bits
                        // are set (every frame until it is in use; not while Damageout is set); a running
                        // event is ended first, and the camera waits for its blend back [?: the
                        // director's queue], then is cut to
                        let model_offset = car.model.simtunes.as_ref().map_or(Vec3::ZERO, |s| Vec3::new(s.base.s.model_offset_x, s.base.s.model_offset_y, s.base.s.model_offset_z));
                        let view_now = boomcam::VIEW_OF_PORT[(mode as usize).min(5)] as u8;
                        if self.crash_bits != 0 && !self.crash_cam.active() && self.event_dir.event() != Some(boomcam::EV_DAMAGEOUT) {
                            let queued = self.event_dir.running() || self.event_dir.leaving();
                            self.event_dir.end(&data_dir, &car.model.name);
                            if !self.event_dir.leaving() {
                                let mut how = boomcam::CrashStart::choose(view_now, self.crash_exploding, self.crash_bits & 0x20 != 0, render_state.up().y, render_state.rot * Vec3::X, render_state.vel);
                                if queued {
                                    how.blend = 0.0;
                                }
                                self.crash_cam.start(model_offset, &track_car, how);
                                // (F-37740's tail: the look-back slow motion released)
                                let now = self.wipe_scale.min(self.ability.time_scale());
                                self.warp_look.release(now);
                                lookback_scene(&mut self.look_scene, self.audio.as_mut(), false);
                                if how.blend == 0.0 && !queued {
                                    self.cam_switch.mask.start(0.01, 0.1, 0.01);
                                }
                                if self.crash_exploding {
                                    let now = self.wipe_scale;
                                    self.warp_expl.start(now, 0.0);
                                }
                                self.shake_wipe = true;
                            }
                        }
                        // the bits gone while it is the current camera: a cut to the drive view
                        if self.crash_bits == 0 && self.crash_cam.active() {
                            self.crash_cam.stop();
                            self.boom.reset();
                            let now = self.wipe_scale.min(self.ability.time_scale());
                            self.warp_look.release(now);
                            lookback_scene(&mut self.look_scene, self.audio.as_mut(), false);
                        }
                        let out = if self.crash_cam.active() {
                            self.crash_cam.frame(&track_car, out, dt, sim_dt, &cam_level, gpu.config.width as f32 / gpu.config.height.max(1) as f32)
                        } else {
                            out
                        };
                        let ein = boomcam::EventIn {
                            crash: self.crash_cam.active(),
                            damage_out: self.crash_bits & 4 != 0 && self.crash_timer <= 0.0 && !car.model.bike,
                            destroyed: self.crash_bits & 4 != 0,
                            ..ein
                        };
                        let out = self.event_dir.frame(
                            &data_dir,
                            &car.model.name,
                            &ein,
                            &track_car,
                            out,
                            dt,
                            &cam_level,
                            // (the line probe: one-sided, the fixed objects - F-20855)
                            &|a, b| cam_level.line(a, b),
                            gpu.config.width as f32 / gpu.config.height.max(1) as f32,
                        );
                        // the ability camera (F-38892): pushed over InTime while the ability runs in a
                        // chase view (no look back, no blend; the event camera's event cleared), handed
                        // back over OutTime when it stops; not while the wipeout camera has the view.
                        // (approx.: "Agro at the car's load" is taken as the ability equipped now)
                        let out = if self.crash_cam.active() {
                            self.ability_cam.reset();
                            out
                        } else {
                            let running = self.ability.running();
                            let push = running && boomcam::VIEW_OF_PORT[(mode as usize).min(5)] >= 2 && !self.look_back && !self.cam_switch.blend.active();
                            let agro = self.ability.kind == ability::Kind::Agro;
                            let (o, pushed) = self.ability_cam.frame(push, running, self.ability.cam_times(), agro, &track_car, out, dt, &cam_level, gpu.config.width as f32 / gpu.config.height.max(1) as f32);
                            if pushed {
                                self.event_dir.reset();
                            }
                            o
                        };
                        if std::env::var("MCLA_EVENT_DEBUG").is_ok() && self.frame_no % 10 == 0 {
                            log::info!("event cam: running {} allowed {} handbrake {} charge {:.2} yaw {:.2} cam {:.1} {:.1} {:.1}", self.event_dir.running(), allowed, car.input.handbrake, charge, render_state.ang_vel.y, out.pos.x, out.pos.y, out.pos.z);
                        }
                        if std::env::var("MCLA_CRASH_DEBUG").is_ok() && (self.crash_bits != 0 || self.frame_no % 30 == 0) {
                            log::info!("crash: bits {:#x} timer {:.2} exploding {} wipeout cam {} event {:?} scale {:.2} speed {:.1} damage {:.2} cam {:.1} {:.1} {:.1} car {:.1} {:.1} {:.1} fov {:.0}", self.crash_bits, self.crash_timer, self.crash_exploding, self.crash_cam.active(), self.event_dir.event(), self.wipe_scale, render_state.speed(), render_state.damage, out.pos.x, out.pos.y, out.pos.z, render_state.pos.x, render_state.pos.y, render_state.pos.z, out.fov);
                        }
                        // (the Damageout view wholly in: the wipeout camera is out of the director)
                        if self.crash_cam.active() && self.event_dir.event() == Some(boomcam::EV_DAMAGEOUT) && self.event_dir.shown() {
                            self.crash_cam.stop();
                        }
                        self.chase.pos = out.pos;
                        self.chase.look = out.look;
                        // the camera kept out of walls (camBoomCS CollideType 2, F-12669): a ball
                        // of the near plane's corner from the file's Collide point on the car to the
                        // camera; the camera stops where it touches, the look keeps its direction
                        // (the event camera has its own, not ported)
                        if tune.collide_type() & 2 != 0 && !self.event_dir.running() && !self.crash_cam.active() && !self.ability_cam.active() {
                            let aspect = gpu.config.width as f32 / gpu.config.height.max(1) as f32;
                            let ty = (out.fov.to_radians() * 0.5).tan();
                            let corner = tune.near() * (ty * ty + (ty * aspect) * (ty * aspect)).sqrt();
                            let inside_car = boomcam::VIEW_OF_PORT[(mode as usize).min(5)] < 2 && !self.look_back;
                            let r = if inside_car { corner * 2.0 } else { corner + 0.01 };
                            let from = self.boom.collide_from(tune, &target);
                            if let Some(m) = boomcam::boom_collide(from, self.chase.pos, r, inside_car, &cam_level) {
                                self.chase.pos += m;
                                self.chase.look += m;
                            }
                        }
                        self.boom_up = Some(out.up);
                        self.cam.fov_y = (out.fov + if self.event_dir.running() || self.crash_cam.active() || self.ability_cam.active() { 0.0 } else { self.look.fov }).to_radians();
                        boom_view = true;
                        if let Some(d) = self.game.replay.as_mut().and_then(|p| p.director.as_mut()) {
                            d.drive_eye = Some(self.chase.pos);
                        }
                    } else {
                        self.cam.fov_y = self.chase.update(render_state.pos, render_state.forward(), render_state.vel, if nitro { 1.0 } else { 0.0 }, dt);
                    }
                }
                if !boom_view {
                    self.boom.reset();
                    self.cam_switch.reset();
                    self.event_dir.reset();
                    self.crash_cam.stop();
                    self.ability_cam.reset();
                    self.look.reset();
                    self.view_mode = if own_view { 255 } else { self.cam_mode };
                }
                // the HUD's camera bar shows the view in use (F-43809: the list place; the
                // overhead camera of the race editor would be 5) and a cut puts the mask up
                self.game.hud_camera = boomcam::VIEW_OF_PORT[(self.cam_mode as usize).min(5)] as i32;
                self.game.cut_mask = self.cam_switch.mask.update(dt);
                if std::env::var("MCLA_FRAME_TRACE").is_ok() {
                    let vs = render_state.veh.as_ref();
                    log::info!("frame {} ms {:.2} sim {:.2} ticks {} alpha {:.3} gear {} shift {} rpm {:.0} tach {:.0} pos {:.4} {:.4} {:.4} vel {:.4} cam {:.4} {:.4} {:.4} look {:.4} {:.4} {:.4} fov {:.4} gz {:.4}", self.frame_no, self.frame_ms, sim_ms, ticks, self.vehicles.render_alpha, render_state.gear, vs.map_or(0, |v| if v.gear_change.active { v.gear_change.direction } else { 0 }), vs.map_or(render_state.rpm, |v| v.engine_w * 9.549_296), self.sound.tach_rpm().unwrap_or(-1.0), render_state.pos.x, render_state.pos.y, render_state.pos.z, render_state.speed(), self.chase.pos.x, self.chase.pos.y, self.chase.pos.z, self.chase.look.x, self.chase.look.y, self.chase.look.z, self.cam.fov_y, self.boom.gz);
                }
                // camera shake (camFX's blocks, F-38892 / F-10707; rule in
                // docs/reference/cameras/shake_rule.md): one-shots on impacts, landings and wipeouts
                // from the car's vehCamShake, the driving, burnout and drift rumbles each frame; only
                // with the CAMERA SHAKE option, not in a replay
                let shake_on = boom_view && self.game.opts.shake && self.game.replay.is_none() && self.game.mode != Mode::Garage;
                if shake_on {
                    let data_dir = self.opt.data_dir.clone();
                    let sh = self.shake.get_or_insert_with(|| boomcam::CamShake::load(&data_dir));
                    let st = car.model.simtunes.as_ref().map(|s| s.base.cam_shake.clone()).unwrap_or_default();
                    let dm = car.model.simtunes.as_ref().map(|s| s.base.damage.clone()).unwrap_or_default();
                    let fixed = boomcam::VIEW_OF_PORT[(self.view_mode as usize).min(5)] < 2;
                    let lerp = |a: f32, b: f32, k: f32| a + (b - a) * k;
                    let speed = render_state.speed();
                    // an impact (F-11688): dv, the change of velocity the step's contact impulses
                    // gave the car (0.4 of those against fixed things; F-19931), over a threshold
                    // that goes from JoltThreshold to ImpactThreshold between 5 and 30 m/s; k = the
                    // excess over FatalBlow
                    let impact = shake_dv;
                    if impact > 0.0 {
                        let s5 = ((speed - 5.0) * 0.04).clamp(0.0, 1.0);
                        let thr = lerp(dm.jolt_threshold, dm.impact_threshold, s5);
                        if impact > thr && dm.fatal_blow > 0.0 {
                            let k = ((impact - thr) / dm.fatal_blow).clamp(0.0, 1.0);
                            sh.jolt(lerp(st.impact_duration_min, st.impact_duration_max, k), lerp(st.impact_amplitude_min, st.impact_amplitude_max, k), fixed);
                        }
                    }
                    self.shake_impact = impact;
                    // the slipstream turbo going off (F-68670): 0.5 s (half in the bumper and
                    // cockpit) at 0.78, the object's own constants
                    let sst = render_state.veh.as_ref().map_or(false, |v| v.sst.burning);
                    if sst && !self.shake_sst {
                        sh.jolt(if fixed { 0.25 } else { 0.5 }, 0.78, fixed);
                    }
                    self.shake_sst = sst;
                    for (d, a) in self.shake_jolts.drain(..) {
                        sh.jolt(d, a, fixed);
                    }
                    // a landing after more than half a second in the air (F-53665, car +473):
                    // DurationMax / AmplitudeMax on the first tick down, then again on every tick a
                    // wheel is still bottomed out on fixed ground (wheel +530)
                    let down = render_state.wheels.iter().any(|w| w.grounded);
                    if down {
                        if self.shake_air > 0.5 {
                            self.shake_land = true;
                        }
                        if self.shake_land {
                            sh.jolt(st.impact_duration_max, st.impact_amplitude_max, fixed);
                            if !render_state.veh.as_ref().map_or(false, |v| v.wheels.iter().any(|w| w.bottomed)) {
                                self.shake_land = false;
                            }
                        }
                        self.shake_air = 0.0;
                    } else {
                        self.shake_air += dt;
                    }
                    // the wipeout camera starts (F-37740): every block off (the wipeout's own
                    // impact jolt of the same frame with them), then its JoltDuration 0.75 s at
                    // JoltAmplitude 0.6 for a wipeout or an exploding car
                    if std::mem::take(&mut self.shake_wipe) {
                        sh.reset();
                        if self.crash_bits & 0x10 != 0 || self.crash_exploding {
                            sh.jolt(0.75, 0.6, fixed);
                        }
                    }
                    // the rumbles (F-38892): driving between the tune's two shares of the top
                    // gear's speed, times MagnitudeMultiplier, none in the air
                    let top = car.model.params.veh.as_ref().map_or(60.0, |v| v.top_speed.max(1.0));
                    let pct = (speed / top).clamp(0.0, 1.0);
                    let (thr, max) = (st.rumble_threshold_speed_pct, st.rumble_max_shake_speed_pct);
                    let mut r = 0.0;
                    if pct > thr {
                        let range = if max - thr <= 0.0 { 0.01 } else { max - thr };
                        r = ((pct - thr).clamp(0.0, 1.0) / range).min(1.0);
                    }
                    r *= st.magnitude_multiplier;
                    if self.shake_air > 0.1 {
                        r = 0.0;
                    }
                    sh.set(boomcam::SHAKE_DRIVING, if fixed { r * 0.4 } else { r });
                    // burnout: the charge in any burnout state (charging, full, launching), the tyres'
                    // melt if more, full while a nitrous shot burns
                    let b = render_state.veh.as_ref().map_or(0.0, |v| {
                        let b = if v.burn_state != 0 { v.burn.max(0.0) } else { 0.0 };
                        let b = b.max(v.melt);
                        if v.nitro_burning { b.max(1.0) } else { b }
                    });
                    sh.set(boomcam::SHAKE_BURNOUT, if fixed { b * 0.2 } else { b });
                    // drift: above 20 m/s the most sliding tyre (wheel +520, its slide), ramped in up
                    // to 30 m/s
                    let mut d = 0.0;
                    if speed > 20.0 {
                        let m = render_state.veh.as_ref().map_or(0.0, |v| v.wheels.iter().map(|w| ((w.slide - 0.5) * 2.0).clamp(0.0, 1.0)).fold(0.0, f32::max));
                        d = (m * 2.0).clamp(0.0, 1.0);
                        if speed - 20.0 < 10.0 {
                            d *= (speed - 20.0) * 0.1;
                        }
                    }
                    sh.set(boomcam::SHAKE_DRIFT, if fixed { d * 0.08 } else { d });
                } else {
                    self.shake_jolts.clear();
                    if let Some(sh) = self.shake.as_mut() {
                        sh.reset();
                    }
                }
                // the seconds the car is left alone (the pad block's +112): the event camera's Idle
                // view comes after camFX IdleDelayTime (60 s; boomcam::EventDirector). The clock runs
                // while gas, brake and handbrake are all exactly 0 - the steering and the car's
                // movement do not stop it - and stands still while the game is paused
                {
                    let busy = car.input.throttle != 0.0 || car.input.brake != 0.0 || self.own_handbrake;
                    if self.frame_no % 120 == 0 && std::env::var("MCLA_IDLE_DEBUG").is_ok() {
                        log::info!("idle: time {:.1} busy {} (gas {} brake {} handbrake {}) speed {:.2} boom {} idle view {}", self.idle_time, busy, car.input.throttle, car.input.brake, self.own_handbrake, render_state.speed(), boom_view, self.event_dir.idle());
                    }
                    if busy || !boom_view || !matches!(self.game.mode, Mode::FreeRoam | Mode::Race) {
                        self.idle_time = 0.0;
                    } else if !self.game.in_menu() {
                        self.idle_time += dt;
                    }
                    if let Ok(v) = std::env::var("MCLA_IDLE_CAM") {
                        // (test hook: the idle time starts at this many seconds short of the delay)
                        if self.idle_time > 0.0 && self.idle_time < dt * 1.5 {
                            self.idle_time = (60.0 - v.parse::<f32>().unwrap_or(60.0)).max(0.0);
                        }
                    }
                    self.game.idle_cam = self.event_dir.idle();
                }
                self.hide_player = boom_view && self.view_mode == 4 && !self.look_back && !(self.crash_cam.active() || self.event_dir.event() == Some(boomcam::EV_DAMAGEOUT));
                // keep the camera out of walls and hillsides (camBoomCS CollideType): pull it in
                // along the line from the car to where it wants to be
                // (the driving views have the original's collision in the boom section; this
                // ray rule is the port's own, left for the garage's camera)
                if self.game.mode == Mode::Garage && !own_view {
                    let from = render_state.pos + Vec3::Y * 1.1;
                    let to = self.chase.pos;
                    let d = to - from;
                    let len = d.length();
                    if len > 1.0 {
                        // (the original sweeps a sphere that holds the near plane's corners from the
                        // car to the camera: here the middle ray and four at that radius round it)
                        let dir = d / len;
                        let side = dir.cross(Vec3::Y).normalize_or(Vec3::X);
                        let upv = side.cross(dir);
                        let r = 0.45;
                        let mut hit_t: Option<f32> = None;
                        // (the middle ray every frame, one of the four outer ones in turn; an outer
                        // ray's last answer stands until its turn comes again)
                        if let Some(h) = world.collider.raycast(from, dir, len + 0.5) {
                            hit_t = Some(h.t);
                        }
                        let k = (self.frame_no % 4) as usize;
                        let off = [side * r, -side * r, upv * r, -upv * r][k];
                        self.cam_side_t[k] = world.collider.raycast(from + off, dir, len + 0.5).map_or(f32::INFINITY, |h| h.t);
                        let outer = self.cam_side_t.iter().cloned().fold(f32::INFINITY, f32::min);
                        if outer.is_finite() {
                            hit_t = Some(hit_t.map_or(outer, |t: f32| t.min(outer)));
                        }
                        if let Some(ht) = hit_t {
                            // (CollideMinDist 2 m; in the garage small props may stand in the way:
                            // never closer than a car length)
                            let t = (ht - 0.5).max(if self.game.mode == Mode::Garage { 4.6 } else { 2.0 });
                            if t < len {
                                self.chase.pos = from + dir * t + Vec3::Y * ((len - t) * 0.2).min(1.4);
                            }
                        }
                    }
                }
                // (the shake on the camera in use, after its collision - see CAMERAS_REPLAY.md)
                // (none while the player's Zone runs: the blocks keep their state, F-36289)
                let zone = self.ability.kind == ability::Kind::Zone && self.ability.active > 0.0;
                if shake_on && sim_dt > 0.0 && !zone {
                    if let Some(sh) = self.shake.as_mut() {
                        let mut up = self.boom_up.unwrap_or(Vec3::Y);
                        sh.apply(dt, &mut self.chase.pos, &mut self.chase.look, &mut up, render_state.pos);
                        self.boom_up = Some(up);
                    }
                }
                // cheat: the race seen from straight above
                if self.game.cheats[3] && matches!(self.game.mode, Mode::FreeRoam | Mode::Race) && self.cine.playing.is_none() {
                    self.chase.pos = render_state.pos + Vec3::Y * 62.0 - render_state.forward() * 1.5;
                    self.chase.look = render_state.pos + render_state.forward() * 6.0;
                }
                // the race's end: forms 54 and 60 come up 0.2 s after the line and the pause movie
                // takes the camera over (recorded twice in the original, CAREER_RACES.md "Race end:
                // camera and timing"). The movie's camera: from `cur_camera` 0 (PauseCamera1) to the
                // standings form's 2 (PauseCamera3; its mirror 13 when a line from it to its look-at
                // point is blocked, else 1 - F-32014), `fs_LerpFactor` += (1 - it) / 10 a movie
                // frame; the game's camera mixed into it by `player0_cam_ctrl` = sin(acos(1 - temp /
                // 100)), temp += 3 a frame; both mixes as SetPolarCam's F-22967 (the angles and
                // length of eye - look-at, the look-at and the lens straight). Movie frames 30 a
                // second (the settling seen in the original: 0.9 a frame at 30). The pause at 1.2 s
                // does not stop the movie: the camera settles on. At 2.4 s the race-over panel's
                // camera is the movie's next `cur_camera`: the same lerp from where it stands, the
                // short way round unless that path is blocked, then the long way, else a cut
                // (F-32916 tests t = 0.25, 0.5, 0.75).
                if race_end_on {
                    let t = self.game.race_end_t.unwrap_or(0.0);
                    let polar = |d: Vec3| (d.x.atan2(d.z), d.y.atan2(Vec3::new(d.x, 0.0, d.z).length()), d.length());
                    let mix = |a: (Vec3, Vec3, f32), b: (Vec3, Vec3, f32), k: f32, long: bool| -> (Vec3, Vec3, f32) {
                        let (y0, p0, r0) = polar(a.0 - a.1);
                        let (y1, p1, r1) = polar(b.0 - b.1);
                        let mut dy = y1 - y0;
                        if dy > std::f32::consts::PI { dy -= std::f32::consts::TAU; }
                        if dy < -std::f32::consts::PI { dy += std::f32::consts::TAU; }
                        if long {
                            dy -= std::f32::consts::TAU.copysign(dy);
                        }
                        let (yaw, pitch, r) = (y0 + dy * k, p0 + (p1 - p0) * k, r0 + (r1 - r0) * k);
                        let look = a.1.lerp(b.1, k);
                        (look + Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()) * r, look, a.2 + (b.2 - a.2) * k)
                    };
                    if race_end_swing && t >= game::Game::RACE_END_PANEL {
                        let fwd = render_state.forward();
                        let fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
                        let right = Vec3::new(-fwd.z, 0.0, fwd.x);
                        let origin = render_state.pos + right * PAUSE_FRAME.x + Vec3::Y * PAUSE_FRAME.y - fwd * PAUSE_FRAME.z;
                        let place = |p: Vec3| origin + right * p.x + Vec3::Y * p.y - fwd * p.z;
                        if self.race_end_cam.is_none() {
                            let logic = menu3d::UiLogic::load(&self.opt.data_dir);
                            let cam = |n: usize| logic.pause_cams.get(n - 1).copied();
                            let clear = |c: &menu3d::PauseCam| world.collider.raycast(place(c.position), (place(c.lookat) - place(c.position)).normalize_or(Vec3::Z), place(c.lookat).distance(place(c.position))).is_none();
                            let to = cam(3).filter(|c| clear(c)).or_else(|| cam(13).filter(|c| clear(c))).or_else(|| cam(1));
                            self.race_end_cam = to.zip(cam(1));
                        }
                        if let Some((to, from)) = self.race_end_cam {
                            let n = ((t - game::Game::RACE_END_PANEL) * 30.0).floor();
                            let f = 1.0 - 0.9f32.powf(n);
                            let k = (1.0 - (n * 3.0).min(100.0) / 100.0).clamp(-1.0, 1.0).acos().sin();
                            let view = |c: &menu3d::PauseCam| (place(c.position), place(c.lookat), c.fovy.to_radians());
                            let movie = mix(view(&from), view(&to), f, false);
                            let out = mix((self.chase.pos, self.chase.look, self.cam.fov_y), movie, k, false);
                            (self.chase.pos, self.chase.look, self.cam.fov_y) = out;
                            self.race_end_from = Some(out);
                        }
                    } else if !race_end_swing && t < game::Game::RACE_END_MENU + 5.0 {
                        if let Some(from) = self.race_end_from {
                            let to = (self.chase.pos, self.chase.look, self.cam.fov_y);
                            if self.race_end_way.is_none() {
                                let blocked = |long: bool| {
                                    [0.25, 0.5, 0.75].iter().any(|&k| {
                                        let (e, l, _) = mix(from, to, k, long);
                                        world.collider.raycast(l, (e - l).normalize_or(Vec3::Z), l.distance(e)).is_some()
                                    })
                                };
                                self.race_end_way = Some(if !blocked(false) { 0.0 } else if !blocked(true) { 1.0 } else { -1.0 });
                            }
                            let n = ((t - game::Game::RACE_END_MENU) * 30.0).floor();
                            let f = 1.0 - 0.9f32.powf(n);
                            match self.race_end_way {
                                Some(w) if w >= 0.0 => (self.chase.pos, self.chase.look, self.cam.fov_y) = mix(from, to, f, w > 0.5),
                                _ => {}
                            }
                        }
                    }
                }
                self.cam.pos = self.chase.pos;
            }
        }
        // diagnostics: look at the nearest parked ("parked") or moving ("traffic") ambient car
        if let Ok(v) = std::env::var("MCLA_CAR_CAM") {
            // ("player_side": beside the player's car at its tail, looking at the exhaust - carfx.rs checks)
            if v == "player_side" {
                if let Some(c) = self.vehicles.player() {
                    let rs = c.render_state(self.vehicles.render_alpha);
                    let (f, r) = (rs.forward(), rs.right());
                    let tail = rs.pos - f * 2.3 + Vec3::Y * 0.4;
                    self.chase.pos = tail + r * 3.2 - f * 0.6 + Vec3::Y * 0.2;
                    self.chase.look = tail - f * 0.5;
                    self.cam.pos = self.chase.pos;
                    self.cam.fov_y = 0.7;
                }
            }
            let want_parked = v == "parked";
            let from = self.cam.pos;
            // ("cop": the nearest cruiser, from its left side - into the driver's window)
            let cop = v == "cop";
            if let Some(c) = self.vehicles.cars.iter().filter(|c| !c.is_player && if cop { c.cop } else { c.placed && c.parked.is_some() == want_parked && c.model.name.starts_with("va_") }).min_by(|a, b| a.state.pos.distance_squared(from).partial_cmp(&b.state.pos.distance_squared(from)).unwrap()) {
                let render_state = c.render_state(self.vehicles.render_alpha);
                let (f, r) = (render_state.forward(), render_state.right());
                self.chase.pos = render_state.pos + if cop { f * 0.2 - r * 2.4 + Vec3::Y * 1.2 } else { f * 4.2 - r * 1.6 + Vec3::Y * 1.3 };
                self.chase.look = render_state.pos + if cop { f * 0.2 + Vec3::Y * 0.9 } else { Vec3::Y * 0.8 };
                self.cam.pos = self.chase.pos;
                self.cam.fov_y = 0.7;
            }
        }
        self.chase.up = self.boom_up.unwrap_or(Vec3::Y);
        // (the race-start warp keeps the picture until it lands on the scene's camera, though the scene
        // already runs for its last second - mcWarper state 19 / 29)
        if let Some((eye, fwd, up, fov)) = self.cine.camera().filter(|_| self.warp_view.is_none() && !self.garage_in.as_ref().map_or(false, |g| g.dive.is_some())) {
            self.chase.pos = eye;
            self.chase.look = eye + fwd;
            self.chase.up = up;
            self.cam.pos = eye;
            self.cam.fov_y = fov;
            if self.frame_no % 30 == 0 && std::env::var("MCLA_CINE_DUMP").is_ok() {
                log::info!("cine cam eye {:?} fwd {:?} fov {:.1}", eye, fwd, fov.to_degrees());
                for c in &self.vehicles.cars {
                    let render_state = c.render_state(self.vehicles.render_alpha);
                    log::info!("  car {} player {} pos {:?} fwd {:?}", c.model.name, c.is_player, render_state.pos, render_state.forward());
                }
            }
        }
        // (step 23b: the hangout's camera - the pull-in scene's last frame under the panel, the
        // pink slip choice's on the profiles' cars)
        if self.cine.playing.is_none() {
            if let Some((eye, look, up, fov)) = self.hscene.camera(&self.game) {
                self.chase.pos = eye;
                self.chase.look = look;
                self.chase.up = up;
                self.cam.pos = eye;
                self.cam.fov_y = fov;
            }
        }
        // (DEFAULT_WITH_CUT's climb, and its end held until the dive or the scene takes over)
        if let Some((eye, look, up, fov)) = self.climb_view.filter(|_| self.warp_view.is_none() && self.cine.playing.is_none()) {
            self.chase.pos = eye;
            self.chase.look = look;
            self.chase.up = up;
            self.cam.pos = eye;
            self.cam.fov_y = fov;
        }
        // (the camera move after START: up to the whole city, held while the world streams at the car, down onto
        // the game camera - startwarp.rs; MCLA_START_WARP_SHOTS=<prefix>,t1,..: a screenshot as its clock passes each t)
        if let Some(w) = self.startwarp.as_mut() {
            w.stay = self.game.fresh_game;
            let car = self.vehicles.player().map(|c| c.state.pos);
            let ready = car.map_or(true, |p| world.ready_at(p)) && world.streaming() == 0;
            let t0 = self.startwarp_t;
            match w.step(dt, ready, (self.chase.pos, self.chase.look, self.chase.up)) {
                Some((eye, look, up, fov)) => {
                    self.chase.pos = eye;
                    self.chase.look = look;
                    self.chase.up = up;
                    self.cam.pos = eye;
                    self.cam.fov_y = fov;
                    // (a new career's tags are the start-up checks' - `Game::save_boxes_step`)
                    if !self.game.fresh_game {
                        self.game.loading_tag = self.game.loading_tag.max(if w.loading() { 0.5 } else { 0.0 });
                    }
                }
                None => {
                    log::info!("start warp: landed at {:.3} s", t0 + dt);
                    self.startwarp = None;
                    // (mc.loadSuccess: the career script's WAITUNWARPED(1000) after the flight)
                    self.game.loading_tag = startwarp::LOAD_SUCCESS_WAIT;
                }
            }
            self.startwarp_t = t0 + dt;
            if let Ok(v) = std::env::var("MCLA_START_WARP_SHOTS") {
                let mut it = v.split(',');
                let prefix = it.next().unwrap_or("sw").to_string();
                for x in it {
                    let at = x.parse::<f32>().unwrap_or(f32::MAX);
                    if t0 < at && t0 + dt >= at {
                        self.photo_request = Some(format!("{prefix}_{x}.png"));
                    }
                }
            }
        }
        self.game.start_warp_on = self.startwarp.is_some();
        self.game.start_warp_held = self.startwarp.as_ref().is_some_and(|w| w.phase() == startwarp::Phase::Hold);
        // diagnostics: look at the n-th liveried rival: "n,side,front,up" (metres off the car)
        if let Ok(v) = std::env::var("MCLA_RIVAL_CAM") {
            let f: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let Some(c) = self.vehicles.cars.iter().filter(|c| !c.is_player && !c.vinyl_name.is_empty()).nth(f.first().copied().unwrap_or(0.0) as usize) {
                let render_state = c.render_state(self.vehicles.render_alpha);
                let (fw, r) = (render_state.forward(), render_state.right());
                self.chase.pos = render_state.pos + r * f.get(1).copied().unwrap_or(4.5) + fw * f.get(2).copied().unwrap_or(0.0) + Vec3::Y * f.get(3).copied().unwrap_or(1.0);
                self.chase.look = render_state.pos + Vec3::Y * 0.6;
                self.cam.pos = self.chase.pos;
                self.cam.fov_y = 0.7;
            }
        }
        // diagnostics: fixed camera "x,y,z,tx,ty,tz"
        if let Ok(v) = std::env::var("MCLA_CAM") {
            let f: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if f.len() >= 6 {
                self.chase.pos = Vec3::new(f[0], f[1], f[2]);
                self.chase.look = Vec3::new(f[3], f[4], f[5]);
                self.cam.pos = self.chase.pos;
                // (a seventh figure: the lens, vertical field of view in degrees)
                self.cam.fov_y = f.get(6).map_or(1.0, |d| d.to_radians());
                // (figures 8 - 10: the camera's up vector - a capture's chase camera rolls with the car,
                // 0.97 degrees at the garage gate; work/lighting/gw_camof.py. Without them: the boom's own up)
                if f.len() >= 10 {
                    self.chase.up = Vec3::new(f[7], f[8], f[9]).normalize_or_zero();
                } else if std::env::var("MCLA_CAM_DUMP").is_ok() && self.frame_no == 60 {
                    eprintln!("camup {:?}", self.chase.up);
                }
            }
        }
        // diagnostics: look into the player's car ("front", "side" or "rear")
        if let Ok(v) = std::env::var("MCLA_DRV_CAM") {
            if let Some(c) = self.vehicles.player() {
                let render_state = c.render_state(self.vehicles.render_alpha);
                let (f, r) = (render_state.forward(), render_state.right());
                let off = match v.as_str() {
                    "side" => -r * 3.2 + f * 0.6,
                    "rear" => -f * 3.6 - r * 0.8,
                    _ => f * 3.6 - r * 0.9,
                };
                self.chase.pos = render_state.pos + off + Vec3::Y * 1.25;
                self.chase.look = render_state.pos + Vec3::Y * 0.75;
                self.cam.pos = self.chase.pos;
                self.cam.fov_y = 0.7;
            }
        }
        // diagnostics (N-VP-3): "i,dist,height,fov" - side view of the player's wheel i, the lens aimed
        // at the drawn tyre's lowest point (hub - car up x drawn radius) from `height` above it, so
        // that point is the image's middle
        if let Ok(v) = std::env::var("MCLA_WHEEL_CAM") {
            let f: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let Some(c) = self.vehicles.player() {
                let i = f.first().copied().unwrap_or(0.0) as usize & 3;
                let render_state = c.render_state(self.vehicles.render_alpha);
                let up = render_state.rot * Vec3::Y;
                let side = render_state.rot * if i % 2 == 0 { Vec3::NEG_X } else { Vec3::X };
                let r = c.params().veh.as_ref().map(|v| if i < 2 { v.t.wheel_front.radius } else { v.t.wheel_back.radius }).unwrap_or(c.model.params.wheel_radius) + 0.00635;
                let bottom = render_state.wheels[i].world_pos - up * r;
                self.chase.pos = bottom + side * f.get(1).copied().unwrap_or(6.0) + Vec3::Y * f.get(2).copied().unwrap_or(0.15);
                self.chase.look = bottom;
                self.chase.up = Vec3::Y;
                self.cam.pos = self.chase.pos;
                self.cam.fov_y = f.get(3).copied().unwrap_or(6.0).to_radians();
                if self.frame_no % 60 == 0 {
                    eprintln!("wheel cam: {} wheel {i} bottom {:?} r {r:.4} cam {:?}", c.model.name, bottom, self.cam.pos);
                }
            }
        }
        // diagnostics: look at the n-th nearest pedestrian from the front
        if let Ok(v) = std::env::var("MCLA_PED_CAM") {
            let k: usize = v.parse().unwrap_or(0);
            if let Some((p, f, _)) = self.peds.nearest(self.cam.pos, k + 1).into_iter().nth(k) {
                let side = Vec3::new(-f.z, 0.0, f.x);
                self.chase.pos = p + f * 3.2 + side * 1.6 + Vec3::Y * 1.5;
                self.chase.look = p + Vec3::Y * 0.95;
                self.cam.pos = self.chase.pos;
                self.cam.fov_y = 0.75;
            }
        }
        // --- the map screen: the city from the air --------------------------------
        self.take_citymap(&mut gpu, false);
        // (ARCADE stands over the city seen from the air, near the player - `h_arc2`,
        // `call_accept_0`: the map's own view, close in)
        let arcade_view = self.game.mode == Mode::RaceSelect && self.game.flash_navsys.is_some() && self.game.hangout_key.is_none() && self.hangout_in.is_none() && !self.game.wager_here && std::env::var("MCLA_ARCADE_FLAT").is_err();
        let map3d = (self.game.show_map || arcade_view) && self.citymap.is_some() && self.drive;
        // The original's GPS map is the city itself drawn from the map camera (its far models, the
        // hour's light, rule_frame.md 4.1); MCLA_MAP_BAKED=1: the port's baked top-down picture.
        let map_baked = map3d && std::env::var("MCLA_MAP_BAKED").is_ok();
        if map3d {
            // (the game camera the map's opening flies from)
            let (game_eye, game_look) = (self.chase.pos, self.chase.look);
            let game_fov = self.cam.fov_y.to_degrees();
            // (the coarse relief for the GPS map's x-ray, hudmap::Heights)
            if let (Some(hm), Some(cm)) = (self.game.hm.as_mut(), self.citymap.as_ref()) {
                if hm.heights.is_none() {
                    let m = &cm.map;
                    hm.heights = Some(hudmap::Heights { min_x: m.min_x, min_z: m.min_z, cell: citymap::RES * citymap::STEP as f32, gw: m.gw, gh: m.gh, h: std::sync::Arc::new(m.height.clone()), coarse: Default::default() });
                }
            }
            let cm = self.citymap.as_ref().unwrap();
            if self.map_open == 0.0 {
                // opened: above the player, one step out
                self.map_target = self.vehicles.player().map(|c| c.state.pos).unwrap_or(self.cam.pos);
                self.map_zoom = 0.0;
                // (the original's GPS opens on the whole city: reference `z00`)
                self.map_zoom_want = std::env::var("MCLA_MAP_ZOOM").ok().and_then(|v| v.parse().ok()).unwrap_or(if self.game.flash_navsys.is_some() { 3.0 } else { 1.0 });
                // (the mission log brought up from a call lies over the streets around the
                // player, seen from near - `ng_mlog_active`; height and tilt by eye)
                if arcade_view {
                    self.map_zoom_want = std::env::var("MCLA_ARCADE_ZOOM").ok().and_then(|v| v.parse().ok()).unwrap_or(0.9);
                    self.map_zoom = self.map_zoom_want;
                }
                if self.game.flash_navsys.is_some() && self.game.mlog_near {
                    self.map_zoom_want = std::env::var("MCLA_MLOG_ZOOM").ok().and_then(|v| v.parse().ok()).unwrap_or(0.7);
                }
            }
            if self.game.mlog_near && self.game.mission_log.2 {
                // (the map shown from there is the whole city again)
                self.game.mlog_near = false;
                self.map_zoom_want = 3.0;
            }
            self.map_open = (self.map_open + dt * 1.6).min(1.0);
            // the race editor's cursor leads the view; otherwise the arrows pan it
            if let Some(_) = self.game.editor.as_ref() {
                self.map_target = self.map_target.lerp(self.game.editor_cursor, (dt * 6.0).min(1.0));
            } else if let (Some(p), true) = (self.game.map_cursor_pos, self.game.flash_navsys.is_some()) {
                // (with the map movie the view goes with the map's cursor)
                self.map_target = self.map_target.lerp(p, (dt * 4.0).min(1.0));
            } else {
                let span = 420.0 * 2.5f32.powf(self.map_zoom);
                let mut mv = Vec3::ZERO;
                if self.input.down(KeyCode::ArrowLeft) || self.input.down(KeyCode::KeyA) { mv.x -= 1.0; }
                if self.input.down(KeyCode::ArrowRight) || self.input.down(KeyCode::KeyD) { mv.x += 1.0; }
                if self.input.down(KeyCode::ArrowUp) || self.input.down(KeyCode::KeyW) { mv.z -= 1.0; }
                if self.input.down(KeyCode::ArrowDown) || self.input.down(KeyCode::KeyS) { mv.z += 1.0; }
                self.map_target += mv * span * dt * 1.2;
            }
            let (sx, sz) = cm.map.size();
            let c = cm.map.center();
            self.map_target.x = self.map_target.x.clamp(c.x - sx * 0.5, c.x + sx * 0.5);
            self.map_target.z = self.map_target.z.clamp(c.z - sz * 0.5, c.z + sz * 0.5);
            self.map_zoom += (self.map_zoom_want - self.map_zoom) * (1.0 - (-dt * 5.0).exp());
            // the further out, the steeper the view; all the way out it shows the whole city
            let z = self.map_zoom.clamp(0.0, 3.0);
            let dist = 520.0 * 2.5f32.powf(z);
            let pitch = (48.0 + 12.0 * z).min(84.0).to_radians();
            let k = (z - 2.0).clamp(0.0, 1.0);
            // (the original's whole-city view fills the screen: reference `z00`)
            let dist = if self.game.flash_navsys.is_some() { dist * (1.0 - 0.38 * k) } else { dist };
            let ground = cm.map.height_at(self.map_target.x, self.map_target.z).max(0.0);
            let mut look = Vec3::new(self.map_target.x, ground, self.map_target.z);
            look = look.lerp(Vec3::new(c.x, 0.0, c.z + sz * 0.02), k);
            let eye = look + Vec3::new(0.0, pitch.sin(), pitch.cos()) * dist;
            self.chase.pos = eye;
            self.chase.look = look;
            self.chase.up = Vec3::Y;
            self.cam.pos = eye;
            self.cam.fov_y = 40f32.to_radians();
            // The original's GPS camera (NavMap, rule_frame.md 4.3): height H from 850 to 9000 on
            // t = 1 - sin(u pi / 2), u stepped 0.02 a frame (30 a second) while a zoom trigger is held;
            // the eye stands south of the look point by (1 - (H - 850) / 4150) x 1300 m below 5000 m,
            // 10 m above; 25 degree lens. (MCLA_MAP_OLDCAM=1: the port's own view.)
            if self.game.flash_navsys.is_some() && self.game.hm.is_some() && !arcade_view && self.game.editor.is_none() && std::env::var("MCLA_MAP_OLDCAM").is_err() {
                let fresh = self.map_open <= dt * 1.6 + 1e-4;
                let aspect = gpu.config.width as f32 / gpu.config.height.max(1) as f32;
                let at = self.game.map_cursor_pos.or(self.vehicles.player().map(|c| c.state.pos)).unwrap_or(c);
                if fresh {
                    self.nav.open(game_eye, game_look, at, aspect);
                    // (the lens goes from the game camera's to 25 degrees - NavLens)
                    // (only on the map's first frame: the second "fresh" one already has the map's lens)
                    if self.nav_rows.is_none() {
                        let settled = std::env::var("MCLA_MAP_SETTLED").is_ok() || std::env::var("MCLA_MAP_ZOOM_IN").is_ok();
                        self.nav.lens = if settled { hudmap::NavLens::done() } else { hudmap::NavLens::new(game_fov) };
                        if std::env::var("MCLA_NAV_LOG").is_ok() {
                            log::info!("navcam: lens from {:.2}", game_fov);
                        }
                    }
                    if std::env::var("MCLA_MAP_SETTLED").is_ok() {
                        self.nav.update(10.0, false, false, glam::Vec2::ZERO, aspect);
                    }
                    // (MCLA_MAP_ZOOM_IN=n: n frames of RT at once, as the recorded zoom steps)
                    if let Some(n) = std::env::var("MCLA_MAP_ZOOM_IN").ok().and_then(|v| v.parse::<u32>().ok()) {
                        self.nav.update(10.0, false, false, glam::Vec2::ZERO, aspect);
                        for _ in 0..n {
                            self.nav.update(1.0 / 30.0, true, false, glam::Vec2::ZERO, aspect);
                        }
                    }
                } else if self.game.map_free.is_some() {
                    // (the free cursor is the camera's own)
                } else if self.nav_free {
                    // (the stick let go on an icon: the cursor sits on it, the view stays)
                    self.nav.cursor = at;
                } else if at.distance(self.nav.cursor) > 0.5 {
                    // CHANGE TARGET: the camera flies over the new place (Change_Target, 1 s)
                    self.nav.fly_to(at);
                }
                // the left stick / WASD move the cursor freely (F-62325): 0.5 H m/s at full tilt
                let mut stick = glam::Vec2::ZERO;
                if self.input.down(KeyCode::KeyA) { stick.x -= 1.0; }
                if self.input.down(KeyCode::KeyD) { stick.x += 1.0; }
                if self.input.down(KeyCode::KeyW) { stick.y -= 1.0; }
                if self.input.down(KeyCode::KeyS) { stick.y += 1.0; }
                if let Some(pad) = self.gamepad.as_ref().and_then(|g| active_pad(g, self.pad_id)) {
                    let v = hudmap::NavCam::shape_stick(glam::Vec2::new(pad.axis(pads::Axis::LeftX), -pad.axis(pads::Axis::LeftY)));
                    if v != glam::Vec2::ZERO {
                        stick = v;
                    }
                }
                self.nav_free = self.game.map_free.is_some();
                if stick != glam::Vec2::ZERO && !fresh {
                    let before = self.nav.cursor;
                    self.nav.update(0.0, false, false, stick, aspect);
                    let step = stick * 0.5 * self.nav.eye.y * dt;
                    self.nav.cursor = before + Vec3::new(step.x, 0.0, step.y);
                    // (at the screen's edge the view goes along - approx.: the inner 80 %; the
                    // game's limits NM+572..584 hold runtime values not read)
                    if let Some(q) = self.nav.ndc(self.nav.cursor, aspect) {
                        let mut push = Vec3::ZERO;
                        if (q.x > 0.8 && step.x > 0.0) || (q.x < -0.8 && step.x < 0.0) { push.x = step.x; }
                        if (q.y < -0.8 && step.y > 0.0) || (q.y > 0.8 && step.y < 0.0) { push.z = step.y; }
                        if push != Vec3::ZERO {
                            self.nav.push(push);
                        }
                    }
                    self.game.map_free = Some(self.nav.cursor);
                } else if self.game.map_free.is_some() && self.nav_free_moving {
                    self.game.map_free_release = true;
                }
                self.nav_free_moving = stick != glam::Vec2::ZERO;
                let zin = self.input.down(KeyCode::PageUp) || self.input.down(KeyCode::KeyZ) || self.input.down(KeyCode::Equal);
                let zout = self.input.down(KeyCode::PageDown) || self.input.down(KeyCode::KeyX) || self.input.down(KeyCode::Minus);
                self.nav.update(dt, zin, zout, glam::Vec2::ZERO, aspect);
                let rows = self.nav.view_rows();
                let (eye, look) = (rows.eye, rows.look_point());
                if std::env::var("MCLA_NAV_LOG").is_ok() && self.tick_no % 30 == 0 {
                    log::info!("navcam: eye {:?} look {:?} cursor {:?} at {:?} map_open {:.3} dt {:.4} lens {:.2}", eye, look, self.nav.cursor, at, self.map_open, dt, self.nav.lens.deg);
                }
                self.chase.pos = eye;
                self.chase.look = look;
                self.chase.up = rows.r1;
                self.cam.pos = eye;
                self.cam.fov_y = self.nav.lens.step(dt).to_radians();
                self.nav_rows = Some(rows);
            }
            // close in, a finer patch of the relief lies over the coarse city
            let want = if z < 0.6 { Some((650.0f32, 1usize)) } else if z < 1.7 { Some((1500.0, 2)) } else { None };
            let cm = self.citymap.as_mut().unwrap();
            match want {
                Some((half, step)) => {
                    let stale = cm.patch.as_ref().map_or(true, |(_, _, _, (cx, cz, h, s))| *s != step || *h != half || (look.x - cx).abs().max((look.z - cz).abs()) > half * 0.25);
                    if stale {
                        use wgpu::util::DeviceExt;
                        let (verts, idx) = cm.map.patch(look.x, look.z, half, step);
                        if !idx.is_empty() {
                            let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("citymap patch vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
                            let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("citymap patch ib"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
                            cm.patch = Some((vb, ib, idx.len() as u32, (look.x, look.z, half, step)));
                        }
                    }
                }
                None => cm.patch = None,
            }
        } else {
            self.map_open = 0.0;
            // The map closed: fly back to the driving camera (Exiting_Map_GPS_HighAltitude /
            // AboveCar, rule_exitcam.md) - from the map's last camera to this frame's game camera.
            if let Some(start) = self.nav_rows.take() {
                if std::env::var("MCLA_MAP_CUT").is_err() {
                    let end = hudmap::CamRows::look_at(self.chase.pos, self.chase.look, self.boom_up.unwrap_or(Vec3::Y));
                    let ex = hudmap::NavExit::moving(start, end, self.nav.eye_velocity());
                    // (the lens back from 25 degrees to the driving camera's - NavLens)
                    let t = ex.total();
                    self.nav.lens = hudmap::NavLens::closing(self.cam.fov_y.to_degrees(), 0.2 * t, 0.7 * t);
                    self.nav.exit = Some(ex);
                }
            }
            if let Some(ex) = self.nav.exit.as_mut() {
                match ex.step(dt) {
                    Some(c) => {
                        if std::env::var("MCLA_NAV_LOG").is_ok() {
                            log::info!("nav exit: eye {:?} pitch {:.1} dt {:.4} lens {:.2}", c.eye, (-c.r2.y).clamp(-1.0, 1.0).asin().to_degrees(), dt, self.nav.lens.deg);
                        }
                        self.chase.pos = c.eye;
                        self.chase.look = c.look_point();
                        self.chase.up = c.r1;
                        self.cam.pos = c.eye;
                        self.cam.fov_y = self.nav.lens.step(dt).to_radians();
                    }
                    None => self.nav.exit = None,
                }
            }
        }
        self.vehicles.hide_player = self.hide_player || (self.game.fresh_game && self.intro_stage != 0) || self.game.starter_pick.is_some();
        // (test hook: MCLA_CAM_FIX=px,py,pz,bx,by,bz,ux,uy,uz,fov_y_deg - the chase camera put where a recorded frame of the
        // original had it, for pixel comparisons: its position and back / up axes (gViewInverse c15, c14, c13 of the frame,
        // the same world as the port's) and its vertical field of view)
        if let Ok(s) = std::env::var("MCLA_CAM_FIX") {
            let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if v.len() == 10 {
                let (p, b, u) = (Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5]), Vec3::new(v[6], v[7], v[8]));
                self.chase.pos = p;
                self.chase.look = p - b;
                self.chase.up = u;
                self.cam.fov_y = v[9].to_radians();
            }
        }
        // (the driver's camera: the original's POV camera sets G+122 of the car it follows - F-95074 -
        // and its windows take the in-car glass program; damage_map_rule.md Part C 6. The draw mode (racer
        // +680, F-60712): 5 in the POV view, forced to 3 while the wipeout camera or the Damageout event has
        // the view or a cutscene runs - the glass is bit 4 of it, looking back or not)
        let pov = self.view_mode == 3 && !self.event_dir.idle() && !(self.crash_cam.active() || self.event_dir.event() == Some(boomcam::EV_DAMAGEOUT)) && self.cine.playing.is_none();
        self.vehicles.interior_glass = pov;
        // (the player's driver, F-60712 -> F-45131 -> F-14542 / F-56050: in the POV view the set's cockpit view
        // model (arms and hands), none at all while looking back from it; in the bumper view none either)
        self.drivers.cockpit = if pov { if self.look_back { 2 } else { 1 } } else if self.view_mode == 4 && self.cine.playing.is_none() && !self.crash_cam.active() { 2 } else { 0 };
        // (m_NFLFade for the night fill light the traffic cars add to their ambient - fillmap.rs)
        fillmap::set_fade(self.sky_now().tod.map_or(0.0, |t| t.nfl));
        // (the traffic cars' lamp states and glow levels, for the cars drawn now - vehicles.rs
        // `AmbLamps`: in the view of this frame's camera and within the lamps' reach along its axis)
        {
            let aspect = gpu.config.width as f32 / gpu.config.height.max(1) as f32;
            let (eye, view, vp) = if self.drive && self.vehicles.player().is_some() { (self.chase.pos, self.chase.view(), self.cam.proj(aspect) * self.chase.view()) } else { (self.cam.pos, self.cam.view(), self.cam.view_proj(aspect)) };
            let axis = -Vec3::new(view.x_axis.z, view.y_axis.z, view.z_axis.z);
            self.vehicles.traffic_lamps(eye, axis, &Frustum::from_view_proj(vp), night, sim_dt);
            // (and for the wheels' side test - F-57869)
            self.vehicles.set_wheel_eye(eye);
        }
        // (the set-up cars' lamp states - vehicles.rs `setup_lamps`, F-10129: head lamps due past the
        // light manager's 18.0 or before its 6.0, or in Stormy or Foggy weather weighing over 0.85; in the
        // garage by its view, `garage_lamps_due`)
        self.cine_lamps_tick();
        {
            let due = if self.game.mode == Mode::Garage {
                vehicles::garage_lamps_due(self.game.garage_view())
            } else {
                self.time_of_day > 18.0 || self.time_of_day < 6.0 || self.weather_weight(sky::Weather::Stormy) > 0.85 || self.weather_weight(sky::Weather::Foggy) > 0.85
            };
            self.vehicles.setup_lamps(due, sim_dt);
        }
        // (each traffic and parked car's light-slot order, F-99357)
        self.traffic_light_slots();
        self.vehicles.upload_uniforms(&gpu, night);
        // --- camera / globals ---------------------------------------------------
        let aspect = gpu.config.width as f32 / gpu.config.height.max(1) as f32;
        let vp = if self.drive && self.vehicles.player().is_some() { self.cam.proj(aspect) * self.chase.view() } else { self.cam.view_proj(aspect) };
        let frustum = Frustum::from_view_proj(vp);
        self.last_frustum = Some(frustum);
        let occluded = occluder::frame(&self.opt.data_dir, vp, &frustum);
        // (the traffic cars' LOD for this frame, by the main view - vehicles.rs `pick_lods`: its frustum and
        // occluders, its camera's axis and the boom file's CameraNear)
        {
            let (eye, view) = if self.drive && self.vehicles.player().is_some() { (self.chase.pos, self.chase.view()) } else { (self.cam.pos, self.cam.view()) };
            let axis = -Vec3::new(view.x_axis.z, view.y_axis.z, view.z_axis.z);
            self.vehicles.pick_lods(eye, axis, self.lod_near, crate::props::Lod::now(), &frustum, &occluded);
            self.peds.pick_lods(eye, axis, self.lod_near, crate::props::Lod::now(), &frustum, &occluded);
            // (the original's renderer flag of F-31789, set while the game is paused: the plants' model angle x 0.9)
            crate::props::set_paused(self.game.mode == Mode::Paused);
        }
        // (the main view's passes have pix_center 0: half a scene pixel right and down - render::half_pixel; its
        // globals' inverse goes with it, so what reads the depth by screen place finds the same points)
        let vp_px = render::half_pixel(vp, gpu.scene_w as f32, gpu.scene_h as f32);
        let mut globals = self.lighting(vp_px);
        // The garage casts the cars' shadows too (its frames draw the four cascades and the collector,
        // work/lighting/cap/gar_*): straight down (the garage's light, TODGarage's orbit), with the
        // collector's c63.z = (1 - m_ShadowFade) / 2 of the garage's keys - the port's night path
        // would leave the strength 0. (MCLA_GARAGE_SHADOW=0: none, as before.)
        let gar_light = self.garage_light().filter(|_| std::env::var("MCLA_GARAGE_SHADOW").map_or(true, |v| v.trim() != "0"));
        if let Some((_, gt)) = gar_light.as_ref() {
            globals.shadow_params[0] = gt.shadow_fade.clamp(0.0, 1.0);
        }
        // sun shadow map: an orthographic box around the area in front of the camera
        let shadow_on = globals.shadow_params[0] > 0.0 && !self.opt.no_shadows && !map3d && (self.game.mode != Mode::Garage || gar_light.is_some());
        if map3d {
            // from the air: only a thin haze
            let d = self.cam.pos.distance(self.chase.look);
            globals.params[0] = d * 0.9;
            globals.params[1] = d * 6.0 + 6000.0;
            globals.fog_color[3] = globals.fog_color[3].min(0.45);
            // (the original's GPS frame has no distance fog over the city: its fog is the height fog
            // pass, PSBlendedHeightFogAboveFogNoPFog, which leaves a view from 1-9 km clear;
            // MCLA_MAP_FOG=1: the port's thin haze)
            if !map_baked && std::env::var("MCLA_MAP_FOG").is_err() {
                globals.fog_color[3] = 0.0;
            }
        }
        self.map_vp = if map3d { Some(vp) } else { None };
        let light_vp = {
            let sun = Vec3::new(globals.sun_dir[0], globals.sun_dir[1], globals.sun_dir[2]).normalize();
            // The game does not cast its shadows along the light (F-70200, F-16780): between 5:00
            // and 18:00 the x of the shadows' direction is the light's at 9 + (h - 5) / 2 h, y and
            // z the light's own (`sky::shadow_ray`) - the eleven hours read off the cascades'
            // matrices to 0.07 degrees (test `shadow_ray_matches_the_recorded_cascades`). Shadows
            // are shorter than the light is low: 32 degrees up at 06:15 for a light at 18.
            // The weathers' light directions are summed by weight without normalising (as the
            // manager does), at every hour of the window whatever the night fade. Outside 5 .. 18 h:
            // the light itself by day; the night path (nfl > 0) keeps its own light.
            // In the garage with a cine scene running: F-16780's offset (`sky::shadow_ray_cutscene`).
            // (MCLA_SHADOW_L: along the light.)
            let gar = self.garage_light();
            let hour = gar.as_ref().map_or(self.time_of_day, |g| g.0.hour);
            let to_light_sum = |h: f32| -> Option<Vec3> {
                let a = self.sky.tod(h)?.to_light;
                Some(match self.sky_next.as_ref() {
                    Some((next, _, f)) => next.tod(h).map_or(a, |b| a.lerp(b.to_light, *f)),
                    None => a,
                })
            };
            let sun = if std::env::var("MCLA_SHADOW_L").is_ok() {
                sun
            } else if let Some((real, at)) = sky::shadow_hour(hour).and_then(|h| Some((to_light_sum(hour)?, to_light_sum(h)?))) {
                sky::shadow_ray(real, at.x)
            } else if self.sky_now().tod.map_or(true, |t| t.nfl > 0.0) {
                sun
            } else {
                to_light_sum(hour).map_or(sun, |r| r.normalize_or(sun))
            };
            let sun = match gar_light.as_ref() {
                Some((_, gt)) => gt.to_light,
                None => sun,
            };
            let sun = if gar.is_some() && self.cine.playing.is_some() { sky::shadow_ray_cutscene(sun) } else { sun };
            // (the direction the frame is drawn in: in a car that is the chase camera's, not the
            // free camera's - which stood still behind it and left the box beside the view)
            let fwd = if self.drive && self.vehicles.player().is_some() { self.chase.look - self.chase.pos } else { self.cam.forward() };
            let fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::Z);
            let eye = self.vehicles.player().filter(|_| self.drive).map(|c| c.state.pos).unwrap_or(self.cam.pos);
            // (diagnostics: MCLA_SHADOW_EXTENT, half the box's side in metres)
            let extent = std::env::var("MCLA_SHADOW_EXTENT").ok().and_then(|v| v.parse().ok()).unwrap_or(95.0f32);
            let center = eye + fwd * (extent * 0.55);
            camera::shadow_view_proj(center, sun, extent, render::SHADOW_SIZE)
        };
        globals.light_vp = light_vp.to_cols_array_2d();
        globals.shadow_params = [if shadow_on { globals.shadow_params[0] } else { 0.0 }, 1.0 / render::SHADOW_SIZE as f32, globals.shadow_params[2], 0.0];
        gpu.queue.write_buffer(&gpu.shadow_buf, 0, bytemuck::bytes_of(&globals.light_vp));
        // The original's four shadow cascades (render/mod.rs `Gpu::set_cascades`; drawn after
        // the single map below, which the fast collector and the plants' masks still use):
        // down the same rays, fitted to this frame's view. ON by default since 2026-10-14: with the
        // tone map's encode and the textures' decoding right the frames are equal or nearer with
        // them at every day hour (07:00 2.6 -> 2.3, 09:00 1.7 -> 1.4). MCLA_CASCADES=0 gives the
        // single map alone; MCLA_DEBUG_VIEW=67 is the single map again, for A/B.
        let casc_vps = {
            let inv = vp.inverse();
            let eye = Vec3::new(globals.cam_pos[0], globals.cam_pos[1], globals.cam_pos[2]);
            let mid = inv.project_point3(Vec3::new(0.0, 0.0, 0.5));
            let (up, side) = (inv.project_point3(Vec3::new(0.0, 1.0, 0.5)) - mid, inv.project_point3(Vec3::new(1.0, 0.0, 0.5)) - mid);
            let depth = (mid - eye).length().max(1e-6);
            let ray = light_vp.row(2).truncate().normalize_or(Vec3::NEG_Y);
            let ray = if ray.y > 0.0 { -ray } else { ray };
            let on = shadow_on && std::env::var("MCLA_CASCADES").map_or(true, |v| v.trim() != "0") && std::env::var("MCLA_DEBUG_VIEW").map_or(true, |v| v.trim() != "67");
            // (F-34053: the garage's lighting, or a cutscene that has launched character / car events - `Scene::close_shadows`)
            let close = self.garage_light().is_some() || self.cine.playing.as_ref().map_or(false, |sc| sc.close_shadows());
            let target = if close { [0.3f32, 0.8] } else { [1.0, 1.0] };
            for (p, t) in self.casc_scale.iter_mut().zip(target) {
                let step = (t - *p).abs().max(0.2) * 1.0 * dt;
                *p = if *p < t { (*p + step).min(t) } else { (*p - step).max(t) };
            }
            let gate = gar_light.as_ref().map_or(Vec3::new(globals.sun_dir[0], globals.sun_dir[1], globals.sun_dir[2]), |(_, gt)| gt.to_light);
            // (ShaderGlobals.z, F-45287: 8 + (1 - ShadowFade), plus 16 while a cine scene runs under the garage's lighting
            // (F-22850 and the renderer's +393) and 1 - ShadowFade <= 0.95; the floor and the interior walls then take
            // their z >= 23 forms - city.wgsl `z23`)
            let z23 = gar_light.as_ref().filter(|(_, gt)| self.cine.playing.is_some() && 1.0 - gt.shadow_fade <= 0.95).map_or(0.0, |(_, gt)| 1.0 + (1.0 - gt.shadow_fade));
            gpu.set_cascades(on, eye, (mid - eye) / depth, up.normalize_or(Vec3::Y), side.normalize_or(Vec3::X), up.length() / depth, side.length() / up.length().max(1e-6), ray, self.casc_scale, gate.extend(z23))
        };
        // the fast shadow collector's map is kept up while the sun casts shadows (its pass
        // follows the shadow pass below): where it lies, for the shaders that read it
        let fc_cam = if self.drive && self.vehicles.player().is_some() { self.chase.pos } else { self.cam.pos };
        if shadow_on {
            globals.collector = [fc_cam.x.trunc(), fc_cam.z.trunc(), 1.0, 0.0];
        }
        if let Ok(v) = std::env::var("MCLA_DEBUG_VIEW") {
            globals.params[3] = v.parse().unwrap_or(0.0);
        }
        if self.frame_no == 5 {
            log::debug!("lighting: sun_dir {:?} sun_color {:?} amb_sky {:?} amb_ground {:?} zen {:?}", globals.sun_dir, globals.sun_color, globals.ambient_sky, globals.ambient_ground, globals.sky_zenith);
        }
        // the environment cube map's cameras: six square 90 degree views from just above the
        // player's car (cube maps are left-handed: the picture is mirrored left to right)
        let env_on = self.drive && !map3d && self.vehicles.player().is_some() && std::env::var("MCLA_NO_ENV").is_err();
        // (the original's map is taken from exactly 2 m above the car's origin - c15 of its
        // environment passes and c51 of its sky, 819.61 19.572 -808.409 with the car at 17.572,
        // at every hour recorded; 1.1 m here until 2026-10-14. MCLA_ENV_EYE=<m> for A/B.)
        let env_up = std::env::var("MCLA_ENV_EYE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(2.0);
        let env_eye = self.vehicles.player().map_or(self.cam.pos, |c| c.render_pose(self.vehicles.render_alpha).pos + Vec3::Y * env_up);
        // Only one face is rendered this frame. Avoid allocating two vectors and
        // building five unused view matrices/frusta, including when env is off.
        let env_face = (self.frame_no % 6) as usize;
        let env_frustum = if env_on {
            let mut ec = self.cam.clone();
            ec.fov_y = std::f32::consts::FRAC_PI_2;
            let proj = glam::Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)) * ec.proj(1.0);
            let (dir, up) = [(Vec3::X, Vec3::Y), (Vec3::NEG_X, Vec3::Y), (Vec3::Y, Vec3::NEG_Z), (Vec3::NEG_Y, Vec3::Z), (Vec3::Z, Vec3::Y), (Vec3::NEG_Z, Vec3::Y)][env_face];
            let env_vp = proj * glam::Mat4::look_to_rh(env_eye, dir, up);
            let mut eg = globals;
            eg.view_proj = env_vp.to_cols_array_2d();
            eg.inv_view_proj = env_vp.inverse().to_cols_array_2d();
            eg.cam_pos = [env_eye.x, env_eye.y, env_eye.z, globals.cam_pos[3]];
            // (tone.w = 2: the night programs of the environment map - PSMultiLightEnv - which light
            // far more simply than the main view's)
            if eg.tone[3] > 0.5 {
                eg.tone[3] = 2.0;
            }
            // (no sun shadows in the map: its day programs have no shadow map - PSCityBumpSpecEnv
            // samples the diffuse texture alone and lights by ambient + sun x n.l - and the
            // original's map, read out of the game at 17:30 and noon, shows the street evenly
            // lit where the port's had the houses' and the car's shadows across it.
            // MCLA_ENV_SHADOWS=1: with them, as until 2026-10-14.)
            if std::env::var("MCLA_ENV_SHADOWS").is_err() {
                eg.shadow_params[0] = 0.0;
            }
            gpu.queue.write_buffer(&gpu.env_globals[env_face], 0, bytemuck::bytes_of(&eg));
            // (the paraboloid half drawn this frame - city.wgsl vs_env_dp: moon_dir.w = 1 the half
            // looking along the world's -z, 2 the other)
            let dp_half = (self.frame_no % 2) as usize;
            eg.moon_dir[3] = 1.0 + dp_half as f32;
            gpu.queue.write_buffer(&gpu.env_globals[6 + dp_half], 0, bytemuck::bytes_of(&eg));
            globals.shadow_params[3] = 1.0;
            Some(Frustum::from_view_proj(env_vp))
        } else {
            None
        };
        // the wet road's planar reflection picture is drawn this frame (after the collector's
        // pass below): collector.w tells the road programs of the main view that it is there
        // (MCLA_DEBUG_VIEW=63: without it - the older stand-in, the cars' map)
        // (water work, 2026-10-14: in the GPS map too - the original's map frame draws the pass, its mirror at the
        // player's height from the map's own camera, and the sea's program reads it; MCLA_MAP_NO_REFL=1: not there, as before)
        let refl_on = (!map3d || std::env::var("MCLA_MAP_NO_REFL").is_err()) && (globals.params[3] - 63.0).abs() > 0.5 && std::env::var("MCLA_NO_REFL").is_err();
        if refl_on {
            globals.collector[3] = 1.0;
        }
        gpu.write_globals(&globals);
        // the cars' lamp glows (lights are on from dusk to dawn)
        {
            let lamp_night = if self.time_of_day > 18.0 || self.time_of_day < 6.0 { 1.0 } else { 0.0 };
            let eye = if self.drive && self.vehicles.player().is_some() { self.chase.pos } else { self.cam.pos };
            self.vehicles.update_glows(&gpu, lamp_night, eye);
            self.props.update_signal_glows(&gpu, eye, lamp_night);
        }
        // The lamps' glow pass, which the original draws after its tone map (halo.rs; LIGHTING_WORK.md
        // "The lamps' glow pass after the tone map"): a seed and three star quads for every lit
        // prop lamp - the list the cars' map shows as glows, the same props out to the same
        // distance - and for every lit signal lamp. A lamp is lit after 17.9 - o and before
        // 6.1 + o o'clock, o = one of sixteen steps of a minute a lamp (F-28456: bits 4..7
        // of the prop's address x 1 / 960 h - here a hash of its place; recorded: none at 17:30,
        // 26 of 70 at 17:48, all from 17:57 to 06:00, 12 at 06:18, none at 06:30 or at noon in
        // any weather). The cars' head, tail, brake
        // and reversing lamps too (vehicles.rs `halo_lamps`); police lights are still sprites.
        // (the game's list of lamp props, one for the glow pass and the cars' map: halo::ListView)
        let lamp_list = halo::ListView::new(glam::Mat4::from_cols_array_2d(&globals.view_proj), self.cam.pos, self.cam.forward(), self.cam.fov_y);
        {
            let mut glows: Vec<(f32, halo::Glow)> = Vec::new();
            // (the same lamps' rectangles in the road's reflection picture: siglrefl.rs)
            let mut sig_refl: Vec<siglrefl::Rect> = Vec::new();
            if let (Some(mut tune), false, false) = (halo::tune(), map_baked || matches!(self.game.mode, Mode::Garage), halo::old_look()) {
                let w = match self.weather { sky::Weather::Cloudy => 1, sky::Weather::Stormy => 2, sky::Weather::Foggy => 3, _ => 0 };
                // (StarSize is the one glow member the light manager blends by the weather weights, F-86405:
                // a blend in progress moves it smoothly instead of at the blend's end)
                if let Some((_, nw, f)) = self.sky_next.as_ref() {
                    let nw = match nw { sky::Weather::Cloudy => 1, sky::Weather::Stormy => 2, sky::Weather::Foggy => 3, _ => 0 };
                    let (a, b) = (tune.star[w], tune.star[nw]);
                    tune.star[w] = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
                }
                let cam = self.cam.pos;
                // (a light whose place is off the screen has no quads - both vertex programs - and
                // the game's list holds 96: keep the list for those on it)
                let vp = glam::Mat4::from_cols_array_2d(&globals.view_proj);
                let on_screen = |g: &halo::Glow| {
                    let c = vp * glam::Vec4::new(g.pos_a[0], g.pos_a[1], g.pos_a[2], 1.0);
                    c.w > 0.0 && c.x.abs() <= c.w && c.y.abs() <= c.w
                };
                let hour = self.time_of_day;
                if hour > 17.6 || hour < 6.4 {
                    let no_occ = globals.params[3] == 118.0;
                    for (((p, c, _), bx), (bc, bw, br, org)) in self.props.glow_lamps.iter().zip(self.props.glow_boxes.iter()).zip(self.props.glow_cull.iter()) {
                        // (the occlusion test belongs to the rule's second branch alone - large
                        // props 200 .. 250 m off; MCLA_DEBUG_VIEW=118: without it)
                        if !halo::lamp_lit(*org, hour) || !lamp_list.holds(*bc, *bw, *br, || !no_occ && occluded.hides_aabb(bx.0, bx.1)) {
                            continue;
                        }
                        if let Some(g) = tune.prop(w, *p, *c, cam) {
                            glows.push((lamp_list.order(*org) as f32, g));
                        }
                    }
                }
                let tan = (self.cam.fov_y * 0.5).tan();
                let mut head_ok: Option<(u32, bool)> = None;
                for (p, z, c, h) in self.props.signal_lamps_lit() {
                    // (a head is listed as any prop - halo::ListView::holds with its own box and radius - and
                    // then F-79347 tests it once: a 2 m sphere round its origin in the main view (the near
                    // clause, 300 m, never decides inside the list's reach), or within 20 m)
                    if head_ok.map_or(true, |(k, _)| k != *h) {
                        let (o, bc, bw, br, bx) = self.props.signal_head_cull(*h);
                        let ok = lamp_list.holds(bc, bw, br, || occluded.hides_aabb(bx.0, bx.1)) && (lamp_list.sphere_in_view(o, 2.0) || o.distance_squared(cam) <= 400.0);
                        head_ok = Some((*h, ok));
                    }
                    if !head_ok.map_or(false, |(_, ok)| ok) {
                        continue;
                    }
                    if let Some(g) = tune.signal(w, *p, *z, *c, cam, tan) {
                        glows.push((lamp_list.order(*p) as f32, g));
                    }
                    if let (true, Some((rp, ri, rs))) = (sig_refl.len() < siglrefl::MAX, tune.signal_reflection(w, *p, *z, cam)) {
                        let oy = self.props.signal_head_cull(*h).0.y;
                        let size = render::REFL_SIZE as f32;
                        if let Some(r) = siglrefl::rect(vp, self.lod_near, oy, rp, cam, *c, ri, rs, (size, size)) {
                            if (self.frame_no == 60 || self.frame_no == 280) && std::env::var("MCLA_GLOW_DUMP").is_ok() { eprintln!("siglrefl P {:.3} {:.3} {:.3} origin y {:.3} I {:.4} streak {:.4} near {:.3} rect {:.4} {:.4} {:.4} {:.4} depth {:.6} col {:.3} {:.3} {:.3}", rp.x, rp.y, rp.z, oy, ri, r.z[1], self.lod_near, r.r[0], r.r[1], r.r[2], r.r[3], r.z[0], c[0], c[1], c[2]); }
                            sig_refl.push(r);
                        }
                    }
                }
                // (the cars' lamps: lights on as the sprites had them)
                let lamp_night = if self.time_of_day > 18.0 || self.time_of_day < 6.0 { 1.0 } else { 0.0 };
                for (p, f, c, k) in self.vehicles.halo_lamps(lamp_night, cam) {
                    if (self.frame_no == 60 || self.frame_no == 280) && std::env::var("MCLA_GLOW_DUMP").is_ok() { eprintln!("carlamp {:.3} {:.3} {:.3} facing {:.3} {:.3} {:.3} col {:.3} {:.3} {:.3} k {:.3}", p.x, p.y, p.z, f.x, f.y, f.z, c[0], c[1], c[2], k); }
                    if let Some(g) = tune.car(w, p, f, c, k, cam, tan) {
                        // (ahead of the props for the cut at 96: the port's prop list is longer than the
                        // game's - 74 props at the gate at 23:30, the car's four lamps at 74 .. 77 of 80 -
                        // and a list of over 96 props dropped every car lamp)
                        glows.push((-1.0e6 + p.distance(cam), g));
                    }
                }
                // (the police lamps: their own AddGlow calls - vehicles.rs `police_glows`, halo.rs `Tune::police`)
                for (p, f, c, k) in self.vehicles.police_glows(cam) {
                    if let Some(g) = tune.police(w, p, f, c, k, cam) {
                        glows.push((-1.0e6 + p.distance(cam), g));
                    }
                }
                // (the list in the game's order - the grid's spiral from the camera's cell, cars
                // after the props here - cut at 96 as AddGlow does; then those off the screen go)
                glows.sort_by(|a, b| a.0.total_cmp(&b.0));
                if (self.frame_no == 60 || self.frame_no == 280) && std::env::var("MCLA_GLOW_DUMP").is_ok() { eprintln!("halo list before the cut: {}", glows.len()); }
                glows.truncate(halo::MAX);
                glows.retain(|g| on_screen(&g.1));
                if (self.frame_no == 60 || self.frame_no == 280) && std::env::var("MCLA_GLOW_DUMP").is_ok() {
                    for (d, g) in &glows { eprintln!("halo {:.2} {:.2} {:.2} d {:.1} seed {:.3} star {:.3} col {:.2} {:.2} {:.2} size {:.2} {:.2}", g.pos_a[0], g.pos_a[1], g.pos_a[2], d, g.pos_a[3], g.col_s[3], g.col_s[0], g.col_s[1], g.col_s[2], g.size[0] * 10.0, g.size[1] * 10.0); }
                }
            }
            halo::set(&gpu, &glows.iter().map(|g| g.1).collect::<Vec<_>>());
            siglrefl::set(&gpu, &sig_refl);
        }
        {
            // local lights: headlights of the cars around the camera + nearest street lamps (night only)
            // the player's lamp table is its garage record's - the save's copy of the set-up it was
            // made from (bought: `<car>.xcc`, a prize: the reward's set-up) - as the original draws it
            self.vehicles.player_lamps = self.vehicles.player().and_then(|p| {
                let rec = self.game.mc4.as_ref()?.car(&p.model.name)?;
                let v3 = |v: &[mc4::F]| [0, 1, 2].map(|k| v.get(k).map_or(0.0, |x| x.0 as f32));
                let rows: Vec<_> = rec.visual.lights.iter().map(|l| (v3(&l.LightBulbColor), v3(&l.LightCoverColor))).collect();
                render::car_lamps::LampRows::from_record(&rows).map(std::sync::Arc::new)
            });
            let mut ls = render::LightSet::default();
            let dark = 1.0 - globals.sky_zenith[3];
            let mut k = 0usize;
            // the player's neon (the garage's exterior neon): its colour on the road under
            // the car, front and rear
            // (MCLA_NEON=1: on for screenshots)
            if let (true, Some(c)) = (self.game.neon_on || std::env::var("MCLA_NEON").is_ok(), self.vehicles.player()) {
                let col = self.game.neon_linear();
                let gain = 6.0 + 16.0 * dark;
                for off in [-1.0f32, 1.0] {
                    let pose = c.render_pose(self.vehicles.render_alpha);
                    let p = pose.pos + pose.rot * Vec3::NEG_Z * off + pose.rot * Vec3::Y * 0.42;
                    ls.pos_range[k] = [p.x, p.y, p.z, 3.0];
                    ls.color[k] = [col[0] * gain, col[1] * gain, col[2] * gain, -1.0];
                    k += 1;
                }
            }
            // (under the weather's keys the player's head lamps light the world whenever the night
            // programs run - from 18:00 to 06:00, at full strength at 18:15 already - not only
            // once the sky has gone dark)
            // (not in the garage - garage round, 2026-10-13: the night programs' two head-lamp lights are
            // there, at their places ahead of the car with their cones, but their COLOURS are zero:
            // gMultiLightDynamicLightData c84 .. c86 = (0, 0, 0.0196, 0) in every garage capture, the
            // 0.0196 being the key light's column)
            if (dark > 0.3 || globals.tone[3] > 0.5) && !self.vehicles.lights_off && globals.car_amb[3] < 0.5 {
                let focus = self.vehicles.player().map(|c| c.state.pos).unwrap_or(self.cam.pos);
                let mut cars: Vec<(f32, Vec3, Vec3)> = self.vehicles.cars.iter().filter(|c| c.parked.is_none()).map(|c| { let p = c.render_pose(self.vehicles.render_alpha); (p.pos.distance(focus), p.pos, p.rot * Vec3::NEG_Z) }).collect();
                cars.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                // With the weather's keys (the original's night programs): its dynamic lights are
                // the PLAYER's two head lamps only - the car's own colour (R8: 1.85, 2.39, 3.0), sat(500 / d squared),
                // a cone from 30 to 50 degrees about an axis 42 degrees below the car's nose
                // (PSMultiLight c84..c95, measured at 23:30) - and the city's lamps come from the
                // lamp grid, not from this list.
                let original_night = globals.tone[3] > 0.5;
                if std::env::var("MCLA_TOD_DUMP").is_ok() && self.frame_no == 60 {
                    eprintln!("head lamps: night programs {} dark {:.2} player {:?}", original_night, dark, self.vehicles.player().map(|c| (c.parked.is_some(), c.state.pos)));
                }
                if original_night {
                    if let Some(c) = self.vehicles.player().filter(|c| c.parked.is_none()) {
                        let pose = c.render_pose(self.vehicles.render_alpha);
                        let (fwd, right, up) = (pose.rot * Vec3::NEG_Z, pose.rot * Vec3::X, pose.rot * Vec3::Y);
                        // (comparison aid, tunnel round: MCLA_HEADLAMP_CAR = x,y,z,yaw puts the two lamps at a level car
                        // standing there - the pair scripts' car cannot be placed inside a tunnel and stands on the street
                        // above it, from where its lamps shone down through the ground onto the tunnel's road)
                        let hl_at: Option<Vec<f32>> = std::env::var("MCLA_HEADLAMP_CAR").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).filter(|v: &Vec<f32>| v.len() == 4);
                        let (hl_pos, fwd, right, up) = match &hl_at {
                            Some(v) => { let r = v[3].to_radians(); let f = Vec3::new(-r.sin(), 0.0, -r.cos()); (Vec3::new(v[0], v[1], v[2]), f, f.cross(Vec3::Y), Vec3::Y) }
                            None => (pose.pos, fwd, right, up),
                        };
                        // (41.8 degrees below the car's own nose: the game's direction at the garage gate
                        // is 44.1 degrees below the horizon while the car stands there 2.3 degrees nose
                        // down - the far edge of the cone decides how much of it reaches the walls ahead)
                        let dir = (fwd * 0.7455 - up * 0.6665).normalize();
                        // (their colour is the car's own: 3 x the head lamp's bulb x cover of its set-up's
                        // LightTune - R8 (1.85, 2.39, 3.0), Saleen S281 (2.16, 2.06, 1.86); the same at every hour)
                        // (held HEADLIGHTS turns it towards the high beam's: the flash state, F-19475)
                        let fl = c.amb_lamps.s[3];
                        let hl_col = self.vehicles.lamp_rows(c).map_or_else(|| render::car_lamps::R8.head_light_flash(fl), |t| t.head_light_flash(fl));
                        // (and while it is held - the flash TARGET, not the eased state - F-43427 adds them with twice
                        // the strength and a cone 1.2 times as wide: `car_lamps::head_lamp_cone`; dir.w carries it to the
                        // programs. MCLA_FLASH_CONE_OLD=1: as before, the colour only)
                        let flash_t = if self.vehicles.player_flash && std::env::var("MCLA_FLASH_CONE_OLD").is_err() { 1.0 } else { 0.0 };
                        for side in [-1.0f32, 1.0] {
                            // (where the game puts them, from the lamps' positions in c88..c90 against the
                            // car's own matrix in two captures, fix_night and car_n: 3.2 m ahead of the
                            // car's origin - a metre beyond the nose -, 0.72 m to each side, 0.57 m up)
                            let p = hl_pos + fwd * 3.2 + right * (0.72 * side) + up * 0.57;
                            if std::env::var("MCLA_TOD_DUMP").is_ok() && (self.frame_no == 60 || (flash_t > 0.0 && self.frame_no % 60 == 0)) { eprintln!("head lamp at {:?} dir {:?} colour {:?} flash {} cone {:?}", p, dir, hl_col, flash_t, render::car_lamps::head_lamp_cone(flash_t > 0.5)); }
                            ls.pos_range[k] = [p.x, p.y, p.z, 120.0];
                            ls.color[k] = [hl_col[0], hl_col[1], hl_col[2], 2.0];
                            ls.dir[k] = [dir.x, dir.y, dir.z, flash_t];
                            k += 1;
                        }
                    }
                }
                for (d, pos, fwd) in cars.into_iter().take(if original_night { 0 } else { 8 }) {
                    if d > 150.0 {
                        break;
                    }
                    let dir = (fwd - Vec3::Y * 0.12).normalize();
                    let p = pos + fwd * 1.6 + Vec3::Y * 0.75;
                    ls.pos_range[k] = [p.x, p.y, p.z, 58.0];
                    ls.color[k] = [1.2 * dark, 1.17 * dark, 1.05 * dark, 0.86];
                    ls.dir[k] = [dir.x, dir.y, dir.z, 0.0];
                    k += 1;
                }
                for (pos, col) in self.police.light_bar(&self.vehicles).into_iter().take(6) {
                    ls.pos_range[k] = [pos.x, pos.y + 0.3, pos.z, 18.0];
                    ls.color[k] = [col[0] * 1.5, col[1] * 1.5, col[2] * 1.5, -1.0];
                    k += 1;
                }
                for (pos, range, col) in self.props.lights_near(focus, if original_night { 0 } else { render::MAX_LIGHTS.saturating_sub(k) }) {
                    ls.pos_range[k] = [pos.x, pos.y, pos.z, range * 1.25];
                    ls.color[k] = [col.x * 1.6 * dark, col.y * 1.6 * dark, col.z * 1.6 * dark, -1.0];
                    k += 1;
                }
            }
            // the race checkpoint's flickering light: the night programs' fourth dynamic light
            // (checkpoints.rs `Markers::light`; colour.w 3 = all round, its strength in dir.w)
            if globals.tone[3] > 0.5 && globals.car_amb[3] < 0.5 && k < render::MAX_LIGHTS && std::env::var("MCLA_NO_MARKERS").is_err() {
                if let Some((p, c)) = self.cp_markers.light {
                    ls.pos_range[k] = [p.x, p.y, p.z, 1000.0];
                    ls.color[k] = [c[0], c[1], c[2], 3.0];
                    ls.dir[k] = [0.0, -1.0, 0.0, checkpoints::LIGHT_INTENSITY];
                    k += 1;
                }
            }
            ls.count[0] = k as u32;
            gpu.write_lights(&ls);
            // (the walkers' picker at night - figlight.rs: the head lamps as the light set has them
            // and the key light; by day, in the garage and without the weather's keys none)
            let hour = globals.sun_color[3];
            self.peds.night_lights = ((hour > 18.0 || hour < 6.0) && globals.tone[3] > 0.5 && globals.car_amb[3] < 0.5).then(|| {
                let mut heads = [None; 2];
                for (h, i) in (0..k).filter(|&i| ls.color[i][3] > 1.5 && ls.color[i][3] < 2.5).take(2).enumerate() {
                    let (p, c, d) = (ls.pos_range[i], ls.color[i], ls.dir[i]);
                    // (the flash: dir.w, F-43427's strength x 2 and cone x 1.2)
                    let (strength, cone) = if d[3] > 0.5 { let (s, k, b) = render::car_lamps::head_lamp_cone(true); (s, [k, b]) } else { (500.0, [4.4696, -2.872]) };
                    heads[h] = Some(render::LampRec { pos: Vec3::new(p[0], p[1], p[2]), color: Vec3::new(c[0], c[1], c[2]), strength, back: -Vec3::new(d[0], d[1], d[2]), cone });
                }
                let key = Some((Vec3::new(globals.key_col[0], globals.key_col[1], globals.key_col[2]), Vec3::new(globals.key_dir[0], globals.key_dir[1], globals.key_dir[2])));
                (figlight::Dynamic { heads, key }, dt * 30.0)
            });
        }
        // (around the city map: the dark of deep water)
        let clear = if map_baked { [0.012, 0.018, 0.028, 1.0] } else { globals.fog_color };
        let t_sim = now.elapsed().as_secs_f32() * 1000.0;

        // (RENDER SCALE: the scene at window x scale, filtered down by the post chain - render/post.rs)
        gpu.set_render_scale(crate::hooks::render_scale());
        let frame = match gpu.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                let (w, h) = (gpu.config.width, gpu.config.height);
                gpu.resize(w, h);
                self.world = Some(world);
                self.gpu = Some(gpu);
                return;
            }
            Err(e) => {
                log::error!("surface: {:?}", e);
                self.world = Some(world);
                self.gpu = Some(gpu);
                return;
            }
        };
        let t_acq = now.elapsed().as_secs_f32() * 1000.0;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(gpu.config.format), ..Default::default() });
        // The UI blends in gamma space, as the original does: through an sRGB view a
        // 30% white pane over a dark scene comes out about twice as bright and a 77%
        // black one visibly thinner (measured against the reference captures).
        let ui_view = frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(gpu.config.format.remove_srgb_suffix()), ..Default::default() });
        let mut enc = gpu.device.create_command_encoder(&Default::default());

        // --- egui ---------------------------------------------------------------
        let mut egui_jobs = None;
        let rain_now = self.rain_amount();
        // the police radar (F-61846): the nearest police car to the player - a payback goon is not one (+872),
        // an underground unit has no car up in the port (+880) - from the UILogic tune; 0 with none
        let radar_now = {
            let [near, falloff, exponent] = self.game.ui_shared.as_ref().map_or([40.0, 140.0, 1.0], |s| s.logic.radar);
            let pp = self.vehicles.player().map(|c| c.state.pos);
            let d = pp.and_then(|pp| self.vehicles.cars.iter().filter(|c| c.cop && !c.is_player).map(|c| c.state.pos.distance(pp)).reduce(f32::min));
            d.map_or(0.0, |d| (1.0 - ((d - near).max(0.0) / falloff).min(1.0)).powf(exponent))
        };
        if let Some(eg) = self.egui.as_mut() {
            let mut raw = eg.state.take_egui_input(&window);
            // (--fast: the UI's clock steps as the game's, so the movies' animations repeat run for run)
            if self.opt.fixed_dt > 0.0 {
                raw.time = None;
                raw.predicted_dt = self.opt.fixed_dt;
            }
            let fps = self.fps;
            let ms = self.frame_ms;
            let loaded = world.loaded.len();
            let pending = world.pending.len();
            let tris = world.stats_tris;
            let pos = self.cam.pos;
            let draws = self.draws;
            let tod = self.time_of_day;
            let ntex = gpu.textures.len();
            let show = self.show_ui;
            let drive = self.drive;
            let ncars = self.vehicles.cars.len();
            let car_hud = self.vehicles.player().map(|c| (c.state.fwd_speed() * 3.6, c.state.gear, c.state.rpm, c.state.nitro, c.state.damage, c.state.pos, c.state.wheels.iter().filter(|w| w.grounded).count()));
            // (the garage's SPECIAL ABILITIES page chose one)
            if let Some(k) = self.game.ability_pick.take() {
                self.ability.kind = crate::ability::Kind::ALL[k.min(3)];
            }
            let hud = self.vehicles.player().map(|c| HudInfo { damage_goal: self.game.goal_attack && self.game.race.is_some(), speed_kmh: c.state.fwd_speed() * 3.6, speed_abs: c.state.speed(), gear: c.state.gear, rpm: self.sound.tach_rpm().unwrap_or(c.state.rpm), redline: c.model.params.redline, nitro: (c.state.nitro_shots as f32 + c.state.veh.as_ref().map_or(0.0, |v| v.nitro_level)) / 4.0, damage: if self.game.race.as_ref().map_or(false, |r| r.kind == game::RaceKind::Delivery) { c.state.delivery_damage() } else { c.state.damage }, player_pos: c.state.pos, player_heading: { let f = c.state.forward(); f.x.atan2(-f.z) }, heat: std::env::var("MCLA_HUD_HEAT").ok().filter(|_| self.tick_no > 400).and_then(|v| v.parse().ok()).unwrap_or(if self.police.goons { 0 } else { self.police.heat as u8 }), fine: if self.police.goons { 0 } else { self.police.fine as i64 }, law: self.police.level(), search: self.police.search / self.police.tun.get("fSearchMaxTime", 35.0), rain: rain_now, radar: radar_now, ability: if (self.game.mc4_ability_none() || (self.game.career.loaded() && !self.game.career.reached("INSTALL_ABILITY"))) && std::env::var("MCLA_ABILITY").is_err() { "" } else { self.ability.kind.name() }, ability_meter: self.ability.meter, ability_fx: self.ability.fx(), draft: c.state.veh.as_ref().map_or(0.0, |v| v.sst.strength), sst: c.state.veh.as_ref().map_or(0.0, |v| v.sst.level), sst_full: c.state.veh.as_ref().map_or(false, |v| v.sst.full && !v.sst.burning), sst_on: vehicles::Vehicles::has_sst(c), sst_lit: self.game.tutorial_box.as_ref().map_or(false, |t| t.0 == "tutorial_slipstream_title" && t.2 <= 0.0), map_vp: self.map_vp, ability_tint: self.ability.tint().map(|t| (t, self.ability.blend())), cam_vp: Some(vp), cam_pos: self.chase.pos, cam_look: self.chase.look, cam_fwd: { let f = self.chase.look - self.chase.pos; Vec3::new(f.x, 0.0, f.z).normalize_or_zero() }, cam_vel: c.state.vel, night: self.time_of_day > 18.0 || self.time_of_day < 6.0, /* d_IsNight = F-47292: light manager +14568 / +14564 */ cruise: self.cruise_hold, pu_hud: self.powerups.as_ref().map_or([0.0, 0.0, 1.0], |p| p.hud(self.vehicles.cars.iter().position(|x| x.is_player).unwrap_or(0))) }).unwrap_or_default();
            let mut action = MenuAction::None;
            let game = &mut self.game;
            let vehicles = &self.vehicles;
            let world_ref = &world;
            let tu0 = Instant::now();
            let out = eg.ctx.run(raw, |ctx| {
                action = game.ui(ctx, vehicles, world_ref, hud);
                // (the port's FPS counter, over everything but a photo: hooks.rs)
                hooks::draw_fps_monitor(ctx, game.art.as_ref().and_then(|a| a.font.as_ref()), game.photo_on);
                if let Some(msg) = render::xs::problem() {
                    // (the translated shader files are missing: one notice; the log has the same sentence)
                    egui::Area::new(egui::Id::new("xs_missing")).anchor(egui::Align2::CENTER_TOP, [0.0, 20.0]).order(egui::Order::Foreground).show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_max_width(640.0);
                            ui.colored_label(egui::Color32::from_rgb(255, 120, 100), msg);
                        });
                    });
                }
                if show {
                    egui::Window::new("debug").default_pos([10.0, 10.0]).resizable(false).default_open(false).show(ctx, |ui| {
                        ui.label(format!("{:.0} fps  {:.2} ms", fps, ms));
                        ui.label(format!("sectors {} loaded, {} pending", loaded, pending));
                        ui.label(format!("{} M tris resident, {} draws, {} textures, {} cars", tris / 1_000_000, draws, ntex, ncars));
                        ui.label(format!("pos {:.0} {:.0} {:.0}", pos.x, pos.y, pos.z));
                        ui.label(format!("time {:02}:{:02}   ([ ] to change)", tod as u32, ((tod % 1.0) * 60.0) as u32));
                        ui.label(if drive { "WASD drive, Space handbrake, Shift nitro, R reset, F2 fly cam" } else { "WASD/QE fly, Shift fast, RMB look, F2 drive" });
                        if let Some(c) = car_hud {
                            ui.label(format!("car {:.0} {:.0} {:.0}  wheels down {}  dmg {:.0}%", c.5.x, c.5.y, c.5.z, c.6, c.4 * 100.0));
                        }
                    });
                }
            });
            if action != MenuAction::None {
                self.pending_action = action;
            }
            eg.state.handle_platform_output(&window, out.platform_output);
            let tu1 = Instant::now();
            let jobs = eg.ctx.tessellate(out.shapes, out.pixels_per_point);
            let tu2 = Instant::now();
            for (id, delta) in &out.textures_delta.set {
                eg.renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
                eg.renderer4.update_texture(&gpu.device, &gpu.queue, *id, delta);
                eg.renderer2.update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
            let tu3 = Instant::now();
            // (the menus' 3D-scene movies drawn into their own pictures first - selfmovie.rs)
            selfmovie::render_jobs(&eg.ctx, &gpu.device, &gpu.queue, &mut eg.renderer2, &eg.selfs, gpu.config.format.remove_srgb_suffix());
            // (and the free-roam icons' movies into their 4x targets - flashpic.rs)
            flashpic::render(&gpu.device, &gpu.queue);
            // (and the minimap into the HUD movie's texture of it - minimap_gpu.rs)
            minimap_gpu::render(&gpu.device, &gpu.queue, [&eg.renderer, &eg.renderer4, &eg.renderer2]);
            let tu4 = Instant::now();
            let desc = egui_wgpu::ScreenDescriptor { size_in_pixels: [gpu.config.width, gpu.config.height], pixels_per_point: out.pixels_per_point };
            // (the original's menu flag: the UI pass at 4x while a menu is open - uipass.rs)
            let ui_n = aamode::ui_samples(self.game.ui_msaa4());
            let r = match ui_n { 1 => &mut eg.renderer, 2 => &mut eg.renderer2, _ => &mut eg.renderer4 };
            // (the 3D frame's depth for the GPS map's x-ray callbacks, hudmap_gpu.rs)
            if let Some(x) = r.callback_resources.get_mut::<hudmap_gpu::XrayRes>() {
                x.depth = Some(gpu.depth_ui.clone());
            }
            if let Some(s) = r.callback_resources.get_mut::<selfmovie::SelfRes>() {
                s.new_frame();
            }
            r.update_buffers(&gpu.device, &gpu.queue, &mut enc, &jobs, &desc);
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
            self.fl_ui = [ms(tu0, tu1), ms(tu1, tu2), ms(tu2, tu3), ms(tu3, tu4), ms(tu4, Instant::now())];
            egui_jobs = Some((jobs, desc, out.textures_delta.free, ui_n));
        }

        let t_ui = now.elapsed().as_secs_f32() * 1000.0;
        passprof::mark("setup");
        // The scene's sample count (render/ms.rs): the original's 2x while driving, 4x while its menu flag (+373) or
        // garage flag (+393) is set; the city's sampler at maximum anisotropy 16 in that mode (F-23692), else 4
        // (TEXTURE FILTERING 16X: 16 always).
        let menu4 = self.game.ui_msaa4() || self.garage_light().is_some();
        let scene_n = aamode::scene_samples(menu4);
        gpu.set_scene_samples(scene_n);
        gpu.set_aniso16(menu4 || crate::hooks::aniso_16x());
        render::ms::set_plain();
        // PSStreakMotionBlur's matrices and the cars' stencil ids for this frame (render/mb.rs)
        let mb_mtx = {
            let eye_view = if self.drive && self.vehicles.player().is_some() { self.chase.view() } else { self.cam.view() };
            let proj = glam::Mat4::from_cols_array_2d(&globals.view_proj) * eye_view.inverse();
            let alpha = self.vehicles.render_alpha;
            let cars: Vec<Option<render::mb::Car>> = self
                .vehicles
                .cars
                .iter()
                .map(|c| {
                    (!c.pu_stealth).then(|| {
                        let p = c.render_pose(alpha);
                        render::mb::Car { pos: p.pos, rot: p.rot, vel: c.state.vel, bike: c.model.bike }
                    })
                })
                .collect();
            let still = sim_dt <= 0.0;
            render::mb::frame(eye_view, proj, &cars, dt, self.game.mc4_time_scale() < 1.0, still)
        };
        let mut stat_shadow_sec = 0usize;
        let mut early_cbs: Vec<wgpu::CommandBuffer> = Vec::new();
        let mut stat_shadow = 0usize;
        passprof::mark("shadow+cascades+collector+reflection");
        gpuprof::begin();
        gpuprof::stamp(&gpu.device, &gpu.queue, &mut enc, "pre");
        // --- shadow pass and the four shadow cascades --------------------------------
        // (recorded side by side, each into a command buffer of its own, submitted in this order after what is
        // recorded so far and before the rest: the same passes, in the same order, as one encoder gave -
        // docs/PERFORMANCE.md 2026-10-23 (4); the first tile clears the cascades' map, the others keep it)
        if shadow_on {
            // (the plants' shadow impostors: the silhouette mask that is due - props/impostor.rs)
            self.props.update_shadow_impostors(&gpu, &mut enc, light_vp, self.time_of_day);
        }
        gpuprof::stamp(&gpu.device, &gpu.queue, &mut enc, "shadows");
        early_cbs.push(std::mem::replace(&mut enc, gpu.device.create_command_encoder(&Default::default())).finish());
        {
            let gpu = &gpu;
            // (of the world only its sectors: the rest of it is not shared between threads)
            let (loaded, sets) = (&world.loaded, &world.sets);
            // (in the garage neither the room nor the city round it casts: the original's garage frames draw no set and
            // no sector into its shadow passes - work/lighting/cap/gar_top, gar_cust, gar_rb_top, gar_paint_gloss_blue:
            // the car's 640 VS_CarShadowDepthLocal draws, a few locals and characters, no city program. MCLA_DEBUG_VIEW=716:
            // as before.)
            let room_casts = self.garage_light().is_none() || std::env::var("MCLA_DEBUG_VIEW").map_or(false, |v| v.trim() == "716");
            let sets_cast = || sets.values().filter(move |_| room_casts);
            let loaded_cast = || loaded.values().filter(move |_| room_casts);
            let (vehicles, cine, props, cam_pos) = (&self.vehicles, &self.cine, &self.props, self.cam.pos);
            // (the walkers as the shadow passes have drawn them: last frame's draw over the vertices this frame's
            // `peds.prepare` writes before the submit - prepare now runs beside these passes, below)
            let peds_shadow = self.peds.shadow_part();
            let shadow_job = || -> (wgpu::CommandBuffer, usize, usize) {
                let mut e = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("shadow") });
                let mut n = 0usize;
                let mut sec_n = 0usize;
                {
                    let lf = Frustum::from_view_proj(light_vp);
                    let mut n = 0usize;
                    let mut sec_n = 0usize;
                    let mut sp = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("shadow"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &gpu.shadow_view,
                            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    sp.set_pipeline(&gpu.pipeline_shadow);
                    sp.set_bind_group(0, &gpu.shadow_bg, &[]);
                    sp.set_bind_group(3, &gpu.identity_obj.bg, &[]);
                    for sec in loaded_cast().chain(sets_cast()) {
                        let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
                        let mut bound = false;
                        // every plain opaque caster of the sector in a single draw
                        if sec.caster_count > 0 && lf.intersects_aabb(sec.caster_min, sec.caster_max) {
                            sp.set_vertex_buffer(0, vb.slice(..));
                            sp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                            bound = true;
                            sp.set_bind_group(1, sec.batches[0].bind.as_ref(), &[]);
                            sp.set_bind_group(2, &gpu.mat_bgs[&(MatMode::Opaque as u32)], &[]);
                            sp.draw_indexed(0..sec.caster_count, 0, 0..1);
                            sec_n += 1;
                        }
                        // cut-out materials need their texture
                        for b in &sec.batches[sec.caster_batches..] {
                            if b.mode.blended() || !lf.intersects_aabb(b.min, b.max) {
                                continue;
                            }
                            let ext = (b.max - b.min).length();
                            if ext < ((b.min + b.max) * 0.5).distance(self.cam.pos) * 0.03 {
                                continue;
                            }
                            if !bound {
                                sp.set_vertex_buffer(0, vb.slice(..));
                                sp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                                bound = true;
                            }
                            sp.set_bind_group(1, b.bind.as_ref(), &[]);
                            sp.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                            sp.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                            sec_n += 1;
                        }
                    }
                    self.vehicles.draw_shadow(&mut sp, &gpu, &mut n, Some((self.cam.pos, 140.0)));
                    // (the windows cast too, as in the original's shadow passes)
                    self.vehicles.draw_glass_casters(&mut sp, &gpu, &mut n, Some((self.cam.pos, 140.0)));
                    self.vehicles.draw_solid_casters(&mut sp, &gpu, &mut n, Some((self.cam.pos, 140.0)));
                    sp.set_pipeline(&gpu.pipeline_shadow);
                    if let Some(s) = &peds_shadow {
                        s.draw(&mut sp, gpu, &mut n);
                    }
                    self.cine.draw(&mut sp, &gpu, true, &mut n);
                    sp.set_pipeline(&gpu.pipeline_shadow_inst);
                    sp.set_bind_group(3, &gpu.identity_obj.bg, &[]);
                    self.props.draw(&mut sp, &gpu, self.cam.pos, &lf, None, &mut n);
                }
                (e.finish(), n, sec_n)
            };
            let casc = casc_vps.map(|cvps| {
                let car_casc_all = std::env::var("MCLA_CAR_CASC_ALL").is_ok();
                // (sel, 2026-10-13: WHICH sector batches go into a tile - the original's rule, read from its code
                // (F-56244 the sector, F-76875 + F-37078 the batch; LIGHTING_WORK.md "The cascade tiles'
                // casters: the selection rule from the code"). A sector within 200 m (flat, the eye to its box) hands
                // every batch of its file to the test, whatever its material - the panes and signs too, as solid
                // casters; a batch by the sphere round its stored box (centre c, radius r):
                //   1. |eye - c| - r < 200;
                //   2. the MAIN view's near, left and right planes (inward normal n): where the light runs outwards
                //      through the plane (d . n < 0, d = the light's own direction, not the cascades' ray) a sphere
                //      wholly outside it is in no tile - what lies beside or behind the view and throws its shadow
                //      away from it;
                //   3. the sphere against the tile's own planes.
                // No size cut, no extrusion, no top / bottom / far plane of the main view. Checked on the depth draws
                // of 17 captures: no batch the rule keeps out is drawn, none it lets in is missing but those of
                // sectors the original did not hold in detail at the time (work/lighting/sl_rule.py).
                // MCLA_CASC_SEL=0: the box against the tile's rectangle, as before; =noblend: the rule without the
                // blended batches (the port's own skip).)
                let casc_sel: Option<(Vec3, Vec3, [glam::Vec4; 3], bool)> = match std::env::var("MCLA_CASC_SEL").ok().as_deref().map(str::trim) {
                    Some("0") => None,
                    v => {
                        let m = glam::Mat4::from_cols_array_2d(&globals.view_proj);
                        let eye = Vec3::new(globals.cam_pos[0], globals.cam_pos[1], globals.cam_pos[2]);
                        let unit = |p: glam::Vec4| p / p.truncate().length().max(1e-9);
                        let fwd = m.row(3).truncate().normalize_or(Vec3::Z);
                        // (the near plane 0.2 m in front of the eye, as the original's view has it)
                        let planes = [fwd.extend(-fwd.dot(eye) - 0.2), unit(m.row(3) + m.row(0)), unit(m.row(3) - m.row(0))];
                        let d = -Vec3::new(globals.sun_dir[0], globals.sun_dir[1], globals.sun_dir[2]).normalize_or(Vec3::Y);
                        Some((eye, d, planes, v != Some("noblend")))
                    }
                };
                (cvps, std::env::var("MCLA_CAR_CASC_ALL").is_ok(), casc_sel)
            });
            let casc_job = |ci: usize| -> (wgpu::CommandBuffer, usize) {
                let (cvps, car_casc_all, casc_sel) = casc.as_ref().unwrap();
                let (car_casc_all, cvp) = (*car_casc_all, &cvps[ci]);
                let mut e = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("shadow cascades") });
                let mut n = 0usize;
                let mut sec_n = 0usize;
                {
                    let mut sp = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("shadow cascades"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &gpu.casc_view,
                            depth_ops: Some(wgpu::Operations { load: if ci == 0 { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    let tile = render::casc_tile();
                    let lf = Frustum::from_view_proj(*cvp);
                    let (ox, oy) = ((ci as u32 % 2) * tile, (ci as u32 / 2) * tile);
                    sp.set_viewport(ox as f32, oy as f32, tile as f32, tile as f32, 0.0, 1.0);
                    sp.set_scissor_rect(ox, oy, tile, tile);
                    // (casc2: each kind of caster with the original's culling or without - render/mod.rs CASC_CULL_*)
                    let casc_pl = |kind: u32| if gpu.casc_cull_kinds & kind != 0 { &gpu.pipeline_casc_cull } else { &gpu.pipeline_casc };
                    sp.set_pipeline(casc_pl(render::CASC_CULL_SEC));
                    sp.set_bind_group(0, &gpu.casc_pass_bgs[ci], &[]);
                    sp.set_bind_group(3, &gpu.identity_obj.bg, &[]);
                    // (sel: the tile's four side planes, inward, unit normals)
                    let tile_pl = {
                        let unit = |p: glam::Vec4| p / p.truncate().length().max(1e-9);
                        [unit(cvp.row(3) + cvp.row(0)), unit(cvp.row(3) - cvp.row(0)), unit(cvp.row(3) + cvp.row(1)), unit(cvp.row(3) - cvp.row(1))]
                    };
                    for (detailed, sec) in loaded_cast().map(|s| (true, s)).chain(sets_cast().map(|s| (false, s))) {
                        let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
                        if let (true, Some((eye, d, main_pl, blend))) = (detailed, casc_sel.as_ref()) {
                            // the sector: its box within 200 m of the eye, flat
                            let (mut smin, mut smax) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                            for b in &sec.batches {
                                smin = smin.min(b.min);
                                smax = smax.max(b.max);
                            }
                            let (dx, dz) = ((smin.x - eye.x).max(eye.x - smax.x).max(0.0), (smin.z - eye.z).max(eye.z - smax.z).max(0.0));
                            if dx * dx + dz * dz >= 200.0 * 200.0 {
                                continue;
                            }
                            let keep = |c: Vec3, r: f32| -> bool {
                                eye.distance(c) - r < 200.0
                                    && !main_pl.iter().any(|p| p.truncate().dot(*d) < 0.0 && p.dot(c.extend(1.0)) < -r)
                                    && tile_pl.iter().all(|p| p.dot(c.extend(1.0)) >= -r)
                            };
                            let mut bound = false;
                            // (solid casters: neighbouring runs go out as one draw; a cut-out batch with its own picture)
                            let mut run: Option<(u32, u32)> = None;
                            let mut solid_bind = false;
                            macro_rules! flush_solid {
                                ($bind:expr) => {
                                    if let Some((a, e)) = run.take() {
                                        if !solid_bind {
                                            sp.set_bind_group(1, $bind, &[]);
                                            sp.set_bind_group(2, &gpu.mat_bgs[&(MatMode::Opaque as u32)], &[]);
                                            solid_bind = true;
                                        }
                                        sp.draw_indexed(a..e, 0, 0..1);
                                        sec_n += 1;
                                    }
                                };
                            }
                            for (bi, b) in sec.batches.iter().enumerate() {
                                if b.mode.blended() && !*blend {
                                    continue;
                                }
                                let cutout = bi >= sec.caster_batches && world::shadow_cutout(b.mode);
                                let whole = [(b.first, b.count, (b.min + b.max) * 0.5, (b.max - b.min).length() * 0.5, 0u32)];
                                let parts: &[(u32, u32, Vec3, f32, u32)] = if b.parts.is_empty() { &whole } else { &b.parts };
                                let mut cut: Vec<(u32, u32)> = Vec::new();
                                for p in parts {
                                    if !keep(p.2, p.3) {
                                        continue;
                                    }
                                    if !bound {
                                        sp.set_vertex_buffer(0, vb.slice(..));
                                        sp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                                        bound = true;
                                    }
                                    if cutout {
                                        match cut.last_mut() {
                                            Some((_, end)) if *end == p.0 => *end = p.0 + p.1,
                                            _ => cut.push((p.0, p.0 + p.1)),
                                        }
                                    } else {
                                        match run.as_mut() {
                                            Some((_, end)) if *end == p.0 => *end = p.0 + p.1,
                                            _ => {
                                                flush_solid!(b.bind.as_ref());
                                                run = Some((p.0, p.0 + p.1));
                                            }
                                        }
                                    }
                                }
                                if !cut.is_empty() {
                                    flush_solid!(b.bind.as_ref());
                                    sp.set_bind_group(1, b.bind.as_ref(), &[]);
                                    sp.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                                    solid_bind = false;
                                    for (a, e) in cut {
                                        sp.draw_indexed(a..e, 0, 0..1);
                                        sec_n += 1;
                                    }
                                }
                            }
                            flush_solid!(sec.batches[0].bind.as_ref());
                            continue;
                        }
                        let mut bound = false;
                        if sec.caster_count > 0 && lf.intersects_aabb(sec.caster_min, sec.caster_max) {
                            sp.set_vertex_buffer(0, vb.slice(..));
                            sp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                            bound = true;
                            sp.set_bind_group(1, sec.batches[0].bind.as_ref(), &[]);
                            sp.set_bind_group(2, &gpu.mat_bgs[&(MatMode::Opaque as u32)], &[]);
                            sp.draw_indexed(0..sec.caster_count, 0, 0..1);
                            sec_n += 1;
                        }
                        for b in &sec.batches[sec.caster_batches..] {
                            if b.mode.blended() || !lf.intersects_aabb(b.min, b.max) {
                                continue;
                            }
                            let ext = (b.max - b.min).length();
                            if ext < ((b.min + b.max) * 0.5).distance(self.cam.pos) * 0.03 {
                                continue;
                            }
                            if !bound {
                                sp.set_vertex_buffer(0, vb.slice(..));
                                sp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                                bound = true;
                            }
                            sp.set_bind_group(1, b.bind.as_ref(), &[]);
                            sp.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                            sp.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                            sec_n += 1;
                        }
                    }
                    // (cars: those within the tile's far slice of the camera, and a car's length more)
                    // (2026-10-15: into the first two tiles only - every capture has its 2 x 93
                    // VS_CarShadowDepthLocal draws in two tiles and none in the other two; where the
                    // top-down collector reads a further tile the original's car side is unshadowed:
                    // l3L_15, the R8's sunlit side = the paint program with collector 1.0, the port
                    // had 0.9. MCLA_CAR_CASC_ALL=1: into all four, as before.)
                    if ci < 2 || car_casc_all {
                        sp.set_pipeline(casc_pl(render::CASC_CULL_CAR));
                        self.vehicles.draw_shadow(&mut sp, &gpu, &mut n, Some((self.cam.pos, render::CASC_TUNE[ci][0] + 20.0)));
                        // (the glass, the lamps' lenses, the occluder box and the wheels' spheres: both sides, as the
                        // original's - 0x18000 on its PS_GlassFront / PS_Light / PS_fixupAlpha meshes' depth draws)
                        sp.set_pipeline(&gpu.pipeline_casc);
                        self.vehicles.draw_glass_casters(&mut sp, &gpu, &mut n, Some((self.cam.pos, render::CASC_TUNE[ci][0] + 20.0)));
                        self.vehicles.draw_solid_casters(&mut sp, &gpu, &mut n, Some((self.cam.pos, render::CASC_TUNE[ci][0] + 20.0)));
                    }
                    sp.set_pipeline(casc_pl(render::CASC_CULL_PED));
                    if let Some(s) = &peds_shadow {
                        s.draw(&mut sp, gpu, &mut n);
                    }
                    self.cine.draw(&mut sp, &gpu, true, &mut n);
                    sp.set_pipeline(if gpu.casc_cull_kinds & render::CASC_CULL_PROP != 0 { &gpu.pipeline_casc_inst_cull } else { &gpu.pipeline_casc_inst });
                    sp.set_bind_group(3, &gpu.identity_obj.bg, &[]);
                    self.props.draw(&mut sp, &gpu, self.cam.pos, &lf, None, &mut n);
                }
                (e.finish(), n + sec_n)
            };
            // --- the fast shadow collector (the far city's ground from above, blurred, kept) ---
            let far: Vec<&world::GpuSector> = world.far_models().collect();
            let to_light = Vec3::new(globals.sun_dir[0], globals.sun_dir[1], globals.sun_dir[2]).normalize();
            let fc_dir = if self.drive && self.vehicles.player().is_some() { self.chase.look - self.chase.pos } else { self.cam.forward() };
            let collector_job = || -> wgpu::CommandBuffer {
                let mut e = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("collector") });
                gpu.collector_pass(&mut e, fc_cam, fc_dir.normalize_or(Vec3::Z), to_light, light_vp, far.iter().copied());
                e.finish()
            };
            // --- the wet road's planar reflection (the far city mirrored, the sky dome behind) ---
            // the mirror's height (gReflectionParams.z): recorded 17.572 with the car standing
            // on ground at 17.58 - exactly 2 m under the centre of the cars' environment map
            // (c51.y 19.572): the height of the car's own place. (MCLA_REFL_PLANE: another.)
            let plane = self.vehicles.player().filter(|_| self.drive).map_or(globals.cam_pos[1] - 2.0, |c| c.render_pose(self.vehicles.render_alpha).pos.y);
            let plane = std::env::var("MCLA_REFL_PLANE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(plane);
            let sky = self.sky_now().hat.is_some() && gpu.sky_hat.ready();
            let frame_no = self.frame_no;
            // (its 256 x 256 picture is drawn with pix_center 0 as well: half ITS pixel - the main view's matrix
            // carries half a scene pixel, refl.wgsl adds the difference)
            let refl_px = if render::half_pixel_on() {
                let r = render::REFL_SIZE as f32;
                [1.0 / r - 1.0 / gpu.scene_w as f32, -(1.0 / r - 1.0 / gpu.scene_h as f32)]
            } else {
                [0.0, 0.0]
            };
            let reflection_job = || -> wgpu::CommandBuffer {
                let mut e = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("reflection") });
                let n = gpu.reflection_pass(&mut e, glam::Mat4::from_cols_array_2d(&globals.view_proj), refl_px, Vec3::new(globals.cam_pos[0], globals.cam_pos[1], globals.cam_pos[2]), plane, sky, far.iter().copied());
                if frame_no == 100 && std::env::var("MCLA_REFL_DUMP").is_ok() {
                    eprintln!("refl: plane {:.3} camera {:.3} batches {} sky {}", plane, globals.cam_pos[1], n, sky);
                }
                e.finish()
            };
            let tiles = if casc.is_some() { 4 } else { 0 };
            use rayon::prelude::*;
            // (the walkers' skinning for this frame, docs/PERFORMANCE.md 2026-10-27 (4): beside the shadow passes)
            let (peds, ped_eye, ped_frustum) = (&mut self.peds, self.cam.pos, &frustum);
            let (mut parts, ()): (Vec<(usize, wgpu::CommandBuffer, usize, usize)>, ()) = mcla_flash::raster::pool().install(|| rayon::join(|| {
                // (0 the shadow map, 1 - 4 the cascade tiles, 5 the collector, 6 the reflection)
                (0..7usize)
                    .into_par_iter()
                    .filter(|&j| match j {
                        0 | 5 => shadow_on,
                        1..=4 => j <= tiles,
                        _ => refl_on,
                    })
                    .map(|j| match j {
                        0 => {
                            let (c, n, s) = shadow_job();
                            (j, c, n, s)
                        }
                        1..=4 => {
                            let (c, n) = casc_job(j - 1);
                            (j, c, n, 0)
                        }
                        5 => (j, collector_job(), 0, 0),
                        _ => (j, reflection_job(), 0, 0),
                    })
                    .collect()
            }, || peds.prepare(gpu, ped_eye, ped_frustum)));
            parts.sort_by_key(|p| p.0);
            for (j, cb, n, sec_n) in parts {
                if j == 0 {
                    stat_shadow_sec = sec_n;
                    stat_shadow = n + sec_n;
                } else if j <= 4 {
                    stat_shadow += n;
                }
                early_cbs.push(cb);
            }
            // The garage floor's mirror (floormirror.rs): in the garage the reflection picture also holds the room
            // and the car, mirrored about the plane. MCLA_DEBUG_VIEW=719: without (the floor's mirror term off too).
            if refl_on && globals.car_amb[3] > 0.5 && (globals.params[3] - 719.0).abs() > 0.5 {
                let mut e = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("garage floor mirror") });
                let n = floormirror::draw(&mut e, &gpu, &world, &self.vehicles, &globals, plane);
                if self.frame_no % 300 == 0 && std::env::var("MCLA_GARAGE_FLOOR_LOG").is_ok() {
                    log::info!("garage floor mirror: plane {:.4}, {} draws", plane, n);
                }
                early_cbs.push(e.finish());
            }
        }
        passprof::mark("impostors+prepare");
        gpuprof::stamp(&gpu.device, &gpu.queue, &mut enc, "impostors+prepare");
        // (the fast shadow collector and the wet road's planar reflection: recorded with the shadows, above)
        // (the plants' impostor pictures that are due this frame - props/impostor.rs)
        self.props.update_impostors(&gpu, &mut enc, &globals);
        // (the particles' fog: RMPTFX_FogMinColor = sm_KeyMinFogColor x the light factor the
        // fog pass's colour takes, RMPTFX_FogParams.x = sm_KeyFogHeight - recorded c81 / c79)
        self.fx.fog_min = self.sky_now().tod.map_or([0.0, 0.0, 0.0, 0.0, 1000.0], |d| {
            let k = (d.light + d.ambient).lerp(Vec3::ONE, d.light_w);
            [d.fog_min[0] * k.x, d.fog_min[1] * k.y, d.fog_min[2] * k.z, d.fog_min[3], d.fog_height]
        });
        self.fx.prepare(&gpu, self.cam.pos);
        // (the rain and the lamps' cones in it, as the original draws them - rain.rs)
        rain::prepare(&gpu, &rain::Frame { cam_pos: globals.cam_pos, inv_view_proj: globals.inv_view_proj, rain: self.rain_amount(), light: self.sky_now().tod.map(|t| (t.light + t.ambient, t.nfl)), car: self.vehicles.player().filter(|_| self.drive).map(|c| c.render_pose(self.vehicles.render_alpha).pos), lamps: &self.props.prop_lamps, weather: match self.weather { sky::Weather::Cloudy => 1, sky::Weather::Stormy => 2, sky::Weather::Foggy => 3, _ => 0 }, next: self.sky_next.as_ref().map(|(_, w, f)| (match w { sky::Weather::Cloudy => 1, sky::Weather::Stormy => 2, sky::Weather::Foggy => 3, _ => 0 }, *f)), hour: self.time_of_day, collider: Some(&world.collider) });
        // (the story scenes' characters: their own programs, lit by the light manager's state -
        // skin.rs `CharLight`; MCLA_CHAR_OLD=1: the generic look as before)
        let hour = globals.sun_color[3];
        self.cine.set_light(std::env::var("MCLA_CHAR_OLD").is_err().then(|| skin::CharLight {
            to_light: Vec3::new(globals.sun_dir[0], globals.sun_dir[1], globals.sun_dir[2]),
            key: Vec3::new(globals.sun_color[0], globals.sun_color[1], globals.sun_color[2]),
            strength: globals.sun_dir[3],
            amb: Vec3::new(globals.ambient_sky[0], globals.ambient_sky[1], globals.ambient_sky[2]),
            night: hour > 18.0 || hour < 6.0,
        }));
        self.cine.prepare(&gpu);
        {
            // drivers of the player's car, the racers and the police (traffic models have their own)
            let force = std::env::var("MCLA_DRV_STEER").ok().and_then(|v| v.parse::<f32>().ok());
            let seats: Vec<drivers::Seat> = self
                .vehicles
                .cars
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    // ambient cars get a driver too, except the big ones whose cabs sit elsewhere
                    let n = c.model.name.to_ascii_lowercase();
                    let big = n.contains("bus") || n.contains("rstr") || n.contains("semitruck") || n.contains("limo");
                    // (ambient cars have opaque windows: nobody to see in there)
                    let _ = big;
                    // in a cutscene the people of its cars are the scene's own characters
                    // (in a scene a driver is drawn only with its show flag, the draw gate F-14542: cinedriver.rs; while
                    // the scene casts the player on foot his seat stays empty until the scene's own showdriver -
                    // docs/reference/cutscenes/scene_driver_flag.md)
                    let staged = self.cine.playing.is_some() && (!c.show_driver || (c.is_player && self.cine.has_actor("player") && !self.cine_showdriver.contains(&c.uid)));
                    // (VEHICLE SELECT: the three stand empty - `ng_vsel_0`)
                    // (the cruiser has no driver: mcRacer::LoadDriver, F-39898, returns before it makes one when
                    // the car is vp_chv_police_96 - its own black interior is what shows; GitHub #17)
                    c.placed && c.parked.is_none() && !c.model.name.eq_ignore_ascii_case(police::COP_CAR) && !n.starts_with("va_") && self.game.mode != Mode::Garage && !staged && !self.vehicles.lights_off && !c.driver.starts_with("hangout:")
                })
                .map(|(i, c)| {
                    let pose = c.render_pose(self.vehicles.render_alpha);
                    let mut mat = glam::Mat4::from_rotation_translation(pose.rot, pose.pos);
                    if c.model.bike {
                        mat = mat * glam::Mat4::from_rotation_z(pose.lean);
                    }
                    let n = c.model.name.to_ascii_lowercase();
                    if n.starts_with("va_") && ["escalade", "yukon", "silverado", "cad_ext", "caravan"].iter().any(|k| n.contains(k)) {
                        // trucks and SUVs seat their driver higher than the shared animation does
                        mat = mat * glam::Mat4::from_translation(Vec3::new(0.0, 0.3, 0.0));
                    }
                    drivers::Seat { id: i, uid: c.uid, car: c.model.name.as_str(), mat, pos: pose.pos, steer: force.unwrap_or(c.input.steer), speed: c.state.fwd_speed(), reversing: c.state.gear < 0, player: c.is_player, cop: c.cop, bike: c.model.bike, set: c.driver_set.as_str(), hi: c.is_player || c.hi_driver }
                })
                .collect();
            self.drivers.prepare(&mut gpu, &seats, self.cam.pos, &frustum, dt);
        }
        passprof::mark("main setup");
        // --- main pass ----------------------------------------------------------
        let post_on = !self.opt.no_post;
        // a long lens magnifies: scale the small-object culling with it
        let lens = ((self.cam.fov_y * 0.5).tan() / 0.637).clamp(0.3, 1.2);
        let cam_pos = self.cam.pos;
        let mut draws = 0usize;
        let mut stat_props = 0usize;
        let mut stat_cars = 0usize;
        // The lights round the car as the environment map shows them (VS_LightGlow): the
        // nearest lamps within 200 m, each the lamp's colour with a strength that falls with
        // distance - recorded 2.5 at 7 m, 2.1 at 49 m, 1.7 at 84 m, 1.0 at 150 m, 0.4 at 200 m:
        // about 2.6 x (1 - d / 240) - while the lamps are on (m_NFLFade).
        let mut env_glows = 0u32;
        if let (Some(_), Some(t)) = (env_frustum.as_ref(), self.sky_now().tod.as_ref()) {
            // (the list is AddGlow's - the lamps' own on / off minutes, not m_NFLFade: halo::lamp_lit)
            if (t.nfl > 0.0 || self.time_of_day > 17.6 || self.time_of_day < 6.4) && std::env::var("MCLA_NO_ENV_GLOW").is_err() {
                // CORRECTED 2026-10-14 against the lists of four captures (work/lighting/glowpair.py,
                // 23:30, 05:45, 18:02, 18:15): the glows are the PROPS' lamps - every prop with a
                // light file, also those of Intensity 0 (la_prop_glow_warm_01x: a row of eleven by
                // the start spot, there for this alone) - and none of the city's own light records
                // (the port had those of range 30 and more in: sixteen glows the original lacks);
                // the strength is the same from the first night frame to the last (not x m_NFLFade:
                // 2.54 at 6.7 m at 05:45 and 18:02 as at 23:30); it falls in a straight line, 2.61
                // at the car to 0.29 at the farthest drawn. (The game hands the places over pulled
                // towards the car by 0.915 - same direction - so against true distance the line
                // ends at 274 m, and the farthest glow stands 243 m off.) MCLA_GLOW_OLD=1: as before.
                let old = std::env::var("MCLA_GLOW_OLD").is_ok();
                let mut near: Vec<(f32, Vec3, Vec3)> = Vec::new();
                if old {
                    for (p, c) in self.props.prop_lamps.iter().map(|l| (l.pos, l.color)).chain(self.props.lights.iter().filter(|l| l.range >= 30.0).map(|l| (l.pos, l.color))) {
                        let d = p.distance(env_eye);
                        if d < 200.0 && d > 0.5 {
                            near.push((d, p, c));
                        }
                    }
                } else {
                    let no_occ = globals.params[3] == 118.0;
                    for (((p, c, r), bx), (bc, bw, br, org)) in self.props.glow_lamps.iter().zip(self.props.glow_boxes.iter()).zip(self.props.glow_cull.iter()) {
                        // (the game's one list of lamp props, taken from the MAIN view: halo::ListView)
                        if !halo::lamp_lit(*org, self.time_of_day) || !lamp_list.holds(*bc, *bw, *br, || !no_occ && occluded.hides_aabb(bx.0, bx.1)) {
                            continue;
                        }
                        let d = lamp_list.order(*org) as f32 + p.distance(env_eye) * 1.0e-4;
                        {
                            near.push((d, *p, *c));
                            if self.frame_no == 60 && std::env::var("MCLA_GLOW_DUMP").is_ok() { eprintln!("glowrad {:.1} {:.1} {:.1} r {:.3} d {:.1} box {:.2} {:.2} {:.2} w {:.3} R {:.3}", p.x, p.y, p.z, r, d, bc.x, bc.y, bc.z, bw, br); }
                        }
                    }
                }
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                near.truncate(render::ENV_GLOWS);
                // (the places the game hands over are the lamps' pulled 8.5 % of the way towards the MAIN
                // view's camera - 818.0 25.6 -806.0 for the lamp at 817.5 26.2 -805.1 with the camera at
                // 823.8 19.5 -816.0, the same fraction on the three nearest and 0.915 of the distance
                // on the far ones - so a near lamp stands some degrees off its true direction in the
                // map (the gate's lamp 5 degrees higher), and the strength goes by that place's
                // distance: 2.61 x (1 - d' / 251).)
                let cam_main = self.cam.pos;
                let data: Vec<[f32; 8]> = near.iter().map(|(d, p, c)| {
                    if old {
                        [p.x, p.y, p.z, 2.6 * (1.0 - d / 240.0) * t.nfl.min(1.0), c.x, c.y, c.z, 0.0]
                    } else {
                        let q = *p + (cam_main - *p) * 0.085;
                        [q.x, q.y, q.z, (2.61 * (1.0 - q.distance(env_eye) / 251.0)).max(0.0), c.x, c.y, c.z, 0.0]
                    }
                }).collect();
                // (MCLA_GLOW_DUMP: the list once, to lay beside a capture's - work/lighting/glowcmp.py)
                if self.frame_no == 60 && std::env::var("MCLA_GLOW_DUMP").is_ok() {
                    for g in &data { eprintln!("envglow {:.1} {:.1} {:.1} a {:.3} col {:.2} {:.2} {:.2}", g[0], g[1], g[2], g[3], g[4], g[5], g[6]); }
                    for l in self.props.prop_lamps.iter().filter(|l| l.pos.distance(env_eye) < 240.0) { eprintln!("glowsrc lamp {:.1} {:.1} {:.1} col {:.2} {:.2} {:.2} int {:.1}", l.pos.x, l.pos.y, l.pos.z, l.color.x, l.color.y, l.color.z, l.intensity); }
                    for l in self.props.lights.iter().filter(|l| l.pos.distance(env_eye) < 240.0) { eprintln!("glowsrc city {:.1} {:.1} {:.1} col {:.2} {:.2} {:.2} int {:.1} range {:.1} cone {:.2} kind {}", l.pos.x, l.pos.y, l.pos.z, l.color.x, l.color.y, l.color.z, l.intensity, l.range, l.cone, l.kind); }
                }
                if !data.is_empty() {
                    gpu.queue.write_buffer(&gpu.env_glow_buf, 0, bytemuck::cast_slice(&data));
                    env_glows = data.len() as u32;
                }
            }
        }
        // The game's own sky dome: its constants of this frame and its Perlin noise. The cloud
        // phase runs at the weather's cloud speed / 60 a second (recorded: 0.145 a second with
        // the speed key 8.70; 96 about a minute and a half after the start), the detail noise's
        // offset is a hundredth of it, wrapped at 2 pi / detail scale.
        let mut sky_hat_on = false;
        if let Some(mut c) = self.sky_now().hat.filter(|_| gpu.sky_hat.ready()) {
            // (MCLA_SKY_PHASE: a recorded phase, to lay the clouds where a capture of the original has them)
            let phase = std::env::var("MCLA_SKY_PHASE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(83.0 + self.sim_time * c.perlin[8][1] / 60.0);
            c.perlin[8] = [phase, 0.0, 0.0, 0.0];
            let off = (0.01 * phase) % (std::f32::consts::TAU / c.vs[2][2].max(0.01));
            // (MCLA_SKY_DETAIL: a recorded DetailOffset - it runs on the game's own clock, not with the phase)
            let off = std::env::var("MCLA_SKY_DETAIL").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(off);
            c.vs[5] = [off, off, 0.0, 0.0];
            // (MCLA_SKY_PS="68:1,0,0,1;74:0,1,0,0": pixel constants by register, for experiments
            // against the same override on the running original)
            if let Ok(v) = std::env::var("MCLA_SKY_PS") {
                for item in v.split(';') {
                    let Some((r, vals)) = item.split_once(':') else { continue };
                    let f: Vec<f32> = vals.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    let slot = match r.trim().parse::<usize>() { Ok(n @ 64..=83) => n - 64, Ok(n @ 100..=103) => n - 80, _ => continue };
                    if f.len() == 4 { c.ps[slot] = [f[0], f[1], f[2], f[3]]; }
                }
            }
            // (MCLA_SKY_DUMP: the constants of one frame, to lay beside a capture's)
            if std::env::var("MCLA_SKY_DUMP").is_ok() {
                static ONCE: std::sync::Once = std::sync::Once::new();
                ONCE.call_once(|| {
                    for (i, v) in c.vs.iter().enumerate() { eprintln!("skyhat vs {} {:?}", [64, 65, 66, 67, 68, 69, 72, 73, 75, 96, 97, 98][i], v); }
                    for (i, v) in c.ps.iter().enumerate() { eprintln!("skyhat ps {} {:?}", if i < 20 { 64 + i } else { 80 + i }, v); }
                    for (i, v) in c.perlin.iter().enumerate() { eprintln!("skyhat perlin {} {:?}", 64 + i, v); }
                });
            }
            gpu.sky_hat.write(&gpu.queue, &c);
            gpu.sky_hat.perlin_pass(&mut enc);
            sky_hat_on = true;
        }
        // The mini sky the panes mirror: the game's own quad (ps_miniSky) into the 128 x 128 map,
        // every frame after the Perlin pass. (MCLA_MINISKY_OLD=1: not drawn - city.wgsl's old
        // stand-in function instead.)
        gpu.sky_hat.mini_pass(&mut enc, &gpu.minisky_view, sky_hat_on && std::env::var("MCLA_MINISKY_OLD").is_err());
        passprof::mark("environment");
        gpuprof::stamp(&gpu.device, &gpu.queue, &mut enc, "environment");
        // ---- the environment map: one face of the cube a frame - the sky and the city's
        // opaque geometry around the player's car, for the car shaders to mirror ----
        // The city of the map is drawn the original's way (2026-10-14): into two paraboloid
        // halves with the warp applied per vertex (VSCity*Env; city.wgsl vs_env_dp), one half a
        // frame, and the cube's faces are filled from them (env_fill.wgsl) over their sky; the
        // glows go on the cube as before. MCLA_ENV_CUBE=1: the city straight into the cube's
        // faces with a true projection, as until then.
        let env_dp = std::env::var("MCLA_ENV_CUBE").is_err();
        if let (true, Some(_)) = (env_dp, env_frustum.as_ref()) {
            let half = (self.frame_no % 2) as usize;
            let axis = if half == 0 { Vec3::NEG_Z } else { Vec3::Z };
            let mut dp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("environment, paraboloid half"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.env_dp_views[half],
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &gpu.env_dp_depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            dp.set_bind_group(0, &gpu.env_bgs[6 + half], &[]);
            dp.set_bind_group(3, &gpu.identity_obj.bg, &[]);
            // WHAT the original's map holds (2026-10-14; its environment passes by program and by
            // vertex buffer in four captures, matched to the sector files' batches by index and
            // vertex count - work/lighting/em_list.py, em_geom.py, em_far.py, em_miss.py,
            // em_match.py, em_fit3.py): the FAR MODEL of every sector within 200 m in plan of the
            // main view's camera (VSCityLODEnv / WindowLODEnv, at night VSCityLODEnvNight), the
            // sectors round the car included, and of the detail models only the few batches round
            // the eye (below); no shop panes (CityWindowLower), no upper windows (CityWindowUpper),
            // no shop signs, nothing blended - all 235 draws of the two halves are opaque (blend
            // 0x00010001) and without alpha test (colour control 0xaa000004: fences and wires are
            // solid), the same 118 at noon, dusk and night. The port drew every loaded sector (to
            // 450 m) in detail with its panes: lit shop fronts and windows along the skyline where
            // the original's map has the far models' blocks. MCLA_ENV_ALL=1: as until then.
            let env_all = std::env::var("MCLA_ENV_ALL").is_ok();
            let env_only = std::env::var("MCLA_ENV_ONLY").ok();
            // (2026-10-15: the CityDefault batches are NOT in the original's map. Its passes do draw
            // them - 12 draws / 25,221 indices a half, the port's list to the index - but
            // VSCityDefaultEnv has no warp: lines 8 - 11 put out (x, y, forward, 1) in METRES from
            // the eye (the "matrix" c8 .. c11 of the env draws is a bare translation), a box of
            // +-1 m across the whole 512 x 256 and 0 .. 1 m deep, where nothing stands. The port
            // warped them like the rest: the wires across the sky and the dark stroke at the lamp
            // post. MCLA_ENV_DEFAULT=1: with them, as until then.)
            let env_no_default = !env_all && std::env::var("MCLA_ENV_DEFAULT").is_err();
            // (2026-10-13, downtown among towers - cap/np1_12, work/lighting/ed_fit.py: the eye's distance to a
            // batch's sphere counts IN PLAN. Of 530 batches with a unique size 30 are in the passes; the test in
            // space gets 7 wrong, the one in plan 3 - tower walls whose spheres' centres stand 40 - 150 m above
            // the street were left to their far models. And the passes hold the UPPER WINDOW batches
            // (VSCityWindowUpperEnv / PSCityWindowUpper, 4 + 2 + 1 draws a half there, none at the gate, where
            // no window batch is in reach): the glass fronts of the towers. MCLA_ENV_3D=1: in space and without
            // the windows, as until then.)
            let env_3d = std::env::var("MCLA_ENV_3D").is_ok();
            let env_farhalf = std::env::var("MCLA_ENV_FARHALF").map_or(true, |v| v != "0");
            let env_far_old = std::env::var("MCLA_ENV_FAR_OLD").is_ok();
            // (the main camera's back axis in space - the view's own direction, pitch included; F-56244 reads the
            // viewport's camera matrix row +224)
            let env_main_back = -{ let d = if self.drive && self.vehicles.player().is_some() { self.chase.look - self.chase.pos } else { self.cam.forward() }; d.normalize_or(Vec3::Z) };
            let env_main_fwd = { let d = if self.drive && self.vehicles.player().is_some() { self.chase.look - self.chase.pos } else { self.cam.forward() }; Vec3::new(d.x, 0.0, d.z).normalize_or(Vec3::Z) };
            let env_farwin = std::env::var("MCLA_ENV_FARWIN").map_or(true, |v| v != "0");
            let env_list = std::env::var("MCLA_ENV_LIST").ok().map_or(false, |v| { let f0 = v.trim().parse::<u64>().ok().filter(|&f| f > 1).unwrap_or(60); self.frame_no == f0 || self.frame_no == f0 + 1 }); // (MCLA_ENV_LIST=<frame>: that frame and the next; 1: 60 / 61)
            let cam_main = self.cam.pos;
            // (the distance F-56244 tests against the tunable 200 - F-82919 from the LOD view's position (+112) to the
            // sector record's box (+208 / +224): the LARGER of the two plan gaps, x and z, not their hypotenuse. The
            // LOD view is the main camera moved back along its own back axis by the tunable 10 (F-66201 copies the
            // viewport's camera and adds +224 x 10; a second view at 20 feeds only the record's +260). Read live off
            // the original's 109 sector records: +268 against the LOD view's position, 109 of 109 within 0.05 m, and
            // the LOD view = viewport camera + 10.000 x back axis. MCLA_ENV_FAR_OLD=1: the camera's plan distance.)
            let env_far_old0 = std::env::var("MCLA_ENV_FAR_OLD").is_ok();
            let lod_eye = if env_far_old0 { cam_main } else { cam_main + env_main_back * 10.0 };
            let plan = |mn: [f32; 3], mx: [f32; 3]| -> f32 {
                let (cx, cz) = (lod_eye.x.clamp(mn[0], mx[0]), lod_eye.z.clamp(mn[2], mx[2]));
                let (gx, gz) = ((cx - lod_eye.x).abs(), (cz - lod_eye.z).abs());
                if env_far_old0 { (gx * gx + gz * gz).sqrt() } else { gx.max(gz) }
            };
            let sec_dist = |id: usize| world.index.sectors.get(id).map(|s| plan(s.min, s.max));
            if env_list {
                eprintln!("envlist eye {:.3} {:.3} {:.3} main camera {:.3} {:.3} {:.3}", env_eye.x, env_eye.y, env_eye.z, cam_main.x, cam_main.y, cam_main.z);
            }
            // ... and of the detail models only the batches round the eye: of the 6,849 stored batches
            // of the thirty sectors loaded at the gate the passes draw 61 - those whose bounding
            // sphere (the sector file's table at +1c: the box's centre and half diagonal) has the
            // map's eye within 1.13 radii of its centre (work/lighting/em_fit3.py: 2 of 2,670
            // batches with a unique size fall on the wrong side; the same list in both halves and
            // at every hour), in any loaded sector. So the map holds the ground and the large
            // walls round the car, and the far models for everything else.
            let mut env_secs: Vec<(&world::GpuSector, bool, usize)> = world.loaded.iter().map(|(id, s)| (s, true, *id)).collect();
            if !env_all {
                for (sid, gs) in world.far_models_by_sector() {
                    let d = match sec_dist(sid) {
                        Some(d) => d,
                        None => {
                            let (mn, mx) = gs.batches.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |a, b| (a.0.min(b.min), a.1.max(b.max)));
                            plan(mn.to_array(), mx.to_array())
                        }
                    };
                    if d < 200.0 {
                        env_secs.push((gs, false, sid));
                    }
                }
            }
            // (the garage's lights: the stored picture the original adds to its map there - PS_Blit4x,
            // garageEnvMap.dds; uploaded when the garage's light state comes or goes. MCLA_DEBUG_VIEW=714: without.)
            {
                static GLOW_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
                let want = if globals.car_amb[3] > 0.5 && std::env::var("MCLA_DEBUG_VIEW").map_or(true, |v| v.trim() != "714") { 2 } else { 1 };
                if GLOW_STATE.swap(want, std::sync::atomic::Ordering::Relaxed) != want {
                    gpu.set_garage_glow(if want == 2 { GARAGE_GLOW.get().map(|v| &v[..]) } else { None });
                }
            }
            // (the garage - garage round, 2026-10-13: the room is in the original's map, drawn with the Env
            // forms of its programs - VSCityBumpSpecEnv / VSCityGrimeEnv + PSMultiLightEnv, VSCityGarageFloorEnv,
            // VSCityDefaultEnv, VSCityGrimeDecalEnv + PSMultiLight, VSCityNeonEnv: 108 of the two halves' 268
            // draws in cap/gar_top.bin. The room is an interior set here, not a loaded sector: the set the
            // map's eye stands in joins the list. MCLA_DEBUG_VIEW=713: without.)
            if globals.car_amb[3] > 0.5 && std::env::var("MCLA_DEBUG_VIEW").map_or(true, |v| v.trim() != "713") {
                for (id, gs) in world.sets.iter().filter(|e| *e.0 >= world::SET_ID && *e.0 < world::FAR_ID) {
                    let (mn, mx) = gs.batches.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |a, b| (a.0.min(b.min), a.1.max(b.max)));
                    if env_eye.x > mn.x && env_eye.x < mx.x && env_eye.z > mn.z && env_eye.z < mx.z {
                        env_secs.push((gs, true, *id));
                    }
                }
            }
            for blend in [false, true] {
                if blend && (!env_all || std::env::var("MCLA_ENV_NO_PANES").is_ok()) {
                    continue;
                }
                dp.set_pipeline(if blend { gpu.pipeline_env_dp_blend.at(render::ms::PLAIN) } else { gpu.pipeline_env_dp.at(render::ms::PLAIN) });
                let mut last_mode = u32::MAX;
                for (sec, detail, sid) in env_secs.iter().copied() {
                    // (MCLA_ENV_ONLY=far|detail: one of the two kinds alone, to see what each covers)
                    if env_only.as_deref() == Some(if detail { "far" } else { "detail" }) { continue; }
                    // (2026-10-13: which FAR models a half holds does not go by the half's own axis. The first half -
                    // the one that looks along the world's -z - gets every model whose sphere (the box of all its
                    // geometries: centre, half diagonal) reaches the side AHEAD of the MAIN CAMERA from the map's
                    // eye, the second half every model that reaches the side BEHIND it: (c - eye) . f + R > 0 with
                    // +f / -f, f the main view's direction laid flat. Read off the passes' draw lists against the
                    // far city file in six captures (work/lighting/ed_farhalf.py: gate standard / back / right view,
                    // np1, np6 - 22 of 23 models, the miss 4 m from the limit; the half's own axis gets 15, the
                    // 'other half's side' reading of the gate 17). So a block that stands wholly ahead of the camera
                    // is missing from the map wherever the second half would show it, and the other way round -
                    // at the gate (camera looking +z) the far blocks to the south, downtown the towers behind the
                    // camera. The port drew every far batch not wholly behind its half. MCLA_ENV_FARHALF=0: as until then.)
                    // (2026-10-29, decoded: F-56244 sets a sector's pass bits 4 / 8 - the two passes, F-77511 / F-75080 then
                    // draw its far model WHOLE - when the LOD view's gap to the sector's box (see `plan` above) is under the
                    // tunable 200 (F-89368 hands it in; the gap by F-82919; pass bits 4 / 8 by F-30389): the sector's sphere
                    // (box centre, 3D half diagonal) against the plane through the main camera's position across its back
                    // axis (the viewport's camera matrix +224 / +240, in space): d = (c - camera) . back, |d| < r both, else
                    // d < 0 (ahead) mask 4, d > 0 mask 8. Live in the original 10 of 10 records' bits. Mask 4 is the
                    // capture's FIRST pass, whose view matrix has z negated - the half that looks along the world's +z,
                    // the port's half 1; mask 8 the second, along -z, the port's half 0 (the 15 degree cone of the R8's
                    // bumper holds far blocks 224 / 221 behind the camera only with them in the port's half 0). The far
                    // model is not cut at the half's own plane. MCLA_ENV_FAR_OLD=1: the eye's plane, the model's box and
                    // the cut, as until then.)
                    if !detail && !env_all && env_farhalf {
                        let (mn, mx) = sec.batches.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |a, b| (a.0.min(b.min), a.1.max(b.max)));
                        if env_far_old {
                            let side = if half == 0 { env_main_fwd } else { -env_main_fwd };
                            if ((mn + mx) * 0.5 - env_eye).dot(side) + (mx - mn).length() * 0.5 <= 0.0 { continue; }
                        } else {
                            let (smn, smx) = world.index.sectors.get(sid).map_or((mn, mx), |s| (Vec3::from(s.min), Vec3::from(s.max)));
                            let (c, r) = ((smn + smx) * 0.5, (smx - smn).length() * 0.5);
                            let d = (c - cam_main).dot(env_main_back);
                            let both = d.abs() - r < 0.0;
                            if !both && ((half == 1) != (d < 0.0)) { continue; }
                        }
                    }
                    let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
                    let mut bound = false;
                    for b in &sec.batches {
                        let pane = matches!(b.mode, MatMode::StoreGlass);
                        if pane != blend || matches!(b.mode, MatMode::Blend) {
                            continue;
                        }
                        // (what lies wholly behind the half's plane is not in it; the rest is drawn
                        // and the half's own rectangle cuts it, as the original's does)
                        let c = (b.min + b.max) * 0.5;
                        let ext = (b.max - b.min).length();
                        let dist = c.distance(env_eye);
                        let skip = ((detail || env_all || !env_farhalf || env_far_old) && (c - env_eye).dot(axis) + ext * 0.5 < 0.0) || (env_all && (ext < dist * 0.08 || dist > 420.0 + ext)) || (!env_all && (matches!(b.mode, MatMode::StoreSign) || ((env_3d || (!detail && !env_farwin)) && matches!(b.mode, MatMode::Window)))) || (env_no_default && b.shader == "CityDefault");
                        // (a detail model: its stored batches one by one)
                        let by_part = detail && !env_all && !b.parts.is_empty();
                        // (the windows' reach is shorter: at np1 the four window batches in the passes have the eye at
                        // 0.63 .. 0.98 r in plan, one at 1.11 r is not drawn, and at the gate the window band of
                        // sector 132 at 1.06 r is not in the map either - drawn, it darkened the band ahead of the
                        // car from 0.92 to 0.64 of the original's at noon. Inside the sphere: 1.0; the limit is only
                        // bracketed, 0.98 .. 1.06, and why it differs from the other batches' 1.13 is not known.)
                        let env_reach = if matches!(b.mode, MatMode::Window) { 1.0 } else { 1.13 };
                        let mut runs: Vec<(u32, u32)> = Vec::new();
                        if by_part && !skip {
                            for p in &b.parts {
                                // (flags: 1 the blended bucket, 4 an interior wall - the passes draw
                                // those with the MAIN view's vertex program, not as part of the map)
                                if p.4 & 5 == 0 && (if env_3d { p.2.distance(env_eye) } else { glam::Vec2::new(p.2.x - env_eye.x, p.2.z - env_eye.z).length() }) < env_reach * p.3 && (p.2 - env_eye).dot(axis) + p.3 >= 0.0 {
                                    match runs.last_mut() {
                                        Some(r) if r.0 + r.1 == p.0 => r.1 += p.1,
                                        _ => runs.push((p.0, p.1)),
                                    }
                                }
                            }
                        }
                        let skip = skip || (by_part && runs.is_empty());
                        if env_list && !skip {
                            eprintln!("envlist half {} {} sector {} shader {} mode {:?} idx {} dist {:.0} runs {}", half, if detail { "detail" } else { "far" }, sid as i64, b.shader, b.mode, if by_part { runs.iter().map(|r| r.1).sum() } else { b.count }, dist, runs.len());
                        }
                        if skip {
                            continue;
                        }
                        if !bound {
                            dp.set_vertex_buffer(0, vb.slice(..));
                            dp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                            bound = true;
                        }
                        dp.set_bind_group(1, b.bind.as_ref(), &[]);
                        if b.mode as u32 != last_mode {
                            dp.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                            last_mode = b.mode as u32;
                        }
                        if by_part {
                            for r in &runs {
                                dp.draw_indexed(r.0..r.0 + r.1, 0, 0..1);
                            }
                        } else {
                            dp.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                        }
                    }
                }
            }
        }
        if let Some(ef) = env_frustum.as_ref() {
            let mut ep = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("environment"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.env_faces[env_face],
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0] as f64, g: clear[1] as f64, b: clear[2] as f64, a: 1.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &gpu.env_depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            ep.set_bind_group(0, &gpu.env_bgs[env_face], &[]);
            if let (true, Some(tb)) = (sky_hat_on, gpu.sky_hat.tex_bg.as_ref()) {
                ep.set_pipeline(&gpu.sky_hat.pipeline_env);
                ep.set_bind_group(1, tb, &[]);
                ep.set_bind_group(2, &gpu.sky_hat.buf_bg, &[]);
                ep.draw(0..3, 0..1);
            } else if let Some(sb) = self.sky_bind.as_ref() {
                ep.set_pipeline(&gpu.pipeline_sky);
                ep.set_bind_group(1, sb.as_ref(), &[]);
                ep.draw(0..3, 0..1);
            }
            ep.set_bind_group(3, &gpu.identity_obj.bg, &[]);
            ep.set_pipeline(&gpu.pipeline_opaque);
            let mut last_mode = u32::MAX;
            for sec in world.loaded.values().filter(|_| !env_dp) {
                let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
                let mut bound = false;
                for b in &sec.batches {
                    if matches!(b.mode, MatMode::Blend | MatMode::StoreGlass) || !ef.intersects_aabb(b.min, b.max) {
                        continue;
                    }
                    // (only what is big enough to show in so small a picture)
                    let ext = (b.max - b.min).length();
                    let dist = ((b.min + b.max) * 0.5).distance(env_eye);
                    if ext < dist * 0.08 || dist > 420.0 + ext {
                        continue;
                    }
                    if !bound {
                        ep.set_vertex_buffer(0, vb.slice(..));
                        ep.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                        bound = true;
                    }
                    ep.set_bind_group(1, b.bind.as_ref(), &[]);
                    if b.mode as u32 != last_mode {
                        ep.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                        last_mode = b.mode as u32;
                    }
                    ep.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                }
            }
            // The shop panes over the lit shops behind them (2026-10-14): left out of this pass
            // with everything blended, they let the store interiors - four times their lit-up
            // texture at night, in the original's map as here (VSCityStoreInteriorEnv hands the
            // same 4 on) - show uncovered, 3.3 times too bright: a yellow band along the
            // horizon of the night map where the original's shops are dim. MCLA_ENV_NO_PANES=1: as before.
            if std::env::var("MCLA_ENV_NO_PANES").is_err() {
                ep.set_pipeline(&gpu.pipeline_blend);
                let mut last_mode = u32::MAX;
                for sec in world.loaded.values().filter(|_| !env_dp) {
                    let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
                    let mut bound = false;
                    for b in &sec.batches {
                        if !matches!(b.mode, MatMode::StoreGlass) || !ef.intersects_aabb(b.min, b.max) {
                            continue;
                        }
                        let ext = (b.max - b.min).length();
                        let dist = ((b.min + b.max) * 0.5).distance(env_eye);
                        if ext < dist * 0.08 || dist > 420.0 + ext {
                            continue;
                        }
                        if !bound {
                            ep.set_vertex_buffer(0, vb.slice(..));
                            ep.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                            bound = true;
                        }
                        ep.set_bind_group(1, b.bind.as_ref(), &[]);
                        if b.mode as u32 != last_mode {
                            ep.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                            last_mode = b.mode as u32;
                        }
                        ep.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                    }
                }
            }
            if env_dp {
                ep.set_pipeline(&gpu.pipeline_env_fill);
                ep.set_bind_group(0, &gpu.env_fill_bg, &[]);
                ep.draw(0..3, env_face as u32..env_face as u32 + 1);
            }
            if env_glows > 0 {
                ep.set_pipeline(&gpu.pipeline_env_glow);
                ep.set_bind_group(0, &gpu.env_bgs[env_face], &[]);
                ep.set_vertex_buffer(0, gpu.env_glow_buf.slice(..));
                ep.draw(0..4, 0..env_glows);
            }
        }
        // ... and its blurred copy, for the paint's soft specular
        if env_frustum.is_some() {
            for face in 0..6u32 {
                let mut bp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("environment blur"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &gpu.env_soft_faces[face as usize],
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                bp.set_pipeline(&gpu.pipeline_env_blur);
                bp.set_bind_group(0, &gpu.env_blur_bg, &[]);
                bp.draw(0..3, face..face + 1);
            }
            // ... and its levels 1 .. 3, as the original's map has (PSFastMipMap; render/env_mip.wgsl)
            for (k, view) in gpu.env_mip_views.iter().enumerate() {
                let mut mp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("environment mips"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                mp.set_pipeline(&gpu.pipeline_env_mip);
                mp.set_bind_group(0, &gpu.env_mip_bgs[k], &[]);
                mp.draw(0..3, 0..1);
            }
        }
        render::ms::set_scene(gpu.scene_n);
        passprof::mark("city");
        // (the city pass as two passes - docs/PERFORMANCE.md 2026-10-24: A, the sky and the opaque sectors, recorded on
        // a worker into a command buffer of its own while this thread records B - the ground sheet, props, cars, people,
        // the blended sectors, glass and glows - into a pass that loads what A stored. The same draws in the same order
        // on the same targets: every draw of B sets its own bind groups and buffers, and B starts from the state A
        // leaves it (the globals, the identity object, the opaque pipeline). MCLA_CITY_ONE_PASS=1: one pass as before.)
        let split_city = std::env::var("MCLA_CITY_ONE_PASS").is_err();
        let sector_view = SectorView { gpu: &gpu, occluded: &occluded, frustum: &frustum, cam_pos, lens, only: self.opt.only.as_deref(), hide: self.opt.hide.as_deref() };
        let (sky_bind, citymap, loaded, sets) = (self.sky_bind.as_ref(), self.citymap.as_ref(), &world.loaded, &world.sets);
        // (2026-10-27 (3): A in two halves of the sector list, A1 with the sky, and B's ground sheet and props (B1) each on
        // a worker too, each a pass of its own that loads what the one before stored; this thread records the rest of B.
        // The halves split between sectors, and every sector sets its own object, buffers and binds: A2 and B2 start from
        // the state the pass before leaves - the globals, the identity object, the opaque pipeline, stencil reference 0)
        let secs: Vec<&world::GpuSector> = if map_baked { Vec::new() } else { loaded.values().chain(sets.values()).collect() };
        let half = {
            let total: usize = secs.iter().map(|s| s.batches.len()).sum();
            let mut acc = 0;
            secs.iter()
                .position(|s| {
                    acc += s.batches.len();
                    acc * 2 >= total
                })
                .map_or(secs.len(), |i| i + 1)
        };
        let city_a = |pass: &mut wgpu::RenderPass<'_>, draws: &mut usize, part: &[&world::GpuSector], first: bool, mut probe: Option<&mut gpuprof::Probe>| {
            let gpu = &gpu;
            pass.set_bind_group(0, &gpu.globals_bg, &[]);
            if first {
                // (the sky writes stencil 1: ps_main_withstarfield, ref 1 in 0x210D)
                pass.set_stencil_reference(1);
                if let Some(p) = probe.as_deref_mut() {
                    p.stamp(pass, || "sky".into());
                }
                if let (false, true, Some(tb)) = (map_baked, sky_hat_on, gpu.sky_hat.tex_bg.as_ref()) {
                    pass.set_pipeline(&gpu.sky_hat.pipeline);
                    pass.set_bind_group(1, tb, &[]);
                    pass.set_bind_group(2, &gpu.sky_hat.buf_bg, &[]);
                    pass.draw(0..3, 0..1);
                } else if let (false, Some(sb)) = (map_baked, sky_bind) {
                    pass.set_pipeline(&gpu.pipeline_sky);
                    pass.set_bind_group(1, sb.as_ref(), &[]);
                    pass.draw(0..3, 0..1);
                }
                pass.set_stencil_reference(0);
            }
            pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
            pass.set_pipeline(&gpu.pipeline_opaque);
            if let (true, true, Some(cm)) = (first, map_baked, citymap) {
                pass.set_bind_group(1, cm.bind.as_ref(), &[]);
                pass.set_bind_group(2, &gpu.mat_bgs[&(MatMode::Opaque as u32)], &[]);
                if let Some((vb, ib, n, _)) = cm.patch.as_ref() {
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..*n, 0, 0..1);
                    *draws += 1;
                    pass.set_bind_group(3, &cm.low.bg, &[]);
                }
                pass.set_vertex_buffer(0, cm.vb.slice(..));
                pass.set_index_buffer(cm.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..cm.count, 0, 0..1);
                pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
                *draws += 1;
            }
            if !map_baked {
                draw_sectors(pass, &sector_view, part.iter().copied(), 0, false, draws, probe.as_deref_mut());
                pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
            }
            if let Some(p) = probe {
                p.close(pass);
            }
        };
        // (the original's ground sheet: after the opaque city and the sky, before the props; then the instanced foliage)
        let (props, frustum_r, occluded_r, cam_eye) = (&self.props, &frustum, &occluded, self.cam.pos);
        let city_b1 = |pass: &mut wgpu::RenderPass<'_>, draws: &mut usize| {
            let gpu = &gpu;
            ground_sheet::draw(pass, gpu, vp_px);
            pass.set_pipeline(&gpu.pipeline_inst);
            pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
            props.draw_scene(pass, gpu, cam_eye, frustum_r, Some(occluded_r), draws);
        };
        // (the parts' command buffers in their order: A1, A2, B1)
        let mut parts_out: [Option<(wgpu::CommandBuffer, usize)>; 3] = [None, None, None];
        gpuprof::stamp(&gpu.device, &gpu.queue, &mut enc, "city A");
        let split_secs = split_city && !map_baked;
        let slot_b1 = if split_secs { gpuprof::slot(&gpu.device, &gpu.queue, "B props") } else { None };
        // (a probe frame of the GPU times: pass A's draws timed one by one - gpuprof.rs; the same draws)
        let [mut probe_a1, mut probe_a2] = match gpuprof::probe_frame(&gpu.device).filter(|_| split_secs) {
            Some([a, b]) => [Some(a), Some(b)],
            None => [None, None],
        };
        if split_city {
            early_cbs.push(std::mem::replace(&mut enc, gpu.device.create_command_encoder(&Default::default())).finish());
            gpuprof::stamp(&gpu.device, &gpu.queue, &mut enc, "city B");
        }
        mcla_flash::raster::pool().in_place_scope(|scope| {
            if split_city {
                let [o1, o2, o3] = &mut parts_out;
                let (city_a, city_b1, secs, gpu) = (&city_a, &city_b1, &secs, &gpu);
                let (pr1, pr2) = (&mut probe_a1, &mut probe_a2);
                scope.spawn(move |_| {
                    let mut e = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("city sky and sectors") });
                    let mut n = 0usize;
                    {
                        let mut pass = begin_city_pass(&mut e, gpu, clear, false);
                        city_a(&mut pass, &mut n, if split_secs { &secs[..half] } else { &secs[..] }, true, pr1.as_mut());
                    }
                    *o1 = Some((e.finish(), n));
                });
                if split_secs {
                    scope.spawn(move |_| {
                        let mut e = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("city sectors (2)") });
                        let mut n = 0usize;
                        {
                            let mut pass = begin_city_pass(&mut e, gpu, clear, true);
                            city_a(&mut pass, &mut n, &secs[half..], false, pr2.as_mut());
                        }
                        *o2 = Some((e.finish(), n));
                    });
                    scope.spawn(move |_| {
                        let mut e = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("city ground and props") });
                        gpuprof::write(&mut e, slot_b1);
                        let mut n = 0usize;
                        {
                            let mut pass = begin_city_pass(&mut e, gpu, clear, true);
                            pass.set_bind_group(0, &gpu.globals_bg, &[]);
                            city_b1(&mut pass, &mut n);
                        }
                        *o3 = Some((e.finish(), n));
                    });
                }
            }
            let mut pass = begin_city_pass(&mut enc, &gpu, clear, split_city);
            if split_city {
                pass.set_bind_group(0, &gpu.globals_bg, &[]);
                pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
                pass.set_pipeline(&gpu.pipeline_opaque);
            } else {
                city_a(&mut pass, &mut draws, &secs, true, None);
            }
            if !map_baked {
                if !split_secs {
                    gpuprof::stamp_pass(&gpu.device, &mut pass, "B props");
                    let d0 = draws;
                    city_b1(&mut pass, &mut draws);
                    stat_props = draws - d0;
                }
                gpuprof::stamp_pass(&gpu.device, &mut pass, "B cars");
                pass.set_pipeline(&gpu.pipeline_opaque);
                let d1 = draws;
                self.vehicles.draw(&mut pass, &gpu, false, &mut draws, Some((cam_pos, 450.0)), Some(&frustum));
                pass.set_stencil_reference(0);
                stat_cars = draws - d1;
                gpuprof::stamp_pass(&gpu.device, &mut pass, "B people");
                pass.set_pipeline(&gpu.pipeline_opaque);
                self.peds.draw(&mut pass, &gpu, false, &mut draws);
                self.cine.draw(&mut pass, &gpu, false, &mut draws);
                self.drivers.draw(&mut pass, &gpu, &mut draws);
                pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
                pass.set_pipeline(&gpu.pipeline_blend);
                // (shop panes write depth as the original's pane draws do; MCLA_DEBUG_VIEW=330: as before, no depth writes)
                let pane_zw = std::env::var("MCLA_DEBUG_VIEW").map_or(true, |v| v.trim() != "330");
                gpuprof::stamp_pass(&gpu.device, &mut pass, "B blended sectors");
                draw_sectors(&mut pass, &sector_view, loaded.values().chain(sets.values()), 1, pane_zw, &mut draws, None);
                pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
            }
            if !map3d {
                // (lighting, 2026-10-13: the glass props' panes - door leaves, glass walls: VSPropGlass / PSPropGlass,
                // drawn after the shop panes, blended, both sides, no depth writes; MCLA_DEBUG_VIEW=580: not drawn)
                gpuprof::stamp_pass(&gpu.device, &mut pass, "B glass props");
                if std::env::var("MCLA_DEBUG_VIEW").map_or(true, |v| v.trim() != "580") {
                    pass.set_pipeline(&gpu.pipeline_inst_blend);
                    pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
                    self.props.draw_glass(&mut pass, &gpu, self.cam.pos, &frustum, Some(&occluded), &mut draws);
                    pass.set_pipeline(&gpu.pipeline_blend);
                }
                gpuprof::stamp_pass(&gpu.device, &mut pass, "B car glass");
                self.vehicles.draw(&mut pass, &gpu, true, &mut draws, Some((cam_pos, 200.0)), Some(&frustum));
                pass.set_stencil_reference(0);
                gpuprof::stamp_pass(&gpu.device, &mut pass, "B glows");
                if self.time_of_day > 18.0 || self.time_of_day < 6.0 {
                    self.props.draw_glows(&mut pass, &gpu);
                }
                // (the signal lamps' glows are in the pass after the tone map now, halo.rs;
                // MCLA_DEBUG_VIEW=115: the sprites of before, here)
                if halo::old_look() {
                    self.props.draw_signal_glows(&mut pass, &gpu);
                }
                // (brake lamps glow by day too)
                self.vehicles.draw_glows(&mut pass, &gpu);
            }
            pass.set_pipeline(&gpu.pipeline_blend);
            pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
        });
        gpuprof::probe_end(&mut enc, [probe_a1, probe_a2].into_iter().flatten().collect());
        for (k, (cb, n)) in parts_out.into_iter().enumerate().filter_map(|(k, o)| o.map(|o| (k, o))) {
            early_cbs.push(cb);
            draws += n;
            if k == 2 {
                stat_props = n;
            }
        }
        passprof::mark("mirror+particles+post+ui");
        gpuprof::stamp(&gpu.device, &gpu.queue, &mut enc, "mirror+particles");
        // The original's hidden rear-view mirror (rearview.rs; OPTIONS > DISPLAY > REAR-VIEW MIRROR, DECISIONS.md D20;
        // MCLA_REARVIEW=1 turns it on for a run): after the main view's draws, before its particles and post chain,
        // while the driving HUD is up. Nothing of it runs while it is off.
        let rear_view = (self.game.opts.rear_view || std::env::var("MCLA_REARVIEW").map_or(false, |v| v.trim() == "1"))
            && !map3d
            && self.drive
            && matches!(self.game.mode, Mode::FreeRoam | Mode::Race)
            && !self.game.show_map
            && !self.game.idle_cam
            && self.game.cine.is_none()
            && !self.game.scene_hud_off;
        if let Some(pose) = self.vehicles.player().filter(|_| rear_view).map(|c| c.render_pose(self.vehicles.render_alpha)) {
            let mv = rearview::view(&globals, pose.pos, pose.rot, self.cam.near, (gpu.scene_w, gpu.scene_h));
            let mut lods = rearview::pick_lods(&self.vehicles, mv.eye, mv.axis, self.cam.near, &mv.frustum, &occluded);
            std::mem::swap(&mut self.vehicles.lod_pick, &mut lods);
            draws += rearview::draw(&mut enc, &gpu, &world, &self.vehicles, &mv);
            std::mem::swap(&mut self.vehicles.lod_pick, &mut lods);
        }
        if !map3d {
            // (the effect list's model particles - litter, cans - before them, writing the depth: modelfx.rs)
            modelfx::run(&gpu, &mut enc);
        }
        // (the depth and stencil resolved - sample 0 - for what reads them: the trails' fog, the particles' soft fade,
        // the blobs)
        gpu.resolve_scene_depth(&mut enc);
        if !map3d {
            // (the slipstream trails: drawn by the original last of the cars' alpha draws - sst_trail.rs; they write
            // no depth or stencil, so the resolve before them holds for what follows)
            sst_trail::run(&gpu, &mut enc, &self.vehicles, globals.view_proj);
        }
        if !map3d {
            // (the particles and the rain in a pass of their own, the depth read-only: the
            // checkpoint smoke reads it for its soft fade - PSCheckpointProj)
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("particles"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: None, stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &gpu.globals_bg, &[]);
            self.fx.draw(&mut pass, &gpu);
            rain::draw(&mut pass);
        }
        {
            // the original's post chain, tuned by the weather's keys; the radial blur follows the speed
            let (speed, nitro) = self.vehicles.player().filter(|_| self.drive).map(|c| (c.state.speed(), c.state.nitro_on())).unwrap_or((0.0, false));
            let moving = !self.game.in_menu();
            let mut radial = if moving { ((speed - 28.0) / 70.0).clamp(0.0, 1.0) * 0.085 } else { 0.0 };
            if nitro && moving {
                radial = radial * 1.6 + 0.03;
            }
            // abilities: Zone tunnels the view, the others kick it
            radial += self.ability.fx() * 0.05;
            let mut tune = self.post_tune(&self.sky_now());
            tune.dt = dt;
            // (the port's own radial speed blur is retired: the original has no such term - its motion blur is the
            // streak pass, render/mb.rs; work/ui-sharp/agent_tone. MCLA_RADIAL=1: as before)
            tune.radial = if post_on && std::env::var("MCLA_RADIAL").is_ok_and(|v| v.trim() == "1") { radial } else { 0.0 };
            tune.center = [0.5, 0.46];
            tune.plain = !post_on;
            // (test hook: MCLA_LUM = the adapted luminance to expose by, instead of the measured one)
            tune.force_lum = std::env::var("MCLA_LUM").ok().and_then(|v| v.parse().ok());
            if std::env::var("MCLA_TOD_DUMP").is_ok() && self.frame_no == 60 { eprintln!("post tune: grey {} white {} bright {:?} limits {:?} sat {} cc {:?}", tune.middle_grey, tune.white, tune.bright, tune.limits, tune.saturation, tune.color_correct); }
            // the lamps' glow pass: reads the scene's depth, the tone map adds its picture
            // the sun's glow quad over the finished scene (sunglow.rs), then the lamps' pass
            // the occlusion blobs under the cars (carblob.rs): over the finished scene, before the glow passes
            if !map3d {
                carblob::run(&gpu, &mut enc, &self.vehicles, self.cam.pos);
            }
            // (step 19g: the effects on the cars - ice blocks, shields, stealth - pu_fx.rs)
            if !map3d {
                pu_fx::run(&gpu, &mut enc, &self.vehicles, self.powerups.as_ref(), Vec3::new(globals.cam_pos[0], globals.cam_pos[1], globals.cam_pos[2]));
            }
            // (step 19f: the power-up sources' cards and ground glows - pu_draw.rs)
            if let Some(pu) = self.powerups.as_ref() {
                let (cards, glows) = pu_draw::instances(&pu.draw_list(), &pu.looks, self.cam.pos);
                // (the camera's right and up: the rows of the view the frame is drawn with)
                let v = if self.drive && self.vehicles.player().is_some() { self.chase.view() } else { self.cam.view() };
                let (r, u) = (Vec3::new(v.x_axis.x, v.y_axis.x, v.z_axis.x), Vec3::new(v.x_axis.y, v.y_axis.y, v.z_axis.y));
                pu_draw::run(&gpu, &mut enc, &cards, &glows, r, u);
            }
            // (the depth once more - the power-ups' passes write it - then the sun's glow, the last draw into the
            // multisampled scene, and the colour resolve: the samples averaged)
            gpu.resolve_scene_depth(&mut enc);
            sunglow::run(&gpu, &mut enc, self.sky.sun_color.rgb(self.time_of_day).to_array());
            gpu.resolve_scene_color(&mut enc);
            render::ms::set_plain();
            halo::run(&gpu, &mut enc);
            // the depth of field (post.rs: the far blur's buffer, lerped in by the tone map): the weather's
            // m_DoFParams at the hour; not in the garage or on the 3D map
            if !map3d && self.game.mode != Mode::Garage {
                if let Some(t) = self.sky_now().tod {
                    tune.dof = t.dof;
                }
                // (the start screen's shot: the cutscene depth of field when it sets one - titlecam.rs)
                if let Some(d) = self.titlecam.dof().filter(|_| self.game.mode == Mode::MainMenu) {
                    tune.dof = d;
                }
            }
            // (the garage has its own - garage round, 2026-10-13: DofParams of PSStreakMotionBlur in seven
            // recorded garage views, work/lighting/gar/dof.py: x = the camera's distance to the car's
            // place + 2 m, to the digit in all seven; y = x + 73; z = 0.8. TODGarage's own m_DoFParams,
            // 500 / 1000 / 0.75, is not what runs. MCLA_DEBUG_VIEW=712: without.)
            if !map3d && self.garage_light().is_some() && std::env::var("MCLA_DEBUG_VIEW").map_or(true, |v| v.trim() != "712") {
                if let Some(c) = self.vehicles.player() {
                    let x = (self.cam.pos - c.state.pos).length() + 2.0;
                    tune.dof = [x, x + 73.0, 0.8, 1.0];
                }
            }
            tune.inv_view_proj = globals.inv_view_proj;
            tune.cam_pos = [globals.cam_pos[0], globals.cam_pos[1], globals.cam_pos[2]];
            tune.mb_mtx = mb_mtx;
            // (OPTIONS > DISPLAY: BLOOM, DEPTH OF FIELD, MOTION BLUR - ON = the original's)
            tune.bloom = crate::hooks::bloom_on();
            tune.dof_on = crate::hooks::dof_on();
            tune.motion_blur = crate::hooks::motion_blur_on();
            gpuprof::stamp(&gpu.device, &gpu.queue, &mut enc, "post");
            gpu.post.run(&gpu.device, &gpu.queue, &mut enc, &view, &gpu.depth, &gpu.stencil_copy, &tune);
            gpuprof::stamp(&gpu.device, &gpu.queue, &mut enc, "ui");
            gpu.depth_to_ui(&mut enc);
        }
        {
            if let (Some(eg), Some((jobs, desc, _, n))) = (self.egui.as_mut(), egui_jobs.as_ref().filter(|j| j.3 > 1)) {
                let plain = gpu.config.format.remove_srgb_suffix();
                let (w, h) = (gpu.config.width, gpu.config.height);
                let (pass, r) = if *n == 2 { (&mut eg.ui2, &eg.renderer2) } else { (&mut eg.ui4, &eg.renderer4) };
                let p = pass.get_or_insert_with(|| uipass::Ui4::new(&gpu.device, gpu.config.format, plain, w, h, if *n == 2 { selfmovie::samples() } else { uipass::SAMPLES }));
                p.run(&gpu.device, &mut enc, &frame.texture, &ui_view, plain, r, jobs, desc);
            } else if let (Some(eg), Some((jobs, desc, _, 1))) = (self.egui.as_mut(), egui_jobs.as_ref()) {
                let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &ui_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                    // (read-only: egui writes no depth, and the GPS map's x-ray callbacks sample it - hudmap_gpu.rs)
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &gpu.depth_ui,
                        depth_ops: None,
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                let mut pass = pass.forget_lifetime();
                eg.renderer.render(&mut pass, jobs, desc);
            }
        }
        self.draws = draws;
        if self.frame_no % 240 == 0 {
            log::info!("draws: main {} (props {}, cars {}), shadow {} (sectors {})", draws, stat_props, stat_cars, stat_shadow, stat_shadow_sec);
        }

        passprof::mark("screenshot");
        // --- screenshot capture -------------------------------------------------
        let mut shot_buf = None;
        let photo = self.photo_request.take();
        if photo.is_some() {
            self.photo_msg = 2.0;
        }
        // (test hook MCLA_SHOT_EVERY=n[,from]: a screenshot run also writes <shot>_<frame>.png every n frames, from frame `from`)
        let photo = photo.or_else(|| {
            let v = std::env::var("MCLA_SHOT_EVERY").ok()?;
            let (n, from) = v.split_once(',').map_or((v.as_str(), "0"), |(a, b)| (a, b));
            let n = n.trim().parse::<u64>().ok().filter(|n| *n > 0)?;
            let from = from.trim().parse::<u64>().unwrap_or(0);
            let cfg = self.opt.shot.as_ref()?;
            (self.frame_no % n == 0 && self.frame_no >= from && self.frame_no < cfg.frames).then(|| format!("{}_{:05}.png", cfg.path.trim_end_matches(".png"), self.frame_no))
        });
        let exit_after = photo.is_none();
        let want = match (photo, self.opt.shot.clone()) {
            (Some(p), _) => Some(p),
            (None, Some(cfg)) if self.frame_no >= cfg.frames || self.game.shot_now => Some(cfg.path.clone()),
            _ => None,
        };
        if let Some(path) = want {
            {
                let w = gpu.config.width;
                let h = gpu.config.height;
                let bpr = (w * 4 + 255) / 256 * 256;
                let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("shot"), size: (bpr * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
                enc.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo { texture: &frame.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
                shot_buf = Some((buf, w, h, bpr, path));
            }
        }
        passprof::end();
        gpuprof::end(&gpu.device, &gpu.queue, &mut enc);
        let t_enc = now.elapsed().as_secs_f32() * 1000.0;
        gpu.queue.submit(early_cbs.into_iter().chain([enc.finish()]));
        gpuprof::after_submit();
        window.pre_present_notify();
        frame.present();
        let t_end = now.elapsed().as_secs_f32() * 1000.0;
        if self.framelog.is_some() {
            self.fl_prev = [ticks as f32, sim_ms, self.fl_stream_ms, t_sim - sim_ms, t_acq - t_sim, t_ui - t_acq, t_enc - t_ui, t_end - t_enc, world.loaded.len() as f32];
        }
        // (the frame's parts, the mean of the 240 frames since the last line: the game ticks, the frame's preparation,
        // the swap chain, the UI, recording the passes, submit and present - docs/PERFORMANCE.md)
        thread_local!(static PHASES: std::cell::Cell<([f64; 6], u32)> = const { std::cell::Cell::new(([0.0; 6], 0)) });
        PHASES.with(|c| {
            let (mut acc, mut n) = c.get();
            for (a, v) in acc.iter_mut().zip([sim_ms, t_sim - sim_ms, t_acq - t_sim, t_ui - t_acq, t_enc - t_ui, t_end - t_enc]) {
                *a += v as f64;
            }
            n += 1;
            if self.frame_no % 240 == 0 {
                let m: Vec<f64> = acc.iter().map(|a| a / n as f64).collect();
                log::info!("phases: ticks {:.2} ms, prep {:.2}, acquire {:.2} + ui {:.2}, passes {:.2}, submit+present {:.2} (mean of {} frames)", m[0], m[1], m[2], m[3], m[4], m[5], n);
                (acc, n) = ([0.0; 6], 0);
            }
            c.set((acc, n));
        });
        // (MCLA_ENV_DUMP=<file>: with the screenshot, the cars' environment maps as numbers)
        if shot_buf.is_some() {
            if let Ok(p) = std::env::var("MCLA_ENV_DUMP") {
                gpu.dump_env(&p);
            }
            // (MCLA_POST_DUMP=<file>: the bright pass's quarter-size scene and the finished bloom)
            if let Ok(p) = std::env::var("MCLA_POST_DUMP") {
                gpu.post.dump(&gpu.device, &gpu.queue, &p);
            }
            // (MCLA_CASC_DUMP=<file>: the sun's shadow cascades' depth atlas)
            if let Ok(p) = std::env::var("MCLA_CASC_DUMP") {
                gpu.dump_casc(&p);
            }
            // (MCLA_FC_DUMP=<file>: the fast shadow collector's map)
            if let Ok(p) = std::env::var("MCLA_FC_DUMP") {
                gpu.dump_fc(&p);
            }
            // (MCLA_MINISKY_DUMP=<file>: the mini sky the panes mirror)
            if let Ok(p) = std::env::var("MCLA_MINISKY_DUMP") {
                gpu.dump_minisky(&p);
            }
            // (MCLA_REFL_DUMP=<file>: the wet road's planar reflection picture)
            if let Ok(p) = std::env::var("MCLA_REFL_DUMP") {
                gpu.dump_refl(&p);
            }
        }
        if let Some((buf, w, h, bpr, path)) = shot_buf {
            let slice = buf.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            gpu.device.poll(wgpu::Maintain::Wait);
            if let Ok(Ok(())) = rx.recv() {
                let data = slice.get_mapped_range();
                let bgra = gpu.config.format == wgpu::TextureFormat::Bgra8UnormSrgb || gpu.config.format == wgpu::TextureFormat::Bgra8Unorm;
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h {
                    let row = &data[(y * bpr) as usize..(y * bpr + w * 4) as usize];
                    for px in row.chunks_exact(4) {
                        if bgra {
                            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                        } else {
                            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                        }
                    }
                }
                drop(data);
                buf.unmap();
                if let Ok(f) = std::fs::File::create(&path) {
                    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
                    enc.set_color(png::ColorType::Rgba);
                    enc.set_depth(png::BitDepth::Eight);
                    if let Ok(mut wr) = enc.write_header() {
                        let _ = wr.write_image_data(&rgba);
                    }
                    log::info!("screenshot written to {}", path);
                }
            }
            if exit_after {
                hooks::flush_log();
                std::process::exit(0);
            }
        }
        if let (Some(eg), Some((_, _, free, _))) = (self.egui.as_mut(), egui_jobs) {
            for id in free {
                eg.renderer.free_texture(&id);
                eg.renderer4.free_texture(&id);
                eg.renderer2.free_texture(&id);
            }
        }
        self.world = Some(world);
        self.gpu = Some(gpu);
        let action = std::mem::replace(&mut self.pending_action, MenuAction::None);
        self.handle_action(action);
    }

    fn handle_action(&mut self, action: MenuAction) {
        if action == MenuAction::None {
            return;
        }
        let (Some(gpu), Some(world)) = (self.gpu.as_mut(), self.world.as_mut()) else { return };
        // (a pink slip race quit is lost: `Dlg_Desc_Pinkslip_Quit` - step 21)
        let mut pink_quit = false;
        if action == MenuAction::QuitRace && self.game.mc4_pink_on() {
            if let Some(key) = self.game.mc4_pink_end(false) {
                let m = self.game.career.strings.get(key).map(|s| s.to_uppercase()).unwrap_or_else(|| "PINKSLIP LOST".into());
                self.game.show_message(&m);
                // (step 21b: the staked vehicle is gone at once - out of the garage since the start)
                pink_quit = true;
            }
        }
        // (ARCADE / GOAL ATTACK's ACCEPT on a race with the police on the player and a fine pays the
        // fine first - F-15258: the manager lets go of the player (F-41497, approx.: the port's police
        // reset), the profile's money less the fine with no floor at 0 (F-38145: min 0x80723100), the
        // save written; then the race is set up)
        if action == MenuAction::StartRace && std::mem::take(&mut self.game.fine_pay) && self.game.police_fine > 0 && self.game.police_on_player {
            self.game.money = (self.game.money - self.game.police_fine).max(-2_140_007_168);
            log::info!("police: fine {} paid starting the race, money now {}", self.game.police_fine, self.game.money);
            self.police.reset(&mut self.vehicles);
            self.game.police_fine = 0;
            self.game.police_on_player = false;
            self.game.write_save(&self.opt.data_dir);
        }
        // hand the delivery car back when the event is left
        if self.game.borrowed && (pink_quit || matches!(action, MenuAction::StartFreeRoam | MenuAction::MainMenu | MenuAction::OpenGarage | MenuAction::OpenRaceSelect | MenuAction::OpenArcade(_))) {
            self.game.borrowed = false;
            if let Some(dir) = self.game.car_dirs.get(self.game.current_car).cloned() {
                let color = self.game.paint_color();
                // (MCLA_COLOR=n: a palette swatch for screenshots)
                let color = std::env::var("MCLA_COLOR").ok().and_then(|v| v.parse::<usize>().ok()).map_or(color, |i| vehicles::PALETTE[i % vehicles::PALETTE.len()]);
                let rim = self.game.rim_dirs.get(self.game.cfg.rim % self.game.rim_dirs.len().max(1)).cloned();
                let cfg = self.game.cfg.clone();
                self.vehicles.player_rim_inches = self.game.rim_inches;
                let _ = self.vehicles.replace_player(gpu, world, &dir, color, &cfg, rim.as_deref());
                let tuned = self.game.perf_frac(self.game.current_car);
                if let Some(p) = self.vehicles.player_mut() {
                    p.set_perf(tuned);
                    p.state.damage = 0.0;
                }
            }
        }
        match action {
            MenuAction::None => {}
            MenuAction::OpenMap | MenuAction::OpenEditor => {
                self.game.mode = Mode::FreeRoam;
                self.game.show_map = true;
                if action == MenuAction::OpenEditor {
                    self.game.editor = Some(Vec::new());
                    if let Some(p) = self.vehicles.player().map(|c| c.state.pos) {
                        self.game.editor_cursor = p;
                    }
                }
            }
            MenuAction::StartFreeRoam | MenuAction::QuitRace => {
                // (START on the start screen: Warper_Warp('MAXHEIGHT') from the shot - startwarp.rs)
                if let (true, Some((eye, look, lens))) = (self.game.mode == Mode::MainMenu && std::env::var("MCLA_NO_START_WARP").is_err(), self.titlecam.view()) {
                    let car = self.vehicles.player().map_or(eye, |c| c.state.pos);
                    self.startwarp = Some(startwarp::StartWarp::start(eye, look, lens, car, gpu.config.width as f32 / gpu.config.height.max(1) as f32));
                    self.startwarp_t = 0.0;
                }
                // (the race object goes: its destructor's Race_Reset, F-83102 - kMRS, xRRT)
                if self.game.race.is_some() && !self.game.goal_attack {
                    self.game.mc4_race_reset();
                }
                if let Some(sp) = self.game.alt_leave(action == MenuAction::QuitRace) {
                    log::info!("alt hookman: {} leaving the race", sp);
                    self.speech.say_as(sp, &self.game.althk.voice);
                }
                self.game.event_diff = None;
                // (back to the cruise leaves GOAL ATTACK: F-65087 clears the mode's +796)
                self.game.goal_attack = false;
                self.game.series = None;
                self.game.series_first = false;
                // (leaving a tournament - RETURN TO CRUISE, QUIT, GO TO GARAGE, a race quit - ends
                // the attempt: TournamentCore's leave 0x2a7f; the next challenge is a new core, race 1
                // at nought points and a new field. A won one stays as it is.)
                self.tour_field = None;
                if self.game.career.tour_table.as_ref().map_or(true, |t| t.decided != Some(true)) {
                    if let Some(id) = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).map(|d| d.id.clone()) {
                        let _ = self.game.career.tournament_restart(&id);
                    }
                }
                self.game.career.tour_table = None;
                self.game.series_ask = None;
                self.game.wager = None;
                self.game.race = None;
                self.game.flag = None;
                self.game.current_race = None;
                if !self.vehicles.models.iter().any(|m| m.name.starts_with("va_")) {
                    // ambient traffic set
                    let root = world.assets.path("resources/vehicle");
                    let mut dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
                        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.file_name().map_or(false, |n| {
                            let n = n.to_string_lossy();
                            n.starts_with("va_") && !n.contains("semitruck") && !n.contains("limo")
                        })).collect())
                        .unwrap_or_default();
                    dirs.sort();
                    let mut loaded = 0;
                    for (i, d) in dirs.iter().enumerate() {
                        if i % 4 == 1 && loaded < 9 && self.vehicles.load_model(gpu, world, d).is_ok() {
                            loaded += 1;
                        }
                    }
                }
                if !self.vehicles.models.iter().any(|m| m.name == police::COP_CAR) {
                    let d = world.assets.path("resources/vehicle").join(police::COP_CAR);
                    if let Err(e) = self.vehicles.load_model(gpu, world, &d) {
                        log::warn!("police car: {e}");
                    }
                }
                self.police.reset(&mut self.vehicles);
                self.vehicles.cars.retain(|c| c.is_player);
                self.vehicles.traffic_target = self.opt.traffic.max(10);
                self.vehicles.parked_target = 36;
                self.game.mode = Mode::FreeRoam;
                self.game.menu_sel = 0;
                self.game.pink_over = None;
            }
            // Warper_Warp('DEFAULT_WITH_CUT') (a series' / tournament's next race, its restart, a tournament's
            // first race - CAREER_RACES.md "DEFAULT_WITH_CUT climb"): the camera climbs from where it is
            // (`hudmap::CineClimb`); the race is set up once it is over (IsReadyToLoadStuff), the dive
            // onto the start scene then starts from the climb's end. RESTART from a series' or a
            // tournament race's pause too (OrderedSeriesCore / TournamentCore `restart`).
            MenuAction::StartRace | MenuAction::RestartRace
                if self.climb.is_none()
                    && self.climb_end.is_none()
                    && self.climb_wait.is_none()
                    && (std::mem::take(&mut self.game.warp_with_cut)
                        || (action == MenuAction::RestartRace && (self.game.series.is_some() || self.game.current_race.and_then(|i| self.game.race_defs.get(i)).map_or(false, |d| self.game.career.tournament_of(&d.id).is_some()))))
                    && std::env::var("MCLA_NO_CLIMB").is_err()
                    && self.vehicles.player().is_some() =>
            {
                let p = self.vehicles.player().unwrap();
                let e = hudmap::CamRows { r0: p.state.rot * Vec3::X, r1: p.state.rot * Vec3::Y, r2: p.state.rot * Vec3::Z, eye: p.state.pos };
                let look = (self.chase.look - self.chase.pos).normalize_or(Vec3::NEG_Z);
                let s = hudmap::CamRows::look_at(self.cam.pos, self.cam.pos + look, self.chase.up);
                let c = hudmap::CineClimb::new(s, e, self.cam.fov_y, 25f32.to_radians(), self.screen_v);
                log::info!("DEFAULT_WITH_CUT climb: {:?} ({:.3} s) from {:?} over the car at {:?}", c.kind(), c.total(), s.eye, e.eye);
                self.climb = Some((c, action));
                self.climb_end = None;
                self.climb_hold = 0;
                self.game.climb_on = true;
            }
            // (a tournament's first race and a payback open their panel first: `mc4_event_panel_first`)
            MenuAction::StartRace if self.game.mc4_event_panel_first() => {}
            // ACCEPT on the payback's panel (`Hangouts/0xfc2c6c0a` 0x0c3d): PoliceManager_DestroyAllPolice, the player
            // Racer_ZeroVelocity + Racer_Deactivate, "ease from pause" (form 68, 0.5 s), then
            // Warper_TransitionTo('STORY_LOCATION_CUT_SIMPLE', 513, eye, up point, look-at) - the Beaches garage's
            // wait camera; the race is set up once it is ready (the per-frame part)
            MenuAction::StartRace if self.game.payback_keys && self.payback_in.is_none() && self.vehicles.player().is_some() => {
                self.game.payback_keys = false;
                self.police.destroy_all(&mut self.vehicles);
                let c = self.vehicles.player_mut().unwrap();
                c.state.vel = Vec3::ZERO;
                c.state.ang_vel = Vec3::ZERO;
                let hold = (c.state.pos, c.state.rot);
                self.game.hangout_slow.set(1.0, 1.0 / 0.5, false, true, 1.0);
                self.game.hangout_slow.visible = false;
                self.game.mode = Mode::FreeRoam;
                let e = garagein::wait_rows(&garagein::SETUPS[PAYBACK_PLACE_CAM]);
                let look = (self.chase.look - self.chase.pos).normalize_or(Vec3::NEG_Z);
                let s = hudmap::CamRows::look_at(self.cam.pos, self.cam.pos + look, self.boom_up.unwrap_or(Vec3::Y));
                let warp = (s.eye.distance(e.eye) < garagein::CLOSE).then(|| hudmap::CineWarp::target_close(s, e, self.cam.fov_y, garagein::LENS_DEG.to_radians()));
                log::info!("payback warp: from {:?} to {:?} (Warp_Target_Close {}), tick {}", s.eye, e.eye, warp.is_some(), self.tick_no);
                self.payback_in = Some((0.0, warp, hold));
            }
            MenuAction::StartRace | MenuAction::RestartRace => {
                // (p080: delivery_hangout_core plays the walk to the car only when the panel is
                // accepted; a restart or the next delivery goes straight to the intro)
                let p080_again = action == MenuAction::RestartRace || self.game.race.as_ref().map_or(false, |r| r.kind == game::RaceKind::Delivery);
                // (the race scripts' `restart`: kTRS, xRRT, kARS; then the set-up's Race_Reset - F-83102)
                if action == MenuAction::RestartRace {
                    self.game.mc4_restart_stats();
                }
                if !self.game.goal_attack {
                    self.game.mc4_race_reset();
                }
                self.prerace_replay = action == MenuAction::RestartRace;
                // (the last pink slip race's race-over tab is not this race's)
                self.game.pink_over = None;
                // (a race started from another's results - NEXT RACE IN SERIES, CONTINUE, RESTART - never
                // passes through `race == None`, so the last race's start plan and warp go here)
                self.prerace_plan = None;
                self.prerace_wait = 0;
                self.warp = None;
                self.warp_view = None;
                self.delivery_intro = None;
                self.delivery_tail = None;
                // (RESTART SERIES from a race's pause: its first race again, the tally at nought)
                if action == MenuAction::RestartRace {
                    if let Some(sr) = self.game.series.as_mut() {
                        sr.at = 0;
                        sr.wins.clear();
                        self.game.current_race = sr.races.first().copied().or(self.game.current_race);
                        self.game.next_event_race = None;
                    }
                }
                if action == MenuAction::RestartRace {
                    if let Some((stake, _)) = self.game.wager {
                        if self.game.money >= stake {
                            self.game.money -= stake;
                        } else {
                            self.game.wager = None;
                        }
                    }
                }
                self.game.race = None;
                self.game.flag = None;
                self.game.results_plain = false;
                // the field: named rivals in their cars of the player's performance tier
                if action == MenuAction::StartRace {
                    let pname = self.vehicles.player().map(|c| c.model.name.clone()).unwrap_or_default();
                    let tier = rivals::tier(&self.game.catalog.get(&pname).map(|c| c.group.clone()).unwrap_or_default());
                    let dirs = self.game.car_dirs.clone();
                    // (a car the port can load; the original races motorcycles too)
                    let usable = |car: &str| dirs.iter().any(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(car)));
                    let mut grid: Vec<rivals::Rival> = Vec::new();
                    // (a series keeps its field: the event's own man and two more)
                    // (a pink slip race is one against one, the opponent in the car raced for -
                    // wager_hangout_core 0x54c3..0x557d: step 21)
                    let pink = self.game.mc4_pink_rival();
                    let field = if pink.is_some() { 1 } else if self.game.series.is_some() { 3 } else { 7 };
                    if let Some((id, car, cfg)) = pink.clone().filter(|m| usable(&m.1)) {
                        grid.push(self.game.rivals.with_config(&id, &car, &cfg));
                    }
                    let kept = self.game.series.as_ref().map(|s| s.rivals.clone()).unwrap_or_default();
                    if !kept.is_empty() {
                        grid = kept;
                    } else if let Some(key) = self.game.series.as_ref().map(|s| s.key.clone()) {
                        if let Some(r) = self.game.career.event_rival_char(&key).and_then(|id| self.game.rivals.of(&id, tier)).filter(|r| usable(&r.car)) {
                            grid.push(r);
                        }
                    }
                    // (step 19d: a generic local hookman leads his race's grid himself, in his
                    // vehicle and set-up)
                    if self.game.mc4_local_man_race() {
                        if let Some((id, car, cfg)) = self.game.opt_x.arcade.d.race_man.clone().filter(|m| usable(&m.1)) {
                            grid.push(self.game.rivals.with_config(&id, &car, &cfg));
                        }
                    }
                    // (step 19d: the red light racer races in his own car and set-up)
                    if self.game.current_race.and_then(|i| self.game.race_defs.get(i)).map_or(false, |d| d.id.ends_with("red_light_race") || d.id == althk::FREEWAY_RACE_ID) {
                        if let Some((id, car, cfg)) = self.game.opt_x.arcade.d.redlight_man.clone().filter(|m| usable(&m.1)) {
                            grid.push(self.game.rivals.with_config(&id, &car, &cfg));
                        }
                    }
                    if self.game.hookman_race {
                        // the challenged hookman leads the grid
                        if let Some(r) = self.game.career.rival_id().and_then(|id| self.game.rivals.of(&id, tier)).filter(|r| usable(&r.car)) {
                            grid.push(r);
                        }
                    }
                    let tour_event = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).and_then(|d| self.game.career.tournament_of(&d.id)).map(|t| t.0);
                    if let (Some(ev), Some((fe, kept))) = (tour_event.as_ref(), self.tour_field.as_ref()) {
                        if ev == fe {
                            grid = kept.clone();
                        }
                    }
                    // (a man's values belong to his own race only)
                    let alt = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).map_or(false, |d| d.id.ends_with("red_light_race") || d.id == althk::FREEWAY_RACE_ID);
                    if !(self.game.hookman_race || self.game.mc4_local_man_race() || alt) {
                        self.game.man_mod = None;
                        self.game.man_diff = None;
                    }
                    // the rest of the field: the race script's pick (oppick.rs) - cars of the difficulty's band
                    // around the player's car, each with the first free driver who has it
                    let d = self.game.grid_difficulty();
                    self.game.grid_diff = Some(d);
                    let masks = self.game.grid_masks();
                    let p = std::env::var("MCLA_PLAYER_PERF").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| self.game.player_perf());
                    let used: Vec<String> = grid.iter().map(|r| r.id.clone()).collect();
                    let need = (field as usize).saturating_sub(grid.len());
                    let seed = oppick::state_seed(&oppick::career_rng());
                    let picks = self.game.oppick.pick(need, p, d, masks, &used, &usable);
                    // (a mod car with rims = "random" wears a car rim drawn from the race's seed per set-up; stage S4b)
                    let shop_rims: Vec<i32> = self.game.rim_shop.car_rims().into_iter().filter(|r| self.game.rim_by_name.contains_key(r.1)).map(|r| r.0).collect();
                    let draws: Vec<(String, i32)> = picks.iter().enumerate().filter(|(_, pk)| modcars::rims_random(&pk.car)).filter_map(|(i, pk)| Some((pk.setup.clone(), modcars::draw_rim(&shop_rims, seed, i)?))).collect();
                    for (s, r) in &draws {
                        log::info!("grid rim: {s} wears {} (rim table {r})", self.game.rim_shop.entry(*r).map_or("?", |e| e.resource.as_str()));
                    }
                    modcars::set_rim_draws(draws);
                    for pk in picks {
                        log::info!("grid pick: {} drew {} -> {} in {} ({}), difficulty {} player {:.4}", if pk.driver.is_empty() { "-" } else { &pk.driver }, pk.drawn, pk.driver, pk.car, pk.setup, d, p);
                        grid.push(self.game.rivals.with_config(&pk.driver, &pk.car, &pk.setup));
                    }
                    if let Some(ev) = tour_event {
                        self.tour_field = Some((ev, grid.clone()));
                    }
                    if let Some(s) = self.game.series.as_mut() {
                        if s.rivals.is_empty() {
                            s.rivals = grid.clone();
                        }
                    }
                    // (the grid's cars read side by side on worker threads first, then built - docs/PERFORMANCE.md 2026-10-23 (5))
                    for r in &grid {
                        if let Some(d) = dirs.iter().find(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(&r.car))) {
                            self.vehicles.prefetch(world, d);
                        }
                        // (and their set-ups' rims, tyres and brakes into the file cache, for their dressing)
                        if let Some(d) = self.game.mc4_dress_of(&r.car, &r.config) {
                            vehicles::warm_dress(&world.assets.root, &d);
                        }
                    }
                    for r in &grid {
                        if !self.vehicles.models.iter().any(|m| m.name.eq_ignore_ascii_case(&r.car)) {
                            if let Some(d) = dirs.iter().find(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(&r.car))) {
                                let _ = self.vehicles.load_model(gpu, world, d);
                            }
                        }
                    }
                    log::info!("grid: {}", grid.iter().map(|r| format!("{} {} [{}]", self.game.career.name_of(&r.id), r.car, r.plate)).collect::<Vec<_>>().join(", "));
                    self.game.grid_rivals = grid;
                }
                // (diagnostics, docs/AI.md: the cars MCLA_GRID_CARS puts on the grid)
                if let Ok(v) = std::env::var("MCLA_GRID_CARS") {
                    let dirs = self.game.car_dirs.clone();
                    for n in v.split(',').map(|n| n.trim()) {
                        if !self.vehicles.models.iter().any(|m| m.name.eq_ignore_ascii_case(n)) {
                            if let Some(d) = dirs.iter().find(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(n))) {
                                let _ = self.vehicles.load_model(gpu, world, d);
                            }
                        }
                    }
                }
                let racers = self.vehicles.models.iter().filter(|m| !m.name.starts_with("va_")).count();
                if racers < 5 {
                    // load opponent cars spread over the car list
                    let dirs = self.game.car_dirs.clone();
                    let step = (dirs.len() / 5).max(1);
                    let mut have = racers;
                    for (i, d) in dirs.iter().enumerate() {
                        // motorcycles need their own physics model; keep them out of the grid
                        let bike = d.file_name().map_or(false, |n| {
                            let n = n.to_string_lossy();
                            n.contains("_duc_") || n.contains("_kaw_")
                        });
                        if bike {
                            continue;
                        }
                        if i % step == step / 2 && have < 5 {
                            if self.vehicles.load_model(gpu, world, d).is_ok() {
                                have += 1;
                            }
                        }
                    }
                }
                // deliveries are driven in the customer's car
                if let Some(def) = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).cloned() {
                    // (a time trial is driven in the vehicle of the row chosen, with its set-up's
                    // share of upgrades: `mc4_tt_run`)
                    let s18_tt = self.game.mc4_tt_run(&def.id);
                    // (step 19d: a payback is driven in the garage's car -
                    // `pyback_plr_vp_buk_grandnational_87`, PaybackCore 0x26; the run's later
                    // legs go on in it as it stands)
                    let s18_tt = s18_tt.or_else(|| (def.race_type == "Payback").then(|| (mc4::PAYBACK_PLAYER.0.to_string(), mc4::PAYBACK_PLAYER.1.to_string(), 0.0)));
                    let s19d_keep = def.race_type == "Payback" && self.game.borrowed && self.vehicles.player().map_or(false, |c| c.model.name.eq_ignore_ascii_case(mc4::PAYBACK_PLAYER.0));
                    let s18_car = match s18_tt.as_ref() {
                        Some((car, _, _)) => car.clone(),
                        None if def.race_type == "Delivery" => def.car.clone(),
                        None => String::new(),
                    };
                    let s18_perf = s18_tt.as_ref().map_or(0.0, |(car, cfg, _)| self.game.mc4_setup_percent(car, cfg).clamp(0, 100) as f32 / 100.0);
                    if !s18_car.is_empty() && !s19d_keep {
                        let dir = world.assets.path("resources/vehicle").join(&s18_car);
                        if dir.is_dir() {
                            let col = vehicles::PALETTE[(def.name.len() * 5) % vehicles::PALETTE.len()];
                            // (a time trial's vehicle wears its set-up's paint: the `.xcc`'s colour)
                            let col = s18_tt.as_ref().and_then(|(_, cfg, _)| self.game.rivals.paint_of(cfg)).map_or(col, |c| [c[0], c[1], c[2], 1.0]);
                            // (step 19d: the body's first colour is the set-up's paint entry, 0x660 -
                            // `mc4::setup_look`; with it the set-up's other paints, tires and brakes)
                            let col = s18_tt.as_ref().and_then(|(_, cfg, _)| self.game.mc4_borrow_look(cfg)).unwrap_or(col);
                            // (step 19c: in its set-up's parts, on its rims at their size -
                            // `mc4_setup_dress`; a delivery's vehicle as before)
                            let s19a_dress = s18_tt.as_ref().and_then(|(car, cfg, _)| self.game.mc4_setup_dress(car, cfg));
                            let (s19a_cfg, s19a_rim) = match s19a_dress {
                                Some((c, r, inches)) => {
                                    if inches > 0 {
                                        self.vehicles.player_rim_inches = inches as u8;
                                    }
                                    (c, r)
                                }
                                None => (vehicle_model::CarConfig::default(), None),
                            };
                            if self.vehicles.replace_player(gpu, world, &dir, col, &s19a_cfg, s19a_rim.as_deref()).is_ok() {
                                self.game.borrowed = true;
                                if let Some(p) = self.vehicles.player_mut() {
                                    p.set_perf(s18_perf);
                                    p.state.damage = 0.0;
                                }
                            }
                        }
                    }
                }
                let flag_def = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).filter(|d| flags::FlagGame::is_flag_event(d)).cloned();
                let started = match (flag_def, self.game.current_race) {
                    (Some(def), _) => {
                        let career = &self.game.career;
                        let fg = flags::FlagGame::start(&def, &mut self.vehicles, gpu, &self.game.grid_rivals, &|id| career.name_of(id), self.frame_no as u32 ^ 0x5bd1e995);
                        let ok = fg.is_some();
                        self.game.flag = fg;
                        if ok {
                            self.game.mode = Mode::Race;
                        }
                        ok
                    }
                    (None, Some(i)) => {
                        // (step 19d: the payback leg's client, in a vehicle of the run's table)
                        if self.game.race_defs.get(i).map_or(false, |d| d.race_type == "Payback") {
                            let (car, _, _) = self.game.mc4_payback_pick();
                            if !self.vehicles.models.iter().any(|m| m.name.eq_ignore_ascii_case(&car)) {
                                let dir = world.assets.path("resources/vehicle").join(&car);
                                if dir.is_dir() {
                                    let _ = self.vehicles.load_model(gpu, world, &dir);
                                }
                            }
                        }
                        // (test hook MCLA_ARCADE_OPTS: the race as ARCADE's, with those options)
                        self.game.mc4_arcade_hook(i);
                        self.game.start_race_def(i, world, &mut self.vehicles, gpu)
                    }
                    (None, None) => self.game.start_race(world, &mut self.vehicles, gpu),
                };
                // (a race against a red light or freeway man: its checkpoints' radius; a freeway
                // race goes on from where the two are - althk.rs)
                if started {
                    self.game.alt_after_start(&mut self.vehicles);
                }
                // a delivery from the garage lot opens with the walk to the customer's car
                // (p080, the original's order - delivery_hangout_core `Hangouts/0x37c63b19`: ACCEPT ->
                // warp -> START_NEW_SCRIPT(cut_delivery_get_keys, from the district's set-up
                // `Hangouts/0x4dd86889` / sc 0xc8c6051d) -> the leg: RAND_INT_RANGE(0, 2) picks
                // gen_prerace_delivery_1x1_01/02/03 into the race data (+3196), the car is changed,
                // the race script's core plays it at the grid's first slot; a restart / the next
                // delivery skip the walk. Walkthrough 2:44:20..2:44:50 and 2:46:18..2:46:36.)
                if started {
                    if let Some(def) = self.game.current_race.and_then(|i| self.game.race_defs.get(i)).filter(|d| d.race_type == "Delivery").cloned() {
                        let intro = hudmap::delivery_intro_pick((self.frame_no as u32).wrapping_mul(2654435761) >> 7);
                        let (g0, yaw) = def.grid[0];
                        self.delivery_intro = Some((intro.scene.to_string(), g0, yaw));
                        if !p080_again {
                            for sc in ["racestart/cut_delivery_get_keys", "racestart/sc_cut_delivery_get_keys"] {
                                if let Some((p, _)) = self.cine.car_pose(sc, &|l| l == "delivery", false) {
                                    if def.grid.first().map_or(false, |g| g.0.distance(p) < 90.0) {
                                        self.game.prerace_scene = Some(sc.to_string());
                                        self.game.prerace = Some((p, 0.0, String::new()));
                                    }
                                }
                            }
                        }
                        if self.game.prerace_scene.is_none() {
                            self.delivery_intro = None;
                            self.game.prerace_scene = Some(intro.scene.to_string());
                            self.game.prerace = Some((g0, yaw, String::new()));
                        }
                        log::info!("delivery start: {} - walk {}, intro {}", def.id, self.game.prerace_scene.as_deref().map_or(false, |s| s.contains("get_keys")), intro.scene);
                    }
                }
                // the payback's first leg after its panel: PaybackCore plays `cut_payback_get_keys` (staged in the
                // world, the player's car its `drv00`) and the leg goes from where the scene leaves the car - no warp
                // (RunPayBack 0x06d5, main local3 = 0). A restart and the later legs as before.
                if started && self.payback_hold {
                    let sc = "racestart/cut_payback_get_keys";
                    let pose = self.cine.car_pose(sc, &|l| cine::is_player_car(l), false);
                    let payback = self.game.race.as_ref().map_or(false, |r| r.kind == game::RaceKind::Payback);
                    if let (true, Some((p, q))) = (payback && self.cine.has(sc), pose) {
                        if let Some(i) = self.vehicles.cars.iter().position(|c| c.is_player) {
                            self.vehicles.pose_car(i, p, q);
                        }
                        let f = q * Vec3::NEG_Z;
                        self.game.prerace_scene = Some(sc.to_string());
                        self.game.prerace = Some((p, f.x.atan2(-f.z), String::new()));
                        log::info!("payback start: keys scene, the car at {:?}", p);
                    } else {
                        self.payback_hold = false;
                        self.place_view = None;
                    }
                }
                // the race's own dressing: ramps and flares, scripted traffic, held lights
                if let Some(rt) = self.dress.take() {
                    rt.unload(&mut self.vehicles);
                }
                self.props_dirty = true;
                self.vehicles.signals.release();
                self.vehicles.road_density.clear();
                self.vehicles.road_density_new.clear();
                self.props.signals.release();
                if started && std::env::var("MCLA_NO_DRESS").is_err() {
                    if let Some(d) = self.game.active_def.as_ref().or(self.game.current_race.and_then(|i| self.game.race_defs.get(i))).and_then(|def| racescript::load(&self.opt.data_dir, &def.id, def.checkpoints.first().copied())) {
                        log::info!("race script: {} props, {} triggers, {} scripted cars, checkpoint radii {:?}", d.props.len(), d.triggers.len(), d.paths.len(), d.cp_radius);
                        if let Some(r) = self.game.race.as_mut() {
                            // (the checkpoints' activation radii as the script sets them - what the power-up
                            // layout reads, +168 of each, F-28343; no longer raised to 9 m)
                            if !r.cp_specs.is_empty() {
                                r.cp_radius = r.cp_specs.iter().map(|s| s.radius).collect();
                            } else if d.cp_radius.len() == r.lap_len && r.kind == game::RaceKind::Race {
                                r.cp_radius = d.cp_radius.clone();
                            }
                        }
                        let rt = racescript::Runtime::new(d, self.frame_no as u32 ^ 0x9e3779b9);
                        // (the race core's ResetTriggers resets the global arrays too, F-55754)
                        for z in self.dispatch_zones.iter_mut() {
                            z.done = false;
                        }
                        // their vehicle types (a few at a time would be kinder; there are rarely more than a dozen)
                        let root = world.assets.path("resources/vehicle");
                        for t in rt.types().into_iter().take(14) {
                            if !self.vehicles.models.iter().any(|m| m.name.eq_ignore_ascii_case(&t)) {
                                let dir = root.join(&t);
                                if dir.is_dir() {
                                    let _ = self.vehicles.load_model(gpu, world, &dir);
                                }
                            }
                        }
                        self.dress = Some(rt);
                    }
                }
                if started {
                    // street races are run in (somewhat thinner) traffic
                    let have_traffic = self.vehicles.models.iter().any(|m| m.name.starts_with("va_"));
                    self.vehicles.traffic_target = if !have_traffic || std::env::var("MCLA_RACE_NO_TRAFFIC").is_ok() { 0 } else { (self.opt.traffic.max(10) * 2 / 3).max(6) };
                    // (step 19c: an ARCADE race's TRAFFIC, TIME OF DAY and WEATHER - the menu's
                    // ACCEPT, F-15258: the density scale 0 / 0.5 / 1, the clock warped, the
                    // weather set at once; Dynamic leaves clock and weather be - `mc4_arcade_env`)
                    // (the scale enters the spawner's target: F-49549; level 0 switches the
                    // traffic off - AmbientMgr_SetTrafficEnable)
                    self.vehicles.density_scale = None;
                    if let Some(env) = self.game.mc4_arcade_env() {
                        if env.traffic == 0.0 {
                            self.vehicles.traffic_target = 0;
                        } else {
                            self.vehicles.density_scale = Some(env.traffic);
                        }
                        if let Some(h) = env.hour {
                            self.time_of_day = h;
                        }
                        if let Some(w) = env.weather {
                            let w = [sky::Weather::Nice, sky::Weather::Cloudy, sky::Weather::Stormy, sky::Weather::Foggy][w as usize % 4];
                            if w != self.weather {
                                self.sky = sky::Sky::load_weather(&self.opt.data_dir, w);
                                self.weather = w;
                                self.sky_next = None;
                            }
                        }
                        log::info!("arcade: options record {:02x?}, traffic x {} -> {} cars, hour {:?}, weather {:?}", self.game.mc4_arcade_record(), env.traffic, self.vehicles.traffic_target, env.hour, env.weather);
                    }
                    // (GOAL ATTACK's START RACE: SetAllPoliceUnderground(1, 1) - F-15258
                    // A-96349; the race object's end hook brings them up again, F-84893)
                    if self.game.goal_attack {
                        self.police.units_down(&mut self.vehicles);
                        log::info!("goal attack: race started, police units underground");
                    }
                    // (step 19e: an ARCADE race with POWER UPS on or Custom - the record's byte
                    // 21, EventOptions_IsPowerUps - has its pick-up sources: the race script's
                    // own list, else laid out from the checkpoints - ordered races 1, unordered 0)
                    if let Some(mut pu) = self.powerups.take() {
                        pu.clear(&mut self.vehicles, &mut self.ability);
                    }
                    let rec = self.game.mc4_arcade_record();
                    if self.game.mc4_arcade_env().is_some() && rec[21] != 0 {
                        if let Some(r) = self.game.race.as_ref() {
                            let def = self.game.active_def.as_ref().or(self.game.current_race.and_then(|i| self.game.race_defs.get(i)));
                            let spots: Vec<powerups::Spot> = match def.and_then(|d| powerups::script_list(&self.opt.data_dir, &d.id)) {
                                Some(l) => l.iter().flat_map(|(p, w, q)| powerups::hand_spots(*p, *w, *q)).collect(),
                                None => powerups::checkpoint_spots(&powerups::race_checkpoints(r), r.rivals_checkpoints.is_none()),
                            };
                            log::info!("powerups: {} sources for {} racers, record {:02x?}", spots.len(), r.participants.len(), &rec[..2]);
                            self.powerups = Some(powerups::PowerUps::new(&self.opt.data_dir, &rec, spots, &r.participants, self.frame_no as u32 ^ 0x5eed_1e));
                        }
                    }
                    self.vehicles.parked_target = 24;
                    self.chase.pos = self.vehicles.player().map(|c| c.state.pos - c.state.forward() * 7.0 + Vec3::Y * 2.5).unwrap_or(self.chase.pos);
                    self.game.show_message("");
                } else {
                    self.game.show_message("Could not build a race here, drive to a street first");
                    self.game.mode = Mode::FreeRoam;
                }
                self.game.menu_sel = 0;
            }
            MenuAction::OpenArcade(goal) => {
                // (the type list first, the flag set: F-40647 / F-35999)
                self.game.goal_attack = goal;
                self.game.arcade.0 = 0;
                if self.game.race_defs.is_empty() {
                    self.pending_action = MenuAction::StartRace;
                } else {
                    self.game.mode = Mode::RaceSelect;
                }
            }
            MenuAction::OpenRaceSelect => {
                if self.game.race_defs.is_empty() {
                    self.pending_action = MenuAction::StartRace;
                } else {
                    self.game.mode = Mode::RaceSelect;
                }
            }
            MenuAction::PhotoMode | MenuAction::PhotoAlbum => {
                // the scene stays frozen (still paused); the camera flies free from where it was
                // (SnapShot, or PM_PhotoAlbum straight into the album - `photoalbum`)
                self.game.album.begin(action == MenuAction::PhotoAlbum);
                self.photo = Some((self.game.mode, self.drive));
                self.cam.pos = self.chase.pos;
                let d = (self.chase.look - self.chase.pos).normalize_or(Vec3::Z);
                self.cam.yaw = d.x.atan2(-d.z);
                self.cam.pitch = d.y.asin();
                self.drive = false;
                self.game.photo_on = true;
            }
            // GO TO GARAGE (F-17595): the chosen garage gets state 4 and the Career_Loading box comes up; the menu goes, and
            // garage_core's drive-in takes it from there (the warp, the scene, the garage)
            MenuAction::GoToGarage => {
                let pp = self.vehicles.player().map(|c| c.state.pos);
                if let Some(g) = self.game.go_to_garage(pp) {
                    log::info!("go to garage: {} (running {:?})", garagein::SETUPS[g].key, [0, 1, 2].map(|g| self.game.garage_running(g)));
                    self.game.mode = Mode::FreeRoam;
                    self.goto_garage = Some(g);
                }
            }
            MenuAction::OpenGarage => {
                self.game.career.event(career::Ev::Garage);
                // (the garage does not repair the car by itself: it asks, before MANAGE VEHICLES /
                // CUSTOMIZE VEHICLE / VEHICLE SHOWROOM for a dented car and before the way out for
                // a wreck - `Game::mc4_repair_gate`; a car goes in, stands there and is put away
                // with the damage it has)
                self.game.prev_mode = match self.game.mode {
                    Mode::Paused => Mode::Paused,
                    Mode::FreeRoam => Mode::FreeRoam,
                    _ => Mode::MainMenu,
                };
                self.game.selected_car = self.game.current_car;
                self.game.mode = Mode::Garage;
                // (back from a test drive: the showroom again, as `garage.WarpToGarage` leaves it)
                if self.game.test_drive.take().is_some() {
                    self.game.showroom_again = true;
                }
            }
            MenuAction::TestDrive => {
                // the showroom's vehicle as it stands there, lent like a delivery's car (it is
                // handed back when the garage is entered again - `borrowed`)
                if let Some(dir) = self.game.test_drive.and_then(|i| self.game.car_dirs.get(i).cloned()) {
                    let color = vehicles::PALETTE[0];
                    if self.vehicles.replace_player(gpu, world, &dir, color, &vehicle_model::CarConfig::default(), None).is_ok() {
                        self.game.borrowed = true;
                        if let Some(p) = self.vehicles.player_mut() {
                            p.set_perf(0.0);
                            p.state.damage = 0.0;
                            // (its own plate, not the player's)
                            p.plate_text = String::new();
                            p.plate = None;
                        }
                        self.game.mode = Mode::FreeRoam;
                    } else {
                        self.game.test_drive = None;
                    }
                }
            }
            MenuAction::ApplyCar => {
                let car_changed = self.game.selected_car != self.game.current_car;
                if car_changed {
                    self.crash_cam.new_car();
                }
                if car_changed {
                    // each vehicle keeps its own parts: those of the one left are put by, the
                    // taken one's brought out (a vehicle never fitted keeps the rim on, as before)
                    let left = self.game.car_dirs.get(self.game.current_car).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    let taken = self.game.car_dirs.get(self.game.selected_car).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    let old = self.game.cfg.clone();
                    self.game.cfg = match self.game.cfgs.get(&taken) {
                        Some(c) => vehicle_model::CarConfig { rim: old.rim, ..c.clone() },
                        None => vehicle_model::CarConfig { rim: old.rim, ..Default::default() },
                    };
                    if !left.is_empty() {
                        self.game.cfgs.insert(left, old);
                    }
                    // (an imported save's rim for the vehicle taken)
                    let was = std::mem::replace(&mut self.game.current_car, self.game.selected_car);
                    self.game.apply_imported_rim();
                    self.game.current_car = was;
                }
                let dir = self.game.car_dirs[self.game.selected_car].clone();
                let color = self.game.paint_color();
                // (MCLA_COLOR=n: a palette swatch for screenshots)
                let color = std::env::var("MCLA_COLOR").ok().and_then(|v| v.parse::<usize>().ok()).map_or(color, |i| vehicles::PALETTE[i % vehicles::PALETTE.len()]);
                let rim = self.game.rim_dirs.get(self.game.cfg.rim % self.game.rim_dirs.len().max(1)).cloned();
                let cfg = self.game.cfg.clone();
                self.vehicles.player_rim_inches = self.game.rim_inches;
                // (in the garage the vehicle taken stands exactly where the one before stood)
                let stand = (self.game.mode == Mode::Garage).then(|| self.vehicles.player().map(|c| (c.state.pos, c.state.rot))).flatten();
                match self.vehicles.replace_player(gpu, world, &dir, color, &cfg, rim.as_deref()) {
                    Ok(()) => {
                        if let (Some((pos, rot)), Some(c)) = (stand, self.vehicles.player_mut()) {
                            let old = std::mem::replace(&mut c.state, physics::car::CarState::new(pos + Vec3::Y * 0.05, 0.0));
                            c.state.keep_damage(&old);
                            c.pose_history.reset();
                            c.state.rot = rot;
                            c.placed = true;
                        }
                        self.game.current_car = self.game.selected_car;
                        // performance package bought for this car (lower levels stay selectable)
                        let tuned = self.game.perf_frac(self.game.current_car);
                        if let Some(p) = self.vehicles.player_mut() {
                            p.set_perf(tuned);
                        }
                        let n = self.game.car_name(self.game.current_car);
                        self.game.show_message(&format!("{} ready", n));
                        self.game.write_save(&self.opt.data_dir);
                    }
                    Err(e) => {
                        log::error!("garage: {:#}", e);
                        self.game.show_message("Failed to load that car");
                    }
                }
                if !car_changed {
                    self.game.mode = self.game.prev_mode;
                }
                self.game.dirty = false;
            }
            MenuAction::Back => {
                log::debug!("back from {:?} (prev {:?}, in a garage {})", self.game.mode, self.game.prev_mode, self.garage_return.is_some());
                self.game.mode = match self.game.mode {
                    // (out to the street: the garage fades to black first, then the drive-out scene comes
                    // up with its fade_to_scene - `garagein::EXIT_FADE`)
                    Mode::Garage if self.game.prev_mode == Mode::FreeRoam && self.garage_return.is_some() && std::env::var("MCLA_NO_CINE").is_err() => {
                        if self.garage_out.is_none() {
                            self.garage_out = Some(0.0);
                            self.garage_mask.start(garagein::EXIT_FADE, 0.01, f32::MAX);
                        }
                        Mode::Garage
                    }
                    Mode::Garage => self.game.prev_mode,
                    // (out of a place's own panel - a hangout's, the delivery's - back to the cruise)
                    Mode::RaceSelect if self.hangout_in.is_some() => Mode::FreeRoam,
                    Mode::RaceSelect => Mode::Paused,
                    // (START / Esc where the original's gate says no: nothing - `pause_allowed`)
                    Mode::FreeRoam | Mode::Race if !self.pause_allowed() => self.game.mode,
                    Mode::FreeRoam | Mode::Race => Mode::Paused,
                    _ => Mode::MainMenu,
                };
                self.game.menu_sel = 0;
            }
            MenuAction::RestartTournament => {
                if self.game.tournament_restart() {
                    // (TournamentCore's `restart`: kTRS, xRRT, kARS)
                    self.game.mc4_restart_stats();
                    self.pending_action = MenuAction::StartRace;
                    // (TournamentCore `restart` 0x2191: Warper_Warp('DEFAULT_WITH_CUT'))
                    self.game.warp_with_cut = true;
                }
            }
            MenuAction::GasStation(what) => {
                // (FULL FIX: the car seated upright where it stands and fully repaired, no money taken
                // - G_MoneySpent reads "FREE" -, QUICK_FIX_MASTER; REFILL NITRO: every canister,
                // NITROUS_REFILL_MASTER even when full. Then the form closes and the game resumes:
                // F-35780 / F-41930, F-81587)
                if let Some(fix) = what {
                    if let Some(p) = self.vehicles.player_mut() {
                        if fix {
                            // (F-12181, the garage's full repair: the scrapes go too)
                            p.state.damage = 0.0;
                            p.state.dmg.reset();
                            p.state.zones = [0.0; 4];
                        } else {
                            p.state.nitro_shots = p.state.nitro_max;
                        }
                    }
                    if fix {
                        // (and the unpainted flags cleared, the tyres' effects killed: F-12181, F-74884)
                        self.game.mc4_set_unpainted([false; 6]);
                        self.kill_player_tyre_fx();
                    }
                    if let Some(a) = self.audio.as_mut() {
                        a.obj(if fix { "QUICK_FIX_MASTER" } else { "NITROUS_REFILL_MASTER" });
                    }
                    log::info!("gas station: {}", if fix { "FULL FIX" } else { "REFILL NITRO" });
                }
                // the form's close (F-81587): input back, the event camera off over 0.7 s, the
                // Idle event's clock 0, the game resumed
                if let Some(name) = self.vehicles.player().map(|c| c.model.name.clone()) {
                    self.event_dir.off_with(&self.opt.data_dir, &name, 0.7);
                }
                self.idle_time = 0.0;
                self.game.mode = if self.game.race.is_some() || self.game.flag.is_some() { Mode::Race } else { Mode::FreeRoam };
            }
            MenuAction::Resume => {
                self.game.mode = if self.game.race.is_some() || self.game.flag.is_some() { Mode::Race } else { Mode::FreeRoam };
            }
            MenuAction::MainMenu => {
                self.game.race = None;
                self.game.flag = None;
                self.vehicles.cars.retain(|c| c.is_player);
                self.game.mode = Mode::MainMenu;
                self.game.menu_sel = 0;
            }
            MenuAction::Quit => {
                self.quit = true;
            }
        }
    }

    fn spawn_player(&mut self, gpu: &mut Gpu, world: &mut World) {
        let Some(dir) = self.opt.car_dir.clone() else { return };
        match self.vehicles.load_model(gpu, world, std::path::Path::new(&dir)) {
            Ok(model) => {
                let mut pos = self.cam.pos + self.cam.forward() * 8.0;
                let mut yaw = self.cam.yaw + std::f32::consts::PI;
                if let Some(nav) = world.nav.as_ref() {
                    let connected: usize = nav.links.iter().filter(|l| !l.next.is_empty() && !l.prev.is_empty()).count();
                    log::info!("navsys connectivity: {} of {} links connected", connected, nav.links.len());
                    let mut best: Option<(usize, f32, f32)> = None;
                    for li in nav.links_near(pos, 150.0) {
                        let l = &nav.links[li];
                        if l.next.is_empty() || l.prev.is_empty() {
                            continue;
                        }
                        let t = l.project(pos);
                        let d = l.point_at(t).distance(pos);
                        if best.map_or(true, |b| d < b.2) {
                            best = Some((li, t, d));
                        }
                    }
                    // (MCLA_POS_EXACT: start exactly at --pos instead of on the nearest road)
                    if let Some((li, t, _)) = best.or_else(|| nav.nearest_link(pos)).filter(|_| std::env::var("MCLA_POS_EXACT").is_err() && (self.exact_start.is_none() || self.opt.auto)) {
                        let l = &nav.links[li];
                        let h = l.dir();
                        pos = l.point_at(t) + Vec3::Y * 1.0;
                        yaw = (-h.x).atan2(-h.z);
                    }
                }
                // (the self-driving test car starts on the street instead)
                if let Some((p, y)) = self.exact_start.filter(|_| !self.opt.auto) {
                    pos = p + Vec3::Y * 0.6;
                    yaw = y;
                }
                self.vehicles.spawn(gpu, model, pos, yaw, self.game.paint_color(), true);
                self.chase.pos = self.cam.pos;
                self.chase.look = pos;
                self.vehicles.traffic_target = self.opt.traffic;
                // (F-31099 puts the density scale back to 1)
                self.vehicles.density_scale = None;
                self.vehicles.parked_target = if self.opt.traffic > 0 { 36 } else { 0 };
            }
            Err(e) => log::error!("car load failed: {:#}", e),
        }
        if self.opt.traffic > 0 {
            let d = world.assets.path("resources/vehicle").join(police::COP_CAR);
            if let Err(e) = self.vehicles.load_model(gpu, world, &d) {
                log::warn!("police car: {e}");
            }
            let root = world.assets.path("resources/vehicle");
            let mut dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.file_name().map_or(false, |n| {
                    let n = n.to_string_lossy();
                    // the game's whole ambient traffic set: the spawner picks any type of the
                    // district's list (traffic.rs), semi cabs with their trailers
                    n.starts_with("va_")
                })).collect())
                .unwrap_or_default();
            dirs.sort();
            let t0 = std::time::Instant::now();
            for d in dirs.iter() {
                if let Err(e) = self.vehicles.load_model(gpu, world, d) {
                    log::warn!("traffic model {}: {:#}", d.display(), e);
                }
            }
            log::info!("traffic models: {} loaded in {:.1} s", dirs.len(), t0.elapsed().as_secs_f32());
        }
    }
}

impl App {
    /// Play one of the converted disc movies ("intro", "attract") with its sound track.
    /// The phone's MISSION LOG (Back) line under a mission's call. A new career has none
    /// until the mission log has been explained (`ng_c4`: the first call's strip is bare).
    fn mission_log_prompt(&self) -> String {
        let st = &self.game.career.stats;
        if st.contains_key("tut:sidekick") && !st.contains_key("tut:mission_log") {
            return String::new();
        }
        self.game.career.strings.get("Mission_Log").map(|s| s.to_uppercase()).unwrap_or_else(|| "MISSION LOG".into())
    }

    /// The new career's car at the kerb outside the lot: where the first story scene has the player's car (the
    /// `drv00` mover of CUT_STORY_RACE_GH_06). MCLA_STARTER_HERE leaves it where it is.
    fn place_player_at_kerb(&mut self) {
        if std::env::var("MCLA_STARTER_HERE").is_ok() {
            return;
        }
        if let Some((p, r)) = self.cine.car_pose("story/cut_story_race_gh_06", &cine::is_player_car, false) {
            if let Some(c) = self.vehicles.player_mut() {
                c.state.relocate(p + Vec3::Y * 0.3, r);
                c.state.vel = Vec3::ZERO;
                c.state.ang_vel = Vec3::ZERO;
                c.pose_history.reset();
            }
        }
    }

    /// A story scene's audio as its picture comes up (F-58551): the sound objects' `CUTSCENES_<scene>`
    /// (dialogue, effects stream, mixer scene) - or, without them, the converted dialogue stream. The speech and the
    /// police radio stop (the object's flags 0x1 / 0x2: every story scene has both). INTRO FAST's CUT_INTRO in the
    /// film's place (DECISIONS.md D24) plays the film's own sound from where the film shows the same picture: the
    /// phone call of the arrival is in the film's sound track only (CUT_INTRO_MASTER has the bus, no voice).
    fn start_scene_audio(&mut self, stream: Option<&str>) {
        let Some((name, t)) = self.cuts.playing.as_ref().map(|p| (p.def.name.clone(), p.t)) else { return };
        self.speech.stop();
        self.radio.stop();
        let Some(a) = self.audio.as_mut() else { return };
        // (INTRO FAST's stand-in: the scene's mixer scene with the film's sound in place of the scene's own)
        let film = name == "CUT_INTRO" && self.intro_stage == 2;
        self.cut_audio_objects = a.obj_cutscene(&name, true, !film);
        if film {
            let path = std::path::Path::new(&self.opt.data_dir).join("../video_pc/intro.mp3");
            log::info!("cutscene audio: CUT_INTRO in the film's place - the film's sound from {:.2} s", FILM_CUT_INTRO_AT + t);
            a.play_stream_at(path.to_str(), 1.0, FILM_CUT_INTRO_AT + t);
        } else if self.cut_audio_objects {
            if t > 0.0 {
                a.obj_cutscene_seek(t);
            }
        } else {
            a.play_stream(stream, 1.0);
        }
    }

    fn start_movie(&mut self, name: &str, subs: Vec<(f32, f32, String)>, any_key: bool) -> bool {
        let dir = std::path::Path::new(&self.opt.data_dir).join("../video_pc");
        let Some(mut m) = video::Video::open(&dir, name) else { return false };
        m.subs = subs;
        m.any_key = any_key;
        let at = std::env::var("MCLA_MOVIE_T").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        self.speech.stop();
        if let Some(a) = self.audio.as_mut() {
            a.play_stream_at(m.audio.to_str(), 1.0, at);
        }
        m.restart(at);
        self.game.movie = Some(m);
        true
    }

    /// Whether START (the keyboard's Esc) opens the pause menu now. The original: the HUD state's
    /// event handler (F-88073, mcHudState `Invisible`) takes start_pressed (event 55) only when
    /// the pad has a player and the race is not in its last three seconds before GO, then asks
    /// F-38972, which says no while (in its order): the perspective form (108, the "ease to
    /// pause" slow motion) is visible; there is no player on the pad; a race is in its last 3000 ms
    /// before GO (race +3764, F-98822 in [-3000, 0) - F-54121); the race is over (+3773,
    /// offline); an open form does not allow it (vt+672); the registry's `disablePause` is set (only
    /// the CruisingHookmen scripts set it, around a hookman's challenge); GameState_Loading or
    /// GameState_Intro is active; a cinematic plays (0x828CD440, the cine manager +288 / +324 - +328
    /// window - F-46334 / F-52539); the player's car is wrecked (+36 / +24 >= 1,
    /// F-36476). VEHICLE SELECT is CarSelection's own loop: it polls only left / right / A
    /// (IS_BUTTON_DOWN 13 / 3, 15 / 2, 6) - START does nothing there; the opening before the car is
    /// given (save boxes, film, VEHICLE SELECT, its black) and the story scenes are cinematics or the
    /// intro state. Docs: UI.md "Where START opens the pause menu (2026-10-23)".
    /// The phone's update (F-37697) brings up no call while the police level F-35540(1) is 1 or more (pull-over,
    /// pursuit, search, arrest) or -1 (a dispatch to the player) unless that dispatch responds (state 2, F-85485 ->
    /// F-48923), while a career speech line plays (the speech manager's +2256 and +2260), a scene script runs
    /// (F-52539: the ticket and arrest scenes), the warper holds the camera (`camera_held`) or the UI is off its HUD
    /// (`ui_off_hud`); the calls wait in the queue. (The phone's own hold after a call is `phone_hold`.)
    fn phone_held(&self) -> bool {
        let (level, dsp) = self.police.activity();
        level >= 1 || (level == -1 && dsp < 2)
            || self.speech.busy()
            // (F-37697's other flag, +0x51350 (F-55498), is set only by Warper_SetWarping (F-59289), which no script
            // calls - none of the 1771 script files: always clear)
            || self.camera_held()
            || self.ui_off_hud()
            || self.cuts.playing.is_some() || self.cine.playing.is_some() || self.game.movie.is_some()
            || self.ticket_run.is_some() || self.arrest_form.is_some() || self.game.arrest_panel.is_some() || self.arrest_station.is_some()
    }

    /// F-37697's argument from UILogic's update (F-28336 passes !F-85659 || Game_IsPaused): the UI's current
    /// state (F-85659: vhsmState vt#78 F-13804, every parent's current child down to it) is neither `Invisible` (the HUD,
    /// tune/ui/UserInterface.sc.xml) nor `Tutorial` (pushed only by the mission log's lesson form, F-79137), or the pause
    /// manager has the game paused (+4 set, +10 clear: game.pause() - the pause and race-over menus, every GPS / mission
    /// log scene, the photo album, the save scenes). Off the HUD: the menu modes (pause menu, garage menus, results,
    /// VEHICLE SELECT), the map, photo mode and the album. The lessons' boxes are forms, not UI states: the HUD stays
    /// current and their OnShow only pauses the simulation (no pause flag) - they hold no call.
    fn ui_off_hud(&self) -> bool {
        self.game.in_menu() || self.game.show_map || self.game.album.stage != crate::photoalbum::Stage::Off
    }

    /// UILogic_IsCameraPaused (F-23025): the warper holds the camera, UILogic +0x51382. Set by F-83799 when a cinematic
    /// request starts (F-35052) and, at a warp's end (F-99030), when the request asks for the camera to stay (its +7:
    /// pause camera at end of warp - DEFAULT_WITH_CUT's climb, the start screen's Warper_SetPauseCameraAtEndOfWarp; a
    /// STORY / LOCATION cut holds it on its own matrix, F-99229); cleared when the next flight starts (the
    /// cinematic warp's F-18267, the climb's F-98643 - both F-70736), by the warper's loading states 12 / 17
    /// once Career_Loading goes (F-31347), or by UILogic_UnPauseCamera (F-20502: CarSelection 0x917, the career's load
    /// 0x6fc, FIND_SH_02C's core 0xfe7 after its scene's fade). Nothing counts it down (+0x51384 is always -1).
    /// A STORY_LOCATION_CUT_SIMPLE (Warper_TransitionTo, F-68460 -> F-86808 -> Warp() F-32081, type 17: +7 = 1)
    /// flies at once (F-98643, or F-47096 when the camera is within 100 m - both clear the hold), holds at the
    /// flight's end (F-99030 -> F-99229, the same moment IsReadyToLoadStuff turns true) and lets go when the scene is
    /// up: state 17 once the scene's time is >= 0 (F-31347: F-62707, then F-98849 / F-48345 -> F-70736), state 15
    /// (the far case) at its cinematic flight (state 26, F-18267). So FIND_SH_02C's UnPauseCamera after its fade finds
    /// the hold clear (Warper_SetPauseAtEndOfCut, F-97499 -> F-30425, sets the request's +8, which nothing reads).
    /// Port: the camera held after the climb until the race warp's flight (`climb_end`, `climb_wait`). The warps'
    /// flights are hold-free; the ones out of Cruising_Offline are `out_of_cruise_warp`.
    fn camera_held(&self) -> bool {
        self.climb_end.is_some() || self.climb_wait.is_some()
    }

    /// The warps whose script has left Cruising_Offline before it warps: the race scripts throw their race event
    /// before Warper_Warp (race/0xc4e26a20 0x21e8, PaybackCore 0x642 / 0x6b9), the climb comes after mc.seriesRace /
    /// mc.tournamentRace (race/0xb8b948b6 0x2d82 / 0x34f7, race/0x039dd301 0xf75 / 0x2139), garage_core throws
    /// mc.garage (0x43e) before its warp (0x4ad), the payback place mc.paybackHangout (0x72a) before its (0xd3f),
    /// and the career loader throws mc.cruising only after the start-up flight (0x62e9486d: USER_WAIT_NO_CUT_DOWN
    /// 0x674, mc.cruising 0x6ce, UnPauseCamera 0x6fc). The port's mode and race state do not show these yet. The
    /// local and wager hangouts' entries warp inside Cruising_Offline (0xe166126d: TransitionTo 0x926,
    /// mc.localHangout 0x99f once IsReadyToLoadStuff; 0x16e89a34 0xdc7 / 0xe40): a call can come up during their
    /// ease and flight. The garage's way out runs in the garage's menu mode (mc.cruising at 0x1b99).
    fn out_of_cruise_warp(&self) -> bool {
        self.game.start_warp_on || self.warp.is_some() || self.climb.is_some() || self.climb_end.is_some() || self.climb_wait.is_some()
            || self.garage_in.is_some() || self.payback_in.is_some()
    }

    fn pause_allowed(&self) -> bool {
        let g = &self.game;
        // (the new career's opening: the save boxes, the film, VEHICLE SELECT, the black to the first scene)
        let opening = g.fresh_game || g.starter_pick.is_some() || g.hold_starter || g.opening_black || self.intro_stage != 0;
        // (cinematics: story scenes, a race's opening shots, the film)
        let cinematic = self.cuts.playing.is_some() || self.cine.playing.is_some() || g.movie.is_some();
        // (the warper's moves: the start-up flight, the race-start warp, the climb, garage / hangout /
        // payback entries and the garage's way out - *(approx.)* which of the gates above holds them is
        // not read; the port's warps hold the car as a scene does)
        let warping = g.start_warp_on || self.warp.is_some() || self.climb.is_some() || self.garage_in.is_some() || self.garage_out.is_some() || self.hangout_entry.is_some() || self.payback_in.is_some();
        // (the race's last three seconds before GO, and the race's end - PostRace / race over)
        let race_gate = g.race.as_ref().map_or(false, |r| r.countdown > 0.0 && r.countdown <= 3.0) || g.race_end_t.is_some();
        // (the perspective form's slow motion holds the game: the lessons' "ease to pause")
        let form108 = g.tutorial_hold();
        // (the player's car wrecked: F-36476 - damage / its limit >= 1)
        let wrecked = self.vehicles.player().map_or(false, |c| c.state.damage >= 1.0);
        !(opening || cinematic || warping || race_gate || form108 || g.photo_on || wrecked)
    }

    /// The new career's opening after the save boxes (CarSelection, `intro_stage`): every tick, the film and
    /// the scenes included.
    fn opening_tick(&mut self, dt: f32, streaming: usize) {
        self.intro_t += dt;
        // (VEHICLE SELECT's loop: one step of the camera function at 0x1069 toward the car under the cursor, then
        // WAITUNWARPED(30))
        if let (3, Some(k), Some(c)) = (self.intro_stage, self.game.starter_pick, self.sel_cam.as_mut()) {
            c.4 += dt;
            while c.4 >= 0.030 {
                c.4 -= 0.030;
                let (eye, look, se, sl) = sel_cam_step((c.0, c.1, c.2, c.3), SEL_EYE[k.min(2)], SEL_LOOK[k.min(2)]);
                *c = (eye, look, se, sl, c.4);
            }
        }
        match self.intro_stage {
            // (the film once the black is up; settings.toml game.intro `fast` puts CUT_INTRO in its place, `skip` (OFF)
            // keeps it - the original always plays it, GitHub #8 - `hooks::intro_film`)
            1 if self.intro_mask.full() && self.starters_made => {
                // (a new career's held camera goes under the black - the career script's UILogic_UnPauseCamera)
                if self.startwarp.take().is_some() {
                    log::info!("start warp: released under CarSelection's black");
                }
                let film = hooks::intro_film() && (self.opt.shot.is_none() || std::env::var("MCLA_MOVIE").is_ok());
                // (the film carries the phone call: the subtitles of CUT_INTRO's table)
                let subs = self.cuts.by_name("CUT_INTRO").map(|i| self.cuts.scenes[i].subs.clone()).unwrap_or_default();
                if film && self.start_movie("intro", subs, false) {
                    log::info!("opening: the film");
                } else if !hooks::intro_film() && hooks::intro_scene() && std::env::var("MCLA_NO_CUTS").is_err() && (self.opt.shot.is_none() || std::env::var("MCLA_CUTS").is_ok()) {
                    // (INTRO FAST, the user's own path - DECISIONS.md D24: the film is left out but the arrival by
                    // bus stays, as the scene CUT_INTRO in its place; the original has no such path)
                    if let Some(i) = self.cuts.by_name("CUT_INTRO") {
                        log::info!("opening: the arrival scene in place of the film (INTRO FAST)");
                        self.cuts.queue(i);
                    }
                }
                self.intro_stage = 2;
                self.intro_t = 0.0;
                // (the camera over the lot, set after the film: CarSelection's eye local45 and look local48)
                self.sel_cam = Some((SEL_EYE0, SEL_LOOK0, 0.0, 0.0, 0.0));
                // (after the film: Graphics_WarpToTimeOfDay(17, 100) - CarSelection, before City_DoneStreaming)
                self.intro_tod = Some(17.0);
            }
            // (after the film: `City_DoneStreaming(100)` polled every 10 ms after three frames, then the fade up
            // over 1 s - CarSelection 0xb68, 0xbaa - with VEHICLE SELECT already up)
            2 if self.game.movie.is_none() && self.cuts.idle() && self.intro_t > 3.0 / 60.0 && streaming == 0 => {
                log::info!("opening: VEHICLE SELECT");
                self.intro_mask.release(1.0);
                self.intro_stage = 3;
            }
            // ACCEPT: fade down, 1 s (CarSelection 0x13f5), held until the story scene's picture
            3 if !self.game.fresh_game => {
                self.intro_mask = Default::default();
                self.intro_mask.start(1.0, 0.01, f32::MAX);
                self.intro_stage = 4;
                self.intro_t = 0.0;
            }
            4 => {
                // (the black is up: the two not taken go, the car is the player's - CarSelection 0x1402 on)
                if self.intro_mask.full() {
                    self.game.starter_pick = None;
                }
                let scenes =std::env::var("MCLA_NO_CUTS").is_err() && std::env::var("MCLA_NO_CINE").is_err() && (self.opt.shot.is_none() || std::env::var("MCLA_CUTS").is_ok());
                let up = self.cuts.playing.is_some() && self.cine_hold.is_none();
                if self.intro_mask.full() && (up || !scenes) {
                    log::info!("opening: the first story scene comes up ({:.2} h, {} cars)", self.time_of_day, self.vehicles.cars.len());
                    self.intro_mask.release(1.0);
                    self.intro_stage = 0;
                }
            }
            _ => {}
        }
        // (the warp, F-64257 with +14552 set: the hour moves the short way round at the rate (+14548, 100 h a
        // second) x the frame's unscaled seconds (G+88), stops on the target and the warp is over; then mod 24)
        if let Some(to) = self.intro_tod.filter(|_| self.game.movie.is_none() && self.cuts.idle()) {
            let h = self.time_of_day;
            let (up, down) = if h < to { (to, to - 24.0) } else { (to + 24.0, to) };
            let step = 100.0 * dt;
            let (next, done) = if up - h < h - down { ((h + step).min(up), h + step >= up) } else { ((h - step).max(down), h - step <= down) };
            self.time_of_day = next.rem_euclid(24.0);
            if done {
                log::info!("opening: the clock at {:.2} h", self.time_of_day);
                self.intro_tod = None;
            }
        }
        // (the film, or FAST's arrival scene, plays over the held black)
        let over_black = self.game.movie.is_some() || (self.intro_stage == 2 && self.cuts.playing.is_some());
        self.game.intro_mask = if over_black { 0.0 } else { self.intro_mask.update(dt) };
        self.game.opening_black = self.intro_stage == 4 || (self.intro_stage == 3 && !self.game.fresh_game);
    }

    /// The cine scenes' lamp switches (vehicles.rs `CINE_LAMP_EVENTS`): at a scene's start every car's switch
    /// is kept (F-32153), the scene's events set the car of their slot as its clock passes them, and when
    /// it ends the kept switches come back (FinishScene, F-62501).
    fn cine_lamps_tick(&mut self) {
        let now = self.cine.playing.as_ref().map(|s| (s.name.clone(), s.t));
        if self.cine_lamps.as_ref().is_some_and(|(name, _, _)| now.as_ref().map_or(true, |(n, _)| n != name)) {
            let (ended, _, kept) = self.cine_lamps.take().unwrap();
            for (uid, off) in kept {
                if let Some(c) = self.vehicles.cars.iter_mut().find(|c| c.uid == uid) {
                    c.amb_lamps.off = off;
                }
            }
            // FinishScene (F-62501 with 1) leaves the engine word alone; the story scenes' callers run Racer_Activate
            // (F-76651(racer, 0) -> F-30391(0): +46 = 1) or Racer_Reset right after (payback_intro's three, race_gh_06,
            // bestof_sh_05, beat_sh_02_exotic, rep_on_strip_garage, hangout_series_hollywoodc, district_champs, the
            // pinkslip hills). find_sh_02c's core plays Racer_PlayIgnition and then Racer_Activate after its fade.
            // The downtown hangouts' and city_champ's go on into a race, whose core (OrderedCheckpointCore) runs
            // Racer_ResetWithDamageIntact / Racer_Activate at its set-up; hollywood_garage_intro goes into the garage,
            // whose core activates the car at its exit. (approx.: the engine comes back at the scene's end, not at
            // that later set-up / exit / fade, which the player does not drive through)
            if ended.starts_with("story/") || ended.starts_with("racestart/") {
                if let Some(c) = self.vehicles.player_mut() {
                    c.engine_off = false;
                    c.deactivated = false;
                }
                if ended == "story/cut_story_find_sh_02c" {
                    self.sound.play_ignition();
                }
            }
        }
        let Some((name, t)) = now else { return };
        if self.cine_lamps.is_none() {
            let kept = self.vehicles.cars.iter().map(|c| (c.uid, c.amb_lamps.off)).collect();
            self.cine_lamps = Some((name.clone(), -1.0, kept));
            self.cine_showdriver.clear();
            // EndDescription (F-39589): every UseRacerNativeCharacter slot's racer shows its driver (F-13669(racer, 1)),
            // and one that is damaged out is set to 0.95 of its MaxDamage (damage +36 / +40, the test F-98898 of
            // Racer_IsDamagedOut); UseRacerNativeCharacter itself (F-94066) raises slot 0 / 1's racer to the hi-res
            // driver (+178) unless it is the local human
            let slots = cinedriver::native_slots(&name);
            let mut shown = Vec::new();
            for n in (0..16u8).filter(|n| slots >> n & 1 != 0) {
                if let Some(i) = self.cine_slot_car(n) {
                    let c = &mut self.vehicles.cars[i];
                    c.show_driver = true;
                    if c.state.damage >= 1.0 {
                        c.state.damage = 0.95;
                    }
                    if n <= 1 && !c.is_player {
                        c.hi_driver = true;
                    }
                    shown.push(c.uid);
                }
            }
            log::info!("cine: {} - UseRacerNativeCharacter slots {:#x}, drivers shown (uid) {:?}; shown before it {:?}", name, slots, shown, self.vehicles.cars.iter().filter(|c| c.show_driver && !shown.contains(&c.uid)).map(|c| c.uid).collect::<Vec<_>>());
        }
            // (garage_core 0x1486, after the `_ext` scene and Racer_Activate: Racer_Reset on the player - F-89258 clears
            // his driver's scene flag, cinedriver.rs)
            if cinedriver::GARAGE_EXIT.contains(&ended.as_str()) {
                if let Some(c) = self.vehicles.player_mut() {
                    c.show_driver = false;
                }
            }
        let last = self.cine_lamps.as_ref().map_or(-1.0, |l| l.1);
        for &(scene, on, slot, at) in vehicles::CINE_LAMP_EVENTS {
            if scene == name && at > last && at <= t {
                if let Some(i) = self.cine_slot_car(slot) {
                    self.vehicles.cars[i].amb_lamps.off = !on;
                }
            }
        }
        for &(scene, ev, slot, at) in vehicles::CINE_ENGINE_EVENTS {
            if scene == name && at > last && at <= t {
                // the player's engine word (the port keeps it for the player only: its sound and exhaust); the arrest
                // scenes' car_off / engine_off do the same, kept until CopArrest's PlayIgnition
                if let Some(i) = self.cine_slot_car(slot).filter(|&i| self.vehicles.cars[i].is_player) {
                    // (the port's +592 bits other than 0x1: the crash bits and the grid hold 0x2)
                    let other_bits = self.crash_bits != 0 || self.vehicles.grid_hold.is_some();
                    let c = &mut self.vehicles.cars[i];
                    match ev {
                        vehicles::EngineEv::CarOff => c.engine_off = true,
                        vehicles::EngineEv::EngineOff => {
                            // F-30391(1): +592 |= 1; standing at exactly 1 the car stops - the speeds set to 0 (F-72737,
                            // the set-down with zero speeds), +46 = 0 - and is held (`deactivated`)
                            if !other_bits {
                                c.state.vel = Vec3::ZERO;
                                c.state.ang_vel = Vec3::ZERO;
                                c.engine_off = true;
                            }
                            c.deactivated = true;
                        }
                        vehicles::EngineEv::EngineOn => {
                            // F-30391(0): +592 = 0, +46 = 1
                            c.deactivated = false;
                            c.engine_off = false;
                        }
                    }
                }
            }
        }
        // showdriver / hidedriver (events 11 / 12, F-91263 -> F-13669 on the event's car slot)
        for &(scene, show, slot, at) in cinedriver::DRIVER_EVENTS {
            if scene == name && at > last && at <= t {
                if let Some(i) = self.cine_slot_car(slot) {
                    self.vehicles.cars[i].show_driver = show;
                }
            }
        }
        for &(scene, slot, at) in vehicles::CINE_CAR_ON {
            if scene == name && at > last && at <= t {
                // car_on's engine start (F-91984 -> vt+52 with 0); the port's engine sound is the player's only
                if self.cine_slot_car(slot).is_some_and(|i| self.vehicles.cars[i].is_player) {
                    self.sound.play_ignition();
                }
            }
        }
        if let Some(l) = self.cine_lamps.as_mut() {
            l.1 = t;
        }
    }

    /// The car of a cine scene's car slot (CineScript_Cars_AssignIdToSlot): a start scene's racers in their
    /// slots; otherwise slot 0 the player's car, slot k the car moved as `drv0k`.
    fn cine_slot_car(&self, slot: u8) -> Option<usize> {
        if let Some(&ci) = self.prerace_acts.as_ref().and_then(|a| a.racers.get(slot as usize)) {
            return Some(ci);
        }
        if slot == 0 {
            return self.vehicles.cars.iter().position(|c| c.is_player);
        }
                    let uid = self.vehicles.cars[i].uid;
                    self.cine_showdriver.retain(|&u| u != uid);
                    if show {
                        self.cine_showdriver.push(uid);
                    }
        let label = format!("cine:drv{:02}", slot);
        self.vehicles.cars.iter().position(|c| c.driver == label)
    }

    /// The car a cutscene's mover `label` is (car, colour, plate, set-up): "henry" -> the hookman called Henry in
    /// his car; "hookman" / "drv01" -> the rival of the current mission; the bus and the police bring their own models.
    fn cine_car_of(&mut self, label: &str) -> Option<(String, [f32; 4], String, String)> {
        let who = self.cine_hookman_of(label);
        let rival = who.and_then(|id| self.game.rivals.of(&id, 0));
        match (rival, label) {
            (Some(r), _) => Some((r.car, r.color, r.plate, r.config)),
            (None, "omnibus") => Some(("vp_citybus_07".to_string(), [0.85, 0.85, 0.85, 1.0], String::new(), String::new())),
            (None, "cop" | "cop1" | "cop2") => Some(("vp_chv_police_96".to_string(), [0.05, 0.05, 0.06, 1.0], String::new(), String::new())),
            _ => None,
        }
    }

    /// A new career's opening read ahead on worker threads, so that none of it waits for the disk on the main thread
        // (the scene's car record of the slot names its mover: slot 0 Player_GetRacer, slot k the racer the caller put
        // there - the port's car brought for that mover, cinedriver::SLOT_MOVERS)
        if let Some(label) = self.cine.playing.as_ref().and_then(|sc| cinedriver::slot_label(&sc.name, slot)) {
            if cine::is_player_car(label) {
                return self.vehicles.cars.iter().position(|c| c.is_player);
            }
            let tag = format!("cine:{label}");
            return self.vehicles.cars.iter().position(|c| c.driver == tag);
        }
    /// (docs/PERFORMANCE.md 2026-10-25): from the start screen the three starters (made under CarSelection's first
    /// black, `starters_made`: 1.7 s of reading in that frame before); from the opening's first black its story
    /// scene - CUT_INTRO in the film's place (INTRO FAST, DECISIONS.md D24) and CUT_STORY_RACE_GH_06 after VEHICLE
    /// SELECT - with its animation packs, its people and the cars it moves (henry's 280Z and his set-up's parts,
    /// `cine_spawn_cars`: 0.6 s at the scene's start before). What is made, and when, is unchanged.
    fn opening_read_ahead(&mut self, world: &world::World) {
        if self.opening_read & 1 == 0 && self.game.fresh_game && !self.starters_made {
            self.opening_read |= 1;
            for name in game::STARTERS {
                if let Some(d) = self.game.car_dirs.iter().find(|d| d.file_name().map_or(false, |n| n.to_string_lossy().eq_ignore_ascii_case(name))) {
                    self.vehicles.prefetch(world, d);
                }
            }
        }
        if self.opening_read & 2 == 0 && self.intro_stage >= 1 {
            self.opening_read |= 2;
            let mut scenes = vec!["story/cut_story_race_gh_06"];
            if !hooks::intro_film() && hooks::intro_scene() {
                scenes.insert(0, "story/cut_intro");
            }
            for scene in scenes {
                for label in self.cine.read_ahead_scene(scene) {
                    if cine::is_player_car(&label) {
                        continue;
    /// The hookman a cutscene's mover `label` is: "henry" -> the one called Henry; "hookman" / "drv01" -> the rival of
    /// the current mission.
    fn cine_hookman_of(&self, label: &str) -> Option<String> {
        match label {
            "hookman" | "hookmans" | "drv01" => self.game.career.rival_id(),
            l => self.game.rivals.characters.iter().map(|c| c.id.clone()).find(|id| self.game.career.name_of(id).eq_ignore_ascii_case(l)),
        }
    }

                    }
                    let Some((car, _, _, config)) = self.cine_car_of(&label) else { continue };
                    if !self.vehicles.models.iter().any(|m| m.name.eq_ignore_ascii_case(&car)) {
                        if let Some(d) = self.game.car_dirs.iter().find(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(&car))) {
                            self.vehicles.prefetch(world, d);
                        }
                    }
                    // (its set-up's rims, tyres and brakes into the file cache, for its dressing - `vehicles::warm_dress`)
                    if let Some(d) = self.game.mc4_dress_of(&car, &config) {
                        vehicles::warm_dress(&world.assets.root, &d);
                    }
                }
            }
        }
    }

    /// Cars a cutscene moves besides the player's: a named rival brings his own car.
    fn cine_spawn_cars(&mut self, gpu: &mut render::Gpu, world: &mut world::World) {
        for label in self.cine.movers() {
            if cine::is_player_car(&label) {
                continue;
            }
            let Some((pos, rot)) = self.cine.mover(&label) else { continue };
            if label.starts_with("cop") {
                // a cruiser that is already here plays the part
                let near = self.vehicles.cars.iter().enumerate().filter(|(_, c)| c.cop && c.driver.is_empty()).min_by(|a, b| a.1.state.pos.distance_squared(pos).partial_cmp(&b.1.state.pos.distance_squared(pos)).unwrap()).map(|(i, _)| i);
                if let Some(i) = near {
                    self.vehicles.cars[i].driver = format!("cine:{label}");
                    self.vehicles.cars[i].ai = None;
                    self.vehicles.pose_car(i, pos, rot);
                    continue;
                }
            }
            let Some((car, color, plate, config)) = self.cine_car_of(&label) else { continue };
            let dirs = self.game.car_dirs.clone();
            if !self.vehicles.models.iter().any(|m| m.name.eq_ignore_ascii_case(&car)) {
                if let Some(d) = dirs.iter().find(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(&car))) {
                    let _ = self.vehicles.load_model(gpu, world, d);
                }
            }
            let Some(model) = self.vehicles.models.iter().find(|m| m.name.eq_ignore_ascii_case(&car)).cloned() else { continue };
            let idx = self.vehicles.spawn(gpu, model, pos, 0.0, color, false);
            self.vehicles.cars[idx].driver = format!("cine:{label}");
            self.vehicles.cars[idx].plate_text = plate;
            self.vehicles.cars[idx].vinyl_name = config;
            self.vehicles.cars[idx].ai = None;
            self.vehicles.cars[idx].cop = car.contains("police");
            self.vehicles.pose_car(idx, pos, rot);
            // (the scene's car is its set-up's whole car - RACE_GH_06_READY_TO_START_ON_ENTER loads
            // GH_06_0_vp_dsn_280z_75 with OpponentManager_LoadOpponent and waits for it to stream before the
            // scene: parts, rims, paint now, not by the cruise's one-car-a-frame dressing, which a story scene's
            // stopped clock never reaches)
            let game = &self.game;
            while !self.vehicles.cars[idx].dressed && !self.vehicles.cars[idx].vinyl_name.is_empty() {
                if self.vehicles.dress_setups(gpu, world, &mut |car, cfg| game.mc4_dress_of(car, cfg)).is_none() {
                    break;
                }
            }
            log::info!("cine car: {} is {} in set-up {:?} (dressed {})", label, car, self.vehicles.cars[idx].vinyl_name, self.vehicles.cars[idx].dressed);
        }
    }
}

impl App {
    /// The player's car's tyre effects stopped at once, their particles gone (a reset or a repair of the car: F-83592).
    fn kill_player_tyre_fx(&mut self) {
        if let (Some(t), Some(ci)) = (self.tyre_fx.as_mut(), self.vehicles.cars.iter().position(|c| c.is_player)) {
            t.kill(&mut self.fx, ci);
            // (the hookman's racer carries his character set - mcCareer_GetHookmanCharacterSet, cinedriver::HOOKMAN_SETS)
            if let Some(set) = self.cine_hookman_of(&label).as_deref().and_then(cinedriver::hookman_set) {
                self.vehicles.cars[idx].driver_set = set.to_string();
            }
        }
    }

    fn start_at_once(&mut self, event_loop: &ActiveEventLoop) {
        // (everything at once before the first frame: OFF, headless runs and the test hooks - with the start-up
        // movie the loading runs behind it instead, bootload.rs)
        let window = self.open_window(event_loop);
        let t0 = Instant::now();
        let gpu = pollster::block_on(Gpu::new(window.clone())).unwrap_or_else(|e| crate::hooks::gpu_init_failed(&e));
        log::info!("startup: gpu at {} ms", t0.elapsed().as_millis());
        self.egui = Some(App::make_egui(&window, &gpu.device, &gpu.queue, gpu.config.format));
        if self.opt.shot.is_none() {
            self.audio = audio::Audio::new(&self.opt.data_dir);
        } else if self.opt.audio_dump.is_some() {
            self.audio = audio::Audio::new_offline(&self.opt.data_dir);
        }
        let root = self.audio.as_ref().map(|a| a.root().to_path_buf());
        self.load_game(gpu, root.as_deref(), None);
        if let Some(a) = self.audio.as_mut() {
            a.set_music_catalog(&self.game.career.strings);
        }
        self.window = Some(window);
        self.last = Instant::now();
    }

    /// The window (settings.toml's mode and size), logged; the data folder handed to the renderer.
    fn open_window(&mut self, event_loop: &ActiveEventLoop) -> Arc<Window> {
        let attrs = Window::default_attributes().with_title("Midnight Club: Los Angeles (Rust)").with_inner_size(winit::dpi::LogicalSize::new(1600.0, 900.0));
        // (settings.toml: display.window_mode and display.resolution - `hooks`)
        let attrs = hooks::window_attributes(event_loop, attrs);
        // (diagnostics, MCLA_WIN=1280x720: the inner size in pixels - shots at a capture's own size)
        let attrs = match std::env::var("MCLA_WIN").ok().and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?)))) {
            Some((w, h)) if w >= 320 && h >= 180 => attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h)),
            _ => attrs,
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        hooks::log_window(&window);
        let _ = render::DATA_DIR.set(self.opt.data_dir.clone());
        window
    }

    /// The UI's context, input state and renderer (drawn into the surface's plain view, with the scene's depth).
    fn make_egui(window: &Window, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Egui {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(ctx.clone(), egui::ViewportId::ROOT, window, Some(window.scale_factor() as f32), None, Some(2048));
        let plain = format.remove_srgb_suffix();
        let mut renderer = egui_wgpu::Renderer::new(device, plain, Some(wgpu::TextureFormat::Depth32Float), 1, false);
        // (the GPS map's icons and road overlay against the scene's depth: hudmap_gpu.rs)
        renderer.callback_resources.insert(hudmap_gpu::XrayRes::new(device, queue, plain, 1, true));
        let selfs = selfmovie::Shared::new(device);
        renderer.callback_resources.insert(selfmovie::SelfRes::new(device, selfs.clone(), plain, 1, true));
        // (the menus' pass: 4 samples, no depth attachment - uipass.rs)
        let mut renderer4 = egui_wgpu::Renderer::new(device, plain, None, uipass::SAMPLES, false);
        renderer4.callback_resources.insert(hudmap_gpu::XrayRes::new(device, queue, plain, uipass::SAMPLES, false));
        renderer4.callback_resources.insert(selfmovie::SelfRes::new(device, selfs.clone(), plain, uipass::SAMPLES, false));
        // (also the UI pass at ANTI-ALIASING 2X: the same callbacks as the others)
        let mut renderer2 = egui_wgpu::Renderer::new(device, plain, None, selfmovie::samples(), false);
        renderer2.callback_resources.insert(hudmap_gpu::XrayRes::new(device, queue, plain, selfmovie::samples(), false));
        renderer2.callback_resources.insert(selfmovie::SelfRes::new(device, selfs.clone(), plain, selfmovie::samples(), false));
        // (the 3D icons' and the minimap's pipelines - flashpic.rs, minimap_gpu.rs)
        flashpic::init(device, queue);
        minimap_gpu::init(device, queue);
        Egui { ctx, state, renderer, renderer4, ui4: None, ui2: None, renderer2, selfs }
    }

    /// Whether the start-up movie plays (OPTIONS > PERFORMANCE > INTRO = settings.toml game.intro; not in a headless
    /// run, unless MCLA_BOOT_INTRO=normal|fast asks for it - tests).
    fn boot_mode(&self) -> mcla_config::Intro {
        match std::env::var("MCLA_BOOT_INTRO").ok().as_deref() {
            Some(v) => mcla_config::Intro::parse(v).unwrap_or(mcla_config::Intro::Skip),
            None if self.opt.shot.is_some() => mcla_config::Intro::Skip,
            None => hooks::intro_mode(),
        }
    }

    /// The city seen from the air (the map screen), once its thread has made it; `wait`: until it has (behind the
    /// start-up movie, bootload.rs).
    fn take_citymap(&mut self, gpu: &mut Gpu, wait: bool) {
        let Some(rx) = self.citymap_rx.as_ref() else { return };
        let got = if wait { rx.recv().ok() } else { rx.try_recv().ok() };
        if wait || got.is_some() {
            self.citymap_rx = None;
        }
        if let Some(map) = got {
            if (map.w.max(map.h) as u32) <= gpu.max_tex {
                use wgpu::util::DeviceExt;
                let (verts, idx) = map.mesh();
                let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("citymap vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
                let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("citymap ib"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
                let tex = Arc::new(render::create_rgba(&gpu.device, &gpu.queue, map.w as u32, map.h as u32, &map.rgba(), "citymap"));
                let clear = gpu.clear_tex.clone();
                let bind = gpu.texture_bind_group(&tex, &clear, 0xC17A_0001, 0xC17A_0002);
                log::info!("city map: ready ({} x {} texels, {} vertices)", map.w, map.h, verts.len());
                // (under a close-up patch the coarse city is drawn a little lower)
                let low = gpu.new_object(&render::ObjectUniform { model: glam::Mat4::from_translation(Vec3::new(0.0, -30.0, 0.0)).to_cols_array_2d(), color: [1.0; 4], params: [0.0; 4], dents: [[0.0; 4]; 6], ..Default::default() });
                // (the coarse relief for the GPS map's x-ray, hudmap::Heights)
                if let Some(hm) = self.game.hm.as_mut() {
                    hm.heights = Some(hudmap::Heights { min_x: map.min_x, min_z: map.min_z, cell: citymap::RES * citymap::STEP as f32, gw: map.gw, gh: map.gh, h: std::sync::Arc::new(map.height.clone()), coarse: Default::default() });
                }
                self.citymap = Some(CityMapGpu { vb, ib, count: idx.len() as u32, bind, map, low, patch: None });
            } else {
                log::warn!("city map: {} texels is more than this GPU's textures hold", map.w.max(map.h));
            }
        }
    }

    /// Whether the loading runs behind the start-up movie (bootload.rs): the movie plays and nothing asked for the
    /// game at once (headless runs, `--auto`, `--race-select`, `--garage`; MCLA_BOOT_LOAD=0 loads before it as before).
    fn loads_behind_movie(&self) -> bool {
        self.opt.shot.is_none() && !self.opt.auto && !self.opt.race_select && !self.opt.garage && self.boot_mode() != mcla_config::Intro::Skip && std::env::var("MCLA_BOOT_LOAD").map_or(true, |v| v != "0")
    }

    /// Everything the game needs after the window and the renderer (`resumed`, or the loading thread behind the
    /// start-up movie - bootload.rs). `audio_root`: the converted audio's folder when there is sound. `ui`: behind
    /// the movie, the UI's context, so that the UI's art and movies and the traffic's lanes are made here too
    /// instead of in the first frames.
    fn load_game(&mut self, mut gpu: Gpu, audio_root: Option<&std::path::Path>, ui: Option<&egui::Context>) {
        let t0 = Instant::now();
        let lap = |what: &str| log::info!("startup: {} at {} ms", what, t0.elapsed().as_millis());
        let assets = Arc::new(Assets::new(&self.opt.data_dir));
        lap("egui");
        let mut world = World::new(assets).expect("world");
        world.index_globaltex();
        lap("world");
        // the garage's lights for the cars' mirror map (render/mod.rs `set_garage_glow`): kept, and handed
        // to the renderer while the garage's own light state is in force (the render function)
        if let Some(p) = world.globaltex.get(&mcla_formats::hash::joaat("garageEnvMap.dds")) {
            if let Ok(t) = std::fs::read(p).map_err(anyhow::Error::from).and_then(|d| mcla_formats::dds::parse_dds("garageEnvMap.dds", &d)) {
                let _ = GARAGE_GLOW.set(t.to_rgba8());
            }
        }
        // the scrape texture of damaged paint
        if let Some(p) = world.globaltex.get(&mcla_formats::hash::joaat("dmg_scrape.dds")) {
            if let Ok(t) = std::fs::read(p).map_err(anyhow::Error::from).and_then(|d| mcla_formats::dds::parse_dds("dmg_scrape.dds", &d)) {
                gpu.set_scrape(&t.to_rgba8());
            }
        }
        // the damage map's stamps: a dent's shape and a scrape's (damagemap.rs, 0x8288D010 / 0x8288D00C)
        {
            let shape = |name: &str| {
                let p = world.globaltex.get(&mcla_formats::hash::joaat(name))?;
                let t = std::fs::read(p).map_err(anyhow::Error::from).and_then(|d| mcla_formats::dds::parse_dds(name, &d)).ok()?;
                // (as the GPU's texture unit decodes them - xenos::decode_dxt5_gpu)
                let px = t.to_rgba8_gpu();
                Some(damagemap::Shape::from_rgba(t.width as usize, t.height as usize, &px))
            };
            match (shape("dmg_test_4.dds"), shape("dmg_test_3.dds")) {
                (Some(d), Some(s)) => damagemap::set_shapes(d, s),
                _ => log::warn!("damage map: dmg_test_4.dds / dmg_test_3.dds not found, no dents or scrapes drawn"),
            }
        }
        self.ability.load(&self.opt.data_dir);
        // the time warpers' tunes (`tune/vehicle/<name>.timewarper`)
        self.warp_wipe = boomcam::TimeWarper::load(&self.opt.data_dir, "wipeout", self.warp_wipe.tune);
        self.warp_expl = boomcam::TimeWarper::load(&self.opt.data_dir, "explosion", self.warp_expl.tune);
        self.warp_look = boomcam::TimeWarper::load(&self.opt.data_dir, "lookback", self.warp_look.tune);
        if std::env::var("MCLA_NO_CITYMAP").is_err() && std::env::var("MCLA_SIM_TEST").is_err() {
            self.citymap_rx = Some(citymap::spawn(world.assets.clone(), world.index.sectors.clone(), world.shared.clone()));
        }
        lap("damage shapes, abilities");
        self.spawn_player(&mut gpu, &mut world);
        if self.vehicles.player().is_none() {
            self.drive = false;
        }
        lap("player");
        // garage car list
        let root = world.assets.path("resources/vehicle");
        let mut dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.file_name().map_or(false, |n| n.to_string_lossy().starts_with("vp_") || n.to_string_lossy().starts_with("vpd_"))).collect())
            .unwrap_or_default();
        dirs.sort();
        // (the mod cars after the disc's, in their table order: MODS.md M-5, modcars.rs)
        crate::modcars::init(std::path::Path::new(&self.opt.data_dir));
        dirs.extend(crate::modcars::car_dirs());
        let cur = self.opt.car_dir.as_ref().and_then(|d| dirs.iter().position(|p| p.to_string_lossy().replace('\\', "/").ends_with(&d.replace('\\', "/").trim_end_matches('/').rsplit('/').next().unwrap_or("")))).unwrap_or(0);
        self.game = Game::new(dirs, cur);
        self.game.data_dir = self.opt.data_dir.clone();
        self.game.race_defs = races::load(&self.opt.data_dir);
        // (MCLA_RACE_LENGTHS: every race's length along its checkpoints, for picking one)
        if std::env::var("MCLA_RACE_LENGTHS").is_ok() {
            for r in &self.game.race_defs {
                let l: f32 = r.checkpoints.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>() * r.laps.max(1) as f32;
                log::info!("racelen {:.0} {} {} {} laps {} cps {} | {}", l, r.sub_type, r.race_type, r.name, r.laps, r.checkpoints.len(), self.game.career.strings.get(&r.id).map(|s| s.to_string()).unwrap_or_default());
            }
        }
        if let Some(name) = self.opt.race_name.as_ref() {
            self.game.current_race = self.game.race_defs.iter().position(|r| r.name.contains(name.as_str()));
            if self.game.current_race.is_none() {
                log::warn!("no race matching {:?}", name);
            }
        }
        let rims_root = world.assets.path("resources/rims");
        // Buyable rims are `whl_am_*` and, for the Complete Edition's downloadable
        // wheels, `whld_am_*` (the DLC_Rim.sc.xml tabs). The `*_stk_*` folders are
        // other cars' stock wheels, which are not for sale; entry 0 keeps a car's own.
        let mut rims: Vec<std::path::PathBuf> = std::fs::read_dir(&rims_root)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.file_name().map_or(false, |n| { let n = n.to_string_lossy(); n.starts_with("whl_") || n.starts_with("whld_") })).collect())
            .unwrap_or_default();
        rims.sort();
        rims.retain(|p| p.file_name().map_or(true, |n| !n.to_string_lossy().contains("_stk_")));
        rims.insert(0, std::path::PathBuf::new());
        self.game.rim_names = rims.iter().map(|p| p.file_name().map(|n| n.to_string_lossy().trim_start_matches("whld_").trim_start_matches("whl_").trim_start_matches("am_").replace('_', " ")).unwrap_or_else(|| "stock".to_string())).collect();
        self.game.rim_dirs = rims;
        // The rim shop names its rims by the resource folder under resources/rims
        // (`fs_text1`), which is not the order rim_dirs is in, so resolve by name.
        self.game.rim_shop = rimshop::load(&self.opt.data_dir);
        // (MCLA_RIM_BRAND=n, MCLA_RIM_PICK=n: open the rim shop at that brand and rim)
        if let Some(v) = std::env::var("MCLA_RIM_BRAND").ok().and_then(|v| v.parse::<usize>().ok()) {
            self.game.rim_brand = v;
        }
        if let Some(v) = std::env::var("MCLA_RIM_PICK").ok().and_then(|v| v.parse::<usize>().ok()) {
            self.game.rim_pick = v;
        }
        // key by both the folder's own name and the name the shop gives it, so the one
        // rim the disc spells wrongly still resolves
        let folders: std::collections::HashMap<String, usize> = self.game.rim_dirs.iter().enumerate().filter_map(|(i, p)| Some((p.file_name()?.to_string_lossy().to_string(), i))).collect();
        self.game.rim_by_name = folders.iter().filter_map(|(name, i)| Some((name.clone(), *i))).chain(self.game.rim_shop.brands.iter().flat_map(|b| &b.rims).filter_map(|r| folders.get(rimshop::folder_for(&r.resource)).map(|i| (r.resource.clone(), *i)))).collect();
        log::info!("rim shop: {} brands, {} rims", self.game.rim_shop.brands.len(), self.game.rim_shop.brands.iter().map(|b| b.rims.len()).sum::<usize>());
        let unmatched: Vec<&str> = self.game.rim_shop.brands.iter().flat_map(|b| b.rims.iter()).filter(|r| !self.game.rim_by_name.contains_key(&r.resource)).map(|r| r.resource.as_str()).collect();
        if !unmatched.is_empty() {
            log::warn!("rim shop: {} rims with no resources/rims folder: {:?}", unmatched.len(), &unmatched[..unmatched.len().min(8)]);
        }
        if let Some(v) = std::env::var("MCLA_RIM").ok().and_then(|v| v.parse::<u8>().ok()) {
            self.game.rim_inches = v;
        }
        self.game.catalog = catalog::Catalog::load(&self.opt.data_dir);
        self.game.rubber = rubber::load_all(&self.opt.data_dir);
        self.game.ensure_owned();
        self.game.career = career::Career::load(&self.opt.data_dir);
        self.game.rivals = rivals::Rivals::load(&self.opt.data_dir);
        // (a car the port adds as data joins the pick with its vehicle record here)
        // (the cut GT-R races as an opponent too: DECISIONS.md D21)
        crate::catalog::write_cut_setups(&self.opt.data_dir);
        // (then the opponent-marked mod cars, in name order: MODS.md design 7b, stage S4)
        self.game.oppick = oppick::OpPick::load(&self.opt.data_dir, &format!("{}{}{}", crate::catalog::CUT_CATALOG, crate::catalog::CUT_OPPONENT, crate::modcars::opponent_extra()));
        self.game.oppick.free_cars = crate::modcars::free_driver_cars();
        if std::env::var("MCLA_CAREER_TEST").is_ok() {
            self.game.career.selftest();
        }
        self.game.career.new_game();
        // (test runs - a screenshot, a frame limit, the self-driving and test hooks - never
        // write the player's save)
        self.game.no_save = self.opt.shot.is_some() || self.opt.auto || ["MCLA_ROAM_TEST", "MCLA_CAREER_TEST", "MCLA_SIM_TEST", "MCLA_WAGER_TEST"].iter().any(|v| std::env::var(v).is_ok());
        // (MCLA_SAVE_FILE: a test run on a save of its own, read whatever the run is)
        let test_save = std::env::var("MCLA_SAVE_FILE").is_ok();
        if self.opt.car_dir.is_none() || self.opt.shot.is_none() || test_save {
            self.game.load_save(&self.opt.data_dir);
            self.game.ensure_owned();
            self.game.sync_car_paint();
            self.game.sync_extra_parts();
            self.game.apply_imported_rim();
            if self.opt.shot.is_none() || test_save {
                let dir = self.game.car_dirs.get(self.game.current_car).cloned();
                if let Some(dir) = dir {
                    let color = self.game.paint_color();
                // (MCLA_COLOR=n: a palette swatch for screenshots)
                let color = std::env::var("MCLA_COLOR").ok().and_then(|v| v.parse::<usize>().ok()).map_or(color, |i| vehicles::PALETTE[i % vehicles::PALETTE.len()]);
                    let rim = self.game.rim_dirs.get(self.game.cfg.rim % self.game.rim_dirs.len().max(1)).cloned();
                    let cfg = self.game.cfg.clone();
                    self.vehicles.player_rim_inches = self.game.rim_inches;
                    if let Err(e) = self.vehicles.replace_player(&mut gpu, &mut world, &dir, color, &cfg, rim.as_deref()) {
                        log::warn!("restoring saved car: {:#}", e);
                    }
                }
            }
            let tuned = self.game.perf_frac(self.game.current_car);
            if let Some(p) = self.vehicles.player_mut() {
                p.set_perf(tuned);
            }
        }
        if let Ok(v) = std::env::var("MCLA_PERF") {
            // test helper: force a performance level 0..3
            let level: f32 = v.parse().unwrap_or(0.0);
            if let Some(p) = self.vehicles.player_mut() {
                p.set_perf(level / 3.0);
            }
        }
        lap("races, rims, catalog, career, save, cars");
        self.sound.amb_types = sound::load_amb_types(&self.opt.data_dir);
        if let Some(root) = audio_root {
            self.speech = speech::Speech::load(&self.opt.data_dir, root);
            lap("speech");
            self.radio = radio::Radio::load(&self.opt.data_dir, root);
            lap("police radio");
            self.sound.scripted_radio = self.radio.loaded();
        }
        self.cuts = cutscene::Cutscenes::load(&self.opt.data_dir, audio_root);
        self.cine = cine::Cine::load(&self.opt.data_dir);
        lap("cutscenes, cine");
        // (test hook: MCLA_CINE_DUMP=<part of a scene's name> logs where its car movers stand)
        if let Ok(part) = std::env::var("MCLA_CINE_DUMP") {
            for sc in self.cine.scenes("").into_iter().filter(|s| s.contains(&part.to_ascii_lowercase())) {
                for label in self.cine.mover_labels(&sc) {
                    for end in [false, true] {
                        let l = label.clone();
                        if let Some((p, r)) = self.cine.car_pose(&sc, &move |x| x == l, end) {
                            let f = r * Vec3::NEG_Z;
                            log::info!("cine dump: {} {} {} pos {:.2} {:.2} {:.2} fwd {:.2} {:.2}", sc, label, if end { "end" } else { "start" }, p.x, p.y, p.z, f.x, f.z);
                        }
                    }
                }
            }
        }
        // hangouts: where their pull-in scenes park the car
        for sc in self.cine.scenes("hangout/") {
            let Some(name) = sc.strip_suffix("_ent_01") else { continue };
            if !name.contains("cut_hangout_") {
                continue;
            }
            if let Some((p, _)) = self.cine.car_pose(&sc, &cine::is_player_car, true) {
                self.hangouts.push((name.to_string(), p));
            }
        }
        // (the delivery's place in Hollywood: where its scene `cut_delivery_get_keys` has the car
        // to be delivered; the recomp's panel came up eleven metres short of it, at 827.3, -836.5)
        // (test hook: MCLA_DELIVERY_SPOT=x,y,z puts it elsewhere)
        let spot = std::env::var("MCLA_DELIVERY_SPOT").ok().map(|v| v.split(',').filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<f32>>()).filter(|f| f.len() == 3).map_or(Vec3::new(837.25, 17.9, -841.48), |f| Vec3::new(f[0], f[1], f[2]));
        self.hangouts.push(("delivery".to_string(), spot));
        // (South Central's, from its own scene `sc_cut_delivery_get_keys`; not seen in the recomp)
        let sc_spot = Vec3::new(-38.14, -19.67, 1316.12);
        self.hangouts.push(("delivery_sc".to_string(), sc_spot));
        self.game.delivery_spots = vec![spot, sc_spot];
        self.game.hangouts = self.hangouts.iter().filter(|h| !h.0.starts_with("delivery")).map(|h| h.1).collect();
        // (their scenes: which set-up script stands for each - step 20b)
        self.game.hangout_names = self.hangouts.iter().filter(|h| !h.0.starts_with("delivery")).map(|h| h.0.clone()).collect();
        // the garages' doors: where their leaving scenes begin (only Hollywood's is set in the
        // world; the others' scenes are local to their sets - not marked yet)
        self.game.garage_doors = ["cut_la_hw_oh_garage_01x_ext", "cut_la_be_smb_garage_01x_ext", "sc_cut_garage_01_ext"]
            .iter()
            .filter_map(|s| self.cine.car_pose(s, &cine::is_player_car, false))
            .map(|p| p.0)
            .filter(|p| p.length() > 10.0 && p.y.abs() > 0.5)
            .collect();
        // (test hook: MCLA_DOOR=x,y,z puts one more marker there)
        if let Ok(v) = std::env::var("MCLA_DOOR") {
            let f: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
            if f.len() == 3 {
                self.game.garage_doors.push(Vec3::new(f[0], f[1], f[2]));
            }
        }
        log::info!("garage doors: {:?}", self.game.garage_doors);
        // (the wager hangouts are the scenes 04, 06, 08, 10 and South Central's second)
        self.game.hangout_wager = self.hangouts.iter().map(|h| ["_04", "_06", "_08", "_10", "sc_cut_hangout_02"].iter().any(|n| h.0.ends_with(n))).collect();
        self.redlights = redlight::RedLights::load(&self.cine, &self.opt.data_dir);
        self.roam.load_slots(&self.opt.data_dir);
        physics::collision::load_surfaces(&self.opt.data_dir);
        self.game.load_perf_prices(&self.opt.data_dir);
        self.game.samples = rewards::Samples::load(&self.opt.data_dir);
        self.game.vinyl_presets = vinyl::Presets::load(&self.opt.data_dir);
        self.game.vinyl_cats = vinyl::ShapeLib::categories(&self.opt.data_dir);
        self.game.data_dir = self.opt.data_dir.clone();
        {
            self.dispatch_zones = racescript::dispatch_zones(&self.opt.data_dir);
            log::info!("police radio: {} places with a dispatcher's report", self.dispatch_zones.len());
            let lines = racescript::whoosh_lines(&self.opt.data_dir);
            for (a, b) in &lines {
                let m = (*a + *b) * 0.5;
                self.whoosh.entry(((m.x / 128.0).floor() as i32, (m.z / 128.0).floor() as i32)).or_default().push((*a, *b));
            }
            log::info!("whoosh lines: {}", lines.len());
        }
        // collectables: the script fills an array of 60 positions (x, y, z, 3, index, 3, ...)
        if let Some(sco) = std::fs::read(std::path::Path::new(&self.opt.data_dir).join("script_obj/game/MiscTasks/Collectables.sco")).ok().and_then(|d| mcla_formats::sco::decode(&d).ok()) {
            use mcla_formats::sco::Tok;
            let t = sco.consts();
            let num = |i: usize| if let Some(Tok::Num(v)) = t.get(i) { Some(*v as f32) } else { None };
            for i in 0..t.len() {
                if let (Some(x), Some(y), Some(z), Some(a), Some(k), Some(b)) = (num(i), num(i + 1), num(i + 2), num(i + 3), num(i + 4), num(i + 5)) {
                    if a == 3.0 && b == 3.0 && k == self.barrels.len() as f32 && (x.abs() > 5.0 || z.abs() > 5.0) {
                        self.barrels.push(Vec3::new(x, y, z));
                    }
                }
            }
            log::info!("collectables: {} barrels", self.barrels.len());
        }
        self.game.redlights = self.redlights.spots.iter().map(|s| s.player.0).collect();
        log::info!("hangouts: {}", self.hangouts.len());
        // (test hook: MCLA_EVENT_DUMP=<part of a race id> logs where those races start)
        if let Ok(part) = std::env::var("MCLA_EVENT_DUMP") {
            for i in 0..self.game.race_defs.len() {
                if self.game.race_defs[i].id.to_ascii_lowercase().contains(&part.to_ascii_lowercase()) {
                    log::info!("event dump: {} ({}) starts at {:?}, grid yaw {:?}", self.game.race_defs[i].id, self.game.race_defs[i].sub_type, self.game.race_start(i), self.game.race_defs[i].grid.first().map(|g| g.1));
                }
            }
        }
        if std::env::var("MCLA_HANGOUT_DUMP").is_ok() {
            for h in &self.hangouts {
                log::info!("hangout: {:?}", h);
            }
            // (the entry lines against their scenes: where the car starts and stops, and whether that
            // move takes the line - `hangoutin::crossed`)
            for su in hangoutin::SETUPS.iter() {
                let a = self.cine.car_pose(su.scene, &cine::is_player_car, false).map(|p| p.0);
                let b = self.cine.car_pose(su.scene, &cine::is_player_car, true).map(|p| p.0);
                let x = a.zip(b).map(|(a, b)| hangoutin::crossed(su, a, b));
                log::info!("hangout entry line {}: scene {} car {:?} -> {:?}, crosses {:?}", su.key, su.scene, a, b, x);
            }
        }
        self.game.mode = if self.opt.shot.is_some() && !self.opt.menu { Mode::FreeRoam } else { Mode::MainMenu };
        // the start-up movie before the start screen (boot.rs; behind the movie bootload.rs hands over its own)
        if self.game.mode == Mode::MainMenu {
            self.game.boot = boot::Boot::new(self.boot_mode());
        }
        if self.opt.auto {
            self.game.mode = Mode::FreeRoam;
        }
        if self.opt.race_select {
            self.game.mode = Mode::RaceSelect;
        }
        if self.opt.garage {
            self.game.mode = Mode::Garage;
            self.game.prev_mode = Mode::MainMenu;
        }
        // sky textures: the original star field and moon
        {
            let load = |gpu: &mut Gpu, name: &str| -> Arc<render::GpuTexture> {
                let p = std::path::Path::new(&self.opt.data_dir).join("sky").join(name);
                match std::fs::read(&p).ok().and_then(|d| mcla_formats::dds::parse_dds(name, &d).ok()) {
                    Some(t) => gpu.upload_texture(mcla_formats::hash::joaat(&format!("sky/{}", name)), &t),
                    None => {
                        log::warn!("sky texture {} missing", name);
                        gpu.white.clone()
                    }
                }
            };
            let stars = load(&mut gpu, "starfield.dds");
            let moon = load(&mut gpu, "moon.dds");
            // the dome's high-detail noise map shapes the clouds
            let noise = load(&mut gpu, "noise16_p.dds");
            // the game's own dome programs (render/skyhat.rs) also read the galaxy and, for
            // their Perlin pass, the small noise texture
            // (the dome reads all of them as they are, without gamma decoding: separate copies)
            let raw = |gpu: &mut Gpu, name: &str| -> Arc<render::GpuTexture> {
                let p = std::path::Path::new(&self.opt.data_dir).join("sky").join(name);
                match std::fs::read(&p).ok().and_then(|d| mcla_formats::dds::parse_dds(name, &d).ok()) {
                    Some(t) => gpu.upload_texture_raw(mcla_formats::hash::joaat(&format!("sky/raw/{}", name)), &t),
                    None => gpu.white.clone(),
                }
            };
            if std::env::var("MCLA_OLD_SKY").is_err() {
                let (s, g, n, small) = (raw(&mut gpu, "starfield.dds"), raw(&mut gpu, "galaxy.dds"), raw(&mut gpu, "noise16_p.dds"), raw(&mut gpu, "perlinnoise_small.dds"));
                gpu.bind_sky_hat(&s, &g, &n, &small);
            }
            self.sky_bind = Some(gpu.texture_bind_group3(&stars, &moon, &noise, 0x5ca7_0001, 0x5ca7_0002, 0x5ca7_0003));
        }
        lap("scripts, hangouts, sky");
        if !self.opt.no_props {
            self.vehicles.load_parking(&self.opt.data_dir);
            self.police.load(&self.opt.data_dir);
            // garage interiors
            for (k, name) in ["cut_la_hw_oh_garage_set_items", "cut_la_be_smb_garage_set_items", "cut_sc_cs_junkyard_garage_set_items"].iter().enumerate() {
                let p = std::path::Path::new(&self.opt.data_dir).join("resources/cutsceneitems").join(format!("{name}.r102"));
                if p.exists() {
                    world.request_set(k, p);
                }
            }
            lap("parking, police");
            self.fx.load(&mut gpu, &self.opt.data_dir);
            self.tyre_fx = Some(tyrefx::TyreFx::load(std::path::Path::new(&self.opt.data_dir), &self.fx));
            rain::load(&mut gpu, &self.opt.data_dir);
            halo::load(&mut gpu, &self.opt.data_dir);
            siglrefl::load(&gpu, &self.opt.data_dir);
            sunglow::load(&mut gpu);
            pu_draw::load(&mut gpu, &self.opt.data_dir);
            pu_fx::load(&mut gpu, &self.opt.data_dir);
            sst_trail::load(&mut gpu, &self.opt.data_dir);
            carblob::load(&mut gpu);
            // (the scene's pipelines at the driving and the menus' sample counts, built ahead on a thread of their
            // own - render/ms.rs; the city's plain ones for the environment map too)
            render::ms::warm(vec![render::ms::scene(aamode::scene_samples(false)), render::ms::PLAIN, render::ms::scene(aamode::scene_samples(true))]);
            if std::env::var("MCLA_NO_PEDS").is_err() {
                self.peds.load(&self.opt.data_dir);
                self.drivers.load(&self.opt.data_dir);
            }
            lap("effects, peds, drivers");
            self.props = props::Props::load(&mut gpu, &mut world, &self.opt.data_dir);
            lap("props");
            // the city's placed lamps (the baked light list: its "range" is the light files'
            // Intensity) for the night lamp grid
            // (a record's cone and axis go into the table as they stand - the game's table writer
            // F-85100 reads +4C / +50 and the direction of every record; byte +59 zero = a
            // lamp that is off. MCLA_LAMP_NOCONE=1: all omni, as before)
            let nocone = std::env::var("MCLA_LAMP_NOCONE").is_ok();
            let mut lamps: Vec<render::Lamp> = self.props.lights.iter().map(|l| if nocone { render::Lamp { pos: l.pos, color: l.color.max(Vec3::ZERO), intensity: l.range.max(0.0), back: Vec3::Y, cone: [0.0, 1.0] } } else { render::Lamp { pos: l.pos, color: l.color.max(Vec3::ZERO), intensity: if l.on() { l.range.max(0.0) } else { 0.0 }, back: -l.dir, cone: l.table_cone() } }).collect();
            if std::env::var("MCLA_LAMP_DUMP").is_ok() {
                let mut n = std::collections::BTreeMap::new();
                for l in &self.props.lights {
                    *n.entry((l.kind, l.cone_sb[0] > 0.0, l.dir.y < 0.0, l.cone >= 359.0)).or_insert(0u32) += 1;
                }
                eprintln!("lampkinds (kind, cone, down, 360) {:?}", n);
                for l in self.props.lights.iter().filter(|l| l.kind >> 8 == 1).take(12) {
                    eprintln!("lampspot at {:.1} {:.1} {:.1} I {} cone {} pen {} +4c {} +50 {} dir {:.3} {:.3} {:.3}", l.pos.x, l.pos.y, l.pos.z, l.range, l.cone, l.penumbra, l.cone_sb[0], l.cone_sb[1], l.dir.x, l.dir.y, l.dir.z);
                }
            }
            // ... and the lamps of the street light props
            lamps.extend(self.props.prop_lamps.iter().map(|l| render::Lamp { pos: l.pos, color: l.color, intensity: l.intensity, back: l.back, cone: l.cone }));
            gpu.set_lamps(&lamps);
            // (diagnostics: MCLA_FIGLIGHT_PROBE = "x,y,z;x,y,z..." prints the walkers' four grid lanes
            // there, as a walker that stood long enough has them - compared with the recordings,
            // work/lighting2/figprobe.py)
            if let (Ok(v), Some(t)) = (std::env::var("MCLA_FIGLIGHT_PROBE"), render::LAMP_TABLES.read().ok().and_then(|t| t.clone())) {
                for q in v.split(';') {
                    let c: Vec<f32> = q.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if c.len() == 3 {
                        let p = Vec3::new(c[0], c[1], c[2]);
                        eprintln!("figlight {} {} {} lanes {:?}", c[0], c[1], c[2], figlight::FigureLight::new(&t, p).lanes(p, &figlight::Dynamic::default()));
                    }
                }
            }
            self.game.gas_stations = self.props.gas_stations.iter().map(|g| g.0).collect();
        }
        self.cp_markers = checkpoints::Markers::new(&self.opt.data_dir);
        lap("lamps, markers");
        if let Some(ctx) = ui {
            // (what the first frames would read: the UI's art and movies, the traffic's lane curves)
            self.game.load_ui(ctx);
            if let Some(nav) = world.nav.as_ref() {
                let _ = vehicles::traffic::rail_nav(nav);
            }
            let _ = occluder::city(&self.opt.data_dir);
            self.take_citymap(&mut gpu, true);
            lap("ui, traffic lanes, occluders, city map");
        }
        self.gpu = Some(gpu);
        self.world = Some(world);
        self.start_screen_ahead();
    }

    /// A race about to start (a roamer's challenge, a beat-me-there's race, the race chosen while none runs): its grid
    /// guessed as `handle_action`'s StartRace will draw it, on a copy of the career's generator and with the game's
    /// state put back, and the guessed cars read on worker threads (`Vehicles::prefetch`; unused ones age out) and their
    /// set-ups' files into the OS's file cache. The draw at the start is
    /// unchanged (it may differ from the guess: then only a read was wasted); a car read cold had stopped the start's
    /// frame for 746 ms, warm ~90 ms - docs/PERFORMANCE.md 2026-10-27 (6).
    fn grid_ahead(&mut self, world: &world::World) {
        let pending = self.roam.challenge.as_ref().map(|c| (c.race, true))
            .or(self.game.bmt_go)
            .or_else(|| self.game.current_race.filter(|_| self.game.race.is_none()).map(|r| (r, self.game.hookman_race)));
        if pending.is_none() || pending == self.grid_warm {
            return;
        }
        self.grid_warm = pending;
        let Some((race, hookman)) = pending else { return };
        let was = (self.game.current_race, self.game.hookman_race);
        (self.game.current_race, self.game.hookman_race) = (Some(race), hookman);
        let t = Instant::now();
        let guess = self.guess_grid();
        (self.game.current_race, self.game.hookman_race) = was;
        for (car, setup) in &guess {
            if let Some(d) = self.game.car_dirs.iter().find(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(car))).cloned() {
                if !self.vehicles.models.iter().any(|m| m.name.eq_ignore_ascii_case(car)) {
                    self.vehicles.prefetch(world, &d);
                }
            }
            if let Some(d) = self.game.mc4_dress_of(car, setup) {
                vehicles::warm_dress(&world.assets.root, &d);
            }
        }
        log::info!("grid ahead: race {} guessed {:?} ({} ms)", self.game.race_defs.get(race).map_or("?", |d| d.id.as_str()), guess.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), t.elapsed().as_millis());
    }

    /// The grid StartRace would draw now, as (car, set-up) - its composition in `handle_action` without its side
    /// effects: the named men by their ids only, the script's pick on a copy of the generator.
    fn guess_grid(&self) -> Vec<(String, String)> {
        let g = &self.game;
        let usable = |car: &str| g.car_dirs.iter().any(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(car)));
        let pink = g.mc4_pink_rival();
        let field = if pink.is_some() { 1 } else if g.series.is_some() { 3 } else { 7 };
        let mut out: Vec<(String, String)> = Vec::new();
        let mut used: Vec<String> = Vec::new();
        if let Some((id, car, cfg)) = pink.filter(|m| usable(&m.1)) {
            used.push(id);
            out.push((car, cfg));
        }
        let kept = g.series.as_ref().map(|s| s.rivals.clone()).unwrap_or_default();
        if !kept.is_empty() {
            return kept.into_iter().map(|r| (r.car, r.config)).collect();
        } else if let Some(id) = g.series.as_ref().and_then(|s| g.career.event_rival_char(&s.key)) {
            used.push(id);
        }
        let alt = g.current_race.and_then(|i| g.race_defs.get(i)).map_or(false, |d| d.id.ends_with("red_light_race") || d.id == althk::FREEWAY_RACE_ID);
        let man = if g.mc4_local_man_race() { g.opt_x.arcade.d.race_man.clone() } else if alt { g.opt_x.arcade.d.redlight_man.clone() } else { None };
        if let Some((id, car, cfg)) = man.filter(|m| usable(&m.1)) {
            used.push(id);
            out.push((car, cfg));
        }
        if g.hookman_race {
            used.extend(g.career.rival_id());
        }
        let tour_event = g.current_race.and_then(|i| g.race_defs.get(i)).and_then(|d| g.career.tournament_of(&d.id)).map(|t| t.0);
        if let (Some(ev), Some((fe, kept))) = (tour_event.as_ref(), self.tour_field.as_ref()) {
            if ev == fe {
                return kept.iter().map(|r| (r.car.clone(), r.config.clone())).collect();
            }
        }
        let p = std::env::var("MCLA_PLAYER_PERF").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| g.player_perf());
        let mut rng = *oppick::career_rng();
        let need = (field as usize).saturating_sub(used.len());
        out.extend(g.oppick.pick_with(&mut rng, need, p, g.grid_difficulty(), g.grid_masks(), &used, &usable).into_iter().map(|pk| (pk.car, pk.setup)));
        out
    }

    /// A start-screen shot's weather (titlecam.rs `Apply`): snapped.
    fn title_weather(&mut self, a: titlecam::Apply) {
        let w = if a.weather == 1 { sky::Weather::Cloudy } else { sky::Weather::Nice };
        if self.weather != w || self.sky_next.is_some() {
            self.sky = sky::Sky::load_weather(&self.opt.data_dir, w);
            self.weather = w;
            self.sky_next = None;
        }
    }

    /// The start screen's first shot drawn and the game's first tick run on the loading thread, behind LOADING: that tick
    /// holds the ambient manager's front-end frames (traffic.rs `FRONT_END_PREROLL`: 120 frames of the spawners, the cars
    /// and the walkers), which the original runs behind its loading logo too - at the hand-over it was one 250 ms tick
    /// (docs/PERFORMANCE.md 2026-10-27 (2)). The same calls as the first frame's, in the same order. MCLA_FE_AHEAD=0: not.
    fn start_screen_ahead(&mut self) {
        if self.game.mode != Mode::MainMenu || std::env::var("MCLA_FE_AHEAD").map_or(false, |v| v.trim() == "0") {
            return;
        }
        if let Some(a) = self.titlecam.frame(true, None, self.time_of_day) {
            self.title_weather(a);
        }
        if let Some(i) = self.titlecam.shot {
            self.time_of_day = titlecam::SHOTS[i].hour;
        }
        let t = Instant::now();
        self.update_gameplay(fixed_step::DT);
        log::info!("startup: the start screen's first tick run behind LOADING in {} ms", t.elapsed().as_millis());
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        self.start_at_once(event_loop);
    }


    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(eg), Some(w)) = (self.egui.as_mut(), self.window.as_ref()) {
            let r = eg.state.on_window_event(w, &event);
            if r.consumed {
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gpu.as_mut() {
                    g.resize(size.width, size.height);
                }
            }
            // (Alt held, for Alt+Enter - `hooks`)
            WindowEvent::ModifiersChanged(m) => hooks::set_alt(m.state().alt_key()),
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    // (Alt+Enter: the window mode, before anything else hears the Enter - `hooks`)
                    if self.alt_enter_key(code, event.state == ElementState::Pressed, event.repeat, is_synthetic) {
                        return;
                    }
                    // (a key pressed: the prompts show the keyboard's keys - `glyphs`)
                    if event.state == ElementState::Pressed && !is_synthetic && crate::glyphs::use_keyboard() {
                        log::info!("prompts: keyboard keys");
                    }
                    match event.state {
                        ElementState::Pressed if self.photo.is_some() => {
                            // (the camera keys only while SnapShot has the camera input; the
                            // album's states take Enter = A, Esc = B, Y, the arrows - `photoalbum`)
                            if self.game.album.camera_input() {
                                self.input.keys.insert(code);
                                self.input.pending_keys.insert(code);
                            }
                            match code {
                                KeyCode::Enter | KeyCode::NumpadEnter => self.game.nav_enter = true,
                                KeyCode::Escape => self.game.nav_back = true,
                                KeyCode::KeyY => self.game.nav_y = true,
                                KeyCode::KeyX => self.game.nav_x = true,
                                KeyCode::ArrowUp => self.game.nav_up = true,
                                KeyCode::ArrowDown => self.game.nav_down = true,
                                KeyCode::ArrowLeft => self.game.nav_left = true,
                                KeyCode::ArrowRight => self.game.nav_right = true,
                                KeyCode::F12 => self.screenshot_key(),
                                _ => {}
                            }
                        }
                        ElementState::Pressed => {
                            self.idle = 0.0;
                            if self.game.movie.as_ref().map_or(false, |m| m.any_key) {
                                // any key ends the attract movie
                                self.game.nav_enter = true;
                                return;
                            }
                            // (the keyboard page of OPTIONS, `keybinds`: it is handed the keys, and
                            // while it waits for one nothing else hears it)
                            if self.game.mode == Mode::Paused && self.game.pause_page == crate::mc4::PAGE_KEYBOARD {
                                let k = &mut self.game.opt_x.ctrl.kbd;
                                if !event.repeat && !is_synthetic {
                                    k.queue.push(code);
                                }
                                if k.polling {
                                    return;
                                }
                            }
                            self.input.keys.insert(code);
                            self.input.pending_keys.insert(code);
                            match code {
                                // (nothing on the start screen - the user's choice, DECISIONS.md;
                                // the window's close button quits)
                                KeyCode::Escape if self.game.mode == Mode::MainMenu => {}
                                KeyCode::Escape => self.game.nav_back = true,
                                KeyCode::KeyX if self.game.replay.is_some() => self.game.nav_x = true,
                                KeyCode::KeyY if self.game.replay.is_some() => self.game.nav_y = true,
                                // (Y skips the opening film and a story scene - `cine::skip_to`)
                                KeyCode::KeyY if self.cuts.playing.is_some() || self.game.movie.is_some() => self.game.nav_y = true,
                                KeyCode::Backspace if self.game.replay.is_some() => self.game.nav_select = true,
                                KeyCode::KeyP if self.game.replay.is_some() => self.game.nav_start = true,
                                // (the GPS map's MAP MODE is the pad's START; N on the keyboard)
                                KeyCode::KeyN if self.game.show_map && self.game.hm.is_some() => self.game.nav_start = true,
                                // (the arrows are the menus'; one bound to an action of the keyboard's
                                // table that is a press is that as well, outside a menu)
                                KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                                    match code {
                                        KeyCode::ArrowUp => self.game.nav_up = true,
                                        KeyCode::ArrowDown => self.game.nav_down = true,
                                        KeyCode::ArrowLeft => self.game.nav_left = true,
                                        _ => self.game.nav_right = true,
                                    }
                                    if !self.game.in_menu() && !self.game.show_map {
                                        self.key_action(code);
                                    }
                                }
                                KeyCode::Enter | KeyCode::NumpadEnter => self.game.nav_enter = true,
                                // (on the GPS map WASD are the left stick: the free cursor)
                                KeyCode::KeyW if self.game.in_menu() && !(self.game.show_map && self.game.hm.is_some()) => self.game.nav_up = true,
                                KeyCode::KeyS if self.game.in_menu() && !(self.game.show_map && self.game.hm.is_some()) => self.game.nav_down = true,
                                KeyCode::KeyA if self.game.in_menu() && !(self.game.show_map && self.game.hm.is_some()) => self.game.nav_left = true,
                                KeyCode::KeyD if self.game.in_menu() && !(self.game.show_map && self.game.hm.is_some()) => self.game.nav_right = true,
                                // LB / RB: the rim shop's wheel-brand tabs
                                KeyCode::KeyQ if self.game.in_menu() => self.game.nav_tab_prev = true,
                                KeyCode::KeyE if self.game.in_menu() => self.game.nav_tab_next = true,
                                // the shoulder buttons the rim shop steps brands with
                                KeyCode::KeyQ if self.game.in_menu() => self.game.nav_tab_prev = true,
                                KeyCode::KeyE if self.game.in_menu() => self.game.nav_tab_next = true,
                                KeyCode::F1 => self.show_ui = !self.show_ui,
                                KeyCode::F7 => self.cycle_weather = true,
                                // (the screenshot key: a PNG in the screenshots folder - `hooks`)
                                KeyCode::F12 => self.screenshot_key(),
                                // (the gameplay keys - music, headlights, map, garage, hydraulics, camera,
                                // abilities, GPS map, reset - are the keyboard's table: `key_action`, below)
                                KeyCode::PageUp | KeyCode::Equal | KeyCode::KeyZ if self.game.show_map => self.map_zoom_want = (self.map_zoom_want - 1.0).max(0.0),
                                KeyCode::PageDown | KeyCode::Minus | KeyCode::KeyX if self.game.show_map => self.map_zoom_want = (self.map_zoom_want + 1.0).min(3.0),
                                KeyCode::KeyE if self.game.show_map => self.game.nav_edit = true,
                                KeyCode::KeyR if self.game.show_map => self.game.nav_go = true,
                                KeyCode::Backspace if self.game.show_map => self.game.nav_del = true,
                                KeyCode::F2 => {
                                    if self.vehicles.player().is_some() {
                                        self.drive = !self.drive;
                                        if !self.drive {
                                            self.cam.pos = self.chase.pos;
                                        }
                                    }
                                }
                                // (every other key: the action the keyboard's table has for it)
                                _ => self.key_action(code),
                            }
                        }
                        ElementState::Released => {
                            self.input.keys.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } => {
                self.input.rmb = state == ElementState::Pressed;
                if let Some(w) = self.window.as_ref() {
                    w.set_cursor_visible(!self.input.rmb);
                }
            }
            WindowEvent::RedrawRequested => {
                self.render();
                self.after_frame();
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.input.mouse_dx += delta.0 as f32;
            self.input.mouse_dy += delta.1 as f32;
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            event_loop.exit();
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }
}

/// SDL's gamepad layer; none (and a line in the log) when it does not start.
fn open_pads() -> Option<pads::Pads> {
    pads::Pads::new().map_err(|e| log::warn!("pad: SDL's gamepad layer did not start: {e}")).ok()
}

/// The pad being played with (`App::pad_id`), while it is connected; none before a pad was used.
fn active_pad(g: &pads::Pads, id: Option<pads::PadId>) -> Option<&pads::Pad> {
    id.and_then(|id| g.get(id))
}

fn parse_args() -> Options {
    parse_options(&std::env::args().collect::<Vec<_>>())
}

fn parse_options(args: &[String]) -> Options {
    // (the data folder: --data, MCLA_DATA, settings, next to the exe - `hooks::init`, after the loop)
    let mut o = Options { traffic: 0, ..Default::default() };
    let mut data_flag: Option<String> = None;
    let mut shot_frames = 240;
    let _ = &o.menu;
    let mut i = 1;
    while i < args.len() {
        let next = |i: usize| args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--data" => {
                data_flag = Some(next(i));
                i += 1;
            }
            "--settings" => {
                o.settings_file = Some(next(i));
                i += 1;
            }
            "--save-dir" => {
                o.save_dir = Some(next(i));
                i += 1;
            }
            "--screenshot" => {
                o.shot = Some(ShotCfg { path: next(i), frames: 240 });
                i += 1;
            }
            "--frames" => {
                shot_frames = next(i).parse().unwrap_or(240);
                i += 1;
            }
            "--pos" => {
                let v: Vec<f32> = next(i).split(',').filter_map(|x| x.parse().ok()).collect();
                if v.len() == 3 {
                    o.pos = Some(Vec3::new(v[0], v[1], v[2]));
                }
                i += 1;
            }
            "--yaw" => {
                o.yaw = next(i).parse().ok();
                i += 1;
            }
            "--pitch" => {
                o.pitch = next(i).parse().ok();
                i += 1;
            }
            "--time" => {
                o.time = next(i).parse().ok();
                i += 1;
            }
            "--only" => {
                o.only = Some(next(i));
                i += 1;
            }
            "--hide" => {
                o.hide = Some(next(i));
                i += 1;
            }
            "--car" => {
                o.car_dir = Some(next(i));
                i += 1;
            }
            "--orbit" => {
                o.orbit = next(i).parse().unwrap_or(0.0);
                i += 1;
            }
            "--traffic" => {
                o.traffic = next(i).parse().unwrap_or(8);
                i += 1;
            }
            "--drive" => o.drive = true,
            "--fly" => o.fly = true,
            "--menu" => o.menu = true,
            "--race" => { o.race = true; o.drive = true; }
            "--fast" => o.fixed_dt = 1.0 / 60.0,
            "--no-props" => o.no_props = true,
            "--audio-dump" => {
                o.audio_dump = Some(next(i));
                i += 1;
            }
            "--no-post" => o.no_post = true,
            "--find-straight" => o.find_straight = true,
            "--map" => o.map = true,
            "--weather" => {
                o.weather = sky::Weather::parse(&next(i));
                i += 1;
            }
            "--aim-prop" => { o.aim_prop = Some(next(i)); i += 1; o.wot = true; o.drive = true; }
            "--no-shadows" => o.no_shadows = true,
            "--race-select" => { o.race_select = true; o.menu = true; }
            "--race-name" => {
                o.race = true;
                o.drive = true;
                o.race_name = Some(next(i));
                i += 1;
            }
            "--garage" => { o.garage = true; o.drive = true; }
            "--wot" => { o.wot = true; o.drive = true; }
            "--auto" => {
                o.auto = true;
                o.drive = true;
            }
            other => {
                if !other.starts_with("--") {
                    data_flag = Some(other.to_string());
                }
            }
        }
        i += 1;
    }
    if let Some(shot) = o.shot.as_mut() {
        shot.frames = shot_frames;
    }
    o.data_dir = hooks::init(data_flag.as_deref(), o.settings_file.as_deref(), o.shot.is_some()).data.path.to_string_lossy().into_owned();
    // (the anti-aliasing mode from the settings, for the renderer)
    hooks::apply_aa_at_start();
    if !o.fly {
        o.drive = true;
        if o.car_dir.is_none() {
            o.car_dir = Some(format!("{}/resources/vehicle/vp_vw_golf_83", o.data_dir));
        }
        if o.traffic == 0 && o.shot.is_none() {
            o.traffic = 8;
        }
    }
    o
}

#[cfg(test)]
mod option_tests {
    use super::*;

    fn options(args: &[&str]) -> Options {
        parse_options(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn screenshot_frame_count_is_independent_of_argument_order() {
        for args in [
            vec!["mcla", "--screenshot", "shot.png", "--frames", "1700", "--fast"],
            vec!["mcla", "--frames", "1700", "--fast", "--screenshot", "shot.png"],
        ] {
            let opt = options(&args);
            let shot = opt.shot.unwrap();
            assert_eq!(shot.path, "shot.png");
            assert_eq!(shot.frames, 1700);
            assert_eq!(opt.fixed_dt, 1.0 / 60.0);
        }
    }

    #[test]
    fn settings_flag_is_taken_in_any_position() {
        assert_eq!(options(&["mcla", "--settings", "s.toml", "--screenshot", "a.png"]).settings_file.as_deref(), Some("s.toml"));
        assert_eq!(options(&["mcla", "--screenshot", "a.png", "--settings", "t.toml"]).settings_file.as_deref(), Some("t.toml"));
        assert!(options(&["mcla", "--screenshot", "a.png"]).settings_file.is_none());
    }

    #[test]
    fn screenshot_default_and_last_explicit_frame_count() {
        assert_eq!(options(&["mcla", "--screenshot", "shot.png"]).shot.unwrap().frames, 240);
        assert_eq!(options(&["mcla", "--frames", "20", "--screenshot", "shot.png", "--frames", "30"]).shot.unwrap().frames, 30);
        assert!(options(&["mcla", "--frames", "20"]).shot.is_none());
    }

    #[test]
    fn a_short_key_press_survives_renders_without_a_simulation_tick() {
        let mut input = Input::default();
        input.keys.insert(KeyCode::Space);
        input.pending_keys.insert(KeyCode::Space);
        input.keys.remove(&KeyCode::Space);
        let mut clock = fixed_step::Clock::default();
        for _ in 0..3 {
            assert_eq!(clock.advance(1.0 / 240.0), 0);
            assert!(input.down(KeyCode::Space));
        }
        assert_eq!(clock.advance(1.0 / 240.0), 1);
        assert!(input.down(KeyCode::Space));
        input.pending_keys.clear();
        assert!(!input.down(KeyCode::Space));
        input.keys.insert(KeyCode::KeyW);
        input.pending_keys.clear();
        assert!(input.down(KeyCode::KeyW));
    }
}

/// What the police radio may say about the player: his street, the nearest cross street,
/// his heading and his car.
fn radio_ctx(vehicles: &vehicles::Vehicles, world: &world::World, hour: f32) -> radio::Ctx {
    let mut ctx = radio::Ctx { hour, ..Default::default() };
    let Some(p) = vehicles.player() else { return ctx };
    ctx.heading = p.state.forward();
    ctx.car = p.model.name.clone();
    if let Some(nav) = world.nav.as_ref() {
        let zone_name = |li: usize| nav.links[li].zone.and_then(|z| nav.zones.get(z)).map(|z| z.name.clone()).filter(|n| !n.is_empty());
        if let Some((li, _, d)) = nav.nearest_link(p.state.pos) {
            if d < 40.0 {
                ctx.street = zone_name(li).unwrap_or_default();
            }
        }
        // another street within a block
        let own = ctx.street.splitn(3, '_').nth(2).unwrap_or("").to_string();
        let mut best = (f32::INFINITY, String::new());
        for li in nav.links_near(p.state.pos, 90.0) {
            if let Some(n) = zone_name(li) {
                let d = nav.links[li].a.distance(p.state.pos);
                if n.splitn(3, '_').nth(2).unwrap_or("") != own && d < best.0 {
                    best = (d, n);
                }
            }
        }
        ctx.cross = best.1;
    }
    ctx
}

/// The city pass (`load`: its second part, which keeps what the first stored - docs/PERFORMANCE.md 2026-10-24).
fn begin_city_pass<'e>(enc: &'e mut wgpu::CommandEncoder, gpu: &Gpu, clear: [f32; 4], load: bool) -> wgpu::RenderPass<'e> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(if load { "city (rest)" } else { "city" }),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            // (always the HDR scene target - multisampled, resolved before the post chain: the tone map is
            // what writes the display)
            view: gpu.scene_color(),
            resolve_target: None,
            ops: wgpu::Operations {
                load: if load { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color { r: clear[0] as f64, g: clear[1] as f64, b: clear[2] as f64, a: 1.0 }) },
                store: wgpu::StoreOp::Store,
            },
        })],
        // (the stencil: 0 the world, 1 the sky and the ground dome, 4 k (+1) the k-th car - render/mb.rs)
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: gpu.scene_ds(),
            depth_ops: Some(wgpu::Operations { load: if load { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(0.0) }, store: wgpu::StoreOp::Store }),
            stencil_ops: Some(wgpu::Operations { load: if load { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(0) }, store: wgpu::StoreOp::Store }),
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}

/// What the city pass's sector loop reads (`draw_sectors`), shared with the worker that records the first part of
/// the pass (docs/PERFORMANCE.md 2026-10-24).
struct SectorView<'a> {
    gpu: &'a Gpu,
    occluded: &'a occluder::Selected,
    frustum: &'a Frustum,
    cam_pos: Vec3,
    lens: f32,
    only: Option<&'a str>,
    hide: Option<&'a str>,
}

/// The city's sector batches of one phase: 0 the opaque ones, 1 the blended ones.
/// `probe`: a probe frame's stamps (gpuprof.rs), one before each draw, labelled with its material and shader.
fn draw_sectors<'a>(pass: &mut wgpu::RenderPass<'_>, v: &SectorView<'a>, secs: impl Iterator<Item = &'a world::GpuSector>, phase: usize, pane_zw: bool, draws: &mut usize, mut probe: Option<&mut gpuprof::Probe>) {
    let gpu = v.gpu;
    let mut last_mode = u32::MAX;
    let mut pane_pipe = false;
    for sec in secs {
        let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
        if v.occluded.hides_all(sec.batches.iter().map(|b| (b.min, b.max))) {
            continue;
        }
        let mut bound = false;
        let mut last_bind: *const wgpu::BindGroup = std::ptr::null();
        // (the sector's own object uniforms: its cross-fade with the far model, and
        // which of its batches are interiors - world.rs `GpuSector::obj`)
        let mut last_obj = 0u8;
        for b in &sec.batches {
            let blend = matches!(b.mode, MatMode::Blend | MatMode::StoreGlass);
            if blend != (phase == 1) {
                continue;
            }
            if !v.frustum.intersects_aabb(b.min, b.max) {
                continue;
            }
            if let Some(o) = sec.obj.as_ref() {
                let want = if b.interior { 2 } else { 1 };
                if want != last_obj {
                    pass.set_bind_group(3, if b.interior { &o.1.bg } else { &o.0.bg }, &[]);
                    last_obj = want;
                }
            }
            // contribution culling: skip what covers less than a few pixels
            let ext = (b.max - b.min).length();
            let dist = ((b.min + b.max) * 0.5).distance(v.cam_pos);
            if ext < dist * 0.012 * v.lens {
                continue;
            }
            if let Some(o) = v.only {
                if !b.shader.contains(o) {
                    continue;
                }
            }
            if let Some(h) = v.hide {
                if b.shader.contains(h) {
                    continue;
                }
            }
            if !bound {
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                bound = true;
            }
            let bp = Arc::as_ptr(&b.bind);
            if bp != last_bind {
                pass.set_bind_group(1, b.bind.as_ref(), &[]);
                last_bind = bp;
            }
            if b.mode as u32 != last_mode {
                pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                last_mode = b.mode as u32;
            }
            if pane_zw {
                let want_pane = matches!(b.mode, MatMode::StoreGlass);
                if want_pane != pane_pipe {
                    pass.set_pipeline(if want_pane { &gpu.pipeline_pane } else { &gpu.pipeline_blend });
                    pane_pipe = want_pane;
                }
            }
            if let Some(p) = probe.as_deref_mut() {
                p.stamp(pass, || format!("{:?} {}", b.mode, b.shader));
            }
            pass.draw_indexed(b.first..b.first + b.count, 0, 0..1);
            *draws += 1;
        }
    }
    if pane_pipe {
        pass.set_pipeline(&gpu.pipeline_blend);
    }
}

/// game.dat's DYNAMIC_MIXER_LOOKBACK (the mixer's fixed slot 5, F-50037 / F-84487): asked for every
/// frame the look-back start passes (a refused start is tried again), stopped with its exit only
/// when it was asked for.
fn lookback_scene(asked: &mut bool, audio: Option<&mut audio::Audio>, on: bool) {
    if !on && !*asked {
        return;
    }
    *asked = on;
    if let Some(a) = audio {
        a.obj_scene("DYNAMIC_MIXER_LOOKBACK", on);
    }
}

/// The replay director's tests (replay::Probe, F-51014) on the cameras' level: a ball at a
/// point (a == b) touching the city or a car, else the ball swept a -> b, directed (F-43031),
/// the place where it first touches; the heli's look at the car: city only, both ways
/// (approx.: the original's undirected sweep F-80278 - the ball at both ends and the sweep in
/// both directions).
struct ReplayProbe<'a>(&'a physics::camprobe::CamLevel<'a>);

impl replay::Probe for ReplayProbe<'_> {
    fn sweep(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<Vec3> {
        if (b - a).length_squared() < 1e-12 {
            return (!self.0.contacts(a, r, true, 1).is_empty()).then_some(a);
        }
        self.0.sweep(a, b, r, true).map(|(t, _)| a + (b - a) * t)
    }
    fn heli(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<Vec3> {
        let l = self.0;
        if !l.contacts(a, r, false, 1).is_empty() || !l.contacts(b, r, false, 1).is_empty() {
            return Some(a);
        }
        l.sweep(a, b, r, false).or_else(|| l.sweep(b, a, r, false).map(|(t, n)| (1.0 - t, n))).map(|(t, _)| a + (b - a) * t)
    }
}

fn main() -> std::process::ExitCode {
    hooks::run(run_game)
}

fn run_game() -> Result<()> {
    // (logging, the panic hook and the exit code: `hooks::run`)
    let opt = parse_args();
    game::savedir::init(opt.save_dir.as_deref());
    // (the mods first: the sound library's thread merges their sound objects while the game loads - modfs.rs)
    crate::modcars::init(std::path::Path::new(&opt.data_dir));
    if std::env::var("MCLA_PART_AUDIT").is_ok() {
        return part_audit::run(&opt.data_dir);
    }
    // self-test: MCLA_SIM_TEST="<car>[;<upgrade share>]" drives the car on a flat plane and
    // compares it with the acceleration table of its tuning file
    // self-test: MCLA_SIM_ALL=1 runs every car against its calibration table (one line each)
    if std::env::var("MCLA_SIM_ALL").is_ok() {
        let assets = Assets::new(&opt.data_dir);
        let root = std::path::Path::new(&opt.data_dir).join("resources/vehicle");
        let mut names: Vec<String> = std::fs::read_dir(&root)?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with("vp")).collect();
        names.sort();
        let mut col = physics::WorldCollider::default();
        let mut tris = Vec::new();
        let (cell, half) = (16.0f32, 220i32);
        for ix in -half..half {
            for iz in -half..half {
                let (x0, z0) = (ix as f32 * cell, iz as f32 * cell);
                tris.push([Vec3::new(x0, 0.0, z0), Vec3::new(x0, 0.0, z0 + cell), Vec3::new(x0 + cell, 0.0, z0)]);
                tris.push([Vec3::new(x0 + cell, 0.0, z0), Vec3::new(x0, 0.0, z0 + cell), Vec3::new(x0 + cell, 0.0, z0 + cell)]);
            }
        }
        col.sectors.insert(0, physics::SectorMesh::build(tris));
        let dt = 1.0 / 60.0;
        println!("car                          top sim/table   t(0-75% top) sim/table   steer .15@22  .4@15   1.0@6");
        for name in names {
            let Ok(car) = vehicle_model::load_car(&assets, &root.join(&name)) else { continue };
            let (Some(st), Some(ai)) = (car.simtune.as_ref(), car.aitune.as_ref()) else { continue };
            let mut p = vehicles::car_params(&car);
            let mut vt = physics::vehsim::VehTune::new(st.base.clone());
            vt.bike = car.bike;
            p.veh = Some(Arc::new(vt));
            let ai = &ai.base;
            // acceleration
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            for _ in 0..90 {
                c.step(&p, physics::CarInput::default(), &col, dt);
            }
            let goal = ai.max_speed * 0.75;
            let (mut t, mut t_goal) = (0.0f32, None);
            let mut top = 0.0f32;
            for _ in 0..(60 * 70) {
                // (MCLA_SIM_AI: as a computer driver's car - no boost, no converter)
                c.step(&p, physics::CarInput { throttle: 1.0, ai: std::env::var("MCLA_SIM_AI").is_ok(), ..Default::default() }, &col, dt);
                t += dt;
                if t_goal.is_none() && c.fwd_speed() >= goal {
                    t_goal = Some(t);
                }
                // (the test plane ends: turn the car round where it stands)
                if c.pos.z.abs() > 3300.0 || c.pos.x.abs() > 3300.0 {
                    c.pos = Vec3::new(0.0, c.pos.y, 0.0);
                }
                top = top.max(c.fwd_speed());
            }
            // the table's time to the same speed (its pull less the air's and the shafts' drag)
            let vt = p.veh.clone().unwrap();
            let (mut v, mut tt) = (0.0f32, 0.0f32);
            while v < goal && tt < 200.0 {
                v += (ai.accel_at(v) - vt.resistance(v)).max(0.05) * 0.01;
                tt += 0.01;
            }
            // the top speed the table stops at
            let table_top = ai.accel.iter().rposition(|x| *x != 0.0).unwrap_or(127) as f32 * ai.max_speed / 127.0;
            let gain = |steer: f32, v: f32| -> f32 {
                if v > ai.max_speed {
                    return 0.0;
                }
                let want = steer / ai.steer_scalar_at(v).max(0.1);
                let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                c.vel = c.forward() * v;
                let (mut ks, mut n) = (0.0f32, 0.0f32);
                for i in 0..300 {
                    let err = v - c.fwd_speed();
                    c.step(&p, physics::CarInput { throttle: (0.5 + err).clamp(0.0, 1.0), brake: if err < -1.0 { 0.3 } else { 0.0 }, steer: if i > 30 { steer } else { 0.0 }, ai: true, ..Default::default() }, &col, dt);
                    if (120..240).contains(&i) {
                        ks += c.ang_vel.dot(c.up()).abs() / c.speed().max(0.5);
                        n += 1.0;
                    }
                }
                ks / n / want
            };
            println!("{:28} {:5.1}/{:5.1} {:4.2}   {:5.1}/{:5.1} {:4.2}        {:4.2}   {:4.2}   {:4.2}", name, top, table_top, top / table_top, t_goal.unwrap_or(99.0), tt, t_goal.unwrap_or(99.0) / tt.max(0.1), gain(0.15, 22.0), gain(0.4, 15.0), gain(1.0, 6.0));
        }
        return Ok(());
    }
    // self-test: MCLA_VEH_REPLAY runs a scripted drive and writes it out tick by tick (physics/replay.rs)
    if let Ok(v) = std::env::var("MCLA_VEH_REPLAY") {
        return physics::replay::run(&opt.data_dir, &v);
    }
    if let Ok(v) = std::env::var("MCLA_SIM_TEST") {
        let mut it = v.split(";");
        let car_name = it.next().unwrap_or("");
        let share: f32 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
        let assets = Assets::new(&opt.data_dir);
        let car = vehicle_model::load_car(&assets, &std::path::Path::new(&opt.data_dir).join("resources/vehicle").join(car_name))?;
        let mut p = vehicles::car_params(&car);
        let Some(st) = car.simtune.as_ref() else { anyhow::bail!("no simulation tuning") };
        let tune = mcla_formats::rage::simtune::SimTune::lerp(&st.base, &st.mods, share);
        println!("torque curve {:?} idle {:.0} opt {:.0} max {:.0} rpm, boost torque {:.0}, drag {:.2}..{:.2}", tune.engine.torque, tune.engine.s.idle_rpm, tune.engine.s.opt_rpm, tune.engine.s.max_rpm, tune.engine.s.boost_torque, tune.aero.drag_min, tune.aero.drag_max);
        println!("engine inertia {:.3}, drivetrain inertia {:.2}/{:.2}, gear change inertia? gcl {:.2}", tune.engine.s.inertia, tune.powertrain.inertia, tune.freetrain.inertia, tune.engine.s.gcl);
        let tune_front = (tune.wheel_front.steering_limit, tune.wheel_front.steering_offset);
        let mut vt = physics::vehsim::VehTune::new(tune);
        vt.bike = car.bike;
        let vt = Arc::new(vt);
        println!("ratios {:?} top {:.1} m/s gravity {:.2}", vt.ratios, vt.top_speed, vt.gravity);
        p.veh = Some(vt.clone());
        let mut col = physics::WorldCollider::default();
        // a flat plane of 16 m tiles: 800 m wide, 6.4 km ahead (forward is -z)
        let mut tris = Vec::new();
        for ix in -25..25 {
            for iz in -400..25 {
                let (x0, z0) = (ix as f32 * 16.0, iz as f32 * 16.0);
                let (x1, z1) = (x0 + 16.0, z0 + 16.0);
                tris.push([Vec3::new(x0, 0.0, z0), Vec3::new(x0, 0.0, z1), Vec3::new(x1, 0.0, z0)]);
                tris.push([Vec3::new(x1, 0.0, z0), Vec3::new(x0, 0.0, z1), Vec3::new(x1, 0.0, z1)]);
            }
        }
        col.sectors.insert(0, physics::SectorMesh::build(tris));
        if let Some(v) = p.veh.as_ref() {
            println!("shift points (up full, up light, down full, down light): {:?}", v.shift.iter().skip(1).map(|s| s.map(|x| x.round())).collect::<Vec<_>>());
        }
        println!("plane test: {:?}", col.raycast(Vec3::new(1.0, 2.0, 1.0), -Vec3::Y, 5.0).map(|h| (h.t, h.normal)));
        let dt = 1.0 / 60.0;
        // ---- full throttle from rest ----
        let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
        for _ in 0..120 {
            c.step(&p, physics::CarInput::default(), &col, dt);
        }
        println!("at rest: y {:.3} speed {:.2} compression {:?}", c.pos.y, c.speed(), c.wheels.iter().map(|w| (w.compression * 100.0).round() / 100.0).collect::<Vec<_>>());
        println!("wheels at rest: centre heights {:?}, drawn radius {:.3}, tune radius {:.3}/{:.3}, bone heights {:?}", c.wheels.iter().map(|w| ((w.world_pos.y) * 1000.0).round() / 1000.0).collect::<Vec<_>>(), p.wheel_radius, vt.t.wheel_front.radius, vt.t.wheel_back.radius, p.wheel_pos.map(|w| ((w.y - p.susp_rest) * 1000.0).round() / 1000.0));
        let (mut t, mut t60, mut t100) = (0.0f32, None, None);
        let mut last_v = 0.0;
        let table = car.tune.as_ref().map(|t| if share > 0.5 { t.engine_mods.clone() } else { t.engine.clone() });
        // (the calibration table: 128 samples over 0..top speed)
        let ai_accel = car.aitune.as_ref().map(|ai| mcla_formats::rage::aitune::AiTune::lerp(&ai.base, &ai.mods, share));
        // (the tables hold the pull alone: the air's and the shafts' drag come off it)
        let veh_tune = p.veh.clone();
        let accel_table = |v: f32| -> f32 { ai_accel.as_ref().map_or(0.0, |ai| ai.accel_at(v)) - veh_tune.as_ref().map_or(0.0, |vt| vt.resistance(v)) };
        for k in 0..(60 * 60) {
            c.step(&p, physics::CarInput { throttle: 1.0, ..Default::default() }, &col, dt);
            t += dt;
            let v = c.fwd_speed();
            if t60.is_none() && v >= 26.82 {
                t60 = Some(t);
            }
            if t100.is_none() && v >= 44.7 {
                t100 = Some(t);
            }
            if k % 30 == 29 && (t < 12.0 || k % 300 == 299) {
                let a = (v - last_v) / 0.5;
                let vs = c.veh.as_ref().unwrap();
                println!("t {:5.1} v {:6.2} m/s ({:5.1} mph) a {:6.2} (table {:5.2}) rpm {:5.0} gear {} slide {:.2}/{:.2} load {:.0}/{:.0} drift x {:.2}", t, v, v * 2.237, a, accel_table((v + last_v) * 0.5), c.rpm, c.gear, vs.wheels[0].slide, vs.wheels[2].slide, vs.wheels[0].load, vs.wheels[2].load, c.pos.x);
            }
            if k % 30 == 29 {
                last_v = v;
            }
        }
        {
            // the same from the table: its pull less the drags, from 2 m/s on
            let (mut v, mut tt, mut a60, mut a100) = (0.0f32, 0.0f32, None, None);
            while v < 45.0 && tt < 60.0 {
                v += accel_table(v.max(2.0)).max(0.05) * 0.005;
                tt += 0.005;
                if a60.is_none() && v >= 26.82 {
                    a60 = Some(tt);
                }
                if a100.is_none() && v >= 44.7 {
                    a100 = Some(tt);
                }
            }
            println!("table: 0-60 mph {:?} s, 0-100 mph {:?} s", a60, a100);
        }
        println!("0-60 mph {:?} s, 0-100 mph {:?} s, top speed {:.1} m/s (tune {:.1})", t60, t100, c.fwd_speed(), vt.top_speed);
        // ---- brake from there ----
        let v0 = c.fwd_speed();
        let (mut tb, x0) = (0.0f32, c.pos);
        while c.fwd_speed() > 0.5 && tb < 30.0 {
            c.step(&p, physics::CarInput { brake: 1.0, ..Default::default() }, &col, dt);
            tb += dt;
        }
        println!("braking from {:.1} m/s: {:.2} s, {:.1} m, mean {:.2} m/s^2 (table {:?})", v0, tb, (c.pos - x0).length(), v0 / tb.max(0.01), ai_accel.as_ref().map(|a| a.max_brake_decel));
        // ---- reverse from rest, then forward again ----
        {
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            for _ in 0..60 {
                c.step(&p, physics::CarInput::default(), &col, dt);
            }
            for _ in 0..180 {
                c.step(&p, physics::CarInput { brake: 1.0, ..Default::default() }, &col, dt);
            }
            let back = c.fwd_speed();
            for _ in 0..120 {
                c.step(&p, physics::CarInput { throttle: 1.0, ..Default::default() }, &col, dt);
            }
            println!("reverse: {:.1} m/s after 3 s (gear {}), then forward 2 s: {:.1} m/s", back, -1, c.fwd_speed());
            // coasting from 20 m/s
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            c.vel = c.forward() * 20.0;
            for _ in 0..180 {
                c.step(&p, physics::CarInput::default(), &col, dt);
            }
            println!("coasting from 20 m/s: {:.1} m/s after 3 s", c.fwd_speed());
            // handbrake from 20 m/s
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            c.vel = c.forward() * 20.0;
            let mut tt = 0.0;
            while c.speed() > 0.5 && tt < 20.0 {
                c.step(&p, physics::CarInput { handbrake: true, ..Default::default() }, &col, dt);
                tt += dt;
            }
            println!("handbrake stop from 20 m/s: {:.1} s", tt);
        }
        // ---- burnout: two seconds on the handbrake, then away ----
        {
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            for _ in 0..60 {
                c.step(&p, physics::CarInput::default(), &col, dt);
            }
            for _ in 0..120 {
                c.step(&p, physics::CarInput { throttle: 1.0, handbrake: true, steer: 0.0, ..Default::default() }, &col, dt);
            }
            let (held, charge) = (c.speed(), c.burnout);
            let slide = c.veh.as_ref().map(|v| (v.wheels[0].slide, v.wheels[2].slide, v.omega));
            for _ in 0..60 {
                c.step(&p, physics::CarInput { throttle: 1.0, ..Default::default() }, &col, dt);
            }
            println!("burnout: after 2 s on the handbrake speed {:.1} m/s, charge {:.2}, slide/omega {:?}; one second after letting go {:.1} m/s", held, charge, slide, c.fwd_speed());
            // nitrous from 20 m/s in the right gear
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            c.vel = c.forward() * 20.0;
            for _ in 0..30 {
                c.step(&p, physics::CarInput { throttle: 1.0, ..Default::default() }, &col, dt);
            }
            let v0 = c.fwd_speed();
            let mut c2 = c.clone();
            for _ in 0..150 {
                c.step(&p, physics::CarInput { throttle: 1.0, nitro: true, ..Default::default() }, &col, dt);
                c2.step(&p, physics::CarInput { throttle: 1.0, ..Default::default() }, &col, dt);
            }
            println!("nitrous: from {:.1} m/s, 2.5 s later {:.1} m/s with a shot, {:.1} without", v0, c.fwd_speed(), c2.fwd_speed());
        }
        // ---- stability: a flick of the wheel at speed, and a shove sideways ----
        for v0 in [25.0f32, 50.0] {
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            c.vel = c.forward() * v0;
            let mut peak = 0.0f32;
            for k in 0..360 {
                let steer = if (30..48).contains(&k) { 1.0 } else if (48..66).contains(&k) { -1.0 } else { 0.0 };
                c.step(&p, physics::CarInput { throttle: 0.8, steer, ..Default::default() }, &col, dt);
                peak = peak.max(c.ang_vel.dot(c.up()).abs());
            }
            let f = c.forward();
            println!("flick at {:.0} m/s: peak yaw rate {:.2}, 5 s later yaw rate {:.3} heading {:.1} deg slip {:.1} deg speed {:.1} upright {}", v0, peak, c.ang_vel.dot(c.up()), f.x.atan2(-f.z).to_degrees(), (c.vel.dot(c.right()) / c.speed().max(0.1)).asin().to_degrees(), c.speed(), c.up().y > 0.7);
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            c.vel = c.forward() * v0;
            for _ in 0..30 {
                c.step(&p, physics::CarInput { throttle: 0.8, ..Default::default() }, &col, dt);
            }
            c.ang_vel += c.up() * 1.5;
            for _ in 0..240 {
                c.step(&p, physics::CarInput { throttle: 0.8, ..Default::default() }, &col, dt);
            }
            println!("spun at {:.0} m/s with 1.5 rad/s: 4 s later yaw rate {:.3}, slip {:.1} deg, speed {:.1}", v0, c.ang_vel.dot(c.up()), (c.vel.dot(c.right()) / c.speed().max(0.1)).asin().to_degrees(), c.speed());
        }
        // ---- handbrake turn at 25 m/s: full lock and the handbrake for a second, then drive on ----
        {
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            c.vel = c.forward() * 25.0;
            for _ in 0..30 {
                c.step(&p, physics::CarInput { throttle: 0.8, ..Default::default() }, &col, dt);
            }
            let mut peak = 0.0f32;
            for _ in 0..60 {
                c.step(&p, physics::CarInput { steer: 1.0, handbrake: true, ..Default::default() }, &col, dt);
                peak = peak.max(c.ang_vel.dot(c.up()).abs());
            }
            let f = c.forward();
            let turned = f.x.atan2(-f.z).to_degrees();
            let v1 = c.speed();
            for _ in 0..120 {
                c.step(&p, physics::CarInput { throttle: 1.0, ..Default::default() }, &col, dt);
            }
            println!("handbrake turn: peak yaw rate {:.2} rad/s, turned {:.0} deg in 1 s (speed {:.1}), 2 s later yaw rate {:.2} speed {:.1} upright {}", peak, turned, v1, c.ang_vel.dot(c.up()), c.speed(), c.up().y > 0.7);
            // brake + handbrake spin from 22 m/s, and a J-turn: 3 s of reverse, then handbrake and lock
            {
                let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                for _ in 0..(60 * 6) {
                    c.step(&p, physics::CarInput { throttle: if c.fwd_speed() < 22.0 { 1.0 } else { 0.6 }, ..Default::default() }, &col, dt);
                }
                let v0 = c.speed();
                let mut peak = 0.0f32;
                for _ in 0..90 {
                    c.step(&p, physics::CarInput { steer: 1.0, brake: 1.0, handbrake: true, ..Default::default() }, &col, dt);
                    peak = peak.max(c.ang_vel.dot(c.up()).abs());
                }
                let f = c.forward();
                let spin = format!("spin from {:.0} m/s: peak yaw rate {:.2} rad/s, heading {:.0} deg after 1.5 s, speed {:.1}, upright {}", v0, peak, f.x.atan2(-f.z).to_degrees(), c.speed(), c.up().y > 0.9);
                let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                for _ in 0..(60 * 4) {
                    c.step(&p, physics::CarInput { brake: 1.0, ..Default::default() }, &col, dt);
                }
                let v0 = c.fwd_speed();
                let mut peak = 0.0f32;
                for _ in 0..90 {
                    c.step(&p, physics::CarInput { steer: 1.0, handbrake: true, ..Default::default() }, &col, dt);
                    peak = peak.max(c.ang_vel.dot(c.up()).abs());
                }
                let f = c.forward();
                println!("{spin}; J-turn from {:.1} m/s: peak yaw rate {:.2}, heading {:.0} deg after 1.5 s, upright {}", v0, peak, f.x.atan2(-f.z).to_degrees(), c.up().y > 0.9);
            }
            // top speed in a slipstream (no air drag for the cars): the table's MaxSpeed
            {
                let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                let mut top = 0.0f32;
                for _ in 0..(60 * 90) {
                    c.step(&p, physics::CarInput { throttle: 1.0, draft: true, ..Default::default() }, &col, dt);
                    if c.pos.length() > 3000.0 {
                        c.pos = Vec3::new(0.0, c.pos.y, 0.0);
                    }
                    top = top.max(c.fwd_speed());
                }
                println!("top speed in a slipstream {:.2} m/s; the table's MaxSpeed {:.2}", top, ai_accel.as_ref().map_or(0.0, |a| a.max_speed));
            }
            // kick-down: cruise on a light pedal, then floor it
            {
                let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                for _ in 0..(60 * 12) {
                    c.step(&p, physics::CarInput { throttle: if c.fwd_speed() < 26.0 { 0.8 } else { 0.6 }, ..Default::default() }, &col, dt);
                    if c.pos.length() > 3000.0 {
                        c.pos = Vec3::new(0.0, c.pos.y, 0.0);
                    }
                }
                let (g0, v0, r0) = (c.gear, c.fwd_speed(), c.rpm);
                c.step(&p, physics::CarInput { throttle: 1.0, ..Default::default() }, &col, dt);
                let g1 = c.gear;
                for _ in 0..90 {
                    c.step(&p, physics::CarInput { throttle: 1.0, ..Default::default() }, &col, dt);
                }
                println!("kick-down: cruising at {:.1} m/s in gear {} ({:.0} rpm); floored: gear {}, 1.5 s later gear {} at {:.1} m/s ({:.0} rpm)", v0, g0, r0, g1, c.gear, c.fwd_speed(), c.rpm);
            }
            // two wheels: up to speed, weight transfer with a little right lock for 5 s, then let go
            {
                let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                let mut line = String::from("two wheels (roll deg / speed):");
                let mut peak = 0.0f32;
                for f in 0..(60 * 14) {
                    let tt = f as f32 / 60.0;
                    let xfer = tt > 5.0 && tt < 10.0;
                    let inp = physics::CarInput { throttle: if c.fwd_speed() < 25.0 { 1.0 } else { 0.62 }, steer: if xfer && tt < 5.3 { 0.3 } else { 0.0 }, xfer, ..Default::default() };
                    c.step(&p, inp, &col, dt);
                    if c.pos.length() > 3000.0 {
                        c.pos = Vec3::new(0.0, c.pos.y, 0.0);
                    }
                    let roll = c.right().y.clamp(-1.0, 1.0).asin().to_degrees();
                    peak = peak.max(roll.abs());
                    if f % 60 == 59 && tt > 4.0 {
                        line += &format!(" {:.0}s {:.0}/{:.0}", tt, roll, c.speed());
                    }
                }
                println!("{line}; steepest {:.0} deg, upright at the end {}", peak, c.up().y > 0.9);
            }
        }
        if car.bike {
            // ---- stoppie: from 25 m/s, brake leaning forward for 1.5 s ----
            if car.bike {
                let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                c.vel = c.forward() * 25.0;
                let mut line = String::from("stoppie (pitch deg / speed / rear wheel down):");
                for k in 0..180 {
                    let on = (30..120).contains(&k);
                    c.step(&p, physics::CarInput { brake: if on { 0.6 } else { 0.0 }, lean: if on { -1.0 } else { 0.0 }, throttle: if on { 0.0 } else { 0.6 }, ..Default::default() }, &col, dt);
                    if k % 20 == 19 {
                        line += &format!(" {:.0}/{:.0}/{}", c.forward().y.clamp(-1.0, 1.0).asin().to_degrees(), c.speed(), (c.wheels[2].grounded || c.wheels[3].grounded) as u8);
                    }
                }
                println!("{line}; upright {}", c.up().y > 0.7);
            }
            // ---- wheelie: lean back on the throttle at 15 m/s for 3 s, then let go ----
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            c.vel = c.forward() * 15.0;
            for k in 0..360 {
                let lean = if (30..210).contains(&k) { 1.0 } else { 0.0 };
                c.step(&p, physics::CarInput { throttle: 1.0, lean, ..Default::default() }, &col, dt);
                if k % 30 == 29 {
                    let vs = c.veh.as_ref().unwrap();
                    println!("wheelie t {:.1}: pitch {:5.1} deg state {} front down {} speed {:.1} boost {:.1}", (k + 1) as f32 * dt, c.forward().y.asin().to_degrees(), vs.wheelie, c.wheels[0].grounded, c.fwd_speed(), vs.wheelie_boost);
                }
            }
        }
        // ---- the computer drivers' calibration against this simulation: for some bends, the
        // fastest speed at which the steering the table asks for still holds the bend ----
        if let Some(ai) = car.aitune.as_ref() {
            let ai = mcla_formats::rage::aitune::AiTune::lerp(&ai.base, &ai.mods, share);
            println!("ai tune: max speed {:.1}, curvature up to {:.3}, total accel {:.1}, brake {:.1}", ai.max_speed, ai.max_curvature, ai.max_total_accel, ai.max_brake_decel);
            println!("steer table: {}", ai.steer_scalar.iter().map(|a| format!("{a:.3}")).collect::<Vec<_>>().join(" "));
            println!("decel table ends {:?}, accel ends {:?}", ai.decel.iter().map(|d| d.iter().rposition(|x| *x != 0.0)).collect::<Vec<_>>(), ai.accel.iter().rposition(|x| *x != 0.0));
            println!("idle speeds {:?}", ai.idle_speed);
            println!("accel table: {}", ai.accel.iter().map(|a| format!("{a:.2}")).collect::<Vec<_>>().join(" "));
            println!("wheels at {:?}, steering limit {:.3} offset {:.3}", p.wheel_pos.map(|w| ((w.x * 100.0).round() / 100.0, (w.z * 100.0).round() / 100.0)), tune_front.0, tune_front.1);
            {
                let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                c.vel = c.forward() * 3.0;
                for i in 0..300 {
                    let err = 3.0 - c.fwd_speed();
                    c.step(&p, physics::CarInput { throttle: (0.5 + err).clamp(0.0, 1.0), steer: 0.15, ai: true, ..Default::default() }, &col, dt);
                    if i == 299 {
                        let vs = c.veh.as_ref().unwrap();
                        println!("3 m/s steer 0.15: wheel angles {:?} vs.steer {:.3} yaw {:.4} speed {:.2} slips {:?}", c.wheels.map(|w| (w.steer * 1000.0).round() / 1000.0), vs.steer, c.ang_vel.dot(c.up()), c.speed(), vs.wheels.map(|w| (w.slip_lat * 1000.0).round() / 1000.0));
                    }
                }
            }
            // hydraulics: fitted, switched on, a tap on the front pair, then held up, then off
            {
                use physics::car::{HYD_FITTED, HYD_ON};
                let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                let mut line = String::from("hydraulics (body height cm / pitch deg):");
                let mut peak = 0.0f32;
                for f in 0..(60 * 14) {
                    let tt = f as f32 / 60.0;
                    let hyd = match tt {
                        t if t < 1.0 => 0,
                        t if t < 2.0 => HYD_FITTED,
                        t if t < 4.0 => HYD_FITTED | HYD_ON,
                        t if t < 4.15 => HYD_FITTED | HYD_ON | 0b0011,
                        t if t < 6.0 => HYD_FITTED | HYD_ON,
                        t if t < 6.15 => HYD_FITTED | HYD_ON | 0b0011,
                        t if t < 8.0 => HYD_FITTED | HYD_ON,
                        t if t < 9.5 => HYD_FITTED | HYD_ON | 0b1100,
                        t if t < 11.0 => HYD_FITTED | HYD_ON,
                        _ => HYD_FITTED,
                    };
                    c.step(&p, physics::CarInput { hyd, ..Default::default() }, &col, dt);
                    peak = peak.max(c.pos.y);
                    if std::env::var("MCLA_HYD_TRACE").is_ok() && f % 6 == 0 && tt > 5.9 && tt < 9.0 {
                        let v = c.veh.as_ref().unwrap();
                        println!("  t {:.1} y {:.3} vy {:.2} rams {:?} order {:?} loads {:?} comp {:?} grounded {:?}", tt, c.pos.y, c.vel.y, v.hyd.height.map(|h| (h * 100.0).round()), v.hyd.order, v.wheels.map(|w| w.load.round()), v.wheels.map(|w| (w.comp * 100.0).round()), c.wheels.map(|w| w.grounded as u8));
                    }
                    if f % 60 == 59 {
                        let pitch = c.forward().y.asin().to_degrees();
                        line += &format!(" {:.0}s {:.1}/{:.1}", tt, c.pos.y * 100.0, pitch);
                    }
                }
                println!("{line}; highest {:.1} cm, upright {}", peak * 100.0, c.up().y > 0.9);
            }
            // creeping: held in each gear with the pedals up, the speed the car settles at
            {
                let mut line = String::from("creep speed (here / table):");
                for g in 1..=6 {
                    if ai.idle_speed[g - 1] <= 0.0 {
                        continue;
                    }
                    physics::vehsim::FORCE_GEAR.store(g as i32, std::sync::atomic::Ordering::Relaxed);
                    for from in [0.0f32, 12.0] {
                        let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                        c.vel = c.forward() * from;
                        for _ in 0..(60 * 40) {
                            c.step(&p, physics::CarInput::default(), &col, dt);
                            if c.pos.length() > 300.0 {
                                c.pos = Vec3::new(0.0, c.pos.y, 0.0);
                            }
                        }
                        line += &format!(" {:.2}", c.fwd_speed());
                    }
                    line += &format!(" / {:.2} |", ai.idle_speed[g - 1]);
                }
                physics::vehsim::FORCE_GEAR.store(0, std::sync::atomic::Ordering::Relaxed);
                println!("{line}");
            }
            // engine braking: lift at several speeds and compare with the table's row for the gear
            {
                let mut line = String::from("lift-off deceleration (here / table for that gear):");
                for v0 in [10.0f32, 18.0, 26.0, 36.0, 48.0] {
                    if v0 > ai.max_speed * 0.95 {
                        continue;
                    }
                    let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                    c.vel = c.forward() * v0;
                    for _ in 0..30 {
                        c.step(&p, physics::CarInput { throttle: 1.0, ..Default::default() }, &col, dt);
                    }
                    for _ in 0..12 {
                        c.step(&p, physics::CarInput::default(), &col, dt);
                    }
                    let v1 = c.fwd_speed();
                    for _ in 0..30 {
                        c.step(&p, physics::CarInput::default(), &col, dt);
                    }
                    let v2 = c.fwd_speed();
                    let g = c.gear.clamp(1, 6) as usize;
                    let vm = 0.5 * (v1 + v2);
                    let idx = vm * 127.0 / ai.max_speed;
                    let row = &ai.decel[g.min(ai.decel.len() - 1)];
                    let tab = row[(idx as usize).min(127)];
                    line += &format!("  {:.0} m/s gear {}: {:.2} / {:.2}", vm, c.gear, (v1 - v2) / 0.5, tab);
                }
                println!("{line}");
            }
            // steering gain: the bend a given steering input makes at each speed, against the table
            for steer in [0.15f32, 0.4, 1.0] {
                let mut line = format!("steer {:.2}: bend made / bend the table expects at", steer);
                for v in [3.0f32, 6.0, 10.0, 15.0, 22.0, 30.0, 40.0, 50.0] {
                    if v > ai.max_speed {
                        continue;
                    }
                    let want = steer / ai.steer_scalar_at(v).max(0.1);
                    let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                    c.vel = c.forward() * v;
                    let (mut ks, mut n) = (0.0f32, 0.0f32);
                    for i in 0..360 {
                        let err = v - c.fwd_speed();
                        c.step(&p, physics::CarInput { throttle: (0.5 + err).clamp(0.0, 1.0), brake: if err < -1.0 { 0.3 } else { 0.0 }, steer: if i > 30 { steer } else { 0.0 }, ai: true, ..Default::default() }, &col, dt);
                        if (120..240).contains(&i) {
                            ks += c.ang_vel.dot(c.up()).abs() / c.speed().max(0.5);
                            n += 1.0;
                        }
                    }
                    line += &format!("  {:.0} m/s {:.2} (lat {:.0}/{:.0})", v, ks / n / want, ks / n * v * v, want * v * v);
                }
                println!("{line}");
            }
            for radius in [300.0f32, 150.0, 75.0, 40.0, 20.0, 10.0] {
                let k = 1.0 / radius;
                let table_v = ai.speed_for_curvature(k).min(ai.max_speed);
                let run = |v: f32| -> (f32, f32) {
                    let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                    c.vel = c.forward() * v;
                    let steer = (k * ai.steer_scalar_at(v)).min(1.0);
                    let (mut ks, mut vs, mut n) = (0.0f32, 0.0f32, 0.0f32);
                    for i in 0..480 {
                        let err = v - c.fwd_speed();
                        let thr = (0.5 + err * 1.0).clamp(0.0, 1.0);
                        let brake = if err < -1.5 { 0.3 } else { 0.0 };
                        c.step(&p, physics::CarInput { throttle: thr, brake, steer: if i > 30 { steer } else { 0.0 }, ai: true, ..Default::default() }, &col, dt);
                        if i >= 300 {
                            ks += c.ang_vel.dot(c.up()).abs() / c.speed().max(1.0);
                            vs += c.speed();
                            n += 1.0;
                        }
                    }
                    (ks / n, vs / n)
                };
                // the hardest the car turns on the way in: peak lateral acceleration, and the tightest
                // radius it reaches before the speed is scrubbed off
                {
                    let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                    c.vel = c.forward() * table_v;
                    let steer = (k * ai.steer_scalar_at(table_v)).min(1.0);
                    let (mut peak, mut tight, mut at_v) = (0.0f32, f32::INFINITY, 0.0f32);
                    for i in 0..300 {
                        c.step(&p, physics::CarInput { throttle: 1.0, steer: if i > 20 { steer } else { 0.0 }, ai: true, ..Default::default() }, &col, dt);
                        let yaw = c.ang_vel.dot(c.up()).abs();
                        let lat = yaw * c.speed();
                        if lat > peak {
                            peak = lat;
                            at_v = c.speed();
                        }
                        if yaw > 0.01 && i > 60 {
                            tight = tight.min(c.speed() / yaw);
                        }
                    }
                    println!("   on the way in: peak lateral {:.1} m/s^2 at {:.1} m/s (table's bend needs {:.1}); tightest radius {:.1} m", peak, at_v, table_v * table_v * k, tight);
                }
                let (k_at, v_at) = run(table_v);
                if std::env::var("MCLA_SIM_BEND").is_ok() {
                    // what the tyres are doing in that bend
                    let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
                    c.vel = c.forward() * table_v;
                    let steer = (k * ai.steer_scalar_at(table_v)).min(1.0);
                    for i in 0..480 {
                        let err = table_v - c.fwd_speed();
                        c.step(&p, physics::CarInput { throttle: (0.5 + err).clamp(0.0, 1.0), steer: if i > 30 { steer } else { 0.0 }, ai: true, ..Default::default() }, &col, dt);
                        if i % 120 == 119 {
                            let vs = c.veh.as_ref().unwrap();
                            let w = |i: usize| format!("[slip {:.2}/{:.2} load {:.0} f {:.0}/{:.0}]", vs.wheels[i].slip_lat, vs.wheels[i].slip_long, vs.wheels[i].load, vs.wheels[i].f_lat, vs.wheels[i].f_long);
                            println!("   t {:.0}s v {:.1} steer in {:.2} wheel {:.3} rad yaw {:.2} side slip {:.1} deg turn force {:.0} gear {} rpm {:.0}  FL {} FR {} RL {} RR {}", (i + 1) as f32 / 60.0, c.speed(), steer, c.steer, c.ang_vel.dot(c.up()), (c.vel.dot(c.right()) / c.fwd_speed().max(1.0)).atan().to_degrees(), vs.turn_force, c.gear, c.rpm, w(0), w(1), w(2), w(3));
                        }
                    }
                }
                // the fastest speed that still turns at 90 % of the bend
                let mut best = 0.0f32;
                let mut v = table_v * 0.5;
                while v <= table_v * 1.5 && v <= ai.max_speed * 1.1 {
                    let (kk, vv) = run(v);
                    if kk >= k * 0.9 && vv >= v * 0.95 {
                        best = v;
                    }
                    v += table_v * 0.05;
                }
                println!("bend r {:5.0} m: table {:5.1} m/s ({:4.1} m/s^2); here at that speed: radius {:6.1} m, speed {:5.1}; fastest holding it {:5.1} m/s ({:.2}x)", radius, table_v, table_v * table_v * k, 1.0 / k_at.max(1e-4), v_at, best, best / table_v.max(0.1));
            }
        }
        // ---- cornering: hold a speed and full lock ----
        for target in [15.0f32, 30.0, 45.0] {
            let mut c = physics::CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            c.vel = c.forward() * target;
            let mut lat_max = 0.0f32;
            let mut yaw_rate = 0.0;
            for k in 0..600 {
                let thr = if c.fwd_speed() < target { 1.0 } else { 0.55 };
                let steer = if k > 60 { 1.0 } else { 0.0 };
                c.step(&p, physics::CarInput { throttle: thr, steer, ..Default::default() }, &col, dt);
                if k > 200 {
                    let w = c.ang_vel.dot(c.up());
                    yaw_rate = w;
                    lat_max = lat_max.max((w * c.speed()).abs());
                }
            }
            println!("cornering at {:.0} m/s: speed {:.1}, yaw rate {:.2} rad/s, lateral {:.1} m/s^2 ({:.2} g), roll {:.1} deg, upright {}", target, c.speed(), yaw_rate, lat_max, lat_max / 9.81, c.up().dot(Vec3::Y).acos().to_degrees(), c.up().y > 0.5);
            if let Some(vs) = c.veh.as_ref() {
                if car.bike {
                    println!("   lean {:.1} deg", vs.lean.to_degrees());
                }
            }
        }
        return Ok(());
    }
    // self-test: MCLA_VINYL_COMPOSE="<car>;<set-up or preset>;<out.png>" paints a livery into the unwrap
    if let Ok(v) = std::env::var("MCLA_VINYL_COMPOSE") {
        let p: Vec<&str> = v.split(";").collect();
        if p.len() == 3 {
            let root = std::path::Path::new(&opt.data_dir);
            let zones = vinyl::CarZones::load(&root.join("resources/vehicle").join(p[0]));
            let mut lib = vinyl::ShapeLib::load(&opt.data_dir);
            let livery = match render::car_lamps::setup_file(root, p[1]).map(std::fs::read) {
                Some(Ok(d)) => vinyl::Livery::from_xcc(&d),
                _ => vinyl::Presets::load(&opt.data_dir).livery(p[1]),
            };
            println!("zones {:?}, layers {:?}", zones.as_ref().map(|z| z.centres), livery.zones.iter().map(|z| z.len()).collect::<Vec<_>>());
            if let Some(z) = zones {
                let t = Instant::now();
                let px = vinyl::compose(&z, &livery, &mut lib, 512);
                println!("composed in {:.0} ms", t.elapsed().as_secs_f32() * 1000.0);
                let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(p[2])?), 512, 512);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header()?.write_image_data(&px)?;
            }
        }
        return Ok(());
    }
    hooks::start_file_log();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = bootload::Runner::new(App::new(opt));
    event_loop.run_app(&mut app)?;
    Ok(())
}


/// The most open piece of floor inside an interior set: (position, free radius).
fn garage_spot(col: &physics::collision::WorldCollider, mn: Vec3, mx: Vec3) -> Option<(Vec3, f32)> {
    let mut best: Option<(Vec3, f32)> = None;
    let dirs: Vec<Vec3> = (0..12).map(|k| {
        let a = k as f32 / 12.0 * std::f32::consts::TAU;
        Vec3::new(a.cos(), 0.0, a.sin())
    }).collect();
    let mut x = mn.x + 3.0;
    while x < mx.x - 3.0 {
        let mut z = mn.z + 3.0;
        while z < mx.z - 3.0 {
            let zz = z;
            z += 1.5;
            // the floor may lie above the lowest point of the set (pits, ramps): probe a few levels
            let floor = [1.6f32, 3.6, 5.6].iter().find_map(|h| col.raycast(Vec3::new(x, mn.y + h, zz), -Vec3::Y, 2.2).filter(|g| g.normal.y > 0.97 && col.raycast(g.point + Vec3::Y * 2.2, Vec3::Y, 40.0).is_some()));
            let Some(g) = floor else { continue };
            let mut clear = 9.0f32;
            for h in [0.3f32, 0.9] {
                for d in &dirs {
                    if let Some(hit) = col.raycast(g.point + Vec3::Y * h, *d, clear) {
                        clear = clear.min(hit.t);
                    }
                }
                if clear < 3.0 {
                    break;
                }
            }
            if clear >= 3.0 && best.map_or(true, |b| clear > b.1) {
                best = Some((g.point, clear));
            }
        }
        x += 1.5;
    }
    log::info!("garage spot in {:.0?}..{:.0?}: {:?}", mn, mx, best);
    best
}

/// The city map on the GPU (see `citymap.rs`).
struct CityMapGpu {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
    bind: Arc<wgpu::BindGroup>,
    map: citymap::CityMap,
    low: render::ObjectBinding,
    /// the close-up patch: buffers, index count, and what it was built for (centre, half size, step)
    patch: Option<(wgpu::Buffer, wgpu::Buffer, u32, (f32, f32, f32, usize))>,
}

/// `garageEnvMap.dds` as RGBA8 (256 x 128): the garage's lights for the cars' mirror map.
static GARAGE_GLOW: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// CarSelection's camera over the lot (game/career/0xebb682f7): the first eye and look (local45, local48), and the
/// eye and look targets for the three starters in the screen's order (local25, local35)
const SEL_EYE0: Vec3 = Vec3::new(-174.542, 20.308, -773.402);
const SEL_LOOK0: Vec3 = Vec3::new(-176.885, 17.332, -746.174);
const SEL_EYE: [Vec3; 3] = [Vec3::new(-174.154, 19.609, -769.263), Vec3::new(-181.709, 19.577, -764.572), Vec3::new(-179.077, 19.065, -765.562)];
const SEL_LOOK: [Vec3; 3] = [Vec3::new(-172.353, 16.673, -751.904), Vec3::new(-165.489, 16.825, -755.183), Vec3::new(-179.634, 18.415, -749.725)];

/// One step of CarSelection's camera function (0x1069): the leg with further to go speeds up by 0.1 (to 1.4) while it
/// is more than four steps away and slows by 0.7 inside that, never past its target; the other leg takes the speed
/// that brings it in at the same time. Returns eye, look and their speeds.
fn sel_cam_step((eye, look, se, sl): (Vec3, Vec3, f32, f32), eye_t: Vec3, look_t: Vec3) -> (Vec3, Vec3, f32, f32) {
    let (d8, d9) = (eye.distance(eye_t), look.distance(look_t));
    let lead = |d: f32, v: f32| (if d > v * 4.0 { (v + 0.1).min(1.4) } else { v * 0.7 }).min(d);
    let (se, sl) = if d8 >= d9 {
        let se = lead(d8, se);
        (se, if d8 > 0.0 { (se * d9 / d8).min(d9) } else { 0.0 })
    } else {
        let sl = lead(d9, sl);
        ((sl * d8 / d9).min(d8), sl)
    };
    let go = |p: Vec3, t: Vec3, d: f32, v: f32| if d > 0.0 { p + (t - p) / d * v } else { p };
    (go(eye, eye_t, d8, se), go(look, look_t, d9, sl), se, sl)
}

/// The hour's light and m_NFLFade of the last frame's lighting, for the traffic cars' light slots.
static LAST_HOUR: std::sync::Mutex<Option<(f32, (Vec3, Vec3))>> = std::sync::Mutex::new(None);

/// The two best of a car's light candidates, F-24398: (best id, its weight, second id, its weight);
/// -1 where none weighs over 0. A candidate replaces the best only when strictly heavier.
fn pick_two(cand: &[(i32, f32, Vec3, Vec3, f32)]) -> (i32, f32, i32, f32) {
    let (mut b0, mut b1, mut w0, mut w1) = (-1i32, -1i32, 0.0f32, 0.0f32);
    for c in cand {
        if c.1 > w0 {
            b1 = b0;
            w1 = w0;
            b0 = c.0;
            w0 = c.1;
        } else if c.1 > w1 {
            b1 = c.0;
            w1 = c.1;
        }
    }
    (b0, w0, b1, w1)
}

/// The slot keeping of F-99357 / F-56246: the best goes to the first slot and the second to the other,
/// swapped when the best already sits in the second slot (and not in the first) or the second sits
/// in the first (and not in the second).
fn slot_swap(held: [i32; 2], b0: i32, b1: i32) -> bool {
    (b0 != held[0] && b0 == held[1]) || (b1 != held[1] && b1 == held[0])
}

/// F-99357's calls per drawn frame: the ambient cars' pass dispatcher F-27833 runs it in the main pass and
/// again in the blended bucket's pass (0x100000), so twice for a car drawn in both. Read live (2026-10-29,
/// a parked Cobalt's first-slot strength knocked to 0 and polled): it steps by 6 % twice ~3 ms apart, then
/// waits ~31 ms - two calls per 30 Hz frame, every step exactly one call (log ratio 1.00).
fn amb_slot_calls() -> f32 {
    2.0
}
