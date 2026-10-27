//! The game's own sky dome ("Skyhat"): its vertex and pixel programs translated from the disc's
//! microcode (`xs_sky.wgsl`, translated at install time from the disc, see `xs.rs`), fed with the constants the
//! original uploads (LIGHTING_WORK.md has the table), and the Perlin noise pass that makes its
//! clouds (`xs_perlin.wgsl`, an effect inside the executable).

use super::{post, GpuTexture};

/// vec4s of the uniform block: 12 of the vertex program, 24 of the pixel program, 10 of the
/// Perlin pass (see `sky_hat.wgsl`)
pub const HAT_VEC4S: usize = 48;
/// edge of the Perlin noise target (the original resolves 256 x 256 of an 8888 target)
pub const PERLIN_SIZE: u32 = 256;

/// The constants of one frame.
#[derive(Clone, Copy, Debug)]
pub struct HatConsts {
    /// vs_main: c64 CloudShadowOffset, c65 AzimuthStrength, c66 thickness / edge smooth / detail
    /// scale / detail strength, c67 StarFieldUVRepeat, c68 GalaxyOffset, c69 DetailOffset,
    /// c72 SkyColor, c73 sun direction, c75 AzimuthColor (west), c96 AzimuthColorEast,
    /// c97 AzimuthHeight, c98 top cloud bias / detail / threshold / height
    pub vs: [[f32; 4]; 12],
    /// ps_main_withstarfield_withmoon: c64 .. c83, then c100 .. c103
    pub ps: [[f32; 4]; 24],
    /// the Perlin pass: c64 Velocity1, c65 Velocity2, c66 Frequency1, c67 Frequency2,
    /// c68 Contrast, c69 Brightness, c70 NoiseWarp, c71 Persistance, c72 Phase, c73 ClumpBump
    pub perlin: [[f32; 4]; 10],
    /// ps_miniSky c100 HDRExposureClamp (the weather's)
    pub clamp: [f32; 4],
}

impl HatConsts {
    /// Every constant by the weight `f` of the way to another set (weather changes).
    pub fn lerp(&self, o: &HatConsts, f: f32) -> HatConsts {
        let mut out = *self;
        let mix = |a: &mut [f32; 4], b: &[f32; 4]| {
            for i in 0..4 {
                a[i] += (b[i] - a[i]) * f;
            }
        };
        for i in 0..12 {
            mix(&mut out.vs[i], &o.vs[i]);
        }
        for i in 0..24 {
            mix(&mut out.ps[i], &o.ps[i]);
        }
        for i in 0..10 {
            mix(&mut out.perlin[i], &o.perlin[i]);
        }
        mix(&mut out.clamp, &o.clamp);
        out
    }
}

pub struct SkyHat {
    pub buf: wgpu::Buffer,
    pub buf_bg: wgpu::BindGroup,
    pub pipeline: super::ms::MsPipe,
    /// the dome for the passes of the cars' environment map (`ps_dpenvmapsky`)
    pub pipeline_env: wgpu::RenderPipeline,
    pub pipeline_perlin: wgpu::RenderPipeline,
    /// the mini sky (`sky_mini.wgsl`: ps_miniSky into the 128 x 128 map the panes mirror)
    pub pipeline_mini: wgpu::RenderPipeline,
    pub perlin_view: wgpu::TextureView,
    /// stars, galaxy, the Perlin target, the detail noise (None until the textures are loaded)
    pub tex_bg: Option<wgpu::BindGroup>,
    /// the Perlin pass's noise texture
    pub perlin_bg: Option<wgpu::BindGroup>,
}

