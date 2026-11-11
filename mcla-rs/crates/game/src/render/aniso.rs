//! The car effects' own sampler states: maximum anisotropy (state 9).
//!
//! The sampler set F-15464 copies the default sampler table that F-23692 fills every frame through F-59547
//! (state 9 = 4, or 16 while the scene is 4x). It then lays the material sampler's own (state, value) pairs over that
//! table and writes states 0 .. 11. So a sampler keeps its own anisotropy only when its effect declares state 9. Only
//! the car effects (and post effects) do: 2:1 for most car materials, 4:1 for the chrome badge's normal map, the glass
//! damage map, the skid mark and the font, and 16:1 for the tyre (WheelRubber, AmbientTire) and the brake rotor. Every
//! one of them also declares the anisotropic min filter (state 5 = 4), so the fetch's anisotropy field is the value
//! (recorded: docs/reference/lighting/sampler_aniso_evidence_2026-11-10.md).
//!
//! Each sampler variable of an effect (`shaders/cars/fxl_final/0x<joaat(name.fxc)>`) is stored as
//! `06 00 <len> name\0 <len> texname\0 <annotation count> annotations <word count> (u32 state, u32 value) pairs`,
//! little-endian. The models' texture parameters are keyed by the sampler's name hash (`DiffuseSampler` 0xf1fe2b71).
//!
//! A draw's code (`code`): bits 0 - 3 say which fetches take the material's own sampler (0 the first texture, 1 the
//! second, 2 the third - the livery, 3 the scrape map); bits 4 - 5 its anisotropy (1 = 2:1, 2 = 4:1, 3 = 16:1).
//! city.wgsl reads the mask from group 1 binding 5 (each fetch's value index) and samples through globals bindings 17 - 19.
//! MCLA_ANISO_OWN_OFF=1: every fetch through the global sampler, as before.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The anisotropy of the material's own sampler, by the code's value field (index 0 = none).
pub const OWN_MAX: [u16; 4] = [1, 2, 4, 16];

pub const SLOT_FIRST: u32 = 1;
pub const SLOT_SECOND: u32 = 2;
pub const SLOT_THIRD: u32 = 4;
pub const SLOT_SCRAPE: u32 = 8;

/// One car effect's own anisotropy: the value and the samplers (name hashes) that declare it.
#[derive(Clone, Debug, PartialEq)]
pub struct Own {
    pub max: u32,
    pub samplers: Vec<u32>,
}

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static TABLE: OnceLock<HashMap<u32, Own>> = OnceLock::new();

/// Where the data lies: the first ancestor of `dir` (a car's or the data's own folder) that holds the cars' effects.
pub fn init_from(dir: &Path) {
    if ROOT.get().is_some() {
        return;
    }
    if let Some(r) = dir.ancestors().find(|a| a.join("shaders/cars/fxl_final").is_dir()) {
        let _ = ROOT.set(r.to_path_buf());
    }
}

fn skip_annotations(d: &[u8], mut p: usize, n: usize) -> Option<usize> {
    for _ in 0..n {
        let ln = *d.get(p)? as usize;
        p += 1 + ln;
        let ty = *d.get(p)?;
        p += 1;
        match ty {
            2 => p += 1 + *d.get(p)? as usize,
            1 | 3 => p += 4,
            _ => return None,
        }
    }
    Some(p)
}

