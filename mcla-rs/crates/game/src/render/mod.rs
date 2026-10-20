//! wgpu renderer: device setup, static-geometry pipeline, texture cache.

pub mod car_lamps;
pub mod post;
pub mod skyhat;
pub mod xs;

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
}

pub const MAX_LIGHTS: usize = 48;
/// Where the game's data lies (set before the renderer is made; the paint types are read from it).
pub static DATA_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The paint types of the garage, in the order of its menu (`garage.ChangePaintType`: 1 Matte,
/// 2 Gloss, 3 Metallic, 4 Pearl, 5 Sparkle; 0 = the executable's "DirtyMatte").
pub const PAINT_TYPES: [&str; 6] = ["DIRTY MATTE", "MATTE", "GLOSS", "METALLIC", "PEARLESCENT", "SPARKLE"];

/// The shader with the disc's paint tuning written into its table: the six
/// `tune/vehicle/globalGfxTuning/<type>_PaintType` files (mcCarPaintTypeTuning; their names
/// survive only as hashes - which file is which type is told from its values: no reflection
/// at all for the mattes, blotchy noise for the dirty one, no flakes in gloss, fine ones in
/// metallic and pearl, coarse strong ones in sparkle).
fn paint_tuned(src: &str) -> String {
    const FILES: [&str; 6] = ["0xcec1263b", "0xcc0ab1c3", "0xa91a37a8", "0x3c752d39", "0x7c619591", "0x3547dd17"];
    const FIELDS: [&str; 12] = ["m_fDiffuseIntensity", "m_fSpecIntensity", "m_fAmbientIntensity", "m_fSpecExp", "m_fFresnelExp", "m_fFresnelMin", "m_fFresnelMax", "m_fPaintFalloff", "m_fPearlescence", "m_fNoiseScale", "m_fNoiseStrength", "m_fNoiseExponent"];
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

pub const SHADOW_SIZE: u32 = 4096;
/// The original's sun shadow maps: four cascades of 640 x 640 texels, two by two in one
/// 1280 x 1280 depth map (tile 1 right of tile 0, tiles 2 and 3 under them) - `Gpu::set_cascades`.
pub const CASC_TILE: u32 = 640;
/// tune/360CascadedShadowMaps.xml, per tile: FarSlice, ViewportZShift, FovYDiv, BiasSlope.
pub const CASC_TUNE: [[f32; 4]; 4] = [[15.0, 1.5, 0.653, 4.485], [31.0, 1.5, 0.75, 3.8], [80.0, 4.0, 0.7, 1.938], [160.0, 8.0, 0.7, 2.8]];
/// The tiles' constant depth bias in metres along the rays: the file's DepthBias (1e-7, 1e-7,
/// 4e-7, 0 of the depth range) over the depth scale of the recorded matrices. That scale changes
/// with the hour by a rule not found (5 .. 13, 16 .. 32, 47 .. 103 mm in six recordings): the means.
pub const CASC_BIAS_M: [f32; 4] = [0.0096, 0.024, 0.078, 0.0];
/// The cascades' constant depth bias in metres: tune/360CascadedShadowMaps.xml's DepthBias (1e-7, 1e-7, 4e-7, 0)
/// times the tile's depth range as the original's matrices have it in the standard view's recordings
/// (tg_6.25, 7, 9, 12, 15, 17.5b: 1 / |z row| of c64 ..), interpolated by the shadow ray's x.
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
/// (diagnostics: the scene's cascade uniform of the last frame, for `Gpu::dump_casc`)
static CASC_LAST: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());
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
    /// 1 / 4463.75) + (0.5485759, 0.5282554), x (512, 640); clamped to the grid as sub_82303900).
    pub fn cell(&self, p: Vec3) -> [u16; 4] {
        let u = p.x * (1.0 / 6320.0) + 0.5485759;
        let v = p.z * (1.0 / 4463.75) + 0.5282554;
        let cx = ((u * LAMP_GRID.0 as f32) as i64).clamp(0, LAMP_GRID.0 as i64 - 1) as usize;
        let cz = ((v * LAMP_GRID.1 as f32) as i64).clamp(0, LAMP_GRID.1 as i64 - 1) as usize;
        self.cells.get(cz * LAMP_GRID.0 as usize + cx).copied().unwrap_or([0; 4])
    }
}

