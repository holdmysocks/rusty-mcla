//! `--car-picture <car> --out <png> [--size WxH] [--view showroom|side]`: a picture of one car alone on a clear
//! background, for the mod tools (MODS.md "Car pictures"; the car exporter makes its showroom pictures with it).
//!
//! The run is a garage test run (`--garage`, a screenshot run: the save is not written) with `<car>` on the stand,
//! stock. After the warm-up (`--frames`, default 240) the clock stands still and the frame is drawn twice with
//! nothing but the cars: no sky, city, garage room, floor, props, people, blobs under the cars, glows or menus, the
//! tone map at the adapted luminance of the last warm-up frame with no bloom, depth of field or blur; once over
//! black, once over white. The pair gives each pixel its cover (`matte`): glass comes out see-through as it is drawn.
//! The picture is cut to what the car covers and, with `--size`, fitted into that box, centred (`fit`).
//!
//! The port's own tool: the original's pictures are made beforehand and come from the disc.

/// How the car is looked at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// The garage's camera as it stands on the car (the garage menu's view).
    #[default]
    Showroom,
    /// Square from the car's right, at the height of its middle.
    Side,
}

impl View {
    pub fn parse(s: &str) -> Option<View> {
        match s.trim().to_ascii_lowercase().as_str() {
            "showroom" => Some(View::Showroom),
            "side" => Some(View::Side),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Cfg {
    /// the car's folder name (`vp_aud_r8_08`, a mod car's name)
    pub car: String,
    pub out: String,
    pub size: Option<(u32, u32)>,
    pub view: View,
    /// frames before the two drawn for the picture
    pub warm: u64,
}

impl Cfg {
    /// Which of the two picture frames frame `frame_no` is (0 over black, 1 over white), none while warming up.
    pub fn phase(&self, frame_no: u64) -> Option<u8> {
        (frame_no >= self.warm).then(|| (frame_no - self.warm).min(1) as u8)
    }
}

/// `WxH`.
pub fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.trim().split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// How far from the frame's centre the car's box may reach (1 = the edge).
pub const FILL: f32 = 0.9;

/// The clear colour of a picture frame (linear, before the tone map).
pub fn clear(phase: u8) -> [f32; 4] {
    if phase == 0 { [0.0, 0.0, 0.0, 1.0] } else { [1.0, 1.0, 1.0, 1.0] }
}

/// The background as the tone map put it out: the median of the four corners.
fn corner(px: &[u8], w: u32, h: u32) -> [f32; 3] {
    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i] as f32, px[i + 1] as f32, px[i + 2] as f32]
    };
    let c = [at(0, 0), at(w - 1, 0), at(0, h - 1), at(w - 1, h - 1)];
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut v: Vec<f32> = c.iter().map(|p| p[k]).collect();
        v.sort_by(f32::total_cmp);
        *o = (v[1] + v[2]) * 0.5;
    }
    out
}

/// RGBA (straight alpha) from the same frame over black and over white (RGBA8 each, alpha ignored).
pub fn matte(black: &[u8], white: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (kb, kw) = (corner(black, w, h), corner(white, w, h));
    let span: Vec<f32> = (0..3).map(|k| (kw[k] - kb[k]).max(1.0)).collect();
    let mut out = vec![0u8; (w * h * 4) as usize];
    for i in 0..(w * h) as usize {
        let (b, wh) = (&black[i * 4..i * 4 + 3], &white[i * 4..i * 4 + 3]);
        // (the background's share: how far the pixel moved with it)
        let mut a = 0.0;
        for k in 0..3 {
            a += 1.0 - ((wh[k] as f32 - b[k] as f32) / span[k]).clamp(0.0, 1.0);
        }
        let a = a / 3.0;
        if a < 0.5 / 255.0 {
            continue;
        }
        for k in 0..3 {
            out[i * 4 + k] = ((b[k] as f32 - (1.0 - a) * kb[k]) / a).round().clamp(0.0, 255.0) as u8;
        }
        out[i * 4 + 3] = (a * 255.0).round() as u8;
    }
    out
}

