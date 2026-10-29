//! City world: sector index, distance-based streaming on worker threads, GPU batches.

use crate::assets::{load_resource_file, Assets};
use crate::physics::{SectorMesh, WorldCollider};
use crate::render::{Gpu, GpuTexture, MatMode, Vertex};
use anyhow::Result;
use glam::Vec3;
use hashbrown::HashMap;
use mcla_formats::rage::navsys::NavSys;
use mcla_formats::rage::sector::Sector;
use mcla_formats::rage::texture::Texture;
use mcla_formats::rage::{Dictionary, Rd};
use mcla_formats::rsc::Resource;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SectorInfo {
    pub file: String,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub tris: u32,
}

#[derive(Serialize, Deserialize, Default)]
pub struct WorldIndex {
    pub sectors: Vec<SectorInfo>,
    pub dicts: Vec<String>,
}

impl WorldIndex {
    /// Build (or load cached) index of all city sectors.
    pub fn load_or_build(assets: &Assets) -> Result<WorldIndex> {
        let cache = assets.path("world_index.json");
        if let Ok(t) = std::fs::read_to_string(&cache) {
            if let Ok(idx) = serde_json::from_str::<WorldIndex>(&t) {
                if !idx.sectors.is_empty() {
                    return Ok(idx);
                }
            }
        }
        log::info!("building world index…");
        let files = assets.city_files()?;
        let results: Vec<(PathBuf, u32, Option<SectorInfo>)> = files
            .par_iter()
            .map(|p| {
                let Ok(res) = load_resource_file(p) else { return (p.clone(), 0, None) };
                let vt = Rd::new(&res).vtable(0);
                if vt != mcla_formats::rage::vt::CITY_SECTOR {
                    return (p.clone(), vt, None);
                }
                let Ok(sec) = Sector::parse(&res) else { return (p.clone(), vt, None) };
                let mut mn = Vec3::splat(f32::INFINITY);
                let mut mx = Vec3::splat(f32::NEG_INFINITY);
                let mut tris = 0u32;
                for b in &sec.batches {
                    let (a, c) = b.mesh.bbox();
                    mn = mn.min(a);
                    mx = mx.max(c);
                    tris += (b.mesh.indices.len() / 3) as u32;
                }
                let rel = p.strip_prefix(&assets.root).unwrap_or(p).to_string_lossy().replace('\\', "/");
                (p.clone(), vt, Some(SectorInfo { file: rel, min: mn.to_array(), max: mx.to_array(), tris }))
            })
            .collect();
        let mut idx = WorldIndex::default();
        for (p, vt, s) in results {
            if let Some(s) = s {
                idx.sectors.push(s);
            } else if vt == mcla_formats::rage::vt::TEXTURE_DICT {
                idx.dicts.push(p.strip_prefix(&assets.root).unwrap_or(&p).to_string_lossy().replace('\\', "/"));
            }
        }
        let _ = std::fs::write(&cache, serde_json::to_string_pretty(&idx)?);
        log::info!("index: {} sectors, {} shared texture dicts", idx.sectors.len(), idx.dicts.len());
        Ok(idx)
    }
}

/// Decoded, CPU-side sector ready for upload.
pub struct CpuSector {
    pub id: usize,
    pub batches: Vec<CpuBatch>,
    pub textures: Vec<(u32, Texture)>, // sector-local textures
    pub collision: Vec<[Vec3; 3]>,
    /// surface of each collision triangle (empty = road)
    pub collision_mat: Vec<u8>,
    /// `batches` put together for the buffers on the decoding worker (`prepare_geo`); None: done at upload
    pub geo: Option<Geo>,
}

/// A sector's batches put together for its two buffers - the vertices and the kept indices in draw order, each
/// batch's place in them - on the worker that decoded it, so the main thread only makes the buffers and the bind
/// groups (docs/PERFORMANCE.md 2026-10-23 (5)). The same as `upload` did in place before.
pub struct Geo {
    verts: Vec<Vertex>,
    indices: Vec<u32>,
    items: Vec<GeoItem>,
}

struct GeoItem {
    /// the batch, its vertices and indices moved out (an empty one: `keep` false, not drawn)
    b: CpuBatch,
    keep: bool,
    base: u32,
    n: u32,
    first: u32,
    count: u32,
    parts: Vec<(u32, u32, Vec3, f32, u32)>,
}

fn geo_rank(b: &CpuBatch) -> u32 {
    if b.mode.blended() {
        2
    } else if shadow_cutout(b.mode) {
        1
    } else {
        0
    }
}

pub fn prepare_geo(batches: Vec<CpuBatch>) -> Geo {
    // plain opaque casters first (one shadow draw), then by material mode
    let mut src: Vec<(bool, CpuBatch)> = batches.into_iter().map(|b| (!b.indices.is_empty() && !b.vertices.is_empty(), b)).collect();
    // (the shop panes keep the sector's own batch order - the sort is stable: the original draws them
    // blended WITH depth writes in that order, so of two panes one behind the other the first drawn decides
    // what shows - a shop door behind an angled front pane is not drawn, LIGHTING_WORK.md 2026-10-15)
    src.sort_by_key(|(_, b)| if b.mode == MatMode::StoreGlass { (geo_rank(b), b.mode as u32, Default::default(), Default::default()) } else { (geo_rank(b), b.mode as u32, b.diffuse, b.second) });
    let mut verts: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut items = Vec::with_capacity(src.len());
    for (keep, mut b) in src {
        if !keep {
            items.push(GeoItem { b, keep, base: 0, n: 0, first: 0, count: 0, parts: Vec::new() });
            continue;
        }
        let base = verts.len() as u32;
        let n = b.vertices.len() as u32;
        let first = indices.len() as u32;
        // (kept[k]: how many indices of the first k triangles are kept - the parts' runs follow)
        let mut kept: Vec<u32> = Vec::with_capacity(b.indices.len() / 3 + 1);
        kept.push(0);
        for t in b.indices.chunks_exact(3) {
            if t.iter().all(|&i| i < n) {
                indices.extend(t.iter().map(|&i| i + base));
            }
            kept.push(indices.len() as u32 - first);
        }
        let count = indices.len() as u32 - first;
        let at = |i: u32| kept[((i / 3) as usize).min(kept.len() - 1)];
        let parts: Vec<(u32, u32, Vec3, f32, u32)> = b.parts.iter().map(|p| (first + at(p.0), at(p.0 + p.1) - at(p.0), p.2, p.3, p.4)).filter(|p| p.1 > 0).collect();
        verts.extend(std::mem::take(&mut b.vertices));
        b.indices = Vec::new();
        items.push(GeoItem { b, keep, base, n, first, count, parts });
    }
    Geo { verts, indices, items }
}

pub struct CpuBatch {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub diffuse: u32,
    pub second: u32,
    /// grime layer of ground shaders (0 = none); terrain: the second of its three textures
    pub third: u32,
    /// terrain: the third of its three textures
    pub fourth: u32,
    pub mode: MatMode,
    pub min: Vec3,
    pub max: Vec3,
    pub shader: String,
    /// a ground batch of a tunnel or car park (sector batch flag 4; walls carry the mark in
    /// their vertices instead): lit as an interior
    pub interior: bool,
    /// The batches of the sector file this one is merged from, each a run of `indices`: (first
    /// index, count, bounding sphere centre and radius - the box's centre and half diagonal, as the
    /// file's own sphere table has them -, the file's batch flags). The cars' environment map
    /// draws a sector batch by batch (main.rs, "environment, paraboloid half"). Empty: one run.
    pub parts: Vec<(u32, u32, Vec3, f32, u32)>,
}

pub struct GpuBatch {
    pub shader: String,
    pub vb: wgpu::Buffer,
    pub ib: wgpu::Buffer,
    pub count: u32,
    pub bind: Arc<wgpu::BindGroup>,
    pub mode: MatMode,
    pub min: Vec3,
    pub max: Vec3,
    /// the shader's draw bucket (`Shader::bucket`; only traffic cars' batches carry 1 - vehicles.rs `Vehicles::draw`)
    pub bucket: u8,
}

/// One material range inside a sector's shared vertex / index buffers.
pub struct SectorBatch {
    /// drawn with the sector's interior object uniform (see `GpuSector::obj`)
    pub interior: bool,
    pub shader: String,
    pub first: u32,
    pub count: u32,
    pub bind: Arc<wgpu::BindGroup>,
    pub mode: MatMode,
    pub min: Vec3,
    pub max: Vec3,
    /// the sector file's batches inside this range (see `CpuBatch::parts`; first index absolute)
    pub parts: Vec<(u32, u32, Vec3, f32, u32)>,
}

pub struct GpuSector {
    /// all batches of the sector share one vertex and one index buffer
    pub vb: Option<wgpu::Buffer>,
    pub ib: Option<wgpu::Buffer>,
    pub batches: Vec<SectorBatch>,
    /// index range of the plain opaque shadow casters (they come first in the buffer)
    pub caster_count: u32,
    pub caster_min: Vec3,
    pub caster_max: Vec3,
    /// number of leading batches covered by the caster range
    pub caster_batches: usize,
    pub tris: u32,
    /// The sector's two object uniforms (plain batches, interior batches). Their last dent
    /// slot holds (alpha, interior, 0, -7): the alpha of the cross-fade between a sector and its
    /// far model, dithered by the pixel program.
    pub obj: Option<(crate::render::ObjectBinding, crate::render::ObjectBinding)>,
    /// a detailed sector: how far it is in (0 .. 1 over a third of a second); a far model: its alpha
    pub u: f32,
    /// the alpha last written to `obj`
    pub shown: f32,
    /// a detailed sector on its way out
    pub leaving: bool,
}

/// Object uniform of a sector's batches: alpha of the cross-fade, interior or not.
fn sector_uniform(alpha: f32, interior: bool) -> crate::render::ObjectUniform {
    let mut u = crate::render::ObjectUniform::default();
    u.dents[5] = [alpha, if interior { 1.0 } else { 0.0 }, 0.0, -7.0];
    u
}

/// Does this material need its texture in the shadow pass (cut-outs)?
pub fn shadow_cutout(mode: MatMode) -> bool {
    matches!(mode, MatMode::AlphaTest | MatMode::BlackGloss | MatMode::Foliage)
}

/// Shared texture dictionaries (common.r1 etc.) kept resident on the CPU side.
pub struct SharedTextures {
    dicts: Vec<(Resource, Dictionary)>,
}

impl SharedTextures {
    /// Add every texture dictionary found in `dir` (recursively).
    pub fn add_dir(&mut self, dir: &Path) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                self.add_dir(&p);
                continue;
            }
            if p.extension().map_or(false, |x| x == "r1") {
                if let Ok(r) = load_resource_file(&p) {
                    let rdr = Rd::new(&r);
                    if rdr.vtable(0) == mcla_formats::rage::vt::TEXTURE_DICT {
                        if let Ok(d) = Dictionary::parse(&rdr, 0) {
                            self.dicts.push((r, d));
                        }
                    }
                }
            }
        }
    }
}

impl SharedTextures {
    pub fn load(assets: &Assets, files: &[String]) -> SharedTextures {
        let mut dicts = Vec::new();
        for f in files {
            match assets.load_resource(f) {
                Ok(r) => {
                    if let Ok(d) = Dictionary::parse(&Rd::new(&r), 0) {
                        log::info!("shared texture dict {} ({} textures)", f, d.len());
                        dicts.push((r, d));
                    }
                }
                Err(e) => log::warn!("{}: {:#}", f, e),
            }
        }
        SharedTextures { dicts }
    }
    pub fn get(&self, hash: u32) -> Option<Texture> {
        for (r, d) in &self.dicts {
            if let Some(o) = d.get(hash) {
                if let Ok(t) = Texture::parse(&Rd::new(r), o) {
                    return Some(t);
                }
            }
        }
        None
    }
}

/// MCLA_WATER_OLD=1: the sea as the generic blended material (before its own programs, 2026-10-14)
fn water_old() -> bool {
    static OLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OLD.get_or_init(|| std::env::var("MCLA_WATER_OLD").map_or(false, |v| v == "1"))
}

