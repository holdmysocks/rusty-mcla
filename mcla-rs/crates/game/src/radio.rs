//! The police radio: `tune/audio/PoliceScript.xml` holds the calls the dispatcher and
//! the officers make (scenarios such as `REPORT_SUSPECT_FLEES` or
//! `DISPATCH_REPORT_VIOLATION_LA_RIVER`, each with several wordings: a squelch, "any
//! unit", the violation code, the place, how to respond). The words are clips named
//! `joaat("<ASSET>_NN")` in the `police_radio` banks, one bank per voice.
//!
//! Besides fixed assets a wording has "special cases" the game fills in:
//! * `DIRECTION_PLAYER` (and `_STREETRACE`, `DIRECTION_CHANGE_*`) -> `..._NORTH` etc.
//! * `LOCATION1` / `LOCATION2` -> `LOCATION1_SUNSET_BLVD`: the street and the cross
//!   street, named as in the road network's zones ("HW_oh_Sunset Blvd.")
//! * `VEHICLE_MODEL` -> `MODEL_vp_vw_golf_83`: the car's PoliceDesc, as VEHICLE_REACQUIRE
//! * `CALL` / `REPLY` / `ACKNOWLEDGE` -> the officer's plain line (`CALL_DISPATCH`, `REPLY`), or the
//!   dispatcher's with the other party's call sign (`CALL_3ADAM5`)
//! * `PRE_SQUELCH` / `POST_SQUELCH` -> the clicks of the dispatcher's or the car's set
//! * `CALL_NEIGHBORHOOD` / `SUSPECT_LAST_SEEN` -> `CALL_NEIGHBORHOOD_HW_SS`: the district part of his street
//! * `VEHICLE_REACQUIRE` -> `VEHICLE_REACQUIRE_vp_aud_r8_08`: the car's PoliceDesc in the vehicle table
//!
//! Who speaks (docs/reference/audio/POLICE_RADIO_VOICES.md): a call has one sender and one receiver voice for all
//! its lines (F-95157 keeps them at mgr +2200 / +2204; a step of type Sender or Receiver takes that side's voice,
//! F-62806). The request names them (F-14569: 0 "DISPATCH", 1 "OFFICER", 2 the lead cruiser's call
//! sign); "DISPATCH" is the dispatcher on shift (PoliceVoices.xml TimeStart / TimeEnd), "OFFICER" one of the
//! officers at random, a call sign that officer's voice (F-69140).

use crate::audio::Audio;
use glam::Vec3;
use hashbrown::{HashMap, HashSet};
use mcla_formats::hash::joaat;
use std::collections::VecDeque;
use std::path::Path;

#[derive(Clone, Debug)]
enum Step {
    /// an asset spoken by the sender (0) or the receiver (1) of the call
    Play { asset: String, group: String, role: u8 },
    Special { name: String, role: u8 },
    /// a WaitTime: the next line waits this long after the last one's sound ended
    Wait(f32),
}

/// What a call may mention.
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// which way the suspect is heading (context +208, F-78553: his velocity on the ground, or his forward when he
    /// is almost still, made unit)
    pub heading: Vec3,
    /// his place (the matrix's +48 row)
    pub pos: Vec3,
    /// the road within 3 m of his place on the ground (F-55639), if any
    pub road: Option<usize>,
    /// his car (`vp_vw_golf_83`)
    pub car: String,
    pub hour: f32,
}

/// A zone name as the radio context keeps it (F-78553, and F-31988 for a cross street): the dots taken out, spaces
/// made `_`; `upper`: then upper case (strupr).
fn radio_name(name: &str, upper: bool) -> String {
    let s: String = name.chars().filter(|&c| c != '.').map(|c| if c == ' ' { '_' } else { c }).collect();
    if upper {
        s.to_ascii_uppercase()
    } else {
        s
    }
}

/// The two prefixes the location words keep whole (0x820C4F00 "FW", 0x820C4EFC "XT": freeways and ramps); any other
/// name loses its first 6 characters (the district part, "HW_OH_").
fn whole_name(n: &str) -> bool {
    n.as_bytes().get(..2).is_some_and(|p| p == b"FW" || p == b"XT")
}

/// The DIRECTION special case's word (F-95031): north / south when the road under the suspect (the last one he was on;
/// his heading before any) runs within 45 degrees of the z axis, by the sign of his heading's z (negative north),
/// else west / east by the sign of its x (negative west). Both tests are dot products with the unit axes, so the
/// components themselves; ties (0) go south / east.
pub fn direction(heading: Vec3, road: Option<Vec3>) -> &'static str {
    let a = road.unwrap_or(heading);
    if a.z.abs() > 0.707_106_77 {
        if heading.z < 0.0 { "NORTH" } else { "SOUTH" }
    } else if heading.x < 0.0 {
        "WEST"
    } else {
        "EAST"
    }
}

