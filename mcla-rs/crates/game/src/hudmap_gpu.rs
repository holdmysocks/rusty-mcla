//! The GPS map's icons and MAP MODE's road overlay drawn in the 3D frame against the scene's depth,
//! as the original does it (`work/mm/rule_frame.md` 4.2 and 4.4).
//!
//! The original draws the HudMap icon queue (`sub_82236558`) and mcSimpleMap's five road meshes
//! (`sub_82624238`) through `sub_82723940` with DepthMap = the scene's depth, which picks the
//! "forced_zbuffer" technique of ui effect 0x62ca36ca (xUnlitRuntimeAlpha): VS_TransformAndTextureAlpha +
//! PS_TexturedAlternateDepth. From the microcode and the recorded GPU state (`cap/*.gpu.bin`,
//! `work/mm/xraygpu.py`, `xraycam.py`):
//! - no hardware depth test or depth write (RB_DEPTHCONTROL 0x00708761), blend SRC_ALPHA /
//!   INV_SRC_ALPHA on colour and alpha (RB_BLENDCONTROL 0x07060706), back faces culled
//!   (PA_SU_SC_MODE_CNTL 0x00018002), alpha test "greater than 0";
//! - the pixel shader reads the DepthMap (point sampled, the scene's reversed depth 1 - z) at its own
//!   pixel and calls the pixel BEHIND when `z - 7.5e-6 > 1 - DepthMap`, z = its own z/w of the D3D
//!   projection (0 near .. 1 far); colour = texture.rgb x gSimpleHue.rgb, alpha = texture.a x hue.a,
//!   times gXrayTerm when behind;
//! - gXrayTerm = min(3 t + 0.25, 1), t = the map camera's zoom fraction (`sub_82628DA8`, every frame;
//!   `sub_82623748` sets 0.25 on entry): 25 % at the lowest view, no x-ray from t = 0.25 up;
//! - the map camera's planes (`sub_82628DA8`): near = H - 700, far = H + 2000 for a camera height
//!   H > 800 (11 recordings, H 850 .. 9000), near 0.2 below that.
//! The target_ripple mesh's effect (0x13a285e4 RippleTarget, PS_RippleAltDepth) applies the same test
//! with the same 7.5e-6 to its rings' alpha.
//!
//! The port draws these with an egui paint callback in the UI layer of the GPS map, at the place the
//! egui meshes stood (roads, then the street names, then the icons), reading the 3D frame's depth
//! buffer (the UI pass holds it read-only). The port's depth is reversed and infinite (near / distance),
//! so the original's z is rebuilt from it: z = k (1 - n / d), d = port near / port depth.
//! `MCLA_XRAY_CPU=1` paints the old way (egui meshes, the 6 m relief, `hudmap::Heights`).

use crate::hudmap::{Assets, IconMesh, Queue, Ripple};
use crate::mapmesh::RoadMap;
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use wgpu::util::DeviceExt;

/// PS_TexturedAlternateDepth's (and PS_RippleAltDepth's) bias on the comparison, in z/w.
pub const XRAY_EPS: f32 = 7.5e-6;

/// `MCLA_XRAY_CPU=1`: the old CPU approximation instead of the depth test.
pub fn cpu_path() -> bool {
    std::env::var("MCLA_XRAY_CPU").map_or(false, |v| v.trim() != "0")
}

/// gXrayTerm for a map camera at height `cam_y` (`sub_82628DA8`: min(NM+1396 x 3 + 0.25, 1)).
pub fn xray_term(cam_y: f32) -> f32 {
    let t = ((cam_y - 850.0) / (9000.0 - 850.0)).clamp(0.0, 1.0);
    (t * 3.0 + 0.25).min(1.0)
}

/// The original map camera's near and far planes at height `cam_y` (`sub_82628DA8`). (Below 800 m
/// the code sets only the near plane, 0.2; the far one is then the caller's - 11000 here, inferred.)
pub fn orig_planes(cam_y: f32) -> (f32, f32) {
    if cam_y > 800.0 {
        (cam_y - 700.0, cam_y + 2000.0)
    } else {
        (0.2, 11000.0)
    }
}

/// z/w of the original's D3D projection (0 at `near`, 1 at `far`) for a point at view depth `d`.
pub fn orig_z(d: f32, near: f32, far: f32) -> f32 {
    far / (far - near) * (1.0 - near / d)
}