fn classify(shader: &str, flags: u32) -> MatMode {
    let s = shader.to_ascii_lowercase();
    if s.contains("cityterrain") || s.contains("citygrass") {
        // (PSCityTerrain / PSCityGrass: DiffuseSamplerA, B and C weighted by the channel map)
        MatMode::Terrain
    } else if s.contains("storesign") {
        MatMode::StoreSign
    } else if s.contains("neontint") {
        MatMode::NeonTint
    } else if s.contains("storeinterior") {
        MatMode::StoreInterior
    } else if s.contains("windowdirt") {
        // (PSCityWindowDirt: a film of dirt blended over shop glass - batch flag 1)
        MatMode::Blend
    } else if s.contains("windowlower") {
        MatMode::StoreGlass
    } else if s.contains("window") {
        MatMode::Window
    } else if (s.contains("cityroad") && !s.contains("stud")) || s.contains("citytunnelroad") {
        // (PSCityRoad / PSCityTunnelRoad: the colour is the diffuse map's red channel as grey,
        // its green channel the specular mask; decal atlas and grime over it)
        MatMode::Road
    } else if s.contains("sidewalk") || s.contains("groundgrime") || s.contains("grounddecalgrime") || s.contains("riverroad") {
        // (PSCitySidewalk, PSCityGroundGrime(Decal), PSCityGroundDecalGrime, PSCityRiverRoad:
        // the diffuse map in its own colours, grime and decals over it)
        MatMode::Ground
    } else if s.contains("decal") {
        MatMode::Decal
    } else if s.contains("neon") || s.contains("billboard") {
        MatMode::Neon
    } else if s.contains("oceanwater") && water_old() {
        // (MCLA_WATER_OLD=1: the sea drawn blended with the generic rule, as until 2026-10-14)
        MatMode::Blend
    } else if s.contains("oceanwaterlod") {
        // (the sea, CityOceanWater / CityOceanWaterLOD: the game's own programs - xs_water.wgsl, render/xs.rs;
        // the pond water, CityPondWater, stays in the blended mode and runs its own programs there - `mark_pond`)
        MatMode::OceanWaterLod
    } else if s.contains("oceanwater") {
        MatMode::OceanWater
    } else if s.contains("water") || s.contains("glass") {
        MatMode::Blend
    } else if s.contains("decal") || s.contains("alpha") || s.contains("fence") || s.contains("tree") || s.contains("foliage") || s.contains("cutout") || s.contains("grass") || flags & 3 != 0 {
        // (batch flag 2 marks cut-out geometry drawn with the ordinary wall shaders:
        // grass tufts, chain-link and picket fences, hanging wires)
        MatMode::AlphaTest
    } else {
        MatMode::Opaque
    }
}

/// Shader parameter name hashes (joaat of the HLSL sampler names).
pub mod ph {
    pub const DIFFUSE: u32 = 0xf1fe2b71; // DiffuseSampler
    pub const GRIME: u32 = 0xe3381c99; // GrimeSampler
    pub const BUMP: u32 = 0x332ebe1c; // HeightSpecularSampler (*_bs)
    pub const DECAL: u32 = 0xa79aeec0; // DecalSampler
    pub const PUDDLE: u32 = 0xfe553678; // PuddleSampler
    pub const REFLECTION: u32 = 0x3c9796a8; // ReflectionSampler
    pub const LIT: u32 = 0x3c870418; // NeonSampler
    pub const WINDOW_LIT: u32 = 0x44c9041c; // WindowSampler
    pub const HEIGHT_SPEC: u32 = 0x332ebe1c; // HeightSpecularSampler (bump in red and alpha, specular in green)
    pub const LITUP: u32 = 0x2b8f09d1; // LitUpSampler (store interiors)
    pub const GRASS_MAP: u32 = 0x8bfcef8d; // ChannelMapSampler
    pub const GRASS_A: u32 = 0xc9a79fed; // DiffuseSamplerA
    pub const GRASS_B: u32 = 0xf7357b08; // DiffuseSamplerB
    pub const GRASS_C: u32 = 0xa4cfd63a; // DiffuseSamplerC
}

/// Added to the packed uv2.y of an interior wall's vertices (bit 20; the bump terms end at bit 18).
pub const INTERIOR_MARK: f32 = 1048576.0;
/// Added to the packed uv2.y of the garage sets' floor (`CityGarageFloor`, bit 22): its program (VSCityGarageFloor +
/// PSMultiLight) multiplies the baked light V by the car's cascade shadow, 2 x the collector, which the walls' does not.
pub const GARAGE_FLOOR_MARK: f32 = 4194304.0;

/// The garage floor's material terms, laid on its vertices for its program (city.wgsl, the garage floor): the floor
/// mark, and below it SpecularExponent, SpecularIntensity in hundredths and SpecularColorMixer in tenths in the walls'
/// layout (bits 0 - 18 of uv2.y); in uv2.x, negative, -(1 + ReflectionIntensityA + 101 ReflectionIntensityB + 10201
/// Bumpiness), the first two in hundredths, Bumpiness in quarters up to 1.5 (above -77000: not the sea's mark).
/// Recorded in the Hollywood garage (cap/gar_*.gpu.bin, all four floor batches): 8, 0.4, 0.7, 0.1, 0.1, 1.
fn mark_garage_floor(vertices: &mut [Vertex], sh: &mcla_formats::rage::drawable::Shader) {
    use mcla_formats::rage::drawable::ShaderParam;
    let f = |h: u32, d: f32| {
        sh.params.iter().find_map(|(ph, p)| match p {
            ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0].x),
            _ => None,
        }).unwrap_or(d)
    };
    let spec = f(0x93d520e8, 8.0).round().clamp(1.0, 255.0) + (f(0xa95fc535, 0.4) * 100.0).round().clamp(0.0, 127.0) * 256.0 + (f(0xbaf643f4, 0.7) * 10.0).round().clamp(0.0, 10.0) * 32768.0;
    let (ra, rb, bump) = (f(0x99dcb623, 0.1), f(0xf1f66655, 0.1), f(0xf6712b81, 1.0));
    let packed = 1.0 + (ra * 100.0).round().clamp(0.0, 100.0) + 101.0 * (rb * 100.0).round().clamp(0.0, 100.0) + 10201.0 * (bump * 4.0).round().clamp(0.0, 6.0);
    if std::env::var("MCLA_GARAGE_FLOOR_LOG").is_ok() {
        log::info!("garage floor {}: SpecularExponent / Intensity / ColorMixer {:.3} {:.3} {:.3}, ReflectionIntensityA / B {:.3} {:.3}, Bumpiness {:.3}", sh.name, f(0x93d520e8, -1.0), f(0xa95fc535, -1.0), f(0xbaf643f4, -1.0), f(0x99dcb623, -1.0), f(0xf1f66655, -1.0), f(0xf6712b81, -1.0));
    }
    for v in vertices.iter_mut() {
        v.uv2[0] = -packed;
        v.uv2[1] += GARAGE_FLOOR_MARK + spec;
    }
}

/// The classes of SpecularExponent a CityGrimeDecal batch can carry (see `mark_decal_spec`).
pub const DECAL_SPEC_EXP: [f32; 8] = [8.0, 15.0, 25.0, 50.0, 100.0, 150.0, 175.0, 255.0];

/// Walls with a decal AND a grime layer (CityGrimeDecal, mode 14) have the bump-spec walls' map
/// and highlight too (HeightSpecularSampler - their fourth texture; SpecularExponent / Intensity /
/// ColorMixer 25 / 0.5 / 0.5, 15 / 0.3 / 0.75, 8 / 0.33 / 0.7 in the frames looked at). All their
/// vertex streams are taken (three UV sets, two colours), so the terms ride on the third UV set -
/// the grime's - as whole multiples of 64 (the map repeats: what is sampled stays the same; the
/// shader takes them off again): x + 64 x (class of the exponent + 8 x mixer in twentieths),
/// y + 64 x intensity in hundredths (1 .. 127; 0 = no map). Batches whose third set leaves
/// -31 .. 31 stay unmarked.
fn mark_decal_spec(vertices: &mut [Vertex], sh: &mcla_formats::rage::drawable::Shader) {
    use mcla_formats::rage::drawable::ShaderParam;
    if !sh.textures().any(|(h, _)| h == ph::HEIGHT_SPEC) || vertices.iter().any(|v| v.uv2[0].abs() >= 31.0 || v.uv2[1].abs() >= 31.0) {
        return;
    }
    let f = |h: u32, d: f32| {
        sh.params.iter().find_map(|(ph, p)| match p {
            ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0].x),
            _ => None,
        }).unwrap_or(d)
    };
    let e = f(0x93d520e8, 8.0);
    let class = (0..8).min_by(|a, b| (DECAL_SPEC_EXP[*a] - e).abs().partial_cmp(&(DECAL_SPEC_EXP[*b] - e).abs()).unwrap()).unwrap_or(0) as f32;
    let inten = (f(0xa95fc535, 0.33) * 100.0).round().clamp(0.0, 127.0);
    let mixer = (f(0xbaf643f4, 0.7) * 20.0).round().clamp(0.0, 20.0);
    if inten < 1.0 {
        return;
    }
    for v in vertices.iter_mut() {
        v.uv2[0] += 64.0 * (class + 8.0 * mixer);
        v.uv2[1] += 64.0 * inten;
    }
}

/// Walls with a grime layer (PSCityGrime and its kin: the grime map lies on the second UV
/// set and is tinted by the vertex colour) are flagged for the shader in the spare third UV
/// set of their vertices; the other wall shaders' second texture stays on the first set.
///
/// `interior`: the batch carries flag 4 - a tunnel, car park or garage wall, which the original
/// draws with PSMultiLightInterior at every hour. Passed on in bit 20 of the packed y.
fn mark_grime(vertices: &mut [Vertex], sh: &mcla_formats::rage::drawable::Shader, mode: MatMode, interior: bool) {
    if mode == MatMode::Decal {
        mark_decal_spec(vertices, sh);
    }
    if !matches!(mode, MatMode::Opaque | MatMode::AlphaTest) {
        return;
    }
    let grime = sh.textures().any(|(h, _)| h == ph::GRIME) && !sh.textures().any(|(h, _)| h == ph::LITUP);
    // Walls with a bump and specular map (PSCityBumpSpec, PSCityGrime ...: HeightSpecularSampler)
    // carry their specular terms in y: SpecularExponent, SpecularIntensity in hundredths and
    // SpecularColorMixer in tenths (0 = no such map).
    let bump = if sh.textures().any(|(h, _)| h == ph::HEIGHT_SPEC) {
        use mcla_formats::rage::drawable::ShaderParam;
        let f = |h: u32, d: f32| {
            sh.params.iter().find_map(|(ph, p)| match p {
                ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0].x),
                _ => None,
            }).unwrap_or(d)
        };
        f(0x93d520e8, 8.0).round().clamp(1.0, 255.0) + (f(0xa95fc535, 0.33) * 100.0).round().clamp(0.0, 127.0) * 256.0 + (f(0xbaf643f4, 0.7) * 10.0).round().clamp(0.0, 10.0) * 32768.0
    } else {
        0.0
    };
    // Metal walls and tunnel tiles (PSCityGrimeMetal, PSCityTunnelTile) mirror their
    // surroundings at a glancing angle: 3 + ReflectionIntensity in x.
    let lname = sh.name.to_ascii_lowercase();
    let metal = if lname.contains("grimemetal") || lname.contains("tunneltile") {
        use mcla_formats::rage::drawable::ShaderParam;
        sh.params.iter().find_map(|(ph, p)| match p {
            ShaderParam::Vector(v) if *ph == 0x73e4f90c && !v.is_empty() => Some(3.0 + v[0].x.clamp(0.0, 4.0)),
            _ => None,
        })
    } else {
        None
    };
    if interior && std::env::var("MCLA_TUNNEL_DUMP").is_ok() && (lname.contains("grimemetal") || lname.contains("tunneltile")) {
        log::info!("interior {}: 3 + ReflectionIntensity {:?}, packed specular terms {}", sh.name, metal, bump);
    }
    let bump = bump + if interior { INTERIOR_MARK } else { 0.0 };
    for v in vertices.iter_mut() {
        v.uv2 = [metal.unwrap_or(if grime { 1.0 } else { 0.0 }), bump];
    }
}

