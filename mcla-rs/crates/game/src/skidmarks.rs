//! The tyres' marks on the road (`mcSkidMarks`, the global 0x8287E18C): every car's four wheels leave a strip of
//! the `tire_track` texture where they slide, as the original lays it. Decode: docs/reference/map/skid_marks.md.
//!
//! - The system (ctor F-64030, set-up F-57145): 28 trails of 128 points for the traffic and 4 of 1024 for a car
//!   with a player or racer slot (mcCar byte +593: the slot is not -1; the pools F-30775 / F-65116 give the first free
//!   one, or none). A car takes its four at its set-up (F-60786, the model's set-up) and gives them back, unreset,
//!   when it goes (F-46262): the next car to take one carries on where it stopped.
//! - Each game frame, after the model's update, F-55954 runs F-48637 for each wheel with a trail: the mark the
//!   ground's material asks for (F-52604: the suspension's compression over SuspensionLimit past ptxthreshold[1], or
//!   the slide past ptxthreshold[0], picks a type from ptxindex through a byte table, -1 none, and a strength
//!   (v - t) / (1 - t)); a point every 0.165 m of the contact point (0.01 m above the ground), a strip 1.2 x the
//!   tyre's width across, square to the ground's normal and the way travelled; a straight stretch under 2 m only
//!   moves its last pair, a new mark starts with two transparent pairs from where the last one ended.
//! - The vertices (two per point, 32 bytes each: place, colour (0.01, 0.01, 0.01, 0.5 x strength) as D3DCOLOR, the
//!   texture's (0 or 1, distance x 0.2 / width)) are written to a ring, the pair ahead of the head transparent.
//!   The draw (F-59006, F-81044) draws every taken trail's whole ring as one strip.

use crate::physics::artic::V4;
use crate::render::Gpu;
use glam::Vec3;
use std::sync::Mutex;

/// The system's colour (+208, F-57145): its w is the strength's scale.
pub const COLOUR: [f32; 4] = [0.01, 0.01, 0.01, 0.5];
/// The traffic's trails, their points; the racers' (F-64030's set-up loops).
pub const SMALL: (usize, usize) = (28, 128);
pub const BIG: (usize, usize) = (4, 1024);

/// The byte table F-52604 indexes with a material's ptxindex (0x8203F140; what follows it in the image is the
/// next object's words, read as they are by an index past 15).
const KIND: [u8; 32] = [0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0xff, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x82, 0x0e, 0x75, 0x84, 0x82, 0x2e, 0xa0, 0x88, 0x82, 0x13, 0x05, 0xb8, 0x82, 0x13, 0x05, 0xb8];

/// What a wheel hands its trail for a frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wheel {
    /// the contact point (+368) and the ground's normal (+336)
    pub contact: Vec3,
    pub normal: Vec3,
    /// the suspension's compression (+496) and the slide (+520)
    pub comp: f32,
    pub slide: f32,
    /// the ground's material record: ptxindex[0], [1] (+44, +46), ptxthreshold[0], [1] (+48, +52); None in the air
    pub mat: Option<([i16; 2], [f32; 2])>,
    /// the tyre's width (+8), TravelLimit (+72), TravelExtent (+76), SuspensionLimit (+84)
    pub width: f32,
    pub travel_limit: f32,
    pub travel_extent: f32,
    pub suspension_limit: f32,
}

/// F-52604: the mark's type (-1 none) and strength.
pub fn mark(w: &Wheel) -> (i32, f32) {
    let Some((ix, th)) = w.mat else { return (-1, 0.0) };
    // (F-92300: the compression clamped to -TravelExtent .. TravelLimit by fsel)
    let lo = -w.travel_extent;
    let c = if w.comp - lo >= 0.0 { w.comp } else { lo };
    let c = if c - w.travel_limit >= 0.0 { w.travel_limit } else { c };
    let r = if c >= 0.0 { c / w.suspension_limit } else { 0.0 };
    let kind = |i: i16| if i == -1 { -1 } else { KIND.get(i as usize).map_or(-1, |&b| b as i8 as i32) };
    let (mut t, mut s) = (-1, 0.0);
    if !(r < th[1]) {
        t = kind(ix[1]);
        s = (r - th[1]) / (1.0 - th[1]);
    }
    if !(w.slide < th[0]) {
        t = kind(ix[0]);
        s = (w.slide - th[0]) / (1.0 - th[0]);
    }
    (t, s)
}

