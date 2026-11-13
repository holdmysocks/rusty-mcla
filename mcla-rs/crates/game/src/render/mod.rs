//! wgpu renderer: device setup, static-geometry pipeline, texture cache.

pub mod aniso;
pub mod pcache;
pub mod car_lamps;
pub mod mb;
pub mod ms;
pub mod post;
pub mod skyhat;
pub mod xs;
#[cfg(test)]
mod lightref;

use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use hashbrown::HashMap;
use mcla_formats::rage::texture::Texture;
use mcla_formats::rage::xenos::TexFormat;
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub color: [u8; 4],
    pub color1: [u8; 4],
    /// third UV set (grime layer of ground shaders)
    pub uv2: [f32; 2],
}

impl Vertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 32, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 40, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 44, shader_location: 5 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 48, shader_location: 8 },
        ],
    };
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
    pub cam_pos: [f32; 4],
    pub sun_dir: [f32; 4],
    pub sun_color: [f32; 4],
    pub ambient_sky: [f32; 4],
    pub ambient_ground: [f32; 4],
    pub fog_color: [f32; 4],
    pub params: [f32; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    /// rgb, a = day factor
    pub sky_zenith: [f32; 4],
    /// rgb, a = star brightness
    pub sky_horizon: [f32; 4],
    /// rgb, a = sunset strength
    pub sky_sunset: [f32; 4],
    /// xyz = the sun's true direction; w = 1 in the rear-view mirror's pass: its cut-out materials keep every
    /// pixel with alpha above 0 (rearview.rs `view`)
    pub true_sun: [f32; 4],
    pub moon_dir: [f32; 4],
    /// world -> shadow map clip space
    pub light_vp: [[f32; 4]; 4],
    /// x = shadow strength (0 = off), y = texel size in uv
    pub shadow_params: [f32; 4],
    /// clouds: lit colour (rgb), a = coverage 0..1
    pub cloud_color: [f32; 4],
    /// clouds: shaded underside (rgb), a = drift (uv offset)
    pub cloud_shade: [f32; 4],
    /// the tone map as it will treat this frame: x = exposure (middle grey over the adapted
    /// luminance), y = its white, z = saturation, w = 1 while the game's night programs draw
    pub tone: [f32; 4],
    /// rgb = twice the colour correction, w = OcclusionInfo.w (vertex occlusion lifted towards 1)
    pub grade: [f32; 4],
    /// the fog's colour towards the east (rgb); w = how far it leans to the horizon colours
    pub fog_east: [f32; 4],
    /// ... and towards the west
    pub fog_west: [f32; 4],
    /// the dome as the game's Skyhat program mixes it: x = AzimuthStrength, y = AzimuthHeight,
    /// z = HDRExposure, w = 1 when sky_zenith / sky_horizon (west) / sky_sunset (east) hold the
    /// dome's own keys
    pub dome: [f32; 4],
    /// the cars' key lamp at night (VS_CarPaint's positional light, 100 m off): xyz = the
    /// direction towards it
    pub car_key: [f32; 4],
    /// ... and its colour times its strength, sat(Intensity / distance squared)
    pub car_key_col: [f32; 4],
    /// the weather's key light while the night programs draw (it is their third "dynamic"
    /// light, 100 km off and unshadowed): xyz = the direction towards it
    pub key_dir: [f32; 4],
    /// ... and its colour (gLightColor)
    pub key_col: [f32; 4],
    /// the fast shadow collector's map (`Gpu::collector_pass`): xy = the world x, z of its
    /// centre, z = 1 while the map is kept up (0: the shaders fall back to their own taps);
    /// w = 1 while the wet road's planar reflection picture is drawn for this view
    /// (`Gpu::reflection_pass`)
    pub collector: [f32; 4],
    /// the cars' other light slot at night (`car_key_light`): the direction towards it, w = 1: use it
    pub car_key2: [f32; 4],
    /// ... its colour x strength
    pub car_key2_col: [f32; 4],
    /// the garage (garage round, 2026-10-13): rgb = the cars' own ambient there (ambYPos / gLightAmbient of
    /// the car programs: the light manager's sm_vGarageAmbientLighting), w = 1: in force - and then
    /// `car_key2` is a POSITION when its w is 2 (the garage's second light, a point 1 m above the camera)
    pub car_amb: [f32; 4],
    /// OPTIONS > PERFORMANCE: x = DRAW DISTANCE, y = DETAIL LEVEL (1 = the original; props.rs `Lod`, city.wgsl `vs_inst`; w = the clock of the plants' wind)
    pub lod: [f32; 4],
    /// x = the plants' night factor (props.rs `Lod::night`: 1 by day, 2 at night), yzw unused
    pub lod2: [f32; 4],
}

pub const MAX_LIGHTS: usize = 48;
/// Where the game's data lies (set before the renderer is made; the paint types are read from it).
pub static DATA_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The paint types of the garage, in the order of its menu (`garage.ChangePaintType`: 1 Matte,
/// 2 Gloss, 3 Metallic, 4 Pearl, 5 Sparkle; 0 = the executable's "DirtyMatte").
pub const PAINT_TYPES: [&str; 6] = ["DIRTY MATTE", "MATTE", "GLOSS", "METALLIC", "PEARLESCENT", "SPARKLE"];

/// The materials of the city's sectors whose opaque batches draw with a pipeline of their own (`Gpu::pipeline_opaque_mode`).
pub const MODE_PIPES: [MatMode; 13] = [
    MatMode::Opaque,
    MatMode::AlphaTest,
    MatMode::Window,
    MatMode::Road,
    MatMode::Neon,
    MatMode::Decal,
    MatMode::Ground,
    MatMode::Terrain,
    MatMode::StoreInterior,
    MatMode::StoreSign,
    MatMode::NeonTint,
    MatMode::OceanWater,
    MatMode::OceanWaterLod,
];

/// The materials of the cars' opaque batches (bodies, wheels, brakes) with a pipeline of their own, without a cull
/// (`Gpu::pipeline_opaque_mode`) and with the set-up car's cull (`Gpu::pipeline_opaque_cull_mode`).
pub const CAR_MODE_PIPES: [MatMode; 12] = [
    MatMode::Opaque,
    MatMode::AlphaTest,
    MatMode::Decal,
    MatMode::Paint,
    MatMode::Chrome,
    MatMode::Light,
    MatMode::Object,
    MatMode::Rubber,
    MatMode::BlackGloss,
    MatMode::Interior,
    MatMode::Rim,
    MatMode::DarkGlass,
];

/// The materials of the props' opaque batches with a pipeline of their own (`Gpu::pipeline_inst_mode`).
pub const PROP_MODE_PIPES: [MatMode; 6] = [MatMode::Object, MatMode::Foliage, MatMode::AlphaTest, MatMode::Signal, MatMode::BlackGloss, MatMode::StoreSign];

/// `MCLA_MODE_PIPES=0`: every opaque sector batch with the one city pipeline, as before.
pub fn mode_pipes_on() -> bool {
    std::env::var("MCLA_MODE_PIPES").map_or(true, |v| v.trim() != "0")
}

/// The city program reads its material's mode from the constant `MODE_K` when a pipeline sets it, else from the material
/// uniform as before. The program is one for every material (city, props, cars, people) and branches on the mode; a
/// pipeline built for one mode is drawn only with that mode's material bound (`mat_bgs`, whose `mode` is the same
/// number), so it computes the same, and the driver compiles it without the other materials' paths. RTX 3090, 1440p:
/// city pass A 2 - 3.5 ms -> 0.9 - 1.3 ms on Vulkan, 4.6 -> 1.2 ms on DX12, the same picture (docs/PERFORMANCE.md 2026-11-04 (2)).
fn mode_constant(src: &str) -> String {
    format!("override MODE_K: u32 = 4294967295u;\nfn mmode() -> u32 {{ if (MODE_K != 4294967295u) {{ return MODE_K; }} return mat.mode; }}\n{}", src.replace("mat.mode", "mmode()"))
}

/// The six mcCarPaintTypeTuning files in the paint type's order (F-60347 names them `CarPaint%s` with DirtyMatte, Matte,
/// Gloss, Metallic, Pearl, Sparkle, and the loader adds the class's extension `.parFileIO`; the archive keeps their names'
/// hashes only).
const PAINT_FILES: [&str; 6] = ["0xcec1263b", "0xcc0ab1c3", "0xa91a37a8", "0x3c752d39", "0x7c619591", "0x3547dd17"];
/// Their fields in the order of the struct (F-44499 registers them at +12 .. +56); every file sets all twelve, so the
/// constructor's defaults (F-85434) never show.
const PAINT_FIELDS: [&str; 12] = ["m_fDiffuseIntensity", "m_fSpecIntensity", "m_fAmbientIntensity", "m_fSpecExp", "m_fFresnelExp", "m_fFresnelMin", "m_fFresnelMax", "m_fPaintFalloff", "m_fPearlescence", "m_fNoiseScale", "m_fNoiseStrength", "m_fNoiseExponent"];

/// The shader with the disc's paint tuning written into its table: the six
/// `tune/vehicle/globalGfxTuning/CarPaint<type>.parFileIO` files (`PAINT_FILES`).
fn paint_tuned(src: &str) -> String {
    const FILES: [&str; 6] = PAINT_FILES;
    const FIELDS: [&str; 12] = PAINT_FIELDS;
    let Some(dir) = DATA_DIR.get() else { return src.to_string() };
    let mut rows = Vec::new();
    for f in FILES {
        let Ok(text) = std::fs::read_to_string(std::path::Path::new(dir).join("tune/vehicle/globalgfxtuning").join(f)) else { return src.to_string() };
        let mut v = [0.0f32; 12];
        for l in text.lines() {
            let mut w = l.split_whitespace();
            if let (Some(k), Some(x)) = (w.next(), w.next().and_then(|x| x.parse::<f32>().ok())) {
                if let Some(i) = FIELDS.iter().position(|n| *n == k) {
                    v[i] = x;
                }
            }
        }
        rows.push(v);
    }
    let table = |name: &str, at: usize| {
        let cells: Vec<String> = rows.iter().map(|v| format!("vec4<f32>({:?}, {:?}, {:?}, {:?})", v[at], v[at + 1], v[at + 2], v[at + 3])).collect();
        format!("const {}: array<vec4<f32>, 6> = array<vec4<f32>, 6>({});\n", name, cells.join(", "))
    };
    let (Some(a), Some(b)) = (src.find("//#PAINT_BEGIN"), src.find("//#PAINT_END")) else { return src.to_string() };
    log::info!("paint types: {} tunings from the disc", rows.len());
    format!("{}{}{}{}{}", &src[..a], table("PAINT_A", 0), table("PAINT_B", 4), table("PAINT_C", 8), &src[b..])
}

/// The cabin trim's nine surface kinds as the disc's GlobalInteriorTuning has them (tune/vehicle `intTune`, 0xdc0293e5):
/// per kind DetailTile, SpecMapIntensity, SpecMapExp, Smoothness, Metallic (the file's field order, +4 .. +20 of the
/// global's 28-byte rows). F-19629 hands kind k's row to the trim program as intDL0.w = DetailTile and intDL1 =
/// (SpecMapExp, SpecMapIntensity, Smoothness, Metallic) for every zone of that kind - equal to every recorded trim draw
/// (work/carparts2/trim/tables.py). Without the file: these, the disc's values.
pub const TRIM_DISC: [[f32; 5]; 9] = [
    [1.0, 0.0175, 0.42499998, 0.0, 0.0],
    [1.0, 0.37999991, 1.6000003, 0.0, 0.0],
    [0.875, 0.1375, 0.72500002, 0.0, 0.0],
    [1.0, 0.408, 1.305, 0.0, 0.5],
    [1.0, 0.025000019, 0.40000001, 0.0, 0.0],
    [1.0, 0.02, 0.5, 0.0, 0.0],
    [1.0, 1.0, 1.0, 0.963, 1.0],
    [1.0, 0.15999998, 0.40000001, 0.25, 0.0],
    [1.0, 0.11, 0.40000001, 0.25, 1.0],
];

/// The trim's kinds read from the disc (`TRIM_DISC` where the file or a field is missing).
pub fn trim_tune() -> [[f32; 5]; 9] {
    static T: std::sync::OnceLock<[[f32; 5]; 9]> = std::sync::OnceLock::new();
    *T.get_or_init(|| {
        let Some(dir) = DATA_DIR.get() else { return TRIM_DISC };
        std::fs::read_to_string(std::path::Path::new(dir).join("tune/vehicle/0xdc0293e5")).map_or(TRIM_DISC, |t| trim_tune_from(&t))
    })
}

/// The kinds from the text of the tune file (fields it lacks: `TRIM_DISC`'s).
pub fn trim_tune_from(text: &str) -> [[f32; 5]; 9] {
    let mut t = TRIM_DISC;
    const F: [&str; 5] = ["m_fDetailTile", "m_fSpecMapIntensity", "m_fSpecMapExp", "m_fSmoothness", "m_fMetallic"];
    for l in text.lines() {
        let mut w = l.split_whitespace();
        let (Some(k), Some(x)) = (w.next(), w.next().and_then(|x| x.parse::<f32>().ok())) else { continue };
        for (i, f) in F.iter().enumerate() {
            if let Some(n) = k.strip_prefix(f).and_then(|n| n.parse::<usize>().ok()).filter(|n| *n < 9) {
                t[n][i] = x;
            }
        }
    }
    t
}

/// The kind of surface of a cabin trim zone (0 .. 24), F-19629's fixed list: zones 0 .. 2 kind 0, 3 .. 5 kind 2,
/// 6 .. 8 kind 1, 9 .. 11 kind 3, 12 .. 14 kind 4, 15 .. 17 kind 5, 18 kind 6, 19 .. 21 kind 7, 22 .. 24 kind 8.
pub fn trim_kind(zone: usize) -> usize {
    [0, 0, 0, 2, 2, 2, 1, 1, 1, 3, 3, 3, 4, 4, 4, 5, 5, 5, 6, 7, 7, 7, 8, 8, 8][zone.min(24)]
}

/// The shader with the trim's intDL1 rows by kind (SpecMapExp, SpecMapIntensity, Smoothness, Metallic) written in.
fn trim_tuned(src: &str) -> String {
    let (Some(a), Some(b)) = (src.find("//#TRIM_BEGIN"), src.find("//#TRIM_END")) else { return src.to_string() };
    let cells: Vec<String> = trim_tune().iter().map(|r| format!("vec4<f32>({:?}, {:?}, {:?}, {:?})", r[2], r[1], r[3], r[4])).collect();
    format!("{}//#TRIM_BEGIN\nconst TRIM_DL1: array<vec4<f32>, 9> = array<vec4<f32>, 9>({});\n{}", &src[..a], cells.join(", "), &src[b..])
}

pub const SHADOW_SIZE: u32 = 4096;
/// The original's sun shadow maps: four cascades of 640 x 640 texels, two by two in one
/// 1280 x 1280 depth map (tile 1 right of tile 0, tiles 2 and 3 under them) - `Gpu::set_cascades`.
pub const CASC_TILE: u32 = 640;

/// SHADOW RESOLUTION (hooks::shadow_res_scale: 1 = ORIGINAL, 2 = 2X, 4 = 4X), read once when the renderer starts:
/// the cascades' tiles and the fast collector's map get so many times the original's texels on each side over the
/// same ground (their uv rules, the taps' radii in uv, the map's 256 m are kept; the slope bias is per texel as the
/// hardware's). At 1 every size is the original's.
pub fn shadow_scale() -> u32 {
    static S: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *S.get_or_init(|| match crate::hooks::shadow_res_scale() {
        0 | 1 => 1,
        2 | 3 => 2,
        _ => 4,
    })
}

/// A cascade tile's side in texels (the original's 640 x SHADOW RESOLUTION; the atlas is two by two of them).
pub fn casc_tile() -> u32 {
    CASC_TILE * shadow_scale()
}

/// The fast collector's map side in texels (the original's 256 x SHADOW RESOLUTION, always 256 m).
pub fn fc_size() -> u32 {
    FC_SIZE * shadow_scale()
}
/// tune/360CascadedShadowMaps.xml, per tile: FarSlice, ViewportZShift, FovYDiv, BiasSlope.
pub const CASC_TUNE: [[f32; 4]; 4] = [[15.0, 1.5, 0.653, 4.485], [31.0, 1.5, 0.75, 3.8], [80.0, 4.0, 0.7, 1.938], [160.0, 8.0, 0.7, 2.8]];
/// The tiles' constant depth bias in metres along the rays: the file's DepthBias (1e-7, 1e-7,
/// 4e-7, 0 of the depth range) over the depth scale of the recorded matrices. That scale changes
/// with the hour by a rule not found (5 .. 13, 16 .. 32, 47 .. 103 mm in six recordings): the means.
pub const CASC_BIAS_M: [f32; 4] = [0.0096, 0.024, 0.078, 0.0];
/// The cascades' constant depth bias in metres: tune/360CascadedShadowMaps.xml's DepthBias (1e-7, 1e-7, 4e-7, 0)
/// times the tile's depth range as the original's matrices have it in the standard view's recordings
/// (tg_6.25, 7, 9, 12, 15, 17.5b: 1 / |z row| of c64 ..), interpolated by the shadow ray's x.
/// Only under MCLA_CASC_ZOLD=1 now: the depth rule is decoded (`set_cascades`).
fn casc_const_bias(tile: usize, ray_x: f32) -> f32 {
    const X: [f32; 6] = [-0.847, -0.675, -0.263, 0.108, 0.470, 0.874];
    const R: [[f32; 6]; 3] = [
        [128720.0, 88063.0, 53307.0, 92987.0, 90639.0, 123946.0],
        [316880.0, 221683.0, 155898.0, 221202.0, 221099.0, 306187.0],
        [257406.0, 178140.0, 116468.0, 182892.0, 180501.0, 248299.0],
    ];
    const DEPTH_BIAS: [f32; 4] = [1e-7, 1e-7, 4e-7, 0.0];
    if tile > 2 {
        return 0.0;
    }
    let x = ray_x.clamp(X[0], X[5]);
    let k = (0..5).find(|&k| x <= X[k + 1]).unwrap_or(4);
    let t = (x - X[k]) / (X[k + 1] - X[k]);
    DEPTH_BIAS[tile] * (R[tile][k] + (R[tile][k + 1] - R[tile][k]) * t)
}
/// tune/360CascadedShadowMaps.xml, per tile: LightDepthScale and DepthBias.
const CASC_LDS: [f32; 4] = [35.0, 25.0, 9.0, 5.0];
const CASC_DEPTH_BIAS: [f32; 4] = [1e-7, 1e-7, 4e-7, 0.0];
fn casc_zold() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_CASC_ZOLD").map_or(false, |v| v.trim() == "1"))
}
/// (diagnostics: the scene's cascade uniform of the last frame, for `Gpu::dump_casc`)
static CASC_LAST: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());
/// The eye of the last cascade drawn (tile 3), the frame's gViewInverse row 3 after the shadow passes: the impostor
/// tiles' generation pass sets no view of its own, so PSGenerateImposter's specular reads this point (c15).
static CASC_EYE3: std::sync::Mutex<[f32; 3]> = std::sync::Mutex::new([0.0; 3]);
pub fn casc_eye3() -> Vec3 {
    Vec3::from_array(*CASC_EYE3.lock().unwrap())
}
/// The cascades' depth here: 0 at 600 m up the rays from the camera, 1 at 200 m down them.
const CASC_Z_NEAR: f32 = 600.0;
const CASC_Z_RANGE: f32 = 800.0;

/// The cascades as the scene's programs read them (city.wgsl `Casc`; the original's
/// PS_ShadowBlend constants c64 .. c87).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct CascUniform {
    /// world -> the depth map's uv and depth, per tile (gLightMatrixArr)
    pub m: [[[f32; 4]; 4]; 4],
    /// the rectangles' long axis (gTopPlanes: c82; w as recorded, used only where no tile matches)
    pub top: [f32; 4],
    /// their short axis (gRightPlanes: c85)
    pub right: [f32; 4],
    /// c83, c84: minus the rectangles' centres along `top`, and their half lengths
    pub top_off: [f32; 4],
    pub top_half: [f32; 4],
    /// c86, c87: the same along `right`
    pub right_off: [f32; 4],
    pub right_half: [f32; 4],
    /// x = 1: the maps are drawn this frame; y, z = gFadeOutLastSMap (the fade's start and length)
    pub params: [f32; 4],
    /// xyz: the direction to the light the taps are gated by (n . l > 0; VS_ShadowBlend's gLightPosDir c16);
    /// w: 1 + f while ShaderGlobals.z >= 23 (f = z - 24: the garage's lighting under a cine scene), else 0
    pub light: [f32; 4],
}

/// What one cascade's depth pass is drawn with (shadow.wgsl `ShadowGlobals`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct CascPass {
    pub light_vp: [[f32; 4]; 4],
    pub bias: [f32; 4],
}
/// The fast shadow collector's map: 256 x 256 texels of 1 m round the camera.
pub const FC_SIZE: u32 = 256;
/// The night fast collector's map (F-28445's FastCollectorNight: 512 x 512, one 8-bit channel), whatever the
/// SHADOW RESOLUTION.
pub const NFC_SIZE: u32 = 512;
/// Its half reach in metres and its eye's lead: 16 (F-55997's night branch, `*0x82023A98`).
pub const NFC_REACH: f32 = 16.0;

/// The night fast collector's view-projection (F-55997, state 4 at night; LIGHTING.md "The night fast collector"):
/// an ortho box 32 m square and 600 m deep (F-11735's +-16, +-300) seen from straight above, its eye 100 m over
/// the camera and 16 m ahead of it along the camera's forward in the plan (the camera matrix's row 2 x -16 in x and z,
/// not set to unit length), not snapped. `px` = the half texel the console's pixel centres put on it (1 / NFC_SIZE
/// with `half_pixel_on`, else 0): the map is drawn that much along +x and +z. Read with the day's rule, uv =
/// (p.xz - trunc(camera.xz)) / 256 + 0.5 - so a point reads the map at eye + (p - trunc(camera)) / 8.
pub fn night_collector_vp(cam: Vec3, forward: Vec3, px: f32) -> Mat4 {
    let e = Vec3::new(cam.x + forward.x * NFC_REACH, cam.y + 100.0, cam.z + forward.z * NFC_REACH);
    let (h, d) = (NFC_REACH, 600.0);
    Mat4::from_cols(Vec4::new(1.0 / h, 0.0, 0.0, 0.0), Vec4::new(0.0, 0.0, -1.0 / d, 0.0), Vec4::new(0.0, -1.0 / h, 0.0, 0.0), Vec4::new(px - e.x / h, e.z / h - px, e.y / d + 0.5, 1.0))
}

#[cfg(test)]
mod night_collector_tests {
    use super::*;

    /// The night pass's view-projection of 20 frames of the running original at 23:00 (tools/night_collector_reference.py:
    /// World^-1 x gWorldViewProj of its first VS_ShadowNight draw) against `night_collector_vp` of the frame's camera
    /// (gViewInverse: place, forward = -row 2), without the half texel (the recorded matrix has none: the console puts
    /// it in its pixel centres); and the value its PS_ShadowNight writes, c63.z = 1 - m_ShadowFade (0.8 at 23:00, Nice).
    #[test]
    fn the_view_is_the_originals() {
        let mut n = 0;
        for l in include_str!("../testdata/night_collector_ref.txt").lines().filter(|l| !l.starts_with('#')) {
            let v: Vec<f32> = l.split_whitespace().skip(1).map(|x| x.parse().unwrap()).collect();
            let (cam, row2) = (Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5]));
            let m = night_collector_vp(cam, -row2, 0.0);
            for i in 0..4 {
                for k in 0..4 {
                    let (ours, theirs) = (m.col(i)[k], v[6 + 4 * i + k]);
                    assert!((ours - theirs).abs() <= 2e-5 * theirs.abs().max(1.0), "{} VP[{}][{}]: {} against {}", l.split_whitespace().next().unwrap(), i, k, ours, theirs);
                }
            }
            assert_eq!(v[22 + 2], 1.0 - 0.8f32);
            n += 1;
        }
        assert_eq!(n, 20);
    }
}
/// The wet road's planar reflection picture (refl.wgsl): 256 x 256, as the game's.
pub const REFL_SIZE: u32 = 256;
/// edge of the mini sky (the original's target: 128 x 128, 8:8:8:8 with the console's gamma)
pub const MINISKY_SIZE: u32 = 128;

/// What the collector's pass is drawn with (fast_collector.wgsl `Fc`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct FcUniform {
    pub view_proj: [[f32; 4]; 4],
    pub light_vp: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub to_light: [f32; 4],
    pub blur: [f32; 4],
    pub top: [f32; 4],
    pub right: [f32; 4],
    pub rect: [f32; 4],
    pub hist: [f32; 4],
}
/// The night lamp grid, laid out as the original's (PSMultiLight, its constants c96..c98): the
/// city from (-3467, -2358) over 6320 x 4463.75 m in 512 x 640 cells, four lamps a cell.
pub const LAMP_GRID: (u32, u32) = (512, 640);
pub const LAMP_ORIGIN: (f32, f32) = (-3467.0, -2358.0);
pub const LAMP_SPAN: (f32, f32) = (6320.0, 4463.75);
/// four texels a lamp (position + intensity, colour, cone axis + scale, cone bias), 256 lamps a row
pub const LAMP_DATA: (u32, u32) = (1024, 128);

/// A lamp of the night table as the light manager holds it (the walkers' picker reads these):
/// index 0 is the empty slot (no colour, no strength, at the origin).
#[derive(Clone, Copy, Debug, Default)]
pub struct LampRec {
    pub pos: Vec3,
    pub color: Vec3,
    pub strength: f32,
    /// the direction back to the lamp from what it lights, and the cone sat(cos x [0] + [1])
    pub back: Vec3,
    pub cone: [f32; 2],
}

/// The night lamp grid and table on the CPU side (`Gpu::set_lamps` fills it).
pub struct LampTables {
    /// four table indices per cell (0 = none), row by row from the grid's origin
    pub cells: Vec<[u16; 4]>,
    pub recs: Vec<LampRec>,
}

impl LampTables {
    /// The four lamps of the cell under `p` (the programs' cell: uv = (x, z) x (1 / 6320,
    /// 1 / 4463.75) + (0.54857594, 0.5282554), x (512, 640); clamped to the grid as F-25968).
    pub fn cell(&self, p: Vec3) -> [u16; 4] {
        self.cells.get(lamp_cell_index(p)).copied().unwrap_or([0; 4])
    }

    /// The lamps of the cells over [p - half, p + half] in x and z (F-79329): each corner `(v x scale - bias) x size`
    /// with one rounding for the multiply-subtract, cut towards zero and clamped into the grid; rows z outer, a cell's
    /// four in order, each index once (0 too), into a list of capacity `cap` that holds `cap - 1`. Checked on 3,000
    /// cases against the original's code run offline (`lamp_query_reference.py`, test `lamp_query_matches_the_original`).
    pub fn lamp_query(&self, p: Vec3, half: f32, cap: usize) -> Vec<u16> {
        let axis = |v: f32, scale: f32, bias: f32, n: u32| ((v.mul_add(scale, bias) * n as f32) as i64).clamp(0, n as i64 - 1) as usize;
        let (x0, x1) = (axis(p.x - half, 1.0 / 6320.0, 0.54857594, LAMP_GRID.0), axis(p.x + half, 1.0 / 6320.0, 0.54857594, LAMP_GRID.0));
        let (z0, z1) = (axis(p.z - half, 1.0 / 4463.75, 0.5282554, LAMP_GRID.1), axis(p.z + half, 1.0 / 4463.75, 0.5282554, LAMP_GRID.1));
        let mut cand: Vec<u16> = Vec::new();
        for z in z0..=z1 {
            for x in x0..=x1 {
                for &i in self.cells.get(z * LAMP_GRID.0 as usize + x).unwrap_or(&[0; 4]) {
                    if !cand.contains(&i) && cand.len() + 1 < cap {
                        cand.push(i);
                    }
                }
            }
        }
        cand
    }

    /// The light pickers' lamps at `p` (F-16230 / F-24398): the grid's lamps within 4 m (`lamp_query`, a list of 32),
    /// each scored sat(strength / d^2) x its cone (F-60154), the best four kept (F-48037): (table index, score), best first.
    pub fn best_four(&self, p: Vec3) -> Vec<(u16, f32)> {
        let cand = self.lamp_query(p, 4.0, 32);
        pick_best(&cand, |i| self.recs.get(i as usize).map_or(0.0, |l| crate::figlight::score(l, p)), 4)
    }
}

/// F-48037's pick: the candidates' (score, index) pairs in candidate order, sorted ascending by score with the
/// original's STLport introsort (F-39252, `<` on the score: equal scores and NaN fall as its steps leave them), then the
/// best `cap` read from the top down. (The original fills the places left with the index "none", the lamp count,
/// and score 0; the callers add the scores, so they are left out here.) Checked against F-48037 run offline on 3,102
/// lists (`tools/lamp_pick_reference.py`, test `lamp_pick_matches_the_original`).
pub fn pick_best(cand: &[u16], score: impl Fn(u16) -> f32, cap: usize) -> Vec<(u16, f32)> {
    let mut pairs: Vec<(f32, u16)> = cand.iter().map(|&i| (score(i), i)).collect();
    crate::physics::loose::stl_sort_by(&mut pairs, |a, b| a.0 < b.0);
    pairs.iter().rev().take(cap).map(|&(s, i)| (i, s)).collect()
}

