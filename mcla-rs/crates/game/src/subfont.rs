//! The cutscene subtitles' font and text layout as the original's (docs/reference/ui/subtitle_text.md): the bitmap
//! font `Arial12` (`ui/fonts`: the descriptor `arial12.fonttex` read as F-61630 reads it, three 128 x 128 glyph pages
//! and three outline pages), laid out by the text renderer F-74945 with the subtitle object's layout record (flags 9:
//! wrap inside the rect, each line centred; F-94291) in the rect F-58587 computes from the table's position / size
//! rows, and drawn in two passes: the outline pages, then the glyph pages, both in the line's colour (F-37072).

use std::path::Path;

/// One glyph of the descriptor: its code, texel cell on its page, top relative to the baseline and advance minus width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub code: u32,
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
    pub yoff: i8,
    pub adv: i8,
    pub page: usize,
}

#[derive(Clone, Debug)]
pub struct Font {
    /// font +40: the pen's step to the next line
    pub line_height: i32,
    /// font +44: added to every glyph's advance
    pub spacing: i32,
    /// font +48: the advance of a space and of a character with no glyph
    pub space: i32,
    /// font +56: added to the pen's y to place a glyph
    pub baseline: i32,
    pub glyphs: Vec<Glyph>,
    /// the outline pages exist (header words +28..+40 not all zero)
    pub outline: bool,
}

/// One glyph quad in screen pixels (whole pixels, as the original snaps them) and its texel rectangle (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub page: usize,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub uv: [f32; 4],
}

/// The renderer's pen start offset (F-74945, a constant of its own).
const K: f32 = 0.499;

impl Font {
    /// The descriptor (little-endian): f32 version, u32 line height, spacing, space advance, monospace advance,
    /// baseline (version >= 1.02), extra descent, four shadow words, u32 count, 10-byte glyphs {u32 code, u8 x, y,
    /// w, h, i8 yoff, adv}, u32 page count, page end indices.
    pub fn parse(d: &[u8]) -> Option<Font> {
        let u = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
        let version = f32::from_le_bytes(d.get(0..4)?.try_into().ok()?);
        let baseline = if version >= 1.015 { u(20)? as i32 } else { 0 };
        let outline = (28..44).step_by(4).any(|o| u(o) != Some(0));
        let n = u(44)? as usize;
        let mut glyphs = Vec::with_capacity(n);
        for i in 0..n {
            let o = 48 + i * 10;
            let b = d.get(o..o + 10)?;
            glyphs.push(Glyph { code: u(o)?, x: b[4], y: b[5], w: b[6], h: b[7], yoff: b[8] as i8, adv: b[9] as i8, page: 0 });
        }
        let p = 48 + n * 10;
        let pages = u(p)? as usize;
        let ends: Vec<usize> = (0..pages).map(|k| u(p + 4 + 4 * k).map(|e| e as usize)).collect::<Option<_>>()?;
        for (i, g) in glyphs.iter_mut().enumerate() {
            g.page = ends.iter().position(|&e| i < e).unwrap_or(pages.saturating_sub(1));
        }
        Some(Font { line_height: u(4)? as i32, spacing: u(8)? as i32, space: u(12)? as i32, baseline, glyphs, outline })
    }

    /// The glyph of a character: codes 10, 13, 8, 32, 160 have none; below 256 by the code table, above by search.
    fn glyph(&self, c: u16) -> Option<&Glyph> {
        if matches!(c, 10 | 13 | 8 | 32 | 160) {
            return None;
        }
        self.glyphs.iter().find(|g| g.code == c as u32)
    }

