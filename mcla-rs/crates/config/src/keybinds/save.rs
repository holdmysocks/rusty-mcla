//! The `keyboard` member of the port's save file (`mcla_save.json`, see docs/SAVE.md), read and written for the
//! launcher's Controls tab without the game's `SaveData` type.
//!
//! Writing edits only that member: the new text is spliced into the file in place of the old member (or added after
//! the last one, or the member is removed when the bindings are the defaults - as the game writes it), so every other
//! byte of the file stays as it was. Before anything is written the result is parsed again and compared, as a
//! `serde_json::Value`, with the old file with only `keyboard` changed; a difference refuses the write. The file is
//! replaced atomically (temp file + rename), and the first write of a [`SaveWriter`] first copies the save to
//! `<save>.bak`. A file that is not a JSON object is refused and left alone.

use super::Bindings;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The member's name in the save.
pub const MEMBER: &str = "keyboard";

#[derive(Debug)]
pub enum SaveError {
    /// no save at that path (first run: the game has not written one yet)
    NotFound(PathBuf),
    Io(PathBuf, std::io::Error),
    /// not JSON, or not a JSON object at the top: nothing was changed
    NotJson(PathBuf, String),
    /// the spliced text did not read back as the intended file: nothing was written (a bug, never expected)
    Verify(PathBuf),
}

impl SaveError {
    /// A stable word for the launcher's texts.
    pub fn code(&self) -> &'static str {
        match self {
            SaveError::NotFound(_) => "no_save",
            SaveError::Io(..) => "io",
            SaveError::NotJson(..) => "not_json",
            SaveError::Verify(_) => "verify",
        }
    }
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::NotFound(p) => write!(f, "no save file at {}", p.display()),
            SaveError::Io(p, e) => write!(f, "{}: {e}", p.display()),
            SaveError::NotJson(p, e) => write!(f, "{} is not a valid save (not a JSON object: {e}); it was not changed", p.display()),
            SaveError::Verify(p) => write!(f, "{}: the edited save did not read back as intended; nothing was written", p.display()),
        }
    }
}

impl std::error::Error for SaveError {}

/// What [`read`] found.
#[derive(Debug)]
pub struct Read {
    pub bindings: Bindings,
    /// the member exists (false = the defaults, as the game writes a save of a player who never rebound)
    pub present: bool,
    /// the member exists but has a shape the game would not read; the defaults are shown
    pub warning: Option<String>,
}