pub static LAMP_TABLES: std::sync::RwLock<Option<Arc<LampTables>>> = std::sync::RwLock::new(None);

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
        }
    }
}

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
}

impl Default for ObjectUniform {
    fn default() -> Self {
        // (lamps without a table of their own: the Audi R8's, as before the tables were read)
        let (lamp_a, lamp_b) = car_lamps::R8.uniform();
        ObjectUniform { model: Mat4::IDENTITY.to_cols_array_2d(), color: [1.0, 1.0, 1.0, 0.0], params: [0.0; 4], dents: [[0.0; 4]; 6], color2: [0.0; 4], fade_min: [0.0; 4], fade_size: [1.0; 4], tint: [0.0; 4], interior: [[0.0; 4]; 5], fill: [0.0; 4], lamp_a, lamp_b, area_a: [[0.0; 4]; 5], area_b: [[0.0; 4]; 5] }
    }
}

/// A per-object uniform buffer + bind group (model matrix, paint colour, light state).
pub struct ObjectBinding {
    pub buf: wgpu::Buffer,
    pub bg: wgpu::BindGroup,
}

pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub depth: wgpu::TextureView,
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
    pub tex_layout: wgpu::BindGroupLayout,
    pub mat_layout: wgpu::BindGroupLayout,
    pub mat_bgs: HashMap<u32, wgpu::BindGroup>,
    pub obj_layout: wgpu::BindGroupLayout,
    /// the object's damage map when it has none (all zero)
    pub blank_dmg: wgpu::TextureView,
    pub identity_obj: ObjectBinding,
    pub pipeline_opaque: wgpu::RenderPipeline,
    pub pipeline_blend: wgpu::RenderPipeline,
    /// car windows: back faces, then front faces, both with depth writes
    pub pipeline_glass_back: wgpu::RenderPipeline,
    pub pipeline_glass_front: wgpu::RenderPipeline,
    /// the player's windows in the driver's view (interiorwindowglass): the back faces' cull, no depth writes
    pub pipeline_glass_inner: wgpu::RenderPipeline,
    /// shop panes (PSCityWindowLower and its night program): blended WITH depth writes, like the original's draws
    pub pipeline_pane: wgpu::RenderPipeline,
    pub pipeline_sky: wgpu::RenderPipeline,
    /// the game's own sky dome programs and their Perlin pass (skyhat.rs)
    pub sky_hat: skyhat::SkyHat,
    /// opaque / alpha-tested geometry drawn with per-instance transforms (props)
    pub pipeline_inst: wgpu::RenderPipeline,
    /// the glass props: instanced, blended, both sides, depth tested but not written (the original's draws: blend
    /// control 0x07060706, cull register 0x18000, depth control 0x00708763)
    pub pipeline_inst_blend: wgpu::RenderPipeline,
    /// additive light glows (instanced billboards)
    pub pipeline_glow: wgpu::RenderPipeline,
    /// the light glows of the environment map (env_glow.wgsl) and their instances
    /// (position + strength, colour: 8 floats each, at most ENV_GLOWS)
    pub pipeline_env_glow: wgpu::RenderPipeline,
    /// the blurred copy of the environment cube (env_blur.wgsl): its pipeline, the sharp
    /// cube as its input, and its six faces to draw into
    pub pipeline_env_blur: wgpu::RenderPipeline,
    /// the cars' map drawn the original's way: the city into two paraboloid halves (warp per
    /// vertex), the cube filled from them
    pub pipeline_env_dp: wgpu::RenderPipeline,
    pub pipeline_env_dp_blend: wgpu::RenderPipeline,
    pub env_dp_views: Vec<wgpu::TextureView>,
    pub env_dp_depth: wgpu::TextureView,
    pub pipeline_env_fill: wgpu::RenderPipeline,
    pub env_fill_bg: wgpu::BindGroup,
    pub env_blur_bg: wgpu::BindGroup,
    pub env_soft_faces: Vec<wgpu::TextureView>,
    /// the cars' environment cube and its blurred copy (kept for `dump_env`)
    pub env_tex: wgpu::Texture,
    pub env_soft_tex: wgpu::Texture,
    pub env_glow_buf: wgpu::Buffer,
    pub pipeline_particle: wgpu::RenderPipeline,
    pub pipeline_particle_add: wgpu::RenderPipeline,
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
    pub sampler: wgpu::Sampler,
    pub white: Arc<GpuTexture>,
    pub textures: HashMap<u32, Arc<GpuTexture>>,
    pub bind_cache: HashMap<(u32, u32, u32), Arc<wgpu::BindGroup>>,
    /// 1x1 transparent texture (no grime layer)
    pub clear_tex: Arc<GpuTexture>,
    pub bc_supported: bool,
    pub max_tex: u32,
}