/// F-37078, the cascade casters' test against the main view's near, left and right planes (the view's +768 / +784 /
/// +800, `n . p + w` positive inside): per plane, in that order, the light's run `s = -(n . d)` as the code sums it
/// (y, then z, then x, each a fused add rounded to a float) and, when `s > 0`, the sphere's depth outside it
/// `-(n . c) - w` (a three-lane dot); bit 8, 2, 4 when that is more than the radius. The near plane's bit ends the
/// test. Checked on 6,000 cases against the original's code run offline (`tools/casc_planes_reference.py`, test
/// `casc_planes_match_the_original`).
pub fn casc_main_out(planes: &[Vec4; 3], d: Vec3, c: Vec3, r: f32) -> u32 {
    let fma = |a: f32, b: f32, c: f32| (a as f64).mul_add(b as f64, c as f64) as f32;
    let mut out = 0;
    for (i, p) in planes.iter().enumerate() {
        let n = -p.truncate();
        let s = fma(d.x, n.x, fma(d.z, n.z, d.y * n.y));
        if s > 0.0 && crate::physics::exact::dot3(n, c) - p.w > r {
            out |= [8, 2, 4][i];
            if i == 0 {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod casc_planes_tests {
    use super::*;

    #[test]
    fn casc_planes_match_the_original() {
        let t = include_str!("testdata/casc_planes_reference.txt");
        let mut n = 0;
        for l in t.lines() {
            let (a, b) = l.split_once(" -> ").unwrap();
            let w: Vec<f32> = a.split_whitespace().skip(1).map(|x| f32::from_bits(u32::from_str_radix(x, 16).unwrap())).collect();
            let pl = [Vec4::new(w[0], w[1], w[2], w[3]), Vec4::new(w[4], w[5], w[6], w[7]), Vec4::new(w[8], w[9], w[10], w[11])];
            let got = casc_main_out(&pl, Vec3::new(w[16], w[17], w[18]), Vec3::new(w[12], w[13], w[14]), w[15]);
            assert_eq!(got, b.trim().parse::<u32>().unwrap(), "{l}");
            n += 1;
        }
        assert_eq!(n, 6000);
    }
}

/// The cell index F-25968 reads for a place: `(p x scale - bias) x grid size` cut towards zero (a NaN cuts to
/// the most negative int) and clamped into the grid, row `z` times the grid width plus column `x`. Scale and bias are
/// the floats the grid builder (F-12750) stores in the grid object: 1 / (max - min) and min x scale per axis, the
/// bias negated and subtracted (so added). Checked on 1,114 places against the original's code run offline
/// (`lamp_cell_reference.py`, test `lamp_cell_index_matches_the_original`).
pub fn lamp_cell_index(p: Vec3) -> usize {
    let u = p.x * (1.0 / 6320.0) + 0.54857594;
    let v = p.z * (1.0 / 4463.75) + 0.5282554;
    let cx = ((u * LAMP_GRID.0 as f32) as i64).clamp(0, LAMP_GRID.0 as i64 - 1) as usize;
    let cz = ((v * LAMP_GRID.1 as f32) as i64).clamp(0, LAMP_GRID.1 as i64 - 1) as usize;
    cz * LAMP_GRID.0 as usize + cx
}

#[cfg(test)]
mod paint_file_tests {
    use super::*;

    /// The paint type files are the ones the original loads for each type: `CarPaint<type>.parFileIO` (F-60347, the
    /// class's extension from its vtable) hashed as the archive hashes names, and the field names as F-44499 registers them.
    #[test]
    fn paint_files_are_the_originals() {
        for (i, ty) in ["DirtyMatte", "Matte", "Gloss", "Metallic", "Pearl", "Sparkle"].iter().enumerate() {
            assert_eq!(format!("{:#010x}", mcla_formats::hash::joaat(&format!("CarPaint{ty}.parFileIO"))), PAINT_FILES[i], "{ty}");
        }
        assert_eq!(PAINT_FIELDS.len(), 12);
    }
}

#[cfg(test)]
mod lamp_cell_tests {
    use super::*;

    /// F-48037 run offline (its sort and output as the generated code has them; the lamp fetch and the score stubbed
    /// to the case's table): 3,000 candidate lists of 0 - 40 lamps with many equal, zero and NaN scores and 102 killers, organ pipes and near-killers (40 of their lists reach the depth limit and its heap sort).
    #[test]
    fn lamp_pick_matches_the_original() {
        let mut n = 0;
        for l in include_str!("testdata/lamp_pick_reference.txt").lines() {
            let (case, want) = l.split_once(" -> ").unwrap();
            let t: Vec<&str> = case.split_whitespace().collect();
            assert_eq!(t[0], "PICK");
            let (lamps, cap, k): (usize, usize, usize) = (t[1].parse().unwrap(), t[2].parse().unwrap(), t[3].parse().unwrap());
            let cand: Vec<u16> = t[4..4 + k].iter().map(|x| x.parse().unwrap()).collect();
            let sc: Vec<u32> = t[4 + k..4 + k + lamps].iter().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let w: Vec<&str> = want.split_whitespace().collect();
            let m: usize = w[0].parse().unwrap();
            assert_eq!(m, cap, "{l:.80}");
            let got = pick_best(&cand, |i| f32::from_bits(sc[i as usize]), cap);
            for j in 0..m {
                let (wi, ws): (u16, u32) = (w[1 + j].parse().unwrap(), u32::from_str_radix(w[1 + m + j], 16).unwrap());
                match got.get(j) {
                    Some(&(gi, gs)) => assert_eq!((gi, gs.to_bits()), (wi, ws), "{l:.120} place {j}"),
                    None => assert_eq!((wi as usize, ws), (lamps, 0), "{l:.120} pad {j}"),
                }
            }
            n += 1;
        }
        assert_eq!(n, 3102);
    }

    /// F-25968 (the cell lookup) and its index arithmetic, run offline from the original's code on 1,114 places:
    /// the 14 of a recording of the open city, random places over and around the map, the places on cell edges
    /// (+-1 ulp) and NaN / infinite / huge ones. The scale and bias the original reads are the recorded floats.
    #[test]
    fn lamp_cell_index_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for l in include_str!("testdata/lamp_cell_ref.txt").lines() {
            let t: Vec<&str> = l.split_whitespace().collect();
            assert_eq!((t[0], t[1]), ("512", "640"));
            // the constants the port uses are the floats the original's builder stored
            assert_eq!((1.0f32 / 6320.0).to_bits(), f(t[2]).to_bits());
            assert_eq!((1.0f32 / 4463.75).to_bits(), f(t[4]).to_bits());
            assert_eq!(0.54857594f32.to_bits(), (-f(t[5])).to_bits());
            assert_eq!(0.5282554f32.to_bits(), (-f(t[7])).to_bits());
            let p = Vec3::new(f(t[8]), f(t[9]), f(t[10]));
            let want: usize = t[11].parse().unwrap();
            assert_eq!(lamp_cell_index(p), want, "{l}");
            n += 1;
        }
        assert!(n > 1000);
    }

    #[test]
    fn lamp_query_matches_the_original() {
        // (the harness's cells: a hash of the cell index, few distinct ids)
        let cells: Vec<[u16; 4]> = (0..LAMP_GRID.0 * LAMP_GRID.1)
            .map(|i| {
                let (h, g) = (i.wrapping_mul(2654435761), (i + 12345).wrapping_mul(2246822519));
                [(h >> 7 & 63) as u16, (h >> 13 & 63) as u16, (g >> 7 & 63) as u16, (g >> 13 & 63) as u16]
            })
            .collect();
        let t = LampTables { cells, recs: Vec::new() };
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for l in include_str!("testdata/lamp_query_ref.txt").lines() {
            let v: Vec<&str> = l.split_whitespace().collect();
            let want: Vec<u16> = v[5..].iter().map(|x| x.parse().unwrap()).collect();
            assert_eq!(want.len(), v[4].parse::<usize>().unwrap());
            assert_eq!(t.lamp_query(Vec3::new(f(v[0]), 0.0, f(v[1])), f(v[2]), v[3].parse().unwrap()), want, "{l}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }
}

pub static LAMP_TABLES: std::sync::RwLock<Option<Arc<LampTables>>> = std::sync::RwLock::new(None);

/// The lamp table's half float as the game writes it (F-70220): cut off, not rounded; NaN and |v| < 2^-14 give 0,
/// beyond +-65504 it is clamped. Checked against the original's code in `lightref.rs`.
pub fn half_cut(v: f32) -> u16 {
    const TINY: f32 = 6.103515625e-05;
    if v.is_nan() || (v < TINY && v > -TINY) {
        return 0;
    }
    let b = v.clamp(-65504.0, 65504.0).to_bits();
    ((((b >> 23) & 0x1ff) + 16) << 10 | (b >> 13) & 0x3ff | (b >> 16) & 0x8000) as u16
}

/// The value a half float of `half_cut` reads back as (it writes no subnormals).
pub fn half_value(h: u16) -> f32 {
    let e = (h >> 10) & 0x1f;
    if e == 0 {
        return if h & 0x8000 != 0 { -0.0 } else { 0.0 };
    }
    f32::from_bits((h as u32 & 0x8000) << 16 | (e as u32 + 112) << 23 | (h as u32 & 0x3ff) << 13)
}

/// The night grid's cell lists: each cell keeps (score, 1-based lamp number) of the four lamps that light its middle most.
///
/// The game's own rule (its grid builder, F-12750 of the executable, LIGHTING_WORK.md):
/// a lamp reaches as far as it still gives a hundredth of its light; a spot that points
/// down covers the stretch of ground (15 m below) its cone falls on instead. A cell keeps
/// the four lamps with the highest score `term / distance squared` taken at the cell's
/// centre from the lamp's own position, the score a 16 bit number in steps of 1 / 256
/// that wraps round; equal scores keep the earlier lamp.
pub fn bin_lamps(lamps: &[Lamp]) -> Vec<[(u16, u16); 4]> {
    let (gw, gh) = (LAMP_GRID.0 as usize, LAMP_GRID.1 as usize);
    let (cw, ch) = (LAMP_SPAN.0 / gw as f32, LAMP_SPAN.1 / gh as f32);
    let mut best = vec![[(0u16, 0u16); 4]; gw * gh];
    for (i, l) in lamps.iter().enumerate() {
        if l.intensity <= 0.0 {
            continue;
        }
        let reach = (l.intensity * 100.0).sqrt();
        let (mut cx, mut cz, mut radius) = (l.pos.x, l.pos.z, reach);
        let dir = -l.back;
        if l.cone[0] > 0.0 && dir.y < 0.0 {
            let half = (-l.cone[1] / l.cone[0]).clamp(-1.0, 1.0).acos();
            let tilt = (dir.x * dir.x + dir.z * dir.z).sqrt().atan2(-dir.y);
            let (mut far, mut near) = ((tilt + half).tan() * 15.0, (tilt - half).tan() * 15.0);
            if far.abs() > reach {
                far = reach;
            }
            if near.abs() > reach {
                near = -reach;
            }
            let flat = dir.x * dir.x + dir.z * dir.z;
            let (hx, hz) = if flat < 0.001 { (1.0, 0.0) } else { (dir.x / flat.sqrt(), dir.z / flat.sqrt()) };
            cx += hx * (far + near) * 0.5;
            cz += hz * (far + near) * 0.5;
            radius = (far - near) * 0.5;
        }
        let term = radius * radius * 0.01 * l.intensity;
        let cell = |v: f32, origin: f32, span: f32, n: usize| -> usize { ((((v - origin) / span) * n as f32) as i64).clamp(0, n as i64 - 1) as usize };
        let (x0, x1) = (cell(cx - radius, LAMP_ORIGIN.0, LAMP_SPAN.0, gw), cell(cx + radius, LAMP_ORIGIN.0, LAMP_SPAN.0, gw));
        let (z0, z1) = (cell(cz - radius, LAMP_ORIGIN.1, LAMP_SPAN.1, gh), cell(cz + radius, LAMP_ORIGIN.1, LAMP_SPAN.1, gh));
        for gz in z0..=z1 {
            for gx in x0..=x1 {
                let (mx, mz) = (LAMP_ORIGIN.0 + (gx as f32 + 0.5) * cw, LAMP_ORIGIN.1 + (gz as f32 + 0.5) * ch);
                let score = ((cell_score(true, term, (l.pos.x, l.pos.z), (mx, mz)) * 256.0) as i64 & 0xffff) as u16;
                if score == 0 {
                    continue;
                }
                let slot = &mut best[gz * gw + gx];
                if let Some(k) = slot.iter().position(|s| score > s.0) {
                    slot.copy_within(k..3, k + 1);
                    slot[k] = (score, (i + 1) as u16);
                }
            }
        }
    }
    best
}

/// One lamp's row of the three lamp tables as the game's writer (F-85100) stores it: `a` = x, z, cone scale, cone bias as
/// cut half floats; `b` = the intensity over 256 and the colour halved, each clamped to 0..1, times 255, cut to a byte;
/// `c` = the height byte (`y x 0.00909091 + 0.28181821` as one fused multiply-add) and the axis bytes (`0.5 + 0.5 x back`).
/// Checked on 36 calls of the original's writer and 391 lamps of its whole tables (`lamp_row_tests`).
pub struct LampRow {
    pub a: [u16; 4],
    pub b: [u8; 4],
    pub c: [u8; 4],
}

pub fn lamp_row(l: &Lamp) -> LampRow {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    let y = (0.00909091f32 as f64).mul_add(l.pos.y as f64, 0.28181821f32 as f64) as f32;
    let axis = |v: f32| byte(0.5 + 0.5 * v);
    LampRow {
        a: [half_cut(l.pos.x), half_cut(l.pos.z), half_cut(l.cone[0]), half_cut(l.cone[1])],
        b: [byte(l.intensity / 256.0), byte(l.color.x * 0.5), byte(l.color.y * 0.5), byte(l.color.z * 0.5)],
        c: [byte(y), axis(l.back.x), axis(l.back.y), axis(l.back.z)],
    }
}

#[cfg(test)]
mod lamp_row_tests {
    use super::*;

    /// The writer's rows (F-85100): 36 recorded calls of the original (the record it was given and the row it wrote into
    /// the three tables) and 391 lamps of the whole tables of an open-city build (every 40th of the 15,174 lamps whose cone
    /// halves agree; the cone itself comes from the lamp loader, not the writer). `tools/objdump_export.py lamprow`.
    #[test]
    fn lamp_row_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for line in include_str!("testdata/lamp_row_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let v: Vec<f32> = i.split(' ').map(f).collect();
            let l = Lamp { pos: Vec3::new(v[0], v[1], v[2]), color: Vec3::new(v[3], v[4], v[5]), back: Vec3::new(v[6], v[7], v[8]), intensity: v[9], cone: [v[10], v[11]] };
            let r = lamp_row(&l);
            let w: Vec<&str> = o.split(' ').collect();
            let a: Vec<u16> = w[..4].iter().map(|x| u16::from_str_radix(x, 16).unwrap()).collect();
            assert_eq!(r.a.to_vec(), a, "{line}");
            assert_eq!(r.b.iter().map(|b| format!("{b:02x}")).collect::<String>(), w[4], "{line}");
            assert_eq!(r.c.iter().map(|b| format!("{b:02x}")).collect::<String>(), w[5], "{line}");
            n += 1;
        }
        assert!(n > 300);
    }
}

#[cfg(test)]
mod lamp_tile_tests {
    use super::*;

    /// The night grid's cell lists (the footprint F-58353, the score F-47404 and the builder's four-best rule F-12750):
    /// two windows of the open city, the lamps that can reach them and the original's list of every cell inside whose lamps are
    /// all from the checked rows (the original's whole tables decoded with its own lookup, `lamp_grid_reference.py`;
    /// `objdump_export.py lamptile`). The lists come out equal in rank order.
    #[test]
    fn cell_lists_match_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let (mut lamps, mut rows, mut cells) = (Vec::new(), Vec::new(), Vec::new());
        for line in include_str!("testdata/lamp_tile_ref.txt").lines() {
            let t: Vec<&str> = line.split(' ').collect();
            if t[0] == "L" {
                let v: Vec<f32> = t[2..].iter().map(|x| f(x)).collect();
                rows.push(t[1].parse::<u16>().unwrap());
                lamps.push(Lamp { pos: Vec3::new(v[0], v[1], v[2]), color: Vec3::new(v[3], v[4], v[5]), back: Vec3::new(v[6], v[7], v[8]), intensity: v[9], cone: [v[10], v[11]] });
            } else {
                cells.push(t[1..].iter().map(|x| x.parse::<usize>().unwrap()).collect::<Vec<_>>());
            }
        }
        let best = bin_lamps(&lamps);
        for c in &cells {
            // (the windows' lamps are listed in the original's order, so the ties keep the same lamp)
            let got: Vec<usize> = best[c[0]].iter().map(|s| if s.1 == 0 { 0 } else { rows[s.1 as usize - 1] as usize }).collect();
            assert_eq!(got, c[1..].to_vec(), "cell {}", c[0]);
        }
        assert!(cells.len() > 1000);
    }
}

/// A lamp's score at a grid cell's centre (F-47404): its term over the flat distance squared - the distance's square
/// root taken and squared again, as the game does; 0 for a lamp that is not lit. The f32 steps of the original.
pub fn cell_score(lit: bool, term: f32, lamp: (f32, f32), at: (f32, f32)) -> f32 {
    if !lit {
        return 0.0;
    }
    let (dx, dz) = (lamp.0 - at.0, lamp.1 - at.1);
    let d = ((dz as f64).mul_add(dz as f64, (dx * dx) as f64) as f32).sqrt();
    term / (d * d)
}

/// One of the city's placed lamps: position, colour (as in the light files) and intensity (the
/// light is `colour / 2 x sat(intensity / distance squared)`, see city.wgsl `grid_lamps`).
#[derive(Clone, Copy, Debug)]
pub struct Lamp {
    pub pos: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    /// spots: the direction from the lit ground back to the lamp, and the cone as
    /// sat(cos x scale + bias); lamps that shine all round: scale 0, bias 1
    pub back: Vec3,
    pub cone: [f32; 2],
}
/// edge of a face of the environment cube map
pub const ENV_SIZE: u32 = 128;
/// most light glows drawn into the environment map (the original draws 64 + 27 at the
/// reference spot)
pub const ENV_GLOWS: usize = 96;
/// edge of a face of the blurred environment cube
pub const ENV_SOFT_SIZE: u32 = 16;
/// edge of one paraboloid half of the environment map as the city is drawn into it
pub const ENV_DP_SIZE: u32 = 256;

/// The Xenos's pixel centres (PA_SU_VTX_CNTL bit 0, set by the D3D library's render-state setter F-93002): every pass
/// the original draws with geometry - scene, sky, cars, people, props, glows, the occlusion blobs, the soft particles,
/// the env map's halves, the road's reflection, the mirror, the shadow and impostor maps - has pix_center 0
/// (kD3DZero, recorded 0x4: pixel centres on whole positions, as D3D9), only the height fog and the bloom /
/// luminance chain have 1 (0x5; LIGHTING.md "Half pixel"). A pixel of such a pass is the sample at its corner, so
/// the picture lies half a pixel right of and below what wgpu (centres at .5) draws from the same vertices. Ported as
/// that translation of the projection: +1 / w, -1 / h in clip x, y (x w) for a pass `w` x `h` pixels big.
/// MCLA_HALF_PIXEL_OFF=1: none, as until 2026-10-29. (The cascades, the far collector and the damage map carry their
/// half texel in their own look-ups / matrices already - render/mod.rs `set_cascades`, `collector_pass`.)
pub fn half_pixel_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MCLA_HALF_PIXEL_OFF").map_or(true, |v| v.trim() != "1"))
}

/// `vp` moved by half a pixel of a `w` x `h` pass (`half_pixel_on`).
pub fn half_pixel(vp: Mat4, w: f32, h: f32) -> Mat4 {
    if half_pixel_on() && w > 0.0 && h > 0.0 {
        Mat4::from_translation(Vec3::new(1.0 / w, -1.0 / h, 0.0)) * vp
    } else {
        vp
    }
}
/// levels of the environment cube: the base and three made from it, as the original's map has (levels 0 .. 3,
/// PSFastMipMap; env_mip.wgsl)
pub const ENV_MIPS: u32 = 4;

/// Dynamic lights near the camera (street lamps, headlights).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct LightSet {
    /// x = number of lights
    pub count: [u32; 4],
    /// xyz position, w = range
    pub pos_range: [[f32; 4]; MAX_LIGHTS],
    /// rgb colour * intensity, w = spot cosine (-1 = omni)
    pub color: [[f32; 4]; MAX_LIGHTS],
    /// xyz spot direction
    pub dir: [[f32; 4]; MAX_LIGHTS],
}

impl Default for LightSet {
    fn default() -> Self {
        LightSet { count: [0; 4], pos_range: [[0.0; 4]; MAX_LIGHTS], color: [[0.0; 4]; MAX_LIGHTS], dir: [[0.0; 4]; MAX_LIGHTS] }
    }
}

impl Default for Globals {
    fn default() -> Self {
        Globals {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            cam_pos: [0.0; 4],
            sun_dir: [0.3, 0.8, 0.5, 1.0],
            sun_color: [1.0, 0.95, 0.85, 1.0],
            ambient_sky: [0.35, 0.42, 0.55, 1.0],
            ambient_ground: [0.18, 0.16, 0.14, 1.0],
            fog_color: [0.55, 0.62, 0.75, 1.0],
            params: [300.0, 2500.0, 1.2, 0.0],
            inv_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            sky_zenith: [0.35, 0.5, 0.75, 1.0],
            sky_horizon: [0.6, 0.65, 0.75, 0.0],
            sky_sunset: [0.9, 0.5, 0.38, 0.0],
            true_sun: [0.3, 0.8, 0.5, 0.0],
            moon_dir: [-0.072, 0.215, 0.974, 0.0],
            light_vp: Mat4::IDENTITY.to_cols_array_2d(),
            shadow_params: [0.0, 1.0 / SHADOW_SIZE as f32, 0.0, 0.0],
            cloud_color: [1.0, 1.0, 1.0, 0.0],
            cloud_shade: [0.6, 0.6, 0.65, 0.0],
            tone: [1.25, 0.21646, 0.9, 0.0],
            grade: [1.0, 1.0, 1.0, 0.0],
            fog_east: [0.14, 0.19, 0.2, 0.25],
            fog_west: [0.16, 0.2, 0.2, 0.0],
            dome: [1.0, 4.0, 0.8, 0.0],
            car_key: [0.0, 1.0, 0.0, 0.473],
            car_key_col: [0.0; 4],
            key_dir: [0.0, 1.0, 0.0, 0.0],
            key_col: [0.0; 4],
            collector: [0.0; 4],
            car_key2: [0.0, 1.0, 0.0, 0.0],
            car_key2_col: [0.0; 4],
            car_amb: [0.0; 4],
            lod: [1.0, 1.0, 1.0, 0.0],
            lod2: [1.0, 0.0, 0.0, 0.0],
        }
    }
}

/// `mat_bgs` key bit of the surf strip's material (`pipeline_shore`): the blended mode with `MaterialUniform::_pad[1]` = 1,
/// which tells city.wgsl to run the game's programs on the strip (world.rs `mark_shore`)
pub const SHORE_PROGRAM: u32 = 0x200;
/// `mat_bgs` key bit of the sea's unfogged material (`pipeline_sea`)
pub const SEA_NOFOG: u32 = 0x100;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct MaterialUniform {
    pub mode: u32,
    pub _pad: [u32; 3],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MatMode {
    Opaque = 0,
    AlphaTest = 1,
    Blend = 2,
    Window = 3,
    Road = 4,
    Neon = 5,
    Paint = 6,
    Chrome = 7,
    Light = 8,
    Object = 9,
    Rubber = 10,
    Marker = 11,
    /// vehicle window glass (blended, tinted, reflective)
    CarGlass = 12,
    /// black glossy plastic / opaque black glass / grills (alpha-tested, dark, reflective)
    BlackGloss = 13,
    /// wall / object surface with a decal atlas overlay on the second UV set
    Decal = 14,
    /// leaf cards: alpha-tested, wrap-lit, translucent
    Foliage = 15,
    /// traffic signal head: the lit lamp follows the per-instance signal state
    Signal = 16,
    /// cabin trim: light pattern textures tinted with the interior colour
    Interior = 17,
    /// wheel rim: material zone in uv1.x (0 = dark barrel, others bright metal), AO in colour alpha
    Rim = 18,
    /// glass of ambient traffic cars: dark, opaque, mirror-like (they have no cabin)
    DarkGlass = 19,
    /// ground drawn in the texture's own colours (CitySidewalk, CityGroundGrime*, CityRiverRoad;
    /// `Road` - CityRoad, CityTunnelRoad - takes the red channel as grey), with grime and decals
    Ground = 20,
    /// hillsides and lawns (PSCityTerrain, PSCityGrass): three textures mixed by a channel map
    Terrain = 21,
    /// shop fronts (PSCityWindowLower): glass one can see the shop through
    StoreGlass = 22,
    /// the shops behind them (PSCityStoreInterior): dim by day, lit up at night
    StoreInterior = 23,
    /// shop signs (PSCityStoreSign): a night texture over the sign while it is switched on
    StoreSign = 24,
    /// untextured neon tubes (PSCityNeonTint): a day colour, a night colour
    NeonTint = 25,
    /// glass props (PSPropGlass and its night program): door leaves, glass walls - blended, both sides
    PropGlass = 26,
    /// the detailed sea (CityOceanWater): the game's own programs, VSCityOceanWater (waves moved by
    /// the vertex colour) + PSCityOceanWater - xs_water.wgsl (translated at install time, xs.rs); opaque, as the original draws it
    OceanWater = 27,
    /// the far sea (CityOceanWaterLOD): flat, the same pixel program
    OceanWaterLod = 28,
}

impl MatMode {
    pub const COUNT: u32 = 29;
    pub fn blended(self) -> bool {
        matches!(self, MatMode::Blend | MatMode::CarGlass | MatMode::Marker | MatMode::StoreGlass)
    }
}

/// Which kinds of casters the cascades' depth passes cull by winding (`Gpu::casc_cull_kinds`).
pub const CASC_CULL_SEC: u32 = 1;
pub const CASC_CULL_CAR: u32 = 2;
pub const CASC_CULL_PED: u32 = 4;
pub const CASC_CULL_PROP: u32 = 8;
/// (the cars stay unculled: the original casts a car's LOD model, the port its full one - see LIGHTING_WORK.md, casc2)
pub const CASC_CULL_DEFAULT: u32 = CASC_CULL_SEC | CASC_CULL_PED | CASC_CULL_PROP;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct ObjectUniform {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
    pub params: [f32; 4],
    /// dents of a car body: where it was hit (in the car's own space) and how deep (0..1)
    pub dents: [[f32; 4]; 6],
    /// car paint: the second colour (DL3) and in w the fade type (0 none, 1 two-tone,
    /// 2 front to back, 3 left to right, 4 top to bottom)
    pub color2: [f32; 4],
    /// the body's box for the fade (DL6, DL7): its lowest corner (w unused) ...
    pub fade_min: [f32; 4],
    /// ... and its size
    pub fade_size: [f32; 4],
    /// window tint (the garage's Tint Color and Tint Transparency): colour, and in w the
    /// level 1..10 (0 = the stock glass)
    pub tint: [f32; 4],
    /// the cabin's colours (the garage's INTERIOR COLOR A, B, C, then GAUGE COLOR A, B), w 1
    /// where one is chosen: a trim mesh takes the one of its zone (uv1.x) - 9, 10, 11, 7, 0
    pub interior: [[f32; 4]; 5],
    /// added to the ambient: the night fill light at the object's place (traffic cars; fillmap.rs)
    pub fill: [f32; 4],
    /// a car's lamp table by lamp id (the second UV set of its lamp meshes; car_lamps.rs): the
    /// bulb colours - rows c128 + id of the game's gCarConstantBuffer; w of row 0 = 1 on a
    /// traffic car, whose lamps the ambient program draws ...
    pub lamp_a: [[f32; 4]; car_lamps::IDS],
    /// ... and the cover (lens) colours - rows c163 + id (traffic: c148 + id)
    pub lamp_b: [[f32; 4]; car_lamps::IDS],
    /// the player's paint areas after the body (doors, hood, front bumper, rear bumper,
    /// skirts - the parts' vertices carry the area in uv2.y, 8 + area): the first colour,
    /// w 1 = the area has a paint of its own ...
    pub area_a: [[f32; 4]; 5],
    /// ... and the second colour, w = paint type + 8 x fade
    pub area_b: [[f32; 4]; 5],
    /// a traffic or parked car's own light slots (F-99357): row 0 x = 1 the hour's light second, w = 1 given;
    /// rows 1 / 2 the other slot's direction and colour x strength, rows 3 / 4 the hour's slot's
    pub slots: [[f32; 4]; 5],
    /// a car's glass colours by window zone 0 .. 8 (the zone in uv1.x): TintColors for the windows and BumpedTintColors
    /// for the lamps' lenses, as F-50822 hands them on (`damagemap::glass_vars`)
    pub glass_tint: [[f32; 4]; 9],
    pub glass_bumped: [[f32; 4]; 9],
    /// a set-up car's part bones' rest rotations (restMtx's 3 x 3, F-14987), up to `vehicles::REST_ROTS`, nine floats
    /// each (three columns), packed: the vertex dent of a mesh whose index k > 0 (in its normal's length,
    /// `vehicles::rest_mark`) is turned by rotation k - 1 (city.wgsl vs_body)
    pub rest_rot: [[f32; 4]; 72],
    /// siColor2 (c59) of a set-up car's draws: its exterior neon's colour x pulse x intensity x 0.3 (`vehicles::ExtNeon::si_color`,
    /// F-96984 / F-56246), screened in by the third byte of the vertex colour; 0 for others
    pub neon_si: [f32; 4],
    /// the cabin trim's colours (InteriorTrim, city.wgsl mode 17): rows 0 .. 9 the vehicle record's m_vBaseColor[g] as
    /// stored (F-19629 hands row G[zone] of these to the trim program as intDL0, no gamma), rows 10 / 11 its gauge colours
    /// A / B (DashGaugeColor, DashGaugeColorB - the cabin trim's siColor0 / siColor1); w 1 = given, 0 = the old look
    pub trim: [[f32; 4]; 12],
    /// BrushedMetalTintDownload[0] (VS_BrakeRotor / VS_BumpedBrushedmetal c209, the brushed metal's tint): the vehicle
    /// record's m_vBrushedMetalColorA as stored (F-91597); w 1 = given, 0 = none (0.75, the stock row)
    pub brushed: [f32; 4],
}

impl Default for ObjectUniform {
    fn default() -> Self {
        // (lamps without a table of their own: the Audi R8's, as before the tables were read)
        let (lamp_a, lamp_b) = car_lamps::R8.uniform();
        let g = crate::damagemap::glass_vars([0.0; 4], false, false, true, false, &crate::damagemap::RECORDED_GLASS, [0.0, 1.0, 0.0], [0.0; 3], -1.0, false);
        ObjectUniform { model: Mat4::IDENTITY.to_cols_array_2d(), color: [1.0, 1.0, 1.0, 0.0], params: [0.0; 4], dents: [[0.0; 4]; 6], color2: [0.0; 4], fade_min: [0.0; 4], fade_size: [1.0; 4], tint: [0.0; 4], interior: [[0.0; 4]; 5], fill: [0.0; 4], lamp_a, lamp_b, area_a: [[0.0; 4]; 5], area_b: [[0.0; 4]; 5], slots: [[0.0; 4]; 5], glass_tint: g.tint, glass_bumped: g.bumped, rest_rot: [[0.0; 4]; 72], neon_si: [0.0; 4], trim: [[0.0; 4]; 12], brushed: [0.0; 4] }
    }
}

/// What the GPU reads of an `ObjectUniform` for each object: city.wgsl's `Object` (group 3 binding 0). The rest of it,
/// equal for every object of one car, is `CarUniform` (binding 2), written once for the car and shared by its
/// parts and windows (D59: the cars' uniforms 4.9 -> ~1.3 MB a frame with traffic; docs/reference/carparts/
/// object_uniform_split.md). The fields are the `ObjectUniform`'s, in its order.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct ObjPart {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
    pub params: [f32; 4],
    pub dents: [[f32; 4]; 6],
    pub color2: [f32; 4],
    pub fade_min: [f32; 4],
    pub fade_size: [f32; 4],
    pub tint: [f32; 4],
    pub fill: [f32; 4],
    pub neon_si: [f32; 4],
    pub trim: [[f32; 4]; 12],
    pub brushed: [f32; 4],
}

/// The part of an `ObjectUniform` that is the car's (city.wgsl's `CarBlock`, group 3 binding 2).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct CarUniform {
    pub interior: [[f32; 4]; 5],
    pub lamp_a: [[f32; 4]; car_lamps::IDS],
    pub lamp_b: [[f32; 4]; car_lamps::IDS],
    pub area_a: [[f32; 4]; 5],
    pub area_b: [[f32; 4]; 5],
    pub slots: [[f32; 4]; 5],
    pub glass_tint: [[f32; 4]; 9],
    pub glass_bumped: [[f32; 4]; 9],
    pub rest_rot: [[f32; 4]; 72],
}

impl Default for ObjPart {
    /// `ObjectUniform::default()`'s own part.
    fn default() -> Self {
        ObjPart { model: Mat4::IDENTITY.to_cols_array_2d(), color: [1.0, 1.0, 1.0, 0.0], params: [0.0; 4], dents: [[0.0; 4]; 6], color2: [0.0; 4], fade_min: [0.0; 4], fade_size: [1.0; 4], tint: [0.0; 4], fill: [0.0; 4], neon_si: [0.0; 4], trim: [[0.0; 4]; 12], brushed: [0.0; 4] }
    }
}

impl ObjectUniform {
    /// The object's own part (`ObjPart`).
    pub fn part(&self) -> ObjPart {
        ObjPart { model: self.model, color: self.color, params: self.params, dents: self.dents, color2: self.color2, fade_min: self.fade_min, fade_size: self.fade_size, tint: self.tint, fill: self.fill, neon_si: self.neon_si, trim: self.trim, brushed: self.brushed }
    }

    /// The car's part (`CarUniform`).
    pub fn car(&self) -> CarUniform {
        CarUniform { interior: self.interior, lamp_a: self.lamp_a, lamp_b: self.lamp_b, area_a: self.area_a, area_b: self.area_b, slots: self.slots, glass_tint: self.glass_tint, glass_bumped: self.glass_bumped, rest_rot: self.rest_rot }
    }

    /// The two put back together (the check `MCLA_OBJ_CHECK`: what the shader reads, in the old layout).
    pub fn join(p: &ObjPart, c: &CarUniform) -> ObjectUniform {
        ObjectUniform { model: p.model, color: p.color, params: p.params, dents: p.dents, color2: p.color2, fade_min: p.fade_min, fade_size: p.fade_size, tint: p.tint, interior: c.interior, fill: p.fill, lamp_a: c.lamp_a, lamp_b: c.lamp_b, area_a: c.area_a, area_b: c.area_b, slots: c.slots, glass_tint: c.glass_tint, glass_bumped: c.glass_bumped, rest_rot: c.rest_rot, neon_si: p.neon_si, trim: p.trim, brushed: p.brushed }
    }
}

/// MCLA_OBJ_CHECK=1: every frame, the objects the cars push are read back out of the pools' mirrors (what the GPU gets)
/// as their part and the car block they are bound to, put back into the old `ObjectUniform` and compared byte for byte
/// with what the old code pushed for them (`UniformBatch::expect`); the counts in the log every 120 frames.
pub fn obj_check_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MCLA_OBJ_CHECK").is_some())
}

