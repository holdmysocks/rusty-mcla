//! The menus' 4x UI pass.
//!
//! The original (`work/ui-sharp/agent_msaa/notes.md`; UI.md "UI edge smoothing"): while a menu holds the renderer's
//! menu flag (+373, sub_82215A60 / sub_822150D8), phase 0x20000000 (sub_82227428 -> sub_822E5358) begins a 4x pass
//! on the back buffer, copies the tone-mapped frame into it full screen with point sampling (gps_whole draw 4281),
//! draws the UI - movies, the GPS map's 3D icons and roads, the self movies' meshes - at 4x and resolves it (the
//! samples averaged). Without the flag the UI draws straight on the 1x frame, as the port always did.
//!
//! Here: the finished frame is copied out of the surface, written into every sample of a 4x target, the UI is
//! drawn with a 4-sample egui renderer (no depth attachment: the x-ray callbacks read the scene's depth as a
//! texture), and the pass resolves into the surface's plain view.

pub const SAMPLES: u32 = 4;

pub struct Ui4 {
    samples: u32,
    size: (u32, u32),
    msaa: wgpu::TextureView,
    base: wgpu::Texture,
    bg: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl Ui4 {
    /// `surface`: the surface's format (its frames are copied into a texture of the same format); `plain`: the
    /// UI pass's colour format (the surface's view without sRGB).
    pub fn new(device: &wgpu::Device, surface: wgpu::TextureFormat, plain: wgpu::TextureFormat, w: u32, h: u32, samples: u32) -> Ui4 {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui4 copy"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ui4 copy"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ui4 copy"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui4 copy"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: plain, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: None,
        });
        let (msaa, base, bg) = Self::targets(device, &layout, surface, plain, w, h, samples);
        Ui4 { samples, size: (w, h), msaa, base, bg, layout, pipeline }
    }

    fn targets(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, surface: wgpu::TextureFormat, plain: wgpu::TextureFormat, w: u32, h: u32, samples: u32) -> (wgpu::TextureView, wgpu::Texture, wgpu::BindGroup) {
        let size = wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 };
        let msaa = device.create_texture(&wgpu::TextureDescriptor { label: Some("ui4"), size, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format: plain, usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] });
        let base = device.create_texture(&wgpu::TextureDescriptor { label: Some("ui4 frame copy"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: surface, usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[plain] });
        // (read through the plain view: the encoded values as they are, as the original's copy)
        let bv = base.create_view(&wgpu::TextureViewDescriptor { format: Some(plain), ..Default::default() });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ui4 copy"), layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&bv) }] });
        (msaa.create_view(&Default::default()), base, bg)
    }

    /// The pass: the frame copied into every sample, then the UI (`renderer`: 4 samples, no depth), resolved
    /// into `out` (the surface's plain view of `frame`).
    #[allow(clippy::too_many_arguments)]
    pub fn run(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, frame: &wgpu::Texture, out: &wgpu::TextureView, plain: wgpu::TextureFormat, renderer: &egui_wgpu::Renderer, jobs: &[egui::ClippedPrimitive], desc: &egui_wgpu::ScreenDescriptor) {
        let (w, h) = (frame.width(), frame.height());
        if self.size != (w, h) || self.base.format() != frame.format() {
            let (msaa, base, bg) = Self::targets(device, &self.layout, frame.format(), plain, w, h, self.samples);
            (self.msaa, self.base, self.bg, self.size) = (msaa, base, bg, (w, h));
        }
        enc.copy_texture_to_texture(frame.as_image_copy(), self.base.as_image_copy(), wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
        let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui4"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.msaa, resolve_target: Some(out), ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let mut pass = pass.forget_lifetime();
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bg, &[]);
        pass.draw(0..3, 0..1);
        renderer.render(&mut pass, jobs, desc);
    }
}

const SHADER: &str = r#"
@group(0) @binding(0) var frame: texture_2d<f32>;
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(frame, vec2<i32>(p.xy), 0);
}
"#;