    /// F-74945 with the subtitle record (flags 9: wrap at `right`, centre each line): the quads of `text` from the
    /// pen start (left, top), x / y scale `sx` / `sy`. The rect's bottom is not used.
    pub fn layout(&self, text: &str, left: i32, top: i32, right: i32, sx: f32, sy: f32) -> Vec<Quad> {
        let chars: Vec<u16> = text.encode_utf16().collect();
        let n = chars.len();
        // per character: (x16, y16, w16, h16, glyph) and its line
        let mut pos: Vec<Option<(i32, i32, i32, i32, Glyph)>> = vec![None; n];
        let mut line_of = vec![0usize; n];
        let (mut px, mut py) = (left as f32 + K, top as f32 + K);
        let (mut line, mut line_start, mut last_space): (usize, usize, Option<usize>) = (0, 0, None);
        let mut i = 0;
        while i < n {
            let c = chars[i];
            line_of[i] = line;
            match c {
                10 => {
                    px = left as f32 + K;
                    py += self.line_height as f32;
                    line += 1;
                    line_start = i + 1;
                    last_space = None;
                }
                32 => {
                    last_space = Some(i);
                    px += self.space as f32;
                }
                8 => px -= (self.space / 4) as f32,
                _ => match self.glyph(c) {
                    None => px += self.space as f32,
                    Some(&g) => {
                        let mut qx = px * sx;
                        if qx + g.w as f32 * sx >= right as f32 {
                            let wrap_at = match last_space {
                                Some(s) if s >= line_start => Some(s + 1),
                                _ if i > line_start => Some(i),
                                _ => None,
                            };
                            if let Some(start) = wrap_at {
                                px = left as f32 + K;
                                py += self.line_height as f32;
                                line += 1;
                                line_start = start;
                                last_space = None;
                                for k in start..i {
                                    pos[k] = None;
                                }
                                if start < i {
                                    // the walk restarts after the space
                                    i = start;
                                    continue;
                                }
                                line_of[i] = line;
                                qx = px * sx;
                            }
                        }
                        let qy = (py + self.baseline as f32 + g.yoff as f32) * sy;
                        pos[i] = Some(((qx * 16.0) as i32, (qy * 16.0) as i32, (g.w as f32 * sx * 16.0) as i32, (g.h as f32 * sy * 16.0) as i32, g));
                        px += (g.w as i32 + g.adv as i32 + self.spacing) as f32;
                    }
                },
            }
            i += 1;
        }
        // centring: half the free space of each line, in 1/16 px
        for l in 0..=line {
            let Some(last) = (0..n).rev().find(|&k| line_of[k] == l && pos[k].is_some()) else { continue };
            let (x16, _, _, _, g) = pos[last].unwrap();
            let line_right = ((x16 / 16) as f32 + (g.w as i32 + g.adv as i32) as f32 * sx) as i32;
            let shift = (right - line_right) * 8;
            for k in 0..n {
                if line_of[k] == l {
                    if let Some(p) = pos[k].as_mut() {
                        p.0 += shift;
                    }
                }
            }
        }
        pos.into_iter()
            .flatten()
            .map(|(x16, y16, w16, h16, g)| Quad {
                page: g.page,
                x0: x16 >> 4,
                y0: y16 >> 4,
                x1: (x16 + w16) >> 4,
                y1: (y16 + h16) >> 4,
                uv: [g.x as f32 / 128.0, g.y as f32 / 128.0, (g.x as f32 + g.w as f32) / 128.0, (g.y as f32 + g.h as f32) / 128.0],
            })
            .collect()
    }
}

/// The subtitle placement (F-58587): left / top / right in pixels of a W x H screen from the table's position (x, y)
/// and size (w, h) rows.
pub fn rect(place: [f32; 4], w: f32, h: f32) -> (i32, i32, i32) {
    ((place[0] * w) as i32, (place[1] * h) as i32, ((place[0] + place[2]) * w) as i32)
}

/// The x scale (record +48): (W / H) over 16:9 on a wide screen, over 4:3 otherwise (F-94291).
pub fn x_scale(w: f32, h: f32) -> f32 {
    let wide = w / h > 1.5;
    (w / h) / if wide { f32::from_bits(0x3FE3_8E39) } else { f32::from_bits(0x3FAA_AAAB) }
}

/// The loaded font with its pages as egui textures: [glyph pages, outline pages].
pub struct SubFont {
    pub font: Font,
    pub pages: Vec<egui::TextureHandle>,
    pub outlines: Vec<egui::TextureHandle>,
}