fn load(path: &Path) -> Result<(String, Value), SaveError> {
    let bytes = match crate::read_bytes(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(SaveError::NotFound(path.to_path_buf())),
        Err(e) => return Err(SaveError::Io(path.to_path_buf(), e)),
    };
    let text = String::from_utf8(bytes).map_err(|e| SaveError::NotJson(path.to_path_buf(), e.to_string()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| SaveError::NotJson(path.to_path_buf(), e.to_string()))?;
    if !v.is_object() {
        return Err(SaveError::NotJson(path.to_path_buf(), "the top level is not an object".into()));
    }
    Ok((text, v))
}

/// The bindings in a save file.
pub fn read(path: &Path) -> Result<Read, SaveError> {
    let (_, v) = load(path)?;
    Ok(from_value(&v))
}

fn from_value(v: &Value) -> Read {
    match v.get(MEMBER) {
        None | Some(Value::Null) => Read { bindings: Bindings::default(), present: false, warning: None },
        Some(m) => match serde_json::from_value::<BTreeMap<String, Vec<String>>>(m.clone()) {
            Ok(f) => Read { bindings: Bindings::from_save(Some(&f)), present: true, warning: None },
            Err(e) => Read { bindings: Bindings::default(), present: true, warning: Some(format!("the save's keyboard member is not readable ({e}); showing the defaults")) },
        },
    }
}

/// Writes bindings into save files; the first write of each file it touches copies that file to `<save>.bak` first
/// (one backup per launcher session, so the copy is the save as it was before this session's edits).
#[derive(Default)]
pub struct SaveWriter {
    backed_up: Vec<PathBuf>,
}

/// What [`SaveWriter::write`] did.
#[derive(Debug, PartialEq, Eq)]
pub struct Written {
    /// the file's bytes changed (false: the bindings were already what the file said)
    pub changed: bool,
    /// the backup made by this write, if it was the first one of the session
    pub backup: Option<PathBuf>,
}

impl SaveWriter {
    pub fn new() -> SaveWriter {
        SaveWriter::default()
    }

    /// Put `b` into the save's `keyboard` member (or remove the member when `b` is the defaults). The save must exist.
    pub fn write(&mut self, path: &Path, b: &Bindings) -> Result<Written, SaveError> {
        let (text, v) = load(path)?;
        let field = b.to_save();
        let new_text = splice(&text, field.as_ref()).map_err(|e| SaveError::NotJson(path.to_path_buf(), e))?;
        // (the result must be the old file with only `keyboard` changed)
        let mut want = v;
        let obj = want.as_object_mut().expect("checked in load");
        match &field {
            Some(f) => {
                obj.insert(MEMBER.to_string(), serde_json::to_value(f).expect("a map of strings"));
            }
            None => {
                obj.remove(MEMBER);
            }
        }
        match serde_json::from_str::<Value>(&new_text) {
            Ok(got) if got == want => {}
            _ => return Err(SaveError::Verify(path.to_path_buf())),
        }
        if new_text == text {
            return Ok(Written { changed: false, backup: None });
        }
        let mut backup = None;
        if !self.backed_up.iter().any(|p| p == path) {
            let bak = bak_path(path);
            std::fs::copy(path, &bak).map_err(|e| SaveError::Io(bak.clone(), e))?;
            self.backed_up.push(path.to_path_buf());
            backup = Some(bak);
        }
        crate::atomic_write(path, new_text.as_bytes()).map_err(|e| SaveError::Io(path.to_path_buf(), e))?;
        Ok(Written { changed: true, backup })
    }
}

/// `<save>.bak` beside the save.
pub fn bak_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "mcla_save.json".into());
    name.push(".bak");
    path.with_file_name(name)
}