/// Where an object's car block lies: its own slot, or the one of the object it shares it with (`shared`: its pushes
/// do not write it - a car's parts and windows the body's, the wheels the all-default block).
#[derive(Clone)]
struct CarRef {
    buf: wgpu::Buffer,
    off: u64,
    slot: Option<Arc<CarSlot>>,
    shared: bool,
}

/// A car block's slot in `Gpu::car_pool`; given back when the last object bound to it is dropped.
struct CarSlot {
    pool: Arc<std::sync::Mutex<ObjPool>>,
    buf: u32,
    off: u64,
}

impl Drop for CarSlot {
    fn drop(&mut self) {
        if let Ok(mut p) = self.pool.lock() {
            p.free.push((self.buf, self.off));
        }
    }
}

/// A per-object uniform buffer + bind group (model matrix, paint colour, light state).
pub struct ObjectBinding {
    pub buf: wgpu::Buffer,
    pub bg: wgpu::BindGroup,
    /// where in `buf` its uniform lies (0, or its slot in a shared buffer - `Gpu::new_pooled_object`)
    pub off: u64,
    slot: Option<PoolSlot>,
    car: CarRef,
}

/// Objects whose uniforms share buffers of `POOL_SLOTS` slots (`Gpu::new_pooled_object`, the cars' parts), each
/// buffer written once a frame for all of them by `UniformBatch` instead of a copy per object (docs/PERFORMANCE.md
/// 2026-10-24). `mirror` holds what each buffer holds - every write goes through it -, `dirty` the part written
/// since the last flush.
#[derive(Default)]
pub struct ObjPool {
    bufs: Vec<wgpu::Buffer>,
    mirror: Vec<Vec<u8>>,
    dirty: Vec<Option<(usize, usize)>>,
    free: Vec<(u32, u64)>,
    stride: u64,
}

const POOL_SLOTS: u64 = 256;

impl ObjPool {
    /// A slot's new value. The mirror holds what the buffer holds, so a value equal to it byte for byte is not
    /// written again (the parked and still cars' parts, most of a frame's ~2,300 objects in traffic; docs/PERFORMANCE.md
    /// 2026-11-09 (7)).
    fn put(&mut self, buf: u32, off: u64, b: &[u8]) {
        let (i, o) = (buf as usize, off as usize);
        let m = &mut self.mirror[i][o..o + b.len()];
        if m == b {
            return;
        }
        m.copy_from_slice(b);
        let d = &mut self.dirty[i];
        *d = Some(d.map_or((o, o + b.len()), |(lo, hi)| (lo.min(o), hi.max(o + b.len()))));
    }
}

/// An object's slot in the pool; given back when the object is dropped.
struct PoolSlot {
    pool: Arc<std::sync::Mutex<ObjPool>>,
    buf: u32,
}

impl Drop for ObjectBinding {
    fn drop(&mut self) {
        if let Some(s) = self.slot.as_ref() {
            if let Ok(mut p) = s.pool.lock() {
                p.free.push((s.buf, self.off));
            }
        }
    }
}

/// Many small uniform buffers written with one upload (docs/PERFORMANCE.md 2026-10-24): every
/// `queue.write_buffer` makes a staging buffer of its own on the device, ~1000 a frame for the
/// city's cars. The values are gathered here, written once into `arena` and copied from there
/// into each buffer by a command buffer submitted at once: what any later submit sees is the
/// same as with a `write_buffer` per buffer, in the same order (the last value of a buffer wins).
#[derive(Default)]
pub struct UniformBatch {
    bytes: Vec<u8>,
    dst: Vec<(wgpu::Buffer, u64, u64, u64)>,
    /// a pooled object's value was put in its pool's mirror (`push_object`): the pools' changed parts are written at the flush
    pooled: bool,
    arena: Option<wgpu::Buffer>,
    /// MCLA_OBJ_CHECK: the objects' old values this frame - (object pool slot, car pool slot, the old value)
    checks: Vec<((u32, u64), (u32, u64), ObjectUniform)>,
    check_tally: (u64, u64, u64),
}

impl UniformBatch {
    pub fn push<T: Pod>(&mut self, buf: &wgpu::Buffer, v: &T) {
        let b = bytemuck::bytes_of(v);
        // (copies go in steps of 4 bytes: every uniform here is a multiple of 16)
        debug_assert!(b.len() % 4 == 0);
        self.dst.push((buf.clone(), 0, self.bytes.len() as u64, b.len() as u64));
        self.bytes.extend_from_slice(b);
    }

    pub fn push_object(&mut self, b: &ObjectBinding, u: &ObjectUniform) {
        self.push_part(b, &u.part());
        if !b.car.shared {
            self.push_car(b, &u.car());
        }
    }

    /// The object's own part only: its car block is the one it shares (`Gpu::new_pooled_part`), written by the object
    /// that owns it.
    pub fn push_part(&mut self, b: &ObjectBinding, u: &ObjPart) {
        let v = bytemuck::bytes_of(u);
        match b.slot.as_ref() {
            // (into the mirror at once, not through `bytes`: the buffer is written from the mirror at the flush, as before)
            Some(s) => {
                if let Ok(mut p) = s.pool.lock() {
                    p.put(s.buf, b.off, v);
                }
                self.pooled = true;
            }
            None => {
                self.dst.push((b.buf.clone(), b.off, self.bytes.len() as u64, v.len() as u64));
                self.bytes.extend_from_slice(v);
            }
        }
    }

    fn push_car(&mut self, b: &ObjectBinding, c: &CarUniform) {
        let v = bytemuck::bytes_of(c);
        match b.car.slot.as_ref() {
            Some(s) => {
                if let Ok(mut p) = s.pool.lock() {
                    p.put(s.buf, s.off, v);
                }
                self.pooled = true;
            }
            None => {
                self.dst.push((b.car.buf.clone(), b.car.off, self.bytes.len() as u64, v.len() as u64));
                self.bytes.extend_from_slice(v);
            }
        }
    }

    /// MCLA_OBJ_CHECK: what the old code pushed for this pooled object this frame, compared at the flush with what its
    /// slot and its car block's slot hold then.
    pub fn expect(&mut self, b: &ObjectBinding, u: &ObjectUniform) {
        if let (Some(s), Some(c)) = (b.slot.as_ref(), b.car.slot.as_ref()) {
            self.checks.push(((s.buf, b.off), (c.buf, c.off), *u));
        }
    }

    fn flush(&mut self, gpu: &Gpu) {
        if !self.checks.is_empty() {
            self.check(gpu);
        }
        if std::mem::take(&mut self.pooled) {
            for pool in [&gpu.obj_pool, &gpu.car_pool] {
                if let Ok(mut p) = pool.lock() {
                    for i in 0..p.bufs.len() {
                        if let Some((lo, hi)) = p.dirty[i].take() {
                            gpu.queue.write_buffer(&p.bufs[i], lo as u64, &p.mirror[i][lo..hi]);
                        }
                    }
                }
            }
        }
        if self.dst.is_empty() {
            self.bytes.clear();
            return;
        }
        let need = self.bytes.len() as u64;
        if self.arena.as_ref().map_or(true, |a| a.size() < need) {
            self.arena = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("uniform batch"),
                size: need.next_power_of_two().max(1 << 16),
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let arena = self.arena.as_ref().unwrap();
        gpu.queue.write_buffer(arena, 0, &self.bytes);
        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("uniform batch") });
        for (buf, at, off, len) in &self.dst {
            enc.copy_buffer_to_buffer(arena, *off, buf, *at, *len);
        }
        gpu.queue.submit([enc.finish()]);
        self.bytes.clear();
        self.dst.clear();
    }

    /// MCLA_OBJ_CHECK: each expected object read back out of the mirrors (its part + its car block) against its old value.
    fn check(&mut self, gpu: &Gpu) {
        let (Ok(op), Ok(cp)) = (gpu.obj_pool.lock(), gpu.car_pool.lock()) else { return };
        let (ps, cs) = (std::mem::size_of::<ObjPart>(), std::mem::size_of::<CarUniform>());
        let (mut n, mut bad) = (0u64, 0u64);
        for ((ob, oo), (cb, co), want) in self.checks.drain(..) {
            let (oo, co) = (oo as usize, co as usize);
            let part: ObjPart = bytemuck::pod_read_unaligned(&op.mirror[ob as usize][oo..oo + ps]);
            let car: CarUniform = bytemuck::pod_read_unaligned(&cp.mirror[cb as usize][co..co + cs]);
            let got = ObjectUniform::join(&part, &car);
            n += 1;
            if bytemuck::bytes_of(&got) != bytemuck::bytes_of(&want) {
                bad += 1;
                if self.check_tally.1 + bad <= 8 {
                    let (g, w) = (bytemuck::cast_slice::<_, [f32; 4]>(bytemuck::bytes_of(&got)), bytemuck::cast_slice::<_, [f32; 4]>(bytemuck::bytes_of(&want)));
                    let rows: Vec<usize> = (0..g.len()).filter(|&i| bytemuck::bytes_of(&g[i]) != bytemuck::bytes_of(&w[i])).collect();
                    log::warn!("objcheck: an object differs in vec4 rows {rows:?} (first: got {:?} want {:?})", g[rows[0]], w[rows[0]]);
                }
            }
        }
        let t = &mut self.check_tally;
        t.0 += 1;
        t.1 += bad;
        t.2 += n;
        if t.0 % 120 == 0 {
            log::info!("objcheck: {} frames, {} objects compared, {} differ", t.0, t.2, t.1);
        }
    }
}

/// DX12's compiled programs kept on disk (GitHub #33: DXC took 14 s for the city's general fragment program and so on,
/// at every start): the patched wgpu-hal (vendor/wgpu-hal) reads and writes them in the folder `MCLA_DXIL_CACHE`
/// names, here `<user folder>\shadercache\dx12`. A file's name is the hash of everything that goes into its compile
/// (the HLSL text, entry point, profile, flags, wgpu-hal's and DXC's versions), so it stays right across the game's
/// updates: a program an update did not change is not compiled again (GitHub #48: a folder per version, the others
/// removed, made every update's first DX12 start compile all ~170 programs, 1 - 2 s each in DXC). The newest folder
/// of the old kind (`dx12-<version>`) becomes it once, the rest are removed; over 64 MB (~2.6 MB a version) it is
/// emptied. A set `MCLA_DXIL_CACHE` is kept (empty: no cache, a test hook).
fn dxil_cache_dir() {
    if std::env::var_os("MCLA_DXIL_CACHE").is_some() {
        return;
    }
    let root = mcla_config::paths::user_data_dir().join("shadercache");
    let dir = root.join("dx12");
    let mut old: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(&root)
        .map(|l| l.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("dx12-")).map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path())).collect())
        .unwrap_or_default();
    old.sort();
    if !dir.is_dir() {
        if let Some((_, newest)) = old.pop() {
            match std::fs::rename(&newest, &dir) {
                Ok(()) => log::info!("shader cache: {} kept as {}", newest.display(), dir.display()),
                Err(err) => log::warn!("shader cache: could not keep {}: {err}", newest.display()),
            }
        }
    }
    for (_, p) in old {
        match std::fs::remove_dir_all(&p) {
            Ok(()) => log::info!("shader cache: removed the old {}", p.display()),
            Err(err) => log::warn!("shader cache: could not remove {}: {err}", p.display()),
        }
    }
    let files: Vec<(std::path::PathBuf, u64)> = std::fs::read_dir(&dir).map(|l| l.flatten().filter_map(|e| Some((e.path(), e.metadata().ok()?.len()))).collect()).unwrap_or_default();
    let bytes: u64 = files.iter().map(|f| f.1).sum();
    if bytes > 64 << 20 {
        log::info!("shader cache: {} files, {} MB - emptied", files.len(), bytes >> 20);
        for (p, _) in files {
            let _ = std::fs::remove_file(p);
        }
    }
    log::info!("shader cache: {}", dir.display());
    std::env::set_var("MCLA_DXIL_CACHE", dir);
}

/// The device's memory blocks: wgpu's default (`Performance`: on DX12 gpu-allocator's 256 MB blocks, freed only when
/// empty). `MCLA_MEM_BLOCK_MB=<n>`: blocks of n MB (a test hook, GitHub #48: DX12 held ~1.1 GB more than Vulkan for the
/// same textures); `MCLA_MEM_BLOCK_MB=usage`: wgpu's `MemoryUsage` (8 MB).
fn memory_hints() -> wgpu::MemoryHints {
    match std::env::var("MCLA_MEM_BLOCK_MB").ok().as_deref().map(str::trim) {
        Some("usage") => wgpu::MemoryHints::MemoryUsage,
        Some(v) => match v.parse::<u64>() {
            Ok(n) if n > 0 => {
                let b = n << 20;
                log::info!("memory blocks: {n} MB");
                wgpu::MemoryHints::Manual { suballocated_device_memory_block_size: b..b }
            }
            _ => wgpu::MemoryHints::Performance,
        },
        None => wgpu::MemoryHints::Performance,
    }
}

pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    /// its size in video memory (all levels), counted in `TEX_BYTES` while it lives
    bytes: TexBytes,
}

/// A texture's share of `TEX_BYTES` / `TEX_COUNT`, given back when it goes.
pub struct TexBytes(u64);

