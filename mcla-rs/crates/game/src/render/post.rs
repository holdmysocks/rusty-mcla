//! Post-processing chain, laid out as the original's (rage_postfx; see LIGHTING_WORK.md): the
//! scene is drawn into an HDR target; half and quarter size copies feed the log-average
//! luminance (128 -> 64 -> 16 -> 4 -> 1, adapted over time) and the bloom (bright pass and two
//! Gauss passes at quarter size); the tone map writes the display. The composite also carries
//! this port's speed-dependent radial blur.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// The scene's colour target: linear light, values above 1 kept.
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const LUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The luminance history and the adapted luminance: 32_FLOAT as the original's (the history's gate compares
/// values a few 1e-5 apart)
const ADAPT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// The history's size (the original's 4 x 4 target)
const HIST: u32 = 4;

/// What the tone map is tuned by: the weather's keys at the hour (`sky::Tod`).
#[derive(Clone, Copy, Debug)]
pub struct Tune {
    /// `m_Luminance`.x
    pub middle_grey: f32,
    /// `m_Luminance`.y squared
    pub white: f32,
    /// `m_BrightPass`: threshold, offset, white squared
    pub bright: [f32; 3],
    /// lower and upper limit of the adapted luminance
    pub limits: [f32; 2],
    /// adaption rate (Lambda = ln 2 / `m_LightAdaption`.x, F-31612)
    pub lambda: f32,
    /// the luminance history's slots (`numofframesinlumhistory` of the post-processing settings)
    pub lum_frames: u32,
    pub saturation: f32,
    pub color_correct: [f32; 3],
    /// seconds since the last frame
    pub dt: f32,
    /// radial blur amount and its centre in uv
    pub radial: f32,
    pub center: [f32; 2],
    /// tone map only: no bloom, no blur
    pub plain: bool,
    /// test hook: this adapted luminance instead of the measured one
    pub force_lum: Option<f32>,
    /// `m_DoFParams` of the weather's keys (PSStreakMotionBlur's DofParams): the distance the far
    /// blur begins at (m), its ramp (m), its scale; scale 0: no depth of field
    pub dof: [f32; 4],
    /// the scene's inverse view-projection and the camera's place: the depth of field's distance
    pub inv_view_proj: [[f32; 4]; 4],
    pub cam_pos: [f32; 3],
    /// PSStreakMotionBlur's previous-frame matrices (Mc4MotionBlurVehicleMtxs, render/mb.rs): world -> the
    /// previous frame's clip space, per stencil id / 4
    pub mb_mtx: [[[f32; 4]; 4]; super::mb::RECORDS],
    /// OPTIONS > DISPLAY toggles (ON = the original's): the bloom's share of the tone map; the depth of field's
    /// weight; the motion part of the streak pass (M = 0 when off)
    pub bloom: bool,
    pub dof_on: bool,
    pub motion_blur: bool,
    /// the police camera (post +340, set by Graphics_SetCopCamera F-29933 and the cine events bw_toggle_on / _off
    /// F-86832, setter F-77562): the bright pass takes the settings file's second set and the final pass F-34349 draws
    /// ToneMapFogPoliceCam (post.wgsl `fs_police`)
    pub police: bool,
    /// the streak pass' blurEffectTintFade / blurEffectDesatScale (post +512 / +516, uploaded by F-12598): F-55690 writes
    /// them from the player car's frame update F-16712 - TintFade = 1 + 3 x (Agro running), DesatScale = sat(grip running
    /// + the car's nitrous ease); they keep the last values written (1 / 0 in every recorded plain frame)
    pub blur_fx: [f32; 2],
}

/// `numofframesinlumhistory` of the post-processing settings (`tune/lighting/0xa0829ade`, read by the settings loader F-15434):
/// the slots of the luminance history the adapt pass compares
pub static LUM_FRAMES: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

/// The police camera's keys of the post-processing settings (`tune/lighting/0xa0829ade`, F-15434): the second bright-pass
/// set (threshold, offset, middle grey, white - squared by F-31612 as the first set's), saturationpolicecam,
/// grainintensity, interlaceintensity, timeframe (the timer's wrap), interference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoliceSet {
    pub threshold: f32,
    pub offset: f32,
    pub middle_grey: f32,
    pub white: f32,
    pub saturation: f32,
    pub grain: f32,
    pub interlace: f32,
    pub timeframe: f32,
    pub interference: f32,
}

pub static POLICE: std::sync::OnceLock<PoliceSet> = std::sync::OnceLock::new();

/// A float key of the settings file (`<key content="ascii">value</key>`).
fn key(t: &str, k: &str) -> Option<f32> {
    let i = t.find(&format!("<{k} "))?;
    let r = &t[i..];
    r[r.find('>')? + 1..r.find("</")?].trim().parse::<f32>().ok()
}

