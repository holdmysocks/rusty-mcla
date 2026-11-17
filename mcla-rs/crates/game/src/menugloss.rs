//! The UI meshes of the `Gloss` effect (ui/0x35719848) drawn with the original's own programs: the pause menu's glass
//! frame (`pause_screen`) and the pane that carries the menu's picture.
//!
//! The original (recorded with work/recomp-light, pause menu over Hollywood, draws 5109 / 5110 of `pause_main`): the
//! frame with VS_TransformSkinned + PS_TexturedGloss, then the pane with VS_TransformAndTextureAlpha + PS_TexturedGloss,
//! both blended SRC_ALPHA / INV_SRC_ALPHA (RB_BLENDCONTROL 0x07060706), depth tested and written (RB_DEPTHCONTROL
//! 0x00708767, GEQUAL on the reversed depth), back faces culled (PA_SU_SC_MODE_CNTL 0x18002). Constants: the material's
//! Gloss, materialEmissive, materialDiffuse, hackEnvMultiplier, materialSpecular and shininess (the mesh's shader
//! parameters, `GlossMat`), gLightAmbient (0, 0, 0, 1) and gLightColor (1, 1, 1, -1), gSimpleHue = the node's colour
//! and alpha, gViewInverse the camera. The pixel program takes its light from the camera (VS o3 = -gViewInverse[2]);
//! the scene's light direction is not used.
//!
//! Here the programs are the translated microcode (`xs_gloss.wgsl`, crates/shaders). The frame's bones are pure
//! translations (pause_main draw 5109: c64.. identity rotations), so the frame is stretched on the CPU as before and
//! drawn with VS_TransformAndTextureAlpha, which computes the same terms for a vertex with one bone at weight 1 (each
//! vertex follows its heaviest bone, menu3d.rs, approx.). The UI
//! pass has no depth buffer of its own while a menu is open, so the meshes are drawn in `prepare` into a picture of
//! their own (4 samples, with a depth buffer: the original's z test and write, so the frame's bevels no longer blend
//! twice) and that premultiplied picture is laid "over" the UI pass in `paint` - the same result as blending the
//! meshes one after the other over the screen.
//!
//! The pane's texture is the menu's own picture (selfmovie.rs). The original draws the movie into it with RB_BLENDCONTROL
//! 0x01610706 (pause_main draws 1768 - 1860, 960 x 640): colour SRC_ALPHA / INV_SRC_ALPHA over the cleared colour, alpha
//! MAX; so its colour is the movie laid over the clear colour, which is what the port's premultiplied picture holds when
//! the clear is black, and PS_TexturedGloss takes it as it is (blending it with SRC_ALPHA once more). *(approx.)* the
//! port's picture is cleared to transparent black (the original to the movie's background colour) and its alpha is
//! accumulated "over" (egui's renderer) where the original keeps the largest alpha drawn: the same where one shape covers
//! a pixel.
//!
//! *(approx.)* dpFrontSampler, the dual-paraboloid map of the surroundings, is the port's environment cube looked up
//! along the mirror direction (as pu_fx.wgsl does); the vertex colour is white (it only meets gLightAmbient, 0 in the
//! pause); no test against the 3D frame's depth (the pause scene's own camera is not the world camera here).
//!
//! Compared with the original (pause_main, 1280 x 720, night; the port at 1600 x 900, 00:43, `MCLA_PAUSE_TEST`; share of
//! white over the scene beside the frame): outer bevel 0.20 - 0.23 / port 0.229, rim line 0.54 - 0.58 / 0.572, pane
//! margin 0.19 - 0.24 / 0.226 - 0.231, the same profile. OPEN: inside the movie's dark panels the original reads 7 - 9 where
//! the port reads 23 (night): the frame's front sheet, which wraps its 60 % pane across the whole panel (uv -3.7 .. 0.67),
//! shows through the pane (alpha = picture alpha x 0.77) at 0.24 here; by the original's constants it should show there
//! too, yet its panel needs the frame at 0.08 or less. Not the winding (the other one loses the pane). A clue: where
//! the sheet lands depends on the stretch - with the four bone weights blended as VS_TransformSkinned does (instead of the
//! heaviest bone) even the egui path's panels lighten (40 -> 61); the stretch's bones are not decoded yet. Until then
//! this path is off: `MCLA_MENU_GLOSS=1` turns it on (the old egui way otherwise).