/// F-44647: a colour clamped to 0 .. 1, times 255, truncated, as A R G B bytes.
pub fn pack(c: [f32; 4]) -> u32 {
    let k = |v: f32| -> u32 {
        let v = if v >= 0.0 { if v > 1.0 { 1.0 } else { v } } else { 0.0 };
        (v * 255.0) as u32
    };
    // (the red's own test: under 0 it is 0, else no more than 1 - NaN gives 1, not 0)
    let r = if c[0] < 0.0 { 0.0 } else if c[0] <= 1.0 { c[0] } else { 1.0 };
    (k(c[3]) << 24) | (((r * 255.0) as u32) << 16) | (k(c[1]) << 8) | k(c[2])
}

/// One trail (F-73020, 144 bytes) and its ring.
#[derive(Clone, Debug)]
pub struct Trail {
    /// +28 the mark's type, +32 the last point, +48 the way it went, +64 / +80 the last pair's ends
    pub kind: i32,
    pub last: V4,
    pub dir: V4,
    pub end_a: V4,
    pub end_b: V4,
    /// +96 the length laid, +100 the head, +104 the points, +112 / +116 / +118
    pub dist: f32,
    pub head: usize,
    pub cap: usize,
    pub k112: u32,
    pub new: bool,
    pub k118: bool,
    /// +120: per point two vertices of 32 bytes (place xyz + colour word, texture coordinates), as words
    pub ring: Vec<[u32; 16]>,
    /// +124: per point the buffers it has still to go to (bit k for buffer k)
    pub dirty: Vec<u32>,
}

fn with_w(v: V4, w: u32) -> [u32; 4] {
    let b = v.bits();
    [b[0], b[1], b[2], w]
}

impl Trail {
    /// F-73020 and its reset F-29621.
    pub fn new(cap: usize) -> Trail {
        // (each pair's second texture word: x is vupkd3d's 1 - the reset's merge of it with zeros)
        let mut ring = vec![[0u32; 16]; cap];
        for p in ring.iter_mut() {
            p[12] = 1.0f32.to_bits();
        }
        Trail { kind: -1, last: V4::ZERO, dir: V4::ZERO, end_a: V4::ZERO, end_b: V4::ZERO, dist: 0.0, head: 0, cap, k112: 0, new: false, k118: false, ring, dirty: vec![0; cap] }
    }

    fn uv(&mut self, i: usize, u: f32) {
        let ub = u.to_bits();
        self.ring[i][4..8].copy_from_slice(&[0, ub, 0, ub]);
        self.ring[i][12..16].copy_from_slice(&[1.0f32.to_bits(), ub, 1.0f32.to_bits(), ub]);
    }

    /// F-99065: a new pair at the head (the ring wraps one short of its end), `alpha` the colour's share; true when
    /// it wrapped.
    fn append(&mut self, u: f32, alpha: f32, a: V4, b: V4) -> bool {
        let mut wrapped = false;
        if self.head as i32 >= self.cap as i32 - 1 {
            self.head = 0;
            wrapped = true;
        }
        let c = pack([COLOUR[0], COLOUR[1], COLOUR[2], COLOUR[3] * alpha]);
        let i = self.head;
        self.ring[i][0..4].copy_from_slice(&with_w(a, c));
        self.ring[i][8..12].copy_from_slice(&with_w(b, c));
        self.uv(i, u);
        self.dirty[i] = 255;
        self.head += 1;
        self.ahead(a, b, pack([COLOUR[0], COLOUR[1], COLOUR[2], 0.0]));
        wrapped
    }

    /// F-89324: the pair at the head the same ends, and the one after it, transparent (`c`).
    fn ahead(&mut self, a: V4, b: V4, c: u32) {
        let top = self.cap - 1;
        let j = self.head.min(top);
        self.ring[j][0..4].copy_from_slice(&with_w(a, c));
        self.ring[j][8..12].copy_from_slice(&with_w(b, c));
        self.dirty[j] = 255;
        let k = (self.head + 1).min(top);
        self.ring[k][3] = c;
        self.ring[k][11] = c;
        self.dirty[k] = 255;
    }