impl Gpu {
    pub async fn new(window: Arc<winit::window::Window>) -> Result<Gpu> {
        // (the game's own sky / road / sea programs: translated from the disc at install time, read once here - xs.rs)
        xs::load(&crate::hooks::get().map(|r| r.data.path.clone()).unwrap_or_else(|| std::path::PathBuf::from(".")));
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let surface = instance.create_surface(window.clone()).context("create surface")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no suitable GPU adapter")?;
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
        crate::hooks::set_adapter(&info);
        let feats = adapter.features();
        let bc_supported = feats.contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let mut required = wgpu::Features::empty();
        if bc_supported {
            required |= wgpu::Features::TEXTURE_COMPRESSION_BC;
        }
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
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .context("request device")?;
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
        surface.configure(&device, &config);
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
        let shadow_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow globals"),
            contents: bytemuck::bytes_of(&CascPass { light_vp: Mat4::IDENTITY.to_cols_array_2d(), bias: [0.0; 4] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let shadow_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow bg"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_buf.as_entire_binding() }],
        });
        // the original's four cascades: their depth map, a uniform and bind group per tile's
        // pass, and what the scene reads them with
        let casc_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow cascades"),
            size: wgpu::Extent3d { width: CASC_TILE * 2, height: CASC_TILE * 2, depth_or_array_layers: 1 },
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
            .map(|b| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("cascade pass bg"), layout: &shadow_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() }] }))
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
                size: wgpu::Extent3d { width: FC_SIZE, height: FC_SIZE, depth_or_array_layers: 1 },
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
        let fc_depth = create_depth(&device, FC_SIZE, FC_SIZE);
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
                mip_level_count: 1,
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
        let env_faces: Vec<wgpu::TextureView> = (0..6).map(|i| env_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: i, array_layer_count: Some(1), ..Default::default() })).collect();
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
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("environment"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let make_globals_bg = |buf: &wgpu::Buffer, cube: &wgpu::TextureView, soft: &wgpu::TextureView, refl: &wgpu::TextureView| {
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
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&fc_view) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(refl) },
                    wgpu::BindGroupEntry { binding: 12, resource: casc_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(&casc_view) },
                    wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(&minisky_view) },
                ],
            })
        };
        let globals_bg = make_globals_bg(&globals_buf, &env_cube, &env_soft_cube, &refl_view);
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
            cache: None,
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
            cache: None,
        });
        // (six for the cube's faces, two for the paraboloid halves)
        let env_globals: Vec<wgpu::Buffer> = (0..8)
            .map(|_| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("environment globals"), contents: bytemuck::bytes_of(&Globals::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST }))
            .collect();
        let env_bgs: Vec<wgpu::BindGroup> = env_globals.iter().map(|b| make_globals_bg(b, &env_none_cube, &env_none_cube, &fc_view)).collect();
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
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
        let mat_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mat layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
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
            ],
        });
        let blank_dmg = damage_map_texture(&device, 1).create_view(&Default::default());
        let identity_obj = make_object_binding(&device, &obj_layout, &ObjectUniform::default(), &blank_dmg);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("city"),
            source: wgpu::ShaderSource::Wgsl(paint_tuned(&[include_str!("city.wgsl"), xs::text("xs_road.wgsl"), xs::text("xs_roadn.wgsl"), xs::text("xs_water.wgsl")].concat()).into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("city pl"),
            bind_group_layouts: &[&globals_layout, &tex_layout, &mat_layout, &obj_layout],
            push_constant_ranges: &[],
        });
        let make_pipeline_x = |blend: bool, vs: &'static str, cull: Option<wgpu::Face>, zwrite: bool| {
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
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: cull,
                    front_face: wgpu::FrontFace::Cw,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: zwrite,
                    depth_compare: wgpu::CompareFunction::GreaterEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
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
        let pipeline_blend = make_pipeline(true, "vs_main");
        // (the cars' environment map drawn the original's way: the paraboloid warp per vertex - city.wgsl vs_env_dp)
        let pipeline_env_dp = make_pipeline(false, "vs_env_dp");
        let pipeline_env_dp_blend = make_pipeline(true, "vs_env_dp");
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
                cache: None,
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
                cache: None,
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
            cache: None,
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
            cache: None,
        });
        let pipeline_inst = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, front_face: wgpu::FrontFace::Cw, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let pipeline_inst_blend = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let glow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glow.wgsl").into()),
        });
        let glow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glow pl"),
            bind_group_layouts: &[&globals_layout],
            push_constant_ranges: &[],
        });
        let pipeline_glow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
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
            cache: None,
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
            let blend = if additive {
                wgpu::BlendState {
                    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                    alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                }
            } else {
                wgpu::BlendState::ALPHA_BLENDING
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::GreaterEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let pipeline_particle = make_particle(false);
        let pipeline_particle_add = make_particle(true);
        let sky_hat = skyhat::SkyHat::new(&device, &globals_layout, &tex_layout);
        // the wet road's planar reflection: the far models mirrored, the dome mirrored behind
        let refl_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("road reflection"),
            source: wgpu::ShaderSource::Wgsl([include_str!("refl.wgsl"), xs::text("xs_sky.wgsl"), xs::text("xs_sky_cheap.wgsl")].concat().into()),
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
            cache: None,
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
            cache: None,
        });
        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sky.wgsl").into()),
        });
        let sky_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pl"),
            bind_group_layouts: &[&globals_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let pipeline_sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("aniso"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            // (diagnostics, p2day 2026-10-13: MCLA_ANISO=<1..16>; the original's city fetches ask for 4 : 1)
            anisotropy_clamp: std::env::var("MCLA_ANISO").ok().and_then(|v| v.parse::<u16>().ok()).map_or(8, |v| v.clamp(1, 16)),
            ..Default::default()
        });
        let white = Arc::new(create_rgba(&device, &queue, 1, 1, &[255, 255, 255, 255], "white"));
        let clear_tex = Arc::new(create_rgba(&device, &queue, 1, 1, &[0, 0, 0, 0], "clear"));
        let refl_bg = make_refl_bg(&device, &refl_layout, &refl_buf, &sky_hat.buf, &clear_tex.view, &clear_tex.view, &sampler);
        let max_tex = limits.max_texture_dimension_2d;
        Ok(Gpu {
            device,
            queue,
            surface,
            config,
            depth,
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
            tex_layout,
            mat_layout,
            mat_bgs,
            obj_layout,
            blank_dmg,
            identity_obj,
            pipeline_opaque,
            pipeline_blend,
            pipeline_glass_back,
            pipeline_glass_front,
            pipeline_glass_inner,
            pipeline_pane,
            pipeline_sky,
            sky_hat,
            pipeline_inst,
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
            sampler,
            white,
            clear_tex,
            textures: HashMap::new(),
            bind_cache: HashMap::new(),
            bc_supported,
            max_tex,
        })
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.depth = create_depth(&self.device, w, h);
        self.post.resize(&self.device, w, h);
    }

    pub fn new_object(&self, u: &ObjectUniform) -> ObjectBinding {
        make_object_binding(&self.device, &self.obj_layout, u, &self.blank_dmg)
    }

    /// The object drawn with a damage map (`None`: the empty one) - its uniform kept.
    pub fn object_damage_map(&self, b: &mut ObjectBinding, map: Option<&wgpu::TextureView>) {
        b.bg = object_bind_group(&self.device, &self.obj_layout, &b.buf, map.unwrap_or(&self.blank_dmg));
    }

    /// A car's damage map: 256 x 256 RGBA8, linear (crate::damagemap).
    pub fn new_damage_map(&self) -> wgpu::Texture {
        damage_map_texture(&self.device, crate::damagemap::SIZE as u32)
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
        self.queue.write_buffer(&b.buf, 0, bytemuck::bytes_of(u));
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
        let (cw, ch) = (LAMP_SPAN.0 / gw as f32, LAMP_SPAN.1 / gh as f32);
        let max = (LAMP_DATA.0 / 4 * LAMP_DATA.1) as usize - 1;
        let lamps = &lamps[..lamps.len().min(max)];
        // The game's own rule (its grid builder, sub_82303A58 of the executable, LIGHTING_WORK.md):
        // a lamp reaches as far as it still gives a hundredth of its light; a spot that points
        // down covers the stretch of ground (15 m below) its cone falls on instead. A cell keeps
        // the four lamps with the highest score `term / distance squared` taken at the cell's
        // centre from the lamp's own position, the score a 16 bit number in steps of 1 / 256
        // that wraps round; equal scores keep the earlier lamp.
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
                    let d2 = (l.pos.x - mx).powi(2) + (l.pos.z - mz).powi(2);
                    let score = ((term / d2 * 256.0) as i64 & 0xffff) as u16;
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
        let grid: Vec<u16> = best.iter().flat_map(|c| c.iter().map(|s| s.1)).collect();
        let mut data = vec![0.0f32; (LAMP_DATA.0 * LAMP_DATA.1 * 4) as usize];
        // The game's tables are coarse (its writer sub_823033F0): x, z and the cone's two numbers
        // half floats cut off (not rounded), the colour, the axis and the height bytes cut off -
        // the height over the 110 m from -31 m up. The same numbers here.
        let half = |v: f32| f32::from_bits(v.to_bits() & 0xffff_e000);
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).trunc() / 255.0;
        for (i, l) in lamps.iter().enumerate() {
            let o = (i + 1) * 16;
            let y = byte((l.pos.y + 31.0) / 110.0) * 110.0 - 31.0;
            let axis = [byte(0.5 + 0.5 * l.back.x) * 2.0 - 1.0, byte(0.5 + 0.5 * l.back.y) * 2.0 - 1.0, byte(0.5 + 0.5 * l.back.z) * 2.0 - 1.0];
            data[o..o + 16].copy_from_slice(&[half(l.pos.x), y, half(l.pos.z), byte(l.intensity / 256.0) * 256.0, byte(l.color.x * 0.5), byte(l.color.y * 0.5), byte(l.color.z * 0.5), 0.0, axis[0], axis[1], axis[2], half(l.cone[0]), half(l.cone[1]), 0.0, 0.0, 0.0]);
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
            let _ = std::fs::write(path, out);
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
    pub fn set_cascades(&self, on: bool, cam: Vec3, fwd: Vec3, up: Vec3, side: Vec3, tan_v: f32, aspect: f32, ray: Vec3) -> Option<[Mat4; 4]> {
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
            let a = half_fov * fov_div;
            let (tv, th) = (a.tan(), (a * aspect).tan());
            let apex = cam - fwd * zshift;
            let (mut t0, mut t1, mut r0, mut r1) = (top.dot(apex), top.dot(apex), right.dot(apex), right.dot(apex));
            for (sv, sh) in [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
                let p = apex + (fwd + up * (sv * tv) + side * (sh * th)) * far;
                let (t, r) = (top.dot(p), right.dot(p));
                t0 = t0.min(t);
                t1 = t1.max(t);
                r0 = r0.min(r);
                r1 = r1.max(r);
            }
            let (tc, thh, rc, rh) = ((t0 + t1) * 0.5, (t1 - t0) * 0.5, (r0 + r1) * 0.5, (r1 - r0) * 0.5);
            let m = Mat4::from_cols(
                Vec4::new(right.x / rh, top.x / thh, ray.x / CASC_Z_RANGE, 0.0),
                Vec4::new(right.y / rh, top.y / thh, ray.y / CASC_Z_RANGE, 0.0),
                Vec4::new(right.z / rh, top.z / thh, ray.z / CASC_Z_RANGE, 0.0),
                Vec4::new(-rc / rh, -tc / thh, (CASC_Z_NEAR - ray.dot(cam)) / CASC_Z_RANGE, 1.0),
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
            let (dx, dy) = (shift.first().copied().unwrap_or(-0.125) / (CASC_TILE * 2) as f32, shift.get(1).copied().unwrap_or(-0.375) / (CASC_TILE * 2) as f32);
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
            let cbias = if std::env::var("MCLA_CASC_BIASM").is_ok() { CASC_BIAS_M[i] } else { casc_const_bias(i, ray.x) };
            let pass = CascPass { light_vp: m.to_cols_array_2d(), bias: [slope * bk.first().copied().unwrap_or(1.0), cbias / CASC_Z_RANGE * bk.get(1).copied().unwrap_or(1.0), alpha, 0.0] };
            self.queue.write_buffer(&self.casc_pass_bufs[i], 0, bytemuck::bytes_of(&pass));
        }
        // (c82.w: recorded as top . camera + 13.4 .. 14.1 - what it is was not found; the scene's
        // program meets it only where no tile holds the point)
        u.top = [top.x, top.y, top.z, top.dot(cam) + 13.85];
        // (right.w: the CARS' materials keep the look-up of before 2026-10-15, half a texel further each way - the
        // original's cars are no receivers of this pass at all (no car draw among PS_ShadowBlend's), their sun term
        // is the car work's; the measured shift darkened the R8's sunlit side by 1 - 2 levels in the left view.
        // MCLA_CASC_CARSHIFT=0: cars read the map like everything else)
        let car_shift = if std::env::var("MCLA_CASC_SHIFT").is_ok() || std::env::var("MCLA_CASC_CARSHIFT").map_or(false, |v| v.trim() == "0") { 0.0 } else { 0.5 / (CASC_TILE * 2) as f32 };
        u.right = [right.x, right.y, right.z, car_shift];
        // (params.w = 1, MCLA_CASC_PXGRID=1: the taps' turn taken at the centres of a 1280 x 720 grid, as the
        // original's collector has it, whatever the window's size - a trial)
        u.params = [1.0, 82.0, 78.0, if std::env::var("MCLA_CASC_PXGRID").map_or(false, |v| v.trim() == "1") { 1.0 } else { 0.0 }];
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
        let half = FC_SIZE as f32 * 0.5;
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
        let view_proj = Mat4::from_cols(Vec4::new(1.0 / half, 0.0, 0.0, 0.0), Vec4::new(0.0, 0.0, -1.0 / depth, 0.0), Vec4::new(0.0, -1.0 / half, 0.0, 0.0), Vec4::new((fhx - cx) / half, (cz - fhz) / half, top / depth, 1.0));
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
                (false, Some((lx, lz))) => [cx - lx, cz - lz, fc_w, 0.0],
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
        let all = wgpu::Extent3d { width: FC_SIZE, height: FC_SIZE, depth_or_array_layers: 1 };
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
        let n = CASC_TILE * 2;
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
        let bpr = FC_SIZE * 4;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("fc dump"), size: (bpr * FC_SIZE) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            // (MCLA_FC_DUMP_WORK: the fresh map of the last frame, before its blur, instead)
            wgpu::TexelCopyTextureInfo { texture: if std::env::var("MCLA_FC_DUMP_WORK").is_ok() { &self.fc_work } else { &self.fc_tex }, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(FC_SIZE) } },
            wgpu::Extent3d { width: FC_SIZE, height: FC_SIZE, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(&FC_SIZE.to_le_bytes());
        out.extend_from_slice(&FC_SIZE.to_le_bytes());
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
    pub fn reflection_pass<'a>(&self, enc: &mut wgpu::CommandEncoder, view_proj: Mat4, cam: Vec3, plane: f32, sky: bool, models: impl Iterator<Item = &'a crate::world::GpuSector>) -> usize {
        let sky = sky && self.sky_hat.tex_bg.is_some();
        self.queue.write_buffer(&self.refl_buf, 0, bytemuck::bytes_of(&[plane, if sky { 1.0f32 } else { 0.0 }, 0.0, 0.0]));
        let frustum = Frustum::from_view_proj(view_proj);
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("road reflection"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.refl_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.refl_depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
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
        let third = self.clear_tex.clone();
        self.texture_bind_group3(diffuse, second, &third, k0, k1, 0)
    }

    /// Three-texture material (ground shaders: diffuse, decal atlas, grime).
    /// The sky dome's textures (see `skyhat::SkyHat::bind`).
    pub fn bind_sky_hat(&mut self, stars: &GpuTexture, galaxy: &GpuTexture, detail: &GpuTexture, noise1: &GpuTexture) {
        self.sky_hat.bind(&self.device, &self.tex_layout, &self.sampler, stars, galaxy, detail, noise1, &self.clear_tex);
        // (the reflection pass draws the dome too: the Perlin pass's target and the detail noise)
        self.refl_bg = make_refl_bg(&self.device, &self.refl_layout, &self.refl_buf, &self.sky_hat.buf, &self.sky_hat.perlin_view, &detail.view, &self.sampler);
    }

    pub fn texture_bind_group3(&mut self, diffuse: &Arc<GpuTexture>, second: &Arc<GpuTexture>, third: &Arc<GpuTexture>, k0: u32, k1: u32, k2: u32) -> Arc<wgpu::BindGroup> {
        let fourth = self.clear_tex.clone();
        self.texture_bind_group4(diffuse, second, third, &fourth, k0, k1, k2, 0)
    }

    /// Four-texture material (terrain: channel map and the three textures it mixes).
    #[allow(clippy::too_many_arguments)]
    pub fn texture_bind_group4(&mut self, diffuse: &Arc<GpuTexture>, second: &Arc<GpuTexture>, third: &Arc<GpuTexture>, fourth: &Arc<GpuTexture>, k0: u32, k1: u32, k2: u32, k3: u32) -> Arc<wgpu::BindGroup> {
        let k2 = k2 ^ k3.rotate_left(7);
        if let Some(b) = self.bind_cache.get(&(k0, k1, k2)) {
            return b.clone();
        }
        let bg = Arc::new(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&diffuse.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&second.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&third.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&fourth.view) },
            ],
        }));
        self.bind_cache.insert((k0, k1, k2), bg.clone());
        bg
    }
}