/// The police set of a settings file's text (the shipped file's values where a key is missing).
pub fn police_set(t: &str) -> PoliceSet {
    PoliceSet {
        threshold: key(t, "brightpassthresholdtwo").unwrap_or(1.5),
        offset: key(t, "brightpassoffsettwo").unwrap_or(2.0),
        middle_grey: key(t, "middlegraytwo").unwrap_or(0.18),
        white: key(t, "brightpasswhitetwo").unwrap_or(1.0),
        saturation: key(t, "saturationpolicecam").unwrap_or(0.427),
        grain: key(t, "grainintensity").unwrap_or(0.031),
        interlace: key(t, "interlaceintensity").unwrap_or(7.7),
        timeframe: key(t, "timeframe").unwrap_or(88.4),
        interference: key(t, "interference").unwrap_or(0.0015),
    }
}

pub fn init(data_dir: &str) {
    let t = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/lighting/0xa0829ade")).unwrap_or_default();
    let n = t.find("<numofframesinlumhistory").and_then(|i| {
        let r = &t[i..];
        r[r.find('>')? + 1..r.find("</")?].trim().parse::<u32>().ok()
    });
    let _ = LUM_FRAMES.set(n.unwrap_or(5));
    let _ = POLICE.set(police_set(&t));
}

/// The police camera's timer (F-34349: its global timer += 10 x the game frame time, back to 0 once it reaches timeframe; it
/// runs only while the camera is on and is not reset when it goes off).
pub fn police_timer(t: f32, dt: f32, timeframe: f32) -> f32 {
    let n = dt.mul_add(10.0, t);
    if n >= timeframe {
        0.0
    } else {
        n
    }
}

impl Default for Tune {
    fn default() -> Self {
        // noon in the Nice weather
        Tune { middle_grey: 0.25, white: 0.21646, bright: [1.6203, 4.6213, 0.032782], limits: [0.15, 0.2], lambda: 3.7266, lum_frames: *LUM_FRAMES.get().unwrap_or(&5), saturation: 0.9, color_correct: [0.5862, 0.53442, 0.43101], dt: 1.0 / 60.0, radial: 0.0, center: [0.5, 0.5], plain: false, force_lum: None, dof: [0.0; 4], inv_view_proj: [[0.0; 4]; 4], cam_pos: [0.0; 3], mb_mtx: [[[0.0; 4]; 4]; super::mb::RECORDS], bloom: true, dof_on: true, motion_blur: true, police: false, blur_fx: [1.0, 0.0] }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Params {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
    e: [f32; 4],
    /// tone map: x = 1: no bloom (OPTIONS > DISPLAY > BLOOM off)
    f: [f32; 4],
    /// the police camera: ElapsedTime, intensityGrain, intensityInterlace, interference
    g: [f32; 4],
    /// the police camera: deSat, Contrast, Gamma
    h: [f32; 4],
}

/// What the depth of field's pass is drawn with (post.wgsl `DofU`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct DofU {
    inv_view_proj: [[f32; 4]; 4],
    cam: [f32; 4],
    dof: [f32; 4],
    /// xy = one pixel of the original's 1280 x 720 frame in uv (half a texel of the buffer); z = 1: the motion part
    /// on; w = 1: the depth of field's weight on
    px: [f32; 4],
    /// x = blurEffectTintFade, y = blurEffectDesatScale (`Tune::blur_fx`)
    fx: [f32; 4],
    mtx: [[[f32; 4]; 4]; super::mb::RECORDS],
}

/// The depth of field's buffer. The original's is 640 x 360 beside a 1280 x 720 frame: 360 rows at
/// any window size, like the bloom's 180.
fn dof_size(w: u32, h: u32) -> (u32, u32) {
    if h < 360 {
        return ((w / 2).max(1), (h / 2).max(1));
    }
    (((w as f32) * 360.0 / (h as f32)).round() as u32, 360)
}

/// MCLA_DEBUG_VIEW 500 .. 519 (the depth of field's): 500 off, 501 the weight, 502 the buffer
/// everywhere, 503 its blurred part through the 7e3 format once more, 504 the scene through it
/// too, 505 the taps as they fall without the half-pixel shift of the quads' coordinates, 506 the
/// sky takes part, 507 without the one sharp tap, 508 the tone map's tap on the pixel's centre.
fn dof_view() -> u32 {
    static V: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_DEBUG_VIEW").ok().and_then(|v| v.trim().parse().ok()).filter(|v| (500..520).contains(v)).unwrap_or(0))
}

/// MCLA_DOF=0 (or view 500): the tone map reads the scene alone, as before.
fn dof_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MCLA_DOF").map_or(true, |v| v.trim() != "0") && dof_view() != 500)
}