/// The binormal's sign (2026-10-14). Every bump-mapped program of the original (VSCityBumpSpec,
/// VSCityGrime, VSCityGrimeDecal, the ground forms, their night forms, VSPropBumpSpec) fetches the
/// packed tangent word signed and hands the pixel program `cross(T, N) x w`: the word's two top
/// bits read +1 (raw 1) or -1 (raw 3; raw 2 would clamp to -1, raw 0 would be 0 - neither occurs
/// in the city's files). There is no free vertex stream for it, so it rides in the LENGTH of the
/// vertex's normal - 1: w = +1 (everything else too), 2: w = -1, 1.5: w = 0 - which the vertex
/// stages take off again (city.wgsl `bflip`; fast_collector.wgsl and refl.wgsl normalise).
/// Only batches with a height / specular map are touched.
pub fn mark_binormal(vertices: &mut [Vertex], tangents: &[glam::Vec4], sh: &mcla_formats::rage::drawable::Shader) {
    if !sh.textures().any(|(h, _)| h == ph::HEIGHT_SPEC) {
        return;
    }
    mark_binormal_any(vertices, tangents);
}

/// `mark_binormal` for a batch whose program reads the tangent without a height / specular map (the garage floor's
/// VSCityGarageFloor: cross(T, N) x w too).
pub fn mark_binormal_any(vertices: &mut [Vertex], tangents: &[glam::Vec4]) {
    if tangents.len() != vertices.len() {
        return;
    }
    for (v, t) in vertices.iter_mut().zip(tangents) {
        let k = if t.w >= 1.5 { 2.0 } else if t.w < 0.5 { 1.5 } else { continue };
        for c in &mut v.normal {
            *c *= k;
        }
    }
}

/// Mark in the spare third UV set (bit 21 of y) of the batches drawn with the far city's
/// programs (CityLOD: a day and a night picture of the sector). At night their PSMultiLight
/// takes the baked light from the night picture, `NightCPVSampler`, where the other walls have
/// it in their second vertex colour (the far models carry no colours at all): the picture goes
/// into the batch's third texture slot.
pub const LOD_MARK: f32 = 2097152.0;
fn mark_lod(vertices: &mut [Vertex], sh: &mcla_formats::rage::drawable::Shader, mode: MatMode, third: &mut u32) {
    if !matches!(mode, MatMode::Opaque | MatMode::AlphaTest) {
        return;
    }
    // (joaat of "nightcpvsampler")
    let Some(night) = sh.textures().find(|(h, _)| *h == 0x148121fc).map(|(_, n)| mcla_formats::hash::joaat(n)) else { return };
    *third = night;
    for v in vertices.iter_mut() {
        v.uv2[1] += LOD_MARK;
    }
}

/// Tunnel roads (CityTunnelRoad batches with flag 4) have their own interior program
/// (PSMultiLightInterior of city/0x3301345a: no sun at any hour - work/lighting/tun_rules.py);
/// the other interior floors (CityRoad, CityGarageFloor) take the CityRoad one. Both are
/// `MatMode::Road` drawn with the sector's interior uniform, so the tunnel road is told apart
/// in its vertices: a whole number added to the grime coordinate's y (the third UV set; the grime
/// map repeats, so what is sampled stays the same - the shader tests `uv2.y > TUNNEL_ROAD_MARK / 2`).
/// A batch whose own coordinates reach that far is left unmarked (and reported).
/// (the coordinate runs to about +-50 on the tunnel roads looked at; 512 costs 1/32 of a texel of
/// a 512 map in precision)
pub const TUNNEL_ROAD_MARK: f32 = 512.0;
/// The sea's vertices (CityOceanWater / CityOceanWaterLOD batches) carry WATER_MARK in uv2.x - their third
/// coordinate set is not read by the sea's programs - and 0 (detailed) / 1 (far) in uv2.y: city.wgsl's
/// vertex stage runs the game's VSCityOceanWater on them PER VERTEX, as the original does (`water_vertex`):
/// the waves move the vertices, and the wave frame is interpolated across the sea's large triangles.
pub const WATER_MARK: f32 = -77777.0;
fn mark_water(vertices: &mut [Vertex], mode: MatMode) {
    let kind = match mode {
        MatMode::OceanWater => 0.0,
        MatMode::OceanWaterLod => 1.0,
        _ => return,
    };
    for v in vertices.iter_mut() {
        v.uv2 = [WATER_MARK, kind];
    }
}

/// The pond water (CityPondWater batches: the fountain basins, 32 materials in the city, all with the same values -
/// RippleParams (1, 0.1, 0.1, 0), SpecularExponent 8, SpecularIntensity 0.33, SpecularColorMixer 0.7, DiffuseSampler
/// l_sha_ground_water.dds, WaveSampler l_sha_water_ripple_n.dds) stays in the blended mode, as the original draws it
/// (SrcAlpha / InvSrcAlpha, after the opaque city), and carries POND_MARK in uv2.x - its third coordinate set is not
/// read by its programs: city.wgsl runs the game's VSCityPondWater + PSCityPondWater on it (`pond_program`,
/// xs_pond.wgsl). MCLA_POND_OLD=1: unmarked, the generic blended rule as before.
pub const POND_MARK: f32 = -55555.0;
fn is_pond(shader: &str) -> bool {
    static OLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    !*OLD.get_or_init(|| std::env::var("MCLA_POND_OLD").map_or(false, |v| v == "1")) && shader.to_ascii_lowercase().contains("citypondwater")
}
fn mark_pond(vertices: &mut [Vertex], sh: &mcla_formats::rage::drawable::Shader, mode: MatMode) {
    if mode != MatMode::Blend || !is_pond(&sh.name) {
        return;
    }
    for v in vertices.iter_mut() {
        v.uv2 = [POND_MARK, 0.0];
    }
}

fn mark_tunnel_road(vertices: &mut [Vertex], sh: &mcla_formats::rage::drawable::Shader, mode: MatMode, interior: bool) {
    if mode != MatMode::Road || !interior || !sh.name.to_ascii_lowercase().contains("citytunnelroad") {
        return;
    }
    let (lo, hi) = vertices.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(v.uv2[1]), hi.max(v.uv2[1])));
    if std::env::var("MCLA_TUNNEL_DUMP").is_ok() {
        log::info!("tunnel road batch: {} vertices, grime v {:.2} .. {:.2}", vertices.len(), lo, hi);
    }
    if !(lo > -TUNNEL_ROAD_MARK * 0.45 && hi < TUNNEL_ROAD_MARK * 0.45) {
        log::warn!("tunnel road batch not marked: grime v {} .. {}", lo, hi);
        return;
    }
    for v in vertices.iter_mut() {
        v.uv2[1] += TUNNEL_ROAD_MARK;
    }
}

/// What the window shaders take from their material, stowed in the spare third UV set.
/// Upper windows (PSCityWindowUpper): `WindowTintColor`, 7 bits a channel, in x; in y the
/// brightness of the lit room behind - `WindowNight`, plus 2 where `WindowDay` keeps it lit
/// by day as well. Shop fronts (PSCityWindowLower): `OpaqueReflectionIntensity` in x.
fn mark_window(vertices: &mut [Vertex], sh: &mcla_formats::rage::drawable::Shader, mode: MatMode) {
    use mcla_formats::rage::drawable::ShaderParam;
    let val = |h: u32| {
        sh.params.iter().find_map(|(ph, p)| match p {
            ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0]),
            _ => None,
        })
    };
    let uv2 = match mode {
        MatMode::Window => {
            let t = val(0x95b09a44).map_or(glam::Vec3::ONE, |v| v.truncate()).clamp(glam::Vec3::ZERO, glam::Vec3::ONE);
            let q = |c: f32| (c * 127.0).round();
            let lit = sh.textures().any(|(h, _)| h == ph::WINDOW_LIT);
            let night = if lit { val(0x056fd4ad).map_or(1.0, |v| v.x).clamp(0.0, 1.5) } else { 0.0 };
            let day = lit && val(0x36e21f16).map_or(0.0, |v| v.x) >= 0.5;
            // (+ 8: a window model of the FAR city, CityWindowLOD - its own programs, city.wgsl mode 3)
            let lod = sh.name.to_ascii_lowercase().contains("windowlod");
            // (far, 2026-10-13: + 16 x SpecularExponent on the NEAR windows - their night program's highlight on the
            // opaque texels, city.wgsl mode 3 `fc_e3`; the far windows' program has none)
            let fc_exp = if lod { 0.0 } else { val(0x93d520e8).map_or(0.0, |v| v.x.round().clamp(0.0, 255.0)) };
            [q(t.x) + q(t.y) * 128.0 + q(t.z) * 16384.0, night + if day { 2.0 } else { 0.0 } + if lod { 8.0 } else { 0.0 } + 16.0 * fc_exp]
        }
        // (y: 8 x SpecularExponent + SpecularColorMixer - the night pane program's highlight on opaque texels)
        MatMode::StoreGlass => [val(0xa25bd5a4).map_or(0.05, |v| v.x), val(0x93d520e8).map_or(0.0, |v| (v.x * 8.0).round().clamp(0.0, 2040.0)) + val(0xbaf643f4).map_or(0.0, |v| v.x.clamp(0.0, 0.99))],
        // Signs (VSCityNeon, VSCityBillboard, VSCityStoreSign): lit from the material's OnTime
        // to its OffTime; the neon map a flip-book of NumAnimsU x NumAnimsV frames stepped at
        // AnimSpeed. x: both times in eighths of an hour and the strength of the light in
        // eighths (PSCityNeon: twice NeonIntensity; PSCityBillboard: 1; PSCityStoreSign:
        // NightIntensity); y: the frame counts, the speed, and whether the map lies on the
        // second UV set (neon and shop signs) or the first (billboards).
        MatMode::Neon | MatMode::StoreSign => {
            let f = |h: u32, d: f32| val(h).map_or(d, |v| v.x);
            let billboard = sh.name.to_ascii_lowercase().contains("billboard");
            let gain = match mode {
                MatMode::StoreSign if !sh.textures().any(|(h, _)| h == 0xdc27608e) => 0.0,
                MatMode::StoreSign => f(0xcebc7801, 1.0),
                _ if billboard => 1.0,
                _ => 2.0 * f(0x2fed46c3, 1.0),
            };
            let q = |x: f32, k: f32| (x * k).round().clamp(0.0, 255.0);
            let (nu, nv) = (f(0x823d2dbc, 1.0).round().clamp(1.0, 7.0), f(0x69dc7cff, 1.0).round().clamp(1.0, 7.0));
            let speed = f(0xaf53f5a8, 0.0).round().clamp(0.0, 63.0);
            // Neon signs' highlight (VSCityNeon / PSCityNeon, the night PSMultiLight): the
            // FIRST colour word, which those programs do not read (their one colour input is
            // the night word), carries SpecularExponent, SpecularIntensity and
            // SpecularColorMixer in hundredths. (All 3335 CityNeon materials of the city:
            // exponent 8 / 15 / 25 / 175, intensity 0.3 / 0.33 / 0.5 / 0.7, mixer 0.25 / 0.5 /
            // 0.7 / 0.75, SpecularTint white. A material without them: no highlight.)
            if mode == MatMode::Neon && !billboard {
                let spec = [f(0x93d520e8, 1.0).round().clamp(1.0, 255.0) as u8, q(f(0xa95fc535, 0.0), 100.0) as u8, q(f(0xbaf643f4, 0.0), 100.0) as u8, 255];
                for v in vertices.iter_mut() {
                    v.color = spec;
                }
            }
            [
                q(f(0xeec362e8, 18.0), 8.0).max(1.0) + q(f(0xd60eac7d, 6.0), 8.0) * 256.0 + q(gain, 8.0) * 65536.0,
                nu + nv * 8.0 + speed * 64.0 + if billboard { 0.0 } else { 4096.0 },
            ]
        }
        // PSCityNeonTint: DayColor in x; NightColor (6 bits a channel) and NightIntensity (in
        // quarters) in y
        MatMode::NeonTint => {
            let c = |h: u32| val(h).map_or(glam::Vec3::ONE, |v| v.truncate()).clamp(glam::Vec3::ZERO, glam::Vec3::ONE);
            let (d, n) = (c(0xcb39572b), c(0x7e2ff13e));
            let q = |x: f32, k: f32| (x * k).round();
            let i = (val(0xcebc7801).map_or(1.0, |v| v.x) * 4.0).round().clamp(0.0, 31.0);
            [q(d.x, 127.0) + q(d.y, 127.0) * 128.0 + q(d.z, 127.0) * 16384.0, q(n.x, 63.0) + q(n.y, 63.0) * 64.0 + q(n.z, 63.0) * 4096.0 + i * 262144.0]
        }
        _ => return,
    };
    if std::env::var("MCLA_WIN_DUMP").is_ok() && mode == MatMode::Window {
        let f = |g: fn(&Vertex) -> [f32; 2]| vertices.iter().fold([f32::MAX, f32::MIN, f32::MAX, f32::MIN], |a, v| { let u = g(v); [a[0].min(u[0]), a[1].max(u[0]), a[2].min(u[1]), a[3].max(u[1])] });
        eprintln!("win {} uv0 {:?} uv1 {:?} uv2 {:?} {:?}", sh.source_path, f(|v| v.uv0), f(|v| v.uv1), uv2, sh.textures().map(|t| t.1.to_string()).collect::<Vec<_>>());
    }
    for v in vertices.iter_mut() {
        v.uv2 = uv2;
    }
}

