//! `rage::swfFILE`: the game's compiled Flash movies (`resources/ui/**/*.r27`,
//! resource type 27, the original `hud/hud.swf`, `pause/pause.swf` ...).
//!
//! Layouts are read from the executable's fixup and draw routines (generated
//! recomp, see `UI_WORK.md`): `swfFILE` fixup `F-66443`, object dispatch
//! `F-55359` (type byte at +8), shape draw `F-19892`, text draw
//! `F-11515`, glyph quad `F-83474`, gradient `F-54894`, frame command
//! fixup `F-43457`, PlaceObject2 execute `F-70772`.
//!
//! Shapes are stored pre-tessellated: a command stream of fill/line style
//! selections and triangle lists in twips. Fonts are glyph sheets (textures)
//! with per-glyph texel rectangles and em-square (1024) quads. ActionScript is
//! ordinary AVM1 byte code (see [`crate::avm1`]).

use super::texture::Texture;
use super::Rd;
use crate::rsc::Resource;
use crate::{bail, Result};
use hashbrown::HashMap;

/// 2x3 affine matrix in Flash order: x' = a x + c y + tx, y' = b x + d y + ty.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    fn read(rd: &Rd, o: usize) -> Matrix {
        Matrix { a: rd.f32(o), b: rd.f32(o + 4), c: rd.f32(o + 8), d: rd.f32(o + 12), tx: rd.f32(o + 16), ty: rd.f32(o + 20) }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    /// `self * child`: maps child space through `child` then `self`.
    pub fn concat(&self, m: &Matrix) -> Matrix {
        Matrix {
            a: self.a * m.a + self.c * m.b,
            b: self.b * m.a + self.d * m.b,
            c: self.a * m.c + self.c * m.d,
            d: self.b * m.c + self.d * m.d,
            tx: self.a * m.tx + self.c * m.ty + self.tx,
            ty: self.b * m.tx + self.d * m.ty + self.ty,
        }
    }
}

/// Colour transform as the runtime keeps it: signed multipliers in 1/64 units
/// (64 = 1.0) and signed additive terms in units of 4, order R G B A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CxForm {
    pub mult: [i8; 4],
    pub add: [i8; 4],
}

impl CxForm {
    pub const IDENTITY: CxForm = CxForm { mult: [64; 4], add: [0; 4] };
}