impl SubFont {
    pub fn load(ctx: &egui::Context, data_dir: &str) -> Option<SubFont> {
        let dir = Path::new(data_dir).join("ui/fonts");
        let font = Font::parse(&std::fs::read(dir.join(format!("0x{:08x}", mcla_formats::hash::joaat("arial12.fonttex")))).ok()?)?;
        let npages = font.glyphs.iter().map(|g| g.page + 1).max().unwrap_or(0);
        let tex = |name: String| -> Option<egui::TextureHandle> {
            let d = std::fs::read(dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(&name)))).ok()?;
            let t = mcla_formats::dds::parse_dds(&name, &d).ok()?;
            let img = egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.to_rgba8_gpu());
            Some(ctx.load_texture(&name, img, egui::TextureOptions::NEAREST))
        };
        let pages: Vec<_> = (0..npages).map(|p| tex(format!("arial12_{p:02}.dds"))).collect::<Option<_>>()?;
        let outlines: Vec<_> = if font.outline { (0..npages).map(|p| tex(format!("arial12_{p:02}s.dds"))).collect::<Option<_>>()? } else { Vec::new() };
        Some(SubFont { font, pages, outlines })
    }

    /// Draw a subtitle line on a screen of `screen` points (`ppp` pixels a point) at the table's placing, in `colour`
    /// (white, or the thought colour): outline pass, then glyph pass, page by page in text order.
    pub fn draw(&self, painter: &egui::Painter, screen: egui::Rect, ppp: f32, place: [f32; 4], text: &str, colour: egui::Color32) {
        let (w, h) = (screen.width() * ppp, screen.height() * ppp);
        let (l, t, r) = rect(place, w, h);
        let quads = self.font.layout(text, l, t, r, x_scale(w, h), 1.0);
        for set in [&self.outlines, &self.pages] {
            for (p, tex) in set.iter().enumerate() {
                let mut mesh = egui::Mesh::with_texture(tex.id());
                for q in quads.iter().filter(|q| q.page == p) {
                    let rect = egui::Rect::from_min_max(screen.min + egui::vec2(q.x0 as f32, q.y0 as f32) / ppp, screen.min + egui::vec2(q.x1 as f32, q.y1 as f32) / ppp);
                    mesh.add_rect_with_uv(rect, egui::Rect::from_min_max(egui::pos2(q.uv[0], q.uv[1]), egui::pos2(q.uv[2], q.uv[3])), colour);
                }
                if !mesh.is_empty() {
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
        }
    }
}

/// The table's placing of the 18 story scenes (position 0.2 / 0.85, size 0.6 / 0.15).
pub const STORY_PLACE: [f32; 4] = [0.2, 0.85, 0.6, 0.15];
/// The film's placing (cut_intro's table: position 0.1 / 0.83, size 0.8 / 0.5).
pub const INTRO_PLACE: [f32; 4] = [0.1, 0.83, 0.8, 0.5];
/// The thought colour (F-94291: 0.75, 0.9, 0.95, packed by F-44647).
pub const THOUGHT: egui::Color32 = egui::Color32::from_rgb(0xBF, 0xE5, 0xF2);

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule file's worked example (docs/reference/ui/subtitle_text.md section 6): "Wassup, esse?" at 1280 x 720
    /// in the story scenes' rect 256 / 612 / 1024: W at 572, 616 - 590, 633, `?` at 699, 616 - 711, 633, and the
    /// whole line's quads as listed there.
    #[test]
    fn subtitle_layout_as_the_rule() {
        let Ok(dir) = std::env::var("MCLA_DATA_DIR") else { return };
        let d = std::fs::read(Path::new(&dir).join(format!("ui/fonts/0x{:08x}", mcla_formats::hash::joaat("arial12.fonttex")))).unwrap();
        let f = Font::parse(&d).unwrap();
        assert_eq!((f.line_height, f.spacing, f.space, f.baseline, f.glyphs.len(), f.outline), (19, 2, 6, 25, 171, true));
        assert_eq!(rect(STORY_PLACE, 1280.0, 720.0), (256, 612, 1024));
        assert_eq!(x_scale(1280.0, 720.0), 1.0);
        let q = f.layout("Wassup, esse?", 256, 612, 1024, 1.0, 1.0);
        let boxes: Vec<(i32, i32, i32, i32, usize)> = q.iter().map(|q| (q.x0, q.y0, q.x1, q.y1, q.page)).collect();
        assert_eq!(
            boxes,
            [(572, 616, 590, 633, 0), (589, 620, 601, 633, 0), (600, 620, 612, 633, 1), (611, 620, 623, 633, 1), (622, 620, 634, 633, 1), (633, 620, 645, 637, 1), (644, 628, 650, 635, 0), (655, 620, 667, 633, 0), (666, 620, 678, 633, 1), (677, 620, 689, 633, 1), (688, 620, 700, 633, 0), (699, 616, 711, 633, 0)]
        );
        // (a line wider than the rect breaks at its last space, both lines centred, the second 19 px lower)
        let long = "a ".repeat(60);
        let long = long.trim_end();
        let q = f.layout(long, 256, 612, 1024, 1.0, 1.0);
        let ys: std::collections::BTreeSet<i32> = q.iter().map(|q| q.y0).collect();
        assert!(ys.len() == 2 && ys.iter().next_back().unwrap() - ys.iter().next().unwrap() == 19, "{ys:?}");
        assert!(q.iter().all(|q| q.x0 >= 256 && q.x1 <= 1024));
    }
}