struct Target {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    w: u32,
    h: u32,
}

fn target(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, label: &str) -> Target {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    Target { view: t.create_view(&Default::default()), tex: t, w: w.max(1), h: h.max(1) }
}

/// The bloom's two targets. The original always draws 1280 x 720 and blooms at 320 x 180: its
/// nine Gauss taps reach +-4 texels = +-16 of 720 rows. A quarter of a larger window made the
/// halo narrower on screen (1600 x 900: 400 x 225, +-12.8 of 720 rows - measured on the car's
/// side: the halo's tails 0.006 against the original's 0.013 at 20 px). So: 180 rows at any
/// window size; the bright pass reads the quarter-size scene with one bilinear tap as before.
/// MCLA_BLOOM_WIN=1: a quarter of the window, as before.
fn bloom_size(w: u32, h: u32) -> (u32, u32) {
    if std::env::var("MCLA_BLOOM_WIN").is_ok() || h < 180 {
        return (w / 4, h / 4);
    }
    (((w as f32) * 180.0 / (h as f32)).round() as u32, 180)
}

const STEPS: usize = 12;

pub struct Post {
    scene: Target,
    half: Target,
    quarter: Target,
    bloom_a: Target,
    bloom_b: Target,
    /// the lamps' glow pass (halo.rs): display values the tone map adds to its encoded result
    halo: Target,
    /// the depth of field's buffer (PSStreakMotionBlur's target): rgb = the scene blurred, a = how far
    /// beyond `m_DoFParams`.x the pixel lies; the tone map lerps the scene towards it
    dof: Target,
    /// PSInitialScaleBuffer's 640 x 360 copy as the streak pass reads it (texel m = the 2 x 2 box of scene pixels
    /// 2m+1, 2m+2: one bilinear tap a scene pixel down-right of its centre)
    copy: Target,
    pipe_copy: wgpu::RenderPipeline,
    dof_layout: wgpu::BindGroupLayout,
    dof_buf: wgpu::Buffer,
    pipe_dof: wgpu::RenderPipeline,
    /// log luminance: 128, 64, 16, 4
    lum: [Target; 4],
    /// the adapted luminance of the frame before and of this one (they swap)
    adapted: [Target; 2],
    /// the luminance history (4 x 4): one frame's average a slot, F-31612's ring (`ring` = its count, column,
    /// row: 1, 0, 0 in the image)
    hist: Target,
    ring: std::sync::Mutex<(u32, u32, u32)>,
    pipe_lum_slot: wgpu::RenderPipeline,
    // (atomics: the frame's recording borrows the Gpu from several threads - docs/PERFORMANCE.md 2026-10-23 (4))
    flip: std::sync::atomic::AtomicUsize,
    first: std::sync::atomic::AtomicBool,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipe_scale: wgpu::RenderPipeline,
    pipe_lum_log: wgpu::RenderPipeline,
    pipe_lum_down: wgpu::RenderPipeline,
    pipe_lum_adapt: wgpu::RenderPipeline,
    pipe_bright: wgpu::RenderPipeline,
    pipe_gauss: wgpu::RenderPipeline,
    pipe_tonemap: wgpu::RenderPipeline,
    /// ToneMapFogPoliceCam (post.wgsl `fs_police`), its noise (textures/random3d.dds) and sampler, and its timer
    pipe_police: wgpu::RenderPipeline,
    noise: wgpu::TextureView,
    noise_sampler: wgpu::Sampler,
    police_t: std::sync::Mutex<f32>,
    bufs: Vec<wgpu::Buffer>,
    /// [adapted target of this frame][step]
    groups: [Vec<wgpu::BindGroup>; 2],
    /// RENDER SCALE (hooks::render_scale): the display's format; the tone map's target at the scene's size when the
    /// window is another size (None at scale 1), its group and the box filter into the window
    format: wgpu::TextureFormat,
    out_size: (u32, u32),
    display: Option<(Target, wgpu::BindGroup)>,
    down_layout: wgpu::BindGroupLayout,
    pipe_down: wgpu::RenderPipeline,
}