#[derive(Debug, Clone)]
pub struct Gradient {
    /// gradient-square matrix (twips -> gradient space, the square is -16384..16384)
    pub matrix: Matrix,
    pub radial: bool,
    pub ratios: Vec<u8>,
    /// ARGB
    pub colors: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct FillStyle {
    /// SWF fill type: 0x00 solid, 0x10 linear, 0x12 radial, 0x40..0x43 bitmap
    pub kind: u8,
    /// ARGB
    pub color: u32,
    /// pixel coordinates -> texture coordinates (bitmap fills)
    pub matrix: Matrix,
    pub bitmap: Option<u16>,
    pub gradient: Option<Gradient>,
}

#[derive(Debug, Clone, Copy)]
pub struct LineStyle {
    /// twips
    pub width: u16,
    /// ARGB
    pub color: u32,
}

#[derive(Debug, Clone)]
pub enum ShapeCmd {
    /// opcode 1: connected line points (twips)
    LineStrip(Vec<[f32; 2]>),
    /// opcode 2: triangle fan with integer twips
    FanInt(Vec<[i32; 2]>),
    /// opcode 3: two words, skipped by the renderer
    Skip(u32, u32),
    /// opcode 4: select fill style (1-based; 0 keeps the current one)
    Fill(u32),
    /// opcode 5: select line style (1-based; 0 keeps the current one)
    Line(u32),
    /// opcode 6: independent line segments x0,y0,x1,y1 (twips)
    Segments(Vec<[f32; 4]>),
    /// opcode 7: triangle list (twips) with the current fill
    Triangles(Vec<[f32; 2]>),
}

#[derive(Debug, Clone)]
pub struct Shape {
    /// twips: xmin, ymin, xmax, ymax
    pub bounds: [i32; 4],
    pub fills: Vec<FillStyle>,
    pub lines: Vec<LineStyle>,
    pub cmds: Vec<ShapeCmd>,
}

#[derive(Debug, Clone)]
pub struct Bitmap {
    /// texture embedded in this movie
    pub texture: Option<Texture>,
    /// texture of a shared dictionary, looked up by name (`shared_34.dds`)
    pub external: Option<String>,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Glyph {
    /// texel rectangle in its sheet: x, y, w, h
    pub texel: [f32; 4],
    /// quad in em units (1024): left, top, right, bottom
    pub quad: [f32; 4],
    pub sheet: u8,
}

/// A glyph sheet: embedded, or a texture of the shared UI dictionary by name
/// (page +16 names, resolved at load by `F-29581`).
#[derive(Debug, Clone)]
pub enum Sheet {
    Embedded(Texture),
    Shared(String),
    Missing,
}

#[derive(Debug, Clone)]
pub struct FontPage {
    /// the text height (twips) this page was rasterised for
    pub height: i16,
    /// indexed by `Glyph::sheet`
    pub sheets: Vec<Sheet>,
    pub glyphs: Vec<Glyph>,
}

#[derive(Debug, Clone)]
pub struct Font {
    pub name: String,
    /// glyph index of ASCII codes 0..127
    pub ascii: [u8; 128],
    /// character code of each glyph
    pub codes: Vec<u16>,
    /// advance of each glyph in em units (1024)
    pub advances: Vec<f32>,
    pub ascent: i16,
    pub descent: i16,
    pub leading: i16,
    pub pages: Vec<FontPage>,
}

impl Font {
    pub fn glyph_index(&self, ch: char) -> Option<usize> {
        let c = ch as u32;
        if c < 128 && (c >= 32 || self.ascii[c as usize] != 0) {
            let g = self.ascii[c as usize] as usize;
            if self.codes.get(g).map_or(false, |&k| k as u32 == c) {
                return Some(g);
            }
        }
        self.codes.iter().position(|&k| k as u32 == c)
    }

