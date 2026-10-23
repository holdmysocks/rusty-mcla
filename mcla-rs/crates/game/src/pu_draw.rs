//! Step 19f: the power-up sources drawn as the original draws them (sub_82682590 ->
//! sub_82681760 the icon card, sub_82682318 the ground glow; textures and shader names from
//! sub_82681CD8). The rules are in render/powerup.wgsl; SAVE_WORK.md "Step 19f".

use crate::render::Gpu;
use glam::Vec3;
use std::sync::Mutex;

struct State {
    card: crate::render::ms::MsPipe,
    glow: crate::render::ms::MsPipe,
    bind: wgpu::BindGroup,
    cam: wgpu::Buffer,
    cards: wgpu::Buffer,
    glows: wgpu::Buffer,
    /// the original's turning counter (0x829076E0)
    spin: f32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// most cards / glows a frame
const MAX: usize = 512;
/// the icon textures' size (every hud_pu_*.dds is 128 x 128)
const SIZE: u32 = 128;
/// the mask's layer (hud_pu_bkgrnd_mask); 0 .. 23 the types' icons
const MASK: u32 = 24;

/// The icon texture of each type: `$/ui/powerups/hud_pu_%s` with the names of table 0x828282C8
/// (Pulse is "impulse"; the megas and HitByAgro have none on the disc).
const HUD: [&str; 24] = [
    "Nitro", "Ice", "Reverse", "StealthMode", "Disrupter", "Shield", "Stop", "impulse", "Mirror", "Agro", "HotPotato", "MegaNitro", "MegaIce", "MegaReverse", "MegaStealthMode", "MegaDisrupter", "MegaShield",
    "MegaStop", "Megaimpulse", "MegaMirror", "MegaAgro", "MegaHotPotato", "Mystery", "HitByAgro",
];

fn icon(data_dir: &str, name: &str) -> Option<Vec<u8>> {
    let file = format!("hud_pu_{name}.dds");
    let dir = std::path::Path::new(data_dir).join("ui/powerups");
    let p = [dir.join(&file), dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(&file)))].into_iter().find(|p| p.is_file())?;
    let t = mcla_formats::dds::parse_dds(&file, &std::fs::read(p).ok()?).ok()?;
    (t.width == SIZE && t.height == SIZE).then(|| t.to_rgba8())
}

/// Once, with the city's data.
pub fn load(gpu: &mut Gpu, data_dir: &str) {
    let device = &gpu.device;
    // the 24 icons and the mask, one layer each; a missing one is all black
    let levels = SIZE.trailing_zeros() + 1;
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("power-up icons"),
        size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: MASK + 1 },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut missing = Vec::new();
    for layer in 0..=MASK {
        let px = if layer == MASK { icon(data_dir, "bkgrnd_mask") } else { icon(data_dir, HUD[layer as usize]) };
        let mut px = px.unwrap_or_else(|| {
            missing.push(layer);
            [0u8, 0, 0, 255].repeat((SIZE * SIZE) as usize)
        });
        let mut w = SIZE;
        for l in 0..levels {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: l, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All },
                &px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(w) },
                wgpu::Extent3d { width: w, height: w, depth_or_array_layers: 1 },
            );
            if w == 1 {
                break;
            }
            let n = w / 2;
            let mut next = vec![0u8; (n * n * 4) as usize];
            for y in 0..n {
                for x in 0..n {
                    for c in 0..4 {
                        let at = |xx: u32, yy: u32| px[((yy * w + xx) * 4 + c) as usize] as u32;
                        next[((y * n + x) * 4 + c) as usize] = ((at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1) + 2) / 4) as u8;
                    }
                }
            }
            px = next;
            w = n;
        }
    }
    if missing.len() > 13 {
        log::warn!("pu_draw: icon textures missing for layers {missing:?}");
    }
    let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("power-up icons"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let cam = device.create_buffer(&wgpu::BufferDescriptor { label: Some("power-up camera"), size: 64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("power-up layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("power-up"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }, wgpu::BindGroupEntry { binding: 2, resource: cam.as_entire_binding() }],
    });
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("power-up"), source: wgpu::ShaderSource::Wgsl(include_str!("render/powerup.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("power-up pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    let attrs: [wgpu::VertexAttribute; 3] = std::array::from_fn(|i| wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16 * i as u64, shader_location: i as u32 });
    // (reversed depth: tested, not written - as the other passes over the finished scene)
    let make = |label: &'static str, vs: &'static str, fs: &'static str, blend: wgpu::BlendState| {
        let (pl, shader) = (pl.clone(), shader.clone());
        crate::render::ms::MsPipe::new(device, move |device, v| {
        let inst = wgpu::VertexBufferLayout { array_stride: 48, step_mode: wgpu::VertexStepMode::Instance, attributes: &attrs };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), buffers: &[inst.clone()], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fs),
                targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(blend), write_mask: wgpu::ColorWrites::COLOR })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(v.ds(false, wgpu::CompareFunction::GreaterEqual, false, Default::default())),
            multisample: v.ms(),
            multiview: None,
            cache: None,
        })
        })
    };
    // (the card: the two passes' source alpha / inverse source alpha put together - premultiplied)
    let over = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
    // (the ground glow: blend mode 7 of sub_8217F768 - source alpha, one)
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let card = make("power-up card", "vs_card", "fs_card", wgpu::BlendState { color: over, alpha: over });
    let glow = make("power-up ground glow", "vs_glow", "fs_glow", wgpu::BlendState { color: add, alpha: add });
    let buf = |label: &str| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (MAX * 48) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let (cards, glows) = (buf("power-up cards"), buf("power-up glows"));
    *STATE.lock().unwrap() = Some(State { card, glow, bind, cam, cards, glows, spin: 0.0 });
}