    /// F-56849: the last pair moved to new ends (its colour kept).
    fn extend(&mut self, u: f32, a: V4, b: V4) {
        let i = if self.head == 0 { self.cap - 1 } else { self.head - 1 };
        let (ca, cb) = (self.ring[i][3], self.ring[i][11]);
        self.ring[i][0..4].copy_from_slice(&with_w(a, ca));
        self.ring[i][8..12].copy_from_slice(&with_w(b, cb));
        self.uv(i, u);
        self.dirty[i] = 255;
    }

    /// F-48637: one frame of a wheel.
    pub fn update(&mut self, w: &Wheel) {
        let v = |a: Vec3| V4::new(a.x, a.y, a.z, 0.0);
        let n = v(w.normal);
        let p = n.madd(V4::splat(0.01), v(w.contact));
        let half = w.width * 0.6;
        let per = 0.2 / w.width;
        let (kind, strength) = mark(w);
        if kind == -1 || strength == 0.0 {
            self.dist = 0.0;
            self.new = false;
            self.kind = -1;
            return;
        }
        if self.kind != kind && !self.new {
            self.new = true;
            self.k112 = 0;
        }
        let d = p.sub(self.last);
        let (dx, dy, dz) = (d.0[0], d.0[1], d.0[2]);
        let len = crate::physics::exact::fma(dx, dx, crate::physics::exact::fma(dy, dy, dz * dz)).sqrt();
        if len < 0.165 {
            return;
        }
        let dirn = d.mul(V4::splat(1.0 / len));
        let c = n.yzx().nmsub(dirn.zxy(), n.zxy().mul(dirn.yzx()));
        let a = c.madd(V4::splat(-half), p);
        let b = c.madd(V4::splat(half), p);
        let u = self.dist * per;
        if self.new {
            self.dir = dirn;
            self.end_b = c.madd(V4::splat(half), self.last);
            self.end_a = c.madd(V4::splat(-half), self.last);
            let (ea, eb) = (self.end_a, self.end_b);
            self.append(u, 0.0, ea, eb);
            self.kind = kind;
            self.new = false;
            self.append(u, 0.0, a, b);
            self.k112 = 2;
            self.last = p;
            self.dist += len;
            return;
        }
        let r = &self.dir.0;
        let along = crate::physics::exact::fma(dirn.0[0], r[0], crate::physics::exact::fma(r[1], dirn.0[1], r[2] * dirn.0[2]));
        if !(len > 2.0) && !(along < 0.995) {
            self.extend(u, a, b);
        } else {
            self.append(u, strength, a, b);
            self.dir = dirn;
            self.last = p;
            self.k118 = true;
            self.dist += len;
        }
    }
}

// ---------------------------------------------------------------------------------------
// The system: the pools, the cars' trails, the frame update

/// One pool trail: taken (the pool's flag byte), the trail, and the ring as the draw shows it (see [`System::step`]).
struct Slot {
    taken: bool,
    trail: Trail,
    shown: Vec<[u32; 16]>,
}

/// mcSkidMarks (F-64030 / F-57145) as the port keeps it, with the ground materials' ptxindex / ptxthreshold.
pub struct System {
    /// the 28 traffic trails, then the 4 big ones (the vertex buffer's order)
    slots: Vec<Slot>,
    /// a car's uid -> its four trails (an index into `slots`, or none when the pool had no free one)
    cars: std::collections::HashMap<u32, [Option<usize>; 4]>,
    mats: std::collections::HashMap<u8, ([i16; 2], [f32; 2])>,
    /// bumped by every frame update, for the draw's upload
    frame: u64,
}

impl System {
    pub fn new(mats: std::collections::HashMap<u8, ([i16; 2], [f32; 2])>) -> System {
        let mut slots = Vec::new();
        for (n, cap) in [SMALL, BIG] {
            for _ in 0..n {
                let trail = Trail::new(cap);
                let shown = trail.ring.clone();
                slots.push(Slot { taken: false, trail, shown });
            }
        }
        System { slots, cars: Default::default(), mats, frame: 0 }
    }

    /// F-30775 / F-65116: the first free trail of a pool, taken.
    fn take(&mut self, big: bool) -> Option<usize> {
        let r = if big { SMALL.0..SMALL.0 + BIG.0 } else { 0..SMALL.0 };
        let i = r.into_iter().find(|&i| !self.slots[i].taken)?;
        self.slots[i].taken = true;
        Some(i)
    }