fn is_ident(s: &[u8]) -> bool {
    !s.is_empty() && (s[0].is_ascii_alphabetic() || s[0] == b'_') && s.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// Every sampler variable of an effect file with its (state, value) pairs.
pub fn samplers(d: &[u8]) -> Vec<(String, Vec<(u32, u32)>)> {
    let mut out: Vec<(String, Vec<(u32, u32)>)> = Vec::new();
    let mut i = 0;
    while i + 3 <= d.len() {
        if !(d[i] == 6 && d[i + 1] == 0 && (3..=0x40).contains(&d[i + 2])) {
            i += 1;
            continue;
        }
        let at = i;
        i += 1;
        let ln = d[at + 2] as usize;
        let p = at + 3;
        let Some(name) = d.get(p..p + ln - 1) else { continue };
        if !is_ident(name) || d.get(p + ln - 1) != Some(&0) {
            continue;
        }
        let mut p = p + ln;
        let Some(&tl) = d.get(p) else { continue };
        let tl = tl as usize;
        match d.get(p + 1..p + 1 + tl) {
            Some(t) if tl >= 2 && is_ident(&t[..tl - 1]) && t[tl - 1] == 0 => {}
            _ => continue,
        }
        p += 1 + tl;
        let Some(&na) = d.get(p) else { continue };
        let Some(q) = skip_annotations(d, p + 1, na as usize) else { continue };
        let Some(&nw) = d.get(q) else { continue };
        let nw = nw as usize;
        if nw % 2 != 0 || nw > 30 {
            continue;
        }
        let Some(words) = d.get(q + 1..q + 1 + 4 * nw) else { continue };
        let w: Vec<u32> = words.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let pairs: Vec<(u32, u32)> = w.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        if pairs.iter().any(|(k, _)| *k > 12) {
            continue;
        }
        let name = String::from_utf8_lossy(name).into_owned();
        out.retain(|(n, _)| *n != name);
        out.push((name, pairs));
    }
    out
}

/// The effect's own anisotropy: the samplers declaring state 9, and its value (one value per car effect).
pub fn own_of(d: &[u8]) -> Option<Own> {
    let mut max = 0;
    let mut samplers = Vec::new();
    for (name, pairs) in self::samplers(d) {
        if let Some((_, v)) = pairs.iter().find(|(k, _)| *k == 9) {
            if max != 0 && max != *v {
                log::warn!("aniso: effect sampler {name} declares {v}, another {max}");
            }
            max = *v;
            samplers.push(mcla_formats::hash::joaat(&name));
        }
    }
    (max != 0).then_some(Own { max, samplers })
}

fn table() -> Option<&'static HashMap<u32, Own>> {
    let root = ROOT.get()?;
    Some(TABLE.get_or_init(|| {
        let mut t = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(root.join("shaders/cars/fxl_final")) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(h) = name.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()) else { continue };
                if let Some(o) = std::fs::read(e.path()).ok().and_then(|d| own_of(&d)) {
                    t.insert(h, o);
                }
            }
        }
        log::info!("aniso: {} car effects declare their own anisotropy", t.len());
        t
    }))
}

/// The car effect of the shader named `sname` (`<sname>.fxc`), if it declares its own anisotropy.
pub fn own(sname: &str) -> Option<&'static Own> {
    if sname.is_empty() || std::env::var("MCLA_ANISO_OWN_OFF").is_ok() {
        return None;
    }
    table()?.get(&mcla_formats::hash::joaat(&format!("{sname}.fxc")))
}

/// The draw code from the effect and the sampler (name hash) behind each fetch: first, second, third texture, scrape map
/// (0 = none).
pub fn code_of(own: Option<&Own>, slots: [u32; 4]) -> u32 {
    let Some(o) = own else { return 0 };
    let v = match o.max {
        2 => 1,
        4 => 2,
        16 => 3,
        _ => return 0,
    };
    let mut mask = 0;
    for (k, h) in slots.iter().enumerate() {
        if *h != 0 && o.samplers.contains(h) {
            mask |= 1 << k;
        }
    }
    if mask == 0 {
        0
    } else {
        mask | v << 4
    }
}

/// `code_of` for the shader named `sname`.
pub fn code(sname: &str, slots: [u32; 4]) -> u32 {
    code_of(own(sname), slots)
}

/// The fetch words of a code (group 1 binding 5): for each fetch the own sampler's index (1 = 2:1, 2 = 4:1, 3 = 16:1;
/// globals bindings 17 - 19), 0 = the texture sampler.
pub fn mask_words(code: u32) -> [u32; 4] {
    let (m, v) = (code & 15, (code >> 4) & 3);
    [m & 1, (m >> 1) & 1, (m >> 2) & 1, (m >> 3) & 1].map(|b| b * v)
}

/// The name hashes of the samplers the port binds.
pub mod ph {
    pub const DIFFUSE: u32 = 0xf1fe2b71; // DiffuseSampler
    pub const NORMAL: u32 = 0x8ac11cb0; // NormalSampler
    pub const VINYL: u32 = 0xeaad91a8; // VinylSampler
    pub const SCRAPE: u32 = 0xfc8c0e7e; // ScrapeSampler
    pub const GLASS_DMG: u32 = 0x7fd1f1a5; // GlassDmgSampler
    pub const PLATE_BG: u32 = 0x50022388; // PlateBgSampler
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_words_pick_the_samplers_the_texture_group_did() {
        // (before the move to globals 17 - 19: binding 1 = OWN_MAX[(code >> 4) & 3], taken by fetch i when bit i is set)
        for v in 1..4u32 {
            for m in 1..16u32 {
                let w = mask_words(m | v << 4);
                for (i, x) in w.iter().enumerate() {
                    let old = if m >> i & 1 == 1 { Some(OWN_MAX[v as usize]) } else { None };
                    let new = if *x != 0 { Some(OWN_MAX[*x as usize]) } else { None };
                    assert_eq!(old, new, "code {:#x} fetch {}", m | v << 4, i);
                }
            }
        }
        assert_eq!(mask_words(0), [0; 4]);
    }