use glam::{Mat4, Vec4};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Off until the panel difference below is found (`MCLA_MENU_GLOSS=1` turns it on).
pub fn enabled() -> bool {
    std::env::var("MCLA_MENU_GLOSS").is_ok_and(|v| v.trim() == "1") && !crate::render::xs::text("xs_gloss.wgsl").starts_with("// stub")
}

/// The Gloss effect's material terms of a mesh (its shader's parameters by the lowercase joaat of their names; a
/// parameter the mesh lacks keeps the effect's default).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlossMat {
    pub gloss: f32,
    pub emissive: Vec4,
    pub diffuse: Vec4,
    pub hack_env: f32,
    pub specular: Vec4,
    pub shininess: f32,
}

pub mod ph {
    pub const GLOSS: u32 = 0xb66af745;
    pub const EMISSIVE: u32 = 0xf27d3f99;
    pub const DIFFUSE: u32 = 0x1948c16c;
    pub const HACK_ENV: u32 = 0x15a081ef;
    pub const SPECULAR: u32 = 0xfede494e;
    pub const SHININESS: u32 = 0x111c9c95;
}

impl GlossMat {
    /// The material of a shader instance of the Gloss effect; `None` for any other effect.
    pub fn of(s: &mcla_formats::rage::drawable::Shader) -> Option<GlossMat> {
        use mcla_formats::rage::drawable::ShaderParam;
        if !s.name.eq_ignore_ascii_case("gloss") {
            return None;
        }
        let v = |h: u32, d: Vec4| {
            s.params.iter().find_map(|(ph, p)| match p {
                ShaderParam::Vector(v) if *ph == h && !v.is_empty() => Some(v[0]),
                _ => None,
            }).unwrap_or(d)
        };
        Some(GlossMat {
            gloss: v(ph::GLOSS, Vec4::ZERO).x,
            emissive: v(ph::EMISSIVE, Vec4::new(0.0, 0.0, 0.0, 1.0)),
            diffuse: v(ph::DIFFUSE, Vec4::ONE),
            hack_env: v(ph::HACK_ENV, Vec4::X).x,
            specular: v(ph::SPECULAR, Vec4::new(0.0, 0.0, 0.0, 1.0)),
            shininess: v(ph::SHININESS, Vec4::ZERO).x,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vtx {
    /// the mesh's own space (the frame already stretched), w = 1
    pub pos: [f32; 4],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// The constant registers the programs read, in this order in the uniform block.
const REGS: [u32; 22] = [0, 1, 2, 3, 8, 9, 10, 11, 12, 13, 14, 15, 26, 39, 100, 101, 102, 103, 104, 105, 106, 208];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Consts {
    c: [[f32; 4]; 22],
    /// (unused)
    flags: [f32; 4],
    _pad: [[f32; 4]; 9],
}

/// The texture of one draw.
#[derive(Clone)]
pub enum Tex {
    /// a mesh texture of `resources/ui/meshtextures.r9`: name, size and RGBA (uploaded once)
    Mesh(String, [u32; 2], Arc<Vec<u8>>),
    /// a self movie's picture (selfmovie.rs)
    Movie(String),
}

/// One mesh: its vertices as a triangle list, the camera and world, its material and hue.
pub struct Draw {
    pub verts: Vec<Vtx>,
    pub world: Mat4,
    pub vp: Mat4,
    pub view_inverse: Mat4,
    pub mat: GlossMat,
    /// gSimpleHue: the node's colour and alpha
    pub hue: Vec4,
    pub ambient: Vec4,
    pub light: Vec4,
    pub tex: Tex,
}

impl Draw {
    fn consts(&self) -> Consts {
        let wvp = self.vp * self.world;
        let mut c = [[0.0f32; 4]; 22];
        let put = |c: &mut [[f32; 4]; 22], r: u32, v: Vec4| {
            if let Some(i) = REGS.iter().position(|x| *x == r) {
                c[i] = v.to_array();
            }
        };
        for k in 0..4 {
            put(&mut c, k as u32, self.world.col(k));
            put(&mut c, 8 + k as u32, wvp.col(k));
            put(&mut c, 12 + k as u32, self.view_inverse.col(k));
        }
        put(&mut c, 26, self.ambient);
        put(&mut c, 39, self.light);
        put(&mut c, 100, Vec4::new(self.mat.gloss, 0.0, 0.0, 0.0));
        put(&mut c, 101, self.mat.emissive);
        put(&mut c, 102, self.mat.diffuse);
        put(&mut c, 103, Vec4::new(self.mat.hack_env, 0.0, 0.0, 0.0));
        put(&mut c, 104, self.hue);
        put(&mut c, 105, self.mat.specular);
        put(&mut c, 106, Vec4::new(self.mat.shininess, 0.0, 0.0, 0.0));
        put(&mut c, 208, self.hue);
        Consts { c, flags: [0.0; 4], _pad: [[0.0; 4]; 9] }
    }
}

const SAMPLES: u32 = 4;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Target {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::TextureView,
}

/// The programs, the meshes' picture and the textures, shared by the UI pass's renderers.
pub struct Shared {
    pipeline: Option<wgpu::RenderPipeline>,
    c_layout: wgpu::BindGroupLayout,
    t_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    env_sampler: wgpu::Sampler,
    env: Option<wgpu::TextureView>,
    /// per draw of a frame: its constants and its group 0 (constants + environment)
    slots: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    textures: HashMap<String, wgpu::BindGroup>,
    target: Option<Target>,
    /// the composite's binding of the resolved picture
    pic_layout: wgpu::BindGroupLayout,
    pic_bg: Option<wgpu::BindGroup>,
    /// the frame that last drew (its picture is laid over the UI pass)
    drawn: u64,
}

impl Shared {
    pub fn new(device: &wgpu::Device) -> Arc<Mutex<Shared>> {
        let tex = |b: u32, dim: wgpu::TextureViewDimension| wgpu::BindGroupLayoutEntry { binding: b, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dim, multisampled: false }, count: None };
        let samp = |b: u32| wgpu::BindGroupLayoutEntry { binding: b, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None };
        let c_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("menu gloss consts"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                tex(1, wgpu::TextureViewDimension::Cube),
                samp(2),
            ],
        });
        let t_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("menu gloss texture"), entries: &[tex(0, wgpu::TextureViewDimension::D2), samp(1)] });
        let pic_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("menu gloss picture"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }],
        });
        // (the self picture's fetch: linear, level 0, wrap - selfmovie.rs; the mesh textures likewise)
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("menu gloss"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("menu gloss env"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        Arc::new(Mutex::new(Shared { pipeline: None, c_layout, t_layout, sampler, env_sampler, env: None, slots: Vec::new(), textures: HashMap::new(), target: None, pic_layout, pic_bg: None, drawn: 0 }))
    }

    /// The environment cube (set once by the render loop; a read-only view of the cars' cube).
    pub fn set_env(&mut self, env: &wgpu::Texture) {
        if self.env.is_none() {
            self.env = Some(env.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::Cube), ..Default::default() }));
            self.slots.clear();
        }
    }

    fn pipeline(&mut self, device: &wgpu::Device) -> &wgpu::RenderPipeline {
        if self.pipeline.is_none() {
            let src = crate::render::xs::split_consts(&format!("{GLUE}\n{}", crate::render::xs::text("xs_gloss.wgsl")));
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("menu gloss"), source: wgpu::ShaderSource::Wgsl(src.into()) });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("menu gloss"), bind_group_layouts: &[&self.c_layout, &self.t_layout], push_constant_ranges: &[] });
            // SRC_ALPHA / INV_SRC_ALPHA on colour; alpha accumulated "over" so the picture is premultiplied
            let colour = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
            let alpha = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
            self.pipeline = Some(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("menu gloss"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vtx>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x3, 2 => Float32x2] }],
                },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: Some(wgpu::BlendState { color: colour, alpha }), write_mask: wgpu::ColorWrites::ALL })] }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: Some(wgpu::Face::Back), front_face: wgpu::FrontFace::Ccw, ..Default::default() },
                // (the original's GEQUAL on its reversed depth: nearer or equal passes; written)
                depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::LessEqual, stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
                multiview: None,
                cache: crate::render::pcache::get(),
            }));
        }
        self.pipeline.as_ref().unwrap()
    }

    fn target(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let ext = wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 };
        let mk = |label, format, samples, usage| device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: ext, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] }).create_view(&Default::default());
        let msaa = mk("menu gloss msaa", FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let depth = mk("menu gloss depth", wgpu::TextureFormat::Depth32Float, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let resolve = mk("menu gloss picture", FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        self.pic_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("menu gloss picture"), layout: &self.pic_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&resolve) }] }));
        self.target = Some(Target { size, msaa, depth, resolve });
    }

    fn texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, tex: &Tex, movies: &crate::selfmovie::Pics) -> Option<wgpu::BindGroup> {
        let bg = |view: &wgpu::TextureView| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("menu gloss texture"), layout: &self.t_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }] });
        match tex {
            Tex::Movie(key) => movies.view(key).map(|v| bg(&v)),
            Tex::Mesh(name, [w, h], rgba) => {
                if let Some(b) = self.textures.get(name) {
                    return Some(b.clone());
                }
                if *w == 0 || *h == 0 || rgba.len() < (*w * *h * 4) as usize {
                    return None;
                }
                use wgpu::util::DeviceExt;
                let t = device.create_texture_with_data(queue, &wgpu::TextureDescriptor { label: Some("menu gloss mesh texture"), size: wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }, wgpu::util::TextureDataOrder::LayerMajor, rgba);
                let b = bg(&t.create_view(&Default::default()));
                self.textures.insert(name.clone(), b.clone());
                Some(b)
            }
        }
    }
}

