//! The top GPS arrow's picture (ARROW: TOP) made on the GPU, by the CPU picture's own arithmetic
//! (`mc4::top_arrow_picture`), into the egui texture the CPU's picture went into - so the arrow can follow the drawn
//! pose every frame at next to no CPU cost (the CPU picture took 1.46 ms at 1440 lines; docs/UI.md 2026-11-09).
//!
//! The CPU still sets the faces up (`mc4::top_arrow_raster`: the corners lit, placed on the 3 x 3 sample grid, the box
//! the arrow can reach); one compute thread a pixel then does what the CPU's band loop does for that pixel's nine
//! samples - every face in its order, the same edge test (all three weights >= 0, so a sample on an edge belongs to
//! both faces), the same 1 / depth comparison (an equal one keeps the earlier face), the same perspective weights,
//! clamp and sums in the same order - and rounds to 8 bits as the CPU does. Every product, sum and difference goes
//! through an fma the compiler cannot fold or reorder, every division is rounded to nearest (see the shader), so the
//! bytes are the CPU's on any driver (test `top_arrow_gpu_matches_the_cpu_picture`: Vulkan and DX12, 0 of 204 M
//! channels differ). A hardware raster snaps corners to its sub-pixel grid and owns an edge's samples on one side
//! only (585 channels of 72 pictures differed), so it is not used. Then the bytes are premultiplied as egui's
//! `Color32::from_rgba_unmultiplied` does it, by its own 64 K table uploaded here: what egui would have uploaded.
//! `MCLA_TOPARROW_CPU=1`: the CPU's picture.

use std::sync::Mutex;

use crate::mc4::{TopArrowFace, TopArrowRaster};

pub fn enabled() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_TOPARROW_CPU").map_or(true, |v| v.trim() != "1"))
}

pub struct Job {
    pub size: u32,
    /// None: nothing of the arrow in the picture
    pub raster: Option<TopArrowRaster>,
    pub tex: egui::TextureId,
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());

/// Ask for the picture of this frame (the last one asked wins).
pub fn queue(job: Job) {
    if let Ok(mut j) = JOBS.lock() {
        j.clear();
        j.push(job);
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    /// px0, py0, px1, py1
    bx: [u32; 4],
    /// size, faces, bw, 0
    n: [u32; 4],
}

const OUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Target {
    size: u32,
    out: wgpu::Texture,
    view: wgpu::TextureView,
}

pub struct State {
    pipe: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    lut: wgpu::Buffer,
    target: Option<Target>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// egui's premultiply table (`Color32::from_rgba_unmultiplied` for 0 < a < 255): entry a x 256 + value, one byte each,
/// four to a word.
fn lut_words() -> Vec<u32> {
    let mut b = vec![0u8; 65536];
    for a in 0..=255u32 {
        for v in 0..=255u32 {
            b[(a * 256 + v) as usize] = egui::Color32::from_rgba_unmultiplied(v as u8, v as u8, v as u8, a as u8).r();
        }
    }
    b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

impl State {
    pub fn new(device: &wgpu::Device) -> State {
        use wgpu::util::DeviceExt;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("top arrow"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let buf = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("top arrow"),
            entries: &[
                buf(0, wgpu::BufferBindingType::Uniform),
                buf(1, ro),
                buf(2, ro),
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: OUT, view_dimension: wgpu::TextureViewDimension::D2 }, count: None },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("top arrow"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("top arrow"), layout: Some(&pl), module: &shader, entry_point: Some("main"), compilation_options: Default::default(), cache: crate::render::pcache::get() });
        let lut = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("top arrow egui premultiply"), contents: bytemuck::cast_slice(&lut_words()), usage: wgpu::BufferUsages::STORAGE });
        State { pipe, layout, lut, target: None }
    }

    /// Make the picture (`size` x `size`, the bytes egui would hold) and return the texture it is in.
    pub fn picture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: u32, raster: Option<&TopArrowRaster>) -> &wgpu::Texture {
        use wgpu::util::DeviceExt;
        if self.target.as_ref().map_or(true, |t| t.size != size) {
            let out = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("top arrow"),
                size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: OUT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.target = Some(Target { size, view: out.create_view(&Default::default()), out });
        }
        let t = self.target.as_ref().unwrap();
        // (no arrow: an empty box, every pixel written 0)
        let empty = TopArrowFace { x: [0.0; 3], y: [0.0; 3], area: 1.0, inv: [0.0; 3], rgb: [[0.0; 3]; 3], alpha: [0.0; 3], sx: [0; 2], sy: [0; 2] };
        let (bx, faces, bw): ([u32; 4], &[TopArrowFace], u32) = match raster {
            Some(r) if !r.faces.is_empty() => ([r.px0 as u32, r.py0 as u32, r.px1 as u32, r.py1 as u32], &r.faces, r.bw as u32),
            _ => ([0; 4], std::slice::from_ref(&empty), 0),
        };
        let n = if bw == 0 { 0 } else { faces.len() as u32 };
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("top arrow"), contents: bytemuck::bytes_of(&Params { bx, n: [size, n, bw, 0] }), usage: wgpu::BufferUsages::UNIFORM });
        let fb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("top arrow faces"), contents: bytemuck::cast_slice(faces), usage: wgpu::BufferUsages::STORAGE });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("top arrow"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.lut.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&t.view) },
            ],
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("top arrow") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("top arrow"), timestamp_writes: None });
            pass.set_pipeline(&self.pipe);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 1);
        }
        queue.submit([enc.finish()]);
        &t.out
    }
}