#[derive(Default)]
pub struct Radio {
    /// scenario -> its wordings
    scenarios: HashMap<String, Vec<Vec<Step>>>,
    /// clip hash -> clips of the voices that have it ("police_radio/<bank>/<file>")
    clips: HashMap<u32, Vec<String>>,
    /// PoliceVoices.xml: the OFFICER group's voices, the DISPATCH group's with their shift (start, end hour)
    officers: Vec<String>,
    dispatchers: Vec<(String, f32, f32)>,
    /// a voice -> its banks (speech.dat's POLICE_RADIO\<voice>_<part> and SC_<voice>)
    voice_banks: HashMap<String, HashSet<String>>,
    /// a bank's folder by its name hash: "0x<hash>", or the name the extraction gave it when the hash matched a name
    /// of its dictionary (DISPATCHER_2_A is in "G_RemoveItemsFromCart.xsc", DISPATCHER_01_R / _S in "U-Tk.xsc" /
    /// "U-Tk.xsd": the same joaat)
    dirs: HashMap<u32, String>,
    /// the call signs of the lead cruiser and of a second one, as the cars' sounds hand them out (`Sound::radio_units`)
    pub units: (Option<String>, Option<String>),
    /// a scenario's `frequency` in s (default 10) and `priority`; its last start on `clock` and the wording it took
    /// (F-14569 -> F-95157's gate, force 0 for every request class); the playing line's priority
    freq: HashMap<String, (f32, i32)>,
    playing: i32,
    last: HashMap<String, (f32, usize)>,
    clock: f32,
    /// the radio context of F-78553, each kept while not refreshed: +60 the name of the last road he was on (dots out,
    /// spaces to _, upper case), +188 its first 5 characters, +124 the last ramp (+60 when it starts "XT", F-21712),
    /// +252 the last road (its direction +96), +256 the junction ahead of it (F-80667), +260 the last such junction
    street: String,
    district: String,
    ramp: String,
    road: Option<usize>,
    road_dir: Option<Vec3>,
    junction: Option<usize>,
    last_junction: Option<usize>,
    /// the LOCATION2 word's street (F-31988) as of the last context update
    cross: Option<String>,
    /// clips to play, in order
    queue: VecDeque<(String, f32)>,
    wait: f32,
    rng: u32,
    /// each script step's last take (the asset record's +20 of F-70477, global per record, 0 at load): (scenario,
    /// wording, step)
    last_take: HashMap<(String, usize, usize), i32>,
    /// each vehicle table row's PoliceDesc (record +12) by its lower-case Name: the car the radio names it as
    police_desc: HashMap<String, String>,
    /// the player's speaker plays a scenario (the one speech manager's +2260 with +2252 the player's, `speech::PLAYER`,
    /// rank 1 against the radio's 0): set by the game before the radio's requests
    pub speech_on: bool,
}

fn attr<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let k = format!("{name}=\"");
    let i = s.find(&k)? + k.len();
    let e = s[i..].find('"')?;
    Some(&s[i..i + e])
}

fn bank_of(key: &str) -> &str {
    key.rsplit_once('/').map_or("", |x| x.0)
}

