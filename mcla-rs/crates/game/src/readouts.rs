//! The cockpit readouts: speed, odometer, trip, gear and clock on the dashboard, drawn only in the in-car view
//! (the car draw F-73214 calls F-43500 while the car's interior-view flag is set). Rule and evidence:
//! docs/reference/carparts/cockpit_readouts_rule.md.
//!
//! - Places (F-10977): the bones `fontspeedo0`, `fontodo0`, `fonttrip0`, `fontgear0`, `fontclock0` of the base fragment's
//!   skeleton or a fitted part's, the first found; the box = the bone's translation limits (max - min of x and y).
//! - Fonts (F-11553): `$/vehicle/shared_font` fnt_digital_block / fnt_license_plate / fnt_digital_dots / fnt_odometer,
//!   by the tune's font index; 8 x 8 cells of 32 pixels.
//! - The strip (F-57755): 16 quads, glyph k over x [(k - 1) 0.1, k 0.1], y [-0.1, 0]; FontDraw.fx (cars 0x8146788c):
//!   letterSize 0.125, numLettersX = numLettersY = 8 (the effect's defaults), blend SRCALPHA / INVSRCALPHA.
//! - Numbers (F-17778 -> F-27249, the Roller technique) and strings (F-73437 -> F-61840, the Font technique).

use glam::{Mat4, Vec3, Vec4};
use std::sync::Mutex;

use crate::render::Gpu;

/// The readouts' bone names, in F-10977's order.
pub const BONES: [&str; 5] = ["fontspeedo0", "fontodo0", "fonttrip0", "fontgear0", "fontclock0"];

/// The four fonts by the tune's index (F-11553's table).
pub const FONTS: [&str; 4] = ["fnt_digital_block", "fnt_license_plate", "fnt_digital_dots", "fnt_odometer"];

/// The odometer drum's font index: with it the odometer and the trip are drawn as numbers on the drum (F-43500).
pub const DRUM_FONT: usize = 3;

/// The gear names by the gearbox's index, 0 = reverse, 1 = neutral (F-43500's table).
pub const GEARS: [&str; 10] = ["R", "N", "1", "2", "3", "4", "5", "6", "7", "8"];

/// The string draw's cell by character '0' ..= '^' (F-61840's table, read from the executable); others 63.
const CHARS: [u8; 47] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 36, 37, 38, 39, 63, 63, 63, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39];

const STEP: f32 = 0.1;
const GLYPHS: usize = 16;

/// The trip's start (the car object's +4640), set by the restore F-11744 to the vehicle's distance stat; a new car
/// object starts at 0. Held per car uid.
static TRIP: Mutex<(u32, f32)> = Mutex::new((u32::MAX, 0.0));

pub fn set_trip_start(uid: u32, miles: f32) {
    *TRIP.lock().unwrap() = (uid, miles);
}

pub fn trip_start(uid: u32) -> f32 {
    let t = TRIP.lock().unwrap();
    if t.0 == uid { t.1 } else { 0.0 }
}

pub fn bone_kind(name: &str) -> Option<u8> {
    BONES.iter().position(|b| name.eq_ignore_ascii_case(b)).map(|k| k as u8)
}

/// F-61840: the cells of a string (upper-cased; past the 16th character none), and its length.
pub fn string_glyphs(s: &str) -> ([f32; GLYPHS], usize) {
    let mut out = [0.0; GLYPHS];
    for (i, c) in s.bytes().take(GLYPHS).enumerate() {
        let c = c.to_ascii_uppercase();
        out[i] = if (48..=94).contains(&c) { CHARS[(c - 48) as usize] as f32 } else { 63.0 };
    }
    (out, s.len())
}

/// F-17778: a number's drum values, rightmost digit last: v = value x 10; with `tenths` the rightmost cell is the tenths'
/// digit plus the fraction of v, without it the tenths are not stored at all (the cell pointer steps only on a store) and
/// the rightmost cell is the ones' digit; each digit takes, while the digit to its right's fraction and the tenths' share
/// are both at least 0.9, (share - 0.9) x 10 - the roll in the last tenth. With `tenths` the last of the count + 1 stores
/// falls before the 16 and is dropped. Bit for bit the original's (test drum_values_match_the_original).
pub fn drum_values(value: f32, count: usize, tenths: bool) -> [f32; GLYPHS] {
    let mut out = [0.0f32; GLYPHS];
    let mut cur = value * 10.0;
    let frac0 = (cur as f64 % 1.0) as f32;
    let share = ((cur as f64 % 10.0) as f32) * STEP;
    let mut at = 0usize;
    for i in 0..=count {
        let digit = ((cur as f64 % 10.0) as f32).trunc();
        let m = (cur as f64 % 1.0) as f32;
        let mut x = if i == 0 {
            frac0
        } else if m >= 0.9 && share >= 0.9 {
            (share - 0.9) * 10.0
        } else {
            0.0
        };
        cur *= STEP;
        if i != 0 || tenths {
            x += digit;
            // (the store and the pointer's step, r29 -= 4, together)
            if at < count && count <= GLYPHS {
                out[count - 1 - at] = x;
            }
            at += 1;
        }
    }
    out
}

