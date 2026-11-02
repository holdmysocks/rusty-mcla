//! The logo of a rim mod's own brand tab (docs/MODS.md M-6 item 4).
//!
//! The rim panel (`rimselection`, template 47) shows the brand's logo in its `vs_brands` clip: one frame per maker, each a
//! 256 x 64 bitmap quad (uv 0..1) over the panel's backdrop shape - read off the movie, `flash_ui::tests::rim_tab_logo_frames`.
//! A brand that only mods make has no frame, so its tab shows frame 0 (`mcla_mods::rims::CUSTOM_LOGO_MFR`) with the clip's
//! bitmap swapped for the picture made here (`FlashMovie::swap_handle`, the way the game's own dyntexture clips take theirs):
//! the mod's PNG fitted into 256 x 64, or - when the mod brought none - the brand's name drawn as text. The movie is not
//! touched; the picture sits where a logo sits, scaled and warped with the panel.

pub const W: usize = 256;
pub const H: usize = 64;

/// The 256 x 64 picture for a brand: its PNG fitted in (aspect kept, centred), else its name as text. Straight RGBA.
pub fn image(name: &str, png: Option<&[u8]>) -> (usize, usize, Vec<u8>) {
    if let Some(p) = png.and_then(fit_png) {
        return (W, H, p);
    }
    (W, H, text(name))
}

fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0u8; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let src = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => src.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => src.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => src.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    (w > 0 && h > 0 && rgba.len() == w * h * 4).then_some((w, h, rgba))
}

/// A mod's garage picture (MODS.md M-7): the PNG decoded, and scaled to `size` (the disc picture's) when it has another
/// one - a triangle filter (bilinear when enlarging, averaging when reducing) over alpha-weighted colours, so the
/// picture fills the quad the game's own picture fills. Straight RGBA; None when the bytes are no readable PNG.
pub fn picture(bytes: &[u8], size: Option<(u32, u32)>) -> Option<(usize, usize, Vec<u8>)> {
    let (w, h, px) = decode(bytes)?;
    match size {
        Some((tw, th)) if (tw as usize, th as usize) != (w, h) && tw > 0 && th > 0 => Some((tw as usize, th as usize, resample(&px, w, h, tw as usize, th as usize))),
        _ => Some((w, h, px)),
    }
}

/// Separable triangle-filter resize of straight RGBA (alpha-weighted).
fn resample(src: &[u8], w: usize, h: usize, tw: usize, th: usize) -> Vec<u8> {
    // premultiplied floats
    let mut a: Vec<f32> = src.chunks_exact(4).flat_map(|p| {
        let al = p[3] as f32 / 255.0;
        [p[0] as f32 * al, p[1] as f32 * al, p[2] as f32 * al, al * 255.0]
    }).collect();
    // one axis: `n` samples of `len` lines to `m` samples
    let pass = |data: &[f32], n: usize, m: usize, lines: usize, stride_n: usize, stride_l: usize| -> Vec<f32> {
        let scale = n as f32 / m as f32;
        let r = scale.max(1.0);
        let mut out = vec![0.0f32; m * lines * 4];
        for x in 0..m {
            let c = (x as f32 + 0.5) * scale - 0.5;
            let (lo, hi) = ((c - r).ceil().max(0.0) as usize, ((c + r).floor() as i64).clamp(0, n as i64 - 1) as usize);
            let taps: Vec<(usize, f32)> = (lo..=hi).map(|i| (i, (1.0 - (i as f32 - c).abs() / r).max(0.0))).filter(|t| t.1 > 0.0).collect();
            let total: f32 = taps.iter().map(|t| t.1).sum::<f32>().max(1e-6);
            for l in 0..lines {
                let mut acc = [0.0f32; 4];
                for &(i, wt) in &taps {
                    let o = (i * stride_n + l * stride_l) * 4;
                    for k in 0..4 {
                        acc[k] += data[o + k] * wt;
                    }
                }
                // (output laid out with the resampled axis in the same stride role as the input's)
                let o = if stride_n == 1 { (l * m + x) * 4 } else { (x * lines + l) * 4 };
                for k in 0..4 {
                    out[o + k] = acc[k] / total;
                }
            }
        }
        out
    };
    // horizontal: lines = rows (stride_n 1, stride_l w) ; vertical: lines = columns (stride_n tw, stride_l 1)
    a = pass(&a, w, tw, h, 1, w);
    a = pass(&a, h, th, tw, tw, 1);
    a.chunks_exact(4).flat_map(|p| {
        let al = p[3] / 255.0;
        if al > 0.0 {
            [(p[0] / al).round().clamp(0.0, 255.0) as u8, (p[1] / al).round().clamp(0.0, 255.0) as u8, (p[2] / al).round().clamp(0.0, 255.0) as u8, p[3].round().clamp(0.0, 255.0) as u8]
        } else {
            [0, 0, 0, 0]
        }
    }).collect()
}