/// The program glue: inputs in the vertex program's fetch order, the constants, the two samplers.
const GLUE: &str = r#"
struct XsVsOut {
    pos: vec4<f32>,
    o: array<vec4<f32>, 16>,
};
var<private> xs_c: array<vec4<f32>, 256>;
struct Consts { c: array<vec4<f32>, 22>, flags: vec4<f32>, pad: array<vec4<f32>, 9> };
@group(0) @binding(0) var<uniform> u: Consts;
@group(0) @binding(1) var t_env: texture_cube<f32>;
@group(0) @binding(2) var s_env: sampler;
@group(1) @binding(0) var t_diff: texture_2d<f32>;
@group(1) @binding(1) var s_diff: sampler;
var<private> g_refl: vec3<f32>;
fn load_consts() {
    xs_c[0] = u.c[0]; xs_c[1] = u.c[1]; xs_c[2] = u.c[2]; xs_c[3] = u.c[3];
    xs_c[8] = u.c[4]; xs_c[9] = u.c[5]; xs_c[10] = u.c[6]; xs_c[11] = u.c[7];
    xs_c[12] = u.c[8]; xs_c[13] = u.c[9]; xs_c[14] = u.c[10]; xs_c[15] = u.c[11];
    xs_c[26] = u.c[12]; xs_c[39] = u.c[13];
    xs_c[100] = u.c[14]; xs_c[101] = u.c[15]; xs_c[102] = u.c[16]; xs_c[103] = u.c[17];
    xs_c[104] = u.c[18]; xs_c[105] = u.c[19]; xs_c[106] = u.c[20]; xs_c[208] = u.c[21];
}
// dpFrontSampler: the port's environment cube along the mirror direction (approx., see menugloss.rs)
fn xs_gloss_s0(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_env, s_env, g_refl, 0.0);
}
// DiffuseSampler: the mesh's texture, or the menu's picture as it is (see menugloss.rs)
fn xs_gloss_s1(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_diff, s_diff, uv);
}
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) o0: vec4<f32>,
    @location(1) o2: vec4<f32>,
    @location(2) o3: vec4<f32>,
    @location(3) o4: vec4<f32>,
    @location(4) o8: vec4<f32>,
    @location(5) o9: vec4<f32>,
};
@vertex fn vs_main(@location(0) p: vec4<f32>, @location(1) n: vec3<f32>, @location(2) uv: vec2<f32>) -> VOut {
    load_consts();
    var vin: array<vec4<f32>, 16>;
    vin[0] = p;
    vin[1] = vec4<f32>(1.0);
    vin[2] = vec4<f32>(n, 1.0);
    vin[3] = vec4<f32>(1.0, 0.0, 0.0, 1.0);
    vin[4] = vec4<f32>(uv, 0.0, 1.0);
    let o = xs_gloss_vs(vin);
    var r: VOut;
    r.pos = o.pos;
    r.o0 = o.o[0];
    r.o2 = o.o[2];
    r.o3 = o.o[3];
    r.o4 = o.o[4];
    r.o8 = o.o[8];
    r.o9 = o.o[9];
    return r;
}
@fragment fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    load_consts();
    var rin: array<vec4<f32>, 16>;
    rin[0] = i.o0;
    rin[2] = i.o2;
    rin[3] = i.o3;
    rin[4] = i.o4;
    rin[8] = i.o8;
    rin[9] = i.o9;
    g_refl = reflect(-normalize(xs_c[15].xyz - i.o2.xyz), normalize(i.o4.xyz));
    return xs_gloss_ps(rin);
}
"#;