/// F-27249: a drum value to its cell of the odometer font (0 .. 7 -> 8 .. 15, 8 .. 9 -> 24 .. 25, the fraction kept);
/// out of range 63; past `count` 0.
pub fn drum_cells(values: &[f32; GLYPHS], count: usize) -> [f32; GLYPHS] {
    let mut out = [0.0; GLYPHS];
    for i in 0..count.min(GLYPHS) {
        let v = values[i];
        out[i] = if !(0.0..10.0).contains(&v) {
            63.0
        } else if v >= 24.0 {
            v + 32.0
        } else if v >= 16.0 {
            v + 24.0
        } else if v >= 8.0 {
            v + 16.0
        } else {
            v + 8.0
        };
    }
    out
}

/// VS_Font's / VS_Roller's cell origin in the atlas for a letter value: (col, row) / 8, the Roller's fraction
/// moving the cell up by its share of a cell.
pub fn cell_uv(letter: f32, roller: bool) -> [f32; 2] {
    let (l, frac) = if roller { (letter.trunc(), letter - letter.trunc()) } else { (letter, 0.0) };
    let q = l / 8.0;
    let row = q.trunc();
    [q - row, row / 8.0 - frac / 8.0]
}

/// One readout as F-43500 hands it to the draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Text {
    pub cells: [f32; GLYPHS],
    pub count: usize,
    pub roller: bool,
    pub font: usize,
}

/// The values the readouts show, F-43500: speed (m/s) x 2.237 as "%d"; odometer (miles: the vehicle's distance stat)
/// as "%.6d" or on the drum (6 digits); trip = odometer - the distance at the last restore (F-11744), "%d" of it x 10
/// or on the drum with its tenths; gear by the table; the clock "%d %.2d" in 12 hours.
pub fn texts(fonts: [usize; 5], speed: f32, odo: f32, trip_start: f32, gear: i32, hours: f32) -> [Option<Text>; 5] {
    let s = |t: String, font: usize| {
        let (cells, count) = string_glyphs(&t);
        Some(Text { cells, count, roller: false, font })
    };
    let drum = |v: f32, tenths: bool| Some(Text { cells: drum_cells(&drum_values(v, 6, tenths), 6), count: 6, roller: true, font: DRUM_FONT });
    let speed = s(format!("{}", (speed * 2.237) as i32).chars().take(7).collect(), fonts[0]);
    let odo_t = if fonts[1] == DRUM_FONT { drum(odo, false) } else { s(format!("{:06}", odo as i32), fonts[1]) };
    let trip = odo - trip_start;
    let trip_t = if fonts[2] == DRUM_FONT { drum((trip * 10.0) * STEP, true) } else { s(format!("{}", (trip * 10.0) as i32).chars().take(8).collect(), fonts[2]) };
    let gear_t = usize::try_from(gear).ok().and_then(|g| GEARS.get(g)).and_then(|g| s(g.to_string(), fonts[3]));
    let h = hours as i32;
    let m = ((hours - h as f32) * 60.0) as i32;
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    let clock = s(format!("{} {:02}", h12, m), fonts[4]);
    [speed, odo_t, trip_t, gear_t, clock]
}

/// The strip's matrix rows in the model's frame (F-43500, then the draw's right alignment): x = the bone's x x w x 10,
/// y = its y x h x 10 + skew x x; the place moved by x x skew x 0.05, then left by x x (count - 1) x 0.1.
pub fn rows(bone: Mat4, w: f32, h: f32, skew: f32, count: usize) -> [Vec3; 3] {
    let x = bone.x_axis.truncate() * (w * 10.0);
    let y = x * skew + bone.y_axis.truncate() * (h * 10.0);
    let p = x * skew * STEP * 0.5 + bone.w_axis.truncate();
    let p = p - x * ((count as f32 - 1.0) * STEP);
    [x, y, p]
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub emissive: [f32; 3],
    /// the depth offset (render state 12) the strip is drawn with - `set_depth_offset`
    pub depth: f32,
    /// the strip matrix's z axis in the world (VS_Font's normal: gWorld's third row, normalised in the program)
    pub normal: [f32; 3],
    /// the player car's fill (ambYPos: the sky's ambient + rgb, x w when w > 0 - fillmap::player_now)
    pub fill: [f32; 4],
}

