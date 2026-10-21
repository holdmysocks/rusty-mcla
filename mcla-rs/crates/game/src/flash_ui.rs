//! The game's own Flash UI movies (`resources/ui/**/*.r27`) played with
//! `mcla-flash` and drawn through egui: textures uploaded on first use, the
//! 1280x720 stage fitted to the screen height and centred, masks approximated
//! by the clip rectangle of their geometry.
//!
//! [`FlashHud`] drives `hud/hud.swf` the way the original game does: it writes
//! the movie's `_global` variables (names and units read from the movie's
//! scripts and the executable's callers of the variable API, see `UI_WORK.md`).

use crate::game::HudInfo;
use egui::{Color32, Mesh, Pos2, Rect, TextureHandle};
use hashbrown::HashMap;
use mcla_flash::draw::{DrawList, Item, TexRef};
use mcla_flash::player::Player;
use mcla_flash::swf::{Character, Movie};
use mcla_flash::value::{Ref, Value};
use mcla_formats::rage::swf::Matrix;
use mcla_formats::rage::texture::Texture;
use mcla_formats::strtbl::StringTable;
use std::path::Path;
use std::sync::Arc;

pub struct FlashMovie {
    pub player: Player,
    textures: HashMap<(TexRef, bool), Option<TextureHandle>>,
    /// host textures by key (`TexRef::Swap`): the pictures the game streams into a movie
    swaps: HashMap<u32, Option<TextureHandle>>,
    /// file each key was loaded from
    swap_files: HashMap<u32, String>,
    shared: Arc<HashMap<String, Texture>>,
    name: String,
    /// placement of a stage smaller than 1280x720, in 720p pixels
    pub offset: [f32; 2],
    /// the game's string table (what the game itself writes into movies)
    pub strings: Arc<StringTable>,
}

impl FlashMovie {
    /// A string of the game's table, as the game hands it to a movie (`0x82218310`);
    /// the key itself when missing.
    pub fn string(&self, key: &str) -> String {
        self.strings.get(key).map(|s| s.to_string()).unwrap_or_else(|| key.to_string())
    }
}

/// Textures of `resources/ui/shared_latin.r9` by short lower-case name.
pub fn load_shared(data_dir: &str) -> HashMap<String, Texture> {
    use mcla_formats::rage::{Dictionary, Rd};
    let mut out = HashMap::new();
    if let Ok(res) = crate::assets::load_resource_file(&Path::new(data_dir).join("resources/ui/shared_latin.r9")) {
        let rd = Rd::new(&res);
        if let Ok(dict) = Dictionary::parse(&rd, 0) {
            for o in dict.values.iter().flatten() {
                if let Ok(t) = Texture::parse(&rd, *o) {
                    let short = t.name.rsplit(['/', '\\']).next().unwrap_or(&t.name).to_ascii_lowercase();
                    out.insert(short, t);
                }
            }
        }
    }
    out
}

/// What the movies share: the UI texture dictionary and the game's string table
/// (for `FSCommand:Getstring`).
#[derive(Clone)]
pub struct UiShared {
    pub textures: Arc<HashMap<String, Texture>>,
    pub strings: Arc<StringTable>,
    /// `tune/ui` `UILogic`: the pause cameras the menu movies move between
    pub logic: Arc<crate::menu3d::UiLogic>,
}

impl FlashMovie {
    pub fn load(data_dir: &str, rel: &str, shared: &UiShared) -> Option<FlashMovie> {
        let res = match crate::assets::load_resource_file(&Path::new(data_dir).join(rel)) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("flash: {}: {}", rel, e);
                return None;
            }
        };
        let movie = match Movie::parse(&res) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                log::warn!("flash: {}: {}", rel, e);
                return None;
            }
        };
        let mut player = Player::new(movie);
        let strings = shared.strings.clone();
        let logic = shared.logic.clone();
        player.fscommand = Some(Box::new(move |p: &mut Player, cmd: &str, arg: &str| fscommand(p, cmd, arg, &strings, &logic)));
        log::info!("flash: loaded {} ({} characters)", rel, player.movie.chars.len());
        Some(FlashMovie { player, textures: HashMap::new(), swaps: HashMap::new(), swap_files: HashMap::new(), shared: shared.textures.clone(), name: rel.to_string(), offset: [0.0, 0.0], strings: shared.strings.clone() })
    }

    /// Give the clip at `path` the first texture of the dictionary `file` in place of
    /// its own bitmaps (what the game does for a clip with `__texturename`). `key` names
    /// the slot; the file is read again only when it changes. `false`: no such clip or
    /// file (the clip then keeps its bitmap).
    pub fn swap_texture(&mut self, ctx: &egui::Context, path: &str, key: u32, file: &Path) -> bool {
        let Some(id) = self.player.find(path) else { return false };
        self.swap_texture_id(ctx, id, key, file)
    }

    /// The live clips of the name `clip`, in the display list's order, each with the first
    /// texture of its file (`swap_texture` for clips found by name: the rewards screen's
    /// car rows each hold a clip `reward_dyntexture`, which names its texture so - the
    /// pause movie's sprite 202, frame 6). Keys from `key0` up. Returns how many were set.
    pub fn swap_textures_named(&mut self, ctx: &egui::Context, clip: &str, key0: u32, files: &[Option<std::path::PathBuf>]) -> usize {
        let mut found = Vec::new();
        let mut stack = vec![self.player.root];
        while let Some(id) = stack.pop() {
            let Some(i) = self.player.inst(id) else { continue };
            if i.name.eq_ignore_ascii_case(clip) {
                found.push(id);
            }
            // (pushed in reverse: popped in the list's order)
            stack.extend(i.children.values().rev().copied());
        }
        let mut n = 0;
        for (k, id) in found.into_iter().enumerate() {
            if let Some(Some(f)) = files.get(k) {
                if self.swap_texture_id(ctx, id, key0 + k as u32, f) {
                    n += 1;
                }
            }
        }
        n
    }

    /// Give the clip at `path` a texture the game makes itself (a render target: the minimap's
    /// `__mini_map` in `HUD.renderMe`). `false`: no such clip.
    pub fn swap_handle(&mut self, path: &str, key: u32, handle: TextureHandle) -> bool {
        let Some(id) = self.player.find(path) else { return false };
        self.swaps.insert(key, Some(handle));
        self.swap_files.insert(key, String::new());
        if let Some(i) = self.player.inst_mut(id) {
            i.tex_swap = key;
        }
        true
    }

    /// `swap_texture` for a clip in hand.
    pub fn swap_texture_id(&mut self, ctx: &egui::Context, id: mcla_flash::value::InstId, key: u32, file: &Path) -> bool {
        use mcla_formats::rage::{Dictionary, Rd};
        let name = file.to_string_lossy().to_string();
        if self.swap_files.get(&key) != Some(&name) {
            let mut handle = None;
            if let Ok(res) = crate::assets::load_resource_file(file) {
                let rd = Rd::new(&res);
                if let Ok(dict) = Dictionary::parse(&rd, 0) {
                    if let Some(t) = dict.values.iter().flatten().find_map(|o| Texture::parse(&rd, *o).ok()) {
                        handle = Some(ctx.load_texture(format!("swf:{}:swap{}", self.name, key), plain_image(t.width, t.height, &t.to_rgba8()), egui::TextureOptions::LINEAR));
                    }
                }
            }
            self.swaps.insert(key, handle);
            self.swap_files.insert(key, name);
        }
        let have = self.swaps.get(&key).map_or(false, |h| h.is_some());
        if let Some(i) = self.player.inst_mut(id) {
            i.tex_swap = if have { key } else { 0 };
        }
        have
    }

    fn texture(&mut self, ctx: &egui::Context, t: TexRef, repeat: bool) -> Option<TextureHandle> {
        if let TexRef::Swap(k) = t {
            return self.swaps.get(&k).cloned().flatten();
        }
        if let Some(h) = self.textures.get(&(t, repeat)) {
            return h.clone();
        }
        let tex = mcla_flash::textures::resolve(&self.player.movie, t, &self.shared);
        let h = tex.map(|t| {
            let img = plain_image(t.width, t.height, &t.to_rgba8());
            let opt = if repeat { egui::TextureOptions::LINEAR_REPEAT } else { egui::TextureOptions::LINEAR };
            ctx.load_texture(format!("swf:{}:{:?}:{}", self.name, t.name, repeat), img, opt)
        });
        if h.is_none() && t != TexRef::None {
            log::warn!("flash: {}: no texture for {:?}", self.name, t);
        }
        self.textures.insert((t, repeat), h.clone());
        h
    }

    /// Stage rectangle on screen. 1280x720 movies: the stage height fills the
    /// screen height, centred. Smaller stages (pause.swf, 720x512) keep the
    /// 1280x720 pixel scale and sit at `self.offset` (720p pixels).
    pub fn stage_rect(&self, screen: Rect) -> (f32, Pos2) {
        let s = &self.player.movie.stage;
        let (w, h) = (s[1] - s[0], s[3] - s[2]);
        if h >= 719.0 {
            let k = screen.height() / h;
            return (k, Pos2::new(screen.center().x - w * k * 0.5 - s[0] * k + self.offset[0] * k, screen.top() - s[2] * k + self.offset[1] * k));
        }
        let k = screen.height() / 720.0;
        let left = screen.center().x - 640.0 * k;
        (k, Pos2::new(left + self.offset[0] * k - s[0] * k, screen.top() + self.offset[1] * k - s[2] * k))
    }

    pub fn paint(&mut self, ctx: &egui::Context, painter: &egui::Painter) {
        let list = mcla_flash::draw::build(&self.player);
        if std::env::var("MCLA_FLASH_LIST_LOG").map_or(false, |v| self.name.contains(&v)) {
            for it in &list.items {
                if let Item::Triangles { tex, kind, verts } = it {
                    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                    for v in verts {
                        x0 = x0.min(v.pos[0]);
                        y0 = y0.min(v.pos[1]);
                        x1 = x1.max(v.pos[0]);
                        y1 = y1.max(v.pos[1]);
                    }
                    if (x1 - x0) * (y1 - y0) > 20000.0 {
                        log::info!("flash list: {:?} kind {:#x} n {} color {:?} box {:.0},{:.0}..{:.0},{:.0}", tex, kind, verts.len(), verts[0].color, x0, y0, x1, y1);
                    }
                }
            }
        }
        self.paint_list(ctx, painter, &list);
    }

    /// The stage drawn into a `w` x `h` picture over transparent black: a movie the game renders
    /// into its own target (`3dicons`, 256x256, the texture of its `player0mesh`). Pictures are
    /// kept in `cache`.
    pub fn raster(&self, w: u32, h: u32, cache: &mut HashMap<TexRef, Option<Arc<mcla_flash::raster::TexImage>>>) -> Vec<u8> {
        let list = mcla_flash::draw::build(&self.player);
        for it in &list.items {
            if let Item::Triangles { tex, .. } = it {
                if *tex != TexRef::None && !matches!(tex, TexRef::Swap(_)) && !cache.contains_key(tex) {
                    let t = mcla_flash::textures::resolve(&self.player.movie, *tex, &self.shared);
                    cache.insert(*tex, t.map(|t| Arc::new(mcla_flash::raster::TexImage { w: t.width, h: t.height, rgba: t.to_rgba8() })));
                }
            }
        }
        let st = self.player.movie.stage;
        let k = w as f32 / (st[1] - st[0]).max(1.0);
        let mut c = mcla_flash::raster::Canvas::new(w, h, [0, 0, 0, 0]);
        c.draw(&list, k, [-st[0] * k, -st[2] * k], &|t| cache.get(&t).cloned().flatten());
        c.rgba
    }

    /// The rectangle a sprite covers, in stage pixels.
    ///
    /// The garage movie places its menu box by moving sprites rather than by a
    /// fixed rect, so anything the port draws around the box (its outline, the
    /// prompt bar it takes to the bottom of the screen) has to follow where the
    /// movie actually put it. The sprite's own drawing is measured in its
    /// parent's space, so the ancestor chain is composed over it - the port
    /// scales the box horizontally, and a rect that ignored that would sit at
    /// the unscaled width.
    pub fn sprite_rect(&self, path: &str) -> Option<Rect> {
        let id = self.player.find(path)?;
        let b = mcla_flash::draw::bounds(&self.player, id)?;
        // the ancestors, root first, so the fold runs outside in
        let mut chain = Vec::new();
        let mut cur = self.player.inst(id)?.parent;
        while let Some(p) = cur {
            let i = self.player.inst(p)?;
            chain.push(i.matrix);
            cur = i.parent;
        }
        let mut m = Matrix::IDENTITY;
        for a in chain.into_iter().rev() {
            m = m.concat(&a);
        }
        let (x0, y0) = m.apply(b[0], b[1]);
        let (x1, y1) = m.apply(b[2], b[3]);
        Some(Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1)))
    }

    /// A sprite's stage rectangle on screen, in the same pixels `paint` uses.
    pub fn sprite_rect_on(&self, path: &str, screen: Rect) -> Option<Rect> {
        let (k, o) = self.stage_rect(screen);
        let r = self.sprite_rect(path)?;
        Some(Rect::from_min_max(Pos2::new(o.x + r.min.x * k, o.y + r.min.y * k), Pos2::new(o.x + r.max.x * k, o.y + r.max.y * k)))
    }

    /// Stretch a sprite horizontally about its own origin, `x` as a factor
    /// (1.0 leaves it alone; the movie's `_xscale` is a percentage).
    ///
    /// Used on the garage menu box: the movie sizes it to whatever the brand
    /// logo and the price column happen to measure, and there is no global it
    /// will hold still (it recomputes `blk_width` from its content every
    /// frame), so the laid-out box is scaled to one width after the fact.
    pub fn scale_sprite_x(&mut self, path: &str, x: f32) {
        if let Some(id) = self.player.find(path) {
            self.player.set_member(Ref::Inst(id), &Value::str("_xscale"), Value::Float(x * 100.0));
        }
    }

    /// Draw the movie through a matrix (stage pixels -> clip space): the movie as
    /// the texture of a mesh in the menus' 3D scene (`menu3d`). Masks and the
    /// stage edge clip in stage space, where they are rectangles; `alpha`
    /// (0..1) is the mesh's own.
    pub fn paint_warped(&mut self, ctx: &egui::Context, painter: &egui::Painter, list: &DrawList, m: glam::Mat4, screen: Rect, alpha: f32, part: Option<[f32; 4]>) {
        let s = self.player.movie.stage;
        // (`part`: the piece of the stage the mesh carries, when it is not all of it)
        let stage = match part {
            Some(p) => p,
            None => [s[0], s[2], s[1], s[3]],
        };
        let mut clips: Vec<[f32; 4]> = vec![stage];
        let mut mask: Option<[f32; 4]> = None;
        let meet = |a: [f32; 4], b: [f32; 4]| [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
        for it in &list.items {
            match it {
                Item::MaskBegin => mask = Some([f32::MAX, f32::MAX, f32::MIN, f32::MIN]),
                Item::MaskEnd => {
                    let r = mask.take().unwrap_or([0.0; 4]);
                    let top = *clips.last().unwrap();
                    clips.push(if r[2] > r[0] && r[3] > r[1] { meet(top, r) } else { top });
                }
                Item::MaskPop => {
                    if clips.len() > 1 {
                        clips.pop();
                    }
                }
                Item::Triangles { tex, kind, verts } => {
                    if let Some(r) = &mut mask {
                        for v in verts {
                            *r = [r[0].min(v.pos[0]), r[1].min(v.pos[1]), r[2].max(v.pos[0]), r[3].max(v.pos[1])];
                        }
                        continue;
                    }
                    let handle = if *tex == TexRef::None { None } else { self.texture(ctx, *tex, *kind == 0x40 || *kind == 0x42) };
                    if *tex != TexRef::None && handle.is_none() {
                        continue;
                    }
                    let mut mesh = match &handle {
                        Some(h) => Mesh::with_texture(h.id()),
                        None => Mesh::default(),
                    };
                    let clip = *clips.last().unwrap();
                    let mut poly: Vec<WarpVert> = Vec::with_capacity(8);
                    for t in verts.chunks_exact(3) {
                        poly.clear();
                        poly.extend(t.iter().map(|v| WarpVert { pos: v.pos, uv: v.uv, color: [v.color[0] as f32, v.color[1] as f32, v.color[2] as f32, v.color[3] as f32] }));
                        clip_poly(&mut poly, clip);
                        if poly.len() < 3 {
                            continue;
                        }
                        let base = mesh.vertices.len() as u32;
                        let mut ok = true;
                        for v in &poly {
                            let Some(pos) = crate::menu3d::to_screen(m * glam::Vec4::new(v.pos[0], v.pos[1], 0.0, 1.0), screen) else {
                                ok = false;
                                break;
                            };
                            let uv = if handle.is_some() { Pos2::new(v.uv[0], v.uv[1]) } else { egui::epaint::WHITE_UV };
                            mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: plain_color(v.color[0] as u8, v.color[1] as u8, v.color[2] as u8, (v.color[3] * alpha) as u8) });
                        }
                        if !ok {
                            mesh.vertices.truncate(base as usize);
                            continue;
                        }
                        for k in 1..poly.len() as u32 - 1 {
                            mesh.indices.extend([base, base + k, base + k + 1]);
                        }
                    }
                    if !mesh.indices.is_empty() {
                        painter.add(egui::Shape::mesh(mesh));
                    }
                }
            }
        }
    }

    pub fn paint_list(&mut self, ctx: &egui::Context, painter: &egui::Painter, list: &DrawList) {
        let screen = painter.clip_rect();
        let (k, o) = self.stage_rect(ctx.screen_rect());
        // a stage smaller than the screen is a render target in the original: clip to it
        let s = self.player.movie.stage;
        let stage = Rect::from_min_max(Pos2::new(o.x + s[0] * k, o.y + s[2] * k), Pos2::new(o.x + s[1] * k, o.y + s[3] * k));
        let screen = if s[3] - s[2] < 719.0 { screen.intersect(stage) } else { screen };
        let mut clips: Vec<Rect> = vec![screen];
        // the shape of each mask (its triangles), beside its box
        let mut shapes: Vec<Vec<Pos2>> = vec![Vec::new()];
        let mut shape: Vec<Pos2> = Vec::new();
        let mut mask: Option<Rect> = None;
        for it in &list.items {
            match it {
                Item::MaskBegin => {
                    mask = Some(Rect::NOTHING);
                    shape.clear();
                }
                Item::MaskEnd => {
                    shapes.push(std::mem::take(&mut shape));
                    let m = mask.take().unwrap_or(Rect::NOTHING);
                    let top = *clips.last().unwrap();
                    if std::env::var("MCLA_FLASH_MASK_LOG").is_ok() {
                        log::info!("flash: {} mask {:?} within {:?}", self.name, m, top);
                    }
                    clips.push(if m.is_positive() { top.intersect(m) } else { top });
                }
                Item::MaskPop => {
                    if clips.len() > 1 {
                        clips.pop();
                        shapes.pop();
                    }
                }
                Item::Triangles { tex, kind, verts } => {
                    if let Some(m) = &mut mask {
                        for v in verts {
                            let at = Pos2::new(o.x + v.pos[0] * k, o.y + v.pos[1] * k);
                            *m = m.union(Rect::from_min_size(at, egui::Vec2::ZERO));
                            shape.push(at);
                        }
                        continue;
                    }
                    // A plain fill of one colour that covers its whole mask is drawn as the
                    // mask's own shape: the box of a mask would square off what the movie
                    // rounds (the replay legend's `BG_masked` under the rounded `BG`).
                    if let (true, Some(sh), Some(first)) = (*tex == TexRef::None && clips.len() > 1, shapes.last().filter(|s| s.len() >= 3), verts.first()) {
                        let same = verts.iter().all(|v| v.color == first.color);
                        let mut cover = Rect::NOTHING;
                        for v in verts {
                            cover = cover.union(Rect::from_min_size(Pos2::new(o.x + v.pos[0] * k, o.y + v.pos[1] * k), egui::Vec2::ZERO));
                        }
                        let mut need = Rect::NOTHING;
                        for q in sh {
                            need = need.union(Rect::from_min_size(*q, egui::Vec2::ZERO));
                        }
                        if same && cover.expand(0.5).contains_rect(need) {
                            let mut mesh = Mesh::default();
                            let color = plain_color(first.color[0], first.color[1], first.color[2], first.color[3]);
                            for q in sh {
                                mesh.vertices.push(egui::epaint::Vertex { pos: *q, uv: egui::epaint::WHITE_UV, color });
                            }
                            mesh.indices = (0..mesh.vertices.len() as u32).collect();
                            painter.with_clip_rect(clips[clips.len() - 2]).add(egui::Shape::mesh(mesh));
                            continue;
                        }
                    }
                    let handle = if *tex == TexRef::None { None } else { self.texture(ctx, *tex, *kind == 0x40 || *kind == 0x42) };
                    if *tex != TexRef::None && handle.is_none() {
                        continue;
                    }
                    let mut mesh = match &handle {
                        Some(h) => Mesh::with_texture(h.id()),
                        None => Mesh::default(),
                    };
                    for v in verts {
                        let uv = if handle.is_some() { Pos2::new(v.uv[0], v.uv[1]) } else { egui::epaint::WHITE_UV };
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: Pos2::new(o.x + v.pos[0] * k, o.y + v.pos[1] * k),
                            uv,
                            color: plain_color(v.color[0], v.color[1], v.color[2], v.color[3]),
                        });
                    }
                    mesh.indices = (0..mesh.vertices.len() as u32).collect();
                    painter.with_clip_rect(*clips.last().unwrap()).add(egui::Shape::mesh(mesh));
                }
            }
        }
    }
}

