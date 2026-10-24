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
        // (the wrap's division only for texels outside the picture: same texel, far cheaper)
        let (x, y) = if wrap {
            (if x < 0 || x >= w { x.rem_euclid(w) } else { x }, if y < 0 || y >= h { y.rem_euclid(h) } else { y })
        } else {
            (x.clamp(0, w - 1), y.clamp(0, h - 1))
        };
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

/// `x.round()` for x >= 0 (or NaN), without the library call `f32::round` is on the baseline x86-64 target:
/// the whole part by truncation, up by one from a fraction of a half (half away from zero, as `round`).
#[inline(always)]
fn round_pos(x: f32) -> f32 {
    let t = x as i32 as f32;
    if x - t >= 0.5 {
        t + 1.0
    } else {
        t
    }
}

/// The rows' threads: half the logical processors, at most 8 (measured on the real 3D icon, 8 cores / 16
/// threads: 8.4 ms a picture on 8 threads, 12.2 ms on 16 - docs/PERFORMANCE.md, 2026-10-23).
pub fn pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(4, |n| n.get());
        rayon::ThreadPoolBuilder::new().num_threads((n / 2).clamp(1, 8)).thread_name(|i| format!("raster-{i}")).build().expect("raster threads")
    })
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
        if background != [0; 4] {
            for p in buf.chunks_exact_mut(4) {
                p.copy_from_slice(&background);
            }
        }
        // (every sample the background: the resolved picture is the background)
        let rgba = buf[..(w * h) as usize * 4].to_vec();
        Canvas { w, h, rgba, samples: samples.to_vec(), buf, masks: Vec::new(), defining: None }
    }

    fn resolve(&mut self) {
        let n = self.samples.len();
        if n == 1 {
            self.rgba.clone_from(&self.buf);
            return;
        }
        // (the samples' sum / n, rounded - plain loops, the iterator chain was slow)
        self.rgba.resize((self.w * self.h) as usize * 4, 0);
        for (px, out) in self.buf.chunks_exact(4 * n).zip(self.rgba.chunks_exact_mut(4)) {
            for k in 0..4 {
                let mut sum = 0u32;
                for s in 0..n {
                    sum += px[s * 4 + k] as u32;
                }
                out[k] = round_pos(sum as f32 / n as f32) as u8;
            }
        }
    }

    /// Draw a list whose coordinates are scaled by `scale` and offset by `offset`.
    pub fn draw(&mut self, list: &DrawList, scale: f32, offset: [f32; 2], tex: &dyn Fn(TexRef) -> Option<std::sync::Arc<TexImage>>) {
        if list.items.is_empty() {
            // (nothing drawn: `rgba` is already the samples resolved)
            return;
        }
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
        if maxx <= minx || maxy <= miny {
            return;
        }
        let ns = self.samples.len();
        // (the sample positions copied once per triangle: cloning them per pixel was a heap allocation per
        // pixel of the bounding box - docs/PERFORMANCE.md, 2026-10-23)
        let mut sp = [(0f32, 0f32); 16];
        let nsp = ns.min(16);
        sp[..nsp].copy_from_slice(&self.samples[..nsp]);
        let sp = &sp[..nsp];
        let w = self.w as usize;
        let bary = |px: f32, py: f32| {
            let e = |a: &Vertex, b: &Vertex| (b.pos[0] - a.pos[0]) * (py - a.pos[1]) - (px - a.pos[0]) * (b.pos[1] - a.pos[1]);
            (e(&v[1], &v[2]) / area, e(&v[2], &v[0]) / area, e(&v[0], &v[1]) / area)
        };
        let masks = &self.masks;
        // One row of the bounding box: `buf` the row's samples (RGBA each), or `def` the row's part of the
        // mask being defined. Rows share no samples, so they run in parallel with the same result.
        // (the pixels of row y the triangle can reach: the triangle cut to the band y..y+1, one pixel more on
        // each side for the rounding of the exact test below - the pixels outside fail it anyway)
        let span = |y: i64| -> (i64, i64) {
            let (b0, b1) = (y as f64, y as f64 + 1.0);
            let (mut lo, mut hi) = (f64::MAX, f64::MIN);
            for i in 0..3 {
                let (p, q) = (v[i].pos, v[(i + 1) % 3].pos);
                let (px, py, qx, qy) = (p[0] as f64, p[1] as f64, q[0] as f64, q[1] as f64);
                if py >= b0 && py <= b1 {
                    lo = lo.min(px);
                    hi = hi.max(px);
                }
                if py != qy {
                    for b in [b0, b1] {
                        let t = (b - py) / (qy - py);
                        if (0.0..=1.0).contains(&t) {
                            let x = px + (qx - px) * t;
                            lo = lo.min(x);
                            hi = hi.max(x);
                        }
                    }
                }
            }
            if lo > hi {
                return (0, 0);
            }
            ((lo.floor() as i64 - 1).max(minx), (hi.ceil() as i64 + 1).min(maxx))
        };
        let row = |y: i64, buf: &mut [u8], def: Option<&mut [bool]>| {
            let base = y as usize * w * ns;
            let mut def = def;
            let (sx0, sx1) = span(y);
            for x in sx0..sx1 {
                let mut colour: Option<[f32; 4]> = None;
                // (the blend of the pixel's colour over a sample's bytes, kept for the next sample with the same bytes)
                let mut memo: Option<([u8; 4], [u8; 4])> = None;
                for (si, &(sx, sy)) in sp.iter().enumerate() {
                    let (w0, w1, w2) = bary(x as f32 + sx, y as f32 + sy);
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let li = x as usize * ns + si;
                    if !masks.iter().all(|m| m[base + li]) {
                        continue;
                    }
                    if let Some(d) = def.as_deref_mut() {
                        d[li] = true;
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
                    let o = li * 4;
                    let dst = [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]];
                    if let Some((m, r)) = memo {
                        if m == dst {
                            buf[o..o + 4].copy_from_slice(&r);
                            continue;
                        }
                    }
                    let a = (c[3] / 255.0).clamp(0.0, 1.0);
                    for i in 0..3 {
                        let d = buf[o + i] as f32;
                        buf[o + i] = round_pos(c[i].clamp(0.0, 255.0) * a + d * (1.0 - a)) as u8;
                    }
                    let da = buf[o + 3] as f32 / 255.0;
                    buf[o + 3] = round_pos((a + da * (1.0 - a)) * 255.0) as u8;
                    memo = Some((dst, [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]));
                }
            }
        };
        let (y0, y1) = (miny as usize, maxy as usize);
        let rl = w * ns;
        let parallel = (maxx - minx) * (maxy - miny) >= 4096;
        use rayon::prelude::*;
        if let Some(d) = self.defining.as_mut() {
            let rows = &mut d[y0 * rl..y1 * rl];
            if parallel {
                pool().install(|| rows.par_chunks_mut(rl).enumerate().for_each(|(i, r)| row(miny + i as i64, &mut [], Some(r))));
            } else {
                rows.chunks_mut(rl).enumerate().for_each(|(i, r)| row(miny + i as i64, &mut [], Some(r)));
            }
        } else {
            let rows = &mut self.buf[y0 * rl * 4..y1 * rl * 4];
            if parallel {
                pool().install(|| rows.par_chunks_mut(rl * 4).enumerate().for_each(|(i, r)| row(miny + i as i64, r, None)));
            } else {
                rows.chunks_mut(rl * 4).enumerate().for_each(|(i, r)| row(miny + i as i64, r, None));
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn round_pos_for_test(x: f32) -> f32 {
    round_pos(x)
}
