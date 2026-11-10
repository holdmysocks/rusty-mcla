//! Career speech: the phone calls and radio chatter of `tune/audio/CareerScript.xml`
//! (scenarios such as `SIDEKICK_<MISSION>`: a script of `PlayAsset` / `WaitTime` steps).
//! An asset's clips are named `joaat("<ASSET>_NN")` in the `career_speech` banks; the
//! player's lines live in banks of their own, the same asset name in another bank is the
//! other speaker.

use crate::audio::Audio;
use hashbrown::{HashMap, HashSet};
use mcla_formats::hash::joaat;
use std::collections::VecDeque;
use std::path::Path;

#[derive(Clone, Debug)]
enum Step {
    /// `voice`: the profile whose banks say it (GH_01 ...), None - any; `rec`: the asset record (its own last take, +20 of
    /// the record: F-70477); `force`: ForceVariation (+21, 0 = none); `post`: SoundPostFix (the voice's sound object
    /// "<voice>_<post>"); `attach`: said by the requesting entity (F-30989 gives the line the entity, F-18143)
    Play { asset: String, player: bool, voice: Option<String>, rec: u32, force: u8, post: Option<String>, attach: bool },
    /// a clip by its full name (the taunts' special-case assets): from `voice`'s banks, or the player's
    Clip { name: String, player: bool, voice: Option<String> },
    Wait(f32),
    /// `<SpecialCase name="SIDEKICK_TELEPHONE_INVITE|DECLINE">` (F-87287): the clip is named `<name>_<profile>_<A..E>_01`
    /// when the scenario is queued, in the caller's voice (`player`: the receiver, the player's own line)
    Phone { name: String, player: bool },
    /// the scenario's dynamic mixer (`DynMixer`, default DYNAMIC_MIXER_SPEECH_DEFAULT; "" = NULL): on while it plays
    Mixer(String),
}

/// The bank that holds the player's replies to every hookman's pre-race taunt (`PRERACE_TAUNT_HP_*` /
/// `_PH_*` receiver / sender lines of the player).
const PLAYER_TAUNT_BANK: &str = "0x0624a35b";
/// The bank of the player's telephone lines (`SIDEKICK_TELEPHONE_INVITE_` / `_DECLINE_<profile>_<A..E>_01`).
const PLAYER_PHONE_BANK: &str = "0xb84d2992";

/// A scenario's speech-manager fields (the loader F-66242): Priority (+12, default 3), Delay (+16, s, default 0),
/// Frequency (+20, s, default 10), Probability (+64, percent clamped 0..100, default 100), the time of its last start
/// (+24, -FLT_MAX at load) and its last script (+28, -1 at load); `force_sender`: its ForceSender voice (+32) is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gate {
    pub priority: i32,
    pub delay: f32,
    pub frequency: f32,
    pub probability: i32,
    pub last: f32,
    pub last_script: i32,
    pub force_sender: bool,
}

/// The entity a speech request comes from (F-95157's r5, kept at +2252 while its scenario plays): `rank` is its +36.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sender {
    pub id: u32,
    pub rank: i32,
}

/// The two senders on the one speech manager: the player's speaker (the global F-44958 makes: +36 = 1), whose requests
/// are every career / taunt / player line (F-30989, F-70230, F-19599, F-51720 and the natives all pass it), and the police
/// radio's (F-45942: +36 = 0 from the base constructor F-32285), whose reports go through the same start (F-84173, F-14569).
/// So a player request cuts a radio line whatever the Priorities, and a radio request is refused while the player's
/// speaker plays.
pub const PLAYER: Sender = Sender { id: 1, rank: 1 };
pub const RADIO: Sender = Sender { id: 2, rank: 0 };

/// A request to the scenario start F-95157 beside the scenario: `profile` the voice the computer lines get (`say_as`),
/// `attach` the lines are said by the requesting entity (F-30989), `sender` the entity (+2252), `voice` the sender's name
/// (None: no name; Some(found): the name and whether the entity's vt+44 found its voice), `forced` (r8: no Frequency /
/// Probability gate), `keep` (r9: the first line's +467 - it holds until the same scenario is asked again), `wait` (r10:
/// while a scenario plays the request waits in the ring of 8).
#[derive(Clone, Copy, Debug, Default)]
pub struct Ask<'a> {
    pub profile: &'a str,
    pub attach: bool,
    pub sender: Option<Sender>,
    pub voice: Option<bool>,
    pub forced: bool,
    pub keep: bool,
    pub wait: bool,
}

/// A request waiting in the manager's ring (the 268-byte records at +28: the scenario +256, the sender +260, the names
/// +0 / +128, forced +264, the byte +265; F-13869 pushes, F-85023 pops). The ring is the manager's, one for both senders:
/// `radio` = the police radio's request (sender `RADIO`) with its sender and receiver call signs (+0 / +128).
#[derive(Clone, Debug)]
struct Pending {
    key: String,
    radio: Option<(String, String)>,
    profile: String,
    attach: bool,
    sender: Option<Sender>,
    voice: Option<bool>,
    forced: bool,
    keep: bool,
}

impl Gate {
    /// The fields of a `<Scenario ...>` tag.
    pub fn parse(tag: &str) -> Gate {
        let num = |a: &str| attr(tag, a).and_then(|v| v.trim().parse::<f32>().ok());
        Gate {
            priority: num("Priority").map_or(3, |v| v as i32),
            delay: num("Delay").unwrap_or(0.0),
            frequency: num("Frequency").unwrap_or(10.0),
            probability: num("Probability").map_or(100, |v| (v as i32).clamp(0, 100)),
            last: -f32::MAX,
            last_script: -1,
            force_sender: attr(tag, "ForceSender").is_some(),
        }
    }
}

#[derive(Default)]
pub struct Speech {
    /// each scenario's `<Script>` blocks: alternatives, one of them is said
    scenarios: HashMap<String, Vec<Vec<Step>>>,
    /// clip hash -> clips ("career_speech/<bank>/<file>")
    clips: HashMap<u32, Vec<String>>,
    player_banks: HashSet<String>,
    /// profile (GH_01, SH_02, PLAYER_01 ..) -> its banks ("career_speech/0x<joaat(bank name)>")
    voice_banks: HashMap<String, HashSet<String>>,
    queue: VecDeque<Step>,
    wait: f32,
    rng: u32,
    /// the line playing now: (the player's, seconds left) - the speakers' jaw flap
    line: Option<(bool, f32)>,
    /// the pre-race taunt counters (F-44958 starts them at random): H 1..12, the HP / PH letter A..F
    taunt_h: u32,
    taunt_l: u32,
    /// the telephone lines' letter, 'A'..'E' (the byte at +70 of F-87287's instance: random at the start, one on per call)
    phone_l: u8,
    taunt_ready: Vec<Step>,
    /// the dynamic mixer of the scenario playing now, and whether it was switched on
    mixer: String,
    mixer_on: bool,
    /// each scenario's gate fields (`Gate`)
    gates: HashMap<String, Gate>,
    /// the speech manager's clock (+2244): the update (F-85023, from F-57187 every frame) adds G+92, the frame's real time, also while the game is
    /// paused (recorded: work/slipstream/audio3/cap/n0.txt, 9.03 s in 9.05 s of a pause)
    clock: f32,
    /// each asset record's last take (+20 of the record, F-70477: global per record)
    last_take: HashMap<u32, i32>,
    /// the player's voice (CareerVoices.xml, profile PLAYER_01): its sound objects are CAREER_SPEECH_<voice>_<SoundPostFix>
    player_voice: String,
    /// the game is paused: the line's voice is on the audio timer 0 (the defaults of its request, F-70477), which F-99037
    /// pauses - it holds where it is and the line with it (set by the game each frame)
    pub paused: bool,
    /// the wait running now is a line playing (it holds in a pause), not a WaitTime
    line_wait: bool,
    /// the Priority (+12) of the scenario `request` started, while it plays
    cur_priority: Option<i32>,
    /// the scenario playing (+2260) and its sender entity (+2252)
    cur_key: Option<String>,
    sender: Option<Sender>,
    /// the first line of the scenario playing is held (+467: its delay does not run, F-35459, until a request of the same
    /// scenario plays it, F-34685)
    held: bool,
    /// the requests waiting while a scenario plays (the ring of 8, F-13869; F-85023 starts the oldest when none plays)
    pending: VecDeque<Pending>,
    /// the sender's voice found (+2200: kept from one start to the next when a start finds none of its own)
    sender_voice: bool,
    /// the last request was refused for a script with no line (tests)
    #[cfg(test)]
    no_line: bool,
    /// the police radio plays a scenario (the port's radio is its own player: this is the manager's +2260 with the radio
    /// as +2252 - `RADIO`); set by the game before the requests
    pub radio_on: bool,
    /// a start cut the radio's scenario (F-73277 stops it): the game stops the radio
    pub cut_radio: bool,
    /// the radio's request F-85023 took out of the ring this update (scenario, sender, receiver): the game hands it to
    /// the radio (`radio::Radio::say_from`), whose start is the port's F-95157 for the sender `RADIO`
    pub radio_next: Option<(String, (String, String))>,
}