/// The in-car view's depth offset at the readouts: the car draw F-73214 runs them right after the part loop F-92759, which
/// set render state 12 to G+5700 = 0.275 for a car that is no motorcycle (F-78985), with no state 12 between - so the strip
/// is drawn 0.275 nearer, as the dash under it. Set by vehicles.rs each frame (0 for a motorcycle or with the offset off).
static DEPTH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn set_depth_offset(v: f32) {
    DEPTH.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// One readout's quads in the world (`world` = the car's drawn matrix): glyph k < count, its four corners as F-57755
/// lays them, the cell's uv as the programs compute it.
pub fn quads(world: Mat4, r: [Vec3; 3], normal: Vec3, t: &Text, color: [f32; 4], emissive: [f32; 3], fill: [f32; 4], out: &mut Vec<Vertex>) {
    let normal = world.transform_vector3(normal).to_array();
    for k in 0..t.count.min(GLYPHS) {
        let x0 = (k as f32 - 1.0) * STEP;
        let x1 = k as f32 * STEP;
        let cell = cell_uv(t.cells[k], t.roller);
        for (x, y, u, v) in [(x0, -STEP, 0.0, 1.0), (x0, 0.0, 0.0, 0.0), (x1, 0.0, 1.0, 0.0), (x0, -STEP, 0.0, 1.0), (x1, 0.0, 1.0, 0.0), (x1, -STEP, 1.0, 1.0)] {
            let local = r[0] * x + r[1] * y + r[2];
            let p = world * Vec4::new(local.x, local.y, local.z, 1.0);
            out.push(Vertex { pos: p.truncate().into(), uv: [u * 0.125 + cell[0], v * 0.125 + cell[1]], color, emissive, depth: 0.0, normal, fill });
        }
    }
}

/// What the draw needs of the player's car this frame.
pub struct Frame<'a> {
    pub world: Mat4,
    pub bones: &'a [(u8, Mat4, f32, f32)],
    /// per readout: colour rgba, skew, font index (the record's GaugeText, 0x190 + 0x20 k)
    pub tune: [[f32; 6]; 5],
    /// emIntColor: DashGaugeColor x the head lamp state (the car object's +168)
    pub emissive: [f32; 3],
    /// the player car's fill (fillmap::player_now), for ambYPos as the car's programs take it
    pub fill: [f32; 4],
    pub speed: f32,
    pub odo: f32,
    pub trip_start: f32,
    pub gear: i32,
    pub hours: f32,
}

// ---------------------------------------------------------------------------------------
// GPU

const MAX_VERTS: usize = 5 * GLYPHS * 6;

struct State {
    pipe: crate::render::ms::MsPipe,
    vb: wgpu::Buffer,
    bgs: Vec<wgpu::BindGroup>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub fn load(gpu: &mut Gpu, data_dir: &str) {
    let dir = std::path::Path::new(data_dir).join("vehicle/shared_font");
    let mut views = Vec::new();
    for name in FONTS {
        let file = format!("{name}.dds");
        let tex = std::fs::read(dir.join(&file)).ok().and_then(|d| mcla_formats::dds::parse_dds(&file, &d).ok());
        views.push(match tex {
            Some(t) => gpu.upload_texture(mcla_formats::hash::joaat(&format!("shared_font/{file}")), &t).texture.create_view(&Default::default()),
            None => {
                log::warn!("readouts: {file} missing");
                crate::render::create_rgba(&gpu.device, &gpu.queue, 1, 1, &[0, 0, 0, 0], "readouts").view
            }
        });
    }
    let device = &gpu.device;
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("readouts"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::FilterMode::Linear, ..Default::default() });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("readouts layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ],
    });
    let bgs = views
        .iter()
        .map(|v| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("readouts"), layout: &layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(v) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }] }))
        .collect();
    let vb = device.create_buffer(&wgpu::BufferDescriptor { label: Some("readouts vb"), size: (MAX_VERTS * std::mem::size_of::<Vertex>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("readouts"), source: wgpu::ShaderSource::Wgsl(include_str!("render/readouts.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("readouts pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    let pipe = crate::render::ms::MsPipe::new(device, move |device, v| {
        let attrs: [wgpu::VertexAttribute; 7] = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 20, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 36, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 48, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 52, shader_location: 5 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 64, shader_location: 6 },
        ];
        // (the technique's pass: AlphaBlendEnable 1, SrcBlend SRCALPHA, DestBlend INVSRCALPHA - rage ids 10 / 4 / 5)
        let blend = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("readouts"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs }], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_main"), targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: blend, alpha: blend }), write_mask: wgpu::ColorWrites::COLOR })], compilation_options: Default::default() }),
            // (*(approx.)* both sides, no depth write: the pass sets neither cull nor depth states)
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: v.depth_format(), depth_write_enabled: false, depth_compare: wgpu::CompareFunction::GreaterEqual, stencil: Default::default(), bias: Default::default() }),
            multisample: v.ms(),
            multiview: None,
            cache: crate::render::pcache::get(),
        })
    });
    *STATE.lock().unwrap() = Some(State { pipe, vb, bgs });
}