/// The PNG fitted into the canvas (bilinear, alpha-weighted so a transparent edge does not darken the rim).
fn fit_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let (w, h, src) = decode(bytes)?;
    let k = (W as f32 / w as f32).min(H as f32 / h as f32);
    let (dw, dh) = (((w as f32 * k).round() as usize).clamp(1, W), ((h as f32 * k).round() as usize).clamp(1, H));
    let (x0, y0) = ((W - dw) / 2, (H - dh) / 2);
    let mut out = vec![0u8; W * H * 4];
    let px = |x: usize, y: usize| -> [f32; 4] {
        let i = (y.min(h - 1) * w + x.min(w - 1)) * 4;
        let a = src[i + 3] as f32 / 255.0;
        [src[i] as f32 * a, src[i + 1] as f32 * a, src[i + 2] as f32 * a, a]
    };
    for y in 0..dh {
        for x in 0..dw {
            // (the source point of the destination pixel's centre; a reduction averages a box of source pixels)
            let (sx0, sx1) = (x as f32 / k, ((x + 1) as f32 / k).max(x as f32 / k + 1.0));
            let (sy0, sy1) = (y as f32 / k, ((y + 1) as f32 / k).max(y as f32 / k + 1.0));
            let (mut acc, mut n) = ([0.0f32; 4], 0.0f32);
            let (ix0, ix1, iy0, iy1) = (sx0.floor() as usize, (sx1.ceil() as usize).min(w), sy0.floor() as usize, (sy1.ceil() as usize).min(h));
            for sy in iy0..iy1.max(iy0 + 1) {
                for sx in ix0..ix1.max(ix0 + 1) {
                    let p = px(sx, sy);
                    for c in 0..4 {
                        acc[c] += p[c];
                    }
                    n += 1.0;
                }
            }
            let a = acc[3] / n;
            let o = ((y0 + y) * W + x0 + x) * 4;
            if a > 0.0 {
                for c in 0..3 {
                    out[o + c] = (acc[c] / n / a).round().clamp(0.0, 255.0) as u8;
                }
            }
            out[o + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Some(out)
}

/// The brand's name in capitals, white, as large as fits the canvas (at most 44 px), centred.
fn text(name: &str) -> Vec<u8> {
    use ab_glyph::{Font, FontRef, ScaleFont};
    let mut out = vec![0u8; W * H * 4];
    let defs = egui::FontDefinitions::default();
    let Some(data) = defs.font_data.get("Ubuntu-Light").or_else(|| defs.font_data.values().next()) else { return out };
    let Ok(font) = FontRef::try_from_slice(&data.font) else { return out };
    let name = name.trim().to_uppercase();
    let mut size = 44.0f32;
    let width = |size: f32| {
        let f = font.as_scaled(ab_glyph::PxScale::from(size));
        let mut x = 0.0;
        let mut prev = None;
        for c in name.chars() {
            let g = f.glyph_id(c);
            if let Some(p) = prev {
                x += f.kern(p, g);
            }
            x += f.h_advance(g);
            prev = Some(g);
        }
        x
    };
    while size > 8.0 && width(size) > (W - 16) as f32 {
        size -= 1.0;
    }
    let f = font.as_scaled(ab_glyph::PxScale::from(size));
    let mut x = (W as f32 - width(size)) / 2.0;
    let base = (H as f32 - (f.ascent() + f.descent())) / 2.0 + f.ascent() - 2.0;
    let mut prev = None;
    for c in name.chars() {
        let id = f.glyph_id(c);
        if let Some(p) = prev {
            x += f.kern(p, id);
        }
        let glyph = id.with_scale_and_position(ab_glyph::PxScale::from(size), ab_glyph::point(x, base));
        if let Some(o) = font.outline_glyph(glyph) {
            let b = o.px_bounds();
            o.draw(|gx, gy, cov| {
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if (0..W as i32).contains(&px) && (0..H as i32).contains(&py) {
                    let i = (py as usize * W + px as usize) * 4;
                    let a = ((cov * 255.0) as u8).max(out[i + 3]);
                    out[i..i + 3].copy_from_slice(&[255, 255, 255]);
                    out[i + 3] = a;
                }
            });
        }
        x += f.h_advance(id);
        prev = Some(id);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(&(0..w * h).flat_map(|_| rgba).collect::<Vec<u8>>()).unwrap();
        drop(wr);
        out
    }

    fn bbox(img: &[u8]) -> Option<(usize, usize, usize, usize)> {
        let mut b: Option<(usize, usize, usize, usize)> = None;
        for y in 0..H {
            for x in 0..W {
                if img[(y * W + x) * 4 + 3] > 8 {
                    b = Some(b.map_or((x, y, x, y), |b| (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y))));
                }
            }
        }
        b
    }

    /// A wide PNG fills the width, a tall one the height; both keep the aspect and sit in the middle.
    #[test]
    fn a_png_is_fitted_and_centred() {
        let (w, h, a) = image("X", Some(&png(512, 64, [200, 10, 10, 255])));
        assert_eq!((w, h), (256, 64));
        assert_eq!(bbox(&a), Some((0, 16, 255, 47)), "512x64 -> 256x32 in the middle");
        assert_eq!(&a[(30 * W + 100) * 4..(30 * W + 100) * 4 + 4], &[200, 10, 10, 255]);
        let (_, _, b) = image("X", Some(&png(16, 64, [1, 2, 3, 255])));
        assert_eq!(bbox(&b), Some((120, 0, 135, 63)), "16x64 stays 16 wide");
    }

    /// Without a PNG (or with a broken one) the name is drawn: capitals, inside the canvas, centred.
    #[test]
    fn a_name_is_drawn_as_text() {
        for img in [image("Acme Wheels", None).2, image("Acme Wheels", Some(b"not a png")).2] {
            let (x0, y0, x1, y1) = bbox(&img).expect("text drawn");
            assert!(x0 >= 8 && x1 <= W - 8 && y0 >= 2 && y1 < H - 2, "{x0} {y0} {x1} {y1}");
            let (l, r) = (x0, W - 1 - x1);
            assert!((l as i32 - r as i32).abs() <= 4, "centred: {l} vs {r}");
            assert!(y1 - y0 >= 20, "large enough to read");
        }
        // a long name shrinks to fit
        let (x0, _, x1, _) = bbox(&image("A VERY LONG BRAND NAME HERE", None).2).unwrap();
        assert!(x0 >= 4 && x1 <= W - 4);
        // the name is not the same as another name
        assert_ne!(image("AAA", None).2, image("BBB", None).2);
    }

    /// A mod's garage picture of another size is scaled to the disc picture's; one of the right size is left as it is.
    #[test]
    fn a_garage_picture_is_scaled_to_the_discs_size() {
        let (w, h, a) = picture(&png(1024, 256, [10, 200, 30, 255]), Some((512, 128))).unwrap();
        assert_eq!((w, h, a.len()), (512, 128, 512 * 128 * 4));
        assert!(a.chunks_exact(4).all(|p| p == [10, 200, 30, 255]), "a flat picture stays flat");
        let (w, h, b) = picture(&png(64, 32, [255, 0, 0, 255]), Some((128, 64))).unwrap();
        assert_eq!((w, h), (128, 64));
        assert!(b.chunks_exact(4).all(|p| p == [255, 0, 0, 255]));
        // the right size is not touched; no size asked: as it is
        assert_eq!(picture(&png(8, 4, [1, 2, 3, 255]), Some((8, 4))).unwrap().2, picture(&png(8, 4, [1, 2, 3, 255]), None).unwrap().2);
        assert!(picture(b"not a png", Some((8, 4))).is_none());
        // a hard edge stays where it is (left half red, right half blue, 2x1 -> 8x4): left columns red, right columns blue
        let mut e = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut e, 2, 1);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        }
        let (_, _, c) = picture(&e, Some((8, 4))).unwrap();
        assert_eq!(&c[0..4], &[255, 0, 0, 255]);
        assert_eq!(&c[7 * 4..7 * 4 + 4], &[0, 0, 255, 255]);
        assert!(c[3 * 4] > 100 && c[3 * 4] < 255 || c[3 * 4 + 2] > 0, "a blend across the edge");
        // transparency survives: a transparent picture does not turn dark at its edges
        let (_, _, d) = picture(&png(4, 4, [200, 100, 50, 0]), Some((16, 16))).unwrap();
        assert!(d.chunks_exact(4).all(|p| p[3] == 0));
    }
}
