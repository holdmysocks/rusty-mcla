//! Display list -> triangle batches, following the original draw routines:
//! instance traversal `F-92070`, colour transform combine `F-57014`,
//! shape `F-19892` (fill/line styles, triangle lists, gradients evaluated
//! per vertex `F-54894`), thick lines `F-78733`, static text
//! `F-11515` with glyph quads `F-83474`.
//!
//! Output coordinates are stage pixels; the host scales the stage to its screen.

use crate::player::Player;
use crate::value::InstId;
use mcla_formats::rage::swf::{Character, CxForm, FillStyle, Matrix, Movie, Shape, ShapeCmd, Text, TextRecord};

/// What a batch samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TexRef {
    None,
    /// bitmap character of the movie
    Bitmap(u16),
    /// glyph sheet `sheet` of page `page` of font character `font`
    Glyph { font: u16, page: u8, sheet: u8 },
    /// a texture of the host's, by the key on the clip (`Inst::tex_swap`)
    Swap(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    /// stage pixels
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    /// RGBA, straight alpha
    pub color: [u8; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// triangle list; `kind` is the SWF fill type for bitmap fills (0x40 repeat, 0x41 clamp ...)
    Triangles { tex: TexRef, kind: u8, verts: Vec<Vertex> },
    /// following triangles until [`Item::MaskEnd`] define a mask instead of drawing
    MaskBegin,
    MaskEnd,
    /// remove the innermost mask
    MaskPop,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct DrawList {
    pub items: Vec<Item>,
}

impl DrawList {
    fn tris(&mut self, tex: TexRef, kind: u8, v: &[Vertex]) {
        if v.is_empty() {
            return;
        }
        if let Some(Item::Triangles { tex: t, kind: k, verts }) = self.items.last_mut() {
            if *t == tex && *k == kind {
                verts.extend_from_slice(v);
                return;
            }
        }
        self.items.push(Item::Triangles { tex, kind, verts: v.to_vec() });
    }

    pub fn triangle_count(&self) -> usize {
        self.items.iter().map(|i| if let Item::Triangles { verts, .. } = i { verts.len() / 3 } else { 0 }).sum()
    }
}

/// World colour transform of an instance (`F-57014`): the runtime's byte
/// arithmetic, including truncation of the multiplier product to a signed byte,
/// then the parent's tint bytes (RGB from `setRGB`/`textColor`, A from `_alpha`).
pub fn combine_cx(local: &CxForm, parent: &CxForm, tint: [u8; 4]) -> CxForm {
    let mut out = CxForm::IDENTITY;
    for i in 0..4 {
        out.add[i] = (((local.add[i] as i32 * parent.mult[i] as i32) >> 6) + parent.add[i] as i32) as i8;
        out.mult[i] = ((((local.mult[i] as i32 * parent.mult[i] as i32) >> 6) as i8 as i32 * tint[i] as i32) / 255) as i8;
    }
    out
}

/// ARGB through a colour transform -> RGBA (`F-25297`); the alpha is then
/// scaled by the drawn instance's own alpha byte.
pub fn apply_cx(argb: u32, cx: &CxForm, alpha: u8) -> [u8; 4] {
    let ch = |c: u32, i: usize| -> u8 { (((c as i32 * cx.mult[i] as i32) >> 6) + cx.add[i] as i32 * 4).clamp(0, 255) as u8 };
    let a = ch(argb >> 24, 3) as i32 * alpha as i32 / 255;
    [ch((argb >> 16) & 0xff, 0), ch((argb >> 8) & 0xff, 1), ch(argb & 0xff, 2), a as u8]
}

/// Gradient colour at a point in twips (`F-54894`), ARGB.
pub fn gradient_color(fill: &FillStyle, x: f32, y: f32) -> u32 {
    let Some(g) = &fill.gradient else { return fill.color };
    let m = &g.matrix;
    let (gx, gy) = (m.c.mul_add(y, m.a * x) + m.tx, m.b.mul_add(x, m.d * y) + m.ty);
    let t = if g.radial { gx.mul_add(gx, gy * gy).sqrt() * 0.015_563_964 } else { (gx + 16384.0) * 0.007_781_982_4 };
    // (below 0 -> 0; not up to 255, NaN included -> 255)
    let t = if t < 0.0 { 0.0 } else if t <= 255.0 { t } else { 255.0 };
    let t = t as u32 as u8;
    let n = g.ratios.len().min(g.colors.len());
    for i in 0..n.saturating_sub(1) {
        let (r0, r1) = (g.ratios[i], g.ratios[i + 1]);
        if t >= r0 && t <= r1 {
            let (c0, c1) = (g.colors[i], g.colors[i + 1]);
            let span = (r1 - r0) as u32;
            let f = (t - r0) as u32;
            if span == 0 {
                return c0;
            }
            // (each channel's sum in unsigned 32 bits, shifted into place and or'ed without a mask: a falling
            // channel's wrapped quotient spills into the channels above it, as in the original)
            let lerp = |sh: u32| -> u32 {
                let a = (c0 >> sh) & 0xff;
                let b = (c1 >> sh) & 0xff;
                a.wrapping_add((b.wrapping_sub(a)).wrapping_mul(f) / span)
            };
            return lerp(24) << 24 | lerp(16) << 16 | lerp(8) << 8 | lerp(0);
        }
    }
    g.colors.first().copied().unwrap_or(fill.color)
}

const TWIP: f32 = 0.05;

pub struct Builder<'a> {
    pub movie: &'a Movie,
    pub out: DrawList,
    /// alpha byte of the instance being drawn
    alpha: u8,
    /// host texture of the clip being drawn (0 = the movie's own bitmaps)
    swap: u32,
    /// draw the clips of `Player::key_caps` as authored (measuring them)
    no_caps: bool,
}

impl<'a> Builder<'a> {
    pub fn new(movie: &'a Movie) -> Self {
        Builder { movie, out: DrawList::default(), alpha: 255, swap: 0, no_caps: false }
    }

    /// Draw an instance's children (depth order, with clip-depth masks).
    fn children(&mut self, p: &Player, id: InstId, world: &Matrix, cx: &CxForm, tint: [u8; 4]) {
        let Some(inst) = p.inst(id) else { return };
        let mut masks: Vec<u16> = Vec::new();
        for &c in inst.children.values() {
            let Some(ci) = p.inst(c) else { continue };
            while let Some(&end) = masks.last() {
                if ci.depth > end {
                    masks.pop();
                    self.out.items.push(Item::MaskPop);
                } else {
                    break;
                }
            }
            if ci.clip_depth > 0 {
                self.out.items.push(Item::MaskBegin);
                self.instance(p, c, world, cx, tint);
                self.out.items.push(Item::MaskEnd);
                masks.push(ci.clip_depth);
                continue;
            }
            self.instance(p, c, world, cx, tint);
        }
        for _ in masks {
            self.out.items.push(Item::MaskPop);
        }
    }

    /// `F-92070`: world transform and colour, then children if the instance
    /// has any, else its character.
    pub fn instance(&mut self, p: &Player, id: InstId, parent: &Matrix, pcx: &CxForm, ptint: [u8; 4]) {
        let Some(inst) = p.inst(id) else { return };
        let world = parent.concat(&inst.matrix);
        let cx = combine_cx(&inst.cxform, pcx, ptint);
        if cx.mult[3] == 0 {
            return;
        }
        if !self.no_caps && !p.key_caps.is_empty() {
            if let Some((_, t, argb)) = p.key_caps.iter().find(|(n, ..)| inst.name.eq_ignore_ascii_case(n)) {
                self.alpha = inst.tint[3];
                self.key_cap(p, id, t, *argb, parent, &cx);
                return;
            }
        }
        if inst.tex_swap != 0 {
            // (this clip and what it holds take the host's texture)
            let outer = std::mem::replace(&mut self.swap, inst.tex_swap);
            self.instance_body(p, id, inst, &world, &cx);
            self.swap = outer;
            return;
        }
        self.instance_body(p, id, inst, &world, &cx);
    }

    fn instance_body(&mut self, p: &Player, id: InstId, inst: &crate::player::Inst, world: &Matrix, cx: &CxForm) {
        let (world, cx) = (*world, *cx);
        if !inst.children.is_empty() {
            self.children(p, id, &world, &cx, inst.tint);
            return;
        }
        self.alpha = inst.tint[3];
        match self.movie.char(inst.character) {
            Some(Character::Shape(s)) => self.shape(s, &world, &cx),
            Some(Character::Text(t)) => self.text(t, &world, &cx),
            Some(Character::EditText(e)) => {
                let text = match &inst.text {
                    Some(t) => t.to_string(),
                    None if !e.variable.is_empty() => p.get_global(&e.variable).map(|v| v.to_string().to_string()).unwrap_or_default(),
                    None => e.initial.clone(),
                };
                self.edit_text(inst.character, &text, &world, &cx, inst.tint);
            }
            _ => {}
        }
    }

    pub fn shape(&mut self, s: &Shape, world: &Matrix, cx: &CxForm) {
        let default_fill = FillStyle { kind: 0, color: 0xffff_ffff, matrix: Matrix::IDENTITY, bitmap: None, gradient: None };
        let mut fill = &default_fill;
        let mut line_w = 20.0f32;
        let mut line_c = 0xffff_ffffu32;
        let mut buf: Vec<Vertex> = Vec::new();
        for cmd in &s.cmds {
            match cmd {
                ShapeCmd::Fill(i) if *i > 0 => {
                    if let Some(f) = s.fills.get(*i as usize - 1) {
                        fill = f;
                    }
                }
                ShapeCmd::Line(i) if *i > 0 => {
                    if let Some(l) = s.lines.get(*i as usize - 1) {
                        line_w = l.width as f32;
                        line_c = l.color;
                    }
                }
                ShapeCmd::Triangles(pts) => {
                    buf.clear();
                    for p in pts {
                        buf.push(self.fill_vertex(fill, world, cx, p[0], p[1]));
                    }
                    let tex = fill.bitmap.map_or(TexRef::None, |b| if self.swap != 0 { TexRef::Swap(self.swap) } else { TexRef::Bitmap(b) });
                    self.out.tris(tex, fill.kind, &buf);
                }
                ShapeCmd::FanInt(pts) => {
                    buf.clear();
                    for i in 1..pts.len().saturating_sub(1) {
                        for p in [pts[0], pts[i], pts[i + 1]] {
                            buf.push(self.fill_vertex(fill, world, cx, p[0] as f32, p[1] as f32));
                        }
                    }
                    let tex = fill.bitmap.map_or(TexRef::None, |b| if self.swap != 0 { TexRef::Swap(self.swap) } else { TexRef::Bitmap(b) });
                    self.out.tris(tex, fill.kind, &buf);
                }
                ShapeCmd::LineStrip(pts) => {
                    let color = apply_cx(line_c, cx, self.alpha);
                    for w in pts.windows(2) {
                        self.thick_line(world, color, w[0][0], w[0][1], w[1][0], w[1][1], line_w);
                    }
                }
                ShapeCmd::Segments(segs) => {
                    let color = apply_cx(line_c, cx, self.alpha);
                    for s in segs {
                        self.thick_line(world, color, s[0], s[1], s[2], s[3], line_w);
                    }
                }
                _ => {}
            }
        }
    }

    fn fill_vertex(&self, fill: &FillStyle, world: &Matrix, cx: &CxForm, x: f32, y: f32) -> Vertex {
        let (px, py) = (x * TWIP, y * TWIP);
        let color = apply_cx(if fill.gradient.is_some() { gradient_color(fill, x, y) } else { fill.color }, cx, self.alpha);
        let (u, v) = fill.matrix.apply(px, py);
        let (sx, sy) = world.apply(px, py);
        Vertex { pos: [sx, sy], uv: [u, v], color }
    }

    /// The original's axis-aligned hexagon for a thick segment (a 6-vertex fan).
    #[allow(clippy::too_many_arguments)]
    fn thick_line(&mut self, world: &Matrix, color: [u8; 4], x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
        let a = w * 0.5;
        let b = w - a;
        let fan: [(f32, f32); 6] = if (x1 - x0).abs() > (y1 - y0).abs() {
            [
                (x0, y0 - a),
                (x1, y1 - a),
                (x1 + if x1 > x0 { b } else { -b }, y1),
                (x1, y1 + b),
                (x0, y0 + b),
                (x0 + if x0 < x1 { -a } else { a }, y0),
            ]
        } else {
            [
                (x0 - a, y0),
                (x0, y0 + if y0 < y1 { -a } else { a }),
                (x0 + b, y0),
                (x1 + b, y1),
                (x1, y1 + if y1 > y0 { b } else { -b }),
                (x1 - a, y1),
            ]
        };
        let v = |p: (f32, f32)| {
            let (sx, sy) = world.apply(p.0 * TWIP, p.1 * TWIP);
            Vertex { pos: [sx, sy], uv: [0.0, 0.0], color }
        };
        let mut buf = Vec::with_capacity(12);
        for i in 1..5 {
            buf.extend([v(fan[0]), v(fan[i]), v(fan[i + 1])]);
        }
        self.out.tris(TexRef::None, 0, &buf);
    }

    fn glyph_quad(&mut self, font_id: u16, height: i16, glyph: usize, ox: f32, oy: f32, m: &Matrix, color: [u8; 4]) {
        let Some(Character::Font(font)) = self.movie.char(font_id) else { return };
        let Some(pi) = glyph_page(&font.pages, height) else { return };
        let page = &font.pages[pi];
        let Some(g) = page.glyphs.get(glyph) else { return };
        let scale = height as f32 * (1.0 / 20480.0);
        let k = 1.0 / 256.0;
        let (u0, v0, u1, v1) = (g.texel[0] * k, g.texel[1] * k, (g.texel[0] + g.texel[2]) * k, (g.texel[1] + g.texel[3]) * k);
        let p = |x: f32, y: f32| m.apply((x + ox) * scale, (y + oy) * scale);
        let (l, t, r, b) = (g.quad[0], g.quad[1], g.quad[2], g.quad[3]);
        let mk = |xy: (f32, f32), u: f32, v: f32| Vertex { pos: [xy.0, xy.1], uv: [u, v], color };
        let (tl, tr, bl, br) = (mk(p(l, t), u0, v0), mk(p(r, t), u1, v0), mk(p(l, b), u0, v1), mk(p(r, b), u1, v1));
        self.out.tris(TexRef::Glyph { font: font_id, page: pi as u8, sheet: g.sheet }, 0x41, &[br, bl, tr, bl, tr, tl]);
    }

    /// A key's name in place of a clip (`Player::key_caps`): in the host's colour, in the movie's
    /// "Dev Gothic" (its largest glyph page), the capitals 0.45 of the clip's height,
    /// centred where the clip draws.
    fn key_cap(&mut self, p: &Player, id: InstId, text: &str, argb: u32, parent: &Matrix, cx: &CxForm) {
        let mut b = Builder::new(self.movie);
        b.no_caps = true;
        b.instance(p, id, &Matrix::IDENTITY, &CxForm::IDENTITY, [255; 4]);
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for it in &b.out.items {
            if let Item::Triangles { verts, .. } = it {
                for v in verts {
                    r = [r[0].min(v.pos[0]), r[1].min(v.pos[1]), r[2].max(v.pos[0]), r[3].max(v.pos[1])];
                }
            }
        }
        if r[0] > r[2] {
            return;
        }
        let font = (0..self.movie.chars.len() as u16).find_map(|i| match self.movie.char(i) {
            Some(Character::Font(f)) if f.name.to_ascii_lowercase().contains("dev gothic") && !f.pages.is_empty() => Some((i, f)),
            _ => None,
        });
        let Some((fid, font)) = font else { return };
        let Some(h) = font.pages.iter().map(|pg| pg.height).max() else { return };
        let adv = |c: char| font.glyph_index(c).and_then(|g| font.advances.get(g).copied()).unwrap_or(0.0);
        let width: f32 = text.chars().map(adv).sum();
        // (em units -> pixels: the glyph quads scale by h / 20480; the capitals stand
        // about 0.72 of the ascent)
        let cap = 0.72 * font.ascent as f32;
        let k = (r[3] - r[1]) * 0.45 / cap / (h as f32 / 20480.0);
        let m = parent.concat(&Matrix { a: k, b: 0.0, c: 0.0, d: k, tx: (r[0] + r[2]) * 0.5, ty: (r[1] + r[3]) * 0.5 });
        let color = apply_cx(argb, cx, self.alpha);
        let mut x = -width * 0.5;
        for ch in text.chars() {
            if let Some(g) = font.glyph_index(ch) {
                self.glyph_quad(fid, h, g, x, cap * 0.5, &m, color);
            }
            x += adv(ch);
        }
    }

    pub fn text(&mut self, t: &Text, world: &Matrix, cx: &CxForm) {
        let m = world.concat(&t.matrix);
        let mut font = 0u16;
        let mut height = 0i16;
        let mut color = apply_cx(0xffff_ffff, cx, self.alpha);
        let (mut x, mut y) = (0.0f32, 0.0f32);
        // offsets and advances are twips, converted to em units with the
        // original's integer division
        for r in &t.records {
            match r {
                TextRecord::Font(f, h) => {
                    font = *f;
                    height = *h;
                }
                TextRecord::Color(c) => color = apply_cx(*c, cx, self.alpha),
                TextRecord::Y(v) => y = (((*v as i32) << 10) / (height.max(1) as i32)) as f32,
                TextRecord::X(v) => x = (((*v as i32) << 10) / (height.max(1) as i32)) as f32,
                TextRecord::Glyphs(gs) => {
                    for &(g, adv) in gs {
                        self.glyph_quad(font, height, g as usize, x, y, &m, color);
                        x += (((adv as i32) << 10) / (height.max(1) as i32)) as f32;
                    }
                }
            }
        }
    }

    /// Dynamic text field (`F-88391`, layout `F-50496`, line measure
    /// `F-53874`): em-unit layout from the field origin, first baseline at
    /// the font ascent, lines `ascent + descent + leading` apart, broken at
    /// carriage returns and wrapped at spaces beyond the field width, aligned
    /// left/right/centre/justified. The colour is the instance tint.
    pub fn edit_text(&mut self, id: u16, text: &str, world: &Matrix, cx: &CxForm, tint: [u8; 4]) {
        let Some(Character::EditText(et)) = self.movie.char(id) else { return };
        let Some(Character::Font(font)) = self.movie.char(et.font) else { return };
        let h = et.height as i16;
        if font.page(h).is_none() {
            return;
        }
        let argb = 0xff00_0000 | (tint[0] as u32) << 16 | (tint[1] as u32) << 8 | tint[2] as u32;
        let color = apply_cx(argb, cx, self.alpha);
        let adv = |c: char| font.glyph_index(c).and_then(|g| font.advances.get(g).copied()).unwrap_or(0.0);
        let width = et.rect[0];
        let line_adv = (font.ascent as i32 + font.descent as i32 + et.leading as i32) as f32;
        let mut y = font.ascent as f32;
        for para in text.split(['\r', '\n']) {
            for line in wrap(para, width, &adv) {
                let line = line.trim_start_matches(' ');
                let lw: f32 = line.chars().map(adv).sum();
                let n = line.chars().count();
                let (mut x, extra) = match et.align {
                    1 => (width - lw, 0.0),
                    2 => ((width - lw) * 0.5, 0.0),
                    3 if n > 1 => (0.0, (width - lw) / (n - 1) as f32),
                    _ => (0.0, 0.0),
                };
                for ch in line.chars() {
                    if let Some(g) = font.glyph_index(ch) {
                        self.glyph_quad(et.font, h, g, x, y, world, color);
                    }
                    x += adv(ch) + extra;
                }
                y += line_adv;
            }
        }
    }
}

/// Greedy word wrap in em units; a width of 0 or less never wraps.
/// The first glyph page of a font made for this height (`F-23783`: the page list in order, equal heights).
pub(crate) fn glyph_page(pages: &[mcla_formats::rage::swf::FontPage], height: i16) -> Option<usize> {
    pages.iter().position(|p| p.height == height)
}

fn wrap<'t>(s: &'t str, width: f32, adv: &dyn Fn(char) -> f32) -> Vec<&'t str> {
    if width <= 0.0 {
        return vec![s];
    }
    let mut out = Vec::new();
    let mut start = 0;
    let mut w = 0.0;
    let mut last_space: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if c == ' ' {
            last_space = Some(i);
        }
        w += adv(c);
        if w > width && i > start {
            let cut = match last_space {
                Some(sp) if sp > start => sp,
                _ => i,
            };
            out.push(&s[start..cut]);
            start = cut;
            w = s[start..i + c.len_utf8()].chars().map(adv).sum();
            last_space = None;
        }
    }
    out.push(&s[start..]);
    out
}

/// Draw a whole player from its root at the stage origin.
pub fn build(p: &Player) -> DrawList {
    let mut b = Builder::new(&p.movie);
    b.instance(p, p.root, &Matrix::IDENTITY, &CxForm::IDENTITY, [255; 4]);
    b.out
}

/// Draw one instance subtree as if it were placed at the stage origin.
pub fn build_instance(p: &Player, id: InstId) -> DrawList {
    let mut b = Builder::new(&p.movie);
    b.instance(p, id, &Matrix::IDENTITY, &CxForm::IDENTITY, [255; 4]);
    b.out
}

/// Bounds of an instance's drawing in its parent's coordinates (pixels):
/// xmin, ymin, xmax, ymax.
pub fn bounds(p: &Player, id: InstId) -> Option<[f32; 4]> {
    let list = build_instance(p, id);
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for it in &list.items {
        if let Item::Triangles { verts, .. } = it {
            for v in verts {
                r[0] = r[0].min(v.pos[0]);
                r[1] = r[1].min(v.pos[1]);
                r[2] = r[2].max(v.pos[0]);
                r[3] = r[3].max(v.pos[1]);
            }
        }
    }
    (r[0] <= r[2]).then_some(r)
}
