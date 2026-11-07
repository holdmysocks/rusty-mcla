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

/// One record of a subtitle table as the original splits it (F-47413): fields end at `sep` outside quotes; a double
/// quote toggles the quoting and is dropped.
fn fields(line: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c == sep && !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// A scene's subtitles in language `lang` (0 English, 1 Spanish, 2 French, 3 German, 4 Italian, 5 Japanese) as
/// (start, end, text). The original's loader (F-99606) opens `subtitle.txt` (UTF-16, tab separated: time, English,
/// French, German, Italian, Spanish, Japanese, thought, end time) and only without it `subtitle.csv` (Latin-1, comma
/// separated, the same columns less Japanese); it keeps each record's texts in the game's language order and the
/// subtitle draw (F-37072) shows the one of the language. The rows "time correction" / "position" / "size" hold the
/// placing, not a line.
pub fn subtitle_lines(dir: &Path, lang: usize) -> Option<Vec<(f32, f32, String)>> {
    let (text, sep, cols): (String, char, &[usize]) = if let Ok(b) = std::fs::read(dir.join("subtitle.txt")) {
        let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        (String::from_utf16_lossy(&units).trim_start_matches('\u{feff}').to_string(), '\t', &[1, 5, 2, 3, 4, 6])
    } else {
        let b = std::fs::read(dir.join("subtitle.csv")).ok()?;
        (b.iter().map(|&b| b as char).collect(), ',', &[1, 5, 2, 3, 4])
    };
    let col = cols.get(lang).copied().unwrap_or(1);
    let mut subs = Vec::new();
    for line in text.lines().skip(1) {
        let f = fields(line.trim_end_matches('\r'), sep);
        if f.len() < 2 {
            continue;
        }
        let Ok(start) = f[0].trim().parse::<f32>() else { continue };
        if matches!(f[1].trim(), "" | "time correction" | "position" | "size") {
            continue;
        }
        let t = f.get(col).map_or("", |t| t.trim());
        if t.is_empty() {
            continue;
        }
        let end = f.last().and_then(|e| e.trim().parse::<f32>().ok()).filter(|e| *e > start).unwrap_or(start + 2.5);
        subs.push((start, end, t.to_string()));
    }
    Some(subs)
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
            let Some(subs) = subtitle_lines(&dir.join(&name), mcla_formats::strtbl::language()) else { continue };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The subtitle tables as the original reads them (F-99606): `subtitle.txt` first, the language's column. Every
    /// scene's English from the .txt is the .csv's line for line (so the switch of file changes no English text), the
    /// five languages give their own lines, and the Japanese column is there only in the .txt.
    #[test]
    fn subtitles_in_every_language() {
        let Ok(dir) = std::env::var("MCLA_DATA_DIR") else { return };
        let root = Path::new(&dir).join("tune/cutscene/Story");
        let Ok(rd) = std::fs::read_dir(&root) else { return };
        let mut scenes = 0;
        for e in rd.flatten().filter(|e| e.path().join("subtitle.txt").exists()) {
            let d = e.path();
            let txt = subtitle_lines(&d, 0).unwrap();
            // (the .csv alone: the same reader on a copy without the .txt)
            let tmp = std::env::temp_dir().join(format!("mcla_subs_{}", e.file_name().to_string_lossy()));
            let _ = std::fs::create_dir_all(&tmp);
            std::fs::copy(d.join("subtitle.csv"), tmp.join("subtitle.csv")).unwrap();
            let csv: Vec<Vec<(f32, f32, String)>> = (0..5).map(|l| subtitle_lines(&tmp, l).unwrap()).collect();
            let _ = std::fs::remove_dir_all(&tmp);
            assert_eq!(txt, csv[0], "{:?} English", d);
            for l in 1..5 {
                let t = subtitle_lines(&d, l).unwrap();
                assert_eq!(t, csv[l], "{:?} language {l}: .txt and .csv", d);
            }
            assert!(!subtitle_lines(&d, 5).unwrap().is_empty());
            scenes += 1;
        }
        assert_eq!(scenes, 19);
        let one = |l: usize| subtitle_lines(&root.join("CUT_STORY_BESTOF_SH_05"), l).unwrap()[0].clone();
        assert_eq!(one(0), (3.5, 5.0, "Wassup, esse?".to_string()));
        let words: Vec<String> = (1..5).map(|l| one(l).2).collect();
        assert_eq!(words, ["\u{bf}Qu\u{e9} pasa, pavo?", "H\u{e9}, la forme ?", "Was geht, Kumpel?", "Che si dice, ese?"]);
    }
}
