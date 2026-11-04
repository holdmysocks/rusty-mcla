//! The game's own sky, road and ocean shader programs, as WGSL, loaded at start-up.
//!
//! The nine `xs_*.wgsl` files are a mechanical translation of the original's shader microcode, made on the player's machine from the
//! player's disc (the installer's stage "Translate shaders", crate `mcla-shaders`) into `<data>/shaders_wgsl/`. They are not part of
//! the source tree, so the shader modules that append them (the city / road / sea module, the sky dome, the Perlin and mini-sky passes,
//! the road reflection) take their text from [`text`].
//!
//! A missing or unreadable file does not stop the game. Its text is replaced by a stub with the same function names that draws nothing
//! useful (vertex programs put every vertex at the origin, pixel programs return black), the log gets one error saying what to do, and
//! [`problem`] gives the same sentence for the on-screen notice. What degrades: with `xs_sky`, `xs_perlin`, `xs_sky_cheap`,
//! `xs_sky_env` or `xs_minisky` missing the sky dome, the clouds and the sky reflections are black; with `xs_road` / `xs_roadn` missing
//! the roads are not drawn (day / night); with `xs_water` missing the sea is not drawn; with `xs_pond` missing the ponds keep the generic blended look. Everything else (cars, buildings, sound, menus,
//! driving) runs as usual.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

struct Programs {
    text: HashMap<&'static str, String>,
    problem: Option<String>,
}

static PROGRAMS: OnceLock<Programs> = OnceLock::new();

/// The WGSL declarations a file would have supplied, as a program that draws nothing (see the module comment).
fn stub(name: &str) -> &'static str {
    match name {
        "xs_sky.wgsl" => concat!(
            "fn xs_sky_vs(vin: array<vec4<f32>, 16>) -> XsVsOut { return XsVsOut(vec4<f32>(0.0, 0.0, 0.0, 1.0), array<vec4<f32>, 16>()); }\n",
            "fn xs_sky_ps(rin: array<vec4<f32>, 16>) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n"
        ),
        "xs_perlin.wgsl" => "fn xs_perlin_ps(rin: array<vec4<f32>, 16>) -> vec4<f32> { return vec4<f32>(0.0); }\n",
        "xs_sky_cheap.wgsl" => "fn xs_sky_cheap_ps(rin: array<vec4<f32>, 16>) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n",
        "xs_sky_env.wgsl" => "fn xs_sky_env_ps(rin: array<vec4<f32>, 16>) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n",
        "xs_minisky.wgsl" => "fn xs_minisky_ps(rin: array<vec4<f32>, 16>) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n",
        "xs_road.wgsl" => concat!(
            "fn xs_road_vs(vin: array<vec4<f32>, 16>) -> XsVsOut { return XsVsOut(vec4<f32>(0.0, 0.0, 0.0, 1.0), array<vec4<f32>, 16>()); }\n",
            "fn xs_road_ps(rin: array<vec4<f32>, 16>) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n"
        ),
        "xs_roadn.wgsl" => concat!(
            "fn xs_roadn_vs(vin: array<vec4<f32>, 16>) -> XsVsOut { return XsVsOut(vec4<f32>(0.0, 0.0, 0.0, 1.0), array<vec4<f32>, 16>()); }\n",
            "fn xs_roadn_ps(rin: array<vec4<f32>, 16>) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n"
        ),
        "xs_water.wgsl" => concat!(
            "fn xs_water_vs(vin: array<vec4<f32>, 16>) -> XsVsOut { return XsVsOut(vec4<f32>(0.0, 0.0, 0.0, 1.0), array<vec4<f32>, 16>()); }\n",
            "fn xs_water_lod_vs(vin: array<vec4<f32>, 16>) -> XsVsOut { return XsVsOut(vec4<f32>(0.0, 0.0, 0.0, 1.0), array<vec4<f32>, 16>()); }\n",
            "fn xs_water_ps(rin: array<vec4<f32>, 16>) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n"
        ),
        // (a missing pond file: city.wgsl reads alpha 0 from this stub as "no programs" and draws the basins with the generic blended rule)
        "xs_pond.wgsl" => concat!(
            "fn xs_pond_vs(vin: array<vec4<f32>, 16>) -> XsVsOut { return XsVsOut(vec4<f32>(0.0, 0.0, 0.0, 1.0), array<vec4<f32>, 16>()); }\n",
            "fn xs_pond_ps(rin: array<vec4<f32>, 16>) -> vec4<f32> { return vec4<f32>(0.0); }\n"
        ),
        _ => "",
    }
}