const COMPOSITE: &str = r#"
@group(0) @binding(0) var pic: texture_2d<f32>;
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}
@fragment fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let d = vec2<i32>(textureDimensions(pic));
    let q = min(vec2<i32>(p.xy), d - vec2<i32>(1));
    return textureLoad(pic, q, 0);
}
"#;

/// One UI renderer's composite pipeline (its sample count and depth attachment) and this frame's vertex buffers.
pub struct GlossRes {
    composite: wgpu::RenderPipeline,
    shared: Arc<Mutex<Shared>>,
    movies: crate::selfmovie::Pics,
}

impl GlossRes {
    pub fn new(device: &wgpu::Device, shared: Arc<Mutex<Shared>>, movies: crate::selfmovie::Pics, format: wgpu::TextureFormat, samples: u32, depth: bool) -> GlossRes {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("menu gloss composite"), source: wgpu::ShaderSource::Wgsl(COMPOSITE.into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("menu gloss composite"), bind_group_layouts: &[&shared.lock().unwrap().pic_layout], push_constant_ranges: &[] });
        // (premultiplied "over"; alpha as selfmovie.rs's)
        let colour = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let alpha = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::OneMinusDstAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("menu gloss composite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState { color: colour, alpha }), write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: depth.then(|| wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: false, depth_compare: wgpu::CompareFunction::Always, stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: crate::render::pcache::get(),
        });
        GlossRes { composite, shared, movies }
    }

    pub fn shared(&self) -> &Arc<Mutex<Shared>> {
        &self.shared
    }
}