/// The ground glow's distance fade (sub_82682590 0x826825F0..0x82682640): full to 125 m,
/// nothing from 250 m.
pub fn fade(d: f32) -> f32 {
    ((1.0 - d * 0.004) * 2.0).clamp(0.0, 1.0)
}

/// The cards' and glows' instance data for a draw list (`PowerUps::draw_list`): the card's
/// middle and half width, m_powerupColor, (layer, half height); the glow's place and radius and
/// its two colours with their alphas faded.
pub fn instances(list: &[(Vec3, u8, f32, bool)], looks: &[crate::powerups::Look; 24], cam: Vec3) -> (Vec<[f32; 12]>, Vec<[f32; 12]>) {
    let (mut cards, mut glows) = (Vec::new(), Vec::new());
    for &(p, k, lift, ground) in list {
        let Some(l) = looks.get(k as usize) else { continue };
        let c = p + Vec3::new(0.0, l.icon_height + lift, 0.0);
        let col = l.powerup_color;
        cards.push([c.x, c.y, c.z, l.powerup_width, col[0], col[1], col[2], col[3], k as f32, l.powerup_height, 0.0, 0.0]);
        if ground {
            let f = fade(p.distance(cam));
            if f > 0.0 {
                let (a, b) = (l.ground_glow_color1, l.ground_glow_color2);
                glows.push([p.x, p.y, p.z, l.ground_glow_scale, a[0], a[1], a[2], a[3] * f, b[0], b[1], b[2], b[3] * f]);
            }
        }
    }
    (cards, glows)
}

/// The pass: over the finished scene (its depth tested), before the tone map. The card's turn:
/// the original's counter (0x829076E0) goes up 0.02 for every card drawn and the card turns
/// by its whole part (mod 8) eighths of a turn - which its eight-fold pattern does not show.
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder, cards: &[[f32; 12]], glows: &[[f32; 12]], right: Vec3, up: Vec3) {
    let mut guard = STATE.lock().unwrap();
    let Some(s) = guard.as_mut() else { return };
    if cards.is_empty() && glows.is_empty() {
        return;
    }
    s.spin += 0.02 * cards.len() as f32;
    let a = ((s.spin as i32).rem_euclid(8)) as f32 * std::f32::consts::FRAC_PI_4;
    let b = a + std::f32::consts::FRAC_PI_4;
    let consts = [right.x, right.y, right.z, 0.0, up.x, up.y, up.z, 0.0, a.cos(), a.sin(), -a.sin(), a.cos(), b.cos(), b.sin(), -b.sin(), b.cos()];
    gpu.queue.write_buffer(&s.cam, 0, bytemuck::cast_slice(&consts));
    let (nc, ng) = (cards.len().min(MAX), glows.len().min(MAX));
    if nc > 0 {
        gpu.queue.write_buffer(&s.cards, 0, bytemuck::cast_slice(&cards[..nc]));
    }
    if ng > 0 {
        gpu.queue.write_buffer(&s.glows, 0, bytemuck::cast_slice(&glows[..ng]));
    }
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("power-ups"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_bind_group(0, &gpu.globals_bg, &[]);
    pass.set_bind_group(1, &s.bind, &[]);
    // (sub_82682590: the card first, then the source's ground glow)
    if nc > 0 {
        pass.set_pipeline(&s.card);
        pass.set_vertex_buffer(0, s.cards.slice(..(nc * 48) as u64));
        pass.draw(0..6, 0..nc as u32);
    }
    if ng > 0 {
        pass.set_pipeline(&s.glow);
        pass.set_vertex_buffer(0, s.glows.slice(..(ng * 48) as u64));
        pass.draw(0..93, 0..ng as u32);
    }
}