/// RENDER SCALE's filter from the scene's size to the window's: each window pixel the area-weighted mean of the
/// scene pixels it covers (a box filter; 2.0 = the plain 2 x 2 average), read from the sRGB target so the mean is
/// taken in linear light; below scale 1 one bilinear tap.
const DOWN_WGSL: &str = r#"
struct D { size: vec4<f32> };
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<uniform> d: D;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let k = d.size.xy / d.size.zw;
    if (k.x <= 1.0 || k.y <= 1.0) {
        return vec4<f32>(textureSampleLevel(t, smp, pos.xy / d.size.zw, 0.0).rgb, 1.0);
    }
    let o = floor(pos.xy);
    let lo = o * k;
    let hi = lo + k;
    let i0 = vec2<i32>(floor(lo));
    let i1 = vec2<i32>(ceil(hi)) - vec2<i32>(1);
    let top = vec2<i32>(d.size.xy) - vec2<i32>(1);
    var acc = vec3<f32>(0.0);
    var ws = 0.0;
    for (var y = i0.y; y <= i1.y; y = y + 1) {
        let wy = min(hi.y, f32(y + 1)) - max(lo.y, f32(y));
        for (var x = i0.x; x <= i1.x; x = x + 1) {
            let w = (min(hi.x, f32(x + 1)) - max(lo.x, f32(x))) * wy;
            if (w > 0.0) {
                acc = acc + w * textureLoad(t, clamp(vec2<i32>(x, y), vec2<i32>(0), top), 0).rgb;
                ws = ws + w;
            }
        }
    }
    return vec4<f32>(acc / max(ws, 1e-6), 1.0);
}
"#;