static FRAME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The Gloss meshes of one scene, in drawing order.
pub struct Paint {
    draws: Vec<Draw>,
    id: u64,
}

impl egui_wgpu::CallbackTrait for Paint {
    fn prepare(&self, device: &wgpu::Device, queue: &wgpu::Queue, sd: &egui_wgpu::ScreenDescriptor, enc: &mut wgpu::CommandEncoder, res: &mut egui_wgpu::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        use wgpu::util::DeviceExt;
        let Some(r) = res.get::<GlossRes>() else { return Vec::new() };
        let movies = r.movies.clone();
        let Ok(mut sh) = r.shared.lock() else { return Vec::new() };
        let Some(env) = sh.env.clone() else { return Vec::new() };
        sh.pipeline(device);
        sh.target(device, sd.size_in_pixels);
        // group 0 per draw: constants and the environment, kept from frame to frame
        while sh.slots.len() < self.draws.len() {
            let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("menu gloss consts"), size: std::mem::size_of::<Consts>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("menu gloss consts"),
                layout: &sh.c_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&env) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sh.env_sampler) }],
            });
            sh.slots.push((buf, bg));
        }
        let mut jobs = Vec::with_capacity(self.draws.len());
        for (k, d) in self.draws.iter().enumerate() {
            queue.write_buffer(&sh.slots[k].0, 0, bytemuck::bytes_of(&d.consts()));
            let Some(tb) = sh.texture(device, queue, &d.tex, &movies) else { continue };
            if d.verts.is_empty() {
                continue;
            }
            let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("menu gloss mesh"), contents: bytemuck::cast_slice(&d.verts), usage: wgpu::BufferUsages::VERTEX });
            jobs.push((k, tb, vb, d.verts.len() as u32));
        }
        let t = sh.target.as_ref().unwrap();
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("menu gloss"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.msaa, resolve_target: Some(&t.resolve), ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(sh.pipeline.as_ref().unwrap());
            for (k, tb, vb, n) in &jobs {
                pass.set_bind_group(0, &sh.slots[*k].1, &[]);
                pass.set_bind_group(1, tb, &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*n, 0..1);
            }
        }
        sh.drawn = self.id;
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &egui_wgpu::CallbackResources) {
        let Some(r) = res.get::<GlossRes>() else { return };
        let Ok(sh) = r.shared.lock() else { return };
        let (true, Some(bg)) = (sh.drawn == self.id, sh.pic_bg.as_ref()) else { return };
        pass.set_pipeline(&r.composite);
        pass.set_bind_group(0, bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Lay the scene's Gloss meshes (in drawing order) over the screen rectangle `rect`.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, draws: Vec<Draw>) {
    if draws.is_empty() {
        return;
    }
    let id = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, Paint { draws, id }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_hashes_are_the_lowercase_names() {
        let j = mcla_formats::hash::joaat;
        assert_eq!(ph::GLOSS, j("gloss"));
        assert_eq!(ph::EMISSIVE, j("materialemissive"));
        assert_eq!(ph::DIFFUSE, j("materialdiffuse"));
        assert_eq!(ph::HACK_ENV, j("hackenvmultiplier"));
        assert_eq!(ph::SPECULAR, j("materialspecular"));
        assert_eq!(ph::SHININESS, j("shininess"));
    }

    #[test]
    fn consts_land_in_their_registers() {
        let d = Draw {
            verts: vec![],
            world: Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0)),
            vp: Mat4::IDENTITY,
            view_inverse: Mat4::IDENTITY,
            mat: GlossMat { gloss: 0.5, emissive: Vec4::ZERO, diffuse: Vec4::ONE, hack_env: 1.0, specular: Vec4::W, shininess: 60.0 },
            hue: Vec4::new(1.0, 1.0, 1.0, 0.5),
            ambient: Vec4::W,
            light: Vec4::new(1.0, 1.0, 1.0, -1.0),
            tex: Tex::Movie("pause".into()),
        };
        let c = d.consts();
        let at = |r: u32| c.c[REGS.iter().position(|x| *x == r).unwrap()];
        assert_eq!(at(3), [1.0, 2.0, 3.0, 1.0]);
        assert_eq!(at(11), [1.0, 2.0, 3.0, 1.0]);
        assert_eq!(at(100)[0], 0.5);
        assert_eq!(at(106)[0], 60.0);
        assert_eq!(at(104), [1.0, 1.0, 1.0, 0.5]);
        assert_eq!(at(208), [1.0, 1.0, 1.0, 0.5]);
        assert_eq!(std::mem::size_of::<Consts>(), 512);
    }
}