    pub fn page(&self, height: i16) -> Option<&FontPage> {
        self.pages.iter().find(|p| p.height == height)
    }
}

#[derive(Debug, Clone)]
pub enum TextRecord {
    /// font id, height (twips)
    Font(u16, i16),
    /// ARGB
    Color(u32),
    /// em units relative to the height
    Y(i16),
    X(i16),
    /// glyph index, advance (twips)
    Glyphs(Vec<(u16, i16)>),
}

#[derive(Debug, Clone)]
pub struct Text {
    pub matrix: Matrix,
    pub records: Vec<TextRecord>,
}

/// Dynamic text field. Drawn by `F-88391` / layout `F-50496`: the text is
/// the instance's own text, else the variable's value, else `initial`; the
/// colour is the instance's tint (not `color`); lines wrap at `width`.
#[derive(Debug, Clone)]
pub struct EditText {
    pub initial: String,
    pub variable: String,
    pub max_len: u16,
    /// extra line spacing, em units (added to ascent + descent)
    pub leading: i16,
    /// ARGB as authored (unused by the original renderer)
    pub color: u32,
    pub font: u16,
    /// twips
    pub height: u16,
    /// +32 field width in em units (1024 per text height), then three raw words
    pub rect: [f32; 4],
    /// +48: non-zero selects the other draw path (`F-28519`)
    pub html: u8,
    /// +49: 0 left, 1 right, 2 centre, 3 justify
    pub align: u8,
}

#[derive(Debug, Clone)]
pub struct Sprite {
    pub frames: Vec<Frame>,
}

/// Button: its state records (`+12`, count byte `+21`; the 8-byte condition
/// action records at `+16` / `+22` are not read, see `docs/UI.md`).
#[derive(Debug, Clone)]
pub struct Button {
    pub records: Vec<ButtonRecord>,
}

/// One 40-byte button record as the button draw F-28198 reads it.
#[derive(Debug, Clone)]
pub struct ButtonRecord {
    /// +0 state mask: 1 up, 2 over, 4 down, 8 hit
    pub states: u8,
    /// +4 six floats, final units like an instance matrix
    pub matrix: Matrix,
    /// +28 four multipliers, +32 four additions (signed bytes, 64 = 1)
    pub cxform: CxForm,
    /// +36 the drawn character (pointer to its object)
    pub character: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct Frame {
    /// first word of the frame record (root: the stage colour 0xff666666)
    pub tag: u32,
    pub cmds: Vec<Cmd>,
}

/// AVM1 code block, shared by the runtime's function objects.
pub type Code = std::sync::Arc<[u8]>;

#[derive(Debug, Clone)]
pub struct ClipEvent {
    /// event mask: bit 0 placed (load), bit 1 frame (enterFrame)
    pub flags: u32,
    pub key: u32,
    pub actions: Code,
}

#[derive(Debug, Clone)]
pub struct Place {
    pub depth: u16,
    pub clip_depth: u16,
    /// None = modify the existing object at `depth`
    pub character: Option<u16>,
    pub matrix: Option<Matrix>,
    pub cxform: Option<CxForm>,
    pub ratio: Option<u16>,
    pub name: Option<String>,
    pub events: Vec<ClipEvent>,
}

#[derive(Debug, Clone)]
pub enum Cmd {
    Place(Place),
    Remove(u16),
    /// AVM1 byte code (without the 2-byte record header)
    DoAction(Code),
    DoInitAction(Code),
}

#[derive(Debug, Clone)]
pub enum Character {
    Shape(Shape),
    Sprite(Sprite),
    Button(Button),
    Bitmap(Bitmap),
    Font(Font),
    Text(Text),
    EditText(EditText),
    Morph,
}

impl Character {
    pub fn kind(&self) -> &'static str {
        match self {
            Character::Shape(_) => "shape",
            Character::Sprite(_) => "sprite",
            Character::Button(_) => "button",
            Character::Bitmap(_) => "bitmap",
            Character::Font(_) => "font",
            Character::Text(_) => "text",
            Character::EditText(_) => "edittext",
            Character::Morph => "morph",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Movie {
    /// stage rectangle in pixels: xmin, xmax, ymin, ymax
    pub stage: [f32; 4],
    pub fps: u8,
    pub root: Sprite,
    /// index = character id
    pub chars: Vec<Option<Character>>,
    /// linkage names for attachMovie
    pub exports: Vec<(String, u16)>,
}

impl Movie {
    pub fn char(&self, id: u16) -> Option<&Character> {
        self.chars.get(id as usize).and_then(|c| c.as_ref())
    }

    pub fn export(&self, name: &str) -> Option<u16> {
        self.exports.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|e| e.1)
    }

    pub fn sprite(&self, id: u16) -> Option<&Sprite> {
        if id == 0 {
            return Some(&self.root);
        }
        match self.char(id) {
            Some(Character::Sprite(s)) => Some(s),
            _ => None,
        }
    }

    pub fn parse(res: &Resource) -> Result<Movie> {
        let rd = Rd::new(res);
        if rd.u32(0) != 0x4449_5500 {
            bail!("not a swf movie (magic {:#x})", rd.u32(0));
        }
        let dict = rd.ptr(24).ok_or_else(|| crate::anyhow!("swf without dictionary"))?;
        let count = rd.u16(50) as usize;
        let mut offsets: HashMap<usize, u16> = HashMap::new();
        let mut objs = Vec::with_capacity(count);
        for i in 0..count {
            let o = rd.ptr(dict + i * 4);
            if let Some(o) = o {
                offsets.insert(o, i as u16);
            }
            objs.push(o);
        }
        let p = Parser { rd, offsets };
        let mut chars = Vec::with_capacity(count);
        for o in &objs {
            chars.push(match o {
                Some(o) if *o != 0 => p.character(*o)?,
                _ => None,
            });
        }
        let root = p.sprite(0)?;
        let mut exports = Vec::new();
        if let Some(buckets) = rd.ptr(56) {
            for b in 0..rd.u16(60) as usize {
                let mut e = rd.ptr(buckets + b * 4);
                let mut guard = 0;
                while let Some(eo) = e {
                    if let Some(k) = rd.ptr(eo) {
                        exports.push((rd.cstr(k), rd.u16(eo + 4)));
                    }
                    e = rd.ptr(eo + 8);
                    guard += 1;
                    if guard > 10_000 {
                        bail!("export chain loops");
                    }
                }
            }
        }
        exports.sort();
        Ok(Movie { stage: [rd.f32(32), rd.f32(36), rd.f32(40), rd.f32(44)], fps: rd.u8(48), root, chars, exports })
    }
}

struct Parser<'a> {
    rd: Rd<'a>,
    offsets: HashMap<usize, u16>,
}

impl<'a> Parser<'a> {
    fn character(&self, o: usize) -> Result<Option<Character>> {
        let rd = &self.rd;
        Ok(Some(match rd.u8(o + 8) {
            1 => Character::Shape(self.shape(o)?),
            2 => Character::Sprite(self.sprite(o)?),
            3 => Character::Button(self.button(o)),
            4 => Character::Bitmap(self.bitmap(o)),
            5 => Character::Font(self.font(o)?),
            6 => Character::Text(self.text(o)?),
            7 => Character::EditText(self.edit_text(o)),
            9 => Character::Morph,
            t => {
                log::warn!("swf: object at {:#x} has unknown type {}", o, t);
                return Ok(None);
            }
        }))
    }