/// Make the pipeline now (at start-up).
pub fn init(device: &wgpu::Device) {
    if let Ok(mut st) = STATE.lock() {
        st.get_or_insert_with(|| State::new(device));
    }
}

/// Make this frame's arrow picture (if one was asked for) and copy it into its egui texture in each of the UI's renderers.
pub fn render(device: &wgpu::Device, queue: &wgpu::Queue, renderers: [&egui_wgpu::Renderer; 3]) {
    let Some(job) = JOBS.lock().ok().and_then(|mut j| j.pop()) else { return };
    let Ok(mut st) = STATE.lock() else { return };
    let st = st.get_or_insert_with(|| State::new(device));
    let out = st.picture(device, queue, job.size, job.raster.as_ref());
    let ext = wgpu::Extent3d { width: job.size, height: job.size, depth_or_array_layers: 1 };
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("top arrow copy") });
    for r in renderers {
        if let Some(dst) = r.texture(&job.tex).and_then(|x| x.texture.as_ref()) {
            if dst.width() == job.size && dst.height() == job.size {
                enc.copy_texture_to_texture(out.as_image_copy(), dst.as_image_copy(), ext);
            }
        }
    }
    queue.submit([enc.finish()]);
}

const SHADER: &str = r#"
struct Params {
    bx: vec4<u32>,
    n: vec4<u32>,
};
struct Face {
    x0: f32, x1: f32, x2: f32,
    y0: f32, y1: f32, y2: f32,
    area: f32,
    ia: f32, ib: f32, ic: f32,
    r0: f32, g0: f32, b0: f32,
    r1: f32, g1: f32, b1: f32,
    r2: f32, g2: f32, b2: f32,
    a0: f32, a1: f32, a2: f32,
    sx0: u32, sx1: u32, sy0: u32, sy1: u32,
};
@group(0) @binding(0) var<uniform> prm: Params;
@group(0) @binding(1) var<storage, read> faces: array<Face>;
@group(0) @binding(2) var<storage, read> lut: array<u32>;
@group(0) @binding(3) var outp: texture_storage_2d<rgba8unorm, write>;

// (the CPU's IEEE arithmetic: a product rounded on its own - never fused into an add - and a division rounded to
// nearest; the GPU's own division is within a few ulp, the correction picks the neighbour with the least remainder)
// (the zero comes from the uniform, so the compiler cannot fold the fma back into a product it may fuse)
fn mul(a: f32, b: f32) -> f32 {
    return fma(a, b, bitcast<f32>(prm.n.w));
}
// (a sum and a difference rounded once, in the order written: by an fma with a 1 from the uniform, which the compiler
// can neither reorder with the next operation nor fuse; HLSL and the Vulkan drivers may otherwise reassociate)
fn add(a: f32, b: f32) -> f32 {
    return fma(a, bitcast<f32>(prm.n.w ^ 0x3f800000u), b);
}
fn sub(a: f32, b: f32) -> f32 {
    return fma(a, bitcast<f32>(prm.n.w ^ 0x3f800000u), -b);
}
// (a - q b, exactly enough to rank the candidates: q b split by Dekker into its rounded product and the error of
// it, from products and sums rounded once each - right whether the compiler's fma is fused or not (HLSL's mad))
fn split(a: f32) -> vec2<f32> {
    let c = mul(4097.0, a);
    let hi = sub(c, sub(c, a));
    return vec2<f32>(hi, sub(a, hi));
}
fn rest(a: f32, q: f32, b: f32) -> f32 {
    let p = mul(q, b);
    let x = split(q);
    let y = split(b);
    let e = add(add(add(sub(mul(x.x, y.x), p), mul(x.x, y.y)), mul(x.y, y.x)), mul(x.y, y.y));
    return abs(sub(sub(a, p), e));
}
fn dv(a: f32, b: f32) -> f32 {
    let q = a / b;
    var best = q;
    var err = rest(a, q, b);
    for (var k = 0; k < 4; k++) {
        let up = bitcast<f32>(bitcast<u32>(best) + 1u);
        let dn = bitcast<f32>(bitcast<u32>(best) - 1u);
        let eu = rest(a, up, b);
        let ed = rest(a, dn, b);
        if (eu < err) {
            best = up;
            err = eu;
        } else if (ed < err) {
            best = dn;
            err = ed;
        }
    }
    return best;
}

