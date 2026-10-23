//! The sun's glow quad (LIGHTING_WORK.md "The low sun's veil"): one additive full-screen pass
//! over the finished scene, before the tone map's chain (render/sun_glow.wgsl has the rule and
//! where it comes from). Reads the scene's depth for the fog that the original lays over it.

use crate::render::Gpu;
use std::sync::Mutex;

struct SunGlow {
    pipeline: crate::render::ms::MsPipe,
    layout: wgpu::BindGroupLayout,
    /// rgb = the sky's SunColor of the hour, w = the share of the sun the query sees
    consts: wgpu::Buffer,
}

static STATE: Mutex<Option<SunGlow>> = Mutex::new(None);

/// Once, with the city's data.
pub fn load(gpu: &mut Gpu) {
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sun glow layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        }, wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let consts = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sun glow consts"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    // (the frame's globals: the particles' pipeline has their layout)
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sun glow"), source: wgpu::ShaderSource::Wgsl(include_str!("render/sun_glow.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sun glow pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    // (blend one / one, as the original's 0x01010101; the target's alpha is left alone)
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let pipeline = {
        let (pl, shader) = (pl.clone(), shader.clone());
        crate::render::ms::MsPipe::new(device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sun glow"),
        layout: Some(&pl),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_sun_glow"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_sun_glow"),
            targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::COLOR })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: v.ms(),
        multiview: None,
        cache: None,
    }))
    };
    *STATE.lock().unwrap() = Some(SunGlow { pipeline, layout, consts });
}

/// The share of the sun the game's occlusion query reports: its pixel count / 1250, and the
/// count reads 1000 in the reference whatever stands before the sun (0x827E1048 + 0 / + 24).
pub const VISIBLE: f32 = 0.8;

/// The pass: after the scene (its depth is read), before the tone map's chain. `sun_color`
/// is the sky file's SunColorKF at the hour, as stored (no decoding).
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder, sun_color: [f32; 3]) {
    let guard = STATE.lock().unwrap();
    let Some(s) = guard.as_ref() else { return };
    gpu.queue.write_buffer(&s.consts, 0, bytemuck::cast_slice(&[sun_color[0], sun_color[1], sun_color[2], VISIBLE]));
    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sun glow"),
        layout: &s.layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&gpu.depth) }, wgpu::BindGroupEntry { binding: 1, resource: s.consts.as_entire_binding() }],
    });
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("sun glow"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_pipeline(&s.pipeline);
    pass.set_bind_group(0, &gpu.globals_bg, &[]);
    pass.set_bind_group(1, &bind, &[]);
    pass.draw(0..3, 0..1);
}