impl Drop for TexBytes {
    fn drop(&mut self) {
        TEX_BYTES.fetch_sub(self.0, std::sync::atomic::Ordering::Relaxed);
        TEX_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The bytes and the number of the `GpuTexture`s alive (the perf line, GitHub #48: a 4 GB card ran out of video memory).
pub static TEX_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static TEX_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl GpuTexture {
    pub fn new(texture: wgpu::Texture, view: wgpu::TextureView, width: u32, height: u32) -> GpuTexture {
        let bytes = texture_bytes(&texture);
        TEX_BYTES.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
        TEX_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        GpuTexture { texture, view, width, height, bytes: TexBytes(bytes) }
    }

    /// What it takes in video memory (`texture_bytes`).
    pub fn bytes(&self) -> u64 {
        self.bytes.0
    }
}

/// What a texture takes in video memory: every level's blocks times the block's size, the layers and samples.
pub fn texture_bytes(t: &wgpu::Texture) -> u64 {
    let f = t.format();
    let (bw, bh) = f.block_dimensions();
    let block = f.block_copy_size(None).or_else(|| f.block_copy_size(Some(wgpu::TextureAspect::DepthOnly))).unwrap_or(4) as u64;
    let s = t.size();
    let mut sum = 0u64;
    for l in 0..t.mip_level_count() {
        let (w, h) = ((s.width >> l).max(1), (s.height >> l).max(1));
        sum += w.div_ceil(bw) as u64 * h.div_ceil(bh) as u64 * block;
    }
    sum * s.depth_or_array_layers as u64 * t.sample_count() as u64
}

/// `create_texture` with an out-of-memory failure caught (wgpu's default handler panics on it - GitHub #48, a licence
/// plate's texture on a full 4 GB card): None then, logged. Both filters: wgpu 24 files the allocator's "Not enough memory
/// left" of `create_texture` as a validation error (its OutOfMemory test does not see through CreateTextureError - the
/// reporter's 0.20.0 crash), and the port's own descriptors are valid, so either error here is the memory.
pub fn try_create_texture(device: &wgpu::Device, desc: &wgpu::TextureDescriptor) -> Option<wgpu::Texture> {
    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let t = device.create_texture(desc);
    let validation = pollster::block_on(device.pop_error_scope());
    let oom = pollster::block_on(device.pop_error_scope());
    match validation.or(oom) {
        None => Some(t),
        Some(e) => {
            log::warn!("video memory full: texture {:?} ({}x{}) not made, drawn white: {e}", desc.label, desc.size.width, desc.size.height);
            None
        }
    }
}

/// The stand-in for a texture that could not be made: one white texel (`try_create_texture`).
fn white_texel(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> GpuTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("out of memory"),
        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if format.is_srgb() { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &[255, 255, 255, 255],
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&Default::default());
    GpuTexture::new(texture, view, 1, 1)
}

/// [`Gpu::base`]: the device and the window's surface, before the renderer's programs are made.
pub struct GpuBase {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: Arc<wgpu::Surface<'static>>,
    pub config: wgpu::SurfaceConfiguration,
    pub bc_supported: bool,
    pub max_tex: u32,
}

pub struct Gpu {
    /// the shared buffers of the pooled objects' uniforms (`new_pooled_object`)
    pub obj_pool: Arc<std::sync::Mutex<ObjPool>>,
    /// the pooled objects' car blocks (`CarUniform`, `new_pooled_object`)
    pub car_pool: Arc<std::sync::Mutex<ObjPool>>,
    /// the all-default car block the wheels share (`new_pooled_on_default_car`)
    default_car: std::sync::OnceLock<CarRef>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: Arc<wgpu::Surface<'static>>,
    pub config: wgpu::SurfaceConfiguration,
    /// the scene's depth, resolved (sample 0 of the multisampled depth, `resolve_scene_depth`): what every pass
    /// after the scene reads (particles' soft fade, blobs, glows, depth of field, the GPS map's x-ray)
    pub depth: wgpu::TextureView,
    /// The scene's multisampled targets (render/ms.rs): the sample count of this frame (`set_scene_samples`), the
    /// targets per count (made when first used), the stencil resolved like the depth (R8Unorm, s / 255: the
    /// original's 8888 copy of its depth-stencil that PSStreakMotionBlur reads), the resolve's programs.
    pub scene_n: u32,
    scene_sets: [Option<SceneSet>; 3],
    /// when each set was last the frame's (`set_scene_samples` frees one unused for `SCENE_SET_IDLE`)
    scene_used: [Option<std::time::Instant>; 3],
    pub stencil_copy: wgpu::TextureView,
    resolve_layouts: [wgpu::BindGroupLayout; 2],
    resolve_pipes: [wgpu::RenderPipeline; 2],
    /// RENDER SCALE (hooks::render_scale): the scale in force and the scene's size (window x scale: every scene
    /// target, `depth`, `stencil_copy`, the post chain); `depth_ui` = the depth at the window's size for the UI pass
    /// and the GPS map's x-ray (`depth` itself at scale 1, else a point-sampled copy, `depth_to_ui`)
    pub render_scale: f32,
    pub scene_w: u32,
    pub scene_h: u32,
    pub depth_ui: wgpu::TextureView,
    depth_down: (wgpu::BindGroupLayout, wgpu::RenderPipeline),
    depth_down_bg: Option<wgpu::BindGroup>,
    /// the globals' bind group with the city's sampler at maximum anisotropy 16 (F-23692: while the scene
    /// is 4x) while `globals_bg` has the device default 4 (`*(0x827E19C4)`) - `set_aniso16` swaps them
    pub globals_bg_alt: wgpu::BindGroup,
    pub aniso16: bool,
    /// off-screen scene target + bloom / motion blur chain
    pub post: post::Post,
    pub globals_buf: wgpu::Buffer,
    pub lights_buf: wgpu::Buffer,
    pub globals_bg: wgpu::BindGroup,
    /// the environment cube map: its six faces to draw into, their depth buffer, and the
    /// globals (buffer + bind group) each face is drawn with
    pub env_faces: Vec<wgpu::TextureView>,
    pub env_depth: wgpu::TextureView,
    pub env_globals: Vec<wgpu::Buffer>,
    pub env_bgs: Vec<wgpu::BindGroup>,
    /// 512 x 512 RGBA: green = rubbed paint, alpha = scratches to the metal
    pub scrape_tex: wgpu::Texture,
    /// the garage's lights as the cars' mirror map shows them (`set_garage_glow`): 256 x 128, the two
    /// paraboloid halves side by side; black outside the garage
    pub garage_glow_tex: wgpu::Texture,
    /// the night lamp grid and the lamps it numbers (`set_lamps`)
    pub lamp_grid: wgpu::Texture,
    pub lamp_data: wgpu::Texture,
    /// the city pipelines' four groups (globals, textures, material, object)
    pub globals_layout: wgpu::BindGroupLayout,
    pub tex_layout: wgpu::BindGroupLayout,
    /// a draw's fetch masks by its code's low six bits (render/aniso.rs; the own samplers are globals bindings 17 - 19)
    pub own_masks: Vec<wgpu::Buffer>,
    pub mat_layout: wgpu::BindGroupLayout,
    pub mat_bgs: HashMap<u32, wgpu::BindGroup>,
    pub obj_layout: wgpu::BindGroupLayout,
    /// the object's damage map when it has none (all zero)
    pub blank_dmg: wgpu::TextureView,
    pub identity_obj: ObjectBinding,
    pub pipeline_opaque: ms::MsPipe,
    /// the same for one material (`mode_constant`, `MODE_PIPES`; None, or not `ready` yet: that material draws with
    /// `pipeline_opaque`), by MatMode
    pub pipeline_opaque_mode: Vec<Option<ms::MsPipe>>,
    /// the sea after the ground dome: [0] where the stencil is not 1 (fogged), [1] where it is (`SEA_NOFOG`, unfogged)
    pub pipeline_sea: [ms::MsPipe; 2],
    /// the surf strip after the sea (`SHORE_PROGRAM`, world.rs `mark_shore`)
    pub pipeline_shore: ms::MsPipe,
    /// a set-up car's opaque batches with back faces culled (`GpuBatch::cull`): the view as is, a mirrored view
    pub pipeline_opaque_cull: ms::MsPipe,
    pub pipeline_opaque_cull_m: ms::MsPipe,
    /// `pipeline_opaque_cull` for one material (`CAR_MODE_PIPES`), as `pipeline_opaque_mode`
    pub pipeline_opaque_cull_mode: Vec<Option<ms::MsPipe>>,
    /// the stencil written and nothing else (the wheels' blur discs)
    pub pipeline_stencil_only: ms::MsPipe,
    pub pipeline_blend: ms::MsPipe,
    /// car windows: back faces, then front faces, both with depth writes
    pub pipeline_glass_back: ms::MsPipe,
    pub pipeline_glass_front: ms::MsPipe,
    /// the player's windows in the driver's view (interiorwindowglass): the back faces' cull, no depth writes
    pub pipeline_glass_inner: ms::MsPipe,
    /// shop panes (PSCityWindowLower and its night program): blended WITH depth writes, like the original's draws
    pub pipeline_pane: ms::MsPipe,
    pub pipeline_sky: ms::MsPipe,
    /// the game's own sky dome programs and their Perlin pass (skyhat.rs)
    pub sky_hat: skyhat::SkyHat,
    /// opaque / alpha-tested geometry drawn with per-instance transforms (props)
    pub pipeline_inst: ms::MsPipe,
    /// the same for one material (`PROP_MODE_PIPES`; None, or not `ready` yet: `pipeline_inst`), by MatMode
    pub pipeline_inst_mode: Vec<Option<ms::MsPipe>>,
    /// the glass props: instanced, blended, both sides, depth tested but not written (the original's draws: blend
    /// control 0x07060706, cull register 0x18000, depth control 0x00708763)
    pub pipeline_inst_blend: ms::MsPipe,
    /// additive light glows (instanced billboards)
    pub pipeline_glow: ms::MsPipe,
    /// the light glows of the environment map (env_glow.wgsl) and their instances
    /// (position + strength, colour: 8 floats each, at most ENV_GLOWS)
    pub pipeline_env_glow: wgpu::RenderPipeline,
    /// the blurred copy of the environment cube (env_blur.wgsl): its pipeline, the sharp
    /// cube as its input, and its six faces to draw into
    pub pipeline_env_blur: wgpu::RenderPipeline,
    /// the cars' map drawn the original's way: the city into two paraboloid halves (warp per
    /// vertex), the cube filled from them
    pub pipeline_env_dp: ms::MsPipe,
    pub pipeline_env_dp_blend: ms::MsPipe,
    pub env_dp_views: Vec<wgpu::TextureView>,
    pub env_dp_depth: wgpu::TextureView,
    pub pipeline_env_fill: wgpu::RenderPipeline,
    pub env_fill_bg: wgpu::BindGroup,
    pub env_blur_bg: wgpu::BindGroup,
    pub env_soft_faces: Vec<wgpu::TextureView>,
    /// the cube's levels 1 .. ENV_MIPS - 1 (env_mip.wgsl): per (level - 1) x 6 + face, the face's view of that level
    /// to draw into and the bind group reading the level above
    pub pipeline_env_mip: wgpu::RenderPipeline,
    pub env_mip_views: Vec<wgpu::TextureView>,
    pub env_mip_bgs: Vec<wgpu::BindGroup>,
    /// the cars' environment cube and its blurred copy (kept for `dump_env`)
    pub env_tex: wgpu::Texture,
    pub env_soft_tex: wgpu::Texture,
    pub env_glow_buf: wgpu::Buffer,
    pub pipeline_particle: ms::MsPipe,
    pub pipeline_particle_add: ms::MsPipe,
    pub particle_depth_layout: wgpu::BindGroupLayout,
    /// sun shadow map
    pub shadow_view: wgpu::TextureView,
    pub shadow_buf: wgpu::Buffer,
    pub shadow_bg: wgpu::BindGroup,
    pub pipeline_shadow: wgpu::RenderPipeline,
    pub pipeline_shadow_inst: wgpu::RenderPipeline,
    /// The original's four shadow cascades (`set_cascades`): the 2 x 2 depth map, per tile the
    /// pass's uniform and bind group, the scene's constants (binding 12 of the globals) and
    /// the depth pipelines.
    pub casc_view: wgpu::TextureView,
    pub casc_tex: wgpu::Texture,
    pub casc_pass_bufs: Vec<wgpu::Buffer>,
    pub casc_pass_bgs: Vec<wgpu::BindGroup>,
    pub casc_buf: wgpu::Buffer,
    pub pipeline_casc: wgpu::RenderPipeline,
    pub pipeline_casc_inst: wgpu::RenderPipeline,
    /// The same two with the original's culling by winding (shadow.wgsl fs_shadow_casc_cull), and which kinds
    /// of casters take them (`CASC_CULL_*` bits; MCLA_CASC_CULL).
    pub pipeline_casc_cull: wgpu::RenderPipeline,
    pub pipeline_casc_inst_cull: wgpu::RenderPipeline,
    pub casc_cull_kinds: u32,
    /// The fast shadow collector (fast_collector.wgsl): the working map the far city's ground
    /// is drawn into, the blurred map the scene reads (`t_fc` of the globals), their depth
    /// buffer, and the two steps' pipelines and bind groups.
    pub fc_work: wgpu::Texture,
    pub fc_tex: wgpu::Texture,
    /// last frame's collector (what PS_Copy lays over the fresh one)
    pub fc_prev: wgpu::Texture,
    pub fc_work_view: wgpu::TextureView,
    pub fc_view: wgpu::TextureView,
    pub fc_depth: wgpu::TextureView,
    pub fc_buf: wgpu::Buffer,
    pub fc_draw_bg: wgpu::BindGroup,
    pub fc_blur_bg: wgpu::BindGroup,
    pub pipeline_fc: wgpu::RenderPipeline,
    pub pipeline_fc_blur: wgpu::RenderPipeline,
    /// The night fast collector (`night_collector_begin`): the casters' map, the blurred map bound where the collector
    /// is at night (`globals_bg_night`), the casters' constants and bind group (the shadow passes' layout), the
    /// pipelines (plain, instanced, blur) and the blur's bind group.
    pub nfc_tex: wgpu::Texture,
    pub nfc_work_view: wgpu::TextureView,
    pub nfc_view: wgpu::TextureView,
    pub nfc_buf: wgpu::Buffer,
    pub nfc_bg: wgpu::BindGroup,
    pub pipeline_nfc: wgpu::RenderPipeline,
    pub pipeline_nfc_inst: wgpu::RenderPipeline,
    pub pipeline_nfc_blur: wgpu::RenderPipeline,
    pub nfc_blur_bg: wgpu::BindGroup,
    /// The globals' bind groups with the night map in the collector's slot (binding 10) - `set_night_collector` swaps
    /// them with `globals_bg` / `globals_bg_alt` / `mirror_bg`, as the original binds FastCollectorNight to s13 at night.
    pub globals_bg_night: wgpu::BindGroup,
    pub globals_bg_alt_night: wgpu::BindGroup,
    pub mirror_bg_night: wgpu::BindGroup,
    pub night_fc: bool,
    /// The wet road's planar reflection (refl.wgsl, `Gpu::reflection_pass`): the picture the
    /// road programs read (`t_refl` of the globals), its depth buffer, the pass's own globals
    /// bind group (the main globals without the picture itself), its uniform (the plane's
    /// height) with the sky dome's constants, and the two pipelines.
    pub refl_tex: wgpu::Texture,
    pub refl_view: wgpu::TextureView,
    /// The mini sky (`skyhat::SkyHat::mini_pass`, sky_mini.wgsl): the 128 x 128 picture of the
    /// sky the window panes mirror (`t_minisky` of the globals), bytes in the console's gamma.
    pub minisky_tex: wgpu::Texture,
    pub minisky_view: wgpu::TextureView,
    pub refl_depth: wgpu::TextureView,
    pub refl_globals_bg: wgpu::BindGroup,
    pub refl_buf: wgpu::Buffer,
    pub refl_bg: wgpu::BindGroup,
    pub refl_layout: wgpu::BindGroupLayout,
    pub pipeline_refl: wgpu::RenderPipeline,
    pub pipeline_refl_win: wgpu::RenderPipeline,
    pub pipeline_refl_sky: wgpu::RenderPipeline,
    /// The rear-view mirror (rearview.rs, OPTIONS > DISPLAY, DECISIONS.md D20): its own globals (the main
    /// view's with the mirror's camera), their bind group (the main view's textures), and its sky, which
    /// also clears the strip's depth.
    pub mirror_globals: wgpu::Buffer,
    pub mirror_bg: wgpu::BindGroup,
    /// The garage floor's mirrored pass (floormirror.rs): its globals (the main view's, the view mirrored about the floor)
    /// and their bind group - the collector's map in the picture's slot, which cannot be read while it is drawn into.
    pub floor_globals: wgpu::Buffer,
    pub floor_bg: wgpu::BindGroup,
    pub pipeline_mirror_sky: ms::MsPipe,
    pub sampler: wgpu::Sampler,
    pub white: Arc<GpuTexture>,
    pub textures: HashMap<u32, Arc<GpuTexture>>,
    /// (with the textures in it: while a bind group is in use by anything, its textures stay)
    pub bind_cache: HashMap<(u32, u32, u32), (Arc<wgpu::BindGroup>, [Arc<GpuTexture>; 4])>,
    /// The city's textures in `textures`, which `World` can upload again from the same file when it needs them:
    /// bytes, and the count of `trim_textures` since which nothing has used them (0: in use).
    pub tex_idle: HashMap<u32, (u64, u64)>,
    pub tex_trims: u64,
    /// 1x1 transparent texture (no grime layer)
    pub clear_tex: Arc<GpuTexture>,
    pub bc_supported: bool,
    pub max_tex: u32,
}

impl Gpu {
    pub async fn new(window: Arc<winit::window::Window>) -> Result<Gpu> {
        let gpu = Gpu::build(Gpu::base(window).await?);
        vram_ballast(&gpu.device, &gpu.queue);
        Ok(gpu)
    }

    /// The device, queue and configured surface: what the start-up movie needs to be drawn while the rest of
    /// the renderer (`build`: its programs, a long wait for the driver) is made on the loading thread
    /// (docs/reference/ui/boot.md, "Loading behind the movie").
    pub async fn base(window: Arc<winit::window::Window>) -> Result<GpuBase> {
        // (the game's own sky / road / sea programs: translated from the disc at install time, read once here - xs.rs)
        xs::load(&crate::hooks::get().map(|r| r.data.path.clone()).unwrap_or_else(|| std::path::PathBuf::from(".")));
        // Vulkan when the driver has it, else DX12; settings.toml display.backend = "vulkan" / "dx12" takes that one (issue
        // #12: to compare the two on a card where the game is slow); WGPU_BACKEND=dx12 etc.: a test hook over both.
        // On DX12 the programs go through DXC, linked into the exe: FXC (wgpu's default) needed its optimiser off (/Od)
        // not to recurse without end on city.wgsl's fs_main (docs/LIGHTING.md, 2026-10-24), and even so took 86 s and
        // more a city pipeline, the start-up movie's LOADING waiting minutes for them (GitHub #33). With
        // WGPU_DX12_COMPILER=fxc (a test hook) the instance is still made again with InstanceFlags::DEBUG for that /Od.
        let mut backends = wgpu::Backends::from_env().unwrap_or(match crate::hooks::display().backend {
            mcla_config::Backend::Auto => wgpu::Backends::PRIMARY,
            mcla_config::Backend::Vulkan => wgpu::Backends::VULKAN,
            mcla_config::Backend::Dx12 => wgpu::Backends::DX12,
        });
        dxil_cache_dir();
        // (a swap chain that cannot be made: on DX12 in exclusive fullscreen the window goes borderless and it is tried
        // again; what still fails starts over on Vulkan - GitHub #12, `configure_checked`)
        loop {
            let mut dx12_od = false;
            let (instance, surface, adapter) = loop {
                let mut flags = wgpu::InstanceFlags::from_build_config().with_env();
                if dx12_od {
                    flags |= wgpu::InstanceFlags::DEBUG;
                }
                let shader_compiler = wgpu::Dx12Compiler::StaticDxc.with_env();
                let fxc = matches!(shader_compiler, wgpu::Dx12Compiler::Fxc);
                let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                    backends: if dx12_od { wgpu::Backends::DX12 } else { backends },
                    flags,
                    backend_options: wgpu::BackendOptions { dx12: wgpu::Dx12BackendOptions { shader_compiler }, ..Default::default() },
                });
                let surface = instance.create_surface(window.clone()).context("create surface")?;
                // (test hook MCLA_ADAPTER=<part of a name>: that GPU instead of the high-performance one - a second card or
                // the CPU's own, for driver-specific reports such as GitHub #14; none matching: the usual choice)
                let named = std::env::var("MCLA_ADAPTER").ok().filter(|v| !v.trim().is_empty()).and_then(|want| {
                    let want = want.trim().to_ascii_lowercase();
                    let all = instance.enumerate_adapters(if dx12_od { wgpu::Backends::DX12 } else { backends });
                    for a in &all {
                        let i = a.get_info();
                        log::info!("adapter: {} ({:?}, {:?}, driver {})", i.name, i.backend, i.device_type, i.driver_info);
                    }
                    let a = all.into_iter().find(|a| a.get_info().name.to_ascii_lowercase().contains(&want) && a.is_surface_supported(&surface));
                    if a.is_none() {
                        log::warn!("MCLA_ADAPTER: no adapter named like {want:?} can draw to the window; the usual one instead");
                    }
                    a
                });
                let adapter = match named {
                    Some(a) => a,
                    None => instance
                        .request_adapter(&wgpu::RequestAdapterOptions {
                            power_preference: wgpu::PowerPreference::HighPerformance,
                            compatible_surface: Some(&surface),
                            force_fallback_adapter: false,
                        })
                        .await
                        .context("no suitable GPU adapter")?,
                };
                if fxc && adapter.get_info().backend == wgpu::Backend::Dx12 && !dx12_od && !flags.contains(wgpu::InstanceFlags::DEBUG) {
                    dx12_od = true;
                    continue;
                }
                break (instance, surface, adapter);
            };
            let info = adapter.get_info();
            log::info!("GPU: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
            crate::hooks::set_adapter(&info);
            let feats = adapter.features();
            let bc_supported = feats.contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
            let mut required = wgpu::Features::empty();
            if bc_supported {
                required |= wgpu::Features::TEXTURE_COMPRESSION_BC;
            }
            // (2-sample targets, the original's "fullscreenself" - selfmovie.rs; WebGPU guarantees only 1 and 4)
            if feats.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
                required |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
            }
            // (the scene's depth-stencil: the original's ids in the stencil - render/ms.rs)
            if feats.contains(wgpu::Features::DEPTH32FLOAT_STENCIL8) {
                required |= wgpu::Features::DEPTH32FLOAT_STENCIL8;
                ms::set_scene_ds_format(wgpu::TextureFormat::Depth32FloatStencil8);
            } else {
                ms::set_scene_ds_format(wgpu::TextureFormat::Depth24PlusStencil8);
            }
            // (the timestamps of the `gputimes:` log line, where the adapter has them - gpuprof.rs)
            required |= crate::gpuprof::features(feats);
            // (the game's own Vulkan pipeline cache - pcache.rs)
            required |= pcache::feature(&info, feats);
            let limits = adapter.limits();
            let (device, queue) = adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        label: Some("mcla"),
                        required_features: required,
                        required_limits: wgpu::Limits {
                            max_texture_dimension_2d: limits.max_texture_dimension_2d.max(4096),
                            max_buffer_size: limits.max_buffer_size,
                            ..wgpu::Limits::default()
                        },
                        memory_hints: memory_hints(),
                    },
                    None,
                )
                .await
                .context("request device")?;
            pcache::init(&device, &info);
            let size = window.inner_size();
            let mut config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .context("surface config")?;
            // (settings.toml display.vsync - `hooks::present_mode`)
            config.present_mode = crate::hooks::present_mode();
            config.format = if config.format.is_srgb() { config.format } else { config.format.add_srgb_suffix() };
            // (the UI pass writes through the plain view of the same surface, so that its alpha
            // blending happens on the encoded values as the original's does - see main.rs "ui")
            config.view_formats = vec![config.format, config.format.remove_srgb_suffix()];
            config.usage |= wgpu::TextureUsages::COPY_SRC;
            let mut surface = surface;
            // (test hook MCLA_LOSE_DEVICE=1: the device lost here in exclusive fullscreen, as in GitHub #33)
            if std::env::var_os("MCLA_LOSE_DEVICE").is_some() && matches!(window.fullscreen(), Some(winit::window::Fullscreen::Exclusive(_))) {
                device.destroy();
            }
            let mut ok = configure_checked(&device, &surface, &config).await;
            if !ok && info.backend == wgpu::Backend::Dx12 && matches!(window.fullscreen(), Some(winit::window::Fullscreen::Exclusive(_))) {
                log::warn!("surface: no DX12 swap chain on the exclusive-fullscreen window; borderless fullscreen instead");
                window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(window.current_monitor())));
                // (leaving the exclusive mode gives the monitor its own mode back: the window's size is the screen's now)
                let size = window.inner_size();
                surface = instance.create_surface(window.clone()).context("create surface")?;
                config.width = size.width.max(1);
                config.height = size.height.max(1);
                ok = configure_checked(&device, &surface, &config).await;
            }
            // (GitHub #33: an RX 5500 XT loses the Vulkan device here in exclusive fullscreen; a lost device cannot be used
            // again, so borderless fullscreen starts over with a new one)
            if !ok && matches!(window.fullscreen(), Some(winit::window::Fullscreen::Exclusive(_))) {
                log::warn!("surface: no swap chain on the exclusive-fullscreen window on {:?}; borderless fullscreen instead, on a new device", info.backend);
                window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(window.current_monitor())));
                continue;
            }
            if !ok {
                if info.backend != wgpu::Backend::Vulkan && backends != wgpu::Backends::VULKAN {
                    log::warn!("surface: no swap chain on {:?}; starting over on Vulkan", info.backend);
                    backends = wgpu::Backends::VULKAN;
                    continue;
                }
                anyhow::bail!("surface configuration failed on {:?}", info.backend);
            }
            log::info!("surface: {}x{} {:?} {:?}", config.width, config.height, config.format, config.present_mode);
            crate::selfmovie::set_two_samples(
                required.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
                    && adapter.get_texture_format_features(config.format.remove_srgb_suffix()).flags.sample_count_supported(2),
            );
            return Ok(GpuBase { device, queue, surface: Arc::new(surface), config, bc_supported, max_tex: limits.max_texture_dimension_2d });
        }
    }

    /// The rest of the renderer on a [`GpuBase`] (any thread).
    pub fn build(base: GpuBase) -> Gpu {
        let GpuBase { device, queue, surface, config, bc_supported, max_tex } = base;
        let depth = create_depth(&device, config.width, config.height);
        let post = post::Post::new(&device, config.format, config.width, config.height);

        let globals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&Globals::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        // sun shadow map
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_tex.create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow cmp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        // The materials' sampler. It is bound once in group 0 (globals binding 16, the shadow layout's binding 1, the sky
        // hat's binding 1), not in every material's texture group: wgpu 24 on DX12 copies a bind group's samplers into a
        // 2048-entry heap for every bind group, and the city's materials ran it out (docs/LIGHTING.md, DX12 fallback).
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("aniso"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            // (diagnostics, p2day 2026-10-13: MCLA_ANISO=<1..16>; the original's city fetches ask for 4 : 1)
            anisotropy_clamp: std::env::var("MCLA_ANISO").ok().and_then(|v| v.parse::<u16>().ok()).map_or(4, |v| v.clamp(1, 16)),
            ..Default::default()
        });
        let shadow_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow globals"),
            contents: bytemuck::bytes_of(&CascPass { light_vp: Mat4::IDENTITY.to_cols_array_2d(), bias: [0.0; 4] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow globals layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let shadow_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow bg"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
        });
        // the original's four cascades: their depth map, a uniform and bind group per tile's
        // pass, and what the scene reads them with
        let casc_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow cascades"),
            size: wgpu::Extent3d { width: casc_tile() * 2, height: casc_tile() * 2, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let casc_view = casc_tex.create_view(&Default::default());
        let casc_pass_bufs: Vec<wgpu::Buffer> = (0..4)
            .map(|_| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("cascade pass"), contents: bytemuck::bytes_of(&CascPass::zeroed()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST }))
            .collect();
        let casc_pass_bgs: Vec<wgpu::BindGroup> = casc_pass_bufs
            .iter()
            .map(|b| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("cascade pass bg"), layout: &shadow_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }] }))
            .collect();
        let casc_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("cascades"), contents: bytemuck::bytes_of(&CascUniform::zeroed()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let lights_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lights"),
            contents: bytemuck::bytes_of(&LightSet::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // the surroundings as a cube map, for what mirrors them (paint, chrome, glass)
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::Cube, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // the surroundings again, blurred (the paint's soft specular)
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::Cube, multisampled: false },
                    count: None,
                },
                // the cars' scrape texture (the disc's dmg_scrape.dds, written once it is found)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // the night lamp grid (four lamp numbers per cell) and the lamps' data
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // the fast shadow collector's map
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // the wet road's planar reflection picture
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // the sun shadow cascades: their constants and their depth map
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // the mini sky
                wgpu::BindGroupLayoutEntry {
                    binding: 14,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // the city's texture sampler: the device's default sampler state, maximum anisotropy by mode (ms.rs)
                wgpu::BindGroupLayoutEntry { binding: 15, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                // the materials' sampler (was binding 1 of every texture group)
                wgpu::BindGroupLayoutEntry { binding: 16, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                // a car effect's own anisotropy, 2:1 / 4:1 / 16:1 (render/aniso.rs; here and not in each texture group: DX12's
                // sampler heap holds 2,048 entries and every bind group with a sampler takes its own, GitHub #48)
                wgpu::BindGroupLayoutEntry { binding: 17, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 18, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 19, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let minisky_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mini sky"),
            size: wgpu::Extent3d { width: MINISKY_SIZE, height: MINISKY_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let minisky_view = minisky_tex.create_view(&Default::default());
        let refl_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("road reflection"),
            size: wgpu::Extent3d { width: REFL_SIZE, height: REFL_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: post::SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let refl_view = refl_tex.create_view(&Default::default());
        let refl_depth = create_depth(&device, REFL_SIZE, REFL_SIZE);
        let fc_map = |label: &str, usage: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: fc_size(), height: fc_size(), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | usage,
                view_formats: &[],
            })
        };
        let fc_work = fc_map("fast collector, working", wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC);
        let fc_tex = fc_map("fast collector", wgpu::TextureUsages::COPY_SRC);
        let fc_prev = fc_map("fast collector, last frame", wgpu::TextureUsages::COPY_DST);
        let fc_prev_view = fc_prev.create_view(&Default::default());
        let (fc_work_view, fc_view) = (fc_work.create_view(&Default::default()), fc_tex.create_view(&Default::default()));
        let fc_depth = create_depth(&device, fc_size(), fc_size());
        // the night fast collector's two maps (the casters', the blurred one the scene reads), white to start with
        let nfc_map = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: NFC_SIZE, height: NFC_SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let nfc_tex = nfc_map("night fast collector");
        let (nfc_work_view, nfc_view) = (nfc_map("night fast collector, casters").create_view(&Default::default()), nfc_tex.create_view(&Default::default()));
        {
            let mut enc = device.create_command_encoder(&Default::default());
            for view in [&nfc_work_view, &nfc_view] {
                enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("night fast collector, start"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            queue.submit([enc.finish()]);
        }
        {
            // the maps start lit (the game's too: its map's corners, where the goal is always 255, read 255 and
            // not the 251 a rise from below ends at). MCLA_FC_START=<0..255>: another start level (diagnostics).
            let fc_start = std::env::var("MCLA_FC_START").ok().and_then(|v| v.split(',').next().and_then(|x| x.trim().parse::<f64>().ok())).map_or(1.0, |v| (v / 255.0).clamp(0.0, 1.0));
            let mut enc = device.create_command_encoder(&Default::default());
            for view in [&fc_work_view, &fc_view, &fc_prev_view] {
                enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fast collector, start"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: fc_start, g: fc_start, b: 1.0, a: 1.0 }), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            queue.submit([enc.finish()]);
        }
        let lamp_tex = |w: u32, h: u32, format: wgpu::TextureFormat, label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let lamp_grid = lamp_tex(LAMP_GRID.0, LAMP_GRID.1, wgpu::TextureFormat::Rgba16Uint, "lamp grid");
        let lamp_data = lamp_tex(LAMP_DATA.0, LAMP_DATA.1, wgpu::TextureFormat::Rgba32Float, "lamp data");
        let (lamp_grid_view, lamp_data_view) = (lamp_grid.create_view(&Default::default()), lamp_data.create_view(&Default::default()));
        let scrape_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scrape"),
            size: wgpu::Extent3d { width: 512, height: 512, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let scrape_view = scrape_tex.create_view(&Default::default());
        let garage_glow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("garage glow"),
            size: wgpu::Extent3d { width: 256, height: 128, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let garage_glow_view = garage_glow_tex.create_view(&Default::default());
        // The environment map: the world seen from the player's car in six directions, one
        // face redrawn each frame (the original keeps a dual-paraboloid map of the same kind
        // for its car shaders - dpFrontSampler). The passes that draw it see a black cube in
        // its place.
        let cube_tex = |size: u32, label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 6 },
                mip_level_count: if size == ENV_SIZE && label == "environment" { ENV_MIPS } else { 1 },
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: post::SCENE_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let cube_view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::Cube), ..Default::default() });
        let env_tex = cube_tex(ENV_SIZE, "environment");
        let env_none = cube_tex(1, "no environment");
        let (env_cube, env_none_cube) = (cube_view(&env_tex), cube_view(&env_none));
        let env_soft_tex = cube_tex(ENV_SOFT_SIZE, "environment, blurred");
        let env_soft_cube = cube_view(&env_soft_tex);
        let env_soft_faces: Vec<wgpu::TextureView> = (0..6).map(|i| env_soft_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: i, array_layer_count: Some(1), ..Default::default() })).collect();
        let env_face_level = |level: u32, i: u32| env_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_mip_level: level, mip_level_count: Some(1), base_array_layer: i, array_layer_count: Some(1), ..Default::default() });
        let env_faces: Vec<wgpu::TextureView> = (0..6).map(|i| env_face_level(0, i)).collect();
        let env_depth = create_depth(&device, ENV_SIZE, ENV_SIZE);
        // the two paraboloid halves the city is drawn into for the cars' map (256 x 256 each, as
        // the halves of the original's 512 x 256), and their depth
        let env_dp_views: Vec<wgpu::TextureView> = (0..2)
            .map(|_| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("environment, paraboloid half"),
                        size: wgpu::Extent3d { width: ENV_DP_SIZE, height: ENV_DP_SIZE, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: post::SCENE_FORMAT,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            })
            .collect();
        let env_dp_depth = create_depth(&device, ENV_DP_SIZE, ENV_DP_SIZE);
        // (mip filter linear: the badge's fetch of the map, cars/0x1d68d869 dpFrontSampler, word 3 0x00a80d11 - mag, min and
        // mip filter linear, levels 0 .. 3; every other read of the cube names its level)
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("environment"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::FilterMode::Linear, ..Default::default() });
        // (F-23692 each frame: grc sampler state 9, maximum anisotropy, = 16 while the 4x flag is set, else
        // `*(0x827E19C4)` = 4; the city's fetches take it from the fetch constant - driving trace aniso field 3 = 4 : 1,
        // GPS map 5 = 16 : 1. MCLA_ANISO=<n> forces one value.)
        let aniso_env = std::env::var("MCLA_ANISO").ok().and_then(|v| v.parse::<u16>().ok()).map(|v| v.clamp(1, 16));
        let city_sampler = |aniso: u16| device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("city aniso"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: aniso_env.unwrap_or(aniso),
            ..Default::default()
        });
        let city_samp4 = city_sampler(4);
        let city_samp16 = city_sampler(16);
        // (a draw's own sampler, the material sampler's state 9 that F-15464 lays over the default table: render/aniso.rs.
        // Index 0, no own value, is never read: a code with a mask always has a value)
        let own_samplers = aniso::OWN_MAX.map(|a| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("own aniso"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                anisotropy_clamp: a,
                ..Default::default()
            })
        });
        let make_globals_bg_sf = |buf: &wgpu::Buffer, cube: &wgpu::TextureView, soft: &wgpu::TextureView, refl: &wgpu::TextureView, samp: &wgpu::Sampler, fc: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("globals bg"),
                layout: &globals_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: lights_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&shadow_view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&scrape_view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(cube) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&env_sampler) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&lamp_grid_view) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&lamp_data_view) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(soft) },
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(fc) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(refl) },
                    wgpu::BindGroupEntry { binding: 12, resource: casc_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(&casc_view) },
                    wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(&minisky_view) },
                    wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::Sampler(samp) },
                    wgpu::BindGroupEntry { binding: 16, resource: wgpu::BindingResource::Sampler(&sampler) },
                    wgpu::BindGroupEntry { binding: 17, resource: wgpu::BindingResource::Sampler(&own_samplers[1]) },
                    wgpu::BindGroupEntry { binding: 18, resource: wgpu::BindingResource::Sampler(&own_samplers[2]) },
                    wgpu::BindGroupEntry { binding: 19, resource: wgpu::BindingResource::Sampler(&own_samplers[3]) },
                ],
            })
        };
        let make_globals_bg_s = |buf: &wgpu::Buffer, cube: &wgpu::TextureView, soft: &wgpu::TextureView, refl: &wgpu::TextureView, samp: &wgpu::Sampler| make_globals_bg_sf(buf, cube, soft, refl, samp, &fc_view);
        let make_globals_bg = |buf: &wgpu::Buffer, cube: &wgpu::TextureView, soft: &wgpu::TextureView, refl: &wgpu::TextureView| make_globals_bg_s(buf, cube, soft, refl, &city_samp4);
        let globals_bg = make_globals_bg(&globals_buf, &env_cube, &env_soft_cube, &refl_view);
        let globals_bg_alt = make_globals_bg_s(&globals_buf, &env_cube, &env_soft_cube, &refl_view, &city_samp16);
        // (the reflection pass draws with the main globals; the picture itself cannot be bound
        // while it is drawn into - the collector's map stands in its slot)
        let refl_globals_bg = make_globals_bg(&globals_buf, &env_none_cube, &env_none_cube, &fc_view);
        let env_blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("environment blur"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::Cube, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let env_blur_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("environment blur"),
            layout: &env_blur_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&env_cube) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&env_sampler) },
            ],
        });
        let env_blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("environment blur"),
            // (MCLA_SOFT_OLD=1: the blur as it was until 2026-10-14 - a 64-tap disc, env_blur_old.wgsl - for A/B)
            source: wgpu::ShaderSource::Wgsl(if std::env::var("MCLA_SOFT_OLD").is_ok() { include_str!("env_blur_old.wgsl") } else { include_str!("env_blur.wgsl") }.into()),
        });
        let env_blur_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("environment blur"), bind_group_layouts: &[&env_blur_layout], push_constant_ranges: &[] });
        let pipeline_env_blur = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("environment blur"),
            layout: Some(&env_blur_pl),
            vertex: wgpu::VertexState { module: &env_blur_shader, entry_point: Some("vs_blur"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &env_blur_shader,
                entry_point: Some("fs_blur"),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        // the cube's levels 1 .. 3, each a 4 x 4 box of the level above (env_mip.wgsl)
        let env_mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("environment mips"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let mut env_mip_views = Vec::new();
        let mut env_mip_bgs = Vec::new();
        for level in 1..ENV_MIPS {
            for face in 0..6 {
                let src = env_face_level(level - 1, face);
                env_mip_bgs.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("environment mips"),
                    layout: &env_mip_layout,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&env_sampler) }],
                }));
                env_mip_views.push(env_face_level(level, face));
            }
        }
        let env_mip_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("environment mips"), source: wgpu::ShaderSource::Wgsl(include_str!("env_mip.wgsl").into()) });
        let env_mip_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("environment mips"), bind_group_layouts: &[&env_mip_layout], push_constant_ranges: &[] });
        let pipeline_env_mip = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("environment mips"),
            layout: Some(&env_mip_pl),
            vertex: wgpu::VertexState { module: &env_mip_shader, entry_point: Some("vs_mip"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &env_mip_shader,
                entry_point: Some("fs_mip"),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        // the cube filled from the two paraboloid halves (env_fill.wgsl), drawn inside a face's pass
        let env_fill_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("environment fill"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            ],
        });
        let env_fill_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("environment fill"),
            layout: &env_fill_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&env_dp_views[0]) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&env_dp_views[1]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&env_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&garage_glow_view) },
            ],
        });
        let env_fill_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("environment fill"), source: wgpu::ShaderSource::Wgsl(include_str!("env_fill.wgsl").into()) });
        let env_fill_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("environment fill"), bind_group_layouts: &[&env_fill_layout], push_constant_ranges: &[] });
        let pipeline_env_fill = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("environment fill"),
            layout: Some(&env_fill_pl),
            vertex: wgpu::VertexState { module: &env_fill_shader, entry_point: Some("vs_fill"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &env_fill_shader,
                entry_point: Some("fs_fill"),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: false, depth_compare: wgpu::CompareFunction::Always, stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        // (six for the cube's faces, two for the paraboloid halves)
        let env_globals: Vec<wgpu::Buffer> = (0..8)
            .map(|_| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("environment globals"), contents: bytemuck::bytes_of(&Globals::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST }))
            .collect();
        let env_bgs: Vec<wgpu::BindGroup> = env_globals.iter().map(|b| make_globals_bg(b, &env_none_cube, &env_none_cube, &fc_view)).collect();
        // (the rear-view mirror's: the original's mirror pass draws the main view's programs with the main
        // view's textures bound - rearview.rs)
        let mirror_globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("rear-view mirror globals"), contents: bytemuck::bytes_of(&Globals::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let mirror_bg = make_globals_bg(&mirror_globals, &env_cube, &env_soft_cube, &refl_view);
        let floor_globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("garage floor mirror globals"), contents: bytemuck::bytes_of(&Globals::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let floor_bg = make_globals_bg(&floor_globals, &env_cube, &env_soft_cube, &fc_view);
        // (the same three with the night map where the collector is)
        let globals_bg_night = make_globals_bg_sf(&globals_buf, &env_cube, &env_soft_cube, &refl_view, &city_samp4, &nfc_view);
        let globals_bg_alt_night = make_globals_bg_sf(&globals_buf, &env_cube, &env_soft_cube, &refl_view, &city_samp16, &nfc_view);
        let mirror_bg_night = make_globals_bg_sf(&mirror_globals, &env_cube, &env_soft_cube, &refl_view, &city_samp4, &nfc_view);
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // (the texture sampler is in group 0: globals binding 15 / 16, shadow binding 1, sky hat binding 1; so are a car effect's
                // own samplers, globals 17 - 19. Binding 5 here says which fetches take which: render/aniso.rs. No sampler in this group:
                // DX12 gives every bind group holding one its own slots of a 2,048-entry heap, GitHub #48)
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
            ],
        });
        // (a draw's fetch masks by its code's low six bits: each fetch's own sampler index, 0 = the texture sampler)
        let own_masks: Vec<wgpu::Buffer> = (0u32..64)
            .map(|c| {
                let w = aniso::mask_words(c);
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("own aniso mask"), contents: bytemuck::cast_slice(&w), usage: wgpu::BufferUsages::UNIFORM })
            })
            .collect();
        let mat_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mat layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // (the vertex stage too: the surf strip's vertex program runs in its own draw only - `SHORE_PROGRAM`)
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let mut mat_bgs = HashMap::new();
        for mode in 0u32..MatMode::COUNT {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mat"),
                contents: bytemuck::bytes_of(&MaterialUniform { mode, _pad: [0; 3] }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mat bg"),
                layout: &mat_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
            });
            mat_bgs.insert(mode, bg);
        }
        // (the sea where the stencil holds 1 - the sky's or the ground dome's pixels: the original's fog pass leaves them
        // alone; `MaterialUniform::_pad[0]` = 1 tells city.wgsl to lay no fog; keyed mode | SEA_NOFOG - `pipeline_sea`)
        for mode in [MatMode::OceanWater as u32, MatMode::OceanWaterLod as u32] {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mat sea unfogged"),
                contents: bytemuck::bytes_of(&MaterialUniform { mode, _pad: [1, 0, 0] }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mat bg sea unfogged"),
                layout: &mat_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
            });
            mat_bgs.insert(mode | SEA_NOFOG, bg);
        }
        {
            let mode = MatMode::Blend as u32;
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mat shore"),
                contents: bytemuck::bytes_of(&MaterialUniform { mode, _pad: [0, 1, 0] }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mat bg shore"),
                layout: &mat_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
            });
            mat_bgs.insert(mode | SHORE_PROGRAM, bg);
        }
        // (binding 1: a car's vertex damage map, crate::damagemap - DmgTex; everything else has the
        // empty one, as the original's blank_dmg.dds: all zero)
        let obj_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("obj layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                // (binding 2: the car block, `CarUniform` - D59, the rows equal for every object of one car)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let blank_dmg = damage_map_texture(&device, 1).create_view(&Default::default());
        let identity_obj = make_object_binding(&device, &obj_layout, &ObjectUniform::default(), &blank_dmg);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("city"),
            source: wgpu::ShaderSource::Wgsl(mode_constant(&xs::split_consts(&trim_tuned(&paint_tuned(&[include_str!("city.wgsl"), xs::text("xs_road.wgsl"), xs::text("xs_roadn.wgsl"), xs::text("xs_water.wgsl"), xs::text("xs_pond.wgsl"), xs::text("xs_shore.wgsl")].concat())))).replace("const HALF_PX_DP: f32 = 1.0;", if half_pixel_on() { "const HALF_PX_DP: f32 = 1.0;" } else { "const HALF_PX_DP: f32 = 0.0;" }).into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("city pl"),
            bind_group_layouts: &[&globals_layout, &tex_layout, &mat_layout, &obj_layout],
            push_constant_ranges: &[],
        });
        // (every city pipeline per ms::Variant: the scene's are multisampled and write the stencil reference)
        // (`sea`: a stencil test against reference 1 that writes nothing, in place of the usual REPLACE - `pipeline_sea`)
        let city_build_x = {
            let (pl, shader) = (pl.clone(), shader.clone());
            Arc::new(move |device: &wgpu::Device, v: ms::Variant, blend: bool, vs: &'static str, cull: Option<wgpu::Face>, zwrite: bool, writes: wgpu::ColorWrites, mode: Option<u32>, sea: Option<wgpu::CompareFunction>| {
            let constants: std::collections::HashMap<String, f64> = mode.map(|m| ("MODE_K".to_string(), m as f64)).into_iter().collect();
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if blend { "city blend" } else { "city opaque" }),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[Vertex::LAYOUT],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: post::SCENE_FORMAT,
                        blend: Some(if blend { wgpu::BlendState::ALPHA_BLENDING } else { wgpu::BlendState::REPLACE }),
                        write_mask: writes,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: cull,
                    front_face: wgpu::FrontFace::Cw,
                    ..Default::default()
                },
                depth_stencil: Some({
                    let mut ds = v.ds(zwrite, wgpu::CompareFunction::GreaterEqual, sea.is_none(), Default::default());
                    if let Some(compare) = sea {
                        let f = wgpu::StencilFaceState { compare, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op: wgpu::StencilOperation::Keep };
                        ds.stencil = wgpu::StencilState { front: f, back: f, read_mask: 0xff, write_mask: 0 };
                    }
                    ds
                }),
                multisample: v.ms(),
                multiview: None,
                cache: crate::render::pcache::get(),
            })
            })
        };
        let city_build = {
            let x = city_build_x.clone();
            Arc::new(move |device: &wgpu::Device, v: ms::Variant, blend: bool, vs: &'static str, cull: Option<wgpu::Face>, zwrite: bool, writes: wgpu::ColorWrites, mode: Option<u32>| x(device, v, blend, vs, cull, zwrite, writes, mode, None))
        };
        let make_pipeline_x = |blend: bool, vs: &'static str, cull: Option<wgpu::Face>, zwrite: bool| {
            let b = city_build.clone();
            ms::MsPipe::new_shared(&device, move |d, v| b(d, v, blend, vs, cull, zwrite, wgpu::ColorWrites::ALL, None))
        };
        let make_pipeline = |blend: bool, vs: &'static str| make_pipeline_x(blend, vs, None, !blend);
        // (the cars' windows as the original draws them: blended WITH depth writes, back faces first
        // (register 0x2205 = 0x18006), then front faces (0x18002) - LIGHTING_WORK.md, car round 11)
        // (which winding is the outer face was settled by eye: the R8's rear pane from behind; MCLA_GLASS_FLIP swaps)
        let glass_flip = std::env::var("MCLA_GLASS_FLIP").is_err();
        let pipeline_glass_back = make_pipeline_x(true, "vs_main", Some(if glass_flip { wgpu::Face::Back } else { wgpu::Face::Front }), true);
        let pipeline_glass_front = make_pipeline_x(true, "vs_main", Some(if glass_flip { wgpu::Face::Front } else { wgpu::Face::Back }), true);
        // (the in-car glass: register 0x2205 = 0x19806 as the back faces' pass, depth control 0x00708762 - tested,
        // not written; recorded in the driver's view, work/intglass cap/pov_noon)
        let pipeline_glass_inner = make_pipeline_x(true, "vs_main", Some(if glass_flip { wgpu::Face::Back } else { wgpu::Face::Front }), false);
        // (the shop panes: the original draws them blended with depth writes (depth control 0x00708767 on
        // every pane draw, back faces culled), in batch order - a pane behind one drawn earlier is not drawn,
        // so the shop behind an angled entrance shows through ONE pane; back faces go in city.wgsl)
        let pipeline_pane = make_pipeline_x(true, "vs_main", None, true);
        let pipeline_opaque = make_pipeline(false, "vs_main");
        // (the opaque phase of the city's sectors with a pipeline per material: `mode_constant`; drawn only in the scene pass,
        // built behind the loading - main.rs `ms::build_behind`; until then `pipeline_opaque` draws the same)
        let pipeline_opaque_mode: Vec<Option<ms::MsPipe>> = (0..MatMode::COUNT)
            .map(|m| {
                (mode_pipes_on() && MODE_PIPES.iter().chain(&CAR_MODE_PIPES).any(|k| *k as u32 == m)).then(|| {
                    let b = city_build.clone();
                    ms::MsPipe::new_behind(&device, move |d, v| b(d, v, false, "vs_main", None, true, wgpu::ColorWrites::ALL, Some(m)))
                })
            })
            .collect();
        // (render state 0 = 2, the original's 0x18002 - the face the windows' front pass culls - and in a mirrored view 1)
        let pipeline_opaque_cull = make_pipeline_x(false, "vs_main", Some(if glass_flip { wgpu::Face::Front } else { wgpu::Face::Back }), true);
        let pipeline_opaque_cull_mode: Vec<Option<ms::MsPipe>> = (0..MatMode::COUNT)
            .map(|m| {
                (mode_pipes_on() && CAR_MODE_PIPES.iter().any(|k| *k as u32 == m)).then(|| {
                    let b = city_build.clone();
                    let face = if glass_flip { wgpu::Face::Front } else { wgpu::Face::Back };
                    ms::MsPipe::new_behind(&device, move |d, v| b(d, v, false, "vs_main", Some(face), true, wgpu::ColorWrites::ALL, Some(m)))
                })
            })
            .collect();
        let pipeline_opaque_cull_m = make_pipeline_x(false, "vs_main", Some(if glass_flip { wgpu::Face::Back } else { wgpu::Face::Front }), true);
        // (the sea, drawn after the ground dome: the original's sea keeps the stencil, and its fog pass
        // (PSBlendedHeightFogInFogNoPFog, depth control 0x00700561) fogs only where the stencil is not 1 - the sky's and the
        // dome's mark. The port lays the fog in each draw, so the sea is drawn twice through the stencil: [0] where it is
        // not 1, fogged, [1] where it is 1, unfogged (`SEA_NOFOG`). Both test against reference 1 and write no stencil.)
        // (the surf strip, right after the sea: blended SrcAlpha / InvSrcAlpha, depth written, the stencil replaced by the
        // pass's reference (0) - the recorded state of its draws (blend 0x07060706, depth control 0x00708767, ref 0))
        let pipeline_shore = {
            let b = city_build_x.clone();
            ms::MsPipe::new(&device, move |d, v| b(d, v, true, "vs_main", None, true, wgpu::ColorWrites::ALL, None, None))
        };
        let pipeline_sea = [wgpu::CompareFunction::NotEqual, wgpu::CompareFunction::Equal].map(|cmp| {
            let b = city_build_x.clone();
            ms::MsPipe::new(&device, move |d, v| b(d, v, false, "vs_main", None, true, wgpu::ColorWrites::ALL, None, Some(cmp)))
        });
        let pipeline_blend = make_pipeline(true, "vs_main");
        // (the wheels' blur discs, F-49519: colour writes off, depth tested and not written, no cull, the stencil
        // replaced - recorded registers 0x2104 = 0, 0x2200 = 0x00708763, 0x2205 = 0x18000)
        let pipeline_stencil_only = {
            let b = city_build.clone();
            ms::MsPipe::new_shared(&device, move |d, v| b(d, v, false, "vs_main", None, false, wgpu::ColorWrites::empty(), None))
        };
        // (the cars' environment map drawn the original's way: the paraboloid warp per vertex - city.wgsl vs_env_dp)
        // (each ~4 s to compile with the driver's cache cold: built on threads of their own while the loading goes on)
        let pipeline_env_dp = {
            let b = city_build.clone();
            ms::MsPipe::new_plain(&device, move |d, _| b(d, ms::PLAIN, false, "vs_env_dp", None, true, wgpu::ColorWrites::ALL, None))
        };
        let pipeline_env_dp_blend = {
            let b = city_build.clone();
            ms::MsPipe::new_plain(&device, move |d, _| b(d, ms::PLAIN, true, "vs_env_dp", None, false, wgpu::ColorWrites::ALL, None))
        };
        ms::build_ahead(&[&pipeline_env_dp, &pipeline_env_dp_blend], ms::PLAIN);
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shadow.wgsl").into()),
        });
        let shadow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow pl"),
            bind_group_layouts: &[&shadow_layout, &tex_layout, &mat_layout, &obj_layout],
            push_constant_ranges: &[],
        });
        let make_shadow = |inst: bool| {
            let bufs_plain = [Vertex::LAYOUT];
            let bufs_inst = [Vertex::LAYOUT, crate::props::Inst::LAYOUT];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow"),
                layout: Some(&shadow_pl),
                vertex: wgpu::VertexState {
                    module: &shadow_shader,
                    entry_point: Some(if inst { "vs_shadow_inst" } else { "vs_shadow" }),
                    buffers: if inst { &bufs_inst } else { &bufs_plain },
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState { module: &shadow_shader, entry_point: Some("fs_shadow"), targets: &[], compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.5, clamp: 0.0 },
                }),
                multisample: Default::default(),
                multiview: None,
                cache: crate::render::pcache::get(),
            })
        };
        let pipeline_shadow = make_shadow(false);
        let pipeline_shadow_inst = make_shadow(true);
        // the cascades' depth passes: the same, the depth written by the program with the
        // tile's own bias (shadow.wgsl fs_shadow_casc)
        // (casc2, 2026-10-15: the original's depth draws cull the faces turned from the light - per mesh, see
        // shadow.wgsl fs_shadow_casc_cull. MCLA_CASC_CULL = which kinds of casters do so here: a comma list of
        // sec (the city's sectors), car, ped, prop; all; none.)
        let casc_cull_kinds = match std::env::var("MCLA_CASC_CULL") {
            Ok(v) => v.split(',').map(|k| match k.trim() {
                "sec" => CASC_CULL_SEC,
                "car" => CASC_CULL_CAR,
                "ped" => CASC_CULL_PED,
                "prop" => CASC_CULL_PROP,
                "all" => CASC_CULL_SEC | CASC_CULL_CAR | CASC_CULL_PED | CASC_CULL_PROP,
                _ => 0,
            }).fold(0, |a, b| a | b),
            Err(_) => CASC_CULL_DEFAULT,
        };
        let make_casc = |inst: bool, cull: bool| {
            let bufs_plain = [Vertex::LAYOUT];
            let bufs_inst = [Vertex::LAYOUT, crate::props::Inst::LAYOUT];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow cascade"),
                layout: Some(&shadow_pl),
                vertex: wgpu::VertexState {
                    module: &shadow_shader,
                    entry_point: Some(if inst { "vs_shadow_inst" } else { "vs_shadow" }),
                    buffers: if inst { &bufs_inst } else { &bufs_plain },
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState { module: &shadow_shader, entry_point: Some(if cull { "fs_shadow_casc_cull" } else { "fs_shadow_casc" }), targets: &[], compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, front_face: wgpu::FrontFace::Cw, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: crate::render::pcache::get(),
            })
        };
        let pipeline_casc = make_casc(false, false);
        let pipeline_casc_inst = make_casc(true, false);
        let pipeline_casc_cull = make_casc(false, true);
        let pipeline_casc_inst_cull = make_casc(true, true);
        // the fast shadow collector's two steps
        let fc_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fast collector"),
            source: wgpu::ShaderSource::Wgsl(include_str!("fast_collector.wgsl").into()),
        });
        let fc_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fast collector"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                // (the scene's cascades - their constants and depth atlas: what the original's pass tests against)
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                // (last frame's collector)
                wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            ],
        });
        let fc_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fast collector"),
            size: std::mem::size_of::<FcUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // (the draw step writes the working map and so reads the other one - which it does not use)
        let make_fc_bg = |src: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fast collector"),
                layout: &fc_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: fc_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&env_sampler) },
                    wgpu::BindGroupEntry { binding: 5, resource: casc_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&casc_view) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&fc_prev_view) },
                ],
            })
        };
        let fc_draw_bg = make_fc_bg(&fc_view);
        let fc_blur_bg = make_fc_bg(&fc_work_view);
        let fc_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fast collector"), bind_group_layouts: &[&fc_layout], push_constant_ranges: &[] });
        let fc_target = [Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let pipeline_fc = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fast collector"),
            layout: Some(&fc_pl),
            vertex: wgpu::VertexState { module: &fc_shader, entry_point: Some("vs_fc"), buffers: &[Vertex::LAYOUT], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &fc_shader, entry_point: Some("fs_fc"), targets: &fc_target, compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        let pipeline_fc_blur = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fast collector blur"),
            layout: Some(&fc_pl),
            vertex: wgpu::VertexState { module: &fc_shader, entry_point: Some("vs_blur"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &fc_shader, entry_point: Some("fs_blur"), targets: &fc_target, compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        // The night fast collector (F-55997's night branch): the casters drawn with the shadow passes' vertex programs
        // and PS_ShadowNight's constant, back faces culled as the recorded draws (0x18002: those turned to its eye
        // stay, as in the main view), no depth (the colour is one constant); then PSBlur into the map the scene reads.
        let nfc_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("night fast collector"), size: std::mem::size_of::<CascPass>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let nfc_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("night fast collector"), layout: &shadow_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: nfc_buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }] });
        let nfc_target = [Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let make_nfc = |inst: bool| {
            let bufs_plain = [Vertex::LAYOUT];
            let bufs_inst = [Vertex::LAYOUT, crate::props::Inst::LAYOUT];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("night fast collector"),
                layout: Some(&shadow_pl),
                vertex: wgpu::VertexState { module: &shadow_shader, entry_point: Some(if inst { "vs_shadow_inst" } else { "vs_shadow" }), buffers: if inst { &bufs_inst } else { &bufs_plain }, compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: &shadow_shader, entry_point: Some("fs_shadow_night"), targets: &nfc_target, compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: Some(wgpu::Face::Back), front_face: wgpu::FrontFace::Cw, ..Default::default() },
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: crate::render::pcache::get(),
            })
        };
        let pipeline_nfc = make_nfc(false);
        let pipeline_nfc_inst = make_nfc(true);
        let nfc_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("night fast collector"), source: wgpu::ShaderSource::Wgsl(include_str!("night_collector.wgsl").into()) });
        let nfc_blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("night fast collector blur"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }],
        });
        let nfc_blur_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("night fast collector blur"), layout: &nfc_blur_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&nfc_work_view) }] });
        let nfc_blur_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("night fast collector blur"), bind_group_layouts: &[&nfc_blur_layout], push_constant_ranges: &[] });
        let pipeline_nfc_blur = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("night fast collector blur"),
            layout: Some(&nfc_blur_pl),
            vertex: wgpu::VertexState { module: &nfc_shader, entry_point: Some("vs_blur"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &nfc_shader, entry_point: Some("fs_blur"), targets: &nfc_target, compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        let inst_build = {
            let (pl, shader) = (pl.clone(), shader.clone());
            std::sync::Arc::new(move |device: &wgpu::Device, v: ms::Variant, mode: Option<u32>| {
            let constants: std::collections::HashMap<String, f64> = mode.map(|m| ("MODE_K".to_string(), m as f64)).into_iter().collect();
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("city instanced"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_inst"),
                buffers: &[Vertex::LAYOUT, crate::props::Inst::LAYOUT],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, front_face: wgpu::FrontFace::Cw, ..Default::default() },
            depth_stencil: Some(v.ds(true, wgpu::CompareFunction::GreaterEqual, true, Default::default())),
            multisample: v.ms(),
            multiview: None,
            cache: crate::render::pcache::get(),
        })
            })
        };
        let pipeline_inst = {
            let b = inst_build.clone();
            ms::MsPipe::new(&device, move |d, v| b(d, v, None))
        };
        // (the props' opaque batches with a pipeline per material, as `pipeline_opaque_mode`)
        let pipeline_inst_mode: Vec<Option<ms::MsPipe>> = (0..MatMode::COUNT)
            .map(|m| {
                (mode_pipes_on() && PROP_MODE_PIPES.iter().any(|k| *k as u32 == m)).then(|| {
                    let b = inst_build.clone();
                    ms::MsPipe::new_behind(&device, move |d, v| b(d, v, Some(m)))
                })
            })
            .collect();
        let pipeline_inst_blend = {
            let (pl, shader) = (pl.clone(), shader.clone());
            ms::MsPipe::new(&device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("city instanced blend"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_inst"),
                buffers: &[Vertex::LAYOUT, crate::props::Inst::LAYOUT],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, front_face: wgpu::FrontFace::Cw, ..Default::default() },
            depth_stencil: Some(v.ds(false, wgpu::CompareFunction::GreaterEqual, true, Default::default())),
            multisample: v.ms(),
            multiview: None,
            cache: crate::render::pcache::get(),
        }))
        };
        let glow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glow.wgsl").into()),
        });
        let glow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glow pl"),
            bind_group_layouts: &[&globals_layout],
            push_constant_ranges: &[],
        });
        let pipeline_glow = {
            let (glow_pl, glow_shader) = (glow_pl.clone(), glow_shader.clone());
            ms::MsPipe::new(&device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glow"),
            layout: Some(&glow_pl),
            vertex: wgpu::VertexState { module: &glow_shader, entry_point: Some("vs_glow"), buffers: &[crate::props::Inst::LAYOUT], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &glow_shader,
                entry_point: Some("fs_glow"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: post::SCENE_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, cull_mode: None, ..Default::default() },
            depth_stencil: Some(v.ds(false, wgpu::CompareFunction::GreaterEqual, false, Default::default())),
            multisample: v.ms(),
            multiview: None,
            cache: crate::render::pcache::get(),
        }))
        };
        let env_glow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("environment glows"),
            source: wgpu::ShaderSource::Wgsl(include_str!("env_glow.wgsl").into()),
        });
        let pipeline_env_glow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("environment glows"),
            layout: Some(&glow_pl),
            vertex: wgpu::VertexState {
                module: &env_glow_shader,
                entry_point: Some("vs_env_glow"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &env_glow_shader,
                entry_point: Some("fs_env_glow"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: post::SCENE_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Max },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Max },
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, cull_mode: None, ..Default::default() },
            // (the quads stand at unit distance from the eye: in front of everything)
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        let env_glow_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("environment glows"),
            size: (ENV_GLOWS * 32) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let particle_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle"),
            source: wgpu::ShaderSource::Wgsl(include_str!("particle.wgsl").into()),
        });
        // (the scene's depth, read by the checkpoint smoke's soft fade - PSCheckpointProj's
        // DepthMapTexSampler; the particles' pass holds the depth read-only)
        let particle_depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle depth"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let particle_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle pl"),
            bind_group_layouts: &[&globals_layout, &tex_layout, &particle_depth_layout],
            push_constant_ranges: &[],
        });
        let make_particle = |additive: bool| {
            let (particle_pl, particle_shader) = (particle_pl.clone(), particle_shader.clone());
            let blend = if additive {
                wgpu::BlendState {
                    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                    alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                }
            } else {
                wgpu::BlendState::ALPHA_BLENDING
            };
            ms::MsPipe::new(&device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("particle"),
                layout: Some(&particle_pl),
                vertex: wgpu::VertexState { module: &particle_shader, entry_point: Some("vs_particle"), buffers: &[crate::particles::PInst::LAYOUT], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &particle_shader,
                    entry_point: Some(if additive { "fs_particle_add" } else { "fs_particle" }),
                    targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, cull_mode: None, ..Default::default() },
                depth_stencil: Some(v.ds(false, wgpu::CompareFunction::GreaterEqual, false, Default::default())),
                multisample: v.ms(),
                multiview: None,
                cache: crate::render::pcache::get(),
            }))
        };
        let pipeline_particle = make_particle(false);
        let pipeline_particle_add = make_particle(true);
        let sky_hat = skyhat::SkyHat::new(&device, &globals_layout, &tex_layout, &sampler);
        // the wet road's planar reflection: the far models mirrored, the dome mirrored behind
        let refl_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("road reflection"),
            source: wgpu::ShaderSource::Wgsl(xs::split_consts(&[include_str!("refl.wgsl"), xs::text("xs_sky.wgsl"), xs::text("xs_sky_cheap.wgsl")].concat()).into()),
        });
        let refl_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("road reflection"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // the dome's Perlin target and detail noise (the far models' own textures hold group 1)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let refl_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("road reflection"), contents: bytemuck::bytes_of(&[0.0f32; 4]), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let refl_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("road reflection"), bind_group_layouts: &[&globals_layout, &tex_layout, &refl_layout], push_constant_ranges: &[] });
        // (the game turns its culling round for the mirrored models - 0x2205 = 0x18006 - so
        // that the ground flattened onto the mirror is not drawn over the picture.
        // Diagnostics: MCLA_REFL_CULL=none / front / back)
        let refl_cull = match std::env::var("MCLA_REFL_CULL").as_deref() {
            Ok("none") => None,
            Ok("back") => Some(wgpu::Face::Back),
            Ok("front") => Some(wgpu::Face::Front),
            _ => Some(wgpu::Face::Back),
        };
        let make_refl = |entry: &str| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("road reflection"),
            layout: Some(&refl_pl),
            vertex: wgpu::VertexState { module: &refl_shader, entry_point: Some("vs_refl"), buffers: &[Vertex::LAYOUT], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &refl_shader,
                entry_point: Some(entry),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: refl_cull, front_face: wgpu::FrontFace::Cw, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        let pipeline_refl = make_refl("fs_refl");
        let pipeline_refl_win = make_refl("fs_refl_win");
        let pipeline_refl_sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("road reflection sky"),
            layout: Some(&refl_pl),
            vertex: wgpu::VertexState { module: &refl_shader, entry_point: Some("vs_refl_sky"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &refl_shader,
                entry_point: Some("fs_refl_sky"),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        // (the rear-view mirror's sky - refl.wgsl `fs_mirror_sky`: into the scene target, writing the far depth
        // with the test off, so that it clears the strip's depth as well)
        let pipeline_mirror_sky = {
            let (refl_pl, refl_shader) = (refl_pl.clone(), refl_shader.clone());
            ms::MsPipe::new(&device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rear-view mirror sky"),
            layout: Some(&refl_pl),
            vertex: wgpu::VertexState { module: &refl_shader, entry_point: Some("vs_refl_sky"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &refl_shader,
                entry_point: Some("fs_mirror_sky"),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(v.ds(true, wgpu::CompareFunction::Always, true, Default::default())),
            multisample: v.ms(),
            multiview: None,
            cache: crate::render::pcache::get(),
        }))
        };
        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sky.wgsl").into()),
        });
        let sky_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pl"),
            bind_group_layouts: &[&globals_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let pipeline_sky = {
            let (sky_pl, sky_shader) = (sky_pl.clone(), sky_shader.clone());
            ms::MsPipe::new_shared(&device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_pl),
            vertex: wgpu::VertexState { module: &sky_shader, entry_point: Some("vs_sky"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &sky_shader,
                entry_point: Some("fs_sky"),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(v.ds(false, wgpu::CompareFunction::Always, true, Default::default())),
            multisample: v.ms(),
            multiview: None,
            cache: crate::render::pcache::get(),
        }))
        };
        let white = Arc::new(create_rgba(&device, &queue, 1, 1, &[255, 255, 255, 255], "white"));
        let clear_tex = Arc::new(create_rgba(&device, &queue, 1, 1, &[0, 0, 0, 0], "clear"));
        let refl_bg = make_refl_bg(&device, &refl_layout, &refl_buf, &sky_hat.buf, &clear_tex.view, &clear_tex.view, &sampler);
        let stencil_copy = create_stencil_copy(&device, config.width, config.height);
        let (resolve_layouts, resolve_pipes) = resolve_programs(&device);
        let depth_down = depth_down_program(&device);
        let (scene_w, scene_h) = (config.width, config.height);
        let depth_ui = depth.clone();
        Gpu {
            obj_pool: Default::default(),
            car_pool: Default::default(),
            default_car: Default::default(),
            device,
            queue,
            surface,
            config,
            depth,
            render_scale: 1.0,
            scene_w,
            scene_h,
            depth_ui,
            depth_down,
            depth_down_bg: None,
            scene_n: 1,
            scene_sets: [None, None, None],
            scene_used: [None, None, None],
            stencil_copy,
            resolve_layouts,
            resolve_pipes,
            globals_bg_alt,
            aniso16: false,
            post,
            globals_buf,
            lights_buf,
            globals_bg,
            env_faces,
            env_depth,
            env_globals,
            env_bgs,
            scrape_tex,
            garage_glow_tex,
            lamp_grid,
            lamp_data,
            globals_layout: globals_layout.clone(),
            tex_layout,
            own_masks,
            mat_layout,
            mat_bgs,
            obj_layout,
            blank_dmg,
            identity_obj,
            pipeline_opaque,
            pipeline_opaque_mode,
            pipeline_sea,
            pipeline_shore,
            pipeline_opaque_cull_mode,
            pipeline_opaque_cull,
            pipeline_opaque_cull_m,
            pipeline_stencil_only,
            pipeline_blend,
            pipeline_glass_back,
            pipeline_glass_front,
            pipeline_glass_inner,
            pipeline_pane,
            pipeline_sky,
            sky_hat,
            pipeline_inst,
            pipeline_inst_mode,
            pipeline_inst_blend,
            pipeline_glow,
            pipeline_env_glow,
            pipeline_env_blur,
            pipeline_env_dp,
            pipeline_env_dp_blend,
            env_dp_views,
            env_dp_depth,
            pipeline_env_fill,
            env_fill_bg,
            env_blur_bg,
            env_soft_faces,
            pipeline_env_mip,
            env_mip_views,
            env_mip_bgs,
            env_tex,
            env_soft_tex,
            env_glow_buf,
            pipeline_particle,
            pipeline_particle_add,
            particle_depth_layout,
            shadow_view,
            shadow_buf,
            shadow_bg,
            pipeline_shadow,
            pipeline_shadow_inst,
            casc_view,
            casc_tex,
            casc_pass_bufs,
            casc_pass_bgs,
            casc_buf,
            pipeline_casc,
            pipeline_casc_inst,
            pipeline_casc_cull,
            pipeline_casc_inst_cull,
            casc_cull_kinds,
            fc_work,
            fc_tex,
            fc_prev,
            fc_work_view,
            fc_view,
            fc_depth,
            fc_buf,
            fc_draw_bg,
            fc_blur_bg,
            pipeline_fc,
            pipeline_fc_blur,
            nfc_tex,
            nfc_work_view,
            nfc_view,
            nfc_buf,
            nfc_bg,
            pipeline_nfc,
            pipeline_nfc_inst,
            pipeline_nfc_blur,
            nfc_blur_bg,
            globals_bg_night,
            globals_bg_alt_night,
            mirror_bg_night,
            night_fc: false,
            refl_tex,
            refl_view,
            minisky_tex,
            minisky_view,
            refl_depth,
            refl_globals_bg,
            refl_buf,
            refl_bg,
            refl_layout,
            pipeline_refl,
            pipeline_refl_win,
            pipeline_refl_sky,
            mirror_globals,
            mirror_bg,
            floor_globals,
            floor_bg,
            pipeline_mirror_sky,
            sampler,
            white,
            clear_tex,
            textures: HashMap::new(),
            bind_cache: HashMap::new(),
            tex_idle: HashMap::new(),
            tex_trims: 0,
            bc_supported,
            max_tex,
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.resize_scene();
    }

    /// RENDER SCALE: the scene drawn at window x `scale` (1.0 / 1.25 / 1.5 / 1.75 / 2.0 on the page; 0.5 - 2.0
    /// taken); the targets made again when it changes.
    pub fn set_render_scale(&mut self, scale: f32) {
        // (down to 0.25: a fullscreen fallen back to borderless at a small resolution - hooks::fallback_scale)
        let scale = scale.clamp(0.25, 2.0);
        if (scale - self.render_scale).abs() > 1e-4 {
            self.render_scale = scale;
            self.resize_scene();
        }
    }

    /// The scene's targets at window x render scale.
    fn resize_scene(&mut self) {
        let (w, h) = (self.config.width.max(1), self.config.height.max(1));
        let lim = self.device.limits().max_texture_dimension_2d;
        let sw = ((w as f32 * self.render_scale).round() as u32).clamp(1, lim);
        let sh = ((h as f32 * self.render_scale).round() as u32).clamp(1, lim);
        if (sw, sh) != (self.scene_w, self.scene_h) {
            log::info!("scene: {sw}x{sh} for the {w}x{h} window (render scale {:.3})", self.render_scale);
        }
        self.scene_w = sw;
        self.scene_h = sh;
        self.depth = create_depth(&self.device, sw, sh);
        self.depth_ui = if (sw, sh) == (w, h) { self.depth.clone() } else { create_depth(&self.device, w, h) };
        self.depth_down_bg = ((sw, sh) != (w, h)).then(|| {
            let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("depth to ui"), contents: bytemuck::cast_slice(&[sw as f32 / w as f32, sh as f32 / h as f32, sw as f32, sh as f32]), usage: wgpu::BufferUsages::UNIFORM });
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("depth to ui"),
                layout: &self.depth_down.0,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.depth) }, wgpu::BindGroupEntry { binding: 1, resource: buf.as_entire_binding() }],
            })
        });
        self.post.resize(&self.device, sw, sh);
        self.post.set_out_size(&self.device, w, h);
        self.stencil_copy = create_stencil_copy(&self.device, sw, sh);
        self.scene_sets = [None, None, None];
        let n = self.scene_n;
        self.set_scene_samples(n);
    }

    /// RENDER SCALE: the scene's depth point-sampled into `depth_ui` (the window pixel's centre); nothing at scale 1.
    pub fn depth_to_ui(&self, enc: &mut wgpu::CommandEncoder) {
        let Some(bg) = self.depth_down_bg.as_ref() else { return };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("depth to ui"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.depth_ui, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.depth_down.1);
        pass.set_bind_group(0, bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The scene's sample count for this frame (`aamode::scene_samples`); makes its targets when first used.
    pub fn set_scene_samples(&mut self, n: u32) {
        let n = match n {
            0 | 1 => 1,
            2 => 2,
            _ => 4,
        };
        self.scene_n = n;
        let i = ms::scene(n).slot() - 1;
        let now = std::time::Instant::now();
        if self.scene_sets[i].is_none() {
            let (w, h) = (self.scene_w.max(1), self.scene_h.max(1));
            self.scene_sets[i] = Some(SceneSet::new(&self.device, &self.resolve_layouts, w, h, n));
            log::info!("scene targets: {n} samples made in {:.2} ms", now.elapsed().as_secs_f64() * 1000.0);
        }
        self.scene_used[i] = Some(now);
        // (a set of another sample count nothing has drawn into for SCENE_SET_IDLE goes - GitHub #48: the garage's and the
        // menus' 4x set, 164 MB at 1080p, stayed for the session after them; the original's multisampled targets are its
        // EDRAM's, which hold nothing between uses. Never the driving set (`aamode::scene_samples(false)`), which comes
        // back after every garage and menu; making the 4x set takes ~15 ms. MCLA_SCENE_SETS_KEEP=1: kept, as before)
        if std::env::var_os("MCLA_SCENE_SETS_KEEP").is_none() {
            let drive = ms::scene(match crate::aamode::scene_samples(false) { 0 | 1 => 1, 2 => 2, _ => 4 }).slot() - 1;
            for j in 0..3 {
                if j != i && j != drive && self.scene_sets[j].is_some() && self.scene_used[j].is_some_and(|t| now.duration_since(t) > SCENE_SET_IDLE) {
                    self.scene_sets[j] = None;
                    self.scene_used[j] = None;
                    log::info!("scene targets: slot {} freed (unused {} s)", j + 1, SCENE_SET_IDLE.as_secs());
                }
            }
        }
    }

    fn scene_set(&self) -> &SceneSet {
        self.scene_sets[ms::scene(self.scene_n).slot() - 1].as_ref().expect("set_scene_samples first")
    }

    /// The colour attachment of the scene passes: the multisampled target, or the scene target itself at 1 sample.
    pub fn scene_color(&self) -> &wgpu::TextureView {
        self.scene_set().color.as_ref().unwrap_or_else(|| self.post.scene_view())
    }

    /// The depth-stencil attachment of the scene passes (the scene's sample count, `ms::scene_ds_format`).
    pub fn scene_ds(&self) -> &wgpu::TextureView {
        &self.scene_set().ds
    }

    /// Depth and stencil of the scene resolved into `depth` / `stencil_copy`: sample 0 (the original's depth
    /// resolves use flags 0x14 - DEPTHSTENCIL | FRAGMENT0 by the SDK's values).
    pub fn resolve_scene_depth(&self, enc: &mut wgpu::CommandEncoder) {
        let set = self.scene_set();
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene depth resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.stencil_copy, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.resolve_pipes[if self.scene_n > 1 { 1 } else { 0 }]);
        pass.set_bind_group(0, &set.resolve_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The scene's colour resolved into the scene target (the samples averaged); nothing at 1 sample.
    pub fn resolve_scene_color(&self, enc: &mut wgpu::CommandEncoder) {
        let Some(c) = self.scene_set().color.as_ref() else { return };
        let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene colour resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: c, resolve_target: Some(self.post.scene_view()), ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    /// The city's sampler at maximum anisotropy 16 (the 4x mode, or TEXTURE FILTERING 16X) or the device default 4.
    pub fn set_aniso16(&mut self, on: bool) {
        if on != self.aniso16 {
            std::mem::swap(&mut self.globals_bg, &mut self.globals_bg_alt);
            std::mem::swap(&mut self.globals_bg_night, &mut self.globals_bg_alt_night);
            self.aniso16 = on;
        }
    }

    /// At night the night fast collector's map stands where the collector is (the original binds FastCollectorNight
    /// to s13 then): the globals' bind groups with it in binding 10 change places with the day's.
    pub fn set_night_collector(&mut self, on: bool) {
        if on != self.night_fc {
            std::mem::swap(&mut self.globals_bg, &mut self.globals_bg_night);
            std::mem::swap(&mut self.globals_bg_alt, &mut self.globals_bg_alt_night);
            std::mem::swap(&mut self.mirror_bg, &mut self.mirror_bg_night);
            self.night_fc = on;
        }
    }

    /// The night fast collector's pass (F-55997, state 4 at night): its constants (the view-projection and
    /// PS_ShadowNight's value, ShaderGlobals.z = 1 - m_ShadowFade), then the casters' map cleared to white (r26 = -1),
    /// with `pipeline_nfc` and the group 0 set: the caller draws the casters into it (`pipeline_nfc_inst` for the
    /// props), ends the pass and calls `night_collector_blur`.
    pub fn night_collector_begin<'e>(&self, enc: &'e mut wgpu::CommandEncoder, vp: Mat4, value: f32) -> wgpu::RenderPass<'e> {
        self.queue.write_buffer(&self.nfc_buf, 0, bytemuck::bytes_of(&CascPass { light_vp: vp.to_cols_array_2d(), bias: [value, 0.0, 0.0, 0.0] }));
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("night fast collector"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.nfc_work_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rp.set_pipeline(&self.pipeline_nfc);
        rp.set_bind_group(0, &self.nfc_bg, &[]);
        rp
    }

    /// ... and its PSBlur into the map the scene reads.
    pub fn night_collector_blur(&self, enc: &mut wgpu::CommandEncoder) {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("night fast collector blur"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.nfc_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rp.set_pipeline(&self.pipeline_nfc_blur);
        rp.set_bind_group(0, &self.nfc_blur_bg, &[]);
        rp.draw(0..3, 0..1);
    }


    pub fn new_object(&self, u: &ObjectUniform) -> ObjectBinding {
        make_object_binding(&self.device, &self.obj_layout, u, &self.blank_dmg)
    }

    /// The same with its uniform in a slot of a shared buffer (`ObjPool`): for objects written every frame through a
    /// `UniformBatch` (the cars' bodies). Its car block (`CarUniform`) is a slot of its own in `car_pool`.
    pub fn new_pooled_object(&self, u: &ObjectUniform) -> ObjectBinding {
        let (cb, co, cbuf) = pool_slot(&self.device, &self.queue, &self.car_pool, bytemuck::bytes_of(&u.car()), "car block pool");
        let car = CarRef { buf: cbuf, off: co, slot: Some(Arc::new(CarSlot { pool: self.car_pool.clone(), buf: cb, off: co })), shared: false };
        self.pooled_with_car(&u.part(), car)
    }

    /// A pooled object that shares `car_of`'s car block (a car's parts and windows: every row of it equal to the
    /// body's); its pushes write its own part only (`UniformBatch::push_part`).
    pub fn new_pooled_part(&self, car_of: &ObjectBinding) -> ObjectBinding {
        self.pooled_with_car(&ObjPart::default(), CarRef { shared: true, ..car_of.car.clone() })
    }

    /// A pooled object whose car block is `ObjectUniform::default()`'s, shared by all such (a car's wheels, rotors and
    /// calipers: their uniforms set none of the car block's rows).
    pub fn new_pooled_on_default_car(&self) -> ObjectBinding {
        let car = self.default_car.get_or_init(|| {
            let (cb, co, cbuf) = pool_slot(&self.device, &self.queue, &self.car_pool, bytemuck::bytes_of(&ObjectUniform::default().car()), "car block pool");
            CarRef { buf: cbuf, off: co, slot: Some(Arc::new(CarSlot { pool: self.car_pool.clone(), buf: cb, off: co })), shared: true }
        });
        self.pooled_with_car(&ObjPart::default(), car.clone())
    }

    fn pooled_with_car(&self, u: &ObjPart, car: CarRef) -> ObjectBinding {
        let (bi, off, buf) = pool_slot(&self.device, &self.queue, &self.obj_pool, bytemuck::bytes_of(u), "object pool");
        let bg = object_bind_group(&self.device, &self.obj_layout, &buf, off, &car, &self.blank_dmg);
        ObjectBinding { buf, bg, off, slot: Some(PoolSlot { pool: self.obj_pool.clone(), buf: bi }), car }
    }

    /// The object drawn with a damage map (`None`: the empty one) - its uniform kept.
    pub fn object_damage_map(&self, b: &mut ObjectBinding, map: Option<&wgpu::TextureView>) {
        b.bg = object_bind_group(&self.device, &self.obj_layout, &b.buf, b.off, &b.car, map.unwrap_or(&self.blank_dmg));
    }

    /// A car's damage map: 256 x 256 RGBA8, linear (crate::damagemap).
    pub fn new_damage_map(&self) -> wgpu::Texture {
        damage_map_texture(&self.device, crate::damagemap::SIZE as u32)
    }

    /// A traffic type's `_maxdamage` as its vertex programs read it (n x n RGBA8, as decoded).
    pub fn new_layout_texture(&self, n: u32, rgba: &[u8]) -> wgpu::Texture {
        let t = damage_map_texture(&self.device, n);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * n), rows_per_image: Some(n) },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        );
        t
    }

    pub fn write_damage_map(&self, t: &wgpu::Texture, px: &[[u8; 4]]) {
        let n = crate::damagemap::SIZE as u32;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(px),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * n), rows_per_image: Some(n) },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        );
    }

    pub fn update_object(&self, b: &ObjectBinding, u: &ObjectUniform) {
        let part = u.part();
        let v = bytemuck::bytes_of(&part);
        // (a pooled object: its value also into the pool's mirror, so a later flush of a changed range around it writes
        // this value; the write itself stays here, at once, as for an object of its own)
        if let Some(s) = b.slot.as_ref() {
            if let Ok(mut p) = s.pool.lock() {
                let o = b.off as usize;
                p.mirror[s.buf as usize][o..o + v.len()].copy_from_slice(v);
            }
        }
        self.queue.write_buffer(&b.buf, b.off, v);
        // (its car block, when it is its own; as the part)
        if !b.car.shared {
            let car = u.car();
            let v = bytemuck::bytes_of(&car);
            if let Some(s) = b.car.slot.as_ref() {
                if let Ok(mut p) = s.pool.lock() {
                    let o = s.off as usize;
                    p.mirror[s.buf as usize][o..o + v.len()].copy_from_slice(v);
                }
            }
            self.queue.write_buffer(&b.car.buf, b.car.off, v);
        }
    }

    /// Writes a batch's uniforms (`UniformBatch::flush`).
    pub fn flush_uniforms(&self, batch: &mut UniformBatch) {
        batch.flush(self);
    }

    pub fn write_lights(&self, l: &LightSet) {
        self.queue.write_buffer(&self.lights_buf, 0, bytemuck::bytes_of(l));
    }

    pub fn write_globals(&self, g: &Globals) {
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(g));
    }

    /// Upload a decoded texture; cached by `key`.
    /// The garage's lights in the cars' mirror map (garage round, 2026-10-13): the original adds a
    /// stored picture to its dual paraboloid map there - `textures/global/cars/globaltex/garageEnvMap.dds`,
    /// 256 x 128 RGBA8, the garage's ceiling lamps as 48 discs laid out as the map's two halves - with
    /// PS_Blit4x (specmapgen: 2 x the texel, blend one / one, over the whole 512 x 256 target; in the
    /// city the same draw runs with the colour channels masked off). `None`: black, outside the garage.
    pub fn set_garage_glow(&self, rgba: Option<&[u8]>) {
        let zero;
        let data = match rgba {
            Some(d) if d.len() == 256 * 128 * 4 => d,
            _ => {
                zero = vec![0u8; 256 * 128 * 4];
                &zero[..]
            }
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.garage_glow_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(128) },
            wgpu::Extent3d { width: 256, height: 128, depth_or_array_layers: 1 },
        );
    }

    /// Fill the cars' scrape texture (512 x 512 RGBA8).
    pub fn set_scrape(&self, rgba: &[u8]) {
        if rgba.len() != 512 * 512 * 4 {
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.scrape_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(512 * 4), rows_per_image: Some(512) },
            wgpu::Extent3d { width: 512, height: 512, depth_or_array_layers: 1 },
        );
    }

    /// Bins the city's lamps into the night grid: each cell keeps the four that light its
    /// middle most (which four the original's own table keeps where more reach a cell is not
    /// known; in the cell checked against its table - Selma Avenue by the Hollywood garage -
    /// the three placed lamps agree).
    pub fn set_lamps(&self, lamps: &[Lamp]) {
        let (gw, gh) = (LAMP_GRID.0 as usize, LAMP_GRID.1 as usize);
        let max = (LAMP_DATA.0 / 4 * LAMP_DATA.1) as usize - 1;
        let lamps = &lamps[..lamps.len().min(max)];
        let best = bin_lamps(lamps);
        let grid: Vec<u16> = best.iter().flat_map(|c| c.iter().map(|s| s.1)).collect();
        let mut data = vec![0.0f32; (LAMP_DATA.0 * LAMP_DATA.1 * 4) as usize];
        // The game's tables are coarse (its writer F-85100): x, z and the cone's two numbers
        // half floats cut off (not rounded), the colour, the axis and the height bytes cut off -
        // the height over the 110 m from -31 m up. The same numbers here.
        let unit = |b: u8| b as f32 / 255.0;
        for (i, l) in lamps.iter().enumerate() {
            let o = (i + 1) * 16;
            let r = lamp_row(l);
            let (hx, hz, c0, c1) = (half_value(r.a[0]), half_value(r.a[1]), half_value(r.a[2]), half_value(r.a[3]));
            let y = unit(r.c[0]) * 110.0 - 31.0;
            let axis = [unit(r.c[1]) * 2.0 - 1.0, unit(r.c[2]) * 2.0 - 1.0, unit(r.c[3]) * 2.0 - 1.0];
            data[o..o + 16].copy_from_slice(&[hx, y, hz, unit(r.b[0]) * 256.0, unit(r.b[1]), unit(r.b[2]), unit(r.b[3]), 0.0, axis[0], axis[1], axis[2], c0, c1, 0.0, 0.0, 0.0]);
        }
        let write = |tex: &wgpu::Texture, bytes: &[u8], w: u32, h: u32, bpp: u32| {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        };
        write(&self.lamp_grid, bytemuck::cast_slice(&grid), LAMP_GRID.0, LAMP_GRID.1, 8);
        write(&self.lamp_data, bytemuck::cast_slice(&data), LAMP_DATA.0, LAMP_DATA.1, 16);
        // (the same cells on this side, for the walkers' light picker - figlight.rs; the lamps there
        // as the light manager holds them, not cut down like the tables: the walkers' recorded
        // lanes settle on strengths 20, 40, 60, 70 exactly)
        let recs = std::iter::once(LampRec { cone: [0.0, 1.0], ..LampRec::default() })
            .chain(lamps.iter().map(|l| LampRec { pos: l.pos, strength: l.intensity, color: l.color, back: l.back, cone: l.cone }))
            .collect();
        if let Ok(mut t) = LAMP_TABLES.write() {
            *t = Some(Arc::new(LampTables { cells: best.iter().map(|c| [c[0].1, c[1].1, c[2].1, c[3].1]).collect(), recs }));
        }
        log::info!("night lamp grid: {} lamps, {} of {} cells lit", lamps.len(), best.iter().filter(|c| c[0].1 != 0).count(), gw * gh);
        // diagnostics: the table as built, for comparison with the original's (work/lighting/gridcmp.py)
        if let Ok(path) = std::env::var("MCLA_LAMP_DUMP") {
            let mut out: Vec<u8> = Vec::new();
            out.extend_from_slice(&(lamps.len() as u32).to_le_bytes());
            // (position, colour, axis and cone as the table holds them; the intensity as given)
            for (i, l) in lamps.iter().enumerate() {
                let d = &data[(i + 1) * 16..(i + 2) * 16];
                for v in [d[0], d[1], d[2], l.intensity, d[4] * 2.0, d[5] * 2.0, d[6] * 2.0, d[8], d[9], d[10], d[11], d[12]] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
            out.extend_from_slice(bytemuck::cast_slice(&grid));
            let _ = std::fs::write(&path, out);
            // (the lamps as given, before the writer's cuts: pos, colour, back, intensity, cone - 11 floats each)
            let mut raw: Vec<u8> = Vec::new();
            for l in lamps {
                for v in [l.pos.x, l.pos.y, l.pos.z, l.color.x, l.color.y, l.color.z, l.back.x, l.back.y, l.back.z, l.intensity, l.cone[0], l.cone[1]] {
                    raw.extend_from_slice(&v.to_le_bytes());
                }
            }
            let _ = std::fs::write(format!("{path}.raw"), raw);
        }
    }

    pub fn upload_texture(&mut self, key: u32, tex: &Texture) -> Arc<GpuTexture> {
        if let Some(t) = self.textures.get(&key) {
            return t.clone();
        }
        let t = Arc::new(upload(&self.device, &self.queue, tex, self.bc_supported));
        self.textures.insert(key, t.clone());
        t
    }

    /// The original's four shadow cascades for this frame, as its matrices have them in six
    /// recordings (work/lighting/sh_fit.py, sh_fit2.py - to the millimetre):
    ///   - all four look down the shadow rays (`ray`, the way the light travels); their
    ///     rectangles' long axis `top` is the view's HORIZONTAL direction made perpendicular to
    ///     the rays, the short one `right` = top x ray;
    ///   - tile i's rectangle is the bounding box, in (right, top), of a pyramid: its apex the
    ///     camera moved back by ViewportZShift along the view, its base FarSlice from the apex,
    ///     half angles `a = half the view's vertical angle x FovYDiv` up and down and
    ///     `a x aspect` to the sides (the angles are scaled, not their tangents);
    ///   - the scene takes a tile for a point only 0.1 m inside its rectangle (c83 .. c87).
    /// Writes the passes' uniforms and the scene's; returns the tiles' world -> clip matrices
    /// (x along `right`, y along `top`, depth down the rays), or None when off.
    /// `scale` = the shadow map object's slice and angle scales (+160, +164 of F-87623's object;
    /// (1, 1) in the city, eased to (0.3, 0.8) under the garage's lighting - `casc_scale` in main.rs):
    /// every FarSlice x scale[0], every half angle x scale[1], gFadeOutLastSMap x scale[0];
    /// ViewportZShift as it is (the garage's six recorded views to 0.1 mm, work/lighting-gaps/gs/fit.py).
    /// `light`: the direction to the light the scene's taps are gated by; its w goes to `CascUniform::light.w`.
    pub fn set_cascades(&self, on: bool, cam: Vec3, fwd: Vec3, up: Vec3, side: Vec3, tan_v: f32, aspect: f32, ray: Vec3, scale: [f32; 2], light: glam::Vec4) -> Option<[Mat4; 4]> {
        let mut u = CascUniform::zeroed();
        if !on {
            self.queue.write_buffer(&self.casc_buf, 0, bytemuck::bytes_of(&u));
            return None;
        }
        let h = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::Z);
        let top = (h - ray * ray.dot(h)).normalize_or(Vec3::Z);
        let right = top.cross(ray).normalize_or(Vec3::X);
        let half_fov = tan_v.atan();
        let mut out = [Mat4::IDENTITY; 4];
        for i in 0..4 {
            let [far, zshift, fov_div, slope] = CASC_TUNE[i];
            let far = far * scale[0];
            let a = half_fov * fov_div * scale[1];
            let (tv, th) = (a.tan(), (a * aspect).tan());
            let apex = cam - fwd * zshift;
            let (mut t0, mut t1, mut r0, mut r1) = (top.dot(apex), top.dot(apex), right.dot(apex), right.dot(apex));
            let (mut d0, mut d1) = (ray.dot(apex), ray.dot(apex));
            for (sv, sh) in [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
                let p = apex + (fwd + up * (sv * tv) + side * (sh * th)) * far;
                let (t, r) = (top.dot(p), right.dot(p));
                t0 = t0.min(t);
                t1 = t1.max(t);
                r0 = r0.min(r);
                r1 = r1.max(r);
                d0 = d0.min(ray.dot(p));
                d1 = d1.max(ray.dot(p));
            }
            // The depth (F-87623's end, 2026-10-27): the box's extent along the rays D times the tile's LightDepthScale
            // is F; the light's eye stands 0.5 F + CameraOffset (10) up the rays from the box's middle, its near plane
            // 5 m below the eye; the matrices' depth is (d - 5) / (F (F - 5)), d the way down the ray from the eye -
            // the recorded depth rows of every tile in the garage's views and the city's 09:00 / noon recordings to
            // 1e-5 (work/lighting-gaps/gs/zfit.py, zvar.py). The pyramid's near quad sits at the apex: the slices'
            // NearSlice goes in times the shadow map object's +168, which is 0. (MCLA_CASC_ZOLD=1: the port's fixed
            // 800 m from 600 m up, as before.)
            let fz = (d1 - d0) * CASC_LDS[i];
            let eye_d = (d0 + d1) * 0.5 - (0.5 * fz + 10.0);
            let (z_scale, z_off) = if casc_zold() { (1.0 / CASC_Z_RANGE, (CASC_Z_NEAR - ray.dot(cam)) / CASC_Z_RANGE) } else { let k = 1.0 / (fz * (fz - 5.0)); (k, -(eye_d + 5.0) * k) };
            let (tc, thh, rc, rh) = ((t0 + t1) * 0.5, (t1 - t0) * 0.5, (r0 + r1) * 0.5, (r1 - r0) * 0.5);
            if i == 3 {
                *CASC_EYE3.lock().unwrap() = (right * rc + top * tc + ray * eye_d).to_array();
            }
            let m = Mat4::from_cols(
                Vec4::new(right.x / rh, top.x / thh, ray.x * z_scale, 0.0),
                Vec4::new(right.y / rh, top.y / thh, ray.y * z_scale, 0.0),
                Vec4::new(right.z / rh, top.z / thh, ray.z * z_scale, 0.0),
                Vec4::new(-rc / rh, -tc / thh, z_off, 1.0),
            );
            out[i] = m;
            let (ox, oy) = ((i % 2) as f32 * 0.5, (i / 2) as f32 * 0.5);
            // Where the scene looks a point up in the map. The original's matrices (c64 ..) are
            // its depth draws' times a quarter plus the tile's place PLUS HALF A TEXEL each way,
            // while those draws have their pixel centres at .5 already (register 0x2301 bit 0):
            // every look-up lands half a texel right of and below the point's own place. And the
            // depth kept in a texel is that of sample 0 of a 4x multisampled pixel (surface
            // register 0x2000, the copy takes sample 0) - taken at D3D's standard place for it,
            // (-1/8, -3/8) of a pixel from the centre: together the map is read (3/8, 1/8) of a
            // texel off (MCLA_CASC_SHIFT=<x>,<y>: another shift, in texels). It is what speckles
            // the walls the rays graze: the tile's slope bias no longer covers the outer taps.
            let shift: Vec<f32> = std::env::var("MCLA_CASC_SHIFT").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
            // MEASURED 2026-10-15 on the original's own depth atlas read out of the running game (work/lighting
            // se_grab.py / se_dcmp.py / se_fit2.py, 09:00 tiles 2 and 3): a texel of it holds the surface the port's
            // map has (-0.53, -0.87) of a texel away - pixel centres at whole numbers and sample 0 at (-1/8, -3/8),
            // nominally (-0.625, -0.875) - so with the matrices' half texel the map is read (-1/8, -3/8) of a
            // texel off, not (+3/8, +1/8) as assumed until now (MCLA_CASC_SHIFT=0.375,0.125: as before).
            let (dx, dy) = (shift.first().copied().unwrap_or(-0.125) / (casc_tile() * 2) as f32, shift.get(1).copied().unwrap_or(-0.375) / (casc_tile() * 2) as f32);
            let to_map = Mat4::from_cols(Vec4::new(0.25, 0.0, 0.0, 0.0), Vec4::new(0.0, -0.25, 0.0, 0.0), Vec4::new(0.0, 0.0, 1.0, 0.0), Vec4::new(0.25 + ox + dx, 0.25 + oy + dy, 0.0, 1.0));
            u.m[i] = (to_map * m).to_cols_array_2d();
            u.top_off[i] = -tc;
            u.top_half[i] = thh - 0.1;
            u.right_off[i] = -rc;
            u.right_half[i] = rh - 0.1;
            // (bias.z: the cut-outs' alpha threshold - PS_ShadowDepthAlpha kills under 10 / 255;
            // MCLA_CASC_ALPHA: another)
            let alpha = std::env::var("MCLA_CASC_ALPHA").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(10.0 / 255.0);
            // (the constant bias is OFF since the tail round: the file's DepthBias of 1e-7 is next to nothing, and
            // CASC_BIAS_M's 1 .. 8 cm hid the grazing-light speckle the original shows - the awning at 09:00, the
            // 07:00 frame 1.4 -> 1.1. MCLA_CASC_BIASK=<slope factor>,<constant factor>: a trial; 1,1 = as before)
            let bk: Vec<f32> = std::env::var("MCLA_CASC_BIASK").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
            // (2026-10-15, measured on the original's atlas: the constant IS there - the file's DepthBias times the
            // tile's own depth range, which is 50 .. 320 KILOMETRES and changes with the hour and the view: 5 / 16 / 47 mm
            // at 09:00, 13 / 32 / 103 mm at 06:15; without it the ground of tiles 0 and 2 speckles where the original's
            // is clean. The range's rule was not found: `casc_const_bias` interpolates the standard view's six
            // recordings by the ray's east-west part - approx., the left view's ranges are up to 25 % smaller.
            // MCLA_CASC_BIASK=1,0: without; MCLA_CASC_BIASM=1: the three fixed means of 2026-10-13.)
            // (with the decoded depth the file's DepthBias goes in as it is, in the depth's own units)
            let cbias = if casc_zold() { (if std::env::var("MCLA_CASC_BIASM").is_ok() { CASC_BIAS_M[i] } else { casc_const_bias(i, ray.x) }) / CASC_Z_RANGE } else { CASC_DEPTH_BIAS[i] };
            let pass = CascPass { light_vp: m.to_cols_array_2d(), bias: [slope * bk.first().copied().unwrap_or(1.0), cbias * bk.get(1).copied().unwrap_or(1.0), alpha, 0.0] };
            self.queue.write_buffer(&self.casc_pass_bufs[i], 0, bytemuck::bytes_of(&pass));
        }
        // (c82.w: recorded as top . camera + 13.4 .. 14.1 - what it is was not found; the scene's
        // program meets it only where no tile holds the point)
        u.top = [top.x, top.y, top.z, top.dot(cam) + 13.85];
        // (right.w: the CARS' materials keep the look-up of before 2026-10-15, half a texel further each way - the
        // original's cars are no receivers of this pass at all (no car draw among PS_ShadowBlend's), their sun term
        // is the car work's; the measured shift darkened the R8's sunlit side by 1 - 2 levels in the left view.
        // MCLA_CASC_CARSHIFT=0: cars read the map like everything else)
        let car_shift = if std::env::var("MCLA_CASC_SHIFT").is_ok() || std::env::var("MCLA_CASC_CARSHIFT").map_or(false, |v| v.trim() == "0") { 0.0 } else { 0.5 / (casc_tile() * 2) as f32 };
        u.right = [right.x, right.y, right.z, car_shift];
        // (params.w = 1, MCLA_CASC_PXGRID=1: the taps' turn taken at the centres of a 1280 x 720 grid, as the
        // original's collector has it, whatever the window's size - a trial)
        u.light = light.to_array();
        u.params = [1.0, 82.0 * scale[0], 78.0 * scale[0], if std::env::var("MCLA_CASC_PXGRID").map_or(false, |v| v.trim() == "1") { 1.0 } else { 0.0 }];
        self.queue.write_buffer(&self.casc_buf, 0, bytemuck::bytes_of(&u));
        if let Ok(mut l) = CASC_LAST.lock() {
            *l = bytemuck::bytes_of(&u).to_vec();
        }
        Some(out)
    }

    /// One frame of the fast shadow collector (fast_collector.wgsl; LIGHTING_WORK.md "The
    /// collector pass decoded"): the far city's models round the camera drawn from straight
    /// above over the kept working map - their faces take the on / off sun shadow at the face
    /// and 8 m up, faded by the distance from an eye 100 m above the camera, roofs the fade
    /// alone -, the four-tap blur of that into the map the scene reads, and
    /// that map copied back as the next frame's start. The map is not shifted when its centre -
    /// the camera's whole metres - moves, as in the game. Returns the centre (world x, z).
    /// Needs this frame's shadow map.
    ///
    /// The game's shadow test in this pass answers "not lit" for every point outside its four
    /// cascades' rectangles (PS_ShadowFastBlend 10 - 21: no cascade matches, the matrix index
    /// runs past the array) - so the ground is lit only inside the largest of them and holds the
    /// bare fade elsewhere, as its map shows (work/lighting/fccasc2.py: the lit lot ends at the
    /// rectangle's edges). The rectangle lies in the plane across the shadow rays, along the
    /// view's direction: at the start spot at noon (the trace's c82 .. c87) from 7 m behind
    /// the camera to 151 m ahead and 47.5 m to either side. The same here, with the port's one
    /// shadow map inside it. (MCLA_FC_NORECT: lit outside too.)
    pub fn collector_pass<'a>(&self, enc: &mut wgpu::CommandEncoder, cam: Vec3, view_dir: Vec3, to_light: Vec3, light_vp: Mat4, models: impl Iterator<Item = &'a crate::world::GpuSector>) -> [f32; 2] {
        // The map over time, as the game keeps it (LIGHTING_WORK.md "The collector's two lit levels"; its
        // draws: clear, the models, resolve to the working map; PS_Blur of that; PS_Copy of LAST frame's
        // collector over it, alpha-blended with 229 / 255; resolve to the collector): the working map is
        // cleared and drawn afresh every frame, and the collector is `round((229 old + 26 blur(fresh)) / 255)`
        // in 8 bits, `old` read where the same ground lay last frame (the map's content moves with its centre,
        // whole texels). MCLA_FC_OLD=1: as until 2026-10-14 (no clear, the blur copied back, no history).
        // MCLA_FC_PRE=<frames>,<x>,<y>,<z>,<dx>,<dy>,<dz>: for the first frames the pass sees another camera
        // (place, view direction) and tests the single shadow map - a past for a still picture: the game's
        // captures were taken seconds after the camera had jumped from its idle place into the chase view.
        // MCLA_FC_START=<level 0..255>[,<frames>]: the map starts at this level and is held there for the first
        // frames (0: every texel then arrives from below and stops 4 under its goal - a map that was dark once,
        // as the game's standing one mostly is; 255: from above). MCLA_FC_W=<n>: the fresh map's weight of 255
        // (26; 255 = no history, the blurred fresh map itself - the middle of the band).
        static FC_KNOBS: std::sync::OnceLock<(f64, u32, f32)> = std::sync::OnceLock::new();
        let &(fc_level, fc_hold, fc_w) = FC_KNOBS.get_or_init(|| {
            let s: Vec<f64> = std::env::var("MCLA_FC_START").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
            let w = std::env::var("MCLA_FC_W").ok().and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(26.0).clamp(1.0, 255.0);
            ((s.first().copied().unwrap_or(255.0) / 255.0).clamp(0.0, 1.0), s.get(1).copied().unwrap_or(0.0) as u32, w)
        });
        static FC_STATE: std::sync::Mutex<(u32, Option<(f32, f32)>)> = std::sync::Mutex::new((0, None));
        static FC_PRE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
        let fc_old = std::env::var("MCLA_FC_OLD").is_ok();
        let pre = FC_PRE.get_or_init(|| std::env::var("MCLA_FC_PRE").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default());
        let (fc_frame, fc_last) = {
            let mut st = FC_STATE.lock().unwrap();
            let r = *st;
            st.0 += 1;
            r
        };
        let in_pre = pre.len() >= 7 && (fc_frame as f32) < pre[0];
        let (cam, view_dir) = if in_pre { (Vec3::new(pre[1], pre[2], pre[3]), Vec3::new(pre[4], pre[5], pre[6]).normalize_or(Vec3::Z)) } else { (cam, view_dir) };
        // (half the map's reach in metres - the original's 256 m at any SHADOW RESOLUTION; `fs` texels a metre)
        let half = FC_SIZE as f32 * 0.5;
        let fs = shadow_scale() as f32;
        let (cx, cz) = (cam.x.trunc(), cam.z.trunc());
        // from above: +x to the right, +z down the map, the highest face in front
        let (top, depth) = (1000.0f32, 1400.0f32);
        // The original's pass samples the world at each texel's CORNER, not its middle (its rasteriser's pixel
        // centres): the models are drawn half a texel further along +x and +z. Measured against the game's own
        // map at 07:00 / 09:00 / 17:30 (work/lighting/c6_fcstat.py, LIGHTING_WORK.md car round 12): mean abs
        // error of the ground layer 8.5 7.8 11.6 -> 7.5 6.7 10.5 over the map and 7.4 7.7 8.2 -> 5.0 3.9 3.2 in
        // the 13 m round the player's car; 0.25 / 1 texel and x alone are worse. MCLA_FC_HALF=<dx>,<dz>: another
        // displacement in texels (0,0 = as until 2026-10-14).
        let fc_half: Vec<f32> = std::env::var("MCLA_FC_HALF").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
        let (fhx, fhz) = (fc_half.first().copied().unwrap_or(0.5), fc_half.get(1).copied().unwrap_or(0.5));
        let view_proj = Mat4::from_cols(Vec4::new(1.0 / half, 0.0, 0.0, 0.0), Vec4::new(0.0, 0.0, -1.0 / depth, 0.0), Vec4::new(0.0, -1.0 / half, 0.0, 0.0), Vec4::new((fhx / fs - cx) / half, (cz - fhz / fs) / half, top / depth, 1.0));
        // (diagnostics: MCLA_FC_TAP=<dx>,<dy> shifts the blur's taps by so many texels,
        // MCLA_FC_FIRST draws level ground alone - the first reading of the pass)
        let tap: Vec<f32> = std::env::var("MCLA_FC_TAP").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
        let ray = light_vp.row(2).truncate().normalize_or(Vec3::Y);
        let top = (view_dir - ray * view_dir.dot(ray)).normalize_or(Vec3::Z);
        let right = top.cross(ray).normalize_or(Vec3::X);
        let u = FcUniform {
            view_proj: view_proj.to_cols_array_2d(),
            light_vp: light_vp.to_cols_array_2d(),
            // the pass's own eye (its c15: 823, 119.52, -816 at the start spot, the camera at
            // 19.5 m): the map's centre, 100 m above the camera; the reflection plane's height
            // (c43.z) as city.wgsl has it, 2 m under the camera
            eye: [cx, cam.y + 100.0, cz, cam.y - 2.0],
            // (w = 1, MCLA_FC_SINGLE: test the single shadow map as until 2026-10-14, not the cascades)
            to_light: [to_light.x, to_light.y, to_light.z, if in_pre || std::env::var("MCLA_FC_SINGLE").is_ok() { 1.0 } else { 0.0 }],
            blur: [tap.first().copied().unwrap_or(0.0), tap.get(1).copied().unwrap_or(0.0), if std::env::var("MCLA_FC_FIRST").is_ok() { 1.0 } else { 0.0 }, 0.0],
            top: [top.x, top.y, top.z, -top.dot(cam)],
            right: [right.x, right.y, right.z, -right.dot(cam)],
            rect: [-7.05, 151.05, 47.5, if std::env::var("MCLA_FC_NORECT").is_ok() { 0.0 } else { 1.0 }],
            hist: match (fc_old, fc_last) {
                (true, _) => [0.0; 4],
                (false, Some((lx, lz))) => [(cx - lx) * fs, (cz - lz) * fs, fc_w, 0.0],
                (false, None) => [0.0, 0.0, fc_w, 0.0],
            },
        };
        FC_STATE.lock().unwrap().1 = Some((cx, cz));
        self.queue.write_buffer(&self.fc_buf, 0, bytemuck::bytes_of(&u));
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fast collector"),
                // (the game clears the target before its draws - four rectangles, the trace's draws 1948 - 1951;
                // white here: the map's third byte, which every draw writes as 1, reads 255 all over the game's map)
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.fc_work_view, resolve_target: None, ops: wgpu::Operations { load: if fc_old { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::WHITE) }, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.fc_depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline_fc);
            rp.set_bind_group(0, &self.fc_draw_bg, &[]);
            for sec in models {
                let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
                let mut bound = false;
                for b in &sec.batches {
                    if b.max.x < cx - half || b.min.x > cx + half || b.max.z < cz - half || b.min.z > cz + half {
                        continue;
                    }
                    if !bound {
                        rp.set_vertex_buffer(0, vb.slice(..));
                        rp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                        bound = true;
                    }
                    rp.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                }
            }
        }
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fast collector blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.fc_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: fc_level, g: fc_level, b: 1.0, a: 1.0 }), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // (MCLA_FC_START's hold: the cleared level stands)
            if fc_old || fc_frame >= fc_hold {
                rp.set_pipeline(&self.pipeline_fc_blur);
                rp.set_bind_group(0, &self.fc_blur_bg, &[]);
                rp.draw(0..3, 0..1);
            }
        }
        let all = wgpu::Extent3d { width: fc_size(), height: fc_size(), depth_or_array_layers: 1 };
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.fc_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: if fc_old { &self.fc_work } else { &self.fc_prev }, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            all,
        );
        [cx, cz]
    }

    /// Diagnostics (MCLA_CASC_DUMP=<file>): the cascades' depth atlas of the last frame - two u32
    /// (width, height), a u32 = the byte length of the scene's cascade uniform, that uniform
    /// (`CascUniform`: the four look-up matrices first), then the depths as f32 rows (0 at 600 m up
    /// the rays from the camera, 1 at 200 m down them). Compare with the original's by
    /// work/lighting/se_dcmp.py.
    pub fn dump_casc(&self, path: &str) {
        let n = casc_tile() * 2;
        let bpr = n * 4;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("casc dump"), size: (bpr * n) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.casc_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::DepthOnly },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(n) } },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        let u = CASC_LAST.lock().map(|l| l.clone()).unwrap_or_default();
        out.extend_from_slice(&(u.len() as u32).to_le_bytes());
        out.extend_from_slice(&u);
        if let Ok(Ok(())) = rx.recv() {
            out.extend_from_slice(&slice.get_mapped_range());
        }
        buf.unmap();
        if let Err(e) = std::fs::write(path, &out) {
            log::warn!("casc dump {}: {}", path, e);
        }
    }

    /// Diagnostics (MCLA_FC_DUMP=<file>): the fast shadow collector's map as it is now - two
    /// u32 (width, height), then per texel two bytes: the ground's value and the one 8 m up.
    /// Rows run along +z, texels along +x; texel (128, 128) has the map's centre at its corner.
    /// Compare with the original's by work/lighting/fccmp.py.
    pub fn dump_fc(&self, path: &str) {
        let n = fc_size();
        let bpr = n * 4;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("fc dump"), size: (bpr * n) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            // (MCLA_FC_DUMP_WORK: the fresh map of the last frame, before its blur, instead)
            wgpu::TexelCopyTextureInfo { texture: if std::env::var("MCLA_FC_DUMP_WORK").is_ok() { &self.fc_work } else { &self.fc_tex }, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(n) } },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        if let Ok(Ok(())) = rx.recv() {
            let data = slice.get_mapped_range();
            for px in data.chunks_exact(4) {
                out.extend_from_slice(&px[..2]);
            }
        }
        buf.unmap();
        if let Err(e) = std::fs::write(path, &out) {
            log::warn!("fc dump {}: {}", path, e);
        }
    }

    /// Diagnostics (MCLA_NFC_DUMP=<file>): the night fast collector's map as it is now, a PGM picture (512 x 512 bytes,
    /// rows along +z, texels along +x, the eye at the middle).
    pub fn dump_nfc(&self, path: &str) {
        let n = NFC_SIZE;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("nfc dump"), size: (n * n) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.nfc_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n), rows_per_image: Some(n) } },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let mut out = format!("P5\n{} {}\n255\n", n, n).into_bytes();
        if let Ok(Ok(())) = rx.recv() {
            out.extend_from_slice(&slice.get_mapped_range());
        }
        buf.unmap();
        if let Err(e) = std::fs::write(path, &out) {
            log::warn!("nfc dump {}: {}", path, e);
        }
    }

    /// One frame of the wet road's planar reflection picture (refl.wgsl): the sky dome mirrored
    /// (when the game's dome is drawn: `sky`), then the far city's models - the batches drawn
    /// with CityLOD and CityWindowLOD, as in the game's pass - mirrored about `plane`, with the
    /// main view's matrix, which the globals hold. The road programs read it at the pixel's
    /// place on the screen (`t_refl`, city.wgsl `xs_road_s4`).
    ///
    /// Which models: the game's pass at the start spot (work/lighting/rf_match.py: its 60
    /// draws matched to these batches by their index counts) has every far model in view whose
    /// box lies within about 960 m of the camera - the detailed sectors' neighbourhood included
    /// - a few of those between 1,000 and 1,250 m, none beyond. Here: within 1,000 m.
    pub fn reflection_pass<'a>(&self, enc: &mut wgpu::CommandEncoder, view_proj: Mat4, px: [f32; 2], cam: Vec3, plane: f32, sky: bool, models: impl Iterator<Item = &'a crate::world::GpuSector>) -> usize {
        let sky = sky && self.sky_hat.tex_bg.is_some();
        self.queue.write_buffer(&self.refl_buf, 0, bytemuck::bytes_of(&[plane, if sky { 1.0f32 } else { 0.0 }, px[0], px[1]]));
        let frustum = Frustum::from_view_proj(view_proj);
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("road reflection"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.refl_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.refl_depth,
                // (kept: the garage floor's mirrored pass draws the room and the car over it - floormirror.rs)
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rp.set_bind_group(0, &self.refl_globals_bg, &[]);
        rp.set_bind_group(2, &self.refl_bg, &[]);
        if let (true, Some(tb)) = (sky, self.sky_hat.tex_bg.as_ref()) {
            rp.set_pipeline(&self.pipeline_refl_sky);
            // (group 1 is not read by the dome's programs; something has to be bound)
            rp.set_bind_group(1, tb, &[]);
            rp.draw(0..3, 0..1);
        }
        // (0: nothing set, 1: the walls' pipeline, 2: the windows')
        let mut set = 0u8;
        let mut n = 0usize;
        // (diagnostics: MCLA_REFL_ONLY=<part of a shader's name> draws those batches alone)
        let only = std::env::var("MCLA_REFL_ONLY").ok().map(|s| s.to_ascii_lowercase());
        // (MCLA_REFL_REACH=<metres>: another reach)
        let reach = std::env::var("MCLA_REFL_REACH").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1000.0);
        // (MCLA_REFL_LIST: the batches of the hundredth pass, listed)
        static PASSES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let list = PASSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 100 && std::env::var("MCLA_REFL_LIST").is_ok();
        for sec in models {
            let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
            let mut bound = false;
            for b in &sec.batches {
                // (CityLOD, CityWindowLOD; not the ground, water and plain models of the far city)
                let name = b.shader.as_bytes();
                let win = name.windows(13).any(|w| w.eq_ignore_ascii_case(b"citywindowlod"));
                let lod = win || name.windows(7).any(|w| w.eq_ignore_ascii_case(b"citylod"));
                // the mirrored batch lies between its top and that top's mirror image
                if only.as_ref().is_some_and(|o| !b.shader.to_ascii_lowercase().contains(o.as_str())) {
                    continue;
                }
                let (dx, dz) = ((b.min.x - cam.x).max(cam.x - b.max.x).max(0.0), (b.min.z - cam.z).max(cam.z - b.max.z).max(0.0));
                if dx * dx + dz * dz > reach * reach {
                    continue;
                }
                if !lod || !frustum.intersects_aabb(Vec3::new(b.min.x, (2.0 * plane - b.max.y).min(b.min.y), b.min.z), b.max) {
                    continue;
                }
                if !bound {
                    rp.set_vertex_buffer(0, vb.slice(..));
                    rp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    bound = true;
                }
                if list {
                    let c = (b.min + b.max) * 0.5;
                    eprintln!("refl batch {} {} n {} min {:.0} {:.0} {:.0} max {:.0} {:.0} {:.0} centre {:.0} {:.0}", n, b.shader, b.count, b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z, c.x, c.z);
                }
                if set != if win { 2 } else { 1 } {
                    rp.set_pipeline(if win { &self.pipeline_refl_win } else { &self.pipeline_refl });
                    set = if win { 2 } else { 1 };
                }
                rp.set_bind_group(1, b.bind.as_ref(), &[]);
                rp.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                n += 1;
            }
        }
        // the signal lamps' glow rectangles, after the city and the sky (siglrefl.rs)
        crate::siglrefl::draw(&mut rp);
        n
    }

    /// Diagnostics (MCLA_REFL_DUMP=<file>): the reflection picture as it is now - two u32
    /// (width, height), then per texel four f32 (r g b a), rows from the top.
    /// Compare with the original's by work/lighting/reflcmp.py.
    pub fn dump_refl(&self, path: &str) {
        let bpr = REFL_SIZE * 8;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("refl dump"), size: (bpr * REFL_SIZE) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.refl_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(REFL_SIZE) } },
            wgpu::Extent3d { width: REFL_SIZE, height: REFL_SIZE, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(&REFL_SIZE.to_le_bytes());
        out.extend_from_slice(&REFL_SIZE.to_le_bytes());
        if let Ok(Ok(())) = rx.recv() {
            let data = slice.get_mapped_range();
            for h in data.chunks_exact(2) {
                // (a half float, widened)
                let w = u16::from_le_bytes([h[0], h[1]]);
                let (s, e, m) = ((w >> 15) as u32, ((w >> 10) & 31) as i32, (w & 1023) as f32);
                let v = if e == 0 { m / 1024.0 * (2.0f32).powi(-14) } else if e == 31 { if m == 0.0 { f32::INFINITY } else { f32::NAN } } else { (1.0 + m / 1024.0) * (2.0f32).powi(e - 15) };
                out.extend_from_slice(&(if s == 1 { -v } else { v }).to_le_bytes());
            }
        }
        buf.unmap();
        if let Err(e) = std::fs::write(path, &out) {
            log::warn!("refl dump {}: {}", path, e);
        }
    }

    /// Diagnostics (MCLA_MINISKY_DUMP=<file>): the mini sky as it is now - two u32 (width,
    /// height), then per texel four bytes (r g b in the console's gamma, a), rows from the top.
    /// Compare with the original's by work/lighting/ms/cmp_tex.py.
    pub fn dump_minisky(&self, path: &str) {
        let bpr = MINISKY_SIZE * 4;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("mini sky dump"), size: (bpr * MINISKY_SIZE) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.minisky_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(MINISKY_SIZE) } },
            wgpu::Extent3d { width: MINISKY_SIZE, height: MINISKY_SIZE, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(&MINISKY_SIZE.to_le_bytes());
        out.extend_from_slice(&MINISKY_SIZE.to_le_bytes());
        if let Ok(Ok(())) = rx.recv() {
            out.extend_from_slice(&slice.get_mapped_range());
        }
        buf.unmap();
        if let Err(e) = std::fs::write(path, &out) {
            log::warn!("mini sky dump {}: {}", path, e);
        }
    }

    /// Diagnostics (MCLA_ENV_DUMP=<file>): the cars' environment cube and its blurred copy as
    /// they are now, written raw - "MENV", the two edge lengths, then per map six faces of
    /// RGBA half floats (+x -x +y -y +z -z). Read with work/lighting/envport.py.
    pub fn dump_env(&self, path: &str) {
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"MENV");
        out.extend_from_slice(&ENV_SIZE.to_le_bytes());
        out.extend_from_slice(&ENV_SOFT_SIZE.to_le_bytes());
        for (tex, size) in [(&self.env_tex, ENV_SIZE), (&self.env_soft_tex, ENV_SOFT_SIZE)] {
            let bpr = (size * 8 + 255) / 256 * 256;
            for face in 0..6u32 {
                let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("env dump"), size: (bpr * size) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
                let mut enc = self.device.create_command_encoder(&Default::default());
                enc.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z: face }, aspect: wgpu::TextureAspect::All },
                    wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(size) } },
                    wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                );
                self.queue.submit([enc.finish()]);
                let slice = buf.slice(..);
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(wgpu::MapMode::Read, move |r| {
                    let _ = tx.send(r);
                });
                self.device.poll(wgpu::Maintain::Wait);
                if let Ok(Ok(())) = rx.recv() {
                    let data = slice.get_mapped_range();
                    for y in 0..size {
                        out.extend_from_slice(&data[(y * bpr) as usize..(y * bpr + size * 8) as usize]);
                    }
                }
                buf.unmap();
            }
        }
        if let Err(e) = std::fs::write(path, &out) {
            log::warn!("env dump {}: {}", path, e);
        }
    }

    /// Level 0 of a 2D colour texture as raw bytes, rows packed (D59 checks).
    pub fn read_texture_bytes(&self, tex: &wgpu::Texture) -> Vec<u8> {
        let (w, h) = (tex.width(), tex.height());
        let bpp = tex.format().block_copy_size(None).unwrap_or(4);
        let bpr = (w * bpp + 255) / 256 * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("texture check"), size: (bpr * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((w * h * bpp) as usize);
        for y in 0..h {
            out.extend_from_slice(&data[(y * bpr) as usize..(y * bpr + w * bpp) as usize]);
        }
        drop(data);
        buf.unmap();
        out
    }

    /// The cars' environment cube's levels 1 .. ENV_MIPS - 1, all six faces, as raw bytes (for the D59 check below).
    fn env_mips_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for level in 1..ENV_MIPS {
            let size = ENV_SIZE >> level;
            let bpr = (size * 8 + 255) / 256 * 256;
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("env mip check"), size: (bpr * size * 6) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            let mut enc = self.device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &self.env_tex, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(size) } },
                wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 6 },
            );
            self.queue.submit([enc.finish()]);
            let slice = buf.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            self.device.poll(wgpu::Maintain::Wait);
            let data = slice.get_mapped_range();
            for row in 0..size * 6 {
                out.extend_from_slice(&data[(row * bpr) as usize..(row * bpr + size * 8) as usize]);
            }
            drop(data);
            buf.unmap();
        }
        out
    }

    /// D59 check (MCLA_ENV_MIP_CHECK=1): the cube's mips as the frame left them (only the face drawn that frame is
    /// re-mipped) against all six faces re-mipped now from their level 0. Returns (differing bytes, bytes).
    pub fn env_mip_check(&self) -> (usize, usize) {
        let a = self.env_mips_bytes();
        let mut enc = self.device.create_command_encoder(&Default::default());
        for (k, view) in self.env_mip_views.iter().enumerate() {
            let mut mp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("environment mips (check)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            mp.set_pipeline(&self.pipeline_env_mip);
            mp.set_bind_group(0, &self.env_mip_bgs[k], &[]);
            mp.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
        let b = self.env_mips_bytes();
        (a.iter().zip(&b).filter(|(x, y)| x != y).count(), a.len())
    }

    /// A texture the game reads without gamma decoding (see `upload_as`).
    pub fn upload_texture_raw(&mut self, key: u32, tex: &Texture) -> Arc<GpuTexture> {
        if let Some(t) = self.textures.get(&key) {
            return t.clone();
        }
        let t = Arc::new(upload_as(&self.device, &self.queue, tex, self.bc_supported, false));
        self.textures.insert(key, t.clone());
        t
    }

    pub fn texture_bind_group(&mut self, diffuse: &Arc<GpuTexture>, second: &Arc<GpuTexture>, k0: u32, k1: u32) -> Arc<wgpu::BindGroup> {
        self.texture_bind_group_c(diffuse, second, k0, k1, 0)
    }

    /// `texture_bind_group` with a draw's own sampler code (render/aniso.rs).
    pub fn texture_bind_group_c(&mut self, diffuse: &Arc<GpuTexture>, second: &Arc<GpuTexture>, k0: u32, k1: u32, code: u32) -> Arc<wgpu::BindGroup> {
        let third = self.clear_tex.clone();
        self.texture_bind_group3_c(diffuse, second, &third, k0, k1, 0, code)
    }

    /// A bind group of its own, kept out of `bind_cache`: for a texture made for one user (a licence plate), which then goes
    /// with its last holder - the cache keeps what it holds until the next `trim_textures` (GitHub #48: about 13 plates a second
    /// with the traffic, 512 x 256 RGBA each).
    pub fn texture_bind_group_own(&self, diffuse: &GpuTexture, second: &GpuTexture, code: u32) -> Arc<wgpu::BindGroup> {
        Arc::new(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&diffuse.view) },
                wgpu::BindGroupEntry { binding: 5, resource: self.own_masks[(code & 63) as usize].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&second.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.clear_tex.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&self.clear_tex.view) },
            ],
        }))
    }

    /// Three-texture material (ground shaders: diffuse, decal atlas, grime).
    /// The sky dome's textures (see `skyhat::SkyHat::bind`).
    pub fn bind_sky_hat(&mut self, stars: &GpuTexture, galaxy: &GpuTexture, detail: &GpuTexture, noise1: &GpuTexture) {
        self.sky_hat.bind(&self.device, &self.tex_layout, &self.own_masks[0], stars, galaxy, detail, noise1, &self.clear_tex);
        // (the reflection pass draws the dome too: the Perlin pass's target and the detail noise)
        self.refl_bg = make_refl_bg(&self.device, &self.refl_layout, &self.refl_buf, &self.sky_hat.buf, &self.sky_hat.perlin_view, &detail.view, &self.sampler);
    }

    pub fn texture_bind_group3(&mut self, diffuse: &Arc<GpuTexture>, second: &Arc<GpuTexture>, third: &Arc<GpuTexture>, k0: u32, k1: u32, k2: u32) -> Arc<wgpu::BindGroup> {
        self.texture_bind_group3_c(diffuse, second, third, k0, k1, k2, 0)
    }

    /// `texture_bind_group3` with a draw's own sampler code (render/aniso.rs).
    #[allow(clippy::too_many_arguments)]
    pub fn texture_bind_group3_c(&mut self, diffuse: &Arc<GpuTexture>, second: &Arc<GpuTexture>, third: &Arc<GpuTexture>, k0: u32, k1: u32, k2: u32, code: u32) -> Arc<wgpu::BindGroup> {
        let fourth = self.clear_tex.clone();
        self.texture_bind_group4_c(diffuse, second, third, &fourth, k0, k1, k2, 0, code)
    }

    /// Four-texture material (terrain: channel map and the three textures it mixes).
    #[allow(clippy::too_many_arguments)]
    pub fn texture_bind_group4(&mut self, diffuse: &Arc<GpuTexture>, second: &Arc<GpuTexture>, third: &Arc<GpuTexture>, fourth: &Arc<GpuTexture>, k0: u32, k1: u32, k2: u32, k3: u32) -> Arc<wgpu::BindGroup> {
        self.texture_bind_group4_c(diffuse, second, third, fourth, k0, k1, k2, k3, 0)
    }

    /// `texture_bind_group4` with a draw's own sampler code (render/aniso.rs; 0 = every fetch through the global sampler).
    #[allow(clippy::too_many_arguments)]
    pub fn texture_bind_group4_c(&mut self, diffuse: &Arc<GpuTexture>, second: &Arc<GpuTexture>, third: &Arc<GpuTexture>, fourth: &Arc<GpuTexture>, k0: u32, k1: u32, k2: u32, k3: u32, code: u32) -> Arc<wgpu::BindGroup> {
        let k2 = k2 ^ k3.rotate_left(7) ^ code.wrapping_mul(0x9E37_79B1);
        if let Some((b, _)) = self.bind_cache.get(&(k0, k1, k2)) {
            return b.clone();
        }
        let bg = Arc::new(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&diffuse.view) },
                wgpu::BindGroupEntry { binding: 5, resource: self.own_masks[(code & 63) as usize].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&second.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&third.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&fourth.view) },
            ],
        }));
        self.bind_cache.insert((k0, k1, k2), (bg.clone(), [diffuse.clone(), second.clone(), third.clone(), fourth.clone()]));
        bg
    }

    /// What `textures` holds, in bytes (the perf line, GitHub #48): [all, the city's and cars' (`texture_evictable`), of those
    /// unused now, the others (never dropped), of those unused now], and the count of `textures`.
    pub fn texture_census(&self) -> ([u64; 5], usize) {
        let mut c = [0u64; 5];
        for (k, t) in &self.textures {
            let (b, unused) = (t.bytes(), Arc::strong_count(t) == 1);
            c[0] += b;
            let i = if self.tex_idle.contains_key(k) { 1 } else { 3 };
            c[i] += b;
            if unused {
                c[i + 1] += b;
            }
        }
        (c, self.textures.len())
    }

    /// `key` in `textures` is a copy of a texture of the city: `trim_textures` may drop it when nothing uses it.
    pub fn texture_evictable(&mut self, key: u32, tex: &Texture) {
        self.tex_idle.entry(key).or_insert_with(|| (tex.levels.iter().map(|l| l.len() as u64).sum(), 0));
    }

    /// Keep what the unused city textures hold under `budget` bytes, the longest unused dropped first (without
    /// this every sector ever driven past stays in video memory: docs/PERFORMANCE.md, 2026-10-30). A sector
    /// that comes back uploads its textures again; the same name has the same content in every sector and
    /// the shared dictionaries (docs/MAP.md), so nothing drawn changes. Bind groups come first: a cached one
    /// nothing draws with would keep its textures alive.
    pub fn trim_textures(&mut self, budget: u64) {
        let t0 = std::time::Instant::now();
        let (n_bind, n_tex, b_tex) = (self.bind_cache.len(), self.textures.len(), TEX_BYTES.load(std::sync::atomic::Ordering::Relaxed));
        self.trim_textures_in(budget);
        // (MCLA_TRIMLOG=<ms>: a trim longer than that is logged with what it let go)
        static LIM: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
        if let Some(lim) = *LIM.get_or_init(|| std::env::var("MCLA_TRIMLOG").ok().and_then(|v| v.parse().ok())) {
            let ms = t0.elapsed().as_secs_f32() * 1000.0;
            if ms >= lim {
                let b_now = TEX_BYTES.load(std::sync::atomic::Ordering::Relaxed);
                log::info!("texture trim: {:.2} ms, bind groups {} -> {}, cache {} -> {}, texture MB {} -> {}", ms, n_bind, self.bind_cache.len(), n_tex, self.textures.len(), b_tex >> 20, b_now >> 20);
            }
        }
    }

    fn trim_textures_in(&mut self, budget: u64) {
        self.tex_trims += 1;
        self.bind_cache.retain(|_, e| Arc::strong_count(&e.0) > 1);
        let (mut idle, mut order) = (0u64, Vec::new());
        for (key, e) in self.tex_idle.iter_mut() {
            let unused = self.textures.get(key).is_some_and(|t| Arc::strong_count(t) == 1);
            if !unused {
                e.1 = 0;
                continue;
            }
            if e.1 == 0 {
                e.1 = self.tex_trims;
            }
            idle += e.0;
            order.push((e.1, *key, e.0));
        }
        if idle <= budget {
            return;
        }
        order.sort_unstable();
        for (_, key, bytes) in order {
            if idle <= budget {
                break;
            }
            self.textures.remove(&key);
            self.tex_idle.remove(&key);
            idle -= bytes;
        }
    }
}