/// The same from the port's reversed infinite depth (`port_near / d`; 0 = nothing drawn).
pub fn orig_z_from_port(z_port: f32, port_near: f32, near: f32, far: f32) -> f32 {
    far / (far - near) * (1.0 - near * z_port / port_near)
}

/// PS_TexturedAlternateDepth: the pixel (z = its own z/w) is behind the scene whose z/w at that
/// pixel is `z_scene` (the DepthMap holds 1 - z_scene).
pub fn behind(z: f32, z_scene: f32) -> bool {
    z - XRAY_EPS > z_scene
}

/// The pixel's alpha: texture alpha x hue alpha, x gXrayTerm when behind.
pub fn alpha(tex_a: f32, hue_a: f32, behind: bool, xray: f32) -> f32 {
    if behind {
        tex_a * xray * hue_a
    } else {
        tex_a * hue_a
    }
}

/// The port's near plane from its view-projection (reversed infinite: clip z = near for every point).
pub fn port_near(vp: &Mat4) -> f32 {
    vp.w_axis.z
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    vp: [[f32; 4]; 4],
    /// x = gXrayTerm, y = k = far / (far - near) of the original's camera, z = its near, w = the port's near
    xray: [f32; 4],
    /// the ripple: x = clock, y = frequency, z = speed, w = falloff exponent
    ripple: [f32; 4],
    /// x = ripple amplitude
    ripple2: [f32; 4],
}

/// gXrayTerm as the GPS map last set it (0x8282DED0: 0.25 at boot and on entering the map, then the
/// map camera's every frame); the race markers are drawn with whatever it holds.
static LAST_XRAY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3e80_0000);

pub fn last_xray() -> f32 {
    f32::from_bits(LAST_XRAY.load(std::sync::atomic::Ordering::Relaxed))
}

impl Globals {
    fn new(vp: Mat4, cam: Vec3, ripple: Option<Ripple>) -> Globals {
        let (n, f) = orig_planes(cam.y);
        let r = ripple.unwrap_or(Ripple { t: 0.0, freq: 0.0, speed: 0.0, falloff: 1.0, amplitude: 0.0 });
        let x = xray_term(cam.y);
        LAST_XRAY.store(x.to_bits(), std::sync::atomic::Ordering::Relaxed);
        Globals {
            vp: vp.to_cols_array_2d(),
            xray: [x, f / (f - n), n, port_near(&vp)],
            ripple: [r.t, r.freq, r.speed, r.falloff],
            ripple2: [r.amplitude, 0.0, 0.0, 0.0],
        }
    }
}

/// One draw: world matrix, colour (gSimpleHue), mode (0 textured, 1 ripple, 2 flat colour).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawRec {
    world: [[f32; 4]; 4],
    colour: [f32; 4],
    mode: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vtx {
    pos: [f32; 3],
    uv: [f32; 2],
}

struct GpuMesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    /// per part: first index, index count, base vertex
    parts: Vec<(u32, u32, i32)>,
}