fn attr<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let k = format!("{name}=\"");
    let i = s.find(&k)? + k.len();
    let e = s[i..].find('"')?;
    Some(&s[i..i + e])
}

impl Speech {
    pub fn load(data_dir: &str, audio_root: &Path) -> Speech {
        let mut sp = Speech { rng: 0x2545_f491, ..Default::default() };
        // clip index of the speech banks
        if let Ok(rd) = std::fs::read_dir(audio_root.join("career_speech")) {
            for e in rd.flatten() {
                let bank = e.file_name().to_string_lossy().to_string();
                let Ok(t) = std::fs::read_to_string(e.path().join("_index.tsv")) else { continue };
                for line in t.lines() {
                    let mut c = line.split('\t');
                    let (Some(name), Some(hash)) = (c.next(), c.next()) else { continue };
                    if let Ok(h) = u32::from_str_radix(hash.trim(), 16) {
                        sp.clips.entry(h).or_default().push(format!("career_speech/{bank}/{name}"));
                    }
                }
            }
        }
        // the banks' names (speech.dat: CAREER_SPEECH\[SC_]GH_01_MARTIN_RADIO ...): whose voice each is
        if let Ok(d) = std::fs::read(Path::new(data_dir).join("audio/X360/config/speech.dat")) {
            let pat = b"CAREER_SPEECH\\";
            let mut i = 0;
            while let Some(k) = d[i..].windows(pat.len()).position(|w| w == pat) {
                let st = i + k + pat.len();
                let end = d[st..].iter().position(|c| !(c.is_ascii_alphanumeric() || *c == b'_')).map_or(d.len(), |e| st + e);
                let name = String::from_utf8_lossy(&d[st..end]).to_string();
                let core = name.strip_prefix("SC_").unwrap_or(&name);
                let mut parts = core.splitn(3, '_');
                if let (Some(a), Some(b)) = (parts.next(), parts.next()) {
                    sp.voice_banks.entry(format!("{a}_{b}").to_ascii_uppercase()).or_default().insert(format!("career_speech/0x{:08x}", joaat(&name)));
                }
                i = end.max(st);
            }
        }
        // the career's script, and the South Central add-on's beside it
        let read = |name: &str| std::fs::read(Path::new(data_dir).join("tune/audio").join(name)).map(|b| String::from_utf8_lossy(&b).to_string());
        let Ok(mut t) = read("CareerScript.xml") else { return sp };
        if let Ok(sc) = read("SC_CareerScript.xml") {
            t.push_str(&sc);
        }
        let mut player_only: HashMap<String, u32> = HashMap::new();
        // (each PlayAsset / SpecialCase is an asset record of its own)
        let mut recs = 0u32;
        // the player's voice (CareerVoices.xml: the Voice whose ProfileName is PLAYER_01 - DRV_MP_01_SET)
        if let Ok(v) = read("CareerVoices.xml") {
            sp.player_voice = v.split("<Voice ").skip(1).find(|t| attr(t, "ProfileName") == Some("PLAYER_01")).and_then(|t| attr(t, "name")).unwrap_or("").to_string();
        }
        for sc in t.split("<Scenario ").skip(1) {
            let Some(name) = attr(sc, "name") else { continue };
            let body = sc.split("</Scenario>").next().unwrap_or("");
            let mut scripts: Vec<Vec<Step>> = Vec::new();
            let mixer = match attr(sc.split('>').next().unwrap_or(""), "DynMixer") {
                Some("NULL") => String::new(),
                Some(m) => m.to_string(),
                None => "DYNAMIC_MIXER_SPEECH_DEFAULT".to_string(),
            };
            let mut steps = vec![Step::Mixer(mixer.clone())];
            for line in body.lines() {
                let l = line.trim();
                if l.starts_with("<!--") {
                    continue;
                }
                // (step 19d: a new <Script> is another way of saying it)
                if l.starts_with("<Script") && steps.len() > 1 {
                    scripts.push(std::mem::replace(&mut steps, vec![Step::Mixer(mixer.clone())]));
                }
                if l.starts_with("<PlayAsset") {
                    let Some(asset) = attr(l, "name") else { continue };
                    let player = attr(l, "VoiceGroup") == Some("PLAYER");
                    if player {
                        // banks that hold this player line (unambiguous ones tell us whose banks they are)
                        if let Some(c) = sp.clips.get(&joaat(&format!("{asset}_01"))) {
                            if c.len() == 1 {
                                let bank = c[0].rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default();
                                *player_only.entry(bank).or_default() += 1;
                            }
                        }
                    }
                    let force = attr(l, "ForceVariation").and_then(|v| v.trim().parse::<u8>().ok()).unwrap_or(0);
                    let post = attr(l, "SoundPostFix").map(|v| v.to_string());
                    recs += 1;
                    steps.push(Step::Play { asset: asset.to_string(), player, voice: None, rec: recs, force, post, attach: false });
                } else if l.starts_with("<SpecialCase") {
                    // (the career radio's squelch clicks around a line: RADIO_PRE_SQUELCH_01..03 / RADIO_POST_SQUELCH_01..03,
                    // bank 0x52b9ce47 - a take of three, like any asset)
                    if let Some(n) = attr(l, "name").filter(|n| *n == "RADIO_PRE_SQUELCH" || *n == "RADIO_POST_SQUELCH") {
                        recs += 1;
                        steps.push(Step::Play { asset: n.to_string(), player: false, voice: None, rec: recs, force: 0, post: None, attach: false });
                    }
                    if let Some(n) = attr(l, "name").filter(|n| *n == "SIDEKICK_TELEPHONE_INVITE" || *n == "SIDEKICK_TELEPHONE_DECLINE") {
                        steps.push(Step::Phone { name: n.to_string(), player: attr(l, "VoiceGroup") == Some("PLAYER") });
                    }
                } else if l.starts_with("<WaitTime") {
                    steps.push(Step::Wait(attr(l, "value").and_then(|v| v.parse().ok()).unwrap_or(0.4)));
                }
            }
            if steps.len() > 1 {
                scripts.push(steps);
            }
            if !scripts.is_empty() {
                sp.gates.insert(name.to_ascii_uppercase(), Gate::parse(sc.split('>').next().unwrap_or("")));
                sp.scenarios.insert(name.to_ascii_uppercase(), scripts);
            }
        }
        // the player's voice: banks that hold many player-only lines
        sp.player_banks = player_only.into_iter().filter(|(_, n)| *n >= 8).map(|(b, _)| b).collect();
        log::info!("speech: {} scenarios, {} clips, {} player banks", sp.scenarios.len(), sp.clips.len(), sp.player_banks.len());
        sp
    }

