//! The game's localised string table (`ui/0xa4c1e8fa`, little-endian):
//! `u32 language count, u32 offset per language, u32 version, u32 key count, keys
//! {u32 length, bytes, NUL}`; every language block is `u32 count` followed by
//! `{u32 joaat(key), u16 flags, u32 font name length, font name, u32 UTF-16 length
//! (with terminator), UTF-16LE text, f32 scale x, f32 scale y, u16}` entries.
//! Language 0 is English.

use crate::hash::joaat;
use crate::{bail, Result};
use hashbrown::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The game's language for this run (0 English .. 4 Italian), set once at start from settings.toml `game.language`
/// (the launcher's choice). Every table read after that reads its block (`StringTable::parse`).
static LANGUAGE: AtomicUsize = AtomicUsize::new(0);

/// Set the run's language (the game's number, as the original's console query gives it).
pub fn set_language(lang: usize) {
    LANGUAGE.store(lang, Ordering::Relaxed);
}

/// The run's language (0 English, 1 Spanish, 2 French, 3 German, 4 Italian).
pub fn language() -> usize {
    LANGUAGE.load(Ordering::Relaxed)
}

#[derive(Default, Clone)]
pub struct StringTable {
    pub keys: Vec<String>,
    pub langs: Vec<HashMap<u32, String>>,
    /// the language block `get` reads (0 English, 1 Spanish, 2 French, 3 German, 4 Italian)
    pub lang: usize,
}

impl StringTable {
    pub fn parse(d: &[u8]) -> Result<StringTable> {
        let u32at = |o: usize| -> Result<usize> {
            if o + 4 > d.len() {
                bail!("string table truncated");
            }
            Ok(u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as usize)
        };
        let n = u32at(0)?;
        if n == 0 || n > 64 {
            bail!("not a string table");
        }
        let mut offs = Vec::new();
        for i in 0..n {
            offs.push(u32at(4 + i * 4)?);
        }
        let mut p = 4 + 4 * n + 4;
        let count = u32at(p)?;
        p += 4;
        let mut keys = Vec::with_capacity(count);
        for _ in 0..count {
            let l = u32at(p)?;
            p += 4;
            if p + l + 1 > d.len() {
                bail!("string table keys truncated");
            }
            keys.push(String::from_utf8_lossy(&d[p..p + l]).to_string());
            p += l + 1;
        }
        let mut langs = Vec::new();
        let mut seen: Vec<usize> = Vec::new();
        for &o in &offs {
            // unused languages point at the end of the file or repeat a block
            if o + 4 > d.len() || seen.contains(&o) {
                langs.push(HashMap::new());
                continue;
            }
            seen.push(o);
            let c = u32at(o)?;
            let mut q = o + 4;
            let mut m = HashMap::with_capacity(c);
            for _ in 0..c {
                if q + 10 > d.len() {
                    break;
                }
                let h = u32at(q)? as u32;
                let fl = u32at(q + 6)?;
                q += 10 + fl;
                let tl = u32at(q)?;
                q += 4;
                if q + tl * 2 + 10 > d.len() {
                    break;
                }
                let units: Vec<u16> = d[q..q + tl * 2].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
                m.insert(h, String::from_utf16_lossy(&units));
                q += tl * 2 + 10;
            }
            langs.push(m);
        }
        let mut t = StringTable { keys, langs, lang: 0 };
        t.set_lang(language());
        Ok(t)
    }

    /// Add the entries of another table (DLC and tip tables next to the main one).
    pub fn merge(&mut self, other: StringTable) {
        self.keys.extend(other.keys);
        for (i, l) in other.langs.into_iter().enumerate() {
            if i >= self.langs.len() {
                self.langs.push(l);
            } else {
                for (k, v) in l {
                    self.langs[i].entry(k).or_insert(v);
                }
            }
        }
    }

    /// The language to read: a block the table has and that holds text; anything else is
    /// English.
    pub fn set_lang(&mut self, lang: usize) {
        self.lang = if self.langs.get(lang).map_or(false, |m| !m.is_empty()) { lang } else { 0 };
    }

    /// Text of a key in the table's language, in English where that has none (keys are
    /// case-insensitive).
    pub fn get(&self, key: &str) -> Option<&str> {
        let h = joaat(key);
        self.langs.get(self.lang).and_then(|m| m.get(&h)).or_else(|| self.langs.first()?.get(&h)).map(|s| s.as_str())
    }

    pub fn get_or<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).unwrap_or(key)
    }
}