enum Cmd {
    Mesh { mesh: &'static str, part: usize, tex: Option<String>, inst: u32 },
    Road { layer: usize, inst: u32 },
}

#[derive(Default)]
struct Slot {
    globals: Option<wgpu::Buffer>,
    recs: Option<(wgpu::Buffer, usize)>,
    bg: Option<wgpu::BindGroup>,
    cmds: Vec<Cmd>,
}

/// The pipeline and the GPU copies of the meshes and pictures (egui's callback resources).
pub struct XrayRes {
    pipeline: wgpu::RenderPipeline,
    /// the same, both sides drawn (the free-roam icons' meshes - hookicons.rs)
    pipeline_both: wgpu::RenderPipeline,
    g_layout: wgpu::BindGroupLayout,
    t_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white: wgpu::BindGroup,
    textures: HashMap<String, wgpu::BindGroup>,
    meshes: HashMap<&'static str, GpuMesh>,
    roads: Option<Vec<(wgpu::Buffer, u32)>>,
    /// the 3D frame's depth buffer (set by the render loop each frame)
    pub depth: Option<wgpu::TextureView>,
    /// the pictures written every frame (the free-roam icons' movies): texture and binding
    dyn_textures: HashMap<String, (wgpu::Texture, wgpu::BindGroup)>,
    /// 0 = the road overlay, 1 = the icons, 2 = the race markers, 3 = the free-roam icons
    slots: [Slot; 4],
}

impl XrayRes {
    /// `format`: the UI pass's colour target (the surface's plain view); `samples`: the pass's sample count (4 in
    /// the menus' pass, UI.md "UI edge smoothing"); `depth`: the pass has the scene's Depth32Float attached.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, samples: u32, depth: bool) -> XrayRes {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hudmap xray"), source: wgpu::ShaderSource::Wgsl(include_str!("hudmap_xray.wgsl").into()) });
        let g_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hudmap xray globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            ],
        });
        let t_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hudmap xray texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hudmap xray"), bind_group_layouts: &[&g_layout, &t_layout], push_constant_ranges: &[] });
        let blend = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let make = |cull: Option<wgpu::Face>| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hudmap xray"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vtx>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2] }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState { color: blend, alpha: blend }), write_mask: wgpu::ColorWrites::ALL })],
            }),
            // (back faces culled as PA_SU_SC_MODE_CNTL 0x18002; seen from above, the icons' and roads' triangles run
            // counter-clockwise on the port's screen - with clockwise every one was culled; MCLA_XRAY_CULL=0: both sides)
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: cull,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            // (the UI pass's depth, read-only: no test, no writes - the shader does the test)
            depth_stencil: depth.then(|| wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: false, depth_compare: wgpu::CompareFunction::Always, stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: None,
        });
        let pipeline = make(if std::env::var("MCLA_XRAY_CULL").map_or(true, |v| v.trim() != "0") { Some(wgpu::Face::Back) } else { None });
        let pipeline_both = make(None);
        // (DiffuseSampler's fetch constant: wrap, trilinear)
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hudmap xray"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white = texture_bg_q(device, queue, &t_layout, &sampler, &[(1, 1, vec![255u8; 4])]);
        XrayRes { pipeline, pipeline_both, g_layout, t_layout, sampler, white, textures: HashMap::new(), dyn_textures: HashMap::new(), meshes: HashMap::new(), roads: None, depth: None, slots: Default::default() }
    }

    /// Fill a slot: globals, the draw records, the depth binding.
    fn fill(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, slot: usize, g: &Globals, recs: &[DrawRec], cmds: Vec<Cmd>) {
        let s = &mut self.slots[slot];
        s.cmds.clear();
        s.bg = None;
        let Some(depth) = self.depth.as_ref() else { return };
        if recs.is_empty() {
            return;
        }
        let gb = s.globals.get_or_insert_with(|| device.create_buffer(&wgpu::BufferDescriptor { label: Some("hudmap xray globals"), size: std::mem::size_of::<Globals>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        queue.write_buffer(gb, 0, bytemuck::bytes_of(g));
        if s.recs.as_ref().map_or(true, |(_, cap)| *cap < recs.len()) {
            let cap = recs.len().next_power_of_two().max(64);
            s.recs = Some((device.create_buffer(&wgpu::BufferDescriptor { label: Some("hudmap xray draws"), size: (cap * std::mem::size_of::<DrawRec>()) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }), cap));
        }
        let (rb, _) = s.recs.as_ref().unwrap();
        queue.write_buffer(rb, 0, bytemuck::cast_slice(recs));
        s.bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hudmap xray globals"),
            layout: &self.g_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: gb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: rb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
            ],
        }));
        s.cmds = cmds;
    }

    fn mesh(&mut self, device: &wgpu::Device, name: &'static str, m: &IconMesh) {
        if self.meshes.contains_key(name) {
            return;
        }
        let (mut vtx, mut idx, mut parts) = (Vec::new(), Vec::new(), Vec::new());
        for p in &m.parts {
            let base = vtx.len() as i32;
            for (i, q) in p.pos.iter().enumerate() {
                let uv = p.uv.get(i).copied().unwrap_or_default();
                vtx.push(Vtx { pos: q.to_array(), uv: uv.to_array() });
            }
            parts.push((idx.len() as u32, p.idx.len() as u32, base));
            idx.extend_from_slice(&p.idx);
        }
        if vtx.is_empty() || idx.is_empty() {
            vtx.push(Vtx { pos: [0.0; 3], uv: [0.0; 2] });
            idx.extend([0u32; 3]);
        }
        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(name), contents: bytemuck::cast_slice(&vtx), usage: wgpu::BufferUsages::VERTEX });
        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(name), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
        self.meshes.insert(name, GpuMesh { vb, ib, parts });
    }

    fn texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: &str, w: usize, h: usize, rgba8: &[u8]) {
        if self.textures.contains_key(key) || w == 0 || h == 0 || rgba8.len() < w * h * 4 {
            return;
        }
        let levels = mips(w, h, rgba8);
        let bg = texture_bg_q(device, queue, &self.t_layout, &self.sampler, &levels);
        self.textures.insert(key.to_string(), bg);
    }

    /// A picture rewritten every frame (one level: a render target has no mips).
    fn dyn_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: &str, w: u32, h: u32, rgba8: &[u8]) {
        if w == 0 || h == 0 || rgba8.len() < (w * h * 4) as usize {
            return;
        }
        if self.dyn_textures.get(key).map_or(true, |(t, _)| t.width() != w || t.height() != h) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hudmap xray movie"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let bg = bind(device, &self.t_layout, &self.sampler, &tex);
            self.dyn_textures.insert(key.to_string(), (tex, bg));
        }
        let (tex, _) = &self.dyn_textures[key];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &rgba8[..(w * h * 4) as usize],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }
}