fn noise_placeholder(device: &wgpu::Device) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("noise placeholder"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl Post {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32) -> Post {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("post"), source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()) });
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post layout"),
            entries: &[
                tex_entry(0),
                // (the adapted luminance, R32Float: not filterable, read with textureLoad)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(4),
                tex_entry(5),
                tex_entry(6),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                tex_entry(8),
                wgpu::BindGroupLayoutEntry { binding: 9, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post pl"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let make = |entry: &str, fmt: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_post"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState { format: fmt, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: crate::render::pcache::get(),
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // the depth of field's pass: its own bindings (the scene, the scene's depth)
        let dof_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dof layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 10, ..tex_entry(0) },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 12, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // the stencil copy (s / 255, bilinear - the original's 8888 copy) and the 640 x 360 copy
                wgpu::BindGroupLayoutEntry { binding: 14, ..tex_entry(0) },
                wgpu::BindGroupLayoutEntry { binding: 15, ..tex_entry(0) },
            ],
        });
        let dof_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("dof pl"), bind_group_layouts: &[&dof_layout], push_constant_ranges: &[] });
        let pipe_dof = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fs_dof"),
            layout: Some(&dof_pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_post"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_dof"),
                targets: &[Some(wgpu::ColorTargetState { format: SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        let pipe_copy = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fs_copy"),
            layout: Some(&dof_pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_post"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_copy"),
                targets: &[Some(wgpu::ColorTargetState { format: SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        let down_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render scale down"),
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipe_down = {
            let m = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("render scale down"), source: wgpu::ShaderSource::Wgsl(DOWN_WGSL.into()) });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&down_layout], push_constant_ranges: &[] });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("render scale down"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &m, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: &m, entry_point: Some("fs"), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: crate::render::pcache::get(),
            })
        };
        let dof_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("dof params"), contents: bytemuck::bytes_of(&DofU::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let bufs = (0..STEPS).map(|_| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("post params"), contents: bytemuck::bytes_of(&Params::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST })).collect();
        let lum = |n: u32, label: &str| target(device, n, n, LUM_FORMAT, label);
        let mut p = Post {
            scene: target(device, w, h, SCENE_FORMAT, "scene"),
            half: target(device, w / 2, h / 2, SCENE_FORMAT, "scene half"),
            quarter: target(device, w / 4, h / 4, SCENE_FORMAT, "scene quarter"),
            bloom_a: target(device, bloom_size(w, h).0, bloom_size(w, h).1, SCENE_FORMAT, "bloom a"),
            bloom_b: target(device, bloom_size(w, h).0, bloom_size(w, h).1, SCENE_FORMAT, "bloom b"),
            halo: target(device, w, h, SCENE_FORMAT, "halo"),
            dof: target(device, dof_size(w, h).0, dof_size(w, h).1, SCENE_FORMAT, "dof"),
            copy: target(device, dof_size(w, h).0, dof_size(w, h).1, SCENE_FORMAT, "dof copy"),
            pipe_copy,
            dof_layout,
            dof_buf,
            pipe_dof,
            lum: [lum(128, "lum 128"), lum(64, "lum 64"), lum(16, "lum 16"), lum(4, "lum 4")],
            adapted: [target(device, 1, 1, ADAPT_FORMAT, "adapted a"), target(device, 1, 1, ADAPT_FORMAT, "adapted b")],
            hist: target(device, HIST, HIST, ADAPT_FORMAT, "lum history"),
            ring: std::sync::Mutex::new((1, 0, 0)),
            pipe_lum_slot: make("fs_lum_slot", ADAPT_FORMAT),
            flip: std::sync::atomic::AtomicUsize::new(0),
            first: std::sync::atomic::AtomicBool::new(true),
            pipe_scale: make("fs_scale", SCENE_FORMAT),
            pipe_lum_log: make("fs_lum_log", LUM_FORMAT),
            pipe_lum_down: make("fs_lum_down", LUM_FORMAT),
            pipe_lum_adapt: make("fs_lum_adapt", ADAPT_FORMAT),
            pipe_bright: make("fs_bright", SCENE_FORMAT),
            pipe_gauss: make("fs_gauss", SCENE_FORMAT),
            pipe_tonemap: make("fs_tonemap", format),
            pipe_police: make("fs_police", format),
            noise: noise_placeholder(device),
            // (NoiseSampler's states in the effect: address 0 / 1 = 0 (wrap), filters 4 / 5 / 6 = 1 (linear))
            noise_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("police noise"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            police_t: std::sync::Mutex::new(0.0),
            layout,
            sampler,
            bufs,
            groups: [Vec::new(), Vec::new()],
            format,
            out_size: (w.max(1), h.max(1)),
            display: None,
            down_layout,
            pipe_down,
        };
        p.rebuild_groups(device);
        p
    }

    /// The police camera's noise: `__noisemap`, which F-22695 registers as the post's textures/random3d.dds (64 x 64 DXT1).
    pub fn set_noise(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data_dir: &str) {
        let p = std::path::Path::new(data_dir).join("textures/random3d.dds");
        let Some(t) = std::fs::read(&p).ok().and_then(|d| mcla_formats::dds::parse_dds("random3d.dds", &d).ok()).filter(|t| t.width > 0 && t.height > 0 && !t.levels.is_empty()) else {
            log::warn!("random3d.dds missing: the police camera has no noise");
            return;
        };
        let tex = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("random3d"),
                size: wgpu::Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &t.to_rgba8(),
        );
        self.noise = tex.create_view(&Default::default());
        self.rebuild_groups(device);
    }

    fn rebuild_groups(&mut self, device: &wgpu::Device) {
        let mk7 = |a: &wgpu::TextureView, b: &wgpu::TextureView, c: &wgpu::TextureView, buf: &wgpu::Buffer, h: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post bg"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(a) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(b) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(c) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&self.halo.view) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.dof.view) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(h) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&self.noise) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&self.noise_sampler) },
                ],
            })
        };
        let mk = |a: &wgpu::TextureView, b: &wgpu::TextureView, c: &wgpu::TextureView, buf: &wgpu::Buffer| mk7(a, b, c, buf, &self.hist.view);
        let mut sets = Vec::new();
        for now in 0..2 {
            let (new, old) = (&self.adapted[now].view, &self.adapted[1 - now].view);
            let s = &self.scene.view;
            sets.push(vec![
                // 0: scene -> half
                mk(s, s, s, &self.bufs[0]),
                // 1: half -> quarter
                mk(&self.half.view, s, s, &self.bufs[1]),
                // 2: quarter -> log luminance 128
                mk(&self.quarter.view, s, s, &self.bufs[2]),
                // 3..5: 128 -> 64 -> 16 -> 4
                mk(&self.lum[0].view, s, s, &self.bufs[3]),
                mk(&self.lum[1].view, s, s, &self.bufs[4]),
                mk(&self.lum[2].view, s, s, &self.bufs[5]),
                // 6: 4 -> the adapted luminance (with the one of the frame before)
                mk(&self.lum[3].view, old, s, &self.bufs[6]),
                // 7: bright pass, quarter -> bloom a
                mk(&self.quarter.view, new, s, &self.bufs[7]),
                // 8, 9: Gauss, a -> b -> a
                mk(&self.bloom_a.view, s, s, &self.bufs[8]),
                mk(&self.bloom_b.view, s, s, &self.bufs[9]),
                // 10: tone map, scene + adapted luminance + bloom -> the display
                mk(s, new, &self.bloom_a.view, &self.bufs[10]),
                // 11: the frame's average into the history's slot (binding 7 must not be the target)
                mk7(&self.lum[3].view, old, s, &self.bufs[11], new),
            ]);
        }
        let second = sets.pop().unwrap();
        let first = sets.pop().unwrap();
        self.groups = [first, second];
    }

    pub fn resize(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        self.scene = target(device, w, h, SCENE_FORMAT, "scene");
        self.half = target(device, w / 2, h / 2, SCENE_FORMAT, "scene half");
        self.quarter = target(device, w / 4, h / 4, SCENE_FORMAT, "scene quarter");
        self.bloom_a = target(device, bloom_size(w, h).0, bloom_size(w, h).1, SCENE_FORMAT, "bloom a");
        self.bloom_b = target(device, bloom_size(w, h).0, bloom_size(w, h).1, SCENE_FORMAT, "bloom b");
        self.halo = target(device, w, h, SCENE_FORMAT, "halo");
        self.dof = target(device, dof_size(w, h).0, dof_size(w, h).1, SCENE_FORMAT, "dof");
        self.copy = target(device, dof_size(w, h).0, dof_size(w, h).1, SCENE_FORMAT, "dof copy");
        self.rebuild_groups(device);
        self.refresh_display(device);
    }

    /// The window's size (RENDER SCALE: the chain runs at the scene's, `resize`; the tone map's result is filtered
    /// down to this).
    pub fn set_out_size(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        self.out_size = (w.max(1), h.max(1));
        self.refresh_display(device);
    }

    fn refresh_display(&mut self, device: &wgpu::Device) {
        let (ow, oh) = self.out_size;
        if (ow, oh) == (self.scene.w, self.scene.h) {
            self.display = None;
            return;
        }
        let t = target(device, self.scene.w, self.scene.h, self.format, "display (render scale)");
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("render scale down"),
            contents: bytemuck::cast_slice(&[self.scene.w as f32, self.scene.h as f32, ow as f32, oh as f32]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render scale down"),
            layout: &self.down_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() },
            ],
        });
        self.display = Some((t, bg));
    }

    /// Diagnostics (MCLA_POST_DUMP=<file>): the bright pass's input (the quarter-size scene) and
    /// the finished bloom, raw - "MPS2", then per picture width, height and RGBA half floats.
    pub fn dump(&self, device: &wgpu::Device, queue: &wgpu::Queue, path: &str) {
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"MPS2");
        for t in [&self.quarter, &self.bloom_a] {
            out.extend_from_slice(&t.w.to_le_bytes());
            out.extend_from_slice(&t.h.to_le_bytes());
            let bpr = (t.w * 8 + 255) / 256 * 256;
            let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("post dump"), size: (bpr * t.h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &t.tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(t.h) } },
                wgpu::Extent3d { width: t.w, height: t.h, depth_or_array_layers: 1 },
            );
            queue.submit([enc.finish()]);
            let slice = buf.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            device.poll(wgpu::Maintain::Wait);
            if let Ok(Ok(())) = rx.recv() {
                let data = slice.get_mapped_range();
                for y in 0..t.h {
                    out.extend_from_slice(&data[(y * bpr) as usize..(y * bpr + t.w * 8) as usize]);
                }
            }
            buf.unmap();
        }
        if let Err(e) = std::fs::write(path, &out) {
            log::warn!("post dump {}: {}", path, e);
        }
    }

    /// The quarter-size copy of the scene (PSScaleBuffer's output: after fog and glows, before
    /// bloom and the tone map). Until this frame's chain runs it holds the frame before - what
    /// the ground sheet shows (ground_sheet.rs).
    pub fn quarter_view(&self) -> &wgpu::TextureView {
        &self.quarter.view
    }

    /// The picture of the lamps' glow pass (halo.rs draws it every frame, the tone map adds it).
    pub fn halo_view(&self) -> &wgpu::TextureView {
        &self.halo.view
    }

    /// The off-screen colour target the scene is drawn into.
    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene.view
    }

    /// Run the chain and write the result to `out`. `depth`: the scene's depth buffer (the depth of
    /// field's distance).
    pub fn run(&self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView, depth: &wgpu::TextureView, stencil: &wgpu::TextureView, t: &Tune) {
        let now = self.flip.load(std::sync::atomic::Ordering::Relaxed);
        self.flip.store(1 - now, std::sync::atomic::Ordering::Relaxed);
        let first = self.first.swap(false, std::sync::atomic::Ordering::Relaxed);
        let aspect = self.scene.w as f32 / self.scene.h as f32;
        let texel = |t: &Target| [1.0 / t.w as f32, 1.0 / t.h as f32, 0.0, 0.0];
        // The depth of field (PSStreakMotionBlur + PSToneMap instr. 6 - 9, LIGHTING_WORK.md): not under
        // --no-post, not without the weather's key.
        let dof_w = t.dof[2] > 0.0 && t.dof_on && dof_enabled();
        // (the streak pass runs for either part; with neither the tone map reads the scene alone)
        let mb_on = t.motion_blur && !t.plain && dof_enabled();
        // (the police program reads no depth of field buffer)
        let dof_on = (dof_w || mb_on) && !t.plain && !t.police;
        // the police camera: F-31612 takes the second bright-pass set (the tone map's BrightPassValues too), F-34349 steps
        // the timer
        let pset = POLICE.get().copied().unwrap_or_else(|| police_set(""));
        let pt = {
            let mut g = self.police_t.lock().unwrap();
            if t.police {
                *g = police_timer(*g, t.dt, pset.timeframe);
            }
            *g
        };
        let bpv = if t.police { [pset.middle_grey, pset.threshold, pset.offset, pset.white * pset.white] } else { [t.middle_grey, t.bright[0], t.bright[1], t.bright[2]] };
        let grade = Params {
            a: bpv,
            b: [t.limits[0], t.limits[1], t.white, t.saturation],
            c: [t.color_correct[0], t.color_correct[1], t.color_correct[2], t.radial],
            d: [t.center[0], t.center[1], aspect, if let Some(l) = t.force_lum { 2.0 + l.max(0.0) } else if t.plain { 1.0 } else { 0.0 }],
            e: [if dof_on { 1.0 } else { 0.0 }, dof_view() as f32, 0.5 / self.dof.w as f32, 0.5 / self.dof.h as f32],
            f: [if t.bloom { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0],
            // (Contrast 1 - 1.061 only with a debug byte; Gamma the tone map's 0.45455, docs/LIGHTING.md)
            g: [pt, pset.grain, pset.interlace, pset.interference],
            h: [pset.saturation, 1.0, 0.45455, 0.0],
        };
        let only = |a: [f32; 4]| Params { a, ..Default::default() };
        let ps = [
            only([0.0; 4]),
            only([0.0; 4]),
            only(texel(&self.quarter)),
            only(texel(&self.lum[0])),
            only(texel(&self.lum[1])),
            only(texel(&self.lum[2])),
            only([1.0 - (-t.dt.clamp(0.0, 0.5) * t.lambda).exp(), if first { 1.0 } else { 0.0 }, t.lum_frames.min(HIST * HIST) as f32, 0.0]),
            grade,
            only([1.0 / self.bloom_a.w as f32, 0.0, 0.0, 0.0]),
            only([0.0, 1.0 / self.bloom_b.h as f32, 0.0, 0.0]),
            grade,
        ];
        // (the 4 -> 1 step reads its 16 taps from the 4x4 target, a texel apart)
        let mut ps = ps;
        ps[6].b = texel(&self.lum[3]);
        for (b, p) in self.bufs.iter().zip(ps.iter()) {
            queue.write_buffer(b, 0, bytemuck::bytes_of(p));
        }
        queue.write_buffer(&self.bufs[11], 0, bytemuck::bytes_of(&only(texel(&self.lum[3]))));
        // F-31612's ring: after Frames frames back to slot (0, 0), else the next slot in raster order
        let (sx, sy) = {
            let mut r = self.ring.lock().unwrap();
            let (n, x, y) = *r;
            *r = if n >= t.lum_frames {
                (1, 0, 0)
            } else if x as f64 >= HIST as f64 - 1.0 {
                (n + 1, 0, if y + 1 >= HIST { 0 } else { y + 1 })
            } else {
                (n + 1, x + 1, y)
            };
            (r.1, r.2)
        };
        let steps: [(&wgpu::TextureView, &wgpu::RenderPipeline, usize); 11] = [
            (&self.half.view, &self.pipe_scale, 0),
            (&self.quarter.view, &self.pipe_scale, 1),
            (&self.lum[0].view, &self.pipe_lum_log, 2),
            (&self.lum[1].view, &self.pipe_lum_down, 3),
            (&self.lum[2].view, &self.pipe_lum_down, 4),
            (&self.lum[3].view, &self.pipe_lum_down, 5),
            (&self.adapted[now].view, &self.pipe_lum_adapt, 6),
            (&self.bloom_a.view, &self.pipe_bright, 7),
            (&self.bloom_b.view, &self.pipe_gauss, 8),
            (&self.bloom_a.view, &self.pipe_gauss, 9),
            (self.display.as_ref().map_or(out, |d| &d.0.view), if t.police { &self.pipe_police } else { &self.pipe_tonemap }, 10),
        ];
        if dof_on {
            let u = DofU {
                inv_view_proj: t.inv_view_proj,
                cam: [t.cam_pos[0], t.cam_pos[1], t.cam_pos[2], dof_view() as f32],
                dof: t.dof,
                // one pixel of the original's 1280 x 720 frame in uv (half a texel of the buffer)
                px: [0.5 / self.dof.w as f32, 0.5 / self.dof.h as f32, if mb_on { 1.0 } else { 0.0 }, if dof_w { 1.0 } else { 0.0 }],
                fx: [t.blur_fx[0], t.blur_fx[1], 0.0, 0.0],
                mtx: t.mb_mtx,
            };
            queue.write_buffer(&self.dof_buf, 0, bytemuck::bytes_of(&u));
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dof bg"),
                layout: &self.dof_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&self.scene.view) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(depth) },
                    wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 13, resource: self.dof_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(stencil) },
                    wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(&self.copy.view) },
                ],
            });
            // (the copy reads the scene only; binding 15 is the copy itself, so it gets a group of its own)
            let bg_copy = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dof copy bg"),
                layout: &self.dof_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&self.scene.view) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(depth) },
                    wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 13, resource: self.dof_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(stencil) },
                    wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(&self.dof.view) },
                ],
            });
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("dof copy"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.copy.view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&self.pipe_copy);
                pass.set_bind_group(0, &bg_copy, &[]);
                pass.draw(0..3, 0..1);
            }
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("dof"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.dof.view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipe_dof);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }
        for (view, pipe, g) in steps {
            if g == 6 {
                // PSReSampleAverageLuminanceExp into the one texel of the history (the other slots kept)
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("lum history slot"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.hist.view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_viewport(sx as f32, sy as f32, 1.0, 1.0, 0.0, 1.0);
                pass.set_pipeline(&self.pipe_lum_slot);
                pass.set_bind_group(0, &self.groups[now][11], &[]);
                pass.draw(0..3, 0..1);
            }
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &self.groups[now][g], &[]);
            pass.draw(0..3, 0..1);
        }
        // (RENDER SCALE: the tone map's result filtered down to the window)
        if let Some((_, bg)) = self.display.as_ref() {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render scale down"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: out, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipe_down);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// The post's depth of field vector one frame on toward `target` (F-48241): x, y and z each by F-90102 at the rate
/// |target - now| / 0.1 (the transition time 0x827E5744, at least 1e-9) times the frame time `dt`; w is not
/// approached (the port takes the target's). The rate's reciprocal is the original's refined estimate, here 1 / 0.1.
pub fn dof_approach(now: &mut [f32; 4], target: [f32; 4], dt: f32) {
    for i in 0..3 {
        let rate = (target[i] - now[i]).abs() * (1.0 / 0.1f32);
        crate::caraudio::approach_fma(&mut now[i], target[i], rate, dt);
    }
    now[3] = target[3];
}

#[cfg(test)]
mod dof_approach_tests {
    use super::dof_approach;

    /// The original's landing after a GPS flight (work/ui2/gf/rec2.jsonl, post +240 at 30 Hz): from the jump's
    /// 10000 / 3000 / 50 toward SetGameDof's 500 / 1000 / 0.75 for one frame, then the hour's 517.24 / 3000 / 50.
    #[test]
    fn dof_approaches_as_the_recording() {
        let dt = 1.0f32 / 30.0;
        let mut v = [10000.0, 3000.0, 50.0, 1.0];
        dof_approach(&mut v, [500.0, 1000.0, 0.75, 1.0], dt);
        let rec = [[6833.3, 2333.3, 33.6], [4728.0, 2555.6, 39.1], [3324.4, 2703.7, 42.7], [2388.7, 2802.5, 45.1], [1764.9, 2868.3, 46.8]];
        let hour = [517.241_33, 3000.0, 50.0, 1.0];
        for (n, r) in rec.iter().enumerate() {
            if n > 0 {
                dof_approach(&mut v, hour, dt);
            }
            for i in 0..3 {
                assert!((v[i] - r[i]).abs() < 0.06, "frame {n} [{i}]: {} vs {}", v[i], r[i]);
            }
        }
    }
}

#[cfg(test)]
mod police_tests {
    use super::*;

    #[test]
    fn police_set_reads_the_settings_keys() {
        let t = "<brightpassthresholdtwo content=\"ascii\">1.500000</brightpassthresholdtwo>\n<brightpassoffsettwo content=\"ascii\">2.000000</brightpassoffsettwo>\n<brightpasswhitetwo content=\"ascii\">1.000000</brightpasswhitetwo>\n<middlegraytwo content=\"ascii\">0.180000</middlegraytwo>\n<saturationpolicecam content=\"ascii\">0.427000</saturationpolicecam>\n<grainintensity content=\"ascii\">0.031000</grainintensity>\n<interlaceintensity content=\"ascii\">7.700000</interlaceintensity>\n<timeframe content=\"ascii\">88.400002</timeframe>\n<interference content=\"ascii\">0.001500</interference>";
        let p = police_set(t);
        assert_eq!((p.threshold, p.offset, p.middle_grey, p.white), (1.5, 2.0, 0.18, 1.0));
        assert_eq!((p.saturation, p.grain, p.interlace, p.timeframe, p.interference), (0.427, 0.031, 7.7, 88.400002, 0.0015));
    }

    #[test]
    fn police_timer_wraps_to_zero() {
        // (F-34349: fmadds dt x 10 + t; at or past timeframe it is set to 0.0, not wrapped by the excess)
        assert_eq!(police_timer(0.0, 0.5, 88.4), 5.0);
        assert_eq!(police_timer(88.0, 0.05, 88.4), 0.0);
        assert_eq!(police_timer(88.0, 0.02, 88.4), 88.2);
    }
}