/// Read the nine files from `<data>/shaders_wgsl/`. The first call decides (later calls change nothing); one error is logged when
/// anything is missing.
pub fn load(data: &Path) {
    PROGRAMS.get_or_init(|| {
        let dir = data.join(mcla_shaders::OUT_DIR);
        let mut text = HashMap::new();
        let mut bad: Vec<&'static str> = Vec::new();
        for name in mcla_shaders::OUTPUTS {
            match std::fs::read_to_string(dir.join(name)) {
                Ok(t) if t.contains("fn xs_") => {
                    text.insert(name, split_regs(&unroll_copy(&t)));
                }
                Ok(_) | Err(_) => {
                    bad.push(name);
                    text.insert(name, stub(name).to_string());
                }
            }
        }
        let problem = (!bad.is_empty()).then(|| {
            let msg = format!(
                "{} of the game's sky, road and sea shader files are missing or unreadable in {} ({}). Open the launcher, go to Install and rebuild the game data from your disc. Until then the sky, roads or sea are not drawn.",
                bad.len(),
                dir.display(),
                bad.join(", ")
            );
            log::error!("{msg}");
            msg
        });
        if problem.is_none() {
            log::info!("shader programs: {} files read from {}", mcla_shaders::OUTPUTS.len(), dir.display());
        }
        Programs { text, problem }
    });
}

/// The pixel programs' first line, `r[i] = rin[i]` for i < 16 as a loop, written out as the same 16 copies (files from an
/// installer keep the loop: the translation stays byte for byte the reference one, crates/shaders/tests/outputs.rs). A register array indexed by a loop
/// counter has to live in memory on a driver that does not unroll the loop, and then every register of the program is a
/// memory access: the roads 15x a 3090's time on an RTX 5060 (GitHub #12, docs/PERFORMANCE.md 2026-10-28 (2)). The same
/// arithmetic; the driver may round a few results one step apart (measured there). MCLA_XS_LOOP=1: the loop as before.
fn unroll_copy(t: &str) -> String {
    const LOOP: &str = "for (var i = 0; i < 16; i = i + 1) { r[i] = rin[i]; }";
    if std::env::var("MCLA_XS_LOOP").is_ok() {
        return t.to_string();
    }
    t.replace(LOOP, &(0..16).map(|i| format!("r[{i}] = rin[{i}];")).collect::<Vec<_>>().join(" "))
}

/// The programs' register arrays as one variable per register: the constants `xs_c` (`var<private> xs_c: array<vec4<f32>,
/// 256>` in city.wgsl, refl.wgsl and the sky files) and each program's own registers (`var r: array<vec4<f32>, 64>`),
/// `xs_c[26]` -> `xs_c_26`, `r[5]` -> `r_5`. Every index is a literal (after [`unroll_copy`]) and an unset variable is zero
/// as the array's element was, so this is the same program; but an array that size stays in memory on the compilers
/// that do not split it - and the city's one pixel program then pays for it in every draw of the city, props and cars
/// (DX12 with DXC on a 3090: city A 26 ms, props 33; the same shape on an RX 5500 XT's Vulkan; GitHub #33).
/// MCLA_XS_ARRAY=1: the arrays as before.
pub fn split_consts(src: &str) -> String {
    const DECL: &str = "var<private> xs_c: array<vec4<f32>, 256>;";
    if std::env::var("MCLA_XS_ARRAY").is_ok() || src.matches(DECL).count() != 1 {
        return src.to_string();
    }
    let Some((out, used)) = split_index(src, "xs_c") else { return src.to_string() };
    let decls: String = used.iter().map(|k| format!("var<private> xs_c_{k}: vec4<f32>;\n")).collect();
    out.replacen(DECL, &decls, 1)
}