/// The box of the pixels with cover above 2 / 255 (x0, y0, x1, y1 exclusive); none when the picture is empty.
pub fn bounds(px: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if px[((y * w + x) * 4 + 3) as usize] > 2 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x1 > x0).then_some((x0, y0, x1, y1))
}

/// The car's box of `px` scaled to fit `size` (aspect kept, centred, clear round it), or cut out as it is without a
/// size. Scaled down by the area each output pixel covers, up bilinearly; colour weighted by cover.
pub fn fit(px: &[u8], w: u32, h: u32, size: Option<(u32, u32)>) -> Option<(Vec<u8>, u32, u32)> {
    let (x0, y0, x1, y1) = bounds(px, w, h)?;
    let (cw, ch) = (x1 - x0, y1 - y0);
    let Some((ow, oh)) = size else {
        let mut out = Vec::with_capacity((cw * ch * 4) as usize);
        for y in y0..y1 {
            out.extend_from_slice(&px[((y * w + x0) * 4) as usize..((y * w + x1) * 4) as usize]);
        }
        return Some((out, cw, ch));
    };
    let s = (ow as f32 / cw as f32).min(oh as f32 / ch as f32);
    let (dw, dh) = (cw as f32 * s, ch as f32 * s);
    let (ox, oy) = ((ow as f32 - dw) * 0.5, (oh as f32 - dh) * 0.5);
    // (premultiplied source, in f32)
    let src = |x: i64, y: i64| -> [f32; 4] {
        if x < 0 || y < 0 || x >= cw as i64 || y >= ch as i64 {
            return [0.0; 4];
        }
        let i = (((y as u32 + y0) * w + x as u32 + x0) * 4) as usize;
        let a = px[i + 3] as f32 / 255.0;
        [px[i] as f32 * a, px[i + 1] as f32 * a, px[i + 2] as f32 * a, a]
    };
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for y in 0..oh {
        for x in 0..ow {
            let mut acc = [0.0f32; 4];
            if s < 1.0 {
                // (the source rectangle under this pixel, each source pixel by its overlap)
                let (sx0, sx1) = ((x as f32 - ox) / s, (x as f32 + 1.0 - ox) / s);
                let (sy0, sy1) = ((y as f32 - oy) / s, (y as f32 + 1.0 - oy) / s);
                let mut wsum = 0.0;
                for yy in sy0.floor() as i64..sy1.ceil() as i64 {
                    let wy = (sy1.min(yy as f32 + 1.0) - sy0.max(yy as f32)).max(0.0);
                    for xx in sx0.floor() as i64..sx1.ceil() as i64 {
                        let wx = (sx1.min(xx as f32 + 1.0) - sx0.max(xx as f32)).max(0.0);
                        let p = src(xx, yy);
                        for k in 0..4 {
                            acc[k] += p[k] * wx * wy;
                        }
                        wsum += wx * wy;
                    }
                }
                if wsum > 0.0 {
                    acc.iter_mut().for_each(|v| *v /= wsum);
                }
            } else {
                // (outside the scaled cut: clear)
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                if cx < ox || cx > ox + dw || cy < oy || cy > oy + dh {
                    continue;
                }
                let (fx, fy) = ((x as f32 + 0.5 - ox) / s - 0.5, (y as f32 + 0.5 - oy) / s - 0.5);
                let (ix, iy) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - ix, fy - iy);
                for (dx, dy, wt) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
                    // (clamped to the cut: its edge is the car's own)
                    let p = src((ix as i64 + dx).clamp(0, cw as i64 - 1), (iy as i64 + dy).clamp(0, ch as i64 - 1));
                    for k in 0..4 {
                        acc[k] += p[k] * wt;
                    }
                }
            }
            let i = ((y * ow + x) * 4) as usize;
            if acc[3] > 0.0 {
                for k in 0..3 {
                    out[i + k] = (acc[k] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
                out[i + 3] = (acc[3] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some((out, ow, oh))
}

/// The picture frames as they come back from the GPU.
#[derive(Default)]
pub struct Shots {
    black: Option<Vec<u8>>,
}

impl Shots {
    /// Take picture frame `phase`; after the second, write the picture. Some(exit code) when the run is over.
    pub fn take(&mut self, cfg: &Cfg, phase: u8, rgba: Vec<u8>, w: u32, h: u32) -> Option<i32> {
        if phase == 0 {
            self.black = Some(rgba);
            return None;
        }
        let black = self.black.take()?;
        let Some((px, ow, oh)) = fit(&matte(&black, &rgba, w, h), w, h, cfg.size) else {
            log::error!("car picture: {} covers nothing in the frame", cfg.car);
            return Some(3);
        };
        match write_png(&cfg.out, &px, ow, oh) {
            Ok(()) => {
                log::info!("car picture: {} written to {} ({} x {})", cfg.car, cfg.out, ow, oh);
                Some(0)
            }
            Err(e) => {
                log::error!("car picture: writing {}: {e}", cfg.out);
                Some(4)
            }
        }
    }
}

fn write_png(path: &str, px: &[u8], w: u32, h: u32) -> std::io::Result<()> {
    let f = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(std::io::Error::other)?;
    wr.write_image_data(px).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4 x 4 frame over a background `bg` with one pixel of colour `c` and cover `a` at (1, 1).
    fn frame(bg: u8, c: [f32; 3], a: f32) -> Vec<u8> {
        let mut v = vec![bg; 64];
        for p in v.chunks_exact_mut(4) {
            p[3] = 255;
        }
        for k in 0..3 {
            v[(4 + 1) * 4 + k] = (c[k] * a + bg as f32 * (1.0 - a)).round() as u8;
        }
        v
    }

    #[test]
    fn matte_gives_back_colour_and_cover() {
        let m = matte(&frame(0, [200.0, 100.0, 40.0], 0.5), &frame(240, [200.0, 100.0, 40.0], 0.5), 4, 4);
        let p = &m[(4 + 1) * 4..(4 + 1) * 4 + 4];
        assert!((p[3] as i32 - 128).abs() <= 1, "{p:?}");
        assert!((p[0] as i32 - 200).abs() <= 2 && (p[1] as i32 - 100).abs() <= 2 && (p[2] as i32 - 40).abs() <= 2, "{p:?}");
        // (the background is clear, the solid pixel solid)
        assert_eq!(m[3], 0);
        let s = matte(&frame(0, [10.0, 20.0, 30.0], 1.0), &frame(240, [10.0, 20.0, 30.0], 1.0), 4, 4);
        assert_eq!(&s[(4 + 1) * 4..(4 + 1) * 4 + 4], &[10, 20, 30, 255]);
    }

    #[test]
    fn fit_cuts_and_centres() {
        // a 2 x 1 car in an 8 x 8 frame
        let mut px = vec![0u8; 8 * 8 * 4];
        for x in [3, 4] {
            px[(5 * 8 + x) * 4..(5 * 8 + x) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
        assert_eq!(bounds(&px, 8, 8), Some((3, 5, 5, 6)));
        let (cut, w, h) = fit(&px, 8, 8, None).unwrap();
        assert_eq!((w, h, cut.len()), (2, 1, 8));
        // into 4 x 4: 4 x 2 across the middle rows
        let (out, w, h) = fit(&px, 8, 8, Some((4, 4))).unwrap();
        assert_eq!((w, h), (4, 4));
        let a = |x: usize, y: usize| out[(y * 4 + x) * 4 + 3];
        assert_eq!((a(0, 0), a(0, 3)), (0, 0));
        assert!(a(0, 1) == 255 && a(3, 2) == 255);
        assert_eq!(out[(4 + 1) * 4], 255);
        assert!(fit(&vec![0u8; 64], 4, 4, Some((2, 2))).is_none());
    }

    #[test]
    fn options() {
        assert_eq!(parse_size("512x128"), Some((512, 128)));
        assert_eq!(parse_size("0x5"), None);
        assert_eq!(View::parse("SIDE"), Some(View::Side));
        assert_eq!(View::parse("front"), None);
        let c = Cfg { warm: 10, ..Default::default() };
        assert_eq!((c.phase(9), c.phase(10), c.phase(11), c.phase(12)), (None, Some(0), Some(1), Some(1)));
    }
}