/// The picture's box-filtered mip chain (straight alpha), as the CPU minimap's `hudmap::Img`.
fn mips(w: usize, h: usize, rgba8: &[u8]) -> Vec<(usize, usize, Vec<u8>)> {
    let mut out = vec![(w, h, rgba8[..w * h * 4].to_vec())];
    let mut cur: Vec<f32> = rgba8[..w * h * 4].iter().map(|&b| b as f32 / 255.0).collect();
    let (mut pw, mut ph) = (w, h);
    while pw > 1 || ph > 1 {
        let (nw, nh) = ((pw / 2).max(1), (ph / 2).max(1));
        let mut next = vec![0.0f32; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = ((2 * x + dx).min(pw - 1), (2 * y + dy).min(ph - 1));
                    for c in 0..4 {
                        next[(y * nw + x) * 4 + c] += cur[(sy * pw + sx) * 4 + c] * 0.25;
                    }
                }
            }
        }
        out.push((nw, nh, next.iter().map(|v| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8).collect()));
        cur = next;
        pw = nw;
        ph = nh;
    }
    out
}

fn texture_bg_q(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, levels: &[(usize, usize, Vec<u8>)]) -> wgpu::BindGroup {
    let (w, h, _) = &levels[0];
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hudmap xray picture"),
        size: wgpu::Extent3d { width: *w as u32, height: *h as u32, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (l, (lw, lh, px)) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: l as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*lw as u32 * 4), rows_per_image: Some(*lh as u32) },
            wgpu::Extent3d { width: *lw as u32, height: *lh as u32, depth_or_array_layers: 1 },
        );
    }
    bind(device, layout, sampler, &tex)
}

fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, tex: &wgpu::Texture) -> wgpu::BindGroup {
    let view = tex.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hudmap xray picture"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) }],
    })
}