    fn texture(&self, t: usize) -> Option<Texture> {
        let rd = &self.rd;
        let name = rd.cstr_ptr(t + 0x18).unwrap_or_default();
        let d3d = rd.ptr(t + 0x1c)?;
        let mut fetch = [0u32; 6];
        for (i, f) in fetch.iter_mut().enumerate() {
            *f = rd.u32(d3d + 0x1c + i * 4);
        }
        let short = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_ascii_lowercase();
        match Texture::from_fetch(rd, short, fetch) {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("swf: texture {}: {}", name, e);
                None
            }
        }
    }

    fn bitmap(&self, o: usize) -> Bitmap {
        let rd = &self.rd;
        Bitmap {
            texture: rd.ptr(o + 12).and_then(|t| self.texture(t)),
            external: rd.cstr_ptr(o + 16).map(|s| s.to_ascii_lowercase()),
            width: rd.u16(o + 20),
            height: rd.u16(o + 22),
        }
    }

    fn shape(&self, o: usize) -> Result<Shape> {
        let rd = &self.rd;
        let bounds = [rd.u32(o + 32) as i32, rd.u32(o + 36) as i32, rd.u32(o + 40) as i32, rd.u32(o + 44) as i32];
        let mut fills = Vec::new();
        if let Some(fp) = rd.ptr(o + 12) {
            for i in 0..rd.u16(o + 28) as usize {
                let f = fp + i * 40;
                let gradient = rd.ptr(f + 36).map(|g| {
                    let n = rd.u8(g + 32) as usize;
                    let rp = rd.ptr(g + 24);
                    let cp = rd.ptr(g + 28);
                    Gradient {
                        matrix: Matrix::read(rd, g),
                        radial: rd.u8(g + 33) != 0,
                        ratios: rp.map(|p| (0..n).map(|k| rd.u8(p + k)).collect()).unwrap_or_default(),
                        colors: cp.map(|p| (0..n).map(|k| rd.u32(p + k * 4)).collect()).unwrap_or_default(),
                    }
                });
                fills.push(FillStyle {
                    kind: rd.u8(f),
                    color: rd.u32(f + 4),
                    matrix: Matrix::read(rd, f + 8),
                    bitmap: rd.ptr(f + 32).and_then(|b| self.offsets.get(&b).copied()),
                    gradient,
                });
            }
        }
        let mut lines = Vec::new();
        if let Some(lp) = rd.ptr(o + 16) {
            for i in 0..rd.u16(o + 30) as usize {
                lines.push(LineStyle { width: rd.u16(lp + i * 8), color: rd.u32(lp + i * 8 + 4) });
            }
        }
        let mut cmds = Vec::new();
        if let Some(mut p) = rd.ptr(o + 20) {
            let pts = |p: usize, n: usize| -> Vec<[f32; 2]> { (0..n).map(|k| [rd.f32(p + k * 8), rd.f32(p + k * 8 + 4)]).collect() };
            loop {
                if !rd.has(p, 4) {
                    bail!("shape {:#x}: command stream runs off the resource", o);
                }
                let op = rd.u32(p);
                p += 4;
                match op {
                    0 => break,
                    1 | 7 => {
                        let n = rd.u32(p) as usize;
                        let v = pts(p + 4, n);
                        p += 4 + n * 8;
                        cmds.push(if op == 1 { ShapeCmd::LineStrip(v) } else { ShapeCmd::Triangles(v) });
                    }
                    2 => {
                        let n = rd.u32(p) as usize;
                        let v = (0..n).map(|k| [rd.u32(p + 4 + k * 8) as i32, rd.u32(p + 8 + k * 8) as i32]).collect();
                        p += 4 + n * 8;
                        cmds.push(ShapeCmd::FanInt(v));
                    }
                    3 => {
                        cmds.push(ShapeCmd::Skip(rd.u32(p), rd.u32(p + 4)));
                        p += 8;
                    }
                    4 => {
                        cmds.push(ShapeCmd::Fill(rd.u32(p)));
                        p += 4;
                    }
                    5 => {
                        cmds.push(ShapeCmd::Line(rd.u32(p)));
                        p += 4;
                    }
                    6 => {
                        let n = rd.u32(p) as usize;
                        let v = (0..n).map(|k| [rd.f32(p + 4 + k * 16), rd.f32(p + 8 + k * 16), rd.f32(p + 12 + k * 16), rd.f32(p + 16 + k * 16)]).collect();
                        p += 4 + n * 16;
                        cmds.push(ShapeCmd::Segments(v));
                    }
                    // the original loops on unknown opcodes > 7; treat as corrupt
                    _ => bail!("shape {:#x}: unknown command {}", o, op),
                }
            }
        }
        Ok(Shape { bounds, fills, lines, cmds })
    }