/// Test hook MCLA_VRAM_BALLAST=<GB>: that much video memory taken by textures nothing draws, written once so the driver
/// commits them, kept for the whole run - a smaller card on a big one (a 24 GB card with 20 GB taken is a 4 GB card for
/// the game's allocations; docs/PERFORMANCE.md 2026-11-11 (5)).
fn vram_ballast(device: &wgpu::Device, queue: &wgpu::Queue) {
    static KEEP: std::sync::Mutex<Vec<wgpu::Texture>> = std::sync::Mutex::new(Vec::new());
    let Some(gb) = std::env::var("MCLA_VRAM_BALLAST").ok().and_then(|v| v.parse::<f32>().ok()) else { return };
    let n = (gb * 4.0).round() as usize; // 256 MB each: 8192 x 8192 RGBA8
    let row = vec![0x5au8; 8192 * 4 * 64];
    let mut keep = KEEP.lock().unwrap_or_else(|e| e.into_inner());
    for i in 0..n {
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vram ballast"),
            size: wgpu::Extent3d { width: 8192, height: 8192, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for y in (0..8192).step_by(64) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d { x: 0, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                &row,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8192 * 4), rows_per_image: Some(64) },
                wgpu::Extent3d { width: 8192, height: 64, depth_or_array_layers: 1 },
            );
        }
        queue.submit([]);
        let _ = device.poll(wgpu::Maintain::Wait);
        keep.push(t);
        if i % 8 == 7 {
            log::info!("vram ballast: {} MB", (i + 1) * 256);
        }
    }
    log::info!("vram ballast: {} MB taken", n * 256);
}

