//! The original's "ground dome" (effect `GroundHat`, programs `vs_GroundDome` / `ps_GroundDome`,
//! model `resources/cutsceneItems/groundDome_items`; work/map/render_rule.md section 2).
//!
//! It is not a dome and does not follow the camera: a fixed sheet of 27 vertices / 30 triangles
//! under and round the whole city (x -4049 .. 3331, z -3386 .. 2850, a shallow bowl whose rim
//! lies on the map's rectangle), drawn with the identity matrix. Every pixel of it shows the
//! PREVIOUS frame's picture at the same place on the screen, read from the post chain's
//! quarter-size copy of the scene (after fog and glows, before bloom and the tone map):
//!
//!     uv = (0.5 x / w + 0.4998, -0.5 y / w + 0.4995)
//!
//! Depth test on with the scene's compare, depth write off, no blend, no culling; drawn after
//! the opaque city and the sky and before the props, so it fills what would be sky below the
//! horizon - beyond the far city, past the map's edge - with a blurred continuation of what the
//! screen showed there a frame ago. `MCLA_NO_GROUND_SHEET=1` leaves it out;
//! `MCLA_GROUND_SHEET_DEBUG=1` draws it with the effect's debug program (plain red).

use std::sync::OnceLock;
use wgpu::util::DeviceExt;

/// `groundDome_items.r102`: the 27 positions (world metres) ...
const POS: [[f32; 3]; 27] = [
    [-2288.08, -9.42, 2850.14],
    [-473.23, -18.04, 2848.90],
    [-2296.27, -165.94, 1765.90],
    [-2348.71, -33.47, 2848.90],
    [-3506.84, -67.13, 2850.15],
    [-4048.82, -67.13, 765.13],
    [-3756.97, -76.05, -1144.82],
    [-1210.09, -273.96, -3087.39],
    [-3782.68, -38.07, -3385.57],
    [-2829.04, -49.18, -3385.57],
    [-1271.36, 148.92, -3385.57],
    [-570.74, 56.96, -3385.57],
    [472.05, 193.17, -3385.57],
    [1149.86, -53.14, -2819.33],
    [1322.70, 96.72, -3385.57],
    [3236.73, 12.60, -3385.57],
    [2893.77, -127.25, -19.82],
    [2868.19, -108.21, -915.68],
    [3241.37, -12.72, -1234.09],
    [3331.39, 87.58, -791.48],
    [3328.46, 54.79, -566.97],
    [3328.46, -29.54, 89.82],
    [2075.85, -148.62, 1055.97],
    [3243.15, -17.19, 2850.15],
    [-473.23, -18.04, 2848.90],
    [1502.21, -17.47, 2849.85],
    [910.82, -29.32, 2850.31],
];

/// ... and its 30 triangles.
const TRI: [u16; 90] = [
    0, 1, 2, 3, 0, 2, 4, 3, 2, 5, 4, 2, 6, 5, 2, 6, 2, 7, 6, 7, 8, 7, 9, 8, 10, 9, 7, 7, 11, 10, 7, 12, 11, 13, 12, 7, 13, 14, 12, 13, 15, 14, 7, 16, 13, 2, 16, 7, 17, 15, 13, 16, 17, 13, 17, 18,
    15, 17, 19, 18, 17, 20, 19, 16, 20, 17, 16, 21, 20, 22, 21, 16, 22, 16, 2, 22, 23, 21, 22, 2, 24, 25, 23, 22, 24, 26, 22, 26, 25, 22,
];

const SHADER: &str = r#"
struct U { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var t_prev: texture_2d<f32>;
@group(0) @binding(2) var s_prev: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) clip: vec4<f32>,
};

// vs_GroundDome: clip = position x gWorldViewProj, handed on whole
@vertex
fn vs_sheet(@location(0) p: vec3<f32>) -> VsOut {
    var o: VsOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    o.clip = o.pos;
    return o;
}

