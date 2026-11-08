//! The signal lamps' glows in the wet road's reflection picture (work/map2/siglrefl/signal_reflection_rule.md).
//!
//! Every lit signal lamp the main glow takes (F-79347 / F-39998, the prop manager's bucket 2 with
//! refl = 1) also adds a rectangle to the list at 0x8287E3B0 (F-44648, at most 96 a frame), drawn into the
//! reflection picture after its city and sky (F-50970, render/siglrefl.wgsl):
//!   - the lamp's place P (0.3 m behind its locator) mirrored about the horizontal plane through the head's
//!     origin, projected with the main view's matrix: NDC (x, y, z), z in the D3D 0 .. 1 form;
//!   - ny = max(normalize(C - P).y, 0), a = the main view's aspect;
//!   - zc = z >= 0.999 ? 0.999 : max(z, 0), s = (1 - zc) x 100, wx = s rsize.x, h = s a (1 - ny) rsize.y,
//!     rsize = sm_fSignalGlowReflectionSizes[weather] x 0.01;
//!   - x in [x - wx, x + wx], y in [y - 1.75 h, y + 0.25 h]; dropped when wholly off [-1, 1] in x or in y,
//!     or z > 1, or z < 1e-7;
//!   - at the draw: at least 1 / target width (height) across; uv (0, 0) at (x0, y0), (1, 1) at (x1, y1);
//!     strength I / [0x828393D4] = I / 4; the lamp's colour; the rectangle at depth z.

use crate::render::Gpu;
use glam::{Mat4, Vec3};
use std::sync::Mutex;

/// most rectangles a frame (F-44648: the count at 0x8287E360 against 96)
pub const MAX: usize = 96;
/// the width and height of the picture object the prop lamps' rectangles are sized by (F-28456: seed size x
/// 1 / its vfuncs 24 and 28, the object the reflection draw takes its target size from); read off the six prop rectangles of hl_L_23.5 (rsize
/// 0.3125 x 0.5556 = 100 / 320 x 100 / 180)
pub const PICTURE: (f32, f32) = (320.0, 180.0);
/// [0x828393D4], the integer the draw divides the strength by (recorded: StreakParams.y = I / 4)
pub const DIVISOR: f32 = 4.0;
/// the main view's far clip in the recordings (its projection's z column: near 0.2000, far 5994 .. 6000)
const FAR: f32 = 6000.0;

/// One rectangle as the shader takes it (siglrefl.wgsl `vs_siglrefl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Rect {
    /// x0 y0 x1 y1 in NDC
    pub r: [f32; 4],
    /// the port's depth of the rectangle, the strength (StreakParams.y), u at x0, v at y0
    pub z: [f32; 4],
    /// the vertex colour
    pub col: [f32; 4],
}

/// The rectangle of one lamp (F-44648 + the draw's part of F-50970).
/// `view_proj`: the main view's (the port's reverse-Z form); `near`: its near clip; `origin_y`: the
/// head's origin height; `p`, `i`, `rsize`: `halo::Tune::signal_reflection`; `cam`: the main camera;
/// `target`: the reflection picture's size in pixels.
pub fn rect(view_proj: Mat4, near: f32, origin_y: f32, p: Vec3, cam: Vec3, colour: [f32; 3], i: f32, rsize: [f32; 2], target: (f32, f32)) -> Option<Rect> {
    let m = Vec3::new(p.x, 2.0 * origin_y - p.y, p.z);
    let c = view_proj * m.extend(1.0);
    let w = c.w;
    if w == 0.0 {
        return None;
    }
    let (x, y) = (c.x / w, c.y / w);
    // the original's z (D3D form, near .. far -> 0 .. 1)
    let a_z = FAR / (FAR - near);
    let z = a_z * (1.0 - near / w);
    // the viewport's aspect: the projection's y scale over its x scale
    let (rx, ry) = (view_proj.row(0).truncate().length(), view_proj.row(1).truncate().length());
    let aspect = if rx > 0.0 { ry / rx } else { 1.0 };
    let ny = (cam - p).normalize_or_zero().y.max(0.0);
    let zc = if z >= 0.999 { 0.999 } else { z.max(0.0) };
    let s = (1.0 - zc) * 100.0;
    let wx = s * rsize[0];
    let h = s * aspect * (1.0 - ny) * rsize[1];
    let (x0, x1, y0, y1) = (x - wx, x + wx, y - 1.75 * h, y + 0.25 * h);
    if (x0 < -1.0 && x1 < -1.0) || (x1 > 1.0 && x0 > 1.0) || (y0 < -1.0 && y1 < -1.0) || (y1 > 1.0 && y0 > 1.0) || z > 1.0 || z < 1.0e-7 {
        return None;
    }
    let (r, u0, v0) = draw_rect([x0, y0, x1, y1], target);
    Some(Rect { r, z: [c.z / w, strength(i), u0, v0], col: { let c = crate::halo::pack(colour); /* (F-44648 takes the packed word) */ [c[0], c[1], c[2], 1.0] } })
}