    fn sprite(&self, o: usize) -> Result<Sprite> {
        let rd = &self.rd;
        let n = rd.u16(o + 16) as usize;
        let mut frames = Vec::with_capacity(n);
        if let Some(fp) = rd.ptr(o + 12) {
            for i in 0..n {
                let f = fp + i * 8;
                let mut cmds = Vec::new();
                let mut c = rd.ptr(f + 4);
                let mut guard = 0;
                while let Some(co) = c {
                    if let Some(cmd) = self.cmd(co)? {
                        cmds.push(cmd);
                    }
                    c = rd.ptr(co + 12);
                    guard += 1;
                    if guard > 100_000 {
                        bail!("frame command list loops");
                    }
                }
                frames.push(Frame { tag: rd.u32(f), cmds });
            }
        }
        Ok(Sprite { frames })
    }

    fn actions(&self, p: usize) -> Code {
        let rd = &self.rd;
        let p = p.min(rd.data().len());
        let len = crate::avm1::block_len(&rd.data()[p..]);
        Code::from(&rd.data()[p..p + len])
    }

    fn cmd(&self, c: usize) -> Result<Option<Cmd>> {
        let rd = &self.rd;
        Ok(Some(match rd.u8(c + 4) {
            t @ (0 | 1) => {
                let flags = rd.u8(c + 5);
                let matrix = rd.ptr(c + 16).map(|m| {
                    let s = |k: usize| rd.u16(m + k * 2) as i16 as f32;
                    let mut k = 0;
                    let mut mx = Matrix::IDENTITY;
                    if flags & 1 != 0 {
                        mx.a = s(0) * (1.0 / 256.0);
                        mx.d = s(1) * (1.0 / 256.0);
                        k = 2;
                    }
                    if flags & 2 != 0 {
                        mx.b = s(k) * (1.0 / 256.0);
                        mx.c = s(k + 1) * (1.0 / 256.0);
                        k += 2;
                    }
                    mx.tx = s(k) * 0.05;
                    mx.ty = s(k + 1) * 0.05;
                    mx
                });
                let cxform = rd.ptr(c + 28).map(|x| {
                    let b = |k: usize| rd.u8(x + k) as i8;
                    CxForm { mult: [b(0), b(1), b(2), b(3)], add: [b(4), b(5), b(6), b(7)] }
                });
                let ch = rd.u16(c + 24);
                let ratio = rd.u16(c + 32);
                let mut place = Place {
                    depth: rd.u16(c + 20),
                    clip_depth: rd.u16(c + 22),
                    character: (ch != 0xffff).then_some(ch),
                    matrix,
                    cxform,
                    ratio: (ratio != 0xcdcd).then_some(ratio),
                    name: None,
                    events: Vec::new(),
                };
                if t == 1 {
                    place.name = rd.cstr_ptr(c + 60);
                    if let Some(ev) = rd.ptr(c + 52) {
                        for k in 0..rd.u32(c + 56) as usize {
                            let e = ev + k * 12;
                            place.events.push(ClipEvent {
                                flags: rd.u32(e),
                                key: rd.u32(e + 8),
                                // the block starts with the same 2-byte header as DoAction
                                actions: rd.ptr(e + 4).map(|a| self.actions(a + 2)).unwrap_or_else(|| Code::from(&[0u8][..])),
                            });
                        }
                    }
                }
                Cmd::Place(place)
            }
            2 => Cmd::Remove(rd.u16(c + 6)),
            t @ (3 | 4) => {
                let Some(a) = rd.ptr(c + 16) else { return Ok(None) };
                let code = self.actions(a + 2);
                if t == 3 {
                    Cmd::DoAction(code)
                } else {
                    Cmd::DoInitAction(code)
                }
            }
            t => bail!("frame command at {:#x}: unknown type {}", c, t),
        }))
    }

