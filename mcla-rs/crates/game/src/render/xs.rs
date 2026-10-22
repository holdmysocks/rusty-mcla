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
                    text.insert(name, t);
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
}