    #[test]
    fn code_packs_mask_and_value() {
        let o = Own { max: 16, samplers: vec![ph::DIFFUSE, ph::NORMAL] };
        assert_eq!(code_of(Some(&o), [ph::DIFFUSE, ph::NORMAL, 0, 0]), 3 | 3 << 4);
        assert_eq!(code_of(Some(&o), [ph::DIFFUSE, 0x1234, 0, 0]), 1 | 3 << 4);
        assert_eq!(code_of(Some(&o), [0x1234, 0, 0, 0]), 0);
        assert_eq!(code_of(None, [ph::DIFFUSE, 0, 0, 0]), 0);
        let p = Own { max: 2, samplers: vec![ph::VINYL, ph::SCRAPE, ph::NORMAL] };
        assert_eq!(code_of(Some(&p), [0, 0, ph::VINYL, ph::SCRAPE]), 12 | 1 << 4);
    }

    #[test]
    fn sampler_hashes_are_the_names() {
        for (h, n) in [(ph::DIFFUSE, "DiffuseSampler"), (ph::NORMAL, "NormalSampler"), (ph::VINYL, "VinylSampler"), (ph::SCRAPE, "ScrapeSampler"), (ph::GLASS_DMG, "GlassDmgSampler"), (ph::PLATE_BG, "PlateBgSampler")] {
            assert_eq!(h, mcla_formats::hash::joaat(n), "{n}");
        }
    }

    /// Every car shader the vehicles and rims use, read from the disc's effects: the values of the evidence table
    /// (docs/reference/lighting/sampler_aniso_evidence_2026-11-10.md).
    #[test]
    #[ignore]
    fn car_effects_own_anisotropy_from_the_disc() {
        let Ok(root) = std::env::var("MCLA_DATA_DIR") else { return };
        init_from(Path::new(&root));
        let want: &[(&str, Option<(u32, &[&str])>)] = &[
            ("CarPaintCustomizable", Some((2, &["VinylSampler", "ScrapeSampler", "NormalSampler"]))),
            ("BumpSpec", Some((2, &["DiffuseSampler"]))),
            ("BumpSpecAlpha", Some((2, &["DiffuseSampler"]))),
            ("GrillOpaque", Some((2, &["DiffuseSampler"]))),
            ("Underbody", Some((2, &["DiffuseSampler", "NormalSampler"]))),
            ("BumperSubframe", Some((2, &["DiffuseSampler", "NormalSampler"]))),
            ("CarbonFiber", Some((2, &["NormalSampler"]))),
            ("LicensePlate", Some((2, &["PlateBgSampler", "FontSampler"]))),
            ("RimBadge", Some((2, &["DiffuseSampler"]))),
            ("BrakeCaliper", Some((2, &["DiffuseSampler"]))),
            ("BumpedAlphaChrome", Some((4, &["NormalSampler"]))),
            ("CarGlass", Some((4, &["GlassDmgSampler"]))),
            ("SkidMark", Some((4, &["DiffuseSampler"]))),
            ("WheelRubber", Some((16, &["DiffuseSampler", "NormalSampler"]))),
            ("AmbientTire", Some((16, &["DiffuseSampler", "NormalSampler"]))),
            ("WheelBrakeRotor", Some((16, &["DiffuseSampler"]))),
            ("InteriorTrim", None),
            ("Chrome", None),
            ("BrushedMetal", None),
            ("CarLight", None),
            ("AmbientBumpSpec", None),
            ("AmbientCarPaint", None),
            ("RimMain", None),
        ];
        for (name, w) in want {
            let got = own(name).map(|o| (o.max, o.samplers.clone()));
            let w = w.map(|(m, s)| (m, s.iter().map(|n| mcla_formats::hash::joaat(n)).collect::<Vec<_>>()));
            match (&got, &w) {
                (Some((gm, gs)), Some((wm, ws))) => {
                    assert_eq!(gm, wm, "{name}");
                    let (mut a, mut b) = (gs.clone(), ws.clone());
                    a.sort();
                    b.sort();
                    assert_eq!(a, b, "{name}");
                }
                _ => assert_eq!(got.is_none(), w.is_none(), "{name}: {got:?}"),
            }
        }
    }
}