fn make_object_binding(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, u: &ObjectUniform, dmg: &wgpu::TextureView) -> ObjectBinding {
    // (one buffer: the object's part at 0, its car block after it at the next offset the device allows)
    let a = (device.limits().min_uniform_buffer_offset_alignment as u64).max(16);
    let co = (std::mem::size_of::<ObjPart>() as u64).div_ceil(a) * a;
    let mut bytes = vec![0u8; co as usize + std::mem::size_of::<CarUniform>()];
    bytes[..std::mem::size_of::<ObjPart>()].copy_from_slice(bytemuck::bytes_of(&u.part()));
    bytes[co as usize..].copy_from_slice(bytemuck::bytes_of(&u.car()));
    let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("object"), contents: &bytes, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
    let car = CarRef { buf: buf.clone(), off: co, slot: None, shared: false };
    let bg = object_bind_group(device, layout, &buf, 0, &car, dmg);
    ObjectBinding { buf, bg, off: 0, slot: None, car }
}

/// A free slot of `pool` (a new buffer of `POOL_SLOTS` when there is none), holding `first` - written now and kept in
/// the mirror, as a buffer of its own is made with its value: (buffer index, offset, buffer).
fn pool_slot(device: &wgpu::Device, queue: &wgpu::Queue, pool: &std::sync::Mutex<ObjPool>, first: &[u8], label: &str) -> (u32, u64, wgpu::Buffer) {
    let mut p = pool.lock().unwrap_or_else(|e| e.into_inner());
    if p.stride == 0 {
        let a = (device.limits().min_uniform_buffer_offset_alignment as u64).max(16);
        p.stride = (first.len() as u64).div_ceil(a) * a;
    }
    if p.free.is_empty() {
        let i = p.bufs.len() as u32;
        let bytes = p.stride * POOL_SLOTS;
        p.bufs.push(device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        p.mirror.push(vec![0u8; bytes as usize]);
        p.dirty.push(None);
        let stride = p.stride;
        p.free.extend((0..POOL_SLOTS).rev().map(|s| (i, s * stride)));
    }
    let (bi, off) = p.free.pop().unwrap();
    let buf = p.bufs[bi as usize].clone();
    let o = off as usize;
    p.mirror[bi as usize][o..o + first.len()].copy_from_slice(first);
    drop(p);
    queue.write_buffer(&buf, off, first);
    (bi, off, buf)
}

fn object_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, buf: &wgpu::Buffer, off: u64, car: &CarRef, dmg: &wgpu::TextureView) -> wgpu::BindGroup {
    let size = wgpu::BufferSize::new(std::mem::size_of::<ObjPart>() as u64);
    let car_size = wgpu::BufferSize::new(std::mem::size_of::<CarUniform>() as u64);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("object bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: buf, offset: off, size }) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(dmg) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &car.buf, offset: car.off, size: car_size }) },
        ],
    })
}