    pub fn has(&self, scenario: &str) -> bool {
        self.scenarios.contains_key(&scenario.to_ascii_uppercase())
    }

    pub fn busy(&self) -> bool {
        !self.queue.is_empty() || self.wait > 0.0
    }

    /// A scenario's gate fields (None: no such scenario).
    #[cfg(test)]
    pub fn gate(&self, scenario: &str) -> Option<Gate> {
        self.gates.get(&scenario.to_ascii_uppercase()).copied()
    }

    /// F-62480(lo, hi) on the audio's shared state (`sndobj::audio_rand`: the sound objects' variances and Randomized
    /// picks draw from the same state in the original - not the game's other generator, `sst_trail::game_rng`).
    /// Always draws.
    fn rand(&mut self, lo: i32, hi: i32) -> i32 {
        crate::sndobj::audio_rand(lo, hi)
    }

    /// A speech request as F-30989 / F-62728 make it (`"PLAYER_NITROUS"`, voice group "PLAYER", not forced, not kept, not
    /// waiting): `start`. (The name PLAYER's lookup is taken as found, not traced: every PLAYER_* scenario has a ForceSender,
    /// which comes first.)
    pub fn request(&mut self, scenario: &str) -> bool {
        self.start(scenario, Ask { attach: true, sender: Some(PLAYER), voice: Some(true), ..Default::default() })
    }

    /// The same start asked by F-52240 through F-70230 (scenario, voice group = a racer's driver name: not forced, not
    /// kept, not waiting): no entity for the lines, the computer lines in `profile`'s voice as `say_as` gives them; an
    /// empty `profile` (no voice found for the racer) finds no sender voice.
    pub fn request_as(&mut self, scenario: &str, profile: &str) -> bool {
        self.start(scenario, Ask { profile, sender: Some(PLAYER), voice: Some(!profile.is_empty()), ..Default::default() })
    }

    /// F-23328's request (scenario, the racer's driver name): F-70230 with r9 = 1, so it waits in the ring while a scenario
    /// plays (`request_as` otherwise).
    pub fn request_taunt(&mut self, scenario: &str, profile: &str) -> bool {
        self.start(scenario, Ask { profile, sender: Some(PLAYER), voice: Some(!profile.is_empty()), wait: true, ..Default::default() })
    }

    /// The scenario start F-95157 (offline: the online check F-24338 refuses everything online). A waiting request while
    /// a scenario plays goes to the ring (refused when 8 wait) and is taken, with no gate. Not forced: refused while the
    /// clock is below its last start + Frequency, then unless rand(0, 99) < Probability (the draw only when the clock
    /// passed). No script: refused. A scenario playing: the same scenario with its first line held is played (F-34685),
    /// taken; another sender entity playing is cut only by a higher rank (+36), else a higher Priority (+12, unsigned) cuts
    /// it (F-73277); otherwise refused. On a start: the last start = the clock, the script drawn = rand(0, count - 1) is the
    /// one said, the last script (+28) = that draw, one on (wrapping) when it equals the last one (the script's pointer is
    /// taken before the bump: a scenario of two scripts can say the same one twice); the sender's voice is the ForceSender
    /// (+32), else the name's (vt+44), else the one kept (+2200): none refuses (the last start and script kept). The Delay
    /// comes before the first line; a script with no line (PlayAsset / SpecialCase) stops the speech and the start is
    /// refused. The first line keeps `keep` (+467). Recorded (work/slipstream/audio3/cap/n0.txt): the gate draw 27 < 30
    /// passes, the script draw 0 next, a request 66.3 s after the start refused with no draw, one at 120.7 s drawn 36 and
    /// refused (the last start kept). Run offline whole on 4,000 states: verify_audio_b1.md batches 54 and 55.
    pub fn start(&mut self, scenario: &str, ask: Ask) -> bool {
        let key = scenario.to_ascii_uppercase();
        let (Some(g), Some(count)) = (self.gates.get(&key).copied(), self.scenarios.get(&key).map(|s| s.len())) else { return false };
        // (a scenario plays: this speaker's, or the radio's - `radio_on`)
        let playing = self.busy() || self.radio_on;
        if ask.wait && playing {
            if self.pending.len() == 8 {
                return false;
            }
            let Ask { profile, attach, sender, voice, forced, keep, .. } = ask;
            self.pending.push_back(Pending { key, radio: None, profile: profile.to_string(), attach, sender, voice, forced, keep });
            return true;
        }
        if !ask.forced && (self.clock < g.last + g.frequency || self.rand(0, 99) >= g.probability) {
            return false;
        }
        if count == 0 {
            return false;
        }
        if playing {
            if !self.radio_on && self.cur_key.as_deref() == Some(&key) && self.held {
                // (F-34685: the held line's +467 cleared, its tick runs)
                self.held = false;
                return true;
            }
            let on = if self.radio_on { Some(RADIO) } else { self.sender };
            let cut = match on.filter(|s| Some(*s) != ask.sender) {
                Some(s) => ask.sender.map_or(0, |a| a.rank) > s.rank,
                None => self.cur_priority.is_none_or(|p| g.priority as u32 > p as u32),
            };
            if !cut {
                return false;
            }
            self.cut();
            if self.radio_on {
                self.radio_on = false;
                self.cut_radio = true;
            }
        }
        let pick = self.rand(0, count as i32 - 1);
        let mut last = pick;
        if last == g.last_script {
            last += 1;
            if last == count as i32 {
                last = 0;
            }
        }
        if let Some(e) = self.gates.get_mut(&key) {
            e.last = self.clock;
            e.last_script = last;
        }
        if g.force_sender {
            self.sender_voice = true;
        } else if let Some(found) = ask.voice {
            self.sender_voice = found;
        }
        if !self.sender_voice {
            return false;
        }
        let steps = self.scenarios[&key][pick as usize].clone();
        // (no line in the script: F-95157 logs it, stops everything and refuses)
        if !steps.iter().any(|st| matches!(st, Step::Play { .. } | Step::Clip { .. } | Step::Phone { .. })) {
            #[cfg(test)]
            {
                self.no_line = true;
            }
            self.cut();
            return false;
        }
        let profile = ask.profile;
        let steps = self.phone_steps(steps, profile);
        // (F-30989 then gives the lines the requesting entity, F-18143: the player's car for PLAYER_NITROUS / PLAYER_IN_AIR)
        let pv = (!profile.is_empty()).then(|| profile.to_ascii_uppercase());
        let steps = steps.into_iter().map(|st| match st {
            Step::Play { asset, player, voice, rec, force, post, attach: at } => {
                let voice = if player || pv.is_none() { voice } else { pv.clone() };
                Step::Play { asset, player, voice, rec, force, post, attach: if ask.attach { true } else { at } }
            }
            Step::Clip { name, player: false, voice } => Step::Clip { name, player: false, voice: pv.clone().or(voice) },
            o => o,
        });
        if g.delay > 0.0 {
            self.queue.push_back(Step::Wait(g.delay));
        }
        self.queue.extend(steps);
        self.cur_priority = Some(g.priority);
        self.cur_key = Some(key.clone());
        self.sender = ask.sender;
        self.held = ask.keep;
        log::debug!("speech: {} started at {:.3} s (script {}, last {})", key, self.clock, pick, last);
        true
    }