    /// One game frame (the original's 30 Hz, from the model's update F-16712): the cars that went give their
    /// trails back (F-46262, the trail left as it is), new ones take four each in the order they were made (the
    /// player's car - the slot that is not -1 - from the big pool, the rest from the traffic's), and F-55954 runs
    /// F-48637 for each wheel with a trail. Each taken trail's ring is first kept as the draw will show it: the
    /// draw (F-81044) copies the changed pairs into buffer counter % 3 and draws buffer (counter + 2) % 3, the one
    /// it filled a frame before - so a frame's draw shows the ring as the frame before left it.
    pub fn step(&mut self, cars: &[crate::vehicles::CarInstance]) {
        self.frame += 1;
        let live: Vec<&crate::vehicles::CarInstance> = cars.iter().filter(|c| c.state.veh.is_some() && c.model.params.veh.is_some() && c.parked.is_none() && c.placed).collect();
        let gone: Vec<u32> = self.cars.keys().copied().filter(|u| !live.iter().any(|c| c.uid == *u)).collect();
        for u in gone {
            for i in self.cars.remove(&u).into_iter().flatten().flatten() {
                self.slots[i].taken = false;
            }
        }
        let mut new: Vec<&crate::vehicles::CarInstance> = live.iter().copied().filter(|c| !self.cars.contains_key(&c.uid)).collect();
        new.sort_by_key(|c| c.uid);
        for c in new {
            let t = [(); 4].map(|_| self.take(c.is_player));
            self.cars.insert(c.uid, t);
        }
        for s in self.slots.iter_mut().filter(|s| s.taken) {
            s.shown.copy_from_slice(&s.trail.ring);
        }
        for c in live {
            let (Some(vs), Some(tune)) = (c.state.veh.as_deref(), c.model.params.veh.as_deref()) else { continue };
            let trails = self.cars[&c.uid];
            // (the sim's wheels in its order: a bike has two, the front and the back)
            let wheels: &[usize] = if c.model.bike { &[0, 2] } else { &[0, 1, 2, 3] };
            for (k, &wi) in wheels.iter().enumerate() {
                let (Some(ti), Some(w)) = (trails[k], vs.wheels.get(wi)) else { continue };
                let wt = if wi < 2 { &tune.t.wheel_front } else { &tune.t.wheel_back };
                let input = Wheel {
                    contact: w.frame[3],
                    normal: w.frame[1],
                    comp: w.comp,
                    slide: w.slide,
                    mat: w.ground.and_then(|g| self.mats.get(&g).copied()),
                    width: wt.width,
                    travel_limit: wt.travel_limit,
                    travel_extent: wt.travel_extent,
                    suspension_limit: wt.suspension_limit,
                };
                self.slots[ti].trail.update(&input);
                // (MCLA_SKID_LOG=1: the player's wheels each frame - their mark and their trail's head)
                if c.is_player && std::env::var("MCLA_SKID_LOG").is_ok_and(|v| v.trim() == "1") {
                    let t = &self.slots[ti].trail;
                    log::info!("skid: frame {} wheel {wi} trail {ti} ground {:?} comp {:.3} slide {:.3} mark {:?} head {} kind {} dist {:.2} contact {:.2?} car {:.2?} pair {:?}", self.frame, w.ground, w.comp, w.slide, mark(&input), t.head, t.kind, t.dist, w.frame[3], c.state.pos, t.ring[t.head.saturating_sub(1)].map(f32::from_bits));
                }
            }
        }
    }
}

static SYS: Mutex<Option<System>> = Mutex::new(None);

/// The frame update (System::step), once the system is loaded.
pub fn step(cars: &[crate::vehicles::CarInstance]) {
    if let Some(s) = SYS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        s.step(cars);
    }
}

// ---------------------------------------------------------------------------------------
// GPU

struct State {
    pipe: crate::render::ms::MsPipe,
    vb: wgpu::Buffer,
    bg: wgpu::BindGroup,
    scene_layout: wgpu::BindGroupLayout,
    /// the frame whose rings are in `vb`
    uploaded: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Each slot's first vertex in the vertex buffer and its point count.
fn ranges() -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (n, cap) in [SMALL, BIG] {
        for _ in 0..n {
            out.push((at, cap as u32));
            at += 2 * cap as u32;
        }
    }
    out
}