/// A colour for the UI pass, which blends on the encoded values as the original does
/// (see main.rs "ui"): premultiplied plainly. `Color32::from_rgba_unmultiplied`
/// premultiplies in linear light, which made every translucent white about twice as
/// strong and every translucent black weaker.
pub fn plain_color(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    let m = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(m(r), m(g), m(b), a)
}

/// A texture for the UI pass from straight-alpha RGBA, premultiplied the same way.
pub fn plain_image(w: u32, h: u32, rgba: &[u8]) -> egui::ColorImage {
    let pixels = rgba.chunks_exact(4).map(|p| plain_color(p[0], p[1], p[2], p[3])).collect();
    egui::ColorImage { size: [w as usize, h as usize], pixels }
}

/// A vertex being clipped in stage space.
#[derive(Clone, Copy)]
struct WarpVert {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

/// Sutherland-Hodgman against `[x0, y0, x1, y1]`, attributes interpolated.
fn clip_poly(poly: &mut Vec<WarpVert>, r: [f32; 4]) {
    for (axis, bound, keep_less) in [(0, r[0], false), (0, r[2], true), (1, r[1], false), (1, r[3], true)] {
        if poly.is_empty() {
            return;
        }
        let inside = |v: &WarpVert| if keep_less { v.pos[axis] <= bound } else { v.pos[axis] >= bound };
        if poly.iter().all(inside) {
            continue;
        }
        let src = std::mem::take(poly);
        for i in 0..src.len() {
            let (a, b) = (src[i], src[(i + 1) % src.len()]);
            let (ia, ib) = (inside(&a), inside(&b));
            if ia {
                poly.push(a);
            }
            if ia != ib {
                let t = (bound - a.pos[axis]) / (b.pos[axis] - a.pos[axis]);
                let l = |x: f32, y: f32| x + (y - x) * t;
                poly.push(WarpVert { pos: [l(a.pos[0], b.pos[0]), l(a.pos[1], b.pos[1])], uv: [l(a.uv[0], b.uv[0]), l(a.uv[1], b.uv[1])], color: [l(a.color[0], b.color[0]), l(a.color[1], b.color[1]), l(a.color[2], b.color[2]), l(a.color[3], b.color[3])] });
            }
        }
    }
}

/// FSCommands the movies send (`0x82721CC8` handles getvar/setvar; the
/// measuring commands write their result back into `_global`).
fn fscommand(p: &mut Player, cmd: &str, arg: &str, strings: &StringTable, logic: &crate::menu3d::UiLogic) {
    if mcla_flash::player::common_fscommand(p, cmd, arg) {
        return;
    }
    match cmd.to_ascii_lowercase().as_str() {
        // `_global.getstring(id)`: fs_stringID -> t_stringID (game side 0x82205D78)
        "getstring" => {
            let id = p.get_global("fs_stringID").map(|v| v.to_string().to_string()).unwrap_or_default();
            let text = strings.get(&id).map(|s| s.to_string()).unwrap_or(id);
            p.set_global("t_stringID", Value::str(&text));
        }
        // the menu movies move the game's camera between the tuned pause cameras
        "setpolarcam" => logic.set_polar_cam(p, arg),
        _ => log::debug!("flash: FSCommand {} {}", cmd, arg),
    }
}

/// Button glyphs of the prompt bar (`iconID`, frame of the prompt icon; the
/// PlayStation set follows 49 frames later).
pub const ICON_A: i32 = 1;
pub const ICON_B: i32 = 2;
pub const ICON_X: i32 = 3;
pub const ICON_Y: i32 = 4;
pub const ICON_LB: i32 = 5;
pub const ICON_RB: i32 = 6;
pub const ICON_START: i32 = 9;
/// (7 LT, 8 RT, 11..18 the sticks, 19 the d-pad; from the prompt movie's frames)
pub const ICON_LT_RT: i32 = 23;
pub const ICON_LB_RB: i32 = 24;
pub const ICON_ARROWS: i32 = 25;
pub const ICON_BACK_BUTTON: i32 = 10;

/// The garage movie's logo (`vs_brands` frame - 1, the value of a part row's `text3`)
/// of a car's maker, from the maker part of the car's resource name
/// (`vp_aud_r8_08`). The frames were read off a render of all 96 (`work/ui/s11/
/// brands_sheet.png`); 0 is the game's own "custom" badge, 12 is blank.
pub fn maker_logo(car_dir: &str) -> i32 {
    let name = car_dir.to_ascii_lowercase();
    let maker = name.split('_').nth(1).unwrap_or("");
    match maker {
        "aud" => 35,
        "buk" => 36,
        "chv" => 37,
        "cry" => 38,
        "dsn" => 39,
        "dge" => 40,
        "duc" => 41,
        "frd" => 42,
        "kaw" => 46,
        "lam" => 47,
        "mbz" => 50,
        "mit" => 51,
        "nsn" => 52,
        "pon" => 53,
        "sln" => 54,
        "vw" => 55,
        "mzd" => 62,
        "am" => 71,
        "cad" => 81,
        "lr" => 82,
        _ => 12,
    }
}

/// Row values with a meaning for the menu movie: a ticked / empty box, and a prefix
/// for a value that left and right change (drawn between arrows).
pub const CHECK_ON: &str = "\u{1}on";
pub const CHECK_OFF: &str = "\u{1}off";
pub const CHOICE: &str = "\u{2}";
/// A colour row of the paint shop: `SWATCH` then `rgb;select;c0,c1,...` - the colour on
/// the car, the open strip's cursor (-1 = the strip is closed) and the strip's colours.
pub const SWATCH: &str = "\u{3}";
/// A row whose value is a button glyph: `GLYPH` then the glyph's number (the movie's row
/// `type` 7 with the number in `text2` - the custom controls' BUTTON column, sub_8264BDA8).
pub const GLYPH: &str = "\u{4}";
/// A garage row whose part type wears the "new" mark: the movie's row `type` 50
/// (sub_826A8D48, 0x826A9208..0x826A9244).
pub const NEW_ROW: &str = "\u{5}new";

/// A choice row's value with all its choices (`at` is the one shown).
pub fn choice_list(at: usize, names: &[String]) -> String {
    format!("{}{}\t{}", CHOICE, at, names.join("\t"))
}

/// A paint shop colour row's value (see `SWATCH`).
pub fn swatch_value(rgb: u32, select: i32, strip: &[u32]) -> String {
    format!("{}{};{};{}", SWATCH, rgb, select, strip.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","))
}