    /// The radio's waiting request (F-14569's report with +40 = 1 -> F-95157 with r10 = 1, sender `RADIO`) while a
    /// scenario plays: into the one ring (refused when 8 wait - the player's waiting requests count too).
    pub fn queue_radio(&mut self, scenario: &str, parties: (String, String)) -> bool {
        if self.pending.len() == 8 {
            return false;
        }
        self.pending.push_back(Pending { key: scenario.to_string(), radio: Some(parties), profile: String::new(), attach: false, sender: Some(RADIO), voice: None, forced: false, keep: false });
        true
    }

    /// Requests waiting in the ring (both senders').
    pub fn waiting(&self) -> usize {
        self.pending.len()
    }

    /// F-73277's bookkeeping: the lines to come, the scenario playing and its sender gone.
    fn cut(&mut self) {
        self.queue.clear();
        self.wait = 0.0;
        self.cur_priority = None;
        self.cur_key = None;
        self.sender = None;
        self.held = false;
    }

    /// Queue a scenario; returns false if it does not exist.
    pub fn say(&mut self, scenario: &str) -> bool {
        let Some(scripts) = self.scenarios.get(&scenario.to_ascii_uppercase()) else { return false };
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let steps = scripts[(self.rng >> 16) as usize % scripts.len()].clone();
        let steps = self.phone_steps(steps, "");
        if !self.queue.is_empty() {
            self.queue.push_back(Step::Wait(0.8));
        }
        self.queue.extend(steps);
        true
    }

    /// Queue a scenario said in a character's voice (`profile` = GH_01 ..: the racer's - the
    /// original passes the racer, whose character set is the voice of CareerVoices.xml); the
    /// player's lines stay the player's. False if it does not exist.
    pub fn say_as(&mut self, scenario: &str, profile: &str) -> bool {
        let Some(scripts) = self.scenarios.get(&scenario.to_ascii_uppercase()) else { return false };
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let steps = scripts[(self.rng >> 16) as usize % scripts.len()].clone();
        let steps = self.phone_steps(steps, profile);
        if !self.queue.is_empty() {
            self.queue.push_back(Step::Wait(0.8));
        }
        let voice = (!profile.is_empty()).then(|| profile.to_ascii_uppercase());
        self.queue.extend(steps.into_iter().map(|s| match s {
            Step::Play { asset, player: false, rec, force, post, attach, .. } => Step::Play { asset, player: false, voice: voice.clone(), rec, force, post, attach },
            Step::Clip { name, player: false, .. } => Step::Clip { name, player: false, voice: voice.clone() },
            o => o,
        }));
        true
    }

    /// The telephone lines of a scenario become clips of `profile`'s voice (F-87287: the letter steps on once per
    /// call, 'A'..'E', and both lines of the call have it); without a voice they are dropped.
    fn phone_steps(&mut self, steps: Vec<Step>, profile: &str) -> Vec<Step> {
        if !steps.iter().any(|s| matches!(s, Step::Phone { .. })) {
            return steps;
        }
        if self.phone_l == 0 {
            self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
            self.phone_l = b'A' + ((self.rng >> 16) % 5) as u8;
        }
        self.phone_l = if self.phone_l >= b'E' { b'A' } else { self.phone_l + 1 };
        let voice = profile.to_ascii_uppercase();
        steps
            .into_iter()
            .filter_map(|s| match s {
                Step::Phone { name, player } => (!voice.is_empty()).then(|| Step::Clip { name: format!("{name}_{voice}_{}_01", self.phone_l as char), player, voice: (!player).then(|| voice.clone()) }),
                o => Some(o),
            })
            .collect()
    }

    /// The banks of a profile's voice (a test aid).
    pub fn voice_banks(&self, profile: &str) -> usize {
        self.voice_banks.get(&profile.to_ascii_uppercase()).map_or(0, |b| b.len())
    }

    pub fn stop(&mut self) {
        self.queue.clear();
    }

    /// mcCareerSpeech_Stop (F-90827 -> F-73277, while the speech system is the career speech's): everything stops -
    /// the line on show, the lines to come, its wait; the scenario's mixer goes with the speech no longer busy.
    pub fn stop_now(&mut self, a: &mut Audio) {
        self.cut();
        self.line_wait = false;
        self.line = None;
        a.obj_line_stop();
    }

    /// Whose line is playing: Some(true) the player's, Some(false) the other speaker's.
    pub fn line(&self) -> Option<bool> {
        self.line.map(|l| l.0)
    }

    fn clip_in(&self, name: &str, player: bool, voice: Option<&str>) -> Option<String> {
        let c = self.clips.get(&joaat(name))?;
        let bank_of = |k: &String| k.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default();
        if player {
            return c.iter().find(|k| bank_of(k).ends_with(PLAYER_TAUNT_BANK) || bank_of(k).ends_with(PLAYER_PHONE_BANK)).cloned();
        }
        let vb = self.voice_banks.get(voice?)?;
        c.iter().find(|k| vb.contains(&bank_of(k))).cloned()
    }

    /// Prepare a pre-race taunt (`CineScript_RequestPreraceTauntScenarioAudio`, scenario `TAUNT_PRERACE_<kind>`:
    /// HP = the hookman, 0.6 s, the player's reply; PH = the player first; H = the hookman alone). The asset
    /// names are the special cases of F-87287: H `PRERACE_TAUNT_H_<NN>` (NN 1..12 in turn), HP / PH
    /// `PRERACE_TAUNT_<kind>_<hookman's profile>_<A..F in turn>_01` for both lines. False when the hookman has
    /// no voice for it (the scenario is then not prepared: no windows, no speech).
    pub fn prepare_taunt(&mut self, kind: &str, hookman: &str) -> bool {
        let voice = hookman.to_ascii_uppercase();
        if self.taunt_h == 0 {
            self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
            self.taunt_h = 1 + (self.rng >> 16) % 12;
            self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
            self.taunt_l = (self.rng >> 16) % 6;
        }
        let steps = match kind {
            "H" => {
                let name = format!("PRERACE_TAUNT_H_{:02}", self.taunt_h);
                vec![Step::Clip { name, player: false, voice: Some(voice.clone()) }]
            }
            "HP" | "PH" => {
                let name = format!("PRERACE_TAUNT_{kind}_{voice}_{}_01", (b'A' + self.taunt_l as u8) as char);
                let hook = Step::Clip { name: name.clone(), player: false, voice: Some(voice.clone()) };
                let me = Step::Clip { name, player: true, voice: None };
                if kind == "HP" { vec![hook, Step::Wait(0.6), me] } else { vec![me, Step::Wait(0.6), hook] }
            }
            _ => return false,
        };
        let ok = steps.iter().all(|st| match st {
            Step::Clip { name, player, voice } => self.clip_in(name, *player, voice.as_deref()).is_some(),
            _ => true,
        });
        if !ok {
            for st in &steps {
                if let Step::Clip { name, player, .. } = st {
                    log::info!("speech: taunt clip {} player {}: {:?} voice banks {:?}", name, player, self.clips.get(&joaat(name)), self.voice_banks.get(&voice));
                }
            }
            log::info!("speech: taunt {} for {} not on the disc", kind, voice);
            return false;
        }
        if kind == "H" {
            self.taunt_h = self.taunt_h % 12 + 1;
        } else {
            self.taunt_l = (self.taunt_l + 1) % 6;
        }
        self.taunt_ready = steps;
        true
    }