/// A damage map's texture (`n` x `n` RGBA8, linear: the map's values are read as they are).
fn damage_map_texture(device: &wgpu::Device, n: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("damage map"),
        size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// The reflection pass's own bind group (refl.wgsl group 2): its uniform, the sky dome's
/// constants, the dome's Perlin target and detail noise.
fn make_refl_bg(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, buf: &wgpu::Buffer, hat: &wgpu::Buffer, perlin: &wgpu::TextureView, detail: &wgpu::TextureView, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("road reflection"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: hat.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(perlin) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(detail) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

/// How long a scene target set of another sample count is kept unused (`Gpu::set_scene_samples`).
const SCENE_SET_IDLE: std::time::Duration = std::time::Duration::from_secs(60);

/// The scene's multisampled colour (none at 1 sample) and depth-stencil, and the resolve's bindings of it.
struct SceneSet {
    color: Option<wgpu::TextureView>,
    ds: wgpu::TextureView,
    resolve_bg: wgpu::BindGroup,
}

impl SceneSet {
    fn new(device: &wgpu::Device, layouts: &[wgpu::BindGroupLayout; 2], w: u32, h: u32, n: u32) -> SceneSet {
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let color = (n > 1).then(|| {
            device
                .create_texture(&wgpu::TextureDescriptor { label: Some("scene msaa"), size, mip_level_count: 1, sample_count: n, dimension: wgpu::TextureDimension::D2, format: post::SCENE_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] })
                .create_view(&Default::default())
        });
        let ds_tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("scene depth-stencil"), size, mip_level_count: 1, sample_count: n, dimension: wgpu::TextureDimension::D2, format: ms::scene_ds_format(), usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let ds = ds_tex.create_view(&Default::default());
        let dv = ds_tex.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
        let sv = ds_tex.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::StencilOnly, ..Default::default() });
        let resolve_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene resolve"),
            layout: &layouts[if n > 1 { 1 } else { 0 }],
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&dv) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&sv) }],
        });
        SceneSet { color, ds, resolve_bg }
    }
}