// ps_GroundDome: the texture at the pixel's own place on the screen, written as it is
@fragment
fn fs_sheet(in: VsOut) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(0.5 * in.clip.x / in.clip.w + 0.4998, -0.5 * in.clip.y / in.clip.w + 0.4995);
    return vec4<f32>(textureSampleLevel(t_prev, s_prev, uv, 0.0).rgb, 1.0);
}

// ps_GroundDomeDebug: plain red (MCLA_GROUND_SHEET_DEBUG=1 - where the sheet shows)
@fragment
fn fs_sheet_debug(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
"#;

struct Sheet {
    pipeline: crate::render::ms::MsPipe,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    ubuf: wgpu::Buffer,
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
}

static SHEET: OnceLock<Sheet> = OnceLock::new();
static OFF: OnceLock<bool> = OnceLock::new();
static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn build(device: &wgpu::Device) -> Sheet {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ground sheet"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ground sheet layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ground sheet pl"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
    let pipeline = {
        let (pl, shader) = (pl.clone(), shader.clone());
        crate::render::ms::MsPipe::new(device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ground sheet"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_sheet"),
            buffers: &[wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }] }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(if std::env::var("MCLA_GROUND_SHEET_DEBUG").is_ok() { "fs_sheet_debug" } else { "fs_sheet" }),
            // (no blending: blend control 0 in the recorded draw)
            targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        // (no culling: 0x00018000 in the recorded draw)
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        // (depth test with the scene's compare - reversed z -, no depth write)
        depth_stencil: Some(v.ds(false, wgpu::CompareFunction::GreaterEqual, true, Default::default())),
        multisample: v.ms(),
        multiview: None,
        cache: None,
    }))
    };
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("ground sheet sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ground sheet view-projection"), size: 64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("ground sheet vertices"), contents: bytemuck::cast_slice(&POS), usage: wgpu::BufferUsages::VERTEX });
    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("ground sheet indices"), contents: bytemuck::cast_slice(&TRI), usage: wgpu::BufferUsages::INDEX });
    Sheet { pipeline, layout, sampler, ubuf, vb, ib }
}

/// Draw the sheet into the scene pass. Call after the opaque city (and the sky) and before the
/// props; `view_proj` is the camera's plain view-projection of this frame. The texture is the
/// post chain's quarter-size scene copy, which at this point still holds the frame before.
/// Leaves the pass with the globals bound in group 0 again; the caller sets its own pipeline
/// and groups 1 .. 3 afterwards, as it does anyway.
pub fn draw(pass: &mut wgpu::RenderPass<'_>, gpu: &crate::render::Gpu, view_proj: glam::Mat4) {
    if *OFF.get_or_init(|| std::env::var("MCLA_NO_GROUND_SHEET").is_ok()) {
        return;
    }
    // (The first two frames go without it: the quarter-size copy does not exist before the post
    // chain has run once, and the sheet would feed that black back to itself for a long time.
    // The original starts from whatever its last frame was - a loading screen.)
    if CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 2 {
        return;
    }
    let s = SHEET.get_or_init(|| build(&gpu.device));
    gpu.queue.write_buffer(&s.ubuf, 0, bytemuck::cast_slice(&view_proj.to_cols_array()));
    // (made each frame: the quarter target is replaced when the window is resized)
    let bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ground sheet bg"),
        layout: &s.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: s.ubuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(gpu.post.quarter_view()) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&s.sampler) },
        ],
    });
    pass.set_pipeline(&s.pipeline);
    pass.set_bind_group(0, &bg, &[]);
    pass.set_vertex_buffer(0, s.vb.slice(..));
    pass.set_index_buffer(s.ib.slice(..), wgpu::IndexFormat::Uint16);
    // (vs_GroundDome: stencil ALWAYS / REPLACE ref 1 where its depth test passes - LIGHTING.md, sea; the depth of
    // field leaves those pixels alone)
    pass.set_stencil_reference(1);
    pass.draw_indexed(0..TRI.len() as u32, 0, 0..1);
    pass.set_stencil_reference(0);
    pass.set_bind_group(0, &gpu.globals_bg, &[]);
}