impl SkyHat {
    pub fn new(device: &wgpu::Device, globals_layout: &wgpu::BindGroupLayout, tex_layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler) -> SkyHat {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky hat"),
            size: (HAT_VEC4S * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hat_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky hat"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // the materials' sampler, for the Perlin pass (its group 0; render/mod.rs `sampler`)
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let buf_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky hat"),
            layout: &hat_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky hat"),
            source: wgpu::ShaderSource::Wgsl([include_str!("sky_hat.wgsl"), super::xs::text("xs_sky.wgsl"), super::xs::text("xs_sky_env.wgsl")].concat().into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky hat"),
            bind_group_layouts: &[globals_layout, tex_layout, &hat_layout],
            push_constant_ranges: &[],
        });
        let make_v = |device: &wgpu::Device, v: super::ms::Variant, entry: &str| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky hat"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_hat"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry),
                targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            // (the sky writes stencil 1 - ps_main_withstarfield, ref 1 in 0x210D; the depth of field skips it)
            depth_stencil: Some(v.ds(false, wgpu::CompareFunction::Always, true, Default::default())),
            multisample: v.ms(),
            multiview: None,
            cache: None,
        });
        let pipeline = {
            let (pl, shader) = (pl.clone(), shader.clone());
            super::ms::MsPipe::new(device, move |d, v| {
                let make_v = |device: &wgpu::Device, v: super::ms::Variant, entry: &str| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("sky hat"),
                    layout: Some(&pl),
                    vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_hat"), buffers: &[], compilation_options: Default::default() },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(entry),
                        targets: &[Some(wgpu::ColorTargetState { format: post::SCENE_FORMAT, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                    depth_stencil: Some(v.ds(false, wgpu::CompareFunction::Always, true, Default::default())),
                    multisample: v.ms(),
                    multiview: None,
                    cache: None,
                });
                make_v(d, v, "fs_hat")
            })
        };
        let pipeline_env = make_v(device, super::ms::PLAIN, "fs_hat_env");
        let perlin_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky perlin"),
            source: wgpu::ShaderSource::Wgsl([include_str!("sky_perlin.wgsl"), super::xs::text("xs_perlin.wgsl")].concat().into()),
        });
        let perlin_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky perlin"),
            bind_group_layouts: &[&hat_layout, tex_layout],
            push_constant_ranges: &[],
        });
        let pipeline_perlin = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky perlin"),
            layout: Some(&perlin_pl),
            vertex: wgpu::VertexState { module: &perlin_shader, entry_point: Some("vs_perlin"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &perlin_shader,
                entry_point: Some("fs_perlin"),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let mini_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky mini"),
            source: wgpu::ShaderSource::Wgsl([include_str!("sky_mini.wgsl"), super::xs::text("xs_minisky.wgsl")].concat().into()),
        });
        let pipeline_mini = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky mini"),
            layout: Some(&perlin_pl),
            vertex: wgpu::VertexState { module: &mini_shader, entry_point: Some("vs_mini"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &mini_shader,
                entry_point: Some("fs_mini"),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let perlin_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky perlin"),
            size: wgpu::Extent3d { width: PERLIN_SIZE, height: PERLIN_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let perlin_view = perlin_tex.create_view(&Default::default());
        SkyHat { buf, buf_bg, pipeline, pipeline_env, pipeline_perlin, pipeline_mini, perlin_view, tex_bg: None, perlin_bg: None }
    }

    /// The dome's textures: `starfield.dds`, `galaxy.dds`, `noise16_p.dds` (the high detail
    /// noise) and `perlinnoise_small.dds` (what the Perlin pass reads) of the disc's sky folder.
    #[allow(clippy::too_many_arguments)]
    pub fn bind(&mut self, device: &wgpu::Device, tex_layout: &wgpu::BindGroupLayout, stars: &GpuTexture, galaxy: &GpuTexture, detail: &GpuTexture, noise1: &GpuTexture, spare: &GpuTexture) {
        let make = |a: &wgpu::TextureView, b: &wgpu::TextureView, c: &wgpu::TextureView, d: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sky hat textures"),
                layout: tex_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(a) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(b) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(c) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(d) },
                ],
            })
        };
        self.tex_bg = Some(make(&stars.view, &galaxy.view, &self.perlin_view, &detail.view));
        self.perlin_bg = Some(make(&noise1.view, &spare.view, &spare.view, &spare.view));
    }

    pub fn ready(&self) -> bool {
        self.tex_bg.is_some() && self.perlin_bg.is_some()
    }

    pub fn write(&self, queue: &wgpu::Queue, c: &HatConsts) {
        let mut data = [[0.0f32; 4]; HAT_VEC4S];
        data[..12].copy_from_slice(&c.vs);
        data[12..36].copy_from_slice(&c.ps);
        data[36..46].copy_from_slice(&c.perlin);
        data[46] = c.clamp;
        queue.write_buffer(&self.buf, 0, bytemuck::cast_slice(&data));
    }

    /// The Perlin noise of this frame (before any pass that draws the sky).
    pub fn perlin_pass(&self, enc: &mut wgpu::CommandEncoder) {
        let Some(bg) = self.perlin_bg.as_ref() else { return };
        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sky perlin"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.perlin_view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        p.set_pipeline(&self.pipeline_perlin);
        p.set_bind_group(0, &self.buf_bg, &[]);
        p.set_bind_group(1, bg, &[]);
        p.draw(0..3, 0..1);
    }

    /// The mini sky of this frame (after the Perlin pass, before any pass whose panes mirror
    /// it): the game's one quad with ps_miniSky into the 128 x 128 map. `draw` false: the map
    /// is only cleared - alpha 0 tells city.wgsl `mini_sky` to fall back on its old stand-in.
    pub fn mini_pass(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, draw: bool) {
        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sky mini"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let (true, Some(bg)) = (draw, self.tex_bg.as_ref()) else { return };
        p.set_pipeline(&self.pipeline_mini);
        p.set_bind_group(0, &self.buf_bg, &[]);
        p.set_bind_group(1, bg, &[]);
        p.draw(0..3, 0..1);
    }
}