/// The depth / stencil resolve (sample 0), for 1 sample and for more.
fn resolve_programs(device: &wgpu::Device) -> ([wgpu::BindGroupLayout; 2], [wgpu::RenderPipeline; 2]) {
    let mk = |ms: bool| {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene resolve"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: ms }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: ms }, count: None },
            ],
        });
        let (dt, st) = if ms { ("texture_depth_multisampled_2d", "texture_multisampled_2d<u32>") } else { ("texture_depth_2d", "texture_2d<u32>") };
        let src = format!(
            "@group(0) @binding(0) var d: {dt};
@group(0) @binding(1) var s: {st};
struct O {{ @builtin(frag_depth) z: f32, @location(0) s: vec4<f32> }};
@vertex fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {{
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> O {{
    let c = vec2<i32>(p.xy);
    var o: O;
    o.z = textureLoad(d, c, 0);
    o.s = vec4<f32>(f32(textureLoad(s, c, 0).r) / 255.0, 0.0, 0.0, 0.0);
    return o;
}}
"
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("scene resolve"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scene resolve"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene resolve"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::Always, stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        (layout, pipe)
    };
    let (l0, p0) = mk(false);
    let (l1, p1) = mk(true);
    ([l0, l1], [p0, p1])
}

/// RENDER SCALE: the program of `Gpu::depth_to_ui` (frag_depth = the scene depth at the window pixel's centre).
fn depth_down_program(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("depth to ui"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ],
    });
    let src = "@group(0) @binding(0) var d: texture_depth_2d;
struct K { k: vec4<f32> };
@group(0) @binding(1) var<uniform> u: K;
@vertex fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    let c = clamp(vec2<i32>(p.xy * u.k.xy), vec2<i32>(0), vec2<i32>(u.k.zw) - vec2<i32>(1));
    return textureLoad(d, c, 0);
}
";
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("depth to ui"), source: wgpu::ShaderSource::Wgsl(src.into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("depth to ui"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
    let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("depth to ui"),
        layout: Some(&pl),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), targets: &[], compilation_options: Default::default() }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::Always, stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        multiview: None,
        cache: crate::render::pcache::get(),
    });
    (layout, pipe)
}

fn create_stencil_copy(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor { label: Some("stencil copy"), size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::R8Unorm, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
        .create_view(&Default::default())
}

fn create_depth(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        // (read by the lamps' glow pass, halo.rs)
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    t.create_view(&Default::default())
}

pub fn create_rgba(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32, rgba: &[u8], label: &str) -> GpuTexture {
    create_rgba_as(device, queue, w, h, rgba, label, true)
}

/// `gamma` as for `upload_as`: false keeps the texels as they are on sampling (a car's map, `car_tex_raw`).
pub fn create_rgba_as(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32, rgba: &[u8], label: &str, gamma: bool) -> GpuTexture {
    let format = if gamma { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
    // (test hook MCLA_TEX_FAIL_TEST=<label>: those textures asked far past the device's limit, so `create_texture` fails as on a full card)
    let fail = std::env::var("MCLA_TEX_FAIL_TEST").is_ok_and(|v| v == label);
    let Some(texture) = try_create_texture(device, &wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: if fail { 1 << 20 } else { w }, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    }) else {
        return white_texel(device, queue, format);
    };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&Default::default());
    GpuTexture::new(texture, view, w, h)
}

/// Build GPU texture from a decoded RAGE texture. Uses BC formats directly when
/// possible, otherwise software-decodes to RGBA8.
pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, tex: &Texture, bc: bool) -> GpuTexture {
    upload_as(device, queue, tex, bc, true)
}

/// `gamma`: the texels are colours to be decoded on sampling, as the game does for its city
/// textures (their fetch constants ask for gamma on red, green and blue); false for those it
/// reads as they are - the sky dome's noise, stars and galaxy, the cars' vinyls ...
pub fn upload_as(device: &wgpu::Device, queue: &wgpu::Queue, tex: &Texture, bc: bool, gamma: bool) -> GpuTexture {
    let (fmt, direct) = match tex.format {
        TexFormat::Dxt1 if bc => (wgpu::TextureFormat::Bc1RgbaUnormSrgb, true),
        TexFormat::Dxt3 if bc => (wgpu::TextureFormat::Bc2RgbaUnormSrgb, true),
        TexFormat::Dxt5 if bc => (wgpu::TextureFormat::Bc3RgbaUnormSrgb, true),
        TexFormat::Dxn if bc => (wgpu::TextureFormat::Bc5RgUnorm, true),
        TexFormat::Dxt5a if bc => (wgpu::TextureFormat::Bc4RUnorm, true),
        _ => (wgpu::TextureFormat::Rgba8UnormSrgb, false),
    };
    let fmt = if gamma { fmt } else { fmt.remove_srgb_suffix() };
    let w = tex.width.max(1);
    let h = tex.height.max(1);
    let (bw, bpb) = tex.format.block();
    // usable mip count
    let mut levels = 0u32;
    let (mut lw, mut lh) = (w, h);
    for (i, l) in tex.levels.iter().enumerate() {
        if direct && (lw < 4 || lh < 4) && i > 0 {
            break;
        }
        let need = if direct {
            ((lw as usize + 3) / 4) * ((lh as usize + 3) / 4) * bpb
        } else {
            ((lw as usize + bw - 1) / bw) * ((lh as usize + bw - 1) / bw) * bpb
        };
        if l.len() < need {
            break;
        }
        levels += 1;
        lw = (lw / 2).max(1);
        lh = (lh / 2).max(1);
    }
    let levels = levels.max(1);
    // (a texture read without a name gets one from its size, so the allocator report can tell them apart - GitHub #48)
    let unnamed = if tex.name.is_empty() { format!("unnamed texture {w}x{h}") } else { String::new() };
    let Some(texture) = try_create_texture(device, &wgpu::TextureDescriptor {
        label: Some(if tex.name.is_empty() { &unnamed } else { &tex.name }),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    }) else {
        return white_texel(device, queue, fmt);
    };
    let (mut lw, mut lh) = (w, h);
    for level in 0..levels {
        let data = &tex.levels[level as usize];
        let (bytes, bpr, rows) = if direct {
            let bx = (lw + 3) / 4;
            let by = (lh + 3) / 4;
            (std::borrow::Cow::Borrowed(&data[..]), bx * bpb as u32, by)
        } else {
            let rgba = mcla_formats::rage::xenos::decode_to_rgba8(tex.format, data, lw as usize, lh as usize);
            (std::borrow::Cow::Owned(rgba), lw * 4, lh)
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
        );
        lw = (lw / 2).max(1);
        lh = (lh / 2).max(1);
    }
    let view = texture.create_view(&Default::default());
    GpuTexture::new(texture, view, w, h)
}

/// What the original's frustum builder F-53768 scales a plane by, from its normal's squared length: the reciprocal square
/// root estimate (vrsqrtefp; the recomp's exact 1 / sqrt, used here) with one Newton-Raphson step (1.0, 0.5), an infinite
/// estimate (a zero normal) taken as 0. Bit for bit as the original's code run offline (tools/frustum_reference.py, test
/// `frustum_planes_match_the_original`); within an ulp of dividing by the length, as the port did before.
pub fn plane_scale(x: f32) -> f32 {
    let e = 1.0 / x.sqrt();
    let e = if e == f32::INFINITY { 0.0 } else { e };
    (1.0 - x * (e * e)) * (0.5 * e) + e
}

/// Simple frustum from a view-projection matrix (reverse-Z friendly: uses clip planes).
#[derive(Clone, Copy)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    pub fn from_view_proj(m: Mat4) -> Frustum {
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        let mut planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2];
        for p in planes.iter_mut() {
            *p *= plane_scale(p.truncate().length_squared());
        }
        Frustum { planes }
    }
    pub fn intersects_aabb(&self, mn: Vec3, mx: Vec3) -> bool {
        for p in &self.planes {
            let n = p.truncate();
            let v = Vec3::new(
                if n.x >= 0.0 { mx.x } else { mn.x },
                if n.y >= 0.0 { mx.y } else { mn.y },
                if n.z >= 0.0 { mx.z } else { mn.z },
            );
            if n.dot(v) + p.w < 0.0 {
                return false;
            }
        }
        true
    }
    #[cfg(test)]
    pub fn from_planes(planes: [Vec4; 6]) -> Frustum {
        Frustum { planes }
    }
    /// False when the sphere lies entirely outside one plane.
    pub fn intersects_sphere(&self, c: Vec3, r: f32) -> bool {
        self.planes.iter().all(|p| !(p.truncate().dot(c) + p.w < -r))
    }
    /// The viewport's sphere test F-29884: every plane's d = x X + (y Y + (z Z + w)) (a product then a sum each) at
    /// least 0 - r (NaN: outside). Checked against the original's code run offline (`view_sphere_tests`).
    pub fn sphere_touches(&self, c: Vec3, r: f32) -> bool {
        let d = |p: &Vec4| c.x * p.x + (c.y * p.y + (c.z * p.z + p.w));
        let nr = 0.0 - r;
        self.planes.iter().all(|p| d(p) >= nr)
    }
    /// The viewport's sphere test F-91781: every plane's (d + r) at least 0, d as in `sphere_touches`.
    pub fn sphere_plus(&self, c: Vec3, r: f32) -> bool {
        let d = |p: &Vec4| c.x * p.x + (c.y * p.y + (c.z * p.z + p.w));
        self.planes.iter().all(|p| d(p) + r >= 0.0)
    }
    /// The viewport's sphere test F-50958 (its planes as F-53768 lays them: six, two of them twice): per plane
    /// d = x X + (y Y + (z Z + w)), a product then a sum each; 0 when some d is below -r (or NaN), 2 when every d is
    /// at least r, else 1 (crossing).
    pub fn sphere_class(&self, c: Vec3, r: f32) -> u32 {
        let d = |p: &Vec4| c.x * p.x + (c.y * p.y + (c.z * p.z + p.w));
        let nr = 0.0 - r;
        self.planes.iter().all(|p| d(p) >= nr) as u32 + self.planes.iter().all(|p| d(p) >= r) as u32
    }
    /// True when every point lies more than `margin` outside the same plane.
    pub fn points_outside(&self, pts: &[Vec3], margin: f32) -> bool {
        self.planes.iter().any(|p| pts.iter().all(|v| p.truncate().dot(*v) + p.w < -margin))
    }

    /// One bit per plane the point lies more than `margin` outside of (the outside half of F-40381's clip codes).
    pub fn outside_bits(&self, v: Vec3, margin: f32) -> u32 {
        self.planes.iter().enumerate().fold(0, |m, (k, p)| if p.truncate().dot(v) + p.w < -margin { m | 1 << k } else { m })
    }
}

/// `surface.configure` with its error caught (wgpu's default handler panics on it): false when the swap chain could not
/// be made (GitHub #12: DX12 on an exclusive-fullscreen window, DXGI_ERROR_INVALID_CALL).
async fn configure_checked(device: &wgpu::Device, surface: &wgpu::Surface<'_>, config: &wgpu::SurfaceConfiguration) -> bool {
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    surface.configure(device, config);
    match device.pop_error_scope().await {
        None => true,
        Some(e) => {
            log::warn!("surface: configuration failed: {e}");
            false
        }
    }
}

#[cfg(test)]
mod frustum_reference_tests {
    use super::Frustum;

    /// The planes as F-53768 stores them (near, left, right, far, top, bottom) against `Frustum::from_view_proj` of the
    /// same product (its columns are the port's rows): every bit. CSV from tools/frustum_reference.py, path in
    /// MCLA_FRUSTUM_REFERENCE.
    #[test]
    #[ignore]
    fn frustum_planes_match_the_original() {
        let path = std::env::var("MCLA_FRUSTUM_REFERENCE").expect("MCLA_FRUSTUM_REFERENCE");
        let text = std::fs::read_to_string(path).unwrap();
        let mut n = 0;
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split(',').collect();
            let w: Vec<u32> = f[1..].iter().map(|v| u32::from_str_radix(v, 16).unwrap()).collect();
            assert_eq!(w.len(), 40, "{}", f[0]);
            let m = glam::Mat4::from_cols_array(&std::array::from_fn(|i| f32::from_bits(w[i])));
            let fr = Frustum::from_view_proj(m);
            for (k, port) in [4usize, 0, 1, 5, 3, 2].into_iter().enumerate() {
                let got = fr.planes[port].to_array().map(f32::to_bits);
                let want: [u32; 4] = std::array::from_fn(|i| w[16 + 4 * k + i]);
                assert_eq!(got, want, "{} plane {}", f[0], k);
            }
            n += 1;
        }
        assert!(n > 0);
    }
}

#[cfg(test)]
mod view_sphere_tests {
    use super::Frustum;
    use glam::{Vec3, Vec4};

    #[test]
    fn view_sphere_tests_match_the_original() {
        // tools/view_sphere_reference.py: F-29884 and F-91781 on 6,000 spheres against the 8 rows F-53768 lays out
        let text = include_str!("../testdata/view_sphere_reference.txt");
        let lines: Vec<&str> = text.lines().collect();
        let (mut n, mut inside) = (0, 0);
        for pair in lines.chunks(2) {
            let w: Vec<f32> = pair[0].split_whitespace().skip(2).map(|x| f32::from_bits(u32::from_str_radix(x, 16).unwrap())).collect();
            let o: Vec<&str> = pair[1].split_whitespace().collect();
            let mut planes = [Vec4::ZERO; 6];
            // (the original's planes P0..P5 in the port's order, as frustum_planes_match_the_original)
            for (k, port) in [4usize, 0, 1, 5, 3, 2].into_iter().enumerate() {
                planes[port] = Vec4::new(w[4 * k], w[4 * k + 1], w[4 * k + 2], w[4 * k + 3]);
            }
            let fr = Frustum::from_planes(planes);
            let (c, r) = (Vec3::new(w[24], w[25], w[26]), w[27]);
            let want: (u32, u32) = (o[1].parse().unwrap(), o[2].parse().unwrap());
            assert_eq!((fr.sphere_touches(c, r) as u32, fr.sphere_plus(c, r) as u32), want, "{}", pair[0]);
            inside += want.0;
            n += 1;
        }
        assert_eq!((n, inside), (6000, 1271));
    }
}

#[cfg(test)]
mod trim_tests {
    use super::*;

    /// F-19629's two tables as the original handed them to the cabin trim's program (all 56 trim draws of the recorded
    /// frame work/lighting/cap/am_base, the R8 with an imported save: intDL0 c205 + zone, intDL1 c230 + zone - bits from
    /// work/carparts2/trim/bits.py) against the port's rule: intDL1[z] = (SpecMapExp, SpecMapIntensity, Smoothness,
    /// Metallic) of the zone's kind and intDL0[z].w = its DetailTile, from the disc's tune file (intTune) through the
    /// fixed zone -> kind list; intDL0[z].rgb = the record's colour of the zone's group by the R8's own table - every
    /// zone of one group recorded with the same colour.
    const DL0: [[u32; 4]; 25] = [[0x3d70f0f2, 0x3d70f0f2, 0x3d70f0f2, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f600000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f600000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f600000], [0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000], [0x3e28a8a9, 0x3f048485, 0x00000000, 0x3f800000], [0x3d70f0f2, 0x3d70f0f2, 0x3d70f0f2, 0x3f800000], [0x3d70f0f2, 0x3d70f0f2, 0x3d70f0f2, 0x3f800000], [0x3d70f0f2, 0x3d70f0f2, 0x3d70f0f2, 0x3f800000], [0x3d70f0f2, 0x3d70f0f2, 0x3d70f0f2, 0x3f800000], [0x3d70f0f2, 0x3d70f0f2, 0x3d70f0f2, 0x3f800000], [0x3d70f0f2, 0x3d70f0f2, 0x3d70f0f2, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3d70f0f2, 0x3d70f0f2, 0x3d70f0f2, 0x3f800000], [0x3d70f0f2, 0x3d70f0f2, 0x3d70f0f2, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000], [0x3e989899, 0x3e989899, 0x3e989899, 0x3f800000]];
    const DL1: [[u32; 4]; 25] = [[0x3ed99999, 0x3c8f5c29, 0x00000000, 0x00000000], [0x3ed99999, 0x3c8f5c29, 0x00000000, 0x00000000], [0x3ed99999, 0x3c8f5c29, 0x00000000, 0x00000000], [0x3f39999a, 0x3e0ccccd, 0x00000000, 0x00000000], [0x3f39999a, 0x3e0ccccd, 0x00000000, 0x00000000], [0x3f39999a, 0x3e0ccccd, 0x00000000, 0x00000000], [0x3fcccccf, 0x3ec28f59, 0x00000000, 0x00000000], [0x3fcccccf, 0x3ec28f59, 0x00000000, 0x00000000], [0x3fcccccf, 0x3ec28f59, 0x00000000, 0x00000000], [0x3fa70a3d, 0x3ed0e560, 0x00000000, 0x3f000000], [0x3fa70a3d, 0x3ed0e560, 0x00000000, 0x3f000000], [0x3fa70a3d, 0x3ed0e560, 0x00000000, 0x3f000000], [0x3ecccccd, 0x3cccccd7, 0x00000000, 0x00000000], [0x3ecccccd, 0x3cccccd7, 0x00000000, 0x00000000], [0x3ecccccd, 0x3cccccd7, 0x00000000, 0x00000000], [0x3f000000, 0x3ca3d70a, 0x00000000, 0x00000000], [0x3f000000, 0x3ca3d70a, 0x00000000, 0x00000000], [0x3f000000, 0x3ca3d70a, 0x00000000, 0x00000000], [0x3f800000, 0x3f800000, 0x3f76872b, 0x3f800000], [0x3ecccccd, 0x3e23d709, 0x3e800000, 0x00000000], [0x3ecccccd, 0x3e23d709, 0x3e800000, 0x00000000], [0x3ecccccd, 0x3e23d709, 0x3e800000, 0x00000000], [0x3ecccccd, 0x3de147ae, 0x3e800000, 0x3f800000], [0x3ecccccd, 0x3de147ae, 0x3e800000, 0x3f800000], [0x3ecccccd, 0x3de147ae, 0x3e800000, 0x3f800000]];

    #[test]
    fn trim_tables_match_the_recording() {
        let data = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let Ok(text) = std::fs::read_to_string(std::path::Path::new(&data).join("tune/vehicle/0xdc0293e5")) else {
            eprintln!("(no game data: skipped)");
            return;
        };
        let t = trim_tune_from(&text);
        // (the file's values are the table the port keeps for a missing file)
        assert_eq!(t.map(|r| r.map(f32::to_bits)), TRIM_DISC.map(|r| r.map(f32::to_bits)));
        for z in 0..25 {
            let k = t[trim_kind(z)];
            assert_eq!([k[2], k[1], k[3], k[4]].map(f32::to_bits), DL1[z], "intDL1 of zone {z}");
            assert_eq!(k[0].to_bits(), DL0[z][3], "DetailTile of zone {z}");
        }
        let Some(g) = crate::mc4::zone_groups_dir(&std::path::Path::new(&data).join("resources/vehicle/vp_aud_r8_08")) else { return };
        for a in 0..25 {
            for b in 0..25 {
                if g[a] == g[b] {
                    assert_eq!(DL0[a][..3], DL0[b][..3], "zones {a} and {b}, group {}", g[a]);
                }
            }
        }
        // (group 3: the save's INTERIOR COLOR D, the green 42 / 255, 132 / 255, 0 as stored - no gamma on the way)
        assert_eq!(DL0[7][..3], [(42.0f32 / 255.0).to_bits(), (132.0f32 / 255.0).to_bits(), 0]);
    }
}

#[cfg(test)]
mod object_split_tests {
    use super::*;

    /// D59 object / car block split: the two blocks hold every row of the old `ObjectUniform` once, `join` puts them back
    /// byte for byte, and city.wgsl reads the moved rows from `car` only, its structs in the Rust order.
    #[test]
    fn split_is_the_old_uniform() {
        assert_eq!(std::mem::size_of::<ObjPart>(), 31 * 16);
        assert_eq!(std::mem::size_of::<CarUniform>(), 148 * 16);
        assert_eq!(std::mem::size_of::<ObjPart>() + std::mem::size_of::<CarUniform>(), std::mem::size_of::<ObjectUniform>());
        let mut u = ObjectUniform::default();
        let f: &mut [f32] = bytemuck::cast_slice_mut(bytemuck::bytes_of_mut(&mut u));
        for (i, x) in f.iter_mut().enumerate() {
            *x = i as f32 + 0.5;
        }
        let back = ObjectUniform::join(&u.part(), &u.car());
        assert_eq!(bytemuck::bytes_of(&back), bytemuck::bytes_of(&u));
        assert_eq!(ObjPart::default(), ObjectUniform::default().part());

        let src = include_str!("city.wgsl").replace("\r\n", "\n");
        let fields = |name: &str| -> Vec<String> {
            let i = src.find(&format!("struct {name} {{\n")).unwrap();
            let j = src[i..].find("\n};").unwrap();
            src[i..i + j].lines().skip(1).filter_map(|l| l.trim().split(':').next().map(str::to_string)).filter(|n| !n.is_empty() && !n.starts_with("//")).collect()
        };
        let obj = ["model", "color", "params", "dents", "color2", "fade_min", "fade_size", "tint", "fill", "neon_si", "trim", "brushed"];
        let car = ["interior", "lamp_a", "lamp_b", "area_a", "area_b", "slots", "glass_tint", "glass_bumped", "rest_rot"];
        assert_eq!(fields("Object"), obj);
        assert_eq!(fields("CarBlock"), car);
        for k in car {
            assert!(!src.contains(&format!("obj.{k}")), "city.wgsl still reads obj.{k}");
        }
    }
}
