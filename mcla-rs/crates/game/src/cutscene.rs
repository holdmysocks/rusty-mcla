//! Story scenes: the dialogue of the game's cutscenes (`tune/cutscene/Story/<NAME>`:
//! the audio stream name, the set dressing and `subtitle.csv` with time codes), played
//! with letterbox and subtitles while the camera circles the car. The character
//! animation of the original scenes (cine animation packs driven by compiled scripts)
//! is not reproduced.

use std::path::Path;

#[derive(Clone, Debug)]
pub struct SceneDef {
    /// "CUT_STORY_RACE_GH_06"
    pub name: String,
    /// (start, end, text)
    pub subs: Vec<(f32, f32, String)>,
    pub length: f32,
    /// audio stream relative to the converted audio root, if it exists
    pub stream: Option<String>,
}

pub struct Playing {
    pub def: SceneDef,
    pub t: f32,
    /// the clock at the last update (the music events passed since)
    last: f32,
}

/// The story scenes' music events, from their generated scripts' event records (CineScript_Events_LaunchEvent; the
/// cine event dispatcher F-91263): racer_enablemusic (38: the music manager's suspend F-64824) and musicplay (42:
/// Play(0), F-33831) at the scene's seconds. No other scene has one.
pub const STORY_MUSIC: &[(&str, &[(u8, f32)])] = &[("CUT_STORY_RACE_GH_06", &[(38, 57.6333), (42, 63.1)])];

#[derive(Default)]
pub struct Cutscenes {
    pub scenes: Vec<SceneDef>,
    pub playing: Option<Playing>,
    /// the music events the playing scene passed, taken by the game for the music manager
    pub music: Vec<u8>,
    pending: std::collections::VecDeque<usize>,
}

/// One CSV record: fields split on commas outside quotes.
fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

impl Cutscenes {
    pub fn load(data_dir: &str, audio_root: Option<&Path>) -> Cutscenes {
        let mut c = Cutscenes::default();
        let dir = Path::new(data_dir).join("tune/cutscene/Story");
        let mut names: Vec<String> = std::fs::read_dir(&dir).map(|d| d.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default();
        names.sort();
        for name in names {
            if !name.to_ascii_uppercase().starts_with("CUT_") {
                continue;
            }
            // the table is Latin-1
            let Ok(bytes) = std::fs::read(dir.join(&name).join("subtitle.csv")) else { continue };
            let text: String = bytes.iter().map(|&b| b as char).collect();
            let mut subs = Vec::new();
            for line in text.lines().skip(1) {
                let f = csv_fields(line);
                if f.len() < 2 {
                    continue;
                }
                let Ok(start) = f[0].trim().parse::<f32>() else { continue };
                let t = f[1].trim();
                if t.is_empty() || matches!(t, "time correction" | "position" | "size") {
                    continue;
                }
                let end = f.last().and_then(|e| e.trim().parse::<f32>().ok()).filter(|e| *e > start).unwrap_or(start + 2.5);
                subs.push((start, end, t.to_string()));
            }
            let mut length = subs.last().map_or(4.0, |s| s.1 + 1.5);
            let mut stream = None;
            if let Some(root) = audio_root {
                let rel = format!("0xf615164c/{}.mp3", name.to_ascii_lowercase());
                if root.join(&rel).exists() {
                    // "index hash name samples rate" per channel
                    if let Ok(t) = std::fs::read_to_string(root.join(format!("{rel}.txt"))) {
                        let p: Vec<&str> = t.lines().next().unwrap_or("").split('\t').collect();
                        if let (Some(n), Some(r)) = (p.get(3).and_then(|x| x.parse::<f32>().ok()), p.get(4).and_then(|x| x.parse::<f32>().ok())) {
                            if r > 0.0 {
                                length = length.max(n / r);
                            }
                        }
                    }
                    stream = Some(rel);
                }
            }
            c.scenes.push(SceneDef { name: name.to_ascii_uppercase(), subs, length, stream });
        }
        log::info!("cutscenes: {} story scenes, {} with their dialogue stream", c.scenes.len(), c.scenes.iter().filter(|s| s.stream.is_some()).count());
        c
    }

    /// The scene that introduces a mission, if the game has one.
    /// Nothing is playing and nothing waits to.
    pub fn idle(&self) -> bool {
        self.playing.is_none() && self.pending.is_empty()
    }

    pub fn for_mission(&self, id: &str, night: bool) -> Option<usize> {
        let key = match id {
            "RACES_IN_BEACHES" => "RACES_IN_BEACH",
            "GARAGE_HOLLYWOOD" => "HOLLYWOOD_GARAGE_INTRO",
            "REP_ON_HW_STRIP" => "REP_ON_STRIP_GARAGE",
            x => x,
        };
        let find = |n: String| self.scenes.iter().position(|s| s.name == n);
        if night {
            if let Some(i) = find(format!("CUT_STORY_{key}_NIGHT")) {
                return Some(i);
            }
        }
        find(format!("CUT_STORY_{key}"))
    }

    pub fn by_name(&self, name: &str) -> Option<usize> {
        self.scenes.iter().position(|s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn queue(&mut self, i: usize) {
        if !self.pending.contains(&i) {
            self.pending.push_back(i);
        }
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    /// Start a queued scene; returns the audio stream to play.
    pub fn start_pending(&mut self) -> Option<Option<String>> {
        if self.playing.is_some() {
            return None;
        }
        let i = self.pending.pop_front()?;
        let def = self.scenes.get(i)?.clone();
        log::info!("cutscene: {} ({:.0} s, {} lines)", def.name, def.length, def.subs.len());
        let stream = def.stream.clone();
        self.playing = Some(Playing { def, t: 0.0, last: 0.0 });
        Some(stream)
    }

    /// Advance; returns false when the scene has just ended.
    pub fn update(&mut self, dt: f32, skip: bool) -> bool {
        let Some(p) = self.playing.as_mut() else { return true };
        p.t += dt;
        // (an event the clock passed since the last update, a jump to the bookmark too)
        for (_, evs) in STORY_MUSIC.iter().filter(|m| m.0.eq_ignore_ascii_case(&p.def.name)) {
            self.music.extend(evs.iter().filter(|e| p.last < e.1 && e.1 <= p.t).map(|e| e.0));
        }
        p.last = p.t;
        if skip || p.t >= p.def.length {
            self.playing = None;
            return false;
        }
        true
    }

    pub fn subtitle(&self) -> Option<&str> {
        let p = self.playing.as_ref()?;
        p.def.subs.iter().find(|s| p.t >= s.0 && p.t < s.1).map(|s| s.2.as_str())
    }
}
