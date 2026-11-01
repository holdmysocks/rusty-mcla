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
}