/// One list row: label (`text1`), value (`text2`), selectable (`property1`).
#[derive(Clone, PartialEq, Debug)]
pub struct MenuRow {
    pub label: String,
    pub value: String,
    pub enabled: bool,
    pub part: Option<PartRow>,
}

/// Part selection takes a number in text2, not a preformatted "$..." string.
/// text3 selects vs_brands at frame (brand + 1); 12 is the blank/default logo
/// in garage/elements/PartType.sc.xml. Unknown makers must not become brand 0.
#[derive(Clone, PartialEq, Debug)]
pub struct PartRow {
    pub price: i64,
    pub brand: i32,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub enum MenuLayout {
    #[default]
    List,
    /// In the movie, index + 1 is the LEFT neighbour, index - 1 the RIGHT.
    /// This is title_loop (template 6), distinct from GarageTabs.
    /// (`flat`: the movie's `garageLoop2d`, template 49 - the areas seen from inside the
    /// car hang flat in the screen's top right corner, not in the scene)
    GarageLoop { names: Vec<String>, index: usize, flat: bool },
    GarageParts,
    /// The rim shop (`Rim.sc.xml`, `fs_template` 47): the movie's `rimselection`
    /// panel. `tabs` holds each brand's `fs_mfr` in tab order and `cur_tab` the
    /// open brand; the movie shows `vs_brands` at `tabs[cur_tab - 1] + 1`, which
    /// is the maker logo drawn between the LB/RB arrows.
    GarageRims,
    /// the REWARDS panel after an event (`template_reward` 62 of the pause movie; reference
    /// `y_rewards`): the rows are rewards by kind (`row_props`: 11 reputation - label in
    /// the row's label, the rank's line in its value - 12 money); the figures the movie
    /// counts up come as globals: total reputation, the words and figure of the next rank,
    /// how full the rank's meter is (0..100) and the money
    Reward { total_rep: i64, next_rank: String, rep_label: String, next_label: String, meter: f32, money: i64 },
    /// the ARRESTED panel (`template_ArrestedPanel` 59 of the popup movie; reference
    /// `y_arrested`): the page's description, then its rows as label and figure
    Arrested,
    /// a hangout's panel (`template_HangoutMenu` 64, flat: the place's name, what is run
    /// there, then rows - the first a choice; reference `hg_series_0`)
    /// `slider`: a row that is the wager's slider (`property1` 16: the amount, its least
    /// and most in `text2..4`); `money`: the player's money on the panel's foot
    Hangout { description: String, slider: Option<(usize, i64, i64, i64)>, money: Option<i64> },
    /// a tutorial's box (`template_tutorial` 60: the page's title in the title colour and
    /// this text under it, top left; reference `ng_c4`)
    Tutorial { description: String },
    /// the place a race was finished in, in its ring (`template_position` 51: the page's
    /// title is the number, this its ending - "2", "ND"; reference `ng_finish_2nd`)
    /// (`look`: 0 the place, 1 TIED, 2 DNF - the movie's `position_tie` / `position_DNF`)
    Position { suffix: String, look: u8 },
    /// the badge over a race's opening shot (`template_StartOfRace` 57: the clip
    /// `raceName` - the page's title, this line under it, a ring of this colour around the
    /// page's icon; reference `y_replay_intro`)
    StartOfRace { description: String, ring: u32 },
    /// the plate's seven characters as places in the movie's own table `LP_chars`
    /// (`template_licenceplate` 45; the page's `select` is the character under the cursor)
    LicensePlate { chars: [usize; 7] },
    /// The vehicle list (`template_manageride`, 41): names on the left; on the right
    /// the chosen vehicle's class line (`text2`), kind (`text3`), three meters and a
    /// money figure (`price`). The meters are the globals `cur_/base_/max_` +
    /// `acceleration` / `speed` / `handling` (bar widths in percent): `base` the stock
    /// car (teal), `cur` with what is fitted (yellow), `max` the outline. `value_label`
    /// replaces "TRADE IN VALUE:" (`t_tradeinvalue`). The layout is 2D: a full-screen
    /// panel on a black backdrop.
    /// `ids`: per vehicle the game's id (the hash of its folder name, which the movie
    /// looks up in its `vehicle_ID` table to name the pictures) and whether it can be
    /// bought; given, the page is the showroom's carousel (`template_showroom`, 40)
    Vehicles { details: Vec<(String, String, i64)>, meters: [[f32; 3]; 3], value_label: String, ids: Vec<(i32, bool)> },
    /// The performance shop (`template_PS_level`, 121). Per part: `level` and `text3`
    /// place the "installed" mark on one of three boxes (stock, stage 1, stage 2),
    /// `property1` is how many boxes are unlocked, `text2` the price text, `ID` the
    /// brand logo (`ps_brands_mc` frame - 1). `carclassID` lights the class tab
    /// (0 = A .. 3 = D), `val_pstotal` is the total line, and the three meters are the
    /// same globals as the vehicle list's.
    PerfShop { details: Vec<(u8, u8, String, i32)>, class: i32, total: i64, meters: [[f32; 3]; 3] },
    /// The mission log (`template_MissionLog`, 11, of the GPS map movie): the list on the
    /// left; on the right the chosen mission's picture, `menu.summary`, and up to five
    /// `menu.objective<n>` lines with a tick box each (`menu.objective_status[n-1]`).
    MissionLog { summary: String, objectives: Vec<(String, bool)> },
    /// A message box of the popup movie (`template_confbox`, 3): the title bar, a
    /// description wrapped by the movie (it asks the host for the line count,
    /// `FSCommand:MeasureSlotHeight`) and the choices.
    Confirm { description: String },
    /// SPECIAL ABILITIES (`template_abilities`, 123): per ability its description
    /// (`text2`), level 0..3 (`text3`, the stars) and meter in percent (`text4`)
    Abilities { details: Vec<(String, i32, i32)>, level_label: String },
    /// VINYL PACKAGES (`Vinyl_Preset_Selection`: `template_VE_preset`, 115): the list and,
    /// beside it, the picture of the package under the cursor - a frame of the movie's
    /// `VE_preset_lib`, which it picks from `cur_vehicle` (the car's hash) and the cursor
    VinylPackages { vehicle: i32 },
    /// the vinyl editor's CHOOSE GROUP (`template_VE_area`, 110): the car's diagram with
    /// the group `menu.select` lit
    VinylGroup,
    /// CHOOSE LAYER (`template_VE_layers`, 101): a strip of the group's layer slots;
    /// globals `cur_VE_group_type` 0 and `cur_VE_group_ID` put the group's diagram beside it
    VinylLayers { group: i32 },
    /// RACE RESULTS (`template_ResultsPanel`, 66): a table - `menu.type` columns under
    /// `menu.headers1..`, a row's cells in `text1..`
    Results { headers: Vec<String>, cells: Vec<Vec<String>> },
    /// the shape strip (`template_VE_shapes`, 102): the group's shapes seven at a time
    /// from `menu.offset`, the groups as tabs
    VinylShapes { offset: i32 },
    /// a tab of EDIT LAYER: `template` 104 (two figures), 105 (one), 116 (mask) or 103
    /// (colour strip); the figures go in `VElayer_dimension`, the labels are the movie's
    /// own by `cur_tab`; `colors` fill the strip
    VinylEditTab { template: i32, dims: [i32; 2], colors: Vec<u32> },
    /// LAYER OPTIONS (`template_VE_layer_menu`, 113): a list over the layer strip
    VinylOptions { group: i32 },
    /// the GPS map's own panel (`template_Navsys`, 10): rank, vehicle rank and class,
    /// money, and - with `legend` - the key to the map's icons. `globals` are the
    /// movie's text variables by name.
    GpsMap { legend: bool, globals: Vec<(String, String)>, meters: [f32; 2] },
    /// the CART (`template_shoppingcart`, 122): the parts chosen and their prices, the
    /// total on a yellow bar (`sc_total`, `sc_totaltxt`)
    Cart { total: i64, label: String },
    /// the paint shop (`PartColorMenu`: `template_Menu_2d`, 42): a flat panel at the top
    /// right with choice rows and colour rows
    Paint,
    /// `template_colorpicker2D` (117): hue, saturation and brightness in percent and the
    /// pure hue as RGB (`colorpickerH/S/B`, `colorpickerHtoRGB`)
    ColorPicker { h: i32, s: i32, b: i32, hue_rgb: i32 },
    /// `template_SongList` (67): the playlist pages, a wide box centred on the screen
    SongList { description: String },
    /// a plain list (`template_Menu`, 2) with a header over each column (`menu.headers<n>`;
    /// the custom controls' ACTION / BUTTON)
    Columns { headers: Vec<String> },
}

/// A page of the menu movie as the game's menu classes write it: the tab strip
/// (`0x82630E40`: `tabs_count`, `tabs`, `tabs_iconID`, `cur_tab`, `template`
/// 56 = template_MenuTabs), or a titled page (`template` 2 = template_Menu,
/// `menu.name`), the list (`0x82631FD0`: `menu.count`, `menu.type` = columns,
/// `menu.elements[i].text1/text2/ID/color/property1`, `menu.select`,
/// `menu.List_max`) and the prompt bar (`prompt.count`, `prompt.elements[i]
/// .text1/iconID`).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct MenuPage {
    pub layout: MenuLayout,
    pub icon: i32,
    /// the line under the title (`menu.description`: "Select Genre:")
    pub description: String,
    pub tabs: Vec<String>,
    /// icon frame per tab (`tabs_iconID`); empty = 1, 2, 3 (the pause menu's)
    pub tab_icons: Vec<i32>,
    /// per row `property1` in place of 0/1 (the GPS movie: 3..6 = a row icon, frames 1..4)
    pub row_props: Vec<i32>,
    pub cur_tab: usize,
    pub title: String,
    pub rows: Vec<MenuRow>,
    pub select: usize,
    pub prompts: Vec<(String, i32)>,
    /// the money line of the prompt bar (`showmoney`, `t_banktxt`, `bank`; `0x8267F518`)
    pub money: Option<(String, i64)>,
    /// pad row labels to this many characters so the movie measures a constant
    /// content width and the panel does not resize as the cursor moves
    pub pad_labels: usize,
    /// rows the list shows at once (`menu.List_max`; 0 = the layout's own). The pause
    /// menu shows six and a scrollbar beside longer lists; the movie scrolls itself.
    pub list_max: usize,
}

/// A menu movie: `pause/pause.swf` (pause menu) or `garage/garage.swf`, which
/// share the list protocol.
pub struct FlashMenu {
    pub movie: FlashMovie,
    shown: Option<MenuPage>,
}