/// The draw's part of a rectangle (F-50970): the corners put in order with the texture turned with them (u at x0, v at
/// y0 become 1 when swapped), then at least 1 / the target's width (height) across, x1 (y1) moved; the target's size
/// from the picture's vfuncs 24 / 28 as floats. `draw_rect_matches_the_original`.
pub fn draw_rect(r: [f32; 4], target: (f32, f32)) -> ([f32; 4], f32, f32) {
    let [mut x0, mut y0, mut x1, mut y1] = r;
    let (mut u0, mut v0) = (0.0, 0.0);
    if x0 > x1 {
        (x0, x1, u0) = (x1, x0, 1.0);
    }
    if y0 > y1 {
        (y0, y1, v0) = (y1, y0, 1.0);
    }
    let (mw, mh) = (1.0 / target.0, 1.0 / target.1);
    if x1 - x0 < mw {
        x1 = x0 + mw;
    }
    if y1 - y0 < mh {
        y1 = y0 + mh;
    }
    ([x0, y0, x1, y1], u0, v0)
}

/// The strength the draw sends: the lamp's I times 1 / [0x828393D4] (the integer 4 as a float).
pub fn strength(i: f32) -> f32 {
    i * (1.0 / DIVISOR)
}

struct State {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    buf: wgpu::Buffer,
    count: u32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// textures/lightglow.dds (128 x 128 DXT1, one level; the fetch constant: no gamma, clamp, bilinear)
fn glow_texture(gpu: &Gpu, data_dir: &str) -> wgpu::TextureView {
    let p = std::path::Path::new(data_dir).join("textures/lightglow.dds");
    let (w, h, px) = match std::fs::read(&p).ok().and_then(|d| mcla_formats::dds::parse_dds("lightglow.dds", &d).ok()) {
        Some(t) if t.width > 0 && t.height > 0 && !t.levels.is_empty() => (t.width, t.height, t.to_rgba8()),
        _ => {
            log::warn!("lightglow.dds missing: no signal glows in the road's reflection");
            (1, 1, vec![0, 0, 0, 0])
        }
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("light glow"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &px[..(w * h * 4) as usize],
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    texture.create_view(&Default::default())
}

/// Once, with the city's data.
pub fn load(gpu: &Gpu, data_dir: &str) {
    let view = glow_texture(gpu, data_dir);
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("siglrefl layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("siglrefl sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("siglrefl"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("siglrefl"), source: wgpu::ShaderSource::Wgsl(include_str!("render/siglrefl.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("siglrefl pl"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
    const ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("siglrefl"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_siglrefl"),
            buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Rect>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTRS }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_siglrefl"),
            // (blend one / one, the original's 0x01010101)
            targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        // (0x00708763: the depth test on, no writes; GEQUAL on reversed depth, as the reflection pass's own)
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: false, depth_compare: wgpu::CompareFunction::GreaterEqual, stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("siglrefl rects"), size: (MAX * std::mem::size_of::<Rect>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    *STATE.lock().unwrap() = Some(State { pipeline, bind, buf, count: 0 });
}

/// This frame's rectangles (at most `MAX`, the first ones kept).
pub fn set(gpu: &Gpu, rects: &[Rect]) {
    if let Some(s) = STATE.lock().unwrap().as_mut() {
        let n = rects.len().min(MAX);
        if n > 0 {
            gpu.queue.write_buffer(&s.buf, 0, bytemuck::cast_slice(&rects[..n]));
        }
        s.count = n as u32;
    }
}

/// Inside the reflection pass, after its city and sky (`Gpu::reflection_pass`). Sets its own pipeline.
pub fn draw(pass: &mut wgpu::RenderPass<'_>) {
    if std::env::var_os("MCLA_NO_SIGLREFL").is_some() {
        return;
    }
    let guard = STATE.lock().unwrap();
    if let Some(s) = guard.as_ref().filter(|s| s.count > 0) {
        pass.set_pipeline(&s.pipeline);
        pass.set_bind_group(0, &s.bind, &[]);
        pass.set_vertex_buffer(0, s.buf.slice(..));
        pass.draw(0..6, 0..s.count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_rect_matches_the_original() {
        // tools/rect_draw_reference.py: F-50970 run offline, 600 lists, 12,995 records, 8,047 draws
        let t = include_str!("testdata/rect_draw_reference.txt");
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut lines = t.lines().peekable();
        let (mut lists, mut draws) = (0, 0);
        while let Some(l) = lines.next() {
            let c: Vec<u32> = l.split_whitespace().skip(1).map(|x| x.parse().unwrap()).collect();
            let target = (c[0] as f32, c[1] as f32);
            let recs: Vec<Vec<u32>> = (0..c[2]).map(|_| lines.next().unwrap().split_whitespace().skip(1).map(h).collect()).collect();
            for q in &recs {
                let f = |i: usize| f32::from_bits(q[i]);
                // (a record is drawn when its +24 is above 0)
                if !(f(0) > 0.0) {
                    continue;
                }
                let d: Vec<u32> = lines.next().unwrap().strip_prefix("DRAW ").unwrap().split_whitespace().map(h).collect();
                let (r, u0, v0) = draw_rect([f(1), f(2), f(3), f(4)], target);
                let want = [r[0], r[1], r[2], r[3], f(0), u0, v0, 1.0 - u0, 1.0 - v0];
                for (k, w) in want.iter().enumerate() {
                    assert_eq!(w.to_bits(), d[k], "{l} word {k}");
                }
                assert_eq!((d[9], d[10], d[11]), (q[5], 0x7E57, strength(f(6)).to_bits()), "{l}");
                draws += 1;
            }
            assert_eq!(lines.next(), Some("END 0"));
            lists += 1;
        }
        assert_eq!((lists, draws), (600, 8047));
    }

    #[test]
    fn the_rectangle_as_decoded() {
        // a camera at the origin looking down -z, fov 25 deg, 16:9, near 0.2; a lamp 100 m ahead,
        // 4 m above a head origin at y = -1.5 (its mirror image 5.5 m under that plane: y = -9.5)
        let near = 0.2;
        let f = 1.0 / (12.5f32.to_radians()).tan();
        let proj = Mat4::from_cols_array(&[f / (16.0 / 9.0), 0.0, 0.0, 0.0, 0.0, f, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, near, 0.0]);
        let p = Vec3::new(0.0, 2.5, -100.0);
        let r = rect(proj, near, -1.5, p, Vec3::ZERO, [1.0, 0.0, 0.0], 4.52, [0.05, 0.05], (256.0, 256.0)).unwrap();
        let y = f * -5.5 / 100.0;
        let z = 6000.0 / 5999.8 * (1.0 - 0.2 / 100.0);
        let s = (1.0 - z) * 100.0;
        let ny = (-p).normalize().y.max(0.0);
        let h = s * (16.0 / 9.0) * (1.0 - ny) * 0.05;
        assert!((r.r[0] + s * 0.05).abs() < 1e-5 && (r.r[2] - s * 0.05).abs() < 1e-5, "{:?}", r);
        assert!((r.r[1] - (y - 1.75 * h)).abs() < 1e-5 && (r.r[3] - (y + 0.25 * h)).abs() < 1e-5, "{:?}", r);
        assert!((r.z[0] - 0.002).abs() < 1e-6 && (r.z[1] - 1.13).abs() < 1e-6 && r.z[2] == 0.0 && r.z[3] == 0.0);
        // behind the camera: none
        assert!(rect(proj, near, -1.5, Vec3::new(0.0, 2.5, 100.0), Vec3::ZERO, [1.0; 3], 4.52, [0.05, 0.05], (256.0, 256.0)).is_none());
    }

    #[test]
    fn the_rectangles_of_the_recording() {
        // work/lighting/cap/hl_L_12 (Selma / Highland from the gate, noon, nice): the main view's matrix (c8 .. c11 of
        // the reflection pass's VSCityLODReflect draws, row vectors; its z column replaced by the port's reverse-Z
        // form, near 0.2), the camera (c15), the two red lamps' places and their head's origin height as the port
        // has them; against the vertices of PSSeedRTNoZ draws 133 / 134 read from the running game
        // (x0 y0 x1 y1 z) and their StreakParams.y.
        let m = [
            [-1.8506852f32, 0.20796002, 0.6825558, 0.682533],
            [-0.021566208, 4.4973507, -0.07643374, -0.07643119],
            [1.7355853, 0.27763507, 0.7268715, 0.72684723],
            [2920.2622, -31.040251, 38.12732, 38.32605],
        ];
        let col = |i: usize| glam::Vec4::new(m[i][0], m[i][1], if i == 3 { 0.2 } else { 0.0 }, m[i][3]);
        let vp = Mat4::from_cols(col(0), col(1), col(2), col(3));
        let cam = Vec3::new(813.6567, 19.573135, -814.7209);
        let t = crate::halo::Tune::default();
        for (loc, i_rec, rec) in [
            (Vec3::new(916.730, 23.031, -739.832), 1.10225f32, [-0.48896f32, -0.13994, -0.47339, -0.11226, 0.99844]),
            (Vec3::new(912.581, 19.107, -730.923), 1.08126, [-0.2977, 0.01298, -0.28253, 0.03984, 0.99848]),
        ] {
            // (the dumped places are already 0.3 m behind the locator: hand rect() P directly)
            let r = rect(vp, 0.2, 14.990, loc, cam, [1.0, 0.0, 0.0], i_rec * DIVISOR, t.signal_refl[0].map(|v| v * 0.01), (256.0, 256.0)).unwrap();
            for k in 0..4 {
                assert!((r.r[k] - rec[k]).abs() < 5e-5, "{k}: {:?} against {:?}", r.r, rec);
            }
            let z = 6000.0 / (6000.0 - 0.2) * (1.0 - r.z[0]);
            assert!((z - rec[4]).abs() < 2e-5, "z {z}");
            assert!((r.z[1] - i_rec).abs() < 1e-6);
        }
    }
}