    /// `playspeech`: say the prepared taunt.
    pub fn play_taunt(&mut self) -> bool {
        let steps = std::mem::take(&mut self.taunt_ready);
        if steps.is_empty() {
            return false;
        }
        self.queue.clear();
        self.wait = 0.0;
        // (the TAUNT_PRERACE_* scenarios' DynMixer)
        self.queue.push_back(Step::Mixer("DYNAMIC_MIXER_PRERACE_TAUNT".into()));
        self.queue.extend(steps);
        true
    }

    /// The take of a line (F-70477): n = the takes of the context in the speaker's voice (F-99296 counts them), v =
    /// rand(1, n) from the shared generator; when v is the asset record's last take and n > 1, one on (past n: 1);
    /// ForceVariation, when set, replaces it (at most n); v is the record's new last take; the clip is "<context>_%02u".
    fn resolve(&mut self, asset: &str, player: bool, voice: Option<&str>, rec: u32, force: u8) -> Option<String> {
        // numbered takes of the same line (F-99296 counts them from speech.dat: up to 22 for PLAYER_NITROUS - not 8)
        let mut takes: Vec<&Vec<String>> = Vec::new();
        for n in 1..=99 {
            match self.clips.get(&joaat(&format!("{asset}_{n:02}"))) {
                Some(c) => takes.push(c),
                None => break,
            }
        }
        let bank_of = |k: &String| k.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default();
        // (in the speaker's own voice: the takes his banks hold)
        let mut cand: Vec<String> = Vec::new();
        if let Some(vb) = voice.and_then(|v| self.voice_banks.get(v)) {
            cand = takes.iter().filter_map(|t| t.iter().find(|k| vb.contains(&bank_of(k))).cloned()).collect();
        }
        if cand.is_empty() {
            let is_player = |k: &String| self.player_banks.contains(&bank_of(k));
            cand = takes.iter().filter_map(|t| t.iter().find(|k| is_player(k) == player).or(t.first()).cloned()).collect();
        }
        if cand.is_empty() {
            return None;
        }
        let n = cand.len() as i32;
        let mut v = self.rand(1, n);
        if self.last_take.get(&rec) == Some(&v) && n > 1 {
            v += 1;
            if v > n {
                v = 1;
            }
        }
        if force != 0 {
            v = (force as i32).min(n);
        }
        self.last_take.insert(rec, v);
        let k = cand[v as usize - 1].clone();
        log::debug!("speech: {} take {} of {} -> {}", asset, v, n, k);
        Some(k)
    }

    pub fn update(&mut self, a: &mut Audio, dt: f32) {
        self.clock += dt;
        if !self.busy() {
            self.cut();
        }
        // a line's voice holds in a pause (its sound is on the paused timer 0: F-89113 skips it, so the line's end - its
        // sound slot freed, F-78934 - waits); WaitTime delays run on the real frame time (+392 += G+92, F-35459); a held
        // first line (+467) does not run at all
        if !(self.paused && self.line_wait) && !self.held {
            self.wait -= dt;
        }
        if let Some(l) = self.line.as_mut().filter(|_| !self.paused) {
            l.1 -= dt;
            if l.1 <= 0.0 {
                self.line = None;
            }
        }
        if !self.held {
            self.step(a);
        }
        // (F-85023: when no scenario plays, the oldest waiting request is started - everything stopped first, F-73277 -
        // with its own forced flag and byte, not waiting)
        if !self.busy() && !self.radio_on {
            if let Some(p) = self.pending.pop_front() {
                self.stop_now(a);
                if let Some(parties) = p.radio {
                    self.radio_next = Some((p.key, parties));
                    return self.mixer_tick(a);
                }
                let ask = Ask { profile: &p.profile, attach: p.attach, sender: p.sender, voice: p.voice, forced: p.forced, keep: p.keep, wait: false };
                self.start(&p.key, ask);
            }
        }
        self.mixer_tick(a);
    }

    /// The scenario's dynamic mixer while it plays (F-70477 / F-38026: switched on when a line starts - not while the
    /// first line is held).
    fn mixer_tick(&mut self, a: &mut Audio) {
        let on = self.busy() && !self.held && !self.mixer.is_empty();
        if on != self.mixer_on && !self.mixer.is_empty() {
            a.obj_scene(&self.mixer, on);
        }
        self.mixer_on = on;
    }