pub const PAUSE_MOVIE: &str = "resources/ui/pause/0xaed9b6c9.r27";
pub const GARAGE_MOVIE: &str = "resources/ui/garage/0xfcc35320.r27";
/// `navsys.swf`: the GPS map's panels (mission log, legend, event panels).
pub const NAVSYS_MOVIE: &str = "resources/ui/navsys/0x733d5b6e.r27";
/// `popup.swf` (the folder is the hash of "popup"): message boxes, and the 2D set of the
/// menu templates (results, event and arrest panels, rewards ...). It has no 3D scene:
/// the movie lays its box out on the 1280x720 stage itself.
pub const POPUP_MOVIE: &str = "resources/ui/0xe83b42ec/0x03770225.r27";

/// One box of `tune/ui/MessageBoxes.sc.xml` (`class="UIMessageBox"`): string keys of
/// the title and the description, and the choices in the file's order (the first is
/// the one the box opens on).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MessageBoxDef {
    pub title: String,
    pub description: String,
    /// (state id, `desc` string key)
    pub options: Vec<(String, String)>,
}

impl MessageBoxDef {
    pub fn load(data_dir: &str, id: &str) -> Option<MessageBoxDef> {
        // (the garage keeps its own boxes in its state file)
        for file in ["tune/ui/MessageBoxes.sc.xml", "tune/ui/garage/Main.sc.xml", "tune/ui/garage/Vinyls.sc.xml"] {
            if let Some(def) = std::fs::read_to_string(Path::new(data_dir).join(file)).ok().and_then(|text| MessageBoxDef::parse(&text, id)) {
                return Some(def);
            }
        }
        None
    }

    pub fn parse(text: &str, id: &str) -> Option<MessageBoxDef> {
        let start = text.find(&format!("<state id=\"{id}\""))?;
        let rest = &text[start..];
        // the box ends at its own closing tag (states nest)
        let mut depth = 0i32;
        let mut end = rest.len();
        let mut at = 0usize;
        while let Some(k) = rest[at..].find("state") {
            let i = at + k;
            if i >= 1 && rest.as_bytes()[i - 1] == b'<' {
                depth += 1;
            } else if i >= 2 && &rest[i - 2..i] == "</" {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            at = i + 5;
        }
        let body = &rest[..end];
        let attr = |s: &str, name: &str| -> Option<String> {
            let t = s.split_once(&format!("{name}=\""))?.1;
            Some(t[..t.find('"')?].to_string())
        };
        let data = |name: &str| body.split("<data ").skip(1).find(|d| attr(d, "name").as_deref() == Some(name)).and_then(|d| attr(d, "expr")).unwrap_or_default();
        let options = body
            .split("<state id=\"")
            .skip(2)
            .filter_map(|c| {
                let (oid, tail) = c.split_once('"')?;
                let head = tail.split('>').next().unwrap_or("");
                Some((oid.to_string(), attr(head, "desc")?))
            })
            .collect();
        Some(MessageBoxDef { title: data("title"), description: data("description"), options })
    }
}

impl FlashMenu {
    /// `offset`: where the stage goes, in 720p pixels. The original shows the
    /// pause movie on a panel in the 3D scene, left of the car; the port places
    /// it flat.
    pub fn load(data_dir: &str, rel: &str, offset: [f32; 2], shared: &UiShared) -> Option<FlashMenu> {
        let mut movie = FlashMovie::load(data_dir, rel, shared)?;
        movie.offset = offset;
        // let the movie's frame-0 setup run before the first page arrives
        for _ in 0..4 {
            movie.player.tick();
        }
        Some(FlashMenu { movie, shown: None })
    }

    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }

    /// Show `page`. A changed structure rebuilds the page (`TransitionLevel` 0);
    /// a changed selection only refreshes the list (`TransitionLevel` 1, `0x82635840`).
    pub fn show(&mut self, page: &MenuPage) {
        if self.shown.as_ref() == Some(page) {
            return;
        }
        let p = &mut self.movie.player;
        let structure_changed = self.shown.as_ref().map_or(true, |s| s.layout != page.layout || s.icon != page.icon || s.tabs != page.tabs || s.cur_tab != page.cur_tab || s.title != page.title || s.rows.len() != page.rows.len() || s.prompts != page.prompts);
        p.set_global("cur_visibility", Value::Int(1));
        // (wide screen; the menus' 2D layouts take their size from `pos_2d[aspect]`)
        p.set_global("aspect", Value::Int(1));
        p.set_global("tabs_count", Value::Int(page.tabs.len() as i32));
        // the rim shop's tabs are the maker ids and the movie does arithmetic on
        // them (vs_brands goes to frame tabs[cur_tab - 1] + 1), so they are numbers
        let rim_tabs = page.layout == MenuLayout::GarageRims;
        for (i, t) in page.tabs.iter().enumerate() {
            p.set_path(&format!("tabs.{}", i), if rim_tabs { Value::Int(t.parse().unwrap_or(0)) } else { Value::str(t) });
            // the pause tabs' icons are frames 1..3 of the tab icon: pause, game pad, tick
            p.set_path(&format!("tabs_iconID.{}", i), Value::Int(if rim_tabs { 0 } else { page.tab_icons.get(i).copied().unwrap_or(i as i32 + 1) }));
        }
        p.set_global("cur_tab", Value::Int(page.cur_tab as i32 + 1));
        p.set_global("type", Value::Int(1));
        let template = match &page.layout {
            MenuLayout::GarageLoop { names, index, flat } => {
                p.set_global("glprompt_count", Value::Int(names.len() as i32));
                p.set_global("glprompt_pos", Value::Int((*index).min(names.len().saturating_sub(1)) as i32));
                for (i, name) in names.iter().enumerate() {
                    p.set_path(&format!("glprompt_STnames.{}", i), Value::str(name));
                }
                if *flat { 49 } else { 6 }
            }
            MenuLayout::GarageParts => 5,
            MenuLayout::GarageRims => 47,
            MenuLayout::Reward { total_rep, next_rank, rep_label, next_label, meter, money } => {
                p.set_global("reward_rep", Value::Int((*total_rep).clamp(0, i32::MAX as i64) as i32));
                p.set_global("reward_money", Value::Int((*money).clamp(0, i32::MAX as i64) as i32));
                p.set_global("next_rank", Value::str(next_rank));
                p.set_global("t_rep", Value::str(rep_label));
                p.set_global("t_next_rank", Value::str(next_label));
                p.set_global("rep_meter", Value::Float(meter.clamp(0.0, 100.0)));
                for (i, r) in page.rows.iter().enumerate() {
                    p.set_path(&format!("menu.elements.{}.text3", i), Value::str(&r.value));
                    // (a mission's line - `property1` 25: `text4` 1 stamps it COMPLETE, else FAIL)
                    p.set_path(&format!("menu.elements.{}.text4", i), Value::Int((r.value == "1") as i32));
                }
                62
            }
            MenuLayout::Arrested => 59,
            MenuLayout::Position { look, .. } => {
                p.set_global("position_tie", Value::Int((*look == 1) as i32));
                p.set_global("position_DNF", Value::Int((*look == 2) as i32));
                51
            }
            MenuLayout::Tutorial { .. } => 60,
            MenuLayout::Hangout { .. } => 64,
            MenuLayout::StartOfRace { description, ring } => {
                p.set_path("menu.description", Value::str(description));
                p.set_path("menu.type", Value::Int(*ring as i32));
                57
            }
            MenuLayout::LicensePlate { chars } => {
                for (i, c) in chars.iter().enumerate() {
                    p.set_path(&format!("LP_slotsIdx.{}", i), Value::Int(*c as i32));
                }
                45
            }
            MenuLayout::MissionLog { summary, objectives } => {
                p.set_path("menu.summary", Value::str(summary));
                for k in 0..5 {
                    let (text, done) = objectives.get(k).cloned().unwrap_or_default();
                    p.set_path(&format!("menu.objective{}", k + 1), Value::str(&text));
                    p.set_path(&format!("menu.objective_status.{}", k), Value::Int(done as i32));
                }
                11
            }
            MenuLayout::PerfShop { details, class, total, meters } => {
                for (i, (have, unlocked, price, brand)) in details.iter().enumerate() {
                    p.set_path(&format!("menu.elements.{}.level", i), Value::Int(*have as i32));
                    p.set_path(&format!("menu.elements.{}.text3", i), Value::Int(*have as i32));
                    // (`text2` 1..3 is the meter's frame: how many of the three boxes are owned - the
                    // stock part and every stage fitted; the price column is `price`, where 0 reads
                    // "---": garage movie 0x1039b, 0x10a0f, reference `n13`)
                    p.set_path(&format!("menu.elements.{}.text2", i), Value::Int(*have as i32 + 1));
                    p.set_path(&format!("menu.elements.{}.select", i), Value::Int(*have as i32));
                    if price == "---" {
                        p.set_path(&format!("menu.elements.{}.price", i), Value::Int(0));
                    } else {
                        p.set_path(&format!("menu.elements.{}.price", i), Value::str(price));
                    }
                    p.set_path(&format!("menu.elements.{}.text4", i), Value::str(price));
                    p.set_path(&format!("menu.elements.{}.property1", i), Value::Int(*unlocked as i32));
                    p.set_path(&format!("menu.elements.{}.ID", i), Value::Int(*brand));
                }
                for (k, name) in ["acceleration", "speed", "handling"].iter().enumerate() {
                    for (j, pre) in ["cur", "base", "max"].iter().enumerate() {
                        p.set_global(&format!("{pre}_{name}"), Value::Float(meters[k][j].clamp(0.0, 100.0)));
                    }
                }
                p.set_global("carclassID", Value::Int(*class));
                p.set_global("val_pstotal", Value::Int((*total).clamp(0, i32::MAX as i64) as i32));
                121
            }
            MenuLayout::Vehicles { details, meters, value_label, ids } => {
                for (i, (id, open)) in ids.iter().enumerate() {
                    p.set_path(&format!("menu.elements.{}.ID", i), Value::Int(*id));
                    // (`lock` 1 greys the price and shows the padlock)
                    p.set_path(&format!("menu.elements.{}.lock", i), Value::Int(!*open as i32));
                }
                for (i, (class, kind, price)) in details.iter().enumerate() {
                    p.set_path(&format!("menu.elements.{}.text2", i), Value::str(class));
                    p.set_path(&format!("menu.elements.{}.text3", i), Value::str(kind));
                    p.set_path(&format!("menu.elements.{}.price", i), Value::Int((*price).clamp(0, i32::MAX as i64) as i32));
                }
                for (k, name) in ["acceleration", "speed", "handling"].iter().enumerate() {
                    for (j, pre) in ["cur", "base", "max"].iter().enumerate() {
                        p.set_global(&format!("{pre}_{name}"), Value::Float(meters[k][j].clamp(0.0, 100.0)));
                    }
                }
                p.set_global("t_tradeinvalue", Value::str(value_label));
                if ids.is_empty() { 41 } else { 40 }
            }
            MenuLayout::Confirm { description } => {
                p.set_path("menu.description", Value::str(description));
                3
            }
            MenuLayout::Abilities { details, level_label } => {
                for (i, (text, level, meter)) in details.iter().enumerate() {
                    p.set_path(&format!("menu.elements.{}.text2", i), Value::str(text));
                    p.set_path(&format!("menu.elements.{}.text3", i), Value::Int(*level));
                    p.set_path(&format!("menu.elements.{}.text4", i), Value::Int(*meter));
                }
                p.set_global("t_level", Value::str(level_label));
                123
            }
            MenuLayout::VinylPackages { vehicle } => {
                p.set_global("cur_vehicle", Value::Int(*vehicle));
                115
            }
            MenuLayout::VinylGroup => 110,
            MenuLayout::VinylLayers { group } => {
                p.set_global("cur_VE_group_type", Value::Int(0));
                p.set_global("cur_VE_group_ID", Value::Int(*group));
                101
            }
            MenuLayout::Results { headers, cells } => {
                for (k, h) in headers.iter().enumerate() {
                    p.set_path(&format!("menu.headers{}", k + 1), Value::str(h));
                }
                for (i, row) in cells.iter().enumerate() {
                    for (k, c) in row.iter().enumerate() {
                        p.set_path(&format!("menu.elements.{}.text{}", i, k + 1), Value::str(c));
                    }
                }
                66
            }
            MenuLayout::VinylShapes { offset } => {
                p.set_path("menu.offset", Value::Int(*offset));
                102
            }
            MenuLayout::VinylEditTab { template, dims, colors } => {
                p.set_path("VElayer_dimension.0", Value::Int(dims[0]));
                p.set_path("VElayer_dimension.1", Value::Int(dims[1]));
                p.set_global("cur_VE_group_type", Value::Int(1));
                for (i, c) in colors.iter().enumerate() {
                    p.set_path(&format!("menu.elements.{}.color", i), Value::Int(*c as i32));
                }
                *template
            }
            MenuLayout::VinylOptions { group } => {
                p.set_global("cur_VE_group_type", Value::Int(0));
                p.set_global("cur_VE_group_ID", Value::Int(*group));
                113
            }
            MenuLayout::GpsMap { legend, globals, meters } => {
                for (k, v) in globals {
                    p.set_global(k, Value::str(v));
                }
                p.set_global("legend_visibility", Value::Int(*legend as i32));
                p.set_global("legend_type", Value::Int(0));
                p.set_global("rank_meter", Value::Float(meters[0].clamp(0.0, 1.0)));
                p.set_global("veh_meter", Value::Float(meters[1].clamp(0.0, 1.0)));
                10
            }
            MenuLayout::Cart { total, label } => {
                for (i, r) in page.rows.iter().enumerate() {
                    let price: i32 = r.value.trim_start_matches('$').parse().unwrap_or(0);
                    p.set_path(&format!("menu.elements.{}.price", i), Value::Int(price));
                }
                p.set_global("sc_total", Value::Int((*total).clamp(0, i32::MAX as i64) as i32));
                p.set_global("sc_totaltxt", Value::str(label));
                122
            }
            MenuLayout::Paint => 42,
            MenuLayout::ColorPicker { h, s, b, hue_rgb } => {
                p.set_global("colorpickerH", Value::Int(*h));
                p.set_global("colorpickerS", Value::Int(*s));
                p.set_global("colorpickerB", Value::Int(*b));
                p.set_global("colorpickerHtoRGB", Value::Int(*hue_rgb));
                117
            }
            MenuLayout::SongList { description } => {
                p.set_path("menu.description", Value::str(description));
                67
            }
            MenuLayout::List => if page.tabs.is_empty() { 2 } else { 56 },
            MenuLayout::Columns { headers } => {
                for (k, h) in headers.iter().enumerate() {
                    p.set_path(&format!("menu.headers{}", k + 1), Value::str(h));
                }
                2
            }
        };
        p.set_global("template", Value::Int(template));
        p.set_path("menu.template", Value::Int(template));
        p.set_path("menu.name", Value::str(&page.title));
        if !matches!(page.layout, MenuLayout::Confirm { .. } | MenuLayout::SongList { .. }) {
            p.set_path("menu.description", Value::str(&page.description));
        }
        p.set_path("menu.iconID", Value::Int(page.icon));
        p.set_global("transition_type", Value::Int(1));
        let two = page.rows.iter().any(|r| !r.value.is_empty());
        // (the paint shop is a simple list: its choices and swatches are drawn by that type's rows)
        p.set_path("menu.type", Value::Int(if matches!(page.layout, MenuLayout::GarageParts | MenuLayout::GarageRims) { 3 } else if matches!(page.layout, MenuLayout::Paint | MenuLayout::VinylPackages { .. }) { 1 } else if let MenuLayout::Results { headers, .. } = &page.layout { headers.len() as i32 } else if let MenuLayout::Columns { headers } = &page.layout { headers.len() as i32 } else if two || matches!(page.layout, MenuLayout::PerfShop { .. }) { 2 } else { 1 }));
        if let MenuLayout::Hangout { description, .. } = &page.layout {
            p.set_path("menu.description", Value::str(description));
        }
        if let MenuLayout::Tutorial { description } = &page.layout {
            p.set_path("menu.description", Value::str(description));
        }
        if let MenuLayout::Position { suffix, .. } = &page.layout {
            p.set_path("menu.description", Value::str(suffix));
        }
        // (the race badge's own: its second line, and the ring's colour in the menu's type)
        if let MenuLayout::StartOfRace { description, ring } = &page.layout {
            p.set_path("menu.description", Value::str(description));
            p.set_path("menu.type", Value::Int(*ring as i32));
        }
        // the movie measures its own content to size the panel, so a brand with
        // short rim names draws a narrower panel than one with long names and the
        // shop resizes as the tabs are stepped. padding the labels to a constant
        // length makes every tab measure the same and the panel hold still.
        let pad = page.pad_labels;
        for (i, r) in page.rows.iter().enumerate() {
            let label: String = if pad > r.label.chars().count() {
                let mut s = r.label.clone();
                s.extend(std::iter::repeat(' ').take(pad - r.label.chars().count()));
                s
            } else {
                r.label.clone()
            };
            p.set_path(&format!("menu.elements.{}.text1", i), Value::str(&label));
            // A value that says what kind of row this is (see `CHECK_ON` ...): the game's
            // option rows are a tick box (`type` 4, ticked by `select`) or a choice with
            // arrows either side (`count` > 0).
            let check = r.value == CHECK_ON || r.value == CHECK_OFF;
            let choice = r.value.starts_with(CHOICE);
            let glyph = r.value.strip_prefix(GLYPH);
            p.set_path(&format!("menu.elements.{}.type", i), Value::Int(if check { 4 } else if glyph.is_some() { 7 } else if r.value == NEW_ROW { 50 } else { 0 }));
            // (the performance shop's rows keep the box their cursor stands on, set above)
            if !matches!(page.layout, MenuLayout::PerfShop { .. }) {
                p.set_path(&format!("menu.elements.{}.select", i), Value::Int((r.value == CHECK_ON) as i32));
            }
            // A choice row carries its own list of choices (`elements[i].elements[j].name`,
            // `count` of them, `select` the one shown between the arrows). The port hands it
            // the current value as the only choice it needs to show.
            // (`choice_list`: all the choices, tab separated after the index of the one shown -
            // the movie sizes its value column from every name)
            let listed: Option<(i32, Vec<&str>)> = r.value.strip_prefix(CHOICE).filter(|v| v.contains('\t')).map(|v| {
                let mut f = v.split('\t');
                (f.next().and_then(|n| n.parse().ok()).unwrap_or(0), f.collect())
            });
            p.set_path(&format!("menu.elements.{}.count", i), Value::Int(match &listed { Some((_, l)) => l.len() as i32, None if choice => 3, None => 0 }));
            if let Some((at, names)) = &listed {
                for (j, n) in names.iter().enumerate() {
                    p.set_path(&format!("menu.elements.{}.elements.{}.name", i, j), Value::str(n));
                }
                p.set_path(&format!("menu.elements.{}.select", i), Value::Int(*at));
            } else if choice {
                for j in 0..3 {
                    p.set_path(&format!("menu.elements.{}.elements.{}.name", i, j), Value::str(&r.value[CHOICE.len()..]));
                }
                p.set_path(&format!("menu.elements.{}.select", i), Value::Int(1));
            }
            if let Some(sw) = r.value.strip_prefix(SWATCH) {
                // a colour row: `color` is the colour on the car; with `select` >= 0 the strip of
                // five from `elements[j].color` is open about that cursor
                let mut f = sw.split(';');
                let rgb: i32 = f.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0) as i32;
                let select: i32 = f.next().and_then(|v| v.parse().ok()).unwrap_or(-1);
                let strip: Vec<i32> = f.next().unwrap_or("").split(',').filter_map(|v| v.parse::<u32>().ok()).map(|v| v as i32).collect();
                // (the states say `fs_type` 30 / 31; the movie knows the row as `type` 3)
                p.set_path(&format!("menu.elements.{}.type", i), Value::Int(3));
                p.set_path(&format!("menu.elements.{}.color", i), Value::Int(rgb));
                p.set_path(&format!("menu.elements.{}.select", i), Value::Int(select));
                p.set_path(&format!("menu.elements.{}.count", i), Value::Int(strip.len() as i32));
                for (j, c) in strip.iter().enumerate() {
                    p.set_path(&format!("menu.elements.{}.elements.{}.color", i, j), Value::Int(*c));
                }
                p.set_path(&format!("menu.elements.{}.text2", i), Value::str(""));
                p.set_path(&format!("menu.elements.{}.text3", i), Value::str(""));
            } else if check || choice {
                p.set_path(&format!("menu.elements.{}.text2", i), Value::str(if choice { &r.value[CHOICE.len()..] } else { "" }));
                p.set_path(&format!("menu.elements.{}.text3", i), Value::str(""));
                // (a choice row shows its `price` member between the arrows)
                p.set_path(&format!("menu.elements.{}.price", i), Value::str(if choice { &r.value[CHOICE.len()..] } else { "" }));
            } else if let MenuLayout::Reward { .. } = page.layout {
                // (the reward row's second line is its `text3`; a group's header - `property1` 13
                // UNLOCKED, 17 REWARDED - carries its count in `text2`: sub_82676B08)
                p.set_path(&format!("menu.elements.{}.text2", i), Value::str(if matches!(page.row_props.get(i), Some(13 | 17)) { &r.value } else { "" }));
                p.set_path(&format!("menu.elements.{}.text3", i), Value::str(&r.value));
            } else if !matches!(page.layout, MenuLayout::Vehicles { .. } | MenuLayout::PerfShop { .. } | MenuLayout::Abilities { .. } | MenuLayout::Results { .. }) {
                p.set_path(&format!("menu.elements.{}.text2", i), r.part.as_ref().map_or_else(|| Value::str(&r.value), |part| Value::Int(part.price.clamp(0, i32::MAX as i64) as i32)));
                p.set_path(&format!("menu.elements.{}.text3", i), Value::Int(r.part.as_ref().map_or(12, |part| part.brand)));
            }
            if !matches!(page.layout, MenuLayout::PerfShop { .. } | MenuLayout::Abilities { .. } | MenuLayout::Results { .. }) {
                p.set_path(&format!("menu.elements.{}.text4", i), Value::str(""));
            }
            if let Some(g) = glyph {
                p.set_path(&format!("menu.elements.{}.text2", i), Value::str(g));
            }
            if r.value == NEW_ROW {
                p.set_path(&format!("menu.elements.{}.text2", i), Value::str(""));
            }
            // (a colour row keeps its own colour)
            if !r.value.starts_with(SWATCH) && !matches!(&page.layout, MenuLayout::VinylEditTab { colors, .. } if !colors.is_empty()) {
                p.set_path(&format!("menu.elements.{}.color", i), Value::Int(0xffffff));
            }
            if !matches!(page.layout, MenuLayout::PerfShop { .. }) {
                p.set_path(&format!("menu.elements.{}.ID", i), Value::Int(0));
                p.set_path(&format!("menu.elements.{}.property1", i), Value::Int(page.row_props.get(i).copied().unwrap_or(r.enabled as i32)));
            }
        }
        p.set_path("menu.count", Value::Int(page.rows.len() as i32));
        // (`select` usize::MAX: no row under the cursor - a panel of figures)
        p.set_path("menu.select", Value::Int(if page.select == usize::MAX { -1 } else { page.select.min(page.rows.len().saturating_sub(1)) as i32 }));
        // PartMenu shows six PartType rows, the rim shop four (`fs_List_max`); the
        // movie runs its own selection expansion and scrolling.
        let list_max = match page.layout {
            MenuLayout::Vehicles { .. } => 12,
            MenuLayout::VinylPackages { .. } => 4,
            MenuLayout::VinylShapes { .. } => 7,
            MenuLayout::VinylEditTab { .. } => 23,
            MenuLayout::VinylOptions { .. } => 10,
            MenuLayout::MissionLog { .. } => 15,
            MenuLayout::PerfShop { .. } => 10,
            MenuLayout::GarageParts => 6,
            MenuLayout::GarageRims => 4,
            _ if page.list_max > 0 => page.list_max,
            _ => 10,
        };
        p.set_path("menu.List_max", Value::Int(list_max as i32));
        p.set_global("ScrollbarVisible", Value::Int((page.list_max > 0 && page.rows.len() > list_max) as i32));
        p.set_path("prompt.count", Value::Int(page.prompts.len() as i32));
        for (i, (t, icon)) in page.prompts.iter().enumerate() {
            p.set_path(&format!("prompt.elements.{}.text1", i), Value::str(t));
            p.set_path(&format!("prompt.elements.{}.iconID", i), Value::Int(*icon));
        }
        match &page.money {
            Some((label, amount)) => {
                p.set_global("showmoney", Value::Int(1));
                p.set_global("t_banktxt", Value::str(label));
                p.set_global("bank", Value::Int((*amount).clamp(i32::MIN as i64, i32::MAX as i64) as i32));
                // (the GPS panel prints `bank` as it is: give it the text)
                if matches!(page.layout, MenuLayout::GpsMap { .. }) {
                    let d = amount.abs().to_string();
                    let mut out = String::from("$");
                    for (i, c) in d.chars().enumerate() {
                        if i > 0 && (d.len() - i) % 3 == 0 {
                            out.push(',');
                        }
                        out.push(c);
                    }
                    p.set_global("bank", Value::str(&out));
                }
            }
            None => p.set_global("showmoney", Value::Int(0)),
        }
        p.set_global("update", Value::Int(1));
        let level = if structure_changed { 0 } else { 1 };
        let cur = p.get_global("TransitionLevel").map_or(-1, |v| v.to_i32());
        if cur < 0 || level < cur {
            p.set_global("TransitionLevel", Value::Int(level));
        }
        // (after the rows, the rewards' own members: a money row's "TOTAL:" before the
        // counted figure; a mission's header - `property1` 18 - with the number of its
        // objectives in `text3` and 1 in `text4` for the COMPLETE stamp; an objective -
        // 6 - ticked by `select` 1)
        if let MenuLayout::Reward { .. } = &page.layout {
            for (i, r) in page.rows.iter().enumerate() {
                match page.row_props.get(i).copied().unwrap_or(0) {
                    12 => p.set_path(&format!("menu.elements.{}.text2", i), Value::str(&r.value)),
                    18 => {
                        let n = page.row_props[i + 1..].iter().take_while(|k| **k == 6).count();
                        p.set_path(&format!("menu.elements.{}.text3", i), Value::Int(n as i32));
                        p.set_path(&format!("menu.elements.{}.text4", i), Value::Int((r.value == "1") as i32));
                    }
                    6 => p.set_path(&format!("menu.elements.{}.select", i), Value::Int((r.value == "1") as i32)),
                    // (a results header - `property1` 25, the time trial's TIME: the number of
                    // its rows of kind 19 in `text2`, 1 in `text4` for the COMPLETE stamp -
                    // sub_82678B90; the movie's rule at 0x13f10..0x14399)
                    25 => {
                        let n = page.row_props[i + 1..].iter().take_while(|k| **k == 19).count();
                        p.set_path(&format!("menu.elements.{}.text2", i), Value::Int(n as i32));
                        p.set_path(&format!("menu.elements.{}.text4", i), Value::Int((r.value == "1") as i32));
                    }
                    _ => {}
                }
            }
        }
        // (after the rows: the wager's slider row and the money line)
        if let MenuLayout::Hangout { description, slider, money } = &page.layout {
            p.set_path("menu.description", Value::str(description));
            if let Some((row, cur, lo, hi)) = slider {
                p.set_path(&format!("menu.elements.{}.property1", row), Value::Int(16));
                // (one column: a second one would write the amount over the knob)
                p.set_path("menu.type", Value::Int(std::env::var("MCLA_SLIDER_TYPE").ok().and_then(|t| t.parse().ok()).unwrap_or(1)));
                p.set_path(&format!("menu.elements.{}.text2", row), Value::Int(*cur as i32));
                p.set_path(&format!("menu.elements.{}.text3", row), Value::Int(*lo as i32));
                p.set_path(&format!("menu.elements.{}.text4", row), Value::Int(*hi as i32));
            }
            p.set_global("showmoney", Value::Int(money.is_some() as i32));
            if let Some(m) = money {
                p.set_global("lang", Value::Int(0));
                // (this movie's own names for the money line: `bank` the figure, `t_banktxt`
                // the word before it)
                p.set_global("bank", Value::Int((*m).clamp(0, i32::MAX as i64) as i32));
                p.set_global("t_bank", Value::Int((*m).clamp(0, i32::MAX as i64) as i32));
                p.set_global("t_banktxt", Value::str(std::env::var("MCLA_BANKTXT").as_deref().unwrap_or("MONEY  ")));
            }
        }
        self.shown = Some(page.clone());
    }

    pub fn hide(&mut self) {
        if self.shown.take().is_some() {
            self.movie.player.set_global("cur_visibility", Value::Int(0));
        }
    }
}