/// A member of the top-level object: the key's opening quote, the value's first and one-past-last byte.
struct Member {
    key: String,
    key_start: usize,
    value_start: usize,
    value_end: usize,
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// The end of the string whose opening quote is at `i` (one past the closing quote).
fn string_end(b: &[u8], mut i: usize) -> Result<usize, String> {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    Err("an unterminated string".into())
}

/// The top-level object: (index of `{`, index of the closing `}`, its members in order). The text was already parsed
/// by serde_json, so this only has to find the places, not judge the syntax.
fn members(text: &str) -> Result<(usize, usize, Vec<Member>), String> {
    let b = text.as_bytes();
    let mut i = skip_ws(b, if text.starts_with('\u{feff}') { 3 } else { 0 });
    if b.get(i) != Some(&b'{') {
        return Err("the top level is not an object".into());
    }
    let open = i;
    i += 1;
    let mut out = Vec::new();
    loop {
        i = skip_ws(b, i);
        match b.get(i) {
            Some(b'}') => return Ok((open, i, out)),
            Some(b',') => {
                i += 1;
                continue;
            }
            Some(b'"') => {}
            _ => return Err(format!("unexpected text at byte {i}")),
        }
        let key_start = i;
        let key_end = string_end(b, i)?;
        let key: String = serde_json::from_str(&text[key_start..key_end]).map_err(|e| e.to_string())?;
        i = skip_ws(b, key_end);
        if b.get(i) != Some(&b':') {
            return Err(format!("no ':' after a key at byte {i}"));
        }
        let value_start = skip_ws(b, i + 1);
        // (the value runs to the ',' or '}' at depth 0)
        let mut depth = 0i32;
        let mut j = value_start;
        let mut last = value_start;
        while j < b.len() {
            match b[j] {
                b'"' => {
                    j = string_end(b, j)?;
                    last = j;
                    continue;
                }
                b'{' | b'[' => depth += 1,
                b'}' | b']' if depth > 0 => depth -= 1,
                b',' | b'}' if depth == 0 => break,
                _ => {}
            }
            if !matches!(b[j], b' ' | b'\t' | b'\n' | b'\r') {
                last = j + 1;
            }
            j += 1;
        }
        out.push(Member { key, key_start, value_start, value_end: last });
        i = j;
    }
}

/// The member's value as the game's pretty printer writes it at this indentation (`indent` = the members' leading
/// blanks; empty = the file is compact).
fn value_text(f: &BTreeMap<String, Vec<String>>, indent: &str) -> String {
    if indent.is_empty() {
        return serde_json::to_string(f).expect("a map of strings");
    }
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    serde::Serialize::serialize(f, &mut ser).expect("a map of strings");
    let s = String::from_utf8(buf).expect("JSON is UTF-8");
    s.replace('\n', &format!("\n{indent}"))
}

/// The save's text with the `keyboard` member set to `field` (None = removed).
fn splice(text: &str, field: Option<&BTreeMap<String, Vec<String>>>) -> Result<String, String> {
    let (open, close, ms) = members(text)?;
    // (the members' indentation: the blanks before the first key on its line; none = compact)
    let indent = ms.first().map_or("  ".to_string(), |m| {
        let line = &text[..m.key_start];
        match line.rfind('\n') {
            Some(nl) => text[nl + 1..m.key_start].to_string(),
            None => String::new(),
        }
    });
    let sep = if indent.is_empty() { "" } else { " " };
    let pos = ms.iter().position(|m| m.key == MEMBER);
    let mut out = String::with_capacity(text.len() + 2048);
    match (pos, field) {
        (Some(p), Some(f)) => {
            let m = &ms[p];
            out.push_str(&text[..m.value_start]);
            out.push_str(&value_text(f, &indent));
            out.push_str(&text[m.value_end..]);
        }
        (Some(p), None) => {
            // (with the comma that separates it from its neighbour)
            let (cut_from, cut_to) = if p > 0 {
                (ms[p - 1].value_end, ms[p].value_end)
            } else if ms.len() > 1 {
                (ms[0].key_start, ms[1].key_start)
            } else {
                (ms[0].key_start, ms[0].value_end)
            };
            out.push_str(&text[..cut_from]);
            out.push_str(&text[cut_to..]);
            if ms.len() == 1 {
                // (the only member: an empty object, as serde writes it)
                let (o, c, _) = members(&out)?;
                return Ok(format!("{}{{}}{}", &out[..o], &out[c + 1..]));
            }
        }
        (None, Some(f)) => {
            let member = format!("\"{MEMBER}\":{sep}{}", value_text(f, &indent));
            let nl = if indent.is_empty() { String::new() } else { format!("\n{indent}") };
            match ms.last() {
                Some(last) => {
                    out.push_str(&text[..last.value_end]);
                    out.push(',');
                    out.push_str(&nl);
                    out.push_str(&member);
                    out.push_str(&text[last.value_end..]);
                }
                None => {
                    out.push_str(&text[..open + 1]);
                    out.push_str(&nl);
                    out.push_str(&member);
                    out.push_str(if indent.is_empty() { "" } else { "\n" });
                    out.push_str(&text[close..]);
                }
            }
        }
        (None, None) => out.push_str(text),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcla-config-kbd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rebound() -> Bindings {
        let mut b = Bindings::default();
        let k = |c: &str| Key::from_code(c).unwrap();
        assert!(b.assign(NITRO, 0, k("ControlLeft")) && b.assign(SHIFT_UP, 0, k("KeyP")) && b.assign(SHIFT_DOWN, 0, k("Semicolon")) && b.assign(HORN, 0, k("KeyE")));
        b
    }

    /// Everything but `keyboard`, as values.
    fn others(v: &Value) -> Value {
        let mut v = v.clone();
        v.as_object_mut().unwrap().remove(MEMBER);
        v
    }

    /// The pretty text of a save as the game writes it (serde's pretty printer, 2 blanks).
    fn game_pretty(v: &Value) -> String {
        serde_json::to_string_pretty(v).unwrap()
    }

    #[test]
    fn splice_keeps_every_other_byte() {
        let d = tmp("splice");
        let p = d.join("mcla_save.json");
        let base = "{\n  \"car\": \"vp_aud_r8_08\",\n  \"money\": 1341764,\n  \"x\": 0.1,\n  \"s\": \"a,}\\\"]{\",\n  \"mc4\": {\n    \"a\": [1, 2, {\"b\": \"}\"}]\n  }\n}";
        std::fs::write(&p, base).unwrap();
        let orig: Value = serde_json::from_str(base).unwrap();
        let mut w = SaveWriter::new();
        // (add: appended after the last member, formatted as the game would)
        let r = w.write(&p, &rebound()).unwrap();
        assert_eq!(r, Written { changed: true, backup: Some(bak_path(&p)) });
        assert_eq!(std::fs::read_to_string(bak_path(&p)).unwrap(), base);
        let t1 = std::fs::read_to_string(&p).unwrap();
        assert!(t1.starts_with(&base[..base.len() - 2]), "the old bytes come first unchanged");
        let v1: Value = serde_json::from_str(&t1).unwrap();
        assert_eq!(others(&v1), orig);
        assert_eq!(read(&p).unwrap().bindings, rebound());
        // (the member's text is what the game's pretty printer writes at that place)
        let game = game_pretty(&serde_json::json!({ "keyboard": v1["keyboard"] }));
        assert!(t1.ends_with(&game[1..]), "{t1}");
        // (replace: a second write; no second backup)
        let mut b2 = rebound();
        b2.clear(GPS_MAP, 0);
        assert_eq!(w.write(&p, &b2).unwrap(), Written { changed: true, backup: None });
        assert_eq!(read(&p).unwrap().bindings, b2);
        assert_eq!(std::fs::read_to_string(bak_path(&p)).unwrap(), base);
        // (same again: nothing to change)
        assert_eq!(w.write(&p, &b2).unwrap(), Written { changed: false, backup: None });
        // (back to the defaults: the member goes and the file is byte for byte the original)
        assert_eq!(w.write(&p, &Bindings::default()).unwrap().changed, true);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), base);
        let r = read(&p).unwrap();
        assert!(!r.present && r.bindings == Bindings::default());
        // (no temp file left)
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2);
    }

    #[test]
    fn member_in_the_middle_first_only_and_compact() {
        let b = rebound();
        let f = b.to_save().unwrap();
        for text in [
            "{\n  \"keyboard\": {\"GAS\": [\"W\"]},\n  \"car\": \"x\"\n}",
            "{\n  \"car\": \"x\",\n  \"keyboard\": {},\n  \"money\": 3\n}",
            "{\n  \"keyboard\": {\"GAS\": []}\n}",
            "{\"car\":\"x\",\"money\":3}",
            "{}",
            "\u{feff}{\n  \"car\": \"x\"\n}\n",
        ] {
            let orig: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap();
            for field in [Some(&f), None] {
                let out = splice(text, field).unwrap();
                let v: Value = serde_json::from_str(out.trim_start_matches('\u{feff}')).unwrap();
                assert_eq!(others(&v), others(&orig), "{text:?} -> {out:?}");
                match field {
                    Some(f) => assert_eq!(v[MEMBER], serde_json::to_value(f).unwrap()),
                    None => assert!(v.get(MEMBER).is_none(), "{out:?}"),
                }
            }
        }
        // (compact stays compact)
        assert!(!splice("{\"car\":\"x\"}", Some(&f)).unwrap().contains('\n'));
    }

    #[test]
    fn refusals() {
        let d = tmp("refuse");
        let mut w = SaveWriter::new();
        let missing = d.join("none.json");
        assert_eq!(w.write(&missing, &rebound()).unwrap_err().code(), "no_save");
        assert_eq!(read(&missing).unwrap_err().code(), "no_save");
        assert!(!missing.exists());
        for (name, body) in [("bad.json", "{ \"car\": "), ("arr.json", "[1, 2]"), ("bin.json", "\u{0}\u{1}")] {
            let p = d.join(name);
            std::fs::write(&p, body).unwrap();
            let e = w.write(&p, &rebound()).unwrap_err();
            assert_eq!(e.code(), "not_json", "{name}: {e}");
            assert_eq!(std::fs::read_to_string(&p).unwrap(), body);
            assert!(!bak_path(&p).exists());
        }
        // (a member the game would not read: shown as the defaults, with a warning; writing replaces it)
        let p = d.join("odd.json");
        std::fs::write(&p, "{\n  \"keyboard\": 7\n}").unwrap();
        let r = read(&p).unwrap();
        assert!(r.present && r.warning.is_some() && r.bindings == Bindings::default());
        w.write(&p, &rebound()).unwrap();
        assert_eq!(read(&p).unwrap().bindings, rebound());
    }

    /// Against a copy of the user's real save (never the save itself): every member but `keyboard` is unchanged, as a
    /// value and as bytes. Uses `%LOCALAPPDATA%\RustyMCLA\saves\mcla_save.json`, else the old copy in the data folder,
    /// else `MCLA_TEST_SAVE`; skipped (with a line) when none exists.
    #[test]
    fn real_save_copy_keeps_every_other_field() {
        let candidates: Vec<PathBuf> = [
            std::env::var_os("MCLA_TEST_SAVE").map(PathBuf::from),
            std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("RustyMCLA").join("saves").join("mcla_save.json")),
            Some(PathBuf::from("D:/MCLA/extracted/cache/mcla_save.json")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let Some(real) = candidates.iter().find(|p| p.is_file()) else {
            eprintln!("real_save_copy_keeps_every_other_field: no save found, skipped");
            return;
        };
        let real_hash = std::fs::read(real).unwrap();
        let d = tmp("real");
        let p = d.join("mcla_save.json");
        std::fs::copy(real, &p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        let orig: Value = serde_json::from_str(&text).unwrap();
        let mut w = SaveWriter::new();
        let b = rebound();
        w.write(&p, &b).unwrap();
        let t1 = std::fs::read_to_string(&p).unwrap();
        let v1: Value = serde_json::from_str(&t1).unwrap();
        assert_eq!(others(&v1), others(&orig));
        for (k, val) in orig.as_object().unwrap() {
            if k != MEMBER {
                assert_eq!(&v1[k], val, "member {k}");
            }
        }
        assert_eq!(read(&p).unwrap().bindings, b);
        // (bytes: the part before the member is the original's)
        let (_, _, ms) = members(&t1).unwrap();
        let kb = ms.iter().find(|m| m.key == MEMBER).unwrap();
        if orig.get(MEMBER).is_none() {
            let cut = ms[ms.len() - 2].value_end;
            assert_eq!(&t1[..cut], &text[..cut]);
            assert_eq!(t1[kb.value_end..].trim(), "}");
        }
        // (and back to what it was)
        let back = Bindings::from_save(orig.get(MEMBER).and_then(|m| serde_json::from_value(m.clone()).ok()).as_ref());
        w.write(&p, &back).unwrap();
        if orig.get(MEMBER).is_none() {
            assert_eq!(std::fs::read_to_string(&p).unwrap(), text, "restoring the defaults gives the original bytes");
        }
        assert_eq!(std::fs::read(bak_path(&p)).unwrap(), real_hash);
        // (the real save itself was only read)
        assert_eq!(std::fs::read(real).unwrap(), real_hash);
        eprintln!("real_save_copy_keeps_every_other_field: {} ({} bytes, {} members) ok", real.display(), text.len(), orig.as_object().unwrap().len());
        let _ = std::fs::remove_dir_all(&d);
    }
}