    fn step(&mut self, a: &mut Audio) {
        while self.wait <= 0.0 {
            let Some(step) = self.queue.pop_front() else {
                self.wait = 0.0;
                return;
            };
            match step {
                Step::Wait(t) => {
                    self.wait = t;
                    self.line_wait = false;
                }
                Step::Phone { .. } => {}
                Step::Mixer(m) => {
                    if m != self.mixer {
                        if self.mixer_on {
                            a.obj_scene(&self.mixer, false);
                            self.mixer_on = false;
                        }
                        self.mixer = m;
                    }
                }
                Step::Clip { name, player, voice } => {
                    let Some(key) = self.clip_in(&name, player, voice.as_deref()) else { continue };
                    let dur = a.clip(&key).map_or(0.0, |c| c.data.len() as f32 / c.rate.max(1) as f32);
                    if dur > 0.0 {
                        log::info!("speech: taunt line {} ({}) -> {} ({:.1} s)", name, if player { "player" } else { "hookman" }, key, dur);
                        // (the career speech plays through its Speech sound: category SPEECH_CAREER)
                        if !a.obj_line(&key, "SPEECH_CAREER", false) {
                            a.play(&key, 1.0);
                        }
                        self.wait = dur;
                        self.line_wait = true;
                        self.line = Some((player, dur));
                    }
                }
                Step::Play { asset, player, voice, rec, force, post, attach } => {
                    let Some(key) = self.resolve(&asset, player, voice.as_deref(), rec, force) else { continue };
                    let dur = a.clip(&key).map_or(0.0, |c| c.data.len() as f32 / c.rate.max(1) as f32);
                    if dur > 0.0 {
                        log::debug!("speech: {} -> {} ({:.1} s)", asset, key, dur);
                        // the player's line through his voice's sound object "CAREER_SPEECH_<voice>_<SoundPostFix>" (F-70477;
                        // POS_YELL: category SPEECH_CAREER_POS, positional) at his car when the request gave it (F-32206);
                        // (approx.) the other speakers' lines keep the category SPEECH_CAREER
                        let object = match (&post, player && !self.player_voice.is_empty()) {
                            (Some(p), true) if p == "POS_YELL" => format!("CAREER_SPEECH_{}_{}", self.player_voice, p),
                            _ => "SPEECH_CAREER".to_string(),
                        };
                        if !a.obj_line(&key, &object, attach && object != "SPEECH_CAREER") {
                            a.play(&key, 1.0);
                        }
                        // the line ends when its sound does (F-78934: the slot's sound gone -> the next line, whose delay
                        // is its WaitTime only; recorded n2: REMINDER_.._B started 0.44 s after A, its WaitTime 0.4, in
                        // two 0.2 s updates) - no gap of its own (was + 0.15 s)
                        self.wait = dur;
                        self.line_wait = true;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The speech manager's gate and script pick (F-95157) against the running original (work/slipstream/audio3/cap,
    /// recorder sst_audio: GATE / PICK / SCEN lines): the generator's state words before the draws, the clock, the outcome.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR (data dir)"]
    fn the_crash_taunt_is_a_gated_request_in_the_racers_voice() {
        let data = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let mut sp = Speech::load(&data, Path::new("."));
        // CareerScript.xml: Probability 50, Delay 1.0, Frequency 20; the hookman line in the GENERIC_HOOKMAN group
        assert_eq!(sp.gate("TAUNT_RACE_BIG_CRASH").map(|g| (g.probability, g.delay, g.frequency)), Some((50, 1.0, 20.0)));
        sp.clock = 100.0;
        // a gate draw below 50 starts it (the same draws as `request`)
        let mut started = false;
        for s in 0..64u32 {
            crate::sndobj::set_audio_rng_state((0x1234_5678 ^ s.wrapping_mul(0x9E37_79B9), 0x0bad_f00d));
            sp.stop();
            sp.wait = 0.0;
            if let Some(e) = sp.gates.get_mut("TAUNT_RACE_BIG_CRASH") {
                e.last = -f32::MAX;
            }
            if sp.request_as("TAUNT_RACE_BIG_CRASH", "gh_01") {
                started = true;
                break;
            }
        }
        assert!(started);
        let plays: Vec<_> = sp.queue.iter().filter_map(|s| match s { Step::Play { voice, attach, player, .. } => Some((voice.clone(), *attach, *player)), _ => None }).collect();
        assert!(!plays.is_empty());
        assert!(plays.iter().all(|(v, a, p)| !p && !a && v.as_deref() == Some("GH_01")), "{:?}", plays);
        assert!(matches!(sp.queue.front(), Some(Step::Wait(w)) if *w == 1.0));
    }

    #[test]
    #[ignore = "requires MCLA_DATA_DIR (data dir)"]
    fn request_gate_from_disc() {
        let data = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let mut sp = Speech::load(&data, Path::new("."));
        // the loader's fields (F-66242)
        let g = sp.gate("PLAYER_NITROUS").unwrap();
        assert_eq!((g.priority, g.delay, g.frequency, g.probability, g.last, g.last_script), (3, 0.0, 120.0, 30, -f32::MAX, -1));
        assert_eq!(sp.gate("PLAYER_HORN").map(|g| (g.probability, g.frequency)), Some((30, 10.0)));
        assert_eq!(sp.gate("PLAYER_IN_AIR").map(|g| (g.probability, g.frequency)), Some((50, 10.0)));
        assert_eq!(sp.gate("PLAYER_WIPE_OUT_TIMEWARP").map(|g| (g.probability, g.frequency, g.delay)), Some((50, 0.0, 1.5)));
        assert_eq!(sp.scenarios["PLAYER_NITROUS"].len(), 2);
        // n0.txt f=311, the nitrous' first burn: clock 279.983, state 505886d5 04f6b060 -> gate draw 27 < 30, the script draw 0,
        // state after 4f0f6058 1c169b0a
        sp.clock = 279.983;
        crate::sndobj::set_audio_rng_state((0x5058_86d5, 0x04f6_b060));
        assert!(sp.request("PLAYER_NITROUS"));
        assert_eq!(crate::sndobj::audio_rng_state(), (0x4f0f_6058, 0x1c16_9b0a));
        let g = sp.gate("PLAYER_NITROUS").unwrap();
        assert_eq!((g.last, g.last_script), (279.983, 0));
        sp.stop();
        sp.wait = 0.0;
        // f=639: clock 346.263 < 279.983 + 120: refused before the draw
        sp.clock = 346.263;
        let st = crate::sndobj::audio_rng_state();
        assert!(!sp.request("PLAYER_NITROUS"));
        assert_eq!(crate::sndobj::audio_rng_state(), st);
        // f=852: clock 400.699, state 52c60400 2a421f84 -> draw 36: refused, the last start kept
        sp.clock = 400.699;
        crate::sndobj::set_audio_rng_state((0x52c6_0400, 0x2a42_1f84));
        assert!(!sp.request("PLAYER_NITROUS"));
        assert_eq!(sp.gate("PLAYER_NITROUS").unwrap().last, 279.983);
        // s0.txt f=467, the slipstream burn: clock 115.061, state cb06e3f0 3347957c -> draw 68: refused
        let mut s0 = Speech::load(&data, Path::new("."));
        s0.clock = 115.061;
        crate::sndobj::set_audio_rng_state((0xcb06_e3f0, 0x3347_957c));
        assert!(!s0.request("PLAYER_NITROUS"));
        assert_eq!(s0.gate("PLAYER_NITROUS").unwrap().last, -f32::MAX);
        // n1.txt (the gate hook printing the draw): f=360 clock 63.031, state 541d8400 24164598 -> 72 (state 463f6198
        // 1e833729): refused; f=573 clock 119.423, state 0154510f 3d3d6924 -> 17, script draw 1 (state 9457687f 2e2ccf41)
        let mut n1 = Speech::load(&data, Path::new("."));
        n1.clock = 63.031;
        crate::sndobj::set_audio_rng_state((0x541d_8400, 0x2416_4598));
        assert!(!n1.request("PLAYER_NITROUS"));
        assert_eq!(crate::sndobj::audio_rng_state(), (0x463f_6198, 0x1e83_3729));
        n1.clock = 119.423;
        crate::sndobj::set_audio_rng_state((0x0154_510f, 0x3d3d_6924));
        assert!(n1.request("PLAYER_NITROUS"));
        assert_eq!((crate::sndobj::audio_rng_state(), n1.gate("PLAYER_NITROUS").unwrap().last_script), ((0x9457_687f, 0x2e2c_cf41), 1));
        // two scripts: the last script kept (+28) alternates, after 0 the next start keeps 1 whatever the draw, and back (the
        // script said is the draw)
        for seed in 0..64u32 {
            sp.stop();
            sp.wait = 0.0;
            let before = sp.gate("PLAYER_NITROUS").unwrap().last_script;
            sp.clock += 1000.0;
            crate::sndobj::set_audio_rng_state((seed.wrapping_mul(0x9E37_79B9), seed));
            // (a draw below 30 passes the gate: try draws until one does)
            while !sp.request("PLAYER_NITROUS") {
                sp.clock += 1000.0;
            }
            assert_eq!(sp.gate("PLAYER_NITROUS").unwrap().last_script, 1 - before);
        }
        // a line plays: another of the same Priority is refused
        sp.clock += 1000.0;
        crate::sndobj::set_audio_rng_state((0x5058_86d5, 0x04f6_b060));
        assert!(sp.busy() && !sp.request("PLAYER_IN_AIR"));
    }

    /// The take of a line (F-70477) against the running original (work/slipstream/audio4/cap/n2.txt, TAKER / TAKE lines): the
    /// draw rand(1, n) from the shared audio state, n the takes of the context (22 for PLAYER_NITROUS, not 8), the clip
    /// "<context>_%02u"; the bump past the record's last take and ForceVariation as decoded.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn take_pick_as_recorded() {
        let data = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&data).expect("audio");
        let mut sp = Speech::load(&data, a.root());
        let file = |n: &str| format!("{:08x}", joaat(n));
        // f=346: state 11f1e126 593987cc, rand(1, 22) -> 7, state after 688a8396 06826a59: PLAYER_NITROUS_07
        crate::sndobj::set_audio_rng_state((0x11f1_e126, 0x5939_87cc));
        let k = sp.resolve("PLAYER_NITROUS", true, None, 1, 0).unwrap();
        assert!(k.ends_with(&file("PLAYER_NITROUS_07")), "{k}");
        assert_eq!(crate::sndobj::audio_rng_state(), (0x688a_8396, 0x0682_6a59));
        // f=447: one take - rand(1, 1) still draws: state 5406f025 21dfaa6c -> 64b6748f 1e7b0689
        crate::sndobj::set_audio_rng_state((0x5406_f025, 0x21df_aa6c));
        let k = sp.resolve("REMINDER_REPEAT_BESTOF_HILLS_A", true, None, 2, 0).unwrap();
        assert!(k.ends_with(&file("REMINDER_REPEAT_BESTOF_HILLS_A_01")), "{k}");
        assert_eq!(crate::sndobj::audio_rng_state(), (0x64b6_748f, 0x1e7b_0689));
        // the same record drawing its last take again says the next one (22 -> 1)
        crate::sndobj::set_audio_rng_state((0x11f1_e126, 0x5939_87cc));
        let k = sp.resolve("PLAYER_NITROUS", true, None, 1, 0).unwrap();
        assert!(k.ends_with(&file("PLAYER_NITROUS_08")), "{k}");
        sp.last_take.insert(1, 22);
        let st = (0..u32::MAX).map(|i| (i, 1)).find(|&s| {
            crate::sndobj::set_audio_rng_state(s);
            crate::sndobj::audio_rand(1, 22) == 22
        });
        crate::sndobj::set_audio_rng_state(st.unwrap());
        assert!(sp.resolve("PLAYER_NITROUS", true, None, 1, 0).unwrap().ends_with(&file("PLAYER_NITROUS_01")));
        // ForceVariation (at most n)
        assert!(sp.resolve("PLAYER_NITROUS", true, None, 9, 30).unwrap().ends_with(&file("PLAYER_NITROUS_22")));
        assert!(sp.resolve("PLAYER_NITROUS", true, None, 9, 3).unwrap().ends_with(&file("PLAYER_NITROUS_03")));
    }

    /// F-73277: the stop takes the line on show and all to come
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn stop_now_ends_the_line_and_the_rest() {
        let data = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let mut a = crate::audio::Audio::new_offline(&data).expect("audio");
        let mut sp = Speech::load(&data, a.root());
        assert!(sp.say("TUTORIAL_SST_INGAME") && sp.say("TUTORIAL_SST_INGAME"));
        sp.update(&mut a, 0.1);
        assert!(sp.busy());
        sp.stop_now(&mut a);
        assert!(!sp.busy() && sp.line().is_none());
    }

    /// N-AU-2: the telephone invite / decline are `<SpecialCase>` scenarios; each caller of the challenge has all
    /// five letters of both lines and the player's line is in the player's phone bank (needs the user's data).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR (data dir) and MCLA_AUDIO_DIR (converted audio root)"]
    fn telephone_lines() {
        let data = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let root = std::env::var("MCLA_AUDIO_DIR").expect("MCLA_AUDIO_DIR");
        let mut sp = Speech::load(&data, Path::new(&root));
        for who in ["GH_03", "GH_07", "GH_13", "GH_16", "GH_22"] {
            for (sc, kind) in [("HOOKMAN_PHONE_INVITE_PLAYER", "INVITE"), ("HOOKMAN_PHONE_PLAYER_DECLINES", "DECLINE")] {
                for _ in 0..5 {
                    sp.queue.clear();
                    assert!(sp.say_as(sc, who));
                    let clips: Vec<_> = sp.queue.iter().filter_map(|s| if let Step::Clip { name, player, voice } = s { Some((name.clone(), *player, voice.clone())) } else { None }).collect();
                    assert_eq!(clips.len(), 2, "{sc} {who}");
                    for (n, p, v) in &clips {
                        assert!(n.starts_with(&format!("SIDEKICK_TELEPHONE_{kind}_{who}_")) && n.ends_with("_01"));
                        let k = sp.clip_in(n, *p, v.as_deref());
                        assert!(k.is_some(), "no clip {n} player {p}");
                    }
                }
            }
        }
    }

    /// verify_audio_b1.md batches 54 and 55: the speech manager's scenario start F-95157 run offline whole
    /// (tools/speech_start_reference.py, 4,000 random manager and scenario states, the audio's generator real): for every
    /// state but the online ones (the port is offline) - forced or not, waiting or not, the first line held or not, a
    /// scenario playing (this one held or not, another), its sender entity (none, this one, another with its rank), the
    /// ring full or empty, the sender's voice (ForceSender, kept, none) - the result, the last start (+24) and script
    /// (+28), the generator's state after, the push to the ring, the held line played, every line with the delay it gets
    /// (Delay and the WaitTimes before it), the stops, and after it the sender's voice (+2200), the sender entity (+2252),
    /// the scenario playing (+2260) and its first line's byte (+467) are the original's
    /// The two senders of the one manager (`PLAYER` rank 1, `RADIO` rank 0): a player request cuts a radio scenario
    /// whatever its Priority (the cut stops the radio), and a waiting one waits behind it - the ring is taken only once
    /// neither plays (F-95157, F-85023).
    #[test]
    fn a_player_request_cuts_the_radio_and_waits_behind_it() {
        let mut sp = Speech::load("D:/nonexistent", Path::new("D:/nonexistent"));
        let line = || vec![Step::Play { asset: "L".into(), player: true, voice: None, rec: 0, force: 0, post: None, attach: false }];
        sp.scenarios.insert("T".into(), vec![line()]);
        sp.gates.insert("T".into(), Gate { priority: 0, delay: 0.0, frequency: 0.0, probability: 100, last: -1.0e9, last_script: -1, force_sender: true });
        sp.clock = 10.0;
        // the radio plays: a request of Priority 0 still cuts it (rank 1 > 0)
        sp.radio_on = true;
        assert!(sp.request("T"));
        assert!(sp.cut_radio && !sp.radio_on && sp.sender == Some(PLAYER));
        // a waiting request behind the radio's line: into the ring, no line
        sp.cut();
        sp.cut_radio = false;
        sp.radio_on = true;
        assert!(sp.request_taunt("T", ""));
        assert!(sp.pending.len() == 1 && !sp.busy() && !sp.cut_radio);
        // not taken while the radio plays; taken once it is quiet (the update needs an audio device: the data's)
        let Some(mut a) = std::env::var("MCLA_DATA_DIR").ok().and_then(|d| crate::audio::Audio::new_offline(&d)) else { return };
        sp.update(&mut a, 0.0);
        assert_eq!(sp.pending.len(), 1);
        sp.radio_on = false;
        sp.update(&mut a, 0.0);
        assert!(sp.pending.is_empty() && sp.cur_key.as_deref() == Some("T"));
    }

    /// The one ring of 8 (F-13869 / F-85023) holds both senders' waiting requests in their order: a radio request
    /// taken out is handed to the radio (`radio_next`), the next update's is the player's; 8 waiting refuse either.
    #[test]
    fn both_senders_wait_in_the_one_ring() {
        let mut sp = Speech::load("D:/nonexistent", Path::new("D:/nonexistent"));
        let line = || vec![Step::Play { asset: "L".into(), player: true, voice: None, rec: 0, force: 0, post: None, attach: false }];
        sp.scenarios.insert("T".into(), vec![line()]);
        sp.gates.insert("T".into(), Gate { priority: 0, delay: 0.0, frequency: 0.0, probability: 100, last: -1.0e9, last_script: -1, force_sender: true });
        sp.radio_on = true;
        assert!(sp.queue_radio("REPORT_END_PURSUIT", ("4ADAM8".into(), "DISPATCH".into())));
        assert!(sp.request_taunt("T", ""));
        for _ in 0..6 {
            assert!(sp.queue_radio("R", (String::new(), String::new())));
        }
        assert!(!sp.request_taunt("T", "") && !sp.queue_radio("R", (String::new(), String::new())));
        assert_eq!(sp.waiting(), 8);
        let Some(mut a) = std::env::var("MCLA_DATA_DIR").ok().and_then(|d| crate::audio::Audio::new_offline(&d)) else { return };
        sp.radio_on = false;
        sp.update(&mut a, 0.0);
        assert_eq!(sp.radio_next.take(), Some(("REPORT_END_PURSUIT".to_string(), ("4ADAM8".to_string(), "DISPATCH".to_string()))));
        assert!(!sp.busy() && sp.waiting() == 7);
        sp.update(&mut a, 0.0);
        assert!(sp.radio_next.is_none() && sp.cur_key.as_deref() == Some("T") && sp.waiting() == 6);
    }

    #[test]
    fn speech_start_matches_the_original() {
        let mut lines = include_str!("testdata/speech_start_ref.txt").lines();
        let (mut n, mut used, mut started) = (0, 0, 0);
        let mut seen = [0u32; 6];
        while let Some(c) = lines.next() {
            let w: Vec<u32> = c.split(' ').skip(1).map(|t| t[1..].parse().unwrap()).collect();
            let (want_calls, want_after) = (lines.next().unwrap(), lines.next().unwrap());
            n += 1;
            // (online: F-24338 refuses before anything; the port has no online play)
            if w[0] == 1 {
                continue;
            }
            used += 1;
            let mut sp = Speech::load("D:/nonexistent", Path::new("D:/nonexistent"));
            let scripts: Vec<Vec<Step>> = (0..w[12] as usize)
                .map(|s| {
                    (0..4)
                        .map_while(|j| match w[23 + 4 * s + j] {
                            0 => Some(Step::Play { asset: format!("S{s}_{j}"), player: false, voice: None, rec: 0, force: 0, post: None, attach: false }),
                            2 => Some(Step::Wait(f32::from_bits(w[35 + 4 * s + j]))),
                            _ => None,
                        })
                        .collect()
                })
                .collect();
            sp.scenarios.insert("T".into(), scripts);
            sp.gates.insert(
                "T".into(),
                Gate {
                    priority: w[14] as i32,
                    delay: f32::from_bits(w[16]),
                    frequency: f32::from_bits(w[10]),
                    probability: w[11] as i32,
                    last: f32::from_bits(w[9]),
                    last_script: w[13] as i32,
                    force_sender: w[17] == 1,
                },
            );
            sp.clock = f32::from_bits(w[8]);
            sp.sender_voice = w[18] == 1;
            let me = Sender { id: 1, rank: w[19] as i32 };
            let busy = w[4] != 0;
            if busy {
                sp.queue.push_back(Step::Wait(-1.0));
                sp.cur_priority = Some(if w[4] == 1 { w[14] } else { w[15] } as i32);
                sp.cur_key = Some(if w[4] == 1 { "T" } else { "O" }.into());
                sp.held = w[5] == 1;
                sp.sender = match w[6] {
                    1 => Some(me),
                    2 => Some(Sender { id: 2, rank: w[20] as i32 }),
                    _ => None,
                };
            }
            for _ in 0..w[7] {
                sp.pending.push_back(Pending { key: "X".into(), radio: None, profile: String::new(), attach: false, sender: None, voice: None, forced: false, keep: false });
            }
            let held_before = sp.held;
            crate::sndobj::set_audio_rng_state((w[21], w[22]));
            let ok = sp.start("T", Ask { attach: true, sender: Some(me), voice: None, forced: w[2] == 1, keep: w[3] == 1, wait: w[1] == 1, profile: "" });
            let g = sp.gate("T").unwrap();
            let no_line = sp.no_line;
            let pushed = sp.pending.len() > w[7] as usize;
            let cut = busy && !matches!(sp.queue.front(), Some(Step::Wait(x)) if *x == -1.0);
            let played = held_before && !sp.held && ok && busy && !cut;
            let mut got = String::new();
            if pushed {
                got += " U";
                seen[0] += 1;
            }
            if played {
                got += " A";
                seen[1] += 1;
            }
            for _ in 0..(cut as usize + no_line as usize) {
                got += " S";
            }
            if ok && !pushed && !played {
                started += 1;
                let mut acc = 0.0f32;
                for st in &sp.queue {
                    match st {
                        Step::Wait(x) => acc += *x,
                        Step::Play { asset, .. } => {
                            let (s, j) = asset[1..].split_once('_').unwrap();
                            got += &format!(" L{s} {j} d{:08x} m", acc.to_bits());
                            acc = 0.0;
                        }
                        _ => unreachable!(),
                    }
                }
            }
            // (the original's calls: the ring push, the held line played, its stops and line builds; the logs and the
            // pool's reset left out)
            let want: String = want_calls[1..].split(' ').filter(|t| !t.is_empty()).fold((String::new(), false), |(mut o, line), t| {
                if line {
                    o += " ";
                    o += t;
                    return (o, !t.starts_with('m') && !t.starts_with('x'));
                }
                if t == "S" || t == "U" || t == "A" {
                    o += " ";
                    o += t;
                } else if t.starts_with('L') {
                    o += " ";
                    o += t;
                    return (o, true);
                }
                (o, false)
            }).0;
            assert_eq!(got, want, "case {}: {c}", n - 1);
            let (r0, r1) = crate::sndobj::audio_rng_state();
            let after = format!("A r{} x{:08x} x{:08x} x{r0:08x} x{r1:08x}", ok as u8, g.last.to_bits(), g.last_script as u32);
            assert_eq!(&want_after[..after.len()], after, "case {}: {c}", n - 1);
            // +2200, +2204, +2252, +2256, +2260 and the first line's +440 / +444 / +467
            let t: Vec<&str> = want_after.split(' ').collect();
            let word = |k: usize| u32::from_str_radix(&t[k][1..], 16).unwrap();
            assert_eq!(word(6) != 0, sp.sender_voice, "case {} +2200: {c}", n - 1);
            let snd = match sp.sender { None => 0, Some(Sender { id: 1, .. }) => 0x4028_0000, Some(_) => 0x4029_0000 };
            assert_eq!(word(8), snd, "case {} +2252: {c}", n - 1);
            let cur = match sp.cur_key.as_deref() { None => 0, Some("T") => 0x4020_0000, Some(_) => 0x4021_0000 };
            assert_eq!(word(10), cur, "case {} +2260: {c}", n - 1);
            assert_eq!(word(9) != 0, sp.busy(), "case {} +2256: {c}", n - 1);
            // (F-34685 is a stub there: the byte it clears stays)
            if word(9) != 0 && !played {
                assert_eq!(t[13], format!("b{}", sp.held as u8), "case {} +467: {c}", n - 1);
            }
            seen[2] += (busy && w[6] == 2 && !ok && !pushed) as u32;
            seen[3] += (!sp.sender_voice && !ok) as u32;
            seen[4] += (w[2] == 1 && ok) as u32;
            seen[5] += (w[7] == 8 && w[1] == 1 && busy && !ok) as u32;
        }
        eprintln!("pushed {}, played held {}, refused by rank {}, no voice {}, forced starts {}, ring full {}", seen[0], seen[1], seen[2], seen[3], seen[4], seen[5]);
        assert_eq!(n, 4000);
        assert!(used > 3700 && started > 500 && seen.iter().all(|&k| k > 10), "{used} {started} {seen:?}");
    }
}