/// `prompt/prompt.swf`: the button prompt bar along the bottom of the screen with
/// the money (and cart) line, as the garage shows it. Same protocol as the menus'
/// own bar: `prompt.count`, `prompt.elements[i].text1/iconID`, `update`; the
/// money is `showmoney` and `t_bank`.
pub struct FlashPrompt {
    pub movie: FlashMovie,
    shown: Option<(Vec<(String, i32)>, Option<i64>, Option<i64>)>,
}

impl FlashPrompt {
    pub fn load(data_dir: &str, shared: &UiShared) -> Option<FlashPrompt> {
        let mut movie = FlashMovie::load(data_dir, "resources/ui/prompt/0x3ae388d8.r27", shared)?;
        for _ in 0..4 {
            movie.player.tick();
        }
        Some(FlashPrompt { movie, shown: None })
    }

    /// `prompts` in the game's order (the bar lays them out right to left).
    /// `cart`: what is in the garage's cart (`showcart`, `t_cart`), shown before the money.
    pub fn show(&mut self, prompts: &[(String, i32)], money: Option<i64>, cart: Option<i64>) {
        let state = (prompts.to_vec(), money, cart);
        if self.shown.as_ref() == Some(&state) {
            return;
        }
        let p = &mut self.movie.player;
        p.set_global("cur_visibility", Value::Int(1));
        p.set_path("prompt.template", Value::Int(0));
        // the money's thousands separator goes by language; the movie starts on 2
        p.set_global("lang", Value::Int(0));
        p.set_path("prompt.count", Value::Int(prompts.len() as i32));
        for (i, (t, icon)) in prompts.iter().enumerate() {
            p.set_path(&format!("prompt.elements.{}.text1", i), Value::str(t));
            p.set_path(&format!("prompt.elements.{}.iconID", i), Value::Int(*icon));
        }
        p.set_global("showmoney", Value::Int(money.is_some() as i32));
        if let Some(m) = money {
            p.set_global("t_bank", Value::Int(m.clamp(0, i32::MAX as i64) as i32));
        }
        p.set_global("showcart", Value::Int(cart.is_some() as i32));
        if let Some(c) = cart {
            p.set_global("t_cart", Value::Int(c.clamp(0, i32::MAX as i64) as i32));
        }
        p.set_global("update", Value::Int(1));
        self.shown = Some(state);
    }

    /// The replay's legend and playhead (`prompt.template` 1, `template_Replay`): the
    /// prompts in a column at the top right, the bar along the bottom filled to `position`
    /// (0..1) with the knob's `icon` (`prompt.replayIcon`, a frame of `playhead.icons`).
    pub fn show_replay(&mut self, prompts: &[(String, i32)], icon: i32, position: f32) {
        let p = &mut self.movie.player;
        p.set_global("cur_visibility", Value::Int(1));
        p.set_path("prompt.template", Value::Int(1));
        p.set_path("prompt.replayIcon", Value::Int(icon));
        p.set_path("prompt.replayPosition", Value::Float(position.clamp(0.0, 1.0)));
        p.set_path("prompt.count", Value::Int(prompts.len() as i32));
        for (i, (t, icon)) in prompts.iter().enumerate() {
            p.set_path(&format!("prompt.elements.{}.text1", i), Value::str(t));
            p.set_path(&format!("prompt.elements.{}.iconID", i), Value::Int(*icon));
        }
        p.set_global("showmoney", Value::Int(0));
        p.set_global("showcart", Value::Int(0));
        p.set_global("update", Value::Int(1));
        // (the bar's own state is another template's: the next `show` lays it out again)
        self.shown = Some((vec![("\u{1}replay".to_string(), 0)], None, None));
    }

    pub fn hide(&mut self) {
        if self.shown.take().is_some() {
            self.movie.player.set_global("showcart", Value::Int(0));
            self.movie.player.set_path("prompt.count", Value::Int(0));
            self.movie.player.set_global("showmoney", Value::Int(0));
            self.movie.player.set_global("update", Value::Int(1));
        }
    }
}

/// `sidekick.swf` (the folder is the hash of "sidekick"): the phone. The movie is the
/// phone's screen and also steers the phone itself - three meshes it lists in
/// `dyn3Ds` (`sidekick_body`, `sidekick_screenshell`, `sidekick_screen`), slid in from
/// the corner and flipped open by its own script when `cur_visibility` rises.
/// The game fills `message` (`0x82650BB0`): `title`, `description`, `prompt`,
/// `IconID` (portrait frame - 1; 30 / 31 are the gamer pictures), and sets
/// `sidekickisvisible` and `promptisvisible`.
pub struct FlashPhone {
    pub movie: FlashMovie,
    shown: Option<(String, String, i32, String)>,
    /// frames left to let the phone fold and slide away after the message
    pub closing: u32,
}

pub const PHONE_MOVIE: &str = "resources/ui/0xabc77a4f/0x1ddfa2df.r27";

impl FlashPhone {
    pub fn load(data_dir: &str, shared: &UiShared) -> Option<FlashPhone> {
        let mut movie = FlashMovie::load(data_dir, PHONE_MOVIE, shared)?;
        for _ in 0..5 {
            movie.player.tick();
        }
        Some(FlashPhone { movie, shown: None, closing: 0 })
    }

    pub fn is_active(&self) -> bool {
        self.shown.is_some() || self.closing > 0
    }

    /// A message is up (the sidekick control's SetVisible: the phone's ring / close sounds).
    pub fn visible(&self) -> bool {
        self.shown.is_some()
    }

    /// Show a message (or none), and run the movie.
    /// `prompt`: the line at the foot of the screen with the Back glyph ("MISSION LOG"
    /// on a mission call), empty for none.
    pub fn drive(&mut self, message: Option<(&str, &str, i32)>, prompt: &str, night: bool, dt: f32) {
        let p = &mut self.movie.player;
        // (wide screen: the movie's `ratiooffsetx[aspect]` is 0 there and 0.4 on 4:3; it
        // never asks for the value itself)
        p.set_global("aspect", Value::Int(1));
        // The reference capture, taken at night with the gauge cluster in its night colours,
        // shows the phone's screen in its day (grey) look: the game does not seem to answer
        // this movie's `getvar IsNight`, so the flag stays 0.
        let _ = night;
        p.set_global("d_IsNight", Value::Int(0));
        match message {
            Some((title, text, icon)) => {
                let state = (title.to_string(), text.to_string(), icon, prompt.to_string());
                if self.shown.as_ref() != Some(&state) {
                    p.set_path("message.title", Value::str(title));
                    p.set_path("message.description", Value::str(text));
                    p.set_path("message.prompt", Value::str(prompt));
                    p.set_path("message.IconID", Value::Int(icon));
                    p.set_global("promptisvisible", Value::Int(!prompt.is_empty() as i32));
                    p.set_global("sidekickisvisible", Value::Int(1));
                    p.set_global("cur_visibility", Value::Int(1));
                    self.shown = Some(state);
                }
            }
            None => {
                if self.shown.take().is_some() {
                    p.set_global("cur_visibility", Value::Int(0));
                    // (the script folds it by its timer: 22 movie frames)
                    self.closing = 45;
                }
                self.closing = self.closing.saturating_sub(1);
            }
        }
        p.update(dt);
    }
}

/// `hudmap/hudmap.swf`: the minimap's frame (background tint, ring, north
/// marker, GPS arrow on the ring). The original renders the map itself (streets,
/// 3D icon meshes) into a render target that the movie shows in `HUD.renderMe`
/// (a magenta placeholder bitmap on the disc); the port draws its own street map
/// at that place and hides the placeholder.
pub struct FlashMinimap {
    pub movie: FlashMovie,
    /// the map camera the movie steers (`target0_cam`): metres above the player
    /// (750 + 60 x speed, the speed held to 19..40 m/s, eased) and how far ahead of
    /// the player it looks (110 m); its lens is 35 degrees
    pub cam_height: f32,
    pub look_ahead: f32,
    /// the square of the screen the map is rendered to (`vpx`, `vpy`, `vpw`, `vph`
    /// in 720p pixels: 78, 464, 219, 219)
    pub viewport: [f32; 4],
    /// the GPS arrow is being pointed out (the MINI MAP lesson): the movie's
    /// `gpsArrowActive` above 1 makes it pulse in its glow
    pub highlight: bool,
    /// `target0_cam.position` / `.lookat` (in the car's heading frame) and `.fovy`, after the
    /// movie's own easing (`hudmap.rs` builds the map camera from them)
    pub cam_eye: glam::Vec3,
    pub cam_look: glam::Vec3,
    pub cam_fovy: f32,
    /// `map_rotation` (degrees): the screen angle of world north, from the map camera
    pub map_rotation: f32,
    /// the map is drawn by `hudmap.rs` into `renderMe` (else the port's own drawing replaces it)
    pub own_map: bool,
    /// the overhead map is up (the HUD state hudOverheadMap: `cur_Property[1]` = 2), and the title
    /// its bar carries (a red light race's landmark: HUD control 11's text with its property 0 on)
    pub overhead: bool,
    pub overhead_title: Option<String>,
    /// the title bar's icon (ctrl+196, `overhead_iconID`) and second text (ctrl+200,
    /// `overhead_text1`): a delivery intro's district emblem and "Delivery" (walkthrough 2:46:30)
    pub overhead_sub: Option<(i32, String)>,
    /// `target0.aspect` (= viewportWindow width / height, the movie's own easing)
    pub aspect: f32,
}

impl FlashMinimap {
    pub fn load(data_dir: &str, shared: &UiShared) -> Option<FlashMinimap> {
        Some(FlashMinimap { movie: FlashMovie::load(data_dir, "resources/ui/hudmap/0x7064220c.r27", shared)?, cam_height: 1890.0, look_ahead: 110.0, viewport: [78.0, 464.0, 219.0, 219.0], highlight: false, cam_eye: glam::Vec3::new(0.0, 1891.0, 0.0), cam_look: glam::Vec3::new(0.0, 0.0, -110.0), cam_fovy: 35.0, map_rotation: 0.0, own_map: false, overhead: false, overhead_title: None, overhead_sub: None, aspect: 1.0 })
    }

