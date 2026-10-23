//! Minimal software rasteriser for draw lists: pixel-centre sampling,
//! textures modulated by the vertex colour, source-alpha blending, nested masks.
//! Used for headless checks and the `mcla-tool swf png` command.

use crate::draw::{DrawList, Item, TexRef, Vertex};

pub struct TexImage {
    pub w: u32,
    pub h: u32,
    /// RGBA8
    pub rgba: Vec<u8>,
}

impl TexImage {
    fn texel(&self, x: i64, y: i64, wrap: bool) -> [f32; 4] {
        let (w, h) = (self.w as i64, self.h as i64);
        let (x, y) = if wrap { (x.rem_euclid(w), y.rem_euclid(h)) } else { (x.clamp(0, w - 1), y.clamp(0, h - 1)) };
        let o = ((y * w + x) * 4) as usize;
        [self.rgba[o] as f32, self.rgba[o + 1] as f32, self.rgba[o + 2] as f32, self.rgba[o + 3] as f32]
    }

    fn sample(&self, u: f32, v: f32, kind: u8) -> [f32; 4] {
        let wrap = kind == 0x40 || kind == 0x42;
        let smooth = kind == 0x40 || kind == 0x41;
        let x = u * self.w as f32 - 0.5;
        let y = v * self.h as f32 - 0.5;
        if !smooth {
            return self.texel((x + 0.5).floor() as i64, (y + 0.5).floor() as i64, wrap);
        }
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = self.texel(x0, y0, wrap);
        let b = self.texel(x0 + 1, y0, wrap);
        let c = self.texel(x0, y0 + 1, wrap);
        let d = self.texel(x0 + 1, y0 + 1, wrap);
        let mut o = [0.0; 4];
        for i in 0..4 {
            o[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
        }
        o
    }
}

pub struct Canvas {
    pub w: u32,
    pub h: u32,
    /// the resolved picture (the samples of each pixel averaged), after [`Canvas::draw`]
    pub rgba: Vec<u8>,
    /// where the samples of a pixel lie (pixel corner = 0)
    samples: Vec<(f32, f32)>,
    /// RGBA per sample, `samples.len()` per pixel
    buf: Vec<u8>,
    masks: Vec<Vec<bool>>,
    defining: Option<Vec<bool>>,
}

/// D3D's standard 4x pattern, (-2,-6) (6,-2) (-6,2) (2,6) / 16 px from the pixel's centre: what the recomp's
/// captures of the original's 4x targets use (the console's own positions are not decoded).
pub const SAMPLES_4X: [(f32, f32); 4] = [(0.375, 0.125), (0.875, 0.375), (0.125, 0.625), (0.625, 0.875)];

impl Canvas {
    /// One sample at the pixel's centre.
    pub fn new(w: u32, h: u32, background: [u8; 4]) -> Canvas {
        Canvas::with_samples(w, h, background, &[(0.5, 0.5)])
    }

    /// A multisampled target: coverage per sample, colour once per pixel at its centre (as the hardware
    /// shades), the samples averaged into `rgba`.
    pub fn with_samples(w: u32, h: u32, background: [u8; 4], samples: &[(f32, f32)]) -> Canvas {
        let n = samples.len().max(1);
        let mut buf = vec![0u8; (w * h) as usize * n * 4];
        for p in buf.chunks_exact_mut(4) {
            p.copy_from_slice(&background);
        }
        let mut c = Canvas { w, h, rgba: Vec::new(), samples: samples.to_vec(), buf, masks: Vec::new(), defining: None };
        c.resolve();
        c
    }

    fn resolve(&mut self) {
        let n = self.samples.len();
        if n == 1 {
            self.rgba.clone_from(&self.buf);
            return;
        }
        self.rgba = self.buf.chunks_exact(4 * n).flat_map(|px| (0..4).map(move |k| ((0..n).map(|s| px[s * 4 + k] as u32).sum::<u32>() as f32 / n as f32).round() as u8)).collect();
    }

