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
    /// `voice`: the profile whose banks say it (GH_01 ...), None - any
    Play { asset: String, player: bool, voice: Option<String> },
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
/// (+24, -FLT_MAX at load) and its last script (+28, -1 at load).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gate {
    pub priority: i32,
    pub delay: f32,
    pub frequency: f32,
    pub probability: i32,
    pub last: f32,
    pub last_script: i32,
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
    /// the shared random numbers (F-62480: lo, carry); (approx.) seeded here, the original's stream position is not modelled
    mwc: (u32, u32),
    /// the Priority (+12) of the scenario `request` started, while it plays
    cur_priority: Option<i32>,
}

fn attr<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let k = format!("{name}=\"");
    let i = s.find(&k)? + k.len();
    let e = s[i..].find('"')?;
    Some(&s[i..i + e])
}

impl Speech {
    pub fn load(data_dir: &str, audio_root: &Path) -> Speech {
        let mut sp = Speech { rng: 0x2545_f491, mwc: (0x2545_f491, 1), ..Default::default() };
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
                    steps.push(Step::Play { asset: asset.to_string(), player, voice: None });
                } else if l.starts_with("<SpecialCase") {
                    // (the career radio's squelch clicks around a line: RADIO_PRE_SQUELCH_01..03 / RADIO_POST_SQUELCH_01..03,
                    // bank 0x52b9ce47 - a take of three, like any asset)
                    if let Some(n) = attr(l, "name").filter(|n| *n == "RADIO_PRE_SQUELCH" || *n == "RADIO_POST_SQUELCH") {
                        steps.push(Step::Play { asset: n.to_string(), player: false, voice: None });
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

    /// F-62480(lo, hi): state x 0x5CDCFAA7 + carry; lo + its low 31 bits mod (hi - lo + 1). Always draws.
    fn rand(&mut self, lo: i32, hi: i32) -> i32 {
        let t = self.mwc.0 as u64 * 0x5CDC_FAA7 + self.mwc.1 as u64;
        self.mwc = (t as u32, (t >> 32) as u32);
        lo + ((self.mwc.0 & 0x7FFF_FFFF) % (hi - lo + 1).max(1) as u32) as i32
    }

    /// A speech request as F-30989 / F-62728 make it (`"PLAYER_NITROUS"`, voice group "PLAYER", not forced): the start
    /// F-95157 refuses it while the clock is below its last start + Frequency, then unless rand(0, 99) < Probability (the
    /// draw only when the clock passed); a scenario playing is cut only by a higher Priority (+12). On a start: the last
    /// start = the clock, the script = rand(0, count - 1), one on (wrapping) when it equals the last one (so a scenario of
    /// two scripts alternates), the Delay before the first line. Recorded (work/slipstream/audio3/cap/n0.txt): the gate
    /// draw 27 < 30 passes, the script draw 0 next, a request 66.3 s after the start refused with no draw, one at 120.7 s
    /// drawn 36 and refused (the last start kept).
    pub fn request(&mut self, scenario: &str) -> bool {
        let key = scenario.to_ascii_uppercase();
        let (Some(g), Some(count)) = (self.gates.get(&key).copied(), self.scenarios.get(&key).map(|s| s.len())) else { return false };
        if self.clock < g.last + g.frequency || self.rand(0, 99) >= g.probability || count == 0 {
            return false;
        }
        if self.busy() {
            if self.cur_priority.is_some_and(|p| g.priority <= p) {
                return false;
            }
            self.queue.clear();
            self.wait = 0.0;
        }
        let mut pick = self.rand(0, count as i32 - 1);
        if pick == g.last_script {
            pick += 1;
            if pick == count as i32 {
                pick = 0;
            }
        }
        if let Some(e) = self.gates.get_mut(&key) {
            e.last = self.clock;
            e.last_script = pick;
        }
        let steps = self.scenarios[&key][pick as usize].clone();
        let steps = self.phone_steps(steps, "");
        if g.delay > 0.0 {
            self.queue.push_back(Step::Wait(g.delay));
        }
        self.queue.extend(steps);
        self.cur_priority = Some(g.priority);
        log::debug!("speech: {} started at {:.3} s (script {})", key, self.clock, pick);
        true
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
            Step::Play { asset, player: false, .. } => Step::Play { asset, player: false, voice: voice.clone() },
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

    fn resolve(&mut self, asset: &str, player: bool, voice: Option<&str>) -> Option<String> {
        // numbered takes of the same line
        let mut takes: Vec<&Vec<String>> = Vec::new();
        for n in 1..=8 {
            match self.clips.get(&joaat(&format!("{asset}_{n:02}"))) {
                Some(c) => takes.push(c),
                None => break,
            }
        }
        // (in the speaker's own voice: the takes his banks hold, one of them)
        if let Some(vb) = voice.and_then(|v| self.voice_banks.get(v)) {
            let bank_of = |k: &String| k.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default();
            let mine: Vec<String> = takes.iter().filter_map(|t| t.iter().find(|k| vb.contains(&bank_of(k))).cloned()).collect();
            if !mine.is_empty() {
                self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
                let k = mine[(self.rng >> 16) as usize % mine.len()].clone();
                log::info!("speech: {} in {}'s voice: {}", asset, voice.unwrap_or(""), k);
                return Some(k);
            }
        }
        if takes.is_empty() {
            return None;
        }
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let take = takes[(self.rng >> 16) as usize % takes.len()];
        let is_player = |k: &String| k.rsplit_once('/').map_or(false, |x| self.player_banks.contains(x.0));
        take.iter().find(|k| is_player(k) == player).or(take.first()).cloned()
    }

    pub fn update(&mut self, a: &mut Audio, dt: f32) {
        self.clock += dt;
        if !self.busy() {
            self.cur_priority = None;
        }
        self.wait -= dt;
        if let Some(l) = self.line.as_mut() {
            l.1 -= dt;
            if l.1 <= 0.0 {
                self.line = None;
            }
        }
        self.step(a);
        // the scenario's dynamic mixer while it plays (F-70477 / F-38026)
        let on = self.busy() && !self.mixer.is_empty();
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
                Step::Wait(t) => self.wait = t,
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
                        if !a.obj_clip(&key, "SPEECH_CAREER", None) {
                            a.play(&key, 1.0);
                        }
                        self.wait = dur;
                        self.line = Some((player, dur));
                    }
                }
                Step::Play { asset, player, voice } => {
                    let Some(key) = self.resolve(&asset, player, voice.as_deref()) else { continue };
                    let dur = a.clip(&key).map_or(0.0, |c| c.data.len() as f32 / c.rate.max(1) as f32);
                    if dur > 0.0 {
                        log::debug!("speech: {} -> {} ({:.1} s)", asset, key, dur);
                        if !a.obj_clip(&key, "SPEECH_CAREER", None) {
                            a.play(&key, 1.0);
                        }
                        self.wait = dur + 0.15;
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
        sp.mwc = (0x5058_86d5, 0x04f6_b060);
        assert!(sp.request("PLAYER_NITROUS"));
        assert_eq!(sp.mwc, (0x4f0f_6058, 0x1c16_9b0a));
        let g = sp.gate("PLAYER_NITROUS").unwrap();
        assert_eq!((g.last, g.last_script), (279.983, 0));
        sp.stop();
        sp.wait = 0.0;
        // f=639: clock 346.263 < 279.983 + 120: refused before the draw
        sp.clock = 346.263;
        let st = sp.mwc;
        assert!(!sp.request("PLAYER_NITROUS"));
        assert_eq!(sp.mwc, st);
        // f=852: clock 400.699, state 52c60400 2a421f84 -> draw 36: refused, the last start kept
        sp.clock = 400.699;
        sp.mwc = (0x52c6_0400, 0x2a42_1f84);
        assert!(!sp.request("PLAYER_NITROUS"));
        assert_eq!(sp.gate("PLAYER_NITROUS").unwrap().last, 279.983);
        // s0.txt f=467, the slipstream burn: clock 115.061, state cb06e3f0 3347957c -> draw 68: refused
        let mut s0 = Speech::load(&data, Path::new("."));
        s0.clock = 115.061;
        s0.mwc = (0xcb06_e3f0, 0x3347_957c);
        assert!(!s0.request("PLAYER_NITROUS"));
        assert_eq!(s0.gate("PLAYER_NITROUS").unwrap().last, -f32::MAX);
        // n1.txt (the gate hook printing the draw): f=360 clock 63.031, state 541d8400 24164598 -> 72 (state 463f6198
        // 1e833729): refused; f=573 clock 119.423, state 0154510f 3d3d6924 -> 17, script draw 1 (state 9457687f 2e2ccf41)
        let mut n1 = Speech::load(&data, Path::new("."));
        n1.clock = 63.031;
        n1.mwc = (0x541d_8400, 0x2416_4598);
        assert!(!n1.request("PLAYER_NITROUS"));
        assert_eq!(n1.mwc, (0x463f_6198, 0x1e83_3729));
        n1.clock = 119.423;
        n1.mwc = (0x0154_510f, 0x3d3d_6924);
        assert!(n1.request("PLAYER_NITROUS"));
        assert_eq!((n1.mwc, n1.gate("PLAYER_NITROUS").unwrap().last_script), ((0x9457_687f, 0x2e2c_cf41), 1));
        // two scripts alternate: after script 0 the next start takes 1 whatever the draw, and back
        for seed in 0..64u32 {
            sp.stop();
            sp.wait = 0.0;
            let before = sp.gate("PLAYER_NITROUS").unwrap().last_script;
            sp.clock += 1000.0;
            sp.mwc = (seed.wrapping_mul(0x9E37_79B9), seed);
            // (a draw below 30 passes the gate: try draws until one does)
            while !sp.request("PLAYER_NITROUS") {
                sp.clock += 1000.0;
            }
            assert_eq!(sp.gate("PLAYER_NITROUS").unwrap().last_script, 1 - before);
        }
        // a line plays: another of the same Priority is refused
        sp.clock += 1000.0;
        sp.mwc = (0x5058_86d5, 0x04f6_b060);
        assert!(sp.busy() && !sp.request("PLAYER_IN_AIR"));
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
}