    /// `gps`: bearing of the GPS target relative to the map's up, degrees clockwise.
    pub fn drive(&mut self, info: &HudInfo, gps: Option<f32>, dt: f32) {
        let p = &mut self.movie.player;
        p.set_global("Mapisvisible", Value::Int(1));
        p.set_global("d_IsNight", Value::Int(info.night as i32));
        p.set_global("d_Speedvalue", Value::Float((info.speed_kmh / 3.6).abs()));
        p.set_global("gpsArrowActive", Value::Int(if gps.is_some() { 1 + self.highlight as i32 } else { 0 }));
        p.set_global("gpsArrowRotation", Value::Float(gps.unwrap_or(0.0)));
        // the ring and its north marker turn with the map, which turns with the car (with the
        // map drawn here: the screen angle of world north through the map camera, 0x822375CC)
        p.set_global("map_rotation", Value::Float(if self.own_map { self.map_rotation } else { -info.player_heading.to_degrees() }));
        // The overhead map (rule_frame.md 9): hud.sc.xml's hudOverheadMap writes cur_Property[1] = 2,
        // hudBigMap's exit and dashboardNormal 0. SC_map = (city prefix == "sc"), sub_8220CC10 - the
        // port's city is always "sc". The title bar: mcMapCtrl (HUD control 11) vfunc 44,
        // sub_826510F0 - with its property 0 on overhead_type 1, overhead_iconID = ctrl+196 (-1 from
        // the constructor: no icon), overhead_text0 = its text, overhead_text1 = ctrl+200 (empty);
        // else overhead_type 0 and both texts empty.
        p.set_path("cur_Property.1", Value::Int(if self.overhead { 2 } else { 0 }));
        p.set_global("SC_map", Value::Int(1));
        match self.overhead_title.as_deref() {
            Some(t) => {
                p.set_global("overhead_type", Value::Int(1));
                p.set_global("overhead_iconID", Value::Int(self.overhead_sub.as_ref().map_or(-1, |s| s.0)));
                p.set_global("overhead_text0", Value::str(t));
                p.set_global("overhead_text1", Value::str(self.overhead_sub.as_ref().map_or("", |s| s.1.as_str())));
            }
            None => {
                p.set_global("overhead_type", Value::Int(0));
                p.set_global("overhead_text0", Value::str(""));
                p.set_global("overhead_text1", Value::str(""));
            }
        }
        p.update(dt);
        // the map itself is the game's render target shown in `renderMe` (a magenta
        // placeholder on the disc) over the `miniBG` plate; without `hudmap.rs`'s picture the
        // port draws its own map in their place
        if !self.own_map {
            for name in ["HUD.renderMe", "HUD.miniBG"] {
                if let Some(id) = p.find(name) {
                    if let Some(i) = p.inst_mut(id) {
                        i.tint[3] = 0;
                    }
                }
            }
        }
        let num = |p: &mut Player, path: &str, or: f32| p.get_path(path).map_or(or, |v| v.to_f32());
        self.cam_height = num(p, "target0_cam.position.1", 1890.0).max(100.0);
        self.look_ahead = -num(p, "target0_cam.lookat.2", -110.0);
        self.viewport = [num(p, "vpx", 78.0), num(p, "vpy", 464.0), num(p, "vpw", 219.0), num(p, "vph", 219.0)];
        self.cam_eye = glam::Vec3::new(num(p, "target0_cam.position.0", 0.0), num(p, "target0_cam.position.1", 1891.0), num(p, "target0_cam.position.2", 0.0));
        self.cam_look = glam::Vec3::new(num(p, "target0_cam.lookat.0", 0.0), num(p, "target0_cam.lookat.1", 0.0), num(p, "target0_cam.lookat.2", -110.0));
        self.cam_fovy = num(p, "target0_cam.fovy", 35.0);
        self.aspect = num(p, "target0.aspect", 1.0);
    }
}

/// The race state the HUD movie shows.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum RaceHudKind {
    #[default]
    Race,
    Delivery,
    Payback,
}

#[derive(Clone, Copy, Default)]
pub struct RaceHud {
    pub kind: RaceHudKind,
    /// seconds since the start
    pub elapsed: f32,
    /// missions against the clock: seconds left
    pub time_left: Option<f32>,
    pub lap: i32,
    pub laps: i32,
    pub pos: i32,
    pub total: i32,
    /// seconds before the start (the port counts READY above 3)
    pub countdown: f32,
    pub delivery_damage: f32,
    pub target_damage: f32,
    /// the race clock counts down to a target (HUD control 6 with its property 0 on:
    /// sub_8264F620): the movie's `TotalCountdown` - the time left in ms x 0.0001 - and
    /// `red_countdown` (its property 2); `time_left` is then the time shown
    pub clock_count: Option<(f32, bool)>,
    /// the race clock is not on show (a payback before its client is reached)
    pub clock_off: bool,
    /// a payback's wins, wins needed and the meter's full mark (HUD control 10's
    /// properties 1, 2 and 4: `payback_stored`, `payback_level`, `payback_meterMax`)
    pub payback: Option<(i32, i32, f32)>,
    /// step 19d: `paybackisvisible` (HUD control 10's property 5): 1 from the leg's start,
    /// 2 once its time is up - the movie hides the damage bar then
    pub payback_vis: i32,
}

/// The game's race clock text (`0x8223DE50`): `[-][h:]mm:ss.hh` from milliseconds.
pub fn race_time(ms: i32) -> String {
    let sign = if ms < 0 { "-" } else { "" };
    let ms = ms.unsigned_abs();
    let (h, m, s, c) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000 / 10);
    if h > 0 {
        format!("{}{}:{:02}:{:02}.{:02}", sign, h, m, s, c)
    } else {
        format!("{}{:02}:{:02}.{:02}", sign, ms / 60_000, s, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Print the game's own wording for the garage dimension menu.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn dump_dimension_strings() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let t = StringTable::parse(&std::fs::read(Path::new(&dir).join("ui/0xa4c1e8fa")).unwrap()).unwrap();
        for k in ["G_RimSize", "G_TireProfile", "G_TireWidth", "G_RideHeight", "G_FrontAxle", "G_RearAxle", "G_BothAxle", "G_Install"] {
            println!("{k} = {:?}", t.get(k));
        }
    }

    /// Exercises the host bridge against the shipped ActionScript, including a
    /// long-list wrap and a template change with the same row count/title.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn garage_templates_render_prices_scroll_and_return_to_the_area_loop() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let strings = StringTable::parse(&std::fs::read(Path::new(&dir).join("ui/0xa4c1e8fa")).unwrap()).unwrap();
        let shared = UiShared { textures: Arc::new(HashMap::new()), strings: Arc::new(strings), logic: Arc::new(crate::menu3d::UiLogic::load(&dir)) };
        let mut menu = FlashMenu::load(&dir, GARAGE_MOVIE, [0.0; 2], &shared).unwrap();
        let mut page = MenuPage {
            layout: MenuLayout::GarageParts,
            icon: 10,
            title: "WIDEBODY".into(),
            rows: (0..16).map(|i| MenuRow {
                label: format!("PART {}", i), value: String::new(), enabled: true,
                part: Some(PartRow { price: i * 12_000, brand: 12 }),
            }).collect(),
            select: 1,
            money: Some(("MONEY".into(), 5000)),
            ..MenuPage::default()
        };
        let settle = |menu: &mut FlashMenu, page: &MenuPage| {
            menu.show(page);
            for _ in 0..45 { menu.movie.player.tick(); }
        };
        let field = |p: &Player, path: &str| p.inst(p.find(path).unwrap()).unwrap().text.as_deref().unwrap_or_default().to_string();
        settle(&mut menu, &page);
        assert_eq!(field(&menu.movie.player, "screen.menu.slotsB.slot1.slot"), "$12,000");
        assert_eq!(menu.movie.player.inst(menu.movie.player.find("screen.menu.vs_brands").unwrap()).unwrap().frame, 12);
        for select in [15, 0, 8, 7] {
            page.select = select;
            settle(&mut menu, &page);
            let offset = menu.movie.player.get_global("ScrollOffset").unwrap().to_i32() as usize;
            let visible = select.checked_sub(offset).expect("selected row must be in the window");
            assert!(visible < 6, "selection {} offset {}", select, offset);
            let slot = format!("screen.menu.slotsA.slot{}", visible);
            assert_eq!(menu.movie.player.inst(menu.movie.player.find(&slot).unwrap()).unwrap().tint[3], 255);
            assert_eq!(field(&menu.movie.player, &format!("{}.slot", slot)), format!("PART {}", select));
            assert_eq!(field(&menu.movie.player, &format!("screen.menu.slotsB.slot{}.slot", visible)), format!("${},000", select * 12).replace("$0,000", "$0"));
        }
        page.rows.truncate(2);
        page.select = 0;
        page.layout = MenuLayout::GarageLoop { names: vec!["VEHICLE".into(), "FRONT".into(), "SIDE".into()], index: 1, flat: false };
        for row in &mut page.rows { row.part = None; }
        settle(&mut menu, &page);
        assert_eq!(field(&menu.movie.player, "screen.menu.title_loop.slot1.slot"), "SIDE");
        assert_eq!(field(&menu.movie.player, "screen.menu.title_loop.slot2.slot"), "FRONT");
        assert_eq!(field(&menu.movie.player, "screen.menu.title_loop.slot3.slot"), "VEHICLE");
        assert_eq!(menu.movie.player.inst(menu.movie.player.find("screen.menu.vs_brands").unwrap()).unwrap().tint[3], 0);
        // Same title and row count: layout alone must invalidate the template.
        page.layout = MenuLayout::GarageParts;
        page.rows[0].part = Some(PartRow { price: 12345, brand: 12 });
        settle(&mut menu, &page);
        assert_eq!(field(&menu.movie.player, "screen.menu.slotsB.slot0.slot"), "$12,345");
        menu.hide();
        settle(&mut menu, &page);
        assert_eq!(field(&menu.movie.player, "screen.menu.slotsB.slot0.slot"), "$12,345");
    }

    #[test]
    fn message_boxes_read_title_text_and_choices() {
        let xml = r#"<scxml>
   <state id="QuitRaceMenu" class="UIMessageBox">
      <data name="type"				expr="1"	                    ></data>
      <data name="title"         expr="Dialog_Title_QuitRace" ></data>
      <data name="description"   expr="ol_msg_quitrace_desc"  ></data>
      <state id="QRM_No" desc="No" class="UIComponent">
         <transition event="uinput.action_pressed" consume="false" ></transition>
      </state>
      <state id="QRM_Yes" desc="Yes" class="UIComponent">
         <transition event="uinput.action_pressed" consume="false"></transition>
      </state>
   </state>
  <state id="FullFixConfirm" class="UIMessageBox">
    <data name="title"	expr="G_FullFixTitle"	></data>
    <state id="FF_Yes" desc="Yes" class="UIComponent"></state>
  </state>
</scxml>"#;
        let b = MessageBoxDef::parse(xml, "QuitRaceMenu").unwrap();
        assert_eq!((b.title.as_str(), b.description.as_str()), ("Dialog_Title_QuitRace", "ol_msg_quitrace_desc"));
        assert_eq!(b.options, vec![("QRM_No".to_string(), "No".to_string()), ("QRM_Yes".to_string(), "Yes".to_string())]);
        assert!(MessageBoxDef::parse(xml, "Missing").is_none());
    }

    #[test]
    fn race_clock_text_follows_the_games_format() {
        // 0x8223DE50: "%s%02d:%02d.%02d", from an hour "%s%d:%02d:%02d.%02d"
        assert_eq!(super::race_time(0), "00:00.00");
        assert_eq!(super::race_time(4_910), "00:04.91");
        assert_eq!(super::race_time(83_456), "01:23.45");
        assert_eq!(super::race_time(3_723_450), "1:02:03.45");
        assert_eq!(super::race_time(-2_500), "-00:02.50");
    }
}

/// `hud/hud.swf` driven from the port's HUD state.
pub struct FlashHud {
    pub movie: FlashMovie,
    countdown_shown: bool,
    /// the game's splash queues (`0x82234440`): per slot, up to five waiting
    /// (text, colour, seconds), posted when the movie has freed the slot
    splash_queue: [std::collections::VecDeque<(String, i32, f32)>; 3],
    /// a splash was posted to queue 1 (the movie then plays RACER_FOUND_MASTER)
    pub racer_found: bool,
    /// the camera bar: the view last seen (-1 at the start, as the game's own copy)
    camera_last: i32,
}

/// Splash slots of the HUD movie (`SplashTypes` 5, 4, 10 for slots 0..2; slot 3
/// is the countdown). Colours: 0 white, 1 blue, 2 red.
pub const SPLASH_CENTER: usize = 0;
pub const SPLASH_TOP: usize = 1;
pub const SPLASH_BOTTOM: usize = 2;

impl FlashHud {
    pub fn load(data_dir: &str, shared: &UiShared) -> Option<FlashHud> {
        let movie = FlashMovie::load(data_dir, "resources/ui/hud/0x1f0e1cda.r27", shared)?;
        Some(FlashHud { movie, countdown_shown: false, splash_queue: Default::default(), racer_found: false, camera_last: -1 })
    }