/// Once, with the city's data: the materials, the texture (`tire_track`, the cars' globaltex), the program.
pub fn load(gpu: &mut Gpu, data_dir: &str) {
    let root = std::path::Path::new(data_dir);
    *SYS.lock().unwrap_or_else(|e| e.into_inner()) = Some(System::new(crate::tyrefx::materials(root).into_iter().collect()));
    let base = root.join("textures/global/cars/globaltex");
    let tex = std::fs::read(base.join("tire_track.dds"))
        .or_else(|_| std::fs::read(base.join(format!("0x{:08x}", mcla_formats::hash::joaat("tire_track.dds")))))
        .ok()
        .and_then(|d| mcla_formats::dds::parse_dds("tire_track.dds", &d).ok());
    // (fetched without gamma: the draws' fetch constant has sign bits "uuuu" - recorded, trailrec g.0.bin)
    let view = match tex {
        Some(t) => gpu.upload_texture_raw(mcla_formats::hash::joaat("skidmarks/tire_track.dds"), &t).texture.create_view(&Default::default()),
        None => {
            log::warn!("skidmarks: tire_track.dds missing");
            crate::render::create_rgba(&gpu.device, &gpu.queue, 1, 1, &[0, 0, 0, 0], "skidmarks").view
        }
    };
    let device = &gpu.device;
    // (the fetch constant 81000802 05ab9054 0007e03f 06a80d10: clamp_x 2 - clamp to the last texel, across the
    // strip -, clamp_y 0 - wrap, along it -; mag / min / mip filters linear, anisotropy 4:1)
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("skidmarks"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        anisotropy_clamp: 4,
        ..Default::default()
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("skidmarks layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ],
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("skidmarks"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
    });
    // (the scene's depth and stencil resolved, for the fog pass that follows the marks in the original)
    let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("skidmarks scene"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
        ],
    });
    let verts: u64 = ranges().iter().map(|r| 2 * r.1 as u64).sum();
    let vb = device.create_buffer(&wgpu::BufferDescriptor { label: Some("skidmarks vb"), size: verts * 32, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    // (the slipstream trail's program file brings the globals, the scene's depth / stencil and `fog_at`)
    let src = format!("{}\n{}", include_str!("render/sst_trail.wgsl"), include_str!("render/skidmarks.wgsl"));
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("skidmarks"), source: wgpu::ShaderSource::Wgsl(src.into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("skidmarks pl"), bind_group_layouts: &[&globals_layout, &layout, &scene_layout], push_constant_ranges: &[] });
    let pipe = crate::render::ms::MsPipe::new(device, move |device, v| {
        // (32 bytes a vertex: place, the D3DCOLOR (A R G B in a word: bytes B G R A here), the texture's (across, along))
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 16, shader_location: 2 },
        ];
        // (the pass: SrcBlend SRCALPHA, DestBlend INVSRCALPHA - the port's colour is the fogged one, `fs_skid`)
        let blend = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skidmarks"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_skid"), buffers: &[wgpu::VertexBufferLayout { array_stride: 32, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs }], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_skid"),
                targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: blend, alpha: blend }), write_mask: wgpu::ColorWrites::COLOR })],
                compilation_options: Default::default(),
            }),
            // (the draw's primitive: a strip of 2N vertices - recorded; CullMode 0)
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, cull_mode: None, ..Default::default() },
            // (ZWriteEnable 0, the Z test on - recorded depth control 00708762; reversed depth; no stencil test)
            depth_stencil: Some(wgpu::DepthStencilState { format: v.depth_format(), depth_write_enabled: false, depth_compare: wgpu::CompareFunction::GreaterEqual, stencil: Default::default(), bias: Default::default() }),
            multisample: v.ms(),
            multiview: None,
            cache: None,
        })
    });
    *STATE.lock().unwrap() = Some(State { pipe, vb, bg, scene_layout, uploaded: 0 });
}