fn make_object_binding(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, u: &ObjectUniform, dmg: &wgpu::TextureView) -> ObjectBinding {
    let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("object"),
        contents: bytemuck::bytes_of(u),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let bg = object_bind_group(device, layout, &buf, dmg);
    ObjectBinding { buf, bg }
}

fn object_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, buf: &wgpu::Buffer, dmg: &wgpu::TextureView) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("object bg"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(dmg) }],
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
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&Default::default());
    GpuTexture { texture, view, width: w, height: h }
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
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&tex.name),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let (mut lw, mut lh) = (w, h);
    for level in 0..levels {
        let data = &tex.levels[level as usize];
        let (bytes, bpr, rows) = if direct {
            let bx = (lw + 3) / 4;
            let by = (lh + 3) / 4;
            (data.clone(), bx * bpb as u32, by)
        } else {
            let rgba = mcla_formats::rage::xenos::decode_to_rgba8(tex.format, data, lw as usize, lh as usize);
            (rgba, lw * 4, lh)
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
    GpuTexture { texture, view, width: w, height: h }
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
            let l = p.truncate().length();
            if l > 0.0 {
                *p /= l;
            }
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
    /// True when every point lies outside the same plane.
    pub fn points_outside(&self, pts: &[Vec3]) -> bool {
        self.planes.iter().any(|p| pts.iter().all(|v| p.truncate().dot(*v) + p.w < 0.0))
    }
}