    /// The camera bar at the bottom (`camera_indicator`). The game names the views once
    /// (sub_8223D300: `_global.cameras[0..4]` = the string table's "Bumper", "POV", "Boom 1",
    /// "Boom 2", "Boom 3" - BUMPER, COCKPIT, STATIC, CLASSIC, ACTION - and [5] the race
    /// editor's OVERHEAD VIEW) and every frame (sub_8223D450) sets `max_cameraNumber` 4 and,
    /// when the view is not the one of the frame before (and that one was known), writes the
    /// new one into `cameraNumber` - unless a menu is up. The movie does the rest: the bar
    /// comes in by a quarter of the way a frame, stands 45 frames and goes out the same way
    /// (`cameraTimer`, `cur_cameraAlpha`), the name in `slot`, the place on the `meter`.
    pub fn camera(&mut self, view: i32, menu: bool) {
        // (after the movie's own first frame, which fills the list with its sample names)
        let first = self.movie.string(crate::boomcam::VIEW_KEYS[0]);
        let p = &mut self.movie.player;
        if p.get_path("cameras.0").map_or(false, |v| v.to_string().to_string() != first) {
            for (i, key) in crate::boomcam::VIEW_KEYS.iter().enumerate() {
                let name = self.movie.string(key);
                self.movie.player.set_path(&format!("cameras.{i}"), Value::str(&name));
            }
        }
        let p = &mut self.movie.player;
        p.set_global("max_cameraNumber", Value::Int(4));
        if view != self.camera_last {
            if self.camera_last != -1 && view != -1 && !menu {
                p.set_global("cameraNumber", Value::Int(view));
            }
            self.camera_last = view;
        }
    }

    /// The line under the speed (`d_selectedlevel_longname`; the movie's own sample
    /// text is "HOLLYWOOD"). `None` leaves it empty, as the game does when it has
    /// nothing queued. A road keeps its name on show through junctions: the text only
    /// changes when another name arrives or the car stops.
    pub fn info_line(&mut self, text: Option<&str>) {
        let p = &mut self.movie.player;
        p.set_global("infotextHilight", Value::Int(0));
        p.set_global("d_selectedlevel_longname", Value::str(text.unwrap_or("")));
    }

    /// Queue a splash message (`0x82234A10`). The game keeps five per slot and
    /// drops the oldest when full.
    pub fn splash(&mut self, slot: usize, text: &str, color: i32, seconds: f32) {
        let q = &mut self.splash_queue[slot.min(2)];
        if q.len() >= 5 {
            q.pop_front();
        }
        q.push_back((text.to_string(), color, seconds));
    }

    /// Post the next queued splash of each free slot (`0x82234440` -> `0x8223FFB0`:
    /// `SplashColor[i]`, `SplashVisibles[i]` = 1, `duration[i]` = seconds (at least
    /// 2), `SplashStrings[i]`). The movie clears `SplashVisibles[i]` when done.
    fn post_splashes(&mut self) {
        let p = &mut self.movie.player;
        for (slot, q) in self.splash_queue.iter_mut().enumerate() {
            if q.is_empty() || p.get_path(&format!("SplashVisibles.{}", slot)).map_or(0, |v| v.to_i32()) != 0 {
                continue;
            }
            let (text, color, seconds) = q.pop_front().unwrap();
            p.set_path(&format!("SplashColor.{}", slot), Value::Int(color));
            p.set_path(&format!("SplashVisibles.{}", slot), Value::Int(1));
            p.set_path(&format!("duration.{}", slot), Value::Float(seconds.max(2.0)));
            p.set_path(&format!("SplashStrings.{}", slot), Value::str(&text));
            self.racer_found |= slot == 1;
        }
    }

    /// The start of a replay: nothing of the HUD but the race's 3-2-1 (reference `vr_2`).
    pub fn drive_replay_intro(&mut self, countdown: f32, dt: f32) {
        let p = &mut self.movie.player;
        for name in ["Speedisvisible", "RPMisvisible", "Gearisvisible", "Nitroisvisible", "SAisvisible", "SSTisvisible", "Totaltimeisvisible", "Posisvisible", "Lapisvisible", "Laptimeisvisible", "deliveryisvisible", "paybackisvisible", "cruiseisvisible"] {
            p.set_global(name, Value::Int(0));
        }
        p.set_global("policePursuit", Value::Int(0));
        p.set_path("playerlist.type", Value::Int(0));
        if countdown > 0.0 && countdown <= 3.0 && !self.countdown_shown {
            self.countdown_shown = true;
            p.set_global("countdownUpdate", Value::Float(1.0 / 30.0));
            p.set_path("SplashVisibles.3", Value::Int(1));
        }
        if countdown > 3.0 {
            self.countdown_shown = false;
        }
        self.movie.player.update(dt);
    }

    /// Write this frame's values (names and units from the movie's scripts:
    /// `d_Speedvalue` m/s (x2.237 for mph), `d_RPMvalue` rpm, `RPMopt` redline rpm,
    /// `GearValue` text, `Nitrovalue` and `SAvalue` 0..1).
    pub fn drive(&mut self, info: &HudInfo, race: Option<RaceHud>, dt: f32) {
        // the police strip under the cluster. Text: `0x82650FF0` writes per message
        // kind (0 pull over, 1 pursuit -> t_police_pursuit; 2 investigation,
        // 3 searching -> t_police_search). Mode: `0x82206458` sets `policePursuit`
        // from the police state (0 none, 1 and 2 the two flashing "pursuit" looks,
        // 3 searching).
        let police = match info.heat {
            1 => Some(("cop_hud_msg_pullover", 2)),
            2 => Some(("cop_hud_msg_pursuit", 1)),
            3 => Some(("cop_hud_msg_searching", 3)),
            _ => None,
        };
        let police_text = police.map(|(k, _)| self.movie.string(k).to_uppercase());
        let p = &mut self.movie.player;
        match (police, police_text) {
            (Some((_, mode)), Some(text)) => {
                p.set_global(if mode == 3 { "t_police_search" } else { "t_police_pursuit" }, Value::str(&text));
                p.set_global("policePursuit", Value::Int(mode));
            }
            _ => p.set_global("policePursuit", Value::Int(0)),
        }
        let mps = info.speed_kmh / 3.6;
        p.set_global("d_Speedvalue", Value::Float(mps.abs()));
        p.set_global("Speedvalue", Value::Float(mps.abs()));
        p.set_global("d_RPMvalue", Value::Float(info.rpm.max(0.0)));
        p.set_global("RPMvalue", Value::Float(info.rpm.max(0.0)));
        let red = info.redline.max(1000.0);
        p.set_global("RPMopt", Value::Float(red));
        p.set_global("d_RPMopt", Value::Float(red));
        let gear = if info.gear < 0 { "R".to_string() } else if info.gear == 0 { "N".to_string() } else { info.gear.to_string() };
        p.set_global("GearValue", Value::str(&gear));
        // (the movie's meter counts canisters: frame 1 + 100 x value, value 0..5 - two lit
        // bars for two canisters in `ref/h_cruise_elevado`; `info.nitro` is canisters / 4)
        p.set_global("Nitrovalue", Value::Float((info.nitro * 4.0).clamp(0.0, 5.0)));
        p.set_global("Nitroisvisible", Value::Int(1));
        p.set_global("d_Damagevalue", Value::Float(info.damage.clamp(0.0, 1.0)));
        p.set_global("d_IsNight", Value::Int(info.night as i32));
        p.set_global("cruiseisvisible", Value::Int(info.cruise as i32));
        let has_ability = !info.ability.is_empty();
        // (with no ability installed the cluster shows two empty rings - `ng_c4`: the movie's
        // power-up slots, drawn when `PowerUpisvisible` is set and the ability meter is not)
        p.set_global("SAisvisible", Value::Int(has_ability as i32));
        p.set_global("PowerUpisvisible", Value::Int(!has_ability as i32));
        // (step 19e: the slots' types and the value - sub_82203AC8 -> sub_8223E200)
        p.set_global("PowerUptype_0", Value::Int(info.pu_hud[0] as i32));
        p.set_global("PowerUptype_1", Value::Int(info.pu_hud[1] as i32));
        p.set_global("PowerUpvalue", Value::Float(info.pu_hud[2]));
        // (the dial's icon: 1 the clock, 2 the fist, 3 the rings, 4 the bolt - the movie's
        // `SA_meter.icons` frames, seen by setting each)
        let icon = match info.ability.to_ascii_uppercase().as_str() { "AGRO" => 2, "ROAR" => 3, "PULSE" | "EMP" => 4, _ => 1 };
        p.set_global("SA_iconID", Value::Int(icon));
        p.set_global("SAvalue", Value::Float(if info.ability_fx > 0.0 { info.ability_fx } else { info.ability_meter }.clamp(0.0, 1.0)));
        p.set_global("SSTisvisible", Value::Int((info.draft > 0.15 || info.sst > 0.02) as i32));
        p.set_global("SSTvalue", Value::Float(info.sst.clamp(0.0, 1.0)));
        // the online player list (`_global.playerlist`, bound by the game in
        // 0x8228BED8): type 0 = not shown in single player
        p.set_path("playerlist.type", Value::Int(0));
        // single player: no team totals (also selects the single-player colours)
        p.set_path("playerlist.showTotals", Value::Int(0));
        let r = race.unwrap_or_default();
        let racing = race.is_some() && r.kind == RaceHudKind::Race;
        // the race clock (0x8223E000): Totaltime text, total_MS
        let shown_time = r.time_left.unwrap_or(r.elapsed);
        let ms = (shown_time.max(0.0) * 1000.0) as i32;
        // on the grid the original shows the countdown and the map only: clock, place and
        // the cluster come in at GO (reference `t09`..`t11`)
        let started = race.is_none() || r.countdown <= 0.0;
        // (p080: a delivery's cluster is up already under its intro's overhead map and through the
        // count - walkthrough 2:46:30 / 2:46:34; its clock still comes in at GO)
        let gauges = started || r.kind == RaceHudKind::Delivery;
        for name in ["Speedisvisible", "RPMisvisible", "Gearisvisible"] {
            p.set_global(name, Value::Int(gauges as i32));
        }
        if !started {
            p.set_global("Nitroisvisible", Value::Int(0));
            p.set_global("SAisvisible", Value::Int(0));
        }
        p.set_global("Totaltimeisvisible", Value::Int((race.is_some() && started && !r.clock_off) as i32));
        p.set_global("Totaltime", Value::str(&race_time(ms)));
        p.set_global("total_MS", Value::Int(ms));
        // (sub_8264F620: without a countdown `TotalCountdown` is 2 and `red_countdown` 0)
        p.set_global("TotalCountdown", Value::Float(r.clock_count.map_or(2.0, |c| c.0)));
        p.set_global("red_countdown", Value::Int(r.clock_count.map_or(false, |c| c.1) as i32));
        p.set_global("Posisvisible", Value::Int((racing && started && r.total > 1) as i32));
        p.set_global("PosNumber", Value::Int(r.pos));
        p.set_global("PosMax", Value::Int(r.total));
        p.set_global("Lapisvisible", Value::Int((racing && started && r.laps > 1) as i32));
        p.set_global("LapNumber", Value::Int(r.lap));
        p.set_global("LapMax", Value::Int(r.laps));
        p.set_global("Laptimeisvisible", Value::Int(0));
        // delivery / payback meters (0x82651240, 0x82651320)
        let delivery = race.is_some() && r.kind == RaceHudKind::Delivery;
        p.set_global("deliveryisvisible", Value::Int(delivery as i32));
        p.set_global("delivery_meter", Value::Float(r.delivery_damage.clamp(0.0, 1.0)));
        p.set_global("delivery_meterMax", Value::Float(1.0));
        let payback = race.is_some() && r.kind == RaceHudKind::Payback;
        p.set_global("paybackisvisible", Value::Int(if payback { r.payback_vis.max(1) } else { 0 }));
        p.set_global("payback_meter", Value::Float(r.target_damage.clamp(0.0, 1.0)));
        p.set_global("payback_meterMax", Value::Float(r.payback.map_or(1.0, |x| x.2)));
        // (sub_82651320: the wins of the run and the wins it asks for)
        if let Some((wins, needed, _)) = r.payback {
            p.set_global("payback_stored", Value::Int(wins));
            p.set_global("payback_level", Value::Int(needed));
        }
        // the 3-2-1-GO splash: the movie animates it itself once its splash slot 3 is
        // raised (`showCountdown`, one number per 1/countdownUpdate frames)
        if race.is_some() && r.countdown > 0.0 && r.countdown <= 3.0 && !self.countdown_shown {
            self.countdown_shown = true;
            p.set_global("countdownUpdate", Value::Float(1.0 / 30.0));
            p.set_path("SplashVisibles.3", Value::Int(1));
        }
        if race.is_none() || r.countdown > 3.0 {
            self.countdown_shown = false;
        }
        self.post_splashes();
        self.movie.player.update(dt);
    }
}