impl Radio {
    pub fn load(data_dir: &str, audio_root: &Path) -> Radio {
        let mut r = Radio { rng: 0x9e37_79b9, ..Default::default() };
        if let Ok(rd) = std::fs::read_dir(audio_root.join("police_radio")) {
            for e in rd.flatten() {
                let bank = e.file_name().to_string_lossy().to_string();
                let h = bank.strip_prefix("0x").and_then(|x| u32::from_str_radix(x, 16).ok()).unwrap_or_else(|| joaat(&bank));
                r.dirs.insert(h, format!("police_radio/{bank}"));
                // (a folder of an older conversion without its index: the waves are named by their hash)
                let t = std::fs::read_to_string(e.path().join("_index.tsv")).unwrap_or_else(|_| {
                    std::fs::read_dir(e.path())
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter_map(|f| f.file_name().to_string_lossy().strip_suffix(".wav").map(|n| format!("{n}\t{n}\n")))
                        .collect()
                });
                for line in t.lines() {
                    let mut c = line.split('\t');
                    let (Some(name), Some(hash)) = (c.next(), c.next()) else { continue };
                    if let Ok(h) = u32::from_str_radix(hash.trim(), 16) {
                        r.clips.entry(h).or_default().push(format!("police_radio/{bank}/{name}"));
                    }
                }
            }
        }
        // the vehicle table's PoliceDesc (the three lists, parser F-87336; the row's +12, F-55561 reads it)
        for f in ["0xe94bde63", "0xccb5daee", "0x3cb91343"] {
            let Ok(t) = std::fs::read(Path::new(data_dir).join("tune/vehicle").join(f)) else { continue };
            let mut name = None;
            for l in String::from_utf8_lossy(&t).lines().map(str::trim) {
                let mut w = l.split_whitespace();
                match (w.next(), w.next()) {
                    (Some("Vehicle"), _) => name = None,
                    (Some("Name"), Some(n)) => name = Some(n.to_ascii_lowercase()),
                    (Some("PoliceDesc"), Some(d)) => {
                        if let Some(n) = name.clone() {
                            r.police_desc.entry(n).or_insert_with(|| d.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        // the voice groups (F-27393: OFFICER, DISPATCH with optional shifts; the SQUELCH group is the clicks)
        if let Ok(t) = std::fs::read_to_string(Path::new(data_dir).join("tune/audio/PoliceVoices.xml")) {
            let mut group = String::new();
            for l in t.lines().map(str::trim) {
                if l.starts_with("<!--") {
                    continue;
                }
                if l.starts_with("<VoiceGroup") {
                    group = attr(l, "name").unwrap_or("").to_string();
                } else if l.starts_with("<Voice ") {
                    let Some(n) = attr(l, "name") else { continue };
                    match group.as_str() {
                        "OFFICER" => r.officers.push(n.to_string()),
                        "DISPATCH" => {
                            let h = |k: &str| attr(l, k).and_then(|v| v.parse::<f32>().ok());
                            r.dispatchers.push((n.to_string(), h("TimeStart").unwrap_or(0.0), h("TimeEnd").unwrap_or(24.0)));
                        }
                        _ => {}
                    }
                }
            }
        }
        // each voice's banks: speech.dat names them POLICE_RADIO\<voice>_<part> (the add-on's SC_<voice>); dispatcher
        // 2's are spelled DISPATCHER_2_*
        if let Ok(d) = std::fs::read(Path::new(data_dir).join("audio/X360/config/speech.dat")) {
            let pat = b"POLICE_RADIO\\";
            let voices: Vec<String> = r.officers.iter().cloned().chain(r.dispatchers.iter().map(|d| d.0.clone())).collect();
            let mut i = 0;
            while let Some(k) = d[i..].windows(pat.len()).position(|w| w == pat) {
                let st = i + k + pat.len();
                let end = d[st..].iter().position(|c| !(c.is_ascii_alphanumeric() || *c == b'_')).map_or(d.len(), |e| st + e);
                let name = String::from_utf8_lossy(&d[st..end]).to_string();
                let core = name.strip_prefix("SC_").unwrap_or(&name).replace("DISPATCHER_2_", "DISPATCHER_02_");
                let core = if core == "DISPATCHER_2" { "DISPATCHER_02".to_string() } else { core };
                if let Some(v) = voices.iter().filter(|v| core == **v || core.starts_with(&format!("{v}_"))).max_by_key(|v| v.len()) {
                    let bank = r.bank(joaat(&name));
                    r.voice_banks.entry(v.clone()).or_default().insert(bank);
                }
                i = end.max(st);
            }
        }
        let Ok(bytes) = std::fs::read(Path::new(data_dir).join("tune/audio/PoliceScript.xml")) else { return r };
        let t = String::from_utf8_lossy(&bytes);
        for sc in t.split("<Scenario ").skip(1) {
            let Some(name) = attr(sc, "name") else { continue };
            let body = sc.split("</Scenario>").next().unwrap_or("");
            let mut wordings = Vec::new();
            for script in body.split("<Script>").skip(1) {
                let script = script.split("</Script>").next().unwrap_or("");
                let mut steps = Vec::new();
                for line in script.lines() {
                    let l = line.trim();
                    let role = if attr(l, "type") == Some("Receiver") { 1 } else { 0 };
                    if l.starts_with("<PlayAsset") {
                        if let Some(asset) = attr(l, "name") {
                            steps.push(Step::Play { asset: asset.to_string(), group: attr(l, "voicegroup").unwrap_or("ALL").to_string(), role });
                        }
                    } else if l.starts_with("<SpecialCase") {
                        if let Some(n) = attr(l, "name") {
                            steps.push(Step::Special { name: n.to_string(), role });
                        }
                    } else if l.starts_with("<WaitTime") {
                        if let Some(t) = attr(l, "value").and_then(|v| v.parse::<f32>().ok()) {
                            steps.push(Step::Wait(t));
                        }
                    }
                }
                // (a freeway report is all SpecialCase)
                if steps.iter().any(|s| match s {
                    Step::Play { .. } => true,
                    Step::Special { name, .. } => !name.ends_with("_SQUELCH"),
                    Step::Wait(_) => false,
                }) {
                    wordings.push(steps);
                }
            }
            if !wordings.is_empty() {
                r.freq.insert(name.to_ascii_uppercase(), (attr(sc, "frequency").and_then(|f| f.parse().ok()).unwrap_or(10.0), attr(sc, "priority").and_then(|p| p.parse().ok()).unwrap_or(0)));
                r.scenarios.insert(name.to_ascii_uppercase(), wordings);
            }
        }
        log::info!("police radio: {} calls, {} clips, {} officers, {} dispatchers, {} voices with banks", r.scenarios.len(), r.clips.len(), r.officers.len(), r.dispatchers.len(), r.voice_banks.len());
        r
    }

    /// A bank's folder ("police_radio/0x<hash>" when none is found).
    fn bank(&self, h: u32) -> String {
        self.dirs.get(&h).cloned().unwrap_or_else(|| format!("police_radio/0x{h:08x}"))
    }

    pub fn loaded(&self) -> bool {
        !self.scenarios.is_empty() && !self.clips.is_empty()
    }

    pub fn busy(&self) -> bool {
        !self.queue.is_empty() || self.wait > 0.0
    }

    fn rand(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        self.rng >> 16
    }

    /// Every take of an asset, by any voice.
    fn takes(&self, asset: &str) -> Vec<String> {
        let mut out = Vec::new();
        for n in 1..=12 {
            match self.clips.get(&joaat(&format!("{asset}_{n:02}"))) {
                Some(c) => out.extend(c.iter().cloned()),
                None => break,
            }
        }
        out
    }

    /// The takes of an asset in one voice's banks (_01, _02 ... while they go on).
    fn voice_takes(&self, asset: &str, voice: &str) -> Vec<String> {
        let Some(vb) = self.voice_banks.get(voice) else { return Vec::new() };
        let mut out = Vec::new();
        for n in 1..=99 {
            match self.clips.get(&joaat(&format!("{asset}_{n:02}"))).and_then(|c| c.iter().find(|k| vb.contains(bank_of(k)))) {
                Some(k) => out.push(k.clone()),
                None => break,
            }
        }
        out
    }

    /// The dispatcher whose shift holds the hour (F-67536 via F-84173: start..end, through midnight when end < start).
    fn on_shift(&self, hour: f32) -> Option<String> {
        self.dispatchers.iter().find(|(_, s, e)| if s <= e { (*s..*e).contains(&hour) } else { hour >= *s || hour < *e }).map(|d| d.0.clone())
    }

    /// A party's voice (F-69140): "DISPATCH" the dispatcher on shift, "OFFICER" one of the officers at random,
    /// else the voice of that name (a call sign).
    fn voice_of(&mut self, who: &str, hour: f32) -> Option<String> {
        match who {
            "DISPATCH" => self.on_shift(hour),
            "OFFICER" if !self.officers.is_empty() => {
                let k = self.rand() as usize % self.officers.len();
                Some(self.officers[k].clone())
            }
            "" => None,
            n => Some(n.to_string()),
        }
    }

    /// Sender and receiver of a scenario, as its request class sets them (F-72931 .. F-19465: +16 / +20 = 0 DISPATCH,
    /// 1 OFFICER, 2 a call sign - the lead cruiser's for the sender, the request's +24 for the receiver;
    /// DISPATCH_REPORT_* go from the dispatcher to an officer).
    fn parties(&self, scenario: &str) -> (String, String) {
        let lead = self.units.0.clone().unwrap_or_default();
        if scenario.starts_with("DISPATCH_") {
            return ("DISPATCH".into(), "OFFICER".into());
        }
        if scenario.ends_with("OFFICER_TO_OFFICER") || scenario == "REPORT_PURSUIT_STATEMENT" || scenario.starts_with("REPORT_UPGRADE_") {
            return (lead, self.units.1.clone().unwrap_or_default());
        }
        (lead, "DISPATCH".into())
    }

    /// Put a call on the air, as the tactic's lead cruiser says it to the dispatcher (or as `parties` has it).
    pub fn say(&mut self, scenario: &str, ctx: &Ctx) -> bool {
        let parties = self.parties(&scenario.to_ascii_uppercase());
        self.say_line(scenario, parties, ctx)
    }

    /// The same with the request's queue flag (a report's +40, F-14569 -> F-95157): while a line plays (the radio's or
    /// the player's speaker's) the request goes to the speech manager's one ring (+28 + 268 i, write +2172, read +2176,
    /// count +2180: true; refused when 8 wait, the player's waiting requests counted), ahead of every gate; else as `say`.
    /// F-85023 takes it out once nothing plays (`speech::Speech::radio_next`) and it is asked again through the whole
    /// gate (`say_from`).
    pub fn say_queued(&mut self, sp: &mut crate::speech::Speech, scenario: &str, ctx: &Ctx) -> bool {
        let parties = self.parties(&scenario.to_ascii_uppercase());
        if self.busy() || sp.busy() {
            return sp.queue_radio(scenario, parties);
        }
        self.say_line(scenario, parties, ctx)
    }

    /// A line of a tactic (F-26639), its sender and receiver named.
    pub fn say_from(&mut self, scenario: &str, sender: String, receiver: String, ctx: &Ctx) -> bool {
        self.say_line(scenario, (sender, receiver), ctx)
    }

    #[cfg(test)]
    fn first_ramp(&self) -> String {
        "XT_HW_GRAND_AVE".to_string()
    }

    /// The radio context's update F-78553 (each frame of a pursuit in the original): the road within 3 m (kept when
    /// none), its name, district and ramp, and the junction ahead of it (F-80667: the first two junctions of the road's
    /// connection list, the first unless it lies behind his heading); then the LOCATION2 street of that state.
    pub fn note(&mut self, ctx: &Ctx, nav: Option<&mcla_formats::rage::navsys::NavSys>) {
        use mcla_formats::rage::navsys::NavNode;
        let Some(nav) = nav else { return };
        let flat = Vec3::new(ctx.pos.x, 0.0, ctx.pos.z);
        // F-11078 / F-78553: the dot of the heading with (the point - his place on the ground), not negative
        let ahead = |p: Vec3, h: Vec3| {
            let d = p - flat;
            crate::sndobj::fmadds(h.x, d.x, crate::sndobj::fmadds(h.y, d.y, h.z * d.z)) >= 0.0
        };
        if let Some(r) = ctx.road {
            self.road = Some(r);
            self.road_dir = nav.zones.get(r).map(|z| z.dir);
            let name = nav.zones.get(r).map_or("", |z| z.name.as_str());
            if !name.is_empty() {
                self.street = radio_name(&name.chars().take(63).collect::<String>(), true);
                self.district = self.street.chars().take(5).collect();
                if self.street.starts_with("XT") {
                    self.ramp = self.street.clone();
                }
            }
            let mut js = nav.road_next.get(r).into_iter().flatten().filter_map(|n| if let NavNode::Junction(j) = n { Some(*j) } else { None });
            let a = js.next();
            let b = a.and_then(|a| js.find(|&j| j != a));
            match (a, b) {
                (Some(a), None) => self.junction = Some(a),
                (Some(a), Some(b)) => {
                    let j = if nav.junction_mid.get(a).is_some_and(|&p| ahead(p, ctx.heading)) { a } else { b };
                    self.junction = Some(j);
                    self.last_junction = Some(j);
                }
                _ => self.junction = None,
            }
        }
        // F-31988: the roads at the junction ahead (else at the road, else at the last junction), in the file's order;
        // the first with a name that is not his own street: compared after the district part when neither is a
        // freeway / ramp, else whole and only if it lies ahead (F-11078)
        let node = self.junction.map(NavNode::Junction).or(self.road.map(NavNode::Road)).or(self.last_junction.map(NavNode::Junction));
        let list = match node {
            Some(NavNode::Junction(j)) => nav.junction_next.get(j),
            Some(NavNode::Road(r)) => nav.road_next.get(r),
            None => None,
        };
        let own_whole = whole_name(&self.street);
        self.cross = None;
        for n in list.into_iter().flatten() {
            let NavNode::Road(c) = *n else { continue };
            let Some(z) = nav.zones.get(c) else { continue };
            if z.name.is_empty() {
                continue;
            }
            let buf = radio_name(&z.name, true);
            let whole = whole_name(&buf);
            let take = if !own_whole && !whole {
                buf.get(6..).unwrap_or("") != self.street.get(6..).unwrap_or("")
            } else {
                buf != self.street && ahead(z.mid, ctx.heading)
            };
            if take {
                let raw = if whole { z.name.as_str() } else { z.name.get(6..).unwrap_or("") };
                self.cross = Some(radio_name(raw, false));
                break;
            }
        }
    }

    /// F-95157 with force 0: refused until the scenario's Frequency has run out; over a playing line only with a higher
    /// priority, which stops it; the start time is kept even if no line of the wording then resolves.
    fn say_line(&mut self, scenario: &str, (sender, receiver): (String, String), ctx: &Ctx) -> bool {
        let key = scenario.to_ascii_uppercase();
        let Some(n) = self.scenarios.get(&key).map(|w| w.len()) else { return false };
        let (frequency, priority) = self.freq.get(&key).copied().unwrap_or((10.0, 0));
        let before = self.last.get(&key).copied();
        if before.is_some_and(|l| self.clock < l.0 + frequency) {
            return false;
        }
        // (the player's speaker playing: F-84173 says nothing while +2252 is the player's and a scenario plays, and the
        // start F-95157 refuses the radio's request, rank 0 not above the player's 1)
        if self.speech_on {
            return false;
        }
        if self.busy() {
            if priority <= self.playing {
                return false;
            }
            self.stop();
        }
        let mut pick = self.rand() as usize % n;
        if n > 1 && before.is_some_and(|l| l.1 == pick) {
            pick = (pick + 1) % n;
        }
        self.last.insert(key.clone(), (self.clock, pick));
        let steps = self.scenarios[&key][pick].clone();
        let Some(sv) = self.voice_of(&sender, ctx.hour) else { return false };
        let rv = self.voice_of(&receiver, ctx.hour);
        let voice: [Option<String>; 2] = [Some(sv), rv];
        let disp: Vec<String> = self.dispatchers.iter().map(|d| d.0.clone()).collect();
        let is_dispatch = |v: &Option<String>| v.as_ref().is_some_and(|v| disp.contains(v));
        let dir = direction(ctx.heading, self.road_dir);
        // (clip, role) in order; squelches are filled in afterwards
        // (clip, role, pre squelch, step, WaitTime: role 2)
        let mut line: Vec<(Option<String>, u8, bool, usize, f32)> = Vec::new();
        let mut said: Vec<String> = Vec::new();
        for (si, st) in steps.iter().enumerate() {
            let (asset, role): (String, u8) = match st {
                Step::Play { asset, role, .. } => (asset.clone(), *role),
                Step::Wait(t) => {
                    line.push((None, 2, false, si, *t));
                    continue;
                }
                Step::Special { name, role } => {
                    let r = *role as usize;
                    // the dispatcher names the other party's call sign; an officer calls dispatch (F-28061)
                    let other = voice[1 - r].clone().filter(|_| is_dispatch(&voice[r]));
                    let with = |base: &str, plain: &str| match &other {
                        Some(o) => format!("{base}_{o}"),
                        None => plain.to_string(),
                    };
                    let a = match name.as_str() {
                        "PRE_SQUELCH" | "POST_SQUELCH" => {
                            line.push((None, *role, name == "PRE_SQUELCH", si, 0.0));
                            continue;
                        }
                        "FREEWAY_ENTER" | "FREEWAY_EXIT" if !self.ramp.is_empty() => format!("{name}_{}", self.ramp),
                        "DIRECTION_PLAYER" | "DIRECTION_STREETRACE" | "DIRECTION_CHANGE_PLAYER" | "DIRECTION_CHANGE_STREETRACE" => format!("{name}_{dir}"),
                        // F-84504: the street whole when a freeway / ramp, else after its district part
                        "LOCATION1" if !self.street.is_empty() => format!("LOCATION1_{}", if whole_name(&self.street) { &self.street[..] } else { self.street.get(6..).unwrap_or("") }),
                        "LOCATION2" if self.cross.is_some() => format!("LOCATION2_{}", self.cross.as_deref().unwrap_or("")),
                        // F-73009 / F-26958 (CALL_NEIGHBORHOOD_%s / SUSPECT_LAST_SEEN_%s): the context's district (+188), none
                        // when it is empty
                        "CALL_NEIGHBORHOOD" | "SUSPECT_LAST_SEEN" if !self.district.is_empty() => format!("{name}_{}", self.district),
                        // F-26276 ("%s_%s"): the word and the chased car's PoliceDesc (VEHICLE_REACQUIRE_vp_aud_r8_08); the
                        // resolver F-28061 passes "MODEL" for VEHICLE_MODEL (MODEL_vp_aud_r8_08)
                        "VEHICLE_REACQUIRE" | "VEHICLE_MODEL" if self.police_desc.contains_key(&ctx.car.to_ascii_lowercase()) => {
                            format!("{}_{}", if name == "VEHICLE_MODEL" { "MODEL" } else { name }, self.police_desc[&ctx.car.to_ascii_lowercase()])
                        }
                        "CALL" => with("CALL", "CALL_DISPATCH"),
                        "REPLY" => with("REPLY", "REPLY"),
                        "ACKNOWLEDGE" => with("ACKNOWLEDGE", "ACKNOWLEDGE"),
                        "ACKNOWLEDGE_STANDBY" => "ACKNOWLEDGE_STANDBY".to_string(),
                        _ => continue,
                    };
                    (a, *role)
                }
            };
            // the side's own voice speaks the line; a word that voice does not have stays silent (the line does not
            // resolve), no other voice takes it
            let Some(v) = voice[role as usize].as_ref() else { continue };
            let takes = self.voice_takes(&asset, v);
            if takes.is_empty() {
                continue;
            }
            let k = takes[self.take(&key, pick, si, takes.len())].clone();
            said.push(format!("{v}:{asset}"));
            line.push((Some(k), role, false, si, 0.0));
        }
        if !line.iter().any(|l| l.0.is_some()) {
            return false;
        }
        self.playing = priority;
        for (clip, role, pre, si, t) in line {
            match clip {
                Some(k) => self.queue.push_back((k, 0.7)),
                // (an empty name: a WaitTime of t s)
                None if role == 2 => self.queue.push_back((String::new(), t)),
                None => {
                    // the click of the speaker's set
                    let bank = self.bank(joaat(if is_dispatch(&voice[role as usize]) { "POLICE_RADIO_SQUELCH_DISPATCH" } else { "POLICE_RADIO_SQUELCH_OFFICER" }));
                    let takes: Vec<String> = self.takes(if pre { "PRE_SQUELCH" } else { "POST_SQUELCH" }).into_iter().filter(|k| bank_of(k) == bank).collect();
                    if !takes.is_empty() {
                        let k = takes[self.take(&key, pick, si, takes.len())].clone();
                        self.queue.push_back((k, 0.5));
                    }
                }
            }
        }
        log::info!("police radio: {} (wording {} of {}): {}", key, pick + 1, n, said.join(" / "));
        true
    }

    /// A step's take (F-70477, as the speech's lines): v = rand(1, n) on the audio's shared state; when v is the step
    /// record's last take and n > 1, one on (past n: 1); v is its new last take. The index into the takes (v - 1).
    fn take(&mut self, scenario: &str, wording: usize, step: usize, n: usize) -> usize {
        let n = n as i32;
        let rec = (scenario.to_string(), wording, step);
        let v = bump_take(crate::sndobj::audio_rand(1, n), self.last_take.get(&rec).copied().unwrap_or(0), n);
        self.last_take.insert(rec, v);
        v as usize - 1
    }

    pub fn stop(&mut self) {
        self.queue.clear();
    }

    /// A player request cut the radio's scenario (F-95157 -> F-73277, `speech::Speech::cut_radio`): the lines to come and
    /// the wait of the one playing.
    pub fn cut(&mut self) {
        self.queue.clear();
        self.wait = 0.0;
    }

    pub fn update(&mut self, a: &mut Audio, dt: f32) {
        self.clock += dt;
        self.wait -= dt;
        if self.queue.is_empty() {
        }
        while self.wait <= 0.0 {
            let Some((key, vol)) = self.queue.pop_front() else {
                self.wait = 0.0;
                return;
            };
            if key.is_empty() {
                self.wait = vol;
                continue;
            }
            // (D59: the length from the wave's chunk headers; its samples are read by the take's slot play)
            let dur = a.clip_secs(&key);
            if dur > 0.0 {
                // (the police voices are Speech sounds of category SPEECH_POLICE; the port's own level without them)
                if !a.obj_clip(&key, "SPEECH_POLICE", None) {
                    a.play(&key, vol);
                }
                // the next line starts when this one's sound has ended (F-78934's end callback, then the next line's
                // WaitTime counted from there, F-35459: +392 >= +400) - one after the other, the squelches too, no gap
                // of their own. Recorded on the original (h1, hookmen2): a squelch ended at update 455 and the word
                // started in it; the receiver's squelch started 9 updates after the end with +392 0.503 >= 0.5
                self.wait = dur;
            }
        }
    }
}

/// F-70477's bump: the drawn take v (1..n) when it is not the record's last, else the next (past n: 1); n = 1 keeps it.
fn bump_take(v: i32, last: i32, n: i32) -> i32 {
    if v == last && n > 1 {
        if v + 1 > n {
            1
        } else {
            v + 1
        }
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    /// The radio's take picks recorded on the running original (r1, one pursuit, 79 picks of F-70477 with 8 bumps):
    /// drawn v, n, the step record's last take, the take said. The record is the script step (the same LOCATION2 step
    /// keeps its last take over different streets; another wording of the scenario starts at 0).
    #[test]
    fn radio_takes_bump_as_recorded() {
        let mut bumps = 0;
        for l in include_str!("testdata/radio_take_ref.txt").lines() {
            let f: Vec<i32> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            assert_eq!(super::bump_take(f[0], f[2], f[1]), f[3], "{l}");
            bumps += (f[0] != f[3]) as i32;
        }
        assert_eq!(bumps, 8);
    }

    /// The player's speaker playing (rank 1 above the radio's 0): a report is refused; a queued one goes to the speech
    /// manager's ring.
    #[test]
    fn the_player_speaker_holds_the_radio_off() {
        let mut r = super::Radio::default();
        let mut sp = crate::speech::Speech::default();
        let ctx = super::Ctx::default();
        r.speech_on = true;
        r.scenarios.insert("REPORT_PULL_SUSPECT_OVER".into(), vec![vec![]]);
        assert!(!r.say("REPORT_PULL_SUSPECT_OVER", &ctx));
        // (the speech plays: queued in the one ring)
        r.wait = 1.0;
        assert!(r.say_queued(&mut sp, "REPORT_PULL_SUSPECT_OVER", &ctx));
        assert_eq!(sp.waiting(), 1);
    }

    /// F-95157's queue flag: queued while a line plays, 8 at most in the one ring (the player's requests count).
    #[test]
    fn queued_requests_share_the_ring_of_8() {
        let mut r = super::Radio::default();
        let mut sp = crate::speech::Speech::default();
        let ctx = super::Ctx::default();
        r.wait = 1.0;
        for _ in 0..3 {
            assert!(sp.queue_radio("X", ("A".into(), "B".into())));
        }
        for k in 0..6 {
            assert_eq!(r.say_queued(&mut sp, "REPORT_PULL_SUSPECT_OVER", &ctx), k < 5);
        }
        assert_eq!(sp.waiting(), 8);
    }

    /// F-95031 against the original's code run offline on 4,000 cases (tools/radio_direction_reference.py): headings
    /// and road directions round the circle, on and off the 45 degree line, signed zeros, with and without a road.
    #[test]
    fn direction_matches_the_original() {
        let f = |h: &str| f32::from_bits(u32::from_str_radix(h, 16).unwrap());
        let mut n = 0;
        for l in include_str!("testdata/radio_direction_ref.txt").lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            let h = Vec3::new(f(w[0]), f(w[1]), f(w[2]));
            let r = (w[3] == "1").then(|| Vec3::new(f(w[4]), f(w[5]), f(w[6])));
            assert_eq!(direction(h, r), w[8], "{l}");
            n += 1;
        }
        assert_eq!(n, 4000);
    }

    use super::*;

    /// The radio context (F-78553) on every road of the city, heading along it both ways: how many LOCATION1 /
    /// LOCATION2 words it builds and how many of them some voice has (the words are the original's clip names, so a
    /// word no voice has is a name the transform got wrong - or one the game has no clip for either).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn location_words_have_clips() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let r0 = Radio::load(&dir, a.root());
        let res = crate::assets::load_resource_file(&std::path::Path::new(&dir).join("resources/city/sc/0x9960fcd8.r66")).unwrap();
        let nav = mcla_formats::rage::navsys::NavSys::parse(&res).unwrap();
        let has = |w: &str| r0.officers.iter().chain(r0.dispatchers.iter().map(|d| &d.0)).any(|v| !r0.voice_takes(w, v).is_empty());
        let (mut l1, mut l1_ok, mut l2, mut l2_ok) = (0, 0, 0, 0);
        let mut missing: Vec<String> = Vec::new();
        let mut all_missing = std::collections::BTreeSet::new();
        for (ri, z) in nav.zones.iter().enumerate() {
            for sgn in [1.0f32, -1.0] {
                let mut r = Radio::default();
                let ctx = Ctx { heading: Vec3::new(z.dir.x, 0.0, z.dir.z).normalize_or_zero() * sgn, pos: z.mid, road: Some(ri), ..Default::default() };
                r.note(&ctx, Some(&nav));
                if !r.street.is_empty() {
                    let w = format!("LOCATION1_{}", if whole_name(&r.street) { &r.street[..] } else { r.street.get(6..).unwrap_or("") });
                    l1 += 1;
                    if has(&w) { l1_ok += 1 } else { all_missing.insert(w.clone()); if missing.len() < 12 { missing.push(w) } }
                }
                if let Some(c) = &r.cross {
                    let w = format!("LOCATION2_{c}");
                    l2 += 1;
                    if has(&w) { l2_ok += 1 } else { all_missing.insert(w.clone()); if missing.len() < 24 { missing.push(w) } }
                }
            }
        }
        println!("LOCATION1 {l1_ok} / {l1} with a clip, LOCATION2 {l2_ok} / {l2}; e.g. without: {missing:?}");
        assert!(l1 > 0 && l2 > 0);
        // SC_MissingPoliceAssets.xml (the +196 list F-97364 loads; F-31988 drops a listed word, the resolver then takes the next step): no voice has any of them
        let xml = std::fs::read_to_string(std::path::Path::new(&dir).join("tune/audio/SC_MissingPoliceAssets.xml")).unwrap();
        let listed: Vec<&str> = xml.split("name=\"").skip(1).filter_map(|t| t.split('"').next()).collect();
        assert_eq!(listed.len(), 28);
        let up = |w: &str| w.to_ascii_uppercase();
        let with_clip: Vec<&&str> = listed.iter().filter(|w| has(&up(w)) || has(w)).collect();
        println!("listed with a clip: {with_clip:?}; unlisted without: {:?}", all_missing.iter().filter(|w| !listed.iter().any(|l| l.eq_ignore_ascii_case(w))).collect::<Vec<_>>());
        assert!(with_clip.is_empty());
    }

    /// A freeway report is all SpecialCase and names the last ramp; its Frequency of 15 s refuses it until it has run out.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn a_freeway_report_names_the_ramp() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut r = Radio::load(&dir, a.root());
        let ctx = Ctx { hour: 12.0, ..Default::default() };
        let sign = || ("4ADAM8".to_string(), "DISPATCH".to_string());
        // (with no line that resolves the start time is kept, F-95157)
        assert!(!r.say_from("REPORT_FREEWAY_ENTER", sign().0, sign().1, &ctx), "no ramp yet");
        r.clock += 15.0;
        r.ramp = r.first_ramp();
        assert!(r.say_from("REPORT_FREEWAY_ENTER", sign().0, sign().1, &ctx));
        assert!(!r.say_from("REPORT_FREEWAY_EXIT", sign().0, sign().1, &ctx), "the same priority does not cut it");
        r.stop();
        assert!(!r.say_from("REPORT_FREEWAY_ENTER", sign().0, sign().1, &ctx), "Frequency");
        r.clock += 15.0;
        assert!(r.say_from("REPORT_FREEWAY_ENTER", sign().0, sign().1, &ctx));
    }

    /// A call keeps its two voices for all its lines (GitHub #17: the description changed voice word by word): every
    /// clip a call queues is in the sender's or the receiver's banks or a squelch set, the sender of a REPORT_* call is
    /// the lead cruiser, its receiver the dispatcher on shift; DISPATCH_REPORT_* go the other way.
    /// The recorded call of h1 (hookmen2, the original running): REPORT_SUSPECT_DESTRUCTION is squelch, word, squelch,
    /// then the receiver's squelch 0.5 s after the end (its WaitTime), word, squelch - one after the other; the port
    /// queues the WaitTime and starts each clip when the one before ends.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn a_call_waits_its_wait_time() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut r = Radio::load(&dir, a.root());
        r.units = (Some("3ADAM5".into()), None);
        assert!(r.say_from("REPORT_SUSPECT_DESTRUCTION", "3ADAM5".into(), "DISPATCH".into(), &Ctx { hour: 12.0, ..Default::default() }));
        let q: Vec<(bool, f32)> = r.queue.iter().map(|(k, t)| (k.is_empty(), *t)).collect();
        assert_eq!(q.len(), 7, "{:?}", r.queue);
        assert_eq!(q[3], (true, 0.5));
        assert!(q.iter().enumerate().all(|(i, e)| e.0 == (i == 3)));
    }

    /// The recorded re-sighting (hookmen2's v2, the original running with the player in the R8): REPORT_SUSPECT_REACQUIRE
    /// from 4ADAM8 says VEHICLE_REACQUIRE_vp_aud_r8_08 (take 1 of 1) between the officer's squelches, the dispatcher
    /// answers ACKNOWLEDGE after 1.5 s. A car whose PoliceDesc is another's is named as that one (the pack's CHP Charger
    /// as vp_dge_charger_06).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn a_reacquire_names_the_police_desc() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut r = Radio::load(&dir, a.root());
        r.units = (Some("4ADAM8".into()), None);
        let generic: HashSet<u32> = (1..=20).map(|t| joaat(&format!("VEHICLE_REACQUIRE_GENERIC_{t:02}"))).collect();
        // (a clip is named by its index name, or by its hash in an older conversion)
        let hash = |w: &str| { let w = w.trim_end_matches(".wav"); u32::from_str_radix(w, 16).ok().filter(|_| w.len() == 8).unwrap_or_else(|| joaat(w)) };
        for (car, desc) in [("vp_aud_r8_08", "vp_aud_r8_08"), ("vp_se_dge_chp_06", "vp_dge_charger_06")] {
            let want: HashSet<u32> = (1..=20).map(|t| joaat(&format!("VEHICLE_REACQUIRE_{desc}_{t:02}"))).collect();
            let mut named = 0;
            for _ in 0..60 {
                r.queue.clear();
                r.stop();
                r.clock += 60.0;
                let ctx = Ctx { car: car.into(), hour: 12.0, ..Default::default() };
                assert!(r.say_from("REPORT_SUSPECT_REACQUIRE", "4ADAM8".into(), "DISPATCH".into(), &ctx), "{car}");
                let q: Vec<(String, f32)> = r.queue.iter().cloned().collect();
                assert!(q.len() >= 7, "{car}: {q:?}");
                let word = hash(q[1].0.rsplit('/').next().unwrap());
                if want.contains(&word) {
                    // (the special case's script: WaitTime 1.5)
                    named += 1;
                    let banks = r.voice_banks.iter().find(|(v, _)| v.ends_with("4ADAM8")).map(|(_, b)| b.clone()).unwrap_or_default();
                    assert!(banks.contains(q[1].0.rsplit_once('/').unwrap().0), "{car}: the officer's banks {banks:?}: {q:?}");
                    assert_eq!((q[3].0.is_empty(), q[3].1), (true, 1.5), "{car}");
                } else {
                    assert!(generic.contains(&word), "{car}: {q:?}");
                }
            }
            assert!(named > 0, "{car}: never named");
        }
    }

    /// VEHICLE_MODEL is F-26276 with the word "MODEL" (the resolver F-28061's case 18): MODEL_ and the chased car's
    /// PoliceDesc - the pack's CHP Charger is named MODEL_vp_dge_charger_06 (its own name has no clip on the disc).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn a_model_names_the_police_desc() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut r = Radio::load(&dir, a.root());
        r.units = (Some("4ADAM8".into()), None);
        let hash = |w: &str| { let w = w.trim_end_matches(".wav"); u32::from_str_radix(w, 16).ok().filter(|_| w.len() == 8).unwrap_or_else(|| joaat(w)) };
        for (car, desc) in [("vp_aud_r8_08", "vp_aud_r8_08"), ("vp_se_dge_chp_06", "vp_dge_charger_06"), ("vpd_aud_s5_08", "vp_aud_r8_08")] {
            let want: HashSet<u32> = (1..=20).map(|t| joaat(&format!("MODEL_{desc}_{t:02}"))).collect();
            let own: HashSet<u32> = (1..=20).map(|t| joaat(&format!("MODEL_{car}_{t:02}"))).collect();
            let mut named = 0;
            for _ in 0..30 {
                r.queue.clear();
                r.stop();
                r.clock += 60.0;
                let ctx = Ctx { car: car.into(), hour: 12.0, ..Default::default() };
                assert!(r.say_from("DISPATCH_REPORT_VIOLATION_3RD_ST_PROMENADE", "DISPATCH".into(), "4ADAM8".into(), &ctx), "{car}");
                let words: Vec<u32> = r.queue.iter().filter(|q| !q.0.is_empty()).map(|q| hash(q.0.rsplit('/').next().unwrap())).collect();
                named += words.iter().filter(|w| want.contains(*w)).count();
                if car != desc {
                    assert!(!words.iter().any(|w| own.contains(w)), "{car}: its own name");
                }
            }
            assert!(named > 0, "{car}: the model never said");
        }
    }

    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn a_call_keeps_its_voices() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let mut r = Radio::load(&dir, a.root());
        assert_eq!((r.officers.len(), r.dispatchers.len()), (6, 2));
        assert_eq!(r.voice_banks.len(), 8, "{:?}", r.voice_banks.keys().collect::<Vec<_>>());
        assert_eq!(r.on_shift(12.0).as_deref(), Some("DISPATCHER_02"));
        assert_eq!(r.on_shift(23.0).as_deref(), Some("DISPATCHER_01"));
        assert_eq!(r.on_shift(3.0).as_deref(), Some("DISPATCHER_01"));
        r.units = (Some("4ADAM8".into()), Some("2ADAM6".into()));
        let squelch: HashSet<String> = ["POLICE_RADIO_SQUELCH_DISPATCH", "POLICE_RADIO_SQUELCH_OFFICER"].iter().map(|n| format!("police_radio/0x{:08x}", joaat(n))).collect();
        (r.street, r.cross) = ("HW_OH_SUNSET_BLVD".into(), Some("Vine_St".into()));
        let ctx = Ctx { heading: Vec3::X, car: "vp_vw_golf_83".into(), hour: 12.0, ..Default::default() };
        let mut calls = 0;
        let names: Vec<String> = r.scenarios.keys().cloned().collect();
        for (k, name) in names.iter().enumerate() {
            for hour in [3.0, 12.0] {
                r.queue.clear();
                let c = Ctx { hour, ..ctx.clone() };
                if !r.say(name, &c) {
                    continue;
                }
                calls += 1;
                let (s, rcv) = r.parties(name);
                let sv = r.voice_of(&s, hour).unwrap();
                let rv = if rcv == "OFFICER" { None } else { r.voice_of(&rcv, hour) };
                if name.starts_with("REPORT_") {
                    // (officer to officer: the pursuit statement, the upgrades and *_OFFICER_TO_OFFICER)
                    let o2o = name.ends_with("OFFICER_TO_OFFICER") || name == "REPORT_PURSUIT_STATEMENT" || name.starts_with("REPORT_UPGRADE_");
                    let want = if o2o { Some("2ADAM6".to_string()) } else { r.on_shift(hour) };
                    assert_eq!((sv.as_str(), rv.clone()), ("4ADAM8", want), "{name}");
                }
                let mut voices: HashSet<&str> = HashSet::new();
                for (clip, _) in r.queue.iter().filter(|q| !q.0.is_empty()) {
                    let b = bank_of(clip).to_string();
                    if squelch.contains(&b) {
                        continue;
                    }
                    let v = r.voice_banks.iter().find(|(_, bs)| bs.contains(&b)).map(|(v, _)| v.as_str()).unwrap_or("?");
                    voices.insert(v);
                }
                assert!(voices.len() <= 2, "{name} ({k}) at {hour} h: {voices:?}");
                if rv.is_some() || s != "DISPATCH" {
                    assert!(voices.iter().all(|v| *v == sv || Some(*v) == rv.as_deref() || rcv == "OFFICER" || s == "OFFICER"), "{name}: {voices:?} not {sv} / {rv:?}");
                }
            }
        }
        assert!(calls > 100, "{calls} calls");
    }
}