/// The third and fourth texture of a material: the grime layer of the ground shaders, the
/// second and third texture of a terrain blend.
fn extra_textures(sh: &mcla_formats::rage::drawable::Shader, mode: MatMode) -> (u32, u32) {
    extra_textures_at(sh, mode, true)
}

/// `interior`: the batch carries flag 4 (see `mark_grime`). An OUTDOOR metal wall / tunnel tile
/// mirrors `ReflectSphereDay.dds` by day (PSCityGrimeMetal's ReflectionSampler) and
/// `ReflectSphereNight.dds` at night (its PSMultiLight's) - work/lighting/og_tile.py; a batch has
/// one slot left, so the day picture goes in and the night term stays out (city.wgsl `tile_out`).
fn extra_textures_at(sh: &mcla_formats::rage::drawable::Shader, mode: MatMode, interior: bool) -> (u32, u32) {
    let tex = |h: u32| sh.textures().find(|(ph, _)| *ph == h).map(|(_, n)| mcla_formats::hash::joaat(n));
    match mode {
        // (the road's puddle map, PuddleSampler: fourth)
        MatMode::Road => (tex(ph::GRIME).unwrap_or(0), tex(0xfe553678).unwrap_or(0)),
        // (the ground programs' bump and specular map, PSCityGroundGrime / ...Decal: fourth)
        MatMode::Ground => (tex(ph::GRIME).unwrap_or(0), tex(ph::HEIGHT_SPEC).unwrap_or(0)),
        // (metal walls and tunnel tiles drawn as interiors mirror a fixed picture: the sampler their
        // PSMultiLightInterior reads holds the same 512 x 512 texture at every hour, a third one
        // beside the panes' day and night pictures - `ReflectSphereInterior.dds`; fourth)
        MatMode::Opaque | MatMode::AlphaTest => (tex(ph::HEIGHT_SPEC).unwrap_or(0), {
            let l = sh.name.to_ascii_lowercase();
            if l.contains("grimemetal") || l.contains("tunneltile") { mcla_formats::hash::joaat(if interior { "reflectsphereinterior.dds" } else { "reflectsphereday.dds" }) } else { 0 }
        }),
        MatMode::StoreGlass => (mcla_formats::hash::joaat("reflectspherenight.dds"), 0),
        // (upper windows mirror the same fixed day picture as the shop panes, laid over the live map)
        // (and at night ReflectSphereNight.dds - their night program's ReflectionSampler: fourth)
        MatMode::Window => (mcla_formats::hash::joaat("reflectsphereday.dds"), mcla_formats::hash::joaat("reflectspherenight.dds")),
        // (walls with a decal AND a grime layer, CityGrimeDecal: the grime map, third)
        // (and their height / specular map, fourth - the highlight of PSCityGrimeDecal / its night program)
        MatMode::Decal => (tex(ph::GRIME).unwrap_or(0), tex(ph::HEIGHT_SPEC).unwrap_or(0)),
        // (the sea's ripple map, WaveSampler - read as stored: its fetch constant asks for no gamma decoding)
        MatMode::OceanWater | MatMode::OceanWaterLod => (tex(0x00a67acd).unwrap_or(0), 0),
        // (the pond water's WaveSampler - the same ripple map, also read as stored: third)
        MatMode::Blend if is_pond(&sh.name) => (tex(0x00a67acd).unwrap_or(0), 0),
        MatMode::Terrain => {
            let a = tex(ph::GRASS_A).unwrap_or(0);
            let b = tex(mcla_formats::hash::joaat("diffusesamplerb")).unwrap_or(a);
            (b, tex(ph::GRASS_C).unwrap_or(b))
        }
        _ => (0, 0),
    }
}

fn pick_textures(sh: &mcla_formats::rage::drawable::Shader, mode: MatMode) -> (u32, u32) {
    let tex = |h: u32| sh.textures().find(|(ph, _)| *ph == h).map(|(_, n)| mcla_formats::hash::joaat(n));
    let first = sh.textures().next().map(|(_, n)| mcla_formats::hash::joaat(n)).unwrap_or(0);
    // CityGrass: ChannelMapSampler + DiffuseSamplerA/B/C
    if mode == MatMode::Terrain {
        // (the channel map in the first slot, texture A in the second; B and C follow in `extra_textures`)
        let h = |n: &str| tex(mcla_formats::hash::joaat(n));
        if let Some(cm) = h("channelmapsampler") {
            return (cm, tex(ph::GRASS_A).unwrap_or(first));
        }
    }
    let diffuse = tex(ph::DIFFUSE).or_else(|| tex(ph::GRASS_A)).unwrap_or(first);
    let second = match mode {
        MatMode::Road | MatMode::Ground | MatMode::Decal => tex(ph::DECAL),
        MatMode::Neon => tex(ph::LIT),
        MatMode::Window => tex(ph::WINDOW_LIT),
        MatMode::StoreSign => tex(0xdc27608e), // NightSampler
        // (the sea: WaveFoamSampler second, WaveSampler third - `extra_textures_at`)
        MatMode::OceanWater | MatMode::OceanWaterLod => tex(0x63f7c0e8),
        // (shop panes mirror a fixed picture, the game's `__WindowReflection`: ReflectSphereDay.dds
        // of the shared dictionary by day - the night one goes into the third slot)
        MatMode::StoreGlass => Some(mcla_formats::hash::joaat("reflectsphereday.dds")),
        _ => tex(ph::LITUP).or_else(|| tex(ph::GRIME)),
    }
    .unwrap_or(diffuse);
    (diffuse, second)
}

/// Merge the batches of a sector that share textures and material mode (and lie in the
/// same 128 m cell, so frustum culling keeps working): thousands of tiny draws become a
/// few hundred.
fn merge_batches(batches: Vec<CpuBatch>) -> Vec<CpuBatch> {
    type BatchKey = (u32, u32, u32, u32, u32, i32, i32);
    let mut order: Vec<BatchKey> = Vec::new();
    let mut map: HashMap<BatchKey, CpuBatch> = HashMap::new();
    for b in batches {
        if b.indices.is_empty() || b.vertices.is_empty() {
            continue;
        }
        let c = (b.min + b.max) * 0.5;
        // Both texture identities must be compared. XORing them can merge
        // different terrain layers and bind the first material to both meshes.
        let key = (b.diffuse, b.second, b.third, b.fourth, b.mode as u32 + if b.interior { 1000 } else { 0 }, (c.x / 128.0).floor() as i32, (c.z / 128.0).floor() as i32);
        match map.get_mut(&key) {
            Some(acc) => {
                let base = acc.vertices.len() as u32;
                let n = b.vertices.len() as u32;
                let run = acc.indices.len() as u32;
                acc.vertices.extend(b.vertices);
                for t in b.indices.chunks_exact(3) {
                    if t.iter().all(|&i| i < n) {
                        acc.indices.extend(t.iter().map(|&i| i + base));
                    }
                }
                if let [p] = b.parts[..] {
                    acc.parts.push((run, acc.indices.len() as u32 - run, p.2, p.3, p.4));
                }
                acc.min = acc.min.min(b.min);
                acc.max = acc.max.max(b.max);
            }
            None => {
                order.push(key);
                map.insert(key, b);
            }
        }
    }
    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

/// Ids of interior sets (garages) start here; they are kept apart from the streamed sectors.
pub const SET_ID: usize = 1 << 40;
/// ids of the cells of the city's own collision in the collider
pub const BOUND_ID: usize = 1 << 41;

/// The city's collision as the original has it (`rage::bound`): one mesh a cell, each triangle
/// with its physics material. None when the file is not there.
pub fn city_bound(assets: &Assets) -> Option<Vec<SectorMesh>> {
    let dir = assets.path("resources/city/sc");
    // (the one resource of that class among the city's files)
    let file = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "r1")).find(|p| {
        use std::io::Read;
        let mut head = [0u8; 20];
        std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head[..4] == *b"MCLR" && u32::from_le_bytes([head[16], head[17], head[18], head[19]]) == 0x005c_9a50
    })?;
    let bound = match load_resource_file(&file).and_then(|r| mcla_formats::rage::bound::CityBound::parse(&r)) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("city bound {}: {:#}", file.display(), e);
            return None;
        }
    };
    let table = crate::physics::collision::Surface::table(&assets.root.to_string_lossy());
    let meshes: Vec<SectorMesh> = bound
        .cells
        .iter()
        .map(|cell| {
            let tris = cell.tris.iter().map(|t| [Vec3::from(cell.verts[t[0] as usize]), Vec3::from(cell.verts[t[1] as usize]), Vec3::from(cell.verts[t[2] as usize])]).collect();
            let mats = cell.material.iter().map(|&id| table.get((id & 0xff) as usize).copied().unwrap_or(crate::physics::collision::Surface::Default) as u8).collect();
            let mut mesh = SectorMesh::of_bound(tris, mats, &cell.area, cell.neighbours.clone());
            mesh.cell = Some(crate::physics::collision::GridCell { x: cell.grid.0, z: cell.grid.1, inv_size: bound.inv_size, margin: bound.margin });
            mesh
        })
        .collect();
    // (MCLA_DBG_BOUNDAT=x,y,z: the triangles within half a metre of that place, with what the
    // file names as their neighbours)
    if let Some(at) = std::env::var("MCLA_DBG_BOUNDAT").ok().map(|v| v.split(',').filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<f32>>()).filter(|v| v.len() == 3) {
        let at = Vec3::new(at[0], at[1], at[2]);
        for cell in &bound.cells {
            let corners = |t: &[u32; 3]| [Vec3::from(cell.verts[t[0] as usize]), Vec3::from(cell.verts[t[1] as usize]), Vec3::from(cell.verts[t[2] as usize])];
            let normal = |c: &[Vec3; 3]| (c[1] - c[0]).cross(c[2] - c[0]).normalize_or_zero();
            for (i, t) in cell.tris.iter().enumerate() {
                let c = corners(t);
                let (mn, mx) = (c[0].min(c[1]).min(c[2]) - Vec3::splat(0.5), c[0].max(c[1]).max(c[2]) + Vec3::splat(0.5));
                if at.cmpge(mn).all() && at.cmple(mx).all() {
                    eprintln!("BOUND tri {i} {:.3?} n {:.3?} neighbours {:?}", c, normal(&c), cell.neighbours[i].map(|n| n.map(|n| (n, normal(&corners(&cell.tris[n as usize]))))));
                }
            }
        }
    }
    log::info!("city bound: {} cells, {} triangles", meshes.len(), bound.triangle_count());
    Some(meshes)
}

