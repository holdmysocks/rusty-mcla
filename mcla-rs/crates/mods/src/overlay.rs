//! Plain-name file overlay: a path in the data tree -> a mod's file, without writing into the data folder.
//!
//! The extracted data names a file either by its own name (`tune/vehicle/wheel_models.list`,
//! `resources/vehicle/vp_aud_r8_08/`) or by the joaat of its name (`tune/garage/0x{joaat("vp_aud_r8_08.garage")}`,
//! the game's own archive naming; `mcla_formats::hash::joaat`: ASCII lower-case, `\` = `/`). Resource files whose
//! names the extraction recovered only as a hash carry the compiled extension after the hash of the source name:
//! `.r9` = joaat(`<stem>.xtd`), `.r102` = joaat(`<stem>.xdr`) (the port's own lookups: game.rs, hudmap.rs). A mod file
//! is written with its plain name; it is registered under every form a reader may ask for.

use mcla_formats::hash::joaat;

/// Normalise a relative data path: forward slashes, lower case, no leading `./` or `/`, no empty parts.
pub fn norm(rel: &str) -> String {
    rel.replace('\\', "/").split('/').filter(|p| !p.is_empty() && *p != ".").collect::<Vec<_>>().join("/").to_ascii_lowercase()
}

fn split(rel: &str) -> (&str, &str) {
    match rel.rfind('/') {
        Some(i) => (&rel[..i + 1], &rel[i + 1..]),
        None => ("", rel),
    }
}

/// `0x` + 8 lower-case hex digits (+ optional `.ext`): already a hashed name.
fn is_hashed(leaf: &str) -> bool {
    let stem = leaf.split('.').next().unwrap_or("");
    stem.len() == 10 && stem.starts_with("0x") && stem[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Every key a file at `rel` (a path inside the package = a path in the data tree) answers to, the plain one first.
pub fn keys(rel: &str) -> Vec<String> {
    let n = norm(rel);
    let (dir, leaf) = split(&n);
    let mut out = vec![n.clone()];
    if leaf.is_empty() || is_hashed(leaf) {
        return out;
    }
    out.push(format!("{dir}0x{:08x}", joaat(leaf)));
    if let Some((stem, ext)) = leaf.rsplit_once('.') {
        let src = match ext {
            "r9" => Some("xtd"),
            "r102" => Some("xdr"),
            _ => None,
        };
        if let Some(src) = src {
            out.push(format!("{dir}0x{:08x}.{ext}", joaat(&format!("{stem}.{src}"))));
        }
    }
    out
}

/// The key a reader's path is looked up by (the same normalisation; a hashed path is matched as written).
pub fn lookup_key(rel: &str) -> String {
    norm(rel)
}

/// Whether a disc file or a mod file wins when both exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// the mod's file replaces the disc's (a mod's own car files, an override)
    ModFirst,
    /// the disc's file is used when it exists (a mod only fills gaps)
    DiscFirst,
}