/// [`split_consts`]'s split of each program's `r` (a translated file: its functions start at a line `fn `).
fn split_regs(t: &str) -> String {
    const DECL: &str = "    var r: array<vec4<f32>, 64>;\n";
    if std::env::var("MCLA_XS_ARRAY").is_ok() {
        return t.to_string();
    }
    let mut out = String::with_capacity(t.len() + 4096);
    for (n, f) in t.split("\nfn ").enumerate() {
        if n > 0 {
            out.push_str("\nfn ");
        }
        match (f.matches(DECL).count() == 1).then(|| split_index(f, "r")).flatten() {
            Some((g, used)) => {
                let decls: String = used.iter().map(|k| format!("    var r_{k}: vec4<f32>;\n")).collect();
                out.push_str(&g.replacen(DECL, &decls, 1));
            }
            None => out.push_str(f),
        }
    }
    out
}

/// `name[N]` -> `name_N` wherever `name` stands as a word, and the registers met; None at an index that is not a literal.
fn split_index(src: &str, name: &str) -> Option<(String, std::collections::BTreeSet<u32>)> {
    let pat = format!("{name}[");
    let mut out = String::with_capacity(src.len() + 4096);
    let mut used = std::collections::BTreeSet::new();
    let mut rest = src;
    while let Some(i) = rest.find(&pat) {
        let word = rest[..i].chars().next_back().is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        let tail = &rest[i + pat.len()..];
        if !word {
            out.push_str(&rest[..i + pat.len()]);
            rest = tail;
            continue;
        }
        let n = tail.bytes().take_while(|b| b.is_ascii_digit()).count();
        if n == 0 || !tail[n..].starts_with(']') {
            return None;
        }
        let k: u32 = tail[..n].parse().ok()?;
        used.insert(k);
        out.push_str(&rest[..i]);
        out.push_str(&format!("{name}_{k}"));
        rest = &tail[n + 1..];
    }
    out.push_str(rest);
    Some((out, used))
}

/// The text of one translated file (or its stub). Empty before [`load`].
pub fn text(name: &str) -> &'static str {
    PROGRAMS.get().and_then(|p| p.text.get(name)).map(|s| s.as_str()).unwrap_or("")
}

/// The sentence for the on-screen notice when a file could not be read; `None` when all nine were.
pub fn problem() -> Option<&'static str> {
    PROGRAMS.get().and_then(|p| p.problem.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_output_has_a_stub_with_its_functions() {
        for name in mcla_shaders::OUTPUTS {
            let s = stub(name);
            assert!(s.contains("fn xs_"), "{name}");
        }
    }

    #[test]
    fn register_arrays_split_by_literal_index() {
        let t = "fn xs_a(rin: array<vec4<f32>, 16>) -> vec4<f32> {\n    var r: array<vec4<f32>, 64>;\n    r[0] = rin[0]; r[12] = xs_c[3] + r[0]; hr[1] = r[12];\n    return r[12];\n}\nfn xs_b() {\n    var q = 1;\n}\n";
        let r = split_regs(t);
        assert_eq!(r, "fn xs_a(rin: array<vec4<f32>, 16>) -> vec4<f32> {\n    var r_0: vec4<f32>;\n    var r_12: vec4<f32>;\n    r_0 = rin[0]; r_12 = xs_c[3] + r_0; hr[1] = r_12;\n    return r_12;\n}\nfn xs_b() {\n    var q = 1;\n}\n");
        let c = split_consts(&format!("var<private> xs_c: array<vec4<f32>, 256>;\n{r}"));
        assert!(c.starts_with("var<private> xs_c_3: vec4<f32>;\n\nfn xs_a") && c.contains("r_12 = xs_c_3 + r_0;"), "{c}");
        // (an index that is not a literal: the text as it was)
        let d = "fn xs_c(i: i32) {\n    var r: array<vec4<f32>, 64>;\n    r[i] = vec4<f32>(0.0);\n}\n";
        assert_eq!(split_regs(d), d);
    }
}