/// Interior sets (`resources/cutsceneitems/*_set_items.r102`: the garages): drawables
/// in world coordinates with the city's shaders and their own texture dictionary.
pub fn decode_set(id: usize, path: &Path) -> Result<CpuSector> {
    let res = load_resource_file(path)?;
    let d = mcla_formats::rage::scan::Drawable::parse(&res)?;
    let rd = mcla_formats::rage::Rd::new(&res);
    let mut batches = Vec::new();
    let mut collision: Vec<[Vec3; 3]> = Vec::new();
    for model in &d.models {
        for (m, si) in &model.parts {
            let n = m.positions.len();
            let Some(sh) = d.shader_group.shaders.get(*si as usize) else { continue };
            let sname = sh.name.to_ascii_lowercase();
            if !(sname.contains("neon") || sname.contains("glass") || sname.contains("window") || sname.contains("alpha") || sname.contains("decal")) {
                for t in m.indices.chunks_exact(3) {
                    let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                    if a < n && b < n && c < n {
                        collision.push([m.positions[a], m.positions[b], m.positions[c]]);
                    }
                }
            }
            let mut vertices = Vec::with_capacity(n);
            for i in 0..n {
                let nrm = m.normals.get(i).copied().unwrap_or(Vec3::Y);
                let nrm = if nrm.is_finite() && nrm.length_squared() > 1e-4 { nrm.normalize() } else { Vec3::Y };
                let uv0 = m.uv0.get(i).copied().unwrap_or_default();
                let uv1 = m.uv1.get(i).copied().unwrap_or_default();
                let c = m.colors.get(i).copied().unwrap_or([255, 255, 255, 255]);
                // (a mesh with one colour set carries its baked night light and occlusion there - the
                // one-colour programs, CityDefault, CityBumpSpec ..., read rgb = night, a = occlusion from it)
                let c1 = m.colors1.get(i).copied().unwrap_or(c);
                vertices.push(Vertex { pos: m.positions[i].to_array(), normal: nrm.to_array(), uv0: uv0.to_array(), uv1: uv1.to_array(), color: [c[1], c[2], c[3], c[0]], color1: [c1[1], c1[2], c1[3], c1[0]], uv2: m.uv2.get(i).copied().unwrap_or(uv1).to_array() });
            }
            let (mn, mx) = m.bbox();
            if std::env::var("MCLA_SECTOR_LIST").map_or(false, |v| v == "all") {
                sector_list_line(format!("sectorlist {} shader {} flags {:#x} idx {} verts {} min {:.1} {:.1} {:.1} max {:.1} {:.1} {:.1}", 100000 + id - SET_ID, sh.name, 0, m.indices.len(), n, mn.x, mn.y, mn.z, mx.x, mx.y, mx.z));
            }
            let mode = classify(&sh.name, 0);
            let (diffuse, second) = pick_textures(sh, mode);
            let (mut third, fourth) = extra_textures(sh, mode);
            // (garage round, 2026-10-13: the garages' rooms are INTERIOR walls - every CityBumpSpec / CityGrime /
            // CityDefault batch of the Hollywood garage is drawn with PSMultiLightInterior in the original's garage
            // frames, work/lighting/cap/gar_*.bin: 174 + 54 + 24 draws - where the port lit them as outdoor night
            // walls, with the ambient and the lamps at 1.5. MCLA_SET_INTERIOR=0: as before.)
            mark_grime(&mut vertices, sh, mode, std::env::var("MCLA_SET_INTERIOR").map_or(true, |v| v != "0")); mark_window(&mut vertices, sh, mode);
            let gfloor = sname.contains("citygaragefloor") && matches!(mode, MatMode::Opaque | MatMode::AlphaTest);
            let mut second = second;
            if gfloor {
                // (its program blends the DecalSampler over the diffuse by the decal's alpha, on the second
                // texture coordinates - `mark_garage_floor`)
                if let Some((_, n)) = sh.textures().find(|(h, _)| *h == ph::DECAL) {
                    second = mcla_formats::hash::joaat(n);
                }
                mark_garage_floor(&mut vertices, sh);
            }
            mark_lod(&mut vertices, sh, mode, &mut third);
            mark_binormal(&mut vertices, &m.tangents, sh);
            if gfloor {
                mark_binormal_any(&mut vertices, &m.tangents);
            }
            mark_water(&mut vertices, mode);
            mark_pond(&mut vertices, sh, mode);
            batches.push(CpuBatch { vertices, indices: m.indices.clone(), diffuse, second, third, fourth, mode, min: mn, max: mx, shader: sh.name.clone(), interior: false, parts: Vec::new() });
        }
    }
    let batches = merge_batches(batches);
    let mut textures = Vec::new();
    for (h, o) in &d.texture_offsets {
        if let Ok(t) = Texture::parse(&rd, *o) {
            textures.push((*h, t));
        }
    }
    Ok(CpuSector { id, batches, textures, collision, collision_mat: Vec::new(), geo: None })
}

/// (diagnostics, MCLA_SECTOR_LIST: one line a stored batch, appended to work/lighting/out/sectorlist.txt -
/// the redirected stderr of a run loses lines written from the loader's threads)
fn sector_list_line(s: String) {
    use std::io::Write;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("out/sectorlist.txt") {
        let _ = writeln!(f, "{}", s);
    }
}