/// The pass, over the scene after its draws (*(approx.)* the original draws them in the car's own draw, before its
/// glass); `frame` is None outside the in-car view.
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder, frame: Option<&Frame>) {
    let Some(f) = frame else { return };
    let guard = STATE.lock().unwrap();
    let Some(s) = guard.as_ref() else { return };
    let fonts = f.tune.map(|t| (t[5].max(0.0) as usize).min(FONTS.len() - 1));
    let list = texts(fonts, f.speed, f.odo, f.trip_start, f.gear, f.hours);
    // (one draw per readout: each its own font)
    let mut verts: Vec<Vertex> = Vec::new();
    let mut draws: Vec<(usize, u32, u32)> = Vec::new();
    for (k, t) in list.iter().enumerate() {
        let (Some(t), Some(&(_, bone, w, h))) = (t, f.bones.iter().find(|b| b.0 as usize == k)) else { continue };
        let tune = f.tune[k];
        let start = verts.len() as u32;
        quads(f.world, rows(bone, w, h, tune[4], t.count), bone.z_axis.truncate(), t, [tune[0], tune[1], tune[2], tune[3]], f.emissive, f.fill, &mut verts);
        draws.push((t.font, start, verts.len() as u32));
    }
    let depth = f32::from_bits(DEPTH.load(std::sync::atomic::Ordering::Relaxed));
    verts.iter_mut().for_each(|v| v.depth = depth);
    // (MCLA_READOUT_LOG=1: what each readout shows and where)
    if std::env::var("MCLA_READOUT_LOG").is_ok_and(|v| v.trim() == "1") {
        for (k, t) in list.iter().enumerate() {
            let at = f.bones.iter().find(|b| b.0 as usize == k).map(|b| (f.world * b.1).w_axis.truncate());
            log::info!("readout {}: {:?} at {:?}", BONES[k], t.map(|t| (t.cells[..t.count.min(GLYPHS)].to_vec(), t.font)), at);
        }
    }
    if verts.is_empty() || verts.len() > MAX_VERTS {
        return;
    }
    gpu.queue.write_buffer(&s.vb, 0, bytemuck::cast_slice(&verts));
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("readouts"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: None, stencil_ops: None }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_bind_group(0, &gpu.globals_bg, &[]);
    pass.set_pipeline(&s.pipe);
    pass.set_vertex_buffer(0, s.vb.slice(..));
    for (font, a, b) in draws {
        pass.set_bind_group(1, &s.bgs[font], &[]);
        pass.draw(a..b, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The string table and the drum against the executable's code read by hand (cockpit_readouts_rule.md).
    #[test]
    fn readout_glyphs() {
        let (g, n) = string_glyphs("12 05");
        assert_eq!(n, 5);
        assert_eq!(&g[..5], &[1.0, 2.0, 63.0, 0.0, 5.0]);
        assert_eq!(string_glyphs("r").0[0], 27.0);
        assert_eq!(string_glyphs("N").0[0], 23.0);
        // 123.45 miles: v = 1234.5; the odometer (no tenths) ends on the ones' digit, the trip on the tenths' with the fraction
        let d = drum_values(123.45, 6, false);
        assert!(d[5] == 3.0 && d[4] == 2.0 && d[3] == 1.0 && d[2] == 0.0, "{d:?}");
        let t = drum_values(123.45, 6, true);
        assert!((t[5] - 4.5).abs() < 1e-3, "{t:?}");
        // 9.96: v = 99.6, share 0.96 >= 0.9: the digit left of a fraction >= 0.9 rolls by (0.96 - 0.9) x 10
        let r = drum_values(9.96, 6, true);
        assert!((r[4] - (9.0 + 0.6)).abs() < 1e-3, "{r:?}");
        assert_eq!(drum_cells(&[0.0, 7.5, 8.0, 9.25, 10.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 6)[..6], [8.0, 15.5, 24.0, 25.25, 63.0, 63.0]);
        // the drum's '0' (cell 8) half rolled: half a cell up, into the '1' above it
        let uv = cell_uv(8.5, true);
        assert!((uv[0] - 0.0).abs() < 1e-6 && (uv[1] - (0.125 - 0.0625)).abs() < 1e-6, "{uv:?}");
        let uv = cell_uv(27.0, false);
        assert!((uv[0] - 0.375).abs() < 1e-6 && (uv[1] - 0.375).abs() < 1e-6, "{uv:?}");
    }

    /// F-17778 run in the original's code on 3,200 values, counts and both tenths (tools/drum_values_reference.py,
    /// testdata/drum_values_ref.txt): every one of the 16 values the draw is handed equals the port's, bit for bit.
    #[test]
    fn drum_values_match_the_original() {
        let text = include_str!("testdata/drum_values_ref.txt");
        let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let mut n = 0;
        for l in text.lines() {
            let (a, b) = l.split_once(" -> ").unwrap();
            let a: Vec<&str> = a.split_whitespace().collect();
            let b: Vec<u32> = b.split_whitespace().map(h).collect();
            let (v, count, tenths) = (f32::from_bits(h(a[0])), a[1].parse::<usize>().unwrap(), a[2] == "1");
            assert_eq!(b[0] as usize, count);
            let got = drum_values(v, count, tenths);
            for k in 0..GLYPHS {
                assert_eq!(got[k].to_bits(), b[1 + k], "{l}: value {k} {} vs {}", got[k], f32::from_bits(b[1 + k]));
            }
            n += 1;
        }
        assert_eq!(n, 3200);
    }

    /// F-27249's cell remap run offline (`tools/drum_cells_reference.py`: 17 edge rows with -0, NaN, the infinities and
    /// the range ends, then 2,000 random): every cell's bits equal.
    #[test]
    fn drum_cells_match_the_original() {
        let text = include_str!("testdata/drum_cells_ref.txt");
        let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let mut n = 0;
        for l in text.lines() {
            let (a, b) = l.split_once(" -> ").unwrap();
            let a: Vec<&str> = a.split_whitespace().collect();
            let b: Vec<u32> = b.split_whitespace().map(h).collect();
            let count = a[0].parse::<usize>().unwrap();
            let mut v = [0.0f32; GLYPHS];
            for k in 0..GLYPHS {
                v[k] = f32::from_bits(h(a[1 + k]));
            }
            let got = drum_cells(&v, count);
            for k in 0..GLYPHS {
                assert_eq!(got[k].to_bits(), b[k], "{l}: cell {k} {} vs {}", got[k], f32::from_bits(b[k]));
            }
            n += 1;
        }
        assert_eq!(n, 2017);
    }

    /// The clock in 12 hours with two-digit minutes, the gear by the table, the speed in mph.
    #[test]
    fn readout_texts() {
        let t = texts([0, 3, 3, 0, 0], 26.8224, 1500.25, 1500.0, 3, 13.5);
        let s = |k: usize| t[k].map(|x| (x.cells, x.count));
        assert_eq!(s(0).map(|x| x.1), Some(2));
        assert_eq!(s(0).unwrap().0[..2], [6.0, 0.0]);
        assert_eq!(s(3).unwrap().0[0], 2.0);
        assert_eq!(s(4).unwrap().1, 4);
        assert_eq!(s(4).unwrap().0[..4], [1.0, 63.0, 3.0, 0.0]);
        assert!(t[1].unwrap().roller && t[1].unwrap().font == DRUM_FONT);
        assert!(texts([0; 5], 0.0, 0.0, 0.0, 10, 0.0)[3].is_none());
        assert_eq!(texts([0; 5], 0.0, 0.0, 0.0, 0, 0.0)[4].unwrap().cells[..4], [1.0, 2.0, 63.0, 0.0]);
    }

    /// The strip's rows: the right-aligned last glyph covers the bone's box (its top right corner the bone's place).
    #[test]
    fn readout_rows() {
        let bone = Mat4::from_translation(Vec3::new(-0.3273, 0.9001, -0.3471));
        let r = rows(bone, 0.0095, 0.0142, 0.0, 3);
        // glyph 2 (the last of 3) spans x [0.1, 0.2] of the strip
        let corner = |x: f32, y: f32| r[0] * x + r[1] * y + r[2];
        assert!((corner(0.2, 0.0) - Vec3::new(-0.3273, 0.9001, -0.3471)).length() < 1e-6);
        assert!((corner(0.1, -0.1) - Vec3::new(-0.3368, 0.8859, -0.3471)).length() < 1e-5);
    }
}