    fn font(&self, o: usize) -> Result<Font> {
        let rd = &self.rd;
        let nglyph = rd.u16(o + 0xa0) as usize;
        let mut ascii = [0u8; 128];
        for (i, a) in ascii.iter_mut().enumerate() {
            *a = rd.u8(o + 0x18 + i);
        }
        let codes = rd.ptr(o + 16).map(|p| (0..nglyph).map(|k| rd.u16(p + k * 2)).collect()).unwrap_or_default();
        let advances = rd.ptr(o + 20).map(|p| (0..nglyph).map(|k| rd.f32(p + k * 4)).collect()).unwrap_or_default();
        let mut pages = Vec::new();
        for i in 0..(rd.u16(o + 152) as i16).max(0) as usize {
            let Some(pg) = rd.ptr(o + 164 + i * 4) else { continue };
            let n = rd.u16(pg + 10) as usize;
            let nsheet = rd.u32(pg + 24) as usize;
            let names = rd.ptr(pg + 16);
            let sheets = (0..nsheet)
                .map(|k| match rd.ptr(pg).and_then(|a| rd.ptr(a + k * 4)).and_then(|t| self.texture(t)) {
                    Some(t) => Sheet::Embedded(t),
                    None => match names.and_then(|n| rd.cstr_ptr(n + k * 4)) {
                        Some(s) if !s.is_empty() => Sheet::Shared(s.to_ascii_lowercase()),
                        _ => Sheet::Missing,
                    },
                })
                .collect();
            let idx = rd.ptr(pg + 12);
            let glyphs = rd
                .ptr(pg + 4)
                .map(|g| {
                    (0..n)
                        .map(|k| {
                            let r = g + k * 32;
                            Glyph {
                                texel: [rd.f32(r), rd.f32(r + 4), rd.f32(r + 8), rd.f32(r + 12)],
                                quad: [rd.f32(r + 16), rd.f32(r + 20), rd.f32(r + 24), rd.f32(r + 28)],
                                sheet: idx.map_or(0, |p| rd.u8(p + k)),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            pages.push(FontPage { height: rd.u16(pg + 8) as i16, sheets, glyphs });
        }
        Ok(Font {
            name: rd.cstr(o + 0xc4),
            ascii,
            codes,
            advances,
            ascent: rd.u16(o + 0x9a) as i16,
            descent: rd.u16(o + 0x9c) as i16,
            leading: rd.u16(o + 0x9e) as i16,
            pages,
        })
    }

    fn text(&self, o: usize) -> Result<Text> {
        let rd = &self.rd;
        let mut records = Vec::new();
        if let Some(mut p) = rd.ptr(o + 36) {
            let w = |p: usize| rd.u16(p) as i16;
            loop {
                if !rd.has(p, 2) {
                    bail!("text {:#x}: record stream runs off the resource", o);
                }
                let op = w(p);
                p += 2;
                match op {
                    0 => break,
                    1 => {
                        records.push(TextRecord::Font(w(p) as u16, w(p + 2)));
                        p += 4;
                    }
                    2 => {
                        records.push(TextRecord::Color((rd.u16(p + 2) as u32) << 16 | rd.u16(p) as u32));
                        p += 4;
                    }
                    3 => {
                        records.push(TextRecord::Y(w(p)));
                        p += 2;
                    }
                    4 => {
                        records.push(TextRecord::X(w(p)));
                        p += 2;
                    }
                    5 => {
                        let n = w(p).max(0) as usize;
                        let g = (0..n).map(|k| (rd.u16(p + 2 + k * 4), w(p + 4 + k * 4))).collect();
                        p += 2 + n * 4;
                        records.push(TextRecord::Glyphs(g));
                    }
                    _ => bail!("text {:#x}: unknown record {}", o, op),
                }
            }
        }
        Ok(Text { matrix: Matrix::read(rd, o + 12), records })
    }

    fn button(&self, o: usize) -> Button {
        let rd = &self.rd;
        let records = match rd.ptr(o + 12) {
            Some(a) => (0..rd.u8(o + 21) as usize)
                .map(|i| {
                    let r = a + 40 * i;
                    let b = |k: usize| rd.u8(r + 28 + k) as i8;
                    ButtonRecord {
                        states: rd.u8(r),
                        matrix: Matrix::read(rd, r + 4),
                        cxform: CxForm { mult: [b(0), b(1), b(2), b(3)], add: [b(4), b(5), b(6), b(7)] },
                        character: rd.ptr(r + 36).and_then(|c| self.offsets.get(&c).copied()),
                    }
                })
                .collect(),
            None => Vec::new(),
        };
        Button { records }
    }

    fn edit_text(&self, o: usize) -> EditText {
        let rd = &self.rd;
        EditText {
            initial: rd.cstr_ptr(o + 12).unwrap_or_default(),
            variable: rd.cstr_ptr(o + 16).unwrap_or_default(),
            max_len: rd.u16(o + 20),
            leading: rd.u16(o + 22) as i16,
            color: rd.u32(o + 24),
            font: rd.u16(o + 28),
            height: rd.u16(o + 30),
            rect: [rd.f32(o + 32), rd.f32(o + 36), rd.f32(o + 40), rd.f32(o + 44)],
            html: rd.u8(o + 48),
            align: rd.u8(o + 49),
        }
    }
}