pub fn decode_sector(id: usize, path: &Path) -> Result<CpuSector> {
    let res = load_resource_file(path)?;
    let sec = Sector::parse(&res)?;
    let mut batches = Vec::with_capacity(sec.batches.len());
    let mut collision: Vec<[Vec3; 3]> = Vec::new();
    let mut collision_mat: Vec<u8> = Vec::new();
    for b in &sec.batches {
        let m = &b.mesh;
        let n = m.positions.len();
        {
            let sname = sec.shaders[b.shader].name.to_ascii_lowercase();
            use crate::physics::collision::Surface;
            let surf = Surface::from_material(&sec.shaders[b.shader].source_path, &sec.shaders[b.shader].name, sec.shaders[b.shader].diffuse().unwrap_or(""));
            if std::env::var("MCLA_SURF_DUMP").is_ok() {
                log::info!("surf {:?} <- {} [{}]", surf, sec.shaders[b.shader].source_path.rsplit("/mtl/").next().unwrap_or(""), sec.shaders[b.shader].name);
            }
            // (the grass tufts and other plant cards set into a sector - materials named after the
            // la_prop_gfx plants - are for the eye: a recorded run of the original drives its flank
            // through one by the garage door without losing any speed)
            let plant = sec.shaders[b.shader].source_path.rsplit('/').next().unwrap_or("").to_ascii_lowercase().starts_with("la_prop_gfx_");
            let solid = !plant && !(sname.contains("neon") || sname.contains("glass") || sname.contains("water") || sname.contains("tree") || sname.contains("foliage") || sname.contains("sky") || sname.contains("cloud") || sname.contains("light") || sname.contains("alpha"));
            if solid {
                for t in m.indices.chunks_exact(3) {
                    let (a, bb, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                    if a < n && bb < n && c < n {
                        let tri = [m.positions[a], m.positions[bb], m.positions[c]];
                        // (the flat parts of structures - decks, ramps, roofs one can drive on -
                        // are plain ground; only what stands is wall)
                        let s = if surf == Surface::Wall && (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero().y.abs() > 0.5 { Surface::Exterior } else { surf };
                        collision.push(tri);
                        collision_mat.push(s as u8);
                    }
                }
            }
        }
        let mut vertices = Vec::with_capacity(n);
        for i in 0..n {
            let nrm = m.normals.get(i).copied().unwrap_or(Vec3::Y);
            // some meshes store garbage (non-finite) normals: light them as facing up
            // (lighting, 2026-10-15: a packed normal word of ZERO - 7 % of the cut-out bump-spec batches' vertices,
            // the dry weeds at the walls' feet - is still the vertical for every reader that normalises, but at a
            // quarter length: the mark city.wgsl's vs_body takes for "no normal", see fs_main "WITHOUT a stored normal")
            let nrm = if nrm.is_finite() && nrm.length_squared() > 1e-4 { nrm.normalize() } else if nrm.is_finite() { Vec3::Y * 0.25 } else { Vec3::Y };
            let uv0 = m.uv0.get(i).copied().unwrap_or_default();
            let uv1 = m.uv1.get(i).copied().unwrap_or_default();
            let c = m.colors.get(i).copied().unwrap_or([255, 255, 255, 255]);
            // (a mesh with one colour set carries its baked night light and occlusion there - the
                // one-colour programs, CityDefault, CityBumpSpec ..., read rgb = night, a = occlusion from it)
                let c1 = m.colors1.get(i).copied().unwrap_or(c);
            vertices.push(Vertex {
                pos: m.positions[i].to_array(),
                normal: nrm.to_array(),
                uv0: uv0.to_array(),
                uv1: uv1.to_array(),
                // stored A,R,G,B big-endian → use R,G,B,A
                color: [c[1], c[2], c[3], c[0]],
                color1: [c1[1], c1[2], c1[3], c1[0]],
                uv2: m.uv2.get(i).copied().unwrap_or(uv1).to_array(),
            });
        }
        let (mn, mx) = m.bbox();
        let sh = &sec.shaders[b.shader];
        let mode = classify(&sh.name, b.flags);
        let (diffuse, second) = pick_textures(sh, mode);
        // diagnostics: MCLA_SECTOR_LIST="126,132" lists those sectors' batches as stored
        if std::env::var("MCLA_SECTOR_LIST").map_or(false, |v| v == "all" || v.split(',').any(|x| x.trim().parse::<usize>().ok() == Some(id))) {
            sector_list_line(format!("sectorlist {} shader {} flags {:#x} idx {} verts {} min {:.1} {:.1} {:.1} max {:.1} {:.1} {:.1}", id, sh.name, b.flags, m.indices.len(), n, mn.x, mn.y, mn.z, mx.x, mx.y, mx.z));
        }
        // diagnostics: MCLA_SHADER_AT="x,z,r" lists the small meshes around a point
        if let Ok(v) = std::env::var("MCLA_SHADER_AT") {
            let f: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
            let c = (mn + mx) * 0.5;
            if f.len() == 3 && f[0] > mn.x - f[2] && f[0] < mx.x + f[2] && f[1] > mn.z - f[2] && f[1] < mx.z + f[2] {
                log::info!("mesh near: shader {} flags {:#x} mode {:?} size {:.1?} at {:.0?} tex {:?}", sh.name, b.flags, mode, mx - mn, c, sh.textures().map(|(_, n)| n.to_string()).collect::<Vec<_>>());
                // (which of them the sector's own dictionary holds, with size, format and fetch words)
                for (_, n) in sh.textures() {
                    let h = mcla_formats::hash::joaat(n);
                    if let Some(t) = sec.textures.get(&h) {
                        log::info!("    own texture {} {} x {} {:?} levels {} fetch {:08x?} first bytes {:02x?}", n, t.width, t.height, t.format, t.levels.len(), t.fetch, &t.levels[0][..t.levels[0].len().min(16)]);
                    }
                }
            }
        }
        let (mut third, fourth) = extra_textures_at(sh, mode, b.flags & 4 != 0);
        mark_grime(&mut vertices, sh, mode, b.flags & 4 != 0); mark_window(&mut vertices, sh, mode);
        mark_lod(&mut vertices, sh, mode, &mut third);
        mark_tunnel_road(&mut vertices, sh, mode, b.flags & 4 != 0);
        mark_binormal(&mut vertices, &m.tangents, sh);
        mark_water(&mut vertices, mode);
        mark_pond(&mut vertices, sh, mode);
        batches.push(CpuBatch {
            vertices,
            indices: m.indices.clone(),
            diffuse,
            second,
            third,
            fourth,
            mode,
            min: mn,
            max: mx,
            shader: sh.name.clone(),
            interior: b.flags & 4 != 0 && !matches!(mode, MatMode::Opaque | MatMode::AlphaTest),
            parts: vec![(0, m.indices.len() as u32, (mn + mx) * 0.5, (mx - mn).length() * 0.5, b.flags)],
        });
    }
    let batches = merge_batches(batches);
    let textures = sec.textures.into_iter().collect();
    Ok(CpuSector { id, batches, textures, collision, collision_mat, geo: None })
}

/// Ids of the far city's sector models while they are shown (among `World::sets`):
/// `FAR_ID + model * FAR_SLOTS + sector` (sector `FAR_SLOTS - 1`: none).
pub const FAR_ID: usize = 1 << 42;
const FAR_SLOTS: usize = 4096;

/// The far city, `city/sc/0xa6e77868.r1` (class 0x596410): root +08 -> shader group (625
/// shaders: CityLOD with a day and a night picture of the sector from above, CityWindowLOD ...;
/// the textures are in the shared city dictionary), +0c -> 439 model pointers (u16 count at
/// +10; one is null), each one sector's simple shape in world coordinates. A model is matched to
/// the sector it stands for by how far their boxes overlap. 382,734 triangles in all.
pub fn decode_far(path: &Path, sectors: &[SectorInfo]) -> Result<Vec<CpuSector>> {
    use mcla_formats::rage::drawable::Shader;
    let res = load_resource_file(path)?;
    let rd = Rd::new(&res);
    let mut shaders: Vec<Option<Shader>> = Vec::new();
    if let Some(g) = rd.ptr(0x08) {
        if let Some(arr) = rd.ptr(g + 0x08) {
            shaders = (0..rd.u16(g + 0x0c) as usize).map(|i| rd.ptr(arr + i * 4).and_then(|so| Shader::parse(&rd, so).ok())).collect();
        }
    }
    let (Some(arr), n) = (rd.ptr(0x0c), rd.u16(0x10) as usize) else { anyhow::bail!("far city: no models") };
    let mut out = Vec::new();
    for i in 0..n {
        let Some(m) = rd.ptr(arr + i * 4) else { continue };
        let (Some(ga), gc) = (rd.ptr(m + 0x04), rd.u16(m + 0x08) as usize) else { continue };
        let si = rd.ptr(m + 0x10);
        let mut batches = Vec::new();
        for g in 0..gc {
            let Some(go) = rd.ptr(ga + g * 4) else { continue };
            let Ok(mesh) = mcla_formats::rage::geometry::parse_geometry(&rd, go) else { continue };
            let Some(Some(sh)) = shaders.get(si.map_or(0, |p| rd.u16(p + g * 2)) as usize) else { continue };
            let n = mesh.positions.len();
            let mut vertices = Vec::with_capacity(n);
            for v in 0..n {
                let nrm = mesh.normals.get(v).copied().unwrap_or(Vec3::Y);
                let nrm = if nrm.is_finite() && nrm.length_squared() > 1e-4 { nrm.normalize() } else { Vec3::Y };
                let uv0 = mesh.uv0.get(v).copied().unwrap_or_default();
                let uv1 = mesh.uv1.get(v).copied().unwrap_or(uv0);
                let c = mesh.colors.get(v).copied().unwrap_or([255, 255, 255, 255]);
                let c1 = mesh.colors1.get(v).copied().unwrap_or(c);
                vertices.push(Vertex { pos: mesh.positions[v].to_array(), normal: nrm.to_array(), uv0: uv0.to_array(), uv1: uv1.to_array(), color: [c[1], c[2], c[3], c[0]], color1: [c1[1], c1[2], c1[3], c1[0]], uv2: uv1.to_array() });
            }
            let (mn, mx) = mesh.bbox();
            let mode = classify(&sh.name, 0);
            let (diffuse, second) = pick_textures(sh, mode);
            let (mut third, fourth) = extra_textures(sh, mode);
            mark_grime(&mut vertices, sh, mode, false);
            mark_window(&mut vertices, sh, mode);
            mark_lod(&mut vertices, sh, mode, &mut third);
            mark_binormal(&mut vertices, &mesh.tangents, sh);
            mark_water(&mut vertices, mode);
            mark_pond(&mut vertices, sh, mode);
            batches.push(CpuBatch { vertices, indices: mesh.indices, diffuse, second, third, fourth, mode, min: mn, max: mx, shader: sh.name.clone(), interior: false, parts: Vec::new() });
        }
        let batches = merge_batches(batches);
        if batches.is_empty() {
            continue;
        }
        let (mn, mx) = batches.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |a, b| (a.0.min(b.min), a.1.max(b.max)));
        // the sector whose box (seen from above) shares the most with the model's
        let mut best = (0.25f32, FAR_SLOTS - 1);
        for (j, s) in sectors.iter().enumerate().take(FAR_SLOTS - 1) {
            let (ix, iz) = (mx.x.min(s.max[0]) - mn.x.max(s.min[0]), mx.z.min(s.max[2]) - mn.z.max(s.min[2]));
            if ix <= 0.0 || iz <= 0.0 {
                continue;
            }
            let inter = ix * iz;
            let iou = inter / ((mx.x - mn.x) * (mx.z - mn.z) + (s.max[0] - s.min[0]) * (s.max[2] - s.min[2]) - inter);
            if iou > best.0 {
                best = (iou, j);
            }
        }
        out.push(CpuSector { id: FAR_ID + i * FAR_SLOTS + best.1, batches, textures: Vec::new(), collision: Vec::new(), collision_mat: Vec::new(), geo: None });
    }
    Ok(out)
}

pub struct World {
    pub index: WorldIndex,
    pub assets: Arc<Assets>,
    pub shared: Arc<SharedTextures>,
    pub loaded: HashMap<usize, GpuSector>,
    /// interior sets (garages), always resident once loaded
    pub sets: HashMap<usize, GpuSector>,
    /// bounding box of each interior set
    pub set_bounds: HashMap<usize, (Vec3, Vec3)>,
    /// the far city's sector models that are not shown now (their sector itself is loaded);
    /// the shown ones are among `sets`
    far_hidden: HashMap<usize, GpuSector>,
    last_fade: std::time::Instant,
    /// what is left of this frame's upload time (`new_frame`), shared by the frame's ticks
    frame_left_ms: f32,
    pub pending: hashbrown::HashSet<usize>,
    tx: Sender<Result<CpuSector>>,
    rx: Receiver<Result<CpuSector>>,
    pub load_radius: f32,
    pub unload_radius: f32,
    pub tex_fail: hashbrown::HashSet<u32>,
    pub stats_tris: u64,
    pub collider: WorldCollider,
    /// the collider holds the city's own collision: the drawn sectors add none
    pub own_bound: bool,
    pub globaltex: HashMap<u32, PathBuf>,
    pub nav: Option<NavSys>,
}

impl World {
    pub fn new(assets: Arc<Assets>) -> Result<World> {
        let index = WorldIndex::load_or_build(&assets)?;
        let mut shared_t = SharedTextures::load(&assets, &index.dicts);
        shared_t.add_dir(&assets.path("resources/tires/textures"));
        shared_t.add_dir(&assets.path("resources/brakes"));
        shared_t.add_dir(&assets.path("resources/license_plates"));
        let shared = Arc::new(shared_t);
        // diagnostics: MCLA_TEX_FIND=name,name... - which of these the shared dictionaries hold
        if let Ok(v) = std::env::var("MCLA_TEX_FIND") {
            for n in v.split(',') {
                match shared.get(mcla_formats::hash::joaat(&n.to_ascii_lowercase())) {
                    Some(t) => {
                        eprintln!("texture {n}: {} x {} {:?} levels {} fetch {:08x?}", t.width, t.height, t.format, t.levels.len(), t.fetch);
                        // (MCLA_TEX_FIND_OUT=dir: the first level's bytes as the port holds them)
                        if let Ok(d) = std::env::var("MCLA_TEX_FIND_OUT") { for (li, l) in t.levels.iter().enumerate() { let _ = std::fs::write(std::path::Path::new(&d).join(format!("{n}.{li}.bin")), l); } }
                    }
                    None => eprintln!("texture {n}: not in the shared dictionaries"),
                }
            }
        }
        // road network: the largest .r66 in city/sc
        let mut nav = None;
        if let Ok(rd) = std::fs::read_dir(assets.path("resources/city/sc")) {
            let mut best: Option<(u64, PathBuf)> = None;
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map_or(false, |x| x == "r66") {
                    let sz = e.metadata().map(|m| m.len()).unwrap_or(0);
                    if best.as_ref().map_or(true, |b| sz > b.0) {
                        best = Some((sz, p));
                    }
                }
            }
            if let Some((_, p)) = best {
                match load_resource_file(&p).and_then(|r| NavSys::parse(&r)) {
                    Ok(n) => {
                        log::info!("navsys: {} links, {} zones, {} streets", n.links.len(), n.zones.len(), n.streets.len());
                        if std::env::var("MCLA_STREETS").is_ok() {
                            let mut names: Vec<String> = n.zones.iter().map(|z| z.name.clone()).collect();
                            names.sort();
                            names.dedup();
                            log::info!("zone names ({}): {:?}", names.len(), names);
                            let mut st: Vec<String> = n.streets.iter().map(|z| z.name.clone()).collect();
                            st.sort();
                            st.dedup();
                            log::info!("street names ({}): {:?}", st.len(), st);
                        }
                        nav = Some(n);
                    }
                    Err(e) => log::warn!("navsys: {:#}", e),
                }
            }
        }
        let (tx, rx) = channel();
        // the far city, read in the background (MCLA_NO_FARCITY leaves it out)
        if std::env::var("MCLA_NO_FARCITY").is_err() {
            let (tx, path, sectors) = (tx.clone(), assets.path("resources/city/sc/0xa6e77868.r1"), index.sectors.clone());
            std::thread::spawn(move || match decode_far(&path, &sectors) {
                Ok(list) => {
                    log::info!("far city: {} sector models, {} triangles", list.len(), list.iter().flat_map(|c| c.batches.iter()).map(|b| b.indices.len() / 3).sum::<usize>());
                    for cs in list {
                        let _ = tx.send(Ok(cs));
                    }
                }
                Err(e) => log::warn!("far city: {:#}", e),
            });
        }
        // the city's own collision, whole, from the start (MCLA_MESH_COLLISION: the drawn
        // sectors' triangles instead, as before it was found)
        let mut collider = WorldCollider::default();
        let mut own_bound = false;
        // (stage P4: the level's loose props are given to the vehicles' physics, which then meets them as the
        // original does, physics/loose.rs - one level for every car, vehicles.rs `level_swap`; props.rs leaves
        // those props to it and draws them where their bodies are. MCLA_NO_LOOSE_PHYSICS: props.rs's own knock.)
        if std::env::var_os("MCLA_NO_LOOSE_PHYSICS").is_none() {
            collider.loose = crate::physics::loose::LooseWorld::load(&assets.path("resources/prop/0x0e66973c.r131"));
        }
        // (without them the level still holds the cars' torn-off bumpers: their type only, no prop - nothing else
        // of the vehicles' physics changes while a car's level has no object, damageparts.rs)
        if collider.loose.is_none() {
            collider.loose = Some(std::sync::Arc::new(crate::physics::loose::LooseWorld::new(Vec::new(), Vec::new())));
        }
        if std::env::var("MCLA_MESH_COLLISION").is_err() {
            if let Some(cells) = city_bound(&assets) {
                for (i, mesh) in cells.into_iter().enumerate() {
                    collider.sectors.insert(BOUND_ID + i, mesh);
                }
                own_bound = true;
            }
        }
        Ok(World {
            index,
            assets,
            shared,
            loaded: HashMap::new(),
            sets: HashMap::new(),
            set_bounds: HashMap::new(),
            far_hidden: HashMap::new(),
            last_fade: std::time::Instant::now(),
            frame_left_ms: f32::INFINITY,
            pending: Default::default(),
            tx,
            rx,
            load_radius: 450.0,
            unload_radius: 540.0,
            tex_fail: Default::default(),
            stats_tris: 0,
            collider,
            own_bound,
            globaltex: HashMap::new(),
            nav,
        })
    }

    /// Index textures/global/cars/globaltex (plain DDS files named by hash).
    pub fn index_globaltex(&mut self) {
        let dir = self.assets.path("textures/global/cars/globaltex");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                let h = if let Some(hex) = name.strip_prefix("0x") {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    Some(mcla_formats::hash::joaat(&name))
                };
                if let Some(h) = h {
                    self.globaltex.insert(h, p);
                }
            }
        }
        log::info!("globaltex: {} textures", self.globaltex.len());
    }

    /// Texture by name hash from any source: GPU cache, globaltex DDS, shared dictionaries.
    pub fn texture_any(&mut self, gpu: &mut Gpu, h: u32) -> Option<Arc<GpuTexture>> {
        if let Some(t) = gpu.textures.get(&h) {
            return Some(t.clone());
        }
        if let Some(p) = self.globaltex.get(&h) {
            if let Ok(d) = std::fs::read(p) {
                if let Ok(t) = mcla_formats::dds::parse_dds(&p.to_string_lossy(), &d) {
                    return Some(gpu.upload_texture(h, &t));
                }
            }
        }
        if let Some(t) = self.shared.get(h) {
            return Some(gpu.upload_texture(h, &t));
        }
        None
    }

    /// `texture_any` as stored, not gamma-decoded, under the key `h ^ 0x0052_4157` (the cars' maps, vehicles.rs)
    pub fn texture_any_raw(&mut self, gpu: &mut Gpu, h: u32) -> Option<Arc<GpuTexture>> {
        let key = h ^ 0x0052_4157;
        if let Some(t) = gpu.textures.get(&key) {
            return Some(t.clone());
        }
        if let Some(p) = self.globaltex.get(&h) {
            if let Ok(d) = std::fs::read(p) {
                if let Ok(t) = mcla_formats::dds::parse_dds(&p.to_string_lossy(), &d) {
                    return Some(gpu.upload_texture_raw(key, &t));
                }
            }
        }
        if let Some(t) = self.shared.get(h) {
            return Some(gpu.upload_texture_raw(key, &t));
        }
        None
    }

    /// Are the sectors around `p` resident (so physics can run)?
    pub fn ready_at(&self, p: Vec3) -> bool {
        self.index.sectors.iter().enumerate().all(|(id, s)| Self::dist_to(s, p) > 60.0 || self.loaded.contains_key(&id))
    }

    fn dist_to(info: &SectorInfo, p: Vec3) -> f32 {
        let mn = Vec3::from(info.min);
        let mx = Vec3::from(info.max);
        let c = p.clamp(mn, mx);
        Vec3::new(c.x - p.x, 0.0, c.z - p.z).length()
    }

    /// Kick off loads for sectors near `pos`, drop far ones, upload finished ones.
    /// Sectors still being loaded.
    pub fn streaming(&self) -> usize {
        self.pending.len()
    }

    /// The original's detail-model distance of a sector (sub_822D5A30: `MaxHilodDrawDistance` 300 x the
    /// lens scale, 1.5 at a 25 degree lens .. 1.0 from 50 degrees) times the DETAIL LEVEL `detail`.
    pub fn detail_distance(lens_deg: f32, detail: f32) -> f32 {
        300.0 * (1.5 - 0.5 * ((lens_deg - 25.0) * 0.04).clamp(0.0, 1.0)) * detail.max(1.0)
    }

    /// A rendered frame starts: the uploads of its ticks share one `budget_ms` of `update` (each tick had its own: after a
    /// long frame its catch-up ticks spent 6 ms each, 25 - 35 ms, and made the next frame long too - docs/PERFORMANCE.md
    /// 2026-10-27 (8)).
    pub fn new_frame(&mut self, budget_ms: f32) {
        self.frame_left_ms = budget_ms;
    }

    pub fn update(&mut self, gpu: &mut Gpu, pos: Vec3, budget_ms: f32) {
        let budget_ms = budget_ms.min(self.frame_left_ms);
        // The original draws a sector in detail while the level distance from the camera to its
        // box is under 300 m times a lens scale - 1.5 at the 25 degree lens of a standing car,
        // falling to 1 at 50 degrees (450 m .. 300 m) - and its far model beyond; it keeps
        // sectors in memory to 600 m (sub_822D5A30 / sub_822D7D00, work/map/lod_rule.md). Here
        // "loaded" is "drawn in detail", so the load distance is that rule (plus a margin before
        // a sector is dropped again). The original cross-fades the two over a third of a second;
        // here they swap. MCLA_SECTOR_DIST=<metres> overrides. OPTIONS > PERFORMANCE > DETAIL LEVEL
        // multiplies the 300 m (MaxHilodDrawDistance, 0x827E0DE0) - and so what is loaded with it;
        // ORIGINAL = 1. (DRAW DISTANCE has nothing to scale here: the far models are drawn at any distance.)
        let lens_deg = crate::props::lens().to_degrees();
        let detail = Self::detail_distance(lens_deg, crate::props::Lod::now().detail);
        let detail = std::env::var("MCLA_SECTOR_DIST").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(detail);
        // (the lens widens with speed: keep to the larger distance for a while rather than drop and reload)
        self.load_radius = detail.max(self.load_radius - 2.0);
        self.unload_radius = self.load_radius + 90.0;
        // unload
        let far: Vec<usize> = self
            .loaded
            .keys()
            .copied()
            .filter(|&id| Self::dist_to(&self.index.sectors[id], pos) > self.unload_radius)
            .collect();
        // (a sector that is out of range fades out against its far model before it is dropped; one
        // that comes back in range meanwhile stays)
        for s in self.loaded.values_mut() {
            s.leaving = false;
        }
        for id in far {
            if let Some(s) = self.loaded.get_mut(&id) {
                s.leaving = true;
            }
        }
        // request loads, nearest first
        let mut want: Vec<(f32, usize)> = self
            .index
            .sectors
            .iter()
            .enumerate()
            .filter(|(id, _)| !self.loaded.contains_key(id) && !self.pending.contains(id))
            .map(|(id, s)| (Self::dist_to(s, pos), id))
            .filter(|(d, _)| *d < self.load_radius)
            .collect();
        want.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let max_inflight: usize = 12;
        for (_, id) in want.into_iter().take(max_inflight.saturating_sub(self.pending.len())) {
            self.pending.insert(id);
            let path = self.assets.path(&self.index.sectors[id].file);
            let tx = self.tx.clone();
            rayon::spawn(move || {
                let r = decode_sector(id, &path).map(|mut cs| {
                    cs.geo = Some(prepare_geo(std::mem::take(&mut cs.batches)));
                    cs
                });
                let _ = tx.send(r);
            });
        }
        // upload finished (time-boxed)
        let t0 = std::time::Instant::now();
        let (mut n_up, mut ms_up, mut ms_col) = (0, 0.0f32, 0.0f32);
        while let Ok(r) = self.rx.try_recv() {
            match r {
                Ok(cs) if cs.id >= FAR_ID => {
                    let id = cs.id;
                    let gs = self.upload(gpu, cs);
                    self.far_hidden.insert(id, gs);
                }
                Ok(cs) if cs.id >= SET_ID => {
                    let id = cs.id;
                    let col = cs.collision.clone();
                    let mut mn = Vec3::splat(f32::INFINITY);
                    let mut mx = Vec3::splat(f32::NEG_INFINITY);
                    for t in &col {
                        for p in t {
                            mn = mn.min(*p);
                            mx = mx.max(*p);
                        }
                    }
                    self.set_bounds.insert(id, (mn, mx));
                    let gs = self.upload(gpu, cs);
                    log::info!("interior set {}: {} batches, {} triangles, bounds {:.0?} .. {:.0?}", id - SET_ID, gs.batches.len(), gs.tris, gs.caster_min, gs.caster_max);
                    self.sets.insert(id, gs);
                    self.collider.sectors.insert(id, SectorMesh::build(col));
                }
                Ok(cs) => {
                    self.pending.remove(&cs.id);
                    if Self::dist_to(&self.index.sectors[cs.id], pos) <= self.unload_radius {
                        let id = cs.id;
                        let col = std::mem::take(&mut { cs.collision.clone() });
                        let mats = cs.collision_mat.clone();
                        let tu = std::time::Instant::now();
                        let gs = self.upload(gpu, cs);
                        ms_up += tu.elapsed().as_secs_f32() * 1000.0;
                        self.stats_tris += gs.tris as u64;
                        self.loaded.insert(id, gs);
                        if !self.own_bound {
                            let tc = std::time::Instant::now();
                            self.collider.sectors.insert(id, SectorMesh::build_with(col, mats));
                            ms_col += tc.elapsed().as_secs_f32() * 1000.0;
                        }
                        n_up += 1;
                    }
                }
                Err(e) => log::warn!("sector load failed: {:#}", e),
            }
            if t0.elapsed().as_secs_f32() * 1000.0 > budget_ms {
                break;
            }
        }
        self.frame_left_ms = (self.frame_left_ms - t0.elapsed().as_secs_f32() * 1000.0).max(0.0);
        if ms_up + ms_col > 8.0 {
            log::info!("streaming: {} sectors in, upload {:.1} ms, collision {:.1} ms", n_up, ms_up, ms_col);
        }
        // The cross-fade of a sector with its far model, as the original times it (a third of a
        // second; sub_822D7D00): with u running 0 -> 1 the sector's alpha is min(2u, 1) and the
        // far model's 1 - 2 max(u - 0.5, 0) - the sector comes in over the far model, then the far
        // model goes; the other way round on the way out.
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last_fade).as_secs_f32().min(0.1);
        self.last_fade = now;
        let mut gone: Vec<usize> = Vec::new();
        for (id, s) in self.loaded.iter_mut() {
            s.u = (s.u + if s.leaving { -3.0 } else { 3.0 } * dt).clamp(0.0, 1.0);
            let alpha = (2.0 * s.u).min(1.0);
            if alpha != s.shown {
                if let Some(o) = s.obj.as_ref() {
                    gpu.update_object(&o.0, &sector_uniform(alpha, false));
                    gpu.update_object(&o.1, &sector_uniform(alpha, true));
                }
                s.shown = alpha;
            }
            if s.leaving && s.u <= 0.0 {
                gone.push(*id);
            }
        }
        for id in gone {
            if let Some(s) = self.loaded.remove(&id) {
                self.stats_tris -= s.tris as u64;
            }
            if !self.own_bound {
                self.collider.sectors.remove(&id);
            }
        }
        // The far city: a sector's simple model is drawn while the sector itself is not fully in.
        let sector_of = |id: usize| (id - FAR_ID) % FAR_SLOTS;
        let alpha_of = |loaded: &HashMap<usize, GpuSector>, id: usize| loaded.get(&sector_of(id)).map_or(1.0, |s| 1.0 - 2.0 * (s.u - 0.5).max(0.0));
        let show: Vec<usize> = self.far_hidden.keys().copied().filter(|id| alpha_of(&self.loaded, *id) > 0.0).collect();
        for id in show {
            if let Some(gs) = self.far_hidden.remove(&id) {
                self.sets.insert(id, gs);
            }
        }
        let hide: Vec<usize> = self.sets.keys().copied().filter(|id| *id >= FAR_ID && alpha_of(&self.loaded, *id) <= 0.0).collect();
        for id in hide {
            if let Some(gs) = self.sets.remove(&id) {
                self.far_hidden.insert(id, gs);
            }
        }
        for (id, gs) in self.sets.iter_mut().filter(|e| *e.0 >= FAR_ID) {
            let alpha = alpha_of(&self.loaded, *id);
            if alpha != gs.shown {
                if let Some(o) = gs.obj.as_ref() {
                    gpu.update_object(&o.0, &sector_uniform(alpha, false));
                    gpu.update_object(&o.1, &sector_uniform(alpha, true));
                }
                gs.shown = alpha;
            }
        }
    }

    /// Every far-city sector model that is uploaded, shown or not (the fast shadow collector's
    /// pass draws them all the time, `render::Gpu::collector_pass`).
    pub fn far_models(&self) -> impl Iterator<Item = &GpuSector> {
        self.sets.iter().filter(|e| *e.0 >= FAR_ID).map(|e| e.1).chain(self.far_hidden.values())
    }

    /// The same with the index of the sector each stands for (`usize::MAX`: none): the cars'
    /// environment map picks detail or far model by a distance rule of its own.
    pub fn far_models_by_sector(&self) -> impl Iterator<Item = (usize, &GpuSector)> {
        self.sets.iter().filter(|e| *e.0 >= FAR_ID).chain(self.far_hidden.iter()).map(|(id, gs)| {
            let s = (*id - FAR_ID) % FAR_SLOTS;
            (if s == FAR_SLOTS - 1 { usize::MAX } else { s }, gs)
        })
    }

    /// Load an interior set in the background.
    pub fn request_set(&self, k: usize, path: PathBuf) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(decode_set(SET_ID + k, &path));
        });
    }

    fn upload(&mut self, gpu: &mut Gpu, mut cs: CpuSector) -> GpuSector {
        let sector_id = cs.id;
        let Geo { mut verts, indices, items } = match cs.geo.take() {
            Some(g) => g,
            None => prepare_geo(std::mem::take(&mut cs.batches)),
        };
        // local textures first
        let mut local: HashMap<u32, Arc<GpuTexture>> = HashMap::new();
        // An interior set's OWN textures (the garages' dictionaries) keep to the set: the GPU's
        // texture and bind group caches go by the name's hash alone, and the garages carry
        // textures named like the city's with other content - their `l_sha_roof_top.dds` is a dark
        // grey (first block 630c / 4228) where the city dictionary's is the light concrete (a513 /
        // 94b2, mean 154) the original draws on the shop roofs by the garage gate (capture pr_L15,
        // draw 3954). Whichever was uploaded first won for both. Their keys get a mark of their own.
        const SET_TEX_KEY: u32 = 0x5345_5421;
        let own: std::collections::HashSet<u32> = if sector_id >= SET_ID && sector_id < FAR_ID { cs.textures.iter().map(|(h, _)| *h).collect() } else { Default::default() };
        let sk = |h: u32| if own.contains(&h) { h ^ SET_TEX_KEY } else { h };
        for (h, t) in &cs.textures {
            // (diagnostics, MCLA_TEX_FIND: a sector's OWN texture of that name)
            if let Ok(v) = std::env::var("MCLA_TEX_FIND") {
                if v.split(',').any(|n| mcla_formats::hash::joaat(&n.to_ascii_lowercase()) == *h) {
                    eprintln!("sector {} own texture {} ({:08x}): {} x {} {:?} first bytes {:02x?} already uploaded {}", sector_id, t.name, h, t.width, t.height, t.format, &t.levels[0][..t.levels[0].len().min(8)], gpu.textures.contains_key(h));
                }
            }
            local.insert(*h, gpu.upload_texture(sk(*h), t));
        }
        // The walls' height / specular maps (HeightSpecularSampler of PSCityBumpSpec, PSCityGrime
        // ...) are read as they are: the game's fetch constants ask for no gamma decoding on
        // them (LIGHTING_WORK.md). Decoded, a flat map's red of 0.5 became 0.21 and tilted every
        // such wall's normal 30 degrees along its tangent. Their own copies, uploaded raw:
        const RAW_KEY: u32 = 0x4853_5241;
        let mut raw: HashMap<u32, Arc<GpuTexture>> = HashMap::new();
        for b in items.iter().map(|it| &it.b) {
            // (walls: the third texture; the ground programs: the fourth)
            let hs = match b.mode {
                MatMode::Opaque | MatMode::AlphaTest => b.third,
                // (the sea's ripple map: the third)
                MatMode::OceanWater | MatMode::OceanWaterLod => b.third,
                // (the pond water's ripple map: the third)
                MatMode::Blend if is_pond(&b.shader) => b.third,
                // (and the walls with a decal and a grime layer: the fourth too)
                MatMode::Ground | MatMode::Decal => b.fourth,
                _ => 0,
            };
            // (diagnostics MCLA_HS_DUMP: once per sector and map - own / shared / MISSING, size, the share of zero bytes in level 0)
            if hs != 0 && !raw.contains_key(&hs) && std::env::var_os("MCLA_HS_DUMP").is_some() {
                let zero = |t: &Texture| t.levels.first().map_or(1.0, |l| l.iter().filter(|x| **x == 0).count() as f32 / l.len().max(1) as f32);
                if let Some((_, t)) = cs.textures.iter().find(|(h, _)| *h == hs) {
                    eprintln!("hs map sector {} shader {} mode {:?}: own {:08x} {} x {} zero {:.2}", sector_id, b.shader, b.mode, hs, t.width, t.height, zero(t));
                } else if let Some(t) = self.shared.get(hs) {
                    eprintln!("hs map sector {} shader {} mode {:?}: shared {:08x} {} x {} zero {:.2}", sector_id, b.shader, b.mode, hs, t.width, t.height, zero(&t));
                } else {
                    eprintln!("hs map sector {} shader {} mode {:?}: MISSING {:08x}", sector_id, b.shader, b.mode, hs);
                }
            }
            if hs != 0 && !raw.contains_key(&hs) {
                if let Some((_, t)) = cs.textures.iter().find(|(h, _)| *h == hs) {
                    raw.insert(hs, gpu.upload_texture_raw(sk(hs) ^ RAW_KEY, t));
                } else if let Some(t) = self.shared.get(hs) {
                    raw.insert(hs, gpu.upload_texture_raw(hs ^ RAW_KEY, &t));
                }
            }
        }
        // (neon2, 2026-10-15) The signs' two textures - CityNeon / CityBillboard / CityStoreSign batches - as stored
        // too: city.wgsl (`sign_raw`) decodes them after the filter, the reference's order. Their own copies under
        // the raw key. MCLA_DEBUG_VIEW=441: the sRGB copies as before.
        // (lead) Decoding AFTER the filter is the recomp's host sampling (its SDK says so: dxbc_translator_fetch.cpp,
        // "gamma is after filtering here, likely should be before"), not the console's. So it is OFF in the game and
        // ON only for comparisons with captures of the recomp: MCLA_REF_FILTER=1 (the pair scripts set it).
        // (user, 2026-10-13: the recomp's order is to be the game's - it looks better. ON by default; MCLA_REF_FILTER=0 = the console's order.)
        let sign_raw_on = std::env::var("MCLA_REF_FILTER").ok().as_deref() != Some("0") && std::env::var("MCLA_DEBUG_VIEW").ok().as_deref() != Some("441");
        let mut sign_raw: HashMap<u32, Arc<GpuTexture>> = HashMap::new();
        if sign_raw_on {
            for b in items.iter().map(|it| &it.b) {
                if !matches!(b.mode, MatMode::Neon | MatMode::StoreSign) {
                    continue;
                }
                for h in [b.diffuse, b.second] {
                    if h == 0 || sign_raw.contains_key(&h) {
                        continue;
                    }
                    if let Some((_, t)) = cs.textures.iter().find(|(th, _)| *th == h) {
                        sign_raw.insert(h, gpu.upload_texture_raw(sk(h) ^ RAW_KEY, t));
                    } else if let Some(t) = self.shared.get(h) {
                        sign_raw.insert(h, gpu.upload_texture_raw(h ^ RAW_KEY, &t));
                    }
                }
            }
        }
        let mut resolve = |gpu: &mut Gpu, h: u32| -> Arc<GpuTexture> {
            if h == 0 {
                return gpu.white.clone();
            }
            if let Some(t) = local.get(&h) {
                return t.clone();
            }
            if let Some(t) = gpu.textures.get(&h) {
                return t.clone();
            }
            if self.tex_fail.contains(&h) {
                return gpu.white.clone();
            }
            match self.shared.get(h) {
                Some(t) => gpu.upload_texture(h, &t),
                None => {
                    self.tex_fail.insert(h);
                    gpu.white.clone()
                }
            }
        };
        // (the batches in draw order, put together by `prepare_geo`)
        let mut batches = Vec::with_capacity(items.len());
        let mut tris = 0u32;
        let mut caster_count = 0u32;
        let mut caster_batches = 0usize;
        let mut caster_min = Vec3::splat(f32::INFINITY);
        let mut caster_max = Vec3::splat(f32::NEG_INFINITY);
        for GeoItem { b, keep, base, n, first, count, parts } in items {
            if !keep {
                continue;
            }
            let is_sign = matches!(b.mode, MatMode::Neon | MatMode::StoreSign);
            // (the mark for city.wgsl, bit 14 of uv2.y: this sign's textures ARE the raw copies - both of them)
            if is_sign && sign_raw.contains_key(&b.diffuse) && sign_raw.contains_key(&b.second) {
                for v in verts[base as usize..(base + n) as usize].iter_mut() {
                    v.uv2[1] += 16384.0;
                }
            }
            // (a sign's textures: the raw copies, keyed apart - see `sign_raw` above)
            let is_sign = is_sign && sign_raw.contains_key(&b.diffuse) && sign_raw.contains_key(&b.second);
            let (d, kd) = match sign_raw.get(&b.diffuse).filter(|_| is_sign) {
                Some(t) => (t.clone(), sk(b.diffuse) ^ RAW_KEY),
                None => (resolve(gpu, b.diffuse), sk(b.diffuse)),
            };
            let (s, ks) = match sign_raw.get(&b.second).filter(|_| is_sign) {
                Some(t) => (t.clone(), sk(b.second) ^ RAW_KEY),
                None => (resolve(gpu, b.second), sk(b.second)),
            };
            let bind = if b.fourth != 0 {
                // (walls with a fourth texture - the interior mirror picture of metal walls and
                // tunnel tiles: their height / specular map stays the raw copy, as below)
                let wall4 = matches!(b.mode, MatMode::Opaque | MatMode::AlphaTest);
                // (the road, ground and decal programs' third texture is their grime layer, laid over
                // by its own alpha: a batch without one gets the transparent texture, as it does when
                // it has no fourth either - `resolve(0)` is opaque white, which painted South
                // Central's lots and sidewalks white, CityDecal / CityTintBumpDecal2 with a
                // height map and no GrimeSampler. LIGHTING.md "White ground in South Central")
                let no_grime = b.third == 0 && matches!(b.mode, MatMode::Road | MatMode::Ground | MatMode::Decal);
                let (t3, k3) = match raw.get(&b.third).filter(|_| wall4) {
                    Some(t) => (t.clone(), sk(b.third) ^ RAW_KEY),
                    None if no_grime => (gpu.clear_tex.clone(), 0),
                    None => (resolve(gpu, b.third), sk(b.third)),
                };
                let (t4, k4) = match raw.get(&b.fourth).filter(|_| b.mode == MatMode::Ground || b.mode == MatMode::Decal) {
                    Some(t) => (t.clone(), sk(b.fourth) ^ RAW_KEY),
                    None => (resolve(gpu, b.fourth), sk(b.fourth)),
                };
                gpu.texture_bind_group4(&d, &s, &t3, &t4, kd, ks, k3, k4)
            } else if b.third != 0 {
                let wall = matches!(b.mode, MatMode::Opaque | MatMode::AlphaTest | MatMode::OceanWater | MatMode::OceanWaterLod) || (b.mode == MatMode::Blend && is_pond(&b.shader));
                let (t3, k3) = match raw.get(&b.third).filter(|_| wall) {
                    Some(t) => (t.clone(), sk(b.third) ^ RAW_KEY),
                    None => (resolve(gpu, b.third), sk(b.third)),
                };
                gpu.texture_bind_group3(&d, &s, &t3, kd, ks, k3)
            } else {
                gpu.texture_bind_group(&d, &s, kd, ks)
            };
            let caster = geo_rank(&b) == 0;
            if count == 0 {
                continue;
            }
            tris += count / 3;
            if caster {
                caster_count = first + count;
                caster_batches = batches.len() + 1;
                caster_min = caster_min.min(b.min);
                caster_max = caster_max.max(b.max);
            }
            batches.push(SectorBatch { interior: b.interior, shader: b.shader, first, count, bind, mode: b.mode, min: b.min, max: b.max, parts });
        }
        let (vb, ib) = if indices.is_empty() {
            (None, None)
        } else {
            (
                Some(gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("sector vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX })),
                Some(gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("sector ib"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX })),
            )
        };
        // (a detailed sector comes in over a third of a second; interiors and far models are just there)
        let u = if sector_id < SET_ID { 0.0 } else { 1.0 };
        let obj = Some((gpu.new_object(&sector_uniform(u, false)), gpu.new_object(&sector_uniform(u, true))));
        GpuSector { vb, ib, batches, caster_count, caster_min, caster_max, caster_batches, tris, obj, u, shown: u, leaving: false }
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;

    fn triangle(third: u32, fourth: u32) -> CpuBatch {
        let vertex = Vertex { pos: [0.0; 3], normal: [0.0, 1.0, 0.0], uv0: [0.0; 2],
            uv1: [0.0; 2], uv2: [0.0; 2], color: [255; 4], color1: [255; 4] };
        CpuBatch { vertices: vec![vertex; 3], indices: vec![0, 1, 2], diffuse: 10,
            second: 20, third, fourth, mode: MatMode::Terrain,
            min: Vec3::ZERO, max: Vec3::ONE, shader: "CityTerrain".into(), interior: false, parts: Vec::new() }
    }

    #[test]
    fn terrain_layers_with_colliding_xor_keys_remain_distinct() {
        // Both pairs produced 128 under the old third ^ rotate(fourth, 7) key.
        let batches = merge_batches(vec![triangle(128, 0), triangle(0, 1)]);
        assert_eq!(batches.len(), 2);
        assert_eq!((batches[0].third, batches[0].fourth), (128, 0));
        assert_eq!((batches[1].third, batches[1].fourth), (0, 1));
        assert_eq!(batches.iter().map(|b| b.indices.len()).sum::<usize>(), 6);
    }

    #[test]
    fn matching_materials_still_merge_and_rebase_geometry() {
        let batches = merge_batches(vec![triangle(30, 40), triangle(30, 40)]);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].vertices.len(), 6);
        assert_eq!(batches[0].indices, [0, 1, 2, 3, 4, 5]);
    }
}

#[cfg(test)]
mod lod_option_tests {
    /// sub_822D5A30: 300 x (1.5 at 25 degrees .. 1.0 from 50), times DETAIL LEVEL.
    #[test]
    fn sector_detail_distance() {
        let d = super::World::detail_distance;
        assert_eq!([d(25.0, 1.0), d(37.5, 1.0), d(50.0, 1.0), d(70.0, 1.0)], [450.0, 375.0, 300.0, 300.0]);
        assert_eq!([d(25.0, 2.0), d(50.0, 3.0)], [900.0, 900.0]);
    }
}