fn premul(v: u32, a: u32) -> u32 {
    let i = a * 256u + v;
    return (lut[i >> 2u] >> ((i & 3u) * 8u)) & 255u;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = prm.n.x;
    if (id.x >= size || id.y >= size) {
        return;
    }
    let px = id.x;
    let py = id.y;
    if (prm.n.y == 0u || px < prm.bx.x || px >= prm.bx.z || py < prm.bx.y || py >= prm.bx.w) {
        textureStore(outp, vec2<i32>(id.xy), vec4<f32>(0.0));
        return;
    }
    // (this pixel's nine samples, on the box's sample grid; per sample 1 / depth of the nearest face so far, its colour)
    let qx = (px - prm.bx.x) * 3u;
    let qy = (py - prm.bx.y) * 3u;
    var near: array<f32, 9>;
    var col: array<vec4<f32>, 9>;
    for (var k = 0; k < 9; k++) {
        near[k] = 0.0;
        col[k] = vec4<f32>(0.0);
    }
    for (var fi = 0u; fi < prm.n.y; fi++) {
        let f = faces[fi];
        let ys0 = max(f.sy0, qy);
        let ys1 = min(f.sy1, qy + 3u);
        let xs0 = max(f.sx0, qx);
        let xs1 = min(f.sx1, qx + 3u);
        for (var y = ys0; y < ys1; y++) {
            let gy = add(f32(y), 0.5);
            for (var x = xs0; x < xs1; x++) {
                let gx = add(f32(x), 0.5);
                let w0 = dv(sub(mul(sub(f.x1, gx), sub(f.y2, gy)), mul(sub(f.x2, gx), sub(f.y1, gy))), f.area);
                let w1 = dv(sub(mul(sub(f.x2, gx), sub(f.y0, gy)), mul(sub(f.x0, gx), sub(f.y2, gy))), f.area);
                let w2 = sub(sub(1.0, w0), w1);
                if (w0 < 0.0 || w1 < 0.0 || w2 < 0.0) {
                    continue;
                }
                let inv = add(add(mul(w0, f.ia), mul(w1, f.ib)), mul(w2, f.ic));
                let at = (y - qy) * 3u + (x - qx);
                if (inv <= near[at]) {
                    continue;
                }
                near[at] = inv;
                let p0 = dv(mul(w0, f.ia), inv);
                let p1 = dv(mul(w1, f.ib), inv);
                let p2 = dv(mul(w2, f.ic), inv);
                let rgb = clamp(vec3<f32>(
                    add(add(mul(f.r0, p0), mul(f.r1, p1)), mul(f.r2, p2)),
                    add(add(mul(f.g0, p0), mul(f.g1, p1)), mul(f.g2, p2)),
                    add(add(mul(f.b0, p0), mul(f.b1, p1)), mul(f.b2, p2))), vec3<f32>(0.0), vec3<f32>(1.0));
                col[at] = vec4<f32>(rgb, clamp(add(add(mul(f.a0, p0), mul(f.a1, p1)), mul(f.a2, p2)), 0.0, 1.0));
            }
        }
    }
    var sum = vec4<f32>(0.0);
    var hit = 0.0;
    for (var k = 0; k < 9; k++) {
        if (near[k] > 0.0) {
            hit = add(hit, 1.0);
            sum = vec4<f32>(add(sum.x, col[k].x), add(sum.y, col[k].y), add(sum.z, col[k].z), add(sum.w, col[k].w));
        }
    }
    if (hit == 0.0) {
        textureStore(outp, vec2<i32>(id.xy), vec4<f32>(0.0));
        return;
    }
    let r = u32(add(mul(dv(sum.x, hit), 255.0), 0.5));
    let g = u32(add(mul(dv(sum.y, hit), 255.0), 0.5));
    let b = u32(add(mul(dv(sum.z, hit), 255.0), 0.5));
    let a = u32(add(mul(dv(sum.w, 9.0), 255.0), 0.5));
    var o = vec4<f32>(0.0);
    if (a == 255u) {
        o = vec4<f32>(f32(r), f32(g), f32(b), 255.0) / 255.0;
    } else if (a != 0u) {
        o = vec4<f32>(f32(premul(r, a)), f32(premul(g, a)), f32(premul(b, a)), f32(a)) / 255.0;
    }
    textureStore(outp, vec2<i32>(id.xy), o);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The GPU picture against the CPU's (`mc4::top_arrow_picture` premultiplied by egui), byte for byte, on each of
    /// the machine's Vulkan and DX12 adapters (DX12 with the game's DXC), at the sizes of 720, 1080, 1440 and 2160
    /// lines, for 48 directions all round the car, level, up and down; and the CPU time of the two paths.
    /// (cargo test --release top_arrow_gpu -- --ignored --nocapture; needs a GPU and MCLA_DATA_DIR)
    #[test]
    #[ignore = "requires a GPU and MCLA_DATA_DIR with the extracted disc"]
    fn top_arrow_gpu_matches_the_cpu_picture() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let t = crate::mc4::TopArrowTune::load(&dir);
        let m = crate::mc4::top_arrow_model(&dir).expect("model");
        let mut cases = Vec::new();
        for lines in [720.0f32, 1080.0, 1440.0, 2160.0] {
            let r = crate::mc4::top_arrow_rect(&t, lines * 16.0 / 9.0, lines);
            let size = (r[2].max(r[3]) as usize).clamp(16, 2048);
            for k in 0..48 {
                let a = k as f32 * std::f32::consts::TAU / 48.0 + 0.1;
                let local = [a.sin() * 300.0, [-40.0, 0.0, 25.0][k % 3], -a.cos() * 300.0];
                let c0 = std::time::Instant::now();
                let cpu = egui::ColorImage::from_rgba_unmultiplied([size, size], &crate::mc4::top_arrow_picture(&t, m, size, local));
                let cpu_ms = c0.elapsed().as_secs_f64() * 1000.0;
                let g0 = std::time::Instant::now();
                let raster = crate::mc4::top_arrow_raster(&t, m, size, local);
                let gpu_ms = g0.elapsed().as_secs_f64() * 1000.0;
                cases.push((lines, size, cpu, raster, cpu_ms, gpu_ms));
            }
        }
        for lines in [720.0f32, 1080.0, 1440.0, 2160.0] {
            let c: Vec<_> = cases.iter().filter(|c| c.0 == lines).collect();
            eprintln!("{lines} lines ({} px): CPU picture {:.3} ms, the GPU path's CPU part {:.4} ms a picture", c[0].1, c.iter().map(|c| c.4).sum::<f64>() / c.len() as f64, c.iter().map(|c| c.5).sum::<f64>() / c.len() as f64);
        }
        let mut backends = 0;
        for backend in [wgpu::Backends::VULKAN, wgpu::Backends::DX12] {
            let got = pollster::block_on(async {
                let inst = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                    backends: backend,
                    backend_options: wgpu::BackendOptions { dx12: wgpu::Dx12BackendOptions { shader_compiler: wgpu::Dx12Compiler::StaticDxc }, ..Default::default() },
                    ..Default::default()
                });
                let ad = inst.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }).await?;
                let name = format!("{:?} {}", ad.get_info().backend, ad.get_info().name);
                ad.request_device(&wgpu::DeviceDescriptor::default(), None).await.ok().map(|(d, q)| (name, d, q))
            });
            let Some((name, dev, q)) = got else {
                eprintln!("{backend:?}: no adapter");
                continue;
            };
            backends += 1;
            let mut st = State::new(&dev);
            let (mut channels, mut diff, mut worst) = (0usize, 0usize, 0i32);
            let mut gpu_ms = 0.0f64;
            for (_, size, cpu, raster, _, _) in &cases {
                let g0 = std::time::Instant::now();
                let tex = st.picture(&dev, &q, *size as u32, raster.as_ref());
                let _ = dev.poll(wgpu::Maintain::Wait);
                gpu_ms += g0.elapsed().as_secs_f64() * 1000.0;
                let gpu = read(&dev, &q, tex, *size as u32);
                for (p, c) in cpu.pixels.iter().enumerate() {
                    let c = c.to_array();
                    for ch in 0..4 {
                        let d = (gpu[p * 4 + ch] as i32 - c[ch] as i32).abs();
                        worst = worst.max(d);
                        diff += (d > 0) as usize;
                    }
                }
                channels += size * size * 4;
            }
            eprintln!("{name}: {} pictures, {channels} channels, {diff} differ (worst {worst}); upload + compute + wait {:.3} ms a picture", cases.len(), gpu_ms / cases.len() as f64);
            assert_eq!(diff, 0, "{name}: the GPU picture differs from the CPU's");
        }
        assert!(backends > 0, "no GPU");
    }

    fn read(dev: &wgpu::Device, q: &wgpu::Queue, tex: &wgpu::Texture, size: u32) -> Vec<u8> {
        let row = (size * 4).div_ceil(256) * 256;
        let buf = dev.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * size) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = dev.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size) } },
            wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        );
        q.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = dev.poll(wgpu::Maintain::Wait);
        let data = buf.slice(..).get_mapped_range();
        data.chunks(row as usize).flat_map(|r| r[..(size * 4) as usize].iter().copied()).collect()
    }
}