    /// Draw a list whose coordinates are scaled by `scale` and offset by `offset`.
    pub fn draw(&mut self, list: &DrawList, scale: f32, offset: [f32; 2], tex: &dyn Fn(TexRef) -> Option<std::sync::Arc<TexImage>>) {
        let n = (self.w * self.h) as usize * self.samples.len();
        for item in &list.items {
            match item {
                Item::MaskBegin => self.defining = Some(vec![false; n]),
                Item::MaskEnd => {
                    if let Some(m) = self.defining.take() {
                        self.masks.push(m);
                    }
                }
                Item::MaskPop => {
                    self.masks.pop();
                }
                Item::Triangles { tex: t, kind, verts } => {
                    let image = if *t == TexRef::None { None } else { tex(*t) };
                    for tri in verts.chunks_exact(3) {
                        let s = |v: &Vertex| Vertex { pos: [v.pos[0] * scale + offset[0], v.pos[1] * scale + offset[1]], ..*v };
                        self.triangle([s(&tri[0]), s(&tri[1]), s(&tri[2])], image.as_deref(), *kind);
                    }
                }
            }
        }
        self.resolve();
    }

    fn triangle(&mut self, v: [Vertex; 3], tex: Option<&TexImage>, kind: u8) {
        let area = (v[1].pos[0] - v[0].pos[0]) * (v[2].pos[1] - v[0].pos[1]) - (v[2].pos[0] - v[0].pos[0]) * (v[1].pos[1] - v[0].pos[1]);
        if area.abs() < 1e-9 {
            return;
        }
        let minx = v.iter().map(|p| p.pos[0]).fold(f32::MAX, f32::min).floor().max(0.0) as i64;
        let maxx = v.iter().map(|p| p.pos[0]).fold(f32::MIN, f32::max).ceil().min(self.w as f32) as i64;
        let miny = v.iter().map(|p| p.pos[1]).fold(f32::MAX, f32::min).floor().max(0.0) as i64;
        let maxy = v.iter().map(|p| p.pos[1]).fold(f32::MIN, f32::max).ceil().min(self.h as f32) as i64;
        let ns = self.samples.len();
        let bary = |px: f32, py: f32| {
            let e = |a: &Vertex, b: &Vertex| (b.pos[0] - a.pos[0]) * (py - a.pos[1]) - (px - a.pos[0]) * (b.pos[1] - a.pos[1]);
            (e(&v[1], &v[2]) / area, e(&v[2], &v[0]) / area, e(&v[0], &v[1]) / area)
        };
        for y in miny..maxy {
            for x in minx..maxx {
                let mut colour: Option<[f32; 4]> = None;
                for (si, (sx, sy)) in self.samples.clone().into_iter().enumerate() {
                    let (w0, w1, w2) = bary(x as f32 + sx, y as f32 + sy);
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let idx = (y as u32 * self.w + x as u32) as usize * ns + si;
                    if !self.masks.iter().all(|m| m[idx]) {
                        continue;
                    }
                    if let Some(d) = &mut self.defining {
                        d[idx] = true;
                        continue;
                    }
                    // (the colour once per pixel, at its centre)
                    let c = *colour.get_or_insert_with(|| {
                        let (w0, w1, w2) = bary(x as f32 + 0.5, y as f32 + 0.5);
                        let mut c = [0f32; 4];
                        for i in 0..4 {
                            c[i] = v[0].color[i] as f32 * w0 + v[1].color[i] as f32 * w1 + v[2].color[i] as f32 * w2;
                        }
                        if let Some(t) = tex {
                            let u = v[0].uv[0] * w0 + v[1].uv[0] * w1 + v[2].uv[0] * w2;
                            let vv = v[0].uv[1] * w0 + v[1].uv[1] * w1 + v[2].uv[1] * w2;
                            let s = t.sample(u, vv, kind);
                            for i in 0..4 {
                                c[i] = c[i] * s[i] / 255.0;
                            }
                        }
                        c
                    });
                    let a = (c[3] / 255.0).clamp(0.0, 1.0);
                    let o = idx * 4;
                    for i in 0..3 {
                        let d = self.buf[o + i] as f32;
                        self.buf[o + i] = (c[i].clamp(0.0, 255.0) * a + d * (1.0 - a)).round() as u8;
                    }
                    let da = self.buf[o + 3] as f32 / 255.0;
                    self.buf[o + 3] = ((a + da * (1.0 - a)) * 255.0).round() as u8;
                }
            }
        }
    }
}