/// The draw (F-59006): every taken trail's whole ring as one strip (F-81044), the traffic's first, then the big
/// ones, with the identity matrix (the rings are in the world). Where the original draws it: after the props'
/// glass, before the people, the slipstream trails and the fog pass (recorded, trailrec g.0.bin draws 1651 - 1658:
/// four of 256 vertices, four of 2048). *(not the original's)* the port's people are drawn in the scene pass, before
/// the marks. MCLA_NOSKIDMARKS=1 leaves them out.
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder) {
    if std::env::var("MCLA_NOSKIDMARKS").is_ok_and(|v| v.trim() == "1") {
        return;
    }
    let mut guard = STATE.lock().unwrap();
    let Some(st) = guard.as_mut() else { return };
    let sys = SYS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sys) = sys.as_ref() else { return };
    let ranges = ranges();
    if st.uploaded != sys.frame {
        st.uploaded = sys.frame;
        for (s, r) in sys.slots.iter().zip(&ranges).filter(|(s, _)| s.taken) {
            gpu.queue.write_buffer(&st.vb, r.0 as u64 * 32, bytemuck::cast_slice(&s.shown));
        }
    }
    let draws: Vec<(u32, u32)> = sys.slots.iter().zip(&ranges).filter(|(s, _)| s.taken).map(|(_, r)| (r.0, r.0 + 2 * r.1)).collect();
    if draws.is_empty() {
        return;
    }
    let scene_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("skidmarks scene"),
        layout: &st.scene_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&gpu.depth) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&gpu.stencil_copy) }],
    });
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("skid marks"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: None, stencil_ops: None }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_bind_group(0, &gpu.globals_bg, &[]);
    pass.set_pipeline(&st.pipe);
    pass.set_bind_group(1, &st.bg, &[]);
    pass.set_bind_group(2, &scene_bg, &[]);
    pass.set_vertex_buffer(0, st.vb.slice(..));
    for (a, b) in draws {
        pass.draw(a..b, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(w: &str) -> u32 {
        u32::from_str_radix(w.trim_start_matches('x'), 16).unwrap()
    }

    /// F-48637 with its callees run offline (tools/skid_marks_reference.py): 40 trails of 128 points, 400 frames each -
    /// every frame's trail fields and, at the end, the whole ring and its dirty words, against Trail::update, to the bit.
    #[test]
    fn skid_marks_match_the_original() {
        let text = include_str!("testdata/skid_marks_ref.txt");
        let mut lines = text.lines();
        let (mut seqs, mut frames, mut marked) = (0, 0, 0);
        while let Some(q) = lines.next() {
            let q: Vec<u32> = q.strip_prefix("q ").unwrap().split(' ').map(word).collect();
            let f = |i: usize| f32::from_bits(q[i]);
            let mats: Vec<([i16; 2], [f32; 2])> = (0..3).map(|m| ([q[4 + 4 * m] as u16 as i16, q[5 + 4 * m] as u16 as i16], [f(6 + 4 * m), f(7 + 4 * m)])).collect();
            let mut t = Trail::new(128);
            loop {
                let l = lines.next().unwrap();
                if let Some(r) = l.strip_prefix("r ") {
                    let r: Vec<u32> = r.split(' ').map(word).collect();
                    for (i, p) in t.ring.iter().enumerate() {
                        assert_eq!(&p[..], &r[16 * i..16 * i + 16], "sequence {seqs} pair {i}");
                    }
                    let d: Vec<u32> = lines.next().unwrap().strip_prefix("d ").unwrap().split(' ').map(word).collect();
                    assert_eq!(t.dirty, d, "sequence {seqs}");
                    break;
                }
                let (i, o) = l.strip_prefix("f ").unwrap().split_once(" -> ").unwrap();
                let i: Vec<u32> = i.split(' ').map(word).collect();
                let o: Vec<u32> = o.split(' ').map(word).collect();
                let g = |k: usize| f32::from_bits(i[k]);
                let w = Wheel {
                    contact: Vec3::new(g(0), g(1), g(2)),
                    normal: Vec3::new(g(3), g(4), g(5)),
                    comp: g(6),
                    slide: g(7),
                    mat: (i[8] != 0).then(|| mats[i[8] as usize - 1]),
                    width: f(0),
                    travel_limit: f(1),
                    travel_extent: f(2),
                    suspension_limit: f(3),
                };
                t.update(&w);
                let got = [t.kind as u32, t.last.0[0].to_bits(), t.last.0[1].to_bits(), t.last.0[2].to_bits(), t.dir.0[0].to_bits(), t.dir.0[1].to_bits(), t.dir.0[2].to_bits(), t.dist.to_bits(), t.head as u32, t.k112, t.new as u32, t.k118 as u32];
                assert_eq!(&got[..], &o[..], "sequence {seqs} frame {frames}: {l}");
                frames += 1;
                marked += (t.kind != -1) as u32;
            }
            seqs += 1;
        }
        assert!(seqs == 40 && frames == 16000 && marked > 3000, "{seqs} {frames} {marked}");
    }
}