fn paint_slot(res: &XrayRes, slot: usize, pass: &mut wgpu::RenderPass<'static>) {
    let s = &res.slots[slot];
    let Some(bg) = s.bg.as_ref() else { return };
    if s.cmds.is_empty() {
        return;
    }
    pass.set_pipeline(if slot == 3 { &res.pipeline_both } else { &res.pipeline });
    pass.set_bind_group(0, bg, &[]);
    for c in &s.cmds {
        match c {
            Cmd::Mesh { mesh, part, tex, inst } => {
                let Some(m) = res.meshes.get(mesh) else { continue };
                let Some(&(first, count, base)) = m.parts.get(*part) else { continue };
                let t = tex.as_ref().and_then(|k| res.textures.get(k).or_else(|| res.dyn_textures.get(k).map(|d| &d.1))).unwrap_or(&res.white);
                pass.set_bind_group(1, t, &[]);
                pass.set_vertex_buffer(0, m.vb.slice(..));
                pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(first..first + count, base, *inst..*inst + 1);
            }
            Cmd::Road { layer, inst } => {
                let Some((vb, n)) = res.roads.as_ref().and_then(|r| r.get(*layer)) else { continue };
                pass.set_bind_group(1, &res.white, &[]);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.draw(0..*n, *inst..*inst + 1);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The icon queue

/// One frame's icon queue for the callback: (mesh name, mesh, world, colour).
pub struct Icons {
    g: Globals,
    draws: Vec<(&'static str, Arc<IconMesh>, Mat4, [f32; 4])>,
}

/// The GPS map's icon queue as a paint callback over `screen` (the 3D frame's rectangle).
pub fn icons(screen: egui::Rect, assets: &mut Assets, q: &Queue, view_proj: Mat4, cam: Vec3, ripple: Option<Ripple>) -> egui::Shape {
    let draws = q.draws.iter().filter_map(|d| assets.mesh(d.mesh).map(|m| (d.mesh, m, d.m, d.colour))).collect();
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(screen, Icons { g: Globals::new(view_proj, cam, ripple), draws }))
}

impl egui_wgpu::CallbackTrait for Icons {
    fn prepare(&self, device: &wgpu::Device, queue: &wgpu::Queue, _sd: &egui_wgpu::ScreenDescriptor, _enc: &mut wgpu::CommandEncoder, res: &mut egui_wgpu::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = res.get_mut::<XrayRes>() else { return Vec::new() };
        let (mut recs, mut cmds) = (Vec::new(), Vec::new());
        for (name, m, world, colour) in &self.draws {
            r.mesh(device, name, m);
            let ripple = *name == "target_ripple";
            for (k, part) in m.parts.iter().enumerate() {
                // (as the egui path: a part without a picture is not drawn, the ripple's rings are the shader's own)
                let tex = if ripple {
                    None
                } else {
                    let Some(img) = part.tex.as_ref() else { continue };
                    let key = format!("{name}#{k}");
                    r.texture(device, queue, &key, img.w, img.h, &img.rgba8);
                    Some(key)
                };
                let inst = recs.len() as u32;
                recs.push(DrawRec { world: world.to_cols_array_2d(), colour: *colour, mode: [if ripple { 1 } else { 0 }, 0, 0, 0] });
                cmds.push(Cmd::Mesh { mesh: name, part: k, tex, inst });
            }
        }
        r.fill(device, queue, 1, &self.g, &recs, cmds);
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &egui_wgpu::CallbackResources) {
        if let Some(r) = res.get::<XrayRes>() {
            paint_slot(r, 1, pass);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The race markers (racemark.rs)

/// The race markers over the computer racers, in the 3D frame of the game camera `vp` at `cam`
/// (`sub_82225170` -> `sub_82723940` with the scene's DepthMap: the same technique as the GPS
/// map's icons, gXrayTerm as the map left it). *(approx.)* the original game camera's near and
/// far planes are not read: the port's near and a far of 4000 m stand in (they only move the
/// 7.5e-6 bias of the behind test).
pub struct Marks {
    g: Globals,
    mesh: Option<Arc<IconMesh>>,
    draws: Vec<(Mat4, [f32; 4])>,
}

pub fn race_marks(screen: egui::Rect, assets: &mut Assets, marks: &[crate::racemark::Mark], vp: Mat4, depth_test: bool) -> egui::Shape {
    let n = port_near(&vp);
    let f = 4000.0;
    let g = Globals { vp: vp.to_cols_array_2d(), xray: [if depth_test { last_xray() } else { 1.0 }, f / (f - n), n, n], ripple: [0.0, 0.0, 0.0, 1.0], ripple2: [0.0; 4] };
    let draws = marks.iter().map(|m| (m.m, m.colour)).collect();
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(screen, Marks { g, mesh: assets.mesh("icon_racer"), draws }))
}

impl egui_wgpu::CallbackTrait for Marks {
    fn prepare(&self, device: &wgpu::Device, queue: &wgpu::Queue, _sd: &egui_wgpu::ScreenDescriptor, _enc: &mut wgpu::CommandEncoder, res: &mut egui_wgpu::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = res.get_mut::<XrayRes>() else { return Vec::new() };
        let (mut recs, mut cmds) = (Vec::new(), Vec::new());
        if let Some(m) = self.mesh.as_ref() {
            r.mesh(device, "icon_racer", m);
            for (world, colour) in &self.draws {
                for (k, part) in m.parts.iter().enumerate() {
                    let Some(img) = part.tex.as_ref() else { continue };
                    let key = format!("icon_racer#{k}");
                    r.texture(device, queue, &key, img.w, img.h, &img.rgba8);
                    let inst = recs.len() as u32;
                    recs.push(DrawRec { world: world.to_cols_array_2d(), colour: *colour, mode: [0; 4] });
                    cmds.push(Cmd::Mesh { mesh: "icon_racer", part: k, tex: Some(key), inst });
                }
            }
        }
        r.fill(device, queue, 2, &self.g, &recs, cmds);
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &egui_wgpu::CallbackResources) {
        if let Some(r) = res.get::<XrayRes>() {
            paint_slot(r, 2, pass);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The free-roam icons (hookicons.rs)

/// One mesh of a free-roam icon's movie: its mesh, world matrix (the icon's frame times the
/// mesh's own), colour (the mesh's `color`, its `_alpha`), and the movie's stage as its texture
/// (`player0mesh`: slot, pixels) or the mesh's own pictures; `ripple`: the rings of RippleCustomDepth.
pub struct HookDraw {
    pub mesh: &'static str,
    pub world: Mat4,
    pub colour: [f32; 4],
    pub movie: Option<(usize, Arc<Vec<u8>>)>,
    pub ripple: bool,
}

pub struct HookIcons {
    g: Globals,
    meshes: Vec<(&'static str, Arc<IconMesh>)>,
    draws: Vec<HookDraw>,
}

/// The free-roam icons in the game camera's 3D frame (`sub_82225170` kind 1 -> `sub_82721258(movie,
/// 6)`: UnlitCustomDepth / RippleCustomDepth with no depth map - the hardware depth test).
/// *(approx.)* The hardware test is the shader's behind test with gXrayTerm 0 (bias 7.5e-6); both
/// sides are drawn (the cull state of the movies' 3D pass is not read); the rings take the GPS
/// map's Ripple tune (where RippleCustomDepth's parameters are set is not read).
pub fn hook_icons(screen: egui::Rect, assets: &mut Assets, draws: Vec<HookDraw>, vp: Mat4, ripple: Option<Ripple>) -> egui::Shape {
    let n = port_near(&vp);
    let f = 4000.0;
    let r = ripple.unwrap_or(Ripple { t: 0.0, freq: 0.0, speed: 0.0, falloff: 1.0, amplitude: 0.0 });
    // (MCLA_HOOK_XRAY=<0..1>: the pixels behind the scene drawn at that alpha - to see the test work)
    let x = std::env::var("MCLA_HOOK_XRAY").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
    let g = Globals { vp: vp.to_cols_array_2d(), xray: [x, f / (f - n), n, n], ripple: [r.t, r.freq, r.speed, r.falloff], ripple2: [r.amplitude, 0.0, 0.0, 0.0] };
    let mut meshes: Vec<(&'static str, Arc<IconMesh>)> = Vec::new();
    for d in &draws {
        if !meshes.iter().any(|m| m.0 == d.mesh) {
            if let Some(m) = assets.mesh(d.mesh) {
                meshes.push((d.mesh, m));
            }
        }
    }
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(screen, HookIcons { g, meshes, draws }))
}

impl egui_wgpu::CallbackTrait for HookIcons {
    fn prepare(&self, device: &wgpu::Device, queue: &wgpu::Queue, _sd: &egui_wgpu::ScreenDescriptor, _enc: &mut wgpu::CommandEncoder, res: &mut egui_wgpu::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = res.get_mut::<XrayRes>() else { return Vec::new() };
        let (mut recs, mut cmds) = (Vec::new(), Vec::new());
        let mut written = [false; 3];
        for d in &self.draws {
            let Some((name, m)) = self.meshes.iter().find(|m| m.0 == d.mesh) else { continue };
            r.mesh(device, name, m);
            for (k, part) in m.parts.iter().enumerate() {
                let tex = if d.ripple {
                    None
                } else if let Some((slot, rgba)) = d.movie.as_ref() {
                    let key = format!("3dicons#{slot}");
                    if !written[*slot % 3] {
                        written[*slot % 3] = true;
                        r.dyn_texture(device, queue, &key, crate::hookicons::RT, crate::hookicons::RT, rgba);
                    }
                    Some(key)
                } else {
                    let Some(img) = part.tex.as_ref() else { continue };
                    let key = format!("{name}#{k}");
                    r.texture(device, queue, &key, img.w, img.h, &img.rgba8);
                    Some(key)
                };
                let inst = recs.len() as u32;
                recs.push(DrawRec { world: d.world.to_cols_array_2d(), colour: d.colour, mode: [if d.ripple { 1 } else { 0 }, 0, 0, 0] });
                cmds.push(Cmd::Mesh { mesh: name, part: k, tex, inst });
            }
        }
        r.fill(device, queue, 3, &self.g, &recs, cmds);
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &egui_wgpu::CallbackResources) {
        if let Some(r) = res.get::<XrayRes>() {
            paint_slot(r, 3, pass);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// MAP MODE's road overlay

/// The five road layers as plain triangle lists in world space (built once).
pub struct RoadVerts(Vec<Vec<Vtx>>);

static ROAD_VERTS: OnceLock<Arc<RoadVerts>> = OnceLock::new();

fn road_verts(roads: &RoadMap) -> Arc<RoadVerts> {
    ROAD_VERTS
        .get_or_init(|| {
            Arc::new(RoadVerts(
                roads
                    .layers
                    .iter()
                    .map(|l| {
                        let mut v = Vec::with_capacity(l.tris.len() * 3);
                        for (i, t) in l.tris.iter().enumerate() {
                            let y = l.ys.get(i).copied().unwrap_or([0.0; 3]);
                            for j in 0..3 {
                                v.push(Vtx { pos: [t[j].x, y[j], t[j].y], uv: [0.0; 2] });
                            }
                        }
                        v
                    })
                    .collect(),
            ))
        })
        .clone()
}

pub struct Roads {
    g: Globals,
    verts: Arc<RoadVerts>,
    /// (layer, colour, lift)
    draws: Vec<(usize, [f32; 4], f32)>,
}

/// MAP MODE's overlay (`hudmap::OVERLAY` order, colours and fades; lift 2 + 98 t) as a paint callback.
pub fn roads(screen: egui::Rect, roads: &RoadMap, view_proj: Mat4, cam: Vec3, fades: &[f32; 5]) -> egui::Shape {
    let t = ((cam.y - 850.0) / (9000.0 - 850.0)).clamp(0.0, 1.0);
    let lift = 2.0 + 98.0 * t;
    let mut draws = Vec::new();
    for (k, (layer, col, _)) in crate::hudmap::OVERLAY.iter().enumerate() {
        let a = col[3] * fades[k];
        if a <= 0.01 || *layer >= roads.layers.len() {
            continue;
        }
        draws.push((*layer, [col[0], col[1], col[2], a], lift));
    }
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(screen, Roads { g: Globals::new(view_proj, cam, None), verts: road_verts(roads), draws }))
}

impl egui_wgpu::CallbackTrait for Roads {
    fn prepare(&self, device: &wgpu::Device, queue: &wgpu::Queue, _sd: &egui_wgpu::ScreenDescriptor, _enc: &mut wgpu::CommandEncoder, res: &mut egui_wgpu::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = res.get_mut::<XrayRes>() else { return Vec::new() };
        if r.roads.is_none() {
            r.roads = Some(
                self.verts
                    .0
                    .iter()
                    .map(|v| {
                        let data: &[Vtx] = if v.is_empty() { &[Vtx { pos: [0.0; 3], uv: [0.0; 2] }] } else { v };
                        (device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("hudmap xray roads"), contents: bytemuck::cast_slice(data), usage: wgpu::BufferUsages::VERTEX }), v.len() as u32)
                    })
                    .collect(),
            );
        }
        let (mut recs, mut cmds) = (Vec::new(), Vec::new());
        for &(layer, colour, lift) in &self.draws {
            let inst = recs.len() as u32;
            recs.push(DrawRec { world: Mat4::from_translation(Vec3::new(0.0, lift, 0.0)).to_cols_array_2d(), colour, mode: [2, 0, 0, 0] });
            cmds.push(Cmd::Road { layer, inst });
        }
        r.fill(device, queue, 0, &self.g, &recs, cmds);
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &egui_wgpu::CallbackResources) {
        if let Some(r) = res.get::<XrayRes>() {
            paint_slot(r, 0, pass);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xray_term_follows_the_zoom() {
        // (recorded gXrayTerm: 0.25 at 850 m, 0.2633 at 886.17, 0.4967 at 1520.3, 0.99967 at 2886.59, 1 above)
        assert!((xray_term(850.0) - 0.25).abs() < 1e-6);
        assert!((xray_term(886.17) - 0.2633).abs() < 1e-4);
        assert!((xray_term(1520.3) - 0.4967).abs() < 1e-4);
        assert!((xray_term(2886.59) - 0.99967).abs() < 1e-4);
        assert_eq!(xray_term(4633.0), 1.0);
        assert_eq!(xray_term(9000.0), 1.0);
    }

    #[test]
    fn planes_as_recorded() {
        // (near / far read off gWorldViewProj of the x-ray draws in 11 recordings)
        for (h, n, f) in [(850.0f32, 150.0f32, 2850.0f32), (886.17, 186.17, 2886.17), (1520.3, 820.3, 3520.3), (9000.0, 8300.0, 11000.0)] {
            let (a, b) = orig_planes(h);
            assert!((a - n).abs() < 0.01 && (b - f).abs() < 0.01, "{h}: {a} {b}");
        }
        assert_eq!(orig_planes(500.0).0, 0.2);
        // (k = f / (f - n) of the recorded matrices: 1.068952 at 886.17, 4.074074 at 9000)
        let (n, f) = orig_planes(886.17);
        assert!((f / (f - n) - 1.068952).abs() < 1e-5);
        let (n, f) = orig_planes(9000.0);
        assert!((f / (f - n) - 4.074074).abs() < 1e-5);
    }

    #[test]
    fn depth_rule() {
        let (n, f) = orig_planes(850.0);
        // a road 1500 m away: the scene 1 m in front of it hides it, 1 cm in front does not (the bias is ~11 cm there)
        let d = 1500.0;
        assert!(behind(orig_z(d, n, f), orig_z(d - 1.0, n, f)));
        assert!(!behind(orig_z(d, n, f), orig_z(d - 0.01, n, f)));
        assert!(!behind(orig_z(d, n, f), orig_z(d + 5.0, n, f)));
        // nothing drawn (the DepthMap's clear, 1 - 0): never behind
        assert!(!behind(orig_z(f, n, f), 1.0));
        // the bias in metres: d^2 eps / (k n)
        let k = f / (f - n);
        let tol = d * d * XRAY_EPS / (k * n);
        assert!(behind(orig_z(d, n, f), orig_z(d - tol * 1.2, n, f)));
        assert!(!behind(orig_z(d, n, f), orig_z(d - tol * 0.8, n, f)));
        assert_eq!(alpha(0.8, 0.5, false, 0.25), 0.4);
        assert_eq!(alpha(0.8, 0.5, true, 0.25), 0.8 * 0.25 * 0.5);
    }

    #[test]
    fn port_depth_gives_the_original_z() {
        // the port's reversed infinite depth (near / d) rebuilds the original's z at that distance
        let cam = crate::camera::Camera { pos: Vec3::new(0.0, 886.0, 0.0), yaw: 0.0, pitch: -0.6, fov_y: 0.6, near: 0.3 };
        let vp = cam.view_proj(16.0 / 9.0);
        let pn = port_near(&vp);
        assert!((pn - 0.3).abs() < 1e-6);
        let (n, f) = orig_planes(cam.pos.y);
        for p in [Vec3::new(10.0, 0.0, -1200.0), Vec3::new(-300.0, 120.0, -2000.0)] {
            let c = vp * p.extend(1.0);
            let z_port = c.z / c.w;
            let d = c.w;
            let a = orig_z(d, n, f);
            let b = orig_z_from_port(z_port, pn, n, f);
            assert!((a - b).abs() < 1e-5, "{a} {b}");
        }
        // nothing drawn in the port (depth 0) reads as beyond everything
        assert!(orig_z_from_port(0.0, pn, n, f) > 1.0);
    }

    #[test]
    fn mip_chain() {
        let px = [255u8, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 255, 255, 255, 255, 255];
        let m = mips(2, 2, &px);
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].2, vec![128, 128, 128, 191]);
    }
}
